//! sciart scenes in Forces & Waves (`Category::{Physics}`, ADR-0013, ADR-0023).
//!
//! One module per scene; each is listed once in `sciart-generators`' `registry![…]`
//! (GEN-R3). Scenes import only `sciart_core`, `sciart_shared` and `rand`, never another
//! scene, so this crate has no `crate::` paths between modules (GEN-R16).

pub mod brownian_motion;
pub mod chladni_plate;
pub mod cloud_chamber;
pub mod doppler_effect;
pub mod double_pendulum;
pub mod kelvin_helmholtz;
pub mod particle_accelerator;
pub mod quantum_double_slit;
pub mod radioactive_decay;
pub mod rayleigh_benard;
pub mod strange_attractors;
pub mod three_body;
pub mod vortex_street;
pub mod wave_interference;
