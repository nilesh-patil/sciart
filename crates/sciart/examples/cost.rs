//! Cost probe (RND-R13): what each scene costs the CPU and the terminal, per drawn frame.
//!
//! ```text
//! pixi run rs-cost [scene ...] [--size WxH] [--secs S] [--seed N] [--theme T] [--sort COL]
//!                  [--replays N] [--crossterm] [--exact] [--emit-limits]
//! ```
//!
//! Drives every scene the way the app does: 30 drawn frames per simulated second, each
//! `STEPS_PER_FRAME` fixed steps + one render through `Stage`, then the app's `Painter`
//! into a ratatui buffer, then the same protocol as `ratatui::Terminal::draw` (diff
//! against the previous frame, `draw`, `hide_cursor`, `flush`, swap, reset) through the
//! app's `FrameBackend` into a `LineWriter` like `stdout`'s. Reported per scene (means
//! over the steady-state frames, the first full-screen draw excluded):
//!
//! - `upd+rnd`: the steps and the render, µs;
//! - `paint`: the painter, µs;
//! - `diff+enc`: diff + escape encoding + the buffer swap/reset, µs;
//! - `B/frame`, `p95`: bytes written per frame, mean and 95th percentile;
//! - `cells`: changed cells per frame; `wr`: `write` calls (syscalls) per frame;
//! - `blk@5`, `blk@20`, `blk2@20` (2×2 squares, not ratcheted), `dark@5`, `dark@20`,
//!   `luma@20`, `cov@5`, `cov@20` (lit coverage), `cmass@5`, `cmass@20` (chroma mass),
//!   `edge@5`, `edge@20` (edge weight), `neon` (chroma p90 at the worse time; cap 0.20): the
//!   GEN-R17/R18/R21 art metrics of the exact `astro` frame at t = 5 s and 20 s
//!   (`sciart_render::metrics`), when `--secs` reaches them.
//!
//! `--sort` orders the rows by `bytes` (descending), `cpu` (descending), `blk` (blk@20,
//! descending), `dark` (dark@20, ascending), `cov`, `cmass`, `edge`, `neon` (at 20 s,
//! descending) or `name` (default: registry order).
//! `--crossterm` uses ratatui's `CrosstermBackend` and `--exact` turns
//! the RND-R14 colour hysteresis off, for before/after comparisons. `--emit-limits`
//! prints ready-to-paste `LIMITS` rows for `crates/sciart/tests/art_metrics.rs` instead.
//! `--replays N` runs each scene N times and keeps every frame's fastest time (use 3 or
//! more on a busy machine).
#![allow(clippy::disallowed_methods)] // wall-clock timing is the point of this tool

use std::cell::Cell;
use std::io::{self, LineWriter, Write};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sciart_core::prelude::*;
use sciart_generators::Entry;
use sciart_render::metrics::{ArtMetrics, Clean, art_metrics};
use sciart_render::{FrameBackend, Painter, Theme, paint};

const USAGE: &str = "usage: cost [scene ...] [--size WxH] [--secs S] [--seed N] [--theme T] \
                     [--sort bytes|cpu|blk|dark|cov|cmass|edge|neon|name] [--replays N] [--crossterm] [--exact] [--emit-limits]";
/// The app's default 30 FPS over the fixed 60 Hz update (ADR-0003).
const STEPS_PER_FRAME: u64 = 2;
/// Art metrics are sampled at these simulated times (GEN-R17, GEN-R18, GEN-R21).
const METRIC_TIMES: [f64; 2] = [5.0, 20.0];

