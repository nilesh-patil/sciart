//! Voyager 2's Grand Tour, 1977–1989, and its run out to the heliopause (SPEC-002
//! `voyager_journey`).
//!
//! A top-down heliocentric map. Planets move on circular orbits with their real J2000
//! semi-major axes and mean longitudes and periods from Kepler's third law (VOY-A2). The
//! spacecraft flies four Kepler conics, one per leg, each the Lambert solution that
//! leaves one planet's model position on its real encounter date and reaches the next
//! planet's on its date (VOY-A1, VOY-A5). A flyby is where one conic ends and the next
//! begins: the jump in heliocentric speed there *is* the gravity assist, and the inset
//! plots it against the Sun's escape speed (VOY-A6). After Neptune the craft coasts out
//! on a fifth conic, a hyperbola with today's excess speed whose heading is fitted so it
//! reaches the real heliopause crossing, 119 AU on 2018-11-05, while the map pulls back
//! to show it (VOY-A7).
//!
//! Every position is a pure function of a *mission day* (days since 2000-01-01); the scene
//! clock maps sim seconds to mission days with slow-downs at each encounter (VOY-A3), and
//! radii are drawn through a strictly monotone log compression (VOY-A4). The Sun is an
//! asinh-toned glow and planets and the craft are half-block [`Pixels`]; orbits, trail
//! and inset are braille [`Dots`]; all are rasterised in `update`.

use std::f64::consts::{PI, TAU};
use std::fmt::{self, Write as _};

use rand::Rng;
use sciart_core::geom::{self, ASPECT};
use sciart_core::ink::{self, ramp};
use sciart_core::prelude::*;
use sciart_core::{Tone, ease};

pub static META: Meta = Meta {
    name: "voyager_journey",
    category: Category::Voyages,
    description: "Voyager 2 Grand Tour past Jupiter, Saturn, Uranus and Neptune to the heliopause",
    title: "Voyager 2 · planets fling it on",
    captions: &ACTS,
};

/// The title's caption around each encounter and after Neptune (GEN-R12); `None` on the
/// cruise legs, where the title's own mechanism reads. Neptune bent Voyager 2 south over
/// its pole toward Triton and slowed it, so its caption says flyby, not assist.
const ACTS: [&str; 6] = [
    "Jupiter assist",
    "Saturn assist",
    "Uranus assist",
    "Neptune flyby",
    "to the heliopause",
    "interstellar space",
];

pub fn build() -> Box<dyn Generator> {
    Box::new(VoyagerJourney::new())
}

// ---------------------------------------------------------------------------------------
// Calendar (VOY-A3). Mission days are days since 2000-01-01 00:00 (civil, UTC-ish).
// ---------------------------------------------------------------------------------------

/// Days from 2000-01-01 to the proleptic-Gregorian date `y-m-d` (Hinnant's
/// `days_from_civil`, shifted from the 1970 epoch by 10 957 days).
pub(crate) const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 - 10_957
}

/// Inverse of [`days_from_civil`]: `(year, month, day)` of mission day `z`.
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 10_957 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Write mission day `day` (floored to its civil date) as `YYYY-MM-DD`.
fn write_date(out: &mut impl fmt::Write, day: f64) -> fmt::Result {
    // Saturating cast; the scene only ever passes days within the mission.
    let (y, m, d) = civil_from_days(day.floor() as i64);
    write!(out, "{y:04}-{m:02}-{d:02}")
}

const fn day(y: i64, m: i64, d: i64) -> f64 {
    days_from_civil(y, m, d) as f64
}

/// Voyager 2 launch, Cape Canaveral LC-41.
const LAUNCH: f64 = day(1977, 8, 20);
/// Closest approaches (NASA/JPL Voyager mission fact sheets).
const JUPITER_CA: f64 = day(1979, 7, 9);
const SATURN_CA: f64 = day(1981, 8, 26);
const URANUS_CA: f64 = day(1986, 1, 24);
const NEPTUNE_CA: f64 = day(1989, 8, 25);
/// Termination-shock crossing, at 83.7 AU (Stone et al. 2008, Nature 454).
const SHOCK: f64 = day(2007, 8, 30);
const SHOCK_AU: f64 = 83.7;
/// Heliopause crossing into interstellar space, at 119 AU (Stone et al. 2019, Nat.
/// Astron. 3).
const HELIOPAUSE: f64 = day(2018, 11, 5);
const HELIOPAUSE_AU: f64 = 119.0;
/// Voyager 2's hyperbolic excess speed (km/s): ~15.4 km/s at 119 AU, less the Sun's pull.
const V_INF_KMS: f64 = 14.9;

// ---------------------------------------------------------------------------------------
// Planets (VOY-A2).
// ---------------------------------------------------------------------------------------

/// J2000.0 = 2000-01-01 12:00 TT, i.e. mission day 0.5.
const J2000: f64 = 0.5;
/// Sidereal year (days) — the Kepler-law anchor, `P = YEAR · (a / a⊕)^{3/2}`.
const YEAR_DAYS: f64 = 365.256_363;
const A_EARTH: f64 = 1.000_002_61;

struct Body {
    /// Semi-major axis (AU), J2000 (Standish, JPL "Keplerian Elements for Approximate
    /// Positions of the Major Planets", Table 1, 1800–2050 AD).
    a: f64,
    /// Mean longitude at J2000 (degrees), same table.
    l0_deg: f64,
    /// Map label.
    initial: &'static str,
    /// Drawn disc radius (pixels at full size), ramp and level: a false-colour hint of
    /// each world (blue Earth, banded ochre Jupiter, pale-gold Saturn, cyan Uranus,
    /// azure Neptune), not to scale.
    radius: f64,
    ramp: Ramp,
    level: f64,
}

/// Earth, then the four encounter targets, in encounter order.
const BODIES: [Body; 5] = [
    Body {
        a: A_EARTH,
        l0_deg: 100.464_571_66,
        initial: "E",
        radius: 1.45,
        ramp: Ramp::Ice,
        level: 0.58,
    },
    Body {
        a: 5.202_887,
        l0_deg: 34.396_440_51,
        initial: "J",
        radius: 2.6,
        ramp: Ramp::Sienna,
        level: 0.72,
    },
    Body {
        a: 9.536_675_94,
        l0_deg: 49.954_244_23,
        initial: "S",
        radius: 2.2,
        ramp: Ramp::Sienna,
        level: 0.84,
    },
    Body {
        a: 19.189_164_64,
        l0_deg: 313.238_104_51,
        initial: "U",
        radius: 1.7,
        ramp: Ramp::Ice,
        level: 0.82,
    },
    Body {
        a: 30.069_922_76,
        l0_deg: -55.120_029_69,
        initial: "N",
        radius: 1.7,
        ramp: Ramp::Ice,
        level: 0.52,
    },
];
const JUPITER: usize = 1;
const SATURN: usize = 2;
const NEPTUNE: usize = 4;

/// Kepler's third law with Earth's sidereal year as the unit (VOY-A2).
pub(crate) fn period_days(a: f64) -> f64 {
    YEAR_DAYS * (a / A_EARTH).powf(1.5)
}

/// Unwrapped heliocentric longitude (radians) of body `b` on mission day `day`:
/// `L = L₀ + 2π (day − J2000) / P` on a circular orbit.
pub(crate) fn longitude(b: usize, day: f64) -> f64 {
    let body = &BODIES[b];
    body.l0_deg.to_radians() + TAU / period_days(body.a) * (day - J2000)
}

/// Ecliptic position (AU) of body `b` on mission day `day`.
fn planet_pos(b: usize, day: f64) -> [f64; 2] {
    let (s, c) = longitude(b, day).sin_cos_det();
    [BODIES[b].a * c, BODIES[b].a * s]
}

