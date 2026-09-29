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
use sciart_core::rng::{cell_key, hash01, value_noise};
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
    (SHELLS, 6.0),
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
/// Rings already puffed off when the loop starts, so the first frame shows shells.
const PRE_RINGS: f64 = 5.0;
/// AGB rings: launch interval (s), count, expansion speed (H/s), relative thickness.
pub(crate) const RING_EVERY: f64 = 2.2;
pub(crate) const RINGS: usize = 9;
pub(crate) const V_SHELL: f64 = 0.042;
const RING_THICK: f64 = 2.2;
/// Fast-wind bubble: equatorial radius at full size (H), growth time (s), prolateness
/// `R(θ) = R_b (1 + P cos² θ)`, shell thickness (fraction of the radius).
const BUBBLE_R: f64 = 0.36;
const BUBBLE_GROW: f64 = 7.0;
/// Slow further growth through the ionised act (share of the full radius, over s).
const BUBBLE_CREEP: (f64, f64) = (0.25, 32.0);
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
/// Slow turn of the axis on the sky (rad/s).
const TURN: f64 = 0.012;
/// FLIER drift outward (bubble radii per s) and flicker rate (rad/s).
const FLIER_DRIFT: f64 = 0.004;
const FLICKER: f64 = 3.1;
/// Ink caps.
const RING_MAX: f32 = 80.0;
/// Outer [O III] halo: reach (bubble radii) and brightness; the interior texture salt.
const HALO_REACH: f64 = 1.7;
const HALO_GLOW: f64 = 0.16;
/// AGB rings fade out from this radius (H) over this span, and wobble in radius by up
/// to this share with the clumpy wind.
const RING_FADE: (f64, f64) = (0.18, 0.6);
const RING_WOBBLE: f64 = 0.04;
/// Ring/haze levels from here up to the visibility threshold are dithered.
const EDGE_DITHER: f64 = 8.5;
/// Peak of the AGB wind haze (share of the ring ink cap).
const HAZE: f64 = 0.55;
const TEXTURE_SALT: u64 = 0x5eed_ca75;
const NII_MAX: u8 = 205;

/// The AGB wind's clumpy texture at sky offset `(x, y)` (H): a warp in `[−½, ½]` and a
/// brightness factor in `[0.55, 1.35]`. Fixed to the gas, so it is tabulated per pixel
/// on resize.
pub(crate) fn wind_texture(x: f64, y: f64) -> (f64, f64) {
    let warp = value_noise(TEXTURE_SALT ^ 0xa6b, x * 5.0, y * 5.0, 1.0) - 0.5;
    let clump = value_noise(
        TEXTURE_SALT ^ 0xc1,
        x * 8.0 + 2.0 * warp,
        y * 8.0 - 2.0 * warp,
        1.0,
    );
    (warp, 0.55 + 0.8 * clump * clump)
}

