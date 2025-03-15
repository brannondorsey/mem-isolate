//! # `mem-isolate`: *Run unsafe code safely*
//!
//! It runs your function via a `fork()`, waits for the result, and
//! returns it.
//!
//! This grants your code access to an exact copy of memory and state at the
//! time just before the call, but guarantees that the function will not affect
//! the parent process's memory footprint in any way. It forces functions to be
//! *pure*, even if they aren't.
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

use libc::c_int;
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
#[allow(clippy::missing_panics_doc)] // We have to panic in the child process but clippy can't tell this is OK
#[allow(clippy::too_many_lines)] // TODO: Break this up for readability
pub fn execute_in_isolated_process<F, T>(callable: F) -> Result<T, MemIsolateError>
where
    F: FnOnce() -> T,
    T: Serialize + DeserializeOwned,
{
    let sys = get_system_functions();
    let PipeFds { read_fd, write_fd } = create_pipe(&sys)?;

    match sys.fork() {
        Err(err) => Err(CallableDidNotExecute(ForkFailed(err))),

        // Child process
        Ok(ForkReturn::Child) => {
            // NOTE: Fallible actions in the child must either serialize
            // and send their error over the pipe, or exit with a code
            // that can be inerpreted by the parent.
            // TODO: Consider removing the serializations and just
            // using exit codes as the only way to communicate errors.
            // TODO: Get rid of all of the .expect()s

            // Droping the writer will close the write_fd, so we take it early
            // and explicitly to prevent use after free with two unsafe codes
            // scattered around the child code below.
            let mut writer = unsafe { File::from_raw_fd(write_fd) };
            close_read_end_of_pipe_in_child_or_exit(&sys, &mut writer, read_fd);

            // Execute the callable and handle serialization
            let result = callable();
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
    sys
}

fn create_pipe<S: SystemFunctions>(sys: &S) -> Result<PipeFds, MemIsolateError> {
    let pipe_fds = match sys.pipe() {
        Ok(pipe_fds) => pipe_fds,
        Err(err) => {
            return Err(CallableDidNotExecute(PipeCreationFailed(err)));
        }
    };

    Ok(pipe_fds)
}

fn wait_for_child<S: SystemFunctions>(
    sys: &S,
    child_pid: c_int,
) -> Result<WaitpidStatus, MemIsolateError> {
    // Wait for the child process to exit
    let waitpid_bespoke_status = match sys.waitpid(child_pid) {
        Ok(status) => status,
        Err(wait_err) => {
            return Err(CallableStatusUnknown(WaitFailed(wait_err)));
        }
    };

    Ok(waitpid_bespoke_status)
}

fn error_if_child_unhappy(waitpid_bespoke_status: WaitpidStatus) -> Result<(), MemIsolateError> {
    if let Some(exit_status) = child_process_exited_on_its_own(waitpid_bespoke_status) {
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
    }
}

fn deserialize_result<T: DeserializeOwned>(buffer: &[u8]) -> Result<T, MemIsolateError> {
    match bincode::deserialize::<Result<T, MemIsolateError>>(buffer) {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(err)) => Err(err),
        Err(err) => Err(CallableExecuted(DeserializationFailed(err.to_string()))),
    }
}

/// Doesn't matter if the value is an error or not, we just want to serialize it either way
///
/// # Panics
///
/// Panics if the serialization of a [`MemIsolateError`] fails
fn serialize_result_or_error_value<T: Serialize>(result: T) -> Vec<u8> {
    match bincode::serialize(&Ok::<T, MemIsolateError>(result)) {
        Ok(encoded) => encoded,
        Err(err) => {
            let err = CallableExecuted(SerializationFailed(err.to_string()));
            bincode::serialize(&Err::<T, MemIsolateError>(err)).expect("failed to serialize error")
        }
    }
}

fn write_and_flush_or_exit<S, W>(sys: &S, writer: &mut W, buffer: &[u8])
where
    S: SystemFunctions,
    W: Write,
{
    let result = writer.write_all(buffer).and_then(|()| writer.flush());
    if let Err(_err) = result {
        // If we can't write to the pipe, we can't communicate the error either
        // Parent will detect this as an UnexpectedChildDeath
        #[allow(clippy::used_underscore_items)]
        sys._exit(CHILD_EXIT_IF_WRITE_FAILED);
    }
}

fn exit_happy<S: SystemFunctions>(sys: &S) -> ! {
    #[allow(clippy::used_underscore_items)]
    sys._exit(CHILD_EXIT_HAPPY);
}

fn read_all_of_child_result_pipe(read_fd: c_int) -> Result<Vec<u8>, MemIsolateError> {
    // Read from the pipe by wrapping the read fd as a File
    let mut buffer = Vec::new();
    {
        let mut reader = unsafe { File::from_raw_fd(read_fd) };
        if let Err(err) = reader.read_to_end(&mut buffer) {
            return Err(CallableStatusUnknown(ParentPipeReadFailed(err)));
        }
    } // The read_fd will automatically be closed when the File is dropped

    if buffer.is_empty() {
        // TODO: How can we more rigorously know this? Maybe we write to a mem map before and after execution?
        return Err(CallableStatusUnknown(CallableProcessDiedDuringExecution));
    }

    Ok(buffer)
}

fn close_write_end_of_pipe_in_parent<S: SystemFunctions>(
    sys: &S,
    write_fd: c_int,
) -> Result<(), MemIsolateError> {
    if let Err(err) = sys.close(write_fd) {
        return Err(CallableStatusUnknown(ParentPipeCloseFailed(err)));
    }

    Ok(())
}

fn close_read_end_of_pipe_in_child_or_exit<S: SystemFunctions>(
    sys: &S,
    writer: &mut impl Write,
    read_fd: c_int,
) {
    if let Err(close_err) = sys.close(read_fd) {
        let err = CallableDidNotExecute(ChildPipeCloseFailed(Some(close_err)));
        let encoded = bincode::serialize(&err).expect("failed to serialize error");
        writer
            .write_all(&encoded)
            .expect("failed to write error to pipe");
        writer.flush().expect("failed to flush error to pipe");
        #[allow(clippy::used_underscore_items)]
        sys._exit(CHILD_EXIT_IF_READ_CLOSE_FAILED);
    }
}