// ---------------------------------------------------------------------------------------
// Two-body mechanics in AU and days (VOY-A5, VOY-A6).
// ---------------------------------------------------------------------------------------

/// The Sun's gravitational parameter, `k²` with Gauss's constant (AU³/day²).
pub(crate) const GM: f64 = 2.959_122_082_9e-4;
/// km/s per AU/day.
pub(crate) const KM_S: f64 = 1_731.456_836_8;

// Two local 2-D helpers, kept on purpose: `geom::v3` (CORE-R60) is 3-D, and the map is
// strictly planar (the scene's stylisation). A core `v2` for two one-liners used by a
// single module would be surface without reuse.
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn norm(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

/// Stumpff functions `(C(z), S(z))`, with series near zero.
fn stumpff(z: f64) -> (f64, f64) {
    if z.abs() < 1e-6 {
        (0.5 - z / 24.0, 1.0 / 6.0 - z / 120.0)
    } else if z > 0.0 {
        let s = z.sqrt();
        ((1.0 - s.cos_det()) / z, (s - s.sin_det()) / (s * z))
    } else {
        let s = (-z).sqrt();
        ((s.cosh() - 1.0) / -z, (s.sinh() - s) / (s * -z))
    }
}

/// Position and velocity `dt` days after `(r0, v0)` on the conic they define: the
/// universal-variable Kepler problem (Curtis, *Orbital Mechanics*, Alg. 3.4), solved by
/// Newton's method, for ellipses and hyperbolas alike.
pub(crate) fn propagate(r0: [f64; 2], v0: [f64; 2], dt: f64) -> ([f64; 2], [f64; 2]) {
    let mu_s = GM.sqrt();
    let rn = norm(r0);
    let vr = dot(r0, v0) / rn;
    let alpha = 2.0 / rn - dot(v0, v0) / GM;
    let mut chi = mu_s * alpha.abs() * dt;
    for _ in 0..60 {
        let z = alpha * chi * chi;
        let (c, s) = stumpff(z);
        let f = rn * vr / mu_s * chi * chi * c + (1.0 - alpha * rn) * chi.powi(3) * s + rn * chi
            - mu_s * dt;
        let df = rn * vr / mu_s * chi * (1.0 - z * s) + (1.0 - alpha * rn) * chi * chi * c + rn;
        let step = f / df;
        chi -= step;
        if step.abs() < 1e-13 * (1.0 + chi.abs()) {
            break;
        }
    }
    let z = alpha * chi * chi;
    let (c, s) = stumpff(z);
    let f = 1.0 - chi * chi / rn * c;
    let g = dt - chi.powi(3) / mu_s * s;
    let r = [f * r0[0] + g * v0[0], f * r0[1] + g * v0[1]];
    let r1 = norm(r);
    let fd = mu_s / (r1 * rn) * (alpha * chi.powi(3) * s - chi);
    let gd = 1.0 - chi * chi / r1 * c;
    (r, [fd * r0[0] + gd * v0[0], fd * r0[1] + gd * v0[1]])
}

/// The prograde single-revolution conic from `r1` to `r2` in `dt` days: the departure
/// and arrival velocities (Lambert's problem in universal variables, Curtis Alg. 5.2, with
/// bisection on `z`, which is robust for every transfer angle below 2π).
pub(crate) fn lambert(r1: [f64; 2], r2: [f64; 2], dt: f64) -> ([f64; 2], [f64; 2]) {
    let (n1, n2) = (norm(r1), norm(r2));
    let cross = r1[0] * r2[1] - r1[1] * r2[0];
    let cos = (dot(r1, r2) / (n1 * n2)).clamp(-1.0, 1.0);
    let theta = if cross >= 0.0 {
        cos.acos()
    } else {
        TAU - cos.acos()
    };
    let a = theta.sin_det() * (n1 * n2 / (1.0 - cos)).sqrt();
    let y = |z: f64| {
        let (c, s) = stumpff(z);
        n1 + n2 + a * (z * s - 1.0) / c.sqrt()
    };
    let time = |z: f64| {
        let (c, s) = stumpff(z);
        let yz = y(z);
        ((yz / c).powf(1.5) * s + a * yz.sqrt()) / GM.sqrt()
    };
    // `z` from deep hyperbolic to just short of a full revolution; lift the low end until
    // `y(z) ≥ 0`, where the time of flight is defined and increases with `z`.
    let (mut lo, mut hi) = (-4.0 * TAU * TAU, TAU * TAU - 1e-9);
    if y(lo) < 0.0 {
        let (mut a0, mut b0) = (lo, hi);
        for _ in 0..200 {
            let m = 0.5 * (a0 + b0);
            if y(m) < 0.0 { a0 = m } else { b0 = m }
        }
        lo = b0;
    }
    for _ in 0..200 {
        let m = 0.5 * (lo + hi);
        if time(m) < dt { lo = m } else { hi = m }
    }
    let yz = y(0.5 * (lo + hi));
    let f = 1.0 - yz / n1;
    let g = a * (yz / GM).sqrt();
    let gd = 1.0 - yz / n2;
    let v1 = [(r2[0] - f * r1[0]) / g, (r2[1] - f * r1[1]) / g];
    let v2 = [(gd * r2[0] - r1[0]) / g, (gd * r2[1] - r1[1]) / g];
    (v1, v2)
}

/// Solar escape speed at `r` AU (AU/day).
#[cfg(test)]
pub(crate) fn escape_speed(r: f64) -> f64 {
    (2.0 * GM / r).sqrt()
}

// ---------------------------------------------------------------------------------------
// Spacecraft path (VOY-A1, VOY-A5..A7).
// ---------------------------------------------------------------------------------------

/// Encounter dates, launch first.
const ENCOUNTERS: [f64; 5] = [LAUNCH, JUPITER_CA, SATURN_CA, URANUS_CA, NEPTUNE_CA];

/// One leg: its conic's departure state.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Leg {
    t0: f64,
    r0: [f64; 2],
    v0: [f64; 2],
}

/// Where the spacecraft is: `(r AU, φ rad, speed AU/day)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Fix {
    pub r: f64,
    pub phi: f64,
    pub speed: f64,
}

/// Samples of the precomputed path, uniform in mission day from launch to the
/// heliopause: the trail and the speed inset are drawn from them, so a frame solves one
/// Kepler problem (the craft's own position) instead of hundreds.
const TABLE_N: usize = 1500;
const TABLE_STEP: f64 = (HELIOPAUSE - LAUNCH) / (TABLE_N - 1) as f64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Path {
    /// The four Lambert legs, then the post-Neptune cruise.
    legs: [Leg; 5],
    table: Box<[Fix]>,
}

impl Path {
    pub(crate) fn new() -> Self {
        let mut legs = [Leg {
            t0: 0.0,
            r0: [0.0; 2],
            v0: [0.0; 2],
        }; 5];
        for (k, leg) in legs.iter_mut().take(4).enumerate() {
            let r0 = planet_pos(k, ENCOUNTERS[k]);
            let r1 = planet_pos(k + 1, ENCOUNTERS[k + 1]);
            let (v0, _) = lambert(r0, r1, ENCOUNTERS[k + 1] - ENCOUNTERS[k]);
            *leg = Leg {
                t0: ENCOUNTERS[k],
                r0,
                v0,
            };
        }
        legs[4] = Self::cruise();
        let mut path = Self {
            legs,
            table: Box::default(),
        };
        path.table = (0..TABLE_N)
            .map(|i| path.fix(LAUNCH + TABLE_STEP * i as f64))
            .collect();
        path
    }

