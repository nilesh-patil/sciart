//! Semi-Lagrangian transport of scalar fields (dye, cloud water) by a prescribed,
//! kinematic velocity on a capped lattice (M13 shared model: `jupiter_storms`,
//! `hurricane`; ADR-0012 caps).
//!
//! The flow is not solved: a scene writes node velocities (lattice cells per second) into
//! a [`Flow`], usually as differences of a stream function across each cell (the form of
//! `stream_velocity`, which scenes reproduce with cached factors), which makes the
//! discrete field exactly divergence-free, and [`advect`] carries a field along it. Each
//! node traces its departure point back with a midpoint (RK2) step and takes the bilinear
//! value there, so the scheme is unconditionally stable for any `dt` (GEN-R4) at the price
//! of a little numerical diffusion, which the scenes counter with their sources.
//!
//! Lattice coordinates: node `(i, j)` sits at `(x, y) = (i, j)`, `y` down the screen.
//! Everything here is allocation-free once the grids are sized (GEN-R11).

use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Grid, Size};

/// What lies beyond the left and right edges: a planet's longitude wraps round, a storm's
/// box is clamped (its edge value is repeated). Rows are always clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XEdge {
    Wrap,
    Clamp,
}

/// Node velocities in lattice cells per second.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub u: Grid<f32>,
    pub v: Grid<f32>,
}

