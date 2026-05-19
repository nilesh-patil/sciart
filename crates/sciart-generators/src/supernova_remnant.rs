//! A young supernova remnant like Cassiopeia A in X-ray false colour (SPEC-002
//! `supernova_remnant`).
//!
//! Three analytic surfaces, all scaled by the forward-shock radius `R_f(t)` (a
//! Truelove–McKee-style blend from ejecta-dominated to Sedov–Taylor expansion): the blast
//! wave, the contact discontinuity rippled by Rayleigh–Taylor fingers, and the reverse
//! shock inside it. Each is drawn as a projected, limb-brightened shell per pixel; ejecta
//! knots coast outward homologously and light up as the reverse shock reaches them.
//! Lengths on screen are in pixels, the model in units of `R_f`.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, disc_pixels};
use sciart_core::rng::{cell_key, hash01, value_noise};
use sciart_core::timeline::ease;
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "supernova_remnant",
    category: Category::DeepCosmos,
    description: "A supernova blast wave sweeping up gas while ejecta fingers light up at the reverse shock",
    title: "Supernova remnant · blast wave",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SupernovaRemnant::default())
}

/// Loop: expansion, then the remnant fades before the next flash (s).
const EXPAND: f64 = 36.0;
const FADE: f64 = 4.0;
pub(crate) const LOOP: f64 = EXPAND + FADE;
/// Model time at the loop's start and the Sedov–Taylor transition time (model s).
const T0: f64 = 2.0;
const T_ST: f64 = 8.0;
/// Blast-wave radius at the end of the expansion, in half-heights.
const R_END: f64 = 0.9;
/// Log slopes of the two regimes, and the blend's sharpness.
const SLOPE_ED: f64 = 4.0 / 7.0;
const SLOPE_ST: f64 = 2.0 / 5.0;
const BLEND: f64 = 2.0;
/// Contact discontinuity (units of `R_f`) and the finger amplitudes about it.
const CONTACT: f64 = 0.8;
const FINGER_OUT: f64 = 0.18;
const FINGER_IN: f64 = 0.054;
/// Reverse shock (units of `R_f`), easing inward over the expansion.
const REVERSE: (f64, f64) = (0.62, 0.45);
/// Blast-wave shell thickness (units of `R_f`) and the band (pixels) it is drawn in.
const RIM_THICK: f64 = 0.035;
const RIM_BAND: f64 = 3.0;
/// Rayleigh–Taylor modes: count, wavenumber range, seed amplitude, growth rate and cap.
const RT_MODES: usize = 24;
const RT_K: (u32, u32) = (5, 22);
const RT_SEED: f64 = 0.004;
const RT_RATE: f64 = 0.35;
const RT_CAP: f64 = 0.12;
/// Angle table resolution.
const ANGLES: usize = 512;
/// Comoving clump texture: `NOISE²` texels over `±NOISE_SPAN` `R_f`.
const NOISE: usize = 64;
const NOISE_SPAN: f64 = 1.2;
/// Emitting ejecta sit in the outer part of the shocked shell, from this fraction of
/// the contact radius out (or from the reverse shock, if that is farther out).
const EJECTA_INNER: f64 = 0.82;
/// Ejecta knots (SNR-A5), the share rich in iron, and their afterglow.
pub(crate) const KNOTS: usize = 1500;
const IRON_SHARE: f64 = 0.3;
const KNOT_TAU: f64 = 1.5;
const KNOT_FLOOR: f64 = 0.25;
/// Iron-rich sector of the shell (radians) and its half-width.
const IRON_AT: f64 = 2.4;
const IRON_HALF: f64 = 0.7;

/// Forward-shock radius in model units at model time `t` (SNR-A1): log slope `4/7`
/// early, `2/5` late.
pub(crate) fn forward_shock(t: f64) -> f64 {
    let x = (t / T_ST).powf(BLEND);
    t.powf(SLOPE_ED) / (1.0 + x).powf((SLOPE_ED - SLOPE_ST) / BLEND)
}

