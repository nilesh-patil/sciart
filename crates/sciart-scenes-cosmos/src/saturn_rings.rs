//! Saturn and its rings at a 20° tilt (SPEC-002 `saturn_rings`).
//!
//! The rings are an optical-depth profile `τ(r)` built once from published radii (C, B
//! and A rings, the Cassini Division, the Encke and Keeler gaps, the Huygens ringlet),
//! seen through `1 − e^{−τ/μ}`. Every ring pixel is inverted to ring-plane `(r, λ)`; its
//! clumpy texture is value noise in `(r, λ − Ω(r)·t)` with the Keplerian `Ω ∝ r^{−3/2}`,
//! so the rings' clumps shear honestly. Pan, in the Encke gap, raises edge waves and a
//! wake of density ripples that decay downstream; Mimas orbits outside the rings (its 2:1
//! resonance holds the Cassini Division's inner edge); spokes corotate with the planet's
//! magnetic field across the B ring; the globe (oblate, softly banded, limb-darkened, lit
//! from the side) hides the far rings and throws its shadow on them, and the rings shadow
//! the globe in turn. The globe is shaded once per resize; the rings are re-shaded every
//! frame in `update`.
//!
//! Units: lengths in Saturn equatorial radii (60 268 km), time in simulated seconds with
//! the ring motion time-lapsed ×2000.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::wrap_angle;
use sciart_core::prelude::*;

use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "saturn_rings",
    category: Category::SunPlanets,
    description: "Saturn's tilted rings: Cassini Division, Encke and Keeler gaps, spokes, shadow",
    title: "Saturn rings · moons carve gaps",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SaturnRings::new())
}

/// Ring opening angle seen from the viewer, and the Sun's elevation and azimuth.
const TILT: f64 = 20.0 * PI / 180.0;
const SUN_ELEV: f64 = 14.0 * PI / 180.0;
const SUN_AZ: f64 = 38.0 * PI / 180.0;
/// Polar radius of the globe (flattening ≈ 0.1).
const POLAR: f64 = 0.9;
/// Saturn's rotation (10.66 h) time-lapsed ×2000, rad/s; the radius where the Keplerian
/// rate equals it (synchronous orbit, 1.86 R_S).
pub(crate) const OMEGA_SATURN: f64 = TAU / (10.66 * 3600.0) * 2000.0;
const R_SYNC: f64 = 1.86;

/// The profile LUT spans `[R_IN, R_OUT)`.
const R_IN: f64 = 1.20;
const R_OUT: f64 = 2.34;
const LUT: usize = 2048;

/// Gap radii (the gap widths are exaggerated so they show at terminal resolution; the
/// Cassini Division is widened outward by ~15 %).
pub(crate) const CASSINI: (f64, f64) = (1.951, 2.04);
pub(crate) const ENCKE: (f64, f64) = (2.203, 2.224);
pub(crate) const KEELER: (f64, f64) = (2.247, 2.259);
const A_OUTER: f64 = 2.2695;
/// Pan's orbit, mid-Encke gap (R_S).
pub(crate) const PAN_R: f64 = 0.5 * (ENCKE.0 + ENCKE.1);
/// Pan's edge waves: amplitude (R_S, exaggerated), decay length and wavenumber in
/// radians downstream of the moon.
const WAVE_AMP: f64 = 0.012;
const WAVE_DECAY: f64 = 1.0;
const WAVE_K: f64 = 16.0;
/// Pan's wake: radial reach either side of the gap (R_S), its brightness contrast and
/// its decay (rad downstream). Exaggerated so a terminal shows it.
const WAKE_REACH: f64 = 0.09;
const WAKE_AMP: f64 = 0.55;
const WAKE_DECAY: f64 = 1.4;
/// Mimas: orbital radius (R_S, 185 540 km) and drawn radius (R_S; ×10 so it shows).
pub(crate) const MIMAS_R: f64 = 3.078;
const MIMAS_SIZE: f64 = 0.033;
/// Clumps: azimuthal lattice cells per turn and radial cells per R_S of the ring
/// texture, and its contrast. Coarse in radius so the rings do not read as grain.
const CLUMP_AZ: i32 = 20;
const CLUMP_R: f64 = 9.0;
const CLUMP_AMP: f64 = 0.5;
const CLUMP_LIFE: f64 = 10.0;
/// Spokes alive at most, their radial band and life (s).
const SPOKES: usize = 10;
const SPOKE_BAND: (f64, f64) = (1.58, 1.92);
const SPOKE_LIFE: (f64, f64) = (6.0, 10.0);
/// Deepest spoke darkening (exaggerated so they read).
const SPOKE_DIM: f64 = 0.5;
/// Share of the frame the ring system spans (dark ground around it).
const FILL: f64 = 0.74;

/// Ringlets (SPEC-002 Look, M16): one braille thread every `RINGLET_GAP` dots of ring
/// radius at the ansae; where the tilt crowds them on screen (near the minor axis) only
/// every m-th is kept, so neighbouring threads stay at least `MIN_GAP` dots apart. A
/// ringlet within `GAP_CLEAR` dots of a gap (Cassini, Encke, Keeler) is left out, so
/// every gap is a black lane at least a dot wider than itself.
const RINGLET_GAP: f64 = 4.0;
/// On a small frame (fewer than `SMALL_SCALE` dots per Saturn radius) the ringlets are
/// `RINGLET_GAP_SMALL` dots apart, so the rings keep a few threads each.
const RINGLET_GAP_SMALL: f64 = 3.0;
const SMALL_SCALE: f64 = 45.0;

/// Ringlet spacing (dots of ring radius at the ansae) for `scale` dots per Saturn radius.
fn ringlet_gap(scale: f64) -> f64 {
    if scale < SMALL_SCALE {
        RINGLET_GAP_SMALL
    } else {
        RINGLET_GAP
    }
}
const MIN_GAP: f64 = 2.5;
const GAP_CLEAR: f64 = 0.75;

/// Whether a ringlet at radius `r` lies within `clear` (R_S) of a gap.
pub(crate) fn near_gap(r: f64, clear: f64) -> bool {
    [CASSINI, ENCKE, KEELER]
        .iter()
        .any(|&(a, b)| (r - r.clamp(a, b)).abs() < clear)
}

