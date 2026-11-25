//! Termination signals (SPEC-003 RT-R20): `SIGTERM` and `SIGHUP` restore the terminal and
//! exit with `128 + signal` within 250 ms, whatever the event loop is doing.
//!
//! A watcher thread handles them, not the event loop: the loop may sit in a long
//! `event::poll` (a paused picture or the help page wait for input, RT-R21), and the exit
//! must not wait for it to come round.
//!
//! The restore runs under the stdout lock. Each frame is one `write_all` under that lock
//! (RT-R15), so the restore never splits a frame, and the lock is kept until the process
//! exits, so no later frame lands after the restore. `TerminalGuard::enter` sets the
//! terminal up under the same lock, so a signal that arrives during start-up restores
//! either before anything was changed (and start-up then never gets the lock) or after the
//! set-up is complete, never in between.
//!
//! The lock can be held indefinitely by a frame write that has stalled (a terminal or link
//! that stopped reading). So the restore runs on its own thread, the watcher waits for it at
//! most [`RESTORE_BUDGET`], and exits regardless: the restore is attempted, never awaited.
//!
//! The keep-awake guard's `Drop` does not run on this path (the guard lives on the main
//! thread), and does not need to: its helper ends with the process however it ends —
//! `caffeinate -w` sees the exit (KA-R3) and the inhibitor's `cat` reads end-of-file
//! (KA-R7, KA-R9).

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use signal_hook::consts::{SIGHUP, SIGTERM};
use signal_hook::iterator::Signals;

/// How long the exit waits for the restore: RT-R20's 250 ms less room for the exit itself.
pub const RESTORE_BUDGET: Duration = Duration::from_millis(150);

/// Start the watcher. Call before the terminal is set up, so no signal finds the terminal
/// set up and unwatched.
pub fn watch() -> io::Result<()> {
    let mut signals = Signals::new([SIGTERM, SIGHUP])?;
    thread::Builder::new()
        .name("sciart-signals".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                let code = on_signal(signal, RESTORE_BUDGET, || {
                    let held = io::stdout().lock();
                    sciart_render::restore_terminal();
                    held
                });
                std::process::exit(code);
            }
        })?;
    Ok(())
}

/// The signal path apart from the exit itself: attempt `restore` for at most `budget`,
/// then return the exit status `128 + signal` whether or not it finished.
fn on_signal<G>(
    signal: i32,
    budget: Duration,
    restore: impl FnOnce() -> G + Send + 'static,
) -> i32 {
    restore_within(budget, restore);
    128 + signal
}

/// Run `restore` on a thread of its own and wait for it at most `budget`; `true` when it
/// finished in time. What `restore` returns (the stdout lock) is kept by that thread until
/// the process exits.
fn restore_within<G>(budget: Duration, restore: impl FnOnce() -> G + Send + 'static) -> bool {
    let (done, finished) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("sciart-restore".into())
        .spawn(move || {
            let _held = restore();
            let _ = done.send(());
            loop {
                thread::park();
            }
        });
    spawned.is_ok() && finished.recv_timeout(budget).is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use super::*;

    #[test]
    fn rt_r20_a_prompt_restore_is_waited_for() {
        static RESTORED: AtomicBool = AtomicBool::new(false);
        assert!(restore_within(Duration::from_secs(5), || {
            RESTORED.store(true, Ordering::SeqCst);
        }));
        assert!(RESTORED.load(Ordering::SeqCst));
    }

    /// A restore that never completes: the stdout lock held by a stalled frame write.
    fn stalled() {
        loop {
            thread::park();
        }
    }

    // These tests assert only what a loaded machine cannot break: that the wait returns at
    // all (an unbounded wait hangs the test until nextest's timeout), and never before the
    // budget (`recv_timeout`'s guarantee). How promptly it returns is the scheduler's.

    #[test]
    fn rt_r20_a_stalled_restore_is_abandoned_after_the_budget() {
        let budget = Duration::from_millis(20);
        #[allow(clippy::disallowed_methods)] // a lower bound only
        let start = Instant::now();
        assert!(!restore_within(budget, stalled));
        assert!(start.elapsed() >= budget);
    }

    #[test]
    fn rt_r20_the_exit_status_comes_whether_or_not_the_restore_finished() {
        let budget = Duration::from_millis(20);
        assert_eq!(on_signal(SIGTERM, budget, stalled), 143);
        assert_eq!(on_signal(SIGHUP, budget, stalled), 129);
        assert_eq!(on_signal(SIGTERM, Duration::from_secs(5), || ()), 143);
    }

    #[test]
    fn rt_r20_the_restore_keeps_its_lock_until_exit() {
        static LOCK: Mutex<()> = Mutex::new(());
        assert!(restore_within(Duration::from_secs(5), || LOCK
            .lock()
            .unwrap()));
        assert!(LOCK.try_lock().is_err(), "no frame may follow the restore");
    }

    /// RT-R20 pins the deadline: 150 ms of the app's 250 ms budget, the rest for the exit.
    #[test]
    fn rt_r20_the_restore_deadline_is_150_ms() {
        assert_eq!(RESTORE_BUDGET, Duration::from_millis(150));
    }
}
