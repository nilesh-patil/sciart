//! Point-mass gravitational lensing (SPEC-002 `gravitational_lensing`).
//!
//! Angles are measured in rows on the sky, centred on the lens; screen x is stretched by
//! `ASPECT`. A point source at `β` has two images `θ±` on the line through the lens.
//!
//! M17 (orbital-thread style, GEN-R26): ray-traced stipple. The background galaxy is a
//! fixed cloud of source-plane samples (a compact bulge and two logarithmic arms, drawn
//! once from `ctx.rng`, each with its own fixed uniform). Every sample is mapped in closed
//! form to both of its images, so the arcs, the Einstein ring and the faint counter-images
//! *emerge* from the lens equation instead of being drawn. Major images are `Wave`, the
//! parity-flipped minor ones `Cold` (thinned by their magnification). The lens galaxy is a
//! small `Sun` stipple cloud with fixed per-dot thresholds, the critical circle a faint
//! dotted `Dim` thread, and the background stars stay image pairs with bent trails (M16).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::particles::gaussian2;
use sciart_core::prelude::*;
use sciart_core::timeline::ease::smoothstep;
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
/// Galaxy radius in units of `θE` (3.6 rows at 200×50) and the time between its passes.
const GALAXY_R: f64 = 0.41;
const PASS: f64 = 12.0;
/// The galaxy crosses `x ∈ [−SWEEP θE, SWEEP θE]` in the source plane.
const SWEEP: f64 = 2.2;
/// Largest |impact parameter| of a pass, in `θE`: within the bulge's core the pass makes a
/// full ring, otherwise arcs.
const MAX_IMPACT: f64 = 0.1;
/// Display floor: images demagnified below this are too faint to draw. Without it the
/// minor images of all 160 stars pile up inside the Einstein radius
/// (`|μ−| = (μ_tot − 1)/2 ≈ u⁻⁴` for a star at `u = |β|/θE`).
const MIN_MU: f64 = 0.05;

/// Galaxy samples (fixed for the scene's life) and their make-up, in units of the galaxy
/// radius: a share `BULGE_SHARE` in a Gaussian bulge of width `BULGE_SD` (cut at
/// `BULGE_MAX`), the rest along two arms `r = ARM_R0·e^{ARM_K φ}` out to 1, scattered
/// across the arm by `ARM_SD0 + ARM_SD1·r`.
const SAMPLES: usize = 1800;
const BULGE_SHARE: f64 = 0.26;
const BULGE_SD: f64 = 0.07;
const BULGE_MAX: f64 = 0.2;
const ARM_R0: f64 = 0.14;
const ARM_K: f64 = 0.3;
const ARM_SD0: f64 = 0.012;
const ARM_SD1: f64 = 0.05;
/// The disc's spin (rad/s) and inclination (`cos i`, the projected flattening).
const SPIN: f64 = 0.2;
const COS_I: f64 = 0.75;
/// The pass fades in and out by thinning: a sample shows while its uniform is below
/// `smoothstep((envelope − FADE_LO)/(FADE_HI − FADE_LO))`, so the cloud fills in and
/// empties without shimmer.
const FADE_LO: f64 = 0.1;
const FADE_HI: f64 = 0.6;
/// A minor image is kept with probability `min(1, MINOR_K·|μ−|)`: faint counter-images
/// are sparse, the inner arc of a near-aligned pass is solid.
const MINOR_K: f64 = 1.5;

/// Star trails: the last `TRAIL` s of drift, in `TRAIL_LEVELS.len()` steps, oldest first.
const TRAIL: f64 = 2.5;
const TRAIL_LEVELS: [u8; 2] = [70, 120];
/// Trails are drawn where lensing bends them: full length within `TRAIL_NEAR θE` of the
/// lens (source plane), shrinking to nothing at `TRAIL_FAR θE`, so the far field stays
/// single dots instead of a uniform rain of streaks.
const TRAIL_NEAR: f64 = 1.2;
const TRAIL_FAR: f64 = 2.4;
/// Image chord (dots) above which a source segment is split, the split depth, and the
/// longest chord still drawn at full depth (longer ones jump across the lens).
const CHORD: f64 = 2.0;
const MAX_DEPTH: u32 = 8;
const MAX_CHORD: f64 = 6.0;