/// Reverse shock as a fraction of `R_f` at loop time `tau`.
pub(crate) fn reverse_frac(tau: f64) -> f64 {
    ease::lerp(REVERSE.0, REVERSE.1, ease::smoothstep(tau / EXPAND))
}

/// Contact radius as a fraction of `R_f` for a finger perturbation `p`: pushed out up to
/// `FINGER_OUT` where fingers reach, pulled in at most `FINGER_IN` between them, so
/// `R_r < R_c < R_f` always (SNR-A2).
pub(crate) fn contact_frac(p: f64) -> f64 {
    let out = (p.max(0.0) / 0.25).tanh();
    let inn = ((-p).max(0.0) / 0.25).tanh();
    CONTACT * (1.0 + FINGER_OUT * out * out - FINGER_IN * inn)
}

/// Projected path length through a shell between radii `inner < outer` at impact
/// parameter `rho`, the limb-brightened emission of a thin shell.
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

/// Bilinear sample of the `(clump, iron)` texture at comoving position `(u, v)` (`R_f`).
fn sample_noise(tex: &[(f32, f32)], u: f64, v: f64) -> (f64, f64) {
    if tex.len() < NOISE * NOISE {
        return (0.5, 0.0);
    }
    let scale = (NOISE - 1) as f64 / (2.0 * NOISE_SPAN);
    let x = ((u + NOISE_SPAN) * scale).clamp(0.0, (NOISE - 2) as f64);
    let y = ((v + NOISE_SPAN) * scale).clamp(0.0, (NOISE - 2) as f64);
    let (i, j) = (x as usize, y as usize);
    let (fx, fy) = (x - i as f64, y - j as f64);
    let at = |i: usize, j: usize| tex[j * NOISE + i];
    let mix = |a: f32, b: f32, t: f64| f64::from(a) + (f64::from(b) - f64::from(a)) * t;
    let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
    let top = (mix(a.0, b.0, fx), mix(a.1, b.1, fx));
    let bottom = (mix(c.0, d.0, fx), mix(c.1, d.1, fx));
    (
        top.0 + (bottom.0 - top.0) * fy,
        top.1 + (bottom.1 - top.1) * fy,
    )
}

/// A knot of ejecta: homologous speed (its radius in `R_f` at the end of the expansion),
/// direction, whether iron-rich, and when the reverse shock lit it (loop s).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    u: f64,
    dir: (f64, f64),
    iron: bool,
    lit: Option<f64>,
}

impl Knot {
    /// Radius in units of `R_f` at loop time `tau`: free coasting, `r ∝ t`, against a
    /// blast wave that slows.
    fn frac(&self, tau: f64, r_end: f64) -> f64 {
        let t = T0 + tau;
        let t_end = T0 + EXPAND;
        self.u * (t / t_end) * r_end / forward_shock(t)
    }

