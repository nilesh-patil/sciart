//! A rocker–bogie rover crossing Martian dunes (SPEC-002 `mars_rover`, MR-A1..A3).
//!
//! **World.** Everything physical lives in world units (1 unit ≈ [`UNIT_M`] m) with
//! isotropic axes: x to the right, y up. `render` alone maps to cells with a zoom `s`
//! (rows per unit) chosen from the canvas size, stretching x by [`ASPECT`].
//!
//! **Terrain (MR-A2).** Each of the three parallax layers is a [`Ridge`]: fractal 1-D
//! value noise whose lattice values are hashes of `(seed, integer lattice point)`, so a
//! height is a pure function of world x — no chunks, no caches, no state that scrolling
//! or resizing could disturb. Smoothstep interpolation makes it C¹, and its Lipschitz
//! constant follows from the construction (see [`Ridge::lipschitz`]).
//!
//! **Suspension (MR-A1).** Per side, a rocker pivots on the body and carries the front
//! wheel; its rear end carries a bogie with the middle and rear wheels. With the pivot's
//! x fixed by the drive, the unknowns (pivot height, rocker angle, bogie angle) are
//! exactly determined by "all three wheels touch the ground"; [`solve_pose`] finds them
//! by Newton's method with the analytic terrain slope. The profile is 1-D, so the left
//! and right rockers see the same ground: the differential (body pitch = mean of the two
//! rocker angles) then gives pitch = rocker angle and zero roll, and the far-side wheels
//! are hidden exactly behind the near ones in this pure side view.
//!
//! **Story (MR-A3).** One work plan per sol, a [`Timeline`] exactly as long as the sol:
//! drive in the morning → deploy arm → drill → stow → image through the afternoon → UHF
//! relay at the evening orbiter pass → sleep through the night. Rover position, arm pose
//! and captions are pure functions of the sim time, so resizes never desync the story;
//! only the dust, the dust devils and the seeds are state, drawn from `ctx.rng`.
//!
//! **Sky.** A 90 s sol with a real-ish solar geometry at Jezero's latitude, on a warped
//! clock (an easy day, a lingering sunset, a brief night). By day the sky is butterscotch
//! on the Blackbody ramp. Martian dust forward-scatters blue light, so around the low Sun
//! a blue aureole grows, drawn in `Ramp(Ice)` and dithered into the tan sky; sky and
//! aureole fade together through a shortened dusty twilight, and only then do the stars
//! come out (MR-A4). Phobos, orbiting faster than Mars spins, rises in the west. The
//! rover dims with the daylight (MR-A9).

use std::f64::consts::{FRAC_PI_2, PI};

use rand::Rng;
use sciart_core::geom::{self, ASPECT};
use sciart_core::ink::{self, ramp};
use sciart_core::particles::{gaussian, plot};
use sciart_core::prelude::*;
use sciart_core::text::StackText;
use sciart_core::timeline::ease;
use sciart_core::{draw, rng};

pub static META: Meta = Meta {
    name: "mars_rover",
    category: Category::Voyages,
    description: "A rocker-bogie rover crossing Martian dunes, drilling and phoning home",
    title: "Mars rover · rocker–bogie drive",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(MarsRover::default())
}

// ---------------------------------------------------------------------------------------
// Terrain: fractal value noise (MR-A2).
// ---------------------------------------------------------------------------------------

/// Metres per world unit: a 0.5-unit wheel radius is Perseverance's 0.26 m wheel.
const UNIT_M: f64 = 0.52;
/// Octaves of value noise per layer (spec: ≥ 3).
const OCTAVES: usize = 4;
/// Amplitude ratio between successive octaves (frequency doubles each octave).
const PERSISTENCE: f64 = 0.5;
/// `max |S'(f)|` of the smoothstep `S = 3f² − 2f³`, reached at `f = ½`.
#[cfg(test)]
const SMOOTHSTEP_SLOPE_MAX: f64 = 1.5;

const _: () = assert!(OCTAVES >= 3, "SPEC-002 mars_rover: at least 3 octaves");

/// Value in `[0, 1)` at integer lattice point `i`.
#[inline]
fn lattice(seed: u64, i: i64) -> f64 {
    rng::hash01(seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// 1-D value noise and its derivative: lattice values joined by smoothstep.
///
/// `n(x) = v_i + (v_{i+1} − v_i)·S(x − i)`, so `n ∈ [0, 1)` and
/// `|n'| = |v_{i+1} − v_i|·S' < 1 · 1.5`. `S'(0) = S'(1) = 0`, so the derivative is
/// continuous across lattice points too (C¹).
#[inline]
fn value_noise(seed: u64, x: f64) -> (f64, f64) {
    let i = x.floor();
    let f = x - i;
    let i = i as i64;
    let (a, b) = (lattice(seed, i), lattice(seed, i.wrapping_add(1)));
    let s = f * f * (3.0 - 2.0 * f);
    let ds = 6.0 * f * (1.0 - f);
    (a + (b - a) * s, (b - a) * ds)
}

/// One terrain layer: `H(x) = A · Σₖ gᵏ nₖ(2ᵏ x / λ) / Σₖ gᵏ`, `H ∈ [0, A)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ridge {
    seed: u64,
    /// Height range `A` (world units).
    amp: f64,
    /// Lattice spacing `λ` of the first octave (world units).
    wavelength: f64,
}

impl Ridge {
    pub(crate) fn new(seed: u64, amp: f64, wavelength: f64) -> Self {
        Self {
            seed,
            amp,
            wavelength,
        }
    }

    fn norm() -> f64 {
        (0..OCTAVES).map(|k| PERSISTENCE.powi(k as i32)).sum()
    }

    /// Height and slope `(H(x), H'(x))` — a pure function of `x` (MR-A2).
    ///
    /// Non-finite `x` maps to `x = 0` so callers never see NaN.
    pub(crate) fn eval(&self, x: f64) -> (f64, f64) {
        let x = if x.is_finite() { x } else { 0.0 };
        let (mut h, mut dh) = (0.0, 0.0);
        let (mut weight, mut freq) = (1.0, 1.0 / self.wavelength);
        for k in 0..OCTAVES as u64 {
            let seed = SplitMix64::mix(
                self.seed
                    .wrapping_add(k.wrapping_mul(0x632b_e59b_d9b4_e019)),
            );
            let (n, dn) = value_noise(seed, x * freq);
            h += weight * n;
            dh += weight * freq * dn;
            weight *= PERSISTENCE;
            freq *= 2.0;
        }
        let k = self.amp / Self::norm();
        (k * h, k * dh)
    }

    pub(crate) fn height(&self, x: f64) -> f64 {
        self.eval(x).0
    }

    /// Lipschitz constant of `H`, derived from the construction (MR-A2):
    ///
    /// `|H'| ≤ A/Σgᵏ · Σ gᵏ · (2ᵏ/λ) · max|Δv| · max S' < A · 1.5/λ · Σ(2g)ᵏ / Σgᵏ`,
    /// with `max|Δv| < 1` because lattice values lie in `[0, 1)`. So two columns
    /// `δ = 1/(s·ASPECT)` units apart differ by less than `s·L·δ = L/ASPECT` rows at
    /// any zoom `s`.
    #[cfg(test)]
    pub(crate) fn lipschitz(&self) -> f64 {
        let sum2g: f64 = (0..OCTAVES)
            .map(|k| (2.0 * PERSISTENCE).powi(k as i32))
            .sum();
        self.amp * SMOOTHSTEP_SLOPE_MAX / self.wavelength * sum2g / Self::norm()
    }
}

/// Near terrain: rows of relief the rover drives on. `L/ASPECT ≈ 0.34` rows per column.
const NEAR_AMP: f64 = 3.0;
const NEAR_WAVE: f64 = 14.0;
const MID_AMP: f64 = 3.8;
const MID_WAVE: f64 = 20.0;
const FAR_AMP: f64 = 8.5;
const FAR_WAVE: f64 = 34.0;
/// Parallax factors: how fast each layer scrolls relative to the near ground.
const PARALLAX_MID: f64 = 0.4;
const PARALLAX_FAR: f64 = 0.12;

// ---------------------------------------------------------------------------------------
// Rocker–bogie kinematics (MR-A1).
// ---------------------------------------------------------------------------------------

const WHEEL_R: f64 = 0.5;
/// Rocker: front-wheel hub at `(AF, −BF)` from the body pivot, bogie joint at `(−AB, −BB)`.
const AF: f64 = 3.0;
const BF: f64 = 1.2;
const AB: f64 = 1.4;
const BB: f64 = 0.5;
/// Bogie: middle and rear hubs at `(±BOGIE_C, −BOGIE_D)` from the bogie joint.
/// `BF = BB + BOGIE_D` so all hubs are level on flat ground.
const BOGIE_C: f64 = 1.2;
const BOGIE_D: f64 = 0.7;
const NEWTON_ITERS: usize = 16;

// Local 2-D helpers on tuples, kept on purpose: every world and screen point in this
// scene is a `(x, y)` tuple that feeds `plot(x, y)` and destructuring directly, while
// `geom::v3` (CORE-R60) is array-based. Three one-liners do not justify a core `v2`
// that this scene would have to convert to and from at every call.
type P2 = (f64, f64);

#[inline]
fn add(a: P2, b: P2) -> P2 {
    (a.0 + b.0, a.1 + b.1)
}

#[inline]
fn sub(a: P2, b: P2) -> P2 {
    (a.0 - b.0, a.1 - b.1)
}

/// Rotate `v` counter-clockwise by `a`.
#[inline]
fn rot(a: f64, v: P2) -> P2 {
    let (s, c) = a.sin_cos_det();
    (v.0 * c - v.1 * s, v.0 * s + v.1 * c)
}

/// The suspension state of one side: body pivot, rocker angle `phi` and bogie angle
/// `theta` (both absolute, CCW positive). Body pitch equals `phi` (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Pose {
    pub pivot: P2,
    pub phi: f64,
    pub theta: f64,
}

/// World positions of the pivots and wheel hubs of one side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Joints {
    pub pivot: P2,
    pub front: P2,
    pub bogie: P2,
    pub middle: P2,
    pub rear: P2,
}

impl Pose {
    /// Forward kinematics: rigid links, so link lengths hold by construction.
    pub(crate) fn joints(&self) -> Joints {
        let p = self.pivot;
        let bogie = add(p, rot(self.phi, (-AB, -BB)));
        Joints {
            pivot: p,
            front: add(p, rot(self.phi, (AF, -BF))),
            bogie,
            middle: add(bogie, rot(self.theta, (BOGIE_C, -BOGIE_D))),
            rear: add(bogie, rot(self.theta, (-BOGIE_C, -BOGIE_D))),
        }
    }

    /// The near side's hubs (front, middle, rear). On a 1-D profile the far side's
    /// coincide with them, hidden exactly behind.
    pub(crate) fn wheel_centers(&self) -> [P2; 3] {
        let j = self.joints();
        [j.front, j.middle, j.rear]
    }

    /// Point `(u, v)` of the body frame (pivot origin, x along the deck) in world units.
    fn body(&self, u: f64, v: f64) -> P2 {
        add(self.pivot, rot(self.phi, (u, v)))
    }
}

/// Solve the suspension for body pivot at world `px` on `terrain(x) = (H, H')` (MR-A1).
///
/// Residuals `rⱼ = yⱼ − R − H(xⱼ)` for the three hubs, unknowns `(y_p, φ, θ)`. The
/// Jacobian columns follow from `d/dα rot(α)v = rot90(rot(α)v)`; on flat ground its
/// determinant is `2·C·(AF + AB) > 0`, so Newton is well posed. A cold start from
/// chord slopes makes the pose a pure function of `px`.
pub(crate) fn solve_pose(terrain: impl Fn(f64) -> (f64, f64), px: f64) -> Pose {
    let px = if px.is_finite() { px } else { 0.0 };
    let h = |x: f64| terrain(x).0;
    let bx = px - AB;
    let mut pose = Pose {
        pivot: (px, h(px + AF) + WHEEL_R + BF),
        phi: ((h(px + AF) - h(bx)) / (AF + AB)).atan(),
        theta: ((h(bx + BOGIE_C) - h(bx - BOGIE_C)) / (2.0 * BOGIE_C)).atan(),
    };
    for _ in 0..NEWTON_ITERS {
        let j = pose.joints();
        let ((hf, sf), (hm, sm), (hr, sr)) =
            (terrain(j.front.0), terrain(j.middle.0), terrain(j.rear.0));
        let r = [
            j.front.1 - WHEEL_R - hf,
            j.middle.1 - WHEEL_R - hm,
            j.rear.1 - WHEEL_R - hr,
        ];
        if r.iter().all(|v| v.abs() < 1e-12) {
            break;
        }
        let (fp, bp) = (sub(j.front, j.pivot), sub(j.bogie, j.pivot));
        let (mb, rb) = (sub(j.middle, j.bogie), sub(j.rear, j.bogie));
        let jac = [
            [1.0, fp.0 + sf * fp.1, 0.0],
            [1.0, bp.0 + sm * bp.1, mb.0 + sm * mb.1],
            [1.0, bp.0 + sr * bp.1, rb.0 + sr * rb.1],
        ];
        let Some(d) = solve3(jac, [-r[0], -r[1], -r[2]]) else {
            break;
        };
        pose.pivot.1 += d[0];
        pose.phi += d[1].clamp(-0.3, 0.3);
        pose.theta += d[2].clamp(-0.3, 0.3);
    }
    let ok = pose.pivot.1.is_finite() && pose.phi.is_finite() && pose.theta.is_finite();
    if ok {
        pose
    } else {
        Pose {
            pivot: (px, h(px) + WHEEL_R + BF),
            phi: 0.0,
            theta: 0.0,
        }
    }
}

