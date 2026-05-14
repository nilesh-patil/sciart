//! Apollo 11's powered descent to Tranquility Base (SPEC-002 `moon_landing`).
//!
//! The Lunar Module is a 2-D point mass with real Apollo numbers: descent-engine thrust
//! `T ≤ 45.04 kN` (throttle floor 10 %) along the vehicle's attitude, `ṁ = −T/(I_sp g₀)`
//! with `I_sp = 311 s`, lunar `g = 1.62 m/s²`, integrated with RK4 (`ode::integrate`,
//! ML-A2). The script compresses the last minute of Apollo 11 into ≈ 21 s:
//!
//! - **P64** (approach): E-guidance — the acceleration profile linear in time that
//!   reaches a gate `H_GATE` above the site with a gentle sink rate at a fixed time
//!   (the Apollo "quadratic guidance" family, solved in closed form every substep);
//! - **P66** (rate of descent): the sink rate follows an exponential profile down to
//!   `V_TD` at touchdown while horizontal velocity is nulled, so it lands softly from
//!   any gate state the seeded start set produces (ML-A1).
//!
//! Everything world-side is in metres; `render` maps metres to rows with a zoom `k`
//! (rows per metre) that grows as the LM descends, so the approach starts wide and ends
//! with the sprite roughly to scale. The terrain is a pure function of x (rolling mare
//! plus a seeded crater field, ML-A3) and the story runs on a looping `Timeline`
//! (ML-A4): descent → touchdown → flag → earth → a new seeded approach. The Earth hangs
//! fixed high in the sky throughout: from Tranquility Base it never rises or sets.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use rand::Rng;
use sciart_core::draw;
use sciart_core::ease;
use sciart_core::geom::ASPECT;
use sciart_core::ink::{self, ramp};
use sciart_core::ode;
use sciart_core::particles::plot;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::text::StackText;

use crate::satellite_orbit::Globe;

pub static META: Meta = Meta {
    name: "moon_landing",
    category: Category::Voyages,
    description: "Apollo 11 powered descent to Tranquility Base",
    title: "Apollo 11 · powered descent",
    captions: &CAPTIONS,
};

/// The title's caption per act (GEN-R12), so it never names a phase the LM has left:
/// under power until the footpads touch, then on the ground, then on the surface.
const CAPTIONS: [&str; 3] = ["powered descent", "touchdown", "Tranquility Base"];

pub fn build() -> Box<dyn Generator> {
    Box::new(MoonLanding::new())
}

// ---------------------------------------------------------------------------------------
// Vehicle and environment (SI units: m, s, kg, N).
// ---------------------------------------------------------------------------------------

/// Lunar surface gravity.
const G_MOON: f64 = 1.62;
/// Standard gravity, which defines specific impulse.
const G0: f64 = 9.806_65;
/// LM descent-engine specific impulse (s).
const ISP: f64 = 311.0;
/// Effective exhaust velocity `I_sp g₀`.
const VE: f64 = ISP * G0;
/// Descent-engine full thrust (10 125 lbf).
const T_MAX: f64 = 45_040.0;
/// Deepest throttle setting the engine allows.
const THROTTLE_MIN: f64 = 0.10;
/// LM mass without descent propellant (ascent stage, crew, dry descent stage).
const M_DRY: f64 = 6_900.0;
/// Full descent-stage propellant load; FUEL % is relative to this.
const PROP_FULL: f64 = 8_200.0;
/// Lunar-surface sensing probes hang this far below the footpads: "CONTACT LIGHT".
const PROBE: f64 = 1.7;
/// Half the footpad span seen from the side.
const FOOT_HALF: f64 = 4.2;

// ---------------------------------------------------------------------------------------
// Guidance.
// ---------------------------------------------------------------------------------------

/// P64 targets the gate at this time after the scene's descent starts.
const T64: f64 = 15.0;
/// P64 hands over to P66 with this much time-to-go left (its gains grow as 1/t_go²).
const T_SWITCH: f64 = 1.0;
/// Footpad height above the site at the P64 gate.
const H_GATE: f64 = 8.0;
/// Sink rate at the gate (m/s).
const V_GATE: f64 = 2.5;
/// Sink rate the P66 profile reaches at zero height (m/s).
const V_TD: f64 = 0.6;
/// P66 sink-rate cap if the LM arrives above the gate.
const V_P66_MAX: f64 = 4.0;
/// P66 sink-rate loop gain (1/s).
const K_V: f64 = 1.5;
/// P66 horizontal velocity / position gains (critically damped: K_H² = 4 K_P).
const K_H: f64 = 0.8;
const K_P: f64 = 0.16;
/// Tilt limits from vertical (rad).
const TILT_P64: f64 = 45.0 * PI / 180.0;
const TILT_P66: f64 = 15.0 * PI / 180.0;
/// Attitude slew-rate limit (rad/s), the RCS authority.
const SLEW: f64 = 20.0 * PI / 180.0;
/// Largest physics substep (s); the guidance runs every substep.
const H_MAX: f64 = 1.0 / 120.0;

// ---------------------------------------------------------------------------------------
// Timeline (simulated seconds) — ML-A4.
// ---------------------------------------------------------------------------------------

const DESCENT: f64 = 23.0;
const PHASES: [(&str, f64); 4] = [
    ("descent", DESCENT),
    ("touchdown", 4.0),
    ("flag", 5.0),
    ("earth", 4.0),
];
const PH_DESCENT: usize = 0;
const PH_TOUCHDOWN: usize = 1;
const PH_FLAG: usize = 2;
const PH_EARTH: usize = 3;

// ---------------------------------------------------------------------------------------
// Physics.
// ---------------------------------------------------------------------------------------

/// Point-mass LM: position (m; `z` is the footpad height above the datum), velocity,
/// mass, attitude `theta` (rad from vertical, positive tilts the thrust towards +x) and
/// the thrust currently commanded (N).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Lm {
    pub x: f64,
    pub z: f64,
    pub vx: f64,
    pub vz: f64,
    pub m: f64,
    pub theta: f64,
    pub thrust: f64,
}

impl Lm {
    fn fuel(&self) -> f64 {
        (self.m - M_DRY).max(0.0)
    }
}

/// Fly `h` seconds with the thrust and attitude held (ML-A2).
///
/// `y = [x, z, vx, vz, m]`, `ẏ = [vx, vz, T sinθ/m, T cosθ/m − g, −T/(I_sp g₀)]`, one RK4
/// step per `H_MAX`. The burn is cut short so the mass never drops below `M_DRY`.
pub(crate) fn fly(lm: &mut Lm, h: f64) {
    // Also rejects NaN.
    if h.is_nan() || h <= 0.0 {
        return;
    }
    let (thrust, (s, c)) = (lm.thrust.max(0.0), lm.theta.sin_cos_det());
    // Burn only as long as there is propellant; coast for the rest of the step.
    let burn = if thrust > 0.0 {
        (lm.fuel() * VE / thrust).min(h)
    } else {
        0.0
    };
    let deriv = |f: f64, y: &[f64; 5]| [y[2], y[3], f * s / y[4], f * c / y[4] - G_MOON, -f / VE];
    let mut y = [lm.x, lm.z, lm.vx, lm.vz, lm.m];
    if burn > 0.0 {
        ode::integrate(&mut y, 0.0, burn, H_MAX, |_, y| deriv(thrust, y));
    }
    let coast = h - burn;
    if coast > 0.0 {
        ode::integrate(&mut y, 0.0, coast, H_MAX, |_, y| deriv(0.0, y));
        lm.thrust = 0.0;
    }
    [lm.x, lm.z, lm.vx, lm.vz] = [y[0], y[1], y[2], y[3]];
    // Pin burn-out to the dry mass exactly (RK4 integrates the linear mass flow exactly
    // anyway, up to rounding).
    lm.m = if coast > 0.0 { M_DRY } else { y[4].max(M_DRY) };
}

/// E-guidance along one axis: the initial value `a₀` of the acceleration profile
/// `a(τ) = a₀ + j τ` that takes `(r, v)` to `(r_f, v_f)` in exactly `t_go` seconds,
/// `a₀ = [6 (r_f − r) − 2 t_go (2 v + v_f)] / t_go²`.
pub(crate) fn e_guidance(r: f64, v: f64, r_f: f64, v_f: f64, t_go: f64) -> f64 {
    // The gains grow as 1/t_go²; P66 takes over before this floor matters.
    let t = t_go.max(T_SWITCH);
    (6.0 * (r_f - r) - 2.0 * t * (2.0 * v + v_f)) / (t * t)
}

/// Guidance program in control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Program {
    P64,
    P66,
    Landed,
}

/// Commanded acceleration `(a_x, a_z)` (excluding gravity) and the tilt limit.
///
/// P64 aims for the gate `(site_x, site_ground + H_GATE)` with velocity `(0, −V_GATE)`
/// at `T64`; P66 flies the sink-rate profile `v_ref(h) = −(V_TD + h (V_GATE − V_TD) /
/// H_GATE)` (with its derivative as feed-forward, so tracking is exact when the throttle
/// is not saturated) and nulls horizontal motion over the site.
pub(crate) fn guidance(lm: &Lm, t: f64, site_x: f64, site_ground: f64) -> (f64, f64, f64) {
    match program_at(t) {
        Program::P64 | Program::Landed => {
            let t_go = T64 - t;
            let ax = e_guidance(lm.x, lm.vx, site_x, 0.0, t_go);
            let az = e_guidance(lm.z, lm.vz, site_ground + H_GATE, -V_GATE, t_go);
            (ax, az, TILT_P64)
        }
        Program::P66 => {
            let slope = (V_GATE - V_TD) / H_GATE;
            let h = (lm.z - site_ground).max(0.0);
            let raw = V_TD + slope * h;
            let (v_ref, ff) = if raw < V_P66_MAX {
                (-raw, -slope * lm.vz)
            } else {
                (-V_P66_MAX, 0.0)
            };
            let az = K_V * (v_ref - lm.vz) + ff;
            let ax = -K_H * lm.vx - K_P * (lm.x - site_x);
            (ax, az, TILT_P66)
        }
    }
}

