//! A mature hurricane seen from above (SPEC-002 `hurricane`).
//!
//! Cloud water rides a kinematic vortex on a capped lattice (`vortex_advect`,
//! semi-Lagrangian): a Holland-profile tangential wind, counter-clockwise as in the
//! northern hemisphere, plus a low-level inflow that stops at the eyewall and so draws
//! the bands in as logarithmic spirals. Convection feeds the eyewall and three spiral rain
//! bands (pulsing, noise-modulated cells), subsidence clears the eye, and evaporation
//! clears the ocean between the bands. A `Timeline` alternates the visible picture with an
//! enhanced-infrared one, in which the coldest (highest) cloud tops glow.
//!
//! Units: lengths in lattice heights (`H`), speeds in `H` per simulated second, so the
//! storm keeps its shape when the lattice is regridded (ADR-0012).

use std::f64::consts::TAU;

use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, half_block};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Grid, ink};

use crate::shared::vortex_advect::{self, Flow, XEdge};
use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "hurricane",
    category: Category::EarthSky,
    description: "Spiral rain bands wind counter-clockwise into a clear eye; an infrared act",
    title: "Hurricane · Coriolis spirals",
    captions: &["visible", "infrared"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Hurricane::new())
}

/// Lattice cap (HUR-A5, ADR-0012) and aspect bucket.
pub(crate) const CAP: usize = 160 * 90;
const BUCKET: f64 = 1.25;

/// Storm radius (the outer edge of the bands), radius of maximum wind and the Holland
/// shape parameter, in `H`.
const R_STORM: f64 = 0.46;
pub(crate) const R_MAX_WIND: f64 = 0.14 * R_STORM;
const HOLLAND_B: f64 = 1.5;
/// Peak wind: one turn at the radius of maximum wind every 3 s (time-lapsed).
const V_MAX: f64 = TAU * R_MAX_WIND / 3.0;
/// Inflow as a fraction of the tangential wind outside the eyewall.
const INFLOW: f64 = 0.28;
/// Spiral bands: count and pitch angle.
const BANDS: usize = 3;
const PITCH: f64 = 0.28;
/// Rates per second: evaporation everywhere, subsidence in the eye, band and eyewall
/// convection.
const EVAPORATE: f32 = 0.4;
const SUBSIDE: f32 = 5.0;
const BAND_RATE: f32 = 1.4;
const WALL_RATE: f32 = 2.5;
/// The eye and eyewall in radii of maximum wind: subsidence clears `r < EYE`, the
/// eyewall convects from `WALL_IN` to 1.5, and the central dense overcast is fed at
/// `CDO_RATE` out to `CDO_OUT` (HUR-A3, HUR-A9).
const EYE: f64 = 0.6;
const WALL_IN: f64 = 0.75;
const CDO_OUT: f64 = 2.2;
const CDO_RATE: f32 = 1.3;
/// Cirrus outflow streaks: peak brightness and pitch angle (rad) of their clockwise
/// spiral.
const OUTFLOW: f64 = 0.07;
const OUTFLOW_PITCH: f64 = 0.45;
/// Brightness below which the sea is clear, and above which cloud is opaque.
const CLEAR: f64 = 0.012;
const THICK: f64 = 0.07;
/// Convective texture tile: texels per side, the disc it spans (in `R_STORM`) and how
/// fast it turns with the storm, rad/s.
const TURRET_N: usize = 256;
const TURRET_SPAN: f64 = 1.15;
const TURRET_SPIN: f64 = 0.35;
/// Radial wind table resolution over `[0, 2·R_STORM]`.
const TABLE: usize = 512;
/// Steps run in the first resize, so the first frame is a mature storm.
const SPIN_UP: usize = 180;
/// Seconds of dithered dissolve between the visible and infrared pictures.
const DISSOLVE: f64 = 1.5;

/// The open sea (`Ramp(Ice)`): its level floor, swell texture depth and the extra
/// brightness at the centre of the sunglint, whose mirror point sits at `GLINT` (a share
/// of the frame width, `H` down) with a Gaussian radius `GLINT_R` (`H`) (HUR-A6).
const SEA: f64 = 0.085;
const SEA_SWELL: f64 = 0.035;
const SEA_GLINT: f64 = 0.22;
const GLINT: (f64, f64) = (0.3, 0.3);
const GLINT_R: f64 = 0.26;
/// Thin cloud below this `Ramp` value is drawn as lighter sea blue, above it as grey-white.
const HAZE_TOP: f64 = 0.26;
/// Lattices at most this tall (80×24 and smaller) are too coarse for the tight inner
/// bands: the secondary band and fine texture drop out and the picture is box-blurred
/// once (HUR-A8).
const SMALL: u16 = 64;
/// Steps between swell refreshes (0.1 s): the swell moves 0.02 noise cells meanwhile.
const SWELL_EVERY: u32 = 6;
/// The enhanced-infrared picture (HUR-A4): the warm sea's level, and the level the
/// cloud-top curve starts from.
const IR_SEA: f64 = 0.1;
const IR_LOW: f64 = 0.16;
/// Cloud-top height factor in the IR act: 1 inside `IR_CORE` (`H`, the overcast), falling
/// linearly to `IR_BANDS` at `IR_OUTER`.
const IR_CORE: f64 = CDO_OUT * R_MAX_WIND;
const IR_OUTER: f64 = 0.9 * R_STORM;
const IR_BANDS: f64 = 0.62;

const ACTS: [(&str, f64); 2] = [("visible", 25.0), ("infrared", 12.0)];
const IR: usize = 1;

