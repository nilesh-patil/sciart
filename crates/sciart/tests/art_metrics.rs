//! Per-scene art ratchets, measured with the RND-R13 metrics (`sciart_render::metrics`) on
//! the painted `astro` frame at 200×50, seed 42, t = 5 s and 20 s: the same numbers
//! `pixi run rs-cost` prints.
//!
//! `LIMITS` holds, per scene and time, a blockiness *ceiling* (GEN-R17), a darkness
//! *floor* (GEN-R18), and *ceilings* on lit coverage, chroma mass and edge weight (GEN-R21,
//! ADR-0017: clean, minimal scenes on black); the GEN-R21 neon cap (chroma p90 ≤ 0.20) is
//! one global ceiling. GEN-R21's legible subject adds a per-scene lit-coverage *floor* and a
//! global floor on the p99 luma of lit pixels (`LEGIBLE_LUMA`, allowlist `KNOWN_FAINT`).
//! Seeded with slack (blockiness +0.02, darkness −0.02, coverage
//! +0.02, chroma mass +0.003, edge weight +10 % + 0.2, coverage floor −0.02); `pixi run rs-cost -- --emit-limits`
//! prints fresh rows. Ratchet: a ceiling may only be lowered and a floor only raised; when
//! a scene improves, tighten its entry (the tests print loose entries). Raising a GEN-R21
//! ceiling needs a reviewed note in the scene's SPEC-002 section.
//! `cargo test -p sciart --test art_metrics -- --nocapture` prints the headroom.

// Measured table values that happen to look like 1/π are not the constant.
#![allow(clippy::approx_constant)]

use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_render::metrics::{ArtMetrics, LEGIBLE_LUMA, NEON_CAP, art_metrics};
use sciart_render::{Theme, paint};

const SEED: u64 = 42;
const SIZE: (u16, u16) = (200, 50);
const TIMES: [f64; 2] = [5.0, 20.0];

/// Per scene, each at `TIMES`: blockiness ceiling, darkness floor, the GEN-R21 ceilings on
/// lit coverage, chroma mass and edge weight, and the GEN-R21 lit-coverage floor.
/// Alphabetical.
type Limits = (
    &'static str,
    [f64; 2],
    [f64; 2],
    [f64; 2],
    [f64; 2],
    [f64; 2],
    [f64; 2],
);

/// Scenes whose brightest lit pixels (p99 luma) are still below `LEGIBLE_LUMA` (GEN-R21,
/// ADR-0017 rule 6), as measured when the rule landed. An allowlist that may only shrink:
/// a listed scene that passes fails the test until its entry is deleted.
const KNOWN_FAINT: &[&str] = &[
    "firefly_sync",
    "liesegang_rings",
    "light_echo",
    "planetary_nebula",
    "solar_eruption",
    "supernova_remnant",
];