/// Which program flies at descent time `t`.
pub(crate) fn program_at(t: f64) -> Program {
    if t < T64 - T_SWITCH {
        Program::P64
    } else {
        Program::P66
    }
}

/// Turn a commanded acceleration into attitude (slew-limited over `h`) and thrust:
/// the thrust is the projection of the needed force on the body axis, clamped to the
/// throttle range, so a lagging attitude never over-drives the vertical channel.
pub(crate) fn steer(lm: &mut Lm, a: (f64, f64), tilt_max: f64, h: f64) {
    let (ax, az) = (a.0, a.1 + G_MOON);
    let want = ax.atan2(az).clamp(-tilt_max, tilt_max);
    let d = (want - lm.theta).clamp(-SLEW * h, SLEW * h);
    lm.theta += d;
    let (s, c) = lm.theta.sin_cos_det();
    let f = lm.m * (ax * s + az * c);
    lm.thrust = if lm.fuel() > 0.0 {
        f.clamp(THROTTLE_MIN * T_MAX, T_MAX)
    } else {
        0.0
    };
}

/// One crater: centre, radius, bowl depth, rim height (m).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Crater {
    c: f64,
    r: f64,
    depth: f64,
    rim: f64,
}

impl Crater {
    /// Parabolic bowl inside the rim, Gaussian ejecta apron outside; continuous at r = 1.
    fn height(&self, x: f64) -> f64 {
        let r = (x - self.c).abs() / self.r;
        if r < 1.0 {
            self.depth * (r * r - 1.0) + self.rim
        } else if r < 2.5 {
            let u = (r - 1.0) / 0.35;
            self.rim * (-u * u).exp()
        } else {
            0.0
        }
    }
}

const N_CRATERS: usize = 18;
/// Craters on the plain in front of the skyline (ML-A8).
const N_PITS: usize = 16;

/// A crater out on the plain: world x, how far toward the viewer (0 at the skyline, 1 at
/// the bottom of the frame) and radius (m).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Pit {
    x: f64,
    v: f64,
    r: f64,
}
/// Craters are scattered over this distance either side of the site.
const CRATER_SPAN: f64 = 460.0;
/// Clear radius around the landing site (plus each crater's apron).
const SITE_CLEAR: f64 = 14.0;

/// Seeded mare surface; `height` is a pure function of x (ML-A3).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Terrain {
    craters: [Crater; N_CRATERS],
    /// Rolling swell: (amplitude m, wavenumber 1/m, phase).
    waves: [(f64, f64, f64); 2],
    pits: [Pit; N_PITS],
    pub site_x: f64,
}

impl Terrain {
    /// A crater field with one large crater ("West crater") on the approach side.
    pub(crate) fn seeded(rng: &mut impl Rng, side: f64) -> Self {
        let site_x = 0.0;
        // A crater of radius r must keep its ejecta apron (≈ 1.8 r) off the site.
        let keep_out = |r: f64| 1.8 * r + SITE_CLEAR;
        let mut craters = [Crater::default(); N_CRATERS];
        for (i, k) in craters.iter_mut().enumerate() {
            let big = i == 0;
            let r = if big {
                rng.random_range(45.0..70.0)
            } else {
                // Many small craters, few large ones (a steep size distribution).
                let u: f64 = rng.random_range(0.04..1.0);
                (3.0 * u.powf(-0.8)).min(30.0)
            };
            let mut c = if big {
                // The big one lies under the approach path, as West crater did.
                site_x - side * (keep_out(r) + rng.random_range(5.0..40.0))
            } else {
                site_x + rng.random_range(-CRATER_SPAN..CRATER_SPAN)
            };
            if (c - site_x).abs() <= keep_out(r) {
                let dir = if c >= site_x { 1.0 } else { -1.0 };
                c = site_x + dir * (keep_out(r) + rng.random_range(0.0..30.0));
            }
            *k = Crater {
                c,
                r,
                depth: r * rng.random_range(0.2..0.4),
                rim: r * rng.random_range(0.05..0.1),
            };
        }
        let mut wave = |amp: std::ops::Range<f64>, wavelength: std::ops::Range<f64>| {
            (
                rng.random_range(amp),
                TAU / rng.random_range(wavelength),
                rng.random_range(0.0..TAU),
            )
        };
        let waves = [wave(0.8..2.0, 140.0..240.0), wave(0.2..0.6, 35.0..60.0)];
        let pits = std::array::from_fn(|_| {
            let u: f64 = rng.random_range(0.0..1.0);
            Pit {
                x: site_x + rng.random_range(-CRATER_SPAN..CRATER_SPAN),
                v: rng.random_range(0.1..0.9),
                r: 3.0 + 22.0 * u * u,
            }
        });
        Self {
            craters,
            waves,
            pits,
            site_x,
        }
    }

    /// Surface height (m) at world x.
    pub(crate) fn height(&self, x: f64) -> f64 {
        let mut h = 0.0;
        for &(a, k, p) in &self.waves {
            h += a * (k * x + p).sin_det();
        }
        for c in &self.craters {
            h += c.height(x);
        }
        h
    }

    /// Highest point under the footprint `[x − FOOT_HALF, x + FOOT_HALF]`: the LM rests
    /// on (and must never go below) this.
    pub(crate) fn ground(&self, x: f64) -> f64 {
        let mut g = f64::NEG_INFINITY;
        for i in 0..=8 {
            let s = x + FOOT_HALF * (f64::from(i) / 4.0 - 1.0);
            g = g.max(self.height(s));
        }
        g
    }
}

/// Contact conditions at footpad touchdown (ML-A1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Touchdown {
    pub t: f64,
    pub vx: f64,
    pub vz: f64,
    pub x: f64,
    pub fuel: f64,
}

/// The powered descent: LM state, clock and events.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Descent {
    pub lm: Lm,
    pub t: f64,
    /// Descent time of probe contact ("CONTACT LIGHT").
    pub contact: Option<f64>,
    pub touchdown: Option<Touchdown>,
    /// Ground height under the site, cached (the P64 target reference).
    site_ground: f64,
}

impl Descent {
    /// A seeded start inside the envelope P64 is sized for: range and height roughly
    /// what a constant deceleration would need in `T64`, ±15–30 % (ML-A1).
    pub(crate) fn seeded(rng: &mut impl Rng, terrain: &Terrain, side: f64) -> Self {
        let vx0 = rng.random_range(12.0..20.0);
        let range = 0.5 * T64 * vx0 * rng.random_range(0.85..1.15);
        let vz0 = rng.random_range(6.0..9.0);
        let drop = 0.5 * T64 * (vz0 + V_GATE) * rng.random_range(0.9..1.3);
        let prop = PROP_FULL * rng.random_range(0.06..0.09);
        let site_ground = terrain.ground(terrain.site_x);
        let mut lm = Lm {
            x: terrain.site_x - side * range,
            z: site_ground + H_GATE + drop,
            vx: side * vx0,
            vz: -vz0,
            m: M_DRY + prop,
            theta: 0.0,
            thrust: 0.0,
        };
        // Start already pointing where P64 wants, so there is no initial slew transient.
        let (ax, az, tilt) = guidance(&lm, 0.0, terrain.site_x, site_ground);
        lm.theta = ax.atan2(az + G_MOON).clamp(-tilt, tilt);
        steer(&mut lm, (ax, az), tilt, 0.0);
        Self {
            lm,
            t: 0.0,
            contact: None,
            touchdown: None,
            site_ground,
        }
    }

    pub(crate) fn program(&self) -> Program {
        if self.touchdown.is_some() {
            Program::Landed
        } else {
            program_at(self.t)
        }
    }

    /// Advance `h ≤ H_MAX` seconds: guide, fly, then detect probe and footpad contact.
    /// A footpad that reaches the ground is set on it exactly (ML-A3).
    pub(crate) fn step(&mut self, h: f64, terrain: &Terrain) {
        if self.touchdown.is_some() || h <= 0.0 {
            self.t += h.max(0.0);
            return;
        }
        let (ax, az, tilt) = guidance(&self.lm, self.t, terrain.site_x, self.site_ground);
        steer(&mut self.lm, (ax, az), tilt, h);
        fly(&mut self.lm, h);
        self.t += h;
        let g = terrain.ground(self.lm.x);
        if self.contact.is_none() && self.lm.z - g <= PROBE {
            self.contact = Some(self.t);
        }
        if self.lm.z <= g {
            self.land(g);
        }
    }

    fn land(&mut self, g: f64) {
        self.touchdown = Some(Touchdown {
            t: self.t,
            vx: self.lm.vx,
            vz: self.lm.vz,
            x: self.lm.x,
            fuel: self.lm.fuel(),
        });
        self.contact.get_or_insert(self.t);
        self.lm.z = g;
        self.lm.vx = 0.0;
        self.lm.vz = 0.0;
        self.lm.thrust = 0.0;
        self.lm.theta = 0.0;
    }

    /// Radar altitude of the footpads above the ground under them.
    fn altitude(&self, terrain: &Terrain) -> f64 {
        (self.lm.z - terrain.ground(self.lm.x)).max(0.0)
    }
}

