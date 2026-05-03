//! Particle helpers shared by the particle-system scenes (SPEC-001 CORE-R28..R32,
//! CORE-R54, CORE-R58).
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

/// An empty pool of capacity 0, the placeholder until a scene sizes it with
/// [`ParticlePool::set_capacity`]; allocates nothing.
impl<P> Default for ParticlePool<P> {
    fn default() -> Self {
        Self::with_capacity(0)
    }
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

/// A uniform-grid cell list for fixed-radius neighbour queries (boids, molecular
/// dynamics, SPH) over `[0, width) × [0, height)` (CORE-R54).
///
/// [`rebuild`](Self::rebuild) counting-sorts the point indices by bucket into one flat
/// array (CSR layout: `starts[b]..starts[b + 1]` is bucket `b`), so a query reads a few
/// contiguous runs instead of chasing per-bucket `Vec`s. With the bucket side at least
/// the query radius, a query touches ≤ 9 buckets and the total cost is O(n) per frame
/// instead of O(n²).
#[derive(Debug, Clone)]
pub struct SpatialHash {
    nx: usize,
    ny: usize,
    /// Buckets per unit length along x and y.
    inv_x: f64,
    inv_y: f64,
    /// `nx·ny + 1` prefix offsets into `order`.
    starts: Vec<u32>,
    /// Point indices grouped by bucket, ascending within each bucket.
    order: Vec<u32>,
    /// Bucket of each point, scratch for `rebuild`.
    keys: Vec<u32>,
}

impl Default for SpatialHash {
    /// A one-bucket unit square, the placeholder until a scene's first `resize`.
    fn default() -> Self {
        Self::new(1.0, 1.0, 1.0)
    }
}

impl SpatialHash {
    /// Upper bound on buckets per axis, so a tiny `cell` cannot allocate without bound.
    pub const MAX_AXIS: usize = 1024;

    /// A cell list over `[0, width) × [0, height)` with square buckets of side `cell`
    /// (rounded so they tile the domain; at most [`MAX_AXIS`](Self::MAX_AXIS) per
    /// axis). A non-finite or non-positive `width`/`height` is taken as 1, and such a
    /// `cell` as the whole domain (one bucket).
    pub fn new(width: f64, height: f64, cell: f64) -> Self {
        let mut hash = Self {
            nx: 1,
            ny: 1,
            inv_x: 1.0,
            inv_y: 1.0,
            starts: Vec::new(),
            order: Vec::new(),
            keys: Vec::new(),
        };
        hash.resize(width, height, cell);
        hash
    }

    /// Change the domain and bucket size (see [`new`](Self::new)) and forget the
    /// points; allocates only when the bucket count grows.
    pub fn resize(&mut self, width: f64, height: f64, cell: f64) {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let width = if positive(width) { width } else { 1.0 };
        let height = if positive(height) { height } else { 1.0 };
        let cell = if positive(cell) {
            cell
        } else {
            width.max(height)
        };
        // Saturating float→int cast, then the axis cap.
        let buckets = |len: f64| ((len / cell).ceil() as usize).clamp(1, Self::MAX_AXIS);
        (self.nx, self.ny) = (buckets(width), buckets(height));
        self.inv_x = self.nx as f64 / width;
        self.inv_y = self.ny as f64 / height;
        self.starts.clear();
        self.starts.resize(self.nx * self.ny + 1, 0);
        self.order.clear();
        self.keys.clear();
    }

    /// Bucket index along one axis: out-of-range values clamp to the border bucket, and
    /// NaN goes to bucket 0.
    #[inline]
    fn axis(v: f64, inv: f64, n: usize) -> usize {
        let b = (v * inv).floor();
        // NaN (from a NaN coordinate) and negatives go to bucket 0; the cast saturates.
        if b.is_nan() || b < 0.0 {
            0
        } else {
            (b as usize).min(n - 1)
        }
    }

    /// Re-bucket `points` (by index) in one counting sort: O(n + buckets), stable
    /// within a bucket, allocation-free unless the point count grows. Points outside
    /// the domain or non-finite clamp to the border bucket. Indices beyond `u32::MAX`
    /// are ignored.
    pub fn rebuild(&mut self, points: &[[f64; 2]]) {
        let points = &points[..points.len().min(u32::MAX as usize)];
        let nb = self.nx * self.ny;
        self.starts.fill(0);
        self.keys.clear();
        for p in points {
            let b = Self::axis(p[1], self.inv_y, self.ny) * self.nx
                + Self::axis(p[0], self.inv_x, self.nx);
            // b < nb ≤ MAX_AXIS², which fits in u32.
            self.keys.push(b as u32);
            self.starts[b + 1] += 1;
        }
        for b in 1..=nb {
            self.starts[b] += self.starts[b - 1];
        }
        // Scatter: `starts[b]` serves as bucket b's write cursor, ending at the start
        // of bucket b + 1; shifting right by one restores the offsets.
        self.order.clear();
        self.order.resize(points.len(), 0);
        for (i, &b) in self.keys.iter().enumerate() {
            let slot = &mut self.starts[b as usize];
            self.order[*slot as usize] = i as u32;
            *slot += 1;
        }
        self.starts.copy_within(0..nb, 1);
        self.starts[0] = 0;
    }

