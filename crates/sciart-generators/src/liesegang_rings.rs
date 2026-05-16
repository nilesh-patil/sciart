//! Liesegang bands snapping into place down a rack of gel columns, then rings around a
//! silver drop in a gel dish (SPEC-002 `liesegang_rings`).
//!
//! Chemistry: per tube a 1-D reaction–diffusion column, `A + B → C` (soluble product,
//! idealised 1:1) and `C → D` (immobile precipitate), with Ostwald's supersaturation
//! mechanism made discrete: a node nucleates once `c ≥ c*` (Keller & Rubinow 1981), or
//! `c ≥ c_g` next to an already nucleated node (growth on adjacent precipitate; Chopard,
//! Luthi & Droz 1994), and only nucleated nodes precipitate. The outer electrolyte A
//! diffuses in from a reservoir on top and the product C piles up ahead of its front until
//! it nucleates a band, which then drains the surrounding C and leaves a depleted gap:
//! the rhythm of periodic precipitation. Five tubes at different `a₀` show the spacing
//! (Jablczynski), width, time (Morse & Pierce 1903) and Matalon–Packter laws side by side.
//! The dish is the same column in polar coordinates around the drop (LIES-A12).
//!
//! The chemistry never depends on the terminal: every column is simulated on 160 nodes (a
//! capped lattice, ADR-0012), the live act's columns stepping; a resize only rebuilds the
//! layout. The raster is rebuilt in `update`/`resize`; `render` only blits it and writes
//! the walls and header (the M10 raster convention).

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::ink;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "liesegang_rings",
    category: Category::Chemistry,
    description: "Liesegang bands and rings snapping into place in gel tubes and a gel dish",
    title: "Liesegang rings · form in bursts",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(LiesegangRings::new(None))
}

/// The two acts (LIES-A6): the rack of tubes, then Liesegang's own 1896 dish, a drop
/// of silver nitrate in the centre of a dichromate gel. The discriminant indexes every
/// per-act table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    Rack,
    Dish,
}

const ACTS: [Act; 2] = [Act::Rack, Act::Dish];
/// One caption per act (LIES-A13).
const CAPTIONS: [&str; 2] = ["bands in tubes", "a silver drop"];
/// How far into `grow` each act opens, so the first frame already shows bands forming
/// (LIES-A11). The rack steps five columns to the dish's one, so it opens earlier and
/// both catch-ups stay within 4 500 column steps, a one-off cost at scene entry.
const PREGROWN_S: [f64; 2] = [8.0, 16.0];
/// The dish: well radius in nodes, outer electrolyte (the most concentrated preset, so
/// the positivity bound of the tubes covers it) (LIES-A12).
const WELL: f64 = 10.0;
const DISH_A0: f64 = 3.0;
/// Header column of the dish's one ratio: right of the full legend and its 2-column gap,
/// clear of the rim's top arc (LIES-A13).
const DISH_RATIO_COL: i32 = 1 + "x(n+1)/x(n)".len() as i32 + 2;
/// The palette of every gel pixel (LIES-A9), in false colour: real silver dichromate is
/// ruby to brick red, and in a real gel the densest bands are the darkest, whereas here
/// lightness encodes the amount of precipitate and the yellow dichromate gel is drawn
/// black. `Cobalt` (ultramarine → cobalt → silver-blue) reads as a cool mineral
/// precipitate that brightens towards silver, and keeps the scene apart from the
/// gallery's many amber (`Ember`) and crimson (`HAlpha`) scenes and from its neighbours'
/// magenta (`gray_scott`) and red-to-blue (`belousov_zhabotinsky`).
const PALETTE: Ramp = Ramp::Cobalt;
/// Ramp position of the densest precipitate: silver-blue, below the ramp's near-white
/// top, so a mature band never reads as white light.
const BAND_TOP: f64 = 0.85;

/// Nodes per column; node `i` sits at depth `x = i + 1`, the reservoir ghost at `x = 0`.
pub(crate) const NODES: usize = 160;
/// Nodes shown on screen, `[0, VIEW_NODES)`, once the gel has ≥ 42 pixel rows.
const VIEW_NODES: f64 = 100.0;
/// Most nodes per gel pixel: shorter terminals show fewer nodes (never below
/// [`MIN_VIEW_NODES`]) rather than merge bands and gaps into one striped column.
const NODES_PER_PX: f64 = 2.4;
const MIN_VIEW_NODES: f64 = 50.0;
/// The dish's fewest nodes out to the rim (small dishes zoom in on the first rings), and
/// its most nodes per pixel of radius: finer than the tubes', because a ring gap under
/// one pixel wide rasterises as a dotted circle rather than a line (LIES-A13).
const DISH_MIN_VIEW: f64 = 30.0;
const DISH_NODES_PER_PX: f64 = 1.8;

// Dimensionless model parameters (SPEC-002 Model; D_a = D_b = 1).
const TAU: f64 = 0.15;
const D_C: f64 = 0.3;
const K: f64 = 1.0;
const LAMBDA: f64 = 0.05;
const C_S: f64 = 0.01;
const C_G: f64 = 0.037;
const C_STAR: f64 = 0.04;
pub(crate) const B0: f64 = 0.05;
/// Outer electrolyte concentration of each tube, dilute → concentrated.
pub(crate) const PRESETS: [f64; 5] = [0.7, 1.0, 1.5, 2.0, 3.0];

/// The 100 s loop (LIES-A6): the three phases of each act, the rack's then the dish's;
/// a beat's phase is its index mod [`PHASES_PER_ACT`], matching [`sim_time`].
const PHASES: [(&str, f64); 6] = [
    ("rack grow", 36.0),
    ("rack hold", 6.0),
    ("rack dissolve", 8.0),
    ("dish grow", 36.0),
    ("dish hold", 6.0),
    ("dish dissolve", 8.0),
];
const PHASES_PER_ACT: usize = 3;
const GROW: usize = 0;
const HOLD: usize = 1;
const DISSOLVE: usize = 2;
/// Length of one act, s.
const ACT_S: f64 = 50.0;
/// Sim time reached at the end of `grow`; `t_sim = T_END·(s/36)^{3/2}`.
const T_END: f64 = 1200.0;
/// The warp's final rate, `d t_sim/ds` at the end of `grow` (50 units/s), kept through
/// `hold`.
const HOLD_RATE: f64 = 1.5 * T_END / PHASES[GROW].1;
/// Per-cycle jitter of `b₀` (LIES-A6).
const B0_JITTER: f64 = 0.03;
/// Redissolution front `y_e = 120·√progress` nodes, and the decay rate above it.
const FRONT_NODES: f64 = 120.0;
const DISSOLVE_RATE: f64 = 3.0;
/// Band-scan period, s, and the run threshold as a fraction of the column's max d.
const SCAN_EVERY: f64 = 0.25;
const RUN_FRAC: f64 = 0.05;
/// Room for every band a 160-node column can hold (≈ 16 at a₀ = 3).
const MAX_BANDS: usize = 24;

// Inks (SPEC-002 Inks).
const D_FULL: f64 = 0.2;
/// Precipitate is fully visible from this d; below it the level fades linearly to 0.
const D_VIS: f64 = 0.02;
/// Precipitate below this d is not drawn at all: a band dissolved for the rest of the
/// loop (d ≈ 2·10⁻⁴ at the view bottom) must leave clear gel, not a faint class.
const D_FAINT: f64 = 0.002;
/// The junction zone is dimmed by this factor so the eye goes to the bands.
const JUNCTION_DIM: f64 = 0.3;
const HAZE: f64 = 0.25;
const LIQUID_INK: Ink = Ink::new(Role::Wave, 60);
/// Tube walls and the dish rim.
const WALL_INK: Ink = Ink::new(Role::Structure, 100);
const RATIO_INK: Ink = Ink::new(Role::Text, 170);
const LEGEND_INK: Ink = Ink::new(Role::Text, 110);
/// ASCII band threshold on the box-filtered d.
const D_BAND: f64 = 0.04;

/// ASCII class of a pixel, from the fields rather than the shaded ink. Ordered by how
/// much a pixel shows, so a cell can take the ink of its stronger half.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    #[default]
    Empty,
    /// Clear gel carrying only the supersaturation haze.
    Haze,
    /// Faint precipitate, or the junction zone.
    Faint,
    Band,
    Liquid,
    /// The dish's rim.
    Glass,
}

/// Visible presets for `n` tubes, always dilute → concentrated.
const SHOWN: [&[usize]; 5] = [&[2], &[1, 3], &[0, 2, 4], &[0, 1, 3, 4], &[0, 1, 2, 3, 4]];

