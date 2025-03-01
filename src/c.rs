use libc::c_int;
#[cfg(not(test))]
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkReturn {
    Parent(i32),
    Child,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeFds {
    pub read_fd: c_int,
    pub write_fd: c_int,
}

// For test builds, use our mocking framework
#[cfg(test)]
pub mod mock;

#[cfg(test)]
pub use mock::*;

// For non-test builds, use the real implementations directly
#[cfg(not(test))]
pub fn fork() -> Result<ForkReturn, io::Error> {
    const FORK_FAILED: i32 = -1;
    const FORK_CHILD: i32 = 0;

    let ret = unsafe { libc::fork() };
    match ret {
        FORK_FAILED => Err(io::Error::last_os_error()),
        FORK_CHILD => Ok(ForkReturn::Child),
        _ => {
            let child_pid = ret;
            Ok(ForkReturn::Parent(child_pid))
        }
    }
}

#[cfg(not(test))]
pub fn pipe() -> Result<PipeFds, io::Error> {
    let mut pipe_fds: [c_int; 2] = [0; 2];
    let ret = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(PipeFds {
            read_fd: pipe_fds[0],
            write_fd: pipe_fds[1],
        })
    }
}

#[cfg(not(test))]
pub fn close(fd: c_int) -> Result<(), io::Error> {
    let ret = unsafe { libc::close(fd) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
pub fn _exit(status: c_int) -> ! {
    unsafe { libc::_exit(status) };
}

#[cfg(not(test))]
pub fn waitpid(pid: c_int) -> Result<c_int, io::Error> {
    let mut status: c_int = 0;
    // TODO: Explore if any options are needed
    let ret = unsafe { libc::waitpid(pid, &mut status as *mut c_int, 0) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_mock_enabling_disabling() {
        // Start with mocking disabled
        assert!(!is_mocking_enabled());

        // Enable mocking and verify it's enabled
        let mock = MockSystemFunctions::new();
        enable_mocking(mock);
        assert!(is_mocking_enabled());

        // Disable mocking and verify it's disabled
        disable_mocking();
        assert!(!is_mocking_enabled());
    }

    #[test]
    fn test_with_mock_system_helper() {
        // Verify mocking is disabled before the block
        assert!(!is_mocking_enabled());

        // Use the helper function
        with_mock_system(|_mock| {
            // Verify mocking is enabled inside the block
            assert!(is_mocking_enabled());
        });

        // Verify mocking is disabled after the block
        assert!(!is_mocking_enabled());
    }

    #[test]
    fn test_fork_mocking() {
        let mock = MockSystemFunctions::new();
        // Set up parent process return
        mock.expect_fork(Ok(ForkReturn::Parent(123)));
        // Also test child process return
        mock.expect_fork(Ok(ForkReturn::Child));

        enable_mocking(mock);

        // First call should return Parent
        let result1 = fork().expect("Fork should succeed");
        assert!(matches!(result1, ForkReturn::Parent(123)));

        // Second call should return Child
        let result2 = fork().expect("Fork should succeed");
        assert!(matches!(result2, ForkReturn::Child));

        disable_mocking();
    }

    #[test]
    #[ignore] // TODO: Fix this test
    fn test_pipe_mocking() {
        // Use the with_mock_system helper to ensure proper cleanup
        with_mock_system(|mock| {
            mock.expect_pipe(Ok(PipeFds {
                read_fd: 10,
                write_fd: 11,
            }));

            // Add expectations for closing both file descriptors
            mock.expect_close(Ok(()));
            mock.expect_close(Ok(()));

            let pipe_fds = pipe().expect("Pipe should succeed");
            assert_eq!(pipe_fds.read_fd, 10);
            assert_eq!(pipe_fds.write_fd, 11);

            // Close the file descriptors to prevent the IO safety violation
            close(pipe_fds.read_fd).expect("Close should succeed");
            close(pipe_fds.write_fd).expect("Close should succeed");
        });
        // The with_mock_system helper will automatically disable mocking
    }

    #[test]
    fn test_close_mocking() {
        let mock = MockSystemFunctions::new();
        mock.expect_close(Ok(()));

        enable_mocking(mock);

        let result = close(999); // fd value doesn't matter for mocked call
        assert!(result.is_ok());

        disable_mocking();
    }

    #[test]
    fn test_waitpid_mocking() {
        let mock = MockSystemFunctions::new();
        mock.expect_waitpid(Ok(42)); // Mock an exit status of 42

        enable_mocking(mock);

        let status = waitpid(123).expect("Waitpid should succeed");
        assert_eq!(status, 42);

        disable_mocking();
    }

    #[test]
    fn test_error_conditions() {
        let mock = MockSystemFunctions::new();

        // Set up various error conditions
        mock.expect_fork(Err(io::Error::from_raw_os_error(libc::EAGAIN)));
        mock.expect_pipe(Err(io::Error::from_raw_os_error(libc::EMFILE)));
        mock.expect_close(Err(io::Error::from_raw_os_error(libc::EBADF)));
        mock.expect_waitpid(Err(io::Error::from_raw_os_error(libc::ECHILD)));

        enable_mocking(mock);

        // Test fork error
        let fork_result = fork();
        assert!(fork_result.is_err());
        assert_eq!(fork_result.unwrap_err().raw_os_error(), Some(libc::EAGAIN));

        // Test pipe error
        let pipe_result = pipe();
        assert!(pipe_result.is_err());
        assert_eq!(pipe_result.unwrap_err().raw_os_error(), Some(libc::EMFILE));

        // Test close error
        let close_result = close(1);
        assert!(close_result.is_err());
        assert_eq!(close_result.unwrap_err().raw_os_error(), Some(libc::EBADF));

        // Test waitpid error
        let waitpid_result = waitpid(1);
        assert!(waitpid_result.is_err());
        assert_eq!(
            waitpid_result.unwrap_err().raw_os_error(),
            Some(libc::ECHILD)
        );

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock result configured for fork()")]
    fn test_missing_fork_expectation() {
        let mock = MockSystemFunctions::new();
        mock.disable_fallback();
        enable_mocking(mock);

        // This should panic because we didn't set an expectation
        let _ = fork();

        // We won't reach this, but it's good practice to clean up
        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock result configured for pipe()")]
    fn test_missing_pipe_expectation() {
        let mock = MockSystemFunctions::new();
        mock.disable_fallback();
        enable_mocking(mock);

        let _ = pipe();

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock result configured for close()")]
    fn test_missing_close_expectation() {
        let mock = MockSystemFunctions::new();
        mock.disable_fallback();
        enable_mocking(mock);

        let _ = close(1);

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock result configured for waitpid()")]
    fn test_missing_waitpid_expectation() {
        let mock = MockSystemFunctions::new();
        mock.disable_fallback();
        enable_mocking(mock);

        let _ = waitpid(1);

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "_exit(0) called in mock context")]
    fn test_exit_in_mock_context() {
        let mock = MockSystemFunctions::new();
        mock.disable_fallback();
        enable_mocking(mock);

        // This should panic with a specific message
        _exit(0);

        // WARNING: No disable_mocking() here because its unreachable
    }

    #[test]
    fn test_multiple_expectations() {
        let mock = MockSystemFunctions::new();

        // Set up a sequence of expectations
        mock.expect_fork(Ok(ForkReturn::Parent(1)));
        mock.expect_fork(Ok(ForkReturn::Parent(2)));
        mock.expect_fork(Ok(ForkReturn::Parent(3)));

        enable_mocking(mock);

        // Verify calls are processed in order
        assert!(matches!(fork().unwrap(), ForkReturn::Parent(1)));
        assert!(matches!(fork().unwrap(), ForkReturn::Parent(2)));
        assert!(matches!(fork().unwrap(), ForkReturn::Parent(3)));

        disable_mocking();
    }

    #[test]
    fn test_non_os_error_handling() {
        // Create a special io::Error that's not an OS error
        let custom_error = io::Error::new(io::ErrorKind::Other, "Custom error");

        let mock = MockSystemFunctions::new();

        // For MockResult::from_result, this should convert to EIO
        mock.expect_fork(Err(custom_error));

        enable_mocking(mock);

        let result = fork();
        assert!(result.is_err());
        // Should be converted to EIO
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EIO));

        disable_mocking();
    }
}
