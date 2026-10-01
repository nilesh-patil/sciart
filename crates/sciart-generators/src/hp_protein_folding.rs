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
use sciart_core::{draw, title};

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
/// The loop: the three acts.
const LOOP_S: f64 = ACT_S[0] + ACT_S[1] + ACT_S[2];

// M16 (GEN-R25, strange_attractors' thread idiom): all braille dots on black.

/// The backbone: a Catmull-Rom spline through the beads, one dot thick, below the
/// residues' levels so a bead's cell keeps the bead's colour.
const BACKBONE: Ink = Ink::new(Role::Structure, 110);
/// Residue dots (M11's complementary pair, ADR-0018): H (hydrophobic) `Hot`, a step
/// brighter with each H·H contact up to two (buried cores read hot); P (polar) `Cold`.
/// A small braille disc whose radius follows the zoom (`BEAD_FRAC` of a lattice step,
/// within `BEAD_R` dots).
const H_INKS: [Ink; 3] = [
    Ink::new(Role::Hot, 130),
    Ink::new(Role::Hot, 190),
    Ink::new(Role::Hot, 255),
];
const P_INK: Ink = Ink::new(Role::Cold, 170);
const BEAD_FRAC: f64 = 0.16;
const BEAD_R: (f64, f64) = (0.5, 2.5);
/// An H–H contact: a faint dotted `Accent` link (every other dot) between the two
/// residues, stopping short of their dots.
const CONTACT_INK: Ink = Ink::new(Role::Accent, 90);
/// The energy panel (right): `E(t)` over one loop, sampled `TRACE_N` times, as a dot
/// line coloured by the temperature it was sampled at (`Sun` hot coil, `Accent` while
/// cooling above `T_WARM`, `Wave` cold), descending from 0 (top) towards the best
/// known fold `E_BEST` (a dashed `Dim` floor) on an axis down to `E_AXIS`; its head a
/// `Star` dot. Shown when the scene is at least `PANEL_MIN` cells wide and 12 high.
const TRACE_N: usize = 80;
const E_AXIS: f64 = -24.0;
const E_BEST: f64 = -23.0;
const T_WARM: f64 = 0.5;
const TRACE_INKS: [Ink; 3] = [
    Ink::new(Role::Sun, 200),
    Ink::new(Role::Accent, 200),
    Ink::new(Role::Wave, 230),
];
const HEAD_INK: Ink = Ink::new(Role::Star, 255);
const AXIS_INK: Ink = Ink::new(Role::Dim, 90);
const PANEL_MIN: u16 = 60;
/// The panel's box in fractions of the width and of the art height (above the title).
const PANEL_X: (f64, f64) = (0.66, 0.96);
const PANEL_Y: (f64, f64) = (0.16, 0.84);
/// The chain's share of the width when the panel shows.
const CHAIN_FRAC: f64 = 0.62;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// The trace ink of a sample taken at loop time `local`.
pub(crate) fn trace_ink(local: f64) -> Ink {
    match schedule(local) {
        (_, 0) => TRACE_INKS[0],
        (t, _) if t > T_WARM => TRACE_INKS[1],
        _ => TRACE_INKS[2],
    }
}

#[derive(Debug)]
struct Folding {
    chain: Chain,
    /// Bead positions on screen (lattice units), easing towards `chain.pos`.
    disp: [(f64, f64); N],
    /// Camera centre (lattice units) and dots per lattice step.
    cam: (f64, f64, f64),
    time: f64,
    owed: f64,
    started: bool,
    /// The energy over `TRACE_N` equal slices of the loop (each its mean over the
    /// updates in the slice), the first `n_trace` of them taken so far this loop.
    trace: [f32; TRACE_N],
    /// Sum and count of the energies since the last sample.
    acc: (f64, u32),
    n_trace: usize,
    /// The loop the trace belongs to.
    trace_loop: u64,
    dots: Dots,
    cells: Size,
    readout_room: usize,
    /// The dots are stale: the state moved since the last `rasterise` (GEN-R20).
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
            trace: [0.0; TRACE_N],
            acc: (0.0, 0),
            n_trace: 0,
            trace_loop: 0,
            dots: Dots::default(),
            cells: Size::new(1, 1),
            readout_room: 0,
        }
    }
}

