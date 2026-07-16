//! Fireflies: a mangrove at night whose fireflies flash at random, then fall into step
//! until the whole tree pulses as one (SPEC-002 `firefly_sync`).
//!
//! Mirollo–Strogatz pulse-coupled oscillators: each firefly's phase `φ` rises at `1/T`
//! (`T` = 0.6 s ± 5 %, as *Pteroptyx*); on reaching 1 it flashes and resets to 0. A flash bumps every
//! neighbour within [`RADIUS`] along the concave charging curve: its state
//! `x = f(φ) = ln(1 + (eᵇ − 1)φ)/b` rises by `ε`, and a neighbour pushed to `x ≥ 1` flashes
//! at once (absorption), which is what locks groups together. Neighbours are fixed (the
//! fireflies sit on the tree), so the graph is built once. Every [`CYCLE_S`] seconds a
//! boat's lantern crosses the water and startles the fireflies it passes, scrambling their
//! phases; the tree then synchronises again.
//!
//! ## Look (M18, GEN-R26: flash puffs + coupling bonds)
//! A black night; the tree is a 1-px `Dim` braille outline. A flash is a tiny stipple puff
//! (CORE-R108) that shrinks to a fainter afterglow puff (its dots are a subset: the same
//! fixed thresholds at a lower density), then goes dark. Each pulse that bumps a drawn
//! neighbour lights a 1-px bond thread between the two for 0.2 s, so the coupling network
//! flashes like a molecule. A phase-circle inset puts every drawn firefly at angle `2πφ`:
//! a full ring while they flash at random, one sweeping arc once they are in step.

use std::f64::consts::TAU as TWO_PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::stipple_keep;
use sciart_core::{draw, title};

pub static META: Meta = Meta {
    name: "firefly_sync",
    category: Category::Biology,
    description: "Fireflies on a mangrove flash at random, then fall into step (Mirollo-Strogatz pulse coupling)",
    title: "Fireflies · flashes fall in step",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Fireflies::default())
}

// ---- Model (SPEC-002 Model) ----

const N: usize = 600;
/// Charging-curve concavity `b`.
const B: f64 = 3.0;
/// Pulse strength `ε`.
const EPS: f64 = 0.01;
/// Coupling radius in mask units (the mask is 2 wide by 1 tall).
const RADIUS: f64 = 0.06;
/// Mean period and its spread.
const PERIOD: f64 = 0.6;
const PERIOD_SPREAD: f64 = 0.05;
/// Model sub-steps per simulated second.
const SUB_PER_S: f64 = 240.0;
/// Seconds per story cycle; the boat crosses at its start, taking [`BOAT_S`].
const CYCLE_S: f64 = 45.0;
const BOAT_S: f64 = 5.0;
/// First boat after this long (the first cycle starts unsynchronised anyway).
const FIRST_BOAT_S: f64 = 45.0;
/// How long a startled firefly ignores its neighbours.
const STARTLE_S: f64 = 7.0;

// ---- Scenery (mask units: x ∈ [0, 2], y ∈ [0, 1] down) ----

const WATER_Y: f64 = 0.74;
const BLOBS: usize = 14;

// ---- Display ----

/// A flash: its peak, then a short dimmer afterglow, in two flat steps (M11 idiom).
const FLASH_S: f32 = 0.16;
const AFTERGLOW_S: f32 = 0.3;
/// Canopy pixels (half-block pixels, a quarter of the braille dots) per displayed firefly
/// at least: every k-th firefly is drawn, so each flash stands alone as a round atom
/// (≈ 55 drawn at 120×40, ≈ 115 at 200×50, scaling with the canopy's area).
const PIXELS_PER_FLY: f64 = 28.0;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// Inks in M11's roles (ADR-0018): black night; the tree a dim 1-px outline; the flashes
/// green `Particle` puffs (the bioluminescent yellow-green of *Pteroptyx*), full at
/// their peak and a step down in the afterglow; the boat's lantern a `Sun` atom.
const OUTLINE: Ink = Ink::new(Role::Dim, 70);
/// Sync is never perfect: a few laggards glow dim between the pulses (every
/// `N / LAGGARDS`-th firefly), so the tree is never blank.
const LAGGARDS: usize = 5;
const LAGGARD_INK: Ink = Ink::new(Role::Particle, 60);
const FLASH_INK: Ink = Ink::new(Role::Particle, 255);
const AFTERGLOW_INK: Ink = Ink::new(Role::Particle, 110);
const LANTERN: Ink = Ink::new(Role::Sun, 255);
const WAKE: Ink = Ink::new(Role::Hot, 90);
/// Look (M18): a flash puff (σ in dots, peak density), its afterglow's peak, and the
/// fewest dots a fresh puff shows (each firefly's stipple salt is picked in `resize` so
/// its puff has at least this many: a flash never reads as a lone dot).
const FLASH_SIGMA: f64 = 1.0;
const FLASH_PEAK: f64 = 0.95;
const GLOW_PEAK: f64 = 0.55;
const MIN_PUFF: usize = 3;
/// Candidate salts per firefly for its roundest puff (chosen once per resize).
const SALT_TRIES: u64 = 32;
const FLASH_SALT: u64 = 0xA076_1D64_78BD_642F;
/// Coupling bonds: a thin 1-px `Dim` 90 connector from a flasher to one drawn neighbour
/// its pulse bumped (an absorbed one, which the pulse made flash, if any), for 0.2 s, at
/// most one per flash, in a fixed pool. The neighbour must be at least `BOND_MIN` dots
/// away, and the connector stops `BOND_GAP` (1.5σ) short of each atom, so it reads as a
/// bond between two round atoms, never as part of a blob.
const BOND_S: f64 = 0.2;
const BOND_POOL: usize = 256;
const BOND_MIN: f64 = 4.0;
const BOND_GAP: f64 = 1.5 * FLASH_SIGMA;
const BOND_INK: Ink = Ink::new(Role::Dim, 90);
/// The phase circle: radius 10 dots (2.5 rows), a dotted `Dim` 50 ring (a dot about every
/// third along it), each drawn firefly a bright `Particle` 200 dot at angle `2πφ`, so
/// bunched fireflies read as one bright arc; shown from 60×16 cells when it clears the
/// tree, its bottom one dot above the readout's row.
const CIRCLE_R: f64 = 10.0;
const RING_INK: Ink = Ink::new(Role::Dim, 50);
const PHASE_INK: Ink = Ink::new(Role::Particle, 200);
const RING_DOTS: usize = 21;
/// Dots of afterglow (`Particle` 110) behind each phase hand.
const PHASE_TAIL: usize = 3;