/// The rings' flat steps by brightness, brightest first: the B ring's bright white, the
/// grey A and C rings; below the last a ringlet dot is dark (the Cassini Division and
/// the gaps stay black).
pub(crate) const RING_STEPS: [(f64, Ink); 4] = [
    (0.72, Ink::new(Role::Star, 230)),
    (0.5, Ink::new(Role::Structure, 200)),
    (0.3, Ink::new(Role::Structure, 120)),
    (0.14, Ink::new(Role::Structure, 50)),
];
/// A ringlet dot inside a spoke: a faint grey streak.
pub(crate) const SPOKE_INK: Ink = Ink::new(Role::Dim, 150);
/// The globe's threads: the lit limb, zones and belts (sine-latitude lines), each one
/// step dimmer towards the terminator.
pub(crate) const LIMB_INKS: [Ink; 2] = [Ink::new(Role::Sun, 230), Ink::new(Role::Sun, 120)];
pub(crate) const NIGHT_LIMB: Ink = Ink::new(Role::Dim, 90);
pub(crate) const ZONE_INKS: [Ink; 2] = [Ink::new(Role::Sun, 200), Ink::new(Role::Sun, 100)];
pub(crate) const BELT_INKS: [Ink; 2] = [Ink::new(Role::Hot, 170), Ink::new(Role::Hot, 80)];
/// Planetographic latitudes (rad) of the globe's band lines.
const BAND_LATS: [f64; 10] = [
    -0.96, -0.68, -0.52, -0.33, -0.17, 0.17, 0.33, 0.52, 0.68, 0.96,
];
/// Moons: Pan a white dot, Mimas a small white disc (dots).
const MOON_INK: Ink = Ink::new(Role::Star, 245);

/// The first step of `steps` that brightness `v` reaches.
fn step(v: f64, steps: &[(f64, Ink)]) -> Option<Ink> {
    steps.iter().find(|(from, _)| v >= *from).map(|s| s.1)
}

/// A ringlet dot's ink at brightness `v`, or `None` (dark) below the last step.
pub(crate) fn ring_ink(v: f64) -> Option<Ink> {
    step(v, &RING_STEPS)
}

/// Background stars: a share of the cells, one dot each.
const STAR_SHARE: f64 = 0.004;

/// Keplerian angular rate at `r`, rad/s (time-lapsed): `Ω ∝ r^{−3/2}` (RING-A2).
pub(crate) fn kepler_rate(r: f64) -> f64 {
    // (R_SYNC / r)^{3/2} without `powf`: this runs for every ring pixel.
    let q = R_SYNC / r;
    OMEGA_SATURN * q * q.sqrt()
}

/// Optical depth at ring radius `r` (RING-A1), with the fine ringlet structure; 0 off
/// the rings.
pub(crate) fn tau(r: f64) -> f64 {
    let ringlets = 0.75 + 0.5 * value_noise(0x51AC, r * 60.0, 0.5, 1.0);
    let base = match r {
        r if r < 1.239 => 0.0,
        // C ring: faint, with a few denser plateaus.
        r if r < 1.527 => {
            if (1.43..1.47).contains(&r) || (1.35..1.37).contains(&r) {
                0.35
            } else {
                0.09
            }
        }
        // B ring: inner B, the opaque core, the outer edge.
        r if r < 1.64 => 1.1,
        r if r < 1.90 => 2.8 + 1.4 * ((r - 1.64) * 40.0).sin_det().abs(),
        r if r < CASSINI.0 => 1.8,
        // Cassini Division: the Huygens gap, its ringlet, then sparse material.
        r if r < CASSINI.1 => {
            if (1.9565..1.9625).contains(&r) {
                0.6
            } else if r < 1.966 {
                0.0
            } else {
                0.05
            }
        }
        r if r < ENCKE.0 => 0.75 - 0.9 * (r - CASSINI.1),
        r if r < ENCKE.1 => 0.0,
        r if r < KEELER.0 => 0.55,
        r if r < KEELER.1 => 0.0,
        r if r < A_OUTER => 0.5,
        _ => 0.0,
    };
    base * ringlets
}

/// Relative albedo: the C ring and Cassini material are darker, dirtier ice.
fn albedo(r: f64) -> f64 {
    if r < 1.527 {
        0.42
    } else if r < CASSINI.0 {
        1.0
    } else if r < CASSINI.1 {
        0.5
    } else {
        0.82
    }
}

/// Clump texture multiplier at ring radius `r`, angle `λ` and time `t` (RING-A2, A9).
/// Clumps (self-gravity wakes) are value noise that moves at the Keplerian rate, so the
/// inner rings overtake the outer; like real wakes they dissolve and re-form, here over
/// `CLUMP_LIFE` s, two staggered generations cross-faded by `sin²`, so the shear never
/// winds them into radial grain. Coarse in radius: radially adjacent pixels vary smoothly
/// (the fine radial structure is the τ profile's).
#[cfg(test)]
pub(crate) fn clump(r: f64, lambda: f64, t: f64) -> f64 {
    let mut layers = [ClumpLayer::default(); 2];
    for (k, layer) in layers.iter_mut().enumerate() {
        layer.refresh(t, k);
    }
    clump_at(&layers, r, lambda)
}

/// Lattice rows of the clump noise: `r·CLUMP_R` stays below 23 on the rings (r < 2.55).
const CLUMP_ROWS: usize = 24;
const CLUMP_CELLS: usize = CLUMP_AZ as usize * CLUMP_ROWS;

/// One clump generation: its cross-fade weight, age (s) and value-noise lattice. The
/// lattice (`hash01` per corner, wrapping in azimuth) is rebuilt only when a new
/// generation is born, so a ring pixel costs four table reads, not six hashes.
#[derive(Debug, Clone, Copy)]
struct ClumpLayer {
    weight: f64,
    age: f64,
    salt: u64,
    lattice: [f32; CLUMP_CELLS],
}

impl Default for ClumpLayer {
    fn default() -> Self {
        Self {
            weight: 0.0,
            age: 0.0,
            // No generation hashes to this, so the first refresh builds the lattice.
            salt: u64::MAX,
            lattice: [0.0; CLUMP_CELLS],
        }
    }
}

impl ClumpLayer {
    /// Generation `layer` (0 or 1, staggered by half a life) at time `t`.
    fn refresh(&mut self, t: f64, layer: usize) {
        let shifted = t.max(0.0) + 0.5 * CLUMP_LIFE * layer as f64;
        let epoch = (shifted / CLUMP_LIFE).floor();
        self.age = shifted - epoch * CLUMP_LIFE;
        self.weight = (PI * self.age / CLUMP_LIFE).sin_det().powi(2);
        let salt = 0x7E47 ^ (epoch as u64).wrapping_mul(0x9E37_79B9) ^ ((layer as u64) << 40);
        if salt != self.salt {
            self.salt = salt;
            for (k, v) in self.lattice.iter_mut().enumerate() {
                let (i, j) = (k % CLUMP_AZ as usize, k / CLUMP_AZ as usize);
                *v = hash01(salt ^ cell_key(i as i32, j as i32)) as f32;
            }
        }
    }

