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
//! set-up is complete, never in between. Once the art has closed ([`SetUp`] dropped) there is
//! nothing to restore, and the restore is skipped: leaving the alternate screen a second
//! time would move the shell's cursor back to where the art started.
//!
//! The lock can be held indefinitely by a frame write that has stalled (a terminal or link
//! that stopped reading). So the restore runs on its own thread, the watcher waits for it at
//! most [`RESTORE_BUDGET`], and exits regardless: the restore is attempted, never awaited.
//!
//! Under `while` (CLI-R14) the command must not outlive the exit, and its output must not be
//! lost with it. The watcher kills the command first (a signal, no waiting), and the restore
//! thread replays what was spooled right after the restore, under the same lock and within
//! the same budget: a replay too long for it is cut short by the exit.
//!
//! The keep-awake guard's `Drop` does not run on this path (the guard lives on the main
//! thread), and does not need to: its helper ends with the process however it ends —
//! `caffeinate -w` sees the exit (KA-R3) and the inhibitor's `cat` reads end-of-file
//! (KA-R7, KA-R9).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use signal_hook::consts::{SIGHUP, SIGTERM};
use signal_hook::iterator::Signals;

use crate::child::{Child, Stopper};

/// How long the exit waits for the restore: RT-R20's 250 ms less room for the exit itself.
pub const RESTORE_BUDGET: Duration = Duration::from_millis(150);

/// Whether the watcher has been started.
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Whether the terminal may be set up, so a signal must restore it (see [`SetUp`]).
static SET_UP: AtomicBool = AtomicBool::new(false);

/// The `while` command, for the watcher to stop (CLI-R14).
static COMMAND: Mutex<Option<Stopper>> = Mutex::new(None);

fn command_slot() -> MutexGuard<'static, Option<Stopper>> {
    COMMAND.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Start the watcher. Call before the terminal is set up, so no signal finds the terminal
/// set up and unwatched. Later calls do nothing: `while` starts it before its command, and
/// the art then calls it again.
pub fn watch() -> io::Result<()> {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let started = Signals::new([SIGTERM, SIGHUP]).and_then(|mut signals| {
        thread::Builder::new()
            .name("sciart-signals".into())
            .spawn(move || {
                if let Some(signal) = signals.forever().next() {
                    exit_on(signal);
                }
            })
    });
    if started.is_err() {
        WATCHING.store(false, Ordering::SeqCst);
    }
    started.map(drop)
}

/// The watcher's exit: stop the command, restore the terminal, replay, exit `128 + signal`.
fn exit_on(signal: i32) -> ! {
    let command = command_slot().take();
    if let Some(command) = &command {
        command.kill();
    }
    restore_within(RESTORE_BUDGET, move || {
        let mut held = io::stdout().lock();
        if SET_UP.load(Ordering::SeqCst) {
            sciart_render::restore_terminal();
        }
        if let Some(command) = command {
            command.replay(&mut held, &mut io::stderr());
        }
        held
    });
    std::process::exit(128 + signal);
}

/// Start the `while` command with `start` and register it with the watcher (CLI-R14). The
/// watcher's slot is held meanwhile, so a signal that arrives while the command starts
/// waits for it and then stops it: there is no moment where the command runs unregistered.
pub fn start_command<E>(start: impl FnOnce() -> Result<Child, E>) -> Result<Child, E> {
    let mut slot = command_slot();
    let child = start()?;
    *slot = Some(child.stopper());
    Ok(child)
}

/// Marks the span in which the terminal may be set up, from before `TerminalGuard::enter`
/// to after the guard has restored it. Declare it before the guard so it drops after it.
#[derive(Debug)]
pub struct SetUp(());

impl SetUp {
    pub fn mark() -> Self {
        SET_UP.store(true, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for SetUp {
    fn drop(&mut self) {
        SET_UP.store(false, Ordering::SeqCst);
    }
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
    use std::ffi::OsString;
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

    #[test]
    fn rt_r20_cli_r14_the_terminal_counts_as_set_up_only_while_marked() {
        assert!(!SET_UP.load(Ordering::SeqCst));
        let marked = SetUp::mark();
        assert!(SET_UP.load(Ordering::SeqCst));
        drop(marked);
        assert!(
            !SET_UP.load(Ordering::SeqCst),
            "nothing to restore after the art"
        );
    }

    #[test]
    fn cli_r14_rt_r20_a_started_command_is_registered_for_the_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let argv = ["sleep", "30"].map(OsString::from);
        let failed = start_command(|| Child::start(&[], dir.path()));
        assert!(failed.is_err());
        assert!(
            command_slot().is_none(),
            "nothing registered for a failed start"
        );

        let mut child = start_command(|| Child::start(&argv, dir.path())).unwrap();
        let stopper = command_slot().take().expect("registered");
        stopper.kill();
        let status = loop {
            if let Some(status) = child.try_status().unwrap() {
                break status;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            crate::child::exit_code(status),
            128 + 9,
            "the watcher's kill"
        );
    }
}
