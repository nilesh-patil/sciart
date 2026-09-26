//! A pollen grain jittering among fast solvent molecules, leaving a fading random-walk
//! trail, with a live diffusion-constant readout (SPEC-002 `brownian_motion`).
//!
//! Physics: the grain's velocity is an Ornstein–Uhlenbeck process
//! `dv = −γ v dt + σ dW` (Langevin equation without a potential). Position and velocity are
//! advanced with the *exact* joint Gaussian transition of the OU process ([`OuStep`]), so
//! the statistics (stationary variance `σ²/(2γ)`, diffusion constant `D = σ²/(2γ²)`) are
//! exact for any frame step `dt` — no sub-stepping and no step-size bias.
//!
//! Walls: the grain lives in an *unfolded* free space and is drawn at its mirror-folded
//! position ([`fold`]). For dynamics that are symmetric under reflection (free OU is), the
//! method of images makes this exactly a specularly reflecting box, at any `dt`.

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::particles::{gaussian2, plot};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "brownian_motion",
    category: Category::PhysicsChemistry,
    description: "A pollen grain jittering among solvent molecules (Langevin dynamics)",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(BrownianMotion::default())
}

/// Friction rate γ, 1/s.
const GAMMA: f64 = 2.0;
/// Diffusion constant, rows²/s: `√(4 D · 1 s) = 3` rows of rms 2-D displacement per √s.
const D: f64 = 2.25;
/// Noise strength σ = γ √(2D), from `D = σ²/(2γ²)` (4.2426… rows/s^{3/2}).
const SIGMA: f64 = GAMMA * 2.121_320_343_559_642_4;
/// Trail length in samples (BRO-A3).
const TRAIL_LEN: usize = 180;
/// Sampling tick for the trail and the MSD estimator, s. 180 ticks ≈ 22 s of path.
const TICK: f64 = 0.125;
/// MSD lag τ for the D̂ estimator, in ticks (2 s ≫ 1/γ = 0.5 s).
const LAG_TICKS: usize = 16;
/// Solvent molecules per cell and their hard cap (GEN-R8).
const MOL_PER_CELL: f64 = 0.02;
const MOL_MAX: usize = 400;
/// Maxwell–Boltzmann velocity standard deviation per axis for the molecules, rows/s.
const MOL_SD: f64 = 7.0;
/// Collision flashes: pool cap, lifetime (s), and the chance a hit flashes.
const FLASH_MAX: usize = 32;
const FLASH_LIFE: f64 = 0.3;
const FLASH_P: f64 = 0.35;
/// Molecules bounce off the grain at this extra distance, rows.
const CONTACT_PAD: f64 = 0.35;

const GRAIN_CORE: Glyph = Glyph::new('●', '@');
const GRAIN_EDGE: Glyph = Glyph::new('◍', 'O');
const MOL_SLOW: Glyph = Glyph::new('·', '.');
const MOL_FAST: Glyph = Glyph::new('•', 'o');
const TRAIL: Glyph = Glyph::new('∙', '.');
const FLASH: Glyph = Glyph::new('✦', '*');

/// Exact one-axis transition of the OU process over a step `h` (BRO-A1, BRO-A2).
///
/// With `a = e^{−γh}`, `(Δx, v′)` given `v` is bivariate normal:
/// - `E[v′] = a v`, `Var v′ = σ²(1 − a²)/(2γ)`
/// - `E[Δx] = v (1 − a)/γ`, `Var Δx = (σ²/γ²)[h − 2(1 − a)/γ + (1 − a²)/(2γ)]`
/// - `Cov(Δx, v′) = σ²(1 − a)²/(2γ²)`
///
/// sampled as `v′ = a v + s_v z₁`, `Δx = v(1 − a)/γ + c z₁ + s_x z₂` (Cholesky).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OuStep {
    a: f64,
    drift_x: f64,
    sv: f64,
    cx: f64,
    sx: f64,
}

