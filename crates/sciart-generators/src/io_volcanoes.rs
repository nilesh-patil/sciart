//! Io's volcanoes on its limb against black space (SPEC-002 `io_volcanoes`).
//!
//! Three vents throw umbrella plumes with the shared `plume` model: ballistic grains in
//! Io's gravity (1.796 m/s², no air) on exact parabolas, a giant Pele-type vent at 1 km/s
//! (≈ 280 km high) and two squat Prometheus-type vents near 0.5 km/s (≈ 70 km). Grains
//! launched near 45° land at the ballistic range and pile up there, so the fallout paints
//! rings: Pele's red (short-chain sulfur), the Prometheus types' white (SO₂ frost).
//!
//! M17 (GEN-R26, orbital-thread style; the "limb + ballistic plume threads" design): Io is
//! a thin `Sun` limb arc over a sparse sulphur stipple (fixed per-dot thresholds, densest
//! on the sub-solar side and thinning across the terminator); the plumes are hundreds of
//! grains, each a short decaying thread along its own parabola, coloured by its state in
//! flight: `Hot` leaving the vent, `Sun` while rising, `Cold` near the apex and falling
//! (where SO₂ condenses), so the umbrella canopy and the fallout curtains emerge from the
//! trajectories. Landed grains add a slowly fading stipple ring; active paterae are tiny
//! `Hot` knots that flicker with their output; the vents pulse with the tide.
//!
//! Units: plume physics in km and real seconds, run ×100 time-lapsed; the plumes are
//! drawn ×1.6 larger relative to Io than they are (heights keep their exact ratio). The
//! screen is in braille dots (square).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::ease;
use sciart_core::prelude::*;
use sciart_core::raster::stipple_keep;
use sciart_core::rng::{cell_key, hash01, value_noise};

use crate::shared::plume::{self, Plume};

pub static META: Meta = Meta {
    name: "io_volcanoes",
    category: Category::SunPlanets,
    description: "Pele and Prometheus throw umbrella plumes on exact ballistic arcs that paint rings on Io",
    title: "Io volcanoes · tides melt rock",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(IoVolcanoes::new())
}

/// Io's radius (km) and surface gravity (km/s²).
const IO_KM: f64 = 1821.6;
pub(crate) const G_IO: f64 = 1.796e-3;
/// Time-lapse: real seconds per simulated second.
const LAPSE: f64 = 100.0;
/// Plume drawing scale relative to Io.
const EXAG: f64 = 1.6;
/// Grain pool (IO-A4).
pub(crate) const POOL: usize = 900;
/// Period of the tidal pulse of the vent glow, simulated seconds (IO-A5).
pub(crate) const TIDE_PERIOD: f64 = 20.0;
/// Plume filaments: grains leave in this many azimuthal rays (Pele's canopy shows
/// radial streaks in Voyager and Galileo images).
const RAYS: u32 = 18;
/// Simulated steps of eruption run in the first resize, so the plumes and rings already
/// show; in strides (the flight is exact and each grain lands at its exact point, so the
/// rings do not depend on the stride).
const WARM: usize = 2400;
const WARM_STRIDE: usize = 40;
const _: () = assert!(WARM % WARM_STRIDE == 0, "the stride divides the warm-up");
/// Launch zenith angles (rad): a fan from near-vertical (the umbrella's top, IO-A12) to
/// the 45° grains that land at the maximum range, each jittered by up to `FAN_JITTER`.
const FAN: [f64; 6] = [0.07, 0.3, 0.48, 0.62, 0.73, 0.8];
const FAN_JITTER: f64 = 0.025;
/// Landings drawn: only beyond this share of the 45° range (the ring's caustic).
const RING_INNER: f64 = 0.9;
/// The drawn plume threads, `(zenith rad, azimuth °)`: Pele's nested dome (a thin
/// near-vertical column, then two shells of arcs, evenly spaced round the vent).
const PELE_ARCS: [(f64, f64); 14] = [
    (0.1, 0.0),
    (0.1, 180.0),
    (0.48, 0.0),
    (0.48, 180.0),
    (0.48, 35.0),
    (0.48, 145.0),
    (0.48, 215.0),
    (0.48, 325.0),
    (0.76, 0.0),
    (0.76, 180.0),
    (0.76, 35.0),
    (0.76, 145.0),
    (0.76, 215.0),
    (0.76, 325.0),
];
/// Share of the Prometheus types' frost-ring dots drawn.
const FROST_KEEP: f64 = 0.55;
/// Thread sampling (dots between samples, at most), its level, the grains riding each
/// thread and their trail (share of the flight).
const ARC_STEP: f64 = 1.0;
const ARC_LEVEL: u8 = 170;
const ARC_HEADS: usize = 2;
const HEAD_TRAIL: f64 = 0.07;
/// Flight states (IO-A11 colours): `Hot` below this share of the grain's apex while
/// rising, `Cold` above `CONDENSE` of it or falling, `Sun` between.
const VENT_SHARE: f64 = 0.22;
const CONDENSE: f64 = 0.82;
/// Fallout: landings remembered per vent, and how slowly they fade (s, e-folding).
const FALLS: [usize; 3] = [1400, 260, 200];
const FALL_TAU: f64 = 30.0;
/// Surface stipple (IO-A13): keep probabilities at full light, by terrain.
const P_PLAINS: f64 = 0.035;
const P_SULFUR: f64 = 0.07;
const P_RUST: f64 = 0.06;
const P_FROST: f64 = 0.04;
pub(crate) const P_MAX: f64 = 0.15;
/// Stipple layer salts.
const SALT_GROUND: u64 = 0x10_5EED;
/// Active paterae (Loki and others): tiny `Hot` knots.
const PATERAE: usize = 9;
/// Inks (M17).
const LIMB_LIT: Ink = Ink::new(Role::Sun, 210);
const LIMB_DIM: Ink = Ink::new(Role::Sun, 120);
const PLAINS: Ink = Ink::new(Role::Sun, 140);
const SULFUR: Ink = Ink::new(Role::Sun, 200);
const RUST: Ink = Ink::new(Role::Hot, 160);
const FROST: Ink = Ink::new(Role::Star, 170);
const RED_FALL: Ink = Ink::new(Role::Hot, 200);
const WHITE_FALL: Ink = Ink::new(Role::Star, 160);
const STAR: Ink = Ink::new(Role::Star, 60);
const LAVA: Ink = Ink::new(Role::Hot, 255);
const LAVA_DIM: Ink = Ink::new(Role::Hot, 180);
const VENT_HOT: Ink = Ink::new(Role::Star, 255);
const VENT_WARM: Ink = Ink::new(Role::Sun, 255);

