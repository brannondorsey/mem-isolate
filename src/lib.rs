//! # `mem-isolate`: *Run unsafe code safely*
//!
//! It runs your function via a `fork()`, waits for the result, and
//! returns it.
//!
//! This grants your code access to an exact copy of memory and state at the
//! time just before the call, but guarantees that the function will not affect
//! the parent process's memory footprint in any way.
//!
//! It forces functions to be *memory pure* (pure with respect to memory), even
//! if they aren't.
//!
//! ```
//! use mem_isolate::execute_in_isolated_process;
//!
//! // No heap, stack, or program memory out here...
//! let result = mem_isolate::execute_in_isolated_process(|| {
//!     // ...Can be affected by anything in here
//!     Box::leak(Box::new(vec![42; 1024]));
//! });
//! ```
//!
//! To keep things simple, this crate exposes only two public interfaces:
//!
//! * [`execute_in_isolated_process`] - The function that executes your code in
//!   an isolated process.
//! * [`MemIsolateError`] - The error type that function returns ☝️
//!
//! For more code examples, see [`examples/`](https://github.com/brannondorsey/mem-isolate/tree/main/examples).
//! [This one](https://github.com/brannondorsey/mem-isolate/blob/main/examples/error-handling-basic.rs)
//! in particular shows how you should think about error handling.
//!
//! For more information, see the [README](https://github.com/brannondorsey/mem-isolate).
//!
//! ## Supported platforms
//!
//! Because of its heavy use of POSIX system calls, this crate only
//! supports Unix-like operating systems (e.g. Linux, macOS, BSD).
//!
//! Windows and wasm support are not planned at this time.
#![warn(missing_docs)]
#![warn(clippy::pedantic, clippy::unwrap_used)]
#![warn(missing_debug_implementations)]

#[cfg(not(any(target_family = "unix")))]
compile_error!(
    "Because of its heavy use of POSIX system calls, this crate only supports Unix-like operating systems (e.g. Linux, macOS, BSD)"
);

#[cfg(feature = "tracing")]
use tracing::{Level, instrument, span};
// Don't import debug, error, warn directly to avoid conflicts with our macros

use libc::c_int;
use std::fmt::Debug;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;

#[cfg(test)]
mod tests;

mod c;
use c::{
    ForkReturn, PipeFds, SystemFunctions, WaitpidStatus, child_process_exited_on_its_own,
    child_process_killed_by_signal,
};

pub mod errors;
pub use errors::MemIsolateError;
use errors::{
    CallableDidNotExecuteError::{ChildPipeCloseFailed, ForkFailed, PipeCreationFailed},
    CallableExecutedError::{ChildPipeWriteFailed, DeserializationFailed, SerializationFailed},
    CallableStatusUnknownError::{
        CallableProcessDiedDuringExecution, ChildProcessKilledBySignal, ParentPipeCloseFailed,
        ParentPipeReadFailed, UnexpectedChildExitStatus, UnexpectedWaitpidReturnValue, WaitFailed,
    },
};

use MemIsolateError::{CallableDidNotExecute, CallableExecuted, CallableStatusUnknown};

// Re-export the serde traits our public API depends on
pub use serde::{Serialize, de::DeserializeOwned};

// Child process exit status codes
const CHILD_EXIT_HAPPY: i32 = 0;
const CHILD_EXIT_IF_READ_CLOSE_FAILED: i32 = 3;
const CHILD_EXIT_IF_WRITE_FAILED: i32 = 4;

#[cfg(feature = "tracing")]
const HIGHEST_LEVEL: Level = Level::ERROR;

mod macros;
use macros::{debug, error};

