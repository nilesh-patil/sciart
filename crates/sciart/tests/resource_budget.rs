//! RT-R16 (M15, ADR-0018): restoring the M11 look keeps M14's resource gains. Every
//! registered scene at 200×50, seed 42, 10 s of simulated time drawn at 30 FPS through the
//! real frame pipeline (`Stage`, the app's `Painter` with its colour hysteresis, the
//! buffer diff and the app's `FrameBackend`, as `examples/cost.rs` drives it): the mean
//! terminal output per frame and the mean number of changed cells per frame, over all
//! scenes, stay within budget. The first frame (a full-screen draw) is excluded, as in
//! `rs-cost`. Output is deterministic, so this is a test; CPU time stays a release-mode
//! check (GEN-R8, `rs-cost`, `rs-worst`).

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};
use sciart_render::{FrameBackend, Painter, Theme};

const SEED: u64 = 42;
const SIZE: (u16, u16) = (200, 50);
const SECS: u64 = 10;
const FPS: u64 = 30;
/// The runtime takes two fixed steps per drawn frame at 30 FPS (ADR-0003).
const STEPS_PER_FRAME: u64 = 2;

/// Mean bytes per frame over all scenes (M14: 18 812).
const BYTES_BUDGET: f64 = 20_480.0;
/// Mean changed cells per frame over all scenes (M14: 806).
const CELLS_BUDGET: f64 = 880.0;

/// Counts the bytes that reach it.
#[derive(Default)]
struct Count(u64);

impl Write for Count {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0 += b.len() as u64;
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Per scene: (name, mean bytes per frame, mean changed cells per frame).
type Cost = (&'static str, f64, f64);

fn cost(entry: &Entry) -> Cost {
    let (w, h) = SIZE;
    let area = Rect::new(0, 0, w, h);
    let mut stage = Stage::new((entry.build)(), SEED, Size::new(w, h), GlyphMode::Unicode);
    let mut painter = Painter::new();
    let mut backend = FrameBackend::with_size(Count::default(), w, h);
    let (mut prev, mut cur) = (Buffer::empty(area), Buffer::empty(area));
    let (mut bytes, mut cells, mut counted) = (0u64, 0u64, 0u64);
    for frame in 1..=SECS * FPS {
        for _ in 0..STEPS_PER_FRAME {
            stage.step(SIM_DT);
        }
        stage.render();
        painter.paint(stage.frame(), Theme::Astro, area, &mut cur);
        let before = backend.writer().0;
        let mut changed = 0u64;
        {
            // `ratatui::Terminal::draw`'s protocol: diff, draw, hide the cursor, flush.
            let diff = prev.diff_iter(&cur).inspect(|_| changed += 1);
            backend.draw(diff).expect("counting sink cannot fail");
            backend.hide_cursor().expect("counting sink cannot fail");
            backend.flush().expect("counting sink cannot fail");
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.reset();
        if frame > 1 {
            bytes += backend.writer().0 - before;
            cells += changed;
            counted += 1;
        }
    }
    let n = counted.max(1) as f64;
    (entry.meta.name, bytes as f64 / n, cells as f64 / n)
}

/// Every scene's cost, on parallel threads (scenes are independent).
fn all() -> Vec<Cost> {
    let entries = registry();
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<Cost> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(entry) = entries.get(i) else { break };
                        out.push(cost(entry));
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("cost worker panicked"))
            .collect()
    });
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

#[test]
fn rt_r16_gallery_mean_bytes_and_changed_cells_stay_within_budget() {
    let costs = all();
    assert_eq!(costs.len(), registry().len());
    let n = costs.len() as f64;
    let bytes = costs.iter().map(|c| c.1).sum::<f64>() / n;
    let cells = costs.iter().map(|c| c.2).sum::<f64>() / n;
    println!("RT-R16: {bytes:.0} B and {cells:.0} changed cells per frame over {n} scenes");

    let top = |key: fn(&Cost) -> f64, unit: &str| {
        let mut rows: Vec<&Cost> = costs.iter().collect();
        rows.sort_by(|a, b| key(b).total_cmp(&key(a)));
        rows.iter()
            .take(10)
            .map(|c| format!("  {:>9.0} {unit}  {}", key(c), c.0))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        bytes <= BYTES_BUDGET && cells <= CELLS_BUDGET,
        "RT-R16: mean {bytes:.0} B/frame (budget {BYTES_BUDGET}) and {cells:.0} changed \
         cells/frame (budget {CELLS_BUDGET}) over {n} scenes.\nTop 10 by bytes:\n{}\n\
         Top 10 by changed cells:\n{}",
        top(|c| c.1, "B"),
        top(|c| c.2, "cells"),
    );
}
