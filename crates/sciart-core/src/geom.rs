//! Geometry helpers shared by generators (CORE-R16..R18).

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