/// Radius of AGB ring `k` at loop time `t` (H), or `None` before it is launched.
pub(crate) fn ring_radius(k: usize, t: f64) -> Option<f64> {
    let t0 = (k as f64 - PRE_RINGS) * RING_EVERY;
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
    /// Per pixel: [`wind_texture`] quantised to bytes (warp, clumps).
    wind: Vec<(u8, u8)>,
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
        self.t.rem_euclid(LOOP)
    }

    /// Equatorial bubble radius now (H).
    fn bubble(&self) -> f64 {
        let t = self.tau() - T_WIND;
        if t <= 0.0 {
            0.0
        } else {
            let creep = (t / BUBBLE_CREEP.1).min(1.0);
            BUBBLE_R
                * ((1.0 - BUBBLE_CREEP.0) * ease::out_quad(t / BUBBLE_GROW)
                    + BUBBLE_CREEP.0 * creep)
        }
    }

    /// Symmetry axis now: unit vector on screen (x right, y up).
    fn axis(&self) -> (f64, f64) {
        let a = AXIS + TURN * self.t + PRECESS * (std::f64::consts::TAU * self.t / 50.0).sin_det();
        let (s, c) = a.sin_cos_det();
        (c, s)
    }

    /// Normalised bubble radius `q` of sky offset `(x, y)` (H) for a bubble of
    /// equatorial radius `r_b` about `axis`, and the cosine to the axis.
    #[cfg(test)]
    fn q(x: f64, y: f64, r_b: f64, axis: (f64, f64)) -> (f64, f64) {
        Self::q_at(x, y, x.hypot(y), r_b, axis)
    }

    /// [`Self::q`] with `rho = x.hypot(y)` already known (the per-pixel path).
    fn q_at(x: f64, y: f64, rho: f64, r_b: f64, axis: (f64, f64)) -> (f64, f64) {
        if rho == 0.0 || r_b <= 0.0 {
            return (if r_b > 0.0 { 0.0 } else { f64::INFINITY }, 1.0);
        }
        let c = (x * axis.0 + y * axis.1) / rho;
        (rho / bubble_radius(r_b, c), c)
    }

    /// Ionisation: how far out (in `q`) the front has swept, and the rim's [N II] glow.
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

    /// Everything the per-pixel emission needs this frame. Whatever does not depend on
    /// the pixel is worked out here once, not per pixel (GEN-R8 at 500×150: ~150 k pixels
    /// a frame); each value is computed exactly as the per-pixel code did, so frames are
    /// bit-identical.
    /// A radius (H) beyond which this frame's emission is exactly zero, so those pixels
    /// are blank without shading: past the last ring that can show, the visible haze
    /// (where [`Frame::emission`] would skip it), the [O III] halo (`q ≥ HALO_REACH` at the
    /// bubble's longest radius) and the FLIERs. A 1e-6 relative margin covers rounding
    /// (`hypot` vs `x² + y²`, and `cos² ≤ 1`), so the cut never changes a pixel.
    fn reach(r_b: f64, tip: f64, haze_on: f64, px: f64, rings: &[Option<RingNow>]) -> f64 {
        let thick = RING_THICK * px;
        let mut reach = rings
            .iter()
            .map_while(|g| *g)
            .filter(|g| g.r > thick && g.r < RING_FADE.0 + RING_FADE.1)
            .map(|g| g.r * (1.0 + RING_WOBBLE))
            .fold(0.0, f64::max);
        // Haze shows while HAZE·haze_on·e^(−ρ/0.45)·RING_MAX ≥ EDGE_DITHER / 1.35.
        let haze_peak = HAZE * haze_on * f64::from(RING_MAX) * 1.35 / EDGE_DITHER;
        if haze_peak > 1.0 {
            reach = reach.max(0.45 * haze_peak.ln());
        }
        if r_b > 0.0 {
            reach = reach
                .max(HALO_REACH * bubble_radius(r_b, 1.0))
                .max(tip + FLIER_R + 0.6 * px);
        }
        reach * (1.0 + 1e-6) + 1e-9
    }

    fn frame(&self, px: f64) -> Frame {
        let (front, rim_glow) = self.ionisation();
        let tau = self.tau();
        let (t, r_b, axis) = (self.t, self.bubble(), self.axis());
        let (s, co) = (INNER_TILT.sin_det(), INNER_TILT.cos_det());
        let mut rings = [None; RINGS];
        for (k, slot) in rings.iter_mut().enumerate() {
            *slot = ring_radius(k, tau).map(|r| RingNow {
                r,
                dim: (-r / 0.75).exp(),
                fade: 1.0 - ease::smoothstep((r - RING_FADE.0) / RING_FADE.1),
                tint: 0.55 + 0.45 * hash01(cell_key(k as i32, 77)),
            });
        }
        let drift = FLIER_DRIFT * (tau - T_WIND).max(0.0);
        let tip = bubble_radius(r_b, 1.0) * (FLIER_AT + drift);
        let flier = |sign: f64| Flier {
            at: (sign * tip * axis.0, sign * tip * axis.1),
            flicker: 0.8 + 0.2 * (FLICKER * t + 2.0 * sign).sin_det(),
        };
        let haze_on = 1.0 - ease::smoothstep((tau - T_WIND) / 4.0);
        Frame {
            cut2: Self::reach(r_b, tip, haze_on, px, &rings).powi(2),
            r_b,
            axis,
            inner_axis: (axis.0 * co - axis.1 * s, axis.0 * s + axis.1 * co),
            front,
            rim_glow,
            haze_on,
            rings,
            fliers: [flier(1.0), flier(-1.0)],
        }
    }
}

/// AGB ring `k` this frame: its radius and the per-ring factors of its emission.
#[derive(Debug, Clone, Copy)]
struct RingNow {
    r: f64,
    dim: f64,
    fade: f64,
    tint: f64,
}

/// A FLIER this frame: its centre (H) and flicker factor.
#[derive(Debug, Clone, Copy)]
struct Flier {
    at: (f64, f64),
    flicker: f64,
}

/// A snapshot of the nebula's state for shading one frame.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// Squared radius (H²) beyond which the frame is blank ([`PlanetaryNebula::reach`]).
    cut2: f64,
    r_b: f64,
    axis: (f64, f64),
    inner_axis: (f64, f64),
    front: f64,
    rim_glow: f64,
    haze_on: f64,
    /// Launched rings in launch order; `None` from the first not yet launched.
    rings: [Option<RingNow>; RINGS],
    fliers: [Flier; 2],
}

