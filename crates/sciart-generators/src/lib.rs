//! **Internal crate of [`sciart`](https://docs.rs/sciart)**, with no semver promise: its API
//! can change in any release. Use the `sciart` crate, the supported entry point.
//!
//! The registry of all sciart scenes (SPEC-002), over the category crates (ADR-0023).
//!
//! Adding a generator (GEN-R3): create `crates/sciart-scenes-<crate>/src/<name>.rs` in the
//! crate its `Meta.category` maps to (cosmos: Deep Cosmos, Sun & Planets; earth: Earth & Sky,
//! Voyages; physics: Forces & Waves; matter: Reactions, States of Matter; life: Living
//! Systems, Physics of Life, Molecules of Life, Life from Matter), exposing
//! `pub static META: Meta` and `pub fn build() -> Box<dyn Generator>`; add `pub mod <name>;`
//! to that crate's `lib.rs` and `<name> in sciart_scenes_<crate>` to `registry!` below (keep
//! it alphabetical). The suites in `tests/` cover it automatically.

use sciart_core::{Generator, Meta};

/// A registered generator: its metadata and a constructor.
#[derive(Debug)]
pub struct Entry {
    pub meta: &'static Meta,
    pub build: fn() -> Box<dyn Generator>,
}

macro_rules! registry {
    ($($module:ident in $krate:ident),+ $(,)?) => {
        // Re-exported so `sciart_generators::<scene>` paths keep working (ADR-0023).
        $(pub use $krate::$module;)+

        static REGISTRY: &[Entry] = &[$(Entry { meta: &$module::META, build: $module::build }),+];
    };
}

// One line per scene, alphabetical, naming the category crate its module lives in
// (GEN-R3, ADR-0023: `Meta.category` decides the crate; a source rule checks it).
registry![
    action_potential in sciart_scenes_life,
    aurora in sciart_scenes_earth,
    belousov_zhabotinsky in sciart_scenes_matter,
    binary_inspiral in sciart_scenes_cosmos,
    black_hole in sciart_scenes_cosmos,
    boids_flocking in sciart_scenes_life,
    brownian_motion in sciart_scenes_physics,
    catalytic_surface in sciart_scenes_matter,
    chemical_garden in sciart_scenes_matter,
    chladni_plate in sciart_scenes_physics,
    cloud_chamber in sciart_scenes_physics,
    comet_breakup in sciart_scenes_cosmos,
    comet_tails in sciart_scenes_cosmos,
    constellation_map in sciart_scenes_earth,
    cosmic_microwave_background in sciart_scenes_cosmos,
    cosmic_web in sciart_scenes_cosmos,
    dicty_aggregation in sciart_scenes_life,
    dna_helix in sciart_scenes_life,
    doppler_effect in sciart_scenes_physics,
    double_pendulum in sciart_scenes_physics,
    electron_orbitals in sciart_scenes_matter,
    exoplanet_transit in sciart_scenes_cosmos,
    firefly_sync in sciart_scenes_life,
    fractal_growth in sciart_scenes_matter,
    galaxy_collision in sciart_scenes_cosmos,
    globular_cluster in sciart_scenes_cosmos,
    gravitational_lensing in sciart_scenes_cosmos,
    gravity_assist in sciart_scenes_earth,
    gray_scott in sciart_scenes_matter,
    heart_fibrillation in sciart_scenes_life,
    hp_protein_folding in sciart_scenes_life,
    hurricane in sciart_scenes_earth,
    io_volcanoes in sciart_scenes_cosmos,
    ising_model in sciart_scenes_matter,
    jupiter_storms in sciart_scenes_cosmos,
    jwst_unfolding in sciart_scenes_earth,
    kelvin_helmholtz in sciart_scenes_physics,
    lenia in sciart_scenes_life,
    lichtenberg_discharge in sciart_scenes_matter,
    liesegang_rings in sciart_scenes_matter,
    light_echo in sciart_scenes_cosmos,
    magnetosphere in sciart_scenes_earth,
    mars_rover in sciart_scenes_earth,
    meteor_shower in sciart_scenes_earth,
    mitosis in sciart_scenes_life,
    molecular_dynamics in sciart_scenes_matter,
    molecular_motors in sciart_scenes_life,
    molecule_viewer in sciart_scenes_matter,
    moon_landing in sciart_scenes_earth,
    moon_phases in sciart_scenes_earth,
    motility_clustering in sciart_scenes_life,
    noctilucent_clouds in sciart_scenes_earth,
    orbital_rendezvous in sciart_scenes_earth,
    particle_accelerator in sciart_scenes_physics,
    physarum in sciart_scenes_life,
    planetary_nebula in sciart_scenes_cosmos,
    protocell_assembly in sciart_scenes_life,
    pulsar in sciart_scenes_cosmos,
    quantum_double_slit in sciart_scenes_physics,
    radical_chain in sciart_scenes_matter,
    radioactive_decay in sciart_scenes_physics,
    rainbow in sciart_scenes_earth,
    rayleigh_benard in sciart_scenes_physics,
    relativistic_jet in sciart_scenes_cosmos,
    ribosome_translation in sciart_scenes_life,
    rocket_launch in sciart_scenes_earth,
    satellite_orbit in sciart_scenes_earth,
    saturn_rings in sciart_scenes_cosmos,
    snowflake_growth in sciart_scenes_matter,
    solar_eruption in sciart_scenes_cosmos,
    solar_surface in sciart_scenes_cosmos,
    solar_system in sciart_scenes_cosmos,
    spacetime_grid in sciart_scenes_cosmos,
    spinodal_decomposition in sciart_scenes_matter,
    spiral_galaxy in sciart_scenes_cosmos,
    star_trails in sciart_scenes_earth,
    starfield in sciart_scenes_cosmos,
    stellar_nursery in sciart_scenes_cosmos,
    strange_attractors in sciart_scenes_physics,
    supernova_remnant in sciart_scenes_cosmos,
    three_body in sciart_scenes_physics,
    total_solar_eclipse in sciart_scenes_earth,
    vortex_street in sciart_scenes_physics,
    voyager_journey in sciart_scenes_earth,
    wave_interference in sciart_scenes_physics,
];

/// Every generator, sorted by name (GEN-R1).
pub fn registry() -> &'static [Entry] {
    REGISTRY
}

/// Retired scene names and the scenes that replaced them (GEN-R14), so old command lines
/// and config files keep working. Never listed by `registry()`.
pub static ALIASES: &[(&str, &str)] = &[
    ("chemical_reaction", "gray_scott"),
    ("crystal_growth", "fractal_growth"),
];

/// Exact-name lookup, falling back to a retired name (GEN-R2, GEN-R14).
pub fn find(name: &str) -> Option<&'static Entry> {
    let by_name = |name: &str| REGISTRY.iter().find(|e| e.meta.name == name);
    by_name(name).or_else(|| {
        let (_, new) = ALIASES.iter().find(|(old, _)| *old == name)?;
        by_name(new)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_r2_r14_retired_names_resolve_to_their_renamed_scene() {
        assert_eq!(
            find("chemical_reaction").map(|e| e.meta.name),
            Some("gray_scott")
        );
        assert_eq!(
            find("crystal_growth").map(|e| e.meta.name),
            Some("fractal_growth")
        );
        for &(old, new) in ALIASES {
            assert_eq!(find(old).map(|e| e.meta.name), Some(new), "{old}");
            assert!(
                REGISTRY.iter().all(|e| e.meta.name != old),
                "alias {old} shadows a registered scene"
            );
        }
    }
}
