#![allow(unused_imports)]

use crate::macros::{debug, error};
#[cfg(feature = "tracing")]
// Don't import event macros like debug, error, etc. directly to avoid conflicts
// with our macros (see just above^)
use tracing::{Level, instrument, span};

use libc::c_int;
use std::fmt::Debug;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt};
use tokio::task;

use crate::c::{
    ForkReturn, PipeFds, SystemFunctions, WaitpidStatus, child_process_exited_on_its_own,
    child_process_killed_by_signal,
};

pub use crate::errors::MemIsolateError;
use crate::errors::{
    CallableDidNotExecuteError::{ChildPipeCloseFailed, ForkFailed, PipeCreationFailed},
    CallableExecutedError::{ChildPipeWriteFailed, DeserializationFailed, SerializationFailed},
    CallableStatusUnknownError::{
        CallableProcessDiedDuringExecution, ChildProcessKilledBySignal, ParentPipeCloseFailed,
        ParentPipeReadFailed, UnexpectedChildExitStatus, UnexpectedWaitpidReturnValue, WaitFailed,
    },
};
use crate::{deserialize_result, error_if_child_unhappy, get_system_functions};

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

    let fork_result = fork(sys.clone()).await?;
    match fork_result {
        ForkReturn::Child => {
            // TODO: Pick up here and continue implementing the child process
            todo!()
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

#[cfg_attr(feature = "tracing", instrument)]
async fn fork<S: SystemFunctions>(sys: Arc<S>) -> Result<ForkReturn, MemIsolateError> {
    let fork_result = task::spawn_blocking(move || sys.fork())
        .await
        .map_err(|join_err| CallableDidNotExecute(ForkFailed(join_err.into())))?;
    fork_result.map_err(|err| CallableDidNotExecute(ForkFailed(err)))
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
