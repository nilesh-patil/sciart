//! Ink-exact frame dump: the reference format of the M11 fidelity fixture (GEN-R22).
//!
//! ```text
//! cargo run -q -p sciart-generators --example inkframes -- [<scene> ...] > frames.txt
//! ```
//!
//! For every registered scene (or the named ones), seed 42, a `Stage` at 80×24, in
//! Unicode and in ASCII mode, stepped at the fixed `SIM_DT` to t = 0, 1 and 5 s, each
//! frame is printed as:
//!
//! ```text
//! == <scene> <u|a> t<secs>
//! <24 glyph rows>
//! <24 foreground ink rows>
//! <24 background ink rows, or the single line `nobg` when no cell has a background>
//! ```
//!
//! An ink row is a run-length list of space-separated tokens `<role><level>[*<n>]`: one
//! role letter (see `role_char`), the level as two hex digits, and a repeat count when a
//! run is longer than one cell.
//!
//! The file is self-contained and uses only API that existed at M11 (the M11 merge), so
//! the same example builds in that tree and writes the reference (the fixture) there. Roles
//! added after M11 all map to `?`: they never occur in the reference, so any such cell is
//! a mismatch. `tests/m11_fidelity.rs` includes this file to decode the fixture and
//! encode live frames with the same code.

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::ExitCode;

use sciart_core::prelude::*;

pub const SEED: u64 = 42;
pub const WIDTH: u16 = 80;
pub const HEIGHT: u16 = 24;
pub const TIMES: [u32; 3] = [0, 1, 5];
pub const MODES: [(GlyphMode, char); 2] = [(GlyphMode::Unicode, 'u'), (GlyphMode::Ascii, 'a')];

/// A stable one-letter code per role: the M11 roles only (not `Debug`, which differs
/// once a role carries data). Uppercase letters, `.` for Background.
#[allow(unreachable_patterns)] // In the M11 tree the wildcard is unreachable.
pub fn role_char(role: Role) -> char {
    match role {
        Role::Background => '.',
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
        _ => '?',
    }
}

/// One cell as the fixture sees it: glyph, then foreground and background as
/// (role letter, level).
pub type CellKey = (char, (char, u8), (char, u8));

pub fn cell_key(cell: &Cell) -> CellKey {
    (
        cell.ch,
        (role_char(cell.ink.role), cell.ink.level),
        (role_char(cell.bg.role), cell.bg.level),
    )
}

/// The frame's cells in row-major order.
pub fn frame_keys(canvas: &Canvas) -> Vec<CellKey> {
    canvas.cells().iter().map(cell_key).collect()
}

fn ink_row(out: &mut String, row: &[CellKey], pick: impl Fn(&CellKey) -> (char, u8)) {
    let mut i = 0;
    let mut first = true;
    while i < row.len() {
        let ink = pick(&row[i]);
        let mut n = 1;
        while i + n < row.len() && pick(&row[i + n]) == ink {
            n += 1;
        }
        if !first {
            out.push(' ');
        }
        first = false;
        let _ = write!(out, "{}{:02x}", ink.0, ink.1);
        if n > 1 {
            let _ = write!(out, "*{n}");
        }
        i += n;
    }
    out.push('\n');
}

/// Append one frame block (header line included) to `out`.
pub fn encode(out: &mut String, header: &str, width: usize, keys: &[CellKey]) {
    out.push_str(header);
    out.push('\n');
    for row in keys.chunks(width) {
        out.extend(row.iter().map(|k| k.0));
        out.push('\n');
    }
    for row in keys.chunks(width) {
        ink_row(out, row, |k| k.1);
    }
    if keys.iter().all(|k| k.2 == ('.', 0)) {
        out.push_str("nobg\n");
    } else {
        for row in keys.chunks(width) {
            ink_row(out, row, |k| k.2);
        }
    }
}

pub fn header(scene: &str, mode: char, t: u32) -> String {
    format!("== {scene} {mode} t{t}")
}

