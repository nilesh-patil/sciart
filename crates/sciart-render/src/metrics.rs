//! Art metrics over a painted frame (RND-R13; GEN-R17 blockiness, GEN-R18 darkness).
//!
//! Both work on the half-block *pixel grid* of a painted ratatui [`Buffer`]: every cell is
//! two pixels, top and bottom. A `▀` cell is its foreground on top and its background
//! below, `▄` the other way round, `█` its foreground twice, and any other cell whose
//! background is set is that background twice (a blank field cell). These are the
//! *field* pixels a scene draws as colour. A cell with any other glyph (text, braille,
//! stars) has a *glyph* pixel pair: it never forms or joins a flat square.
//!
//! Pure functions of the buffer: the probe (`pixi run rs-cost`) and the per-scene ratchet
//! tables in `crates/sciart/tests/art_metrics.rs` compute exactly the same numbers.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::Rgb;

/// Luma at or below which a colour counts as "near-black" ground (GEN-R18), and below
/// which a flat square is background, not a block (GEN-R17).
pub const DARK_LUMA: f64 = 0.08;
/// Side of the smallest flat square that counts as a block (GEN-R17).
pub const BLOCK: usize = 4;
/// Largest per-channel step between a flat square and a neighbouring field pixel that
/// still reads as a *stair* (the nearest-upsample signature), not an object edge.
pub const STAIR_STEP: u8 = 64;

/// The GEN-R17/R18 numbers of one painted frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ArtMetrics {
    /// Share of visible field pixels inside a flat `BLOCK`×`BLOCK` (or larger) square of
    /// one colour that has a stair step on its border (GEN-R17), in `[0, 1]`.
    pub blockiness: f64,
    /// The same share for flat squares of 2×2 or more pixels: small stairs that the
    /// GEN-R17 4×4 threshold lets through (reported by the probe, not ratcheted).
    pub blockiness_2x2: f64,
    /// Share of ground pixels that are the default background or near-black (GEN-R18),
    /// in `[0, 1]`.
    pub darkness: f64,
    /// Mean luma of the ground pixels, default background counting as 0, in `[0, 1]`.
    pub mean_luma: f64,
}

/// Rec. 709 luma of gamma-encoded sRGB, in `[0, 1]`: cheap, and ordered like perceived
/// brightness closely enough for "is this near black".
pub fn luma(Rgb(r, g, b): Rgb) -> f64 {
    (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
}

/// One pixel of the half-block grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Px {
    /// Terminal default background (nothing drawn).
    Default,
    /// Part of a text/braille/star cell: never part of a flat square.
    Glyph,
    /// A field pixel of this colour.
    Field(Rgb),
}

fn rgb_of(c: Color) -> Option<Rgb> {
    match c {
        Color::Rgb(r, g, b) => Some(Rgb(r, g, b)),
        _ => None,
    }
}

fn field(c: Color) -> Px {
    rgb_of(c).map_or(Px::Default, Px::Field)
}

/// The two pixels (top, bottom) a cell shows, and the two *ground* colours behind them
/// (what GEN-R18 calls the cell background: the field colour of a half-block pixel, the
/// cell background otherwise; `None` is the terminal default).
pub fn cell_pixels(cell: &ratatui::buffer::Cell) -> ([Px; 2], [Option<Rgb>; 2]) {
    let (fg, bg) = (cell.fg, cell.bg);
    match cell.symbol() {
        "▀" => ([field(fg), field(bg)], [rgb_of(fg), rgb_of(bg)]),
        "▄" => ([field(bg), field(fg)], [rgb_of(bg), rgb_of(fg)]),
        "█" => ([field(fg); 2], [rgb_of(fg); 2]),
        " " | "" => ([field(bg); 2], [rgb_of(bg); 2]),
        _ => {
            let ground = rgb_of(bg);
            ([Px::Glyph; 2], [ground; 2])
        }
    }
}

