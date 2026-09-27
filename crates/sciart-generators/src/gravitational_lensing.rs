//! Galaxy-scale strong lensing by a singular isothermal sphere (SPEC-002
//! `gravitational_lensing`).
//!
//! Angles are in units of the Einstein radius `θE` on the sky, centred on the lens; on
//! screen `θE` is `R(0.4)` rows and x is stretched by `ASPECT`. The picture is drawn by
//! inverse ray-shooting: every lattice node `θ` is mapped back to the source plane with
//! the lens equation `β = θ − θ̂` (fixed, built in `resize`) and takes the source's surface
//! brightness there, which lensing conserves. Each step only samples the moving sources.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::{self, ASPECT};
use sciart_core::grid::lattice_for;
use sciart_core::prelude::*;
use sciart_core::raster::Tone;
use sciart_core::timeline::ease;
use sciart_core::{draw, ink};

use crate::black_hole::ascii_cell;

pub static META: Meta = Meta {
    name: "gravitational_lensing",
    category: Category::DeepCosmos,
    description: "A galaxy lens bending background galaxies into arcs and an Einstein ring",
    title: "Gravity lens · mass bends light",
    captions: &[RING],
};

/// Caption while the hero galaxy's light covers the lens axis (LENS-A6).
const RING: &str = "Einstein ring";
/// Hero surface brightness on the axis from which the closed ring counts as shown.
const RING_LIGHT: f64 = 0.05;

pub fn build() -> Box<dyn Generator> {
    Box::new(GravitationalLensing::default())
}

/// Einstein radius as a fraction of the fitted radius, `θE = R(0.4)`.
const EINSTEIN_FRAC: f64 = 0.4;
/// Cap on ray-shot lattice nodes (ADR-0012): the 200×50 display's 20 000 pixels fit.
const MAX_NODES: usize = 24_000;

/// Background field: a tileable texture of `TEX`² texels with period `PERIOD` θE,
/// holding `FIELD` small galaxies, drifting at `DRIFT` θE/s. Their peak brightness is
/// `FIELD_FAINT + FIELD_SPAN · u³` (`u` uniform): mostly faint smudges, a few bright
/// ones, all well below the hero's peak of ~1.5 (LENS-A7).
const TEX: usize = 192;
const PERIOD: f64 = 6.0;
const FIELD: usize = 28;
const FIELD_FAINT: f64 = 0.03;
const FIELD_SPAN: f64 = 0.5;
const DRIFT: f64 = 0.12;

/// Hero galaxy: radius (θE), time between passes (s), the crossing's half-length (θE)
/// and the largest impact parameter as a fraction of the hero's minor semi-axis
/// `HERO_R · cos i`, so the axis passes well inside its bright disk and every pass
/// closes the ring (LENS-A6).
const HERO_R: f64 = 0.6;
const PASS: f64 = 14.0;
const SWEEP: f64 = 2.2;
const IMPACT: f64 = 0.35;
/// The pass slows near closest approach: position `SWEEP (x (1 − k) + k x³)` for
/// `x ∈ [−1, 1]`, so the closed ring holds for several seconds (LENS-A6).
const LINGER: f64 = 0.6;
/// Relative contrast of the hero's spiral arms.
const ARMS: f64 = 0.55;

/// Lens light: a Sérsic n = 2 core with effective radius `LENS_RE` θE and centre
/// `LENS_PEAK`, plus a faint `r⁻²` halo, the sum tapered to zero between `FADE.0` and
/// `FADE.1` θE, inside the critical curve, so a dark gap is left for the Einstein ring
/// (stylised, SPEC-002). Below the faintest visible level it is halftoned (LENS-A8), so
/// the glow fades out instead of ending on an edge.
const LENS_RE: f64 = 0.2;
const LENS_PEAK: f64 = 1.5;
const HALO_PEAK: f64 = 0.06;
const HALO_CORE: f64 = 0.2;
const FADE: (f64, f64) = (0.3, 0.6);

