//! Two coherent point sources and their interference pattern (SPEC-002 `wave_interference`).
//!
//! M18 (orbital lobes + nodal threads, GEN-R26): the instantaneous field
//! `a = 2 A(s̄)·sin(k s̄ − ωt)·cos(kΔ/2)` (`s̄` the mean distance to the sources, `Δ` the
//! path difference, `A = min(1, √(r₀/s̄))` the common 2-D cylindrical spreading; the sum
//! of two sines, WAVE-A1/A2) is a hydrogen-style stipple on the braille lattice: a dot is
//! lit with probability `F·P_MAX·(a/2)²` (`F` a display vignette) through a fixed per-dot
//! threshold (CORE-R107), `Hot` where `a > 0` and `Cold` where `a < 0`, so each crest and
//! trough is an orbital lobe that glides outward along the bright bands without shimmer.
//! The nodal hyperbolas `|Δ| = (n + ½)λ` are faint dotted `Dim` threads, and the sources
//! are `Sun` atoms (`Dots::stipple_puff`, CORE-R108).
//!
//! Cost: `resize` folds everything static into two bytes per dot. The dot's orbit phase
//! `k s̄` and the sign of its envelope share one `u8` (a negative envelope is half a turn
//! of phase), the envelope magnitude is a `u8`, and the dot's stipple threshold, already
//! divided by its envelope, is a `u8`. The field is mirror-symmetric about the line
//! between the sources, so phase and envelope are stored for half the rows. A frame is a
//! table lookup and a compare per dot: no trigonometry and no multiplies except for lit
//! dots' level.

use std::f64::consts::{PI, TAU};

use sciart_core::prelude::*;
use sciart_core::raster::stipple_keep;
use sciart_core::rng::{cell_key, hash01};

pub static META: Meta = Meta {
    name: "wave_interference",
    category: Category::Physics,
    description: "Two coherent sources whose ripples interfere into bright and dark bands",
    title: "Interference · crests add up",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(WaveInterference::default())
}

/// Angular frequency, rad/s.
const OMEGA: f64 = 3.0;
/// Source separation as a fraction of the height.
const SEPARATION: f64 = 0.35;
/// Peak keep probability of the stipple, reached where `|a| = 2`.
const P_MAX: f64 = 0.75;
/// Dots whose envelope `|cos(kΔ/2)|` is below this are never lit: it keeps a clear lane
/// of at least 0.05 λ on both sides of every nodal line (WAVE-A4: within 0.05 λ of a node
/// the envelope is at most `sin(0.1π) ≈ 0.31`).
const ENV_FLOOR: f64 = 0.35;
/// Crest cores (`|a| ≥ 1.4`) are drawn brighter.
const BRIGHT_A: f64 = 1.4;
pub(crate) const HOT: [Ink; 2] = [Ink::new(Role::Hot, 170), Ink::new(Role::Hot, 230)];
pub(crate) const COLD: [Ink; 2] = [Ink::new(Role::Cold, 170), Ink::new(Role::Cold, 230)];
pub(crate) const NODE_INK: Ink = Ink::new(Role::Dim, 70);
pub(crate) const SOURCE_INK: Ink = Ink::new(Role::Sun, 255);
/// Source atoms: Gaussian stipple, σ in dots, and peak keep probability (CORE-R108).
const SOURCE_SIGMA: f64 = 1.6;
const SOURCE_PEAK: f64 = 0.95;
/// Field dots within this many dots of a source are left dark around its atom (`3σ` and
/// a dot and a half).
const SOURCE_CLEAR: f64 = 3.0 * SOURCE_SIGMA + 1.5;
/// Nodal threads: one dot about every `NODE_GAP` dots of arc.
const NODE_GAP: f64 = 3.0;
const FIELD_SALT: u64 = 0x3A7E_0000_0000_0001;
const NODE_SALT: u64 = 0x3A7E_0000_0000_0003;
/// Cylindrical spreading: the amplitude is 1 within `R0_LAMBDAS` wavelengths of the
/// sources and falls as `√(r₀/s̄)` beyond (energy ∝ 1/r in 2-D).
const R0_LAMBDAS: f64 = 1.5;
/// Display fade of the stipple density (not the colour), a vignette about the sources'
/// midpoint: on the elliptical radius `ρ` that is 1 at the left, top and bottom frame
/// edges and `FADE_RIGHT` of the frame width right of the midpoint, the density is full
/// up to `FADE_FROM` and falls smoothly to 0 at `ρ = 1`, so the outer third of the frame
/// is nearly black and the interference rays fan out to the right.
const FADE_FROM: f64 = 0.55;
const FADE_RIGHT: f64 = 0.6;
const SOURCE_SALT: u64 = 0x3A7E_0000_0000_0002;
/// Magnitude scale of the sine table: `|sin|` as `0..=MAG_MAX`.
const MAG_MAX: f64 = 254.0;
/// Threshold that no dot reaches: the dot is never lit.
const NEVER: u8 = u8::MAX;

