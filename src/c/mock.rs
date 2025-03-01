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

// Type for representing a mock result
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

/// Defines how a call should be implemented - real or mocked
#[derive(Clone)]
enum CallImplementation<T> {
    Real,                // Use real implementation
    Mock(MockResult<T>), // Use mocked result
}

/// Public API for specifying call behavior
pub enum CallBehavior<T> {
    Real,                       // Use the real system implementation
    Mock(Result<T, io::Error>), // Use a mock result
}

/// A generic queue of call implementations
#[derive(Clone)]
struct CallQueue<T> {
    queue: RefCell<VecDeque<CallImplementation<T>>>,
    name: &'static str, // For better error messages
}

impl<T: Clone> CallQueue<T> {
    fn new(name: &'static str) -> Self {
        Self {
            queue: RefCell::new(VecDeque::new()),
            name,
        }
    }

    fn push(&self, behavior: CallBehavior<T>) {
        let mut queue = self.queue.borrow_mut();
        match behavior {
            CallBehavior::Real => queue.push_back(CallImplementation::Real),
            CallBehavior::Mock(result) => {
                queue.push_back(CallImplementation::Mock(MockResult::from_result(result)))
            }
        }
    }

    fn next<F>(&self, real_impl: F, fallback_enabled: bool) -> Result<T, io::Error>
    where
        F: FnOnce() -> Result<T, io::Error>,
    {
        // Get explicit reference to make the borrow checker happy
        let mut queue = self.queue.borrow_mut();
        match queue.pop_front() {
            Some(CallImplementation::Real) => real_impl(),
            Some(CallImplementation::Mock(result)) => result.to_result(),
            None if fallback_enabled => real_impl(),
            None => panic!(
                "No mock behavior configured for {}() and fallback is disabled",
                self.name
            ),
        }
    }
}

/// Mock implementation that returns predefined values
#[derive(Clone)]
pub struct MockSystemFunctions {
    fallback_enabled: Cell<bool>,
    fork_queue: CallQueue<ForkReturn>,
    pipe_queue: CallQueue<PipeFds>,
    close_queue: CallQueue<()>,
    waitpid_queue: CallQueue<c_int>,
}

impl Default for MockSystemFunctions {
    fn default() -> Self {
        Self {
            fallback_enabled: Cell::new(true), // Enable fallback by default
            fork_queue: CallQueue::new("fork"),
            pipe_queue: CallQueue::new("pipe"),
            close_queue: CallQueue::new("close"),
            waitpid_queue: CallQueue::new("waitpid"),
        }
    }
}

impl MockSystemFunctions {
    pub fn new() -> Self {
        Self::default()
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

    // Generic methods for specifying behavior

    pub fn expect_fork(&self, behavior: CallBehavior<ForkReturn>) -> &Self {
        self.fork_queue.push(behavior);
        self
    }

    pub fn expect_pipe(&self, behavior: CallBehavior<PipeFds>) -> &Self {
        self.pipe_queue.push(behavior);
        self
    }

    pub fn expect_close(&self, behavior: CallBehavior<()>) -> &Self {
        self.close_queue.push(behavior);
        self
    }

    pub fn expect_waitpid(&self, behavior: CallBehavior<c_int>) -> &Self {
        self.waitpid_queue.push(behavior);
        self
    }

    // Convenience methods for better readability

    pub fn expect_real_fork(&self) -> &Self {
        self.expect_fork(CallBehavior::Real)
    }

    pub fn expect_mock_fork(&self, result: Result<ForkReturn, io::Error>) -> &Self {
        self.expect_fork(CallBehavior::Mock(result))
    }

    pub fn expect_real_pipe(&self) -> &Self {
        self.expect_pipe(CallBehavior::Real)
    }

    pub fn expect_mock_pipe(&self, result: Result<PipeFds, io::Error>) -> &Self {
        self.expect_pipe(CallBehavior::Mock(result))
    }

    pub fn expect_real_close(&self) -> &Self {
        self.expect_close(CallBehavior::Real)
    }

    pub fn expect_mock_close(&self, result: Result<(), io::Error>) -> &Self {
        self.expect_close(CallBehavior::Mock(result))
    }

    pub fn expect_real_waitpid(&self) -> &Self {
        self.expect_waitpid(CallBehavior::Real)
    }

    pub fn expect_mock_waitpid(&self, result: Result<c_int, io::Error>) -> &Self {
        self.expect_waitpid(CallBehavior::Mock(result))
    }
}

impl SystemFunctions for MockSystemFunctions {
    fn fork(&self) -> Result<ForkReturn, io::Error> {
        self.fork_queue
            .next(|| RealSystemFunctions.fork(), self.is_fallback_enabled())
    }

    fn pipe(&self) -> Result<PipeFds, io::Error> {
        self.pipe_queue
            .next(|| RealSystemFunctions.pipe(), self.is_fallback_enabled())
    }

    fn close(&self, fd: c_int) -> Result<(), io::Error> {
        self.close_queue
            .next(|| RealSystemFunctions.close(fd), self.is_fallback_enabled())
    }

    fn waitpid(&self, pid: c_int) -> Result<c_int, io::Error> {
        self.waitpid_queue.next(
            || RealSystemFunctions.waitpid(pid),
            self.is_fallback_enabled(),
        )
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

/// Configuration options for the mock system
pub enum MockConfig {
    /// Use fallback mode (real implementations when no mock is configured)
    Fallback,

    /// Use strict mode (panic when no mock is configured)
    Strict,

    /// Configure the mock with a function, using fallback mode
    ConfiguredWithFallback(Box<dyn FnOnce(&MockSystemFunctions)>),

    /// Configure the mock with a function, using strict mode
    ConfiguredStrict(Box<dyn FnOnce(&MockSystemFunctions)>),
}

/// Set up mocking environment and execute a test function
pub fn with_mock_system<R>(config: MockConfig, test_fn: impl FnOnce() -> R) -> R {
    let mock = MockSystemFunctions::new();

    // Configure based on the enum variant
    match config {
        MockConfig::Fallback => {
            // Use defaults (fallback enabled)
        }
        MockConfig::Strict => {
            mock.disable_fallback();
        }
        MockConfig::ConfiguredWithFallback(configure_fn) => {
            configure_fn(&mock);
        }
        MockConfig::ConfiguredStrict(configure_fn) => {
            mock.disable_fallback();
            configure_fn(&mock);
        }
    }

    // Enable mocking with the configured mock
    enable_mocking(mock);

    // Run the test function with mocking enabled
    let result = test_fn();

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

/// Helper to create a ConfiguredWithFallback variant
pub fn configured_with_fallback<F>(configure_fn: F) -> MockConfig
where
    F: FnOnce(&MockSystemFunctions) + 'static,
{
    MockConfig::ConfiguredWithFallback(Box::new(configure_fn))
}

/// Helper to create a ConfiguredStrict variant
pub fn configured_strict<F>(configure_fn: F) -> MockConfig
where
    F: FnOnce(&MockSystemFunctions) + 'static,
{
    MockConfig::ConfiguredStrict(Box::new(configure_fn))
}
