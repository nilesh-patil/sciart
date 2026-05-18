//! A two-stage launch from the pad to the edge of space (SPEC-002 `rocket_launch`).
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
//! The camera pulls back with altitude so the whole arc stays in frame. The sky darkens
//! with the air above and pales toward the horizon (ROC-A5); the plume is a white-hot
//! flame that widens as the air thins and shows shock diamonds only low down, and the
//! smoke trail spreads and thins as it ages (ROC-A6); a Δv bar splits the engine's work
//! into speed gained, gravity loss, drag and steering losses (ROC-A7). Smoke and pad steam
//! are a tone-mapped pixel-resolution buffer blended into the sky [`Pixels`]; everything
//! is rasterised in `update`. Text sits on dark chips, legible over any sky (ROC-A9).

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::ink::{self, blackbody, ramp};
use sciart_core::ode::rk4_step;
use sciart_core::particles::gaussian2;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Tone, draw, ease};

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
pub(crate) const IDEAL: usize = 5;
pub(crate) const GRAV: usize = 6;
pub(crate) const DRAG: usize = 7;
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

/// Sky brightness (Ice ramp value) at altitude `h`: the thinner the air above, the less
/// blue it scatters. Art-directed: a `√p` fall (single scattering would go roughly as `p`)
/// keeps the blue in frame longer, black by ~60 km.
pub(crate) fn sky_level(h: f64) -> f64 {
    0.60 * pressure_ratio(h).sqrt()
}

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

/// The closest view spans this much altitude (m): pad, tower and a 70 m vehicle.
const VIEW_MIN: f64 = 150.0;
/// Day-sky gradient over the frame (ROC-A5): the level is `sky_level(h)` times
/// `SKY_ZENITH + SKY_HORIZON·exp(−rise / SKY_FALL)`, `rise` the height above the horizon as a
/// share of the frame: a deep-blue zenith paling to near-white at the horizon.
const SKY_ZENITH: f64 = 0.5;
const SKY_HORIZON: f64 = 0.95;
const SKY_FALL: f64 = 0.3;
/// The Sun (ROC-A10): its place on screen as shares of the pixel height (x, y), its
/// radius and glare radius in the same units, and the glare's peak added to the haze.
const SUN_AT: (f64, f64) = (0.42, 0.34);
const SUN_R: f64 = 0.035;
const SUN_GLOW_R: f64 = 0.22;
const SUN_GLARE: f64 = 0.55;
const VEHICLE_M: f64 = 70.0;
/// Launch tower: x offset from the pad centre and height (m).
const TOWER_X: f64 = -24.0;
const TOWER_H: f64 = 85.0;
/// Coastline, downrange of the pad (m).
const COAST: f64 = 1_800.0;
/// One background star per this many cells (seen only once the sky is dark).
const CELLS_PER_STAR: usize = 45;
/// Δv bar size (cells) and the smallest canvas that shows it.
const BAR_W: i32 = 30;
const BAR_MIN_W: u16 = 100;

/// Total ideal Δv of the stack (m/s): both stages' rocket equations, at vacuum Isp.
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
    fn world_h(&self, py: f64) -> f64 {
        self.top - py / self.k
    }
}