/// Executes a user-supplied `callable` in a forked child process so that any
/// memory changes during execution do not affect the parent. The child
/// serializes its result (using bincode) and writes it through a pipe, which
/// the parent reads and deserializes.
///
/// # Example
///
/// ```rust
/// use mem_isolate::execute_in_isolated_process;
///
/// let leaky_fn = || {
///     // Leak 1KiB of memory
///     let data: Vec<u8> = Vec::with_capacity(1024);
///     let data = Box::new(data);
///     Box::leak(data);
/// };
///
/// let _ = execute_in_isolated_process(leaky_fn);
/// // However, the memory is not leaked in the parent process here
/// ```
///
/// # Errors
///
/// Error handling is organized into three levels:
///
/// 1. The first level describes the effect of the error on the `callable` (e.g.
///    did your callable function execute or not)
/// 2. The second level describes what `mem-isolate` operation caused the error
///    (e.g. did serialization fail)
/// 3. The third level is the underlying OS error if it is available (e.g. an
///    `io::Error`)
///
/// For most applications, you'll care only about the first level:
///
/// ```rust
/// use mem_isolate::{execute_in_isolated_process, MemIsolateError};
///
/// // Function that might cause memory issues
/// let result = execute_in_isolated_process(|| {
///     // Some operation
///     "Success!".to_string()
/// });
///
/// match result {
///     Ok(value) => println!("Callable succeeded: {}", value),
///     Err(MemIsolateError::CallableDidNotExecute(_)) => {
///         // Safe to retry, callable never executed
///         println!("Callable did not execute, can safely retry");
///     },
///     Err(MemIsolateError::CallableExecuted(_)) => {
///         // Do not retry unless idempotent
///         println!("Callable executed but result couldn't be returned");
///     },
///     Err(MemIsolateError::CallableStatusUnknown(_)) => {
///         // Retry only if idempotent
///         println!("Unknown if callable executed, retry only if idempotent");
///     }
/// }
/// ```
///
/// For a more detailed look at error handling, see the documentation in the
/// [`errors`] module.
///
/// ## Important Note on Closures
///
/// When using closures that capture and mutate variables from their environment,
/// these mutations **only occur in the isolated child process** and do not affect
/// the parent process's memory. For example, it may seem surprising that the
/// following code will leave the parent's `counter` variable unchanged:
///
/// ```rust
/// use mem_isolate::execute_in_isolated_process;
///
/// let mut counter = 0;
/// let result = execute_in_isolated_process(|| {
///     counter += 1;  // This increment only happens in the child process
///     counter        // Returns 1
/// });
/// assert_eq!(counter, 0);  // Parent's counter remains unchanged
/// ```
///
/// This is the intended behavior as the function's purpose is to isolate all
/// memory effects of the callable. However, this can be surprising, especially
/// for [`FnMut`] or [`FnOnce`] closures.
#[allow(clippy::too_many_lines)] // TODO: Break this up for readability
#[cfg_attr(feature = "tracing", instrument(skip(callable)))]
pub fn execute_in_isolated_process<F, T>(callable: F) -> Result<T, MemIsolateError>
where
    F: FnOnce() -> T,
    T: Serialize + DeserializeOwned,
{
    #[cfg(feature = "tracing")]
    let parent_span = span!(HIGHEST_LEVEL, "parent").entered();

    let sys = get_system_functions();
    let PipeFds { read_fd, write_fd } = create_pipe(&sys)?;

    match sys.fork() {
        Err(err) => Err(CallableDidNotExecute(ForkFailed(err))),

        // Child process
        Ok(ForkReturn::Child) => {
            #[cfg(feature = "tracing")]
            std::mem::drop(parent_span);
            #[cfg(feature = "tracing")]
            let _child_span = span!(HIGHEST_LEVEL, "child").entered();
            // NOTE: Fallible actions in the child must either serialize
            // and send their error over the pipe, or exit with a code
            // that can be inerpreted by the parent.
            // TODO: Consider removing the serializations and just
            // using exit codes as the only way to communicate errors.
            // TODO: Get rid of all of the .expect()s

            let mut writer = unsafe { File::from_raw_fd(write_fd) };
            close_read_end_of_pipe_in_child_or_exit(&sys, &mut writer, read_fd);

            let result = execute_callable(callable);
            let encoded = serialize_result_or_error_value(result);
            write_and_flush_or_exit(&sys, &mut writer, &encoded);
            exit_happy(&sys)
        }

        // Parent process
        Ok(ForkReturn::Parent(child_pid)) => {
            close_write_end_of_pipe_in_parent(&sys, write_fd)?;

            let waitpid_bespoke_status = wait_for_child(&sys, child_pid)?;
            error_if_child_unhappy(waitpid_bespoke_status)?;

            let buffer: Vec<u8> = read_all_of_child_result_pipe(read_fd)?;
            deserialize_result(&buffer)
        }
    }
}

#[must_use]
#[cfg_attr(feature = "tracing", instrument)]
fn get_system_functions() -> impl SystemFunctions {
    // Use the appropriate implementation based on build config
    #[cfg(not(test))]
    let sys = c::RealSystemFunctions;

    #[cfg(test)]
    let sys = if c::mock::is_mocking_enabled() {
        // Use the mock from thread-local storage
        c::mock::get_current_mock()
    } else {
        // Create a new fallback mock if no mock is active
        c::mock::MockableSystemFunctions::with_fallback()
    };

    // Use the macro directly
    debug!("using {:?}", sys);
    sys
}

