//! Per-scene art bands (GEN-R24, ADR-0018), measured with the RND-R13 metrics
//! (`sciart_render::metrics`) on the painted `astro` frame at 200×50, seed 42, t = 5 s and
//! 20 s: the same numbers `pixi run rs-cost` prints.
//!
//! `LIMITS` holds each scene's reviewed look as a band: per scene and time, a darkness
//! *floor* (a black ground) and *ceilings* on lit coverage, chroma mass and edge weight (a
//! small subject with much black around it, no washes or blobs beyond the reviewed look).
//! M14's global rules (GEN-R17 blockiness, GEN-R21 neon cap and legible-subject floor)
//! are superseded: M11's flat, stepped fills and saturated colours are the target look.
//! Seeded with slack (darkness −0.02, coverage +0.02, chroma mass +0.003, edge weight
//! +10 % + 0.2); `pixi run rs-cost -- <scene> --emit-limits` prints fresh rows, and a
//! scene's row is re-seeded only after its look is reviewed. Between reviews the band is a
//! ratchet: a ceiling may only be lowered and a floor only raised (the tests print loose
//! entries). `cargo test -p sciart --test art_metrics -- --nocapture` prints the headroom.

// Measured table values that happen to look like 1/π are not the constant.
#![allow(clippy::approx_constant)]

use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_render::metrics::{ArtMetrics, art_metrics};
use sciart_render::{Theme, paint};

const SEED: u64 = 42;
const SIZE: (u16, u16) = (200, 50);
const TIMES: [f64; 2] = [5.0, 20.0];

