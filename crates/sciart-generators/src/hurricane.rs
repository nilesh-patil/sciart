//! A mature hurricane seen from above (SPEC-002 `hurricane`).
//!
//! Cloud water rides a kinematic vortex on a capped lattice (`vortex_advect`,
//! semi-Lagrangian): a Holland-profile tangential wind, counter-clockwise as in the
//! northern hemisphere, plus a low-level inflow that stops at the eyewall and so draws
//! the bands in as logarithmic spirals. Convection feeds the eyewall and three spiral rain
//! bands (pulsing, noise-modulated cells), subsidence clears the eye, and evaporation
//! clears the ocean between the bands. A `Timeline` alternates the visible picture with an
//! enhanced-infrared one, in which the coldest (highest) cloud tops stand out.
//!
//! M16 (thread style, GEN-R25): the cloud-top brightness is drawn as contour threads,
//! spiral band edges winding into a ring round the eye, coloured by radius (visible) or
//! by cloud-top coldness (infrared); tracer dots with short trails ride the inflow.
//!
//! Units: lengths in lattice heights (`H`), speeds in `H` per simulated second, so the
//! storm keeps its shape when the lattice is regridded (ADR-0012).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::Grid;
use sciart_core::grid;
use sciart_core::prelude::*;

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
/// Brightness below which the sea is clear.
const CLEAR: f64 = 0.012;
/// Radial wind table resolution over `[0, 2·R_STORM]`.
const TABLE: usize = 512;
/// Steps run in the first resize, so the first frame is a mature storm.
const SPIN_UP: usize = 180;
/// Lattice cap for the spin-up (GEN-R19).
const SPIN_UP_CAP: usize = CAP / 24;
/// Seconds of the cross-fade between the visible and infrared pictures: a dip through
/// black, the one fading out while the IR weight rises to ½, the other fading in after.
const DISSOLVE: f64 = 1.5;

/// Lattices at most this tall (80×24 and smaller) are too coarse for the tight inner
/// bands: the secondary band drops out and the brightness is blurred twice (HUR-A8).
const SMALL: u16 = 64;
/// Steps between cirrus refreshes (0.1 s).
const SWELL_EVERY: u32 = 6;
/// The enhanced-infrared curve (HUR-A4): where the cloud-top curve starts from.
const IR_LOW: f64 = 0.16;
/// Cloud-top height factor in the IR act: 1 inside `IR_CORE` (`H`, the overcast), falling
/// linearly to `IR_BANDS` at `IR_OUTER`.
const IR_CORE: f64 = CDO_OUT * R_MAX_WIND;
const IR_OUTER: f64 = 0.9 * R_STORM;
const IR_BANDS: f64 = 0.62;

const ACTS: [(&str, f64); 2] = [("visible", 25.0), ("infrared", 12.0)];
const IR: usize = 1;

/// M16 threads (GEN-R25): contour levels of the cloud-top brightness and their ink
/// levels (visible act), the radii (in radii of maximum wind) inside which threads are
/// `Star` (eyewall) and `Wave` (inner bands), and the sub-cell refinement.
/// (The lowest is 0.3: below it the band cells' noise, near the lattice scale, only
/// frays into specks.)
pub(crate) const LEVELS: [f32; 3] = [0.3, 0.5, 0.7];
const VIS_LEVELS: [u8; 3] = [150, 200, 245];
const EYEWALL_RM: f64 = 1.8;
/// Inside `EYEWALL_RM` only these contour levels are drawn (two eyewall rings, M16
/// round 2); outside it every band is one thread along its ridge (its centre line).
const EYE_LEVELS: [usize; 2] = [1, 2];
/// Band ridges in cloud dimmer than this are noise between cells, not bands.
const RIDGE_MIN: f32 = 0.4;
const INNER_RM: f64 = 3.2;
const REFINE: f64 = 2.2;
const MAX_REFINE: usize = 5;
/// Cross-fade weights below which no threads are drawn, and below which one step dimmer.
const FADE_OFF: f64 = 0.25;
const FADE_DIM: f64 = 0.6;
/// Inflow tracers: how many, the trail (positions kept, steps between them), their
/// life (s) and where they respawn (in `R_STORM`).
const TRACERS: usize = 90;
const TRAIL: usize = 12;
const TRAIL_EVERY: u32 = 2;
const TRACER_LIFE: (f64, f64) = (12.0, 20.0);
const RESPAWN: (f64, f64) = (0.5, 1.0);
const HEAD: Ink = Ink::new(Role::Star, 255);
const TAIL: [Ink; 2] = [Ink::new(Role::Cold, 150), Ink::new(Role::Cold, 110)];

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