// Pixels are square: a cell is one pixel wide and two tall.
const _: () = assert!(ASPECT == 2.0);
/// Critical-curve dashes around the circle, and their ink.
const DASHES: f64 = 36.0;
const CURVE: Ink = Ink::new(Role::Structure, 40);

/// Source and lens brightness in `[0, 1]` map to ramp levels through an asinh stretch.
const KNEE: f32 = 0.08;
const WHITE: f32 = 1.1;

/// Lens equation of a singular isothermal sphere in units of `θE`: `β = θ − θ̂`
/// (LENS-A1). The lens centre maps to itself.
pub(crate) fn source_plane(theta: (f64, f64)) -> (f64, f64) {
    let r = theta.0.hypot(theta.1);
    if r == 0.0 {
        return theta;
    }
    let k = 1.0 - 1.0 / r;
    (theta.0 * k, theta.1 * k)
}

/// The images of a point source at `β` (θE units): `β̂(|β| + 1)` always, and
/// `β̂(|β| − 1)` on the far side of the lens when `|β| < 1` (LENS-A1..A3).
#[cfg(test)]
pub(crate) fn images(beta: (f64, f64)) -> ([f64; 2], Option<[f64; 2]>) {
    let b = beta.0.hypot(beta.1);
    let (ux, uy) = if b > 0.0 {
        (beta.0 / b, beta.1 / b)
    } else {
        (1.0, 0.0)
    };
    let outer = [ux * (b + 1.0), uy * (b + 1.0)];
    let inner = (b < 1.0).then_some([ux * (b - 1.0), uy * (b - 1.0)]);
    (outer, inner)
}

/// Surface brightness of the hero spiral at source offset `q` (θE units) from its
/// centre, seen at inclination `cos_inc` and position angle `(pa_cos, pa_sin)`: an
/// exponential disk with two logarithmic arms of relative contrast `arms` (0: the
/// arm-averaged disk) and a small bulge, 0 beyond its rim.
fn hero_profile(q: (f64, f64), cos_inc: f64, pa: (f64, f64), arms: f64) -> f64 {
    let u = (q.0 * pa.0 + q.1 * pa.1) / HERO_R;
    let v = (-q.0 * pa.1 + q.1 * pa.0) / (HERO_R * cos_inc);
    let rho2 = u * u + v * v;
    if rho2 >= 1.0 {
        return 0.0;
    }
    let rho = rho2.sqrt();
    // Arms wound with a 20° pitch: phase constant along ln ρ = tan(pitch)·angle.
    let arm = (2.0 * (v.atan2(u) - rho.max(1e-3).ln() / 0.364)).cos_det();
    let disk = (-rho / 0.35).exp() * (1.0 + arms * arm);
    let bulge = (-(rho / 0.1).powi(2)).exp();
    let rim = 1.0 - rho2 * rho2;
    (0.44 * disk + 0.7 * bulge) * rim
}

/// Lens-galaxy light at `r` θE from its centre: a Sérsic n = 2 core and an `r⁻²` halo,
/// the sum tapered to zero over `FADE` (inside the critical curve).
fn lens_light(r: f64) -> f64 {
    let halo = HALO_PEAK / (1.0 + (r / HALO_CORE).powi(2));
    let taper = 1.0 - ease::smoothstep((r - FADE.0) / (FADE.1 - FADE.0));
    (LENS_PEAK * (-3.67 * (r / LENS_RE).sqrt()).exp() + halo) * taper
}

/// The brightness the tone maps just above `MIN_VISIBLE_LEVEL` (inverting the stretch).
fn faintest_visible() -> f64 {
    let (knee, white) = (f64::from(KNEE), f64::from(WHITE));
    let level = ink::MIN_VISIBLE_LEVEL + 0.5;
    knee * (level / 255.0 * (white / knee).asinh()).sinh()
}

