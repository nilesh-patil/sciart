//! Diffusion-limited aggregation (Witten–Sander) in two acts that alternate: a frost
//! dendrite growing from a seed, then a metal forest electrodeposited on a cathode strip.
//! Each grows, glows in the colours of its arrival times and holds; then the next act's
//! seed starts growing at once while the old aggregate fades out as a ghost behind it,
//! tips first (a staged cross-fade, not modelled dissolution), so the screen is never
//! empty between acts (SPEC-002 `fractal_growth`).
//!
//! Physics: random walkers diffuse in from a launch curve just outside the aggregate and
//! freeze onto it when they touch it (8-neighbourhood) with sticking probability `p_s`. The
//! frost act launches from a circle around a single seed; the forest act from a line above
//! a cathode that covers the bottom row: DLA on a line (Meakin 1983). Diffusion-limited
//! electrodeposits are DLA fractals (Brady & Ball 1984; Matsushita et al. 1984; Grier et
//! al. 1986; Sawada et al. 1986, mostly in radial cells); the cathode strip is their
//! line-geometry version (real cells add ion migration, which is not modelled). Both are
//! the same walk: only the launch curve and the "height" a walker is measured by differ
//! ([`Dla`]).
//!
//! Aspect correction: the aggregate lives on the braille sub-cell lattice ([`Dots`],
//! 2 × 4 dots per terminal cell). With cells ~2:1 tall (`ASPECT`), that lattice is
//! *square* — 1/4 row a side — so walks are isotropic with four equally likely steps
//! ([`walk_step`]), and so are the contact distance and the size of each frozen particle.
//! (On the raw 2:1 cell grid vertical contacts reach twice as far as horizontal ones and
//! grow vertical needles with `D_f ≈ 1.4`.) The fine lattice also gives 4× the linear
//! resolution, which a sparse fractal needs.
//!
//! Efficiency (standard DLA accelerations):
//! - a free walk first leaves a disc uniformly on its rim, so a walker inside a
//!   crystal-free disc jumps straight to a uniform point on its circle (one budget step).
//!   Far out the disc's radius is the walker's height above the launch curve; near the
//!   aggregate it comes from a capped map of the distance to the nearest crystal site,
//!   updated around each new site (the Meakin / Ball–Brady trick);
//! - frost: a walker beyond the kill radius is relaunched on the launch circle (the usual
//!   approximation: `2×` the launch radius);
//! - forest: a walker above the launch line is put straight back on it at its exact
//!   first-passage point, a Cauchy offset of its height ([`line_return`]), so the strip
//!   needs no kill height at all.
//!
//! The lattice walls reflect; a jump that would cross a wall is mirrored back (method of
//! images: exact for the walk's position, an approximation for its first-passage point)
//! and only kept if it lands well clear of the crystal.
//!
//! The crystal's colours follow the growth clock every step, so its `Dots` raster is
//! rebuilt at the end of `update` (and `resize`); `render` only blits it.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::text::StackText;
use sciart_core::title::{self, Title};
use sciart_core::{draw, grid, ink, raster};

pub static META: Meta = Meta {
    name: "fractal_growth",
    category: Category::PhysicalChemistry,
    description: "Diffusion-limited aggregation: a frost dendrite from a seed, then a metal forest on a cathode",
    title: "Fractal growth · walkers stick",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(FractalGrowth::default())
}

/// Readout buffer (bytes): the longest form, `p_s 1.00   N 131072   D_f ~2.0` (N ≤
/// [`MAX_NODES`]), is 33.
const LABEL_CAP: usize = 48;

/// The acts' captions (GEN-R12): walkers stick to a seed, then ions deposit on a cathode.
const CAPTIONS: [&str; 2] = ["walkers stick", "ions deposit"];

/// Braille dots per terminal cell; `SUB_Y / SUB_X = ASPECT` makes the dot lattice square.
const SUB_X: i32 = Dots::CELL_W;
const SUB_Y: i32 = Dots::CELL_H;

/// Most lattice sites (ADR-0012, CRY-A5): 200×50 (80 000 dots) is uncapped; larger
/// terminals grow the crystal in a centred window of about this many dots.
const MAX_NODES: usize = 1 << 17;

/// Attach-time sentinel for an empty site (real attach times are ≥ 0).
const EMPTY: f32 = -1.0;

