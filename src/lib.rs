use serde::{Serialize, de::DeserializeOwned};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;

/// Execute `callable` in a forked child so that any memory changes in the child do not affect the parent.
/// The child serializes its result (using bincode) and writes it through a pipe, which the parent reads and deserializes.
///
/// # Safety
/// This code directly calls glibc functions (via the libc crate) and should only be used in a Unix environment.
/// Forking in a multithreaded process is dangerous because only the calling thread is copied.
pub fn execute_in_isolated_process<F, T>(callable: F) -> T
where
    F: FnOnce() -> T,
    T: Serialize + DeserializeOwned,
{
    // Create a pipe.
    let mut pipefds: [i32; 2] = [0; 2];
    if unsafe { libc::pipe(pipefds.as_mut_ptr()) } != 0 {
        panic!("pipe failed: {}", std::io::Error::last_os_error());
    }
    let read_fd = pipefds[0];
    let write_fd = pipefds[1];

    // Fork the process.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => panic!("fork failed: {}", std::io::Error::last_os_error()),
        0 => {
            // Child process.
            // Close the read end of the pipe.
            if unsafe { libc::close(read_fd) } != 0 {
                panic!(
                    "child: failed to close read end: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Execute the callable.
            let result = callable();
            // Serialize the result using bincode.
            let encoded = bincode::serialize(&result).expect("serialization failed");
            // Wrap the write file descriptor as a File and write the encoded data.
            let mut writer = unsafe { File::from_raw_fd(write_fd) };
            writer.write_all(&encoded).expect("failed to write to pipe");
            writer.flush().ok();
            // Exit immediately; use _exit to avoid running destructors.
            unsafe { libc::_exit(0) };
        }
        _ => {
            // Parent process.
            // Close the write end of the pipe.
            if unsafe { libc::close(write_fd) } != 0 {
                panic!(
                    "parent: failed to close write end: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Wait for the child process to exit.
            let mut status: i32 = 0;
            if unsafe { libc::waitpid(pid, &mut status as *mut i32, 0) } < 0 {
                panic!("waitpid failed: {}", std::io::Error::last_os_error());
            }
            // Read from the pipe by wrapping the read fd as a File.
            let mut reader = unsafe { File::from_raw_fd(read_fd) };
            let mut buffer = Vec::new();
            reader
                .read_to_end(&mut buffer)
                .expect("failed to read from pipe");
            // Deserialize the result and return it.
            bincode::deserialize(&buffer).expect("deserialization failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct MyResult {
        value: i32,
    }

    #[test]
    fn simple_example() {
        let result = execute_in_isolated_process(|| MyResult { value: 42 });
        assert_eq!(result, MyResult { value: 42 });
    }

    // TODO: Prove a function which leaks memory (via Box::leak) results in more memory usage after execution without execute_in_isolated_process().

    // TODO: Prove that same leaky function results in zero additional memory usage after execution with execute_in_isolated_process().
}
