//! Electrons from a steady source meet a wall with two slits: the wave spreads through
//! both into a fan of bright lobes and dark nodal rays, and on the screen behind single
//! clicks land one at a time until the fringes emerge from them; a last act aims the
//! same beam at a thin barrier it partly tunnels through (SPEC-002 `quantum_double_slit`).
//!
//! ## Model
//!
//! The 2-D time-dependent Schrödinger equation in lattice units (ħ = 1, 2m = 1, grid
//! spacing 1), `i ∂ψ/∂t = Hψ` with `H = −∇² + V`, on a capped Dirichlet grid. Each step is
//! the product of two Crank–Nicolson (Cayley) half-operators, one per axis,
//!
//! ```text
//! ψ ← (1 + iA_x)⁻¹(1 − iA_x) ψ,  then the same along y,   A_d = (dt/2)(−∂_dd + V/2)
//! ```
//!
//! with the axis order alternated every step (a symmetric splitting over pairs). Each
//! factor is the Cayley transform of a Hermitian matrix when V is real, so the step is
//! exactly unitary (QDS-A1) at any `dt`. Every grid line is a tridiagonal solve with
//! fixed coefficients: the matrices are factored once per potential
//! (`ode::tridiag_factor_c`, CORE-R100) and each step only substitutes
//! (`ode::tridiag_apply_c`), two complex multiply-adds a node.
//!
//! The slit wall is a real potential of 10⁴ (a hard wall that keeps the step unitary),
//! the tunnelling barrier a real step `V₀` above the beam's energy, and the edges a
//! quadratic imaginary potential `−iη` that absorbs outgoing waves instead of reflecting
//! them.
//!
//! The source is a soft line source a few nodes in from the left absorber: after every
//! step `ψ(x_src, y) += S·g(y)·e^{−iφn}`, with `g` a Gaussian narrow enough to light just
//! the two slits and `φ` the step's eigenphase for the carrier `k` (`2 atan(dt(1 − cos k))`),
//! so it radiates a monochromatic beam of wave number `k` both ways (the left half into
//! the absorber). The field therefore settles into a steady diffraction pattern. A new
//! grid starts from the analytic incident beam and runs 3× fast for its first second, so
//! the fan fills the view within a second of any start or resize.
//!
//! ## Detection
//!
//! The screen is a grid column near the right edge. Clicks arrive at a rate proportional
//! to the probability current crossing it (as a share of the beam the source emits) and
//! land at positions sampled from that current (Born rule) through `InverseCdf` with
//! `ctx.rng`.
//! Clicks accumulate in a fixed pool and are drawn on a strip to the right, so the
//! fringes build up from single dots.
//!
//! ## Picture
//!
//! Half-block pixels show the wave as a soft single-hue glow (`Ramp(Aurora)`, a
//! phosphor green): brightness follows `|ψ|` (the square root of the probability
//! density, so the fan's weak outer lobes still read), and where the view resolves the
//! carrier a light shimmer of `Re ψ/|ψ|` rides on it, so wavefronts visibly stream
//! through the steady fan of lobes and dark nodal rays. Faint density fades smoothly to
//! black (no hard cut). Walls and the screen are Starlight; clicks are warm-white braille
//! dots over a glowing histogram of where they landed.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::ink;
use sciart_core::ode::{C64, tridiag_apply_c, tridiag_factor_c};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "quantum_double_slit",
    category: Category::Physics,
    description: "An electron wave packet diffracts through two slits; single clicks build the fringes",
    title: "Double slit · waves of chance",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(QuantumDoubleSlit::new())
}

const CAPTIONS: [&str; 3] = ["interference", "one at a time", "tunnelling"];
/// Act lengths (simulated seconds), in `CAPTIONS` order.
const ACT_S: f64 = 20.0;

/// Node budget of the Schrödinger grid (ADR-0012); the view's aspect is clamped.
const MAX_NODES: usize = 11_000;
const ASPECT: (f64, f64) = (1.0, 3.0);
/// Absorbing layer width on every side, and the part of it hidden from view.
const ABSORB: usize = 12;
const VIEW_MARGIN: usize = 6;
/// Peak absorption rate η of the edge layer (quadratic ramp).
const ETA: f64 = 0.35;

/// Carrier wave number: a wavelength of 6 nodes.
pub(crate) const K_WAVE: f64 = TAU / 6.0;
/// Time step (lattice units) and steps per 60 Hz frame: ≈ 18 time units a second, so a
/// wavefront crosses the grid in ≈ 4 s.
pub(crate) const DT: f64 = 0.15;
const STEPS_PER_FRAME: f64 = 2.0;
/// A new grid's first `FAST_S` seconds run `FAST` times as many steps, so the fan
/// reaches the screen within about a second of a start or resize.
const FAST: f64 = 2.0;
const FAST_S: f64 = 2.0;
/// Step cap per `update`, so a long `dt` slows the scene instead of the frame.
const MAX_STEPS: usize = 6;
/// Source column (nodes in from the left edge) and its strength; the incident amplitude
/// it settles to is `S·SOURCE_GAIN` (QDS-A6).
const SOURCE_X: usize = ABSORB + 3;
const SOURCE_S: f64 = 0.1;
/// Wall and barrier potentials. The packet's kinetic energy per axis is
/// `2(1 − cos k) = 1` for λ = 6.
const V_WALL: f64 = 1.0e4;
pub(crate) const V_BARRIER: f64 = 1.6;
const BARRIER_W: usize = 2;
/// Beam width σ_y as a fraction of the view height: the two slits sit at ±0.18.
const SIGMA_Y_FRAC: f64 = 0.15;
/// Packet length for the norm test (QDS-A1).
#[cfg(test)]
const SIGMA_X: f64 = 8.0;

