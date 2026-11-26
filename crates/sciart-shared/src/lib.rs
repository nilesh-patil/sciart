//! **Internal crate of [`sciart`](https://docs.rs/sciart)**, with no semver promise: its API
//! can change in any release. Use the `sciart` crate, the supported entry point.
//!
//! Models shared by several sciart scenes (GEN-R16, ADR-0023). A model with users in a
//! single scene crate lives in that crate as a helper module instead (GEN-R3).
//!
//! Not scenes, so never registered (GEN-R3). A shared model depends only on `sciart-core`
//! and `rand`, so it cannot import a scene; the GEN-R7 source rule scans it like every
//! scene file.

pub mod excitable;
pub mod plume;
pub mod skyline;
pub mod vortex_advect;
