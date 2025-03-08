use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;

#[cfg(test)]
mod tests;

mod c;
use c::{ForkReturn, PipeFds, SystemFunctions};

mod errors;
pub use errors::MemIsolateError;
use errors::{CallableDidNotExecuteError, CallableExecutedError, CallableStatusUnknownError};

// Re-export the serde traits our public API depends on
pub use serde::{Serialize, de::DeserializeOwned};

/// Execute `callable` in a forked child process so that any memory changes during execution do not affect the parent.
/// The child serializes its result (using bincode) and writes it through a pipe, which the parent reads and deserializes.
///
/// # Safety
/// This code directly calls glibc functions (via the libc crate) and should only be used in a Unix environment.
// TODO: Real error handling with thiserror. Plumbing errors in the child also need to be passed back to the parent.
// TODO: Benchmark nicing the child process.
pub fn execute_in_isolated_process<F, T>(callable: F) -> Result<T, MemIsolateError>
where
    // TODO: Consider restricting to disallow FnMut() closures
    F: FnOnce() -> T,
    T: Serialize + DeserializeOwned,
{
    use CallableDidNotExecuteError::*;
    use CallableExecutedError::*;
    use CallableStatusUnknownError::*;
    use MemIsolateError::*;

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

    // Create a pipe.
    // TODO: Should I use dup2 somewhere here?
    let PipeFds { read_fd, write_fd } = match sys.pipe() {
        Ok(pipe_fds) => pipe_fds,
        Err(err) => {
            return Err(CallableDidNotExecute(PipeCreationFailed(err)));
        }
    };

    // Statuses 3-63 are normally fair game
    const CHILD_EXIT_HAPPY: i32 = 0;
    const CHILD_EXIT_IF_READ_CLOSE_FAILED: i32 = 3;
    const CHILD_EXIT_IF_WRITE_FAILED: i32 = 4;

    match sys.fork() {
        Err(err) => Err(CallableDidNotExecute(ForkFailed(err))),
        Ok(ForkReturn::Child) => {
            // NOTE: We chose to panic in the child if we can't communicate an error back to the parent.
            // The parent can then interpret this an an UnexpectedChildDeath.

            // Droping the writer will close the write_fd, so we take it early and explicitly to
            // prevent use after free with two unsafe calls scattered around the child code below.
            let mut writer = unsafe { File::from_raw_fd(write_fd) };

            // Close the read end of the pipe
            if let Err(close_err) = sys.close(read_fd) {
                let err = CallableDidNotExecute(ChildPipeCloseFailed(Some(close_err)));
                let encoded = bincode::serialize(&err).expect("failed to serialize error");
                writer
                    .write_all(&encoded)
                    .expect("failed to write error to pipe");
                writer.flush().expect("failed to flush error to pipe");
                sys._exit(CHILD_EXIT_IF_READ_CLOSE_FAILED);
            }

            // Execute the callable and handle serialization
            let result = callable();
            let encoded = match bincode::serialize(&Ok::<T, MemIsolateError>(result)) {
                Ok(encoded) => encoded,
                Err(err) => {
                    let err = CallableExecuted(SerializationFailed(err.to_string()));
                    bincode::serialize(&Err::<T, MemIsolateError>(err))
                        .expect("failed to serialize error")
                }
            };

            // Write the result to the pipe
            let write_result = writer.write_all(&encoded).and_then(|_| writer.flush());

            if let Err(_err) = write_result {
                // If we can't write to the pipe, we can't communicate the error either
                // Parent will detect this as an UnexpectedChildDeath
                sys._exit(CHILD_EXIT_IF_WRITE_FAILED);
            }

            // Exit immediately; use _exit to avoid running atexit()/on_exit() handlers
            // and flushing stdio buffers, which are exact clones of the parent in the child process.
            sys._exit(CHILD_EXIT_HAPPY);
            // The code after _exit is unreachable because _exit never returns
        }
        Ok(ForkReturn::Parent(child_pid)) => {
            // Close the write end of the pipe
            if let Err(close_err) = sys.close(write_fd) {
                return Err(CallableStatusUnknown(ParentPipeCloseFailed(close_err)));
            }

            // Wait for the child process to exit
            // TODO: waitpid doesn't return the exit status you expect it to.
            // See the Linux Programming Interface book for more details.
            let status = match sys.waitpid(child_pid) {
                Ok(status) => status,
                Err(wait_err) => {
                    return Err(CallableStatusUnknown(WaitFailed(wait_err)));
                }
            };

            match status {
                CHILD_EXIT_HAPPY => {}
                CHILD_EXIT_IF_READ_CLOSE_FAILED => {
                    return Err(CallableDidNotExecute(ChildPipeCloseFailed(None)));
                }
                CHILD_EXIT_IF_WRITE_FAILED => {
                    return Err(CallableExecuted(ChildPipeWriteFailed(None)));
                }
                unhandled_status => {
                    return Err(CallableStatusUnknown(UnexpectedChildExitStatus(
                        unhandled_status,
                    )));
                }
            }

            // Read from the pipe by wrapping the read fd as a File
            let mut buffer = Vec::new();
            {
                let mut reader = unsafe { File::from_raw_fd(read_fd) };
                if let Err(err) = reader.read_to_end(&mut buffer) {
                    return Err(CallableStatusUnknown(ParentPipeReadFailed(err)));
                }
            } // The read_fd will automatically be closed when the File is dropped

            if buffer.is_empty() {
                // TODO: How can we more rigerously know this? Maybe we write to a mem map before and after execution?
                return Err(CallableStatusUnknown(CallableProcessDiedDuringExecution));
            }
            // Update the deserialization to handle child errors
            match bincode::deserialize::<Result<T, MemIsolateError>>(&buffer) {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(err)) => Err(err),
                Err(err) => Err(CallableExecuted(DeserializationFailed(err.to_string()))),
            }
        }
    }
}