/// Mirollo–Strogatz charging curve and its inverse.
#[inline]
pub(crate) fn charge(phi: f64) -> f64 {
    (1.0 + (B.exp() - 1.0) * phi).ln() / B
}

#[inline]
pub(crate) fn discharge(x: f64) -> f64 {
    ((B * x).exp() - 1.0) / (B.exp() - 1.0)
}

/// What a flasher's pulse did to a listening neighbour (reported by
/// [`Swarm::step_with`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pulse {
    /// Pushed it past threshold: it flashes with the flasher (absorption).
    Absorbed,
    /// Advanced its phase along the charging curve.
    Bumped,
    /// It was already flashing in this sub-step: the two are locked in step.
    InStep,
}

/// The oscillators and their fixed neighbour graph.
#[derive(Debug, Clone)]
pub(crate) struct Swarm {
    pub(crate) pos: Vec<[f64; 2]>,
    pub(crate) phase: Vec<f64>,
    period: Vec<f64>,
    /// Seconds since each firefly's last flash.
    pub(crate) since: Vec<f32>,
    /// CSR neighbour lists.
    starts: Vec<u32>,
    adj: Vec<u32>,
    /// Sub-step of each firefly's last flash, so it fires once per sub-step.
    fired: Vec<u64>,
    queue: Vec<u32>,
    /// Sub-steps each startled firefly still ignores its neighbours' flashes.
    deaf: Vec<u32>,
    step: u64,
    pub(crate) eps: f64,
}

impl Swarm {
    /// `pos` in mask units; phases and periods drawn from `rng`.
    pub(crate) fn new(pos: Vec<[f64; 2]>, eps: f64, rng: &mut impl Rng) -> Self {
        Self::with_radius(pos, eps, RADIUS, rng)
    }

    /// As [`new`](Self::new) with coupling radius `radius`.
    pub(crate) fn with_radius(
        pos: Vec<[f64; 2]>,
        eps: f64,
        radius: f64,
        rng: &mut impl Rng,
    ) -> Self {
        let n = pos.len();
        let mut hash = SpatialHash::new(2.0, 1.0, radius);
        hash.rebuild(&pos);
        let mut starts = Vec::with_capacity(n + 1);
        let mut adj = Vec::new();
        for (i, p) in pos.iter().enumerate() {
            starts.push(adj.len() as u32);
            hash.for_each_within(*p, radius, |j| {
                let q = pos[j];
                if j != i && (p[0] - q[0]).hypot(p[1] - q[1]) <= radius {
                    adj.push(j as u32);
                }
            });
        }
        starts.push(adj.len() as u32);
        let phase = (0..n).map(|_| rng.random_range(0.0..1.0)).collect();
        let period = (0..n)
            .map(|_| PERIOD * (1.0 + rng.random_range(-PERIOD_SPREAD..=PERIOD_SPREAD)))
            .collect();
        Self {
            pos,
            phase,
            period,
            since: vec![10.0; n],
            starts,
            adj,
            fired: vec![u64::MAX; n],
            queue: Vec::with_capacity(n),
            deaf: vec![0; n],
            step: 0,
            eps,
        }
    }

    /// Firefly `j`'s neighbours (within the coupling radius).
    #[cfg(test)]
    pub(crate) fn neighbours(&self, j: usize) -> &[u32] {
        &self.adj[self.starts[j] as usize..self.starts[j + 1] as usize]
    }

    /// Flash firefly `i` now: reset to 0 and queue its pulse.
    fn flash(&mut self, i: usize) {
        self.phase[i] = 0.0;
        self.since[i] = 0.0;
        self.fired[i] = self.step;
        self.queue.push(i as u32);
    }

    /// One sub-step of `h` seconds: advance, flash, and relay pulses (absorption).
    #[cfg(test)]
    pub(crate) fn step(&mut self, h: f64) {
        self.step_with(h, |_, _, _| {});
    }

    /// [`step`](Self::step), reporting every pulse that reaches a listening neighbour:
    /// `on_pulse(j, i, kind)` when flasher `j`'s pulse reaches neighbour `i`. The display
    /// draws its coupling bonds from these; the model is unchanged.
    pub(crate) fn step_with(&mut self, h: f64, mut on_pulse: impl FnMut(u32, u32, Pulse)) {
        self.step += 1;
        self.queue.clear();
        for i in 0..self.phase.len() {
            self.phase[i] += h / self.period[i];
            self.since[i] += h as f32;
            self.deaf[i] = self.deaf[i].saturating_sub(1);
            if self.phase[i] >= 1.0 {
                self.flash(i);
            }
        }
        let mut k = 0;
        while k < self.queue.len() {
            let j = self.queue[k] as usize;
            k += 1;
            let (a, b) = (self.starts[j] as usize, self.starts[j + 1] as usize);
            for n in a..b {
                let i = self.adj[n] as usize;
                if self.deaf[i] > 0 {
                    continue;
                }
                if self.fired[i] == self.step {
                    // Already flashing this step: the pulse lands, changes nothing.
                    on_pulse(j as u32, i as u32, Pulse::InStep);
                    continue;
                }
                let x = charge(self.phase[i]) + self.eps;
                let kind = if x >= 1.0 {
                    self.flash(i);
                    Pulse::Absorbed
                } else {
                    self.phase[i] = discharge(x);
                    Pulse::Bumped
                };
                on_pulse(j as u32, i as u32, kind);
            }
        }
    }

    /// Kuramoto order parameter `R = |⟨e^{2πiφ}⟩|`.
    pub(crate) fn order(&self) -> f64 {
        let (mut c, mut s) = (0.0, 0.0);
        for &p in &self.phase {
            let (sn, cs) = (TWO_PI * p).sin_cos_det();
            c += cs;
            s += sn;
        }
        let n = self.phase.len().max(1) as f64;
        c.hypot(s) / n
    }

    /// Startle firefly `i`: a random phase, and deaf to its neighbours for [`STARTLE_S`],
    /// as disturbed fireflies flash on their own for a while before rejoining.
    pub(crate) fn scramble(&mut self, i: usize, rng: &mut impl Rng) {
        self.phase[i] = rng.random_range(0.0..1.0);
        self.deaf[i] = (STARTLE_S * SUB_PER_S) as u32;
    }
}

/// An ellipse of the canopy: centre and radii in mask units.
type Blob = [f64; 4];

/// The mangrove: canopy blobs, trunk feet and prop roots, drawn from `rng`.
#[derive(Debug, Clone, Default)]
struct Tree {
    blobs: [Blob; BLOBS],
    /// Prop roots: quadratic Béziers `(start, control, end)` from trunk to water.
    roots: [[[f64; 2]; 3]; 12],
}

