//! A slime mould solving a transport problem in time-lapse: an inoculum spills outward in a
//! lace of veins, finds the oat flakes and thickens into a few tubes that link them, then
//! starves and dissolves (SPEC-002 `physarum`).
//!
//! The model is Jones's (2010) agent-based approximation of *Physarum polycephalum*.
//! Agents on a capped square lattice (ADR-0012) move one site per step along one of 16
//! headings, each depositing chemoattractant where it lands, and turn toward the strongest
//! of three forward sensors. The trail diffuses and decays, so their positive feedback
//! condenses a web of veins that coarsens into a transport network. Food discs pinned at a
//! high concentration anchor trunk lines between the sources, as in Nakagaki et al. (2000)
//! and Tero et al. (2010).
//!
//! `update` runs the Jones steps; `prepare` draws a lightly time-smoothed copy of the
//! trail (Look (M18), GEN-R26: stipple veins + food atoms) as a braille probability
//! stipple, `Hot` filling in to `Sun` toward a tube's core, with a 1-px `Sun` ridge
//! thread on the strongest tubes and each oat flake a small `Cold` stipple atom
//! (CORE-R108); `render` only blits.

use std::fmt::Write as _;

use rand::Rng;
use rand::seq::SliceRandom;
use sciart_core::draw;
use sciart_core::grid::{self, SampleMap};
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};

pub static META: Meta = Meta {
    name: "physarum",
    category: Category::Biology,
    description: "A slime mould spreading a lace of veins that thickens into a transport network",
    title: "Slime mould · trails reinforce",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Physarum::new())
}

/// The lattice holds at most this many sites (ADR-0012, CORE-R48), in aspect buckets of
/// 1.25.
const MAX_SITES: usize = 20_000;
const BUCKET: f64 = 1.25;
/// Agents per lattice site (5 %), within the population range Jones explored.
const DENSITY: f64 = 0.05;
/// Every agent buffer is allocated once at this size (GEN-R11).
const MAX_AGENTS: usize = (DENSITY * MAX_SITES as f64) as usize;
/// Chemoattractant deposited per move, and its decay per step.
const DEP_T: f32 = 5.0;
const DECAY: f32 = 0.1;
/// Food discs (radius 1.5 sites, so the 3×3 block) are held at `T ≥ 4·depT`.
const FOOD_T: f32 = 20.0;
const MAX_FOOD: usize = 9;
/// Sensor angle and rotation angle in units of 22.5°: SA = 22.5°, RA = 45°.
const SA: i8 = 1;
const RA: i8 = 2;
/// One Jones step per `SIM_DT` (60 per simulated second, a time-lapse), and at most this
/// many per update.
const STEP: f64 = SIM_DT;
const MAX_STEPS: u32 = 15;

/// The 48 s life cycle of one colony.
const BEATS: [(&str, f64); 4] = [
    ("explore", 10.0),
    ("consolidate", 30.0),
    ("fade", 6.0),
    ("rest", 2.0),
];
const EXPLORE: usize = 0;
const FADE: usize = 2;
const REST: usize = 3;
const FADE_SECS: f64 = BEATS[FADE].1;

/// `T_ref` follows the network's bright half with this time constant (s), faster while a
/// fresh inoculum piles up deposits, so the opening bloom keeps its tonal detail.
const T_REF_TAU: f64 = 1.0;
const T_REF_TAU_BLOOM: f64 = 0.25;
const BLOOM_SECS: f64 = 1.5;
/// During the bloom `T_ref` is at least this fraction of the peak, so `#` (`V ≥ 2.03
/// T_ref`) is left for the top ≈ 10 % of the drop's range.
const BLOOM_PEAK: f32 = 0.45;
/// The drawn field trails `V` with this time constant (s): per-step deposit noise at the
/// tube edges averages out, while the network's growth and pulsing still show.
const SHOWN_TAU: f64 = 0.15;
/// The old flakes fade out over the last second of the fade, so rest opens on an empty
/// plate.
const FLAKE_OUT: f64 = 1.0;
/// Pixels below this tone (`T < T_ref / 2`) stay blank: the plate is the terminal's own
/// dark. A lower floor lights the diffuse attractant around every tube, which fattens
/// the veins into blobs.
const U_FLOOR: f64 = 0.45;
/// Look (M18): the vein stipple's density at full tone, the tone where its dots turn
/// from `Hot` to `Sun`, and the tone a ridge dot needs. The spec's draft `u ≥ 0.8` put
/// ridges only in the junction blobs; `0.62` threads every established tube.
const VEIN_PEAK: f64 = 0.85;
const SUN_U: f64 = 0.8;
const SUN_V: f32 = (SUN_U * SUN_U) as f32;
const RIDGE_U: f64 = 0.62;
const RIDGE_V: f32 = (RIDGE_U * RIDGE_U) as f32;
/// A ridge dot's fall across the tube over ±2 dots (`2v − v₋₂ − v₊₂`) is at least this
/// share of its value, so flat junction blobs grow no maze of stubs (`is_ridge`).
const RIDGE_FALL: f32 = 0.12;
/// The ridge test reads two dot rows either side: the sampled rows are a ring of five.
const REACH: usize = 2;
const RING: usize = 2 * REACH + 1;
const VEIN_INK: Ink = Ink::new(Role::Hot, 200);
const CORE_INK: Ink = Ink::new(Role::Sun, 230);
const RIDGE_INK: Ink = Ink::new(Role::Sun, 255);
/// Fixed stipple thresholds (CORE-R107): the veins' layer and the flakes' atoms.
const VEIN_SALT: u64 = 0x5851_F42D_4C95_7F2D;
const FLAKE_SALT: u64 = 0x1405_7B7E_F767_814F;
/// An oat flake: a `Cold` 230 atom of σ 1.6 dots, peak density 0.95 (CORE-R108).
const FLAKE_LEVEL: u8 = 230;
const FLAKE_SIGMA: f64 = 1.6;
const FLAKE_PEAK: f64 = 0.95;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// `(cos, sin)` of the 16 headings, `k · 22.5°` with y down.
pub(crate) const DIRS: [(f32, f32); 16] = {
    const C1: f32 = 0.923_879_5;
    const C2: f32 = 0.707_106_77;
    const C3: f32 = 0.382_683_43;
    [
        (1.0, 0.0),
        (C1, C3),
        (C2, C2),
        (C3, C1),
        (0.0, 1.0),
        (-C3, C1),
        (-C2, C2),
        (-C1, C3),
        (-1.0, 0.0),
        (-C1, -C3),
        (-C2, -C2),
        (-C3, -C1),
        (0.0, -1.0),
        (C3, -C1),
        (C2, -C2),
        (C1, -C3),
    ]
};

/// Jones's (2010) rotation rule for the forward, left (`h − SA`) and right (`h + SA`)
/// sensor readings, in 22.5° units with + toward the right sensor (PHY-A1): keep going
/// if forward is strictly strongest; if it is strictly weakest, turn ±RA by `coin`;
/// otherwise turn toward the strictly stronger side, and keep going on a tie.
pub(crate) fn steer(f: f32, fl: f32, fr: f32, coin: bool) -> i8 {
    if f > fl && f > fr {
        0
    } else if f < fl && f < fr {
        if coin { RA } else { -RA }
    } else if fl < fr {
        RA
    } else if fr < fl {
        -RA
    } else {
        0
    }
}

/// Rotate a heading in Z₁₆ by `turn` units of 22.5° (+ = clockwise on screen).
#[inline]
pub(crate) fn rotate(h: u8, turn: i8) -> u8 {
    (i16::from(h) + i16::from(turn)).rem_euclid(16) as u8
}

/// One diffuse-and-decay step, `T ← (1 − δ)·mean₃ₓ₃(T)` as a separable 3-tap box filter
/// through `scratch` (PHY-A2). The plate edge absorbs: sites off the lattice count as 0,
/// so attractant leaks out there and the network does not run along the frame. An
/// average never exceeds its inputs, so the field stays non-negative and its maximum
/// shrinks by at least `1 − δ`. Returns the new total (the fused mean for `T_ref`).
pub(crate) fn diffuse_decay(t: &mut Grid<f32>, scratch: &mut Grid<f32>, decay: f32) -> f32 {
    let (w, h) = (usize::from(t.width()), usize::from(t.height()));
    if w == 0 || h == 0 {
        return 0.0;
    }
    if scratch.size() != t.size() {
        scratch.resize(t.size(), 0.0);
    }
    for (src, dst) in t
        .as_slice()
        .chunks_exact(w)
        .zip(scratch.as_mut_slice().chunks_exact_mut(w))
    {
        for x in 0..w {
            let l = if x == 0 { 0.0 } else { src[x - 1] };
            let r = if x + 1 == w { 0.0 } else { src[x + 1] };
            dst[x] = (l + src[x] + r) / 3.0;
        }
    }
    let k = (1.0 - decay) / 3.0;
    let s = scratch.as_slice();
    let mut total = 0.0f32;
    for (y, row) in t.as_mut_slice().chunks_exact_mut(w).enumerate() {
        let c = &s[y * w..(y + 1) * w];
        let n = if y == 0 {
            None
        } else {
            Some(&s[(y - 1) * w..y * w])
        };
        let so = if y + 1 == h {
            None
        } else {
            Some(&s[(y + 1) * w..(y + 2) * w])
        };
        let mut row_sum = 0.0f32;
        for (x, v) in row.iter_mut().enumerate() {
            let above = n.map_or(0.0, |r| r[x]);
            let below = so.map_or(0.0, |r| r[x]);
            *v = (above + c[x] + below) * k;
            row_sum += *v;
        }
        total += row_sum;
    }
    total
}