    /// Brightness at loop time `tau`: 0 until shocked, then a flare fading to a floor
    /// (non-increasing, SNR-A3).
    fn brightness(&self, tau: f64) -> f64 {
        match self.lit {
            None => 0.0,
            Some(at) => KNOT_FLOOR + (1.0 - KNOT_FLOOR) * (-(tau - at).max(0.0) / KNOT_TAU).exp(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Mode {
    k: u32,
    phase: f64,
    weight: f64,
}

#[derive(Debug, Default)]
pub struct SupernovaRemnant {
    seeded: bool,
    /// Loop time and completed loops.
    tau: f64,
    modes: Vec<Mode>,
    /// `cos(kθ)` and `sin(kθ)` per mode per angle bin.
    cos_k: Vec<f32>,
    sin_k: Vec<f32>,
    /// Finger perturbation `p(θ)` this frame, per angle bin.
    finger: Vec<f32>,
    clumps: Vec<(f32, f32)>,
    knots: ParticlePool<Knot>,
    /// Screen centre, half-height (pixels), and per pixel: radius and angle bin.
    centre: (f64, f64),
    half: f64,
    angle: Vec<u16>,
    knot_glow: Glow,
    iron_glow: Glow,
    tone_rim: Tone,
    tone_ejecta: Tone,
    tone_knot: Tone,
    pixels: Pixels,
    stars: Vec<(i32, i32, u8)>,
}

impl SupernovaRemnant {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.modes = (0..RT_MODES)
            .map(|_| Mode {
                k: ctx.rng.random_range(RT_K.0..=RT_K.1),
                phase: ctx.rng.random_range(0.0..TAU),
                weight: ctx.rng.random_range(0.5..1.0),
            })
            .collect();
        self.cos_k.clear();
        self.sin_k.clear();
        for m in &self.modes {
            for j in 0..ANGLES {
                let th = f64::from(m.k) * TAU * j as f64 / ANGLES as f64;
                let (s, c) = th.sin_cos_det();
                self.cos_k.push(c as f32);
                self.sin_k.push(s as f32);
            }
        }
        self.finger = vec![0.0; ANGLES];
        let salt: u64 = ctx.rng.random();
        self.clumps = (0..NOISE * NOISE)
            .map(|i| {
                let (x, y) = ((i % NOISE) as f64, (i / NOISE) as f64);
                let clump = 0.65 * value_noise(salt, x, y, 5.0)
                    + 0.35 * value_noise(salt ^ 0x9e37, x, y, 2.2);
                let iron = value_noise(salt ^ 0x51ed, x, y, 11.0);
                (clump as f32, iron as f32)
            })
            .collect();
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.tone_rim = Tone::asinh(0.25, 1.0);
        self.tone_ejecta = Tone::asinh(0.25, 2.6);
        self.tone_knot = Tone::asinh(0.15, 1.6);
        self.tau = ctx.rng.random_range(3.0..6.0);
        self.explode(ctx);
        self.light_knots();
    }

    /// A new supernova: fresh knots (all dark, inside the reverse shock).
    fn explode(&mut self, ctx: &mut Ctx) {
        self.knots.clear();
        for _ in 0..KNOTS {
            let u = ctx.rng.random_range(0.5..0.97);
            let a = ctx.rng.random_range(0.0..TAU);
            let near_iron = sciart_core::geom::wrap_angle(a - IRON_AT).abs() < IRON_HALF;
            let iron = ctx
                .rng
                .random_bool(if near_iron { 0.75 } else { IRON_SHARE * 0.1 });
            self.knots.spawn(Knot {
                u,
                dir: a.sin_cos_det(),
                iron,
                lit: None,
            });
        }
        for m in &mut self.modes {
            m.phase = ctx.rng.random_range(0.0..TAU);
        }
    }

    fn r_end(&self) -> f64 {
        forward_shock(T0 + EXPAND)
    }

    /// Forward-shock radius in pixels now.
    fn rf_px(&self) -> f64 {
        let tau = self.tau.min(EXPAND);
        let grow = ease::out_quad(self.tau / 1.2);
        R_END * self.half * forward_shock(T0 + tau) / self.r_end() * grow
    }

    /// Mark knots that the reverse shock has reached by `tau`.
    fn light_knots(&mut self) {
        let (tau, r_end) = (self.tau.min(EXPAND), self.r_end());
        let rr = reverse_frac(tau);
        for k in self.knots.iter_mut() {
            if k.lit.is_none() && k.frac(tau, r_end) > rr {
                k.lit = Some(self.tau);
            }
        }
    }

    fn update_fingers(&mut self) {
        let tau = self.tau.min(EXPAND);
        self.finger.fill(0.0);
        for (m, mode) in self.modes.iter().enumerate() {
            let a = (RT_SEED * (RT_RATE * (f64::from(mode.k) / 6.0).sqrt() * tau).exp())
                .min(RT_CAP)
                * mode.weight;
            let (cp, sp) = (mode.phase.cos_det(), mode.phase.sin_det());
            let (cs, ss) = (
                &self.cos_k[m * ANGLES..(m + 1) * ANGLES],
                &self.sin_k[m * ANGLES..(m + 1) * ANGLES],
            );
            for ((f, &c), &s) in self.finger.iter_mut().zip(cs).zip(ss) {
                // cos(kθ + φ)
                *f += (a * (f64::from(c) * cp - f64::from(s) * sp)) as f32;
            }
        }
    }

    fn envelope(&self) -> f64 {
        1.0 - ease::smoothstep((self.tau - EXPAND) / FADE)
    }

    fn raster(&mut self) {
        self.update_fingers();
        let tau = self.tau.min(EXPAND);
        let rf = self.rf_px();
        let rr = reverse_frac(tau) * rf;
        let env = self.envelope();
        let r_end = self.r_end();
        // Knots into their glows, each with a short tail pointing back to the centre.
        self.knot_glow.clear();
        self.iron_glow.clear();
        for k in self.knots.iter() {
            let b = k.brightness(self.tau) as f32;
            if b <= 0.0 {
                continue;
            }
            let r = k.frac(tau, r_end) * rf;
            let (s, c) = k.dir;
            let (x, y) = (self.centre.0 + r * c - 0.5, self.centre.1 - r * s - 0.5);
            let glow = if k.iron {
                &mut self.iron_glow
            } else {
                &mut self.knot_glow
            };
            glow.splat_bilinear(x, y, b);
            glow.splat_bilinear(x - 1.2 * c, y + 1.2 * s, 0.35 * b);
            glow.splat_bilinear(x - 2.4 * c, y + 2.4 * s, 0.15 * b);
        }
        let (angle, centre) = (&self.angle, self.centre);
        let (finger, clumps) = (&self.finger, &self.clumps);
        let (kg, ig) = (self.knot_glow.values(), self.iron_glow.values());
        let (t_rim, t_ej, t_kn) = (&self.tone_rim, &self.tone_ejecta, &self.tone_knot);
        let width = usize::from(self.pixels.pixel_size().width());
        let rim_norm = 1.0 / (rf * (1.0 - (1.0 - RIM_THICK) * (1.0 - RIM_THICK)).sqrt()).max(1e-9);
        let env32 = env as f32;
        self.pixels.shade(|x, y| {
            let i = y as usize * width + x as usize;
            let Some(&a) = angle.get(i) else {
                return Ink::BLANK;
            };
            let (dx, dy) = (f64::from(x) + 0.5 - centre.0, centre.1 - f64::from(y) - 0.5);
            let r = dx.hypot(dy);
            let a = usize::from(a);
            let p = f64::from(finger[a]);
            let rc = contact_frac(p) * rf;
            // Blast wave: the limb of a thin shell, only within a few pixels of R_f.
            let rim = if (rf - r).abs() <= RIM_BAND && r <= rf {
                shell_path(r, rf * (1.0 - RIM_THICK), rf) * rim_norm
            } else {
                0.0
            };
            // Shocked ejecta between the reverse shock and the contact: densest near the
            // contact (so the shell reads as a ring, not a disc), clumpy in comoving
            // coordinates, brightest in the fingers.
            let (ej, iron) = if r < rc && rf > 0.0 {
                let inner = rr.max(EJECTA_INNER * rc);
                let peak = (rc * rc - inner * inner).sqrt().max(1e-9);
                let path = shell_path(r, inner, rc) / peak;
                let (clump, iron) = sample_noise(clumps, dx / rf, dy / rf);
                let finger_boost = (0.7 + 1.2 * p.max(0.0) / 0.25).min(2.0);
                let hollow = ease::smoothstep((r / rc - 0.45) / 0.4);
                let v = hollow * path * path * (0.08 + 3.0 * clump * clump * clump) * finger_boost;
                // Iron sits in a broad patch, broken up by the clumps.
                let th = a as f64 / ANGLES as f64 * TAU;
                let sector = 1.0
                    - ease::smoothstep(
                        (sciart_core::geom::wrap_angle(th - IRON_AT).abs() - IRON_HALF * 0.5)
                            / IRON_HALF,
                    );
                (v, iron * (0.4 + 0.9 * sector) > 0.62)
            } else {
                (0.0, false)
            };
            let kv = kg.get(i).copied().unwrap_or(0.0);
            let iv = ig.get(i).copied().unwrap_or(0.0);
            let l_rim = t_rim.level((rim as f32) * env32);
            let l_ej = t_ej.level((ej as f32) * env32);
            let l_kn = t_kn.level((kv + 0.5 * ej as f32 * kv.min(1.0)) * env32);
            let l_fe = t_kn.level(iv * env32).min(200);
            // Ejecta hues stop short of Ferroin's azure top, which is the rim's.
            let l_ej = l_ej.min(160);
            let l_kn = l_kn.min(205);
            let best = l_rim.max(l_ej).max(l_kn).max(l_fe);
            if f64::from(best) < ink::MIN_VISIBLE_LEVEL {
                return Ink::BLANK;
            }
            if best == l_rim {
                Ink::new(Role::Ramp(Ramp::Cobalt), l_rim)
            } else if best == l_fe {
                Ink::new(Role::Ramp(Ramp::Sulfur), l_fe)
            } else if best == l_kn {
                Ink::new(Role::Ramp(Ramp::Ferroin), l_kn)
            } else if iron {
                Ink::new(Role::Ramp(Ramp::Sulfur), (f64::from(l_ej) * 0.85) as u8)
            } else {
                Ink::new(Role::Ramp(Ramp::Ferroin), l_ej)
            }
        });
        self.pixels.dissolve(30);
        // Background stars, then the central compact object and the flash.
        for &(x, y, l) in &self.stars {
            if self.pixels.get(x, y) == Some(Ink::BLANK) {
                self.pixels
                    .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
            }
        }
        let (cx, cy) = (self.centre.0.floor() as i32, self.centre.1.floor() as i32);
        self.pixels
            .set(cx, cy, Ink::new(Role::Ramp(Ramp::Starlight), 200));
        let flash = 1.0 - ease::smoothstep(self.tau / 1.5);
        if flash > 0.0 {
            let radius = 1.0 + 0.07 * self.half * flash;
            let (pixels, self_c) = (&mut self.pixels, self.centre);
            disc_pixels(self.centre, radius, |x, y, cover| {
                let d =
                    (f64::from(x) + 0.5 - self_c.0).hypot(f64::from(y) + 0.5 - self_c.1) / radius;
                let v = 255.0 * flash * cover * (1.0 - d.min(1.0)).powi(2);
                if v >= ink::MIN_VISIBLE_LEVEL {
                    pixels.set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), ink::level(v)));
                }
            });
            if flash > 0.3 {
                self.pixels.set(cx, cy, ink::blackbody(25_000.0));
            }
        }
    }
}