fn parse_ink_row(line: &str, out: &mut Vec<(char, u8)>) -> Result<(), String> {
    for tok in line.split(' ').filter(|t| !t.is_empty()) {
        let (ink, n) = match tok.split_once('*') {
            Some((ink, n)) => (ink, n.parse::<usize>().map_err(|_| tok.to_owned())?),
            None => (tok, 1),
        };
        let mut chars = ink.chars();
        let role = chars.next().ok_or_else(|| tok.to_owned())?;
        let level = u8::from_str_radix(chars.as_str(), 16).map_err(|_| tok.to_owned())?;
        out.extend(std::iter::repeat_n((role, level), n));
    }
    Ok(())
}

/// Decode a fixture: every frame block as (header, cells).
pub fn decode(
    text: &str,
    width: usize,
    height: usize,
) -> Result<Vec<(String, Vec<CellKey>)>, String> {
    let mut lines = text.lines().peekable();
    let mut frames = Vec::new();
    while let Some(line) = lines.next() {
        if !line.starts_with("== ") {
            continue;
        }
        let head = line.to_owned();
        let mut take = |what: &str| {
            lines
                .next()
                .ok_or_else(|| format!("{head}: truncated {what}"))
        };
        let mut glyphs = Vec::with_capacity(width * height);
        for _ in 0..height {
            glyphs.extend(take("glyphs")?.chars());
        }
        let mut fg = Vec::with_capacity(width * height);
        for _ in 0..height {
            parse_ink_row(take("fg")?, &mut fg).map_err(|t| format!("{head}: bad token {t}"))?;
        }
        let mut bg = Vec::with_capacity(width * height);
        let first = take("bg")?;
        if first == "nobg" {
            bg.resize(width * height, ('.', 0));
        } else {
            parse_ink_row(first, &mut bg).map_err(|t| format!("{head}: bad token {t}"))?;
            for _ in 1..height {
                parse_ink_row(take("bg")?, &mut bg)
                    .map_err(|t| format!("{head}: bad token {t}"))?;
            }
        }
        let n = width * height;
        if glyphs.len() != n || fg.len() != n || bg.len() != n {
            return Err(format!(
                "{head}: {} glyphs, {} fg, {} bg; want {n} each",
                glyphs.len(),
                fg.len(),
                bg.len()
            ));
        }
        let cells = glyphs
            .into_iter()
            .zip(fg)
            .zip(bg)
            .map(|((c, f), b)| (c, f, b))
            .collect();
        frames.push((head, cells));
    }
    Ok(frames)
}

/// Render `build`'s scene in `mode` at each of `TIMES` and hand the stage to `each`
/// (its `frame()` is the new frame, `title()` the live title).
pub fn render_frames(
    build: fn() -> Box<dyn Generator>,
    mode: GlyphMode,
    mut each: impl FnMut(u32, &Stage),
) {
    let mut stage = Stage::new(build(), SEED, Size::new(WIDTH, HEIGHT), mode);
    let mut steps: u64 = 0;
    for t in TIMES {
        let target = (f64::from(t) / SIM_DT).round() as u64;
        while steps < target {
            stage.step(SIM_DT);
            steps += 1;
        }
        stage.render();
        each(t, &stage);
    }
}

#[allow(dead_code)] // Unused when `tests/m11_fidelity.rs` includes this file.
fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let mut entries = Vec::new();
    if names.is_empty() {
        entries.extend(sciart_generators::registry().iter());
    } else {
        for name in &names {
            match sciart_generators::find(name) {
                Some(e) => entries.push(e),
                None => {
                    eprintln!("inkframes: unknown scene {name:?}");
                    return ExitCode::from(2);
                }
            }
        }
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# inkframes seed={SEED} size={WIDTH}x{HEIGHT} t=0,1,5 (see examples/inkframes.rs)"
    );
    for entry in entries {
        for (mode, m) in MODES {
            render_frames(entry.build, mode, |t, stage| {
                let keys = frame_keys(stage.frame());
                encode(
                    &mut out,
                    &header(entry.meta.name, m, t),
                    usize::from(WIDTH),
                    &keys,
                );
            });
        }
    }
    if std::io::stdout().write_all(out.as_bytes()).is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
