//! A micron-sized grain (like the particles Brown saw released from pollen) jittering among
//! fast solvent molecules, leaving a fading random-walk trail inside a growing
//! rms-displacement ring, with a live diffusion-constant readout (SPEC-002
//! `brownian_motion`). The ring's radius is `√⟨r²(t)⟩` of free Langevin (OU) motion, which
//! tends to `√(4Dt)` for `t ≫ 1/γ`.
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
//!
//! Raster (M12): solvent, trail and ring are braille [`Dots`] and the grain a shaded
//! half-block [`Pixels`] sphere, both rasterised in `update`/`resize`; `render` only blits
//! and writes the readout.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::particles::gaussian2;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{draw, ink};

pub static META: Meta = Meta {
    name: "brownian_motion",
    category: Category::Physics,
    description: "A micron-sized grain jittering among solvent molecules (Langevin dynamics)",
    title: "Brownian motion · random kicks",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(BrownianMotion::default())
}

/// Friction rate γ, 1/s. The velocity memory 1/γ = 0.5 s is stretched ~10⁶× (a real micron
/// grain in water forgets its velocity in ~0.1 µs), so the ballistic start of the ring is
/// visible (SPEC-002 Stylised).
const GAMMA: f64 = 2.0;
/// Diffusion constant, rows²/s: `√(4 D · 1 s) = 3` rows of rms 2-D displacement per √s.
const D: f64 = 2.25;
/// Noise strength σ = γ √(2D), from `D = σ²/(2γ²)` (4.2426… rows/s^{3/2}).
const SIGMA: f64 = GAMMA * 2.121_320_343_559_642_4;
/// Sampling tick for the MSD estimator, s.
const TICK: f64 = 0.125;
/// The trail takes `TRAIL_PER_TICK` samples per tick (32 Hz) and keeps 22 s of path.
const TRAIL_PER_TICK: usize = 4;
pub(crate) const TRAIL_LEN: usize = 704;
/// MSD lag τ for the D̂ estimator, in ticks (2 s ≫ 1/γ = 0.5 s).
const LAG_TICKS: usize = 16;
/// Lag samples (ticks) before the readout shows D̂: 10 s of lag samples, so about 12 s of
/// track, since the first lands once the 2 s lag window has filled. A single path's first
/// few seconds hold under one independent window, and its estimate swings by a factor of
/// four; the readout waits rather than show that as the simulation disagreeing with itself.
const LAG_MIN: u64 = 80;
/// Solvent molecules per cell and their hard cap (GEN-R8).
const MOL_PER_CELL: f64 = 0.12;
const MOL_MAX: usize = 1600;
/// Maxwell–Boltzmann velocity standard deviation per axis for the molecules, rows/s.
const MOL_SD: f64 = 7.0;
/// A molecule that hits the grain glows, decaying with this time constant, s (BRO-A4).
const GLOW_TAU: f64 = 0.3;
/// Above this glow a molecule is drawn as a 5-dot bead instead of one dot.
const GLOW_BEAD: f64 = 0.35;
/// Brightest a flash is drawn: below the trail's hot head (BRO-A6).
const GLOW_TOP: f64 = 0.68;
/// The newest this many seconds of trail are drawn hot (BRO-A6).
const TRAIL_HOT: f64 = 5.0;
/// The newest this many seconds of trail are a two-dot stroke (BRO-A6).
const TRAIL_BOLD: f64 = 3.0;
const _: () = assert!(TRAIL_BOLD <= TRAIL_HOT, "only the hot head is bold");
const _: () = assert!(GLOW_TOP < 0.7, "flashes stay below the hot trail (BRO-A6)");
/// Molecules bounce off the grain at this extra distance, rows.
const CONTACT_PAD: f64 = 0.35;
/// Grain radius as a fraction of the box's short side (rows), and its limits.
const GRAIN_FRAC: f64 = 0.075;
const GRAIN_R: (f64, f64) = (0.6, 3.5);
/// Rms-displacement ring epochs: at most this long, and the ring stops growing at this fraction of
/// the box's half short side (BRO-A5).
const EPOCH_MAX: f64 = 16.0;
const RING_REACH: f64 = 0.8;
/// The ring fades in and out over this long at each end of its epoch, s.
const RING_FADE: f64 = 0.8;