/// Clicks a second for a beam that reached the screen whole, by act. About 5 % of the
/// beam's current reaches the screen through the slits: ≈ 30 clicks a second while the
/// fringes build, ≈ 10 a second in "one at a time", so single arrivals read; the
/// barrier's share sets the tunnelling rate.
const CLICK_RATE: [f64; 3] = [640.0, 210.0, 150.0];
const CLICK_CAP: usize = 4000;
const CLICKS_PER_FRAME: usize = 24;
/// A click flashes at full level and settles to `CLICK_REST` over `CLICK_FADE_S`.
const CLICK_REST: f64 = 185.0;
const CLICK_FADE_S: f32 = 1.2;
/// Wave visibility by act: act 2 holds the fan almost at full brightness (a dimmed fan
/// read as the scene fading out); the brighter histogram glow carries the bands.
const WAVE_VIS: [f32; 3] = [1.0, 0.92, 1.0];
/// Tone: `v = Re ψ/√peak` drawn as `sign(v)·√|v|`, blank below `CUTOFF`. Each side of
/// the wall has its own peak, so the weak transmitted fan is exposed like the incident
/// packet; the peaks decay with this half-life so a spreading wave stays visible.
const CUTOFF: f64 = 0.2;
/// Below this many pixels per node the carrier aliases: show `|ψ|` instead.
const MIN_PX_PER_NODE: f64 = 0.7;
const PEAK_HALF_LIFE_S: f64 = 4.0;
/// The source side (incident + reflected standing wave) is exposed at this share of the
/// fan's brightness, so the diffraction fan carries the picture.
const SOURCE_EXPOSURE: f64 = 0.55;
/// Click histogram along the screen (fractions of the view height) and its glow levels.
const HIST_BINS: usize = 160;
const GLOW_LEVELS: (f64, f64) = (18.0, 170.0);
/// The glow reaches full strength once this many clicks are in, so a few early clicks
/// do not paint noise bands.
const GLOW_FULL_CLICKS: f64 = 500.0;
/// Binomial smoothing of the histogram: 9 taps (σ = √2 bins ≈ 1 % of the screen).
const GLOW_TAPS: [f64; 9] = [1.0, 8.0, 28.0, 56.0, 70.0, 56.0, 28.0, 8.0, 1.0];
/// The wave's glow ramp and the carrier shimmer's depth.
const WAVE: Ramp = Ramp::Aurora;
const SHIMMER: f64 = 0.25;
const WALL_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 111);
const BARRIER_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 71);
const SCREEN_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 55);

/// Grid layout for one view aspect (square nodes), all in nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Geometry {
    pub w: usize,
    pub h: usize,
    /// First visible node and the visible extent (the rest is absorbing margin).
    pub vx0: usize,
    pub vy0: usize,
    pub view_w: usize,
    pub view_h: usize,
    /// Slit wall (first column), its thickness 2, centre row, slit spacing and width.
    pub xw: usize,
    pub yc: f64,
    pub d: usize,
    pub slit: usize,
    /// Screen column.
    pub xs: usize,
}

impl Geometry {
    /// The largest grid within [`MAX_NODES`] whose view has the clamped `aspect`.
    pub(crate) fn for_aspect(aspect: f64) -> Self {
        let aspect = if aspect.is_finite() {
            aspect.clamp(ASPECT.0, ASPECT.1)
        } else {
            ASPECT.0
        };
        let pad_x = VIEW_MARGIN + ABSORB;
        let pad_y = 2 * VIEW_MARGIN;
        let mut vh = 200;
        while vh > 24 {
            let vw = (aspect * vh as f64).round() as usize;
            if (vw + pad_x) * (vh + pad_y) <= MAX_NODES {
                break;
            }
            vh -= 1;
        }
        let vw = (aspect * vh as f64).round() as usize;
        let (w, h) = (vw + pad_x, vh + pad_y);
        let d = 2 * ((0.18 * vh as f64).round() as usize).max(4);
        Self {
            w,
            h,
            vx0: VIEW_MARGIN,
            vy0: VIEW_MARGIN,
            view_w: vw,
            view_h: vh,
            // A short run from the source leaves most of the view to the fan.
            xw: (VIEW_MARGIN + (0.28 * vw as f64).round() as usize).max(SOURCE_X + 16),
            yc: (h as f64 - 1.0) / 2.0,
            d,
            slit: 5,
            xs: w - ABSORB - 1,
        }
    }

    /// Whether node row `y` is inside slit `which` (−1 lower on screen … +1 upper).
    fn in_slit(&self, y: usize, which: f64) -> bool {
        let centre = self.yc + which * self.d as f64 / 2.0;
        (y as f64 - centre).abs() <= (self.slit as f64 - 1.0) / 2.0 + 1e-9
    }
}

/// What occupies a node, for the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Solid {
    Free,
    Wall,
    Barrier,
}

/// Which obstacle an act puts in the packet's way. `OneSlit` and `None` are the
/// acceptance tests' controls (QDS-A1, QDS-A3).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Obstacle {
    /// Both slits open.
    TwoSlits,
    /// Only the upper slit open (QDS-A3).
    OneSlit,
    /// A thin barrier of `V_BARRIER` and this width, all the way across (QDS-A5).
    Barrier(usize),
    /// Nothing (QDS-A1).
    None,
}

/// The real potential and the node's kind at `(x, y)`.
pub(crate) fn potential(g: &Geometry, obstacle: Obstacle, x: usize, y: usize) -> (f64, Solid) {
    let in_wall = (g.xw..g.xw + 2).contains(&x);
    match obstacle {
        Obstacle::TwoSlits if in_wall && !g.in_slit(y, -1.0) && !g.in_slit(y, 1.0) => {
            (V_WALL, Solid::Wall)
        }
        Obstacle::OneSlit if in_wall && !g.in_slit(y, 1.0) => (V_WALL, Solid::Wall),
        Obstacle::Barrier(bw) if (g.xw..g.xw + bw).contains(&x) => (V_BARRIER, Solid::Barrier),
        _ => (0.0, Solid::Free),
    }
}

/// The edge layer's absorption rate η at `(x, y)`: `ETA·s²`, `s` the depth into the
/// layer in `[0, 1]`.
pub(crate) fn absorption(g: &Geometry, x: usize, y: usize) -> f64 {
    let depth = |i: usize, n: usize| {
        let from_edge = i.min(n - 1 - i);
        if from_edge >= ABSORB {
            0.0
        } else {
            (ABSORB - from_edge) as f64 / ABSORB as f64
        }
    };
    let s = depth(x, g.w).max(depth(y, g.h));
    ETA * s * s
}

/// `i·z`.
#[inline(always)]
fn times_i(z: C64) -> C64 {
    C64::new(-z.im, z.re)
}

/// One potential's line matrices, factored (see [`Adi::set_potential`]).
#[derive(Debug, Clone, Default)]
struct Factors {
    /// Explicit diagonal `1 − i(2a + b)`, `b = dt·V/4`; the implicit one is `2 − e`.
    e: Vec<C64>,
    /// Factored rows (row-major) and columns (column-major).
    inv_x: Vec<C64>,
    cp_x: Vec<C64>,
    inv_y: Vec<C64>,
    cp_y: Vec<C64>,
}

/// The alternating-direction Crank–Nicolson propagator (QDS-A1) on a `w × h` grid, with
/// a few potentials factored ahead of time so switching between them costs nothing.
#[derive(Debug, Clone, Default)]
pub(crate) struct Adi {
    w: usize,
    h: usize,
    /// ψ, row-major.
    pub psi: Vec<C64>,
    sets: Vec<Factors>,
    active: usize,
    /// Off-diagonal `−i·a` of the implicit matrices, `a = dt/2`, and line scratch.
    off: Vec<C64>,
    line: Vec<C64>,
    col: Vec<C64>,
    a: f64,
    flip: bool,
}