#[cfg_attr(feature = "tracing", instrument)]
fn create_pipe<S: SystemFunctions>(sys: &S) -> Result<PipeFds, MemIsolateError> {
    let pipe_fds = match sys.pipe() {
        Ok(pipe_fds) => pipe_fds,
        Err(err) => {
            let err = CallableDidNotExecute(PipeCreationFailed(err));
            error!("error creating pipe, propagating {:?}", err);
            return Err(err);
        }
    };

    debug!("pipe created: {:?}", pipe_fds);
    Ok(pipe_fds)
}

#[cfg_attr(feature = "tracing", instrument(skip(callable)))]
fn execute_callable<F, T>(callable: F) -> T
where
    F: FnOnce() -> T,
{
    debug!("starting execution of user-supplied callable");

    let result = {
        #[cfg(feature = "tracing")]
        let _span = span!(HIGHEST_LEVEL, "inside_callable").entered();
        callable()
    };

    debug!("finished execution of user-supplied callable");

    result
}

#[cfg_attr(feature = "tracing", instrument)]
fn wait_for_child<S: SystemFunctions>(
    sys: &S,
    child_pid: c_int,
) -> Result<WaitpidStatus, MemIsolateError> {
    // Wait for the child process to exit
    let waitpid_bespoke_status = match sys.waitpid(child_pid) {
        Ok(status) => status,
        Err(wait_err) => {
            let err = CallableStatusUnknown(WaitFailed(wait_err));
            error!("error waiting for child process, propagating {:?}", err);
            return Err(err);
        }
    };

    debug!(
        "wait completed, received status: {:?}",
        waitpid_bespoke_status
    );
    Ok(waitpid_bespoke_status)
}

#[cfg_attr(feature = "tracing", instrument)]
fn error_if_child_unhappy(waitpid_bespoke_status: WaitpidStatus) -> Result<(), MemIsolateError> {
    let result = if let Some(exit_status) = child_process_exited_on_its_own(waitpid_bespoke_status)
    {
        match exit_status {
            CHILD_EXIT_HAPPY => Ok(()),
            CHILD_EXIT_IF_READ_CLOSE_FAILED => {
                Err(CallableDidNotExecute(ChildPipeCloseFailed(None)))
            }
            CHILD_EXIT_IF_WRITE_FAILED => Err(CallableExecuted(ChildPipeWriteFailed(None))),
            unhandled_status => Err(CallableStatusUnknown(UnexpectedChildExitStatus(
                unhandled_status,
            ))),
        }
    } else if let Some(signal) = child_process_killed_by_signal(waitpid_bespoke_status) {
        Err(CallableStatusUnknown(ChildProcessKilledBySignal(signal)))
    } else {
        Err(CallableStatusUnknown(UnexpectedWaitpidReturnValue(
            waitpid_bespoke_status,
        )))
    };

    if let Ok(()) = result {
        debug!("child process exited happily on its own");
    } else {
        error!("child process signaled an error, propagating {:?}", result);
    }

    result
}

#[cfg_attr(feature = "tracing", instrument)]
fn deserialize_result<T: DeserializeOwned>(buffer: &[u8]) -> Result<T, MemIsolateError> {
    match bincode::deserialize::<Result<T, MemIsolateError>>(buffer) {
        Ok(Ok(result)) => {
            debug!("successfully deserialized happy result");
            Ok(result)
        }
        Ok(Err(err)) => {
            debug!("successfully deserialized error result: {:?}", err);
            Err(err)
        }
        Err(err) => {
            let err = CallableExecuted(DeserializationFailed(err.to_string()));
            error!("failed to deserialize result, propagating {:?}", err);
            Err(err)
        }
    }
}

/// Doesn't matter if the value is an error or not, we just want to serialize it either way
///
/// # Panics
///
/// Panics if the serialization of a [`MemIsolateError`] fails
#[cfg_attr(feature = "tracing", instrument(skip(result)))]
fn serialize_result_or_error_value<T: Serialize>(result: T) -> Vec<u8> {
    match bincode::serialize(&Ok::<T, MemIsolateError>(result)) {
        Ok(encoded) => {
            debug!(
                "serialization successful, resulted in {} bytes",
                encoded.len()
            );
            encoded
        }
        Err(err) => {
            let err = CallableExecuted(SerializationFailed(err.to_string()));
            error!(
                "serialization failed, now attempting to serialize error: {:?}",
                err
            );

            let encoded = bincode::serialize(&Err::<T, MemIsolateError>(err))
                .expect("failed to serialize error");

            debug!(
                "serialization of error successful, resulting in {} bytes",
                encoded.len()
            );
            encoded
        }
    }
}