    /// Smoothstep-blended value noise at azimuth `x` (lattice cells, wrapping every
    /// `CLUMP_AZ`) and radius `y` (lattice cells).
    #[inline]
    fn noise(&self, x: f64, y: f64) -> f64 {
        let n = CLUMP_AZ as usize;
        let x = x.rem_euclid(f64::from(CLUMP_AZ));
        let y = y.clamp(0.0, (CLUMP_ROWS - 1) as f64 - 1e-9);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let (i, j) = ((x0 as usize) % n, y0 as usize);
        let i1 = (i + 1) % n;
        let at = |i: usize, j: usize| f64::from(self.lattice[j * n + i]);
        let top = at(i, j) + sx * (at(i1, j) - at(i, j));
        let bottom = at(i, j + 1) + sx * (at(i1, j + 1) - at(i, j + 1));
        top + sy * (bottom - top)
    }
}

#[inline]
fn clump_at(layers: &[ClumpLayer; 2], r: f64, lambda: f64) -> f64 {
    let omega = kepler_rate(r);
    let mut sum = 0.0;
    for layer in layers {
        if layer.weight < 1e-3 {
            continue;
        }
        // Each generation is born at a random orientation, then shears.
        let phase = (lambda - omega * layer.age) / TAU;
        sum += layer.weight * layer.noise(phase * f64::from(CLUMP_AZ), r * CLUMP_R);
    }
    1.0 - 0.5 * CLUMP_AMP + CLUMP_AMP * (0.5 + 1.4 * (sum - 0.5)).clamp(0.0, 1.0)
}

/// Zonal band profile by planetographic latitude (rad), about −1..0.5: the bright
/// equatorial zone, the darker equatorial belts near ±19°, fainter temperate belts
/// near ±39° and a fine ripple between them (RING-A6).
fn saturn_bands(lat: f64) -> f64 {
    let a = lat.abs();
    let g = |c: f64, w: f64| (-((a - c) / w).powi(2)).exp();
    0.55 * g(0.0, 0.12) - g(0.33, 0.08) - 0.6 * g(0.68, 0.07) + 0.12 * (lat * 30.0).sin_det()
}

/// Pan's wake (RING-A8): brightness multiplier of ring material at radius `r` and angle
/// `lag` downstream of the moon (rad, in the sense the material drifts past it). Streaks
/// of azimuthal wavelength `3π·|Δr|` (the classic moonlet wake), decaying with `lag`
/// and with distance from the gap.
pub(crate) fn wake(r: f64, lag: f64) -> f64 {
    let dr = (r - PAN_R).abs().max(0.008);
    if dr > WAKE_REACH {
        return 1.0;
    }
    let lag = lag.rem_euclid(TAU);
    let env = (-lag / WAKE_DECAY).exp() * (1.0 - dr / WAKE_REACH);
    let k = 2.0 * r / (3.0 * dr);
    1.0 + WAKE_AMP * env * (k * lag).sin_det()
}

/// Edge-wave displacement (R_S) at angle `lag` downstream of Pan (RING-A5): a
/// decaying wave train; the envelope is `WAVE_AMP·e^{−lag/WAVE_DECAY}`.
pub(crate) fn edge_wave(lag: f64) -> f64 {
    let lag = lag.rem_euclid(TAU);
    WAVE_AMP * (-lag / WAVE_DECAY).exp() * (WAVE_K * lag).sin_det()
}

/// Unit vector towards the Sun: `z` north, `y` away from the viewer.
fn sun() -> [f64; 3] {
    let (se, ce) = SUN_ELEV.sin_cos_det();
    let (sa, ca) = SUN_AZ.sin_cos_det();
    [sa * ce, -ca * ce, se]
}

/// The ray `p + t·d` meets the globe (oblate spheroid) at some `t > 0`.
fn hits_globe(p: [f64; 3], d: [f64; 3]) -> bool {
    let q2 = POLAR * POLAR;
    let a = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] / q2;
    let b = 2.0 * (p[0] * d[0] + p[1] * d[1] + p[2] * d[2] / q2);
    let c = p[0] * p[0] + p[1] * p[1] + p[2] * p[2] / q2 - 1.0;
    let disc = b * b - 4.0 * a * c;
    disc > 0.0 && (-b + disc.sqrt()) > 0.0 && c > -1e-9
}

/// A ring-plane point lies in the globe's shadow (RING-A4).
pub(crate) fn in_planet_shadow(p: [f64; 3]) -> bool {
    // Cheap rejections first: sunward of the globe, or farther than its equatorial
    // radius from the shadow axis.
    let l = sun();
    let along = p[0] * l[0] + p[1] * l[1] + p[2] * l[2];
    if along >= 0.0 || p[0] * p[0] + p[1] * p[1] + p[2] * p[2] - along * along > 1.0 {
        return false;
    }
    hits_globe(p, l)
}

#[derive(Debug, Clone, Copy)]
struct Spoke {
    /// Angle in the corotating frame, its half-width (rad), radial extent, age and life.
    lambda: f64,
    half: f64,
    r0: f64,
    r1: f64,
    age: f64,
    life: f64,
}

impl Spoke {
    /// Angle now: spokes turn with the magnetic field, at Saturn's rate (RING-A3).
    fn angle(&self, t: f64) -> f64 {
        self.lambda + OMEGA_SATURN * t
    }

    fn strength(&self) -> f64 {
        (PI * (self.age / self.life).clamp(0.0, 1.0)).sin_det()
    }
}

/// One ringlet: its ring-plane radius and the shadow × lighting factor near Pan's gap.
#[derive(Debug, Clone, Copy)]
struct Ringlet {
    r: f32,
}

/// One dot of a ringlet, with what does not change per frame (built per resize).
#[derive(Debug, Clone, Copy)]
struct RingDot {
    x: u16,
    y: u16,
    ringlet: u16,
    lambda: f32,
    /// Brightness before texture, waves and spokes (footprint-averaged profile, lit).
    b0: f32,
    /// The lighting factor, kept (non-zero) only near the Encke gap, whose brightness is
    /// recomputed each frame for Pan's waves and wake.
    k: f32,
}

/// Screen geometry, in dots.
#[derive(Debug, Clone, Copy, Default)]
struct View {
    cx: f64,
    cy: f64,
    /// Dots per Saturn radius.
    scale: f64,
}

