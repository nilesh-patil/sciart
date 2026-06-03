//! A two-stage night launch from the pad to the edge of space (SPEC-002 `rocket_launch`).
//!
//! The vehicle is Falcon 9–class (approximate public figures): a 2-D point mass over a
//! flat Earth with inverse-square gravity and the centrifugal relief of horizontal speed,
//! an exponential atmosphere, Mach-dependent drag and sea-level-to-vacuum thrust
//! (ROC-A1). It climbs vertically, pitches over by a few degrees and then flies a zero
//! angle-of-attack gravity turn (ROC-A3), through max-Q (ROC-A4) to first-stage cutoff,
//! separation and second-stage ignition. The ascent is integrated once, in `build`, with
//! RK4; the scene replays it on a time-lapse clock (1× on the pad, up to ~8× in the
//! climb), so every frame is a lookup.
//!
//! The camera pulls back with altitude so the whole arc stays in frame. The look is the
//! M11 night launch (ADR-0017): a black sky with sparse stars, the horizon a 1-px dim
//! line; the vehicle a thin pale-grey braille silhouette; the plume its `Blackbody` core
//! only, widening as the air thins, with white-hot shock diamonds low down (the accent);
//! the smoke trail and pad steam sparse dim braille that spreads and thins as it ages
//! (ROC-A5, ROC-A6). The Δv ledger (ROC-A7) is kept in the model. Telemetry is dim grey
//! text on row 0 (ROC-A9).

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::ink::{self, blackbody};
use sciart_core::ode::rk4_step;
use sciart_core::particles::gaussian2;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Tone, draw, ease, raster};

pub static META: Meta = Meta {
    name: "rocket_launch",
    category: Category::Voyages,
    description: "A two-stage gravity-turn launch from the pad through max-Q and staging",
    title: "Rocket launch · stage drops away",
    captions: &ACTS,
};

/// The title's caption per act (GEN-R12); the countdown shows the title's own mechanism.
const ACTS: [&str; 5] = [
    "liftoff",
    "gravity turn",
    "max-Q",
    "stage separation",
    "second stage",
];

pub fn build() -> Box<dyn Generator> {
    Box::new(RocketLaunch::new())
}

// ---------------------------------------------------------------------------------------
// Earth, air and vehicle (SI units).
// ---------------------------------------------------------------------------------------

const G0: f64 = 9.806_65;
const RE: f64 = 6_371_000.0;
/// Sea-level air density (kg/m³) and the scale height of an isothermal atmosphere (m);
/// pressure falls with the same scale height.
const RHO0: f64 = 1.225;
const SCALE_H: f64 = 8_500.0;
/// Speed of sound (m/s), held at its sea-level value (stylised).
const SOUND: f64 = 340.0;
/// Frontal area of a 3.7 m core (m²).
const AREA: f64 = 10.75;

/// Stage 1: nine sea-level engines. Thrust (N), vacuum Isp (s), propellant and dry mass (kg).
const T1_SL: f64 = 7_607e3;
const T1_VAC: f64 = 8_227e3;
const ISP1_VAC: f64 = 311.0;
const PROP1: f64 = 411_000.0;
const DRY1: f64 = 25_600.0;
/// Stage 2 (one vacuum engine) plus payload.
const T2: f64 = 981e3;
const ISP2: f64 = 348.0;
const PROP2: f64 = 107_500.0;
const DRY2: f64 = 4_000.0;
const PAYLOAD: f64 = 15_000.0;
const M_LIFTOFF: f64 = PROP1 + DRY1 + PROP2 + DRY2 + PAYLOAD;
const M_UPPER: f64 = PROP2 + DRY2 + PAYLOAD;

const MDOT1: f64 = T1_VAC / (ISP1_VAC * G0);
const MDOT2: f64 = T2 / (ISP2 * G0);
/// Main-engine cutoff when stage 1 runs dry, separation 3 s later, second-stage ignition
/// 8 s after cutoff (Falcon 9's own rhythm).
const MECO: f64 = PROP1 / MDOT1;
const SEP: f64 = MECO + 3.0;
const SES: f64 = MECO + 8.0;
/// The replay ends this long into the second-stage burn.
const MET_END: f64 = SES + 56.0;

/// Pitch program: vertical rise, then a small kick over `KICK_S`, then the thrust
/// follows the velocity (zero angle of attack).
const KICK_AT: f64 = 9.0;
const KICK_S: f64 = 4.0;
const KICK: f64 = 3.2 * PI / 180.0;

/// Air density (kg/m³) at altitude `h` (m).
pub(crate) fn air_density(h: f64) -> f64 {
    RHO0 * (-h.max(0.0) / SCALE_H).exp()
}

/// Ambient pressure over its sea-level value.
pub(crate) fn pressure_ratio(h: f64) -> f64 {
    (-h.max(0.0) / SCALE_H).exp()
}

/// Drag coefficient against Mach number: a transonic hump on a slender-body floor.
fn drag_coefficient(mach: f64) -> f64 {
    0.26 + 0.28 * (-((mach - 1.15) / 0.45).powi(2)).exp()
}

/// Which engine burns at mission elapsed time `met` (s): thrust (N) at altitude `h` and
/// mass flow (kg/s).
fn engine(met: f64, h: f64) -> (f64, f64) {
    if (0.0..MECO).contains(&met) {
        // Constant mass flow; back-pressure on the nozzle exits costs sea-level thrust.
        (T1_VAC - (T1_VAC - T1_SL) * pressure_ratio(h), MDOT1)
    } else if (SES..).contains(&met) {
        (T2, MDOT2)
    } else {
        (0.0, 0.0)
    }
}

/// Thrust direction from vertical at `met` for a velocity `(vx, vh)`.
fn pitch(met: f64, vx: f64, vh: f64) -> f64 {
    if met < KICK_AT {
        0.0
    } else if met < KICK_AT + KICK_S {
        KICK * ease::smoothstep((met - KICK_AT) / KICK_S)
    } else {
        vx.atan2(vh).max(0.0)
    }
}

/// Integrated state: downrange `x` and altitude `h` (m), velocity, mass (kg), and the
/// running Δv ledger (m/s): ideal `∫T/m`, gravity, drag and steering losses (ROC-A7).
pub(crate) const X: usize = 0;
pub(crate) const H: usize = 1;
pub(crate) const VX: usize = 2;
pub(crate) const VH: usize = 3;
pub(crate) const M: usize = 4;
#[cfg(test)]
pub(crate) const IDEAL: usize = 5;
#[cfg(test)]
pub(crate) const GRAV: usize = 6;
#[cfg(test)]
pub(crate) const DRAG: usize = 7;
#[cfg(test)]
pub(crate) const STEER: usize = 8;
pub(crate) type State = [f64; 9];

/// Right-hand side of the ascent at `met` (ROC-A1): thrust along the pitch, drag against
/// the velocity, gravity `g0 (Re/(Re+h))²` less the centrifugal `vx²/(Re+h)`.
pub(crate) fn derivs(met: f64, y: &State, drag_on: bool) -> State {
    let (vx, vh, m) = (y[VX], y[VH], y[M].max(1.0));
    let (thrust, mdot) = engine(met, y[H]);
    let r = RE + y[H];
    let g = G0 * (RE / r).powi(2) - vx * vx / r;
    let v = vx.hypot(vh);
    let (ux, uh) = if v > 1e-6 {
        (vx / v, vh / v)
    } else {
        (0.0, 1.0)
    };
    let drag = if drag_on {
        0.5 * air_density(y[H]) * v * v * drag_coefficient(v / SOUND) * AREA / m
    } else {
        0.0
    };
    let th = pitch(met, vx, vh);
    let a = thrust / m;
    let steer = a * (1.0 - (th - ux.atan2(uh)).cos_det());
    [
        vx,
        vh,
        a * th.sin_det() - drag * ux,
        a * th.cos_det() - drag * uh - g,
        -mdot,
        a,
        g * uh,
        drag,
        if v > 1e-6 { steer } else { 0.0 },
    ]
}