impl Tree {
    fn new(rng: &mut impl Rng) -> Self {
        let mut t = Self::default();
        // A broad, lumpy dome: an upper row of small crowns along an arc and a lower
        // row filling the underside.
        let half = BLOBS / 2;
        for (k, b) in t.blobs.iter_mut().enumerate() {
            let (row, i, n) = if k < half {
                (0.0, k, half)
            } else {
                (1.0, k - half, BLOBS - half)
            };
            let span = 1.3 - 0.25 * row;
            let x = 1.0 - span / 2.0 + span * (i as f64 + rng.random_range(0.3..0.7)) / n as f64;
            let arc = ((x - 1.0) / 0.75).powi(2);
            *b = [
                x,
                0.24 + 0.14 * arc + 0.1 * row + rng.random_range(-0.02..0.02),
                rng.random_range(0.1..0.15),
                rng.random_range(0.07..0.1),
            ];
        }
        // Prop roots arch out and down from three trunks into the water.
        for (k, r) in t.roots.iter_mut().enumerate() {
            let trunk = [0.78, 1.0, 1.22][k % 3] + rng.random_range(-0.03..0.03);
            // Each trunk fans its roots out like the ribs of a half-open umbrella, the
            // higher roots reaching further: outer trunks splay outward only, the middle
            // one a little both ways, so roots never close into loops.
            let rank = (k / 3) as f64;
            let side = match k % 3 {
                0 => -(0.5 + 0.2 * rank),
                2 => 0.5 + 0.2 * rank,
                _ => 0.45 * if k % 2 == 0 { 1.0 } else { -1.0 },
            };
            let out = rng.random_range(0.1..0.16) * side;
            let start = [trunk, 0.6 - 0.035 * rank + rng.random_range(0.0..0.03)];
            let ctrl = [trunk + 0.75 * out, start[1] + 0.25 * (WATER_Y - start[1])];
            *r = [start, ctrl, [trunk + out, WATER_Y + 0.01]];
        }
        t
    }

    /// How deep `(x, y)` lies inside the canopy: `1 − r²` of the deepest ellipse
    /// (positive inside, negative outside).
    fn canopy_depth(&self, x: f64, y: f64) -> f64 {
        self.blobs.iter().fold(f64::NEG_INFINITY, |m, b| {
            let (dx, dy) = ((x - b[0]) / b[2], (y - b[1]) / b[3]);
            m.max(1.0 - (dx * dx + dy * dy))
        })
    }

    fn in_canopy(&self, x: f64, y: f64) -> bool {
        self.blobs.iter().any(|b| {
            let (dx, dy) = ((x - b[0]) / b[2], (y - b[1]) / b[3]);
            dx * dx + dy * dy <= 1.0
        })
    }

    /// The canopy's bounding box in mask units, `(x0, y0, x1, y1)`.
    fn bounds(&self) -> (f64, f64, f64, f64) {
        self.blobs.iter().fold(
            (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ),
            |(x0, y0, x1, y1), b| {
                (
                    x0.min(b[0] - b[2]),
                    y0.min(b[1] - b[3]),
                    x1.max(b[0] + b[2]),
                    y1.max(b[1] + b[3]),
                )
            },
        )
    }

    /// Place `n` fireflies in the canopy.
    fn fireflies(&self, n: usize, rng: &mut impl Rng) -> Vec<[f64; 2]> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let p = [rng.random_range(0.0..2.0), rng.random_range(0.0..WATER_Y)];
            if self.in_canopy(p[0], p[1]) {
                out.push(p);
            }
        }
        out
    }
}

/// A coupling bond: flasher `a`'s pulse bumped drawn neighbour `b` at scene time `born`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Bond {
    a: u16,
    b: u16,
    born: f64,
}

/// The fixed pool of bonds (FLY-A9): a ring of [`BOND_POOL`] that overwrites its oldest
/// entry, so a synchronised cascade never allocates or grows.
#[derive(Debug, Clone)]
struct Bonds {
    ring: [Bond; BOND_POOL],
    next: usize,
    len: usize,
}

impl Default for Bonds {
    fn default() -> Self {
        Self {
            ring: [Bond::default(); BOND_POOL],
            next: 0,
            len: 0,
        }
    }
}

impl Bonds {
    fn push(&mut self, bond: Bond) {
        self.ring[self.next] = bond;
        self.next = (self.next + 1) % BOND_POOL;
        self.len = (self.len + 1).min(BOND_POOL);
    }

    /// The bonds still lit at time `now`: younger than [`BOND_S`].
    fn live(&self, now: f64) -> impl Iterator<Item = &Bond> {
        self.ring[..self.len]
            .iter()
            .filter(move |b| now - b.born < BOND_S - 1e-9)
    }
}

/// One flasher's bond while its pulse is relayed, at most one per flash: the first
/// neighbour the pulse made flash (absorbed) if any, else the first it bumped, else the
/// first already flashing with it (locked in step, so a synchronised flash lights its
/// coupling web).
#[derive(Debug, Clone, Copy, Default)]
struct Pending {
    flasher: Option<u32>,
    absorbed: Option<u32>,
    bumped: Option<u32>,
    in_step: Option<u32>,
}

impl Pending {
    /// Offer neighbour `i` of flasher `j` (already filtered to drawn neighbours at least
    /// [`BOND_MIN`] dots away).
    fn offer(&mut self, j: u32, i: u32, kind: Pulse, bonds: &mut Bonds, now: f64) {
        if self.flasher != Some(j) {
            self.flush(bonds, now);
            self.flasher = Some(j);
        }
        let slot = match kind {
            Pulse::Absorbed => &mut self.absorbed,
            Pulse::Bumped => &mut self.bumped,
            Pulse::InStep => &mut self.in_step,
        };
        slot.get_or_insert(i);
    }

    fn flush(&mut self, bonds: &mut Bonds, now: f64) {
        let pick = self.absorbed.or(self.bumped).or(self.in_step);
        if let (Some(j), Some(i)) = (self.flasher, pick) {
            bonds.push(Bond {
                a: j as u16,
                b: i as u16,
                born: now,
            });
        }
        *self = Self::default();
    }
}

/// The visible part of a bond between atoms at `p` and `q` (dots): the segment stopped
/// [`BOND_GAP`] short of each centre, or `None` when nothing would be left.
fn bond_span(p: (f64, f64), q: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let len = dx.hypot(dy);
    if len <= 2.0 * BOND_GAP + 0.5 {
        return None;
    }
    let (ux, uy) = (dx / len * BOND_GAP, dy / len * BOND_GAP);
    Some(((p.0 + ux, p.1 + uy), (q.0 - ux, q.1 - uy)))
}