// ---------------------------------------------------------------------------------------
// Dust: lunar regolith blown out by the plume. In vacuum there is no drag and no
// billowing — grains fly flat, ballistic sheets and fall straight back.
// ---------------------------------------------------------------------------------------

/// Dust is raised when the footpads are below this height (m).
const DUST_ALT: f64 = 30.0;
/// Peak emission rate (grains/s) at full scale.
const DUST_RATE: f64 = 420.0;
const DUST_MAX: usize = 700;
const DUST_LIFE: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Grain {
    x: f64,
    z: f64,
    vx: f64,
    vz: f64,
    age: f64,
}

/// Exact ballistic flight in lunar gravity.
fn ballistic(p: &mut Grain, h: f64) {
    p.x += p.vx * h;
    p.z += h * (p.vz - 0.5 * G_MOON * h);
    p.vz -= G_MOON * h;
    p.age += h;
}

// ---------------------------------------------------------------------------------------
// Light: a low Sun behind the approach, and the Earth overhead (ML-A5, ML-A6).
// ---------------------------------------------------------------------------------------

/// Unit vector toward the Sun in the (x, z) plane: `SUN_ELEV` up, behind an LM flying
/// toward `side` (Apollo 11 came in with the Sun at its back, so its shadow ran ahead).
pub(crate) fn sun_dir(side: f64) -> (f64, f64) {
    let (s, c) = SUN_ELEV.sin_cos_det();
    (-side * c, s)
}

/// Lambert light on ground of slope `dhdx` under the Sun `sun`: the cosine between the
/// profile's normal and the Sun, 0 when the slope turns its back (ML-A5).
pub(crate) fn slope_light(dhdx: f64, sun: (f64, f64)) -> f64 {
    ((sun.1 - dhdx * sun.0) / (1.0 + dhdx * dhdx).sqrt()).max(0.0)
}

/// Mark every sample of the profile `heights` (spacing `dx` m) that terrain nearer the
/// Sun hides from it (ML-A5): one sweep down-Sun, carrying the shadow line cast by the
/// highest point so far, which drops by `dx · tan(elevation)` per sample.
pub(crate) fn cast_shadows(heights: &[f64], dx: f64, sun: (f64, f64), out: &mut [bool]) {
    out.fill(false);
    if !(dx > 0.0 && sun.0 != 0.0) {
        return;
    }
    let drop = dx * sun.1 / sun.0.abs();
    let n = heights.len().min(out.len());
    let mut line = f64::NEG_INFINITY;
    for k in 0..n {
        // The Sun on the −x side lights from the left, so the sweep runs left to right.
        let i = if sun.0 < 0.0 { k } else { n - 1 - k };
        line -= drop;
        if heights[i] < line {
            out[i] = true;
        } else {
            line = heights[i];
        }
    }
}

/// Regolith at brightness `level` (0..255): below [`ink::MIN_VISIBLE_LEVEL`] it is the
/// terminal's black, since even `Structure`'s lowest level renders as a grey in every
/// theme (CORE-R66).
fn regolith(level: f64) -> Ink {
    if level < ink::MIN_VISIBLE_LEVEL {
        Ink::BLANK
    } else {
        Ink::new(Role::Structure, ink::level(level))
    }
}

/// Regolith in shadow where it meets the sky: the darkest grey, not black, so a shadowed
/// stretch of skyline still reads as ground (ML-A8).
const SHADE: f64 = 16.0;

/// How far the sun-side rim's shadow reaches across a crater's floor under the 11° Sun,
/// in crater radii: `depth / tan 11°` for a mature, shallow mare bowl (depth ≈ D/13).
const PIT_SHADOW: f64 = 0.8;
/// The same reach for the black core; between the two the shadow is filled by light
/// scattered off the sunlit far wall (ML-A8).
const PIT_CORE: f64 = 0.4;

/// Light on a crater out on the plain at `(u, w)`, its ellipse normalised to the unit
/// circle (ML-A8), for a Sun on the `sun_x` side, as a multiple of the plain's level:
/// `None` off the crater. The rim's shadow is the bowl minus the bowl shifted `PIT_SHADOW`
/// away from the Sun, a crescent against the sun-side wall: 0 (black) in its core, dark
/// grey at its edge. The lit floor is a mid grey, the far wall facing the Sun bright, and
/// the outer rim bright on the sun side, dim on the far slope.
pub(crate) fn pit_light(u: f64, w: f64, sun_x: f64) -> Option<f64> {
    let d2 = u * u + w * w;
    if d2 >= 1.35 {
        return None;
    }
    // Position along x away from the Sun.
    let away = if sun_x < 0.0 { u } else { -u };
    if d2 >= 1.0 {
        return Some(if away < 0.0 { 1.4 } else { 0.8 });
    }
    let shaded = |reach: f64| (away - reach).powi(2) + w * w >= 1.0;
    Some(if shaded(PIT_CORE) {
        0.0
    } else if shaded(PIT_SHADOW) {
        0.3
    } else {
        let half = (1.0 - w * w).sqrt();
        0.85 + 0.6 * (away / half).max(0.0)
    })
}

/// Illuminated fraction of the Earth seen from Tranquility Base on 20 July 1969: the
/// Moon was a waxing crescent six days old, so the Earth, opposite, was a gibbous.
const EARTH_LIT: f64 = 0.65;
/// Longitude facing the Moon at landing (the mid-Atlantic, approximate).
const EARTH_FACING_LON: f64 = -45.0 * PI / 180.0;
/// Latitude facing the Moon (the sub-lunar point): the viewer's elevation over the
/// Earth's equator.
const EARTH_FACING_LAT: f64 = -5.0 * PI / 180.0;
/// Subsolar point at 20:17 UTC on 20 July 1969: northern summer.
const SUBSOLAR_LON: f64 = -123.0 * PI / 180.0;
const SUBSOLAR_LAT: f64 = 20.6 * PI / 180.0;

/// The Sun on the Earth's globe in its north-up view frame (x east, y north, z toward
/// the Moon), at the phase angle that lights `EARTH_LIT` of the disc, `k = (1 + cos α) /
/// 2`, and in the sky direction of the subsolar point seen from the sub-lunar one: over
/// the west limb (−x), since the subsolar point lay west of the facing longitude, and
/// tilted north, so the Arctic is lit and the Antarctic dark (ML-A6).
pub(crate) fn earth_sun() -> [f64; 3] {
    let c = 2.0 * EARTH_LIT - 1.0;
    let dl = SUBSOLAR_LON - EARTH_FACING_LON;
    let east = SUBSOLAR_LAT.cos_det() * dl.sin_det();
    let north = SUBSOLAR_LAT.sin_det() * EARTH_FACING_LAT.cos_det()
        - SUBSOLAR_LAT.cos_det() * EARTH_FACING_LAT.sin_det() * dl.cos_det();
    let s = (1.0 - c * c).sqrt() / east.hypot(north);
    [s * east, s * north, c]
}

// ---------------------------------------------------------------------------------------
// Sprites.
// ---------------------------------------------------------------------------------------

/// LM sprite: unicode and ASCII rows of equal char count, per-row ink, the row whose
/// centre holds the engine bell, and the zoom cap (rows/m) at which it is roughly to
/// scale (the LM is ~7 m tall and ~9 m across its footpads).
struct Sprite {
    uni: &'static [&'static str],
    asc: &'static [&'static str],
    ink: &'static [Ink],
    bell: usize,
    k_max: f64,
}

const ASCENT: Ink = Ink::new(Role::Text, 235);
/// Gold Kapton foil over the descent stage.
const DESCENT_STAGE: Ink = Ink::new(Role::Sun, 205);
const LEGS: Ink = Ink::new(Role::Structure, 170);

const LM_TALL: Sprite = Sprite {
    uni: &[
        "   ▗▆▖   ",
        "  ▟█▣█▙  ",
        " ▐▓▓▓▓▓▌ ",
        " ╱ ╶▼╴ ╲ ",
        "▁╱     ╲▁",
    ],
    asc: &[
        "   .^.   ",
        "  /#o#\\  ",
        " [=====] ",
        " / -V- \\ ",
        "_/     \\_",
    ],
    ink: &[ASCENT, ASCENT, DESCENT_STAGE, LEGS, LEGS],
    bell: 3,
    k_max: 0.5,
};

const LM_MID: Sprite = Sprite {
    uni: &["  ▟▣▙  ", " ▐▓▓▓▌ ", "╱  ▼  ╲"],
    asc: &["  /o\\  ", " [===] ", "/  V  \\"],
    ink: &[ASCENT, DESCENT_STAGE, LEGS],
    bell: 2,
    k_max: 0.3,
};

const LM_TINY: Sprite = Sprite {
    uni: &["A"],
    asc: &["A"],
    ink: &[ASCENT],
    bell: 0,
    k_max: 0.12,
};

fn sprite_for(w: u16, h: u16) -> &'static Sprite {
    match (w, h) {
        (20.., 20..) => &LM_TALL,
        (10.., 10..) => &LM_MID,
        _ => &LM_TINY,
    }
}

// ---------------------------------------------------------------------------------------
// Screen mapping.
// ---------------------------------------------------------------------------------------

/// Extra metres added to the altitude when choosing the zoom, so the ground stays in
/// frame and the zoom saturates smoothly near touchdown.
const ALT_PAD: f64 = 6.0;
/// Zoom floor (rows/m) for absurd altitudes.
const K_MIN: f64 = 0.01;
/// Sun elevation over the site (Apollo 11: ≈ 11°); the LM's shadow falls ahead of it.
const SUN_ELEV: f64 = 11.0 * PI / 180.0;
/// Apparent LM height for its shadow (m).
const LM_HEIGHT: f64 = 7.0;
/// Where the flag is planted relative to the LM (m, against the direction of travel).
const FLAG_OFFSET: f64 = 11.0;
const N_STARS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Star {
    fx: f64,
    fy: f64,
    level: u8,
    big: bool,
}