/// False colours (BRO-A6): cold solvent, a warm grain and its warm path.
const SOLVENT: Ramp = Ramp::Ice;
const GRAIN: Ramp = Ramp::Ember;

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
    /// Collision glow, 1 at a hit, decaying with `GLOW_TAU` (BRO-A4).
    glow: f64,
}

/// Length of the epoch of the rms-displacement ring for a box of half short side `half` rows: the
/// time for the free-space rms displacement to reach `RING_REACH·half`, capped at
/// `EPOCH_MAX` (BRO-A5). While the ring is well inside the box the walls barely matter.
pub(crate) fn epoch_len(half: f64) -> f64 {
    let reach = RING_REACH * half.max(0.0);
    // 4D·t ≈ msd_ou(t) + 4D/γ for t ≫ 1/γ; solve the exact curve by bisection instead.
    let (mut lo, mut hi) = (0.0, EPOCH_MAX);
    if msd_ou(hi) <= reach * reach {
        return EPOCH_MAX;
    }
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if msd_ou(mid) < reach * reach {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Trail ramp value for a segment `age` s old (BRO-A6): the newest `TRAIL_HOT` s glow
/// from 0.8 down to 0.7, above the ring, the solvent and its flashes; older path fades
/// from 0.55 to 0.2 at the 22 s tail.
pub(crate) fn trail_value(age: f64) -> f64 {
    let span = TRAIL_LEN as f64 * TICK / TRAIL_PER_TICK as f64;
    if age < TRAIL_HOT {
        0.8 - 0.1 * age / TRAIL_HOT
    } else {
        let fresh = (1.0 - (age - TRAIL_HOT) / (span - TRAIL_HOT)).clamp(0.0, 1.0);
        0.2 + 0.35 * fresh.powf(1.2)
    }
}

/// Draw a trail segment between dots `p` and `q`, doubled one dot across its direction
/// when `bold`, so the newest path reads as a stroke rather than a hairline (BRO-A6).
fn stroke(dots: &mut Dots, p: (f64, f64), q: (f64, f64), ink: Ink, bold: bool) {
    dots.line(p.0, p.1, q.0, q.1, ink);
    if bold {
        let (ox, oy) = if (q.0 - p.0).abs() >= (q.1 - p.1).abs() {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        };
        dots.line(p.0 + ox, p.1 + oy, q.0 + ox, q.1 + oy, ink);
    }
}

/// Ring brightness factor at `age` s into an epoch of `len` s: faded in and out.
fn ring_fade(age: f64, len: f64) -> f64 {
    let f = |u: f64| (u / RING_FADE).clamp(0.0, 1.0);
    f(age).min(f(len - age))
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
    /// Folded grain positions, oldest → newest (BRO-A3, BRO-A6).
    trail: Trail<(f64, f64)>,
    /// Unfolded positions over the last lag τ, for the time-averaged MSD.
    lag: Trail<(f64, f64)>,
    msd_sum: f64,
    msd_count: u64,
    /// The rms-displacement ring: its centre (folded grain position at the epoch start), age and
    /// epoch length (BRO-A5).
    ring_origin: (f64, f64),
    ring_age: f64,
    epoch: f64,
    molecules: ParticlePool<Molecule>,
    /// Solvent, trail and ring; the grain. Rasterised in `update`/`resize`.
    dots: Dots,
    pixels: Pixels,
    label: String,
    label_row: Option<i32>,
    /// Chars the readout may take on its row, right of the title (GEN-R13).
    room: usize,
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
            ring_origin: (0.0, 0.0),
            ring_age: 0.0,
            epoch: EPOCH_MAX,
            molecules: ParticlePool::default(),
            dots: Dots::default(),
            pixels: Pixels::default(),
            label: String::with_capacity(64),
            label_row: None,
            room: 0,
        }
    }
}

