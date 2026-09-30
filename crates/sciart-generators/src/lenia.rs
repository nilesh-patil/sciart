//! Lenia: smooth, glowing creatures on a continuous cellular automaton; Orbium gliders
//! swim, meet, and one is torn and regrows (SPEC-002 `lenia`).
//!
//! Lenia (Chan 2019) on a capped periodic lattice: `A ← clip(A + Δt·G(K * A), 0, 1)` with a
//! ring kernel `K(r) = exp(−((r/R − ½)/0.15)²/2)` (normalised, radius `R = 13`, ≤ 560 taps),
//! growth `G(u) = 2·exp(−(u − μ)²/(2σ²)) − 1` and the Orbium parameters `μ = 0.15`,
//! `σ = 0.015`, `Δt = 0.1`. Orbium is stamped from its published `R = 13` table, so it is the canonical
//! creature (a smaller `R` makes a cheaper but lumpier, more fragile variant).
//!
//! The world steps at 12 Hz from the fixed 60 Hz update; the display lerps between the
//! last two states, so gliders move smoothly. Every [`DAMAGE_S`] seconds a disc is cut out
//! of the heaviest creature and it regrows; a watchdog reseeds if the total mass leaves
//! `[0.3, 1.6]×` the seeded mass. Acts of [`CYCLE_S`] seconds alternate a parallel shoal
//! with staged meetings (one glider engulfs another, or two brush past); a lost glider is
//! replaced by a birth a few seconds later. Every reseed (act change or watchdog)
//! cross-fades: the old creatures keep swimming as a ghost while the new ones condense,
//! so no frame is empty.

use rand::Rng;
use sciart_core::ease;
use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::ASCII_RAMP;

pub static META: Meta = Meta {
    name: "lenia",
    category: Category::Biology,
    description: "Lenia: Orbium gliders swim across a continuous cellular automaton, torn and regrown",
    title: "Lenia · life from smooth rules",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Lenia::default())
}

// ---- Model (SPEC-002 Model) ----

/// Kernel radius in nodes.
const R: f32 = 13.0;
const MU: f32 = 0.15;
const SIGMA: f32 = 0.015;
const DT: f32 = 0.1;
/// World steps per simulated second.
const STEPS_PER_S: f64 = 12.0;
/// Kernel taps at most (SPEC-002).
#[cfg(test)]
const MAX_TAPS: usize = 560;
/// Lattice nodes at most (≤ 96×64).
const MAX_NODES: usize = 6144;
const BUCKET: f64 = 1.25;
/// Seconds between damage beats.
const DAMAGE_S: f64 = 12.0;
/// Seconds per act: then the next act hatches on a new heading.
const CYCLE_S: f64 = 30.0;
/// Cross-fade at every reseed (LNA-A9): the new creatures condense over the first half,
/// while the old ones (still swimming, as a ghost world) dissolve over the second half,
/// so the dish is never empty.
const XFADE_S: f64 = 2.0;

/// What a cycle stages (SPEC-002 Round 3): a parallel shoal, or two gliders aimed at each
/// other, whose meeting (deterministic, so chosen once, LNA-A6/A7) is clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    /// Up to three gliders on one heading: they never meet.
    Shoal,
    /// Crossing courses a quarter turn apart: one glider engulfs the other.
    Engulf,
    /// Opposite courses, offset sideways: they brush past and both swim on.
    Glance,
}
const ACTS: [Act; 4] = [Act::Shoal, Act::Engulf, Act::Shoal, Act::Glance];
/// Orbium's velocity at quarter turn 0, in nodes per second (measured, at 12 steps/s).
const V0: (f64, f64) = (2.70, 6.86);
/// Seconds until a staged pair meets, and the sideways offsets (nodes) of each meeting.
const MEET_S: f64 = 2.5;
const ENGULF_OFF: f64 = -8.0;
const GLANCE_OFF: f64 = 12.0;
/// Smallest lattice side on which each meeting is staged (it needs the room to aim).
const ENGULF_MIN: u16 = 48;
const GLANCE_MIN: u16 = 56;
/// A lost glider is replaced by a birth this many seconds later, fading in over
/// `BIRTH_FADE` seconds within `BIRTH_R` nodes of its hatching place.
const BIRTH_DELAY: f64 = 3.0;
const BIRTH_FADE: f64 = 1.2;
const BIRTH_R: f64 = 14.0;
/// Radius of the damage disc in nodes, and its distance from the creature's heaviest
/// node: a bite from the rim, which an Orbium survives (a disc through the
/// core kills it).
const DAMAGE_R: f32 = 4.0;
const DAMAGE_OFFSET: f32 = 8.0;
/// Watchdog band for the total mass, as a multiple of the seeded mass: the upper bound
/// catches a runaway merger (a growing chimera) early.
const MASS_BAND: (f32, f32) = (0.3, 1.6);
/// Lattice area per seeded glider.
const AREA_PER_GLIDER: usize = 2000;
const MAX_GLIDERS: usize = 3;

