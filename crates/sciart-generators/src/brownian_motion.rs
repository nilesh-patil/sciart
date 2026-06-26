//! A micron-sized grain (like the particles Brown saw released from pollen) jittering among
//! fast solvent molecules, leaving a fading random-walk trail, with a live
//! diffusion-constant readout (SPEC-002 `brownian_motion`). The readout's estimator divides
//! by `⟨r²(t)⟩` of free Langevin (OU) motion, which tends to `4Dt` for `t ≫ 1/γ`. (The M12
//! dotted rms-displacement ring was dropped at M14 as decoration, ADR-0017 rule 4.)
//!
//! Physics: the grain's velocity is an Ornstein–Uhlenbeck process
//! `dv = −γ v dt + σ dW` (Langevin equation without a potential). Position and velocity are
//! advanced with the *exact* joint Gaussian transition of the OU process ([`OuStep`]), so
//! the statistics (stationary variance `σ²/(2γ)`, diffusion constant `D = σ²/(2γ²)`) are
//! exact for any frame step `dt` — no sub-stepping and no step-size bias.
//!
//! Tracking stage (M12): the grain walks in unbounded free space, and the view is a
//! microscope stage that follows it. When the grain drifts out of the central band the
//! stage glides (exponential ease, [`STAGE_TAU`]) until it is centred again, so the hero
//! never hugs a wall. The solvent is a homogeneous bath seen through the moving window:
//! molecules live in view coordinates, periodic across the screen ([`wrap`]), and shift
//! with the stage. The trail is in world coordinates and glides with it.
//!
//! Raster (M12): solvent and trail are braille [`Dots`] and the grain a shaded
//! half-block [`Pixels`] sphere, both rasterised in `update`/`resize`; `render` only blits
//! and writes the readout.

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
/// grain in water forgets its velocity in ~0.1 µs), so the ballistic jitter of the path is
/// visible (SPEC-002 Stylised).
const GAMMA: f64 = 2.0;
/// Diffusion constant, rows²/s: `√(4 D · 1 s) = 4` rows of rms 2-D displacement per √s.
const D: f64 = 4.0;
/// Noise strength σ = γ √(2D), from `D = σ²/(2γ²)` (4.2426… rows/s^{3/2}).
const SIGMA: f64 = GAMMA * 2.828_427_124_746_19;
/// Sampling tick for the MSD estimator, s.
const TICK: f64 = 0.125;
/// The trail takes `TRAIL_PER_TICK` samples per tick (32 Hz) and keeps 32 s of path.
const TRAIL_PER_TICK: usize = 4;
pub(crate) const TRAIL_LEN: usize = 1024;
/// MSD lag τ for the D̂ estimator, in ticks (2 s ≫ 1/γ = 0.5 s).
const LAG_TICKS: usize = 16;
/// Lag samples (ticks) before the readout shows D̂: 10 s of lag samples, so about 12 s of
/// track, since the first lands once the 2 s lag window has filled. A single path's first
/// few seconds hold under one independent window, and its estimate swings by a factor of
/// four; the readout waits rather than show that as the simulation disagreeing with itself.
const LAG_MIN: u64 = 80;
/// Solvent molecules per cell and their hard cap (GEN-R8).
const MOL_PER_CELL: f64 = 0.16;
const MOL_MAX: usize = 1600;
/// Maxwell–Boltzmann velocity standard deviation per axis for the molecules, rows/s.
const MOL_SD: f64 = 7.0;
/// A molecule that hits the grain glows, decaying with this time constant, s (BRO-A4).
const GLOW_TAU: f64 = 0.3;
/// A molecule is drawn with a dimmer tail dot where it was this long ago, s, so the
/// solvent reads as motion rather than salt.
const MOL_STREAK: f64 = 0.06;
/// Largest frame step honoured (GEN-R4's range); bounds the per-frame cost.
const MAX_DT: f64 = 0.25;
/// Every resize starts from a pre-rolled past: this much exact OU track (trail and MSD
/// lags) is simulated before the first frame, so the trail and the estimate both show
/// from t = 0. The velocity starts stationary, so this is an honest past.
const PREROLL: f64 = 20.0;
/// Above this glow a molecule is drawn as a 5-dot bead instead of one dot.
const GLOW_BEAD: f64 = 0.35;
/// Brightest a flash is drawn: below the trail's hot head (BRO-A6).
const GLOW_TOP: f64 = 0.6;
/// The solvent is a quiet ground: its dots sit at `MOL_BASE + MOL_SPEED·speed` (≤ 0.2).
const MOL_BASE: f64 = 0.15;
const MOL_SPEED: f64 = 0.05;
/// The newest this many seconds of trail are drawn hot (BRO-A6).
const TRAIL_HOT: f64 = 10.0;
/// The newest this many seconds of trail are a two-dot stroke (BRO-A6).
const TRAIL_BOLD: f64 = 8.0;
const _: () = assert!(TRAIL_BOLD <= TRAIL_HOT, "only the hot head is bold");
const _: () = assert!(GLOW_TOP < 0.85, "flashes stay below the hot trail (BRO-A6)");
/// Molecules bounce off the grain at this extra distance, rows.
const CONTACT_PAD: f64 = 0.35;
/// Grain radius as a fraction of the box's short side (rows), and its limits.
const GRAIN_FRAC: f64 = 0.06;
const GRAIN_R: (f64, f64) = (0.6, 3.5);
/// The stage starts to recentre when the grain is more than this fraction of the view's
/// half extent (per axis) from the centre, and stops once it is within `STAGE_DONE` of
/// that band; it closes the gap with time constant `STAGE_TAU`, s.
const STAGE_BAND: f64 = 0.45;
const STAGE_DONE: f64 = 0.15;
const STAGE_TAU: f64 = 0.9;