/// Wavelength in rows for a screen `h` rows tall.
pub(crate) fn wavelength(h: u16) -> f64 {
    (f64::from(h) / 6.0).max(3.0)
}

/// Superposed displacement of two unit waves at distances `d1`, `d2` (WAVE-A1, WAVE-A2).
/// Reference form; frames use the precomputed phase and envelope tables.
#[cfg(test)]
pub(crate) fn amplitude(d1: f64, d2: f64, k: f64, t: f64) -> f64 {
    (k * d1 - OMEGA * t).sin_det() + (k * d2 - OMEGA * t).sin_det()
}

/// Common amplitude of the two waves at mean distance `s̄` (dots), wavelength `λ`:
/// `min(1, √(r₀/s̄))`, `r₀ = R0_LAMBDAS·λ` (2-D cylindrical spreading). Common to both
/// waves, so `cos(kΔ/2)` and the nodal lines stay exact.
pub(crate) fn spreading(sbar: f64, lambda: f64) -> f64 {
    let r0 = R0_LAMBDAS * lambda;
    if sbar <= r0 { 1.0 } else { (r0 / sbar).sqrt() }
}

/// `1 − smoothstep(from, 1, ρ)`: 1 up to `from`, falling smoothly to 0 at `ρ = 1`.
fn fade(rho: f64) -> f64 {
    let x = ((rho - FADE_FROM) / (1.0 - FADE_FROM)).clamp(0.0, 1.0);
    1.0 - x * x * (3.0 - 2.0 * x)
}

/// The frame's phase index: `ωt` in 1/256 turns.
fn time_index(t: f64) -> u8 {
    ((OMEGA * t / TAU).rem_euclid(1.0) * 256.0).round() as u32 as u8
}

/// The static part of a dot, shared by the dot and its mirror image.
#[derive(Debug, Clone, Copy, Default)]
struct Wave {
    /// `k s̄` in 1/256 turns, plus half a turn where the envelope is negative.
    phase: u8,
    /// `|cos(kΔ/2)|` as `0..=255`.
    env: u8,
}

#[derive(Debug)]
pub struct WaveInterference {
    t: f64,
    /// Dot lattice size.
    dw: usize,
    dh: usize,
    /// Sources in dot coordinates.
    sources: [(f64, f64); 2],
    /// Wavelength in dots.
    lambda: f64,
    /// Phase and envelope of the rows `0..⌈dh/2⌉` (the others are their mirror images).
    waves: Vec<Wave>,
    /// Per dot: the stipple threshold on `|sin|·MAG_MAX`, divided by the envelope
    /// (`NEVER` below the visibility floor).
    thresholds: Vec<u8>,
    /// `|sin(π j/128)|·MAG_MAX`, the magnitude of the sine of phase index `j` mod 128.
    mag: [u8; 128],
    /// Nodal-thread dots, built per resize.
    nodes: Vec<(i32, i32)>,
    dots: Dots,
    dirty: bool,
}