/// Replay samples per second of mission time.
const RATE: f64 = 10.0;
/// Integration substeps per sample.
const SUB: usize = 4;

/// One replay sample: the vehicle state and dynamic pressure `q` (Pa).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Sample {
    pub y: State,
    pub q: f64,
}

/// The whole ascent from liftoff to `MET_END`, sampled at `RATE` Hz, plus the spent
/// first stage's fall after separation.
#[derive(Debug, Clone)]
pub(crate) struct Ascent {
    pub samples: Vec<Sample>,
    pub booster: Vec<[f64; 2]>,
    /// Index of the sample with the largest dynamic pressure.
    pub max_q: usize,
}

impl Ascent {
    pub(crate) fn fly(drag_on: bool) -> Self {
        let n = (MET_END * RATE) as usize + 1;
        let mut samples = Vec::with_capacity(n);
        let mut booster = Vec::with_capacity(n);
        let mut y: State = [0.0, 0.0, 0.0, 0.0, M_LIFTOFF, 0.0, 0.0, 0.0, 0.0];
        let mut spent: Option<[f64; 4]> = None;
        let h = 1.0 / (RATE * SUB as f64);
        for i in 0..n {
            let met = i as f64 / RATE;
            let v2 = y[VX] * y[VX] + y[VH] * y[VH];
            samples.push(Sample {
                y,
                q: 0.5 * air_density(y[H]) * v2,
            });
            if let Some(b) = spent {
                booster.push([b[0], b[1]]);
            }
            for k in 0..SUB {
                let t = met + k as f64 * h;
                // Staging: the stack sheds the first stage's dry mass at separation.
                if spent.is_none() && t + 1e-9 >= SEP {
                    y[M] = M_UPPER;
                    spent = Some([y[X], y[H], y[VX], y[VH] - 2.0]);
                }
                rk4_step(&mut y, t, h, |t, s| derivs(t, s, drag_on));
                if let Some(b) = &mut spent {
                    // The spent stage coasts ballistically (drag ignored above the air).
                    b[0] += b[2] * h;
                    b[1] += (b[3] - 0.5 * G0 * h) * h;
                    b[3] -= G0 * h;
                }
            }
        }
        let max_q = samples
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.q.total_cmp(&b.1.q))
            .map_or(0, |(i, _)| i);
        Self {
            samples,
            booster,
            max_q,
        }
    }

    /// The state at mission time `met` (linear between samples; clamped).
    pub(crate) fn at(&self, met: f64) -> Sample {
        let last = self.samples.len().saturating_sub(1);
        let u = (met.max(0.0) * RATE).min(last as f64);
        let i = (u as usize).min(last);
        let j = (i + 1).min(last);
        let f = u - i as f64;
        let (a, b) = (self.samples[i], self.samples[j]);
        Sample {
            y: std::array::from_fn(|k| a.y[k] + (b.y[k] - a.y[k]) * f),
            q: a.q + (b.q - a.q) * f,
        }
    }

    /// The spent stage at `met` (only after separation).
    fn booster_at(&self, met: f64) -> Option<[f64; 2]> {
        let i = ((met - SEP) * RATE).floor();
        (i >= 0.0)
            .then(|| self.booster.get(i as usize).copied())
            .flatten()
    }

    fn max_q_met(&self) -> f64 {
        self.max_q as f64 / RATE
    }
}

// ---------------------------------------------------------------------------------------
// Scene clock: countdown and liftoff in real time, the climb in time-lapse (ROC-A8).
// ---------------------------------------------------------------------------------------

const COUNTDOWN: f64 = 6.0;
const LIFTOFF: f64 = 10.0;
const CLIMB: f64 = 24.0;
const STAGING: f64 = 8.0;
const UPPER: f64 = 12.0;
/// Engines light this long before liftoff, while the clamps hold the vehicle.
const IGNITION: f64 = 3.0;
/// Mission seconds covered by the staging and upper-stage acts.
const STAGING_MET: f64 = 16.0;

/// Mission elapsed time at scene time `t` in the cycle: strictly increasing, 1× through
/// liftoff, a Hermite ease peaking near 8× in the climb, then 2× and 4× (ROC-A8).
pub(crate) fn met_at(t: f64) -> f64 {
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
    let climb0 = COUNTDOWN + LIFTOFF;
    let stage0 = climb0 + CLIMB;
    let upper0 = stage0 + STAGING;
    if t < climb0 {
        t - COUNTDOWN
    } else if t < stage0 {
        // Hermite from LIFTOFF to MECO with end slopes (in MET per scene second) of 1 and
        // 2, the neighbouring acts' rates, so the time-lapse never lurches.
        let span = MECO - LIFTOFF;
        let u = (t - climb0) / CLIMB;
        let (m0, m1) = (CLIMB / span, 2.0 * CLIMB / span);
        let (u2, u3) = (u * u, u * u * u);
        let f = (-2.0 * u3 + 3.0 * u2) + m0 * (u3 - 2.0 * u2 + u) + m1 * (u3 - u2);
        LIFTOFF + span * f
    } else if t < upper0 {
        MECO + (t - stage0) * STAGING_MET / STAGING
    } else {
        let rate = (MET_END - MECO - STAGING_MET) / UPPER;
        (MECO + STAGING_MET + (t - upper0) * rate).min(MET_END)
    }
}

// ---------------------------------------------------------------------------------------
// Look: sky, plume, smoke (ROC-A5, ROC-A6).
// ---------------------------------------------------------------------------------------

/// Plume half-width over the nozzle radius at ambient pressure ratio `p`: pressure-matched
/// at sea level, then ballooning as the ambient pressure falls (under-expansion).
pub(crate) fn plume_width(p: f64) -> f64 {
    1.0 + 2.6 * (1.0 - p.clamp(0.0, 1.0)).powf(1.5)
}

/// The flame's length (pixels, for a vehicle or, in vacuum, an upper stage `len` pixels
/// long) and its widening at the far end (over the nozzle width) at altitude `h`
/// (ROC-A6). In air it is 1.5 vehicle lengths at the pad and longer as the air thins; the
/// upper stage's vacuum plume is an under-expanded fan 2.5 stage lengths long, 3 nozzle
/// widths at 40 km opening to 5 by 100 km. Art-directed.
pub(crate) fn plume_shape(len: f64, h: f64, vacuum: bool) -> (f64, f64) {
    let p = pressure_ratio(h);
    if vacuum {
        let open = ease::smoothstep((h - 40_000.0) / 60_000.0);
        (len * 2.5, 3.0 + 2.0 * open)
    } else {
        (len * (1.5 + 1.0 * (1.0 - p)), plume_width(p))
    }
}

/// Shock-diamond spacing over the plume length, or `None` once the air is too thin for
/// the jet to hold a diamond train (above ~11 km). The spacing grows as `1/√p`
/// (Prandtl's `x ≈ 0.67 D √(p_c/pₐ)`, with the chamber pressure `p_c` fixed).
pub(crate) fn diamond_spacing(p: f64) -> Option<f64> {
    (p > 0.27).then(|| 0.16 / p.sqrt())
}

/// Radius (m) of the exhaust smoke laid down `age` seconds ago where the ambient pressure
/// ratio was `p`: a turbulent puff grows like `t^0.75`, and faster in thin air (ROC-A6).
pub(crate) fn smoke_radius(age: f64, p: f64) -> f64 {
    (8.0 + 30.0 * age.max(0.0).powf(0.75)) / p.clamp(0.05, 1.0).sqrt()
}

