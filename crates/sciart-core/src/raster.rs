//! Sub-cell rasters layered onto a [`Canvas`] (SPEC-001 CORE-R41, CORE-R42, CORE-R51,
//! CORE-R55, CORE-R64, CORE-R65, CORE-R85, CORE-R86, CORE-R95, ADR-0010).
//!
//! - [`Dots`]: a braille lattice of 2 × 4 dots per cell, one ink per cell. With cells
//!   ~2:1 tall ([`ASPECT`](crate::geom::ASPECT)) the dots are *square*, so line art,
//!   particles and random walks are isotropic at 8× the cell resolution.
//! - [`Pixels`]: two square pixels per cell drawn as half blocks, with the lower pixel
//!   in the cell's background colour: 2× the vertical resolution with real colour, for
//!   fields.
//! - [`Density`]: an `f32` long-exposure buffer at `Dots` resolution, thresholded onto
//!   a `Dots` lattice.
//! - [`Glow`]: an `f32` long-exposure buffer at `Pixels` resolution, tone-mapped onto a
//!   `Pixels` layer.
//!
//! All are plain row-major buffers sized in `resize` and allocation-free afterwards
//! (GEN-R11). A scene draws into one, then `blit`s it onto the canvas; a cell holds a
//! single glyph, so one region uses one raster.

use crate::{Canvas, Glyph, GlyphMode, Ink, Role, Size, geom, ink};

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
/// 128, so a deep negative lobe (low level) must beat a faint positive one, and a
/// [`Role::Phase`] field, whose level is an angle: all its dots tie, so the last one set
/// wins (CORE-R78).
#[inline]
fn intensity(ink: Ink) -> u8 {
    match ink.role {
        Role::Div => ink.level.abs_diff(128),
        Role::Phase => 0,
        _ => ink.level,
    }
}

/// The more intense of two inks, `a` on a tie, by the measure a [`Dots`] cell uses to
/// pick its ink (CORE-R87): the one colour to keep when a cell's two pixels must become a
/// single glyph (a [`Pixels::blit`] ASCII fallback) or a single background
/// ([`Pixels::cell_bg`]). An empty ink loses to any drawn one.
#[inline]
pub fn stronger(a: Ink, b: Ink) -> Ink {
    match (is_empty(a), is_empty(b)) {
        (_, true) => a,
        (true, false) => b,
        (false, false) if intensity(b) > intensity(a) => b,
        (false, false) => a,
    }
}

/// The ASCII brightness ramp of [`ascii_ramp_cell`], dimmest first.
pub const ASCII_RAMP: [char; 9] = ['.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// A [`Pixels::blit`] ASCII fallback for half-block pictures of light (CORE-R91): the
/// [`stronger`] of the cell's two pixels, drawn as its level on [`ASCII_RAMP`]
/// (`level · 9 / 256`) in that pixel's ink; `None` (cell untouched) when both are empty.
/// Lifted out of black_hole, which gravitational_lensing imported it from (M12).
#[inline]
pub fn ascii_ramp_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = stronger(top, bottom);
    if is_empty(ink) {
        return None;
    }
    Some((
        ASCII_RAMP[usize::from(ink.level) * ASCII_RAMP.len() / 256],
        ink,
    ))
}

/// Share of the square pixel `(x, y)` (spanning `[x, x + 1) × [y, y + 1)`) inside the
/// circle of radius `r` about `c` (CORE-R95), estimated from the pixel centre's distance
/// `d` as `clamp(r + ½ − d, 0, 1)`: one `hypot` per pixel, exact for a straight edge
/// through the centre and symmetric about the circle's centre. NaN input ⇒ 0.
#[inline]
pub fn disc_cover(x: i32, y: i32, c: (f64, f64), r: f64) -> f64 {
    let d = (f64::from(x) + 0.5 - c.0).hypot(f64::from(y) + 0.5 - c.1);
    // `f64::min` would turn a NaN distance into full cover; compare first.
    let cover = r + 0.5 - d;
    if cover > 0.0 { cover.min(1.0) } else { 0.0 }
}

