//! A frost dendrite growing from a seed by diffusion-limited aggregation (Witten–Sander),
//! glowing at its growing tips, then melting from the tips inward and regrowing
//! (SPEC-002 `fractal_growth`).
//!
//! Physics: random walkers diffuse in from a launch circle just outside the cluster and
//! freeze onto it when they touch it (8-neighbourhood) with sticking probability `p_s`.
//!
//! Aspect correction: the aggregate lives on the braille sub-cell lattice ([`Dots`],
//! 2 × 4 dots per terminal cell). With cells ~2:1 tall (`ASPECT`), that lattice is
//! *square* — 1/4 row a side — so walks are isotropic with four equally likely steps
//! ([`walk_step`]), and so are the contact distance and the size of each frozen particle. (On the raw 2:1 cell
//! grid, even with `ASPECT²`-weighted horizontal steps, vertical contacts reach twice as far
//! as horizontal ones and a horizontal arm costs twice the particles per row, which grows
//! vertical needles with `D_f ≈ 1.4`.) The fine lattice also gives 4× the linear
//! resolution, which a sparse fractal needs: a DLA cluster 60 spacings in radius holds only
//! ~600–1000 particles (measured).
//!
//! Efficiency (standard DLA accelerations):
//! - a free walk first leaves a disc uniformly on its rim, so a walker inside a
//!   crystal-free disc jumps straight to a uniform point on its circle (one budget step).
//!   Far out the disc has radius `d − r_launch` (`r_launch = r_max + gap`); near the
//!   crystal it comes from a capped map of the distance to the nearest crystal site,
//!   updated around each new site (the Meakin / Ball–Brady trick), so walkers cross the
//!   fjords in a few hops instead of hundreds of unit steps;
//! - a walker beyond the kill radius `2 × r_launch` is relaunched on the launch circle.
//!
//! The lattice walls reflect; a jump that would cross a wall is mirrored back (method of
//! images: exact for the walk's position, an approximation for its first-passage point)
//! and only kept if it lands well clear of the crystal.
//!
//! The crystal's colours age every step, so its `Dots` raster is rebuilt at the end of
//! `update` (and `resize`); `render` only blits it (the M10 raster convention).

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{draw, grid, raster};

pub static META: Meta = Meta {
    name: "fractal_growth",
    category: Category::PhysicalChemistry,
    description: "A dendrite growing by diffusion-limited aggregation, then melting and regrowing",
    title: "Fractal growth · walkers stick",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(FractalGrowth::default())
}

/// Braille dots per terminal cell; `SUB_Y / SUB_X = ASPECT` makes the dot lattice square.
const SUB_X: i32 = Dots::CELL_W;
const SUB_Y: i32 = Dots::CELL_H;

/// Most lattice sites (ADR-0012, TEST-R7): up to about 230×62 cells (114 000 dots) the
/// lattice is the whole dot lattice, as at M11; larger terminals grow the crystal in a
/// centred window of about this many dots, so live memory stays bounded.
const MAX_NODES: usize = 115_000;

/// Attach-time sentinel for an empty site (real attach times are ≥ 0).
const EMPTY: f32 = -1.0;
/// Walker-step budget per simulated second at 200×50 (the spec caps a frame at 4000; with
/// the jump accelerations a crystal needs far fewer). Measured cost grows ~R^2.5 with the
/// crystal radius R, so the budget scales as `(R / R_REF)^2.5` for a ~15 s growth at any
/// size, within `[STEPS_MIN, STEPS_MAX]`.
const STEPS_REF: f64 = 4_400.0;
const R_REF_ROWS: f64 = 21.25;
const COST_EXP: f64 = 2.5;
const STEPS_MIN: f64 = 400.0;
const STEPS_MAX: f64 = 240_000.0;
/// Concurrent walkers: a tiny fraction of the cluster, so growth is effectively
/// one-particle-at-a-time (Witten–Sander), yet enough to draw a faint "vapour".
const WALKERS: usize = 12;
/// Launch circle radius = `r_max + LAUNCH_GAP`, lattice units ("just outside").
const LAUNCH_GAP: f64 = 2.0;
/// Kill radius = `KILL_FACTOR × r_launch` (spec: 2×).
const KILL_FACTOR: f64 = 2.0;
/// Shortest jump worth taking instead of a unit step, lattice units.
const JUMP_MIN: f64 = 1.5;
/// Cap of the nearest-crystal distance map, lattice units (bigger = more update work per
/// new site, longer jumps near the crystal).
const FREE_CAP: u8 = 10;
/// A mirrored (wall-crossing) jump must land at least this far from any crystal.
const FOLD_CLEAR: u8 = 3;
/// A walker within this distance of `r_max` may touch the crystal (diagonal contact √2).
const TOUCH_REACH: f64 = 1.5;
/// Growth stops at `R(GROW_FRAC)` or after `MAX_GROW` s; then hold, then melt (s).
const GROW_FRAC: f64 = 0.85;
const MAX_GROW: f64 = 60.0;
const HOLD: f64 = 3.0;
const MELT: f64 = 3.0;
/// Sticking probability range (lower = denser, fuzzier crystals).
const P_STICK: (f64, f64) = (0.3, 1.0);
/// Live mass–radius fit, lattice units: from a few spacings up to a fraction of `r_max`
/// (the outer shell is still growing, so it is not yet at its asymptotic density).
const FIT_LO: f64 = 3.0;
const FIT_HI_FRAC: f64 = 0.5;
/// How often the live D_f readout is refreshed, s.
const DF_TICK: f64 = 0.5;
/// Width of the melting front, in chemical-distance steps.
const MELT_BAND: f64 = 8.0;

