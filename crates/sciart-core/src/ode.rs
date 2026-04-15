//! Fixed-step ODE integration for deterministic physics scenes (SPEC-001 CORE-R37,
//! CORE-R44).
//!
//! RK4 state is a const-generic `[f64; N]` so every stage lives on the stack: no
//! allocation, no trait objects, and the closure is monomorphised into a tight loop.
//! Classical RK4 is the sweet spot for a 60 Hz visual simulation: 4th-order accurate
//! with four evaluations, and far better behaved than Euler on orbits and pendulums.
//!
//! Long-lived conservative systems (N-body, choreographies) instead want the symplectic
//! kick–drift–kick leapfrog ([`verlet_step`]): 2nd order, one force evaluation per step,
//! time-reversible, and with a bounded energy error where RK4 slowly drifts. It works on
//! caller-owned slices because N-body state is sized at runtime.

/// Smallest substep `integrate` will use; smaller, zero, negative or NaN `h_max` is
/// raised to this (CORE-R37).
pub const MIN_STEP: f64 = 1e-6;

/// Upper bound on substeps per `integrate` call (CORE-R37).
///
/// Keeps the per-frame cost bounded (GEN-R8) whatever `dt`/`h_max` a caller passes:
/// at the 1/60 s sim step this still allows substeps of ~4 µs of simulated time.
pub const MAX_STEPS: usize = 4096;

/// Relative slack when counting substeps, so that exact multiples such as
/// `integrate(.., 0.9, 0.3, ..)` take 3 steps rather than `ceil(3.0000000000000004) = 4`.
/// The resulting step exceeds `h_max` by at most this fraction — irrelevant for accuracy.
const COUNT_SLACK: f64 = 1e-9;

/// `y + a * k`, element-wise, on the stack.
///
/// Plain multiply-add rather than `f64::mul_add`: without a hardware FMA target feature
/// (baseline x86_64) `mul_add` becomes a libm call, and results would differ by platform.
#[inline(always)]
fn axpy<const N: usize>(y: &[f64; N], a: f64, k: &[f64; N]) -> [f64; N] {
    std::array::from_fn(|i| y[i] + a * k[i])
}

/// Advance `y' = f(t, y)` by one classical Runge–Kutta 4 step of size `h` (CORE-R37).
///
/// Exact for solutions that are polynomials of degree ≤ 4 in `t`; global error
/// `O(h⁴)` otherwise. Non-finite values produced by `f` propagate into `y` untouched —
/// detecting blow-up is the caller's policy, not the integrator's.
#[inline]
pub fn rk4_step<const N: usize, F>(y: &mut [f64; N], t: f64, h: f64, mut f: F)
where
    F: FnMut(f64, &[f64; N]) -> [f64; N],
{
    let half = 0.5 * h;
    let k1 = f(t, y);
    let k2 = f(t + half, &axpy(y, half, &k1));
    let k3 = f(t + half, &axpy(y, half, &k2));
    let k4 = f(t + h, &axpy(y, h, &k3));
    let sixth = h / 6.0;
    for i in 0..N {
        // Pairwise grouping keeps the weighted sum symmetric and reproducible.
        let slope = (k1[i] + k4[i]) + 2.0 * (k2[i] + k3[i]);
        y[i] += sixth * slope;
    }
}

/// Integrate `y' = f(t, y)` from `t` to `t + dt` in equal RK4 substeps no longer than
/// `h_max`, returning how many were taken (CORE-R37).
///
/// - `dt ≤ 0` or non-finite: nothing happens, `f` is never called, returns 0.
/// - Steps: `ceil(dt / h_max)` (with a 1e-9 relative slack for exact multiples), at
///   least 1 and at most [`MAX_STEPS`].
/// - `h_max` below [`MIN_STEP`] (including 0, negative, NaN) is raised to `MIN_STEP`;
///   `h_max = +∞` means a single step.
///
/// Substep times are computed as `t + i·h`, not accumulated, so they do not drift.
pub fn integrate<const N: usize, F>(
    y: &mut [f64; N],
    t: f64,
    dt: f64,
    h_max: f64,
    mut f: F,
) -> usize
where
    F: FnMut(f64, &[f64; N]) -> [f64; N],
{
    let steps = substeps(dt, h_max);
    let h = dt / steps as f64;
    for i in 0..steps {
        rk4_step(y, t + i as f64 * h, h, &mut f);
    }
    steps
}

/// Number of equal substeps covering `dt` with none longer than `h_max`: the count rule
/// shared by [`integrate`] and [`verlet_integrate`] (CORE-R37, CORE-R44). 0 for `dt ≤ 0`
/// or non-finite `dt`, otherwise in `1..=MAX_STEPS`.
fn substeps(dt: f64, h_max: f64) -> usize {
    if !(dt.is_finite() && dt > 0.0) {
        return 0;
    }
    // `f64::max` returns the non-NaN operand, so NaN also lands on MIN_STEP.
    let h_max = h_max.max(MIN_STEP);
    // Saturating float->int cast; the clamp covers both underflow (tiny dt / huge h_max)
    // and the MAX_STEPS budget.
    ((dt / h_max - COUNT_SLACK).ceil() as usize).clamp(1, MAX_STEPS)
}

/// `v += a * k`, element-wise (plain multiply-add like [`axpy`], for determinism).
#[inline(always)]
fn add_scaled(v: &mut [f64], a: f64, k: &[f64]) {
    for (v, k) in v.iter_mut().zip(k) {
        *v += a * k;
    }
}

/// One kick–drift–kick leapfrog (velocity Verlet) step of size `h` for `x'' = a(x)`
/// (CORE-R44).
///
/// `accel(pos, acc_out)` must overwrite `acc_out` with the accelerations at `pos`.
/// On entry `acc` must hold `a(pos)`; on exit it holds `a` at the new position, so
/// consecutive steps cost one force evaluation each. Mismatched slice lengths make the
/// step a no-op (and `accel` is not called).
#[inline]
pub fn verlet_step<F>(pos: &mut [f64], vel: &mut [f64], acc: &mut [f64], h: f64, mut accel: F)
where
    F: FnMut(&[f64], &mut [f64]),
{
    if pos.len() != vel.len() || pos.len() != acc.len() {
        return;
    }
    let half = 0.5 * h;
    add_scaled(vel, half, acc);
    add_scaled(pos, h, vel);
    accel(pos, acc);
    add_scaled(vel, half, acc);
}

/// Advance `x'' = a(x)` by `dt` in equal leapfrog substeps no longer than `h_max`,
/// returning how many were taken (CORE-R44).
///
/// Step-count rules are those of [`integrate`] (`MIN_STEP`, `MAX_STEPS`, none for
/// `dt ≤ 0` or non-finite `dt`); mismatched slice lengths take no steps. The `acc`
/// contract is that of [`verlet_step`]: prime it once with `accel(pos, acc)` after
/// building the state, then carry it alongside `pos` and `vel`.
pub fn verlet_integrate<F>(
    pos: &mut [f64],
    vel: &mut [f64],
    acc: &mut [f64],
    dt: f64,
    h_max: f64,
    mut accel: F,
) -> usize
where
    F: FnMut(&[f64], &mut [f64]),
{
    if pos.len() != vel.len() || pos.len() != acc.len() {
        return 0;
    }
    let steps = substeps(dt, h_max);
    let h = dt / steps as f64;
    for _ in 0..steps {
        verlet_step(pos, vel, acc, h, &mut accel);
    }
    steps
}
