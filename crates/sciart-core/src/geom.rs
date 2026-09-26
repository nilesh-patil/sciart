//! Geometry helpers shared by generators (CORE-R16..R18, CORE-R45, CORE-R50).

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