/// Where the phase circle sits (dot units) when it is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Circle {
    cx: f64,
    cy: f64,
}

/// Dot position of phase `phi` on a circle of radius `r` about `(cx, cy)` (FLY-A10):
/// angle `2πφ` clockwise from twelve o'clock, as a clock hand sweeps.
pub(crate) fn phase_dot(cx: f64, cy: f64, r: f64, phi: f64) -> (f64, f64) {
    let (s, c) = (TWO_PI * phi).sin_cos_det();
    (cx + r * s, cy - r * c)
}

#[derive(Debug)]
struct Fireflies {
    swarm: Option<Swarm>,
    tree: Tree,
    time: f64,
    owed: f64,
    /// Order parameter once a second, oldest first.
    history: Trail<f32>,
    next_sample: f64,
    /// The tree's outline as braille dots, built in `resize`.
    outline: Vec<(u16, u16)>,
    /// Each firefly's dot (snapped) and its puff's stipple salt, built in `resize`.
    spots: Vec<(f64, f64)>,
    salts: Vec<u64>,
    bonds: Bonds,
    web: Dots,
    lights: Dots,
    circle: Option<Circle>,
    cells: Size,
    readout_room: usize,
    boat_x: Option<f64>,
    /// Draw every `stride`-th firefly (small screens).
    stride: usize,
    /// The dots are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Fireflies {
    fn default() -> Self {
        Self {
            dirty: true,
            swarm: None,
            tree: Tree::default(),
            time: 0.0,
            owed: 0.0,
            history: Trail::new(2),
            next_sample: 0.0,
            outline: Vec::new(),
            spots: Vec::new(),
            salts: Vec::new(),
            bonds: Bonds::default(),
            web: Dots::default(),
            lights: Dots::default(),
            circle: None,
            cells: Size::new(1, 1),
            readout_room: 0,
            boat_x: None,
            stride: 1,
        }
    }
}

/// A firefly's light `s` seconds after it flashed, in flat steps: the flash, a dimmer
/// afterglow over ~0.3 s, then dark [FLY-A6].
pub(crate) fn flash_ink(s: f32) -> Option<Ink> {
    if s < FLASH_S {
        Some(FLASH_INK)
    } else if s < AFTERGLOW_S {
        Some(AFTERGLOW_INK)
    } else {
        None
    }
}

/// Where the boat is at time `t` (mask x), if crossing.
fn boat_at(t: f64) -> Option<f64> {
    if t < FIRST_BOAT_S {
        return None;
    }
    let local = (t - FIRST_BOAT_S) % CYCLE_S;
    (local < BOAT_S).then(|| -0.1 + 2.2 * local / BOAT_S)
}

/// Reach of a flash puff in whole dots, `⌊3σ⌋`.
const PUFF_R: i32 = (3.0 * FLASH_SIGMA) as i32;
const PUFF_W: usize = 2 * PUFF_R as usize + 1;

/// A flash puff's keep density per dot offset from its (whole-dot) centre, `0` beyond
/// 3σ: `FLASH_PEAK·e^(−r²/2σ²)` computed exactly as `Dots::stipple_puff` does, so a shape
/// scored here is the shape drawn.
fn puff_table() -> [[f64; PUFF_W]; PUFF_W] {
    let reach = 3.0 * FLASH_SIGMA;
    let k = -0.5 / (FLASH_SIGMA * FLASH_SIGMA);
    let mut t = [[0.0; PUFF_W]; PUFF_W];
    for (iy, row) in t.iter_mut().enumerate() {
        for (ix, p) in row.iter_mut().enumerate() {
            let (dx, dy) = (ix as f64 - f64::from(PUFF_R), iy as f64 - f64::from(PUFF_R));
            let r2 = dx * dx + dy * dy;
            if r2 <= reach * reach {
                *p = FLASH_PEAK * (k * r2).exp();
            }
        }
    }
    t
}

/// The shape of a flash puff (CORE-R108) about the whole dot `(x0, y0)` on a `dots`-sized
/// lattice: `(dots, core, stray)`, its lit dots in all, those within one dot of the
/// centre (at most 5), and those beyond 1.5 dots.
fn puff_shape(
    (x0, y0): (i32, i32),
    salt: u64,
    dots: Size,
    table: &[[f64; PUFF_W]; PUFF_W],
) -> (usize, usize, usize) {
    let (w, h) = (i32::from(dots.width()), i32::from(dots.height()));
    let (mut n, mut core, mut stray) = (0, 0, 0);
    for (iy, row) in table.iter().enumerate() {
        for (ix, &p) in row.iter().enumerate() {
            let (dx, dy) = (ix as i32 - PUFF_R, iy as i32 - PUFF_R);
            let (x, y) = (x0 + dx, y0 + dy);
            if !(0..w).contains(&x) || !(0..h).contains(&y) {
                continue;
            }
            if (dx == 0 && dy == 0) || stipple_keep(salt, x, y, p) {
                let r2 = dx * dx + dy * dy;
                n += 1;
                core += usize::from(r2 <= 1);
                stray += usize::from(r2 > 2);
            }
        }
    }
    (n, core, stray)
}

/// The salt (of [`SALT_TRIES`] seeded candidates for firefly `i`) whose flash puff at
/// the whole dot `centre` is the roundest atom: at least [`MIN_PUFF`] dots, then the
/// fullest core and the fewest stray dots (score `2·core − stray`, first best wins). The
/// puff is still a CORE-R108 stipple with fixed thresholds; only which fixed thresholds
/// is chosen, so each flash reads as a small round atom rather than a ragged cluster.
fn roundest_salt(i: usize, centre: (i32, i32), dots: Size, table: &[[f64; PUFF_W]; PUFF_W]) -> u64 {
    let mut best = (i64::MIN, FLASH_SALT);
    for k in 0..SALT_TRIES {
        let salt = SplitMix64::mix(FLASH_SALT ^ ((i as u64) << 8) ^ k) ^ FLASH_SALT;
        let (n, core, stray) = puff_shape(centre, salt, dots, table);
        if n < MIN_PUFF {
            continue;
        }
        let score = 2 * core as i64 - stray as i64;
        if score > best.0 {
            best = (score, salt);
        }
        // A full core and no strays cannot be beaten.
        if core == 5 && stray == 0 {
            break;
        }
    }
    best.1
}

