//! Per-scene art ratchets (SPEC-002 GEN-R17 fine texture, GEN-R18 dark ground), measured
//! with the RND-R13 metrics (`sciart_render::metrics`) on the painted `astro` frame at
//! 200×50, seed 42, t = 5 s and 20 s: the same numbers `pixi run rs-cost` prints.
//!
//! `LIMITS` holds, per scene, a blockiness *ceiling* and a darkness *floor* at each time,
//! seeded from the M14 baseline with 0.02 of slack. Ratchet: a ceiling may only be
//! lowered and a floor only raised; when a scene improves, tighten its entry (the tests
//! print the headroom). `cargo test -p sciart --test art_metrics -- --nocapture`.

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
/// Headroom above which the report suggests tightening an entry.
const LOOSE: f64 = 0.05;

/// `(scene, blockiness ceiling at TIMES, darkness floor at TIMES)`, alphabetical.
#[rustfmt::skip]
const LIMITS: &[(&str, [f64; 2], [f64; 2])] = &[
    ("action_potential", [0.041, 0.108], [0.922, 0.924]),
    ("aurora", [0.033, 0.030], [0.806, 0.661]),
    ("belousov_zhabotinsky", [0.020, 0.020], [0.744, 0.684]),
    ("black_hole", [0.020, 0.020], [0.729, 0.737]),
    ("boids_flocking", [0.083, 0.080], [0.012, 0.012]),
    ("brownian_motion", [0.020, 0.020], [0.975, 0.975]),
    ("catalytic_surface", [0.147, 0.020], [0.000, 0.000]),
    ("chemical_garden", [0.211, 0.179], [0.753, 0.713]),
    ("comet_breakup", [0.020, 0.020], [0.854, 0.963]),
    ("comet_tails", [0.020, 0.020], [0.869, 0.703]),
    ("constellation_map", [0.034, 0.020], [0.546, 0.979]),
    ("cosmic_microwave_background", [0.027, 0.020], [0.299, 0.299]),
    ("cosmic_web", [0.020, 0.020], [0.708, 0.721]),
    ("dicty_aggregation", [0.020, 0.020], [0.818, 0.856]),
    ("dna_helix", [0.020, 0.020], [0.960, 0.953]),
    ("doppler_effect", [0.020, 0.020], [0.980, 0.980]),
    ("double_pendulum", [0.020, 0.020], [0.963, 0.962]),
    ("electron_orbitals", [0.020, 0.020], [0.877, 0.867]),
    ("exoplanet_transit", [0.064, 0.064], [0.843, 0.843]),
    ("firefly_sync", [0.119, 0.127], [0.576, 0.609]),
    ("fractal_growth", [0.020, 0.020], [0.980, 0.980]),
    ("galaxy_collision", [0.020, 0.020], [0.860, 0.803]),
    ("globular_cluster", [0.020, 0.020], [0.920, 0.917]),
    ("gravitational_lensing", [0.020, 0.020], [0.868, 0.892]),
    ("gray_scott", [0.020, 0.020], [0.784, 0.336]),
    ("heart_fibrillation", [0.212, 0.020], [0.682, 0.680]),
    ("hp_protein_folding", [0.020, 0.020], [0.931, 0.875]),
    ("hurricane", [0.135, 0.127], [0.000, 0.000]),
    ("io_volcanoes", [0.029, 0.029], [0.402, 0.404]),
    ("ising_model", [0.171, 0.088], [0.000, 0.000]),
    ("jupiter_storms", [0.020, 0.020], [0.101, 0.101]),
    ("jwst_unfolding", [0.046, 0.056], [0.909, 0.781]),
    ("kelvin_helmholtz", [0.215, 0.234], [0.266, 0.270]),
    ("lenia", [0.020, 0.020], [0.864, 0.862]),
    ("lichtenberg_discharge", [0.020, 0.020], [0.761, 0.921]),
    ("liesegang_rings", [0.106, 0.075], [0.781, 0.673]),
    ("light_echo", [0.033, 0.048], [0.625, 0.693]),
    ("mars_rover", [0.300, 0.316], [0.000, 0.000]),
    ("meteor_shower", [0.020, 0.020], [0.597, 0.597]),
    ("molecular_dynamics", [0.020, 0.020], [0.952, 0.934]),
    ("molecular_motors", [0.020, 0.020], [0.580, 0.571]),
    ("molecule_viewer", [0.081, 0.020], [0.717, 0.755]),
    ("moon_landing", [0.020, 0.020], [0.773, 0.735]),
    ("moon_phases", [0.423, 0.038], [0.687, 0.750]),
    ("motility_clustering", [0.020, 0.020], [0.980, 0.643]),
    ("noctilucent_clouds", [0.023, 0.020], [0.149, 0.282]),
    ("orbital_rendezvous", [0.020, 0.020], [0.816, 0.815]),
    ("particle_accelerator", [0.020, 0.020], [0.980, 0.980]),
    ("physarum", [0.020, 0.046], [0.835, 0.814]),
    ("planetary_nebula", [0.020, 0.020], [0.957, 0.827]),
    ("pulsar", [0.020, 0.020], [0.907, 0.880]),
    ("quantum_double_slit", [0.020, 0.020], [0.331, 0.334]),
    ("radical_chain", [0.020, 0.020], [0.566, 0.940]),
    ("radioactive_decay", [0.020, 0.020], [0.714, 0.890]),
    ("rainbow", [0.173, 0.108], [0.120, 0.118]),
    ("relativistic_jet", [0.020, 0.020], [0.836, 0.837]),
    ("rocket_launch", [0.036, 0.022], [0.000, 0.000]),
    ("satellite_orbit", [0.035, 0.033], [0.906, 0.910]),
    ("saturn_rings", [0.060, 0.058], [0.509, 0.509]),
    ("seashell_pigmentation", [0.049, 0.063], [0.847, 0.854]),
    ("solar_eruption", [0.020, 0.020], [0.511, 0.511]),
    ("solar_surface", [0.020, 0.020], [0.586, 0.589]),
    ("solar_system", [0.066, 0.048], [0.939, 0.939]),
    ("spacetime_grid", [0.020, 0.020], [0.975, 0.975]),
    ("spinodal_decomposition", [0.020, 0.020], [0.023, 0.008]),
    ("spiral_galaxy", [0.020, 0.020], [0.678, 0.676]),
    ("star_trails", [0.020, 0.020], [0.733, 0.591]),
    ("starfield", [0.020, 0.020], [0.710, 0.710]),
    ("stellar_nursery", [0.020, 0.020], [0.445, 0.481]),
    ("strange_attractors", [0.020, 0.020], [0.700, 0.823]),
    ("supernova_remnant", [0.020, 0.020], [0.932, 0.860]),
    ("three_body", [0.020, 0.020], [0.947, 0.975]),
    ("total_solar_eclipse", [0.341, 0.020], [0.115, 0.703]),
    ("vortex_street", [0.046, 0.046], [0.713, 0.711]),
    ("voyager_journey", [0.020, 0.020], [0.975, 0.975]),
    ("wave_interference", [0.020, 0.020], [0.183, 0.205]),
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

fn limits(name: &str) -> Option<([f64; 2], [f64; 2])> {
    LIMITS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|&(_, ceil, floor)| (ceil, floor))
}