impl View {
    /// Dot of screen point `(sx, up)` (Saturn radii, `up` positive).
    fn dot(&self, sx: f64, up: f64) -> (f64, f64) {
        (self.cx + self.scale * sx, self.cy - self.scale * up)
    }
}

/// The globe's visible surface point under screen point `(sx, up)`, if any.
fn globe_point(sx: f64, up: f64) -> Option<[f64; 3]> {
    let (st, ct) = TILT.sin_cos_det();
    // Ray p0 + t·d towards the viewer, p0 on the plane through the centre.
    let p0 = [sx, up * st, up * ct];
    let d = [0.0, -ct, st];
    let q2 = POLAR * POLAR;
    let a = d[1] * d[1] + d[2] * d[2] / q2;
    let b = 2.0 * (p0[1] * d[1] + p0[2] * d[2] / q2);
    let c = p0[0] * p0[0] + p0[1] * p0[1] + p0[2] * p0[2] / q2 - 1.0;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b + disc.sqrt()) / (2.0 * a);
    Some([p0[0], p0[1] + t * d[1], p0[2] + t * d[2]])
}

#[derive(Debug)]
pub struct SaturnRings {
    t: f64,
    view: View,
    /// Ring brightness by radius (optical depth through the slant, albedo), `LUT` bins.
    profile: Box<[f32]>,
    /// Running sum of `profile` (`LUT + 1` entries) for footprint-averaged lookups.
    cumulative: Box<[f64]>,
    ringlets: Vec<Ringlet>,
    /// Every visible ringlet dot, built per resize.
    ring_dots: Vec<RingDot>,
    /// The globe's threads and the background stars, built per resize.
    still: Vec<(i32, i32, Ink)>,
    dots: Dots,
    spokes: Vec<Spoke>,
    /// The two live clump generations (RING-A9).
    clumps: [ClumpLayer; 2],
    size: Size,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl SaturnRings {
    fn new() -> Self {
        let mu = TILT.sin_det();
        let profile = (0..LUT)
            .map(|k| {
                let r = R_IN + (R_OUT - R_IN) * (k as f64 + 0.5) / LUT as f64;
                (albedo(r) * (1.0 - (-tau(r) / mu).exp())) as f32
            })
            .collect::<Box<[f32]>>();
        let cumulative = std::iter::once(0.0)
            .chain(profile.iter().scan(0.0, |acc, &v| {
                *acc += f64::from(v);
                Some(*acc)
            }))
            .collect();
        Self {
            t: 0.0,
            view: View::default(),
            profile,
            cumulative,
            ringlets: Vec::new(),
            ring_dots: Vec::new(),
            still: Vec::new(),
            dots: Dots::default(),
            spokes: Vec::with_capacity(SPOKES),
            clumps: [ClumpLayer::default(); 2],
            size: Size::new(1, 1),
            dirty: false,
        }
    }

    /// Mean ring brightness over radii `r ± width/2`, so a gap narrower than a ringlet's
    /// spacing darkens it instead of flickering.
    fn brightness_over(&self, r: f64, width: f64) -> f64 {
        let bins = LUT as f64 / (R_OUT - R_IN);
        let half = (0.5 * width * bins).max(0.5);
        let at = |x: f64| {
            let x = x.clamp(0.0, LUT as f64);
            let k = (x.floor() as usize).min(LUT - 1);
            self.cumulative[k] + (x - k as f64) * f64::from(self.profile[k])
        };
        let c = (r - R_IN) * bins;
        (at(c + half) - at(c - half)) / (2.0 * half)
    }

    /// The near-side ring hides the globe at screen point `(sx, up)`: the view ray meets
    /// the ring plane in front of the globe where the ring is fairly opaque.
    fn ring_in_front(&self, sx: f64, up: f64) -> bool {
        let ry = up / TILT.sin_det();
        ry < 0.0 && self.brightness_over(sx.hypot(ry), 0.01) > 0.25
    }

    /// The globe as threads (SPEC-002 Look, M16): its lit limb and band lines in flat
    /// steps, hidden on the night side, behind the near ring and in the rings' shadow,
    /// plus the background stars. Static, so built once per resize.
    fn build_still(&mut self) {
        self.still.clear();
        let l = sun();
        let (st, ct) = TILT.sin_cos_det();
        let v = self.view;
        let lit_of = |p: [f64; 3]| {
            let n = [p[0], p[1], p[2] / (POLAR * POLAR)];
            let nn = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) / nn
        };
        // The rings' shadow on the southern globe (the Sun is north of the rings).
        let shadowed = |s: &Self, p: [f64; 3]| {
            p[2] < 0.0 && {
                let k = -p[2] / l[2];
                let r = (p[0] + k * l[0]).hypot(p[1] + k * l[1]);
                s.brightness_over(r, 0.01) > 0.2
            }
        };
        let push = |s: &mut Self, (x, y): (f64, f64), ink: Ink| {
            s.still.push((x.round() as i32, y.round() as i32, ink));
        };
        // The limb: the outline ellipse (semi-axes 1 and b), lit side only.
        let b = (POLAR * POLAR * ct * ct + st * st).sqrt();
        let n = ((TAU * v.scale) as usize).clamp(16, 8192);
        for k in 0..n {
            let (s, c) = (TAU * k as f64 / n as f64).sin_cos_det();
            let (sx, up) = (0.999 * c, 0.999 * b * s);
            let Some(p) = globe_point(sx, up) else {
                continue;
            };
            if self.ring_in_front(sx, up) {
                continue;
            }
            // The night limb faint, so the globe's outline closes.
            let lit = lit_of(p);
            let ink = if lit < -0.02 {
                NIGHT_LIMB
            } else {
                LIMB_INKS[usize::from(lit < 0.3)]
            };
            push(self, v.dot(c, b * s), ink);
        }
        // Band lines: circles of planetographic latitude on the visible hemisphere.
        for lat in BAND_LATS {
            // Reduced latitude of the spheroid point.
            let beta = (POLAR * lat.sin_det()).atan2(lat.cos_det());
            let (sb, cb) = beta.sin_cos_det();
            let belt = saturn_bands(lat) < -0.3;
            let inks = if belt { BELT_INKS } else { ZONE_INKS };
            let m = ((TAU * cb * v.scale) as usize).clamp(8, 8192);
            for k in 0..m {
                let (sp, cp) = (TAU * k as f64 / m as f64).sin_cos_det();
                let p = [cb * sp, -cb * cp, POLAR * sb];
                // Facing the viewer.
                let nv = -ct * p[1] + st * p[2] / (POLAR * POLAR);
                if nv <= 0.02 {
                    continue;
                }
                let (sx, up) = (p[0], p[1] * st + p[2] * ct);
                let lit = lit_of(p);
                if lit < 0.05 || shadowed(self, p) || self.ring_in_front(sx, up) {
                    continue;
                }
                push(self, v.dot(sx, up), inks[usize::from(lit < 0.3)]);
            }
        }
        // Background stars, clear of the globe and the rings.
        let d = self.dots.dot_size();
        let (cw, ch) = (
            i32::from(d.width()) / Dots::CELL_W,
            i32::from(d.height()) / Dots::CELL_H,
        );
        for cy in 0..ch {
            for cx in 0..cw {
                if hash01(0x5A7 ^ cell_key(cx, cy)) >= STAR_SHARE {
                    continue;
                }
                let (x, y) = (
                    f64::from(cx * Dots::CELL_W),
                    f64::from(cy * Dots::CELL_H + 1),
                );
                let (sx, up) = ((x - v.cx) / v.scale, (v.cy - y) / v.scale);
                if sx.hypot(up / st) < R_OUT || sx.hypot(up) < 1.05 {
                    continue;
                }
                let bright = hash01(0x5A8 ^ cell_key(cx, cy)) > 0.7;
                let ink = Ink::new(Role::Star, if bright { 120 } else { 50 });
                push(self, (x, y), ink);
            }
        }
    }

