//! Jupiter's whole disc on black space (SPEC-002 `jupiter_storms`, ADR-0017).
//!
//! Two dyes ride a prescribed flow (`vortex_advect`, semi-Lagrangian on a capped
//! lattice): the zone/belt dye, pale in the anticyclonic zones and blue-grey in the
//! cyclonic belts, and the red chromophore of the Great Red Spot. The flow is alternating
//! zonal jets plus Gaussian vortices given as stream functions: the Great Red Spot and
//! white ovals, anticyclones sitting in the zero of the jets whose shear matches their
//! spin (so they roll like ball bearings), and short-lived small eddies that wind the
//! dye into filaments. A weak relaxation back to the banded pattern keeps the belts from
//! mixing out. The flow lives on a longitude × latitude strip, mapped onto a limb-darkened
//! disc whose view follows the Great Red Spot.
//!
//! Units: lengths in lattice heights (`H`), so the flow keeps its shape when the lattice
//! is regridded (ADR-0012); speeds in `H` per simulated second.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::grid;
use sciart_core::prelude::*;
use sciart_core::{Grid, ease};

use crate::shared::vortex_advect::{self, Flow, XEdge, value_noise_wrap_x};

pub static META: Meta = Meta {
    name: "jupiter_storms",
    category: Category::SunPlanets,
    description: "Jupiter's belts and zones sheared by jets, the Great Red Spot and white ovals",
    title: "Jupiter storms · belts shear",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(JupiterStorms::new())
}

/// Lattice cap (JUP-A5, ADR-0012) and aspect bucket.
pub(crate) const CAP: usize = 160 * 80;
const BUCKET: f64 = 1.25;

/// Jet wavenumber (three belt/zone pairs over the frame height) and phase: anticyclonic
/// shear lanes (zones) at `k·y + φ = 3π/2 (mod 2π)`, cyclonic ones (belts) at `π/2`.
const K: f64 = 6.0 * PI;
const PHI: f64 = 0.06 * PI;
/// Peak jet speed, `H` per second (time-lapsed).
const U0: f64 = 0.11;
/// Relaxation rate of the zone dye towards the banded pattern, and decay of the red
/// chromophore outside the Great Red Spot, per second.
const RELAX: f32 = 0.035;
const RED_DECAY: f32 = 0.3;
/// Great Red Spot: half-axes (1.4:1, its shape today; it was ~2:1 in the Voyager era) and
/// turnover period.
pub(crate) const GRS_A: f64 = 0.14;
pub(crate) const GRS_B: f64 = 0.1;
const GRS_PERIOD: f64 = 6.0;
/// Small eddies alive at once and their life, seconds.
const EDDIES: usize = 30;
const EDDY_LIFE: (f64, f64) = (5.0, 10.0);
/// Steps run in the first resize, so the first frame already shows curls.
const SPIN_UP: usize = 100;
/// Lattice cap for the spin-up (GEN-R19).
const SPIN_UP_CAP: usize = CAP / 10;
/// Hot-spot sources: on the fastest eastward jet at the edge of the belt below the top
/// zone, where the jet draws their blue-grey clearings out into festoons. Longitudes as
/// fractions of the period (drifting slowly, `H`/s), half-axes in `H`, and the decay
/// rate of the clearing dye per second.
const HOT_Y: f64 = (TAU - PHI) / K;
const HOT_X: [f64; 3] = [0.12, 0.47, 0.78];
const HOT_AB: (f64, f64) = (0.036, 0.024);
const HOT_DRIFT: f64 = 0.015;
const HOT_DECAY: f32 = 0.8;
/// Half-height (`H`) of the lane the clearing dye can reach (sources ± eddy spread).
const HOT_LANE: f64 = 0.12;
/// The disc (ADR-0017): radius as a share of the frame's smaller side, polar flattening,
/// the view's sway about the spot (rad, rad/s), the clouds' brightness scale (before
/// the five steps) and the spot's.
const DISC: f64 = 0.47;
const OBLATE: f64 = 0.935;
const SWAY: (f64, f64) = (0.5, 0.05);
/// Io's shadow crosses the disc in `1 / SHADOW_SPEED` seconds.
const SHADOW_SPEED: f64 = 1.0 / 20.0;
/// Latitude (and longitude) one lattice height spans (rad).
const LAT_SPAN: f64 = 65.0 * PI / 180.0;
/// M16 thread look (GEN-R25): at most this many contour nodes across the disc (spacing
/// ≥ 2 dots).
const MAX_ACROSS: f64 = 150.0;
/// Band threads: dye level, role, ink level on the disc and near the limb.
pub(crate) const BAND_LEVELS: [(f32, Role, [u8; 2]); 3] = [
    (0.2, Role::Hot, [200, 150]),
    (0.45, Role::Hot, [235, 175]),
    (0.75, Role::Sun, [235, 175]),
];
/// The Great Red Spot's chromophore threads, and band threads inside its ellipse.
const RED_LEVELS: [(f32, u8); 3] = [(0.2, 190), (0.45, 225), (0.7, 255)];
const SPOT_INK: Ink = Ink::new(Role::Hot, 210);
/// Band threads within `OVAL_REACH` of a white oval's half-axes.
const OVAL_INK: Ink = Ink::new(Role::Star, 230);
const OVAL_REACH: f64 = 1.5;
/// The clearing dye's festoon thread.
const FESTOON: (f32, Ink) = (0.3, Ink::new(Role::Hot, 130));
/// The limb ellipse, and the `μ` below which threads are one step dimmer.
const LIMB_INK: Ink = Ink::new(Role::Sun, 170);
const LIMB_PTS: usize = 240;
const LIMB_MU: f32 = 0.35;
/// Largest vortex box half-width in lattice cells (stack tables in `fill_flow`).
const MAX_BOX: i64 = 96;

