//! This example demonstrates how to block signals in a parent process while
//! executing code in an isolated process.
//!
//! This is useful if:
//!
//! 1. You want to simulate the expected behavior of running your code without
//!    `mem_isolate::execute_in_isolated_process()`, which would be gauranteed
//!    to treat signals the same both inside and outside of the `callable()`
//!    function.
//! 2. You want to prevent either process from being interrupted by signals
//!    while your `callable()` is running.
//! 3. You want to ensure that the parent process is not killed while the
//!    isolated process is running, leaving an orphaned child process.
//!
//! It is important to remember that `mem_isolate::execute_in_isolated_process()`
//! uses `fork()` under the hood, which creates a child process that will not
//! receive signals sent to the main process as your `callable()` would otherwise.
//!
//! Run this example with `cargo run --example blocking-signals-demonstration`
//!
//! This example is great for illustrating how signal blocking works, but if you
//! want to just copy and paste some code, see `blocking-signals-minimal.rs`
//!
//! NOTE: Because both SIGKILL and SIGSTOP are unblockable, nothing can be done
//! to prevent them from killing the parent process or the child process.

use mem_isolate::{MemIsolateError, execute_in_isolated_process};
use nix::errno::Errno;
use nix::sys::signal::{SigSet, SigmaskHow, sigprocmask};
use nix::unistd::Pid;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), MemIsolateError> {
    let parent_pid = Pid::this();
    println!("Parent PID: {}", parent_pid);
    let wait_time_for_readability = Duration::from_secs(5);

    // Get closures for blocking and restoring signals
    let (block_signals, restore_signals) = get_block_and_restore_signal_closures();

    // Block all signals before calling `mem_isolate::execute_in_isolated_process()`
    // This ensures the main program won't be killed leaving an orphaned child process
    println!("Parent: Blocking all signals");
    block_signals().expect("Failed to block signals");

    // Kick-off a subprocess that will send a SIGTERM to this process
    let sigterm_sender_proc = send_sigterm_to_parent(Duration::from_secs(1));

    // Run your code in an isolated process. NOTE: The child process created by
    // `fork()` inside `execute_in_isolated_process()` will inherit the signal
    // mask set by main process just above.
    let result = execute_in_isolated_process(move || {
        println!(
            "\nChild: I've started executing a user-defined callable. I'll wait for {} seconds before exiting...",
            wait_time_for_readability.as_secs()
        );
        thread::sleep(wait_time_for_readability);
        println!("Child: I'm all done now, exiting\n");
    });

    reap_child_process_so_it_doesnt_become_a_zombie(sigterm_sender_proc);

    println!(
        "Parent: Notice that the SIGTERM is pending and didn't interrupt this process or the child process. Unblocking signals in {} seconds...",
        wait_time_for_readability.as_secs()
    );
    thread::sleep(wait_time_for_readability);
    println!("Parent: Restoring signals, expect the parent to now recieve the SIGTERM");
    restore_signals().expect("Failed to restore signals");
    // WARNING: Don't expect code to ever reach this point, because the pending SIGTERM will kill the parent process
    // as soon as we unblock the SIGTERM that has been pending this whole time
    println!("Parent: Notice how I never ran");
    result
}

fn get_block_and_restore_signal_closures() -> (
    impl FnOnce() -> Result<(), Errno>,
    impl FnOnce() -> Result<(), Errno>,
) {
    let all_signals = SigSet::all();
    let mut old_signals = SigSet::empty();

    let block_signals = move || {
        sigprocmask(
            SigmaskHow::SIG_SETMASK,
            Some(&all_signals),
            Some(&mut old_signals),
        )
    };

    let restore_signals = move || sigprocmask(SigmaskHow::SIG_SETMASK, Some(&old_signals), None);

    (block_signals, restore_signals)
}

fn send_sigterm_to_parent(wait_time: Duration) -> Child {
    // We do this via a subprocess instead of a thread because the latter will
    // break the signal mask that we have set. `mem_isolate::execute_in_isolated_process()`
    // also SHOULD NOT be used in a multi-threaded program (see limitations in README)
    println!(
        "Parent: Sending SIGTERM to self in {} seconds. NOTE: This signal will be sent to the parent while the child is executing the user-defined callable",
        wait_time.as_secs()
    );
    Command::new("sh")
        .arg("-c")
        .arg(format!(
            "sleep {} && kill -s TERM {}",
            wait_time.as_secs(),
            Pid::this()
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn delayed kill command")
}

fn reap_child_process_so_it_doesnt_become_a_zombie(mut child: Child) {
    let exit_status = child.wait().expect("Failed to wait for child process");
    if !exit_status.success() {
        panic!("Other process: failed to send SIGTERM to parent process");
    }
}