/// A vent: launch speed (km/s), zenith-angle range, launch rate (grains per simulated
/// second), where it sits on Io (unit normal, screen axes: x right, y down, z to the
/// viewer), its fallout colour and the phase of its breathing.
#[derive(Debug, Clone, Copy)]
struct Vent {
    v0: f64,
    zenith: (f64, f64),
    rate: f64,
    normal: [f64; 3],
    fall: Ink,
    phase: f64,
}

const VENTS: [Vent; 3] = [
    // Pele: close to the northern limb, its canopy silhouetted against space.
    Vent {
        v0: 1.0,
        zenith: (0.05, 0.9),
        rate: 10.0,
        normal: [-0.16, -0.96, 0.22],
        fall: RED_FALL,
        phase: 0.0,
    },
    // Prometheus: on the disc, seen more from above.
    Vent {
        v0: 0.5,
        zenith: (0.05, 0.9),
        rate: 5.0,
        normal: [0.4, -0.62, 0.68],
        fall: WHITE_FALL,
        phase: 2.1,
    },
    // A smaller Prometheus type (Amirani-like) to the west.
    Vent {
        v0: 0.42,
        zenith: (0.05, 0.9),
        rate: 4.0,
        normal: [-0.52, -0.58, 0.63],
        fall: WHITE_FALL,
        phase: 4.0,
    },
];

/// The Sun's direction: upper right and in front, so the disc is mostly lit and its
/// lower-left limb turns away into a soft terminator.
const SUN: [f64; 3] = [0.45, -0.5, 0.74];

/// Daylight at unit normal `n`, in `[0, 1]`: the lit fraction eases in over ≈ 60° of the
/// dot product with the Sun (a soft terminator), times gentle limb darkening. The
/// surface stipple's density follows it (IO-A13).
pub(crate) fn daylight(n: [f64; 3]) -> f64 {
    let s = normalize(SUN);
    let lam = dot(n, s);
    let day = ease::smoothstep(((lam + 0.25) / 1.0).clamp(0.0, 1.0));
    day * (0.7 + 0.3 * n[2].max(0.0).powf(0.35))
}

/// Ballistic range for a 45° launch (IO-A3, IO-A12).
pub(crate) fn range_45(v0: f64) -> f64 {
    plume::range(v0, G_IO, std::f64::consts::FRAC_PI_4)
}

/// Height (km) of a grain launched with vertical speed `vz` (km/s) after flight time `t`
/// (s): the exact parabola (IO-A11).
pub(crate) fn height(vz: f64, t: f64) -> f64 {
    vz * t - 0.5 * G_IO * t * t
}

/// Vent glow in `[0, 1]` at simulated time `t`: a pulse with Io's (compressed) orbital
/// period, the tidal flexing that heats it (IO-A5).
pub(crate) fn vent_glow(t: f64) -> f64 {
    0.55 + 0.45 * (TAU * t / TIDE_PERIOD).cos_det()
}

/// The eruption's breathing: the plume's linear size relative to its mean, at simulated
/// time `t` (IO-A8). Launch speed varies by up to ±4 %, so the size (∝ v²) by ≈ ±8 %.
pub(crate) fn plume_scale(t: f64, phase: f64) -> f64 {
    let s = 1.0 + 0.03 * (TAU * t / 7.3 + phase).sin_det() + 0.012 * (TAU * t / 2.9).sin_det();
    s * s
}

/// Patera output in `[0, 1]` for patera `k` at time `t`: smooth flicker (IO-A8).
fn lava_flicker(k: usize, t: f64) -> f64 {
    let n = value_noise(0x1A7A ^ k as u64, 2.3 * t, 7.1 * k as f64, 1.0);
    ease::smoothstep(n)
}

/// The role of a grain in flight (IO-A11 colours): `Hot` near the vent while rising,
/// `Sun` while rising, `Cold` near its apex and once falling (condensed SO₂).
pub(crate) fn flight_role(z: f64, vz: f64, apex: f64) -> Role {
    if vz <= 0.0 || z >= CONDENSE * apex {
        Role::Cold
    } else if z < VENT_SHARE * apex {
        Role::Hot
    } else {
        Role::Sun
    }
}

/// Io's radius in dots at 200×50, at and above which every grain is drawn.
const FULL_R: f64 = 190.0;