/// Zone lanes (anticyclonic shear) nearest `y`: `y` of lane `m`.
fn zone_lane(m: f64) -> f64 {
    (1.5 * PI + TAU * m - PHI) / K
}

/// Zonal jet speed at height `y` (in `H`, `y` down), `H`/s: eastward positive. The
/// envelope is positive, so the jets alternate exactly at the zeros of the cosine.
pub(crate) fn jet(y: f64) -> f64 {
    let envelope = 0.8 + 0.2 * (TAU * 1.3 * y + 0.4).cos_det();
    U0 * envelope * (K * y + PHI).cos_det()
}

/// The zone/belt dye the belts relax to: 1 in the anticyclonic zones, 0 in the belts,
/// sharpened, with a little mottling; `x` in noise cells, `period` of them per turn.
fn band(x: f64, y: f64, period: i32) -> f64 {
    let s = -(K * y + PHI).sin_det();
    let sharp = 0.5 + 0.5 * s.signum() * s.abs().powf(0.6);
    let mottle = value_noise_wrap_x(0x0B5E, x, y * 30.0, period) - 0.5;
    (sharp + 0.5 * mottle).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    RedSpot,
    Oval,
    Eddy,
}

/// A Gaussian vortex `ψ = s·exp(−(dx²/a² + dy²/b²))`; `s < 0` turns counter-clockwise on
/// screen (anticyclonic in the south), `s > 0` clockwise.
#[derive(Debug, Clone, Copy)]
struct Vortex {
    x: f64,
    y: f64,
    a: f64,
    b: f64,
    s: f64,
    /// Seconds lived and lifetime (∞ for the long-lived storms).
    age: f64,
    life: f64,
    kind: Kind,
}

impl Vortex {
    /// Strength now: eddies fade in and out over their life.
    fn strength(&self) -> f64 {
        if self.life.is_finite() {
            self.s * (PI * (self.age / self.life).clamp(0.0, 1.0)).sin_det()
        } else {
            self.s
        }
    }

    /// Wrapped `x` offset from the centre on a lattice `period` wide.
    fn dx(&self, x: f64, period: f64) -> f64 {
        let dx = x - self.x;
        dx - period * (dx / period).round()
    }

    #[cfg(test)]
    /// Stream function at `(x, y)` with `x` wrapped onto a lattice `period` wide, as
    /// `s·e^{−dx²/a²}·e^{−dy²/b²}`: separable, so the lattice fill can reuse one factor
    /// per column and one per row (bit-identical to this).
    fn psi(&self, x: f64, y: f64, period: f64) -> f64 {
        let dx = self.dx(x, period);
        let dy = y - self.y;
        self.strength()
            * (-(dx * dx) / (self.a * self.a)).exp()
            * (-(dy * dy) / (self.b * self.b)).exp()
    }
}

/// `ψ` amplitude giving a turnover `period` along the ellipse at 0.7 of its half-axes:
/// the speed there is `|s|·2·0.7/b·e^{−0.49}`.
fn strength_for_period(a: f64, b: f64, period: f64) -> f64 {
    let (ra, rb) = (0.7 * a, 0.7 * b);
    let h = ((ra - rb) / (ra + rb)).powi(2);
    let perimeter = PI * (ra + rb) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
    let speed = perimeter / period;
    speed * b / (2.0 * 0.7 * (-0.49f64).exp())
}

