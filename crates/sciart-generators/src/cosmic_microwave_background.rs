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
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "cosmic_microwave_background",
    category: Category::Astronomy,
    description: "A shimmering Planck-style map of the CMB anisotropies",
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
/// any sensible map; above `L_MAX` it (< 1.74 rows) would alias on the 1-row vertical grid.
const L_MIN: f64 = 10.0;
const L_MAX: f64 = 760.0;
/// Gaussian beam `B²(ℓ) = exp(−(ℓ/L_BEAM)²)`: the terminal cell is our telescope's beam.
/// It keeps the first peak (B² ≈ 0.74) and suppresses power near the vertical Nyquist
/// scale (ℓ ≈ 660, B² ≈ 0.07), just like Planck's beam rolls off its damping tail.
const L_BEAM: f64 = 400.0;
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
const RAMP: [Glyph; 4] = [
    Glyph::new('░', '.'),
    Glyph::new('▒', ':'),
    Glyph::new('▓', '*'),
    Glyph::new('█', '#'),
];
const CAPTION: &str = "CMB  T=2.725K  dT~1e-5";

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
        let (l0, l1) = ((a + step * i as f64).exp(), (a + step * (i + 1) as f64).exp());
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
    /// Per row `y`, `2·MODES` values: `cos(kyᵢ·y + φᵢ)` for all i, then `sin(…)`.
    rows: Vec<f64>,
    /// `σ` of the field for random phases: `1/√(2N)`.
    sigma: f64,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    height: u16,
    caption: bool,
}

impl CosmicMicrowaveBackground {
    /// `[cos B, sin B]` with `B = ky·y + φ + ωt` for every mode on row `y`, by rotating the
    /// per-resize row table by `ωt` (no trig per row).
    fn row_vector(&self, y: usize, rot: &[(f64, f64); MODES]) -> [f64; 2 * MODES] {
        let r = &self.rows[y * 2 * MODES..(y + 1) * 2 * MODES];
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
    /// the column and row vectors (4 accumulators so it vectorises).
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
        for y in 0..h {
            let start = self.rows.len();
            self.rows.resize(start + 2 * MODES, 0.0);
            for (i, m) in self.modes.iter().enumerate() {
                let (s, c) = (m.ky * y as f64 + m.phase).sin_cos();
                (self.rows[start + i], self.rows[start + MODES + i]) = (c, s);
            }
        }
        self.height = size.height();
        self.caption = size.height() > 6;
        let sky_rows = f64::from(size.height()) - if self.caption { 1.0 } else { 0.0 };
        self.cx = (w as f64 - 1.0) / 2.0;
        self.cy = (sky_rows - 1.0) / 2.0;
        self.rx = (OVAL * w as f64 / 2.0).max(0.5);
        self.ry = (OVAL * sky_rows / 2.0).max(0.5);
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let w = self.cols.len() / (2 * MODES);
        if w == 0 {
            return;
        }
        let full_scale = SATURATION_SIGMA * self.sigma;
        let rot = self.rotation();
        for y in 0..self.height {
            let dy = (f64::from(y) - self.cy) / self.ry;
            if dy.abs() > 1.0 {
                continue;
            }
            // Only the cells inside the oval; the corners stay black like a Mollweide map.
            let half = self.rx * (1.0 - dy * dy).sqrt();
            let x0 = (self.cx - half).ceil().max(0.0) as usize;
            let x1 = ((self.cx + half).floor().max(0.0) as usize).min(w - 1);
            let row = self.row_vector(usize::from(y), &rot);
            for x in x0..=x1 {
                let f = self.cell_value(x, &row);
                let v = (f.abs() / full_scale).min(1.0);
                let role = if f > 0.0 { Role::Hot } else { Role::Cold };
                let ink = Ink::new(role, (45.0 + 210.0 * v) as u8);
                canvas.put(x as i32, i32::from(y), draw::ramp(v, &RAMP), ink);
            }
        }
        if self.caption {
            let y = i32::from(self.height) - 1;
            draw::text_centered(canvas, y, CAPTION, Ink::new(Role::Text, 200));
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
            assert!(h >= 0.9 && v >= 0.7,"seed {seed}: rho_h {h:.3}, rho_v {v:.3}");
        }
    }

    #[test]
    fn cmb_a3_full_frame_has_hot_and_cold_cells() {
        for seed in 0..8 {
            let mut stage = Stage::new(build(), seed, Size::new(200, 50), GlyphMode::Unicode);
            let frame = stage.render();
            let count = |r| frame.cells().iter().filter(|c| c.ink.role == r).count();
            assert!(
                count(Role::Hot) > 100 && count(Role::Cold) > 100,
                "seed {seed}"
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
            assert!(aniso <= 0.05,"seed {seed}: direction anisotropy {aniso:.3}");
        }
    }

    #[test]
    fn cmb_a5_spectrum_has_plateau_and_first_acoustic_peak() {
        let grid: Vec<f64> = (0..=400).map(|i| (2f64.ln() + f64::from(i) * 0.015).exp()).collect();
        let peak = grid
            .iter()
            .copied()
            .filter(|&l| l <= L_MAX)
            .max_by(|a, b| d_ell(*a).total_cmp(&d_ell(*b)))
            .unwrap();
        assert!((200.0..=240.0).contains(&peak), "first peak at ℓ = {peak:.0}");
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
        for y in [0usize, 7, 39] {
            let row = g.row_vector(y, &rot);
            for x in [0usize, 31, 119] {
                let direct = field_value(&g.modes, x as f64 / ASPECT, y as f64, g.t);
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
    fn cmb_oval_leaves_corners_empty_and_caption_on_last_row() {
        let mut stage = Stage::new(build(), 1, Size::new(100, 30), GlyphMode::Ascii);
        let frame = stage.render();
        assert_eq!(frame.get(0, 0).unwrap().ch, ' ');
        assert_eq!(frame.get(99, 28).unwrap().ch, ' ');
        assert_ne!(frame.get(50, 14).unwrap().ch, ' ');
        assert!(frame.to_lines()[29].contains(CAPTION));
    }
}
