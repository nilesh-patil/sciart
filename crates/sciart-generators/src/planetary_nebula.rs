//! A planetary nebula like the Cat's Eye (SPEC-002 `planetary_nebula`).
//!
//! Act 1: the dying giant puffs off evenly spaced spherical shells (the AGB rings).
//! Act 2: the exposed hot core's fast wind inflates a prolate bubble with a pinched
//! waist inside the slow wind. Act 3: the core, now ~100 kK, ionises it: [O III] inside,
//! [N II] at the rim and in the knots at the tips. Every shell is drawn as analytic
//! limb-brightened emission (projected path length through a shell); lengths are in
//! units of the half-height `H` (pixels on screen).

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, disc_pixels};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "planetary_nebula",
    category: Category::DeepCosmos,
    description: "A dying star's shells and the bipolar bubble its fast wind blows inside them",
    title: "Planetary nebula · winds collide",
    captions: &[SHELLS, FAST_WIND, IONISED],
};

const SHELLS: &str = "shells";
const FAST_WIND: &str = "fast wind";
const IONISED: &str = "ionised";

pub fn build() -> Box<dyn Generator> {
    Box::new(PlanetaryNebula::default())
}

/// Acts (s) and their indices.
const ACTS: [(&str, f64); 4] = [
    (SHELLS, 18.0),
    (FAST_WIND, 15.0),
    (IONISED, 12.0),
    ("fade", 5.0),
];
const ACT_WIND: usize = 1;
const ACT_IONISED: usize = 2;
const ACT_FADE: usize = 3;
const T_WIND: f64 = 18.0;
const T_IONISED: f64 = 33.0;
const T_FADE: f64 = 45.0;
/// AGB rings: launch interval (s), count, expansion speed (H/s), relative thickness.
pub(crate) const RING_EVERY: f64 = 2.2;
pub(crate) const RINGS: usize = 9;
pub(crate) const V_SHELL: f64 = 0.042;
const RING_THICK: f64 = 0.9;
/// Fast-wind bubble: equatorial radius at full size (H), growth time (s), prolateness
/// `R(θ) = R_b (1 + P cos² θ)`, shell thickness (fraction of the radius).
const BUBBLE_R: f64 = 0.36;
const BUBBLE_GROW: f64 = 13.0;
pub(crate) const PROLATE: f64 = 0.6;
const BUBBLE_THICK: f64 = 0.2;
/// The Cat's Eye's second, tilted inner bubble: size and tilt relative to the first.
const INNER_SCALE: f64 = 0.62;
const INNER_TILT: f64 = 0.75;
/// Ionisation front: sweeps out to this multiple of the bubble radius over the act.
const ION_REACH: f64 = 1.25;
/// Low-ionisation knots (FLIERs) just outside the tips: distance (bubble radii), size.
const FLIER_AT: f64 = 1.12;
const FLIER_R: f64 = 0.05;
/// [N II] rim band in normalised radius.
const RIM_BAND: (f64, f64) = (0.9, 1.04);
/// Symmetry axis on the sky (radians) and its precession.
const AXIS: f64 = 1.0;
const PRECESS: f64 = 0.12;
/// Ink caps.
const RING_MAX: f32 = 60.0;
const NII_MAX: u8 = 205;

/// Radius of AGB ring `k` at loop time `t` (H), or `None` before it is launched.
pub(crate) fn ring_radius(k: usize, t: f64) -> Option<f64> {
    let t0 = k as f64 * RING_EVERY;
    (t >= t0).then_some(V_SHELL * (t - t0))
}

/// Bubble radius along direction `cos_axis` (the cosine of the angle from the symmetry
/// axis), for equatorial radius `r_b` (PNE-A2).
pub(crate) fn bubble_radius(r_b: f64, cos_axis: f64) -> f64 {
    r_b * (1.0 + PROLATE * cos_axis * cos_axis)
}

/// Projected path through a shell `inner < outer` at impact parameter `rho`.
fn shell_path(rho: f64, inner: f64, outer: f64) -> f64 {
    if rho >= outer {
        return 0.0;
    }
    let o = (outer * outer - rho * rho).sqrt();
    let i = if rho < inner {
        (inner * inner - rho * rho).sqrt()
    } else {
        0.0
    };
    o - i
}

