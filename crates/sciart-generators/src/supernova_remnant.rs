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
use sciart_core::raster::{ascii_ramp_cell, disc_pixels, halo_kernel};
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
const T0: f64 = 0.3;
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
/// How far (pixels) the blast wave's outward glow reaches beyond its one-pixel line.
const RIM_BAND: f64 = 2.5;
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
/// Out-of-round shape (share of `R_f`) the whole remnant shares at low angular order
/// (SNR-A6), and the angular fineness of the ejecta streaks.
const RIM_WOBBLE: f64 = 0.09;
/// Wobble modes `m` = 2, 3, 4: relative amplitudes and the slow drift of their phases
/// (rad/s), so the lopsided shell turns and reshapes as it ploughs into lumpy gas.
const WOBBLE_AMP: [f64; 3] = [1.0, 0.6, 0.35];
const WOBBLE_DRIFT: f64 = 0.05;
const STREAK: f64 = 7.0;
/// The shell is hollow inside this share of the contact radius.
const HOLLOW: f64 = 0.45;
/// A lit knot's weight in the ejecta it sits in.
const KNOT_WEIGHT: f32 = 0.9;
/// A knot's soft glow.
const KNOT_BLOB: [(i32, i32, f32); 9] = halo_kernel(1);
/// Level caps: a thin, faint forward shock under the bright ejecta.
const RIM_MAX: u8 = 85;
const EJECTA_MAX: u8 = 215;
/// Chandra-style element colours: the forward shock's synchrotron cyan, Si/S teal-green,
/// Fe orange-red, O blue.
const RIM: Ramp = Ramp::Ice;
const SILICON: Ramp = Ramp::Aurora;
const IRON: Ramp = Ramp::Ember;
const OXYGEN: Ramp = Ramp::Cobalt;

/// Which element dominates a patch of ejecta.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Element {
    Silicon,
    Iron,
    Oxygen,
}
/// Ejecta knots (SNR-A5), the share rich in iron, and their afterglow.
pub(crate) const KNOTS: usize = 1500;
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

/// Blur `buf` (`w × h`, row-major) in place with the separable 3 × 3 binomial kernel
/// `[1 2 1]/4 ⊗ [1 2 1]/4`, edges clamped; `rows` is scratch for two rows (`2·w`), so the
/// blur costs no frame-sized buffer (TEST-R7).
fn blur121(buf: &mut [f32], rows: &mut [f32], w: usize, h: usize) {
    if w == 0 || h == 0 || buf.len() < w * h || rows.len() < 2 * w {
        return;
    }
    let (prev, cur) = rows.split_at_mut(w);
    for y in 0..h {
        let row = &mut buf[y * w..(y + 1) * w];
        cur[..w].copy_from_slice(row);
        for x in 0..w {
            row[x] = 0.25 * (cur[x.saturating_sub(1)] + cur[(x + 1).min(w - 1)]) + 0.5 * cur[x];
        }
    }
    // Vertical: `prev` holds the row above as it was before this pass.
    prev.copy_from_slice(&buf[..w]);
    for y in 0..h {
        cur[..w].copy_from_slice(&buf[y * w..(y + 1) * w]);
        let down = (y + 1).min(h - 1) * w;
        for x in 0..w {
            let below = if y + 1 < h { buf[down + x] } else { cur[x] };
            buf[y * w + x] = 0.25 * (prev[x] + below) + 0.5 * cur[x];
        }
        prev.copy_from_slice(&cur[..w]);
    }
}

/// Pack a rim strength in `[0, 1]` (6 bits) and the dominant element (2 bits).
fn pack(rim: f32, element: Element) -> u8 {
    let e = match element {
        Element::Silicon => 0,
        Element::Iron => 1,
        Element::Oxygen => 2,
    };
    ((rim.clamp(0.0, 1.0) * 63.0).round() as u8) << 2 | e
}

