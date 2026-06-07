//! The Ising ferromagnet quenched, reheated and cooled through its critical point, then
//! a conserved-spin act in which an even mixture un-mixes (SPEC-002 `ising_model`).
//!
//! Physics: the 2-D Ising model `H = −J Σ⟨ij⟩ sᵢsⱼ` on a torus, `J = k_B = 1`, zero field.
//! Act one evolves it by single-spin Glauber heat-bath moves in random-sequential order
//! (Glauber 1963). Random-sequential rather than checkerboard: sublattice sweeps flip every
//! `ΔE = 0` spin together at high T (a period-2 strobe), while random attempts give smooth
//! fractional sweeps per frame at any lattice size. T/T_c follows a looping script:
//! quench-and-coarsen (Allen–Cahn `ℓ ∝ t^{1/2}`), reheat, a hot boil, a slow cool into
//! T_c, a critical hold and a quench back. Act two scrambles the spins into an exact
//! 50/50 mixture and evolves it at 0.6 T_c by Kawasaki spin swaps (Kawasaki 1966), which
//! conserve `M`: the model of a binary alloy, it un-mixes into a labyrinth that coarsens
//! as `ℓ ∝ t^{1/3}` (spinodal decomposition).
//!
//! The lattice is capped at 16 384 spins (`grid::lattice_for`, ADR-0012) and shown as one
//! half-block pixel per spin at or below the cap, nearest-sampled above it (the lattice
//! is the art). On black (ADR-0017): down spins are the terminal's black, up spins a dim
//! neutral grey (`Role::Text`, the one neutral grey ramp; Starlight read khaki), and the domain walls, up spins with a down
//! neighbour, a 1-px brighter line that is brightest along straight walls (three aligned
//! neighbours) and dim on ragged ones, so ordering reads as clean pale contours and the
//! hot boil as a quiet grey haze. Thermal specks (clusters under [`SPECK`] spins) are
//! quiet flecks that draw no wall. The raster is rebuilt in `prepare` (GEN-R20); `render`
//! only blits it and writes the label.

use std::fmt::Write as _;

use sciart_core::ease;
use sciart_core::grid::{lattice_for, regrid_nearest};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "ising_model",
    category: Category::PhysicalChemistry,
    description: "The Ising ferromagnet quenched, reheated and cooled through its critical point",
    title: "Magnetic domains · spins align",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(IsingModel::new())
}

/// Onsager's critical temperature `2 / ln(1 + √2)` in units of J/k_B (Onsager 1944).
pub(crate) const T_C: f64 = 2.269_185_314_213_022;

/// Spin cap of the lattice and the aspect bucket of `lattice_for` (ADR-0012).
const MAX_SPINS: usize = 16_384;
const BUCKET: f64 = 1.25;
/// Monte Carlo sweeps per simulated second at the cap: one MCS per 1/60 s step.
const MCS_PER_S_AT_CAP: f64 = 60.0;
/// Temperature clamp for the heat-bath table; outside it the table is already 0/½/1.
const T_MIN: f64 = 0.05;
const T_MAX: f64 = 100.0;

/// The 90 s schedule of T/T_c (ISING-A6); indices match [`t_over_tc`]. The first six
/// phases are the Glauber act; `spinodal` is the Kawasaki act (ISING-A7).
const PHASES: [(&str, f64); 7] = [
    ("coarsen", 20.0),
    ("reheat", 8.0),
    ("hot", 5.0),
    ("cool", 15.0),
    ("critical", 8.0),
    ("quench", 2.0),
    ("spinodal", 32.0),
];
/// Index of the Kawasaki phase in [`PHASES`].
const SPINODAL: usize = 6;
const COLD: f64 = 0.60;
const HOT: f64 = 1.25;
/// The two acts' captions (GEN-R12): single-spin flips order the magnet; spin swaps keep
/// the total fixed, so the mixture un-mixes instead (spinodal decomposition).
const CAPTIONS: [&str; 2] = ["spins align", "spins swap"];
/// Kawasaki sweeps run up to this many times faster than Glauber ones: conserved
/// coarsening (`ℓ ∝ t^{1/3}`) is much slower than non-conserved (`t^{1/2}`), and 2× keeps
/// the labyrinth visibly ripening within the act; the swaps make this the scene's
/// heaviest phase (SPEC-002 `ising_model` Perf, GEN-R8). The speed-up ramps up from 1×
/// over the act's first `SWAP_RAMP` s ([`swap_speedup`]): a fresh mixture de-mixes within a few sweeps
/// anyway, and there nearly every attempt is an antiparallel pair, the costly case.
const SWAP_SPEEDUP: f64 = 2.0;
const SWAP_RAMP: f64 = 4.0;

/// ASCII ramp over the cell-mean signed value, `−1 → '.'` to `+1 → '#'`.
const ASCII_RAMP: [Glyph; 5] = [
    Glyph::ascii('.'),
    Glyph::ascii(','),
    Glyph::ascii(':'),
    Glyph::ascii('+'),
    Glyph::ascii('#'),
];
const LABEL_INK: Ink = Ink::new(Role::Text, 110);
/// Up spins' role (a neutral grey, false colour): the interior level, and the wall
/// levels by aligned-neighbour count 0–3 (ADR-0017: the walls carry the light, straight
/// walls brightest). Down spins are black.
const UP_ROLE: Role = Role::Text;
const INTERIOR_LEVEL: u8 = 20;
const WALL_LEVELS: [u8; 4] = [8, 14, 110, 200];

/// Heat-bath probability that a spin with local field `h = Σ_nn sⱼ` becomes +1 at
/// temperature `t`: `p₊ = 1 / (1 + e^{−2h/T})` (ISING-A1). `t` is clamped to
/// `[T_MIN, T_MAX]` and NaN means T_c, so the result is always a probability.
pub(crate) fn heat_bath_p(h: f64, t: f64) -> f64 {
    let t = if t.is_nan() {
        T_C
    } else {
        t.clamp(T_MIN, T_MAX)
    };
    1.0 / (1.0 + (-2.0 * h / t).exp())
}

