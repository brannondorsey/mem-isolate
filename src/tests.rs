#![allow(clippy::unwrap_used)]

use super::*;
use crate::c::mock::{
    CallBehavior, MockConfig, configured_strict, configured_with_fallback, is_mocking_enabled,
    with_mock_system,
};
use errors::CallableDidNotExecuteError;
use errors::CallableExecutedError;
use errors::CallableStatusUnknownError;
use rstest::*;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::process;
use std::sync::mpsc::channel;
use std::thread;
use std::time;
use std::time::Duration;
use tempfile::NamedTempFile;

pub(crate) const TEST_TIMEOUT: Duration = Duration::from_secs(1);

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

fn error_eintr() -> io::Error {
    io::Error::from_raw_os_error(libc::EINTR)
}

fn write_pid_to_file(path: &Path) -> io::Result<()> {
    let pid: u32 = process::id();
    fs::write(path, format!("{pid}\n"))
}

#[derive(Debug, PartialEq)]
enum WaitForPidfileToPopulateResult {
    Success(i32),
    Timeout,
    Error(std::num::ParseIntError),
}

fn wait_for_pidfile_to_populate(
    path_with_eventual_pid: &Path,
    timeout: Duration,
) -> WaitForPidfileToPopulateResult {
    let start = time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            return WaitForPidfileToPopulateResult::Timeout;
        }
        if let Ok(child_pid) = fs::read_to_string(path_with_eventual_pid) {
            if child_pid.ends_with('\n') {
                match child_pid.trim().parse::<i32>() {
                    Ok(pid) => return WaitForPidfileToPopulateResult::Success(pid),
                    Err(e) => return WaitForPidfileToPopulateResult::Error(e),
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(feature = "tracing")]
#[ctor::ctor]
fn before_tests() {
    use tracing_subscriber::EnvFilter;
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
fn simple_example() {
    let result = execute_in_isolated_process(|| MyResult { value: 42 }).unwrap();
    assert_eq!(result, MyResult { value: 42 });
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
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

#[rstest]
#[timeout(TEST_TIMEOUT)]
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

#[rstest]
#[timeout(TEST_TIMEOUT)]
fn isolate_memory_leak() {
    fn check_memory_exists_and_holds_vec_data(ptr_str: &str) -> bool {
        let addr = usize::from_str_radix(ptr_str.trim_start_matches("0x"), 16).unwrap();
        let vec_ptr = addr as *const Vec<u8>;

        if vec_ptr.is_null() {
            return false;
        }

        // Use a recovery mechanism to safely check memory
        std::panic::catch_unwind(|| {
            // Safety: We're verifying if memory exists and has expected properties
            unsafe {
                let vec = &*vec_ptr;
                if vec.capacity() != 1024 || vec.len() != 1024 {
                    return false;
                }
                if vec.first() != Some(&42) {
                    return false;
                }
                // Memory exists and has the expected properties
                true
            }
        })
        .unwrap_or(false)
    }

    let leaky_fn = || {
        // Leak 1KiB of memory
        let data: Vec<u8> = vec![42; 1024];
        let data = Box::new(data);
        let uh_oh = Box::leak(data);
        let leaked_ptr = format!("{uh_oh:p}");
        assert!(
            check_memory_exists_and_holds_vec_data(&leaked_ptr),
            "The memory should exist in `leaky_fn()` where it was leaked"
        );
        leaked_ptr
    };

    let leaked_ptr: String = execute_in_isolated_process(leaky_fn).unwrap();
    assert!(
        !check_memory_exists_and_holds_vec_data(&leaked_ptr),
        "The leaked memory doesn't exist out here though"
    );
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
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
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            value: value.len() as i32,
        }
    };
    let result = execute_in_isolated_process(fn_once_closure).unwrap();
    assert_eq!(result, MyResult { value: 5 });
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
fn handle_large_result() {
    let initial_string = "The quick brown fox jumps over the lazy dog";
    let result = execute_in_isolated_process(|| initial_string.repeat(10000)).unwrap();
    assert_eq!(result.len(), 10000 * initial_string.len());
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
fn panic_in_child() {
    #[allow(clippy::semicolon_if_nothing_returned)]
    let error = execute_in_isolated_process(|| {
        // TODO: Figure out why this casues the child process to exit happily on its own
        // if buffer.is_empty() check is removed (resulting in a deserialization error)
        // Shouldn't the waitpid check result in noticing that the child panicked?
        panic!("Panic in child");
        #[allow(clippy::unused_unit)]
        #[allow(unreachable_code)]
        ()
    })
    .unwrap_err();
    eprintln!("error: {error:?}",);
    assert!(matches!(
        error,
        MemIsolateError::CallableStatusUnknown(
            CallableStatusUnknownError::CallableProcessDiedDuringExecution
        )
    ));
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
#[allow(clippy::unit_cmp)]
fn empty_result() {
    assert_eq!(execute_in_isolated_process(|| {}).unwrap(), ());
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
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
                "Expected error about sequence length, got: {err}"
            );
        }
        other => panic!("Expected SerializationFailed error, got: {other:?}"),
    }
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
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
                "Expected custom deserialization error, got: {err}"
            );
        }
        other => panic!("Expected DeserializationFailed error, got: {other:?}"),
    }
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
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

