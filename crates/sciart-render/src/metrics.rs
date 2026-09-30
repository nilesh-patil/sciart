//! Art metrics over a painted frame (RND-R13): blockiness, darkness, and the clean-style
//! measures lit coverage, chroma mass, edge weight, chroma p90 and lit-luma p99. The M14
//! rules built on them (GEN-R17, GEN-R18, GEN-R21) are superseded (ADR-0018); darkness,
//! lit coverage, chroma mass and edge weight still hold each scene's reviewed look as a
//! per-scene band (GEN-R24), the others are reported by the probe only.
//!
//! All work on the half-block *pixel grid* of a painted ratatui [`Buffer`]: every cell is
//! two pixels, top and bottom. A `▀` cell is its foreground on top and its background
//! below, `▄` the other way round, `█` its foreground twice, and any other cell whose
//! background is set is that background twice (a blank field cell). These are the
//! *field* pixels a scene draws as colour. A cell with any other glyph (text, braille,
//! stars) has a *glyph* pixel pair: it never forms or joins a flat square, its *ground*
//! is the cell background, and what it *shows* is the glyph's colour.
//!
//! Pure functions of the buffer: the probe (`pixi run rs-cost`) and the per-scene ratchet
//! tables in `crates/sciart/tests/art_metrics.rs` compute exactly the same numbers.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::Rgb;

/// Luma at or below which a field pixel is background, not part of a block (GEN-R17).
pub const DARK_LUMA: f64 = 0.08;
/// Largest channel of a colour that still counts as truly black (GEN-R18): a navy or teal
/// ground is not dark, however low its luma.
pub const DARK_MAX: u8 = 28;
/// Side of the smallest flat square that counts as a block (GEN-R17).
pub const BLOCK: usize = 4;
/// Largest per-channel step between a flat square and a neighbouring field pixel that
/// still reads as a *stair* (the nearest-upsample signature), not an object edge.
pub const STAIR_STEP: u8 = 64;

/// The RND-R13 art numbers of one painted frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ArtMetrics {
    /// Share of visible field pixels inside a flat `BLOCK`×`BLOCK` (or larger) square of
    /// one colour that has a stair step on its border (GEN-R17), in `[0, 1]`.
    pub blockiness: f64,
    /// The same share for flat squares of 2×2 or more pixels: small stairs that the
    /// GEN-R17 4×4 threshold lets through (reported by the probe, not ratcheted).
    pub blockiness_2x2: f64,
    /// Share of ground pixels that are the default background or truly black (GEN-R18),
    /// in `[0, 1]`.
    pub darkness: f64,
    /// Mean luma of the ground pixels, default background counting as 0, in `[0, 1]`.
    pub mean_luma: f64,
    /// Clean-style measures, on the preview grid (see [`clean_grid`]).
    pub clean: Clean,
}

/// The clean-style measures of one frame (RND-R13; bands per scene, GEN-R24).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Clean {
    /// Share of pixels that are lit (max channel > [`DARK_MAX`]), in `[0, 1]`.
    pub coverage: f64,
    /// Mean OKLCh chroma over all pixels, unlit ones counting 0: how much loud colour.
    pub chroma_mass: f64,
    /// Lit pixels per boundary pixel of the lit mask: ~1 for points and thin lines, the
    /// mean half-width of a filled shape in grid pixels (tens for big blobs).
    pub edge_weight: f64,
    /// 90th percentile of OKLCh chroma over the lit pixels (0 with 50 or fewer lit): the
    /// M14 neon cap was [`NEON_CAP`].
    pub chroma_p90: f64,
    /// 99th percentile of the luma of the lit pixels (0 with none lit): the subject's
    /// brightest parts (M14's legible-subject floor was [`LEGIBLE_LUMA`]).
    /// Unlike the other clean measures it is read from the half-block pixel grid's *shown*
    /// colours (a braille dot counts at its glyph colour): on the pooled preview grid a
    /// quarter-covered dot can never be brighter than luma ≈ 0.3.
    pub lit_luma_p99: f64,
}

