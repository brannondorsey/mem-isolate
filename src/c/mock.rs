use crate::c::{ForkReturn, PipeFds};
use libc::c_int;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::thread_local;

/// Trait defining all system functions that can be mocked
pub trait SystemFunctions {
    fn fork(&self) -> Result<ForkReturn, io::Error>;
    fn pipe(&self) -> Result<PipeFds, io::Error>;
    fn close(&self, fd: c_int) -> Result<(), io::Error>;
    fn waitpid(&self, pid: c_int) -> Result<c_int, io::Error>;
    // Note: _exit is not included as it's a ! function and would complicate testing
}

/// Real implementation that delegates to actual system calls
pub struct RealSystemFunctions;

impl SystemFunctions for RealSystemFunctions {
    fn fork(&self) -> Result<ForkReturn, io::Error> {
        // Call the actual implementation
        let ret = unsafe { libc::fork() };
        match ret {
            -1 => Err(io::Error::last_os_error()),
            0 => Ok(ForkReturn::Child),
            _ => Ok(ForkReturn::Parent(ret)),
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
        let ret = unsafe { libc::waitpid(pid, &mut status as *mut c_int, 0) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(status)
        }
    }
}

// Type for representing the mock result
#[derive(Clone)]
enum MockResult<T> {
    Ok(T),
    Err(i32), // OS error code
}

impl<T> MockResult<T> {
    #[allow(clippy::wrong_self_convention)]
    fn to_result(self) -> Result<T, io::Error> {
        match self {
            MockResult::Ok(val) => Ok(val),
            MockResult::Err(code) => Err(io::Error::from_raw_os_error(code)),
        }
    }

    fn from_result(result: Result<T, io::Error>) -> Self {
        match result {
            Ok(val) => MockResult::Ok(val),
            Err(err) => {
                if let Some(code) = err.raw_os_error() {
                    MockResult::Err(code)
                } else {
                    // Default to a generic error code if it's not an OS error
                    MockResult::Err(libc::EIO) // I/O error
                }
            }
        }
    }
}

/// Mock implementation that returns predefined values
#[derive(Default, Clone)]
pub struct MockSystemFunctions {
    // Add a fallback mode flag
    fallback_enabled: Cell<bool>,
    fork_results: RefCell<VecDeque<MockResult<ForkReturn>>>,
    pipe_results: RefCell<VecDeque<MockResult<PipeFds>>>,
    close_results: RefCell<VecDeque<MockResult<()>>>,
    waitpid_results: RefCell<VecDeque<MockResult<c_int>>>,
}

impl MockSystemFunctions {
    pub fn new() -> Self {
        let instance = Self::default();
        // Enable fallback by default for a better user experience
        instance.enable_fallback();
        instance
    }

    /// Enable fallback to real implementations when no mock is configured
    pub fn enable_fallback(&self) -> &Self {
        self.fallback_enabled.set(true);
        self
    }

    /// Disable fallback to real implementations (strict mocking mode)
    pub fn disable_fallback(&self) -> &Self {
        self.fallback_enabled.set(false);
        self
    }

    /// Check if fallback is enabled
    pub fn is_fallback_enabled(&self) -> bool {
        self.fallback_enabled.get()
    }

    pub fn expect_fork(&self, result: Result<ForkReturn, io::Error>) -> &Self {
        self.fork_results
            .borrow_mut()
            .push_back(MockResult::from_result(result));
        self
    }

    pub fn expect_pipe(&self, result: Result<PipeFds, io::Error>) -> &Self {
        self.pipe_results
            .borrow_mut()
            .push_back(MockResult::from_result(result));
        self
    }

    pub fn expect_close(&self, result: Result<(), io::Error>) -> &Self {
        self.close_results
            .borrow_mut()
            .push_back(MockResult::from_result(result));
        self
    }

    pub fn expect_waitpid(&self, result: Result<c_int, io::Error>) -> &Self {
        self.waitpid_results
            .borrow_mut()
            .push_back(MockResult::from_result(result));
        self
    }
}

// TODO: Make these generic
impl SystemFunctions for MockSystemFunctions {
    fn fork(&self) -> Result<ForkReturn, io::Error> {
        match self.fork_results.borrow_mut().pop_front() {
            Some(result) => result.to_result(),
            None if self.is_fallback_enabled() => {
                // Fall back to real implementation when fallback is enabled
                RealSystemFunctions.fork()
            }
            None => {
                // Strict mode - panic when no mock is configured
                panic!("No mock result configured for fork() and fallback is disabled")
            }
        }
    }