/// Boundary conditions of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ends {
    /// Reservoir `a = a₀` on top with no flux of b and c; a semi-infinite gel below
    /// (`a = 0, b = b₀, c = 0`), since a closed bottom starves the front of b.
    Gel,
    /// No flux at both ends: only the conservation check (LIES-A2) needs it.
    #[cfg(test)]
    Closed,
}

/// One gel column: the four fields plus the irreversible nucleation flags. `Clone` but
/// not `Copy`: at 160 nodes a column is ≈ 6.4 kB, too big to copy silently.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Column<const N: usize> {
    a: [f64; N],
    b: [f64; N],
    c: [f64; N],
    d: [f64; N],
    nuc: [bool; N],
    /// Sim time at which each node nucleated, ∞ until then.
    t_nuc: [f64; N],
    a0: f64,
    /// This cycle's gel concentration `b₀,c`.
    b0: f64,
    /// Radius of the central well in nodes for a dish column, `∞` for a planar tube
    /// (LIES-A12): node `i` sits at `r = well + i + 1`, so a tube's radial term is 0.
    well: f64,
    /// Whole Euler steps taken this cycle.
    steps: u32,
}

impl<const N: usize> Column<N> {
    /// A fresh tube: gel loaded with `b0`, no A, C, D or nuclei. This is the reset state
    /// `(0, b₀,c, 0, 0, false, ∞)` (LIES-A6).
    pub(crate) fn new(a0: f64, b0: f64) -> Self {
        Self::radial(a0, b0, f64::INFINITY)
    }

    /// A fresh axisymmetric dish column around a well of `well` nodes (LIES-A12).
    pub(crate) fn radial(a0: f64, b0: f64, well: f64) -> Self {
        Self {
            a: [0.0; N],
            b: [b0; N],
            c: [0.0; N],
            d: [0.0; N],
            nuc: [false; N],
            t_nuc: [f64::INFINITY; N],
            a0,
            b0,
            well,
            steps: 0,
        }
    }

    /// Back to the reset state with a new `b₀,c` (LIES-A6).
    pub(crate) fn reset(&mut self, b0: f64) {
        *self = Self::radial(self.a0, b0, self.well);
    }

    /// Sim time of the steps taken so far.
    pub(crate) fn time(&self) -> f64 {
        f64::from(self.steps) * TAU
    }

    /// One explicit Euler step of `τ` (3-point Laplacian), then the Jacobi nucleation
    /// update from the pre-step flags, so the result does not depend on sweep order.
    ///
    /// Positivity and the bounds `a ≤ a₀`, `b ≤ b₀` hold because every update is a convex
    /// combination: `τ(2 + k·a₀) ≤ 0.75` and `τ(2D_c + λ) ≤ 1` (LIES-A1).
    ///
    /// A dish column adds the radial term of the polar Laplacian, `f'/r`, as
    /// `g·(f₊ − f₋)` with `g = 1/(2r)` (LIES-A12): the neighbour weights `1 ∓ g` stay
    /// positive (r ≥ 1) and still sum to 2, so the same bounds hold, and `Σ r·f` is what
    /// diffusion conserves. A tube (`well = ∞`) takes the planar sweep, the M10
    /// arithmetic bit for bit, without a per-node division.
    pub(crate) fn step(&mut self, ends: Ends) {
        if self.well.is_finite() {
            self.sweep::<true>(ends);
        } else {
            self.sweep::<false>(ends);
        }
    }

    fn sweep<const RADIAL: bool>(&mut self, ends: Ends) {
        const { assert!(N > 0, "a column needs at least one node") };
        // Ghost values above node 0 and below node N − 1 (old values, read before the
        // sweep overwrites them).
        let top_a = match ends {
            Ends::Gel => self.a0,
            #[cfg(test)]
            Ends::Closed => self.a[0],
        };
        let bottom = match ends {
            Ends::Gel => (0.0, self.b0, 0.0),
            #[cfg(test)]
            Ends::Closed => (self.a[N - 1], self.b[N - 1], self.c[N - 1]),
        };
        self.steps += 1;
        let now = self.time();
        // Old values of node i − 1, carried along the in-place sweep.
        let (mut al, mut bl, mut cl, mut nl) = (top_a, self.b[0], self.c[0], false);
        for i in 0..N {
            let (a, b, c, was) = (self.a[i], self.b[i], self.c[i], self.nuc[i]);
            let (ar, br, cr, nr) = if i + 1 < N {
                (self.a[i + 1], self.b[i + 1], self.c[i + 1], self.nuc[i + 1])
            } else {
                (bottom.0, bottom.1, bottom.2, false)
            };
            let g = if RADIAL {
                0.5 / (self.well + (i + 1) as f64)
            } else {
                0.0
            };
            let lap = |up: f64, x: f64, down: f64| {
                let planar = up - 2.0 * x + down;
                if RADIAL {
                    planar + g * (down - up)
                } else {
                    planar
                }
            };
            let r = K * a * b;
            let p = if was {
                LAMBDA * (c - C_S).max(0.0)
            } else {
                0.0
            };
            self.a[i] = a + TAU * (lap(al, a, ar) - r);
            self.b[i] = b + TAU * (lap(bl, b, br) - r);
            let c_new = c + TAU * (D_C * lap(cl, c, cr) + r - p);
            self.c[i] = c_new;
            self.d[i] += TAU * p;
            if !was && (c_new >= C_STAR || (c_new >= C_G && (nl || nr))) {
                self.nuc[i] = true;
                self.t_nuc[i] = now;
            }
            (al, bl, cl, nl) = (a, b, c, was);
        }
    }

    /// Take whole steps until the next one would pass `t_sim`, so the scheme always
    /// uses exactly `τ`.
    pub(crate) fn run_to(&mut self, t_sim: f64) {
        // The tiny margin keeps `t_sim = n·τ` from losing its last step to round-off.
        let target = (t_sim / TAU + 1e-9).floor().clamp(0.0, f64::from(u32::MAX)) as u32;
        while self.steps < target {
            self.step(Ends::Gel);
        }
    }

    /// Scripted redissolution: above the front (`i < front` nodes) c and d decay by `k`;
    /// a, b and the flags are untouched (LIES-A6).
    fn dissolve(&mut self, front: f64, k: f64) {
        for i in (0..N).take_while(|&i| (i as f64) < front) {
            self.c[i] *= k;
            self.d[i] *= k;
        }
    }
}

/// A band found by [`scan_bands`]: a run of nodes with `d ≥ 5 %` of the column max.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Band {
    /// d-weighted centroid depth (`x = i + 1`).
    x: f64,
    /// Node range `start..end`.
    start: usize,
    end: usize,
    /// Birth time: the earliest nucleation among its nodes.
    t_n: f64,
}

/// Up to [`MAX_BANDS`] bands, top-down; the first is the junction zone, excluded from
/// every law.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Bands {
    list: [Band; MAX_BANDS],
    len: usize,
}

impl Bands {
    pub(crate) fn as_slice(&self) -> &[Band] {
        &self.list[..self.len]
    }

    /// End of the junction zone (0 before any precipitate).
    fn junction_end(&self) -> usize {
        self.as_slice().first().map_or(0, |b| b.end)
    }
}

/// Split a column's precipitate into bands (SPEC-002 Model: band scan).
pub(crate) fn scan_bands(d: &[f64], t_nuc: &[f64]) -> Bands {
    let mut bands = Bands::default();
    let max = d.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        return bands;
    }
    let on = |i: usize| d[i] >= RUN_FRAC * max;
    let mut i = 0;
    while i < d.len() && bands.len < MAX_BANDS {
        if !on(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < d.len() && on(i) {
            i += 1;
        }
        let run = start..i;
        let mass: f64 = d[run.clone()].iter().sum();
        let moment: f64 = run.clone().map(|j| d[j] * (j + 1) as f64).sum();
        let t_n = t_nuc[run].iter().copied().fold(f64::INFINITY, f64::min);
        bands.list[bands.len] = Band {
            x: moment / mass,
            start,
            end: i,
            t_n,
        };
        bands.len += 1;
    }
    bands
}

/// Successive spacing ratios `x_{n+1}/x_n` of the bands after the junction zone.
pub(crate) fn ratios(bands: &[Band]) -> impl Iterator<Item = f64> + '_ {
    bands
        .get(1..)
        .unwrap_or_default()
        .windows(2)
        .map(|w| w[1].x / w[0].x)
}