/// M14's global ceiling on [`Clean::chroma_p90`] (the superseded GEN-R21 neon cap; a
/// reference value for the probe, no longer enforced, ADR-0018).
pub const NEON_CAP: f64 = 0.20;
/// M14's global floor on [`Clean::lit_luma_p99`] (the superseded GEN-R21 legible subject;
/// a reference value for the probe, no longer enforced, ADR-0018).
pub const LEGIBLE_LUMA: f64 = 0.6;

/// numpy's default percentile of sorted `v` (linear between order statistics).
fn percentile(v: &[f64], q: f64) -> f64 {
    let Some(&last) = v.last() else {
        return 0.0;
    };
    let pos = q * (v.len() - 1) as f64;
    let (i, t) = (pos.floor() as usize, pos - pos.floor());
    let hi = v.get(i + 1).copied().unwrap_or(last);
    v[i] + (hi - v[i]) * t
}

/// Rec. 709 luma of gamma-encoded sRGB, in `[0, 1]`: cheap, and ordered like perceived
/// brightness closely enough for these metrics.
pub fn luma(Rgb(r, g, b): Rgb) -> f64 {
    (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
}

/// Truly black: every channel ≤ [`DARK_MAX`] (GEN-R18).
pub fn is_black(Rgb(r, g, b): Rgb) -> bool {
    r.max(g).max(b) <= DARK_MAX
}

/// One pixel of the half-block grid, for flat-square detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Px {
    /// Terminal default background (nothing drawn).
    Default,
    /// Part of a text/braille/star cell: never part of a flat square.
    Glyph,
    /// A field pixel of this colour.
    Field(Rgb),
}

/// One pixel's three readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pixel {
    /// For flat-square detection (GEN-R17).
    pub px: Px,
    /// The ground behind it (GEN-R18); `None` is the terminal default.
    pub ground: Option<Rgb>,
    /// The colour it shows: the field colour, else the glyph's colour, else the ground
    /// (GEN-R21); `None` is the terminal default.
    pub shown: Option<Rgb>,
}

fn rgb_of(c: Color) -> Option<Rgb> {
    match c {
        Color::Rgb(r, g, b) => Some(Rgb(r, g, b)),
        _ => None,
    }
}

fn field(c: Color) -> Pixel {
    let rgb = rgb_of(c);
    Pixel {
        px: rgb.map_or(Px::Default, Px::Field),
        ground: rgb,
        shown: rgb,
    }
}

/// The two pixels (top, bottom) a cell shows.
pub fn cell_pixels(cell: &ratatui::buffer::Cell) -> [Pixel; 2] {
    let (fg, bg) = (cell.fg, cell.bg);
    match cell.symbol() {
        "▀" => [field(fg), field(bg)],
        "▄" => [field(bg), field(fg)],
        "█" => [field(fg); 2],
        " " | "" => [field(bg); 2],
        _ => {
            let ground = rgb_of(bg);
            [Pixel {
                px: Px::Glyph,
                ground,
                shown: rgb_of(fg).or(ground),
            }; 2]
        }
    }
}

/// The frame as a `width × 2·height` pixel grid, row-major; returns `(width, pixels)`.
pub fn pixel_grid(buf: &Buffer) -> (usize, Vec<Pixel>) {
    let area = buf.area;
    let (w, h) = (usize::from(area.width), usize::from(area.height));
    let blank = field(Color::Reset);
    let mut grid = vec![blank; w * 2 * h];
    for cy in 0..h {
        for cx in 0..w {
            let [top, bottom] = cell_pixels(&buf.content[cy * w + cx]);
            grid[2 * cy * w + cx] = top;
            grid[(2 * cy + 1) * w + cx] = bottom;
        }
    }
    (w, grid)
}

fn visible(p: Px) -> Option<Rgb> {
    match p {
        Px::Field(c) if luma(c) > DARK_LUMA => Some(c),
        _ => None,
    }
}

fn stair(a: Rgb, b: Rgb) -> bool {
    a != b
        && a.0
            .abs_diff(b.0)
            .max(a.1.abs_diff(b.1))
            .max(a.2.abs_diff(b.2))
            <= STAIR_STEP
}