    fn pipe(&self) -> Result<PipeFds, io::Error> {
        match self.pipe_results.borrow_mut().pop_front() {
            Some(result) => result.to_result(),
            None if self.is_fallback_enabled() => {
                // Fall back to real implementation
                RealSystemFunctions.pipe()
            }
            None => {
                panic!("No mock result configured for pipe() and fallback is disabled")
            }
        }
    }

    fn close(&self, fd: c_int) -> Result<(), io::Error> {
        match self.close_results.borrow_mut().pop_front() {
            Some(result) => result.to_result(),
            None if self.is_fallback_enabled() => {
                // Fall back to real implementation
                RealSystemFunctions.close(fd)
            }
            None => {
                panic!("No mock result configured for close() and fallback is disabled")
            }
        }
    }

    fn waitpid(&self, pid: c_int) -> Result<c_int, io::Error> {
        match self.waitpid_results.borrow_mut().pop_front() {
            Some(result) => result.to_result(),
            None if self.is_fallback_enabled() => {
                // Fall back to real implementation
                RealSystemFunctions.waitpid(pid)
            }
            None => {
                panic!("No mock result configured for waitpid() and fallback is disabled")
            }
        }
    }
}

// Thread-local storage for the current system functions implementation and mocking state
thread_local! {
    static SYSTEM_FUNCTIONS: RefCell<Box<dyn SystemFunctions>> = RefCell::new(Box::new(RealSystemFunctions));
    static IS_MOCKING: Cell<bool> = Cell::new(false);
}

/// Enable mocking for the current thread with the specified mock configuration
pub fn enable_mocking(mock: MockSystemFunctions) {
    SYSTEM_FUNCTIONS.with(|f| {
        *f.borrow_mut() = Box::new(mock);
    });
    IS_MOCKING.with(|m| m.set(true));
}

/// Disable mocking for the current thread and restore real system calls
pub fn disable_mocking() {
    SYSTEM_FUNCTIONS.with(|f| {
        *f.borrow_mut() = Box::new(RealSystemFunctions);
    });
    IS_MOCKING.with(|m| m.set(false));
}

/// Returns true if mocking is currently enabled for the current thread
pub fn is_mocking_enabled() -> bool {
    IS_MOCKING.with(|m| m.get())
}

/// Set up mocking environment and execute a test function with the configured mock
pub fn with_mock_system<F, R>(test_fn: F) -> R
where
    F: FnOnce(&MockSystemFunctions) -> R,
{
    let mock = MockSystemFunctions::new();

    // Enable mocking with our mock
    enable_mocking(mock.clone());

    // Run the test
    let result = test_fn(&mock);

    // Disable mocking
    disable_mocking();

    result
}

// Helper functions that delegate to the current implementation
pub fn fork() -> Result<ForkReturn, io::Error> {
    SYSTEM_FUNCTIONS.with(|f| f.borrow().fork())
}

pub fn pipe() -> Result<PipeFds, io::Error> {
    SYSTEM_FUNCTIONS.with(|f| f.borrow().pipe())
}

pub fn close(fd: c_int) -> Result<(), io::Error> {
    SYSTEM_FUNCTIONS.with(|f| f.borrow().close(fd))
}

pub fn waitpid(pid: c_int) -> Result<c_int, io::Error> {
    SYSTEM_FUNCTIONS.with(|f| f.borrow().waitpid(pid))
}

// Special case for _exit since it's a ! function
pub fn _exit(status: c_int) -> ! {
    // Check if we're in a mocking context
    let is_mocking = IS_MOCKING.with(|m| m.get());

    if is_mocking {
        // In a mock context, we panic instead of exiting
        panic!("_exit({}) called in mock context", status);
    } else {
        // In real context, call the actual _exit
        unsafe { libc::_exit(status) }
    }
}

// Add a helper function to easily create a mock with strict behavior
pub fn strict_mock() -> MockSystemFunctions {
    let mock = MockSystemFunctions::new();
    mock.disable_fallback();
    mock
}
