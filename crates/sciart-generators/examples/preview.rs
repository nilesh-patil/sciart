//! Truecolor review tool: renders a scene's frames through the real `Theme` into PNGs, so
//! art reviewers can see colour without a terminal.
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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use font8x8::UnicodeFonts as _;
use font8x8::{BASIC_FONTS, BLOCK_FONTS, BOX_FONTS, GREEK_FONTS, LATIN_FONTS, MISC_FONTS};
use sciart_core::prelude::*;
use sciart_render::{Rgb, Theme};

const USAGE: &str = "usage: preview <scene> [--size WxH] [--t 0,5,20] [--theme astro|rainbow|mono] \
[--seed N] [--ascii] [--out PREFIX] [--sheet]\n       preview --swatches [--out PREFIX]";

/// Pixels per cell: a typical terminal cell is about twice as tall as it is wide.
const CELL_W: usize = 8;
const CELL_H: usize = 16;
/// The terminal's default colours, used where an ink is `Role::Background` (RND-R3, RND-R7).
const TERM_BG: Rgb = Rgb(12, 12, 18);
const TERM_FG: Rgb = Rgb(204, 204, 204);
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

/// An RGB8 image.
struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Image {
    fn new(width: usize, height: usize, fill: Rgb) -> Self {
        let mut img = Self {
            width,
            height,
            rgb: vec![0; 3 * width * height],
        };
        img.rect(0, 0, width, height, fill);
        img
    }

    fn rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, Rgb(r, g, b): Rgb) {
        for y in y0..(y0 + h).min(self.height) {
            let row = &mut self.rgb[3 * (y * self.width + x0)..3 * (y * self.width + self.width)];
            for px in row.chunks_exact_mut(3).take(w) {
                px.copy_from_slice(&[r, g, b]);
            }
        }
    }

    /// Paste `other` with its top-left corner at row `y0`.
    fn paste(&mut self, other: &Image, y0: usize) {
        let n = 3 * other.width.min(self.width);
        for (y, row) in other.rgb.chunks_exact(3 * other.width).enumerate() {
            let start = 3 * (y0 + y) * self.width;
            if let Some(dst) = self.rgb.get_mut(start..start + n) {
                dst.copy_from_slice(&row[..n]);
            }
        }
    }

    fn write_png(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let dims = |n: usize| u32::try_from(n).map_err(|_| "image too large".to_owned());
        let mut enc = png::Encoder::new(
            std::io::BufWriter::new(file),
            dims(self.width)?,
            dims(self.height)?,
        );
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&self.rgb)
            .map_err(|e| e.to_string())
    }
}

/// An 8 × 8 bitmap for `ch` (row-major, bit 0 = leftmost pixel), if the font has one.
fn bitmap(ch: char) -> Option<[u8; 8]> {
    BASIC_FONTS
        .get(ch)
        .or_else(|| LATIN_FONTS.get(ch))
        .or_else(|| BOX_FONTS.get(ch))
        .or_else(|| BLOCK_FONTS.get(ch))
        .or_else(|| GREEK_FONTS.get(ch))
        .or_else(|| MISC_FONTS.get(ch))
}

/// Draw one cell's glyph in `fg` at pixel `(x0, y0)` over its already-filled background.
/// `fallback` is the ASCII glyph the scene gives for the same cell.
fn glyph(img: &mut Image, x0: usize, y0: usize, ch: char, fallback: char, fg: Rgb) {
    const HALF: usize = CELL_H / 2;
    match ch {
        ' ' => {}
        '▀' => img.rect(x0, y0, CELL_W, HALF, fg),
        '▄' => img.rect(x0, y0 + HALF, CELL_W, HALF, fg),
        '█' => img.rect(x0, y0, CELL_W, CELL_H, fg),
        '\u{2801}'..='\u{28FF}' => {
            // Dot n of the braille pattern is bit n − 1: 1-2-3-7 down the left column,
            // 4-5-6-8 down the right (CORE-R41). A 2 × 2 px dot centred in each 4 × 4 slot.
            const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
            let mask = (u32::from(ch) - 0x2800) as u8;
            for (dx, column) in BITS.iter().enumerate() {
                for (dy, &bit) in column.iter().enumerate() {
                    if mask & bit != 0 {
                        img.rect(x0 + 4 * dx + 1, y0 + 4 * dy + 1, 2, 2, fg);
                    }
                }
            }
        }
        _ => {
            let Some(rows) = bitmap(ch).or_else(|| bitmap(fallback)) else {
                // Neither glyph is in the font: a small centred block marks the cell.
                img.rect(x0 + 2, y0 + 5, 4, 6, fg);
                return;
            };
            for (r, bits) in rows.iter().enumerate() {
                for c in 0..CELL_W {
                    if bits & (1 << c) != 0 {
                        img.rect(x0 + c, y0 + 2 * r, 1, 2, fg);
                    }
                }
            }
        }
    }
}

/// Paint `canvas` as a terminal would with `theme`; `fallback` is the same frame in ASCII.
fn paint(canvas: &Canvas, fallback: &Canvas, theme: Theme) -> Image {
    let size = canvas.size();
    let w = usize::from(size.width());
    let mut img = Image::new(w * CELL_W, usize::from(size.height()) * CELL_H, TERM_BG);
    for (i, (cell, ascii)) in canvas.cells().iter().zip(fallback.cells()).enumerate() {
        let (cx, cy) = (i % w, i / w);
        // Both fit in u16: the canvas is at most u16::MAX cells a side.
        let colour = |ink: Ink, default: Rgb| {
            if ink.role == Role::Background {
                default
            } else {
                theme.color(ink, cx as u16, cy as u16)
            }
        };
        let (x0, y0) = (cx * CELL_W, cy * CELL_H);
        img.rect(x0, y0, CELL_W, CELL_H, colour(cell.bg, TERM_BG));
        glyph(
            &mut img,
            x0,
            y0,
            cell.ch,
            ascii.ch,
            colour(cell.ink, TERM_FG),
        );
    }
    img
}

/// Stack `frames` vertically with a gap between them.
fn sheet(frames: &[Image]) -> Image {
    let width = frames.iter().map(|f| f.width).max().unwrap_or(1);
    let height = frames.iter().map(|f| f.height + GAP).sum::<usize>() - GAP;
    let mut img = Image::new(width, height.max(1), GAP_COLOUR);
    let mut y = 0;
    for f in frames {
        img.paste(f, y);
        y += f.height + GAP;
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
        swatches().write_png(&path)?;
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
    let mut stage = Stage::new((entry.build)(), args.seed, args.size, mode);
    let mut steps = 0;
    let mut frames = Vec::new();
    for &t in &args.times {
        common::step_to(&mut stage, &mut steps, t);
        let canvas = stage.render().clone();
        // Rendering does not advance the scene, so the ASCII frame is the same moment.
        stage.set_glyph_mode(GlyphMode::Ascii);
        let img = paint(&canvas, stage.render(), args.theme);
        stage.set_glyph_mode(mode);
        if args.sheet {
            frames.push(img);
        } else {
            let path = PathBuf::from(format!("{}-t{t}.png", out.display()));
            img.write_png(&path)?;
            println!("{}", path.display());
        }
    }
    if args.sheet {
        let path = PathBuf::from(format!("{}.png", out.display()));
        sheet(&frames).write_png(&path)?;
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