/// The heat-bath acceptance table for `h ∈ {−4, −2, 0, 2, 4}`: `round(p₊·2³²)`, compared
/// against the low 32 bits of a draw, so `p = 0` and `p = 1` are exact (ISING-A1).
pub(crate) fn thresholds(t: f64) -> [u64; 5] {
    std::array::from_fn(|i| {
        let h = 2.0 * i as f64 - 4.0;
        (heat_bath_p(h, t) * 4_294_967_296.0).round() as u64
    })
}

/// Monte Carlo sweeps per simulated second for an `n`-spin lattice: one per 1/60 s step
/// at the cap, proportionally fewer below it, so the domain size relative to the lattice
/// (`∝ √(MCS / N)`) grows at the same pace on every terminal (ISING-A5).
pub(crate) fn sweep_rate(n: usize) -> f64 {
    (MCS_PER_S_AT_CAP * n as f64 / MAX_SPINS as f64).max(1.0)
}

/// The looping temperature schedule (ISING-A6).
fn schedule() -> Timeline {
    Timeline::new(&PHASES)
}

/// T/T_c for a point of the schedule, continuous at every phase boundary (ISING-A6).
pub(crate) fn t_over_tc(beat: Beat) -> f64 {
    let p = beat.progress;
    match beat.index {
        0 => COLD,
        1 => ease::lerp(COLD, HOT, ease::smoothstep(p)),
        2 => HOT,
        // Eases into T_c so most of the phase sits near criticality.
        3 => 1.0 + (HOT - 1.0) * (1.0 - p) * (1.0 - p),
        4 => 1.0,
        5 => ease::lerp(1.0, COLD, ease::smoothstep(p)),
        _ => COLD,
    }
}

/// Heat-bath acceptance of a Kawasaki swap of two antiparallel neighbours with energy
/// change `ΔE = 4k − 12`, `k ∈ 0..7`: `p = 1/(1 + e^{ΔE/T})` as `round(p·2³⁰)` against 30
/// random bits (ISING-A7). Same clamps as [`heat_bath_p`], which it is with `h = −ΔE/2`.
pub(crate) fn swap_thresholds(t: f64) -> [u64; 7] {
    std::array::from_fn(|k| {
        let de = 4.0 * k as f64 - 12.0;
        (heat_bath_p(-de / 2.0, t) * f64::from(1u32 << 30)).round() as u64
    })
}

/// Kawasaki sweep rate relative to Glauber `local` seconds into the act: 1× at its start,
/// rising linearly to [`SWAP_SPEEDUP`] at [`SWAP_RAMP`] s (ISING-A7).
pub(crate) fn swap_speedup(local: f64) -> f64 {
    1.0 + (SWAP_SPEEDUP - 1.0) * (local / SWAP_RAMP).clamp(0.0, 1.0)
}

/// The corner readout, `T/Tc 0.93  |m| 0.71`, or `None` where it does not fit (the `|m|`
/// part needs 24 columns, the rest 12 columns and 4 rows) (ISING-A6). With `m_abs = None`
/// (the Kawasaki act, where swaps pin M) the second part reads `M fixed`: a rule, not a
/// measurement (ISING-A9).
pub(crate) fn label(t_rel: f64, m_abs: Option<f64>, size: Size) -> Option<StackText<24>> {
    if size.width() < 12 || size.height() < 4 {
        return None;
    }
    let mut s = StackText::new();
    let _ = write!(s, "T/Tc {t_rel:.2}");
    if size.width() >= 24 {
        let _ = match m_abs {
            Some(m) => write!(s, "  |m| {m:.2}"),
            None => write!(s, "  M fixed"),
        };
    }
    Some(s)
}

/// The depth [`Lattice::classify`] seeds interior sites with (only its classes are drawn).
const DEPTH_CAP: u8 = 20;
/// Same-sign clusters smaller than this are thermal specks: they draw no wall, so a few
/// flipped spins never outline a hole in a big domain.
const SPECK: usize = 6;
/// A speck's shade class: drawn as an interior spin, a quiet fleck of its own colour.
const SPECK_CLASS: u8 = 4;
/// [`Lattice::classify`]'s cluster marks.
const UNSEEN: u8 = 0;
const SMALL: u8 = 1;
const LARGE: u8 = 2;
/// [`Lattice::classify`]'s class for a site that is not on a wall.
const WALL_INTERIOR: u8 = u8::MAX;

/// The ink of a site with spin `s` and shade class `class`: black for down spins, and for
/// up spins a wall level by aligned count (`0..4`) or the interior level.
pub(crate) fn site_ink(s: i8, class: u8) -> Ink {
    if s <= 0 {
        return Ink::BLANK;
    }
    match class {
        c @ 0..4 => Ink::new(UP_ROLE, WALL_LEVELS[usize::from(c)]),
        _ => Ink::new(UP_ROLE, INTERIOR_LEVEL),
    }
}

#[derive(Debug, Default)]
pub(crate) struct Lattice {
    spins: Grid<i8>,
    /// Left and right neighbour columns of each column.
    xl: Vec<u16>,
    xr: Vec<u16>,
    /// Offsets `row · w` of the rows above and below each row, so the hot loop has no
    /// modulo.
    yu: Vec<u32>,
    yd: Vec<u32>,
    /// `Σ s`, kept incrementally by [`Lattice::attempts`].
    m: i32,
}

impl Lattice {
    /// A lattice holding `spins`, with fresh tables and magnetisation.
    pub(crate) fn new(spins: Grid<i8>) -> Self {
        let mut lattice = Self {
            spins,
            ..Self::default()
        };
        lattice.rebuild();
        lattice
    }

    /// Independent ±1 spins: the T = ∞ state a quench starts from.
    pub(crate) fn random(size: Size, rng: &mut SplitMix64) -> Self {
        let mut lattice = Self::new(Grid::new(size, 1i8));
        lattice.randomise(rng);
        lattice
    }

    /// Refill in place with independent ±1 spins: a quench from T = ∞, no allocation.
    pub(crate) fn randomise(&mut self, rng: &mut SplitMix64) {
        for s in self.spins.as_mut_slice() {
            *s = if rng.next_u64() >> 63 == 1 { 1 } else { -1 };
        }
        self.m = self.spins.as_slice().iter().map(|&s| i32::from(s)).sum();
    }