#[derive(Debug)]
struct Args {
    scenes: Vec<&'static Entry>,
    size: Size,
    secs: f64,
    seed: u64,
    theme: Theme,
    sort: String,
    /// ratatui's `CrosstermBackend` instead of the app's `FrameBackend` (before/after).
    crossterm: bool,
    /// No colour hysteresis (RND-R14), for before/after.
    exact: bool,
    /// Print `LIMITS` rows for `tests/art_metrics.rs` instead of the table.
    emit_limits: bool,
    /// Replays per scene; timings keep each frame's fastest replay.
    replays: usize,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        scenes: Vec::new(),
        size: Size::new(200, 50),
        secs: 20.0,
        seed: 42,
        theme: Theme::Astro,
        sort: "name".into(),
        crossterm: false,
        exact: false,
        emit_limits: false,
        replays: 1,
    };
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--size" => {
                let v = value()?;
                let dim = |s: &str| s.parse::<u16>().ok().filter(|&n| n > 0);
                args.size = v
                    .split_once('x')
                    .and_then(|(w, h)| dim(w).zip(dim(h)))
                    .map(|(w, h)| Size::new(w, h))
                    .ok_or(format!("bad --size {v}"))?;
            }
            "--secs" => {
                let v = value()?;
                args.secs = v
                    .parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite() && *s > 0.0)
                    .ok_or(format!("bad --secs {v}"))?;
            }
            "--seed" => {
                let v = value()?;
                args.seed = v.parse().map_err(|_| format!("bad --seed {v}"))?;
            }
            "--theme" => {
                let v = value()?;
                args.theme = Theme::from_name(&v).ok_or(format!("bad --theme {v}"))?;
            }
            "--sort" => {
                let v = value()?;
                if ![
                    "bytes", "cpu", "blk", "dark", "cov", "cmass", "edge", "neon", "name",
                ]
                .contains(&v.as_str())
                {
                    return Err(format!("bad --sort {v}\n{USAGE}"));
                }
                args.sort = v;
            }
            "--replays" => {
                let v = value()?;
                args.replays = v
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or(format!("bad --replays {v}"))?;
            }
            "--crossterm" => args.crossterm = true,
            "--exact" => args.exact = true,
            "--emit-limits" => args.emit_limits = true,
            name if !name.starts_with('-') => args
                .scenes
                .push(sciart_generators::find(name).ok_or(format!("unknown scene {name:?}"))?),
            other => return Err(format!("unknown flag {other}\n{USAGE}")),
        }
    }
    if args.scenes.is_empty() {
        args.scenes = sciart_generators::registry().iter().collect();
    }
    Ok(args)
}

#[derive(Debug, Default)]
struct Row {
    name: &'static str,
    update_us: f64,
    paint_us: f64,
    encode_us: f64,
    bytes_mean: f64,
    bytes_p95: usize,
    cells: f64,
    writes: f64,
    /// Per frame: update+render, paint, diff+encode µs.
    times: Vec<[f64; 3]>,
    metrics: [Option<ArtMetrics>; 2],
}

/// Counts the bytes and the `write` calls that reach it: behind a `LineWriter`, like
/// `stdout`, the calls are the frame's `write` syscalls.
#[derive(Debug, Clone, Default)]
struct Sink(Rc<Cell<(u64, u64)>>);

impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let (bytes, writes) = self.0.get();
        self.0.set((bytes + b.len() as u64, writes + 1));
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `std::io::stdout()` is a `LineWriter` with a 1 KiB buffer.
fn stdout_like(sink: &Sink) -> LineWriter<Sink> {
    LineWriter::with_capacity(1024, sink.clone())
}

fn probe(entry: &Entry, args: &Args) -> Row {
    let once = || {
        let sink = Sink::default();
        let (w, h) = (args.size.width(), args.size.height());
        if args.crossterm {
            let backend = CrosstermBackend::new(stdout_like(&sink));
            run(entry, args, backend, &sink)
        } else {
            let backend = FrameBackend::with_size(stdout_like(&sink), w, h);
            run(entry, args, backend, &sink)
        }
    };
    // Every replay is identical but for the clock: each frame keeps its fastest time, so
    // a scheduler hiccup has to hit the same frame in every replay to count.
    let mut row = once();
    for _ in 1..args.replays {
        let again = once();
        for (t, u) in row.times.iter_mut().zip(&again.times) {
            for k in 0..3 {
                t[k] = t[k].min(u[k]);
            }
        }
    }
    let n = row.times.len().max(1) as f64;
    let mean = |k: usize| row.times.iter().map(|t| t[k]).sum::<f64>() / n;
    (row.update_us, row.paint_us, row.encode_us) = (mean(0), mean(1), mean(2));
    row
}

