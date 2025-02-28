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
    #[error("an error occurred after the callable was executed")]
    CallableExecuted(CallableExecutedError),
    #[error("an error occurred before the callable was executed")]
    CallableDidNotExecute(CallableDidNotExecuteError),
    #[error("the callable process exited with an unknown status")]
    CallableStatusUnknown(CallableStatusUnknownError),
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableExecutedError {
    #[error("an error occurred while serializing the result of the callable")]
    #[serde(
        // WARNING: This is improper serialization. This is a bug.
        // FIXME
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    SerializationFailed(bincode::Error),
    #[error("an error occurred while deserializing the result of the callable")]
    #[serde(
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    DeserializationFailed(bincode::Error),
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableDidNotExecuteError {
    // TODO: Consider making these io::Errors be RawOsError typedefs instead. That rules out a ton of overloaded io::Error posibilities. It's more precise.
    // WARNING: Serialization will fail if this is not an OS error.
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error(
        "system error encountered creating the pipe used to communicate with the child process"
    )]
    PipeCreationFailed(io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered closing the child's copy of the pipe's read end")]
    ChildPipeCloseFailed(io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered writing the child'sresult to the pipe")]
    ChildPipeWriteFailed(io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered forking the child process")]
    ForkFailed(io::Error),
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CallableStatusUnknownError {
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered closing the parent's copy of the pipe's write end")]
    ParentPipeCloseFailed(io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered waiting for the child process")]
    WaitFailed(io::Error),
    #[serde(
        serialize_with = "serialize_os_error",
        deserialize_with = "deserialize_os_error"
    )]
    #[error("system error encountered reading the child's result from the pipe")]
    ParentPipeReadFailed(io::Error),
    #[error("the callable process died during execution")]
    CallableProcessDiedDuringExecution,
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
