use std::io::{self, Stdout, Write, stdout};

use crossterm::event::{DisableFocusChange, EnableFocusChange};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::Terminal;

use crate::FrameBackend;

/// The app's terminal: frames go out as one write each (RT-R15).
pub type AppTerminal = Terminal<FrameBackend<Stdout>>;

/// Raw mode + alternate screen + focus reporting for the lifetime of the guard (RND-R5).
///
/// Set up like `ratatui::init` (whose `restore` it uses), but with a [`FrameBackend`]:
/// the panic hook restores the terminal before the default hook prints, and also resets
/// the colours and shows the cursor, which the frame backend keeps across frames. The
/// binary wraps the hook again so *caught* generator panics (ADR-0006) don't tear anything
/// down.
#[derive(Debug)]
pub struct TerminalGuard {
    terminal: AppTerminal,
}

/// SGR reset + show cursor: what the shell expects after the alternate screen.
const RESTORE: &[u8] = b"\x1b[0m\x1b[?25h";

/// Focus reporting off, raw mode off, alternate screen left: the steps every exit shares.
fn leave() {
    let _ = execute!(stdout(), DisableFocusChange);
    ratatui::restore();
}

/// Put the terminal back the way the shell expects it without the guard: SGR reset, cursor
/// shown, then [`leave`]. For exits that cannot run the guard's `Drop`: the panic hook and
/// the termination-signal watcher (RT-R20). Idempotent, best effort (errors are ignored: the
/// terminal may already be gone after `SIGHUP`).
pub fn restore() {
    let mut out = stdout();
    let _ = out.write_all(RESTORE);
    let _ = out.flush();
    leave();
}

impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        // Best effort: terminals without focus reporting simply never send the events.
        let _ = execute!(stdout(), EnableFocusChange);
        let terminal = Terminal::new(FrameBackend::new(stdout()))?;
        Ok(Self { terminal })
    }

    pub fn terminal(&mut self) -> &mut AppTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.terminal.backend_mut().finish();
        let _ = self.terminal.show_cursor();
        leave();
    }
}
