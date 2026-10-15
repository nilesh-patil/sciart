//! Liesegang bands snapping into place down a rack of gel columns (SPEC-002
//! `liesegang_rings`).
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
//!
//! The chemistry never depends on the terminal: every tube is always simulated on 160
//! nodes (a capped lattice, ADR-0012); a resize only rebuilds the layout. The raster is
//! rebuilt in `update`/`resize`; `render` only blits it and writes the walls and header
//! (the M10 raster convention).

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::ink;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "liesegang_rings",
    category: Category::Chemistry,
    description: "Liesegang bands snapping into place down a rack of gel columns",
    title: "Liesegang bands · form in bursts",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(LiesegangRings::new())
}

/// Nodes per column; node `i` sits at depth `x = i + 1`, the reservoir ghost at `x = 0`.
pub(crate) const NODES: usize = 160;
/// Nodes shown on screen, `[0, VIEW_NODES)`, once the gel has ≥ 42 pixel rows.
const VIEW_NODES: f64 = 100.0;
/// Most nodes per gel pixel: shorter terminals show fewer nodes (never below
/// [`MIN_VIEW_NODES`]) rather than merge bands and gaps into one striped column.
const NODES_PER_PX: f64 = 2.4;
const MIN_VIEW_NODES: f64 = 50.0;

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

/// The 50 s loop (LIES-A6); indices match [`sim_time`].
const PHASES: [(&str, f64); 3] = [("grow", 36.0), ("hold", 6.0), ("dissolve", 8.0)];
const GROW: usize = 0;
const HOLD: usize = 1;
const DISSOLVE: usize = 2;
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
const JUNCTION_DIM: f64 = 0.45;
const HAZE: f64 = 0.25;
const LIQUID_INK: Ink = Ink::new(Role::Wave, 60);
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
    /// Whole Euler steps taken this cycle.
    steps: u32,
}

impl<const N: usize> Column<N> {
    /// A fresh column: gel loaded with `b0`, no A, C, D or nuclei. This is the reset
    /// state `(0, b₀,c, 0, 0, false, ∞)` (LIES-A6).
    pub(crate) fn new(a0: f64, b0: f64) -> Self {
        Self {
            a: [0.0; N],
            b: [b0; N],
            c: [0.0; N],
            d: [0.0; N],
            nuc: [false; N],
            t_nuc: [f64::INFINITY; N],
            a0,
            b0,
            steps: 0,
        }
    }