/// Call `f(x, y, cover)` for every pixel the disc of radius `r` about `c` covers
/// (`disc_cover > 0`, CORE-R95), row by row, left to right: the one anti-aliased disc
/// walk for half-block `Pixels` pictures of suns, moons and planets. Pixel coordinates
/// are unclipped; `f` ignores the ones off its buffer. Non-finite input or `r < 0` visits
/// nothing.
pub fn disc_pixels(c: (f64, f64), r: f64, mut f: impl FnMut(i32, i32, f64)) {
    if !(c.0.is_finite() && c.1.is_finite() && r.is_finite()) || r < 0.0 {
        return;
    }
    let reach = r + 0.5;
    let (x0, x1) = ((c.0 - reach).floor() as i32, (c.0 + reach).ceil() as i32);
    let (y0, y1) = ((c.1 - reach).floor() as i32, (c.1 + reach).ceil() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let cover = disc_cover(x, y, c, r);
            if cover > 0.0 {
                f(x, y, cover);
            }
        }
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

    /// Light the filled disc of radius `r` (dot units, so round on screen) about `(x, y)`:
    /// every dot whose centre lies within `r`, plus the dot nearest the centre, so a
    /// radius below ½ is one dot (CORE-R82). Clipped to the lattice; a non-finite centre
    /// or radius, or `r < 0`, lights nothing. Bobs, suns and grains in line-art scenes.
    pub fn disc(&mut self, x: f64, y: f64, r: f64, ink: Ink) {
        if !(x.is_finite() && y.is_finite() && r.is_finite() && r >= 0.0) {
            return;
        }
        let size = self.dot_size();
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        // Only the part of the bounding box on the lattice, so a huge r stays cheap.
        let (x0, x1) = ((x - r).ceil().max(0.0), (x + r).floor().min(w - 1.0));
        let (y0, y1) = ((y - r).ceil().max(0.0), (y + r).floor().min(h - 1.0));
        let mut dy = y0;
        while dy <= y1 {
            let mut dx = x0;
            while dx <= x1 {
                if (dx - x).hypot(dy - y) <= r {
                    self.set(dx as i32, dy as i32, ink);
                }
                dx += 1.0;
            }
            dy += 1.0;
        }
        self.plot(x, y, ink);
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
        self.blit_with(canvas, |_, _| Ink::BLANK);
    }

    /// [`blit`](Self::blit) over a half-block layer (CORE-R84): each written cell gets
    /// [`Pixels::cell_bg`] of `under` as its background, so a glow drawn as [`Pixels`]
    /// carries on under a point cloud or a track instead of stopping at the first dotted
    /// cell. `under` should cover the same cells; outside it the background is blank.
    /// ASCII mode has no half blocks, so there it is plain `blit`.
    pub fn blit_over(&self, canvas: &mut Canvas, under: &Pixels) {
        if canvas.mode() == GlyphMode::Ascii {
            self.blit(canvas);
        } else {
            self.blit_with(canvas, |x, y| under.cell_bg(x, y));
        }
    }

    /// Write every lit cell `(x, y)` with `bg(x, y)` behind its dots.
    fn blit_with(&self, canvas: &mut Canvas, mut bg: impl FnMut(i32, i32) -> Ink) {
        const BLOCK: usize = 16;
        let mut blocks = self.masks.chunks_exact(BLOCK);
        let mut start = 0;
        for block in &mut blocks {
            if block != [0; BLOCK] {
                self.blit_run(canvas, start, block, &mut bg);
            }
            start += BLOCK;
        }
        self.blit_run(canvas, start, blocks.remainder(), &mut bg);
    }

    /// Blit the cells `start..start + masks.len()` (row-major indices).
    fn blit_run(
        &self,
        canvas: &mut Canvas,
        start: usize,
        masks: &[u8],
        bg: &mut impl FnMut(i32, i32) -> Ink,
    ) {
        let w = usize::from(self.cells.width());
        for (i, &mask) in (start..).zip(masks) {
            if mask != 0 {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                canvas.put_bg(x, y, braille(mask), self.inks[i], bg(x, y));
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
        fade(&mut self.buf, k);
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

    /// Set every pixel of `pixels` from the sum of its 2 × 2 dot block through `tone`
    /// (CORE-R81): level 0 ⇒ [`Ink::BLANK`], otherwise `ink(level)`. A `Density` and a
    /// `Pixels` over the same cells line up exactly (4 dot rows per 2 pixel rows); pixels
    /// with no full block in the density are emptied. One pass, no allocation.
    pub fn to_pixels(&self, pixels: &mut Pixels, tone: &Tone, mut ink: impl FnMut(u8) -> Ink) {
        let dw = usize::from(self.dots.width());
        // Two dot rows per pixel row; the dot height (4·h) is always even.
        let mut blocks = self.buf.chunks_exact(2 * dw);
        for row in pixels
            .px
            .chunks_exact_mut(usize::from(pixels.cells.width()))
        {
            let mut out = row.iter_mut();
            if let Some(pair) = blocks.next() {
                let (top, bottom) = pair.split_at(dw);
                let columns = top.chunks_exact(2).zip(bottom.chunks_exact(2));
                for ((t, b), px) in columns.zip(&mut out) {
                    let level = tone.level(t[0] + t[1] + b[0] + b[1]);
                    *px = if level == 0 { Ink::BLANK } else { ink(level) };
                }
            }
            out.for_each(|px| *px = Ink::BLANK);
        }
    }
}

/// Multiply every value by `k`, clamped to `[0, 1]` (NaN leaves the values as they are):
/// the shared fade of [`Density::decay`] and [`Glow::decay`].
fn fade(buf: &mut [f32], k: f32) {
    if k.is_nan() || k >= 1.0 {
        return;
    }
    let k = k.max(0.0);
    for v in buf {
        *v *= k;
    }
}

/// The 3 × 3 [`Glow::splat`] kernel for strokes (CORE-R96): centre 0.36, edge
/// neighbours 0.12, corners 0.04 (sum 1), so a stroked line exposes a soft two-pixel band.
pub const STROKE_KERNEL: [(i32, i32, f32); 9] = [
    (0, 0, 0.36),
    (-1, 0, 0.12),
    (1, 0, 0.12),
    (0, -1, 0.12),
    (0, 1, 0.12),
    (-1, -1, 0.04),
    (1, -1, 0.04),
    (-1, 1, 0.04),
    (1, 1, 0.04),
];

/// A round [`Glow::splat`] halo kernel of radius `r` pixels (CORE-R96): every offset of
/// the `(2r + 1)²` square, row by row, weighted `(1 − d²/(r + ½)²)²` (0 outside radius
/// `r + ½`), so it is 1 at the centre and falls smoothly to 0. `N` must be `(2r + 1)²`;
/// usable in a `const` (`const HALO: [_; 49] = halo_kernel(3);`), where a mismatch fails
/// the build.
pub const fn halo_kernel<const N: usize>(r: i32) -> [(i32, i32, f32); N] {
    assert!(r >= 0 && ((2 * r + 1) * (2 * r + 1)) as usize == N);
    let mut k = [(0, 0, 0.0f32); N];
    let reach = (r as f32 + 0.5) * (r as f32 + 0.5);
    let mut i = 0;
    let mut dy = -r;
    while dy <= r {
        let mut dx = -r;
        while dx <= r {
            let w = 1.0 - (dx * dx + dy * dy) as f32 / reach;
            k[i] = (dx, dy, if w > 0.0 { w * w } else { 0.0 });
            i += 1;
            dx += 1;
        }
        dy += 1;
    }
    k
}

/// A long-exposure `f32` buffer at [`Pixels`] resolution, `w × 2·h` (CORE-R85): points
/// [`splat`](Self::splat) weight into it by their [`Dots`] position, [`decay`](Self::decay)
/// fades it, and [`to_pixels`](Self::to_pixels) tone-maps it 1:1 onto a `Pixels` layer. A
/// soft glow under a point cloud or an event display, at an eighth of a [`Density`]'s
/// memory (one value per pixel instead of per dot).
#[derive(Debug, Clone)]
pub struct Glow {
    /// Buffer size in pixels.
    px: Size,
    /// Row-major, `px.w × px.h`.
    buf: Vec<f32>,
    /// One row of scratch for [`blur121`](Self::blur121), sized in `resize`.
    row: Vec<f32>,
}

impl Default for Glow {
    /// A 1×1-cell buffer, the placeholder until a scene's first `resize`.
    fn default() -> Self {
        Self::new(Size::new(1, 1))
    }
}

impl Glow {
    /// A pixel is this many dots across and down (a cell is 2 × 4 dots and 1 × 2 pixels).
    const DOTS_PER_PIXEL: i32 = 2;

    /// A zeroed buffer over `cells` terminal cells (`w × 2·h` pixels).
    pub fn new(cells: Size) -> Self {
        let mut g = Self {
            px: Size::new(1, 1),
            buf: Vec::new(),
            row: Vec::new(),
        };
        g.resize(cells);
        g
    }

    /// Buffer dimensions in pixels, as [`Pixels::pixel_size`].
    pub fn pixel_size(&self) -> Size {
        self.px
    }

    /// Resize to `cells` and zero. Reuses the buffer unless it must grow.
    pub fn resize(&mut self, cells: Size) {
        self.px = Size::new(cells.width(), sub_axis(cells.height(), 2));
        self.buf.clear();
        self.buf.resize(self.px.area(), 0.0);
        self.row.clear();
        self.row.resize(usize::from(self.px.width()), 0.0);
    }

    /// Zero every pixel.
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    /// Blur in place with the separable 3 × 3 binomial kernel `[1 2 1]/4 ⊗ [1 2 1]/4`,
    /// edges clamped (CORE-R103): a glow's rim ends softly instead of in pixel steps.
    /// Both passes walk rows (cache-friendly, vectorisable) and share one scratch row
    /// sized in `resize`, so it allocates nothing; a constant field is unchanged and
    /// interior mass is conserved.
    pub fn blur121(&mut self) {
        let w = usize::from(self.px.width());
        if w == 0 || self.row.len() < w {
            return;
        }
        for row in self.buf.chunks_exact_mut(w) {
            let mut prev = row[0];
            for x in 0..w {
                let cur = row[x];
                let next = row[(x + 1).min(w - 1)];
                row[x] = 0.25 * (prev + 2.0 * cur + next);
                prev = cur;
            }
        }
        // Vertical: `up` holds the row above as it was before this pass; each row is
        // replaced while its old values move into `up` for the row below.
        let up = &mut self.row[..w];
        up.copy_from_slice(&self.buf[..w]);
        let h = self.buf.len() / w;
        for y in 0..h {
            let (head, tail) = self.buf.split_at_mut((y + 1) * w);
            let cur = &mut head[y * w..];
            if y + 1 < h {
                for ((c, u), &d) in cur.iter_mut().zip(up.iter_mut()).zip(&tail[..w]) {
                    let old = *c;
                    *c = 0.25 * (*u + 2.0 * old + d);
                    *u = old;
                }
            } else {
                // The last row's neighbour below is itself (clamped).
                for (c, u) in cur.iter_mut().zip(up.iter_mut()) {
                    let old = *c;
                    *c = 0.25 * (*u + 2.0 * old + old);
                    *u = old;
                }
            }
        }
    }

    /// Multiply every value by `k`, clamped to `[0, 1]` (NaN leaves the buffer as is):
    /// an exponential fade, `k = exp(−dt / τ)` per step.
    pub fn decay(&mut self, k: f32) {
        fade(&mut self.buf, k);
    }

    /// Add `scale · w` to pixel `(px + dx, py + dy)` for each `(dx, dy, w)` of `kernel`,
    /// where `(px, py)` is the pixel holding dot `(x, y)` (dot units, rounded to the
    /// nearest dot; a pixel is 2 × 2 dots). Non-finite coordinates or weights and
    /// off-buffer pixels are ignored.
    pub fn splat(&mut self, x: f64, y: f64, kernel: &[(i32, i32, f32)], scale: f32) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        // Saturating casts: anything huge lands off the buffer.
        let (px, py) = (
            (x.round() as i32).div_euclid(Self::DOTS_PER_PIXEL),
            (y.round() as i32).div_euclid(Self::DOTS_PER_PIXEL),
        );
        let (w, h) = (i32::from(self.px.width()), i32::from(self.px.height()));
        for &(dx, dy, k) in kernel {
            let (x, y) = (px.saturating_add(dx), py.saturating_add(dy));
            let add = scale * k;
            if (0..w).contains(&x) && (0..h).contains(&y) && add.is_finite() {
                self.buf[y as usize * w as usize + x as usize] += add;
            }
        }
    }

    /// Stroke the dot-space segment `a → b` (CORE-R96): [`splat`](Self::splat) `kernel`
    /// at `scale` at `n = clamp(⌈len / 2⌉, 1, 2048)` evenly spaced points (the midpoints
    /// of `n` equal parts, one per pixel of length, a pixel being 2 dots), so a line's
    /// exposure per pixel of length does not depend on its length. A non-finite length
    /// draws nothing; the cap bounds a runaway segment.
    pub fn stroke(&mut self, a: (f64, f64), b: (f64, f64), kernel: &[(i32, i32, f32)], scale: f32) {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if !len.is_finite() {
            return;
        }
        let n = (len / f64::from(Self::DOTS_PER_PIXEL))
            .ceil()
            .clamp(1.0, 2048.0) as usize;
        for k in 0..n {
            let u = (k as f64 + 0.5) / n as f64;
            self.splat(a.0 + u * (b.0 - a.0), a.1 + u * (b.1 - a.1), kernel, scale);
        }
    }

    /// Add `w` at continuous **pixel** position `(x, y)` (integers are pixel centres; the
    /// centre of the pixel holding dot `(dx, dy)` is `((dx − 0.5)/2, (dy − 0.5)/2)`),
    /// split bilinearly over the four nearest pixels (CORE-R92). Motion below a pixel stays
    /// smooth and a moving source leaves no nearest-pixel stair steps; the in-bounds share
    /// of `w` is conserved. Non-finite input and off-buffer pixels are ignored.
    #[inline]
    pub fn splat_bilinear(&mut self, x: f64, y: f64, w: f32) {
        if !(x.is_finite() && y.is_finite() && w.is_finite()) {
            return;
        }
        let (pw, ph) = (usize::from(self.px.width()), usize::from(self.px.height()));
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        for (dx, dy, k) in [
            (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
            (1.0, 0.0, fx * (1.0 - fy)),
            (0.0, 1.0, (1.0 - fx) * fy),
            (1.0, 1.0, fx * fy),
        ] {
            let (px, py) = (x0 + dx, y0 + dy);
            // Saturating casts: anything huge fails the bounds check.
            if px >= 0.0 && py >= 0.0 && (px as usize) < pw && (py as usize) < ph {
                self.buf[py as usize * pw + px as usize] += w * k;
            }
        }
    }

    /// Every pixel value, row-major (`pixel_size().width()` per row).
    #[inline]
    pub fn values(&self) -> &[f32] {
        &self.buf
    }

    /// Every pixel value, mutable, row-major: for scene-side filters (a blur) between
    /// splatting and toning.
    #[inline]
    pub fn values_mut(&mut self) -> &mut [f32] {
        &mut self.buf
    }

    /// The value at pixel `(x, y)`, or `None` off the buffer.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<f32> {
        let (w, h) = (i32::from(self.px.width()), i32::from(self.px.height()));
        ((0..w).contains(&x) && (0..h).contains(&y))
            .then(|| self.buf[y as usize * w as usize + x as usize])
    }

    /// Set every pixel of `pixels` from its value through `tone`: level 0 ⇒
    /// [`Ink::BLANK`], otherwise `ink(level)` (which may itself be blank). A `Glow` and a
    /// `Pixels` over the same cells line up 1:1; pixels outside the glow are emptied. One
    /// pass, no allocation.
    pub fn to_pixels(&self, pixels: &mut Pixels, tone: &Tone, mut ink: impl FnMut(u8) -> Ink) {
        let gw = usize::from(self.px.width());
        let mut rows = self.buf.chunks_exact(gw.max(1));
        for row in pixels
            .px
            .chunks_exact_mut(usize::from(pixels.cells.width()).max(1))
        {
            let mut out = row.iter_mut();
            if let Some(src) = rows.next() {
                for (&v, px) in src.iter().zip(&mut out) {
                    let level = tone.level(v);
                    *px = if level == 0 { Ink::BLANK } else { ink(level) };
                }
            }
            out.for_each(|px| *px = Ink::BLANK);
        }
    }
}

/// An asinh tone curve for long exposures, tabulated so that a lookup costs one `sqrt`
/// (CORE-R81): linear for faint values, logarithmic for bright ones (Lupton et al. 2004),
/// so a sparse trail and a dense core both read without clipping. Inline and allocation-
/// free, so a scene can rebuild it in `resize`.
#[derive(Debug, Clone)]
pub struct Tone {
    /// `1 / white`: a value's position along the table is `√(v / white)`.
    inv_white: f32,
    /// Levels at `v = white · (i / (LUT − 1))²`: finest near zero, where asinh is steepest.
    lut: [u8; Tone::LUT],
}

impl Default for Tone {
    /// `Tone::asinh(1, 1)`, the fallback curve.
    fn default() -> Self {
        Self::asinh(1.0, 1.0)
    }
}

impl Tone {
    /// Table entries.
    pub const LUT: usize = 1024;

    /// `255 · asinh(v / knee) / asinh(white / knee)` on `[0, white]`, 255 above. A
    /// non-finite or non-positive parameter becomes 1, and `knee` is raised to
    /// `white / 1000`, the steepest stretch the table resolves to 2 levels.
    pub fn asinh(knee: f32, white: f32) -> Self {
        let sane = |v: f32| {
            if v.is_finite() && v > 0.0 {
                f64::from(v)
            } else {
                1.0
            }
        };
        let white = sane(white);
        let knee = sane(knee).max(white / 1000.0);
        let norm = (white / knee).asinh();
        let last = (Self::LUT - 1) as f64;
        let lut = std::array::from_fn(|i| {
            let s = i as f64 / last;
            ink::level(255.0 * (white * s * s / knee).asinh() / norm)
        });
        Self {
            inv_white: (1.0 / white) as f32,
            lut,
        }
    }

    /// The level of value `v`: 0 for `v ≤ 0` or NaN, 255 from `white` up.
    #[inline]
    pub fn level(&self, v: f32) -> u8 {
        // No branches: √ of a negative or NaN is NaN and the saturating cast maps it to
        // entry 0 (level 0); +∞ saturates to the last entry (255).
        let pos = (v * self.inv_white).sqrt() * (Self::LUT - 1) as f32;
        self.lut[(pos.round() as usize).min(Self::LUT - 1)]
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

    /// The one ink that stands for cell `(x, y)`'s two pixels (CORE-R84): the
    /// [`stronger`] of the two, or [`Ink::BLANK`] when both are empty or the cell is off
    /// the buffer. [`Dots::blit_over`] puts it behind a cell's dots.
    #[inline]
    pub fn cell_bg(&self, x: i32, y: i32) -> Ink {
        let px = |py: i32| self.get(x, py).unwrap_or(Ink::BLANK);
        let ink = stronger(px(2 * y), px(2 * y + 1));
        if is_empty(ink) { Ink::BLANK } else { ink }
    }

    /// Give every glyph already drawn over this buffer's pixels its cell's
    /// [`cell_bg`](Self::cell_bg) as background (CORE-R86), so line art, sprites and
    /// labels drawn after [`blit`](Self::blit) do not punch black boxes into a half-block
    /// picture. Glyphs and inks are kept; cells that already have a background, half and
    /// full blocks, and cells over empty pixels are left alone. Unicode only: an ASCII
    /// canvas has no half blocks to match.
    pub fn backdrop(&self, canvas: &mut Canvas) {
        if canvas.mode() != GlyphMode::Unicode {
            return;
        }
        let w = self.cells.width().min(canvas.size().width());
        let h = self.cells.height().min(canvas.size().height());
        for y in 0..i32::from(h) {
            for x in 0..i32::from(w) {
                let Some(&cell) = canvas.get(x, y) else {
                    continue;
                };
                if cell.bg != Ink::BLANK || matches!(cell.ch, '▀' | '▄' | '█') {
                    continue;
                }
                let under = self.cell_bg(x, y);
                if under != Ink::BLANK {
                    canvas.put_bg(x, y, Glyph::ascii(cell.ch), cell.ink, under);
                }
            }
        }
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

    /// Fade a glow's faint fringe smoothly into the empty sky (CORE-R89, M14), so it ends
    /// in the terminal's black instead of a ramp's tinted floor, with no dither. A pixel of
    /// intensity `i` (as for [`stronger`]) below `floor` is emptied when `i < floor / 2`,
    /// and in the band `[floor / 2, floor)` its intensity becomes
    /// `i·smoothstep((i − floor/2)/(floor/2))` (a `Div` pixel keeps its sign about 128),
    /// emptied if that rounds to 0. The fade is a pure function of the pixel's own
    /// intensity, continuous and non-decreasing in it, so a uniform field stays uniform: no
    /// ordered or noise pattern. Pixels at or above `floor`, and `Phase` pixels (no
    /// intensity), are untouched; `floor = 0` changes nothing. Allocation-free.
    pub fn dissolve(&mut self, floor: u8) {
        let half = f64::from(floor) / 2.0;
        for px in &mut self.px {
            let i = intensity(*px);
            if is_empty(*px) || px.role == Role::Phase || i >= floor {
                continue;
            }
            let u = ((f64::from(i) - half) / half).clamp(0.0, 1.0);
            let faded = (f64::from(i) * u * u * (3.0 - 2.0 * u)).round() as u8;
            *px = match (faded, px.role) {
                (0, _) => Ink::BLANK,
                (f, Role::Div) if px.level >= 128 => Ink::new(Role::Div, 128 + f),
                (f, Role::Div) => Ink::new(Role::Div, 128 - f),
                (f, role) => Ink::new(role, f),
            };
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
                half_block(canvas, x, y, top, bottom);
            }
        }
    }
}

/// Draw one cell from its two pixels by the Unicode rule of [`Pixels::blit`] (CORE-R42),
/// for a scene that composites an overlay over a blitted buffer.
#[inline]
pub fn half_block(canvas: &mut Canvas, x: i32, y: i32, top: Ink, bottom: Ink) {
    match (is_empty(top), is_empty(bottom)) {
        (true, true) => {}
        (false, true) => canvas.put(x, y, UPPER, top),
        (true, false) => canvas.put(x, y, LOWER, bottom),
        (false, false) => canvas.put_bg(x, y, UPPER, top, bottom),
    }
}