/// Wind (m/s, downrange) at altitude `h`: calm at the pad, rising toward a jet-stream
/// ~25 m/s at 12 km, which shears the smoke column as it ages.
fn wind(h: f64) -> f64 {
    25.0 * ease::smoothstep(h / 12_000.0)
}

/// Share of the vacuum plume's length that is an opaque core; past it the plume is
/// braille gas (ROC-A6).
const VACUUM_CORE: f64 = 0.3;

/// Smoke and steam puffs (world metres).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Puff {
    x: f64,
    h: f64,
    vx: f64,
    vh: f64,
    age: f64,
    life: f64,
}

const STEAM_MAX: usize = 420;
const STEAM_RATE: f64 = 90.0;

// ---------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------

/// The closest view spans this much altitude (m): the pad and a 70 m vehicle.
const VIEW_MIN: f64 = 150.0;
const VEHICLE_M: f64 = 70.0;
/// One background star per this many cells: a sparse night sky.
const CELLS_PER_STAR: usize = 90;
/// The horizon (ROC-A5): a 1-px dim line where the ground meets the black sky, the pad a
/// brighter segment on it (half-width, m); the ground below is black (ADR-0017).
const HORIZON: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 42);
const PAD: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 90);
const PAD_HALF: f64 = 15.0;
/// The vehicle and its dark interstage band as solid pale pixels, the spent stage a
/// dimmer sliver, and the launch tower a 1-px dim grey line (ROC-A6).
const BODY: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 235);
const BAND: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 60);
const SPENT: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 150);
const TOWER: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 70);
/// Launch tower: x offset from the pad centre and height (m).
const TOWER_X: f64 = -24.0;
const TOWER_H: f64 = 85.0;
/// Smoke and steam dots: `Starlight` level for thin and for dense smoke, and the share of
/// a dense smoke pixel's four braille dots that are lit.
const SMOKE_LEVEL: (f64, f64) = (32.0, 80.0);
const SMOKE_FILL: f64 = 0.35;
/// Flame temperature (K) where the core fades and at the nozzle on its axis; the shock
/// diamonds are white-hot, the scene's one accent.
const FLAME_K: (f64, f64) = (2600.0, 5200.0);
const DIAMOND_K: f64 = 6000.0;
/// Flame heat below which the gas is left dark: the plume is drawn as its core only.
const FLAME_CUT: f64 = 0.3;
/// The plume's core in air: its first share of the length, at most this many pixels
/// either side of the axis; past it, gas dots.
const AIR_CORE: f64 = 0.45;
const CORE_HALF: f64 = 0.3;
/// Telemetry and callouts: dim grey.
const TEXT: Ink = Ink::new(Role::Text, 100);

/// Total ideal Δv of the stack (m/s): both stages' rocket equations, at vacuum Isp.
#[cfg(test)]
fn dv_budget() -> f64 {
    ISP1_VAC * G0 * (M_LIFTOFF / (M_LIFTOFF - PROP1)).ln()
        + ISP2 * G0 * (M_UPPER / (M_UPPER - PROP2)).ln()
}

/// Where the exhaust leaves the vehicle: pixel position, heading from vertical and the
/// body's width in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Nozzle {
    x: f64,
    y: f64,
    th: f64,
    width: f64,
}

/// World → pixel mapping for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Camera {
    /// Pixels per metre.
    k: f64,
    /// World x at pixel column 0 and altitude at pixel row 0 (top).
    x0: f64,
    top: f64,
    /// Frame height in pixels.
    rows: f64,
}

impl Camera {
    fn px(&self, x: f64, h: f64) -> (f64, f64) {
        ((x - self.x0) * self.k, (self.top - h) * self.k)
    }
    fn world_x(&self, px: f64) -> f64 {
        self.x0 + px / self.k
    }
}

/// The horizon's pixel row in column `px` (the pixel holding the ground line, which
/// curves away with the Earth) and its ink, the pad brighter; `None` off the frame.
fn horizon(cam: &Camera, px: i32) -> Option<(i32, Ink)> {
    let x = cam.world_x(f64::from(px) + 0.5);
    let ground = cam.px(x, -x * x / (2.0 * RE)).1;
    let row = ground.floor();
    let ink = if x.abs() < PAD_HALF { PAD } else { HORIZON };
    (row >= 0.0 && row < cam.rows).then_some((row as i32, ink))
}