    /// Back to the reset state with a new `b₀,c` (LIES-A6).
    pub(crate) fn reset(&mut self, b0: f64) {
        *self = Self::new(self.a0, b0);
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
    pub(crate) fn step(&mut self, ends: Ends) {
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
            let r = K * a * b;
            let p = if was {
                LAMBDA * (c - C_S).max(0.0)
            } else {
                0.0
            };
            self.a[i] = a + TAU * ((al - 2.0 * a + ar) - r);
            self.b[i] = b + TAU * ((bl - 2.0 * b + br) - r);
            let c_new = c + TAU * (D_C * (cl - 2.0 * c + cr) + r - p);
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
    if beat.index == DISSOLVE {
        FRONT_NODES * beat.progress.sqrt()
    } else {
        0.0
    }
}

/// Sim time at a point of the loop: `T_END·(s/36)^{3/2}` through `grow`, then on at its
/// final rate through `hold`, frozen at 1500 during `dissolve` (LIES-A6).
pub(crate) fn sim_time(beat: Beat) -> f64 {
    match beat.index {
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
    /// Gel pixel rows, from pixel row 4 (cell row 2) to the bottom edge.
    gel_px: i32,
    /// Nodes shown, `[0, view)`: `clamp(2.4·gel_px, 50, 100)`.
    view: f64,
    /// Meniscus depth at the tube axis, pixels: 0 below 40 gel rows (h < 22), where
    /// a pixel spans several nodes and a 1 px sag only notches single pixels.
    sag: f64,
}

impl Layout {
    fn new(size: Size) -> Self {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let n = tube_count(size.width()) as i32;
        let inner = ((f64::from(w) / (2.6 * f64::from(n))).round() as i32).clamp(1, 16);
        let pitch = inner + 2;
        let gap = ((w - n * pitch) / (n + 1)).max(0);
        let left = (w - (n * pitch + (n - 1) * gap)) / 2;
        let mut x0 = [0; 5];
        for (k, x) in x0.iter_mut().enumerate().take(n as usize) {
            *x = left + k as i32 * (pitch + gap) + 1;
        }
        let gel_px = 2 * (h - 2).max(0);
        let view = (NODES_PER_PX * f64::from(gel_px)).clamp(MIN_VIEW_NODES, VIEW_NODES);
        // h ≥ 22 is gel_px ≥ 40.
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

/// The Seq level and ASCII class of a gel pixel from its box-filtered fields (SPEC-002
/// Inks and ASCII), before cylinder shading; `None` leaves clear gel to the terminal's
/// background (SPEC-002 Inks), so depletion gaps read as dark.
///
/// Precipitate fades in linearly over `d ∈ [0, D_VIS]`, so the level is continuous in d:
/// a newborn band is a thin faint line that brightens, and a dissolving band fades all
/// the way out instead of freezing at a floor until the loop cuts.
pub(crate) fn gel_pixel(d: f64, c: f64, junction: bool) -> Option<(f64, Class)> {
    // x^0.75 as √x·⁴√x: two square roots are several times cheaper than `powf`.
    let x = (d / D_FULL).clamp(0.0, 1.0).sqrt();
    let mut precip = (0.30 + 0.70 * x * x.sqrt()) * (d / D_VIS).clamp(0.0, 1.0);
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
    use Class::{Band, Empty, Faint, Liquid};
    match (top, bottom) {
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
/// roles (Wave vs Seq) and could pick a blank pixel over a lit one of equal level.
fn ascii_cell(ct: Class, cb: Class, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    match ascii_glyph(ct, cb) {
        ' ' => None,
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
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    schedule: Timeline,
    /// Scene time, s.
    t: f64,
    /// Loop count at `t`; a change resets the tubes.
    cycle: i64,
    /// Time since the last band scan, s.
    since_scan: f64,
    tubes: [Tube; 5],
    layout: Layout,
    /// [`Layout::windows`], cached on resize.
    windows: Vec<Option<(f64, f64)>>,
    pixels: Pixels,
    /// ASCII class per pixel, same layout as `pixels`.
    classes: Grid<Class>,
}

impl LiesegangRings {
    fn new() -> Self {
        let tube = |a0| Tube {
            col: Column::new(a0, B0),
            bands: Bands::default(),
        };
        Self {
            dirty: false,
            schedule: schedule(),
            t: 0.0,
            cycle: 0,
            since_scan: 0.0,
            tubes: PRESETS.map(tube),
            layout: Layout::default(),
            windows: Vec::new(),
            pixels: Pixels::default(),
            classes: Grid::default(),
        }
    }

    fn scan(&mut self) {
        for tube in &mut self.tubes {
            tube.bands = scan_bands(&tube.col.d, &tube.col.t_nuc);
        }
        self.since_scan = 0.0;
    }

    /// A new cycle: fresh gel at a slightly different `b₀` (LIES-A6).
    fn restart(&mut self, rng: &mut impl Rng) {
        let xi: f64 = rng.random_range(-1.0..=1.0);
        let b0 = B0 * (1.0 + B0_JITTER * xi);
        for tube in &mut self.tubes {
            tube.col.reset(b0);
        }
        self.scan();
    }

    /// Paint the liquid row and every visible gel pixel.
    fn rasterise(&mut self) {
        let Self {
            tubes,
            layout: lay,
            windows,
            pixels,
            classes,
            ..
        } = self;
        pixels.clear();
        classes.fill(Class::Empty);
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
                        set(x, y, ink::seq(v * shade), class);
                    }
                }
            }
        }
    }

    fn render_header(&self, canvas: &mut Canvas) {
        let lay = self.layout;
        if lay.height < 6 || lay.inner < 4 {
            return;
        }
        let front = dissolve_front(self.schedule.at(self.t));
        let mut any = false;
        for (slot, preset) in lay.tubes() {
            let tube = &self.tubes[preset];
            let Some((r, end)) = live_ratio(tube.bands.as_slice(), &tube.col.nuc) else {
                continue;
            };
            // Once the dissolve front has passed the deeper measured band (or the view
            // bottom), the number would describe bands that are gone.
            if (end as f64).min(lay.view) <= front {
                continue;
            }
            any = true;
            let mut s = StackText::<8>::new();
            let _ = write!(s, "{r:.2}");
            canvas.text(lay.ratio_col(slot), 0, s.as_str(), RATIO_INK);
        }
        if let Some(text) = legend(lay.ratio_col(0)).filter(|_| any) {
            canvas.text(1, 0, text, LEGEND_INK);
        }
    }
}

impl Generator for LiesegangRings {
    fn meta(&self) -> &'static Meta {
        &META
    }

    /// Only the layout changes: the chemistry is independent of the terminal and a
    /// resize never draws from `ctx.rng` (LIES-A7).
    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
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
        let cycle = self.schedule.cycle(self.t);
        if cycle != self.cycle {
            self.cycle = cycle;
            self.restart(&mut ctx.rng);
        }
        let beat = self.schedule.at(self.t);
        let t_sim = sim_time(beat);
        for tube in &mut self.tubes {
            tube.col.run_to(t_sim);
        }
        if beat.index == DISSOLVE {
            // Only the part of this step spent in `dissolve` decays.
            let k = (-DISSOLVE_RATE * dt.min(beat.local)).exp();
            let front = dissolve_front(beat);
            for tube in &mut self.tubes {
                tube.col.dissolve(front, k);
            }
        } else {
            self.since_scan += dt;
            if self.since_scan >= SCAN_EVERY {
                self.scan();
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
        let class = |x: i32, y: i32| self.classes.get(x, y).copied();
        self.pixels.blit(canvas, |x, y, top, bottom| {
            ascii_cell(class(x, 2 * y)?, class(x, 2 * y + 1)?, top, bottom)
        });
        let lay = self.layout;
        for (slot, _) in lay.tubes() {
            let (left, right) = (lay.x0[slot] - 1, lay.x0[slot] + lay.inner);
            for y in 1..i32::from(lay.height) {
                canvas.put(left, y, Glyph::new('▕', '|'), WALL_INK);
                canvas.put(right, y, Glyph::new('▏', '|'), WALL_INK);
            }
        }
        self.render_header(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn scene(size: Size, seed: u64) -> (LiesegangRings, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut s = LiesegangRings::new();
        s.resize(size, &mut ctx);
        (s, ctx)
    }

    #[test]
    fn lies_a1_fields_stay_bounded_and_precipitate_is_irreversible() {
        let (mut s, mut ctx) = scene(Size::new(80, 24), 9);
        let mut dts = SplitMix64::new(11);
        let mut prev = s.tubes.clone();
        while s.t < 2.0 * s.schedule.total() + 1.0 {
            let dt = 0.25 * dts.next_f64();
            let cycle = s.cycle;
            s.update(dt, &mut ctx);
            s.prepare();
            let growing = s.cycle == cycle && s.schedule.at(s.t).index != DISSOLVE;
            for (tube, old) in s.tubes.iter().zip(&prev) {
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
            prev = s.tubes.clone();
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
        assert_eq!(tl.total(), 50.0);
        assert_eq!(
            (tl.at(0.0).index, tl.at(36.0).index, tl.at(42.0).index),
            (0, 1, 2)
        );
        assert!((sim_time(tl.at(36.0)) - 1200.0).abs() < 1e-9);
        assert!((sim_time(tl.at(42.0)) - 1500.0).abs() < 1e-9);
        let mut prev = -1.0;
        for i in 0..42 * 60 {
            let t = sim_time(tl.at(f64::from(i) / 60.0));
            assert!(t > prev, "t_sim not increasing at step {i}");
            prev = t;
        }

        let (mut s, mut ctx) = scene(Size::new(80, 24), 5);
        while s.schedule.at(s.t + SIM_DT).index != DISSOLVE {
            s.update(SIM_DT, &mut ctx);
            s.prepare();
        }
        s.update(SIM_DT, &mut ctx);
        s.prepare();
        let start = s.tubes.clone();
        let mut mass: Vec<f64> = start.iter().map(|t| view_mass(&t.col)).collect();
        while s.schedule.cycle(s.t + SIM_DT) == 0 {
            s.update(SIM_DT, &mut ctx);
            s.prepare();
            for ((tube, old), m) in s.tubes.iter().zip(&start).zip(&mut mass) {
                let col = &tube.col;
                assert_eq!((col.a, col.b, col.nuc), (old.col.a, old.col.b, old.col.nuc));
                let now = view_mass(col);
                assert!(now <= *m, "view mass grew during dissolve");
                *m = now;
            }
        }
        for (tube, old) in s.tubes.iter().zip(&start) {
            assert!(view_mass(&tube.col) < 0.02 * view_mass(&old.col));
        }

        s.update(SIM_DT, &mut ctx);
        s.prepare();
        assert_eq!(s.cycle, 1);
        let b0 = s.tubes[0].col.b0;
        assert!((b0 / B0 - 1.0).abs() <= B0_JITTER);
        for (tube, a0) in s.tubes.iter().zip(PRESETS) {
            assert_eq!(tube.col, Column::new(a0, b0), "reset state");
        }
    }

    #[test]
    fn lies_a6_dissolve_leaves_the_tubes_clear_before_the_loop() {
        // The display must fade with the fields: no precipitate floor may keep dissolved
        // bands on screen until the reset cut.
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let (mut s, mut ctx) = scene(size, 42);
            while s.t + SIM_DT < 49.9 {
                s.update(SIM_DT, &mut ctx);
                s.prepare();
            }
            assert_eq!(s.schedule.at(s.t).index, DISSOLVE);
            let px = s.pixels.pixel_size();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let class = *s.classes.get(x, y).unwrap();
                    if class == Class::Liquid {
                        continue;
                    }
                    let ink = s.pixels.get(x, y).unwrap();
                    assert_eq!(class, Class::Empty, "{size:?} ({x}, {y}): {ink:?}");
                    assert!(
                        ink.role != Role::Seq || ink.level <= 10,
                        "{size:?} ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn lies_meniscus_is_flat_on_short_terminals() {
        // Below 40 gel rows a 1 px sag only notches single pixels (SPEC-002 Resize).
        let sag = |h| Layout::new(Size::new(80, h)).sag;
        assert_eq!((sag(12), sag(21)), (0.0, 0.0));
        assert_eq!((sag(22), sag(39)), (1.0, 1.0));
        assert_eq!((sag(40), sag(50)), (2.0, 2.0));
    }

    #[test]
    fn lies_ascii_cell_takes_the_ink_of_its_stronger_half() {
        let haze = Ink::new(Role::Seq, 0);
        let band = Ink::new(Role::Seq, 200);
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
            s.prepare();
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
            s.prepare();
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
        assert_eq!(view(80, 16), 2.4 * 28.0);
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
            a.prepare();
            b.update(dt, &mut ctx_b);
            b.prepare();
            if a.t > 45.0 && k < sizes.len() {
                let size = Size::new(sizes[k].0, sizes[k].1);
                ctx_b.set_size(size);
                b.resize(size, &mut ctx_b);
                k += 1;
            }
        }
        assert_eq!(a.cycle, 1);
        for (x, y) in a.tubes.iter().zip(&b.tubes) {
            assert_eq!(x.col, y.col);
        }
        assert_eq!(shown(20), &[2]);
        assert_eq!(shown(80), &[0, 2, 4]);
        assert_eq!(shown(200), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn lies_live_ratio_waits_for_a_deeper_nucleus() {
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
    fn lies_integral_is_an_exact_box_filter() {
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