/// Whether the grain or landing with fixed bits `key` is among the share `keep` drawn at
/// this size (a fixed per-item threshold, so nothing flickers).
fn kept(key: u32, keep: f64) -> bool {
    keep >= 1.0 || hash01(u64::from(key) ^ 0x6A11) < keep
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Fractal value noise in `[0, 1]`: `octaves` octaves, each rotated and offset off the
/// lattice so no grid line survives the sum.
fn fbm(salt: u64, x: f64, y: f64, octaves: u64) -> f64 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let (mut u, mut v) = (x + 0.37, y + 0.61);
    for o in 0..octaves {
        sum += amp * value_noise(salt ^ (o << 12), u, v, 1.0);
        norm += amp;
        amp *= 0.5;
        // Rotate by ≈ 37° and double: the next octave's lattice lies across this one's.
        (u, v) = (1.6 * u - 1.2 * v + 5.3, 1.2 * u + 1.6 * v + 2.9);
    }
    sum / norm
}

/// Io's terrain under unit normal `n`: its stipple ink and keep probability at full
/// light: sulfur-yellow plains with brighter patches, white SO₂ frost, rust-red
/// highlands and the redder poles.
fn terrain(n: [f64; 3]) -> (Ink, f64) {
    let lon = n[0].atan2(n[2]);
    let lat = (-n[1]).asin();
    let (u, w) = (lon * 4.0, lat * 4.0);
    let frost = fbm(0x13, 1.1 * u + 7.0, 1.1 * w, 2);
    if frost > 0.72 {
        return (FROST, P_FROST);
    }
    let mottle = fbm(0x10, 2.0 * u, 2.0 * w, 2);
    let polar = ease::smoothstep(((lat.abs() - 0.75) / 0.5).clamp(0.0, 1.0));
    let rust = fbm(0x17, 0.8 * u - 3.0, w + 1.0, 2) + 0.25 * polar + 0.18 * (mottle - 0.5);
    if rust > 0.72 {
        (RUST, P_RUST)
    } else if fbm(0x19, 1.4 * u + 2.0, 1.4 * w - 1.0, 2) > 0.56 {
        (SULFUR, P_SULFUR)
    } else {
        (PLAINS, P_PLAINS)
    }
}

/// A vent's local frame: up, and two ground axes.
#[derive(Debug, Clone, Copy)]
struct Frame {
    up: [f64; 3],
    e1: [f64; 3],
    e2: [f64; 3],
}

impl Frame {
    fn of(normal: [f64; 3]) -> Self {
        let up = normalize(normal);
        let e1 = normalize(cross([0.0, 0.0, 1.0], up));
        let e2 = cross(up, e1);
        Self { up, e1, e2 }
    }