#[test]
fn gen_r17_r18_limits_table_lists_every_scene_once() {
    let names: Vec<&str> = registry().iter().map(|e| e.meta.name).collect();
    for name in &names {
        assert!(
            limits(name).is_some(),
            "{name}: add a LIMITS entry (GEN-R17/R18)"
        );
    }
    for (name, ..) in LIMITS {
        assert!(names.contains(name), "{name}: stale LIMITS entry");
    }
    assert!(
        LIMITS.windows(2).all(|w| w[0].0 < w[1].0),
        "LIMITS must be alphabetical without duplicates"
    );
}

#[test]
fn gen_r17_blockiness_stays_under_each_scenes_ceiling() {
    let mut failures = Vec::new();
    for (name, m) in all() {
        let Some((ceil, _)) = limits(name) else {
            continue;
        };
        for (k, t) in TIMES.iter().enumerate() {
            let b = m[k].blockiness;
            if b > ceil[k] {
                failures.push(format!(
                    "{name} t={t}: blockiness {b:.3} > ceiling {:.3}",
                    ceil[k]
                ));
            } else if ceil[k] - b > LOOSE {
                println!(
                    "{name} t={t}: blockiness {b:.3}; lower the ceiling {:.3}",
                    ceil[k]
                );
            }
        }
    }
    assert!(failures.is_empty(), "GEN-R17:\n{}", failures.join("\n"));
}

#[test]
fn gen_r18_darkness_stays_above_each_scenes_floor() {
    let mut failures = Vec::new();
    for (name, m) in all() {
        let Some((_, floor)) = limits(name) else {
            continue;
        };
        for (k, t) in TIMES.iter().enumerate() {
            let d = m[k].darkness;
            if d < floor[k] {
                failures.push(format!(
                    "{name} t={t}: darkness {d:.3} < floor {:.3}",
                    floor[k]
                ));
            } else if d - floor[k] > LOOSE {
                println!(
                    "{name} t={t}: darkness {d:.3}; raise the floor {:.3}",
                    floor[k]
                );
            }
        }
    }
    assert!(failures.is_empty(), "GEN-R18:\n{}", failures.join("\n"));
}
