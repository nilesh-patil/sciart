//! Shared drawing helpers (SPEC-001 CORE-R22..R27, CORE-R49, CORE-R62, CORE-R63,
//! CORE-R67, CORE-R76).
//!
//! Centers and radii are in rows; x is stretched by [`ASPECT`] so circles look round.

use std::f64::consts::TAU;

use crate::geom::ASPECT;
use crate::{Canvas, Glyph, GlyphMode, Ink, Size};

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
        for x in x0 as i32..=x1 as i32 {
            let (d, nx, ny) = disc_offset(cx, cy, r, x, y);
            if d <= 1.0
                && let Some((g, ink)) = shader(d, nx, ny)
            {
                canvas.put(x, y, g, ink);
            }
        }
    }
}

/// Normalised offset `(d, nx, ny)` of cell `(x, y)` from a disc's centre, `nx` in units
/// of `r·ASPECT` and `ny` in units of `r`: the one inside test shared by [`disc`] and
/// [`disc_covers`], so the two cannot drift apart.
#[inline]
fn disc_offset(cx: f64, cy: f64, r: f64, x: i32, y: i32) -> (f64, f64, f64) {
    let nx = (f64::from(x) - cx) / (r * ASPECT);
    let ny = (f64::from(y) - cy) / r;
    ((nx * nx + ny * ny).sqrt(), nx, ny)
}

/// Whether [`disc`]`(cx, cy, r, ..)` visits cell `(x, y)` on a canvas large enough to
/// hold it (CORE-R63): the cells with `d ≤ 1`, or only the rounded centre cell when
/// `r < 0.5`. False wherever `disc` draws nothing (non-finite input, `r ≤ 0`). For scenes
/// that keep other strokes off a ball without painting the ball first.
pub fn disc_covers(cx: f64, cy: f64, r: f64, x: i32, y: i32) -> bool {
    if !(cx.is_finite() && cy.is_finite() && r.is_finite()) || r <= 0.0 {
        return false;
    }
    if r < 0.5 {
        return f64::from(x) == cx.round() && f64::from(y) == cy.round();
    }
    disc_offset(cx, cy, r, x, y).0 <= 1.0
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

/// Left-aligned text on row `y` from column `x`: `unicode` on a Unicode canvas, `ascii`
/// on an ASCII one, clipped at both edges (CORE-R49, GEN-R10). Returns the column after
/// the last char written (saturating), so labels can be chained.
///
/// For readouts whose Unicode form uses symbols (`ψ²`, `λ₁`, `→`) that a plain
/// `Canvas::text` would pass through unchanged in ASCII mode.
pub fn text_mode(canvas: &mut Canvas, x: i32, y: i32, unicode: &str, ascii: &str, ink: Ink) -> i32 {
    let s = match canvas.mode() {
        GlyphMode::Unicode => unicode,
        GlyphMode::Ascii => ascii,
    };
    canvas.text(x, y, s, ink);
    let n = i32::try_from(s.chars().count()).unwrap_or(i32::MAX);
    x.saturating_add(n)
}

/// A label on row `y`: a blank gutter cell at `x`, `s` from `x + 1`, and a blank cell
/// after its last char, so the text never looks glued to the art around it (CORE-R67).
/// Returns the column after the trailing blank. Chars are counted, not bytes.
pub fn label(canvas: &mut Canvas, x: i32, y: i32, s: &str, ink: Ink) -> i32 {
    let gap = Glyph::ascii(' ');
    let start = x.saturating_add(1);
    let end = start.saturating_add(i32::try_from(s.chars().count()).unwrap_or(i32::MAX));
    canvas.put(x, y, gap, Ink::BLANK);
    canvas.text(start, y, s, ink);
    canvas.put(end, y, gap, Ink::BLANK);
    end.saturating_add(1)
}

/// A right-aligned [`label`] on row `y` (CORE-R76): a blank cell in the last column, `s`
/// ending in the column before it, and a blank cell just before its first char, clipped
/// at column 0. Returns the column of that leading blank (negative when clipped), so a
/// readout can be shortened or dropped before it reaches the title span (GEN-R13).
pub fn label_right(canvas: &mut Canvas, y: i32, s: &str, ink: Ink) -> i32 {
    let gap = Glyph::ascii(' ');
    let last = i32::from(canvas.size().width()) - 1;
    let start = last.saturating_sub(i32::try_from(s.chars().count()).unwrap_or(i32::MAX));
    let lead = start.saturating_sub(1);
    canvas.put(last, y, gap, Ink::BLANK);
    canvas.text(start, y, s, ink);
    canvas.put(lead, y, gap, Ink::BLANK);
    lead
}

/// The glyph for one step `(dx, dy)` of a [`geom::stroked_line`] (CORE-R62): `─` along
/// a row, `│` down a column, `╲`/`╱` where both change. Picked per step, a shallow line
/// reads `──╲──` rather than two dash runs with a gap, and a steep one `│╲│`.
///
/// [`geom::stroked_line`]: crate::geom::stroked_line
pub fn step_glyph(dx: i32, dy: i32) -> Glyph {
    match (dx != 0, dy != 0) {
        (_, false) => Glyph::new('─', '-'),
        (false, true) => Glyph::new('│', '|'),
        _ if (dx > 0) == (dy > 0) => Glyph::new('╲', '\\'),
        _ => Glyph::new('╱', '/'),
    }
}

/// Largest circle radius (rows) fitting `frac` of the canvas (CORE-R27).
pub fn fit_radius(size: Size, frac: f64) -> f64 {
    let limit = (f64::from(size.width()) / (2.0 * ASPECT)).min(f64::from(size.height()) / 2.0);
    (frac.max(0.0) * limit).max(0.0)
}