    /// Io-radius position of local point `(a, b, h)` km (drawn ×EXAG).
    fn place(&self, a: f64, b: f64, h: f64) -> [f64; 3] {
        let k = EXAG / IO_KM;
        std::array::from_fn(|i| self.up[i] * (1.0 + h * k) + (self.e1[i] * a + self.e2[i] * b) * k)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct View {
    /// Io's centre and radius, dots.
    cx: f64,
    cy: f64,
    r: f64,
}

impl View {
    fn to_dot(self, p: [f64; 3]) -> (f64, f64) {
        (self.cx + self.r * p[0], self.cy + self.r * p[1])
    }

    /// Whether 3-D point `p` (Io radii) is hidden behind Io.
    fn hidden(p: [f64; 3]) -> bool {
        p[2] < 0.0 && p[0] * p[0] + p[1] * p[1] < 1.0
    }

    /// Dots per km of plume (the ×EXAG scale).
    fn dots_per_km(self) -> f64 {
        self.r * EXAG / IO_KM
    }
}

/// A landed grain: where (vent-local km), when (simulated s), and how long its stipple
/// dot lasts (`−τ·ln u` for a fixed uniform `u`, so the ring fades as `e^{−age/τ}`).
#[derive(Debug, Clone, Copy, Default)]
struct Fall {
    a: f32,
    b: f32,
    at: f32,
    life: f32,
}

/// A fixed-capacity ring of landings (GEN-R11: no allocation after construction).
#[derive(Debug, Clone)]
struct Falls {
    buf: Vec<Fall>,
    next: usize,
    len: usize,
}

impl Falls {
    fn new(cap: usize) -> Self {
        Self {
            buf: vec![Fall::default(); cap],
            next: 0,
            len: 0,
        }
    }

    fn push(&mut self, f: Fall) {
        let cap = self.buf.len();
        self.buf[self.next] = f;
        self.next = (self.next + 1) % cap;
        self.len = (self.len + 1).min(cap);
    }

    fn iter(&self) -> impl Iterator<Item = &Fall> {
        self.buf[..self.len].iter()
    }
}

/// A stipple dot of the surface, fixed per resize.
#[derive(Debug, Clone, Copy)]
struct Speck {
    x: u16,
    y: u16,
    ink: Ink,
}

#[derive(Debug)]
pub struct IoVolcanoes {
    t: f64,
    view: View,
    frames: [Frame; 3],
    plume: Plume,
    /// Grains owed to each vent (fractional launches carried over).
    owed: [f64; 3],
    falls: [Falls; 3],
    /// The surface stipple and the stars, fixed per resize.
    specks: Vec<Speck>,
    /// Limb arc: the visible run of angles (start, step, count).
    limb: (f64, f64, usize),
    /// Active paterae: dot position, on screen and lit.
    paterae: [(f64, f64, bool); PATERAE],
    dots: Dots,
    warmed: bool,
    /// Share of grains and landings drawn at this size.
    keep: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl IoVolcanoes {
    fn new() -> Self {
        Self {
            t: 0.0,
            view: View::default(),
            frames: VENTS.map(|v| Frame::of(v.normal)),
            plume: Plume::with_capacity(POOL),
            owed: [0.0; 3],
            falls: FALLS.map(Falls::new),
            specks: Vec::new(),
            limb: (0.0, 0.0, 0),
            paterae: [(0.0, 0.0, false); PATERAE],
            dots: Dots::default(),
            keep: 1.0,
            warmed: false,
            dirty: false,
        }
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        self.t += dt;
        for (k, vent) in VENTS.iter().enumerate() {
            self.owed[k] += vent.rate * dt;
            while self.owed[k] >= 1.0 {
                self.owed[k] -= 1.0;
                let breathe = plume_scale(self.t, vent.phase).sqrt();
                let v0 = vent.v0 * breathe * rng.random_range(0.985..1.0);
                // A symmetric fan of launch angles, a little jittered, so the arcs nest
                // into a crisp dome and land in a thin ring.
                let k_fan = rng.random_range(0..FAN.len());
                let z = (FAN[k_fan] + rng.random_range(-FAN_JITTER..FAN_JITTER))
                    .clamp(vent.zenith.0, vent.zenith.1) as f32;
                let zen = (z, z);
                if !self.plume.launch_rays(rng, k as u8, v0 as f32, zen, RAYS) {
                    self.owed[k] = 0.0;
                    break;
                }
            }
        }
        let (falls, t) = (&mut self.falls, self.t);
        self.plume.step((dt * LAPSE) as f32, G_IO as f32, |p| {
            let k = usize::from(p.src).min(VENTS.len() - 1);
            // A fixed uniform per landing from its exact point: the ring fades as
            // e^{−age/τ} without any per-frame randomness.
            let u = hash01(cell_key((p.pos[0] * 64.0) as i32, (p.pos[1] * 64.0) as i32)).max(1e-6);
            falls[k].push(Fall {
                a: p.pos[0],
                b: p.pos[1],
                at: t as f32,
                life: (-FALL_TAU * u.ln()) as f32,
            });
        });
    }

    /// Io's unit normal under dot `(x, y)`, if the dot is on the disc.
    fn io_normal(&self, x: f64, y: f64) -> Option<[f64; 3]> {
        let v = self.view;
        let nx = (x - v.cx) / v.r;
        let ny = (y - v.cy) / v.r;
        let z2 = 1.0 - nx * nx - ny * ny;
        (z2 > 0.0).then(|| [nx, ny, z2.sqrt()])
    }

    /// The fixed surface stipple (IO-A13): a dot is kept with probability
    /// `p_terrain · daylight` against its fixed threshold. The cheap upper bound is
    /// tested first, so the terrain noise is sampled for at most `P_MAX` of the disc.
    fn build_surface(&mut self) {
        self.specks.clear();
        let ds = self.dots.dot_size();
        let (w, h) = (i32::from(ds.width()), i32::from(ds.height()));
        let v = self.view;
        let y0 = (v.cy - v.r).floor().max(0.0) as i32;
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (f64::from(x), f64::from(y));
                let Some(n) = (if y >= y0 {
                    self.io_normal(fx, fy)
                } else {
                    None
                }) else {
                    // Sparse stars in the sky.
                    if hash01(0x5E ^ cell_key(x, y)) < 0.0012 {
                        self.specks.push(Speck {
                            x: x as u16,
                            y: y as u16,
                            ink: STAR,
                        });
                    }
                    continue;
                };
                let light = daylight(n);
                if !stipple_keep(SALT_GROUND, x, y, P_MAX * light) {
                    continue;
                }
                let (ink, p) = terrain(n);
                if stipple_keep(SALT_GROUND, x, y, p * light) {
                    self.specks.push(Speck {
                        x: x as u16,
                        y: y as u16,
                        ink,
                    });
                }
            }
        }
        // The limb: the arc of angles whose points lie in the frame.
        let n = (TAU * v.r).ceil().max(8.0) as usize;
        let step = TAU / n as f64;
        let inside = |a: f64| {
            let (s, c) = a.sin_cos_det();
            let (x, y) = (v.cx + v.r * c, v.cy + v.r * s);
            (-1.0..f64::from(w) + 1.0).contains(&x) && (-1.0..f64::from(h) + 1.0).contains(&y)
        };
        // Start at the top of the disc (always in frame) and walk both ways.
        let top = -std::f64::consts::FRAC_PI_2;
        let mut lo = 0;
        while lo < n / 2 && inside(top - (lo + 1) as f64 * step) {
            lo += 1;
        }
        let mut hi = 0;
        while hi < n / 2 && inside(top + (hi + 1) as f64 * step) {
            hi += 1;
        }
        self.limb = (top - lo as f64 * step, step, lo + hi + 1);
        // Active paterae: hashed spots on the visible, lit hemisphere.
        let mut k = 0;
        for i in 0..64_u64 {
            if k == PATERAE {
                break;
            }
            let hh = |s: u64| 2.0 * hash01(0x9A7E ^ (i << 8) ^ s) - 1.0;
            let c = normalize([hh(1), hh(2), hh(3)]);
            if c[2] < 0.2 || daylight(c) < 0.3 {
                continue;
            }
            let (x, y) = v.to_dot(c);
            self.paterae[k] = (
                x,
                y,
                (0.0..f64::from(w)).contains(&x) && (0.0..f64::from(h)).contains(&y),
            );
            k += 1;
        }
        for p in &mut self.paterae[k..] {
            p.2 = false;
        }
    }

    /// Every grain's position as `(dot, z km, vz km/s, apex km)` at `tau` real seconds
    /// before now (clamped to its launch), for vent-local grain `g` (IO-A11: the exact
    /// parabola run backwards).
    #[cfg(test)]
    fn grain_at(&self, g: &plume::Grain, tau: f64) -> Option<((f64, f64), f64, f64, f64)> {
        let k = usize::from(g.src).min(VENTS.len() - 1);
        let tau = tau.min(f64::from(g.age));
        let (vz, z) = (f64::from(g.vel[2]), f64::from(g.pos[2]));
        let apex = z + vz * vz / (2.0 * G_IO);
        let zt = z - vz * tau - 0.5 * G_IO * tau * tau;
        let vzt = vz + G_IO * tau;
        let a = f64::from(g.pos[0]) - f64::from(g.vel[0]) * tau;
        let b = f64::from(g.pos[1]) - f64::from(g.vel[1]) * tau;
        let p = self.frames[k].place(a, b, zt.max(0.0));
        if View::hidden(p) {
            return None;
        }
        Some((self.view.to_dot(p), zt, vzt, apex))
    }

    /// Dot, height (km) and vertical speed of a grain of vent `k` launched with speeds
    /// `(vh, vz)` (km/s) along azimuth `(cos, sin)`, `u` real seconds into its flight: the
    /// exact parabola (IO-A11). `None` behind Io.
    fn arc_at(
        &self,
        k: usize,
        (vh, vz): (f64, f64),
        (ca, sa): (f64, f64),
        u: f64,
    ) -> Option<((f64, f64), f64, f64)> {
        let z = height(vz, u);
        let d = vh * u;
        let p = self.frames[k].place(d * ca, d * sa, z.max(0.0));
        (!View::hidden(p)).then(|| (self.view.to_dot(p), z, vz - G_IO * u))
    }

    /// One plume thread (IO-A11, IO-A12): the whole parabola of a grain launched at `v0`,
    /// zenith `zen`, azimuth `az_deg`, sampled at most `ARC_STEP` dots apart and joined by
    /// lines (a smooth curve), coloured `Hot` → `Sun` → `Cold` by the state in flight;
    /// `ARC_HEADS` bright grains ride it, each with a short trail.
    fn draw_arc(&mut self, k: usize, v0: f64, zen: f64, az_deg: f64, i: usize) {
        let (sz, cz) = zen.sin_cos_det();
        let speeds = (v0 * sz, v0 * cz);
        let (sa, ca) = az_deg.to_radians().sin_cos_det();
        let dir = (ca, sa);
        let flight = 2.0 * speeds.1 / G_IO;
        let apex = speeds.1 * speeds.1 / (2.0 * G_IO);
        let reach = self.view.dots_per_km() * (speeds.0 * flight + 2.0 * apex);
        let n = ((reach / ARC_STEP).ceil() as usize).clamp(8, 4000);
        let ink_at = |z: f64, vz: f64, level: u8| {
            let role = flight_role(z, vz, apex);
            // The falling limbs fade, so the dome's top reads above the curtains.
            let level = if vz < 0.0 && z < 0.6 * apex {
                (f64::from(level) * 0.6) as u8
            } else {
                level
            };
            Ink::new(role, level)
        };
        let mut prev: Option<(f64, f64)> = None;
        for j in 0..=n {
            let u = flight * j as f64 / n as f64;
            match self.arc_at(k, speeds, dir, u) {
                Some((pt, z, vz)) => {
                    if let Some(q) = prev {
                        let ink = ink_at(z, vz, ARC_LEVEL);
                        self.dots.line(q.0, q.1, pt.0, pt.1, ink);
                    }
                    prev = Some(pt);
                }
                None => prev = None,
            }
        }
        // Grains riding the thread, evenly spaced in flight time.
        let offset = hash01(0xA2C ^ ((k as u64) << 8) ^ i as u64);
        for h in 0..ARC_HEADS {
            let phase = (self.t * LAPSE / flight + h as f64 / ARC_HEADS as f64 + offset).fract();
            let u1 = phase * flight;
            let u0 = (u1 - HEAD_TRAIL * flight).max(0.0);
            let mut prev: Option<(f64, f64)> = None;
            for s in 0..=4 {
                let u = u0 + (u1 - u0) * f64::from(s) / 4.0;
                if let Some((pt, z, vz)) = self.arc_at(k, speeds, dir, u) {
                    if let Some(q) = prev {
                        self.dots.line(q.0, q.1, pt.0, pt.1, ink_at(z, vz, 255));
                    }
                    prev = Some(pt);
                }
            }
        }
    }

    fn compose(&mut self) {
        self.dots.clear();
        for s in &self.specks {
            self.dots.set(i32::from(s.x), i32::from(s.y), s.ink);
        }
        let v = self.view;
        // Pele's fallout ring: each live landing one stipple dot.
        let now = self.t as f32;
        for (k, falls) in self.falls.iter().enumerate().take(1) {
            let f = self.frames[k];
            let (fall, inv_range) = (VENTS[k].fall, 1.0 / range_45(VENTS[k].v0));
            for l in falls.iter() {
                if now - l.at >= l.life
                    || !kept(l.a.to_bits() ^ l.b.to_bits().rotate_left(16), self.keep)
                {
                    continue;
                }
                let p = f.place(f64::from(l.a), f64::from(l.b), 0.0);
                if p[2] < 0.0 {
                    continue;
                }
                let (x, y) = v.to_dot(p);
                // Only the caustic at the maximum range (Galileo's sharp-edged Pele ring):
                // a thin, crisp arc of stipple.
                let rho = f64::from(l.a).hypot(f64::from(l.b)) * inv_range;
                if rho > RING_INNER {
                    self.dots.plot(x, y, fall);
                }
            }
        }
        // The limb: one continuous thin arc, dimmer on the night side.
        let (a0, step, n) = self.limb;
        let mut prev: Option<(f64, f64)> = None;
        for i in 0..n {
            let a = a0 + i as f64 * step;
            let (s, c) = a.sin_cos_det();
            let pt = (v.cx + v.r * c, v.cy + v.r * s);
            let ink = if daylight([c, s, 0.0]) > 0.25 {
                LIMB_LIT
            } else {
                LIMB_DIM
            };
            if let Some(q) = prev {
                self.dots.line(q.0, q.1, pt.0, pt.1, ink);
            }
            prev = Some(pt);
        }
        // Active paterae: tiny knots, more dots and brighter as their output rises.
        for (i, &(x, y, on)) in self.paterae.iter().enumerate() {
            if !on {
                continue;
            }
            let out = lava_flicker(i, self.t);
            let ink = if out > 0.5 { LAVA } else { LAVA_DIM };
            let (x, y) = (x.round() as i32, y.round() as i32);
            let n = 1 + (3.0 * out).round() as usize;
            for &(dx, dy) in &[(0, 0), (1, 0), (0, 1), (-1, 0)][..n.min(4)] {
                self.dots.set(x + dx, y + dy, ink);
            }
        }
        // The plumes: a fixed fan of exact parabolas per vent (the umbrella's nested
        // dome), each sampled at dot resolution and coloured by the state in flight, with
        // bright grains riding them.
        let pele = VENTS[0].v0 * plume_scale(self.t, VENTS[0].phase).sqrt();
        for (i, &(zen, az)) in PELE_ARCS.iter().enumerate() {
            self.draw_arc(0, pele, zen, az, i);
        }
        // The Prometheus types, seen from above: their thin SO₂ frost ring (a sparse
        // stipple ellipse at the 45° range, foreshortened by the globe) round a bright
        // `Hot` vent knot.
        let per_km = v.dots_per_km();
        for (k, vent) in VENTS.iter().enumerate().skip(1) {
            let f = self.frames[k];
            let r = range_45(vent.v0 * plume_scale(self.t, vent.phase).sqrt());
            let n = ((TAU * r * per_km).ceil() as usize).clamp(12, 2000);
            for i in 0..n {
                if hash01(0xF207 ^ ((k as u64) << 16) ^ i as u64) >= FROST_KEEP {
                    continue;
                }
                let (s, c) = (TAU * i as f64 / n as f64).sin_cos_det();
                let p = f.place(r * c, r * s, 0.0);
                if p[2] >= 0.0 {
                    let (x, y) = v.to_dot(p);
                    self.dots.plot(x, y, vent.fall);
                }
            }
        }
        // The vents, pulsing with the tide (IO-A5): Pele one dot, the others small knots.
        let glow = vent_glow(self.t) > 0.55;
        for (k, f) in self.frames.iter().enumerate() {
            let p = f.place(0.0, 0.0, 0.0);
            if p[2] < 0.0 {
                continue;
            }
            let (x, y) = v.to_dot(p);
            if k == 0 {
                self.dots
                    .plot(x, y, if glow { VENT_HOT } else { VENT_WARM });
            } else {
                let (x, y) = (x.round() as i32, y.round() as i32);
                let core = if glow { VENT_WARM } else { LAVA };
                self.dots.set(x, y, core);
                for (dx, dy) in [(1, 0), (-1, 0), (0, -1)] {
                    self.dots.set(x + dx, y + dy, LAVA);
                }
            }
        }
    }
}

impl Generator for IoVolcanoes {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.dots.resize(size);
        let ds = self.dots.dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        let r = (0.95 * h).min(0.65 * w).max(1.0);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.5 * h + r,
            r,
        };
        if !self.warmed {
            self.warmed = true;
            for _ in 0..WARM / WARM_STRIDE {
                self.step(SIM_DT * WARM_STRIDE as f64, &mut ctx.rng);
            }
            // Keep the landings' ages: shift them so the scene clock starts at 0.
            let shift = self.t as f32;
            for falls in &mut self.falls {
                for l in &mut falls.buf {
                    l.at -= shift;
                }
            }
            self.t = 0.0;
        }
        self.build_surface();
        // Smaller screens draw a fixed share of the grains and landings, so the plume keeps
        // its threads instead of filling in.
        self.keep = (r / FULL_R).clamp(0.35, 1.0);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt, &mut ctx.rng);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.compose();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    fn built(w: u16, h: u16, secs: f64) -> IoVolcanoes {
        let mut g = IoVolcanoes::new();
        let mut ctx = Ctx::new(7, Size::new(w, h));
        g.resize(Size::new(w, h), &mut ctx);
        g.prepare();
        for _ in 0..(secs / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        g
    }

    #[test]
    fn io_a1_apex_matches_the_closed_form() {
        // Vertical launch: v0²/(2g).
        for vent in VENTS {
            let want = vent.v0 * vent.v0 / (2.0 * G_IO);
            assert!((plume::apex(vent.v0, G_IO, 0.0) - want).abs() < 1e-9 * want);
            // The parabola peaks there, at t = v0/g.
            assert!((height(vent.v0, vent.v0 / G_IO) - want).abs() < 1e-9 * want);
        }
        // Pele ≈ 280 km, Prometheus ≈ 70 km.
        assert!((plume::apex(1.0, G_IO, 0.0) - 278.0).abs() < 5.0);
    }

    #[test]
    fn io_a2_pele_is_at_least_three_times_taller() {
        let tallest = |k: usize| {
            let mut rng = SplitMix64::new(9);
            let mut p = Plume::with_capacity(400);
            for _ in 0..400 {
                p.launch(
                    &mut rng,
                    0,
                    VENTS[k].v0 as f32,
                    (0.0, VENTS[k].zenith.1 as f32),
                );
            }
            let mut top = 0.0f32;
            for _ in 0..2000 {
                p.step(1.0, G_IO as f32, |_| {});
                top = p.iter().map(|g| g.pos[2]).fold(top, f32::max);
            }
            top
        };
        let (pele, prometheus) = (tallest(0), tallest(1));
        assert!(pele >= 3.0 * prometheus, "{pele} vs {prometheus}");
    }

    /// The densest annulus of vent `k`'s remembered landings (km from the vent).
    fn ring_radius(g: &IoVolcanoes, k: usize) -> f64 {
        let span = 1.5 * range_45(VENTS[k].v0);
        let mut bins = [0usize; 30];
        for l in g.falls[k].iter() {
            let r = f64::from(l.a).hypot(f64::from(l.b)) / span;
            if r < 1.0 {
                bins[(r * 30.0) as usize] += 1;
            }
        }
        let best = (0..30).max_by_key(|&b| bins[b]).unwrap();
        (best as f64 + 0.5) / 30.0 * span
    }

    #[test]
    fn io_a3_deposits_ring_at_the_45_degree_range() {
        let g = built(80, 24, 0.0);
        for (k, vent) in VENTS.iter().enumerate() {
            let ring = ring_radius(&g, k);
            let want = range_45(vent.v0);
            assert!(
                (ring / want - 1.0).abs() < 0.15,
                "vent {k}: ring {ring} vs {want}"
            );
        }
    }

    #[test]
    fn io_a4_the_pool_never_exceeds_capacity() {
        let mut g = IoVolcanoes::new();
        let mut ctx = Ctx::new(4, Size::new(80, 24));
        for _ in 0..1200 {
            g.step(0.25, &mut ctx.rng);
            assert!(g.plume.len() <= POOL);
        }
        assert_eq!(g.plume.capacity(), POOL);
        // Over a hundred grains in flight at steady state (each a long thread), with room
        // to spare.
        assert!(
            g.plume.len() > 100 && g.plume.len() < POOL,
            "{}",
            g.plume.len()
        );
    }

    #[test]
    fn io_a5_vent_glow_pulses_with_the_tidal_period() {
        for k in 0..40 {
            let t = f64::from(k) * 0.37;
            assert!((vent_glow(t) - vent_glow(t + TIDE_PERIOD)).abs() < 1e-9);
        }
        assert!(vent_glow(0.0) > 0.9 && vent_glow(0.5 * TIDE_PERIOD) < 0.2);
    }

    #[test]
    fn io_a8_the_plume_breathes_and_the_lava_flickers() {
        let sizes: Vec<f64> = (0..100)
            .map(|i| plume_scale(f64::from(i) * 0.1, 0.0))
            .collect();
        let (lo, hi) = sizes
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), &s| (a.min(s), b.max(s)));
        assert!(hi / lo > 1.08 && hi / lo < 1.25, "breathing {lo}..{hi}");
        let changed = (0..PATERAE)
            .filter(|&k| (lava_flicker(k, 1.0) - lava_flicker(k, 1.6)).abs() > 0.1)
            .count();
        assert!(changed >= PATERAE / 3, "{changed} paterae flicker");
        // The plume's top stays in the frame through the breathing.
        for t in [1.0, 2.5, 4.0, 5.5] {
            let g = built(120, 40, t);
            let top = g
                .plume
                .iter()
                .filter_map(|p| g.grain_at(p, 0.0))
                .map(|q| q.0.1)
                .fold(f64::MAX, f64::min);
            assert!(top >= 0.0, "t={t}: plume top at dot row {top:.1}");
        }
    }

    #[test]
    fn io_a9_the_stipple_thins_across_a_soft_terminator() {
        // M17: the light reaches the ground as stipple density, falling to the
        // lower-left limb, with no hard terminator.
        let n = |x: f64, y: f64| [x, y, (1.0 - x * x - y * y).sqrt()];
        assert!(daylight(n(-0.7, 0.3)) + 0.3 < daylight(n(0.2, -0.3)));
        assert!(daylight(n(0.0, 0.0)) > daylight(n(-0.9, 0.0)));
        let mut prev = daylight(n(-0.999, 0.0));
        for k in 1..180 {
            let a = (f64::from(k) - 90.0).to_radians();
            let l = daylight([a.sin_det(), 0.0, a.cos_det()]);
            assert!((l - prev).abs() < 0.03, "{k}: {prev} -> {l}");
            prev = l;
        }
    }

    #[test]
    fn io_a10_at_most_3_percent_half_block_cells() {
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let g = built(w, h, 2.0);
            let mut canvas = Canvas::new(Size::new(w, h), GlyphMode::Unicode);
            g.render(&mut canvas);
            let (mut lit, mut block) = (0, 0);
            for y in 0..i32::from(h) {
                for x in 0..i32::from(w) {
                    let c = canvas.get(x, y).unwrap();
                    let is_block = c.bg != Ink::BLANK || matches!(c.ch, '▀' | '▄' | '█');
                    if is_block || c.ch != ' ' {
                        lit += 1;
                        block += usize::from(is_block);
                    }
                }
            }
            assert!(lit > 200, "{w}x{h}: {lit} lit cells");
            assert!(
                block * 100 <= 3 * lit,
                "{w}x{h}: {block} of {lit} half blocks"
            );
        }
    }

    #[test]
    fn io_a11_grains_fly_exact_parabolas_and_land_at_the_ballistic_range() {
        let (v, zen) = (0.9_f64, 0.6_f64);
        let (vh, vz) = (v * zen.sin_det(), v * zen.cos_det());
        let mut p = Plume::with_capacity(1);
        let mut rng = SplitMix64::new(3);
        assert!(p.launch(&mut rng, 0, v as f32, (zen as f32, zen as f32)));
        let dt = 7.0_f32;
        let mut t = 0.0_f64;
        let mut landed = None;
        for _ in 0..400 {
            p.step(dt, G_IO as f32, |g| landed = Some(*g));
            if landed.is_some() {
                break;
            }
            t += f64::from(dt);
            let g = p.iter().next().unwrap();
            let want = height(vz, t);
            assert!(
                (f64::from(g.pos[2]) - want).abs() < 1e-3 * want.max(1.0),
                "t={t}: {} vs {want}",
                g.pos[2]
            );
            let across = f64::from(g.pos[0]).hypot(f64::from(g.pos[1]));
            assert!((across - vh * t).abs() < 1e-3 * (vh * t).max(1.0));
        }
        let g = landed.expect("the grain lands");
        let range = f64::from(g.pos[0]).hypot(f64::from(g.pos[1]));
        let want = plume::range(v, G_IO, zen);
        assert!((range / want - 1.0).abs() < 1e-3, "{range} vs {want}");
        assert!((want - v * v * (2.0 * zen).sin_det() / G_IO).abs() < 1e-9);
        // Coloured by state in flight: hot at the vent, rising sun, cold near the apex
        // and falling.
        let apex = vz * vz / (2.0 * G_IO);
        assert_eq!(flight_role(0.05 * apex, vz, apex), Role::Hot);
        assert_eq!(flight_role(0.5 * apex, 0.5 * vz, apex), Role::Sun);
        assert_eq!(flight_role(0.9 * apex, 0.1 * vz, apex), Role::Cold);
        assert_eq!(flight_role(0.3 * apex, -0.5 * vz, apex), Role::Cold);
    }

    #[test]
    fn io_a12_the_umbrella_top_is_at_the_apex_and_fallout_rings_the_vent() {
        let mut g = built(200, 50, 0.0);
        let mut ctx = Ctx::new(11, Size::new(200, 50));
        let per_km = g.view.dots_per_km();
        // The umbrella's top: the highest apex of any Pele grain (`z + v_z²/2g`, exact on
        // its parabola), against `v²/2g` for that grain's launch speed `v` (the eruption
        // breathes, so each grain is held to its own `v_max`).
        let (mut best, mut short) = (0.0_f64, 0.0_f64);
        for _ in 0..(6.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            for p in g.plume.iter().filter(|p| p.src == 0) {
                let z = f64::from(p.pos[2]);
                let vz = f64::from(p.vel[2]);
                let v2 = p
                    .vel
                    .iter()
                    .map(|&c| f64::from(c) * f64::from(c))
                    .sum::<f64>()
                    + 2.0 * G_IO * z;
                let full = v2 / (2.0 * G_IO);
                // No grain rises above the apex its launch speed allows.
                assert!(z <= full * (1.0 + 1e-4));
                let own = z + vz * vz / (2.0 * G_IO);
                if own > best {
                    (best, short) = (own, full - own);
                }
            }
        }
        assert!(
            short * per_km <= 1.0,
            "umbrella top {best:.1} km is {:.2} dot rows below v²/2g",
            short * per_km
        );
        let ring = ring_radius(&g, 0);
        assert!(
            (ring / range_45(VENTS[0].v0) - 1.0).abs() < 0.15,
            "ring {ring}"
        );
    }

    #[test]
    fn io_a13_sparse_surface_stipple_and_a_continuous_limb() {
        for (w, h) in [(80u16, 24u16), (200, 50), (500, 150)] {
            let mut g = IoVolcanoes::new();
            let mut ctx = Ctx::new(7, Size::new(w, h));
            g.resize(Size::new(w, h), &mut ctx);
            let ds = g.dots.dot_size();
            let mut disc = 0usize;
            for y in 0..i32::from(ds.height()) {
                for x in 0..i32::from(ds.width()) {
                    disc += usize::from(g.io_normal(f64::from(x), f64::from(y)).is_some());
                }
            }
            let ground = g.specks.iter().filter(|s| s.ink != STAR).count();
            assert!(
                ground as f64 <= P_MAX * disc as f64 && ground * 50 > disc,
                "{w}x{h}: {ground} of {disc} disc dots"
            );
            // The limb: walk its drawn dots in order; consecutive ones touch.
            let mut dots = Dots::new(Size::new(w, h));
            let (a0, step, n) = g.limb;
            let v = g.view;
            let pts: Vec<(f64, f64)> = (0..n)
                .map(|i| {
                    let (s, c) = (a0 + i as f64 * step).sin_cos_det();
                    (v.cx + v.r * c, v.cy + v.r * s)
                })
                .collect();
            for p in pts.windows(2) {
                dots.line(p[0].0, p[0].1, p[1].0, p[1].1, LIMB_LIT);
            }
            assert!(n > 20, "{w}x{h}: limb of {n} samples");
            for p in pts.windows(2) {
                let (a, b) = (p[0], p[1]);
                assert!((a.0 - b.0).hypot(a.1 - b.1) <= 1.5, "limb step too long");
            }
            // Every lit limb dot has a lit neighbour (no isolated dots, no gaps).
            let lit: Vec<(i32, i32)> = pts
                .iter()
                .map(|p| (p.0.round() as i32, p.1.round() as i32))
                .filter(|&(x, y)| dots.is_lit(x, y))
                .collect();
            for &(x, y) in &lit {
                let near = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                    .filter(|&(dx, dy)| (dx, dy) != (0, 0) && dots.is_lit(x + dx, y + dy))
                    .count();
                assert!(near >= 1, "{w}x{h}: isolated limb dot at {x},{y}");
            }
        }
    }
}
