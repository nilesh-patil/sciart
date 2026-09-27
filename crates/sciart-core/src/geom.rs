//! Geometry helpers shared by generators (CORE-R16..R18, CORE-R45, CORE-R50, CORE-R60..R63).

use std::f64::consts::{PI, TAU};

/// Terminal cells are ~2x taller than wide; scale x by this for round shapes.
pub const ASPECT: f64 = 2.0;

/// Point at angle `theta` (radians) on a circle of radius `r` rows around `(cx, cy)`,
/// with x stretched by [`ASPECT`].
pub fn polar(cx: f64, cy: f64, r: f64, theta: f64) -> (i32, i32) {
    let x = cx + r * theta.cos() * ASPECT;
    let y = cy + r * theta.sin();
    (x.round() as i32, y.round() as i32)
}

/// `v` wrapped into `[0, max)`. `max` must be positive.
pub fn wrap(v: f64, max: f64) -> f64 {
    let w = v.rem_euclid(max);
    // rem_euclid can round up to exactly `max` for tiny negative v.
    if w >= max { 0.0 } else { w }
}

/// Angle `a` (radians) wrapped into `[−π, π)`, e.g. the signed turn from one heading to
/// another, `wrap_angle(to − from)` (CORE-R61). Non-finite input gives NaN.
#[inline]
pub fn wrap_angle(a: f64) -> f64 {
    wrap(a + PI, TAU) - PI
}

/// Plain 3-vectors as `[f64; 3]` (CORE-R60): the handful of operations the 3-D scenes
/// need, so each scene does not grow its own `dot`/`cross`/`norm`.
///
/// Free functions on arrays rather than a newtype, so scene data (`Camera` points,
/// orbital samples, model atoms) stays plain `[f64; 3]` and needs no conversions.
/// Every function spells out the same component-wise arithmetic the scenes used, so
/// swapping one in is bit-identical.
pub mod v3 {
    /// A 3-vector.
    pub type V3 = [f64; 3];

    /// `a + b`.
    #[inline]
    pub fn add(a: V3, b: V3) -> V3 {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    /// `a − b`.
    #[inline]
    pub fn sub(a: V3, b: V3) -> V3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    /// `a · k`.
    #[inline]
    pub fn scale(a: V3, k: f64) -> V3 {
        [a[0] * k, a[1] * k, a[2] * k]
    }

    /// `k·a + b` (BLAS `axpy`).
    #[inline]
    pub fn axpy(k: f64, a: V3, b: V3) -> V3 {
        [k * a[0] + b[0], k * a[1] + b[1], k * a[2] + b[2]]
    }

    /// The dot product `a · b`.
    #[inline]
    pub fn dot(a: V3, b: V3) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    /// The cross product `a × b` (right-handed).
    #[inline]
    pub fn cross(a: V3, b: V3) -> V3 {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// The Euclidean length `|a|`.
    #[inline]
    pub fn norm(a: V3) -> f64 {
        dot(a, a).sqrt()
    }

    /// `a / |a|`; the zero vector gives NaNs (callers normalise non-degenerate
    /// directions only).
    #[inline]
    pub fn unit(a: V3) -> V3 {
        let n = norm(a);
        a.map(|c| c / n)
    }
}

/// A minimal 3-D view for scenes that spin a model in front of the viewer (CORE-R45).
///
/// World axes: x right, y up, z toward the viewer. The model is turned by `yaw` about
/// y, then by `pitch` about x; the eye sits on the +z axis at `distance` (use
/// `f64::INFINITY`, the default, for an orthographic view).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Rotation about the world y axis, radians.
    pub yaw: f64,
    /// Rotation about the x axis after the yaw, radians.
    pub pitch: f64,
    /// Eye distance from the origin in world units; `INFINITY` ⇒ orthographic.
    pub distance: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            distance: f64::INFINITY,
        }
    }
}

impl Camera {
    /// Project world point `p` onto the screen: `(x, y)` in cell units around
    /// `centre` (x stretched by [`ASPECT`], y down) with `scale` rows per world unit,
    /// plus the rotated depth `z` (larger = nearer, for painter's sorting and shading).
    ///
    /// `None` for non-finite input or output, and for points at or behind the eye
    /// (`z ≥ distance − 1e-9`), so callers can `let Some(..) else { continue }`.
    pub fn project(&self, p: [f64; 3], centre: (f64, f64), scale: f64) -> Option<(f64, f64, f64)> {
        self.projector().project(p, centre, scale)
    }