/// Limb-brightened emission of a shell in normalised radius `q` (1 at its outer edge),
/// scaled to 1 at the inner edge where it peaks.
fn shell_profile(q: f64, thick: f64) -> f64 {
    let inner = 1.0 - thick;
    shell_path(q, inner, 1.0) / (1.0 - inner * inner).sqrt()
}

#[derive(Debug, Default)]
pub struct PlanetaryNebula {
    seeded: bool,
    t: f64,
    timeline: Option<Timeline>,
    centre: (f64, f64),
    h: f64,
    tone: Tone,
    pixels: Pixels,
    stars: Vec<(i32, i32, u8)>,
}

impl PlanetaryNebula {
    fn beat(&self) -> Beat {
        match &self.timeline {
            Some(tl) => tl.at(self.t),
            None => Beat {
                index: 0,
                label: SHELLS,
                local: 0.0,
                progress: 0.0,
            },
        }
    }

    /// Loop time in `[0, total)`.
    fn tau(&self) -> f64 {
        self.t.rem_euclid(T_FADE + 5.0)
    }

    /// Equatorial bubble radius now (H).
    fn bubble(&self) -> f64 {
        let t = self.tau() - T_WIND;
        if t <= 0.0 {
            0.0
        } else {
            BUBBLE_R * ease::out_quad(t / BUBBLE_GROW)
        }
    }

    /// Symmetry axis now: unit vector on screen (x right, y up).
    fn axis(&self) -> (f64, f64) {
        let a = AXIS + PRECESS * (std::f64::consts::TAU * self.t / 50.0).sin_det();
        let (s, c) = a.sin_cos_det();
        (c, s)
    }

    /// Normalised bubble radius `q` of sky offset `(x, y)` (H) for a bubble of
    /// equatorial radius `r_b` about `axis`, and the cosine to the axis.
    fn q(x: f64, y: f64, r_b: f64, axis: (f64, f64)) -> (f64, f64) {
        let rho = x.hypot(y);
        if rho == 0.0 || r_b <= 0.0 {
            return (if r_b > 0.0 { 0.0 } else { f64::INFINITY }, 1.0);
        }
        let c = (x * axis.0 + y * axis.1) / rho;
        (rho / bubble_radius(r_b, c), c)
    }

    /// Ionisation: how far out (in `q`) the front has swept, and the rim's [N II] glow.
    fn ionisation(&self) -> (f64, f64) {
        let tau = self.tau();
        let front = ION_REACH * ease::smoothstep((tau - T_IONISED + 1.0) / 8.0);
        let rim = ease::smoothstep((tau - T_WIND - 5.0) / 7.0);
        (front, rim)
    }

    fn envelope(&self) -> f64 {
        1.0 - ease::smoothstep((self.tau() - T_FADE) / 4.5)
    }

    /// Everything the per-pixel emission needs this frame.
    fn frame(&self) -> Frame {
        let (front, rim_glow) = self.ionisation();
        Frame {
            tau: self.tau(),
            r_b: self.bubble(),
            axis: self.axis(),
            front,
            rim_glow,
        }
    }
}

/// A snapshot of the nebula's state for shading one frame.
#[derive(Debug, Clone, Copy)]
struct Frame {
    tau: f64,
    r_b: f64,
    axis: (f64, f64),
    front: f64,
    rim_glow: f64,
}

