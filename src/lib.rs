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
                let mut writer = unsafe { File::from_raw_fd(write_fd) };
                writer.write_all(&encoded).expect("failed to write to pipe");
                writer.flush().expect("failed to flush pipe");
            }
            // TODO: Close the write_fd
            // Exit immediately; use _exit to avoid running destructors
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
                let mut reader = unsafe { File::from_raw_fd(read_fd) };
                reader
                    .read_to_end(&mut buffer)
                    .expect("failed to read from pipe");
            }

            // TODO: Close the read_fd
            // Deserialize the result and return it
            bincode::deserialize(&buffer).expect("deserialization failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory_stats::memory_stats;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct MyResult {
        value: i32,
    }

    fn get_memory_usage() -> usize {
        memory_stats().map(|stats| stats.virtual_mem).unwrap_or(0)
    }

    fn leak_memory() -> MyResult {
        // Leak 50MB of memory
        for _ in 0..50 {
            let leak = Box::new(vec![0u8; 1024 * 1024]); // 1MB
            Box::leak(leak);
        }
        MyResult { value: 42 }
    }

    #[test]
    fn simple_example() {
        let result = execute_in_isolated_process(|| MyResult { value: 42 });
        assert_eq!(result, MyResult { value: 42 });
    }

    #[test]
    fn test_memory_leak_without_isolation() {
        let initial_memory = get_memory_usage();

        // Run the leaky function directly
        leak_memory();

        let final_memory = get_memory_usage();
        let diff = final_memory - initial_memory;

        // Virtual memory usage should have increased by roughly 50MB (allowing some variance)
        assert!(
            diff > 45_000_000,
            "Memory leak not detected: diff = {} bytes",
            diff
        );
    }

    #[test]
    fn test_memory_leak_with_isolation() {
        let initial_memory = get_memory_usage();

        // Run the leaky function in isolated process
        execute_in_isolated_process(leak_memory);
        // TODO: Remove me
        std::thread::sleep(std::time::Duration::from_millis(100));

        let final_memory = get_memory_usage();
        let diff = final_memory.saturating_sub(initial_memory);

        // Virtual memory difference should be minimal (allowing for some small variance)
        assert!(
            diff < 45_000_000,
            "Unexpected memory growth: diff = {} bytes",
            diff
        );
    }
}
