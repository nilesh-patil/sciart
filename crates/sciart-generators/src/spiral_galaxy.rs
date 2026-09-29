//! A grand-design spiral as a Kalnajs kinematic density wave (SPEC-002 `spiral_galaxy`).
//!
//! Every star moves on a closed oval about the centre, `r = r_g (1 − e sin ψ)` with
//! `ψ = 2(α + ln r_g / tan p)`, where `α` is its angle in the frame turning with the
//! pattern. The ovals are nested with an orientation that turns with `ln r_g`, so they
//! crowd into two trailing logarithmic arms while every star keeps flowing through them.
//! Positions are closed form in time (no integration); lengths are in units of the disc
//! radius `R`, angles in radians.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;
use sciart_core::{InverseCdf, Tone, ink};

pub static META: Meta = Meta {
    name: "spiral_galaxy",
    category: Category::DeepCosmos,
    description: "Stars streaming through the two density-wave arms of a grand-design spiral",
    title: "Spiral galaxy · density waves",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SpiralGalaxy::default())
}

/// Arm pitch angle (18°) as `1 / tan p`, the winding rate of `ln r`.
const COT_PITCH: f64 = 3.077_683_537_175_253;
/// Pattern speed: one turn in 120 s.
pub(crate) const OMEGA_P: f64 = TAU / 120.0;
/// Corotation radius, where `Ω(r) = Ω_p`.
#[cfg(test)]
const COROTATION: f64 = 0.6;
/// Rotation-curve softening radius.
const SOFT: f64 = 0.1;
/// Flat circular speed, set so that corotation sits at `COROTATION`.
const V_FLAT: f64 = OMEGA_P * 0.608_276_253_029_821_9; // √(0.6² + 0.1²)
/// Stellar disc: exponential scale length, inner and outer guiding radii.
const SCALE_LENGTH: f64 = 0.3;
const R_IN: f64 = 0.04;
const R_OUT: f64 = 1.02;
/// Oval eccentricities.
const ECC: (f64, f64) = (0.08, 0.12);
/// Star cap and stars per cell (ADR-0012).
const MAX_STARS: usize = 20_000;
const STARS_PER_CELL: f64 = 0.8;
/// View: inclination (25°) and the position angle of the major axis on screen.
const COS_INC: f64 = 0.906_307_787_036_649_9;
const PA: f64 = 0.35;
/// Bulge σ (R) and its peak brightness, and the unresolved disc light.
const BULGE_SIGMA: f64 = 0.08;
const BULGE_PEAK: f64 = 4.0;
const DISC_LIGHT: f64 = 2.6;
const DISC_LIGHT_SCALE: f64 = 0.42;
/// Dust lane: centre and half-width in arm phase (SPG-A4), and its opacity.
const DUST_PHI: f64 = -0.4;
const DUST_HALF: f64 = 0.2;
const DUST_OPACITY: f32 = 0.85;
/// Arm contrast of the unresolved disc light, `√(1 − a²) / (1 − a cos φ)`: the same
/// crowding the stars show, without their shot noise.
const ARM_A: f64 = 0.85;
/// Arm-phase table resolution (one turn of `φ`).
const PHASES: usize = 4096;
/// Radius table: bins over `[0, R_LUT_MAX]` R.
const RBINS: usize = 4096;
const R_LUT_MAX: f64 = 2.5;
/// H II knots: pool, lifetime (s), spawn rate (per s), the arm phase they are born at
/// (SPG-A5) and their guiding-radius range.
pub(crate) const KNOTS: usize = 48;
const KNOT_LIFE: f64 = 3.0;
const KNOT_RATE: f64 = 13.0;
const KNOT_PHI: f64 = 0.25;
const KNOT_R: (f64, f64) = (0.25, 0.92);
/// ADR-0017: an H II region shows as one `Ramp(HAlpha)` pixel at up to `KNOT_PEAK`, and
/// only the larger ones (`size ≥ KNOT_SHOWN`, about a third), so a few pink points dot
/// the arms instead of blobs.
const KNOT_PEAK: f64 = 100.0;
const KNOT_SHOWN: f64 = 0.87;
/// Young blue stars: share of the population and the arm-phase band they shine in.
const YOUNG_SHARE: f64 = 0.12;
const YOUNG_BAND: (f64, f64) = (-0.15, 0.7);
/// Only the brighter young stars show (ADR-0017: the arms slightly blue, not dotted).
const YOUNG_LUM: f32 = 0.2;
/// Tone curve for the integrated starlight.
const KNEE: f32 = 0.35;
const WHITE: f32 = 9.0;
/// ADR-0017: the unresolved light below `HAZE` times the mean disc light at that radius
/// is black (no inter-arm haze; the arms and the bulge stand on a dark disc), and the
/// toned light spans `Ramp(Starlight)` up to `LIGHT_TOP` (no white-out).
const HAZE: f32 = 0.8;
const LIGHT_TOP: f32 = 180.0 / 255.0;
/// Field stars of our own Galaxy in front, per cell.
const FIELD_PER_CELL: f64 = 1.0 / 90.0;

