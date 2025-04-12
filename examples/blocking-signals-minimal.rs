//! See `blocking-signals-demonstration.rs` for a more detailed example of
//! how signal blocking works at runtime.
//!
//! Use this example for copy + paste code snippets.

use mem_isolate::{MemIsolateError, execute_in_isolated_process};
use nix::errno::Errno;
use nix::sys::signal::{SigSet, SigmaskHow, sigprocmask};

fn main() -> Result<(), MemIsolateError> {
    // Block all signals right before calling `mem_isolate::execute_in_isolated_process()`
    // This ensures the main program won't be killed leaving an orphaned child process
    let (block_signals, restore_signals) = get_block_and_restore_signal_closures();
    block_signals().expect("Failed to block signals");

    // Run your code in an isolated process
    let result = execute_in_isolated_process(|| ());

    // Restore the signal mask
    restore_signals().expect("Failed to restore signals");
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
