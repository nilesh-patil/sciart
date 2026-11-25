//! The preview renderer: a [`Canvas`] painted as a terminal would, at 8 × 16 px per cell.

use std::path::Path;

use font8x8::UnicodeFonts as _;
use font8x8::{BASIC_FONTS, BLOCK_FONTS, BOX_FONTS, GREEK_FONTS, LATIN_FONTS, MISC_FONTS};
use sciart_core::prelude::*;
use sciart_render::{Rgb, Theme};

use crate::glyphs::builtin;

/// Pixels per cell: a typical terminal cell is about twice as tall as it is wide.
pub const CELL_W: usize = 8;
pub const CELL_H: usize = 16;
/// The terminal's default colours, used where an ink is `Role::Background` (RND-R3, RND-R7).
pub const TERM_BG: Rgb = Rgb(12, 12, 18);
pub const TERM_FG: Rgb = Rgb(204, 204, 204);

/// An RGB8 image, row-major.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgb.len())
            .finish()
    }
}

impl Image {
    /// A `width` × `height` image filled with `fill`.
    pub fn new(width: usize, height: usize, fill: Rgb) -> Self {
        let mut img = Self {
            width,
            height,
            rgb: vec![0; 3 * width * height],
        };
        img.rect(0, 0, width, height, fill);
        img
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Row-major RGB triples.
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// Fill `w` × `h` pixels from `(x0, y0)`, clipped to the image.
    pub fn rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, Rgb(r, g, b): Rgb) {
        for y in y0..(y0 + h).min(self.height) {
            let row = &mut self.rgb[3 * (y * self.width + x0)..3 * (y * self.width + self.width)];
            for px in row.chunks_exact_mut(3).take(w) {
                px.copy_from_slice(&[r, g, b]);
            }
        }
    }

    /// Paste `other` with its top-left corner at row `y0` (used by the example's `--sheet`).
    pub fn paste(&mut self, other: &Image, y0: usize) {
        let n = 3 * other.width.min(self.width);
        for (y, row) in other.rgb.chunks_exact(3 * other.width).enumerate() {
            let start = 3 * (y0 + y) * self.width;
            if let Some(dst) = self.rgb.get_mut(start..start + n) {
                dst.copy_from_slice(&row[..n]);
            }
        }
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
            if let Some(rows) = builtin(ch) {
                // CLI-R18: a full-resolution 8 × 16 bitmap for a glyph `font8x8` lacks.
                for (r, bits) in rows.iter().enumerate() {
                    for c in 0..CELL_W {
                        if bits & (1 << c) != 0 {
                            img.rect(x0 + c, y0 + r, 1, 1, fg);
                        }
                    }
                }
                return;
            }
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

/// CLI-R18: whether the capture draws `ch` as itself (braille, half blocks, `font8x8`, or a
/// built-in bitmap) rather than through the scene's ASCII fallback.
pub fn draws_natively(ch: char) -> bool {
    matches!(ch, ' ' | '▀' | '▄' | '█' | '\u{2801}'..='\u{28FF}')
        || builtin(ch).is_some()
        || bitmap(ch).is_some()
}

/// Paint `canvas` as a terminal would with `theme`; `fallback` is the same frame in ASCII.
pub fn paint(canvas: &Canvas, fallback: &Canvas, theme: Theme) -> Image {
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

/// Write `image` to `path` as an 8-bit RGB PNG, creating the parent directory if needed.
pub fn write_png(image: &Image, path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(path)?;
    encode_png(image, std::io::BufWriter::new(file))
}

/// Encode `image` as an 8-bit RGB PNG into `out` (the library's `Stage::render_png`, LIB-R5).
pub fn encode_png<W: std::io::Write>(image: &Image, out: W) -> std::io::Result<()> {
    let dims = |n: usize| {
        u32::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "image dimension too large",
            )
        })
    };
    let mut enc = png::Encoder::new(out, dims(image.width)?, dims(image.height)?);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(&image.rgb)
        .map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)
}
