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
//! (ML-A4): descent → touchdown → flag → earthrise → a new seeded approach.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::draw;
use sciart_core::ease;
use sciart_core::geom::ASPECT;
use sciart_core::ode;
use sciart_core::particles::plot;
use sciart_core::prelude::*;

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
    ("earthrise", 4.0),
];
const PH_DESCENT: usize = 0;
const PH_TOUCHDOWN: usize = 1;
const PH_FLAG: usize = 2;
const PH_EARTHRISE: usize = 3;

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
    let (thrust, (s, c)) = (lm.thrust.max(0.0), lm.theta.sin_cos());
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
    let (s, c) = lm.theta.sin_cos();
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
        Self {
            craters,
            waves,
            site_x,
        }
    }

    /// Surface height (m) at world x.
    pub(crate) fn height(&self, x: f64) -> f64 {
        let mut h = 0.0;
        for &(a, k, p) in &self.waves {
            h += a * (k * x + p).sin();
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
const DESCENT_STAGE: Ink = Ink::new(Role::Structure, 210);
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

const FLAME: [(Glyph, Ink); 4] = [
    (Glyph::new('▓', '#'), Ink::new(Role::Hot, 255)),
    (Glyph::new('▒', '*'), Ink::new(Role::Hot, 225)),
    (Glyph::new('░', ':'), Ink::new(Role::Hot, 170)),
    (Glyph::new('·', '.'), Ink::new(Role::Hot, 120)),
];

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
}

impl MoonLanding {
    fn new() -> Self {
        let terrain = Terrain {
            craters: [Crater::default(); N_CRATERS],
            waves: [(0.0, 0.0, 0.0); 2],
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
        if !self.ready {
            self.ready = true;
            self.restart(ctx);
        }
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
    }

    fn render(&self, canvas: &mut Canvas) {
        if !self.ready || self.width == 0 || self.height == 0 {
            return;
        }
        let sprite = sprite_for(self.width, self.height);
        let view = self.view(sprite);
        let beat = self.beat();
        self.draw_stars(canvas);
        if beat.index == PH_EARTHRISE {
            self.draw_earth(canvas, &view, beat.progress);
        }
        self.draw_terrain(canvas, &view);
        self.draw_dust(canvas, &view);
        if beat.index >= PH_FLAG {
            self.draw_flag(canvas, &view, sprite, beat);
        }
        self.draw_lm(canvas, &view, sprite);
        self.draw_hud(canvas, beat);
    }

    fn caption(&self) -> Option<&'static str> {
        // Touchdown is a model event, not a beat: the LM lands whenever its physics does.
        Some(match self.beat().index {
            PH_FLAG | PH_EARTHRISE => CAPTIONS[2],
            _ if self.descent.touchdown.is_some() => CAPTIONS[1],
            _ => CAPTIONS[0],
        })
    }
}

impl MoonLanding {
    fn view(&self, sprite: &Sprite) -> View {
        let h = f64::from(self.height);
        let ground_rows = (h / 6.0).floor().clamp(1.0, 5.0);
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

    /// Earth rising over the far horizon, lit from the side the Sun is on.
    fn draw_earth(&self, canvas: &mut Canvas, view: &View, progress: f64) {
        let h = f64::from(self.height);
        let r = (h / 9.0).clamp(0.6, 4.0);
        let rise = ease::smoothstep(progress / 0.65);
        let cy = ease::lerp(view.base_row + r + 1.0, (0.42 * h).max(r), rise);
        let cx = if self.side > 0.0 { 0.72 } else { 0.28 } * f64::from(self.width);
        let sun = -self.side;
        draw::disc(canvas, cx, cy, r, |d, nx, ny| {
            let lit = nx * sun * 0.9 + (1.0 - d * d).max(0.0).sqrt() * 0.45;
            if lit < 0.05 {
                return Some((Glyph::new('░', '.'), Ink::new(Role::Cold, 45)));
            }
            // Swirling cloud bands over the oceans.
            let band = (6.0 * ny + 2.5 * (3.0 * nx).sin()).sin();
            let level = (120.0 + 135.0 * lit.min(1.0)) as u8;
            Some(if band > 0.55 {
                (Glyph::new('▓', '#'), Ink::new(Role::Text, level))
            } else {
                (Glyph::new('█', '@'), Ink::new(Role::Cold, level))
            })
        });
    }

    fn draw_terrain(&self, canvas: &mut Canvas, view: &View) {
        const EIGHTHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let (w, hgt) = (i32::from(self.width), i32::from(self.height));
        let lm = &self.descent.lm;
        // Shadow of the LM cast by the low Sun, ahead of the LM along its track.
        let above = (lm.z - self.terrain.ground(lm.x)).max(0.0);
        let s0 = lm.x + self.side * above / SUN_ELEV.tan();
        let s1 = s0 + self.side * LM_HEIGHT / SUN_ELEV.tan();
        let (sa, sb) = (s0.min(s1) - FOOT_HALF * 0.5, s0.max(s1));
        for col in 0..w {
            let x = view.world_x(f64::from(col) + 0.5);
            let r = view.row(self.terrain.height(x));
            if !r.is_finite() || r >= f64::from(hgt) {
                continue;
            }
            let shadow = (sa..=sb).contains(&x);
            let top = r.floor();
            let fill = (top + 1.0 - r).clamp(0.0, 1.0);
            let surf = if shadow {
                Ink::new(Role::Dim, 40)
            } else {
                Ink::new(Role::Structure, 150)
            };
            let first = top.max(0.0) as i32;
            if top >= 0.0 {
                let e = ((fill * 8.0).ceil() as usize).clamp(1, 8) - 1;
                let a = if fill < 0.5 { '_' } else { '#' };
                canvas.put(col, first, Glyph::new(EIGHTHS[e], a), surf);
            }
            let body = if shadow {
                Ink::new(Role::Dim, 25)
            } else {
                Ink::new(Role::Dim, 90)
            };
            let start = if top >= 0.0 { first + 1 } else { 0 };
            for y in start..hgt {
                let depth = f64::from(y) - r;
                let (g, ink) = if depth < 1.5 {
                    (Glyph::new('▓', ':'), body)
                } else {
                    (
                        Glyph::new('▒', '.'),
                        Ink::new(Role::Dim, body.level / 2 + 20),
                    )
                };
                canvas.put(col, y, g, ink);
            }
        }
    }

    fn draw_dust(&self, canvas: &mut Canvas, view: &View) {
        for p in &self.dust {
            let fade = 1.0 - (p.age / DUST_LIFE).clamp(0.0, 1.0);
            let g = if p.vx.abs() > 4.0 * p.vz.abs() {
                Glyph::new('─', '-')
            } else {
                Glyph::new('·', '.')
            };
            let ink = Ink::new(Role::Particle, (70.0 + 170.0 * fade) as u8);
            plot(canvas, view.col(p.x), view.row(p.z) - 0.5, g, ink);
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
        // Flame first so the bell sits on top of it; clipped at the ground.
        if lm.thrust > 0.0 && self.descent.touchdown.is_none() {
            let throttle = lm.thrust / T_MAX;
            let len = ((1.0 + 3.5 * throttle) * (0.55 + 0.45 * self.flicker)).round() as usize;
            let surface = view.row(self.terrain.height(lm.x)).floor();
            let bell = foot - (n - 1 - sprite.bell) as f64;
            for (i, &(g, ink)) in FLAME.iter().take(len.clamp(1, FLAME.len())).enumerate() {
                let y = bell + 1.0 + i as f64;
                if y >= surface {
                    break;
                }
                plot(canvas, x_of(y), y, g, ink);
            }
        }
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
        let mut buf = LineBuf::default();
        let ok = if w >= 44 {
            write!(
                buf,
                "ALT {alt:>5.1} m  V/S {vs:>+5.1} m/s  FUEL {fuel:>4.1} %"
            )
        } else {
            write!(buf, "ALT {alt:.0} V/S {vs:+.1} F {fuel:.0}%")
        };
        if ok.is_ok() && buf.len < w {
            canvas.text(1, 0, buf.as_str(), Ink::new(Role::Text, 160));
        }
        let prog = match d.program() {
            Program::P64 => "P64",
            Program::P66 => "P66",
            Program::Landed => "",
        };
        if w >= buf.len + 8 && !prog.is_empty() {
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

/// A fixed-capacity text buffer, so `render` can format numbers without allocating.
struct LineBuf {
    bytes: [u8; 64],
    len: usize,
}

impl Default for LineBuf {
    fn default() -> Self {
        Self {
            bytes: [0; 64],
            len: 0,
        }
    }
}

impl LineBuf {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl std::fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len + s.len();
        if end > self.bytes.len() {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                PH_FLAG | PH_EARTHRISE => CAPTIONS[2],
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
                PH_EARTHRISE,
                PH_DESCENT,
                PH_TOUCHDOWN,
                PH_FLAG,
                PH_EARTHRISE,
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