impl Fireflies {
    /// The scenery on black: only the tree's silhouette, as a 1-px braille outline of its
    /// leafy crown, its trunks and its prop roots, down to the waterline. No sky, moon,
    /// stars or water. Also lays out the fireflies' dots and the phase circle.
    fn layout(&mut self) {
        let ds = self.web.dot_size();
        let (dw, dh) = (f64::from(ds.width()), f64::from(ds.height()));
        let (w, h) = (i32::from(ds.width()), i32::from(ds.height()));
        let tree = &self.tree;
        let noise = sciart_core::rng::value_noise;
        // Leaf clumps a few half-block pixels across at any size fray the crown's edge;
        // the noise runs in pixel units (two dots), as the M15 outline did.
        let leaf = (dw / 2.0 / 60.0).clamp(1.5, 4.0);
        let crown_at = |x: i32, y: i32| {
            let (mx, my) = ((f64::from(x) + 0.5) / dw * 2.0, (f64::from(y) + 0.5) / dh);
            if my >= WATER_Y {
                return false;
            }
            // The noise moves the edge by at most ±0.15 in depth: decide without it
            // away from the edge.
            let depth = tree.canopy_depth(mx, my);
            if depth > 0.15 {
                true
            } else if depth < -0.15 {
                false
            } else {
                let (fx, fy) = ((f64::from(x) + 0.5) / 2.0, (f64::from(y) + 0.5) / 2.0);
                depth > 0.3 * (noise(5, fx, fy, leaf) - 0.5)
            }
        };
        // The crown mask over its bounding box only (one evaluation per dot), then its
        // outline: crown dots with a 4-neighbour outside.
        let (bx0, by0, bx1, by1) = tree.bounds();
        let to_dot = |mx: f64, my: f64| ((mx / 2.0 * dw) as i32, (my * dh) as i32);
        let (x0, y0) = to_dot(bx0 - 0.03, by0 - 0.03);
        let (x1, y1) = to_dot(bx1 + 0.03, (by1 + 0.03).min(WATER_Y));
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(w - 1), y1.min(h - 1));
        let (mw, mh) = ((x1 - x0 + 1).max(0) as usize, (y1 - y0 + 1).max(0) as usize);
        let mut mask = vec![false; mw * mh];
        for (k, m) in mask.iter_mut().enumerate() {
            *m = crown_at(x0 + (k % mw) as i32, y0 + (k / mw) as i32);
        }
        let crown = |x: i32, y: i32| {
            (x0..=x1).contains(&x)
                && (y0..=y1).contains(&y)
                && mask[(y - y0) as usize * mw + (x - x0) as usize]
        };
        self.outline.clear();
        let mut canopy = 0usize;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if crown(x, y) {
                    canopy += 1;
                    let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(dx, dy)| !crown(x + dx, y + dy));
                    if edge {
                        self.outline.push((x as u16, y as u16));
                    }
                }
            }
        }
        // Trunks and prop roots: 1-px threads from the crown into the water.
        let on = |x: i32, y: i32| (0..w).contains(&x) && (0..h).contains(&y) && !crown(x, y);
        for (n, r) in self.tree.roots.iter().enumerate() {
            let steps = (2.0 * (dw + dh)) as i32;
            for k in 0..=steps {
                let t = f64::from(k) / f64::from(steps);
                let q = |i: usize| {
                    (1.0 - t) * (1.0 - t) * r[0][i]
                        + 2.0 * t * (1.0 - t) * r[1][i]
                        + t * t * r[2][i]
                };
                let (px, py) = to_dot(q(0), q(1));
                if on(px, py) && self.outline.last() != Some(&(px as u16, py as u16)) {
                    self.outline.push((px as u16, py as u16));
                }
            }
            if n < 3 {
                let xc = (r[0][0] / 2.0 * dw) as i32;
                let (top, foot) = ((0.36 * dh) as i32, (r[0][1] * dh) as i32);
                for y in top..=foot {
                    if on(xc, y) {
                        self.outline.push((xc as u16, y as u16));
                    }
                }
            }
        }
        // Draw every k-th firefly where the canopy is small, so flashes stay puffs. The
        // canopy is counted in half-block pixels (4 dots each), as in M15.
        let pixels = canopy / 4;
        self.stride = ((N as f64 * PIXELS_PER_FLY / pixels.max(1) as f64).ceil() as usize).max(1);

        // Each firefly's dot, and the salt of its roundest flash puff.
        self.spots.clear();
        self.salts.clear();
        let table = puff_table();
        if let Some(sw) = &self.swarm {
            for (i, p) in sw.pos.iter().enumerate() {
                let (sx, sy) = ((p[0] / 2.0 * dw).floor(), (p[1] * dh).floor());
                self.spots.push((sx, sy));
                // Only drawn fireflies ever show a puff.
                self.salts.push(if i % self.stride == 0 {
                    roundest_salt(i, (sx as i32, sy as i32), ds, &table)
                } else {
                    FLASH_SALT
                });
            }
        }

        // The phase circle, right-aligned above the readout (its right edge at the
        // readout's last column), shown on 60×16 cells or more when it clears the tree.
        self.circle = None;
        let cells = self.cells;
        if cells.width() >= 60 && cells.height() >= 16 {
            let cx = f64::from(2 * i32::from(cells.width()) - 3) - CIRCLE_R;
            // Its bottom dot is the last of the row above the readout.
            let cy = f64::from(4 * i32::from(cells.height()) - 5) - CIRCLE_R;
            let m = CIRCLE_R + 2.0;
            let clear = self
                .outline
                .iter()
                .all(|&(x, y)| (f64::from(x) - cx).abs() > m || (f64::from(y) - cy).abs() > m);
            if clear {
                self.circle = Some(Circle { cx, cy });
            }
        }
    }

    /// Two layers: the dim `web` (outline, bonds, ring, wake) and the bright `lights`
    /// (puffs, laggards, phase dots, lantern), blitted over it. A cell holding a light
    /// shows the light alone, so a bond stays a faint thread between bright puffs instead
    /// of taking the flash's colour wherever it touches one.
    fn rasterise(&mut self) {
        self.web.clear();
        self.lights.clear();
        for &(x, y) in &self.outline {
            self.web.set(i32::from(x), i32::from(y), OUTLINE);
        }
        let Some(sw) = &self.swarm else {
            return;
        };
        if self.spots.len() != sw.pos.len() {
            return;
        }
        for b in self.bonds.live(self.time) {
            let (p, q) = (self.spots[usize::from(b.a)], self.spots[usize::from(b.b)]);
            if let Some((a, b)) = bond_span(p, q) {
                self.web.line(a.0, a.1, b.0, b.1, BOND_INK);
            }
        }
        // Each drawn firefly is a puff: between flashes the crown is dark, so a
        // synchronised flash lights the whole tree at once.
        for i in (0..sw.pos.len()).step_by(self.stride) {
            let Some(ink) = flash_ink(sw.since[i]) else {
                continue;
            };
            let peak = if ink == FLASH_INK {
                FLASH_PEAK
            } else {
                GLOW_PEAK
            };
            let (x, y) = self.spots[i];
            self.lights
                .stipple_puff(x, y, FLASH_SIGMA, peak, self.salts[i], ink);
        }
        // The laggards: dim single dots wherever no flash outshines them.
        let every = (sw.pos.len() / LAGGARDS).max(1);
        for &(x, y) in self.spots.iter().step_by(every).take(LAGGARDS) {
            self.lights.plot(x, y, LAGGARD_INK);
        }
        // The phase circle: a dotted ring, and each drawn firefly at angle 2πφ.
        if let Some(Circle { cx, cy }) = self.circle {
            for k in 0..RING_DOTS {
                let (x, y) = phase_dot(cx, cy, CIRCLE_R, k as f64 / RING_DOTS as f64);
                self.web.plot(x, y, RING_INK);
            }
            // Each hand is a bright head with a short fading tail behind it (where its
            // phase was over the last ≈ 5 % of a period), so bunched fireflies sweep
            // round as one bright arc.
            let step = 1.0 / (TWO_PI * CIRCLE_R);
            for i in (0..sw.phase.len()).step_by(self.stride) {
                for k in (0..=PHASE_TAIL).rev() {
                    let phi = sw.phase[i] - k as f64 * step;
                    let (x, y) = phase_dot(cx, cy, CIRCLE_R, phi);
                    let ink = if k == 0 { PHASE_INK } else { AFTERGLOW_INK };
                    self.lights.plot(x, y, ink);
                }
            }
        }
        // The boat's lantern on the waterline: a small `Sun` atom with a short wake.
        if let Some(bx) = self.boat_x {
            let ds = self.web.dot_size();
            let (dw, dh) = (f64::from(ds.width()), f64::from(ds.height()));
            let (x, y) = ((bx / 2.0 * dw).floor(), (WATER_Y * dh).floor());
            self.web.line(x - 0.06 * dw, y, x - 2.0, y, WAKE);
            self.lights
                .stipple_puff(x, y - 1.0, FLASH_SIGMA, FLASH_PEAK, FLASH_SALT, LANTERN);
        }
    }

    /// Whether dot `(x, y)` is lit in either layer.
    #[cfg(test)]
    fn is_lit(&self, x: i32, y: i32) -> bool {
        self.web.is_lit(x, y) || self.lights.is_lit(x, y)
    }
}

