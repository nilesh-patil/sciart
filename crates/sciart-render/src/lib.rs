//! Themes and ratatui painting for sciart canvases (SPEC-003 RND-*).

mod paint;
mod terminal;
mod theme;

pub use paint::paint;
pub use terminal::TerminalGuard;
pub use theme::{Rgb, Theme};
