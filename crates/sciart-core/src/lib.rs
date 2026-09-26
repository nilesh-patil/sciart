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
pub mod grid;
pub mod ink;
pub mod ode;
pub mod particles;
pub mod raster;
mod size;
mod stage;
pub mod timeline;

pub use canvas::{Canvas, Cell, Glyph, GlyphMode};
pub use clock::FixedStep;
pub use ctx::Ctx;
pub use generator::{Category, Generator, Meta};
pub use geom::Camera;
pub use grid::{Edges, Grid, Hood};
pub use ink::{Ink, Role};
pub use particles::{ParticlePool, Trail};
pub use raster::{Dots, Pixels};
pub use size::Size;
pub use stage::Stage;
pub use timeline::{Beat, Timeline, ease};

/// Simulation step used by the runtime (ADR-0003).
pub const SIM_DT: f64 = 1.0 / 60.0;

/// Everything a generator module needs: `use sciart_core::prelude::*;`
pub mod prelude {
    pub use crate::{
        Beat, Camera, Canvas, Category, Cell, Ctx, Dots, Edges, FixedStep, Generator, Glyph,
        GlyphMode, Grid, Hood, Ink, Meta, ParticlePool, Pixels, Role, SIM_DT, Size, Stage,
        Timeline, Trail,
    };
}
