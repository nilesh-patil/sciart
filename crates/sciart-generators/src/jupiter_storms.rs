//! Jupiter's cloud tops in close-up, JunoCam style (SPEC-002 `jupiter_storms`).
//!
//! Two dyes ride a prescribed flow (`vortex_advect`, semi-Lagrangian on a capped
//! lattice): the zone/belt dye, pale in the anticyclonic zones and blue-grey in the
//! cyclonic belts, and the red chromophore of the Great Red Spot. The flow is alternating
//! zonal jets plus Gaussian vortices given as stream functions: the Great Red Spot and
//! white ovals, anticyclones sitting in the zero of the jets whose shear matches their
//! spin (so they roll like ball bearings), and short-lived small eddies that wind the
//! dye into filaments. A weak relaxation back to the banded pattern keeps the belts from
//! mixing out. The planet's limb curves across the top of the frame.
//!
//! Units: lengths in lattice heights (`H`), so the flow keeps its shape when the lattice
//! is regridded (ADR-0012); speeds in `H` per simulated second.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
use sciart_core::{Grid, ease, ink};

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
/// Great Red Spot: half-axes (2:1), turnover period and its lane.
const GRS_A: f64 = 0.17;
const GRS_B: f64 = 0.085;
const GRS_PERIOD: f64 = 6.0;
/// Small eddies alive at once and their life, seconds.
const EDDIES: usize = 30;
const EDDY_LIFE: (f64, f64) = (5.0, 10.0);
/// Io's shadow crosses the frame width in `1 / SHADOW_SPEED` seconds.
const SHADOW_SPEED: f64 = 1.0 / 30.0;
/// Steps run in the first resize, so the first frame already shows curls.
const SPIN_UP: usize = 100;
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
    scratch_dye: Grid<f32>,
    scratch_red: Grid<f32>,
    /// The banded pattern the dye relaxes to (static per lattice).
    target: Grid<f32>,
    flow: Flow,
    /// Jet speed per lattice row, cells per second.
    jets: Vec<f32>,
    vortices: Vec<Vortex>,
    map: SampleMap,
    pixels: Pixels,
    warmed: bool,
    /// Simulated seconds shown (drives Io's shadow).
    t: f64,
}

