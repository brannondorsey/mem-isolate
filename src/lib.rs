use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs::File;
use std::io;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use thiserror::Error;

mod c;
use c::{_exit, ForkReturn, PipeFds, close, fork, pipe, waitpid};

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
    bincode::Error,
);

#[derive(Error, Debug, Serialize, Deserialize)]
#[error("deserialization failed")]
pub struct DeserializationFailed(
    #[serde(
        serialize_with = "serialize_bincode_error",
        deserialize_with = "deserialize_bincode_error"
    )]
    bincode::Error,
);

/// Execute `callable` in a forked child process so that any memory changes during do not affect the parent.
/// The child serializes its result (using bincode) and writes it through a pipe, which the parent reads and deserializes.
///
/// # Safety
/// This code directly calls glibc functions (via the libc crate) and should only be used in a Unix environment.
// TODO: Real error handling with thiserror. Plumbing errors in the child also need to be passed back to the parent.
// TODO: Benchmark nicing the child process.
pub fn execute_in_isolated_process<F, T>(callable: F) -> Result<T, MemIsolateError>
where
    F: FnOnce() -> T,
    T: Serialize + DeserializeOwned,
{
    // Create a pipe.
    // TODO: Should I use dup2 somewhere here?
    let PipeFds { read_fd, write_fd } = match pipe() {
        Ok(pipe_fds) => pipe_fds,
        Err(err) => {
            let err = ParentPlumbingError::PipeCreationFailed(err);
            let err = ParentError::Plumbing(err);
            let err = MemIsolateError::Parent(err);
            return Err(err);
        }
    };

    const CHILD_EXIT_HAPPY: i32 = 0;
    const CHILD_EXIT_IF_READ_CLOSE_FAILED: i32 = 1;
    const CHILD_EXIT_IF_WRITE_FAILED: i32 = 2;

    // TODO: Wrap all libc calls in a safe wrapper function that returns a Result<T, io::Error>
    match fork() {
        Err(err) => {
            let err = ParentPlumbingError::ForkFailed(err);
            let err = ParentError::Plumbing(err);
            let err = MemIsolateError::Parent(err);
            Err(err)
        }
        Ok(ForkReturn::Child) => {
            // NOTE: We chose to panic in the child if we can't communicate an error back to the parent.
            // The parent can then interpret this an an UnexpectedChildDeath.

            // Close the read end of the pipe
            if let Err(close_err) = close(read_fd) {
                let err = ChildPlumbingError::PipeCloseFailed(close_err);
                let encoded = bincode::serialize(&ChildError::Plumbing(err))
                    .expect("failed to serialize the child error");

                // TODO: Make sure we uphold the invariant that the write_fd is always open and not shared with anything else.
                let mut writer = unsafe { File::from_raw_fd(write_fd) };
                writer
                    .write_all(&encoded)
                    .expect("failed to write error to pipe");
                writer.flush().expect("failed to flush error to pipe");
                _exit(CHILD_EXIT_IF_READ_CLOSE_FAILED);
            }

            // Execute the callable and handle serialization
            let result = callable();
            let encoded = match bincode::serialize(&Ok::<T, ChildError>(result)) {
                Ok(encoded) => encoded,
                Err(err) => {
                    let err = ChildError::Data(SerializationFailed(err));
                    bincode::serialize(&Err::<T, ChildError>(err))
                        .expect("failed to serialize error")
                }
            };

            // Write the result to the pipe
            let mut writer = unsafe { File::from_raw_fd(write_fd) };
            let write_result = writer.write_all(&encoded).and_then(|_| writer.flush());

            if let Err(_err) = write_result {
                // If we can't write to the pipe, we can't communicate the error either
                // Parent will detect this as an UnexpectedChildDeath
                _exit(CHILD_EXIT_IF_WRITE_FAILED);
            }

            // Exit immediately; use _exit to avoid running atexit()/on_exit() handlers
            // and flushing stdio buffers, which are exact clones of the parent in the child process.
            _exit(CHILD_EXIT_HAPPY);
        }
        Ok(ForkReturn::Parent(child_pid)) => {
            // Close the write end of the pipe
            if let Err(close_err) = close(write_fd) {
                let err = ParentPlumbingError::PipeCloseFailed(close_err);
                let err = ParentError::Plumbing(err);
                let err = MemIsolateError::Parent(err);
                return Err(err);
            }

            // Wait for the child process to exit
            // TODO: Compare _status to CHILD_EXIT_* and transform the error if necessary
            let _status = match waitpid(child_pid) {
                Ok(status) => status,
                Err(wait_err) => {
                    let err = ParentPlumbingError::WaitFailed(wait_err);
                    let err = ParentError::Plumbing(err);
                    let err = MemIsolateError::Parent(err);
                    return Err(err);
                }
            };

            // Read from the pipe by wrapping the read fd as a File
            let mut buffer = Vec::new();
            {
                let mut reader = unsafe { File::from_raw_fd(read_fd) };
                if let Err(err) = reader.read_to_end(&mut buffer) {
                    let err = ParentPlumbingError::PipeReadFailed(err);
                    let err = ParentError::Plumbing(err);
                    let err = MemIsolateError::Parent(err);
                    return Err(err);
                }
            } // The read_fd will automatically be closed when the File is dropped

            if buffer.is_empty() {
                // TODO: #1 make this a child error
                // TODO: #2 On second thought, maybe we should frame errors more from the perspective of the caller instead of the implementation.
                // For instance, maybe we have FunctionDidNotRunError(Reason), FunctionRanError(Reason), and FunctionPanickedError().
                let err = ParentError::Plumbing(ParentPlumbingError::UnexpectedChildDeath);
                let err = MemIsolateError::Parent(err);
                return Err(err);
            }
            // Update the deserialization to handle child errors
            match bincode::deserialize::<Result<T, ChildError>>(&buffer) {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(err)) => Err(MemIsolateError::Child(err)),
                Err(err) => {
                    let err = ParentError::Data(DeserializationFailed(err));
                    Err(MemIsolateError::Parent(err))
                }
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct MyResult {
        value: i32,
    }

    // TODO: Add test for memory leaks with Box::leak(). My first attempt at this proved challenging only in the detection mechanism.
    // TODO: Add tests for all three closure types + fn pointers.

    #[test]
    fn simple_example() {
        let result = execute_in_isolated_process(|| MyResult { value: 42 }).unwrap();
        assert_eq!(result, MyResult { value: 42 });
    }

    #[test]
    #[allow(static_mut_refs)]
    fn test_static_memory_mutation_without_isolation() {
        static mut MEMORY: bool = false;
        let mutate = || unsafe { MEMORY = true };

        // Directly modify static memory
        mutate();

        // Verify the change persists
        unsafe {
            assert!(MEMORY, "Static memory should be modified");
        }
    }

    #[test]
    #[allow(static_mut_refs)]
    fn test_static_memory_mutation_with_isolation() {
        static mut MEMORY: bool = false;
        let mutate = || unsafe { MEMORY = true };

        // Modify static memory in isolated process
        execute_in_isolated_process(mutate).unwrap();

        // Verify the change does not affect parent process
        unsafe {
            assert!(
                !MEMORY,
                "Static memory should remain unmodified in parent process"
            );
        }
    }
}
