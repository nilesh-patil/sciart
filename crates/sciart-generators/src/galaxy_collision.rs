//! Two disk galaxies in a close encounter (SPEC-002 `galaxy_collision`).
//!
//! Restricted three-body model after Toomre & Toomre (1972): two softened point-mass
//! cores on a bound eccentric orbit, integrated with kick-drift-kick leapfrog, carry
//! massless test stars that start on circular orbits in their host's disk. Units are
//! `G = 1`, disk radius `RD = 1`; one real second is `time_scale()` model time units,
//! chosen so the Kepler period of the cores is 24 s. The cycle starts at apocentre, so
//! the first pericentre falls at ≈ 12 s and the second close passage (a restricted
//! model cannot merge) near 36 s, just before the 40 s restart.
//!
//! The camera looks at the centre of mass from a seeded tilt and zooms smoothly so the
//! cores, disks and (a bounded part of) the tidal tails stay on screen.
//!
//! The picture is a short long exposure: each host has its own exposure buffer at
//! half-block pixel resolution; stars splat into it every step and it decays with
//! `EXPOSURE_TAU`, so a few thousand test stars read as smooth glowing disks. After the
//! first close approach a fixed sample of "cloud" stars lights up as Hα knots where the
//! stellar density is high (triggered star formation, stylised: the model has no gas).

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::geom::v3::{add, cross, dot, norm, scale, sub};
use sciart_core::particles::{gaussian, plot};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{Tone, ink, raster, rng};

pub static META: Meta = Meta {
    name: "galaxy_collision",
    category: Category::DeepCosmos,
    description: "Two spiral galaxies colliding and pulling out tidal tails",
    title: "Galaxy collision · tidal tails",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(GalaxyCollision::default())
}

const M1: f64 = 1.0;
const M2: f64 = 0.8;
const M: f64 = M1 + M2;
/// Disk radius and core softening (`ε = 0.3 RD`).
const RD: f64 = 1.0;
const EPS: f64 = 0.3 * RD;
/// Semi-major axis and eccentricity of the (unsoftened) core orbit.
const A: f64 = 3.5;
const E: f64 = 0.63;
/// Real-time Kepler period of the cores, cycle length, and largest real sub-step.
const PERIOD: f64 = 24.0;
const CYCLE: f64 = 40.0;
const MAX_SUB_DT: f64 = 1.0 / 120.0;
/// Test stars per cell and their bounds (the force loop is O(N), so a dense disk is cheap).
const STAR_DENSITY: f64 = 0.3;
const MIN_STARS: usize = 64;
const MAX_STARS: usize = 3000;
/// Inner edge of the stellar disks (the bulge is the nucleus kernel).
const R_MIN: f64 = 0.1 * RD;
/// Camera: easing time (s), room kept around each core for its disk, and how far
/// beyond that the tails may push the frame.
const CAMERA_TAU: f64 = 0.8;
const DISK_ROOM: f64 = 1.5 * RD;
const TAIL_ROOM: f64 = 2.5 * RD;

/// e-folding time of the exposure (s): long enough to smooth a disk of test stars, short
/// enough that the tails stay crisp while the camera zooms.
const EXPOSURE_TAU: f64 = 0.15;
/// Tail and bridge stars are also exposed back along their path over this many seconds
/// (± `TAIL_JITTER` of it per star), in `TAIL_SAMPLES` splats: the sparse streams need a
/// longer exposure than the disks.
const TAIL_EXPOSURE: f64 = 0.6;
const TAIL_JITTER: f64 = 0.3;
const TAIL_SAMPLES: usize = 6;
/// Light of a nucleus in stars, spread by `NUCLEUS` (weights sum to 1).
const BULGE: f32 = 3.0;
const NUCLEUS: [(f64, f64, f32); 9] = [
    (0.0, 0.0, 0.25),
    (-1.0, 0.0, 0.125),
    (1.0, 0.0, 0.125),
    (0.0, -1.0, 0.125),
    (0.0, 1.0, 0.125),
    (-1.0, -1.0, 0.0625),
    (1.0, -1.0, 0.0625),
    (-1.0, 1.0, 0.0625),
    (1.0, 1.0, 0.0625),
];
/// Tone curve: exposure (≈ stars per pixel) at the knee and at white.
const KNEE: f32 = 0.1;
const WHITE: f32 = 14.0;
/// Two-armed spiral pattern (GAL-A8): winding (radians of azimuth per e-fold in radius,
/// a pitch of ≈ 20°), pattern period (s), brightness contrast `1 ± ARM_CONTRAST`, and the
/// radii over which it fades out beyond the disk.
const ARM_WIND: f64 = 2.7;
const ARM_PERIOD: f64 = 30.0;
const ARM_CONTRAST: f64 = 0.75;
const ARM_R_IN: f64 = 1.0 * RD;
const ARM_R_OUT: f64 = 1.4 * RD;
/// Fraction of outer-disk stars that trace star-forming clouds, and where the outer
/// disk starts.
const CLOUD_FRACTION: f64 = 0.05;
const CLOUD_R: f64 = 0.35 * RD;
/// The burst starts when the cores first come this close, rises over `BURST_RISE` s and
/// decays with e-folding `BURST_DECAY` s.
const BURST_SEP: f64 = 2.2 * RD;
const BURST_RISE: f64 = 1.5;
const BURST_DECAY: f64 = 7.0;
/// Host exposure at which a cloud glows fully (the compression proxy saturates).
const KNOT_RHO: f32 = 1.2;
/// Knots dimmer than this are not drawn; a full knot lifts its orthogonal neighbours by
/// `KNOT_HALO` of white and its diagonal ones by `KNOT_DIAGONAL` of that.
const KNOT_MIN: f64 = 0.12;
const KNOT_HALO: f64 = 0.3;
const KNOT_DIAGONAL: f64 = 0.55;
/// Where the two hosts' light overlaps, the minority host's share from which its pixels
/// start to show through (stippled), and the fraction of pixels they take at an even
/// (½) share.
const MIX_FROM: f64 = 0.3;
const MIX_MAX: f64 = 0.3;
/// Host ramps, galaxy 1 and galaxy 2: false colour telling which galaxy a star came from.
const HOSTS: [Ramp; 2] = [Ramp::Ice, Ramp::Ember];
const SKY: Glyph = Glyph::new('·', '.');
const SKY_STARS: usize = 70;
/// ASCII stand-ins for a pixel pair, by the brighter pixel's level.
const ASCII_RAMP: [char; 5] = ['.', ':', '+', '*', '@'];

