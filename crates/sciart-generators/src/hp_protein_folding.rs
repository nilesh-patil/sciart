//! Protein folding: a chain of oily (H) and water-loving (P) residues writhes as a loose
//! coil while hot, then, as it cools, collapses and hides its oily residues in a core
//! (SPEC-002 `hp_protein_folding`).
//!
//! The 2-D square-lattice HP model (Lau & Dill 1989): a self-avoiding chain whose energy
//! is `E = −(H–H contacts)`, counting lattice neighbours that are not chain neighbours.
//! Metropolis Monte Carlo with end, corner, crankshaft and pull moves (Lesh, Mitzenmacher
//! & Whitesides 2003) on an occupancy grid; simulated annealing on a looping `Timeline`
//! (hot coil, geometric cooling, cold fold, then heat again).
//!
//! On screen the beads ease towards their lattice sites, so the chain writhes smoothly;
//! a lazy camera frames it, zooming in as it collapses.

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{draw, ease, ink, raster, title};

pub static META: Meta = Meta {
    name: "hp_protein_folding",
    category: Category::LivingMatter,
    description: "HP lattice protein: a hot coil cools and collapses, hiding its hydrophobic residues in a core",
    title: "Protein folding · oil hides away",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 3] = ["unfolded", "collapse", "folded core"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Folding::default())
}

// ---- Model (SPEC-002 Model) ----

/// The 48-residue 2-D HP benchmark (best known fold: 23 H–H contacts).
pub(crate) const SEQ: &[u8; N] = b"PPHPPHHPPHHPPPPPHHHHHHHHHHPPPPPPHHPPHHPPHPPHHHHH";
pub(crate) const N: usize = 48;
/// Occupancy grid side; the chain is recentred before it nears an edge.
const G: i16 = 128;
const MARGIN: i16 = 6;
/// Annealing temperatures (units of the contact energy).
pub(crate) const T_HOT: f64 = 4.0;
pub(crate) const T_COLD: f64 = 0.15;
/// The collapse act starts here: just above the coil-globule transition, so its cooling
/// time is spent where the fold is decided.
pub(crate) const T_MELT: f64 = 1.4;
/// Act lengths (s): unfolded, collapse, folded core.
const ACT_S: [f64; 3] = [10.0, 16.0, 14.0];
/// Attempted moves per simulated second once cool; a hot chain is sampled more slowly
/// (see [`moves_per_s`]) so its beads stay distinct on screen instead of blurring.
const MOVES_PER_S: f64 = 40_000.0;

/// Attempted moves per second at temperature `t`: the full rate below `T = 1.6`, where
/// the fold is decided; down to 6 % of it in the hot coil.
pub(crate) fn moves_per_s(t: f64) -> f64 {
    MOVES_PER_S * (1.6 / t.max(1e-3)).powi(2).clamp(0.06, 1.0)
}
const DIRS: [P; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// A lattice site.
type P = (i16, i16);

fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}

fn adjacent(a: P, b: P) -> bool {
    (a.0 - b.0).abs() + (a.1 - b.1).abs() == 1
}

/// The chain on its occupancy grid, with the scratch of the move being tried.
#[derive(Debug, Clone)]
pub(crate) struct Chain {
    pub(crate) pos: [P; N],
    h: [bool; N],
    /// `G × G`, 0 = empty, else residue + 1.
    occ: Vec<u8>,
    /// Energy of `pos` (−contacts).
    pub(crate) energy: i32,
    /// The move being tried: residue and new site, and the moved residues' old sites.
    moved: [(u8, P); N],
    old: [P; N],
    n_moved: usize,
}

impl Chain {
    /// A straight chain in the middle of the grid.
    pub(crate) fn new() -> Self {
        let mut c = Self {
            pos: [(0, 0); N],
            h: [false; N],
            occ: vec![0; (G as usize) * (G as usize)],
            energy: 0,
            moved: [(0, (0, 0)); N],
            old: [(0, 0); N],
            n_moved: 0,
        };
        for (i, &r) in SEQ.iter().enumerate() {
            c.pos[i] = (G / 2 - N as i16 / 2 + i as i16, G / 2);
            c.h[i] = r == b'H';
            let k = c.idx(c.pos[i]);
            c.occ[k] = i as u8 + 1;
        }
        c.energy = c.energy_full();
        c
    }