    /// Recompute the neighbour tables and `M` after the spins were replaced.
    fn rebuild(&mut self) {
        let (w, h) = (
            usize::from(self.spins.width()),
            usize::from(self.spins.height()),
        );
        // Grid axes are u16, so column indices fit a u16 and row offsets (< 2¹⁶·2¹⁶) a
        // u32: the narrowing casts are lossless.
        self.xl = (0..w).map(|x| ((x + w - 1) % w) as u16).collect();
        self.xr = (0..w).map(|x| ((x + 1) % w) as u16).collect();
        self.yu = (0..h).map(|y| (((y + h - 1) % h) * w) as u32).collect();
        self.yd = (0..h).map(|y| (((y + 1) % h) * w) as u32).collect();
        self.m = self.spins.as_slice().iter().map(|&s| i32::from(s)).sum();
    }

    pub(crate) fn size(&self) -> Size {
        self.spins.size()
    }

    pub(crate) fn len(&self) -> usize {
        self.spins.len()
    }

    #[cfg(test)]
    pub(crate) fn spins(&self) -> &Grid<i8> {
        &self.spins
    }

    /// The tracked magnetisation `M = Σ s`.
    pub(crate) fn magnetisation(&self) -> i32 {
        self.m
    }

    /// `n` random-sequential heat-bath attempts, one `u64` each: bits 32–47 and 48–63
    /// pick the site by multiply-shift, the low 32 bits decide the new spin against
    /// `thr[h]` (SPEC-002 Model).
    pub(crate) fn attempts(&mut self, n: u64, thr: &[u64; 5], rng: &mut SplitMix64) {
        let Self {
            spins,
            xl,
            xr,
            yu,
            yd,
            m,
        } = self;
        let (w, h) = (u64::from(spins.width()), u64::from(spins.height()));
        let s = spins.as_mut_slice();
        let mut dm = 0i32;
        for _ in 0..n {
            let r = rng.next_u64();
            let x = ((((r >> 32) & 0xFFFF) * w) >> 16) as usize;
            let y = (((r >> 48) * h) >> 16) as usize;
            let row = y * w as usize;
            let field = s[row + usize::from(xl[x])]
                + s[row + usize::from(xr[x])]
                + s[yu[y] as usize + x]
                + s[yd[y] as usize + x];
            let new = if r & 0xFFFF_FFFF < thr[((field + 4) >> 1) as usize] {
                1
            } else {
                -1
            };
            dm += i32::from(new - s[row + x]);
            s[row + x] = new;
        }
        *m += dm;
    }

    /// `n` random-sequential Kawasaki attempts (Kawasaki 1966), one `u64` each: bits 32–47
    /// and 48–63 pick the site as in [`Lattice::attempts`], bits 30–31 one of its four
    /// neighbours, and an antiparallel pair swaps when the low 30 bits are below
    /// `thr[(ΔE + 12)/4]` ([`swap_thresholds`]). A swap never changes `M` (ISING-A7).
    pub(crate) fn swaps(&mut self, n: u64, thr: &[u64; 7], rng: &mut SplitMix64) {
        let Self {
            spins,
            xl,
            xr,
            yu,
            yd,
            ..
        } = self;
        let (w, h) = (usize::from(spins.width()), usize::from(spins.height()));
        let s = spins.as_mut_slice();
        // The local field at `(x, y)`, from the wrapped-neighbour tables.
        let field = |s: &[i8], x: usize, y: usize| {
            let row = y * w;
            i32::from(s[row + usize::from(xl[x])])
                + i32::from(s[row + usize::from(xr[x])])
                + i32::from(s[yu[y] as usize + x])
                + i32::from(s[yd[y] as usize + x])
        };
        for _ in 0..n {
            let r = rng.next_u64();
            let x = ((((r >> 32) & 0xFFFF) * w as u64) >> 16) as usize;
            let y = (((r >> 48) * h as u64) >> 16) as usize;
            let (xj, yj) = match (r >> 30) & 3 {
                0 => (usize::from(xl[x]), y),
                1 => (usize::from(xr[x]), y),
                2 => (x, if y == 0 { h - 1 } else { y - 1 }),
                _ => (x, if y + 1 == h { 0 } else { y + 1 }),
            };
            let (i, j) = (y * w + x, yj * w + xj);
            let (a, b) = (i32::from(s[i]), i32::from(s[j]));
            // Equal spins, or a lattice one site wide where the "neighbour" is the site.
            if a == b {
                continue;
            }
            // ΔE = 2a(h_i′ − h_j′), each field without the partner's own spin.
            let de = 2 * a * ((field(s, x, y) - b) - (field(s, xj, yj) - a));
            if r & 0x3FFF_FFFF < thr[((de + 12) / 4) as usize] {
                s.swap(i, j);
            }
        }
    }

    /// A fresh 50/50 mixture: exactly `⌊N/2⌋` down spins, shuffled (Fisher–Yates), the
    /// infinite-temperature start of the Kawasaki act.
    pub(crate) fn scramble(&mut self, rng: &mut SplitMix64) {
        let s = self.spins.as_mut_slice();
        let half = s.len() / 2;
        for (k, v) in s.iter_mut().enumerate() {
            *v = if k < half { -1 } else { 1 };
        }
        for k in (1..s.len()).rev() {
            // Multiply-shift onto 0..=k (bias ≤ 2⁻³², harmless here).
            let j = (((rng.next_u64() >> 32) * (k as u64 + 1)) >> 32) as usize;
            s.swap(k, j);
        }
        self.m = self.spins.as_slice().iter().map(|&s| i32::from(s)).sum();
    }

    /// Indices of the four wrapped neighbours of site `(x, y)`.
    #[inline]
    fn neighbours(&self, x: usize, y: usize) -> [usize; 4] {
        let row = y * usize::from(self.spins.width());
        [
            row + usize::from(self.xl[x]),
            row + usize::from(self.xr[x]),
            self.yu[y] as usize + x,
            self.yd[y] as usize + x,
        ]
    }

