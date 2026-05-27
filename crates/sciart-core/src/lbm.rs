//! D2Q9 lattice Boltzmann in lattice units (CORE-R98, CORE-R99), lifted from
//! `vortex_street` for M13 so every flow scene runs the same, tested kernel.
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

use crate::{Grid, Size};

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
const PAIRS: [(usize, usize); 4] = [(1, 3), (2, 4), (5, 7), (6, 8)];
/// Nodes collided together in the inner loop: one 128-bit SIMD register of f32 (NEON,
/// SSE2). Measured 7 ns/node against 29 ns unbatched; 8 and 16 lanes spill.
const LANES: usize = 4;

/// `τ⁻` from the magic parameter `Λ = (τ − ½)(τ⁻ − ½)` (CORE-R98).
pub fn tau_minus(tau: f64, magic: f64) -> f64 {
    0.5 + magic / (tau - 0.5)
}

/// Shifted equilibrium `f_i^eq − w_i = w_i (δρ + ρ(3 c·u + 4.5 (c·u)² − 1.5 u²))` with
/// `ρ = 1 + δρ` (CORE-R98).
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
/// `(ρ, uₓ, u_y)`, which the collision conserves (CORE-R98).
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

/// Populations, moments and the solid mask of one lattice (CORE-R99).
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
    pub fn populations(&self) -> &[f32] {
        &self.f
    }

    pub fn populations_mut(&mut self) -> &mut [f32] {
        &mut self.f
    }

    /// The spare buffer's planes (zeroed by [`Self::new`]).
    pub fn spare(&self) -> &[f32] {
        &self.next
    }

    /// The spare buffer's planes, mutable.
    pub fn spare_mut(&mut self) -> &mut [f32] {
        &mut self.next
    }

    pub fn rho(&self) -> &Grid<f32> {
        &self.rho
    }

    pub fn ux(&self) -> &Grid<f32> {
        &self.ux
    }

    pub fn uy(&self) -> &Grid<f32> {
        &self.uy
    }

    pub fn rho_mut(&mut self) -> &mut Grid<f32> {
        &mut self.rho
    }

    pub fn ux_mut(&mut self) -> &mut Grid<f32> {
        &mut self.ux
    }

    pub fn uy_mut(&mut self) -> &mut Grid<f32> {
        &mut self.uy
    }

    /// The moment grids `(ρ, uₓ, u_y)` for writing, e.g. before [`Self::equilibrate`].
    pub fn moments_mut(&mut self) -> (&mut Grid<f32>, &mut Grid<f32>, &mut Grid<f32>) {
        (&mut self.rho, &mut self.ux, &mut self.uy)
    }

    /// The spare buffer just written by [`Self::stream_collide`] and the moment grids,
    /// for the caller's boundary conditions before [`Self::swap`].
    pub fn next_and_moments(
        &mut self,
    ) -> (&mut [f32], &mut Grid<f32>, &mut Grid<f32>, &mut Grid<f32>) {
        (&mut self.next, &mut self.rho, &mut self.ux, &mut self.uy)
    }

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