impl Generator for SupernovaRemnant {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.pixels.resize(size);
        self.knot_glow.resize(size);
        self.iron_glow.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        self.centre = (w / 2.0, h / 2.0);
        self.half = (h / 2.0).min(w / 2.0 / R_END * 0.95);
        self.angle.clear();
        let n = usize::from(px.width()) * usize::from(px.height());
        self.angle.reserve_exact(n);
        for y in 0..px.height() {
            for x in 0..px.width() {
                let (dx, dy) = (
                    f64::from(x) + 0.5 - self.centre.0,
                    self.centre.1 - f64::from(y) - 0.5,
                );
                let a = (dy.atan2(dx) / TAU).rem_euclid(1.0);
                self.angle
                    .push(((a * ANGLES as f64) as usize % ANGLES) as u16);
            }
        }
        self.stars.clear();
        let n = size.area() / 70;
        for k in 0..n as i32 {
            let x = (hash01(cell_key(k, 3)) * w) as i32;
            let y = (hash01(cell_key(k, 7)) * h) as i32;
            let l = 20 + (hash01(cell_key(k, 11)).powi(4) * 150.0) as u8;
            self.stars.push((x, y, l));
        }
        self.raster();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        self.tau += dt.max(0.0);
        if self.tau >= LOOP {
            self.tau = (self.tau - LOOP).rem_euclid(LOOP);
            self.explode(ctx);
        }
        self.light_knots();
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (SupernovaRemnant, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = SupernovaRemnant::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn log_slope(t: f64) -> f64 {
        let h = 1e-4 * t;
        (forward_shock(t + h).ln() - forward_shock(t - h).ln()) / ((t + h).ln() - (t - h).ln())
    }

    #[test]
    fn snr_a1_blast_wave_decelerates_from_free_expansion_to_sedov() {
        let mut last = 0.0;
        for i in 0..=400 {
            let t = T0 + EXPAND * f64::from(i) / 400.0;
            let r = forward_shock(t);
            assert!(r > last);
            last = r;
        }
        assert!(log_slope(T0) >= 0.55, "early {}", log_slope(T0));
        assert!(
            log_slope(T0 + EXPAND) <= 0.45,
            "late {}",
            log_slope(T0 + EXPAND)
        );
    }

    #[test]
    fn snr_a2_reverse_shock_inside_contact_inside_blast_wave() {
        for i in 0..=200 {
            let tau = EXPAND * f64::from(i) / 200.0;
            let rr = reverse_frac(tau);
            for j in -200..=200 {
                let p = f64::from(j) / 20.0;
                let rc = contact_frac(p);
                assert!(rr < rc && rc < 1.0, "τ={tau} p={p}: {rr} {rc}");
            }
        }
        // And for the live finger field.
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for _ in 0..600 {
            g.update(SIM_DT * 3.0, &mut ctx);
            let rr = reverse_frac(g.tau.min(EXPAND));
            assert!(g.finger.iter().all(|&p| {
                let rc = contact_frac(f64::from(p));
                rr < rc && rc < 1.0
            }));
        }
    }

    #[test]
    fn snr_a3_knots_light_only_after_the_reverse_shock_and_then_only_fade() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        g.tau = 0.0;
        g.explode(&mut ctx);
        let r_end = g.r_end();
        let mut last: Vec<f64> = vec![0.0; g.knots.len()];
        for _ in 0..(EXPAND / SIM_DT) as usize - 2 {
            g.update(SIM_DT, &mut ctx);
            let tau = g.tau.min(EXPAND);
            let rr = reverse_frac(tau);
            for (k, prev) in g.knots.iter().zip(&mut last) {
                let b = k.brightness(g.tau);
                if k.frac(tau, r_end) <= rr {
                    assert_eq!(b, 0.0, "a knot inside the reverse shock is dark");
                }
                if *prev > 0.0 {
                    assert!(b <= *prev + 1e-12, "brightness rose after lighting");
                }
                *prev = b;
            }
        }
        assert!(g.knots.iter().filter(|k| k.lit.is_some()).count() > KNOTS / 2);
    }

