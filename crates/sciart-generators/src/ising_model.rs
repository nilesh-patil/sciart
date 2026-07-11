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
//! ## Picture (M16, GEN-R25: domain walls only)
//!
//! Black ground; only the domain walls are drawn, as braille threads one dot thick. The
//! block magnetisation `b` (the mean of each spin's periodic 3×3 block, in steps of 2/9)
//! is traced by marching squares (`Dots::contour`, CORE-R105) at `+4/9` (`Hot`, the up
//! side of each wall) and `−4/9` (`Cold`, the down side), so every wall is a warm and a
//! cool thread side by side and a lone thermal spin, whose block stays at ±7/9, draws
//! nothing. Walls coarsen and straighten after the quench and roughen into a fractal
//! froth near T_c. Spins flipped since the last frame inside ordered regions
//! (`|b| ≥ 5/9` after the flip) blink as single dots for that frame, coloured by the new
//! spin. Rasterised once per drawn frame (GEN-R20); `render` blits and writes the label.

use std::fmt::Write as _;

use sciart_core::ease;
use sciart_core::grid::{lattice_for, regrid_nearest};
use sciart_core::prelude::*;

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

/// M16 (GEN-R25): the wall threads, at the block-magnetisation levels `±WALL` (between
/// the discrete block values 3/9 and 5/9), and the flip blinks.
pub(crate) const WALL: f32 = 4.0 / 9.0;
pub(crate) const UP_INK: Ink = Ink::new(Role::Hot, 220);
pub(crate) const DOWN_INK: Ink = Ink::new(Role::Cold, 210);
pub(crate) const FLIP_UP: Ink = Ink::new(Role::Hot, 140);
pub(crate) const FLIP_DOWN: Ink = Ink::new(Role::Cold, 140);
/// A flip blinks only where the block after it is this ordered (thermal flips inside
/// domains, not the boil).
const FLIP_ORDER: f32 = 5.0 / 9.0;

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

    /// Mean spin of the periodic 3×3 block about `(x, y)`, in steps of 2/9.
    #[inline]
    fn block(&self, x: usize, y: usize) -> f32 {
        let s = self.spins.as_slice();
        let (l, r) = (usize::from(self.xl[x]), usize::from(self.xr[x]));
        let rows = [
            self.yu[y] as usize,
            y * usize::from(self.spins.width()),
            self.yd[y] as usize,
        ];
        let sum: i32 = rows
            .iter()
            .map(|&row| i32::from(s[row + l]) + i32::from(s[row + x]) + i32::from(s[row + r]))
            .sum();
        sum as f32 / 9.0
    }
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
    /// Block magnetisation per site, and the spins before the last update (its flips blink);
    /// sized in `resize`.
    blocks: Vec<f32>,
    shown: Vec<i8>,
    dots: Dots,
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
            shown: Vec::new(),
            dots: Dots::default(),
            size: Size::new(0, 0),
        }
    }

    fn t_rel(&self) -> f64 {
        t_over_tc(self.schedule.at(self.t))
    }

    /// The walls as threads and this frame's flips as dots (SPEC-002 Look, ISING-A7).
    fn rasterise(&mut self) {
        let size = self.lattice.size();
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        self.dots.clear();
        if w * h == 0 || self.blocks.len() != w * h || self.shown.len() != w * h {
            return;
        }
        for y in 0..h {
            for x in 0..w {
                self.blocks[y * w + x] = self.lattice.block(x, y);
            }
        }
        let ds = self.dots.dot_size();
        let sx = f64::from(ds.width()) / w as f64;
        let sy = f64::from(ds.height()) / h as f64;
        let to_dot = |x: f64, y: f64| ((x + 0.5) * sx - 0.5, (y + 0.5) * sy - 0.5);
        self.dots
            .contour(&self.blocks, w, h, -WALL, to_dot, DOWN_INK);
        self.dots.contour(&self.blocks, w, h, WALL, to_dot, UP_INK);
        let spins = self.lattice.spins.as_slice();
        for (i, (&s, &was)) in spins.iter().zip(&self.shown).enumerate() {
            if s != was {
                let b = self.blocks[i];
                // Ordered surroundings: a thermal flip in a domain (its block keeps the
                // domain's sign), not the boil, where |b| is small.
                if b.abs() >= FLIP_ORDER {
                    let (x, y) = to_dot((i % w) as f64, (i / w) as f64);
                    self.dots
                        .plot(x, y, if s > 0 { FLIP_UP } else { FLIP_DOWN });
                }
            }
        }
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
        // Size the raster buffers here, so `prepare` never allocates (GEN-R11); a resize
        // shows no blinks (nothing flipped on screen).
        self.blocks.clear();
        self.blocks.resize(self.lattice.len(), 0.0);
        self.shown.clear();
        self.shown.extend_from_slice(self.lattice.spins.as_slice());
        self.dots.resize(size);
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
        // The blinks are this update's flips (GEN-R20: bound to updates, not frames).
        if self.shown.len() == n {
            self.shown.copy_from_slice(self.lattice.spins.as_slice());
        }
        self.lattice.attempts(whole as u64, &thr, &mut self.rng);
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

    /// Lit dots as (dot x, dot y, cell role).
    fn lit(s: &IsingModel) -> Vec<(i32, i32, Role)> {
        let mut canvas = Canvas::new(s.size, GlyphMode::Unicode);
        s.dots.blit(&mut canvas);
        let ds = s.dots.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if s.dots.is_lit(x, y) {
                    out.push((x, y, canvas.get(x / 2, y / 4).unwrap().ink.role));
                }
            }
        }
        out
    }

    /// Set every spin from `f(x, y)` and rebuild the tracked state; no blinks.
    fn paint(s: &mut IsingModel, f: impl Fn(usize, usize) -> i8) {
        let w = usize::from(s.lattice.size().width());
        for (i, v) in s.lattice.spins.as_mut_slice().iter_mut().enumerate() {
            *v = f(i % w, i / w);
        }
        s.lattice.rebuild();
        s.shown.copy_from_slice(s.lattice.spins.as_slice());
    }

    #[test]
    fn ising_a7_walls_only_hot_on_the_up_side_cold_on_the_down_side() {
        let (mut s, _) = scene(Size::new(80, 24));
        let (w, _) = (usize::from(s.lattice.size().width()), 0);
        let sx = f64::from(s.dots.dot_size().width()) / w as f64;
        // An up stripe over columns 20..60 in a down sea: walls at 19.5 (up on its right)
        // and 59.5 (up on its left).
        paint(&mut s, |x, _| if (20..60).contains(&x) { 1 } else { -1 });
        s.rasterise();
        let dots = lit(&s);
        assert!(dots.iter().any(|d| d.2 == Role::Hot) && dots.iter().any(|d| d.2 == Role::Cold));
        for &(x, _, role) in &dots {
            // Site coordinate of the dot, and its signed distance into the up side.
            let at = (f64::from(x) + 0.5) / sx - 0.5;
            let d = if at < 40.0 { at - 19.5 } else { 59.5 - at };
            match role {
                Role::Hot => assert!((-0.5..1.5).contains(&d), "Hot at {at}"),
                Role::Cold => assert!((-1.5..0.5).contains(&d), "Cold at {at}"),
                r => panic!("{r:?}"),
            }
        }
        // An ordered domain with lone thermal spins: no wall, nothing drawn.
        paint(
            &mut s,
            |x, y| if (x * 7 + y * 13) % 29 == 0 { -1 } else { 1 },
        );
        s.rasterise();
        assert!(lit(&s).is_empty());
        // Walls roughen: the hot boil (t = 30 s) draws more wall than the coarsened
        // domains at the end of `coarsen` (t = 20 s).
        let (mut s, mut ctx) = scene(Size::new(200, 50));
        let mut run_to = |s: &mut IsingModel, t: f64| {
            while s.t + 1e-9 < t {
                s.update((t - s.t).min(0.25), &mut ctx);
                s.prepare();
            }
        };
        run_to(&mut s, 20.0);
        let ordered = lit(&s).len();
        run_to(&mut s, 30.0);
        let hot = lit(&s).len();
        assert!(hot >= 2 * ordered, "hot {hot} vs ordered {ordered}");
        // No half-blocks or backgrounds anywhere.
        let mut canvas = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut canvas);
        for y in 1..50 {
            for x in 0..200 {
                let c = canvas.get(x, y).unwrap();
                assert_eq!(c.bg, Ink::BLANK);
                assert!(!matches!(c.ch, '▀' | '▄' | '█'));
            }
        }
    }

    #[test]
    fn ising_a8_flips_blink_for_one_update_inside_ordered_regions() {
        let (mut s, mut ctx) = scene(Size::new(80, 24));
        paint(&mut s, |_, _| 1);
        s.rasterise();
        assert!(lit(&s).is_empty(), "a resize or repaint shows no blinks");
        // One spin flipped inside the up domain: a single down (Cold) dot at its site.
        let w = usize::from(s.lattice.size().width());
        s.lattice.spins.as_mut_slice()[20 * w + 30] = -1;
        s.lattice.rebuild();
        s.rasterise();
        let dots = lit(&s);
        assert_eq!(dots.len(), 1, "{dots:?}");
        assert_eq!(dots[0].2, Role::Cold);
        assert_eq!(FLIP_DOWN.role, Role::Cold);
        assert_eq!(FLIP_UP.role, Role::Hot);
        // Blinks depend on the updates, not on how often frames are drawn (GEN-R20).
        let (mut a, mut ca) = scene(Size::new(80, 24));
        let (mut b, mut cb) = scene(Size::new(80, 24));
        for _ in 0..90 {
            a.update(SIM_DT, &mut ca);
            a.prepare();
            b.update(SIM_DT, &mut cb);
        }
        b.prepare();
        assert_eq!(lit(&a), lit(&b));
        let _ = &mut ctx;
    }
}
