use serde::{de::DeserializeOwned, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;

/// Execute `callable` in a forked child process so that any memory changes during do not affect the parent.
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
    let mut pipe_fds: [i32; 2] = [0; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        panic!("pipe failed: {}", std::io::Error::last_os_error());
    }
    let read_fd = pipe_fds[0];
    let write_fd = pipe_fds[1];

    // Fork the process.
    let pid = unsafe { libc::fork() };
    match pid {
        // Fork failure
        -1 => panic!("fork failed: {}", std::io::Error::last_os_error()),
        // Child process
        0 => {
            // Close the read end of the pipe
            if unsafe { libc::close(read_fd) } != 0 {
                panic!(
                    "child: failed to close read end: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Execute the callable
            let result = callable();
            // Serialize the result using bincode
            let encoded = bincode::serialize(&result).expect("serialization failed");
            {
                // The write_fd will automatically be closed when the File is dropped
                let mut writer = unsafe { File::from_raw_fd(write_fd) };
                writer.write_all(&encoded).expect("failed to write to pipe");
                writer.flush().expect("failed to flush pipe");
            }
            // Exit immediately; use _exit to avoid running atexit()/on_exit() handlers
            // and flushing stdio buffers, which are exact clones of the parent in the child process.
            unsafe { libc::_exit(0) };
        }
        // Parent process
        _ => {
            // Close the write end of the pipe
            if unsafe { libc::close(write_fd) } != 0 {
                panic!(
                    "parent: failed to close write end: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Wait for the child process to exit
            let mut status: i32 = 0;
            if unsafe { libc::waitpid(pid, &mut status as *mut i32, 0) } < 0 {
                panic!("waitpid failed: {}", std::io::Error::last_os_error());
            }
            // Read from the pipe by wrapping the read fd as a File
            let mut buffer = Vec::new();
            {
                // The read_fd will automatically be closed when the File is dropped
                let mut reader = unsafe { File::from_raw_fd(read_fd) };
                reader
                    .read_to_end(&mut buffer)
                    .expect("failed to read from pipe");
            }
            // Deserialize the result and return it
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

    // TODO: Add test for memory leaks

    #[test]
    #[allow(static_mut_refs)]
    fn test_static_memory_mutation_without_isolation() {
        static mut MEMORY: bool = false;
        let mutate = || unsafe { MEMORY = true };

        // Directly modify static memory
        mutate();

        // Verify the change persists
        unsafe {
            assert!(MEMORY, "Static memory should be modified");
        }
    }

    #[test]
    #[allow(static_mut_refs)]
    fn test_static_memory_mutation_with_isolation() {
        static mut MEMORY: bool = false;
        let mutate = || unsafe { MEMORY = true };

        // Modify static memory in isolated process
        execute_in_isolated_process(mutate);

        // Verify the change does not affect parent process
        unsafe {
            assert!(
                !MEMORY,
                "Static memory should remain unmodified in parent process"
            );
        }
    }
}
