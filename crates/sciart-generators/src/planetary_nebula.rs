//! A planetary nebula like the Cat's Eye or NGC 6302 (SPEC-002 `planetary_nebula`).
//!
//! Act 1: the dying giant's slow, dusty wind drifts out, densest round its equator.
//! Act 2: the exposed hot core's fast wind inflates two lobes, an hourglass, sweeping the
//! slow wind up. Act 3: the core, now ~100 kK, ionises the lobes: [O III] inside, [N II]
//! at the tips and in the knots beyond them.
//!
//! M16 (GEN-R25, thread style): the lobes are the fast wind's streamlines, deflected toward
//! the axis by the dense equatorial gas, drawn as braille threads whose dashes flow
//! outward; colour along a thread is its ionisation. No shells, rings, fills or haze.
//! Lengths are in units of the half-height `H` (dots on screen).

use std::cell::RefCell;
use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "planetary_nebula",
    category: Category::DeepCosmos,
    description: "A dying star's slow wind and the bipolar lobes its fast wind blows through it",
    title: "Planetary nebula · winds collide",
    captions: &[SLOW_WIND, FAST_WIND, IONISED],
};

const SLOW_WIND: &str = "slow wind";
const FAST_WIND: &str = "fast wind";
const IONISED: &str = "ionised";

pub fn build() -> Box<dyn Generator> {
    Box::new(PlanetaryNebula::default())
}

/// Acts (s) and their indices.
const ACTS: [(&str, f64); 4] = [
    (SLOW_WIND, 6.0),
    (FAST_WIND, 8.0),
    (IONISED, 26.0),
    ("fade", 5.0),
];
const ACT_WIND: usize = 1;
const ACT_IONISED: usize = 2;
const ACT_FADE: usize = 3;
const T_WIND: f64 = 6.0;
const T_IONISED: f64 = 14.0;
const T_FADE: f64 = 40.0;
const LOOP: f64 = 45.0;
/// Slow wind (PNE-A1): grains, their speed (H/s, ±20 %), the radius they coast to, the
/// share launched near the equator and that spread (rad), and where they thin out.
pub(crate) const DUST: usize = 360;
pub(crate) const V_DUST: f64 = 0.045;
const R_DUST: f64 = 0.95;
const TORUS_SHARE: f64 = 0.7;
const TORUS_SPREAD: f64 = 0.35;
const DUST_THIN: (f64, f64) = (0.6, 0.35);
/// Lobes (PNE-A2): full length (H), the quick inflation and the slow creep after it
/// (share of the full length, over s).
const L_FULL: f64 = 0.85;
const LOBE_GROW: f64 = 7.0;
const LOBE_CREEP: (f64, f64) = (0.38, 32.0);
/// Streamlines per lobe and side as `(p, F)`: the curve `r = F·L·|cos θ|^p`; the first
/// is the lobe wall.
const PETALS: [(f64, f64); 5] = [
    (2.7, 1.0),
    (3.3, 0.93),
    (4.2, 0.86),
    (5.5, 0.79),
    (7.5, 0.72),
];
pub(crate) const WALL_POW: f64 = PETALS[0].0;
/// The Cat's Eye's second, tilted pair of lobes: size, tilt (rad), streamlines.
const INNER_SCALE: f64 = 0.55;
const INNER_TILT: f64 = 0.35;
const INNER_PETALS: [(f64, f64); 3] = [(5.0, 1.0), (7.0, 0.85), (10.0, 0.7)];
/// Streamlines start this far from the star (lobe lengths): the free-streaming wind next
/// to the star is too thin to show, and the threads would crowd there.
const R0: f64 = 0.12;
/// The flowing dashes (PNE-A7): period (H), duty, speed (H/s).
const DASH: f64 = 0.08;
const DUTY: f64 = 0.75;
pub(crate) const V_FLOW: f64 = 0.06;
/// Ionisation front: sweeps out to this multiple of the lobe radius over the act.
const ION_REACH: f64 = 1.25;
/// [N II] on the lobe wall beyond this share of the lobe length, once the rim glow is
/// this strong.
const NII_FROM: f64 = 0.82;
const NII_GLOW: f64 = 0.5;
/// Low-ionisation knots (FLIERs) just beyond the tips: distance (lobe lengths), drift
/// (lobe lengths per s), trail (dots).
const FLIER_AT: f64 = 1.12;
const FLIER_DRIFT: f64 = 0.004;
const FLIER_TRAIL: f64 = 3.0;
/// Symmetry axis on the sky (radians), its precession and slow turn (rad/s).
const AXIS: f64 = 1.0;
const PRECESS: f64 = 0.12;
const TURN: f64 = 0.012;