/// Orbium unicaudatus at `R = 13` (Chan 2019, the Lenia tutorial's pattern), 20×20.
#[rustfmt::skip]
const ORBIUM: [[f32; 20]; 20] = [
    [0.0,0.0,0.0,0.0,0.0,0.0,0.1,0.14,0.1,0.0,0.0,0.03,0.03,0.0,0.0,0.3,0.0,0.0,0.0,0.0],
    [0.0,0.0,0.0,0.0,0.0,0.08,0.24,0.3,0.3,0.18,0.14,0.15,0.16,0.15,0.09,0.2,0.0,0.0,0.0,0.0],
    [0.0,0.0,0.0,0.0,0.0,0.15,0.34,0.44,0.46,0.38,0.18,0.14,0.11,0.13,0.19,0.18,0.45,0.0,0.0,0.0],
    [0.0,0.0,0.0,0.0,0.06,0.13,0.39,0.5,0.5,0.37,0.06,0.0,0.0,0.0,0.02,0.16,0.68,0.0,0.0,0.0],
    [0.0,0.0,0.0,0.11,0.17,0.17,0.33,0.4,0.38,0.28,0.14,0.0,0.0,0.0,0.0,0.0,0.18,0.42,0.0,0.0],
    [0.0,0.0,0.09,0.18,0.13,0.06,0.08,0.26,0.32,0.32,0.27,0.0,0.0,0.0,0.0,0.0,0.0,0.82,0.0,0.0],
    [0.27,0.0,0.16,0.12,0.0,0.0,0.0,0.25,0.38,0.44,0.45,0.34,0.0,0.0,0.0,0.0,0.0,0.22,0.17,0.0],
    [0.0,0.07,0.2,0.02,0.0,0.0,0.0,0.31,0.48,0.57,0.6,0.57,0.0,0.0,0.0,0.0,0.0,0.0,0.49,0.0],
    [0.0,0.59,0.19,0.0,0.0,0.0,0.0,0.2,0.57,0.69,0.76,0.76,0.49,0.0,0.0,0.0,0.0,0.0,0.36,0.0],
    [0.0,0.58,0.19,0.0,0.0,0.0,0.0,0.0,0.67,0.83,0.9,0.92,0.87,0.12,0.0,0.0,0.0,0.0,0.22,0.07],
    [0.0,0.0,0.46,0.0,0.0,0.0,0.0,0.0,0.7,0.93,1.0,1.0,1.0,0.61,0.0,0.0,0.0,0.0,0.18,0.11],
    [0.0,0.0,0.82,0.0,0.0,0.0,0.0,0.0,0.47,1.0,1.0,0.98,1.0,0.96,0.27,0.0,0.0,0.0,0.19,0.1],
    [0.0,0.0,0.46,0.0,0.0,0.0,0.0,0.0,0.25,1.0,1.0,0.84,0.92,0.97,0.54,0.14,0.04,0.1,0.21,0.05],
    [0.0,0.0,0.0,0.4,0.0,0.0,0.0,0.0,0.09,0.8,1.0,0.82,0.8,0.85,0.63,0.31,0.18,0.19,0.2,0.01],
    [0.0,0.0,0.0,0.36,0.1,0.0,0.0,0.0,0.05,0.54,0.86,0.79,0.74,0.72,0.6,0.39,0.28,0.24,0.13,0.0],
    [0.0,0.0,0.0,0.01,0.3,0.07,0.0,0.0,0.08,0.36,0.64,0.7,0.64,0.6,0.51,0.39,0.29,0.19,0.04,0.0],
    [0.0,0.0,0.0,0.0,0.1,0.24,0.14,0.1,0.15,0.29,0.45,0.53,0.52,0.46,0.4,0.31,0.21,0.08,0.0,0.0],
    [0.0,0.0,0.0,0.0,0.0,0.08,0.21,0.21,0.22,0.29,0.36,0.39,0.37,0.33,0.26,0.18,0.09,0.0,0.0,0.0],
    [0.0,0.0,0.0,0.0,0.0,0.0,0.03,0.13,0.19,0.22,0.24,0.24,0.23,0.18,0.13,0.05,0.0,0.0,0.0,0.0],
    [0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.02,0.06,0.08,0.09,0.07,0.05,0.01,0.0,0.0,0.0,0.0,0.0],
];
/// The radius the table was found at.
const ORBIUM_R: f32 = 13.0;