/// Mean over the sites above the global `mean` (the brightness of the network itself,
/// which `T_ref` tracks) and the field's peak.
fn bright_mean(t: &[f32], mean: f32) -> (f32, f32) {
    let (sum, n, peak) = t.iter().fold((0.0f32, 0u32, 0.0f32), |(s, n, p), &v| {
        if v > mean {
            (s + v, n + 1, p.max(v))
        } else {
            (s, n, p)
        }
    });
    if n == 0 {
        (mean, mean)
    } else {
        (sum / n as f32, peak)
    }
}

/// Per-step weight of an EMA with time constant `tau` seconds.
fn ema_weight(tau: f64) -> f32 {
    (1.0 - (-STEP / tau).exp()) as f32
}

/// Agents for a lattice: 5 % of the sites, at least 1.
fn population(lattice: Size) -> usize {
    ((DENSITY * lattice.area() as f64).round() as usize).clamp(1, MAX_AGENTS)
}

/// Sensor offset: 9 % of the shorter side, 3..=9 sites.
fn sensor_offset(lattice: Size) -> f32 {
    (0.09 * f64::from(lattice.width().min(lattice.height())))
        .round()
        .clamp(3.0, 9.0) as f32
}

/// Oat flakes per plate: one per 800 sites, 4..=9.
fn food_count(lattice: Size) -> usize {
    ((lattice.area() as f64 / 800.0).round() as usize).clamp(4, MAX_FOOD)
}

/// Nearest lattice site of a continuous position.
#[inline]
fn site(x: f32, y: f32) -> (i32, i32) {
    (x.round() as i32, y.round() as i32)
}

/// The next colony's layout: its inoculum centre and its oat flakes (lattice sites).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Plan {
    centre: (f32, f32),
    food: [(i32, i32); MAX_FOOD],
    n_food: usize,
}

impl Plan {
    const EMPTY: Plan = Plan {
        centre: (0.0, 0.0),
        food: [(0, 0); MAX_FOOD],
        n_food: 0,
    };

    /// Radius of an inoculum of `n` agents at 50 % occupancy.
    fn inoculum_radius(n: usize) -> f32 {
        (2.0 * n as f32 / std::f32::consts::PI).sqrt()
    }

    /// A seeded layout (SPEC-002 Food, Inoculum): the centre in the plate's central 40 %,
    /// then up to `n_food` flakes in the central 80 %, at least a quarter of the shorter
    /// side apart and beyond the inoculum's rim by `max(2·SO, 0.15·min(lw, lh))`, so the
    /// colony has to search before it finds one, within 200 tries.
    fn draw(lattice: Size, n_agents: usize, n_food: usize, so: f32, rng: &mut impl Rng) -> Plan {
        let (w, h) = (f32::from(lattice.width()), f32::from(lattice.height()));
        let mut plan = Plan::EMPTY;
        plan.centre = (
            w * rng.random_range(0.3..0.7) - 0.5,
            h * rng.random_range(0.3..0.7) - 0.5,
        );
        let gap = 0.25 * w.min(h);
        let clear = Self::inoculum_radius(n_agents) + (2.0 * so).max(0.15 * w.min(h));
        let n_food = n_food.min(MAX_FOOD);
        for _ in 0..200 {
            if plan.n_food == n_food {
                break;
            }
            let f = (
                ((w * rng.random_range(0.1..0.9)) as i32).min(i32::from(lattice.width()) - 1),
                ((h * rng.random_range(0.1..0.9)) as i32).min(i32::from(lattice.height()) - 1),
            );
            let dist = |a: (f32, f32)| (f.0 as f32 - a.0).hypot(f.1 as f32 - a.1);
            let spaced = plan.food[..plan.n_food]
                .iter()
                .all(|&g| dist((g.0 as f32, g.1 as f32)) >= gap);
            if spaced && dist(plan.centre) >= clear {
                plan.food[plan.n_food] = f;
                plan.n_food += 1;
            }
        }
        plan
    }

    fn food(&self) -> &[(i32, i32)] {
        &self.food[..self.n_food]
    }

    /// Map the layout onto a resized lattice, cell-centre aligned.
    fn rescale(&mut self, from: Size, to: Size) {
        self.centre = remap(self.centre, from, to);
        for f in &mut self.food[..self.n_food] {
            let (x, y) = remap((f.0 as f32, f.1 as f32), from, to);
            let (x, y) = site(x, y);
            *f = (
                x.clamp(0, i32::from(to.width()) - 1),
                y.clamp(0, i32::from(to.height()) - 1),
            );
        }
    }
}

/// A position on a `from` grid mapped onto a `to` grid, cell-centre aligned (CORE-R68).
#[inline]
fn remap((x, y): (f32, f32), from: Size, to: Size) -> (f32, f32) {
    let axis = |v: f32, f: u16, t: u16| {
        grid::centre_of(f64::from(v), usize::from(f), usize::from(t)) as f32
    };
    (
        axis(x, from.width(), to.width()),
        axis(y, from.height(), to.height()),
    )
}

/// One agent: a continuous position, a heading in Z₁₆, and when it dies in the fade.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Agent {
    x: f32,
    y: f32,
    h: u8,
    death: f32,
}

/// The colony on its lattice: the sensed trail `T`, the drawn field `V`, occupancy,
/// agents and food. It knows nothing of pixels or beats; the scene passes fade times to
/// `starve`.
#[derive(Debug, Clone)]
pub(crate) struct Colony {
    /// The chemoattractant the agents sense, food pinning included.
    trail: Grid<f32>,
    /// What is drawn (through the scene's short display EMA): the same deposits,
    /// diffusion and decay without the food pinning, so a flake shows no glow until the
    /// organism reaches it.
    vis: Grid<f32>,
    /// Total of `vis` after the last step (fused into its diffusion).
    vis_sum: f32,
    scratch: Grid<f32>,
    /// One agent per site at most (particle exclusion).
    occ: Grid<bool>,
    agents: Vec<Agent>,
    order: Vec<u16>,
    plan: Plan,
    so: f32,
}

impl Colony {
    fn new(lattice: Size) -> Self {
        Self {
            trail: Grid::new(lattice, 0.0),
            vis: Grid::new(lattice, 0.0),
            vis_sum: 0.0,
            scratch: Grid::new(lattice, 0.0),
            occ: Grid::new(lattice, false),
            agents: Vec::with_capacity(MAX_AGENTS),
            order: Vec::with_capacity(MAX_AGENTS),
            plan: Plan::EMPTY,
            so: sensor_offset(lattice),
        }
    }

    fn lattice(&self) -> Size {
        self.trail.size()
    }

    /// Mean of `vis` over the sites above its mean (the drawn network's brightness), and
    /// its peak.
    fn vis_bright_mean(&self) -> (f32, f32) {
        bright_mean(
            self.vis.as_slice(),
            self.vis_sum / self.vis.len().max(1) as f32,
        )
    }

    /// Deposit `DEP_T` at a site of both fields.
    #[inline]
    fn deposit(&mut self, x: i32, y: i32) {
        if let Some(t) = self.trail.get_mut(x, y) {
            *t += DEP_T;
        }
        if let Some(v) = self.vis.get_mut(x, y) {
            *v += DEP_T;
        }
    }

    /// A fresh colony of `n` agents at `plan`'s inoculum: random free sites in its disc,
    /// random headings, one deposit each so the colony shows at once, and a seeded death
    /// time in the fade. The drawn field is then blurred twice (no decay), so the first
    /// frame is a soft drop rather than a checkerboard of single deposits.
    fn inoculate(&mut self, plan: Plan, n: usize, rng: &mut impl Rng) {
        self.plan = plan;
        self.agents.clear();
        self.occ.fill(false);
        let r = Plan::inoculum_radius(n);
        for _ in 0..n.min(MAX_AGENTS) {
            for _ in 0..32 {
                let d = r * rng.random::<f32>().sqrt();
                let (s, c) = (std::f32::consts::TAU * rng.random::<f32>()).sin_cos_det();
                let (x, y) = (plan.centre.0 + d * c, plan.centre.1 + d * s);
                let (sx, sy) = site(x, y);
                if self.occ.get(sx, sy) == Some(&false) {
                    self.occ.set(sx, sy, true);
                    self.deposit(sx, sy);
                    let (h, death) = (
                        rng.random_range(0..16),
                        rng.random_range(0.0..FADE_SECS) as f32,
                    );
                    self.agents.push(Agent {
                        x: sx as f32,
                        y: sy as f32,
                        h,
                        death,
                    });
                    break;
                }
            }
        }
        diffuse_decay(&mut self.vis, &mut self.scratch, 0.0);
        self.vis_sum = diffuse_decay(&mut self.vis, &mut self.scratch, 0.0);
    }

    /// Chemoattractant at the site nearest `(x, y)`; off the lattice reads −1, so walls
    /// repel.
    #[inline]
    fn sense(&self, x: f32, y: f32) -> f32 {
        let (sx, sy) = site(x, y);
        self.trail.get(sx, sy).copied().unwrap_or(-1.0)
    }