    /// Every ringlet's visible dots with their static brightness (the profile over the
    /// ringlet's band, the globe's shadow, the lighting). Far-side dots behind the globe
    /// and dots of dark ring are left out (RING-A1, RING-A4).
    fn build_rings(&mut self) {
        let l = sun();
        let st = TILT.sin_det();
        let light = 0.85 + 0.15 * l[2] / SUN_ELEV.sin_det();
        let v = self.view;
        let spacing = ringlet_gap(v.scale);
        let dr = spacing / v.scale;
        let d = self.dots.dot_size();
        let (dw, dh) = (i32::from(d.width()), i32::from(d.height()));
        self.ringlets.clear();
        self.ring_dots.clear();
        let mut r = R_IN + 0.5 * dr;
        while r < A_OUTER + 0.5 * dr {
            let k = self.ringlets.len();
            self.ringlets.push(Ringlet { r: r as f32 });
            let keeler = (r - PAN_R).abs() < WAKE_REACH + 0.02;
            let b_static = self.brightness_over(r, 0.5 * dr);
            if !near_gap(r, GAP_CLEAR / v.scale) && (b_static > 0.02 || keeler) {
                let n = ((TAU * r * v.scale * 1.2) as usize).clamp(16, 1 << 15);
                let mut last = (i32::MIN, i32::MIN);
                for j in 0..n {
                    let lambda = TAU * j as f64 / n as f64;
                    let (s, c) = lambda.sin_cos_det();
                    // Screen distance to the next ringlet, across the ellipse (normal
                    // to it): `gap·sin i / |(sin λ, cos λ·sin i)|`; thin where it crowds.
                    let gap = spacing * st / s.hypot(c * st);
                    let every = (MIN_GAP / gap.max(1e-3)).ceil().max(1.0) as usize;
                    if k % every != 0 {
                        continue;
                    }
                    let (sx, up) = (r * c, r * s * st);
                    let (x, y) = v.dot(sx, up);
                    let (xi, yi) = (x.round() as i32, y.round() as i32);
                    if (xi, yi) == last || !(0..dw).contains(&xi) || !(0..dh).contains(&yi) {
                        continue;
                    }
                    last = (xi, yi);
                    // Behind the globe (far side), or in its shadow.
                    if s > 0.0 && globe_point(sx, up).is_some() {
                        continue;
                    }
                    let shade = if in_planet_shadow([r * c, r * s, 0.0]) {
                        0.06
                    } else {
                        1.0
                    };
                    let kf = light * shade;
                    let b0 = b_static * kf;
                    if b0 <= 0.02 && !keeler {
                        continue;
                    }
                    self.ring_dots.push(RingDot {
                        x: xi as u16,
                        y: yi as u16,
                        ringlet: k as u16,
                        lambda: lambda as f32,
                        b0: b0 as f32,
                        k: if keeler { kf as f32 } else { 0.0 },
                    });
                }
            }
            r += dr;
        }
        // TEST-R7 (M22): built by `push` once per resize, so drop the doubling slack (up to half the buffer at 500x150); frames are unchanged.
        self.ring_dots.shrink_to_fit();
    }

    fn spawn_spoke(&mut self, rng: &mut impl Rng) {
        if self.spokes.len() >= SPOKES {
            return;
        }
        // Spokes form on the morning ansa (left, where ring material leaves the shadow).
        let lambda = PI + rng.random_range(-0.6..0.6) - OMEGA_SATURN * self.t;
        let r0 = rng.random_range(SPOKE_BAND.0..SPOKE_BAND.0 + 0.1);
        self.spokes.push(Spoke {
            lambda,
            half: rng.random_range(0.1..0.2),
            r0,
            r1: rng.random_range(r0 + 0.14..SPOKE_BAND.1),
            age: 0.0,
            life: rng.random_range(SPOKE_LIFE.0..SPOKE_LIFE.1),
        });
    }

    /// Spoke strength at `(r, λ)`: 0 none, up to `SPOKE_DIM`.
    fn spoke_dim(&self, r: f64, lambda: f64) -> f64 {
        let mut dim: f64 = 0.0;
        for s in &self.spokes {
            if r < s.r0 || r > s.r1 {
                continue;
            }
            // Wedge-shaped: wider outward.
            let half = s.half * (0.6 + 0.8 * (r - s.r0) / (s.r1 - s.r0));
            let d = wrap_angle(lambda - s.angle(self.t)).abs();
            if d < half {
                let edge = 1.0 - d / half;
                dim = dim.max(SPOKE_DIM * s.strength() * ease::smoothstep(edge));
            }
        }
        dim
    }