/// The ring kernel's taps `(dx, dy, weight)`, weights summing to 1: every offset closer
/// than `R` whose weight is worth a multiply.
pub(crate) fn kernel() -> Vec<(i32, i32, f32)> {
    let reach = R as i32;
    let mut taps = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let d = ((dx * dx + dy * dy) as f32).sqrt() / R;
            if d >= 1.0 {
                continue;
            }
            let z = (d - 0.5) / 0.15;
            let w = (-0.5 * z * z).exp();
            if w > 5e-3 {
                taps.push((dx, dy, w));
            }
        }
    }
    let sum: f32 = taps.iter().map(|t| t.2).sum();
    for t in &mut taps {
        t.2 /= sum;
    }
    taps
}

/// An Orbium's velocity at quarter turn `turn` (the stamp turns `(u, v) → (−v, u)`).
fn velocity(turn: u8) -> (f64, f64) {
    let (mut x, mut y) = V0;
    for _ in 0..turn % 4 {
        (x, y) = (-y, x);
    }
    (x, y)
}

/// Lenia's growth mapping.
#[inline]
pub(crate) fn growth(u: f32) -> f32 {
    let z = (u - MU) / SIGMA;
    2.0 * (-0.5 * z * z).exp() - 1.0
}

/// The world: field `a` on a `size` periodic lattice, plus scratch.
#[derive(Debug, Clone)]
pub(crate) struct World {
    pub(crate) a: Grid<f32>,
    /// The state one step ago, for display interpolation.
    pub(crate) prev: Grid<f32>,
    pot: Vec<f32>,
    taps: Vec<(i32, i32, f32)>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            a: Grid::new(Size::new(1, 1), 0.0),
            prev: Grid::new(Size::new(1, 1), 0.0),
            pot: vec![0.0],
            taps: kernel(),
        }
    }
}

