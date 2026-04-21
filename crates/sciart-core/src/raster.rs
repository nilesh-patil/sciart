//! Sub-cell rasters layered onto a [`Canvas`] (SPEC-001 CORE-R41, CORE-R42, CORE-R51,
//! CORE-R55, CORE-R64, CORE-R65, ADR-0010).
//!
//! - [`Dots`]: a braille lattice of 2 × 4 dots per cell, one ink per cell. With cells
//!   ~2:1 tall ([`ASPECT`](crate::geom::ASPECT)) the dots are *square*, so line art,
//!   particles and random walks are isotropic at 8× the cell resolution.
//! - [`Pixels`]: two square pixels per cell drawn as half blocks, with the lower pixel
//!   in the cell's background colour: 2× the vertical resolution with real colour, for
//!   fields.
//! - [`Density`]: an `f32` long-exposure buffer at `Dots` resolution, thresholded onto
//!   a `Dots` lattice.
//!
//! All are plain row-major buffers sized in `resize` and allocation-free afterwards
//! (GEN-R11). A scene draws into one, then `blit`s it onto the canvas; a cell holds a
//! single glyph, so one region uses one raster.

use crate::{Canvas, Glyph, GlyphMode, Ink, Role, Size, geom};

/// Bit of dot `(dx, dy)` within its braille cell, indexed `[dx][dy]`: standard Unicode
/// dot numbering 1-2-3-7 down the left column and 4-5-6-8 down the right, dot `n` being
/// bit `n − 1` (CORE-R41).
const BRAILLE_BIT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// The braille bit of lattice dot `(x, y)` within its cell (CORE-R41).
///
/// With [`braille`], this lets a scene draw a lone dot with [`Canvas::put`] when it must
/// *replace* a cell rather than merge into it (as a [`Dots`] layer would).
#[inline]
pub fn dot_bit(x: i32, y: i32) -> u8 {
    BRAILLE_BIT[x.rem_euclid(Dots::CELL_W) as usize][y.rem_euclid(Dots::CELL_H) as usize]
}

/// The braille glyph `U+2800 + mask`, with an ASCII fallback graded by dot count:
/// 1 ⇒ `.`, 2 ⇒ `:`, 3–4 ⇒ `+`, ≥ 5 ⇒ `#` (and 0 ⇒ ` `) (CORE-R41, GEN-R10).
#[inline]
pub fn braille(mask: u8) -> Glyph {
    let ascii = match mask.count_ones() {
        0 => ' ',
        1 => '.',
        2 => ':',
        3 | 4 => '+',
        _ => '#',
    };
    // U+2800..=U+28FF are all valid chars; the fallback is unreachable.
    let ch = char::from_u32(0x2800 + u32::from(mask)).unwrap_or(ascii);
    Glyph::new(ch, ascii)
}

/// How strongly an ink stands out, for picking a cell's one ink among its dots
/// (CORE-R41): its `level`, except for a signed [`Role::Div`] field, whose zero sits at
/// 128, so a deep negative lobe (low level) must beat a faint positive one.
#[inline]
fn intensity(ink: Ink) -> u8 {
    if ink.role == Role::Div {
        ink.level.abs_diff(128)
    } else {
        ink.level
    }
}

