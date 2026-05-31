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

use std::f64::consts::TAU as TWO_PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::ASCII_RAMP;
use sciart_core::{Tone, draw, ink, title};

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

const FLASH_S: f32 = 0.08;
const AFTERGLOW_S: f32 = 0.3;
/// A firefly is a one-pixel lamp with a soft ring: centre 1, edge neighbours ~0.3.
const HALO: [(i32, i32, f32); 9] = sciart_core::raster::halo_kernel(1);
/// Weight of the slow halo term (decaying over [`AFTERGLOW_S`]) against the flash.
const AFTERGLOW: f32 = 0.12;
/// Canopy pixels per displayed firefly at least: on small screens only every k-th
/// firefly is drawn, so flashes stay separate points instead of a solid wash.
const PIXELS_PER_FLY: f64 = 3.0;
/// The water shows the crown's flashes as a shimmering, squashed reflection: its top row
/// mirrors the crown's underside at `REFLECT_FROM` (mask y), each water row a further
/// `REFLECT_SQUASH` rows up (artistic: the true mirror image would fall off screen).
const REFLECT_FROM: f64 = 0.46;
const REFLECT_SQUASH: f64 = 1.4;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
const SPARK_INK: Ink = Ink::new(Role::Ramp(Ramp::Sulfur), 170);
/// Order-parameter samples in the inset, one per second.
const SPARK_N: usize = 16;
const SPARK: [Glyph; 8] = [
    Glyph::new('▁', '_'),
    Glyph::new('▂', '_'),
    Glyph::new('▃', '.'),
    Glyph::new('▄', '-'),
    Glyph::new('▅', '-'),
    Glyph::new('▆', '='),
    Glyph::new('▇', '='),
    Glyph::new('█', '#'),
];

/// Mirollo–Strogatz charging curve and its inverse.
#[inline]
pub(crate) fn charge(phi: f64) -> f64 {
    (1.0 + (B.exp() - 1.0) * phi).ln() / B
}

#[inline]
pub(crate) fn discharge(x: f64) -> f64 {
    ((B * x).exp() - 1.0) / (B.exp() - 1.0)
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

    /// Flash firefly `i` now: reset to 0 and queue its pulse.
    fn flash(&mut self, i: usize) {
        self.phase[i] = 0.0;
        self.since[i] = 0.0;
        self.fired[i] = self.step;
        self.queue.push(i as u32);
    }

    /// One sub-step of `h` seconds: advance, flash, and relay pulses (absorption).
    pub(crate) fn step(&mut self, h: f64) {
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
                if self.fired[i] == self.step || self.deaf[i] > 0 {
                    continue;
                }
                let x = charge(self.phase[i]) + self.eps;
                if x >= 1.0 {
                    self.flash(i);
                } else {
                    self.phase[i] = discharge(x);
                }
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

#[derive(Debug)]
struct Fireflies {
    swarm: Option<Swarm>,
    tree: Tree,
    time: f64,
    owed: f64,
    /// Order parameter once a second, oldest first.
    history: Trail<f32>,
    next_sample: f64,
    /// Scenery (sky, water, silhouette) per pixel, built in `resize`.
    base: Pixels,
    glow: Glow,
    tone: Tone,
    pixels: Pixels,
    cells: Size,
    readout_room: usize,
    boat_x: Option<f64>,
    /// Draw every `stride`-th firefly (small screens).
    stride: usize,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
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
            history: Trail::new(SPARK_N),
            next_sample: 0.0,
            base: Pixels::default(),
            glow: Glow::default(),
            tone: Tone::asinh(0.8, 1.6),
            pixels: Pixels::default(),
            cells: Size::new(1, 1),
            readout_room: 0,
            boat_x: None,
            stride: 1,
        }
    }
}

/// A firefly's light `s` seconds after it flashed: a sharp flash and a soft halo that
/// fades over ~0.3 s [FLY-A6].
pub(crate) fn brightness(s: f32) -> f32 {
    (-s / FLASH_S).exp() + AFTERGLOW * (-s / AFTERGLOW_S).exp()
}

/// Tone levels below this show the scenery, not the glow.
const FLASH_FLOOR: u8 = 34;

