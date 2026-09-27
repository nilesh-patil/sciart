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
//! **Story (MR-A3).** A looping [`Timeline`] sol: drive → deploy arm → drill → stow →
//! transmit. Rover position, arm pose and captions are pure functions of the sim time,
//! so resizes never desync the story; only the dust, the dust devils and the seeds are
//! state, drawn from `ctx.rng`.
//!
//! **Sky.** A 90 s sol with a real-ish solar geometry at Jezero's latitude. Martian dust
//! forward-scatters blue light, so around the low sun the sky turns blue while the rest
//! stays butterscotch; Phobos, orbiting faster than Mars spins, rises in the west.

use std::f64::consts::{FRAC_PI_2, PI};

use rand::Rng;
use sciart_core::geom::{self, ASPECT};
use sciart_core::particles::{gaussian, plot};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{draw, rng};

pub static META: Meta = Meta {
    name: "mars_rover",
    category: Category::SpaceExploration,
    description: "A rocker-bogie rover crossing Martian dunes, drilling and phoning home",
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
    let (s, c) = a.sin_cos();
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

    /// All six hubs: near side (front, middle, rear) then far side, which on a 1-D
    /// profile coincides with the near side.
    pub(crate) fn wheel_centers(&self) -> [P2; 6] {
        let j = self.joints();
        [j.front, j.middle, j.rear, j.front, j.middle, j.rear]
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

const DRIVE: f64 = 18.0;
const DEPLOY: f64 = 3.0;
const DRILL: f64 = 4.0;
const STOW: f64 = 2.0;
const TRANSMIT: f64 = 4.0;
const PLAN: [(&str, f64); 5] = [
    ("drive", DRIVE),
    ("deploy arm", DEPLOY),
    ("drill", DRILL),
    ("stow", STOW),
    ("transmit", TRANSMIT),
];
const CYCLE: f64 = DRIVE + DEPLOY + DRILL + STOW + TRANSMIT;
const PH_DRIVE: usize = 0;
const PH_DEPLOY: usize = 1;
const PH_DRILL: usize = 2;
const PH_STOW: usize = 3;
const PH_TRANSMIT: usize = 4;

/// Cruise speed (units/s ≈ 0.8 m/s): Perseverance's 4 cm/s, time-lapsed ×20 for the screen.
const CRUISE: f64 = 1.6;
/// Time to reach cruise speed (and to stop) — a trapezoidal speed profile.
const RAMP: f64 = 1.5;
/// Distance covered in one drive phase.
const DRIVE_DIST: f64 = CRUISE * (DRIVE - RAMP);

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
/// Deck box height, mast top, camera-head box and the high-gain antenna's dish centre.
const BODY_V: (f64, f64) = (0.3, 1.8);
const MAST_TOP: f64 = 3.8;
const HEAD_U: (f64, f64) = (1.15, 2.35);
const HEAD_V: (f64, f64) = (3.8, 4.4);
const HGA: P2 = (-0.9, 2.4);
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
    let elbow_ik = add(s, (ARM_L1 * ik1.cos(), ARM_L1 * ik1.sin()));
    let t2 = sub(target, elbow_ik);
    let ik2 = t2.1.atan2(t2.0);
    let a1 = lerp_angle(STOW_A1 + pose.phi, ik1, k);
    let a2 = lerp_angle(STOW_A2 + pose.phi, ik2, k);
    let e = add(s, (ARM_L1 * a1.cos(), ARM_L1 * a1.sin()));
    let t = add(e, (ARM_L2 * a2.cos(), ARM_L2 * a2.sin()));
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
/// Jezero crater latitude and a southern-summer solar declination.
const LATITUDE: f64 = 18.4 * PI / 180.0;
const DECLINATION: f64 = -12.0 * PI / 180.0;
/// Hour angle (deg) vs sol time (s): day at an easy pace, a lingering sunset, a quick
/// night. Starts at 09:00 LMST; 315° ≡ −45° closes the loop.
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
    (LATITUDE.sin() * DECLINATION.sin() + LATITUDE.cos() * DECLINATION.cos() * h.cos())
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

/// Blue forward-scattered glow around a low sun: on between ~25° and −16° elevation.
fn glow_strength(e_deg: f64) -> f64 {
    smooth(25.0, 6.0, e_deg) * smooth(-16.0, -5.0, e_deg)
}

fn daylight(e_deg: f64) -> f64 {
    smooth(-6.0, 8.0, e_deg)
}

fn night(e_deg: f64) -> f64 {
    smooth(-4.0, -14.0, e_deg)
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

// Solid fills carry the picture in colour; each has its own ASCII fallback so ASCII
// mode stays legible (the sky falls back to blank, not to a wall of `#`).
const SKY: [Glyph; 4] = [
    Glyph::new('█', ' '),
    Glyph::new('▓', ' '),
    Glyph::new('▒', ' '),
    Glyph::new('░', ' '),
];
const GLOW: [Glyph; 4] = [
    Glyph::new('█', '*'),
    Glyph::new('▓', ':'),
    Glyph::new('▒', '.'),
    Glyph::new('░', '.'),
];
const SUN: Glyph = Glyph::new('█', '@');
const FAR_FILL: Glyph = Glyph::new('█', ':');
const MID_FILL: Glyph = Glyph::new('█', '+');
const GROUND: Glyph = Glyph::new('█', '#');
const PEBBLE: Glyph = Glyph::new('▓', '%');
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

/// Hash of a screen cell, `[0, 1)`: a stable dither pattern.
#[inline]
fn cell_hash(x: i32, y: i32, salt: u64) -> f64 {
    rng::hash01(salt ^ rng::cell_key(x, y))
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
    /// Column of the rover's body pivot.
    rover_col: f64,
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
            rover_col: 0.4 * w,
        }
    }

    /// World units per column.
    fn dx(&self) -> f64 {
        1.0 / (self.s * ASPECT)
    }
}

/// Where the near-layer camera is: world x of column 0.
fn camera(l: &Layout, rover_x: f64) -> f64 {
    rover_x - l.rover_col * l.dx()
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
            let cam_mid = camera(&l, self.rover_x(w)) * PARALLAX_MID;
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
    }

    fn render(&self, canvas: &mut Canvas) {
        let Some(w) = self.world else { return };
        let l = self.layout;
        let rover_x = self.rover_x(&w);
        let cam = camera(&l, rover_x);
        let view = View {
            cam,
            k: l.s * ASPECT,
            s: l.s,
            base: l.near_base,
        };
        let beat = self.plan.at(self.t);
        let hour = hour_angle(self.t);
        let e = sun_elevation(hour);
        let sun = (
            l.w * (0.5 + 0.3 * hour.sin()),
            l.horizon - (l.horizon - 1.0) * e.sin() / max_elevation().sin(),
        );
        self.draw_sky(canvas, &w, sun, e.to_degrees());
        self.draw_bodies(canvas, &w, &beat, sun, e.to_degrees());
        self.draw_layers(canvas, &w, cam, daylight(e.to_degrees()));
        self.draw_drill_marks(canvas, &w, &view, &beat);
        let pose = solve_pose(|x| w.near.eval(x), rover_x);
        self.draw_dust(canvas, &view);
        self.draw_rover(canvas, &w, &view, &pose, &beat);
        if beat.index == PH_TRANSMIT {
            self.draw_signal(canvas, &view, &pose, &beat);
        }
        self.draw_captions(canvas, &w, &beat, hour);
    }
}