impl Adi {
    /// A zero ψ with `slots` potentials, each V = 0 until set.
    pub(crate) fn new(w: usize, h: usize, dt: f64, slots: usize) -> Self {
        let (n, m) = (w * h, w.max(h));
        let a = dt / 2.0;
        let mut adi = Self {
            w,
            h,
            psi: vec![C64::ZERO; n],
            sets: (0..slots.max(1))
                .map(|_| Factors {
                    e: vec![C64::ONE; n],
                    inv_x: vec![C64::ZERO; n],
                    cp_x: vec![C64::ZERO; n],
                    inv_y: vec![C64::ZERO; n],
                    cp_y: vec![C64::ZERO; n],
                })
                .collect(),
            active: 0,
            off: vec![C64::new(0.0, -a); m],
            line: vec![C64::ZERO; m],
            col: vec![C64::ZERO; m],
            a,
            flip: false,
        };
        for slot in 0..adi.sets.len() {
            adi.set_potential(slot, |_, _| C64::ZERO);
        }
        adi
    }

    /// Set slot `slot`'s (complex) potential `V(x, y)` and factor every line's matrix.
    /// The pivots of `1 + iA` stay off zero for finite V with `Im V ≤ 0`.
    pub(crate) fn set_potential(&mut self, slot: usize, v: impl Fn(usize, usize) -> C64) {
        let (w, h, a) = (self.w, self.h, self.a);
        let Some(f) = self.sets.get_mut(slot) else {
            return;
        };
        let two_a = C64::new(2.0 * a, 0.0);
        for y in 0..h {
            for x in 0..w {
                let b = v(x, y).scale(a / 2.0);
                f.e[y * w + x] = C64::ONE - times_i(two_a + b);
            }
        }
        let two = C64::new(2.0, 0.0);
        for y in 0..h {
            let r = y * w..(y + 1) * w;
            for (d, &e) in self.line[..w].iter_mut().zip(&f.e[r.clone()]) {
                *d = two - e;
            }
            let (off, line) = (&self.off[..w], &self.line[..w]);
            tridiag_factor_c(off, line, off, &mut f.inv_x[r.clone()], &mut f.cp_x[r]);
        }
        for x in 0..w {
            for y in 0..h {
                self.line[y] = two - f.e[y * w + x];
            }
            let r = x * h..(x + 1) * h;
            let (off, line) = (&self.off[..h], &self.line[..h]);
            tridiag_factor_c(off, line, off, &mut f.inv_y[r.clone()], &mut f.cp_y[r]);
        }
    }

    /// Step with slot `slot`'s potential from now on.
    pub(crate) fn select(&mut self, slot: usize) {
        self.active = slot.min(self.sets.len().saturating_sub(1));
    }

    /// One step: the two axis factors, in alternating order.
    pub(crate) fn step(&mut self) {
        if self.w * self.h == 0 || self.sets.is_empty() {
            return;
        }
        self.flip = !self.flip;
        if self.flip {
            self.sweep_x();
            self.sweep_y();
        } else {
            self.sweep_y();
            self.sweep_x();
        }
    }

    /// `(1 − iA)` of one line from `src` into `dst`, Dirichlet ends.
    #[inline(always)]
    fn explicit(a: f64, e: impl Fn(usize) -> C64, src: &[C64], dst: &mut [C64]) {
        let n = src.len();
        for i in 0..n {
            let left = if i > 0 { src[i - 1] } else { C64::ZERO };
            let right = if i + 1 < n { src[i + 1] } else { C64::ZERO };
            dst[i] = e(i) * src[i] + times_i((left + right).scale(a));
        }
    }

    fn sweep_x(&mut self) {
        let (w, a) = (self.w, self.a);
        let f = &self.sets[self.active];
        for y in 0..self.h {
            let r = y * w..(y + 1) * w;
            let e = &f.e[r.clone()];
            let line = &mut self.line[..w];
            Self::explicit(a, |i| e[i], &self.psi[r.clone()], line);
            tridiag_apply_c(
                &self.off[..w],
                &f.inv_x[r.clone()],
                &f.cp_x[r.clone()],
                line,
            );
            self.psi[r].copy_from_slice(line);
        }
    }

    fn sweep_y(&mut self) {
        let (w, h, a) = (self.w, self.h, self.a);
        let f = &self.sets[self.active];
        for x in 0..w {
            for y in 0..h {
                self.col[y] = self.psi[y * w + x];
            }
            let e = &f.e;
            let line = &mut self.line[..h];
            Self::explicit(a, |y| e[y * w + x], &self.col[..h], line);
            let r = x * h..(x + 1) * h;
            tridiag_apply_c(&self.off[..h], &f.inv_y[r.clone()], &f.cp_y[r], line);
            for (y, &v) in line.iter().enumerate() {
                self.psi[y * w + x] = v;
            }
        }
    }

    /// `Σ|ψ|²`.
    #[cfg(test)]
    pub(crate) fn norm(&self) -> f64 {
        self.psi.iter().map(|z| z.norm_sqr()).sum()
    }
}

/// The steady line source (see the module docs): `ψ(x, y) += amp·g(y)·e^{−iφn}` after
/// step `n`, with `φ` the carrier's eigenphase per step, so the beam it radiates is
/// monochromatic at wave number `k`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Source {
    x: usize,
    profile: Vec<f64>,
    phi: f64,
    n: u64,
}

impl Source {
    pub(crate) fn new(g: &Geometry, k: f64, dt: f64) -> Self {
        let sigma = SIGMA_Y_FRAC * g.view_h as f64;
        let profile = (0..g.h)
            .map(|y| {
                let dy = y as f64 - g.yc;
                (-(dy * dy) / (2.0 * sigma * sigma)).exp()
            })
            .collect();
        Self {
            x: SOURCE_X.min(g.w.saturating_sub(1)),
            profile,
            phi: carrier_phase(k, dt),
            n: 0,
        }
    }

    /// Add this step's source term to `psi` (`w` wide) and advance the phase.
    pub(crate) fn emit(&mut self, psi: &mut [C64], w: usize, amp: f64) {
        let (s, c) = (self.phi * self.n as f64).sin_cos_det();
        let e = C64::new(c, -s).scale(amp);
        for (y, &g) in self.profile.iter().enumerate() {
            if let Some(z) = psi.get_mut(y * w + self.x) {
                *z = *z + e.scale(g);
            }
        }
        self.n += 1;
    }