/// Cramer's rule for a 3×3 system; `None` when (near-)singular or non-finite.
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(a);
    if !d.is_finite() || d.abs() < 1e-9 {
        return None;
    }
    let mut x = [0.0; 3];
    for (col, xi) in x.iter_mut().enumerate() {
        let mut m = a;
        for row in 0..3 {
            m[row][col] = b[row];
        }
        *xi = det(m) / d;
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

// ---------------------------------------------------------------------------------------
// The sol's work plan (MR-A3) and the drive profile.
// ---------------------------------------------------------------------------------------

/// One plan per sol, starting at 09:00 with the sol clock (`SOL_KNOTS`): the drive and
/// the coring by day, imaging through the afternoon, the UHF relay at the evening
/// orbiter pass with the Sun a few degrees up, then sleep through the night, as a real
/// rover works (it does not drive in the dark).
/// The rover's shadow (MR-A12): half its footprint (units), the longest it gets, how
/// deep into the ground surface it darkens (pixel rows / 2) and by how much at full
/// daylight.
const SHADOW_HALF: f64 = 2.6;
const SHADOW_MAX: f64 = 14.0;
const SHADOW_ROWS: f64 = 3.0;
const SHADOW_DEPTH: f64 = 0.5;
/// The rover's screen column (share of the width) at the start and end of a drive.
const ROVER_COLS: (f64, f64) = (0.3, 0.62);
const DRIVE: f64 = 18.0;
const DEPLOY: f64 = 3.0;
const DRILL: f64 = 4.0;
const STOW: f64 = 2.0;
const IMAGE: f64 = 13.0;
const TRANSMIT: f64 = 6.0;
const SLEEP: f64 = 44.0;
const PLAN: [(&str, f64); 7] = [
    ("drive", DRIVE),
    ("deploy arm", DEPLOY),
    ("drill", DRILL),
    ("stow", STOW),
    ("image", IMAGE),
    ("transmit", TRANSMIT),
    ("sleep", SLEEP),
];
const CYCLE: f64 = DRIVE + DEPLOY + DRILL + STOW + IMAGE + TRANSMIT + SLEEP;
const PH_DRIVE: usize = 0;
const PH_DEPLOY: usize = 1;
const PH_DRILL: usize = 2;
const PH_STOW: usize = 3;
const PH_IMAGE: usize = 4;
const PH_TRANSMIT: usize = 5;
/// Perseverance carries 43 tubes: 38 for samples and 5 witness tubes.
const SAMPLE_TUBES: i64 = 38;

/// The number of the sample cored in plan cycle `cycle`: one per sol, wrapping within
/// the rover's 38 sample tubes (MR-A3).
fn sample_number(cycle: i64) -> i64 {
    cycle.rem_euclid(SAMPLE_TUBES) + 1
}

/// Cruise speed (units/s ≈ 0.8 m/s): Perseverance's 4 cm/s, time-lapsed ×20 for the screen.
const CRUISE: f64 = 1.6;
/// Time to reach cruise speed (and to stop) — a trapezoidal speed profile.
const RAMP: f64 = 1.5;
/// Distance covered in one drive phase.
const DRIVE_DIST: f64 = CRUISE * (DRIVE - RAMP);

/// The rover's shadow on the ground in world x, `(lo, hi)`, for the rover at `rover_x`,
/// the Sun on the `toward` side (+1 right) at elevation `e` (rad): its footprint
/// (`SHADOW_HALF` either side) stretched down-Sun by `BODY_V.1 / tan e`, at most
/// `SHADOW_MAX` (MR-A12). `None` once the Sun is down.
pub(crate) fn shadow_span(rover_x: f64, toward: f64, e: f64) -> Option<(f64, f64)> {
    if e.is_nan() || e <= 0.0 {
        return None;
    }
    let reach = (BODY_V.1 / e.tan()).min(SHADOW_MAX);
    let tip = rover_x - toward * reach;
    Some((
        (rover_x - SHADOW_HALF).min(tip - SHADOW_HALF),
        (rover_x + SHADOW_HALF).max(tip + SHADOW_HALF),
    ))
}

pub(crate) fn sol_timeline() -> Timeline {
    Timeline::new(&PLAN)
}

/// Distance (units) and speed (units/s) `local` seconds into a drive phase.
fn drive_profile(local: f64) -> (f64, f64) {
    let u = local.clamp(0.0, DRIVE);
    let a = CRUISE / RAMP;
    if u < RAMP {
        (0.5 * a * u * u, a * u)
    } else if u < DRIVE - RAMP {
        (CRUISE * (u - 0.5 * RAMP), CRUISE)
    } else {
        let r = DRIVE - u;
        (DRIVE_DIST - 0.5 * a * r * r, a * r)
    }
}

/// Total distance driven by sim time `t`: a pure, continuous, non-decreasing function
/// that only advances during drive phases.
pub(crate) fn odometer(t: f64) -> f64 {
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
    let cycles = (t / CYCLE).floor();
    let local = t - cycles * CYCLE;
    cycles * DRIVE_DIST + drive_profile(local).0
}

fn speed_at(t: f64) -> f64 {
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
    let local = t - (t / CYCLE).floor() * CYCLE;
    if local < DRIVE {
        drive_profile(local).1
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------
// Robotic arm.
// ---------------------------------------------------------------------------------------

// Body-frame geometry (pivot at the origin, u along the deck, v up; world units).
/// Deck box height, mast top, camera-head box, the high-gain antenna's dish centre and
/// the UHF antenna's tip on the rear deck. Orbiter relay goes over the UHF antenna,
/// which is not steered; the HGA is the X-band direct-to-Earth dish and stays parked.
const BODY_V: (f64, f64) = (0.3, 1.8);
const MAST_TOP: f64 = 3.8;
const HEAD_U: (f64, f64) = (1.15, 2.35);
const HEAD_V: (f64, f64) = (3.8, 4.4);
const HGA: P2 = (-0.9, 2.4);
const UHF: P2 = (-1.8, BODY_V.1 + 0.7);
/// Arm shoulder, link lengths, how far ahead of the pivot the drill touches down, and
/// the turret's height above the ground while drilling.
const SHOULDER: P2 = (2.2, 0.9);
const ARM_L1: f64 = 1.9;
const ARM_L2: f64 = 1.7;
const ARM_REACH_X: f64 = 4.5;
const TURRET_H: f64 = 0.35;
/// Stowed link angles relative to the deck: upper arm forward-down, forearm folded back.
const STOW_A1: f64 = -0.55;
const STOW_A2: f64 = PI + 0.12;

/// Shoulder, elbow and turret positions for deployment fraction `k ∈ [0, 1]`.
fn arm(pose: &Pose, near: &Ridge, k: f64) -> (P2, P2, P2) {
    let s = pose.body(SHOULDER.0, SHOULDER.1);
    let tx = pose.pivot.0 + ARM_REACH_X;
    let target = (tx, near.height(tx) + TURRET_H);
    let d = sub(target, s);
    let dist =
        d.0.hypot(d.1)
            .clamp(ARM_L1 - ARM_L2 + 0.05, ARM_L1 + ARM_L2 - 1e-6);
    // Law of cosines; adding the angle puts the elbow above the shoulder–target line.
    let cos_a = ((ARM_L1 * ARM_L1 + dist * dist - ARM_L2 * ARM_L2) / (2.0 * ARM_L1 * dist))
        .clamp(-1.0, 1.0);
    let ik1 = d.1.atan2(d.0) + cos_a.acos();
    let elbow_ik = add(s, (ARM_L1 * ik1.cos_det(), ARM_L1 * ik1.sin_det()));
    let t2 = sub(target, elbow_ik);
    let ik2 = t2.1.atan2(t2.0);
    let a1 = lerp_angle(STOW_A1 + pose.phi, ik1, k);
    let a2 = lerp_angle(STOW_A2 + pose.phi, ik2, k);
    let e = add(s, (ARM_L1 * a1.cos_det(), ARM_L1 * a1.sin_det()));
    let t = add(e, (ARM_L2 * a2.cos_det(), ARM_L2 * a2.sin_det()));
    (s, e, t)
}

/// Interpolate angles along the shorter arc.
fn lerp_angle(a: f64, b: f64, k: f64) -> f64 {
    a + geom::wrap_angle(b - a) * k
}

/// Deployment fraction of the arm at a point of the sol.
fn arm_extension(beat: &Beat) -> f64 {
    match beat.index {
        PH_DEPLOY => ease::in_out_cubic(beat.progress),
        PH_DRILL => 1.0,
        PH_STOW => 1.0 - ease::in_out_cubic(beat.progress),
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------------------
// Sky: solar geometry over a 90 s sol.
// ---------------------------------------------------------------------------------------

const SOL: f64 = 90.0;
const _: () = assert!(CYCLE == SOL, "SPEC-002 mars_rover: one work plan per sol");
/// Jezero crater latitude and a southern-summer solar declination.
const LATITUDE: f64 = 18.4 * PI / 180.0;
const DECLINATION: f64 = -12.0 * PI / 180.0;
/// Hour angle (deg) vs sol time (s): day at an easy pace, a lingering sunset, a quick
/// night. Starts at 09:00 local true solar time; 315° ≡ −45° closes the loop.
const SOL_KNOTS: [(f64, f64); 5] = [
    (0.0, -45.0),
    (40.0, 78.0),
    (62.0, 100.0),
    (80.0, 255.0),
    (90.0, 315.0),
];
/// Phobos' synodic period (11.12 h) and time above the horizon (4.25 h) of a 24.66 h sol.
const PHOBOS_PERIOD: f64 = SOL * 11.12 / 24.66;
const PHOBOS_UP: f64 = SOL * 4.25 / 24.66;

/// Hour angle (rad) at sim time `t`.
fn hour_angle(t: f64) -> f64 {
    let u = geom::wrap(if t.is_finite() { t } else { 0.0 }, SOL);
    let mut h = SOL_KNOTS[0].1;
    for w in SOL_KNOTS.windows(2) {
        let ((t0, h0), (t1, h1)) = (w[0], w[1]);
        if u >= t0 && u < t1 {
            h = ease::lerp(h0, h1, (u - t0) / (t1 - t0));
        }
    }
    h.to_radians()
}

/// Solar elevation (rad) for hour angle `h`.
fn sun_elevation(h: f64) -> f64 {
    (LATITUDE.sin_det() * DECLINATION.sin_det()
        + LATITUDE.cos_det() * DECLINATION.cos_det() * h.cos_det())
    .clamp(-1.0, 1.0)
    .asin()
}

fn max_elevation() -> f64 {
    sun_elevation(0.0)
}

#[inline]
fn smooth(edge0: f64, edge1: f64, x: f64) -> f64 {
    ease::smoothstep((x - edge0) / (edge1 - edge0))
}

/// The aureole's Gaussian radius, as a share of the Sun's height range (the horizon row).
const GLOW_R: f64 = 0.42;
/// The aureole (MR-A4): a pixel turns `Ramp(Ice)` where its glow `g` passes the edge,
/// dithered in over the band above it.
const AUREOLE_EDGE: f64 = 0.22;
const AUREOLE_BAND: f64 = 0.55;
/// The lift toward the Sun, and the cap, kept under the Sun's top-of-ramp white (MR-A7).
const AUREOLE_LIFT: f64 = 60.0;
const AUREOLE_MAX: f64 = 232.0;

/// The `Ramp(Ice)` level as light as Blackbody level `tan` over the sky's tan stretch
/// (levels 50–125: within 15 % in luminance), so the dithered edge of the aureole
/// changes hue, not brightness.
fn ice_matching(tan: f64) -> f64 {
    // (Blackbody level, Ice level) pairs of equal astro luminance, read off the themes'
    // tables; linear in between and proportional below the first.
    const KNOTS: [(f64, f64); 6] = [
        (48.0, 78.0),
        (64.0, 102.0),
        (80.0, 128.0),
        (96.0, 161.0),
        (112.0, 197.0),
        (128.0, 237.0),
    ];
    if tan <= KNOTS[0].0 {
        return tan * KNOTS[0].1 / KNOTS[0].0;
    }
    for w in KNOTS.windows(2) {
        let ((b0, i0), (b1, i1)) = (w[0], w[1]);
        if tan <= b1 {
            return ease::lerp(i0, i1, (tan - b0) / (b1 - b0));
        }
    }
    KNOTS[5].1
}

/// A 4×4 Bayer threshold in (0, 1) for pixel `(x, y)`: an ordered dither reads as a
/// blend at half-block resolution, where random noise would sparkle.
fn bayer4(x: i32, y: i32) -> f64 {
    const M: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let i = (y.rem_euclid(4) * 4 + x.rem_euclid(4)) as usize;
    (f64::from(M[i]) + 0.5) / 16.0
}

/// Blue forward-scattered glow around a low sun: on below ~25° elevation, fading out
/// with the twilight by −12°.
fn glow_strength(e_deg: f64) -> f64 {
    smooth(25.0, 4.0, e_deg) * smooth(-12.0, -2.0, e_deg)
}

/// The sky's brightness: full above 8°, fading through a long dusty twilight to −14°.
fn daylight(e_deg: f64) -> f64 {
    smooth(-14.0, 8.0, e_deg)
}

/// How dark it is for the stars: they come out once the aureole has gone (MR-A4).
fn night(e_deg: f64) -> f64 {
    smooth(-11.0, -18.0, e_deg)
}

// ---------------------------------------------------------------------------------------
// Dust.
// ---------------------------------------------------------------------------------------

const DUST_MAX: usize = 360;
/// Fine dust: Stokes drag dominates, settling at `g_eff/DRAG`.
const DUST_G: f64 = 2.2;
const DUST_DRAG: f64 = 1.6;
const WIND: f64 = 0.5;
/// Particles per second per wheel at cruise speed, and from the drill.
const WHEEL_RATE: f64 = 3.5;
const DRILL_RATE: f64 = 34.0;
const H_MAX: f64 = 1.0 / 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Dust {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
    life: f64,
}

/// Exact update over `h` for linear drag towards `(WIND, −DUST_G/DUST_DRAG)`.
fn drift(p: &mut Dust, h: f64) {
    let e = (-DUST_DRAG * h).exp();
    let decay = (1.0 - e) / DUST_DRAG;
    let (tx, ty) = (WIND, -DUST_G / DUST_DRAG);
    p.x += tx * h + (p.vx - tx) * decay;
    p.y += ty * h + (p.vy - ty) * decay;
    p.vx = tx + (p.vx - tx) * e;
    p.vy = ty + (p.vy - ty) * e;
    p.age += h;
}

/// A dust devil on the mid-distance plain (mid-layer world coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Devil {
    x: f64,
    vx: f64,
    age: f64,
    life: f64,
    height: f64,
    phase: f64,
}

const DEVILS: usize = 2;

// ---------------------------------------------------------------------------------------
// Glyphs.
// ---------------------------------------------------------------------------------------

/// Rover deck and camera head.
const HULL: Glyph = Glyph::new('█', '=');
const TYRE: Glyph = Glyph::new('█', 'o');
/// A one-cell solid object in the sky: Phobos, the orbiter, a data packet.
const PIXEL: Glyph = Glyph::new('█', 'o');
const WING: Glyph = Glyph::new('█', '-');
const JOINT: Glyph = Glyph::new('•', 'o');
const LIGHT: Glyph = Glyph::new('░', '.');
const DOT: Glyph = Glyph::new('·', '.');
const WHEEL: [Glyph; 4] = [
    Glyph::new('◐', 'O'),
    Glyph::new('◓', 'O'),
    Glyph::new('◑', 'O'),
    Glyph::new('◒', 'O'),
];

/// Link glyph for a screen segment, by its visual angle (rows are ~2× columns).
///
/// One glyph per strut on purpose, not `geom::stroked_line` + `draw::step_glyph`
/// (CORE-R62): those pick a glyph per Bresenham step, which suits long lines, but on a
/// rover strut only 2–6 cells long a shallow link would read `─╲─╲`, a zigzag rather
/// than one rigid member.
fn link_glyph(dx: f64, dy: f64) -> Glyph {
    let a = (dy * ASPECT).atan2(dx).abs();
    let a = if a > FRAC_PI_2 { PI - a } else { a };
    let up_right = (dx >= 0.0) == (dy <= 0.0);
    match a {
        a if a < PI / 8.0 => Glyph::new('─', '-'),
        a if a > 3.0 * PI / 8.0 => Glyph::new('│', '|'),
        _ if up_right => Glyph::new('╱', '/'),
        _ => Glyph::new('╲', '\\'),
    }
}

// ---------------------------------------------------------------------------------------
// Screen layout.
// ---------------------------------------------------------------------------------------

const SCALE_MIN: f64 = 0.35;
const SCALE_MAX: f64 = 2.5;
/// Below this zoom the rover is drawn as a tiny glyph sprite.
const TINY_SCALE: f64 = 0.7;
/// From this zoom up the rover's struts are drawn as solid blocks.
const SOLID_SCALE: f64 = 1.2;

/// Size-dependent mapping from world to cells (set in `resize`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Layout {
    w: f64,
    h: f64,
    /// Rows per world unit.
    s: f64,
    /// Screen rows of `H = 0` for each layer.
    near_base: f64,
    mid_base: f64,
    far_base: f64,
    /// Row where the sun sets.
    horizon: f64,
}

impl Layout {
    fn new(size: Size) -> Self {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let s = (h / 20.0).min(w / 36.0).clamp(SCALE_MIN, SCALE_MAX);
        let near_base = h - 0.6 - 0.9 * s;
        let far_base = near_base - 3.4 * s;
        Self {
            w,
            h,
            s,
            near_base,
            mid_base: near_base - 2.1 * s,
            far_base,
            horizon: far_base - 0.45 * FAR_AMP * s,
        }
    }

    /// World units per column.
    fn dx(&self) -> f64 {
        1.0 / (self.s * ASPECT)
    }
}

/// Where the near-layer camera is at time `t`: world x of column 0, with the rover at
/// [`rover_col`].
fn camera(l: &Layout, rover_x: f64, t: f64) -> f64 {
    rover_x - rover_col(l.w, t) * l.dx()
}

/// The rover's screen column at time `t` in a frame `w` cells wide (MR-A12): it drives
/// from `ROVER_COLS.0` to `ROVER_COLS.1` of the width during the drive (the camera
/// following at a lag, so the terrain scrolls too), holds there through the work plan,
/// and the camera eases back to `ROVER_COLS.0` over the night.
pub(crate) fn rover_col(w: f64, t: f64) -> f64 {
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
    let local = t - (t / CYCLE).floor() * CYCLE;
    let driven = drive_profile(local).0 / DRIVE_DIST;
    let back = ease::smoothstep((local - (CYCLE - SLEEP)) / SLEEP);
    w * (ROVER_COLS.0 + (ROVER_COLS.1 - ROVER_COLS.0) * (driven - back))
}

/// World (near layer) → fractional cell coordinates.
#[derive(Debug, Clone, Copy)]
struct View {
    cam: f64,
    k: f64,
    s: f64,
    base: f64,
}

impl View {
    fn at(&self, p: P2) -> P2 {
        ((p.0 - self.cam) * self.k, self.base - self.s * p.1)
    }
}

// ---------------------------------------------------------------------------------------
// The scene.
// ---------------------------------------------------------------------------------------

/// Depth layers of the landscape, back to front (MR-A5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Layer {
    Sky,
    Far,
    Mid,
    Near,
}

/// The layer covering pixel row `py` of a column whose far, mid and near surfaces sit at
/// pixel rows `tops`: the nearest layer whose surface lies above the pixel's centre, so a
/// column always reads sky, then far, mid and near ground, going down (MR-A5).
pub(crate) fn layer_at(tops: [f64; 3], py: i32) -> Layer {
    let y = f64::from(py) + 0.5;
    if y >= tops[2] {
        Layer::Near
    } else if y >= tops[1] {
        Layer::Mid
    } else if y >= tops[0] {
        Layer::Far
    } else {
        Layer::Sky
    }
}

/// This frame's light: the Sun's cell, its blue glow, daylight, darkness, whether the
/// disc is up, and the glow's radius (rows).
#[derive(Debug, Clone, Copy)]
struct SkyLight {
    sun: P2,
    glow: f64,
    day: f64,
    dark: f64,
    sun_up: bool,
    r_glow: f64,
}

/// Seeded, size-independent state drawn once from `ctx.rng` (MR-A2).
#[derive(Debug, Clone, Copy, PartialEq)]
struct World {
    near: Ridge,
    mid: Ridge,
    far: Ridge,
    /// World x of the pivot at t = 0.
    x0: f64,
    sol0: u32,
    phobos_phase: f64,
    salt: u64,
}

#[derive(Debug)]
pub struct MarsRover {
    t: f64,
    /// The sol's work plan, built once so that `render` never allocates.
    plan: Timeline,
    world: Option<World>,
    layout: Layout,
    dust: ParticlePool<Dust>,
    dust_acc: f64,
    devils: [Option<Devil>; DEVILS],
    devil_timer: f64,
    /// Sky, Sun, Phobos and ground as half blocks, rasterised in `update`.
    px: Pixels,
    /// Night stars as braille dots over the black sky, rasterised in `update`.
    dots: Dots,
    /// Stars as dot coordinates and base levels, rebuilt on resize.
    stars: Vec<(i32, i32, u8)>,
    /// Pixel rows of the far, mid and near surfaces per column, this frame.
    tops: Vec<[f64; 3]>,
    /// Daylight in the sky, in [0, 1], and on the rover and the dust, in [0.35, 1].
    day: f64,
    light: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl Default for MarsRover {
    fn default() -> Self {
        Self {
            t: 0.0,
            plan: sol_timeline(),
            world: None,
            layout: Layout::default(),
            dust: ParticlePool::with_capacity(0),
            dust_acc: 0.0,
            devils: [None; DEVILS],
            devil_timer: 0.0,
            px: Pixels::default(),
            dots: Dots::default(),
            stars: Vec::new(),
            tops: Vec::new(),
            day: 1.0,
            light: 1.0,
            dirty: false,
        }
    }
}

impl MarsRover {
    fn seed(ctx: &mut Ctx) -> World {
        let seed: u64 = ctx.rng.random();
        World {
            near: Ridge::new(SplitMix64::mix(seed ^ 1), NEAR_AMP, NEAR_WAVE),
            mid: Ridge::new(SplitMix64::mix(seed ^ 2), MID_AMP, MID_WAVE),
            far: Ridge::new(SplitMix64::mix(seed ^ 3), FAR_AMP, FAR_WAVE),
            x0: ctx.rng.random_range(0.0..10_000.0),
            sol0: ctx.rng.random_range(120..1400),
            phobos_phase: ctx.rng.random_range(0.0..PHOBOS_PERIOD),
            salt: SplitMix64::mix(seed ^ 4),
        }
    }

    fn rover_x(&self, w: &World) -> f64 {
        w.x0 + odometer(self.t)
    }

    /// One physics substep: emit, move and settle dust; drift dust devils.
    fn substep(&mut self, w: &World, h: f64, ctx: &mut Ctx) {
        self.t += h;
        let beat = self.plan.at(self.t);
        let pose = solve_pose(|x| w.near.eval(x), self.rover_x(w));
        let (rate, drill) = match beat.index {
            PH_DRIVE => (6.0 * WHEEL_RATE * speed_at(self.t) / CRUISE, false),
            PH_DRILL => (DRILL_RATE, true),
            _ => (0.0, false),
        };
        self.dust_acc += rate * h;
        let v = speed_at(self.t);
        let hubs = pose.wheel_centers();
        let drill_at = arm(&pose, &w.near, 1.0).2;
        while self.dust_acc >= 1.0 {
            self.dust_acc -= 1.0;
            let p = if drill {
                Dust {
                    x: drill_at.0 + gaussian(&mut ctx.rng, 0.0, 0.15),
                    y: drill_at.1 - TURRET_H,
                    vx: gaussian(&mut ctx.rng, 0.0, 1.1),
                    vy: ctx.rng.random_range(0.8..3.2),
                    age: 0.0,
                    life: ctx.rng.random_range(1.2..3.0),
                }
            } else {
                let hub = hubs[ctx.rng.random_range(0..hubs.len())];
                Dust {
                    x: hub.0 - 0.3 * WHEEL_R,
                    y: w.near.height(hub.0) + 0.1,
                    vx: -0.5 * v + gaussian(&mut ctx.rng, 0.0, 0.35),
                    vy: ctx.rng.random_range(0.4..1.8),
                    age: 0.0,
                    life: ctx.rng.random_range(0.8..2.2),
                }
            };
            self.dust.spawn(p);
        }
        let near = w.near;
        self.dust.retain(|p| {
            drift(p, h);
            p.age < p.life && p.y > near.height(p.x) - 0.05
        });

        let l = self.layout;
        for d in self.devils.iter_mut() {
            if let Some(dv) = d {
                dv.x += dv.vx * h;
                dv.age += h;
                if dv.age >= dv.life {
                    *d = None;
                }
            }
        }
        self.devil_timer -= h;
        if self.devil_timer <= 0.0 {
            self.devil_timer = ctx.rng.random_range(3.0..9.0);
            let cam_mid = camera(&l, self.rover_x(w), self.t) * PARALLAX_MID;
            if let Some(slot) = self.devils.iter_mut().find(|d| d.is_none()) {
                let col = ctx.rng.random_range(0.05..0.95) * l.w;
                *slot = Some(Devil {
                    x: cam_mid + col * l.dx(),
                    vx: ctx.rng.random_range(0.3..0.9) * if ctx.rng.random() { 1.0 } else { -1.0 },
                    age: 0.0,
                    life: ctx.rng.random_range(10.0..20.0),
                    height: ctx.rng.random_range(3.0..6.5),
                    phase: ctx.rng.random_range(0.0..10.0),
                });
            }
        }
    }
}

impl Generator for MarsRover {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.world.is_none() {
            self.world = Some(Self::seed(ctx));
            self.devil_timer = ctx.rng.random_range(0.5..3.0);
        }
        self.layout = Layout::new(size);
        self.dust.set_capacity(DUST_MAX.min(size.area() / 4));
        self.px.resize(size);
        self.dots.resize(size);
        self.tops.clear();
        self.tops.resize(usize::from(size.width()), [0.0; 3]);
        let dot = self.dots.dot_size();
        self.stars.clear();
        // Below the readout row (dot rows 0..4).
        self.stars.extend((0..size.area() / 30).map(|_| {
            (
                ctx.rng.random_range(0..i32::from(dot.width())),
                ctx.rng.random_range(4..i32::from(dot.height()).max(5)),
                ctx.rng.random_range(60..255u8),
            )
        }));
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        let Some(w) = self.world else {
            self.t += dt;
            return;
        };
        let n = (dt / H_MAX).ceil().max(1.0) as usize;
        let h = dt / n as f64;
        for _ in 0..n {
            self.substep(&w, h, ctx);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let Some(w) = self.world else { return };
        let l = self.layout;
        let rover_x = self.rover_x(&w);
        let cam = camera(&l, rover_x, self.t);
        let view = View {
            cam,
            k: l.s * ASPECT,
            s: l.s,
            base: l.near_base,
        };
        let beat = self.plan.at(self.t);
        self.px.blit(canvas, |x, y, _, _| {
            // ASCII: the sky stays blank but for the Sun and its aureole; ground layers
            // keep their old marks.
            let (top, bottom) = (self.px.get(x, 2 * y), self.px.get(x, 2 * y + 1));
            let ink = bottom.filter(|i| i.role != Role::Background).or(top)?;
            let tops = *self.tops.get(usize::try_from(x).ok()?)?;
            match layer_at(tops, 2 * y + 1) {
                Layer::Sky => match ink.level {
                    255 => Some(('@', ink)),
                    // The blue aureole.
                    _ if ink.role == Role::Ramp(Ramp::Ice) => Some(('.', ink)),
                    _ => None,
                },
                Layer::Far => Some((':', ink)),
                Layer::Mid => Some(('+', ink)),
                Layer::Near => Some(('#', ink)),
            }
        });
        self.dots.blit(canvas);
        self.draw_orbiter(canvas, &beat);
        self.draw_devils(canvas, &w, cam);
        self.draw_drill_marks(canvas, &w, &view, &beat);
        let pose = solve_pose(|x| w.near.eval(x), rover_x);
        self.draw_dust(canvas, &view);
        let link = self
            .orbiter(&beat)
            .map(|o| (view.at(pose.body(UHF.0, UHF.1)), o));
        if let Some((a, o)) = link {
            draw_beam(canvas, a, o);
        }
        self.draw_rover(canvas, &w, &view, &pose, &beat);
        if let Some((a, o)) = link {
            self.draw_packets(canvas, a, o);
        }
        self.draw_captions(canvas, &w, &beat, hour_angle(self.t));
        // Struts, dust and labels keep the landscape behind them (MR-A6).
        self.px.backdrop(canvas);
    }
}

// ---------------------------------------------------------------------------------------
// Rendering.
// ---------------------------------------------------------------------------------------

impl MarsRover {
    /// The Sun's cell position for hour angle `hour` and elevation `e`: across the sky
    /// with the hour, down to the horizon row at sunset.
    fn sun_cell(&self, hour: f64, e: f64) -> P2 {
        let l = self.layout;
        (
            l.w * (0.5 + 0.3 * hour.sin_det()),
            l.horizon - (l.horizon - 1.0) * e.sin_det() / max_elevation().sin_det(),
        )
    }

    /// Pixel rows of the far, mid and near surfaces over column `c`, and the near
    /// ground's slope there (one noise evaluation gives both).
    fn layer_tops(&self, w: &World, cam: f64, c: i32) -> ([f64; 3], f64) {
        let l = self.layout;
        let (dxc, fc) = (l.dx(), f64::from(c) + 0.5);
        let (near, slope) = w.near.eval(cam + fc * dxc);
        let tops = [
            2.0 * (l.far_base - l.s * w.far.height(cam * PARALLAX_FAR + fc * dxc)),
            2.0 * (l.mid_base - l.s * w.mid.height(cam * PARALLAX_MID + fc * dxc)),
            2.0 * (l.near_base - l.s * near),
        ];
        (tops, slope)
    }

    /// Paint the sky, the Sun, Phobos and the three ground layers into the half-block
    /// buffer (MR-A4, MR-A5). False colour in the scene's palette: the dusty sky, its blue
    /// sunset aureole and the far ridges on the Blackbody ramp, the rust ground in Ember.
    fn rasterise(&mut self) {
        let Some(w) = self.world else { return };
        let l = self.layout;
        let cam = camera(&l, self.rover_x(&w), self.t);
        let hour = hour_angle(self.t);
        let e = sun_elevation(hour);
        let e_deg = e.to_degrees();
        let sky = SkyLight {
            sun: self.sun_cell(hour, e),
            glow: glow_strength(e_deg),
            day: daylight(e_deg),
            dark: night(e_deg),
            sun_up: e_deg > 0.0,
            r_glow: (GLOW_R * l.horizon).max(2.0),
        };
        let light = 0.45 + 0.55 * sky.day;
        let far_light = 0.2 + 0.8 * sky.day;
        self.day = sky.day;
        self.light = 0.35 + 0.65 * sky.day;
        // Which way the near slopes must face to catch the Sun.
        let toward = if hour.sin_det() >= 0.0 { 1.0 } else { -1.0 };
        // The rover's shadow on the ground (MR-A12): its body's top falls
        // `BODY_V.1 / tan e` units down-Sun, so the shadow swings from one side at morning
        // to the other through the afternoon and stretches toward sunset.
        let shadow = shadow_span(self.rover_x(&w), toward, e);
        let shade = SHADOW_DEPTH * sky.day;
        let size = self.px.pixel_size();
        let (pw, ph) = (i32::from(size.width()), i32::from(size.height()));
        for c in 0..pw {
            let (tops, slope) = self.layer_tops(&w, cam, c);
            self.tops[c as usize] = tops;
            let xn = cam + (f64::from(c) + 0.5) * l.dx();
            let facing = (slope * toward).clamp(-1.0, 1.0);
            let wc = (xn / l.dx()).floor() as i32;
            for py in 0..ph {
                let ink = match layer_at(tops, py) {
                    Layer::Sky => self.sky_pixel(c, py, &sky),
                    // The far ridges take the dusty air's own ramp, a muted brown
                    // between the sky and the rust foreground (aerial perspective),
                    // lit by the sky, so they sink to a dim silhouette at night.
                    Layer::Far => {
                        let crest = f64::from(py) + 0.5 - tops[0] < 2.0;
                        let level = (0.25 + if crest { 0.03 } else { 0.0 }) * far_light;
                        Ink::new(Role::Blackbody, ink::level(255.0 * level))
                    }
                    Layer::Mid => {
                        let crest = f64::from(py) + 0.5 - tops[1] < 2.0;
                        ramp(Ramp::Ember, (0.32 + if crest { 0.06 } else { 0.0 }) * light)
                    }
                    Layer::Near => {
                        let depth = (f64::from(py) + 0.5 - tops[2]) / 2.0;
                        let n = rng::hash01(w.salt ^ rng::cell_key(wc, py));
                        let base = if depth < 1.0 {
                            0.62 + 0.08 * facing
                        } else if n < 0.06 {
                            0.58
                        } else {
                            0.50 - 0.03 * depth.min(3.0) + 0.04 * n
                        };
                        let shaded = shadow.is_some_and(|(lo, hi)| (lo..=hi).contains(&xn))
                            && depth < SHADOW_ROWS;
                        let base = if shaded { base * (1.0 - shade) } else { base };
                        ramp(Ramp::Ember, base * light)
                    }
                };
                self.px.set(c, py, ink);
            }
        }
        // Phobos: rises in the west (right), sets in the east (left); hidden by ridges.
        let p = geom::wrap(self.t + w.phobos_phase, PHOBOS_PERIOD);
        if p < PHOBOS_UP {
            let u = p / PHOBOS_UP;
            let x = (l.w * (0.95 - 0.9 * u)).floor() as i32;
            let y = (2.0 * (l.horizon - 0.55 * l.horizon * (PI * u).sin_det())).floor() as i32;
            if self
                .tops
                .get(x as usize)
                .is_some_and(|&t| layer_at(t, y) == Layer::Sky)
            {
                let lvl = 150.0 + 100.0 * sky.dark;
                self.px.set(x, y, Ink::new(Role::Structure, lvl as u8));
            }
        }
        // Stars, where the sky has gone black, twinkling.
        self.dots.clear();
        if sky.dark > 0.0 {
            for (i, &(x, y, level)) in self.stars.iter().enumerate() {
                // Whole-sky cells only: braille in a skyline cell would replace its half
                // block, and the backdrop would put the ridge colour behind the star.
                if self.px.cell_bg(x / Dots::CELL_W, y / Dots::CELL_H) == Ink::BLANK {
                    let tw = 0.8 + 0.2 * (self.t * 2.3 + 2.4 * i as f64).sin_det();
                    let level = f64::from(level) * sky.dark * tw;
                    self.dots.set(x, y, Ink::new(Role::Star, level as u8));
                }
            }
        }
    }

    /// One sky pixel (MR-A4). The dusty sky is on the Blackbody ramp: butterscotch (its
    /// tan stretch) by day, paler toward the dusty horizon and round the Sun, fading to
    /// black through the twilight by level. A low Sun wears a blue aureole, a Gaussian in
    /// angle, in `Ramp(Ice)`: Blackbody cannot hold it, since its blue end lies past its
    /// white point and would ring the Sun with white, and cannot dim without turning tan.
    /// Ice darkens as a blue. Its level tracks the tan sky's lightness (`ice_matching`)
    /// plus a lift toward the Sun, so luminance still falls outward, and an ordered dither
    /// blends the two hues over the aureole's edge instead of drawing a hue contour. The
    /// Sun is the white-hot top of the Ember ramp, the brightest ink of every theme.
    fn sky_pixel(&self, x: i32, py: i32, s: &SkyLight) -> Ink {
        // Night: no daylight and no glow (the Sun is down whenever `day` is 0).
        if s.day <= 0.0 && s.glow <= 0.0 {
            return Ink::BLANK;
        }
        let l = self.layout;
        let (fx, fy) = (f64::from(x) + 0.5, (f64::from(py) + 0.5) / 2.0);
        let a = (fy / l.horizon.max(1.0)).clamp(0.0, 1.0);
        let d = ((fx - s.sun.0) / ASPECT).hypot(fy - s.sun.1);
        if s.sun_up && d < 0.6 * l.s {
            return ramp(Ramp::Ember, 1.0);
        }
        let halo = (-d / (1.6 * l.s)).exp();
        let tan = 255.0 * s.day * (0.34 + 0.08 * a * a + 0.14 * halo);
        // Past 3 radii the Gaussian is under 1e-4: skip the `exp`.
        let g = if s.glow > 0.0 && d < 3.0 * s.r_glow {
            s.glow * (-(d / s.r_glow).powi(2)).exp()
        } else {
            0.0
        };
        let share = ((g - AUREOLE_EDGE) / AUREOLE_BAND).clamp(0.0, 1.0);
        if share > bayer4(x, py) {
            let level = (ice_matching(tan) + AUREOLE_LIFT * g).min(AUREOLE_MAX);
            return Ink::new(Role::Ramp(Ramp::Ice), ink::level(level));
        }
        if tan < ink::MIN_VISIBLE_LEVEL {
            Ink::BLANK
        } else {
            Ink::new(Role::Blackbody, ink::level(tan))
        }
    }

    /// The relay orbiter crossing the evening sky during the UHF pass.
    fn draw_orbiter(&self, canvas: &mut Canvas, beat: &Beat) {
        let Some((ox, oy)) = self.orbiter(beat) else {
            return;
        };
        let blink = beat.index == PH_TRANSMIT && (self.t * 6.0).sin_det() > 0.0;
        let ink = Ink::new(Role::Accent, if blink { 255 } else { 190 });
        plot(canvas, ox, oy, PIXEL, ink);
        if self.layout.s >= TINY_SCALE {
            // Solar panels.
            let panel = Ink::new(Role::Structure, 170);
            plot(canvas, ox - 1.0, oy, WING, panel);
            plot(canvas, ox + 1.0, oy, WING, panel);
        }
    }

    fn orbiter(&self, beat: &Beat) -> Option<P2> {
        if beat.index != PH_TRANSMIT {
            return None;
        }
        let u = beat.local / TRANSMIT;
        let l = self.layout;
        Some((
            l.w * (0.08 + 0.84 * u),
            (0.16 * l.horizon).max(1.0) + 0.4 * l.s * (PI * u).sin_det(),
        ))
    }

    /// A dust devil: a tapering, swirling column of dust leaning with the wind. It
    /// veils what is behind it rather than punching glyph-shaped holes in it.
    fn draw_devils(&self, canvas: &mut Canvas, w: &World, cam: f64) {
        let l = self.layout;
        let k = l.s * ASPECT;
        // Devils are raised by the day's heating: gone by night.
        let day = self.day;
        for dv in self.devils.iter().flatten() {
            let alpha = smooth(0.0, 2.0, dv.age) * smooth(dv.life, dv.life - 2.5, dv.age);
            let base_col = (dv.x - cam * PARALLAX_MID) * k;
            let base_row = l.mid_base - l.s * w.mid.height(dv.x);
            let rows = (dv.height * l.s).round().max(1.0) as i32;
            for j in 0..rows {
                let f = f64::from(j) / f64::from(rows);
                let y = base_row - f64::from(j) - 0.5;
                let lean = 0.8 * f * f * f64::from(rows);
                let sway = (2.0 * f - 2.5 * self.t + dv.phase).sin_det() * (0.3 + 0.8 * f);
                let cx = base_col + lean + sway;
                let half = 0.5 + 1.8 * f * l.s;
                let a = alpha * (1.0 - 0.55 * f);
                let n = (2.0 * half).ceil() as i32;
                for i in -n..=n {
                    let q = f64::from(i) / (2.0 * half);
                    if q.abs() > 1.0 {
                        continue;
                    }
                    // Swirl: dust bands rotating around the column's axis.
                    let band =
                        ((q.asin() + 4.0 * self.t - 1.7 * f64::from(j)).sin_det() + 1.0) * 0.5;
                    let v = a * (0.35 + 0.65 * band) * (1.0 - q * q) * day;
                    if v < 0.12 {
                        continue;
                    }
                    let fallback = (LIGHT, Ink::new(Role::Hot, (50.0 + 110.0 * v) as u8));
                    veil(
                        canvas,
                        cx + f64::from(i) * 0.5,
                        y,
                        (70.0 * v) as u8,
                        fallback,
                    );
                }
            }
        }
    }

    /// Cored-sample holes left behind at every previous drill site.
    fn draw_drill_marks(&self, canvas: &mut Canvas, w: &World, view: &View, beat: &Beat) {
        let cycle = (self.t / CYCLE).floor();
        let done_now = beat.index >= PH_STOW;
        let ink = Ink::new(Role::Dim, 60);
        for back in 0..8 {
            let c = cycle - f64::from(back) - if done_now { 0.0 } else { 1.0 };
            if c < 0.0 {
                break;
            }
            let x = w.x0 + (c + 1.0) * DRIVE_DIST + ARM_REACH_X;
            let (col, row) = view.at((x, w.near.height(x)));
            if col < -2.0 {
                break;
            }
            plot(canvas, col, row.floor(), Glyph::new('▾', 'v'), ink);
        }
    }

    fn draw_dust(&self, canvas: &mut Canvas, view: &View) {
        for p in &self.dust {
            let f = (p.age / p.life).clamp(0.0, 1.0);
            let (x, y) = view.at((p.x, p.y));
            let g = if f < 0.3 { Glyph::new('∙', '.') } else { DOT };
            // Dimmed with the daylight, like the rover that kicks it up (MR-A9).
            let fallback = (g, self.lit(Role::Particle, (200.0 - 130.0 * f) as u8));
            veil(
                canvas,
                x,
                y,
                (60.0 * (1.0 - f) * self.light) as u8,
                fallback,
            );
        }
    }

    /// A rover or wheel ink at `level`, dimmed with the daylight so the rover sits in the
    /// dusk rather than on it (MR-A9).
    fn lit(&self, role: Role, level: u8) -> Ink {
        Ink::new(role, ink::level(f64::from(level) * self.light))
    }

    fn draw_rover(&self, canvas: &mut Canvas, w: &World, view: &View, pose: &Pose, beat: &Beat) {
        let j = pose.joints();
        let hubs = [j.front, j.middle, j.rear];
        let body_ink = self.lit(Role::Text, 235);
        let spin = odometer(self.t) / WHEEL_R;
        let wheel = WHEEL[((-spin / FRAC_PI_2).floor() as i64).rem_euclid(4) as usize];
        if view.s < TINY_SCALE {
            // Tiny: a deck on three wheels and a mast stub.
            let (a, b) = (view.at(pose.body(-2.2, 0.9)), view.at(pose.body(2.2, 0.9)));
            segment(canvas, a, b, HULL, body_ink);
            for hub in hubs {
                let (x, y) = view.at(hub);
                plot(canvas, x, y, wheel, self.lit(Role::Structure, 230));
            }
            let m = view.at(pose.body(1.7, 2.6));
            plot(
                canvas,
                m.0,
                m.1,
                Glyph::new('╻', '|'),
                self.lit(Role::Structure, 200),
            );
            return;
        }
        // Struts are solid blocks once they are about a cell thick, line art below that:
        // a line glyph over solid ground reads as a dark cut, a block as a strut.
        let solid = view.s >= SOLID_SCALE;
        let strut = |canvas: &mut Canvas, a: P2, b: P2, ink: Ink| {
            let (sa, sb) = (view.at(a), view.at(b));
            let line = link_glyph(sb.0 - sa.0, sb.1 - sa.1);
            let g = if solid {
                Glyph::new('█', line.ascii)
            } else {
                line
            };
            segment(canvas, sa, sb, g, ink);
        };
        let link = self.lit(Role::Structure, 150);
        for (a, b) in [
            (j.pivot, j.front),
            (j.pivot, j.bogie),
            (j.bogie, j.middle),
            (j.bogie, j.rear),
        ] {
            strut(canvas, a, b, link);
        }
        // RTG: the power source angled off the back of the deck.
        for dv in [0.0, 0.35] {
            let rtg = self.lit(Role::Structure, 105);
            strut(
                canvas,
                pose.body(-2.2, 0.7 + dv),
                pose.body(-3.1, 1.5 + dv),
                rtg,
            );
        }
        // Body: a tilted box with half-block edges, then mast and camera head.
        fill_box(canvas, view, pose, (-2.2, 2.2), BODY_V, body_ink);
        let belly = self.lit(Role::Structure, 200);
        fill_box(
            canvas,
            view,
            pose,
            (-2.2, 2.2),
            (BODY_V.0, BODY_V.0 + 0.45),
            belly,
        );
        let mast = self.lit(Role::Structure, 185);
        strut(
            canvas,
            pose.body(1.7, BODY_V.1),
            pose.body(1.7, MAST_TOP),
            mast,
        );
        let head = self.lit(Role::Text, 250);
        fill_box(canvas, view, pose, HEAD_U, HEAD_V, head);
        let eye = view.at(pose.body(HEAD_U.1, 0.5 * (HEAD_V.0 + HEAD_V.1)));
        plot(
            canvas,
            eye.0,
            eye.1,
            Glyph::new('◘', 'o'),
            Ink::new(Role::Accent, 200),
        );

        // High-gain antenna: the X-band dish for direct-to-Earth links, parked; the relay
        // to the orbiter goes over the small UHF mast on the rear deck.
        let post = pose.body(HGA.0, HGA.1);
        strut(canvas, pose.body(HGA.0, BODY_V.1), post, link);
        strut(
            canvas,
            pose.body(UHF.0, BODY_V.1),
            pose.body(UHF.0, UHF.1),
            link,
        );
        let (sa, ca) = (pose.phi - 0.5).sin_cos_det();
        let half = 0.6;
        let dish = self.lit(Role::Structure, 225);
        let (d0, d1) = (
            add(post, (-half * ca, half * sa)),
            add(post, (half * ca, -half * sa)),
        );
        strut(canvas, d0, d1, dish);
        for hub in hubs {
            self.draw_wheel(canvas, view, hub, wheel);
        }
        // Robotic arm with its turret, and the drill bit while coring.
        let (s, e, t) = arm(pose, &w.near, arm_extension(beat));
        let arm_ink = self.lit(Role::Structure, 205);
        strut(canvas, s, e, arm_ink);
        strut(canvas, e, t, arm_ink);
        let (se, st) = (view.at(e), view.at(t));
        plot(canvas, se.0, se.1, JOINT, self.lit(Role::Structure, 240));
        plot(
            canvas,
            st.0,
            st.1,
            Glyph::new('▣', '#'),
            self.lit(Role::Text, 240),
        );
        if beat.index == PH_DRILL {
            let bit = if (self.t * 24.0).sin_det() > 0.0 {
                Glyph::new('┃', '|')
            } else {
                Glyph::new('╽', '!')
            };
            plot(canvas, st.0, st.1 + 1.0, bit, Ink::new(Role::Hot, 255));
        }
    }

    fn draw_wheel(&self, canvas: &mut Canvas, view: &View, hub: P2, glyph: Glyph) {
        let (x, y) = view.at(hub);
        let r = WHEEL_R * view.s;
        if r >= 0.8 {
            draw::disc(canvas, x, y, r + 0.2, |_, _, _| {
                Some((TYRE, self.lit(Role::Structure, 70)))
            });
        }
        plot(canvas, x, y, glyph, self.lit(Role::Structure, 245));
    }

    /// Data packets climbing from the UHF antenna at `a` to the orbiter at `o`.
    fn draw_packets(&self, canvas: &mut Canvas, a: P2, o: P2) {
        const PACKETS: usize = 5;
        for i in 0..PACKETS {
            let q = (i as f64 / PACKETS as f64 + self.t * 0.9).fract();
            let (x, y) = (ease::lerp(a.0, o.0, q), ease::lerp(a.1, o.1, q));
            plot(canvas, x, y, PIXEL, Ink::new(Role::Wave, 240));
        }
    }

    fn draw_captions(&self, canvas: &mut Canvas, w: &World, beat: &Beat, hour: f64) {
        use std::fmt::Write;
        let l = self.layout;
        if l.h < 6.0 {
            return;
        }
        let width = l.w as usize;
        let cycle = (self.t / CYCLE).floor() as i64;
        let mut right = StackText::<48>::new();
        let _ = match beat.index {
            PH_DRIVE => write!(right, "DRIVE {:.1} m", drive_profile(beat.local).0 * UNIT_M),
            PH_DEPLOY => write!(right, "ARM DEPLOY"),
            PH_DRILL => write!(right, "CORING SAMPLE #{}", sample_number(cycle)),
            PH_STOW => write!(right, "ARM STOW"),
            PH_IMAGE => write!(right, "MASTCAM-Z IMAGING"),
            PH_TRANSMIT => write!(right, "UHF RELAY > ORBITER"),
            _ => write!(right, "SLEEP"),
        };
        let mut left = StackText::<48>::new();
        // Local true solar time: 12 h at the Sun's transit (no equation of time, which
        // on Mars reaches ~±50 min, so not LMST).
        let ltst = (12.0 + hour.to_degrees() / 15.0).rem_euclid(24.0);
        // The sol number ticks over at local midnight.
        let midnight = SOL_KNOTS[2].0
            + (SOL_KNOTS[3].0 - SOL_KNOTS[2].0) * (180.0 - SOL_KNOTS[2].1)
                / (SOL_KNOTS[3].1 - SOL_KNOTS[2].1);
        let sol = i64::from(w.sol0) + ((self.t - midnight) / SOL).floor() as i64 + 1;
        let _ = write!(
            left,
            "SOL {sol}  LTST {:02}:{:02}",
            ltst.floor() as u32,
            (ltst.fract() * 60.0).floor() as u32
        );
        let ink = Ink::new(Role::Text, 170);
        let (ll, rl) = (left.len(), right.len());
        if rl + 2 <= width {
            canvas.text((width - rl - 1) as i32, 0, right.as_str(), ink);
        }
        if ll + rl + 5 <= width {
            canvas.text(1, 0, left.as_str(), ink);
        }
    }
}

/// The highest Blackbody level a dust veil lifts the sky to: ~5000 K, a warm tan below
/// the ramp's white point, so dust never pales the sky into its blue end (MR-A10).
const DUST_WARM: u8 = 118;

/// Semi-transparent overlay: lighten the half-block landscape (or the rover's hull)
/// already in the cell by `boost`, or draw `fallback` over empty or line-art cells.
fn veil(canvas: &mut Canvas, x: f64, y: f64, boost: u8, fallback: (Glyph, Ink)) {
    let (x, y) = (x.round(), y.round());
    let size = canvas.size();
    if !((0.0..f64::from(size.width())).contains(&x)
        && (0.0..f64::from(size.height())).contains(&y))
    {
        return;
    }
    let (xi, yi) = (x as i32, y as i32);
    let Some(cell) = canvas.get(xi, yi).copied() else {
        return;
    };
    let up = |i: Ink| match i.role {
        Role::Background => i,
        // Dust over the sky stays tan, short of the ramp's white point (MR-A10).
        Role::Blackbody => {
            let cap = i.level.max(DUST_WARM);
            Ink::new(i.role, i.level.saturating_add(boost).min(cap))
        }
        // The aureole stays under the Sun's white (MR-A7).
        Role::Ramp(Ramp::Ice) => Ink::new(
            i.role,
            i.level
                .saturating_add(boost)
                .min(i.level.max(AUREOLE_MAX as u8)),
        ),
        _ => Ink::new(i.role, i.level.saturating_add(boost)),
    };
    let ascii = canvas.mode() == GlyphMode::Ascii;
    if !ascii && matches!(cell.ch, '▀' | '▄') {
        canvas.put_bg(xi, yi, Glyph::new(cell.ch, '#'), up(cell.ink), up(cell.bg));
    } else if cell.ch == if ascii { HULL.ascii } else { HULL.unicode } {
        canvas.put(xi, yi, HULL, up(cell.ink));
    } else {
        canvas.put(xi, yi, fallback.0, fallback.1);
    }
}

/// Fill the body-frame box `u ∈ [u0, u1]`, `v ∈ [v0, v1]` column by column: a cell
/// is covered when its centre lies between the box's bottom and top edges.
///
/// Whole cells only: a half-block leaves its other half at the terminal background,
/// which shows as a black seam against the solid sky and ground.
fn fill_box(canvas: &mut Canvas, view: &View, pose: &Pose, u: (f64, f64), v: (f64, f64), ink: Ink) {
    let (sn, cs) = pose.phi.sin_cos_det();
    if cs.abs() < 0.2 {
        return;
    }
    let corners = [
        view.at(pose.body(u.0, v.0)),
        view.at(pose.body(u.1, v.0)),
        view.at(pose.body(u.0, v.1)),
        view.at(pose.body(u.1, v.1)),
    ];
    let lo = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let hi = corners
        .iter()
        .map(|c| c.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let (w, h) = (
        f64::from(canvas.size().width()),
        f64::from(canvas.size().height()),
    );
    if !(lo.is_finite() && hi.is_finite()) || hi < 0.0 || lo >= w {
        return;
    }
    // Screen row of the box edge `v = vv` at column `x`, if the column crosses it.
    let edge = |x: f64, vv: f64| -> Option<f64> {
        let wx = view.cam + x / view.k;
        let uu = (wx - pose.pivot.0 + vv * sn) / cs;
        (u.0..=u.1)
            .contains(&uu)
            .then(|| view.at(pose.body(uu, vv)).1)
    };
    for c in lo.floor().max(0.0) as i32..=hi.ceil().min(w - 1.0) as i32 {
        let x = f64::from(c);
        let (Some(bot), Some(top)) = (edge(x, v.0), edge(x, v.1)) else {
            continue;
        };
        let (r0, r1) = (top.round().max(0.0), bot.round().min(h));
        // A box thinner than a cell still gets its middle row.
        let (r0, r1) = if r1 > r0 {
            (r0, r1)
        } else {
            let m = (0.5 * (top + bot)).floor();
            (m, m + 1.0)
        };
        for r in r0 as i32..r1 as i32 {
            canvas.put(c, r, HULL, ink);
        }
    }
}

/// The faint UHF link from `a` to `o`: a dot every third cell over the sky, drawn before
/// the rover so it passes behind it. Over the half-block picture the dot replaces the
/// half block and `backdrop` puts the sky back behind it (CORE-R86), so the link shows by
/// day as well as by night.
fn draw_beam(canvas: &mut Canvas, a: P2, o: P2) {
    let n = ((o.0 - a.0).abs().max((o.1 - a.1).abs()) / 3.0).min(200.0) as usize;
    for i in 1..n {
        let q = i as f64 / n as f64;
        let (x, y) = (ease::lerp(a.0, o.0, q), ease::lerp(a.1, o.1, q));
        if canvas
            .get(x.round() as i32, y.round() as i32)
            .is_some_and(|c| matches!(c.ch, ' ' | '▀' | '▄'))
        {
            plot(canvas, x, y, DOT, Ink::new(Role::Wave, 150));
        }
    }
}

/// Bresenham segment between fractional cell points; skips absurd coordinates.
fn segment(canvas: &mut Canvas, a: P2, b: P2, glyph: Glyph, ink: Ink) {
    let lim = 4.0 * f64::from(canvas.size().width().max(canvas.size().height())) + 64.0;
    let ok = |v: f64| v.is_finite() && v.abs() < lim;
    if !(ok(a.0) && ok(a.1) && ok(b.0) && ok(b.1)) {
        return;
    }
    let (x0, y0, x1, y1) = (
        a.0.round() as i32,
        a.1.round() as i32,
        b.0.round() as i32,
        b.1.round() as i32,
    );
    for (x, y) in geom::line(x0, y0, x1, y1) {
        canvas.put(x, y, glyph, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ridges() -> Vec<Ridge> {
        (0..4u64)
            .map(|k| Ridge::new(0x5eed ^ (k * 7919), NEAR_AMP, NEAR_WAVE))
            .collect()
    }

    #[test]
    fn mr_a1_every_wheel_contact_lies_on_the_terrain_surface() {
        let mut worst: f64 = 0.0;
        for r in ridges() {
            for i in 0..1500 {
                let px = -3000.0 + 4.1 * f64::from(i);
                let pose = solve_pose(|x| r.eval(x), px);
                for (cx, cy) in pose.wheel_centers() {
                    let err = (cy - WHEEL_R - r.height(cx)).abs();
                    // 0.5 row at the largest zoom is 0.5 / SCALE_MAX world units.
                    assert!(
                        err <= 0.5 / SCALE_MAX,
                        "wheel off surface at px={px}: {err}"
                    );
                    worst = worst.max(err);
                }
                // Rigid links: forward kinematics keeps every link length exact.
                let j = pose.joints();
                let len = |a: P2, b: P2| (a.0 - b.0).hypot(a.1 - b.1);
                assert!((len(j.pivot, j.front) - AF.hypot(BF)).abs() < 1e-9);
                assert!((len(j.pivot, j.bogie) - AB.hypot(BB)).abs() < 1e-9);
                assert!((len(j.middle, j.rear) - 2.0 * BOGIE_C).abs() < 1e-9);
            }
        }
        // Newton converges far inside the spec's half-row tolerance.
        assert!(worst < 1e-6, "worst contact residual {worst}");
    }

    #[test]
    fn mr_a1_generator_wheels_sit_on_the_drawn_ground() {
        // Through the real generator and screen mapping: each hub's contact row is
        // within half a row of the ground row at that column, all through a cycle.
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(11, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let w = g.world.unwrap();
        let l = g.layout;
        for _ in 0..(CYCLE / 0.25) as usize {
            g.update(0.25, &mut ctx);
            g.prepare();
            let rx = g.rover_x(&w);
            let pose = solve_pose(|x| w.near.eval(x), rx);
            let cam = camera(&l, rx, g.t);
            for (hx, hy) in pose.wheel_centers() {
                let contact_row = l.near_base - l.s * (hy - WHEEL_R);
                let ground_row = l.near_base - l.s * w.near.height(hx);
                assert!((contact_row - ground_row).abs() <= 0.5);
                assert!((0.0..l.w).contains(&((hx - cam) * l.s * ASPECT)));
            }
        }
    }

    /// Cells whose glyph or background ink has `role`, rendered at sim time `t`.
    fn count_at(t: f64, role: Role) -> usize {
        let mut s = Stage::new(build(), 5, Size::new(120, 40), GlyphMode::Unicode);
        while s.ctx().time() < t {
            s.step(0.25);
        }
        s.render()
            .cells()
            .iter()
            .filter(|c| c.ink.role == role || c.bg.role == role)
            .count()
    }

    #[test]
    fn mr_a4_sky_is_butterscotch_by_day_pale_round_a_low_sun_and_dark_at_night() {
        // Sun elevation: ~2° at t = 47 s (just before sunset), ~57° near t = 12 s, and
        // well below the horizon at t = 70 s.
        assert!(sun_elevation(hour_angle(47.0)).to_degrees() < 5.0);
        assert!(sun_elevation(hour_angle(12.0)).to_degrees() > 50.0);
        assert!(sun_elevation(hour_angle(70.0)).to_degrees() < -30.0);
        // A Blackbody sky, butterscotch (its tan stretch) by day; a blue `Ramp(Ice)`
        // aureole round a low Sun, and only there.
        let sky = |t: f64| {
            let (g, _) = scene_at(t);
            let hour = hour_angle(g.t);
            let sun = g.sun_cell(hour, sun_elevation(hour));
            let r = (GLOW_R * g.layout.horizon).max(2.0);
            let (mut tan, mut blue) = (Vec::new(), 0);
            for py in 0..80 {
                for x in 0..120 {
                    let ink = g.px.get(x, py).unwrap();
                    if layer_at(g.tops[x as usize], py) != Layer::Sky || ink.level == 255 {
                        continue;
                    }
                    match ink.role {
                        Role::Blackbody => tan.push(ink.level),
                        Role::Ramp(Ramp::Ice) => {
                            let d = ((f64::from(x) + 0.5 - sun.0) / ASPECT)
                                .hypot((f64::from(py) + 0.5) / 2.0 - sun.1);
                            assert!(d < 3.0 * r, "t {t}: blue sky far from the Sun at {x},{py}");
                            blue += 1;
                        }
                        _ => {}
                    }
                }
            }
            (tan, blue)
        };
        let (noon, noon_blue) = sky(12.0);
        assert!(noon.len() > 120 * 40 / 2, "a Blackbody day sky");
        assert_eq!(noon_blue, 0, "no aureole with the Sun high");
        let tan = noon.iter().filter(|&&l| (70..=125).contains(&l)).count();
        assert!(tan * 10 > noon.len() * 9, "butterscotch day sky");
        let (_, dusk_blue) = sky(47.0);
        assert!(dusk_blue > 60, "no blue aureole at sunset: {dusk_blue}");
        assert_eq!(sky(70.0).1, 0, "no aureole at night");
        assert!(count_at(70.0, Role::Star) > 5, "stars at night");
    }

    #[test]
    fn mr_a4_far_ridges_take_the_dusty_air_and_the_near_ground_is_rust() {
        let (g, _) = scene_at(12.0);
        let mut roles = [None; 3];
        for py in 0..80 {
            for x in 0..120 {
                let slot = match layer_at(g.tops[x as usize], py) {
                    Layer::Far => 0,
                    Layer::Mid => 1,
                    Layer::Near => 2,
                    Layer::Sky => continue,
                };
                roles[slot] = g.px.get(x, py).map(|i| i.role);
            }
        }
        assert_eq!(
            roles[0],
            Some(Role::Blackbody),
            "far ridge in the sky's ramp"
        );
        assert_eq!(roles[2], Some(Role::Ramp(Ramp::Ember)), "rust foreground");
    }

    /// A scene at 120×40 stepped to sim time `t`, and its rendered frame.
    fn scene_at(t: f64) -> (MarsRover, Canvas) {
        scene_sized(t, Size::new(120, 40))
    }

    fn scene_sized(t: f64, size: Size) -> (MarsRover, Canvas) {
        let mut ctx = Ctx::new(5, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        while g.t < t {
            g.update(0.25, &mut ctx);
            g.prepare();
        }
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        (g, canvas)
    }

    #[test]
    fn mr_a4_stars_come_out_only_once_the_aureole_has_faded() {
        for i in 0..=600 {
            let e = -30.0 + 0.1 * f64::from(i);
            if night(e) > 0.0 {
                assert!(glow_strength(e) < 0.05, "stars and glow together at {e}°");
            }
        }
        // The sky fades with the glow: dusk is still lit where the aureole is strong.
        for i in 0..=400 {
            let e = -20.0 + 0.1 * f64::from(i);
            if glow_strength(e) > 0.5 {
                assert!(daylight(e) > 0.2, "glow on a black sky at {e}°");
            }
        }
    }

    /// Relative luminance of `ink` under the astro theme: the Blackbody ramp's lightness
    /// peaks at its white point, so a level is not a brightness.
    fn luminance(ink: Ink) -> f64 {
        let sciart_render::Rgb(r, g, b) = sciart_render::Theme::Astro.color(ink, 0, 0);
        let lin = |c: u8| (f64::from(c) / 255.0).powf(2.2);
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    #[test]
    fn mr_a4_the_aureole_fades_away_from_the_sun() {
        let (g, _) = scene_at(47.0);
        let l = g.layout;
        let hour = hour_angle(g.t);
        let sun = g.sun_cell(hour, sun_elevation(hour));
        let r = (GLOW_R * l.horizon).max(2.0);
        // Mean sky luminance in rings around the Sun, going out: brightest at the Sun,
        // with no bright rim round a dimmer core.
        let mut rings = [(0.0f64, 0u32); 6];
        for py in 0..80 {
            for x in 0..120 {
                let d = ((f64::from(x) + 0.5 - sun.0) / ASPECT)
                    .hypot((f64::from(py) + 0.5) / 2.0 - sun.1);
                let ring = (d / (0.4 * r)) as usize;
                let ink = g.px.get(x, py).unwrap();
                let sky = layer_at(g.tops[x as usize], py) == Layer::Sky;
                if ring < rings.len() && sky && is_sky_ink(ink) {
                    rings[ring].0 += luminance(ink);
                    rings[ring].1 += 1;
                }
            }
        }
        // Rings the ridge hides (at sunset, the inner ones) have no sky to judge.
        let seen: Vec<f64> = rings
            .iter()
            .filter(|s| s.1 >= 20)
            .map(|&(sum, n)| sum / f64::from(n))
            .collect();
        for w in seen.windows(2) {
            assert!(w[1] <= w[0] + 0.01, "{seen:?}");
        }
        assert!(seen[0] > 2.0 * seen[seen.len() - 1], "{seen:?}");
        // And at 200×50, straight up from the setting Sun, over 4-pixel windows (one
        // dither period): the sky never brightens going out (no bright rim round the
        // blue core).
        for t in [45.0, 47.0, 50.0] {
            let (g, _) = scene_sized(t, Size::new(200, 50));
            let hour = hour_angle(g.t);
            let sun = g.sun_cell(hour, sun_elevation(hour));
            let x = sun.0 as i32;
            let mut column = Vec::new();
            for py in (0..(2.0 * sun.1) as i32).rev() {
                let ink = g.px.get(x, py).unwrap();
                if layer_at(g.tops[x as usize], py) == Layer::Sky && is_sky_ink(ink) {
                    column.push(luminance(ink));
                }
            }
            let means: Vec<f64> = column
                .chunks_exact(4)
                .map(|c| c.iter().sum::<f64>() / 4.0)
                .collect();
            assert!(means.len() >= 3, "t {t}: too little sky above the Sun");
            for w in means.windows(2) {
                assert!(w[1] <= w[0] + 0.02, "t {t}: {means:?}");
            }
        }
    }

    #[test]
    fn mr_a4_the_aureole_edge_changes_hue_not_brightness() {
        for tan in (50..=125u8).step_by(5) {
            let bb = luminance(Ink::new(Role::Blackbody, tan));
            let ice = luminance(Ink::new(
                Role::Ramp(Ramp::Ice),
                ink::level(ice_matching(f64::from(tan))),
            ));
            assert!(
                (ice / bb - 1.0).abs() < 0.15,
                "level {tan}: ice {ice} vs tan {bb}"
            );
        }
    }

    /// The sky's own inks: the tan Blackbody sky and the blue aureole.
    fn is_sky_ink(ink: Ink) -> bool {
        matches!(ink.role, Role::Blackbody | Role::Ramp(Ramp::Ice))
    }

    #[test]
    fn mr_a4_the_aureole_is_blue_in_every_theme() {
        // At sunset the aureole's pixels are blue in astro (b > r) and brighter than the
        // tan sky around them in mono, so it reads in every theme.
        let (g, _) = scene_at(47.0);
        let (mut blue, mut tan) = ((0.0, 0u32), (0.0, 0u32));
        for py in 0..80 {
            for x in 0..120 {
                let ink = g.px.get(x, py).unwrap();
                if layer_at(g.tops[x as usize], py) != Layer::Sky {
                    continue;
                }
                let mono = |i: Ink| {
                    let sciart_render::Rgb(r, _, _) = sciart_render::Theme::Mono.color(i, 0, 0);
                    f64::from(r)
                };
                match ink.role {
                    Role::Ramp(Ramp::Ice) => {
                        let sciart_render::Rgb(r, _, b) =
                            sciart_render::Theme::Astro.color(ink, 0, 0);
                        assert!(b > r, "not blue: {ink:?}");
                        blue = (blue.0 + mono(ink), blue.1 + 1);
                    }
                    Role::Blackbody => tan = (tan.0 + mono(ink), tan.1 + 1),
                    _ => {}
                }
            }
        }
        assert!(blue.1 > 0 && tan.1 > 0);
        assert!(
            blue.0 / f64::from(blue.1) > tan.0 / f64::from(tan.1),
            "mono aureole {blue:?} vs sky {tan:?}"
        );
    }

    #[test]
    fn mr_a4_far_ridges_darken_into_the_night() {
        // Mean luminance of the far ridges and the near ground at sim time `t`.
        let means = |t: f64| {
            let (g, _) = scene_at(t);
            let (mut far, mut near) = ((0.0, 0u32), (0.0, 0u32));
            for py in 0..80 {
                for x in 0..120 {
                    let ink = g.px.get(x, py).unwrap();
                    match layer_at(g.tops[x as usize], py) {
                        Layer::Far => {
                            assert_ne!(ink, Ink::BLANK, "t {t}: the ridge is always there");
                            far = (far.0 + luminance(ink), far.1 + 1);
                        }
                        Layer::Near => near = (near.0 + luminance(ink), near.1 + 1),
                        _ => {}
                    }
                }
            }
            (far.0 / f64::from(far.1), near.0 / f64::from(near.1))
        };
        let (far_day, _) = means(12.0);
        let (far, near) = means(70.0);
        // At night a dim silhouette, not sunlit hills: no brighter than the foreground.
        assert!(
            far <= near,
            "far {far} brighter than the near ground {near} at night"
        );
        assert!(
            far < far_day / 3.0,
            "far ridge at night {far}, by day {far_day}"
        );
    }

    #[test]
    fn mr_a10_dust_veils_stay_warm_over_the_sky() {
        // A veil lightens the dust-coloured sky toward tan, never past the ramp's white
        // point into its blue end.
        let size = Size::new(4, 2);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        let sky = Ink::new(Role::Blackbody, 100);
        let ember = ramp(Ramp::Ember, 0.4);
        canvas.put_bg(0, 0, Glyph::new('▀', '#'), sky, sky);
        canvas.put_bg(1, 0, Glyph::new('▀', '#'), ember, ember);
        veil(&mut canvas, 0.0, 0.0, 70, (LIGHT, Ink::new(Role::Hot, 100)));
        veil(&mut canvas, 1.0, 0.0, 70, (LIGHT, Ink::new(Role::Hot, 100)));
        let c = canvas.get(0, 0).copied().unwrap();
        assert!(
            c.ink.level >= sky.level && c.ink.level <= DUST_WARM,
            "{c:?}"
        );
        assert!(c.bg.level >= sky.level && c.bg.level <= DUST_WARM, "{c:?}");
        let e = canvas.get(1, 0).copied().unwrap();
        assert!(
            e.ink.level > ember.level,
            "the ground still lightens: {e:?}"
        );
        // In the scene: through a day with live devils and wheel dust, no Blackbody cell is
        // lifted past both its own sky pixel and the warm cap.
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(5, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        let mut veiled = 0;
        while g.t < 40.0 {
            g.update(0.25, &mut ctx);
            g.prepare();
            canvas.clear();
            g.render(&mut canvas);
            for y in 0..50 {
                for x in 0..200 {
                    let cell = canvas.get(x, y).copied().unwrap();
                    let under = [g.px.get(x, 2 * y), g.px.get(x, 2 * y + 1)]
                        .into_iter()
                        .flatten()
                        .filter(|i| i.role == Role::Blackbody)
                        .map(|i| i.level)
                        .max()
                        .unwrap_or(0);
                    for ink in [cell.ink, cell.bg] {
                        if ink.role == Role::Blackbody && ink.level > under {
                            veiled += 1;
                            assert!(ink.level <= DUST_WARM, "t {}: {cell:?} at {x},{y}", g.t);
                        }
                    }
                }
            }
        }
        assert!(veiled > 20, "dust veiled the sky: {veiled}");
    }

    #[test]
    fn mr_a7_the_sun_tops_its_ramp_in_every_theme() {
        // A ramp's top level is its brightest in every theme; a 6000 K blackbody reads
        // mid-grey in mono, a hole in the bright sky.
        for t in [12.0, 30.0] {
            let (g, _) = scene_at(t);
            let (mut sun, mut others) = (0, 0u8);
            for py in 0..80 {
                for x in 0..120 {
                    let ink = g.px.get(x, py).unwrap();
                    if matches!(ink.role, Role::Ramp(_)) && ink.level == 255 {
                        sun += 1;
                    } else if matches!(ink.role, Role::Ramp(_)) {
                        others = others.max(ink.level);
                    }
                }
            }
            assert!(
                sun > 0 && others < 245,
                "t={t}: sun {sun}, brightest other {others}"
            );
        }
    }

    #[test]
    fn mr_a8_night_stars_are_braille_and_keep_off_the_readout() {
        let (_, canvas) = scene_at(70.0);
        let mut stars = 0;
        for y in 0..40 {
            for x in 0..120 {
                let cell = canvas.get(x, y).unwrap();
                if cell.ink.role == Role::Star {
                    assert_ne!(y, 0, "star in the readout row at {x}");
                    assert!(('\u{2801}'..='\u{28FF}').contains(&cell.ch), "{cell:?}");
                    stars += 1;
                }
            }
        }
        assert!(stars > 20, "{stars} stars");
        // Only in all-sky cells: a star in a cell whose other pixel is ridge would turn the
        // skyline's half block into braille over a solid ridge-coloured cell.
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(42, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        while g.t < SOL {
            g.update(1.0 / 6.0, &mut ctx);
            g.prepare();
            if g.t < 60.0 {
                continue;
            }
            canvas.clear();
            g.render(&mut canvas);
            for y in 0..50 {
                for x in 0..200 {
                    if canvas.get(x, y).unwrap().ink.role == Role::Star {
                        assert_eq!(g.px.cell_bg(x, y), Ink::BLANK, "t {}: star at {x},{y}", g.t);
                    }
                }
            }
        }
    }

    #[test]
    fn mr_a12_the_shadow_swings_with_the_sun_and_the_rover_crosses_the_frame() {
        // No shadow at night; by day it covers the footprint and stretches down-Sun,
        // longer as the Sun sinks, capped.
        assert!(shadow_span(10.0, 1.0, -0.1).is_none());
        let (lo, hi) = shadow_span(10.0, 1.0, 0.8).unwrap();
        assert!(lo < 10.0 - SHADOW_HALF - 0.5 && (hi - (10.0 + SHADOW_HALF)).abs() < 1e-9);
        let (lo2, hi2) = shadow_span(10.0, -1.0, 0.8).unwrap();
        assert!(hi2 > 10.0 + SHADOW_HALF + 0.5 && (lo2 - (10.0 - SHADOW_HALF)).abs() < 1e-9);
        let len = |e: f64| {
            let (a, b) = shadow_span(0.0, 1.0, e).unwrap();
            b - a
        };
        assert!(len(0.2) > len(0.5) && len(0.5) > len(1.2));
        assert!(len(0.01) <= SHADOW_MAX + 2.0 * SHADOW_HALF + 1e-9);
        // The rover crosses the frame while it drives, holds, and returns overnight.
        let w = 120.0;
        assert!((rover_col(w, 0.0) - ROVER_COLS.0 * w).abs() < 1e-9);
        assert!((rover_col(w, DRIVE) - ROVER_COLS.1 * w).abs() < 1e-9);
        assert!((rover_col(w, DRIVE + DEPLOY + DRILL + STOW) - ROVER_COLS.1 * w).abs() < 1e-9);
        assert!((rover_col(w, CYCLE - 1e-9) - ROVER_COLS.0 * w).abs() < 1e-6);
        let cols: Vec<f64> = (0..=36).map(|i| rover_col(w, f64::from(i) * 0.5)).collect();
        assert!(cols.windows(2).all(|p| p[1] >= p[0] - 1e-12));
        assert!(rover_col(w, 5.0) + 0.1 * w < rover_col(w, 30.0));
    }

    #[test]
    fn mr_a9_the_rover_dims_with_the_daylight() {
        let hull = |canvas: &Canvas| {
            canvas
                .cells()
                .iter()
                .filter(|c| c.ch == HULL.unicode && c.ink.role == Role::Text)
                .map(|c| c.ink.level)
                .max()
                .unwrap_or(0)
        };
        let (day, night) = (hull(&scene_at(12.0).1), hull(&scene_at(70.0).1));
        assert!(day > 200, "day hull {day}");
        assert!(
            f64::from(night) < 0.6 * f64::from(day),
            "night hull {night}"
        );
    }

    #[test]
    fn mr_a5_every_column_stacks_sky_far_mid_near_downward() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(3, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let w = g.world.unwrap();
        for _ in 0..20 {
            g.update(0.5, &mut ctx);
            g.prepare();
            let cam = camera(&g.layout, g.rover_x(&w), g.t);
            for c in 0..120 {
                let (tops, _) = g.layer_tops(&w, cam, c);
                let mut last = Layer::Sky;
                for py in 0..80 {
                    let l = layer_at(tops, py);
                    assert!(l >= last, "column {c}: {l:?} above {last:?} at pixel {py}");
                    last = l;
                }
                assert_eq!(last, Layer::Near, "column {c} ends in the near ground");
            }
        }
    }

    #[test]
    fn mr_a6_rover_line_art_keeps_the_sky_behind_it() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(5, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        for _ in 0..(10.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut checked = 0;
        for y in 0..40 {
            for x in 0..120 {
                let cell = canvas.get(x, y).unwrap();
                let backed =
                    g.px.get(x, 2 * y + 1)
                        .is_some_and(|i| i.role != Role::Background);
                if cell.ch != ' ' && !matches!(cell.ch, '▀' | '▄' | '█') && backed {
                    checked += 1;
                    assert_ne!(
                        cell.bg,
                        Ink::BLANK,
                        "black box behind {:?} at {x},{y}",
                        cell.ch
                    );
                }
            }
        }
        assert!(checked > 5, "some rover line art over the landscape");
    }

    #[test]
    fn mr_a3_dust_rises_while_driving_and_drilling_and_settles_otherwise() {
        let size = Size::new(160, 48);
        let mut ctx = Ctx::new(9, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let run_to = |g: &mut MarsRover, ctx: &mut Ctx, t: f64| {
            while g.t < t {
                g.update(SIM_DT, ctx);
                g.prepare();
            }
        };
        run_to(&mut g, &mut ctx, 8.0);
        assert!(g.dust.len() > 10, "wheels kick up dust: {}", g.dust.len());
        run_to(&mut g, &mut ctx, DRIVE + DEPLOY + 2.0);
        assert!(g.dust.len() > 20, "drill throws dust: {}", g.dust.len());
        run_to(&mut g, &mut ctx, CYCLE - 0.5);
        assert!(g.dust.is_empty(), "dust settles: {}", g.dust.len());
    }

    #[test]
    fn mr_a1_ramp_pitches_the_body_by_the_slope_angle() {
        for m in [-0.4, -0.1, 0.0, 0.25, 0.6] {
            let pose = solve_pose(|x| (m * x + 1.0, m), 3.0);
            assert!((pose.phi - f64::atan(m)).abs() < 1e-9, "phi {}", pose.phi);
            assert!((pose.theta - f64::atan(m)).abs() < 1e-9);
        }
    }

    #[test]
    fn mr_a2_terrain_is_a_pure_function_of_world_x() {
        let a = Ridge::new(42, 3.2, 14.0);
        let b = Ridge::new(42, 3.2, 14.0);
        let xs = [-1e6, -7.3, 0.0, 13.99, 14.0, 1e5 + 0.5];
        let fwd: Vec<f64> = xs.iter().map(|&x| a.height(x)).collect();
        let rev: Vec<f64> = xs.iter().rev().map(|&x| b.height(x)).collect();
        assert!(fwd.iter().eq(rev.iter().rev()));
        assert!(fwd.iter().all(|h| (0.0..3.2).contains(h)));

        // The generator draws its seed once: scrolling and resizing never change it.
        let mut g = MarsRover::default();
        let mut ctx = Ctx::new(3, Size::new(80, 24));
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        let before = g.world.expect("seeded in first resize");
        for (w, h) in [(200, 50), (1, 1), (37, 11)] {
            for _ in 0..30 {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
            }
            g.resize(Size::new(w, h), &mut ctx);
            g.prepare();
        }
        assert_eq!(g.world, Some(before));
    }

    #[test]
    fn mr_a2_adjacent_columns_obey_the_construction_bound() {
        for r in ridges() {
            let l = r.lipschitz();
            for s in [SCALE_MIN, 1.0, SCALE_MAX] {
                let dx = 1.0 / (s * ASPECT);
                let mut worst: f64 = 0.0;
                for i in 0..20_000 {
                    let x = -5000.0 + 0.5003 * f64::from(i);
                    worst = worst.max((r.height(x + dx) - r.height(x)).abs());
                }
                assert!(worst <= l * dx + 1e-12, "{worst} > {}", l * dx);
                // The bound is meaningful, not vacuous.
                assert!(worst >= 0.2 * l * dx, "{worst} << {}", l * dx);
            }
            // Across every lattice boundary of every octave the height is continuous.
            let mut cell = r.wavelength;
            for _ in 0..OCTAVES {
                for k in -50..50 {
                    let x = f64::from(k) * cell;
                    let eps = 1e-9 * cell.max(1.0);
                    assert!((r.height(x + eps) - r.height(x - eps)).abs() <= l * 2.0 * eps + 1e-12);
                }
                cell /= 2.0;
            }
        }
    }

    #[test]
    fn mr_a3_sol_timeline_visits_phases_in_order_and_loops() {
        let tl = sol_timeline();
        let want = [DRIVE, DEPLOY, DRILL, STOW, IMAGE, TRANSMIT, SLEEP];
        assert!((tl.total() - want.iter().sum::<f64>()).abs() < 1e-12);
        let h = 1e-3;
        let (mut last, mut since, mut seen) = (tl.at(0.0).index, 0.0, Vec::new());
        let mut t = 0.0;
        while t < 2.0 * tl.total() {
            t += h;
            let i = tl.at(t).index;
            since += h;
            if i != last {
                assert_eq!(i, (last + 1) % want.len(), "phase order");
                seen.push((last, since));
                last = i;
                since = 0.0;
            }
        }
        assert!(seen.len() >= 13);
        for &(i, d) in &seen[..13] {
            assert!((d - want[i]).abs() < 2.0 * h, "phase {i} lasted {d}");
        }
    }

    #[test]
    fn mr_a3_one_plan_per_sol_works_by_day_relays_at_dusk_and_sleeps_at_night() {
        let tl = sol_timeline();
        assert_eq!(tl.total(), SOL);
        for i in 0..(2.0 * SOL * 10.0) as i32 {
            let t = f64::from(i) * 0.1;
            let e = sun_elevation(hour_angle(t)).to_degrees();
            match tl.at(t).index {
                // Driving and coring in full daylight.
                PH_DRIVE | PH_DEPLOY | PH_DRILL | PH_STOW => {
                    assert!(e > 20.0, "work at {t} s with the Sun at {e:.1}°");
                }
                // The relay pass in the evening, the Sun low but up.
                PH_TRANSMIT => assert!((0.0..20.0).contains(&e), "relay at {e:.1}°"),
                _ => {}
            }
            // After dark the rover is asleep and still.
            if e < 0.0 {
                assert!(tl.at(t).index > PH_TRANSMIT, "awake at {t} s, Sun {e:.1}°");
                assert_eq!(speed_at(t), 0.0);
            }
        }
        // One core per sol, numbered within Perseverance's 38 sample tubes.
        assert_eq!(sample_number(0), 1);
        assert!((0..500).all(|c| (1..=SAMPLE_TUBES).contains(&sample_number(c))));
        assert_eq!(sample_number(SAMPLE_TUBES), 1);
    }

    #[test]
    fn mr_a11_the_uhf_link_shows_against_the_evening_sky() {
        // The relay pass, Sun still up: packets and a dotted link over the lit sky, and
        // the readout names the UHF relay.
        let start = DRIVE + DEPLOY + DRILL + STOW + IMAGE;
        assert!(sun_elevation(hour_angle(start + 2.0)) > 0.0);
        let (g, canvas) = scene_at(start + 2.0);
        let o = g
            .orbiter(&g.plan.at(g.t))
            .expect("orbiter up during the relay");
        let beam = canvas
            .cells()
            .iter()
            .filter(|c| c.ch == DOT.unicode && c.ink.role == Role::Wave)
            .count();
        assert!(beam >= 4, "link dots over the sky: {beam}");
        let row0: String = (0..120).map(|x| canvas.get(x, 0).unwrap().ch).collect();
        assert!(row0.contains("UHF RELAY"), "{row0}");
        assert!(o.1 < g.layout.horizon, "orbiter in the sky");
    }

    #[test]
    fn mr_a3_rover_moves_only_while_driving_and_continuously() {
        let tl = sol_timeline();
        let mut prev = odometer(0.0);
        for i in 1..=6200 {
            let t = f64::from(i) * 0.01;
            let d = odometer(t);
            assert!(d >= prev && d - prev < 0.05, "jump at {t}");
            if tl.at(t).index != 0 && tl.at(t - 0.01).index != 0 {
                assert_eq!(d, prev, "moved outside drive at {t}");
            }
            prev = d;
        }
        assert!(odometer(tl.total()) > 10.0);
    }
}
