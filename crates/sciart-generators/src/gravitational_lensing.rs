//! Point-mass gravitational lensing (SPEC-002 `gravitational_lensing`).
//!
//! Angles are measured in rows on the sky, centred on the lens; screen x is stretched by
//! `ASPECT`. A point source at `β` has two images `θ±` on the line through the lens; an
//! extended source is drawn by inverse ray-shooting: every image-plane sample `θ` is mapped
//! back to the source plane with the lens equation and takes the source's surface
//! brightness there (lensing conserves surface brightness).

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;
use sciart_core::{draw, geom};

pub static META: Meta = Meta {
    name: "gravitational_lensing",
    category: Category::Astronomy,
    description: "Stars and a galaxy lensed into double images and an Einstein ring",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(GravitationalLensing::default())
}

/// Einstein radius as a fraction of the fitted radius, `θE = R(0.35)`.
const EINSTEIN_FRAC: f64 = 0.35;
const STARS: usize = 160;
/// Background drift speed in rows per second.
const DRIFT: f64 = 0.6;
/// Galaxy radius in rows and the time between its passes.
const GALAXY_R: f64 = 1.5;
const PASS: f64 = 12.0;
/// The galaxy crosses `x ∈ [−SWEEP θE, SWEEP θE]` in the source plane.
const SWEEP: f64 = 2.2;
/// Ray-shooting stops at this many Einstein radii from the lens.
const SHOOT: f64 = 2.5;
/// Largest |impact parameter| of a pass, in rows; below `GALAXY_R` the pass makes a full ring.
const MAX_IMPACT: f64 = 2.2;
/// Display floor: images demagnified below this are too faint to draw. Without it the
/// minor images of all 160 stars pile up into a grey clump inside the Einstein radius
/// (`|μ−| = (μ_tot − 1)/2 ≈ u⁻⁴` for a star at `u = |β|/θE`). At 0.05 only stars within
/// `u ≈ 1.6` show a minor image, which then lies in the annulus `0.48 θE < |θ−| < θE`.
const MIN_MU: f64 = 0.05;

const IMAGE: [Glyph; 4] = [
    Glyph::new('·', '.'),
    Glyph::new('•', 'o'),
    Glyph::ascii('*'),
    Glyph::new('✦', '*'),
];
const ARC: [Glyph; 5] = [
    Glyph::new('·', '.'),
    Glyph::ascii(':'),
    Glyph::new('░', '+'),
    Glyph::new('▒', '*'),
    Glyph::new('▓', '#'),
];
const LENS: Glyph = Glyph::new('◉', '@');

type V2 = (f64, f64);

/// Lens equation `β = θ − θE² θ/|θ|²` (LENS-A1). The lens position itself maps to
/// infinity; it is reported as `(∞, ∞)` so distance tests simply fail instead of giving NaN.
pub(crate) fn source_plane(theta: V2, theta_e: f64) -> V2 {
    let r2 = theta.0 * theta.0 + theta.1 * theta.1;
    if r2 == 0.0 {
        return (f64::INFINITY, f64::INFINITY);
    }
    let k = theta_e * theta_e / r2;
    (theta.0 * (1.0 - k), theta.1 * (1.0 - k))
}

/// The two images `[θ+, θ−]` of a point source at `β` (LENS-A1..A3).
///
/// `θ± = β̂ (|β| ± √(|β|² + 4θE²))/2`. The minor image uses `θ+ θ− = −θE²` (product of the
/// roots) instead of the subtraction, which cancels catastrophically for `|β| ≫ θE`. At
/// `β = 0` the images degenerate into the Einstein ring; we pick `(±θE, 0)`.
pub(crate) fn images(beta: V2, theta_e: f64) -> [V2; 2] {
    let b = beta.0.hypot(beta.1);
    if b < 1e-12 {
        return [(theta_e, 0.0), (-theta_e, 0.0)];
    }
    let (ux, uy) = (beta.0 / b, beta.1 / b);
    let plus = 0.5 * (b + (b * b + 4.0 * theta_e * theta_e).sqrt());
    let minus = -theta_e * theta_e / plus;
    [(ux * plus, uy * plus), (ux * minus, uy * minus)]
}

