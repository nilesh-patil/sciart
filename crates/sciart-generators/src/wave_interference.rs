//! A ripple tank: two coherent sources whose rings interfere into bright antinodal bands
//! and still, dark nodal hyperbolas, with a detector screen's intensity profile showing
//! the fringes (SPEC-002 `wave_interference`). On black (ADR-0017): only the crests are
//! lit, as thin pale-blue bands; troughs, still water and the nodal lines are black, and
//! the sources are single white points.

use std::f64::consts::TAU;

use sciart_core::geom::ASPECT;
use sciart_core::ink::{self, Ramp};
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};

pub static META: Meta = Meta {
    name: "wave_interference",
    category: Category::Physics,
    description: "A ripple tank: two coherent sources interfere into bright bands and still nodal lines",
    title: "Interference · crests add up",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(WaveInterference::default())
}

/// Angular frequency, rad/s.
const OMEGA: f64 = 3.0;
/// Mean source separation as a fraction of the height, its relative sweep and period (s).
const SEPARATION: f64 = 0.3;
const SWEEP: f64 = 0.3;
const SWEEP_PERIOD: f64 = 36.0;
/// Sources sit this fraction of the way across the field.
const SOURCE_X: f64 = 0.2;
/// Vignette length as a fraction of the height (art only, WAVE model).
const VIGNETTE: f64 = 0.8;
/// Brightness below which a pixel is left to the terminal background: the nodal lines.
const DARK: f64 = 0.05;
const PALETTE: Ramp = Ramp::Ice;
const SOURCE_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 255);
/// Crest level at full brightness (a pale blue, the accent), and the level below which a
/// crest's falloff is left black rather than a navy haze.
const CREST_TOP: f64 = 200.0;
const CREST_FLOOR: u8 = 28;
/// ASCII rings by brightness, blank below [`DARK`].
const ASCII_RAMP: [char; 5] = ['.', 'o', 'O', '0', '@'];
/// Blank cells kept after the science title on the last row (`WAVE-A3`).
const TITLE_GAP: usize = 3;
/// Blank columns between the field and the detector screen (`WAVE-A6`).
const SCREEN_GAP: usize = 2;
/// Profile levels `SCREEN_LOW + SCREEN_SPAN·I`, never under `PROFILE_LOW`, so its node
/// rows still read as a line; topping out at 180, under the brightest crest.
const SCREEN_LOW: f64 = 40.0;
const SCREEN_SPAN: f64 = 140.0;
const PROFILE_LOW: f64 = 120.0;
/// The screen strip is `1/SCREEN_FRAC` of the width, at least `SCREEN_MIN` columns.
const SCREEN_FRAC: usize = 12;
const SCREEN_MIN: usize = 4;

/// Wavelength in rows for a screen `h` rows tall.
pub(crate) fn wavelength(h: u16) -> f64 {
    (f64::from(h) / 9.0).max(2.5)
}

/// Source separation in rows at time `t` (WAVE-A5).
pub(crate) fn separation(h: f64, t: f64) -> f64 {
    SEPARATION * h * (1.0 + SWEEP * (TAU * t / SWEEP_PERIOD).sin_det())
}

/// Superposed displacement of two unit waves at distances `d1`, `d2` (WAVE-A1, WAVE-A2).
#[cfg(test)]
pub(crate) fn amplitude(d1: f64, d2: f64, k: f64, t: f64) -> f64 {
    (k * d1 - OMEGA * t).sin_det() + (k * d2 - OMEGA * t).sin_det()
}

/// Displacement and envelope `(a, E)` at distances `d1`, `d2`, with `(sin ωt, cos ωt)`
/// precomputed: `a = S cos ωt − C sin ωt`, `E = |e^{ikd₁} + e^{ikd₂}| = √(S² + C²)`.
#[inline]
fn wave(d1: f64, d2: f64, k: f64, (sw, cw): (f64, f64)) -> (f64, f64) {
    let ((s1, c1), (s2, c2)) = ((k * d1).sin_cos_det(), (k * d2).sin_cos_det());
    let (s, c) = (s1 + s2, c1 + c2);
    (s * cw - c * sw, (s * s + c * c).sqrt())
}

