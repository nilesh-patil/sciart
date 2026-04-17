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
    action_potential,
    aurora,
    belousov_zhabotinsky,
    black_hole,
    boids_flocking,
    brownian_motion,
    chemical_reaction,
    constellation_map,
    cosmic_microwave_background,
    crystal_growth,
    dna_helix,
    doppler_effect,
    double_pendulum,
    electron_orbitals,
    exoplanet_transit,
    galaxy_collision,
    globular_cluster,
    gravitational_lensing,
    ising_model,
    liesegang_rings,
    mars_rover,
    molecular_dynamics,
    molecule_viewer,
    moon_landing,
    moon_phases,
    particle_accelerator,
    physarum,
    pulsar,
    radioactive_decay,
    rocket_launch,
    satellite_orbit,
    solar_surface,
    solar_system,
    spacetime_grid,
    starfield,
    stellar_nursery,
    strange_attractors,
    three_body,
    vortex_street,
    voyager_journey,
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
