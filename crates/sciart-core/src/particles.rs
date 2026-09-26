//! Particle helpers shared by the particle-system scenes (SPEC-001 CORE-R28..R32).
//!
//! Everything here is allocation-free after construction and deterministic given the
//! caller's RNG, so scenes stay inside the GEN-R8 frame budget and reproducible (GEN-R7).

use std::f64::consts::TAU;

use rand::Rng;

use crate::{Canvas, Glyph, Ink};

/// A bounded, unordered set of live particles (CORE-R28).
///
/// Backed by one `Vec<P>` allocated up front. The logical `capacity` is tracked
/// separately from the `Vec`'s so that shrinking never reallocates and a later grow
/// back to the original size reuses the buffer.
#[derive(Debug, Clone)]
pub struct ParticlePool<P> {
    items: Vec<P>,
    capacity: usize,
}

impl<P> ParticlePool<P> {
    /// An empty pool that will hold at most `capacity` particles (CORE-R28).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Add `p` if there is room; otherwise drop it and return `false` (CORE-R28).
    ///
    /// Rejecting instead of evicting keeps spawn cost O(1) and makes the particle
    /// budget a hard cap that emitters can observe.
    #[inline]
    pub fn spawn(&mut self, p: P) -> bool {
        if self.items.len() < self.capacity {
            // Never reallocates: the buffer was reserved for `capacity` elements.
            self.items.push(p);
            true
        } else {
            false
        }
    }

