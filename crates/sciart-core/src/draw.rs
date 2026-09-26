//! Shared drawing helpers (SPEC-001 CORE-R22..R27).
//!
//! Centers and radii are in rows; x is stretched by [`ASPECT`] so circles look round.

use std::f64::consts::TAU;

use crate::geom::ASPECT;
use crate::{Canvas, Glyph, Ink, Size};

/// Upper bound on outline samples, so absurd radii stay cheap.
const MAX_ORBIT_SAMPLES: usize = 20_000;

/// Ellipse outline of radius `r` rows (CORE-R22).
pub fn orbit(canvas: &mut Canvas, cx: f64, cy: f64, r: f64, glyph: Glyph, ink: Ink) {
    if !(cx.is_finite() && cy.is_finite() && r.is_finite()) || r <= 0.0 {
        return;
    }
    // ≤ 0.5 cells between samples keeps the rounded outline 8-connected.
    let samples = ((TAU * r * ASPECT * 2.0).ceil() as usize).clamp(8, MAX_ORBIT_SAMPLES);
    let mut last = None;
    for i in 0..samples {
        let theta = TAU * i as f64 / samples as f64;
        let x = (cx + r * theta.cos() * ASPECT).round() as i32;
        let y = (cy + r * theta.sin()).round() as i32;
        if last != Some((x, y)) {
            canvas.put(x, y, glyph, ink);
            last = Some((x, y));
        }
    }
}

/// Filled disc with a per-cell shader (CORE-R23).
pub fn disc(
    canvas: &mut Canvas,
    cx: f64,
    cy: f64,
    r: f64,
    mut shader: impl FnMut(f64, f64, f64) -> Option<(Glyph, Ink)>,
) {
    if !(cx.is_finite() && cy.is_finite() && r.is_finite()) || r <= 0.0 {
        return;
    }
    let (w, h) = (
        f64::from(canvas.size().width()),
        f64::from(canvas.size().height()),
    );
    if r < 0.5 {
        let (x, y) = (cx.round(), cy.round());
        if (0.0..w).contains(&x)
            && (0.0..h).contains(&y)
            && let Some((g, ink)) = shader(0.0, 0.0, 0.0)
        {
            canvas.put(x as i32, y as i32, g, ink);
        }
        return;
    }
    let rx = r * ASPECT;
    let (x0, x1) = ((cx - rx).floor().max(0.0), (cx + rx).ceil().min(w - 1.0));
    let (y0, y1) = ((cy - r).floor().max(0.0), (cy + r).ceil().min(h - 1.0));
    if x0 > x1 || y0 > y1 {
        return;
    }
    for y in y0 as i32..=y1 as i32 {
        let ny = (f64::from(y) - cy) / r;
        for x in x0 as i32..=x1 as i32 {
            let nx = (f64::from(x) - cx) / rx;
            let d = (nx * nx + ny * ny).sqrt();
            if d <= 1.0
                && let Some((g, ink)) = shader(d, nx, ny)
            {
                canvas.put(x, y, g, ink);
            }
        }
    }
}

/// Shade every cell once (CORE-R24).
pub fn field(canvas: &mut Canvas, mut shader: impl FnMut(i32, i32) -> Option<(Glyph, Ink)>) {
    let (w, h) = (
        i32::from(canvas.size().width()),
        i32::from(canvas.size().height()),
    );
    for y in 0..h {
        for x in 0..w {
            if let Some((g, ink)) = shader(x, y) {
                canvas.put(x, y, g, ink);
            }
        }
    }
}

/// Pick a glyph for `v ∈ [0, 1]`, clamped; NaN ⇒ first (CORE-R25).
pub fn ramp(v: f64, glyphs: &[Glyph]) -> Glyph {
    assert!(!glyphs.is_empty(), "ramp needs at least one glyph");
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    glyphs[(v * (glyphs.len() - 1) as f64).round() as usize]
}

/// Horizontally centered text on row `y`, clipped (CORE-R26).
pub fn text_centered(canvas: &mut Canvas, y: i32, s: &str, ink: Ink) {
    let len = s.chars().count() as i32;
    let x = (i32::from(canvas.size().width()) - len) / 2;
    canvas.text(x, y, s, ink);
}

/// Largest circle radius (rows) fitting `frac` of the canvas (CORE-R27).
pub fn fit_radius(size: Size, frac: f64) -> f64 {
    let limit = (f64::from(size.width()) / (2.0 * ASPECT)).min(f64::from(size.height()) / 2.0);
    (frac.max(0.0) * limit).max(0.0)
}
