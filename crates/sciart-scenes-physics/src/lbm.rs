//! D2Q9 lattice Boltzmann in lattice units (SPEC-002 `physics` LBM-A1/A2), lifted from
//! `vortex_street` for M13 and re-homed to this crate: one crate's scenes, one kernel.
//!
//! ## Model
//!
//! D2Q9 (Qian, d'Humières & Lallemand 1992) with the two-relaxation-time collision (TRT;
//! Ginzburg, Verhaeghe & d'Humières 2008):
//!
//! ```text
//! f_i* = f_i − ω⁺ (f_i⁺ − f_i^{eq+}) − ω⁻ (f_i⁻ − f_i^{eq−}),   f_i^± = (f_i ± f_ī) / 2
//! ω⁺ = 1/τ  (ν = (τ − ½)/3),   ω⁻ = 1/τ⁻  with  Λ = (τ − ½)(τ⁻ − ½)
//! ```
//!
//! TRT has BGK's equilibrium and ν(τ); BGK is TRT with `Λ = (τ − ½)²`, whose stable
//! velocity range collapses at small τ, whereas `Λ = ¼` maximises it.
//!
//! The kernel is a fused pull-scheme stream + collide in `f32` over SoA planes, storing
//! `f_i − w_i` (Lehmann et al. 2022) so the O(Ma²) deviations keep their precision. It
//! pulls post-collision populations from the upwind node; if that node is solid it takes
//! the opposite population of the node itself (halfway bounce-back, Ladd 1994). Rows away
//! from solids run a branch-free sliced loop in groups of [`LANES`] nodes, a shape LLVM
//! vectorises; only the solids' box and the wrap columns take the per-node path.
//!
//! Lattice rows grow downward, like the screen, and all velocities are in that frame.

use sciart_core::{Grid, Size};

/// D2Q9 velocities (rows-down frame): rest, E, S, W, N, SE, SW, NW, NE.
pub const CX: [i32; 9] = [0, 1, 0, -1, 0, 1, -1, -1, 1];
pub const CY: [i32; 9] = [0, 0, 1, 0, -1, 1, 1, -1, -1];
/// Index of the opposite velocity, `c_ī = −c_i`.
pub const OPP: [usize; 9] = [0, 3, 4, 1, 2, 7, 8, 5, 6];
/// Lattice weights `w_i`.
pub const W: [f32; 9] = [
    4.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
];
/// The four opposite pairs that TRT splits into even and odd parts.
pub(crate) const PAIRS: [(usize, usize); 4] = [(1, 3), (2, 4), (5, 7), (6, 8)];
/// Nodes collided together in the inner loop: one 128-bit SIMD register of f32 (NEON,
/// SSE2). Measured 7 ns/node against 29 ns unbatched; 8 and 16 lanes spill.
const LANES: usize = 4;

/// `τ⁻` from the magic parameter `Λ = (τ − ½)(τ⁻ − ½)` (LBM-A1).
pub fn tau_minus(tau: f64, magic: f64) -> f64 {
    0.5 + magic / (tau - 0.5)
}

/// Shifted equilibrium `f_i^eq − w_i = w_i (δρ + ρ(3 c·u + 4.5 (c·u)² − 1.5 u²))` with
/// `ρ = 1 + δρ` (LBM-A1).
#[inline(always)]
pub fn heq(drho: f32, ux: f32, uy: f32) -> [f32; 9] {
    let rho = 1.0 + drho;
    let usq = 1.5 * (ux * ux + uy * uy);
    std::array::from_fn(|i| {
        let cu = CX[i] as f32 * ux + CY[i] as f32 * uy;
        W[i] * (drho + rho * (3.0 * cu + 4.5 * cu * cu - usq))
    })
}