/// Per-frame screen mapping (pure function of size and LM state).
#[derive(Debug, Clone, Copy)]
struct View {
    k: f64,
    cx: f64,
    base_row: f64,
    x0: f64,
    z_ref: f64,
}

impl View {
    fn row(&self, z: f64) -> f64 {
        self.base_row - (z - self.z_ref) * self.k
    }
    fn col(&self, x: f64) -> f64 {
        self.cx + (x - self.x0) * self.k * ASPECT
    }
    fn world_x(&self, col: f64) -> f64 {
        self.x0 + (col - self.cx) / (self.k * ASPECT)
    }
}

// ---------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct MoonLanding {
    timeline: Timeline,
    /// Time within the cycle (s).
    t: f64,
    /// Approach direction (+1: flying towards +x).
    side: f64,
    terrain: Terrain,
    descent: Descent,
    dust: ParticlePool<Grain>,
    dust_acc: f64,
    stars: [Star; N_STARS],
    flicker: f64,
    width: u16,
    height: u16,
    ready: bool,
    /// Regolith and Earth (half blocks), dust and plume (braille), rasterised in `update`.
    px: Pixels,
    dots: Dots,
    /// Terrain height and cast shadow per pixel column, sized in `resize`.
    heights: Vec<f64>,
    shadow: Vec<bool>,
}

impl MoonLanding {
    fn new() -> Self {
        let terrain = Terrain {
            craters: [Crater::default(); N_CRATERS],
            waves: [(0.0, 0.0, 0.0); 2],
            pits: [Pit::default(); N_PITS],
            site_x: 0.0,
        };
        Self {
            timeline: Timeline::new(&PHASES),
            t: 0.0,
            side: 1.0,
            descent: Descent {
                lm: Lm::default(),
                t: 0.0,
                contact: None,
                touchdown: None,
                site_ground: 0.0,
            },
            terrain,
            dust: ParticlePool::with_capacity(DUST_MAX),
            dust_acc: 0.0,
            stars: [Star::default(); N_STARS],
            flicker: 0.0,
            width: 0,
            height: 0,
            ready: false,
            px: Pixels::default(),
            dots: Dots::default(),
            heights: Vec::new(),
            shadow: Vec::new(),
        }
    }

    /// A fresh seeded approach: new terrain, start state and sky.
    fn restart(&mut self, ctx: &mut Ctx) {
        self.t = 0.0;
        self.side = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
        self.terrain = Terrain::seeded(&mut ctx.rng, self.side);
        self.descent = Descent::seeded(&mut ctx.rng, &self.terrain, self.side);
        self.dust.clear();
        self.dust_acc = 0.0;
        for s in &mut self.stars {
            *s = Star {
                fx: ctx.rng.random_range(0.0..1.0),
                fy: ctx.rng.random_range(0.0..1.0),
                level: ctx.rng.random_range(60..200),
                big: ctx.rng.random_bool(0.12),
            };
        }
    }

    fn beat(&self) -> Beat {
        self.timeline.at(self.t)
    }

    /// Emit dust over a substep of length `h` ending `left` seconds before the frame
    /// ends; grains are pre-advanced so emission is smooth for any `dt`.
    fn emit_dust(&mut self, h: f64, left: f64, ctx: &mut Ctx) {
        let lm = &self.descent.lm;
        if self.descent.touchdown.is_some() || lm.thrust <= 0.0 {
            return;
        }
        let alt = self.descent.altitude(&self.terrain);
        if alt >= DUST_ALT {
            return;
        }
        let near = 1.0 - alt / DUST_ALT;
        let push = (2.0 * lm.thrust / T_MAX).min(1.0);
        self.dust_acc += DUST_RATE * push * near * near * h;
        let (x0, vx_lm) = (lm.x, lm.vx);
        while self.dust_acc >= 1.0 {
            self.dust_acc -= 1.0;
            let dir = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
            let x = x0 + ctx.rng.random_range(-2.0..2.0);
            let mut g = Grain {
                x,
                z: self.terrain.height(x) + 0.05,
                vx: dir * ctx.rng.random_range(6.0..26.0) * (0.4 + 0.6 * near) + 0.3 * vx_lm,
                vz: ctx.rng.random_range(0.3..2.6),
                age: 0.0,
            };
            ballistic(&mut g, left);
            self.dust.spawn(g);
        }
    }

    fn age_dust(&mut self, dt: f64) {
        let terrain = &self.terrain;
        self.dust.retain(|p| {
            ballistic(p, dt);
            p.age < DUST_LIFE && p.z >= terrain.height(p.x)
        });
    }

    /// Advance within one cycle; `h` never crosses the end of the descent phase.
    fn substep(&mut self, h: f64, left: f64, ctx: &mut Ctx) {
        if self.t < DESCENT {
            self.descent.step(h, &self.terrain);
            self.emit_dust(h, left, ctx);
        }
        self.t += h;
        // Fail-safe (never reached for the seeded start set, ML-A1): the story needs
        // the LM down when the descent phase ends.
        if self.t >= DESCENT && self.descent.touchdown.is_none() {
            let g = self.terrain.ground(self.descent.lm.x);
            self.descent.land(g);
        }
    }
}

impl Generator for MoonLanding {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.width = size.width();
        self.height = size.height();
        self.px.resize(size);
        self.dots.resize(size);
        self.heights.clear();
        self.heights.resize(usize::from(size.width()), 0.0);
        self.shadow.clear();
        self.shadow.resize(usize::from(size.width()), false);
        if !self.ready {
            self.ready = true;
            self.restart(ctx);
        }
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
        self.age_dust(dt);
        let total = self.timeline.total();
        let mut left = dt;
        while left > 1e-12 {
            // Substeps end exactly on the descent boundary and on the cycle end.
            let mut h = left.min(H_MAX).min(total - self.t);
            if self.t < DESCENT {
                h = h.min(DESCENT - self.t);
            }
            let h = h.max(0.0);
            left = (left - h).max(0.0);
            self.substep(h, left, ctx);
            if self.t >= total - 1e-9 {
                self.restart(ctx);
            }
        }
        self.flicker = ctx.rng.random();
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        if !self.ready || self.width == 0 || self.height == 0 {
            return;
        }
        let sprite = sprite_for(self.width, self.height);
        let view = self.view(sprite);
        let beat = self.beat();
        self.draw_stars(canvas);
        self.px.blit(canvas, |_, _, top, bottom| {
            let ink = if top.role == Role::Background {
                bottom
            } else {
                top
            };
            match ink.role {
                Role::Background => None,
                Role::Structure if ink.level > 150 => Some(('#', ink)),
                Role::Structure if ink.level > 70 => Some((':', ink)),
                Role::Structure => Some(('.', ink)),
                _ => Some(('o', ink)),
            }
        });
        self.dots.blit(canvas);
        if beat.index >= PH_FLAG {
            self.draw_flag(canvas, &view, sprite, beat);
        }
        self.draw_lm(canvas, &view, sprite);
        self.draw_hud(canvas, beat);
        // The LM, flag and text over the regolith keep it behind them.
        self.px.backdrop(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        // Touchdown is a model event, not a beat: the LM lands whenever its physics does.
        Some(match self.beat().index {
            PH_FLAG | PH_EARTH => CAPTIONS[2],
            _ if self.descent.touchdown.is_some() => CAPTIONS[1],
            _ => CAPTIONS[0],
        })
    }
}

impl MoonLanding {
    fn view(&self, sprite: &Sprite) -> View {
        let h = f64::from(self.height);
        let ground_rows = (h / 4.0).floor().clamp(1.0, 12.0);
        let base_row = if self.height >= 6 {
            h - ground_rows
        } else {
            h - 1.0
        };
        // Rows 0–1 hold the readout and the opening caption.
        let top = if self.height >= 12 { 3.0 } else { 0.0 };
        let avail = (base_row - sprite.uni.len() as f64 - top).max(1.0);
        let z_ref = self.descent.site_ground;
        let alt = (self.descent.lm.z - z_ref).max(0.0);
        let k = (avail / (alt + ALT_PAD)).clamp(K_MIN, sprite.k_max);
        View {
            k,
            cx: (f64::from(self.width) * 0.5).floor(),
            base_row,
            x0: self.descent.lm.x,
            z_ref,
        }
    }

    fn draw_stars(&self, canvas: &mut Canvas) {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        // About one star per 40 cells, so small screens are not a snowstorm.
        let n = (usize::from(self.width) * usize::from(self.height) / 40).min(N_STARS);
        for s in &self.stars[..n] {
            let g = if s.big {
                Glyph::new('✦', '*')
            } else {
                Glyph::new('·', '.')
            };
            plot(
                canvas,
                (s.fx * w).floor(),
                (s.fy * h * 0.8).floor(),
                g,
                Ink::new(Role::Star, s.level),
            );
        }
    }

    /// The Earth's centre (cells) and radius (rows): high in the sky toward the side the
    /// LM flies to, the same in every act, since from Tranquility Base it hangs ~66° up
    /// and never rises or sets (ML-A6). Drawn larger than its true 1.9° (stylised), but
    /// small enough to leave the LM the focus.
    fn earth(&self) -> (f64, f64, f64) {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let r = (h / 12.0).clamp(0.8, 3.4);
        (w * (0.5 + 0.3 * self.side), (0.2 * h).max(2.5 + r), r)
    }

