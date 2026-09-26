use std::io::stdout;

use crossterm::event::{DisableFocusChange, EnableFocusChange};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// Raw mode + alternate screen + focus reporting for the lifetime of the guard (RND-R5).
///
/// `ratatui::init` installs a panic hook that restores the terminal before the default
/// hook prints; `enter` wraps it so focus reporting is switched off too. The binary
/// wraps the result again so *caught* generator panics (ADR-0006) don't tear anything down.
#[derive(Debug)]
pub struct TerminalGuard {
    terminal: DefaultTerminal,
}

impl TerminalGuard {
    pub fn enter() -> Self {
        let terminal = ratatui::init();
        // Best effort: terminals without focus reporting simply never send the events.
        let _ = execute!(stdout(), EnableFocusChange);
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(stdout(), DisableFocusChange);
            prev(info);
        }));
        Self { terminal }
    }

    pub fn terminal(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), DisableFocusChange);
        ratatui::restore();
    }
}