#[rustfmt::skip]
const LIMITS: &[Limits] = &[
    ("action_potential", [0.020, 0.045], [0.911, 0.911], [0.105, 0.105], [0.0086, 0.0084], [5.1, 5.1], [0.064, 0.064]),
    ("aurora", [0.034, 0.031], [0.844, 0.706], [0.162, 0.302], [0.0144, 0.0277], [6.3, 8.5], [0.121, 0.261]),
    ("belousov_zhabotinsky", [0.020, 0.020], [0.751, 0.695], [0.254, 0.308], [0.0240, 0.0294], [14.6, 12.3], [0.213, 0.267]),
    ("black_hole", [0.020, 0.020], [0.872, 0.876], [0.130, 0.126], [0.0104, 0.0101], [8.9, 8.9], [0.089, 0.085]),
    ("boids_flocking", [0.020, 0.020], [0.980, 0.980], [0.046, 0.046], [0.0032, 0.0032], [1.4, 1.4], [0.005, 0.005]),
    ("brownian_motion", [0.020, 0.020], [0.974, 0.974], [0.032, 0.034], [0.0033, 0.0034], [3.1, 2.7], [0.000, 0.000]),
    ("catalytic_surface", [0.020, 0.020], [0.713, 0.948], [0.290, 0.053], [0.0288, 0.0045], [14.9, 2.0], [0.250, 0.012]),
    ("chemical_garden", [0.020, 0.020], [0.926, 0.892], [0.075, 0.110], [0.0075, 0.0111], [4.4, 4.7], [0.034, 0.069]),
    ("comet_breakup", [0.020, 0.020], [0.852, 0.964], [0.150, 0.036], [0.0128, 0.0034], [25.3, 4.4], [0.109, 0.000]),
    ("comet_tails", [0.020, 0.020], [0.884, 0.760], [0.117, 0.244], [0.0073, 0.0082], [10.7, 21.2], [0.076, 0.203]),
    ("constellation_map", [0.020, 0.020], [0.978, 0.978], [0.028, 0.025], [0.0033, 0.0032], [1.5, 1.5], [0.000, 0.000]),
    ("cosmic_microwave_background", [0.032, 0.020], [0.532, 0.539], [0.477, 0.470], [0.0206, 0.0195], [8.2, 7.9], [0.436, 0.429]),
    ("cosmic_web", [0.020, 0.020], [0.807, 0.802], [0.193, 0.198], [0.0228, 0.0237], [9.5, 8.9], [0.152, 0.157]),
    ("dicty_aggregation", [0.020, 0.020], [0.926, 0.955], [0.109, 0.067], [0.0090, 0.0057], [1.9, 2.0], [0.068, 0.026]),
    ("dna_helix", [0.020, 0.020], [0.980, 0.980], [0.040, 0.058], [0.0042, 0.0054], [1.4, 1.3], [0.000, 0.017]),
    ("doppler_effect", [0.020, 0.020], [0.980, 0.980], [0.104, 0.060], [0.0063, 0.0041], [1.4, 1.8], [0.063, 0.019]),
    ("double_pendulum", [0.020, 0.020], [0.946, 0.943], [0.064, 0.066], [0.0065, 0.0066], [3.3, 3.0], [0.023, 0.025]),
    ("electron_orbitals", [0.020, 0.020], [0.980, 0.980], [0.070, 0.073], [0.0051, 0.0052], [1.8, 1.9], [0.029, 0.032]),
    ("exoplanet_transit", [0.064, 0.064], [0.843, 0.843], [0.167, 0.167], [0.0114, 0.0114], [16.0, 15.9], [0.126, 0.126]),
    ("firefly_sync", [0.020, 0.020], [0.939, 0.948], [0.062, 0.053], [0.0042, 0.0033], [2.4, 2.5], [0.021, 0.012]),
    ("fractal_growth", [0.020, 0.020], [0.980, 0.980], [0.037, 0.074], [0.0037, 0.0033], [1.4, 1.4], [0.000, 0.033]),
    ("galaxy_collision", [0.020, 0.020], [0.980, 0.980], [0.047, 0.042], [0.0038, 0.0036], [1.5, 1.7], [0.006, 0.001]),
    ("globular_cluster", [0.020, 0.020], [0.931, 0.930], [0.081, 0.082], [0.0039, 0.0039], [5.7, 6.0], [0.040, 0.041]),
    ("gravitational_lensing", [0.020, 0.020], [0.937, 0.947], [0.064, 0.053], [0.0071, 0.0059], [6.3, 5.4], [0.023, 0.012]),
    ("gray_scott", [0.020, 0.020], [0.783, 0.335], [0.217, 0.666], [0.0068, 0.0168], [9.9, 16.6], [0.176, 0.625]),
    ("heart_fibrillation", [0.020, 0.020], [0.950, 0.940], [0.056, 0.067], [0.0050, 0.0064], [2.4, 2.6], [0.015, 0.026]),
    ("hp_protein_folding", [0.020, 0.020], [0.969, 0.970], [0.032, 0.036], [0.0036, 0.0036], [2.7, 2.1], [0.000, 0.000]),
    ("hurricane", [0.020, 0.020], [0.648, 0.645], [0.359, 0.361], [0.0108, 0.0108], [56.5, 61.5], [0.318, 0.320]),
    ("io_volcanoes", [0.024, 0.024], [0.681, 0.681], [0.339, 0.339], [0.0243, 0.0243], [13.4, 13.7], [0.298, 0.298]),
    ("ising_model", [0.196, 0.195], [0.655, 0.860], [0.350, 0.142], [0.0031, 0.0031], [23.3, 21.3], [0.309, 0.101]),
    ("jupiter_storms", [0.020, 0.020], [0.655, 0.655], [0.352, 0.352], [0.0321, 0.0322], [81.7, 82.1], [0.311, 0.311]),
    ("jwst_unfolding", [0.046, 0.056], [0.913, 0.785], [0.089, 0.221], [0.0069, 0.0099], [9.7, 23.5], [0.048, 0.180]),
    ("kelvin_helmholtz", [0.020, 0.020], [0.859, 0.908], [0.143, 0.093], [0.0056, 0.0045], [2.8, 2.9], [0.102, 0.052]),
    ("lenia", [0.020, 0.020], [0.863, 0.862], [0.139, 0.138], [0.0179, 0.0178], [14.6, 13.8], [0.098, 0.097]),
    ("lichtenberg_discharge", [0.020, 0.020], [0.980, 0.980], [0.069, 0.037], [0.0054, 0.0040], [1.5, 1.5], [0.028, 0.000]),
    ("liesegang_rings", [0.020, 0.020], [0.887, 0.810], [0.128, 0.206], [0.0156, 0.0261], [4.9, 6.5], [0.087, 0.165]),
    ("light_echo", [0.034, 0.049], [0.766, 0.853], [0.239, 0.149], [0.0257, 0.0154], [21.3, 17.5], [0.198, 0.108]),
    ("mars_rover", [0.118, 0.123], [0.509, 0.507], [0.485, 0.486], [0.0358, 0.0359], [25.6, 25.7], [0.444, 0.445]),
    ("meteor_shower", [0.020, 0.020], [0.781, 0.781], [0.230, 0.230], [0.0042, 0.0042], [11.8, 11.7], [0.189, 0.189]),
    ("molecular_dynamics", [0.020, 0.020], [0.980, 0.980], [0.069, 0.068], [0.0038, 0.0039], [2.7, 2.7], [0.028, 0.027]),
    ("molecular_motors", [0.020, 0.020], [0.896, 0.898], [0.106, 0.103], [0.0072, 0.0078], [5.3, 5.7], [0.065, 0.062]),
    ("molecule_viewer", [0.056, 0.020], [0.934, 0.937], [0.067, 0.064], [0.0031, 0.0031], [12.0, 5.4], [0.026, 0.023]),
    ("moon_landing", [0.020, 0.020], [0.772, 0.734], [0.217, 0.259], [0.0082, 0.0090], [21.9, 21.0], [0.176, 0.218]),
    ("moon_phases", [0.423, 0.039], [0.687, 0.749], [0.322, 0.258], [0.0081, 0.0076], [48.5, 41.0], [0.281, 0.217]),
    ("motility_clustering", [0.020, 0.020], [0.980, 0.980], [0.045, 0.207], [0.0039, 0.0089], [1.4, 1.4], [0.004, 0.166]),
    ("noctilucent_clouds", [0.020, 0.020], [0.949, 0.951], [0.237, 0.280], [0.0119, 0.0128], [2.2, 2.9], [0.196, 0.239]),
    ("orbital_rendezvous", [0.020, 0.020], [0.816, 0.816], [0.176, 0.176], [0.0172, 0.0171], [9.3, 9.3], [0.135, 0.135]),
    ("particle_accelerator", [0.020, 0.020], [0.979, 0.979], [0.046, 0.041], [0.0041, 0.0039], [1.7, 1.6], [0.005, 0.000]),
    ("physarum", [0.046, 0.071], [0.861, 0.841], [0.141, 0.162], [0.0159, 0.0177], [12.7, 11.3], [0.100, 0.121]),
    ("planetary_nebula", [0.020, 0.020], [0.850, 0.835], [0.153, 0.168], [0.0147, 0.0162], [9.9, 25.2], [0.112, 0.127]),
    ("pulsar", [0.020, 0.020], [0.947, 0.928], [0.054, 0.073], [0.0070, 0.0090], [7.8, 5.5], [0.013, 0.032]),
    ("quantum_double_slit", [0.020, 0.020], [0.578, 0.593], [0.432, 0.424], [0.0238, 0.0224], [12.1, 10.1], [0.391, 0.383]),
    ("radical_chain", [0.020, 0.020], [0.770, 0.958], [0.235, 0.042], [0.0286, 0.0056], [43.6, 9.4], [0.194, 0.001]),
    ("radioactive_decay", [0.020, 0.020], [0.913, 0.957], [0.101, 0.058], [0.0128, 0.0064], [1.7, 1.6], [0.060, 0.017]),
    ("rainbow", [0.099, 0.147], [0.680, 0.671], [0.326, 0.335], [0.0469, 0.0432], [23.4, 22.1], [0.285, 0.294]),
    ("relativistic_jet", [0.020, 0.020], [0.845, 0.845], [0.157, 0.158], [0.0136, 0.0136], [14.9, 15.0], [0.116, 0.117]),
    ("rocket_launch", [0.020, 0.020], [0.959, 0.968], [0.043, 0.035], [0.0034, 0.0032], [2.8, 2.2], [0.002, 0.000]),
    ("satellite_orbit", [0.036, 0.034], [0.910, 0.904], [0.108, 0.114], [0.0102, 0.0107], [5.4, 5.8], [0.067, 0.073]),
    ("saturn_rings", [0.020, 0.020], [0.751, 0.751], [0.253, 0.254], [0.0132, 0.0132], [28.3, 27.8], [0.212, 0.213]),
    ("seashell_pigmentation", [0.056, 0.078], [0.848, 0.856], [0.154, 0.147], [0.0086, 0.0076], [44.1, 43.6], [0.113, 0.106]),
    ("solar_eruption", [0.020, 0.020], [0.530, 0.533], [0.462, 0.459], [0.0268, 0.0300], [16.6, 19.9], [0.421, 0.418]),
    ("solar_surface", [0.020, 0.020], [0.612, 0.612], [0.397, 0.396], [0.0509, 0.0506], [53.7, 54.6], [0.356, 0.355]),
    ("solar_system", [0.067, 0.048], [0.938, 0.938], [0.108, 0.108], [0.0082, 0.0081], [2.3, 2.3], [0.067, 0.067]),
    ("spacetime_grid", [0.020, 0.020], [0.971, 0.971], [0.154, 0.154], [0.0103, 0.0102], [1.5, 1.5], [0.113, 0.113]),
    ("spinodal_decomposition", [0.020, 0.020], [0.743, 0.840], [0.257, 0.161], [0.0080, 0.0059], [2.7, 2.7], [0.216, 0.120]),
    ("spiral_galaxy", [0.020, 0.020], [0.713, 0.713], [0.292, 0.292], [0.0093, 0.0090], [23.9, 23.4], [0.251, 0.251]),
    ("star_trails", [0.020, 0.020], [0.968, 0.943], [0.064, 0.118], [0.0036, 0.0042], [1.7, 1.9], [0.023, 0.077]),
    ("starfield", [0.020, 0.020], [0.980, 0.980], [0.056, 0.057], [0.0033, 0.0033], [1.4, 1.4], [0.015, 0.016]),
    ("stellar_nursery", [0.020, 0.024], [0.721, 0.753], [0.288, 0.251], [0.0138, 0.0130], [8.4, 8.9], [0.247, 0.210]),
    ("strange_attractors", [0.020, 0.020], [0.980, 0.980], [0.082, 0.062], [0.0057, 0.0046], [1.6, 1.8], [0.041, 0.021]),
    ("supernova_remnant", [0.020, 0.020], [0.932, 0.858], [0.069, 0.144], [0.0066, 0.0149], [5.1, 8.8], [0.028, 0.103]),
    ("three_body", [0.020, 0.020], [0.980, 0.980], [0.031, 0.031], [0.0036, 0.0035], [1.4, 1.4], [0.000, 0.000]),
    ("total_solar_eclipse", [0.031, 0.020], [0.893, 0.898], [0.109, 0.103], [0.0038, 0.0041], [10.8, 8.6], [0.068, 0.062]),
    ("vortex_street", [0.024, 0.020], [0.719, 0.717], [0.287, 0.289], [0.0265, 0.0269], [21.7, 22.1], [0.246, 0.248]),
    ("voyager_journey", [0.020, 0.020], [0.974, 0.974], [0.045, 0.047], [0.0042, 0.0042], [1.6, 1.6], [0.004, 0.006]),
    ("wave_interference", [0.020, 0.020], [0.755, 0.762], [0.249, 0.242], [0.0259, 0.0251], [6.1, 6.0], [0.208, 0.201]),
];

