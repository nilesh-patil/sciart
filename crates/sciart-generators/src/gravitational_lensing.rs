//! Point-mass gravitational lensing (SPEC-002 `gravitational_lensing`).
//!
//! Angles are measured in rows on the sky, centred on the lens; screen x is stretched by
//! `ASPECT`. A point source at `β` has two images `θ±` on the line through the lens.
//!
//! M16 (thread style, GEN-R25): everything is a braille thread. The background galaxy is
//! a small spiral of threads in the source plane; each thread is mapped through both
//! image branches, a source segment being split until its image is at most `CHORD` dots
//! long, so the spiral smears into arcs and, aligned, into a thin Einstein ring. Stars
//! are one dot per image with a trail that is the image of their recent drift.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;
use sciart_core::{draw, geom};

pub static META: Meta = Meta {
    name: "gravitational_lensing",
    category: Category::DeepCosmos,
    description: "Stars and a galaxy lensed into double images and an Einstein ring",
    title: "Gravity lens · mass bends light",
    captions: &[],
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
const GALAXY_R: f64 = 3.6;
const PASS: f64 = 12.0;
/// The galaxy crosses `x ∈ [−SWEEP θE, SWEEP θE]` in the source plane.
const SWEEP: f64 = 2.2;
/// Largest |impact parameter| of a pass, in rows: within the bulge ring's half-width
/// (0.4 rows) the pass makes a full ring, otherwise arcs.
const MAX_IMPACT: f64 = 0.9;
/// Display floor: images demagnified below this are too faint to draw. Without it the
/// minor images of all 160 stars pile up inside the Einstein radius
/// (`|μ−| = (μ_tot − 1)/2 ≈ u⁻⁴` for a star at `u = |β|/θE`).
const MIN_MU: f64 = 0.05;
/// Galaxy threads, in units of its radius: the bulge ring's radius; arms
/// `r = BULGE·e^{ARM_K φ}` out to 1; the disc's spin (rad/s) and its inclination
/// (`cos i`, the projected flattening).
const BULGE: f64 = 0.12;
const ARM_K: f64 = 0.3;
const SPIN: f64 = 0.35;
const COS_I: f64 = 0.75;
const RING_PTS: usize = 28;
const ARM_PTS: usize = 44;
/// Below this pass envelope the galaxy is not drawn.
const MIN_ENVELOPE: f64 = 0.12;
/// Star trails: the last `TRAIL` s of drift, in `TRAIL_LEVELS.len()` steps, oldest first.
const TRAIL: f64 = 2.5;
const TRAIL_LEVELS: [u8; 2] = [70, 120];
/// Trails are drawn where lensing bends them: full length within `TRAIL_NEAR θE` of the
/// lens (source plane), shrinking to nothing at `TRAIL_FAR θE`, so the far field stays
/// single dots instead of a uniform rain of streaks.
const TRAIL_NEAR: f64 = 1.2;
const TRAIL_FAR: f64 = 2.4;

/// Share of the full trail drawn for a star at source radius `b` (θE units).
fn trail_share(b: f64) -> f64 {
    1.0 - sciart_core::timeline::ease::smoothstep((b - TRAIL_NEAR) / (TRAIL_FAR - TRAIL_NEAR))
}
/// Image chord (dots) above which a source segment is split, the split depth, and the
/// longest chord still drawn at full depth (longer ones jump across the lens).
const CHORD: f64 = 2.0;
const MAX_DEPTH: u32 = 8;
const MAX_CHORD: f64 = 6.0;
/// The lens: a tiny `Sun` core (radius in dots).
const LENS: Ink = Ink::new(Role::Sun, 255);
const LENS_R: f64 = 1.2;

type V2 = (f64, f64);

/// Lens equation `β = θ − θE² θ/|θ|²` (LENS-A1). The lens position itself maps to
/// infinity; it is reported as `(∞, ∞)` so distance tests simply fail instead of giving NaN.
/// The scene maps sources to images ([`images`]); this direction checks them.
#[cfg(test)]
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

/// `|μ|`, clamped and NaN-safe.
fn abs_mu(mu: f64) -> f64 {
    if mu.is_nan() { 0.0 } else { mu.abs().min(1e6) }
}

/// A star image's level, stepped by magnification: unlensed stars are dimmer dots,
/// magnified ones near the ring flare up.
fn star_level(mu: f64) -> u8 {
    match abs_mu(mu) {
        m if m < 1.1 => 130,
        m if m < 1.6 => 190,
        _ => 255,
    }
}

/// A galaxy image piece's level: stepped by magnification (arcs and the ring are the
/// brightest), scaled by the pass envelope; `None` while the pass is faded out.
fn galaxy_level(mu: f64, envelope: f64) -> Option<u8> {
    if envelope < MIN_ENVELOPE {
        return None;
    }
    let base = match abs_mu(mu) {
        m if m < 4.0 => 230.0,
        _ => 255.0,
    };
    Some((base * (0.45 + 0.55 * envelope.min(1.0))) as u8)
}

/// Galaxy thread `k` (0 the bulge ring, 1 and 2 the arms) at index `i`, in units of the
/// galaxy radius, before spin and projection; and the number of points of the thread.
fn thread_len(k: usize) -> usize {
    if k == 0 { RING_PTS + 1 } else { ARM_PTS + 1 }
}

fn thread_point(k: usize, i: usize) -> (f64, f64) {
    if k == 0 {
        (BULGE, TAU * i as f64 / RING_PTS as f64)
    } else {
        let span = (1.0 / BULGE).ln() / ARM_K;
        let phi = span * i as f64 / ARM_PTS as f64;
        (BULGE * (ARM_K * phi).exp(), phi + PI * (k - 1) as f64)
    }
}

/// Screen geometry: the lens's cell and the Einstein radius (rows).
#[derive(Debug, Clone, Copy, Default)]
struct View {
    cx: f64,
    cy: f64,
    te: f64,
}

impl View {
    /// Dot coordinates of the sky angle `θ` (rows).
    fn to_dot(self, (tx, ty): V2) -> V2 {
        Dots::from_cells(self.cx + 0.5 + tx * ASPECT, self.cy + 0.5 + ty)
    }

    /// Sky angle of dot `(x, y)`, the inverse of [`to_dot`](Self::to_dot).
    #[cfg(test)]
    fn sky_of_dot(self, x: i32, y: i32) -> V2 {
        let (cx, cy) = (
            (f64::from(x) + 0.5) / f64::from(Dots::CELL_W),
            (f64::from(y) + 0.5) / f64::from(Dots::CELL_H),
        );
        ((cx - self.cx - 0.5) / ASPECT, cy - self.cy - 0.5)
    }
}

/// Draw the image, on branch `branch` (0 major, 1 minor), of the source segment `a → b`
/// as a braille thread: the segment is split until each piece's image is at most
/// `CHORD` dots long (an explicit stack, no allocation); pieces still longer than
/// `MAX_CHORD` at full depth straddle the lens and are skipped. `ink(μ)` colours a piece
/// by the magnification at its middle, or skips it.
fn image_thread(
    dots: &mut Dots,
    view: View,
    a: V2,
    b: V2,
    branch: usize,
    mut ink: impl FnMut(f64) -> Option<Ink>,
) {
    let img = |beta: V2| images(beta, view.te)[branch];
    let mid = |p: V2, q: V2| (0.5 * (p.0 + q.0), 0.5 * (p.1 + q.1));
    let mut stack = [(a, b, 0u32); MAX_DEPTH as usize + 2];
    let mut n = 1;
    while n > 0 {
        n -= 1;
        let (p, q, depth) = stack[n];
        let (dp, dq) = (view.to_dot(img(p)), view.to_dot(img(q)));
        let chord = (dq.0 - dp.0).hypot(dq.1 - dp.1);
        if !chord.is_finite() {
            continue;
        }
        if chord > CHORD && depth < MAX_DEPTH {
            let m = mid(p, q);
            stack[n] = (m, q, depth + 1);
            stack[n + 1] = (p, m, depth + 1);
            n += 2;
            continue;
        }
        if chord > MAX_CHORD {
            continue;
        }
        let mu = magnification(img(mid(p, q)), view.te);
        if let Some(ink) = ink(mu) {
            dots.line(dp.0, dp.1, dq.0, dq.1, ink);
        }
    }
}

#[derive(Debug, Default)]
pub struct GravitationalLensing {
    view: View,
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
    /// The galaxy's spin angle (rad).
    spin: f64,
    dots: Dots,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl GravitationalLensing {
    fn star_beta(&self, (u, v): V2) -> V2 {
        ((u - 0.5) * 2.0 * self.hx, (v - 0.5) * 2.0 * self.hy)
    }

    /// Drift velocity of the background in rows per second.
    fn drift(&self) -> V2 {
        (
            DRIFT * self.drift_dir.cos_det(),
            DRIFT * self.drift_dir.sin_det(),
        )
    }

    /// Galaxy centre in the source plane and its brightness envelope for this pass. The
    /// pass eases through closest approach (`0.25u + 0.75u³`), so arcs and ring linger.
    fn galaxy(&self) -> (V2, f64) {
        let s = (self.pass_t / PASS).clamp(0.0, 1.0);
        let u = 2.0 * s - 1.0;
        let along = self.view.te * SWEEP * (0.25 * u + 0.75 * u * u * u);
        let (sn, c) = self.pass_dir.sin_cos_det();
        let beta = (along * c - self.impact * sn, along * sn + self.impact * c);
        (beta, (PI * s).sin_det().max(0.0).powf(0.6))
    }

    fn new_pass(&mut self, ctx: &mut Ctx) {
        self.impact = ctx.rng.random_range(-MAX_IMPACT..MAX_IMPACT);
        self.pass_dir = ctx.rng.random_range(-0.5..0.5);
    }

    /// Source-plane position of galaxy thread `k`'s point `i`: spun, inclined, turned to
    /// the pass's position angle and placed at the galaxy centre `g`.
    fn galaxy_point(&self, g: V2, k: usize, i: usize) -> V2 {
        let (r, phi) = thread_point(k, i);
        let (s, c) = (phi + self.spin).sin_cos_det();
        let (x, y) = (r * c * GALAXY_R, r * s * GALAXY_R * COS_I);
        let (ps, pc) = self.pass_dir.sin_cos_det();
        (g.0 + x * pc - y * ps, g.1 + x * ps + y * pc)
    }

    fn draw_galaxy(&mut self) {
        let (g, envelope) = self.galaxy();
        if envelope < MIN_ENVELOPE || self.view.te <= 0.0 {
            return;
        }
        let view = self.view;
        for k in 0..3 {
            for i in 0..thread_len(k) - 1 {
                let (a, b) = (self.galaxy_point(g, k, i), self.galaxy_point(g, k, i + 1));
                for (branch, role) in [(1, Role::Cold), (0, Role::Wave)] {
                    image_thread(&mut self.dots, view, a, b, branch, |mu| {
                        if abs_mu(mu) < MIN_MU {
                            return None;
                        }
                        galaxy_level(mu, envelope).map(|l| Ink::new(role, l))
                    });
                }
            }
        }
    }

    fn draw_stars(&mut self) {
        let view = self.view;
        let v = self.drift();
        let steps = TRAIL_LEVELS.len();
        for idx in 0..self.stars.len() {
            let beta = self.star_beta(self.stars[idx]);
            let length = TRAIL * trail_share(beta.0.hypot(beta.1) / view.te.max(1e-9));
            for (branch, head) in images(beta, view.te).into_iter().enumerate() {
                let mu = magnification(head, view.te);
                if abs_mu(mu) < MIN_MU {
                    continue;
                }
                // The trail: the image of the last `length` s of drift, oldest step dimmest.
                for (s, &level) in TRAIL_LEVELS.iter().enumerate() {
                    if length < 0.05 {
                        break;
                    }
                    let back = |k: usize| {
                        let age = length * (steps - k) as f64 / steps as f64;
                        (beta.0 - v.0 * age, beta.1 - v.1 * age)
                    };
                    let ink = Ink::new(Role::Star, level);
                    image_thread(&mut self.dots, view, back(s), back(s + 1), branch, |m| {
                        (abs_mu(m) >= MIN_MU).then_some(ink)
                    });
                }
                let (x, y) = view.to_dot(head);
                let ink = Ink::new(Role::Star, star_level(mu));
                let arm = match abs_mu(mu) {
                    m if m < 1.6 => 0,
                    m if m < 3.0 => 1,
                    _ => 2,
                };
                let (x, y) = (x.round(), y.round());
                for a in 0..=arm {
                    let a = f64::from(a);
                    for (dx, dy) in [(a, 0.0), (-a, 0.0), (0.0, a), (0.0, -a)] {
                        self.dots.plot(x + dx, y + dy, ink);
                    }
                }
            }
        }
    }

    fn raster(&mut self) {
        self.dots.clear();
        self.draw_stars();
        self.draw_galaxy();
        let (x, y) = self.view.to_dot((0.0, 0.0));
        self.dots.disc(x, y, LENS_R, LENS);
    }
}

impl Generator for GravitationalLensing {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.view = View {
            cx: f64::from(size.width() / 2),
            cy: f64::from(size.height() / 2),
            te: draw::fit_radius(size, EINSTEIN_FRAC),
        };
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
            self.spin = ctx.rng.random_range(0.0..TAU);
            self.new_pass(ctx);
        }
        self.dots.resize(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        // Drift in rows/s, converted to the normalized sky (hx, hy ≥ 0.25 for any size).
        let (du, dv) = (
            DRIFT * self.drift_dir.cos_det() * dt / (2.0 * self.hx.max(0.25)),
            DRIFT * self.drift_dir.sin_det() * dt / (2.0 * self.hy.max(0.25)),
        );
        for (u, v) in &mut self.stars {
            *u = geom::wrap(*u + du, 1.0);
            *v = geom::wrap(*v + dv, 1.0);
        }
        self.spin = geom::wrap(self.spin + SPIN * dt, TAU);
        self.pass_t += dt;
        if self.pass_t >= PASS {
            self.pass_t = geom::wrap(self.pass_t, PASS);
            self.new_pass(ctx);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
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
        assert_eq!(star_level(f64::NAN), star_level(0.0));
        assert_eq!(star_level(f64::INFINITY), 255);
        assert_eq!(galaxy_level(f64::NAN, 1.0), galaxy_level(0.0, 1.0));
        assert_eq!(galaxy_level(1.0, 0.0), None);
    }

    fn scene(size: Size) -> GravitationalLensing {
        let mut g = GravitationalLensing::default();
        g.resize(size, &mut Ctx::new(9, size));
        g
    }

    /// The galaxy centred behind the lens, no stars, rasterised.
    fn aligned() -> GravitationalLensing {
        let mut g = scene(Size::new(200, 50));
        g.pass_t = PASS / 2.0;
        g.impact = 0.0;
        g.pass_dir = 0.0;
        g.stars.clear();
        g.raster();
        g
    }

    /// Every lit dot as its sky angle, the lens core left out.
    fn lit_thetas(g: &GravitationalLensing) -> Vec<V2> {
        let ds = g.dots.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if g.dots.is_lit(x, y) {
                    let th = g.view.sky_of_dot(x, y);
                    if th.0.hypot(th.1) > 0.6 {
                        out.push(th);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn lens_a4_aligned_galaxy_forms_a_thin_full_einstein_ring() {
        let g = aligned();
        let te = g.view.te;
        // Thin: aligned, the bulge ring's major and minor images straddle θE at most
        // 2 dots apart (1 row = 4 dots, square).
        let [p, m] = images((BULGE * GALAXY_R, 0.0), te);
        let apart = (p.0.hypot(p.1) - m.0.hypot(m.1)) * f64::from(Dots::CELL_H);
        assert!(apart <= 2.0, "ring {apart:.2} dots thick");
        let dots = lit_thetas(&g);
        assert!(!dots.is_empty());
        for &(x, y) in &dots {
            let r = x.hypot(y) / te;
            assert!((0.6..=1.4).contains(&r), "galaxy dot at {r:.2} θE");
        }
        // Every one of 32 directions has a dot on the ring.
        let mut sectors = [false; 32];
        for &(x, y) in &dots {
            if (x.hypot(y) - te).abs() < 0.15 * te {
                let a = geom::wrap(y.atan2(x), TAU);
                sectors[((a / TAU * 32.0) as usize).min(31)] = true;
            }
        }
        assert!(sectors.iter().all(|&s| s), "{sectors:?}");
    }

    #[test]
    fn lens_a5_galaxy_images_are_coloured_by_parity() {
        let g = aligned();
        let size = Size::new(200, 50);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let te = g.view.te;
        let (mut outside, mut inside) = (0, 0);
        for y in 0..50 {
            for x in 0..200 {
                let cell = c.get(x, y).unwrap();
                if cell.ch == ' ' || cell.ink.role == Role::Sun {
                    continue;
                }
                // Cell centre's sky angle; a cell spans ±0.5 rows down and ±0.25 across.
                let th = (
                    (f64::from(x) - g.view.cx) / ASPECT,
                    f64::from(y) - g.view.cy,
                );
                let r = th.0.hypot(th.1);
                if r > te + 0.6 {
                    assert_eq!(cell.ink.role, Role::Wave, "({x},{y}) at {r:.2}");
                    outside += 1;
                } else if r < te - 0.6 {
                    assert_eq!(cell.ink.role, Role::Cold, "({x},{y}) at {r:.2}");
                    inside += 1;
                }
            }
        }
        assert!(outside > 10 && inside > 10, "{outside} / {inside}");
    }

    #[test]
    fn lens_a6_star_trails_are_images_of_the_drift_path() {
        let mut g = scene(Size::new(200, 50));
        g.pass_t = 0.0; // galaxy faded out
        let te = g.view.te;
        let beta = (0.6 * te, 0.15 * te);
        // The normalized position of β.
        let star = (beta.0 / (2.0 * g.hx) + 0.5, beta.1 / (2.0 * g.hy) + 0.5);
        g.stars = vec![star];
        for dir in [0.3, 2.0, 4.4] {
            g.drift_dir = dir;
            g.raster();
            let v = g.drift();
            let tail = (beta.0 - v.0 * TRAIL, beta.1 - v.1 * TRAIL);
            let dots = lit_thetas(&g);
            for &th in &dots {
                let b = source_plane(th, te);
                // Distance from b to the segment tail → beta.
                let (sx, sy) = (beta.0 - tail.0, beta.1 - tail.1);
                let k = (((b.0 - tail.0) * sx + (b.1 - tail.1) * sy) / (sx * sx + sy * sy))
                    .clamp(0.0, 1.0);
                let d = dist(b, (tail.0 + k * sx, tail.1 + k * sy));
                assert!(
                    d < 0.6,
                    "dir {dir}: dot {th:?} maps {d:.2} rows off the path"
                );
            }
            for img in images(beta, te) {
                assert!(
                    dots.iter().any(|&th| dist(th, img) < 0.5),
                    "dir {dir}: image {img:?} not lit"
                );
            }
        }
    }
}