/// Halftone light fainter than `faintest` (the value of the lowest visible level) with an
/// ordered 4 × 4 Bayer threshold at node `(i, j)`: kept at `faintest` where
/// `v / faintest` exceeds it, else dropped, so a fading glow thins out into an even dot
/// pattern instead of stopping on an edge or turning to grain (LENS-A8).
fn halftone(v: f64, faintest: f64, (i, j): (usize, usize)) -> f64 {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let threshold = (f64::from(BAYER[j % 4][i % 4]) + 0.5) / 16.0;
    if v >= faintest {
        v
    } else if v > threshold * faintest {
        faintest
    } else {
        0.0
    }
}

/// The background field: `FIELD` exponential ellipses stamped into a periodic texture.
fn paint_field(rng: &mut impl Rng) -> Grid<f32> {
    let mut tex = Grid::new(Size::new(TEX as u16, TEX as u16), 0.0f32);
    let texel = PERIOD / TEX as f64;
    for _ in 0..FIELD {
        let (cx, cy) = (rng.random_range(0.0..PERIOD), rng.random_range(0.0..PERIOD));
        let a = rng.random_range(0.04..0.14);
        let q = rng.random_range(0.35..1.0);
        let (s, c) = rng.random_range(0.0..PI).sin_cos_det();
        let peak = FIELD_FAINT + FIELD_SPAN * rng.random_range(0.0f64..1.0).powi(3);
        let reach = (4.0 * a / texel).ceil() as i32;
        let (ix, iy) = ((cx / texel) as i32, (cy / texel) as i32);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (x, y) = (f64::from(ix + dx) + 0.5, f64::from(iy + dy) + 0.5);
                let (ox, oy) = (x * texel - cx, y * texel - cy);
                let (u, v) = (ox * c + oy * s, (-ox * s + oy * c) / q);
                let e = peak * (-(u.hypot(v)) / (0.4 * a)).exp();
                let wrap = |i: i32| i.rem_euclid(TEX as i32);
                if let Some(t) = tex.get_mut(wrap(ix + dx), wrap(iy + dy)) {
                    *t += e as f32;
                }
            }
        }
    }
    tex
}