impl OuStep {
    /// Coefficients for friction `gamma > 0`, noise `sigma` and step `h`; `h ≤ 0` (or bad
    /// input) is the identity step.
    pub(crate) fn new(gamma: f64, sigma: f64, h: f64) -> Self {
        if !(h > 0.0 && h.is_finite() && gamma > 0.0 && gamma.is_finite() && sigma.is_finite()) {
            return Self {
                a: 1.0,
                drift_x: 0.0,
                sv: 0.0,
                cx: 0.0,
                sx: 0.0,
            };
        }
        let gh = gamma * h;
        let a = (-gh).exp();
        // expm1 keeps 1 − a and 1 − a² accurate when γh is small (the usual frame step).
        let one_m_a = -(-gh).exp_m1();
        let one_m_a2 = -(-2.0 * gh).exp_m1();
        let s2 = sigma * sigma;
        let var_v = s2 / (2.0 * gamma) * one_m_a2;
        let var_x = s2 / (gamma * gamma) * (h - 2.0 * one_m_a / gamma + one_m_a2 / (2.0 * gamma));
        let cov = s2 / (2.0 * gamma * gamma) * one_m_a * one_m_a;
        let sv = var_v.sqrt();
        let cx = if sv > 0.0 { cov / sv } else { 0.0 };
        // Conditional variance of Δx given v′; ≥ 0 analytically, clamp the rounding.
        let sx = (var_x - cx * cx).max(0.0).sqrt();
        Self {
            a,
            drift_x: one_m_a / gamma,
            sv,
            cx,
            sx,
        }
    }

    /// Advance one axis `(x, v)` using two independent standard normals.
    #[inline]
    pub(crate) fn apply(&self, x: f64, v: f64, z1: f64, z2: f64) -> (f64, f64) {
        let dx = v * self.drift_x + self.cx * z1 + self.sx * z2;
        (x + dx, self.a * v + self.sv * z1)
    }
}

/// Mirror-fold `x` into `[lo, hi]`; the flag is `true` for an odd number of reflections,
/// i.e. when a velocity must flip sign (BRO-A3). Non-finite `x` or an empty interval map
/// to `lo`.
pub(crate) fn fold(x: f64, lo: f64, hi: f64) -> (f64, bool) {
    let len = hi - lo;
    if !x.is_finite() || len.is_nan() || len <= 0.0 {
        return (lo, false);
    }
    // Reflection has period 2·len; the second half of the period is the mirror image.
    let u = (x - lo).rem_euclid(2.0 * len);
    if u > len {
        ((hi - (u - len)).clamp(lo, hi), true)
    } else {
        ((lo + u).clamp(lo, hi), false)
    }
}

/// Expected 2-D MSD after `t` seconds for a walker started from the stationary velocity
/// distribution: `4D [t − (1 − e^{−γt})/γ]` → `4Dt` for `t ≫ 1/γ` (BRO-A2).
pub(crate) fn msd_ou(t: f64) -> f64 {
    if t.is_nan() || t <= 0.0 {
        return 0.0;
    }
    // t − (1 − e^{−γt})/γ via expm1 so the ballistic regime (γt ≪ 1) keeps its digits.
    4.0 * D * (t + (-GAMMA * t).exp_m1() / GAMMA)
}

/// Stationary per-axis velocity standard deviation `σ/√(2γ)`.
fn v_stationary_sd() -> f64 {
    SIGMA / (2.0 * GAMMA).sqrt()
}

/// Draw one free (unfolded) OU step for a 2-D walker `(x, y, vx, vy)`.
#[inline]
fn ou_step_2d(step: &OuStep, s: &mut [f64; 4], rng: &mut impl Rng) {
    let (z1, z2) = gaussian2(rng, 1.0);
    let (z3, z4) = gaussian2(rng, 1.0);
    (s[0], s[2]) = step.apply(s[0], s[2], z1, z2);
    (s[1], s[3]) = step.apply(s[1], s[3], z3, z4);
}

