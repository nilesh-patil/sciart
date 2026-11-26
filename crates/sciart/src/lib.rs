//! Real-time generative science art for the terminal: physics, chemistry, biology, Earth and
//! space, as a library (SPEC-007, ADR-0032).
//!
//! Run any of sciart's scenes without a terminal, list them, turn a frame into text, ANSI
//! or a PNG, or draw one inside a ratatui application. This crate also builds the `sciart`
//! binary; a library user turns its default features off, so the command line's
//! dependencies stay out:
//!
//! ```toml
//! [dependencies]
//! sciart = { version = "1", default-features = false, features = ["png", "widget"] }
//! ```
//!
//! | Feature | Adds |
//! |---|---|
//! | `png` | `Stage::render_png` |
//! | `widget` | `SceneWidget`, a ratatui 0.30 widget |
//! | `unstable-generator` | the `experimental` module: write your own scene, with no semver promise |
//! | `cli`, `record`, `mouse` | the binary (on by default) |
//!
//! # Headless stepping
//!
//! A [`Stage`] runs one scene at a fixed 1/60 s step. The same build, scene, seed, size
//! and calls give the same frames on the same platform; nothing is promised across
//! platforms or versions.
//!
//! ```
//! use sciart::{Options, Stage};
//!
//! let mut stage = Stage::new("aurora", 80, 24, Options::default().seed(7))?;
//! stage.advance(5.0)?;
//! assert_eq!(stage.time(), 5.0);
//! let text = stage.render_text()?;
//! assert_eq!(text.lines().count(), 24);
//! # Ok::<(), sciart::Error>(())
//! ```
//!
//! # Listing scenes
//!
//! ```
//! for scene in sciart::scenes() {
//!     println!("{}\t{}\t{}", scene.name(), scene.category().id(), scene.title());
//! }
//! let renamed = sciart::find("chemical_reaction").expect("a retired name still resolves");
//! assert_eq!(renamed.name(), "gray_scott");
//! ```
//!
//! # ANSI text
//!
//! One line per row, colours as truecolor SGR sequences, a reset at the end of every line:
//! the bytes `sciart frame` prints. Cells without a background of their own stay on the
//! terminal's background.
//!
//! ```
//! use sciart::{Options, Stage, Theme};
//!
//! let mut stage = Stage::new("solar_system", 60, 20, Options::default())?;
//! stage.advance(2.0)?;
//! print!("{}", stage.render_ansi(Theme::Astro)?);
//! # Ok::<(), sciart::Error>(())
//! ```
//!
//! # PNG
//!
//! With the `png` feature, at 8 × 16 pixels per cell:
//!
//! ```
//! # #[cfg(feature = "png")] {
//! use sciart::{Options, Stage, Theme};
//!
//! let mut stage = Stage::new("black_hole", 40, 12, Options::default())?;
//! stage.advance(1.0)?;
//! let mut png = Vec::new();
//! stage.render_png(Theme::Aurora, &mut png)?;
//! assert!(png.starts_with(b"\x89PNG"));
//! # }
//! # Ok::<(), sciart::Error>(())
//! ```
//!
//! # A ratatui widget
//!
//! With the `widget` feature, `SceneWidget` draws a stage into any area of a frame; the
//! stage takes the area's size. Advance it between draws by the wall time that passed.
//!
//! ```
//! # #[cfg(feature = "widget")] {
//! use ratatui::{Terminal, backend::TestBackend};
//! use sciart::{Options, SceneWidget, Stage, Theme};
//!
//! let mut stage = Stage::new("starfield", 1, 1, Options::default())?;
//! let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("a test terminal");
//! for _ in 0..3 {
//!     terminal
//!         .draw(|frame| frame.render_widget(SceneWidget::new(&mut stage).theme(Theme::Astro), frame.area()))
//!         .expect("draw");
//!     stage.advance(1.0 / 30.0)?;
//! }
//! assert_eq!(stage.size(), (40, 12));
//! # }
//! # Ok::<(), sciart::Error>(())
//! ```
//!
//! # Panics
//!
//! Scenes are tested never to panic for any size and step, but if one does, the stage
//! catches it and reports an error of kind [`ErrorKind::ScenePanicked`] from then on,
//! provided the crate is built with unwinding (the default). Built with `panic = "abort"`, a
//! panic in a scene aborts the process. The library never installs a panic hook and never touches the terminal, so the
//! default hook still prints the panic message to stderr unless the application sets its
//! own.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

mod error;
#[cfg(feature = "unstable-generator")]
pub mod experimental;
mod scene;
mod stage;
mod theme;
#[cfg(feature = "widget")]
mod widget;

pub use error::{Error, ErrorKind};
pub use scene::{Category, SceneInfo, find, scenes};
pub use stage::{Options, Stage};
pub use theme::Theme;
#[cfg(feature = "widget")]
pub use widget::SceneWidget;
