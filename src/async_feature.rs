#![allow(unused_imports)]

use crate::macros::{debug, error, info};
#[cfg(feature = "tracing")]
// Don't import event macros like debug, error, etc. directly to avoid conflicts
// with our macros (see just above^)
use tracing::{Level, instrument, span};

use libc::c_int;
use std::fmt::Debug;
use std::fs::File;
use std::io::{Read, Write};
use std::marker::Unpin;
use std::os::unix::io::FromRawFd;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::task;

use crate::c::{
    ForkReturn, PipeFds, SystemFunctions, WaitpidStatus, child_process_exited_on_its_own,
    child_process_killed_by_signal,
};

#[cfg(feature = "tracing")]
use crate::HIGHEST_LEVEL;
pub use crate::errors::MemIsolateError;
use crate::errors::{
    CallableDidNotExecuteError::{ChildPipeCloseFailed, ForkFailed, PipeCreationFailed},
    CallableExecutedError::{ChildPipeWriteFailed, DeserializationFailed, SerializationFailed},
    CallableStatusUnknownError::{
        CallableProcessDiedDuringExecution, ChildProcessKilledBySignal, ParentPipeCloseFailed,
        ParentPipeReadFailed, UnexpectedChildExitStatus, UnexpectedWaitpidReturnValue, WaitFailed,
    },
};
use crate::{
    CHILD_EXIT_HAPPY, CHILD_EXIT_IF_READ_CLOSE_FAILED, CHILD_EXIT_IF_WRITE_FAILED,
    deserialize_result, error_if_child_unhappy, fork, get_system_functions,
    serialize_result_or_error_value,
};

use MemIsolateError::{CallableDidNotExecute, CallableExecuted, CallableStatusUnknown};

use tokio::task::JoinError;
// Re-export the serde traits our public API depends on
pub use serde::{Serialize, de::DeserializeOwned};

