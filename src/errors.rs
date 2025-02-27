use serde::Deserialize;
use serde::Serialize;
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MemIsolateError {
    #[error("An error occurred in the parent process")]
    Parent(ParentError),
    #[error("An error occurred in the child process")]
    Child(ChildError),
}

#[derive(Error, Debug)]
pub enum ParentError {
    #[error(
        "An error occurred in the plumbing of this implementation, likely due to a syscall failure"
    )]
    Plumbing(ParentPlumbingError),
    #[error("unable to deserialize the result of the function you provided")]
    Data(DeserializationFailed),
}

#[derive(Error, Serialize, Deserialize, Debug)]
pub enum ChildError {
    #[error("system error encountered closing the child's copy of the pipe's read end")]
    Plumbing(ChildPlumbingError),
    #[error("unable to serialize the result of the function you provided")]
    Data(SerializationFailed),
}

#[derive(Error, Debug)]
pub enum ParentPlumbingError {
    #[error(
        "system error encountered creating the pipe used to communicate with the child process"
    )]
    // TODO: Consider making these io::Errors be RawOsError typedefs instead. That rules out a ton of overloaded io::Error posibilities. It's more precise.
    PipeCreationFailed(io::Error),
    #[error("system error encountered closing the parent's copy of the pipe's write end")]
    PipeCloseFailed(io::Error),
    #[error("system error encountered reading the child's result from the pipe")]
    PipeReadFailed(io::Error),
    #[error("system error encountered forking the child process")]
    ForkFailed(io::Error),
    #[error("system error encountered waiting for the child process")]
    WaitFailed(io::Error),
    // TODO: Consider having an entire class of UnexpectedErrors. Or maybe make this an ChildDied
    #[error("the child never wrote to the pipe before exiting. Did the process die?")]
    UnexpectedChildDeath,
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum ChildPlumbingError {
    #[error("system error encountered closing the child's copy of the pipe's read end")]
    #[serde(
        serialize_with = "serialize_io_error",
        deserialize_with = "deserialize_io_error"
    )]
    PipeCloseFailed(io::Error),
    #[error("system error encountered writing the result to the pipe")]
    #[serde(
        serialize_with = "serialize_io_error",
        deserialize_with = "deserialize_io_error"
    )]
    PipeWriteFailed(io::Error),
}

#[derive(Error, Debug, Serialize, Deserialize)]
#[error("serialization failed")]
pub struct SerializationFailed(
    #[serde(
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    pub bincode::Error,
);

#[derive(Error, Debug, Serialize, Deserialize)]
#[error("deserialization failed")]
pub struct DeserializationFailed(
    #[serde(
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    pub bincode::Error,
);

fn serialize_io_error<S>(error: &io::Error, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&error.to_string())
}

fn deserialize_io_error<'de, D>(deserializer: D) -> Result<io::Error, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    Ok(io::Error::new(io::ErrorKind::Other, s))
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