    /// One Jones step (PHY-A3): a motor pass and a sensory pass over one fresh shuffle
    /// of the agents, then diffusion and decay, then (unless starving) the food pinning.
    fn step(&mut self, rng: &mut impl Rng, feed: bool) {
        let mut rng = SplitMix64::from_rng(rng);
        self.order.clear();
        self.order.extend(0..self.agents.len() as u16);
        self.order.shuffle(&mut rng);
        self.motor(&mut rng);
        self.sensory(&mut rng);
        diffuse_decay(&mut self.trail, &mut self.scratch, DECAY);
        self.vis_sum = diffuse_decay(&mut self.vis, &mut self.scratch, DECAY);
        if feed {
            self.pin_food();
        }
    }

    /// Motor pass in `order`: step one site along the heading onto a free site and
    /// deposit there, or stay and pick a random heading.
    fn motor(&mut self, rng: &mut SplitMix64) {
        for i in 0..self.order.len() {
            let k = usize::from(self.order[i]);
            let a = self.agents[k];
            let (dx, dy) = DIRS[usize::from(a.h)];
            let (nx, ny) = (a.x + dx, a.y + dy);
            let (from, to) = (site(a.x, a.y), site(nx, ny));
            if to == from || self.occ.get(to.0, to.1) == Some(&false) {
                self.occ.set(from.0, from.1, false);
                self.occ.set(to.0, to.1, true);
                (self.agents[k].x, self.agents[k].y) = (nx, ny);
                self.deposit(to.0, to.1);
            } else {
                self.agents[k].h = rng.below(16) as u8;
            }
        }
    }

    /// Sensory pass in `order`: sample forward, left (`h − SA`) and right (`h + SA`) at
    /// distance SO and turn by Jones's rule.
    fn sensory(&mut self, rng: &mut SplitMix64) {
        for &k in &self.order {
            let a = self.agents[usize::from(k)];
            let probe = |turn: i8| {
                let (dx, dy) = DIRS[usize::from(rotate(a.h, turn))];
                self.sense(a.x + self.so * dx, a.y + self.so * dy)
            };
            let turn = steer(probe(0), probe(-SA), probe(SA), rng.below(2) == 1);
            self.agents[usize::from(k)].h = rotate(a.h, turn);
        }
    }

    /// Hold every food disc (radius 1.5 sites: the 3×3 block) at `T ≥ FOOD_T`, which
    /// anchors trunk lines between the sources.
    fn pin_food(&mut self) {
        for &(fx, fy) in self.plan.food() {
            for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                if let Some(t) = self.trail.get_mut(fx + dx, fy + dy) {
                    *t = t.max(FOOD_T);
                }
            }
        }
    }

    /// Starvation: every agent whose death time falls before the end of this step (at
    /// `local` seconds into the fade) dies and frees its site.
    fn starve(&mut self, local: f64) {
        let end = (local + STEP) as f32;
        let occ = &mut self.occ;
        self.agents.retain(|a| {
            let alive = a.death >= end;
            if !alive {
                let (sx, sy) = site(a.x, a.y);
                occ.set(sx, sy, false);
            }
            alive
        });
    }

    /// Carry the colony onto a new lattice (ADR-0012): resample both fields, scale the
    /// agents and re-insert them in index order (dropping collisions), and scale the
    /// food. With a `target` (outside fade and rest) the population is then trimmed
    /// evenly in index order, or topped up on free sites above the mean trail, to
    /// `target` agents, so the density stays the model's 5 %.
    fn rescale(&mut self, lattice: Size, target: Option<usize>, rng: &mut impl Rng) {
        let from = self.lattice();
        for field in [&mut self.trail, &mut self.vis] {
            grid::regrid_bilinear(field, lattice, &mut self.scratch);
        }
        self.vis_sum = self.vis.as_slice().iter().sum();
        self.scratch.resize(lattice, 0.0);
        self.occ.resize(lattice, false);
        self.occ.fill(false);
        let occ = &mut self.occ;
        let (w, h) = (i32::from(lattice.width()), i32::from(lattice.height()));
        self.agents.retain_mut(|a| {
            let (x, y) = remap((a.x, a.y), from, lattice);
            let (sx, sy) = site(x, y);
            let (sx, sy) = (sx.clamp(0, w - 1), sy.clamp(0, h - 1));
            (a.x, a.y) = (sx as f32, sy as f32);
            let free = occ.get(sx, sy) == Some(&false);
            occ.set(sx, sy, true);
            free
        });
        self.plan.rescale(from, lattice);
        self.so = sensor_offset(lattice);

        let Some(target) = target.map(|t| t.min(MAX_AGENTS)) else {
            return;
        };
        let len = self.agents.len();
        if len > target {
            // Keep agent i when ⌊(i + 1)·target/len⌋ steps up: exactly `target`, spread
            // evenly over the index order, with no randomness.
            let (occ, mut i) = (&mut self.occ, 0);
            self.agents.retain(|a| {
                let keep = (i + 1) * target / len > i * target / len;
                i += 1;
                if !keep {
                    let (sx, sy) = site(a.x, a.y);
                    occ.set(sx, sy, false);
                }
                keep
            });
        }
        let mean = self.trail.as_slice().iter().sum::<f32>() / lattice.area() as f32;
        for _ in self.agents.len()..target {
            for _ in 0..16 {
                let (sx, sy) = (rng.random_range(0..w), rng.random_range(0..h));
                let rich = self.trail.get(sx, sy).is_some_and(|&t| t > mean);
                if rich && self.occ.get(sx, sy) == Some(&false) {
                    self.occ.set(sx, sy, true);
                    let (h, death) = (
                        rng.random_range(0..16),
                        rng.random_range(0.0..FADE_SECS) as f32,
                    );
                    self.agents.push(Agent {
                        x: sx as f32,
                        y: sy as f32,
                        h,
                        death,
                    });
                    break;
                }
            }
        }
    }
}

/// The scene: a colony, its 48 s life cycle, and the display.
#[derive(Debug)]
pub(crate) struct Physarum {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    colony: Colony,
    /// The next cycle's layout, drawn at the start of each rest.
    next: Plan,
    timeline: Timeline,
    /// Jones steps taken; scene time is `steps · STEP`.
    steps: u64,
    clock: FixedStep,
    /// Tone-map reference: an EMA of the network's bright-half mean, frozen while it dies.
    t_ref: f32,
    /// Per-step EMA weights: `T_ref` (settled, bloom) and the drawn field.
    k_ref: f32,
    k_bloom: f32,
    k_shown: f32,
    /// What is drawn: `V` through a short EMA (display only; agents sense `T`).
    shown: Grid<f32>,
    /// Bilinear taps from the lattice onto the plate's braille dots.
    map: SampleMap,
    /// Per dot row, the two lattice rows its bilinear sample reads (built in `resize`),
    /// and per lattice row its peak (each raster), so dark dot rows skip sampling.
    row_src: Vec<(u16, u16)>,
    row_peak: Vec<f32>,
    /// Per lattice column `i`, the first dot column whose left bilinear tap is `≥ i`
    /// (`lw + 1` entries), so whole dark spans of a dot row are skipped.
    col_start: Vec<u32>,
    /// The vein stipple's density per exposed field value.
    density: DensityTable,
    /// Three dot rows of the sampled, exposed field (`S / (2.5·T_ref)`), as a ring, for
    /// the ridge test across rows.
    rows: Vec<f32>,
    /// The vein stipple and its ridge threads (Look (M18)).
    veins: Dots,
    /// The ridge threads, blitted over the veins so a cell a thread crosses shows the
    /// thread alone: a crisp 1-px line through the stipple.
    ridge: Dots,
    /// The oat flakes as `Cold` atoms, blitted over the veins so each stays crisp.
    atoms: Dots,
    /// The drawn flakes' centres in dots and their ink (the ASCII `@` marks too).
    flakes: [(f64, f64); MAX_FOOD],
    n_flakes: usize,
    flake_ink: Ink,
    cells: Size,
    label: StackText<24>,
    ready: bool,
}

impl Physarum {
    fn new() -> Self {
        Self {
            dirty: false,
            colony: Colony::new(Size::new(1, 1)),
            next: Plan::EMPTY,
            timeline: Timeline::new(&BEATS),
            steps: 0,
            clock: FixedStep::new(STEP, MAX_STEPS),
            t_ref: DEP_T,
            k_ref: ema_weight(T_REF_TAU),
            k_bloom: ema_weight(T_REF_TAU_BLOOM),
            k_shown: ema_weight(SHOWN_TAU),
            shown: Grid::default(),
            map: SampleMap::default(),
            row_src: Vec::new(),
            row_peak: Vec::new(),
            col_start: Vec::new(),
            density: DensityTable::new(),
            rows: Vec::new(),
            veins: Dots::default(),
            ridge: Dots::default(),
            atoms: Dots::default(),
            flakes: [(0.0, 0.0); MAX_FOOD],
            n_flakes: 0,
            flake_ink: Ink::BLANK,
            cells: Size::new(1, 1),
            label: StackText::new(),
            ready: false,
        }
    }

    fn time(&self) -> f64 {
        self.steps as f64 * STEP
    }