#[derive(Debug, Clone, Copy)]
struct Molecule {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
}

#[derive(Debug, Clone, Copy)]
struct Flash {
    x: f64,
    y: f64,
    age: f64,
}

/// A rectangle in rows (x already divided by ASPECT).
#[derive(Debug, Clone, Copy, Default)]
struct Bounds {
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Bounds {
    #[cfg(test)]
    fn contains(&self, x: f64, y: f64) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }

    /// Shrink by `r` on every side; collapses to the center when too small.
    fn inset(&self, r: f64) -> Self {
        let (cx, cy) = ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0);
        let (hx, hy) = ((self.x1 - self.x0) / 2.0, (self.y1 - self.y0) / 2.0);
        let (hx, hy) = ((hx - r).max(0.0), (hy - r).max(0.0));
        Self {
            x0: cx - hx,
            x1: cx + hx,
            y0: cy - hy,
            y1: cy + hy,
        }
    }
}

#[derive(Debug)]
pub struct BrownianMotion {
    t: f64,
    /// Time since the last tick.
    tick_clock: f64,
    /// Unfolded grain state `(x, y, vx, vy)`, rows and rows/s.
    grain: [f64; 4],
    grain_r: f64,
    /// Box for molecules and for the grain's center.
    mol_box: Bounds,
    grain_box: Bounds,
    /// Folded grain positions, oldest → newest (BRO-A3).
    trail: Trail<(f64, f64)>,
    /// Unfolded positions over the last lag τ, for the time-averaged MSD.
    lag: Trail<(f64, f64)>,
    msd_sum: f64,
    msd_count: u64,
    molecules: ParticlePool<Molecule>,
    flashes: ParticlePool<Flash>,
    label: String,
    label_row: Option<i32>,
    /// Canvas width in cells, for fitting the readout.
    width: usize,
}

impl Default for BrownianMotion {
    fn default() -> Self {
        Self {
            t: 0.0,
            tick_clock: 0.0,
            grain: [0.0; 4],
            grain_r: 1.0,
            mol_box: Bounds::default(),
            grain_box: Bounds::default(),
            trail: Trail::new(TRAIL_LEN),
            lag: Trail::new(LAG_TICKS + 1),
            msd_sum: 0.0,
            msd_count: 0,
            molecules: ParticlePool::with_capacity(0),
            flashes: ParticlePool::with_capacity(FLASH_MAX),
            label: String::with_capacity(64),
            label_row: None,
            width: 0,
        }
    }
}

impl BrownianMotion {
    /// Folded (on-screen) grain center, rows.
    fn grain_pos(&self) -> (f64, f64) {
        let b = &self.grain_box;
        (
            fold(self.grain[0], b.x0, b.x1).0,
            fold(self.grain[1], b.y0, b.y1).0,
        )
    }

    /// Time-averaged MSD at lag τ, corrected for the ballistic start of the OU walk, so it
    /// estimates `D` without bias: `D̂ = ⟨|Δr(τ)|²⟩ / (4 [τ − (1 − e^{−γτ})/γ])`.
    fn d_hat(&self) -> Option<f64> {
        (self.msd_count > 0).then(|| {
            let tau = LAG_TICKS as f64 * TICK;
            self.msd_sum / self.msd_count as f64 / msd_ou(tau) * D
        })
    }

    fn tick(&mut self) {
        let (gx, gy) = self.grain_pos();
        self.trail.push((gx, gy));
        self.lag.push((self.grain[0], self.grain[1]));
        if self.lag.len() == LAG_TICKS + 1
            && let (Some(&old), Some(new)) = (self.lag.iter().next(), self.lag.newest())
        {
            self.msd_sum += (new.0 - old.0).powi(2) + (new.1 - old.1).powi(2);
            self.msd_count += 1;
        }
    }

