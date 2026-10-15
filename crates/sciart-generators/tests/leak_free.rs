//! Leak guards for every registered generator (TEST-R6, TEST-R7), measured as live heap
//! bytes with the counting global allocator (ADR-0009). Own test binary: a binary has
//! exactly one global allocator.
//!
//! Live bytes are per thread, and each scene runs start to finish on its own scoped
//! thread, so a reading sees only that scene's `Stage` (generator, context and both
//! canvases).
//!
//! Run `cargo test -p sciart-generators --test leak_free -- --nocapture` for the tables.

use std::fmt::Write as _;

use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_testalloc::{CountingAlloc, live_bytes};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const SEED: u64 = 42;
const HOME: Size = Size::new(80, 24);
/// One churn cycle from `HOME`: grow, collapse to the smallest legal size, come back.
const CYCLE: [Size; 3] = [Size::new(200, 50), Size::new(1, 1), HOME];
/// Cycles that settle every buffer's high-water capacity. Two, not one: `grid::regrid_*`
/// swap the field with its scratch, and a cycle has an odd number of regrids, so each of
/// the pair reaches its peak size only on alternate cycles (ising_model).
const WARMUP_CYCLES: usize = 2;
/// Cycles checked after the warm-up: enough that growth of one small element per cycle
/// crosses a `Vec` capacity doubling (8 → 16 bytes) and shows up, not only growth that
/// allocates afresh every cycle.
const CHECKED_CYCLES: usize = 8;
const CYCLES: usize = WARMUP_CYCLES + CHECKED_CYCLES;
/// Half a simulated second per size: enough for respawns and lazily grown buffers.
const CHURN_STEPS: usize = 30;

const BIG: Size = Size::new(500, 150);
/// One simulated second at `BIG` (TEST-R7).
const BIG_STEPS: usize = 60;
/// TEST-R7 (M22, ADR-0023): live bytes at `BIG` stay under this for every scene not in
/// `EXCEPTIONS`. Every scene pays ≈ 1.43 MB before its own state (the `Stage`'s two
/// 500×150 canvases at 8 B a cell, plus a braille `Dots`), and the scenes under it hold
/// ≤ 2.51 MiB (wave_interference), so 2.75 MiB leaves ≥ 9 % margin. May only be lowered;
/// raising it needs a spec change.
const LIVE_CEILING: i64 = 2_883_584; // 2.75 MiB
/// Scenes with their own, higher ceiling, each named with its reason in TEST-R7, so one
/// outlier does not loosen the guard for every other scene. Each is its M22 measurement
/// plus ≈ 8 %, rounded up to 1/8 MiB and capped at the old 4 MiB ceiling (M22 measurement
/// in the comment).
const EXCEPTIONS: &[(&str, i64)] = &[
    ("fractal_growth", 4 << 20), // 3.87 MiB: DLA lattice + pre-reserved site queues
    ("strange_attractors", 4 << 20), // 3.81 MiB: per-dot f32 afterglow density
    ("solar_surface", 4 << 20),  // 3.79 MiB: per-pixel disc and rim tables
    ("rayleigh_benard", 4 << 20), // 3.76 MiB: per-dot f32 stipple thresholds
    ("snowflake_growth", 4 << 20), // 3.74 MiB: hexagonal lattice fields (f64)
    ("aurora", 4 << 20),         // 3.68 MiB: per-pixel curtain layers and glow
    ("star_trails", 4 << 20),    // 3.59 MiB: per-dot exposure and rank
    ("cloud_chamber", 4_063_232), // 3.56 MiB: pre-reserved droplet slots
    ("heart_fibrillation", 4_063_232), // 3.52 MiB: excitable lattice (u, v, slope, age)
    ("quantum_double_slit", 3_801_088), // 3.32 MiB: complex wavefunction and ADI factors
    ("binary_inspiral", 3_801_088), // 3.32 MiB: precomputed field-dot table
    ("kelvin_helmholtz", 3_538_944), // 3.09 MiB: vorticity and dye lattices
    ("noctilucent_clouds", 3_407_872), // 2.93 MiB: wave lattice nodes with packet phases
    ("ising_model", 3_276_800),  // 2.89 MiB: spin lattice and display ages
    ("gray_scott", 3_145_728),   // 2.73 MiB: double-buffered u/v lattices
];

