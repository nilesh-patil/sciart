//! The GEN-R25 (M16, ADR-0019) thread-style scenes, shared by `m11_fidelity.rs` (GEN-R22
//! no longer pins them) and `thread_style.rs` (which checks this list against SPEC-002).

/// The scenes GEN-R25 lists, in the spec's order.
#[allow(dead_code)]
pub const GEN_R25: &[&str] = &[
    "light_echo",
    "catalytic_surface",
    "saturn_rings",
    "dicty_aggregation",
    "total_solar_eclipse",
    "radical_chain",
    "supernova_remnant",
    "comet_breakup",
    "lenia",
    "quantum_double_slit",
    "belousov_zhabotinsky",
    "jupiter_storms",
    "ising_model",
    "globular_cluster",
    "vortex_street",
    "jwst_unfolding",
    "rainbow",
    "solar_eruption",
    "hurricane",
    "spiral_galaxy",
    "relativistic_jet",
    "gravitational_lensing",
    "spacetime_grid",
    "planetary_nebula",
    "cosmic_web",
    "hp_protein_folding",
];

/// Whether GEN-R25 lists `scene`.
#[allow(dead_code)]
pub fn is_thread_scene(scene: &str) -> bool {
    GEN_R25.contains(&scene)
}