    /// The analytic beam the source settles to, `u·g(y)·e^{ik|x − x_src|}` at the current
    /// phase, written left of column `x_end` (the rest is left alone).
    pub(crate) fn fill_beam(&self, psi: &mut [C64], w: usize, k: f64, u: C64, x_end: usize) {
        let (s, c) = (self.phi * self.n as f64).sin_cos_det();
        let now = u * C64::new(c, -s);
        for (y, &g) in self.profile.iter().enumerate() {
            for x in 0..x_end.min(w) {
                let (s, c) = (k * x.abs_diff(self.x) as f64).sin_cos_det();
                psi[y * w + x] = now * C64::new(c, s).scale(g);
            }
        }
    }

    /// The probability current the beam carries to the right, `2 sin k·|u|²·Σ g²`.
    pub(crate) fn flux(&self, k: f64, u: f64) -> f64 {
        2.0 * k.sin_det() * u * u * self.profile.iter().map(|g| g * g).sum::<f64>()
    }
}

/// The step's eigenphase for a plane wave `k` along one axis: `2 atan(dt(1 − cos k))`
/// (the Cayley factor of `A = (dt/2)·2(1 − cos k)`; the other axis contributes nothing).
pub(crate) fn carrier_phase(k: f64, dt: f64) -> f64 {
    2.0 * (dt * (1.0 - k.cos_det())).atan()
}

/// The steady incident amplitude per unit source strength (QDS-A6), measured on the
/// lattice for `K_WAVE` and `DT`: the additive source's discrete-time response.
const SOURCE_GAIN: C64 = C64::new(4.17, 0.20);

/// Fill `psi` (row-major `w`-wide) with a unit-norm Gaussian packet at `(x0, y0)` moving
/// right with wave number `k`; nodes where `solid` holds stay zero.
#[cfg(test)]
pub(crate) fn launch(
    psi: &mut [C64],
    w: usize,
    (x0, y0): (f64, f64),
    (sx, sy): (f64, f64),
    k: f64,
    solid: impl Fn(usize) -> bool,
) {
    let mut norm = 0.0;
    for (i, z) in psi.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f64, (i / w) as f64);
        let (dx, dy) = (x - x0, y - y0);
        let env = (-(dx * dx) / (2.0 * sx * sx) - (dy * dy) / (2.0 * sy * sy)).exp();
        *z = if solid(i) || env < 1e-12 {
            C64::ZERO
        } else {
            let (s, c) = (k * dx).sin_cos_det();
            C64::new(env * c, env * s)
        };
        norm += z.norm_sqr();
    }
    if norm > 0.0 {
        let k = 1.0 / norm.sqrt();
        psi.iter_mut().for_each(|z| *z = z.scale(k));
    }
}

/// The probability current through the screen at row `y`, `j = 2 Im(ψ*_x ψ_{x+1})`
/// (lattice units, `H = −∇²`), clamped at 0: the Born rule at a detector counts what
/// arrives, so a wave running back from the absorber does not click.
pub(crate) fn screen_current(psi: &[C64], w: usize, xs: usize, y: usize) -> f64 {
    let (a, b) = (psi[y * w + xs], psi[y * w + xs + 1]);
    (2.0 * (a.re * b.im - a.im * b.re)).max(0.0)
}

/// The wavelength that sets fringe spacings on the lattice, `2π/sin k` (QDS-A2). Near
/// the axis the lattice's dispersion curve `4 − 2cos kₓ − 2cos k_y = E` bends as
/// `δkₓ = −k_y²/(2 sin k)` where a circle bends as `−k_y²/(2k)`, so rays diffract as a
/// wave `k/sin k` times longer than the carrier's `2π/k` (1.21 at 6 nodes).
#[cfg(test)]
pub(crate) fn lattice_wavelength(k: f64) -> f64 {
    TAU / k.sin_det()
}

/// Linear interpolation of `rows(0..n)` at `y` (rows at integers, clamped): the click
/// density along the screen.
pub(crate) fn interpolate(n: usize, rows: &impl Fn(usize) -> f64, y: f64) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let y = y.clamp(0.0, (n - 1) as f64);
    let (i, f) = (y as usize, y - y.floor());
    rows(i) * (1.0 - f) + rows((i + 1).min(n - 1)) * f
}

/// The Born-rule click sampler over `n` screen rows with weights `rows` (QDS-A4):
/// positions in `[0, n)`, allocation-free.
pub(crate) fn screen_sampler(n: usize, rows: impl Fn(usize) -> f64) -> InverseCdf<256> {
    InverseCdf::build(0.0, n as f64, |y| interpolate(n, &rows, y))
}

