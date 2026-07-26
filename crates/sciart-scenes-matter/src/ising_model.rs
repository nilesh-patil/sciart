//! The Ising ferromagnet quenched, reheated and cooled through its critical point
//! (SPEC-002 `ising_model`).
//!
//! Physics: the 2-D Ising model `H = −J Σ⟨ij⟩ sᵢsⱼ` on a torus, `J = k_B = 1`, zero field,
//! evolved by single-spin Glauber heat-bath moves in random-sequential order (Glauber
//! 1963). Random-sequential rather than checkerboard: sublattice sweeps flip every
//! `ΔE = 0` spin together at high T (a period-2 strobe), while random attempts give smooth
//! fractional sweeps per frame at any lattice size. T/T_c follows a 58 s looping script:
//! quench-and-coarsen (Allen–Cahn `ℓ ∝ t^{1/2}`), reheat, a hot boil, a slow cool into
//! T_c, a critical hold and a quench back.
//!
//! The lattice is capped at 16 384 spins (`grid::lattice_for`, ADR-0012).
//!
//! ## Picture (M17, GEN-R26: stipple domains + wall threads)
//!
//! Black ground. The block magnetisation `b` (the mean of each spin's periodic 5×5 block,
//! in steps of 2/25), averaged over 0.1 s of updates like an exposure, decides the
//! domains: a site is on a wall when its block sign differs from a 4-neighbour's, so a
//! cluster of a few thermal spins is no wall and walls do not jitter with every kick.
//! Each domain is a stipple of its spins, `Hot` on up sites and `Cold` on down sites,
//! kept with a probability that falls from 0.25 at a wall to 0.008 in the bulk over ≈ 3.5
//! dots of 3-4 chamfer distance (fixed per-dot thresholds, CORE-R107), so the walls carry
//! a soft coloured fringe and the domains stay mostly black. Each wall is one bright `Star` thread, the zero
//! iso-line of `b` refined between sites (CORE-R106). Every update ages each site: since
//! it last lay on a wall (a wall that moves on leaves its sites dissolving as a stipple
//! afterglow for 0.5 s) and since it last flipped into the minority inside an ordered
//! region (`|b| ≥ 3/5`; a fixed 5 % sample of sites, a `Star` flash for 0.2 s). The ages
//! advance with the updates, so the picture does not depend on the draw cadence
//! (GEN-R20); the distance and the dots are rasterised once per drawn frame in `prepare`,
//! walking each dot row in runs over one site with cached per-dot thresholds and no branch
//! per dot (the costly part at 500×150).

use std::fmt::Write as _;

use sciart_core::grid::{lattice_for, regrid_nearest};
use sciart_core::prelude::*;
use sciart_core::{ease, raster};

pub static META: Meta = Meta {
    name: "ising_model",
    category: Category::PhysicalChemistry,
    description: "The Ising ferromagnet quenched, reheated and cooled through its critical point",
    title: "Magnetic domains · spins align",
    captions: &[],
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

/// The 58 s schedule of T/T_c (ISING-A6); indices match [`t_over_tc`].
const PHASES: [(&str, f64); 6] = [
    ("coarsen", 20.0),
    ("reheat", 8.0),
    ("hot", 5.0),
    ("cool", 15.0),
    ("critical", 8.0),
    ("quench", 2.0),
];
const COLD: f64 = 0.60;
const HOT: f64 = 1.25;

const LABEL_INK: Ink = Ink::new(Role::Text, 200);

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
        _ => ease::lerp(1.0, COLD, ease::smoothstep(p)),
    }
}

/// The corner readout, `T/Tc 0.93  |m| 0.71`, or `None` where it does not fit (the `|m|`
/// part needs 24 columns, the rest 12 columns and 4 rows) (ISING-A6).
pub(crate) fn label(t_rel: f64, m_abs: f64, size: Size) -> Option<StackText<24>> {
    if size.width() < 12 || size.height() < 4 {
        return None;
    }
    let mut s = StackText::new();
    let _ = write!(s, "T/Tc {t_rel:.2}");
    if size.width() >= 24 {
        let _ = write!(s, "  |m| {m_abs:.2}");
    }
    Some(s)
}

