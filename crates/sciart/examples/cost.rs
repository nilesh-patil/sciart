//! Cost probe (RND-R13): what each scene costs the CPU and the terminal, per drawn frame.
//!
//! ```text
//! pixi run rs-cost [scene ...] [--size WxH] [--secs S] [--seed N] [--theme T] [--sort COL]
//! ```
//!
//! Drives every scene the way the app does: 30 drawn frames per simulated second, each
//! `STEPS_PER_FRAME` fixed steps + one render through `Stage`, then the real `paint` into
//! a ratatui buffer, then ratatui's diff against the previous frame encoded by the
//! crossterm backend into an in-memory writer. Reported per scene (means over the
//! steady-state frames, the first full-screen draw excluded):
//!
//! - `upd+rnd`: the steps and the render, µs;
//! - `paint`: `paint`, µs;
//! - `diff+enc`: diff + escape encoding + the buffer swap/reset, µs;
//! - `B/frame`, `p95`: bytes written per frame, mean and 95th percentile;
//! - `cells`: changed cells per frame;
//! - `blk@5`, `blk@20`, `blk2@20` (2×2 squares, not ratcheted), `dark@5`, `dark@20`, `luma@20`, `col@5`, `col@20` (colourfulness), `con@5`, `con@20` (RMS contrast): the GEN-R17/R18/R21 art metrics of the
//!   `astro` frame at t = 5 s and 20 s (`sciart_render::metrics`), when `--secs` reaches
//!   them.
//!
//! `--sort` orders the rows by `bytes` (descending), `cpu` (descending), `blk` (blk@20,
//! descending), `dark` (dark@20, ascending), `col` (col@20, ascending) or `name` (default: registry order).
#![allow(clippy::disallowed_methods)] // wall-clock timing is the point of this tool

use std::process::ExitCode;
use std::time::Instant;

use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sciart_core::prelude::*;
use sciart_generators::Entry;
use sciart_render::metrics::{ArtMetrics, art_metrics};
use sciart_render::{Theme, paint};

const USAGE: &str = "usage: cost [scene ...] [--size WxH] [--secs S] [--seed N] [--theme T] [--sort bytes|cpu|blk|dark|col|name]";
/// The app's default 30 FPS over the fixed 60 Hz update (ADR-0003).
const STEPS_PER_FRAME: u64 = 2;
/// Art metrics are sampled at these simulated times (GEN-R17, GEN-R18).
const METRIC_TIMES: [f64; 2] = [5.0, 20.0];

#[derive(Debug)]
struct Args {
    scenes: Vec<&'static Entry>,
    size: Size,
    secs: f64,
    seed: u64,
    theme: Theme,
    sort: String,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        scenes: Vec::new(),
        size: Size::new(200, 50),
        secs: 20.0,
        seed: 42,
        theme: Theme::Astro,
        sort: "name".into(),
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
                if !["bytes", "cpu", "blk", "dark", "col", "name"].contains(&v.as_str()) {
                    return Err(format!("bad --sort {v}\n{USAGE}"));
                }
                args.sort = v;
            }
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
    metrics: [Option<ArtMetrics>; 2],
}