impl Default for WaveInterference {
    fn default() -> Self {
        let mut mag = [0u8; 128];
        for (j, m) in mag.iter_mut().enumerate() {
            *m = (MAG_MAX * (PI * j as f64 / 128.0).sin_det()).round() as u8;
        }
        Self {
            t: 0.0,
            dw: 0,
            dh: 0,
            sources: [(0.0, 0.0); 2],
            lambda: 1.0,
            waves: Vec::new(),
            thresholds: Vec::new(),
            mag,
            nodes: Vec::new(),
            dots: Dots::default(),
            dirty: true,
        }
    }
}

impl WaveInterference {
    /// The display fade at dot `(x, y)` (see `FADE_FROM`).
    fn fade_at(&self, x: f64, y: f64) -> f64 {
        let [(mx, y1), (_, y2)] = self.sources;
        let my = 0.5 * (y1 + y2);
        let ax = if x < mx {
            mx.max(1.0)
        } else {
            FADE_RIGHT * self.dw as f64
        };
        let ay = (0.5 * self.dh as f64).max(1.0);
        fade(((x - mx) / ax).hypot((y - my) / ay))
    }

    /// Row of the stored half that holds dot row `y`.
    #[cfg(test)]
    fn half_row(&self, y: usize) -> usize {
        y.min(self.dh - 1 - y)
    }

    /// The field ink of dot `(x, y)` at phase index `tau`, or `None` when it is not lit.
    #[cfg(test)]
    fn field_ink(&self, x: usize, y: usize, tau: u8) -> Option<Ink> {
        let thr = self.thresholds[y * self.dw + x];
        let w = self.waves[self.half_row(y) * self.dw + x];
        lit_ink(thr, w, tau, &self.mag)
    }

    /// Amplitude of dot `(x, y)` at `t` from the stored tables (tests compare it with the
    /// two-sine sum).
    #[cfg(test)]
    fn table_amplitude(&self, x: usize, y: usize, t: f64) -> f64 {
        let w = self.waves[self.half_row(y) * self.dw + x];
        let idx = w.phase.wrapping_sub(time_index(t));
        let sign = if idx < 128 { 1.0 } else { -1.0 };
        let m = f64::from(self.mag[usize::from(idx & 127)]) / MAG_MAX;
        2.0 * f64::from(w.env) / 255.0 * sign * m
    }

    /// The two source atoms (WAVE-A5).
    fn draw_sources(sources: &[(f64, f64); 2], dots: &mut Dots) {
        for (i, &(x, y)) in sources.iter().enumerate() {
            let salt = SOURCE_SALT ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            dots.stipple_puff(x, y, SOURCE_SIGMA, SOURCE_PEAK, salt, SOURCE_INK);
        }
    }

