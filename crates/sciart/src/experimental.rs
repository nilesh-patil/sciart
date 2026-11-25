//! Write your own scene (feature `unstable-generator`, LIB-R3).
//!
//! **No semver promise.** Everything here is re-exported from sciart's internal crates and
//! may change in any release, including a patch release: the trait, its call protocol, the
//! canvas, the inks and the random-number generator behind [`Ctx`]. Pin an exact version
//! of `sciart` if you depend on it.
//!
//! A scene implements [`Generator`] and runs in a stage built by
//! [`Stage::from_generator`](crate::Stage::from_generator):
//!
//! ```
//! use sciart::experimental::prelude::*;
//! use sciart::{Options, Stage};
//!
//! static META: Meta = Meta {
//!     name: "blink",
//!     category: Category::Physics,
//!     description: "A dot that blinks",
//!     title: "Blink · a square wave",
//!     captions: &[],
//! };
//!
//! #[derive(Debug)]
//! struct Blink {
//!     on: bool,
//!     size: Size,
//!     clock: f64,
//! }
//!
//! impl Generator for Blink {
//!     fn meta(&self) -> &'static Meta {
//!         &META
//!     }
//!     fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
//!         self.size = size;
//!     }
//!     fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
//!         self.clock += dt;
//!         self.on = self.clock % 1.0 < 0.5;
//!     }
//!     fn render(&self, canvas: &mut Canvas) {
//!         if self.on {
//!             let (x, y) = (i32::from(self.size.width() / 2), i32::from(self.size.height() / 2));
//!             canvas.put(x, y, Glyph::new('●', 'o'), Ink::new(Role::Accent, 255));
//!         }
//!     }
//! }
//!
//! let blink = Blink { on: false, size: Size::new(1, 1), clock: 0.0 };
//! let mut stage = Stage::from_generator(Box::new(blink), 9, 3, Options::default().title(false))?;
//! stage.advance(0.25)?;
//! assert!(stage.render_text()?.contains('●'));
//! stage.advance(0.5)?;
//! assert!(!stage.render_text()?.contains('●'));
//! # Ok::<(), sciart::Error>(())
//! ```
//!
//! The rules the built-in scenes follow (never panic for any size of at least 1×1 or any
//! step up to 0.25 s, no wall clock or global random numbers, no allocation in `render`)
//! are in the project's SPEC-002; a scene that breaks them still runs, but its frames may
//! not be reproducible.

pub use sciart_core::prelude;
pub use sciart_core::{Canvas, Ctx, Generator, Glyph, Ink, Meta, Role, Size};