impl World {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            a: Grid::new(size, 0.0),
            prev: Grid::new(size, 0.0),
            pot: vec![0.0; size.area()],
            taps: kernel(),
        }
    }

    pub(crate) fn mass(&self) -> f32 {
        self.a.as_slice().iter().sum()
    }

    /// One Lenia step. The convolution runs tap by tap over whole rows, each wrapped row
    /// split into two contiguous runs, so the inner loop is a plain vectorisable `axpy`.
    pub(crate) fn step(&mut self) {
        let (w, h) = (usize::from(self.a.width()), usize::from(self.a.height()));
        self.prev.as_mut_slice().copy_from_slice(self.a.as_slice());
        self.pot.fill(0.0);
        let src = self.prev.as_slice();
        for &(dx, dy, k) in &self.taps {
            let sx = dx.rem_euclid(w as i32) as usize;
            for y in 0..h {
                let sy = (y as i32 + dy).rem_euclid(h as i32) as usize;
                let row = &src[sy * w..][..w];
                let out = &mut self.pot[y * w..][..w];
                let (out_a, out_b) = out.split_at_mut(w - sx);
                for (o, &s) in out_a.iter_mut().zip(&row[sx..]) {
                    *o += k * s;
                }
                for (o, &s) in out_b.iter_mut().zip(&row[..sx]) {
                    *o += k * s;
                }
            }
        }
        for (a, &u) in self.a.as_mut_slice().iter_mut().zip(&self.pot) {
            *a = (*a + DT * growth(u)).clamp(0.0, 1.0);
        }
    }

    /// Stamp an Orbium at `(cx, cy)`, rotated by `turns` quarter turns and mirrored if
    /// `flip` (the table, resampled bilinearly if `R` differs from 13).
    pub(crate) fn stamp_orbium(&mut self, (cx, cy): (i32, i32), turns: u8, flip: bool) {
        let scale = ORBIUM_R / R;
        let n = (20.0 / scale).round() as i32;
        let (w, h) = (i32::from(self.a.width()), i32::from(self.a.height()));
        let table = |x: f32, y: f32| -> f32 {
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            let at = |i: f32, j: f32| {
                if (0.0..20.0).contains(&i) && (0.0..20.0).contains(&j) {
                    ORBIUM[j as usize][i as usize]
                } else {
                    0.0
                }
            };
            let top = at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * fx;
            let bot = at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * fx;
            top + (bot - top) * fy
        };
        for j in 0..n {
            for i in 0..n {
                let (mut u, mut v) = (i, j);
                if flip {
                    u = n - 1 - u;
                }
                for _ in 0..turns % 4 {
                    (u, v) = (n - 1 - v, u);
                }
                let val = table(i as f32 * scale, j as f32 * scale);
                let (x, y) = (
                    (cx + u - n / 2).rem_euclid(w),
                    (cy + v - n / 2).rem_euclid(h),
                );
                if let Some(a) = self.a.get_mut(x, y) {
                    *a = a.max(val);
                }
            }
        }
    }

    /// Stage a meeting of two gliders at `(cx, cy)`, [`MEET_S`] seconds from now: `A`
    /// heading `turn`, `B` a quarter (engulf) or half (glance) turn on, offset sideways
    /// from a head-on course. The whole set-up turns with `turn`, so the (deterministic)
    /// outcome does too.
    pub(crate) fn stage(&mut self, act: Act, (cx, cy): (i32, i32), turn: u8) {
        let (tb, off) = match act {
            Act::Shoal => return,
            Act::Engulf => (turn + 1, ENGULF_OFF),
            Act::Glance => (turn + 2, GLANCE_OFF),
        };
        let (va, vb) = (velocity(turn), velocity(tb));
        let (rx, ry) = (vb.0 - va.0, vb.1 - va.1);
        let rl = rx.hypot(ry).max(1e-9);
        let (px, py) = (-ry / rl, rx / rl);
        let (mx, my) = (f64::from(cx), f64::from(cy));
        let at = |x: f64, y: f64| (x.round() as i32, y.round() as i32);
        self.stamp_orbium(at(mx - va.0 * MEET_S, my - va.1 * MEET_S), turn % 4, false);
        self.stamp_orbium(
            at(mx - vb.0 * MEET_S + px * off, my - vb.1 * MEET_S + py * off),
            tb % 4,
            false,
        );
    }

    /// The heaviest node (ties to the first), the centre of the damage beat.
    fn heaviest(&self) -> (i32, i32) {
        let w = i32::from(self.a.width());
        let (i, _) = self
            .a
            .as_slice()
            .iter()
            .enumerate()
            .fold((0, -1.0f32), |b, (i, &v)| if v > b.1 { (i, v) } else { b });
        (i as i32 % w, i as i32 / w)
    }

    /// The emptiest of a coarse grid of candidate places (least mass within 12 nodes,
    /// wrapping): where a new glider hatches without touching the others.
    pub(crate) fn emptiest(&self) -> (i32, i32) {
        let (w, h) = (i32::from(self.a.width()), i32::from(self.a.height()));
        let mut best = ((0, 0), f32::INFINITY);
        for gy in 0..4 {
            for gx in 0..6 {
                let (cx, cy) = (gx * w / 6 + w / 12, gy * h / 4 + h / 8);
                let mut m = 0.0;
                for dy in -12..=12 {
                    for dx in -12..=12 {
                        let (x, y) = ((cx + dx).rem_euclid(w), (cy + dy).rem_euclid(h));
                        m += self.a.get(x, y).copied().unwrap_or(0.0);
                    }
                }
                if m < best.1 {
                    best = ((cx, cy), m);
                }
            }
        }
        best.0
    }

    /// Zero a disc of radius `r` around `(cx, cy)` (wrapping).
    pub(crate) fn damage(&mut self, (cx, cy): (i32, i32), r: f32) {
        let (w, h) = (i32::from(self.a.width()), i32::from(self.a.height()));
        let reach = r.ceil() as i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if ((dx * dx + dy * dy) as f32) <= r * r {
                    let (x, y) = ((cx + dx).rem_euclid(w), (cy + dy).rem_euclid(h));
                    self.a.set(x, y, 0.0);
                }
            }
        }
    }

    /// Centre of mass on the torus (circular means per axis), in nodes.
    #[cfg(test)]
    pub(crate) fn centre(&self) -> (f64, f64) {
        let (w, h) = (f64::from(self.a.width()), f64::from(self.a.height()));
        let (mut cx, mut sx, mut cy, mut sy) = (0.0, 0.0, 0.0, 0.0);
        for ((x, y), &v) in self.a.iter() {
            let v = f64::from(v);
            let (s, c) = (std::f64::consts::TAU * f64::from(x) / w).sin_cos_det();
            cx += v * c;
            sx += v * s;
            let (s, c) = (std::f64::consts::TAU * f64::from(y) / h).sin_cos_det();
            cy += v * c;
            sy += v * s;
        }
        let ang = |s: f64, c: f64| s.atan2(c).rem_euclid(std::f64::consts::TAU);
        (
            ang(sx, cx) * w / std::f64::consts::TAU,
            ang(sy, cy) * h / std::f64::consts::TAU,
        )
    }
}

