//! Error handling is an important part of the `mem-isolate` crate. If something
//! went wrong, we want to give the caller as much context as possible about how
//! that error effected their `callable`, so they are well-equipped to know what
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
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum MemIsolateError {
    /// Indicates something went wrong before the callable was executed. Because
    /// the callable never executed, it should be safe to naively retry the the
    /// callable with or without mem-isolate, even if the function is not
    /// idempotent.
    #[error("an error occurred before the callable was executed: {0}")]
    CallableDidNotExecute(#[source] CallableDidNotExecuteError),

    /// Indicates something went wrong after the callable was executed. **Do not**
    /// retry execution of the callable unless it is idempotent.
    #[error("an error occurred after the callable was executed: {0}")]
    CallableExecuted(#[source] CallableExecutedError),

    /// Indicates something went wrong, but it is unknown wether the callable was
    /// executed. **You should retry the callable only if it is idempotent.**
    #[error("the callable process exited with an unknown status: {0}")]
    CallableStatusUnknown(#[source] CallableStatusUnknownError),
}

// TODO: Document the rest of these errors
#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableExecutedError {
    #[error("an error occurred while serializing the result of the callable: {0}")]
    SerializationFailed(String),
    #[error("an error occurred while deserializing the result of the callable: {0}")]
    DeserializationFailed(String),
    #[serde(
        serialize_with = "serialize_option_os_error",
        deserialize_with = "deserialize_option_os_error"
    )]
    #[error("system error encountered writing the child's result to the pipe: {}", format_option_error(.0))]
    ChildPipeWriteFailed(#[source] Option<io::Error>),
}

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableDidNotExecuteError {
    // TODO: Consider making these io::Errors be RawOsError typedefs instead. That rules out a ton of overloaded io::Error posibilities. It's more precise.
    // WARNING: Serialization will fail if this is not an OS error.
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error(
        "system error encountered creating the pipe used to communicate with the child process: {0}"
    )]
    PipeCreationFailed(#[source] io::Error),
    #[serde(
        serialize_with = "serialize_option_os_error",
        deserialize_with = "deserialize_option_os_error"
    )]
    #[error("system error encountered closing the child's copy of the pipe's read end: {}", format_option_error(.0))]
    ChildPipeCloseFailed(#[source] Option<io::Error>),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered forking the child process: {0}")]
    ForkFailed(#[source] io::Error),
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableStatusUnknownError {
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered closing the parent's copy of the pipe's write end: {0}")]
    ParentPipeCloseFailed(#[source] io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered waiting for the child process: {0}")]
    WaitFailed(#[source] io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered reading the child's result from the pipe: {0}")]
    ParentPipeReadFailed(#[source] io::Error),
    #[error("the callable process died during execution")]
    CallableProcessDiedDuringExecution,
    #[error("the callable process exited with an unexpected status: {0}")]
    UnexpectedChildExitStatus(i32),
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

fn serialize_option_os_error<S>(error: &Option<io::Error>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if let Some(error) = error {
        serialize_os_error(error, serializer)
    } else {
        serializer.serialize_none()
    }
}

fn deserialize_option_os_error<'de, D>(deserializer: D) -> Result<Option<io::Error>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<i32> = Option::deserialize(deserializer)?;
    match s {
        Some(s) => Ok(Some(io::Error::from_raw_os_error(s))),
        None => Ok(None),
    }
}

fn format_option_error(err: &Option<io::Error>) -> String {
    match err {
        Some(e) => e.to_string(),
        None => "None".to_string(),
    }
}