/// Bilinear, periodic sample of the field at source position `(x, y)` (θE units).
fn sample_field(tex: &Grid<f32>, x: f64, y: f64) -> f32 {
    let n = TEX as f64;
    let (fx, fy) = (
        geom::wrap(x / PERIOD * n - 0.5, n),
        geom::wrap(y / PERIOD * n - 0.5, n),
    );
    let (x0, y0) = ((fx as usize).min(TEX - 1), (fy as usize).min(TEX - 1));
    let (x1, y1) = ((x0 + 1) % TEX, (y0 + 1) % TEX);
    let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
    let t = tex.as_slice();
    let at = |x: usize, y: usize| t[y * TEX + x];
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

#[derive(Debug, Default)]
pub struct GravitationalLensing {
    seeded: bool,
    t: f64,
    field: Grid<f32>,
    drift_dir: (f64, f64),
    /// Time into the current hero pass, its impact parameter, direction, inclination
    /// and position angle.
    pass_t: f64,
    impact: f64,
    pass_dir: f64,
    cos_inc: f64,
    pa: (f64, f64),
    /// Per node: source-plane position `β` (θE units).
    beta: Vec<(f32, f32)>,
    /// Source brightness and (static) lens light per node.
    source: Grid<f32>,
    lens: Grid<f32>,
    map: SampleMap,
    tone: Tone,
    /// Lens centre in pixels and `θE` in pixels (x is already square, like the rows).
    centre: (f64, f64),
    theta_e_px: f64,
    pixels: Pixels,
}

impl GravitationalLensing {
    /// Hero centre in the source plane (θE units) and its brightness envelope.
    fn hero(&self) -> ((f64, f64), f64) {
        let s = (self.pass_t / PASS).clamp(0.0, 1.0);
        let x = 2.0 * s - 1.0;
        let along = SWEEP * (x * (1.0 - LINGER) + LINGER * x * x * x);
        let (sn, c) = self.pass_dir.sin_cos_det();
        let centre = (along * c - self.impact * sn, along * sn + self.impact * c);
        // Fading in and out hides the start and end of a pass.
        (centre, (PI * s).sin_det().powf(0.6))
    }

    /// The hero's light on the lens axis `β = 0`, which the critical curve `|θ| = θE`
    /// images all the way round, arm-averaged so the arms passing over the axis do not
    /// flicker the caption; the ring is closed while it shows (LENS-A6).
    fn axis_light(&self) -> f64 {
        let (hero, envelope) = self.hero();
        envelope * hero_profile((-hero.0, -hero.1), self.cos_inc, self.pa, 0.0)
    }

    fn new_pass(&mut self, ctx: &mut Ctx) {
        self.pass_dir = ctx.rng.random_range(-0.6..0.6);
        self.cos_inc = ctx.rng.random_range(0.45..0.95);
        self.impact = ctx.rng.random_range(-1.0..1.0) * IMPACT * self.cos_inc;
        self.pa = ctx.rng.random_range(0.0..PI).sin_cos_det();
    }

    /// Node centres in θE units, row-major over the lattice.
    fn node_thetas(&self) -> impl Iterator<Item = (f64, f64)> + use<> {
        let (lw, lh) = (self.source.width(), self.source.height());
        let px = self.pixels.pixel_size();
        let (sx, sy) = (
            f64::from(px.width()) / f64::from(lw),
            f64::from(px.height()) / f64::from(lh),
        );
        let (c, te) = (self.centre, self.theta_e_px);
        (0..lh).flat_map(move |j| {
            (0..lw).map(move |i| {
                let x = (f64::from(i) + 0.5) * sx;
                let y = (f64::from(j) + 0.5) * sy;
                ((x - c.0) / te, (y - c.1) / te)
            })
        })
    }

    /// Sample every source at every node's `β`, then fill the pixels.
    fn raster(&mut self) {
        let (hero, envelope) = self.hero();
        let (ox, oy) = (
            DRIFT * self.t * self.drift_dir.0,
            DRIFT * self.t * self.drift_dir.1,
        );
        let reach2 = (HERO_R * HERO_R) as f32;
        for (s, &(bx, by)) in self.source.as_mut_slice().iter_mut().zip(&self.beta) {
            let (bx, by) = (f64::from(bx), f64::from(by));
            let mut v = sample_field(&self.field, bx - ox, by - oy);
            let q = (bx - hero.0, by - hero.1);
            if envelope > 0.0 && ((q.0 * q.0 + q.1 * q.1) as f32) < reach2 {
                v += (envelope * hero_profile(q, self.cos_inc, self.pa, ARMS)) as f32;
            }
            *s = v;
        }
        let (map, source, lens, tone) = (&self.map, &self.source, &self.lens, &self.tone);
        let direct = source.size() == self.pixels.pixel_size();
        let width = usize::from(source.width());
        let (c, te) = (self.centre, self.theta_e_px);
        self.pixels.shade(|x, y| {
            let (s, l) = if direct {
                let i = y as usize * width + x as usize;
                (source.as_slice()[i], lens.as_slice()[i])
            } else {
                (map.sample(source, x, y), map.sample(lens, x, y))
            };
            let (level, ramp) = if s >= l {
                (tone.level(s), Ramp::Ice)
            } else {
                (tone.level(l), Ramp::Ember)
            };
            // Ramps are tinted even at level 0: faint light ends by removing ink (CORE-R66).
            if f64::from(level) >= ink::MIN_VISIBLE_LEVEL {
                return Ink::new(Role::Ramp(ramp), level);
            }
            // The critical curve, dashed, only where nothing else shines.
            let (dx, dy) = (f64::from(x) + 0.5 - c.0, f64::from(y) + 0.5 - c.1);
            let on_curve = ((dx * dx + dy * dy).sqrt() - te).abs() < 0.5;
            if on_curve && geom::wrap(dy.atan2(dx) / TAU * DASHES, 1.0) < 0.5 {
                CURVE
            } else {
                Ink::BLANK
            }
        });
    }
}

impl Generator for GravitationalLensing {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seeded = true;
            self.field = paint_field(&mut ctx.rng);
            self.drift_dir = ctx.rng.random_range(0.0..TAU).sin_cos_det();
            self.pass_t = ctx.rng.random_range(0.0..PASS);
            self.tone = Tone::asinh(KNEE, WHITE);
            self.new_pass(ctx);
        }
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        // Pixels are square (ASPECT = 2): a fitted radius of R cells is 2R pixel rows.
        self.centre = (f64::from(size.width()) / 2.0, f64::from(px.height()) / 2.0);
        self.theta_e_px = 2.0 * draw::fit_radius(size, EINSTEIN_FRAC);
        let lattice = lattice_for(px, MAX_NODES, 1.0);
        self.source.resize(lattice, 0.0);
        self.lens.resize(lattice, 0.0);
        self.map.rebuild(lattice, px);
        self.beta.clear();
        let thetas = self.node_thetas();
        let faintest = faintest_visible();
        let lw = usize::from(lattice.width());
        for (k, ((tx, ty), l)) in thetas.zip(self.lens.as_mut_slice()).enumerate() {
            let (bx, by) = source_plane((tx, ty));
            self.beta.push((bx as f32, by as f32));
            *l = halftone(lens_light(tx.hypot(ty)), faintest, (k % lw, k / lw)) as f32;
        }
        self.raster();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        self.pass_t += dt;
        if self.pass_t >= PASS {
            self.pass_t = geom::wrap(self.pass_t, PASS);
            self.new_pass(ctx);
        }
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }

    fn caption(&self) -> Option<&'static str> {
        (self.axis_light() >= RING_LIGHT).then_some(RING)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: [f64; 2], b: (f64, f64)) -> f64 {
        (a[0] - b.0).hypot(a[1] - b.1)
    }

    fn betas() -> impl Iterator<Item = (f64, f64)> {
        (-12..=12).flat_map(|i| (-12..=12).map(move |j| (f64::from(i) * 0.17, f64::from(j) * 0.09)))
    }

    #[test]
    fn lens_a1_every_image_maps_back_to_the_source() {
        for beta in betas() {
            let (outer, inner) = images(beta);
            for theta in std::iter::once(outer).chain(inner) {
                let back = source_plane((theta[0], theta[1]));
                assert!(
                    dist([back.0, back.1], beta) < 1e-12,
                    "β={beta:?} θ={theta:?}"
                );
            }
        }
    }

    #[test]
    fn lens_a2_two_images_at_one_plus_and_minus_beta_approach_the_einstein_radius() {
        for b in [0.5, 0.1, 1e-3, 1e-8] {
            let (outer, inner) = images((0.6 * b, -0.8 * b));
            let inner = inner.expect("a source inside θE has a second image");
            assert!((outer[0].hypot(outer[1]) - (1.0 + b)).abs() < 1e-12);
            assert!((inner[0].hypot(inner[1]) - (1.0 - b)).abs() < 1e-12);
            // On opposite sides of the lens.
            assert!(outer[0] * inner[0] + outer[1] * inner[1] < 0.0);
        }
    }

    #[test]
    fn lens_a3_outside_the_einstein_radius_one_image_shifted_by_theta_e() {
        for b in [1.0 + 1e-9, 1.5, 4.0, 100.0] {
            let beta = (0.6 * b, 0.8 * b);
            let (outer, inner) = images(beta);
            assert!(inner.is_none(), "b={b}");
            assert!((dist(outer, beta) - 1.0).abs() < 1e-12, "b={b}");
        }
    }

    fn scene(size: Size) -> GravitationalLensing {
        let mut g = GravitationalLensing::default();
        g.resize(size, &mut Ctx::new(9, size));
        g
    }

    #[test]
    fn lens_a4_every_node_shows_the_source_brightness_at_its_beta() {
        let mut g = scene(Size::new(120, 40));
        let mut ctx = Ctx::new(1, Size::new(1, 1));
        for _ in 0..90 {
            g.update(SIM_DT, &mut ctx);
        }
        let (hero, envelope) = g.hero();
        let (ox, oy) = (DRIFT * g.t * g.drift_dir.0, DRIFT * g.t * g.drift_dir.1);
        let peak = g.field.as_slice().iter().fold(0.0f32, |m, &v| m.max(v)) + envelope as f32 * 1.5;
        for (theta, &s) in g.node_thetas().zip(g.source.as_slice()) {
            // Recompute β from θ independently of the stored map.
            let beta = source_plane(theta);
            let q = (beta.0 - hero.0, beta.1 - hero.1);
            let expect = f64::from(sample_field(&g.field, beta.0 - ox, beta.1 - oy))
                + envelope * hero_profile(q, g.cos_inc, g.pa, ARMS);
            assert!((f64::from(s) - expect).abs() < 1e-4, "θ={theta:?}");
            assert!(s <= peak, "lensing never brightens a surface");
        }
    }

    #[test]
    fn lens_a5_an_aligned_hero_closes_the_einstein_ring() {
        let size = Size::new(200, 50);
        let mut g = scene(size);
        g.pass_t = PASS / 2.0;
        g.impact = 0.0;
        g.pass_dir = 0.0;
        g.field.fill(0.0);
        g.raster();
        let (c, te) = (g.centre, g.theta_e_px);
        let at = |r: f64, a: f64| {
            let (x, y) = (c.0 + r * te * a.cos_det(), c.1 + r * te * a.sin_det());
            g.pixels.get(x as i32, y as i32).unwrap()
        };
        for k in 0..32 {
            let a = TAU * f64::from(k) / 32.0;
            assert_eq!(at(1.0, a).role, Role::Ramp(Ramp::Ice), "gap at angle {a}");
            assert_ne!(
                at(0.3, a).role,
                Role::Ramp(Ramp::Ice),
                "light inside at {a}"
            );
        }
        assert_eq!(g.caption(), Some(RING));
    }

    #[test]
    fn lens_a6_caption_names_the_ring_exactly_while_the_hero_lights_the_axis() {
        let mut g = scene(Size::new(200, 50));
        g.field.fill(0.0);
        let mut ctx = Ctx::new(4, Size::new(1, 1));
        let (c, te) = (g.centre, g.theta_e_px);
        let (mut on, mut off) = (0, 0);
        // Lengths of the closed-ring spells (s); the first may be cut by the start.
        let mut spells = Vec::new();
        let mut run = 0.0;
        for _ in 0..(90.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let (hero, envelope) = g.hero();
            let axis = envelope * hero_profile((-hero.0, -hero.1), g.cos_inc, g.pa, 0.0);
            assert_eq!(g.caption().is_some(), axis >= RING_LIGHT);
            if g.caption().is_none() {
                if run > 0.0 {
                    spells.push(run);
                }
                run = 0.0;
                off += 1;
                continue;
            }
            run += SIM_DT;
            on += 1;
            // The whole critical curve images the axis: the ring is closed.
            for k in 0..16 {
                let a = TAU * f64::from(k) / 16.0;
                let (x, y) = (c.0 + te * a.cos_det(), c.1 + te * a.sin_det());
                let p = g.pixels.get(x as i32, y as i32).unwrap();
                assert_eq!(p.role, Role::Ramp(Ramp::Ice), "gap at {a}, t={}", g.t);
            }
        }
        assert!(on > 0 && off > 0, "{on} / {off}");
        assert!(spells.len() >= 5, "{spells:?}");
        // Every pass closes the ring, and it holds long enough to read: over many
        // seeded passes, the longest closed spell of each is at least 2 s.
        let mut ctx = Ctx::new(17, Size::new(1, 1));
        for pass in 0..400 {
            g.new_pass(&mut ctx);
            let (mut best, mut run) = (0.0f64, 0.0);
            for k in 0..(PASS / SIM_DT) as usize {
                g.pass_t = k as f64 * SIM_DT;
                run = if g.caption().is_some() {
                    run + SIM_DT
                } else {
                    0.0
                };
                best = best.max(run);
            }
            assert!(best >= 2.0, "pass {pass}: ring closed for {best} s");
        }
    }

    #[test]
    fn lens_a8_the_lens_glow_fades_out_through_a_stipple() {
        let size = Size::new(200, 50);
        let mut g = scene(size);
        g.field.fill(0.0);
        g.pass_t = 0.0; // no hero
        g.raster();
        let (c, te) = (g.centre, g.theta_e_px);
        // Share of lens-lit pixels in thin annuli: from solid, through a thinning
        // stipple, to none, without a jump from solid to dark.
        let share = |r0: f64| {
            let (mut lit, mut all) = (0, 0);
            let px = g.pixels.pixel_size();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let (dx, dy) = (f64::from(x) + 0.5 - c.0, f64::from(y) + 0.5 - c.1);
                    let r = dx.hypot(dy) / te;
                    if (r0..r0 + 0.025).contains(&r) {
                        all += 1;
                        lit += usize::from(
                            g.pixels.get(x, y).unwrap().role == Role::Ramp(Ramp::Ember),
                        );
                    }
                }
            }
            lit as f64 / f64::from(all.max(1))
        };
        // Annuli 0.025 θE wide from 0.2 θE: a solid core, a halftone fade, then a dark
        // gap from 0.65 θE to the critical curve that the Einstein ring owns.
        let shares: Vec<f64> = (0..30).map(|k| share(0.2 + 0.025 * f64::from(k))).collect();
        assert!(shares[0] > 0.99, "{shares:?}");
        assert!(shares[18..].iter().all(|&s| s == 0.0), "no gap: {shares:?}");
        let between = shares.iter().filter(|&&s| (0.1..0.9).contains(&s)).count();
        assert!(between >= 3, "hard edge: {shares:?}");
        for w in shares.windows(2) {
            assert!(w[1] <= w[0] + 0.1, "{shares:?}");
        }
        // The fade is an ordered halftone, not grain: every 4 × 4 block of a uniform
        // light `k/16` of the faintest level keeps exactly `k` dots.
        for k in 0..=16 {
            for (ox, oy) in [(0, 0), (4, 8), (13, 2)] {
                let v = (f64::from(k) - 0.25).max(0.0) / 16.0;
                let kept = (0..16)
                    .filter(|&n| halftone(v, 1.0, (ox + n % 4, oy + n / 4)) > 0.0)
                    .count();
                assert_eq!(kept, k as usize, "k={k} at {ox},{oy}");
            }
        }
    }

    #[test]
    fn lens_a7_the_field_is_a_quiet_ground_below_the_hero() {
        for seed in [1, 9, 23] {
            let size = Size::new(120, 40);
            let mut g = GravitationalLensing::default();
            g.resize(size, &mut Ctx::new(seed, size));
            let brightest = g.field.as_slice().iter().fold(0.0f32, |m, &v| m.max(v));
            assert!(
                f64::from(brightest) <= 0.75,
                "seed {seed}: field peak {brightest}"
            );
            let px = g.pixels.pixel_size();
            let dark = (0..i32::from(px.height()))
                .flat_map(|y| (0..i32::from(px.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| g.pixels.get(x, y).unwrap().role == Role::Background)
                .count();
            assert!(
                10 * dark >= 6 * px.area(),
                "seed {seed}: {dark} dark pixels"
            );
        }
    }
}