#[derive(Debug)]
pub struct RocketLaunch {
    ascent: Ascent,
    timeline: Timeline,
    t: f64,
    size: Size,
    /// This frame's camera.
    cam: Camera,
    /// Flame and horizon, as half-block pixels.
    px: Pixels,
    /// Smoke and steam density at pixel resolution, re-deposited every frame: each puff
    /// is a disc of its own radius (`Glow::splat` takes one fixed kernel). Drawn as
    /// sparse braille dots.
    smoke: Vec<f32>,
    tone: Tone,
    /// Vehicle, spent stage, vacuum gas, smoke and stars, as braille.
    dots: Dots,
    stars: Vec<(i32, i32, u8)>,
    steam: ParticlePool<Puff>,
    steam_acc: f64,
    /// Flame flicker in [0, 1), redrawn each update.
    flicker: f64,
    ready: bool,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl RocketLaunch {
    fn new() -> Self {
        Self {
            ascent: Ascent::fly(true),
            timeline: Timeline::new(&[
                ("countdown", COUNTDOWN),
                ("liftoff", LIFTOFF),
                ("climb", CLIMB),
                ("staging", STAGING),
                ("upper", UPPER),
            ]),
            t: 0.0,
            size: Size::new(1, 1),
            cam: Camera {
                k: 1.0,
                x0: 0.0,
                top: 0.0,
                rows: 1.0,
            },
            px: Pixels::default(),
            smoke: Vec::new(),
            tone: Tone::default(),
            dots: Dots::default(),
            stars: Vec::new(),
            steam: ParticlePool::with_capacity(STEAM_MAX),
            steam_acc: 0.0,
            flicker: 0.0,
            ready: false,
            dirty: false,
        }
    }

    fn met(&self) -> f64 {
        met_at(self.t)
    }

    /// The vehicle's pixel height: to scale near the pad, never smaller than a readable
    /// sprite once the camera pulls back (stylised).
    fn vehicle_px(&self, cam: &Camera) -> f64 {
        let min = (f64::from(self.size.height()) * 0.28).clamp(3.0, 14.0);
        (VEHICLE_M * cam.k).max(min)
    }

    /// Frame the pad and the rocket: altitude span grows with height so the whole arc
    /// stays in view; the rocket drifts right as the pad recedes left.
    fn camera(&self, x: f64, h: f64) -> Camera {
        let (w, ph) = (
            f64::from(self.size.width()),
            2.0 * f64::from(self.size.height()),
        );
        let span = VIEW_MIN.hypot(h / 0.62);
        let k = ph / span;
        let bottom = -0.1 * span;
        let width = w / k;
        let drift = 0.5 + 0.2 * ease::smoothstep(x / (0.6 * width));
        Camera {
            k,
            x0: x - drift * width,
            top: bottom + span,
            rows: ph,
        }
    }

    fn rasterise(&mut self) {
        let met = self.met();
        let s = self.ascent.at(met);
        let cam = self.camera(s.y[X], s.y[H]);
        let pw = i32::from(self.px.pixel_size().width());
        self.cam = cam;
        self.px.clear();
        self.dots.clear();
        for x in 0..pw {
            if let Some((y, ink)) = horizon(&cam, x) {
                self.px.set(x, y, ink);
            }
        }
        // The tower: a 1-px line beside the pad while it is in frame.
        let (tx, ty) = cam.px(TOWER_X, TOWER_H);
        let (_, t0) = cam.px(TOWER_X, 0.0);
        let tx = tx.floor() as i32;
        for y in ty.floor() as i32..t0.floor() as i32 {
            self.px.set(tx, y, TOWER);
        }
        self.draw_vehicle(&cam, met, &s);
        // The spent stage, a dim sliver falling away, drawn only once it is 3 px clear of
        // the upper stage's flame, its gas and its body: until then it is lost in the plume
        // (ROC-A6).
        if let Some([bx, bh]) = self.ascent.booster_at(met) {
            let (x, y) = cam.px(bx, bh);
            let len = (self.vehicle_px(&cam) * 0.6).max(2.0);
            let tumble = (met - SEP) * 0.9;
            let hot = |x: i32, y: i32| self.px.get(x, y).is_some_and(flame);
            let clear = stroke(x, y, tumble, len)
                .all(|(x, y)| (-3..=3).all(|dy| (-3..=3).all(|dx| !hot(x + dx, y + dy))));
            if clear {
                for (x, y) in stroke(x, y, tumble, len) {
                    self.px.set(x, y, SPENT);
                }
            }
        }
        // Smoke trail and pad steam: sparse dim dots over the dark sky, each pixel's four
        // dots lit by a fixed hash against its density, so the texture holds still while
        // it spreads and fades.
        self.smoke.fill(0.0);
        self.deposit_trail(&cam, met);
        self.deposit_steam(&cam);
        let width = pw.max(1) as usize;
        for (i, &v) in self.smoke.iter().enumerate() {
            let level = self.tone.level(v);
            if level == 0 {
                continue;
            }
            let (x, y) = ((i % width) as i32, (i / width) as i32);
            // Only in cells whose two pixels are both dark, so a dot never replaces a
            // flame or body half block.
            let cy = y - y.rem_euclid(2);
            if (cy..cy + 2).any(|py| self.px.get(x, py) != Some(Ink::BLANK))
                || self.below_horizon(x, y)
            {
                continue;
            }
            let alpha = f64::from(level) / 255.0;
            let ink = Ink::new(
                Role::Ramp(Ramp::Starlight),
                ink::level(ease::lerp(SMOKE_LEVEL.0, SMOKE_LEVEL.1, alpha)),
            );
            for k in 0..4 {
                let (dx, dy) = (2 * x + (k & 1), 2 * y + (k >> 1));
                if hash01(0x5307 ^ cell_key(dx, dy)) < SMOKE_FILL * alpha {
                    self.dots.set(dx, dy, ink);
                }
            }
        }
        // Stars: in the sky only, and not in a cell something else already uses.
        for &(x, y, level) in &self.stars {
            let (cx, cy) = (x / Dots::CELL_W, y / Dots::CELL_H);
            let busy = self.dots.cell_mask(cx, cy).is_some_and(|m| m != 0)
                || (2 * cy..2 * cy + 2).any(|py| self.px.get(cx, py) != Some(Ink::BLANK));
            if !busy && !self.below_horizon(cx, 2 * cy + 1) {
                self.dots.set(x, y, Ink::new(Role::Star, level));
            }
        }
    }

    /// Whether pixel `(x, y)` is on or below the horizon line.
    fn below_horizon(&self, x: i32, y: i32) -> bool {
        horizon(&self.cam, x).is_some_and(|(row, _)| y >= row)
    }

    /// Deposit the smoke trail behind the rocket: puffs every ~¾ pixel of screen path,
    /// widening with age, lingering low in the air and thinning high up.
    fn deposit_trail(&mut self, cam: &Camera, met: f64) {
        if met <= 0.0 {
            return;
        }
        let width = usize::from(self.size.width());
        let last = (met.min(MECO) * RATE) as usize;
        let mut prev: Option<(f64, f64)> = None;
        for i in (0..=last.min(self.ascent.samples.len() - 1)).rev() {
            let s = self.ascent.samples[i].y;
            let age = met - i as f64 / RATE;
            let p = pressure_ratio(s[H]);
            let r = (smoke_radius(age, p) * cam.k).clamp(0.6, 7.0);
            let (x, y) = cam.px(s[X] + wind(s[H]) * age, s[H]);
            // Puffs overlap by about half their radius, however wide they have grown.
            if let Some((qx, qy)) = prev
                && (x - qx).hypot(y - qy) < (0.4 * r).max(0.75)
            {
                continue;
            }
            prev = Some((x, y));
            // Low smoke lingers, high smoke is thin and fades fast; the weight is spread
            // over the puff's area, so it thins as it grows. Above ~45 km (p < 1 %) the
            // exhaust leaves no trail worth drawing.
            let fade =
                (-age / (40.0 + 160.0 * p)).exp() * (0.25 + 0.75 * p) * ease::smoothstep(p / 0.01);
            puff(&mut self.smoke, width, x, y, r, fade as f32);
        }
    }

    fn deposit_steam(&mut self, cam: &Camera) {
        let width = usize::from(self.size.width());
        for p in &self.steam {
            let (x, y) = cam.px(p.x, p.h);
            let r = ((5.0 + 2.5 * p.age) * cam.k).clamp(0.5, 4.5);
            let w = (1.0 - p.age / p.life).max(0.0) * 0.8;
            puff(&mut self.smoke, width, x, y, r, w as f32);
        }
    }

    /// Plume, then the vehicle body at its pitch: a braille silhouette one to three dots
    /// (at most a pixel and a half) across, pale grey, with a dim interstage band.
    fn draw_vehicle(&mut self, cam: &Camera, met: f64, s: &Sample) {
        let th = pitch(met.max(0.0), s.y[VX], s.y[VH]);
        let (bx, by) = cam.px(s.y[X], s.y[H]);
        let len = self.vehicle_px(cam);
        let upper = met >= SEP;
        // After staging the upper stage stays a readable sprite with a visible nose.
        let body = if upper { (len * 0.35).max(4.0) } else { len };
        let width = (len / 11.0).clamp(1.0, 3.0);
        let (sn, cs) = th.sin_cos_det();
        // Base of what is left of the stack.
        let (bx, by) = if upper {
            (bx + (len - body) * sn, by - (len - body) * cs)
        } else {
            (bx, by)
        };
        // Half-pixel steps along the axis: a solid body of half-block pixels, one or two
        // across (ADR-0017 rule 6: the subject is crisp and bright).
        let steps = (body * 2.0).ceil() as i32;
        for i in 0..=steps {
            let u = f64::from(i) / 2.0;
            let f = u / body.max(1e-9);
            // Nose cone over the top 8 %.
            let w = if f > 0.92 {
                width * (1.0 - f) / 0.08
            } else {
                width
            };
            let ink = if !upper && (0.60..0.66).contains(&f) {
                BAND
            } else {
                BODY
            };
            let n = (w.round() as i32).clamp(1, 2);
            for j in 0..n {
                let v = f64::from(j) - (f64::from(n) - 1.0) / 2.0;
                self.px.set(
                    (bx + u * sn + v * cs).floor() as i32,
                    (by - u * cs + v * sn).floor() as i32,
                    ink,
                );
            }
        }
        let (thrust, _) = engine(met, s.y[H]);
        let firing = thrust > 0.0 || (-IGNITION..0.0).contains(&met);
        if firing {
            let nozzle = Nozzle {
                x: bx,
                y: by,
                th,
                width,
            };
            let vacuum = met >= SES;
            self.draw_plume(nozzle, if vacuum { body } else { len }, s.y[H], vacuum);
        }
    }

    /// The exhaust from `nozzle` (ROC-A6): its `Blackbody` core only, hottest along the
    /// axis at the nozzle and cooling toward the tip and the edges, widening as the air
    /// thins, with white-hot shock diamonds low down; on the pad it stops at the ground.
    /// The upper stage's vacuum plume is a short core and then thin gas, drawn as braille
    /// dots whose density follows its heat, so the stars show through the fan.
    fn draw_plume(&mut self, nozzle: Nozzle, len: f64, h: f64, vacuum: bool) {
        let Nozzle {
            x: bx,
            y: by,
            th,
            width,
        } = nozzle;
        let (sn, cs) = th.sin_cos_det();
        let (plen, spread) = plume_shape(len, h, vacuum);
        let plen = plen * (0.9 + 0.2 * self.flicker);
        let diamonds = if vacuum {
            None
        } else {
            diamond_spacing(pressure_ratio(h))
        };
        let at = |u: f64, v: f64| (bx - u * sn + v * cs, by + u * cs + v * sn);
        // The ground's top pixel row under the pad: pixels from there down are not sky.
        let ground = {
            let x = self.cam.world_x(bx);
            (self.cam.px(x, -x * x / (2.0 * RE)).1 - 0.5).ceil() as i32
        };
        let frame = (self.flicker * 1e6) as u64;
        // Half-pixel steps along and across, so a tilted flame has no holes.
        for i in 1..=(plen * 2.0).floor() as i32 {
            let u = f64::from(i) / 2.0;
            // In [0, 1]: past the tip every weight below would change sign.
            let f = u / plen;
            let half = (0.5 * width).max(0.75) * (1.0 + (spread - 1.0) * f.powf(0.6));
            // Shock diamonds: short lozenges on the axis, a pixel or more inside the flame's
            // edge, tapering to a point at each end of their window.
            let lozenge = diamonds.map_or(0.0, |d| {
                let k = (f / d).fract();
                let dk = k.min(1.0 - k) / 0.18;
                if f < 0.75 && dk < 1.0 {
                    0.5 * (1.0 - dk)
                } else {
                    0.0
                }
            });
            let n = (4.0 * half).ceil() as i32;
            for j in -n / 2..=n / 2 {
                let v = f64::from(j) / 2.0;
                let (xf, yf) = at(u, v);
                let (x, y) = (xf.floor() as i32, yf.floor() as i32);
                let a = v.abs();
                if a > half {
                    continue;
                }
                // Flame that reaches the pad runs out along the row above it, both ways,
                // as far as it would have gone down: it stops at the ground, a bright line
                // across the pad at ignition and liftoff.
                let pooled = y >= ground;
                let (x, y) = if pooled {
                    let run = 0.8 * f64::from(y - ground + 1);
                    let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                    ((bx + side * run).floor() as i32, ground - 1)
                } else {
                    (x, y)
                };
                if pooled && self.px.get(x, y).is_some_and(flame) {
                    continue;
                }
                let edge = a / half;
                let heat = (1.0 - f) * (1.0 - 0.5 * edge * edge);
                // Past its short core the plume is thin gas, in air as in vacuum: the core
                // stays a narrow bright line, never a filled wedge (ADR-0017).
                let core = if vacuum { VACUUM_CORE } else { AIR_CORE };
                if !(f < core && a <= CORE_HALF.min(half)) {
                    // Thin gas: a dot per sample by a chance that follows the heat.
                    if heat >= 0.08 && !pooled {
                        let (dx, dy) = ((2.0 * xf).floor() as i32, (2.0 * yf).floor() as i32);
                        if hash01(frame ^ cell_key(dx, dy)) < 0.8 * heat.sqrt() {
                            let k = ease::lerp(FLAME_K.0, FLAME_K.1, 0.3 * heat);
                            self.dots.set(dx, dy, blackbody(k));
                        }
                    }
                    continue;
                }
                if heat < FLAME_CUT {
                    continue;
                }
                let ink = if a < lozenge {
                    blackbody(DIAMOND_K)
                } else {
                    let hot = (heat - FLAME_CUT) / (1.0 - FLAME_CUT);
                    blackbody(ease::lerp(FLAME_K.0, FLAME_K.1, hot))
                };
                self.px.set(x, y, ink);
            }
        }
    }

    /// Pad steam: the flame trench throws water vapour sideways while the plume still
    /// reaches the ground (ROC-A2).
    fn emit_steam(&mut self, met: f64, alt: f64, dt: f64, ctx: &mut Ctx) {
        if !(-IGNITION..MECO).contains(&met) || alt > 150.0 {
            return;
        }
        self.steam_acc += STEAM_RATE * dt * (1.0 - alt / 150.0);
        while self.steam_acc >= 1.0 {
            self.steam_acc -= 1.0;
            let side = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
            let (jx, jh) = gaussian2(&mut ctx.rng, 4.0);
            self.steam.spawn(Puff {
                x: side * ctx.rng.random_range(10.0..30.0),
                h: 2.0,
                vx: side * ctx.rng.random_range(15.0..45.0) + jx,
                vh: ctx.rng.random_range(3.0..12.0) + jh,
                age: 0.0,
                life: ctx.rng.random_range(8.0..18.0),
            });
        }
    }

    fn age_steam(&mut self, dt: f64) {
        let k = (-0.6 * dt).exp();
        self.steam.retain(|p| {
            p.vx *= k;
            p.vh = p.vh * k + 1.6 * dt;
            p.x += p.vx * dt;
            p.h = (p.h + p.vh * dt).max(0.0);
            p.age += dt;
            p.age < p.life
        });
    }

    fn caption_index(&self) -> Option<usize> {
        let met = self.met();
        let q = self.ascent.max_q_met();
        match met {
            m if m < 0.0 => None,
            m if m < LIFTOFF => Some(0),
            m if (m - q).abs() < 8.0 => Some(2),
            m if m < MECO => Some(1),
            m if m < SES => Some(3),
            _ => Some(4),
        }
    }
}

/// A flame pixel: the plume's `Blackbody` core.
fn flame(i: Ink) -> bool {
    i.role == Role::Blackbody
}

/// The pixels of a one-pixel bar `len` long from its base `(x, y)` along heading `th`
/// (from vertical).
fn stroke(x: f64, y: f64, th: f64, len: f64) -> impl Iterator<Item = (i32, i32)> {
    let (s, c) = th.sin_cos_det();
    (0..=(len * 2.0).ceil() as i32).map(move |i| {
        let u = f64::from(i) / 2.0;
        ((x + u * s).floor() as i32, (y - u * c).floor() as i32)
    })
}

/// A [`draw::label`] centred on row `y`.
fn label_centred(canvas: &mut Canvas, y: i32, s: &str, ink: Ink) {
    let x = (i32::from(canvas.size().width()) - s.chars().count() as i32) / 2;
    draw::label(canvas, x - 1, y, s, ink);
}

/// Deposit a soft disc of total weight ~`w` at pixel `(x, y)` with radius `r` pixels into
/// the row-major buffer `buf` of rows `width` pixels long; off-buffer pixels are skipped.
fn puff(buf: &mut [f32], width: usize, x: f64, y: f64, r: f64, w: f32) {
    if !(x.is_finite() && y.is_finite()) || w <= 0.0 || width == 0 {
        return;
    }
    let rows = buf.len() / width;
    let reach = r.ceil() as i32;
    let (cx, cy) = (x.floor() as i32, y.floor() as i32);
    let norm = (f64::from(w) / (r * r).max(1.0)) as f32;
    for oy in -reach..=reach {
        for ox in -reach..=reach {
            let (px, py) = (cx + ox, cy + oy);
            let d2 = f64::from(ox * ox + oy * oy) / (r * r).max(0.25);
            if d2 < 1.0 && (0..width as i32).contains(&px) && (0..rows as i32).contains(&py) {
                buf[py as usize * width + px as usize] += norm * (1.0 - d2 as f32);
            }
        }
    }
}

impl Generator for RocketLaunch {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.px.resize(size);
        self.smoke.clear();
        self.smoke.resize(2 * size.area(), 0.0);
        self.dots.resize(size);
        self.tone = Tone::asinh(0.15, 1.5);
        let dot = self.dots.dot_size();
        let n = size.area() / CELLS_PER_STAR;
        self.stars.clear();
        self.stars.extend((0..n).map(|_| {
            (
                ctx.rng.random_range(0..i32::from(dot.width())),
                ctx.rng.random_range(0..i32::from(dot.height())),
                ctx.rng.random_range(40..130u8),
            )
        }));
        self.ready = true;
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !self.ready {
            self.resize(ctx.size(), ctx);
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let total = self.timeline.total();
        let next = self.t + dt;
        if next >= total {
            self.t = next - total;
            self.steam.clear();
            self.steam_acc = 0.0;
        } else {
            self.t = next;
        }
        let met = self.met();
        let alt = self.ascent.at(met).y[H];
        // Steam moves in mission time, so it billows at the time-lapse rate too.
        let dmet = (met_at(self.t + dt) - met).clamp(0.0, 1.0);
        self.age_steam(dmet.max(dt));
        self.emit_steam(met, alt, dmet.max(dt), ctx);
        self.flicker = ctx.rng.random();
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if !self.ready {
            return;
        }
        // ASCII: the flame `#`, the vehicle and tower `|`, the horizon `_`.
        self.px.blit(canvas, |_, _, top, bottom| {
            let ink = raster::stronger(top, bottom);
            match ink.role {
                Role::Background => None,
                Role::Blackbody => Some(('#', ink)),
                _ if matches!(ink, BODY | BAND | TOWER | SPENT) => Some(('|', ink)),
                _ => Some(('_', ink)),
            }
        });
        self.dots.blit_over(canvas, &self.px);
        if self.size.height() >= 8 {
            self.draw_text(canvas);
        }
    }