impl Folding {
    fn local(&self) -> f64 {
        self.time % LOOP_S
    }

    fn panel(&self) -> bool {
        self.cells.width() >= PANEL_MIN && self.cells.height() >= 12
    }

    /// Dot size of the art area (above the title row).
    fn art(&self) -> (f64, f64) {
        let ds = self.dots.dot_size();
        (
            f64::from(ds.width()),
            (f64::from(ds.height()) - f64::from(Dots::CELL_H)).max(1.0),
        )
    }

    /// The chain's region: its width in dots (the left part when the panel shows).
    fn chain_w(&self) -> f64 {
        let w = self.art().0;
        if self.panel() { w * CHAIN_FRAC } else { w }
    }

    /// Where the camera wants to be: the chain's bounding box, framed with a margin,
    /// capped so a compact fold fills most of the height. It follows closely (so a hot
    /// chain that reptates across the lattice writhes in place instead of leaving the
    /// frame) and zooms slowly.
    fn cam_target(&self) -> (f64, f64, f64) {
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for d in &self.disp {
            lo = (lo.0.min(d.0), lo.1.min(d.1));
            hi = (hi.0.max(d.0), hi.1.max(d.1));
        }
        let (dw, dh) = (self.chain_w(), self.art().1);
        let sp = (dw * 0.92 / (hi.0 - lo.0 + 1.4))
            .min(dh * 0.9 / (hi.1 - lo.1 + 1.4))
            .min(dh / 8.0)
            .max(1.0);
        ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0, sp)
    }

    fn to_dot(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (dw, dh) = (self.chain_w(), self.art().1);
        let (cx, cy, sp) = self.cam;
        (dw / 2.0 + (x - cx) * sp, dh / 2.0 + (y - cy) * sp)
    }

    /// Record the energy at every trace time passed this loop.
    fn sample(&mut self) {
        let lp = (self.time / LOOP_S).floor() as u64;
        if lp != self.trace_loop {
            self.trace_loop = lp;
            self.n_trace = 0;
        }
        let due = ((self.local() / LOOP_S * TRACE_N as f64) as usize + 1).min(TRACE_N);
        self.acc.0 += f64::from(self.chain.energy);
        self.acc.1 += 1;
        if self.n_trace < due {
            // The slice's mean energy: a smooth thread, not Metropolis flicker.
            let e = (self.acc.0 / f64::from(self.acc.1)) as f32;
            self.acc = (0.0, 0);
            while self.n_trace < due {
                self.trace[self.n_trace] = e;
                self.n_trace += 1;
            }
        }
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        self.backbone();
        self.contacts();
        self.residues();
        if self.panel() {
            self.energy_panel();
        }
    }

    /// The backbone: a centripetal-free (uniform) Catmull-Rom spline through the beads.
    fn backbone(&mut self) {
        let mut prev = self.to_dot(self.disp[0]);
        for i in 0..N - 1 {
            let p = |k: usize| self.to_dot(self.disp[k.min(N - 1)]);
            let (p0, p1, p2, p3) = (p(i.saturating_sub(1)), p(i), p(i + 1), p(i + 2));
            let len = (p2.0 - p1.0).hypot(p2.1 - p1.1);
            let n = (len / 2.0).ceil().clamp(1.0, 64.0) as usize;
            for k in 1..=n {
                let t = k as f64 / n as f64;
                let q = catmull_rom(p0, p1, p2, p3, t);
                self.dots.line(prev.0, prev.1, q.0, q.1, BACKBONE);
                prev = q;
            }
        }
    }

    fn bead_r(&self) -> f64 {
        (self.cam.2 * BEAD_FRAC).clamp(BEAD_R.0, BEAD_R.1)
    }

    /// Faint dotted links between H residues in contact.
    fn contacts(&mut self) {
        let skip = self.bead_r() + 1.5;
        for i in 0..N {
            if !self.chain.is_h(i) {
                continue;
            }
            for &d in &DIRS {
                let Some(j) = self.chain.at(add(self.chain.pos[i], d)) else {
                    continue;
                };
                if j <= i + 1 || !self.chain.is_h(j) {
                    continue;
                }
                let (a, b) = (self.to_dot(self.disp[i]), self.to_dot(self.disp[j]));
                let len = (b.0 - a.0).hypot(b.1 - a.1);
                // Only settled pairs: beads still easing after a move draw no link
                // (it would cut across the chain).
                let settled = |k: usize| {
                    let (d, p) = (self.disp[k], self.chain.pos[k]);
                    (d.0 - f64::from(p.0)).hypot(d.1 - f64::from(p.1)) < 0.35
                };
                if len > 1.35 * self.cam.2 || !settled(i) || !settled(j) {
                    continue;
                }
                let n = len.floor() as i32;
                for k in (0..=n).step_by(2) {
                    let s = f64::from(k);
                    if s > skip && len - s > skip {
                        let t = s / len;
                        self.dots
                            .plot(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, CONTACT_INK);
                    }
                }
            }
        }
    }

    fn residues(&mut self) {
        let r = self.bead_r();
        for i in 0..N {
            let c = self.to_dot(self.disp[i]);
            let ink = if self.chain.is_h(i) {
                H_INKS[self.chain.contacts_of(i).min(2)]
            } else {
                P_INK
            };
            self.dots.disc(c.0, c.1, r, ink);
        }
    }

    /// Dot coordinates of the panel point at trace sample `k` and energy `e`.
    fn panel_at(&self, k: f64, e: f64) -> (f64, f64) {
        let (w, h) = self.art();
        let (x0, x1) = (PANEL_X.0 * w, PANEL_X.1 * w);
        let (y0, y1) = (PANEL_Y.0 * h, PANEL_Y.1 * h);
        (
            x0 + (x1 - x0) * k / (TRACE_N - 1) as f64,
            y0 + (y1 - y0) * (e / E_AXIS).clamp(0.0, 1.0),
        )
    }

    /// The energy trace descending the folding funnel.
    fn energy_panel(&mut self) {
        // Axis: a dotted vertical at the left; the best known fold a dashed floor.
        let (top, bottom) = (self.panel_at(0.0, 0.0), self.panel_at(0.0, E_AXIS));
        let mut y = top.1;
        while y <= bottom.1 {
            self.dots.plot(top.0 - 3.0, y, AXIS_INK);
            y += 2.0;
        }
        let (a, b) = (
            self.panel_at(0.0, E_BEST),
            self.panel_at((TRACE_N - 1) as f64, E_BEST),
        );
        let mut x = a.0;
        while x <= b.0 {
            self.dots.plot(x, a.1, AXIS_INK);
            x += 3.0;
        }
        let Some(&first) = self.trace.first() else {
            return;
        };
        let mut prev = self.panel_at(0.0, f64::from(first));
        for k in 1..self.n_trace {
            let q = self.panel_at(k as f64, f64::from(self.trace[k]));
            let ink = trace_ink(LOOP_S * k as f64 / TRACE_N as f64);
            self.dots.line(prev.0, prev.1, q.0, q.1, ink);
            prev = q;
        }
        if self.n_trace > 0 {
            self.dots.disc(prev.0, prev.1, 0.8, HEAD_INK);
        }
    }
}

