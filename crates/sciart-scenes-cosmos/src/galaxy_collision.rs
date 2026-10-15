//! Two disk galaxies in a close encounter (SPEC-002 `galaxy_collision`).
//!
//! Restricted three-body model after Toomre & Toomre (1972): two softened point-mass
//! cores on a bound eccentric orbit, integrated with kick-drift-kick leapfrog, carry
//! massless test stars that start on circular orbits in their host's disk. Units are
//! `G = 1`, disk radius `RD = 1`; one real second is `time_scale()` model time units,
//! chosen so the Kepler period of the cores is 24 s. The cycle starts at apocentre, so
//! the first pericentre falls at ≈ 12 s and the second close passage (the "merger")
//! near 36 s, just before the 40 s restart.
//!
//! The camera looks at the centre of mass from a seeded tilt and zooms smoothly so the
//! cores, disks and (a bounded part of) the tidal tails stay on screen.

use std::f64::consts::{PI, TAU};

use rand::Rng;

use sciart_core::geom::ASPECT;
use sciart_core::geom::v3::{add, cross, dot, norm, scale, sub};
use sciart_core::particles::{gaussian, plot};
use sciart_core::prelude::*;
use sciart_core::raster::Dots;

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
/// Test stars per cell and their cap: 4× M11's, so the arms and tails read as dust
/// (GAL-A6).
const STAR_DENSITY: f64 = 0.16;
const MAX_STARS: usize = 3600;
/// Arm winding (`φ = arm − ARM_WIND·ln(r/R_MIN)`, pitch ≈ 20°) and angular scatter.
const ARM_WIND: f64 = 2.7;
const ARM_SCATTER: f64 = 0.25;
/// Inner edge of the stellar disks (the bulge is represented by the core glyph).
const R_MIN: f64 = 0.1 * RD;
/// Camera: easing time (s), room kept around each core for its disk, and how far
/// beyond that the tails may push the frame.
const CAMERA_TAU: f64 = 0.8;
const DISK_ROOM: f64 = 1.5 * RD;
const TAIL_ROOM: f64 = 2.5 * RD;

/// Afterglow (GAL-A7): positions kept per star, sampled every `TRAIL_DT` s of simulated
/// time, so a trail spans ≤ `TRAIL_K · TRAIL_DT` = 0.32 s of path.
const TRAIL_K: usize = 8;
const TRAIL_DT: f64 = 0.04;
/// Stars farther than this from their host core are tidal material and draw threads.
const TIDAL_R: f64 = 1.15 * RD;
/// Longest thread segment drawn (dots); longer ones are a restart, not motion.
const MAX_SEG_DOTS: f64 = 24.0;
/// Level of a star's own dot; its trail stays below it.
const STAR_LEVEL: u8 = 255;
const CORE: Glyph = Glyph::new('◉', '@');
const SKY_LEVEL: u8 = 60;
const SKY_STARS: usize = 70;

/// Level of the trail segment ending at the sample `age` steps back (0 = newest):
/// falls linearly from 170 to ≈ 56, below `STAR_LEVEL`.
pub(crate) fn trail_level(age: usize) -> u8 {
    let left = TRAIL_K.saturating_sub(age) as f64 / TRAIL_K as f64;
    (40.0 + 130.0 * left) as u8
}

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
    /// 0 = galaxy 1 (`Cold`), 1 = galaxy 2 (`Hot`).
    gal: u8,
    /// Recent positions (model space, ring indexed by `Encounter::head`), so the
    /// afterglow follows the camera instead of smearing with it.
    hist: [[f32; 3]; TRAIL_K],
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
    stars: ParticlePool<Star>,
    /// Real seconds since this encounter began.
    t: f64,
    /// Disk spin axes (GAL-A5 reads the arms in their planes).
    #[cfg_attr(not(test), allow(dead_code))]
    spins: [V3; 2],
    /// Trail ring: newest sample index, samples recorded (≤ `TRAIL_K`), and seconds
    /// since the newest one.
    head: usize,
    samples: usize,
    since: f64,
}

