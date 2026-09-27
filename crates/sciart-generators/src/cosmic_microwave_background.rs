//! A Planck-style oval sky map of CMB temperature anisotropies (SPEC-002
//! `cosmic_microwave_background`).
//!
//! The sky is an isotropic Gaussian random field synthesised from `MODES` plane waves
//! (spectral synthesis). The mode wavenumbers are importance-sampled from a CMB-like
//! angular power spectrum (Sachs–Wolfe plateau, acoustic peaks, damping tail, times a
//! Gaussian "beam" the size of a terminal cell), so every mode carries equal power and
//! the field is as Gaussian as `MODES` terms allow; directions follow a jittered
//! golden-ratio sequence over `[0, π)` and phases are uniform, which makes the field
//! statistically isotropic at every scale.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{draw, ink, raster};

pub static META: Meta = Meta {
    name: "cosmic_microwave_background",
    category: Category::DeepCosmos,
    description: "A shimmering Planck-style map of the CMB anisotropies",
    title: "Oldest light · early sound waves",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CosmicMicrowaveBackground::default())
}

/// Number of plane-wave modes. 96 equal-power modes give a visibly Gaussian, isotropic
/// field (CMB-A4) while one step+render stays ≈ 0.3 ms at 200×50 in release (GEN-R8).
const MODES: usize = 96;
/// Fractional part of the golden ratio, the most irrational rotation (direction sequence).
const GOLDEN: f64 = 0.618_033_988_749_894_9;
/// Fraction of the screen (per axis) covered by the sky oval.
const OVAL: f64 = 0.94;
/// Independent accumulators in the per-cell dot product: enough to hide FP-add latency
/// and fill the SIMD lanes (2·MODES must be a multiple).
const LANES: usize = 16;
/// Multipole of the first acoustic peak and the screen wavelength (rows) it maps to.
/// The real sky's ~1° spots cannot be resolved by a terminal, so the spectrum is mapped
/// to cell scales: ℓ ↦ k = (2π / PEAK_WAVELENGTH)·ℓ / L_PEAK rad/row.
const L_PEAK: f64 = 220.0;
const PEAK_WAVELENGTH: f64 = 6.0;
/// Multipole range that is synthesised. Below `L_MIN` the wavelength (> 60 rows) exceeds
/// any sensible map; above `L_MAX` it (< 1.3 rows) nears the ½-row pixel grid's Nyquist
/// scale (1 row).
const L_MIN: f64 = 10.0;
const L_MAX: f64 = 1000.0;
/// Gaussian beam `B²(ℓ) = exp(−(ℓ/L_BEAM)²)`: the half-block pixel is our telescope's
/// beam. It keeps the first peak (B² ≈ 0.84) and the second (≈ 0.34), so the spots carry
/// sub-structure, and rolls off towards `L_MAX` (≈ 0.025), as Planck's beam rolls off its
/// damping tail.
const L_BEAM: f64 = 520.0;
/// Approximate ΛCDM temperature spectrum `D_ℓ = ℓ(ℓ+1)C_ℓ/2π` in μK², read off the
/// Planck 2018 best fit at the plateau, the first three peaks and the troughs (±10 %).
/// Interpolated linearly in `ln ℓ`.
const SPECTRUM: [(f64, f64); 14] = [
    (2.0, 1000.0),
    (10.0, 1100.0),
    (30.0, 1350.0),
    (60.0, 1750.0),
    (100.0, 2800.0),
    (150.0, 4500.0),
    (220.0, 5750.0),
    (300.0, 4000.0),
    (350.0, 2700.0),
    (415.0, 1750.0),
    (540.0, 2550.0),
    (675.0, 1850.0),
    (815.0, 2450.0),
    (1000.0, 1150.0),
];
/// Temporal angular frequency range (rad/s, random sign): the field decorrelates over a
/// few seconds, a slow shimmer rather than per-frame flicker (GEN-R9 still sees motion).
const OMEGA: (f64, f64) = (0.08, 0.35);
/// Display saturation in units of the field's σ. Planck's map colour bar spans ±300 μK
/// for a CMB rms of ≈ 110 μK, i.e. ≈ ±2.7 σ.
const SATURATION_SIGMA: f64 = 2.7;
/// ASCII stand-ins for a sky pixel pair, by contrast.
const ASCII_RAMP: [char; 4] = ['.', ':', '*', '#'];
/// The spectrum inset: largest size in cells (its last row holds the peak's label), the
/// smallest canvas that shows it, its multipole range, and its levels.
const INSET_MAX: (i32, i32) = (32, 8);
const INSET_MIN_CANVAS: (i32, i32) = (80, 20);
const INSET_MIN_ROWS: i32 = 5;
const INSET_ELL: (f64, f64) = (10.0, 1000.0);
const INSET_AXIS_LEVEL: u8 = 60;
const INSET_CURSOR_LEVEL: u8 = 110;
const INSET_CURVE_LEVEL: u8 = 190;
/// Seconds for the inset's cursor to sweep the ℓ axis once, reading the spectrum off
/// scale by scale; it is what moves in the inset while the sky shimmers.
const SWEEP_PERIOD: f64 = 8.0;
const PEAK_LABEL: (&str, &str) = ("ℓ≈220", "l~220");
const READOUT: &str = "CMB  T=2.725K  dT~1e-5";