    fn idx(&self, p: P) -> usize {
        p.1 as usize * G as usize + p.0 as usize
    }

    fn at(&self, p: P) -> Option<usize> {
        let v = self.occ[self.idx(p)];
        (v != 0).then(|| usize::from(v) - 1)
    }

    fn free(&self, p: P) -> bool {
        self.occ[self.idx(p)] == 0
    }

    pub(crate) fn is_h(&self, i: usize) -> bool {
        self.h[i]
    }

    /// H neighbours of residue `i` that are not its chain neighbours.
    pub(crate) fn contacts_of(&self, i: usize) -> usize {
        if !self.h[i] {
            return 0;
        }
        DIRS.iter()
            .filter_map(|&d| self.at(add(self.pos[i], d)))
            .filter(|&j| self.h[j] && j.abs_diff(i) > 1)
            .count()
    }

    /// `−(H–H contacts)`, from scratch.
    pub(crate) fn energy_full(&self) -> i32 {
        let mut n = 0;
        for i in 0..N {
            if self.h[i] {
                for &d in &DIRS {
                    if let Some(j) = self.at(add(self.pos[i], d)) {
                        n += i32::from(self.h[j] && j > i + 1);
                    }
                }
            }
        }
        -n
    }

    /// Free lattice sites (water) next to residue `i`.
    #[cfg(test)]
    pub(crate) fn exposure(&self, i: usize) -> usize {
        DIRS.iter()
            .filter(|&&d| self.free(add(self.pos[i], d)))
            .count()
    }

    /// Radius of gyration (lattice units).
    /// Radius of gyration (lattice units) of the H residues: the core's size (the polar
    /// loops and tails stay floppy even when folded).
    #[cfg(test)]
    pub(crate) fn rg_h(&self) -> f64 {
        let hs = || (0..N).filter(|&i| self.h[i]).map(|i| self.pos[i]);
        let n = hs().count().max(1) as f64;
        let (mx, my) = hs().fold((0.0, 0.0), |(x, y), p| {
            (x + f64::from(p.0) / n, y + f64::from(p.1) / n)
        });
        let s = hs().fold(0.0, |s, p| {
            s + (f64::from(p.0) - mx).powi(2) + (f64::from(p.1) - my).powi(2)
        });
        (s / n).sqrt()
    }

    fn push(&mut self, i: usize, p: P) {
        self.moved[self.n_moved] = (i as u8, p);
        self.n_moved += 1;
    }

    /// An end residue hops to another free site next to its neighbour.
    fn end_move(&mut self, i: usize, rng: &mut impl Rng) -> bool {
        let anchor = self.pos[if i == 0 { 1 } else { N - 2 }];
        let q = add(anchor, DIRS[rng.random_range(0..4)]);
        if q == self.pos[i] || !self.free(q) {
            return false;
        }
        self.push(i, q);
        true
    }

    /// A corner residue flips across the diagonal of its two neighbours.
    fn corner(&mut self, i: usize) -> bool {
        if i == 0 || i == N - 1 {
            return false;
        }
        let (a, b, p) = (self.pos[i - 1], self.pos[i + 1], self.pos[i]);
        if (a.0 - b.0).abs() != 1 || (a.1 - b.1).abs() != 1 {
            return false;
        }
        let q = (a.0 + b.0 - p.0, a.1 + b.1 - p.1);
        if !self.free(q) {
            return false;
        }
        self.push(i, q);
        true
    }

    /// A U of two residues `i, i + 1` swings to the other side of its base.
    fn crankshaft(&mut self, i: usize) -> bool {
        if i == 0 || i + 2 >= N {
            return false;
        }
        let (a, p, q, d) = (
            self.pos[i - 1],
            self.pos[i],
            self.pos[i + 1],
            self.pos[i + 2],
        );
        if !adjacent(a, d) || (p.0 - a.0, p.1 - a.1) != (q.0 - d.0, q.1 - d.1) {
            return false;
        }
        let (np, nq) = (
            (2 * a.0 - p.0, 2 * a.1 - p.1),
            (2 * d.0 - q.0, 2 * d.1 - q.1),
        );
        if !self.free(np) || !self.free(nq) {
            return false;
        }
        self.push(i, np);
        self.push(i + 1, nq);
        true
    }