    /// Dotted nodal hyperbolas `|d₁ − d₂| = (n + ½)λ` (WAVE-A4). With the sources on a
    /// vertical line, centre `(mx, my)` and half-separation `c`, the branch with
    /// `d₁ − d₂ = ∓D` is `y = my ∓ (D/2) cosh τ`, `x = mx + b sinh τ`, `b² = c² − D²/4`.
    fn build_nodes(&mut self) {
        self.nodes.clear();
        let [(mx, y1), (_, y2)] = self.sources;
        let (my, c) = ((y1 + y2) / 2.0, (y2 - y1) / 2.0);
        let (w, h) = (self.dw as f64, self.dh as f64);
        let cap = 4 * (self.dw + self.dh);
        let mut n = 0.0;
        loop {
            let d = (n + 0.5) * self.lambda;
            if d >= 2.0 * c {
                break;
            }
            n += 1.0;
            let (a, b) = (d / 2.0, (c * c - d * d / 4.0).sqrt());
            for side in [-1.0, 1.0] {
                for dir in [-1.0, 1.0] {
                    // From the vertex outward; the vertex is drawn once.
                    let mut tau: f64 = if dir < 0.0 { -NODE_GAP / b } else { 0.0 };
                    for _ in 0..cap {
                        let (sh, ch) = (tau.sinh(), tau.cosh());
                        let (x, y) = (mx + b * sh, my + side * a * ch);
                        if !(-1.0..=w).contains(&x) || !(-1.0..=h).contains(&y) {
                            break;
                        }
                        let (px, py) = (x.round() as i32, y.round() as i32);
                        // The threads fade with the field, through fixed thresholds.
                        if stipple_keep(NODE_SALT, px, py, self.fade_at(x, y)) {
                            self.nodes.push((px, py));
                        }
                        let speed = (a * a * sh * sh + b * b * ch * ch).sqrt();
                        tau += dir * NODE_GAP / speed;
                    }
                }
            }
        }
    }

    /// Nodal threads, the field stipple and the source atoms (SPEC-002 Look, M18).
    fn raster(&mut self) {
        let tau = time_index(self.t);
        let (dw, dh) = (self.dw, self.dh);
        let dots = &mut self.dots;
        dots.clear();
        for &(x, y) in &self.nodes {
            dots.set(x, y, NODE_INK);
        }
        // Row by row over slices, so the inner loop has no index arithmetic or bounds
        // checks. `NEVER` dots need no test of their own: no magnitude exceeds it, and a
        // separate branch on the hashed thresholds would be mispredicted at random.
        for (y, thr_row) in self.thresholds.chunks_exact(dw).enumerate() {
            let h = y.min(dh - 1 - y) * dw;
            let wave_row = &self.waves[h..h + dw];
            for (x, (&thr, &w)) in thr_row.iter().zip(wave_row).enumerate() {
                if let Some(ink) = lit_ink(thr, w, tau, &self.mag) {
                    dots.set(x as i32, y as i32, ink);
                }
            }
        }
        Self::draw_sources(&self.sources, dots);
    }
}

/// The ink of a field dot with threshold `thr` and wave `w` at phase index `tau`, or
/// `None` when it is not lit: `P_MAX·(a/2)² > u ⇔ |sin|·MAG_MAX > thr` (`thr` already
/// divided by the envelope; `NEVER` exceeds every magnitude), with the sign of `a` from
/// the phase (a negative envelope is folded into it as half a turn).
#[inline]
fn lit_ink(thr: u8, w: Wave, tau: u8, mag: &[u8; 128]) -> Option<Ink> {
    let idx = w.phase.wrapping_sub(tau);
    let m = mag[usize::from(idx & 127)];
    if m <= thr {
        return None;
    }
    // |a| ≥ BRIGHT_A ⇔ |sin|·env ≥ BRIGHT_A/2.
    let bright = u32::from(m) * u32::from(w.env) >= BRIGHT_PRODUCT;
    let inks = if idx < 128 { &HOT } else { &COLD };
    Some(inks[usize::from(bright)])
}

/// `|sin|·MAG_MAX · env·255` at `|a| = BRIGHT_A`.
const BRIGHT_PRODUCT: u32 = (BRIGHT_A / 2.0 * MAG_MAX * 255.0) as u32;