impl Frame {
    /// `(O III, N II, ring)` emission at sky offset `(x, y)` (H units, y up); `px` is
    /// one pixel in H.
    fn emission(&self, x: f64, y: f64, px: f64) -> (f64, f64, f64) {
        let tau = self.tau;
        let rho = x.hypot(y);
        // AGB rings: thin shells, dimmer as they spread.
        let mut ring = 0.0;
        let thick = RING_THICK * px;
        for k in 0..RINGS {
            let Some(r) = ring_radius(k, tau) else { break };
            if r <= thick || rho >= r || rho < r - 3.0 * px {
                continue;
            }
            let p = shell_path(rho, r - thick, r) / (r * r - (r - thick) * (r - thick)).sqrt();
            ring += p
                * (0.35 + 0.65 * (-r / 0.9).exp())
                * (0.55 + 0.45 * hash01(cell_key(k as i32, 77)));
        }
        let r_b = self.r_b;
        if r_b <= 0.0 {
            return (0.0, 0.0, ring);
        }
        let (axis, front, rim_glow) = (self.axis, self.front, self.rim_glow);
        let (q, c) = PlanetaryNebula::q(x, y, r_b, axis);
        let (s, co) = (INNER_TILT.sin_det(), INNER_TILT.cos_det());
        let inner_axis = (axis.0 * co - axis.1 * s, axis.0 * s + axis.1 * co);
        let (q2, _) = PlanetaryNebula::q(x, y, r_b * INNER_SCALE, inner_axis);
        // [O III]: the ionised interior, brightest in the shells' limbs.
        let mut oiii = 0.0;
        if q < 1.0 && q < front {
            oiii = 0.22 + 0.9 * shell_profile(q, BUBBLE_THICK).min(1.6);
            oiii += 0.6 * shell_profile(q2, 0.25).min(1.6);
            // Soft edge on the sweeping front.
            oiii *= ease::smoothstep((front - q) / 0.12);
        }
        // [N II]: the rim band (it glows once the core is hot), and the FLIERs.
        let mut nii = 0.0;
        if (RIM_BAND.0..RIM_BAND.1).contains(&q) {
            let mid = 0.5 * (RIM_BAND.0 + RIM_BAND.1);
            let half = 0.5 * (RIM_BAND.1 - RIM_BAND.0);
            nii = rim_glow * (1.0 - ((q - mid) / half).powi(2)) * (0.5 + 0.5 * c * c);
        }
        let tip = bubble_radius(r_b, 1.0) * FLIER_AT;
        for sign in [1.0, -1.0] {
            let (fx, fy) = (sign * tip * axis.0, sign * tip * axis.1);
            let d = (x - fx).hypot(y - fy) / (FLIER_R + 0.6 * px);
            if d < 1.0 {
                nii = f64::max(nii, rim_glow * (1.0 - d * d) * 1.3);
            }
        }
        (oiii, nii, ring)
    }
}

