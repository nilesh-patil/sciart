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
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, disc_pixels};
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
/// The globe's ramp: cream-butterscotch zones and tan belts, a muted earth tone.
const GLOBE: Ramp = Ramp::Sienna;
/// The lowest Sienna level the globe uses (a muted tan-brown); dimmer globe is drawn in
/// low Starlight. And the globe coverage below which a limb pixel stays space.
const GLOBE_FLOOR: f64 = 0.5;
const GLOBE_EDGE: f64 = 0.4;
/// Background stars per 1000 pixels.
const STAR_DENSITY: f64 = 2.5;

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

/// A ring pixel: where it is on the ring plane and what does not change per frame.
#[derive(Debug, Clone, Copy)]
struct RingPx {
    /// Row-major pixel index.
    idx: u32,
    r: f32,
    lambda: f32,
    /// Brightness before texture and spokes.
    b0: f32,
    /// Shadow × lighting factor, kept (non-zero) only near the Keeler gap, whose
    /// brightness is recomputed each frame for Pan's waves and wake.
    k: f32,
    /// Globe coverage under the pixel, negated on the far side of the rings (behind
    /// the globe's centre plane). 24 bytes a pixel: this table is the scene's largest
    /// buffer at 500×150 (TEST-R7).
    cover: f32,
}

/// Screen geometry, in pixels.
#[derive(Debug, Clone, Copy, Default)]
struct View {
    cx: f64,
    cy: f64,
    /// Pixels per Saturn radius.
    scale: f64,
}

