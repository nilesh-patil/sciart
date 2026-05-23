//! Bacteria crowds: self-propelled swimmers that jam where they collide, so crowds
//! condense into clusters that grow and merge while a gas of free swimmers streams
//! round them (SPEC-002 `motility_clustering`).
//!
//! Active Brownian particles in a periodic box, in units of the swimmer diameter `σ`:
//! each swims at `v₀` along its heading `θ`, the heading diffuses with `D_r` (the run's
//! persistence), and overlapping swimmers push apart with a soft linear repulsion.
//! Nothing attracts them: they cluster because a swimmer that meets a crowd keeps pushing
//! into it until its heading turns away, and crowds slow swimmers down, so crowds grow
//! (motility-induced phase separation, Cates & Tailleur 2015). It needs a high Péclet
//! number `Pe = 3v₀/(σD_r)` and a crowded box.
//!
//! Acts: **dilute** (area fraction 0.12, no clusters) → **clusters** (more swimmers
//! arrive, area fraction 0.6, `Pe ≈ 100`) → **dispersal** (the swimmers slow and tumble
//! more often, `Pe ≈ 8`, and the clusters evaporate) → back to dilute.

use std::f64::consts::TAU as TWO_PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{Tone, draw, ease, title};

pub static META: Meta = Meta {
    name: "motility_clustering",
    category: Category::Biophysics,
    description: "Active Brownian swimmers (bacteria) jam into clusters that coarsen: motility-induced phase separation",
    title: "Bacteria crowds · swimmers jam",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 3] = ["dilute", "clusters", "dispersal"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Motility::default())
}

// ---- Model (SPEC-002 Model) ----

/// Swimmers at most (SPEC-002: ≤ 3000 at 200×50).
pub(crate) const CAP: usize = 2400;
/// Swim speed in diameters per second, and the rotational diffusion (per second).
const V0: f64 = 6.0;
const DR: f64 = 0.18;
/// Repulsion stiffness (per second): overlap at a head-on stall is `v₀/k ≈ 0.075σ`.
const K: f64 = 80.0;
/// Sub-steps per second (keeps `k·h` well below 1).
const SUB_PER_S: f64 = 240.0;
/// Dispersal: speed and tumbling factors (`Pe` falls 12×).
const DISPERSE_V: f64 = 0.5;
const DISPERSE_DR: f64 = 6.0;
/// Area fractions of the dilute and crowded acts.
const PHI_DILUTE: f64 = 0.12;
const PHI_CROWD: f64 = 0.6;
/// Smallest swimmer on screen, in pixels (sets the box scale at small sizes).
const MIN_SIGMA_PX: f64 = 2.6;
/// A swimmer with at least this many contacts counts as clustered (SPEC-002 Inks).
const CLUSTER_CONTACTS: u8 = 5;
/// Contact distance for the neighbour count, in diameters.
const CONTACT: f64 = 1.25;
/// Two swimmers belong to one cluster when pressed this close (a clustered swimmer is
/// squeezed by the swimmers pushing in behind it; passive discs relax to just touching).
#[cfg(test)]
const LINK: f64 = 0.97;

// ---- Story ----

const DILUTE_S: f64 = 6.0;
const CROWD_S: f64 = 34.0;
const DISPERSE_S: f64 = 12.0;
const CROWD: usize = 1;
const DISPERSE: usize = 2;
/// New swimmers arrive over this long at the start of the crowded act, and the extra
/// ones fade out over it at the start of a new cycle.
const ARRIVE_S: f64 = 3.0;

/// Clustered swimmers: Starlight, brighter where denser and more tightly packed (a
/// jammed core glows, its rim fades into the gas).
const CLUSTER_ROLE: Role = Role::Ramp(Ramp::Starlight);
/// Free swimmers: one cold hue (SPEC-002 Inks, as built; heading shows in the rod).
const GAS_ROLE: Role = Role::Ramp(Ramp::Ice);

/// The glow a clustered swimmer exposes around its pixel.
const CLUSTER_HALO: [(i32, i32, f32); 25] = sciart_core::raster::halo_kernel(2);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// One swimmer, in box units (diameters).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Swimmer {
    pub(crate) x: f64,
    pub(crate) y: f64,
    /// Heading as a unit vector.
    pub(crate) ux: f64,
    pub(crate) uy: f64,
    /// Contacts in the last force pass.
    pub(crate) contacts: u8,
    /// Accumulated push, cleared each sub-step.
    fx: f64,
    fy: f64,
}

