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
//! The lattice is capped at 16 384 spins (`grid::lattice_for`, ADR-0012) and shown as one
//! half-block pixel per spin at or below the cap, nearest-sampled above it. Every pixel
//! is a `Div` ink whose magnitude grows convexly with the number of aligned neighbours,
//! so domain interiors glow, walls read as dark seams and thermal noise sinks toward
//! the background. The raster is rebuilt in `update`/`resize`; `render` only blits it and
//! writes the label (the M10 raster convention).

use std::fmt::Write as _;

use sciart_core::grid::{lattice_for, regrid_nearest};
use sciart_core::prelude::*;
use sciart_core::{draw, ease, ink};

pub static META: Meta = Meta {
    name: "ising_model",
    category: Category::PhysicsChemistry,
    description: "The Ising ferromagnet quenched, reheated and cooled through its critical point",
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

/// ASCII ramp over the cell-mean signed value, `−1 → '.'` to `+1 → '#'`.
const ASCII_RAMP: [Glyph; 5] = [
    Glyph::ascii('.'),
    Glyph::ascii(','),
    Glyph::ascii(':'),
    Glyph::ascii('+'),
    Glyph::ascii('#'),
];
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

/// `|v|` of a spin with `n_al ∈ 0..=4` aligned neighbours (SPEC-002 Inks). Convex, not
/// linear: even at 1.25 T_c most spins have 3–4 aligned neighbours, so only a table
/// that saves its brightness for `n_al = 4` lets the hot phase sink toward the dark
/// centre stop while interiors stay a rich mid-tone rather than the palette's pale ends.
const ALIGNED: [f64; 5] = [0.06, 0.12, 0.22, 0.38, 0.72];

/// The `Div` ink of a spin `s` with `n_al` aligned neighbours, indexed by
/// `(s > 0)·5 + n_al`: `ink::div(±ALIGNED[n_al])`. Built once per scene (the one source
/// of truth is [`ALIGNED`]).
fn inks() -> [Ink; 10] {
    std::array::from_fn(|i| ink::div(if i >= 5 { 1.0 } else { -1.0 } * ALIGNED[i % 5]))
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

    /// Ink-table index of site `(x, y)`: `(s > 0)·5 + aligned neighbours`.
    #[inline]
    fn ink_index(&self, x: usize, y: usize) -> usize {
        let s = self.spins.as_slice();
        let row = y * usize::from(self.spins.width());
        let me = s[row + x];
        let aligned = [
            s[row + usize::from(self.xl[x])],
            s[row + usize::from(self.xr[x])],
            s[self.yu[y] as usize + x],
            s[self.yd[y] as usize + x],
        ]
        .iter()
        .filter(|&&n| n == me)
        .count();
        usize::from(me > 0) * 5 + aligned
    }
}

#[derive(Debug)]
struct IsingModel {
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
    /// [`inks`], indexed by `Lattice::ink_index`.
    inks: [Ink; 10],
    pixels: Pixels,
    size: Size,
}

impl IsingModel {
    fn new() -> Self {
        Self {
            schedule: schedule(),
            t: 0.0,
            acc: 0.0,
            rng: SplitMix64::new(0),
            lattice: Lattice::default(),
            scratch: Grid::default(),
            map: SampleMap::default(),
            cols: Vec::new(),
            rows: Vec::new(),
            site_inks: Vec::new(),
            inks: inks(),
            pixels: Pixels::default(),
            size: Size::new(0, 0),
        }
    }

    fn t_rel(&self) -> f64 {
        t_over_tc(self.schedule.at(self.t))
    }

    /// Ink every site, then paint each display pixel from its nearest site.
    fn rasterise(&mut self) {
        let size = self.lattice.size();
        let w = usize::from(size.width());
        for (y, row) in self.site_inks.chunks_exact_mut(w.max(1)).enumerate() {
            for (x, ink) in row.iter_mut().enumerate() {
                *ink = self.inks[self.lattice.ink_index(x, y)];
            }
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
        self.lattice.attempts(whole as u64, &thr, &mut self.rng);
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels.blit(canvas, |_, _, top, bottom| {
            // Cell mean of the two signed pixel values (exact inverse of `ink::div`),
            // normalised by the interior level so domain interiors reach the ramp ends.
            let m = 0.5 * (ink::div_value(top) + ink::div_value(bottom));
            let glyph = draw::ramp(0.5 * (m / ALIGNED[4] + 1.0), &ASCII_RAMP);
            Some((glyph.ascii, ink::div(m)))
        });
        let m_abs = f64::from(self.lattice.magnetisation().unsigned_abs())
            / self.lattice.len().max(1) as f64;
        if let Some(text) = label(self.t_rel(), m_abs, self.size) {
            // A one-space margin either side makes the label a small dark chip (`put`
            // clears the pixel background).
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

    #[test]
    fn ising_inks_follow_alignment() {
        // Pinned ends: a lone down spin sits just below zero, a fully aligned up spin high.
        let inks = inks();
        assert_eq!(inks[0], Ink::new(Role::Div, 120));
        assert_eq!(inks[4], Ink::new(Role::Div, 36));
        assert_eq!(inks[9], Ink::new(Role::Div, 219));
        // Convex in n_al: every extra aligned neighbour brightens more than the last.
        let step: Vec<f64> = ALIGNED.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(step.windows(2).all(|s| s[1] > s[0]), "{step:?}");
    }

    /// Mean |v| over every pixel of the raster.
    fn mean_abs(s: &IsingModel) -> f64 {
        let px = s.pixels.pixel_size();
        let (w, h) = (i32::from(px.width()), i32::from(px.height()));
        let mut sum = 0.0;
        for y in 0..h {
            for x in 0..w {
                sum += ink::div_value(s.pixels.get(x, y).unwrap()).abs();
            }
        }
        sum / f64::from(w * h)
    }

    #[test]
    fn ising_hot_phase_sinks_toward_the_background() {
        // Order lights up (SPEC-002 Intent): the hot boil at s = 30 is at most 0.65 of
        // the brightness of the coarsened domains at the end of `coarsen` (s = 20).
        let (mut s, mut ctx) = scene(Size::new(200, 50));
        let mut run_to = |s: &mut IsingModel, t: f64| {
            while s.t + 1e-9 < t {
                s.update((t - s.t).min(0.25), &mut ctx);
            }
        };
        run_to(&mut s, 20.0);
        let ordered = mean_abs(&s);
        run_to(&mut s, 30.0);
        let hot = mean_abs(&s);
        assert!(hot <= 0.65 * ordered, "hot {hot} vs ordered {ordered}");
    }
}
