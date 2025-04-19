use libc::c_int;
use std::io;

// For test builds, use our mocking framework
#[cfg(test)]
pub mod mock;

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

// Define the core trait for system functions
pub trait SystemFunctions: std::fmt::Debug {
    fn fork(&self) -> Result<ForkReturn, io::Error>;
    fn pipe(&self) -> Result<PipeFds, io::Error>;
    fn close(&self, fd: c_int) -> Result<(), io::Error>;
    fn waitpid(&self, pid: c_int) -> Result<c_int, io::Error>;
    fn _exit(&self, status: c_int) -> !;
}

// The real implementation that calls system functions directly
#[derive(Debug, Clone)]
pub struct RealSystemFunctions;

impl SystemFunctions for RealSystemFunctions {
    fn fork(&self) -> Result<ForkReturn, io::Error> {
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

    fn pipe(&self) -> Result<PipeFds, io::Error> {
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

    fn close(&self, fd: c_int) -> Result<(), io::Error> {
        let ret = unsafe { libc::close(fd) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn waitpid(&self, pid: c_int) -> Result<c_int, io::Error> {
        let mut status: c_int = 0;
        let ret = unsafe { libc::waitpid(pid, &raw mut status, 0) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(status)
        }
    }

    fn _exit(&self, status: c_int) -> ! {
        unsafe { libc::_exit(status) }
    }
}

pub type WaitpidStatus = libc::c_int;
pub type ExitStatus = libc::c_int;
pub type Signal = libc::c_int;

#[inline]
pub fn child_process_exited_on_its_own(waitpid_status: WaitpidStatus) -> Option<ExitStatus> {
    if libc::WIFEXITED(waitpid_status) {
        Some(libc::WEXITSTATUS(waitpid_status))
    } else {
        None
    }
}

#[inline]
pub fn child_process_killed_by_signal(waitpid_status: WaitpidStatus) -> Option<Signal> {
    if libc::WIFSIGNALED(waitpid_status) {
        Some(libc::WTERMSIG(waitpid_status))
    } else {
        None
    }
}

// For test builds, these functions will be provided by the mock module
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::tests::TEST_TIMEOUT;
    use mock::*;
    use rstest::*;
    use std::io;

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    fn mock_enabling_disabling() {
        assert!(!is_mocking_enabled());

        let mock = MockableSystemFunctions::with_fallback();
        enable_mocking(&mock);
        assert!(is_mocking_enabled());

        disable_mocking();
        assert!(!is_mocking_enabled());
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]

    fn with_mock_system_helper() {
        assert!(!is_mocking_enabled());
        with_mock_system(MockConfig::Fallback, |_| {
            assert!(is_mocking_enabled());
        });
        assert!(!is_mocking_enabled());
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    #[should_panic(expected = "No mock behavior configured for fork() and fallback is disabled")]
    fn stict_mocking_panics_when_no_mock_is_configured() {
        let mock = MockableSystemFunctions::strict();
        enable_mocking(&mock);
        let _ = mock.fork();
        disable_mocking();
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    #[should_panic(expected = "No mock behavior configured for fork() and fallback is disabled")]
    fn strict_mocking_panics_when_no_mock_is_configured_with_system_helper() {
        assert!(!is_mocking_enabled());
        with_mock_system(MockConfig::Strict, |mock| {
            let _ = mock.fork();
        });
        assert!(!is_mocking_enabled());
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    #[should_panic(expected = "No mock behavior configured for fork() and fallback is disabled")]
    fn strict_mocking_panics_when_no_mock_is_configured_with_system_helper_configured_strict() {
        assert!(!is_mocking_enabled());
        with_mock_system(configured_strict(|_| {}), |mock| {
            let _ = mock.fork();
        });
        assert!(!is_mocking_enabled());
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    fn fork_mocking() {
        use CallBehavior::Mock;

        let mock = MockableSystemFunctions::strict();
        mock.expect_fork(Mock(Ok(ForkReturn::Parent(123))));
        mock.expect_fork(Mock(Ok(ForkReturn::Child)));

        enable_mocking(&mock);

        let result1 = mock.fork().expect("Fork should succeed");
        assert!(matches!(result1, ForkReturn::Parent(123)));
        let result2 = mock.fork().expect("Fork should succeed");
        assert!(matches!(result2, ForkReturn::Child));

        disable_mocking();
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    fn pipe_mocking() {
        with_mock_system(
            configured_strict(|mock| {
                mock.expect_pipe(CallBehavior::Mock(Ok(PipeFds {
                    read_fd: 1000,
                    write_fd: 1001,
                })))
                .expect_close(CallBehavior::Mock(Ok(())))
                .expect_close(CallBehavior::Mock(Ok(())));
            }),
            |mock| {
                let pipe_fds = mock.pipe().expect("Pipe should succeed");

                assert_eq!(pipe_fds.read_fd, 1000);
                assert_eq!(pipe_fds.write_fd, 1001);

                mock.close(pipe_fds.read_fd).expect("Close should succeed");
                mock.close(pipe_fds.write_fd).expect("Close should succeed");
            },
        );
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    fn close_mocking() {
        let mock = MockableSystemFunctions::strict();
        mock.expect_close(CallBehavior::Mock(Ok(())));

        enable_mocking(&mock);

        let result = mock.close(999);
        assert!(result.is_ok());

        disable_mocking();
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    fn waitpid_mocking() {
        let mock = MockableSystemFunctions::strict();
        mock.expect_waitpid(CallBehavior::Mock(Ok(42)));

        enable_mocking(&mock);

        let status = mock.waitpid(123).expect("Waitpid should succeed");
        assert_eq!(status, 42);

        disable_mocking();
    }

    #[rstest]
    #[timeout(TEST_TIMEOUT)]
    fn error_conditions() {
        use CallBehavior::Mock;

        let mock = MockableSystemFunctions::strict();

        // Set up various error conditions
        mock.expect_fork(Mock(Err(io::Error::from_raw_os_error(libc::EAGAIN))));
        mock.expect_pipe(Mock(Err(io::Error::from_raw_os_error(libc::EMFILE))));
        mock.expect_close(Mock(Err(io::Error::from_raw_os_error(libc::EBADF))));
        mock.expect_waitpid(Mock(Err(io::Error::from_raw_os_error(libc::ECHILD))));

        enable_mocking(&mock);

        // Test fork error
        let fork_result = mock.fork();
        assert!(fork_result.is_err());
        assert_eq!(fork_result.unwrap_err().raw_os_error(), Some(libc::EAGAIN));

        // Test pipe error
        let pipe_result = mock.pipe();
        assert!(pipe_result.is_err());
        assert_eq!(pipe_result.unwrap_err().raw_os_error(), Some(libc::EMFILE));

        // Test close error
        let close_result = mock.close(1);
        assert!(close_result.is_err());
        assert_eq!(close_result.unwrap_err().raw_os_error(), Some(libc::EBADF));

        // Test waitpid error
        let waitpid_result = mock.waitpid(1);
        assert!(waitpid_result.is_err());
        assert_eq!(
            waitpid_result.unwrap_err().raw_os_error(),
            Some(libc::ECHILD)
        );

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock behavior configured for fork()")]
    fn missing_fork_expectation() {
        let mock = MockableSystemFunctions::with_fallback();
        mock.disable_fallback();
        enable_mocking(&mock);

        // This should panic because we didn't set an expectation
        let _ = mock.fork();

        // We won't reach this, but it's good practice to clean up
        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock behavior configured for pipe()")]
    fn missing_pipe_expectation() {
        let mock = MockableSystemFunctions::with_fallback();
        mock.disable_fallback();
        enable_mocking(&mock);

        let _ = mock.pipe();

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock behavior configured for close()")]
    fn missing_close_expectation() {
        let mock = MockableSystemFunctions::with_fallback();
        mock.disable_fallback();
        enable_mocking(&mock);

        let _ = mock.close(1);

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "No mock behavior configured for waitpid()")]
    fn missing_waitpid_expectation() {
        let mock = MockableSystemFunctions::with_fallback();
        mock.disable_fallback();
        enable_mocking(&mock);

        let _ = mock.waitpid(1);

        disable_mocking();
    }

    #[test]
    #[should_panic(expected = "_exit(0) called in mock context")]
    fn exit_in_mock_context() {
        let mock = MockableSystemFunctions::with_fallback();
        mock.disable_fallback();
        enable_mocking(&mock);

        // This should panic with a specific message
        #[allow(clippy::used_underscore_items)]
        mock._exit(0);

        // WARNING: No disable_mocking() here because its unreachable
    }

    #[test]
    fn multiple_expectations() {
        let mock = MockableSystemFunctions::with_fallback();

        // Set up a sequence of expectations
        mock.expect_fork(CallBehavior::Mock(Ok(ForkReturn::Parent(1))))
            .expect_fork(CallBehavior::Mock(Ok(ForkReturn::Parent(2))))
            .expect_fork(CallBehavior::Mock(Ok(ForkReturn::Parent(3))));

        enable_mocking(&mock);

        // Verify calls are processed in order
        assert!(matches!(mock.fork().unwrap(), ForkReturn::Parent(1)));
        assert!(matches!(mock.fork().unwrap(), ForkReturn::Parent(2)));
        assert!(matches!(mock.fork().unwrap(), ForkReturn::Parent(3)));

        disable_mocking();
    }

    #[test]
    fn non_os_error_handling() {
        // Create a special io::Error that's not an OS error
        let custom_error = io::Error::new(io::ErrorKind::Other, "Custom error");

        let mock = MockableSystemFunctions::with_fallback();

        // For MockResult::from_result, this should convert to EIO
        mock.expect_fork(CallBehavior::Mock(Err(custom_error)));

        enable_mocking(&mock);

        let result = mock.fork();
        assert!(result.is_err());
        // Should be converted to EIO
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EIO));

        disable_mocking();
    }

    #[test]
    fn mixed_real_and_mock_calls() {
        with_mock_system(
            configured_with_fallback(|mock| {
                // Configure a mix of real and mock calls
                mock.expect_fork(CallBehavior::Real)
                    .expect_fork(CallBehavior::Mock(Ok(ForkReturn::Parent(123))))
                    .expect_fork(CallBehavior::Real);
            }),
            |sys| {
                // Test function with the configured mock
                println!("First call: Real implementation");
                let _result1 = sys.fork();

                println!("Second call: Mock implementation");
                let result2 = sys.fork().expect("Mock fork should succeed");
                assert_eq!(result2, ForkReturn::Parent(123));

                println!("Third call: Real implementation");
                let _result3 = sys.fork();
            },
        );
    }
}
