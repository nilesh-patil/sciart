//! Themes and ratatui painting for sciart canvases (SPEC-003 RND-*).

//!
//! The `terminal` feature (on by default) adds the ratatui painting, the HUD and the
//! crossterm backend; without it the crate is the themes and the ANSI writer, which build
//! for the browser (WEB-R3, ADR-0034).

mod ansi;
#[cfg(feature = "terminal")]
mod backend;
mod color;
#[cfg(feature = "terminal")]
mod encode;
mod fields;
#[cfg(feature = "terminal")]
mod hud;
#[cfg(feature = "terminal")]
mod paint;
#[cfg(feature = "terminal")]
mod pipeline;
#[cfg(feature = "terminal")]
mod terminal;
mod theme;

pub use ansi::{AnsiColor, AnsiFraming, DEFAULT_FG, write_ansi};
#[cfg(feature = "terminal")]
pub use backend::FrameBackend;
#[cfg(feature = "terminal")]
pub use color::indexed;
pub use color::quantize;
#[cfg(feature = "terminal")]
pub use hud::Hud;
#[cfg(feature = "terminal")]
pub use paint::{Background, HYSTERESIS, Painter, paint};
#[cfg(feature = "terminal")]
pub use pipeline::{Compositor, Content, Draw, Work};
#[cfg(feature = "terminal")]
pub use terminal::{AppTerminal, TerminalGuard, restore as restore_terminal};
pub use theme::{Rgb, Theme};