/// Inks (M16): ionisation along the threads.
const OIII: Ink = Ink::new(Role::Wave, 200);
const OIII_INNER: Ink = Ink::new(Role::Particle, 160);
const NEUTRAL: Ink = Ink::new(Role::Cold, 150);
const NII: Ink = Ink::new(Role::Hot, 190);
const FLIER: Ink = Ink::new(Role::Hot, 220);
const DUST_INK: Ink = Ink::new(Role::Hot, 120);
const STAR: Ink = Ink::new(Role::Star, 255);

/// Radius of the lobe wall of length `l` at the cosine `c` of the angle from the axis:
/// a teardrop from the star (pinched to it at the waist) to the tip at `l` (PNE-A2).
pub(crate) fn lobe_radius(l: f64, c: f64) -> f64 {
    l * c.abs().powf(WALL_POW)
}

/// The streamline `(p, f)` of a lobe of length `l` at `v` ∈ [0, 1]: `(r, θ)` with θ
/// from 90° (at the star, `v` = 0) to 0 (the tip, at `f·l`), `r = f·l·cos^p θ`. Leaving
/// the star sideways and turning to the axis, it is the wind deflected poleward by the
/// dense equatorial gas; with `p ≥ WALL_POW` and `f ≤ 1` it stays inside the wall.
pub(crate) fn petal(p: f64, f: f64, l: f64, v: f64) -> (f64, f64) {
    let th = std::f64::consts::FRAC_PI_2 * (1.0 - v);
    (f * l * th.cos_det().max(0.0).powf(p), th)
}

/// Whether the flowing dash pattern is on at flow coordinate `s` (H) at time `t`
/// (PNE-A7): it moves outward at `V_FLOW`.
pub(crate) fn dash_on(s: f64, t: f64, seed: f64) -> bool {
    ((s - V_FLOW * t) / DASH + seed).rem_euclid(1.0) < DUTY
}

/// A grain of the slow wind: launch angle from the axis, speed (H/s), phase (s).
#[derive(Debug, Clone, Copy)]
struct Grain {
    ang: f64,
    v: f64,
    phase: f64,
}

impl Grain {
    /// Distance from the star (H) at time `t`: coasting out to `R_DUST`, then relaunched.
    fn r(&self, t: f64) -> f64 {
        let period = R_DUST / self.v;
        self.v * (t + self.phase).rem_euclid(period)
    }
}

/// A thread's kind: an outer lobe's streamline (wall or not) or an inner lobe's.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Outer { wall: bool },
    Inner,
}

