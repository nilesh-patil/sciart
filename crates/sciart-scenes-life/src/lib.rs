//! **Internal crate of [`sciart`](https://docs.rs/sciart)**, with no semver promise: its API
//! can change in any release. Use the `sciart` crate, the supported entry point.
//!
//! sciart scenes in Living Systems, Physics of Life, Molecules of Life and Life from Matter (`Category::{Biology, Biophysics, Biochemistry, LivingMatter}`, ADR-0013, ADR-0023).
//!
//! One module per scene; each is listed once in `sciart-generators`' `registry![…]`
//! (GEN-R3). Scenes import only `sciart_core`, `sciart_shared` and `rand`, never another
//! scene, so this crate has no `crate::` paths between modules (GEN-R16).

pub mod action_potential;
pub mod boids_flocking;
pub mod dicty_aggregation;
pub mod dna_helix;
pub mod firefly_sync;
pub mod heart_fibrillation;
pub mod hp_protein_folding;
pub mod lenia;
pub mod mitosis;
pub mod molecular_motors;
pub mod motility_clustering;
pub mod physarum;
pub mod protocell_assembly;
pub mod ribosome_translation;
