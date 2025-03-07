use super::*;
use crate::c::mock::{
    CallBehavior, MockConfig, configured_strict, configured_with_fallback, is_mocking_enabled,
    with_mock_system,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::io;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct MyResult {
    value: i32,
}

fn error_eio() -> io::Error {
    io::Error::from_raw_os_error(libc::EIO)
}

fn error_eagain() -> io::Error {
    io::Error::from_raw_os_error(libc::EAGAIN)
}

fn error_enfile() -> io::Error {
    io::Error::from_raw_os_error(libc::ENFILE)
}

// TODO: Add test for memory leaks with Box::leak(). My first attempt at this proved challenging only in the detection mechanism.

#[test]
fn simple_example() {
    let result = execute_in_isolated_process(|| MyResult { value: 42 }).unwrap();
    assert_eq!(result, MyResult { value: 42 });
}

#[test]
#[allow(static_mut_refs)]
fn static_memory_mutation_without_isolation() {
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
fn static_memory_mutation_with_isolation() {
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
fn all_function_types() {
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

#[test]
fn serialization_error() {
    // Custom type that implements Serialize but fails during serialization
    #[derive(Debug)]
    struct CustomIteratorWrapper {
        _data: Vec<i32>,
    }

    impl Serialize for CustomIteratorWrapper {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("Fake serialization error"))
        }
    }

    impl<'de> Deserialize<'de> for CustomIteratorWrapper {
        fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Ok(CustomIteratorWrapper { _data: vec![] })
        }
    }

    // Use our function with a closure that returns the problematic type
    let result = execute_in_isolated_process(|| CustomIteratorWrapper {
        _data: vec![1, 2, 3],
    });

    // Verify we get the expected serialization error
    match result {
        Err(MemIsolateError::CallableExecuted(CallableExecutedError::SerializationFailed(err))) => {
            assert!(
                err.contains("Fake serialization error"),
                "Expected error about sequence length, got: {}",
                err
            );
        }
        other => panic!("Expected SerializationFailed error, got: {:?}", other),
    }
}

#[test]
fn deserialization_error() {
    // Custom type that successfully serializes but fails during deserialization
    #[derive(Debug, PartialEq)]
    struct DeserializationFailer {
        data: i32,
    }

    impl Serialize for DeserializationFailer {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            // Successfully serialize as a simple integer
            self.data.serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for DeserializationFailer {
        fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            // Always fail deserialization with a custom error
            Err(serde::de::Error::custom(
                "Intentional deserialization failure",
            ))
        }
    }

    // Use our function with a closure that returns the problematic type
    let result = execute_in_isolated_process(|| DeserializationFailer { data: 42 });

    // Verify we get the expected deserialization error
    match result {
        Err(MemIsolateError::CallableExecuted(CallableExecutedError::DeserializationFailed(
            err,
        ))) => {
            assert!(
                err.contains("Intentional deserialization failure"),
                "Expected custom deserialization error, got: {}",
                err
            );
        }
        other => panic!("Expected DeserializationFailed error, got: {:?}", other),
    }
}

#[test]
fn with_mock_helper() {
    with_mock_system(MockConfig::Fallback, |_| {
        // Test with active mocking
        // Check that mocking is properly configured
        assert!(is_mocking_enabled());

        // Test code that uses mocked functions
        let result = execute_in_isolated_process(|| MyResult { value: 42 }).unwrap();
        assert_eq!(result, MyResult { value: 42 });
    });

    // After with_mock_system, mocking is disabled automatically
    assert!(!is_mocking_enabled());
}

#[test]
fn pipe_error() {
    with_mock_system(
        // Pipe creation is the first syscall in execute_in_isolated_process so we can afford
        // to use strict mode here.
        configured_strict(|mock| {
            let pipe_creation_error = error_enfile();
            mock.expect_pipe(CallBehavior::Mock(Err(pipe_creation_error)));
        }),
        |_| {
            let result = execute_in_isolated_process(|| MyResult { value: 42 });
            let err = result.unwrap_err();
            matches!(
                err,
                MemIsolateError::CallableDidNotExecute(
                    CallableDidNotExecuteError::PipeCreationFailed(_)
                )
            );

            let pipe_creation_error = error_enfile();
            assert_eq!(
                err.source().unwrap().source().unwrap().to_string(),
                pipe_creation_error.to_string()
            );
        },
    );
}

#[test]
fn fork_error() {
    with_mock_system(
        configured_with_fallback(|mock| {
            let fork_error = error_eagain();
            mock.expect_fork(CallBehavior::Mock(Err(fork_error)));
        }),
        |_| {
            let result = execute_in_isolated_process(|| MyResult { value: 42 });
            let err = result.unwrap_err();
            matches!(
                err,
                MemIsolateError::CallableDidNotExecute(CallableDidNotExecuteError::ForkFailed(_))
            );

            let fork_error = error_eagain();
            assert_eq!(
                err.source().unwrap().source().unwrap().to_string(),
                fork_error.to_string()
            );
        },
    );
}

#[test]
fn parent_pipe_close_failure() {
    with_mock_system(
        configured_with_fallback(|mock| {
            // Let pipe() and fork() succeed normally
            // But make the first call to close() fail
            // This will affect the parent's attempt to close the write_fd
            let close_error = error_eio(); // I/O error
            mock.expect_close(CallBehavior::Mock(Err(close_error)));
        }),
        |_| {
            let result = execute_in_isolated_process(|| MyResult { value: 42 });

            // The close failure should result in a CallableStatusUnknown error
            match result {
                Err(MemIsolateError::CallableStatusUnknown(
                    CallableStatusUnknownError::ParentPipeCloseFailed(err),
                )) => {
                    // Verify the error matches what we configured
                    let expected_error = error_eio();
                    assert_eq!(err.kind(), expected_error.kind());
                    assert_eq!(err.raw_os_error(), expected_error.raw_os_error());
                }
                other => panic!("Expected ParentPipeCloseFailed error, got: {:?}", other),
            }
        },
    );
}