#[derive(Debug)]
pub struct JupiterStorms {
    lattice: Size,
    /// Zone/belt dye and red chromophore, with their scratch grids.
    dye: Grid<f32>,
    red: Grid<f32>,
    /// Blue-grey clearing dye (hot spots drawn out into festoons).
    hot: Grid<f32>,
    scratch_dye: Grid<f32>,
    scratch_red: Grid<f32>,
    scratch_hot: Grid<f32>,
    /// The banded pattern the dye relaxes to (static per lattice).
    target: Grid<f32>,
    flow: Flow,
    /// Jet speed per lattice row, cells per second.
    jets: Vec<f32>,
    vortices: Vec<Vortex>,
    /// M16: the threads, over a node grid laid on the disc (geometry per resize, dye
    /// samples per frame).
    dots: Dots,
    geo: Vec<Geo>,
    nodes: (usize, usize),
    /// Node (0, 0) in dots, and the node spacing in dots.
    origin: (f64, f64),
    spacing: f64,
    band_v: Vec<f32>,
    red_v: Vec<f32>,
    hot_v: Vec<f32>,
    warmed: bool,
    /// Simulated seconds shown (drives Io's shadow).
    t: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl JupiterStorms {
    fn new() -> Self {
        Self {
            lattice: Size::new(1, 1),
            dye: Grid::default(),
            red: Grid::default(),
            hot: Grid::default(),
            scratch_dye: Grid::default(),
            scratch_red: Grid::default(),
            scratch_hot: Grid::default(),
            target: Grid::default(),
            flow: Flow::default(),
            jets: Vec::new(),
            vortices: Vec::with_capacity(8 + EDDIES),
            dots: Dots::default(),
            geo: Vec::new(),
            nodes: (0, 0),
            origin: (0.0, 0.0),
            spacing: 2.0,
            band_v: Vec::new(),
            red_v: Vec::new(),
            hot_v: Vec::new(),
            warmed: false,
            t: 0.0,
            dirty: false,
        }
    }

    /// Lattice width in `H` units (the longitude period).
    fn period(&self) -> f64 {
        f64::from(self.lattice.width()) / f64::from(self.lattice.height())
    }

    fn seed_storms(&mut self, rng: &mut impl Rng) {
        self.vortices.clear();
        let period = self.period();
        let grs = -strength_for_period(GRS_A, GRS_B, GRS_PERIOD);
        self.vortices.push(Vortex {
            x: 0.55 * period,
            y: zone_lane(1.0),
            a: GRS_A,
            b: GRS_B,
            s: grs,
            age: 0.0,
            life: f64::INFINITY,
            kind: Kind::RedSpot,
        });
        // White ovals: anticyclones in the other zone lanes (a "string of pearls").
        for (m, x) in [(0.0, 0.3), (2.0, 0.7)] {
            let (a, b) = (0.055, 0.04);
            self.vortices.push(Vortex {
                x: x * period,
                y: zone_lane(m),
                a,
                b,
                s: -strength_for_period(a, b, 4.0),
                age: 0.0,
                life: f64::INFINITY,
                kind: Kind::Oval,
            });
        }
        for _ in 0..EDDIES {
            let e = Self::eddy(rng, period);
            self.vortices.push(Vortex {
                age: rng.random_range(0.0..e.life),
                ..e
            });
        }
    }

    fn eddy(rng: &mut impl Rng, period: f64) -> Vortex {
        let a = rng.random_range(0.025..0.05);
        let spin = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
        Vortex {
            x: rng.random_range(0.0..period),
            y: rng.random_range(0.03..0.97),
            a,
            b: a * rng.random_range(0.6..1.0),
            s: spin * strength_for_period(a, a, rng.random_range(1.2..2.4)),
            age: 0.0,
            life: rng.random_range(EDDY_LIFE.0..EDDY_LIFE.1),
            kind: Kind::Eddy,
        }
    }

    #[cfg(test)]
    /// Velocity at `(x, y)` in `H` units, `H`/s: jets plus every vortex (JUP-A1). The
    /// lattice fill uses the same differences, restricted to each vortex's box.
    pub(crate) fn velocity(&self, x: f64, y: f64, h: f64) -> (f64, f64) {
        let period = self.period();
        let (u, v) = vortex_advect::stream_velocity(
            |px, py| self.vortices.iter().map(|w| w.psi(px, py, period)).sum(),
            x,
            y,
            h,
        );
        (u + jet(y), v)
    }

    /// Write node velocities (cells per second) into the flow.
    fn fill_flow(&mut self) {
        let (w, hh) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        let scale = hh as f64;
        let cell = 1.0 / scale;
        let period = self.period();
        for (j, row) in self.flow.u.as_mut_slice().chunks_exact_mut(w).enumerate() {
            row.fill(self.jets[j]);
        }
        self.flow.v.fill(0.0);
        let (us, vs) = (self.flow.u.as_mut_slice(), self.flow.v.as_mut_slice());
        // Each vortex's velocity is `stream_velocity` of its separable ψ over its ±3σ
        // box; the factors at the node and half a cell either side are computed once
        // per column and per row (stack tables), so a node costs no `exp`.
        const HALF: [f64; 3] = [-0.5, 0.0, 0.5];
        for vx in &self.vortices {
            let s = vx.strength();
            let rx = ((3.0 * vx.a * scale).ceil() as i64)
                .min(w as i64 / 2)
                .min(MAX_BOX);
            let ry = ((3.0 * vx.b * scale).ceil() as i64).min(MAX_BOX);
            let (cx, cy) = ((vx.x * scale).round() as i64, (vx.y * scale).round() as i64);
            let mut ex = [[0.0f64; 3]; 2 * MAX_BOX as usize + 1];
            for (c, i) in (cx - rx..=cx + rx).enumerate() {
                for (m, off) in HALF.iter().enumerate() {
                    let dx = vx.dx((i as f64 + off) * cell, period);
                    ex[c][m] = (-(dx * dx) / (vx.a * vx.a)).exp();
                }
            }
            let (j0, j1) = ((cy - ry).max(0), (cy + ry).min(hh as i64 - 1));
            for j in j0..=j1 {
                let mut ey = [0.0f64; 3];
                for (m, off) in HALF.iter().enumerate() {
                    let dy = (j as f64 + off) * cell - vx.y;
                    ey[m] = (-(dy * dy) / (vx.b * vx.b)).exp();
                }
                for (c, i) in (cx - rx..=cx + rx).enumerate() {
                    let k = j as usize * w + i.rem_euclid(w as i64) as usize;
                    let e = ex[c];
                    let u = (s * e[1] * ey[2] - s * e[1] * ey[0]) / cell;
                    let v = -(s * e[2] * ey[1] - s * e[0] * ey[1]) / cell;
                    us[k] += (u * scale) as f32;
                    vs[k] += (v * scale) as f32;
                }
            }
        }
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        let period = self.period();
        // Storms drift with the jet at their centre (≈ 0 in their lane) plus a slow
        // westward drift; eddies are carried by the jets and respawn when spent.
        for k in 0..self.vortices.len() {
            let v = &mut self.vortices[k];
            let drift = match v.kind {
                Kind::RedSpot => -0.004,
                Kind::Oval => 0.006,
                Kind::Eddy => 0.0,
            };
            v.x = (v.x + (jet(v.y) + drift) * dt).rem_euclid(period);
            v.age += dt;
            if v.age >= v.life {
                self.vortices[k] = Self::eddy(rng, period);
            }
        }
        self.fill_flow();
        let dtf = dt as f32;
        let (dye, red, hot) = (&self.dye, &self.red, &self.hot);
        let (nd, nr, nh) = (
            self.scratch_dye.as_mut_slice(),
            self.scratch_red.as_mut_slice(),
            self.scratch_hot.as_mut_slice(),
        );
        vortex_advect::departures(&self.flow, dtf, XEdge::Wrap, |k, x, y| {
            nd[k] = vortex_advect::sample(dye, x, y, XEdge::Wrap);
            nr[k] = vortex_advect::sample(red, x, y, XEdge::Wrap);
            nh[k] = vortex_advect::sample(hot, x, y, XEdge::Wrap);
        });
        std::mem::swap(&mut self.dye, &mut self.scratch_dye);
        std::mem::swap(&mut self.red, &mut self.scratch_red);
        std::mem::swap(&mut self.hot, &mut self.scratch_hot);

        // Sources: relaxation to the bands; the red chromophore is made in the Great Red
        // Spot's core and bleaches elsewhere.
        let relax = 1.0 - (-RELAX * dtf).exp();
        let bleach = (-RED_DECAY * dtf).exp();
        let clear = (-HOT_DECAY * dtf).exp();
        let hot_shift = HOT_DRIFT * self.t;
        let w = usize::from(self.lattice.width());
        let scale = f64::from(self.lattice.height());
        let (target, vortices) = (self.target.as_slice(), &self.vortices);
        for (k, ((d, r), hs)) in self
            .dye
            .as_mut_slice()
            .iter_mut()
            .zip(self.red.as_mut_slice())
            .zip(self.hot.as_mut_slice())
            .enumerate()
        {
            *d += (target[k] - *d) * relax;
            *r *= bleach;
            *hs *= clear;
            let (x, y) = ((k % w) as f64 / scale, (k / w) as f64 / scale);
            let qy = ((y - HOT_Y) / HOT_AB.1).powi(2);
            if qy < 1.0 {
                for fx in HOT_X {
                    let mut dx = x - (fx * period + hot_shift);
                    dx -= period * (dx / period).round();
                    // A soft-edged source (1 − q, not a hard ellipse) so the clearing
                    // has no lattice-stepped outline.
                    let q = (dx / HOT_AB.0).powi(2) + qy;
                    if q < 1.0 {
                        *hs += ((1.0 - q) as f32 - *hs).max(0.0) * relax * 40.0;
                    }
                }
            }
            // The long-lived storms come first in the list, the eddies after them.
            for v in vortices.iter().take_while(|v| v.kind != Kind::Eddy) {
                let mut dx = x - v.x;
                dx -= period * (dx / period).round();
                let q = (dx / v.a).powi(2) + ((y - v.y) / v.b).powi(2);
                match v.kind {
                    // The chromophore is made in the spot's core; its own differential
                    // rotation winds the dye inside into a spiral.
                    // It enters on the spot's east side only, so the spot's differential
                    // rotation winds it into a spiral with paler gaps.
                    Kind::RedSpot if q < 0.6 => {
                        if dx > 0.0 && q > 0.12 {
                            *r += (1.0 - *r) * relax * 60.0;
                        }
                        *d += (0.45 - *d) * relax * 2.0;
                    }
                    // The pale "hollow" collar round the spot.
                    Kind::RedSpot if q > 1.1 && q < 1.9 => *d += (1.0 - *d) * relax * 6.0,
                    // White ovals: bright cores in a darker collar.
                    Kind::Oval if q < 0.6 => *d += (1.0 - *d) * relax * 12.0,
                    Kind::Oval if q > 1.0 && q < 2.0 => *d += (0.05 - *d) * relax * 10.0,
                    _ => {}
                }
            }
        }
    }

    fn rebuild_lattice(&mut self, lattice: Size) {
        if lattice == self.lattice && !self.dye.is_empty() {
            return;
        }
        let fresh = self.dye.is_empty();
        let old_period = self.period();
        self.lattice = lattice;
        if fresh {
            self.dye.resize(lattice, 0.0);
            self.red.resize(lattice, 0.0);
            self.hot.resize(lattice, 0.0);
        } else {
            grid::regrid_bilinear(&mut self.hot, lattice, &mut self.scratch_hot);
            grid::regrid_bilinear(&mut self.dye, lattice, &mut self.scratch_dye);
            grid::regrid_bilinear(&mut self.red, lattice, &mut self.scratch_red);
            let k = self.period() / old_period;
            for v in &mut self.vortices {
                v.x *= k;
            }
        }
        self.scratch_dye.resize(lattice, 0.0);
        self.scratch_red.resize(lattice, 0.0);
        self.scratch_hot.resize(lattice, 0.0);
        self.target.resize(lattice, 0.0);
        self.flow.resize(lattice);
        let (w, h) = (usize::from(lattice.width()), usize::from(lattice.height()));
        let scale = h as f64;
        let period = self.period();
        let period_cells = (30.0 * period).round().max(1.0) as i32;
        for (k, t) in self.target.as_mut_slice().iter_mut().enumerate() {
            let (x, y) = ((k % w) as f64 / scale, (k / w) as f64 / scale);
            *t = band(x * f64::from(period_cells) / period, y, period_cells) as f32;
        }
        self.jets.clear();
        self.jets
            .extend((0..h).map(|j| (jet(j as f64 / scale) * scale) as f32));
        if fresh {
            self.dye
                .as_mut_slice()
                .copy_from_slice(self.target.as_slice());
        }
    }

    /// The disc in dots: centre, horizontal and vertical radius (oblate).
    fn disc(&self) -> (f64, f64, f64, f64) {
        let ds = self.dots.dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        // A pixel is 2 × 2 dots; the radius is DISC of the smaller side in pixels.
        let r = 2.0 * DISC * (0.5 * w).min(0.5 * h);
        (0.5 * w - 0.5, 0.5 * h - 0.5, r, r * OBLATE)
    }

    /// Lay the node grid over the disc and fix each node's geometry (M16).
    fn layout(&mut self) {
        let (cx, cy, rx, ry) = self.disc();
        let s = (2.0 * rx / MAX_ACROSS).max(2.0);
        let nw = (2.0 * (rx + 2.0 * s) / s).ceil() as usize + 1;
        let nh = (2.0 * (ry + 2.0 * s) / s).ceil() as usize + 1;
        self.spacing = s;
        self.nodes = (nw, nh);
        self.origin = (
            cx - 0.5 * (nw - 1) as f64 * s,
            cy - 0.5 * (nh - 1) as f64 * s,
        );
        self.geo.clear();
        for j in 0..nh {
            for i in 0..nw {
                let (x, y) = (self.origin.0 + i as f64 * s, self.origin.1 + j as f64 * s);
                let (nx, ny) = ((x - cx) / rx.max(1e-9), (y - cy) / ry.max(1e-9));
                let rho2 = nx * nx + ny * ny;
                if rho2 >= 1.0 {
                    self.geo.push(Geo::OUTSIDE);
                    continue;
                }
                let nz = (1.0 - rho2).sqrt();
                // Planetographic latitude and the longitude off the centre meridian.
                let lat = (-ny).clamp(-1.0, 1.0).asin();
                let ly = 0.5 - lat / LAT_SPAN;
                let polar = ease::smoothstep((ly.abs().max((1.0 - ly).abs()) - 1.0) / 0.25);
                self.geo.push(Geo {
                    lon: nx.atan2(nz),
                    ly,
                    polar: polar as f32,
                    mu: nz as f32,
                });
            }
        }
        let n = nw * nh;
        for v in [&mut self.band_v, &mut self.red_v, &mut self.hot_v] {
            v.clear();
            v.resize(n, f32::NAN);
        }
    }

    /// The longitude shift of the view: it follows the Great Red Spot, swaying about it.
    fn spin(&self) -> f64 {
        self.vortices.first().map_or(0.0, |v| v.x * LAT_SPAN) + SWAY.0 * (SWAY.1 * self.t).sin_det()
    }

    /// Lattice coordinates `(lx, ly)` (in `H`) under node geometry `g` for view `spin`.
    fn lattice_of(&self, g: &Geo, spin: f64) -> (f64, f64) {
        (
            ((g.lon + spin) / LAT_SPAN).rem_euclid(self.period()),
            g.ly.clamp(0.0, 1.0),
        )
    }

    /// `q = (dx/a)² + (dy/b)²` of the point `(lx, ly)` in vortex `v`'s ellipse.
    fn ellipse_q(&self, v: &Vortex, lx: f64, ly: f64) -> f64 {
        let dx = v.dx(lx, self.period()) / v.a;
        let dy = (ly - v.y) / v.b;
        dx * dx + dy * dy
    }

    /// Io's shadow in dots: centre and radius.
    fn shadow(&self) -> (f64, f64, f64) {
        let (cx, cy, rx, _) = self.disc();
        // In pixels, as before M16: it crosses every `1 / SHADOW_SPEED` s.
        let (pcx, pcy, r) = (0.5 * (cx + 0.5), 0.5 * (cy + 0.5), 0.5 * rx);
        let r_sh = (0.05 * r).max(1.0);
        let span = 2.0 * r + 4.0 * r_sh;
        let sx = pcx - r - 2.0 * r_sh + ((0.35 + SHADOW_SPEED * self.t) * span).rem_euclid(span);
        let sy = pcy - 0.35 * r;
        (2.0 * sx - 0.5, 2.0 * sy - 0.5, 2.0 * r_sh)
    }

    /// Sample the dyes at every node (NaN off the disc and under Io's shadow).
    fn sample_nodes(&mut self) {
        let spin = self.spin();
        let lat_h = f64::from(self.lattice.height());
        let (sx, sy, sr) = self.shadow();
        let grs = self.vortices.first().copied();
        let (nw, _) = self.nodes;
        for k in 0..self.geo.len() {
            let g = self.geo[k];
            let (x, y) = (
                self.origin.0 + (k % nw) as f64 * self.spacing,
                self.origin.1 + (k / nw) as f64 * self.spacing,
            );
            if g.mu < 0.0 || (x - sx).hypot(y - sy) < sr {
                self.band_v[k] = f32::NAN;
                self.red_v[k] = f32::NAN;
                self.hot_v[k] = f32::NAN;
                continue;
            }
            let (lx, ly) = self.lattice_of(&g, spin);
            let (gx, gy) = ((lx * lat_h) as f32, (ly * lat_h) as f32);
            let a = vortex_advect::sample(&self.dye, gx, gy, XEdge::Wrap);
            // Toward the poles the bands fade into a featureless hood: no threads there.
            self.band_v[k] = a + (0.5 - a) * g.polar;
            self.red_v[k] = match grs {
                Some(v) if self.ellipse_q(&v, lx, ly) < 2.5 => {
                    vortex_advect::sample(&self.red, gx, gy, XEdge::Wrap)
                }
                _ => 0.0,
            };
            self.hot_v[k] = if (ly - HOT_Y).abs() < HOT_LANE {
                vortex_advect::sample(&self.hot, gx, gy, XEdge::Wrap)
            } else {
                0.0
            };
        }
    }

    /// The ink of a band thread at band level `level` (index into `BAND_LEVELS`) where
    /// the lattice point is `(lx, ly)` and the limb factor `mu` (JUP-A9..A11).
    fn band_ink(&self, level: usize, lx: f64, ly: f64, mu: f32) -> Ink {
        let mut storms = self.vortices.iter().take_while(|v| v.kind != Kind::Eddy);
        if let Some(grs) = storms.next() {
            if self.ellipse_q(grs, lx, ly) < 1.0 {
                return SPOT_INK;
            }
        }
        if storms
            .any(|v| v.kind == Kind::Oval && self.ellipse_q(v, lx, ly) < OVAL_REACH * OVAL_REACH)
        {
            return OVAL_INK;
        }
        band_level_ink(level, mu)
    }

    /// Draw the frame: band, spot and festoon threads, then the limb.
    fn raster(&mut self) {
        self.dots.clear();
        self.sample_nodes();
        let (nw, nh) = self.nodes;
        let (ox, oy, s) = (self.origin.0, self.origin.1, self.spacing);
        let to_dot = move |x: f64, y: f64| (ox + x * s, oy + y * s);
        let spin = self.spin();
        let corners = |v: &[f32], i: usize, j: usize| {
            let k = j * nw + i;
            [v[k], v[k + 1], v[k + nw + 1], v[k + nw]]
        };
        let crosses = |c: &[f32; 4], level: f32| {
            let below = c.map(|v| v < level);
            c.iter().all(|v| v.is_finite()) && below != [true; 4] && below != [false; 4]
        };
        for j in 0..nh.saturating_sub(1) {
            for i in 0..nw.saturating_sub(1) {
                let g = self.geo[j * nw + i];
                if g.mu < 0.0 {
                    continue;
                }
                let band = corners(&self.band_v, i, j);
                for (n, &(level, _, _)) in BAND_LEVELS.iter().enumerate() {
                    if crosses(&band, level) {
                        let (lx, ly) = self.lattice_of(&g, spin);
                        let ink = self.band_ink(n, lx, ly, g.mu);
                        self.dots.contour_cell(band, level, (i, j), to_dot, ink);
                    }
                }
                let red = corners(&self.red_v, i, j);
                for &(level, l) in &RED_LEVELS {
                    if crosses(&red, level) {
                        let ink = Ink::new(Role::Hot, l);
                        self.dots.contour_cell(red, level, (i, j), to_dot, ink);
                    }
                }
                let hot = corners(&self.hot_v, i, j);
                if crosses(&hot, FESTOON.0) {
                    self.dots
                        .contour_cell(hot, FESTOON.0, (i, j), to_dot, FESTOON.1);
                }
            }
        }
        // The limb: a thin dot ellipse.
        let (cx, cy, rx, ry) = self.disc();
        let point = |k: usize| {
            let (sn, c) = (TAU * k as f64 / LIMB_PTS as f64).sin_cos_det();
            (cx + rx * c, cy + ry * sn)
        };
        for k in 0..LIMB_PTS {
            let (a, b) = (point(k), point(k + 1));
            self.dots.line(a.0, a.1, b.0, b.1, LIMB_INK);
        }
    }
}

/// A band thread's ink away from the storms: belt side `Hot`, zone side `Sun`, one step
/// dimmer near the limb (`μ < LIMB_MU`, limb darkening).
pub(crate) fn band_level_ink(level: usize, mu: f32) -> Ink {
    let (_, role, steps) = BAND_LEVELS[level.min(BAND_LEVELS.len() - 1)];
    Ink::new(role, steps[usize::from(mu < LIMB_MU)])
}

/// Per-node geometry of the disc (fixed per resize): longitude off the centre meridian,
/// lattice row (in `H`, unclamped), polar-hood weight and `μ = cos` of the view angle
/// (negative off the disc).
#[derive(Debug, Clone, Copy)]
struct Geo {
    lon: f64,
    ly: f64,
    polar: f32,
    mu: f32,
}

impl Geo {
    const OUTSIDE: Geo = Geo {
        lon: 0.0,
        ly: 0.0,
        polar: 0.0,
        mu: -1.0,
    };
}

impl Generator for JupiterStorms {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        // The flow lives on a longitude × latitude strip four times as wide as tall: 260° of
        // longitude, so the spot's next copy is always round the back.
        let d = size.height().saturating_mul(2).min(size.width()).max(1);
        let display = Size::new(d.saturating_mul(4), d);
        let lattice = grid::lattice_for(display, CAP, BUCKET);
        if !self.warmed {
            // GEN-R19: the spin-up runs on a coarse lattice and is regridded up; the
            // filaments sharpen on the fine lattice over the first frames.
            self.warmed = true;
            self.rebuild_lattice(grid::lattice_for(display, SPIN_UP_CAP, BUCKET));
            self.seed_storms(&mut ctx.rng);
            for _ in 0..SPIN_UP {
                self.step(SIM_DT, &mut ctx.rng);
            }
        }
        self.rebuild_lattice(lattice);
        self.dots.resize(size);
        self.layout();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.t += dt;
            self.step(dt, &mut ctx.rng);
        }
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
    use sciart_core::{Stage, geom};

