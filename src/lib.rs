use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;

mod c;
use c::{_exit, ForkReturn, PipeFds, close, fork, pipe, waitpid};

mod errors;
use errors::{
    CallableDidNotExecuteError, CallableExecutedError, CallableStatusUnknownError, MemIsolateError,
};

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
    use CallableDidNotExecuteError::*;
    use CallableExecutedError::*;
    use CallableStatusUnknownError::*;
    use MemIsolateError::*;

    // Create a pipe.
    // TODO: Should I use dup2 somewhere here?
    let PipeFds { read_fd, write_fd } = match pipe() {
        Ok(pipe_fds) => pipe_fds,
        Err(err) => {
            return Err(CallableDidNotExecute(PipeCreationFailed(err)));
        }
    };

    // Statuses 3-63 are normally fair game
    const CHILD_EXIT_HAPPY: i32 = 0;
    const CHILD_EXIT_IF_READ_CLOSE_FAILED: i32 = 3;
    const CHILD_EXIT_IF_WRITE_FAILED: i32 = 4;

    match fork() {
        Err(err) => Err(CallableDidNotExecute(ForkFailed(err))),
        Ok(ForkReturn::Child) => {
            // NOTE: We chose to panic in the child if we can't communicate an error back to the parent.
            // The parent can then interpret this an an UnexpectedChildDeath.

            // Close the read end of the pipe
            if let Err(close_err) = close(read_fd) {
                let err = CallableDidNotExecute(ChildPipeCloseFailed(Some(close_err)));
                let encoded = bincode::serialize(&err).expect("failed to serialize error");

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
            let encoded = match bincode::serialize(&Ok::<T, MemIsolateError>(result)) {
                Ok(encoded) => encoded,
                Err(err) => {
                    let err = CallableExecuted(SerializationFailed(err));
                    bincode::serialize(&Err::<T, MemIsolateError>(err))
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
                return Err(CallableStatusUnknown(ParentPipeCloseFailed(close_err)));
            }

            // Wait for the child process to exit
            let status = match waitpid(child_pid) {
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
                Err(err) => Err(CallableExecuted(DeserializationFailed(err))),
            }
        }
    }
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

    #[test]
    fn test_all_function_types() {
        // 1. Function pointer (simplest, most explicit)
        fn function_pointer() -> MyResult {
            MyResult { value: 42 }
        }
        let result = execute_in_isolated_process(function_pointer).unwrap();
        assert_eq!(result, MyResult { value: 42 });

        // 2. Fn closure (immutable captures, can be called multiple times)
        let fn_closure = || MyResult { value: 42 };
        let result = execute_in_isolated_process(fn_closure).unwrap();
        assert_eq!(result, MyResult { value: 42 });

        // 3. FnMut closure (mutable captures, can be called multiple times)
        let mut counter = 0;
        let fn_mut_closure = || {
            counter += 1;
            MyResult { value: counter }
        };
        let result = execute_in_isolated_process(fn_mut_closure).unwrap();
        assert_eq!(result, MyResult { value: 1 });
        // WARNING: This zero is a surprising result if you don't understand that
        // the closure is called in a new process. This is the whole point of mem-isolate.
        assert_eq!(counter, 0);

        // 4. FnOnce closure (consumes captures, can only be called once)
        let value = String::from("hello");
        let fn_once_closure = move || {
            // This closure takes ownership of value
            MyResult {
                value: value.len() as i32,
            }
        };
        let result = execute_in_isolated_process(fn_once_closure).unwrap();
        assert_eq!(result, MyResult { value: 5 });
    }
}