#[derive(Debug, Default)]
pub struct PlanetaryNebula {
    seeded: bool,
    t: f64,
    timeline: Option<Timeline>,
    grains: Vec<Grain>,
    /// Centre and half-height, in dots.
    centre: (f64, f64),
    h: f64,
    stars: Vec<(i32, i32, Ink)>,
    /// The braille lattice; a cell so `prepare` can draw into it while reading the
    /// rest of `self` (no placeholder lattice allocated per frame, GEN-R11).
    dots: RefCell<Dots>,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl PlanetaryNebula {
    fn beat(&self) -> Beat {
        match &self.timeline {
            Some(tl) => tl.at(self.t),
            None => Beat {
                index: 0,
                label: SLOW_WIND,
                local: 0.0,
                progress: 0.0,
            },
        }
    }

    /// Loop time in `[0, LOOP)`.
    fn tau(&self) -> f64 {
        self.t.rem_euclid(LOOP)
    }

    /// Lobe length now (H): 0 before the fast wind.
    fn lobe(&self) -> f64 {
        let t = self.tau() - T_WIND;
        if t <= 0.0 {
            0.0
        } else {
            let creep = (t / LOBE_CREEP.1).min(1.0);
            L_FULL * ((1.0 - LOBE_CREEP.0) * ease::out_quad(t / LOBE_GROW) + LOBE_CREEP.0 * creep)
        }
    }

    /// Symmetry axis angle now (rad, counter-clockwise from +x).
    fn axis_angle(&self) -> f64 {
        AXIS + TURN * self.t + PRECESS * (TAU * self.t / 50.0).sin_det()
    }

    /// Ionisation: how far out (in `q`) the front has swept, and the [N II] glow.
    fn ionisation(&self) -> (f64, f64) {
        let tau = self.tau();
        let front = ION_REACH * ease::smoothstep((tau - T_IONISED + 0.5) / 4.0);
        let rim = ease::smoothstep((tau - T_WIND - 2.0) / 5.0);
        (front, rim)
    }

    fn envelope(&self) -> f64 {
        let tau = self.tau();
        ease::smoothstep(tau / 1.5) * (1.0 - ease::smoothstep((tau - T_FADE) / 4.5))
    }

    /// Dot position of the point at `(r, θ)` (H, rad from the axis) of lobe `lobe` (±1)
    /// and side `side` (±1) about the axis at angle `axis`.
    fn at(&self, axis: (f64, f64), r: f64, th: f64, lobe: f64, side: f64) -> (f64, f64) {
        let (s, c) = th.sin_cos_det();
        let (a, b) = (lobe * r * c, side * r * s);
        // Sky offset (y up): a along the axis, b across it.
        let x = a * axis.0 - b * axis.1;
        let y = a * axis.1 + b * axis.0;
        (self.centre.0 + x * self.h, self.centre.1 - y * self.h)
    }

    /// Every streamline segment `(from, to, ink, kind)` in dots: the flowing dashes of
    /// both lobes' petals (and the tilted inner pair), coloured by ionisation.
    fn threads(&self, mut f: impl FnMut((f64, f64), (f64, f64), Ink, Kind)) {
        let l = self.lobe();
        if l <= 0.0 || self.h <= 0.0 {
            return;
        }
        let (front, rim) = self.ionisation();
        let env = self.envelope();
        let a = self.axis_angle();
        let outer_axis = a.sin_cos_det();
        let inner_axis = (a + INNER_TILT).sin_cos_det();
        let t = self.t;
        let r0 = R0 * l;
        // `wall_pow`: the family's wall, for the normalised radius `q = r / R(θ)`.
        let mut draw = |axis: (f64, f64),
                        lk: f64,
                        (p, reach): (f64, f64),
                        wall_pow: f64,
                        kind: Kind,
                        id: i32| {
            // The fade drops whole threads by a fixed hash.
            if hash01(cell_key(id, 41)) >= env {
                return;
            }
            let seed = hash01(cell_key(id, 43));
            let n = (reach * lk * self.h * 0.8).ceil().max(4.0) as usize;
            for (lobe, side) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let point = |v: f64| {
                    let (r, th) = petal(p, reach, lk, v);
                    (r, th, self.at((axis.1, axis.0), r, th, lobe, side))
                };
                let mut prev = point(0.0);
                for k in 1..=n {
                    let next = point(k as f64 / n as f64);
                    let (r, th) = (0.5 * (prev.0 + next.0), 0.5 * (prev.1 + next.1));
                    // The flow coordinate is the distance from the star.
                    if prev.0 >= r0 && dash_on(r, t, seed) {
                        let c = th.cos_det().max(1e-9);
                        let q = reach * c.powf(p - wall_pow);
                        let ink = match kind {
                            _ if q >= front => NEUTRAL,
                            Kind::Outer { wall: true }
                                if r > NII_FROM * l && rim >= NII_GLOW && front > 0.0 =>
                            {
                                NII
                            }
                            Kind::Outer { .. } => OIII,
                            Kind::Inner => OIII_INNER,
                        };
                        f(prev.2, next.2, ink, kind);
                    }
                    prev = next;
                }
            }
        };
        let mut id = 0_i32;
        for (k, &petal) in PETALS.iter().enumerate() {
            draw(
                outer_axis,
                l,
                petal,
                WALL_POW,
                Kind::Outer { wall: k == 0 },
                id,
            );
            id += 1;
        }
        for &petal in &INNER_PETALS {
            draw(
                inner_axis,
                INNER_SCALE * l,
                petal,
                INNER_PETALS[0].0,
                Kind::Inner,
                id,
            );
            id += 1;
        }
    }

