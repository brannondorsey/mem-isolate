use mem_isolate::{MemIsolateError, execute_in_isolated_process};

const MY_FUNCTION_IS_IDEMPOTENT: bool = true;
fn dummy_function() -> Result<(), MemIsolateError> {
    Ok(())
}

fn main() -> Result<(), MemIsolateError> {
    let result = execute_in_isolated_process(dummy_function);

    match result {
        Ok(dummy_func_return_val) => dummy_func_return_val,
        Err(MemIsolateError::CallableExecuted(ref _err)) => {
            // The callable executed, but something went wrong afterwords
            // For instance, maybe the data it returned failed serialization

            // NOTE: *ref* _err is used above to avoid consuming the error
            result?
        }
        Err(MemIsolateError::CallableDidNotExecute(_err)) => {
            // Something went wrong before the callable was executed, you could retry without mem_isolate
            Ok(dummy_function()?)
        }
        Err(MemIsolateError::CallableStatusUnknown(_err)) => {
            // Uh oh, something went wrong in a way that's impossible to know the status of the callable
            if MY_FUNCTION_IS_IDEMPOTENT {
                // If the function is idempotent, you could retry without mem_isolate
                Ok(dummy_function()?)
            } else {
                panic!("Callable is not idempotent, and we don't know the status of the callable");
            }
        }
    }
}