/// M17 (GEN-R26): the coarse-graining block (sites a side). 5×5 averages away the clusters
/// of a few spins, so the walls are the domains' own boundaries, smooth, and the boil a
/// froth rather than a mesh; the block sum is odd, so `b` is never 0.
pub(crate) const BLOCK: usize = 5;
/// The wall thread is the zero iso-line of the block magnetisation `b`.
pub(crate) const WALL: f32 = 0.0;
/// The walls and the fringe follow the block field averaged over this time (s).
const BLOCK_TAU: f32 = 0.1;
/// Domain stipple: kept share at a wall site and in the deep bulk, and the decay length of
/// the fall-off between them, in dots, so on every screen the walls carry a soft
/// coloured fringe a few dots wide and the domains stay mostly black (Look (M17),
/// ISING-A10).
const P_WALL: f64 = 0.25;
const P_FLOOR: f64 = 0.008;
const FALLOFF_DOTS: f64 = 3.5;
/// Chamfer distance units per site (3-4 chamfer) and the cap of the density table.
const CHAMFER: u16 = 3;
const DIST_CAP: usize = 63;
/// Afterglow of a wall that has moved on (s) and its starting kept share: the dots dissolve
/// through their fixed thresholds as `p` falls, so the glow fades without shimmer.
pub(crate) const GLOW_S: f32 = 0.5;
const P_GLOW: f64 = 0.12;
/// How long a flipped spin flashes (s), and the fixed share of sites whose flips flash.
pub(crate) const FLASH_S: f32 = 0.2;
const FLASH_SHARE: f64 = 0.05;
/// Ages start here: older than any glow or flash.
const OLD: f32 = 1.0e3;
/// Domains are the two signs of an orbital lobe; the faint afterglow ranks below them in a
/// braille cell (one ink per cell), so it shows only on black; the wall thread and the
/// flashes are a separate layer blitted over the stipple, so the cells they cross show
/// the thread alone.
pub(crate) const UP_INK: Ink = Ink::new(Role::Hot, 200);
pub(crate) const DOWN_INK: Ink = Ink::new(Role::Cold, 200);
pub(crate) const GLOW_INK: Ink = Ink::new(Role::Star, 110);
pub(crate) const WALL_INK: Ink = Ink::new(Role::Star, 240);
pub(crate) const FLASH_INK: Ink = Ink::new(Role::Star, 255);
/// Stipple salts (CORE-R107).
const SALT_DOMAIN: u64 = 0x3C6E_F372_FE94_F82B;
const SALT_FLASH: u64 = 0x510E_527F_ADE6_82D1;
/// Sub-cells per lattice cell for the wall thread (CORE-R106): a site spans ~2 dots, so 2
/// rounds the corners of the block field's staircase.
const WALL_K: usize = 2;
/// A flip flashes only where the block after it is this ordered (thermal flips inside
/// domains, not the boil): at least 20 of the 25 spins agree.
const FLIP_ORDER: f32 = 15.0 / 25.0;

/// Layers of [`draw`] (tests draw them one at a time).
pub(crate) const DOMAINS: u8 = 1;
pub(crate) const GLOW: u8 = 2;
pub(crate) const WALLS: u8 = 4;
pub(crate) const FLASHES: u8 = 8;
pub(crate) const ALL_LAYERS: u8 = DOMAINS | GLOW | WALLS | FLASHES;

/// Kept share of the domain stipple at chamfer distance `d` from the nearest wall (in
/// 1/[`CHAMFER`] sites) on a screen of `dots_per_site`:
/// `P_FLOOR + (P_WALL − P_FLOOR)·e^{−d·dots_per_site/FALLOFF_DOTS}`.
pub(crate) fn domain_p(d: u16, dots_per_site: f64) -> f64 {
    let d = f64::from(d) / f64::from(CHAMFER) * dots_per_site;
    P_FLOOR + (P_WALL - P_FLOOR) * (-d / FALLOFF_DOTS).exp()
}

/// `256·domain_p(d)` for `d ≤ DIST_CAP` (farther is the floor), the raster's lookup;
/// rebuilt in `resize`.
fn density_table(dots_per_site: f64) -> [u8; DIST_CAP + 1] {
    std::array::from_fn(|i| (256.0 * domain_p(i as u16, dots_per_site)).round() as u8)
}

/// Each dot's fixed stipple threshold: CORE-R107's `hash01(salt ^ cell_key(x, y))`,
/// quantised to 1/256 and cached in `resize`, since the raster tests every dot of the
/// screen each frame. A dot is kept when its threshold is below `256·p`; the afterglow
/// uses the complementary threshold `255 − t`, so it does not repeat the domain's dots.
pub(crate) fn threshold(x: i32, y: i32) -> u8 {
    (sciart_core::rng::hash01(SALT_DOMAIN ^ sciart_core::rng::cell_key(x, y)) * 256.0) as u8
}

/// Two-pass 3-4 chamfer distance (Borgefors 1986) from the sites flagged by `wall` on a
/// `w × h` grid, in 1/3 sites, saturating; no wrap (a wall across the torus seam is seen
/// one domain late, which only thins the stipple there).
pub(crate) fn chamfer(w: usize, h: usize, wall: impl Fn(usize) -> bool, dist: &mut [u16]) {
    if w == 0 || h == 0 || dist.len() < w * h {
        return;
    }
    let (a, b) = (CHAMFER, CHAMFER + 1);
    for (i, d) in dist[..w * h].iter_mut().enumerate() {
        *d = if wall(i) { 0 } else { u16::MAX };
    }
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut d = dist[i];
            if x > 0 {
                d = d.min(dist[i - 1].saturating_add(a));
            }
            if y > 0 {
                d = d.min(dist[i - w].saturating_add(a));
                if x > 0 {
                    d = d.min(dist[i - w - 1].saturating_add(b));
                }
                if x + 1 < w {
                    d = d.min(dist[i - w + 1].saturating_add(b));
                }
            }
            dist[i] = d;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut d = dist[i];
            if x + 1 < w {
                d = d.min(dist[i + 1].saturating_add(a));
            }
            if y + 1 < h {
                d = d.min(dist[i + w].saturating_add(a));
                if x + 1 < w {
                    d = d.min(dist[i + w + 1].saturating_add(b));
                }
                if x > 0 {
                    d = d.min(dist[i + w - 1].saturating_add(b));
                }
            }
            dist[i] = d;
        }
    }
}