#[cfg_attr(feature = "tracing", instrument)]
fn write_and_flush_or_exit<S, W>(sys: &S, writer: &mut W, buffer: &[u8])
where
    S: SystemFunctions,
    W: Write + Debug,
{
    let result = writer.write_all(buffer).and_then(|()| writer.flush());
    #[allow(unused_variables)]
    if let Err(err) = result {
        error!("error writing to pipe: {:?}", err);
        // If we can't write to the pipe, we can't communicate the error either
        // so we rely on the parent correctly interpreting the exit code
        let exit_code = CHILD_EXIT_IF_WRITE_FAILED;
        debug!("exiting child process with exit code: {}", exit_code);
        #[allow(clippy::used_underscore_items)]
        sys._exit(exit_code);
    } else {
        debug!("wrote and flushed to pipe successfully");
    }
}

fn exit_happy<S: SystemFunctions>(sys: &S) -> ! {
    // NOTE: We don't wrap this in #[cfg_attr(feature = "tracing", instrument)]
    // because doing so results in a compiler error because of the `!` return type
    // No idea why its usage is fine without the cfg_addr...
    #[cfg(feature = "tracing")]
    const FN_NAME: &str = stringify!(exit_happy);
    #[cfg(feature = "tracing")]
    let _span = span!(HIGHEST_LEVEL, FN_NAME).entered();

    let exit_code = CHILD_EXIT_HAPPY;

    debug!("exiting child process with exit code: {}", exit_code);

    #[allow(clippy::used_underscore_items)]
    sys._exit(exit_code);
}

#[cfg_attr(feature = "tracing", instrument)]
fn read_all_of_child_result_pipe(read_fd: c_int) -> Result<Vec<u8>, MemIsolateError> {
    // Read from the pipe by wrapping the read fd as a File
    let mut buffer = Vec::new();
    {
        let mut reader = unsafe { File::from_raw_fd(read_fd) };
        if let Err(err) = reader.read_to_end(&mut buffer) {
            let err = CallableStatusUnknown(ParentPipeReadFailed(err));
            error!("error reading from pipe, propagating {:?}", err);
            return Err(err);
        }
    } // The read_fd will automatically be closed when the File is dropped

    if buffer.is_empty() {
        // TODO: How can we more rigorously know this? Maybe we write to a mem map before and after execution?
        let err = CallableStatusUnknown(CallableProcessDiedDuringExecution);
        error!("buffer unexpectedly empty, propagating {:?}", err);
        return Err(err);
    }

    debug!("successfully read {} bytes from pipe", buffer.len());
    Ok(buffer)
}

#[cfg_attr(feature = "tracing", instrument)]
fn close_write_end_of_pipe_in_parent<S: SystemFunctions>(
    sys: &S,
    write_fd: c_int,
) -> Result<(), MemIsolateError> {
    if let Err(err) = sys.close(write_fd) {
        let err = CallableStatusUnknown(ParentPipeCloseFailed(err));
        error!("error closing write end of pipe, propagating {:?}", err);
        return Err(err);
    }
    debug!("write end of pipe closed successfully");
    Ok(())
}

#[cfg_attr(feature = "tracing", instrument)]
fn close_read_end_of_pipe_in_child_or_exit<S: SystemFunctions>(
    sys: &S,
    writer: &mut (impl Write + Debug),
    read_fd: c_int,
) {
    if let Err(close_err) = sys.close(read_fd) {
        let err = CallableDidNotExecute(ChildPipeCloseFailed(Some(close_err)));
        error!(
            "error closing read end of pipe, now attempting to serialize error: {:?}",
            err
        );

        let encoded = bincode::serialize(&err).expect("failed to serialize error");
        writer
            .write_all(&encoded)
            .expect("failed to write error to pipe");
        writer.flush().expect("failed to flush error to pipe");

        let exit_code = CHILD_EXIT_IF_READ_CLOSE_FAILED;
        error!("exiting child process with exit code: {}", exit_code);
        #[allow(clippy::used_underscore_items)]
        sys._exit(exit_code);
    } else {
        debug!("read end of pipe closed successfully");
    }
}