fn probe(entry: &Entry, args: &Args) -> Row {
    let size = args.size;
    let area = Rect::new(0, 0, size.width(), size.height());
    let mut stage = Stage::new((entry.build)(), args.seed, size, GlyphMode::Unicode);
    let (mut prev, mut cur) = (Buffer::empty(area), Buffer::empty(area));
    let mut out: Vec<u8> = Vec::with_capacity(1 << 20);
    let frames = (args.secs * 30.0).round() as u64;
    let metric_frames = METRIC_TIMES.map(|t| (t / SIM_DT / STEPS_PER_FRAME as f64).round() as u64);

    let mut row = Row {
        name: entry.meta.name,
        ..Row::default()
    };
    let mut bytes = Vec::with_capacity(frames as usize);
    let (mut upd, mut pnt, mut enc, mut cells) = (0.0, 0.0, 0.0, 0usize);
    for frame in 1..=frames {
        let t0 = Instant::now();
        for _ in 0..STEPS_PER_FRAME {
            stage.step(SIM_DT);
        }
        stage.render();
        let t1 = Instant::now();
        paint(stage.frame(), args.theme, area, &mut cur);
        let t2 = Instant::now();
        out.clear();
        let mut changed = 0usize;
        {
            let mut backend = CrosstermBackend::new(&mut out);
            let diff = prev.diff_iter(&cur).inspect(|_| changed += 1);
            // Writing into a `Vec` cannot fail.
            let _ = backend.draw(diff);
            let _ = Backend::flush(&mut backend);
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.reset();
        let t3 = Instant::now();
        // After the swap `prev` holds this frame; measured outside the timed spans.
        for (k, &at) in metric_frames.iter().enumerate() {
            if frame == at {
                row.metrics[k] = Some(if args.theme == Theme::Astro {
                    art_metrics(&prev)
                } else {
                    // The metrics are defined on the astro colours (RND-R13).
                    let mut astro = Buffer::empty(area);
                    paint(stage.frame(), Theme::Astro, area, &mut astro);
                    art_metrics(&astro)
                });
            }
        }
        if frame == 1 {
            continue; // the first frame redraws the whole screen
        }
        upd += (t1 - t0).as_secs_f64();
        pnt += (t2 - t1).as_secs_f64();
        enc += (t3 - t2).as_secs_f64();
        bytes.push(out.len());
        cells += changed;
    }
    let n = bytes.len().max(1) as f64;
    row.update_us = upd / n * 1e6;
    row.paint_us = pnt / n * 1e6;
    row.encode_us = enc / n * 1e6;
    row.bytes_mean = bytes.iter().sum::<usize>() as f64 / n;
    row.cells = cells as f64 / n;
    bytes.sort_unstable();
    row.bytes_p95 = bytes
        .get(((bytes.len() as f64 * 0.95) as usize).min(bytes.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0);
    row
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
    let blk20 = |r: &Row| r.metrics[1].map_or(0.0, |m| m.blockiness);
    let dark20 = |r: &Row| r.metrics[1].map_or(1.0, |m| m.darkness);
    let col20 = |r: &Row| r.metrics[1].map_or(0.0, |m| m.colourfulness);
    match args.sort.as_str() {
        "bytes" => rows.sort_by(|a, b| b.bytes_mean.total_cmp(&a.bytes_mean)),
        "cpu" => rows.sort_by(|a, b| {
            let cpu = |r: &Row| r.update_us + r.paint_us + r.encode_us;
            cpu(b).total_cmp(&cpu(a))
        }),
        "blk" => rows.sort_by(|a, b| blk20(b).total_cmp(&blk20(a))),
        "dark" => rows.sort_by(|a, b| dark20(a).total_cmp(&dark20(b))),
        "col" => rows.sort_by(|a, b| col20(a).total_cmp(&col20(b))),
        _ => {}
    }
    println!(
        "# rs-cost {}x{} {} s seed {} theme {} ({} fps draw, {} steps/frame)",
        args.size.width(),
        args.size.height(),
        args.secs,
        args.seed,
        args.theme.name(),
        30,
        STEPS_PER_FRAME
    );
    println!(
        "{:<28} {:>8} {:>7} {:>8} {:>8} {:>8} {:>6} {:>6} {:>6} {:>7} {:>6} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6}",
        "scene",
        "upd+rnd",
        "paint",
        "diff+enc",
        "B/frame",
        "p95",
        "cells",
        "blk@5",
        "blk@20",
        "blk2@20",
        "dark@5",
        "dark@20",
        "luma@20",
        "col@5",
        "col@20",
        "con@5",
        "con@20"
    );
    let metric = |m: Option<ArtMetrics>, f: fn(ArtMetrics) -> f64| {
        m.map_or_else(|| "-".to_owned(), |m| format!("{:.3}", f(m)))
    };
    let metric1 = |m: Option<ArtMetrics>, f: fn(ArtMetrics) -> f64| {
        m.map_or_else(|| "-".to_owned(), |m| format!("{:.1}", f(m)))
    };
    for r in &rows {
        println!(
            "{:<28} {:>8.1} {:>7.1} {:>8.1} {:>8.0} {:>8} {:>6.0} {:>6} {:>6} {:>7} {:>6} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6}",
            r.name,
            r.update_us,
            r.paint_us,
            r.encode_us,
            r.bytes_mean,
            r.bytes_p95,
            r.cells,
            metric(r.metrics[0], |m| m.blockiness),
            metric(r.metrics[1], |m| m.blockiness),
            metric(r.metrics[1], |m| m.blockiness_2x2),
            metric(r.metrics[0], |m| m.darkness),
            metric(r.metrics[1], |m| m.darkness),
            metric(r.metrics[1], |m| m.mean_luma),
            metric1(r.metrics[0], |m| m.colourfulness),
            metric1(r.metrics[1], |m| m.colourfulness),
            metric(r.metrics[0], |m| m.contrast),
            metric(r.metrics[1], |m| m.contrast),
        );
    }
    let n = rows.len().max(1) as f64;
    let mean = |f: fn(&Row) -> f64| rows.iter().map(f).sum::<f64>() / n;
    let bytes = mean(|r| r.bytes_mean);
    println!(
        "{:<28} {:>8.1} {:>7.1} {:>8.1} {:>8.0} {:>8} {:>6.0}   => {:.0} KiB/s at 30 fps",
        "MEAN",
        mean(|r| r.update_us),
        mean(|r| r.paint_us),
        mean(|r| r.encode_us),
        bytes,
        "",
        mean(|r| r.cells),
        bytes * 30.0 / 1024.0
    );
    ExitCode::SUCCESS
}
