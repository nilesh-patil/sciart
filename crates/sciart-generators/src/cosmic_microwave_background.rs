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
use sciart_core::timeline::ease;
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
/// The muted, dark-centred palette (ADR-0017): the field's full scale maps to `±MUTE` on
/// `Role::Div` (a cool blue and a warm orange, low in chroma this close to zero); small
/// strengths are eased smoothly towards zero (`smoothstep(u / ZERO_EASE)`), and a pixel
/// within `ZERO_FLOOR` level of `Div`'s centre is black, so the map's zero fades into black
/// through the faintest tints rather than a navy wash, with no dither.
const MUTE: f64 = 0.14;
const ZERO_EASE: f64 = 0.35;
const ZERO_FLOOR: u8 = 1;
/// The oval's outline: a 1-px dim `Starlight` rim where the field there is fainter, so the
/// frame reads even where the map is near zero.
const RIM_LEVEL: f64 = 20.0;
/// Beyond `KNEE` of the full scale (≈ 2σ) the rarest hot and cold spots rise steeply to
/// `±SPOT_PEAK` (`((u − KNEE)/(1 − KNEE))^SPOT_POW`), so the map's extremes stay legible
/// (GEN-R21) while its bulk stays muted.
const KNEE: f64 = 0.65;
const SPOT_PEAK: f64 = 0.9;
const SPOT_POW: f64 = 1.5;
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
                kx: k * dir.cos_det(),
                ky: k * dir.sin_det(),
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
        .map(|m| (m.kx * x_rows + m.ky * y + m.phase + m.omega * t).sin_det())
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
    /// The sky map.
    pixels: Pixels,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

/// The sky's ink for field value `v` (in units of the display's full scale, ±1) at oval
/// coverage `cover`: `Role::Div`, muted within `±MUTE` up to the knee and rising to
/// `±SPOT_PEAK` for the rarest spots, faded towards zero (black) at the rim.
pub(crate) fn sky_ink(v: f64, cover: f64) -> Ink {
    let u = v.abs().min(1.0);
    // Only the rare spots past the knee pay for the power (GEN-R8).
    let spot = if u > KNEE {
        ((u - KNEE) / (1.0 - KNEE)).powf(SPOT_POW)
    } else {
        0.0
    };
    let ease = ease::smoothstep((u / ZERO_EASE).min(1.0));
    let out = MUTE * u * ease + (SPOT_PEAK - MUTE) * spot;
    ink::div(out.copysign(v) * cover.clamp(0.0, 1.0))
}

/// Entries of the unit-coverage ink table over `v ∈ [−1, 1]`.
const SKY_LUT: usize = 2049;

/// The sky's ink at oval coverage `cover`: from a size-independent table over `v` where
/// the oval covers the pixel (almost all of them; built once per process, GEN-R8), and
/// [`sky_ink`] itself on the rim.
#[cfg(test)]
fn sky_pixel(v: f64, cover: f64) -> Ink {
    sky_pixel_in(sky_lut(), v, cover)
}

/// The unit-coverage ink table (built once per process).
fn sky_lut() -> &'static [Ink] {
    static LUT: std::sync::OnceLock<Box<[Ink]>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        (0..SKY_LUT)
            .map(|i| sky_ink(2.0 * i as f64 / (SKY_LUT - 1) as f64 - 1.0, 1.0))
            .collect()
    })
}