/// Ripple-tank brightness in [0, 1] (ADR-0017: crests only): `vignette · (E/2)^1.5 ·
/// max(0, a/E)²`. It is 0 wherever the envelope is 0 (WAVE-A4) and at every trough and
/// zero crossing, so only the crests light, as thin bands; on an antinodal band (E = 2) it
/// swings from 0 to the vignette.
pub(crate) fn brightness(a: f64, e: f64, vignette: f64) -> f64 {
    if e < 1e-9 {
        return 0.0;
    }
    let half = 0.5 * e;
    let crest = (a / e).max(0.0);
    (vignette * half * half.sqrt() * crest * crest).clamp(0.0, 1.0)
}

/// The ink of a field pixel at brightness `v`: a crest in the palette up to
/// [`CREST_TOP`], black below [`CREST_FLOOR`].
pub(crate) fn crest_ink(v: f64) -> Ink {
    let level = ink::level(CREST_TOP * v);
    if v < DARK || level < CREST_FLOOR {
        Ink::BLANK
    } else {
        Ink::new(Role::Ramp(PALETTE), level)
    }
}

#[derive(Debug, Default)]
pub struct WaveInterference {
    t: f64,
    height: u16,
    k: f64,
    /// Field pixel columns `0..field_w`; the screen strip is `screen.0..screen.0 + screen.1`.
    field_w: usize,
    screen: (usize, usize),
    /// Per field pixel, the vignette (fixed: the sources' midpoint does not move).
    vignette: Vec<f32>,
    /// Pixel columns and row of the title gap (`WAVE-A3`), empty when there is no title.
    quiet: (std::ops::Range<i32>, i32),
    sources: [(f64, f64); 2],
    pixels: Pixels,
    /// The screen's intensity profile, a braille curve over the fill (WAVE-A8).
    profile: Dots,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl WaveInterference {
    /// Pixel centre in rows (x divided by ASPECT, so distances are isotropic on screen).
    fn at(px: usize, py: usize) -> (f64, f64) {
        ((px as f64 + 0.5) / ASPECT, (py as f64 + 0.5) / 2.0)
    }

    /// Sources for the current time, in rows.
    fn place_sources(&mut self) {
        let h = f64::from(self.height);
        let half = 0.5 * separation(h, self.t);
        let x = SOURCE_X * self.field_w as f64 / ASPECT;
        self.sources = [(x, 0.5 * h - half), (x, 0.5 * h + half)];
    }

    fn distances(&self, p: (f64, f64)) -> (f64, f64) {
        let [a, b] = self.sources;
        ((p.0 - a.0).hypot(p.1 - a.1), (p.0 - b.0).hypot(p.1 - b.1))
    }

    /// Recompute the field, the screen and the sources into the pixels.
    fn rasterise(&mut self) {
        self.place_sources();
        let ph = 2 * usize::from(self.height);
        let phase = (OMEGA * self.t).sin_cos_det();
        let (fw, k) = (self.field_w, self.k);
        // The sources sit symmetrically about the middle row, and swapping d₁ and d₂
        // leaves a and E unchanged, so each row is computed once and mirrored.
        for py in 0..ph.div_ceil(2) {
            let rows = [py as i32, (ph - 1 - py) as i32];
            for px in 0..fw {
                let (d1, d2) = self.distances(Self::at(px, py));
                let (a, e) = wave(d1, d2, k, phase);
                let ink = crest_ink(brightness(a, e, f64::from(self.vignette[py * fw + px])));
                for y in rows {
                    self.pixels.set(px as i32, y, ink);
                }
            }
        }
        self.trace_profile();
        for (sx, sy) in self.sources {
            // A single white point each.
            let (px, py) = (
                (sx * ASPECT - 0.5).round() as i32,
                (sy * 2.0 - 0.5).round() as i32,
            );
            self.pixels.set(px, py, SOURCE_INK);
        }
        let (cols, row) = &self.quiet;
        for x in cols.clone() {
            self.pixels.set(x, 2 * row, Ink::BLANK);
            self.pixels.set(x, 2 * row + 1, Ink::BLANK);
        }
    }

    /// Time-averaged intensity `I = E²/4` at the field's right edge, at `y2` half-rows
    /// (pixel rows) from the top (WAVE-A6).
    fn intensity(&self, y2: f64) -> f64 {
        let x = (self.field_w as f64 + 0.5) / ASPECT;
        let (d1, d2) = self.distances((x, 0.5 * y2));
        let (_, e) = wave(d1, d2, self.k, (0.0, 1.0));
        0.25 * e * e
    }

    /// The profile curve (WAVE-A8): one point per braille dot row, `I` of the way from the
    /// strip's right edge to its left, joined row to row, at `PROFILE_LOW..180` by `I`.
    /// Static in time (`I` is a time average), but redrawn with the sweeping sources.
    fn trace_profile(&mut self) {
        self.profile.clear();
        let (x0, sw) = self.screen;
        if sw < 2 {
            return;
        }
        let base = f64::from(Dots::CELL_W) * (x0 + sw - 1) as f64 - 0.5;
        let reach = base - f64::from(Dots::CELL_W) * x0 as f64;
        let rows = f64::from(Dots::CELL_H) * f64::from(self.height);
        let mut prev: Option<(f64, f64)> = None;
        let mut y = 0.0;
        while y < rows {
            // Dot row y spans y..y+1 of 4 dots per cell, i.e. 2 pixel rows per 4 dots.
            let i = self.intensity((y + 0.5) / 2.0);
            let x = base - i.clamp(0.0, 1.0) * reach;
            let level = (SCREEN_LOW + SCREEN_SPAN * i).max(PROFILE_LOW) / 255.0;
            let ink = ink::ramp(PALETTE, level);
            match prev {
                Some((px, py)) => self.profile.line(px, py, x, y, ink),
                None => self.profile.plot(x, y, ink),
            }
            prev = Some((x, y));
            y += 1.0;
        }
    }

    fn is_screen(&self, x: i32) -> bool {
        let (x0, sw) = self.screen;
        sw > 0 && (x0..x0 + sw).contains(&(x as usize))
    }
}

/// ASCII for one cell of the field from its two pixels: a ring by mean brightness.
fn ascii_field(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let v = |p: Ink| match p.role {
        Role::Ramp(_) => ink::seq_value(p),
        _ => 0.0,
    };
    let mean = 0.5 * (v(top) + v(bottom));
    if mean < DARK {
        return None;
    }
    let i = ((mean * ASCII_RAMP.len() as f64) as usize).min(ASCII_RAMP.len() - 1);
    Some((ASCII_RAMP[i], ink::ramp(PALETTE, mean)))
}

impl Generator for WaveInterference {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let w = usize::from(size.width());
        let ph = 2 * usize::from(size.height());
        self.height = size.height();
        self.k = TAU / wavelength(size.height());
        // A screen only when it leaves the field most of the width.
        let sw = if w >= 24 {
            (w / SCREEN_FRAC).max(SCREEN_MIN)
        } else {
            0
        };
        self.field_w = if sw > 0 { w - sw - SCREEN_GAP } else { w };
        self.screen = (w - sw, sw);
        self.pixels.resize(size);
        self.profile.resize(size);
        // The vignette about the sources' fixed midpoint.
        let h = f64::from(size.height());
        let mid = (SOURCE_X * self.field_w as f64 / ASPECT, 0.5 * h);
        let len = (VIGNETTE * h).max(1.0);
        self.vignette.clear();
        self.vignette.reserve(self.field_w * ph);
        for py in 0..ph {
            for px in 0..self.field_w {
                let (x, y) = Self::at(px, py);
                let r = (x - mid.0).hypot(y - mid.1);
                self.vignette.push((1.0 + r / len).powf(-0.5) as f32);
            }
        }
        self.quiet =
            title::span(size, &Title::new(META.title, None)).map_or((0..0, 0), |(y, cols)| {
                (
                    cols.end..(cols.end + TITLE_GAP as i32).min(i32::from(size.width())),
                    y,
                )
            });
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        // A NaN or infinite t would stay in the phase for good (GEN-R4).
        if !(dt.is_finite() && dt >= 0.0) {
            return;
        }
        self.t += dt;
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels.blit(canvas, |x, _, top, bottom| {
            if top == SOURCE_INK || bottom == SOURCE_INK {
                return Some(('*', SOURCE_INK));
            }
            if self.is_screen(x) {
                // The screen strip holds only the profile's dots.
                return None;
            }
            ascii_field(top, bottom)
        });
        self.profile.blit_over(canvas, &self.pixels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: f64 = TAU / 5.0;

    fn scene(w: u16, h: u16, t: f64) -> WaveInterference {
        let size = Size::new(w, h);
        let mut g = WaveInterference::default();
        g.resize(size, &mut Ctx::new(1, size));
        g.prepare();
        g.t = t;
        g.rasterise();
        g
    }

    #[test]
    fn gen_r4_wave_update_ignores_a_non_finite_or_negative_dt() {
        let mut g = scene(80, 24, 1.5);
        let mut ctx = Ctx::new(1, Size::new(80, 24));
        for dt in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1] {
            g.update(dt, &mut ctx);
            g.prepare();
            assert_eq!(g.t, 1.5, "dt = {dt}");
        }
        // A good step afterwards still advances the clock.
        g.update(0.25, &mut ctx);
        g.prepare();
        assert_eq!(g.t, 1.75);
    }