    fn draw_plan(&self, rng: &mut impl Rng) -> Plan {
        let lattice = self.colony.lattice();
        Plan::draw(
            lattice,
            population(lattice),
            food_count(lattice),
            self.colony.so,
            rng,
        )
    }

    /// One Jones step of the life cycle, over `[t₀, t₁]`: starvation during the fade and
    /// the colony's step (both decided at `t₀`), then the events at `t₁`, the next layout
    /// at rest start and a new colony at each cycle boundary. Because the events land in
    /// the state before `time()` reads `t₁`, `rasterise` never sees a clock that has run
    /// ahead of the colony (PHY-A5).
    fn step(&mut self, rng: &mut impl Rng) {
        let (t0, t1) = (self.time(), (self.steps + 1) as f64 * STEP);
        let start = self.timeline.at(t0);
        if start.index >= FADE {
            self.colony.starve(if start.index == FADE {
                start.local
            } else {
                FADE_SECS
            });
        }
        let feeding = start.index < FADE;
        self.colony.step(rng, feeding);
        if feeding {
            let (bright, peak) = self.colony.vis_bright_mean();
            // A young inoculum is a flat stack of deposits about 2.5× its bright-half
            // mean, which that reference alone paints as one saturated disc; exposing for
            // its peak keeps the dense core for the hottest spots of the drop.
            let (target, k) = if start.index == EXPLORE && start.local < BLOOM_SECS {
                (bright.max(BLOOM_PEAK * peak), self.k_bloom)
            } else {
                (bright, self.k_ref)
            };
            self.t_ref += (target - self.t_ref) * k;
        }
        self.steps += 1;

        if self.timeline.at(t1).index == REST && start.index != REST {
            self.next = self.draw_plan(rng);
        }
        if self.timeline.cycle(t1) != self.timeline.cycle(t0) {
            self.inoculate(self.next, rng);
        } else {
            let k = self.k_shown;
            for (s, &v) in self
                .shown
                .as_mut_slice()
                .iter_mut()
                .zip(self.colony.vis.as_slice())
            {
                *s += (v - *s) * k;
            }
        }
    }

    /// A new colony at `plan`, exposed for the soft drop itself: every bloom opens on the
    /// same drop, whatever the old network's reference was (PHY-A7).
    fn inoculate(&mut self, plan: Plan, rng: &mut impl Rng) {
        let lattice = self.colony.lattice();
        self.colony.inoculate(plan, population(lattice), rng);
        self.t_ref = self.colony.vis_bright_mean().0;
        self.show_now();
    }

    /// Reset the drawn field to `V` (a new colony, or a new lattice).
    fn show_now(&mut self) {
        if self.shown.size() == self.colony.vis.size() {
            self.shown
                .as_mut_slice()
                .copy_from_slice(self.colony.vis.as_slice());
        } else {
            self.shown = self.colony.vis.clone();
        }
    }

    /// Size the dot layers and the sampling tables for a plate of `plate` cells (in
    /// `resize`, so `rasterise` never allocates).
    fn layout(&mut self, plate: Size) {
        let lattice = self.colony.lattice();
        self.veins.resize(plate);
        self.ridge.resize(plate);
        self.atoms.resize(plate);
        let dots = self.veins.dot_size();
        let (dw, dh) = (usize::from(dots.width()), usize::from(dots.height()));
        let lh = usize::from(lattice.height()).max(1);
        self.map.rebuild(lattice, dots);
        // The same cell-centre, edge-clamped taps `SampleMap` uses (CORE-R52).
        self.row_src.clear();
        self.row_src.extend((0..dh).map(|y| {
            let s = grid::centre_of(y as f64, dh, lh).clamp(0.0, (lh - 1) as f64);
            let i0 = s as usize;
            (i0 as u16, (i0 + 1).min(lh - 1) as u16)
        }));
        // Dot columns are monotone in their left tap: `col_start[i]..col_start[i + 1]` are
        // the dots whose bilinear sample reads lattice columns `i` and `i + 1`.
        let lw = usize::from(lattice.width()).max(1);
        self.col_start.clear();
        self.col_start.resize(lw + 1, dw as u32);
        for x in (0..dw).rev() {
            let s = grid::centre_of(x as f64, dw, lw).clamp(0.0, (lw - 1) as f64);
            self.col_start[s as usize] = x as u32;
        }
        for i in (0..lw).rev() {
            self.col_start[i] = self.col_start[i].min(self.col_start[i + 1]);
        }
        self.row_peak.clear();
        self.row_peak.reserve(lh);
        self.rows.clear();
        self.rows.resize(RING * dw, 0.0);
    }

    /// Sample dot row `y` of the exposed field into ring slot `y mod 3`. Only the spans
    /// between lattice columns whose four corners reach the floor are sampled (a bilinear
    /// value never exceeds its corners); the dark plate stays 0, so a frame costs about
    /// the network's area, not the screen's.
    fn sample_row(&mut self, y: usize, scale: f32, floor: f32) {
        let dw = self.rows.len() / RING;
        let slot = &mut self.rows[(y % RING) * dw..][..dw];
        slot.fill(0.0);
        let (a, b) = self.row_src[y];
        let peak = self.row_peak[usize::from(a)].max(self.row_peak[usize::from(b)]);
        if peak < floor {
            return;
        }
        let lw = usize::from(self.shown.width());
        let src = self.shown.as_slice();
        let (r0, r1) = (
            &src[usize::from(a) * lw..][..lw],
            &src[usize::from(b) * lw..][..lw],
        );
        let raw_floor = floor / scale;
        let row = self.map.row(&self.shown, y as i32);
        for i in 0..lw {
            let (x0, x1) = (self.col_start[i] as usize, self.col_start[i + 1] as usize);
            if x0 == x1 {
                continue;
            }
            let j = (i + 1).min(lw - 1);
            if r0[i].max(r0[j]).max(r1[i]).max(r1[j]) < raw_floor {
                continue;
            }
            for (x, v) in slot[x0..x1].iter_mut().enumerate() {
                *v = row.at((x0 + x) as i32) * scale;
            }
        }
    }

    /// The drawn field as a vein stipple with ridge threads, and the oat flakes as atoms
    /// (Look (M18)).
    fn rasterise(&mut self) {
        self.veins.clear();
        self.ridge.clear();
        self.atoms.clear();
        let scale = 1.0 / (2.5 * self.t_ref.max(1e-6));
        // `u = √v` is below the floor exactly when `v` is below its square, so the dark
        // plate skips the square root.
        let floor = (U_FLOOR * U_FLOOR) as f32;
        let lw = usize::from(self.shown.width()).max(1);
        self.row_peak.clear();
        self.row_peak.extend(
            self.shown
                .as_slice()
                .chunks_exact(lw)
                .map(|r| r.iter().fold(0.0f32, |m, &v| m.max(v)) * scale),
        );
        let dw = self.rows.len() / RING;
        let dh = self.row_src.len();
        if dw > 0 && dh > 0 && self.row_peak.len() == usize::from(self.shown.height()) {
            for y in 0..dh.min(REACH) {
                self.sample_row(y, scale, floor);
            }
            for y in 0..dh {
                if y + REACH < dh {
                    self.sample_row(y + REACH, scale, floor);
                }
                // Off the plate counts as dark, which no ridge dot can be.
                let rows = &self.rows;
                let at = |k: isize| {
                    let r = y as isize + k;
                    (0..dh as isize)
                        .contains(&r)
                        .then(|| &rows[(r as usize % RING) * dw..][..dw])
                };
                let cur = &rows[(y % RING) * dw..][..dw];
                let column = [at(-2), at(-1), at(1), at(2)];
                for (x, &v) in cur.iter().enumerate() {
                    if v < floor {
                        continue;
                    }
                    let (xi, yi) = (x as i32, y as i32);
                    if v >= RIDGE_V {
                        let row = |k: isize| {
                            let i = x as isize + k;
                            if (0..dw as isize).contains(&i) {
                                cur[i as usize]
                            } else {
                                0.0
                            }
                        };
                        let col = |r: Option<&[f32]>| r.map_or(0.0, |s| s[x]);
                        let across_x = [row(-2), row(-1), v, row(1), row(2)];
                        let across_y = [
                            col(column[0]),
                            col(column[1]),
                            v,
                            col(column[2]),
                            col(column[3]),
                        ];
                        if is_ridge(across_x, across_y) {
                            self.ridge.set(xi, yi, RIDGE_INK);
                            continue;
                        }
                    }
                    // The dot's fixed threshold first (CORE-R107), then the density from
                    // the table: no square root per dot.
                    let t = hash01(VEIN_SALT ^ cell_key(xi, yi)) as f32;
                    if t < self.density.at(v) {
                        let ink = if v < SUN_V { VEIN_INK } else { CORE_INK };
                        self.veins.set(xi, yi, ink);
                    }
                }
            }
        }
        // The colony's flakes show while it lives and fade out over the last second of
        // the fade; the next colony's fade in over rest, on an empty plate.
        let beat = self.timeline.at(self.time());
        let (plan, level) = match beat.index {
            REST => (self.next, ease::smoothstep(beat.progress)),
            FADE => (
                self.colony.plan,
                1.0 - ease::smoothstep((beat.local - (FADE_SECS - FLAKE_OUT)) / FLAKE_OUT),
            ),
            _ => (self.colony.plan, 1.0),
        };
        let level = ink::level(f64::from(FLAKE_LEVEL) * level);
        if level == 0 {
            self.n_flakes = 0;
            return;
        }
        let ink = Ink::new(Role::Cold, level);
        let (lattice, dots) = (self.colony.lattice(), self.atoms.dot_size());
        self.flake_ink = ink;
        self.n_flakes = plan.food().len();
        for (k, (slot, &f)) in self.flakes.iter_mut().zip(plan.food()).enumerate() {
            // Snapped to a dot, so the atom is symmetric about its centre dot.
            let (cx, cy) = flake_dot(f, lattice, dots);
            let (cx, cy) = (cx.round(), cy.round());
            let salt = FLAKE_SALT ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            self.atoms
                .stipple_puff(cx, cy, FLAKE_SIGMA, FLAKE_PEAK, salt, ink);
            *slot = (cx, cy);
        }
    }

