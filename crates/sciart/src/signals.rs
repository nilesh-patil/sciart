//! Termination signals (SPEC-003 RT-R20): `SIGTERM` and `SIGHUP` restore the terminal and
//! exit with `128 + signal`.
//!
//! A watcher thread handles them, not the event loop: the loop may sit in a long
//! `event::poll` (a paused picture or the help page wait for input, RT-R17), and the
//! restore must not wait for it to come round. The watcher takes the stdout lock before it
//! restores and keeps it until the process exits, so no frame from the main thread can land
//! after the restore; each frame is one `write_all` under that same lock (RT-R15), so the
//! restore never splits a frame either.
//!
//! The keep-awake guard is not dropped here (it lives on the main thread), and does not need
//! to be: its helper already goes with the process however it ends — `caffeinate -w` sees
//! the exit (KA-R3) and the inhibitor's `cat` reads end-of-file (KA-R7, KA-R9).

use std::io;

use signal_hook::consts::{SIGHUP, SIGTERM};
use signal_hook::iterator::Signals;

/// Start the watcher. Call before the terminal enters raw mode, so there is no window in
/// which a signal would end the process with the terminal left raw.
pub fn watch() -> io::Result<()> {
    let mut signals = Signals::new([SIGTERM, SIGHUP])?;
    std::thread::Builder::new()
        .name("sciart-signals".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                let _hold = io::stdout().lock();
                sciart_render::restore_terminal();
                std::process::exit(128 + signal);
            }
        })?;
    Ok(())
}