/// Dot coordinates of a point in rows (x already divided by `ASPECT`).
#[inline]
fn dot_of((x, y): (f64, f64)) -> (f64, f64) {
    Dots::from_cells(x * ASPECT, y)
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
        (self.msd_count >= LAG_MIN).then(|| {
            let tau = LAG_TICKS as f64 * TICK;
            self.msd_sum / self.msd_count as f64 / msd_ou(tau) * D
        })
    }

    fn tick(&mut self) {
        self.lag.push((self.grain[0], self.grain[1]));
        if self.lag.len() == LAG_TICKS + 1
            && let (Some(&old), Some(new)) = (self.lag.iter().next(), self.lag.newest())
        {
            self.msd_sum += (new.0 - old.0).powi(2) + (new.1 - old.1).powi(2);
            self.msd_count += 1;
        }
    }

    /// Readout, e.g. `D 1-path 2.31  true 2.25 rows^2/s  t 42s`: D estimated from this
    /// grain's own track (time-averaged MSD at a 2 s lag, inertia-corrected, see [`d_hat`];
    /// `...` until `LAG_MIN` lags), then the true D, then the elapsed time. Trailing parts
    /// are dropped rather than clipped (GEN-R13); when even the estimate does not fit, the
    /// label is empty. At 80 columns, beside the full title, the time is dropped.
    ///
    /// [`d_hat`]: Self::d_hat
    fn refresh_label(&mut self) {
        let d_hat = self.d_hat();
        // Longest first; the first that fits the room wins.
        for parts in (1..=3).rev() {
            self.label.clear();
            let _ = match d_hat {
                Some(d) => write!(self.label, "D 1-path {d:.2}"),
                None => write!(self.label, "D 1-path ..."),
            };
            if parts >= 2 {
                let _ = write!(self.label, "  true {D:.2} rows^2/s");
            }
            if parts >= 3 {
                let _ = write!(self.label, "  t {:.0}s", self.t);
            }
            if self.label.len() <= self.room {
                return;
            }
        }
        self.label.clear();
    }

    fn move_molecules(&mut self, dt: f64) {
        let b = self.mol_box;
        let (gx, gy) = self.grain_pos();
        let reach = self.grain_r + CONTACT_PAD;
        let fade = (-dt / GLOW_TAU).exp();
        for m in self.molecules.iter_mut() {
            m.glow *= fade;
            let (x, fx) = fold(m.x + m.vx * dt, b.x0, b.x1);
            let (y, fy) = fold(m.y + m.vy * dt, b.y0, b.y1);
            (m.x, m.y) = (x, y);
            if fx {
                m.vx = -m.vx;
            }
            if fy {
                m.vy = -m.vy;
            }
            // Specular bounce off the grain; every hit lights the molecule up.
            let (dx, dy) = (m.x - gx, m.y - gy);
            let d2 = dx * dx + dy * dy;
            if d2 < reach * reach && d2 > 1e-12 {
                let d = d2.sqrt();
                let (nx, ny) = (dx / d, dy / d);
                let vn = m.vx * nx + m.vy * ny;
                if vn < 0.0 {
                    m.vx -= 2.0 * vn * nx;
                    m.vy -= 2.0 * vn * ny;
                    m.glow = 1.0;
                }
                m.x = fold(gx + nx * reach, b.x0, b.x1).0;
                m.y = fold(gy + ny * reach, b.y0, b.y1).0;
            }
        }
    }

    /// Start a new ring epoch at the grain's current position (BRO-A5).
    fn new_epoch(&mut self) {
        self.ring_origin = self.grain_pos();
        self.ring_age = 0.0;
    }

    /// Radius of the rms-displacement ring, rows: the exact free-space rms displacement
    /// of Langevin motion after the ring's age, `√msd_ou(age)`, which tends to `√(4Dt)`
    /// once `age ≫ 1/γ` (BRO-A5).
    fn ring_radius(&self) -> f64 {
        msd_ou(self.ring_age).sqrt()
    }

    /// Draw everything but the readout (the raster convention, ADR-0010).
    fn rasterise(&mut self) {
        self.dots.clear();
        // First dot row not drawn: the readout's (GEN-R13).
        let floor = self.label_row.map_or(i32::MAX, |row| row * Dots::CELL_H);

        // The ring: a thin dotted circle, one dot in two, around the epoch's start.
        let fade = ring_fade(self.ring_age, self.epoch);
        let r = self.ring_radius();
        if fade > 0.0 && r > 0.5 {
            let ink = ink::ramp(SOLVENT, 0.2 + 0.25 * fade);
            let (cx, cy) = dot_of(self.ring_origin);
            let rd = r * f64::from(Dots::CELL_H);
            let n = (TAU * rd).ceil().min(4096.0) as usize;
            for k in (0..n).step_by(2) {
                let (s, c) = (TAU * k as f64 / n as f64).sin_cos_det();
                let (x, y) = ((cx + rd * c).round(), (cy + rd * s).round());
                if (y as i32) < floor {
                    self.dots.plot(x, y, ink);
                }
            }
            // A small cross marks where the epoch started.
            for d in -2..=2 {
                let d = f64::from(d);
                self.dots.plot(cx + d, cy, ink);
                self.dots.plot(cx, cy + d, ink);
            }
        }

        // Solvent: one dot per molecule, brighter when fast, white-hot after a hit.
        for m in self.molecules.iter() {
            let speed = (m.vx.hypot(m.vy) / (3.0 * MOL_SD)).min(1.0);
            let v = (0.3 + 0.35 * speed).max(m.glow);
            let (x, y) = dot_of((m.x, m.y));
            // A fresh hit is a small bead; the flashes stand for the kicks (decorative:
            // they move nothing, SPEC-002 Stylised). Capped below the trail's head so a
            // flash never paints over the hero path.
            let r = if m.glow > GLOW_BEAD { 1.0 } else { 0.0 };
            self.dots.disc(x, y, r, ink::ramp(SOLVENT, v.min(GLOW_TOP)));
        }

        // Trail: the path so far, oldest dimmest, joined to the grain; the newest
        // `TRAIL_BOLD` s as a two-dot stroke, so the walk outweighs the solvent speckle.
        let n = self.trail.len();
        let head = self.grain_pos();
        let newer = self.trail.iter().skip(1).chain([&head]);
        for (k, (&p, &q)) in self.trail.iter().zip(newer).enumerate() {
            let age = (n - 1 - k) as f64 * TICK / TRAIL_PER_TICK as f64;
            let ink = ink::ramp(GRAIN, trail_value(age));
            stroke(&mut self.dots, dot_of(p), dot_of(q), ink, age < TRAIL_BOLD);
        }

        // The grain: a sphere lit from the upper left, on square half-block pixels.
        self.pixels.clear();
        let (gx, gy, r) = (head.0, head.1, self.grain_r);
        // Pixel (px, py) is centred at ((px + ½)/ASPECT, (py + ½)/2) rows.
        let px = |x: f64| (x * ASPECT).floor() as i32;
        let py = |y: f64| (2.0 * y).floor() as i32;
        const LIGHT: [f64; 3] = [-0.48, -0.56, 0.675];
        for y in py(gy - r)..=py(gy + r) {
            for x in px(gx - r)..=px(gx + r) {
                let (dx, dy) = (
                    (f64::from(x) + 0.5) / ASPECT - gx,
                    (f64::from(y) + 0.5) / 2.0 - gy,
                );
                let rr = (dx * dx + dy * dy) / (r * r);
                if rr > 1.0 {
                    continue;
                }
                let n = [dx / r, dy / r, (1.0 - rr).sqrt()];
                let lambert = (n[0] * LIGHT[0] + n[1] * LIGHT[1] + n[2] * LIGHT[2]).max(0.0);
                let v = 0.3 + 0.55 * lambert + 0.15 * lambert.powi(12);
                self.pixels.set(x, y, ink::ramp(GRAIN, v));
            }
        }
    }
}

