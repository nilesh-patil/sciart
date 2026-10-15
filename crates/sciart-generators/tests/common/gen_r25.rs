//! The GEN-R25 (M16, ADR-0019) thread-style and GEN-R26 (M17, ADR-0020) orbital-thread
//! scenes, shared by `m11_fidelity.rs` (GEN-R22 no longer pins them), `thread_style.rs` and
//! `orbital_style.rs` (which check these lists against SPEC-002).

/// The scenes GEN-R25 lists, in the spec's order.
#[allow(dead_code)]
pub const GEN_R25: &[&str] = &[
    "light_echo",
    "catalytic_surface",
    "saturn_rings",
    "dicty_aggregation",
    "total_solar_eclipse",
    "radical_chain",
    "comet_breakup",
    "lenia",
    "quantum_double_slit",
    "belousov_zhabotinsky",
    "jupiter_storms",
    "globular_cluster",
    "vortex_street",
    "jwst_unfolding",
    "rainbow",
    "hurricane",
    "relativistic_jet",
    "planetary_nebula",
    "cosmic_web",
    "hp_protein_folding",
    "galaxy_collision",
    "solar_system",
    "brownian_motion",
    "action_potential",
    "magnetosphere",
    "gravity_assist",
    "snowflake_growth",
    "cloud_chamber",
];

/// Whether GEN-R25 lists `scene`.
#[allow(dead_code)]
pub fn is_thread_scene(scene: &str) -> bool {
    GEN_R25.contains(&scene)
}

/// The scenes GEN-R26 lists, in the spec's order.
#[allow(dead_code)]
pub const GEN_R26: &[&str] = &[
    "gray_scott",
    "molecular_motors",
    "gravitational_lensing",
    "io_volcanoes",
    "spacetime_grid",
    "solar_eruption",
    "three_body",
    "spiral_galaxy",
    "supernova_remnant",
    "ising_model",
    "wave_interference",
    "chemical_garden",
    "physarum",
    "firefly_sync",
    "heart_fibrillation",
    "binary_inspiral",
    "chladni_plate",
    "rayleigh_benard",
    "mitosis",
    "ribosome_translation",
    "protocell_assembly",
];

/// Whether GEN-R25 or GEN-R26 restyled `scene` (so GEN-R22 no longer pins it).
#[allow(dead_code)]
pub fn is_restyled(scene: &str) -> bool {
    is_thread_scene(scene) || GEN_R26.contains(&scene)
}