impl Frame {
    /// `(O III, N II, ring)` emission at sky offset `(x, y)` (H units, y up); `px` is
    /// one pixel in H.
    /// `tex(x, y)` gives the AGB wind's `(warp, clumps)` texture there ([`wind_texture`]).
    fn emission(&self, x: f64, y: f64, px: f64, tex: impl Fn() -> (f64, f64)) -> (f64, f64, f64) {
        let rho = x.hypot(y);
        // The AGB wind is clumpy: a domain-warped texture fixed to the gas, which also
        // nudges each ring's radius a little so the rings read as puffed-off shells, not
        // a target (rings stay evenly spaced on average, PNE-A1). The noise is only
        // sampled where a ring or the haze can show (it is the frame's main cost).
        let haze_on = self.haze_on;
        // Once the fast wind has cleared the haze, skip its exp (the product is 0 anyway).
        let haze = if haze_on == 0.0 {
            0.0
        } else {
            HAZE * haze_on * (-rho / 0.45).exp()
        };
        let thick = RING_THICK * px;
        let reach = |r: f64| {
            rho < r * (1.0 + RING_WOBBLE) && rho > (r - thick - 2.0 * px) * (1.0 - RING_WOBBLE)
        };
        let launched = || self.rings.iter().map_while(|ring| *ring);
        let near_ring =
            launched().any(|g| g.r > thick && g.r < RING_FADE.0 + RING_FADE.1 && reach(g.r));
        if !near_ring && haze * f64::from(RING_MAX) < EDGE_DITHER / 1.35 {
            return self.nebula(x, y, rho, px, 0.0);
        }
        let (warp, clumps) = tex();
        let rho_ring = rho * (1.0 + RING_WOBBLE * warp);
        // AGB rings: thin shells, dimmer as they spread, fading softly into the haze
        // well before the outermost one (no hard outer edge).
        let mut ring = 0.0;
        if near_ring {
            for g in launched() {
                let r = g.r;
                if r <= thick || rho_ring >= r || rho_ring < r - thick - 2.0 * px {
                    continue;
                }
                let p =
                    shell_path(rho_ring, r - thick, r) / (r * r - (r - thick) * (r - thick)).sqrt();
                ring += p * g.dim * g.fade * g.tint;
            }
        }
        // The giant's dusty wind: a faint haze thinning outward, gone once the core is
        // exposed and the fast wind clears it.
        self.nebula(x, y, rho, px, (ring + haze) * clumps)
    }