/// Signed magnification of an image at `θ`: `μ = 1 / (1 − (θE/|θ|)⁴)`; negative for the
/// parity-flipped minor image, infinite on the Einstein ring, → 0 towards the lens.
pub(crate) fn magnification(theta: V2, theta_e: f64) -> f64 {
    let r = theta.0.hypot(theta.1);
    if r == 0.0 {
        return 0.0;
    }
    let q = (theta_e / r).powi(4);
    1.0 / (1.0 - q)
}

/// Glyph and brightness for a point image of magnification `mu` (clamped, NaN-safe).
/// Unlensed stars (`|μ| ≈ 1`) are small dots; magnified ones near the ring flare up.
fn image_look(mu: f64) -> (Glyph, u8) {
    let m = if mu.is_nan() { 0.0 } else { mu.abs().min(1e6) };
    let level = (255.0 * (m / 3.0).sqrt()).clamp(35.0, 255.0) as u8;
    let glyph = match m {
        m if m < 1.1 => IMAGE[0],
        m if m < 1.6 => IMAGE[1],
        m if m < 3.0 => IMAGE[2],
        _ => IMAGE[3],
    };
    (glyph, level)
}

/// Galaxy surface brightness at squared normalized radius `r2` (1 at the centre, 0 for
/// `r2 ≥ 1`).
fn galaxy_profile(r2: f64) -> f64 {
    if r2 >= 1.0 {
        0.0
    } else {
        (1.0 - r2) * (1.0 - r2)
    }
}

#[derive(Debug, Default)]
pub struct GravitationalLensing {
    cx: f64,
    cy: f64,
    theta_e: f64,
    /// Half-extents of the visible sky in rows.
    hx: f64,
    hy: f64,
    /// Normalized source positions `u, v ∈ [0, 1)`.
    stars: Vec<V2>,
    drift_dir: f64,
    /// Time into the current galaxy pass, its impact parameter and direction.
    pass_t: f64,
    impact: f64,
    pass_dir: f64,
}

impl GravitationalLensing {
    fn star_beta(&self, (u, v): V2) -> V2 {
        ((u - 0.5) * 2.0 * self.hx, (v - 0.5) * 2.0 * self.hy)
    }

    /// Galaxy centre in the source plane and its brightness envelope for this pass.
    fn galaxy(&self) -> (V2, f64) {
        let s = (self.pass_t / PASS).clamp(0.0, 1.0);
        let along = self.theta_e * SWEEP * (2.0 * s - 1.0);
        let (c, sn) = (self.pass_dir.cos(), self.pass_dir.sin());
        let beta = (along * c - self.impact * sn, along * sn + self.impact * c);
        // Fading in and out hides the ray-shooting boundary at the ends of a pass.
        (beta, (PI * s).sin().powf(0.6))
    }

    fn new_pass(&mut self, ctx: &mut Ctx) {
        self.impact = ctx.rng.random_range(-MAX_IMPACT..MAX_IMPACT);
        self.pass_dir = ctx.rng.random_range(-0.5..0.5);
    }

    fn render_galaxy(&self, canvas: &mut Canvas) {
        let (g, envelope) = self.galaxy();
        let te = self.theta_e;
        if envelope <= 0.0 || te <= 0.0 {
            return;
        }
        // Every image of the disc lies within the major image of its far edge, so only
        // that bounding box (capped at SHOOT θE) is ray-shot (GEN-R8).
        let far = g.0.hypot(g.1) + GALAXY_R;
        let reach = (0.5 * (far + (far * far + 4.0 * te * te).sqrt()) + 1.0).min(SHOOT * te);
        let (w, h) = (
            i32::from(canvas.size().width()),
            i32::from(canvas.size().height()),
        );
        let x0 = ((self.cx - reach * ASPECT).floor() as i32).max(0);
        let x1 = ((self.cx + reach * ASPECT).ceil() as i32).min(w - 1);
        let y0 = ((self.cy - reach).floor() as i32).max(0);
        let y1 = ((self.cy + reach).ceil() as i32).min(h - 1);
        let inv_r2 = 1.0 / (GALAXY_R * GALAXY_R);
        let reach2 = reach * reach;
        for y in y0..=y1 {
            for x in x0..=x1 {
                // 2×2 supersampling smooths the thin ring.
                let mut sum = 0.0;
                for (ox, oy) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                    let theta = (
                        (f64::from(x) + ox - self.cx) / ASPECT,
                        f64::from(y) + oy - self.cy,
                    );
                    if theta.0 * theta.0 + theta.1 * theta.1 > reach2 {
                        continue;
                    }
                    let b = source_plane(theta, te);
                    let r2 = ((b.0 - g.0).powi(2) + (b.1 - g.1).powi(2)) * inv_r2;
                    sum += galaxy_profile(r2);
                }
                let v = 0.25 * sum * envelope;
                if v > 0.04 {
                    let v = v.sqrt();
                    canvas.put(
                        x,
                        y,
                        draw::ramp(v, &ARC),
                        Ink::new(Role::Accent, (90.0 + 165.0 * v) as u8),
                    );
                }
            }
        }
    }
}