impl Generator for WaveInterference {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.dots.resize(size);
        let d = self.dots.dot_size();
        (self.dw, self.dh) = (usize::from(d.width()), usize::from(d.height()));
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        // Rows → dots: a row is `CELL_H` dots, and dots are square.
        self.lambda = wavelength(size.height()) * f64::from(Dots::CELL_H);
        let ys = [
            h / 2.0 - SEPARATION * h / 2.0,
            h / 2.0 + SEPARATION * h / 2.0,
        ];
        self.sources = ys.map(|y| Dots::from_cells(0.25 * w, y));
        let [(sx, sy1), (_, sy2)] = self.sources;
        let half = self.dh.div_ceil(2);
        self.waves.clear();
        self.waves.resize(half * self.dw, Wave::default());
        // Per half-row dot: the exact envelope, kept for the thresholds below.
        self.thresholds.clear();
        self.thresholds.resize(self.dw * self.dh, NEVER);
        let inv_lambda = 1.0 / self.lambda;
        for y in 0..half {
            let fy = y as f64;
            let (dy1, dy2) = ((fy - sy1).powi(2), (fy - sy2).powi(2));
            let mirror = self.dh - 1 - y;
            for x in 0..self.dw {
                let dx = (x as f64 - sx).powi(2);
                let (d1, d2) = ((dx + dy1).sqrt(), (dx + dy2).sqrt());
                let env = (PI * (d1 - d2) * inv_lambda).cos_det();
                let turns = 0.5 * (d1 + d2) * inv_lambda + if env < 0.0 { 0.5 } else { 0.0 };
                let phase = ((turns.rem_euclid(1.0) * 256.0).round() as u32) as u8;
                let cos = env.abs();
                let sbar = 0.5 * (d1 + d2);
                // The stored envelope carries the spreading: |a|/2 = A(s̄)·|cos(kΔ/2)|·|sin|.
                let env = spreading(sbar, self.lambda) * cos;
                self.waves[y * self.dw + x] = Wave {
                    phase,
                    env: (env * 255.0).round() as u8,
                };
                // The vignette is mirror-symmetric too.
                let keep = P_MAX * self.fade_at(x as f64, fy);
                if cos < ENV_FLOOR || keep <= 0.0 {
                    continue;
                }
                // Lit ⇔ P_MAX·F·(a/2)² > u ⇔ |sin| > √(u/(P_MAX·F))/(A·|cos|).
                for yy in [y, mirror] {
                    let u = hash01(FIELD_SALT ^ cell_key(x as i32, yy as i32));
                    let thr = MAG_MAX * (u / keep).sqrt() / env;
                    self.thresholds[yy * self.dw + x] =
                        if thr >= MAG_MAX { NEVER } else { thr as u8 };
                }
            }
        }
        // A little black around each source, so the atom reads as a nucleus rather than
        // a bright speck inside a lobe.
        for &(sx, sy) in &self.sources {
            let r = SOURCE_CLEAR;
            let (y0, y1) = ((sy - r).ceil().max(0.0), (sy + r).floor());
            let (x0, x1) = ((sx - r).ceil().max(0.0), (sx + r).floor());
            for y in y0 as usize..=(y1 as usize).min(self.dh - 1) {
                for x in x0 as usize..=(x1 as usize).min(self.dw - 1) {
                    if (x as f64 - sx).hypot(y as f64 - sy) <= r {
                        self.thresholds[y * self.dw + x] = NEVER;
                    }
                }
            }
        }
        self.build_nodes();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: f64 = TAU / 5.0;

    fn built(w: u16, h: u16) -> WaveInterference {
        let size = Size::new(w, h);
        let mut g = WaveInterference::default();
        g.resize(size, &mut Ctx::new(1, size));
        g.prepare();
        g
    }