/// `Ω(r)`: flat rotation curve, softened inside `SOFT` (rad/s).
pub(crate) fn omega(r: f64) -> f64 {
    V_FLAT / (r * r + SOFT * SOFT).sqrt()
}

/// One star: guiding radius, pattern-frame angle at `t = 0`, eccentricity, brightness
/// and, for young stars, a colour temperature (0 for the old population).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Star {
    r: f32,
    alpha0: f32,
    ecc: f32,
    lum: f32,
    kelvin: f32,
}

/// Position of `star` at time `t` in the galaxy plane (units of R, inertial frame), and
/// its pattern-frame angle. The oval is closed in the pattern frame (SPG-A1).
pub(crate) fn star_pos(r: f64, alpha0: f64, ecc: f64, t: f64) -> (f64, f64) {
    let alpha = alpha0 + (omega(r) - OMEGA_P) * t;
    let psi = 2.0 * (alpha + r.ln() * COT_PITCH);
    let rad = r * (1.0 - ecc * psi.sin_det());
    let (s, c) = (alpha + OMEGA_P * t).sin_cos_det();
    (rad * c, rad * s)
}

/// Pattern-frame position (for the closure test): the inertial position turned back by
/// the pattern's angle.
#[cfg(test)]
pub(crate) fn pattern_pos(r: f64, alpha0: f64, ecc: f64, t: f64) -> (f64, f64) {
    let (x, y) = star_pos(r, alpha0, ecc, t);
    let (s, c) = (-OMEGA_P * t).sin_cos_det();
    (x * c - y * s, x * s + y * c)
}

/// Arm phase `φ = 2(θ + ln r / tan p − Ω_p t)` wrapped onto `(−π, π]`: 0 on an arm crest,
/// negative on its inner (upstream) edge.
pub(crate) fn arm_phase(x: f64, y: f64, t: f64) -> f64 {
    let r = x.hypot(y).max(1e-9);
    let phi = 2.0 * (y.atan2(x) + r.ln() * COT_PITCH - OMEGA_P * t);
    sciart_core::geom::wrap_angle(phi)
}

/// Dust opacity at arm phase `φ`: a raised cosine on `[DUST_PHI ± DUST_HALF]` (SPG-A4).
pub(crate) fn dust(phi: f64) -> f64 {
    let d = (phi - DUST_PHI) / DUST_HALF;
    if d.abs() >= 1.0 {
        0.0
    } else {
        0.5 + 0.5 * (PI * d).cos_det()
    }
}

/// An H II region: galaxy-plane guiding radius and angle at birth, and its age.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    r: f64,
    theta0: f64,
    age: f64,
    size: f64,
}

impl Knot {
    fn pos(&self) -> (f64, f64) {
        let (s, c) = (self.theta0 + omega(self.r) * self.age).sin_cos_det();
        (self.r * c, self.r * s)
    }

    /// Brightness envelope over its life: a quick flare, then a slow fade.
    fn envelope(&self) -> f64 {
        let x = self.age / KNOT_LIFE;
        ease::smoothstep(x / 0.15) * (1.0 - ease::smoothstep((x - 0.6) / 0.4))
    }
}