    /// A pull move (Lesh et al. 2003) of virtual residue `vi` towards its anchor `vi + 1`,
    /// with virtual indices running down the chain (`down`) or up it: `vi` steps to a
    /// free site `L` beside the anchor, `vi − 1` to the corner `C` beside `L`, and the
    /// rest follow into the sites two ahead until the chain is connected again.
    fn pull(&mut self, vi: usize, down: bool, rng: &mut impl Rng) -> bool {
        let r = |k: usize| if down { k } else { N - 1 - k };
        if vi + 1 >= N {
            return false;
        }
        let (p, anchor) = (self.pos[r(vi)], self.pos[r(vi + 1)]);
        let e = (p.0 - anchor.0, p.1 - anchor.1);
        let perp = if rng.random_bool(0.5) {
            (-e.1, e.0)
        } else {
            (e.1, -e.0)
        };
        let (l, c) = (add(anchor, perp), add(p, perp));
        if !self.free(l) {
            return false;
        }
        self.push(r(vi), l);
        if vi == 0 || self.pos[r(vi - 1)] == c {
            // An end residue, or a corner move: done.
            return true;
        }
        if !self.free(c) {
            return false;
        }
        self.push(r(vi - 1), c);
        let mut last = c;
        for j in (0..vi - 1).rev() {
            if adjacent(self.pos[r(j)], last) {
                break;
            }
            // `pos` still holds the old sites: residue j takes j + 2's.
            let to = self.pos[r(j + 2)];
            self.push(r(j), to);
            last = to;
        }
        true
    }

    /// Move the residues in `moved`, if the result is self-avoiding.
    fn apply(&mut self) -> bool {
        for k in 0..self.n_moved {
            let i = usize::from(self.moved[k].0);
            self.old[k] = self.pos[i];
            let at = self.idx(self.pos[i]);
            self.occ[at] = 0;
        }
        for k in 0..self.n_moved {
            let (i, p) = self.moved[k];
            let at = self.idx(p);
            if self.occ[at] != 0 {
                for kk in 0..k {
                    let at = self.idx(self.moved[kk].1);
                    self.occ[at] = 0;
                }
                for kk in 0..self.n_moved {
                    let at = self.idx(self.old[kk]);
                    self.occ[at] = self.moved[kk].0 + 1;
                }
                return false;
            }
            self.occ[at] = i + 1;
        }
        for k in 0..self.n_moved {
            let (i, p) = self.moved[k];
            self.pos[usize::from(i)] = p;
        }
        true
    }

    fn revert(&mut self) {
        for k in 0..self.n_moved {
            let at = self.idx(self.moved[k].1);
            self.occ[at] = 0;
        }
        for k in 0..self.n_moved {
            let i = self.moved[k].0;
            self.pos[usize::from(i)] = self.old[k];
            let at = self.idx(self.old[k]);
            self.occ[at] = i + 1;
        }
    }

    /// One Metropolis attempt at temperature `t`; true if a move was made.
    pub(crate) fn attempt(&mut self, t: f64, rng: &mut impl Rng) -> bool {
        self.n_moved = 0;
        let i = rng.random_range(0..N);
        let proposed = match rng.random_range(0..4u8) {
            0 if i == 0 || i == N - 1 => self.end_move(i, rng),
            0 => self.corner(i),
            1 => self.crankshaft(i),
            _ => {
                let down = rng.random_bool(0.5);
                self.pull(i, down, rng)
            }
        };
        if !proposed || !self.apply() {
            return false;
        }
        let e = self.energy_full();
        let de = f64::from(e - self.energy);
        if de <= 0.0 || (t > 0.0 && rng.random::<f64>() < (-de / t).exp()) {
            self.energy = e;
            true
        } else {
            self.revert();
            false
        }
    }