/// A periodic spin lattice with its wrapped-neighbour tables and tracked magnetisation.
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
        let mut spins = Grid::new(size, 1i8);
        for s in spins.as_mut_slice() {
            *s = if rng.next_u64() >> 63 == 1 { 1 } else { -1 };
        }
        Self::new(spins)
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

    /// The mean spin of every site's periodic `BLOCK × BLOCK` block into `blocks`, by
    /// separable sliding box sums through `rows` (both of length N): in steps of
    /// `2/BLOCK²`, never 0.
    pub(crate) fn blocks(&self, rows: &mut [i8], blocks: &mut [f32]) {
        let (w, h) = (
            usize::from(self.spins.width()),
            usize::from(self.spins.height()),
        );
        let s = self.spins.as_slice();
        let n = w * h;
        if n == 0 || rows.len() < n || blocks.len() < n {
            return;
        }
        let r = BLOCK / 2;
        // Index `i + d` on a torus of length `len`, for `d ≤ BLOCK`: one wrap suffices
        // unless the axis is shorter than the block.
        let wrap = |i: usize, len: usize| match i {
            _ if i < len => i,
            _ if i < 2 * len => i - len,
            _ => i % len,
        };
        // `back + x` is `x − r` on the torus, with `back < len`.
        let (back_x, back_y) = ((w * BLOCK - r) % w, (h * BLOCK - r) % h);
        for (row, out) in s.chunks_exact(w).zip(rows.chunks_exact_mut(w)) {
            let mut sum: i8 = (0..BLOCK).map(|d| row[wrap(back_x + d, w)]).sum();
            for (x, o) in out.iter_mut().enumerate() {
                *o = sum;
                sum += row[wrap(x + r + 1, w)] - row[wrap(x + back_x, w)];
            }
        }
        // Column sums as exact small integers in `blocks` (f32 holds them exactly): the
        // first row in full, then each row from the one above, adding the row entering the
        // window and dropping the one leaving it.
        for x in 0..w {
            blocks[x] = (0..BLOCK)
                .map(|d| f32::from(rows[wrap(back_y + d, h) * w + x]))
                .sum();
        }
        for y in 1..h {
            let (add, sub) = (wrap(y + r, h) * w, wrap(y - 1 + back_y, h) * w);
            let (above, below) = blocks.split_at_mut(y * w);
            let (prev, cur) = (&above[(y - 1) * w..], &mut below[..w]);
            for x in 0..w {
                cur[x] = prev[x] + f32::from(rows[add + x]) - f32::from(rows[sub + x]);
            }
        }
        let cells = (BLOCK * BLOCK) as f32;
        for b in &mut blocks[..n] {
            *b /= cells;
        }
    }
}

/// A run of dot columns `d0..d1` over site column `site`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run {
    site: usize,
    d0: usize,
    d1: usize,
}

/// The domain stipple and the afterglow as braille: per terminal cell the masks of its up,
/// down and glow dots, accumulated without a branch per dot (stipple keeps are random by
/// design, so branching on them would mispredict half the time). Blitted as `Dots::blit`
/// does: a cell with domain dots takes the sign that has more of them (`Hot` on a tie),
/// otherwise the glow ink.
#[derive(Debug)]
pub(crate) struct Stipple {
    cells: Size,
    masks: Vec<[u8; 3]>,
}

impl Default for Stipple {
    /// Empty until the first `resize`.
    fn default() -> Self {
        Self {
            cells: Size::new(0, 0),
            masks: Vec::new(),
        }
    }
}

impl Stipple {
    fn resize(&mut self, cells: Size) {
        self.cells = cells;
        self.masks.clear();
        self.masks.resize(cells.area(), [0; 3]);
    }

    fn clear(&mut self) {
        self.masks.fill([0; 3]);
    }

    /// Light dot `(x, y)` (on the lattice) in layer `k` (0 up, 1 down, 2 glow) if `keep`.
    #[inline]
    fn put(&mut self, x: usize, y: usize, k: usize, keep: bool) {
        let i = (y / 4) * usize::from(self.cells.width()) + x / 2;
        self.masks[i][k] |= u8::from(keep) * raster::dot_bit(x as i32, y as i32);
    }

    /// The dot masks of cell `(cx, cy)`: up, down, glow.
    #[cfg(test)]
    pub(crate) fn cell(&self, cx: i32, cy: i32) -> [u8; 3] {
        let w = i32::from(self.cells.width());
        self.masks[(cy * w + cx) as usize]
    }

    fn blit(&self, canvas: &mut Canvas) {
        let w = usize::from(self.cells.width());
        for (i, &[up, down, glow]) in self.masks.iter().enumerate() {
            let mask = up | down | glow;
            if mask != 0 {
                let ink = match (up | down, up.count_ones() >= down.count_ones()) {
                    (0, _) => GLOW_INK,
                    (_, true) => UP_INK,
                    (_, false) => DOWN_INK,
                };
                canvas.put((i % w) as i32, (i / w) as i32, raster::braille(mask), ink);
            }
        }
    }
}

/// The per-site fields one raster reads, borrowed so the scene's dot layers stay free.
pub(crate) struct Field<'a> {
    w: usize,
    h: usize,
    spins: &'a [i8],
    blocks: &'a [f32],
    dist: &'a [u16],
    wall_age: &'a [f32],
    flip_age: &'a [f32],
    /// The site row of each dot row, and the runs of dot columns over one site column.
    ys: &'a [u16],
    runs: &'a [Run],
    /// Each dot's fixed threshold ([`threshold`]), row-major over the dot lattice.
    thr: &'a [u8],
    /// `256·p` of the domain stipple by chamfer distance.
    density: &'a [u8; DIST_CAP + 1],
    /// Dots per site along x and y.
    scale: (f64, f64),
}