// Age-colouring timescales, s.
const TIP_AGE: f64 = 0.3;
const WARM_AGE: f64 = 2.0;
const COOL_TAU: f64 = 10.0;

/// One isotropic lattice step `(dx, dy)` for a uniform `u ∈ [0, 1)`: the four directions
/// with equal probability, so `E[Δx²] = E[Δy²] = 1/2` on the square lattice.
pub(crate) fn walk_step(u: f64) -> (i32, i32) {
    match (u * 4.0) as u8 {
        0 => (-1, 0),
        1 => (1, 0),
        2 => (0, -1),
        _ => (0, 1),
    }
}

/// Mass–radius fractal dimension: least-squares slope of `ln N(r)` against `ln r` for
/// ten log-spaced radii in `[r_lo, r_hi]`, where `N(r)` counts `dists ≤ r` (CRY-A4).
///
/// `None` if the range is too narrow (`r_hi < 1.5 r_lo`) or fewer than 3 radii hold mass.
/// O(10 n) and allocation-free, so the live readout can call it.
pub(crate) fn mass_radius_dimension<I: Iterator<Item = f64> + Clone>(
    dists: I,
    r_lo: f64,
    r_hi: f64,
) -> Option<f64> {
    const SAMPLES: usize = 10;
    if !(r_lo > 0.0 && r_hi.is_finite() && r_hi >= 1.5 * r_lo) {
        return None;
    }
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for k in 0..SAMPLES {
        let r = r_lo * (r_hi / r_lo).powf(k as f64 / (SAMPLES - 1) as f64);
        let mass = dists.clone().filter(|&d| d <= r).count();
        if mass == 0 {
            continue;
        }
        let (x, y) = (r.ln(), (mass as f64).ln());
        n += 1.0;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let den = n * sxx - sx * sx;
    (n >= 3.0 && den > 0.0).then(|| (n * sxy - sx * sy) / den)
}

/// The aggregate and its walkers on a square lattice.
#[derive(Debug, Default)]
struct Dla {
    seed: (i32, i32),
    /// Attach time per site (run seconds), [`EMPTY`] if free (CRY-A2).
    attach: Grid<f32>,
    /// Occupied sites in attach order; `cells[0]` is the seed.
    cells: Vec<(i32, i32)>,
    walkers: Vec<(i32, i32)>,
    /// Round-robin cursor, kept across calls so small budgets still move every walker.
    turn: usize,
    /// `floor` of the distance to the nearest crystal site, capped at [`FREE_CAP`].
    free: Grid<u8>,
    /// Largest seed distance of any crystal site, lattice units.
    r_max: f64,
    p_stick: f64,
}

impl Dla {
    /// A `lattice`-sized aggregate with `walkers` walkers; call [`Dla::reset`] to seed it.
    fn new(lattice: Size, walkers: usize) -> Self {
        Self {
            seed: (0, 0),
            attach: Grid::new(lattice, EMPTY),
            cells: Vec::with_capacity(lattice.area()),
            walkers: vec![(0, 0); walkers],
            turn: 0,
            free: Grid::new(lattice, FREE_CAP),
            r_max: 0.0,
            p_stick: 1.0,
        }
    }

    /// A fresh single-site seed at `seed` (clamped onto the lattice) and fresh walkers.
    fn reset(&mut self, seed: (i32, i32), p_stick: f64, rng: &mut impl Rng) {
        let (w, h) = self.dims();
        self.attach.fill(EMPTY);
        self.free.fill(FREE_CAP);
        self.cells.clear();
        self.seed = (
            seed.0.clamp(0, (w - 1).max(0)),
            seed.1.clamp(0, (h - 1).max(0)),
        );
        self.r_max = 0.0;
        self.p_stick = p_stick;
        if self.attach.set(self.seed.0, self.seed.1, 0.0) {
            self.cells.push(self.seed);
            self.mark_free(self.seed);
        }
        for i in 0..self.walkers.len() {
            self.walkers[i] = self.launch(rng);
        }
    }

    fn dims(&self) -> (i32, i32) {
        (
            i32::from(self.attach.width()),
            i32::from(self.attach.height()),
        )
    }

    /// Distance from the seed, lattice units.
    #[inline]
    fn dist(&self, x: i32, y: i32) -> f64 {
        f64::from(x - self.seed.0).hypot(f64::from(y - self.seed.1))
    }

    fn r_launch(&self) -> f64 {
        self.r_max + LAUNCH_GAP
    }

    fn r_kill(&self) -> f64 {
        KILL_FACTOR * self.r_launch()
    }

    #[inline]
    fn occupied(&self, x: i32, y: i32) -> bool {
        self.attach.get(x, y).is_some_and(|&a| a >= 0.0)
    }

    /// Does any Moore neighbour hold crystal? (The sticking rule, so every new site is
    /// 8-adjacent to an old one: CRY-A1 by construction.)
    fn touches(&self, x: i32, y: i32) -> bool {
        (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && self.occupied(x + dx, y + dy)))
    }

    /// Uniform point on the launch circle, clamped onto the lattice. Clamping each axis
    /// toward the seed only shortens the offset, so it stays inside the kill radius (CRY-A3).
    fn launch(&self, rng: &mut impl Rng) -> (i32, i32) {
        let th = rng.random_range(0.0..TAU);
        let r = self.r_launch();
        let (w, h) = self.dims();
        let x = (f64::from(self.seed.0) + r * th.cos_det()).round() as i32;
        let y = (f64::from(self.seed.1) + r * th.sin_det()).round() as i32;
        (x.clamp(0, (w - 1).max(0)), y.clamp(0, (h - 1).max(0)))
    }

    /// Lower the nearest-crystal distance map around a new site `c`.
    fn mark_free(&mut self, c: (i32, i32)) {
        let cap = i32::from(FREE_CAP);
        for dy in -cap..=cap {
            for dx in -cap..=cap {
                // floor(√(dx² + dy²)), made exact in integers after the float estimate.
                let d2 = dx * dx + dy * dy;
                let mut d = f64::from(d2).sqrt() as i32;
                if (d + 1) * (d + 1) <= d2 {
                    d += 1;
                } else if d * d > d2 {
                    d -= 1;
                }
                if d < cap
                    && let Some(f) = self.free.get_mut(c.0 + dx, c.1 + dy)
                {
                    *f = (*f).min(d as u8);
                }
            }
        }
    }

    /// Mirror `v` into `0..=max` (reflecting walls).
    fn fold(v: i32, max: i32) -> i32 {
        let v = v.abs();
        let v = if v > max { 2 * max - v } else { v };
        v.clamp(0, max.max(0))
    }

    /// `steps` walker moves, round-robin over the walkers; new sites get attach time `t`.
    fn advance(&mut self, steps: usize, t: f64, rng: &mut impl Rng) {
        if self.walkers.is_empty() || self.cells.is_empty() {
            return;
        }
        let n = self.walkers.len();
        for _ in 0..steps {
            self.turn = (self.turn + 1) % n;
            self.step_walker(self.turn, t, rng);
        }
    }

    fn step_walker(&mut self, i: usize, t: f64, rng: &mut impl Rng) {
        let (x, y) = self.walkers[i];
        if self.occupied(x, y) {
            self.walkers[i] = self.launch(rng);
            return;
        }
        let d = self.dist(x, y);
        if d <= self.r_max + TOUCH_REACH && self.touches(x, y) && rng.random::<f64>() < self.p_stick
        {
            self.attach.set(x, y, t as f32);
            self.cells.push((x, y));
            self.mark_free((x, y));
            self.r_max = self.r_max.max(d);
            self.walkers[i] = self.launch(rng);
            return;
        }

        let (w, h) = self.dims();
        // Radius of a crystal-free disc around the walker: outside the cluster's bounding
        // circle, or from the distance map (nearest site ≥ `free` away; 2 spacings of
        // slack so rounding the landing point can never put it on the crystal).
        let near = f64::from(self.free.get(x, y).copied().unwrap_or(0)) - 2.0;
        let jump = (d - self.r_launch()).max(near);
        let mut next = None;
        if jump >= JUMP_MIN {
            // First passage out of a crystal-free disc of radius `jump` is uniform on its rim.
            let th = rng.random_range(0.0..TAU);
            let jx = (f64::from(x) + jump * th.cos_det()).round() as i32;
            let jy = (f64::from(y) + jump * th.sin_det()).round() as i32;
            let (fx, fy) = (Self::fold(jx, w - 1), Self::fold(jy, h - 1));
            // A mirrored landing is kept only well clear of the crystal; else a unit step.
            let clear = self.free.get(fx, fy).is_some_and(|&f| f >= FOLD_CLEAR);
            if (fx, fy) == (jx, jy) || clear {
                next = Some((fx, fy));
            }
        }
        let (nx, ny) = next.unwrap_or_else(|| {
            let (dx, dy) = walk_step(rng.random::<f64>());
            (x + dx, y + dy)
        });
        // Walls and crystal reflect: a blocked step leaves the walker in place.
        if self.attach.get(nx, ny).is_some_and(|&a| a < 0.0) {
            self.walkers[i] = (nx, ny);
        }
        let (x, y) = self.walkers[i];
        if self.dist(x, y) > self.r_kill() {
            self.walkers[i] = self.launch(rng);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Grow,
    Hold,
    Melt,
}

#[derive(Debug)]
pub struct FractalGrowth {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    dla: Dla,
    phase: Phase,
    /// Seconds since this crystal was seeded (attach times are on this clock).
    t: f64,
    /// Seconds in the current phase.
    phase_t: f64,
    /// Growth target radius `R(0.85)`, lattice units.
    r_target: f64,
    /// Walker-step budget per simulated second for this size.
    steps_per_sec: f64,
    /// Fractional walker steps carried between updates.
    carry: f64,
    /// Chemical (graph) distance from the seed, computed when melting starts.
    chem: Grid<u16>,
    queue: Vec<(i32, i32)>,
    max_chem: u16,
    /// Melting threshold: sites with `chem > melt_k` are gone.
    melt_k: f64,
    df: Option<f64>,
    df_clock: f64,
    label: String,
    label_row: Option<i32>,
    /// Chars the readout may take on its row, right of the title (GEN-R13).
    room: usize,
    /// The crystal rasterised for the current state, sized in `resize` and rebuilt by
    /// [`FractalGrowth::rasterise`] after every state change; `render` only blits it.
    dots: Dots,
    /// Display dot of lattice site (0, 0): the capped lattice's window is centred on the
    /// display, (0, 0) while the lattice is the whole dot lattice.
    origin: (i32, i32),
}

/// The state that colours the crystal: phase, clock and melting threshold.
#[derive(Debug, Clone, Copy)]
struct Tint {
    phase: Phase,
    t: f64,
    melt_k: f64,
}

impl Tint {
    /// Ink for a terminal cell from its youngest dot's age, the melting front, and the seed.
    fn cell_ink(self, youngest: f32, max_chem: u16, has_seed: bool) -> Ink {
        if self.phase == Phase::Melt {
            let front = (f64::from(max_chem) - (self.melt_k - MELT_BAND)) / MELT_BAND;
            if front > 0.0 {
                let level = 60.0 + 120.0 * (1.0 - front).clamp(0.0, 1.0);
                return Ink::new(Role::Dim, level as u8);
            }
        }
        let age = (self.t - f64::from(youngest)).max(0.0);
        if age < TIP_AGE {
            Ink::new(Role::Hot, 255)
        } else if has_seed || age < WARM_AGE {
            let f = ((age - TIP_AGE) / (WARM_AGE - TIP_AGE)).min(1.0);
            Ink::new(Role::Accent, (250.0 - 70.0 * f) as u8)
        } else {
            let level = 110.0 + 135.0 * (-(age - WARM_AGE) / COOL_TAU).exp();
            Ink::new(Role::Cold, level as u8)
        }
    }
}

impl Default for FractalGrowth {
    fn default() -> Self {
        Self {
            dirty: false,
            dla: Dla::default(),
            phase: Phase::Grow,
            t: 0.0,
            phase_t: 0.0,
            r_target: 0.0,
            steps_per_sec: STEPS_MIN,
            carry: 0.0,
            chem: Grid::default(),
            queue: Vec::new(),
            max_chem: 0,
            melt_k: 0.0,
            df: None,
            df_clock: 0.0,
            label: String::with_capacity(96),
            label_row: None,
            room: 0,
            dots: Dots::default(),
            origin: (0, 0),
        }
    }
}

impl FractalGrowth {
    fn reseed(&mut self, ctx: &mut Ctx) {
        let p = ctx.rng.random_range(P_STICK.0..=P_STICK.1);
        // The lattice centre is the terminal centre: the crystal sits mid-screen.
        let size = self.chem.size();
        let seed = (i32::from(size.width()) / 2, i32::from(size.height()) / 2);
        self.dla.reset(seed, p, &mut ctx.rng);
        self.phase = Phase::Grow;
        self.t = 0.0;
        self.phase_t = 0.0;
        self.carry = 0.0;
        self.df = None;
        self.df_clock = 0.0;
        self.refresh_label();
    }

    fn measure_df(&mut self) {
        let d = &self.dla;
        let dists = d.cells.iter().map(|&(x, y)| d.dist(x, y));
        self.df = mass_radius_dimension(dists, FIT_LO, FIT_HI_FRAC * d.r_max).or(self.df);
    }

    /// Readout, e.g. `p_s 0.62   N 812   D_f 1.71` (the title names the process). Parts
    /// that would reach the title are dropped from the left, whole, so the measured
    /// dimension goes last (GEN-R13); in-place edits of a pre-sized string, no allocation.
    fn refresh_label(&mut self) {
        const SEP: &str = "   ";
        self.label.clear();
        let d = &self.dla;
        let _ = write!(self.label, "p_s {:.2}{SEP}N {}", d.p_stick, d.cells.len());
        if let Some(df) = self.df {
            let _ = write!(self.label, "{SEP}D_f {df:.2}");
        }
        while self.label.len() > self.room {
            match self.label.find(SEP) {
                Some(i) => self.label.replace_range(..i + SEP.len(), ""),
                None => self.label.clear(),
            }
        }
    }

    /// Chemical distance of every crystal site from the seed (8-neighbour BFS). Removing
    /// all sites beyond a threshold always leaves a set connected to the seed (CRY-A1),
    /// and it melts every tip first — outside-in along the branches.
    fn start_melt(&mut self) {
        self.chem.fill(u16::MAX);
        self.queue.clear();
        self.max_chem = 0;
        let seed = self.dla.seed;
        if self.dla.occupied(seed.0, seed.1) && self.chem.set(seed.0, seed.1, 0) {
            self.queue.push(seed);
        }
        let mut head = 0;
        while let Some(&(x, y)) = self.queue.get(head) {
            head += 1;
            let c = self.chem.get(x, y).copied().unwrap_or(0);
            self.max_chem = self.max_chem.max(c);
            for (nx, ny) in self.chem.neighbors(x, y, Hood::Moore, Edges::Clip) {
                if self.dla.occupied(nx, ny) && self.chem.get(nx, ny) == Some(&u16::MAX) {
                    self.chem.set(nx, ny, c.saturating_add(1));
                    self.queue.push((nx, ny));
                }
            }
        }
        self.melt_k = f64::from(self.max_chem);
        self.phase = Phase::Melt;
        self.phase_t = 0.0;
    }

    fn melt_to(&mut self, k: f64) {
        self.melt_k = k;
        let (chem, attach) = (&self.chem, &mut self.dla.attach);
        self.dla.cells.retain(|&(x, y)| {
            let keep = chem.get(x, y).is_some_and(|&c| f64::from(c) <= k);
            if !keep {
                attach.set(x, y, EMPTY);
            }
            keep
        });
    }

    /// Rebuild the crystal raster from the current state (no allocation: the lattice
    /// was sized in `resize`).
    fn rasterise(&mut self) {
        let tint = Tint {
            phase: self.phase,
            t: self.t,
            melt_k: self.melt_k,
        };
        let (dots, d, chem_map) = (&mut self.dots, &self.dla, &self.chem);
        dots.clear();
        // Only the terminal cells that can hold crystal: the seed's r_max box, in display
        // dots (lattice site (x, y) is display dot (ox + x, oy + y)).
        let (ox, oy) = self.origin;
        let r = d.r_max.ceil() as i32 + 1;
        let (lw, lh) = d.dims();
        let (x0, x1) = ((d.seed.0 - r).max(0) + ox, (d.seed.0 + r).min(lw - 1) + ox);
        let (y0, y1) = ((d.seed.1 - r).max(0) + oy, (d.seed.1 + r).min(lh - 1) + oy);
        for cy in y0 / SUB_Y..=y1 / SUB_Y {
            for cx in x0 / SUB_X..=x1 / SUB_X {
                // A cell's colour comes from all its crystal sites together (youngest
                // age, melting front, seed), not from its single brightest dot, so
                // collect the sites first and light them with one shared ink.
                let mut sites = [(0, 0); (SUB_X * SUB_Y) as usize];
                let mut n = 0;
                let (mut youngest, mut chem, mut has_seed) = (EMPTY, 0u16, false);
                for dy in 0..SUB_Y {
                    for dx in 0..SUB_X {
                        let (x, y) = (cx * SUB_X + dx, cy * SUB_Y + dy);
                        let (sx, sy) = (x - ox, y - oy);
                        if let Some(&a) = d.attach.get(sx, sy)
                            && a >= 0.0
                        {
                            sites[n] = (x, y);
                            n += 1;
                            youngest = youngest.max(a);
                            chem = chem.max(chem_map.get(sx, sy).copied().unwrap_or(0));
                            has_seed |= (sx, sy) == d.seed;
                        }
                    }
                }
                if n > 0 {
                    let ink = tint.cell_ink(youngest, chem, has_seed);
                    for &(x, y) in &sites[..n] {
                        dots.set(x, y, ink);
                    }
                }
            }
        }
    }
}

impl Generator for FractalGrowth {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        // One site per braille dot, in a centred window of at most MAX_NODES dots (the
        // whole dot lattice up to ~230×62 cells; ADR-0012). A resize reseeds.
        self.dots.resize(size);
        let dot = self.dots.dot_size();
        let lattice = grid::lattice_for(dot, MAX_NODES, 0.0);
        self.origin = (
            (i32::from(dot.width()) - i32::from(lattice.width())) / 2,
            (i32::from(dot.height()) - i32::from(lattice.height())) / 2,
        );
        self.dla = Dla::new(lattice, WALKERS);
        self.chem = Grid::new(lattice, u16::MAX);
        self.queue = Vec::with_capacity(lattice.area());
        // R(0.85) in rows; a row is SUB_Y lattice spacings. A capped window holds the
        // crystal at R(0.85) of its own half-size.
        self.r_target = draw::fit_radius(size, GROW_FRAC) * f64::from(SUB_Y);
        if lattice != dot {
            let window = f64::from(lattice.width().min(lattice.height())) / 2.0;
            self.r_target = self.r_target.min(GROW_FRAC * window);
        }
        let r_rows = self.r_target / f64::from(SUB_Y);
        self.steps_per_sec =
            (STEPS_REF * (r_rows / R_REF_ROWS).powf(COST_EXP)).clamp(STEPS_MIN, STEPS_MAX);
        self.label_row = (size.height() >= 12).then(|| i32::from(size.height()) - 1);
        self.room = title::readout_room(size, &Title::of(self));
        self.reseed(ctx);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.t += dt;
        self.phase_t += dt;
        match self.phase {
            Phase::Grow => {
                self.carry += self.steps_per_sec * dt;
                let steps = self.carry.floor();
                self.carry -= steps;
                self.dla.advance(steps as usize, self.t, &mut ctx.rng);
                self.df_clock += dt;
                if self.df_clock >= DF_TICK {
                    self.df_clock = 0.0;
                    self.measure_df();
                }
                let full = self.dla.cells.len() * 4 >= self.dla.attach.len();
                if self.dla.r_max >= self.r_target || self.phase_t >= MAX_GROW || full {
                    self.measure_df();
                    self.phase = Phase::Hold;
                    self.phase_t = 0.0;
                }
                self.refresh_label();
            }
            Phase::Hold => {
                if self.phase_t >= HOLD {
                    self.start_melt();
                }
            }
            Phase::Melt => {
                if self.phase_t >= MELT {
                    self.reseed(ctx);
                } else {
                    let f = self.phase_t / MELT;
                    self.melt_to(f64::from(self.max_chem) * (1.0 - f));
                }
            }
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
        if self.phase == Phase::Grow {
            // The vapour is sparse: each walker is a lone dot that replaces its cell
            // (rather than merging into a `Dots` layer); the crystal then draws over it.
            for &(x, y) in &self.dla.walkers {
                let glyph = raster::braille(raster::dot_bit(x, y));
                canvas.put(x / SUB_X, y / SUB_Y, glyph, Ink::new(Role::Dim, 110));
            }
        }
        self.dots.blit(canvas);
        // Last, so no walker or crystal dot lands inside the text.
        if let Some(row) = self.label_row
            && !self.label.is_empty()
        {
            draw::label_right(
                canvas,
                row,
                &self.label,
                Ink::new(Role::Text, title::READOUT_LEVEL),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(seed: u64) -> Ctx {
        Ctx::new(seed, Size::new(1, 1))
    }

    /// A lattice-centred seed for `Dla`-level tests.
    fn centre(d: &Dla) -> (i32, i32) {
        let (w, h) = d.dims();
        (w / 2, h / 2)
    }

    /// Flood fill over 8-neighbours from the seed reaches every crystal site (CRY-A1).
    fn connected_to_seed(d: &Dla) -> bool {
        if d.cells.is_empty() {
            return true;
        }
        let mut seen = vec![false; d.attach.len()];
        let w = i32::from(d.attach.width());
        let mut stack = vec![d.seed];
        seen[(d.seed.1 * w + d.seed.0) as usize] = true;
        let mut n = 0;
        while let Some((x, y)) = stack.pop() {
            n += 1;
            for (nx, ny) in d.attach.neighbors(x, y, Hood::Moore, Edges::Clip) {
                let i = (ny * w + nx) as usize;
                if !seen[i] && d.attach.as_slice()[i] >= 0.0 {
                    seen[i] = true;
                    stack.push((nx, ny));
                }
            }
        }
        n == d.cells.len()
    }

    #[test]
    fn cry_braille_lattice_is_square_in_physical_units() {
        // A dot is 1/SUB_X of a cell wide and 1/SUB_Y of a row tall; cells are ASPECT:1.
        let dot_w = 1.0 / (sciart_core::geom::ASPECT * f64::from(SUB_X));
        let dot_h = 1.0 / f64::from(SUB_Y);
        assert!((dot_w - dot_h).abs() < 1e-12);
        // The DLA lattice is the dot lattice of the screen.
        let size = Size::new(80, 24);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut Ctx::new(1, size));
        assert_eq!(g.dla.attach.size(), Size::new(160, 96));
    }

    #[test]
    fn cry_walk_is_isotropic_and_unbiased() {
        // Exact quadrature over u: equal diffusivity per axis, zero drift.
        let (mut ex, mut ey, mut mx, mut my) = (0.0, 0.0, 0.0, 0.0);
        let n = 1000;
        for k in 0..n {
            let (dx, dy) = walk_step((f64::from(k) + 0.5) / f64::from(n));
            let (px, py) = (f64::from(dx), f64::from(dy));
            ex += px * px;
            ey += py * py;
            mx += px;
            my += py;
        }
        assert!((ex - ey).abs() < 1e-9, "{ex} vs {ey}");
        assert!(mx.abs() < 1e-9 && my.abs() < 1e-9);
        // Robust at the ends of [0, 1).
        assert_eq!(walk_step(0.0), (-1, 0));
        assert_eq!(walk_step(1.0 - f64::EPSILON), (0, 1));
    }

    #[test]
    fn cry_mass_radius_estimator_recovers_line_and_disc() {
        // Analytic anchors: a line has D = 1, a filled disc D = 2. Lattice rounding at
        // r ≥ 3 moves the fitted slope by < 0.05.
        let line = (0..400).map(|i| f64::from(i) * 0.5);
        let d1 = mass_radius_dimension(line, 3.0, 60.0).unwrap();
        assert!((d1 - 1.0).abs() < 0.05, "{d1}");
        let disc =
            (-80..=80).flat_map(|x| (-80..=80).map(move |y| f64::from(x).hypot(f64::from(y))));
        let d2 = mass_radius_dimension(disc, 3.0, 60.0).unwrap();
        assert!((d2 - 2.0).abs() < 0.05, "{d2}");
        assert_eq!(mass_radius_dimension([1.0].into_iter(), 3.0, 4.0), None);
    }

    #[test]
    fn cry_a1_a2_growth_only_adds_and_stays_8_connected() {
        let mut ctx = seeded(3);
        let mut d = Dla::new(Size::new(120, 100), 8);
        d.reset(centre(&d), 0.5, &mut ctx.rng);
        let mut t = 0.0;
        let mut prev: Vec<f32> = d.attach.as_slice().to_vec();
        for _ in 0..120 {
            t += SIM_DT;
            d.advance(2000, t, &mut ctx.rng);
            for (a, b) in prev.iter().zip(d.attach.as_slice()) {
                if *a >= 0.0 {
                    assert_eq!(a, b, "attach time changed");
                }
            }
            assert!(d.cells.len() >= prev.iter().filter(|a| **a >= 0.0).count());
            assert_eq!(
                d.cells.len(),
                d.attach.as_slice().iter().filter(|a| **a >= 0.0).count()
            );
            assert!(connected_to_seed(&d));
            prev.copy_from_slice(d.attach.as_slice());
        }
        assert!(d.cells.len() > 100, "grew only {}", d.cells.len());
    }

    #[test]
    fn cry_a3_walkers_stay_in_kill_radius_and_grid() {
        let mut ctx = seeded(4);
        for (w, h) in [
            (120u16, 100u16),
            (400, 200),
            (3, 3),
            (1, 1),
            (2, 60),
            (80, 1),
        ] {
            let mut d = Dla::new(Size::new(w, h), 8);
            d.reset(centre(&d), 1.0, &mut ctx.rng);
            for k in 0..150 {
                d.advance(400, f64::from(k) * SIM_DT, &mut ctx.rng);
                for &(x, y) in &d.walkers {
                    assert!(d.attach.get(x, y).is_some(), "{w}x{h}: off grid");
                    assert!(d.dist(x, y) <= d.r_kill() + 1e-9, "{w}x{h}: beyond kill");
                }
            }
        }
    }

    /// Grow one seeded cluster to `r_max ≥ R` on a square lattice and fit its mass–radius
    /// dimension over `[3, R/2]`: above the lattice scale, inside the screened core.
    fn grown_dimension(seed: u64, p_stick: f64, r: f64) -> f64 {
        let mut ctx = seeded(seed);
        let side = (2.6 * r) as u16;
        let mut d = Dla::new(Size::new(side, side), 1);
        d.reset(centre(&d), p_stick, &mut ctx.rng);
        let mut guard = 0;
        while d.r_max < r && guard < 20_000 {
            d.advance(1000, 0.0, &mut ctx.rng);
            guard += 1;
        }
        assert!(d.r_max >= r, "cluster did not reach r = {r}");
        let dists = d.cells.iter().map(|&(x, y)| d.dist(x, y));
        mass_radius_dimension(dists, 3.0, 0.5 * d.r_max).unwrap()
    }

    #[test]
    fn cry_a4_grown_cluster_is_fractal() {
        // One seed-centred cluster of r_max = 60 scatters by sd ≈ 0.08 in D_f (40-seed
        // survey at p_s = 1: mean 1.635, the usual finite-size underestimate of the
        // asymptotic 1.71), so assert on a small seeded ensemble (s.e. ≈ 0.04).
        for p in [1.0, 0.3] {
            let seeds = 0..4u64;
            let n = seeds.clone().count() as f64;
            let mean = seeds.map(|s| grown_dimension(s, p, 60.0)).sum::<f64>() / n;
            assert!((1.45..=1.95).contains(&mean), "p_s={p}: D_f = {mean}");
            if p == 1.0 {
                // Witten–Sander 2-D DLA: D_f ≈ 1.71; allow the −0.08 bias plus 3 s.e.
                assert!((mean - 1.71).abs() < 0.2, "D_f = {mean}");
            }
        }
    }

    #[test]
    fn cry_cycle_grows_holds_melts_and_reseeds() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut ctx);
        let (mut shrank, mut reseeded, mut held) = (false, false, false);
        let mut prev = g.dla.cells.len();
        for _ in 0..(70.0 / SIM_DT) as usize {
            let before = g.phase;
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if g.phase != Phase::Grow || g.dla.cells.len() != prev {
                assert!(connected_to_seed(&g.dla), "{:?}", g.phase);
            }
            held |= g.phase == Phase::Hold;
            shrank |= g.phase == Phase::Melt && g.dla.cells.len() < prev;
            reseeded |= before == Phase::Melt && g.phase == Phase::Grow;
            prev = g.dla.cells.len();
        }
        assert!(held && shrank && reseeded, "{held} {shrank} {reseeded}");
    }

    #[test]
    fn cry_gen_r13_readout_drops_whole_parts_from_the_left() {
        let mut ctx = seeded(1);
        let mut g = FractalGrowth::default();
        // The room is w − 6 minus the title text (gutter, 4-cell gap, trailing blank):
        // the full title (30 cells) at 80 wide, the phenomenon (14) at 40; at most half
        // the row, so 44 becomes 40 at 80 wide (CORE-R76).
        for (size, room) in [(Size::new(80, 24), 40), (Size::new(40, 12), 20)] {
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            assert_eq!(g.room, room, "{size:?}");
        }
        g.dla.p_stick = 0.62;
        g.df = Some(1.71);
        for (room, want) in [
            (80, "p_s 0.62   N 1   D_f 1.71"),
            (25, "p_s 0.62   N 1   D_f 1.71"),
            (24, "N 1   D_f 1.71"),
            (13, "D_f 1.71"),
            (8, "D_f 1.71"),
            (7, ""),
        ] {
            g.room = room;
            g.refresh_label();
            assert_eq!(g.label, want, "room {room}");
        }
        // Drawn right-aligned, a blank cell either side, over any walker under it.
        g.room = 22;
        g.refresh_label();
        g.phase = Phase::Grow;
        g.dla.walkers[0] = (30 * SUB_X, 11 * SUB_Y);
        let mut canvas = Canvas::new(Size::new(40, 12), GlyphMode::Ascii);
        g.render(&mut canvas);
        assert!(canvas.to_lines()[11].ends_with(" N 1   D_f 1.71 "));
    }

    #[test]
    fn cry_renders_hot_tips_cold_body_and_ascii_fallbacks() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        let mut roles = std::collections::HashSet::new();
        for _ in 0..(12.0 / SIM_DT) as usize {
            stage.step(SIM_DT);
            let frame = stage.render();
            assert!(frame.to_lines().iter().all(|l| l.is_ascii()));
            roles.extend(frame.cells().iter().map(|c| c.ink.role));
        }
        for r in [Role::Hot, Role::Cold, Role::Accent, Role::Dim, Role::Text] {
            assert!(roles.contains(&r), "{r:?} missing");
        }
    }

    #[test]
    fn test_r7_lattice_is_the_dot_lattice_up_to_the_cap_then_a_centred_window() {
        for (w, h) in [(80, 24), (200, 50), (228, 62), (240, 70), (500, 150)] {
            let size = Size::new(w, h);
            let mut g = FractalGrowth::default();
            g.resize(size, &mut Ctx::new(1, size));
            let (dot, lattice) = (g.dots.dot_size(), g.chem.size());
            assert!(lattice.area() <= MAX_NODES, "{w}x{h}: {lattice:?}");
            if dot.area() <= MAX_NODES {
                // As at M11: one site per display dot, so these frames are unchanged.
                assert_eq!((lattice, g.origin), (dot, (0, 0)), "{w}x{h}");
            } else {
                let (ox, oy) = g.origin;
                let (lw, lh) = (i32::from(lattice.width()), i32::from(lattice.height()));
                let (dw, dh) = (i32::from(dot.width()), i32::from(dot.height()));
                assert!((dw - lw - 2 * ox).abs() <= 1 && (dh - lh - 2 * oy).abs() <= 1);
                let window = f64::from(lattice.width().min(lattice.height())) / 2.0;
                assert!(g.r_target <= GROW_FRAC * window + 1e-9, "{w}x{h}");
            }
        }
    }
}