/// Walk the dots of the segment `(x0, y0)–(x1, y1)` on a `lattice`-sized dot grid
/// (CORE-R51), calling `f` for each; `f` must ignore off-lattice dots.
///
/// Endpoints are rounded and joined by [`geom::line`]. When both lie within a margin of
/// `w + h` around the lattice the walk is the exact unclipped Bresenham segment (the
/// off-lattice part costs at most `3·(w + h)` steps). Farther endpoints are first
/// clipped to that margin box (Liang–Barsky), so the walk length is bounded whatever
/// the input; that only shifts the Bresenham phase of lines that are already wild.
fn segment(x0: f64, y0: f64, x1: f64, y1: f64, lattice: Size, mut f: impl FnMut(i32, i32)) {
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return;
    }
    let (w, h) = (i32::from(lattice.width()), i32::from(lattice.height()));
    let m = f64::from(w + h);
    let (lo_x, hi_x) = (-m, f64::from(w) + m);
    let (lo_y, hi_y) = (-m, f64::from(h) + m);
    let inside = |x: f64, y: f64| (lo_x..=hi_x).contains(&x) && (lo_y..=hi_y).contains(&y);

    let (mut a, mut b) = ((x0.round(), y0.round()), (x1.round(), y1.round()));
    if !(inside(a.0, a.1) && inside(b.0, b.1)) {
        // Parametrise p(t) = p0 + t·(p1 − p0)/2 for t ∈ [0, 2]: the halved difference
        // cannot overflow even for endpoints at ±f64::MAX.
        let (dx, dy) = (x1 * 0.5 - x0 * 0.5, y1 * 0.5 - y0 * 0.5);
        let (mut t0, mut t1) = (0.0f64, 2.0f64);
        for (p, q) in [
            (-dx, x0 - lo_x),
            (dx, hi_x - x0),
            (-dy, y0 - lo_y),
            (dy, hi_y - y0),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return; // parallel to this edge and outside it
                }
            } else if p < 0.0 {
                t0 = t0.max(q / p);
            } else {
                t1 = t1.min(q / p);
            }
        }
        if t0 > t1 {
            return;
        }
        // Clamp away the last ulps of round-off so the casts below stay in range.
        let at = |t: f64| {
            (
                (x0 + t * dx).round().clamp(lo_x, hi_x),
                (y0 + t * dy).round().clamp(lo_y, hi_y),
            )
        };
        (a, b) = (at(t0), at(t1));
    }
    // Both endpoints are within the margin box, so the walk has at most
    // max(w, h) + 2m + 1 ≤ 4·(w + h) points; the `take` only guards the arithmetic.
    let cap = 4 * (w as usize + h as usize);
    for (x, y) in geom::line(a.0 as i32, a.1 as i32, b.0 as i32, b.1 as i32).take(cap) {
        f(x, y);
    }
}

/// `n` cells times `k` sub-cells per cell, saturated to a `u16` axis.
fn sub_axis(n: u16, k: u32) -> u16 {
    u16::try_from(u32::from(n) * k).unwrap_or(u16::MAX)
}

/// A braille dot lattice: `2·w × 4·h` square dots over a `w × h` cell region (CORE-R41).
///
/// Each cell stores its dot mask and one ink, that of its most intense lit dot (highest
/// `level`, or `|level − 128|` for [`Role::Div`]; ties go to the dot set last), since a
/// terminal cell has one colour.
/// Masks and inks are separate arrays so `blit` scans a dense byte array, which is
/// mostly zeros for sparse line art.
#[derive(Debug, Clone)]
pub struct Dots {
    cells: Size,
    masks: Vec<u8>,
    inks: Vec<Ink>,
}

impl Default for Dots {
    /// A 1×1-cell lattice, the placeholder until a scene's first `resize`.
    fn default() -> Self {
        Self::new(Size::new(1, 1))
    }
}

impl Dots {
    /// Dots per cell horizontally.
    pub const CELL_W: i32 = 2;
    /// Dots per cell vertically; `CELL_H / CELL_W = ASPECT` makes the dots square.
    pub const CELL_H: i32 = 4;

    /// An empty lattice over `cells` terminal cells.
    pub fn new(cells: Size) -> Self {
        Self {
            cells,
            masks: vec![0; cells.area()],
            inks: vec![Ink::BLANK; cells.area()],
        }
    }

    /// Dot coordinates of the continuous cell point `(x, y)`, where cell `i` spans
    /// `[i, i + 1)` (CORE-R64): dot `j` is centred at cell `(j + ½) / CELL_W` across and
    /// `(j + ½) / CELL_H` down, so [`plot`](Self::plot) lights the dot whose area holds
    /// the point, inside the cell that holds it. The one cell → dot convention for every
    /// scene; a cell *centre* `(cx + ½, cy + ½)` maps to the middle of its 2×4 block.
    #[inline]
    pub fn from_cells(x: f64, y: f64) -> (f64, f64) {
        (
            x * f64::from(Self::CELL_W) - 0.5,
            y * f64::from(Self::CELL_H) - 0.5,
        )
    }

    /// Lattice dimensions in dots, `2·w × 4·h` (saturated at `u16::MAX` per axis).
    pub fn dot_size(&self) -> Size {
        Size::new(
            sub_axis(self.cells.width(), Self::CELL_W as u32),
            sub_axis(self.cells.height(), Self::CELL_H as u32),
        )
    }