/// GEN-R17 blockiness of a pixel grid `w` wide (see [`ArtMetrics::blockiness`]).
///
/// A `BLOCK`² window counts when all its pixels are the same visible field colour and at
/// least one pixel just outside its edges is a visible field pixel one stair step away
/// (a different colour within `STAIR_STEP` per channel). A pixel is blocky when any
/// counting window covers it; the share is taken over visible field pixels, so black sky
/// and the default background neither count nor dilute.
pub fn blockiness(w: usize, px: &[Px]) -> f64 {
    blockiness_n(w, px, BLOCK)
}

/// [`blockiness`] with flat squares of side `n` (≥ 1) or more.
pub fn blockiness_n(w: usize, px: &[Px], n: usize) -> f64 {
    let n = n.max(1);
    let h = px.len().checked_div(w).unwrap_or(0);
    let total = px.iter().filter(|p| visible(**p).is_some()).count();
    if total == 0 || w < n || h < n {
        return 0.0;
    }
    // `run[i]`: how many pixels from `i` rightwards share its visible colour.
    let mut run = vec![0u16; px.len()];
    for y in 0..h {
        let row = y * w;
        for x in (0..w).rev() {
            let i = row + x;
            run[i] = match visible(px[i]) {
                None => 0,
                Some(c) if x + 1 < w && visible(px[i + 1]) == Some(c) => run[i + 1] + 1,
                Some(_) => 1,
            };
        }
    }
    let mut covered = vec![false; px.len()];
    for y in 0..=h - n {
        for x in 0..=w - n {
            let i = y * w + x;
            let Some(c) = visible(px[i]) else { continue };
            let flat = (0..n).all(|k| {
                let j = i + k * w;
                usize::from(run[j]) >= n && visible(px[j]) == Some(c)
            });
            if !flat {
                continue;
            }
            let at = |x: usize, y: usize| visible(px[y * w + x]).is_some_and(|d| stair(c, d));
            let step = (0..n).any(|k| {
                (y > 0 && at(x + k, y - 1))
                    || (y + n < h && at(x + k, y + n))
                    || (x > 0 && at(x - 1, y + k))
                    || (x + n < w && at(x + n, y + k))
            });
            if step {
                for k in 0..n {
                    covered[i + k * w..i + k * w + n].fill(true);
                }
            }
        }
    }
    let blocky = covered.iter().filter(|&&b| b).count();
    blocky as f64 / total as f64
}

/// GEN-R18 `(darkness, mean_luma)` of the ground colours (see [`ArtMetrics`]).
pub fn darkness(ground: impl ExactSizeIterator<Item = Option<Rgb>>) -> (f64, f64) {
    let n = ground.len();
    if n == 0 {
        return (1.0, 0.0);
    }
    let (mut dark, mut sum) = (0usize, 0.0);
    for g in ground {
        sum += g.map_or(0.0, luma);
        if g.is_none_or(is_black) {
            dark += 1;
        }
    }
    (dark as f64 / n as f64, sum / n as f64)
}

