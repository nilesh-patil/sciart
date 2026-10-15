//! Excitable media shared by `dicty_aggregation` and `heart_fibrillation` (SPEC-002
//! Batch M13).
//!
//! Two kernels, one per kind of tissue:
//!
//! - [`Barkley`] (1991) on a rectangular [`Grid`] with the isotropic 9-point Laplacian:
//!   a fast FitzHugh–Nagumo-type medium with sharp fronts whose semi-implicit update keeps
//!   `u, v ∈ [0, 1]` exactly (DICT-A5). The excitability `b` may vary per node.
//! - [`AlievPanfilov`] (1996) on a masked [`Medium`] (any shape, no-flux walls at the mask
//!   edge): the cardiac model whose restitution lets spirals break up (HRT).
//!
//! Plus the analysis both spiral scenes need: [`count_singularities`] finds phase
//! singularities (spiral tips) by the winding of the excitation phase.

use std::f32::consts::{PI, TAU as TWO_PI};

use sciart_core::grid;
use sciart_core::prelude::*;

// ---- Barkley ----

/// Barkley's model `∂u/∂t = ∇²u + ε⁻¹u(1 − u)(u − u_th)`, `u_th = (v + b)/a`,
/// `∂v/∂t = u − v`, discretised with node spacing `h` and sub-step `tau` (time units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Barkley {
    pub a: f32,
    pub eps: f32,
    /// Node spacing in space units.
    pub h: f32,
    /// Sub-step in time units.
    pub tau: f32,
}

impl Barkley {
    /// Diffusion weight of one explicit stage, `τ/h²` (`D = 1`).
    pub const fn diff(&self) -> f32 {
        self.tau / (self.h * self.h)
    }

    /// Reaction weight `τ/ε`.
    pub const fn react(&self) -> f32 {
        self.tau / self.eps
    }

    /// Whether every stage of [`step`](Self::step) is a convex combination or a shrinking
    /// factor, which keeps `u, v ∈ [0, 1]` for any `b ≥ 0` without clamps: `(10/3)·τ/h² ≤ 1`
    /// (the 9-point stencil's centre weight), `τ/ε ≤ 1` and `τ ≤ 1`.
    pub const fn is_bounded(&self) -> bool {
        10.0 / 3.0 * self.diff() <= 1.0 && self.react() <= 1.0 && self.tau <= 1.0
    }

    /// One Lie-split sub-step in place: explicit diffusion (9-point Laplacian, Neumann
    /// walls), the reaction implicit on the decaying branch (`u < u_th`, a shrinking factor)
    /// and explicit on the growing one (bounded by 1 since `τ/ε ≤ 1`), then
    /// `v ← v + τ(u − v)`. `b(i)` is the excitability of node `i` (row-major); `lap` is
    /// scratch.
    #[inline]
    pub fn step(
        &self,
        u: &mut Grid<f32>,
        v: &mut Grid<f32>,
        lap: &mut Grid<f32>,
        b: impl Fn(usize) -> f32,
    ) {
        debug_assert!(self.is_bounded(), "unbounded Barkley stages: {self:?}");
        let (a, tau, diff, react) = (self.a, self.tau, self.diff(), self.react());
        grid::laplacian9(u, lap, Edges::Clip);
        let fields = u.as_mut_slice().iter_mut().zip(v.as_mut_slice());
        for (i, ((u, v), &l)) in fields.zip(lap.as_slice()).enumerate() {
            let ud = *u + diff * l;
            let u_th = (*v + b(i)) / a;
            let g = react * (1.0 - ud) * (ud - u_th);
            let ur = if ud < u_th {
                ud / (1.0 - g)
            } else {
                ud + g * ud
            };
            *v += tau * (ur - *v);
            *u = ur;
        }
    }
}

/// Excite (`u = 1`) every node within `r` nodes of `(cx, cy)` that has recovered to
/// `v < v_max`: a pacemaker firing into a medium, which leaves refractory nodes alone.
pub fn excite_disc(u: &mut Grid<f32>, v: &Grid<f32>, (cx, cy): (f64, f64), r: f64, v_max: f32) {
    let reach = r.ceil() as i32;
    let (x0, y0) = (cx.round() as i32, cy.round() as i32);
    for y in y0 - reach..=y0 + reach {
        for x in x0 - reach..=x0 + reach {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            let recovered = v.get(x, y).is_some_and(|&v| v < v_max);
            if dx * dx + dy * dy <= r * r && recovered {
                u.set(x, y, 1.0);
            }
        }
    }
}

// ---- Broken fronts (spiral seeds) ----

/// One broken front: a straight segment `a → b`, excited on the `n` side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Front {
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// Unit normal: the direction the front propagates.
    pub n: [f64; 2],
    /// `b` was extended to the wall, leaving `a` as the only free end.
    pub anchored: bool,
}