    /// The Earth's globe. Its day side must face the scene's Sun (on the −`side` of the
    /// sky) and be the Americas. North up puts the west limb on the left, so with the Sun
    /// on the right the globe is drawn south up (ML-A6).
    fn earth_globe(&self) -> Globe {
        let (ex, ey, er) = self.earth();
        Globe {
            cx: ex,
            cy: 2.0 * ey,
            r: 2.0 * er,
            elev: EARTH_FACING_LAT,
            // The globe shows longitude `−π/2 − spin` face-on.
            spin: -FRAC_PI_2 - EARTH_FACING_LON,
            sun: earth_sun(),
            south_up: self.side < 0.0,
        }
    }

    /// Rasterise the Earth, the regolith, the dust and the plume for this frame.
    fn rasterise(&mut self) {
        if !self.ready || self.width == 0 || self.height == 0 {
            return;
        }
        let sprite = sprite_for(self.width, self.height);
        let view = self.view(sprite);
        self.px.clear();
        self.dots.clear();
        self.earth_globe().paint(&mut self.px);
        self.paint_regolith(&view);
        self.dust_dots(&view);
        self.plume_dots(&view, sprite);
    }

    /// The mare in half blocks (ML-A5): the skyline lit by the low Sun through each
    /// slope's Lambert term, the darkest grey (`SHADE`) where a rim or the LM shades it,
    /// and the plain in front evenly lit with a fixed speckle of rocks that scrolls with
    /// the ground, craters with a crescent shadow and black core against the sun-side wall
    /// (ML-A8) and the LM's shadow running ahead.
    fn paint_regolith(&mut self, view: &View) {
        let w = usize::from(self.width);
        let ph = 2 * i32::from(self.height);
        let dx = 1.0 / (view.k * ASPECT);
        for (c, h) in self.heights.iter_mut().enumerate() {
            *h = self.terrain.height(view.world_x(c as f64 + 0.5));
        }
        let sun = sun_dir(self.side);
        cast_shadows(&self.heights, dx, sun, &mut self.shadow);
        // The LM's own shadow, thrown ahead of it along the track.
        let lm = &self.descent.lm;
        let above = (lm.z - self.terrain.ground(lm.x)).max(0.0);
        let s0 = lm.x + self.side * above / SUN_ELEV.tan();
        let s1 = s0 + self.side * LM_HEIGHT / SUN_ELEV.tan();
        let (sa, sb) = (s0.min(s1) - FOOT_HALF * 0.5, s0.max(s1));
        let flat = SUN_ELEV.sin_det();
        // The footpads' pixel row and the columns under the sprite: a site that lies up
        // to a cell below the pads is raised to them, so the LM, whose pad row is rounded
        // to whole cells, stands on regolith instead of over a sky-black notch (ML-A8).
        let pad = 2.0 * (Self::foot_row(view, lm.z) + 1.0);
        let reach = (sprite_for(self.width, self.height).uni[0].chars().count() / 2) as f64;
        let under = (view.cx - reach)..=(view.cx + reach);
        for c in 0..w {
            let x = view.world_x(c as f64 + 0.5);
            let (a, b) = (c.saturating_sub(1), (c + 1).min(w - 1));
            let slope = (self.heights[b] - self.heights[a]) / (dx * (b - a).max(1) as f64);
            let lm_shadow = (sa..=sb).contains(&x);
            let dark = self.shadow[c] || lm_shadow;
            let lit = if dark {
                0.0
            } else {
                (slope_light(slope, sun) / flat).powf(0.6)
            };
            let mut top = 2.0 * view.row(self.heights[c]);
            if !top.is_finite() {
                continue;
            }
            if under.contains(&(c as f64)) && top > pad && top <= pad + 2.0 {
                top = pad;
            }
            // World-anchored speckle: one rock per ~2 m of ground.
            let rock = (x * 0.5).floor() as i32;
            let plain = (f64::from(ph) - top).max(1.0);
            for py in (top.ceil().max(0.0) as i32)..ph {
                let depth = f64::from(py) - top;
                // The lit surface is two pixels deep. In a rim's or the LM's shadow it is
                // the darkest grey, not black, so the skyline stays unbroken against the
                // sky and the shadow reads as a sliver on the ground (ML-A8).
                let level = if depth < 2.0 {
                    if dark {
                        SHADE
                    } else {
                        20.0 + 160.0 * lit.min(1.5)
                    }
                } else {
                    // Bright rocks and shadowed pits on a plain that darkens toward the
                    // viewer, so the skyline reads as the far distance.
                    let n = hash01(cell_key(rock, py));
                    let speck = match n {
                        n if n > 0.95 => 60.0,
                        n if n < 0.04 => -40.0,
                        n => 24.0 * n,
                    };
                    (104.0 - 58.0 * depth / plain + speck).max(12.0)
                };
                self.px.set(c as i32, py, regolith(level));
            }
        }
        self.paint_pits(view, sun.0);
    }

    /// Craters on the plain as foreshortened ellipses, rounder toward the viewer, shaded
    /// by [`pit_light`] over the plain's own level (ML-A8).
    fn paint_pits(&mut self, view: &View, sun_x: f64) {
        let w = usize::from(self.width);
        let ph = 2.0 * f64::from(self.height);
        for pit in &self.terrain.pits {
            let (cx, a) = (view.col(pit.x) + 0.5, pit.r * view.k * ASPECT);
            if a < 0.8 || cx + 1.2 * a < 0.0 || cx - 1.2 * a >= w as f64 {
                continue;
            }
            let col = (cx.max(0.0) as usize).min(w - 1);
            let top = 2.0 * view.row(self.heights[col]) + 3.0;
            let plain = ph - top;
            let b = a * (0.25 + 0.35 * pit.v);
            if !top.is_finite() || plain < 2.0 * b + 2.0 {
                continue;
            }
            let cy = top + b + pit.v * (plain - 2.0 * b);
            let (x0, x1) = ((cx - 1.2 * a).floor() as i32, (cx + 1.2 * a).ceil() as i32);
            let (y0, y1) = ((cy - 1.2 * b).floor() as i32, (cy + 1.2 * b).ceil() as i32);
            for y in y0.max(top.ceil() as i32)..=y1 {
                // The plain's level at this depth, as `paint_regolith` lays it.
                let base = 104.0 - 58.0 * (f64::from(y) - top) / plain;
                for x in x0..=x1 {
                    // Never over another column's skyline: the plain begins below it.
                    let surface = usize::try_from(x)
                        .ok()
                        .and_then(|c| self.heights.get(c))
                        .map_or(f64::INFINITY, |&h| 2.0 * view.row(h) + 2.0);
                    if f64::from(y) < surface {
                        continue;
                    }
                    let (u, v) = ((f64::from(x) + 0.5 - cx) / a, (f64::from(y) + 0.5 - cy) / b);
                    if let Some(light) = pit_light(u, v, sun_x) {
                        // A zero multiple is the black core, whatever the plain's level.
                        let level = if light == 0.0 {
                            0.0
                        } else {
                            (base * light).clamp(SHADE, 235.0)
                        };
                        self.px.set(x, y, regolith(level));
                    }
                }
            }
        }
    }

    /// Dust: flat ballistic sheets in braille, brightest when fresh.
    fn dust_dots(&mut self, view: &View) {
        for p in &self.dust {
            let fade = 1.0 - (p.age / DUST_LIFE).clamp(0.0, 1.0);
            let (x, y) = Dots::from_cells(view.col(p.x) + 0.5, view.row(p.z));
            self.dots
                .plot(x, y, Ink::new(Role::Structure, (90.0 + 160.0 * fade) as u8));
        }
    }

    /// The descent engine's plume: its hypergolic exhaust is nearly invisible in vacuum,
    /// so a faint, flickering violet cone of dots (stylised), clipped at the ground.
    fn plume_dots(&mut self, view: &View, sprite: &Sprite) {
        let lm = self.descent.lm;
        if lm.thrust <= 0.0 || self.descent.touchdown.is_some() {
            return;
        }
        let n = sprite.uni.len();
        let foot = Self::foot_row(view, lm.z);
        let shear = (lm.theta.tan() * ASPECT * 0.5).clamp(-0.6, 0.6);
        let bell = foot - (n - 1 - sprite.bell) as f64;
        let throttle = lm.thrust / T_MAX;
        let len = (1.0 + 3.5 * throttle) * f64::from(Dots::CELL_H);
        let surface = view.row(self.terrain.height(lm.x)) * f64::from(Dots::CELL_H);
        let frame = (self.flicker * 1e6) as u64;
        let (x0, y0) = Dots::from_cells(view.cx + 0.5, bell + 1.0);
        for j in 0..len.ceil() as i32 {
            let y = y0 + f64::from(j);
            if y >= surface {
                break;
            }
            let u = f64::from(j) / len;
            let half = 1.0 + 3.0 * u;
            let cx = x0 - shear * f64::from(Dots::CELL_W) * f64::from(j) / f64::from(Dots::CELL_H);
            for i in -(half.ceil() as i32)..=(half.ceil() as i32) {
                let keep = hash01(frame ^ cell_key(i, j)) < 0.75 * (1.0 - u);
                if keep && f64::from(i).abs() <= half {
                    self.dots
                        .plot(cx + f64::from(i), y, ramp(Ramp::Plasma, 0.62 - 0.35 * u));
                }
            }
        }
    }

    /// Screen row of the sprite's bottom (footpad) row.
    fn foot_row(view: &View, z: f64) -> f64 {
        (view.row(z) - 0.5).round()
    }