    fn caption(&self) -> Option<&'static str> {
        self.caption_index().map(|i| ACTS[i])
    }
}

/// Whole seconds on the mission clock (ROC-A8): a countdown shows the seconds still to go
/// (T−2 until 1 s is left), the climb the seconds elapsed, so the row-0 clock and the big
/// countdown always agree.
fn clock_secs(met: f64) -> u32 {
    (if met < 0.0 {
        (-met).ceil()
    } else {
        met.floor()
    }) as u32
}

impl RocketLaunch {
    /// Countdown and event callouts, and the telemetry, in dim grey (ROC-A9).
    fn draw_text(&self, canvas: &mut Canvas) {
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        let met = self.met();
        let s = self.ascent.at(met);
        let y = h / 5;
        if met < 0.0 {
            // T−3 … T−1 over the last seconds, the clock the whole way.
            let n = clock_secs(met);
            let mut buf = StackText::<8>::new();
            let _ = write!(buf, "T-{n}");
            label_centred(canvas, y, buf.as_str(), TEXT);
        } else {
            let callout = match met {
                m if m < 2.5 => "LIFTOFF",
                m if (m - self.ascent.max_q_met()).abs() < 4.0 => "MAX-Q",
                m if (MECO..SEP).contains(&m) => "MAIN ENGINE CUTOFF",
                m if (SEP..SES).contains(&m) => "STAGE SEPARATION",
                m if (SES..SES + 6.0).contains(&m) => "SECOND STAGE IGNITION",
                _ => "",
            };
            if !callout.is_empty() && callout.len() as i32 + 4 <= w {
                label_centred(canvas, y, callout, TEXT);
            }
        }
        if w >= 44 {
            self.telemetry(canvas, met, &s);
        }
    }

