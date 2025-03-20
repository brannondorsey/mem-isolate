//! This example is meant to show you all of the possible errors that can occur
//! when using `execute_in_isolated_process()`.
//!
//! It doesn't actually do anything meaningful
use MemIsolateError::*;
use mem_isolate::errors::CallableDidNotExecuteError::*;
use mem_isolate::errors::CallableExecutedError::*;
use mem_isolate::errors::CallableStatusUnknownError::*;
use mem_isolate::{MemIsolateError, execute_in_isolated_process};

fn main() {
    let result = execute_in_isolated_process(|| {});
    match result {
        Ok(_) => {}

        Err(CallableExecuted(SerializationFailed)) => {}
        Err(CallableExecuted(DeserializationFailed(_string))) => {}
        Err(CallableExecuted(ChildPipeWriteFailed)) => {}

        Err(CallableDidNotExecute(PipeCreationFailed(_err))) => {}
        Err(CallableDidNotExecute(ChildPipeCloseFailed)) => {}
        Err(CallableDidNotExecute(ForkFailed(_err))) => {}

        Err(CallableStatusUnknown(ParentPipeCloseFailed(_err))) => {}
        Err(CallableStatusUnknown(WaitFailed(_err))) => {}
        Err(CallableStatusUnknown(ParentPipeReadFailed(_err))) => {}
        Err(CallableStatusUnknown(CallableProcessDiedDuringExecution)) => {}
        Err(CallableStatusUnknown(UnexpectedChildExitStatus(_status))) => {}
        Err(CallableStatusUnknown(ChildProcessKilledBySignal(_signal))) => {}
        Err(CallableStatusUnknown(UnexpectedWaitpidReturnValue(_val))) => {}
    };
}

// This test does two things:
// 1. It ensures our errors are being exposed publicly by the crate
// 2. It ensures our error handling is exhaustive and will fail if a change is made (flagging a semver breaking change)
#[cfg(test)]
mod example_error_handling_complete {
    use super::*;

    #[test]
    fn execute_main() {
        main();
    }
}