#[derive(Debug)]
struct Lenia {
    world: World,
    /// Seeded mass, the watchdog's reference.
    seed_mass: f32,
    /// Fractional world steps owed; also the display interpolation weight.
    owed: f64,
    /// Simulated seconds, for the damage beat.
    time: f64,
    next_damage: f64,
    seeded: bool,
    /// Shared heading of the shoal (quarter turns).
    turns: u8,
    /// Gliders seeded.
    gliders: usize,
    /// What this cycle staged.
    act: Act,
    /// When the next birth (the replacement of a lost glider) is due.
    hatch_at: Option<f64>,
    /// The last birth: node and time, for its fade-in.
    birth: Option<((i32, i32), f64)>,
    /// The creatures of the previous act (or a reseeded colony), still stepping while
    /// they dissolve, and when that cross-fade began (LNA-A9).
    ghost: World,
    ghost_at: Option<f64>,
    scratch: Grid<f32>,
    map: SampleMap,
    pixels: Pixels,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Lenia {
    fn default() -> Self {
        Self {
            dirty: true,
            world: World::default(),
            seed_mass: 1.0,
            owed: 0.0,
            time: 0.0,
            next_damage: DAMAGE_S,
            seeded: false,
            turns: 0,
            gliders: 1,
            act: Act::Shoal,
            hatch_at: None,
            birth: None,
            ghost: World::default(),
            ghost_at: None,
            scratch: Grid::default(),
            map: SampleMap::default(),
            pixels: Pixels::default(),
        }
    }
}

impl Lenia {
    /// Clear the world and seed this cycle's act at a random place and heading.
    fn seed(&mut self, rng: &mut impl Rng) {
        // The outgoing creatures become the ghost that cross-fades out (LNA-A9); buffers
        // are sized in `resize`, so this copies without allocating (GEN-R11).
        self.ghost_at = None;
        if self.ghost.a.size() == self.world.a.size() && self.world.mass() > 1e-3 {
            self.ghost
                .a
                .as_mut_slice()
                .copy_from_slice(self.world.a.as_slice());
            self.ghost
                .prev
                .as_mut_slice()
                .copy_from_slice(self.world.prev.as_slice());
            self.ghost_at = Some(self.time);
        }
        self.world.a.fill(0.0);
        self.hatch_at = None;
        self.birth = None;
        let size = self.world.a.size();
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        self.turns = rng.random_range(0..4);
        let (x0, y0) = (rng.random_range(0..w.max(1)), rng.random_range(0..h.max(1)));
        let side = size.width().min(size.height());
        self.act = match ACTS[(self.time / CYCLE_S) as usize % ACTS.len()] {
            Act::Engulf if side >= ENGULF_MIN => Act::Engulf,
            Act::Glance if side >= GLANCE_MIN => Act::Glance,
            _ => Act::Shoal,
        };
        if self.act == Act::Shoal {
            // One heading (and no mirroring, which would flip the heading) for the shoal:
            // gliders that swim in parallel at the same speed keep their spacing and never
            // merge into a chimera. They start on a diagonal, apart along both axes.
            let k = (size.area() / AREA_PER_GLIDER).clamp(1, MAX_GLIDERS);
            self.gliders = k;
            for i in 0..k as i32 {
                let c = (x0 + i * w / k as i32, y0 + i * h / k as i32);
                self.world.stamp_orbium(c, self.turns, false);
            }
        } else {
            self.gliders = 2;
            self.world.stage(self.act, (x0, y0), self.turns);
        }
        self.world
            .prev
            .as_mut_slice()
            .copy_from_slice(self.world.a.as_slice());
        self.seed_mass = self.world.mass().max(1e-3);
    }

    fn world_step(&mut self, rng: &mut impl Rng) {
        self.world.step();
        if let Some(t0) = self.ghost_at {
            if self.time - t0 < XFADE_S {
                self.ghost.step();
            } else {
                self.ghost_at = None;
            }
        }
        let m = self.world.mass() / self.seed_mass;
        if !(MASS_BAND.0..=MASS_BAND.1).contains(&m) {
            self.seed(rng);
        } else if self.gliders > 1 && m < 1.0 - 0.7 / self.gliders as f32 && self.hatch_at.is_none()
        {
            // A glider died (engulfed, or a fatal bite): a new one is born a little later
            // in the emptiest place, so the dish never thins out to a lone survivor.
            self.hatch_at = Some(self.time + BIRTH_DELAY);
        }
    }

    fn hatch(&mut self) {
        self.hatch_at = None;
        let c = self.world.emptiest();
        self.world.stamp_orbium(c, self.turns, false);
        self.birth = Some((c, self.time));
    }

