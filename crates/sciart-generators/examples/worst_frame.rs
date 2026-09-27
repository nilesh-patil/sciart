//! Worst-frame probe (GEN-R8): the slowest single step + render over a replay, not a mean.
//!
//! ```text
//! cargo run -q --release -p sciart-generators --example worst_frame -- \
//!     [scene ...] [--size 200x50,500x150] [--secs 120] [--replays 3] [--seed 42] [--over MS]
//! ```
//!
//! Each scene is replayed `--replays` times from the same seed; every frame keeps its
//! fastest replay, so a scheduler hiccup has to strike the same frame in every replay to
//! count. Prints, per scene and size: the resize (build) time, the worst frame with its
//! simulated time, and the mean; `--over MS` also lists every frame slower than `MS`.
//! Criterion's means hide act hand-overs; this does not.
#![allow(clippy::disallowed_methods)] // wall-clock timing is the point of this tool

#[allow(dead_code)] // shared with frames/preview; this tool uses a subset
mod common;

use std::process::ExitCode;
use std::time::Instant;

use sciart_core::prelude::*;

const USAGE: &str = "usage: worst_frame [scene ...] [--size WxH,WxH] [--secs S] [--replays N] [--seed N] [--over MS]";

#[derive(Debug)]
struct Args {
    scenes: Vec<&'static sciart_generators::Entry>,
    sizes: Vec<Size>,
    secs: f64,
    replays: usize,
    seed: u64,
    over: Option<f64>,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        scenes: Vec::new(),
        sizes: vec![Size::new(200, 50), Size::new(500, 150)],
        secs: 120.0,
        replays: 3,
        seed: 42,
        over: None,
    };
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--size" => {
                args.sizes = value()?
                    .split(',')
                    .map(common::size)
                    .collect::<Result<_, _>>()?;
            }
            "--secs" => {
                let v = value()?;
                args.secs = v
                    .parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite() && *s > 0.0)
                    .ok_or(format!("bad --secs {v}"))?;
            }
            "--replays" => {
                let v = value()?;
                args.replays = v
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or(format!("bad --replays {v}"))?;
            }
            "--seed" => {
                let v = value()?;
                args.seed = v.parse().map_err(|_| format!("bad --seed {v}"))?;
            }
            "--over" => {
                let v = value()?;
                args.over = Some(v.parse().map_err(|_| format!("bad --over {v}"))?);
            }
            name if !name.starts_with('-') => args.scenes.push(common::scene(name)?),
            other => return Err(format!("unknown flag {other}\n{USAGE}")),
        }
    }
    if args.scenes.is_empty() {
        args.scenes = sciart_generators::registry().iter().collect();
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let frames = (args.secs / SIM_DT).round() as usize;
    println!(
        "{:<28} {:>8} {:>10} {:>10} {:>9} {:>9}",
        "scene", "size", "resize ms", "worst ms", "at s", "mean ms"
    );
    for entry in &args.scenes {
        for &size in &args.sizes {
            let mut best = vec![f64::INFINITY; frames];
            let mut resize = f64::INFINITY;
            for _ in 0..args.replays {
                let t0 = Instant::now();
                let mut stage = Stage::new((entry.build)(), args.seed, size, GlyphMode::Unicode);
                resize = resize.min(t0.elapsed().as_secs_f64() * 1e3);
                for slot in &mut best {
                    let t0 = Instant::now();
                    stage.step(SIM_DT);
                    std::hint::black_box(stage.render());
                    *slot = slot.min(t0.elapsed().as_secs_f64() * 1e3);
                }
            }
            let (at, worst) = best
                .iter()
                .copied()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((0, 0.0));
            let mean = best.iter().sum::<f64>() / frames.max(1) as f64;
            if let Some(over) = args.over {
                for (i, ms) in best.iter().enumerate().filter(|(_, ms)| **ms > over) {
                    println!("    t {:>8.2} s  {ms:.3} ms", (i + 1) as f64 * SIM_DT);
                }
            }
            println!(
                "{:<28} {:>8} {:>10.2} {:>10.3} {:>9.2} {:>9.3}",
                entry.meta.name,
                format!("{}x{}", size.width(), size.height()),
                resize,
                worst,
                (at + 1) as f64 * SIM_DT,
                mean
            );
        }
    }
    ExitCode::SUCCESS
}