#[cfg_attr(feature = "tracing", instrument(skip(callable)))]
pub async fn execute_in_isolated_process<F, Fut, T>(callable: F) -> Result<T, MemIsolateError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    T: Serialize + DeserializeOwned + Send + 'static,
{
    // TODO: Manage spans across async code
    // https://docs.rs/tracing/latest/tracing/span/struct.EnteredSpan.html#in-asynchronous-code
    let sys = Arc::new(get_system_functions());
    let PipeFds { read_fd, write_fd } = create_pipe(sys.clone()).await?;

    // NOTE: This fork must be blocking
    match fork(&*sys)? {
        ForkReturn::Child => {
            std::thread::spawn(move || {
                let rt =
                    tokio::runtime::Runtime::new().expect("failed to create new Tokio runtime");
                rt.block_on(async {
                    let writer = unsafe { File::from_raw_fd(write_fd) };
                    let writer = tokio::fs::File::from_std(writer);
                    let writer = Arc::new(Mutex::new(writer));
                    close_read_end_of_pipe_in_child_or_exit(sys.clone(), writer.clone(), read_fd)
                        .await;
                    let result = execute_callable(callable).await;
                    let encoded = serialize_result_or_error_value(result);
                    write_and_flush_or_exit(sys.clone(), writer.clone(), &encoded).await;
                    exit_happy(sys.clone())
                })
            })
            .join()
            .expect("failed to join on thread");
            exit_happy(sys.clone())
        }
        ForkReturn::Parent(child_pid) => {
            close_write_end_of_pipe_in_parent(sys.clone(), write_fd).await?;

            let waitpid_bespoke_status = wait_for_child(sys.clone(), child_pid).await?;
            error_if_child_unhappy(waitpid_bespoke_status)?;

            let buffer: Vec<u8> = read_all_of_child_result_pipe(read_fd).await?;
            deserialize_result(&buffer)
        }
    }
}
#[cfg_attr(feature = "tracing", instrument)]
async fn create_pipe<S: SystemFunctions>(sys: Arc<S>) -> Result<PipeFds, MemIsolateError> {
    let pipe_result = task::spawn_blocking(move || sys.pipe())
        .await
        .map_err(|join_err| CallableDidNotExecute(PipeCreationFailed(join_err.into())))?;
    let pipe_fds = match pipe_result {
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
async fn execute_callable<F, Fut, T>(callable: F) -> T
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    T: Serialize + DeserializeOwned + Send + 'static,
{
    debug!("starting execution of user-supplied callable");
    #[allow(clippy::let_and_return)]
    let result = {
        // #[cfg(feature = "tracing")]
        // TODO: This
        // let _span = span!(HIGHEST_LEVEL, "inside_callable").entered();
        callable().await
    };
    debug!("finished execution of user-supplied callable");
    result
}

#[cfg_attr(feature = "tracing", instrument)]
async fn wait_for_child<S: SystemFunctions>(
    sys: Arc<S>,
    child_pid: c_int,
) -> Result<WaitpidStatus, MemIsolateError> {
    let wait_result = task::spawn_blocking(move || sys.waitpid(child_pid))
        .await
        .map_err(|join_err| CallableStatusUnknown(WaitFailed(join_err.into())))?;

    let waitpid_bespoke_status = match wait_result {
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
async fn write_and_flush_or_exit<S: SystemFunctions>(
    sys: Arc<S>,
    writer: Arc<Mutex<impl AsyncWrite + Debug + Unpin>>,
    buffer: &[u8],
) {
    let mut writer_guard = writer.lock().await;
    let write_result = writer_guard.write_all(buffer).await;
    let result = match write_result {
        Ok(()) => writer_guard.flush().await,
        Err(e) => Err(e),
    };

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

fn exit_happy<S: SystemFunctions>(sys: Arc<S>) -> ! {
    // NOTE: We don't wrap this in #[cfg_attr(feature = "tracing", instrument)]
    // because doing so results in a compiler error because of the `!` return type
    // No idea why its usage is fine without the cfg_addr...
    #[cfg(feature = "tracing")]
    let _span = {
        const FN_NAME: &str = stringify!(exit_happy);
        span!(HIGHEST_LEVEL, FN_NAME).entered()
    };

    let exit_code = CHILD_EXIT_HAPPY;
    debug!("exiting child process with exit code: {}", exit_code);

    #[allow(clippy::used_underscore_items)]
    sys._exit(exit_code);
}

#[cfg_attr(feature = "tracing", instrument)]
async fn read_all_of_child_result_pipe(read_fd: c_int) -> Result<Vec<u8>, MemIsolateError> {
    // Read from the pipe by wrapping the read fd as a File
    let mut buffer = Vec::new();
    {
        let file = unsafe { File::from_raw_fd(read_fd) };
        // TODO: Determine if we need to drop the reader explicitly here
        // because this is a tokio::fs::File
        let mut reader = tokio::fs::File::from_std(file);
        if let Err(err) = reader.read_to_end(&mut buffer).await {
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
async fn close_write_end_of_pipe_in_parent<S: SystemFunctions>(
    sys: Arc<S>,
    write_fd: c_int,
) -> Result<(), MemIsolateError> {
    let close_result = task::spawn_blocking(move || sys.close(write_fd))
        .await
        .map_err(|join_err| CallableStatusUnknown(ParentPipeCloseFailed(join_err.into())))?;

    if let Err(err) = close_result {
        let err = CallableStatusUnknown(ParentPipeCloseFailed(err));
        error!("error closing write end of pipe, propagating {:?}", err);
        return Err(err);
    }
    debug!("write end of pipe closed successfully");
    Ok(())
}

#[cfg_attr(feature = "tracing", instrument)]
async fn close_read_end_of_pipe_in_child_or_exit<S: SystemFunctions>(
    sys: Arc<S>,
    writer: Arc<Mutex<impl AsyncWrite + Debug + Unpin>>,
    read_fd: c_int,
) {
    let sys_clone = sys.clone();
    let handle_error = async move |err: MemIsolateError| {
        error!(
            "error closing read end of pipe, now attempting to serialize error: {:?}",
            err
        );

        let encoded = bincode::serialize(&err).expect("failed to serialize error");
        // TODO: Any drops needed here?
        let mut writer_guard = writer.lock().await;
        writer_guard
            .write_all(&encoded)
            .await
            .expect("failed to write error to pipe");
        writer_guard
            .flush()
            .await
            .expect("failed to flush error to pipe");

        let exit_code = CHILD_EXIT_IF_READ_CLOSE_FAILED;
        error!("exiting child process with exit code: {}", exit_code);
        #[allow(clippy::used_underscore_items)]
        sys_clone._exit(exit_code);
    };

    match task::spawn_blocking(move || sys.close(read_fd)).await {
        Ok(Ok(())) => {
            debug!("read end of pipe closed successfully");
        }
        Ok(Err(err)) => {
            let err = CallableDidNotExecute(ChildPipeCloseFailed(Some(err)));
            handle_error(err).await;
        }
        Err(join_err) => {
            let err = CallableDidNotExecute(ChildPipeCloseFailed(Some(join_err.into())));
            handle_error(err).await;
        }
    }
}