    fn draw_lm(&self, canvas: &mut Canvas, view: &View, sprite: &Sprite) {
        let lm = &self.descent.lm;
        let n = sprite.uni.len();
        let foot = Self::foot_row(view, lm.z);
        let half = (sprite.uni[0].chars().count() / 2) as f64;
        // Lean the sprite about its feet with the attitude (a shear, capped for legibility).
        let shear = (lm.theta.tan() * ASPECT * 0.5).clamp(-0.6, 0.6);
        let x_of = |row: f64| (view.cx + shear * (foot - row)).round();
        for (i, (u, a)) in sprite.uni.iter().zip(sprite.asc.iter()).enumerate() {
            let y = foot - (n - 1 - i) as f64;
            let x0 = x_of(y) - half;
            for (j, (cu, ca)) in u.chars().zip(a.chars()).enumerate() {
                if cu == ' ' {
                    continue;
                }
                let ink = if cu == '▣' {
                    Ink::new(Role::Cold, 200)
                } else {
                    sprite.ink[i]
                };
                plot(canvas, x0 + j as f64, y, Glyph::new(cu, ca), ink);
            }
        }
    }

    fn draw_flag(&self, canvas: &mut Canvas, view: &View, sprite: &Sprite, beat: Beat) {
        let pole_h = match sprite.uni.len() {
            5.. => 3,
            3.. => 2,
            _ => return,
        };
        let lm = &self.descent.lm;
        let fx = lm.x - self.side * FLAG_OFFSET;
        let col = view.col(fx).round();
        let ground = Self::foot_row(view, self.terrain.height(fx));
        // The pole goes in during the first second of the flag phase; then the flag
        // (on its horizontal rod — no wind on the Moon) runs out.
        let t = if beat.index == PH_FLAG {
            beat.local
        } else {
            99.0
        };
        let shown = (ease::out_quad(t / 1.0) * f64::from(pole_h)).round() as i32;
        let pole = Ink::new(Role::Structure, 200);
        for i in 0..shown {
            plot(
                canvas,
                col,
                ground - f64::from(i),
                Glyph::new('│', '|'),
                pole,
            );
        }
        let run = ((t - 1.0) / 0.8).clamp(0.0, 1.0);
        let cloth = (run * 4.0).round() as i32;
        let top = ground - f64::from(pole_h - 1);
        let dir = -self.side;
        for j in 0..cloth {
            let x = col + dir * f64::from(j + 1);
            let (g, ink) = if j == 0 {
                (Glyph::new('▀', '#'), Ink::new(Role::Cold, 220))
            } else {
                (Glyph::new('▀', '='), Ink::new(Role::Hot, 200))
            };
            plot(canvas, x, top, g, ink);
            // The lower stripes, when the pole is tall enough to clear the ground.
            if pole_h >= 3 {
                let stripe = Ink::new(Role::Text, 200);
                plot(canvas, x, top + 1.0, Glyph::new('▀', '='), stripe);
            }
        }
        // An astronaut beside the flag once it is up.
        if pole_h >= 3 && t > 1.2 {
            let ax = col - dir * 2.0;
            let suit = Ink::new(Role::Text, 240);
            plot(canvas, ax, ground - 1.0, Glyph::new('●', 'o'), suit);
            plot(canvas, ax, ground, Glyph::new('▙', 'A'), suit);
        }
    }

    fn draw_hud(&self, canvas: &mut Canvas, beat: Beat) {
        let w = usize::from(self.width);
        let ink = Ink::new(Role::Text, 235);
        let cap_y = i32::from(self.height / 5).max(2);
        let d = &self.descent;
        if self.height >= 8 {
            match beat.index {
                PH_DESCENT | PH_TOUCHDOWN => {
                    // Descent time and cycle time share an origin.
                    let stopped = d.touchdown.is_some_and(|td| self.t - td.t >= 1.5);
                    if stopped {
                        caption(canvas, w, cap_y, &["ENGINE STOP"], ink);
                    } else if d.contact.is_some() {
                        // A blinking panel light.
                        if (self.t * 3.0).fract() < 0.7 {
                            caption(
                                canvas,
                                w,
                                cap_y,
                                &["CONTACT LIGHT"],
                                Ink::new(Role::Accent, 255),
                            );
                        }
                    } else if self.t < 3.0 && self.height >= 12 {
                        // Above the LM, which starts high in the frame. Names the craft
                        // only: the title already says `Apollo 11 · powered descent`.
                        caption(
                            canvas,
                            w,
                            1,
                            &["LUNAR MODULE EAGLE", "LM EAGLE", "EAGLE"],
                            ink,
                        );
                    }
                }
                PH_FLAG => {
                    caption(
                        canvas,
                        w,
                        cap_y,
                        &["Houston, Tranquility Base here.", "Tranquility Base here."],
                        ink,
                    );
                    if beat.local > 1.2 {
                        caption(
                            canvas,
                            w,
                            cap_y + 1,
                            &["The Eagle has landed.", "Eagle has landed."],
                            ink,
                        );
                    }
                }
                _ => {
                    caption(
                        canvas,
                        w,
                        cap_y,
                        &["20 JULY 1969 - MARE TRANQUILLITATIS", "20 JULY 1969"],
                        Ink::new(Role::Text, 170),
                    );
                }
            }
        }
        if beat.index <= PH_TOUCHDOWN && self.height >= 3 {
            self.draw_readout(canvas, w);
        }
    }

    /// `ALT 102.3 m  V/S -6.2 m/s  FUEL 7.4 %` (shortened on narrow screens), without
    /// allocating.
    fn draw_readout(&self, canvas: &mut Canvas, w: usize) {
        use std::fmt::Write;
        let d = &self.descent;
        let alt = d.altitude(&self.terrain);
        let vs = d.lm.vz;
        let fuel = 100.0 * d.lm.fuel() / PROP_FULL;
        let mut buf = StackText::<64>::new();
        // `StackText` truncates instead of failing; the width check below drops a cut line.
        let _ = if w >= 44 {
            write!(
                buf,
                "ALT {alt:>5.1} m  V/S {vs:>+5.1} m/s  FUEL {fuel:>4.1} %"
            )
        } else {
            write!(buf, "ALT {alt:.0} V/S {vs:+.1} F {fuel:.0}%")
        };
        if buf.len() < w {
            canvas.text(1, 0, buf.as_str(), Ink::new(Role::Text, 160));
        }
        let prog = match d.program() {
            Program::P64 => "P64",
            Program::P66 => "P66",
            Program::Landed => "",
        };
        if w >= buf.len() + 8 && !prog.is_empty() {
            canvas.text(w as i32 - 4, 0, prog, Ink::new(Role::Accent, 200));
        }
    }
}