pub(crate) use sciart_core::geom::v3::V3;

/// Model time units per real second: the cores' Kepler period maps to `PERIOD` s.
pub(crate) fn time_scale() -> f64 {
    TAU * (A * A * A / M).sqrt() / PERIOD
}

/// Softened acceleration at offset `d = target − source` from a mass `m`:
/// `−m d / (|d|² + ε²)^{3/2}` (Plummer softening).
pub(crate) fn accel(d: V3, m: f64) -> V3 {
    let s2 = dot(d, d) + EPS * EPS;
    scale(d, -m / (s2 * s2.sqrt()))
}

/// Circular speed at radius `r` around a softened mass `m`:
/// `√(m r² / (r² + ε²)^{3/2})` (GAL-A2).
pub(crate) fn circular_speed(m: f64, r: f64) -> f64 {
    let s2 = r * r + EPS * EPS;
    (m * r * r / (s2 * s2.sqrt())).sqrt()
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Body {
    pos: V3,
    vel: V3,
    acc: V3,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Star {
    body: Body,
    /// 0 = galaxy 1, 1 = galaxy 2 (index into `HOSTS`).
    gal: u8,
    /// Traces a star-forming cloud: lights up as a knot after the close approach.
    cloud: bool,
}

/// Total energy of the softened core pair (GAL-A1).
#[cfg(test)]
pub(crate) fn core_energy(cores: &[Body; 2]) -> f64 {
    let kin =
        0.5 * M1 * dot(cores[0].vel, cores[0].vel) + 0.5 * M2 * dot(cores[1].vel, cores[1].vel);
    let d = sub(cores[1].pos, cores[0].pos);
    // Plummer pair potential, consistent with `accel`.
    kin - M1 * M2 / (dot(d, d) + EPS * EPS).sqrt()
}

/// Accelerations of both cores and all stars at the current positions.
fn update_accels(cores: &mut [Body; 2], stars: &mut ParticlePool<Star>) {
    let d = sub(cores[0].pos, cores[1].pos);
    cores[0].acc = accel(d, M2);
    cores[1].acc = accel(scale(d, -1.0), M1);
    let (p1, p2) = (cores[0].pos, cores[1].pos);
    for s in stars.iter_mut() {
        s.body.acc = add(
            accel(sub(s.body.pos, p1), M1),
            accel(sub(s.body.pos, p2), M2),
        );
    }
}

/// Unit spin axis at inclination `inc` from +z and azimuth `az`.
fn axis(inc: f64, az: f64) -> V3 {
    [
        inc.sin_det() * az.cos_det(),
        inc.sin_det() * az.sin_det(),
        inc.cos_det(),
    ]
}

/// Orthonormal disk basis `(e1, e2)` with `e1 × e2 = n`, so stars placed at
/// `cos φ e1 + sin φ e2` and moving towards increasing φ spin about `n`.
fn disk_basis(n: V3) -> (V3, V3) {
    let a = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let e1 = cross(a, n);
    let e1 = scale(e1, 1.0 / norm(e1));
    (e1, cross(n, e1))
}

/// Add weight `w` at continuous pixel position `(x, y)` (integers are pixel centres) of a
/// row-major `pw`-wide buffer, split bilinearly over the four nearest pixels: motion stays
/// smooth below a pixel and the weight is conserved in bounds (GAL-A5).
pub(crate) fn splat(buf: &mut [f32], pw: usize, x: f64, y: f64, w: f32) {
    if !(x.is_finite() && y.is_finite()) || pw == 0 {
        return;
    }
    let ph = buf.len() / pw;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    for (dx, dy, k) in [
        (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
        (1.0, 0.0, fx * (1.0 - fy)),
        (0.0, 1.0, (1.0 - fx) * fy),
        (1.0, 1.0, fx * fy),
    ] {
        let (px, py) = (x0 + dx, y0 + dy);
        // Saturating casts: anything huge fails the bounds check.
        if px >= 0.0 && py >= 0.0 && (px as usize) < pw && (py as usize) < ph {
            buf[py as usize * pw + px as usize] += w * k;
        }
    }
}

/// The dynamical state of one encounter (no screen knowledge).
#[derive(Debug, Clone)]
pub(crate) struct Encounter {
    cores: [Body; 2],
    /// Each host's disk basis `(e1, e2)`, for its spiral pattern.
    disks: [(V3, V3); 2],
    stars: ParticlePool<Star>,
    /// Real seconds since this encounter began.
    t: f64,
    /// When the cores first came within `BURST_SEP`: starts the star-formation burst.
    close: Option<f64>,
}

impl Encounter {
    /// A fresh encounter of `n` stars (see [`reset`](Self::reset)).
    pub(crate) fn new(n: usize, rng: &mut impl Rng) -> Self {
        let mut sim = Self {
            cores: [Body::default(); 2],
            disks: [([0.0; 3], [0.0; 3]); 2],
            stars: ParticlePool::with_capacity(n),
            t: 0.0,
            close: None,
        };
        sim.reset(n, rng);
        sim
    }

    /// Start over in place, reusing the star buffer (no allocation unless `n` outgrows
    /// it; GEN-R11): cores at apocentre of the bound orbit, `n` stars split ∝ mass on
    /// circular orbits. Galaxy 1 spins nearly prograde; galaxy 2 is inclined 40°–140°
    /// (possibly retrograde).
    pub(crate) fn reset(&mut self, n: usize, rng: &mut impl Rng) {
        let ra = A * (1.0 + E);
        let va = (M * (1.0 - E) / (A * (1.0 + E))).sqrt();
        let (r, v) = ([ra, 0.0, 0.0], [0.0, va, 0.0]);
        let cores = [
            Body {
                pos: scale(r, -M2 / M),
                vel: scale(v, -M2 / M),
                acc: [0.0; 3],
            },
            Body {
                pos: scale(r, M1 / M),
                vel: scale(v, M1 / M),
                acc: [0.0; 3],
            },
        ];
        let spins = [
            axis(
                rng.random_range(0.0..15f64.to_radians()),
                rng.random_range(0.0..TAU),
            ),
            axis(
                rng.random_range(40f64.to_radians()..140f64.to_radians()),
                rng.random_range(0.0..TAU),
            ),
        ];
        let n1 = ((n as f64) * M1 / M).round() as usize;
        let disks = spins.map(disk_basis);
        self.stars.clear();
        self.stars.set_capacity(n);
        for i in 0..n {
            let g = usize::from(i >= n1);
            let (host, m) = (cores[g], [M1, M2][g]);
            let (e1, e2) = disks[g];
            // Two trailing log-spiral arms (pitch ≈ 20°) with scatter; they wind up
            // under differential rotation, as real material arms would.
            let r = R_MIN + (RD - R_MIN) * rng.random_range(0.0f64..1.0);
            let arm = if rng.random_bool(0.5) { 0.0 } else { PI };
            let phi = arm - ARM_WIND * (r / R_MIN).ln() + gaussian(rng, 0.0, 0.35);
            let (c, s) = (phi.cos_det(), phi.sin_det());
            let radial = add(scale(e1, c), scale(e2, s));
            let tangent = add(scale(e1, -s), scale(e2, c));
            let thick = scale(spins[g], gaussian(rng, 0.0, 0.02 * RD));
            let offset = add(scale(radial, r), thick);
            let vc = circular_speed(m, norm(offset));
            let cloud = r > CLOUD_R && rng.random_bool(CLOUD_FRACTION);
            self.stars.spawn(Star {
                body: Body {
                    pos: add(host.pos, offset),
                    vel: add(host.vel, scale(tangent, vc)),
                    acc: [0.0; 3],
                },
                gal: g as u8,
                cloud,
            });
        }
        self.cores = cores;
        self.disks = disks;
        self.t = 0.0;
        self.close = None;
        update_accels(&mut self.cores, &mut self.stars);
    }

    /// Where star `s` sits in its host's two-armed spiral pattern: `(cos ψ, inside)`,
    /// `ψ = 2(φ + ARM_WIND·ln(r/R_MIN) − Ω_p t)` with `(r, φ)` its polar position in the
    /// host's disk plane and `Ω_p = 2π/ARM_PERIOD`; `inside` is 1 within the disk, falling
    /// smoothly to 0 by `ARM_R_OUT` (tails and bridge carry no pattern) (GAL-A8).
    ///
    /// The pattern is a density wave: stars brighten while they pass through it, so the
    /// arms persist while the stars' own initial arms wind up and blur.
    pub(crate) fn arm(&self, s: &Star) -> (f64, f64) {
        let g = usize::from(s.gal);
        let (e1, e2) = self.disks[g];
        let d = sub(s.body.pos, self.cores[g].pos);
        let (x, y) = (dot(d, e1), dot(d, e2));
        let r = x.hypot(y).max(R_MIN);
        let inside = 1.0 - ease::smoothstep((r - ARM_R_IN) / (ARM_R_OUT - ARM_R_IN));
        let psi = 2.0 * (y.atan2(x) + ARM_WIND * (r / R_MIN).ln() - TAU * self.t / ARM_PERIOD);
        (psi.cos_det(), inside)
    }

    /// Advance by `dt` real seconds: velocity-Verlet (kick-drift-kick) sub-stepped to
    /// ≤ `MAX_SUB_DT`, one force evaluation per sub-step (GAL-A1).
    pub(crate) fn step(&mut self, dt: f64) {
        // Also rejects NaN.
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let n = (dt / MAX_SUB_DT).ceil().max(1.0);
        let h = dt / n * time_scale();
        for _ in 0..n as usize {
            // Kick (half) + drift with the accelerations stored from the last step…
            let kick_drift = |b: &mut Body| {
                b.vel = add(b.vel, scale(b.acc, 0.5 * h));
                b.pos = add(b.pos, scale(b.vel, h));
            };
            self.cores.iter_mut().for_each(kick_drift);
            self.stars.iter_mut().for_each(|s| kick_drift(&mut s.body));
            // …then one force evaluation and the closing half kick.
            update_accels(&mut self.cores, &mut self.stars);
            let kick = |b: &mut Body| b.vel = add(b.vel, scale(b.acc, 0.5 * h));
            self.cores.iter_mut().for_each(kick);
            self.stars.iter_mut().for_each(|s| kick(&mut s.body));
        }
        self.t += dt;
        if self.close.is_none() && self.separation() < BURST_SEP {
            self.close = Some(self.t);
        }
    }

    fn separation(&self) -> f64 {
        norm(sub(self.cores[1].pos, self.cores[0].pos))
    }

    /// Star-formation burst envelope in `[0, 1]`: 0 until the cores first close in, then
    /// a smooth rise over `BURST_RISE` s and an exponential decay (GAL-A7).
    pub(crate) fn burst(&self) -> f64 {
        self.close.map_or(0.0, |t0| {
            let s = self.t - t0;
            ease::smoothstep(s / BURST_RISE) * (-s / BURST_DECAY).exp()
        })
    }
}

/// Seeded viewpoint: tilt of the orbital plane, roll, and a horizontal mirror.
#[derive(Debug, Clone, Copy, Default)]
struct View {
    cos_t: f64,
    sin_t: f64,
    cos_r: f64,
    sin_r: f64,
    flip: f64,
}

impl View {
    fn random(rng: &mut impl Rng) -> Self {
        let t = rng.random_range(18f64.to_radians()..40f64.to_radians());
        let r = rng.random_range(-12f64.to_radians()..12f64.to_radians());
        Self {
            cos_t: t.cos_det(),
            sin_t: t.sin_det(),
            cos_r: r.cos_det(),
            sin_r: r.sin_det(),
            flip: if rng.random_bool(0.5) { 1.0 } else { -1.0 },
        }
    }

    /// Screen-plane coordinates (model units, y up).
    fn project(&self, p: V3) -> (f64, f64) {
        let (x, y) = (p[0], p[1] * self.cos_t - p[2] * self.sin_t);
        (
            self.flip * (x * self.cos_r - y * self.sin_r),
            x * self.sin_r + y * self.cos_r,
        )
    }
}

/// Where the camera looks: the screen-plane point at the canvas centre (model units),
/// rows per model unit, and the canvas it frames.
#[derive(Debug, Clone, Copy)]
struct Frame {
    view: View,
    centre: (f64, f64),
    zoom: f64,
    size: Size,
}

impl Frame {
    /// Continuous pixel position of `p`, integers at pixel centres (a cell is one pixel
    /// wide and two tall).
    fn pixel(&self, p: V3) -> (f64, f64) {
        let (x, y) = self.view.project(p);
        let cx = f64::from(self.size.width()) / 2.0 + (x - self.centre.0) * self.zoom * ASPECT;
        let cy = f64::from(self.size.height()) / 2.0 - (y - self.centre.1) * self.zoom;
        (cx - 0.5, 2.0 * cy - 0.5)
    }
}

#[derive(Debug)]
pub struct GalaxyCollision {
    sim: Option<Encounter>,
    frame: Frame,
    /// Per-host exposure at pixel resolution (`w × 2h`, row-major).
    exposure: [Vec<f32>; 2],
    tone: Tone,
    pixels: Pixels,
    /// Distant field stars, normalized `[0, 1)²` (they do not zoom).
    sky: Vec<(f64, f64)>,
}

impl Default for GalaxyCollision {
    fn default() -> Self {
        Self {
            sim: None,
            frame: Frame {
                view: View::default(),
                centre: (0.0, 0.0),
                zoom: 1.0,
                size: Size::new(1, 1),
            },
            exposure: [Vec::new(), Vec::new()],
            tone: Tone::asinh(KNEE, WHITE),
            pixels: Pixels::default(),
            sky: Vec::new(),
        }
    }
}

fn star_count(size: Size) -> usize {
    ((size.area() as f64 * STAR_DENSITY) as usize).clamp(MIN_STARS, MAX_STARS)
}

/// ASCII stand-in for a pixel pair: a density char in the brighter pixel's ink.
fn ascii_pixel(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = raster::stronger(top, bottom);
    (ink.role != Role::Background).then(|| {
        let i = usize::from(ink.level) * ASCII_RAMP.len() / 256;
        (ASCII_RAMP[i], ink)
    })
}

impl GalaxyCollision {
    fn restart(&mut self, ctx: &mut Ctx) {
        let n = star_count(self.frame.size);
        match &mut self.sim {
            Some(sim) => sim.reset(n, &mut ctx.rng),
            None => self.sim = Some(Encounter::new(n, &mut ctx.rng)),
        }
        self.frame.view = View::random(&mut ctx.rng);
        self.snap_camera();
        // A new encounter starts from its own light, not the last one's afterglow.
        self.exposure.iter_mut().for_each(|e| e.fill(0.0));
        self.expose(3.0 * EXPOSURE_TAU);
    }

    /// Camera target `(centre, zoom)`: frame the cores with their disks, centred on the
    /// midpoint of the pair, widened (boundedly) by the stars' spread so tails show.
    fn target_frame(&self) -> ((f64, f64), f64) {
        let Some(sim) = &self.sim else {
            return ((0.0, 0.0), 1.0);
        };
        let view = self.frame.view;
        let [a, b] = sim.cores.map(|c| view.project(c.pos));
        let centre = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
        let cx = 0.5 * (a.0 - b.0).abs() + DISK_ROOM;
        let cy = 0.5 * (a.1 - b.1).abs() + DISK_ROOM;
        let (mut sx, mut sy) = (0.0, 0.0);
        for s in &sim.stars {
            let (x, y) = view.project(s.body.pos);
            sx += (x - centre.0) * (x - centre.0);
            sy += (y - centre.1) * (y - centre.1);
        }
        let n = sim.stars.len().max(1) as f64;
        let ex = cx.max(1.6 * (sx / n).sqrt()).min(cx + TAIL_ROOM);
        let ey = cy.max(1.6 * (sy / n).sqrt()).min(cy + TAIL_ROOM);
        let size = self.frame.size;
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let zx = ((w / 2.0 - 1.0).max(0.5)) / (ex * ASPECT);
        let zy = ((h / 2.0 - 0.5).max(0.5)) / ey;
        (centre, zx.min(zy))
    }

    fn snap_camera(&mut self) {
        (self.frame.centre, self.frame.zoom) = self.target_frame();
    }

    /// Expose for `dt` seconds: decay both buffers, then add every star's and both
    /// nuclei's light (a star holding still accumulates ≈ 1, brighter on a spiral arm).
    fn expose(&mut self, dt: f64) {
        let Some(sim) = &self.sim else { return };
        let (frame, pw) = (self.frame, usize::from(self.frame.size.width()));
        let keep = (-dt.max(0.0) / EXPOSURE_TAU).exp() as f32;
        let w = 1.0 - keep;
        for e in &mut self.exposure {
            e.iter_mut().for_each(|v| *v *= keep);
        }
        for (k, s) in sim.stars.iter().enumerate() {
            let (x, y) = frame.pixel(s.body.pos);
            let (wave, inside) = sim.arm(s);
            let lit = (1.0 + ARM_CONTRAST * inside * wave) as f32;
            let e = &mut self.exposure[usize::from(s.gal)];
            // Beyond the disk (tails and bridge) stars are sparse: each one's light is
            // spread back along its path over the last `TAIL_EXPOSURE` s, a longer exposure
            // for them alone, so a few hundred read as smooth streams rather than speckle
            // (GAL-A5: the weight is conserved either way).
            let spread = (1.0 - inside) as f32;
            splat(e, pw, x, y, w * lit * (1.0 - spread));
            if spread > 0.0 {
                // Each star's exposure has its own length and sample phase, so the ends
                // and samples of neighbouring streaks do not line up into a comb.
                let (u, phase) = (
                    rng::hash01(rng::cell_key(k as i32, 0)),
                    rng::hash01(rng::cell_key(k as i32, 1)),
                );
                let span = TAIL_EXPOSURE * (1.0 - TAIL_JITTER + 2.0 * TAIL_JITTER * u);
                let back = scale(s.body.vel, -span * time_scale());
                let k = w * lit * spread / TAIL_SAMPLES as f32;
                for j in 0..TAIL_SAMPLES {
                    let f = (j as f64 + phase) / TAIL_SAMPLES as f64;
                    let (x, y) = frame.pixel(add(s.body.pos, scale(back, f)));
                    splat(e, pw, x, y, k);
                }
            }
        }
        for (e, core) in self.exposure.iter_mut().zip(&sim.cores) {
            let (x, y) = frame.pixel(core.pos);
            for (dx, dy, k) in NUCLEUS {
                splat(e, pw, x + dx, y + dy, w * BULGE * k);
            }
        }
    }

    /// Brightness envelope: fade in after a restart and out before the next one.
    fn fade(&self) -> f64 {
        let t = self.sim.as_ref().map_or(0.0, |s| s.t);
        let x = (t / 1.5).min((CYCLE - t) / 2.0).clamp(0.0, 1.0);
        0.3 + 0.7 * x * x * (3.0 - 2.0 * x)
    }

    /// Tone-map the exposures onto the pixels, then light the knots (the M10 raster
    /// convention: `render` only blits).
    fn develop(&mut self) {
        let fade = self.fade();
        let pw = usize::from(self.frame.size.width());
        let (tone, [e1, e2]) = (&self.tone, &self.exposure);
        self.pixels.shade(|x, y| {
            let i = y as usize * pw + x as usize;
            let (a, b) = (e1[i], e2[i]);
            match tone.level((a + b) * fade as f32) {
                0 => Ink::BLANK,
                level => Ink::new(Role::Ramp(HOSTS[host_of(a, b, x, y)]), level),
            }
        });
        let Some(sim) = &self.sim else { return };
        let burst = sim.burst() * fade;
        if burst < KNOT_MIN {
            return;
        }
        for s in sim.stars.iter().filter(|s| s.cloud) {
            let (x, y) = self.frame.pixel(s.body.pos);
            let (px, py) = (x.round(), y.round());
            // `NaN as usize` is 0: reject non-finite positions as `splat` does.
            if !(px.is_finite() && py.is_finite()) || px < 0.0 || py < 0.0 || px as usize >= pw {
                continue;
            }
            let i = py as usize * pw + px as usize;
            let Some(&rho) = self.exposure[usize::from(s.gal)].get(i) else {
                continue;
            };
            // Clouds collapse where the gas is squeezed: on an arm's crest in the disk,
            // anywhere dense in the bridge and tails.
            let (wave, inside) = sim.arm(s);
            let squeeze = 1.0 - inside + inside * wave.max(0.0);
            let b = burst * squeeze * f64::from((rho / KNOT_RHO).min(1.0));
            if b >= KNOT_MIN {
                knot(&mut self.pixels, px as i32, py as i32, b);
            }
        }
    }
}

/// Which host's ramp a pixel with exposures `a` (host 1) and `b` (host 2) shows: the
/// brighter one, except where their light overlaps, where the fainter host's share of it
/// stipples through on a fixed per-pixel hash, so two discs crossing mix instead of
/// cutting each other along a hard edge.
fn host_of(a: f32, b: f32, x: i32, y: i32) -> usize {
    let (lo, hi) = (a.min(b), a.max(b));
    let share = f64::from(lo / (lo + hi).max(f32::MIN_POSITIVE));
    let show = MIX_MAX * ease::smoothstep((share - MIX_FROM) / (0.5 - MIX_FROM));
    let minority = usize::from(a > b);
    // Hashed per 2 × 2 clump, so the minority host shows as small patches of its disk
    // rather than single-pixel confetti.
    if show > rng::hash01(rng::cell_key(x.div_euclid(2), y.div_euclid(2))) {
        minority
    } else {
        1 - minority
    }
}

/// A star-forming knot of strength `b ∈ (0, 1]` at pixel `(x, y)` (GAL-A7): a pink Hα
/// core pixel, never dimming a brighter knot already there, in a round glow that
/// brightens its eight neighbours in their own colour (orthogonal ones more), so the knot
/// reads as an HII region lighting its disk rather than a pink `+` sprite.
fn knot(pixels: &mut Pixels, x: i32, y: i32, b: f64) {
    let core = ink::ramp(Ramp::HAlpha, 0.6 + 0.4 * b);
    if pixels
        .get(x, y)
        .is_some_and(|i| i.role != Role::Ramp(Ramp::HAlpha) || i.level < core.level)
    {
        pixels.set(x, y, core);
    }
    for (dx, dy) in [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (1, 1),
        (-1, 1),
        (1, -1),
    ] {
        let (px, py) = (x + dx, y + dy);
        let Some(under) = pixels.get(px, py) else {
            continue;
        };
        let k = if dx == 0 || dy == 0 {
            1.0
        } else {
            KNOT_DIAGONAL
        };
        let lift = KNOT_HALO * k * b * 255.0;
        let ink = if under.role == Role::Background {
            ink::ramp(Ramp::HAlpha, lift / 255.0)
        } else {
            Ink::new(under.role, ink::level(f64::from(under.level) + lift))
        };
        pixels.set(px, py, ink);
    }
}

impl Generator for GalaxyCollision {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.frame.size = size;
        if self.sky.is_empty() {
            self.sky = (0..SKY_STARS)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..1.0),
                    )
                })
                .collect();
        }
        self.pixels.resize(size);
        let px = self.pixels.pixel_size().area();
        for e in &mut self.exposure {
            e.clear();
            e.resize(px, 0.0);
        }
        let n = star_count(size);
        match &mut self.sim {
            // Fewer stars: keep the encounter running with a subset.
            Some(sim) if n <= sim.stars.len() => {
                sim.stars.set_capacity(n);
                self.snap_camera();
                self.expose(3.0 * EXPOSURE_TAU);
            }
            _ => self.restart(ctx),
        }
        self.develop();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let Some(sim) = &mut self.sim else { return };
        sim.step(dt);
        if sim.t >= CYCLE {
            // GAL-A4: a fresh, re-seeded encounter every 40 s.
            self.restart(ctx);
        } else {
            // Exponential camera easing (zoom in log space), exact for any dt.
            let (centre, zoom) = self.target_frame();
            let k = 1.0 - (-dt / CAMERA_TAU).exp();
            let f = &mut self.frame;
            f.centre.0 += (centre.0 - f.centre.0) * k;
            f.centre.1 += (centre.1 - f.centre.1) * k;
            f.zoom = (f.zoom.ln() + (zoom.ln() - f.zoom.ln()) * k).exp();
            self.expose(dt);
        }
        self.develop();
    }

    fn render(&self, canvas: &mut Canvas) {
        let size = self.frame.size;
        if canvas.size() != size {
            return;
        }
        let (wf, hf) = (f64::from(size.width()), f64::from(size.height()));
        for &(u, v) in &self.sky {
            plot(canvas, u * wf, v * hf, SKY, Ink::new(Role::Star, 60));
        }
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_pixel(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encounter(n: usize, seed: u64) -> Encounter {
        let mut ctx = Ctx::new(seed, Size::new(1, 1));
        Encounter::new(n, &mut ctx.rng)
    }

    /// A scene at `size`, seed 42, run for `t` seconds.
    fn scene_at(size: Size, t: f64) -> (GalaxyCollision, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = GalaxyCollision::default();
        g.resize(size, &mut ctx);
        for _ in 0..(t / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
        }
        (g, ctx)
    }

    /// The pixels' inks, row-major.
    fn pixel_inks(g: &GalaxyCollision) -> Vec<Ink> {
        let s = g.pixels.pixel_size();
        let (w, h) = (i32::from(s.width()), i32::from(s.height()));
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter_map(|(x, y)| g.pixels.get(x, y))
            .collect()
    }

    #[test]
    fn gal_a1_core_pair_conserves_energy_over_a_cycle() {
        let mut sim = encounter(0, 1);
        let e0 = core_energy(&sim.cores);
        assert!(e0 < 0.0, "the orbit must be bound");
        let mut worst: f64 = 0.0;
        let mut closest = (f64::INFINITY, 0.0);
        while sim.t < CYCLE {
            sim.step(SIM_DT);
            worst = worst.max(((core_energy(&sim.cores) - e0) / e0).abs());
            if sim.separation() < closest.0 && sim.t < 20.0 {
                closest = (sim.separation(), sim.t);
            }
        }
        assert!(worst < 0.01, "relative energy error {worst}");
        // Cinematic timing: first pericentre ≈ 12 s at ≈ 1.6 RD.
        assert!(
            (10.5..13.5).contains(&closest.1),
            "pericentre at {closest:?}"
        );
        assert!((1.0..2.2).contains(&closest.0), "pericentre at {closest:?}");
        // Same answer with the largest frame dt (sub-stepping keeps it stable).
        let mut big = encounter(0, 1);
        while big.t < CYCLE {
            big.step(0.25);
        }
        assert!(((core_energy(&big.cores) - e0) / e0).abs() < 0.01);
    }

    #[test]
    fn gal_a1_centre_of_mass_stays_at_rest() {
        let mut sim = encounter(0, 2);
        for _ in 0..1200 {
            sim.step(SIM_DT);
        }
        let com = add(scale(sim.cores[0].pos, M1), scale(sim.cores[1].pos, M2));
        let p = add(scale(sim.cores[0].vel, M1), scale(sim.cores[1].vel, M2));
        assert!(norm(com) < 1e-9 && norm(p) < 1e-12, "{com:?} {p:?}");
    }

    #[test]
    fn gal_a2_initial_star_speeds_are_softened_circular() {
        // Softened circular speed: v² / r = m r / (r² + ε²)^{3/2}.
        for r in [0.1, 0.5, 1.0] {
            let v = circular_speed(1.0, r);
            let a = norm(accel([r, 0.0, 0.0], 1.0));
            assert!((v * v / r - a).abs() < 1e-12, "r={r}");
        }
        let sim = encounter(400, 3);
        assert_eq!(sim.stars.len(), 400);
        let n1 = sim.stars.iter().filter(|s| s.gal == 0).count();
        assert_eq!(n1, (400.0 * M1 / M).round() as usize);
        for s in &sim.stars {
            let g = usize::from(s.gal);
            let host = sim.cores[g];
            let d = sub(s.body.pos, host.pos);
            let v = sub(s.body.vel, host.vel);
            let r = norm(d);
            assert!((R_MIN * 0.9..RD * 1.1).contains(&r), "r={r}");
            let expect = circular_speed([M1, M2][g], r);
            assert!(
                (norm(v) - expect).abs() < 1e-12,
                "|v|={} expected {expect}",
                norm(v)
            );
            // Purely tangential (up to the small disk thickness).
            assert!(dot(d, v).abs() < 0.1 * r * norm(v));
        }
    }

    #[test]
    fn gal_a3_tidal_tails_form_after_pericentre() {
        let far = |sim: &Encounter| {
            sim.stars
                .iter()
                .filter(|s| {
                    sim.cores
                        .iter()
                        .all(|c| norm(sub(s.body.pos, c.pos)) > 2.0 * RD)
                })
                .count()
        };
        for seed in [4, 5, 6] {
            let mut sim = encounter(600, seed);
            while sim.t < 6.0 {
                sim.step(SIM_DT);
            }
            assert_eq!(far(&sim), 0, "no tails before the encounter (seed {seed})");
            while sim.t < 20.0 {
                sim.step(SIM_DT);
            }
            let n = far(&sim);
            assert!(n >= 30, "only {n} stars in tails (seed {seed})");
        }
    }

    #[test]
    fn gal_a4_cycle_restarts_at_40_s() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(8, size);
        let mut g = GalaxyCollision::default();
        g.resize(size, &mut ctx);
        let ra = A * (1.0 + E);
        let mut steps = 0;
        while g.sim.as_ref().unwrap().t + SIM_DT < CYCLE - 1e-9 {
            g.update(SIM_DT, &mut ctx);
            steps += 1;
        }
        assert!((g.sim.as_ref().unwrap().separation() - ra).abs() > 0.1);
        g.update(SIM_DT, &mut ctx);
        g.update(SIM_DT, &mut ctx);
        let sim = g.sim.as_ref().unwrap();
        assert!(sim.t < 0.05, "t={} after {steps} steps", sim.t);
        assert!((sim.separation() - ra).abs() < 1e-3);
        assert_eq!(sim.stars.len(), star_count(size));
        assert_eq!(sim.close, None, "the burst resets with the encounter");
    }

    #[test]
    fn gal_a4_restart_reuses_the_star_buffer_and_matches_a_fresh_encounter() {
        let mut ctx = Ctx::new(11, Size::new(1, 1));
        let mut sim = Encounter::new(800, &mut ctx.rng);
        for _ in 0..600 {
            sim.step(SIM_DT);
        }
        let buffer = sim.stars.iter().next().map(std::ptr::from_ref);
        let (mut a, mut b) = (Ctx::new(12, Size::new(1, 1)), Ctx::new(12, Size::new(1, 1)));
        sim.reset(800, &mut a.rng);
        let fresh = Encounter::new(800, &mut b.rng);
        assert_eq!(sim.stars.iter().next().map(std::ptr::from_ref), buffer);
        assert_eq!((sim.t, sim.close), (0.0, None));
        for (s, f) in sim.stars.iter().zip(fresh.stars.iter()) {
            assert_eq!(
                (s.body.pos, s.body.vel, s.gal),
                (f.body.pos, f.body.vel, f.gal)
            );
        }
        assert_eq!(sim.stars.len(), fresh.stars.len());
    }

    #[test]
    fn gal_a8_spiral_pattern_is_trailing_turning_and_confined_to_the_disks() {
        let mut sim = encounter(0, 13);
        let probe = |sim: &Encounter, g: u8, r: f64, phi: f64| {
            let (e1, e2) = sim.disks[usize::from(g)];
            let d = add(scale(e1, r * phi.cos_det()), scale(e2, r * phi.sin_det()));
            let s = Star {
                body: Body {
                    pos: add(sim.cores[usize::from(g)].pos, d),
                    ..Body::default()
                },
                gal: g,
                cloud: false,
            };
            sim.arm(&s)
        };
        for g in [0, 1] {
            // The crest lies at φ = −ARM_WIND·ln(r/R_MIN) (mod π): trailing, and the same
            // arms the stars start in.
            for r in [0.3, 0.6, 0.9] {
                let crest = -ARM_WIND * (r / R_MIN).ln();
                let (on, inside) = probe(&sim, g, r, crest);
                let (off, _) = probe(&sim, g, r, crest + PI / 2.0);
                assert!((on - 1.0).abs() < 1e-9 && (off + 1.0).abs() < 1e-9, "r={r}");
                assert_eq!(inside, 1.0);
                // A ring's mean light is unchanged: the pattern moves light, not adds it.
                let mean = (0..360)
                    .map(|i| probe(&sim, g, r, TAU * f64::from(i) / 360.0).0)
                    .sum::<f64>()
                    / 360.0;
                assert!(mean.abs() < 1e-9);
            }
            // Beyond the disk (tails, bridge) there is no pattern.
            assert_eq!(probe(&sim, g, ARM_R_OUT + 0.1, 0.0).1, 0.0);
        }
        // The pattern turns forwards at 1/ARM_PERIOD turns per second.
        let r = 0.5;
        let crest = -ARM_WIND * (r / R_MIN).ln();
        sim.t = ARM_PERIOD / 8.0;
        let (at, _) = probe(&sim, 0, r, crest + TAU / 8.0);
        assert!((at - 1.0).abs() < 1e-9);
    }

    #[test]
    fn gal_a6_overlapping_hosts_stipple_instead_of_cutting() {
        let pixels = (0..64).flat_map(|y| (0..64).map(move |x| (x, y)));
        let share = |a: f32, b: f32| {
            pixels
                .clone()
                .filter(|&(x, y)| host_of(a, b, x, y) == 1)
                .count() as f64
                / 4096.0
        };
        // A lone host owns its pixels; an even mix shows both, ≈ MIX_MAX of the minority.
        assert_eq!(share(1.0, 0.0), 0.0);
        assert_eq!(share(0.0, 1.0), 1.0);
        assert_eq!(
            share(1.0, 0.25),
            0.0,
            "below MIX_FROM the brighter host keeps it all"
        );
        let even = share(1.0, 0.999);
        assert!((even - MIX_MAX).abs() < 0.05, "{even}");
        // The minority shows in clumps (2 × 2 pixels), not single-pixel confetti: a
        // minority pixel's clump partner is a minority pixel too.
        for (x, y) in pixels
            .clone()
            .filter(|&(x, y)| host_of(1.0, 0.999, x, y) == 1)
        {
            assert_eq!(
                host_of(1.0, 0.999, x ^ 1, y ^ 1),
                1,
                "lone speck at {x},{y}"
            );
        }
    }

    #[test]
    fn gal_a7_knots_are_round_pink_cores_in_a_brightened_host() {
        let mut px = Pixels::new(Size::new(9, 5));
        let host = Ink::new(Role::Ramp(Ramp::Ember), 100);
        px.fill(host);
        knot(&mut px, 4, 4, 1.0);
        let core = px.get(4, 4).unwrap();
        assert_eq!(core.role, Role::Ramp(Ramp::HAlpha));
        // All eight neighbours stay the host's colour, brightened, orthogonal ones more:
        // a round glow rather than a pink `+` sprite.
        let at = |dx: i32, dy: i32| px.get(4 + dx, 4 + dy).unwrap();
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, 1),
            (-1, 1),
            (1, -1),
        ] {
            let ink = at(dx, dy);
            assert_eq!(ink.role, host.role, "{dx},{dy}");
            assert!(ink.level > host.level, "{dx},{dy} not brightened");
        }
        assert!(at(1, 0).level > at(1, 1).level);
        assert_eq!(px.get(6, 4), Some(host), "the knot is local");
    }

    #[test]
    fn gal_frame_keeps_cores_on_screen() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(9, size);
        let mut g = GalaxyCollision::default();
        g.resize(size, &mut ctx);
        for _ in 0..(40 * 60) {
            g.update(SIM_DT, &mut ctx);
            for c in &g.sim.as_ref().unwrap().cores {
                let (x, y) = g.frame.pixel(c.pos);
                assert!((-0.5..199.5).contains(&x) && (-0.5..99.5).contains(&y));
            }
        }
    }

    #[test]
    fn gal_a5_bilinear_splat_conserves_weight() {
        let (pw, ph) = (7, 5);
        for (x, y) in [(3.0, 2.0), (2.25, 1.6), (0.0, 0.0), (5.9, 3.99), (1.5, 3.5)] {
            let mut buf = vec![0.0_f32; pw * ph];
            splat(&mut buf, pw, x, y, 2.0);
            let total: f32 = buf.iter().sum();
            assert!((total - 2.0).abs() < 1e-6, "({x}, {y}) sums to {total}");
            assert!(buf.iter().all(|&v| v >= 0.0));
        }
        let mut buf = vec![0.0_f32; pw * ph];
        splat(&mut buf, pw, 3.0, 2.0, 1.0);
        assert_eq!(
            buf[2 * pw + 3],
            1.0,
            "a star on a pixel centre lights only it"
        );
        // Off the buffer or non-finite: nothing, and no panic.
        let mut buf = vec![0.0_f32; pw * ph];
        for (x, y) in [(-3.0, 1.0), (1.0, 99.0), (f64::NAN, 1.0), (1e300, -1e300)] {
            splat(&mut buf, pw, x, y, 1.0);
        }
        assert!(buf.iter().all(|&v| v == 0.0));
        splat(&mut [], 0, 0.0, 0.0, 1.0);
    }

    #[test]
    fn gal_a6_pixels_carry_the_two_host_ramps() {
        let (g, _) = scene_at(Size::new(200, 50), 2.0);
        let inks = pixel_inks(&g);
        let count = |r: Ramp| inks.iter().filter(|i| i.role == Role::Ramp(r)).count();
        for ink in &inks {
            assert!(
                matches!(
                    ink.role,
                    Role::Background | Role::Ramp(Ramp::Ice | Ramp::Ember | Ramp::HAlpha)
                ),
                "{ink:?}"
            );
        }
        assert!(count(Ramp::Ice) > 200, "host 1: {}", count(Ramp::Ice));
        assert!(count(Ramp::Ember) > 200, "host 2: {}", count(Ramp::Ember));
        // The nuclei are the brightest light in the frame.
        let peak = inks.iter().map(|i| i.level).max().unwrap_or(0);
        assert!(peak > 200, "peak level {peak}");
    }

    #[test]
    fn gal_a7_knots_bloom_only_after_the_close_approach() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = scene_at(size, 6.0);
        let knots = |g: &GalaxyCollision| {
            pixel_inks(g)
                .iter()
                .filter(|i| i.role == Role::Ramp(Ramp::HAlpha))
                .count()
        };
        assert_eq!(g.sim.as_ref().unwrap().close, None);
        assert_eq!(knots(&g), 0, "knots before the encounter");
        while g.sim.as_ref().unwrap().close.is_none() {
            g.update(SIM_DT, &mut ctx);
        }
        let t0 = g.sim.as_ref().unwrap().t;
        let (mut most, mut late) = (0, 0);
        while g.sim.as_ref().unwrap().t < t0 + 10.0 {
            g.update(SIM_DT, &mut ctx);
            let t = g.sim.as_ref().unwrap().t;
            if t > t0 + 3.0 {
                most = most.max(knots(&g));
            }
            if t > t0 + 8.0 {
                late = late.max(knots(&g));
            }
        }
        assert!(most >= 5, "only {most} knots after the close approach");
        // The burst outlasts the encounter: knots still glow 8 s on.
        assert!(
            late >= 30,
            "only {late} knot pixels 8 s after the close approach"
        );
        let sim = g.sim.as_ref().unwrap();
        let clouds = sim.stars.iter().filter(|s| s.cloud).count();
        let n = sim.stars.len() as f64;
        assert!(
            (0.5 * CLOUD_FRACTION * n..1.5 * CLOUD_FRACTION * n).contains(&(clouds as f64)),
            "{clouds} clouds among {n} stars"
        );
    }
}
