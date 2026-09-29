//! Conserved phase separation by Oono–Puri cell dynamics (Oono & Puri 1987, 1988): the
//! lattice map that behaves like the Cahn–Hilliard equation (model B) at a fraction of
//! its cost. Shared `pub(crate)` model (SPEC-002 Batch M13): `spinodal_decomposition`.
//!
//! One step on a periodic lattice, with `⟨⟨X⟩⟩ − X = K·Δ₉X` the isotropic nine-point
//! neighbour average minus the centre (`grid::laplacian9`, whose weights normalised to
//! one are ⅕ for each edge and 1/20 for each corner neighbour, so `K = 3/10`):
//!
//! ```text
//! Γ = A tanh ψ − ψ + D (⟨⟨ψ⟩⟩ − ψ)          local relaxation toward ±ψ* plus stiffness
//! ψ ← ψ − M (⟨⟨Γ⟩⟩ − Γ)                     conserved: ψ moves down the gradient of −Γ
//! ```
//!
//! `A > 1` makes `A tanh ψ = ψ` bistable at `±ψ*` (ψ* ≈ 0.86 at A = 1.3), the two phases;
//! `D` sets the interface width. The update is the discrete Laplacian of a field, so on a
//! periodic lattice it sums to zero: the mean of ψ (the mixture's composition) is
//! conserved, which is what makes domains coarsen as `t^{1/3}` (Lifshitz–Slyozov) instead
//! of shrinking away.

use rand::Rng;
use sciart_core::grid::{self, Edges};
use sciart_core::{Grid, Size, ink};

/// `⟨⟨X⟩⟩ − X = K · laplacian9(X)`.
const K: f32 = 0.3;

/// Map parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Params {
    /// Bistability `A` (> 1 for two phases).
    pub a: f32,
    /// Stiffness `D`.
    pub d: f32,
    /// Mobility `M` (time scale of one step).
    pub m: f32,
}

/// Oono & Puri's standard parameters: stable, smooth interfaces a few cells wide.
pub(crate) const OONO_PURI: Params = Params {
    a: 1.3,
    d: 0.5,
    m: 1.0,
};

/// Caller-owned working grids for [`step`], sized on first use and then reused.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scratch {
    gamma: Grid<f32>,
    lap: Grid<f32>,
}

/// One Oono–Puri step of `psi` in place (periodic edges). Allocation-free once `scratch`
/// has seen a lattice of this size.
pub(crate) fn step(psi: &mut Grid<f32>, s: &mut Scratch, p: &Params) {
    if psi.is_empty() {
        return;
    }
    grid::laplacian9(psi, &mut s.lap, Edges::Wrap);
    if s.gamma.size() != psi.size() {
        s.gamma.resize(psi.size(), 0.0);
    }
    let dk = p.d * K;
    for ((g, &x), &l) in s
        .gamma
        .as_mut_slice()
        .iter_mut()
        .zip(psi.as_slice())
        .zip(s.lap.as_slice())
    {
        *g = p.a * tanh(x) - x + dk * l;
    }
    grid::laplacian9(&s.gamma, &mut s.lap, Edges::Wrap);
    let mk = p.m * K;
    for (x, &l) in psi.as_mut_slice().iter_mut().zip(s.lap.as_slice()) {
        *x -= mk * l;
    }
}

/// `tanh` by the Padé approximant of [`ink::soft_clip`] (within 2 % on |ψ| ≤ 1.5, odd,
/// monotone, exactly ±1 beyond 3): the map only needs a bistable sigmoid, and a libm
/// `tanh` per node per step was half the step's cost.
#[inline]
fn tanh(x: f32) -> f32 {
    ink::soft_clip(f64::from(x)) as f32
}

/// Mean of `psi` in f64 (0 for an empty grid).
pub(crate) fn mean(psi: &Grid<f32>) -> f64 {
    if psi.is_empty() {
        return 0.0;
    }
    psi.as_slice().iter().map(|&x| f64::from(x)).sum::<f64>() / psi.len() as f64
}

/// A fresh quench: `mean` plus uniform noise of half-width `amp`, then recentred so the
/// lattice mean is `mean` to rounding.
pub(crate) fn quench(psi: &mut Grid<f32>, mean_psi: f32, amp: f32, rng: &mut impl Rng) {
    for x in psi.as_mut_slice() {
        *x = mean_psi + amp * (2.0 * rng.random::<f32>() - 1.0);
    }
    shift_mean(psi, f64::from(mean_psi));
}

/// Add a constant so the mean becomes `target` (to f32 rounding).
pub(crate) fn shift_mean(psi: &mut Grid<f32>, target: f64) {
    let delta = (target - mean(psi)) as f32;
    for x in psi.as_mut_slice() {
        *x += delta;
    }
}

/// Carry `psi` onto a `size` lattice (bilinear, ADR-0012) keeping its mean.
pub(crate) fn regrid(psi: &mut Grid<f32>, size: Size, scratch: &mut Grid<f32>) {
    if psi.size() == size {
        return;
    }
    let m = mean(psi);
    grid::regrid_bilinear(psi, size, scratch);
    shift_mean(psi, m);
}