impl Swimmer {
    /// A swimmer at the origin heading at angle `theta`.
    pub(crate) fn heading(theta: f64) -> Self {
        let (uy, ux) = theta.sin_cos_det();
        Self {
            ux,
            uy,
            ..Self::default()
        }
    }
}

/// Advance a free swimmer by `h` seconds: swim, then turn by a Gaussian of variance
/// `2·D_r·h`. Periodic in a `lx × ly` box.
pub(crate) fn swim(
    s: &mut Swimmer,
    v0: f64,
    dr: f64,
    h: f64,
    (lx, ly): (f64, f64),
    rng: &mut impl Rng,
) {
    s.x = (s.x + h * (v0 * s.ux + s.fx)).rem_euclid(lx);
    s.y = (s.y + h * (v0 * s.uy + s.fy)).rem_euclid(ly);
    // Turn by a small random angle of variance 2·D_r·h: a uniform kick (exact
    // variance; many steps add up to the Gaussian of rotational diffusion) applied as a
    // rotation of the heading vector, renormalised, so no trigonometry per step.
    let d = rng.random_range(-1.0..1.0) * (6.0 * dr * h).sqrt();
    let (ux, uy) = (s.ux - d * s.uy, s.uy + d * s.ux);
    let norm = ux.hypot(uy);
    (s.ux, s.uy) = (ux / norm, uy / norm);
}

/// The swimmer box and its neighbour grid.
#[derive(Debug, Clone)]
pub(crate) struct Box2 {
    pub(crate) swimmers: Vec<Swimmer>,
    /// Swimmers taking part (a prefix of `swimmers`).
    pub(crate) active: usize,
    pub(crate) size: (f64, f64),
    hash: SpatialHash,
    points: Vec<[f64; 2]>,
}

impl Box2 {
    pub(crate) fn new(size: (f64, f64), cap: usize) -> Self {
        Self {
            swimmers: Vec::with_capacity(cap),
            active: 0,
            size,
            hash: SpatialHash::new(size.0, size.1, 1.0),
            points: Vec::with_capacity(cap),
        }
    }

    /// Soft repulsion `k(1 − r)` between overlapping swimmers (minimum image), and the
    /// contact counts.
    pub(crate) fn forces(&mut self) {
        let n = self.active;
        self.points.clear();
        self.points
            .extend(self.swimmers[..n].iter().map(|s| [s.x, s.y]));
        self.hash.rebuild(&self.points);
        let (lx, ly) = self.size;
        let reach = CONTACT;
        let image = |v: f64, len: f64| {
            if v < reach {
                len
            } else if v > len - reach {
                -len
            } else {
                f64::NAN
            }
        };
        for i in 0..n {
            let p = self.points[i];
            let (mut fx, mut fy, mut contacts) = (0.0, 0.0, 0u8);
            // The query point and its periodic images near the walls.
            for ox in [0.0, image(p[0], lx)] {
                for oy in [0.0, image(p[1], ly)] {
                    if ox.is_nan() || oy.is_nan() {
                        continue;
                    }
                    let q = [p[0] + ox, p[1] + oy];
                    let points = &self.points;
                    self.hash.for_each_within(q, reach, |j| {
                        if j == i || j >= n {
                            return;
                        }
                        let (dx, dy) = (q[0] - points[j][0], q[1] - points[j][1]);
                        let r2 = dx * dx + dy * dy;
                        if r2 >= reach * reach || r2 == 0.0 {
                            return;
                        }
                        contacts = contacts.saturating_add(1);
                        let r = r2.sqrt();
                        if r < 1.0 {
                            let push = K * (1.0 - r) / r;
                            fx += push * dx;
                            fy += push * dy;
                        }
                    });
                }
            }
            let s = &mut self.swimmers[i];
            s.fx = fx;
            s.fy = fy;
            s.contacts = contacts;
        }
    }

    /// One sub-step for every active swimmer.
    pub(crate) fn step(&mut self, v0: f64, dr: f64, h: f64, rng: &mut impl Rng) {
        self.forces();
        let size = self.size;
        for s in &mut self.swimmers[..self.active] {
            swim(s, v0, dr, h, size, rng);
        }
    }