/// How a [`Front`] is painted: the excited strip's width, the refractory tail's width
/// (nodes), and `v` along the tail from the front to its end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrontStroke {
    pub front_w: f64,
    pub tail_w: f64,
    pub tail_v: (f64, f64),
}

/// Paint `front` into the fields: a `front_w`-node strip of `u = 1` on the `n` side of
/// the segment, backed by a `tail_w`-node strip whose `v` ramps down away from it.
pub fn paint_front(u: &mut Grid<f32>, v: &mut Grid<f32>, f: &Front, s: &FrontStroke) {
    let (front_w, tail_w, tail_v) = (s.front_w, s.tail_w, s.tail_v);
    let (d, len) = unit(f.a, f.b);
    let n = f.n;
    // Bounding box of the painted band, clipped to the lattice.
    let xs = [f.a[0], f.b[0]].map(|x| [x + n[0] * front_w, x - n[0] * tail_w]);
    let ys = [f.a[1], f.b[1]].map(|y| [y + n[1] * front_w, y - n[1] * tail_w]);
    let span = |c: [[f64; 2]; 2], hi: u16| {
        let lo = c.iter().flatten().fold(f64::INFINITY, |m, &x| m.min(x));
        let top = c.iter().flatten().fold(f64::NEG_INFINITY, |m, &x| m.max(x));
        (
            lo.floor().max(0.0) as i32,
            top.ceil().min(f64::from(hi) - 1.0) as i32,
        )
    };
    let (x0, x1) = span(xs, u.width());
    let (y0, y1) = span(ys, u.height());
    for y in y0..=y1 {
        for x in x0..=x1 {
            let rel = [f64::from(x) - f.a[0], f64::from(y) - f.a[1]];
            let along = rel[0] * d[0] + rel[1] * d[1];
            let q = rel[0] * n[0] + rel[1] * n[1];
            if !(0.0..=len).contains(&along) {
                continue;
            }
            if (0.0..front_w).contains(&q) {
                u.set(x, y, 1.0);
                v.set(x, y, 0.0);
            } else if (-tail_w..0.0).contains(&q) {
                let t = -q / tail_w;
                u.set(x, y, 0.0);
                v.set(x, y, sciart_core::ease::lerp(tail_v.0, tail_v.1, t) as f32);
            }
        }
    }
}

/// Unit vector from `a` to `b` and the distance (a zero-length segment points along x).
pub fn unit(a: [f64; 2], b: [f64; 2]) -> ([f64; 2], f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    if len > 0.0 {
        ([dx / len, dy / len], len)
    } else {
        ([1.0, 0.0], 0.0)
    }
}

// ---- Masked medium ----

/// A lattice region of any shape: the nodes inside a mask and each node's four
/// neighbours, a neighbour outside the mask (or the lattice) replaced by the node itself,
/// so the 5-point Laplacian sees a zero gradient there (no-flux wall).
#[derive(Debug, Clone)]
pub struct Medium {
    size: Size,
    /// Row-major indices of the nodes inside, ascending.
    inside: Vec<u32>,
    /// Per lattice node: west, east, north, south.
    nb: Vec<[u32; 4]>,
    /// Per lattice node: inside the mask.
    mask: Vec<bool>,
}

impl Default for Medium {
    /// An empty 1×1 lattice, the placeholder until the first `rebuild`.
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            inside: Vec::new(),
            nb: vec![[0; 4]],
            mask: vec![false],
        }
    }
}