    /// Readout, e.g. `D^ (MSD@2s) = 2.31   D = 2.25 rows^2/s   t = 42s`: the estimate named
    /// by its estimator (time-averaged MSD at the lag τ, inertia-corrected — see [`d_hat`]),
    /// then the true D, then the elapsed time. On narrow screens trailing parts are dropped
    /// rather than clipped; below the estimator alone the label is empty (not drawn).
    ///
    /// [`d_hat`]: Self::d_hat
    fn refresh_label(&mut self) {
        let lag = LAG_TICKS as f64 * TICK;
        let d_hat = self.d_hat();
        // Longest first; the first that fits with a 1-cell margin each side wins.
        for parts in (1..=3).rev() {
            self.label.clear();
            let _ = match d_hat {
                Some(d) => write!(self.label, "D^ (MSD@{lag:.0}s) = {d:.2}"),
                None => write!(self.label, "D^ (MSD@{lag:.0}s) = ..."),
            };
            if parts >= 2 {
                let _ = write!(self.label, "   D = {D:.2} rows^2/s");
            }
            if parts >= 3 {
                let _ = write!(self.label, "   t = {:.0}s", self.t);
            }
            if self.label.len() + 2 <= self.width {
                return;
            }
        }
        self.label.clear();
    }

    fn move_molecules(&mut self, dt: f64, ctx: &mut Ctx) {
        let b = self.mol_box;
        let (gx, gy) = self.grain_pos();
        let reach = self.grain_r + CONTACT_PAD;
        let flashes = &mut self.flashes;
        for m in self.molecules.iter_mut() {
            let (x, fx) = fold(m.x + m.vx * dt, b.x0, b.x1);
            let (y, fy) = fold(m.y + m.vy * dt, b.y0, b.y1);
            (m.x, m.y) = (x, y);
            if fx {
                m.vx = -m.vx;
            }
            if fy {
                m.vy = -m.vy;
            }
            // Specular bounce off the grain; a fraction of hits flash at the contact point.
            let (dx, dy) = (m.x - gx, m.y - gy);
            let d2 = dx * dx + dy * dy;
            if d2 < reach * reach && d2 > 1e-12 {
                let d = d2.sqrt();
                let (nx, ny) = (dx / d, dy / d);
                let vn = m.vx * nx + m.vy * ny;
                if vn < 0.0 {
                    m.vx -= 2.0 * vn * nx;
                    m.vy -= 2.0 * vn * ny;
                    if ctx.rng.random::<f64>() < FLASH_P {
                        flashes.spawn(Flash {
                            x: gx + nx * reach,
                            y: gy + ny * reach,
                            age: 0.0,
                        });
                    }
                }
                m.x = fold(gx + nx * reach, b.x0, b.x1).0;
                m.y = fold(gy + ny * reach, b.y0, b.y1).0;
            }
        }
        self.flashes.retain(|f| {
            f.age += dt;
            f.age < FLASH_LIFE
        });
    }
}

impl Generator for BrownianMotion {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        // Bottom row is the readout when there is room for it.
        self.width = usize::from(size.width());
 self.label_row = (size.height() >= 8).then(|| i32::from(size.height()) - 1);
        let rows = if self.label_row.is_some() { h - 1.0 } else { h };
        self.mol_box = Bounds {
            x0: 0.0,
            x1: (w - 1.0) / ASPECT,
            y0: 0.0,
            y1: rows - 1.0,
        };
        self.grain_r = (0.06 * rows.min(w / ASPECT)).clamp(0.5, 2.5);
        self.grain_box = self.mol_box.inset(self.grain_r);