/// Uniform Catmull-Rom between `p1` and `p2` at `t ∈ [0, 1]`.
fn catmull_rom(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let (t2, t3) = (t * t, t * t * t);
    let f = |a: f64, b: f64, c: f64, d: f64| {
        0.5 * (2.0 * b
            + (c - a) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (3.0 * b - a - 3.0 * c + d) * t3)
    };
    (f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1))
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
        self.dots.resize(size);
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
        self.sample();
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
        self.dots.blit(canvas);
        let h = i32::from(self.cells.height());
        if h < 4 {
            return;
        }
        if self.panel() {
            // "energy" above the axis.
            let (x, y) = self.panel_at(0.0, 0.0);
            let (cx, cy) = (
                ((x - 3.0) / f64::from(Dots::CELL_W)) as i32,
                (y / f64::from(Dots::CELL_H)) as i32 - 1,
            );
            draw::label(canvas, cx, cy.max(0), "energy", LABEL_INK);
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

    #[test]
    fn hpf_a6_residues_are_hot_h_and_cold_p_dots_on_a_braille_chain() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(6, size);
        let mut g = Folding::default();
        g.resize(size, &mut ctx);
        run_to(&mut g, &mut ctx, 30.0);
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let (mut hot, mut cold, mut top) = (0, 0, 0);
        let w = usize::from(size.width());
        for (k, c) in canvas.cells().iter().enumerate() {
            if c.ch == ' ' || c.ink.role == Role::Text {
                continue;
            }
            assert!(
                c.bg == Ink::BLANK && ('\u{2800}'..='\u{28ff}').contains(&c.ch),
                "not a braille cell: {c:?}"
            );
            let in_chain = ((k % w) as f64) < CHAIN_FRAC * w as f64;
            match c.ink {
                i if H_INKS.contains(&i) => {
                    hot += 1;
                    top += usize::from(i == H_INKS[2]);
                }
                i if i == P_INK => cold += 1,
                i if i == BACKBONE || i == CONTACT_INK => assert!(in_chain, "{c:?}"),
                i if TRACE_INKS.contains(&i) || i == HEAD_INK || i == AXIS_INK => {
                    assert!(!in_chain, "trace in the chain's region: {c:?}");
                }
                i => panic!("unexpected ink {i:?}"),
            }
        }
        assert!(hot > 0 && cold > 0, "{hot} H, {cold} P cells");
        // The folded core carries buried H at the top step.
        assert!(top > 0);
    }

    #[test]
    fn hpf_a7_the_energy_trace_records_the_chain_and_descends_the_funnel() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(7, size);
        let mut g = Folding::default();
        g.resize(size, &mut ctx);
        let mut checked = 0;
        let mut seen = (f64::MAX, f64::MIN);
        while g.time < LOOP_S - 0.5 {
            let before = g.n_trace;
            g.update(1.0 / 60.0, &mut ctx);
            let e = f64::from(g.chain.energy);
            seen = (seen.0.min(e), seen.1.max(e));
            // A sample just taken is the mean energy over its slice of the loop.
            if g.n_trace > before {
                let s = f64::from(g.trace[g.n_trace - 1]);
                assert!(
                    seen.0 - 1e-6 <= s && s <= seen.1 + 1e-6,
                    "{s} outside {seen:?}"
                );
                seen = (f64::MAX, f64::MIN);
                checked += 1;
            }
        }
        assert!(checked > 0 && g.n_trace > TRACE_N - 5, "{}", g.n_trace);
        let mean = |a: f64, b: f64| {
            let (i, j) = (
                (a / LOOP_S * TRACE_N as f64) as usize,
                (b / LOOP_S * TRACE_N as f64) as usize,
            );
            g.trace[i..j].iter().map(|&e| f64::from(e)).sum::<f64>() / (j - i) as f64
        };
        let (coil, fold) = (mean(1.0, ACT_S[0]), mean(LOOP_S - 8.0, LOOP_S - 1.0));
        assert!(
            coil > -6.0 && fold <= -14.0,
            "E: coil {coil:.1}, fold {fold:.1}"
        );
        // The trace is coloured by temperature: hot, cooling, cold.
        assert_eq!(trace_ink(1.0), TRACE_INKS[0]);
        assert_eq!(trace_ink(ACT_S[0] + 0.5), TRACE_INKS[1]);
        assert_eq!(trace_ink(LOOP_S - 1.0), TRACE_INKS[2]);
        // The next loop starts a fresh trace.
        while g.time < LOOP_S + 1.0 {
            g.update(1.0 / 60.0, &mut ctx);
        }
        assert!(g.n_trace < 10, "{}", g.n_trace);
        // Drawn: the head dot in the panel, right of the chain.
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let w = usize::from(size.width());
        assert!(
            canvas
                .cells()
                .iter()
                .enumerate()
                .any(|(k, c)| c.ink == HEAD_INK && (k % w) as f64 > CHAIN_FRAC * w as f64)
        );
    }
}