#[rstest]
#[timeout(TEST_TIMEOUT)]
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

#[rstest]
#[timeout(TEST_TIMEOUT)]
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

#[rstest]
#[timeout(TEST_TIMEOUT)]
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

            match result {
                Err(MemIsolateError::CallableStatusUnknown(
                    CallableStatusUnknownError::ParentPipeCloseFailed(err),
                )) => {
                    // Verify the error matches what we configured
                    let expected_error = error_eio();
                    assert_eq!(err.kind(), expected_error.kind());
                    assert_eq!(err.raw_os_error(), expected_error.raw_os_error());
                }
                other => panic!("Expected ParentPipeCloseFailed error, got: {other:?}"),
            }
        },
    );
}

// // TODO: Come back and fix this test.
// #[rstest]
// #[timeout(TEST_TIMEOUT)]
// fn parent_pipe_reader_invalid() {
//     use crate::c::PipeFds;

//     fn error_ebadf() -> io::Error {
//         io::Error::from_raw_os_error(libc::EBADF)
//     }

//     // Create a real pipe, but only

//     with_mock_system(
//         configured_with_fallback(move |mock| {
//             let invalid_read_fd = -100; // If this is a -1 we get another problem

//             let sys = c::RealSystemFunctions;
//             let PipeFds {
//                 read_fd: real_read_fd,
//                 write_fd: real_write_fd,
//             } = sys.pipe().expect("pipe should succeed");

//             // Close the real read_fd since we're replacing it with an invalid one
//             sys.close(real_read_fd)
//                 .expect("closing real read_fd should succeed");

//             mock.expect_pipe(CallBehavior::Mock(Ok(PipeFds {
//                 read_fd: invalid_read_fd,
//                 write_fd: real_write_fd,
//             })));
//         }),
//         |_| {
//             let result = execute_in_isolated_process(|| MyResult { value: 42 });

//             match result {
//                 Err(MemIsolateError::CallableStatusUnknown(
//                     CallableStatusUnknownError::ParentPipeReadFailed(err),
//                 )) => {
//                     // Verify the error matches what we configured
//                     let expected_error = error_ebadf();
//                     assert_eq!(err.kind(), expected_error.kind(), "1");
//                     assert_eq!(err.raw_os_error(), expected_error.raw_os_error(), "2");
//                 }
//                 other => panic!("Expected ParentPipeReadFailed error, got: {:?}", other),
//             }
//         },
//     );
// }

