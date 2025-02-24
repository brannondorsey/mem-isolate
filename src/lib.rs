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
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct MyResult {
        value: i32,
    }

    unsafe fn force_arena_trim() {
        unsafe {
            libc::malloc_trim(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        unsafe {
            libc::malloc_trim(0);
        }
    }

    fn get_rss() -> Option<usize> {
        // Read the process status file.
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        // Look for the line starting with "VmRSS:"
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                // Example line: "VmRSS:	  123456 kB"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<usize>() {
                        // Convert kilobytes to bytes.
                        return Some(kb * 1024);
                    }
                }
            }
        }
        None
    }

    use std::fs::read_to_string;

    /// Returns the total amount of private (active) memory in bytes by summing
    /// the "Private_Clean" and "Private_Dirty" fields from /proc/self/smaps.
    fn get_active_memory() -> Option<usize> {
        let smaps = read_to_string("/proc/self/smaps").ok()?;
        let mut total = 0;
        for line in smaps.lines() {
            if line.starts_with("Private_Clean:") || line.starts_with("Private_Dirty:") {
                // Example line: "Private_Clean:     1024 kB"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<usize>() {
                        total += kb * 1024;
                    }
                }
            }
        }
        Some(total)
    }

    fn get_heap_allocated() -> usize {
        unsafe {
            // mallinfo returns a struct mallinfo.
            let info = libc::mallinfo();
            // uordblks is the total allocated space (in bytes) in the heap.
            info.uordblks as usize
        }
    }

    fn leak_memory() -> MyResult {
        // Leak 50MB of memory
        let mut leaked_memory = 0i32;
        for _ in 0..50 {
            let leak_size = 1024 * 1024; // 1MB
            let mut leak = Box::new(vec![0u8; leak_size]);
            // Touch one byte per 4KB page to force allocation
            // TODO: Get page size from the system
            for i in (0..leak_size).step_by(4096) {
                leak[i] = 1;
            }
            Box::leak(leak);
            leaked_memory += leak_size as i32;
        }
        MyResult {
            value: leaked_memory,
        }
    }

    #[test]
    fn simple_example() {
        let result = execute_in_isolated_process(|| MyResult { value: 42 });
        assert_eq!(result, MyResult { value: 42 });
    }

    // #[test]
    // fn test_memory_leak_without_isolation() {
    //     let initial_memory = get_rss().unwrap_or(0);

    //     // Run the leaky function directly
    //     leak_memory();
    //     unsafe {
    //         force_arena_trim();
    //     }

    //     let final_memory = get_rss().unwrap_or(0);
    //     let diff = final_memory.saturating_sub(initial_memory);

    //     // Virtual memory usage should have increased by roughly 50MB (allowing some variance)
    //     assert!(
    //         diff > 45_000_000,
    //         "Memory leak not detected: diff = {} bytes",
    //         diff
    //     );
    // }

    // #[test]
    // fn test_memory_leak_with_isolation() {
    //     let initial_memory = get_rss().unwrap_or(0);

    //     // Run the leaky function in isolated process
    //     execute_in_isolated_process(leak_memory);
    //     unsafe {
    //         force_arena_trim();
    //     }

    //     let final_memory = get_rss().unwrap_or(0);
    //     let diff = final_memory.saturating_sub(initial_memory);

    //     // Virtual memory difference should be minimal (allowing for some small variance)
    //     assert!(
    //         diff < 45_000_000,
    //         "Unexpected memory growth: diff = {} bytes",
    //         diff
    //     );
    // }

    #[test]
    #[allow(static_mut_refs)]
    fn test_static_memory_mutation_without_isolation() {
        static mut COUNTER: u32 = 0;
        let mutate = || unsafe { COUNTER = 42 };

        // Directly modify static memory
        mutate();

        // Verify the change persists
        unsafe {
            assert_eq!(COUNTER, 42, "Static memory should be modified");
        }
    }

    #[test]
    #[allow(static_mut_refs)]
    fn test_static_memory_mutation_with_isolation() {
        static mut COUNTER: u32 = 0;
        let mutate = || unsafe { COUNTER = 42 };

        // Modify static memory in isolated process
        execute_in_isolated_process(mutate);

        // Verify the change does not affect parent process
        unsafe {
            assert_eq!(
                COUNTER, 0,
                "Static memory should remain unmodified in parent process"
            );
        }
    }
}