impl Field<'_> {
    /// Dot coordinates of the continuous site point `(x, y)` (site centres at integers).
    fn to_dot(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x + 0.5) * self.scale.0 - 0.5,
            (y + 0.5) * self.scale.1 - 0.5,
        )
    }
}

/// Draw `layers` of the picture (SPEC-002 Look (M17)): the domain stipple and the
/// afterglow onto `stipple`, the wall thread and the flip flashes onto `threads`. The
/// stipple walks each dot row in runs of dots over one site, so density, sign and glow are
/// looked up once per run.
pub(crate) fn draw(stipple: &mut Stipple, threads: &mut Dots, f: &Field<'_>, layers: u8) {
    let n = f.w * f.h;
    let dw = f.runs.last().map_or(0, |r| r.d1);
    if n == 0
        || f.spins.len() < n
        || f.blocks.len() < n
        || f.dist.len() < n
        || f.wall_age.len() < n
        || f.flip_age.len() < n
        || f.thr.len() < dw * f.ys.len()
        || stipple.masks.len() * 8 < dw * f.ys.len()
    {
        return;
    }
    if layers & (DOMAINS | GLOW) != 0 {
        for (y, &sy) in f.ys.iter().enumerate() {
            let (row, thr) = (usize::from(sy) * f.w, &f.thr[y * dw..][..dw]);
            for run in f.runs {
                let site = row + run.site;
                if layers & DOMAINS != 0 {
                    let p = f.density[usize::from(f.dist[site]).min(DIST_CAP)];
                    let k = usize::from(f.spins[site] < 0);
                    for (x, &t) in (run.d0..run.d1).zip(&thr[run.d0..run.d1]) {
                        stipple.put(x, y, k, t < p);
                    }
                }
                let age = f.wall_age[site];
                if layers & GLOW != 0 && age > 0.0 && age < GLOW_S {
                    // The complementary threshold, so the glow is not the domain's dots.
                    let p = (256.0 * P_GLOW as f32 * (1.0 - age / GLOW_S)) as u8;
                    for (x, &t) in (run.d0..run.d1).zip(&thr[run.d0..run.d1]) {
                        stipple.put(x, y, 2, 255 - t < p);
                    }
                }
            }
        }
    }
    if layers & WALLS != 0 {
        for j in 0..f.h.saturating_sub(1) {
            for i in 0..f.w.saturating_sub(1) {
                let b = |x: usize, y: usize| f.blocks[y * f.w + x];
                let c = [b(i, j), b(i + 1, j), b(i + 1, j + 1), b(i, j + 1)];
                let below = c.map(|v| v < WALL);
                if below != [true; 4] && below != [false; 4] {
                    let (bx, by) = (i as f64, j as f64);
                    threads.contour_cell_refined(
                        c,
                        WALL,
                        (0, 0),
                        WALL_K,
                        |x, y| f.to_dot(bx + x, by + y),
                        WALL_INK,
                    );
                }
            }
        }
    }
    if layers & FLASHES != 0 {
        for (i, &age) in f.flip_age[..n].iter().enumerate() {
            if age < FLASH_S {
                let (x, y) = f.to_dot((i % f.w) as f64, (i / f.w) as f64);
                threads.plot(x, y, FLASH_INK);
            }
        }
    }
}

/// The [`Field`] of an [`IsingModel`], as a macro so it borrows only the fields it reads
/// and the dot layers stay free to draw on.
macro_rules! field {
    ($s:expr) => {{
        let size = $s.lattice.size();
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let ds = $s.threads.dot_size();
        Field {
            w,
            h,
            spins: $s.lattice.spins.as_slice(),
            blocks: &$s.blocks,
            dist: &$s.dist,
            wall_age: &$s.wall_age,
            flip_age: &$s.flip_age,
            ys: &$s.ys,
            runs: &$s.runs,
            thr: &$s.thr,
            density: &$s.density,
            scale: (
                f64::from(ds.width()) / w.max(1) as f64,
                f64::from(ds.height()) / h.max(1) as f64,
            ),
        }
    }};
}

#[derive(Debug)]
struct IsingModel {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    schedule: Timeline,
    /// Scene time, s; drives the schedule, so resizes never desync it.
    t: f64,
    /// Fractional heat-bath attempts carried between updates.
    acc: f64,
    rng: SplitMix64,
    /// Empty until the first `resize` seeds it.
    lattice: Lattice,
    /// Resample target on a lattice change, then swapped in (ADR-0012).
    scratch: Grid<i8>,
    /// Per site, all sized in `resize`: the block magnetisation and the spins before the
    /// last update (kept every update), the seconds since the site last lay on a wall and
    /// since it last flipped in an ordered region (both advanced by each update's `dt`, so
    /// glow and flashes depend on the updates, not on the draw cadence, GEN-R20), and the
    /// chamfer distance to the nearest wall (in `prepare`).
    blocks: Vec<f32>,
    /// Row box sums, the first pass of [`Lattice::blocks`].
    rows: Vec<i8>,
    /// The instantaneous block field; `blocks` is its average over [`BLOCK_TAU`].
    now: Vec<f32>,
    shown: Vec<i8>,
    wall_age: Vec<f32>,
    flip_age: Vec<f32>,
    dist: Vec<u16>,
    /// The site of each dot column and row, the runs of dot columns over one site column,
    /// and each dot's fixed stipple threshold ([`threshold`]), all built in `resize`.
    xs: Vec<u16>,
    ys: Vec<u16>,
    runs: Vec<Run>,
    thr: Vec<u8>,
    density: [u8; DIST_CAP + 1],
    /// Domain stipple and afterglow; wall thread and flashes, blitted over it.
    dots: Stipple,
    threads: Dots,
    size: Size,
}

