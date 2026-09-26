//! Looping scripted narratives and easing curves (SPEC-001 CORE-R38, CORE-R39).
//!
//! A scene such as a moon landing is a fixed script — descent, hover, touchdown, … —
//! replayed forever. [`Timeline`] turns the scene's accumulated sim time into "which
//! phase, and how far through it", and [`ease`] shapes that progress into motion.

/// One non-empty phase, with its start offset precomputed so queries never re-sum.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Phase {
    /// Offset of this phase from the start of the cycle, in seconds.
    start: f64,
    duration: f64,
    label: &'static str,
    /// Position in the list given to [`Timeline::new`] (zero-length phases included).
    index: usize,
}

/// Where a point in time falls inside a [`Timeline`] (CORE-R38).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beat {
    /// Index of the phase in the list passed to [`Timeline::new`] — skipped
    /// zero-length phases keep their slot, so scenes can match on their own constants.
    pub index: usize,
    pub label: &'static str,
    /// Seconds since the phase started, in `[0, duration)`.
    pub local: f64,
    /// `local / duration`, in `[0, 1)`.
    pub progress: f64,
}

/// A looping sequence of labelled phases (CORE-R38).
///
/// Storage is one `Box<[Phase]>` built in [`Timeline::new`] — the only allocation.
/// Taking `&'static` slices instead would forbid durations computed at runtime (from
/// the canvas size or `ctx.rng`), and precomputing start offsets means a query is a
/// `rem_euclid` plus an O(log n) binary search, with no summing and no allocation.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// Only phases with a finite, positive duration, in order.
    phases: Box<[Phase]>,
    total: f64,
    /// Label reported by an empty timeline: the first phase's, if any.
    fallback_label: &'static str,
}

impl Timeline {
    /// Build a timeline from `(label, seconds)` pairs (CORE-R38).
    ///
    /// Phases whose duration is not a finite positive number (zero, negative, NaN, ∞)
    /// are zero-length and never reported by [`Timeline::at`]. An infinite phase would
    /// make the loop never wrap, which a scene can express by simply not using a
    /// timeline, so it is treated as invalid rather than special-cased.
    pub fn new(phases: &[(&'static str, f64)]) -> Self {
        let mut total = 0.0;
        let kept: Box<[Phase]> = phases
            .iter()
            .enumerate()
            .filter(|(_, (_, d))| d.is_finite() && *d > 0.0)
            .map(|(index, &(label, duration))| {
                let start = total;
                total += duration;
                Phase {
                    start,
                    duration,
                    label,
                    index,
                }
            })
            .collect();
        // Sums of finite values can still overflow; such a timeline is unusable.
        let (phases_kept, total) = if total.is_finite() {
            (kept, total)
        } else {
            (Box::default(), 0.0)
        };
        Self {
            phases: phases_kept,
            total,
            fallback_label: phases.first().map_or("", |p| p.0),
        }
    }

    /// Length of one cycle in seconds; 0 for an empty or all-zero timeline (CORE-R38).
    #[inline]
    pub fn total(&self) -> f64 {
        self.total
    }

    /// `true` when no phase has positive length, so every query reports phase 0.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.phases.is_empty()
    }

    /// Split `t` into (completed cycles, offset in `[0, total)`), or `None` when the
    /// timeline is empty. Non-finite `t` is treated as 0.
    fn wrap(&self, t: f64) -> Option<(f64, f64)> {
        if self.phases.is_empty() {
            return None;
        }
        let t = if t.is_finite() { t } else { 0.0 };
        let mut cycles = t.div_euclid(self.total);
        let mut u = t.rem_euclid(self.total);
        // For tiny negative `t`, `t + total` rounds to exactly `total`: that instant is
        // (to within one ulp) the start of the next cycle.
        if u >= self.total {
            u = 0.0;
            cycles += 1.0;
        }
        Some((cycles, u))
    }

    /// The phase active at sim time `t`, looping every [`Timeline::total`] seconds
    /// (CORE-R38).
    ///
    /// Any finite `t`, including negative, maps into the cycle; non-finite `t` behaves
    /// like `t = 0`. An empty timeline yields index 0 (labelled with the first phase
    /// given, or `""`) with `local = progress = 0`.
    pub fn at(&self, t: f64) -> Beat {
        let Some((_, u)) = self.wrap(t) else {
            return Beat {
                index: 0,
                label: self.fallback_label,
                local: 0.0,
                progress: 0.0,
            };
        };
        // Last phase starting at or before `u`; phase 0 starts at 0 so this is >= 1.
        let i = self
            .phases
            .partition_point(|p| p.start <= u)
            .saturating_sub(1);
        let p = &self.phases[i];
        // `u < start + duration` in exact terms, but the subtraction can round up to
        // `duration`; keep both outputs strictly inside their half-open ranges.
        let local = (u - p.start).clamp(0.0, p.duration.next_down());
        let progress = (local / p.duration).min(1.0f64.next_down());
        Beat {
            index: p.index,
            label: p.label,
            local,
            progress,
        }
    }

    /// Number of completed cycles at time `t`: `floor(t / total)`, so negative for
    /// `t < 0`; saturates at the `i64` range and is 0 when empty or `t` is not finite
    /// (CORE-R38).
    pub fn cycle(&self, t: f64) -> i64 {
        // Float-to-int `as` saturates, which is exactly the behaviour documented above.
        self.wrap(t).map_or(0, |(cycles, _)| cycles as i64)
    }
}

/// Easing curves mapping `[0, 1] → [0, 1]` monotonically with `f(0) = 0`, `f(1) = 1`
/// (CORE-R39).
///
/// Inputs outside `[0, 1]` are clamped and NaN maps to 0, so a curve can be fed a raw
/// [`Beat::progress`]-style value, or garbage, without producing garbage.
pub mod ease {
    /// Clamp to `[0, 1]`, NaN ⇒ 0 (`f64::clamp` would propagate NaN).
    #[inline]
    fn unit(x: f64) -> f64 {
        if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
    }

    /// Hermite smoothstep `3x² − 2x³`: zero slope at both ends (CORE-R39).
    #[inline]
    pub fn smoothstep(x: f64) -> f64 {
        let x = unit(x);
        x * x * (3.0 - 2.0 * x)
    }

    /// Cubic ease-in-out: `4x³` then mirrored; passes through (½, ½) (CORE-R39).
    #[inline]
    pub fn in_out_cubic(x: f64) -> f64 {
        let x = unit(x);
        if x < 0.5 {
            4.0 * x * x * x
        } else {
            let r = 2.0 - 2.0 * x;
            1.0 - 0.5 * r * r * r
        }
    }

    /// Quadratic ease-out `1 − (1 − x)²`: fast start, gentle landing (CORE-R39).
    #[inline]
    pub fn out_quad(x: f64) -> f64 {
        let r = 1.0 - unit(x);
        1.0 - r * r
    }

    /// Linear interpolation from `a` (at `t = 0`) to `b` (at `t = 1`) (CORE-R39).
    ///
    /// `t` is **not** clamped, so it extrapolates; compose with a curve above to stay
    /// in range. Interpolating from the nearer endpoint makes the result exact at
    /// `t = 0` and `t = 1` and exactly `a` when `a == b`; plain `a + (b − a)·t` can
    /// miss `b` at `t = 1` by an ulp.
    #[inline]
    pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
        let d = b - a;
        if t < 0.5 {
            a + d * t
        } else {
            b - d * (1.0 - t)
        }
    }
}