/// Capacity of the site pool and the melt queue for a `lattice`: growth ends once a
/// quarter of the sites are crystal (`FractalGrowth::update`), so this never fills in a
/// normal cycle; `Dla::step_walker` refuses to stick beyond it.
fn pool(lattice: Size) -> usize {
    lattice.area() / 4 + 1
}
/// Walker-step budget per simulated second of the frost act at 200×50. Measured cost grows
/// ~R^2.5 with the crystal radius R, so the budget scales as `(R / R_REF)^2.5` for a
/// comparable growth time at any size, within `[STEPS_MIN, STEPS_MAX]`: with the opening
/// boost ([`EARLY_BOOST`]) the frost grows in 6–7 s at 200×50 (5.6–8 s at 80×24).
const STEPS_REF: f64 = 8_000.0;
const R_REF_ROWS: f64 = 21.25;
const COST_EXP: f64 = 2.5;
const STEPS_MIN: f64 = 600.0;
const STEPS_MAX: f64 = 240_000.0;
/// The forest act's budget per lattice column and second: a deposit from a line costs
/// about the same per unit of width at any size (CRY-A6).
const FOREST_STEPS_PER_COL: f64 = 40.0;
/// The frost opens `EARLY_BOOST` times faster, easing to the plain budget over its first
/// `EARLY` s: a small aggregate is cheap to grow, so the act starts from a visible crystal
/// instead of a lone seed. The forest opens `FOREST_BOOST` times faster, easing over its
/// first `FOREST_EARLY` s, so its first trees rise while the frost's ghost fades (CRY-A9).
const EARLY_BOOST: f64 = 4.0;
const EARLY: f64 = 2.0;
const FOREST_BOOST: f64 = 3.0;
const FOREST_EARLY: f64 = 3.0;
/// Concurrent walkers: a tiny fraction of the cluster, so growth is effectively
/// one-particle-at-a-time (Witten–Sander), yet enough to draw a faint "vapour".
const WALKERS: usize = 12;
/// Launch curve = `extent + LAUNCH_GAP` above the aggregate, lattice units ("just outside").
const LAUNCH_GAP: f64 = 2.0;
/// Frost kill radius = `KILL_FACTOR × r_launch` (spec: 2×).
const KILL_FACTOR: f64 = 2.0;
/// Shortest jump worth taking instead of a unit step, lattice units.
const JUMP_MIN: f64 = 1.5;
/// Cap of the nearest-crystal distance map, lattice units (bigger = more update work per
/// new site, longer jumps near the crystal).
const FREE_CAP: u8 = 10;
/// A mirrored (wall-crossing) jump must land at least this far from any crystal.
const FOLD_CLEAR: u8 = 3;
/// A walker within this distance of the extent may touch the crystal (diagonal contact √2).
const TOUCH_REACH: f64 = 1.5;
/// Growth stops at `R(GROW_FRAC)` (the forest: this fraction of the lattice height) or
/// after `MAX_GROW` s; then it holds for `HOLD` s, and the next act starts while this one
/// fades out as a ghost over `FADE` s (CRY-A9).
const GROW_FRAC: f64 = 0.85;
const FOREST_FRAC: f64 = 0.8;
const MAX_GROW: f64 = 40.0;
const HOLD: f64 = 5.0;
const FADE: f64 = 3.0;
/// Sticking probability range (lower = denser, fuzzier crystals).
const P_STICK: (f64, f64) = (0.3, 1.0);
/// Live mass–radius fit, lattice units: from a few spacings up to a fraction of `r_max`
/// (the outer shell is still growing, so it is not yet at its asymptotic density).
const FIT_LO: f64 = 3.0;
const FIT_HI_FRAC: f64 = 0.5;
/// The frost act shows its live `D_f` estimate only from this many sites (CRY-A7): below
/// it the fit is noise. At `p_s = 1` a grown cluster (N ≈ 1 500) fits 1.45–1.65, the
/// finite-size underestimate of the asymptotic 1.71; low `p_s` packs the small scales
/// denser and fits higher. Hence an estimate with one decimal, `D_f ~1.6`.
const DF_MIN_N: usize = 600;
/// How often the live D_f readout is refreshed, s.
const DF_TICK: f64 = 0.5;
/// The ghost (CRY-A9): a site's strength starts at `1 − GHOST_SPREAD·chem/max_chem` (tips,
/// the chemically farthest sites, weakest), decays as `exp(−t/τ)` and the site is gone below
/// `GHOST_CUT`; τ is set so the roots, at strength 1, go exactly at `FADE`. The ghost is
/// drawn in the old act's ramp from `GHOST_LO` (at the cut) up to `GHOST_HI` (fresh),
/// below the live crystal's core, so it reads as an after-image behind the new growth.
const GHOST_SPREAD: f32 = 0.65;
const GHOST_CUT: f32 = 0.08;
const GHOST_LO: f64 = 0.22;
const GHOST_HI: f64 = 0.45;

// Colour (CRY-A7): a site's ramp position is its arrival time over the growth clock, so
// the core is deep and the newest tips pale; a site younger than TIP_AGE flashes at the
// top of the ramp.
const TIP_AGE: f64 = 0.3;
const RAMP_LO: f64 = 0.5;
const RAMP_HI: f64 = 0.92;
const WALKER_INK: Ink = Ink::new(Role::Dim, 110);
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

/// Mirror `v` into `0..=max` (reflecting walls at 0 and `max`, method of images): the
/// unfolded axis has period `2·max`, so this is exact for any `v`. NaN maps to 0.
fn fold(v: f64, max: i32) -> i32 {
    if max <= 0 {
        return 0;
    }
    let period = 2.0 * f64::from(max);
    let u = v.round().rem_euclid(period);
    (if u > f64::from(max) { period - u } else { u }) as i32
}

/// Where a forest walker at column `x`, height `a > 0` above the launch line, first comes
/// back down onto that line (CRY-A6): `x + a·tan(π(u − ½))` for a uniform `u`, the Cauchy
/// (half-plane Poisson kernel) law of scale `a`, folded into `0..=max` by the reflecting
/// walls. Exact for a free walk above the line, so nothing is killed.
pub(crate) fn line_return(x: i32, a: f64, u: f64, max: i32) -> i32 {
    let offset = a * (std::f64::consts::PI * (u - 0.5)).tan();
    fold(f64::from(x) + offset, max)
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

/// The two acts, alternating: DLA onto a seed, then onto a cathode strip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Act {
    #[default]
    Frost,
    Forest,
}

impl Act {
    /// The act's named ramp (false colour, ADR-0015): frost in Ice, the metal in
    /// Starlight, a neutral grey to pale white, like freshly deposited silver (not the
    /// gallery's overused amber).
    fn ramp(self) -> Ramp {
        match self {
            Act::Frost => Ramp::Ice,
            Act::Forest => Ramp::Starlight,
        }
    }

    fn next(self) -> Act {
        match self {
            Act::Frost => Act::Forest,
            Act::Forest => Act::Frost,
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Act::Frost => CAPTIONS[0],
            Act::Forest => CAPTIONS[1],
        }
    }
}

/// The aggregate and its walkers on a square lattice.
#[derive(Debug, Default)]
struct Dla {
    /// The seed (frost) or the middle of the cathode (forest).
    seed: (i32, i32),
    /// Forest geometry: the cathode is the bottom row and walkers launch from a line.
    line: bool,
    /// Attach time per site (run seconds), [`EMPTY`] if free (CRY-A2).
    attach: Grid<f32>,
    /// Occupied sites in attach order: the seed or the cathode first.
    cells: Vec<(i32, i32)>,
    /// How many of `cells` are roots (the seed, or the cathode sites the pool held).
    roots: usize,
    walkers: Vec<(i32, i32)>,
    /// Round-robin cursor, kept across calls so small budgets still move every walker.
    turn: usize,
    /// `floor` of the distance to the nearest crystal site, capped at [`FREE_CAP`].
    free: Grid<u8>,
    /// Largest height ([`Dla::dist`]) of any crystal site, lattice units.
    r_max: f64,
    p_stick: f64,
}

impl Dla {
    /// A `lattice`-sized aggregate with `walkers` walkers; call [`Dla::reset`] to seed it.
    fn new(lattice: Size, walkers: usize) -> Self {
        Self {
            seed: (0, 0),
            line: false,
            attach: Grid::new(lattice, EMPTY),
            cells: Vec::with_capacity(pool(lattice)),
            roots: 0,
            walkers: vec![(0, 0); walkers],
            turn: 0,
            free: Grid::new(lattice, FREE_CAP),
            r_max: 0.0,
            p_stick: 1.0,
        }
    }