        // A fresh run: grain at the center with a stationary velocity.
        let b = self.grain_box;
        let (vx, vy) = gaussian2(&mut ctx.rng, v_stationary_sd());
        self.grain = [(b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, vx, vy];
        self.t = 0.0;
        self.tick_clock = 0.0;
        self.trail.clear();
        self.lag.clear();
        self.msd_sum = 0.0;
        self.msd_count = 0;
        self.tick();

        let n = ((MOL_PER_CELL * size.area() as f64) as usize).min(MOL_MAX);
        self.molecules.set_capacity(n);
        self.molecules.clear();
        let m = self.mol_box;
        for _ in 0..n {
            let (vx, vy) = gaussian2(&mut ctx.rng, MOL_SD);
            self.molecules.spawn(Molecule {
                x: m.x0 + ctx.rng.random::<f64>() * (m.x1 - m.x0),
                y: m.y0 + ctx.rng.random::<f64>() * (m.y1 - m.y0),
                vx,
                vy,
            });
        }
        self.flashes.clear();
        self.refresh_label();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        // The OU transition is exact for any h, so split the frame at tick boundaries and the
        // trail/MSD samples land at exact multiples of TICK whatever dt is.
        let mut left = dt;
        while left > 0.0 {
            let h = left.min(TICK - self.tick_clock);
            let step = OuStep::new(GAMMA, SIGMA, h);
            ou_step_2d(&step, &mut self.grain, &mut ctx.rng);
            self.tick_clock += h;
            left -= h;
            if self.tick_clock >= TICK - 1e-12 {
                self.tick_clock = 0.0;
                self.tick();
            }
        }
        self.t += dt;
        self.move_molecules(dt, ctx);
        self.refresh_label();
    }

