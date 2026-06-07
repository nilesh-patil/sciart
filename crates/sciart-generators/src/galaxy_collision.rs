//! Two disk galaxies in a close encounter (SPEC-002 `galaxy_collision`, ADR-0017).
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
//! The picture is M11's sparse star field: every test star is one braille dot in its
//! host's muted colour (galaxy 1 warm, galaxy 2 cool), brighter where stars crowd and on
//! the spiral arms, and the tails and bridge as bright streams of dots; each nucleus is a
//! bright 2 × 2-dot cluster in its host's colour.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::geom::v3::{add, cross, dot, norm, scale, sub};
use sciart_core::ink;
use sciart_core::particles::{gaussian, plot};
use sciart_core::prelude::*;
use sciart_core::raster::Dots;
use sciart_core::timeline::ease;

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
/// Test stars per cell and their bounds: a sparse field, one braille dot per star.
const STAR_DENSITY: f64 = 0.25;
const MIN_STARS: usize = 64;
const MAX_STARS: usize = 3000;
/// Inner edge of the stellar disks.
const R_MIN: f64 = 0.1 * RD;
/// Camera: easing time (s), room kept around each core for its disk, and how far
/// beyond that the tails may push the frame.
const CAMERA_TAU: f64 = 0.8;
const DISK_ROOM: f64 = 1.5 * RD;
const TAIL_ROOM: f64 = 2.5 * RD;
/// Two-armed spiral pattern (GAL-A8): winding (radians of azimuth per e-fold in radius,
/// a pitch of ≈ 20°), pattern period (s), brightness contrast `1 ± ARM_CONTRAST`, and the
/// radii over which it fades out beyond the disk.
const ARM_WIND: f64 = 2.7;
const ARM_PERIOD: f64 = 30.0;
const ARM_CONTRAST: f64 = 0.75;
const ARM_R_IN: f64 = 1.0 * RD;
const ARM_R_OUT: f64 = 1.4 * RD;
/// Host colours (ADR-0017, muted): galaxy 1 a warm old population (`Sienna`, ~4500 K
/// light), galaxy 2 a cooler, bluer one (`Cobalt`, ~9000 K light).
const HOSTS: [Ramp; 2] = [Ramp::Sienna, Ramp::Cobalt];
/// A star's dot level: from `STAR_LEVEL.0` for a lone star between the arms to
/// `STAR_LEVEL.1` where `CROWD` or more stars share its cell on an arm's crest, by
/// `DENSITY_SHARE` crowding and the rest arm; tail and bridge stars at least `TAIL_LEVEL`
/// (the tails are the subject).
const STAR_LEVEL: (f64, f64) = (100.0, 210.0);
const CROWD: f64 = 5.0;
const DENSITY_SHARE: f64 = 0.6;
const TAIL_LEVEL: f64 = 150.0;
/// A nucleus: a 2 × 2-dot cluster at this level in its host's colour.
const CORE_LEVEL: f64 = 240.0;
const SKY: Glyph = Glyph::new('·', '.');
const SKY_STARS: usize = 70;
/// Background stars stay dimmer than any galaxy star.
const SKY_LEVEL: u8 = 50;

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
    /// 0 = galaxy 1, 1 = galaxy 2.
    gal: u8,
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

/// The dynamical state of one encounter (no screen knowledge).
#[derive(Debug, Clone)]
pub(crate) struct Encounter {
    cores: [Body; 2],
    /// Each host's disk basis `(e1, e2)`, for its spiral pattern.
    disks: [(V3, V3); 2],
    stars: ParticlePool<Star>,
    /// Real seconds since this encounter began.
    t: f64,
}

impl Encounter {
    /// A fresh encounter of `n` stars (see [`reset`](Self::reset)).
    pub(crate) fn new(n: usize, rng: &mut impl Rng) -> Self {
        let mut sim = Self {
            cores: [Body::default(); 2],
            disks: [([0.0; 3], [0.0; 3]); 2],
            stars: ParticlePool::with_capacity(n),
            t: 0.0,
        };
        sim.reset(n, rng);
        sim
    }

