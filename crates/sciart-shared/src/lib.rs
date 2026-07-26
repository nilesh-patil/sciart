//! Models shared by several sciart scenes (GEN-R16, ADR-0023).
//!
//! Not scenes, so never registered (GEN-R3). A shared model depends only on `sciart-core`
//! and `rand`, so it cannot import a scene; the GEN-R7 source rule scans it like every
//! scene file.

pub mod cahn_hilliard;
pub mod excitable;
pub mod laplacian_growth;
pub mod plume;
pub mod skyline;
pub mod vortex_advect;