    #[test]
    fn wave_a1_bisector_amplitude_is_twice_a_single_wave() {
        for d in [0.0, 1.3, 7.7, 25.0] {
            for t in [0.0, 0.2, 1.1, 9.4] {
                let expected = 2.0 * (K * d - OMEGA * t).sin_det();
                assert!((amplitude(d, d, K, t) - expected).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn wave_a2_half_wavelength_path_difference_cancels_at_all_times() {
        let lambda = TAU / K;
        for d in [2.0, 11.5, 40.0] {
            for i in 0..200 {
                let t = f64::from(i) * 0.037;
                assert!(amplitude(d, d + lambda / 2.0, K, t).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn wave_a1_a4_fast_form_matches_the_reference_and_its_envelope() {
        for (d1, d2, t) in [(3.0, 4.2, 0.3), (10.0, 10.0, 1.7), (2.5, 7.5, 5.0)] {
            let (a, e) = wave(d1, d2, K, (OMEGA * t).sin_cos_det());
            assert!((a - amplitude(d1, d2, K, t)).abs() < 1e-9);
            let want = 2.0 * (K * (d1 - d2) / 2.0).cos_det().abs();
            assert!((e - want).abs() < 1e-9 && a.abs() <= e + 1e-9);
        }
    }

    #[test]
    fn wave_a3_three_cells_after_the_title_stay_blank() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let mut stage = Stage::new(build(), 1, size, GlyphMode::Unicode);
            let (_, cols) = title::span(size, &Title::new(META.title, None)).unwrap();
            let end = cols.end as usize;
            let mut resumed = false;
            for _ in 0..40 {
                stage.step(0.05);
                let last = stage.render().to_lines().pop().unwrap();
                let row: Vec<char> = last.chars().collect();
                assert_eq!(row[end..end + TITLE_GAP], [' '; TITLE_GAP], "{last}");
                resumed |= row[end + TITLE_GAP..].iter().any(|&c| c != ' ');
            }
            assert!(resumed, "the field resumes after the gap");
        }
    }

    #[test]
    fn wave_a4_nodal_lines_stay_dark_and_antinodes_swing() {
        let g = scene(120, 40, 0.0);
        let (fw, ph) = (g.field_w, 80);
        let period = TAU / OMEGA;
        let (mut nodes, mut swings) = (0, 0);
        for py in 0..ph {
            for px in 0..fw {
                let (d1, d2) = g.distances(WaveInterference::at(px, py));
                let vig = f64::from(g.vignette[py * fw + px]);
                // Levels through one period, the sources held where they are at t = 0.
                let (lo, hi) = (0..12).fold((255u8, 0u8), |(lo, hi), i| {
                    let wt = OMEGA * period * f64::from(i) / 12.0;
                    let (a, e) = wave(d1, d2, g.k, wt.sin_cos_det());
                    let v = brightness(a, e, vig);
                    let l = crest_ink(v).level;
                    (lo.min(l), hi.max(l))
                });
                let e = wave(d1, d2, g.k, (0.0, 1.0)).1;
                if e < 0.02 {
                    assert_eq!(hi, 0, "node ({px}, {py}) lit");
                    let p = g.pixels.get(px as i32, py as i32).unwrap();
                    assert!(p == Ink::BLANK || p == SOURCE_INK, "({px}, {py}) {p:?}");
                    nodes += 1;
                } else if e > 1.9 && vig > 0.8 {
                    assert!(hi - lo >= 120, "antinode ({px}, {py}): {lo}..{hi}");
                    swings += 1;
                }
            }
        }
        assert!(
            nodes > 20 && swings > 20,
            "{nodes} nodes, {swings} antinodes"
        );
    }

    #[test]
    fn wave_a5_sources_sweep_apart_and_back() {
        let h = 50.0;
        assert!((separation(h, 0.0) - 0.3 * h).abs() < 1e-12);
        assert!((separation(h, 9.0) - 0.39 * h).abs() < 1e-9);
        assert!((separation(h, 27.0) - 0.21 * h).abs() < 1e-9);
        for t in [1.0, 13.3, 30.0] {
            assert!((separation(h, t + 36.0) - separation(h, t)).abs() < 1e-9);
        }
        // The raster uses the current sources: on their perpendicular bisector E = 2.
        for t in [0.0, 9.0, 20.0] {
            let g = scene(200, 50, t);
            assert!((g.sources[1].1 - g.sources[0].1 - separation(50.0, t)).abs() < 1e-9);
            // Pixel rows 49 and 50 straddle the bisector y = 25 symmetrically.
            let (d1, d2) = g.distances(WaveInterference::at(120, 49));
            let (e1, e2) = g.distances(WaveInterference::at(120, 50));
            assert!((d1 - e2).abs() < 1e-9 && (d2 - e1).abs() < 1e-9);
        }
    }

    #[test]
    fn wave_a6_a9_the_screen_is_a_profile_line_right_of_the_field() {
        let g = scene(120, 40, 0.0);
        let (x0, sw) = g.screen;
        assert_eq!(
            x0,
            g.field_w + SCREEN_GAP,
            "blank columns before the screen"
        );
        assert_eq!(x0 + sw, 120);
        assert!(sw >= 3);
        // ADR-0017: no bars or film; the strip holds only the profile curve (WAVE-A8), and
        // the gap columns are black.
        for py in 0..80 {
            for x in g.field_w..x0 + sw {
                assert_eq!(g.pixels.get(x as i32, py), Some(Ink::BLANK), "({x}, {py})");
            }
        }
        // Never in the title span.
        let (y, cols) = title::span(Size::new(120, 40), &Title::new(META.title, None)).unwrap();
        assert!(cols.end < x0 as i32 && y == 39);
    }

    #[test]
    fn wave_a7_crests_are_ice_on_black_and_sources_white_points() {
        let g = scene(120, 40, 1.0);
        let (mut ice, mut src) = (0, 0);
        for py in 0..80 {
            for px in 0..120 {
                match g.pixels.get(px, py).unwrap() {
                    p if p == SOURCE_INK => src += 1,
                    p if p.role == Role::Ramp(Ramp::Ice) => ice += 1,
                    p => assert_eq!(p, Ink::BLANK),
                }
            }
        }
        assert_eq!(src, 2, "two single-pixel sources");
        assert!(ice > 120 * 80 / 12, "{ice}");
        // Crests only: every lit field pixel is at most CREST_TOP, and troughs are black.
        assert_eq!(crest_ink(brightness(-1.0, 2.0, 1.0)), Ink::BLANK);
        assert_eq!(crest_ink(brightness(0.0, 2.0, 1.0)), Ink::BLANK);
        assert_eq!(crest_ink(brightness(2.0, 2.0, 1.0)).level, CREST_TOP as u8);
    }

    #[test]
    fn wave_a8_screen_profile_is_a_smooth_curve() {
        let g = scene(120, 40, 0.0);
        let (x0, sw) = g.screen;
        let film = (x0 + sw - 1) as i32;
        // Every profile dot lies in the screen, left of the film, in Ice at ≤ 180.
        let (mut far_bright, mut far_dark) = (i32::MAX, i32::MIN);
        let rows = 4 * 40;
        let mut per_row = 0;
        for y in 0..rows {
            let lit: Vec<i32> = (0..240).filter(|&x| g.profile.is_lit(x, y)).collect();
            if lit.is_empty() {
                continue;
            }
            per_row += 1;
            let (lo, hi) = (lit[0], lit[lit.len() - 1]);
            assert!(lo >= 2 * x0 as i32 && hi < 2 * film, "row {y}: {lit:?}");
            let i = g.intensity((f64::from(y) + 0.5) / 2.0);
            if i > 0.9 {
                far_bright = far_bright.min(lo);
            } else if i < 0.1 {
                far_dark = far_dark.max(lo);
            }
        }
        // One continuous curve: a dot on every dot row.
        assert_eq!(per_row, rows);
        // Bright rows reach further left than dark ones (the profile follows I).
        assert!(far_bright + 4 < far_dark, "{far_bright} vs {far_dark}");
        let mut c = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut c);
        for y in 0..40 {
            for x in x0 as i32..film {
                let cell = c.get(x, y).unwrap();
                if cell.ink.role != Role::Background {
                    assert_eq!(cell.ink.role, Role::Ramp(Ramp::Ice), "({x},{y})");
                    assert!(cell.ink.level <= 181, "({x},{y}) {:?}", cell.ink);
                }
            }
        }
    }
}