impl JupiterStorms {
    fn new() -> Self {
        Self {
            lattice: Size::new(1, 1),
            dye: Grid::default(),
            red: Grid::default(),
            scratch_dye: Grid::default(),
            scratch_red: Grid::default(),
            target: Grid::default(),
            flow: Flow::default(),
            jets: Vec::new(),
            vortices: Vec::with_capacity(8 + EDDIES),
            map: SampleMap::default(),
            pixels: Pixels::default(),
            warmed: false,
            t: 0.0,
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
        for (m, x) in [(0.0, 0.2), (0.0, 0.75), (2.0, 0.1), (2.0, 0.45), (2.0, 0.8)] {
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
        let (dye, red) = (&self.dye, &self.red);
        let (nd, nr) = (
            self.scratch_dye.as_mut_slice(),
            self.scratch_red.as_mut_slice(),
        );
        vortex_advect::departures(&self.flow, dtf, XEdge::Wrap, |k, x, y| {
            nd[k] = vortex_advect::sample(dye, x, y, XEdge::Wrap);
            nr[k] = vortex_advect::sample(red, x, y, XEdge::Wrap);
        });
        std::mem::swap(&mut self.dye, &mut self.scratch_dye);
        std::mem::swap(&mut self.red, &mut self.scratch_red);

        // Sources: relaxation to the bands; the red chromophore is made in the Great Red
        // Spot's core and bleaches elsewhere.
        let relax = 1.0 - (-RELAX * dtf).exp();
        let bleach = (-RED_DECAY * dtf).exp();
        let w = usize::from(self.lattice.width());
        let scale = f64::from(self.lattice.height());
        let (target, vortices) = (self.target.as_slice(), &self.vortices);
        for (k, (d, r)) in self
            .dye
            .as_mut_slice()
            .iter_mut()
            .zip(self.red.as_mut_slice())
            .enumerate()
        {
            *d += (target[k] - *d) * relax;
            *r *= bleach;
            let (x, y) = ((k % w) as f64 / scale, (k / w) as f64 / scale);
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
        } else {
            grid::regrid_bilinear(&mut self.dye, lattice, &mut self.scratch_dye);
            grid::regrid_bilinear(&mut self.red, lattice, &mut self.scratch_red);
            let k = self.period() / old_period;
            for v in &mut self.vortices {
                v.x *= k;
            }
        }
        self.scratch_dye.resize(lattice, 0.0);
        self.scratch_red.resize(lattice, 0.0);
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

    fn shade(&mut self) {
        let px = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(px.width()), f64::from(px.height()));
        // The limb: a large circle whose top grazes the frame; beyond it, space.
        let r_limb = 1.7 * pw.max(ph);
        let (lcx, lcy) = (0.5 * pw, 0.06 * ph + r_limb);
        // Io's shadow crossing the clouds (as in JunoCam frames): black umbra, a thin
        // penumbra, drifting east with Io's orbit.
        let r_sh = (0.045 * ph).max(1.5);
        let span = pw + 4.0 * r_sh;
        let sx = (0.3 * pw + SHADOW_SPEED * pw * self.t).rem_euclid(span) - 2.0 * r_sh;
        let sy = 0.3 * ph;
        let (dye, red, map) = (&self.dye, &self.red, &self.map);
        self.pixels.shade(|x, y| {
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let ds = (fx - sx).hypot(fy - sy) / r_sh;
            if ds < 1.0 {
                return Ink::BLANK;
            }
            let penumbra = if ds < 1.6 {
                0.35 + 0.65 * (ds - 1.0) / 0.6
            } else {
                1.0
            };
            let d = (fx - lcx).hypot(fy - lcy);
            let depth = r_limb - d;
            if depth < 0.0 {
                return Ink::BLANK;
            }
            if depth < 1.2 {
                // The thin blue haze layer on the limb.
                return ink::ramp(Ramp::Cobalt, 0.3 + 0.35 * depth.max(0.0));
            }
            // Darker and hazier towards the limb (μ from the depth inside the circle).
            let mu = (depth / (0.35 * ph)).clamp(0.0, 1.0).sqrt();
            let shade = (0.45 + 0.55 * mu) * penumbra;
            let a = map.sample(dye, x, y);
            let r = map.sample(red, x, y);
            // Relief: brighter where the dye rises to the upper left (sunlit side of a
            // cloud edge), the embossed look of enhanced JunoCam frames.
            let up = map.sample(dye, x - 1, y - 1);
            let relief = f64::from(a - up) * 3.2;
            let a = f64::from(a);
            let r = f64::from(r).min(1.0);
            if r > 0.1 {
                // The Great Red Spot: brick red in the core, easing through salmon to a
                // pale orange rim, so it melts into the zone around it; the wound-up
                // dye and the relief give it internal spiral texture.
                let deep = ease::smoothstep(((r - 0.1) / 0.6).min(1.0));
                let v = (0.6 - 0.3 * deep + 0.1 * (a - 0.5) + 0.5 * relief) * shade;
                return ink::ramp(Ramp::Ember, v.clamp(0.2, 0.85));
            }
            if a > 0.5 {
                // Zones: fine streaks (the static mottle is carried off by the jets, so
                // this adds grain the lattice is too coarse to hold).
                let v = (0.45 + 1.0 * (a - 0.5) + relief) * shade;
                ink::ramp(Ramp::Starlight, v.clamp(0.1, 1.0))
            } else {
                let v = (0.1 + 0.7 * a + relief) * shade;
                ink::ramp(Ramp::Cobalt, v.clamp(0.08, 0.8))
            }
        });
    }
}

impl Generator for JupiterStorms {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let display = Size::new(size.width(), size.height().saturating_mul(2).max(1));
        let lattice = grid::lattice_for(display, CAP, BUCKET);
        self.rebuild_lattice(lattice);
        if !self.warmed {
            self.warmed = true;
            self.seed_storms(&mut ctx.rng);
            for _ in 0..SPIN_UP {
                self.step(SIM_DT, &mut ctx.rng);
            }
        }
        self.pixels.resize(size);
        self.map.rebuild(self.lattice, self.pixels.pixel_size());
        self.shade();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.t += dt;
            self.step(dt, &mut ctx.rng);
        }
        self.shade();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::Stage;

    fn scene() -> (JupiterStorms, Ctx) {
        let mut g = JupiterStorms::new();
        let mut ctx = Ctx::new(42, Size::new(100, 40));
        g.resize(Size::new(100, 40), &mut ctx);
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

    #[test]
    fn jup_a5_lattice_stays_within_its_cap() {
        let mut stage = Stage::new(build(), 42, Size::new(500, 150), GlyphMode::Unicode);
        stage.step(SIM_DT);
        let mut g = JupiterStorms::new();
        let mut ctx = Ctx::new(1, Size::new(500, 150));
        g.resize(Size::new(500, 150), &mut ctx);
        assert!(g.lattice.area() <= CAP, "{:?}", g.lattice);
        assert_eq!(g.dye.size(), g.lattice);
    }
}