// ---------------------------------------------------------------------------------------
// Rendering.
// ---------------------------------------------------------------------------------------

impl MarsRover {
    /// Butterscotch dust-laden sky by day (brighter towards the horizon), a blue glow
    /// of forward-scattered light around a low sun, a rose dusk band, stars at night.
    ///
    /// The sky is solid block glyphs whose coverage fades with the light, so it reads
    /// as a sky colour rather than as texture; a per-cell hash dithers the fade.
    fn draw_sky(&self, canvas: &mut Canvas, w: &World, sun: P2, e_deg: f64) {
        let l = self.layout;
        let glow = glow_strength(e_deg);
        let day = daylight(e_deg);
        let dark = night(e_deg);
        let r_glow = (0.35 * l.horizon).max(2.0);
        let rows = l.far_base.ceil().clamp(0.0, l.h) as i32;
        let cols = l.w as i32;
        for y in 0..rows {
            let fy = f64::from(y);
            let a = (fy / l.horizon.max(1.0)).clamp(0.0, 1.0);
            for x in 0..cols {
                let n = cell_hash(x, y, w.salt) - 0.5;
                let (dx, dy) = ((f64::from(x) - sun.0) / ASPECT, fy - sun.1);
                let g = glow * (-(dx.hypot(dy)) / r_glow).exp();
                let band = glow * smooth(0.75, 1.0, a) * (1.0 - g).powi(3);
                // The zenith darkens first; the dusty horizon holds the light longest.
                let cover = (1.6 * day * (0.55 + 0.45 * a)).max(g) + 0.015 * n;
                let aureole = day * (-(dx.hypot(dy)) / (1.6 * l.s)).exp();
                let shade = match cover {
                    c if c > 0.8 => 0,
                    c if c > 0.55 => 1,
                    c if c > 0.3 => 2,
                    c if c > 0.1 => 3,
                    _ => {
                        self.draw_star(canvas, w, x, y, dark);
                        continue;
                    }
                };
                let blue = g > 0.6 * day + 0.08 * n;
                let glyph = if blue { GLOW[shade] } else { SKY[shade] };
                let ink = if blue {
                    Ink::new(Role::Cold, (40.0 + 215.0 * g) as u8)
                } else if band > 0.3 + 0.1 * n {
                    Ink::new(Role::Accent, (10.0 + 40.0 * band) as u8)
                } else {
                    let lvl = day * (40.0 + 70.0 * a * a) + 120.0 * aureole;
                    Ink::new(Role::Sun, lvl.min(255.0) as u8)
                };
                canvas.put(x, y, glyph, ink);
            }
        }
    }