/// TRT collision of one node's shifted populations in place; returns the node's
/// `(ρ, uₓ, u_y)`, which the collision conserves (LBM-A1).
#[inline(always)]
pub fn collide(h: &mut [f32; 9], omega_p: f32, omega_m: f32) -> (f32, f32, f32) {
    let drho = ((h[0] + h[1]) + (h[2] + h[3])) + ((h[4] + h[5]) + (h[6] + h[7])) + h[8];
    let rho = 1.0 + drho;
    let jx = (h[1] - h[3]) + (h[5] - h[7]) + (h[8] - h[6]);
    let jy = (h[2] - h[4]) + (h[5] - h[7]) + (h[6] - h[8]);
    let (ux, uy) = (jx / rho, jy / rho);
    let eq = heq(drho, ux, uy);
    h[0] -= omega_p * (h[0] - eq[0]);
    for (i, j) in PAIRS {
        let (ni, nj) = (h[i] - eq[i], h[j] - eq[j]);
        let (even, odd) = (0.5 * (ni + nj), 0.5 * (ni - nj));
        h[i] -= omega_p * even + omega_m * odd;
        h[j] -= omega_p * even - omega_m * odd;
    }
    (rho, ux, uy)
}

/// Populations, moments and the solid mask of one lattice (LBM-A2).
#[derive(Debug, Clone, Default)]
pub struct Lattice {
    w: usize,
    h: usize,
    /// Nine SoA planes of `w·h` shifted post-collision populations `f_i − w_i`.
    f: Vec<f32>,
    /// The other half of the pull scheme's ping-pong pair.
    next: Vec<f32>,
    rho: Grid<f32>,
    ux: Grid<f32>,
    uy: Grid<f32>,
    solid: Vec<bool>,
    /// Rows that touch a solid node (or neighbour one) and the column range around the
    /// solids: only there does the kernel check the mask.
    near_solid: Vec<bool>,
    solid_cols: (usize, usize),
}

impl Lattice {
    /// A `size` lattice at rest (`f = w`, ρ = 1, u = 0) with `solid(x, y)` nodes.
    pub fn new(size: Size, solid: impl Fn(usize, usize) -> bool) -> Self {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let n = w * h;
        let mask: Vec<bool> = (0..n).map(|i| solid(i % w, i / w)).collect();
        let mut near_solid = vec![false; h];
        let (mut x0, mut x1) = (w, 0);
        for (i, _) in mask.iter().enumerate().filter(|(_, s)| **s) {
            let (x, y) = (i % w, i / w);
            for dy in [h - 1, 0, 1] {
                near_solid[(y + dy) % h] = true;
            }
            x0 = x0.min(x.saturating_sub(1));
            x1 = x1.max((x + 2).min(w));
        }
        Self {
            w,
            h,
            f: vec![0.0; 9 * n],
            next: vec![0.0; 9 * n],
            rho: Grid::new(size, 1.0),
            ux: Grid::new(size, 0.0),
            uy: Grid::new(size, 0.0),
            solid: mask,
            near_solid,
            solid_cols: (x0, x1.max(x0)),
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.w as u16, self.h as u16)
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    /// The nine planes of current shifted populations `f_i − w_i`, plane-major.
    #[cfg(test)]
    pub fn populations(&self) -> &[f32] {
        &self.f
    }

    /// The spare buffer's planes, mutable (spec tests drive boundary columns by hand).
    #[cfg(test)]
    pub fn spare_mut(&mut self) -> &mut [f32] {
        &mut self.next
    }

    pub fn ux(&self) -> &Grid<f32> {
        &self.ux
    }

    pub fn uy(&self) -> &Grid<f32> {
        &self.uy
    }

    /// The moment grids `(ρ, uₓ, u_y)` for writing, e.g. before [`Self::equilibrate`].
    pub fn moments_mut(&mut self) -> (&mut Grid<f32>, &mut Grid<f32>, &mut Grid<f32>) {
        (&mut self.rho, &mut self.ux, &mut self.uy)
    }

    /// The spare buffer just written by [`Self::stream_collide`] and the moment grids,
    /// for the caller's boundary conditions before [`Self::swap`].
    #[cfg(test)]
    pub fn next_and_moments(
        &mut self,
    ) -> (&mut [f32], &mut Grid<f32>, &mut Grid<f32>, &mut Grid<f32>) {
        (&mut self.next, &mut self.rho, &mut self.ux, &mut self.uy)
    }

    #[cfg(test)]
    pub fn solid(&self) -> &[bool] {
        &self.solid
    }

    /// Rebuild every population as the equilibrium of the stored moments (solid nodes
    /// at rest).
    pub fn equilibrate(&mut self) {
        let n = self.w * self.h;
        for i in 0..n {
            if self.solid[i] {
                self.rho.as_mut_slice()[i] = 1.0;
                self.ux.as_mut_slice()[i] = 0.0;
                self.uy.as_mut_slice()[i] = 0.0;
            }
            let (r, ux, uy) = (
                self.rho.as_slice()[i],
                self.ux.as_slice()[i],
                self.uy.as_slice()[i],
            );
            for (q, e) in heq(r - 1.0, ux, uy).into_iter().enumerate() {
                self.f[q * n + i] = e;
            }
        }
    }

    /// Whether every moment is finite and every ρ lies in `band`.
    pub fn healthy(&self, band: (f32, f32)) -> bool {
        let rho_ok = self
            .rho
            .as_slice()
            .iter()
            .all(|r| (band.0..=band.1).contains(r));
        let finite = |g: &Grid<f32>| g.as_slice().iter().all(|v| v.is_finite());
        rho_ok && finite(&self.ux) && finite(&self.uy)
    }

    /// Σρ in f64 over every node, from the current populations.
    #[cfg(test)]
    pub fn mass(&self) -> f64 {
        let n = self.w * self.h;
        (0..n)
            .map(|i| 1.0 + (0..9).map(|q| f64::from(self.f[q * n + i])).sum::<f64>())
            .sum()
    }

    /// Make the spare buffer current.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.f, &mut self.next);
    }