    /// `T+01:12  ALT 12.3 km  VEL 452 m/s  q 31 kPa  x8` on row 0.
    fn telemetry(&self, canvas: &mut Canvas, met: f64, s: &Sample) {
        let mut buf = StackText::<80>::new();
        let sign = if met < 0.0 { '-' } else { '+' };
        let secs = clock_secs(met);
        let v = s.y[VX].hypot(s.y[VH]);
        let _ = write!(
            buf,
            "T{sign}{:02}:{:02}  ALT {:>5.1} km  VEL {:>4.0} m/s  q {:>4.1} kPa",
            secs / 60,
            secs % 60,
            s.y[H] / 1000.0,
            v,
            s.q / 1000.0
        );
        let rate = met_at(self.t + 0.05) - met_at(self.t);
        let lapse = (rate / 0.05).round();
        if lapse >= 2.0 {
            let _ = write!(buf, "  x{lapse:.0}");
        }
        if buf.len() + 2 <= usize::from(self.size.width()) {
            draw::label(canvas, 0, 0, buf.as_str(), TEXT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speed(y: &State) -> f64 {
        y[VX].hypot(y[VH])
    }

    #[test]
    fn roc_a1_vertical_vacuum_burn_follows_the_rocket_equation() {
        // Stage 1 straight up with no drag: v = ∫T/m − ∫g, and ∫T/m for constant mass
        // flow and vacuum thrust is exactly ve ln(m0/m) (Tsiolkovsky).
        let a = Ascent::fly(false);
        for met in [2.0, 5.0, 8.0] {
            let s = a.at(met).y;
            let ideal_sl_bound = ISP1_VAC * G0 * (M_LIFTOFF / s[M]).ln();
            // Sea-level back-pressure costs thrust, so the ideal Δv sits just below the
            // vacuum rocket equation, within the 8 % sea-level thrust deficit.
            assert!(s[IDEAL] <= ideal_sl_bound * (1.0 + 1e-9));
            assert!(s[IDEAL] >= ideal_sl_bound * (T1_SL / T1_VAC) * 0.999);
            // Vertical flight: speed = ideal − gravity loss exactly (no drag, no steer).
            assert!((speed(&s) - (s[IDEAL] - s[GRAV])).abs() < 1e-3 * speed(&s).max(1.0));
            assert!(s[VX].abs() < 1e-9, "still vertical before the kick");
        }
        // Mass bookkeeping: m = m0 − ṁ t.
        let s = a.at(100.0).y;
        assert!((s[M] - (M_LIFTOFF - MDOT1 * 100.0)).abs() < 1.0);
    }

    #[test]
    fn roc_a2_steam_only_while_the_plume_hits_the_pad_and_pool_is_bounded() {
        let mut g = RocketLaunch::new();
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(7, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let mut before_ignition = true;
        for _ in 0..(70 * 60) {
            let n0 = g.steam.len();
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let met = g.met();
            if met < -IGNITION - 0.05 && before_ignition {
                assert_eq!(g.steam.len(), 0, "steam before ignition at {met}");
            } else {
                before_ignition = false;
            }
            if g.ascent.at(met).y[H] > 200.0 && met > 0.0 {
                assert!(g.steam.len() <= n0, "steam still spawning at {met}");
            }
            assert!(g.steam.len() <= STEAM_MAX);
        }
    }

    #[test]
    fn roc_a3_gravity_turn_climbs_and_tips_monotonically_until_cutoff() {
        let a = Ascent::fly(true);
        let mut last_gamma = 0.0;
        let mut last_h = -1.0;
        for i in 0..(MECO * RATE) as usize {
            let y = a.samples[i].y;
            assert!(y[H] >= last_h, "altitude fell at {}", i as f64 / RATE);
            last_h = y[H];
            let met = i as f64 / RATE;
            if met > KICK_AT + KICK_S + 0.5 {
                // Angle from vertical only grows: gravity bends the path over.
                let gamma = y[VX].atan2(y[VH]);
                assert!(gamma >= last_gamma - 1e-9, "turned back at {met}");
                last_gamma = gamma;
            }
        }
        let meco = a.at(MECO).y;
        let gamma_deg = 90.0 - meco[VX].atan2(meco[VH]).to_degrees();
        assert!(
            (55e3..90e3).contains(&meco[H]),
            "MECO at {} km",
            meco[H] / 1e3
        );
        assert!(
            (10.0..40.0).contains(&gamma_deg),
            "flight path {gamma_deg}° at MECO"
        );
        assert!(
            (1_900.0..3_200.0).contains(&speed(&meco)),
            "{} m/s",
            speed(&meco)
        );
    }

    #[test]
    fn roc_a4_max_q_is_where_a_launch_meets_it() {
        let a = Ascent::fly(true);
        let s = a.samples[a.max_q];
        let met = a.max_q_met();
        assert!((55.0..90.0).contains(&met), "max-Q at T+{met}");
        assert!((9e3..16e3).contains(&s.y[H]), "at {} km", s.y[H] / 1e3);
        assert!((20e3..45e3).contains(&s.q), "{} kPa", s.q / 1e3);
    }

    #[test]
    fn roc_a6_plume_widens_as_air_thins_and_diamonds_only_low() {
        let mut prev = 0.0;
        for i in 0..100 {
            let wdt = plume_width(pressure_ratio(f64::from(i) * 1000.0));
            assert!(wdt >= prev);
            prev = wdt;
        }
        assert!((plume_width(1.0) - 1.0).abs() < 1e-12);
        assert!(diamond_spacing(pressure_ratio(1_000.0)).is_some());
        assert!(diamond_spacing(pressure_ratio(12_000.0)).is_none());
        let (a, b) = (diamond_spacing(1.0).unwrap(), diamond_spacing(0.5).unwrap());
        assert!(b > a, "spacing grows as pressure falls");
    }

    #[test]
    fn roc_a7_delta_v_ledger_closes() {
        let a = Ascent::fly(true);
        for met in [30.0, 80.0, MECO - 0.5, SES + 20.0] {
            let y = a.at(met).y;
            let v = speed(&y);
            let spent = y[IDEAL] - y[GRAV] - y[DRAG] - y[STEER];
            assert!((spent - v).abs() < 0.01 * v, "T+{met}: {spent} vs {v}");
            assert!(y[GRAV] > 0.0 && y[DRAG] > 0.0 && y[STEER] >= 0.0);
        }
        // Losses are the textbook size: gravity ≈ 1–1.6 km/s, drag tens to a few hundred m/s
        // (Saturn V ≈ 40 m/s, Shuttle ≈ 110 m/s).
        let y = a.at(MECO).y;
        assert!(
            (900.0..1_700.0).contains(&y[GRAV]),
            "gravity loss {}",
            y[GRAV]
        );
        assert!((20.0..300.0).contains(&y[DRAG]), "drag loss {}", y[DRAG]);
        assert!(y[IDEAL] < dv_budget());
    }

    #[test]
    fn roc_a8_clock_is_monotone_and_the_cycle_loops() {
        let mut prev = f64::NEG_INFINITY;
        for i in 0..6000 {
            let t = f64::from(i) * 0.01;
            let m = met_at(t);
            assert!(m > prev || m == MET_END, "MET stalls at {t}");
            prev = m;
        }
        assert_eq!(met_at(0.0), -COUNTDOWN);
        assert!((met_at(COUNTDOWN + LIFTOFF) - LIFTOFF).abs() < 1e-9);
        assert!((met_at(COUNTDOWN + LIFTOFF + CLIMB) - MECO).abs() < 1e-9);
        let mut g = RocketLaunch::new();
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(1, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let total = g.timeline.total();
        let mut t = 0.0;
        while t < total + 1.0 {
            g.update(0.25, &mut ctx);
            g.prepare();
            t += 0.25;
        }
        assert!(g.t < 1.5, "looped: t = {}", g.t);
    }

    #[test]
    fn roc_a8_the_telemetry_clock_counts_down_with_the_big_countdown() {
        // A countdown shows the whole seconds still to go: at T−1.5 s both read 2.
        let size = Size::new(160, 48);
        for t in [0.3, 0.5, 1.2, 1.5, 2.0, COUNTDOWN - 0.4] {
            let g = scene_at(size, t);
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let rows = canvas.to_lines();
            let n = (-g.met()).ceil() as u32;
            assert!(
                rows[0].contains(&format!("T-00:{n:02}")),
                "t {t}: {}",
                rows[0]
            );
            assert!(
                rows.iter().any(|r| r.contains(&format!(" T-{n} "))),
                "t {t}"
            );
        }
    }

    #[test]
    fn gen_r12_rocket_caption_follows_the_flight() {
        let mut g = RocketLaunch::new();
        g.t = 1.0;
        assert_eq!(g.caption(), None);
        g.t = COUNTDOWN + 3.0;
        assert_eq!(g.caption(), Some("liftoff"));
        g.t = COUNTDOWN + LIFTOFF + CLIMB + 1.0;
        assert_eq!(g.caption(), Some("stage separation"));
        // Somewhere in the climb it reads max-Q.
        let hit = (0..2400).any(|i| {
            g.t = COUNTDOWN + LIFTOFF + f64::from(i) * 0.01;
            g.caption() == Some("max-Q")
        });
        assert!(hit);
    }

    /// A scene at `size` stepped to scene time `t`.
    fn scene_at(size: Size, t: f64) -> RocketLaunch {
        let mut g = RocketLaunch::new();
        let mut ctx = Ctx::new(3, size);
        g.resize(size, &mut ctx);
        g.prepare();
        while g.t < t {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        g
    }

    /// Every pixel of `g`'s half-block layer with its ink.
    fn pixels(g: &RocketLaunch) -> impl Iterator<Item = (i32, i32, Ink)> + '_ {
        let size = g.px.pixel_size();
        (0..i32::from(size.height())).flat_map(move |y| {
            (0..i32::from(size.width())).map(move |x| (x, y, g.px.get(x, y).unwrap()))
        })
    }

    #[test]
    fn roc_a5_the_night_sky_is_black_over_a_one_pixel_horizon() {
        // ADR-0017: the sky and the ground are black; the only lit backdrop pixel in a
        // column is the horizon line (the pad brighter on it).
        for t in [
            2.0,
            COUNTDOWN + LIFTOFF + 6.0,
            COUNTDOWN + LIFTOFF + CLIMB + 10.0,
        ] {
            let g = scene_at(Size::new(120, 40), t);
            let mut line = 0;
            for (x, y, ink) in pixels(&g) {
                if flame(ink) || ink == Ink::BLANK || matches!(ink, BODY | BAND | TOWER | SPENT) {
                    continue;
                }
                let row = horizon(&g.cam, x).map(|(r, _)| r);
                assert_eq!(Some(y), row, "t {t}: {ink:?} off the horizon at {x},{y}");
                assert!(matches!(ink, HORIZON | PAD), "t {t}: {ink:?} at {x},{y}");
                line += 1;
            }
            // The horizon is in frame on the pad and while the climb is low.
            if t < COUNTDOWN + LIFTOFF + CLIMB {
                assert!(line > 100, "t {t}: horizon of {line} px");
            }
        }
    }

    #[test]
    fn roc_a6_smoke_spreads_and_thins_with_age() {
        let mut prev = 0.0;
        for i in 0..100 {
            let r = smoke_radius(f64::from(i), 0.5);
            assert!(r > prev, "smoke stopped spreading at {i} s");
            prev = r;
        }
        // Thinner air lets it spread wider.
        assert!(smoke_radius(20.0, 0.1) > smoke_radius(20.0, 1.0));
        assert!(smoke_radius(60.0, 1.0) > 4.0 * smoke_radius(0.0, 1.0));
        // Drawn as sparse dim dots: never more than about half of a dense pixel's dots.
        let g = scene_at(Size::new(120, 40), COUNTDOWN + LIFTOFF + 8.0);
        let mut canvas = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut canvas);
        let smoke: Vec<_> = canvas
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Ramp(Ramp::Starlight) && c.ink.level <= 80)
            .collect();
        assert!(smoke.len() > 20, "{} smoke cells", smoke.len());
    }

    #[test]
    fn roc_a6_the_plume_is_a_blackbody_core_hottest_at_the_nozzle() {
        let g = scene_at(Size::new(120, 40), COUNTDOWN + 1.5);
        let hot = blackbody(FLAME_K.1 - 200.0).level;
        let flames: Vec<Ink> = pixels(&g).map(|p| p.2).filter(|&i| flame(i)).collect();
        assert!(flames.iter().any(|i| i.level >= hot), "no hot core");
        // Core only: no pixel cooler than the flame's own cool end (and no diamonds on
        // the pad, none brighter than white-hot).
        let (cool, white) = (blackbody(FLAME_K.0).level, blackbody(DIAMOND_K).level);
        assert!(flames.iter().all(|i| (cool..=white).contains(&i.level)));
    }

    #[test]
    fn roc_a6_the_plume_outgrows_the_vehicle_and_fans_out_in_vacuum() {
        for h in [0.0, 2_000.0, 9_000.0] {
            let (len, _) = plume_shape(20.0, h, false);
            assert!(len >= 1.5 * 20.0, "{h} m: plume {len} px");
        }
        let (_, low) = plume_shape(20.0, 40_000.0, true);
        let (_, high) = plume_shape(20.0, 100_000.0, true);
        assert!((3.0..=3.2).contains(&low) && (4.8..=5.0).contains(&high));
    }

    #[test]
    fn roc_a6_the_vehicle_is_a_solid_pale_pixel_silhouette() {
        for t in [
            2.0,
            COUNTDOWN + LIFTOFF + 4.0,
            COUNTDOWN + LIFTOFF + CLIMB + 18.0,
        ] {
            let size = Size::new(120, 40);
            let g = scene_at(size, t);
            // Solid half-block pixels, bright pale grey: the one subject.
            let body = pixels(&g).filter(|p| p.2 == BODY).count();
            assert!(body >= 4, "t {t}: {body} body pixels");
        }
    }

    #[test]
    fn roc_a6_the_spent_stage_keeps_clear_of_the_plume() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(1, size);
        let mut g = RocketLaunch::new();
        g.resize(size, &mut ctx);
        g.prepare();
        let mut seen = false;
        for step in 0..(60.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if step % 10 != 0 || g.met() < SES {
                continue;
            }
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            for cy in 0..24 {
                for cx in 0..80 {
                    let spent = (2 * cy..2 * cy + 2).any(|y| g.px.get(cx, y) == Some(SPENT));
                    if !spent {
                        continue;
                    }
                    seen = true;
                    for dy in -2..=3 {
                        for dx in -2..=2 {
                            let near = g.px.get(cx + dx, 2 * cy + dy).is_some_and(flame);
                            assert!(!near, "MET {:.0}: stage at {cx},{cy} in the plume", g.met());
                        }
                    }
                }
            }
        }
        assert!(seen, "the spent stage never showed");
    }

    #[test]
    fn roc_a6_the_vacuum_plume_is_sparse_gas_behind_a_short_core() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let g = scene_at(size, COUNTDOWN + LIFTOFF + CLIMB + STAGING + UPPER - 1.0);
            assert!(g.met() > SES + 20.0, "the upper stage is firing");
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let opaque = pixels(&g).filter(|p| flame(p.2)).count();
            let gas: Vec<Ink> = canvas
                .cells()
                .iter()
                .filter(|c| ('\u{2801}'..='\u{28FF}').contains(&c.ch) && flame(c.ink))
                .map(|c| c.ink)
                .collect();
            // A braille cell covers two pixels' area.
            assert!(
                opaque < 2 * gas.len(),
                "{size:?}: {opaque} opaque flame pixels, {} gas cells",
                gas.len()
            );
            // Stars show through: the gas is dots, not a filled wedge.
            assert!(gas.len() >= 4, "{size:?}: gas cells {}", gas.len());
            // Even its faint edge is the flame's own colour, never below its cool end.
            let floor = blackbody(FLAME_K.0).level;
            for ink in gas {
                assert!(ink.level >= floor, "{size:?}: {ink:?}");
            }
        }
    }