    /// Recentre the chain on the grid if it strays near an edge; returns the shift.
    pub(crate) fn recentre(&mut self) -> Option<(i16, i16)> {
        let (mut lo, mut hi) = ((G, G), (0, 0));
        for p in &self.pos {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        if lo.0 >= MARGIN && lo.1 >= MARGIN && hi.0 < G - MARGIN && hi.1 < G - MARGIN {
            return None;
        }
        let shift = (G / 2 - (lo.0 + hi.0) / 2, G / 2 - (lo.1 + hi.1) / 2);
        for i in 0..N {
            let at = self.idx(self.pos[i]);
            self.occ[at] = 0;
        }
        for i in 0..N {
            self.pos[i] = add(self.pos[i], shift);
            let at = self.idx(self.pos[i]);
            self.occ[at] = i as u8 + 1;
        }
        Some(shift)
    }
}

/// Temperature at `local` seconds into the loop, and the act index.
pub(crate) fn schedule(local: f64) -> (f64, usize) {
    if local < ACT_S[0] {
        (T_HOT, 0)
    } else if local < ACT_S[0] + ACT_S[1] {
        let u = (local - ACT_S[0]) / ACT_S[1];
        (T_MELT * (T_COLD / T_MELT).powf(u), 1)
    } else {
        (T_COLD, 2)
    }
}

// ---- Scene ----

/// Seconds for beads to settle on their sites on screen (e-folding), and for the camera.
const BEAD_EASE_S: f64 = 0.05;
const CAM_FOLLOW_S: f64 = 0.1;
const CAM_ZOOM_S: f64 = 0.8;
/// Bead core and halo radii, in lattice steps.
const H_R: f64 = 0.34;
const P_R: f64 = 0.3;
const HALO_R: f64 = 0.62;
const BACKBONE: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 140);
const WATER: Role = Role::Ramp(Ramp::Cobalt);
const H_ROLE: Role = Role::Ramp(Ramp::Sulfur);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

#[derive(Debug)]
struct Folding {
    chain: Chain,
    /// Bead positions on screen (lattice units), easing towards `chain.pos`.
    disp: [(f64, f64); N],
    /// Camera centre (lattice units) and pixels per lattice step.
    cam: (f64, f64, f64),
    time: f64,
    owed: f64,
    started: bool,
    pixels: Pixels,
    dots: Dots,
    /// Per-frame sine tables of the water shimmer: two patterns, columns and rows.
    cols: Vec<[f32; 2]>,
    rows: Vec<[f32; 2]>,
    cells: Size,
    readout_room: usize,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Folding {
    fn default() -> Self {
        let chain = Chain::new();
        let mut disp = [(0.0, 0.0); N];
        for (d, p) in disp.iter_mut().zip(&chain.pos) {
            *d = (f64::from(p.0), f64::from(p.1));
        }
        Self {
            dirty: true,
            chain,
            disp,
            cam: (f64::from(G / 2), f64::from(G / 2), 1.0),
            time: 0.0,
            owed: 0.0,
            started: false,
            pixels: Pixels::default(),
            dots: Dots::default(),
            cols: Vec::new(),
            rows: Vec::new(),
            cells: Size::new(1, 1),
            readout_room: 0,
        }
    }
}

/// Blend `level` in `role` into pixel `(x, y)` by `cover`: over blank or same-ramp pixels
/// the levels mix (the ramps run to near-black, so this reads as partial coverage); over
/// another ink the pixel is taken only when mostly covered.
fn blend(px: &mut Pixels, x: i32, y: i32, cover: f64, ink: Ink) {
    let Some(under) = px.get(x, y) else {
        return;
    };
    let base = if under.role == ink.role {
        f64::from(under.level)
    } else if under.role == Role::Background || cover >= 0.5 {
        0.0
    } else {
        return;
    };
    let k = if under.role == ink.role || under.role == Role::Background {
        cover
    } else {
        1.0
    };
    let level = base + (f64::from(ink.level) - base) * k;
    px.set(x, y, Ink::new(ink.role, ink::level(level)));
}

impl Folding {
    fn local(&self) -> f64 {
        self.time % ACT_S.iter().sum::<f64>()
    }

    /// Pixel size of the art area (above the title row).
    fn art(&self) -> (f64, f64) {
        let ps = self.pixels.pixel_size();
        (
            f64::from(ps.width()),
            (f64::from(ps.height()) - 2.0).max(1.0),
        )
    }

