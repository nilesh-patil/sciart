use ratatui::DefaultTerminal;

/// Raw mode + alternate screen for the lifetime of the guard (RND-R5).
///
/// `ratatui::init` also installs a panic hook that restores the terminal before the
/// default hook prints; the binary wraps that hook so *caught* generator panics
/// (ADR-0006) don't tear the terminal down.
#[derive(Debug)]
pub struct TerminalGuard {
    terminal: DefaultTerminal,
}

impl TerminalGuard {
    pub fn enter() -> Self {
        Self {
            terminal: ratatui::init(),
        }
    }

    pub fn terminal(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