    #[test]
    fn roc_a6_the_pad_flame_stops_at_the_ground() {
        let size = Size::new(120, 40);
        let g = scene_at(size, COUNTDOWN - 1.0);
        let mut any = 0;
        for (x, y, ink) in pixels(&g) {
            if !flame(ink) {
                continue;
            }
            any += 1;
            let row = horizon(&g.cam, x).map_or(i32::MAX, |(r, _)| r);
            assert!(y <= row, "flame below the ground at {x},{y}");
        }
        assert!(any > 0, "no flame on the pad");
    }

    #[test]
    fn roc_a6_shock_diamonds_are_white_hot_points_on_the_axis() {
        let g = scene_at(Size::new(200, 50), COUNTDOWN + LIFTOFF + 4.0);
        assert!(diamond_spacing(pressure_ratio(g.ascent.at(g.met()).y[H])).is_some());
        let white = blackbody(DIAMOND_K);
        let mut diamonds = 0;
        for (x, y, ink) in pixels(&g) {
            if ink != white {
                continue;
            }
            diamonds += 1;
            // A point, not a bar across the flame; and inside the flame.
            assert!(g.px.get(x - 1, y) != Some(white) || g.px.get(x + 1, y) != Some(white));
            let beside = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .any(|(dx, dy)| {
                    g.px.get(x + dx, y + dy)
                        .is_some_and(|i| flame(i) && i != white)
                });
            assert!(beside, "diamond at {x},{y} outside the flame");
        }
        assert!(diamonds > 0, "no shock diamonds low down");
    }