    fn relabel(&mut self) {
        self.label.clear();
        let _ = write!(self.label, "SA 22.5 RA 45 SO {}", self.colony.so);
    }
}

/// Kept share of the vein stipple at tone `u` (Look (M18), PHY-A9): 0 up to the floor,
/// then `0.85·((u − 0.45)/0.55)²`, so a vein fills in toward its core.
pub(crate) fn vein_density(u: f64) -> f64 {
    if u <= U_FLOOR {
        0.0
    } else {
        VEIN_PEAK * ((u - U_FLOOR) / (1.0 - U_FLOOR)).powi(2)
    }
}

/// [`vein_density`] tabulated over the exposed field `v = u²` (the square root and the
/// square leave the per-dot loop): `N` steps from the floor `v = 0.45²` to `v = 1`, read
/// at the step below `v` (so never above the exact density), `VEIN_PEAK` from `v ≥ 1`.
#[derive(Debug, Clone)]
struct DensityTable {
    p: [f32; DensityTable::N + 1],
}

impl DensityTable {
    const N: usize = 512;
    const V0: f64 = U_FLOOR * U_FLOOR;

    fn new() -> Self {
        let mut p = [0.0f32; Self::N + 1];
        for (k, d) in p.iter_mut().enumerate() {
            let v = Self::V0 + (1.0 - Self::V0) * k as f64 / Self::N as f64;
            *d = vein_density(v.sqrt()) as f32;
        }
        Self { p }
    }

    /// Density at `v` (0 below the floor).
    #[inline]
    fn at(&self, v: f32) -> f32 {
        let k = (f64::from(v) - Self::V0) * Self::N as f64 / (1.0 - Self::V0);
        if k < 0.0 {
            0.0
        } else {
            self.p[(k as usize).min(Self::N)]
        }
    }
}

/// A ridge dot (PHY-A10), from the sampled field at offsets −2..=2 dots along x and along
/// y (index 2 is the dot itself): a local maximum across the tube, along x
/// (`x[1] < v ≥ x[3]`) or along y. The axis must be the one across the tube: its fall
/// over ±2 dots (`2v − x[0] − x[4]`) is the larger of the two and at least
/// [`RIDGE_FALL`] of `v`. A tube's core falls by ≈ 15–40 % over two dots; a junction
/// blob's flat top by a few per cent, and there a "maximum" is deposit noise, not a tube,
/// so blobs stay stipple and threads run only along tubes.
pub(crate) fn is_ridge(x: [f32; 5], y: [f32; 5]) -> bool {
    let v = x[2];
    let max_x = x[1] < v && v >= x[3];
    let max_y = y[1] < v && v >= y[3];
    let (fx, fy) = (2.0 * v - x[0] - x[4], 2.0 * v - y[0] - y[4]);
    let sharp = RIDGE_FALL * v;
    (max_x && fx >= fy && fx >= sharp) || (max_y && fy >= fx && fy >= sharp)
}

/// A flake's centre in dot coordinates (the centre of its lattice site).
fn flake_dot((fx, fy): (i32, i32), lattice: Size, dots: Size) -> (f64, f64) {
    let (x, y) = remap((fx as f32, fy as f32), lattice, dots);
    (f64::from(x), f64::from(y))
}

/// The terminal cell holding a dot-space point.
fn cell_of((x, y): (f64, f64)) -> (i32, i32) {
    (
        (x.round() as i32).div_euclid(Dots::CELL_W),
        (y.round() as i32).div_euclid(Dots::CELL_H),
    )
}

/// The label is drawn only on terminals of at least 40×12 cells, in a row of its own.
fn shows_label(size: Size) -> bool {
    size.width() >= 40 && size.height() >= 12
}