    fn draw_star(&self, canvas: &mut Canvas, w: &World, x: i32, y: i32, dark: f64) {
        if dark <= 0.0 {
            return;
        }
        let s = cell_hash(x, y, w.salt ^ 0x57a2);
        if s < 0.02 * dark {
            let tw = (self.t * 2.3 + 2000.0 * s).sin();
            let glyph = if s < 0.003 {
                Glyph::new('✦', '*')
            } else {
                DOT
            };
            let lvl = 90.0 + 140.0 * dark * (0.75 + 0.25 * tw);
            canvas.put(x, y, glyph, Ink::new(Role::Star, lvl as u8));
        }
    }

    /// Sun, Phobos and the relay orbiter.
    fn draw_bodies(&self, canvas: &mut Canvas, w: &World, beat: &Beat, sun: P2, e_deg: f64) {
        let l = self.layout;
        if e_deg > -4.0 {
            let lvl = (170.0 + 85.0 * daylight(e_deg)) as u8;
            draw::disc(canvas, sun.0, sun.1, 0.6 * l.s, |d, _, _| {
                Some(if d < 0.6 {
                    (SUN, Ink::new(Role::Sun, lvl))
                } else {
                    (SUN, Ink::new(Role::Sun, lvl.saturating_sub(40)))
                })
            });
        }
        // Phobos: rises in the west (right), sets in the east (left).
        let p = geom::wrap(self.t + w.phobos_phase, PHOBOS_PERIOD);
        if p < PHOBOS_UP {
            let u = p / PHOBOS_UP;
            let x = l.w * (0.95 - 0.9 * u);
            let y = l.horizon - 0.55 * l.horizon * (PI * u).sin();
            let lvl = 150.0 + 100.0 * night(e_deg);
            plot(canvas, x, y, PIXEL, Ink::new(Role::Structure, lvl as u8));
        }
        // Relay orbiter crossing during stow and transmit.
        if let Some((ox, oy)) = self.orbiter(beat) {
            let blink = beat.index == PH_TRANSMIT && (self.t * 6.0).sin() > 0.0;
            let ink = Ink::new(Role::Accent, if blink { 255 } else { 190 });
            plot(canvas, ox, oy, PIXEL, ink);
            if l.s >= TINY_SCALE {
                // Solar panels.
                let panel = Ink::new(Role::Structure, 170);
                plot(canvas, ox - 1.0, oy, WING, panel);
                plot(canvas, ox + 1.0, oy, WING, panel);
            }
        }
    }

