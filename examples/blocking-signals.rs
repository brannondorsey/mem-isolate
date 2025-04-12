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

    // Block all signals right before calling `mem_isolate::execute_in_isolated_process()`
    println!("Parent: blocking all signals");
    block_signals().expect("Failed to block signals");

    // Kick-off a subprocess that will send a SIGTERM to this process
    let sigterm_sender_proc = send_sigterm_to_parent(Duration::from_secs(1));

    // Execute the user-defined callable in an isolated process
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
    println!("Parent: restoring signals, expect the parent to now recieve the SIGTERM");
    restore_signals().expect("Failed to restore signals");
    // WARNING: Don't expect code to ever reach this point, because the pending SIGTERM will kill the parent process
    // as soon as we unblock the SIGTERM that has been pending this whole time

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