/// Holland (1980) tangential wind at radius `r` (`H`), `H`/s: peaks at `R_MAX_WIND`
/// (HUR-A1), decays outward and vanishes at the centre.
pub(crate) fn holland(r: f64) -> f64 {
    if r.is_nan() || r <= 0.0 {
        return 0.0;
    }
    let x = (R_MAX_WIND / r).powf(HOLLAND_B);
    V_MAX * (x * (1.0 - x).exp()).sqrt()
}

/// Inflow speed (positive inward) at radius `r`: a share of the tangential wind outside
/// the eyewall, easing to nothing inside it, so air piles up at the wall and rises.
fn inflow(r: f64) -> f64 {
    let t = ((r - R_MAX_WIND) / (0.6 * R_MAX_WIND)).clamp(0.0, 1.0);
    INFLOW * holland(r) * t * t * (3.0 - 2.0 * t)
}

/// Storm-relative wind at offset `(dx, dy)` (`y` down), `H`/s: counter-clockwise on
/// screen, spiralling in.
#[cfg(test)]
pub(crate) fn wind(dx: f64, dy: f64) -> (f64, f64) {
    let r = dx.hypot(dy);
    wind_from(dx, dy, holland(r), inflow(r))
}

/// The wind at offset `(dx, dy)` from tangential speed `vt` and inflow `vr`.
#[inline]
fn wind_from(dx: f64, dy: f64, vt: f64, vr: f64) -> (f64, f64) {
    let r = dx.hypot(dy);
    if r < 1e-9 {
        return (0.0, 0.0);
    }
    // Counter-clockwise tangent with y down is (dy, −dx)/r; inward is −(dx, dy)/r.
    ((vt * dy - vr * dx) / r, (-vt * dx - vr * dy) / r)
}

/// A 1-2-1 blur in x and then y (edges clamped), through `tmp` (same size as `g`).
fn box_blur(g: &mut Grid<f32>, tmp: &mut Grid<f32>) {
    let (w, h) = (usize::from(g.width()), usize::from(g.height()));
    if w == 0 || h == 0 || tmp.size() != g.size() {
        return;
    }
    let (src, dst) = (g.as_mut_slice(), tmp.as_mut_slice());
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let (l, r) = (x.saturating_sub(1), (x + 1).min(w - 1));
            dst[row + x] = 0.25 * (src[row + l] + src[row + r]) + 0.5 * src[row + x];
        }
    }
    for y in 0..h {
        let (u, d) = (y.saturating_sub(1) * w, (y + 1).min(h - 1) * w);
        for x in 0..w {
            src[y * w + x] = 0.25 * (dst[u + x] + dst[d + x]) + 0.5 * dst[y * w + x];
        }
    }
}

#[derive(Debug)]
pub struct Hurricane {
    t: f64,
    timeline: Timeline,
    lattice: Size,
    cloud: Grid<f32>,
    /// Cloud-top brightness `1 − e^{−1.5·c}`, and the sea's `Ramp(Ice)` value (swell,
    /// glint, curvature; refreshed every `SWELL_EVERY` steps), per node.
    bright: Grid<f32>,
    swell: Grid<f32>,
    /// Cirrus outflow brightness per node, refreshed with the swell.
    cirrus: Grid<f32>,
    /// Steps since the swell was refreshed.
    swell_age: u32,
    /// The sea's `Ramp(Ice)` level per pixel (HUR-A6), resampled from `swell` a sixth
    /// of the rows per frame (`sea_row` is the next), so no frame pays for all of it. A
    /// byte per pixel: the ramp has 256 levels anyway.
    sea: Vec<u8>,
    sea_row: usize,
    /// A coarse lattice (`height ≤ SMALL`): smoothed picture, fewer inner bands.
    small: bool,
    /// Enhanced-IR `Ramp(Infrared)` value by brightness level, before the height factor.
    ir_lut: [f32; 256],
    /// Convective texture (cumulus towers, `[−½, ½]`) on a `TURRET_N²` tile over the
    /// storm disc, fixed per storm, turning slowly with it.
    turrets: Box<[f32]>,
    scratch: Grid<f32>,
    flow: Flow,
    /// `(tangential, inflow)` wind over `[0, 2·R_STORM]`, `H`/s.
    table: Vec<(f32, f32)>,
    map: SampleMap,
    pixels: Pixels,
    warmed: bool,
}

impl Hurricane {
    fn new() -> Self {
        Self {
            t: 0.0,
            timeline: Timeline::new(&ACTS),
            lattice: Size::new(1, 1),
            cloud: Grid::default(),
            bright: Grid::default(),
            swell: Grid::default(),
            cirrus: Grid::default(),
            swell_age: SWELL_EVERY,
            sea: Vec::new(),
            sea_row: 0,
            small: false,
            turrets: (0..TURRET_N * TURRET_N)
                .map(|k| {
                    let (x, y) = ((k % TURRET_N) as f64, (k / TURRET_N) as f64);
                    // Two octaves of cauliflower-like bumps, a few pixels across.
                    let n = 0.65 * value_noise(0x7EE7, x, y, 12.0)
                        + 0.35 * value_noise(0x7EE8, x, y, 5.0);
                    (n * n * 1.6 - 0.5) as f32
                })
                .collect(),
            ir_lut: std::array::from_fn(|k| {
                // Brightness stands in for cloud-top height, hence coldness: thin low
                // cloud stays in the curve's grey, the eyewall and convective cores reach
                // its orange and pale top.
                let b = k as f64 / 255.0;
                let x = ((b - CLEAR) / (0.78 - CLEAR)).clamp(0.0, 1.0);
                (IR_LOW + (1.0 - IR_LOW) * x.powf(1.25)) as f32
            }),
            scratch: Grid::default(),
            flow: Flow::default(),
            table: (0..TABLE)
                .map(|k| {
                    let r = 2.0 * R_STORM * k as f64 / (TABLE - 1) as f64;
                    (holland(r) as f32, inflow(r) as f32)
                })
                .collect(),
            map: SampleMap::default(),
            pixels: Pixels::default(),
            warmed: false,
        }
    }