/// Where the boat is at time `t` (mask x), if crossing.
fn boat_at(t: f64) -> Option<f64> {
    if t < FIRST_BOAT_S {
        return None;
    }
    let local = (t - FIRST_BOAT_S) % CYCLE_S;
    (local < BOAT_S).then(|| -0.1 + 2.2 * local / BOAT_S)
}

impl Fireflies {
    fn paint_base(&mut self) {
        let ps = self.base.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()));
        let tree = &self.tree;
        let water_px = WATER_Y * ph;
        let noise = sciart_core::rng::value_noise;
        // The moon, upper right, lights the scene: the crowns' rims on its side, a
        // glitter path on the water.
        let (moon_x, moon_y, moon_r) = (0.82 * pw, 0.13 * ph, (0.055 * ph).max(1.5));
        // Leaf clumps a few pixels across at any size.
        let leaf = (pw / 60.0).clamp(1.5, 4.0);
        let sky = |my: f64| 10.0 + 14.0 * (my / WATER_Y).powi(2);
        self.base.shade(|x, y| {
            let (mx, my) = ((f64::from(x) + 0.5) / pw * 2.0, (f64::from(y) + 0.5) / ph);
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if my < WATER_Y {
                // Leafy crown: the ellipses' edge frayed by noise into a ragged fringe of
                // leaves, textured with clumps and darker gaps, dark enough that a flash
                // stands out.
                let fray = 0.3 * (noise(5, fx, fy, leaf) - 0.5);
                if tree.canopy_depth(mx, my) > fray {
                    let clump = noise(3, fx, fy, 2.0 * leaf);
                    let fine = noise(4, fx, fy, 0.7 * leaf);
                    let above = tree.canopy_depth(mx, my - 2.5 / ph);
                    let moonward = ((mx - 0.6) / 0.9).clamp(0.0, 1.0);
                    let rim = if above <= fray { 14.0 * moonward } else { 0.0 };
                    let lvl = 5.0 + 16.0 * clump * clump + 7.0 * fine + rim;
                    return Ink::new(Role::Ramp(Ramp::Aurora), ink::level(lvl));
                }
                // The moon: a softly edged disc with faint maria, in a wide glow.
                let d = (fx - moon_x).hypot(fy - moon_y);
                let cover = (moon_r + 0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let mare = noise(7, fx, fy, 0.8 * moon_r);
                    let lit = 240.0 - 45.0 * mare;
                    return Ink::new(
                        Role::Ramp(Ramp::Starlight),
                        ink::level(90.0 + (lit - 90.0) * cover),
                    );
                }
                let halo = 36.0 * (-(d - moon_r) / (1.3 * moon_r)).exp();
                // Stars: sparse, faint, only where the moon's glow does not drown them.
                let h = sciart_core::rng::hash01(sciart_core::rng::cell_key(x, y));
                if my < 0.45 && h > 0.997 && halo < 3.0 {
                    return Ink::new(Role::Ramp(Ramp::Starlight), 60 + (h * 9000.0) as u8 % 60);
                }
                // Sky: darkest at the zenith, a faint glow at the horizon.
                Ink::new(Role::Ramp(Ramp::Cobalt), ink::level(sky(my) + halo))
            } else {
                let depth = (f64::from(y) - water_px) / (ph - water_px).max(1.0);
                // Long low ripples, and the moon's glitter path below it.
                let ripple = noise(9, fx / 5.0, fy * 1.5, 1.0);
                let path = (fx - moon_x).abs() < moon_r * (1.2 + 2.0 * depth);
                if path && ripple > 0.6 {
                    return Ink::new(
                        Role::Ramp(Ramp::Starlight),
                        ink::level(45.0 + 150.0 * (ripple - 0.6)),
                    );
                }
                Ink::new(
                    Role::Ramp(Ramp::Cobalt),
                    ink::level(13.0 - 3.0 * depth + 8.0 * ripple),
                )
            }
        });
        // Solid trunks, tapering upward into the crown, and prop roots arching out into
        // the water, as a black silhouette over the sky.
        let thick = (0.006 * pw).max(0.6);
        for (n, r) in self.tree.roots.iter().enumerate() {
            let steps = (2.0 * (pw + ph)) as i32;
            for k in 0..=steps {
                let t = f64::from(k) / f64::from(steps);
                let q = |i: usize| {
                    (1.0 - t) * (1.0 - t) * r[0][i]
                        + 2.0 * t * (1.0 - t) * r[1][i]
                        + t * t * r[2][i]
                };
                let (px, py) = (q(0) / 2.0 * pw, q(1) * ph);
                // Roots are thickest where they leave the trunk.
                let half = (thick * (1.0 - 0.6 * t)).max(0.5);
                for dx in (px - half).round() as i32..=(px + half - 0.5).round() as i32 {
                    self.base.set(dx, py as i32, Ink::BLANK);
                }
            }
            if n < 3 {
                let xc = r[0][0] / 2.0 * pw;
                let (top, foot) = (0.36 * ph, r[0][1] * ph);
                for y in top as i32..=foot as i32 {
                    let u = ((f64::from(y) - top) / (foot - top).max(1.0)).clamp(0.0, 1.0);
                    let half = thick * (1.0 + 0.8 * u);
                    for x in (xc - half).round() as i32..=(xc + half).round() as i32 {
                        self.base.set(x, y, Ink::BLANK);
                    }
                }
            }
        }
        // The water mirrors the silhouette: tree pixels above leave dark water below.
        let water = (WATER_Y * ph) as i32;
        for y in water..i32::from(ps.height()) {
            let my = 2 * water - y - 1;
            for x in 0..i32::from(ps.width()) {
                let tree = self
                    .base
                    .get(x, my)
                    .is_some_and(|i| i == Ink::BLANK || i.role == Role::Ramp(Ramp::Aurora));
                if tree {
                    self.base.set(x, y, Ink::BLANK);
                }
            }
        }
        // Draw every k-th firefly where the canopy is small, so flashes stay points.
        let canopy = (0..i32::from(ps.height()))
            .flat_map(|y| (0..i32::from(ps.width())).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                self.base
                    .get(x, y)
                    .is_some_and(|i| i.role == Role::Ramp(Ramp::Aurora))
            })
            .count();
        self.stride = ((N as f64 * PIXELS_PER_FLY / canopy.max(1) as f64).ceil() as usize).max(1);
    }

    fn rasterise(&mut self) {
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()));
        self.glow.clear();
        if let Some(sw) = &self.swarm {
            for (p, &s) in sw.pos.iter().zip(&sw.since).step_by(self.stride) {
                // A sharp flash and a soft halo that fades over ~0.3 s: between flashes
                // the crown is dark, so a synchronised flash lights the whole tree at once.
                let b = brightness(s);
                if b > 0.05 {
                    // Glow positions are in dots: two per pixel each way.
                    self.glow.splat(p[0] * pw, 2.0 * p[1] * ph, &HALO, b);
                }
            }
        }
        let water = (WATER_Y * ph) as i32;
        let t = self.time;
        let boat = self.boat_x.map(|bx| (bx / 2.0 * pw) as i32);
        let (base, glow, tone) = (&self.base, &self.glow, &self.tone);
        self.pixels.shade(|x, y| {
            let under = base.get(x, y).unwrap_or(Ink::BLANK);
            let light = if y < water {
                glow.get(x, y).unwrap_or(0.0)
            } else {
                // A rippled reflection of the flashes, stretched and dimmer with depth.
                let (ripple, _) = (f64::from(y) * 1.3 + t * 2.0).sin_cos_det();
                let sx = x + (ripple * 1.5).round() as i32;
                let my = water
                    - 1
                    - ((f64::from(y - water) * REFLECT_SQUASH) as i32)
                    - (f64::from(water) - REFLECT_FROM * ph) as i32;
                let fade = 0.55 - 0.3 * f64::from(y - water) / (ph - f64::from(water)).max(1.0);
                let mirror = glow
                    .get(sx, my)
                    .unwrap_or(0.0)
                    .max(0.7 * glow.get(sx, my + 1).unwrap_or(0.0));
                let mut l = fade as f32 * mirror;
                if let Some(bx) = boat {
                    // The lantern's wake: a streak trailing the boat on the surface rows.
                    let d = bx - x;
                    if (0..(pw * 0.25) as i32).contains(&d) && y - water < 2 {
                        l += 0.5 * (1.0 - f64::from(d) / (pw * 0.25)) as f32;
                    }
                }
                l
            };
            let level = tone.level(light);
            if level >= FLASH_FLOOR {
                Ink::new(Role::Ramp(Ramp::Sulfur), level)
            } else {
                under
            }
        });
        if let Some(bx) = boat {
            let lantern = Ink::new(Role::Ramp(Ramp::Starlight), 250);
            self.pixels.set(bx, water - 1, lantern);
            self.pixels
                .set(bx, water - 2, Ink::new(Role::Ramp(Ramp::Sulfur), 230));
        }
    }
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let lit = |i: Ink| i.role != Role::Background;
    let ink = match (lit(top), lit(bottom)) {
        (false, false) => return None,
        (true, false) => top,
        (false, true) => bottom,
        (true, true) => {
            if top.level >= bottom.level {
                top
            } else {
                bottom
            }
        }
    };
    match ink.role {
        Role::Ramp(Ramp::Sulfur) if ink.level > 120 => Some(('*', ink)),
        Role::Ramp(Ramp::Sulfur) => Some(('.', ink)),
        Role::Ramp(Ramp::Starlight) => Some(('.', ink)),
        _ => Some((ASCII_RAMP[0], Ink::BLANK)).filter(|_| false),
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
        self.base.resize(size);
        self.pixels.resize(size);
        self.glow.resize(size);
        self.paint_base();
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
        for _ in 0..n as u32 {
            sw.step(h);
            self.time += h;
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
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        let (w, h) = (
            i32::from(self.cells.width()),
            i32::from(self.cells.height()),
        );
        if h < 4 {
            return;
        }
        let r = self.history.newest().unwrap_or(0.0);
        let mut label = StackText::<12>::new();
        let _ = write!(label, "R {r:.2}");
        let spark = self.history.len();
        let need = label.len() + 1 + spark;
        if self.readout_room >= need {
            let x0 = draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
            // The sparkline sits just left of the number, oldest first.
            let start = x0 - spark as i32;
            for (k, &v) in self.history.iter().enumerate() {
                let i = ((f64::from(v) * 7.99) as usize).min(7);
                canvas.put(start + k as i32, h - 1, SPARK[i], SPARK_INK);
            }
        } else if self.readout_room >= label.len() {
            draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
        }
        let _ = w;
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
    fn fly_a4_inset_stays_right_of_the_title() {
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
            for x in 0..i32::from(size.width()) {
                // The inset's cells: sparkline glyphs in its ink (the water's half blocks
                // share a glyph but not the ink).
                let inset = frame
                    .get(x, row)
                    .is_some_and(|c| c.ink == SPARK_INK && SPARK.iter().any(|g| g.unicode == c.ch));
                if inset {
                    assert!(
                        x >= span.end + 4,
                        "{size:?}: inset at {x}, title ends {}",
                        span.end
                    );
                }
            }
        }
    }

    #[test]
    fn fly_a6_the_crown_is_dark_between_synchronised_flashes() {
        let tone = Fireflies::default().tone;
        assert!(tone.level(brightness(0.0)) > 190, "a flash is bright");
        assert!(
            tone.level(brightness(0.1)) >= FLASH_FLOOR,
            "the halo lingers briefly"
        );
        // Past half the period nothing of a flash is left on screen, so a synchronised
        // tree goes dark between its pulses.
        assert!(tone.level(brightness(0.5 * PERIOD as f32)) < FLASH_FLOOR);
    }

    #[test]
    fn fly_a5_the_boat_scrambles_the_tree() {
        let mut ctx = Ctx::new(42, Size::new(120, 40));
        let mut g = Fireflies::default();
        g.resize(Size::new(120, 40), &mut ctx);
        g.prepare();
        while g.time < FIRST_BOAT_S - 0.5 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let before = g.swarm.as_ref().map_or(0.0, Swarm::order);
        assert!(before > 0.9, "synchronised before the boat: {before}");
        while g.time < FIRST_BOAT_S + BOAT_S + 0.1 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let after = g.swarm.as_ref().map_or(1.0, Swarm::order);
        assert!(after < 0.4, "R after the boat {after}");
    }
}