/// Angular power spectrum `D_ℓ` (μK²) of the CMB-like model (CMB-A5).
pub(crate) fn d_ell(l: f64) -> f64 {
    let ln = l.max(SPECTRUM[0].0).ln();
    for pair in SPECTRUM.windows(2) {
        let ((l0, d0), (l1, d1)) = (pair[0], pair[1]);
        if ln <= l1.ln() {
            let s = (ln - l0.ln()) / (l1.ln() - l0.ln());
            return d0 + s * (d1 - d0);
        }
    }
    SPECTRUM[SPECTRUM.len() - 1].1
}

/// Field variance per unit `ln ℓ` after the beam: in 2-D, `D_ℓ ∝ ℓ² C_ℓ` is exactly the
/// power per logarithmic interval, so this is the density the modes are drawn from.
fn power_per_ln_ell(l: f64) -> f64 {
    d_ell(l) * (-(l / L_BEAM).powi(2)).exp()
}

/// Screen wavenumber (rad/row) for multipole `ℓ`.
pub(crate) fn wavenumber(l: f64) -> f64 {
    TAU / PEAK_WAVELENGTH * l / L_PEAK
}

/// `MODES` multipoles by stratified inverse-CDF sampling of `power_per_ln_ell` over
/// `[L_MIN, L_MAX]`: equal-power modes whose density in `ln ℓ` follows the spectrum.
fn sample_multipoles(rng: &mut impl Rng) -> [f64; MODES] {
    const GRID: usize = 512;
    let (a, b) = (L_MIN.ln(), L_MAX.ln());
    let step = (b - a) / GRID as f64;
    let mut cdf = [0.0; GRID + 1];
    for i in 0..GRID {
        let (l0, l1) = (
            (a + step * i as f64).exp(),
            (a + step * (i + 1) as f64).exp(),
        );
        cdf[i + 1] = cdf[i] + 0.5 * (power_per_ln_ell(l0) + power_per_ln_ell(l1)) * step;
    }
    let total = cdf[GRID];
    let mut out = [0.0; MODES];
    for (j, l) in out.iter_mut().enumerate() {
        let u = (j as f64 + rng.random::<f64>()) / MODES as f64 * total;
        let i = cdf.partition_point(|&c| c < u).clamp(1, GRID);
        let frac = (u - cdf[i - 1]) / (cdf[i] - cdf[i - 1]).max(f64::MIN_POSITIVE);
        *l = (a + step * ((i - 1) as f64 + frac.clamp(0.0, 1.0))).exp();
    }
    out
}

/// One plane-wave mode `sin(k·(x, y) + φ + ωt)`; `k` in rad/row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mode {
    kx: f64,
    ky: f64,
    phase: f64,
    omega: f64,
}

/// `MODES` modes: wavenumbers from the spectrum, directions over `[0, π)` (a random phase
/// makes `θ` and `θ + π` equivalent), uniform phases, and `ω` of random sign so the
/// pattern boils in place instead of drifting one way.
fn random_modes(rng: &mut impl Rng) -> Vec<Mode> {
    let ells = sample_multipoles(rng);
    // Golden-ratio (Kronecker) sequence over the ℓ-sorted modes, jittered within 1/N:
    // every contiguous band of scales then covers [0, π) evenly, so the correlation
    // function is isotropic scale by scale, not only on average (CMB-A4).
    let offset: f64 = rng.random();
    let angles: Vec<f64> = (0..MODES)
        .map(|j| {
            let u = (offset + j as f64 * GOLDEN + rng.random::<f64>() / MODES as f64).fract();
            PI * u
        })
        .collect();
    ells.iter()
        .zip(angles)
        .map(|(&l, dir)| {
            let k = wavenumber(l);
            let omega = rng.random_range(OMEGA.0..=OMEGA.1);
            Mode {
                kx: k * dir.cos(),
                ky: k * dir.sin(),
                phase: rng.random_range(0.0..TAU),
                omega: if rng.random::<bool>() { omega } else { -omega },
            }
        })
        .collect()
}