/// OKLCh chroma of a linear-light-free sRGB triple in `[0, 255]` (Björn Ottosson's OKLab).
pub fn oklch_chroma(rgb: [f64; 3]) -> f64 {
    let lin = |c: f64| {
        let c = c / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = rgb.map(lin);
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let a = 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s;
    a.hypot(bb)
}

/// Grid pixels per cell of [`clean_grid`]: the `rs-preview` image (8×16 px per cell)
/// pooled 2×2, as the GEN-R21 reference measurements were taken.
pub const CLEAN_CELL: (usize, usize) = (4, 8);
/// The preview's terminal colours: default background and default foreground.
const TERM_BG: [f64; 3] = [12.0, 12.0, 18.0];
const TERM_FG: [f64; 3] = [204.0, 204.0, 204.0];

/// The frame as the `rs-preview` image pooled 2×2 (4×8 grid pixels per cell), last row
/// (the title) excluded, row-major; returns `(width, pixels)`.
///
/// Mirrors the preview's drawing: the cell background everywhere, `▀`/`▄`/`█` as solid
/// halves, braille dots as 2×2 image pixels centred in 4×4 slots (so each dot covers a
/// quarter of 2×2 grid pixels), and any other glyph as the preview's fallback mark, a
/// 4×6 image-pixel block (the font's strokes are not modelled; glyphs are sparse).
pub fn clean_grid(buf: &Buffer) -> (usize, Vec<[f64; 3]>) {
    let (cw, ch) = CLEAN_CELL;
    let (w, h) = (usize::from(buf.area.width), usize::from(buf.area.height));
    let rows = h.saturating_sub(1);
    let gw = w * cw;
    let mut grid = vec![TERM_BG; gw * rows * ch];
    let rgb = |c: Color, default: [f64; 3]| match c {
        Color::Rgb(r, g, b) => [f64::from(r), f64::from(g), f64::from(b)],
        _ => default,
    };
    // Blend `fg` over grid pixel (x, y) with coverage `k`.
    let mix = |grid: &mut Vec<[f64; 3]>, x: usize, y: usize, fg: [f64; 3], k: f64| {
        let p = &mut grid[y * gw + x];
        for c in 0..3 {
            p[c] += (fg[c] - p[c]) * k;
        }
    };
    for cy in 0..rows {
        for cx in 0..w {
            let cell = &buf.content[cy * w + cx];
            let (bg, fg) = (rgb(cell.bg, TERM_BG), rgb(cell.fg, TERM_FG));
            let (x0, y0) = (cx * cw, cy * ch);
            let mut fill = |x: std::ops::Range<usize>, y: std::ops::Range<usize>, c, k| {
                for yy in y {
                    for xx in x.clone() {
                        mix(&mut grid, x0 + xx, y0 + yy, c, k);
                    }
                }
            };
            fill(0..cw, 0..ch, bg, 1.0);
            let sym = cell.symbol();
            let first = sym.chars().next().unwrap_or(' ');
            match first {
                ' ' => {}
                '▀' => fill(0..cw, 0..ch / 2, fg, 1.0),
                '▄' => fill(0..cw, ch / 2..ch, fg, 1.0),
                '█' => fill(0..cw, 0..ch, fg, 1.0),
                '\u{2801}'..='\u{28FF}' => {
                    const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
                    let mask = (u32::from(first) - 0x2800) as u8;
                    for (dx, column) in BITS.iter().enumerate() {
                        for (dy, &bit) in column.iter().enumerate() {
                            if mask & bit != 0 {
                                // Image px 4d+1..4d+3 cover a quarter of grid px 2d, 2d+1.
                                fill(2 * dx..2 * dx + 2, 2 * dy..2 * dy + 2, fg, 0.25);
                            }
                        }
                    }
                }
                '\u{2800}' => {}
                _ => {
                    // Image block x 2..6, y 5..11: grid x 1..3 fully; grid y 3, 4 fully,
                    // 2 and 5 half.
                    fill(1..3, 3..5, fg, 1.0);
                    fill(1..3, 2..3, fg, 0.5);
                    fill(1..3, 5..6, fg, 0.5);
                }
            }
        }
    }
    (gw, grid)
}

/// The GEN-R21 measures of a [`clean_grid`] `w` wide.
pub fn clean_of(w: usize, grid: &[[f64; 3]]) -> Clean {
    let n = grid.len();
    if n == 0 || w == 0 {
        return Clean::default();
    }
    let h = n / w;
    let lit: Vec<bool> = grid
        .iter()
        .map(|p| p[0].max(p[1]).max(p[2]) > f64::from(DARK_MAX))
        .collect();
    let mut chroma: Vec<f64> = Vec::new();
    let mut last: Option<([f64; 3], f64)> = None;
    for (p, _) in grid.iter().zip(&lit).filter(|(_, l)| **l) {
        // Runs of one colour are the common case: reuse the last conversion.
        let c = match last {
            Some((q, c)) if q == *p => c,
            _ => oklch_chroma(*p),
        };
        last = Some((*p, c));
        chroma.push(c);
    }
    let lit_n = chroma.len();

    let mut boundary = 0usize;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !lit[i] {
                continue;
            }
            let interior = x > 0
                && x + 1 < w
                && y > 0
                && y + 1 < h
                && lit[i - 1]
                && lit[i + 1]
                && lit[i - w]
                && lit[i + w];
            if !interior {
                boundary += 1;
            }
        }
    }
    let mass = chroma.iter().sum::<f64>() / n as f64;
    let p90 = if lit_n > 50 {
        // numpy's default percentile: linear interpolation between order statistics.
        chroma.sort_unstable_by(f64::total_cmp);
        percentile(&chroma, 0.9)
    } else {
        0.0
    };
    Clean {
        coverage: lit_n as f64 / n as f64,
        chroma_mass: mass,
        edge_weight: lit_n as f64 / boundary.max(1) as f64,
        chroma_p90: p90,
        lit_luma_p99: 0.0,
    }
}