impl Generator for Fireflies {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        if self.swarm.is_none() {
            self.tree = Tree::new(&mut ctx.rng);
            let pos = self.tree.fireflies(N, &mut ctx.rng);
            self.swarm = Some(Swarm::new(pos, EPS, &mut ctx.rng));
        }
        self.web.resize(size);
        self.lights.resize(size);
        self.layout();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        let Some(sw) = self.swarm.as_mut() else {
            return;
        };
        self.owed += dt * SUB_PER_S;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        let h = 1.0 / SUB_PER_S;
        let (stride, bonds, spots) = (self.stride, &mut self.bonds, &self.spots);
        // A bond needs both ends drawn and far enough apart to show as a line.
        let bondable = |j: u32, i: u32| {
            let (j, i) = (j as usize, i as usize);
            j % stride == 0
                && i % stride == 0
                && spots
                    .get(j)
                    .zip(spots.get(i))
                    .is_some_and(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1) >= BOND_MIN)
        };
        for _ in 0..n as u32 {
            let now = self.time + h;
            let mut pending = Pending::default();
            sw.step_with(h, |j, i, kind| {
                if bondable(j, i) {
                    pending.offer(j, i, kind, bonds, now);
                }
            });
            pending.flush(bonds, now);
            self.time = now;
            let boat = boat_at(self.time);
            if let (Some(bx), Some(prev)) = (boat, self.boat_x) {
                // Startle the fireflies the lantern passes under.
                for i in 0..sw.pos.len() {
                    let x = sw.pos[i][0];
                    if x > prev && x <= bx {
                        sw.scramble(i, &mut ctx.rng);
                    }
                }
            }
            self.boat_x = boat.map(|b| b.max(self.boat_x.unwrap_or(-1.0)));
            if boat.is_none() {
                self.boat_x = None;
            }
        }
        if self.time >= self.next_sample {
            self.next_sample = self.time.floor() + 1.0;
            self.history.push(sw.order() as f32);
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
        self.web.blit(canvas);
        self.lights.blit(canvas);
        if self.cells.height() < 4 {
            return;
        }
        let h = i32::from(self.cells.height());
        let r = self.history.newest().unwrap_or(0.0);
        let mut label = StackText::<12>::new();
        let _ = write!(label, "R {r:.2}");
        if self.readout_room >= label.len() {
            draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swarm(seed: u64, eps: f64) -> Swarm {
        let mut rng = SplitMix64::new(seed);
        let tree = Tree::new(&mut rng);
        let pos = tree.fireflies(N, &mut rng);
        Swarm::new(pos, eps, &mut rng)
    }

    fn run(sw: &mut Swarm, secs: f64) {
        for _ in 0..(secs * SUB_PER_S) as usize {
            sw.step(1.0 / SUB_PER_S);
        }
    }

    fn scene(size: Size, seed: u64) -> (Fireflies, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = Fireflies::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    fn run_to(g: &mut Fireflies, ctx: &mut Ctx, t: f64) {
        while g.time < t - 1e-9 {
            g.update(SIM_DT, ctx);
            g.prepare();
        }
    }

    #[test]
    fn fly_a1_uncoupled_stays_incoherent() {
        let mut sw = swarm(42, 0.0);
        for _ in 0..60 {
            run(&mut sw, 1.0);
            assert!(sw.order() < 0.3, "R = {}", sw.order());
        }
    }

    #[test]
    fn fly_a2_coupled_synchronises_within_a_minute() {
        for seed in [1, 2, 3] {
            let mut sw = swarm(seed, EPS);
            let mut best = 0.0f64;
            for _ in 0..60 {
                run(&mut sw, 1.0);
                best = best.max(sw.order());
            }
            assert!(
                sw.order() > 0.9,
                "seed {seed}: R = {} (best {best})",
                sw.order()
            );
        }
    }

    #[test]
    fn fly_a3_a_flash_resets_the_phase() {
        let mut sw = swarm(7, EPS);
        sw.phase[0] = 1.0 - 1e-9;
        sw.step(1.0 / SUB_PER_S);
        assert_eq!(sw.phase[0], 0.0);
        assert_eq!(sw.since[0], 0.0);
        // Its neighbours were pushed along the charging curve, never back.
        assert!(charge(0.5) > 0.5 && discharge(charge(0.3)) - 0.3 < 1e-12);
    }

    #[test]
    fn fly_a4_readout_stays_right_of_the_title() {
        for size in [Size::new(80, 24), Size::new(200, 50), Size::new(40, 12)] {
            let mut stage = Stage::new(build(), 42, size, GlyphMode::Unicode);
            for _ in 0..(20 * 60) {
                stage.step(SIM_DT);
            }
            let t = stage.title();
            let frame = stage.render();
            let row = i32::from(size.height()) - 1;
            let Some((_, span)) = title::span(size, &t) else {
                continue;
            };
            for x in span.end..(span.end + 2).min(i32::from(size.width())) {
                let c = frame.get(x, row).copied().unwrap_or(Cell::BLANK);
                assert!(
                    c.ink != LABEL_INK || c.ch == ' ',
                    "{size:?}: readout glued to the title"
                );
            }
        }
    }

    #[test]
    fn fly_a6_the_crown_is_dark_between_synchronised_flashes() {
        assert_eq!(flash_ink(0.0), Some(FLASH_INK), "a flash is bright");
        assert_eq!(
            flash_ink(0.2),
            Some(AFTERGLOW_INK),
            "the afterglow lingers briefly"
        );
        // Past half the period nothing of a flash is left on screen, so a synchronised
        // tree goes dark between its pulses.
        assert_eq!(flash_ink(0.5 * PERIOD as f32), None);
    }

    #[test]
    fn fly_a5_the_boat_scrambles_the_tree() {
        let (mut g, mut ctx) = scene(Size::new(120, 40), 42);
        run_to(&mut g, &mut ctx, FIRST_BOAT_S - 0.5);
        let before = g.swarm.as_ref().map_or(0.0, Swarm::order);
        assert!(before > 0.9, "synchronised before the boat: {before}");
        run_to(&mut g, &mut ctx, FIRST_BOAT_S + BOAT_S + 0.1);
        let after = g.swarm.as_ref().map_or(1.0, Swarm::order);
        assert!(after < 0.4, "R after the boat {after}");
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
    fn fly_a8_braille_only_and_a_fresh_flash_is_a_bright_puff() {
        for size in [Size::new(120, 40), Size::new(200, 50)] {
            let (mut g, mut ctx) = scene(size, 42);
            let mut fresh = 0;
            for t in [5.0, 5.5, 20.0, 20.5] {
                run_to(&mut g, &mut ctx, t);
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                g.render(&mut canvas);
                let share = half_block_share(&canvas);
                assert!(share <= 0.03, "{size:?} t = {t}: half blocks {share}");
            }
            // Every fresh flash over a second is a `Particle` 255 puff of ≥ 3 dots.
            let end = g.time + 1.0;
            while g.time < end {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
                let Some(sw) = &g.swarm else { continue };
                for i in (0..N).step_by(g.stride) {
                    if sw.since[i] > 1.0 / 60.0 {
                        continue;
                    }
                    fresh += 1;
                    let (cx, cy) = g.spots[i];
                    let (cx, cy) = (cx as i32, cy as i32);
                    let reach = (3.0 * FLASH_SIGMA) as i32;
                    let lit = (cy - reach..=cy + reach)
                        .flat_map(|y| (cx - reach..=cx + reach).map(move |x| (x, y)))
                        .filter(|&(x, y)| g.lights.is_lit(x, y))
                        .count();
                    assert!(lit >= MIN_PUFF, "{size:?}: flash {i} has {lit} dots");
                    let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                    g.web.blit(&mut canvas);
                    g.lights.blit(&mut canvas);
                    let cell = canvas.get(cx / 2, cy / 4).copied().unwrap_or(Cell::BLANK);
                    assert_eq!(cell.ink, FLASH_INK, "{size:?}: flash {i}");
                }
            }
            assert!(fresh > 50, "{size:?}: {fresh} fresh flashes");
        }
    }

    #[test]
    fn fly_a9_bonds_join_a_flasher_to_the_neighbours_its_pulse_bumped() {
        // The model reports exactly the pulses that land: neighbours within the coupling
        // radius; absorbed and in-step ones flash in the same step.
        let mut sw = swarm(5, EPS);
        run(&mut sw, 5.0);
        let mut events = Vec::new();
        for _ in 0..240 {
            sw.step_with(1.0 / SUB_PER_S, |j, i, a| events.push((j, i, a)));
            for &(j, i, kind) in &events {
                let (j, i) = (j as usize, i as usize);
                assert!(sw.neighbours(j).contains(&(i as u32)));
                let (p, q) = (sw.pos[j], sw.pos[i]);
                assert!((p[0] - q[0]).hypot(p[1] - q[1]) <= RADIUS);
                assert_eq!(sw.since[j], 0.0, "the flasher flashed this step");
                if kind != Pulse::Bumped {
                    assert_eq!(sw.since[i], 0.0, "{kind:?} ⇒ it flashes this step");
                }
            }
            events.clear();
        }

        // In the scene: every live bond joins two drawn fireflies within the coupling
        // radius and at least BOND_MIN dots apart, its flasher flashed when it was born,
        // at most one per flash, and none is drawn 0.2 s after its birth; the drawn
        // connector stops 1.5σ short of each atom.
        let (mut g, mut ctx) = scene(Size::new(120, 40), 42);
        let mut seen = 0;
        while g.time < 20.0 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let Some(sw) = &g.swarm else { continue };
            let mut per_flash = std::collections::HashMap::new();
            for b in g.bonds.live(g.time) {
                seen += 1;
                let (a, n) = (usize::from(b.a), usize::from(b.b));
                assert!(a % g.stride == 0 && n % g.stride == 0, "both ends drawn");
                assert!(sw.neighbours(a).contains(&(n as u32)), "within the radius");
                let (p, q) = (g.spots[a], g.spots[n]);
                assert!(
                    (p.0 - q.0).hypot(p.1 - q.1) >= BOND_MIN,
                    "a line, not a blob"
                );
                let age = g.time - b.born;
                assert!(age < BOND_S, "a bond {age} s old is drawn");
                // The flasher has not flashed since (its period is 0.6 s).
                assert!((f64::from(sw.since[a]) - age).abs() < 1e-3, "flasher {a}");
                let count = per_flash.entry((a, b.born.to_bits())).or_insert(0);
                *count += 1;
                assert!(*count <= 1, "one bond per flash");
            }
        }
        assert!(seen > 30, "{seen} bond sightings");
        // The connector's ends sit 1.5σ outside each atom, on the line between them.
        let ((ax, ay), (bx, by)) = bond_span((0.0, 0.0), (8.0, 0.0)).unwrap();
        assert!((ax - BOND_GAP).abs() < 1e-12 && (bx - (8.0 - BOND_GAP)).abs() < 1e-12);
        assert!(ay == 0.0 && by == 0.0);
        assert!(
            bond_span((0.0, 0.0), (3.0, 0.0)).is_none(),
            "too short to show"
        );
        let mut bonds = Bonds::default();
        bonds.push(Bond {
            a: 0,
            b: 1,
            born: 1.0,
        });
        assert_eq!(bonds.live(1.0 + BOND_S - 0.01).count(), 1);
        assert_eq!(bonds.live(1.0 + BOND_S).count(), 0, "gone 0.2 s later");
    }

    /// Circular spread `1 − |⟨e^{iθ}⟩|` of the lit phase-circle dots about its centre,
    /// leaving out the dotted ring's own dots.
    fn circle_spread(g: &Fireflies) -> f64 {
        let Some(Circle { cx, cy }) = g.circle else {
            return f64::NAN;
        };
        let ring: Vec<(i32, i32)> = (0..RING_DOTS)
            .map(|k| {
                let (x, y) = phase_dot(cx, cy, CIRCLE_R, k as f64 / RING_DOTS as f64);
                (x.round() as i32, y.round() as i32)
            })
            .collect();
        let (mut c, mut s, mut n) = (0.0, 0.0, 0.0);
        let r = CIRCLE_R as i32 + 1;
        for y in cy as i32 - r..=cy as i32 + r {
            for x in cx as i32 - r..=cx as i32 + r {
                let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
                if (dx.hypot(dy) - CIRCLE_R).abs() <= 1.0
                    && g.is_lit(x, y)
                    && !ring.contains(&(x, y))
                {
                    let theta = dx.atan2(-dy);
                    let (sn, cs) = theta.sin_cos_det();
                    c += cs;
                    s += sn;
                    n += 1.0;
                }
            }
        }
        1.0 - c.hypot(s) / n
    }

    #[test]
    fn fly_a10_the_phase_circle_bunches_as_r_rises() {
        // Angle 2πφ clockwise from twelve o'clock, on the radius.
        for (phi, want) in [
            (0.0, (50.0, 40.0)),
            (0.25, (60.0, 50.0)),
            (0.5, (50.0, 60.0)),
            (0.75, (40.0, 50.0)),
        ] {
            let (x, y) = phase_dot(50.0, 50.0, CIRCLE_R, phi);
            assert!((x - want.0).abs() < 1e-9 && (y - want.1).abs() < 1e-9);
        }
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = scene(size, 42);
        assert!(g.circle.is_some(), "the circle clears the tree at 120×40");
        assert!(scene(Size::new(59, 40), 42).0.circle.is_none());
        run_to(&mut g, &mut ctx, 1.0);
        // Each drawn firefly's dot lies within one dot of its angle.
        {
            let (sw, Some(Circle { cx, cy })) = (g.swarm.as_ref().unwrap(), g.circle) else {
                unreachable!()
            };
            for i in (0..N).step_by(g.stride) {
                let (x, y) = phase_dot(cx, cy, CIRCLE_R, sw.phase[i]);
                let near = (-1..=1).any(|dy| {
                    (-1..=1).any(|dx| g.is_lit(x.round() as i32 + dx, y.round() as i32 + dy))
                });
                assert!(near, "firefly {i} at phase {}", sw.phase[i]);
            }
        }
        let early = (
            circle_spread(&g),
            g.swarm.as_ref().map_or(0.0, Swarm::order),
        );
        run_to(&mut g, &mut ctx, 40.0);
        let late = (
            circle_spread(&g),
            g.swarm.as_ref().map_or(0.0, Swarm::order),
        );
        assert!(late.1 > 0.9 && early.1 < 0.5, "R {} -> {}", early.1, late.1);
        assert!(
            late.0 < early.0 - 0.2,
            "spread {} at R {} vs {} at R {}",
            early.0,
            early.1,
            late.0,
            late.1
        );
    }

    #[test]
    fn fly_a7_a11_every_lit_dot_is_part_of_the_picture() {
        // Over 8 s at 120×40: every lit dot is the outline, a flash or afterglow puff, a
        // laggard, a bond, the phase circle (or the lantern), and every lit cell's ink is
        // one of theirs or the label's.
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = scene(size, 42);
        let (mut flashes, mut peak) = (0, 0u8);
        let inks = [
            OUTLINE,
            FLASH_INK,
            AFTERGLOW_INK,
            LAGGARD_INK,
            BOND_INK,
            RING_INK,
            PHASE_INK,
            LANTERN,
            WAKE,
            LABEL_INK,
        ];
        while g.time < 8.0 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let sw = g.swarm.as_ref().unwrap();
            let mut allowed = Dots::new(size);
            for &(x, y) in &g.outline {
                allowed.set(i32::from(x), i32::from(y), OUTLINE);
            }
            for i in 0..N {
                let (x, y) = g.spots[i];
                allowed.disc(x, y, 3.0 * FLASH_SIGMA, FLASH_INK);
            }
            for b in g.bonds.live(g.time) {
                let (p, q) = (g.spots[usize::from(b.a)], g.spots[usize::from(b.b)]);
                allowed.line(p.0, p.1, q.0, q.1, BOND_INK);
            }
            if let Some(Circle { cx, cy }) = g.circle {
                // The annulus the ring and the phase dots round onto.
                allowed.disc(cx, cy, CIRCLE_R + 1.0, RING_INK);
            }
            let d = g.web.dot_size();
            for y in 0..i32::from(d.height()) {
                for x in 0..i32::from(d.width()) {
                    assert!(
                        !g.is_lit(x, y) || allowed.is_lit(x, y),
                        "t = {}: stray dot ({x}, {y})",
                        g.time
                    );
                }
            }
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            for c in canvas.cells().iter().filter(|c| c.ch != ' ') {
                assert!(inks.contains(&c.ink), "t = {}: ink {:?}", g.time, c.ink);
                if c.ink == FLASH_INK {
                    flashes += 1;
                }
                if c.ink.role == Role::Particle {
                    peak = peak.max(c.ink.level);
                }
            }
            assert!(sw.order() <= 1.0);
        }
        assert!(flashes > 0 && peak > 190, "a flash peaks bright: {peak}");
    }
}