/// Normalized temperature contrast `f = (1/N) Σ sin(kᵢ·(x_rows, y) + φᵢ + ωᵢt)` (CMB-A1:
/// the mean of N sines, so `|f| ≤ 1` exactly). Reference form; `render` evaluates the
/// same sum separably (checked by `cmb_separable_render_matches_field_value`).
#[cfg(test)]
pub(crate) fn field_value(modes: &[Mode], x_rows: f64, y: f64, t: f64) -> f64 {
    if modes.is_empty() {
        return 0.0;
    }
    modes
        .iter()
        .map(|m| (m.kx * x_rows + m.ky * y + m.phase + m.omega * t).sin())
        .sum::<f64>()
        / modes.len() as f64
}

/// Largest possible change of the field between horizontally adjacent cells (CMB-A2):
/// `|∂f/∂x_rows| ≤ Σ|kₓᵢ| / N`, and adjacent cells are `1/ASPECT` rows apart.
#[cfg(test)]
pub(crate) fn smoothness_bound(modes: &[Mode]) -> f64 {
    modes.iter().map(|m| m.kx.abs()).sum::<f64>() / (ASPECT * modes.len() as f64)
}

#[derive(Debug, Default)]
pub struct CosmicMicrowaveBackground {
    t: f64,
    modes: Vec<Mode>,
    /// Per column `x`, `2·MODES` values: `sin(kxᵢ·x)` for all i, then `cos(kxᵢ·x)`.
    cols: Vec<f64>,
    /// Per pixel row, `2·MODES` values: `cos(kyᵢ·y + φᵢ)` for all i, then `sin(…)`.
    rows: Vec<f64>,
    /// `σ` of the field for random phases: `1/√(2N)`.
    sigma: f64,
    /// The oval in cell coordinates (centre and semi-axes; y in rows).
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    height: u16,
    readout: bool,
    /// The sky map, and the spectrum inset drawn with dots.
    pixels: Pixels,
    dots: Dots,
    inset: Option<Inset>,
}

/// The spectrum inset's cells: top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Inset {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Row coordinate (in rows) of the centre of pixel row `py`: cell row `y` spans
/// `[y − ½, y + ½)` around the cell's own coordinate `y`.
fn pixel_row(py: usize) -> f64 {
    py as f64 / 2.0 - 0.25
}

/// ASCII stand-in for a sky pixel pair: a density char by the stronger pixel's contrast.
fn ascii_pixel(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = raster::stronger(top, bottom);
    (ink.role != Role::Background).then(|| {
        let i = usize::from(ink.level.abs_diff(128)) * ASCII_RAMP.len() / 129;
        (ASCII_RAMP[i.min(ASCII_RAMP.len() - 1)], ink)
    })
}

impl CosmicMicrowaveBackground {
    /// `[cos B, sin B]` with `B = ky·y + φ + ωt` for every mode on pixel row `py`, by
    /// rotating the per-resize row table by `ωt` (no trig per row).
    fn row_vector(&self, py: usize, rot: &[(f64, f64); MODES]) -> [f64; 2 * MODES] {
        let r = &self.rows[py * 2 * MODES..(py + 1) * 2 * MODES];
        let mut out = [0.0; 2 * MODES];
        for i in 0..MODES {
            let (c0, s0) = (r[i], r[MODES + i]);
            let (st, ct) = rot[i];
            out[i] = c0 * ct - s0 * st;
            out[MODES + i] = s0 * ct + c0 * st;
        }
        out
    }

    /// `(sin ωᵢt, cos ωᵢt)` for the current time.
    fn rotation(&self) -> [(f64, f64); MODES] {
        let mut rot = [(0.0, 1.0); MODES];
        for (r, m) in rot.iter_mut().zip(&self.modes) {
            *r = (m.omega * self.t).sin_cos();
        }
        rot
    }

    /// Field at column `x`: sin(A + B) = sin A cos B + cos A sin B, i.e. a dot product of
    /// the column and row vectors (`LANES` accumulators so it vectorises).
    fn cell_value(&self, x: usize, row: &[f64; 2 * MODES]) -> f64 {
        let col = &self.cols[x * 2 * MODES..(x + 1) * 2 * MODES];
        let mut acc = [0.0; LANES];
        for (c, r) in col.chunks_exact(LANES).zip(row.chunks_exact(LANES)) {
            for k in 0..LANES {
                acc[k] += c[k] * r[k];
            }
        }
        acc.iter().sum::<f64>() / MODES as f64
    }

    /// Whether cell point `(x, y)` (y in rows) lies inside the sky oval.
    fn in_oval(&self, x: f64, y: f64) -> bool {
        let (dx, dy) = ((x - self.cx) / self.rx, (y - self.cy) / self.ry);
        dx * dx + dy * dy <= 1.0
    }