#[derive(Debug)]
pub struct RocketLaunch {
    ascent: Ascent,
    timeline: Timeline,
    budget: f64,
    t: f64,
    size: Size,
    /// This frame's camera, kept for the ASCII fallback.
    cam: Camera,
    px: Pixels,
    /// Smoke and steam at pixel resolution, re-deposited every frame. A local buffer, not
    /// a `raster::Glow`: each puff is a disc of its own radius (`Glow::splat` takes one
    /// fixed kernel), and the result is blended with the pixel under it, not written by
    /// `Glow::to_pixels`.
    smoke: Vec<f32>,
    tone: Tone,
    dots: Dots,
    stars: Vec<(i32, i32, u8)>,
    steam: ParticlePool<Puff>,
    steam_acc: f64,
    /// Flame flicker in [0, 1), redrawn each update.
    flicker: f64,
    ready: bool,
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
            budget: dv_budget(),
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
        let (pw, ph) = {
            let p = self.px.pixel_size();
            (i32::from(p.width()), i32::from(p.height()))
        };
        // Sky and ground, pixel by pixel.
        self.cam = cam;
        for py in 0..ph {
            for px in 0..pw {
                self.px.set(px, py, backdrop(&cam, px, py).0);
            }
        }
        self.draw_tower(&cam);
        // Smoke trail and pad steam, tone-mapped and composited over the sky.
        self.smoke.fill(0.0);
        self.deposit_trail(&cam, met);
        self.deposit_steam(&cam);
        let width = pw.max(1) as usize;
        for (i, &v) in self.smoke.iter().enumerate() {
            let level = self.tone.level(v);
            if level == 0 {
                continue;
            }
            let (px, py) = ((i % width) as i32, (i / width) as i32);
            let alpha = f64::from(level) / 255.0;
            let Some(under) = self.px.get(px, py) else {
                continue;
            };
            // Smoke is a pale veil: over the sky (one ramp) it lifts the level toward
            // white by its density; over black space it is a faint glow of its density
            // (black is the ramp's own bottom); over the ground, which it cannot mix
            // with, it covers a share of the pixels given by its density.
            let smoke = match under.role {
                Role::Ramp(Ramp::Ice) => {
                    let sky = f64::from(under.level) / 255.0;
                    Some(ramp(Ramp::Ice, sky + (0.95 - sky).max(0.0) * 0.9 * alpha))
                }
                Role::Background => (alpha > 0.05).then(|| ramp(Ramp::Ice, 0.06 + 0.45 * alpha)),
                _ => (alpha > noise(px, py)).then(|| ramp(Ramp::Ice, 0.35 + 0.55 * alpha)),
            };
            if let Some(ink) = smoke {
                self.px.set(px, py, ink);
            }
        }
        self.dots.clear();
        self.draw_vehicle(&cam, met, &s);
        // The spent stage, a dim sliver falling away, drawn only once it is 3 px clear of
        // the upper stage's flame and its gas: until then it is lost in the plume (ROC-A6).
        if let Some([bx, bh]) = self.ascent.booster_at(met) {
            let (x, y) = cam.px(bx, bh);
            let len = (self.vehicle_px(&cam) * 0.6).max(2.0);
            let tumble = (met - SEP) * 0.9;
            let flame = |i: Ink| matches!(i.role, Role::Blackbody | Role::Ramp(Ramp::Ember));
            let hot = |x: i32, y: i32| {
                self.px.get(x, y).is_some_and(flame)
                    || self.dots.cell_mask(x, y / 2).is_some_and(|m| m != 0)
            };
            let clear = stroke(x, y, tumble, len)
                .all(|(x, y)| (-3..=3).all(|dy| (-3..=3).all(|dx| !hot(x + dx, y + dy))));
            if clear {
                for (x, y) in stroke(x, y, tumble, len) {
                    self.px.set(x, y, Ink::new(Role::Structure, 120));
                }
            }
        }
        // Stars show only where the sky has gone black, and not in the plume's gas.
        for &(x, y, level) in &self.stars {
            let (cx, cy) = (x / Dots::CELL_W, y / 2);
            let gas = self
                .dots
                .cell_mask(cx, y / Dots::CELL_H)
                .is_some_and(|m| m != 0);
            if !gas
                && self
                    .px
                    .get(cx, cy)
                    .is_some_and(|i| i.role == Role::Background)
            {
                self.dots.set(x, y, Ink::new(Role::Star, level));
            }
        }
    }

    /// Lattice tower beside the pad, to scale.
    fn draw_tower(&mut self, cam: &Camera) {
        let (x0, y0) = cam.px(TOWER_X - 4.0, TOWER_H);
        let (x1, y1) = cam.px(TOWER_X + 4.0, 0.0);
        if x1 - x0 < 0.6 {
            return;
        }
        let steel = Ink::new(Role::Structure, 120);
        for py in y0.floor() as i32..y1.ceil() as i32 {
            for px in x0.floor() as i32..x1.ceil() as i32 {
                let edge = px == x0.floor() as i32 || px + 1 == x1.ceil() as i32;
                // Cross-bracing: a diagonal every few metres.
                let h = cam.world_h(f64::from(py));
                let x = cam.world_x(f64::from(px));
                let brace =
                    ((h + (x - TOWER_X) * 1.5).rem_euclid(8.0)) < 8.0 / (cam.k * 8.0).max(1.0);
                if edge || brace {
                    self.px.set(px, py, steel);
                }
            }
        }
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
            // exhaust leaves no trail worth drawing: it would be a dim bar over black.
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

    /// Plume, then the vehicle body, at its pitch.
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
        let white = Ink::new(Role::Star, 235);
        let band = Ink::new(Role::Dim, 60);
        let steps = (body * 2.0).ceil() as i32;
        for i in 0..=steps {
            let u = f64::from(i) / 2.0;
            let f = u / body.max(1e-9);
            // Nose cone over the top 8 %, a dark interstage band on the full stack.
            let w = if f > 0.92 {
                width * (1.0 - f) / 0.08
            } else {
                width
            };
            let ink = if !upper && (0.60..0.66).contains(&f) {
                band
            } else {
                white
            };
            let n = (w.ceil() as i32).max(1);
            for j in 0..n {
                let v = f64::from(j) - (f64::from(n) - 1.0) / 2.0;
                self.px.set(
                    (bx + u * sn + v * cs).floor() as i32,
                    (by - u * cs + v * sn).floor() as i32,
                    ink,
                );
            }
        }
    }

    /// The exhaust from `nozzle` (ROC-A6): a dark engine bell, then a flame white-hot along
    /// its axis near the bell, cooling through yellow and orange into a deep Ember fringe
    /// and widening as the air thins, with blackbody-white shock diamonds low down, in a
    /// halo that pales the sky around it. On the pad it stops at the ground and splashes
    /// sideways along it (the flame trench is off screen). The upper stage's vacuum plume
    /// is a short opaque core and then thin gas, drawn as braille dots whose density
    /// follows its heat, so the stars show through the fan.
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
        for i in 2..=(plen * 2.0).floor() as i32 {
            let u = f64::from(i) / 2.0;
            // In [0, 1]: past the tip every weight below would change sign.
            let f = u / plen;
            // At least three pixels across at the nozzle, whatever the body's width.
            let half = (0.5 * width).max(1.0) * (1.0 + (spread - 1.0) * f.powf(0.6));
            // In air, a halo ~1.5 px wide that shrinks down the flame.
            let halo = if vacuum { 0.0 } else { 1.5 * (1.0 - f) };
            // Shock diamonds: short lozenges on the axis, a pixel or more inside the flame's
            // edge, tapering to a point at each end of their window.
            let lozenge = diamonds.map_or(0.0, |d| {
                let k = (f / d).fract();
                let dk = k.min(1.0 - k) / 0.18;
                if f < 0.75 && dk < 1.0 {
                    (half - 1.0) * (1.0 - dk)
                } else {
                    0.0
                }
            });
            let n = (4.0 * (half + halo)).ceil() as i32;
            for j in -n / 2..=n / 2 {
                let v = f64::from(j) / 2.0;
                let (xf, yf) = at(u, v);
                let (x, y) = (xf.floor() as i32, yf.floor() as i32);
                let a = v.abs();
                let edge = a / half;
                let heat = (1.0 - f) * (1.0 - 0.5 * edge * edge);
                if y >= ground {
                    if a <= half && heat >= 0.08 {
                        let from = xf - bx + v.signum() * (0.5 * width + 2.0);
                        self.splash(bx, from, y - ground, ground - 1, heat);
                    }
                    continue;
                }
                let ink = if a > half {
                    if halo <= 0.0 {
                        continue;
                    }
                    let glow = (1.0 - f) * (1.0 - (a - half) / halo);
                    match self.halo(x, y, glow) {
                        Some(ink) => ink,
                        None => continue,
                    }
                } else if heat < 0.08 {
                    // Ragged, cooling edges: the faintest gas is left to the sky.
                    continue;
                } else if f < 0.25 && a <= 0.35 * half {
                    ramp(Ramp::Ember, 1.0)
                } else if vacuum {
                    if f < VACUUM_CORE && a <= 0.5 * half {
                        ramp(Ramp::Ember, 0.7 + 0.3 * heat)
                    } else {
                        // Thin gas: a dot per sample by a chance that follows the heat.
                        let (dx, dy) = ((2.0 * xf).floor() as i32, (2.0 * yf).floor() as i32);
                        if hash01(frame ^ cell_key(dx, dy)) < 0.8 * heat.sqrt() {
                            self.dots.set(dx, dy, ramp(Ramp::Ember, 0.5 + 0.45 * heat));
                        }
                        continue;
                    }
                } else if a < lozenge {
                    blackbody(6000.0)
                } else {
                    ramp(Ramp::Ember, 0.40 + 0.55 * heat)
                };
                self.px.set(x, y, ink);
            }
        }
        // The bell, over the flame's first pixel: a dark band between the white body and
        // the white-hot core, so the two never merge.
        let n = (width.ceil() as i32).max(1);
        for u in [0.5, 1.0] {
            for j in 0..n {
                let (x, y) = at(u, f64::from(j) - f64::from(n - 1) / 2.0);
                self.px
                    .set(x.floor() as i32, y.floor() as i32, Ink::new(Role::Dim, 70));
            }
        }
    }

    /// Flame that reaches the ground `depth` pixels below its top row, `from` pixels off the
    /// axis at column `axis` (already clear of the body), turned sideways onto `row`, the
    /// sky row just above the ground: it runs out along the ground, white-hot where it
    /// lands, cooling and curling up into the steam as it goes (ROC-A6).
    fn splash(&mut self, axis: f64, from: f64, depth: i32, row: i32, heat: f64) {
        let run = 0.6 * f64::from(depth);
        let cool = heat * (1.0 - run / (run + 12.0));
        if cool < 0.08 {
            return;
        }
        let ink = if cool > 0.8 {
            ramp(Ramp::Ember, 1.0)
        } else {
            ramp(Ramp::Ember, 0.40 + 0.55 * cool)
        };
        let side = if from < 0.0 { -1.0 } else { 1.0 };
        let out = (axis + from + side * run).floor() as i32;
        let row = row - (run / 5.0) as i32;
        self.px.set(out, row, ink);
        // Two pixels thick while it is hot, one as it thins out.
        if cool > 0.35 {
            self.px.set(out, row - 1, ink);
        }
    }

    /// The flame's halo at pixel `(x, y)` with strength `glow` (≤ 1): the sky or smoke there
    /// paled toward white, never below what it already is, so overlapping halo samples do
    /// not add up; over black space a faint warm glow; `None` where it leaves the pixel.
    fn halo(&self, x: i32, y: i32, glow: f64) -> Option<Ink> {
        if glow <= 0.0 {
            return None;
        }
        let under = self.px.get(x, y)?;
        match under.role {
            Role::Ramp(Ramp::Ice) => {
                let (sky, _) = backdrop(&self.cam, x, y);
                let base = if sky.role == under.role {
                    sky.level
                } else {
                    under.level
                };
                let s = f64::from(base) / 255.0;
                let lift = ramp(Ramp::Ice, s + (0.95 - s) * 0.6 * glow);
                Some(if lift.level > under.level {
                    lift
                } else {
                    under
                })
            }
            Role::Background if glow > 0.2 => Some(ramp(Ramp::Ember, 0.15 + 0.3 * glow)),
            _ => None,
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

/// The sky or ground behind pixel `(px, py)`, and whether it is sky (ROC-A5).
///
/// The sky's blue dims with the air above each pixel and pales toward the horizon, where
/// the eye looks through more air (a Rayleigh-like haze, by the pixel's height above the
/// ground line as a share of the frame). The ground curves away with the Earth: pad,
/// scrub, beach and the Atlantic downrange. Over the sea the limb is anti-aliased (the edge
/// pixel mixes sky and sea by its share of each) and the coast is dithered over a few
/// pixels, so the curved horizon does not step.
fn backdrop(cam: &Camera, px: i32, py: i32) -> (Ink, bool) {
    let x = cam.world_x(f64::from(px) + 0.5);
    let ground_h = -x * x / (2.0 * RE);
    let ground_py = cam.px(x, ground_h).1;
    // Share of this pixel below the ground line.
    let cover = (f64::from(py) + 1.0 - ground_py).clamp(0.0, 1.0);
    let sky = || {
        let alt = cam.world_h(f64::from(py) + 0.5);
        let rise = ((ground_py - f64::from(py) - 0.5) / cam.rows).max(0.0);
        let haze = SKY_ZENITH + SKY_HORIZON * (-rise / SKY_FALL).exp();
        // The Sun low in the east (ROC-A10): a white disc at a fixed place on screen (at
        // infinity it does not move as the camera climbs) in a glare that brightens the
        // blue round it, fading with the air as the sky does.
        let (sx, sy) = (SUN_AT.0 * cam.rows, SUN_AT.1 * cam.rows);
        let d = (f64::from(px) + 0.5 - sx).hypot(f64::from(py) + 0.5 - sy) / cam.rows;
        if d < SUN_R {
            return Ink::new(Role::Ramp(Ramp::Ice), 255);
        }
        let glare = SUN_GLARE * (-(d / SUN_GLOW_R).powi(2)).exp();
        // A smooth ramp down to the last visible level (CORE-R66), held there solid over
        // the next half of the fall, then black: the air ends on its faintest blue, with
        // no speckle (ROC-A5).
        let level = 255.0 * sky_level(alt) * (haze + glare);
        if level >= ink::MIN_VISIBLE_LEVEL {
            Ink::new(Role::Ramp(Ramp::Ice), ink::level(level))
        } else if level > 0.5 * ink::MIN_VISIBLE_LEVEL {
            Ink::new(Role::Ramp(Ramp::Ice), ink::level(ink::MIN_VISIBLE_LEVEL))
        } else {
            Ink::BLANK
        }
    };
    if cover == 0.0 {
        return (sky(), true);
    }
    let n = noise(px, py);
    // The coast blends over ±3 pixels.
    let sea = (x - COAST) * cam.k / 6.0 + 0.5 > n;
    let ground = if sea {
        ramp(Ramp::Ice, 0.22)
    } else if (x - COAST + 30.0).abs() < 30.0 {
        ramp(Ramp::Ember, 0.40)
    } else if x.abs() < 45.0 && f64::from(py) - ground_py < 4.0 * cam.k {
        Ink::new(Role::Structure, 150)
    } else {
        ramp(Ramp::Aurora, 0.14)
    };
    if cover == 1.0 {
        return (ground, false);
    }
    // The limb pixel: the sea mixes with the sky in one ramp; land, which cannot, rounds.
    match (sky(), ground.role) {
        (s, Role::Ramp(Ramp::Ice)) if s.role == Role::Ramp(Ramp::Ice) => {
            let (a, b) = (f64::from(s.level), f64::from(ground.level));
            (ramp(Ramp::Ice, ease::lerp(a, b, cover) / 255.0), false)
        }
        (s, _) if cover < 0.5 => (s, true),
        _ => (ground, false),
    }
}

/// The top of the air, where the sky's `level` has fallen below the last visible one:
/// A stable per-pixel threshold in [0, 1): white noise, without the regular rows of an
/// ordered dither.
fn noise(x: i32, y: i32) -> f64 {
    hash01(cell_key(x, y))
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

/// A [`draw::label`] centred on row `y`: text on a dark chip.
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
        self.tone = Tone::asinh(0.05, 1.2);
        let dot = self.dots.dot_size();
        let n = size.area() / CELLS_PER_STAR;
        self.stars.clear();
        self.stars.extend((0..n).map(|_| {
            (
                ctx.rng.random_range(0..i32::from(dot.width())),
                ctx.rng.random_range(0..i32::from(dot.height())),
                ctx.rng.random_range(40..170u8),
            )
        }));
        self.ready = true;
        self.rasterise();
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
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        if !self.ready {
            return;
        }
        self.dots.blit(canvas);
        self.px.blit(canvas, |x, y, top, bottom| {
            // ASCII: the bare sky stays blank and the bare ground is `=`; only what stands
            // in front of them (smoke, flame, vehicle, tower) gets a mark.
            let (sky, is_sky) = backdrop(&self.cam, x, 2 * y + 1);
            if bottom == sky && top == backdrop(&self.cam, x, 2 * y).0 {
                return (!is_sky).then_some(('=', sky));
            }
            let ink = if bottom == sky || bottom.role == Role::Background {
                top
            } else {
                bottom
            };
            match ink.role {
                Role::Background => None,
                Role::Ramp(Ramp::Ice) => Some(('o', ink)),
                Role::Blackbody => Some(('#', ink)),
                Role::Star => Some(('|', ink)),
                Role::Ramp(Ramp::Ember) => Some(('*', ink)),
                _ => Some(('=', ink)),
            }
        });
        // Text on dark chips (its cells keep no background), legible over any sky in
        // every theme (ROC-A9).
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
    /// Countdown, event callouts, telemetry and the Δv bar.
    fn draw_text(&self, canvas: &mut Canvas) {
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        let met = self.met();
        let s = self.ascent.at(met);
        let ink = Ink::new(Role::Text, 235);
        let y = h / 5;
        if met < 0.0 {
            // T−3 … T−1 over the last seconds, the clock the whole way.
            let n = clock_secs(met);
            let mut buf = StackText::<8>::new();
            let _ = write!(buf, "T-{n}");
            label_centred(canvas, y, buf.as_str(), ink);
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
                label_centred(canvas, y, callout, ink);
            }
        }
        if w >= 44 {
            self.telemetry(canvas, met, &s);
        }
        if self.size.width() >= BAR_MIN_W {
            self.dv_bar(canvas, &s);
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
            draw::label(canvas, 0, 0, buf.as_str(), Ink::new(Role::Text, 220));
        }
    }

    /// Top-right Δv bar: of the stack's ideal Δv, how much became speed, and how much was
    /// paid to gravity, drag and steering (ROC-A7).
    fn dv_bar(&self, canvas: &mut Canvas, s: &Sample) {
        let w = i32::from(self.size.width());
        let x0 = w - BAR_W - 2;
        draw::label(canvas, x0 - 1, 1, "delta-v", Ink::new(Role::Text, 200));
        let speed = s.y[IDEAL] - s.y[GRAV] - s.y[DRAG] - s.y[STEER];
        let parts = [
            (speed, ramp(Ramp::Ember, 0.8)),
            (s.y[GRAV], ramp(Ramp::Plasma, 0.6)),
            (s.y[DRAG], ramp(Ramp::Ice, 0.7)),
            (s.y[STEER], ramp(Ramp::Aurora, 0.7)),
        ];
        let full = Glyph::new('█', '#');
        let empty = Glyph::new('░', '.');
        let mut x = 0;
        let mut acc = 0.0;
        for (v, ink) in parts {
            acc += v.max(0.0);
            let end = ((acc / self.budget) * f64::from(BAR_W))
                .round()
                .min(f64::from(BAR_W)) as i32;
            while x < end {
                canvas.put(x0 + x, 2, full, ink);
                x += 1;
            }
        }
        while x < BAR_W {
            canvas.put(x0 + x, 2, empty, Ink::new(Role::Dim, 60));
            x += 1;
        }
        // Legend, coloured like the bar.
        let mut lx = x0;
        for (name, (_, ink)) in ["speed", "gravity", "drag", "steer"].iter().zip(parts) {
            draw::label(canvas, lx - 1, 3, name, ink);
            lx += name.len() as i32 + 1;
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
        let mut before_ignition = true;
        for _ in 0..(70 * 60) {
            let n0 = g.steam.len();
            g.update(SIM_DT, &mut ctx);
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
    fn roc_a5_sky_darkens_with_altitude_to_black() {
        let mut prev = f64::INFINITY;
        for i in 0..200 {
            let v = sky_level(f64::from(i) * 500.0);
            assert!(v < prev);
            prev = v;
        }
        assert!(sky_level(0.0) > 0.5);
        assert!(sky_level(60e3) < 0.03, "black by 60 km");
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
        let total = g.timeline.total();
        let mut t = 0.0;
        while t < total + 1.0 {
            g.update(0.25, &mut ctx);
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
        while g.t < t {
            g.update(SIM_DT, &mut ctx);
        }
        g
    }

    #[test]
    fn roc_a5_the_day_sky_pales_toward_the_horizon() {
        let g = scene_at(Size::new(120, 40), 2.0);
        // Column 4: open sky from the top down to the ground.
        let top = g.px.get(4, 0).unwrap();
        let low = (0..80)
            .map(|y| g.px.get(4, y).unwrap())
            .take_while(|i| i.role == Role::Ramp(Ramp::Ice))
            .last()
            .unwrap();
        assert_eq!(top.role, Role::Ramp(Ramp::Ice));
        assert!(
            low.level > top.level + 40,
            "zenith {top:?}, horizon {low:?}"
        );
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
    }

    #[test]
    fn roc_a6_the_plume_is_white_hot_at_the_nozzle() {
        let g = scene_at(Size::new(120, 40), COUNTDOWN + 1.5);
        let size = g.px.pixel_size();
        let hottest = (0..i32::from(size.height()))
            .flat_map(|y| (0..i32::from(size.width())).map(move |x| (x, y)))
            .filter_map(|(x, y)| g.px.get(x, y))
            .any(|i| i == ramp(Ramp::Ember, 1.0));
        assert!(hottest, "no white-hot core");
    }

    #[test]
    fn roc_a10_a_low_sun_glares_in_the_morning_sky() {
        let g = scene_at(Size::new(120, 40), 5.0);
        let rows = g.cam.rows;
        let (sx, sy) = (SUN_AT.0 * rows, SUN_AT.1 * rows);
        let at = |x: f64, y: f64| backdrop(&g.cam, x as i32, y as i32).0;
        assert_eq!(at(sx, sy), Ink::new(Role::Ramp(Ramp::Ice), 255));
        // The glare brightens the blue round the disc, falling off with distance.
        let near = at(sx + 0.08 * rows, sy).level;
        let mid = at(sx + 0.2 * rows, sy).level;
        let far = at(sx + 0.6 * rows, sy).level;
        assert!(near > mid && mid > far, "{near} {mid} {far}");
        assert_eq!(at(sx, sy + 0.1 * rows).role, Role::Ramp(Ramp::Ice));
    }

    #[test]
    fn roc_a5_the_sky_fades_to_black_as_a_smooth_ramp() {
        // High on the climb the whole fade from blue to black is in frame.
        for t in [
            COUNTDOWN + LIFTOFF + CLIMB + 4.0,
            COUNTDOWN + LIFTOFF + CLIMB + 18.0,
        ] {
            let g = scene_at(Size::new(200, 50), t);
            let size = g.px.pixel_size();
            let (pw, ph) = (i32::from(size.width()), i32::from(size.height()));
            let mut blanks = 0;
            let faintest = ink::level(ink::MIN_VISIBLE_LEVEL);
            // Pixel rows where the faintest blue and black mix: the stippled edge.
            let mut mixed = vec![(false, false); ph as usize];
            for x in (pw * 3 / 4)..pw {
                // Up the column from the ground: never brighter, never a jump, except the
                // faintest blue stippled into the black at the top of the air.
                let mut prev: Option<u8> = None;
                for y in (0..ph).rev() {
                    let (sky, is_sky) = backdrop(&g.cam, x, y);
                    let ink = g.px.get(x, y).unwrap();
                    if !is_sky || ink != sky {
                        prev = None;
                        continue;
                    }
                    let level = if ink == Ink::BLANK { 0 } else { ink.level };
                    blanks += usize::from(ink == Ink::BLANK);
                    if level == 0 {
                        mixed[y as usize].0 = true;
                    } else if level <= faintest {
                        mixed[y as usize].1 = true;
                    }
                    // In the fade to black (the last 40 levels) no step bigger than the
                    // last visible level (CORE-R66), and no speck: once black going up,
                    // the air stays black.
                    if let Some(p) = prev {
                        assert!(p > 0 || level == 0, "t {t}: speck of blue at {x},{y}");
                        assert!(
                            level <= p || level <= faintest,
                            "t {t}: column {x} brightens at {y}"
                        );
                        let step = p.abs_diff(level);
                        assert!(p > 40 || step <= 13, "t {t}: a step of {step} at {x},{y}");
                    }
                    prev = Some(level);
                }
            }
            assert!(blanks > 100, "t {t}: black space in frame");
            let _ = mixed;
        }
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
    fn roc_a6_the_hot_core_never_touches_the_white_body() {
        for t in [
            COUNTDOWN + 1.5,
            COUNTDOWN + LIFTOFF + 4.0,
            COUNTDOWN + LIFTOFF + CLIMB + 18.0,
        ] {
            let g = scene_at(Size::new(120, 40), t);
            let size = g.px.pixel_size();
            let body = Ink::new(Role::Star, 235);
            let mut cores = 0;
            for y in 0..i32::from(size.height()) {
                for x in 0..i32::from(size.width()) {
                    if g.px.get(x, y) != Some(ramp(Ramp::Ember, 1.0)) {
                        continue;
                    }
                    cores += 1;
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        assert_ne!(g.px.get(x + dx, y + dy), Some(body), "t {t}: {x},{y}");
                    }
                }
            }
            assert!(cores > 0, "t {t}: no white-hot core");
        }
    }

    #[test]
    fn roc_a6_the_spent_stage_keeps_clear_of_the_plume() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(1, size);
        let mut g = RocketLaunch::new();
        g.resize(size, &mut ctx);
        let flame = |i: Ink| matches!(i.role, Role::Blackbody | Role::Ramp(Ramp::Ember));
        let spent = Ink::new(Role::Structure, 120);
        let (w, h) = (80, 48);
        for step in 0..(60.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            if step % 10 != 0 || g.met() < SES {
                continue;
            }
            for y in 0..h {
                for x in 0..w {
                    if g.px.get(x, y) != Some(spent) {
                        continue;
                    }
                    for dy in -2..=2 {
                        for dx in -2..=2 {
                            let near = g.px.get(x + dx, y + dy).is_some_and(flame);
                            assert!(!near, "MET {:.0}: stage at {x},{y} in the plume", g.met());
                        }
                    }
                }
            }
        }
    }

    fn flame(i: Ink) -> bool {
        matches!(i.role, Role::Blackbody | Role::Ramp(Ramp::Ember))
    }

    #[test]
    fn roc_a6_the_vacuum_plume_is_sparse_gas_behind_a_short_core() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let g = scene_at(size, COUNTDOWN + LIFTOFF + CLIMB + STAGING + UPPER - 1.0);
            assert!(g.met() > SES + 20.0, "the upper stage is firing");
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let pixels = g.px.pixel_size();
            let opaque = (0..i32::from(pixels.height()))
                .flat_map(|y| (0..i32::from(pixels.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| g.px.get(x, y).is_some_and(flame))
                .count();
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
            // Even its faint edge is a warm yellow-orange, never the brown bottom of Ember.
            let floor = ramp(Ramp::Ember, 0.5).level;
            for ink in gas {
                assert!(ink.level >= floor, "{size:?}: {ink:?}");
            }
        }
    }

    #[test]
    fn roc_a6_the_pad_flame_stops_at_the_ground_and_splashes_sideways() {
        let size = Size::new(120, 40);
        let g = scene_at(size, COUNTDOWN - 1.0);
        let pixels = g.px.pixel_size();
        let pad = g.cam.px(0.0, 0.0).0;
        let (mut splash, mut any) = (0, 0);
        for y in 0..i32::from(pixels.height()) {
            for x in 0..i32::from(pixels.width()) {
                if !g.px.get(x, y).is_some_and(flame) {
                    continue;
                }
                any += 1;
                let (_, is_sky) = backdrop(&g.cam, x, y);
                assert!(is_sky, "flame below the ground at {x},{y}");
                splash += usize::from((f64::from(x) - pad).abs() > 4.0);
            }
        }
        assert!(any > 0 && splash >= 4, "flame {any}, splash {splash}");
    }

    #[test]
    fn roc_a6_shock_diamonds_are_lozenges_inside_the_core() {
        let g = scene_at(Size::new(200, 50), COUNTDOWN + LIFTOFF + 4.0);
        assert!(diamond_spacing(pressure_ratio(g.ascent.at(g.met()).y[H])).is_some());
        let pixels = g.px.pixel_size();
        let mut diamonds = 0;
        for y in 0..i32::from(pixels.height()) {
            for x in 0..i32::from(pixels.width()) {
                if g.px.get(x, y).is_some_and(|i| i.role == Role::Blackbody) {
                    diamonds += 1;
                    // Flame on both sides: narrower than the flame, not a full-width bar.
                    for dx in [-1, 1] {
                        let side = g.px.get(x + dx, y);
                        assert!(
                            side.is_some_and(flame),
                            "diamond at {x},{y} reaches the edge"
                        );
                    }
                }
            }
        }
        assert!(diamonds > 0, "no shock diamonds low down");
    }

    #[test]
    fn roc_a9_text_sits_on_dark_chips_and_the_legend_names_every_loss() {
        let size = Size::new(160, 48);
        let g = scene_at(size, 2.0);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let rows = canvas.to_lines();
        assert!(rows[0].trim_start().starts_with("T-"), "{}", rows[0]);
        for x in 0..20 {
            let cell = canvas.get(x, 0).unwrap();
            assert_eq!(cell.bg, Ink::BLANK, "telemetry cell {x} over the sky");
        }
        for name in ["speed", "gravity", "drag", "steer"] {
            assert!(rows[3].contains(name), "legend lacks {name}: {}", rows[3]);
        }
    }

    #[test]
    fn roc_a5_a6_palette_blue_sky_blackbody_flame_and_black_space() {
        let mut stage = Stage::new(build(), 42, Size::new(160, 48), GlyphMode::Unicode);
        let mut seen_blue = false;
        let mut seen_flame = false;
        let mut seen_black_top = false;
        for _ in 0..62 {
            for _ in 0..60 {
                stage.step(SIM_DT);
            }
            let f = stage.render();
            let has = |r: Role| f.cells().iter().any(|c| c.ink.role == r || c.bg.role == r);
            seen_blue |= has(Role::Ramp(Ramp::Ice));
            seen_flame |= has(Role::Blackbody);
            seen_black_top |= f.get(80, 5).is_some_and(|c| c.bg.role == Role::Background);
        }
        assert!(seen_blue && seen_flame && seen_black_top);
    }
}