/// A visible-act thread's ink: by radius `rm` (radii of maximum wind), stepped by its
/// contour level `level` (index into [`LEVELS`]).
pub(crate) fn vis_thread(rm: f64, level: usize) -> Ink {
    let role = if rm < EYEWALL_RM {
        Role::Star
    } else if rm < INNER_RM {
        Role::Wave
    } else {
        Role::Cold
    };
    Ink::new(role, VIS_LEVELS[level.min(VIS_LEVELS.len() - 1)])
}

/// An infrared thread's ink by the coldness `cold` (0 … 1) of its cloud tops (HUR-A4).
pub(crate) fn ir_thread(cold: f64) -> Ink {
    match cold {
        c if c < 0.4 => Ink::new(Role::Cold, 170),
        c if c < 0.7 => Ink::new(Role::Wave, 210),
        _ => Ink::new(Role::Accent, 245),
    }
}

/// The enhancement curve's position for cloud-top brightness `b` (before the height
/// factor): thin low cloud stays low, the eyewall and convective cores reach the top.
fn ir_curve(b: f64) -> f64 {
    let x = ((b - CLEAR) / (0.78 - CLEAR)).clamp(0.0, 1.0);
    (IR_LOW + (1.0 - IR_LOW) * x.powf(1.25) - IR_LOW) / (1.0 - IR_LOW)
}

/// Cloud-top height factor at radius `r` (`H`) from the storm centre (HUR-A10).
fn height_factor(r: f64) -> f64 {
    1.0 - (1.0 - IR_BANDS) * ((r - IR_CORE) / (IR_OUTER - IR_CORE)).clamp(0.0, 1.0)
}

/// One inflow tracer, storm-relative (`H`): its position, the trail (newest first, `n`
/// of them valid), its age and life.
#[derive(Debug, Clone, Copy)]
struct Tracer {
    x: f64,
    y: f64,
    trail: [(f32, f32); TRAIL],
    n: usize,
    age: f64,
    life: f64,
}

impl Tracer {
    fn spawn(rng: &mut impl Rng) -> Self {
        let r = R_STORM * rng.random_range(RESPAWN.0..RESPAWN.1);
        let (s, c) = rng.random_range(0.0..TAU).sin_cos_det();
        Self {
            x: r * c,
            y: r * s,
            trail: [(0.0, 0.0); TRAIL],
            n: 0,
            age: 0.0,
            life: rng.random_range(TRACER_LIFE.0..TRACER_LIFE.1),
        }
    }
}

#[derive(Debug)]
pub struct Hurricane {
    t: f64,
    timeline: Timeline,
    lattice: Size,
    cloud: Grid<f32>,
    /// Cloud-top brightness `1 − e^{−1.5·c}` per node.
    bright: Grid<f32>,
    /// Cirrus outflow brightness per node, refreshed with the swell.
    cirrus: Grid<f32>,
    /// Steps since the swell was refreshed.
    swell_age: u32,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
    /// A coarse lattice (`height ≤ SMALL`): smoothed picture, fewer inner bands.
    small: bool,
    scratch: Grid<f32>,
    flow: Flow,
    /// `(tangential, inflow)` wind over `[0, 2·R_STORM]`, `H`/s.
    table: Vec<(f32, f32)>,
    /// M16: the inflow tracers, steps since their trails moved on, and the threads.
    tracers: Vec<Tracer>,
    trail_age: u32,
    /// Radial derivative of the brightness per node (M16 round 2): a band's centre line
    /// is where it crosses zero going downhill outward.
    ridge: Vec<f32>,
    dots: Dots,
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
            cirrus: Grid::default(),
            swell_age: SWELL_EVERY,
            dirty: false,
            small: false,
            scratch: Grid::default(),
            flow: Flow::default(),
            table: (0..TABLE)
                .map(|k| {
                    let r = 2.0 * R_STORM * k as f64 / (TABLE - 1) as f64;
                    (holland(r) as f32, inflow(r) as f32)
                })
                .collect(),
            tracers: Vec::with_capacity(TRACERS),
            trail_age: 0,
            ridge: Vec::new(),
            dots: Dots::default(),
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