#[derive(Debug)]
pub struct SaturnRings {
    t: f64,
    view: View,
    /// Ring brightness by radius (optical depth through the slant, albedo), `LUT` bins.
    profile: Box<[f32]>,
    /// Running sum of `profile` (`LUT + 1` entries) for footprint-averaged lookups.
    cumulative: Box<[f64]>,
    /// The globe and stars, shaded once per resize; the rings go over it each frame.
    globe: Pixels,
    /// Every ring pixel's static data, built per resize.
    ring_px: Vec<RingPx>,
    pixels: Pixels,
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
            globe: Pixels::default(),
            ring_px: Vec::new(),
            pixels: Pixels::default(),
            spokes: Vec::with_capacity(SPOKES),
            clumps: [ClumpLayer::default(); 2],
            size: Size::new(1, 1),
            dirty: false,
        }
    }

    /// Mean ring brightness over radii `r ± width/2`: the pixel's footprint, so gaps
    /// narrower than a pixel darken it instead of flickering in and out.
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

    /// Screen offsets of pixel `(x, y)` from the centre in Saturn radii (`up` positive).
    fn screen(&self, x: i32, y: i32) -> (f64, f64) {
        let v = self.view;
        (
            (f64::from(x) + 0.5 - v.cx) / v.scale,
            (v.cy - f64::from(y) - 0.5) / v.scale,
        )
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

    /// Share of the pixel `(x, y)` the globe covers: four sub-pixel rays, refined to a
    /// 4×4 grid where they disagree or the limb is within a pixel, so the limb steps in
    /// sixteenths rather than quarters.
    fn globe_cover(&self, x: i32, y: i32) -> f64 {
        let (sx, up) = self.screen(x, y);
        let q = 0.25 / self.view.scale;
        let hits = [(-q, -q), (q, -q), (-q, q), (q, q)]
            .iter()
            .filter(|&&(dx, dy)| Self::globe_point(sx + dx, up + dy).is_some())
            .count();
        // The screen outline is an ellipse with semi-axes 1 and ≈ 0.91: within a pixel of
        // it only when the normalised radius is near 1.
        let e = sx.hypot(up / 0.91);
        let near = (e - 1.0).abs() < 1.5 / self.view.scale;
        if !near && (hits == 0 || hits == 4) {
            return hits as f64 / 4.0;
        }
        let step = 1.0 / (4.0 * self.view.scale);
        let mut n = 0;
        for j in 0..4 {
            for i in 0..4 {
                let dx = (f64::from(i) - 1.5) * step;
                let dy = (f64::from(j) - 1.5) * step;
                n += usize::from(Self::globe_point(sx + dx, up + dy).is_some());
            }
        }
        n as f64 / 16.0
    }

    fn shade_globe(&mut self) {
        let l = sun();
        let (st, ct) = TILT.sin_cos_det();
        let view = [0.0, -ct, st];
        let size = self.globe.pixel_size();
        self.globe.clear();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let (sx, up) = self.screen(x, y);
                let cover = self.globe_cover(x, y);
                let Some(p) = Self::globe_point(sx, up).or_else(|| {
                    // A limb pixel whose centre misses: shade it at the nearest hit.
                    (cover > 0.0).then(|| {
                        let q = 0.25 / self.view.scale;
                        [(-q, -q), (q, -q), (-q, q), (q, q)]
                            .iter()
                            .find_map(|&(dx, dy)| Self::globe_point(sx + dx, up + dy))
                    })?
                }) else {
                    let h = hash01(0x5A7 ^ cell_key(x, y));
                    if h < STAR_DENSITY / 1000.0 {
                        let lv = 0.25 + 0.5 * hash01(0x5A8 ^ cell_key(x, y));
                        self.globe.set(x, y, ink::ramp(Ramp::Starlight, lv));
                    }
                    continue;
                };
                let n = [p[0], p[1], p[2] / (POLAR * POLAR)];
                let nn = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let lit = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) / nn;
                // A soft terminator: the day side fades out over a few degrees instead of
                // stepping to black.
                let day = ease::smoothstep((lit + 0.06) / 0.28);
                if day <= 0.0 {
                    continue; // night side: black against black space
                }
                // Limb darkening by the cosine to the viewer (RING-A6).
                let mu = ((n[1] * view[1] + n[2] * view[2]) / nn).max(0.0);
                // Floored so the limb stays a muted tan: Sienna's low levels are
                // rust-orange and would ring the globe.
                let limb = 0.64 + 0.36 * mu.sqrt();
                // Soft zonal bands by planetographic latitude: cream zones, butterscotch
                // and tan belts, a duskier polar hood; faint streaks within them.
                let lat = (p[2] / POLAR).clamp(-1.0, 1.0).asin();
                let lon = p[0].atan2(-p[1]);
                let bands = saturn_bands(lat);
                let streak = value_noise(0x5A70, 3.0 * lon, 40.0 * lat, 1.0) - 0.5;
                let polar = (lat.abs() / 1.2).min(1.0);
                let tone =
                    1.06 + 0.12 * bands * (1.0 - polar) + 0.025 * streak - 0.2 * polar * polar;
                // The rings' shadow on the southern globe (the Sun is north of them),
                // averaged over the pixel so the band's edges are soft.
                let mut shadow = 1.0;
                if p[2] < 0.0 {
                    let s = -p[2] / l[2];
                    let r = (p[0] + s * l[0]).hypot(p[1] + s * l[1]);
                    shadow = 1.0 - 0.7 * self.brightness_over(r, 2.5 / self.view.scale);
                }
                let light = day * (0.6 + 0.4 * lit.max(0.0).sqrt());
                // The limb fades into space within one pixel: pixels the globe covers
                // less than `GLOBE_EDGE` of stay space, the rest dim only slightly with
                // coverage, so the edge never passes through Sienna's orange low end.
                if cover < GLOBE_EDGE {
                    continue;
                }
                let v = tone * limb * light * shadow * (0.8 + 0.2 * cover);
                // Dim globe (terminator, the rings' shadow) goes to a low neutral
                // Starlight rather than down Sienna's rust-orange ramp (RING-A6).
                let ink = if v >= GLOBE_FLOOR {
                    ink::ramp(GLOBE, v)
                } else {
                    ink::ramp(Ramp::Starlight, 0.6 * v)
                };
                if f64::from(ink.level) >= ink::MIN_VISIBLE_LEVEL {
                    self.globe.set(x, y, ink);
                }
            }
        }
    }

    /// Precompute every ring pixel's ring-plane position and static brightness (the
    /// profile over its footprint, the globe's shadow, the lighting), and how much of it
    /// the globe covers. Far-side pixels wholly behind the globe are left out (RING-A4).
    fn build_rings(&mut self) {
        let l = sun();
        let size = self.pixels.pixel_size();
        let sin_t = TILT.sin_det();
        let light = 0.85 + 0.15 * l[2] / SUN_ELEV.sin_det();
        self.ring_px.clear();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let (sx, up) = self.screen(x, y);
                let (rx, ry) = (sx, up / sin_t);
                let r = rx.hypot(ry);
                if !(R_IN..R_OUT).contains(&r) {
                    continue;
                }
                let cover = self.globe_cover(x, y);
                if cover >= 1.0 && ry > 0.0 {
                    continue;
                }
                let foot = (rx.abs() + ry.abs() / sin_t) / (r * self.view.scale);
                // The globe's shadow, averaged over four sub-pixel points so its edge is
                // soft rather than stepped.
                let q = 0.25 / self.view.scale;
                let shadowed = [(-q, -q), (q, -q), (-q, q), (q, q)]
                    .iter()
                    .filter(|&&(dx, dy)| in_planet_shadow([rx + dx, ry + dy / sin_t, 0.0]))
                    .count() as f64
                    / 4.0;
                let k = light * (1.0 - 0.94 * shadowed);
                let b0 = self.brightness_over(r, foot) * k;
                // Near Pan's gap the brightness is recomputed each frame for its edge
                // waves and wake.
                let keeler = (r - PAN_R).abs() < WAKE_REACH + 0.02;
                if b0 <= 0.0 && !keeler {
                    continue;
                }
                self.ring_px.push(RingPx {
                    idx: (y as u32) * u32::from(size.width()) + x as u32,
                    r: r as f32,
                    lambda: ry.atan2(rx) as f32,
                    b0: b0 as f32,
                    k: if keeler { k as f32 } else { 0.0 },
                    cover: if ry > 0.0 {
                        -cover as f32
                    } else {
                        cover as f32
                    },
                });
            }
        }
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

    /// Spoke darkening at `(r, λ)`: 0 none, up to `SPOKE_DIM`.
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
                // A soft-edged, fuzzy wedge.
                dim = dim.max(SPOKE_DIM * s.strength() * ease::smoothstep(edge));
            }
        }
        dim
    }

    fn shade_rings(&mut self) {
        for (k, layer) in self.clumps.iter_mut().enumerate() {
            layer.refresh(self.t, k);
        }
        let globe = &self.globe;
        self.pixels
            .shade(|x, y| globe.get(x, y).unwrap_or(Ink::BLANK));
        let t = self.t;
        let pan = pan_angle(t);
        // Half the Encke gap, the waves' reach and a footprint of up to 3 pixels
        // (foreshortened by 1/sin tilt on the near and far sides).
        let edge_reach =
            0.5 * (ENCKE.1 - ENCKE.0) + WAVE_AMP + 1.5 / (TILT.sin_det() * self.view.scale);
        let w = usize::from(self.pixels.pixel_size().width()).max(1);
        for k in 0..self.ring_px.len() {
            let p = self.ring_px[k];
            let (r, lambda) = (f64::from(p.r), f64::from(p.lambda));
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
                // Only pixels whose footprint can reach a displaced edge need the
                // profile re-averaged; the rest keep their static brightness.
                if (r - PAN_R).abs() < edge_reach {
                    let (s, c) = lambda.sin_cos_det();
                    let foot = (c.abs() + s.abs() / TILT.sin_det()) / self.view.scale;
                    b = self.brightness_over(r - edge_wave(lag), foot) * f64::from(p.k);
                }
                b *= wake(r, lag);
            }
            if b <= 0.0 {
                continue;
            }
            // Clumps sheared at the Keplerian rate (RING-A2, RING-A9).
            b *= clump_at(&self.clumps, r, lambda);
            b *= 1.0 - self.spoke_dim(r, lambda);
            // A soft shoulder instead of a clip, so the opaque B ring keeps its clumps.
            let ring_lum = (0.05 + 0.84 * b / (1.0 + 0.12 * b)).min(1.0);
            let (x, y) = ((p.idx as usize % w) as i32, (p.idx as usize / w) as i32);
            let under = self.globe.get(x, y).unwrap_or(Ink::BLANK);
            let (cover, far) = (f64::from(p.cover.abs()), p.cover < 0.0);
            // How much of the pixel the ring owns: on the far side, what the globe
            // leaves uncovered; on the near side, its opacity over the globe (the thin
            // C ring lets the globe show through), and all of it off the globe.
            let alpha = if far {
                1.0 - cover
            } else {
                // From the static brightness, so clumps never flip who owns the pixel.
                let opaque = ((f64::from(p.b0) - 0.08) / 0.3).clamp(0.0, 1.0);
                opaque + (1.0 - opaque) * (1.0 - cover)
            };
            let under_lum = if under.role == Role::Ramp(GLOBE) {
                f64::from(under.level) / 255.0
            } else {
                0.0
            };
            let lum = alpha * ring_lum + (1.0 - alpha) * under_lum;
            // One ink per pixel: the ramp of whichever owns more of it, at the blended
            // brightness, so the globe's edge steps smoothly.
            let ink = if alpha >= 0.5 || under.role != Role::Ramp(GLOBE) {
                ink::ramp(Ramp::Starlight, lum)
            } else if lum >= GLOBE_FLOOR {
                ink::ramp(GLOBE, lum)
            } else {
                // Never Sienna's rust-orange low end (RING-A6).
                ink::ramp(Ramp::Starlight, 0.6 * lum)
            };
            if lum * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                self.pixels.set(x, y, ink);
            }
        }
        // Pan, a bright point in its gap, and Mimas, a small lit disc outside the rings
        // (both enlarged so they show).
        self.draw_moon(PAN_R, pan, 0.45);
        let size = (MIMAS_SIZE * self.view.scale).max(0.8);
        self.draw_moon(MIMAS_R, mimas_angle(t), size);
    }

    /// A moon at ring-plane radius `r`, angle `lambda`, drawn as an anti-aliased disc of
    /// `size` pixels lit from the Sun's side, hidden where the globe is in front of it.
    fn draw_moon(&mut self, r: f64, lambda: f64, size: f64) {
        let (st, ct) = TILT.sin_cos_det();
        let (s, c) = lambda.sin_cos_det();
        let (sx, up) = (r * c, r * s * st);
        if s > 0.0 && Self::globe_point(sx, up).is_some() {
            return; // behind the globe
        }
        let v = self.view;
        let centre = (v.cx + v.scale * sx, v.cy - v.scale * up);
        let l = sun();
        let pixels = &mut self.pixels;
        disc_pixels(centre, size, |x, y, cover| {
            // The disc's outward normal from its screen offset: right, up, towards us.
            let nx = ((f64::from(x) + 0.5 - centre.0) / size).clamp(-1.0, 1.0);
            let ny = ((centre.1 - f64::from(y) - 0.5) / size).clamp(-1.0, 1.0);
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let n = [nx, ny * st - nz * ct, ny * ct + nz * st];
            let lit = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
            let lum = cover * (0.45 + 0.55 * lit);
            let old = pixels.get(x, y).unwrap_or(Ink::BLANK);
            let ink = ink::ramp(Ramp::Starlight, lum.max(0.35 * cover));
            if old.role != Role::Ramp(Ramp::Starlight) || ink.level > old.level {
                pixels.set(x, y, ink);
            }
        });
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
        self.globe.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let scale = (0.485 * w / R_OUT).min(0.44 * h / POLAR.max(R_OUT * TILT.sin_det()));
        self.view = View {
            cx: 0.5 * w,
            cy: 0.5 * h,
            scale: scale.max(0.5),
        };
        self.shade_globe();
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
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
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
    fn ring_a4_the_globe_hides_and_shadows_the_far_rings() {
        let mut g = SaturnRings::new();
        let mut ctx = Ctx::new(42, Size::new(100, 40));
        g.resize(Size::new(100, 40), &mut ctx);
        g.prepare();
        // The far ring straight behind the globe's centre shows the globe, not ring.
        let (st, v) = (TILT.sin_det(), g.view);
        let far = |r: f64| (v.cx as i32, (v.cy - v.scale * r * st).floor() as i32);
        let (x, y) = far(1.8);
        let px = g.pixels.get(x, y).expect("on screen");
        assert_eq!(px.role, Role::Ramp(GLOBE), "{px:?}");
        // Anti-sunward far-side ring is in the shadow, its mirror image is lit.
        let l = sun();
        let anti = [-l[0], -l[1]];
        let n = anti[0].hypot(anti[1]);
        let p = [1.8 * anti[0] / n, 1.8 * anti[1] / n, 0.0];
        assert!(p[1] > 0.0, "the shadow falls on the far side");
        assert!(in_planet_shadow(p));
        assert!(!in_planet_shadow([-p[0], -p[1], 0.0]));
        assert!(!in_planet_shadow([2.2, -0.3, 0.0]));
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
    fn ring_a6_globe_is_pale_butterscotch_limb_darkened_and_soft_edged() {
        let g = scene(120, 40, 0.0);
        let v = g.view;
        // The globe's bright middle (above the rings) is pale cream in Sienna.
        let row = (v.cy - 0.55 * v.scale) as i32;
        let mid = (-4..=4)
            .filter_map(|dx| g.globe.get(v.cx as i32 + dx, row))
            .max_by_key(|i| i.level)
            .expect("on screen");
        assert_eq!(mid.role, Role::Ramp(Ramp::Sienna), "{mid:?}");
        assert!(mid.level >= 200, "centre level {}", mid.level);
        // Limb darkening: along the row, the lit right limb is darker than the middle.
        let limb_x = (0..i32::from(g.globe.pixel_size().width()))
            .rev()
            .find(|&x| g.globe.get(x, row).is_some_and(|i| i.role == mid.role))
            .expect("globe row");
        let inner = g.globe.get(limb_x - 2, row).expect("inside").level;
        assert!(inner + 6 < mid.level, "inner {inner} vs mid {}", mid.level);
        // No rust rim: every Sienna pixel of the globe, limb included, stays at or above
        // the muted-tan floor; dimmer globe is low Starlight.
        let edge = g.globe.get(limb_x, row).expect("edge");
        assert!(edge.level <= inner, "edge {edge:?} vs inner {inner}");
        let floor = ink::ramp(GLOBE, GLOBE_FLOOR).level;
        let size = g.globe.pixel_size();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let px = g.globe.get(x, y).unwrap();
                if px.role == Role::Ramp(GLOBE) {
                    assert!(px.level >= floor, "({x}, {y}): {px:?}");
                }
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
        // It is on screen, and moves, between t = 1 and t = 30.
        let lit = |g: &SaturnRings| -> Vec<(i32, i32)> {
            let (st, v) = (TILT.sin_det(), g.view);
            let a = mimas_angle(g.t);
            let (s, c) = a.sin_cos_det();
            vec![(
                (v.cx + v.scale * MIMAS_R * c) as i32,
                (v.cy - v.scale * MIMAS_R * s * st) as i32,
            )]
        };
        let (a, b) = (scene(120, 40, 1.0), scene(120, 40, 30.0));
        for g in [&a, &b] {
            let (x, y) = lit(g)[0];
            let px = g.pixels.get(x, y).expect("Mimas on screen");
            assert_eq!(px.role, Role::Ramp(Ramp::Starlight));
            assert!(px.level >= 90, "Mimas level {}", px.level);
        }
        assert_ne!(lit(&a), lit(&b));
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

    #[test]
    fn ring_a9_clumps_are_smooth_in_radius_and_drift_by_t30() {
        // Low-pass in radius: one 200×50 pixel on the ansa is ≈ 0.02 R_S.
        for k in 0..200 {
            let r = 1.3 + f64::from(k) * 0.005;
            let lambda = f64::from(k) * 0.37;
            let d = (clump(r + 0.02, lambda, 3.0) - clump(r, lambda, 3.0)).abs();
            assert!(d < 0.3 * CLUMP_AMP, "r {r}: {d}");
        }
        // The rings visibly change between t = 1 and t = 30 (clumps, spokes, moons).
        let (a, b) = (scene(120, 40, 1.0), scene(120, 40, 30.0));
        let size = a.pixels.pixel_size();
        let (mut ring, mut changed) = (0, 0);
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let (p, q) = (a.pixels.get(x, y).unwrap(), b.pixels.get(x, y).unwrap());
                if p.role == Role::Ramp(Ramp::Starlight) && p.level > 60 {
                    ring += 1;
                    changed += usize::from(p.level.abs_diff(q.level) >= 12);
                }
            }
        }
        assert!(
            changed * 3 >= ring,
            "{changed} of {ring} ring pixels changed"
        );
    }
}