    /// Draw the frame: the globe's threads, then every ringlet dot at its brightness now
    /// (clumps, Pan's waves and wake, spokes), then the moons.
    fn shade_rings(&mut self) {
        for (k, layer) in self.clumps.iter_mut().enumerate() {
            layer.refresh(self.t, k);
        }
        self.dots.clear();
        for &(x, y, ink) in &self.still {
            self.dots.set(x, y, ink);
        }
        let t = self.t;
        let pan = pan_angle(t);
        let dr = ringlet_gap(self.view.scale) / self.view.scale;
        let edge_reach = 0.5 * (ENCKE.1 - ENCKE.0) + WAVE_AMP + dr;
        for i in 0..self.ring_dots.len() {
            let p = self.ring_dots[i];
            let r = f64::from(self.ringlets[usize::from(p.ringlet)].r);
            let lambda = f64::from(p.lambda);
            let mut b = f64::from(p.b0);
            if p.k > 0.0 {
                // Pan's waves displace the Encke gap's edges and its wake ripples the
                // ring beside it: ahead of the moon on the faster inner side, behind it
                // on the slower outer one (RING-A5, RING-A8).
                let lag = if r < PAN_R {
                    lambda - pan
                } else {
                    pan - lambda
                };
                if (r - PAN_R).abs() < edge_reach {
                    b = self.brightness_over(r - edge_wave(lag), dr) * f64::from(p.k);
                }
                b *= wake(r, lag);
            }
            if b <= 0.0 {
                continue;
            }
            // Clumps sheared at the Keplerian rate (RING-A2, RING-A9).
            b *= clump_at(&self.clumps, r, lambda);
            // A soft shoulder instead of a clip, so the opaque B ring keeps its clumps.
            let lum = (0.05 + 0.84 * b / (1.0 + 0.12 * b)).min(1.0);
            let ink = if self.spoke_dim(r, lambda) > 0.15 {
                Some(SPOKE_INK)
            } else {
                ring_ink(lum)
            };
            if let Some(ink) = ink {
                self.dots.set(i32::from(p.x), i32::from(p.y), ink);
            }
        }
        // Pan, a white dot in its gap, and Mimas, a small white disc outside the rings
        // (both enlarged so they show), hidden behind the globe.
        self.draw_moon(PAN_R, pan, 0.0);
        let size = (MIMAS_SIZE * self.view.scale).max(0.8);
        self.draw_moon(MIMAS_R, mimas_angle(t), size);
    }

    /// A moon at ring-plane radius `r`, angle `lambda`, as a white dot disc of radius
    /// `size` dots, hidden where the globe is in front of it.
    fn draw_moon(&mut self, r: f64, lambda: f64, size: f64) {
        let st = TILT.sin_det();
        let (s, c) = lambda.sin_cos_det();
        let (sx, up) = (r * c, r * s * st);
        if s > 0.0 && globe_point(sx, up).is_some() {
            return; // behind the globe
        }
        let (x, y) = self.view.dot(sx, up);
        self.dots.disc(x, y, size, MOON_INK);
    }

    /// Screen position (dots) of a moon at `(r, λ)`, for the tests.
    #[cfg(test)]
    fn moon_dot(&self, r: f64, lambda: f64) -> (f64, f64) {
        let (s, c) = lambda.sin_cos_det();
        self.view.dot(r * c, r * s * TILT.sin_det())
    }
}

/// Pan's orbital angle at time `t` (Keplerian, RING-A8).
pub(crate) fn pan_angle(t: f64) -> f64 {
    kepler_rate(PAN_R) * t - 2.6
}

/// Mimas's orbital angle at time `t` (Keplerian, RING-A7): it starts on the near side,
/// low and left, so it crosses the view in the first half-minute.
pub(crate) fn mimas_angle(t: f64) -> f64 {
    kepler_rate(MIMAS_R) * t - 2.3
}