/// Per scene, each at `TIMES`: darkness floor, then ceilings on lit coverage, chroma mass
/// and edge weight (GEN-R24). Alphabetical.
type Limits = (&'static str, [f64; 2], [f64; 2], [f64; 2], [f64; 2]);

#[rustfmt::skip]
const LIMITS: &[Limits] = &[
    ("action_potential", [0.980, 0.980], [0.057, 0.056], [0.0043, 0.0043], [1.5, 1.5]),
    ("aurora", [0.753, 0.609], [0.251, 0.398], [0.0186, 0.0302], [5.8, 7.2]),
    ("belousov_zhabotinsky", [0.980, 0.980], [0.059, 0.079], [0.0051, 0.0061], [1.4, 1.4]),
    ("binary_inspiral", [0.980, 0.980], [0.042, 0.040], [0.0040, 0.0039], [1.4, 1.4]),
    ("black_hole", [0.980, 0.980], [0.045, 0.046], [0.0052, 0.0053], [1.3, 1.3]),
    ("boids_flocking", [0.980, 0.980], [0.054, 0.054], [0.0036, 0.0036], [1.4, 1.5]),
    ("brownian_motion", [0.980, 0.980], [0.025, 0.026], [0.0033, 0.0034], [1.5, 1.5]),
    ("catalytic_surface", [0.980, 0.980], [0.069, 0.098], [0.0056, 0.0072], [1.7, 1.5]),
    ("chemical_garden", [0.980, 0.980], [0.038, 0.048], [0.0039, 0.0044], [2.2, 2.0]),
    ("chladni_plate", [0.980, 0.980], [0.062, 0.057], [0.0048, 0.0045], [1.4, 1.4]),
    ("cloud_chamber", [0.980, 0.980], [0.030, 0.030], [0.0036, 0.0036], [1.4, 1.5]),
    ("comet_breakup", [0.980, 0.980], [0.033, 0.024], [0.0034, 0.0031], [1.5, 1.6]),
    ("comet_tails", [0.975, 0.975], [0.041, 0.046], [0.0040, 0.0042], [2.0, 1.9]),
    ("constellation_map", [0.980, 0.980], [0.027, 0.031], [0.0032, 0.0033], [1.3, 1.3]),
    ("cosmic_microwave_background", [0.958, 0.969], [0.211, 0.202], [0.0228, 0.0208], [1.5, 1.4]),
    ("cosmic_web", [0.980, 0.980], [0.127, 0.136], [0.0084, 0.0092], [1.4, 1.6]),
    ("dicty_aggregation", [0.980, 0.980], [0.108, 0.061], [0.0068, 0.0052], [1.5, 1.7]),
    ("dna_helix", [0.980, 0.980], [0.046, 0.071], [0.0040, 0.0058], [1.6, 1.6]),
    ("doppler_effect", [0.980, 0.980], [0.088, 0.056], [0.0108, 0.0072], [1.3, 1.3]),
    ("double_pendulum", [0.980, 0.980], [0.031, 0.032], [0.0038, 0.0041], [1.3, 1.3]),
    ("electron_orbitals", [0.980, 0.980], [0.048, 0.061], [0.0042, 0.0048], [1.8, 1.8]),
    ("exoplanet_transit", [0.950, 0.948], [0.114, 0.117], [0.0142, 0.0147], [1.8, 1.9]),
    ("firefly_sync", [0.980, 0.980], [0.035, 0.021], [0.0034, 0.0031], [1.6, 1.5]),
    ("fractal_growth", [0.980, 0.980], [0.029, 0.040], [0.0036, 0.0039], [1.4, 1.4]),
    ("galaxy_collision", [0.980, 0.980], [0.038, 0.051], [0.0041, 0.0047], [1.4, 2.1]),
    ("globular_cluster", [0.980, 0.980], [0.030, 0.030], [0.0033, 0.0034], [1.4, 1.5]),
    ("gravitational_lensing", [0.980, 0.980], [0.036, 0.030], [0.0036, 0.0032], [1.8, 1.6]),
    ("gravity_assist", [0.980, 0.980], [0.027, 0.026], [0.0033, 0.0033], [1.4, 1.5]),
    ("gray_scott", [0.980, 0.980], [0.094, 0.236], [0.0077, 0.0159], [1.4, 1.4]),
    ("heart_fibrillation", [0.980, 0.980], [0.040, 0.046], [0.0037, 0.0039], [1.4, 1.4]),
    ("hp_protein_folding", [0.980, 0.980], [0.031, 0.043], [0.0035, 0.0040], [1.9, 1.9]),
    ("hurricane", [0.980, 0.980], [0.037, 0.038], [0.0037, 0.0037], [1.4, 1.5]),
    ("io_volcanoes", [0.980, 0.980], [0.065, 0.067], [0.0050, 0.0051], [1.6, 1.7]),
    ("ising_model", [0.980, 0.980], [0.074, 0.042], [0.0049, 0.0039], [1.4, 1.4]),
    ("jupiter_storms", [0.980, 0.980], [0.065, 0.063], [0.0052, 0.0051], [1.4, 1.4]),
    ("jwst_unfolding", [0.980, 0.980], [0.052, 0.078], [0.0043, 0.0057], [4.0, 3.5]),
    ("kelvin_helmholtz", [0.854, 0.906], [0.148, 0.095], [0.0229, 0.0162], [2.9, 3.0]),
    ("lenia", [0.980, 0.980], [0.038, 0.039], [0.0038, 0.0038], [1.4, 1.4]),
    ("lichtenberg_discharge", [0.980, 0.980], [0.069, 0.037], [0.0049, 0.0037], [1.5, 1.5]),
    ("liesegang_rings", [0.887, 0.745], [0.127, 0.273], [0.0152, 0.0369], [7.0, 9.5]),
    ("light_echo", [0.980, 0.980], [0.065, 0.051], [0.0054, 0.0046], [1.4, 1.4]),
    ("magnetosphere", [0.978, 0.978], [0.057, 0.057], [0.0051, 0.0050], [1.4, 1.4]),
    ("mars_rover", [0.000, 0.000], [1.000, 1.000], [0.0933, 0.0935], [75.4, 77.2]),
    ("meteor_shower", [0.980, 0.980], [0.052, 0.050], [0.0035, 0.0034], [1.4, 1.4]),
    ("mitosis", [0.980, 0.980], [0.032, 0.043], [0.0035, 0.0037], [1.4, 1.5]),
    ("molecular_dynamics", [0.980, 0.980], [0.055, 0.043], [0.0044, 0.0041], [1.3, 1.3]),
    ("molecular_motors", [0.980, 0.980], [0.043, 0.037], [0.0037, 0.0035], [1.4, 1.4]),
    ("molecule_viewer", [0.969, 0.972], [0.041, 0.037], [0.0032, 0.0033], [2.1, 1.8]),
    ("moon_landing", [0.977, 0.976], [0.045, 0.051], [0.0036, 0.0038], [1.4, 1.4]),
    ("moon_phases", [0.980, 0.951], [0.043, 0.108], [0.0034, 0.0046], [1.3, 1.8]),
    ("motility_clustering", [0.980, 0.980], [0.086, 0.229], [0.0067, 0.0151], [1.4, 1.4]),
    ("noctilucent_clouds", [0.917, 0.922], [0.363, 0.373], [0.0229, 0.0235], [3.7, 4.8]),
    ("orbital_rendezvous", [0.880, 0.878], [0.123, 0.126], [0.0112, 0.0114], [6.0, 6.0]),
    ("particle_accelerator", [0.980, 0.980], [0.027, 0.039], [0.0033, 0.0044], [1.3, 1.3]),
    ("physarum", [0.980, 0.980], [0.056, 0.056], [0.0048, 0.0048], [1.6, 1.5]),
    ("planetary_nebula", [0.980, 0.980], [0.035, 0.045], [0.0036, 0.0040], [1.5, 1.4]),
    ("protocell_assembly", [0.980, 0.980], [0.046, 0.043], [0.0046, 0.0044], [1.4, 1.5]),
    ("pulsar", [0.980, 0.980], [0.062, 0.062], [0.0055, 0.0055], [1.3, 1.3]),
    ("quantum_double_slit", [0.980, 0.980], [0.052, 0.059], [0.0042, 0.0042], [1.4, 1.4]),
    ("radical_chain", [0.980, 0.980], [0.050, 0.036], [0.0042, 0.0035], [1.4, 1.4]),
    ("radioactive_decay", [0.980, 0.980], [0.125, 0.117], [0.0122, 0.0079], [1.3, 1.3]),
    ("rainbow", [0.980, 0.980], [0.049, 0.239], [0.0041, 0.0107], [1.8, 2.2]),
    ("rayleigh_benard", [0.980, 0.980], [0.044, 0.065], [0.0042, 0.0052], [1.4, 1.4]),
    ("relativistic_jet", [0.980, 0.980], [0.037, 0.037], [0.0037, 0.0038], [1.5, 1.5]),
    ("ribosome_translation", [0.980, 0.980], [0.027, 0.030], [0.0033, 0.0034], [1.5, 1.5]),
    ("rocket_launch", [0.979, 0.980], [0.032, 0.028], [0.0035, 0.0032], [1.4, 1.3]),
    ("satellite_orbit", [0.979, 0.979], [0.051, 0.051], [0.0054, 0.0053], [1.4, 1.4]),
    ("saturn_rings", [0.980, 0.980], [0.068, 0.069], [0.0039, 0.0039], [1.5, 1.5]),
    ("snowflake_growth", [0.980, 0.980], [0.044, 0.057], [0.0036, 0.0043], [1.4, 1.4]),
    ("solar_eruption", [0.980, 0.980], [0.055, 0.047], [0.0044, 0.0043], [1.4, 1.5]),
    ("solar_surface", [0.627, 0.629], [0.380, 0.378], [0.0619, 0.0617], [55.1, 60.8]),
    ("solar_system", [0.980, 0.980], [0.036, 0.036], [0.0035, 0.0034], [1.4, 1.4]),
    ("spacetime_grid", [0.980, 0.980], [0.142, 0.145], [0.0061, 0.0063], [1.4, 1.4]),
    ("spinodal_decomposition", [0.656, 0.787], [0.345, 0.214], [0.0582, 0.0359], [3.6, 3.6]),
    ("spiral_galaxy", [0.980, 0.980], [0.039, 0.039], [0.0039, 0.0040], [1.7, 1.7]),
    ("star_trails", [0.980, 0.980], [0.054, 0.091], [0.0045, 0.0058], [1.5, 1.6]),
    ("starfield", [0.980, 0.980], [0.036, 0.037], [0.0033, 0.0032], [1.3, 1.3]),
    ("stellar_nursery", [0.867, 0.865], [0.257, 0.267], [0.0147, 0.0154], [2.2, 2.1]),
    ("strange_attractors", [0.980, 0.980], [0.085, 0.062], [0.0065, 0.0052], [1.6, 1.8]),
    ("supernova_remnant", [0.980, 0.980], [0.034, 0.042], [0.0037, 0.0040], [1.5, 1.5]),
    ("three_body", [0.980, 0.980], [0.053, 0.053], [0.0044, 0.0044], [1.4, 1.5]),
    ("total_solar_eclipse", [0.980, 0.980], [0.028, 0.049], [0.0034, 0.0031], [1.3, 1.5]),
    ("vortex_street", [0.980, 0.980], [0.056, 0.057], [0.0049, 0.0049], [1.4, 1.4]),
    ("voyager_journey", [0.980, 0.980], [0.037, 0.039], [0.0033, 0.0034], [1.3, 1.3]),
    ("wave_interference", [0.980, 0.980], [0.076, 0.073], [0.0060, 0.0058], [1.5, 1.5]),
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
fn gen_r24_limits_table_lists_every_scene_once() {
    let names: Vec<&str> = registry().iter().map(|e| e.meta.name).collect();
    for name in &names {
        assert!(
            limits(name).is_some(),
            "{name}: add a LIMITS entry (GEN-R24)"
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
fn gen_r24_darkness_stays_above_each_scenes_floor() {
    check("GEN-R24", "darkness", false, 0.05, |m| m.darkness, |l| l.1);
}

#[test]
fn gen_r24_lit_coverage_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R24",
        "lit coverage",
        true,
        0.05,
        |m| m.clean.coverage,
        |l| l.2,
    );
}

#[test]
fn gen_r24_chroma_mass_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R24",
        "chroma mass",
        true,
        0.01,
        |m| m.clean.chroma_mass,
        |l| l.3,
    );
}

#[test]
fn gen_r24_edge_weight_stays_under_each_scenes_ceiling() {
    check(
        "GEN-R24",
        "edge weight",
        true,
        3.0,
        |m| m.clean.edge_weight,
        |l| l.4,
    );
}