    /// Resize to `cells` and clear. Reuses the buffers unless they must grow.
    pub fn resize(&mut self, cells: Size) {
        self.cells = cells;
        self.masks.clear();
        self.masks.resize(cells.area(), 0);
        self.inks.clear();
        self.inks.resize(cells.area(), Ink::BLANK);
    }

    /// Unlight every dot.
    pub fn clear(&mut self) {
        // A cell's ink is only read when its mask is non-zero, and `set` overwrites
        // it for the first dot of a cell, so the inks need no reset.
        self.masks.fill(0);
    }

    /// Cell index and bit of dot `(x, y)`, or `None` off the lattice.
    #[inline]
    fn locate(&self, x: i32, y: i32) -> Option<(usize, u8)> {
        let w = i32::from(self.cells.width());
        let h = i32::from(self.cells.height());
        // `w · 2` and `h · 4` fit easily in i32 for u16 dimensions.
        let in_x = (0..w * Self::CELL_W).contains(&x);
        let in_y = (0..h * Self::CELL_H).contains(&y);
        (in_x && in_y).then(|| {
            let (cx, cy) = ((x / Self::CELL_W) as usize, (y / Self::CELL_H) as usize);
            (cy * w as usize + cx, dot_bit(x, y))
        })
    }

    /// Light dot `(x, y)` with `ink`; off-lattice dots are ignored.
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, ink: Ink) {
        let Some((i, bit)) = self.locate(x, y) else {
            return;
        };
        if self.masks[i] == 0 || intensity(ink) >= intensity(self.inks[i]) {
            self.inks[i] = ink;
        }
        self.masks[i] |= bit;
    }

    /// Light the dot nearest to `(x, y)` (dot units); non-finite or off-lattice points
    /// are ignored, so simulation code never casts NaN to a coordinate.
    #[inline]
    pub fn plot(&mut self, x: f64, y: f64, ink: Ink) {
        if x.is_finite() && y.is_finite() {
            // Saturating casts: anything huge lands off the lattice.
            self.set(x.round() as i32, y.round() as i32, ink);
        }
    }

    /// Light the Bresenham segment from `(x0, y0)` to `(x1, y1)` (dot units, rounded),
    /// both endpoints inclusive, clipped to the lattice (CORE-R51). Non-finite endpoints
    /// draw nothing, and the walk is capped at `4·(dot width + dot height)` steps, so
    /// wild coordinates stay cheap.
    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, ink: Ink) {
        let size = self.dot_size();
        segment(x0, y0, x1, y1, size, |x, y| self.set(x, y, ink));
    }

    /// Whether dot `(x, y)` is lit; `false` off the lattice.
    pub fn is_lit(&self, x: i32, y: i32) -> bool {
        self.locate(x, y)
            .is_some_and(|(i, bit)| self.masks[i] & bit != 0)
    }

    /// The dot mask of cell `(cx, cy)` (bit layout as [`braille`]; 0 = no dot lit), or
    /// `None` off the region (CORE-R64).
    #[inline]
    pub fn cell_mask(&self, cx: i32, cy: i32) -> Option<u8> {
        let (w, h) = (
            i32::from(self.cells.width()),
            i32::from(self.cells.height()),
        );
        ((0..w).contains(&cx) && (0..h).contains(&cy))
            .then(|| self.masks[cy as usize * w as usize + cx as usize])
    }

    /// Write every cell with at least one lit dot as its braille glyph (or the
    /// dot-count ASCII fallback) and ink; cells with no lit dots are left untouched, so
    /// a lattice layers over earlier drawing.
    pub fn blit(&self, canvas: &mut Canvas) {
        // Line art and particles light a small fraction of cells, so skip empty runs
        // a block at a time (one wide compare) instead of branching per cell.
        const BLOCK: usize = 16;
        let mut blocks = self.masks.chunks_exact(BLOCK);
        let mut start = 0;
        for block in &mut blocks {
            if block != [0; BLOCK] {
                self.blit_run(canvas, start, block);
            }
            start += BLOCK;
        }
        self.blit_run(canvas, start, blocks.remainder());
    }

    /// Blit the cells `start..start + masks.len()` (row-major indices).
    fn blit_run(&self, canvas: &mut Canvas, start: usize, masks: &[u8]) {
        let w = usize::from(self.cells.width());
        for (i, &mask) in (start..).zip(masks) {
            if mask != 0 {
                canvas.put((i % w) as i32, (i / w) as i32, braille(mask), self.inks[i]);
            }
        }
    }
}