    #[test]
    fn roc_a9_the_telemetry_is_dim_grey_text_on_row_0() {
        let size = Size::new(160, 48);
        let g = scene_at(size, 2.0);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let rows = canvas.to_lines();
        assert!(rows[0].trim_start().starts_with("T-"), "{}", rows[0]);
        for x in 0..20 {
            let cell = canvas.get(x, 0).unwrap();
            assert_eq!(cell.bg, Ink::BLANK, "telemetry cell {x}");
            assert!(cell.ch == ' ' || cell.ink == TEXT, "{cell:?}");
        }
    }

    #[test]
    fn roc_a10_a_sparse_night_frame_with_one_accent() {
        // ADR-0017: black sky, no tower, Sun or Δv bar; only greys, the Blackbody flame
        // and the white stars; and little of the frame lit.
        let mut stage = Stage::new(build(), 42, Size::new(160, 48), GlyphMode::Unicode);
        stage.set_title_visible(false);
        let mut seen_flame = false;
        for _ in 0..62 {
            for _ in 0..60 {
                stage.step(SIM_DT);
            }
            let f = stage.render();
            let mut lit = 0;
            for c in f.cells() {
                for ink in [c.ink, c.bg] {
                    assert!(
                        matches!(
                            ink.role,
                            Role::Background
                                | Role::Ramp(Ramp::Starlight)
                                | Role::Blackbody
                                | Role::Star
                                | Role::Text
                        ),
                        "{ink:?}"
                    );
                    seen_flame |= ink.role == Role::Blackbody;
                }
                lit += usize::from(c.ch != ' ' || c.bg != Ink::BLANK);
            }
            assert!(
                lit * 100 < f.cells().len() * 14,
                "{lit} of {} cells lit",
                f.cells().len()
            );
        }
        assert!(seen_flame);
    }
}
