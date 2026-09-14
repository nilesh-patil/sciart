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
                restore_within(RESTORE_BUDGET, || {
                    let held = io::stdout().lock();
                    sciart_render::restore_terminal();
                    held
                });
                std::process::exit(128 + signal);
            }
        })?;
    Ok(())
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

    #[test]
    fn rt_r20_a_stalled_restore_is_abandoned_at_the_budget() {
        #[allow(clippy::disallowed_methods)] // measures the bound itself
        let start = Instant::now();
        let finished = restore_within(Duration::from_millis(100), || {
            // A write that never completes: the stdout lock held by a stalled frame.
            thread::sleep(Duration::from_secs(30));
        });
        let took = start.elapsed();
        assert!(!finished);
        assert!(
            (Duration::from_millis(100)..Duration::from_millis(400)).contains(&took),
            "{took:?}"
        );
    }

    #[test]
    fn rt_r20_the_restore_keeps_its_lock_until_exit() {
        static LOCK: Mutex<()> = Mutex::new(());
        assert!(restore_within(Duration::from_secs(5), || LOCK
            .lock()
            .unwrap()));
        assert!(LOCK.try_lock().is_err(), "no frame may follow the restore");
    }

    #[test]
    fn rt_r20_the_budget_leaves_room_inside_250_ms() {
        assert!(RESTORE_BUDGET <= Duration::from_millis(200));
    }
}