    /// Where the camera wants to be: the chain's bounding box, framed with a margin,
    /// capped so a compact fold fills most of the height.
    /// The camera frames the chain's bounding box; it follows closely (so a hot chain that
    /// reptates across the lattice writhes in place instead of leaving the frame) and
    /// zooms slowly.
    fn cam_target(&self) -> (f64, f64, f64) {
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for d in &self.disp {
            lo = (lo.0.min(d.0), lo.1.min(d.1));
            hi = (hi.0.max(d.0), hi.1.max(d.1));
        }
        let (pw, ph) = self.art();
        let sp = (pw * 0.92 / (hi.0 - lo.0 + 1.4))
            .min(ph * 0.9 / (hi.1 - lo.1 + 1.4))
            .min(ph / 8.0)
            .max(0.5);
        ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0, sp)
    }

    fn to_px(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (pw, ph) = self.art();
        let (cx, cy, sp) = self.cam;
        (pw / 2.0 + (x - cx) * sp, ph / 2.0 + (y - cy) * sp)
    }

    fn rasterise(&mut self) {
        self.pixels.clear();
        self.dots.clear();
        // Water: two slow interference patterns, kept faint and sparse (caustic ridges).
        let t = self.time;
        for (x, c) in self.cols.iter_mut().enumerate() {
            let x = x as f64;
            *c = [
                (0.21 * x + 0.9 * t).sin_det() as f32,
                (0.13 * x - 0.6 * t).sin_det() as f32,
            ];
        }
        for (y, r) in self.rows.iter_mut().enumerate() {
            let y = y as f64;
            *r = [
                (0.17 * y - 0.7 * t).sin_det() as f32,
                (0.29 * y + 0.5 * t).sin_det() as f32,
            ];
        }
        let (cols, rows) = (&self.cols, &self.rows);
        self.pixels.shade(|x, y| {
            let (Some(c), Some(r)) = (cols.get(x as usize), rows.get(y as usize)) else {
                return Ink::BLANK;
            };
            let s = f64::from(c[0] * r[0] + c[1] * r[1]);
            let lvl = 16.0 * ease::smoothstep((s - 0.9) / 1.0);
            if lvl < 3.0 {
                Ink::BLANK
            } else {
                Ink::new(WATER, ink::level(lvl))
            }
        });
        let sp = self.cam.2;
        // Halos first: P's hydration shell (cool), H's warm glow.
        for i in 0..N {
            let c = self.to_px(self.disp[i]);
            let h = self.chain.is_h(i);
            raster::disc_pixels(c, HALO_R * sp, |x, y, cover| {
                let d = (f64::from(x) + 0.5 - c.0).hypot(f64::from(y) + 0.5 - c.1)
                    / (HALO_R * sp).max(0.5);
                let fall = (1.0 - d).max(0.0).powi(2);
                let ink = if h {
                    ink::blackbody(1100.0 + 1300.0 * fall)
                } else {
                    Ink::new(WATER, ink::level(62.0 * fall))
                };
                blend(&mut self.pixels, x, y, cover, ink);
            });
        }
        // Cores, lit from the upper left; buried H glows brighter.
        for i in 0..N {
            let c = self.to_px(self.disp[i]);
            let h = self.chain.is_h(i);
            let r = if h { H_R } else { P_R } * sp;
            let top = if h {
                150.0 + 42.0 * self.chain.contacts_of(i) as f64
            } else {
                200.0
            };
            let role = if h { H_ROLE } else { WATER };
            raster::disc_pixels(c, r, |x, y, cover| {
                let (ox, oy) = (f64::from(x) + 0.5 - c.0, f64::from(y) + 0.5 - c.1);
                let shade = 0.5 + 0.5 * (-(ox + oy) / (1.5 * r + 1.0)).clamp(-1.0, 1.0);
                let lvl = top.min(245.0) * (0.62 + 0.38 * shade);
                blend(
                    &mut self.pixels,
                    x,
                    y,
                    cover,
                    Ink::new(role, ink::level(lvl)),
                );
            });
        }
        // Backbone: a braille line between beads, stopping at their rims.
        for i in 0..N - 1 {
            let (a, b) = (self.to_px(self.disp[i]), self.to_px(self.disp[i + 1]));
            let (ra, rb) = (
                if self.chain.is_h(i) { H_R } else { P_R } * sp,
                if self.chain.is_h(i + 1) { H_R } else { P_R } * sp,
            );
            // Dots are half a pixel in both axes.
            let (a, b) = ((2.0 * a.0, 2.0 * a.1), (2.0 * b.0, 2.0 * b.1));
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            let n = (len * 1.2).ceil().clamp(1.0, 4096.0) as usize;
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                if t * len > 2.0 * ra + 0.6 && (1.0 - t) * len > 2.0 * rb + 0.6 {
                    self.dots.plot(x, y, BACKBONE);
                }
            }
        }
    }
}

