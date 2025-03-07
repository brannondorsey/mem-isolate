use mem_isolate::{MemIsolateError, execute_in_isolated_process};

// Keep reading, this will soon make sense...
const MY_FUNCTION_IS_IDEMPOTENT: bool = true;
const PREFER_RETRIES_WITH_MEM_ISOLATE: bool = true;

// Your function can be fallible, so feel free to return a Result.
// Heck, you can return _anything_ that's serializable.
fn my_function() -> Result<String, String> {
    use rand::Rng;
    let mut rng = rand::rng();
    if rng.random::<f32>() > 0.5 {
        return Err("Your supplied function non-deterministically failed".to_string());
    }

    Ok("Your function executed without a hitch!".to_string())
}

fn main() -> Result<(), MemIsolateError> {
    // This function wraps your function's return type T in a Result<T,MemIsolateError>,
    // where MemIsolateError indiates something went wrong on the mem-isolate side of things.
    let mem_isolate_result = execute_in_isolated_process(my_function);

    // Your function's return value can be accessed by unwrapping the mem-isolate result
    let my_function_result = match mem_isolate_result {
        // Ok() signifies that the callable executed without a mem-isolate issue
        Ok(my_funct_result) => my_funct_result,
        Err(MemIsolateError::CallableExecuted(ref _err)) => {
            // The callable executed, but something went wrong afterwords.
            // For instance, maybe the data it returned failed serialization
            mem_isolate_result?
        }
        Err(MemIsolateError::CallableDidNotExecute(ref _err)) => {
            // Something went wrong before the callable was executed, you could
            // retry with or without mem_isolate

            // Because we know that the my_function never executed, it's
            // harmless to retry (even if !MY_FUNCTION_IS_IDEMPOTENT)
            if PREFER_RETRIES_WITH_MEM_ISOLATE {
                // You could naively retry with mem_isolate...
                execute_in_isolated_process(my_function)?
            } else {
                // ...or by directly calling the function without mem_isolate
                my_function()
            }
        }
        Err(MemIsolateError::CallableStatusUnknown(ref _err)) => {
            // Uh oh, something went wrong in a way that's impossible to know the
            // status of the callable
            if MY_FUNCTION_IS_IDEMPOTENT {
                // If the function is idempotent, you could retry without
                // mem_isolate
                my_function()
            } else {
                // Ruh, roh
                panic!("Callable is not idempotent, and we don't know the status of the callable");
            }
        }
    };

    // At last, we can handle the result of the function
    match my_function_result {
        Ok(result) => {
            println!("{result}");
        }
        Err(e) => {
            eprintln!("{e}");
        }
    }

    Ok(())
}

// Add a smoke test to make sure this example stays working
#[cfg(test)]
mod example_error_handling {
    use super::*;

    #[test]
    fn execute_main() {
        main().unwrap();
    }
}