    #[test]
    fn snr_a4_rim_pixels_hug_the_blast_wave() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        for _ in 0..300 {
            g.update(SIM_DT, &mut ctx);
        }
        let rf = g.rf_px();
        let px = g.pixels.pixel_size();
        let mut n = 0;
        for y in 0..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let ink = g.pixels.get(x, y).unwrap();
                if ink.role == Role::Ramp(Ramp::Cobalt) {
                    let (dx, dy) = (
                        f64::from(x) + 0.5 - g.centre.0,
                        g.centre.1 - f64::from(y) - 0.5,
                    );
                    let r = dx.hypot(dy);
                    assert!((r - rf).abs() <= 3.0, "rim pixel at {r} vs R_f {rf}");
                    n += 1;
                }
            }
        }
        assert!(n > 50, "rim drawn ({n} px)");
    }

    #[test]
    fn snr_a5_pool_is_capped_and_the_loop_restarts_with_a_flash() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        let mut flashed = false;
        let mut prev_tau = g.tau;
        for _ in 0..(2.5 * LOOP / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            assert!(g.knots.len() <= KNOTS);
            if g.tau < prev_tau {
                // Just restarted: the centre is a flash, every knot dark again.
                let (cx, cy) = (g.centre.0.floor() as i32, g.centre.1.floor() as i32);
                assert_eq!(g.pixels.get(cx, cy).unwrap().role, Role::Blackbody);
                assert!(
                    g.knots
                        .iter()
                        .all(|k| k.lit.is_none() || k.lit == Some(g.tau))
                );
                flashed = true;
            }
            prev_tau = g.tau;
        }
        assert!(flashed);
    }
}