    fn orbiter(&self, beat: &Beat) -> Option<P2> {
        let u = match beat.index {
            PH_STOW => beat.local / (STOW + TRANSMIT),
            PH_TRANSMIT => (STOW + beat.local) / (STOW + TRANSMIT),
            _ => return None,
        };
        let l = self.layout;
        Some((
            l.w * (0.08 + 0.84 * u),
            (0.16 * l.horizon).max(1.0) + 0.4 * l.s * (PI * u).sin(),
        ))
    }

    /// Far ridges, mid plain and near ground, back to front: solid fills whose ink
    /// level carries the depth cue, with half-block crests for sub-row relief. The
    /// dust devils stand on the mid plain, so they go between mid and near.
    fn draw_layers(&self, canvas: &mut Canvas, w: &World, cam: f64, light: f64) {
        let l = self.layout;
        let dxc = l.dx();
        let h = l.h as i32;
        let lit = |v: f64| (v * (0.45 + 0.55 * light)) as u8;
        for c in 0..l.w as i32 {
            let fc = f64::from(c) + 0.5;
            let far = l.far_base - l.s * w.far.height(cam * PARALLAX_FAR + fc * dxc);
            let mid = l.mid_base - l.s * w.mid.height(cam * PARALLAX_MID + fc * dxc);
            fill_column(canvas, c, far, h, |depth, _| {
                (
                    FAR_FILL,
                    Ink::new(Role::Dim, lit(if depth == 0 { 120.0 } else { 85.0 })),
                )
            });
            fill_column(canvas, c, mid, h, |depth, _| {
                (
                    MID_FILL,
                    Ink::new(Role::Hot, lit(if depth == 0 { 120.0 } else { 90.0 })),
                )
            });
        }
        self.draw_devils(canvas, w, cam);
        for c in 0..l.w as i32 {
            let xn = cam + (f64::from(c) + 0.5) * dxc;
            let near = l.near_base - l.s * w.near.height(xn);
            // Speckle hashed on world columns so the texture scrolls with the ground.
            let wc = (xn / dxc).floor() as i64 as u64;
            fill_column(canvas, c, near, h, |depth, row| {
                let n = rng::hash01(w.salt ^ wc.wrapping_mul(0x9e37_79b9) ^ row as u64);
                match depth {
                    0 => (GROUND, Ink::new(Role::Hot, lit(240.0))),
                    _ if n < 0.08 => (PEBBLE, Ink::new(Role::Hot, lit(215.0))),
                    1 => (GROUND, Ink::new(Role::Hot, lit(205.0))),
                    2 => (GROUND, Ink::new(Role::Hot, lit(185.0))),
                    _ => (GROUND, Ink::new(Role::Hot, lit(165.0))),
                }
            });
        }
    }