    fn scene() -> (JupiterStorms, Ctx) {
        let mut g = JupiterStorms::new();
        let mut ctx = Ctx::new(42, Size::new(100, 40));
        g.resize(Size::new(100, 40), &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn jup_a1_velocity_field_is_divergence_free() {
        let (g, _) = scene();
        let h = 1.0 / f64::from(g.lattice.height());
        for k in 0..400 {
            let (x, y) = (
                0.013 * f64::from(k) * g.period() / 5.2,
                (0.37 * f64::from(k)) % 1.0,
            );
            let e = 0.5 * h;
            let (ue, _) = g.velocity(x + e, y, h);
            let (uw, _) = g.velocity(x - e, y, h);
            let (_, vs) = g.velocity(x, y + e, h);
            let (_, vn) = g.velocity(x, y - e, h);
            let div = (ue - uw + vs - vn) / h;
            assert!(div.abs() < 1e-9, "div {div} at ({x}, {y})");
        }
    }

    #[test]
    fn jup_a1_the_lattice_flow_is_that_stream_field() {
        // The fill caches the separable factors; it must be the field tested above.
        let (mut g, _) = scene();
        g.fill_flow();
        let (w, h) = (
            usize::from(g.lattice.width()),
            usize::from(g.lattice.height()),
        );
        let scale = h as f64;
        for k in (0..w * h).step_by(37) {
            let (x, y) = ((k % w) as f64 / scale, (k / w) as f64 / scale);
            let (u, v) = g.velocity(x, y, 1.0 / scale);
            let (fu, fv) = (
                f64::from(g.flow.u.as_slice()[k]),
                f64::from(g.flow.v.as_slice()[k]),
            );
            // Vortex tails beyond the ±3σ boxes are the only difference (≤ 0.01 cell/s,
            // against jets of ~10 cells/s).
            assert!(
                (fu - u * scale).abs() < 0.01 + 1e-3 * (u * scale).abs(),
                "u at {k}"
            );
            assert!(
                (fv - v * scale).abs() < 0.01 + 1e-3 * (v * scale).abs(),
                "v at {k}: {fv} vs {}",
                v * scale
            );
        }
    }

    #[test]
    fn jup_a2_jets_alternate_between_neighbouring_bands() {
        // Band centres sit halfway between zeros of the jet profile.
        let mut last = 0.0;
        for m in 0..6 {
            let y = (f64::from(m) * PI - PHI) / K;
            if !(0.0..=1.0).contains(&y) {
                continue;
            }
            let u = jet(y);
            assert!(u.abs() > 0.5 * U0, "jet at band {m} too weak: {u}");
            assert!(
                u * last <= 0.0,
                "neighbouring jets share a sign: {last} then {u}"
            );
            last = u;
        }
    }

    #[test]
    fn jup_a3_advection_conserves_dye() {
        let (mut g, _) = scene();
        let before: f64 = g.dye.as_slice().iter().map(|&v| f64::from(v)).sum();
        let mut src = g.dye.clone();
        let mut dst = Grid::default();
        g.fill_flow();
        for _ in 0..60 {
            vortex_advect::advect(&src, &mut dst, &g.flow, SIM_DT as f32, XEdge::Wrap);
            std::mem::swap(&mut src, &mut dst);
        }
        let after: f64 = src.as_slice().iter().map(|&v| f64::from(v)).sum();
        let drift = (after - before).abs() / before;
        assert!(drift < 0.01, "dye changed by {:.3} % in 1 s", 100.0 * drift);
    }

    #[test]
    fn jup_a4_great_red_spot_turns_counter_clockwise() {
        let (g, _) = scene();
        let grs = g.vortices[0];
        assert_eq!(grs.kind, Kind::RedSpot);
        let h = 1e-4;
        // East of the centre the flow goes up the screen, west of it down: CCW with y down.
        let (_, v_east) = g.velocity(grs.x + 0.6 * grs.a, grs.y, h);
        let (_, v_west) = g.velocity(grs.x - 0.6 * grs.a, grs.y, h);
        assert!(v_east < 0.0 && v_west > 0.0, "{v_east} {v_west}");
        // It sits between a westward jet to the north and an eastward one to the south.
        assert!(jet(grs.y - grs.b) < 0.0 && jet(grs.y + grs.b) > 0.0);
    }

    fn canvas_of(g: &JupiterStorms, size: Size) -> Canvas {
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        c
    }

    fn is_braille(c: char) -> bool {
        ('\u{2800}'..='\u{28ff}').contains(&c)
    }

    /// Normalised disc coordinates `(nx, ny)` of dot `(x, y)`.
    fn disc_xy(g: &JupiterStorms, x: f64, y: f64) -> (f64, f64) {
        let (cx, cy, rx, ry) = g.disc();
        ((x - cx) / rx, (y - cy) / ry)
    }

    #[test]
    fn jup_a6_a_thread_planet_on_black_space() {
        let size = Size::new(100, 40);
        let (mut g, mut ctx) = scene();
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let c = canvas_of(&g, size);
        let (mut inside, mut lit_inside, mut dots, mut full) = (0usize, 0usize, 0u32, 0usize);
        for y in 0..40 {
            for x in 0..100 {
                let cell = c.get(x, y).unwrap();
                let lit = cell.ch != ' ';
                if lit {
                    assert!(is_braille(cell.ch), "({x}, {y}) {:?}", cell.ch);
                    assert!(
                        matches!(cell.ink.role, Role::Hot | Role::Sun | Role::Star),
                        "({x}, {y}) {:?}",
                        cell.ink
                    );
                }
                let (nx, ny) = disc_xy(&g, f64::from(2 * x) + 0.5, f64::from(4 * y) + 1.5);
                if nx * nx + ny * ny < 0.9 {
                    inside += 1;
                    if lit {
                        let mask = cell.ch as u32 - 0x2800;
                        lit_inside += 1;
                        dots += mask.count_ones();
                        full += usize::from(mask == 0xff);
                    }
                }
            }
        }
        for (x, y) in [(0, 0), (99, 0), (0, 39), (99, 39)] {
            assert_eq!(c.get(x, y).unwrap().ch, ' ', "corner {x},{y}");
        }
        assert!(
            lit_inside * 10 > inside,
            "{lit_inside} of {inside}: too empty"
        );
        let mean = f64::from(dots) / lit_inside as f64;
        assert!(mean <= 4.0, "{mean:.2} dots per lit cell");
        assert!(full * 10 <= lit_inside, "{full} of {lit_inside} cells full");
    }

    #[test]
    fn jup_a8_red_spot_is_a_spiral_of_hot_threads() {
        // The view follows the spot: at the sway's zero (t = 0) it sits on the disc's
        // centre meridian, at its latitude.
        let size = Size::new(100, 40);
        let (mut g, mut ctx) = scene();
        g.update(SIM_DT, &mut ctx);
        g.t = 0.0;
        g.dirty = true;
        g.prepare();
        let grs = g.vortices[0];
        let spin = g.spin();
        assert!(
            (spin - grs.x * LAT_SPAN).abs() < 1e-12,
            "the view centres the spot"
        );
        let c = canvas_of(&g, size);
        let (mut lit, mut hot) = (0, 0);
        for y in 0..40 {
            for x in 0..100 {
                let (nx, ny) = disc_xy(&g, f64::from(2 * x) + 0.5, f64::from(4 * y) + 1.5);
                let rho2 = nx * nx + ny * ny;
                if rho2 >= 1.0 {
                    continue;
                }
                let geo = Geo {
                    lon: nx.atan2((1.0 - rho2).sqrt()),
                    ly: 0.5 - (-ny).asin() / LAT_SPAN,
                    polar: 0.0,
                    mu: 1.0,
                };
                let (lx, ly) = g.lattice_of(&geo, spin);
                let cell = c.get(x, y).unwrap();
                if g.ellipse_q(&grs, lx, ly) < 0.8 && cell.ch != ' ' {
                    lit += 1;
                    hot += usize::from(cell.ink.role == Role::Hot);
                }
            }
        }
        assert!(lit >= 6, "{lit} lit cells in the spot");
        assert!(hot * 10 >= lit * 8, "{hot} of {lit} Hot");
    }

    #[test]
    fn jup_a9_the_limb_is_a_thin_ellipse_and_darkened() {
        let (g, _) = scene();
        let (cx, cy, rx, ry) = g.disc();
        for k in 0..36 {
            let (s, c) = (TAU * f64::from(k) / 36.0).sin_cos_det();
            let (px, py) = (cx + rx * c, cy + ry * s);
            let near = (-2..=2).any(|dy| {
                (-2..=2).any(|dx| {
                    let (x, y) = (px.round() as i32 + dx, py.round() as i32 + dy);
                    g.dots.is_lit(x, y) && (f64::from(x) - px).hypot(f64::from(y) - py) <= 1.5
                })
            });
            assert!(near, "no limb dot at direction {k}");
        }
        let ds = g.dots.dot_size();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if g.dots.is_lit(x, y) {
                    let (nx, ny) = disc_xy(&g, f64::from(x), f64::from(y));
                    let out = (nx.hypot(ny) - 1.0) * rx;
                    assert!(out <= 2.0, "dot ({x}, {y}) {out:.1} dots outside the limb");
                }
            }
        }
        for level in 0..BAND_LEVELS.len() {
            let (disc, limb) = (band_level_ink(level, 0.9), band_level_ink(level, 0.2));
            assert_eq!(disc.role, limb.role);
            assert!(limb.level < disc.level, "{disc:?} vs {limb:?}");
        }
    }