    /// Lattice width in `H`.
    fn width(&self) -> f64 {
        f64::from(self.lattice.width()) / f64::from(self.lattice.height())
    }

    /// Storm centre in `H`: a slow drift about the middle of the frame.
    fn centre(&self, t: f64) -> (f64, f64) {
        let (s, c) = (TAU * t / 90.0).sin_cos_det();
        (0.5 * self.width() + 0.05 * c, 0.5 + 0.03 * s)
    }

    fn wind_at(table: &[(f32, f32)], r: f64) -> (f64, f64) {
        let pos = r / (2.0 * R_STORM) * (TABLE - 1) as f64;
        let k = (pos.max(0.0) as usize).min(TABLE - 2);
        let f = (pos - k as f64).clamp(0.0, 1.0) as f32;
        let (a, b) = (table[k], table[k + 1]);
        (
            f64::from(a.0 + f * (b.0 - a.0)),
            f64::from(a.1 + f * (b.1 - a.1)),
        )
    }

    fn fill_flow(&mut self) {
        let (w, h) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        let scale = h as f64;
        let (cx, cy) = self.centre(self.t);
        let (us, vs) = (self.flow.u.as_mut_slice(), self.flow.v.as_mut_slice());
        for k in 0..w * h {
            let (dx, dy) = ((k % w) as f64 / scale - cx, (k / w) as f64 / scale - cy);
            let r = dx.hypot(dy);
            let (vt, vr) = Self::wind_at(&self.table, r);
            let (u, v) = wind_from(dx, dy, vt, vr);
            us[k] = (u * scale) as f32;
            vs[k] = (v * scale) as f32;
        }
    }

    fn step(&mut self, dt: f64) {
        self.t += dt;
        self.fill_flow();
        vortex_advect::advect(
            &self.cloud,
            &mut self.scratch,
            &self.flow,
            dt as f32,
            XEdge::Clamp,
        );
        std::mem::swap(&mut self.cloud, &mut self.scratch);

        let (w, h) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        let scale = h as f64;
        let (cx, cy) = self.centre(self.t);
        let dtf = dt as f32;
        let evaporate = (-EVAPORATE * dtf).exp();
        let subside = (-SUBSIDE * dtf).exp();
        let t = self.t;
        // The band pattern turns slowly with the storm; `(sp, cp)` rotate the pulse.
        let spin = 0.12 * t;
        let cot = 1.0 / PITCH.tan();
        let (sp, cp) = (0.4 * t).sin_cos_det();
        let (s13, c13) = 1.3f64.sin_cos_det();
        let small = self.small;
        let wall_cells: f32 = if small { 0.1 } else { 0.3 };
        // Bands grow out of the central dense overcast's outer edge; on a coarse lattice
        // they begin a little further out, so the overcast stays one solid disc (HUR-A8).
        let band_start = if small { 3.2 } else { 2.4 } * R_MAX_WIND;
        for (k, c) in self.cloud.as_mut_slice().iter_mut().enumerate() {
            let (dx, dy) = ((k % w) as f64 / scale - cx, (k / w) as f64 / scale - cy);
            let r = dx.hypot(dy);
            *c *= if r < EYE * R_MAX_WIND {
                subside
            } else {
                evaporate
            };
            if !(1e-9..=1.1 * R_STORM).contains(&r) {
                continue;
            }
            // Counter-clockwise screen angle (y down), as its cosine and sine.
            let (ct, st) = (dx / r, -dy / r);
            let mut feed = 0.0f32;
            let rm = r / R_MAX_WIND;
            if (WALL_IN..1.5).contains(&rm) {
                let theta = st.atan2(ct);
                let cell = value_noise(0xE7E, 9.0 * theta + 3.0 * t, 40.0 * r, 1.0);
                feed += WALL_RATE * (1.2 - 2.0 * wall_cells + 2.0 * wall_cells * cell as f32);
            } else if (1.5..CDO_OUT + 0.8).contains(&rm) {
                // The central dense overcast (HUR-A9): the eyewall's cirrus shield, a
                // solid disc out to `CDO_OUT` radii of maximum wind, fading over 0.8 more.
                let s = ((CDO_OUT + 0.8 - rm) / 0.8).clamp(0.0, 1.0);
                feed += CDO_RATE * (s * s * (3.0 - 2.0 * s)) as f32;
            }
            if r > band_start {
                // Logarithmic spirals: the band phase 3·(θ + cot(pitch)·ln r − spin) is
                // constant along curves that wind counter-clockwise inward. cos 3θ and
                // sin 3θ come from the triple-angle formulas, so a node costs one
                // `sin_cos` rather than an `atan2` and three.
                let (sx, cx3) = (BANDS as f64 * (cot * (r / R_MAX_WIND).ln() - spin)).sin_cos_det();
                let (c3, s3) = (4.0 * ct * ct * ct - 3.0 * ct, 3.0 * st - 4.0 * st * st * st);
                let (c1, s1) = (c3 * cx3 - s3 * sx, s3 * cx3 + c3 * sx);
                let c2 = (2.0 * c1 * c1 - 1.0) * c13 - 2.0 * s1 * c1 * s13;
                // A coarse lattice cannot hold the thin secondary band between the main
                // ones: it only aliases into stripes (HUR-A8).
                let second = if small { 0.0 } else { 0.2 };
                // Broad bands, switched on over one radius of maximum wind past
                // `band_start` so they merge into the overcast instead of detaching.
                let grow = ((r - band_start) / R_MAX_WIND).min(1.0);
                let band = grow * ((0.5 + 0.5 * c1).powi(6) + second * (0.5 + 0.5 * c2).powi(10));
                if band < 0.004 {
                    *c += (1.2 - *c).max(0.0) * feed * dtf;
                    continue;
                }
                let cells = value_noise(0xBA7D, 70.0 * dx + 1.5 * t, 70.0 * dy, 1.0).powi(2)
                    + 0.5 * value_noise(0xBA7E, 190.0 * dx, 190.0 * dy - 2.0 * t, 1.0);
                let edge = ((1.1 * R_STORM - r) / (0.5 * R_STORM)).clamp(0.0, 1.0);
                let pulse = 0.7 + 0.3 * (sp * ct + cp * st);
                feed += BAND_RATE * (band * (0.2 + 1.6 * cells) * edge * pulse) as f32;
            }
            *c += (1.2 - *c).max(0.0) * feed * dtf;
        }
        // Cloud-top brightness per node, with the outflow cirrus laid over it.
        for ((b, &c), &ci) in self
            .bright
            .as_mut_slice()
            .iter_mut()
            .zip(self.cloud.as_slice())
            .zip(self.cirrus.as_slice())
        {
            let storm = 1.0 - (-1.5 * c).exp();
            *b = storm + ci * (1.0 - storm);
        }
        // The swell drifts slowly (0.2 noise cells a second), so it is refreshed only
        // every `SWELL_EVERY` steps, in `H` so it keeps its scale when the lattice changes.
        if self.swell_age >= SWELL_EVERY {
            self.fill_swell();
        } else {
            self.swell_age += 1;
        }
        // Soft cloud edges: one 1-2-1 pass everywhere (about 2 px at 120×40), a second
        // on a coarse lattice.
        box_blur(&mut self.bright, &mut self.scratch);
        if small {
            box_blur(&mut self.bright, &mut self.scratch);
        }
    }

