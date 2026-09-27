//! Scene review tool: prints a scene's frames as text, optionally with its ink layout.
//!
//! ```text
//! cargo run -q -p sciart-generators --example frames -- <scene> \
//!     [--size 80x24] [--t 0,1,5,20] [--ascii] [--inks] [--seed 42]
//! ```
//!
//! Times are simulated seconds, stepped at the fixed `SIM_DT` exactly as the app does.
//! `--inks` adds two same-sized grids per frame (foreground and background roles), one
//! letter per cell: uppercase for level >= 128, lowercase below, '.' for Background.

mod common;

use std::process::ExitCode;

use sciart_core::prelude::*;

const USAGE: &str =
    "usage: frames <scene> [--size WxH] [--t 0,1,5,20] [--ascii] [--inks] [--seed N]";
const LEGEND: &str = "roles: D=Dim S=Star U=Sun P=Planet I=Particle W=Wave H=Hot C=Cold \
T=Structure A=Accent X=Text Q=Seq V=Div O=Phase B=Blackbody R=Ramp  \
(lowercase: level < 128, '.': Background)";

#[derive(Debug)]
struct Args {
    scene: String,
    size: Size,
    times: Vec<f64>,
    mode: GlyphMode,
    inks: bool,
    seed: u64,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        scene: String::new(),
        size: Size::new(80, 24),
        times: vec![0.0, 1.0, 5.0, 20.0],
        mode: GlyphMode::Unicode,
        inks: false,
        seed: 42,
    };
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--size" => args.size = common::size(&value()?)?,
            "--t" => args.times = common::times(&value()?)?,
            "--seed" => {
                let v = value()?;
                args.seed = v.parse().map_err(|_| format!("bad --seed {v}"))?;
            }
            "--ascii" => args.mode = GlyphMode::Ascii,
            "--inks" => args.inks = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("unknown flag {s}")),
            s if args.scene.is_empty() => args.scene = s.to_owned(),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    if args.scene.is_empty() {
        return Err("missing <scene>".to_owned());
    }
    Ok(args)
}

/// One letter per role; see `LEGEND`.
fn role_char(ink: Ink) -> char {
    let c = match ink.role {
        Role::Background => return '.',
        Role::Dim => 'D',
        Role::Star => 'S',
        Role::Sun => 'U',
        Role::Planet => 'P',
        Role::Particle => 'I',
        Role::Wave => 'W',
        Role::Hot => 'H',
        Role::Cold => 'C',
        Role::Structure => 'T',
        Role::Accent => 'A',
        Role::Text => 'X',
        Role::Seq => 'Q',
        Role::Div => 'V',
        Role::Phase => 'O',
        Role::Blackbody => 'B',
        Role::Ramp(_) => 'R',
        _ => '?',
    };
    if ink.level < 128 {
        c.to_ascii_lowercase()
    } else {
        c
    }
}

fn print_roles(title: &str, canvas: &Canvas, ink: impl Fn(&Cell) -> Ink) {
    println!("-- {title} roles");
    for row in canvas.cells().chunks(usize::from(canvas.size().width())) {
        println!(
            "{}",
            row.iter().map(|c| role_char(ink(c))).collect::<String>()
        );
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("frames: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let entry = match common::scene(&args.scene) {
        Ok(entry) => entry,
        Err(e) => {
            eprintln!("frames: {e}");
            return ExitCode::from(2);
        }
    };

    let (w, h) = (args.size.width(), args.size.height());
    let mut stage = Stage::new((entry.build)(), args.seed, args.size, args.mode);
    if args.inks {
        println!("{LEGEND}");
    }
    let mut steps: u64 = 0;
    for &t in &args.times {
        common::step_to(&mut stage, &mut steps, t);
        println!(
            "== {} t={t}s {w}x{h} seed={} ({steps} steps)",
            entry.meta.name, args.seed
        );
        let canvas = stage.render();
        for line in canvas.to_lines() {
            println!("{line}");
        }
        if args.inks {
            print_roles("fg", canvas, |c| c.ink);
            print_roles("bg", canvas, |c| c.bg);
        }
    }
    ExitCode::SUCCESS
}