/// ASCII: H `@`, P `o`; halos, water and dim pixels blank.
fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let pick = |i: Ink| match i.role {
        H_ROLE => Some(('@', i)),
        WATER if i.level >= 120 => Some(('o', i)),
        _ => None,
    };
    pick(top).or_else(|| pick(bottom))
}

impl Generator for Folding {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[schedule(self.local()).1])
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        self.pixels.resize(size);
        self.dots.resize(size);
        let ps = self.pixels.pixel_size();
        self.cols.clear();
        self.cols.resize(usize::from(ps.width()), [0.0; 2]);
        self.rows.clear();
        self.rows.resize(usize::from(ps.height()), [0.0; 2]);
        // The chain is size-independent; the camera snaps to frame it at the new size.
        self.cam = self.cam_target();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        if !self.started {
            // A random coil to start from, not a rod.
            self.started = true;
            for _ in 0..4000 {
                self.chain.attempt(1e3, &mut ctx.rng);
            }
            for (d, p) in self.disp.iter_mut().zip(&self.chain.pos) {
                *d = (f64::from(p.0), f64::from(p.1));
            }
            self.cam = self.cam_target();
        }
        self.time += dt;
        let (t, _) = schedule(self.local());
        self.owed += dt * moves_per_s(t);
        let n = self.owed.floor();
        self.owed -= n;
        for _ in 0..n as u32 {
            self.chain.attempt(t, &mut ctx.rng);
        }
        if let Some((sx, sy)) = self.chain.recentre() {
            // Keep the picture still across the grid shift.
            let (sx, sy) = (f64::from(sx), f64::from(sy));
            for d in &mut self.disp {
                *d = (d.0 + sx, d.1 + sy);
            }
            self.cam = (self.cam.0 + sx, self.cam.1 + sy, self.cam.2);
        }
        let k = 1.0 - (-dt / BEAD_EASE_S).exp();
        for (d, p) in self.disp.iter_mut().zip(&self.chain.pos) {
            d.0 += (f64::from(p.0) - d.0) * k;
            d.1 += (f64::from(p.1) - d.1) * k;
        }
        let target = self.cam_target();
        // The centre follows closely, the zoom slowly (a gentle push-in as it folds).
        let (kc, kz) = (
            1.0 - (-dt / CAM_FOLLOW_S).exp(),
            1.0 - (-dt / CAM_ZOOM_S).exp(),
        );
        self.cam = (
            self.cam.0 + (target.0 - self.cam.0) * kc,
            self.cam.1 + (target.1 - self.cam.1) * kc,
            // Zooming out is quick, so a stretching chain never leaves the frame.
            if target.2 < self.cam.2 {
                self.cam.2 + (target.2 - self.cam.2) * kc
            } else {
                self.cam.2 + (target.2 - self.cam.2) * kz
            },
        );
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
        self.dots.blit_over(canvas, &self.pixels);
        let h = i32::from(self.cells.height());
        if h < 4 {
            return;
        }
        let contacts = -self.chain.energy;
        let mut label = StackText::<24>::new();
        let _ = match self.readout_room {
            16.. => write!(label, "HH contacts {contacts:>2}"),
            3.. => write!(label, "{contacts:>2}"),
            _ => return,
        };
        draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected_and_consistent(c: &Chain) {
        for i in 0..N - 1 {
            assert!(adjacent(c.pos[i], c.pos[i + 1]), "bond {i}");
        }
        for i in 0..N {
            assert_eq!(c.at(c.pos[i]), Some(i), "occupancy of {i}");
        }
        let occupied = c.occ.iter().filter(|&&v| v != 0).count();
        assert_eq!(occupied, N, "self-avoiding: {N} distinct sites");
        assert_eq!(c.energy, c.energy_full());
    }

    #[test]
    fn hpf_sequence_is_the_48_residue_benchmark() {
        assert_eq!(SEQ.len(), N);
        assert_eq!(SEQ.iter().filter(|&&r| r == b'H').count(), 25);
        assert!(SEQ.iter().all(|&r| r == b'H' || r == b'P'));
    }

    #[test]
    fn hpf_a1_moves_keep_the_chain_connected_self_avoiding_and_consistent() {
        let mut rng = SplitMix64::new(1);
        let mut c = Chain::new();
        let mut made = 0;
        for k in 0..6000 {
            let t = [5.0, 1.0, 0.3][k % 3];
            made += usize::from(c.attempt(t, &mut rng));
            connected_and_consistent(&c);
            c.recentre();
        }
        assert!(made > 500, "{made} moves made");
    }

    #[test]
    fn hpf_a2_zero_temperature_never_climbs() {
        let mut rng = SplitMix64::new(2);
        let mut c = Chain::new();
        for _ in 0..3000 {
            c.attempt(5.0, &mut rng);
        }
        let mut e = c.energy;
        for _ in 0..5000 {
            c.attempt(0.0, &mut rng);
            assert!(c.energy <= e, "rose {e} → {}", c.energy);
            e = c.energy;
        }
        // Hot: uphill moves happen.
        let mut ups = 0;
        for _ in 0..5000 {
            let before = c.energy;
            c.attempt(3.0, &mut rng);
            ups += usize::from(c.energy > before);
        }
        assert!(ups > 10, "{ups} uphill moves");
    }

    fn run_to(g: &mut Folding, ctx: &mut Ctx, t: f64) {
        while g.time < t {
            g.update(1.0 / 60.0, ctx);
            g.prepare();
        }
    }

    #[test]
    fn hpf_a3_annealing_collapses_the_chain() {
        for seed in [1, 2, 3] {
            let mut ctx = Ctx::new(seed, Size::new(80, 24));
            let mut g = Folding::default();
            g.resize(Size::new(80, 24), &mut ctx);
            g.prepare();
            let mut rg_hot = 0.0;
            let mut n = 0.0;
            while g.time < ACT_S[0] {
                g.update(1.0 / 60.0, &mut ctx);
                g.prepare();
                if g.time > 2.0 {
                    rg_hot += g.chain.rg_h();
                    n += 1.0;
                }
            }
            rg_hot /= n;
            run_to(&mut g, &mut ctx, ACT_S[0] + ACT_S[1]);
            let (rg, contacts) = (g.chain.rg_h(), -g.chain.energy);
            assert!(
                rg < 0.6 * rg_hot && contacts >= 14,
                "seed {seed}: Rg {rg_hot:.1} → {rg:.1}, {contacts} contacts"
            );
        }
    }

    #[test]
    fn hpf_a4_h_residues_face_the_water_less_than_p() {
        let mut ctx = Ctx::new(4, Size::new(80, 24));
        let mut g = Folding::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        run_to(&mut g, &mut ctx, ACT_S[0] + ACT_S[1] + 2.0);
        let (mut h, mut p, mut nh, mut np) = (0u32, 0u32, 0u32, 0u32);
        for i in 0..N {
            if g.chain.is_h(i) {
                h += g.chain.exposure(i) as u32;
                nh += 1;
            } else {
                p += g.chain.exposure(i) as u32;
                np += 1;
            }
        }
        let (h, p) = (f64::from(h) / f64::from(nh), f64::from(p) / f64::from(np));
        assert!(h < p, "exposure H {h:.2} vs P {p:.2}");
    }

    #[test]
    fn hpf_a5_captions_follow_the_acts() {
        let mut ctx = Ctx::new(5, Size::new(80, 24));
        let mut g = Folding::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        let mut start = 0.0;
        for (k, c) in CAPTIONS.iter().enumerate() {
            run_to(&mut g, &mut ctx, start + 1.0);
            assert_eq!(g.caption(), Some(*c));
            start += ACT_S[k];
        }
        run_to(&mut g, &mut ctx, start + 1.0);
        assert_eq!(
            g.caption(),
            Some(CAPTIONS[0]),
            "the loop heats the chain again"
        );
        assert!(schedule(ACT_S[0] + ACT_S[1] - 1e-6).0 < 1.05 * T_COLD);
    }
}