/// Characteristic domain length: the first zero crossing of the two-point correlation
/// of `ψ − ψ̄` along x and along y (periodic), averaged, with linear interpolation
/// between lattice lags. Test and measurement helper (SPIN-A2).
#[cfg(test)]
pub(crate) fn correlation_length(psi: &Grid<f32>) -> f64 {
    let (w, h) = (usize::from(psi.width()), usize::from(psi.height()));
    let m = mean(psi);
    let v = |x: usize, y: usize| f64::from(psi.as_slice()[y * w + x]) - m;
    let corr = |r: usize, along_x: bool| {
        let mut s = 0.0;
        for y in 0..h {
            for x in 0..w {
                let (x2, y2) = if along_x {
                    ((x + r) % w, y)
                } else {
                    (x, (y + r) % h)
                };
                s += v(x, y) * v(x2, y2);
            }
        }
        s
    };
    let zero = |along_x: bool, n: usize| {
        let mut prev = corr(0, along_x);
        for r in 1..n / 2 {
            let c = corr(r, along_x);
            if c <= 0.0 {
                return (r - 1) as f64 + prev / (prev - c);
            }
            prev = c;
        }
        (n / 2) as f64
    };
    0.5 * (zero(true, w) + zero(false, h))
}

/// Number of 4-connected periodic clusters of cells with `ψ < threshold` (SPIN-A3).
#[cfg(test)]
pub(crate) fn count_below(psi: &Grid<f32>, threshold: f32, min_cells: usize) -> usize {
    let (w, h) = (usize::from(psi.width()), usize::from(psi.height()));
    let cells = psi.as_slice();
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut count = 0;
    for start in 0..w * h {
        if seen[start] || cells[start] >= threshold {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut size = 0;
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            for (nx, ny) in [
                ((x + 1) % w, y),
                ((x + w - 1) % w, y),
                (x, (y + 1) % h),
                (x, (y + h - 1) % h),
            ] {
                let j = ny * w + nx;
                if !seen[j] && cells[j] < threshold {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        count += usize::from(size >= min_cells);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::Ctx;

    fn quenched(w: u16, h: u16, m: f32, seed: u64) -> Grid<f32> {
        let mut ctx = Ctx::new(seed, Size::new(1, 1));
        let mut psi = Grid::new(Size::new(w, h), 0.0);
        quench(&mut psi, m, 0.05, &mut ctx.rng);
        psi
    }

    #[test]
    fn uniform_states_are_fixed_points() {
        let mut s = Scratch::default();
        for v in [0.0f32, 0.3, -0.7] {
            let mut psi = Grid::new(Size::new(9, 7), v);
            step(&mut psi, &mut s, &OONO_PURI);
            assert!(psi.as_slice().iter().all(|&x| (x - v).abs() < 1e-6));
        }
    }

    #[test]
    fn step_conserves_the_mean_and_separates_into_two_phases() {
        let mut psi = quenched(64, 48, 0.0, 3);
        let mut s = Scratch::default();
        let m0 = mean(&psi);
        for _ in 0..1500 {
            step(&mut psi, &mut s, &OONO_PURI);
        }
        assert!((mean(&psi) - m0).abs() < 1e-6);
        let bulk = psi.as_slice().iter().filter(|x| x.abs() > 0.7).count();
        assert!(bulk * 2 > psi.len(), "{bulk} of {} in a phase", psi.len());
    }

    #[test]
    fn regrid_keeps_the_mean_and_tiny_lattices_do_not_panic() {
        let mut psi = quenched(40, 20, 0.35, 5);
        let mut scratch = Grid::default();
        let m0 = mean(&psi);
        regrid(&mut psi, Size::new(17, 31), &mut scratch);
        assert_eq!(psi.size(), Size::new(17, 31));
        assert!((mean(&psi) - m0).abs() < 1e-6);
        let mut s = Scratch::default();
        for (w, h) in [(1, 1), (1, 3), (2, 1)] {
            let mut g = quenched(w, h, 0.1, 1);
            step(&mut g, &mut s, &OONO_PURI);
            assert!(g.as_slice().iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn correlation_length_reads_a_stripe_period() {
        // Stripes of period 20 along x: C(r) ∝ cos(2πr/20) crosses zero at r = 5.
        let mut psi = Grid::new(Size::new(40, 40), 0.0);
        for y in 0..40 {
            for x in 0..40 {
                let v = if (x / 10) % 2 == 0 { 1.0 } else { -1.0 };
                psi.set(x, y, v);
            }
        }
        let lx = correlation_length(&psi);
        // x: triangle correlation crosses zero at 5; y: constant rows never cross (20).
        assert!((lx - 0.5 * (5.0 + 20.0)).abs() < 1e-9, "{lx}");
        assert_eq!(count_below(&psi, 0.0, 1), 2);
    }
}