    fn rasterise(&mut self) {
        let f = self.owed.clamp(0.0, 1.0) as f32;
        // Cross-fade weights (LNA-A9): new creatures in over the first half, the ghost
        // out over the second, so one of them is always at full strength.
        let (k_in, ghost) = match self.ghost_at {
            Some(t0) => {
                let u = 2.0 * (self.time - t0) / XFADE_S;
                let k_out = 1.0 - ease::smoothstep(u - 1.0);
                (
                    ease::smoothstep(u) as f32,
                    (k_out > 0.0).then_some((&self.ghost, k_out as f32)),
                )
            }
            None => (1.0, None),
        };
        // A newborn fades in (display only): pixels near its hatching place are dimmed
        // by its age, so it condenses out of the dark instead of popping in.
        let px = self.pixels.pixel_size();
        let lat = self.world.a.size();
        let sx = f64::from(px.width()) / f64::from(lat.width());
        let sy = f64::from(px.height()) / f64::from(lat.height());
        let birth = self.birth.and_then(|((bx, by), t0)| {
            let age = (self.time - t0) / BIRTH_FADE;
            (age < 1.0).then(|| {
                let (cx, cy) = ((f64::from(bx) + 0.5) * sx, (f64::from(by) + 0.5) * sy);
                (cx, cy, BIRTH_R * sx, ease::smoothstep(age.max(0.0)) as f32)
            })
        });
        let (map, a, p) = (&self.map, &self.world.a, &self.world.prev);
        // Row samplers, rebuilt when the shader reaches the next pixel row (CORE-R104).
        let (mut ry, mut ar, mut pr) = (-1, map.row(a, 0), map.row(p, 0));
        self.pixels.shade(|x, y| {
            if y != ry {
                (ry, ar, pr) = (y, map.row(a, y), map.row(p, y));
            }
            let (va, vp) = (ar.at(x), pr.at(x));
            let mut v = (vp + (va - vp) * f) * k_in;
            if let Some((cx, cy, r, k)) = birth {
                let d = (f64::from(x) + 0.5 - cx).hypot((f64::from(y) + 0.5 - cy) * sx / sy);
                if d < r {
                    v *= k;
                }
            }
            if let Some((g, k)) = ghost {
                let (ga, gp) = (map.sample(&g.a, x, y), map.sample(&g.prev, x, y));
                v = v.max((gp + (ga - gp) * f) * k);
            }
            lenia_ink(v)
        });
    }
}

/// The creature as a `Seq` field in four flat steps `(v from, Seq level)`, like M11's
/// reaction fields (ADR-0018): a violet membrane, red and orange flesh, a yellow core;
/// black below the first step.
const LENIA_STEPS: [(f32, u8); 4] = [(0.03, 60), (0.1, 125), (0.3, 185), (0.6, 240)];

fn lenia_ink(v: f32) -> Ink {
    if !v.is_finite() {
        return Ink::BLANK;
    }
    LENIA_STEPS
        .iter()
        .rev()
        .find(|(from, _)| v >= *from)
        .map_or(Ink::BLANK, |&(_, l)| Ink::new(Role::Seq, l))
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = raster_stronger(top, bottom)?;
    let i = usize::from(ink.level) * (ASCII_RAMP.len() - 1) / 255;
    Some((ASCII_RAMP[i], ink))
}

/// The brighter lit pixel of a cell, if any.
fn raster_stronger(top: Ink, bottom: Ink) -> Option<Ink> {
    let lit = |i: Ink| i.role != Role::Background;
    match (lit(top), lit(bottom)) {
        (false, false) => None,
        (true, false) => Some(top),
        (false, true) => Some(bottom),
        (true, true) => Some(if top.level >= bottom.level {
            top
        } else {
            bottom
        }),
    }
}

impl Generator for Lenia {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let lat = grid::lattice_for(px, MAX_NODES, BUCKET);
        if self.ghost.a.size() != lat {
            // A cross-fade in flight is dropped on a lattice change (the ghost is only
            // display, and resizes are rare).
            self.ghost = World::new(lat);
            self.ghost_at = None;
        }
        if !self.seeded {
            self.seeded = true;
            self.world = World::new(lat);
            self.seed(&mut ctx.rng);
        } else if lat != self.world.a.size() {
            // ADR-0012: carry the creatures over; a lattice too small for them reseeds.
            grid::regrid_bilinear(&mut self.world.a, lat, &mut self.scratch);
            grid::regrid_bilinear(&mut self.world.prev, lat, &mut self.scratch);
            self.world.pot.clear();
            self.world.pot.resize(lat.area(), 0.0);
            let m = self.world.mass() / self.seed_mass;
            if !(MASS_BAND.0..=MASS_BAND.1).contains(&m) {
                self.seed(&mut ctx.rng);
            }
        }
        self.map.rebuild(lat, px);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        let cycle = (self.time / CYCLE_S).floor();
        self.time += dt;
        if (self.time / CYCLE_S).floor() > cycle {
            // The next act on a new heading: the dish never settles into one picture.
            self.seed(&mut ctx.rng);
        }
        self.owed += dt * STEPS_PER_S;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            self.world_step(&mut ctx.rng);
        }
        if self.hatch_at.is_some_and(|t| self.time >= t) {
            self.hatch();
        }
        if self.time >= self.next_damage {
            self.next_damage += DAMAGE_S;
            let (x, y) = self.world.heaviest();
            let (s, c) = ctx
                .rng
                .random_range(0.0..std::f32::consts::TAU)
                .sin_cos_det();
            let bite = (
                x + (DAMAGE_OFFSET * c).round() as i32,
                y + (DAMAGE_OFFSET * s).round() as i32,
            );
            self.world.damage(bite, DAMAGE_R);
        }
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lone_orbium() -> World {
        let mut w = World::new(Size::new(64, 64));
        w.stamp_orbium((32, 32), 0, false);
        w
    }