impl Generator for SaturnRings {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let d = self.dots.dot_size();
        let (w, h) = (f64::from(d.width()), f64::from(d.height()));
        let scale = FILL * (0.485 * w / R_OUT).min(0.44 * h / POLAR.max(R_OUT * TILT.sin_det()));
        self.view = View {
            cx: 0.5 * w,
            cy: 0.5 * h,
            scale: scale.max(0.5),
        };
        self.build_still();
        self.build_rings();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        self.t += dt;
        if self.t <= dt && self.spokes.is_empty() {
            // Open with a pair of spokes already on the ansa.
            self.spawn_spoke(&mut ctx.rng);
            self.spawn_spoke(&mut ctx.rng);
            for s in &mut self.spokes {
                s.age = 0.3 * s.life;
            }
        }
        for s in &mut self.spokes {
            s.age += dt;
        }
        self.spokes.retain(|s| s.age < s.life);
        // About one new spoke every 1.5 s.
        if dt > 0.0 && ctx.rng.random_bool((dt / 1.5).min(1.0)) {
            self.spawn_spoke(&mut ctx.rng);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.shade_rings();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_a1_gaps_are_nearly_empty() {
        for r in [
            0.5 * (CASSINI.0 + CASSINI.1),
            0.5 * (ENCKE.0 + ENCKE.1),
            0.5 * (KEELER.0 + KEELER.1),
        ] {
            assert!(tau(r) < 0.1, "τ({r}) = {}", tau(r));
        }
        // …between rings that are not.
        assert!(tau(1.75) > 1.0 && tau(2.1) > 0.4);
    }

    #[test]
    fn ring_a2_texture_shears_at_the_keplerian_rate() {
        let (inner_b, outer_a) = (1.53, 2.26);
        let ratio = kepler_rate(inner_b) / kepler_rate(outer_a);
        let want = (outer_a / inner_b).powf(1.5);
        assert!((ratio / want - 1.0).abs() < 0.05, "{ratio} vs {want}");
        assert!(ratio > 1.0, "inner ring must lead");
    }

    #[test]
    fn ring_a3_spokes_corotate_with_the_planet() {
        let s = Spoke {
            lambda: 0.3,
            half: 0.05,
            r0: 1.6,
            r1: 1.85,
            age: 0.0,
            life: 8.0,
        };
        let rate = (s.angle(2.0) - s.angle(1.0)) / 1.0;
        assert!((rate - OMEGA_SATURN).abs() < 1e-12);
        assert!((rate - kepler_rate(s.r0)).abs() > 0.1 * OMEGA_SATURN);
    }

    #[test]
    fn ring_a5_edge_waves_decay_downstream() {
        let envelope = |lag: f64| {
            (0..64)
                .map(|k| edge_wave(lag + f64::from(k) * TAU / WAVE_K / 64.0).abs())
                .fold(0.0, f64::max)
        };
        let (near, mid, far) = (envelope(0.05), envelope(1.0), envelope(3.0));
        assert!(near > mid && mid > far, "{near} {mid} {far}");
        assert!(near > 0.5 * WAVE_AMP);
    }

    #[test]
    fn ring_a4_the_globe_hides_and_shadows_the_far_rings() {
        let g = scene(100, 40, 0.0);
        // No ringlet dot lies on the far ring straight behind the globe's centre.
        let st = TILT.sin_det();
        for p in &g.ring_dots {
            let r = f64::from(g.ringlets[usize::from(p.ringlet)].r);
            let (s, c) = f64::from(p.lambda).sin_cos_det();
            assert!(
                !(s > 0.0 && globe_point(r * c, r * s * st).is_some()),
                "ring dot behind the globe at r {r}, λ {}",
                p.lambda
            );
        }
        // Anti-sunward far-side ring is in the shadow, its mirror image is lit.
        let l = sun();
        let anti = [-l[0], -l[1]];
        let n = anti[0].hypot(anti[1]);
        let p = [1.8 * anti[0] / n, 1.8 * anti[1] / n, 0.0];
        assert!(p[1] > 0.0, "the shadow falls on the far side");
        assert!(in_planet_shadow(p));
        assert!(!in_planet_shadow([-p[0], -p[1], 0.0]));
        assert!(!in_planet_shadow([2.2, -0.3, 0.0]));
        // In the frame, the shadowed far ring is dark where its mirror is lit.
        let lit_at = |q: [f64; 3]| {
            let (x, y) = g.view.dot(q[0], q[1] * st);
            (-2..=2).any(|d| g.dots.is_lit(x.round() as i32 + d, y.round() as i32))
        };
        assert!(!lit_at(p) && lit_at([-p[0], p[1], 0.0]));
    }

    fn scene(w: u16, h: u16, t: f64) -> SaturnRings {
        let mut g = SaturnRings::new();
        let mut ctx = Ctx::new(42, Size::new(w, h));
        g.resize(Size::new(w, h), &mut ctx);
        g.prepare();
        let steps = (t * 60.0).round() as usize;
        for _ in 0..steps {
            g.update(1.0 / 60.0, &mut ctx);
            g.prepare();
        }
        g
    }

    #[test]
    fn ring_a6_globe_is_a_lit_limb_and_band_threads_in_flat_steps() {
        let g = scene(120, 40, 0.0);
        let mut globe = [0usize; 3];
        for &(_, _, ink) in &g.still {
            if LIMB_INKS.contains(&ink) || ink == NIGHT_LIMB {
                globe[0] += 1;
            } else if ZONE_INKS.contains(&ink) {
                globe[1] += 1;
            } else if BELT_INKS.contains(&ink) {
                globe[2] += 1;
            } else {
                assert_eq!(ink.role, Role::Star, "{ink:?}");
            }
        }
        assert!(
            globe.iter().all(|&n| n > 20),
            "limb, zones, belts: {globe:?}"
        );
        // No band line lies in the rings' shadow on the southern globe, nor on the
        // night side, nor behind the near ring.
        // (Checked at the rounded dot, so a dot or two on an edge may straddle it.)
        let l = sun();
        let st = TILT.sin_det();
        let (mut bad, mut bands) = (0, 0);
        for &(x, y, ink) in &g.still {
            if !(ZONE_INKS.contains(&ink) || BELT_INKS.contains(&ink)) {
                continue;
            }
            bands += 1;
            let (sx, up) = (
                (f64::from(x) - g.view.cx) / g.view.scale,
                (g.view.cy - f64::from(y)) / g.view.scale,
            );
            let ry = up / st;
            if ry < 0.0 && g.brightness_over(sx.hypot(ry), 0.01) > 0.3 {
                bad += 1;
            }
            if let Some(p) = globe_point(sx, up) {
                if p[2] < -0.05 {
                    let k = -p[2] / l[2];
                    let r = (p[0] + k * l[0]).hypot(p[1] + k * l[1]);
                    bad += usize::from(g.brightness_over(r, 0.01) >= 0.5);
                }
            }
        }
        // Half a dot is ≈ 0.025 R_S of ring radius through the tilt: edge dots straddle.
        assert!(
            bad * 16 <= bands,
            "{bad} of {bands} band dots behind or under the rings"
        );
        // The band lines stop at the terminator: none on the far left (night) limb.
        let left = g
            .still
            .iter()
            .filter(|&&(x, _, ink)| {
                (ZONE_INKS.contains(&ink) || BELT_INKS.contains(&ink))
                    && f64::from(x) < g.view.cx - 0.9 * g.view.scale
            })
            .count();
        assert_eq!(left, 0);
        // Every lit cell is braille: the globe is threads, not a half-block disc.
        let mut canvas = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut canvas);
        for y in 0..40 {
            for x in 0..120 {
                let ch = canvas.get(x, y).unwrap().ch;
                assert!(ch == ' ' || ('⠀'..='⣿').contains(&ch), "{x},{y}: {ch:?}");
            }
        }
    }

    #[test]
    fn ring_a7_mimas_orbits_keplerian_and_holds_the_cassini_edge() {
        // Kepler: Mimas turns at Ω(r) and more slowly than any ring.
        let rate = mimas_angle(1.0) - mimas_angle(0.0);
        assert!((rate - kepler_rate(MIMAS_R)).abs() < 1e-12);
        assert!(rate < kepler_rate(A_OUTER));
        // Its 2:1 inner Lindblad resonance, r = r_M·2^{−2/3}, sits at the B ring's
        // outer edge, the Cassini Division's inner boundary (within 1 %).
        let res = MIMAS_R * 2f64.powf(-2.0 / 3.0);
        assert!(
            (res / CASSINI.0 - 1.0).abs() < 0.01,
            "{res} vs {}",
            CASSINI.0
        );
        // It is on screen as a white dot disc, and moves, between t = 1 and t = 30.
        let (a, b) = (scene(120, 40, 1.0), scene(120, 40, 30.0));
        let at = |g: &SaturnRings| {
            let (x, y) = g.moon_dot(MIMAS_R, mimas_angle(g.t));
            (x.round() as i32, y.round() as i32)
        };
        for g in [&a, &b] {
            let (x, y) = at(g);
            assert!(g.dots.is_lit(x, y), "Mimas not drawn at ({x}, {y})");
            let mut canvas = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
            g.render(&mut canvas);
            let cell = canvas.get(x / 2, y / 4).expect("Mimas on screen");
            assert_eq!(cell.ink, MOON_INK);
        }
        assert_ne!(at(&a), at(&b));
    }