#[derive(Debug, Default)]
pub struct SpiralGalaxy {
    seeded: bool,
    t: f64,
    /// Stars in units of R (kept across resizes), and the capacity in use.
    all_stars: Vec<Star>,
    n_stars: usize,
    knots: ParticlePool<Knot>,
    knot_debt: f64,
    field: Vec<(i32, i32, u8)>,
    /// Screen: centre and R in pixels, and the view's rotation.
    centre: (f64, f64),
    r_px: f64,
    pa: (f64, f64),
    /// Per pixel: arm-phase index of `2(θ + ln r / tan p)` and radius bin.
    phase: Vec<u16>,
    rbin: Vec<u16>,
    /// Per radius bin: bulge light, unresolved-disc light (before its arm modulation)
    /// and dust weight.
    radial: Vec<(f32, f32, f32)>,
    /// `dust(φ)` and the arm factor over one turn of `φ`.
    dust_lut: Vec<(f32, f32)>,
    glow: Glow,
    tone: Tone,
    pixels: Pixels,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl SpiralGalaxy {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        let cdf = InverseCdf::<256>::build(R_IN, R_OUT, |r| {
            r * (-r / SCALE_LENGTH).exp() * (1.0 - ease::smoothstep((r - 0.85) / 0.17))
        });
        self.all_stars = (0..MAX_STARS)
            .map(|_| {
                let r = cdf.sample(ctx.rng.random_range(0.0..1.0));
                let young = ctx.rng.random_bool(YOUNG_SHARE);
                let u: f64 = ctx.rng.random_range(0.0..1.0);
                Star {
                    r: r as f32,
                    alpha0: ctx.rng.random_range(0.0..TAU) as f32,
                    ecc: ctx.rng.random_range(ECC.0..ECC.1) as f32,
                    lum: (0.05 + 1.6 * u.powi(6)) as f32,
                    kelvin: if young {
                        ctx.rng.random_range(12_000.0..25_000.0)
                    } else {
                        0.0
                    },
                }
            })
            .collect();
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.dust_lut = (0..PHASES)
            .map(|i| {
                let phi = sciart_core::geom::wrap_angle(i as f64 / PHASES as f64 * TAU);
                let arm = (1.0 - ARM_A * ARM_A).sqrt() / (1.0 - ARM_A * phi.cos_det());
                (dust(phi) as f32, arm as f32)
            })
            .collect();
        self.tone = Tone::asinh(KNEE, WHITE);
        self.t = ctx.rng.random_range(0.0..120.0);
    }