/// The header's live ratio and the end node of the deeper band it measures: the two
/// deepest *complete* bands after the junction zone, a band being complete once a deeper
/// band has nucleated (a nucleated run starts below it).
pub(crate) fn live_ratio(bands: &[Band], nuc: &[bool]) -> Option<(f64, usize)> {
    let deepest_run = (1..nuc.len())
        .rev()
        .find(|&j| nuc[j] && !nuc[j - 1])
        .unwrap_or(0);
    let complete = bands
        .iter()
        .skip(1)
        .take_while(|b| b.end <= deepest_run)
        .count();
    let pair = bands.get(..=complete)?;
    Some((ratios(pair).last()?, pair.last()?.end))
}

/// The loop's phases (LIES-A6).
fn schedule() -> Timeline {
    Timeline::new(&PHASES)
}

/// Depth in nodes of the redissolution front, `y_e = 120·√progress` during `dissolve`
/// and 0 otherwise: nodes above it are dissolving (LIES-A6).
fn dissolve_front(beat: Beat) -> f64 {
    if phase(beat) == DISSOLVE {
        FRONT_NODES * beat.progress.sqrt()
    } else {
        0.0
    }
}

/// The phase (grow, hold, dissolve) of a beat of the loop.
fn phase(beat: Beat) -> usize {
    beat.index % PHASES_PER_ACT
}

/// The act a beat of the loop belongs to.
pub(crate) fn act(beat: Beat) -> Act {
    ACTS[beat.index / PHASES_PER_ACT % ACTS.len()]
}

/// Sim time at a point of an act: `T_END·(s/36)^{3/2}` through `grow`, then on at its
/// final rate through `hold`, frozen at 1500 during `dissolve` (LIES-A6).
pub(crate) fn sim_time(beat: Beat) -> f64 {
    match phase(beat) {
        GROW => T_END * (beat.local / PHASES[GROW].1).powf(1.5),
        DISSOLVE => T_END + HOLD_RATE * PHASES[HOLD].1,
        _ => T_END + HOLD_RATE * beat.local,
    }
}

/// Number of visible tubes for a width (LIES-A7).
pub(crate) fn tube_count(width: u16) -> usize {
    ((usize::from(width) + 4) / 24).clamp(1, 5)
}

/// Visible presets for a width, dilute → concentrated (LIES-A7).
pub(crate) fn shown(width: u16) -> &'static [usize] {
    SHOWN[tube_count(width) - 1]
}

/// Where the tubes sit on screen, rebuilt on resize.
#[derive(Debug, Clone, Copy, Default)]
struct Layout {
    width: u16,
    height: u16,
    /// Interior columns per tube.
    inner: i32,
    /// Left interior column of each visible tube.
    x0: [i32; 5],
    /// Gel pixel rows, from pixel row 4 (cell row 2) to cell row `h − 2`: the last row is
    /// the title's (LIES-A10).
    gel_px: i32,
    /// Nodes shown, `[0, view)`: `clamp(2.4·gel_px, 50, 100)`.
    view: f64,
    /// Meniscus depth at the tube axis, pixels: 0 below 40 gel rows (h < 22), where
    /// a pixel spans several nodes and a 1 px sag only notches single pixels.
    sag: f64,
    dish: Dish,
}

/// Where the dish sits on screen (LIES-A13), in pixels: centred on the canvas between
/// the header and the title rows.
#[derive(Debug, Clone, Copy, Default)]
struct Dish {
    cx: f64,
    cy: f64,
    /// Inner radius of the one-pixel rim.
    rim: f64,
    /// Nodes shown out to the rim, `[0, view)`, and nodes per pixel of radius.
    view: f64,
    scale: f64,
}

impl Dish {
    fn new(w: i32, h: i32) -> Self {
        // Pixel rows 2..2(h − 1) lie between the header and the title rows.
        let rim = f64::from((h - 3).min(w / 2 - 2).max(0));
        let view = (DISH_NODES_PER_PX * rim - WELL).clamp(DISH_MIN_VIEW, VIEW_NODES);
        Self {
            cx: f64::from(w) / 2.0,
            cy: f64::from(h),
            rim,
            view,
            scale: (WELL + view) / rim.max(1.0),
        }
    }
}

impl Layout {
    fn new(size: Size) -> Self {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let n = tube_count(size.width()) as i32;
        // Tubes about as wide as the gaps between them, so the rack fills the width.
        let inner = ((f64::from(w) / (2.2 * f64::from(n))).round() as i32).clamp(1, 20);
        let pitch = inner + 2;
        let gap = ((w - n * pitch) / (n + 1)).max(0);
        let left = (w - (n * pitch + (n - 1) * gap)) / 2;
        let mut x0 = [0; 5];
        for (k, x) in x0.iter_mut().enumerate().take(n as usize) {
            *x = left + k as i32 * (pitch + gap) + 1;
        }
        let gel_px = 2 * (h - 3).max(0);
        let view = (NODES_PER_PX * f64::from(gel_px)).clamp(MIN_VIEW_NODES, VIEW_NODES);
        // h ≥ 22 is about 40 gel rows.
        let sag = match h {
            40.. => 2.0,
            22.. => 1.0,
            _ => 0.0,
        };
        Self {
            width: size.width(),
            height: size.height(),
            inner,
            x0,
            gel_px,
            view,
            sag,
            dish: Dish::new(w, h),
        }
    }

    /// Header column of a tube's ratio (4 characters, centred over the interior).
    fn ratio_col(&self, slot: usize) -> i32 {
        self.x0[slot] + (self.inner - 4) / 2
    }

    /// `(tube slot, preset)` of the visible tubes.
    fn tubes(&self) -> impl Iterator<Item = (usize, usize)> {
        shown(self.width).iter().copied().enumerate()
    }

    /// `1 − u²` across the tube for interior column `c`, `u ∈ (−1, 1)`: the meniscus and
    /// cylinder-shading profile.
    fn bulge(&self, c: i32) -> f64 {
        let u = 2.0 * (f64::from(c) + 0.5) / f64::from(self.inner) - 1.0;
        (1.0 - u * u).max(0.0)
    }

    /// The node window `[lo, hi)` each gel pixel shows, column-major (`inner` columns of
    /// `gel_px` rows); `None` above the concave meniscus, where the pixel is liquid. The
    /// gel's `gel_px` rows show nodes `[0, view)` and the pixel centre's depth is
    /// `row + ½ − sag·(1 − u²)`, so interface and bands curve together.
    fn windows(&self) -> impl Iterator<Item = Option<(f64, f64)>> + '_ {
        let nodes_per_px = self.view / f64::from(self.gel_px.max(1));
        (0..self.inner).flat_map(move |c| {
            let sag = self.sag * self.bulge(c);
            (0..self.gel_px).map(move |row| {
                let depth = f64::from(row) + 0.5 - sag;
                (depth >= 0.0).then(|| {
                    let lo = ((depth - 0.5) * nodes_per_px).max(0.0);
                    (lo, ((depth + 0.5) * nodes_per_px).min(NODES as f64))
                })
            })
        })
    }
}

/// The running integral `P[k] = Σ_{i<k} f[i]` of a node field, node `i` covering depth
/// `[i, i + 1)`, so the box-filtered mean over any window costs two lookups whatever the
/// pixel-to-node ratio.
struct Integral<'a> {
    f: &'a [f64; NODES],
    p: [f64; NODES + 1],
}

impl<'a> Integral<'a> {
    fn new(f: &'a [f64; NODES]) -> Self {
        let mut p = [0.0; NODES + 1];
        for (i, v) in f.iter().enumerate() {
            p[i + 1] = p[i] + v;
        }
        Self { f, p }
    }

    /// `∫₀^z f` for `z ∈ [0, NODES]`.
    fn at(&self, z: f64) -> f64 {
        let k = z as usize;
        match self.f.get(k) {
            Some(v) => self.p[k] + (z - k as f64) * v,
            None => self.p[NODES],
        }
    }

    /// Mean over `[lo, hi)`. Exactly 0 where the field is 0 across the window (adding
    /// 0.0 is exact, so both integrals read the same partial sum), so untouched gel
    /// stays clear; never negative, since rounding is monotone.
    fn mean(&self, lo: f64, hi: f64) -> f64 {
        if hi > lo {
            ((self.at(hi) - self.at(lo)) / (hi - lo)).max(0.0)
        } else {
            0.0
        }
    }
}

