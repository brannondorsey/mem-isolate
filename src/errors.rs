// TODO: Should I be using serde_derive instead of serde?
use serde::Deserialize;
use serde::Serialize;
use std::io;
use thiserror::Error;

// New Error proposal
#[allow(dead_code)]
#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum MemIsolateError {
    #[error("an error occurred after the callable was executed: {0}")]
    CallableExecuted(#[source] CallableExecutedError),
    #[error("an error occurred before the callable was executed: {0}")]
    CallableDidNotExecute(#[source] CallableDidNotExecuteError),
    #[error("the callable process exited with an unknown status: {0}")]
    CallableStatusUnknown(#[source] CallableStatusUnknownError),
}

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableExecutedError {
    #[error("an error occurred while serializing the result of the callable: {0}")]
    #[serde(
        // WARNING: This is improper serialization. This is a bug.
        // FIXME
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    SerializationFailed(#[source] bincode::Error),
    #[error("an error occurred while deserializing the result of the callable: {0}")]
    #[serde(
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    DeserializationFailed(#[source] bincode::Error),
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
    ForkFailed(io::Error),
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

fn serialize_bincode_error<S>(error: &bincode::Error, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&error.to_string())
}

fn deserialize_bincode_error<'de, D>(deserializer: D) -> Result<bincode::Error, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    Ok(bincode::Error::new(bincode::ErrorKind::Custom(s)))
}

fn format_option_error(err: &Option<io::Error>) -> String {
    match err {
        Some(e) => e.to_string(),
        None => "None".to_string(),
    }
}