fn measure(entry: &Entry) -> [ArtMetrics; 2] {
    let size = Size::new(SIZE.0, SIZE.1);
    let area = Rect::new(0, 0, SIZE.0, SIZE.1);
    let mut stage = Stage::new((entry.build)(), SEED, size, GlyphMode::Unicode);
    let mut steps = 0u64;
    TIMES.map(|t| {
        let target = (t / SIM_DT).round() as u64;
        while steps < target {
            stage.step(SIM_DT);
            steps += 1;
        }
        let mut buf = Buffer::empty(area);
        paint(stage.render(), Theme::Astro, area, &mut buf);
        art_metrics(&buf)
    })
}

type Measured = (&'static str, [ArtMetrics; 2]);

/// Every scene's metrics, measured once for all tests, on parallel threads.
fn all() -> &'static [Measured] {
    static ALL: OnceLock<Vec<Measured>> = OnceLock::new();
    ALL.get_or_init(|| {
        let entries: Vec<&'static Entry> = registry().iter().collect();
        let threads = std::thread::available_parallelism().map_or(4, usize::from);
        let chunk = entries.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let handles: Vec<_> = entries
                .chunks(chunk)
                .map(|part| {
                    s.spawn(move || {
                        part.iter()
                            .map(|e| (e.meta.name, measure(e)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("scene thread"))
                .collect()
        })
    })
}

fn limits(name: &str) -> Option<&'static Limits> {
    LIMITS.iter().find(|l| l.0 == name)
}

/// One ratchet: `value` against its `bound` for every scene and time. A ceiling fails
/// above the bound, a floor below it; either way `loose` headroom is reported.
fn check(
    id: &str,
    what: &str,
    ceiling: bool,
    loose: f64,
    value: fn(&ArtMetrics) -> f64,
    bound: fn(&Limits) -> [f64; 2],
) {
    let mut failures = Vec::new();
    for (name, m) in all() {
        let Some(l) = limits(name) else {
            continue;
        };
        let bound = bound(l);
        for (k, t) in TIMES.iter().enumerate() {
            let (v, b) = (value(&m[k]), bound[k]);
            let (bad, room) = if ceiling {
                (v > b, b - v)
            } else {
                (v < b, v - b)
            };
            if bad {
                let rel = if ceiling { ">" } else { "<" };
                failures.push(format!("{name} t={t}: {what} {v:.3} {rel} {b:.3}"));
            } else if room > loose {
                println!("{id} {name} t={t}: {what} {v:.3}, limit {b:.3} can be tightened");
            }
        }
    }
    assert!(failures.is_empty(), "{id}:\n{}", failures.join("\n"));
}

#[test]
fn gen_r17_r18_r21_limits_table_lists_every_scene_once() {
    let names: Vec<&str> = registry().iter().map(|e| e.meta.name).collect();
    for name in &names {
        assert!(
            limits(name).is_some(),
            "{name}: add a LIMITS entry (GEN-R17/R18/R21)"
        );
    }
    for l in LIMITS {
        assert!(names.contains(&l.0), "{}: stale LIMITS entry", l.0);
    }
    assert!(
        LIMITS.windows(2).all(|w| w[0].0 < w[1].0),
        "LIMITS must be alphabetical without duplicates"
    );
}

#[test]
fn gen_r17_blockiness_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R17",
        "blockiness",
        true,
        0.05,
        |m| m.blockiness,
        |l| l.1,
    );
}