impl IsingModel {
    fn new() -> Self {
        Self {
            dirty: false,
            schedule: schedule(),
            t: 0.0,
            acc: 0.0,
            rng: SplitMix64::new(0),
            lattice: Lattice::default(),
            scratch: Grid::default(),
            blocks: Vec::new(),
            rows: Vec::new(),
            now: Vec::new(),
            shown: Vec::new(),
            wall_age: Vec::new(),
            flip_age: Vec::new(),
            dist: Vec::new(),
            xs: Vec::new(),
            ys: Vec::new(),
            runs: Vec::new(),
            thr: Vec::new(),
            density: density_table(2.0),
            dots: Stipple::default(),
            threads: Dots::default(),
            size: Size::new(0, 0),
        }
    }

    fn t_rel(&self) -> f64 {
        t_over_tc(self.schedule.at(self.t))
    }

    /// After the spins moved by `dt` seconds: recompute the block field, restart the wall
    /// age of every site on a wall (a site whose block sign differs from a 4-neighbour's,
    /// so a lone thermal spin, whose block keeps the domain's sign, is no wall) and the
    /// flip age of every spin that `flips` since `shown` inside an ordered region; every
    /// other age grows by `dt`.
    fn refresh(&mut self, dt: f32, flips: bool) {
        let n = self.lattice.len();
        let w = usize::from(self.lattice.size().width());
        let sizes = [
            self.blocks.len(),
            self.now.len(),
            self.wall_age.len(),
            self.flip_age.len(),
        ];
        if n == 0 || sizes != [n; 4] || self.shown.len() != n {
            return;
        }
        self.lattice.blocks(&mut self.rows, &mut self.now);
        // Walls follow the block field averaged over BLOCK_TAU (an exposure, per update so
        // independent of the draw cadence): the domains' boundaries, not each thermal kick.
        if dt > 0.0 {
            let a = (dt / BLOCK_TAU).min(1.0);
            for (b, &x) in self.blocks.iter_mut().zip(&self.now) {
                *b += (x - *b) * a;
            }
        } else {
            self.blocks.copy_from_slice(&self.now);
        }
        let l = &self.lattice;
        let (spins, blocks) = (l.spins.as_slice(), &self.blocks);
        for (y, (&above, &below)) in l.yu.iter().zip(&l.yd).enumerate() {
            let row = y * w;
            for x in 0..w {
                let i = row + x;
                let up = blocks[i] > 0.0;
                let wall = (blocks[row + usize::from(l.xl[x])] > 0.0) != up
                    || (blocks[row + usize::from(l.xr[x])] > 0.0) != up
                    || (blocks[above as usize + x] > 0.0) != up
                    || (blocks[below as usize + x] > 0.0) != up;
                let age = &mut self.wall_age[i];
                *age = if wall { 0.0 } else { (*age + dt).min(OLD) };
                // A flip into the minority inside an ordered block, at a fixed sparse
                // sample of sites, so thermal flips sparkle instead of snowing.
                let flipped = flips
                    && self.shown[i] != spins[i]
                    && (spins[i] > 0) != up
                    && self.now[i].abs() >= FLIP_ORDER
                    && raster::stipple_keep(SALT_FLASH, x as i32, y as i32, FLASH_SHARE);
                let age = &mut self.flip_age[i];
                *age = if flipped { 0.0 } else { (*age + dt).min(OLD) };
            }
        }
    }

    /// The picture (SPEC-002 Look (M17)): the wall distance, then every layer.
    fn rasterise(&mut self) {
        let size = self.lattice.size();
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let ages = &self.wall_age;
        chamfer(w, h, |i| ages[i] == 0.0, &mut self.dist);
        let field = field!(self);
        self.dots.clear();
        self.threads.clear();
        draw(&mut self.dots, &mut self.threads, &field, ALL_LAYERS);
    }
}