impl Generator for Physarum {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        // The plate stops above the label row, so its absorbing edge keeps every tube off
        // the row and the label sits under the organism, not inside it.
        let plate = if shows_label(size) {
            Size::new(size.width(), size.height() - 1)
        } else {
            size
        };
        let display = Size::new(plate.width(), plate.height().saturating_mul(2));
        let lattice = grid::lattice_for(display, MAX_SITES, BUCKET);
        if !self.ready {
            self.colony = Colony::new(lattice);
            let plan = self.draw_plan(&mut ctx.rng);
            self.inoculate(plan, &mut ctx.rng);
            self.ready = true;
        } else if lattice != self.colony.lattice() {
            let beat = self.timeline.at(self.time());
            let target = (beat.index < FADE).then(|| population(lattice));
            self.next.rescale(self.colony.lattice(), lattice);
            self.colony.rescale(lattice, target, &mut ctx.rng);
            self.show_now();
        }
        self.cells = size;
        self.layout(plate);
        self.relabel();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !self.ready {
            return;
        }
        let steps = self.clock.advance(dt);
        if steps == 0 {
            return;
        }
        for _ in 0..steps {
            self.step(&mut ctx.rng);
        }
        self.dirty = true;
    }

    /// GEN-R20: rasterise once per drawn frame, only after the state changed.
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.veins.blit(canvas);
        self.ridge.blit(canvas);
        self.atoms.blit(canvas);
        // In ASCII a braille atom is a small `.`/`:` smudge; one `@` marks the flake.
        if canvas.mode() == GlyphMode::Ascii {
            for &c in &self.flakes[..self.n_flakes] {
                let (x, y) = cell_of(c);
                canvas.put(x, y, Glyph::ascii('@'), self.flake_ink);
            }
        }
        // Bottom-right, clear of the title (GEN-R13), with one dark cell either side so the
        // label never looks glued to a tube.
        if shows_label(self.cells)
            && self.label.len() <= title::readout_room(self.cells, &Title::of(self))
        {
            let y = i32::from(self.cells.height()) - 1;
            draw::label_right(canvas, y, self.label.as_str(), LABEL_INK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// `(cv, fraction above mean, giant fraction, food in giant)` of a trail (PHY-A4): the
    /// giant is the largest 8-connected component of the sites above the mean.
    fn network_stats(t: &Grid<f32>, food: &[(i32, i32)]) -> (f64, f64, f64, usize) {
        let v: Vec<f64> = t.as_slice().iter().map(|&x| f64::from(x)).collect();
        let n = v.len() as f64;
        let mean = v.iter().sum::<f64>() / n;
        let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
        let (w, h) = (i32::from(t.width()), i32::from(t.height()));
        let above = |x: i32, y: i32| t.get(x, y).is_some_and(|&v| f64::from(v) > mean);
        let mut label = vec![0usize; v.len()];
        let mut sizes = vec![0usize];
        for y in 0..h {
            for x in 0..w {
                if !above(x, y) || label[(y * w + x) as usize] != 0 {
                    continue;
                }
                let id = sizes.len();
                sizes.push(0);
                let mut queue = VecDeque::from([(x, y)]);
                label[(y * w + x) as usize] = id;
                while let Some((cx, cy)) = queue.pop_front() {
                    sizes[id] += 1;
                    for (nx, ny) in t.neighbors(cx, cy, Hood::Moore, Edges::Clip) {
                        let i = (ny * w + nx) as usize;
                        if above(nx, ny) && label[i] == 0 {
                            label[i] = id;
                            queue.push_back((nx, ny));
                        }
                    }
                }
            }
        }
        let total: usize = sizes.iter().sum();
        let giant = (1..sizes.len()).max_by_key(|&i| sizes[i]).unwrap_or(0);
        let in_giant = food
            .iter()
            .filter(|&&(x, y)| giant != 0 && label[(y * w + x) as usize] == giant)
            .count();
        (
            sd / mean,
            total as f64 / n,
            sizes[giant] as f64 / total.max(1) as f64,
            in_giant,
        )
    }

    fn colony(lattice: Size, n_food: usize, seed: u64) -> (Colony, SplitMix64) {
        let mut rng = SplitMix64::new(seed);
        let mut c = Colony::new(lattice);
        let n = population(lattice);
        let plan = Plan::draw(lattice, n, n_food, c.so, &mut rng);
        c.inoculate(plan, n, &mut rng);
        (c, rng)
    }

    fn check_exclusion(c: &Colony) {
        let occupied = c.occ.as_slice().iter().filter(|&&o| o).count();
        assert_eq!(occupied, c.agents.len(), "occupancy count vs live agents");
        let mut seen = std::collections::HashSet::new();
        for a in &c.agents {
            let s = site(a.x, a.y);
            assert!(
                c.occ.get(s.0, s.1) == Some(&true),
                "agent off the lattice or unmarked at {s:?}"
            );
            assert!(seen.insert(s), "two agents share {s:?}");
        }
    }

    fn scene(size: Size, seed: u64) -> (Physarum, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut p = Physarum::new();
        p.resize(size, &mut ctx);
        (p, ctx)
    }

    fn run_to(p: &mut Physarum, ctx: &mut Ctx, t: f64) {
        while p.time() < t - 1e-9 {
            p.update(STEP, ctx);
            p.prepare();
        }
    }

    #[test]
    fn phy_a1_steer_follows_jones_rule() {
        // Jones's five cases, spelled out (+ = toward the right sensor FR).
        assert_eq!(
            steer(2.0, 1.0, 1.0, false),
            0,
            "F strictly greatest: keep going"
        );
        assert_eq!(steer(0.0, 1.0, 1.0, false), -2, "F strictly least: coin");
        assert_eq!(steer(0.0, 1.0, 1.0, true), 2, "F strictly least: coin");
        assert_eq!(steer(1.0, 0.0, 2.0, false), 2, "FL < FR: toward FR");
        assert_eq!(steer(1.0, 2.0, 0.0, true), -2, "FR < FL: toward FL");
        assert_eq!(steer(1.0, 1.0, 1.0, true), 0, "all equal: keep going");
        let vals = [0.0f32, 1.0, 2.0];
        for &f in &vals {
            for &fl in &vals {
                for &fr in &vals {
                    for coin in [false, true] {
                        let want = if f > fl.max(fr) {
                            0
                        } else if f < fl.min(fr) {
                            if coin { 2 } else { -2 }
                        } else {
                            match fl.total_cmp(&fr) {
                                std::cmp::Ordering::Less => 2,
                                std::cmp::Ordering::Greater => -2,
                                std::cmp::Ordering::Equal => 0,
                            }
                        };
                        assert_eq!(
                            steer(f, fl, fr, coin),
                            want,
                            "f {f} fl {fl} fr {fr} coin {coin}"
                        );
                    }
                }
            }
        }
        for (c, s) in DIRS {
            assert!((c.hypot(s) - 1.0).abs() < 1e-6);
        }
        for (k, (c, s)) in DIRS.iter().enumerate() {
            let a = k as f64 * std::f64::consts::TAU / 16.0;
            assert!(
                (f64::from(*c) - a.cos_det()).abs() < 1e-6
                    && (f64::from(*s) - a.sin_det()).abs() < 1e-6
            );
        }
        for start in 0..16u8 {
            let mut h = start;
            for _ in 0..16 {
                h = rotate(h, 1);
            }
            assert_eq!(h, start);
        }
        assert_eq!((rotate(0, -RA), rotate(15, RA)), (14, 1));
        // + is clockwise on screen (y down): heading east, h + 1 points below the x axis.
        let ((c0, s0), (c1, s1)) = (DIRS[0], DIRS[usize::from(rotate(0, 1))]);
        assert!(c0 * s1 - s0 * c1 > 0.0);

        // The right sensor is the + side: an agent heading east with attractant only at
        // its FR sensor turns +RA in the sensory pass.
        let mut c = Colony::new(Size::new(21, 21));
        c.agents.push(Agent {
            x: 10.0,
            y: 10.0,
            h: 0,
            death: 1.0,
        });
        let (dx, dy) = DIRS[usize::from(rotate(0, SA))];
        let fr = site(10.0 + c.so * dx, 10.0 + c.so * dy);
        c.trail.set(fr.0, fr.1, 1.0);
        c.order.push(0);
        c.sensory(&mut SplitMix64::new(1));
        assert_eq!(c.agents[0].h, rotate(0, RA));
    }

    #[test]
    fn phy_a2_diffuse_decay_averages_and_decays() {
        let size = Size::new(9, 7);
        let mut scratch = Grid::default();
        let mut t = Grid::new(size, 3.5f32);
        let sum = diffuse_decay(&mut t, &mut scratch, 0.1);
        for ((x, y), &v) in t.iter() {
            // The plate edge absorbs: sites off the lattice count as 0, so an edge site
            // keeps 6/9 of the constant and a corner 4/9.
            let inside = |k: i32, n: u16| {
                if k == 0 || k == i32::from(n) - 1 {
                    2.0
                } else {
                    3.0
                }
            };
            let want = 0.9 * 3.5 * inside(x, size.width()) * inside(y, size.height()) / 9.0;
            assert!(
                (v / want - 1.0).abs() < 1e-6,
                "({x}, {y}) = {v}, want {want}"
            );
        }
        let total: f32 = t.as_slice().iter().sum();
        assert!(
            (sum / total - 1.0).abs() < 1e-5,
            "returned sum {sum} vs {total}"
        );

        let mut t = Grid::new(size, 0.0f32);
        t.set(4, 3, 1.0);
        diffuse_decay(&mut t, &mut scratch, 0.0);
        for ((x, y), &v) in t.iter() {
            let want = if (x - 4).abs() <= 1 && (y - 3).abs() <= 1 {
                1.0 / 9.0
            } else {
                0.0
            };
            assert!((v - want).abs() < 1e-7, "({x}, {y}) = {v}");
        }
        assert!((t.as_slice().iter().sum::<f32>() - 1.0).abs() < 1e-6);

        let mut rng = SplitMix64::new(3);
        for _ in 0..20 {
            let mut t = Grid::new(Size::new(13, 11), 0.0f32);
            for v in t.as_mut_slice() {
                *v = rng.random_range(0.0..50.0);
            }
            let (max, min) = t
                .as_slice()
                .iter()
                .fold((0.0f32, f32::MAX), |(a, b), &v| (a.max(v), b.min(v)));
            diffuse_decay(&mut t, &mut scratch, 0.1);
            for ((x, y), &v) in t.iter() {
                // One ulp of slack for the rounding of `(a + b + c) / 3`. The min bound
                // holds where the whole 3×3 block is on the lattice.
                let interior = (1..12).contains(&x) && (1..10).contains(&y);
                assert!(v <= 0.9 * max * (1.0 + 1e-6) && v >= 0.0);
                assert!(!interior || v >= 0.9 * min * (1.0 - 1e-6));
            }
        }
    }

    #[test]
    fn phy_a3_exclusion_and_population_through_a_fade() {
        let (mut c, mut rng) = colony(Size::new(80, 48), 5, 8);
        let n = c.agents.len();
        assert_eq!(n, 192);
        check_exclusion(&c);
        for _ in 0..600 {
            c.step(&mut rng, true);
            check_exclusion(&c);
            assert_eq!(c.agents.len(), n);
        }
        let mut last = n;
        for s in 0..(FADE_SECS / STEP).round() as usize {
            c.starve(s as f64 * STEP);
            c.step(&mut rng, false);
            check_exclusion(&c);
            assert!(c.agents.len() <= last);
            last = c.agents.len();
        }
        assert_eq!(last, 0, "every agent has died by the end of the fade");
    }

    #[test]
    fn phy_a4_network_forms_and_links_the_food() {
        // ~0.5 s in a debug build.
        let lattice = Size::new(96, 48);
        let (mut c, mut rng) = colony(lattice, 5, 42);
        let food: Vec<(i32, i32)> = c.plan.food().to_vec();
        assert_eq!(food.len(), 5);
        // Flakes lie beyond the inoculum's rim by the larger of two sensor reaches and
        // 15 % of the plate, so the colony has to search before its first contact.
        let (centre, rim) = (c.plan.centre, Plan::inoculum_radius(population(lattice)));
        for &(fx, fy) in &food {
            let d = (fx as f32 - centre.0).hypot(fy as f32 - centre.1);
            assert!(
                d >= rim + (2.0 * c.so).max(0.15 * 48.0),
                "flake ({fx}, {fy}) at {d}"
            );
        }
        // At step 0 no flake's 3×3 disc is 8-adjacent to the inoculum: no site within
        // Chebyshev distance 2 of a flake centre has trail.
        for &(fx, fy) in &food {
            for (dx, dy) in (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (dx, dy))) {
                assert!(c.trail.get(fx + dx, fy + dy).is_none_or(|&t| t == 0.0));
            }
        }
        for _ in 0..600 {
            c.step(&mut rng, true);
        }
        let (cv, above, giant, _) = network_stats(&c.trail, &food);
        assert!(cv >= 1.2, "cv {cv}");
        assert!((0.2..=0.45).contains(&above), "fraction above mean {above}");
        assert!(giant >= 0.85, "giant component {giant}");
        // The links are judged on the unpinned drawn field `V` in both runs: on `T` the
        // pinning itself would hold every fed flake centre above the mean.
        let (_, _, _, fed) = network_stats(&c.vis, &food);
        assert!(fed >= 4, "{fed} of 5 flakes in the giant component");

        // Control: the same colony without food pinning grows a network too, but it does
        // not tie the flakes in, so the pinning is what links the sources.
        let (mut c, mut rng) = colony(lattice, 5, 42);
        for _ in 0..600 {
            c.step(&mut rng, false);
        }
        let (_, _, _, unfed) = network_stats(&c.vis, &food);
        assert!(unfed <= 3, "{unfed} of 5 flakes linked without pinning");
    }

    #[test]
    fn phy_a5_life_cycle_beats_fade_and_new_layouts() {
        // < 1.5 s in a debug build.
        let run = || {
            let (mut p, mut ctx) = scene(Size::new(40, 24), 42);
            assert_eq!(p.colony.lattice(), Size::new(40, 46));
            let first = p.colony.plan;
            let (mut sum, mut count) = (0.0f64, 0);
            while p.time() < 47.9 - 1e-9 {
                p.update(STEP, &mut ctx);
                p.prepare();
                let t = p.time();
                if (10.0..40.0).contains(&t) {
                    sum += p
                        .colony
                        .trail
                        .as_slice()
                        .iter()
                        .map(|&v| f64::from(v))
                        .sum::<f64>();
                    count += 1;
                }
            }
            let end: f64 = p
                .colony
                .trail
                .as_slice()
                .iter()
                .map(|&v| f64::from(v))
                .sum();
            assert!(
                end < 1e-3 * sum / f64::from(count),
                "trail left at 47.9 s: {end}"
            );
            run_to(&mut p, &mut ctx, 48.0 + STEP);
            (first, p.colony.plan)
        };
        let (first, second) = run();
        assert_ne!(first.centre, second.centre);
        assert_ne!(first.food(), second.food());
        assert_eq!(run(), (first, second), "same seed, same layouts");

        let tl = Timeline::new(&BEATS);
        for (t, beat) in [
            (0.0, 0),
            (9.99, 0),
            (10.0, 1),
            (39.99, 1),
            (40.0, 2),
            (45.99, 2),
            (46.0, 3),
            (47.99, 3),
            (48.0, 0),
        ] {
            assert_eq!(tl.at(t).index, beat, "t = {t}");
        }
    }

    #[test]
    fn phy_a5_flakes_follow_the_colony_across_loop_seams() {
        // Every frame over two full cycles (< 2 s in a debug build): the drawn flakes are
        // the growing colony's own while it feeds, the next layout only fades in on the
        // empty plate, and no frame flashes or cuts a layout at full brightness.
        let (mut p, mut ctx) = scene(Size::new(40, 24), 42);
        let drawn = |p: &Physarum| {
            let level = if p.n_flakes == 0 {
                0
            } else {
                p.flake_ink.level
            };
            let at: Vec<(i32, i32)> = p.flakes[..p.n_flakes]
                .iter()
                .map(|&(x, y)| (x.round() as i32, y.round() as i32))
                .collect();
            (at, level)
        };
        let (mut last, mut changes) = (drawn(&p), 0);
        while p.time() < 2.0 * 48.0 + 1.0 {
            p.update(STEP, &mut ctx);
            p.prepare();
            let (flakes, level) = drawn(&p);
            let t = p.time();
            let beat = p.timeline.at(t);
            if beat.index < FADE {
                let dots = p.atoms.dot_size();
                let want: Vec<(i32, i32)> = p
                    .colony
                    .plan
                    .food()
                    .iter()
                    .map(|&f| {
                        let (x, y) = flake_dot(f, p.colony.lattice(), dots);
                        let (x, y) = (x.round(), y.round());
                        (x.round() as i32, y.round() as i32)
                    })
                    .collect();
                assert_eq!(
                    (&flakes, level),
                    (&want, FLAKE_LEVEL),
                    "t = {t}: the colony's own flakes"
                );
            }
            assert!(
                level.abs_diff(last.1) <= 12,
                "t = {t}: flake level {} -> {level}",
                last.1
            );
            if flakes != last.0 {
                assert!(
                    level.max(last.1) <= 12,
                    "t = {t}: layout cut at levels {} -> {level}",
                    last.1
                );
                changes += 1;
            }
            last = (flakes, level);
        }
        assert!(changes >= 4, "{changes} layout changes in two cycles");
    }

    #[test]
    fn phy_a6_resize_keeps_or_resamples_the_colony() {
        // Same lattice: bit-identical state and future.
        let (mut a, mut ca) = scene(Size::new(220, 60), 9);
        let (mut b, mut cb) = scene(Size::new(220, 60), 9);
        run_to(&mut a, &mut ca, 3.0);
        run_to(&mut b, &mut cb, 3.0);
        let lattice = a.colony.lattice();
        assert_eq!(lattice, Size::new(197, 101));
        a.resize(Size::new(240, 64), &mut ca);
        assert_eq!(a.colony.lattice(), lattice);
        assert_eq!(a.colony.trail, b.colony.trail);
        run_to(&mut a, &mut ca, 4.0);
        run_to(&mut b, &mut cb, 4.0);
        assert_eq!(a.colony.trail, b.colony.trail);

        // Shrinking resamples.
        let (mut p, mut ctx) = scene(Size::new(200, 50), 9);
        run_to(&mut p, &mut ctx, 15.0);
        let stats = |g: &Grid<f32>| {
            let s = g.as_slice();
            (
                s.iter().fold(0.0f32, |m, &v| m.max(v)),
                s.iter().sum::<f32>() / s.len() as f32,
            )
        };
        let (max0, mean0) = stats(&p.colony.trail);
        let (food0, beat0) = (p.colony.plan.n_food, p.timeline.at(p.time()).index);
        assert_eq!(p.colony.lattice(), Size::new(197, 98));
        p.resize(Size::new(80, 24), &mut ctx);
        assert_eq!(p.colony.lattice(), Size::new(80, 46));
        let (max1, mean1) = stats(&p.colony.trail);
        assert!(max1 <= max0);
        assert!((mean1 / mean0 - 1.0).abs() < 0.2, "mean {mean0} -> {mean1}");
        check_exclusion(&p.colony);
        // The population is trimmed to the model's 5 % of the new lattice.
        let target = population(Size::new(80, 46));
        assert!(
            p.colony.agents.len() <= target,
            "{} agents > {target}",
            p.colony.agents.len()
        );
        assert!(p.colony.agents.len() * 10 >= target * 9);
        assert_eq!(
            (p.colony.plan.n_food, p.timeline.at(p.time()).index),
            (food0, beat0)
        );

        // Growing during consolidate tops the population up.
        let (mut p, mut ctx) = scene(Size::new(80, 24), 9);
        run_to(&mut p, &mut ctx, 12.0);
        p.resize(Size::new(200, 50), &mut ctx);
        check_exclusion(&p.colony);
        assert!(p.colony.agents.len() * 10 >= population(Size::new(197, 98)) * 9);
    }

    #[test]
    fn phy_a7_gen_r13_flakes_stay_cool_and_the_label_sits_right_with_a_margin() {
        let size = Size::new(80, 24);
        let (p, _) = scene(size, 42);
        let flakes: Vec<(f64, f64)> = p.flakes[..p.n_flakes].to_vec();
        assert_eq!(flakes.len(), 5);
        // At t = 0 no vein dot surrounds a flake (M18: 6 dots, the old 3 pixels): its
        // pinned attractant drives the physics but is not drawn, so the flake is a pale
        // atom on the dark plate.
        for &(fx, fy) in &flakes {
            let (cx, cy) = (fx.round() as i32, fy.round() as i32);
            for dy in -6..=6 {
                for dx in -6..=6 {
                    assert!(
                        !p.veins.is_lit(cx + dx, cy + dy),
                        "vein at ({}, {})",
                        cx + dx,
                        cy + dy
                    );
                }
            }
        }
        for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
            let mut canvas = Canvas::new(size, mode);
            p.render(&mut canvas);
            let lines = canvas.to_lines();
            if mode == GlyphMode::Ascii {
                // One `@` per flake, at the flake's own cell.
                let ats: usize = lines.iter().map(|l| l.matches('@').count()).sum();
                assert_eq!(ats, flakes.len(), "{lines:#?}");
                for &c in &flakes {
                    let (x, y) = cell_of(c);
                    assert_eq!(canvas.get(x, y).map(|c| c.ch), Some('@'));
                }
            }
            // The label is right-aligned (GEN-R13) and keeps one dark cell on either side.
            let y = i32::from(size.height()) - 1;
            let last = i32::from(size.width()) - 1;
            let lead = last - 1 - p.label.as_str().len() as i32;
            for x in [lead, last] {
                assert_eq!(
                    canvas.get(x, y).copied(),
                    Some(Cell::BLANK),
                    "{mode:?} x = {x}"
                );
            }
        }

        // The plate ends above the label row: over two cycles (1 s samples) the row holds
        // the label and nothing else, so the label sits under the organism.
        let (mut p, mut ctx) = scene(size, 42);
        let y = i32::from(size.height()) - 1;
        let label = format!("{:>80}", format!(" {} ", p.label.as_str()));
        while p.time() < 100.0 - 1e-9 {
            let next = p.time().round() + 1.0;
            run_to(&mut p, &mut ctx, next);
            for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
                let mut canvas = Canvas::new(size, mode);
                p.render(&mut canvas);
                let row = &canvas.to_lines()[y as usize];
                assert_eq!(row, &label, "{mode:?} t = {}", p.time());
                for x in 0..i32::from(size.width()) {
                    let cell = canvas.get(x, y).copied().unwrap_or(Cell::BLANK);
                    assert!(
                        cell == Cell::BLANK || cell.ink == LABEL_INK,
                        "{mode:?} t = {}: trail at x = {x}",
                        p.time()
                    );
                }
            }
        }
    }

    #[test]
    fn phy_a7_blooms_keep_tonal_detail_and_tube_edges_hold_still() {
        // ~1 s in a debug build. ASCII frames at 30 FPS on the seeded 80×24 display.
        let size = Size::new(80, 24);
        let (mut p, mut ctx) = scene(size, 42);
        let frame = |p: &Physarum| {
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            p.render(&mut canvas);
            canvas.cells().iter().map(|c| c.ch).collect::<Vec<char>>()
        };
        let share = |cells: &[char], of: &[char]| {
            let lit = cells.iter().filter(|c| ".:+#".contains(**c)).count();
            let hit = cells.iter().filter(|c| of.contains(c)).count();
            hit as f64 / lit.max(1) as f64
        };
        let lit = |cells: &[char]| cells.iter().filter(|c| ".:+#".contains(**c)).count();
        let first = frame(&p);
        // Each bloom (the first and the second cycle's) is a drop with a `+` core and a
        // `:` rim, not one saturated `#` disc (exposed for the bright half alone, `#`
        // peaks at ≈ 44 % of the lit cells). M18: the ASCII grades are now a braille cell's
        // dot count (`#` = 5+ of 8 dots), so the stipple's dense `Sun` core reads as `#`
        // in ≈ 21 % of the lit cells at its peak; the bound is 30 % (was 20 % for the
        // M10 tone grades, ≈ 12 % then).
        for start in [0.0, 48.0] {
            run_to(&mut p, &mut ctx, start);
            // Every bloom opens on the same soft drop as the first: not an underexposed
            // haze under the old network's reference.
            let open = frame(&p);
            let ratio = lit(&open) as f64 / lit(&first) as f64;
            assert!(
                (0.8..=1.2).contains(&ratio),
                "t = {start}: {} lit cells vs {} at t = 0",
                lit(&open),
                lit(&first)
            );
            let core = share(&open, &['+', '#']);
            assert!(core > 0.0, "t = {start}: no `+`/`#` core");
            while p.time() < start + 1.5 - 1e-9 {
                p.update(2.0 * STEP, &mut ctx);
                p.prepare();
                let hot = share(&frame(&p), &['#']);
                assert!(hot <= 0.3, "t = {}: `#` share {hot}", p.time());
            }
        }
        // During consolidation the network pulses, but its edges do not fizz: few lit
        // cells change grade from one frame to the next (unfiltered, about 30 % do).
        run_to(&mut p, &mut ctx, 48.0 + 29.0);
        let (mut changed, mut lit) = (0, 0);
        let mut prev = frame(&p);
        for _ in 0..60 {
            p.update(2.0 * STEP, &mut ctx);
            p.prepare();
            let next = frame(&p);
            for (a, b) in prev.iter().zip(&next) {
                let on = *a != ' ' || *b != ' ';
                lit += usize::from(on);
                changed += usize::from(on && a != b);
            }
            prev = next;
        }
        let churn = changed as f64 / lit as f64;
        assert!(churn <= 0.15, "{churn} of lit cells change per frame");
    }

    /// Half-block cells among the lit cells of a frame.
    fn half_block_share(canvas: &Canvas) -> f64 {
        let lit = canvas.cells().iter().filter(|c| c.ch != ' ').count();
        let half = canvas
            .cells()
            .iter()
            .filter(|c| matches!(c.ch, '▀' | '▄' | '█'))
            .count();
        half as f64 / lit.max(1) as f64
    }

    #[test]
    fn phy_a8_frames_are_braille_only() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let (mut p, mut ctx) = scene(size, 42);
            for t in [5.0, 20.0] {
                run_to(&mut p, &mut ctx, t);
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                p.render(&mut canvas);
                let lit = canvas.cells().iter().filter(|c| c.ch != ' ').count();
                assert!(lit > 50, "{size:?} t = {t}: {lit} lit cells");
                let share = half_block_share(&canvas);
                assert!(share <= 0.03, "{size:?} t = {t}: half blocks {share}");
            }
        }
    }

    /// Lit vein dots in the dot window `[x0, x1) × [y0, y1)`.
    fn lit_in(p: &Physarum, (x0, x1): (i32, i32), (y0, y1): (i32, i32)) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| p.veins.is_lit(x, y))
            .count()
    }

    #[test]
    fn phy_a9_the_stipple_fills_in_toward_the_core() {
        assert_eq!(vein_density(0.45), 0.0);
        assert_eq!(vein_density(0.2), 0.0);
        assert!((vein_density(1.0) - VEIN_PEAK).abs() < 1e-12);
        assert!(vein_density(0.6) < vein_density(0.8) && vein_density(0.8) < vein_density(0.95));
        // A plate in three flat bands of tone u = 0.95, 0.6 and 0.44: flat, so no ridges.
        let size = Size::new(120, 41);
        let (mut p, _) = scene(size, 42);
        let lw = usize::from(p.shown.width());
        let at = |u: f32| u * u * 2.5 * p.t_ref;
        let (hi, mid, lo) = (at(0.95), at(0.6), at(0.44));
        for (i, v) in p.shown.as_mut_slice().iter_mut().enumerate() {
            *v = match 3 * (i % lw) / lw {
                0 => hi,
                1 => mid,
                _ => lo,
            };
        }
        p.rasterise();
        let dw = i32::from(p.veins.dot_size().width());
        let dh = i32::from(p.veins.dot_size().height());
        // Windows well inside each band (away from the bilinear blend at the seams).
        let band = |k: i32| lit_in(&p, (k * dw / 3 + 8, (k + 1) * dw / 3 - 8), (0, dh));
        let (dense, sparse, none) = (band(0), band(1), band(2));
        assert!(sparse > 0, "the u = 0.6 band is a faint stipple");
        assert!(dense >= 3 * sparse, "u 0.95: {dense} dots, u 0.6: {sparse}");
        assert_eq!(none, 0, "no field dot below the floor");
    }

    #[test]
    fn phy_a10_ridge_dots_are_local_maxima_of_strong_tubes() {
        // The rule itself.
        // A sharp tube profile across x, flat along y: a ridge.
        let tube = [0.5, 0.75, 0.9, 0.8, 0.55];
        let along = [0.89, 0.9, 0.9, 0.89, 0.9];
        assert!(is_ridge(tube, along), "a max across x");
        assert!(is_ridge(along, tube), "a max across y");
        assert!(!is_ridge([0.5, 0.95, 0.9, 0.8, 0.5], along), "no local max");
        // A max along a gentle bump of a horizontal tube is not across it.
        assert!(!is_ridge(
            [0.85, 0.89, 0.9, 0.89, 0.85],
            [0.4, 0.9, 0.9, 0.95, 0.5]
        ));
        // A junction blob's flat top: a max, but no tube.
        assert!(!is_ridge(
            [0.88, 0.89, 0.9, 0.89, 0.88],
            [0.89, 0.9, 0.9, 0.9, 0.89]
        ));
        // In frames: every ridge dot is a local maximum of the independently sampled
        // field along x or y, with u ≥ RIDGE_U.
        let size = Size::new(200, 50);
        let (mut p, mut ctx) = scene(size, 42);
        let mut ridges = 0;
        for t in [8.0, 20.0, 35.0] {
            run_to(&mut p, &mut ctx, t);
            let scale = 1.0 / (2.5 * p.t_ref.max(1e-6));
            let d = p.ridge.dot_size();
            let (w, h) = (i32::from(d.width()), i32::from(d.height()));
            let v = |x: i32, y: i32| {
                if x < 0 || y < 0 || x >= w || y >= h {
                    0.0
                } else {
                    p.map.sample(&p.shown, x, y) * scale
                }
            };
            for y in 0..h {
                for x in 0..w {
                    if !p.ridge.is_lit(x, y) {
                        continue;
                    }
                    ridges += 1;
                    let c = v(x, y);
                    assert!(c >= RIDGE_V, "t = {t}: ridge dot ({x}, {y}) at v {c}");
                    assert!(
                        (v(x - 1, y) < c && c >= v(x + 1, y))
                            || (v(x, y - 1) < c && c >= v(x, y + 1)),
                        "t = {t}: ridge dot ({x}, {y}) is no local max"
                    );
                }
            }
        }
        assert!(ridges > 200, "{ridges} ridge dots");
    }

    #[test]
    fn phy_a11_each_flake_is_a_cold_atom_centred_on_it() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let (mut p, mut ctx) = scene(size, 42);
            run_to(&mut p, &mut ctx, 20.0);
            assert!(p.n_flakes >= 4);
            assert_eq!(p.flake_ink, Ink::new(Role::Cold, FLAKE_LEVEL));
            let reach = (3.0 * FLAKE_SIGMA).ceil() as i32;
            for &(fx, fy) in &p.flakes[..p.n_flakes] {
                let (cx, cy) = (fx.round() as i32, fy.round() as i32);
                let (mut n, mut sx, mut sy) = (0, 0.0, 0.0);
                for y in cy - reach..=cy + reach {
                    for x in cx - reach..=cx + reach {
                        if p.atoms.is_lit(x, y) {
                            n += 1;
                            sx += f64::from(x);
                            sy += f64::from(y);
                        }
                    }
                }
                assert!(n >= 5, "{size:?}: flake at ({fx}, {fy}) has {n} dots");
                let (mx, my) = (sx / f64::from(n), sy / f64::from(n));
                assert!(
                    (mx - fx).hypot(my - fy) <= 1.0,
                    "{size:?}: centroid ({mx:.2}, {my:.2}) vs flake ({fx:.2}, {fy:.2})"
                );
            }
        }
    }
}