    /// Fraction of active swimmers in the largest contact cluster (union–find over pairs
    /// closer than `CONTACT`). `parent` is scratch.
    #[cfg(test)]
    pub(crate) fn largest_cluster(&self, parent: &mut Vec<u32>) -> f64 {
        let n = self.active;
        parent.clear();
        parent.extend(0..n as u32);
        fn find(p: &mut [u32], mut i: u32) -> u32 {
            while p[i as usize] != i {
                p[i as usize] = p[p[i as usize] as usize];
                i = p[i as usize];
            }
            i
        }
        let (lx, ly) = self.size;
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&self.swimmers[i], &self.swimmers[j]);
                let mut dx = (a.x - b.x).abs();
                let mut dy = (a.y - b.y).abs();
                dx = dx.min(lx - dx);
                dy = dy.min(ly - dy);
                if dx * dx + dy * dy < LINK * LINK {
                    let (ri, rj) = (find(parent, i as u32), find(parent, j as u32));
                    parent[ri as usize] = rj;
                }
            }
        }
        let mut sizes = vec![0u32; n];
        for i in 0..n {
            let r = find(parent, i as u32);
            sizes[r as usize] += 1;
        }
        f64::from(sizes.into_iter().max().unwrap_or(0)) / n.max(1) as f64
    }
}

#[derive(Debug)]
struct Motility {
    world: Box2,
    /// Swimmers at the crowded area fraction for this box.
    full: usize,
    /// Swimmers at the dilute area fraction.
    dilute: usize,
    /// Box units per pixel.
    scale: f64,
    time: f64,
    owed: f64,
    timeline: Timeline,
    pixels: Pixels,
    glow: Glow,
    tone: Tone,
    cells: Size,
    readout_room: usize,
    clustered: usize,
}

impl Default for Motility {
    fn default() -> Self {
        Self {
            world: Box2::new((1.0, 1.0), CAP),
            full: 0,
            dilute: 0,
            scale: 1.0,
            time: 0.0,
            owed: 0.0,
            timeline: Timeline::new(&[
                (CAPTIONS[0], DILUTE_S),
                (CAPTIONS[1], CROWD_S),
                (CAPTIONS[2], DISPERSE_S),
            ]),
            pixels: Pixels::default(),
            glow: Glow::default(),
            tone: Tone::asinh(0.3, 3.0),
            cells: Size::new(1, 1),
            readout_room: 0,
            clustered: 0,
        }
    }
}

/// Box size (diameters), box units per pixel and crowded swimmer count for a pixel
/// buffer: as many swimmers as fit at `PHI_CROWD` with swimmers at least
/// `MIN_SIGMA_PX` wide, capped at `CAP` (larger screens get larger swimmers).
pub(crate) fn box_for(px: Size) -> ((f64, f64), f64, usize) {
    let (pw, ph) = (f64::from(px.width()), f64::from(px.height()));
    let disc = std::f64::consts::FRAC_PI_4;
    let n_fit = PHI_CROWD * pw * ph / (disc * MIN_SIGMA_PX * MIN_SIGMA_PX);
    let n = (n_fit.round() as usize).clamp(1, CAP);
    // Box area n·(π/4)/φ, same aspect as the pixels.
    let area = n as f64 * disc / PHI_CROWD;
    let scale = (area / (pw * ph)).sqrt();
    ((pw * scale, ph * scale), scale, n)
}

impl Motility {
    fn act(&self) -> Beat {
        self.timeline
            .at(self.time.rem_euclid(self.timeline.total()))
    }

    /// Swimmers taking part now: dilute, rising to full as the crowd arrives, falling back
    /// as a new cycle begins.
    fn target(&self) -> usize {
        let beat = self.act();
        let (d, f) = (self.dilute as f64, self.full as f64);
        let n = match beat.index {
            CROWD => ease::lerp(d, f, ease::smoothstep(beat.local / ARRIVE_S)),
            DISPERSE => f,
            _ => {
                if self.time < self.timeline.total() {
                    d
                } else {
                    ease::lerp(f, d, ease::smoothstep(beat.local / ARRIVE_S))
                }
            }
        };
        (n.round() as usize).min(self.world.swimmers.len())
    }

    fn params(&self) -> (f64, f64) {
        let beat = self.act();
        if beat.index == DISPERSE {
            let k = ease::smoothstep(beat.local / 2.0);
            (
                V0 * ease::lerp(1.0, DISPERSE_V, k),
                DR * ease::lerp(1.0, DISPERSE_DR, k),
            )
        } else {
            (V0, DR)
        }
    }