    /// The post-Neptune leg: from Neptune on its encounter date at today's excess speed,
    /// its flight-path angle (off radial, prograde) fitted by bisection so the craft is at
    /// 119 AU on the heliopause date. More angle means a slower climb away from the Sun.
    fn cruise() -> Leg {
        let r0 = planet_pos(NEPTUNE, NEPTUNE_CA);
        let v_inf = V_INF_KMS / KM_S;
        let speed = (v_inf * v_inf + 2.0 * GM / norm(r0)).sqrt();
        let phi = r0[1].atan2(r0[0]);
        let leg = |gamma: f64| {
            let (s, c) = (phi + gamma).sin_cos_det();
            Leg {
                t0: NEPTUNE_CA,
                r0,
                v0: [speed * c, speed * s],
            }
        };
        let reach = |gamma: f64| {
            let l = leg(gamma);
            norm(propagate(l.r0, l.v0, HELIOPAUSE - NEPTUNE_CA).0)
        };
        let (mut lo, mut hi) = (0.0, 1.5);
        for _ in 0..100 {
            let m = 0.5 * (lo + hi);
            if reach(m) > HELIOPAUSE_AU {
                lo = m;
            } else {
                hi = m;
            }
        }
        leg(0.5 * (lo + hi))
    }

    /// The spacecraft on mission day `day` (clamped to launch … heliopause).
    pub(crate) fn fix(&self, day: f64) -> Fix {
        let day = if day.is_nan() {
            LAUNCH
        } else {
            day.clamp(LAUNCH, HELIOPAUSE)
        };
        let leg = &self.legs[ENCOUNTERS[1..].partition_point(|&d| d <= day)];
        let (r, v) = propagate(leg.r0, leg.v0, day - leg.t0);
        Fix {
            r: norm(r),
            phi: r[1].atan2(r[0]),
            speed: norm(v),
        }
    }

    /// The table's samples up to mission day `day`, then `now`, the exact fix on `day`.
    fn history(&self, day: f64, now: Fix) -> impl Iterator<Item = Fix> + '_ {
        let n = ((day - LAUNCH) / TABLE_STEP)
            .floor()
            .clamp(0.0, (TABLE_N - 1) as f64);
        self.table[..=n as usize]
            .iter()
            .copied()
            .chain(std::iter::once(now))
    }

    /// Heliocentric speed (AU/day) arriving at and leaving encounter `k` of
    /// [`ENCOUNTERS`] (1..=4): the assist's jump (VOY-A6).
    #[cfg(test)]
    pub(crate) fn flyby_speeds(&self, k: usize) -> (f64, f64) {
        let leg = &self.legs[k - 1];
        let (_, v_in) = propagate(leg.r0, leg.v0, ENCOUNTERS[k] - leg.t0);
        (norm(v_in), norm(self.legs[k].v0))
    }

    /// Map rotation that points the outbound cruise at the roomier screen axis: left on
    /// a landscape canvas, down on a portrait one (the map's orientation is free; only
    /// its handedness, prograde counter-clockwise, is kept).
    fn view_turn(&self, landscape: bool) -> f64 {
        let out = if landscape { PI } else { -0.5 * PI };
        out - self.table[TABLE_N - 1].phi
    }
}

// ---------------------------------------------------------------------------------------
// Scene clock (VOY-A3) and map (VOY-A4, VOY-A7).
// ---------------------------------------------------------------------------------------

/// Sim seconds at launch, each encounter and the heliopause crossing.
const KEY_T: [f64; 6] = [0.0, 11.0, 20.0, 31.0, 42.0, 54.0];
const KEY_DAY: [f64; 6] = [
    LAUNCH, JUPITER_CA, SATURN_CA, URANUS_CA, NEPTUNE_CA, HELIOPAUSE,
];
/// Acts: the tour, the run out to the heliopause, a hold beyond it.
const TOUR: f64 = 43.0;
const OUTWARD: f64 = 11.0;
const BEYOND: f64 = 5.0;
/// Share of each leg's calendar that runs at a constant rate; the rest is eased, so the
/// clock runs at 25 % of the leg average at the ends (encounters) and never stops.
const CLOCK_LINEAR: f64 = 0.25;

/// Mission day at sim-time `t ∈ [0, 54]` (clamped). Strictly increasing, and exactly the
/// key date at each key time.
pub(crate) fn mission_day(t: f64) -> f64 {
    let t = if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, KEY_T[5])
    };
    let i = KEY_T[1..KEY_T.len() - 1].partition_point(|&k| k <= t);
    let x = (t - KEY_T[i]) / (KEY_T[i + 1] - KEY_T[i]);
    let f = CLOCK_LINEAR * x + (1.0 - CLOCK_LINEAR) * ease::smoothstep(x);
    ease::lerp(KEY_DAY[i], KEY_DAY[i + 1], f)
}

/// Softening radius (AU) of the log map: keeps Earth ~¼ of the way out.
const R0: f64 = 0.6;
/// Heliocentric distance (AU) at the screen edge during the tour; Neptune lands at ~0.97.
const R_MAX: f64 = 34.0;
/// …and once the map has pulled back: the heliopause sits at ~96 % of the radius.
const R_MAX_OUT: f64 = 150.0;

/// Fraction of the screen radius for heliocentric distance `r` (AU) when the edge is at
/// `r_max`: `ρ = ln(1 + r/r₀) / ln(1 + r_max/r₀)`, strictly increasing, `ρ(0) = 0`.
pub(crate) fn rho(r: f64, r_max: f64) -> f64 {
    (r / R0).ln_1p() / (r_max / R0).ln_1p()
}

/// The map edge at sim time `t`: the tour's, easing out to the heliopause's over the
/// first 8 s of the outward act.
pub(crate) fn map_edge(t: f64) -> f64 {
    let u = ease::smoothstep((t - TOUR) / 8.0);
    // Interpolate the log of the edge so the pull-back feels uniform.
    (R_MAX.ln() + (R_MAX_OUT.ln() - R_MAX.ln()) * u).exp()
}

/// Maps heliocentric (r, φ) to fractional cell coordinates, north ecliptic pole up
/// (prograde = counter-clockwise on screen), turned by `turn`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Proj {
    cx: f64,
    cy: f64,
    /// Rows at `ρ = 1`; chosen so `ρ = 1` exactly touches the nearer canvas edge.
    scale: f64,
    r_max: f64,
    turn: f64,
}

impl Proj {
    pub(crate) fn new(size: Size, r_max: f64) -> Self {
        let cx = (f64::from(size.width()) - 1.0).max(0.0) / 2.0;
        let cy = (f64::from(size.height()) - 1.0).max(0.0) / 2.0;
        Self {
            cx,
            cy,
            scale: (cx / ASPECT).min(cy),
            r_max,
            turn: 0.0,
        }
    }

    /// Whether the canvas has more room across than down, in map units.
    fn landscape(&self) -> bool {
        self.cx / ASPECT >= self.cy
    }

    /// Radius in rows of heliocentric distance `r`.
    fn rows(&self, r: f64) -> f64 {
        rho(r, self.r_max) * self.scale
    }

    pub(crate) fn screen(&self, r: f64, phi: f64) -> (f64, f64) {
        let d = self.rows(r);
        let (s, c) = (phi + self.turn).sin_cos_det();
        (self.cx + d * c * ASPECT, self.cy - d * s)
    }
}

// ---------------------------------------------------------------------------------------
// Text.
// ---------------------------------------------------------------------------------------

/// First option that fits `width` with a one-cell margin each side, or none: captions
/// are shortened or skipped, never clipped mid-word.
fn pick<'a>(options: &[&'a str], width: i32) -> Option<&'a str> {
    let room = usize::try_from(width - 2).unwrap_or(0);
    options
        .iter()
        .copied()
        .find(|s| !s.is_empty() && s.chars().count() <= room)
}