/// The lens galaxy: a `Sun` stipple out to `LENS_R θE`, keep probability
/// `LENS_P0·exp(−LENS_B·(r/R)^LENS_N)` (a Sérsic-like profile, densest at the centre).
const LENS_R: f64 = 0.3;
const LENS_P0: f64 = 1.0;
const LENS_B: f64 = 2.6;
const LENS_N: f64 = 0.8;
const LENS_SALT: u64 = 0x1e45_c0de_5a17_0001;
/// The critical circle: one `Dim` dot every `RING_GAP` dots of arc.
const RING_GAP: f64 = 4.0;
const RING_INK: Ink = Ink::new(Role::Dim, 90);

/// Share of the full trail drawn for a star at source radius `b` (θE units).
fn trail_share(b: f64) -> f64 {
    1.0 - smoothstep((b - TRAIL_NEAR) / (TRAIL_FAR - TRAIL_NEAR))
}

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

/// A galaxy image dot's level, stepped by magnification: arcs and the ring (`|μ| ≥ 4`) are
/// the brightest. Brightness no longer fades with the pass; the stipple thins instead.
fn galaxy_level(mu: f64) -> u8 {
    if abs_mu(mu) < 4.0 { 215 } else { 255 }
}

/// Keep share of the galaxy samples at pass envelope `envelope` (0 while faded out).
fn pass_keep(envelope: f64) -> f64 {
    smoothstep((envelope - FADE_LO) / (FADE_HI - FADE_LO))
}

/// Keep probability of the lens cloud at `q = r/(LENS_R θE)`: falls monotonically from
/// `LENS_P0` at the centre, 0 outside (LENS-A9).
fn lens_density(q: f64) -> f64 {
    if !(0.0..1.0).contains(&q) {
        return 0.0;
    }
    LENS_P0 * (-LENS_B * q.powf(LENS_N)).exp()
}

/// The lens cloud's ink at `q = r/(LENS_R θE)`: a white-hot core, a `Sun` body.
fn lens_ink(q: f64) -> Ink {
    Ink::new(Role::Sun, if q < 0.35 { 255 } else { 200 })
}

/// One fixed background-galaxy sample: its position in units of the galaxy radius (face
/// on, before spin and tilt) and its fixed uniform (CORE-R107's per-sample threshold).
#[derive(Debug, Clone, Copy)]
struct Sample {
    x: f64,
    y: f64,
    u: f64,
}