/// One scene through `Stage`, the app's `Painter` and the buffer protocol of
/// `ratatui::Terminal::draw` (diff, draw, hide cursor, flush, swap, reset).
fn run<B: Backend>(entry: &Entry, args: &Args, mut backend: B, sink: &Sink) -> Row {
    let size = args.size;
    let area = Rect::new(0, 0, size.width(), size.height());
    let mut stage = Stage::new((entry.build)(), args.seed, size, GlyphMode::Unicode);
    let mut painter = if args.exact {
        Painter::exact()
    } else {
        Painter::new()
    };
    let (mut prev, mut cur) = (Buffer::empty(area), Buffer::empty(area));
    let frames = (args.secs * 30.0).round() as u64;
    let metric_frames = METRIC_TIMES.map(|t| (t / SIM_DT / STEPS_PER_FRAME as f64).round() as u64);

    let mut row = Row {
        name: entry.meta.name,
        ..Row::default()
    };
    let mut bytes = Vec::with_capacity(frames as usize);
    let mut times = Vec::with_capacity(frames as usize);
    let (mut cells, mut writes) = (0usize, 0u64);
    for frame in 1..=frames {
        let t0 = Instant::now();
        for _ in 0..STEPS_PER_FRAME {
            stage.step(SIM_DT);
        }
        stage.render();
        let t1 = Instant::now();
        painter.paint(stage.frame(), args.theme, area, &mut cur);
        let t2 = Instant::now();
        let before = sink.0.get();
        let mut changed = 0usize;
        {
            let diff = prev.diff_iter(&cur).inspect(|_| changed += 1);
            // The sink cannot fail.
            let _ = backend.draw(diff);
            let _ = backend.hide_cursor();
            let _ = backend.flush();
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.reset();
        let t3 = Instant::now();
        let after = sink.0.get();
        // Measured outside the timed spans, on the exact colours (RND-R13).
        for (k, &at) in metric_frames.iter().enumerate() {
            if frame == at {
                let mut astro = Buffer::empty(area);
                paint(stage.frame(), Theme::Astro, area, &mut astro);
                row.metrics[k] = Some(art_metrics(&astro));
            }
        }
        if frame == 1 {
            continue; // the first frame redraws the whole screen
        }
        times.push([t1 - t0, t2 - t1, t3 - t2].map(|d| d.as_secs_f64() * 1e6));
        bytes.push((after.0 - before.0) as usize);
        writes += after.1 - before.1;
        cells += changed;
    }
    let n = bytes.len().max(1) as f64;
    row.times = times;
    row.bytes_mean = bytes.iter().sum::<usize>() as f64 / n;
    row.cells = cells as f64 / n;
    row.writes = writes as f64 / n;
    bytes.sort_unstable();
    row.bytes_p95 = bytes
        .get(((bytes.len() as f64 * 0.95) as usize).min(bytes.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0);
    row
}

/// Ready-to-paste `LIMITS` rows for `crates/sciart/tests/art_metrics.rs`: the measured
/// values with the table's slack (blockiness +0.02, darkness −0.02, lit coverage +0.02,
/// chroma mass +0.003, edge weight +10 % + 0.2), alphabetical.
fn emit_limits(rows: &[Row]) {
    let up_by = |v: f64, slack: f64, unit: f64| ((v + slack) / unit - 1e-9).ceil() * unit;
    let up = |v: f64| up_by(v, 0.02, 1e-3);
    let dn = |v: f64| (((v - 0.02) * 1000.0 + 1e-9).floor() / 1000.0).max(0.0);
    let cov = |v: f64| up_by(v, 0.02, 1e-3).min(1.0);
    let mass = |v: f64| up_by(v, 0.003, 1e-4);
    let edge = |v: f64| up_by(v * 1.1, 0.2, 0.1);
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by_key(|r| r.name);
    for r in sorted {
        let [Some(a), Some(b)] = r.metrics else {
            eprintln!("{}: --secs must reach {METRIC_TIMES:?}", r.name);
            continue;
        };
        let (ca, cb) = (a.clean, b.clean);
        println!(
            "    (\"{}\", [{:.3}, {:.3}], [{:.3}, {:.3}], [{:.3}, {:.3}], [{:.4}, {:.4}], [{:.1}, {:.1}]),",
            r.name,
            up(a.blockiness),
            up(b.blockiness),
            dn(a.darkness),
            dn(b.darkness),
            cov(ca.coverage),
            cov(cb.coverage),
            mass(ca.chroma_mass),
            mass(cb.chroma_mass),
            edge(ca.edge_weight),
            edge(cb.edge_weight),
        );
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut rows: Vec<Row> = args.scenes.iter().map(|e| probe(e, &args)).collect();
    if args.emit_limits {
        emit_limits(&rows);
        return ExitCode::SUCCESS;
    }
    let blk20 = |r: &Row| r.metrics[1].map_or(0.0, |m| m.blockiness);
    let dark20 = |r: &Row| r.metrics[1].map_or(1.0, |m| m.darkness);
    let clean20 = |r: &Row, f: fn(Clean) -> f64| r.metrics[1].map_or(0.0, |m| f(m.clean));
    match args.sort.as_str() {
        "bytes" => rows.sort_by(|a, b| b.bytes_mean.total_cmp(&a.bytes_mean)),
        "cpu" => rows.sort_by(|a, b| {
            let cpu = |r: &Row| r.update_us + r.paint_us + r.encode_us;
            cpu(b).total_cmp(&cpu(a))
        }),
        "blk" => rows.sort_by(|a, b| blk20(b).total_cmp(&blk20(a))),
        "dark" => rows.sort_by(|a, b| dark20(a).total_cmp(&dark20(b))),
        "cov" | "cmass" | "edge" | "neon" => {
            let f: fn(Clean) -> f64 = match args.sort.as_str() {
                "cov" => |c| c.coverage,
                "cmass" => |c| c.chroma_mass,
                "edge" => |c| c.edge_weight,
                _ => |c| c.chroma_p90,
            };
            rows.sort_by(|a, b| clean20(b, f).total_cmp(&clean20(a, f)));
        }
        _ => {}
    }
    println!(
        "# rs-cost {}x{} {} s seed {} theme {} ({} fps draw, {} steps/frame, {} backend, {})",
        args.size.width(),
        args.size.height(),
        args.secs,
        args.seed,
        args.theme.name(),
        30,
        STEPS_PER_FRAME,
        if args.crossterm { "crossterm" } else { "frame" },
        if args.exact { "exact" } else { "hysteresis" },
    );
    println!(
        "{:<28} {:>8} {:>7} {:>8} {:>8} {:>8} {:>6} {:>5} {:>6} {:>6} {:>7} {:>6} {:>6} {:>7} {:>6} {:>6} {:>7} {:>7} {:>6} {:>6} {:>6}",
        "scene",
        "upd+rnd",
        "paint",
        "diff+enc",
        "B/frame",
        "p95",
        "cells",
        "wr",
        "blk@5",
        "blk@20",
        "blk2@20",
        "dark@5",
        "dark@20",
        "luma@20",
        "cov@5",
        "cov@20",
        "cmass@5",
        "cmass@20",
        "edge@5",
        "edge@20",
        "neon"
    );
    let fmt = |m: Option<ArtMetrics>, digits: usize, f: fn(ArtMetrics) -> f64| {
        m.map_or_else(|| "-".to_owned(), |m| format!("{:.*}", digits, f(m)))
    };
    let metric = |m: Option<ArtMetrics>, f: fn(ArtMetrics) -> f64| fmt(m, 3, f);
    for r in &rows {
        println!(
            "{:<28} {:>8.1} {:>7.1} {:>8.1} {:>8.0} {:>8} {:>6.0} {:>5.1} {:>6} {:>6} {:>7} {:>6} {:>6} {:>7} {:>6} {:>6} {:>7} {:>7} {:>6} {:>6} {:>6}",
            r.name,
            r.update_us,
            r.paint_us,
            r.encode_us,
            r.bytes_mean,
            r.bytes_p95,
            r.cells,
            r.writes,
            metric(r.metrics[0], |m| m.blockiness),
            metric(r.metrics[1], |m| m.blockiness),
            metric(r.metrics[1], |m| m.blockiness_2x2),
            metric(r.metrics[0], |m| m.darkness),
            metric(r.metrics[1], |m| m.darkness),
            metric(r.metrics[1], |m| m.mean_luma),
            metric(r.metrics[0], |m| m.clean.coverage),
            metric(r.metrics[1], |m| m.clean.coverage),
            fmt(r.metrics[0], 4, |m| m.clean.chroma_mass),
            fmt(r.metrics[1], 4, |m| m.clean.chroma_mass),
            fmt(r.metrics[0], 1, |m| m.clean.edge_weight),
            fmt(r.metrics[1], 1, |m| m.clean.edge_weight),
            r.metrics
                .iter()
                .flatten()
                .map(|m| m.clean.chroma_p90)
                .reduce(f64::max)
                .map_or_else(|| "-".to_owned(), |v| format!("{v:.3}")),
        );
    }
    let n = rows.len().max(1) as f64;
    let mean = |f: fn(&Row) -> f64| rows.iter().map(f).sum::<f64>() / n;
    let bytes = mean(|r| r.bytes_mean);
    println!(
        "{:<28} {:>8.1} {:>7.1} {:>8.1} {:>8.0} {:>8} {:>6.0} {:>5.1}   => {:.0} KiB/s at 30 fps",
        "MEAN",
        mean(|r| r.update_us),
        mean(|r| r.paint_us),
        mean(|r| r.encode_us),
        bytes,
        "",
        mean(|r| r.cells),
        mean(|r| r.writes),
        bytes * 30.0 / 1024.0
    );
    ExitCode::SUCCESS
}
