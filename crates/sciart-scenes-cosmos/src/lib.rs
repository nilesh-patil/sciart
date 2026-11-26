//! **Internal crate of [`sciart`](https://docs.rs/sciart)**, with no semver promise: its API
//! can change in any release. Use the `sciart` crate, the supported entry point.
//!
//! sciart scenes in Deep Cosmos and Sun & Planets (`Category::{DeepCosmos, SunPlanets}`, ADR-0013, ADR-0023).
//!
//! One module per scene; each is listed once in `sciart-generators`' `registry![…]`
//! (GEN-R3). Scenes import only `sciart_core`, `sciart_shared` and `rand`, never another
//! scene, so this crate has no `crate::` paths between modules (GEN-R16).

pub mod binary_inspiral;
pub mod black_hole;
pub mod comet_breakup;
pub mod comet_tails;
pub mod cosmic_microwave_background;
pub mod cosmic_web;
pub mod exoplanet_transit;
pub mod galaxy_collision;
pub mod globular_cluster;
pub mod gravitational_lensing;
pub mod io_volcanoes;
pub mod jupiter_storms;
pub mod light_echo;
pub mod planetary_nebula;
pub mod pulsar;
pub mod relativistic_jet;
pub mod saturn_rings;
pub mod solar_eruption;
pub mod solar_surface;
pub mod solar_system;
pub mod spacetime_grid;
pub mod spiral_galaxy;
pub mod starfield;
pub mod stellar_nursery;
pub mod supernova_remnant;