    /// The FLIERs: `(tail, head)` in dots, beyond both tips, once the core is hot.
    fn fliers(&self, mut f: impl FnMut((f64, f64), (f64, f64))) {
        let l = self.lobe();
        let (front, rim) = self.ionisation();
        if l <= 0.0 || rim < NII_GLOW || front <= 0.0 || self.envelope() < 0.5 {
            return;
        }
        let axis = self.axis_angle().sin_cos_det();
        let drift = FLIER_DRIFT * (self.tau() - T_WIND).max(0.0);
        let r = l * (FLIER_AT + drift);
        let back = FLIER_TRAIL / self.h.max(1e-9);
        for lobe in [1.0, -1.0] {
            let head = self.at((axis.1, axis.0), r, 0.0, lobe, 1.0);
            let tail = self.at((axis.1, axis.0), r - back, 0.0, lobe, 1.0);
            f(tail, head);
        }
    }

    /// Every grain drawn now, as a dot position: not inside the lobe wall (swept up by
    /// the fast wind), thinning out past `DUST_THIN.0`.
    fn grains(&self, mut f: impl FnMut((f64, f64))) {
        let (l, env, axis) = (self.lobe(), self.envelope(), self.axis_angle());
        for (i, g) in self.grains.iter().enumerate() {
            let r = g.r(self.t);
            let keep = 1.0 - ease::smoothstep((r - DUST_THIN.0) / DUST_THIN.1);
            if hash01(cell_key(i as i32, 47)) >= keep * env {
                continue;
            }
            if l > 0.0 && r <= lobe_radius(l, g.ang.cos_det()) * 1.02 {
                continue;
            }
            let (s, c) = (axis + g.ang).sin_cos_det();
            f((
                self.centre.0 + r * c * self.h,
                self.centre.1 - r * s * self.h,
            ));
        }
    }

    fn raster(&mut self) {
        let mut dots = self.dots.borrow_mut();
        dots.clear();
        for &(x, y, ink) in &self.stars {
            dots.set(x, y, ink);
        }
        self.grains(|p| dots.plot(p.0, p.1, DUST_INK));
        self.threads(|a, b, ink, _| dots.line(a.0, a.1, b.0, b.1, ink));
        self.fliers(|tail, head| {
            dots.line(tail.0, tail.1, head.0, head.1, NII);
            dots.disc(head.0, head.1, 1.0, FLIER);
        });
        dots.set(
            self.centre.0.round() as i32,
            self.centre.1.round() as i32,
            STAR,
        );
        drop(dots);
    }
}