/// The ramp position and ASCII class of a gel pixel from its box-filtered fields
/// (SPEC-002 Inks and ASCII), before cylinder shading; `None` leaves clear gel to the
/// terminal's background, so depletion gaps read as dark.
///
/// Precipitate fades in linearly over `d ∈ [0, D_VIS]`, so the level is continuous in d:
/// a newborn band is a thin faint line that brightens, and a dissolving band fades all
/// the way out instead of freezing at a floor until the loop cuts.
pub(crate) fn gel_pixel(d: f64, c: f64, junction: bool) -> Option<(f64, Class)> {
    // x^0.75 as √x·⁴√x: two square roots are several times cheaper than `powf`.
    let x = (d / D_FULL).clamp(0.0, 1.0).sqrt();
    let mut precip = BAND_TOP * (0.30 + 0.70 * x * x.sqrt()) * (d / D_VIS).clamp(0.0, 1.0);
    if junction {
        // Scaled, not capped, so the cap keeps its cloudy gradient below the bands.
        precip *= JUNCTION_DIM;
    }
    let haze = HAZE * ((c / C_STAR - 0.5) / 0.5).clamp(0.0, 1.0);
    let class = if d >= D_BAND && !junction {
        Class::Band
    } else if d > D_FAINT {
        Class::Faint
    } else if haze > 0.0 {
        Class::Haze
    } else {
        return None;
    };
    Some((precip.max(haze), class))
}

/// The ASCII glyph of a cell from its two pixel classes (SPEC-002 ASCII). The haze is
/// never more than a light `.` stipple, so its large sinking cloud cannot outweigh the
/// precipitate.
fn ascii_glyph(top: Class, bottom: Class) -> char {
    use Class::{Band, Empty, Faint, Glass, Liquid};
    match (top, bottom) {
        (Glass, _) | (_, Glass) => 'o',
        (Liquid, _) | (_, Liquid) => '~',
        (Band, Band) => '=',
        (Band, _) => '-',
        (_, Band) => '_',
        (Faint, Faint) => ':',
        (Empty, Empty) => ' ',
        // One faint pixel, or any haze.
        _ => '.',
    }
}

/// The ASCII cell of a pixel pair (SPEC-002 ASCII): `~` always in the liquid ink,
/// otherwise the ink of the half with the higher class. Comparing levels would mix
/// roles (Wave vs Ramp) and could pick a blank pixel over a lit one of equal level.
fn ascii_cell(ct: Class, cb: Class, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    match ascii_glyph(ct, cb) {
        ' ' => None,
        'o' => Some(('o', WALL_INK)),
        '~' => Some(('~', LIQUID_INK)),
        ch => Some((ch, if ct >= cb { top } else { bottom })),
    }
}

/// The header legend that fits at column 1 with a 2-column gap before the first
/// tube's ratio at column `room` (the walls start below the header, so nothing else can
/// collide). Plain ASCII in both modes: subscript glyphs are missing from several common
/// terminal fonts.
fn legend(room: i32) -> Option<&'static str> {
    ["x(n+1)/x(n)", "xn+1/xn"]
        .into_iter()
        .find(|s| 1 + s.len() as i32 + 2 <= room)
}

#[derive(Debug, Clone)]
struct Tube {
    col: Column<NODES>,
    /// The latest band scan (frozen during `dissolve`).
    bands: Bands,
}

#[derive(Debug)]
struct LiesegangRings {
    schedule: Timeline,
    /// Opening act forced by tests; `None` draws it from `ctx.rng` on the first resize.
    start: Option<Act>,
    /// Scene time, s.
    t: f64,
    /// Acts begun up to `t`, two per loop; a change resets the columns.
    run: i64,
    /// Time since the last band scan, s.
    since_scan: f64,
    tubes: [Tube; 5],
    dish: Tube,
    layout: Layout,
    /// [`Layout::windows`], cached on resize.
    windows: Vec<Option<(f64, f64)>>,
    pixels: Pixels,
    /// ASCII class per pixel, same layout as `pixels`.
    classes: Grid<Class>,
}

/// Acts begun up to scene time `t`: loops times [`ACTS`] plus the live act's index.
fn run_of(schedule: &Timeline, t: f64) -> i64 {
    schedule.cycle(t) * ACTS.len() as i64 + (schedule.at(t).index / PHASES_PER_ACT) as i64
}

impl LiesegangRings {
    fn new(start: Option<Act>) -> Self {
        let tube = |a0| Tube {
            col: Column::new(a0, B0),
            bands: Bands::default(),
        };
        Self {
            schedule: schedule(),
            start,
            t: 0.0,
            run: 0,
            since_scan: 0.0,
            tubes: PRESETS.map(tube),
            dish: Tube {
                col: Column::radial(DISH_A0, B0, WELL),
                bands: Bands::default(),
            },
            layout: Layout::default(),
            windows: Vec::new(),
            pixels: Pixels::default(),
            classes: Grid::default(),
        }
    }

    /// The live act.
    fn act(&self) -> Act {
        act(self.schedule.at(self.t))
    }

    /// The columns an act steps: the five tubes, or the dish.
    fn columns(&mut self, act: Act) -> &mut [Tube] {
        match act {
            Act::Rack => &mut self.tubes,
            Act::Dish => std::slice::from_mut(&mut self.dish),
        }
    }

    fn scan(&mut self) {
        for tube in self.tubes.iter_mut().chain([&mut self.dish]) {
            tube.bands = scan_bands(&tube.col.d, &tube.col.t_nuc);
        }
        self.since_scan = 0.0;
    }

    /// A new act: fresh gel at a slightly different `b₀` (LIES-A6).
    fn restart(&mut self, rng: &mut impl Rng) {
        let xi: f64 = rng.random_range(-1.0..=1.0);
        let b0 = B0 * (1.0 + B0_JITTER * xi);
        for tube in self.tubes.iter_mut().chain([&mut self.dish]) {
            tube.col.reset(b0);
        }
        self.scan();
    }

    /// Run the live act's chemistry up to scene time `t`.
    fn advance(&mut self) {
        let beat = self.schedule.at(self.t);
        let t_sim = sim_time(beat);
        for tube in self.columns(act(beat)) {
            tube.col.run_to(t_sim);
        }
    }

    /// Paint the live act.
    fn rasterise(&mut self) {
        self.pixels.clear();
        self.classes.fill(Class::Empty);
        match self.act() {
            Act::Rack => self.rasterise_rack(),
            Act::Dish => self.rasterise_dish(),
        }
    }

    /// The liquid row and every visible gel pixel of the tubes.
    fn rasterise_rack(&mut self) {
        let Self {
            tubes,
            layout: lay,
            windows,
            pixels,
            classes,
            ..
        } = self;
        let mut set = |x: i32, y: i32, ink: Ink, class: Class| {
            pixels.set(x, y, ink);
            classes.set(x, y, class);
        };
        let rows = lay.gel_px.max(0) as usize;
        for (slot, preset) in lay.tubes() {
            let Tube { col, bands } = &tubes[preset];
            let (d, c) = (Integral::new(&col.d), Integral::new(&col.c));
            let junction = bands.junction_end() as f64;
            for i in 0..lay.inner {
                let x = lay.x0[slot] + i;
                let shade = 0.8 + 0.2 * lay.bulge(i).sqrt();
                for y in 2..4 {
                    set(x, y, LIQUID_INK, Class::Liquid);
                }
                let column = &windows[i as usize * rows..][..rows];
                for (y, window) in (4..).zip(column) {
                    let Some((lo, hi)) = *window else {
                        set(x, y, LIQUID_INK, Class::Liquid);
                        continue;
                    };
                    let (d, c) = (d.mean(lo, hi), c.mean(lo, hi));
                    if let Some((v, class)) = gel_pixel(d, c, lo < junction) {
                        set(x, y, ink::ramp(PALETTE, v * shade), class);
                    }
                }
            }
        }
    }

    /// The dish (LIES-A13): the well, the gel box-filtered over each pixel's radial
    /// window through the same running integral as the tubes, and the rim.
    fn rasterise_dish(&mut self) {
        let Dish {
            cx,
            cy,
            rim,
            view,
            scale,
        } = self.layout.dish;
        if rim < 1.0 {
            return;
        }
        let Tube { col, bands } = &self.dish;
        let (d, c) = (Integral::new(&col.d), Integral::new(&col.c));
        let junction = bands.junction_end() as f64;
        let reach = rim + 1.0;
        let span = |c: f64| (c - reach).floor() as i32..(c + reach).ceil() as i32;
        for y in span(cy) {
            for x in span(cx) {
                let rho = (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy);
                let (ink, class) = if rho >= reach {
                    continue;
                } else if rho >= rim {
                    (WALL_INK, Class::Glass)
                } else if rho * scale < WELL {
                    (LIQUID_INK, Class::Liquid)
                } else {
                    // The depth window of radii [ρ − ½, ρ + ½] and its two halves.
                    let depth = |r: f64| (r * scale - WELL).clamp(0.0, view);
                    let (lo, mid, hi) = (depth(rho - 0.5), depth(rho), depth(rho + 0.5));
                    let pixel =
                        |lo: f64, hi: f64| gel_pixel(d.mean(lo, hi), c.mean(lo, hi), lo < junction);
                    let Some((_, class)) = pixel(lo, hi) else {
                        continue;
                    };
                    // The ink curve is steep at low d, so a pixel just grazing a ring would
                    // light fully: the ink is averaged over the two halves instead, and a
                    // gap under a pixel dims it rather than leaving dotted specks
                    // (LIES-A13).
                    let half = |lo: f64, hi: f64| pixel(lo, hi).map_or(0.0, |p| p.0);
                    (
                        ink::ramp(PALETTE, 0.5 * (half(lo, mid) + half(mid, hi))),
                        class,
                    )
                };
                self.pixels.set(x, y, ink);
                self.classes.set(x, y, class);
            }
        }
    }