    /// The columns of row `y` (in rows) that lie inside the oval, the same cells
    /// `in_oval` accepts (up to rounding on the rim), found once per row.
    fn oval_span(&self, y: f64, w: usize) -> std::ops::Range<usize> {
        let dy = (y - self.cy) / self.ry;
        let half = self.rx * (1.0 - dy * dy).sqrt();
        if half.is_nan() {
            return 0..0;
        }
        let lo = (self.cx - half).ceil().max(0.0) as usize;
        let hi = ((self.cx + half).floor() + 1.0).clamp(0.0, w as f64) as usize;
        lo.min(hi)..hi
    }

    /// The largest inset (4 : 1 in cells, down to `INSET_MIN_ROWS` rows) that fits the
    /// top-right corner wholly outside the oval, or `None` (CMB-A7).
    fn fit_inset(&self, size: Size) -> Option<Inset> {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        if w < INSET_MIN_CANVAS.0 || h < INSET_MIN_CANVAS.1 {
            return None;
        }
        (INSET_MIN_ROWS..=INSET_MAX.1.min(h / 5))
            .rev()
            .find_map(|ih| {
                let iw = (4 * ih).min(INSET_MAX.0).min(w / 5);
                let inset = Inset {
                    x: w - iw - 1,
                    y: 1,
                    w: iw,
                    h: ih,
                };
                // The corner nearest the oval, the bottom-left one, must lie outside it.
                let corner = (
                    f64::from(inset.x) - 0.5,
                    f64::from(inset.y + inset.h - 1) + 0.5,
                );
                (!self.in_oval(corner.0, corner.1)).then_some(inset)
            })
    }

    /// Shade the sky map for the current time (the M10 raster convention: `render` only
    /// blits).
    fn shade(&mut self) {
        let w = self.cols.len() / (2 * MODES);
        let full_scale = SATURATION_SIGMA * self.sigma;
        let rot = self.rotation();
        for py in 0..2 * usize::from(self.height) {
            let span = self.oval_span(pixel_row(py), w);
            if span.is_empty() {
                continue;
            }
            let row = self.row_vector(py, &rot);
            for x in span {
                let v = self.cell_value(x, &row) / full_scale;
                self.pixels.set(x as i32, py as i32, ink::div(v));
            }
        }
    }

    /// The model's input spectrum `D_ℓ` over `ℓ ∈ [10, 1000]` on a log axis above a dim
    /// baseline with decade ticks, the first peak marked, and a cursor sweeping `ℓ`
    /// (CMB-A7). The inset's last row is left to the peak's label.
    fn draw_inset(&mut self) {
        self.dots.clear();
        let Some(inset) = self.inset else { return };
        let (dw, dh) = (inset.w * Dots::CELL_W, (inset.h - 1) * Dots::CELL_H);
        let (x0, y0) = (inset.x * Dots::CELL_W, inset.y * Dots::CELL_H);
        let (lo, hi) = (INSET_ELL.0.ln(), INSET_ELL.1.ln());
        let top = SPECTRUM.iter().map(|&(_, d)| d).fold(0.0, f64::max);
        let to_x = |l: f64| f64::from(x0) + (l.ln() - lo) / (hi - lo) * f64::from(dw - 1);
        let to_y = |d: f64| f64::from(y0 + dh - 2) - d / top * f64::from(dh - 3);
        let axis = f64::from(y0 + dh - 1);
        let dim = Ink::new(Role::Dim, INSET_AXIS_LEVEL);
        self.dots
            .line(f64::from(x0), axis, f64::from(x0 + dw - 1), axis, dim);
        for l in [10.0, 100.0, 1000.0] {
            self.dots.plot(to_x(l), axis - 1.0, dim);
        }
        let s = (self.t / SWEEP_PERIOD).rem_euclid(1.0);
        let cursor = (lo + (hi - lo) * s).exp();
        let (cx, cy) = (to_x(cursor), to_y(d_ell(cursor)));
        let dotted = Ink::new(Role::Dim, INSET_CURSOR_LEVEL);
        for y in (y0..y0 + dh - 1).step_by(2) {
            if f64::from(y) > cy + 1.0 {
                self.dots.plot(cx, f64::from(y), dotted);
            }
        }
        let curve = Ink::new(Role::Ramp(Ramp::Plasma), INSET_CURVE_LEVEL);
        let mut prev = (to_x(INSET_ELL.0), to_y(d_ell(INSET_ELL.0)));
        for i in 1..dw {
            let l = (lo + (hi - lo) * f64::from(i) / f64::from(dw - 1)).exp();
            let p = (to_x(l), to_y(d_ell(l)));
            self.dots.line(prev.0, prev.1, p.0, p.1, curve);
            prev = p;
        }
        let peak = Ink::new(Role::Ramp(Ramp::Plasma), 255);
        let (px, py) = (to_x(L_PEAK), to_y(d_ell(L_PEAK)));
        for (dx, dy) in [(0.0, -1.0), (-1.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0)] {
            self.dots.plot(px + dx, py + dy, peak);
        }
        self.dots.plot(cx, cy - 1.0, peak);
    }