impl Generator for PlanetaryNebula {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seeded = true;
            self.timeline = Some(Timeline::new(&ACTS));
            self.t = ctx.rng.random_range(0.0..3.0);
            self.grains = (0..DUST)
                .map(|_| {
                    let ang = if ctx.rng.random_bool(TORUS_SHARE) {
                        // Near the equator, either side: a Gaussian by Box–Muller.
                        let (u1, u2): (f64, f64) = (
                            ctx.rng.random_range(1e-9..1.0),
                            ctx.rng.random_range(0.0..1.0),
                        );
                        let z = (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos_det();
                        let side = if ctx.rng.random_bool(0.5) { 0.5 } else { -0.5 };
                        side * PI + TORUS_SPREAD * z
                    } else {
                        ctx.rng.random_range(0.0..TAU)
                    };
                    let v = V_DUST * ctx.rng.random_range(0.8..1.2);
                    Grain {
                        ang,
                        v,
                        phase: ctx.rng.random_range(0.0..R_DUST / v),
                    }
                })
                .collect();
        }
        self.dots.get_mut().resize(size);
        let ds = self.dots.get_mut().dot_size();
        let (w, hh) = (f64::from(ds.width()), f64::from(ds.height()));
        self.centre = (w / 2.0, hh / 2.0);
        self.h = (hh / 2.0).min(w / 2.6);
        self.stars.clear();
        for k in 0..(size.area() / 200) as i32 {
            let x = (hash01(cell_key(k, 13)) * w) as i32;
            let y = (hash01(cell_key(k, 29)) * hh) as i32;
            let l = if hash01(cell_key(k, 31)) > 0.8 {
                120
            } else {
                40
            };
            self.stars.push((x, y, Ink::new(Role::Star, l)));
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.borrow().blit(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        Some(match self.beat().index {
            0 => SLOW_WIND,
            ACT_WIND => FAST_WIND,
            ACT_IONISED | ACT_FADE => IONISED,
            _ => IONISED,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (PlanetaryNebula, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = PlanetaryNebula::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    fn at_time(g: &mut PlanetaryNebula, ctx: &mut Ctx, t: f64) {
        g.t = t;
        g.update(0.0, ctx);
        g.prepare();
    }

    /// The thread dots, each with its ink and the direction of its segment.
    fn thread_dots(g: &PlanetaryNebula) -> Vec<(i32, i32, Ink, (f64, f64))> {
        let mut out = Vec::new();
        g.threads(|a, b, ink, _| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let n = dx.hypot(dy).max(1e-9);
            for (x, y) in sciart_core::geom::line(
                a.0.round() as i32,
                a.1.round() as i32,
                b.0.round() as i32,
                b.1.round() as i32,
            ) {
                out.push((x, y, ink, (dx / n, dy / n)));
            }
        });
        out
    }

    /// Normalised outer-lobe radius `q` and the dot distance to the wall of a point (dots).
    fn q_of(g: &PlanetaryNebula, (x, y): (f64, f64)) -> (f64, f64) {
        let (dx, dy) = ((x - g.centre.0) / g.h, (g.centre.1 - y) / g.h);
        let r = dx.hypot(dy);
        let (s, c) = g.axis_angle().sin_cos_det();
        let cos = if r > 0.0 { (dx * c + dy * s) / r } else { 1.0 };
        let wall = lobe_radius(g.lobe(), cos);
        (r / wall, (r - wall).abs() * g.h)
    }

    #[test]
    fn pne_a1_dust_coasts_out_radially_densest_round_the_equator() {
        let (g, _) = built(Size::new(120, 40));
        let near = g
            .grains
            .iter()
            .filter(|gr| gr.ang.cos_det().abs() < 0.5)
            .count();
        assert!(
            near as f64 >= 0.55 * DUST as f64,
            "{near} of {DUST} near the equator"
        );
        for gr in &g.grains {
            assert!((0.8 * V_DUST..=1.2 * V_DUST).contains(&gr.v));
            let (a, b) = (gr.r(10.0), gr.r(10.5));
            // Out at its speed, unless it was just relaunched.
            if b > a {
                assert!((b - a - 0.5 * gr.v).abs() < 1e-9);
            } else {
                assert!(b < 0.5 * gr.v + 1e-9);
            }
        }
    }

    #[test]
    fn pne_a2_the_lobes_are_an_hourglass_and_hold_their_streamlines() {
        // Length along the axis over the full width across it (both sides).
        let width = (0..=900)
            .map(|i| {
                let th = f64::from(i) / 10.0 * PI / 180.0;
                lobe_radius(1.0, th.cos_det()) * th.sin_det()
            })
            .fold(0.0, f64::max);
        let ratio = lobe_radius(1.0, 1.0) / (2.0 * width);
        assert!(ratio >= 1.4, "{ratio}");
        for (family, wall) in [
            (&PETALS[..], WALL_POW),
            (&INNER_PETALS[..], INNER_PETALS[0].0),
        ] {
            for &(p, f) in family {
                for i in 0..=100 {
                    let (r, th) = petal(p, f, 0.8, f64::from(i) / 100.0);
                    let c = th.cos_det().max(0.0);
                    assert!(r <= 0.8 * c.powf(wall) + 1e-12, "p {p} at {th}");
                }
            }
        }
    }

    #[test]
    fn pne_a3_oiii_inside_and_nii_at_the_tips() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        at_time(&mut g, &mut ctx, 30.0);
        let (mut oiii, mut nii, mut near) = (0, 0, 0);
        let mut fl = Vec::new();
        g.fliers(|_, head| fl.push(head));
        assert_eq!(fl.len(), 2, "FLIERs drawn");
        g.threads(|a, b, ink, _| {
            for m in [a, b] {
                let (q, d) = q_of(&g, m);
                if matches!(ink.role, Role::Wave | Role::Particle) {
                    oiii += 1;
                    assert!(q < 1.1, "[O III] outside the lobes at q = {q}");
                }
                if ink == NII {
                    nii += 1;
                    let to_flier = fl
                        .iter()
                        .map(|p| (m.0 - p.0).hypot(m.1 - p.1))
                        .fold(f64::MAX, f64::min);
                    if d <= 3.0 || to_flier <= 3.0 {
                        near += 1;
                    }
                }
            }
        });
        assert!(oiii > 20 && nii > 5, "[O III] {oiii}, [N II] {nii}");
        assert!(near * 10 >= nii * 8, "{near} of {nii}");
    }

    #[test]
    fn pne_a4_the_lobes_are_hollow_threads() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let (mut thin, mut total) = (0usize, 0usize);
        for t in [10.0, 20.0, 30.0] {
            at_time(&mut g, &mut ctx, t);
            let mut dots = Dots::new(Size::new(200, 50));
            g.threads(|a, b, ink, _| dots.line(a.0, a.1, b.0, b.1, ink));
            for (x, y, _, (dx, dy)) in thread_dots(&g) {
                let across = (-2..=2)
                    .filter(|&k| {
                        let k = f64::from(k);
                        dots.is_lit(
                            (f64::from(x) - k * dy).round() as i32,
                            (f64::from(y) + k * dx).round() as i32,
                        )
                    })
                    .count();
                thin += usize::from(across <= 3);
                total += 1;
            }
        }
        let share = thin as f64 / total as f64;
        assert!(total > 500 && share >= 0.85, "thin {share:.3} of {total}");
    }

    #[test]
    fn pne_a5_captions_follow_the_acts() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        g.t = 0.0;
        let want = |tau: f64| {
            if tau < T_WIND {
                SLOW_WIND
            } else if tau < T_IONISED {
                FAST_WIND
            } else {
                IONISED
            }
        };
        for _ in 0..(55.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let tau = g.tau();
            // Skip the instants right at a boundary.
            if [T_WIND, T_IONISED, LOOP]
                .iter()
                .any(|b| (tau - b).abs() < 0.05)
                || tau < 0.05
            {
                continue;
            }
            assert_eq!(g.caption(), Some(want(tau)), "τ = {tau}");
        }
    }

    #[test]
    fn pne_a6_braille_threads_one_star_and_no_dust_in_the_lobes() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        for t in [3.0, 8.0, 20.0, 30.0] {
            at_time(&mut g, &mut ctx, t);
            canvas.clear();
            g.render(&mut canvas);
            let mut star = 0;
            for y in 0..50 {
                for x in 0..200 {
                    let c = canvas.get(x, y).unwrap();
                    if c.ch == ' ' {
                        continue;
                    }
                    assert_eq!(c.bg, Ink::BLANK, "t {t}: half block at {x},{y}");
                    assert!(
                        ('\u{2800}'..='\u{28ff}').contains(&c.ch),
                        "t {t}: {:?}",
                        c.ch
                    );
                    assert!(
                        matches!(
                            c.ink.role,
                            Role::Hot | Role::Star | Role::Wave | Role::Particle | Role::Cold
                        ),
                        "t {t}: {:?} at ({x}, {y})",
                        c.ink
                    );
                    star += usize::from(c.ink == STAR);
                }
            }
            assert_eq!(star, 1, "t {t}: star cells");
            if g.lobe() > 0.0 {
                g.grains(|p| {
                    let (q, _) = q_of(&g, p);
                    assert!(q > 0.95, "t {t}: dust inside the lobes at q = {q}");
                });
            }
        }
    }

    #[test]
    fn pne_a7_the_flow_moves_outward() {
        for i in 0..400 {
            let s = f64::from(i) * 0.0037;
            for t in [0.0, 3.3, 17.0] {
                for dt in [0.1, 0.7, 2.0] {
                    assert_eq!(
                        dash_on(s, t, 0.3),
                        dash_on(s + V_FLOW * dt, t + dt, 0.3),
                        "s {s} t {t} dt {dt}"
                    );
                }
            }
        }
        // And the pattern does move: some dashes change within a second.
        let changed = (0..400)
            .filter(|&i| {
                let s = f64::from(i) * 0.0037;
                dash_on(s, 0.0, 0.3) != dash_on(s, 1.0, 0.3)
            })
            .count();
        assert!(changed > 20, "{changed}");
    }
}