    /// Row 0: each shown column's live ratio centred over it, and the legend at column 1
    /// while any ratio is shown.
    fn render_header(&self, canvas: &mut Canvas) {
        let lay = self.layout;
        if lay.height < 6 {
            return;
        }
        // (header column, column, nodes on screen) of each gauge.
        let mut gauges: [Option<(i32, &Tube, f64)>; 5] = [None; 5];
        match self.act() {
            Act::Rack if lay.inner >= 4 => {
                for (slot, preset) in lay.tubes() {
                    gauges[slot] = Some((lay.ratio_col(slot), &self.tubes[preset], lay.view));
                }
            }
            // The ratio (`1.30`) needs columns DISH_RATIO_COL..+4: a narrower header
            // would clip it to a number that misreads, so it shows whole or not at all.
            Act::Dish if lay.dish.rim >= 4.0 && i32::from(lay.width) >= DISH_RATIO_COL + 4 => {
                gauges[0] = Some((DISH_RATIO_COL, &self.dish, lay.dish.view));
            }
            _ => return,
        }
        let front = dissolve_front(self.schedule.at(self.t));
        let mut any = false;
        for &(col, tube, view) in gauges.iter().flatten() {
            let Some((r, end)) = live_ratio(tube.bands.as_slice(), &tube.col.nuc) else {
                continue;
            };
            // Once the dissolve front has passed the deeper measured band (or the view
            // edge), the number would describe bands that are gone.
            if (end as f64).min(view) <= front {
                continue;
            }
            any = true;
            let mut s = StackText::<8>::new();
            let _ = write!(s, "{r:.2}");
            canvas.text(col, 0, s.as_str(), RATIO_INK);
        }
        let first = gauges[0].map_or(0, |g| g.0);
        if let Some(text) = legend(first).filter(|_| any) {
            canvas.text(1, 0, text, LEGEND_INK);
        }
    }
}