    /// One fully periodic step: [`Self::stream_collide`] with wrap in x, then swap.
    pub fn step_periodic(&mut self, omega_p: f32, omega_m: f32) {
        self.stream_collide(omega_p, omega_m, true);
        self.swap();
    }

    /// Fused pull stream + TRT collide into the spare buffer, periodic in y and, with
    /// `wrap_x`, in x; otherwise columns 0 and `w − 1` of the spare buffer are left for
    /// the caller. Writes the pre-collision moments of every node it collides.
    pub fn stream_collide(&mut self, omega_p: f32, omega_m: f32, wrap_x: bool) {
        let (w, h) = (self.w, self.h);
        let n = w * h;
        if n == 0 {
            return;
        }
        let omegas = (omega_p, omega_m);
        let src = planes(&self.f, n);
        let mut dst = planes_mut(&mut self.next, n);
        let (rho, ux, uy) = (
            self.rho.as_mut_slice(),
            self.ux.as_mut_slice(),
            self.uy.as_mut_slice(),
        );
        let (sx0, sx1) = (self.solid_cols.0.max(1), self.solid_cols.1.min(w - 1));
        for y in 0..h {
            let mut fast = |a: usize, b: usize| {
                if a < b {
                    let r = y * w + a..y * w + b;
                    let out = Moments {
                        rho: &mut rho[r.clone()],
                        ux: &mut ux[r.clone()],
                        uy: &mut uy[r],
                    };
                    run_fast(&src, &mut dst, out, (w, h), (y, a, b), omegas);
                }
            };
            if self.near_solid[y] && sx0 < sx1 {
                fast(1, sx0);
                fast(sx1, w - 1);
            } else {
                fast(1, w.saturating_sub(1).max(1));
            }
            let mut slow = |x: usize| {
                let i = y * w + x;
                (rho[i], ux[i], uy[i]) =
                    node_slow(&self.solid, &src, &mut dst, (w, h), (x, y), omegas);
            };
            if self.near_solid[y] {
                (sx0..sx1).for_each(&mut slow);
            }
            if wrap_x {
                slow(0);
                if w > 1 {
                    slow(w - 1);
                }
            }
        }
    }
}

/// The nine planes of a population buffer as separate slices. Both buffers are sized
/// `9·n` by construction; a wrong size fails here rather than as a far-away index panic.
fn planes(buf: &[f32], n: usize) -> [&[f32]; 9] {
    debug_assert_eq!(buf.len(), 9 * n);
    let mut it = buf.chunks_exact(n);
    std::array::from_fn(|_| it.next().unwrap_or_default())
}

/// The nine planes of a plane-major population buffer of `n` nodes, e.g. for boundary
/// conditions on [`Lattice::next_and_moments`]'s buffer.
pub fn planes_mut(buf: &mut [f32], n: usize) -> [&mut [f32]; 9] {
    debug_assert_eq!(buf.len(), 9 * n);
    let mut it = buf.chunks_exact_mut(n);
    std::array::from_fn(|_| it.next().unwrap_or_default())
}