/// A long-exposure `f32` buffer at [`Dots`] resolution (CORE-R55): particles and
/// strokes [`deposit`](Self::deposit) weight into it, [`decay`](Self::decay) fades it,
/// and [`to_dots`](Self::to_dots) thresholds it onto a lattice. For trails, attractor
/// histograms and aurora curtains, where what matters is where things *have been*.
#[derive(Debug, Clone)]
pub struct Density {
    /// Lattice size in dots.
    dots: Size,
    /// Row-major, `dots.w × dots.h`.
    buf: Vec<f32>,
}

impl Default for Density {
    /// A 1×1-cell buffer, the placeholder until a scene's first `resize`.
    fn default() -> Self {
        Self::new(Size::new(1, 1))
    }
}

impl Density {
    /// A zeroed buffer over `cells` terminal cells (`2·w × 4·h` dots).
    pub fn new(cells: Size) -> Self {
        let mut d = Self {
            dots: Size::new(1, 1),
            buf: Vec::new(),
        };
        d.resize(cells);
        d
    }

    /// Buffer dimensions in dots, as [`Dots::dot_size`].
    pub fn dot_size(&self) -> Size {
        self.dots
    }

    /// Resize to `cells` and zero. Reuses the buffer unless it must grow.
    pub fn resize(&mut self, cells: Size) {
        self.dots = Size::new(
            sub_axis(cells.width(), Dots::CELL_W as u32),
            sub_axis(cells.height(), Dots::CELL_H as u32),
        );
        self.buf.clear();
        self.buf.resize(self.dots.area(), 0.0);
    }

    /// Zero every dot.
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    /// Multiply every value by `k`, clamped to `[0, 1]` (NaN leaves the buffer as is):
    /// an exponential fade, `k = exp(−dt / τ)` per step.
    pub fn decay(&mut self, k: f32) {
        if k.is_nan() || k >= 1.0 {
            return;
        }
        let k = k.max(0.0);
        for v in &mut self.buf {
            *v *= k;
        }
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (w, h) = (i32::from(self.dots.width()), i32::from(self.dots.height()));
        ((0..w).contains(&x) && (0..h).contains(&y))
            .then(|| y as usize * usize::from(self.dots.width()) + x as usize)
    }

    #[inline]
    fn add(&mut self, x: i32, y: i32, w: f32) {
        if let Some(i) = self.index(x, y) {
            self.buf[i] += w;
        }
    }

    /// Add `w` at the dot nearest to `(x, y)` (dot units). Non-finite coordinates or
    /// weights and off-buffer dots are ignored.
    #[inline]
    pub fn deposit(&mut self, x: f64, y: f64, w: f32) {
        if x.is_finite() && y.is_finite() && w.is_finite() {
            // Saturating casts: anything huge lands off the buffer.
            self.add(x.round() as i32, y.round() as i32, w);
        }
    }

    /// The value at dot `(x, y)`, or `None` off the buffer.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<f32> {
        self.index(x, y).map(|i| self.buf[i])
    }

    /// Light every dot whose value is `≥ threshold` on `dots` with `map(value)`, in one
    /// row-major pass. `dots` should cover the same cells; dots outside it are skipped,
    /// and a NaN threshold lights nothing.
    pub fn to_dots(&self, dots: &mut Dots, threshold: f32, mut map: impl FnMut(f32) -> Ink) {
        let w = usize::from(self.dots.width());
        for (y, row) in self.buf.chunks_exact(w).enumerate() {
            for (x, &v) in row.iter().enumerate() {
                if v >= threshold {
                    dots.set(x as i32, y as i32, map(v));
                }
            }
        }
    }
}

/// Upper half block: the top pixel is the glyph, the bottom one the cell background.
const UPPER: Glyph = Glyph::new('▀', '#');
const LOWER: Glyph = Glyph::new('▄', '#');

/// A half-block pixel buffer: `w × 2·h` square pixels over a `w × h` cell region
/// (CORE-R42). [`Ink::BLANK`] (any [`Role::Background`] ink) is an empty pixel.
#[derive(Debug, Clone)]
pub struct Pixels {
    cells: Size,
    /// Row-major, `2·h` rows of `w`: pixel rows `2·cy` and `2·cy + 1` form cell row `cy`.
    px: Vec<Ink>,
}