/// The frame as a `width × 2·height` pixel grid, row-major, plus the ground colours.
pub fn pixel_grid(buf: &Buffer) -> (usize, Vec<Px>, Vec<Option<Rgb>>) {
    let area = buf.area;
    let (w, h) = (usize::from(area.width), usize::from(area.height));
    let mut px = vec![Px::Default; w * 2 * h];
    let mut ground = vec![None; w * 2 * h];
    for cy in 0..h {
        for cx in 0..w {
            let cell = &buf.content[cy * w + cx];
            let (p, g) = cell_pixels(cell);
            for k in 0..2 {
                px[(2 * cy + k) * w + cx] = p[k];
                ground[(2 * cy + k) * w + cx] = g[k];
            }
        }
    }
    (w, px, ground)
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
pub fn darkness(ground: &[Option<Rgb>]) -> (f64, f64) {
    if ground.is_empty() {
        return (1.0, 0.0);
    }
    let (mut dark, mut sum) = (0usize, 0.0);
    for g in ground {
        let l = g.map_or(0.0, luma);
        sum += l;
        if l <= DARK_LUMA {
            dark += 1;
        }
    }
    let n = ground.len() as f64;
    (dark as f64 / n, sum / n)
}

/// All art metrics of a painted frame (RND-R13).
pub fn art_metrics(buf: &Buffer) -> ArtMetrics {
    let (w, px, ground) = pixel_grid(buf);
    let (darkness, mean_luma) = darkness(&ground);
    ArtMetrics {
        blockiness: blockiness(w, &px),
        blockiness_2x2: blockiness_n(w, &px, 2),
        darkness,
        mean_luma,
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
    fn gen_r17_nearest_upsampled_plateaus_are_blocky() {
        // A 4-pixel nearest upsample of a gentle ramp: 4×4 plateaus, stair steps between.
        let px = grid(16, 8, |x, _| {
            Px::Field(Rgb(100 + 8 * (x / 4) as u8, 90, 90))
        });
        assert!((blockiness(16, &px) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn gen_r17_smooth_gradient_is_not_blocky() {
        let px = grid(16, 8, |x, y| {
            Px::Field(Rgb(100 + 3 * x as u8, 90 + y as u8, 90))
        });
        assert_eq!(blockiness(16, &px), 0.0);
    }

    #[test]
    fn gen_r17_background_and_isolated_flats_do_not_count() {
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
    }

    #[test]
    fn gen_r17_glyph_pixels_break_flats() {
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
    fn gen_r18_darkness_counts_default_and_near_black_ground() {
        let ground = [
            None,
            Some(Rgb(10, 10, 10)),
            Some(Rgb(255, 255, 255)),
            Some(GREY),
        ];
        let (dark, mean) = darkness(&ground);
        assert_eq!(dark, 0.5);
        assert!((mean - (10.0 / 255.0 + 1.0 + 120.0 / 255.0) / 4.0).abs() < 1e-9);
        assert!(luma(Rgb(20, 20, 20)) <= DARK_LUMA && luma(Rgb(22, 22, 22)) > DARK_LUMA);
    }

    #[test]
    fn rnd_r13_pixel_grid_reads_half_blocks_and_ground() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        let (a, b) = (Color::Rgb(200, 0, 0), Color::Rgb(0, 120, 200));
        buf[(0, 0)].set_char('▀').set_fg(a).set_bg(b);
        buf[(1, 0)].set_char('▄').set_fg(a);
        buf[(2, 0)].set_char('*').set_fg(a).set_bg(b);
        buf[(3, 0)].set_char(' ').set_bg(b);
        let (w, px, ground) = pixel_grid(&buf);
        assert_eq!(w, 4);
        let (ra, rb) = (Rgb(200, 0, 0), Rgb(0, 120, 200));
        assert_eq!(
            &px[..4],
            &[Px::Field(ra), Px::Default, Px::Glyph, Px::Field(rb)]
        );
        assert_eq!(
            &px[4..],
            &[Px::Field(rb), Px::Field(ra), Px::Glyph, Px::Field(rb)]
        );
        assert_eq!(&ground[..4], &[Some(ra), None, Some(rb), Some(rb)]);
        assert_eq!(&ground[4..], &[Some(rb), Some(ra), Some(rb), Some(rb)]);
        let m = art_metrics(&buf);
        assert_eq!(m.blockiness, 0.0);
        assert_eq!(
            m.darkness,
            1.0 / 8.0,
            "only the empty top of the ▄ cell is default"
        );
    }
}