    /// Exact `(a, Δ)` at dot `(x, y)` and time `t`.
    fn exact(g: &WaveInterference, x: usize, y: usize, t: f64) -> (f64, f64) {
        let k = TAU / g.lambda;
        let p = (x as f64, y as f64);
        let d = |s: (f64, f64)| (p.0 - s.0).hypot(p.1 - s.1);
        let (d1, d2) = (d(g.sources[0]), d(g.sources[1]));
        let a = spreading(0.5 * (d1 + d2), g.lambda) * amplitude(d1, d2, k, t);
        (a, d1 - d2)
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
    fn wave_a1_a2_tables_match_the_two_sine_sum() {
        // The two-byte tables are the sum of two sines up to their quantisation (1/256
        // turn of phase, 1/255 of envelope), everywhere, on the bisector and off it.
        for (w, h) in [(90u16, 30u16), (200, 50), (13, 7)] {
            let g = built(w, h);
            for t in [0.0, 0.37, 1.234, 2.0, 57.3] {
                for y in (0..g.dh).step_by(3) {
                    for x in (0..g.dw).step_by(5) {
                        let (a, _) = exact(&g, x, y, t);
                        let got = g.table_amplitude(x, y, t);
                        assert!(
                            (got - a).abs() < 0.06,
                            "{w}x{h} ({x},{y}) t={t}: {got} vs {a}"
                        );
                    }
                }
            }
            // The mirror rows share one stored row and are exact mirror images.
            let (x, y) = (g.dw / 3, 1);
            let (a0, _) = exact(&g, x, y, 0.7);
            let (a1, _) = exact(&g, x, g.dh - 1 - y, 0.7);
            assert!((a0 - a1).abs() < 1e-9);
        }
    }

    #[test]
    fn wave_a3_lit_share_rises_with_a_squared_and_sign_sets_the_ink() {
        let g = built(200, 50);
        let (mut hi, mut hi_n, mut mid, mut mid_n) = (0usize, 0usize, 0usize, 0usize);
        let mut hi_p = 0.0;
        for t in [0.0, 0.5, 1.3] {
            let tau = time_index(t);
            for y in 0..g.dh {
                for x in 0..g.dw {
                    let (a, _) = exact(&g, x, y, t);
                    let ink = g.field_ink(x, y, tau);
                    if let Some(ink) = ink {
                        if a.abs() > 0.05 {
                            let want = if a > 0.0 { Role::Hot } else { Role::Cold };
                            assert_eq!(ink.role, want, "({x},{y}) a={a}");
                        }
                        let bright = ink.level == 230;
                        if (a.abs() - BRIGHT_A).abs() > 0.05 {
                            assert_eq!(bright, a.abs() >= BRIGHT_A, "({x},{y}) a={a}");
                        }
                    }
                    // The density law is measured where the display fade is 1 and outside
                    // the dark zone around each source.
                    let p = (x as f64, y as f64);
                    let d = |s: (f64, f64)| (p.0 - s.0).hypot(p.1 - s.1);
                    let (d1, d2) = (d(g.sources[0]), d(g.sources[1]));
                    if g.fade_at(p.0, p.1) < 1.0 || d1.min(d2) <= SOURCE_CLEAR {
                        continue;
                    }
                    let lit = usize::from(ink.is_some());
                    if a.abs() > 1.85 {
                        (hi, hi_n) = (hi + lit, hi_n + 1);
                        hi_p += P_MAX * (a / 2.0).powi(2);
                    } else if (a.abs() - 0.7).abs() < 0.1 {
                        (mid, mid_n) = (mid + lit, mid_n + 1);
                    }
                }
            }
        }
        let (hi, mid) = (hi as f64 / hi_n as f64, mid as f64 / mid_n as f64);
        assert!(
            hi >= 3.0 * mid,
            "lit share {hi:.3} at |a|≈2 vs {mid:.3} at |a|≈0.7"
        );
        // The lit share is the density P_MAX·(a/2)² (up to sampling noise).
        let want = hi_p / hi_n as f64;
        assert!(
            (hi - want).abs() < 0.05,
            "density {hi:.3} vs P_MAX·(a/2)² {want:.3}"
        );
    }

    #[test]
    fn wave_a4_nodal_lanes_stay_dark_and_threads_lie_on_the_hyperbolas() {
        for (w, h) in [(200u16, 50u16), (80, 24)] {
            let g = built(w, h);
            let lambda = g.lambda;
            // Within 0.05 λ of a nodal line: ||Δ| − (n + ½)λ| ≤ 2·0.05 λ (|∇Δ| ≤ 2).
            let near_node = |delta: f64| {
                let f = (delta.abs() / lambda - 0.5).rem_euclid(1.0);
                f.min(1.0 - f) <= 0.1
            };
            for t in [0.0, 0.3, 0.9, 4.1] {
                let tau = time_index(t);
                for y in 0..g.dh {
                    for x in 0..g.dw {
                        let (_, delta) = exact(&g, x, y, t);
                        if near_node(delta) {
                            assert!(g.field_ink(x, y, tau).is_none(), "({x},{y})");
                        }
                    }
                }
            }
            // Each thread dot is within a dot of `|Δ| = (n + ½)λ`: the curve crosses its
            // 3×3 neighbourhood.
            assert!(!g.nodes.is_empty());
            let gap = |x: f64, y: f64| {
                let d = |s: (f64, f64)| (x - s.0).hypot(y - s.1);
                let delta = (d(g.sources[0]) - d(g.sources[1])).abs() / lambda - 0.5;
                delta - delta.round()
            };
            for &(x, y) in &g.nodes {
                let (x, y) = (f64::from(x), f64::from(y));
                let (mut lo, mut hi) = (f64::MAX, f64::MIN);
                for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let v = gap(x + dx, y + dy);
                    (lo, hi) = (lo.min(v), hi.max(v));
                }
                assert!(lo <= 0.0 && hi >= 0.0, "({x},{y}) off its hyperbola");
            }
            // n = 0 and n = 1 at s = 2.1 λ: four branches, fading out with the field.
            assert!(g.nodes.len() > g.dw / 4, "{} thread dots", g.nodes.len());
        }
    }