impl Default for Pixels {
    /// A 1×1-cell buffer, the placeholder until a scene's first `resize`.
    fn default() -> Self {
        Self::new(Size::new(1, 1))
    }
}

/// Empty pixels are left to the terminal background. Every `Background`-role ink counts,
/// because the renderer paints that role as "no colour" (RND-R3): a half block in it
/// would show up in the default foreground colour.
#[inline]
fn is_empty(ink: Ink) -> bool {
    ink.role == Role::Background
}

impl Pixels {
    /// An empty buffer over `cells` terminal cells.
    pub fn new(cells: Size) -> Self {
        Self {
            cells,
            px: vec![Ink::BLANK; 2 * cells.area()],
        }
    }

    /// Buffer dimensions in pixels, `w × 2·h` (saturated at `u16::MAX`).
    pub fn pixel_size(&self) -> Size {
        Size::new(self.cells.width(), sub_axis(self.cells.height(), 2))
    }

    /// Resize to `cells` and clear. Reuses the buffer unless it must grow.
    pub fn resize(&mut self, cells: Size) {
        self.cells = cells;
        self.px.clear();
        self.px.resize(2 * cells.area(), Ink::BLANK);
    }

    /// Set every pixel to `ink`.
    pub fn fill(&mut self, ink: Ink) {
        self.px.fill(ink);
    }

    /// Empty every pixel.
    pub fn clear(&mut self) {
        self.fill(Ink::BLANK);
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let w = i32::from(self.cells.width());
        let h = 2 * i32::from(self.cells.height());
        ((0..w).contains(&x) && (0..h).contains(&y)).then(|| y as usize * w as usize + x as usize)
    }

    /// Set pixel `(x, y)`; off-buffer pixels are ignored.
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, ink: Ink) {
        if let Some(i) = self.index(x, y) {
            self.px[i] = ink;
        }
    }

    /// Pixel `(x, y)`, or `None` off the buffer.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<Ink> {
        self.index(x, y).map(|i| self.px[i])
    }

    /// Set every pixel to `shader(x, y)`, row by row (CORE-R65): the `Pixels` analogue
    /// of [`draw::field`](crate::draw::field), for sampling a simulation lattice (e.g.
    /// through a [`SampleMap`](crate::SampleMap)) onto the display pixels.
    pub fn shade(&mut self, mut shader: impl FnMut(i32, i32) -> Ink) {
        let w = usize::from(self.cells.width());
        for (y, row) in self.px.chunks_exact_mut(w).enumerate() {
            for (x, px) in row.iter_mut().enumerate() {
                // Both fit: x < w ≤ u16::MAX, y < 2·h ≤ 2·u16::MAX.
                *px = shader(x as i32, y as i32);
            }
        }
    }

    /// Draw the buffer onto `canvas`, one cell per vertical pixel pair.
    ///
    /// Unicode mode: both pixels empty ⇒ cell untouched; top only ⇒ `▀` in the top ink;
    /// bottom only ⇒ `▄` in the bottom ink; both ⇒ `▀` with the top as foreground and
    /// the bottom as background. ASCII mode has no half blocks, so every cell goes
    /// through the scene's own `ascii(x, y, top, bottom)` fallback, whose `Some((ch,
    /// ink))` is written and `None` leaves the cell untouched (GEN-R10, ADR-0010).
    pub fn blit<F>(&self, canvas: &mut Canvas, mut ascii: F)
    where
        F: FnMut(i32, i32, Ink, Ink) -> Option<(char, Ink)>,
    {
        let w = usize::from(self.cells.width());
        let unicode = canvas.mode() == GlyphMode::Unicode;
        for (cy, pair) in self.px.chunks_exact(2 * w).enumerate() {
            let (tops, bottoms) = pair.split_at(w);
            for (cx, (&top, &bottom)) in tops.iter().zip(bottoms).enumerate() {
                let (x, y) = (cx as i32, cy as i32);
                if !unicode {
                    if let Some((ch, ink)) = ascii(x, y, top, bottom) {
                        canvas.put(x, y, Glyph::ascii(ch), ink);
                    }
                    continue;
                }
                match (is_empty(top), is_empty(bottom)) {
                    (true, true) => {}
                    (false, true) => canvas.put(x, y, UPPER, top),
                    (true, false) => canvas.put(x, y, LOWER, bottom),
                    (false, false) => canvas.put_bg(x, y, UPPER, top, bottom),
                }
            }
        }
    }
}