    /// Galaxy plane (units of R) → pixel position.
    fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let (u, v) = (x, y * COS_INC);
        let (s, c) = self.pa;
        let (sx, sy) = (u * c - v * s, u * s + v * c);
        (
            self.centre.0 + sx * self.r_px,
            self.centre.1 - sy * self.r_px,
        )
    }

    /// Pixel centre → galaxy plane (units of R).
    fn to_plane(&self, px: i32, py: i32) -> (f64, f64) {
        let sx = (f64::from(px) + 0.5 - self.centre.0) / self.r_px;
        let sy = -(f64::from(py) + 0.5 - self.centre.1) / self.r_px;
        let (s, c) = self.pa;
        let (u, v) = (sx * c + sy * s, -sx * s + sy * c);
        (u, v / COS_INC)
    }

    fn spawn_knots(&mut self, dt: f64, ctx: &mut Ctx) {
        self.knot_debt += KNOT_RATE * dt;
        while self.knot_debt >= 1.0 {
            self.knot_debt -= 1.0;
            let r = ctx.rng.random_range(KNOT_R.0..KNOT_R.1);
            let arm = if ctx.rng.random_bool(0.5) { 0.0 } else { PI };
            // θ where φ = KNOT_PHI at radius r now.
            let theta0 = KNOT_PHI / 2.0 - r.ln() * COT_PITCH + OMEGA_P * self.t + arm;
            let size = ctx.rng.random_range(0.6..1.0);
            self.knots.spawn(Knot {
                r,
                theta0,
                age: 0.0,
                size,
            });
        }
    }

    fn raster(&mut self) {
        let glow = &mut self.glow;
        glow.clear();
        let t = self.t;
        for s in &self.all_stars[..self.n_stars] {
            let (x, y) = star_pos(f64::from(s.r), f64::from(s.alpha0), f64::from(s.ecc), t);
            let (px, py) = {
                let (u, v) = (x, y * COS_INC);
                let (sn, c) = self.pa;
                let (sx, sy) = (u * c - v * sn, u * sn + v * c);
                (
                    self.centre.0 + sx * self.r_px - 0.5,
                    self.centre.1 - sy * self.r_px - 0.5,
                )
            };
            // The outer disc's few stars are dimmed, so the halo reads smooth, not
            // speckled.
            let outer = 1.0 - 0.7 * ease::smoothstep((f64::from(s.r) - 0.55) / 0.45);
            glow.splat_bilinear(px, py, s.lum * outer as f32);
        }
        let rot = ((2.0 * OMEGA_P * t / TAU).rem_euclid(1.0) * PHASES as f64) as usize;
        let (tone, lut) = (&self.tone, &self.dust_lut);
        let (phase, rbin, radial) = (&self.phase, &self.rbin, &self.radial);
        let values = self.glow.values();
        let width = usize::from(self.pixels.pixel_size().width());
        self.pixels.shade(|x, y| {
            let i = y as usize * width + x as usize;
            let (Some(&v), Some(&p), Some(&rb)) = (values.get(i), phase.get(i), rbin.get(i)) else {
                return Ink::BLANK;
            };
            let (d, arm) = lut[(usize::from(p) + PHASES - rot) % PHASES];
            let (bulge, disc, dust_w) = radial[usize::from(rb)];
            let d = d * dust_w;
            let light = (v + bulge + disc * arm) * (1.0 - DUST_OPACITY * d);
            // The haze floor: the mean disc light here (the arm factor averages 1).
            let level = (f32::from(tone.level((light - HAZE * disc).max(0.0))) * LIGHT_TOP) as u8;
            if f64::from(level) < ink::MIN_VISIBLE_LEVEL {
                Ink::BLANK
            } else {
                Ink::new(Role::Ramp(Ramp::Starlight), level)
            }
        });
        // Young stars: blue-white points on the arms, just downstream of the shock; only
        // the arm-phase band's brighter half shows, so they scatter rather than line up.
        for (n, s) in self.all_stars[..self.n_stars].iter().enumerate() {
            if s.kelvin == 0.0 || s.lum < YOUNG_LUM {
                continue;
            }
            let (x, y) = star_pos(f64::from(s.r), f64::from(s.alpha0), f64::from(s.ecc), t);
            let phi = arm_phase(x, y, t);
            let band = (phi - YOUNG_BAND.0) / (YOUNG_BAND.1 - YOUNG_BAND.0);
            if !(0.0..=1.0).contains(&band) || hash01(n as u64) > 1.0 - band * 0.6 {
                continue;
            }
            let (px, py) = self.to_screen(x, y);
            self.pixels.set(
                px.floor() as i32,
                py.floor() as i32,
                ink::blackbody(f64::from(s.kelvin)),
            );
        }
        // H II regions: a pink point each, the larger ones only, fading with their life.
        for k in self.knots.iter() {
            let level = KNOT_PEAK * k.envelope();
            if k.size < KNOT_SHOWN || level < ink::MIN_VISIBLE_LEVEL {
                continue;
            }
            let (x, y) = k.pos();
            let (px, py) = self.to_screen(x, y);
            let (px, py) = (px.floor() as i32, py.floor() as i32);
            // Two knots born side by side would make a blob: the later one waits.
            let pink = |dx: i32, dy: i32| {
                self.pixels
                    .get(px + dx, py + dy)
                    .is_some_and(|i| i.role == Role::Ramp(Ramp::HAlpha))
            };
            if [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|&(dx, dy)| pink(dx, dy))
            {
                continue;
            }
            let ink = Ink::new(Role::Ramp(Ramp::HAlpha), ink::level(level));
            self.pixels.set(px, py, ink);
        }
        for &(x, y, l) in &self.field {
            if self.pixels.get(x, y) == Some(Ink::BLANK) {
                self.pixels
                    .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
            }
        }
    }
}

