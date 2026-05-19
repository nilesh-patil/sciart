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
    constellation_map,
    cosmic_microwave_background,
    cosmic_web,
    dna_helix,
    doppler_effect,
    double_pendulum,
    electron_orbitals,
    exoplanet_transit,
    fractal_growth,
    galaxy_collision,
    globular_cluster,
    gravitational_lensing,
    gray_scott,
    ising_model,
    jwst_unfolding,
    liesegang_rings,
    light_echo,
    mars_rover,
    molecular_dynamics,
    molecule_viewer,
    moon_landing,
    moon_phases,
    orbital_rendezvous,
    particle_accelerator,
    physarum,
    planetary_nebula,
    pulsar,
    radioactive_decay,
    relativistic_jet,
    rocket_launch,
    satellite_orbit,
    solar_surface,
    solar_system,
    spacetime_grid,
    spiral_galaxy,
    starfield,
    stellar_nursery,
    strange_attractors,
    supernova_remnant,
    three_body,
    vortex_street,
    voyager_journey,
    wave_interference,
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