    #[test]
    fn jup_a10_belts_and_zones_are_threads() {
        let (g, _) = scene();
        for (level, role, _) in BAND_LEVELS {
            assert_eq!(role, if level < 0.5 { Role::Hot } else { Role::Sun });
        }
        let (cx, cy, _, ry) = g.disc();
        // Separate runs of lit dots down the centre column, inside the limb.
        let x = cx.round() as i32;
        let (mut runs, mut was) = (0, false);
        for y in (cy - 0.9 * ry) as i32..=(cy + 0.9 * ry) as i32 {
            let lit = (x - 1..=x + 1).any(|x| g.dots.is_lit(x, y));
            runs += usize::from(lit && !was);
            was = lit;
        }
        assert!(runs >= 8, "{runs} threads cross the centre column");
    }

    #[test]
    fn jup_a11_white_ovals_are_star_threads() {
        let size = Size::new(100, 40);
        let (g, _) = scene();
        let c = canvas_of(&g, size);
        let spin = g.spin();
        let (cx, cy, rx, ry) = g.disc();
        let mut seen = 0;
        for v in g.vortices.iter().filter(|v| v.kind == Kind::Oval) {
            let lon = geom::wrap_angle(v.x * LAT_SPAN - spin);
            if lon.abs() > 1.0 {
                continue;
            }
            let lat = (0.5 - v.y) * LAT_SPAN;
            let (x, y) = (
                cx + rx * lat.cos_det() * lon.sin_det(),
                cy - ry * lat.sin_det(),
            );
            let (col, row) = ((x / 2.0) as i32, (y / 4.0) as i32);
            let star = (row - 2..=row + 2).any(|yy| {
                (col - 4..=col + 4).any(|xx| {
                    c.get(xx, yy)
                        .is_some_and(|cell| cell.ch != ' ' && cell.ink.role == Role::Star)
                })
            });
            assert!(star, "no Star thread at the oval near ({col}, {row})");
            seen += 1;
        }
        assert!(seen >= 1, "no white oval in view");
    }

    #[test]
    fn jup_a7_great_red_spot_has_todays_shape() {
        // ~1.4:1 (it was ~2:1 in the Voyager era and has been rounding since).
        let (g, _) = scene();
        let grs = g.vortices[0];
        let e = grs.a / grs.b;
        assert!((1.3..=1.5).contains(&e), "elongation {e}");
    }

    #[test]
    fn jup_a5_lattice_stays_within_its_cap() {
        let mut stage = Stage::new(build(), 42, Size::new(500, 150), GlyphMode::Unicode);
        stage.step(SIM_DT);
        let mut g = JupiterStorms::new();
        let mut ctx = Ctx::new(1, Size::new(500, 150));
        g.resize(Size::new(500, 150), &mut ctx);
        g.prepare();
        assert!(g.lattice.area() <= CAP, "{:?}", g.lattice);
        assert_eq!(g.dye.size(), g.lattice);
    }
}
