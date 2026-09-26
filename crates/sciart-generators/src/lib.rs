//! All sciart scenes and their registry (SPEC-002).
//!
//! Adding a generator (GEN-R3): create `src/<name>.rs` exposing `pub static META: Meta`
//! and `pub fn build() -> Box<dyn Generator>`, then add `<name>` to `registry!` below
//! (keep it alphabetical). The contract suite in `tests/contract.rs` covers it automatically.

use sciart_core::{Generator, Meta};

/// A registered generator: its metadata and a constructor.
#[derive(Debug)]
pub struct Entry {
    pub meta: &'static Meta,
    pub build: fn() -> Box<dyn Generator>,
}

macro_rules! registry {
    ($($module:ident),+ $(,)?) => {
        $(pub mod $module;)+

        static REGISTRY: &[Entry] = &[$(Entry { meta: &$module::META, build: $module::build }),+];
    };
}

registry![
    constellation_map,
    cosmic_microwave_background,
    doppler_effect,
    exoplanet_transit,
    gravitational_lensing,
    moon_phases,
    pulsar,
    satellite_orbit,
    solar_system,
    starfield,
    wave_interference,
];

/// Every generator, sorted by name (GEN-R1).
pub fn registry() -> &'static [Entry] {
    REGISTRY
}

/// Exact-name lookup (GEN-R2).
pub fn find(name: &str) -> Option<&'static Entry> {
    REGISTRY.iter().find(|e| e.meta.name == name)
}
