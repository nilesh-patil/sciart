//! sciart scenes in Reactions and States of Matter (`Category::{Chemistry, PhysicalChemistry}`, ADR-0013, ADR-0023).
//!
//! One module per scene; each is listed once in `sciart-generators`' `registry![…]`
//! (GEN-R3). Scenes import only `sciart_core`, `sciart_shared` and `rand`, never another
//! scene, so this crate has no `crate::` paths between modules (GEN-R16).

pub mod belousov_zhabotinsky;
pub mod catalytic_surface;
pub mod chemical_garden;
pub mod electron_orbitals;
pub mod fractal_growth;
pub mod gray_scott;
pub mod ising_model;
pub mod lichtenberg_discharge;
pub mod liesegang_rings;
pub mod molecular_dynamics;
pub mod molecule_viewer;
pub mod radical_chain;
pub mod snowflake_growth;
pub mod spinodal_decomposition;
