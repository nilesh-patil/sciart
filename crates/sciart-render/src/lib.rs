//! Themes and ratatui painting for sciart canvases (SPEC-003 RND-*).

mod backend;
mod color;
mod encode;
mod fields;
mod hud;
mod paint;
mod terminal;
mod theme;

pub use backend::FrameBackend;
pub use color::{indexed, quantize};
pub use hud::Hud;
pub use paint::{Background, HYSTERESIS, Painter, paint};
pub use terminal::{AppTerminal, TerminalGuard, restore as restore_terminal};
pub use theme::{Rgb, Theme};
