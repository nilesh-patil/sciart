//! sciart's scenes in the browser (WEB-R3, ADR-0034).
//!
//! A [`Player`] runs one scene and draws it with the capture renderer, at 8 × 16 px per
//! cell, as RGBA pixels the page puts on a `<canvas>`. It has no terminal, no clock and no
//! global RNG: the page passes the time of each animation frame to [`Player::advance`],
//! which turns it into fixed 1/60 s steps by the rule of LIB-R5. Natively, the tests prove
//! that a frame equals the capture renderer's pixels; on `wasm32` the `wasm` module exports
//! the player to JavaScript (`site/static/player.js` wraps it in a `Player` class).

use sciart_capture::{CELL_H, CELL_W, RenderOptions, render_frame};
use sciart_core::prelude::*;
use sciart_generators::{ALIASES, Entry, registry};
use sciart_render::Theme;

#[cfg(target_arch = "wasm32")]
mod wasm;

/// LIB-R5: a size is clamped to 1..=1024 cells in each dimension.
const MAX_CELLS: u32 = 1024;
/// LIB-R5: at most an hour of simulated time in one call.
const MAX_ADVANCE: f64 = 3600.0;
const STEPS_PER_SECOND: f64 = 60.0;

/// One scene, stepped by the page's clock and drawn as RGBA pixels.
pub struct Player {
    entry: &'static Entry,
    stage: Stage,
    columns: u32,
    rows: u32,
    /// Seconds passed to `advance` so far, and the fixed steps taken for them.
    total: f64,
    steps: u64,
    pixels: Vec<u8>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("scene", &self.entry.meta.name)
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .field("steps", &self.steps)
            .finish_non_exhaustive()
    }
}

fn clamp(cells: u32) -> u16 {
    // Both bounds fit a u16.
    cells.clamp(1, MAX_CELLS) as u16
}

impl Player {
    /// A player for `scene` (a registered name or a retired alias) at `columns` × `rows`
    /// cells, seeded with `seed`; `None` when no scene has that name.
    pub fn new(scene: &str, columns: u32, rows: u32, seed: u64) -> Option<Self> {
        let entry = sciart_generators::find(scene)?;
        let size = Size::new(clamp(columns), clamp(rows));
        let stage = Stage::new((entry.build)(), seed, size, GlyphMode::Unicode);
        Some(Self {
            entry,
            stage,
            columns: u32::from(size.width()),
            rows: u32::from(size.height()),
            total: 0.0,
            steps: 0,
            pixels: Vec::new(),
        })
    }

    /// Add `seconds` of simulated time and take the fixed steps they complete: after it,
    /// `floor(60 · total + 1e-6)` steps have been taken in all (LIB-R5). A negative or
    /// non-finite value, or more than an hour, steps nothing and returns `false`.
    pub fn advance(&mut self, seconds: f64) -> bool {
        if !(seconds.is_finite() && (0.0..=MAX_ADVANCE).contains(&seconds)) {
            return false;
        }
        self.total += seconds;
        let target = (self.total * STEPS_PER_SECOND + 1e-6).floor() as u64;
        while self.steps < target {
            self.stage.step(SIM_DT);
            self.steps += 1;
        }
        true
    }

    /// Resize to `columns` × `rows` cells (each clamped to 1..=1024); the scene keeps its
    /// time and rebuilds what depends on the size.
    pub fn resize(&mut self, columns: u32, rows: u32) {
        let size = Size::new(clamp(columns), clamp(rows));
        self.stage.resize(size);
        self.columns = u32::from(size.width());
        self.rows = u32::from(size.height());
    }

    /// The current frame in `theme`: [`width`](Self::width) × [`height`](Self::height)
    /// RGBA pixels, row-major, every pixel opaque. Drawing takes no step.
    pub fn frame(&mut self, theme: Theme) -> &[u8] {
        let options = RenderOptions {
            theme,
            ascii: false,
        };
        let image = render_frame(&mut self.stage, &options);
        self.pixels.clear();
        self.pixels.reserve(image.rgb().len() / 3 * 4);
        for px in image.rgb().chunks_exact(3) {
            self.pixels.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        &self.pixels
    }

    /// The frame's width in pixels.
    pub fn width(&self) -> u32 {
        self.columns * CELL_W as u32
    }

    /// The frame's height in pixels.
    pub fn height(&self) -> u32 {
        self.rows * CELL_H as u32
    }

    pub fn columns(&self) -> u32 {
        self.columns
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Fixed 1/60 s steps taken so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The scene's registered name (an alias resolves to it).
    pub fn scene(&self) -> &'static str {
        self.entry.meta.name
    }
}

/// The theme names, in the order `t` cycles them (RND-R2).
pub fn themes() -> &'static [&'static str] {
    const NAMES: [&str; Theme::ALL.len()] = {
        let mut names = [""; Theme::ALL.len()];
        let mut i = 0;
        while i < names.len() {
            names[i] = Theme::ALL[i].name();
            i += 1;
        }
        names
    };
    &NAMES
}

/// The scene catalogue as `sciart list --json` prints it (CLI-R16): `{"schema":1,
/// "scenes":[...]}` on one line, sorted by name.
pub fn scenes() -> String {
    let mut entries: Vec<&Entry> = registry().iter().collect();
    entries.sort_by_key(|e| e.meta.name);
    let mut out = String::from("{\"schema\":1,\"scenes\":[");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let meta = e.meta;
        for (key, value) in [
            ("{\"name\":", meta.name),
            (",\"category\":", meta.category.id()),
            (",\"category_label\":", meta.category.label()),
            (",\"title\":", meta.title),
            (",\"description\":", meta.description),
        ] {
            out.push_str(key);
            json_string(&mut out, value);
        }
        out.push_str(",\"captions\":");
        json_array(&mut out, meta.captions.iter().copied());
        out.push_str(",\"aliases\":");
        let aliases = ALIASES
            .iter()
            .filter(|(_, name)| *name == meta.name)
            .map(|(old, _)| *old);
        json_array(&mut out, aliases);
        out.push('}');
    }
    out.push_str("]}\n");
    out
}

fn json_array<'a>(out: &mut String, items: impl Iterator<Item = &'a str>) {
    out.push('[');
    for (i, item) in items.enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_string(out, item);
    }
    out.push(']');
}

/// A JSON string literal with the escaping `sciart list --json` uses.
fn json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}