impl Generator for LiesegangRings {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.act() as usize])
    }

    /// Only the layout changes: the chemistry is independent of the terminal. The first
    /// resize also opens the scene part-grown in an act drawn from `ctx.rng` (LIES-A11);
    /// no later resize touches the chemistry or draws from `ctx.rng` (LIES-A7).
    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        // The pixel classes are empty until the first resize.
        if self.classes.is_empty() {
            let act = self
                .start
                .unwrap_or_else(|| ACTS[ctx.rng.random_range(0..ACTS.len())]);
            self.t = act as usize as f64 * ACT_S + PREGROWN_S[act as usize];
            self.run = run_of(&self.schedule, self.t);
            self.advance();
            self.scan();
        }
        self.layout = Layout::new(size);
        self.windows.clear();
        self.windows.extend(self.layout.windows());
        self.pixels.resize(size);
        self.classes.resize(self.pixels.pixel_size(), Class::Empty);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.t += dt;
        let run = run_of(&self.schedule, self.t);
        if run != self.run {
            self.run = run;
            self.restart(&mut ctx.rng);
        }
        self.advance();
        let beat = self.schedule.at(self.t);
        if phase(beat) == DISSOLVE {
            // Only the part of this step spent in `dissolve` decays.
            let k = (-DISSOLVE_RATE * dt.min(beat.local)).exp();
            let front = dissolve_front(beat);
            for tube in self.columns(act(beat)) {
                tube.col.dissolve(front, k);
            }
        } else {
            self.since_scan += dt;
            if self.since_scan >= SCAN_EVERY {
                self.scan();
            }
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        let class = |x: i32, y: i32| self.classes.get(x, y).copied();
        self.pixels.blit(canvas, |x, y, top, bottom| {
            ascii_cell(class(x, 2 * y)?, class(x, 2 * y + 1)?, top, bottom)
        });
        let lay = self.layout;
        if self.act() == Act::Rack {
            // The walls stop above the title row (LIES-A10).
            for (slot, _) in lay.tubes() {
                let (left, right) = (lay.x0[slot] - 1, lay.x0[slot] + lay.inner);
                for y in 1..i32::from(lay.height) - 1 {
                    canvas.put(left, y, Glyph::new('▕', '|'), WALL_INK);
                    canvas.put(right, y, Glyph::new('▏', '|'), WALL_INK);
                }
            }
        }
        self.render_header(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Most column steps the opening catch-up may take (LIES-A11): rack 5 × 838, dish
    /// 2 370.
    const PREGROWN_STEPS: u32 = 4_500;

    fn run_column(a0: f64, t_end: f64) -> Column<NODES> {
        let mut col = Column::new(a0, B0);
        col.run_to(t_end);
        col
    }

    fn bands_of(col: &Column<NODES>) -> Bands {
        scan_bands(&col.d, &col.t_nuc)
    }

    fn mean(v: &[f64]) -> f64 {
        v.iter().sum::<f64>() / v.len() as f64
    }

    fn max_dev(v: &[f64]) -> f64 {
        let m = mean(v);
        v.iter().map(|x| (x / m - 1.0).abs()).fold(0.0, f64::max)
    }

    /// A scene opened in `act` (pre-grown, LIES-A11).
    fn scene_in(act: Act, size: Size, seed: u64) -> (LiesegangRings, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut s = LiesegangRings::new(Some(act));
        s.resize(size, &mut ctx);
        (s, ctx)
    }

    fn scene(size: Size, seed: u64) -> (LiesegangRings, Ctx) {
        scene_in(Act::Rack, size, seed)
    }

    /// Every column, the tubes then the dish.
    fn columns(s: &LiesegangRings) -> Vec<Tube> {
        s.tubes.iter().chain([&s.dish]).cloned().collect()
    }

    #[test]
    fn lies_a1_fields_stay_bounded_and_precipitate_is_irreversible() {
        let (mut s, mut ctx) = scene(Size::new(80, 24), 9);
        let mut dts = SplitMix64::new(11);
        let mut prev = columns(&s);
        while s.t < 2.0 * s.schedule.total() + 1.0 {
            let dt = 0.25 * dts.next_f64();
            let run = s.run;
            s.update(dt, &mut ctx);
            let growing = s.run == run && phase(s.schedule.at(s.t)) != DISSOLVE;
            for (tube, old) in columns(&s).iter().zip(&prev) {
                let col = &tube.col;
                for i in 0..NODES {
                    assert!(col.a[i].is_finite() && (0.0..=col.a0).contains(&col.a[i]));
                    assert!(col.b[i].is_finite() && (0.0..=col.b0).contains(&col.b[i]));
                    assert!(col.c[i].is_finite() && col.c[i] >= 0.0);
                    assert!(col.d[i].is_finite() && col.d[i] >= 0.0);
                    if growing {
                        assert!(col.d[i] >= old.col.d[i], "d decreased at node {i}");
                        if old.col.nuc[i] {
                            assert!(col.nuc[i] && col.t_nuc[i] == old.col.t_nuc[i]);
                        }
                    }
                }
            }
            prev = columns(&s);
        }
    }

    #[test]
    fn lies_a2_closed_column_conserves_stoichiometry() {
        let mut col = Column::<96>::new(1.0, B0);
        col.a[..40].fill(1.0);
        let (bcd0, ab0) = (96.0 * B0, 40.0 - 96.0 * B0);
        for _ in 0..10_000 {
            col.step(Ends::Closed);
        }
        let bcd: f64 = (0..96).map(|i| col.b[i] + col.c[i] + col.d[i]).sum();
        let ab: f64 = (0..96).map(|i| col.a[i] - col.b[i]).sum();
        assert!((bcd / bcd0 - 1.0).abs() < 1e-9, "Σ(b + c + d) = {bcd}");
        assert!((ab / ab0 - 1.0).abs() < 1e-9, "Σ(a − b) = {ab}");
        assert!(col.d.iter().sum::<f64>() > 0.0, "something precipitated");
    }

    #[test]
    fn lies_a3_spacing_and_width_laws() {
        let col = run_column(2.0, 1500.0);
        let bands = bands_of(&col);
        let b = bands.as_slice();
        assert!(b.len() >= 7 && b.len() < MAX_BANDS, "{b:?}");
        assert!(b.iter().all(|band| band.x <= 112.0), "{b:?}");
        let r: Vec<f64> = ratios(b).collect();
        let last5 = &r[r.len() - 5..];
        assert!(max_dev(last5) <= 0.04, "{r:?}");
        assert!((1.27..=1.35).contains(&mean(last5)), "{r:?}");
        // Width law: after the junction zone widths never shrink with depth, and the
        // deepest band is at least twice the first (reference 5, 5, 6, 7, 9, 10, 12).
        let width = |band: &Band| band.end - band.start;
        assert!(
            b[1..].windows(2).all(|w| width(&w[1]) >= width(&w[0])),
            "{b:?}"
        );
        assert!(width(&b[b.len() - 1]) >= 2 * width(&b[1]), "{b:?}");
    }

    #[test]
    fn lies_a4_time_law() {
        let col = run_column(2.0, 1500.0);
        let bands = bands_of(&col);
        let b = bands.as_slice();
        let law: Vec<f64> = b[b.len() - 4..]
            .iter()
            .map(|band| band.x / band.t_n.sqrt())
            .collect();
        assert!(max_dev(&law) <= 0.08, "{law:?}");
        assert!((2.65..=3.0).contains(&mean(&law)), "{law:?}");
    }

    #[test]
    fn lies_a5_matalon_packter_law() {
        let p: Vec<f64> = PRESETS
            .iter()
            .map(|&a0| {
                let bands = bands_of(&run_column(a0, 1400.0));
                let r: Vec<f64> = ratios(bands.as_slice()).collect();
                mean(&r[r.len() - 3..]) - 1.0
            })
            .collect();
        assert!(p.windows(2).all(|w| w[1] < w[0]), "{p:?}");
        let x: Vec<f64> = PRESETS.iter().map(|a0| B0 / a0).collect();
        let (mx, my) = (mean(&x), mean(&p));
        let sxy: f64 = x.iter().zip(&p).map(|(x, y)| (x - mx) * (y - my)).sum();
        let sxx: f64 = x.iter().map(|x| (x - mx).powi(2)).sum();
        let g = sxy / sxx;
        let f = my - g * mx;
        let ss_res: f64 = x.iter().zip(&p).map(|(x, y)| (y - f - g * x).powi(2)).sum();
        let ss_tot: f64 = p.iter().map(|y| (y - my).powi(2)).sum();
        let r2 = 1.0 - ss_res / ss_tot;
        assert!(r2 >= 0.98, "R² {r2} {p:?}");
        assert!((3.5..=5.5).contains(&g), "G {g}");
        assert!((0.12..=0.26).contains(&f), "F {f}");
    }

    fn view_mass(col: &Column<NODES>) -> f64 {
        col.d[..VIEW_NODES as usize].iter().sum()
    }

    #[test]
    fn lies_a6_loop_warp_dissolve_and_reset() {
        let tl = schedule();
        assert_eq!(tl.total(), 100.0);
        for (i, a) in ACTS.into_iter().enumerate() {
            let t0 = i as f64 * ACT_S;
            let beats = [0.0, 36.0, 42.0].map(|s| tl.at(t0 + s));
            assert_eq!(beats.map(phase), [GROW, HOLD, DISSOLVE]);
            assert_eq!(beats.map(act), [a; 3]);
            assert!((sim_time(beats[1]) - 1200.0).abs() < 1e-9);
            assert!((sim_time(beats[2]) - 1500.0).abs() < 1e-9);
            let mut prev = -1.0;
            for k in 0..42 * 60 {
                let t = sim_time(tl.at(t0 + f64::from(k) / 60.0));
                assert!(t > prev, "t_sim not increasing at step {k} of act {i}");
                prev = t;
            }
        }

        // Through each act's dissolve and into the next act, from the rack and from the
        // dish: the dissolve keeps a, b and the flags and clears the view, then the next
        // act starts from fresh gel.
        for opening in ACTS {
            let (mut s, mut ctx) = scene_in(opening, Size::new(80, 24), 5);
            while phase(s.schedule.at(s.t + SIM_DT)) != DISSOLVE {
                s.update(SIM_DT, &mut ctx);
            }
            s.update(SIM_DT, &mut ctx);
            let live = |s: &LiesegangRings| match opening {
                Act::Rack => s.tubes.to_vec(),
                Act::Dish => vec![s.dish.clone()],
            };
            let start = live(&s);
            let mut mass: Vec<f64> = start.iter().map(|t| view_mass(&t.col)).collect();
            let run = s.run;
            while run_of(&s.schedule, s.t + SIM_DT) == run {
                s.update(SIM_DT, &mut ctx);
                for ((tube, old), m) in live(&s).iter().zip(&start).zip(&mut mass) {
                    let col = &tube.col;
                    assert_eq!((col.a, col.b, col.nuc), (old.col.a, old.col.b, old.col.nuc));
                    let now = view_mass(col);
                    assert!(now <= *m, "{opening:?}: view mass grew during dissolve");
                    *m = now;
                }
            }
            for (tube, old) in live(&s).iter().zip(&start) {
                assert!(
                    view_mass(&tube.col) < 0.02 * view_mass(&old.col),
                    "{opening:?}"
                );
            }

            s.update(SIM_DT, &mut ctx);
            assert_eq!(s.run, run + 1);
            assert_ne!(s.act(), opening);
            let b0 = s.tubes[0].col.b0;
            assert!((b0 / B0 - 1.0).abs() <= B0_JITTER);
            for (tube, a0) in s.tubes.iter().zip(PRESETS) {
                assert_eq!(tube.col, Column::new(a0, b0), "reset state");
            }
            let fresh = Column::radial(DISH_A0, b0, WELL);
            // The dish may already have taken its first step of the new act.
            if s.act() == Act::Rack {
                assert_eq!(s.dish.col, fresh, "reset state");
            } else {
                assert_eq!((s.dish.col.d, s.dish.col.nuc), (fresh.d, fresh.nuc));
            }
        }
    }

    #[test]
    fn lies_a6_dissolve_leaves_the_gel_clear_before_the_next_act() {
        // The display must fade with the fields: no precipitate floor may keep dissolved
        // bands on screen until the reset cut.
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            for opening in ACTS {
                let (mut s, mut ctx) = scene_in(opening, size, 42);
                let until = opening as usize as f64 * ACT_S + 49.9;
                while s.t + SIM_DT < until {
                    s.update(SIM_DT, &mut ctx);
                }
                assert_eq!(phase(s.schedule.at(s.t)), DISSOLVE);
                assert_eq!(s.act(), opening);
                let px = s.pixels.pixel_size();
                for y in 0..i32::from(px.height()) {
                    for x in 0..i32::from(px.width()) {
                        let class = *s.classes.get(x, y).unwrap();
                        if matches!(class, Class::Liquid | Class::Glass) {
                            continue;
                        }
                        let ink = s.pixels.get(x, y).unwrap();
                        let at = format!("{size:?} {opening:?} ({x}, {y}): {ink:?}");
                        assert_eq!(class, Class::Empty, "{at}");
                        assert_eq!(ink, Ink::BLANK, "{at}");
                    }
                }
            }
        }
    }

    #[test]
    fn lies_a12_radial_column_conserves_weighted_mass_and_tube_is_unchanged() {
        // Closed dish column: diffusion conserves Σ r·f, the polar volume element.
        let mut col = Column::<96>::radial(1.0, B0, WELL);
        col.a[..40].fill(1.0);
        let r = |i: usize| WELL + (i + 1) as f64;
        let weighted = |col: &Column<96>, f: &dyn Fn(&Column<96>, usize) -> f64| {
            (0..96).map(|i| r(i) * f(col, i)).sum::<f64>()
        };
        let bcd = |c: &Column<96>, i: usize| c.b[i] + c.c[i] + c.d[i];
        let ab = |c: &Column<96>, i: usize| c.a[i] - c.b[i];
        let (bcd0, ab0) = (weighted(&col, &bcd), weighted(&col, &ab));
        for _ in 0..10_000 {
            col.step(Ends::Closed);
        }
        let (bcd1, ab1) = (weighted(&col, &bcd), weighted(&col, &ab));
        assert!(
            (bcd1 / bcd0 - 1.0).abs() < 1e-9,
            "Σ r(b + c + d) = {bcd1} vs {bcd0}"
        );
        assert!(
            (ab1 / ab0 - 1.0).abs() < 1e-9,
            "Σ r(a − b) = {ab1} vs {ab0}"
        );
        assert!(col.d.iter().sum::<f64>() > 0.0, "something precipitated");

        // A tube's radial term is exactly zero: the M10 3-point step, bit for bit.
        let mut tube = Column::<NODES>::new(2.0, B0);
        let mut plain = tube.clone();
        for _ in 0..2000 {
            tube.step(Ends::Gel);
            m10_step(&mut plain);
        }
        assert_eq!(tube, plain);

        // The dish column grows rings obeying Jablczynski's law in the plane.
        let mut dish = Column::<NODES>::radial(DISH_A0, B0, WELL);
        dish.run_to(1500.0);
        let bands = bands_of(&dish);
        let r: Vec<f64> = ratios(bands.as_slice()).collect();
        assert!(bands.as_slice().len() >= 6, "{:?}", bands.as_slice());
        let last4 = &r[r.len() - 4..];
        assert!(max_dev(last4) <= 0.04, "{r:?}");
        assert!((1.25..=1.35).contains(&mean(last4)), "{r:?}");
    }

    /// The M10 tube step, verbatim: the reference the radial form must reproduce at g = 0.
    fn m10_step(col: &mut Column<NODES>) {
        let (top_a, bottom) = (col.a0, (0.0, col.b0, 0.0));
        col.steps += 1;
        let now = col.time();
        let (mut al, mut bl, mut cl, mut nl) = (top_a, col.b[0], col.c[0], false);
        for i in 0..NODES {
            let (a, b, c, was) = (col.a[i], col.b[i], col.c[i], col.nuc[i]);
            let (ar, br, cr, nr) = if i + 1 < NODES {
                (col.a[i + 1], col.b[i + 1], col.c[i + 1], col.nuc[i + 1])
            } else {
                (bottom.0, bottom.1, bottom.2, false)
            };
            let r = K * a * b;
            let p = if was {
                LAMBDA * (c - C_S).max(0.0)
            } else {
                0.0
            };
            col.a[i] = a + TAU * ((al - 2.0 * a + ar) - r);
            col.b[i] = b + TAU * ((bl - 2.0 * b + br) - r);
            let c_new = c + TAU * (D_C * (cl - 2.0 * c + cr) + r - p);
            col.c[i] = c_new;
            col.d[i] += TAU * p;
            if !was && (c_new >= C_STAR || (c_new >= C_G && (nl || nr))) {
                col.nuc[i] = true;
                col.t_nuc[i] = now;
            }
            (al, bl, cl, nl) = (a, b, c, was);
        }
    }

    #[test]
    fn lies_a9_a10_cobalt_inks_below_the_pale_top_and_a_free_title_row() {
        // The densest bands stop at silver-blue, below Cobalt's near-white top (LIES-A9).
        let top = ink::ramp(Ramp::Cobalt, BAND_TOP).level;
        assert!(top < 230, "band top {top}");
        for opening in ACTS {
            for (w, h) in [(80u16, 24u16), (40, 12), (200, 50)] {
                let size = Size::new(w, h);
                let (mut s, mut ctx) = scene_in(opening, size, 42);
                for _ in 0..120 {
                    s.update(SIM_DT, &mut ctx);
                }
                assert_eq!(s.layout.gel_px, 2 * (i32::from(h) - 3).max(0));
                let px = s.pixels.pixel_size();
                for y in 0..i32::from(px.height()) {
                    for x in 0..i32::from(px.width()) {
                        let ink = s.pixels.get(x, y).unwrap();
                        let class = *s.classes.get(x, y).unwrap();
                        let want = match class {
                            Class::Empty => Role::Background,
                            Class::Liquid => Role::Wave,
                            Class::Glass => Role::Structure,
                            _ => Role::Ramp(Ramp::Cobalt),
                        };
                        assert_eq!(ink.role, want, "{opening:?} {w}x{h} ({x}, {y})");
                        if want == Role::Ramp(Ramp::Cobalt) {
                            assert!(ink.level <= top, "{opening:?} {w}x{h} ({x}, {y}): {ink:?}");
                        }
                        match class {
                            Class::Liquid => assert_eq!(ink, LIQUID_INK),
                            Class::Glass => assert_eq!(ink, WALL_INK),
                            _ => {}
                        }
                    }
                }
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                s.render(&mut canvas);
                let last = &canvas.to_lines()[usize::from(h) - 1];
                assert!(
                    last.chars().all(|c| c == ' '),
                    "{opening:?} {w}x{h}: {last:?}"
                );
            }
        }
    }

    #[test]
    fn lies_a11_opens_part_grown_in_either_act() {
        let lit = |s: &LiesegangRings, class: Class| {
            let px = s.pixels.pixel_size();
            (0..i32::from(px.height()))
                .flat_map(|y| (0..i32::from(px.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| *s.classes.get(x, y).unwrap() == class)
                .count()
        };
        // The rack: every visible tube has a junction cap and at least one band.
        let (s, _) = scene_in(Act::Rack, Size::new(80, 24), 42);
        for &preset in shown(80) {
            let bands = s.tubes[preset].bands;
            assert!(
                bands.as_slice().len() >= 2,
                "tube {preset}: {:?}",
                bands.as_slice()
            );
        }
        assert!(lit(&s, Class::Band) > 0);
        // The dish: the junction ring and a first ring.
        let (s, _) = scene_in(Act::Dish, Size::new(80, 24), 42);
        assert!(
            s.dish.bands.as_slice().len() >= 2,
            "{:?}",
            s.dish.bands.as_slice()
        );
        assert!(lit(&s, Class::Band) > 0);
        // A seeded opening draws its act: both occur, each at its pre-grown time, and
        // the catch-up stays within its step budget.
        let mut seen = [false; 2];
        for seed in 0..64 {
            let size = Size::new(80, 24);
            let mut ctx = Ctx::new(seed, size);
            let mut s = LiesegangRings::new(None);
            s.resize(size, &mut ctx);
            let act = s.act() as usize;
            assert_eq!(s.t, act as f64 * ACT_S + PREGROWN_S[act], "seed {seed}");
            seen[act] = true;
            let steps: u32 = columns(&s).iter().map(|t| t.col.steps).sum();
            assert!(steps <= PREGROWN_STEPS, "seed {seed}: {steps} column steps");
        }
        assert_eq!(seen, [true, true]);
    }

    #[test]
    fn lies_a13_dish_is_centred_with_a_rim_a_well_and_captions() {
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let size = Size::new(w, h);
            let (s, _) = scene_in(Act::Dish, size, 42);
            let dish = s.layout.dish;
            let rim = f64::from((i32::from(h) - 3).min(i32::from(w) / 2 - 2));
            assert_eq!(dish.rim, rim, "{w}x{h}");
            // At most 1.8 nodes per pixel of radius, so ring gaps stay a pixel wide.
            let view = (1.8 * rim - WELL).clamp(DISH_MIN_VIEW, VIEW_NODES);
            assert_eq!((dish.view, dish.scale), (view, (WELL + view) / rim));
            assert!(
                dish.scale <= 1.8 + 1e-9 || dish.view == DISH_MIN_VIEW,
                "{w}x{h}"
            );
            // The ratio sits right of the legend, clear of the rim's top arc.
            let row = header(&s);
            assert!(row.starts_with(" x(n+1)/x(n)  1."), "{w}x{h}: {row:?}");
            let px = s.pixels.pixel_size();
            let (mut x0, mut x1, mut y0, mut y1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let class = *s.classes.get(x, y).unwrap();
                    let rho = (f64::from(x) + 0.5 - dish.cx).hypot(f64::from(y) + 0.5 - dish.cy);
                    if class == Class::Glass {
                        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                        assert!((rim..rim + 1.0).contains(&rho), "rim at {rho}");
                    }
                    if rho * dish.scale < WELL {
                        assert_eq!(class, Class::Liquid, "well at ({x}, {y})");
                    }
                }
            }
            // Symmetric bounding box, clear of the header and the title rows.
            assert!(
                (x0 + x1 - (i32::from(w) - 1)).abs() <= 1,
                "{w}x{h}: x {x0}..{x1}"
            );
            assert!(
                (y0 + y1 - (2 * i32::from(h) - 1)).abs() <= 1,
                "{w}x{h}: y {y0}..{y1}"
            );
            assert!(
                y0 >= 2 && y1 < 2 * (i32::from(h) - 1),
                "{w}x{h}: y {y0}..{y1}"
            );
            assert_eq!(s.caption(), Some(CAPTIONS[Act::Dish as usize]));
        }
        let (s, _) = scene(Size::new(80, 24), 42);
        assert_eq!(s.caption(), Some(CAPTIONS[Act::Rack as usize]));
        // A narrow dish shows the legend with its whole ratio or neither: never a
        // clipped `1.` that misreads (the ratio needs columns 14..=17).
        for w in 12u16..=20 {
            let (s, _) = scene_in(Act::Dish, Size::new(w, 24), 42);
            let row = header(&s);
            let ratio = row.get(14..18).unwrap_or("");
            let whole = ratio.len() == 4 && ratio.parse::<f64>().is_ok_and(|r| r > 1.0);
            if w >= 18 {
                assert!(row.starts_with(" x(n+1)/x(n)  ") && whole, "w={w}: {row:?}");
            } else {
                assert_eq!(row.trim(), "", "w={w}: {row:?}");
            }
        }
    }

    #[test]
    fn lies_a8_meniscus_is_flat_on_short_terminals() {
        // Below 40 gel rows a 1 px sag only notches single pixels (SPEC-002 Resize).
        let sag = |h| Layout::new(Size::new(80, h)).sag;
        assert_eq!((sag(12), sag(21)), (0.0, 0.0));
        assert_eq!((sag(22), sag(39)), (1.0, 1.0));
        assert_eq!((sag(40), sag(50)), (2.0, 2.0));
    }

    #[test]
    fn lies_a8_ascii_cell_takes_the_ink_of_its_stronger_half() {
        let haze = Ink::new(Role::Ramp(PALETTE), 0);
        let band = Ink::new(Role::Ramp(PALETTE), 200);
        assert_eq!(
            ascii_cell(Class::Empty, Class::Empty, Ink::BLANK, Ink::BLANK),
            None
        );
        // A tie in level must not pick the blank pixel over the lit one.
        let faint = ascii_cell(Class::Empty, Class::Faint, Ink::BLANK, haze);
        assert_eq!(faint, Some(('.', haze)));
        // Liquid always shows as liquid, whatever sits below it.
        let meniscus = ascii_cell(Class::Liquid, Class::Faint, LIQUID_INK, band);
        assert_eq!(meniscus, Some(('~', LIQUID_INK)));
        assert_eq!(
            ascii_cell(Class::Faint, Class::Band, haze, band),
            Some(('_', band))
        );
        assert_eq!(
            ascii_cell(Class::Band, Class::Faint, band, haze),
            Some(('-', band))
        );
    }

    #[test]
    fn lies_a8_legend_fits_before_the_first_ratio() {
        // The header is row 0 and the walls start at row 1, so only the first tube's
        // ratio can collide with the legend: it keeps a 2-column gap before it.
        assert_eq!(legend(9), None);
        for room in 10..14 {
            assert_eq!(legend(room), Some("xn+1/xn"));
        }
        assert_eq!(legend(14), Some("x(n+1)/x(n)"));
        let room = |w, h| Layout::new(Size::new(w, h)).ratio_col(0);
        assert_eq!(legend(room(80, 24)), Some("x(n+1)/x(n)"));
        assert_eq!(legend(room(200, 50)), Some("x(n+1)/x(n)"));
        assert_eq!(legend(room(40, 12)), Some("x(n+1)/x(n)"));
    }

    fn header(s: &LiesegangRings) -> String {
        let size = Size::new(s.layout.width, s.layout.height);
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        s.render(&mut canvas);
        canvas.to_lines().swap_remove(0)
    }

    #[test]
    fn lies_a8_header_ratios_leave_with_their_bands() {
        let (mut s, mut ctx) = scene(Size::new(80, 24), 42);
        while s.t + SIM_DT < 41.9 {
            s.update(SIM_DT, &mut ctx);
        }
        let full = header(&s);
        assert!(full.contains("x(n+1)/x(n)"), "{full:?}");
        assert_eq!(
            full.matches('.').count(),
            3,
            "three ratios in hold: {full:?}"
        );
        // The dissolve front erases each tube's measured bands, and its ratio with them.
        while s.t + SIM_DT < 49.9 {
            s.update(SIM_DT, &mut ctx);
        }
        assert_eq!(header(&s).trim(), "", "numbers over clear tubes");
    }

    #[test]
    fn lies_a8_ascii_haze_is_lighter_than_precipitate() {
        use Class::{Band, Empty, Faint, Haze};
        let class = |d, c, junction| gel_pixel(d, c, junction).map(|p| p.1);
        // Clear gel carrying only the supersaturation haze is its own, lightest class.
        assert_eq!(class(0.0, 0.8 * C_STAR, false), Some(Haze));
        assert_eq!(class(0.0, 0.4 * C_STAR, false), None);
        assert_eq!(class(0.01, 0.8 * C_STAR, false), Some(Faint));
        assert_eq!(class(0.1, 0.0, true), Some(Faint), "junction zone");
        assert_eq!(class(0.1, 0.0, false), Some(Band));
        assert_eq!(ascii_glyph(Haze, Haze), '.');
        assert_eq!(ascii_glyph(Haze, Empty), '.');
        assert_eq!(ascii_glyph(Empty, Haze), '.');
        assert_eq!(ascii_glyph(Empty, Empty), ' ');
        assert_eq!(ascii_glyph(Faint, Faint), ':');
        assert_eq!(ascii_glyph(Faint, Haze), '.');
    }

    #[test]
    fn lies_a8_short_terminals_show_fewer_nodes() {
        // At most ≈ 2.4 nodes per pixel, so short terminals still resolve the gaps.
        let view = |w, h| Layout::new(Size::new(w, h)).view;
        assert_eq!((view(80, 24), view(200, 50)), (VIEW_NODES, VIEW_NODES));
        assert_eq!(view(40, 12), 50.0);
        assert_eq!(view(80, 16), 2.4 * 26.0);
        let deepest = Layout::new(Size::new(40, 12)).windows().last().flatten();
        assert!((deepest.unwrap().1 - 50.0).abs() < 1e-9, "{deepest:?}");
    }

    #[test]
    fn lies_a7_resizes_never_touch_the_chemistry() {
        let (mut a, mut ctx_a) = scene(Size::new(80, 24), 3);
        let (mut b, mut ctx_b) = scene(Size::new(80, 24), 3);
        let mut dts = SplitMix64::new(4);
        let sizes = [(1, 1), (7, 3), (80, 24), (200, 50)];
        let mut k = 0;
        while a.t < 55.0 {
            let dt = 0.25 * dts.next_f64();
            a.update(dt, &mut ctx_a);
            b.update(dt, &mut ctx_b);
            if a.t > 45.0 && k < sizes.len() {
                let size = Size::new(sizes[k].0, sizes[k].1);
                ctx_b.set_size(size);
                b.resize(size, &mut ctx_b);
                k += 1;
            }
        }
        assert_eq!((a.run, a.act()), (1, Act::Dish), "across the act boundary");
        for (x, y) in columns(&a).iter().zip(&columns(&b)) {
            assert_eq!(x.col, y.col);
        }
        assert_eq!(shown(20), &[2]);
        assert_eq!(shown(80), &[0, 2, 4]);
        assert_eq!(shown(200), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn lies_a8_live_ratio_waits_for_a_deeper_nucleus() {
        let band = |x: f64, start, end| Band {
            x,
            start,
            end,
            t_n: 0.0,
        };
        let b = [band(3.0, 0, 6), band(10.0, 8, 11), band(13.0, 12, 14)];
        let mut nuc = [false; 20];
        nuc[..6].fill(true);
        nuc[8..11].fill(true);
        nuc[12..14].fill(true);
        assert_eq!(
            live_ratio(&b, &nuc),
            None,
            "the deepest band is not complete"
        );
        nuc[17] = true;
        assert_eq!(
            live_ratio(&b, &nuc),
            Some((1.3, 14)),
            "ratio, deeper band's end"
        );
    }

    #[test]
    fn lies_a8_integral_is_an_exact_box_filter() {
        let mut f = [0.0; NODES];
        f[..3].copy_from_slice(&[1.0, 2.0, 4.0]);
        let box_filter = Integral::new(&f);
        assert_eq!(box_filter.mean(0.0, 3.0), 7.0 / 3.0);
        assert_eq!(box_filter.mean(0.5, 1.5), 1.5);
        assert_eq!(box_filter.mean(2.5, 3.5), 2.0);
        assert_eq!(
            box_filter.mean(3.0, 7.5),
            0.0,
            "zero field reads exactly zero"
        );
        assert_eq!(box_filter.mean(158.0, NODES as f64), 0.0);
    }
}
