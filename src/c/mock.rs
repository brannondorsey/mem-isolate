use crate::c::{ForkReturn, PipeFds, RealSystemFunctions, SystemFunctions};
use libc::c_int;
use std::collections::VecDeque;
use std::io;
use std::sync::{
    RwLock,
    atomic::{AtomicBool, Ordering},
};

// Type for representing a mock result
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
enum CallImplementation<T> {
    Real,                // Use real implementation
    Mock(MockResult<T>), // Use mocked result
}

/// Public API for specifying call behavior
pub enum CallBehavior<T> {
    Real,                       // Use the real system implementation
    Mock(Result<T, io::Error>), // Use a mock result
}

/// A generic queue of call implementations, now thread-safe
#[derive(Debug)]
struct CallQueue<T> {
    queue: RwLock<VecDeque<CallImplementation<T>>>,
    name: &'static str, // For better error messages
}

impl<T: Clone> CallQueue<T> {
    fn new(name: &'static str) -> Self {
        Self {
            queue: RwLock::new(VecDeque::new()),
            name,
        }
    }

    fn push(&self, behavior: CallBehavior<T>) {
        let mut queue = self.queue.write().expect("Failed to acquire write lock");
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
        // Get writable reference to make changes
        let mut queue = self.queue.write().expect("Failed to acquire write lock");
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
/// Now implements Send + Sync for thread safety
#[derive(Debug)]
pub struct MockableSystemFunctions {
    real_impl: RealSystemFunctions,
    fallback_enabled: AtomicBool,
    fork_queue: CallQueue<ForkReturn>,
    pipe_queue: CallQueue<PipeFds>,
    close_queue: CallQueue<()>,
    waitpid_queue: CallQueue<c_int>,
}

impl Clone for MockableSystemFunctions {
    fn clone(&self) -> Self {
        // Create a fresh instance rather than trying to clone the locks
        // This is cleaner than trying to deep-clone the RwLocks
        Self {
            real_impl: self.real_impl.clone(),
            fallback_enabled: AtomicBool::new(self.fallback_enabled.load(Ordering::Relaxed)),
            fork_queue: CallQueue::new("fork"),
            pipe_queue: CallQueue::new("pipe"),
            close_queue: CallQueue::new("close"),
            waitpid_queue: CallQueue::new("waitpid"),
        }
    }
}

impl Default for MockableSystemFunctions {
    fn default() -> Self {
        Self {
            real_impl: RealSystemFunctions,
            fallback_enabled: AtomicBool::new(true), // Enable fallback by default
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
        self.fallback_enabled.store(true, Ordering::Relaxed);
        self
    }

    /// Disable fallback to real implementations (strict mocking mode)
    pub fn disable_fallback(&self) -> &Self {
        self.fallback_enabled.store(false, Ordering::Relaxed);
        self
    }

    /// Check if fallback is enabled
    pub fn is_fallback_enabled(&self) -> bool {
        self.fallback_enabled.load(Ordering::Relaxed)
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
        // Just panic in tests, we don't need the global state anymore
        panic!("_exit({status}) called in mock context");
    }
}

/// Configuration options for the mock system
pub enum MockConfig {
    /// Use fallback mode (real implementations when no mock is configured)
    Fallback,

    /// Use strict mode (panic when no mock is configured)
    Strict,

    /// Configure the mock with a function, using fallback mode
    ConfiguredWithFallback(Box<dyn FnOnce(&MockableSystemFunctions) + Send + Sync>),

    /// Configure the mock with a function, using strict mode
    ConfiguredStrict(Box<dyn FnOnce(&MockableSystemFunctions) + Send + Sync>),
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

    // Run the test function with the mock
    test_fn(&mock)
}

/// Helper to create a `ConfiguredWithFallback` variant
pub fn configured_with_fallback<F>(configure_fn: F) -> MockConfig
where
    F: FnOnce(&MockableSystemFunctions) + Send + Sync + 'static,
{
    MockConfig::ConfiguredWithFallback(Box::new(configure_fn))
}

/// Helper to create a `ConfiguredStrict` variant
pub fn configured_strict<F>(configure_fn: F) -> MockConfig
where
    F: FnOnce(&MockableSystemFunctions) + Send + Sync + 'static,
{
    MockConfig::ConfiguredStrict(Box::new(configure_fn))
}

/// Check if mock system functions are enabled (controlled by environment variable)
/// Returns true unless MOCK_SYSTEM_FUNCTIONS=0 environment variable is set
pub fn is_mock_system_enabled() -> bool {
    match std::env::var("MOCK_SYSTEM_FUNCTIONS") {
        Ok(val) if val == "0" => false,
        _ => true,
    }
}

/// Helper function to enable mock system functions by setting the environment variable
pub fn enable_mock_system() {
    unsafe {
        std::env::set_var("MOCK_SYSTEM_FUNCTIONS", "1");
    }
}

/// Helper function to disable mock system functions by setting the environment variable
pub fn disable_mock_system() {
    unsafe {
        std::env::set_var("MOCK_SYSTEM_FUNCTIONS", "0");
    }
}