/// False colours (BRO-A6): a quiet neutral solvent, a muted grey-white grain (the same
/// near-neutral Starlight ramp, lit far brighter) and its path in ice (cyan → white), the
/// one accent (M14, ADR-0017 rule 2: no violet grain).
const SOLVENT: Ramp = Ramp::Starlight;
const GRAIN: Ramp = Ramp::Starlight;
const PATH: Ramp = Ramp::Ice;

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

/// Wrap `x` periodically into `[lo, hi)` (the solvent seen through the moving stage,
/// BRO-A3). Non-finite `x` or an empty interval map to `lo`.
pub(crate) fn wrap(x: f64, lo: f64, hi: f64) -> f64 {
    let len = hi - lo;
    if !x.is_finite() || len.is_nan() || len <= 0.0 {
        return lo;
    }
    // rem_euclid can round up to `len` for tiny negative inputs.
    let u = (x - lo).rem_euclid(len);
    if u >= len { lo } else { lo + u }
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

/// Trail ramp value for a segment `age` s old (BRO-A6): the newest `TRAIL_HOT` s glow
/// from 1 (white) down to 0.85 (pale cyan), above the solvent and its flashes;
/// older path fades from 0.7 to 0.3 at the 32 s tail.
pub(crate) fn trail_value(age: f64) -> f64 {
    let span = TRAIL_LEN as f64 * TICK / TRAIL_PER_TICK as f64;
    if age < TRAIL_HOT {
        1.0 - 0.15 * age / TRAIL_HOT
    } else {
        let fresh = (1.0 - (age - TRAIL_HOT) / (span - TRAIL_HOT)).clamp(0.0, 1.0);
        0.3 + 0.4 * fresh.powf(1.2)
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
    /// Free grain state `(x, y, vx, vy)` in world rows and rows/s.
    grain: [f64; 4],
    grain_r: f64,
    /// The view: the periodic solvent box (view rows) and the band the grain's centre is
    /// kept in (BRO-A3).
    mol_box: Bounds,
    grain_box: Bounds,
    /// The stage: the world point shown at the view's centre, and whether it is gliding.
    cam: (f64, f64),
    recentring: bool,
    /// World grain positions, oldest → newest (BRO-A6).
    trail: Trail<(f64, f64)>,
    /// Unfolded positions over the last lag τ, for the time-averaged MSD.
    lag: Trail<(f64, f64)>,
    msd_sum: f64,
    msd_count: u64,
    molecules: ParticlePool<Molecule>,
    /// Solvent and trail; the grain. Rasterised in `update`/`resize`.
    dots: Dots,
    pixels: Pixels,
    label: String,
    label_row: Option<i32>,
    /// Chars the readout may take on its row, right of the title (GEN-R13).
    room: usize,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for BrownianMotion {
    fn default() -> Self {
        Self {
            dirty: true,
            t: 0.0,
            tick_clock: 0.0,
            grain: [0.0; 4],
            grain_r: 1.0,
            mol_box: Bounds::default(),
            grain_box: Bounds::default(),
            cam: (0.0, 0.0),
            recentring: false,
            trail: Trail::new(TRAIL_LEN),
            lag: Trail::new(LAG_TICKS + 1),
            msd_sum: 0.0,
            msd_count: 0,
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
    /// Grain centre in world rows.
    fn grain_pos(&self) -> (f64, f64) {
        (self.grain[0], self.grain[1])
    }

    /// View rows of world point `p`: the stage puts `cam` at the view's centre.
    fn to_view(&self, p: (f64, f64)) -> (f64, f64) {
        let b = &self.grain_box;
        (
            p.0 - self.cam.0 + 0.5 * (b.x0 + b.x1),
            p.1 - self.cam.1 + 0.5 * (b.y0 + b.y1),
        )
    }

    /// Glide the stage after a step of `dt` (BRO-A3): start when the grain leaves the
    /// central band, ease towards it, stop once it is well inside; never let the grain's
    /// disc leave the view. Returns the stage's shift, which the solvent follows.
    fn track(&mut self, dt: f64) -> (f64, f64) {
        let b = self.grain_box;
        let half = (0.5 * (b.x1 - b.x0), 0.5 * (b.y1 - b.y0));
        let (ex, ey) = (self.grain[0] - self.cam.0, self.grain[1] - self.cam.1);
        let (bx, by) = (STAGE_BAND * half.0, STAGE_BAND * half.1);
        if ex.abs() > bx || ey.abs() > by {
            self.recentring = true;
        }
        let old = self.cam;
        if self.recentring {
            let k = -(-dt / STAGE_TAU).exp_m1();
            self.cam.0 += k * ex;
            self.cam.1 += k * ey;
            let (rx, ry) = (self.grain[0] - self.cam.0, self.grain[1] - self.cam.1);
            if rx.abs() <= STAGE_DONE * bx && ry.abs() <= STAGE_DONE * by {
                self.recentring = false;
            }
        }
        // Hard limit: the grain's centre stays inside the view's grain band.
        self.cam.0 = self
            .cam
            .0
            .clamp(self.grain[0] - half.0, self.grain[0] + half.0);
        self.cam.1 = self
            .cam
            .1
            .clamp(self.grain[1] - half.1, self.grain[1] + half.1);
        (self.cam.0 - old.0, self.cam.1 - old.1)
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

    /// Readout, e.g. `D 1-path 2.31 rows^2/s  true 4.00  t 42s`: D estimated from this
    /// grain's own track (time-averaged MSD at a 2 s lag, inertia-corrected, see [`d_hat`];
    /// the pre-rolled past gives it `LAG_MIN` lags from the first frame, `...` otherwise)
    /// with its unit, then the true D, then the track's age. Trailing parts are dropped
    /// rather than clipped (GEN-R13), so even the shortest form carries the unit; when the
    /// estimate does not fit, the label is empty.
    ///
    /// [`d_hat`]: Self::d_hat
    fn refresh_label(&mut self) {
        let d_hat = self.d_hat();
        // Longest first; the first that fits the room wins.
        for parts in (1..=3).rev() {
            self.label.clear();
            let _ = match d_hat {
                Some(d) => write!(self.label, "D 1-path {d:.2} rows^2/s"),
                None => write!(self.label, "D 1-path ... rows^2/s"),
            };
            if parts >= 2 {
                let _ = write!(self.label, "  true {D:.2}");
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

    /// Advance the grain by `dt`, recording trail samples and MSD lags. The OU transition
    /// is exact for any h, so the step is split at trail-sample boundaries: samples land
    /// at exact multiples of their ticks whatever dt is.
    fn advance_grain(&mut self, dt: f64, rng: &mut impl Rng) {
        let sample = TICK / TRAIL_PER_TICK as f64;
        let mut left = dt;
        while left > 0.0 {
            let next = sample * ((self.tick_clock / sample + 1e-9).floor() + 1.0);
            let h = left.min(next - self.tick_clock);
            let step = OuStep::new(GAMMA, SIGMA, h);
            ou_step_2d(&step, &mut self.grain, rng);
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
    }

    /// Move the solvent by `dt` in view rows, the stage having shifted by `shift` world
    /// rows over the same step (the bath slides the other way), periodic across the view.
    fn move_molecules(&mut self, dt: f64, shift: (f64, f64)) {
        let b = self.mol_box;
        let (gx, gy) = self.to_view(self.grain_pos());
        let reach = self.grain_r + CONTACT_PAD;
        let fade = (-dt / GLOW_TAU).exp();
        for m in self.molecules.iter_mut() {
            m.glow *= fade;
            m.x = wrap(m.x + m.vx * dt - shift.0, b.x0, b.x1);
            m.y = wrap(m.y + m.vy * dt - shift.1, b.y0, b.y1);
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
                m.x = wrap(gx + nx * reach, b.x0, b.x1);
                m.y = wrap(gy + ny * reach, b.y0, b.y1);
            }
        }
    }

    /// Draw everything but the readout (the raster convention, ADR-0010).
    fn rasterise(&mut self) {
        self.dots.clear();
        // First dot row not drawn: the readout's (GEN-R13).
        let floor = self.label_row.map_or(i32::MAX, |row| row * Dots::CELL_H);

        // Solvent: one dim dot per molecule, a little brighter when fast, flashing after
        // a hit: a quiet ground for the path.
        for m in self.molecules.iter() {
            let speed = (m.vx.hypot(m.vy) / (3.0 * MOL_SD)).min(1.0);
            let v = (MOL_BASE + MOL_SPEED * speed).max(m.glow);
            let (x, y) = dot_of((m.x, m.y));
            // A fresh hit is a small bead; the flashes stand for the kicks (decorative:
            // they move nothing, SPEC-002 Stylised). Capped below the trail's head so a
            // flash never paints over the hero path.
            let r = if m.glow > GLOW_BEAD { 1.0 } else { 0.0 };
            let ink = ink::ramp(SOLVENT, v.min(GLOW_TOP));
            // A dimmer tail dot where it was MOL_STREAK s ago, so it reads as moving.
            let (tx, ty) = dot_of((m.x - m.vx * MOL_STREAK, m.y - m.vy * MOL_STREAK));
            if (ty.round() as i32) < floor {
                self.dots
                    .plot(tx, ty, ink::ramp(SOLVENT, 0.6 * v.min(GLOW_TOP)));
            }
            self.dots.disc(x, y, r, ink);
        }

        // Trail: the path so far, oldest dimmest, joined to the grain; the newest
        // `TRAIL_BOLD` s as a two-dot stroke, so the walk outweighs the solvent speckle.
        let n = self.trail.len();
        let world_head = self.grain_pos();
        let newer = self.trail.iter().skip(1).chain([&world_head]);
        for (k, (&p, &q)) in self.trail.iter().zip(newer).enumerate() {
            let age = (n - 1 - k) as f64 * TICK / TRAIL_PER_TICK as f64;
            let ink = ink::ramp(PATH, trail_value(age));
            let (p, q) = (dot_of(self.to_view(p)), dot_of(self.to_view(q)));
            stroke(&mut self.dots, p, q, ink, age < TRAIL_BOLD);
        }
        let head = self.to_view(world_head);

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
                let v = 0.45 + 0.4 * lambert + 0.15 * lambert.powi(12);
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
        // The solvent is periodic over the whole view; the grain's centre is kept a radius
        // inside the drawable span.
        self.mol_box = Bounds {
            x0: 0.0,
            x1: w / ASPECT,
            y0: 0.0,
            y1: rows,
        };
        self.grain_r = (GRAIN_FRAC * rows.min(w / ASPECT)).clamp(GRAIN_R.0, GRAIN_R.1);
        self.grain_box = Bounds {
            x0: 0.0,
            x1: (w - 1.0) / ASPECT,
            y0: 0.0,
            y1: rows - 1.0,
        }
        .inset(self.grain_r);
        self.dots.resize(size);
        self.pixels.resize(size);

        // A fresh run: grain at the world origin with a stationary velocity.
        let (vx, vy) = gaussian2(&mut ctx.rng, v_stationary_sd());
        self.grain = [0.0, 0.0, vx, vy];
        self.t = 0.0;
        self.tick_clock = 0.0;
        self.trail.clear();
        self.lag.clear();
        self.msd_sum = 0.0;
        self.msd_count = 0;
        self.tick();
        self.trail.push(self.grain_pos());
        // The pre-rolled past (BRO-A7): the scene opens on a grain that has already been
        // walking for PREROLL s, with its trail, lags and an estimate.
        self.advance_grain(PREROLL, &mut ctx.rng);
        self.t = PREROLL;
        // The stage opens centred on the grain.
        self.cam = self.grain_pos();
        self.recentring = false;

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
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        // Also rejects +∞, which would never be split down to the sample ticks below.
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let dt = dt.min(MAX_DT);
        self.advance_grain(dt, &mut ctx.rng);
        let shift = self.track(dt);
        self.t += dt;
        // Substeps short enough that a 3σ-fast molecule moves at most half the contact
        // reach, so none tunnels through the grain at a large dt.
        let reach = self.grain_r + CONTACT_PAD;
        let subs = (dt * 3.0 * MOL_SD / (0.5 * reach)).ceil().clamp(1.0, 16.0);
        let part = (shift.0 / subs, shift.1 / subs);
        for _ in 0..subs as usize {
            self.move_molecules(dt / subs, part);
        }
        self.refresh_label();
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
    fn bro_a1_constants_satisfy_d_equals_sigma2_over_2gamma2() {
        assert!((SIGMA * SIGMA / (2.0 * GAMMA * GAMMA) - D).abs() < 1e-12);
        // ~4 rows of rms 2-D displacement per √s (spec).
        assert!(((4.0 * D).sqrt() - 4.0).abs() < 1e-12);
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
    fn bro_a3_wrap_is_periodic() {
        assert_eq!(wrap(0.5, 0.0, 1.0), 0.5);
        assert!((wrap(1.25, 0.0, 1.0) - 0.25).abs() < 1e-12);
        assert!((wrap(-0.25, 0.0, 1.0) - 0.75).abs() < 1e-12);
        let x = wrap(-1e9 - 0.3, 2.0, 5.0);
        assert!((2.0..5.0).contains(&x));
        assert!((2.0..5.0).contains(&wrap(-1e-17, 2.0, 5.0)));
        assert_eq!(wrap(f64::NAN, 2.0, 5.0), 2.0);
        assert_eq!(wrap(3.0, 2.0, 2.0), 2.0);
    }

    #[test]
    fn bro_a3_grain_stays_in_view_and_molecules_in_the_box_and_trail_is_bounded() {
        let mut ctx = seeded(5);
        let mut g = BrownianMotion::default();
        let dts = [SIM_DT, 0.25, 0.0, 0.1, SIM_DT, 0.013];
        for (w, h) in [(200, 50), (1, 1), (2, 9), (80, 24), (7, 3), (120, 40)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            g.prepare();
            for i in 0..1200 {
                g.update(dts[i % dts.len()], &mut ctx);
                g.prepare();
                let (gx, gy) = g.to_view(g.grain_pos());
                let b = g.grain_box;
                let eps = 1e-9;
                assert!(
                    (b.x0 - eps..=b.x1 + eps).contains(&gx)
                        && (b.y0 - eps..=b.y1 + eps).contains(&gy),
                    "{w}x{h}: grain ({gx}, {gy})"
                );
                for m in g.molecules.iter() {
                    assert!(g.mol_box.contains(m.x, m.y), "{w}x{h}: molecule");
                }
                assert!(g.trail.len() <= TRAIL_LEN);
                assert!(g.molecules.len() <= MOL_MAX);
            }
            // 1200 frames averaging ~0.08 s ≫ 256 ticks: the trail is full, never longer.
            assert_eq!(g.trail.len(), TRAIL_LEN, "{w}x{h}");
        }
    }

    #[test]
    fn bro_a8_the_stage_keeps_the_grain_near_the_centre() {
        // Over 5 minutes at 120x40 the free grain wanders ~ √(4D·300) ≈ 69 rows, several
        // view heights, yet the stage keeps it in the central part of the view: always
        // inside the recentre band plus the distance one glide lags, and mostly well
        // inside it.
        let mut ctx = seeded(17);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(120, 40), &mut ctx);
        g.prepare();
        let b = g.grain_box;
        let half = (0.5 * (b.x1 - b.x0), 0.5 * (b.y1 - b.y0));
        let (mut far, mut inner, mut n) = (0.0f64, 0, 0);
        let start = g.grain_pos();
        for _ in 0..(300.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let (ex, ey) = (g.grain[0] - g.cam.0, g.grain[1] - g.cam.1);
            let e = (ex.abs() / half.0).max(ey.abs() / half.1);
            far = far.max(e);
            if e <= STAGE_BAND {
                inner += 1;
            }
            n += 1;
        }
        let (gx, gy) = g.grain_pos();
        let wandered = (gx - start.0).hypot(gy - start.1);
        assert!(wandered > half.1, "the grain barely moved: {wandered}");
        assert!(far < 0.75, "grain reached {far:.2} of the half view");
        assert!(
            f64::from(inner) > 0.9 * f64::from(n),
            "{inner}/{n} frames in the band"
        );
    }

    #[test]
    fn bro_a7_first_frame_opens_on_a_pre_rolled_past_with_an_estimate() {
        let mut ctx = seeded(8);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(200, 50), &mut ctx);
        g.prepare();
        // 20 s of track: a full-length hot trail, ≥ LAG_MIN lags and an estimate, labelled
        // with its unit and the track's age.
        assert_eq!(g.trail.len(), 1 + (PREROLL * 32.0).round() as usize);
        assert!(g.msd_count >= LAG_MIN, "{}", g.msd_count);
        let d = g.d_hat().expect("an estimate from the first frame");
        assert!(
            g.label.starts_with(&format!("D 1-path {d:.2} rows^2/s")),
            "{}",
            g.label
        );
        assert!(g.label.ends_with("  t 20s"), "{}", g.label);
        // (M14: the rms ring and its pre-rolled epoch are gone, ADR-0017 rule 4.)
        // The estimator itself still waits for LAG_MIN lag samples.
        g.msd_count = LAG_MIN - 1;
        assert_eq!(g.d_hat(), None);
        g.refresh_label();
        assert!(g.label.starts_with("D 1-path ... rows^2/s"), "{}", g.label);
    }

    #[test]
    fn bro_a6_newest_trail_outshines_the_flashes() {
        // Hot head: 1 → 0.85 over 10 s, then a dimmer, monotone fade to 0.3 at the tail.
        assert_eq!(trail_value(0.0), 1.0);
        let span = TRAIL_LEN as f64 * TICK / TRAIL_PER_TICK as f64;
        assert!((trail_value(span) - 0.3).abs() < 1e-12);
        let vs: Vec<f64> = (0..=320).map(|i| trail_value(f64::from(i) * 0.1)).collect();
        assert!(vs.windows(2).all(|w| w[1] <= w[0]), "not monotone");
        // Above the flash cap (0.6), which the 0.85 floor of the hot head clears; the
        // solvent ground stays at or under 0.2. (M14: no ring any more.)
        assert!(vs[..100].iter().all(|&v| v >= 0.85), "{:?}", &vs[..100]);
        const { assert!(GLOW_TOP < 0.85 && MOL_BASE + MOL_SPEED <= 0.2 + 1e-12) };
    }

    #[test]
    fn bro_a6_newest_path_is_a_two_dot_stroke_and_the_rest_a_hairline() {
        // Doubled across its direction: two dots per column when shallow, per row when
        // steep; one when not bold.
        let mut dots = Dots::default();
        dots.resize(Size::new(20, 10));
        let ink = ink::ramp(PATH, trail_value(0.0));
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
        g.prepare();
        for _ in 0..(150.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
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
        let estimator = "D 1-path 2.31 rows^2/s".len();
        let mut ctx = seeded(9);
        let mut g = BrownianMotion::default();
        for w in 1..=120u16 {
            let size = Size::new(w, 20);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            g.prepare();
            for _ in 0..(if w % 10 == 0 { 200 } else { 2 }) {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
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
        g.prepare();
        for _ in 0..200 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert!(g.label.contains(" rows^2/s  true 4.00  t "), "{}", g.label);
    }

    #[test]
    fn bro_a6_gen_r10_renders_grain_trail_molecules_and_readout_in_ascii() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        for _ in 0..120 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let lines = frame.to_lines();
        assert!(lines.iter().all(|l| l.is_ascii()));
        let roles: Vec<_> = frame.cells().iter().map(|c| c.ink.role).collect();
        for r in [
            Role::Ramp(GRAIN),
            Role::Ramp(PATH),
            Role::Ramp(SOLVENT),
            Role::Text,
        ] {
            assert!(roles.contains(&r), "{r:?} missing");
        }
        assert!(lines[29].contains("D 1-path"), "{}", lines[29]);
    }

    #[test]
    fn bro_a4_a_molecule_that_hits_the_grain_glows_then_fades() {
        let mut ctx = seeded(4);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        let (gx, gy) = g.to_view(g.grain_pos());
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
        g.move_molecules(SIM_DT, (0.0, 0.0));
        let m = *g.molecules.iter().next().expect("one molecule");
        assert_eq!(m.glow, 1.0);
        assert!(m.vx > 0.0, "bounced: {}", m.vx);
        for _ in 0..60 {
            g.move_molecules(SIM_DT, (0.0, 0.0));
        }
        let glow = g.molecules.iter().next().expect("one molecule").glow;
        assert!((glow - (-1.0 / GLOW_TAU).exp()).abs() < 1e-9, "{glow}");
    }

    #[test]
    fn bro_a4_a_fast_molecule_at_the_largest_dt_bounces_instead_of_tunnelling() {
        let mut ctx = seeded(4);
        let mut g = BrownianMotion::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        let (gx, gy) = g.to_view(g.grain_pos());
        g.molecules.clear();
        let reach = g.grain_r + CONTACT_PAD;
        // A 3σ-fast molecule would cross the whole grain within one unsplit 0.25 s move.
        g.molecules.spawn(Molecule {
            x: gx + reach + 0.1,
            y: gy,
            vx: -3.0 * MOL_SD,
            vy: 0.0,
            glow: 0.0,
        });
        // A huge dt is clamped to 0.25 s, so the frame's work stays bounded (GEN-R4).
        g.update(1e9, &mut ctx);
        g.prepare();
        assert!((g.t - PREROLL - MAX_DT).abs() < 1e-12, "{}", g.t);
        let m = *g.molecules.iter().next().expect("one molecule");
        assert!(m.glow > 0.0 && m.vx > 0.0, "tunnelled: {m:?}");
    }

    #[test]
    fn bro_a5_rms_displacement_encloses_63_percent_of_free_walkers() {
        // M14: the ring is no longer drawn, but its model stays the readout's reference:
        // the free-space rms displacement √msd_ou(t). An OU walker started from its
        // stationary velocity has a 2-D Gaussian displacement, so it lies inside with
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
    fn bro_a6_trail_is_32_s_at_32_hz_and_the_grain_is_a_round_pixel_sphere() {
        assert!((TRAIL_LEN as f64 * TICK / TRAIL_PER_TICK as f64 - 32.0).abs() < 1e-12);
        let mut ctx = seeded(12);
        let mut g = BrownianMotion::default();
        let size = Size::new(120, 40);
        g.resize(size, &mut ctx);
        g.prepare();
        for _ in 0..(30.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(g.trail.len(), TRAIL_LEN);
        // The grain is drawn where the stage puts it: near the view's centre.
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