impl Generator for IsingModel {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        // Square sites: one column by two half-rows per cell. An empty display still
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
        // Size the per-site buffers here, so `update` and `prepare` never allocate
        // (GEN-R11); a resize shows no glow and no flashes.
        let n = self.lattice.len();
        self.blocks.clear();
        self.blocks.resize(n, 0.0);
        self.rows.clear();
        self.rows.resize(n, 0);
        self.now.clear();
        self.now.resize(n, 0.0);
        for v in [&mut self.wall_age, &mut self.flip_age] {
            v.clear();
            v.resize(n, OLD);
        }
        self.dist.clear();
        self.dist.resize(n, 0);
        self.shown.clear();
        self.shown.extend_from_slice(self.lattice.spins.as_slice());
        self.dots.resize(size);
        self.threads.resize(size);
        let ds = self.threads.dot_size();
        let (w, h) = (self.lattice.size().width(), self.lattice.size().height());
        // Dot d's centre (d + ½) over the dot extent, in sites.
        let map = |dots: u16, sites: u16, out: &mut Vec<u16>| {
            out.clear();
            out.extend((0..u32::from(dots)).map(|d| {
                let s = (2 * d + 1) * u32::from(sites) / (2 * u32::from(dots));
                s.min(u32::from(sites.max(1)) - 1) as u16
            }));
        };
        self.density = density_table(f64::from(ds.width()) / f64::from(w.max(1)));
        map(ds.width(), w, &mut self.xs);
        map(ds.height(), h, &mut self.ys);
        self.runs.clear();
        for (d, &s) in self.xs.iter().enumerate() {
            match self.runs.last_mut() {
                Some(run) if run.site == usize::from(s) => run.d1 = d + 1,
                _ => self.runs.push(Run {
                    site: usize::from(s),
                    d0: d,
                    d1: d + 1,
                }),
            }
        }
        let (dw, dh) = (i32::from(ds.width()), i32::from(ds.height()));
        self.thr.clear();
        self.thr
            .extend((0..dh).flat_map(|y| (0..dw).map(move |x| threshold(x, y))));
        self.refresh(0.0, false);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.t += dt;
        let thr = thresholds(T_C * self.t_rel());
        let n = self.lattice.len();
        self.acc += sweep_rate(n) * n as f64 * dt;
        let whole = self.acc.floor();
        self.acc -= whole;
        if self.shown.len() == n {
            self.shown.copy_from_slice(self.lattice.spins.as_slice());
        }
        self.lattice.attempts(whole as u64, &thr, &mut self.rng);
        self.refresh(dt as f32, true);
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
        // ASCII falls back by dot count (`Dots::blit`).
        self.dots.blit(canvas);
        self.threads.blit(canvas);
        let m_abs = f64::from(self.lattice.magnetisation().unsigned_abs())
            / self.lattice.len().max(1) as f64;
        if let Some(text) = label(self.t_rel(), m_abs, self.size) {
            // A one-space margin either side makes the label a small dark chip.
            let s = text.as_str();
            canvas.text(0, 0, " ", LABEL_INK);
            canvas.text(1, 0, s, LABEL_INK);
            canvas.text(1 + s.len() as i32, 0, " ", LABEL_INK);
        }
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
            s.prepare();
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
                s.prepare();
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
        assert_eq!(tl.total(), 58.0);
        assert_eq!(at(0.0), 0.60);
        assert_eq!(at(10.0), 0.60);
        assert_eq!(at(30.0), 1.25);
        assert_eq!(at(50.0), 1.00);
        let mut prev = at(0.0);
        for i in 1..=(58 * 60 * 2) {
            let t = f64::from(i) / 60.0;
            let v = at(t);
            assert!((v - prev).abs() <= 0.01, "jump at {t}: {prev} -> {v}");
            assert!((v - at(t - 58.0)).abs() < 1e-12, "periodic at {t}");
            prev = v;
        }