    /// The cirrus and the cloud-top brightness for the current cloud, as a step leaves
    /// them: after a regrid the picture is right before the next step.
    fn refresh_bright(&mut self) {
        self.fill_swell();
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
        box_blur(&mut self.bright, &mut self.scratch);
        if self.small {
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
        self.ridge.clear();
        self.ridge.resize(lattice.area(), 0.0);
        self.cirrus.resize(lattice, 0.0);
        // A regridded lattice needs its cirrus before the next picture.
        self.swell_age = SWELL_EVERY;
        self.flow.resize(lattice);
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

    /// The cirrus outflow per node at the current time, in `H` so it keeps its scale when
    /// the lattice changes.
    fn fill_swell(&mut self) {
        self.swell_age = 0;
        let w = usize::from(self.lattice.width()).max(1);
        let scale = f64::from(self.lattice.height().max(1));
        let t = self.t;
        let (cx, cy) = self.centre(t);
        let cot = 1.0 / OUTFLOW_PITCH.tan();
        for (k, ci) in self.cirrus.as_mut_slice().iter_mut().enumerate() {
            let (x, y) = ((k % w) as f64 / scale, (k / w) as f64 / scale);
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

    /// Carry the tracers along the low-level wind (midpoint step), keep their trails and
    /// respawn the spent ones (HUR-A11).
    fn move_tracers(&mut self, dt: f64, rng: &mut impl Rng) {
        self.trail_age += 1;
        let push = self.trail_age >= TRAIL_EVERY;
        if push {
            self.trail_age = 0;
        }
        let table = &self.table;
        let wind = |x: f64, y: f64| {
            let (vt, vr) = Self::wind_at(table, x.hypot(y));
            wind_from(x, y, vt, vr)
        };
        for p in &mut self.tracers {
            if push {
                p.trail.copy_within(0..TRAIL - 1, 1);
                p.trail[0] = (p.x as f32, p.y as f32);
                p.n = (p.n + 1).min(TRAIL);
            }
            let (u, v) = wind(p.x, p.y);
            let (u, v) = wind(p.x + 0.5 * dt * u, p.y + 0.5 * dt * v);
            p.x += dt * u;
            p.y += dt * v;
            p.age += dt;
            let r = p.x.hypot(p.y);
            if !(EYE * R_MAX_WIND..=1.1 * R_STORM).contains(&r) || p.age >= p.life {
                *p = Tracer::spawn(rng);
            }
        }
    }

    /// The radial derivative of the brightness at every node (central differences,
    /// one-sided at the lattice's edges), for the bands' ridge lines.
    fn fill_ridge(&mut self) {
        let (w, h) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        if self.ridge.len() != w * h || w < 2 || h < 2 {
            return;
        }
        let scale = h as f64;
        let (cx, cy) = self.centre(self.t);
        let b = self.bright.as_slice();
        for j in 0..h {
            let (ju, jd) = (j.saturating_sub(1), (j + 1).min(h - 1));
            for i in 0..w {
                let (il, ir) = (i.saturating_sub(1), (i + 1).min(w - 1));
                let gx = (b[j * w + ir] - b[j * w + il]) / (ir - il) as f32;
                let gy = (b[jd * w + i] - b[ju * w + i]) / (jd - ju) as f32;
                let (ex, ey) = (i as f64 / scale - cx, j as f64 / scale - cy);
                let r = ex.hypot(ey).max(1e-9);
                self.ridge[j * w + i] = (f64::from(gx) * ex / r + f64::from(gy) * ey / r) as f32;
            }
        }
    }

    /// Draw the frame: the eyewall rings and band ridge threads, then the tracers. The faint cirrus
    /// outflow (below the lowest contour) is not drawn: as threads it only hatched the
    /// storm's flanks.
    fn raster(&mut self) {
        self.dots.clear();
        let ir = self.ir_weight();
        let (use_ir, gain) = if ir >= 0.5 {
            (true, 2.0 * ir - 1.0)
        } else {
            (false, 1.0 - 2.0 * ir)
        };
        let (vw, vh) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        let ds = self.dots.dot_size();
        if vw < 2 || vh < 2 || self.bright.size() != self.lattice {
            return;
        }
        let (sx, sy) = (
            f64::from(ds.width()) / vw as f64,
            f64::from(ds.height()) / vh as f64,
        );
        // Node (x, y) → dot coordinates, the cell-centre convention (CORE-R68).
        let to_dot = move |x: f64, y: f64| ((x + 0.5) * sx - 0.5, (y + 0.5) * sy - 0.5);
        let k = ((sx.max(sy) / REFINE).ceil() as usize).clamp(1, MAX_REFINE);
        let scale = vh as f64;
        let (cx, cy) = self.centre(self.t);
        let dim = |ink: Ink| {
            if gain < FADE_DIM {
                Ink::new(ink.role, (f64::from(ink.level) * 0.7) as u8)
            } else {
                ink
            }
        };
        if gain >= FADE_OFF {
            self.fill_ridge();
            // The cold of each contour level before the height factor (IR act).
            let colds = LEVELS.map(|l| ir_curve(f64::from(l)));
            let bright = self.bright.as_slice();
            let ridge = self.ridge.as_slice();
            let dots = &mut self.dots;
            for j in 0..vh - 1 {
                for i in 0..vw - 1 {
                    let at = |g: &[f32]| {
                        let n = j * vw + i;
                        [g[n], g[n + 1], g[n + vw + 1], g[n + vw]]
                    };
                    let c = at(bright);
                    let hi = c[0].max(c[1]).max(c[2].max(c[3]));
                    let lo = c[0].min(c[1]).min(c[2].min(c[3]));
                    // Radius of the cell's centre from the storm's, `H`.
                    let (ex, ey) = ((i as f64 + 0.5) / scale - cx, (j as f64 + 0.5) / scale - cy);
                    let r = ex.hypot(ey);
                    if hi < LEVELS[0] {
                        continue;
                    }
                    if r / R_MAX_WIND < EYEWALL_RM {
                        // The eyewall: two rings.
                        for &n in &EYE_LEVELS {
                            let level = LEVELS[n];
                            if lo < level && level <= hi {
                                let ink = if use_ir {
                                    ir_thread(colds[n] * height_factor(r))
                                } else {
                                    vis_thread(r / R_MAX_WIND, n)
                                };
                                dots.contour_cell_refined(c, level, (i, j), k, to_dot, dim(ink));
                            }
                        }
                        continue;
                    }
                    // A band: one thread on its ridge, where the radial derivative falls
                    // through zero outward, in cloud at least as bright as the lowest level.
                    let mean = 0.25 * (c[0] + c[1] + c[2] + c[3]);
                    if mean < RIDGE_MIN {
                        continue;
                    }
                    let d = at(ridge);
                    let (dlo, dhi) = (
                        d[0].min(d[1]).min(d[2].min(d[3])),
                        d[0].max(d[1]).max(d[2].max(d[3])),
                    );
                    if !(dlo < 0.0 && 0.0 <= dhi) {
                        continue;
                    }
                    let (ux, uy) = (ex / r.max(1e-9), ey / r.max(1e-9));
                    let slope = 0.5 * f64::from((d[1] + d[2]) - (d[0] + d[3])) * ux
                        + 0.5 * f64::from((d[2] + d[3]) - (d[0] + d[1])) * uy;
                    if slope >= 0.0 {
                        continue; // a trough between bands, not a ridge
                    }
                    let n = LEVELS.iter().rposition(|&l| mean >= l).unwrap_or(0);
                    let ink = if use_ir {
                        ir_thread(ir_curve(f64::from(mean)) * height_factor(r))
                    } else {
                        vis_thread(r / R_MAX_WIND, n)
                    };
                    dots.contour_cell_refined(d, 0.0, (i, j), k, to_dot, dim(ink));
                }
            }
        }
        // The tracers: a trail of their last positions, then the head.
        let node = |x: f64, y: f64| to_dot((cx + x) * scale, (cy + y) * scale);
        for p in &self.tracers {
            let mut prev = node(p.x, p.y);
            for (m, &(tx, ty)) in p.trail[..p.n].iter().enumerate() {
                let q = node(f64::from(tx), f64::from(ty));
                let ink = TAIL[usize::from(2 * m >= TRAIL)];
                self.dots.line(prev.0, prev.1, q.0, q.1, ink);
                prev = q;
            }
        }
        for p in &self.tracers {
            let (x, y) = node(p.x, p.y);
            self.dots.plot(x, y, HEAD);
        }
    }
}

impl Generator for Hurricane {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let display = Size::new(size.width(), size.height().saturating_mul(2).max(1));
        let lattice = grid::lattice_for(display, CAP, BUCKET);
        if !self.warmed {
            // GEN-R19: the spin-up runs on a coarse lattice (a 24th of the nodes) and is
            // regridded up: the storm settles into the fine lattice over its first frames.
            self.warmed = true;
            self.rebuild_lattice(grid::lattice_for(display, SPIN_UP_CAP, BUCKET));
            for _ in 0..SPIN_UP {
                self.step(SIM_DT);
            }
            self.t = 0.0;
            self.tracers.clear();
            for _ in 0..TRACERS {
                let mut p = Tracer::spawn(&mut ctx.rng);
                p.age = ctx.rng.random_range(0.0..p.life);
                self.tracers.push(p);
            }
        }
        self.rebuild_lattice(lattice);
        self.refresh_bright();
        self.dots.resize(size);
        if self.swell_age >= SWELL_EVERY {
            self.fill_swell();
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt);
            self.move_tracers(dt, &mut ctx.rng);
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

    /// The scene at `size`, run to `t` simulated seconds, with its canvas.
    fn run(size: Size, t: f64, dt: f64) -> (Hurricane, Canvas) {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(3, size);
        g.resize(size, &mut ctx);
        g.prepare();
        while g.t < t {
            g.update(dt, &mut ctx);
            g.prepare();
        }
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        (g, c)
    }

    fn storm() -> Hurricane {
        run(Size::new(120, 40), 2.0, SIM_DT).0
    }

    /// Radius (in radii of maximum wind) and screen angle of cell `(x, y)`'s centre.
    fn polar_of(g: &Hurricane, size: Size, x: i32, y: i32) -> (f64, f64) {
        let ds = g.dots.dot_size();
        let (vw, vh) = (f64::from(g.lattice.width()), f64::from(g.lattice.height()));
        // Cell centre → dots → lattice nodes → `H`.
        let (dx, dy) = (f64::from(x) * 2.0 + 0.5, f64::from(y) * 4.0 + 1.5);
        let (nx, ny) = (
            (dx + 0.5) * vw / f64::from(ds.width()) - 0.5,
            (dy + 0.5) * vh / f64::from(ds.height()) - 0.5,
        );
        let _ = size;
        let (cx, cy) = g.centre(g.t);
        let (hx, hy) = (nx / vh - cx, ny / vh - cy);
        (hx.hypot(hy) / R_MAX_WIND, hy.atan2(hx).rem_euclid(TAU))
    }

    fn is_braille(c: char) -> bool {
        ('\u{2800}'..='\u{28ff}').contains(&c)
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

    /// The scene in the middle of the IR act at `size`.
    fn infrared(size: Size) -> (Hurricane, Canvas) {
        run(size, ACTS[0].1 + 0.5 * ACTS[1].1, 0.25)
    }

    #[test]
    fn hur_a4_the_infrared_act_uses_the_stepped_curve() {
        let size = Size::new(60, 20);
        let (g, c) = infrared(size);
        assert_eq!(g.caption(), Some("infrared"));
        let steps: Vec<Ink> = [0.0, 0.5, 0.9].into_iter().map(ir_thread).collect();
        let mut accent = false;
        for cell in c.cells() {
            if cell.ch == ' ' || cell.ink == HEAD || TAIL.contains(&cell.ink) {
                continue;
            }
            assert!(steps.contains(&cell.ink), "{:?}", cell.ink);
            accent |= cell.ink.role == Role::Accent;
        }
        assert!(accent, "no coldest tops");
    }

    #[test]
    fn hur_a4_the_curve_runs_cold_wave_accent() {
        let roles: Vec<Role> = [0.0, 0.5, 0.9]
            .into_iter()
            .map(|c| ir_thread(c).role)
            .collect();
        assert_eq!(roles, [Role::Cold, Role::Wave, Role::Accent]);
        // The curve rises with brightness.
        let mut last = -1.0;
        for k in 0..=100 {
            let v = ir_curve(f64::from(k) / 100.0);
            assert!(v >= last);
            last = v;
        }
    }

    #[test]
    fn hur_a6_a7_the_storm_is_threads_on_a_black_sea() {
        for size in [Size::new(120, 40), Size::new(80, 24)] {
            let (_, c) = run(size, 3.0, SIM_DT);
            let (mut lit, mut dots) = (0usize, 0u32);
            for cell in c.cells() {
                if cell.ch == ' ' {
                    continue;
                }
                assert!(is_braille(cell.ch), "{size:?}: {:?}", cell.ch);
                assert!(
                    matches!(cell.ink.role, Role::Cold | Role::Wave | Role::Star),
                    "{size:?}: {:?}",
                    cell.ink
                );
                lit += 1;
                dots += (cell.ch as u32 - 0x2800).count_ones();
            }
            let n = c.cells().len();
            assert!(lit * 2 < n, "{size:?}: {lit} of {n} cells lit");
            let mean = f64::from(dots) / lit.max(1) as f64;
            assert!(mean <= 4.5, "{size:?}: {mean:.2} dots per lit cell");
        }
    }

    /// Mean lattice brightness in 36 angular bins round the annulus `[r0, r1]` (radii of
    /// maximum wind) about the storm centre.
    fn ring_bins(g: &Hurricane, r0: f64, r1: f64) -> Vec<f64> {
        let (w, h) = (
            usize::from(g.lattice.width()),
            usize::from(g.lattice.height()),
        );
        let (cx, cy) = g.centre(g.t);
        let mut bins = [(0.0f64, 0u32); 36];
        for (k, &b) in g.bright.as_slice().iter().enumerate() {
            let (dx, dy) = (
                (k % w) as f64 / h as f64 - cx,
                (k / w) as f64 / h as f64 - cy,
            );
            let rm = dx.hypot(dy) / R_MAX_WIND;
            if !(r0..=r1).contains(&rm) {
                continue;
            }
            let a = dy.atan2(dx).rem_euclid(TAU);
            let bin = &mut bins[((a / TAU * 36.0) as usize).min(35)];
            bin.0 += f64::from(b);
            bin.1 += 1;
        }
        bins.iter()
            .filter(|b| b.1 > 0)
            .map(|b| b.0 / f64::from(b.1))
            .collect()
    }

    #[test]
    fn hur_a8_small_frames_show_few_broad_arms() {
        // At 80×24 the inner bands must not break into thin stripes: walking round the
        // storm through annuli at 2.5, 3.5 and 4.5 radii of maximum wind meets at most one
        // bright arm per rain band, plus one, counting only swings of more than 0.08.
        let (g, _) = run(Size::new(80, 24), 4.0, SIM_DT);
        for rr in [2.5, 3.5, 4.5] {
            let seq = ring_bins(&g, rr - 0.5, rr + 0.5);
            let (mut peaks, mut lo, mut hi, mut rising) = (0, f64::MAX, f64::MIN, true);
            let n = seq.len();
            for (i, &v) in seq.iter().chain(seq.iter()).enumerate() {
                if rising {
                    hi = hi.max(v);
                    if v < hi - 0.08 {
                        peaks += usize::from(i >= n);
                        rising = false;
                        lo = v;
                    }
                } else {
                    lo = lo.min(v);
                    if v > lo + 0.08 {
                        rising = true;
                        hi = v;
                    }
                }
            }
            assert!(peaks <= BANDS + 1, "{peaks} arms at {rr} R_max");
        }
    }

    #[test]
    fn hur_a9_a_solid_overcast_ringed_by_star_threads() {
        let size = Size::new(120, 40);
        for t in [2.0, 9.0] {
            let (g, c) = run(size, t, SIM_DT);
            let bins = ring_bins(&g, 1.2, 2.0);
            let lo = bins.iter().copied().fold(f64::MAX, f64::min);
            let hi = bins.iter().copied().fold(f64::MIN, f64::max);
            assert!(bins.len() >= 30, "{} bins", bins.len());
            assert!(
                lo >= f64::from(LEVELS[2]) && lo >= 0.75 * hi,
                "t {t}: overcast {lo:.2}..{hi:.2}"
            );
            let mut ring = [false; 12];
            for y in 0..40 {
                for x in 0..120 {
                    let cell = c.get(x, y).unwrap();
                    if cell.ch == ' ' || cell.ink.role != Role::Star || cell.ink == HEAD {
                        continue;
                    }
                    let (rm, a) = polar_of(&g, size, x, y);
                    if (0.5..=2.0).contains(&rm) {
                        ring[((a / TAU * 12.0) as usize).min(11)] = true;
                    }
                }
            }
            assert!(ring.iter().all(|&s| s), "t {t}: eyewall ring {ring:?}");
        }
    }

    #[test]
    fn hur_a10_the_coldest_tops_are_in_the_overcast_not_the_bands() {
        let size = Size::new(120, 40);
        let (g, c) = infrared(size);
        let (mut wall, mut wall_accent, mut bands, mut band_accent) = (0, 0, 0, 0);
        for y in 0..40 {
            for x in 0..120 {
                let cell = c.get(x, y).unwrap();
                if cell.ch == ' ' || cell.ink == HEAD || TAIL.contains(&cell.ink) {
                    continue;
                }
                let accent = usize::from(cell.ink.role == Role::Accent);
                match polar_of(&g, size, x, y).0 {
                    rm if rm < 3.0 => {
                        wall += 1;
                        wall_accent += accent;
                    }
                    rm if (4.5..=6.0).contains(&rm) => {
                        bands += 1;
                        band_accent += accent;
                    }
                    _ => {}
                }
            }
        }
        // Inside the eye the low contours are thin, warm cloud; the overcast's top
        // contour is the coldest.
        assert!(wall > 0 && wall_accent > 0, "{wall_accent} of {wall}");
        assert!(
            bands > 0 && band_accent * 4 < bands,
            "{band_accent} of {bands}"
        );
    }

    #[test]
    fn hur_a11_tracers_spiral_inward() {
        let size = Size::new(120, 40);
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        let before: Vec<(f64, f64, f64)> = g.tracers.iter().map(|p| (p.x, p.y, p.age)).collect();
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            // Each trail point is a position the tracer held in the last 0.4 s.
            for p in &g.tracers {
                for &(tx, ty) in &p.trail[..p.n] {
                    let back = (f64::from(tx)).hypot(f64::from(ty)) - p.x.hypot(p.y);
                    assert!(back > -0.01, "a trail point inside its head's radius");
                }
            }
        }
        let (mut n, mut dr, mut dth) = (0, 0.0, 0.0);
        for (p, &(x, y, age)) in g.tracers.iter().zip(&before) {
            if p.age < age {
                continue; // respawned
            }
            n += 1;
            dr += p.x.hypot(p.y) - x.hypot(y);
            let mut d = (-p.y).atan2(p.x) - (-y).atan2(x);
            d -= TAU * (d / TAU).round();
            dth += d;
        }
        assert!(n > TRACERS / 2, "{n} tracers kept");
        assert!(dr < 0.0 && dth > 0.0, "Δr {dr:.4} Δθ {dth:.3}");
        // Trails hold at most 0.4 s of history.
        assert!(f64::from(TRAIL as u32 * TRAIL_EVERY) * SIM_DT <= 0.4 + 1e-9);
    }

    #[test]
    fn hur_a5_lattice_stays_within_its_cap() {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(1, Size::new(500, 150));
        g.resize(Size::new(500, 150), &mut ctx);
        g.prepare();
        assert!(g.lattice.area() <= CAP, "{:?}", g.lattice);
        assert_eq!(g.cloud.size(), g.lattice);
    }
}