    fn rebuild_lattice(&mut self, lattice: Size) {
        if lattice == self.lattice && !self.cloud.is_empty() {
            return;
        }
        if self.cloud.is_empty() {
            self.cloud.resize(lattice, 0.0);
        } else {
            grid::regrid_bilinear(&mut self.cloud, lattice, &mut self.scratch);
        }
        self.lattice = lattice;
        self.small = lattice.height() <= SMALL;
        self.scratch.resize(lattice, 0.0);
        self.bright.resize(lattice, 0.0);
        self.swell.resize(lattice, 0.0);
        self.cirrus.resize(lattice, 0.0);
        // A regridded lattice needs its swell before the next picture.
        self.swell_age = SWELL_EVERY;
        self.flow.resize(lattice);
    }

    /// The sun's mirror point on the sea, in `H`: up and to the west (left), where the
    /// low sun that lights the towers' western faces glints off the ocean (HUR-A6).
    fn glint_centre(&self) -> (f64, f64) {
        (GLINT.0 * self.width(), GLINT.1)
    }

    /// Weight of the infrared picture now: 0 visible, 1 infrared, dissolving between.
    fn ir_weight(&self) -> f64 {
        let beat = self.timeline.at(self.t);
        let dur = ACTS[beat.index].1;
        let into = (beat.local / DISSOLVE).min(1.0);
        let left = ((dur - beat.local) / DISSOLVE).min(1.0);
        if beat.index == IR {
            // Dissolve in at the start and out at the end.
            into.min(left).max(0.0)
        } else {
            0.0
        }
    }

    /// Convective texture at pixel `(x, y)`: the tile, turned with the storm.
    fn turret(t: &[f32], centre: (f64, f64), x: i32, y: i32, rot: (f64, f64), ph: f64) -> f64 {
        let (cx, cy) = centre;
        let (dx, dy) = (
            (f64::from(x) + 0.5) / ph - cx,
            (f64::from(y) + 0.5) / ph - cy,
        );
        let (s, c) = rot;
        let (u, v) = (c * dx - s * dy, s * dx + c * dy);
        let k = TURRET_N as f64 / (2.0 * TURRET_SPAN * R_STORM);
        let (tu, tv) = (u * k + 0.5 * TURRET_N as f64, v * k + 0.5 * TURRET_N as f64);
        if !(0.0..(TURRET_N - 1) as f64).contains(&tu)
            || !(0.0..(TURRET_N - 1) as f64).contains(&tv)
        {
            return 0.0;
        }
        let (i, j) = (tu as usize, tv as usize);
        let (fu, fv) = ((tu - i as f64) as f32, (tv - j as f64) as f32);
        let at = |i: usize, j: usize| t[j * TURRET_N + i];
        let top = at(i, j) + fu * (at(i + 1, j) - at(i, j));
        let bottom = at(i, j + 1) + fu * (at(i + 1, j + 1) - at(i, j + 1));
        f64::from(top + fv * (bottom - top))
    }