        let text = |w, h| label(0.93, 0.71, Size::new(w, h)).map(|s| s.as_str().to_owned());
        assert_eq!(text(80, 24).as_deref(), Some("T/Tc 0.93  |m| 0.71"));
        assert_eq!(text(24, 4).as_deref(), Some("T/Tc 0.93  |m| 0.71"));
        for w in 12..24 {
            assert_eq!(text(w, 24).as_deref(), Some("T/Tc 0.93"));
        }
        assert_eq!(text(11, 24), None);
        assert_eq!(text(80, 3), None);
    }

    /// The dots of `layers` alone at the scene's current state (stipple and thread layers
    /// merged), as (dot x, dot y).
    fn layer(s: &IsingModel, layers: u8) -> Vec<(i32, i32)> {
        let (mut a, mut b) = (Stipple::default(), Dots::new(s.size));
        a.resize(s.size);
        draw(&mut a, &mut b, &field!(s), layers);
        let ds = b.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if a.cell(x / 2, y / 4)
                    .iter()
                    .any(|&m| m & raster::dot_bit(x, y) != 0)
                    || b.is_lit(x, y)
                {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// The site under dot `(x, y)`.
    fn site(s: &IsingModel, (x, y): (i32, i32)) -> usize {
        usize::from(s.ys[y as usize]) * usize::from(s.lattice.size().width())
            + usize::from(s.xs[x as usize])
    }

    /// Set every spin from `f(x, y)` and rebuild the tracked state: no flashes, and the
    /// walls as they are now (no glow).
    fn paint(s: &mut IsingModel, f: impl Fn(usize, usize) -> i8) {
        let w = usize::from(s.lattice.size().width());
        for (i, v) in s.lattice.spins.as_mut_slice().iter_mut().enumerate() {
            *v = f(i % w, i / w);
        }
        s.lattice.rebuild();
        s.shown.copy_from_slice(s.lattice.spins.as_slice());
        s.wall_age.fill(OLD);
        s.flip_age.fill(OLD);
        s.refresh(0.0, false);
        s.rasterise();
    }

    fn run_to(s: &mut IsingModel, ctx: &mut Ctx, t: f64) {
        while s.t + 1e-9 < t {
            s.update(SIM_DT.min(t - s.t), ctx);
            s.prepare();
        }
    }

    #[test]
    fn ising_a7_a11_one_wall_thread_on_each_wall() {
        let (mut s, _) = scene(Size::new(80, 24));
        let w = usize::from(s.lattice.size().width());
        let sx = f64::from(s.threads.dot_size().width()) / w as f64;
        // An up stripe over columns 20..60 in a down sea: walls at 19.5 and 59.5.
        paint(&mut s, |x, _| if (20..60).contains(&x) { 1 } else { -1 });
        let walls = layer(&s, WALLS);
        assert!(!walls.is_empty());
        // ISING-A11: one thread per wall, on it (the M16 pair stood a site either side):
        // in every dot row, exactly one thread dot near each wall.
        for (x, _) in &walls {
            let at = (f64::from(*x) + 0.5) / sx - 0.5;
            let d = (at - 19.5).abs().min((at - 59.5).abs());
            assert!(d <= 0.75, "thread dot at site {at}");
        }
        // Rows outside the first and last site centres hold no contour (no torus seam).
        let rows = i32::from(s.threads.dot_size().height());
        for y in 2..rows - 2 {
            let left = walls.iter().filter(|p| p.1 == y && p.0 < 80).count();
            let right = walls.iter().filter(|p| p.1 == y && p.0 >= 80).count();
            assert_eq!((left, right), (1, 1), "dot row {y}");
        }
        // An ordered domain with lone thermal spins has no wall.
        paint(
            &mut s,
            |x, y| if (x * 7 + y * 13) % 29 == 0 { -1 } else { 1 },
        );
        assert!(layer(&s, WALLS).is_empty());
        // Walls roughen: the hot boil (t = 30 s) draws more wall than the coarsened
        // domains at the end of `coarsen` (t = 20 s).
        let (mut s, mut ctx) = scene(Size::new(200, 50));
        run_to(&mut s, &mut ctx, 20.0);
        let ordered = layer(&s, WALLS).len();
        run_to(&mut s, &mut ctx, 30.0);
        let hot = layer(&s, WALLS).len();
        assert!(hot >= 2 * ordered, "hot {hot} vs ordered {ordered}");
        // No half-blocks or backgrounds anywhere (GEN-R26).
        let mut canvas = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut canvas);
        for c in canvas.cells() {
            assert_eq!(c.bg, Ink::BLANK);
            assert!(!matches!(c.ch, '▀' | '▄' | '█'));
        }
    }

    #[test]
    fn ising_a8_flips_flash_for_a_fifth_of_a_second_inside_ordered_regions() {
        let (mut s, mut ctx) = scene(Size::new(80, 24));
        paint(&mut s, |_, _| 1);
        assert!(
            layer(&s, FLASHES).is_empty(),
            "a resize or repaint shows no flashes"
        );
        // A fixed sparse sample of sites flashes: about FLASH_SHARE of them.
        let w = usize::from(s.lattice.size().width());
        let n = s.lattice.len();
        let flashing = |i: usize| {
            raster::stipple_keep(SALT_FLASH, (i % w) as i32, (i / w) as i32, FLASH_SHARE)
        };
        let share = (0..n).filter(|&i| flashing(i)).count() as f64 / n as f64;
        assert!((share - FLASH_SHARE).abs() < 0.03, "{share}");
        // One spin of the sample flipped inside the up domain: one Star flash at its site;
        // one flipped off the sample: none.
        let on = (20 * w..n).find(|&i| flashing(i)).unwrap();
        let off = (20 * w..n).find(|&i| !flashing(i)).unwrap();
        for (i, want) in [(off, 0), (on, 1)] {
            paint(&mut s, |_, _| 1);
            s.shown.copy_from_slice(s.lattice.spins.as_slice());
            s.lattice.spins.as_mut_slice()[i] = -1;
            s.lattice.rebuild();
            s.refresh(SIM_DT as f32, true);
            let flash = layer(&s, FLASHES);
            assert_eq!(flash.len(), want, "{flash:?}");
            if want == 1 {
                assert_eq!(site(&s, flash[0]), on);
            }
        }
        assert_eq!(FLASH_INK.role, Role::Star);
        // It lasts ≈ 0.2 s of updates, then goes.
        for i in 1..=13 {
            s.refresh(SIM_DT as f32, false);
            let lit = !layer(&s, FLASHES).is_empty();
            assert_eq!(
                lit,
                (i as f32) * (SIM_DT as f32) < FLASH_S,
                "after {i} steps"
            );
        }
        // Flashes and glow depend on the updates, not on how often frames are drawn
        // (GEN-R20).
        let (mut a, mut ca) = scene(Size::new(80, 24));
        let (mut b, mut cb) = scene(Size::new(80, 24));
        for _ in 0..90 {
            a.update(SIM_DT, &mut ca);
            a.prepare();
            b.update(SIM_DT, &mut cb);
        }
        b.prepare();
        assert_eq!(layer(&a, ALL_LAYERS), layer(&b, ALL_LAYERS));
        let _ = &mut ctx;
    }

    #[test]
    fn ising_a9_stipple_is_hot_on_up_sites_and_cold_on_down_sites() {
        let (mut s, mut ctx) = scene(Size::new(120, 40));
        for t in [3.0, 20.0, 30.0, 50.0] {
            run_to(&mut s, &mut ctx, t);
            let mut dots = Stipple::default();
            dots.resize(s.size);
            draw(&mut dots, &mut Dots::new(s.size), &field!(s), DOMAINS);
            let lit = layer(&s, DOMAINS);
            assert!(lit.len() > 100, "t {t}");
            // Each braille cell takes one ink: wherever a cell holds dots of one sign
            // only, its ink is that sign's.
            let mut canvas = Canvas::new(s.size, GlyphMode::Unicode);
            dots.blit(&mut canvas);
            let mut checked = 0;
            for cy in 0..i32::from(s.size.height()) {
                for cx in 0..i32::from(s.size.width()) {
                    let signs: Vec<i8> = lit
                        .iter()
                        .filter(|p| p.0 / 2 == cx && p.1 / 4 == cy)
                        .map(|&p| s.lattice.spins.as_slice()[site(&s, p)])
                        .collect();
                    if signs.is_empty() {
                        continue;
                    }
                    let ink = canvas.get(cx, cy).unwrap().ink;
                    if signs.iter().all(|&v| v > 0) {
                        assert_eq!(ink, UP_INK, "t {t} cell ({cx}, {cy})");
                        checked += 1;
                    } else if signs.iter().all(|&v| v < 0) {
                        assert_eq!(ink, DOWN_INK, "t {t} cell ({cx}, {cy})");
                        checked += 1;
                    } else {
                        assert!(ink == UP_INK || ink == DOWN_INK);
                    }
                }
            }
            assert!(checked > 50, "t {t}");
        }
    }

    #[test]
    fn ising_a10_stipple_thins_away_from_the_walls() {
        let (mut s, mut ctx) = scene(Size::new(200, 50));
        run_to(&mut s, &mut ctx, 15.0);
        let lit: std::collections::HashSet<_> = layer(&s, DOMAINS).into_iter().collect();
        // Kept share of dots by distance band from the nearest wall (sites).
        let bands = [(0.0, 0.5), (1.0, 2.5), (3.0, 5.0), (6.0, 1e9)];
        let ds = s.threads.dot_size();
        let mut kept = [(0usize, 0usize); 4];
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                let d = f64::from(s.dist[site(&s, (x, y))]) / f64::from(CHAMFER);
                if let Some(b) = bands.iter().position(|&(lo, hi)| (lo..=hi).contains(&d)) {
                    kept[b].0 += usize::from(lit.contains(&(x, y)));
                    kept[b].1 += 1;
                }
            }
        }
        let share: Vec<f64> = kept
            .iter()
            .map(|&(k, n)| {
                assert!(n > 500, "{kept:?}");
                k as f64 / n as f64
            })
            .collect();
        assert!(share.windows(2).all(|p| p[1] < p[0]), "{share:?}");
        assert!((share[0] - P_WALL).abs() < 0.04, "{share:?}");
        assert!((share[3] - P_FLOOR).abs() < 0.02, "{share:?}");
        // The chamfer distance itself: 3 per site straight, 4 diagonally, 0 on walls.
        let mut d = vec![0u16; 25];
        chamfer(5, 5, |i| i == 12, &mut d);
        assert_eq!(&d[10..15], &[6, 3, 0, 3, 6]);
        assert_eq!((d[6], d[0], d[2]), (4, 8, 6));
    }

    #[test]
    fn ising_a11_block_field_is_the_periodic_5x5_mean() {
        // The sliding sums behind the wall thread equal the brute-force periodic mean,
        // also on axes shorter than the block (where the window wraps more than once).
        for (w, h, seed) in [(17, 11, 1), (5, 5, 2), (3, 7, 3), (1, 1, 4), (40, 2, 5)] {
            let l = Lattice::random(Size::new(w, h), &mut SplitMix64::new(seed));
            let (w, h) = (usize::from(w), usize::from(h));
            let (mut rows, mut blocks) = (vec![0i8; w * h], vec![0f32; w * h]);
            l.blocks(&mut rows, &mut blocks);
            let s = l.spins().as_slice();
            for y in 0..h {
                for x in 0..w {
                    let mut sum = 0i32;
                    for dy in 0..BLOCK {
                        for dx in 0..BLOCK {
                            let (sx, sy) =
                                ((x + w * BLOCK + dx - 2) % w, (y + h * BLOCK + dy - 2) % h);
                            sum += i32::from(s[sy * w + sx]);
                        }
                    }
                    let want = sum as f32 / 25.0;
                    assert_eq!(blocks[y * w + x], want, "{w}x{h} at ({x}, {y})");
                    assert_ne!(want, 0.0);
                }
            }
        }
    }

    #[test]
    fn ising_a12_afterglow_fades_within_a_second_when_spins_freeze() {
        let (mut s, _) = scene(Size::new(80, 24));
        paint(&mut s, |x, _| if (20..60).contains(&x) { 1 } else { -1 });
        assert!(
            layer(&s, GLOW).is_empty(),
            "a standing wall has no afterglow"
        );
        // The stripe jumps 6 sites right and the spins freeze. The time-averaged walls
        // glide after it within ≈ 0.25 s, leaving glow only over the swept columns.
        s.shown.copy_from_slice(s.lattice.spins.as_slice());
        let w = usize::from(s.lattice.size().width());
        for (i, v) in s.lattice.spins.as_mut_slice().iter_mut().enumerate() {
            *v = if (26..66).contains(&(i % w)) { 1 } else { -1 };
        }
        s.lattice.rebuild();
        let mut most = 0;
        for _ in 0..24 {
            s.refresh(SIM_DT as f32, false);
            let glow = layer(&s, GLOW);
            most = most.max(glow.len());
            for &p in &glow {
                let x = site(&s, p) % w;
                let swept = (18..=27).contains(&x) || (58..=67).contains(&x);
                assert!(swept, "glow at site column {x}");
            }
        }
        assert!(most > 20, "{most} glow dots");
        // Once the walls have settled the glow only fades, and 1 s after the spins froze
        // it is gone.
        let mut last = layer(&s, GLOW).len();
        for i in 24..60 {
            s.refresh(SIM_DT as f32, false);
            let now = layer(&s, GLOW).len();
            assert!(now <= last, "glow grows at step {i}");
            last = now;
        }
        assert_eq!(last, 0);
    }
}