    fn steps(secs: f64) -> usize {
        (secs * STEPS_PER_S).round() as usize
    }

    #[test]
    fn lna_a1_a2_a5_orbium_survives_moves_and_stays_in_unit_interval() {
        let mut w = lone_orbium();
        let (m0, c0) = (w.mass(), w.centre());
        for _ in 0..steps(30.0) {
            w.step();
            assert!(w.a.as_slice().iter().all(|v| (0.0..=1.0).contains(v)));
        }
        let m = w.mass();
        assert!((m / m0 - 1.0).abs() <= 0.15, "mass {m0} → {m}");
        let c = w.centre();
        // Torus distance.
        let d = |a: f64, b: f64| {
            let d = (a - b).abs();
            d.min(64.0 - d)
        };
        let moved = d(c.0, c0.0).hypot(d(c.1, c0.1));
        assert!(moved >= 5.0, "centre moved {moved:.1} nodes");
    }

    #[test]
    fn lna_a3_damaged_orbium_regrows() {
        let mut w = lone_orbium();
        for _ in 0..steps(5.0) {
            w.step();
        }
        let m0 = w.mass();
        let (cx, cy) = w.centre();
        w.damage(
            (cx.round() as i32, cy.round() as i32 + DAMAGE_OFFSET as i32),
            DAMAGE_R,
        );
        assert!(w.mass() < 0.95 * m0, "the damage removed mass");
        for _ in 0..steps(10.0) {
            w.step();
        }
        assert!(w.mass() >= 0.8 * m0, "mass {m0} → {}", w.mass());
    }

    #[test]
    fn lna_a4_stepping_cost_is_bounded() {
        // The stepping frame is the worst frame (benched by `worst_frame`); its cost is
        // taps × nodes, both capped.
        assert!(kernel().len() <= MAX_TAPS, "{} taps", kernel().len());
        let lat = grid::lattice_for(Size::new(200, 100), MAX_NODES, BUCKET);
        assert!(lat.area() <= MAX_NODES && lat.width() <= 96 * 2);
        let sum: f32 = kernel().iter().map(|t| t.2).sum();
        assert!((sum - 1.0).abs() < 1e-4);
    }

    #[test]
    fn lna_growth_peaks_at_mu() {
        assert_eq!(growth(MU), 1.0);
        assert!(growth(0.0) < -0.99 && growth(0.5) < -0.99);
    }