    /// The sea per node at the current time (HUR-A6), in `H` so it keeps its scale when
    /// the lattice changes: deep blue with a little swell, lighter in the sunglint (whose
    /// sparkle follows the swell), and a faint darkening toward the frame's edges for the
    /// Earth's curvature. Also the cirrus outflow.
    fn fill_swell(&mut self) {
        self.swell_age = 0;
        let w = usize::from(self.lattice.width()).max(1);
        let scale = f64::from(self.lattice.height().max(1));
        let t = self.t;
        let (cx, cy) = self.centre(t);
        let (gx0, gy0) = self.glint_centre();
        let width = self.width();
        let cot = 1.0 / OUTFLOW_PITCH.tan();
        for (k, (sw, ci)) in self
            .swell
            .as_mut_slice()
            .iter_mut()
            .zip(self.cirrus.as_mut_slice())
            .enumerate()
        {
            let (x, y) = ((k % w) as f64 / scale, (k / w) as f64 / scale);
            let swell = value_noise(0x0CEA, x * 14.0 + 0.2 * t, y * 14.0, 1.0);
            let (gx, gy) = ((x - gx0) / GLINT_R, (y - gy0) / (0.8 * GLINT_R));
            let g = (-(gx * gx + gy * gy)).exp() * (0.75 + 0.5 * swell);
            let (ex, ey) = (2.0 * x / width - 1.0, 2.0 * y - 1.0);
            let limb = 1.0 - 0.25 * (0.5 * (ex * ex + ey * ey)).min(1.0);
            *sw = ((SEA + SEA_SWELL * swell + SEA_GLINT * g) * limb) as f32;
            // Cirrus outflow: thin streaks fanning out clockwise (anticyclonic aloft)
            // round the storm's outer edge, patchy, turning slowly.
            let (dx, dy) = (x - cx, y - cy);
            let rs = dx.hypot(dy) / R_STORM;
            *ci = if (0.7..1.5).contains(&rs) {
                let window = (1.0 - ((rs - 1.0) / 0.45).powi(2)).max(0.0);
                let theta = (-dy).atan2(dx);
                let phase = 22.0 * (theta - cot * rs.ln()) - 0.25 * t;
                let streak = (0.5 + 0.5 * phase.cos_det()).powi(6);
                let patch = value_noise(0xC1C1, 6.0 * x, 6.0 * y, 1.0);
                (OUTFLOW * window * streak * (0.3 + 0.7 * patch)) as f32
            } else {
                0.0
            };
        }
    }