/// The first option that fits, centred on row `y` of the columns `0..right`.
fn caption(canvas: &mut Canvas, y: i32, right: i32, options: &[&str], level: u8) {
    if let Some(s) = pick(options, right) {
        let x = (right - s.chars().count() as i32) / 2;
        canvas.text(x, y, s, Ink::new(Role::Text, level));
    }
}

/// Encounter captions, longest first (distances: closest approach to the cloud tops,
/// NASA/JPL Voyager fact sheets). ASCII only, so they read the same in both modes.
const CAPTIONS: [[&str; 3]; 5] = [
    [
        "Launch - Cape Canaveral - 1977-08-20",
        "Launch 1977-08-20",
        "Launch",
    ],
    [
        "Jupiter flyby 1979-07-09 - 570,000 km",
        "Jupiter 1979-07-09",
        "Jupiter",
    ],
    [
        "Saturn flyby 1981-08-26 - 41,000 km",
        "Saturn 1981-08-26",
        "Saturn",
    ],
    [
        "Uranus flyby 1986-01-24 - 81,500 km",
        "Uranus 1986-01-24",
        "Uranus",
    ],
    [
        "Neptune flyby 1989-08-25 - 4,950 km",
        "Neptune 1989-08-25",
        "Neptune",
    ],
];
/// The real crossings, as measured; the readout above shows the model's distance.
const OUTWARD_LINES: [[&str; 3]; 2] = [
    [
        "Termination shock 2007-08-30 - 84 AU",
        "Termination shock 2007",
        "Shock 2007",
    ],
    [
        "Heliopause 2018-11-05 - 119 AU - interstellar space",
        "Heliopause 2018-11-05, 119 AU",
        "Heliopause 2018",
    ],
];
/// Ring labels, longest first.
const RING_LABELS: [[&str; 2]; 2] = [
    ["termination shock 84 AU", "shock"],
    ["heliopause 119 AU", "heliopause"],
];

// ---------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------

/// Seconds (sim) a flyby flash lasts.
const FLASH: f64 = 2.0;
/// Caption window around each encounter, sim seconds, cut short once the mission clock
/// is `CAPTION_YEARS` past the event (it races after Neptune).
const CAPTION_BEFORE: f64 = 0.8;
const CAPTION_AFTER: f64 = 4.5;
const CAPTION_YEARS: f64 = 1.5;
/// One background star per this many cells.
const CELLS_PER_STAR: usize = 40;
/// The craft's trail: pale starlight (VOY-A10), 1 px, at `TRAIL_LEVEL` (oldest the
/// dimmest end); planet orbits faint dotted `Starlight` at `ORBIT_LEVEL` (ADR-0017, M14).
const TRAIL: Ramp = Ramp::Starlight;
const TRAIL_LEVEL: (f64, f64) = (40.0, 70.0);
const ORBIT_LEVEL: u8 = 34;
/// The giants drawn as a plus of five pixels, the rest as one (ADR-0017: 1–2 px planets).
const PLUS: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];

/// The encounter (0 = launch … 4 = Neptune) whose caption window holds sim time `t`.
fn encounter_at(t: f64) -> Option<usize> {
    (0..5).find(|&k| {
        let before = if k == 0 { 0.0 } else { CAPTION_BEFORE };
        (-before..CAPTION_AFTER).contains(&(t - KEY_T[k]))
            && mission_day(t) < KEY_DAY[k] + CAPTION_YEARS * YEAR_DAYS
    })
}

#[derive(Debug)]
pub struct VoyagerJourney {
    /// Sim time within the current cycle.
    t: f64,
    timeline: Timeline,
    path: Path,
    size: Size,
    proj: Proj,
    /// The Sun's tone-mapped glow as `(x, y, ink)` pixels, built in `resize`.
    sun: Vec<(i32, i32, Ink)>,
    px: Pixels,
    dots: Dots,
    /// Background stars as dot coordinates and levels, rebuilt on resize.
    stars: Vec<(i32, i32, u8)>,
    /// The mission day and the craft's fix this frame, set by `rasterise`.
    today: f64,
    fix: Fix,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl VoyagerJourney {
    fn new() -> Self {
        let path = Path::new();
        Self {
            t: 0.0,
            timeline: Timeline::new(&[("tour", TOUR), ("outward", OUTWARD), ("beyond", BEYOND)]),
            path,
            size: Size::new(1, 1),
            proj: Proj::default(),
            px: Pixels::default(),
            dots: Dots::default(),
            sun: Vec::new(),
            stars: Vec::new(),
            today: LAUNCH,
            fix: Fix::default(),
            dirty: false,
        }
    }

    /// A circle of `rows` radius around cell point `(cx, cy)` on the dot lattice, every
    /// `stride`-th dot of ~one per dot of arc, left out of the inset (a flyby's flash can
    /// reach past the map's rim).
    fn ring(&mut self, cx: f64, cy: f64, rows: f64, stride: usize, ink: Ink) {
        if !(rows.is_finite() && rows > 0.0) {
            return;
        }
        let (ox, oy) = Dots::from_cells(cx + 0.5, cy + 0.5);
        let rd = rows * f64::from(Dots::CELL_H);
        let n = ((TAU * rd).ceil() as usize).clamp(8, 8192);
        for i in (0..n).step_by(stride.max(1)) {
            let (s, c) = (TAU * i as f64 / n as f64).sin_cos_det();
            let (x, y) = (ox + rd * c, oy - rd * s);
            self.dots.plot(x, y, ink);
        }
    }

    /// How far the heliosphere's rings have faded in (0 during the tour).
    fn rings_shown(&self) -> f64 {
        ease::smoothstep((self.t - TOUR - 2.0) / 3.0)
    }

    fn rasterise(&mut self) {
        self.proj.r_max = map_edge(self.t);
        self.today = mission_day(self.t);
        self.fix = self.path.fix(self.today);
        let today = self.today;
        let proj = self.proj;
        self.dots.clear();
        for &(x, y, level) in &self.stars {
            self.dots.set(x, y, Ink::new(Role::Star, level));
        }
        // Planet orbits: faint dotted circles.
        for b in &BODIES {
            let ink = Ink::new(Role::Ramp(Ramp::Starlight), ORBIT_LEVEL);
            self.ring(proj.cx, proj.cy, proj.rows(b.a), 3, ink);
        }
        // The heliosphere's edges, once the map has pulled back far enough to hold them;
        // each brightens once the craft has crossed it, the heliopause drawn denser.
        let show = self.rings_shown();
        if show > 0.05 {
            for (au, when, stride) in [(SHOCK_AU, SHOCK, 4), (HELIOPAUSE_AU, HELIOPAUSE, 1)] {
                if proj.rows(au) > proj.scale * 1.02 {
                    continue;
                }
                let level = if today >= when { 0.66 } else { 0.36 };
                self.ring(
                    proj.cx,
                    proj.cy,
                    proj.rows(au),
                    stride,
                    ramp(Ramp::Plasma, level * show),
                );
            }
        }
        self.draw_trail(today);
        // Flyby flashes: rings expanding from the planet (sim-time driven).
        for k in 1..5 {
            let age = self.t - KEY_T[k];
            if (0.0..FLASH).contains(&age) {
                let (x, y) = proj.screen(BODIES[k].a, longitude(k, today));
                let u = age / FLASH;
                let ink = ramp(BODIES[k].ramp, 0.9 * (1.0 - u).powf(0.7));
                self.ring(x, y, proj.scale * (0.03 + 0.14 * u), 1, ink);
            }
        }
        // Pixels: the Sun, the planets, then the craft on top.
        self.px.clear();
        for &(x, y, ink) in &self.sun {
            self.px.set(x, y, ink);
        }
        let small = (proj.scale / 24.0).clamp(0.45, 1.0);
        for (k, b) in BODIES.iter().enumerate() {
            let (x, y) = proj.screen(b.a, longitude(k, today));
            // Each giant flares as the craft swings past it.
            let age = self.t - KEY_T[k];
            let flash = if k > 0 && (0.0..FLASH).contains(&age) {
                0.25 * (1.0 - age / FLASH)
            } else {
                0.0
            };
            self.planet(k, (x + 0.5, 2.0 * y + 1.0), b.radius * small, flash);
        }
        let (x, y) = proj.screen(self.fix.r, self.fix.phi);
        let (cx, cy) = ((x + 0.5).floor() as i32, (2.0 * y + 1.0).floor() as i32);
        // The craft: a white point with a faint cross, the hero of the frame.
        for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            self.px
                .set(cx + ox, cy + oy, Ink::new(Role::Ramp(Ramp::Starlight), 90));
        }
        self.px.set(cx, cy, Ink::new(Role::Star, 255));
    }