impl PlanetaryNebula {
    fn raster(&mut self) {
        let env = self.envelope() as f32;
        let (cx, cy, h) = (self.centre.0, self.centre.1, self.h.max(1e-9));
        let px = 1.0 / h;
        let frame = self.frame();
        let tone = &self.tone;
        self.pixels.shade(|ix, iy| {
            let x = (f64::from(ix) + 0.5 - cx) / h;
            let y = (cy - f64::from(iy) - 0.5) / h;
            let (o, n, r) = frame.emission(x, y, px);
            let lo = tone.level(o as f32 * env);
            let ln = tone.level(n as f32 * env).min(NII_MAX);
            let lr = (r as f32 * env * RING_MAX).min(RING_MAX);
            if ln >= lo && f64::from(ln) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::HAlpha), ln)
            } else if f64::from(lo) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Aurora), lo)
            } else if f64::from(lr) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Starlight), lr as u8)
            } else {
                Ink::BLANK
            }
        });
        for &(x, y, l) in &self.stars {
            if self.pixels.get(x, y) == Some(Ink::BLANK) {
                self.pixels
                    .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
            }
        }
        self.star();
    }

    /// The central star: a cool giant in act 1, then the exposed core heating up.
    fn star(&mut self) {
        let tau = self.tau();
        let heat = ease::smoothstep((tau - T_WIND + 1.0) / 6.0);
        let kelvin = ease::lerp(3_200.0, 30_000.0, heat);
        let radius = ease::lerp(1.6, 0.8, heat);
        let env = self.envelope().max(0.35);
        let (pixels, c) = (&mut self.pixels, self.centre);
        disc_pixels(c, radius * env, |x, y, cover| {
            if cover > 0.3 {
                pixels.set(x, y, ink::blackbody(kelvin));
            }
        });
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
            self.tone = Tone::asinh(0.35, 2.2);
            self.t = ctx.rng.random_range(0.0..3.0);
        }
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, hh) = (f64::from(px.width()), f64::from(px.height()));
        self.centre = (w / 2.0, hh / 2.0);
        self.h = (hh / 2.0).min(w / 2.6);
        self.stars.clear();
        for k in 0..(size.area() / 60) as i32 {
            let x = (hash01(cell_key(k, 13)) * w) as i32;
            let y = (hash01(cell_key(k, 29)) * hh) as i32;
            let l = 20 + (hash01(cell_key(k, 31)).powi(4) * 140.0) as u8;
            self.stars.push((x, y, l));
        }
        self.raster();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }

    fn caption(&self) -> Option<&'static str> {
        Some(match self.beat().index {
            0 => SHELLS,
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
        (g, ctx)
    }

    #[test]
    fn pne_a1_rings_are_evenly_spaced_and_expand_at_v_shell() {
        let t = 30.0;
        let radii: Vec<f64> = (0..RINGS).filter_map(|k| ring_radius(k, t)).collect();
        assert_eq!(radii.len(), RINGS);
        let gaps: Vec<f64> = radii.windows(2).map(|w| w[0] - w[1]).collect();
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / gaps.len() as f64;
        assert!(var.sqrt() < 0.01 * mean);
        for k in 0..RINGS {
            let a = ring_radius(k, 40.0).unwrap();
            let b = ring_radius(k, 41.0).unwrap();
            assert!((b - a - V_SHELL).abs() < 1e-12);
        }
    }

    #[test]
    fn pne_a2_the_bubble_is_prolate() {
        let r = 0.3;
        let ratio = bubble_radius(r, 1.0) / bubble_radius(r, 0.0);
        assert!(ratio >= 1.4, "{ratio}");
    }

    /// Distance (pixels) from pixel `(ix, iy)` to the bubble's rim along its ray.
    fn rim_distance(g: &PlanetaryNebula, ix: i32, iy: i32) -> (f64, f64) {
        let x = (f64::from(ix) + 0.5 - g.centre.0) / g.h;
        let y = (g.centre.1 - f64::from(iy) - 0.5) / g.h;
        let (q, c) = PlanetaryNebula::q(x, y, g.bubble(), g.axis());
        let rim = bubble_radius(g.bubble(), c) * g.h;
        (q, (q - 1.0).abs() * rim)
    }

    #[test]
    fn pne_a3_oiii_inside_and_nii_on_the_rim_or_tips() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        g.t = 40.0;
        g.update(0.0, &mut ctx);
        let px = g.pixels.pixel_size();
        let (mut nii, mut near) = (0, 0);
        let tip = bubble_radius(g.bubble(), 1.0) * FLIER_AT * g.h;
        let axis = g.axis();
        for iy in 0..i32::from(px.height()) {
            for ix in 0..i32::from(px.width()) {
                let ink = g.pixels.get(ix, iy).unwrap();
                let (q, d) = rim_distance(&g, ix, iy);
                if ink.role == Role::Ramp(Ramp::Aurora) {
                    assert!(q < 1.0, "[O III] outside the bubble at q = {q}");
                }
                if ink.role == Role::Ramp(Ramp::HAlpha) {
                    nii += 1;
                    let (x, y) = (
                        f64::from(ix) + 0.5 - g.centre.0,
                        g.centre.1 - f64::from(iy) - 0.5,
                    );
                    let to_tip = (x - tip * axis.0)
                        .hypot(y - tip * axis.1)
                        .min((x + tip * axis.0).hypot(y + tip * axis.1));
                    if d <= 3.0 || to_tip <= 3.0 + FLIER_R * g.h {
                        near += 1;
                    }
                }
            }
        }
        assert!(nii > 20, "[N II] drawn: {nii}");
        assert!(f64::from(near) >= 0.8 * f64::from(nii), "{near} of {nii}");
    }

    #[test]
    fn pne_a4_a_shell_peaks_at_its_projected_edge() {
        let px = 1.0 / 40.0;
        let r = 0.6;
        let thick = RING_THICK * px;
        let mut best = (0.0, 0.0);
        for i in 0..4000 {
            let rho = r * f64::from(i) / 4000.0;
            let p = shell_path(rho, r - thick, r);
            if p > best.1 {
                best = (rho, p);
            }
        }
        assert!(
            (r - best.0) <= px,
            "peak {} px inside the edge",
            (r - best.0) / px
        );
    }

    #[test]
    fn pne_a5_captions_follow_the_acts() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        g.t = 0.0;
        let want = |tau: f64| {
            if tau < T_WIND {
                SHELLS
            } else if tau < T_IONISED {
                FAST_WIND
            } else {
                IONISED
            }
        };
        for _ in 0..(55.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let tau = g.tau();
            // Skip the instants right at a boundary.
            if [T_WIND, T_IONISED, 50.0]
                .iter()
                .any(|b| (tau - b).abs() < 0.05)
                || tau < 0.05
            {
                continue;
            }
            assert_eq!(g.caption(), Some(want(tau)), "τ = {tau}");
        }
    }
}