    /// Column of the peak label's first char: centred under the peak, inside the inset.
    fn peak_label_x(inset: Inset) -> i32 {
        let t = (L_PEAK.ln() - INSET_ELL.0.ln()) / (INSET_ELL.1.ln() - INSET_ELL.0.ln());
        let n = PEAK_LABEL.0.chars().count() as i32;
        let centre = inset.x + (t * f64::from(inset.w)) as i32;
        (centre - n / 2).clamp(inset.x, (inset.x + inset.w - n).max(inset.x))
    }
}

impl Generator for CosmicMicrowaveBackground {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.modes.is_empty() {
            // The sky pattern survives resizes; only the viewport changes.
            self.modes = random_modes(&mut ctx.rng);
            // Independent uniform phases: σ² = N·(1/2)/N² = 1/(2N).
            self.sigma = (0.5 / MODES as f64).sqrt();
        }
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        self.cols.clear();
        for x in 0..w {
            let xr = x as f64 / ASPECT;
            let start = self.cols.len();
            self.cols.resize(start + 2 * MODES, 0.0);
            for (i, m) in self.modes.iter().enumerate() {
                (self.cols[start + i], self.cols[start + MODES + i]) = (m.kx * xr).sin_cos();
            }
        }
        self.rows.clear();
        for py in 0..2 * h {
            let start = self.rows.len();
            self.rows.resize(start + 2 * MODES, 0.0);
            for (i, m) in self.modes.iter().enumerate() {
                let (s, c) = (m.ky * pixel_row(py) + m.phase).sin_cos();
                (self.rows[start + i], self.rows[start + MODES + i]) = (c, s);
            }
        }
        self.height = size.height();
        let room = title::readout_room(size, &Title::of(self));
        self.readout = size.height() > 6 && READOUT.len() <= room;
        let sky_rows = f64::from(size.height()) - if self.readout { 1.0 } else { 0.0 };
        self.cx = (w as f64 - 1.0) / 2.0;
        self.cy = (sky_rows - 1.0) / 2.0;
        self.rx = (OVAL * w as f64 / 2.0).max(0.5);
        self.ry = (OVAL * sky_rows / 2.0).max(0.5);
        self.pixels.resize(size);
        self.dots.resize(size);
        self.inset = self.fit_inset(size);
        self.shade();
        self.draw_inset();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.shade();
        self.draw_inset();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_pixel(top, bottom));
        self.dots.blit(canvas);
        if let Some(inset) = self.inset {
            let ink = Ink::new(Role::Ramp(Ramp::Plasma), INSET_CURVE_LEVEL);
            let (x, y) = (Self::peak_label_x(inset), inset.y + inset.h - 1);
            draw::text_mode(canvas, x, y, PEAK_LABEL.0, PEAK_LABEL.1, ink);
        }
        // Bottom-right, clear of the title (GEN-R13); dropped when the row is too narrow.
        if self.readout {
            let y = i32::from(self.height) - 1;
            let ink = Ink::new(Role::Text, title::READOUT_LEVEL);
            draw::label_right(canvas, y, READOUT, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CMB-A4 tolerance on the spread of the ensemble correlation `Σ cos(kᵢ·r)/N` (the
    /// domain-averaged two-point function, free of finite-screen noise) across the lag
    /// directions 0°, 90°, ±45° at |r| = 1..6 rows. With N = 96 modes some anisotropy is
    /// unavoidable: measured max 0.062 over seeds 0..64 (golden-ratio directions; randomly
    /// shuffled strata gave 0.16).
    const ENSEMBLE_ISOTROPY_TOL: f64 = 0.1;
    /// CMB-A4 tolerance on |ρ_h − ρ_v| sampled on one 200×50 screen at r = 1..3 rows:
    /// measured max 0.051 over 64 seeds; the old 6-mode field reached 0.109 on seed 0.
    const SAMPLE_ISOTROPY_TOL: f64 = 0.1;

    /// Domain-averaged correlation of the field at a lag in rows: with independent phases,
    /// `⟨f(x) f(x+r)⟩ = Σ cos(kᵢ·r) / 2N²` and `σ² = 1/(2N)`.
    fn ensemble_correlation(modes: &[Mode], lag_rows: (f64, f64)) -> f64 {
        modes
            .iter()
            .map(|m| (m.kx * lag_rows.0 + m.ky * lag_rows.1).cos())
            .sum::<f64>()
            / modes.len() as f64
    }

    fn scene(seed: u64, w: u16, h: u16) -> CosmicMicrowaveBackground {
        let size = Size::new(w, h);
        let mut g = CosmicMicrowaveBackground::default();
        g.resize(size, &mut Ctx::new(seed, size));
        g
    }

    /// Sample correlation of the field at a lag of `lag_cells` over the 200×50 screen.
    fn correlation(modes: &[Mode], lag_cells: (usize, usize), t: f64) -> f64 {
        let (w, h) = (200usize, 50usize);
        let at = |x: usize, y: usize| field_value(modes, x as f64 / ASPECT, y as f64, t);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for y in 0..h - lag_cells.1 {
            for x in 0..w - lag_cells.0 {
                a.push(at(x, y));
                b.push(at(x + lag_cells.0, y + lag_cells.1));
            }
        }
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
        for (p, q) in a.iter().zip(&b) {
            sab += (p - ma) * (q - mb);
            saa += (p - ma) * (p - ma);
            sbb += (q - mb) * (q - mb);
        }
        sab / (saa * sbb).sqrt()
    }

    #[test]
    fn cmb_a1_field_is_bounded_by_one() {
        for seed in 0..8 {
            let g = scene(seed, 40, 12);
            for i in 0..2000 {
                let (x, y, t) = (
                    f64::from(i % 97) * 0.7,
                    f64::from(i % 53),
                    f64::from(i) * 0.05,
                );
                assert!(field_value(&g.modes, x, y, t).abs() <= 1.0 + 1e-12);
            }
        }
    }

    #[test]
    fn cmb_a1_display_scale_matches_the_sampled_sigma() {
        // The display saturates at 2.7σ with σ = 1/√(2N); check σ against the field.
        let g = scene(3, 200, 50);
        let mut sum2 = 0.0;
        let mut n = 0.0;
        for t in [0.0, 40.0, 80.0, 120.0] {
            for y in 0..50 {
                for x in 0..200 {
                    let f = field_value(&g.modes, f64::from(x) / ASPECT, f64::from(y), t);
                    sum2 += f * f;
                    n += 1.0;
                }
            }
        }
        let measured = (sum2 / n).sqrt();
        // Large-scale modes make a single screen a finite sample: allow 20 %.
        assert!(
            (measured / g.sigma - 1.0).abs() < 0.2,
            "measured σ {measured:.4} vs model {:.4}",
            g.sigma
        );
    }

    #[test]
    fn cmb_a2_adjacent_cells_differ_by_at_most_the_gradient_bound() {
        for seed in 0..8 {
            let g = scene(seed, 200, 50);
            let bound = smoothness_bound(&g.modes);
            for t in [0.0, 3.3, 17.0] {
                for y in 0..50 {
                    for x in 0..199 {
                        let at =
                            |x: i32| field_value(&g.modes, f64::from(x) / ASPECT, f64::from(y), t);
                        let diff = (at(x + 1) - at(x)).abs();
                        assert!(diff <= bound + 1e-12, "seed {seed}: {diff} > {bound}");
                    }
                }
            }
        }
    }

    #[test]
    fn cmb_a2_neighbouring_cells_are_strongly_correlated() {
        // The Lipschitz bound is loose for many modes; the spots must also be several cells
        // across: ρ ≥ 0.9 between horizontal neighbours (½ row), ≥ 0.7 vertically (1 row);
        // measured minima over 64 seeds: 0.947 and 0.819.
        for seed in 0..8 {
            let g = scene(seed, 200, 50);
            let (h, v) = (
                correlation(&g.modes, (1, 0), 2.0),
                correlation(&g.modes, (0, 1), 2.0),
            );
            assert!(
                h >= 0.9 && v >= 0.7,
                "seed {seed}: rho_h {h:.3}, rho_v {v:.3}"
            );
        }
    }

    #[test]
    fn cmb_a3_full_frame_has_hot_and_cold_pixels() {
        for seed in 0..8 {
            let g = scene(seed, 200, 50);
            let (mut hot, mut cold) = (0, 0);
            for py in 0..100 {
                for x in 0..200 {
                    let ink = g.pixels.get(x, py).unwrap();
                    if ink.role == Role::Div {
                        hot += usize::from(ink.level > 160);
                        cold += usize::from(ink.level < 96);
                    }
                }
            }
            assert!(
                hot > 200 && cold > 200,
                "seed {seed}: {hot} hot, {cold} cold"
            );
        }
    }

    #[test]
    fn cmb_a4_two_point_correlation_is_isotropic() {
        // Correlation at equal aspect-corrected lags in different directions (2r cells = r rows).
        for seed in 0..8 {
            let g = scene(seed, 200, 50);
            for r in 1..=6usize {
                let (r, q) = (r as f64, r as f64 / 2f64.sqrt());
                let rho = [(r, 0.0), (0.0, r), (q, q), (q, -q)]
                    .map(|lag| ensemble_correlation(&g.modes, lag));
                let spread = rho.iter().copied().fold(f64::MIN, f64::max)
                    - rho.iter().copied().fold(f64::MAX, f64::min);
                assert!(
                    spread <= ENSEMBLE_ISOTROPY_TOL,
                    "seed {seed}, lag {r} rows: ensemble rho by direction {rho:.3?}"
                );
                let r = r as usize;
                if r <= 3 {
                    let (h, v) = (
                        correlation(&g.modes, (2 * r, 0), 1.0),
                        correlation(&g.modes, (0, r), 1.0),
                    );
                    assert!(
                        (h - v).abs() <= SAMPLE_ISOTROPY_TOL,
                        "seed {seed}, lag {r} rows: sampled rho_h {h:.3} vs rho_v {v:.3}"
                    );
                }
            }
        }
    }

    #[test]
    fn cmb_a4_power_is_spread_over_all_directions() {
        // Anisotropy of the power-weighted direction tensor Σ k̂k̂ᵀ: 0 for isotropy, 1 for
        // parallel stripes. Measured max 0.013 over 64 seeds; the old 6-mode field gave 0.27.
        for seed in 0..8 {
            let g = scene(seed, 200, 50);
            let (mut txx, mut tyy, mut txy) = (0.0, 0.0, 0.0);
            for m in &g.modes {
                let k2 = m.kx * m.kx + m.ky * m.ky;
                txx += m.kx * m.kx / k2;
                tyy += m.ky * m.ky / k2;
                txy += m.kx * m.ky / k2;
            }
            let tr = txx + tyy;
            let aniso = ((txx - tyy).powi(2) + 4.0 * txy * txy).sqrt() / tr;
            assert!(
                aniso <= 0.05,
                "seed {seed}: direction anisotropy {aniso:.3}"
            );
        }
    }

    #[test]
    fn cmb_a5_spectrum_has_plateau_and_first_acoustic_peak() {
        let grid: Vec<f64> = (0..=400)
            .map(|i| (2f64.ln() + f64::from(i) * 0.015).exp())
            .collect();
        let peak = grid
            .iter()
            .copied()
            .filter(|&l| l <= L_MAX)
            .max_by(|a, b| d_ell(*a).total_cmp(&d_ell(*b)))
            .unwrap();
        assert!(
            (200.0..=240.0).contains(&peak),
            "first peak at ℓ = {peak:.0}"
        );
        // Planck: D_220 / D_10 ≈ 5.2; the plateau is flat within 25 % over ℓ = 2..30.
        let ratio = d_ell(220.0) / d_ell(10.0);
        assert!((4.5..=6.0).contains(&ratio), "peak/plateau {ratio:.2}");
        assert!(d_ell(30.0) / d_ell(2.0) < 1.4);
        // A trough then a second peak, then damping.
        assert!(d_ell(415.0) < d_ell(540.0) && d_ell(540.0) < 0.5 * d_ell(220.0));
        assert!(d_ell(1000.0) < d_ell(815.0));
        // The first peak sits at PEAK_WAVELENGTH rows on screen.
        assert!((TAU / wavenumber(L_PEAK) - PEAK_WAVELENGTH).abs() < 1e-12);
    }

    #[test]
    fn cmb_a5_modes_are_densest_around_the_first_peak() {
        // Equal-power modes: the histogram of ln ℓ is the (beamed) power spectrum, so the
        // most populated band of width Δlnℓ = 0.5 must contain the first peak's scales.
        for seed in 0..8 {
            let g = scene(seed, 80, 24);
            let ells: Vec<f64> = g
                .modes
                .iter()
                .map(|m| m.kx.hypot(m.ky) / wavenumber(1.0))
                .collect();
            let best = (0..=40)
                .map(|i| L_MIN.ln() + f64::from(i) * 0.1)
                .max_by_key(|&lo| {
                    ells.iter()
                        .filter(|l| (lo..lo + 0.5).contains(&l.ln()))
                        .count()
                })
                .unwrap();
            let centre = (best + 0.25).exp();
            assert!(
                (140.0..=300.0).contains(&centre),
                "seed {seed}: densest band centred at ℓ = {centre:.0}"
            );
        }
    }

    #[test]
    fn cmb_separable_render_matches_field_value() {
        let mut g = scene(5, 120, 40);
        g.t = 4.2;
        let rot = g.rotation();
        for py in [0usize, 7, 79] {
            let row = g.row_vector(py, &rot);
            for x in [0usize, 31, 119] {
                let direct = field_value(&g.modes, x as f64 / ASPECT, pixel_row(py), g.t);
                assert!((g.cell_value(x, &row) - direct).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn cmb_modes_follow_the_spec_ranges_and_survive_resize() {
        let mut g = scene(9, 80, 24);
        let before = g.modes.clone();
        g.resize(Size::new(30, 10), &mut Ctx::new(123, Size::new(30, 10)));
        assert_eq!(g.modes, before);
        assert_eq!(g.modes.len(), MODES);
        let (lo, hi) = (TAU / wavenumber(L_MAX), TAU / wavenumber(L_MIN));
        for m in &g.modes {
            let wavelength = TAU / m.kx.hypot(m.ky);
            assert!((lo - 1e-9..=hi + 1e-9).contains(&wavelength));
            assert!((OMEGA.0..=OMEGA.1).contains(&m.omega.abs()));
        }
    }

    #[test]
    fn cmb_gen_r13_oval_leaves_corners_empty_and_readout_bottom_right() {
        let mut stage = Stage::new(build(), 1, Size::new(100, 30), GlyphMode::Ascii);
        let frame = stage.render();
        assert_eq!(frame.get(0, 0).unwrap().ch, ' ');
        assert_eq!(frame.get(99, 28).unwrap().ch, ' ');
        assert_ne!(frame.get(50, 14).unwrap().ch, ' ');
        assert!(frame.to_lines()[29].ends_with(&format!(" {READOUT} ")));
    }

    #[test]
    fn cmb_a6_sky_is_div_pixels_inside_the_oval_with_the_field_sign() {
        let mut g = scene(2, 120, 40);
        g.t = 7.5;
        g.shade();
        let full = SATURATION_SIGMA * g.sigma;
        let (mut inside, mut outside) = (0, 0);
        for py in 0..80 {
            for x in 0..120 {
                let ink = g.pixels.get(x, py).unwrap();
                if g.in_oval(f64::from(x), pixel_row(py as usize)) {
                    inside += 1;
                    let f =
                        field_value(&g.modes, f64::from(x) / ASPECT, pixel_row(py as usize), g.t);
                    assert_eq!(ink, ink::div(f / full), "pixel {x},{py}");
                } else {
                    outside += 1;
                    assert_eq!(ink, Ink::BLANK);
                }
            }
        }
        assert!(inside > 5000 && outside > 500, "{inside} in, {outside} out");
    }

    #[test]
    fn cmb_a7_spectrum_inset_sits_outside_the_oval_and_peaks_at_the_first_peak() {
        for (w, h) in [(120, 40), (200, 50), (160, 48)] {
            let g = scene(4, w, h);
            let inset = g.inset.expect("room for the inset");
            assert!(inset.x + inset.w < i32::from(w) && inset.y + inset.h < i32::from(h) - 1);
            for y in inset.y..inset.y + inset.h {
                for x in inset.x..inset.x + inset.w {
                    let (px, py) = (x as usize, 2 * y as usize);
                    for p in [py, py + 1] {
                        assert_eq!(g.pixels.get(px as i32, p as i32), Some(Ink::BLANK));
                    }
                }
            }
            // The curve's highest dot lies at ℓ within 15 % of 220.
            let (x0, dw) = (inset.x * Dots::CELL_W, inset.w * Dots::CELL_W);
            let top = (inset.y * Dots::CELL_H..(inset.y + inset.h) * Dots::CELL_H)
                .find_map(|y| (x0..x0 + dw).find(|&x| g.dots.is_lit(x, y)))
                .unwrap();
            let t = f64::from(top - x0) / f64::from(dw - 1);
            let ell = (INSET_ELL.0.ln() + t * (INSET_ELL.1.ln() - INSET_ELL.0.ln())).exp();
            assert!(
                (ell / L_PEAK - 1.0).abs() < 0.15,
                "peak drawn at ℓ = {ell:.0}"
            );
            // The peak is labelled on the inset's last row, inside the inset.
            let mut canvas = Canvas::new(Size::new(w, h), GlyphMode::Unicode);
            g.render(&mut canvas);
            let x = CosmicMicrowaveBackground::peak_label_x(inset);
            let row: String = (x..x + 5)
                .map(|x| canvas.get(x, inset.y + inset.h - 1).unwrap().ch)
                .collect();
            assert_eq!(row, PEAK_LABEL.0);
            assert!(x >= inset.x && x + 5 <= inset.x + inset.w);
        }
        // The cursor sweeps: the inset changes within a second, and the sky's cells stay
        // pixels throughout (GEN-R9 sees the sweep, not a hole in the map).
        let mut g = scene(4, 200, 50);
        let before = g.dots.clone();
        let mut ctx = Ctx::new(4, Size::new(200, 50));
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
        }
        let moved = (0..400)
            .flat_map(|x| (0..200).map(move |y| (x, y)))
            .any(|(x, y)| before.is_lit(x, y) != g.dots.is_lit(x, y));
        assert!(moved, "the inset cursor did not move in 1 s");
        assert_eq!(scene(4, 60, 18).inset, None, "too small for an inset");
    }
}