#[test]
fn gen_r18_darkness_stays_above_each_scenes_floor() {
    check("GEN-R18", "darkness", false, 0.05, |m| m.darkness, |l| l.2);
}

#[test]
fn gen_r21_lit_coverage_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R21",
        "lit coverage",
        true,
        0.05,
        |m| m.clean.coverage,
        |l| l.3,
    );
}

#[test]
fn gen_r21_chroma_mass_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R21",
        "chroma mass",
        true,
        0.01,
        |m| m.clean.chroma_mass,
        |l| l.4,
    );
}

#[test]
fn gen_r21_edge_weight_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R21",
        "edge weight",
        true,
        3.0,
        |m| m.clean.edge_weight,
        |l| l.5,
    );
}

#[test]
fn gen_r21_no_scene_is_neon() {
    let mut failures = Vec::new();
    for (name, m) in all() {
        for (k, t) in TIMES.iter().enumerate() {
            let p90 = m[k].clean.chroma_p90;
            if p90 > NEON_CAP {
                failures.push(format!("{name} t={t}: chroma p90 {p90:.3} > {NEON_CAP}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "GEN-R21 neon cap:\n{}",
        failures.join("\n")
    );
}

#[test]
fn gen_r21_lit_coverage_stays_above_each_scenes_floor() {
    check(
        "GEN-R21",
        "lit coverage",
        false,
        0.05,
        |m| m.clean.coverage,
        |l| l.6,
    );
}

#[test]
fn gen_r21_subject_is_legible() {
    let (mut failures, mut stale) = (Vec::new(), Vec::new());
    for (name, m) in all() {
        let worst = m
            .iter()
            .map(|m| m.clean.lit_luma_p99)
            .fold(f64::INFINITY, f64::min);
        let faint = worst < LEGIBLE_LUMA;
        match (faint, KNOWN_FAINT.contains(name)) {
            (true, false) => {
                failures.push(format!("{name}: lit p99 luma {worst:.2} < {LEGIBLE_LUMA}"));
            }
            (false, true) => {
                stale.push(format!(
                    "{name}: now {worst:.2}; delete it from KNOWN_FAINT"
                ));
            }
            _ => {}
        }
    }
    for name in KNOWN_FAINT {
        assert!(
            all().iter().any(|(n, _)| n == name),
            "{name}: KNOWN_FAINT names no scene"
        );
    }
    failures.extend(stale);
    assert!(
        failures.is_empty(),
        "GEN-R21 legible subject:\n{}",
        failures.join("\n")
    );
}
