//! Steady-state heap traffic of every registered generator, measured with a counting
//! global allocator (ADR-0009). Own test binary: a binary has exactly one global allocator.
//!
//! House rule behind the GEN-R8 frame budget: after warm-up, `Generator::render` never
//! allocates — a frame is drawn into the pre-sized, reused canvas (spec §"no per-frame
//! heap allocation in steady state"). `update` is *reported*, and ratcheted with an
//! explicit list: today no scene allocates in `update` either, and new ones should not.
//!
//! Protocol per scene and size (through `Stage`, the production call path):
//! build + resize, `WARMUP_STEPS` steps, one render; then `FRAMES` frames of
//! `STEPS_PER_FRAME` measured `step(SIM_DT)` + one measured `render()` (30 FPS cadence),
//! then a few renders in ASCII mode. Scenes run on parallel scoped threads; the
//! counters are per thread, so each scene sees only its own allocations.
//!
//! Run `cargo test -p sciart-generators --test alloc_free -- --nocapture` for the table.

use std::fmt::Write as _;

use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_testalloc::{Allocs, CountingAlloc, measure};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const SEED: u64 = 42;
const WARMUP_STEPS: usize = 30;
/// 10 simulated seconds at 30 FPS: long enough to reach later phases of most
/// narrative scenes and several respawn cycles of the particle scenes.
const FRAMES: usize = 300;
const STEPS_PER_FRAME: usize = 2;
const ASCII_FRAMES: usize = 3;

/// Scenes whose `render` is known to allocate, with the reason. Each entry is a bug to
/// fix in the scene (then delete the entry: a stale entry fails the test too).
// TODO(alloc): fix each scene listed here, then delete its entry.
const RENDER_ALLOCATES: &[(&str, &str)] = &[];

/// Scenes whose `update` allocates in steady state. Empty today (every scene's update is
/// allocation-free after warm-up), so this is a ratchet: a scene that starts allocating
/// in `update` fails unless listed here with a reason, and a listed scene that no
/// longer allocates fails too (delete its entry).
const UPDATE_ALLOCATES: &[&str] = &[];

#[derive(Debug, Default, Clone, Copy)]
struct Report {
    render: Allocs,
    update: Allocs,
}

fn add(a: Allocs, b: Allocs) -> Allocs {
    Allocs {
        allocs: a.allocs + b.allocs,
        reallocs: a.reallocs + b.reallocs,
        deallocs: a.deallocs + b.deallocs,
        bytes: a.bytes + b.bytes,
    }
}

fn run_scene(entry: &Entry, size: Size) -> Report {
    // Titles are on by default (CORE-R75): the title and `caption()` are measured too.
    let mut stage = Stage::new((entry.build)(), SEED, size, GlyphMode::Unicode);
    for _ in 0..WARMUP_STEPS {
        stage.step(SIM_DT);
    }
    stage.render();

    let mut report = Report::default();
    for _ in 0..FRAMES {
        let ((), u) = measure(|| {
            for _ in 0..STEPS_PER_FRAME {
                stage.step(SIM_DT);
            }
        });
        report.update = add(report.update, u);
        let ((), r) = measure(|| {
            stage.render();
        });
        report.render = add(report.render, r);
    }

    // Glyph-mode switch is a runtime toggle (`t`/--ascii); rendering in ASCII must be
    // allocation-free as well. Switching itself is outside the measured window.
    stage.set_glyph_mode(GlyphMode::Ascii);
    for _ in 0..ASCII_FRAMES {
        let ((), r) = measure(|| {
            stage.render();
        });
        report.render = add(report.render, r);
    }
    report
}

/// Every scene at `size`, one scoped thread each (thread-local counters keep them apart).
fn run_all(size: Size) -> Vec<(&'static str, Report)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = registry()
            .iter()
            .map(|e| (e.meta.name, s.spawn(move || run_scene(e, size))))
            .collect();
        handles
            .into_iter()
            .map(|(name, h)| (name, h.join().unwrap_or_else(|_| panic!("{name} panicked"))))
            .collect()
    })
}

fn table(size: Size, rows: &[(&str, Report)]) -> String {
    let mut out = format!(
        "allocations at {}x{} over {FRAMES} frames (render: +{ASCII_FRAMES} ascii):\n\
         {:<28} {:>10} {:>12} {:>10} {:>12}\n",
        size.width(),
        size.height(),
        "scene",
        "render",
        "render B",
        "update",
        "update B"
    );
    for (name, r) in rows {
        let _ = writeln!(
            out,
            "{name:<28} {:>10} {:>12} {:>10} {:>12}",
            r.render.total(),
            r.render.bytes,
            r.update.total(),
            r.update.bytes
        );
    }
    out
}

fn check(size: Size) {
    let rows = run_all(size);
    let tbl = table(size, &rows);
    // Printed (visible with --nocapture) so the numbers are available even when green.
    println!("{tbl}");

    let mut problems = Vec::new();
    for (name, r) in &rows {
        let render_listed = RENDER_ALLOCATES.iter().any(|(n, _)| n == name);
        match (r.render.total() > 0, render_listed) {
            (true, false) => problems.push(format!(
                "{name}: render allocated {} times ({} B) after warm-up",
                r.render.total(),
                r.render.bytes
            )),
            (false, true) => problems.push(format!(
                "{name}: render no longer allocates - remove it from RENDER_ALLOCATES"
            )),
            _ => {}
        }
        let update_listed = UPDATE_ALLOCATES.contains(name);
        match (r.update.total() > 0, update_listed) {
            (true, false) => problems.push(format!(
                "{name}: update allocated {} times ({} B) in steady state (add to UPDATE_ALLOCATES only with a reason)",
                r.update.total(),
                r.update.bytes
            )),
            (false, true) => problems.push(format!(
                "{name}: update no longer allocates at this size - check UPDATE_ALLOCATES"
            )),
            _ => {}
        }
    }
    assert!(problems.is_empty(), "{}\n\n{tbl}", problems.join("\n"));
}

#[test]
fn gen_r11_render_and_update_are_allocation_free_200x50() {
    check(Size::new(200, 50));
}

#[test]
fn gen_r11_render_and_update_are_allocation_free_80x24() {
    check(Size::new(80, 24));
}

#[test]
fn allowlists_name_registered_scenes_only() {
    let known = |n: &str| registry().iter().any(|e| e.meta.name == n);
    for (n, _) in RENDER_ALLOCATES {
        assert!(known(n), "RENDER_ALLOCATES: unknown scene {n}");
    }
    for n in UPDATE_ALLOCATES {
        assert!(known(n), "UPDATE_ALLOCATES: unknown scene {n}");
    }
}
