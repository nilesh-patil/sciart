//! sciart scenes in Earth & Sky and Voyages (`Category::{EarthSky, Voyages}`, ADR-0013, ADR-0023).
//!
//! One module per scene; each is listed once in `sciart-generators`' `registry![…]`
//! (GEN-R3). Scenes import only `sciart_core`, `sciart_shared` and `rand`, never another
//! scene, so this crate has no `crate::` paths between modules (GEN-R16).

pub mod aurora;
pub mod constellation_map;
pub mod gravity_assist;
pub mod hurricane;
pub mod jwst_unfolding;
pub mod magnetosphere;
pub mod mars_rover;
pub mod meteor_shower;
pub mod moon_landing;
pub mod moon_phases;
pub mod noctilucent_clouds;
pub mod orbital_rendezvous;
pub mod rainbow;
pub mod rocket_launch;
pub mod satellite_orbit;
pub mod star_trails;
pub mod total_solar_eclipse;
pub mod voyager_journey;
