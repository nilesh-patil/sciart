//! Truecolor review tool: renders a scene's frames through the real `Theme` into PNGs, so
//! art reviewers can see colour without a terminal.
//!
//! The renderer lives in `sciart-capture` (ADR-0025), so this example and `sciart record`
//! draw identical frames: frame 0 here at `t = 0` is the recording's frame 0.
//!
//! ```text
//! cargo run -q --release -p sciart-generators --example preview -- <scene> \
//!     [--size 120x40] [--t 0,5,20] [--theme astro] [--seed 42] [--ascii] \
//!     [--out target/preview/<scene>-<theme>] [--sheet]
//! cargo run -q --release -p sciart-generators --example preview -- --swatches [--out …]
//! ```
//!
//! Frames are stepped exactly as `frames` does. Each cell is 8 × 16 px on a dark terminal
//! background: braille dots (2 × 4) and half and full blocks are drawn exactly, other
//! glyphs through the 8 × 8 `font8x8` bitmap font with rows doubled, and glyphs it lacks
//! through the scene's ASCII fallback. Writes `<out>-t<t>.png` per time, or one
//! `<out>.png` stacking every time with `--sheet`. `--swatches` writes every field role
//! (Seq, Div, Phase, Blackbody, each ramp) under every theme instead.

mod common;

use std::path::PathBuf;
use std::process::ExitCode;

use sciart_capture::{Image, RenderOptions, paint, render_frame, write_png};
use sciart_core::prelude::*;
use sciart_render::{Rgb, Theme};

const USAGE: &str = "usage: preview <scene> [--size WxH] [--t 0,5,20] [--theme astro|rainbow|mono] \
[--seed N] [--ascii] [--out PREFIX] [--sheet]\n       preview --swatches [--out PREFIX]";

/// Gap between stacked frames in a sheet.
const GAP: usize = 6;
const GAP_COLOUR: Rgb = Rgb(60, 60, 72);

#[derive(Debug)]
struct Args {
    scene: String,
    size: Size,
    times: Vec<f64>,
    theme: Theme,
    seed: u64,
    ascii: bool,
    out: Option<PathBuf>,
    sheet: bool,
    swatches: bool,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        scene: String::new(),
        size: Size::new(120, 40),
        times: vec![0.0, 5.0, 20.0],
        theme: Theme::Astro,
        seed: 42,
        ascii: false,
        out: None,
        sheet: false,
        swatches: false,
    };
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--size" => args.size = common::size(&value()?)?,
            "--t" => args.times = common::times(&value()?)?,
            "--theme" => {
                let v = value()?;
                args.theme = Theme::from_name(&v).ok_or(format!("bad --theme {v}"))?;
            }
            "--seed" => {
                let v = value()?;
                args.seed = v.parse().map_err(|_| format!("bad --seed {v}"))?;
            }
            "--out" => args.out = Some(PathBuf::from(value()?)),
            "--ascii" => args.ascii = true,
            "--sheet" => args.sheet = true,
            "--swatches" => args.swatches = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("unknown flag {s}")),
            s if args.scene.is_empty() => args.scene = s.to_owned(),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    if args.scene.is_empty() && !args.swatches {
        return Err("missing <scene>".to_owned());
    }
    Ok(args)
}

/// Stack `frames` vertically with a gap between them.
fn sheet(frames: &[Image]) -> Image {
    let width = frames.iter().map(Image::width).max().unwrap_or(1);
    let height = frames.iter().map(|f| f.height() + GAP).sum::<usize>() - GAP;
    let mut img = Image::new(width, height.max(1), GAP_COLOUR);
    let mut y = 0;
    for f in frames {
        img.paste(f, y);
        y += f.height() + GAP;
    }
    img
}

/// Every field role as a 256-level strip, one block per theme (rows: Seq, Div, Phase,
/// Blackbody, then each ramp), labelled on the left.
fn swatches() -> Image {
    let roles: Vec<(&str, Role)> = [("seq", Role::Seq), ("div", Role::Div)]
        .into_iter()
        .chain([("phase", Role::Phase), ("blackbody", Role::Blackbody)])
        .chain(Ramp::ALL.map(|r| (r.name(), Role::Ramp(r))))
        .collect();
    const LABEL: u16 = 11;
    let rows = u16::try_from(roles.len() + 1).unwrap_or(u16::MAX);
    let frames: Vec<Image> = Theme::ALL
        .into_iter()
        .map(|theme| {
            let mut canvas = Canvas::new(Size::new(LABEL + 256, rows), GlyphMode::Unicode);
            canvas.text(0, 0, theme.name(), Ink::new(Role::Text, 255));
            for (row, (name, role)) in (1..).zip(&roles) {
                canvas.text(0, row, name, Ink::new(Role::Text, 200));
                for level in 0..=255u8 {
                    let x = i32::from(LABEL) + i32::from(level);
                    canvas.put(x, row, Glyph::new('█', '#'), Ink::new(*role, level));
                }
            }
            paint(&canvas, &canvas, theme)
        })
        .collect();
    sheet(&frames)
}

fn run(args: &Args) -> Result<(), String> {
    if args.swatches {
        let out = args
            .out
            .clone()
            .unwrap_or_else(|| "target/preview/swatches".into());
        let path = PathBuf::from(format!("{}.png", out.display()));
        write_png(&swatches(), &path).map_err(|e| e.to_string())?;
        println!("{}", path.display());
        return Ok(());
    }
    let entry = common::scene(&args.scene)?;
    let out = args.out.clone().unwrap_or_else(|| {
        format!("target/preview/{}-{}", entry.meta.name, args.theme.name()).into()
    });
    let mode = if args.ascii {
        GlyphMode::Ascii
    } else {
        GlyphMode::Unicode
    };
    let options = RenderOptions {
        theme: args.theme,
        ascii: args.ascii,
    };
    let mut stage = Stage::new((entry.build)(), args.seed, args.size, mode);
    let mut steps = 0;
    let mut frames = Vec::new();
    for &t in &args.times {
        common::step_to(&mut stage, &mut steps, t);
        let img = render_frame(&mut stage, &options);
        if args.sheet {
            frames.push(img);
        } else {
            let path = PathBuf::from(format!("{}-t{t}.png", out.display()));
            write_png(&img, &path).map_err(|e| e.to_string())?;
            println!("{}", path.display());
        }
    }
    if args.sheet {
        let path = PathBuf::from(format!("{}.png", out.display()));
        write_png(&sheet(&frames), &path).map_err(|e| e.to_string())?;
        println!("{} (t = {:?} s, top to bottom)", path.display(), args.times);
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("preview: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("preview: {e}");
            ExitCode::FAILURE
        }
    }
}