impl Medium {
    /// Rebuild for a `size` lattice with `inside(x, y)` as the mask (reusing buffers).
    pub fn rebuild(&mut self, size: Size, inside: impl Fn(i32, i32) -> bool) {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        self.size = size;
        self.mask.clear();
        self.mask.extend(
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| inside(x, y)),
        );
        self.inside.clear();
        self.nb.clear();
        let mask = &self.mask;
        let idx = |x: i32, y: i32| (y * w + x) as usize;
        for y in 0..h {
            for x in 0..w {
                let i = idx(x, y) as u32;
                let pick = |nx: i32, ny: i32| {
                    let ok = (0..w).contains(&nx) && (0..h).contains(&ny) && mask[idx(nx, ny)];
                    if ok { idx(nx, ny) as u32 } else { i }
                };
                self.nb.push([
                    pick(x - 1, y),
                    pick(x + 1, y),
                    pick(x, y - 1),
                    pick(x, y + 1),
                ]);
                if mask[i as usize] {
                    self.inside.push(i);
                }
            }
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Row-major indices of the nodes inside the mask.
    pub fn inside(&self) -> &[u32] {
        &self.inside
    }

    /// Whether lattice node `i` is inside the mask (false off the lattice).
    pub fn contains(&self, i: usize) -> bool {
        self.mask.get(i).copied().unwrap_or(false)
    }

    /// Whether `(x, y)` is inside the mask (false off the lattice).
    pub fn contains_xy(&self, x: i32, y: i32) -> bool {
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        (0..w).contains(&x) && (0..h).contains(&y) && self.mask[(y * w + x) as usize]
    }

    /// The four neighbours of node `i` (walls folded back onto `i`).
    pub fn neighbours(&self, i: usize) -> [u32; 4] {
        self.nb[i]
    }

    /// `out[i] = Σ u[nb] − 4u[i]` for every inside node (other entries untouched).
    fn laplacian(&self, u: &[f32], out: &mut [f32]) {
        for &i in &self.inside {
            let i = i as usize;
            let [w, e, n, s] = self.nb[i].map(|j| u[j as usize]);
            out[i] = (w + e) + (n + s) - 4.0 * u[i];
        }
    }
}

// ---- Aliev–Panfilov ----

/// Aliev & Panfilov (1996): `∂u/∂t = D∇²u − ku(u − a)(u − 1) − uv`,
/// `∂v/∂t = ε(u, v)(−v − ku(u − a − 1))`, `ε = ε₀ + μ₁v/(u + μ₂)`, explicit Euler with
/// node spacing `h` and step `dt` (time units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlievPanfilov {
    pub k: f32,
    pub a: f32,
    pub eps0: f32,
    pub mu1: f32,
    pub mu2: f32,
    pub d: f32,
    pub h: f32,
    pub dt: f32,
}

/// `v` never exceeds this in the model's own dynamics (its nullcline tops out near
/// `k(1 + a)²/4 ≈ 2.6` for `k = 8`); the clamp only guards the display and tests.
pub const AP_V_MAX: f32 = 4.0;

impl AlievPanfilov {
    /// Explicit diffusion is stable for `4·D·dt/h² ≤ 1` (5-point stencil); the reaction
    /// is resolved when `k·dt` is small.
    pub const fn is_stable(&self) -> bool {
        4.0 * self.d * self.dt / (self.h * self.h) <= 1.0 && self.k * self.dt <= 0.5
    }

    /// One Euler step on every inside node; `u` is clamped to `[0, 1]` and `v` to
    /// `[0, AP_V_MAX]` (HRT-A5), and `eps_scale(i)` multiplies `ε` per node (tissue
    /// heterogeneity; `|_| 1.0` for none). `lap` is scratch of the lattice's size.
    pub fn step(
        &self,
        m: &Medium,
        u: &mut [f32],
        v: &mut [f32],
        lap: &mut [f32],
        eps_scale: impl Fn(usize) -> f32,
    ) {
        debug_assert!(self.is_stable(), "unstable Aliev-Panfilov step: {self:?}");
        m.laplacian(u, lap);
        let (k, a, dt) = (self.k, self.a, self.dt);
        let dd = self.d / (self.h * self.h);
        for &i in m.inside() {
            let i = i as usize;
            let (ui, vi) = (u[i], v[i]);
            let du = dd * lap[i] - k * ui * (ui - a) * (ui - 1.0) - ui * vi;
            let eps = eps_scale(i) * (self.eps0 + self.mu1 * vi / (ui + self.mu2));
            let dv = eps * (-vi - k * ui * (ui - a - 1.0));
            u[i] = (ui + dt * du).clamp(0.0, 1.0);
            v[i] = (vi + dt * dv).clamp(0.0, AP_V_MAX);
        }
    }
}

// ---- Phase singularities ----

/// Excitation phase `atan2(v − v*, u − u*)` of a node (radians, `(−π, π]`).
#[inline]
pub fn phase(u: f32, v: f32, (us, vs): (f32, f32)) -> f32 {
    (v - vs).atan2(u - us)
}

/// Wrap a phase difference into `(−π, π]`.
#[inline]
fn wrap(d: f32) -> f32 {
    if d > PI {
        d - TWO_PI
    } else if d <= -PI {
        d + TWO_PI
    } else {
        d
    }
}