    /// Start over in place, reusing the star buffer (no allocation unless `n` outgrows
    /// it; GEN-R11): cores at apocentre of the bound orbit, `n` stars split ∝ mass on
    /// circular orbits. The hosts are interleaved ([`host_of_index`]) so every prefix of
    /// the buffer keeps the mass split and a shrink that truncates it keeps both galaxies. Galaxy 1 spins nearly prograde; galaxy 2 is inclined 40°–140°
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
        let disks = spins.map(disk_basis);
        self.stars.clear();
        self.stars.set_capacity(n);
        for i in 0..n {
            let g = host_of_index(i);
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
            self.stars.spawn(Star {
                body: Body {
                    pos: add(host.pos, offset),
                    vel: add(host.vel, scale(tangent, vc)),
                    acc: [0.0; 3],
                },
                gal: g as u8,
            });
        }
        self.cores = cores;
        self.disks = disks;
        self.t = 0.0;
        update_accels(&mut self.cores, &mut self.stars);
    }

    /// Where star `s` sits in its host's two-armed spiral pattern: `(cos ψ, inside)`,
    /// `ψ = 2(φ + ARM_WIND·ln(r/R_MIN) − Ω_p t)` with `(r, φ)` its polar position in the
    /// host's disk plane and `Ω_p = 2π/ARM_PERIOD`; `inside` is 1 within the disk, falling
    /// smoothly to 0 by `ARM_R_OUT` (tails and bridge carry no pattern) (GAL-A8).
    ///
    /// The pattern is a density wave: stars brighten while they pass through it, so the
    /// arms persist while the stars' own initial arms wind up and blur (a star's dot is
    /// brighter on a crest).
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
    }

    #[cfg(test)]
    fn separation(&self) -> f64 {
        norm(sub(self.cores[1].pos, self.cores[0].pos))
    }
}

/// The host of star `i`: galaxy 2 exactly when `⌊(i + 1)·M2/M⌋ > ⌊i·M2/M⌋` (a Bresenham
/// split), so any first `n` stars hold `⌊n·M2/M⌋` of galaxy 2 and the rest of galaxy 1
/// (GAL-A2, GAL-A4).
fn host_of_index(i: usize) -> usize {
    let share = |k: usize| (k as f64 * M2 / M).floor();
    usize::from(share(i + 1) > share(i))
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
    /// Every star as one braille dot (drawn in `prepare`).
    dots: Dots,
    /// Stars per cell, for the density brightening (sized in `resize`).
    counts: Vec<u8>,
    /// Distant field stars, normalized `[0, 1)²` (they do not zoom).
    sky: Vec<(f64, f64)>,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
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
            dots: Dots::default(),
            counts: Vec::new(),
            sky: Vec::new(),
            dirty: false,
        }
    }
}

fn star_count(size: Size) -> usize {
    ((size.area() as f64 * STAR_DENSITY) as usize).clamp(MIN_STARS, MAX_STARS)
}

/// The level of a star's dot: brighter where `crowd` stars share its cell and on its
/// host's spiral arm (`wave`, `inside` from [`Encounter::arm`]); a tail or bridge star
/// (`inside` 0) at least `TAIL_LEVEL`; times the cycle's `fade`.
fn star_level(crowd: f64, wave: f64, inside: f64, fade: f64) -> f64 {
    let dense = ((crowd - 1.0) / (CROWD - 1.0)).clamp(0.0, 1.0);
    let arm = ((1.0 + ARM_CONTRAST * wave) / (1.0 + ARM_CONTRAST)).clamp(0.0, 1.0);
    let lit = DENSITY_SHARE * dense + (1.0 - DENSITY_SHARE) * arm;
    let disk = STAR_LEVEL.0 + (STAR_LEVEL.1 - STAR_LEVEL.0) * lit;
    let level = disk.max(TAIL_LEVEL * (1.0 - inside));
    level * fade
}