/// Moment outputs of one row segment.
struct Moments<'a> {
    rho: &'a mut [f32],
    ux: &'a mut [f32],
    uy: &'a mut [f32],
}

/// Stream + collide the fluid run `a..b` of row `y` with no solid and no wrap in reach:
/// every population is a plain offset slice, so the loop is branch-free.
fn run_fast(
    src: &[&[f32]; 9],
    dst: &mut [&mut [f32]; 9],
    out: Moments<'_>,
    (w, h): (usize, usize),
    (y, a, b): (usize, usize, usize),
    (wp, wm): (f32, f32),
) {
    let m = b - a;
    let s: [&[f32]; 9] = std::array::from_fn(|i| {
        let sy = (y as i32 - CY[i]).rem_euclid(h as i32) as usize;
        let start = (a as i32 - CX[i]) as usize;
        &src[i][sy * w + start..][..m]
    });
    let mut d = dst.each_mut().map(|p| &mut p[y * w + a..][..m]);
    let (rho, ux, uy) = (&mut out.rho[..m], &mut out.ux[..m], &mut out.uy[..m]);
    let full = m - m % LANES;
    for k in (0..full).step_by(LANES) {
        let mut f: [[f32; LANES]; 9] =
            std::array::from_fn(|q| std::array::from_fn(|l| s[q][k + l]));
        let mut moments = [[0.0f32; LANES]; 3];
        for l in 0..LANES {
            let mut node: [f32; 9] = std::array::from_fn(|q| f[q][l]);
            let (r, u, v) = collide(&mut node, wp, wm);
            (moments[0][l], moments[1][l], moments[2][l]) = (r, u, v);
            for (q, value) in node.into_iter().enumerate() {
                f[q][l] = value;
            }
        }
        for (plane, lanes) in d.iter_mut().zip(&f) {
            plane[k..k + LANES].copy_from_slice(lanes);
        }
        for (field, lanes) in [&mut *rho, &mut *ux, &mut *uy].into_iter().zip(&moments) {
            field[k..k + LANES].copy_from_slice(lanes);
        }
    }
    for k in full..m {
        let mut f: [f32; 9] = std::array::from_fn(|q| s[q][k]);
        (rho[k], ux[k], uy[k]) = collide(&mut f, wp, wm);
        for (plane, value) in d.iter_mut().zip(f) {
            plane[k] = value;
        }
    }
}

/// Stream + collide one node with wrap-around and halfway bounce-back: a pull from a
/// solid node returns the node's own opposite post-collision population.
fn node_slow(
    lat_solid: &[bool],
    src: &[&[f32]; 9],
    dst: &mut [&mut [f32]; 9],
    (w, h): (usize, usize),
    (x, y): (usize, usize),
    (wp, wm): (f32, f32),
) -> (f32, f32, f32) {
    let i = y * w + x;
    if lat_solid[i] {
        for plane in dst.iter_mut() {
            plane[i] = 0.0;
        }
        return (1.0, 0.0, 0.0);
    }
    let mut f = [0.0f32; 9];
    for q in 0..9 {
        let sx = (x as i32 - CX[q]).rem_euclid(w as i32) as usize;
        let sy = (y as i32 - CY[q]).rem_euclid(h as i32) as usize;
        let j = sy * w + sx;
        f[q] = if lat_solid[j] {
            src[OPP[q]][i]
        } else {
            src[q][j]
        };
    }
    let moments = collide(&mut f, wp, wm);
    for (plane, v) in dst.iter_mut().zip(f) {
        plane[i] = v;
    }
    moments
}

// ---------------------------------------------------------------- tests
#[cfg(test)]
mod spec {
    //! SPEC-002 `physics` shared numerics: the D2Q9 TRT lattice Boltzmann module (LBM-A1,
    //! LBM-A2; formerly LBM-A1/R99, re-homed when the module became crate-private).

    use proptest::prelude::*;
    use sciart_core::Size;

    use super::{CX, CY, Lattice, W, tau_minus};

    const TAU: f64 = 0.53;