    /// Shade class of every site, for the inks (SPEC-002 Inks).
    /// Sites in same-sign clusters of fewer than [`SPECK`] spins are specks: class
    /// [`SPECK_CLASS`], and neighbours count them as aligned, so they seed no wall. Any other site with an
    /// antiparallel non-speck neighbour is a wall, class = its effective aligned count
    /// (1–3), and seeds `depth = 0`; every other site is [`WALL_INTERIOR`] with
    /// `depth = DEPTH_CAP`. `speck` is per-site scratch for the cluster marks.
    pub(crate) fn classify(&self, speck: &mut [u8], class: &mut [u8], depth: &mut [u8]) {
        let w = usize::from(self.spins.width());
        let s = self.spins.as_slice();
        speck.fill(UNSEEN);
        for i in 0..s.len() {
            if speck[i] == UNSEEN {
                self.mark_cluster(i, speck);
            }
        }
        for i in 0..s.len() {
            (class[i], depth[i]) = if speck[i] == SMALL {
                (SPECK_CLASS, DEPTH_CAP)
            } else {
                let eff = self
                    .neighbours(i % w, i / w)
                    .iter()
                    .filter(|&&j| s[j] == s[i] || speck[j] == SMALL)
                    .count() as u8;
                if eff < 4 {
                    (eff, 0)
                } else {
                    (WALL_INTERIOR, DEPTH_CAP)
                }
            };
        }
    }

    /// Mark site `i`'s same-sign cluster [`SMALL`] if it has fewer than [`SPECK`] sites,
    /// else mark the sites seen so far [`LARGE`]: a flood fill that stops at `SPECK` sites
    /// or at a site already known to be large, so it costs O(1) per site.
    fn mark_cluster(&self, i: usize, speck: &mut [u8]) {
        let w = usize::from(self.spins.width());
        let s = self.spins.as_slice();
        let mut members = [i; SPECK];
        let (mut n, mut head, mut large) = (1, 0, false);
        'fill: while head < n {
            let k = members[head];
            head += 1;
            for j in self.neighbours(k % w, k / w) {
                if s[j] != s[i] || members[..n].contains(&j) {
                    continue;
                }
                if speck[j] == LARGE {
                    large = true;
                    break 'fill;
                }
                members[n] = j;
                n += 1;
                if n == SPECK {
                    large = true;
                    break 'fill;
                }
            }
        }
        let mark = if large { LARGE } else { SMALL };
        for &k in &members[..n] {
            speck[k] = mark;
        }
    }
}