    /// `(O III, N II, ring)` for the bubble and its knots, given the AGB `ring` emission;
    /// `rho` is `x.hypot(y)`.
    fn nebula(&self, x: f64, y: f64, rho: f64, px: f64, ring: f64) -> (f64, f64, f64) {
        let r_b = self.r_b;
        if r_b <= 0.0 {
            return (0.0, 0.0, ring);
        }
        let (axis, front, rim_glow) = (self.axis, self.front, self.rim_glow);
        let (q, c) = PlanetaryNebula::q_at(x, y, rho, r_b, axis);
        let (q2, _) = PlanetaryNebula::q_at(x, y, rho, r_b * INNER_SCALE, self.inner_axis);
        // [O III]: the ionised interior, brightest in the shells' limbs.
        let mut oiii = 0.0;
        if q < 1.0 && q < front {
            oiii = 0.22 + 0.9 * shell_profile(q, BUBBLE_THICK).min(1.6);
            oiii += 0.6 * shell_profile(q2, 0.25).min(1.6);
            // Soft edge on the sweeping front.
            oiii *= ease::smoothstep((front - q) / 0.12);
            // Filamentary texture fixed to the nebula (axis-aligned coordinates).
            let (u, v) = (x * axis.0 + y * axis.1, -x * axis.1 + y * axis.0);
            let n = value_noise(TEXTURE_SALT, u * 14.0, v * 14.0, 1.0);
            oiii *= 0.8 + 0.35 * n;
        } else if q < HALO_REACH && front > 0.0 {
            // The faint ionised halo beyond the bubble.
            let fall = 1.0 - ease::smoothstep((q - 1.0) / (HALO_REACH - 1.0));
            oiii = HALO_GLOW * fall * (front / ION_REACH);
        }
        // [N II]: the rim band (it glows once the core is hot), and the FLIERs.
        let mut nii = 0.0;
        if (RIM_BAND.0..RIM_BAND.1).contains(&q) {
            let mid = 0.5 * (RIM_BAND.0 + RIM_BAND.1);
            let half = 0.5 * (RIM_BAND.1 - RIM_BAND.0);
            nii = rim_glow * (1.0 - ((q - mid) / half).powi(2)) * (0.5 + 0.5 * c * c);
        }
        for f in &self.fliers {
            let d = (x - f.at.0).hypot(y - f.at.1) / (FLIER_R + 0.6 * px);
            if d < 1.0 {
                nii = f64::max(nii, rim_glow * (1.0 - d * d) * 1.3 * f.flicker);
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
        let frame = self.frame(px);
        let (tone, wind) = (&self.tone, &self.wind);
        let width = usize::from(self.pixels.pixel_size().width());
        self.pixels.shade(|ix, iy| {
            let x = (f64::from(ix) + 0.5 - cx) / h;
            let y = (cy - f64::from(iy) - 0.5) / h;
            if x * x + y * y > frame.cut2 {
                return Ink::BLANK;
            }
            let i = iy as usize * width + ix as usize;
            let tex = || {
                wind.get(i).map_or_else(
                    || wind_texture(x, y),
                    |&(w, c)| {
                        (
                            f64::from(w) / 255.0 - 0.5,
                            0.55 + 0.8 * f64::from(c) / 255.0,
                        )
                    },
                )
            };
            let (o, n, r) = frame.emission(x, y, px, tex);
            let lo = tone.level(o as f32 * env);
            let ln = tone.level(n as f32 * env).min(NII_MAX);
            let lr = (r as f32 * env * RING_MAX).min(RING_MAX);
            if ln >= lo && f64::from(ln) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::HAlpha), ln)
            } else if f64::from(lo) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Aurora), lo)
            } else if f64::from(lr) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Ember), lr as u8)
            } else if f64::from(lr) >= EDGE_DITHER
                && hash01(cell_key(ix, iy))
                    < (f64::from(lr) - EDGE_DITHER) / (ink::MIN_VISIBLE_LEVEL - EDGE_DITHER)
            {
                // The faint outskirts thin out pixel by pixel (a fixed dither) instead of
                // stopping at the visibility threshold in one hard edge.
                Ink::new(Role::Ramp(Ramp::Ember), ink::MIN_VISIBLE_LEVEL as u8)
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
        self.wind.clear();
        let hu = self.h.max(1e-9);
        for iy in 0..px.height() {
            for ix in 0..px.width() {
                let x = (f64::from(ix) + 0.5 - self.centre.0) / hu;
                let y = (self.centre.1 - f64::from(iy) - 0.5) / hu;
                let (warp, clumps) = wind_texture(x, y);
                self.wind.push((
                    ink::level(255.0 * (warp + 0.5)),
                    ink::level(255.0 * (clumps - 0.55) / 0.8),
                ));
            }
        }
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
    fn gen_r8_reach_cut_only_skips_pixels_that_are_blank() {
        // The per-frame cut radius must be conservative: every sky point beyond it emits
        // exactly nothing, and nothing tones to level 0, at every stage of the loop.
        let (mut g, _) = built(Size::new(500, 150));
        assert_eq!(g.tone.level(0.0), 0);
        let px = 1.0 / g.h;
        let mut t = 0.0;
        while t < LOOP {
            g.t = t;
            let f = g.frame(px);
            for i in 0..720 {
                let a = f64::from(i) * std::f64::consts::TAU / 720.0;
                for step in 0..40 {
                    let rho = f.cut2.sqrt() * (1.0 + 1e-7) + f64::from(step) * 0.02;
                    let (s, c) = a.sin_cos_det();
                    let (x, y) = (rho * c, rho * s);
                    let e = f.emission(x, y, px, || wind_texture(x, y));
                    assert_eq!(e, (0.0, 0.0, 0.0), "t {t} at ({x}, {y})");
                }
            }
            t += 0.5;
        }
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
        g.t = 30.0;
        g.update(0.0, &mut ctx);
        let px = g.pixels.pixel_size();
        let (mut nii, mut near) = (0, 0);
        let drift = FLIER_DRIFT * (g.tau() - T_WIND).max(0.0);
        let tip = bubble_radius(g.bubble(), 1.0) * (FLIER_AT + drift) * g.h;
        let axis = g.axis();
        for iy in 0..i32::from(px.height()) {
            for ix in 0..i32::from(px.width()) {
                let ink = g.pixels.get(ix, iy).unwrap();
                let (q, d) = rim_distance(&g, ix, iy);
                if ink.role == Role::Ramp(Ramp::Aurora) {
                    let halo_cap = g.tone.level(HALO_GLOW as f32);
                    assert!(
                        q < 1.0 || ink.level <= halo_cap,
                        "bright [O III] outside the bubble at q = {q}"
                    );
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
            (r - best.0) <= thick + px,
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
}