    /// A dust devil: a tapering, swirling column of dust leaning with the wind. It
    /// veils what is behind it rather than punching glyph-shaped holes in it.
    fn draw_devils(&self, canvas: &mut Canvas, w: &World, cam: f64) {
        let l = self.layout;
        let k = l.s * ASPECT;
        for dv in self.devils.iter().flatten() {
            let alpha = smooth(0.0, 2.0, dv.age) * smooth(dv.life, dv.life - 2.5, dv.age);
            let base_col = (dv.x - cam * PARALLAX_MID) * k;
            let base_row = l.mid_base - l.s * w.mid.height(dv.x);
            let rows = (dv.height * l.s).round().max(1.0) as i32;
            for j in 0..rows {
                let f = f64::from(j) / f64::from(rows);
                let y = base_row - f64::from(j) - 0.5;
                let lean = 0.8 * f * f * f64::from(rows);
                let sway = (2.0 * f - 2.5 * self.t + dv.phase).sin() * (0.3 + 0.8 * f);
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
                    let band = ((q.asin() + 4.0 * self.t - 1.7 * f64::from(j)).sin() + 1.0) * 0.5;
                    let v = a * (0.35 + 0.65 * band) * (1.0 - q * q);
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
            let fallback = (g, Ink::new(Role::Particle, (200.0 - 130.0 * f) as u8));
            veil(canvas, x, y, (60.0 * (1.0 - f)) as u8, fallback);
        }
    }

    fn draw_rover(&self, canvas: &mut Canvas, w: &World, view: &View, pose: &Pose, beat: &Beat) {
        let j = pose.joints();
        let hubs = [j.front, j.middle, j.rear];
        let body_ink = Ink::new(Role::Text, 235);
        let spin = odometer(self.t) / WHEEL_R;
        let wheel = WHEEL[((-spin / FRAC_PI_2).floor() as i64).rem_euclid(4) as usize];
        if view.s < TINY_SCALE {
            // Tiny: a deck on three wheels and a mast stub.
            let (a, b) = (view.at(pose.body(-2.2, 0.9)), view.at(pose.body(2.2, 0.9)));
            segment(canvas, a, b, HULL, body_ink);
            for hub in hubs {
                let (x, y) = view.at(hub);
                plot(canvas, x, y, wheel, Ink::new(Role::Structure, 230));
            }
            let m = view.at(pose.body(1.7, 2.6));
            plot(
                canvas,
                m.0,
                m.1,
                Glyph::new('╻', '|'),
                Ink::new(Role::Structure, 200),
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
        let link = Ink::new(Role::Structure, 150);
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
            let rtg = Ink::new(Role::Structure, 105);
            strut(
                canvas,
                pose.body(-2.2, 0.7 + dv),
                pose.body(-3.1, 1.5 + dv),
                rtg,
            );
        }
        // Body: a tilted box with half-block edges, then mast and camera head.
        fill_box(canvas, view, pose, (-2.2, 2.2), BODY_V, body_ink);
        let belly = Ink::new(Role::Structure, 200);
        fill_box(
            canvas,
            view,
            pose,
            (-2.2, 2.2),
            (BODY_V.0, BODY_V.0 + 0.45),
            belly,
        );
        let mast = Ink::new(Role::Structure, 185);
        strut(
            canvas,
            pose.body(1.7, BODY_V.1),
            pose.body(1.7, MAST_TOP),
            mast,
        );
        let head = Ink::new(Role::Text, 250);
        fill_box(canvas, view, pose, HEAD_U, HEAD_V, head);
        let eye = view.at(pose.body(HEAD_U.1, 0.5 * (HEAD_V.0 + HEAD_V.1)));
        plot(
            canvas,
            eye.0,
            eye.1,
            Glyph::new('◘', 'o'),
            Ink::new(Role::Accent, 200),
        );
        // High-gain antenna: a dish on a post, aimed at the orbiter while transmitting.
        let post = pose.body(HGA.0, HGA.1);
        strut(canvas, pose.body(HGA.0, BODY_V.1), post, link);
        let aim = match self.orbiter(beat) {
            Some(o) if beat.index == PH_TRANSMIT => {
                let hp = view.at(post);
                // Angle of the dish axis from the vertical, in world terms.
                ((o.0 - hp.0) / ASPECT).atan2(hp.1 - o.1)
            }
            _ => pose.phi - 0.5,
        };
        let (sa, ca) = aim.sin_cos();
        let half = 0.6;
        let dish = Ink::new(Role::Structure, 225);
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
        let arm_ink = Ink::new(Role::Structure, 205);
        strut(canvas, s, e, arm_ink);
        strut(canvas, e, t, arm_ink);
        let (se, st) = (view.at(e), view.at(t));
        plot(canvas, se.0, se.1, JOINT, Ink::new(Role::Structure, 240));
        plot(
            canvas,
            st.0,
            st.1,
            Glyph::new('▣', '#'),
            Ink::new(Role::Text, 240),
        );
        if beat.index == PH_DRILL {
            let bit = if (self.t * 24.0).sin() > 0.0 {
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
                Some((TYRE, Ink::new(Role::Structure, 70)))
            });
        }
        plot(canvas, x, y, glyph, Ink::new(Role::Structure, 245));
    }

    fn draw_signal(&self, canvas: &mut Canvas, view: &View, pose: &Pose, beat: &Beat) {
        let Some(o) = self.orbiter(beat) else { return };
        let a = view.at(pose.body(HGA.0, HGA.1));
        const PACKETS: usize = 5;
        for i in 0..PACKETS {
            let q = (i as f64 / PACKETS as f64 + self.t * 0.9).fract();
            let (x, y) = (ease::lerp(a.0, o.0, q), ease::lerp(a.1, o.1, q));
            plot(canvas, x, y, PIXEL, Ink::new(Role::Wave, 240));
        }
        // Faint beam.
        let n = ((o.0 - a.0).abs().max((o.1 - a.1).abs()) / 3.0).min(200.0) as usize;
        for i in 1..n {
            let q = i as f64 / n as f64;
            let (x, y) = (ease::lerp(a.0, o.0, q), ease::lerp(a.1, o.1, q));
            if canvas
                .get(x.round() as i32, y.round() as i32)
                .is_some_and(|c| c.ch == ' ')
            {
                plot(canvas, x, y, DOT, Ink::new(Role::Wave, 110));
            }
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
        let mut right = LineBuf::default();
        let _ = match beat.index {
            PH_DRIVE => write!(right, "DRIVE {:.1} m", drive_profile(beat.local).0 * UNIT_M),
            PH_DEPLOY => write!(right, "ARM DEPLOY"),
            PH_DRILL => write!(right, "CORING SAMPLE #{}", cycle + 1),
            PH_STOW => write!(right, "ARM STOW"),
            _ => write!(right, "UHF RELAY > ORBITER"),
        };
        let mut left = LineBuf::default();
        let lmst = (12.0 + hour.to_degrees() / 15.0).rem_euclid(24.0);
        // The sol number ticks over at local midnight.
        let midnight = SOL_KNOTS[2].0
            + (SOL_KNOTS[3].0 - SOL_KNOTS[2].0) * (180.0 - SOL_KNOTS[2].1)
                / (SOL_KNOTS[3].1 - SOL_KNOTS[2].1);
        let sol = i64::from(w.sol0) + ((self.t - midnight) / SOL).floor() as i64 + 1;
        let _ = write!(
            left,
            "SOL {sol}  LMST {:02}:{:02}",
            lmst.floor() as u32,
            (lmst.fract() * 60.0).floor() as u32
        );
        let ink = Ink::new(Role::Text, 170);
        let (ll, rl) = (left.len, right.len);
        if rl + 2 <= width {
            canvas.text((width - rl - 1) as i32, 0, right.as_str(), ink);
        }
        if ll + rl + 5 <= width {
            canvas.text(1, 0, left.as_str(), ink);
        }
    }
}

/// Glyphs that cover their cell: dust in front of them lightens them instead of
/// replacing them, since a glyph cell has no background to blend with.
const COVER: [Glyph; 13] = [
    GROUND, PEBBLE, MID_FILL, FAR_FILL, HULL, SKY[0], SKY[1], SKY[2], SKY[3], GLOW[0], GLOW[1],
    GLOW[2], GLOW[3],
];

/// Semi-transparent overlay: brighten a covering glyph already in the cell by `boost`,
/// or draw `fallback` over empty or line-art cells.
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
    let ascii = canvas.mode() == GlyphMode::Ascii;
    let under = COVER.iter().find(|g| {
        if ascii {
            g.ascii == cell.ch && !matches!(g.ascii, ' ' | '.')
        } else {
            g.unicode == cell.ch
        }
    });
    match under {
        Some(&g) => {
            let ink = Ink::new(cell.ink.role, cell.ink.level.saturating_add(boost));
            canvas.put(xi, yi, g, ink);
        }
        None => canvas.put(xi, yi, fallback.0, fallback.1),
    }
}

/// Fill column `c` from the surface at fractional row `top` down to the bottom;
/// `shade(depth, row)` picks each cell's glyph and ink.
fn fill_column(
    canvas: &mut Canvas,
    c: i32,
    top: f64,
    h: i32,
    mut shade: impl FnMut(i32, i32) -> (Glyph, Ink),
) {
    if !top.is_finite() {
        return;
    }
    let first = top.round().clamp(-1.0, f64::from(h)) as i32;
    for row in first.max(0)..h {
        let (g, ink) = shade(row - first, row);
        canvas.put(c, row, g, ink);
    }
}

/// Fill the body-frame box `u ∈ [u0, u1]`, `v ∈ [v0, v1]` column by column: a cell
/// is covered when its centre lies between the box's bottom and top edges.
///
/// Whole cells only: a half-block leaves its other half at the terminal background,
/// which shows as a black seam against the solid sky and ground.
fn fill_box(canvas: &mut Canvas, view: &View, pose: &Pose, u: (f64, f64), v: (f64, f64), ink: Ink) {
    let (sn, cs) = pose.phi.sin_cos();
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

/// A fixed-capacity text buffer, so `render` can format numbers without allocating.
struct LineBuf {
    bytes: [u8; 48],
    len: usize,
}

impl Default for LineBuf {
    fn default() -> Self {
        Self {
            bytes: [0; 48],
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
        let w = g.world.unwrap();
        let l = g.layout;
        for _ in 0..(CYCLE / 0.25) as usize {
            g.update(0.25, &mut ctx);
            let rx = g.rover_x(&w);
            let pose = solve_pose(|x| w.near.eval(x), rx);
            let cam = camera(&l, rx);
            for (hx, hy) in pose.wheel_centers() {
                let contact_row = l.near_base - l.s * (hy - WHEEL_R);
                let ground_row = l.near_base - l.s * w.near.height(hx);
                assert!((contact_row - ground_row).abs() <= 0.5);
                assert!((0.0..l.w).contains(&((hx - cam) * l.s * ASPECT)));
            }
        }
    }

    #[test]
    fn sunset_glows_blue_around_the_sun_but_noon_does_not() {
        let size = Size::new(120, 40);
        let cold_near_sun = |t: f64| {
            let mut s = Stage::new(build(), 5, size, GlyphMode::Unicode);
            while s.ctx().time() < t {
                s.step(0.25);
            }
            s.render()
                .cells()
                .iter()
                .filter(|c| c.ink.role == Role::Cold)
                .count()
        };
        // Sun elevation: ~2° at t = 47 s (just before sunset), ~57° near t = 12 s.
        assert!(sun_elevation(hour_angle(47.0)).to_degrees() < 5.0);
        assert!(sun_elevation(hour_angle(12.0)).to_degrees() > 50.0);
        assert!(cold_near_sun(47.0) > 60, "no blue glow at sunset");
        assert_eq!(cold_near_sun(12.0), 0, "blue glow at noon");
    }

    #[test]
    fn dust_rises_while_driving_and_drilling_and_settles_otherwise() {
        let size = Size::new(160, 48);
        let mut ctx = Ctx::new(9, size);
        let mut g = MarsRover::default();
        g.resize(size, &mut ctx);
        let run_to = |g: &mut MarsRover, ctx: &mut Ctx, t: f64| {
            while g.t < t {
                g.update(SIM_DT, ctx);
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
        let before = g.world.expect("seeded in first resize");
        for (w, h) in [(200, 50), (1, 1), (37, 11)] {
            for _ in 0..30 {
                g.update(SIM_DT, &mut ctx);
            }
            g.resize(Size::new(w, h), &mut ctx);
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
        let want = [DRIVE, DEPLOY, DRILL, STOW, TRANSMIT];
        assert!((tl.total() - want.iter().sum::<f64>()).abs() < 1e-12);
        let h = 1e-3;
        let (mut last, mut since, mut seen) = (tl.at(0.0).index, 0.0, Vec::new());
        let mut t = 0.0;
        while t < 2.0 * tl.total() {
            t += h;
            let i = tl.at(t).index;
            since += h;
            if i != last {
                assert_eq!(i, (last + 1) % 5, "phase order");
                seen.push((last, since));
                last = i;
                since = 0.0;
            }
        }
        assert!(seen.len() >= 9);
        for &(i, d) in &seen[..9] {
            assert!((d - want[i]).abs() < 2.0 * h, "phase {i} lasted {d}");
        }
    }

    #[test]
    fn rover_moves_only_while_driving_and_continuously() {
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
