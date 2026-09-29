//! Themes and ratatui painting for sciart canvases (SPEC-003 RND-*).

mod backend;
mod fields;
mod hud;
pub mod metrics;
mod paint;
mod terminal;
mod theme;

pub use backend::FrameBackend;
pub use hud::Hud;
pub use paint::paint;
pub use terminal::TerminalGuard;
pub use theme::{Rgb, Theme};