    fn render(&self, canvas: &mut Canvas) {
        for m in self.molecules.iter() {
            let s2 = m.vx * m.vx + m.vy * m.vy;
            let fast = s2 > 2.0 * MOL_SD * MOL_SD;
            let u = (s2.sqrt() / (3.0 * MOL_SD)).min(1.0);
            let ink = Ink::new(Role::Particle, (70.0 + 150.0 * u) as u8);
            let glyph = if fast { MOL_FAST } else { MOL_SLOW };
            plot(canvas, m.x * ASPECT, m.y, glyph, ink);
        }

        // Trail: join consecutive samples with dots, fading with age.
        let n = self.trail.len();
        let (gx, gy) = self.grain_pos();
        let mut prev: Option<(f64, f64)> = None;
        for (i, &p) in self
            .trail
            .iter()
            .chain(std::iter::once(&(gx, gy)))
            .enumerate()
        {
            let age = 1.0 - (i as f64 / n.max(1) as f64).min(1.0);
            let ink = Ink::new(
                Role::Structure,
                (35.0 + 185.0 * (1.0 - age).powf(1.3)) as u8,
            );
            if let Some(q) = prev {
                let (dc, dr) = ((p.0 - q.0) * ASPECT, p.1 - q.1);
                let steps = dc.abs().max(dr.abs()).ceil().clamp(1.0, 256.0) as usize;
                for k in 1..=steps {
                    let f = k as f64 / steps as f64;
                    plot(canvas, (q.0 * ASPECT) + dc * f, q.1 + dr * f, TRAIL, ink);
                }
            }
            prev = Some(p);
        }

        for f in self.flashes.iter() {
            let level = 255.0 * (1.0 - f.age / FLASH_LIFE).clamp(0.0, 1.0);
            plot(
                canvas,
                f.x * ASPECT,
                f.y,
                FLASH,
                Ink::new(Role::Particle, level.max(90.0) as u8),
            );
        }

        draw::disc(canvas, gx * ASPECT, gy, self.grain_r, |d, nx, ny| {
            // Lit from the upper left.
            let light = (0.75 - 0.35 * (nx + ny)).clamp(0.3, 1.0);
            let glyph = if d < 0.7 { GRAIN_CORE } else { GRAIN_EDGE };
            Some((glyph, Ink::new(Role::Accent, (255.0 * light) as u8)))
        });

        if let Some(row) = self.label_row
            && usize::from(canvas.size().width()) >= self.label.len() + 2
        {
            draw::text_centered(canvas, row, &self.label, Ink::new(Role::Text, 170));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seeded ChaCha8 stream (the same generator the runtime hands to scenes).
    fn seeded(seed: u64) -> Ctx {
        Ctx::new(seed, Size::new(1, 1))
    }

    /// `n` free 2-D walkers with stationary velocities at the origin.
    fn walkers(n: usize, ctx: &mut Ctx) -> Vec<[f64; 4]> {
        (0..n)
            .map(|_| {
                let (vx, vy) = gaussian2(&mut ctx.rng, v_stationary_sd());
                [0.0, 0.0, vx, vy]
            })
            .collect()
    }

    fn advance(ws: &mut [[f64; 4]], h: f64, steps: usize, ctx: &mut Ctx) {
        let step = OuStep::new(GAMMA, SIGMA, h);
        for w in ws.iter_mut() {
            for _ in 0..steps {
                ou_step_2d(&step, w, &mut ctx.rng);
            }
        }
    }

    #[test]
    fn bro_constants_satisfy_d_equals_sigma2_over_2gamma2() {
        assert!((SIGMA * SIGMA / (2.0 * GAMMA * GAMMA) - D).abs() < 1e-12);
        // ~3 rows of rms 2-D displacement per √s (spec).
        assert!(((4.0 * D).sqrt() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn bro_a1_exact_step_coefficients_match_the_ou_moments() {
        for h in [1e-3, SIM_DT, 0.1, 0.25, 3.0] {
            let s = OuStep::new(GAMMA, SIGMA, h);
            let a = (-GAMMA * h).exp();
            let var_v = SIGMA * SIGMA / (2.0 * GAMMA) * (1.0 - a * a);
            let var_x = SIGMA * SIGMA / (GAMMA * GAMMA)
                * (h - 2.0 * (1.0 - a) / GAMMA + (1.0 - a * a) / (2.0 * GAMMA));
            let cov = SIGMA * SIGMA / (2.0 * GAMMA * GAMMA) * (1.0 - a).powi(2);
            // Mean: noise off.
            let (x, v) = s.apply(1.0, 2.0, 0.0, 0.0);
            assert!((v - 2.0 * a).abs() < 1e-12, "h={h}");
            assert!((x - (1.0 + 2.0 * (1.0 - a) / GAMMA)).abs() < 1e-12, "h={h}");
            // Second moments from the Cholesky factors. The reference var_x above suffers
            // cancellation at tiny h (~γ²h³/3 left of h), hence the looser 1e-6.
            let got_var_v = s.sv * s.sv;
            let got_cov = s.sv * s.cx;
            let got_var_x = s.cx * s.cx + s.sx * s.sx;
            assert!((got_var_v - var_v).abs() <= 1e-9 * var_v, "var v, h={h}");
            assert!((got_cov - cov).abs() <= 1e-9 * cov, "cov, h={h}");
            assert!((got_var_x - var_x).abs() <= 1e-6 * var_x, "var x, h={h}");
        }
        // h = 0 is the identity.
        let id = OuStep::new(GAMMA, SIGMA, 0.0);
        assert_eq!(id.apply(1.5, -2.0, 3.0, -1.0), (1.5, -2.0));
    }

    #[test]
    fn bro_a1_stationary_velocity_variance_is_sigma2_over_2gamma() {
        let target = SIGMA * SIGMA / (2.0 * GAMMA);
        for (seed, h) in [(11, 0.1f64), (12, 0.25)] {
            let mut ctx = seeded(seed);
            // Start far from equilibrium (at rest) and let it relax for 3 s = 6/γ.
            let mut ws = vec![[0.0f64; 4]; 2000];
            let per_s = (1.0 / h).round() as usize;
            advance(&mut ws, h, 3 * per_s, &mut ctx);
            // 2000 walkers × 2 axes × 5 samples 1 s apart ≈ 20 000 nearly independent
            // draws (lag-1 s velocity correlation e^{-2}): relative s.e. of the variance
            // ≈ 1 %, so the spec's 5 % tolerance is a > 4σ margin.
            let (mut sum2, mut sum, mut n) = (0.0, 0.0, 0.0);
            for _ in 0..5 {
                advance(&mut ws, h, per_s, &mut ctx);
                for w in &ws {
                    sum2 += w[2] * w[2] + w[3] * w[3];
                    sum += w[2] + w[3];
                    n += 2.0;
                }
            }
            let mean = sum / n;
            let var = sum2 / n - mean * mean;
            assert!(
                (var / target - 1.0).abs() < 0.05,
                "h={h}: var {var:.3} vs σ²/(2γ) = {target:.3}"
            );
            assert!(mean.abs() < 0.05 * target.sqrt(), "h={h}: mean {mean}");
        }
    }

    /// Ensemble MSD of `n` stationary-start walkers after `t` s, stepped at `h`.
    fn ensemble_msd(seed: u64, h: f64, n: usize, t: f64) -> f64 {
        let mut ctx = seeded(seed);
        let mut ws = walkers(n, &mut ctx);
        advance(&mut ws, h, (t / h).round() as usize, &mut ctx);
        ws.iter().map(|w| w[0] * w[0] + w[1] * w[1]).sum::<f64>() / n as f64
    }

    #[test]
    fn bro_a2_ensemble_msd_is_4dt_for_long_times() {
        let (n, t) = (3000, 20.0);
        let msd = ensemble_msd(21, 0.25, n, t);
        // r² of a 2-D Gaussian has relative s.d. 1, so the s.e. is 1/√3000 ≈ 1.8 %. The
        // spec's 10 % leaves > 4 s.e. after the 1/(γt) = 2.5 % inertial offset of 4Dt.
        assert!(
            (msd / (4.0 * D * t) - 1.0).abs() < 0.10,
            "msd {msd:.2} vs 4Dt {:.2}",
            4.0 * D * t
        );
        // Against the exact OU curve: 4 s.e.
        let tol = 4.0 / (n as f64).sqrt();
        assert!((msd / msd_ou(t) - 1.0).abs() < tol, "msd {msd:.2}");
    }

    #[test]
    fn bro_a2_ensemble_msd_matches_the_exact_curve_at_another_step() {
        // The exact update makes the ensemble step-size independent: h = 0.1 s, t = 12 s.
        let (n, t) = (1500, 12.0);
        let msd = ensemble_msd(22, 0.1, n, t);
        let tol = 4.0 / (n as f64).sqrt();
        assert!(
            (msd / msd_ou(t) - 1.0).abs() < tol,
            "msd {msd:.2} vs exact {:.2}",
            msd_ou(t)
        );
    }

    #[test]
    fn bro_a2_msd_curve_is_ballistic_then_diffusive() {
        // Short times: ⟨r²⟩ ≈ 2⟨v²⟩t² = (σ²/γ) t²; long times: slope 4D.
        let t = 1e-3;
        let ballistic = SIGMA * SIGMA / GAMMA * t * t;
        assert!((msd_ou(t) / ballistic - 1.0).abs() < 1e-2);
        let slope = msd_ou(101.0) - msd_ou(100.0);
        assert!((slope / (4.0 * D) - 1.0).abs() < 1e-9);
        assert_eq!(msd_ou(0.0), 0.0);
    }

    #[test]
    fn bro_a3_fold_reflects_and_reports_parity() {
        assert_eq!(fold(0.5, 0.0, 1.0), (0.5, false));
        let (x, flip) = fold(1.25, 0.0, 1.0);
        assert!((x - 0.75).abs() < 1e-12 && flip);
        let (x, flip) = fold(-0.25, 0.0, 1.0);
        assert!((x - 0.25).abs() < 1e-12 && flip);
        let (x, flip) = fold(2.25, 0.0, 1.0);
        assert!((x - 0.25).abs() < 1e-12 && !flip);
        let (x, _) = fold(-1e9 - 0.3, 2.0, 5.0);
        assert!((2.0..=5.0).contains(&x));
        assert_eq!(fold(f64::NAN, 2.0, 5.0), (2.0, false));
        assert_eq!(fold(3.0, 2.0, 2.0), (2.0, false));
    }

    #[test]
    fn bro_a3_grain_and_molecules_stay_in_the_box_and_trail_is_bounded() {
        let mut ctx = seeded(5);
        let mut g = BrownianMotion::default();
        let dts = [SIM_DT, 0.25, 0.0, 0.1, SIM_DT, 0.013];
        for (w, h) in [(200, 50), (1, 1), (2, 9), (80, 24), (7, 3), (120, 40)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            for i in 0..900 {
                g.update(dts[i % dts.len()], &mut ctx);
                let (gx, gy) = g.grain_pos();
                assert!(g.grain_box.contains(gx, gy), "{w}x{h}: grain ({gx}, {gy})");
                for m in g.molecules.iter() {
                    assert!(g.mol_box.contains(m.x, m.y), "{w}x{h}: molecule");
                }
                assert!(g.trail.len() <= TRAIL_LEN);
                assert!(g.molecules.len() <= MOL_MAX);
            }
            // 900 frames averaging ~0.08 s ≫ 180 ticks: the trail is full, never longer.
            assert_eq!(g.trail.len(), TRAIL_LEN, "{w}x{h}");
        }
    }

    #[test]
    fn bro_readout_estimates_d_from_a_single_grain() {
        let mut ctx = seeded(8);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(200, 50), &mut ctx);
        for _ in 0..(150.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        // ~75 independent 2 s windows: s.e. ≈ 12 %; 40 % is a loose, non-flaky sanity bound.
        let d = g.d_hat().expect("estimate after 150 s");
        assert!((d / D - 1.0).abs() < 0.4, "D^ = {d}");
        // The label names the estimator actually used and shows its value.
        assert!(g.label.contains(&format!("D^ (MSD@2s) = {d:.2}")), "{}", g.label);
    }

    #[test]
    fn bro_readout_names_the_estimator_and_fits_every_width() {
        // The lag in the label is the estimator's lag, not a hard-coded string.
        assert_eq!(LAG_TICKS as f64 * TICK, 2.0);
        let mut ctx = seeded(9);
        let mut g = BrownianMotion::default();
        for w in 1..=120u16 {
            let size = Size::new(w, 20);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            for _ in 0..(if w % 10 == 0 { 200 } else { 2 }) {
                g.update(SIM_DT, &mut ctx);
            }
            let label = g.label.clone();
            assert!(label.is_ascii(), "{w}: {label}");
            // Never clipped: the label plus a 1-cell margin each side fits, or it is empty.
            assert!(label.is_empty() || label.len() + 2 <= usize::from(w), "{w}: {label}");
            assert!(!label.contains("4t"), "{w}: stale estimator name: {label}");
            if !label.is_empty() {
                assert!(label.starts_with("D^ (MSD@2s) = "), "{w}: {label}");
            }
            // Parts are dropped, never the estimator: 20 columns is enough for it.
            if w >= 20 {
                assert!(!label.is_empty(), "{w}: readout missing");
            }
            // The frame shows exactly the label on the bottom row.
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut canvas);
            let bottom = &canvas.to_lines()[19];
            assert_eq!(bottom.contains("D^ (MSD@2s)"), !label.is_empty(), "{w}: {bottom}");
        }
        // Wide terminals get every part.
        g.resize(Size::new(120, 30), &mut ctx);
        for _ in 0..200 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.label.contains("D = 2.25 rows^2/s") && g.label.contains("t = "));
    }

    #[test]
    fn bro_renders_grain_trail_molecules_and_readout_in_ascii() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        for _ in 0..120 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let lines = frame.to_lines();
        assert!(lines.iter().all(|l| l.is_ascii()));
        let roles: Vec<_> = frame.cells().iter().map(|c| c.ink.role).collect();
        for r in [Role::Accent, Role::Structure, Role::Particle, Role::Text] {
            assert!(roles.contains(&r), "{r:?} missing");
        }
        assert!(lines[29].contains("D^"), "{}", lines[29]);
    }
}