impl Generator for GravitationalLensing {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cx = f64::from(size.width() / 2);
        self.cy = f64::from(size.height() / 2);
        self.theta_e = draw::fit_radius(size, EINSTEIN_FRAC);
        self.hx = f64::from(size.width()) / (2.0 * ASPECT);
        self.hy = f64::from(size.height()) / 2.0;
        if self.stars.is_empty() {
            // Normalized positions survive resizes; only the sky's scale changes.
            self.stars = (0..STARS)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..1.0),
                    )
                })
                .collect();
            self.drift_dir = ctx.rng.random_range(0.0..TAU);
            self.pass_t = ctx.rng.random_range(0.0..PASS);
            self.new_pass(ctx);
        }
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        // Drift in rows/s, converted to the normalized sky (hx, hy ≥ 0.25 for any size).
        let (du, dv) = (
            DRIFT * self.drift_dir.cos() * dt / (2.0 * self.hx.max(0.25)),
            DRIFT * self.drift_dir.sin() * dt / (2.0 * self.hy.max(0.25)),
        );
        for (u, v) in &mut self.stars {
            *u = geom::wrap(*u + du, 1.0);
            *v = geom::wrap(*v + dv, 1.0);
        }
        self.pass_t += dt;
        if self.pass_t >= PASS {
            self.pass_t = geom::wrap(self.pass_t, PASS);
            self.new_pass(ctx);
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let te = self.theta_e;
        let to_cell = |(tx, ty): V2| {
            (
                (self.cx + tx * ASPECT).round() as i32,
                (self.cy + ty).round() as i32,
            )
        };
        // Minor images first so a bright major image wins a shared cell.
        for which in [1, 0] {
            for &star in &self.stars {
                let theta = images(self.star_beta(star), te)[which];
                let mu = magnification(theta, te);
                if mu.abs() < MIN_MU {
                    continue;
                }
                let (glyph, level) = image_look(mu);
                let (x, y) = to_cell(theta);
                canvas.put(x, y, glyph, Ink::new(Role::Star, level));
            }
        }
        self.render_galaxy(canvas);
        let (x, y) = to_cell((0.0, 0.0));
        canvas.put(x, y, LENS, Ink::new(Role::Structure, 200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TE: f64 = 8.75;

    fn dist(a: V2, b: V2) -> f64 {
        (a.0 - b.0).hypot(a.1 - b.1)
    }

    fn betas() -> impl Iterator<Item = V2> {
        (-12..=12).flat_map(|i| (-12..=12).map(move |j| (f64::from(i) * 1.7, f64::from(j) * 0.9)))
    }

    #[test]
    fn lens_a1_both_images_map_back_to_the_source() {
        for te in [0.1, 1.0, TE, 40.0] {
            for beta in betas() {
                let scale = te.max(beta.0.hypot(beta.1));
                for theta in images(beta, te) {
                    let back = source_plane(theta, te);
                    // A few ulps of the largest length involved.
                    assert!(
                        dist(back, beta) < 1e-11 * scale.max(1.0),
                        "te={te} β={beta:?} θ={theta:?}"
                    );
                }
                // Images lie on the line through the lens, on opposite sides of it.
                let [p, m] = images(beta, te);
                assert!((p.0 * m.1 - p.1 * m.0).abs() < 1e-9 * scale * scale);
                assert!(p.0 * m.0 + p.1 * m.1 <= 0.0);
            }
        }
    }

    #[test]
    fn lens_a2_images_approach_the_einstein_radius_as_beta_vanishes() {
        for b in [1e-2, 1e-4, 1e-8] {
            for theta in images((b, 0.0), TE) {
                // |θ±| = √(θE² + b²/4) ± b/2 = θE ± b/2 + b²/(8θE) + O(b⁴).
                let err = (theta.0.hypot(theta.1) - TE).abs();
                assert!(
                    err <= 0.5 * b + b * b / (4.0 * TE) + 1e-12,
                    "b={b} θ={theta:?}"
                );
            }
        }
        let [p, m] = images((0.0, 0.0), TE);
        assert_eq!((p, m), ((TE, 0.0), (-TE, 0.0)));
    }

    #[test]
    fn lens_a3_far_sources_are_barely_deflected() {
        for b in [100.0 * TE, 1e4 * TE] {
            let beta = (0.6 * b, -0.8 * b);
            let [p, m] = images(beta, TE);
            // Primary: θ+ = β(1 + θE²/b² + …); secondary: |θ−| = θE²/|θ+| → 0.
            assert!(dist(p, beta) / b < 1.01 * (TE / b).powi(2), "b={b}");
            assert!(m.0.hypot(m.1) < 1.01 * TE * TE / b, "b={b}");
            assert!(m.0.hypot(m.1) > 0.0, "minor image lost to cancellation");
            assert!((magnification(p, TE) - 1.0).abs() < 1e-3);
            assert!(magnification(m, TE).abs() < 1e-3);
        }
    }

    #[test]
    fn lens_total_magnification_matches_the_point_lens_formula() {
        // Paczyński (1986): μ_tot = (u² + 2) / (u √(u² + 4)), u = |β|/θE.
        for u in [0.05, 0.3, 1.0, 2.0, 5.0] {
            let [p, m] = images((u * TE, 0.0), TE);
            let total = magnification(p, TE).abs() + magnification(m, TE).abs();
            let expected = (u * u + 2.0) / (u * (u * u + 4.0).sqrt());
            assert!((total - expected).abs() < 1e-9 * expected, "u={u}");
            assert!(magnification(p, TE) > 0.0 && magnification(m, TE) < 0.0);
        }
    }

    #[test]
    fn lens_nan_free_at_the_lens_and_for_degenerate_magnification() {
        let b = source_plane((0.0, 0.0), TE);
        assert!(!b.0.is_nan() && !b.1.is_nan());
        assert_eq!(magnification((0.0, 0.0), TE), 0.0);
        assert_eq!(image_look(f64::NAN).1, 35);
        assert_eq!(image_look(f64::INFINITY).1, 255);
    }

    fn scene(size: Size) -> GravitationalLensing {
        let mut g = GravitationalLensing::default();
        g.resize(size, &mut Ctx::new(9, size));
        g
    }

    #[test]
    fn lens_aligned_galaxy_forms_an_einstein_ring() {
        let size = Size::new(200, 50);
        let mut g = scene(size);
        g.pass_t = PASS / 2.0;
        g.impact = 0.0;
        g.pass_dir = 0.0;
        g.stars.clear();
        let mut c = Canvas::new(size, GlyphMode::Ascii);
        g.render(&mut c);
        // Every direction around the lens hits the ring at |θ| ≈ θE…
        for k in 0..16 {
            let a = TAU * f64::from(k) / 16.0;
            let (x, y) = geom::polar(g.cx, g.cy, g.theta_e, a);
            assert_eq!(
                c.get(x, y).unwrap().ink.role,
                Role::Accent,
                "gap at angle {a}"
            );
        }
        // …and nothing is drawn at half the Einstein radius (inside the ring).
        let (x, y) = geom::polar(g.cx, g.cy, 0.5 * g.theta_e, 0.3);
        assert_ne!(c.get(x, y).unwrap().ink.role, Role::Accent);
    }
}