    /// Number of points in the last [`rebuild`](Self::rebuild).
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Call `f(i)` for every point index `i` whose point lies within distance `r` of
    /// `p`, and possibly for others in the buckets the query disc overlaps, each exactly
    /// once, bucket rows top to bottom and ascending index within a bucket. Callers
    /// filter by exact distance. A non-finite `p` or a negative or NaN `r` visits
    /// nothing; `r = ∞` visits every point.
    #[inline]
    pub fn for_each_within(&self, p: [f64; 2], r: f64, mut f: impl FnMut(usize)) {
        if !(p[0].is_finite() && p[1].is_finite()) || r.is_nan() || r < 0.0 {
            return;
        }
        let (x0, x1) = (
            Self::axis(p[0] - r, self.inv_x, self.nx),
            Self::axis(p[0] + r, self.inv_x, self.nx),
        );
        let (y0, y1) = (
            Self::axis(p[1] - r, self.inv_y, self.ny),
            Self::axis(p[1] + r, self.inv_y, self.ny),
        );
        for by in y0..=y1 {
            // Buckets x0..=x1 of one row are adjacent, so their points are one run.
            let row = by * self.nx;
            let (lo, hi) = (self.starts[row + x0], self.starts[row + x1 + 1]);
            for &i in &self.order[lo as usize..hi as usize] {
                f(i as usize);
            }
        }
    }
}

/// Inverse-transform sampling from a tabulated 1-D density (CORE-R58): radial profiles
/// of orbitals, spectral line shapes, speed distributions.
///
/// [`build`](Self::build) integrates the density over `K` equal bins (midpoint rule)
/// and stores `K` quantiles `F⁻¹(j / (K − 1))` of the resulting piecewise-linear CDF in
/// an inline array, so it never allocates; [`sample`](Self::sample) interpolates them
/// in O(1). Quantiles skip zero-density bins, though interpolation between two quantiles
/// can cross a zero-density gap narrower than one quantile step.
#[derive(Debug, Clone)]
pub struct InverseCdf<const K: usize> {
    a: f64,
    b: f64,
    /// Exactly uniform (constant or degenerate density): `sample` is `a + u·(b − a)`.
    uniform: bool,
    q: [f64; K],
}

impl<const K: usize> InverseCdf<K> {
    /// Tabulate `density` (non-negative; negative or non-finite values count as 0) on
    /// `[a, b]` (CORE-R58). Non-finite bounds are taken as 0 and reversed bounds are
    /// swapped. A density that is zero or non-finite everywhere, a constant one, and
    /// `K < 2` all give the exact uniform sampler.
    pub fn build(a: f64, b: f64, mut density: impl FnMut(f64) -> f64) -> Self {
        let clean = |v: f64| if v.is_finite() { v } else { 0.0 };
        let (a, b) = (clean(a), clean(b));
        let (a, b) = if b < a { (b, a) } else { (a, b) };
        let mut s = Self {
            a,
            b,
            uniform: true,
            q: [0.0; K],
        };
        // An infinite span (a = −MAX, b = MAX) has no finite bins: stay uniform.
        if K < 2 || a == b || !(b - a).is_finite() {
            return s;
        }
        let h = (b - a) / K as f64;
        // Bin masses first (at the bin midpoints), then their running sum in place.
        let mut cdf = [0.0f64; K];
        for (i, m) in cdf.iter_mut().enumerate() {
            let v = density(a + (i as f64 + 0.5) * h);
            *m = if v.is_finite() && v > 0.0 { v } else { 0.0 };
        }
        let max = cdf.iter().fold(0.0f64, |m, &v| m.max(v));
        if max == 0.0 || cdf.iter().all(|&v| v == cdf[0]) {
            return s;
        }
        // Scale by the max first so the running sum cannot overflow.
        let mut acc = 0.0;
        let mut last = 0;
        for (i, m) in cdf.iter_mut().enumerate() {
            if *m > 0.0 {
                last = i;
            }
            acc += *m / max;
            *m = acc;
        }
        for m in &mut cdf {
            *m /= acc;
        }
        cdf[K - 1] = 1.0;

        // Two-pointer inversion: quantile j lies in the first bin whose CDF exceeds
        // u_j (never a zero-mass bin), linearly within it.
        let mut i = 0;
        for (j, q) in s.q.iter_mut().enumerate() {
            let x = if j == K - 1 {
                a + (last + 1) as f64 * h
            } else {
                let u = j as f64 / (K - 1) as f64;
                while i < K - 1 && cdf[i] <= u {
                    i += 1;
                }
                let lo = if i == 0 { 0.0 } else { cdf[i - 1] };
                // cdf[i] > u ≥ lo, so the denominator is positive.
                let frac = ((u - lo) / (cdf[i] - lo)).clamp(0.0, 1.0);
                a + (i as f64 + frac) * h
            };
            *q = x.clamp(a, b);
        }
        s.uniform = false;
        s
    }

    /// The sample for a uniform variate `u ∈ [0, 1]` (clamped, NaN ⇒ 0): monotone
    /// non-decreasing in `u`, always in `[a, b]`, and exactly `a + u·(b − a)` for a
    /// uniform density (CORE-R58).
    #[inline]
    pub fn sample(&self, u: f64) -> f64 {
        let u = if u.is_nan() { 0.0 } else { u.clamp(0.0, 1.0) };
        let x = if self.uniform {
            let x = self.a + u * (self.b - self.a);
            // Only an overflowing span gets here non-finite; blend the ends instead.
            if x.is_finite() {
                x
            } else {
                self.a * (1.0 - u) + self.b * u
            }
        } else {
            let t = u * (K - 1) as f64;
            let j = (t as usize).min(K - 2);
            let f = t - j as f64;
            self.q[j] + (self.q[j + 1] - self.q[j]) * f
        };
        x.clamp(self.a, self.b)
    }
}
