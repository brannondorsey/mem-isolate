use mem_isolate::{MemIsolateError, execute_in_isolated_process};
use nix::errno::Errno;
use nix::sys::signal::{SigSet, SigmaskHow, kill, sigprocmask};
use nix::unistd::Pid;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), MemIsolateError> {
    let parent_pid = Pid::this();
    let dramatic_pause = Duration::from_secs(1);

    let (block_signals, restore_signals) = get_block_and_restore_signal_closures();
    println!("Parent: blocking signals\n");
    block_signals().expect("Failed to block signals");

    let result = execute_in_isolated_process(move || {
        // Don't try this at home, kids
        println!("Child: I'm a nasty lil' bugger, and I'm going to send a SIGTERM to my parent");
        thread::sleep(dramatic_pause); // Pause for dramatic effect
        kill(parent_pid, nix::sys::signal::Signal::SIGTERM)
            .expect("Failed to send SIGTERM from the child");
        println!("Child: SIGTERM sent to parent");
        thread::sleep(dramatic_pause); // Pause for dramatic effect
        println!("Child: I'm all done now, exiting\n");
    });

    println!("Parent: restoring signals in...");
    println!("Parent: 3...");
    thread::sleep(dramatic_pause);
    println!("Parent: 2...");
    thread::sleep(dramatic_pause);
    println!("Parent: 1...");
    thread::sleep(dramatic_pause);
    println!("Parent: restoring signals");
    restore_signals().expect("Failed to restore signals");
    println!("Parent: Notice that I never make it here, because the pending SIGTERM did me in");
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