    /// Update every particle in place and drop those for which `f` returns `false`
    /// (CORE-R28). Order is **not** preserved.
    ///
    /// Swap-remove compaction: a dead particle is replaced by the last one, which is
    /// then visited at the same index. That is one pass, no shifting of the tail, and
    /// no allocation — cheaper than `Vec::retain_mut` when many particles die at once,
    /// and ordering is meaningless for particles anyway.
    pub fn retain(&mut self, mut f: impl FnMut(&mut P) -> bool) {
        let mut i = 0;
        while i < self.items.len() {
            if f(&mut self.items[i]) {
                i += 1;
            } else {
                self.items.swap_remove(i);
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Maximum number of live particles.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn iter(&self) -> std::slice::Iter<'_, P> {
        self.items.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, P> {
        self.items.iter_mut()
    }

    /// Remove every particle, keeping the buffer.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Change the particle budget, e.g. on resize (CORE-R28).
    ///
    /// Shrinking truncates (which particles survive is unspecified) and keeps the
    /// buffer; growing reserves exactly the extra room — the only allocating operation.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.items.truncate(capacity);
        // `reserve_exact` is relative to `len`, and is a no-op when the buffer is already big enough.
        self.items.reserve_exact(capacity - self.items.len());
        self.capacity = capacity;
    }
}

impl<'a, P> IntoIterator for &'a ParticlePool<P> {
    type Item = &'a P;
    type IntoIter = std::slice::Iter<'a, P>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, P> IntoIterator for &'a mut ParticlePool<P> {
    type Item = &'a mut P;
    type IntoIter = std::slice::IterMut<'a, P>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Fixed-length history of the last `n` values, e.g. a comet tail (CORE-R29).
///
/// A ring buffer over a `Vec<T>` reserved once: while filling, values are appended;
/// once full, `head` marks the oldest slot and each push overwrites it. `T: Copy`
/// keeps overwrites trivially cheap and avoids needing `T: Default` for pre-fill.
#[derive(Debug, Clone)]
pub struct Trail<T: Copy> {
    buf: Vec<T>,
    head: usize,
    capacity: usize,
}

impl<T: Copy> Trail<T> {
    /// A trail remembering at most `n` values; `n = 0` keeps nothing (CORE-R29).
    pub fn new(n: usize) -> Self {
        Self {
            buf: Vec::with_capacity(n),
            head: 0,
            capacity: n,
        }
    }

    /// Append `v`, evicting the oldest value when full (CORE-R29).
    #[inline]
    pub fn push(&mut self, v: T) {
        if self.buf.len() < self.capacity {
            self.buf.push(v);
        } else if self.capacity > 0 {
            self.buf[self.head] = v;
            self.head = (self.head + 1) % self.capacity;
        }
    }

    /// Values from oldest to newest (CORE-R29).
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + '_ {
        let (newer, older) = self.buf.split_at(self.head);
        older.iter().chain(newer)
    }

    /// The most recently pushed value, if any.
    pub fn newest(&self) -> Option<T> {
        self.iter().next_back().copied()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Maximum number of remembered values.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.head = 0;
    }
}

/// One Box–Muller draw: a pair of independent standard normal variates.
///
/// `1 - u` maps rand's `[0, 1)` onto `(0, 1]`, so `ln` never sees 0 and the radius is
/// finite (at most `√(-2 ln 2⁻⁵³) ≈ 8.6`).
#[inline]
fn box_muller(rng: &mut impl Rng) -> (f64, f64) {
    let u1 = 1.0 - rng.random::<f64>();
    let u2 = rng.random::<f64>();
    let r = (-2.0 * u1.ln()).sqrt();
    let (s, c) = (TAU * u2).sin_cos();
    (r * c, r * s)
}

/// Clean a standard deviation: non-finite ⇒ 0, negative ⇒ its magnitude.
#[inline]
fn clean_sd(sd: f64) -> f64 {
    if sd.is_finite() { sd.abs() } else { 0.0 }
}

/// Map a non-finite result back to `fallback`, upholding the "always finite" contract.
#[inline]
fn finite_or(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v } else { fallback }
}

/// A normal variate `N(mean, sd²)` (CORE-R30).
///
/// Always finite: non-finite `mean` is treated as 0, non-finite `sd` as 0, negative `sd`
/// as `|sd|`, and overflow falls back to `mean`. `sd == 0` returns exactly `mean`.
/// The RNG is advanced identically regardless of the parameters, so changing `sd` never
/// shifts the rest of a scene's random stream.
pub fn gaussian(rng: &mut impl Rng, mean: f64, sd: f64) -> f64 {
    let (z, _) = box_muller(rng);
    let mean = finite_or(mean, 0.0);
    let sd = clean_sd(sd);
    if sd == 0.0 {
        return mean;
    }
    finite_or(mean + sd * z, mean)
}

/// An isotropic 2-D normal pair `(x, y)`, each `N(0, sd²)` (CORE-R30).
///
/// Uses both Box–Muller outputs, so it costs the same as one [`gaussian`] call.
/// Same sanitising rules as [`gaussian`]; overflow collapses to `(0, 0)`.
pub fn gaussian2(rng: &mut impl Rng, sd: f64) -> (f64, f64) {
    let (zx, zy) = box_muller(rng);
    let sd = clean_sd(sd);
    if sd == 0.0 {
        return (0.0, 0.0);
    }
    let (x, y) = (sd * zx, sd * zy);
    if x.is_finite() && y.is_finite() {
        (x, y)
    } else {
        (0.0, 0.0)
    }
}

/// Valid Kepler inputs: finite `gm ≥ 0` and finite `r > 0`.
#[inline]
fn kepler_ok(gm: f64, r: f64) -> bool {
    gm.is_finite() && gm >= 0.0 && r.is_finite() && r > 0.0
}

/// Circular-orbit speed `√(gm / r)` (CORE-R31); 0 for invalid input or overflow.
pub fn kepler_speed(gm: f64, r: f64) -> f64 {
    if !kepler_ok(gm, r) {
        return 0.0;
    }
    finite_or((gm / r).sqrt(), 0.0)
}

/// Circular-orbit angular velocity `√(gm / r³)` (CORE-R31); 0 for invalid input or overflow.
///
/// Computed as `√(gm / r) / r` so `r³` never overflows/underflows on its own.
pub fn kepler_omega(gm: f64, r: f64) -> f64 {
    if !kepler_ok(gm, r) {
        return 0.0;
    }
    finite_or((gm / r).sqrt() / r, 0.0)
}

/// Plot one glyph at fractional cell coordinates, rounded to the nearest cell (CORE-R32).
///
/// Bounds are checked in `f64` *before* casting, so NaN/∞/huge values are ignored
/// rather than saturating into a valid-looking `i32` edge cell.
#[inline]
pub fn plot(canvas: &mut Canvas, x: f64, y: f64, glyph: Glyph, ink: Ink) {
    let (x, y) = (x.round(), y.round());
    let (w, h) = (
        f64::from(canvas.size().width()),
        f64::from(canvas.size().height()),
    );
    // Range checks are false for NaN, so this also filters non-finite input.
    if (0.0..w).contains(&x) && (0.0..h).contains(&y) {
        canvas.put(x as i32, y as i32, glyph, ink);
    }
}
