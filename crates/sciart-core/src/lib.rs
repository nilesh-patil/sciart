//! Terminal-agnostic primitives for sciart (SPEC-001).
//!
//! Nothing in this crate touches the terminal, the OS, the wall clock or a global RNG:
//! generators built on it are deterministic and testable in isolation.

mod canvas;
mod clock;
mod ctx;
pub mod draw;
mod generator;
pub mod geom;
mod ink;
mod size;
mod stage;

pub use canvas::{Canvas, Cell, Glyph, GlyphMode};
pub use clock::FixedStep;
pub use ctx::Ctx;
pub use generator::{Category, Generator, Meta};
pub use ink::{Ink, Role};
pub use size::Size;
pub use stage::Stage;

/// Simulation step used by the runtime (ADR-0003).
pub const SIM_DT: f64 = 1.0 / 60.0;

/// Everything a generator module needs: `use sciart_core::prelude::*;`
pub mod prelude {
    pub use crate::{
        Canvas, Category, Cell, Ctx, FixedStep, Generator, Glyph, GlyphMode, Ink, Meta, Role,
        SIM_DT, Size, Stage,
    };
}