/// The dot of continuous pixel position `(x, y)` (a pixel is 2 × 2 dots, integers at
/// pixel centres), or `None` off any lattice.
fn dot_of(x: f64, y: f64) -> Option<(i32, i32)> {
    let (dx, dy) = ((2.0 * x + 0.5).round(), (2.0 * y + 0.5).round());
    (dx.is_finite() && dy.is_finite() && dx.abs() < 1e6 && dy.abs() < 1e6)
        .then_some((dx as i32, dy as i32))
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

    /// Brightness envelope: fade in after a restart and out before the next one.
    fn fade(&self) -> f64 {
        let t = self.sim.as_ref().map_or(0.0, |s| s.t);
        0.3 + 0.7 * ease::smoothstep((t / 1.5).min((CYCLE - t) / 2.0))
    }

    /// Every star as one braille dot at its projected position in its host's colour,
    /// brighter where stars crowd (GAL-A5), and each nucleus a bright 2 × 2-dot cluster
    /// (GAL-A7).
    fn draw(&mut self) {
        self.dots.clear();
        self.counts.fill(0);
        let Some(sim) = &self.sim else { return };
        let fade = self.fade();
        let w = i32::from(self.frame.size.width());
        let h = i32::from(self.frame.size.height());
        let cell = |(dx, dy): (i32, i32)| {
            let (cx, cy) = (dx.div_euclid(Dots::CELL_W), dy.div_euclid(Dots::CELL_H));
            ((0..w).contains(&cx) && (0..h).contains(&cy)).then(|| (cy * w + cx) as usize)
        };
        for s in &sim.stars {
            let (x, y) = self.frame.pixel(s.body.pos);
            if let Some(i) = dot_of(x, y).and_then(cell) {
                self.counts[i] = self.counts[i].saturating_add(1);
            }
        }
        for s in &sim.stars {
            let (x, y) = self.frame.pixel(s.body.pos);
            let Some(d) = dot_of(x, y) else { continue };
            let crowd = cell(d).map_or(1.0, |i| f64::from(self.counts[i]));
            let (wave, inside) = sim.arm(s);
            let level = star_level(crowd, wave, inside, fade);
            let ramp = HOSTS[usize::from(s.gal)];
            self.dots.set(d.0, d.1, ink::ramp(ramp, level / 255.0));
        }
        for (core, ramp) in sim.cores.iter().zip(HOSTS) {
            let (x, y) = self.frame.pixel(core.pos);
            let (px, py) = ((x + 0.5).floor() as i32, (y + 0.5).floor() as i32);
            let ink = ink::ramp(ramp, CORE_LEVEL * fade / 255.0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                self.dots.set(2 * px + dx, 2 * py + dy, ink);
            }
        }
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
        self.dots.resize(size);
        self.counts.clear();
        self.counts.resize(size.area(), 0);
        let n = star_count(size);
        match &mut self.sim {
            // Keep the running encounter at any new size (GAL-A4). A shrink keeps a prefix
            // of the stars, which the interleaved hosts keep split by mass; a grow keeps the
            // stars it has, and the next cycle starts with the new count.
            Some(sim) => {
                if n < sim.stars.len() {
                    sim.stars.set_capacity(n);
                }
                self.snap_camera();
            }
            None => self.restart(ctx),
        }
        self.dirty = true;
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
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.draw();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let size = self.frame.size;
        if canvas.size() != size {
            return;
        }
        let (wf, hf) = (f64::from(size.width()), f64::from(size.height()));
        for &(u, v) in &self.sky {
            plot(canvas, u * wf, v * hf, SKY, Ink::new(Role::Star, SKY_LEVEL));
        }
        self.dots.blit(canvas);
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
        g.prepare();
        for _ in 0..(t / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        (g, ctx)
    }

    /// The rendered frame.
    fn frame_of(g: &GalaxyCollision, size: Size, mode: GlyphMode) -> Canvas {
        let mut canvas = Canvas::new(size, mode);
        g.render(&mut canvas);
        canvas
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
        let n2 = sim.stars.iter().filter(|s| s.gal == 1).count();
        assert_eq!(n2, (400.0 * M2 / M).floor() as usize);
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
            // ≥ 20 of 600 (> 3 %): a retrograde galaxy 2 throws almost no tail of its own
            // (as in Toomre & Toomre), so the prograde host alone must supply it.
            let n = far(&sim);
            assert!(n >= 20, "only {n} stars in tails (seed {seed})");
        }
    }

    #[test]
    fn gal_a4_cycle_restarts_at_40_s() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(8, size);
        let mut g = GalaxyCollision::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let ra = A * (1.0 + E);
        let mut steps = 0;
        while g.sim.as_ref().unwrap().t + SIM_DT < CYCLE - 1e-9 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            steps += 1;
        }
        assert!((g.sim.as_ref().unwrap().separation() - ra).abs() > 0.1);
        g.update(SIM_DT, &mut ctx);
        g.prepare();
        g.update(SIM_DT, &mut ctx);
        g.prepare();
        let sim = g.sim.as_ref().unwrap();
        assert!(sim.t < 0.05, "t={} after {steps} steps", sim.t);
        assert!((sim.separation() - ra).abs() < 1e-3);
        assert_eq!(sim.stars.len(), star_count(size));
    }

    #[test]
    fn gal_a2_every_prefix_of_the_stars_is_split_by_mass() {
        let sim = encounter(3000, 7);
        let mut g2 = 0usize;
        for (i, s) in sim.stars.iter().enumerate() {
            g2 += usize::from(s.gal);
            let expect = (i + 1) as f64 * M2 / M;
            assert!(
                (g2 as f64 - expect).abs() <= 1.0,
                "prefix {i}: {g2} vs {expect}"
            );
        }
    }

    #[test]
    fn gal_a4_shrink_keeps_mass_ratio_and_the_running_encounter() {
        let (mut g, mut ctx) = scene_at(Size::new(200, 50), 15.0);
        let t = g.sim.as_ref().unwrap().t;
        for small in [Size::new(120, 40), Size::new(80, 24)] {
            g.resize(small, &mut ctx);
            g.prepare();
            let sim = g.sim.as_ref().unwrap();
            let n = sim.stars.len();
            assert_eq!(n, star_count(small));
            assert_eq!(sim.t, t, "a shrink keeps the running encounter");
            let n2 = sim.stars.iter().filter(|s| s.gal == 1).count() as f64;
            assert!(
                (n2 - n as f64 * M2 / M).abs() <= 1.0,
                "{n2} of {n} in host 2"
            );
        }
        // A grow keeps it too (the next cycle starts with the new count).
        g.resize(Size::new(200, 50), &mut ctx);
        g.prepare();
        let sim = g.sim.as_ref().unwrap();
        assert_eq!(sim.t, t, "a grow keeps the running encounter");
        let n2 = sim.stars.iter().filter(|s| s.gal == 1).count() as f64;
        assert!((n2 - sim.stars.len() as f64 * M2 / M).abs() <= 1.0);
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
        assert_eq!(sim.t, 0.0);
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
    fn gal_frame_keeps_cores_on_screen() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(9, size);
        let mut g = GalaxyCollision::default();
        g.resize(size, &mut ctx);
        g.prepare();
        for _ in 0..(40 * 60) {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            for c in &g.sim.as_ref().unwrap().cores {
                let (x, y) = g.frame.pixel(c.pos);
                assert!((-0.5..199.5).contains(&x) && (-0.5..99.5).contains(&y));
            }
        }
    }

    #[test]
    fn gal_a3_tails_render_as_dot_streams() {
        // Well after pericentre, the tail and bridge stars (beyond their disk's pattern)
        // show as dots of their own: a stream of dots, not a smooth band.
        for seed in [42, 7] {
            let size = Size::new(200, 50);
            let mut ctx = Ctx::new(seed, size);
            let mut g = GalaxyCollision::default();
            g.resize(size, &mut ctx);
            g.prepare();
            for _ in 0..(20.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
            }
            let sim = g.sim.as_ref().unwrap();
            let tail = sim
                .stars
                .iter()
                .filter(|s| sim.arm(s).1 == 0.0)
                .filter(|s| {
                    let (x, y) = g.frame.pixel(s.body.pos);
                    let (dx, dy) = ((2.0 * x + 0.5).round(), (2.0 * y + 0.5).round());
                    g.dots.is_lit(dx as i32, dy as i32)
                })
                .count();
            assert!(tail >= 15, "seed {seed}: only {tail} tail dots");
        }
    }

    #[test]
    fn gal_a5_every_star_is_one_dot_in_its_hosts_colour() {
        // One braille dot per star: never more lit dots than stars on screen (plus the two
        // 2 × 2 nuclei), most stars visible (some share a dot), each in a host ramp.
        for t in [2.0, 20.0] {
            let (g, _) = scene_at(Size::new(200, 50), t);
            let sim = g.sim.as_ref().unwrap();
            let dots: u32 = (0..50)
                .flat_map(|y| (0..200).map(move |x| (x, y)))
                .filter_map(|(x, y)| g.dots.cell_mask(x, y))
                .map(u8::count_ones)
                .sum();
            let on_screen = sim
                .stars
                .iter()
                .filter(|s| {
                    let (x, y) = g.frame.pixel(s.body.pos);
                    (-0.5..199.5).contains(&x) && (-0.5..99.5).contains(&y)
                })
                .count() as u32;
            assert!(
                dots <= on_screen + 8,
                "t {t}: {dots} dots for {on_screen} stars"
            );
            assert!(
                dots * 10 >= on_screen * 5,
                "t {t}: {dots} dots for {on_screen} stars"
            );
            let canvas = frame_of(&g, Size::new(200, 50), GlyphMode::Unicode);
            for (i, cell) in canvas.cells().iter().enumerate() {
                let (x, y) = ((i % 200) as i32, (i / 200) as i32);
                if g.dots.cell_mask(x, y).unwrap_or(0) != 0 {
                    assert!(
                        matches!(cell.ink.role, Role::Ramp(r) if HOSTS.contains(&r)),
                        "{:?}",
                        cell.ink
                    );
                }
            }
        }
        // The level: brighter where stars crowd and on a crest, tails never dim.
        let lone = star_level(1.0, -1.0, 1.0, 1.0);
        let crowded = star_level(CROWD, 1.0, 1.0, 1.0);
        let tail = star_level(1.0, 0.0, 0.0, 1.0);
        let trough = (1.0 - ARM_CONTRAST) / (1.0 + ARM_CONTRAST);
        let want = STAR_LEVEL.0 + (STAR_LEVEL.1 - STAR_LEVEL.0) * (1.0 - DENSITY_SHARE) * trough;
        assert!((lone - want).abs() < 1e-9 && (crowded - STAR_LEVEL.1).abs() < 1e-9);
        assert!(tail >= TAIL_LEVEL);
        assert!((star_level(CROWD, 1.0, 1.0, 0.5) - 0.5 * crowded).abs() < 1e-9);
    }

    #[test]
    fn gal_a6_two_coloured_galaxies_on_black() {
        // ADR-0017: every drawn cell is a host-coloured star dot (both hosts present) or a
        // faint sky star, and the frame stays mostly black.
        let size = Size::new(200, 50);
        let (g, _) = scene_at(size, 2.0);
        let canvas = frame_of(&g, size, GlyphMode::Unicode);
        let (mut lit, mut hosts) = (0, [0; 2]);
        for cell in canvas.cells() {
            if cell.ch == ' ' {
                continue;
            }
            lit += 1;
            match cell.ink.role {
                Role::Ramp(r) if r == HOSTS[0] => hosts[0] += 1,
                Role::Ramp(r) if r == HOSTS[1] => hosts[1] += 1,
                Role::Star => assert!(cell.ink.level <= SKY_LEVEL),
                other => panic!("{other:?}"),
            }
        }
        assert!(hosts[0] > 60 && hosts[1] > 60, "{hosts:?}");
        assert!(lit * 100 < 200 * 50 * 12, "{lit} lit cells: not sparse");
    }

    #[test]
    fn gal_a7_the_nuclei_are_bright_clusters_in_their_hosts_colour() {
        let size = Size::new(200, 50);
        let (g, _) = scene_at(size, 2.0);
        let sim = g.sim.as_ref().unwrap();
        let canvas = frame_of(&g, size, GlyphMode::Unicode);
        for (core, ramp) in sim.cores.iter().zip(HOSTS) {
            let (x, y) = g.frame.pixel(core.pos);
            let (px, py) = ((x + 0.5).floor() as i32, (y + 0.5).floor() as i32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                assert!(g.dots.is_lit(2 * px + dx, 2 * py + dy));
            }
            let cell = canvas.get(px, py.div_euclid(2)).unwrap();
            assert_eq!(cell.ink.role, Role::Ramp(ramp));
            let fade = g.fade();
            assert!(
                f64::from(cell.ink.level) + 2.0 >= CORE_LEVEL * fade,
                "{:?}",
                cell.ink
            );
        }
    }
}