    fn omegas(tau: f64) -> (f32, f32) {
        ((1.0 / tau) as f32, (1.0 / tau_minus(tau, 0.25)) as f32)
    }

    /// `(Σh, Σh·cx, Σh·cy)` in f64.
    fn moments(h: &[f32; 9]) -> (f64, f64, f64) {
        let mut m = (0.0, 0.0, 0.0);
        for q in 0..9 {
            let v = f64::from(h[q]);
            m.0 += v;
            m.1 += v * f64::from(CX[q]);
            m.2 += v * f64::from(CY[q]);
        }
        m
    }

    #[test]
    fn lbm_a1_weights_velocities_and_opposites_are_consistent() {
        let sum: f64 = W.iter().map(|&w| f64::from(w)).sum();
        assert!((sum - 1.0).abs() < 1e-7);
        for q in 0..9 {
            let o = super::OPP[q];
            assert_eq!((CX[o], CY[o]), (-CX[q], -CY[q]));
            assert_eq!(W[o], W[q]);
        }
        assert!((tau_minus(0.53, 0.25) - (0.5 + 0.25 / 0.03)).abs() < 1e-9);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn lbm_a1_equilibrium_has_the_stated_moments(
            drho in -0.1f32..0.1, ux in -0.15f32..0.15, uy in -0.15f32..0.15,
        ) {
            let eq = super::heq(drho, ux, uy);
            let (m0, jx, jy) = moments(&eq);
            let rho = 1.0 + f64::from(drho);
            prop_assert!((m0 - f64::from(drho)).abs() < 1e-6);
            prop_assert!((jx - rho * f64::from(ux)).abs() < 1e-6);
            prop_assert!((jy - rho * f64::from(uy)).abs() < 1e-6);
        }

        #[test]
        fn lbm_a1_collision_conserves_mass_and_momentum(
            h in prop::array::uniform9(-0.02f32..0.02), tau in 0.51f64..1.5,
        ) {
            let (wp, wm) = omegas(tau);
            let mut post = h;
            let (rho, ux, uy) = super::collide(&mut post, wp, wm);
            let (a, b) = (moments(&h), moments(&post));
            prop_assert!((a.0 - b.0).abs() < 1e-6, "mass {a:?} → {b:?}");
            prop_assert!((a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6);
            prop_assert!((f64::from(rho) - (1.0 + a.0)).abs() < 1e-6);
            prop_assert!((f64::from(ux) * f64::from(rho) - a.1).abs() < 1e-6);
            prop_assert!((f64::from(uy) * f64::from(rho) - a.2).abs() < 1e-6);
        }
    }

    #[test]
    fn lbm_a1_equilibrium_is_a_fixed_point_of_the_collision() {
        let (wp, wm) = omegas(TAU);
        let eq = super::heq(0.02, 0.05, -0.03);
        let mut h = eq;
        super::collide(&mut h, wp, wm);
        for q in 0..9 {
            assert!((h[q] - eq[q]).abs() < 1e-7, "{q}: {} vs {}", h[q], eq[q]);
        }
    }

    fn shear_lattice(n: u16, amp: f32) -> Lattice {
        let mut lat = Lattice::new(Size::new(n, n), |_, _| false);
        let k = std::f64::consts::TAU / f64::from(n);
        let (_, ux, _) = lat.moments_mut();
        for y in 0..n {
            let v = amp * sciart_core::trig::sin(k * f64::from(y)) as f32;
            for x in 0..n {
                ux.set(i32::from(x), i32::from(y), v);
            }
        }
        lat.equilibrate();
        lat
    }

    /// Amplitude of the sin(ky) mode of uₓ.
    fn shear_amplitude(lat: &Lattice) -> f64 {
        let n = lat.height();
        let k = std::f64::consts::TAU / n as f64;
        let ux = lat.ux().as_slice();
        let mut s = 0.0;
        for y in 0..n {
            let row_mean: f64 = (0..lat.width())
                .map(|x| f64::from(ux[y * lat.width() + x]))
                .sum::<f64>()
                / lat.width() as f64;
            s += row_mean * sciart_core::trig::sin(k * y as f64);
        }
        2.0 * s / n as f64
    }

    #[test]
    fn lbm_a2_periodic_step_conserves_mass_to_rounding() {
        let (wp, wm) = omegas(TAU);
        let mut lat = shear_lattice(64, 0.08);
        let m0 = lat.mass();
        for _ in 0..200 {
            let before = lat.mass();
            lat.step_periodic(wp, wm);
            let rel = ((lat.mass() - before) / before).abs();
            assert!(rel < 1e-8, "{rel}");
        }
        assert!(((lat.mass() - m0) / m0).abs() < 1e-6);
    }

    #[test]
    fn lbm_a2_uniform_flow_stays_uniform_and_momentum_is_kept() {
        let (wp, wm) = omegas(TAU);
        let mut lat = Lattice::new(Size::new(24, 16), |_, _| false);
        {
            let (_, ux, uy) = lat.moments_mut();
            ux.fill(0.1);
            uy.fill(-0.04);
        }
        lat.equilibrate();
        for _ in 0..50 {
            lat.step_periodic(wp, wm);
        }
        for (&u, &v) in lat.ux().as_slice().iter().zip(lat.uy().as_slice()) {
            assert!((u - 0.1).abs() < 1e-5 && (v + 0.04).abs() < 1e-5, "{u} {v}");
        }
        assert!(lat.healthy((0.99, 1.01)));
    }

    #[test]
    fn lbm_a2_shear_wave_decays_at_the_trt_viscosity() {
        for tau in [0.53, 0.8] {
            let (wp, wm) = omegas(tau);
            let mut lat = shear_lattice(64, 0.01);
            // One step to fill the moments from the populations.
            lat.step_periodic(wp, wm);
            let a0 = shear_amplitude(&lat);
            let steps = 400;
            for _ in 0..steps {
                lat.step_periodic(wp, wm);
            }
            let a1 = shear_amplitude(&lat);
            let k = std::f64::consts::TAU / 64.0;
            let nu = -(a1 / a0).ln() / (k * k * f64::from(steps));
            let expect = (tau - 0.5) / 3.0;
            assert!(
                (nu / expect - 1.0).abs() < 0.02,
                "τ {tau}: ν {nu} vs {expect}"
            );
        }
    }

    #[test]
    fn lbm_a2_bounce_back_keeps_mass_and_solids_at_rest() {
        let (wp, wm) = omegas(TAU);
        let solid = |x: usize, y: usize| (10..14).contains(&x) && (6..10).contains(&y);
        let mut lat = Lattice::new(Size::new(32, 16), solid);
        {
            let (_, ux, _) = lat.moments_mut();
            ux.fill(0.08);
        }
        lat.equilibrate();
        let m0 = lat.mass();
        for _ in 0..300 {
            lat.step_periodic(wp, wm);
        }
        assert!(
            ((lat.mass() - m0) / m0).abs() < 1e-6,
            "{m0} → {}",
            lat.mass()
        );
        for (i, &s) in lat.solid().iter().enumerate() {
            if s {
                assert_eq!(lat.ux().as_slice()[i], 0.0);
            }
        }
        assert!(lat.healthy((0.9, 1.1)));
    }

    #[test]
    fn lbm_a2_without_wrap_the_border_columns_are_left_to_the_caller() {
        let (wp, wm) = omegas(TAU);
        let mut lat = Lattice::new(Size::new(8, 5), |_, _| false);
        lat.spare_mut().fill(7.0);
        lat.stream_collide(wp, wm, false);
        let (next, ..) = lat.next_and_moments();
        for plane in super::planes_mut(next, 40) {
            for row in plane.chunks_exact(8) {
                assert_eq!((row[0], row[7]), (7.0, 7.0));
                assert!(row[1..7].iter().all(|&v| v != 7.0));
            }
        }
    }

    #[test]
    fn lbm_a2_degenerate_lattices_do_not_panic() {
        let (wp, wm) = omegas(TAU);
        for (w, h) in [(1, 1), (1, 5), (5, 1), (2, 2)] {
            let mut lat = Lattice::new(Size::new(w, h), |_, _| false);
            lat.step_periodic(wp, wm);
            lat.stream_collide(wp, wm, false);
            lat.swap();
            assert!(lat.healthy((0.99, 1.01)));
        }
    }
}