impl Encounter {
    /// Cores at apocentre of the bound orbit, `n` stars split ∝ mass on circular orbits.
    /// Galaxy 1 spins nearly prograde; galaxy 2 is inclined 40°–140° (possibly retrograde).
    pub(crate) fn new(n: usize, rng: &mut impl Rng) -> Self {
        let ra = A * (1.0 + E);
        let va = (M * (1.0 - E) / (A * (1.0 + E))).sqrt();
        let (r, v) = ([ra, 0.0, 0.0], [0.0, va, 0.0]);
        let mut cores = [
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
        let mut stars = ParticlePool::with_capacity(n);
        for i in 0..n {
            let g = usize::from(i >= n1);
            let (host, m) = (cores[g], [M1, M2][g]);
            let (e1, e2) = disk_basis(spins[g]);
            // Two trailing log-spiral arms (pitch ≈ 20°) with scatter; they wind up
            // under differential rotation, as real material arms would.
            let r = R_MIN + (RD - R_MIN) * rng.random_range(0.0f64..1.0).sqrt();
            let arm = if rng.random_bool(0.5) { 0.0 } else { PI };
            let phi = arm - ARM_WIND * (r / R_MIN).ln() + gaussian(rng, 0.0, ARM_SCATTER);
            let (c, s) = (phi.cos_det(), phi.sin_det());
            let radial = add(scale(e1, c), scale(e2, s));
            let tangent = add(scale(e1, -s), scale(e2, c));
            let thick = scale(spins[g], gaussian(rng, 0.0, 0.02 * RD));
            let offset = add(scale(radial, r), thick);
            let vc = circular_speed(m, norm(offset));
            stars.spawn(Star {
                body: Body {
                    pos: add(host.pos, offset),
                    vel: add(host.vel, scale(tangent, vc)),
                    acc: [0.0; 3],
                },
                gal: g as u8,
                hist: [[0.0; 3]; TRAIL_K],
            });
        }
        update_accels(&mut cores, &mut stars);
        Self {
            cores,
            stars,
            t: 0.0,
            spins,
            head: 0,
            samples: 0,
            since: 0.0,
        }
    }

    /// Push every star's position onto its trail ring.
    fn record(&mut self) {
        self.head = (self.head + 1) % TRAIL_K;
        self.samples = (self.samples + 1).min(TRAIL_K);
        let head = self.head;
        for s in self.stars.iter_mut() {
            s.hist[head] = s.body.pos.map(|c| c as f32);
        }
    }

    /// Simulated seconds back to the oldest trail sample (GAL-A7).
    #[cfg(test)]
    fn trail_span(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.since + (self.samples - 1) as f64 * TRAIL_DT
        }
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
        // Trail samples follow simulated time, not draw cadence (GEN-R20).
        self.since += dt;
        while self.since >= TRAIL_DT {
            self.since -= TRAIL_DT;
            self.record();
        }
    }