impl Flow {
    /// Resize both components to `lattice` (contents are rewritten by the scene).
    pub fn resize(&mut self, lattice: Size) {
        self.u.resize(lattice, 0.0);
        self.v.resize(lattice, 0.0);
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Set every node's velocity to `f(x, y)`.
    pub fn fill_with(&mut self, mut f: impl FnMut(f64, f64) -> (f64, f64)) {
        let w = usize::from(self.u.width()).max(1);
        let (us, vs) = (self.u.as_mut_slice(), self.v.as_mut_slice());
        for (k, (u, v)) in us.iter_mut().zip(vs.iter_mut()).enumerate() {
            let (a, b) = f((k % w) as f64, (k / w) as f64);
            *u = a as f32;
            *v = b as f32;
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
/// The velocity of stream function `psi` at `(x, y)`: `u = ∂ψ/∂y`, `v = −∂ψ/∂x`, each as
/// the difference of ψ across a cell of side `h` centred on the point. The discrete
/// divergence built from the same differences cancels term by term (JUP-A1), so the
/// sampled flow neither creates nor destroys dye. With `y` down the screen a positive
/// bump of ψ turns clockwise on screen and a negative one counter-clockwise.
#[inline]
pub fn stream_velocity(psi: impl Fn(f64, f64) -> f64, x: f64, y: f64, h: f64) -> (f64, f64) {
    let e = 0.5 * h;
    let u = (psi(x, y + e) - psi(x, y - e)) / h;
    let v = -(psi(x + e, y) - psi(x - e, y)) / h;
    (u, v)
}

/// Bilinear value of `g` at lattice point `(x, y)`, clamped in `y` and wrapped or clamped
/// in `x` by `edge`; 0 for an empty grid or a non-finite point.
#[inline]
pub fn sample(g: &Grid<f32>, x: f32, y: f32, edge: XEdge) -> f32 {
    let (w, h) = (usize::from(g.width()), usize::from(g.height()));
    if w == 0 || h == 0 || !(x.is_finite() && y.is_finite()) {
        return 0.0;
    }
    let cells = g.as_slice();
    let (wf, hf) = (w as f32, h as f32);
    let y = y.clamp(0.0, hf - 1.0);
    let (y0, fy) = (y.floor(), y - y.floor());
    let j0 = y0 as usize;
    let j1 = (j0 + 1).min(h - 1);
    let (i0, i1, fx) = match edge {
        XEdge::Wrap => {
            let x = x.rem_euclid(wf);
            let x0 = x.floor();
            // `rem_euclid` can round up to exactly `wf`; fold that back onto node 0.
            let i0 = (x0 as usize) % w;
            (i0, (i0 + 1) % w, x - x0)
        }
        XEdge::Clamp => {
            let x = x.clamp(0.0, wf - 1.0);
            let x0 = x.floor();
            let i0 = x0 as usize;
            (i0, (i0 + 1).min(w - 1), x - x0)
        }
    };
    let (r0, r1) = (&cells[j0 * w..][..w], &cells[j1 * w..][..w]);
    let top = r0[i0] + fx * (r0[i1] - r0[i0]);
    let bottom = r1[i0] + fx * (r1[i1] - r1[i0]);
    top + fy * (bottom - top)
}

/// Call `f(k, x, y)` with the departure point `(x, y)` of every node `k` (row-major):
/// where the fluid now at the node was `dt` seconds ago, traced back along a midpoint
/// (RK2) characteristic. A scene carrying several fields samples each of them there.
pub fn departures(flow: &Flow, dt: f32, edge: XEdge, mut f: impl FnMut(usize, f32, f32)) {
    let w = usize::from(flow.u.width());
    if w == 0 || flow.u.size() != flow.v.size() {
        return;
    }
    let (us, vs) = (flow.u.as_slice(), flow.v.as_slice());
    for (k, (&u, &v)) in us.iter().zip(vs).enumerate() {
        let (x, y) = ((k % w) as f32, (k / w) as f32);
        let (xm, ym) = (x - 0.5 * dt * u, y - 0.5 * dt * v);
        let (um, vm) = (sample(&flow.u, xm, ym, edge), sample(&flow.v, xm, ym, edge));
        f(k, x - dt * um, y - dt * vm);
    }
}

/// Carry `src` along `flow` for `dt` seconds into `dst` (resized to `src`): each node
/// takes the bilinear value of `src` at its [`departures`] point. `flow` must have
/// `src`'s size, else `dst` is left zeroed.
pub fn advect(src: &Grid<f32>, dst: &mut Grid<f32>, flow: &Flow, dt: f32, edge: XEdge) {
    dst.resize(src.size(), 0.0);
    if flow.u.size() != src.size() {
        dst.fill(0.0);
        return;
    }
    let out = dst.as_mut_slice();
    departures(flow, dt, edge, |k, x, y| out[k] = sample(src, x, y, edge));
}

/// Value noise that wraps every `period` units in `x` (a planet's longitude), so the
/// seam never shows: `sciart_core::rng::value_noise` (CORE-R97) with unit cells, except
/// that the lattice column index is taken modulo `period`, which the core form cannot do.
#[inline]
pub fn value_noise_wrap_x(salt: u64, x: f64, y: f64, period: i32) -> f64 {
    if !(x.is_finite() && y.is_finite()) || period < 1 {
        return 0.0;
    }
    let p = f64::from(period);
    let x = x.rem_euclid(p);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = ((x0 as i32).rem_euclid(period), y0 as i32);
    let at =
        |di: i32, dj: i32| hash01(salt ^ cell_key((i + di).rem_euclid(period), j.wrapping_add(dj)));
    let top = at(0, 0) + sx * (at(1, 0) - at(0, 0));
    let bottom = at(0, 1) + sx * (at(1, 1) - at(0, 1));
    top + sy * (bottom - top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::Trig;

    fn gaussian_psi(x: f64, y: f64) -> f64 {
        let (dx, dy) = (x - 10.0, y - 6.0);
        -30.0 * (-(dx * dx / 16.0 + dy * dy / 4.0)).exp() + 0.7 * (0.4 * y).sin_det()
    }

    #[test]
    fn stream_velocity_is_discretely_divergence_free() {
        let h = 1.0;
        for &(x, y) in &[(3.0, 4.0), (10.2, 6.1), (12.7, 3.3), (0.5, 9.5)] {
            let e = 0.5 * h;
            let (ue, _) = stream_velocity(gaussian_psi, x + e, y, h);
            let (uw, _) = stream_velocity(gaussian_psi, x - e, y, h);
            let (_, vs) = stream_velocity(gaussian_psi, x, y + e, h);
            let (_, vn) = stream_velocity(gaussian_psi, x, y - e, h);
            let div = (ue - uw + vs - vn) / h;
            assert!(div.abs() < 1e-9, "div {div} at ({x}, {y})");
        }
    }

    #[test]
    fn sample_wraps_and_clamps() {
        let mut g = Grid::new(Size::new(4, 2), 0.0f32);
        g.set(3, 0, 1.0);
        // Halfway from the last column to the first (wrapped) column.
        assert!((sample(&g, 3.5, 0.0, XEdge::Wrap) - 0.5).abs() < 1e-6);
        assert!((sample(&g, -0.5, 0.0, XEdge::Wrap) - 0.5).abs() < 1e-6);
        assert!((sample(&g, 9.0, -3.0, XEdge::Clamp) - 1.0).abs() < 1e-6);
        assert_eq!(sample(&g, f32::NAN, 0.0, XEdge::Wrap), 0.0);
        assert_eq!(sample(&Grid::default(), 1.0, 1.0, XEdge::Wrap), 0.0);
    }

    #[test]
    fn uniform_flow_translates_a_field() {
        let size = Size::new(16, 4);
        let mut src = Grid::new(size, 0.0f32);
        src.set(5, 2, 1.0);
        let mut flow = Flow::default();
        flow.resize(size);
        flow.fill_with(|_, _| (2.0, 0.0));
        let mut dst = Grid::default();
        advect(&src, &mut dst, &flow, 0.5, XEdge::Wrap);
        assert!((dst.get(6, 2).copied().unwrap_or(0.0) - 1.0).abs() < 1e-6);
        assert!(dst.get(5, 2).copied().unwrap_or(1.0).abs() < 1e-6);
    }

    #[test]
    fn value_noise_wrap_x_is_the_core_noise_made_periodic() {
        for k in 0..200 {
            let (x, y) = (f64::from(k) * 0.37, f64::from(k) * 0.11);
            let xr = x.rem_euclid(8.0);
            if xr < 7.0 {
                // Away from the seam column it is CORE-R97's noise exactly.
                let core = sciart_core::rng::value_noise(3, xr, y, 1.0);
                assert!((core - value_noise_wrap_x(3, x, y, 8)).abs() < 1e-12);
            }
            let a = value_noise_wrap_x(3, x, y, 8);
            let b = value_noise_wrap_x(3, x + 8.0, y, 8);
            assert!((a - b).abs() < 1e-12);
        }
        assert_eq!(value_noise_wrap_x(1, f64::NAN, 0.0, 8), 0.0);
    }
}