/// Count phase singularities (spiral tips) of the fields on `m`: plaquettes of four
/// inside nodes around which the phase winds by ±2π. A tip found within `merge` nodes
/// (Chebyshev) of one already counted is the same tip seen twice and is not counted
/// again. The first `tips.len()` tips found are written to `tips` (lattice coordinates of
/// the plaquette's top-left node), row by row. `theta` and `taken` are scratch of the
/// lattice's size.
#[allow(clippy::too_many_arguments)] // two fields, two scratch buffers, one output
pub fn count_singularities(
    m: &Medium,
    u: &[f32],
    v: &[f32],
    centre: (f32, f32),
    merge: i32,
    theta: &mut [f32],
    taken: &mut [bool],
    tips: &mut [(i32, i32)],
) -> usize {
    let (w, h) = (i32::from(m.size().width()), i32::from(m.size().height()));
    for &i in m.inside() {
        let i = i as usize;
        theta[i] = phase(u[i], v[i], centre);
    }
    taken.fill(false);
    let mut count = 0;
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
            if !corners.iter().all(|&(cx, cy)| m.contains_xy(cx, cy)) {
                continue;
            }
            let t = corners.map(|(cx, cy)| theta[(cy * w + cx) as usize]);
            let winding: f32 = (0..4).map(|j| wrap(t[(j + 1) % 4] - t[j])).sum();
            if winding.abs() < PI {
                continue;
            }
            if taken[(y * w + x) as usize] {
                continue;
            }
            if let Some(tip) = tips.get_mut(count) {
                *tip = (x, y);
            }
            count += 1;
            for ny in (y - merge).max(0)..=(y + merge).min(h - 1) {
                for nx in (x - merge).max(0)..=(x + merge).min(w - 1) {
                    taken[(ny * w + nx) as usize] = true;
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    const BZ_LIKE: Barkley = Barkley {
        a: 0.75,
        eps: 0.02,
        h: 0.5,
        tau: 0.0125,
    };

    #[test]
    fn barkley_weights_and_bounds() {
        assert!(BZ_LIKE.is_bounded());
        assert_eq!(BZ_LIKE.diff(), 0.0125 / 0.25);
        assert!(
            !Barkley {
                tau: 0.1,
                ..BZ_LIKE
            }
            .is_bounded()
        );
    }

    #[test]
    fn medium_walls_fold_onto_the_node() {
        let mut m = Medium::default();
        // A plus shape: only (1, 0), (0, 1), (1, 1), (2, 1), (1, 2) inside a 3×3 lattice.
        m.rebuild(Size::new(3, 3), |x, y| x == 1 || y == 1);
        assert_eq!(m.inside(), &[1, 3, 4, 5, 7]);
        assert_eq!(m.neighbours(4), [3, 5, 1, 7]);
        assert_eq!(m.neighbours(1), [1, 1, 1, 4]);
        assert!(m.contains(4) && !m.contains(0) && !m.contains(99));
        // A uniform field has a zero Laplacian, walls included.
        let u = [0.7f32; 9];
        let mut lap = [9.0f32; 9];
        m.laplacian(&u, &mut lap);
        assert!(m.inside().iter().all(|&i| lap[i as usize] == 0.0));
    }

    fn spiral_fields(n: u16) -> (Medium, Vec<f32>, Vec<f32>) {
        let mut m = Medium::default();
        m.rebuild(Size::new(n, n), |_, _| true);
        let c = f32::from(n) / 2.0;
        let mut u = vec![0.0; usize::from(n) * usize::from(n)];
        let mut v = u.clone();
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (f32::from(x) + 0.5 - c, f32::from(y) + 0.5 - c);
                let a = dy.atan2(dx);
                let i = usize::from(y) * usize::from(n) + usize::from(x);
                u[i] = 0.5 + 0.5 * a.cos_det();
                v[i] = 0.5 + 0.5 * a.sin_det();
            }
        }
        (m, u, v)
    }

    #[test]
    fn a_single_vortex_is_one_singularity() {
        let (m, u, v) = spiral_fields(12);
        let n = u.len();
        let (mut theta, mut taken) = (vec![0.0; n], vec![false; n]);
        let mut tips = [(0, 0); 2];
        let c = count_singularities(&m, &u, &v, (0.5, 0.5), 2, &mut theta, &mut taken, &mut tips);
        assert_eq!(c, 1);
        assert_eq!(tips[0], (5, 5));
        // A uniform field has none.
        let flat = vec![0.2; n];
        let c = count_singularities(
            &m,
            &flat,
            &flat,
            (0.5, 0.5),
            2,
            &mut theta,
            &mut taken,
            &mut [],
        );
        assert_eq!(c, 0);
    }

    #[test]
    fn aliev_panfilov_rests_and_stays_in_range() {
        let ap = AlievPanfilov {
            k: 8.0,
            a: 0.15,
            eps0: 0.002,
            mu1: 0.2,
            mu2: 0.3,
            d: 1.0,
            h: 1.0,
            dt: 0.05,
        };
        assert!(ap.is_stable());
        let mut m = Medium::default();
        m.rebuild(Size::new(20, 1), |_, _| true);
        let (mut u, mut v, mut lap) = (vec![0.0; 20], vec![0.0; 20], vec![0.0; 20]);
        u[..3].fill(1.0);
        for _ in 0..4000 {
            ap.step(&m, &mut u, &mut v, &mut lap, |_| 1.0);
            assert!(u.iter().all(|x| (0.0..=1.0).contains(x)));
            assert!(v.iter().all(|x| (0.0..=AP_V_MAX).contains(x)));
        }
        assert!(u.iter().all(|&x| x < 0.01), "{u:?}");
    }
}