/// The fixed galaxy cloud: a Gaussian bulge and two scattered logarithmic arms.
fn galaxy_samples(rng: &mut impl Rng) -> Vec<Sample> {
    let span = (1.0 / ARM_R0).ln() / ARM_K;
    (0..SAMPLES)
        .map(|i| {
            let (x, y) = if (i as f64) < BULGE_SHARE * SAMPLES as f64 {
                let (mut x, mut y) = gaussian2(rng, BULGE_SD);
                let r = x.hypot(y);
                if r > BULGE_MAX {
                    // Fold the rare far tail back inside rather than redraw (bounded).
                    let k = (BULGE_MAX * rng.random_range(0.2..1.0)) / r;
                    (x, y) = (x * k, y * k);
                }
                (x, y)
            } else {
                let phi = span * rng.random_range(0.0_f64..1.0);
                let r = ARM_R0 * (ARM_K * phi).exp();
                let a = phi
                    + if i % 2 == 0 {
                        0.0
                    } else {
                        std::f64::consts::PI
                    };
                let (s, c) = a.sin_cos_det();
                let (dr, dt) = gaussian2(rng, ARM_SD0 + ARM_SD1 * r);
                // Scatter across and along the arm; clamp to the galaxy radius.
                let (x, y) = ((r + dr) * c - dt * s, (r + dr) * s + dt * c);
                let rr = x.hypot(y);
                if rr > 1.0 { (x / rr, y / rr) } else { (x, y) }
            };
            Sample {
                x,
                y,
                u: rng.random_range(0.0..1.0),
            }
        })
        .collect()
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

    /// Dots per row (the lattice is square: 4 dots per row, 4 per `ASPECT` columns).
    const DOTS_PER_ROW: f64 = Dots::CELL_H as f64;
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

/// One frame of the galaxy's pass: its source-plane centre, the keep share of its
/// samples and the galaxy-frame → source-plane matrix.
#[derive(Debug, Clone, Copy)]
struct GalaxyFrame {
    centre: V2,
    keep: f64,
    m: [f64; 4],
}

impl GalaxyFrame {
    /// Source-plane position of a sample.
    fn beta(&self, s: &Sample) -> V2 {
        let m = self.m;
        (
            self.centre.0 + m[0] * s.x + m[1] * s.y,
            self.centre.1 + m[2] * s.x + m[3] * s.y,
        )
    }
}

/// Every drawn galaxy image dot: `f(dot, ink, sample index, image θ)`. A sample shows
/// while its uniform is below the pass's keep share; its major image is `Wave`, its minor
/// image `Cold` and kept only while `u < MINOR_K·|μ−|` (LENS-A5, LENS-A7).
fn galaxy_dots(
    samples: &[Sample],
    view: View,
    frame: &GalaxyFrame,
    mut f: impl FnMut((i32, i32), Ink, usize, V2),
) {
    let te = view.te;
    for (i, s) in samples.iter().enumerate() {
        if s.u >= frame.keep {
            continue;
        }
        let [p, q] = images(frame.beta(s), te);
        for (theta, role) in [(p, Role::Wave), (q, Role::Cold)] {
            let mu = magnification(theta, te);
            let a = abs_mu(mu);
            if a < MIN_MU || (role == Role::Cold && s.u >= MINOR_K * a) {
                continue;
            }
            let (x, y) = view.to_dot(theta);
            if x.is_finite() && y.is_finite() {
                f(
                    (x.round() as i32, y.round() as i32),
                    Ink::new(role, galaxy_level(mu)),
                    i,
                    theta,
                );
            }
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
    /// The background galaxy's fixed sample cloud (LENS-A8).
    samples: Vec<Sample>,
    /// Time into the current galaxy pass, its impact parameter (θE) and direction.
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
        let te = self.view.te;
        let along = te * SWEEP * (0.25 * u + 0.75 * u * u * u);
        let (sn, c) = self.pass_dir.sin_cos_det();
        let b = self.impact * te;
        let beta = (along * c - b * sn, along * sn + b * c);
        (
            beta,
            (std::f64::consts::PI * s).sin_det().max(0.0).powf(0.6),
        )
    }

    fn new_pass(&mut self, ctx: &mut Ctx) {
        self.impact = ctx.rng.random_range(-MAX_IMPACT..MAX_IMPACT);
        self.pass_dir = ctx.rng.random_range(-0.5..0.5);
    }

    /// The map from a sample's galaxy-frame position to its source-plane offset (rows):
    /// spin, then the inclination's flattening, then the pass's position angle, scaled to
    /// the galaxy radius. One 2×2 matrix per frame, so a sample costs four multiplies.
    fn galaxy_matrix(&self) -> [f64; 4] {
        let (ss, sc) = self.spin.sin_cos_det();
        let (ps, pc) = self.pass_dir.sin_cos_det();
        let k = GALAXY_R * self.view.te;
        // R(pass) · diag(1, cos i) · R(spin)
        let (a, b, c, d) = (sc, -ss, COS_I * ss, COS_I * sc);
        [
            k * (pc * a - ps * c),
            k * (pc * b - ps * d),
            k * (ps * a + pc * c),
            k * (ps * b + pc * d),
        ]
    }

    /// This frame's galaxy: its centre, the keep share of its samples and the
    /// galaxy-frame → source-plane matrix; `None` while it is faded out.
    fn galaxy_frame(&self) -> Option<GalaxyFrame> {
        let (centre, envelope) = self.galaxy();
        let keep = pass_keep(envelope);
        (keep > 0.0 && self.view.te > 0.0).then(|| GalaxyFrame {
            centre,
            keep,
            m: self.galaxy_matrix(),
        })
    }

    fn draw_galaxy(&mut self) {
        if let Some(frame) = self.galaxy_frame() {
            let dots = &mut self.dots;
            galaxy_dots(&self.samples, self.view, &frame, |(x, y), ink, _, _| {
                dots.set(x, y, ink);
            });
        }
    }

    /// The lens galaxy: fixed-threshold stipple within `LENS_R θE` (CORE-R107, LENS-A9).
    fn draw_lens(&mut self) {
        let view = self.view;
        let r = LENS_R * view.te * View::DOTS_PER_ROW;
        let (cx, cy) = view.to_dot((0.0, 0.0));
        let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
        let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let q = (f64::from(x) - cx).hypot(f64::from(y) - cy) / r.max(1e-9);
                self.dots
                    .stipple(x, y, lens_density(q), LENS_SALT, lens_ink(q));
            }
        }
        // The nucleus is always lit.
        self.dots.plot(cx, cy, lens_ink(0.0));
    }

    /// The critical curve `|θ| = θE`: a faint dotted thread.
    fn draw_critical_circle(&mut self) {
        let view = self.view;
        let r = view.te * View::DOTS_PER_ROW;
        let n = ((TAU * r / RING_GAP).round() as usize).clamp(8, 4096);
        let (cx, cy) = view.to_dot((0.0, 0.0));
        for k in 0..n {
            let (s, c) = (TAU * k as f64 / n as f64).sin_cos_det();
            self.dots.plot(cx + r * c, cy + r * s, RING_INK);
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
        self.draw_critical_circle();
        self.draw_lens();
        self.draw_stars();
        self.draw_galaxy();
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
            // Normalized positions and the galaxy's samples (in units of its radius)
            // survive resizes; only the sky's scale changes.
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
            self.samples = galaxy_samples(&mut ctx.rng);
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
        assert_eq!(galaxy_level(f64::NAN), galaxy_level(0.0));
        assert_eq!(pass_keep(0.0), 0.0);
        assert_eq!(pass_keep(1.0), 1.0);
        assert_eq!(lens_density(f64::NAN), 0.0);
    }

    fn scene(size: Size) -> GravitationalLensing {
        let mut g = GravitationalLensing::default();
        g.resize(size, &mut Ctx::new(9, size));
        g
    }

    /// The galaxy centred behind the lens at mid-pass.
    fn aligned() -> GravitationalLensing {
        let mut g = scene(Size::new(200, 50));
        g.pass_t = PASS / 2.0;
        g.impact = 0.0;
        g.pass_dir = 0.0;
        g.stars.clear();
        g
    }

    /// The lattice with only the galaxy's images drawn.
    fn galaxy_only(g: &mut GravitationalLensing) {
        g.dots.clear();
        g.draw_galaxy();
    }

    /// Every lit dot as `(dot, sky angle)`.
    fn lit(g: &GravitationalLensing) -> Vec<((i32, i32), V2)> {
        let ds = g.dots.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if g.dots.is_lit(x, y) {
                    out.push(((x, y), g.view.sky_of_dot(x, y)));
                }
            }
        }
        out
    }

    #[test]
    fn lens_a4_aligned_galaxy_forms_a_thin_full_einstein_ring() {
        let mut g = aligned();
        galaxy_only(&mut g);
        let te = g.view.te;
        let dots = lit(&g);
        assert!(dots.len() > 400, "{} galaxy dots", dots.len());
        for &(_, (x, y)) in &dots {
            let r = x.hypot(y) / te;
            assert!((0.6..=1.4).contains(&r), "galaxy dot at {r:.2} θE");
        }
        // Thin: the bulge's images hug θE (its 1σ core maps to within ±0.2 rows), and
        // every one of 32 directions has a dot on the ring.
        let [p, m] = images((BULGE_SD * GALAXY_R * te, 0.0), te);
        let apart = (p.0.hypot(p.1) - m.0.hypot(m.1)) * View::DOTS_PER_ROW;
        assert!(apart <= 2.0, "bulge core ring {apart:.2} dots thick");
        let mut sectors = [false; 32];
        for &(_, (x, y)) in &dots {
            if (x.hypot(y) - te).abs() < 0.15 * te {
                let a = geom::wrap(y.atan2(x), TAU);
                sectors[((a / TAU * 32.0) as usize).min(31)] = true;
            }
        }
        assert!(sectors.iter().all(|&s| s), "{sectors:?}");
    }

    #[test]
    fn lens_a5_galaxy_images_are_coloured_by_parity() {
        let mut g = aligned();
        g.raster();
        let size = Size::new(200, 50);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let te = g.view.te;
        let (mut outside, mut inside) = (0, 0);
        for y in 0..50 {
            for x in 0..200 {
                let cell = c.get(x, y).unwrap();
                if cell.ch == ' ' || matches!(cell.ink.role, Role::Sun | Role::Dim) {
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
            g.dots.clear();
            g.draw_stars();
            let v = g.drift();
            let tail = (beta.0 - v.0 * TRAIL, beta.1 - v.1 * TRAIL);
            let dots: Vec<V2> = lit(&g).into_iter().map(|(_, th)| th).collect();
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

    #[test]
    fn lens_a7_galaxy_dots_are_lens_images_of_their_samples() {
        // Several moments of a pass, off-axis: every drawn dot is within one dot of the
        // lens-equation image of its own sample, and every lit dot is such an image.
        let mut g = scene(Size::new(200, 50));
        g.stars.clear();
        g.impact = 0.06;
        for t in [3.0, 5.0, 6.0, 7.5] {
            g.pass_t = t;
            galaxy_only(&mut g);
            let te = g.view.te;
            let frame = g.galaxy_frame().expect("galaxy in view");
            let mut drawn = Vec::new();
            galaxy_dots(&g.samples, g.view, &frame, |dot, ink, i, theta| {
                let beta = frame.beta(&g.samples[i]);
                let img = images(beta, te)[usize::from(ink.role == Role::Cold)];
                assert!(dist(img, theta) < 1e-12);
                let (x, y) = g.view.to_dot(img);
                let off = (f64::from(dot.0) - x).hypot(f64::from(dot.1) - y);
                assert!(off <= 1.0, "t {t}: sample {i} drawn {off:.2} dots off");
                // Its source maps back through the lens equation.
                assert!(dist(source_plane(theta, te), beta) < 1e-9);
                drawn.push(dot);
            });
            assert!(drawn.len() > 300, "t {t}: {} dots", drawn.len());
            for (dot, _) in lit(&g) {
                assert!(drawn.contains(&dot), "t {t}: stray dot {dot:?}");
            }
        }
    }

    #[test]
    fn lens_a8_the_sample_set_is_fixed() {
        let mut g = scene(Size::new(200, 50));
        let n = g.samples.len();
        assert_eq!(n, SAMPLES);
        let first = (g.samples[0].x, g.samples[0].y, g.samples[0].u);
        g.raster();
        let a = lit(&g);
        g.raster();
        assert_eq!(a, lit(&g), "a still scene re-renders identically");
        let mut ctx = Ctx::new(9, Size::new(200, 50));
        for _ in 0..(3.0 * PASS * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g.resize(Size::new(120, 40), &mut ctx);
        assert_eq!(g.samples.len(), n, "samples never added or dropped");
        assert_eq!((g.samples[0].x, g.samples[0].y, g.samples[0].u), first);
    }

    #[test]
    fn lens_a9_lens_cloud_is_a_central_sun_stipple() {
        let mut g = scene(Size::new(200, 50));
        g.dots.clear();
        g.draw_lens();
        let te = g.view.te;
        let mut c = Canvas::new(Size::new(200, 50), GlyphMode::Unicode);
        g.dots.blit(&mut c);
        for y in 0..50 {
            for x in 0..200 {
                let cell = c.get(x, y).unwrap();
                if cell.ch != ' ' {
                    assert_eq!(cell.ink.role, Role::Sun);
                }
            }
        }
        // Density in three rings (dots lit / dots in ring) falls with radius, and nothing
        // lies beyond 0.35 θE.
        let (mut lit_n, mut all_n) = ([0_u32; 3], [0_u32; 3]);
        let r_max = LENS_R * te * View::DOTS_PER_ROW;
        let (cx, cy) = g.view.to_dot((0.0, 0.0));
        let ds = g.dots.dot_size();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                let r = (f64::from(x) - cx).hypot(f64::from(y) - cy);
                let on = g.dots.is_lit(x, y);
                if on {
                    assert!(
                        r / View::DOTS_PER_ROW <= 0.35 * te,
                        "lens dot at {r:.1} dots"
                    );
                }
                let k = (3.0 * r / r_max) as usize;
                if k < 3 {
                    all_n[k] += 1;
                    lit_n[k] += u32::from(on);
                }
            }
        }
        let d: Vec<f64> = (0..3)
            .map(|k| f64::from(lit_n[k]) / f64::from(all_n[k]))
            .collect();
        assert!(d[0] > d[1] && d[1] > d[2] && d[2] > 0.0, "{d:?}");
        // Fixed thresholds: the cloud holds still between frames.
        let before = lit(&g);
        g.dots.clear();
        g.draw_lens();
        assert_eq!(before, lit(&g));
    }
}