/// Centered caption: the first variant that fits with a cell of margin, else nothing
/// (never clipped mid-word).
fn caption(canvas: &mut Canvas, w: usize, y: i32, variants: &[&str], ink: Ink) {
    if let Some(s) = variants.iter().find(|s| s.chars().count() + 2 <= w) {
        draw::text_centered(canvas, y, s, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::geom::wrap_angle;

    fn approach(seed: u64) -> (Terrain, Descent) {
        let mut ctx = Ctx::new(seed, Size::new(80, 24));
        let side = if seed % 2 == 0 { 1.0 } else { -1.0 };
        let terrain = Terrain::seeded(&mut ctx.rng, side);
        let d = Descent::seeded(&mut ctx.rng, &terrain, side);
        (terrain, d)
    }

    /// Fly a seeded approach to touchdown at `H_MAX`, checking thrust and terrain
    /// clearance on every substep.
    fn fly_to_touchdown(seed: u64) -> (Terrain, Descent) {
        let (terrain, mut d) = approach(seed);
        while d.touchdown.is_none() && d.t < 40.0 {
            d.step(H_MAX, &terrain);
            let lm = &d.lm;
            assert!(lm.thrust >= 0.0 && lm.thrust <= T_MAX + 1e-9, "throttle");
            assert!(
                lm.z >= terrain.ground(lm.x) - 1e-9,
                "seed {seed}: below ground at t={}",
                d.t
            );
        }
        (terrain, d)
    }

    #[test]
    fn ml_a1_guidance_lands_softly_for_the_seeded_start_set() {
        let mut worst = (0.0f64, 0.0f64, f64::MAX);
        let mut t_range = (f64::MAX, 0.0f64);
        for seed in 0..120 {
            let (terrain, d) = fly_to_touchdown(seed);
            let td = d
                .touchdown
                .unwrap_or_else(|| panic!("seed {seed}: never landed"));
            assert!(td.vz.abs() <= 1.0, "seed {seed}: vertical {}", td.vz);
            assert!(td.vx.abs() <= 0.5, "seed {seed}: horizontal {}", td.vx);
            assert!(td.fuel > 0.0, "seed {seed}: fuel {}", td.fuel);
            assert!(
                td.t > 15.0 && td.t < DESCENT - 0.5,
                "seed {seed}: touchdown at {} s must fit the descent phase",
                td.t
            );
            assert!(
                (td.x - terrain.site_x).abs() < SITE_CLEAR - FOOT_HALF,
                "seed {seed}: landed {} m from the site, outside the cleared zone",
                td.x - terrain.site_x
            );
            let c = d.contact.expect("contact light before touchdown");
            assert!(c < td.t, "seed {seed}: probes touch first");
            t_range = (t_range.0.min(td.t), t_range.1.max(td.t));
            worst = (
                worst.0.max(td.vz.abs()),
                worst.1.max(td.vx.abs()),
                worst.2.min(td.fuel),
            );
        }
        eprintln!("ML-A1 worst |vz|, |vx|, fuel kg: {worst:?}; touchdown t {t_range:?}");
    }

    #[test]
    fn ml_a1_e_guidance_hits_its_target_exactly_in_open_loop() {
        // Following a(τ) = a₀ + jτ from the closed form reaches (r_f, v_f) at t_go.
        let (r, v, rf, vf, tg): (f64, f64, f64, f64, f64) = (-150.0, 18.0, 0.0, 0.0, 15.0);
        let a0 = e_guidance(r, v, rf, vf, tg);
        let j = 6.0 * (v + vf) / (tg * tg) - 12.0 * (rf - r) / (tg * tg * tg);
        let v_end = v + a0 * tg + 0.5 * j * tg * tg;
        let r_end = r + v * tg + 0.5 * a0 * tg * tg + j * tg * tg * tg / 6.0;
        assert!((v_end - vf).abs() < 1e-9 && (r_end - rf).abs() < 1e-9);
        // Already on a coasting trajectory to the target: no acceleration needed.
        assert!(e_guidance(0.0, 2.0, 20.0, 2.0, 10.0).abs() < 1e-12);
    }

    #[test]
    fn ml_a1_generator_lands_softly_with_uneven_frame_steps() {
        let steps = [0.25, 0.013, SIM_DT, 0.2, 0.07, 0.0];
        for seed in [1, 7, 42, 1969] {
            let size = Size::new(100, 30);
            let mut ctx = Ctx::new(seed, size);
            let mut g = MoonLanding::new();
            g.resize(size, &mut ctx);
            let mut i = 0;
            while g.descent.touchdown.is_none() {
                g.update(steps[i % steps.len()], &mut ctx);
                i += 1;
                assert!(i < 10_000);
            }
            let td = g.descent.touchdown.unwrap();
            assert!(td.t < DESCENT - 0.5, "seed {seed}: landed by guidance");
            assert!(
                td.vz.abs() <= 1.0 && td.vx.abs() <= 0.5,
                "seed {seed}: {td:?}"
            );
        }
    }

    #[test]
    fn ml_a2_constant_thrust_vertical_burn_follows_the_rocket_equation() {
        let m0 = 15_000.0;
        let thrust = 0.9 * T_MAX;
        let mut lm = Lm {
            m: m0,
            thrust,
            ..Lm::default()
        };
        let mut t = 0.0;
        while t < 60.0 - 1e-9 {
            fly(&mut lm, H_MAX);
            t += H_MAX;
            // Tsiolkovsky with gravity loss, from rest.
            let dv = ISP * G0 * (m0 / lm.m).ln() - G_MOON * t;
            assert!(
                (lm.vz - dv).abs() <= 0.005 * dv.abs() + 1e-9,
                "t={t}: {} vs {dv}",
                lm.vz
            );
            assert!((lm.m - (m0 - thrust / (ISP * G0) * t)).abs() < 1e-6);
            assert_eq!(lm.x, 0.0);
        }
        // A non-trivial burn: ≈ 800 kg of propellant, ≈ 166 m/s ideal Δv less 97 m/s of
        // gravity loss.
        assert!(lm.vz > 60.0, "{}", lm.vz);
        // Burning out stops at the dry mass instead of going negative.
        let mut lm = Lm {
            m: M_DRY + 1.0,
            thrust: T_MAX,
            ..Lm::default()
        };
        fly(&mut lm, 1.0);
        assert!(lm.m >= M_DRY - 1e-9 && lm.fuel() == 0.0);
    }

    #[test]
    fn ml_a3_terrain_is_a_pure_continuous_function_of_x() {
        let (t1, _) = approach(3);
        let (t2, _) = approach(3);
        assert_eq!(t1, t2, "same seed, same terrain");
        let mut x = -800.0;
        let mut last = t1.height(x);
        while x < 800.0 {
            x += 0.25;
            let h = t1.height(x);
            assert!(h.is_finite());
            assert_eq!(h, t1.height(x), "pure");
            assert!((h - last).abs() < 0.5, "continuous at x={x}");
            last = h;
        }
        // The site is clear of craters: flat to within the rolling swell.
        let s = t1.site_x;
        let span = t1.ground(s) - t1.height(s - FOOT_HALF).min(t1.height(s + FOOT_HALF));
        assert!(span < 0.5, "site slope {span}");
        // And some craters are deep enough to see.
        let (lo, hi) = (-400..400)
            .map(|i| t1.height(f64::from(i)))
            .fold((f64::MAX, f64::MIN), |(a, b), h| (a.min(h), b.max(h)));
        assert!(hi - lo > 5.0, "relief {}", hi - lo);
    }

    #[test]
    fn ml_a3_lm_never_penetrates_the_terrain() {
        for seed in 0..40 {
            fly_to_touchdown(seed);
        }
        // Through the generator, across loops, resizes and ragged steps.
        let size = Size::new(90, 28);
        let mut ctx = Ctx::new(11, size);
        let mut g = MoonLanding::new();
        g.resize(size, &mut ctx);
        let steps = [0.25, SIM_DT, 0.11, 0.004];
        let mut t = 0.0;
        let mut i = 0;
        while t < 80.0 {
            let dt = steps[i % steps.len()];
            g.update(dt, &mut ctx);
            t += dt;
            i += 1;
            let lm = &g.descent.lm;
            assert!(lm.z >= g.terrain.ground(lm.x) - 1e-9, "t={t}");
        }
    }

    #[test]
    fn gen_r12_caption_names_the_act_the_lm_is_in() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        let mut g = MoonLanding::new();
        g.resize(size, &mut ctx);
        let mut seen = vec![g.caption().unwrap()];
        // Two loops, stopping a second short of the third descent.
        for _ in 0..((2.0 * g.timeline.total() - 1.0) / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let caption = g.caption().unwrap();
            let want = match g.beat().index {
                PH_FLAG | PH_EARTH => CAPTIONS[2],
                _ if g.descent.touchdown.is_some() => CAPTIONS[1],
                _ => CAPTIONS[0],
            };
            assert_eq!(caption, want);
            if *seen.last().unwrap() != caption {
                seen.push(caption);
            }
        }
        assert_eq!(
            seen,
            [CAPTIONS, CAPTIONS].concat(),
            "each act once per loop"
        );
    }

    #[test]
    fn ml_a4_timeline_phases_occur_in_order_and_loop() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        let mut g = MoonLanding::new();
        g.resize(size, &mut ctx);
        let total = g.timeline.total();
        assert!((total - 36.0).abs() < 1e-9);
        let first = g.descent.clone();
        let mut seen = vec![g.beat().index];
        let mut t = 0.0;
        let mut landed_in = None;
        while t < 2.0 * total + 1.0 {
            g.update(SIM_DT, &mut ctx);
            t += SIM_DT;
            let b = g.beat();
            if *seen.last().unwrap() != b.index {
                seen.push(b.index);
            }
            if landed_in.is_none() && g.descent.touchdown.is_some() {
                landed_in = Some(b.index);
            }
        }
        assert_eq!(
            seen,
            [
                PH_DESCENT,
                PH_TOUCHDOWN,
                PH_FLAG,
                PH_EARTH,
                PH_DESCENT,
                PH_TOUCHDOWN,
                PH_FLAG,
                PH_EARTH,
                PH_DESCENT
            ]
        );
        assert_eq!(
            landed_in,
            Some(PH_DESCENT),
            "touchdown happens in the descent"
        );
        assert_ne!(
            g.descent.lm.x, first.lm.x,
            "each loop is a new seeded approach"
        );
        assert!(g.descent.touchdown.is_none() && g.t < 2.0);
    }

    #[test]
    fn ml_a5_the_low_sun_lights_slopes_that_face_it() {
        for side in [1.0, -1.0] {
            let sun = sun_dir(side);
            let flat = slope_light(0.0, sun);
            assert!((flat - SUN_ELEV.sin_det()).abs() < 1e-12);
            // Terrain rising away from the Sun faces it; falling away turns its back.
            let facing = slope_light(0.2 * side, sun);
            let away = slope_light(-0.2 * side, sun);
            assert!(facing > flat && flat > away, "side {side}");
            // Steeper than the Sun's elevation, away from it: in its own shadow.
            assert_eq!(slope_light(-(SUN_ELEV.tan() + 0.01) * side, sun), 0.0);
        }
    }

    #[test]
    fn ml_a5_rims_cast_long_shadows_down_sun() {
        let len = 10.0 / SUN_ELEV.tan();
        for side in [1.0, -1.0] {
            let mut hts = vec![0.0; 240];
            hts[120] = 10.0;
            let mut out = vec![false; 240];
            cast_shadows(&hts, 1.0, sun_dir(side), &mut out);
            for (i, &s) in out.iter().enumerate() {
                // The Sun sits behind the approach (−side), so shadows fall toward +side.
                let d = (i as f64 - 120.0) * side;
                assert_eq!(s, d > 0.0 && d < len, "side {side}, i {i}");
            }
        }
        // Shadows need a sample spacing; a zero one shades nothing.
        let mut out = vec![true; 3];
        cast_shadows(&[1.0, 0.0, 0.0], 0.0, sun_dir(1.0), &mut out);
        assert!(out.iter().all(|&s| !s));
    }

    #[test]
    fn ml_a6_earth_hangs_high_and_still_as_a_gibbous() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(9, size);
        let mut g = MoonLanding::new();
        g.resize(size, &mut ctx);
        let (x0, y0, r0) = g.earth();
        assert!(y0 + r0 < 0.45 * 40.0, "high in the sky");
        assert!(
            r0 <= 40.0 / 11.0,
            "small enough to leave the LM the focus: {r0} rows"
        );
        for _ in 0..((g.timeline.total() - 1.0) / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            if g.side == 1.0 || g.side == -1.0 {
                let (x, y, r) = g.earth();
                assert_eq!((y, r), (y0, r0), "the Earth never rises or sets here");
                assert!((x - x0).abs() < 1e-9 || (x - (120.0 - x0)).abs() < 1.0);
            }
        }
        // ≈ 65 % of the disc lit: the Earth seen from the Moon on 20 July 1969.
        let sun = earth_sun();
        let (mut lit, mut all) = (0u32, 0u32);
        for i in 0..200 {
            for j in 0..200 {
                let (nx, ny) = (f64::from(i) / 100.0 - 1.0, f64::from(j) / 100.0 - 1.0);
                let d2 = nx * nx + ny * ny;
                if d2 < 1.0 {
                    all += 1;
                    lit += u32::from(nx * sun[0] - ny * sun[1] + (1.0 - d2).sqrt() * sun[2] > 0.0);
                }
            }
        }
        let k = f64::from(lit) / f64::from(all);
        assert!((0.6..0.7).contains(&k), "lit fraction {k}");
    }

    #[test]
    fn ml_a6_the_lit_limb_is_the_americas_on_either_approach() {
        let subsolar = SUBSOLAR_LON;
        let size = Size::new(120, 40);
        for side in [1.0, -1.0] {
            let mut ctx = Ctx::new(1, size);
            let mut g = MoonLanding::new();
            g.resize(size, &mut ctx);
            g.side = side;
            let globe = g.earth_globe();
            // The disc point under the Sun (screen offsets, y down), turned with the globe.
            let sun = earth_sun();
            let flip = if globe.south_up { -1.0 } else { 1.0 };
            let (nx, ny) = (flip * sun[0], -flip * sun[1]);
            assert_eq!(nx.signum(), -side, "on the scene's Sun side of the sky");
            let (lon, lat) = globe.lon_lat(nx, ny).expect("on the disc");
            let off = wrap_angle(lon - subsolar).abs();
            assert!(
                off < 40.0_f64.to_radians(),
                "side {side}: lon {}",
                lon.to_degrees()
            );
            // July: the Sun stood 20.6° N, lighting the whole Arctic.
            assert!(
                (lat.to_degrees() - 20.6).abs() < 8.0,
                "side {side}: subsolar lat {}",
                lat.to_degrees()
            );
            // And that limb is the day side on screen.
            let day = globe.shade(0.9 * nx, 0.0, 0.5).expect("disc");
            let night = globe.shade(-0.9 * nx, 0.0, 0.5).expect("disc");
            assert!(day.level > night.level, "side {side}");
        }
    }

    #[test]
    fn ml_a7_gold_descent_stage_over_shaded_regolith() {
        let mut stage = Stage::new(build(), 42, Size::new(120, 40), GlyphMode::Unicode);
        for _ in 0..(12.0 / SIM_DT) as usize {
            stage.step(SIM_DT);
        }
        let f = stage.render();
        let count = |r: Role| {
            f.cells()
                .iter()
                .filter(|c| c.ink.role == r || c.bg.role == r)
                .count()
        };
        assert!(count(Role::Sun) >= 3, "gold foil");
        assert!(count(Role::Structure) > 400, "half-block regolith");
        assert!(count(Role::Ramp(Ramp::Ice)) > 3, "the Earth");
        assert!(f.cells().iter().any(|c| c.ch == '▀' || c.ch == '▄'));
    }

    #[test]
    fn ml_a8_crater_shadows_are_a_crescent_on_the_sun_side_wall() {
        for sun_x in [-1.0, 1.0] {
            // `s` = +1 means "away from the Sun" along x.
            let s = -sun_x;
            let at = |u: f64, w: f64| pit_light(s * u, w, sun_x);
            assert_eq!(
                at(-0.9, 0.0),
                Some(0.0),
                "black core under the sun-side wall"
            );
            let edge = at(-0.3, 0.0).unwrap();
            assert!(
                edge > 0.0 && edge < 0.5,
                "shadow edge is dark, not black: {edge}"
            );
            assert!(
                at(0.05, 0.0).unwrap() >= 0.7,
                "the floor past the middle is lit"
            );
            assert!(at(0.85, 0.0).unwrap() > 1.0, "far wall faces the Sun");
            assert!(at(-1.1, 0.0).unwrap() > 1.0, "sun-side outer rim");
            assert!(at(1.1, 0.0).unwrap() < 1.0, "far outer slope");
            assert_eq!(at(2.0, 0.0), None);
            assert_eq!(at(0.0, 1.5), None);
            // A crescent: the shadow hugs the sun-side wall and covers under half the bowl,
            // its black core less still.
            let (mut bowl, mut shade, mut black) = (0u32, 0u32, 0u32);
            for i in -100..100 {
                for j in -100..100 {
                    let (u, w) = (f64::from(i) / 100.0, f64::from(j) / 100.0);
                    if u * u + w * w < 1.0 {
                        bowl += 1;
                        let m = at(u, w).unwrap();
                        shade += u32::from(m < 0.5);
                        black += u32::from(m == 0.0);
                    }
                }
            }
            let k = f64::from(shade) / f64::from(bowl);
            let kb = f64::from(black) / f64::from(bowl);
            assert!((0.25..0.5).contains(&k), "shadow share {k}");
            assert!(kb > 0.05 && kb < 0.6 * k, "black core share {kb}");
        }
        // In the frame: black crater cores out on the plain, not just on the skyline.
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(4, size);
        let mut g = MoonLanding::new();
        g.resize(size, &mut ctx);
        let plain = (88..100)
            .flat_map(|y| (0..200).map(move |x| (x, y)))
            .filter(|&(x, y)| g.px.get(x, y) == Some(Ink::BLANK))
            .count();
        assert!(plain > 10, "{plain} black plain pixels");
    }

    #[test]
    fn ml_a8_shadows_never_cut_the_ground_out_of_the_skyline() {
        let cases = [
            (3, Size::new(200, 50)),
            (8, Size::new(80, 24)),
            (5, Size::new(120, 40)),
        ];
        for (seed, size) in cases {
            let mut ctx = Ctx::new(seed, size);
            let mut g = MoonLanding::new();
            g.resize(size, &mut ctx);
            let ph = 2 * i32::from(size.height());
            for f in 0..(DESCENT + 6.0) as usize * 4 {
                for _ in 0..15 {
                    g.update(SIM_DT, &mut ctx);
                }
                let view = g.view(sprite_for(g.width, g.height));
                for (c, &h) in g.heights.iter().enumerate() {
                    let py = (2.0 * view.row(h)).ceil();
                    if (0.0..f64::from(ph)).contains(&py) {
                        let ink = g.px.get(c as i32, py as i32);
                        assert_ne!(
                            ink,
                            Some(Ink::BLANK),
                            "seed {seed} {size:?} frame {f}: column {c} surface is black"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ml_a8_the_footpads_stand_on_the_regolith_at_touchdown() {
        // Composited, as the terminal shows it: under every column of the sprite's
        // footpad row, the cell below has regolith in its top half, so the LM never
        // hovers over a sky-black notch cut into the landing site.
        let cases = [
            (42, Size::new(200, 50)),
            (42, Size::new(120, 40)),
            (42, Size::new(80, 24)),
            (3, Size::new(200, 50)),
            (8, Size::new(80, 24)),
            (5, Size::new(120, 40)),
        ];
        for (seed, size) in cases {
            let mut ctx = Ctx::new(seed, size);
            let mut g = MoonLanding::new();
            g.resize(size, &mut ctx);
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            let mut checked = 0;
            while g.t < DESCENT + 3.0 {
                g.update(SIM_DT, &mut ctx);
                let Some(td) = g.descent.touchdown else {
                    continue;
                };
                if g.t < td.t + 0.1 {
                    continue;
                }
                canvas.clear();
                g.render(&mut canvas);
                let sprite = sprite_for(g.width, g.height);
                let view = g.view(sprite);
                let foot = MoonLanding::foot_row(&view, g.descent.lm.z) as i32;
                let half = (sprite.uni[0].chars().count() / 2) as i32;
                let x0 = view.cx as i32 - half;
                let pads = sprite.uni[sprite.uni.len() - 1];
                for (j, ch) in pads.chars().enumerate() {
                    if ch == ' ' {
                        continue;
                    }
                    let x = x0 + j as i32;
                    let cell = canvas.get(x, foot + 1).copied().expect("on screen");
                    let top_ground =
                        matches!(cell.ch, '▀' | '█') || (cell.bg != Ink::BLANK && cell.ch != '▄');
                    assert!(
                        top_ground,
                        "seed {seed} {size:?} t={:.2}: cell ({x}, {}) under the pads is {cell:?}",
                        g.t,
                        foot + 1
                    );
                    checked += 1;
                }
            }
            assert!(checked > 0, "seed {seed} {size:?}: never landed");
        }
    }

    #[test]
    fn ml_readout_and_captions_render() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        let lines = stage.render().to_lines();
        assert!(lines[0].contains("ALT") && lines[0].contains("FUEL"));
        assert!(lines.iter().any(|l| l.contains("/#o#\\")), "LM drawn");
        for _ in 0..(22.5 / SIM_DT) as usize {
            stage.step(SIM_DT);
        }
        let lines = stage.render().to_lines();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("CONTACT LIGHT") || l.contains("ENGINE STOP"))
        );
        for _ in 0..(7.0 / SIM_DT) as usize {
            stage.step(SIM_DT);
        }
        let lines = stage.render().to_lines();
        assert!(lines.iter().any(|l| l.contains("The Eagle has landed.")));
        // Narrow terminals get a shorter caption or none, never a clipped one.
        let mut small = Stage::new(build(), 42, Size::new(12, 10), GlyphMode::Ascii);
        let lines = small.render().to_lines();
        assert!(!lines.iter().any(|l| l.contains("LUNAR")));
    }
}