    /// The trail from launch to `today`: a 1-px pale starlight line, oldest dimmest
    /// (ADR-0017: `TRAIL_LEVEL`).
    fn draw_trail(&mut self, today: f64) {
        let proj = self.proj;
        let n = ((today - LAUNCH) / TABLE_STEP).max(1.0);
        let mut prev: Option<(f64, f64)> = None;
        for (i, f) in self.path.history(today, self.fix).enumerate() {
            let (x, y) = proj.screen(f.r, f.phi);
            let p = Dots::from_cells(x + 0.5, y + 0.5);
            let u = (i as f64 / n).min(1.0);
            let level = TRAIL_LEVEL.0 + (TRAIL_LEVEL.1 - TRAIL_LEVEL.0) * u;
            if let Some(q) = prev {
                let ink = Ink::new(Role::Ramp(TRAIL), ink::level(level));
                self.dots.line(q.0, q.1, p.0, p.1, ink);
            }
            prev = Some(p);
        }
    }

    /// Planet `k` at the pixel holding point `c` in its muted true colour, brightened by
    /// `flash`: one pixel, or a plus of five for the giants Jupiter and Saturn (ADR-0017;
    /// symmetric about its centre pixel, VOY-A4). `_r` is kept for the call sites.
    fn planet(&mut self, k: usize, (cx, cy): (f64, f64), _r: f64, flash: f64) {
        let (x, y) = (cx.floor() as i32, cy.floor() as i32);
        let b = &BODIES[k];
        let ink = ramp(b.ramp, (b.level + flash).min(0.95));
        let stamp: &[(i32, i32)] = if k == JUPITER || k == SATURN {
            &PLUS
        } else {
            &PLUS[..1]
        };
        for &(dx, dy) in stamp {
            self.px.set(x + dx, y + dy, ink);
        }
    }

    /// Initials beside the planets, and the heliosphere rings' names beside their
    /// right-hand edge, off the text rows, the title row and the inset.
    fn draw_labels(&self, canvas: &mut Canvas) {
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        let free = |x: i32, y: i32, len: i32| (2..h - 1).contains(&y) && x >= 0 && x + len < w;
        let proj = self.proj;
        let today = self.today;
        let small = (proj.scale / 24.0).clamp(0.45, 1.0);
        for (k, b) in BODIES.iter().enumerate() {
            let (x, y) = proj.screen(b.a, longitude(k, today));
            let lx = (x + 0.5 + b.radius * small).round() as i32 + 1;
            let ly = y.round() as i32;
            if free(lx, ly, 1) {
                canvas.text(lx, ly, b.initial, Ink::new(Role::Text, 110));
            }
        }
        let show = self.rings_shown();
        if show < 0.5 || proj.rows(HELIOPAUSE_AU) > proj.scale * 1.02 {
            return;
        }
        let edge = (proj.cx + proj.rows(HELIOPAUSE_AU) * ASPECT).round() as i32 + 2;
        let cy = proj.cy.round() as i32;
        for (labels, (dy, level)) in RING_LABELS.iter().zip([(1, 0.36), (-1, 0.66)]) {
            if let Some(s) = pick(labels, w - edge + 1)
                && free(edge, cy + dy, s.len() as i32)
            {
                canvas.text(edge, cy + dy, s, ramp(Ramp::Plasma, level));
            }
        }
    }
}

impl Default for VoyagerJourney {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator for VoyagerJourney {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.proj = Proj::new(size, map_edge(self.t));
        self.proj.turn = self.path.view_turn(self.proj.landscape());
        self.px.resize(size);
        self.dots.resize(size);
        // The Sun: a point source with a soft halo, tone-mapped once per resize (asinh, so
        // a white-hot core fades through Ember into the dark within ~6 % of the map
        // radius) and stamped onto the pixels every frame. The core is the top of the
        // Ember ramp, the brightest ink of every theme (in mono a 5800 K blackbody would
        // read mid-grey).
        let tone = Tone::asinh(0.05, 3.2);
        let (sx, sy) = (self.proj.cx + 0.5, 2.0 * self.proj.cy + 1.0);
        let reach = (self.proj.scale * 0.14).max(1.0);
        let core = (reach / 6.0).max(0.4);
        self.sun.clear();
        for y in (sy - reach).floor() as i32..=(sy + reach).ceil() as i32 {
            for x in (sx - reach).floor() as i32..=(sx + reach).ceil() as i32 {
                let d = (f64::from(x) + 0.5 - sx).hypot(f64::from(y) + 0.5 - sy);
                // A pixel gathers four dots' worth of the halo profile.
                let v = 4.0 / (1.0 + (d / core).powi(2)) * (1.0 - d / reach).max(0.0);
                // The pixels at the centre are the white-hot core itself.
                let level = if d < 0.75 { 255 } else { tone.level(v as f32) };
                if level > 0 {
                    self.sun
                        .push((x, y, ramp(Ramp::Ember, f64::from(level) / 255.0)));
                }
            }
        }
        let dot = self.dots.dot_size();
        let n = size.area() / CELLS_PER_STAR;
        self.stars.clear();
        self.stars.extend((0..n).map(|_| {
            (
                ctx.rng.random_range(0..i32::from(dot.width())),
                ctx.rng.random_range(0..i32::from(dot.height())),
                ctx.rng.random_range(30..120u8),
            )
        }));
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_finite() && dt > 0.0 {
            self.t = geom::wrap(self.t + dt, self.timeline.total());
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (w, h) = (
            i32::from(canvas.size().width()),
            i32::from(canvas.size().height()),
        );
        if w == 0 || h == 0 {
            return;
        }
        self.dots.blit(canvas);
        self.px.blit(canvas, |_, _, top, bottom| {
            let ink = if top.role == Role::Background {
                bottom
            } else {
                top
            };
            match ink.role {
                Role::Background => None,
                _ if ink.level > 245 => Some(('O', ink)),
                _ if ink.level > 150 => Some(('o', ink)),
                _ => Some(('.', ink)),
            }
        });
        self.draw_labels(canvas);
        let today = self.today;
        readout(canvas, today, self.fix);
        let right = w;
        if h < 3 {
            return;
        }
        if let Some(k) = encounter_at(self.t) {
            caption(canvas, 1, right, &CAPTIONS[k], 255);
        } else if self.timeline.at(self.t).index > 0 {
            // The shock's line from a year before the craft reaches it until the caption
            // window closes; the heliopause's from the crossing, with the title's caption.
            let shock = (SHOCK - YEAR_DAYS..SHOCK + CAPTION_YEARS * YEAR_DAYS).contains(&today);
            let line = if today >= HELIOPAUSE {
                Some(&OUTWARD_LINES[1])
            } else {
                shock.then_some(&OUTWARD_LINES[0])
            };
            if let Some(text) = line {
                caption(canvas, 1, right, text, 235);
            }
        }
    }

