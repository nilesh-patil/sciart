//! Per-scene art ratchets, measured with the RND-R13 metrics (`sciart_render::metrics`) on
//! the painted `astro` frame at 200×50, seed 42, t = 5 s and 20 s: the same numbers
//! `pixi run rs-cost` prints.
//!
//! `LIMITS` holds, per scene and time, a blockiness *ceiling* (GEN-R17) and *floors* for
//! darkness (GEN-R18), colourfulness and RMS contrast (GEN-R21). Seeded from the M14
//! baseline with slack (blockiness +0.02, darkness −0.02, colourfulness and contrast
//! −5 %). Ratchet: a ceiling may only be lowered and a floor only raised; when a scene
//! improves, tighten its entry (the tests print loose entries). Lowering a GEN-R21 floor
//! needs a reviewed note in the scene's SPEC-002 section.
//! `cargo test -p sciart --test art_metrics -- --nocapture` prints the headroom.

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

/// Per scene, each at `TIMES`: blockiness ceiling, darkness floor, colourfulness floor,
/// contrast floor. Alphabetical.
type Limits = (&'static str, [f64; 2], [f64; 2], [f64; 2], [f64; 2]);

#[rustfmt::skip]
const LIMITS: &[Limits] = &[
    ("action_potential", [0.041, 0.108], [0.912, 0.912], [47.1, 48.5], [0.139, 0.138]),
    ("aurora", [0.033, 0.030], [0.844, 0.707], [80.6, 86.5], [0.128, 0.193]),
    ("belousov_zhabotinsky", [0.020, 0.020], [0.614, 0.480], [64.7, 63.5], [0.204, 0.213]),
    ("black_hole", [0.020, 0.020], [0.727, 0.735], [51.2, 53.3], [0.223, 0.220]),
    ("boids_flocking", [0.083, 0.080], [0.012, 0.012], [112.1, 114.3], [0.245, 0.245]),
    ("brownian_motion", [0.020, 0.020], [0.975, 0.975], [33.8, 36.4], [0.112, 0.116]),
    ("catalytic_surface", [0.147, 0.020], [0.000, 0.000], [72.5, 37.1], [0.043, 0.051]),
    ("chemical_garden", [0.211, 0.179], [0.000, 0.000], [47.4, 51.1], [0.135, 0.151]),
    ("comet_breakup", [0.020, 0.020], [0.853, 0.965], [50.7, 19.0], [0.125, 0.047]),
    ("comet_tails", [0.020, 0.020], [0.884, 0.761], [64.7, 40.2], [0.104, 0.124]),
    ("constellation_map", [0.034, 0.020], [0.607, 0.979], [23.0, 49.9], [0.103, 0.104]),
    ("cosmic_microwave_background", [0.027, 0.020], [0.299, 0.299], [93.6, 93.0], [0.185, 0.171]),
    ("cosmic_web", [0.020, 0.020], [0.654, 0.697], [55.7, 58.2], [0.112, 0.157]),
    ("dicty_aggregation", [0.020, 0.020], [0.563, 0.821], [82.7, 100.6], [0.129, 0.109]),
    ("dna_helix", [0.020, 0.020], [0.960, 0.953], [81.6, 106.2], [0.176, 0.212]),
    ("doppler_effect", [0.020, 0.020], [0.980, 0.980], [55.6, 50.3], [0.279, 0.240]),
    ("double_pendulum", [0.020, 0.020], [0.947, 0.944], [95.8, 97.5], [0.088, 0.091]),
    ("electron_orbitals", [0.020, 0.020], [0.874, 0.864], [80.2, 88.7], [0.158, 0.159]),
    ("exoplanet_transit", [0.064, 0.064], [0.843, 0.843], [52.1, 52.8], [0.236, 0.236]),
    ("firefly_sync", [0.119, 0.127], [0.110, 0.146], [48.0, 31.1], [0.086, 0.060]),
    ("fractal_growth", [0.020, 0.020], [0.980, 0.980], [78.5, 10.1], [0.136, 0.264]),
    ("galaxy_collision", [0.020, 0.020], [0.832, 0.675], [93.6, 70.9], [0.146, 0.144]),
    ("globular_cluster", [0.020, 0.020], [0.932, 0.931], [18.0, 18.1], [0.157, 0.158]),
    ("gravitational_lensing", [0.020, 0.020], [0.857, 0.881], [86.2, 79.1], [0.096, 0.072]),
    ("gray_scott", [0.020, 0.020], [0.679, 0.000], [61.3, 58.7], [0.167, 0.208]),
    ("heart_fibrillation", [0.212, 0.020], [0.799, 0.683], [118.9, 113.3], [0.177, 0.185]),
    ("hp_protein_folding", [0.020, 0.020], [0.865, 0.779], [69.8, 74.3], [0.078, 0.126]),
    ("hurricane", [0.135, 0.127], [0.000, 0.000], [47.5, 47.5], [0.230, 0.231]),
    ("io_volcanoes", [0.029, 0.029], [0.370, 0.370], [72.6, 73.7], [0.321, 0.320]),
    ("ising_model", [0.171, 0.088], [0.000, 0.000], [72.9, 67.9], [0.186, 0.131]),
    ("jupiter_storms", [0.020, 0.020], [0.095, 0.095], [40.1, 38.1], [0.246, 0.222]),
    ("jwst_unfolding", [0.046, 0.056], [0.913, 0.785], [60.7, 44.1], [0.132, 0.250]),
    ("kelvin_helmholtz", [0.215, 0.234], [0.000, 0.000], [82.6, 79.6], [0.159, 0.182]),
    ("lenia", [0.020, 0.020], [0.864, 0.862], [99.6, 99.4], [0.152, 0.152]),
    ("lichtenberg_discharge", [0.020, 0.020], [0.747, 0.915], [44.4, 45.6], [0.132, 0.086]),
    ("liesegang_rings", [0.106, 0.075], [0.770, 0.649], [56.1, 55.6], [0.125, 0.159]),
    ("light_echo", [0.033, 0.048], [0.766, 0.854], [116.2, 118.3], [0.120, 0.133]),
    ("mars_rover", [0.300, 0.316], [0.000, 0.000], [55.6, 55.6], [0.133, 0.130]),
    ("meteor_shower", [0.020, 0.020], [0.782, 0.782], [22.6, 22.8], [0.176, 0.177]),
    ("molecular_dynamics", [0.020, 0.020], [0.952, 0.918], [51.3, 55.7], [0.155, 0.204]),
    ("molecular_motors", [0.020, 0.020], [0.582, 0.572], [48.7, 43.6], [0.140, 0.163]),
    ("molecule_viewer", [0.081, 0.020], [0.666, 0.723], [46.4, 43.9], [0.135, 0.113]),
    ("moon_landing", [0.020, 0.020], [0.773, 0.734], [26.9, 24.5], [0.143, 0.150]),
    ("moon_phases", [0.423, 0.038], [0.687, 0.750], [10.4, 10.7], [0.161, 0.289]),
    ("motility_clustering", [0.020, 0.020], [0.980, 0.643], [59.5, 68.1], [0.106, 0.278]),
    ("noctilucent_clouds", [0.023, 0.020], [0.099, 0.099], [90.8, 83.0], [0.251, 0.281]),
    ("orbital_rendezvous", [0.020, 0.020], [0.817, 0.817], [79.3, 83.8], [0.169, 0.185]),
    ("particle_accelerator", [0.020, 0.020], [0.980, 0.980], [57.1, 49.8], [0.116, 0.074]),
    ("physarum", [0.020, 0.046], [0.835, 0.814], [71.1, 74.6], [0.208, 0.218]),
    ("planetary_nebula", [0.020, 0.020], [0.851, 0.835], [38.0, 81.5], [0.043, 0.139]),
    ("pulsar", [0.020, 0.020], [0.900, 0.862], [46.5, 46.3], [0.116, 0.118]),
    ("quantum_double_slit", [0.020, 0.020], [0.353, 0.352], [56.2, 54.4], [0.138, 0.159]),
    ("radical_chain", [0.020, 0.020], [0.086, 0.086], [58.5, 32.9], [0.257, 0.149]),
    ("radioactive_decay", [0.020, 0.020], [0.714, 0.890], [110.0, 97.4], [0.217, 0.184]),
    ("rainbow", [0.173, 0.108], [0.121, 0.120], [67.1, 67.7], [0.128, 0.169]),
    ("relativistic_jet", [0.020, 0.020], [0.846, 0.845], [63.9, 62.5], [0.085, 0.084]),
    ("rocket_launch", [0.036, 0.022], [0.000, 0.000], [74.5, 69.6], [0.167, 0.162]),
    ("satellite_orbit", [0.035, 0.033], [0.910, 0.905], [88.5, 85.7], [0.151, 0.146]),
    ("saturn_rings", [0.060, 0.058], [0.535, 0.534], [29.4, 29.4], [0.318, 0.318]),
    ("seashell_pigmentation", [0.049, 0.063], [0.848, 0.855], [46.7, 40.0], [0.167, 0.173]),
    ("solar_eruption", [0.020, 0.020], [0.531, 0.534], [38.9, 47.9], [0.094, 0.150]),
    ("solar_surface", [0.020, 0.020], [0.579, 0.582], [99.1, 99.1], [0.263, 0.261]),
    ("solar_system", [0.066, 0.048], [0.939, 0.939], [93.4, 93.2], [0.129, 0.124]),
    ("spacetime_grid", [0.020, 0.020], [0.972, 0.971], [103.4, 100.0], [0.155, 0.148]),
    ("spinodal_decomposition", [0.020, 0.020], [0.039, 0.018], [67.5, 65.1], [0.220, 0.192]),
    ("spiral_galaxy", [0.020, 0.020], [0.714, 0.714], [30.3, 28.3], [0.163, 0.161]),
    ("star_trails", [0.020, 0.020], [0.204, 0.204], [45.4, 55.0], [0.165, 0.222]),
    ("starfield", [0.020, 0.020], [0.759, 0.759], [13.4, 13.4], [0.205, 0.204]),
    ("stellar_nursery", [0.020, 0.020], [0.364, 0.400], [99.0, 99.2], [0.174, 0.154]),
    ("strange_attractors", [0.020, 0.020], [0.700, 0.823], [69.7, 64.9], [0.152, 0.137]),
    ("supernova_remnant", [0.020, 0.020], [0.933, 0.858], [73.6, 97.2], [0.056, 0.108]),
    ("three_body", [0.020, 0.020], [0.925, 0.974], [101.4, 98.0], [0.114, 0.044]),
    ("total_solar_eclipse", [0.341, 0.020], [0.079, 0.076], [49.0, 51.3], [0.099, 0.075]),
    ("vortex_street", [0.046, 0.046], [0.713, 0.711], [89.4, 89.9], [0.175, 0.177]),
    ("voyager_journey", [0.020, 0.020], [0.974, 0.975], [83.7, 85.7], [0.126, 0.143]),
    ("wave_interference", [0.020, 0.020], [0.170, 0.189], [65.5, 64.9], [0.174, 0.172]),
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
fn gen_r21_colourfulness_stays_above_each_scenes_floor() {
    check(
        "GEN-R21",
        "colourfulness",
        false,
        5.0,
        |m| m.colourfulness,
        |l| l.3,
    );
}

#[test]
fn gen_r21_contrast_stays_above_each_scenes_floor() {
    check("GEN-R21", "contrast", false, 0.03, |m| m.contrast, |l| l.4);
}