    #[test]
    fn wave_a5_each_source_is_a_sun_atom_on_its_position() {
        let g = built(200, 50);
        let mut dots = Dots::new(Size::new(200, 50));
        WaveInterference::draw_sources(&g.sources, &mut dots);
        for (i, &(sx, sy)) in g.sources.iter().enumerate() {
            let (mut n, mut cx, mut cy) = (0.0, 0.0, 0.0);
            let reach = (3.0 * SOURCE_SIGMA).ceil() as i32 + 1;
            for y in sy as i32 - reach..=sy as i32 + reach {
                for x in sx as i32 - reach..=sx as i32 + reach {
                    if dots.is_lit(x, y) {
                        (n, cx, cy) = (n + 1.0, cx + f64::from(x), cy + f64::from(y));
                    }
                }
            }
            assert!(n >= 3.0, "atom of {n} dots");
            assert!(
                (cx / n - sx).hypot(cy / n - sy) <= 1.0,
                "centroid off source {i}"
            );
        }
        // In the frame the source cells carry the atom's ink.
        let mut c = Canvas::new(Size::new(200, 50), GlyphMode::Unicode);
        g.render(&mut c);
        for &(sx, sy) in &g.sources {
            let cell = c.get((sx / 2.0 + 0.25) as i32, (sy / 4.0 + 0.125) as i32);
            assert_eq!(cell.map(|c| c.ink), Some(SOURCE_INK));
        }
    }

    #[test]
    fn wave_a6_frames_are_braille_only() {
        for (w, h) in [(200u16, 50u16), (80, 24)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(42, size);
            let mut g = WaveInterference::default();
            g.resize(size, &mut ctx);
            for _ in 0..90 {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            let mut c = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut c);
            let mut lit = 0;
            for cell in c.cells() {
                assert_eq!(cell.bg, Ink::BLANK);
                if cell.ch != ' ' {
                    assert!(
                        ('\u{2801}'..='\u{28FF}').contains(&cell.ch),
                        "{:?}",
                        cell.ch
                    );
                    lit += 1;
                }
            }
            assert!(lit > c.cells().len() / 10, "{lit} lit cells");
        }
    }
}