    fn caption(&self) -> Option<&'static str> {
        match encounter_at(self.t) {
            Some(k) if k > 0 => Some(ACTS[k - 1]),
            _ if self.timeline.at(self.t).index == 0 => None,
            // By date: inside the heliosphere until the craft crosses the heliopause.
            _ if mission_day(self.t) < HELIOPAUSE => Some(ACTS[4]),
            _ => Some(ACTS[5]),
        }
    }
}

/// Top-left date / distance / speed readout, shortened to fit.
fn readout(canvas: &mut Canvas, today: f64, fix: Fix) {
    let mut date = StackText::<16>::new();
    if write_date(&mut date, today).is_err() {
        return;
    }
    let d = date.as_str();
    let (r, v) = (fix.r, fix.speed * KM_S);
    let mut long = StackText::<64>::new();
    let mut mid = StackText::<32>::new();
    let _ = write!(long, "VOYAGER 2  {d}  {r:.1} AU  {v:.1} km/s");
    let _ = write!(mid, "{d}  {r:.1} AU");
    let width = i32::from(canvas.size().width());
    if let Some(s) = pick(&[long.as_str(), mid.as_str(), d], width) {
        canvas.text(1, 0, s, Ink::new(Role::Text, 200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const ENCOUNTER_DATES: [&str; 5] = [
        "1977-08-20",
        "1979-07-09",
        "1981-08-26",
        "1986-01-24",
        "1989-08-25",
    ];
    const SIZES: [(u16, u16); 6] = [(200, 50), (100, 30), (80, 24), (41, 13), (20, 8), (3, 2)];

    fn date_str(day: f64) -> String {
        let mut s = String::new();
        write_date(&mut s, day).unwrap();
        s
    }

    #[test]
    fn voy_a1_spacecraft_meets_each_planet_at_its_encounter_date() {
        let path = Path::new();
        for k in 0..5 {
            let day = KEY_DAY[k];
            let f = path.fix(day);
            let p = planet_pos(k, day);
            let (a, lon) = (BODIES[k].a, longitude(k, day));
            let miss = (f.r * f.phi.cos_det() - p[0]).hypot(f.r * f.phi.sin_det() - p[1]);
            assert!(miss < 1e-6 * a, "encounter {k}: {miss} AU off");
            assert_eq!(mission_day(KEY_T[k]), day, "clock at encounter {k}");
            for (w, h) in SIZES {
                let pr = Proj::new(Size::new(w, h), R_MAX);
                let (sx, sy) = pr.screen(f.r, f.phi);
                let (px, py) = pr.screen(a, lon);
                assert!((sx - px).hypot(sy - py) < 0.5, "encounter {k} at {w}x{h}");
            }
        }
    }

    #[test]
    fn voy_a1_path_is_continuous_outbound_and_prograde() {
        let path = Path::new();
        let n = 6000;
        let mut prev = path.fix(LAUNCH);
        for i in 1..=n {
            let day = LAUNCH + (HELIOPAUSE - LAUNCH) * f64::from(i) / f64::from(n);
            let cur = path.fix(day);
            let dphi = geom::wrap_angle(cur.phi - prev.phi);
            // Outbound (within 1 % while leaving Earth's perihelion) and prograde.
            assert!(cur.r >= prev.r * 0.99, "r fell at day {day}");
            assert!(dphi >= -1e-9, "retrograde at day {day}");
            assert!(
                dphi < 4f64.to_radians() && (cur.r - prev.r).abs() < 0.2,
                "jump at {day}"
            );
            prev = cur;
        }
    }

    #[test]
    fn voy_a2_planet_periods_follow_kepler_and_match_real_within_1_percent() {
        // Real sidereal periods (days), NASA planetary fact sheets.
        let real = [365.256, 4_332.59, 10_759.22, 30_688.5, 60_182.0];
        for (i, (b, real)) in BODIES.iter().zip(real).enumerate() {
            let p = period_days(b.a);
            let err = (p - real).abs() / real;
            assert!(err < 0.01, "body {i}: model {p} d vs real {real} d");
            let rate = longitude(i, 1000.0) - longitude(i, 0.0);
            assert!((rate / 1000.0 - TAU / p).abs() < 1e-12, "body {i} rate");
            for b2 in &BODIES {
                let ratio = period_days(b.a) / period_days(b2.a);
                assert!((ratio - (b.a / b2.a).powf(1.5)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn voy_a3_calendar_conversion_round_trips_and_is_exact_at_encounters() {
        assert_eq!(days_from_civil(2000, 1, 1), 0);
        assert_eq!(days_from_civil(1977, 8, 20), -8_169);
        assert_eq!(civil_from_days(days_from_civil(1980, 2, 29)), (1980, 2, 29));
        for z in -9000..7000 {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        for (k, want) in ENCOUNTER_DATES.iter().enumerate() {
            assert_eq!(date_str(mission_day(KEY_T[k])), *want, "encounter {k}");
        }
        assert_eq!(date_str(mission_day(KEY_T[5])), "2018-11-05");
    }

    #[test]
    fn voy_a3_mission_clock_is_strictly_monotone_and_slows_at_encounters() {
        let mut prev = mission_day(0.0);
        let n = 54 * 240;
        for i in 1..=n {
            let t = 54.0 * f64::from(i) / f64::from(n);
            let d = mission_day(t);
            assert!(d > prev, "clock not strictly increasing at t={t}");
            prev = d;
        }
        let h = 0.05;
        for k in 1..5 {
            let before = (mission_day(KEY_T[k]) - mission_day(KEY_T[k] - h)) / h;
            let after = (mission_day(KEY_T[k] + h) - mission_day(KEY_T[k])) / h;
            let avg_before = (KEY_DAY[k] - KEY_DAY[k - 1]) / (KEY_T[k] - KEY_T[k - 1]);
            let avg_after = (KEY_DAY[k + 1] - KEY_DAY[k]) / (KEY_T[k + 1] - KEY_T[k]);
            assert!(before < 0.5 * avg_before, "encounter {k}: {before} d/s");
            assert!(after < 0.5 * avg_after, "encounter {k}: {after} d/s");
        }
    }

    #[test]
    fn voy_a4_radial_map_is_strictly_monotone_and_fits_neptune() {
        assert_eq!(rho(0.0, R_MAX), 0.0);
        assert!((rho(R_MAX, R_MAX) - 1.0).abs() < 1e-12);
        for r_max in [R_MAX, 70.0, R_MAX_OUT] {
            let mut prev = 0.0;
            for i in 1..=20_000 {
                let v = rho(0.01 * f64::from(i), r_max);
                assert!(v > prev, "rho not strictly increasing");
                prev = v;
            }
        }
        let rn = rho(BODIES[NEPTUNE].a, R_MAX);
        assert!(rn < 1.0 && rn > 0.9, "Neptune at {rn} of the radius");
        assert!(rho(1.0, R_MAX) > 0.2);
        for (w, h) in SIZES {
            let p = Proj::new(Size::new(w, h), R_MAX);
            for i in 0..360 {
                let (x, y) = p.screen(BODIES[NEPTUNE].a, f64::from(i).to_radians());
                assert!((0.0..f64::from(w)).contains(&x.round()), "{w}x{h}: x {x}");
                assert!((0.0..f64::from(h)).contains(&y.round()), "{w}x{h}: y {y}");
            }
        }
    }

    #[test]
    fn voy_a4_every_planet_is_drawn_throughout_the_tour() {
        let mut g = VoyagerJourney::new();
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(3, size);
        g.resize(size, &mut ctx);
        g.prepare();
        for s in 0..43 {
            g.t = f64::from(s);
            g.rasterise();
            let craft = g.fix;
            let craft = g.proj.screen(craft.r, craft.phi);
            for (k, b) in BODIES.iter().enumerate() {
                let (x, y) = g.proj.screen(b.a, longitude(k, g.today));
                if (x - craft.0).hypot(2.0 * (y - craft.1)) < 1.5 {
                    // The craft is at this planet and drawn over it.
                    continue;
                }
                let ink =
                    g.px.get((x + 0.5).floor() as i32, (2.0 * y + 1.0).floor() as i32);
                assert_eq!(
                    ink.map(|i| i.role),
                    Some(Role::Ramp(b.ramp)),
                    "{k} at t={s}"
                );
            }
        }
    }

    #[test]
    fn voy_a4_planets_are_round_stamps_symmetric_about_their_centre() {
        for (w, h) in [(80, 24), (200, 50), (120, 40)] {
            let size = Size::new(w, h);
            let mut g = VoyagerJourney::new();
            g.resize(size, &mut Ctx::new(3, size));
            g.prepare();
            for s in [0.0, 7.3, 18.9, 31.4] {
                g.t = s;
                g.rasterise();
                let small = (g.proj.scale / 24.0).clamp(0.45, 1.0);
                for (k, b) in BODIES.iter().enumerate() {
                    let (x, y) = g.proj.screen(b.a, longitude(k, g.today));
                    g.px.clear();
                    g.planet(k, (x + 0.5, 2.0 * y + 1.0), b.radius * small, 0.0);
                    let c = ((x + 0.5).floor() as i32, (2.0 * y + 1.0).floor() as i32);
                    let lit = |p: (i32, i32)| g.px.get(p.0, p.1).is_some_and(|i| i != Ink::BLANK);
                    assert!(lit(c), "{w}x{h} t {s}: planet {k} has no centre pixel");
                    let (pw, ph) = (i32::from(w), 2 * i32::from(h));
                    for py in 0..ph {
                        for px in 0..pw {
                            if !lit((px, py)) {
                                continue;
                            }
                            // The whole silhouette is visible: shading only modulates it.
                            let ink = g.px.get(px, py).unwrap();
                            assert!(
                                f64::from(ink.level) >= sciart_core::ink::MIN_VISIBLE_LEVEL,
                                "{ink:?}"
                            );
                            // Mirror images across both axes through the centre pixel,
                            // where they fall on screen.
                            for m in [(2 * c.0 - px, py), (px, 2 * c.1 - py)] {
                                let on = (0..pw).contains(&m.0) && (0..ph).contains(&m.1);
                                assert!(
                                    !on || lit(m),
                                    "{w}x{h} t {s}: planet {k} lit at {px},{py} not at {m:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn voy_a5_each_leg_is_one_kepler_conic_that_lands_on_time() {
        let path = Path::new();
        for (k, leg) in path.legs[..4].iter().enumerate() {
            let energy = |r: [f64; 2], v: [f64; 2]| 0.5 * dot(v, v) - GM / norm(r);
            let e0 = energy(leg.r0, leg.v0);
            let span = ENCOUNTERS[k + 1] - leg.t0;
            for i in 1..=20 {
                let (r, v) = propagate(leg.r0, leg.v0, span * f64::from(i) / 20.0);
                assert!(
                    (energy(r, v) - e0).abs() < 1e-9 * e0.abs(),
                    "leg {k} energy"
                );
                // Angular momentum too.
                let h = |r: [f64; 2], v: [f64; 2]| r[0] * v[1] - r[1] * v[0];
                assert!((h(r, v) - h(leg.r0, leg.v0)).abs() < 1e-9 * h(leg.r0, leg.v0));
            }
            let (end, _) = propagate(leg.r0, leg.v0, span);
            let target = planet_pos(k + 1, ENCOUNTERS[k + 1]);
            assert!(
                (end[0] - target[0]).hypot(end[1] - target[1]) < 1e-6,
                "leg {k}"
            );
        }
        // Propagation round-trips, for an ellipse and a hyperbola.
        for v in [[0.0, 0.017], [0.0, 0.03]] {
            let (r, w) = propagate([1.0, 0.0], v, 400.0);
            let (back, _) = propagate(r, [-w[0], -w[1]], 400.0);
            assert!(
                (back[0] - 1.0).abs() < 1e-9 && back[1].abs() < 1e-9,
                "{back:?}"
            );
        }
    }

    #[test]
    fn voy_a6_the_giants_speed_it_up_past_solar_escape() {
        let path = Path::new();
        for (k, name) in [(1, "Jupiter"), (2, "Saturn"), (3, "Uranus")] {
            let (v_in, v_out) = path.flyby_speeds(k);
            assert!(
                v_out > v_in * 1.05,
                "{name}: {} -> {} km/s",
                v_in * KM_S,
                v_out * KM_S
            );
        }
        // Bound to the Sun before Jupiter, escaping after it.
        let before = path.fix(JUPITER_CA - 30.0);
        assert!(before.speed < escape_speed(before.r));
        let (_, out) = path.flyby_speeds(1);
        assert!(out > escape_speed(BODIES[JUPITER].a), "{} km/s", out * KM_S);
        for day in [
            SATURN_CA + 10.0,
            URANUS_CA + 10.0,
            NEPTUNE_CA + 10.0,
            HELIOPAUSE,
        ] {
            let f = path.fix(day);
            assert!(f.speed > escape_speed(f.r), "bound again at day {day}");
        }
        // It leaves at about today's 15 km/s.
        let v = path.fix(HELIOPAUSE).speed * KM_S;
        assert!((14.0..17.5).contains(&v), "cruise {v} km/s");
    }

    #[test]
    fn voy_a7_it_crosses_the_heliopause_where_and_when_it_did() {
        let path = Path::new();
        let f = path.fix(HELIOPAUSE);
        assert!((f.r - HELIOPAUSE_AU).abs() < 0.01, "{} AU", f.r);
        let shock = path.fix(SHOCK).r;
        assert!(
            (shock - SHOCK_AU).abs() < 1.0,
            "termination shock at {shock} AU"
        );
        // The map pulls back from Neptune's edge to the heliopause's.
        assert!((map_edge(0.0) - R_MAX).abs() < 1e-9);
        assert!((map_edge(TOUR + OUTWARD) - R_MAX_OUT).abs() < 1e-9);
        assert!(rho(HELIOPAUSE_AU, R_MAX_OUT) < 1.0);
        let mut prev = 0.0;
        for i in 0..=600 {
            let e = map_edge(f64::from(i) * 0.1);
            assert!(e >= prev);
            prev = e;
        }
    }

    #[test]
    fn voy_a10_trail_is_starlight_and_planets_are_round() {
        // The trail is not amber: at 25 s it is drawn in `TRAIL`, and no braille cell on
        // the map is Ember apart from the Sun's and Jupiter's own pixels (half blocks).
        let mut stage = Stage::new(build(), 5, Size::new(120, 40), GlyphMode::Unicode);
        for _ in 0..(25 * 60) {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let braille = |c: &&Cell| (0x2801..=0x28FF).contains(&(c.ch as u32));
        let trail = frame.cells().iter().filter(braille);
        assert!(
            trail
                .clone()
                .filter(|c| c.ink.role == Role::Ramp(TRAIL))
                .count()
                > 20
        );
        assert_eq!(
            trail
                .filter(|c| c.ink.role == Role::Ramp(Ramp::Ember))
                .count(),
            0
        );
        assert_ne!(BODIES[SATURN].ramp, Ramp::Ember);
        // M14: the trail's braille is at most `TRAIL_LEVEL.1`.
        assert!(
            frame
                .cells()
                .iter()
                .filter(|c| (0x2801..=0x28FF).contains(&(c.ch as u32)))
                .filter(|c| c.ink.role == Role::Ramp(TRAIL))
                .all(|c| f64::from(c.ink.level) <= TRAIL_LEVEL.1 + 1.0)
        );
    }

    #[test]
    fn voy_captions_never_clip_and_scene_animates() {
        for (w, h) in [(200, 50), (100, 30), (30, 10), (12, 4)] {
            let mut stage = Stage::new(build(), 5, Size::new(w, h), GlyphMode::Unicode);
            let first = stage.render().to_lines();
            for _ in 0..60 {
                stage.step(SIM_DT);
            }
            assert_ne!(
                first,
                stage.render().to_lines(),
                "{w}x{h}: static after 1 s"
            );
            for _ in 0..(59 * 12) {
                for _ in 0..5 {
                    stage.step(SIM_DT);
                }
                let frame = stage.render();
                for y in 0..i32::from(h) {
                    let edge = frame.get(i32::from(w) - 1, y).unwrap();
                    assert!(
                        edge.ink.role != Role::Text,
                        "{w}x{h}: text at the edge, row {y}"
                    );
                }
            }
        }
    }

    #[test]
    fn voy_a6_neptune_slows_it_and_the_cruise_keeps_todays_speed() {
        let path = Path::new();
        let (v_in, v_out) = path.flyby_speeds(4);
        let drop = (v_in - v_out) * KM_S;
        assert!((1.0..5.0).contains(&drop), "Neptune takes {drop} km/s");
        let v = path.fix(HELIOPAUSE).speed * KM_S;
        assert!(
            (14.8..16.0).contains(&v),
            "cruise {v} km/s at the heliopause"
        );
    }

    #[test]
    fn voy_a7_the_craft_stays_inside_the_frame_at_the_heliopause() {
        for (w, h) in [(200, 50), (120, 40), (100, 30), (80, 24), (40, 40)] {
            let size = Size::new(w, h);
            let mut g = VoyagerJourney::new();
            g.resize(size, &mut Ctx::new(1, size));
            g.prepare();
            g.t = KEY_T[5];
            g.rasterise();
            let f = g.fix;
            let (x, y) = g.proj.screen(f.r, f.phi);
            let margin = x
                .min(f64::from(w) - 1.0 - x)
                .min(y)
                .min(f64::from(h) - 2.0 - y);
            assert!(margin >= 2.0, "{w}x{h}: craft at {x:.1},{y:.1}");
        }
    }

    /// A fresh scene at `size`, `t` seconds into the cycle, and its rendered rows.
    fn frame_at(size: Size, t: f64) -> (VoyagerJourney, Vec<String>) {
        let mut g = VoyagerJourney::new();
        g.resize(size, &mut Ctx::new(2, size));
        g.prepare();
        g.t = t;
        g.rasterise();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        (g, canvas.to_lines())
    }

    #[test]
    fn voy_a8_captions_end_within_two_years_of_their_event() {
        let size = Size::new(200, 50);
        let events = [
            ("Launch", LAUNCH),
            ("Jupiter", JUPITER_CA),
            ("Saturn", SATURN_CA),
            ("Uranus", URANUS_CA),
            ("Neptune", NEPTUNE_CA),
            ("Termination shock", SHOCK),
        ];
        let mut seen = [false; 7];
        for i in 0..(TOUR + OUTWARD + BEYOND) as usize * 20 {
            let t = i as f64 * 0.05;
            let (g, rows) = frame_at(size, t);
            let (row, today) = (&rows[1], g.today);
            for (k, (name, day)) in events.iter().enumerate() {
                if row.contains(name) {
                    seen[k] = true;
                    assert!(
                        today < day + 2.0 * YEAR_DAYS,
                        "t {t}: {name} caption on {}",
                        date_str(today)
                    );
                }
            }
            // The heliopause line and the subtitle switch on the same frame.
            let crossed = row.contains("Heliopause");
            seen[6] |= crossed;
            assert_eq!(crossed, today >= HELIOPAUSE, "t {t}: {row}");
            assert_eq!(g.caption() == Some("interstellar space"), crossed, "t {t}");
        }
        assert!(seen.iter().all(|&s| s), "{seen:?}");
    }

    #[test]
    fn voy_a8_a11_no_inset_and_no_flyby_log() {
        // ADR-0017 (M14): the speed inset and the flyby date list are gone; the map uses
        // the whole frame. Below the readout row no text mentions speed or flybys, and
        // every planet is at most a plus of five pixels.
        for size in [Size::new(200, 50), Size::new(120, 40)] {
            let mut stage = Stage::new(build(), 5, size, GlyphMode::Unicode);
            for _ in 0..(30 * 60) {
                stage.step(SIM_DT);
                let lines = stage.render().to_lines();
                for l in &lines[1..] {
                    assert!(
                        !(l.contains("km/s")
                            || l.contains("flybys (model)")
                            || l.contains("heliocentric")
                            || l.contains("escape")),
                        "{size:?}: {l:?}"
                    );
                }
            }
        }
        let mut g = VoyagerJourney::new();
        let size = Size::new(200, 50);
        g.resize(size, &mut Ctx::new(3, size));
        g.prepare();
        for k in 0..BODIES.len() {
            g.px.clear();
            g.planet(k, (100.5, 50.5), 3.0, 0.0);
            let lit = (0..100)
                .flat_map(|y| (0..200).map(move |x| (x, y)))
                .filter(|&(x, y)| g.px.get(x, y).is_some_and(|i| i != Ink::BLANK))
                .count();
            assert!(lit == 1 || lit == 5, "planet {k}: {lit} px");
        }
    }

    #[test]
    fn voy_a9_planets_are_labelled_by_initial() {
        for (w, h) in [(200, 50), (80, 24)] {
            let size = Size::new(w, h);
            let (g, rows) = frame_at(size, 5.0);
            let (proj, day) = (g.proj, g.today);
            for (k, name) in ['E', 'J', 'S', 'U', 'N'].into_iter().enumerate() {
                let (x, y) = proj.screen(BODIES[k].a, longitude(k, day));
                let row: Vec<char> = rows[y.round() as usize].chars().collect();
                let near = (x.round() as usize + 1..(x.round() as usize + 6).min(row.len()))
                    .any(|c| row[c] == name);
                assert!(near, "{w}x{h}: no {name} beside ({x:.0}, {y:.0})");
            }
        }
    }

    #[test]
    fn voy_a4_the_sun_core_tops_its_ramp_in_every_theme() {
        // Theme-independent: a ramp's top level is its brightest in every theme,
        // including mono, where a 5800 K blackbody would read mid-grey.
        let size = Size::new(200, 50);
        let mut g = VoyagerJourney::new();
        g.resize(size, &mut Ctx::new(1, size));
        g.prepare();
        let core = (99..=101)
            .flat_map(|x| (49..=51).map(move |y| (x, y)))
            .filter_map(|(x, y)| g.px.get(x, y))
            .max_by_key(|i| i.level)
            .unwrap();
        assert_eq!(core, ramp(Ramp::Ember, 1.0));
        for y in 0..100 {
            for x in 0..200 {
                let ink = g.px.get(x, y).unwrap();
                if (x - 100).abs() + (y - 50).abs() > 3 {
                    assert!(
                        ink.level < 250 || !matches!(ink.role, Role::Ramp(_)),
                        "{x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn voy_a5_lambert_matches_a_hohmann_transfer() {
        // Earth (1 AU) to Mars-ish (1.524 AU) half an ellipse away: the Hohmann time and
        // perihelion speed come back (angle just short of π to keep the plane defined).
        let (r1, r2): (f64, f64) = (1.0, 1.524);
        let a = 0.5 * (r1 + r2);
        let t = PI * (a.powi(3) / GM).sqrt();
        let th = PI - 1e-6;
        let (v1, _) = lambert([r1, 0.0], [r2 * th.cos_det(), r2 * th.sin_det()], t);
        let vp = (GM * (2.0 / r1 - 1.0 / a)).sqrt();
        assert!((norm(v1) - vp).abs() < 1e-4 * vp, "{} vs {vp}", norm(v1));
    }
}