/// GEN-R21 measures of a painted frame.
pub fn clean(buf: &Buffer) -> Clean {
    let (w, grid) = clean_grid(buf);
    let (pw, px) = pixel_grid(buf);
    // Without the title row (two pixel rows), like the other clean measures.
    let art = px.len().saturating_sub(2 * pw);
    Clean {
        lit_luma_p99: lit_luma_p99(px[..art].iter().map(|p| p.shown)),
        ..clean_of(w, &grid)
    }
}

/// GEN-R21 legible subject: the 99th percentile of the luma of the shown colours that are
/// lit (max channel > [`DARK_MAX`]); 0 with none lit.
pub fn lit_luma_p99(shown: impl Iterator<Item = Option<Rgb>>) -> f64 {
    let mut l: Vec<f64> = shown
        .flatten()
        .filter(|c| !is_black(*c))
        .map(luma)
        .collect();
    l.sort_unstable_by(f64::total_cmp);
    percentile(&l, 0.99)
}

/// All art metrics of a painted frame (RND-R13).
pub fn art_metrics(buf: &Buffer) -> ArtMetrics {
    let (w, grid) = pixel_grid(buf);
    let px: Vec<Px> = grid.iter().map(|p| p.px).collect();
    let (darkness, mean_luma) = darkness(grid.iter().map(|p| p.ground));
    ArtMetrics {
        blockiness: blockiness(w, &px),
        blockiness_2x2: blockiness_n(w, &px, 2),
        darkness,
        mean_luma,
        clean: clean(buf),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    const GREY: Rgb = Rgb(120, 120, 120);
    const GREY2: Rgb = Rgb(130, 120, 120);

    /// `w × h` pixels, colour from `f(x, y)`.
    fn grid(w: usize, h: usize, f: impl Fn(usize, usize) -> Px) -> Vec<Px> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect()
    }

    #[test]
    fn rnd_r13_nearest_upsampled_plateaus_are_blocky() {
        // A 4-pixel nearest upsample of a gentle ramp: 4×4 plateaus, stair steps between.
        let px = grid(16, 8, |x, _| {
            Px::Field(Rgb(100 + 8 * (x / 4) as u8, 90, 90))
        });
        assert!((blockiness(16, &px) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rnd_r13_smooth_gradient_is_not_blocky() {
        let px = grid(16, 8, |x, y| {
            Px::Field(Rgb(100 + 3 * x as u8, 90 + y as u8, 90))
        });
        assert_eq!(blockiness(16, &px), 0.0);
    }

    #[test]
    fn rnd_r13_background_and_isolated_flats_do_not_count() {
        // Default and near-black ground never count.
        assert_eq!(blockiness(8, &grid(8, 8, |_, _| Px::Default)), 0.0);
        assert_eq!(
            blockiness(8, &grid(8, 8, |_, _| Px::Field(Rgb(5, 5, 5)))),
            0.0
        );
        // A flat grey patch on black: its edge is an object edge, not a stair.
        let px = grid(12, 12, |x, y| {
            if (4..8).contains(&x) && (4..8).contains(&y) {
                Px::Field(GREY)
            } else {
                Px::Default
            }
        });
        assert_eq!(blockiness(12, &px), 0.0);
        // 3×3 plateaus (a 3-pixel nearest upsample) are below the block size.
        let px = grid(12, 12, |x, y| {
            if (x / 3 + y / 3) % 2 == 0 {
                Px::Field(GREY)
            } else {
                Px::Field(GREY2)
            }
        });
        assert_eq!(blockiness(12, &px), 0.0);
        assert!(blockiness_n(12, &px, 2) > 0.9, "but they are 2x2 stairs");
    }

    #[test]
    fn rnd_r13_glyph_pixels_break_flats() {
        let px = grid(8, 8, |x, y| {
            if (x + y) % 3 == 0 {
                Px::Glyph
            } else {
                Px::Field(GREY)
            }
        });
        assert_eq!(blockiness(8, &px), 0.0);
    }

    #[test]
    fn rnd_r13_darkness_counts_default_and_truly_black_ground() {
        let ground = [
            None,
            Some(Rgb(28, 20, 10)),
            Some(Rgb(255, 255, 255)),
            Some(GREY),
            // Navy: low luma, but not black.
            Some(Rgb(10, 20, 60)),
        ];
        let (dark, mean) = darkness(ground.into_iter());
        assert_eq!(dark, 2.0 / 5.0);
        let want = ground.iter().map(|g| g.map_or(0.0, luma)).sum::<f64>() / 5.0;
        assert!((mean - want).abs() < 1e-12);
        assert!(is_black(Rgb(28, 28, 28)) && !is_black(Rgb(29, 0, 0)));
    }

    #[test]
    fn rnd_r13_chroma_is_zero_for_grey_and_matches_oklab_references() {
        assert!(oklch_chroma([120.0; 3]) < 1e-6 && oklch_chroma([255.0; 3]) < 1e-6);
        // OKLab reference values: sRGB red C ≈ 0.2577, blue ≈ 0.3132.
        assert!((oklch_chroma([255.0, 0.0, 0.0]) - 0.2577).abs() < 1e-3);
        assert!((oklch_chroma([0.0, 0.0, 255.0]) - 0.3132).abs() < 1e-3);
    }

    /// A `w × h` clean grid of `bg` with the given lit pixels in `fg`.
    fn clean_with(w: usize, h: usize, fg: [f64; 3], lit: impl Fn(usize, usize) -> bool) -> Clean {
        let grid: Vec<[f64; 3]> = (0..w * h)
            .map(|i| if lit(i % w, i / w) { fg } else { TERM_BG })
            .collect();
        clean_of(w, &grid)
    }

    #[test]
    fn rnd_r13_thin_lines_weigh_one_and_blobs_weigh_their_half_width() {
        let red = [200.0, 30.0, 30.0];
        let line = clean_with(40, 40, red, |x, y| y == 20 && x > 2);
        assert!((line.edge_weight - 1.0).abs() < 1e-12);
        assert!((line.coverage - 37.0 / 1600.0).abs() < 1e-12);
        let blob = clean_with(40, 40, red, |x, y| {
            (5..35).contains(&x) && (5..35).contains(&y)
        });
        // 30×30 square: 900 lit, 116 on the boundary.
        assert!((blob.edge_weight - 900.0 / 116.0).abs() < 1e-9);
        assert!(
            blob.chroma_mass > line.chroma_mass * 20.0,
            "mass grows with area"
        );
        assert!(
            blob.chroma_p90 > 0.15 && line.chroma_p90 == 0.0,
            "37 lit: no percentile"
        );
        let empty = clean_with(10, 10, red, |_, _| false);
        assert_eq!(
            empty,
            Clean {
                edge_weight: 0.0,
                ..Clean::default()
            }
        );
    }

    #[test]
    fn rnd_r13_lit_luma_p99_reads_the_brightest_lit_pixels() {
        // 99 dim lit pixels and one white one: p99 interpolates towards white.
        let mut px = vec![Some(Rgb(60, 60, 60)); 99];
        px.push(Some(Rgb(255, 255, 255)));
        px.extend([None, Some(Rgb(20, 20, 20))]); // unlit: ignored
        let dim = 60.0 / 255.0;
        let p99 = lit_luma_p99(px.into_iter());
        assert!((p99 - (dim + (1.0 - dim) * 0.01)).abs() < 1e-9, "{p99}");
        assert!(p99 < LEGIBLE_LUMA, "a faint scene");
        assert!(lit_luma_p99([Some(Rgb(230, 230, 230))].into_iter()) > LEGIBLE_LUMA);
        assert_eq!(lit_luma_p99([None].into_iter()), 0.0, "none lit");
        assert_eq!(percentile(&[1.0, 2.0, 3.0], 0.5), 2.0);
    }

    #[test]
    fn rnd_r13_neon_cap_needs_more_than_fifty_lit_pixels() {
        let few = clean_with(20, 20, [255.0, 0.0, 255.0], |x, y| y == 0 && x < 50);
        assert_eq!(few.chroma_p90, 0.0, "20 lit pixels: no percentile");
        let many = clean_with(100, 2, [255.0, 0.0, 255.0], |_, y| y == 0);
        assert!(
            many.chroma_p90 > NEON_CAP,
            "magenta is neon: {}",
            many.chroma_p90
        );
    }

    #[test]
    fn rnd_r13_clean_grid_mirrors_the_preview() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        buf[(0, 0)].set_char('▀').set_fg(Color::Rgb(200, 0, 0));
        buf[(1, 0)].set_char('⠁').set_fg(Color::Rgb(255, 255, 255)); // dot 1: top left
        buf[(2, 0)].set_char('x').set_fg(Color::Rgb(100, 100, 100));
        buf[(0, 1)].set_char('T').set_fg(Color::Rgb(255, 255, 255)); // title row: dropped
        let (w, g) = clean_grid(&buf);
        assert_eq!((w, g.len()), (12, 12 * 8));
        assert_eq!(g[0], [200.0, 0.0, 0.0]);
        assert_eq!(
            g[4 * 12],
            TERM_BG,
            "bottom half of ▀ is the default background"
        );
        let dot = g[4];
        assert!(
            (dot[0] - (255.0 + 3.0 * 12.0) / 4.0).abs() < 1e-9,
            "a quarter-covered dot"
        );
        assert_eq!(g[12 + 5], dot, "the dot spans 2×2 grid pixels");
        assert_eq!(g[2 * 12 + 4], TERM_BG);
        assert_eq!(g[3 * 12 + 9], [100.0; 3], "glyph mark");
        let c = clean_of(w, &g);
        // Lit: 16 red + 4 dot pixels (73 > 28) + 4 full mark + 4 half mark (56 > 28).
        assert!((c.coverage - 28.0 / 96.0).abs() < 1e-12, "{}", c.coverage);
    }

    #[test]
    fn rnd_r13_pixel_grid_reads_half_blocks_ground_and_glyphs() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        let (a, b) = (Color::Rgb(200, 0, 0), Color::Rgb(0, 120, 200));
        buf[(0, 0)].set_char('▀').set_fg(a).set_bg(b);
        buf[(1, 0)].set_char('▄').set_fg(a);
        buf[(2, 0)].set_char('*').set_fg(a).set_bg(b);
        buf[(3, 0)].set_char(' ').set_bg(b);
        let (w, g) = pixel_grid(&buf);
        assert_eq!(w, 4);
        let (ra, rb) = (Rgb(200, 0, 0), Rgb(0, 120, 200));
        let px: Vec<Px> = g.iter().map(|p| p.px).collect();
        assert_eq!(
            px,
            [
                Px::Field(ra),
                Px::Default,
                Px::Glyph,
                Px::Field(rb),
                Px::Field(rb),
                Px::Field(ra),
                Px::Glyph,
                Px::Field(rb)
            ]
        );
        let ground: Vec<_> = g.iter().map(|p| p.ground).collect();
        assert_eq!(
            ground,
            [
                Some(ra),
                None,
                Some(rb),
                Some(rb),
                Some(rb),
                Some(ra),
                Some(rb),
                Some(rb)
            ]
        );
        assert_eq!(g[2].shown, Some(ra), "a glyph shows its own colour");
        let m = art_metrics(&buf);
        assert_eq!(m.blockiness, 0.0);
        assert_eq!(m.darkness, 1.0 / 8.0, "only the empty top of the ▄ cell");
        assert_eq!(
            m.clean,
            Clean::default(),
            "a one-row frame is all title row"
        );
    }
}