/// The packet's group velocity along x, `2 sin k` (lattice dispersion `2(1 − cos k)`).
#[cfg(test)]
pub(crate) fn group_velocity(k: f64) -> f64 {
    2.0 * k.sin_det()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Click {
    /// Position on the screen strip as fractions of its width and height.
    fx: f32,
    fy: f32,
    age: f32,
}

#[derive(Debug)]
pub struct QuantumDoubleSlit {
    size: Size,
    geo: Geometry,
    adi: Adi,
    solid: Vec<Solid>,
    timeline: Timeline,
    t: f64,
    pending: f64,
    /// The act whose potential is set.
    act: Option<usize>,
    source: Source,
    /// The beam's rightward current (click-rate reference), and fast-forward time left.
    beam_flux: f64,
    fast: f64,
    clicks: ParticlePool<Click>,
    /// Clicks per screen bin since the screen was last cleared.
    hist: [u32; HIST_BINS],
    debt: f64,
    /// Exposure peaks of `|ψ|²` left and right of the wall.
    peak: [f64; 2],
    /// Per node: coverage and phase level; per dot column / row: the node it shows.
    col_node: Vec<(u16, f32)>,
    row_node: Vec<(u16, f32)>,
    /// Dot columns of the wave view, and where the screen strip starts.
    /// Pixel columns of the wave view; whether the view resolves the carrier.
    view_px: i32,
    carrier: bool,
    strip_x: i32,
    pixels: Pixels,
    dots: Dots,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl QuantumDoubleSlit {
    fn new() -> Self {
        Self {
            dirty: true,
            size: Size::new(1, 1),
            geo: Geometry::for_aspect(1.0),
            adi: Adi::default(),
            solid: Vec::new(),
            timeline: Timeline::new(&[
                (CAPTIONS[0], ACT_S),
                (CAPTIONS[1], ACT_S),
                (CAPTIONS[2], ACT_S),
            ]),
            t: 0.0,
            pending: 0.0,
            act: None,
            source: Source::default(),
            beam_flux: 0.0,
            fast: 0.0,
            clicks: ParticlePool::with_capacity(CLICK_CAP),
            hist: [0; HIST_BINS],
            debt: 0.0,
            peak: [0.0; 2],
            col_node: Vec::new(),
            row_node: Vec::new(),
            view_px: 0,
            carrier: true,
            strip_x: 0,
            pixels: Pixels::default(),
            dots: Dots::default(),
        }
    }

    fn obstacle(act: usize) -> Obstacle {
        if act == 2 {
            Obstacle::Barrier(BARRIER_W)
        } else {
            Obstacle::TwoSlits
        }
    }

    /// Factor the slits (slot 0) and the barrier (slot 1) once per grid.
    fn factor(&mut self) {
        let g = self.geo;
        for (slot, obstacle) in [Obstacle::TwoSlits, Obstacle::Barrier(BARRIER_W)]
            .into_iter()
            .enumerate()
        {
            self.adi.set_potential(slot, |x, y| {
                C64::new(potential(&g, obstacle, x, y).0, -absorption(&g, x, y))
            });
        }
    }

    /// Switch to the act's obstacle: its picture mask and its factored potential.
    fn set_act(&mut self, act: usize) {
        let (g, obstacle) = (self.geo, Self::obstacle(act));
        for (i, s) in self.solid.iter_mut().enumerate() {
            *s = potential(&g, obstacle, i % g.w, i / g.w).1;
        }
        self.adi.select(usize::from(act == 2));
        self.act = Some(act);
    }

    /// Start the beam on a new grid: the analytic incident beam up to the wall, nothing
    /// beyond it yet, and a fast first second.
    fn start(&mut self) {
        let g = self.geo;
        self.source = Source::new(&g, K_WAVE, DT);
        self.adi.psi.fill(C64::ZERO);
        let u = SOURCE_GAIN.scale(SOURCE_S);
        self.source
            .fill_beam(&mut self.adi.psi, g.w, K_WAVE, u, g.xw);
        self.beam_flux = self.source.flux(K_WAVE, u.norm_sqr().sqrt()).max(1e-30);
        self.fast = FAST_S;
        self.peak = self.peaks();
    }

    /// The largest `|ψ|²` left of the wall's far side and right of it.
    fn peaks(&self) -> [f64; 2] {
        let (w, split) = (self.geo.w, self.geo.xw + 2);
        let mut peak = [0.0f64; 2];
        for row in self.adi.psi.chunks_exact(w.max(1)) {
            for (x, z) in row.iter().enumerate() {
                let side = usize::from(x >= split);
                peak[side] = peak[side].max(z.norm_sqr());
            }
        }
        peak
    }

    /// The current reaching the screen as a share of the beam's.
    fn screen_share(&self) -> f64 {
        let g = &self.geo;
        let s: f64 = (g.vy0..g.vy0 + g.view_h)
            .map(|y| screen_current(&self.adi.psi, g.w, g.xs, y))
            .sum();
        s / self.beam_flux
    }

    /// Draw up to [`CLICKS_PER_FRAME`] clicks from what arrives at the screen.
    fn detect(&mut self, rng: &mut impl Rng) {
        if self.debt < 1.0 {
            return;
        }
        let g = self.geo;
        let psi = &self.adi.psi;
        let cdf = screen_sampler(g.view_h, |r| screen_current(psi, g.w, g.xs, g.vy0 + r));
        let mut n = 0;
        while self.debt >= 1.0 && n < CLICKS_PER_FRAME {
            let fy = (cdf.sample(rng.random()) / g.view_h as f64) as f32;
            let fx = rng.random::<f32>();
            if self.clicks.spawn(Click { fx, fy, age: 0.0 }) {
                let bin = ((fy * HIST_BINS as f32) as usize).min(HIST_BINS - 1);
                self.hist[bin] += 1;
            }
            self.debt -= 1.0;
            n += 1;
        }
    }

    /// Map the display onto the grid and size the rasters (resize only).
    fn layout(&mut self, size: Size) {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let strip_cells = (w / 6).clamp(1, 20).min(w);
        let (pw, ph) = (w - strip_cells, 2 * h);
        self.view_px = pw;
        self.strip_x = 2 * pw;
        let g = self.geo;
        self.carrier = f64::from(pw) >= MIN_PX_PER_NODE * g.view_w as f64;
        // Pixel centres in node coordinates, as (node, fraction toward the next).
        let at = |p: i32, n: i32, v0: usize, vn: usize| {
            let u = v0 as f64 + (f64::from(p) + 0.5) * vn as f64 / f64::from(n.max(1)) - 0.5;
            let u = u.max(0.0);
            (u.floor() as u16, (u - u.floor()) as f32)
        };
        self.col_node.clear();
        self.col_node
            .extend((0..pw).map(|x| at(x, pw, g.vx0, g.view_w)));
        self.row_node.clear();
        self.row_node
            .extend((0..ph).map(|y| at(y, ph, g.vy0, g.view_h)));
        self.pixels.resize(size);
        self.dots.resize(size);
    }

    fn rasterise(&mut self) {
        let act = self.act.unwrap_or(0);
        let vis = f64::from(WAVE_VIS[act.min(2)]);
        let (gw, split) = (self.geo.w, self.geo.xw + 2);
        // The tunnelling act shares one exposure, so the transmitted share reads true.
        let peak = if act == 2 {
            [self.peak[0].max(self.peak[1]); 2]
        } else {
            self.peak
        };
        // The tunnelling act shows both sides at the same exposure: what gets through
        // is visibly a fraction of what arrives.
        let source = if act == 2 { 1.0 } else { SOURCE_EXPOSURE };
        let scale = [
            source / peak[0].max(1e-30).sqrt(),
            1.0 / peak[1].max(1e-30).sqrt(),
        ];
        let gh = self.geo.h;
        let (cols, rows, psi, solid) = (&self.col_node, &self.row_node, &self.adi.psi, &self.solid);
        let (view, carrier) = (self.view_px, self.carrier);
        let tunnel = act == 2;
        let cutoff = if tunnel { 0.5 * CUTOFF } else { CUTOFF };
        self.pixels.shade(|x, y| {
            if x == view {
                return if y % 2 == 0 { SCREEN_INK } else { Ink::BLANK };
            }
            let (Some(&(gx, fx)), Some(&(gy, fy))) = (cols.get(x as usize), rows.get(y as usize))
            else {
                return Ink::BLANK;
            };
            let (gx, gy) = (usize::from(gx).min(gw - 1), usize::from(gy).min(gh - 1));
            let (gx1, gy1) = ((gx + 1).min(gw - 1), (gy + 1).min(gh - 1));
            let near = (if fx < 0.5 { gx } else { gx1 }) + gw * if fy < 0.5 { gy } else { gy1 };
            match solid[near] {
                Solid::Wall => return WALL_INK,
                Solid::Barrier => return BARRIER_INK,
                Solid::Free => {}
            }
            // Bilinear in ψ, so the carrier's crests stay smooth between nodes.
            let (fx, fy) = (f64::from(fx), f64::from(fy));
            let lerp = |a: C64, b: C64, f: f64| a.scale(1.0 - f) + b.scale(f);
            let top = lerp(psi[gy * gw + gx], psi[gy * gw + gx1], fx);
            let bottom = lerp(psi[gy1 * gw + gx], psi[gy1 * gw + gx1], fx);
            let z = lerp(top, bottom, fy);
            let amp = z.norm_sqr().sqrt();
            let v = scale[usize::from(gx >= split)] * amp;
            // Crests a little brighter than troughs where the carrier is resolved.
            let shimmer = if carrier && amp > 0.0 {
                1.0 - SHIMMER * (0.5 - 0.5 * z.re / amp)
            } else {
                1.0
            };
            // A hard cut before the act's dimming keeps the dimmed wave's outline as crisp
            // as the full one's.
            // √ tone for the carrier (crests stay bold as |ψ| falls off), linear for |ψ|
            // (so the nodal rays of a coarse view keep their dark gradient); the barrier
            // act uses `v^¾` either way, lower cut, so the weak transmitted wave shows at
            // its true, smaller share.
            let a = v.min(1.0);
            let t = if tunnel {
                a.sqrt() * a.sqrt().sqrt()
            } else {
                a.powf(0.8)
            };
            // A soft knee instead of a hard cut: faint density fades out, no speckle.
            let knee = ((t - 0.5 * cutoff) / cutoff).clamp(0.0, 1.0);
            let level = vis * t * knee * knee * shimmer;
            if level * 255.0 < ink::MIN_VISIBLE_LEVEL {
                Ink::BLANK
            } else {
                ink::ramp(WAVE, level)
            }
        });
        self.glow_histogram();
        self.dots.clear();
        let (strip, dh) = (self.strip_x, 4 * i32::from(self.size.height()));
        let width = (2 * i32::from(self.size.width()) - strip - 3).max(1);
        for c in self.clicks.iter() {
            let x = strip + 3 + (c.fx * width as f32) as i32;
            let y = (c.fy * dh as f32) as i32;
            let fresh = 1.0 - (c.age / CLICK_FADE_S).clamp(0.0, 1.0);
            let level = CLICK_REST + (255.0 - CLICK_REST) * f64::from(fresh);
            self.dots
                .set(x, y, ink::ramp(Ramp::Starlight, level / 255.0));
        }
    }
}

impl QuantumDoubleSlit {
    /// The click histogram as a soft Starlight glow across the strip, one level per pixel
    /// row: binomially smoothed, toned against the fullest row and faded in with the
    /// click count, so the fringes read as bands behind the dots, not as early noise.
    fn glow_histogram(&mut self) {
        let (x0, x1) = (self.view_px + 1, i32::from(self.size.width()));
        let ph = 2 * i32::from(self.size.height());
        let total: u32 = self.hist.iter().sum();
        if total == 0 || x0 >= x1 {
            return;
        }
        let norm: f64 = GLOW_TAPS.iter().sum();
        let mut smooth = [0.0f64; HIST_BINS];
        for (b, out) in smooth.iter_mut().enumerate() {
            *out = GLOW_TAPS
                .iter()
                .enumerate()
                .map(|(k, w)| {
                    let i = (b + k).saturating_sub(4).min(HIST_BINS - 1);
                    w * f64::from(self.hist[i])
                })
                .sum::<f64>()
                / norm;
        }
        let top = smooth.iter().fold(0.0f64, |m, &v| m.max(v));
        let strength = (f64::from(total) / GLOW_FULL_CLICKS).min(1.0);
        for y in 0..ph {
            // The bins this pixel row covers, averaged.
            let lo = (y as usize * HIST_BINS) / ph as usize;
            let hi = (((y + 1) as usize * HIST_BINS) / ph as usize).clamp(lo + 1, HIST_BINS);
            let v = smooth[lo..hi].iter().sum::<f64>() / (hi - lo) as f64 / top;
            let level = GLOW_LEVELS.0 + (GLOW_LEVELS.1 - GLOW_LEVELS.0) * v * strength;
            if v <= 0.02 || level < ink::MIN_VISIBLE_LEVEL {
                continue;
            }
            // Even levels only: odd ones mark the wall, barrier and screen (see `ascii_cell`).
            let ink = Ink::new(Role::Ramp(Ramp::Starlight), ink::level(level) & !1);
            for x in x0..x1 {
                self.pixels.set(x, y, ink);
            }
        }
    }
}

/// ASCII fallback of a pixel pair: the stronger ink; `Div` by its distance from zero on
/// the ASCII ramp, walls and the screen as `|`, the click glow as `.` and `:`.
fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = sciart_core::raster::stronger(top, bottom);
    match ink.role {
        Role::Ramp(WAVE) => {
            let i = usize::from(ink.level);
            let ramp = sciart_core::raster::ASCII_RAMP;
            Some((ramp[(i * ramp.len() / 256).min(ramp.len() - 1)], ink))
        }
        // Walls, barrier and screen (odd levels) are rules; the click glow (even) is shade.
        Role::Ramp(Ramp::Starlight) if ink.level % 2 == 1 => Some(('|', ink)),
        Role::Ramp(Ramp::Starlight) => Some((if ink.level < 60 { '.' } else { ':' }, ink)),
        _ => None,
    }
}

impl Generator for QuantumDoubleSlit {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.t).index.min(2)])
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let strip = (size.width() / 6).clamp(1, 20).min(size.width());
        let aspect = 2.0 * (w - f64::from(strip)).max(1.0) / (4.0 * h);
        let geo = Geometry::for_aspect(aspect);
        if geo != self.geo || self.adi.psi.is_empty() {
            self.geo = geo;
            self.adi = Adi::new(geo.w, geo.h, DT, 2);
            self.factor();
            self.solid = vec![Solid::Free; geo.w * geo.h];
            self.set_act(self.timeline.at(self.t).index.min(2));
            self.start();
        }
        self.layout(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.adi.psi.is_empty() {
            return;
        }
        self.t += dt;
        let beat = self.timeline.at(self.t);
        let act = beat.index.min(2);
        if self.act != Some(act) {
            self.set_act(act);
            if act != 1 {
                self.clicks.clear();
                self.hist = [0; HIST_BINS];
                self.debt = 0.0;
            }
        }
        let speed = if self.fast > 0.0 { FAST } else { 1.0 };
        self.fast -= dt;
        self.pending += dt / SIM_DT * STEPS_PER_FRAME * speed;
        // Clicks per step: the act's rate per second over the steps a second holds.
        let per_step = CLICK_RATE[act] / (STEPS_PER_FRAME / SIM_DT);
        let (w, mut steps) = (self.geo.w, 0);
        while self.pending >= 1.0 && steps < MAX_STEPS {
            self.adi.step();
            self.source.emit(&mut self.adi.psi, w, SOURCE_S);
            self.debt += self.screen_share() * per_step;
            self.pending -= 1.0;
            steps += 1;
        }
        self.pending = self.pending.min(1.0);
        self.debt = self.debt.min(4.0 * CLICKS_PER_FRAME as f64);
        self.detect(&mut ctx.rng);
        for c in self.clicks.iter_mut() {
            c.age += dt as f32;
        }
        let now = self.peaks();
        let decay = (-dt * std::f64::consts::LN_2 / PEAK_HALF_LIFE_S).exp();
        self.peak = [0, 1].map(|i| now[i].max(self.peak[i] * decay));
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
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> Geometry {
        Geometry::for_aspect(1.6)
    }

    fn propagator(g: &Geometry, obstacle: Obstacle, absorb: bool) -> Adi {
        let mut adi = Adi::new(g.w, g.h, DT, 1);
        adi.set_potential(0, |x, y| {
            let eta = if absorb { absorption(g, x, y) } else { 0.0 };
            C64::new(potential(g, obstacle, x, y).0, -eta)
        });
        adi
    }

    /// A unit packet left of the wall (QDS-A1).
    fn with_packet(g: &Geometry, obstacle: Obstacle, absorb: bool) -> Adi {
        let mut adi = propagator(g, obstacle, absorb);
        let solid = |i: usize| potential(g, obstacle, i % g.w, i / g.w).1 != Solid::Free;
        let sigma = (SIGMA_X, SIGMA_Y_FRAC * g.view_h as f64);
        let x0 = (g.xw as f64 - 3.0 * SIGMA_X).max(ABSORB as f64 + 2.0 * SIGMA_X);
        launch(&mut adi.psi, g.w, (x0, g.yc), sigma, K_WAVE, solid);
        adi
    }

    /// Steps for the beam to reach the screen and settle.
    fn settle(g: &Geometry) -> usize {
        let travel = (g.xs - SOURCE_X) as f64 / group_velocity(K_WAVE) / DT;
        (1.6 * travel) as usize
    }

    /// The steady source driven through `obstacle` (the scene's beam, from rest), and the
    /// current arriving at the screen per row, averaged over three carrier periods.
    fn screen_profile(obstacle: Obstacle) -> (Geometry, Vec<f64>) {
        let g = geo();
        let mut adi = propagator(&g, obstacle, true);
        let mut src = Source::new(&g, K_WAVE, DT);
        for _ in 0..settle(&g) {
            adi.step();
            src.emit(&mut adi.psi, g.w, SOURCE_S);
        }
        let mut acc = vec![0.0; g.h];
        let period = (TAU / carrier_phase(K_WAVE, DT)).round() as usize;
        for _ in 0..3 * period {
            adi.step();
            src.emit(&mut adi.psi, g.w, SOURCE_S);
            for (y, a) in acc.iter_mut().enumerate() {
                *a += screen_current(&adi.psi, g.w, g.xs, y);
            }
        }
        (g, acc)
    }

    #[test]
    fn qds_a1_adi_step_preserves_the_norm_without_absorbers() {
        let g = geo();
        for obstacle in [Obstacle::None, Obstacle::TwoSlits, Obstacle::Barrier(2)] {
            let mut adi = with_packet(&g, obstacle, false);
            let n0 = adi.norm();
            assert!((n0 - 1.0).abs() < 1e-12);
            for _ in 0..200 {
                adi.step();
            }
            let drift = (adi.norm() - n0).abs() / n0;
            assert!(drift < 1e-10, "{obstacle:?}: relative drift {drift:e}");
        }
        // The absorbing edge only ever removes probability.
        let mut adi = with_packet(&g, Obstacle::None, true);
        let mut last = adi.norm();
        for _ in 0..600 {
            adi.step();
            let n = adi.norm();
            assert!(n <= last + 1e-12);
            last = n;
        }
        assert!(
            last < 0.05,
            "the packet leaves through the absorber: {last}"
        );
    }

    #[test]
    fn qds_a2_fringe_spacing_matches_lambda_l_over_d() {
        let (g, p) = screen_profile(Obstacle::TwoSlits);
        // One fringe period: between the dark fringes either side of the centre (the
        // steady central lobe is flat-topped in the near field, its zeros are sharp).
        let top = p.iter().fold(0.0f64, |m, &v| m.max(v));
        let dark = |y: usize| p[y] < p[y - 1] && p[y] <= p[y + 1] && p[y] < 0.2 * top;
        let c = g.yc.round() as usize;
        let right = (c..p.len() - 1)
            .find(|&y| dark(y))
            .expect("a dark fringe below");
        let left = (1..=c)
            .rev()
            .find(|&y| dark(y))
            .expect("a dark fringe above");
        let spacing = (right - left) as f64;
        // Paraxial diffraction on the lattice: the discrete Laplacian bends rays as a
        // wave of length 2π/sin k (k/sin k = 1.21 longer than 2π/k at λ = 6 nodes).
        let lambda = lattice_wavelength(K_WAVE);
        let l = g.xs as f64 - (g.xw as f64 + 0.5);
        let expect = lambda * l / g.d as f64;
        assert!(
            (spacing / expect - 1.0).abs() < 0.10,
            "spacing {spacing} vs λL/d {expect} (dark rows {left}, {right})"
        );
    }

    /// Fringe visibility `(max − min)/(max + min)`.
    /// Over one fringe period (`λL/d`, QDS-A2) centred on row `centre`: two slits give a
    /// bright and a dark fringe there, one slit only its smooth diffraction envelope.
    fn visibility(g: &Geometry, p: &[f64], centre: f64) -> f64 {
        let l = g.xs as f64 - (g.xw as f64 + 0.5);
        let period = lattice_wavelength(K_WAVE) * l / g.d as f64;
        let span = (0.5 * period).round() as usize;
        let c = centre.round() as usize;
        let window = &p[c - span..=c + span];
        let (lo, hi) = window
            .iter()
            .fold((f64::MAX, 0.0f64), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        (hi - lo) / (hi + lo)
    }

    #[test]
    fn qds_a3_one_slit_closed_the_fringes_vanish() {
        let (g, two) = screen_profile(Obstacle::TwoSlits);
        let (_, one) = screen_profile(Obstacle::OneSlit);
        // The open slit's axis (Obstacle::OneSlit keeps the `+1` slit).
        let axis = g.yc + g.d as f64 / 2.0;
        let (v2, v1) = (visibility(&g, &two, g.yc), visibility(&g, &one, axis));
        assert!(v2 > 0.6, "two slits: visibility {v2}");
        assert!(v1 < 0.2, "one slit: visibility {v1}");
    }

    #[test]
    fn qds_a4_clicks_follow_the_born_rule_at_the_screen() {
        // Build the scene's click sampler over a fringe profile and bin 4000 clicks.
        let (g, p) = screen_profile(Obstacle::TwoSlits);
        let prof = &p[g.vy0..g.vy0 + g.view_h];
        let rows = |r: usize| prof[r];
        let cdf = screen_sampler(prof.len(), rows);
        let mut rng = Ctx::new(42, Size::new(1, 1)).rng;
        const BINS: usize = 22;
        let mut counts = [0usize; BINS];
        let n = 4000u32;
        for _ in 0..n {
            let y = cdf.sample(rng.random());
            counts[((y / prof.len() as f64) * BINS as f64).min(BINS as f64 - 1.0) as usize] += 1;
        }
        // Expected counts: the sampled density integrated over each bin.
        let mass = |a: f64, b: f64| {
            let k = 400u32;
            (0..k)
                .map(|j| {
                    interpolate(
                        prof.len(),
                        &rows,
                        a + (b - a) * (f64::from(j) + 0.5) / f64::from(k),
                    )
                })
                .sum::<f64>()
                * (b - a)
                / f64::from(k)
        };
        let len = prof.len() as f64;
        let total = mass(0.0, len);
        let mut chi2 = 0.0;
        let mut dof = 0u32;
        for (b, &c) in counts.iter().enumerate() {
            let (lo, hi) = (
                b as f64 * len / BINS as f64,
                (b + 1) as f64 * len / BINS as f64,
            );
            let expect = mass(lo, hi) / total * f64::from(n);
            if expect >= 5.0 {
                chi2 += (c as f64 - expect).powi(2) / expect;
                dof += 1;
            }
        }
        assert!(dof >= 10, "too few populated bins: {dof}");
        // χ² 99.9th percentile for ~20 dof is ≈ 45; allow the bin-edge interpolation error.
        assert!(chi2 < 2.5 * f64::from(dof), "χ² {chi2} over {dof} bins");
    }

    /// The share of the steady beam's current that reaches the screen through a barrier.
    fn transmitted(width: usize) -> f64 {
        let (g, p) = screen_profile(Obstacle::Barrier(width));
        let period = (TAU / carrier_phase(K_WAVE, DT)).round();
        let src = Source::new(&g, K_WAVE, DT);
        p.iter().sum::<f64>()
            / (3.0 * period)
            / src.flux(K_WAVE, SOURCE_GAIN.norm_sqr().sqrt() * SOURCE_S)
    }

    #[test]
    fn qds_a5_tunnelling_falls_with_barrier_width() {
        let t: Vec<f64> = (1..=3).map(transmitted).collect();
        assert!(t[0] > t[1] && t[1] > t[2], "transmission {t:?}");
        assert!(t[2] > 1e-4 && t[0] < 0.9, "transmission {t:?}");
    }

    #[test]
    fn qds_captions_follow_the_acts_and_clicks_accumulate() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(42, size);
        let mut g = QuantumDoubleSlit::new();
        g.resize(size, &mut ctx);
        g.prepare();
        let mut seen = Vec::new();
        let mut before_tunnel = 0;
        for f in 0..(3.0 * ACT_S * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let c = g.caption();
            if seen.last() != Some(&c) {
                seen.push(c);
            }
            if f + 1 == (2.0 * ACT_S * 60.0) as usize - 1 {
                before_tunnel = g.clicks.len();
            }
        }
        assert_eq!(seen, CAPTIONS.map(Some).to_vec());
        assert!(
            before_tunnel > 300,
            "clicks after two acts: {before_tunnel}"
        );
        assert!(
            g.clicks.len() < before_tunnel,
            "the tunnelling act starts a fresh screen"
        );
    }

    #[test]
    fn qds_a6_the_source_settles_to_a_steady_beam_of_the_stated_amplitude() {
        let g = geo();
        let mut adi = propagator(&g, Obstacle::None, true);
        let mut src = Source::new(&g, K_WAVE, DT);
        for _ in 0..600 {
            adi.step();
            src.emit(&mut adi.psi, g.w, SOURCE_S);
        }
        // Just downstream of the source, on the axis, the field is the analytic beam.
        let mut beam = vec![C64::ZERO; g.w * g.h];
        let u = SOURCE_GAIN.scale(SOURCE_S);
        src.fill_beam(&mut beam, g.w, K_WAVE, u, g.w);
        let y = g.yc.round() as usize;
        for dx in [3, 6, 10] {
            let i = y * g.w + SOURCE_X + dx;
            let err = (adi.psi[i] - beam[i]).norm_sqr().sqrt() / beam[i].norm_sqr().sqrt();
            assert!(err < 0.1, "dx {dx}: {:?} vs {:?}", adi.psi[i], beam[i]);
        }
        // Steady: the field one carrier period later is the same.
        let period = (TAU / carrier_phase(K_WAVE, DT)).round() as usize;
        let i = y * g.w + SOURCE_X + 6;
        let before = adi.psi[i];
        for _ in 0..period {
            adi.step();
            src.emit(&mut adi.psi, g.w, SOURCE_S);
        }
        let drift = (adi.psi[i] - before).norm_sqr().sqrt() / before.norm_sqr().sqrt();
        assert!(drift < 0.1, "not steady: {drift}");
    }

    #[test]
    fn qds_a6_the_fan_fills_the_right_side_within_seconds_of_a_start() {
        for (w, h) in [(120u16, 40u16), (80, 24), (200, 50)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(42, size);
            let mut g = QuantumDoubleSlit::new();
            g.resize(size, &mut ctx);
            g.prepare();
            for _ in 0..90 {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
            }
            // Rows of the right quarter of the view (next to the screen) that show wave.
            let (view, ph) = (g.view_px, 2 * i32::from(h));
            let lit = (0..ph)
                .filter(|&y| {
                    (view * 3 / 4..view).any(|x| {
                        g.pixels
                            .get(x, y)
                            .is_some_and(|p| p.role == Role::Ramp(WAVE))
                    })
                })
                .count();
            assert!(
                lit as f64 >= 0.6 * f64::from(ph),
                "{w}x{h}: fan lights {lit} of {ph} rows at t = 1.5 s"
            );
        }
    }

    #[test]
    fn qds_a7_the_fan_holds_its_brightness_through_the_acts() {
        // The one-at-a-time act keeps the wave within 10 % of full exposure, so the fan
        // never reads as fading out; the histogram glow carries the bands.
        assert!(
            WAVE_VIS.iter().all(|&v| (0.9..=1.0).contains(&v)),
            "{WAVE_VIS:?}"
        );
    }
}