/// The inverse of [`pack`].
fn unpack(v: u8) -> (f32, Element) {
    let element = match v & 3 {
        1 => Element::Iron,
        2 => Element::Oxygen,
        _ => Element::Silicon,
    };
    (f32::from(v >> 2) / 63.0, element)
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
    /// Angle bin of `dir` (the remnant's shape factor there).
    bin: u16,
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
    /// The remnant's low-order shape per angle bin this frame, in `[−1, 1]`, and its
    /// modes (phase, amplitude, drift).
    wobble: Vec<f32>,
    wob_modes: [(f64, f64, f64); 3],
    /// Salt of the ejecta's streak texture.
    salt: u64,
    clumps: Vec<(f32, f32)>,
    knots: ParticlePool<Knot>,
    /// Screen centre, half-height (pixels), and per pixel: radius and angle bin.
    centre: (f64, f64),
    half: f64,
    angle: Vec<u16>,
    knot_glow: Glow,
    iron_glow: Glow,
    tone_ejecta: Tone,
    /// Per pixel: the rim's strength and the dominant element, packed (the ejecta field
    /// itself is built in `knot_glow`'s buffer); two rows of blur scratch.
    aux: Vec<u8>,
    rows: Vec<f32>,
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
        self.salt = salt;
        // Low-order modes of the remnant's shape (m = 2 dominant: a lopsided shell).
        self.wob_modes = std::array::from_fn(|k| {
            (
                ctx.rng.random_range(0.0..TAU),
                WOBBLE_AMP[k] * ctx.rng.random_range(0.8..1.0),
                WOBBLE_DRIFT * ctx.rng.random_range(-1.0..1.0),
            )
        });
        self.wobble = vec![0.0; ANGLES];
        self.update_wobble();
        self.clumps = (0..NOISE * NOISE)
            .map(|i| {
                let (x, y) = ((i % NOISE) as f64, (i / NOISE) as f64);
                let oxygen = value_noise(salt, x, y, 8.0);
                let iron = value_noise(salt ^ 0x51ed, x, y, 11.0);
                (oxygen as f32, iron as f32)
            })
            .collect();
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.tone_ejecta = Tone::asinh(0.25, 2.6);
        self.tau = ctx.rng.random_range(1.0..1.3);
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
            let iron = ctx.rng.random_bool(if near_iron { 0.75 } else { 0.0 });
            self.knots.spawn(Knot {
                u,
                dir: a.sin_cos_det(),
                iron,
                lit: None,
                bin: ((a / TAU * ANGLES as f64) as usize % ANGLES) as u16,
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

    /// The remnant's shape now: modes `m` = 2–4 with slowly drifting phases, normalised
    /// by the total amplitude so it stays in `[−1, 1]`.
    fn update_wobble(&mut self) {
        let tau = self.tau.min(EXPAND);
        let norm: f64 = self.wob_modes.iter().map(|m| m.1).sum::<f64>().max(1e-9);
        let modes = self.wob_modes;
        for (j, w) in self.wobble.iter_mut().enumerate() {
            let th = TAU * j as f64 / ANGLES as f64;
            let v: f64 = modes
                .iter()
                .enumerate()
                .map(|(k, &(ph, a, drift))| {
                    a * ((k as f64 + 2.0) * th + ph + drift * tau).sin_det()
                })
                .sum();
            *w = (v / norm) as f32;
        }
    }

    /// The shape factor at an angle bin: every radius (blast wave, contact, reverse shock,
    /// knots) scales with it, so the ordering SNR-A2 holds at every angle.
    fn shape(wobble: &[f32], bin: usize) -> f64 {
        1.0 + RIM_WOBBLE * f64::from(wobble.get(bin).copied().unwrap_or(0.0))
    }

    fn envelope(&self) -> f64 {
        1.0 - ease::smoothstep((self.tau - EXPAND) / FADE)
    }

    fn raster(&mut self) {
        self.update_fingers();
        self.update_wobble();
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
            let r = k.frac(tau, r_end) * rf * Self::shape(&self.wobble, usize::from(k.bin));
            let (s, c) = k.dir;
            let (x, y) = (self.centre.0 + r * c - 0.5, self.centre.1 - r * s - 0.5);
            let glow = if k.iron {
                &mut self.iron_glow
            } else {
                &mut self.knot_glow
            };
            glow.splat(2.0 * x + 1.0, 2.0 * y + 1.0, &KNOT_BLOB, 0.45 * b);
        }
        // Pass 1, per pixel: the blast wave's rim, the ejecta's emission (knots added in)
        // and which element dominates.
        let (angle, centre) = (&self.angle, self.centre);
        let (finger, clumps, wobble, salt) = (&self.finger, &self.clumps, &self.wobble, self.salt);
        // The ejecta field is written over the knot glow in place (each pixel reads its
        // knot light before overwriting it), saving a frame-sized buffer.
        let (kg, ig) = (self.knot_glow.values_mut(), self.iron_glow.values());
        let ps = self.pixels.pixel_size();
        let (width, height) = (usize::from(ps.width()), usize::from(ps.height()));
        for (i, &a) in angle.iter().enumerate() {
            let (x, y) = ((i % width) as f64, (i / width) as f64);
            let (dx, dy) = (x + 0.5 - centre.0, centre.1 - y - 0.5);
            let r = dx.hypot(dy);
            let a = usize::from(a);
            let p = f64::from(finger[a]);
            // The whole remnant is out of round at low order where the surrounding gas
            // is lumpy: R_f, R_c and R_r all scale with the local shape factor.
            let sh = Self::shape(wobble, a);
            let rfa = rf * sh;
            let rc = contact_frac(p) * rfa;
            // Blast wave: a one-pixel line at R_f(θ) (antialiased inward), fading softly
            // outward over a couple of pixels.
            let d = r - rfa;
            let rim = if !(-1.0..=RIM_BAND).contains(&d) || rf <= 0.0 {
                0.0
            } else if d < 0.0 {
                (1.0 + d) as f32
            } else {
                (0.45 * (-d / 0.8).exp()) as f32
            };
            // Shocked ejecta between the reverse shock and the contact: densest near the
            // contact (so the shell reads as a ring, not a disc), clumpy in comoving
            // coordinates, brightest in the fingers.
            // Inside 0.45 R_c the shell is hollow (v = 0): skip the noise there.
            let (ej, element) = if r < rc && r > HOLLOW * rc && rf > 0.0 {
                let inner = (rr * sh).max(EJECTA_INNER * rc);
                let peak = (rc * rc - inner * inner).sqrt().max(1e-9);
                let path = shell_path(r, inner, rc) / peak;
                // Radial filaments: noise fine in angle, slow along the radius, so the
                // ejecta streak outward like the fingers they are.
                let (ux, uy) = (dx / r.max(1e-9), dy / r.max(1e-9));
                let q = r / rfa;
                let streak = value_noise(salt, STREAK * ux + 1.3 * q, STREAK * uy - 1.3 * q, 1.0);
                let texture = 0.3 + 1.1 * streak * streak;
                let finger_boost = (0.55 + 2.0 * p.max(0.0) / 0.25).min(2.4);
                let hollow = ease::smoothstep((q / (rc / rfa) - HOLLOW) / 0.4);
                let v = hollow * path * path * texture * finger_boost;
                // Element map: broad comoving patches (Chandra false colour).
                let (o_rich, fe_rich) = sample_noise(clumps, dx / rf, dy / rf);
                let th = a as f64 / ANGLES as f64 * TAU;
                let sector = 1.0
                    - ease::smoothstep(
                        (sciart_core::geom::wrap_angle(th - IRON_AT).abs() - IRON_HALF * 0.5)
                            / IRON_HALF,
                    );
                let element = if fe_rich * (0.4 + 0.9 * sector) > 0.6 {
                    Element::Iron
                } else if o_rich > 0.6 {
                    Element::Oxygen
                } else {
                    Element::Silicon
                };
                (v, element)
            } else {
                (0.0, Element::Silicon)
            };
            let iv = ig.get(i).copied().unwrap_or(0.0);
            let Some(slot) = kg.get_mut(i) else { continue };
            // Knots brighten the filaments they sit in; the colour stays the smooth
            // element map's (a per-knot tint would speckle).
            *slot = ej as f32 + KNOT_WEIGHT * (*slot + iv);
            if let Some(a) = self.aux.get_mut(i) {
                *a = pack(rim, element);
            }
        }
        // Pass 2: a 3 × 3 binomial blur of the ejecta, so the R–T fingers read as smooth
        // filaments, not pixel grain.
        blur121(self.knot_glow.values_mut(), &mut self.rows, width, height);
        // Pass 3: tone-map into inks.
        let t_ej = &self.tone_ejecta;
        let (aux, ej_buf) = (&self.aux, self.knot_glow.values());
        let env32 = env as f32;
        self.pixels.shade(|x, y| {
            let i = y as usize * width + x as usize;
            let (Some(&packed), Some(&ej)) = (aux.get(i), ej_buf.get(i)) else {
                return Ink::BLANK;
            };
            let (rim, element) = unpack(packed);
            let l_rim = ink::level(f64::from(rim * env32) * f64::from(RIM_MAX));
            let l_ej = t_ej.level(ej * env32).min(EJECTA_MAX);
            if f64::from(l_rim.max(l_ej)) < ink::MIN_VISIBLE_LEVEL {
                return Ink::BLANK;
            }
            if l_rim >= l_ej {
                Ink::new(Role::Ramp(RIM), l_rim)
            } else {
                let ramp = match element {
                    Element::Iron => IRON,
                    Element::Oxygen => OXYGEN,
                    Element::Silicon => SILICON,
                };
                Ink::new(Role::Ramp(ramp), l_ej)
            }
        });
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
        let n = usize::from(px.width()) * usize::from(px.height());
        self.aux.clear();
        self.aux.resize(n, 0);
        self.rows.clear();
        self.rows.resize(2 * usize::from(px.width()), 0.0);
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
                if ink.role == Role::Ramp(RIM) {
                    let (dx, dy) = (
                        f64::from(x) + 0.5 - g.centre.0,
                        g.centre.1 - f64::from(y) - 0.5,
                    );
                    let r = dx.hypot(dy);
                    let tol = 3.0 + RIM_WOBBLE * rf;
                    assert!((r - rf).abs() <= tol, "rim pixel at {r} vs R_f {rf}");
                    n += 1;
                }
            }
        }
        assert!(n > 50, "rim drawn ({n} px)");
    }

    #[test]
    fn snr_a6_blast_wave_is_out_of_round_at_low_order_and_reshapes_slowly() {
        for seed in [1_u64, 7, 42, 99, 1234] {
            let size = Size::new(120, 40);
            let mut ctx = Ctx::new(seed, size);
            let mut g = SupernovaRemnant::default();
            g.resize(size, &mut ctx);
            for _ in 0..600 {
                g.update(SIM_DT, &mut ctx);
            }
            let radii = |g: &SupernovaRemnant| -> (f64, f64) {
                let s = (0..ANGLES).map(|j| SupernovaRemnant::shape(&g.wobble, j));
                s.clone()
                    .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
            };
            let (lo, hi) = radii(&g);
            assert!(hi / lo >= 1.08, "seed {seed}: out of round {}", hi / lo);
            assert!(hi / lo <= 1.2, "seed {seed}: too lopsided {}", hi / lo);
            // Low order: the shape changes sign at most 8 times round the rim (m ≤ 4).
            let crossings = (0..ANGLES)
                .filter(|&j| (g.wobble[j] >= 0.0) != (g.wobble[(j + 1) % ANGLES] >= 0.0))
                .count();
            assert!(crossings <= 8, "seed {seed}: {crossings} sign changes");
            // Slowly: one frame later the shape moved by far less than a pixel's share.
            let before = g.wobble.clone();
            g.update(SIM_DT, &mut ctx);
            let moved = before
                .iter()
                .zip(&g.wobble)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(moved < 0.01, "seed {seed}: shape jumped {moved}");
        }
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