    fn rasterise(&mut self) {
        self.pixels.clear();
        self.glow.clear();
        let inv = 1.0 / self.scale;
        let r_px = 0.5 * inv;
        self.clustered = 0;
        for s in &self.world.swimmers[..self.world.active] {
            let (px, py) = (s.x * inv, s.y * inv);
            if s.contacts >= CLUSTER_CONTACTS {
                self.clustered += 1;
                // Clustered swimmers expose a soft glow (dot coordinates: two per
                // pixel), so a cluster reads as one luminous body, densest at its core.
                let w = f32::from(s.contacts.min(8)) / 6.0;
                self.glow.splat(2.0 * px, 2.0 * py, &CLUSTER_HALO, w);
            } else {
                // A free swimmer: a rod along its heading in one cold hue, its head
                // bright and its tail dim, so the gas reads as streaming cells.
                let (sn, cs) = (s.uy, s.ux);
                let len = (2.2 * r_px).max(2.0);
                for k in 0..=3 {
                    let t = f64::from(k) / 3.0 - 0.5;
                    let ink = Ink::new(GAS_ROLE, 70 + 40 * k as u8);
                    let (x, y) = (px + t * len * cs, py + t * len * sn);
                    if self
                        .pixels
                        .get(x as i32, y as i32)
                        .is_none_or(|i| i.role != CLUSTER_ROLE)
                    {
                        self.pixels.set(x as i32, y as i32, ink);
                    }
                }
            }
        }
        let ps = self.pixels.pixel_size();
        for y in 0..i32::from(ps.height()) {
            for x in 0..i32::from(ps.width()) {
                let g = self.glow.get(x, y).unwrap_or(0.0);
                let level = self.tone.level(g);
                if level >= 70 {
                    self.pixels.set(x, y, Ink::new(CLUSTER_ROLE, level));
                }
            }
        }
    }
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let lit = |i: Ink| i.role != Role::Background;
    let ink = match (lit(top), lit(bottom)) {
        (false, false) => return None,
        (true, _) if top.role == CLUSTER_ROLE => top,
        (_, true) if bottom.role == CLUSTER_ROLE => bottom,
        (true, _) => top,
        (false, true) => bottom,
    };
    Some((if ink.role == CLUSTER_ROLE { 'o' } else { '.' }, ink))
}