    /// This view with its rotation's sines and cosines computed once (CORE-R50), for
    /// projecting many points per frame without four transcendental calls each.
    pub fn projector(&self) -> Projector {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Projector {
            sy,
            cy,
            sp,
            cp,
            distance: self.distance,
        }
    }
}

/// A [`Camera`] with its rotation cached, from [`Camera::projector`] (CORE-R50).
///
/// [`Camera::project`] is implemented through this type, so the two are bit-identical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projector {
    sy: f64,
    cy: f64,
    sp: f64,
    cp: f64,
    distance: f64,
}

impl Projector {
    /// `p` in camera space: turned by yaw about y, then by pitch about x. The result's
    /// `z` is the depth that [`project`](Self::project) reports (larger = nearer).
    #[inline]
    pub fn rotate(&self, p: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = p;
        // R_x(pitch) · R_y(yaw) · p; at yaw = π/2, +x lands on −z (away from the eye).
        let (x1, z1) = (x * self.cy + z * self.sy, z * self.cy - x * self.sy);
        let (y2, z2) = (y * self.cp - z1 * self.sp, y * self.sp + z1 * self.cp);
        [x1, y2, z2]
    }

    /// The perspective factor `d / (d − depth)` that [`project`](Self::project) scales
    /// a point at `depth` by: how much nearer things look bigger (CORE-R63). Exactly 1
    /// for an orthographic view (`distance = ∞`), so a scene can size sprites by it
    /// without keeping its own copy of the eye distance.
    #[inline]
    pub fn magnification(&self, depth: f64) -> f64 {
        if self.distance.is_finite() {
            self.distance / (self.distance - depth)
        } else {
            1.0
        }
    }

    /// Same contract as [`Camera::project`] (CORE-R45).
    #[inline]
    pub fn project(&self, p: [f64; 3], centre: (f64, f64), scale: f64) -> Option<(f64, f64, f64)> {
        let [x1, y2, z2] = self.rotate(p);
        // An infinite `distance` passes (orthographic); a NaN on either side fails.
        let in_front = z2 < self.distance - 1e-9;
        if !in_front {
            return None;
        }
        let k = if self.distance.is_finite() {
            scale * self.distance / (self.distance - z2)
        } else {
            scale
        };
        let out = (centre.0 + x1 * k * ASPECT, centre.1 - y2 * k, z2);
        (out.0.is_finite() && out.1.is_finite() && out.2.is_finite()).then_some(out)
    }
}

/// Bresenham line from `(x0, y0)` to `(x1, y1)`, both endpoints inclusive.
pub fn line(x0: i32, y0: i32, x1: i32, y1: i32) -> impl Iterator<Item = (i32, i32)> {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let (mut x, mut y, mut err, mut done) = (x0, y0, dx + dy, false);
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let p = (x, y);
        if x == x1 && y == y1 {
            done = true;
        } else {
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
        Some(p)
    })
}

/// Walk the Bresenham line [`line`] from `(x0, y0)` to `(x1, y1)` as a stroke: `f(k, x,
/// y, (dx, dy))` gets each cell's index `k` along the line and the step into it, so a
/// scene can pick a glyph per step (CORE-R62). The first cell takes its outgoing step,
/// and a lone cell `(0, 1)`, a step down a column. [`draw::step_glyph`] is the default
/// glyph policy.
///
/// [`draw::step_glyph`]: crate::draw::step_glyph
pub fn stroked_line(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    mut f: impl FnMut(i32, i32, i32, (i32, i32)),
) {
    let mut cells = line(x0, y0, x1, y1).peekable();
    let mut prev: Option<(i32, i32)> = None;
    let mut k = 0;
    while let Some((x, y)) = cells.next() {
        let step = match (prev, cells.peek()) {
            (Some((px, py)), _) => (x - px, y - py),
            (None, Some(&(nx, ny))) => (nx - x, ny - y),
            (None, None) => (0, 1),
        };
        f(k, x, y, step);
        prev = Some((x, y));
        k += 1;
    }
}