impl Generator for BrownianMotion {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        // Bottom row is the readout when there is room for it.
        self.room = title::readout_room(size, &Title::of(self));
        self.label_row = (size.height() >= 8).then(|| i32::from(size.height()) - 1);
        let rows = if self.label_row.is_some() { h - 1.0 } else { h };
        self.mol_box = Bounds {
            x0: 0.0,
            x1: (w - 1.0) / ASPECT,
            y0: 0.0,
            y1: rows - 1.0,
        };
        self.grain_r = (GRAIN_FRAC * rows.min(w / ASPECT)).clamp(GRAIN_R.0, GRAIN_R.1);
        self.grain_box = self.mol_box.inset(self.grain_r);
        self.dots.resize(size);
        self.pixels.resize(size);

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
        self.trail.push(self.grain_pos());
        let g = self.grain_box;
        self.epoch = epoch_len(0.5 * (g.x1 - g.x0).min(g.y1 - g.y0));
        self.new_epoch();

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
                glow: 0.0,
            });
        }
        self.refresh_label();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        // Also rejects +∞, which would never be split down to the sample ticks below.
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        // The OU transition is exact for any h, so split the frame at trail-sample
        // boundaries: trail and MSD samples land at exact multiples of their ticks
        // whatever dt is.
        let sample = TICK / TRAIL_PER_TICK as f64;
        let mut left = dt;
        while left > 0.0 {
            let next = sample * ((self.tick_clock / sample + 1e-9).floor() + 1.0);
            let h = left.min(next - self.tick_clock);
            let step = OuStep::new(GAMMA, SIGMA, h);
            ou_step_2d(&step, &mut self.grain, &mut ctx.rng);
            self.tick_clock += h;
            left -= h;
            if self.tick_clock >= next - 1e-12 {
                self.trail.push(self.grain_pos());
                if self.tick_clock >= TICK - 1e-12 {
                    self.tick_clock = 0.0;
                    self.tick();
                }
            }
        }
        self.t += dt;
        self.ring_age += dt;
        if self.ring_age >= self.epoch {
            self.new_epoch();
        }
        self.move_molecules(dt);
        self.refresh_label();
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        self.pixels.blit(canvas, |_, _, top, bottom| {
            // ASCII: a grain cell is `@` when both halves are grain, else `o`.
            let lit = [top, bottom].map(|p| p.role != Role::Background);
            let ink = if lit[0] { top } else { bottom };
            match lit {
                [true, true] => Some(('@', ink)),
                [false, false] => None,
                _ => Some(('o', ink)),
            }
        });

        if let Some(row) = self.label_row
            && !self.label.is_empty()
        {
            draw::label_right(
                canvas,
                row,
                &self.label,
                Ink::new(Role::Text, title::READOUT_LEVEL),
            );
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
    fn bro_a7_readout_waits_for_ten_seconds_of_lag_samples() {
        let mut ctx = seeded(8);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(200, 50), &mut ctx);
        // The first lag sample lands at τ = 2 s, so 10 s of lags complete at 12 s.
        for _ in 0..(11.5 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        assert_eq!(g.d_hat(), None);
        assert!(g.label.starts_with("D 1-path ..."), "{}", g.label);
        for _ in 0..(1.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.d_hat().is_some());
    }

    #[test]
    fn bro_a6_newest_trail_outshines_the_ring_and_the_flashes() {
        // Hot head: 0.8 → 0.7 over 5 s, then a dimmer, monotone fade to 0.2 at the tail.
        assert_eq!(trail_value(0.0), 0.8);
        let span = TRAIL_LEN as f64 * TICK / TRAIL_PER_TICK as f64;
        assert!((trail_value(span) - 0.2).abs() < 1e-12);
        let vs: Vec<f64> = (0..=220).map(|i| trail_value(f64::from(i) * 0.1)).collect();
        assert!(vs.windows(2).all(|w| w[1] <= w[0]), "not monotone");
        // Above the flash cap and the ring's brightest (0.2 + 0.25), which the 0.7 floor of
        // the hot head clears.
        assert!(vs[..50].iter().all(|&v| v >= 0.7), "{:?}", &vs[..50]);
    }

    #[test]
    fn bro_a6_newest_path_is_a_two_dot_stroke_and_the_rest_a_hairline() {
        // Doubled across its direction: two dots per column when shallow, per row when
        // steep; one when not bold.
        let mut dots = Dots::default();
        dots.resize(Size::new(20, 10));
        let ink = ink::ramp(GRAIN, trail_value(0.0));
        let per_column = |d: &Dots, x| (0..40).filter(|&y| d.is_lit(x, y)).count();
        let per_row = |d: &Dots, y| (0..40).filter(|&x| d.is_lit(x, y)).count();
        stroke(&mut dots, (2.0, 5.0), (30.0, 12.0), ink, true);
        assert!((2..=30).all(|x| per_column(&dots, x) == 2));
        dots.clear();
        stroke(&mut dots, (5.0, 2.0), (9.0, 30.0), ink, true);
        assert!((2..=30).all(|y| per_row(&dots, y) == 2));
        dots.clear();
        stroke(&mut dots, (2.0, 5.0), (30.0, 12.0), ink, false);
        assert!((2..=30).all(|x| per_column(&dots, x) == 1));
    }

    #[test]
    fn bro_a7_readout_estimates_d_from_a_single_grain() {
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
        assert!(g.label.contains(&format!("D 1-path {d:.2}")), "{}", g.label);
    }

    #[test]
    fn bro_gen_r13_readout_names_the_estimator_and_fits_right_of_the_title() {
        // The spec's 2 s estimator lag.
        assert_eq!(LAG_TICKS as f64 * TICK, 2.0);
        let estimator = "D 1-path 2.31".len();
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
            let room = title::readout_room(size, &Title::new(META.title, None));
            assert!(label.is_ascii(), "{w}: {label}");
            // Never clipped and clear of the title, or empty.
            assert!(label.len() <= room, "{w}: {label}");
            assert!(!label.contains("4t"), "{w}: stale estimator name: {label}");
            if !label.is_empty() {
                assert!(label.starts_with("D 1-path "), "{w}: {label}");
            }
            // Parts are dropped, never the estimator while it fits.
            if room >= estimator {
                assert!(!label.is_empty(), "{w}: readout missing");
            }
            // The frame shows exactly the label, right-aligned on the bottom row.
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut canvas);
            let bottom = &canvas.to_lines()[19];
            assert_eq!(
                bottom.contains("D 1-path"),
                !label.is_empty(),
                "{w}: {bottom}"
            );
            assert!(
                label.is_empty() || bottom.ends_with(&format!(" {label} ")),
                "{w}: {bottom}"
            );
        }
        // 80 columns, beside the full title, show every part, time included.
        g.resize(Size::new(80, 24), &mut ctx);
        for _ in 0..200 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.label.contains("  true 2.25 rows^2/s  t "), "{}", g.label);
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
        for r in [Role::Ramp(GRAIN), Role::Ramp(SOLVENT), Role::Text] {
            assert!(roles.contains(&r), "{r:?} missing");
        }
        assert!(lines[29].contains("D 1-path"), "{}", lines[29]);
    }

    #[test]
    fn bro_a4_a_molecule_that_hits_the_grain_glows_then_fades() {
        let mut ctx = seeded(4);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(80, 24), &mut ctx);
        let (gx, gy) = g.grain_pos();
        g.molecules.clear();
        let reach = g.grain_r + CONTACT_PAD;
        // Heading straight at the grain from just outside contact.
        g.molecules.spawn(Molecule {
            x: gx + reach + 0.05,
            y: gy,
            vx: -6.0,
            vy: 0.0,
            glow: 0.0,
        });
        g.move_molecules(SIM_DT);
        let m = *g.molecules.iter().next().expect("one molecule");
        assert_eq!(m.glow, 1.0);
        assert!(m.vx > 0.0, "bounced: {}", m.vx);
        for _ in 0..60 {
            g.move_molecules(SIM_DT);
        }
        let glow = g.molecules.iter().next().expect("one molecule").glow;
        assert!((glow - (-1.0 / GLOW_TAU).exp()).abs() < 1e-9, "{glow}");
    }

    #[test]
    fn bro_a5_ring_encloses_63_percent_of_free_walkers() {
        // The ring is the free-space rms displacement √msd_ou(t); an OU walker started from
        // its stationary velocity has a 2-D Gaussian displacement, so it lies inside with
        // probability 1 − e^{−1} ≈ 0.632 at every age.
        let n = 4000;
        let mut ctx = seeded(31);
        let mut ws = walkers(n, &mut ctx);
        let h = 0.1;
        for (i, t) in [1.0, 4.0, 12.0].into_iter().enumerate() {
            let done = [0.0, 1.0, 4.0][i];
            advance(&mut ws, h, ((t - done) / h).round() as usize, &mut ctx);
            let r2 = msd_ou(t);
            let inside = ws
                .iter()
                .filter(|w| w[0] * w[0] + w[1] * w[1] <= r2)
                .count();
            let p = inside as f64 / n as f64;
            // s.e. √(p(1−p)/n) ≈ 0.0076: 4 s.e.
            assert!((p - (1.0 - (-1.0f64).exp())).abs() < 0.031, "t {t}: {p}");
        }
    }

    #[test]
    fn bro_a5_epochs_stay_inside_the_box_and_restart_at_the_grain() {
        for half in [0.0, 1.0, 5.0, 11.0, 24.0, 1e6] {
            let e = epoch_len(half);
            assert!((0.0..=EPOCH_MAX).contains(&e), "{half}: {e}");
            if e < EPOCH_MAX {
                let r = msd_ou(e).sqrt();
                assert!((r - RING_REACH * half).abs() < 1e-6, "{half}: {r}");
            }
        }
        assert_eq!(epoch_len(1e6), EPOCH_MAX);
        let mut ctx = seeded(6);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(80, 24), &mut ctx);
        assert!(g.epoch > 3.0 && g.epoch < EPOCH_MAX, "{}", g.epoch);
        let mut restarts = 0;
        for _ in 0..(3.0 * g.epoch / SIM_DT) as usize {
            let age = g.ring_age;
            g.update(SIM_DT, &mut ctx);
            assert!(g.ring_age <= g.epoch);
            if g.ring_age < age {
                restarts += 1;
                assert_eq!(g.ring_origin, g.grain_pos());
            }
        }
        assert!(restarts >= 2, "{restarts}");
        assert_eq!(ring_fade(0.0, 8.0), 0.0);
        assert_eq!(ring_fade(4.0, 8.0), 1.0);
        assert_eq!(ring_fade(8.0, 8.0), 0.0);
    }

    #[test]
    fn bro_a6_trail_is_22_s_at_32_hz_and_the_grain_is_a_round_pixel_sphere() {
        assert!((TRAIL_LEN as f64 * TICK / TRAIL_PER_TICK as f64 - 22.0).abs() < 1e-12);
        let mut ctx = seeded(12);
        let mut g = BrownianMotion::default();
        let size = Size::new(120, 40);
        g.resize(size, &mut ctx);
        for _ in 0..(30.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        assert_eq!(g.trail.len(), TRAIL_LEN);
        // The sphere covers about π r² pixels of 0.5 × 0.5 rows.
        let lit = (0..2 * 40)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| g.pixels.get(x, y).is_some_and(|p| p != Ink::BLANK))
            .count() as f64;
        let area = std::f64::consts::PI * g.grain_r * g.grain_r * 4.0;
        assert!((lit / area - 1.0).abs() < 0.2, "{lit} pixels vs {area}");
        // Solvent and trail never reach the readout row (GEN-R13).
        assert!((0..120).all(|x| g.dots.cell_mask(x, 39) == Some(0)));
    }
}