#[derive(Debug)]
struct IsingModel {
    schedule: Timeline,
    /// Scene time, s; drives the schedule, so resizes never desync it.
    t: f64,
    /// Fractional heat-bath attempts carried between updates.
    acc: f64,
    /// Schedule phase of the last update: entering the Kawasaki act scrambles the spins.
    phase: usize,
    rng: SplitMix64,
    /// Empty until the first `resize` seeds it.
    lattice: Lattice,
    /// Resample target on a lattice change, then swapped in (ADR-0012).
    scratch: Grid<i8>,
    /// Lattice → pixel sampling tables, rebuilt in place on resize.
    map: SampleMap,
    /// `map.nearest` flattened per axis (lattice column of each pixel column, row of
    /// each pixel row), deliberately: the per-pixel loop then does two plain loads
    /// instead of two clamped tap lookups.
    cols: Vec<u16>,
    rows: Vec<u16>,
    /// Ink of every lattice site, so large displays (several pixels per spin) cost one
    /// lookup per pixel.
    site_inks: Vec<Ink>,
    /// Per-site scratch rebuilt each frame by [`Lattice::classify`]: cluster marks, shade
    /// classes and its (unused) depth seeds.
    speck: Vec<u8>,
    class: Vec<u8>,
    depth: Vec<u8>,
    pixels: Pixels,
    size: Size,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl IsingModel {
    fn new() -> Self {
        Self {
            dirty: true,
            schedule: schedule(),
            t: 0.0,
            acc: 0.0,
            phase: 0,
            rng: SplitMix64::new(0),
            lattice: Lattice::default(),
            scratch: Grid::default(),
            map: SampleMap::default(),
            cols: Vec::new(),
            rows: Vec::new(),
            site_inks: Vec::new(),
            speck: Vec::new(),
            class: Vec::new(),
            depth: Vec::new(),
            pixels: Pixels::default(),
            size: Size::new(0, 0),
        }
    }

    fn t_rel(&self) -> f64 {
        t_over_tc(self.schedule.at(self.t))
    }

    /// Shade every site (down black, up walls by alignment, up interiors flat), then
    /// paint each display pixel from its nearest site.
    fn rasterise(&mut self) {
        let size = self.lattice.size();
        let w = usize::from(size.width());
        self.lattice
            .classify(&mut self.speck, &mut self.class, &mut self.depth);
        let spins = self.lattice.spins.as_slice();
        for ((ink, &s), &c) in self.site_inks.iter_mut().zip(spins).zip(&self.class) {
            *ink = site_ink(s, c);
        }
        let (rows, cols, site_inks) = (&self.rows, &self.cols, &self.site_inks);
        self.pixels.shade(|x, y| {
            // The tables cover the pixels after every resize; `get` only guards the
            // protocol (an empty table before the first resize paints nothing).
            let (Some(&ly), Some(&lx)) = (rows.get(y as usize), cols.get(x as usize)) else {
                return Ink::BLANK;
            };
            site_inks
                .get(usize::from(ly) * w + usize::from(lx))
                .copied()
                .unwrap_or(Ink::BLANK)
        });
    }
}

impl Generator for IsingModel {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        // Square pixels: one column by two half-rows per cell. An empty display still
        // gets a 1-site lattice so the state is always valid.
        let pixels = Size::new(size.width().max(1), size.height().saturating_mul(2).max(1));
        let lattice = lattice_for(pixels, MAX_SPINS, BUCKET);
        if self.lattice.len() == 0 {
            // The only draw from ctx.rng (GEN-R7): t = 0 is a quench from T = ∞.
            self.rng = SplitMix64::from_rng(&mut ctx.rng);
            self.lattice = Lattice::random(lattice, &mut self.rng);
        } else if lattice != self.lattice.size() {
            regrid_nearest(&mut self.lattice.spins, lattice, &mut self.scratch);
            self.lattice.rebuild();
        }
        self.site_inks.resize(self.lattice.len(), Ink::BLANK);
        for v in [&mut self.speck, &mut self.class, &mut self.depth] {
            v.resize(self.lattice.len(), 0);
        }
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let map = &mut self.map;
        map.rebuild(lattice, px);
        // Grid axes are u16, so lattice indices fit a u16: the casts are lossless.
        self.cols.clear();
        self.cols
            .extend((0..i32::from(px.width())).map(|x| map.nearest(x, 0).0 as u16));
        self.rows.clear();
        self.rows
            .extend((0..i32::from(px.height())).map(|y| map.nearest(0, y).1 as u16));
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.t += dt;
        let beat = self.schedule.at(self.t);
        let t = T_C * t_over_tc(beat);
        let n = self.lattice.len();
        if beat.index == SPINODAL {
            // Entering the Kawasaki act: an even mixture quenched from T = ∞.
            if self.phase != SPINODAL {
                self.lattice.scramble(&mut self.rng);
            }
            self.acc += swap_speedup(beat.local) * sweep_rate(n) * n as f64 * dt;
            let whole = self.acc.floor();
            self.acc -= whole;
            self.lattice
                .swaps(whole as u64, &swap_thresholds(t), &mut self.rng);
        } else {
            // Leaving it: the Glauber act starts again from a quench at T = ∞, not from
            // the de-mixed labyrinth.
            if self.phase == SPINODAL {
                self.lattice.randomise(&mut self.rng);
            }
            self.acc += sweep_rate(n) * n as f64 * dt;
            let whole = self.acc.floor();
            self.acc -= whole;
            self.lattice
                .attempts(whole as u64, &thresholds(t), &mut self.rng);
        }
        self.phase = beat.index;
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
        self.pixels.blit(canvas, |_, _, top, bottom| {
            // The brighter half decides: walls `#`, interiors `+`, specks and ragged walls
            // `:`; down spins stay blank.
            let ink = sciart_core::raster::stronger(top, bottom);
            let glyph = match ink.level {
                0 => return None,
                l if l >= WALL_LEVELS[2] => ASCII_RAMP[4],
                l if l >= INTERIOR_LEVEL => ASCII_RAMP[3],
                _ => ASCII_RAMP[2],
            };
            Some((glyph.ascii, ink))
        });
        let m_abs = (self.phase != SPINODAL).then(|| {
            f64::from(self.lattice.magnetisation().unsigned_abs())
                / self.lattice.len().max(1) as f64
        });
        if let Some(text) = label(self.t_rel(), m_abs, self.size) {
            // A one-space margin either side makes the label a small dark chip.
            let s = text.as_str();
            canvas.text(0, 0, " ", LABEL_INK);
            canvas.text(1, 0, s, LABEL_INK);
            canvas.text(1 + s.len() as i32, 0, " ", LABEL_INK);
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(if self.schedule.at(self.t).index == SPINODAL {
            CAPTIONS[1]
        } else {
            CAPTIONS[0]
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Yang's spontaneous magnetisation `(1 − sinh(2/T)⁻⁴)^{1/8}` below T_c, 0 above
    /// (Yang 1952); the reference for ISING-A2/A3.
    fn onsager_m(t: f64) -> f64 {
        if t >= T_C {
            0.0
        } else {
            (1.0 - (2.0 / t).sinh().powi(-4)).powf(0.125)
        }
    }

    /// Arithmetic–geometric mean of `a, b ≥ 0`.
    fn agm(mut a: f64, mut b: f64) -> f64 {
        for _ in 0..64 {
            if (a - b).abs() <= 1e-16 * a {
                break;
            }
            (a, b) = (0.5 * (a + b), (a * b).sqrt());
        }
        a
    }

    /// Onsager's exact energy per spin `−coth 2β·[1 + (2/π)·k′·K(k)]`, using
    /// `K = π / (2·AGM(1, |k′|))` so `(2/π)·k′·K = k′ / AGM(1, |k′|)`, with the signed
    /// complementary modulus `k′ = 2 tanh² 2β − 1` computed directly (no cancellation at
    /// T_c) and `k′·K → 0` at `k′ = 0` (ISING-A2).
    fn onsager_e(t: f64) -> f64 {
        let b2 = 2.0 / t;
        let kp = 2.0 * b2.tanh().powi(2) - 1.0;
        let term = if kp == 0.0 {
            0.0
        } else {
            kp / agm(1.0, kp.abs())
        };
        -(1.0 / b2.tanh()) * (1.0 + term)
    }

    /// Energy per spin `−(1/N) Σ_bonds sᵢsⱼ`, each bond counted once.
    fn energy(l: &Lattice) -> f64 {
        let g = l.spins();
        let (w, h) = (i32::from(g.width()), i32::from(g.height()));
        let s = |x: i32, y: i32| i32::from(*g.get(x.rem_euclid(w), y.rem_euclid(h)).unwrap());
        let mut e = 0;
        for y in 0..h {
            for x in 0..w {
                e -= s(x, y) * (s(x + 1, y) + s(x, y + 1));
            }
        }
        f64::from(e) / f64::from(w * h)
    }

    /// Fraction of antiparallel bonds, `(1 + e/2)/2`.
    fn wall_density(l: &Lattice) -> f64 {
        0.5 * (1.0 + 0.5 * energy(l))
    }

    fn sweeps(l: &mut Lattice, mcs: usize, t: f64, rng: &mut SplitMix64) {
        l.attempts((mcs * l.len()) as u64, &thresholds(t), rng);
    }

    fn scene(size: Size) -> (IsingModel, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut s = IsingModel::new();
        s.resize(size, &mut ctx);
        (s, ctx)
    }

    fn assert_consistent(l: &Lattice) {
        let slice = l.spins().as_slice();
        assert!(slice.iter().all(|&s| s == 1 || s == -1), "spin outside ±1");
        let sum: i32 = slice.iter().map(|&s| i32::from(s)).sum();
        assert_eq!(l.magnetisation(), sum, "tracked M");
    }

    #[test]
    fn ising_a1_heat_bath_obeys_detailed_balance_and_is_a_probability() {
        for t in [0.5, 1.0, T_C, 3.0, 10.0] {
            for h in [-4.0, -2.0, 0.0, 2.0, 4.0] {
                let (up, down) = (heat_bath_p(h, t), heat_bath_p(-h, t));
                let boltzmann = (2.0 * h / t).exp();
                assert!(((up / down) / boltzmann - 1.0).abs() < 1e-12, "T {t} h {h}");
                assert!((up + down - 1.0).abs() < 1e-15, "T {t} h {h}");
            }
            assert_eq!(heat_bath_p(0.0, t), 0.5);
            let thr = thresholds(t);
            assert_eq!(thr[2], 1 << 31);
            assert!(thr.windows(2).all(|w| w[0] <= w[1]), "T {t}: {thr:?}");
            // The quantised table the dynamics use, checked directly: entry i is
            // h = 2i − 4 to within rounding, and mirrored entries sum to 2³² (detailed
            // balance survives the quantisation).
            let scale = 4_294_967_296.0;
            for i in 0..5 {
                let h = 2.0 * i as f64 - 4.0;
                let q = thr[i] as f64 / scale;
                assert!((q - heat_bath_p(h, t)).abs() <= 1.0 / scale, "T {t} i {i}");
                let sum = thr[i] + thr[4 - i];
                assert!(sum.abs_diff(1 << 32) <= 1, "T {t} i {i}: {sum}");
            }
        }
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for h in [-4.0, -2.0, 0.0, 2.0, 4.0] {
                let p = heat_bath_p(h, t);
                assert!((0.0..=1.0).contains(&p), "T {t} h {h}: {p}");
            }
            assert!(thresholds(t).iter().all(|&x| x <= 1 << 32));
        }
    }

    #[test]
    fn ising_a2_onsager_and_yang_references() {
        assert!((T_C - 2.269_185_314).abs() < 1e-9);
        assert!((T_C - 2.0 / (1.0 + 2f64.sqrt()).ln()).abs() < 1e-15);
        assert!((onsager_m(2.0) - 0.911_319).abs() < 1e-6);
        assert!((onsager_m(1.5) - 0.986_500).abs() < 1e-6);
        assert!((onsager_e(2.0) + 1.745_565).abs() < 1e-6);
        assert!((onsager_e(3.0) + 0.817_310).abs() < 1e-6);
        assert!((onsager_e(T_C) + 2f64.sqrt()).abs() < 1e-9);
        for dt in [-1e-6, 1e-6] {
            assert!((onsager_e(T_C + dt) + 2f64.sqrt()).abs() < 1e-4);
        }
        for t in [T_C, 2.5, 3.0, 10.0] {
            assert_eq!(onsager_m(t), 0.0);
        }
        let mut prev = f64::INFINITY;
        for i in 1..=2268 {
            let t = f64::from(i) * 1e-3;
            let m = onsager_m(t);
            assert!(m <= prev, "non-increasing at T {t}");
            if t > 1.0 {
                assert!(m < prev, "strictly decreasing at T {t}");
            }
            prev = m;
        }
    }

    /// Equilibrium means over `samples` MCS, one sample per MCS, after `burn` MCS.
    fn equilibrium(l: &mut Lattice, t: f64, burn: usize, samples: usize) -> (f64, f64) {
        let mut rng = SplitMix64::new(0x1515_2026);
        sweeps(l, burn, t, &mut rng);
        let (mut m, mut e) = (0.0, 0.0);
        for _ in 0..samples {
            sweeps(l, 1, t, &mut rng);
            m += f64::from(l.magnetisation().abs()) / l.len() as f64;
            e += energy(l);
        }
        (m / samples as f64, e / samples as f64)
    }

    #[test]
    fn ising_a3_glauber_reaches_onsager_equilibrium() {
        let size = Size::new(32, 32);
        let mut cold = Lattice::new(Grid::new(size, 1));
        let (m, e) = equilibrium(&mut cold, 2.0, 1000, 3000);
        assert!((m - 0.9113).abs() < 0.015, "<|m|>(2.0) = {m}");
        assert!((e + 1.7456).abs() < 0.015, "<e>(2.0) = {e}");

        let mut hot = Lattice::random(size, &mut SplitMix64::new(3));
        let (m, e) = equilibrium(&mut hot, 3.0, 1000, 3000);
        assert!(m < 0.2, "<|m|>(3.0) = {m}");
        assert!((e + 0.8173).abs() < 0.015, "<e>(3.0) = {e}");
    }

    #[test]
    fn ising_a4_quench_coarsens_with_allen_cahn_exponent() {
        let rho_eq = 0.5 * (1.0 + 0.5 * onsager_e(0.6 * T_C));
        assert!((rho_eq - 0.006_678).abs() < 1e-6);
        let mut rng = SplitMix64::new(7);
        let mut l = Lattice::random(Size::new(192, 192), &mut rng);
        let mut done = 0;
        let mut points = Vec::new();
        for mcs in [24, 48, 96, 192] {
            sweeps(&mut l, mcs - done, 0.6 * T_C, &mut rng);
            done = mcs;
            points.push((f64::from(mcs as u32).ln(), (wall_density(&l) - rho_eq).ln()));
        }
        assert!(points.windows(2).all(|p| p[1].1 < p[0].1), "{points:?}");
        let n = points.len() as f64;
        let (sx, sy) = points
            .iter()
            .fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        let sxx: f64 = points.iter().map(|p| p.0 * p.0).sum();
        let sxy: f64 = points.iter().map(|p| p.0 * p.1).sum();
        let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
        assert!((-0.62..=-0.32).contains(&slope), "slope {slope}");
    }

    #[derive(Debug, Clone)]
    enum Op {
        Resize(u16, u16),
        Step(f64),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            1 => (1u16..=240, 1u16..=70).prop_map(|(w, h)| Op::Resize(w, h)),
            3 => (0.0f64..=0.25).prop_map(Op::Step),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn ising_a5_spins_and_magnetisation_survive_updates_and_resizes(
            ops in proptest::collection::vec(op(), 1..24),
        ) {
            let (mut s, mut ctx) = scene(Size::new(80, 24));
            for op in ops.into_iter().chain([Op::Resize(1, 1), Op::Step(0.25)]) {
                match op {
                    Op::Resize(w, h) => s.resize(Size::new(w, h), &mut ctx),
                    Op::Step(dt) => s.update(dt, &mut ctx),
                }
                assert_consistent(&s.lattice);
            }
        }
    }

    #[test]
    fn ising_a5_resize_keeps_or_resamples_the_lattice() {
        let (mut s, mut ctx) = scene(Size::new(200, 50));
        for _ in 0..30 {
            s.update(SIM_DT, &mut ctx);
        }
        assert_eq!(s.lattice.size(), Size::new(178, 91));
        let before = s.lattice.spins().clone();
        s.resize(Size::new(210, 52), &mut ctx);
        assert_eq!(
            s.lattice.spins(),
            &before,
            "same bucket leaves spins untouched"
        );

        s.resize(Size::new(80, 24), &mut ctx);
        assert_eq!(s.lattice.size(), Size::new(80, 48));
        let mut expected = Grid::new(Size::new(80, 48), 0i8);
        sciart_core::grid::resample_nearest(&before, &mut expected);
        assert_eq!(s.lattice.spins(), &expected, "bucket change resamples");
        assert_consistent(&s.lattice);
    }

    #[test]
    fn ising_a5_one_second_performs_rate_times_n_attempts() {
        for size in [Size::new(200, 50), Size::new(80, 24), Size::new(20, 6)] {
            let (mut s, mut ctx) = scene(size);
            let n = s.lattice.len();
            let expected = (sweep_rate(n) * n as f64).round() as i64;
            let start = s.rng.clone();
            for _ in 0..60 {
                s.update(1.0 / 60.0, &mut ctx);
            }
            // Each attempt draws exactly one u64: find how far the generator moved.
            let mut probe = start;
            let mut drawn = 0i64;
            while probe != s.rng && drawn <= expected + 1 {
                probe.next_u64();
                drawn += 1;
            }
            assert!(
                (drawn - expected).abs() <= 1,
                "{size:?}: {drawn} vs {expected}"
            );
        }
        assert_eq!(sweep_rate(MAX_SPINS), 60.0);
        assert_eq!(sweep_rate(10), 1.0);
    }

    #[test]
    fn ising_a6_schedule_and_label() {
        let tl = schedule();
        let at = |t: f64| t_over_tc(tl.at(t));
        assert_eq!(tl.total(), 90.0);
        assert_eq!(at(0.0), 0.60);
        assert_eq!(at(10.0), 0.60);
        assert_eq!(at(30.0), 1.25);
        assert_eq!(at(50.0), 1.00);
        assert_eq!(at(70.0), 0.60);
        let mut prev = at(0.0);
        for i in 1..=(90 * 60 * 2) {
            let t = f64::from(i) / 60.0;
            let v = at(t);
            assert!((v - prev).abs() <= 0.01, "jump at {t}: {prev} -> {v}");
            assert!((v - at(t - 90.0)).abs() < 1e-12, "periodic at {t}");
            prev = v;
        }

        let text = |w, h| label(0.93, Some(0.71), Size::new(w, h)).map(|s| s.as_str().to_owned());
        assert_eq!(text(80, 24).as_deref(), Some("T/Tc 0.93  |m| 0.71"));
        assert_eq!(text(24, 4).as_deref(), Some("T/Tc 0.93  |m| 0.71"));
        for w in 12..24 {
            assert_eq!(text(w, 24).as_deref(), Some("T/Tc 0.93"));
        }
        assert_eq!(text(11, 24), None);
        assert_eq!(text(80, 3), None);
    }

    #[test]
    fn ising_a7_kawasaki_swaps_conserve_m_obey_detailed_balance_and_coarsen() {
        for t in [0.5, 1.0, 0.6 * T_C, 3.0] {
            let thr = swap_thresholds(t);
            let scale = f64::from(1u32 << 30);
            for k in 0..7 {
                let de = 4.0 * k as f64 - 12.0;
                let (p, q) = (thr[k] as f64 / scale, thr[6 - k] as f64 / scale);
                // p(ΔE) + p(−ΔE) = 1 and p(ΔE)/p(−ΔE) = e^{−ΔE/T}, to the table's rounding.
                assert!((p + q - 1.0).abs() <= 2.0 / scale, "T {t} k {k}");
                let want = 1.0 / (1.0 + (de / t).exp());
                assert!((p - want).abs() <= 1.0 / scale, "T {t} k {k}");
            }
            assert_eq!(thr[3], 1 << 29);
        }
        assert_eq!(swap_speedup(0.0), 1.0);
        assert_eq!(swap_speedup(2.0), 1.5);
        assert_eq!(swap_speedup(4.0), 2.0);
        assert_eq!(swap_speedup(30.0), SWAP_SPEEDUP);
        for t in [0.0, f64::NAN, f64::INFINITY] {
            assert!(swap_thresholds(t).iter().all(|&x| x <= 1 << 30));
        }

        // A scrambled 50/50 lattice un-mixes at 0.6 T_c with M fixed throughout.
        let mut rng = SplitMix64::new(11);
        let mut l = Lattice::random(Size::new(96, 64), &mut rng);
        l.scramble(&mut rng);
        let down = l.spins().as_slice().iter().filter(|&&s| s < 0).count();
        assert_eq!(down, l.len() / 2);
        let m0 = l.magnetisation();
        assert_consistent(&l);
        let thr = swap_thresholds(0.6 * T_C);
        let mut rho = Vec::new();
        for _ in 0..4 {
            l.swaps(200 * l.len() as u64, &thr, &mut rng);
            assert_consistent(&l);
            assert_eq!(l.magnetisation(), m0, "M changed");
            rho.push(wall_density(&l));
        }
        assert!(rho.windows(2).all(|w| w[1] < w[0]), "{rho:?}");
        assert!(rho[3] < 0.2, "{rho:?}");
        // A one-site-wide lattice (its "neighbour" is itself) never swaps or panics.
        let mut thin = Lattice::random(Size::new(1, 5), &mut rng);
        thin.swaps(1000, &thr, &mut rng);
        assert_consistent(&thin);
    }

    #[test]
    fn ising_a7_the_spinodal_act_scrambles_once_and_keeps_m() {
        let (mut s, mut ctx) = scene(Size::new(80, 24));
        while s.t < 57.75 {
            s.update(0.25, &mut ctx);
        }
        assert_eq!(s.caption(), Some("spins align"));
        s.update(0.25, &mut ctx);
        assert_eq!(s.caption(), Some("spins swap"));
        let m = s.lattice.magnetisation();
        assert!(m.abs() <= 1, "scrambled to M = {m}");
        while s.caption() == Some("spins swap") {
            s.update(SIM_DT, &mut ctx);
            if s.caption() == Some("spins swap") {
                assert_eq!(s.lattice.magnetisation(), m);
            }
        }
        assert!(
            s.t >= 90.0 && s.t < 90.0 + 2.0 * SIM_DT,
            "loops back at {}",
            s.t
        );
        assert_eq!(s.caption(), Some("spins align"), "loops back to Glauber");
        assert_eq!(META.captions, &CAPTIONS);
    }

    #[test]
    fn ising_a6_the_glauber_act_restarts_from_a_quench_after_the_swaps() {
        // The loop's second pass starts from T = ∞ like the first, not from the labyrinth.
        let (mut s, mut ctx) = scene(Size::new(80, 24));
        while s.t < 89.9 {
            s.update(0.1, &mut ctx);
        }
        let labyrinth = wall_density(&s.lattice);
        while s.t < 90.0 {
            s.update(SIM_DT, &mut ctx);
        }
        let quenched = wall_density(&s.lattice);
        assert!(
            labyrinth < 0.3 && quenched > 0.4,
            "{labyrinth} -> {quenched}"
        );
        assert_consistent(&s.lattice);
    }

    #[test]
    fn ising_a8_down_is_black_and_walls_carry_the_light() {
        // ADR-0017: down spins are black; up interiors a dim grey; walls brighter, the
        // straighter the brighter; specks draw as interior flecks.
        assert_eq!(site_ink(-1, 3), Ink::BLANK);
        assert_eq!(site_ink(-1, WALL_INTERIOR), Ink::BLANK);
        let interior = site_ink(1, WALL_INTERIOR);
        assert_eq!(interior, Ink::new(UP_ROLE, INTERIOR_LEVEL));
        assert_eq!(site_ink(1, SPECK_CLASS), interior);
        let walls: Vec<u8> = (0..4).map(|c| site_ink(1, c).level).collect();
        assert!(walls.windows(2).all(|w| w[1] > w[0]), "{walls:?}");
        assert!(walls[3] > 2 * interior.level && walls[3] >= 180);

        // A lone flipped spin and a 2×2 speck are specks and leave their neighbours
        // interior; a 2×3 cluster (SPECK sites) is a domain with walls; a straight wall is
        // class 3 on both sides.
        let mut g = Grid::new(Size::new(16, 10), 1i8);
        g.set(3, 3, -1);
        for (x, y) in [(3, 7), (4, 7), (3, 8), (4, 8)] {
            g.set(x, y, -1);
        }
        for y in 0..10 {
            for x in 12..16 {
                g.set(x, y, -1);
            }
        }
        for y in 1..3 {
            for x in 7..10 {
                g.set(x, y, -1);
            }
        }
        let l = Lattice::new(g);
        let n = l.len();
        let (mut speck, mut class, mut depth) = (vec![0; n], vec![0; n], vec![0; n]);
        l.classify(&mut speck, &mut class, &mut depth);
        let at = |x: usize, y: usize| y * 16 + x;
        assert_eq!(class[at(3, 3)], SPECK_CLASS);
        assert_eq!(class[at(4, 8)], SPECK_CLASS);
        for (x, y) in [
            (2, 3),
            (4, 3),
            (3, 2),
            (3, 4),
            (2, 7),
            (5, 8),
            (3, 6),
            (4, 9),
        ] {
            assert_eq!(class[at(x, y)], WALL_INTERIOR, "({x}, {y})");
        }
        assert_eq!(class[at(7, 1)], 2, "the 2x3 block's corner is a wall");
        assert_eq!((class[at(11, 5)], class[at(12, 5)]), (3, 3));
        // The torus wraps: column 0 is next to the down stripe's column 15.
        assert_eq!(class[at(0, 5)], 3);
    }

    #[test]
    fn ising_a9_label_reads_t_over_t_c_and_m() {
        let row = |w: u16| {
            let size = Size::new(w, 24);
            let (s, _) = scene(size);
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            s.render(&mut c);
            c.to_lines()[0].clone()
        };
        assert!(row(80).starts_with(" T/Tc 0.60  |m| 0."), "{}", row(80));
        // No thermometer (ADR-0017: the readout alone explains the science).
        assert!(
            !row(80).contains("-|-") && !row(80).contains('='),
            "{}",
            row(80)
        );
        // The Kawasaki act pins M, so the readout states the rule instead of a number.
        let text = label(0.6, None, Size::new(80, 24)).unwrap();
        assert_eq!(text.as_str(), "T/Tc 0.60  M fixed");
        let (mut s, mut ctx) = scene(Size::new(80, 24));
        while s.t < 62.0 {
            s.update(0.25, &mut ctx);
        }
        let mut c = Canvas::new(Size::new(80, 24), GlyphMode::Ascii);
        s.render(&mut c);
        assert!(
            c.to_lines()[0].starts_with(" T/Tc 0.60  M fixed "),
            "{}",
            c.to_lines()[0]
        );
    }

    /// Mean level over every pixel of the raster, 0..1.
    fn mean_level(s: &IsingModel) -> f64 {
        let px = s.pixels.pixel_size();
        let (w, h) = (i32::from(px.width()), i32::from(px.height()));
        let mut sum = 0.0;
        for y in 0..h {
            for x in 0..w {
                sum += f64::from(s.pixels.get(x, y).unwrap().level) / 255.0;
            }
        }
        sum / f64::from(w * h)
    }

    #[test]
    fn ising_a8_hot_phase_sinks_toward_the_background() {
        // Order lights up (SPEC-002 Intent): the hot boil at s = 30 is at most 0.65 of
        // the brightness of the coarsened domains at the end of `coarsen` (s = 20).
        let (mut s, mut ctx) = scene(Size::new(200, 50));
        let mut run_to = |s: &mut IsingModel, t: f64| {
            while s.t + 1e-9 < t {
                s.update((t - s.t).min(0.25), &mut ctx);
            }
        };
        run_to(&mut s, 20.0);
        let ordered = mean_level(&s);
        run_to(&mut s, 30.0);
        let hot = mean_level(&s);
        assert!(hot <= 0.65 * ordered, "hot {hot} vs ordered {ordered}");
    }

    #[test]
    fn ising_a10_phases_differ_in_lightness_not_hue() {
        // Colour-blind safe and Mono-proof: down is black, up a neutral grey, and no ramp
        // is red or green.
        assert_eq!(site_ink(-1, WALL_INTERIOR), Ink::BLANK);
        assert!(site_ink(1, WALL_INTERIOR).level >= 15);
        assert!(!matches!(
            UP_ROLE,
            Role::Ramp(Ramp::HAlpha | Ramp::Aurora | Ramp::Ember)
        ));
    }
}