    #[cfg(test)]
    fn separation(&self) -> f64 {
        norm(sub(self.cores[1].pos, self.cores[0].pos))
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

#[derive(Debug)]
pub struct GalaxyCollision {
    sim: Option<Encounter>,
    view: View,
    size: Size,
    /// Camera: screen-plane point at the canvas centre (model units) and rows per model
    /// unit, both eased towards `target_frame`.
    centre: (f64, f64),
    zoom: f64,
    /// Star dust and afterglow threads, rasterised in `prepare` when `dirty` (GEN-R20).
    dots: Dots,
    dirty: bool,
    /// Distant field stars, normalized `[0, 1)²` (they do not zoom).
    sky: Vec<(f64, f64)>,
}

impl Default for GalaxyCollision {
    fn default() -> Self {
        Self {
            sim: None,
            view: View::default(),
            size: Size::new(1, 1),
            centre: (0.0, 0.0),
            zoom: 1.0,
            dots: Dots::default(),
            dirty: true,
            sky: Vec::new(),
        }
    }
}

fn star_count(size: Size) -> usize {
    ((size.area() as f64 * STAR_DENSITY) as usize).clamp(16, MAX_STARS)
}

impl GalaxyCollision {
    fn restart(&mut self, ctx: &mut Ctx) {
        self.sim = Some(Encounter::new(star_count(self.size), &mut ctx.rng));
        self.view = View::random(&mut ctx.rng);
        self.snap_camera();
    }

    /// Camera target `(centre, zoom)`: frame the cores with their disks, centred on the
    /// midpoint of the pair, widened (boundedly) by the stars' spread so tails show.
    fn target_frame(&self) -> ((f64, f64), f64) {
        let Some(sim) = &self.sim else {
            return ((0.0, 0.0), 1.0);
        };
        let [a, b] = sim.cores.map(|c| self.view.project(c.pos));
        let centre = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
        let cx = 0.5 * (a.0 - b.0).abs() + DISK_ROOM;
        let cy = 0.5 * (a.1 - b.1).abs() + DISK_ROOM;
        let (mut sx, mut sy) = (0.0, 0.0);
        for s in &sim.stars {
            let (x, y) = self.view.project(s.body.pos);
            sx += (x - centre.0) * (x - centre.0);
            sy += (y - centre.1) * (y - centre.1);
        }
        let n = sim.stars.len().max(1) as f64;
        let ex = cx.max(1.6 * (sx / n).sqrt()).min(cx + TAIL_ROOM);
        let ey = cy.max(1.6 * (sy / n).sqrt()).min(cy + TAIL_ROOM);
        let (w, h) = (f64::from(self.size.width()), f64::from(self.size.height()));
        let zx = ((w / 2.0 - 1.0).max(0.5)) / (ex * ASPECT);
        let zy = ((h / 2.0 - 0.5).max(0.5)) / ey;
        (centre, zx.min(zy))
    }

    fn snap_camera(&mut self) {
        (self.centre, self.zoom) = self.target_frame();
    }

    fn to_cell(&self, p: V3) -> (f64, f64) {
        let (x, y) = self.view.project(p);
        (
            f64::from(self.size.width()) / 2.0 + (x - self.centre.0) * self.zoom * ASPECT,
            f64::from(self.size.height()) / 2.0 - (y - self.centre.1) * self.zoom,
        )
    }

    /// Rasterise the sky, the afterglow threads and the stars into `dots` through the
    /// current camera (GAL-A7, GAL-A8).
    fn rasterise(&mut self) {
        self.dots.clear();
        let ds = self.dots.dot_size();
        let (dw, dh) = (f64::from(ds.width()), f64::from(ds.height()));
        for &(u, v) in &self.sky {
            self.dots
                .plot(u * dw, v * dh, Ink::new(Role::Star, SKY_LEVEL));
        }
        let Some(sim) = &self.sim else { return };
        let fade = self.fade();
        let (view, centre, zoom) = (self.view, self.centre, self.zoom);
        let (hw, hh) = (
            f64::from(self.size.width()) / 2.0,
            f64::from(self.size.height()) / 2.0,
        );
        // `to_cell` then `Dots::from_cells`, with the camera copied out of `self`.
        let to_dot = |p: V3| {
            let (x, y) = view.project(p);
            Dots::from_cells(
                hw + (x - centre.0) * zoom * ASPECT,
                hh - (y - centre.1) * zoom,
            )
        };
        let levels: [u8; TRAIL_K] =
            std::array::from_fn(|age| (f64::from(trail_level(age)) * fade) as u8);
        let star_level = (f64::from(STAR_LEVEL) * fade) as u8;
        for s in &sim.stars {
            // Only tidal material trails; disk stars stay dust, keeping arms and gaps.
            let host = sim.cores[usize::from(s.gal)].pos;
            let d = sub(s.body.pos, host);
            if dot(d, d) <= TIDAL_R * TIDAL_R {
                continue;
            }
            let role = if s.gal == 0 { Role::Cold } else { Role::Hot };
            let mut a = to_dot(s.body.pos);
            // Newest sample first; each segment dims with the age of its far end.
            for (age, &level) in levels.iter().enumerate().take(sim.samples) {
                let h = s.hist[(sim.head + TRAIL_K - age) % TRAIL_K];
                let b = to_dot(h.map(f64::from));
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                if dx * dx + dy * dy > MAX_SEG_DOTS * MAX_SEG_DOTS {
                    break;
                }
                self.dots.line(a.0, a.1, b.0, b.1, Ink::new(role, level));
                a = b;
            }
        }
        // Stars last: `Dots` keeps the strongest ink per cell, so a star outshines trails.
        for s in &sim.stars {
            let role = if s.gal == 0 { Role::Cold } else { Role::Hot };
            let (x, y) = to_dot(s.body.pos);
            self.dots.plot(x, y, Ink::new(role, star_level));
        }
    }

    /// Brightness envelope: fade in after a restart and out before the next one.
    fn fade(&self) -> f64 {
        let t = self.sim.as_ref().map_or(0.0, |s| s.t);
        let x = (t / 1.5).min((CYCLE - t) / 2.0).clamp(0.0, 1.0);
        0.3 + 0.7 * x * x * (3.0 - 2.0 * x)
    }
}

impl Generator for GalaxyCollision {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
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
        let n = star_count(size);
        match &mut self.sim {
            // Fewer stars: keep the encounter running with a subset.
            Some(sim) if n <= sim.stars.len() => sim.stars.set_capacity(n),
            _ => self.restart(ctx),
        }
        self.snap_camera();
        self.dots.resize(size);
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
            self.centre.0 += (centre.0 - self.centre.0) * k;
            self.centre.1 += (centre.1 - self.centre.1) * k;
            self.zoom = (self.zoom.ln() + (zoom.ln() - self.zoom.ln()) * k).exp();
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
        if let Some(sim) = &self.sim {
            for c in &sim.cores {
                let (x, y) = self.to_cell(c.pos);
                plot(canvas, x, y, CORE, Ink::new(Role::Accent, 255));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encounter(n: usize, seed: u64) -> Encounter {
        let mut ctx = Ctx::new(seed, Size::new(1, 1));
        Encounter::new(n, &mut ctx.rng)
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
    }

    #[test]
    fn gal_a5_disks_start_as_two_arm_spirals() {
        let sim = encounter(1200, 11);
        for g in 0..2u8 {
            let (e1, e2) = disk_basis(sim.spins[usize::from(g)]);
            let host = sim.cores[usize::from(g)].pos;
            let (mut re, mut im, mut n) = (0.0, 0.0, 0.0);
            for s in sim.stars.iter().filter(|s| s.gal == g) {
                let d = sub(s.body.pos, host);
                let (x, y) = (dot(d, e1), dot(d, e2));
                let r = (x * x + y * y).sqrt();
                // De-rotate by the arm phase: two arms collapse onto θ' ∈ {0, π}.
                let th = y.atan2(x) + ARM_WIND * (r / R_MIN).ln();
                let (s2, c2) = (2.0 * th).sin_cos_det();
                (re, im, n) = (re + c2, im + s2, n + 1.0);
            }
            let amp = (re * re + im * im).sqrt() / n;
            assert!(amp >= 0.5, "galaxy {g}: m=2 amplitude {amp:.2}");
        }
    }

    #[test]
    fn gal_a6_star_budget_is_four_times_m11() {
        assert_eq!(star_count(Size::new(200, 50)), 1600);
        assert_eq!(star_count(Size::new(500, 150)), 3600);
        assert_eq!(star_count(Size::new(10, 10)), 16);
    }

    fn run(size: Size, secs: f64, seed: u64) -> (GalaxyCollision, Canvas) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = GalaxyCollision::default();
        g.resize(size, &mut ctx);
        for _ in 0..(secs * 60.0).round() as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        (g, canvas)
    }

    #[test]
    fn gal_a7_moving_stars_leave_fading_trails() {
        for k in 1..TRAIL_K {
            assert!(trail_level(k) < trail_level(k - 1), "age {k}");
        }
        assert!(trail_level(0) < STAR_LEVEL);
        assert!(TRAIL_DT * TRAIL_K as f64 <= 0.32 + 1e-12);
        // (lit dots, the stars' and sky's own dots) of a frame after `secs`.
        let counts = |secs: f64| {
            let (g, _) = run(Size::new(200, 50), secs, 12);
            let span = g.sim.as_ref().unwrap().trail_span();
            assert!(span <= 0.32 + 1e-9, "span {span}");
            let ds = g.dots.dot_size();
            let (dw, dh) = (f64::from(ds.width()), f64::from(ds.height()));
            let mut own = std::collections::HashSet::new();
            for &(u, v) in &g.sky {
                own.insert(((u * dw).round() as i32, (v * dh).round() as i32));
            }
            for s in &g.sim.as_ref().unwrap().stars {
                let (x, y) = g.to_cell(s.body.pos);
                let (dx, dy) = Dots::from_cells(x, y);
                own.insert((dx.round() as i32, dy.round() as i32));
            }
            let lit = (0..i32::from(ds.height()))
                .flat_map(|y| (0..i32::from(ds.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| g.dots.is_lit(x, y))
                .count();
            (lit as f64, own.len() as f64)
        };
        // Before the encounter only tidal material would trail: there is none yet.
        let (lit, own) = counts(3.0);
        assert!(lit <= 1.05 * own, "t=3: {lit} lit vs {own} own dots");
        // After pericentre the tails and bridges stream.
        let (lit, own) = counts(16.0);
        assert!(lit >= 1.3 * own, "t=16: {lit} lit vs {own} own dots");
    }

    #[test]
    fn gal_a8_stars_draw_as_braille_dust() {
        let size = Size::new(200, 50);
        for t in [5.0, 20.0] {
            let (_, canvas) = run(size, t, 13);
            let (mut lit, mut braille, mut blocks) = (0u32, 0u32, 0u32);
            for y in 0..49 {
                for x in 0..200 {
                    let ch = canvas.get(x, y).unwrap().ch;
                    if ch == ' ' || ch == '\u{2800}' {
                        continue;
                    }
                    lit += 1;
                    braille += u32::from(('\u{2801}'..='\u{28FF}').contains(&ch));
                    blocks += u32::from(matches!(ch, '▀' | '▄' | '█'));
                }
            }
            assert_eq!(blocks, 0, "t={t}");
            let share = f64::from(braille) / f64::from(lit.max(1));
            assert!(share >= 0.9, "t={t}: braille share {share:.3} of {lit}");
        }
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
                let (x, y) = g.to_cell(c.pos);
                assert!((0.0..200.0).contains(&x) && (0.0..50.0).contains(&y));
            }
        }
    }
}