    /// Resample `rows` pixel rows of the sea from the node sea, from `sea_row` on.
    fn refresh_sea(&mut self, rows: usize) {
        let size = self.pixels.pixel_size();
        let (pw, ph) = (usize::from(size.width()), usize::from(size.height()));
        if pw == 0 || ph == 0 {
            return;
        }
        let (map, swell) = (&self.map, &self.swell);
        for _ in 0..rows.min(ph) {
            let y = self.sea_row % ph;
            for (x, s) in self.sea[y * pw..(y + 1) * pw].iter_mut().enumerate() {
                let v = map.sample(swell, x as i32, y as i32);
                *s = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            self.sea_row = (y + 1) % ph;
        }
    }

    fn shade(&mut self) {
        // A sixth of the rows per frame: the whole sea every `SWELL_EVERY` steps.
        let rows = usize::from(self.pixels.pixel_size().height()).div_ceil(SWELL_EVERY as usize);
        self.refresh_sea(rows);
        let ir = self.ir_weight();
        let size = self.pixels.pixel_size();
        let ph = f64::from(size.height());
        let pw = i32::from(size.width());
        let rot = (TURRET_SPIN * self.t).sin_cos_det();
        let centre = self.centre(self.t);
        // Convective texture fades out on a coarse picture, where it only speckles.
        let (texture, relief) = if self.small { (0.0, 0.8) } else { (1.0, 1.6) };
        let (bright, sea_px, map, ir_lut, turrets) = (
            &self.bright,
            &self.sea,
            &self.map,
            &self.ir_lut,
            &self.turrets,
        );
        self.pixels.shade(|x, y| {
            let b = map.sample(bright, x, y);
            let use_ir = ir > 0.0 && (ir >= 1.0 || hash01(0x1A ^ cell_key(x, y)) < ir);
            if use_ir {
                // Enhanced IR (HUR-A4): the warm sea dark grey, low cloud grey, the
                // coldest (thickest, highest) tops through cyan, green and yellow to
                // orange and pale rose.
                // Tops are highest (coldest) in the central dense overcast and eyewall and
                // lower in the outer bands, so the bands stay in the green-yellow part of
                // the curve (HUR-A10).
                if f64::from(b) <= CLEAR {
                    return ink::ramp(Ramp::Infrared, IR_SEA);
                }
                let r = ((f64::from(x) + 0.5) / ph - centre.0)
                    .hypot((f64::from(y) + 0.5) / ph - centre.1);
                let height =
                    1.0 - (1.0 - IR_BANDS) * ((r - IR_CORE) / (IR_OUTER - IR_CORE)).clamp(0.0, 1.0);
                let v = f64::from(ir_lut[((b * 255.0) as usize).min(255)]);
                return ink::ramp(Ramp::Infrared, IR_LOW + (v - IR_LOW) * height);
            }
            let sea = f64::from(sea_px.get((y * pw + x) as usize).copied().unwrap_or(0)) / 255.0;
            let b = f64::from(b);
            if b <= CLEAR {
                return ink::ramp(Ramp::Ice, sea);
            }
            // Cloud (HUR-A7): starts from the sea's own lightness and only brightens it,
            // so thin cloud is a grey veil over the blue, never a dark rim.
            let v = if b > THICK {
                // Relief: tops facing the low sun in the west brighter, lee sides darker;
                // the convective towers add bumps (and their own sunlit and shadowed
                // sides) where the cloud is thick.
                let lee = f64::from(map.sample(bright, x + 1, y + 1));
                let bump = Self::turret(turrets, centre, x, y, rot, ph);
                let bump_lee = Self::turret(turrets, centre, x + 1, y + 1, rot, ph);
                let conv = texture * (b / 0.5).min(1.0);
                // Shading needs a body of cloud: none at the thin edge, so a band's
                // shadowed side never turns into a dark rim.
                let body = ((b - THICK) / 0.2).min(1.0);
                let lit = cloud_v(b) * (1.0 + 0.1 * conv * bump)
                    + relief * body * (b - lee)
                    + 0.15 * conv * (bump - bump_lee);
                sea + (lit - sea) * veil(b)
            } else {
                sea + (cloud_v(b) - sea) * veil(b)
            };
            // The ramps share lightness per level (CORE-R80), so switching from the sea's
            // Ice to the cloud's Starlight keeps L*. The faintest haze stays in Ice, a
            // lighter blue (a grey that dark reads as smoke on the sea), and turns grey at
            // `HAZE_TOP`, below the level where Ice turns cyan.
            let v = v.clamp(sea, 1.0);
            if v < HAZE_TOP {
                ink::ramp(Ramp::Ice, v)
            } else {
                ink::ramp(Ramp::Starlight, v)
            }
        });
    }
}

/// Visible lightness (`Ramp(Starlight)` value) of cloud with top brightness `b`: cloud
/// reflectance saturates fast, so even a modest deck reads white-grey (a square root:
/// one instruction per pixel, where `powf` cost a third of the frame at 500×150).
#[inline]
fn cloud_v(b: f64) -> f64 {
    0.16 + 0.84 * b.max(0.0).sqrt()
}

/// Opacity of cloud with top brightness `b`: thin cloud is a translucent veil over the
/// sea, opaque from `b = 0.14`, so the storm's edge feathers out instead of ending on a
/// hard outline.
fn veil(b: f64) -> f64 {
    let s = ((b - CLEAR) / (0.14 - CLEAR)).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

/// ASCII fallback: the cloud by brightness, the open sea left blank so the storm's shape
/// stays legible in characters.
fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let cloud = |i: Ink| {
        if i.role == Role::Ramp(Ramp::Ice) {
            Ink::BLANK
        } else {
            i
        }
    };
    ascii_ramp_cell(cloud(top), cloud(bottom))
}

impl Generator for Hurricane {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let display = Size::new(size.width(), size.height().saturating_mul(2).max(1));
        self.rebuild_lattice(grid::lattice_for(display, CAP, BUCKET));
        if !self.warmed {
            self.warmed = true;
            for _ in 0..SPIN_UP {
                self.step(SIM_DT);
            }
            self.t = 0.0;
        }
        self.pixels.resize(size);
        self.map.rebuild(self.lattice, self.pixels.pixel_size());
        if self.swell_age >= SWELL_EVERY {
            self.fill_swell();
        }
        // The whole sea now; afterwards a sixth of it per frame.
        let p = self.pixels.pixel_size();
        self.sea.resize(p.area(), 0);
        self.sea_row = 0;
        self.refresh_sea(usize::from(p.height()));
        self.shade();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt);
        }
        self.shade();
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.mode() != GlyphMode::Unicode {
            self.pixels
                .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
            return;
        }
        // One pass, as `Pixels::blit` does, except that a cell whose two pixels match
        // (most of the open sea) is written as a space on that background: it looks the
        // same as a half block and sends the terminal less.
        let cells = self.pixels.pixel_size();
        let blank = Glyph::new(' ', ' ');
        for y in 0..i32::from(cells.height()) / 2 {
            for x in 0..i32::from(cells.width()) {
                let top = self.pixels.get(x, 2 * y).unwrap_or(Ink::BLANK);
                let bottom = self.pixels.get(x, 2 * y + 1).unwrap_or(Ink::BLANK);
                if top == bottom && top != Ink::BLANK {
                    canvas.put_bg(x, y, blank, top, top);
                } else {
                    half_block(canvas, x, y, top, bottom);
                }
            }
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.timeline.at(self.t).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn hur_a1_wind_peaks_at_the_radius_of_maximum_wind_and_turns_ccw() {
        let peak = (1..400)
            .map(|k| f64::from(k) * 0.002)
            .max_by(|a, b| holland(*a).total_cmp(&holland(*b)))
            .unwrap_or(0.0);
        assert!((peak - R_MAX_WIND).abs() < 0.003, "peak at {peak}");
        // East of the centre the wind blows up the screen (y down): counter-clockwise.
        let (_, v) = wind(R_MAX_WIND, 0.0);
        assert!(v < 0.0);
        let (u, _) = wind(0.0, -R_MAX_WIND);
        assert!(u < 0.0, "north of the centre the wind blows west");
    }

    #[test]
    fn hur_a2_a_tracer_spirals_inward_counter_clockwise() {
        let (mut x, mut y) = (0.8 * R_STORM, 0.0);
        let mut angle = 0.0f64; // unwrapped, counter-clockwise on screen
        let mut last_r = x.hypot(y);
        let dt = 0.01;
        for _ in 0..3000 {
            let (u, v) = wind(x, y);
            let (xm, ym) = (x + 0.5 * dt * u, y + 0.5 * dt * v);
            let (u, v) = wind(xm, ym);
            let before = (-y).atan2(x);
            x += dt * u;
            y += dt * v;
            let mut d = (-y).atan2(x) - before;
            d -= TAU * (d / TAU).round();
            assert!(d >= 0.0, "turned clockwise by {d}");
            angle += d;
            let r = x.hypot(y);
            assert!(r <= last_r + 1e-12, "moved outward");
            last_r = r;
        }
        assert!(last_r < 0.5 * R_STORM, "still at {last_r}");
        assert!(angle > PI);
    }

    fn storm() -> Hurricane {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(42, Size::new(120, 40));
        g.resize(Size::new(120, 40), &mut ctx);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        g
    }

    #[test]
    fn hur_a3_the_eye_is_clear() {
        let g = storm();
        let (w, h) = (
            usize::from(g.lattice.width()),
            usize::from(g.lattice.height()),
        );
        let (cx, cy) = g.centre(g.t);
        let (mut eye, mut ne, mut wall, mut nw) = (0.0, 0, 0.0, 0);
        for (k, &c) in g.cloud.as_slice().iter().enumerate() {
            let (dx, dy) = (
                (k % w) as f64 / h as f64 - cx,
                (k / w) as f64 / h as f64 - cy,
            );
            let r = dx.hypot(dy);
            if r < 0.5 * R_MAX_WIND {
                eye += f64::from(c);
                ne += 1;
            } else if (R_MAX_WIND..1.6 * R_MAX_WIND).contains(&r) {
                wall += f64::from(c);
                nw += 1;
            }
        }
        let (eye, wall) = (eye / f64::from(ne.max(1)), wall / f64::from(nw.max(1)));
        assert!(ne > 0 && nw > 0);
        assert!(eye < 0.1 * wall, "eye {eye} vs eyewall {wall}");
    }

    #[test]
    fn hur_a4_the_infrared_act_uses_the_enhancement_ramp_only() {
        let mut g = storm();
        let mut ctx = Ctx::new(1, Size::new(60, 20));
        g.resize(Size::new(60, 20), &mut ctx);
        let mid_ir = ACTS[0].1 + 0.5 * ACTS[1].1;
        while g.t < mid_ir {
            g.update(0.25, &mut ctx);
        }
        assert_eq!(g.caption(), Some("infrared"));
        let mut canvas = Canvas::new(Size::new(60, 20), GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut top = 0u8;
        for cell in canvas.cells() {
            for ink in [cell.ink, cell.bg] {
                assert_eq!(ink.role, Role::Ramp(Ramp::Infrared), "{ink:?}");
                top = top.max(ink.level);
            }
        }
        // The coldest tops reach the coloured part of the curve.
        assert!(top > 160, "coldest top at level {top}");
    }

    /// The visible picture at `size`, `t` simulated seconds into the visible act.
    fn visible(size: Size, t: f64) -> (Hurricane, Vec<Ink>) {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(3, size);
        g.resize(size, &mut ctx);
        while g.t < t {
            g.update(SIM_DT, &mut ctx);
        }
        let p = g.pixels.pixel_size();
        let inks = (0..i32::from(p.height()))
            .flat_map(|y| (0..i32::from(p.width())).map(move |x| (x, y)))
            .map(|(x, y)| g.pixels.get(x, y).unwrap_or(Ink::BLANK))
            .collect();
        (g, inks)
    }

    #[test]
    fn hur_a6_the_storm_sits_on_a_blue_ocean_with_sunglint() {
        let (g, inks) = visible(Size::new(120, 40), 2.0);
        let p = g.pixels.pixel_size();
        let w = usize::from(p.width());
        let mut ocean = 0;
        for ink in &inks {
            assert!(
                matches!(ink.role, Role::Ramp(Ramp::Ice | Ramp::Starlight)),
                "{ink:?}"
            );
            ocean += usize::from(ink.role == Role::Ramp(Ramp::Ice));
        }
        // No blank space: the sea fills what the storm leaves.
        assert!(ocean * 5 > inks.len(), "ocean {ocean} of {}", inks.len());
        // The glint: open ocean near the sun's mirror point is brighter than far from it.
        let ph = f64::from(p.height());
        let (gx, gy) = g.glint_centre();
        let (mut near, mut nn, mut far, mut nf) = (0.0, 0, 0.0, 0);
        for (k, ink) in inks.iter().enumerate() {
            if ink.role != Role::Ramp(Ramp::Ice) {
                continue;
            }
            let (x, y) = ((k % w) as f64 / ph, (k / w) as f64 / ph);
            let d = (x - gx).hypot(y - gy);
            if d < 0.12 {
                near += f64::from(ink.level);
                nn += 1;
            } else if d > 0.6 {
                far += f64::from(ink.level);
                nf += 1;
            }
        }
        assert!(nn > 0 && nf > 0, "near {nn}, far {nf}");
        let (near, far) = (near / f64::from(nn), far / f64::from(nf));
        assert!(near > far + 10.0, "glint {near:.0} vs open sea {far:.0}");
    }

    #[test]
    fn hur_a7_no_cloud_pixel_is_darker_than_the_open_sea() {
        // The old picture had a navy rim round every band, which read as a galaxy's dust
        // lane: cloud may only lighten the sea it lies over (the ramps share lightness per
        // level, CORE-R80).
        for size in [Size::new(120, 40), Size::new(80, 24)] {
            let (_, inks) = visible(size, 3.0);
            let floor = inks
                .iter()
                .filter(|i| i.role == Role::Ramp(Ramp::Ice))
                .map(|i| i.level)
                .min()
                .unwrap_or(0);
            for ink in inks
                .iter()
                .filter(|i| i.role == Role::Ramp(Ramp::Starlight))
            {
                assert!(ink.level >= floor, "{size:?}: {ink:?} below sea {floor}");
            }
        }
    }

    #[test]
    fn hur_a8_small_frames_show_few_broad_arms() {
        // At 80×24 the inner bands must not break into thin stripes: walking round the
        // storm through annuli at 2.5, 3.5 and 4.5 radii of maximum wind (mean level in
        // 36 angular bins) meets at most one bright arm per rain band, plus one, counting
        // only swings of more than 20 levels.
        let (g, inks) = visible(Size::new(80, 24), 4.0);
        let p = g.pixels.pixel_size();
        let w = usize::from(p.width());
        let ph = f64::from(p.height());
        let (cx, cy) = g.centre(g.t);
        for rr in [2.5, 3.5, 4.5] {
            let mut bins = [(0.0f64, 0u32); 36];
            for (k, ink) in inks.iter().enumerate() {
                let (dx, dy) = (
                    ((k % w) as f64 + 0.5) / ph - cx,
                    ((k / w) as f64 + 0.5) / ph - cy,
                );
                if (dx.hypot(dy) / R_MAX_WIND - rr).abs() > 0.5 {
                    continue;
                }
                let a = dy.atan2(dx).rem_euclid(TAU);
                let bin = &mut bins[((a / TAU * 36.0) as usize).min(35)];
                bin.0 += f64::from(ink.level);
                bin.1 += 1;
            }
            let seq: Vec<f64> = bins
                .iter()
                .filter(|b| b.1 > 0)
                .map(|b| b.0 / f64::from(b.1))
                .collect();
            // Peaks with hysteresis: rise 20 above the last trough, then fall 20. The
            // walk goes round twice so the start is settled; count the second lap.
            let (mut peaks, mut lo, mut hi, mut rising) = (0, f64::MAX, f64::MIN, true);
            let n = seq.len();
            for (i, &v) in seq.iter().chain(seq.iter()).enumerate() {
                if rising {
                    hi = hi.max(v);
                    if v < hi - 20.0 {
                        peaks += usize::from(i >= n);
                        rising = false;
                        lo = v;
                    }
                } else {
                    lo = lo.min(v);
                    if v > lo + 20.0 {
                        rising = true;
                        hi = v;
                    }
                }
            }
            assert!(peaks <= BANDS + 1, "{peaks} arms at {rr} R_max");
        }
    }

    /// Mean pixel level in 36 angular bins round the annulus `[r0, r1]` (radii of
    /// maximum wind) about the storm centre, over `inks` of pixel size `p`.
    fn ring_bins(g: &Hurricane, inks: &[Ink], r0: f64, r1: f64) -> Vec<f64> {
        let p = g.pixels.pixel_size();
        let w = usize::from(p.width());
        let ph = f64::from(p.height());
        let (cx, cy) = g.centre(g.t);
        let mut bins = [(0.0f64, 0u32); 36];
        for (k, ink) in inks.iter().enumerate() {
            let (dx, dy) = (
                ((k % w) as f64 + 0.5) / ph - cx,
                ((k / w) as f64 + 0.5) / ph - cy,
            );
            let rm = dx.hypot(dy) / R_MAX_WIND;
            if !(r0..=r1).contains(&rm) {
                continue;
            }
            let a = dy.atan2(dx).rem_euclid(TAU);
            let bin = &mut bins[((a / TAU * 36.0) as usize).min(35)];
            bin.0 += f64::from(ink.level);
            bin.1 += 1;
        }
        bins.iter()
            .filter(|b| b.1 > 0)
            .map(|b| b.0 / f64::from(b.1))
            .collect()
    }

    #[test]
    fn hur_a9_a_solid_central_dense_overcast_rings_the_eye() {
        // The eye is ringed by one unbroken bright disc (1.2–2 radii of maximum wind),
        // not by spiral arms with sea between them (which read as a galaxy's).
        for t in [2.0, 9.0] {
            let (g, inks) = visible(Size::new(120, 40), t);
            let bins = ring_bins(&g, &inks, 1.2, 2.0);
            let lo = bins.iter().copied().fold(f64::MAX, f64::min);
            let hi = bins.iter().copied().fold(f64::MIN, f64::max);
            assert!(bins.len() >= 30, "{} bins", bins.len());
            assert!(
                lo >= 150.0 && lo >= 0.75 * hi,
                "t {t}: overcast {lo:.0}..{hi:.0}"
            );
        }
    }

    #[test]
    fn hur_a10_the_coldest_tops_are_in_the_overcast_not_the_bands() {
        let mut g = storm();
        let mut ctx = Ctx::new(1, Size::new(120, 40));
        g.resize(Size::new(120, 40), &mut ctx);
        let mid_ir = ACTS[0].1 + 0.5 * ACTS[1].1;
        while g.t < mid_ir {
            g.update(0.25, &mut ctx);
        }
        let p = g.pixels.pixel_size();
        let inks: Vec<Ink> = (0..i32::from(p.height()))
            .flat_map(|y| (0..i32::from(p.width())).map(move |x| (x, y)))
            .map(|(x, y)| g.pixels.get(x, y).unwrap_or(Ink::BLANK))
            .collect();
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let core = mean(&ring_bins(&g, &inks, 1.2, 2.0));
        // The brightest band bins in the outer storm.
        let mut bands = ring_bins(&g, &inks, 4.5, 6.0);
        bands.sort_by(f64::total_cmp);
        let band_top = mean(&bands[bands.len() * 3 / 4..]);
        assert!(
            core > band_top + 100.0,
            "overcast {core:.0} vs bands {band_top:.0}"
        );
    }

    #[test]
    fn hur_a5_lattice_stays_within_its_cap() {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(1, Size::new(500, 150));
        g.resize(Size::new(500, 150), &mut ctx);
        assert!(g.lattice.area() <= CAP, "{:?}", g.lattice);
        assert_eq!(g.cloud.size(), g.lattice);
    }
}