impl Generator for SpiralGalaxy {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.n_stars = ((size.area() as f64 * STARS_PER_CELL) as usize).clamp(1, MAX_STARS);
        self.glow.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        self.centre = (w / 2.0, h / 2.0);
        self.r_px = 0.5 * w.min(h / COS_INC) * 0.94;
        self.pa = PA.sin_cos_det();
        // Per pixel only two indices (TEST-R7): the arm phase and the galactocentric
        // radius bin; the radial profiles live in a small table.
        self.phase.clear();
        self.rbin.clear();
        let n = usize::from(px.width()) * usize::from(px.height());
        self.phase.reserve_exact(n);
        self.rbin.reserve_exact(n);
        for py in 0..i32::from(px.height()) {
            for pxi in 0..i32::from(px.width()) {
                let (x, y) = self.to_plane(pxi, py);
                let r = x.hypot(y).max(1e-6);
                let b = 2.0 * (y.atan2(x) + r.ln() * COT_PITCH);
                let idx = ((b / TAU).rem_euclid(1.0) * PHASES as f64) as usize % PHASES;
                self.phase.push(idx as u16);
                let bin = (r / R_LUT_MAX * RBINS as f64).round() as usize;
                self.rbin.push(bin.min(RBINS - 1) as u16);
            }
        }
        // Field stars: hashed positions, stable for a size.
        self.field.clear();
        let count = (size.area() as f64 * FIELD_PER_CELL) as u64;
        for k in 0..count {
            let x = (hash01(cell_key(k as i32, 17)) * w) as i32;
            let y = (hash01(cell_key(k as i32, 91)) * h) as i32;
            let l = 30 + (hash01(cell_key(k as i32, 5)).powi(3) * 120.0) as u8;
            self.field.push((x, y, l));
        }
        // Star brightness per pixel should not depend on the canvas: normalise by the
        // number of stars per pixel of disc.
        let per_px = self.n_stars as f64 / (PI * self.r_px * self.r_px * COS_INC).max(1.0);
        self.tone = Tone::asinh(
            KNEE * (per_px as f32 / 0.6).max(0.05),
            WHITE * (per_px as f32 / 0.6).max(0.05),
        );
        let k = per_px / 0.6;
        self.radial.clear();
        self.radial.extend((0..RBINS).map(|i| {
            let r = i as f64 / RBINS as f64 * R_LUT_MAX;
            let bulge = BULGE_PEAK * (-(r * r) / (2.0 * BULGE_SIGMA * BULGE_SIGMA)).exp();
            let disc = DISC_LIGHT
                * (-r / DISC_LIGHT_SCALE).exp()
                * (1.0 - ease::smoothstep((r - 0.55) / 0.5));
            let dust_w =
                ease::smoothstep((r - 0.1) / 0.15) * (1.0 - ease::smoothstep((r - 0.8) / 0.25));
            ((k * bulge) as f32, (k * disc) as f32, dust_w as f32)
        }));
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        for k in self.knots.iter_mut() {
            k.age += dt;
        }
        self.knots.retain(|k| k.age < KNOT_LIFE);
        self.spawn_knots(dt, ctx);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size, seed: u64) -> (SpiralGalaxy, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = SpiralGalaxy::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn spg_a1_orbits_close_after_one_pattern_frame_period() {
        for &(r, a0, e) in &[(0.2, 0.3, 0.1), (0.45, 2.0, 0.08), (0.9, 5.0, 0.12)] {
            let period = TAU / (omega(r) - OMEGA_P).abs();
            let p0 = pattern_pos(r, a0, e, 1.5);
            let p1 = pattern_pos(r, a0, e, 1.5 + period);
            assert!(
                (p0.0 - p1.0).abs() < 1e-9 && (p0.1 - p1.1).abs() < 1e-9,
                "r={r}"
            );
        }
    }

    /// Stars binned by arm phase at time `t`, `bins` bins over `(−π, π]`, radii in
    /// `r_range`.
    fn phase_hist(g: &SpiralGalaxy, t: f64, bins: usize, r_range: (f64, f64)) -> Vec<f64> {
        let mut h = vec![0.0; bins];
        for s in &g.all_stars[..g.n_stars] {
            let (x, y) = star_pos(f64::from(s.r), f64::from(s.alpha0), f64::from(s.ecc), t);
            let r = x.hypot(y);
            if r < r_range.0 || r > r_range.1 {
                continue;
            }
            let phi = arm_phase(x, y, t);
            let b = (((phi + PI) / TAU) * bins as f64) as usize % bins;
            h[b] += 1.0;
        }
        h
    }

    #[test]
    fn spg_a2_star_density_peaks_on_the_arm_with_contrast() {
        let (g, _) = built(Size::new(200, 50), 42);
        let h = phase_hist(&g, g.t, 21, (0.15, 0.95));
        let (imax, &max) = h
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        let centre = -PI + (imax as f64 + 0.5) * TAU / 21.0;
        assert!(centre.abs() <= 0.3 + PI / 21.0, "peak at φ = {centre}");
        let min = h.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(max / min >= 1.8, "contrast {}", max / min);
    }

    /// Angle of the density peak in the annulus around `r`, via the m = 2 Fourier phase.
    fn arm_angle(g: &SpiralGalaxy, t: f64, r0: f64) -> f64 {
        let (mut c, mut s) = (0.0, 0.0);
        for st in &g.all_stars[..g.n_stars] {
            let (x, y) = star_pos(f64::from(st.r), f64::from(st.alpha0), f64::from(st.ecc), t);
            if (x.hypot(y) - r0).abs() < 0.03 {
                let th = 2.0 * y.atan2(x);
                c += th.cos_det();
                s += th.sin_det();
            }
        }
        s.atan2(c) / 2.0
    }

    #[test]
    fn spg_a3_pattern_turns_at_omega_p_while_stars_do_not() {
        let (g, _) = built(Size::new(200, 50), 7);
        let dt = 4.0;
        let a0 = arm_angle(&g, 0.0, 0.5);
        let a1 = arm_angle(&g, dt, 0.5);
        let rate = sciart_core::geom::wrap_angle(2.0 * (a1 - a0)) / 2.0 / dt;
        assert!(
            (rate - OMEGA_P).abs() <= 0.05 * OMEGA_P,
            "rate {rate} vs {OMEGA_P}"
        );
        assert!(omega(0.3) > OMEGA_P && omega(0.9) < OMEGA_P);
        assert!((omega(COROTATION) - OMEGA_P).abs() < 1e-12);
    }

    #[test]
    fn spg_a4_dust_lanes_sit_on_the_inner_edge_and_dim_the_light() {
        // The dust window lies at negative arm phase, and at fixed θ smaller radii have
        // smaller φ: the inner side of the crest.
        let mut sum = 0.0;
        let mut n = 0.0;
        for i in 0..2000 {
            let phi = -PI + TAU * (f64::from(i) + 0.5) / 2000.0;
            let d = dust(phi);
            sum += d * phi;
            n += d;
        }
        assert!(sum / n < 0.0);
        let (x, y) = (0.5, 0.0);
        let (x2, y2) = (0.45, 0.0);
        assert!(arm_phase(x2, y2, 0.0) < arm_phase(x, y, 0.0));
        // In the frame, dust-lane pixels are dimmer than the crest at the same radius.
        let (g, _) = built(Size::new(200, 50), 3);
        let px = g.pixels.pixel_size();
        let (mut lane, mut crest) = ((0.0, 0.0), (0.0, 0.0));
        for py in 0..i32::from(px.height()) {
            for pxi in 0..i32::from(px.width()) {
                let (x, y) = g.to_plane(pxi, py);
                let r = x.hypot(y);
                if !(0.3..0.7).contains(&r) {
                    continue;
                }
                let phi = arm_phase(x, y, g.t);
                let l = f64::from(g.pixels.get(pxi, py).unwrap().level);
                if (phi - DUST_PHI).abs() < 0.08 {
                    lane.0 += l;
                    lane.1 += 1.0;
                } else if phi.abs() < 0.08 {
                    crest.0 += l;
                    crest.1 += 1.0;
                }
            }
        }
        assert!(lane.0 / lane.1 < crest.0 / crest.1, "{lane:?} {crest:?}");
    }

    #[test]
    fn spg_a6_dark_between_the_arms_and_knots_are_single_points() {
        // ADR-0017: no inter-arm haze, no pink blobs, no white-out.
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size, 3);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let px = g.pixels.pixel_size();
        let (mut between, mut dark) = (0, 0);
        for py in 0..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let ink = g.pixels.get(x, py).unwrap();
                match ink.role {
                    Role::Ramp(Ramp::Starlight) => assert!(ink.level <= 180, "{ink:?}"),
                    Role::Ramp(Ramp::HAlpha) => {
                        assert!(f64::from(ink.level) <= KNOT_PEAK + 1.0, "{ink:?}");
                        // A point: no H-alpha neighbour left, right, above or below.
                        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                            let n = g.pixels.get(x + dx, py + dy).unwrap_or(Ink::BLANK);
                            assert_ne!(n.role, Role::Ramp(Ramp::HAlpha), "blob at ({x}, {py})");
                        }
                    }
                    _ => {}
                }
                let (u, v) = g.to_plane(x, py);
                let r = u.hypot(v);
                let phi = arm_phase(u, v, g.t);
                if (0.3..0.7).contains(&r) && (phi.abs() - PI).abs() < 0.4 {
                    between += 1;
                    dark += usize::from(ink.level < 40 || ink.role == Role::Blackbody);
                }
            }
        }
        assert!(between > 200, "{between} inter-arm pixels");
        assert!(
            10 * dark >= 7 * between,
            "{dark} of {between} inter-arm pixels dark"
        );
    }

    #[test]
    fn spg_a5_knots_are_born_downstream_and_the_pool_is_capped() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size, 11);
        for _ in 0..1200 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            assert!(g.knots.len() <= KNOTS);
            for k in g.knots.iter() {
                let (x, y) = k.pos();
                let phi = arm_phase(x, y, g.t);
                assert!((0.0..=0.8).contains(&phi), "knot at φ = {phi}");
            }
        }
        assert!(g.knots.len() > KNOTS / 2);
    }
}