    /// A fresh aggregate and fresh walkers: a single seed at `seed` (clamped onto the
    /// lattice), or with `line` a cathode covering the whole bottom row.
    fn reset(&mut self, seed: (i32, i32), line: bool, p_stick: f64, rng: &mut impl Rng) {
        let (w, h) = self.dims();
        self.attach.fill(EMPTY);
        self.free.fill(FREE_CAP);
        self.cells.clear();
        self.line = line;
        self.seed = if line {
            (w / 2, (h - 1).max(0))
        } else {
            (
                seed.0.clamp(0, (w - 1).max(0)),
                seed.1.clamp(0, (h - 1).max(0)),
            )
        };
        self.r_max = 0.0;
        self.p_stick = p_stick;
        let (x0, x1) = if line {
            (0, w)
        } else {
            (self.seed.0, self.seed.0 + 1)
        };
        for x in x0..x1 {
            // The pool holds a quarter of the lattice: a whole row unless the lattice is
            // under 4 dots tall, where the cathode is cut short (never a reallocation).
            if self.cells.len() < self.cells.capacity() && self.attach.set(x, self.seed.1, 0.0) {
                self.cells.push((x, self.seed.1));
                self.mark_free((x, self.seed.1));
            }
        }
        self.roots = self.cells.len();
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

    /// How far a site is from where growth starts, lattice units: its distance from the
    /// seed, or (forest) its height above the cathode row.
    #[inline]
    fn dist(&self, x: i32, y: i32) -> f64 {
        if self.line {
            f64::from(self.seed.1 - y)
        } else {
            f64::from(x - self.seed.0).hypot(f64::from(y - self.seed.1))
        }
    }

    fn r_launch(&self) -> f64 {
        self.r_max + LAUNCH_GAP
    }

    /// Farthest a walker may be: the frost's kill radius, or (forest) the launch line
    /// itself, since a walker above it is returned onto it ([`line_return`]).
    fn r_kill(&self) -> f64 {
        if self.line {
            self.r_launch()
        } else {
            KILL_FACTOR * self.r_launch()
        }
    }

    /// The forest's launch row: the highest row at or below the launch line.
    fn line_row(&self) -> i32 {
        (f64::from(self.seed.1) - self.r_launch()).ceil() as i32
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

    /// Uniform point on the launch curve, clamped onto the lattice: the circle around the
    /// seed, or the line above the cathode. Clamping only moves a point toward the start,
    /// so it stays inside the kill height (CRY-A3).
    fn launch(&self, rng: &mut impl Rng) -> (i32, i32) {
        let r = self.r_launch();
        let (w, h) = self.dims();
        let (x, y) = if self.line {
            (rng.random_range(0..w.max(1)), self.line_row())
        } else {
            let th = rng.random_range(0.0..TAU);
            (
                (f64::from(self.seed.0) + r * th.cos_det()).round() as i32,
                (f64::from(self.seed.1) + r * th.sin_det()).round() as i32,
            )
        };
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
        // Never stick to a full pool (growth is over by then, see `pool`): no reallocation.
        let room = self.cells.len() < self.cells.capacity();
        if room
            && d <= self.r_max + TOUCH_REACH
            && self.touches(x, y)
            && rng.random::<f64>() < self.p_stick
        {
            self.attach.set(x, y, t as f32);
            self.cells.push((x, y));
            self.mark_free((x, y));
            self.r_max = self.r_max.max(d);
            self.walkers[i] = self.launch(rng);
            return;
        }

        let (w, h) = self.dims();
        // Radius of a crystal-free disc around the walker: its height above the launch
        // curve, or from the distance map (nearest site ≥ `free` away; 2 spacings of
        // slack so rounding the landing point can never put it on the crystal).
        let near = f64::from(self.free.get(x, y).copied().unwrap_or(0)) - 2.0;
        let jump = (d - self.r_launch()).max(near);
        let mut next = None;
        if jump >= JUMP_MIN {
            // First passage out of a crystal-free disc of radius `jump` is uniform on its rim.
            let th = rng.random_range(0.0..TAU);
            let (jx, jy) = (
                f64::from(x) + jump * th.cos_det(),
                f64::from(y) + jump * th.sin_det(),
            );
            let (fx, fy) = (fold(jx, w - 1), fold(jy, h - 1));
            // A mirrored landing is kept only well clear of the crystal; else a unit step.
            let mirrored = (f64::from(fx), f64::from(fy)) != (jx.round(), jy.round());
            if !mirrored || self.free.get(fx, fy).is_some_and(|&f| f >= FOLD_CLEAR) {
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
        if self.line {
            let row = self.line_row();
            // Above the launch line (so `row ≥ 1`, on the lattice): back onto it, exactly.
            if y < row {
                let a = f64::from(row - y);
                self.walkers[i] = (line_return(x, a, rng.random(), w - 1), row);
            }
        } else if self.dist(x, y) > self.r_kill() {
            self.walkers[i] = self.launch(rng);
        }
    }

    /// Whether site `(x, y)` is where growth starts: the seed, or a cathode site.
    fn is_root(&self, x: i32, y: i32) -> bool {
        if self.line {
            y == self.seed.1
        } else {
            (x, y) == self.seed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Grow,
    Hold,
}

#[derive(Debug)]
pub struct FractalGrowth {
    dla: Dla,
    act: Act,
    phase: Phase,
    /// Seconds since this aggregate was seeded (attach times are on this clock).
    t: f64,
    /// The growth clock the colours are normalised by: `t` while growing, then frozen.
    t_grown: f64,
    /// Seconds in the current phase.
    phase_t: f64,
    /// Frost growth target `R(0.85)` and forest target height, lattice units.
    r_target: f64,
    h_target: f64,
    /// Walker-step budgets per simulated second for this size.
    steps_frost: f64,
    steps_forest: f64,
    /// Fractional walker steps carried between updates.
    carry: f64,
    /// Chemical (graph) distance from the seed or cathode, computed at the hand-over.
    chem: Grid<u16>,
    queue: Vec<(i32, i32)>,
    max_chem: u16,
    /// The previous act's aggregate fading out (CRY-A9): display dot and initial strength
    /// per site, capacity fixed in `resize`; its ramp and the seconds since the hand-over.
    ghost: Vec<(i32, i32, f32)>,
    ghost_ramp: Ramp,
    ghost_t: f64,
    df: Option<f64>,
    df_clock: f64,
    label: StackText<LABEL_CAP>,
    label_row: Option<i32>,
    /// Chars the readout may take on its row, right of the title (GEN-R13).
    room: [usize; 2],
    /// The crystal rasterised for the current state, sized in `resize` and rebuilt by
    /// [`FractalGrowth::rasterise`] after every state change; `render` only blits it.
    dots: Dots,
    /// The lattice's top-left dot in the display (CRY-A5): the capped lattice is centred,
    /// except that the forest's sits on the bottom of the art area, its cathode's place.
    origin: (i32, i32),
}

/// The ghost's decay time: a root (strength 1) reaches [`GHOST_CUT`] exactly at [`FADE`].
fn ghost_tau() -> f64 {
    FADE / (1.0 / f64::from(GHOST_CUT)).ln()
}

/// A ghost site's strength `t` s after the hand-over, or `None` once it is gone (CRY-A9).
fn ghost_strength(v0: f32, t: f64) -> Option<f32> {
    let v = v0 * (-t / ghost_tau()).exp() as f32;
    (v >= GHOST_CUT).then_some(v)
}

/// The ghost's ink at strength `v`: the old act's ramp from [`GHOST_LO`] at the cut to
/// [`GHOST_HI`] at full strength.
fn ghost_ink(ramp: Ramp, v: f32) -> Ink {
    let u = (f64::from(v - GHOST_CUT) / f64::from(1.0 - GHOST_CUT)).clamp(0.0, 1.0);
    ink::ramp(ramp, GHOST_LO + (GHOST_HI - GHOST_LO) * u)
}

/// The state that colours the crystal: act and clocks.
#[derive(Debug, Clone, Copy)]
struct Tint {
    ramp: Ramp,
    t: f64,
    t_grown: f64,
}

impl Tint {
    /// Ink for a terminal cell from its youngest dot's arrival time (CRY-A7): the ramp
    /// position is `RAMP_LO + (RAMP_HI − RAMP_LO)·arrival/t_grown`; tips younger than
    /// [`TIP_AGE`] go to the top.
    fn cell_ink(self, youngest: f32) -> Ink {
        let arrival = f64::from(youngest);
        let mut v = RAMP_LO + (RAMP_HI - RAMP_LO) * (arrival / self.t_grown.max(1e-9)).min(1.0);
        if self.t - arrival < TIP_AGE {
            v = 1.0;
        }
        ink::ramp(self.ramp, v)
    }
}

impl Default for FractalGrowth {
    fn default() -> Self {
        Self {
            dla: Dla::default(),
            act: Act::Frost,
            phase: Phase::Grow,
            t: 0.0,
            t_grown: 0.0,
            phase_t: 0.0,
            r_target: 0.0,
            h_target: 0.0,
            steps_frost: STEPS_MIN,
            steps_forest: STEPS_MIN,
            carry: 0.0,
            chem: Grid::default(),
            queue: Vec::new(),
            max_chem: 0,
            ghost: Vec::new(),
            ghost_ramp: Ramp::Ice,
            ghost_t: 0.0,
            df: None,
            df_clock: 0.0,
            label: StackText::new(),
            label_row: None,
            room: [0; 2],
            origin: (0, 0),
            dots: Dots::default(),
        }
    }
}

impl FractalGrowth {
    fn reseed(&mut self, ctx: &mut Ctx) {
        let p = ctx.rng.random_range(P_STICK.0..=P_STICK.1);
        // The lattice centre is the art area's centre: the frost sits mid-screen.
        let size = self.chem.size();
        let seed = (i32::from(size.width()) / 2, i32::from(size.height()) / 2);
        // The window is centred, except that the forest's sits on the bottom of the art
        // area: its cathode is the floor of the picture on any terminal (CRY-A5).
        let display = self.dots.dot_size();
        let (mx, my) = (
            i32::from(display.width() - size.width()),
            i32::from(display.height() - size.height()),
        );
        self.origin = (mx / 2, if self.act == Act::Forest { my } else { my / 2 });
        self.dla
            .reset(seed, self.act == Act::Forest, p, &mut ctx.rng);
        self.phase = Phase::Grow;
        self.t = 0.0;
        self.t_grown = 0.0;
        self.phase_t = 0.0;
        self.carry = 0.0;
        self.df = None;
        self.df_clock = 0.0;
        self.refresh_label();
    }

    fn measure_df(&mut self) {
        let d = &self.dla;
        if d.line || d.cells.len() < DF_MIN_N {
            self.df = None;
            return;
        }
        let dists = d.cells.iter().map(|&(x, y)| d.dist(x, y));
        self.df = mass_radius_dimension(dists, FIT_LO, FIT_HI_FRAC * d.r_max).or(self.df);
    }

    /// Readout, e.g. `p_s 0.62   N 812   D_f ~1.6` (the title names the process; the
    /// forest shows no D_f, its mass–height law is another fit). Parts that would reach
    /// the title are dropped from the left, whole, so the measured dimension goes last
    /// (GEN-R13). Stack buffers only, so no allocation by construction (GEN-R11).
    fn refresh_label(&mut self) {
        const SEP: &str = "   ";
        let d = &self.dla;
        // The cathode is not deposit: count only what grew (the frost's seed is N's 1).
        let grown = d.cells.len() - if d.line { d.roots } else { 0 };
        let mut full = StackText::<LABEL_CAP>::new();
        let _ = write!(full, "p_s {:.2}{SEP}N {grown}", d.p_stick);
        if let Some(df) = self.df {
            let _ = write!(full, "{SEP}D_f ~{df:.1}");
        }
        let room = self.room[usize::from(self.act == Act::Forest)];
        let mut text = full.as_str();
        while text.len() > room {
            text = text.find(SEP).map_or("", |i| &text[i + SEP.len()..]);
        }
        self.label.clear();
        let _ = self.label.write_str(text);
    }

    /// Chemical distance of every crystal site from the seed or cathode (8-neighbour BFS).
    /// Every site is reached from a root (CRY-A1), and the tips are the farthest, so the
    /// ghost fades outside-in along the branches.
    fn measure_chem(&mut self) {
        self.chem.fill(u16::MAX);
        self.queue.clear();
        self.max_chem = 0;
        let d = &self.dla;
        for &(x, y) in &d.cells {
            if d.is_root(x, y) && self.chem.set(x, y, 0) {
                self.queue.push((x, y));
            }
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
    }

    /// Hand over to the other act (CRY-A9): the grown aggregate becomes the ghost, each
    /// site at a strength that falls with its chemical distance (tips weakest, so they go
    /// first), and the next act is seeded at once, its readout starting from its own N.
    fn hand_over(&mut self, ctx: &mut Ctx) {
        self.measure_chem();
        self.ghost.clear();
        let (ox, oy) = self.origin;
        let far = f32::from(self.max_chem.max(1));
        for &(x, y) in &self.dla.cells {
            let c = self.chem.get(x, y).copied().unwrap_or(0).min(self.max_chem);
            let v0 = 1.0 - GHOST_SPREAD * f32::from(c) / far;
            if self.ghost.len() < self.ghost.capacity() {
                self.ghost.push((x + ox, y + oy, v0));
            }
        }
        self.ghost_ramp = self.act.ramp();
        self.ghost_t = 0.0;
        self.act = self.act.next();
        self.reseed(ctx);
    }

    /// Rebuild the crystal raster from the current state (no allocation: the lattice
    /// was sized in `resize`): the ghost first, then the live crystal over it.
    fn rasterise(&mut self) {
        let tint = Tint {
            ramp: self.act.ramp(),
            t: self.t,
            t_grown: self.t_grown,
        };
        self.dots.clear();
        for &(x, y, v0) in &self.ghost {
            if let Some(v) = ghost_strength(v0, self.ghost_t) {
                self.dots.set(x, y, ghost_ink(self.ghost_ramp, v));
            }
        }
        // Chemical distances only mean something while melting (`start_melt` fills them).
        let (dots, d) = (&mut self.dots, &self.dla);
        let (ox, oy) = self.origin;
        // Only the terminal cells that can hold crystal: the seed's r_max box (the forest:
        // full width, from the cathode up to its height), in display dots (lattice site
        // (x, y) is display dot (ox + x, oy + y)).
        let r = d.r_max.ceil() as i32 + 1;
        let (lw, lh) = d.dims();
        let (x0, x1, y0) = if d.line {
            (0, lw - 1, d.seed.1 - r)
        } else {
            (d.seed.0 - r, d.seed.0 + r, d.seed.1 - r)
        };
        let (x0, x1) = (x0.max(0) + ox, x1.min(lw - 1) + ox);
        let (y0, y1) = (y0.max(0) + oy, (d.seed.1 + r).min(lh - 1) + oy);
        for cy in y0 / SUB_Y..=y1 / SUB_Y {
            for cx in x0 / SUB_X..=x1 / SUB_X {
                // A cell's colour comes from all its crystal sites together (youngest
                // arrival, melting front), not from its single brightest dot, so collect
                // the sites first and light them with one shared ink.
                let mut sites = [(0, 0); (SUB_X * SUB_Y) as usize];
                let mut n = 0;
                let mut youngest = EMPTY;
                for dy in 0..SUB_Y {
                    for dx in 0..SUB_X {
                        let (x, y) = (cx * SUB_X + dx, cy * SUB_Y + dy);
                        let site = (x - ox, y - oy);
                        if let Some(&a) = d.attach.get(site.0, site.1)
                            && a >= 0.0
                        {
                            sites[n] = (x, y);
                            n += 1;
                            youngest = youngest.max(a);
                        }
                    }
                }
                if n > 0 {
                    let ink = tint.cell_ink(youngest);
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
        // The art keeps off the title row, so the cathode is never under the title.
        self.label_row = (size.height() >= 12).then(|| i32::from(size.height()) - 1);
        let art = Size::new(
            size.width(),
            size.height() - u16::from(self.label_row.is_some()),
        );
        // One site per braille dot, in a window of at most MAX_NODES dots in the art area
        // (ADR-0012), placed by `reseed`. A resize reseeds, so nothing needs resampling.
        self.dots.resize(art);
        let lattice = grid::lattice_for(self.dots.dot_size(), MAX_NODES, 0.0);
        self.dla = Dla::new(lattice, WALKERS);
        self.chem = Grid::new(lattice, u16::MAX);
        self.queue = Vec::with_capacity(pool(lattice));
        self.ghost = Vec::with_capacity(pool(lattice));
        // R(0.85) of the art area in lattice spacings (a row is SUB_Y), within the window.
        let window = f64::from(lattice.width().min(lattice.height())) / 2.0;
        self.r_target =
            (draw::fit_radius(art, GROW_FRAC) * f64::from(SUB_Y)).min(GROW_FRAC * window);
        self.h_target = FOREST_FRAC * f64::from(lattice.height().saturating_sub(1));
        let r_rows = self.r_target / f64::from(SUB_Y);
        self.steps_frost =
            (STEPS_REF * (r_rows / R_REF_ROWS).powf(COST_EXP)).clamp(STEPS_MIN, STEPS_MAX);
        self.steps_forest =
            (FOREST_STEPS_PER_COL * f64::from(lattice.width())).clamp(STEPS_MIN, STEPS_MAX);
        self.room = [Act::Frost, Act::Forest]
            .map(|act| title::readout_room(size, &Title::new(META.title, Some(act.caption()))));
        self.ghost.clear();
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
                self.t_grown = self.t;
                let (budget, target) = match self.act {
                    Act::Frost => {
                        let boost = 1.0 + (EARLY_BOOST - 1.0) * (1.0 - self.t / EARLY).max(0.0);
                        (boost * self.steps_frost, self.r_target)
                    }
                    Act::Forest => {
                        let boost =
                            1.0 + (FOREST_BOOST - 1.0) * (1.0 - self.t / FOREST_EARLY).max(0.0);
                        (boost * self.steps_forest, self.h_target)
                    }
                };
                self.carry += budget * dt;
                let steps = self.carry.floor();
                self.carry -= steps;
                self.dla.advance(steps as usize, self.t, &mut ctx.rng);
                self.df_clock += dt;
                if self.df_clock >= DF_TICK {
                    self.df_clock = 0.0;
                    self.measure_df();
                }
                let full = self.dla.cells.len() * 4 >= self.dla.attach.len();
                if self.dla.r_max >= target || self.phase_t >= MAX_GROW || full {
                    self.measure_df();
                    self.phase = Phase::Hold;
                    self.phase_t = 0.0;
                }
                self.refresh_label();
            }
            Phase::Hold => {
                if self.phase_t >= HOLD {
                    self.hand_over(ctx);
                }
            }
        }
        if !self.ghost.is_empty() {
            self.ghost_t += dt;
            if self.ghost_t >= FADE {
                self.ghost.clear();
            }
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        if self.phase == Phase::Grow {
            // The vapour is sparse: each walker is a lone dot that replaces its cell
            // (rather than merging into a `Dots` layer); the crystal then draws over it.
            for &(x, y) in &self.dla.walkers {
                let (x, y) = (x + self.origin.0, y + self.origin.1);
                let glyph = raster::braille(raster::dot_bit(x, y));
                canvas.put(x / SUB_X, y / SUB_Y, glyph, WALKER_INK);
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
                self.label.as_str(),
                Ink::new(Role::Text, title::READOUT_LEVEL),
            );
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.act.caption())
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

    /// Flood fill over 8-neighbours from the roots (seed or cathode) reaches every crystal
    /// site (CRY-A1).
    fn connected_to_seed(d: &Dla) -> bool {
        if d.cells.is_empty() {
            return true;
        }
        let mut seen = vec![false; d.attach.len()];
        let w = i32::from(d.attach.width());
        let mut stack: Vec<(i32, i32)> = d
            .cells
            .iter()
            .copied()
            .filter(|&(x, y)| d.is_root(x, y))
            .collect();
        for &(x, y) in &stack {
            seen[(y * w + x) as usize] = true;
        }
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
    fn cry_a4_braille_lattice_is_square_in_physical_units() {
        // A dot is 1/SUB_X of a cell wide and 1/SUB_Y of a row tall; cells are ASPECT:1.
        let dot_w = 1.0 / (sciart_core::geom::ASPECT * f64::from(SUB_X));
        let dot_h = 1.0 / f64::from(SUB_Y);
        assert!((dot_w - dot_h).abs() < 1e-12);
        // The DLA lattice is the dot lattice of the art area (all rows but the title's).
        let size = Size::new(80, 24);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut Ctx::new(1, size));
        assert_eq!(g.dla.attach.size(), Size::new(160, 92));
    }

    #[test]
    fn cry_a4_walk_is_isotropic_and_unbiased() {
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
    fn cry_a4_mass_radius_estimator_recovers_line_and_disc() {
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
        for line in [false, true] {
            let mut ctx = seeded(3);
            let mut d = Dla::new(Size::new(120, 100), 8);
            d.reset(centre(&d), line, 0.5, &mut ctx.rng);
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
                assert!(connected_to_seed(&d), "line {line}");
                prev.copy_from_slice(d.attach.as_slice());
            }
            let roots = if line { 120 } else { 1 };
            assert!(d.cells.len() > roots + 100, "grew only {}", d.cells.len());
        }
    }

    #[test]
    fn cry_a5_lattice_is_capped_centred_and_the_forest_sits_on_the_floor() {
        for (w, h) in [
            (1, 1),
            (80, 24),
            (200, 50),
            (250, 70),
            (500, 150),
            (900, 20),
        ] {
            let size = Size::new(w, h);
            let mut g = FractalGrowth::default();
            g.resize(size, &mut Ctx::new(1, size));
            let display = g.dots.dot_size();
            let lattice = g.dla.attach.size();
            assert!(lattice.area() <= MAX_NODES, "{w}x{h}: {lattice:?}");
            if display.area() <= MAX_NODES {
                assert_eq!(lattice, display, "{w}x{h}: uncapped");
            }
            // The frost's window is centred; the forest's is centred across and sits on the
            // bottom of the art area, so its cathode is the picture's floor.
            for act in [Act::Frost, Act::Forest] {
                g.act = act;
                g.reseed(&mut Ctx::new(1, size));
                let (ox, oy) = g.origin;
                let right = i32::from(display.width()) - i32::from(lattice.width()) - ox;
                let bottom = i32::from(display.height()) - i32::from(lattice.height()) - oy;
                let across = right - ox;
                assert!((0..=1).contains(&across), "{w}x{h}: x margins {ox} {right}");
                if act == Act::Forest {
                    assert_eq!(bottom, 0, "{w}x{h}: the forest floats {bottom} dots up");
                } else {
                    let down = bottom - oy;
                    assert!((0..=1).contains(&down), "{w}x{h}: y margins {oy} {bottom}");
                }
            }
        }
    }

    #[test]
    fn cry_a5_both_acts_at_500x150_stay_in_the_window_without_reallocating() {
        let size = Size::new(500, 150);
        let mut ctx = Ctx::new(5, size);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut ctx);
        let (pool_cap, queue_cap) = (g.dla.cells.capacity(), g.queue.capacity());
        let (lw, lh) = g.dla.dims();
        // The window of the live act (the forest's sits lower).
        let window = |g: &FractalGrowth, x: i32, y: i32| {
            let (ox, oy) = g.origin;
            (ox..ox + lw).contains(&x) && (oy..oy + lh).contains(&y)
        };
        let mut acts = [false; 2];
        for _ in 0..(70.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            // An act that has handed over is the ghost behind the next one.
            let ghost_forest = g.ghost_ramp == Act::Forest.ramp();
            acts[usize::from(ghost_forest)] |= !g.ghost.is_empty();
            let (ox, oy) = g.origin;
            for &(x, y) in &g.dla.walkers {
                assert!(window(&g, x + ox, y + oy), "walker ({x}, {y})");
            }
        }
        assert_eq!(
            acts,
            [true, true],
            "both acts grew, held and faded within 70 s"
        );
        assert_eq!(g.dla.cells.capacity(), pool_cap, "site pool reallocated");
        assert_eq!(g.queue.capacity(), queue_cap, "hand-over queue reallocated");
        assert_eq!(g.ghost.capacity(), pool_cap, "ghost reallocated");
        // Every lit dot of the live raster lies in the window.
        g.ghost.clear();
        g.rasterise();
        let d = g.dots.dot_size();
        for y in 0..i32::from(d.height()) {
            for x in 0..i32::from(d.width()) {
                assert!(!g.dots.is_lit(x, y) || window(&g, x, y), "dot ({x}, {y})");
            }
        }
    }

    #[test]
    fn cry_a3_walkers_stay_in_kill_radius_and_grid() {
        let mut ctx = seeded(4);
        for line in [false, true] {
            for (w, h) in [
                (120u16, 100u16),
                (400, 200),
                (3, 3),
                (1, 1),
                (2, 60),
                (80, 1),
            ] {
                let mut d = Dla::new(Size::new(w, h), 8);
                d.reset(centre(&d), line, 1.0, &mut ctx.rng);
                for k in 0..150 {
                    d.advance(400, f64::from(k) * SIM_DT, &mut ctx.rng);
                    for &(x, y) in &d.walkers {
                        assert!(d.attach.get(x, y).is_some(), "{w}x{h}: off grid");
                        assert!(d.dist(x, y) <= d.r_kill() + 1e-9, "{w}x{h}: beyond kill");
                    }
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
        d.reset(centre(&d), false, p_stick, &mut ctx.rng);
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
    fn cry_a6_the_forest_is_a_sparse_deposit_screened_from_below() {
        // DLA from a line (p_s = 1) grows a forest of trees, not a compact film: at 60 % of
        // its height the deposit fills well under half the lattice below its front, and the
        // few tallest trees screen the rest, so the occupied fraction falls with height.
        let mut ctx = seeded(9);
        let mut d = Dla::new(Size::new(200, 120), 12);
        d.reset((0, 0), true, 1.0, &mut ctx.rng);
        while d.r_max < 60.0 {
            d.advance(20_000, 0.0, &mut ctx.rng);
        }
        let (w, h) = d.dims();
        let band = |lo: i32, hi: i32| {
            let n = d
                .cells
                .iter()
                .filter(|&&(_, y)| (h - 1 - hi..h - 1 - lo).contains(&y));
            n.count() as f64 / f64::from(w * (hi - lo))
        };
        let (low, high) = (band(5, 20), band(35, 50));
        assert!(low < 0.45, "low band {low}");
        assert!(high < low, "{high} vs {low}");
        assert!(connected_to_seed(&d));
    }

    #[test]
    fn cry_a7_arrival_colours_run_deep_to_pale_in_each_acts_ramp() {
        let tint = |ramp| Tint {
            ramp,
            t: 10.0,
            t_grown: 10.0,
        };
        let frost = tint(Act::Frost.ramp());
        let levels: Vec<u8> = (0..=9).map(|k| frost.cell_ink(k as f32).level).collect();
        assert!(levels.windows(2).all(|w| w[1] > w[0]), "{levels:?}");
        assert_eq!(frost.cell_ink(0.0), ink::ramp(Ramp::Ice, RAMP_LO));
        assert_eq!(
            frost.cell_ink(9.8),
            ink::ramp(Ramp::Ice, 1.0),
            "a fresh tip"
        );
        // The metal is a silver Starlight, not amber.
        assert_eq!(Act::Forest.ramp(), Ramp::Starlight);
        assert_eq!(
            tint(Act::Forest.ramp()).cell_ink(0.0).role,
            Role::Ramp(Ramp::Starlight)
        );

        // The acts alternate, each with its caption and ramp.
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut ctx);
        let mut seen = Vec::new();
        for _ in 0..(90.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            if seen.last() != Some(&g.caption()) {
                seen.push(g.caption());
            }
        }
        assert!(seen.len() >= 3, "{seen:?}");
        assert_eq!(
            &seen[..3],
            [
                Some("walkers stick"),
                Some("ions deposit"),
                Some("walkers stick")
            ]
        );
        assert_eq!(META.captions, &CAPTIONS);
    }

    #[test]
    fn cry_a1_cycle_grows_holds_and_hands_over() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut ctx);
        let (mut handed, mut faded, mut held) = (false, false, false);
        let mut prev = g.dla.cells.len();
        for _ in 0..(70.0 / SIM_DT) as usize {
            let (before, act) = (g.phase, g.act);
            g.update(SIM_DT, &mut ctx);
            if g.phase != Phase::Grow || g.dla.cells.len() != prev {
                assert!(connected_to_seed(&g.dla), "{:?}", g.phase);
            }
            held |= g.phase == Phase::Hold;
            if g.act != act {
                assert_eq!(before, Phase::Hold);
                assert_eq!(g.phase, Phase::Grow);
                assert!(!g.ghost.is_empty(), "the old act stays as a ghost");
                handed = true;
            }
            faded |= handed && g.ghost.is_empty();
            prev = g.dla.cells.len();
        }
        assert!(held && handed && faded, "{held} {handed} {faded}");
    }

    #[test]
    fn cry_a9_the_old_act_fades_tips_first_while_the_new_one_grows() {
        // Strength: roots (1) last exactly FADE; tips (1 − spread) go first; monotone.
        assert!(ghost_strength(1.0, FADE - 0.01).is_some());
        assert!(ghost_strength(1.0, FADE + 0.01).is_none());
        let tip = 1.0 - GHOST_SPREAD;
        let gone = |v0| (0..400).find(|&k| ghost_strength(v0, f64::from(k) * 0.01).is_none());
        assert!(gone(tip).unwrap() < gone(1.0).unwrap(), "tips fade first");
        // The ghost is drawn in its act's ramp below the live crystal's core.
        let full = ghost_ink(Ramp::Ice, 1.0);
        assert_eq!(full, ink::ramp(Ramp::Ice, GHOST_HI));
        assert!(full.level < ink::ramp(Ramp::Ice, RAMP_LO).level);
        assert!(ghost_ink(Ramp::Ice, GHOST_CUT).level >= 2 * ink::MIN_VISIBLE_LEVEL as u8);

        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut ctx);
        let (mut switches, mut min_lit, mut frames) = (0, usize::MAX, 0);
        for _ in 0..(62.0 / SIM_DT) as usize {
            let act = g.act;
            let old_n = g.dla.cells.len();
            g.update(SIM_DT, &mut ctx);
            if g.act != act {
                switches += 1;
                // Every old site is in the ghost, and the readout restarts with the new act.
                assert_eq!(g.ghost.len(), old_n);
                assert_eq!(g.ghost_ramp, act.ramp());
                let n: usize = g
                    .label
                    .as_str()
                    .split("N ")
                    .nth(1)
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(n < old_n / 2, "N {n} restarted (was {old_n})");
            }
            // The screen is never close to empty: from the first second on, at least
            // 120 cells hold crystal or ghost.
            if g.t > 1.0 || switches > 0 {
                let lit = (0..40)
                    .flat_map(|cy| (0..120).map(move |cx| (cx, cy)))
                    .filter(|&(cx, cy)| g.dots.cell_mask(cx, cy).is_some_and(|m| m != 0))
                    .count();
                min_lit = min_lit.min(lit);
                frames += 1;
            }
        }
        assert!(switches >= 2, "{switches} hand-overs in 62 s");
        assert!(frames > 0 && min_lit >= 120, "min {min_lit} lit cells");
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
            assert_eq!(g.room[0], room, "{size:?}");
        }
        g.dla.p_stick = 0.62;
        g.df = Some(1.71);
        for (room, want) in [
            (80, "p_s 0.62   N 1   D_f ~1.7"),
            (25, "p_s 0.62   N 1   D_f ~1.7"),
            (24, "N 1   D_f ~1.7"),
            (13, "D_f ~1.7"),
            (8, "D_f ~1.7"),
            (7, ""),
        ] {
            g.room = [room; 2];
            g.refresh_label();
            assert_eq!(g.label.as_str(), want, "room {room}");
        }
        // Drawn right-aligned, a blank cell either side, over any walker under it.
        g.room = [22; 2];
        g.refresh_label();
        g.phase = Phase::Grow;
        g.dla.walkers[0] = (30 * SUB_X, 10 * SUB_Y);
        let mut canvas = Canvas::new(Size::new(40, 12), GlyphMode::Ascii);
        g.render(&mut canvas);
        assert!(canvas.to_lines()[11].ends_with(" N 1   D_f ~1.7 "));
    }

    #[test]
    fn cry_a7_small_clusters_show_no_dimension() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(2, size);
        let mut g = FractalGrowth::default();
        g.resize(size, &mut ctx);
        while g.phase == Phase::Grow {
            g.update(SIM_DT, &mut ctx);
            if g.dla.cells.len() < DF_MIN_N {
                assert_eq!(g.df, None, "N {}", g.dla.cells.len());
            }
        }
        // A grown cluster's fit is the finite-size estimate, near but under 1.71.
        let df = g.df.expect("a grown frost shows D_f");
        assert!((1.4..=1.8).contains(&df), "D_f {df}");
    }

    #[test]
    fn cry_a8_the_crystal_draws_on_the_terminal_background() {
        // No halo or backdrop: every cell keeps the terminal's background through both
        // acts, in either glyph mode, so the braille filaments are the brightest thing on
        // screen and the fjords between branches stay dark.
        for size in [Size::new(200, 50), Size::new(80, 24)] {
            let mut ctx = Ctx::new(3, size);
            let mut g = FractalGrowth::default();
            g.resize(size, &mut ctx);
            let mut acts = [false; 2];
            for k in 0..(60.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                acts[usize::from(g.act == Act::Forest)] = true;
                if k % 30 != 0 {
                    continue;
                }
                for mode in [GlyphMode::Unicode, GlyphMode::Ascii] {
                    let mut c = Canvas::new(size, mode);
                    g.render(&mut c);
                    let cells = c.cells();
                    assert!(cells.iter().any(|cell| cell.ink.role != Role::Background));
                    let t = g.t;
                    let bare = cells.iter().all(|cell| cell.bg == Ink::BLANK);
                    assert!(bare, "{size:?} {mode:?} at {t:.1} s");
                }
            }
            assert_eq!(acts, [true, true], "{size:?}: both acts ran");
        }
    }

    #[test]
    fn cry_a6_a_walker_above_the_line_returns_at_its_cauchy_first_passage_point() {
        // Far from the walls |offset| has median a (the Cauchy law of scale a), not that
        // of a uniform point across the strip. Exact quadrature over u.
        let n = 4001;
        let a = 10.0;
        let mut offsets: Vec<i32> = (0..n)
            .map(|k| {
                let u = (f64::from(k) + 0.5) / f64::from(n);
                (line_return(500_000, a, u, 1_000_000) - 500_000).abs()
            })
            .collect();
        offsets.sort_unstable();
        let median = f64::from(offsets[offsets.len() / 2]);
        assert!((median - a).abs() <= 1.0, "median {median}");
        // Walls reflect: every return is on the strip, and from far above (a ≫ width) the
        // landing point is uniform across it, the far-field limit.
        let (max, n) = (40, 40_001);
        let mut hist = [0u32; 41];
        for k in 0..n {
            let x = line_return(3, 1e5, (f64::from(k) + 0.5) / f64::from(n), max);
            assert!((0..=max).contains(&x), "{x}");
            hist[x as usize] += 1;
        }
        let mean = f64::from(n) / 40.0;
        assert!(
            hist[1..40]
                .iter()
                .all(|&c| (f64::from(c) / mean - 1.0).abs() < 0.12),
            "{hist:?}"
        );
        // Degenerate strips and non-finite offsets stay on the lattice.
        assert_eq!(line_return(0, 5.0, 0.9, 0), 0);
        assert!((0..=9).contains(&line_return(4, f64::INFINITY, 0.7, 9)));
        assert_eq!(fold(f64::NAN, 9), 0);
        for (v, want) in [(-1.0, 1), (10.0, 8), (19.0, 1), (-18.0, 0), (27.0, 9)] {
            assert_eq!(fold(v, 9), want, "{v}");
        }
    }

    #[test]
    fn cry_a5_the_forest_survives_lattices_under_four_dots_tall() {
        // There the pool (a quarter of the sites) cannot hold a whole cathode row, so the
        // readout must count the roots the pool took, not the lattice width (GEN-R4).
        for (w, h) in [(20_000, 1), (u16::MAX, 1), (3, 1), (1, 1)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(8, size);
            let mut g = FractalGrowth::default();
            g.resize(size, &mut ctx);
            g.act = Act::Forest;
            g.reseed(&mut ctx);
            assert!(g.dla.roots <= g.dla.cells.capacity(), "{w}x{h}");
            for _ in 0..30 {
                g.update(0.25, &mut ctx);
                assert!(g.dla.cells.len() >= g.dla.roots, "{w}x{h}");
            }
        }
    }

    #[test]
    fn cry_a7_renders_ramps_walkers_and_ascii_fallbacks() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        let mut roles = std::collections::HashSet::new();
        for _ in 0..(40.0 / SIM_DT) as usize {
            stage.step(SIM_DT);
            let frame = stage.render();
            assert!(frame.to_lines().iter().all(|l| l.is_ascii()));
            roles.extend(frame.cells().iter().map(|c| c.ink.role));
        }
        for r in [
            Role::Ramp(Ramp::Ice),
            Role::Ramp(Ramp::Starlight),
            Role::Dim,
            Role::Text,
        ] {
            assert!(roles.contains(&r), "{r:?} missing");
        }
    }
}