impl Generator for Motility {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.act().index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        self.pixels.resize(size);
        self.glow.resize(size);
        let (box_size, scale, full) = box_for(self.pixels.pixel_size());
        let old = self.world.size;
        // Keep swimmers in box units, rescaled to the new box (SPEC-002 Resize).
        let (sx, sy) = (box_size.0 / old.0, box_size.1 / old.1);
        let world = &mut self.world;
        world.swimmers.truncate(full);
        for s in &mut world.swimmers {
            s.x = (s.x * sx).rem_euclid(box_size.0);
            s.y = (s.y * sy).rem_euclid(box_size.1);
        }
        while world.swimmers.len() < full {
            world.swimmers.push(Swimmer {
                x: ctx.rng.random_range(0.0..box_size.0),
                y: ctx.rng.random_range(0.0..box_size.1),
                ..Swimmer::heading(ctx.rng.random_range(0.0..TWO_PI))
            });
        }
        world.size = box_size;
        world.hash.resize(box_size.0, box_size.1, CONTACT);
        self.scale = scale;
        self.full = full;
        self.dilute = ((full as f64 * PHI_DILUTE / PHI_CROWD).round() as usize)
            .max(1)
            .min(full);
        // Size the neighbour grid for the full crowd now, so the arrivals never grow it
        // in `update` (GEN-R11).
        self.world.active = full;
        self.world.forces();
        self.world.active = self.target();
        self.world.forces();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.owed += dt * SUB_PER_S;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        let h = 1.0 / SUB_PER_S;
        for _ in 0..n as u32 {
            self.time += h;
            self.world.active = self.target();
            let (v0, dr) = self.params();
            self.world.step(v0, dr, h, &mut ctx.rng);
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        let h = i32::from(self.cells.height());
        if h < 4 {
            return;
        }
        let n = self.world.active;
        let pct = (100 * self.clustered).checked_div(n).unwrap_or(0);
        let mut label = StackText::<32>::new();
        let _ = match self.readout_room {
            22.. => write!(label, "N {n:>4}  clustered {pct:>2}%"),
            11.. => write!(label, "N {n:>4} {pct:>2}%"),
            _ => return,
        };
        draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crowd(v0: f64, seed: u64) -> (Box2, SplitMix64) {
        let mut rng = SplitMix64::new(seed);
        let n = 900;
        let side = (n as f64 * std::f64::consts::FRAC_PI_4 / PHI_CROWD).sqrt();
        let mut b = Box2::new((side * 1.5, side / 1.5), n);
        for _ in 0..n {
            b.swimmers.push(Swimmer {
                x: rng.random_range(0.0..b.size.0),
                y: rng.random_range(0.0..b.size.1),
                ..Swimmer::heading(rng.random_range(0.0..TWO_PI))
            });
        }
        b.active = n;
        let _ = v0;
        (b, rng)
    }

    fn run(b: &mut Box2, rng: &mut SplitMix64, v0: f64, secs: f64) {
        for _ in 0..(secs * SUB_PER_S) as usize {
            b.step(v0, DR, 1.0 / SUB_PER_S, rng);
        }
    }

    #[test]
    fn mips_a1_passive_discs_do_not_cluster() {
        let (mut b, mut rng) = crowd(0.0, 1);
        run(&mut b, &mut rng, 0.0, 10.0);
        let mut scratch = Vec::new();
        // Passive discs at φ = 0.5 touch in small random groups, never one big cluster.
        let f = b.largest_cluster(&mut scratch);
        assert!(f < 0.02, "largest cluster {f:.3}");
    }

    #[test]
    fn mips_a2_crowded_swimmers_phase_separate() {
        let (mut b, mut rng) = crowd(V0, 2);
        let mut scratch = Vec::new();
        run(&mut b, &mut rng, V0, 30.0);
        let f = b.largest_cluster(&mut scratch);
        assert!(f > 0.2, "largest cluster {f:.3}");
    }

    #[test]
    fn mips_a3_lone_swimmer_is_ballistic_then_diffusive() {
        let mut rng = SplitMix64::new(3);
        let (h, big) = (1.0 / SUB_PER_S, (1e9, 1e9));
        let msd = |secs: f64, rng: &mut SplitMix64| {
            let mut sum = 0.0;
            for _ in 0..400 {
                let mut s = Swimmer {
                    x: 5e8,
                    y: 5e8,
                    ..Swimmer::heading(rng.random_range(0.0..TWO_PI))
                };
                for _ in 0..(secs / h) as usize {
                    swim(&mut s, V0, DR, h, big, rng);
                }
                sum += (s.x - 5e8).powi(2) + (s.y - 5e8).powi(2);
            }
            sum / 400.0
        };
        // Short times (t ≪ 1/D_r ≈ 5.6 s): MSD ≈ (v₀t)², slope 2 in log–log.
        let (a, b) = (msd(0.1, &mut rng), msd(0.2, &mut rng));
        let short = (b / a).log2();
        assert!((short - 2.0).abs() < 0.15, "short-time slope {short:.2}");
        // Long times (t ≫ 1/D_r): MSD ≈ 4·D_eff·t, slope 1.
        let (c, d) = (msd(60.0, &mut rng), msd(120.0, &mut rng));
        let long = (d / c).log2();
        assert!((long - 1.0).abs() < 0.3, "long-time slope {long:.2}");
    }

    #[test]
    fn mips_a4_count_never_exceeds_the_cap() {
        for size in [
            Size::new(80, 24),
            Size::new(200, 50),
            Size::new(500, 150),
            Size::new(1, 1),
        ] {
            let (_, _, n) = box_for(Size::new(size.width(), 2 * size.height()));
            assert!((1..=CAP).contains(&n));
            let mut ctx = Ctx::new(1, size);
            let mut g = Motility::default();
            g.resize(size, &mut ctx);
            for _ in 0..30 {
                g.update(0.25, &mut ctx);
                assert!(g.world.active <= g.world.swimmers.len() && g.world.swimmers.len() <= CAP);
            }
        }
    }

    #[test]
    fn mips_a5_captions_follow_the_acts() {
        let mut ctx = Ctx::new(1, Size::new(80, 24));
        let mut g = Motility::default();
        g.resize(Size::new(80, 24), &mut ctx);
        let mut seen = Vec::new();
        for _ in 0..((DILUTE_S + CROWD_S + DISPERSE_S + 2.0) * 4.0) as usize {
            g.update(0.25, &mut ctx);
            let c = g.caption().unwrap_or("");
            if seen.last() != Some(&c) {
                seen.push(c);
            }
        }
        assert_eq!(seen, ["dilute", "clusters", "dispersal", "dilute"]);
    }
}
