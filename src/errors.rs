//! Error handling is an important part of the `mem-isolate` crate. If something
//! went wrong, we want to give the caller as much context as possible about how
//! that error affected their `callable`, so they are well-equipped to know what
//! to do about it.
//!
//! The primary error type is [`MemIsolateError`], which is returned by
//! [`crate::execute_in_isolated_process`].
//!
//! Philosophically, error handling in `mem-isolate` is organized into three
//! levels of error wrapping:
//!
//! 1. The first level describes the effect of the error on the `callable` (e.g.
//!    did your callable function execute or not)
//! 2. The second level describes what `mem-isolate` operation caused the error
//!    (e.g. did serialization fail)
//! 3. The third level is the underlying OS error if it is available (e.g. an
//!    `io::Error`)
//!
//! For most applications, you'll care only about the first level. For an
//! example of common error handling dealing only with first level errors, see
//! [`examples/error-handling-basic.rs`](https://github.com/brannondorsey/mem-isolate/blob/main/examples/error-handling-basic.rs).
//!
//! Levels two and three are useful if you want to know more about what
//! **exactly** went wrong and expose internals about how `mem-isolate` works.
//!
//! Note: These errors all describe things that went wrong with a `mem-isolate`
//! operation. They have nothing to do with the `callable` you passed to
//! [`crate::execute_in_isolated_process`], which can define its own errors and
//! maybe values by returning a [`Result`] or [`Option`] type.

use serde::Deserialize;
use serde::Serialize;
use std::io;
use thiserror::Error;

/// [`MemIsolateError`] is the **primary error type returned by the crate**. The
/// goal is to give the caller context about what happened to their callable if
/// something went wrong.
///
/// For basic usage, and an introduction of how you should think about error
/// handling with this crate, see
/// [`examples/error-handling-basic.rs`](https://github.com/brannondorsey/mem-isolate/blob/main/examples/error-handling-basic.rs)
///
/// For an exhaustive look of all possible error variants, see [`examples/error-handling-complete.rs`](https://github.com/brannondorsey/mem-isolate/blob/main/examples/error-handling-complete.rs)
// TODO: Consider making this Send + Sync
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum MemIsolateError {
    /// Indicates something went wrong before the callable was executed. Because
    /// the callable never executed, it should be safe to naively retry the
    /// callable with or without mem-isolate, even if the function is not
    /// idempotent.
    #[error("an error occurred before the callable was executed: {0}")]
    CallableDidNotExecute(#[source] CallableDidNotExecuteError),

    /// Indicates something went wrong after the callable was executed. **Do not**
    /// retry execution of the callable unless it is idempotent.
    #[error("an error occurred after the callable was executed: {0}")]
    CallableExecuted(#[source] CallableExecutedError),

    /// Indicates something went wrong, but it is unknown whether the callable was
    /// executed. **You should retry the callable only if it is idempotent.**
    #[error("the callable process exited with an unknown status: {0}")]
    CallableStatusUnknown(#[source] CallableStatusUnknownError),
}

/// An error indicating something went wrong **after** the user-supplied callable was executed
///
/// You should only retry the callable if it is idempotent.
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableExecutedError {
    /// An error occurred while serializing the result of the callable inside the child process
    #[error(
        "an error occurred while serializing the result of the callable inside the child process"
    )]
    SerializationFailed,

    /// An error occurred while deserializing the result of the callable in the parent process
    #[error(
        "an error occurred while deserializing the result of the callable in the parent process: {0}"
    )]
    DeserializationFailed(String),

    /// A system error occurred while writing the child process's result to the pipe.
    #[error("system error encountered writing the child process's result to the pipe")]
    ChildPipeWriteFailed,
}

/// An error indicating something went wrong **before** the user-supplied
/// callable was executed
///
/// It is harmless to retry the callable.
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableDidNotExecuteError {
    // TODO: Consider making these io::Errors be RawOsError typedefs instead.
    // That rules out a ton of overloaded io::Error posibilities. It's more
    // precise. WARNING: Serialization will fail if this is not an OS error.
    //
    /// A system error occurred while creating the pipe used to communicate with
    /// the child process
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error(
        "system error encountered creating the pipe used to communicate with the child process: {0}"
    )]
    PipeCreationFailed(#[source] io::Error),

    /// A system error occurred while closing the child process's copy of the
    /// pipe's read end
    #[error("system error encountered closing the child's copy of the pipe's read end")]
    ChildPipeCloseFailed,

    /// A system error occurred while forking the child process which is used to
    /// execute user-supplied callable
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered forking the child process: {0}")]
    ForkFailed(#[source] io::Error),
}

/// An error indicating that something went wrong in a way where it is difficult
/// or impossible to determine whether the user-supplied callable was executed
/// `¯\_(ツ)_/¯`
///
/// You should only retry the callable if it is idempotent.
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableStatusUnknownError {
    /// A system error occurred while closing the parent's copy of the pipe's
    /// write end
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered closing the parent's copy of the pipe's write end: {0}")]
    ParentPipeCloseFailed(#[source] io::Error),

    /// A system error occurred while waiting for the child process to exit
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered waiting for the child process: {0}")]
    WaitFailed(#[source] io::Error),

    /// A system error occurred while reading the child's result from the pipe
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered reading the child's result from the pipe: {0}")]
    ParentPipeReadFailed(#[source] io::Error),

    /// The callable process died while executing the user-supplied callable
    #[error("the callable process died during execution")]
    CallableProcessDiedDuringExecution,

    /// The child process responsible for executing the user-supplied callable
    /// exited with an unexpected status
    ///
    /// Note this does not represent some sort of exit code or return value
    /// indicating the success or failure of the user-supplied callable itself.
    #[error("the callable process exited with an unexpected status: {0}")]
    UnexpectedChildExitStatus(i32),

    /// The child process responsible for executing the user-supplied callable
    /// was killed by a signal
    #[error("the callable process was killed by a signal: {0}")]
    ChildProcessKilledBySignal(i32),

    /// Waitpid returned an unexpected value
    #[error("waitpid returned an unexpected value: {0}")]
    UnexpectedWaitpidReturnValue(i32),
}

fn serialize_os_error<S>(error: &io::Error, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if let Some(raw_os_error) = error.raw_os_error() {
        serializer.serialize_i32(raw_os_error)
    } else {
        Err(serde::ser::Error::custom("not an os error"))
    }
}

fn deserialize_os_error<'de, D>(deserializer: D) -> Result<io::Error, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: i32 = i32::deserialize(deserializer)?;
    Ok(io::Error::from_raw_os_error(s))
}
