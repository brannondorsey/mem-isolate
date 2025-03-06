use mem_isolate::{MemIsolateError, execute_in_isolated_process};

const MY_FUNCTION_IS_IDEMPOTENT: bool = true;
fn my_function() -> Result<(), MemIsolateError> {
    Ok(())
}

fn main() -> Result<(), MemIsolateError> {
    let result = execute_in_isolated_process(my_function);

    match result {
        // Ok signifies that the callable executed without a mem-isolate issue, however its return value
        // may be a Result if it was fallible.
        Ok(dummy_func_return_val) => dummy_func_return_val,
        // The callable executed, but something went wrong afterwords
        // For instance, maybe the data it returned failed serialization
        Err(MemIsolateError::CallableExecuted(ref _err)) => {
            // NOTE: *ref* _err is used above to avoid consuming the error
            result?
        }
        // Something went wrong before the callable was executed, you could retry without mem_isolate
        Err(MemIsolateError::CallableDidNotExecute(_err)) => Ok(my_function()?),
        // Uh oh, something went wrong in a way that's impossible to know the status of the callable
        Err(MemIsolateError::CallableStatusUnknown(_err)) => {
            if MY_FUNCTION_IS_IDEMPOTENT {
                // If the function is idempotent, you could retry without mem_isolate
                Ok(my_function()?)
            } else {
                panic!("Callable is not idempotent, and we don't know the status of the callable");
            }
        }
    }
}