    #[test]
    fn ring_a8_pan_wake_ripples_decay_downstream_and_away_from_the_gap() {
        let swing = |r: f64, lag: f64| {
            (0..200)
                .map(|k| (wake(r, lag + f64::from(k) * 0.002) - 1.0).abs())
                .fold(0.0, f64::max)
        };
        let r = PAN_R - 0.03;
        let (near, far) = (swing(r, 0.05), swing(r, 3.0));
        assert!(near > 0.15 && near > 4.0 * far, "{near} {far}");
        assert!(swing(PAN_R - 0.07, 0.05) < near);
        assert_eq!(wake(PAN_R - 2.0 * WAKE_REACH, 0.1), 1.0);
        // Wake streaks tighten towards the moon: wavelength 3π·|Δr|.
        let zeros = |r: f64| {
            (0..400)
                .filter(|&k| {
                    let (a, b) = (f64::from(k) * 0.001, f64::from(k + 1) * 0.001);
                    (wake(r, a) - 1.0).signum() != (wake(r, b) - 1.0).signum()
                })
                .count()
        };
        assert!(zeros(PAN_R - 0.02) > zeros(PAN_R - 0.06));
    }

    /// The lit ringlet dots of a scene.
    fn ring_lit(g: &SaturnRings) -> Vec<bool> {
        g.ring_dots
            .iter()
            .map(|p| g.dots.is_lit(i32::from(p.x), i32::from(p.y)))
            .collect()
    }

    #[test]
    fn ring_a9_clumps_are_smooth_in_radius_and_drift_by_t30() {
        // Low-pass in radius: one 200×50 pixel on the ansa is ≈ 0.02 R_S.
        for k in 0..200 {
            let r = 1.3 + f64::from(k) * 0.005;
            let lambda = f64::from(k) * 0.37;
            let d = (clump(r + 0.02, lambda, 3.0) - clump(r, lambda, 3.0)).abs();
            assert!(d < 0.3 * CLUMP_AMP, "r {r}: {d}");
        }
        // The ringlets visibly change between t = 1 and t = 30 (clumps, spokes, moons):
        // at least a fifth of their dots switch on or off, or change ink.
        let (a, b) = (scene(120, 40, 1.0), scene(120, 40, 30.0));
        let (mut ca, mut cb) = (
            Canvas::new(Size::new(120, 40), GlyphMode::Unicode),
            Canvas::new(Size::new(120, 40), GlyphMode::Unicode),
        );
        a.render(&mut ca);
        b.render(&mut cb);
        let (la, lb) = (ring_lit(&a), ring_lit(&b));
        let changed = a
            .ring_dots
            .iter()
            .zip(la.iter().zip(&lb))
            .filter(|(p, (x, y))| {
                let (cx, cy) = (i32::from(p.x) / 2, i32::from(p.y) / 4);
                x != y || ca.get(cx, cy).map(|c| c.ink) != cb.get(cx, cy).map(|c| c.ink)
            })
            .count();
        assert!(
            changed * 5 >= a.ring_dots.len(),
            "{changed} of {} ring dots changed",
            a.ring_dots.len()
        );
    }

    #[test]
    fn ring_a11_ringlets_never_hatch_into_a_fill() {
        // At 80×24, 160×45 and 200×50, under 10 % of the lit ringlet dots have all four
        // neighbours (left, right, above, below) lit; every gap clears its ringlets.
        for (w, h) in [(80, 24), (160, 45), (200, 50)] {
            let g = scene(w, h, 3.0);
            let (mut lit, mut packed) = (0, 0);
            for p in &g.ring_dots {
                let (x, y) = (i32::from(p.x), i32::from(p.y));
                if !g.dots.is_lit(x, y) {
                    continue;
                }
                lit += 1;
                packed += usize::from(
                    [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .all(|&(dx, dy)| g.dots.is_lit(x + dx, y + dy)),
                );
            }
            assert!(lit > 200, "{w}×{h}: {lit} ring dots");
            assert!(packed * 10 < lit, "{w}×{h}: {packed} of {lit} packed");
            let clear = GAP_CLEAR / g.view.scale;
            for rl in &g.ringlets {
                let r = f64::from(rl.r);
                let drawn = g
                    .ring_dots
                    .iter()
                    .any(|p| g.ringlets[usize::from(p.ringlet)].r == rl.r);
                assert!(
                    !(drawn && near_gap(r, clear)),
                    "{w}×{h}: ringlet at {r} in a gap"
                );
            }
        }
        // The Cassini Division is at least 3 dots of black at 160×45 on the ansae.
        let g = scene(160, 45, 0.0);
        let drawn: Vec<f64> = g
            .ring_dots
            .iter()
            .map(|p| f64::from(g.ringlets[usize::from(p.ringlet)].r))
            .collect();
        let below = drawn
            .iter()
            .copied()
            .filter(|&r| r < CASSINI.0)
            .fold(0.0, f64::max);
        let above = drawn
            .iter()
            .copied()
            .filter(|&r| r > CASSINI.1)
            .fold(9.0, f64::min);
        assert!((above - below) * g.view.scale >= 4.0, "{below}..{above}");
    }

    #[test]
    fn ring_a10_ringlets_are_threads_with_black_gaps() {
        let g = scene(200, 50, 2.0);
        // The Cassini Division stays black: no ringlet dot in its core (the Encke and
        // Keeler gaps are narrower than the ringlet spacing: they darken their ringlet).
        let core = (CASSINI.0 + 0.02)..(CASSINI.1 - 0.02);
        assert!(g.ringlets.iter().any(|rl| core.contains(&f64::from(rl.r))));
        // On the ansae, where the ringlets do not overlap on screen.
        for p in &g.ring_dots {
            if f64::from(p.lambda).cos_det().abs() < 0.8 {
                continue;
            }
            let r = f64::from(g.ringlets[usize::from(p.ringlet)].r);
            let lit = g.dots.is_lit(i32::from(p.x), i32::from(p.y));
            assert!(
                !(lit && core.contains(&r)),
                "lit dot in the Cassini Division at r {r}"
            );
        }
        // Threads, not a fill: on the ansae (|cos λ| > 0.8) neighbouring ringlets are
        // `ringlet_gap` dots apart, so under half of the dots in the ring band are lit.
        let st = TILT.sin_det();
        let d = g.dots.dot_size();
        let (mut lit, mut n) = (0, 0);
        for y in 0..i32::from(d.height()) {
            for x in 0..i32::from(d.width()) {
                let (sx, up) = (
                    (f64::from(x) - g.view.cx) / g.view.scale,
                    (g.view.cy - f64::from(y)) / g.view.scale,
                );
                let (rx, ry) = (sx, up / st);
                let r = rx.hypot(ry);
                if (1.55..1.9).contains(&r) && rx.abs() > 0.8 * r {
                    n += 1;
                    lit += usize::from(g.dots.is_lit(x, y));
                }
            }
        }
        assert!(n > 200 && lit * 2 < n, "{lit} of {n} ring-band dots lit");
        assert!(lit * 5 > n, "B ring too sparse: {lit} of {n}");
    }
}