#[rstest]
#[timeout(TEST_TIMEOUT)]
fn waitpid_child_process_exited_on_its_own() {
    // The default case
    execute_in_isolated_process(|| {}).unwrap();
}

#[rstest]
#[timeout(Duration::from_secs(3))]
#[allow(clippy::semicolon_if_nothing_returned)]
fn waitpid_child_killed_by_signal() {
    let tmp_file = NamedTempFile::new().expect("Failed to create temp file");
    let tmp_path_clone = tmp_file.path().to_path_buf().clone();

    let callable = move || {
        write_pid_to_file(&tmp_path_clone).expect("Failed to write pid to temp file");
        // Wait for SIGTERM by parking the thread
        loop {
            thread::park();
        }
        #[allow(unreachable_code)]
        ()
    };

    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = execute_in_isolated_process(callable);
        tx.send(result)
    });

    let timeout = Duration::from_secs(2);
    if let WaitForPidfileToPopulateResult::Success(child_pid) =
        wait_for_pidfile_to_populate(tmp_file.path(), timeout)
    {
        // SIGTERM the child
        unsafe {
            libc::kill(child_pid, libc::SIGTERM);
        }
    } else {
        panic!("Failed to retrieve child pid from temp file");
    }

    let result = rx.recv().unwrap();
    assert!(matches!(
        result,
        Err(MemIsolateError::CallableStatusUnknown(
            CallableStatusUnknownError::ChildProcessKilledBySignal(libc::SIGTERM)
        ))
    ));
}

#[rstest]
#[timeout(Duration::from_secs(3))]
#[allow(clippy::semicolon_if_nothing_returned)]
fn waitpid_child_killed_by_signal_after_suspension_and_continuation() {
    let tmp_file = NamedTempFile::new().expect("Failed to create temp file");
    let tmp_path_clone = tmp_file.path().to_path_buf().clone();

    let callable = move || {
        write_pid_to_file(&tmp_path_clone).expect("Failed to write pid to temp file");
        // Wait for SIGTERM by parking the thread
        loop {
            thread::park();
        }
        #[allow(unreachable_code)]
        ()
    };

    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = execute_in_isolated_process(callable);
        tx.send(result)
    });

    let timeout = Duration::from_secs(2);
    if let WaitForPidfileToPopulateResult::Success(child_pid) =
        wait_for_pidfile_to_populate(tmp_file.path(), timeout)
    {
        unsafe {
            libc::kill(child_pid, libc::SIGSTOP);
        }

        thread::sleep(Duration::from_millis(100));
        unsafe {
            libc::kill(child_pid, libc::SIGCONT);
        }

        thread::sleep(Duration::from_millis(100));
        // No reason to choose SIGKILL here other than we already tested SIGTERM
        unsafe {
            libc::kill(child_pid, libc::SIGKILL);
        }
    } else {
        panic!("Failed to retrieve child pid from temp file");
    }

    let result = rx.recv().unwrap();
    assert!(matches!(
        result,
        Err(MemIsolateError::CallableStatusUnknown(
            CallableStatusUnknownError::ChildProcessKilledBySignal(libc::SIGKILL)
        ))
    ));
}

#[rstest]
#[timeout(TEST_TIMEOUT)]
fn waitpid_interrupted_by_signal_mock() {
    with_mock_system(
        configured_with_fallback(|mock| {
            // waitpid() will return EINTR if a signal is delivered to the parent,
            // we want to continue in this case. Here we mock the first three calls to
            // waitpid() returning EINTR, then the fourth call will fallback to the
            // real syscall.
            mock.expect_waitpid(CallBehavior::Mock(Err(error_eintr())));
            mock.expect_waitpid(CallBehavior::Mock(Err(error_eintr())));
            mock.expect_waitpid(CallBehavior::Mock(Err(error_eintr())));
        }),
        |_| {
            let result = execute_in_isolated_process(|| MyResult { value: 42 });
            assert_eq!(result.unwrap(), MyResult { value: 42 });
        },
    );
}