fn ceiling(name: &str) -> i64 {
    EXCEPTIONS
        .iter()
        .find(|&&(scene, _)| scene == name)
        .map_or(LIVE_CEILING, |&(_, ceiling)| ceiling)
}

/// Titles are on by default (CORE-R75), so the title and `caption()` are guarded too.
fn stage(entry: &Entry, size: Size) -> Stage {
    Stage::new((entry.build)(), SEED, size, GlyphMode::Unicode)
}

fn run(stage: &mut Stage, steps: usize) {
    for _ in 0..steps {
        stage.step(SIM_DT);
    }
    stage.render();
}

/// Live bytes after each churn cycle (TEST-R6), relative to the end of the warm-up.
fn churn(entry: &Entry) -> [i64; CYCLES] {
    let mut stage = stage(entry, HOME);
    let mut live = [0; CYCLES];
    for slot in &mut live {
        for size in CYCLE {
            stage.resize(size);
            run(&mut stage, CHURN_STEPS);
        }
        *slot = live_bytes();
    }
    live.map(|l| l - live[WARMUP_CYCLES - 1])
}

/// Live bytes the scene's `Stage` holds at `BIG` after one simulated second (TEST-R7).
fn footprint(entry: &Entry) -> i64 {
    let before = live_bytes();
    let mut stage = stage(entry, BIG);
    run(&mut stage, BIG_STEPS);
    let held = live_bytes() - before;
    drop(stage);
    held
}

/// `f` for every scene, one scoped thread each (live bytes are per thread).
fn per_scene<T: Send>(f: fn(&Entry) -> T) -> Vec<(&'static str, T)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = registry()
            .iter()
            .map(|e| (e.meta.name, s.spawn(move || f(e))))
            .collect();
        handles
            .into_iter()
            .map(|(name, h)| (name, h.join().unwrap_or_else(|_| panic!("{name} panicked"))))
            .collect()
    })
}

#[test]
fn test_r6_resize_churn_returns_to_the_same_live_bytes() {
    let rows = per_scene(churn);
    let mut table =
        format!("live bytes after each churn cycle, relative to cycle {WARMUP_CYCLES}:\n");
    for (name, live) in &rows {
        let _ = writeln!(table, "{name:<28} {live:?}");
    }
    println!("{table}");
    // Live bytes count allocations, i.e. capacity, not length: growth inside a `Vec`'s
    // spare capacity is invisible here until the buffer reallocates (about doubles).
    let leaks: Vec<_> = rows
        .iter()
        .filter(|(_, live)| live[WARMUP_CYCLES..].iter().any(|&l| l != 0))
        .map(|(name, live)| format!("{name}: live bytes keep changing after warm-up: {live:?}"))
        .collect();
    assert!(
        leaks.is_empty(),
        "TEST-R6:\n{}\n\n{table}",
        leaks.join("\n")
    );
}

#[test]
fn test_r7_live_bytes_at_500x150_stay_under_the_ceiling() {
    let mut rows = per_scene(footprint);
    rows.sort_by_key(|&(_, held)| std::cmp::Reverse(held));
    let mut table = format!(
        "live bytes at {}x{} after {BIG_STEPS} steps (ceiling {LIVE_CEILING}, exceptions \
         {EXCEPTIONS:?}):\n",
        BIG.width(),
        BIG.height()
    );
    for (name, held) in &rows {
        let _ = writeln!(
            table,
            "{name:<28} {held:>10} B {:>6.2} MiB",
            *held as f64 / f64::from(1 << 20)
        );
    }
    println!("{table}");
    let over: Vec<_> = rows
        .iter()
        .filter(|&&(name, held)| held >= ceiling(name))
        .map(|(name, held)| format!("{name}: {held} B, ceiling {}", ceiling(name)))
        .collect();
    assert!(
        over.is_empty(),
        "TEST-R7: over the ceiling:\n{}\n\n{table}",
        over.join("\n")
    );
}
