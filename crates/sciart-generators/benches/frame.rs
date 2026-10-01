//! GEN-R8: one simulated step + one render per generator must stay < 5 ms at 200x50.
//! Run with `pixi run rs-bench`.
//!
//! Three groups:
//! - `frame`: the mean frame of every scene, warmed past its opening.
//! - `worst`: the slowest known frames, act hand-overs and peaks found with
//!   `examples/worst_frame` (the mean hides them); each sample times that one frame.
//! - `resize`: the scenes whose resize is a visible one-off hitch.
#![allow(clippy::disallowed_methods)] // wall-clock timing of single frames (GEN-R8)

use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use sciart_core::prelude::*;
use sciart_generators::{find, registry};

const SIZES: [(u16, u16); 2] = [(200, 50), (400, 100)];

/// Worst frames: scene and the simulated second of a slow frame (M12 probe, seed 42).
const WORST: [(&str, f64); 8] = [
    ("aurora", 22.5),            // substorm breakup
    ("chemical_garden", 80.0),   // vent act: the chimney wall is drawn in full
    ("constellation_map", 96.5), // Cassiopeia fading in: the dust is redrawn per frame
    ("doppler_effect", 26.0),    // sound -> light: light prefill over fading sound
    ("doppler_effect", 48.0),    // light -> sound: sound prefill over fading light
    ("electron_orbitals", 33.2), // transition beat
    ("liesegang_rings", 75.5),   // band precipitation burst
    ("radioactive_decay", 6.7),  // N_B peak: most marks on the lattice
];
/// Frames timed around each worst time; the slowest counts. A hand-over lands in one of
/// them whichever way the float clock rounds at the boundary.
const AROUND: usize = 3;

/// Resize hitches (one-off, not per frame): geodesic trace, lazily built charts, star field.
const RESIZE: [&str; 3] = ["black_hole", "constellation_map", "starfield"];

fn frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame");
    for entry in registry() {
        for (w, h) in SIZES {
            let mut stage = Stage::new((entry.build)(), 42, Size::new(w, h), GlyphMode::Unicode);
            // Warm the scene past its initial state.
            for _ in 0..120 {
                stage.step(SIM_DT);
            }
            let id = BenchmarkId::new(entry.meta.name, format!("{w}x{h}"));
            group.bench_function(id, |b| {
                b.iter(|| {
                    stage.step(SIM_DT);
                    std::hint::black_box(stage.render());
                });
            });
        }
    }
    group.finish();
}

/// A stage one fixed step short of scene time `t`: coarse steps (the GEN-R4 maximum)
/// most of the way, then the last half second at `SIM_DT`, as the app steps.
fn stage_before(name: &str, size: Size, t: f64) -> Stage {
    let entry = find(name).unwrap_or_else(|| panic!("unknown scene {name}"));
    let mut stage = Stage::new((entry.build)(), 42, size, GlyphMode::Unicode);
    let mut now = 0.0;
    while now + 0.25 < t - 0.5 {
        stage.step(0.25);
        now += 0.25;
    }
    while now + SIM_DT < t - 1e-9 {
        stage.step(SIM_DT);
        now += SIM_DT;
    }
    stage
}

fn worst(c: &mut Criterion) {
    let mut group = c.benchmark_group("worst");
    // Every sample replays the approach to the frame (untimed), so keep the count low.
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.warm_up_time(Duration::from_millis(1));
    for (name, t) in WORST {
        for (w, h) in SIZES {
            let size = Size::new(w, h);
            let id = BenchmarkId::new(format!("{name}@{t}s"), format!("{w}x{h}"));
            group.bench_function(id, |b| {
                // Time the frames around `t`; report the slowest as the per-iteration cost.
                b.iter_custom(|iters| {
                    let mut stage = stage_before(name, size, t - 1.5 * SIM_DT);
                    let mut slowest = Duration::ZERO;
                    for _ in 0..AROUND {
                        let t0 = Instant::now();
                        stage.step(SIM_DT);
                        std::hint::black_box(stage.render());
                        slowest = slowest.max(t0.elapsed());
                    }
                    slowest * u32::try_from(iters).unwrap_or(u32::MAX)
                });
            });
        }
    }
    group.finish();
}

fn resize(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize");
    group.sample_size(10);
    for name in RESIZE {
        for (w, h) in SIZES {
            let entry = find(name).unwrap_or_else(|| panic!("unknown scene {name}"));
            let (size, other) = (Size::new(w, h), Size::new(w - 1, h));
            let mut stage = Stage::new((entry.build)(), 42, size, GlyphMode::Unicode);
            stage.step(SIM_DT);
            let id = BenchmarkId::new(name, format!("{w}x{h}"));
            group.bench_function(id, |b| {
                // Time only the resize to `size`, from a neighbouring size.
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        stage.resize(other);
                        let t0 = Instant::now();
                        stage.resize(size);
                        total += t0.elapsed();
                    }
                    total
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, frame, worst, resize);
criterion_main!(benches);
