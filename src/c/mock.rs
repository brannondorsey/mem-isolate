use crate::c::{ForkReturn, PipeFds, RealSystemFunctions, SystemFunctions};
use libc::c_int;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::thread_local;

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
                queue.push_back(CallImplementation::Mock(MockResult::from_result(result)));
            }
        }
    }

    fn execute_next<F>(&self, real_impl: F, fallback_enabled: bool) -> Result<T, io::Error>
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

/// Mock implementation that returns predefined values and can fall back to real implementation
#[derive(Clone)]
pub struct MockableSystemFunctions {
    real_impl: RealSystemFunctions,
    fallback_enabled: Cell<bool>,
    fork_queue: CallQueue<ForkReturn>,
    pipe_queue: CallQueue<PipeFds>,
    close_queue: CallQueue<()>,
    waitpid_queue: CallQueue<c_int>,
}

impl Default for MockableSystemFunctions {
    fn default() -> Self {
        Self {
            real_impl: RealSystemFunctions,
            fallback_enabled: Cell::new(true), // Enable fallback by default
            fork_queue: CallQueue::new("fork"),
            pipe_queue: CallQueue::new("pipe"),
            close_queue: CallQueue::new("close"),
            waitpid_queue: CallQueue::new("waitpid"),
        }
    }
}

impl MockableSystemFunctions {
    /// Enable fallback to real implementations when no mock is configured
    pub fn with_fallback() -> Self {
        let mock = Self::default();
        mock.enable_fallback();
        mock
    }

    /// Disable fallback to real implementations (strict mocking mode)
    pub fn strict() -> Self {
        let mock = Self::default();
        mock.disable_fallback();
        mock
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
}

impl SystemFunctions for MockableSystemFunctions {
    fn fork(&self) -> Result<ForkReturn, io::Error> {
        self.fork_queue
            .execute_next(|| self.real_impl.fork(), self.is_fallback_enabled())
    }

    fn pipe(&self) -> Result<PipeFds, io::Error> {
        self.pipe_queue
            .execute_next(|| self.real_impl.pipe(), self.is_fallback_enabled())
    }

    fn close(&self, fd: c_int) -> Result<(), io::Error> {
        self.close_queue
            .execute_next(|| self.real_impl.close(fd), self.is_fallback_enabled())
    }

    fn waitpid(&self, pid: c_int) -> Result<c_int, io::Error> {
        self.waitpid_queue
            .execute_next(|| self.real_impl.waitpid(pid), self.is_fallback_enabled())
    }

    fn _exit(&self, status: c_int) -> ! {
        if is_mocking_enabled() {
            // In mock context, we panic instead of exiting
            panic!("_exit({status}) called in mock context");
        } else {
            // Otherwise, use the real implementation
            #[allow(clippy::used_underscore_items)]
            self.real_impl._exit(status)
        }
    }
}

// Thread-local storage for the current mocking state
thread_local! {
    static CURRENT_MOCK: RefCell<Option<MockableSystemFunctions>> = const { RefCell::new(None) };
    static IS_MOCKING_ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// Enable mocking for the current thread with the specified mock configuration
pub fn enable_mocking(mock: &MockableSystemFunctions) {
    CURRENT_MOCK.with(|m| {
        *m.borrow_mut() = Some(mock.clone());
    });
    IS_MOCKING_ENABLED.with(|e| e.set(true));
}

/// Disable mocking for the current thread
pub fn disable_mocking() {
    CURRENT_MOCK.with(|m| {
        *m.borrow_mut() = None;
    });
    IS_MOCKING_ENABLED.with(|e| e.set(false));
}

/// Returns true if mocking is currently enabled for the current thread
pub fn is_mocking_enabled() -> bool {
    IS_MOCKING_ENABLED.with(std::cell::Cell::get)
}

/// Get the current mock from thread-local storage
pub fn get_current_mock() -> MockableSystemFunctions {
    CURRENT_MOCK.with(|m| {
        m.borrow()
            .clone()
            .expect("No mock available in thread-local storage")
    })
}

/// Configuration options for the mock system
pub enum MockConfig {
    /// Use fallback mode (real implementations when no mock is configured)
    Fallback,

    /// Use strict mode (panic when no mock is configured)
    Strict,

    /// Configure the mock with a function, using fallback mode
    ConfiguredWithFallback(Box<dyn FnOnce(&MockableSystemFunctions)>),

    /// Configure the mock with a function, using strict mode
    ConfiguredStrict(Box<dyn FnOnce(&MockableSystemFunctions)>),
}

/// Set up mocking environment and execute a test function
pub fn with_mock_system<R>(
    config: MockConfig,
    test_fn: impl FnOnce(&MockableSystemFunctions) -> R,
) -> R {
    let mock = MockableSystemFunctions::with_fallback();

    // Configure based on the enum variant
    match config {
        MockConfig::Fallback => {}
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
    enable_mocking(&mock);

    // Run the test function with mocking enabled
    let result = test_fn(&mock);

    // Disable mocking
    disable_mocking();

    result
}

/// Helper to create a `ConfiguredWithFallback` variant
pub fn configured_with_fallback<F>(configure_fn: F) -> MockConfig
where
    F: FnOnce(&MockableSystemFunctions) + 'static,
{
    MockConfig::ConfiguredWithFallback(Box::new(configure_fn))
}

/// Helper to create a `ConfiguredStrict` variant
pub fn configured_strict<F>(configure_fn: F) -> MockConfig
where
    F: FnOnce(&MockableSystemFunctions) + 'static,
{
    MockConfig::ConfiguredStrict(Box::new(configure_fn))
}