    /// Connected creatures (nodes > 0.1, 4-connected, wrapping).
    fn blobs(w: &World) -> usize {
        let (ww, hh) = (i32::from(w.a.width()), i32::from(w.a.height()));
        let mut seen = vec![false; w.a.len()];
        let mut n = 0;
        let mut stack = Vec::new();
        for i in 0..w.a.len() {
            if seen[i] || w.a.as_slice()[i] <= 0.1 {
                continue;
            }
            n += 1;
            stack.push(i);
            seen[i] = true;
            let mut size = 0;
            while let Some(j) = stack.pop() {
                size += 1;
                let (x, y) = (j as i32 % ww, j as i32 / ww);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let k = ((y + dy).rem_euclid(hh) * ww + (x + dx).rem_euclid(ww)) as usize;
                    if !seen[k] && w.a.as_slice()[k] > 0.1 {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
            if size < 8 {
                n -= 1;
            }
        }
        n
    }

    /// Mass (relative to the pair) and creature count `secs` after a staged encounter.
    fn after_encounter(size: Size, act: Act, turn: u8, secs: f64) -> (f32, usize) {
        let mut w = World::new(size);
        let c = (i32::from(size.width()) / 2, i32::from(size.height()) / 2);
        w.stage(act, c, turn);
        let m0 = w.mass();
        for _ in 0..steps(secs) {
            w.step();
        }
        (w.mass() / m0, blobs(&w))
    }

    #[test]
    fn lna_a6_an_aimed_collision_ends_with_one_clean_survivor() {
        for (size, turn) in [
            (Size::new(96, 64), 0),
            (Size::new(88, 60), 0),
            (Size::new(96, 64), 2),
            (Size::new(80, 64), 1),
            (Size::new(80, 48), 3),
        ] {
            for secs in [6.0, 10.0] {
                let (m, n) = after_encounter(size, Act::Engulf, turn, secs);
                assert!(
                    n == 1 && (0.4..0.6).contains(&m),
                    "{size:?} turn {turn} at {secs} s: {n} creatures, mass {m:.2}"
                );
            }
        }
    }

    #[test]
    fn lna_a7_a_glancing_meeting_leaves_both_alive() {
        for (size, turn) in [(Size::new(96, 64), 0), (Size::new(88, 60), 0)] {
            let (m, n) = after_encounter(size, Act::Glance, turn, 8.0);
            assert!(n == 2 && m > 0.9, "{size:?}: {n} creatures, mass {m:.2}");
        }
    }

    #[test]
    fn lna_a8_a_lost_glider_is_replaced_by_a_birth_after_a_delay() {
        let mut ctx = Ctx::new(8, Size::new(160, 50));
        let mut g = Lenia::default();
        g.resize(Size::new(160, 50), &mut ctx);
        g.prepare();
        // Kill every creature but the heaviest.
        let (hx, hy) = g.world.heaviest();
        let (w, h) = (i32::from(g.world.a.width()), i32::from(g.world.a.height()));
        for (i, v) in g.world.a.as_mut_slice().iter_mut().enumerate() {
            let (x, y) = (i as i32 % w, i as i32 / w);
            let (dx, dy) = ((x - hx).rem_euclid(w), (y - hy).rem_euclid(h));
            if dx.min(w - dx).max(dy.min(h - dy)) > 12 {
                *v = 0.0;
            }
        }
        g.world
            .prev
            .as_mut_slice()
            .copy_from_slice(g.world.a.as_slice());
        let m_one = g.world.mass();
        g.update(0.1, &mut ctx);
        g.prepare();
        assert!(g.hatch_at.is_some(), "a birth is scheduled");
        let born = g.hatch_at.unwrap_or(0.0);
        while g.time < born - 0.2 {
            g.update(1.0 / 60.0, &mut ctx);
            g.prepare();
        }
        assert!(g.world.mass() < 1.3 * m_one, "no birth before the delay");
        while g.time < born + 0.5 {
            g.update(1.0 / 60.0, &mut ctx);
            g.prepare();
        }
        assert!(g.world.mass() > 1.6 * m_one, "a glider hatched");
        assert!(g.birth.is_some());
    }

    /// Lit creature pixels: shaded at least mid-level (a body, not a faint halo).
    fn lit_pixels(g: &Lenia) -> usize {
        let px = g.pixels.pixel_size();
        let (w, h) = (i32::from(px.width()), i32::from(px.height()));
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                g.pixels
                    .get(x, y)
                    .is_some_and(|i| i.role != Role::Background && i.level >= 110)
            })
            .count()
    }

    #[test]
    fn lna_a9_never_blank_across_acts_births_and_reseeds() {
        // Two full act loops (shoal → engulf → shoal → glance, 4 × 30 s), sampled every
        // 0.25 s: at act changes the next act must already be showing (LNA-A9).
        let secs = 2.0 * ACTS.len() as f64 * CYCLE_S;
        for size in [Size::new(80, 24), Size::new(120, 40)] {
            let mut ctx = Ctx::new(42, size);
            let mut g = Lenia::default();
            g.resize(size, &mut ctx);
            g.prepare();
            let mut frame = 0usize;
            while g.time < secs {
                g.update(1.0 / 60.0, &mut ctx);
                g.prepare();
                frame += 1;
                if frame % 15 == 0 {
                    let n = lit_pixels(&g);
                    assert!(n >= 20, "{size:?} t = {:.2} s: {n} lit pixels", g.time);
                }
            }
        }
    }

    #[test]
    fn lna_watchdog_reseeds_an_empty_world() {
        let mut ctx = Ctx::new(42, Size::new(80, 24));
        let mut g = Lenia::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        g.world.a.fill(0.0);
        g.update(0.1, &mut ctx);
        g.prepare();
        assert!(g.world.mass() > 0.5 * g.seed_mass);
    }

    #[test]
    fn lna_a10_the_creature_is_a_stepped_seq_field_on_black() {
        assert_eq!(lenia_ink(0.0), Ink::BLANK);
        assert_eq!(lenia_ink(1.0), Ink::new(Role::Seq, 240));
        let mut levels: Vec<u8> = (0..=100)
            .map(|i| lenia_ink(i as f32 / 100.0).level)
            .collect();
        levels.dedup();
        assert_eq!(levels.len(), 1 + LENIA_STEPS.len(), "{levels:?}");
    }
}