/// [`sky_pixel`] with the table in hand (hoisted out of the pixel loop).
#[inline]
fn sky_pixel_in(lut: &[Ink], v: f64, cover: f64) -> Ink {
    if cover < 1.0 {
        return sky_ink(v, cover);
    }
    let i = ((v.clamp(-1.0, 1.0) + 1.0) * 0.5 * (SKY_LUT - 1) as f64).round() as usize;
    lut[i.min(SKY_LUT - 1)]
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
        let top = (SPOT_PEAK * 128.0) as usize + 1;
        let i = usize::from(ink.level.abs_diff(128)) * ASCII_RAMP.len() / top;
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
            *r = (m.omega * self.t).sin_cos_det();
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
    #[cfg(test)]
    fn in_oval(&self, x: f64, y: f64) -> bool {
        let (dx, dy) = ((x - self.cx) / self.rx, (y - self.cy) / self.ry);
        dx * dx + dy * dy <= 1.0
    }

    /// The columns of row `y` (in rows) that lie inside the oval, the same cells
    /// `in_oval` accepts (up to rounding on the rim), found once per row.
    fn oval_span(&self, y: f64, w: usize) -> std::ops::Range<usize> {
        let dy = (y - self.cy) / (self.ry + 0.25);
        let half = (self.rx + 0.5) * (1.0 - dy * dy).sqrt();
        if half.is_nan() {
            return 0..0;
        }
        let lo = (self.cx - half).ceil().max(0.0) as usize;
        let hi = ((self.cx + half).floor() + 1.0).clamp(0.0, w as f64) as usize;
        lo.min(hi)..hi
    }

    /// Shade the sky map for the current time (the M10 raster convention: `render` only
    /// blits).
    fn shade(&mut self) {
        let w = self.cols.len() / (2 * MODES);
        let full_scale = SATURATION_SIGMA * self.sigma;
        let rot = self.rotation();
        let lut = sky_lut();
        for py in 0..2 * usize::from(self.height) {
            let span = self.oval_span(pixel_row(py), w);
            if span.is_empty() {
                continue;
            }
            let row = self.row_vector(py, &rot);
            let y = pixel_row(py);
            // Columns whose coverage is surely 1 (the rim's `cover` is computed only
            // outside them).
            let inner = self.inner_half(y);
            for x in span {
                let v = self.cell_value(x, &row) / full_scale;
                let cover = if (x as f64 - self.cx).abs() <= inner {
                    1.0
                } else {
                    self.cover(x as f64, y)
                };
                let mut ink = sky_pixel_in(lut, v, cover);
                if ink.level.abs_diff(128) <= ZERO_FLOOR {
                    ink = Ink::BLANK;
                }
                // The rim: a dim outline where the field is fainter.
                if cover < 1.0 && cover > 0.0 {
                    let rim = ink::ramp(Ramp::Starlight, RIM_LEVEL * cover / 255.0);
                    if ink.level.abs_diff(128) < 3 || ink == Ink::BLANK {
                        ink = rim;
                    }
                }
                self.pixels.set(x as i32, py as i32, ink);
            }
        }
    }

    /// Half-width (cells) of row `y`'s run of pixels the oval surely covers fully.
    fn inner_half(&self, y: f64) -> f64 {
        let span = self.rx.min(2.0 * self.ry);
        let inner = (1.0 - 0.5 / span).max(0.0);
        let dy = (y - self.cy) / self.ry;
        let q = inner * inner - dy * dy;
        if q > 0.0 {
            self.rx * q.sqrt() - 1e-9
        } else {
            -1.0
        }
    }

    /// How much of the pixel at `(x, y)` (y in rows) the oval covers: 1 inside, falling
    /// linearly across the rim's last pixel (an anti-aliased edge, CMB-A7).
    fn cover(&self, x: f64, y: f64) -> f64 {
        let (dx, dy) = ((x - self.cx) / self.rx, (y - self.cy) / self.ry);
        let r2 = dx * dx + dy * dy;
        // The rim's distance in pixels (a cell is one pixel wide and two tall); well
        // inside it the coverage is 1 without a square root.
        let span = self.rx.min(2.0 * self.ry);
        let inner = (1.0 - 0.5 / span).max(0.0);
        if r2 <= inner * inner {
            return 1.0;
        }
        let px = (1.0 - r2.sqrt()) * span;
        (px + 0.5).clamp(0.0, 1.0)
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
                (self.cols[start + i], self.cols[start + MODES + i]) = (m.kx * xr).sin_cos_det();
            }
        }
        self.rows.clear();
        for py in 0..2 * h {
            let start = self.rows.len();
            self.rows.resize(start + 2 * MODES, 0.0);
            for (i, m) in self.modes.iter().enumerate() {
                let (s, c) = (m.ky * pixel_row(py) + m.phase).sin_cos_det();
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
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.shade();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_pixel(top, bottom));
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
            .map(|m| (m.kx * lag_rows.0 + m.ky * lag_rows.1).cos_det())
            .sum::<f64>()
            / modes.len() as f64
    }

    fn scene(seed: u64, w: u16, h: u16) -> CosmicMicrowaveBackground {
        let size = Size::new(w, h);
        let mut g = CosmicMicrowaveBackground::default();
        g.resize(size, &mut Ctx::new(seed, size));
        g.prepare();
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
                    // Beyond 0.6 of the muted full scale either way.
                    if ink.role == Role::Div {
                        let d = f64::from(ink.level) - 128.0;
                        hot += usize::from(d > 0.6 * MUTE * 127.0);
                        cold += usize::from(d < -0.6 * MUTE * 128.0);
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
        g.prepare();
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
        // The oval is drawn (its zero dissolves into black, so not every cell).
        let drawn = (40..60)
            .flat_map(|x| (10..18).map(move |y| (x, y)))
            .filter(|&(x, y)| frame.get(x, y).unwrap().ch != ' ')
            .count();
        assert!(drawn > 40, "{drawn} of 160 central cells drawn");
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
                    let cover = g.cover(f64::from(x), pixel_row(py as usize));
                    let want = sky_pixel(f / full, cover);
                    // At zero the map is black (CMB-A7); on the rim a dim outline may
                    // stand in for a faint field; elsewhere it is the field's muted ink.
                    if cover < 1.0 {
                        assert!(
                            ink == want
                                || ink.role == Role::Ramp(Ramp::Starlight)
                                || ink == Ink::BLANK,
                            "pixel {x},{py}"
                        );
                    } else if want.level.abs_diff(128) > ZERO_FLOOR {
                        assert_eq!(ink, want, "pixel {x},{py}");
                    } else {
                        assert_eq!(ink, Ink::BLANK, "pixel {x},{py}");
                    }
                } else if g.cover(f64::from(x), pixel_row(py as usize)) == 0.0 {
                    outside += 1;
                    assert_eq!(ink, Ink::BLANK);
                }
            }
        }
        assert!(inside > 5000 && outside > 500, "{inside} in, {outside} out");
    }

    #[test]
    fn cmb_a7_a_muted_dark_centred_map_with_a_soft_edge() {
        // ADR-0017: the field spans only `±MUTE` of `Div` (low chroma), its zero is black,
        // and the oval's rim fades over its last pixel instead of stepping.
        let g = scene(5, 200, 50);
        let (mut drawn, mut blank_inside, mut muted) = (0, 0, 0);
        for py in 0..100 {
            for x in 0..200 {
                let ink = g.pixels.get(x, py).unwrap();
                if ink.role == Role::Ramp(Ramp::Starlight) {
                    // The oval's dim outline.
                    assert!(f64::from(ink.level) <= RIM_LEVEL + 1.0);
                    assert!(g.cover(f64::from(x), pixel_row(py as usize)) < 1.0);
                } else if ink.role == Role::Div {
                    drawn += 1;
                    assert!(
                        f64::from(ink.level.abs_diff(128)) <= SPOT_PEAK * 128.0 + 1.0,
                        "{ink:?}"
                    );
                    muted += usize::from(
                        f64::from(ink.level.abs_diff(128)) <= KNEE * MUTE * 128.0 + 1.0,
                    );
                } else {
                    assert_eq!(ink, Ink::BLANK);
                    blank_inside += usize::from(g.in_oval(f64::from(x), pixel_row(py as usize)));
                }
            }
        }
        assert!(drawn > 5000, "{drawn} drawn");
        // Most of the map is muted; only the rarest spots rise past the knee.
        assert!(muted * 10 > drawn * 8, "{muted} of {drawn} muted");
        assert!(
            blank_inside * 10 > drawn,
            "the zero is black: {blank_inside} blank inside"
        );
        // Coverage: 1 well inside, 0 well outside, and in between across the rim.
        assert_eq!(g.cover(g.cx, g.cy), 1.0);
        assert_eq!(g.cover(g.cx + 2.0 * g.rx, g.cy), 0.0);
        let rim = g.cover(g.cx + g.rx, g.cy);
        assert!((0.3..=0.7).contains(&rim), "rim coverage {rim}");
        assert_eq!(sky_ink(1.0, 0.0), ink::div(0.0));
        assert_eq!(sky_ink(0.5, 1.0), ink::div(0.5 * MUTE));
        // Small strengths ease smoothly to zero (no dither).
        let small = sky_ink(0.05, 1.0).level.abs_diff(128);
        assert!(small <= ZERO_FLOOR, "{small}");
        assert_eq!(sky_ink(-1.0, 1.0), ink::div(-SPOT_PEAK));
    }
}
