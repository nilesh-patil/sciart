//! V838 Monocerotis: a stellar flash echoing off surrounding dust (SPEC-002 `light_echo`).
//!
//! Light scattered towards us at time `t` after the flash's direct light arrived left the
//! star at `t_e = t − Δ/c`, where `Δ = √(ρ² + z²) − z` is the extra path via the dust
//! point at sky radius `ρ` and depth `z` (towards us): the scattering surface is the
//! paraboloid `z = ρ²/(2ct) − ct/2`. The dust is four static, rippled sheets in front of
//! the star; each node's path lag `Δ` and geometric weight (density, Henyey–Greenstein
//! phase function, inverse-square dilution) are fixed per resize, so a frame is a lookup
//! of the light curve per node and sheet. Lengths are in lattice half-heights.

use rand::Rng;
use sciart_core::grid::lattice_for;
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
use sciart_core::rng::value_noise;

use sciart_core::timeline::ease;
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "light_echo",
    category: Category::DeepCosmos,
    description: "A stellar outburst lighting up surrounding dust in rings that sweep outward",
    title: "Light echo · flash lights dust",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(LightEcho::default())
}

/// Speed of light in half-heights per second.
pub(crate) const C: f64 = 0.034;
/// Loop: dark before the flash, then the echo (s). Loop time 0 is the flash.
const PRE: f64 = 0.5;
const ECHO: f64 = 21.0;
pub(crate) const LOOP: f64 = PRE + ECHO;
/// The outburst's duration (s) and its three peaks: time, width, height.
pub(crate) const BURST: f64 = 6.0;
/// M14 (ADR-0017): sharp peaks, so each lit sheet is a thin shell (its width is the speed
/// of light times the peak's), not a broad band (were 0.55–0.95 s wide).
const PEAKS: [(f64, f64, f64); 3] = [(0.7, 0.2, 1.0), (2.4, 0.25, 0.7), (4.2, 0.3, 0.8)];
/// Outburst colour temperature: hot at first, cooling towards a red supergiant.
const T_HOT: f64 = 8_000.0;
const T_COOL: f64 = 3_000.0;
const COOL_TAU: f64 = 2.2;
/// Dust sheets: depths in front of the star, ripple amplitude, and the dust-free cavity.
const SHEETS: [f64; 4] = [0.12, 0.3, 0.55, 0.85];
pub(crate) const N_SHEETS: usize = SHEETS.len();
const RIPPLE: f64 = 0.14;
const CAVITY: f64 = 0.22;
/// Henyey–Greenstein asymmetry (forward scattering).
const G: f64 = 0.6;
/// Lattice cap (ADR-0012).
const MAX_NODES: usize = 24_000;
/// Unlit dust: brightest level (M14: 0, the unlit dust is not drawn; ADR-0017).
const DUST_LEVEL: f64 = 0.0;
/// Flashes whose echoes are drawn: age offset (s) and weight. The newest is weight 1;
/// the one before keeps expanding, fading, while the next flash lights up.
const GENERATIONS: [(f64, f32); 3] = [(0.0, 1.0), (LOOP, 0.55), (2.0 * LOOP, 0.25)];
/// Light-curve tables.
const LUT: usize = 512;
/// Echo inks (ADR-0018, [`echo_ink`]): the contrast curve's level (up to `HIGHLIGHT_MAX`)
/// in flat steps, black below the faintest; light emitted hotter than `HOT_FROM` (the
/// first, blue-white peak) in blue steps, the cooler light of the later peaks in warm ones.
const HIGHLIGHT_MAX: u8 = 240;
pub(crate) const HOT_FROM: f32 = 5_500.0;
/// Exposure of the scattered light, like a camera's auto-exposure: `update` estimates the
/// peak scattering weight lit now, on every `EXPOSE_STRIDE`-th lattice node (a fixed
/// subsample), with the light curve at its envelope (1 anywhere in the burst), so the
/// exposure follows the shells' geometric dilution and not the flash's own peaks (which
/// then brighten and fade the frame, as they should), and eases the gain toward
/// `EXPOSE·WHITE / peak` with time constant `EXPOSE_TAU` in simulated seconds, so it is smooth in time and independent of the render cadence
/// (GEN-R20); the gain is clamped to `ECHO_GAIN`, so the shells stay legible as the echo
/// spreads and dims and the dark before the flash is never amplified into noise.
const WHITE: f32 = 2.2;
const EXPOSE: f32 = 6.0;
const ECHO_GAIN: (f32, f32) = (1.0, 6.0);
const EXPOSE_STRIDE: usize = 13;
const EXPOSE_TAU: f64 = 1.5;
/// Contrast on the toned light: a shell's soft skirts fall to black, its crest stays
/// legible (thin shells).
const ECHO_GAMMA: f64 = 3.0;
/// The star: a white point (V838 Mon itself became a red supergiant, drawn so in M12).
const STAR: Ink = Ink::new(Role::Star, 255);
/// Unlit dust (none while `DUST_LEVEL` is 0).
const DUST: Ink = Ink::new(Role::Dim, 0);

/// Scattered light at contrast level `level` (0–255) in three flat steps: blue-white
/// (`Wave`, `Cold`) for light that left the star hot, sun-yellow to orange (`Sun`, `Hot`)
/// for the cooler peaks; `None` below the faintest step.
pub(crate) fn echo_ink(level: f64, hot: bool) -> Option<Ink> {
    let step = match level {
        l if l >= 150.0 => 2,
        l if l >= 70.0 => 1,
        l if l >= 18.0 => 0,
        _ => return None,
    };
    let steps = if hot {
        [
            Ink::new(Role::Cold, 60),
            Ink::new(Role::Cold, 200),
            Ink::new(Role::Wave, 230),
        ]
    } else {
        [
            Ink::new(Role::Hot, 50),
            Ink::new(Role::Hot, 190),
            Ink::new(Role::Sun, 230),
        ]
    };
    Some(steps[step])
}
/// Echo weight fades out beyond this sky radius (half-heights) over `VIGNETTE_SPAN`.
const VIGNETTE_AT: f64 = 0.95;
const VIGNETTE_SPAN: f64 = 0.5;
/// Inverse-square dilution, softened (stylised): weight `∝ 1 / r^DILUTION`.
const DILUTION: f64 = 1.0;
/// Passes of the separable `[1 2 1]` blur over the echo before tone mapping (ECHO-A7):
/// two passes are a 5-tap binomial, so the lit sheets read as soft dust, not blocks.
const BLUR_PASSES: usize = 2;

/// In-place separable `[1 2 1] / 4` blur of a `w × h` field (edges clamped), `tmp` the
/// same size.
fn blur121(buf: &mut [f32], tmp: &mut [f32], w: usize, h: usize) {
    if w == 0 || h == 0 || buf.len() < w * h || tmp.len() < w * h {
        return;
    }
    for y in 0..h {
        let row = &buf[y * w..(y + 1) * w];
        let out = &mut tmp[y * w..(y + 1) * w];
        for x in 0..w {
            let (l, r) = (row[x.saturating_sub(1)], row[(x + 1).min(w - 1)]);
            out[x] = 0.25 * (l + r) + 0.5 * row[x];
        }
    }
    for y in 0..h {
        let (up, down) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for x in 0..w {
            buf[y * w + x] = 0.25 * (tmp[up * w + x] + tmp[down * w + x]) + 0.5 * tmp[y * w + x];
        }
    }
}

/// `(L, T)` at `t_e` from the table `curve` (`LUT` entries over `[0, BURST]`), linearly
/// interpolated so band edges stay smooth; 0 outside the burst.
fn lerp_curve(curve: &[(f32, f32)], te: f64) -> (f32, f32) {
    if !(0.0..=BURST).contains(&te) || curve.len() < LUT {
        return (0.0, 0.0);
    }
    let x = te / BURST * (LUT - 1) as f64;
    let i = (x as usize).min(LUT - 2);
    let f = (x - i as f64) as f32;
    let (a, b) = (curve[i], curve[i + 1]);
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

/// Light curve `L(t_e)` of the outburst, 0 outside `[0, BURST]`.
pub(crate) fn light_curve(te: f64) -> f64 {
    if !(0.0..=BURST).contains(&te) {
        return 0.0;
    }
    let taper = 1.0 - ease::smoothstep((te - (BURST - 1.2)) / 1.2);
    PEAKS
        .iter()
        .map(|&(at, w, h)| h * (-((te - at) / w).powi(2)).exp())
        .sum::<f64>()
        * taper
        * ease::smoothstep(te / 0.25)
}

/// Colour temperature of the outburst at `t_e` (K), non-increasing (ECHO-A4).
pub(crate) fn temperature(te: f64) -> f64 {
    T_COOL + (T_HOT - T_COOL) * (-te.max(0.0) / COOL_TAU).exp()
}

/// Extra light path via dust at sky radius `rho` and depth `z` (ECHO-A1).
pub(crate) fn lag(rho: f64, z: f64) -> f64 {
    (rho * rho + z * z).sqrt() - z
}

/// Sky radius of the echo from a flat sheet at depth `z0` at time `t` after the flash:
/// `ρ = √(ct (ct + 2 z0))` (ECHO-A2).
#[cfg(test)]
pub(crate) fn ring_radius(t: f64, z0: f64) -> f64 {
    let d = C * t.max(0.0);
    (d * (d + 2.0 * z0)).sqrt()
}

fn henyey_greenstein(cos: f64) -> f64 {
    (1.0 - G * G) / (1.0 + G * G - 2.0 * G * cos).powf(1.5)
}

/// Per node and sheet: path lag (half-heights) and scattering weight.
#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    lag: f32,
    weight: f32,
}

/// One dust seed's sheets over the lattice, and its unlit-dust levels.
#[derive(Debug, Default)]
struct Dust {
    samples: Vec<[Sample; N_SHEETS]>,
    unlit: Vec<u8>,
}

impl Dust {
    fn build(&mut self, salt: u64, lattice: Size) {
        let (w, h) = (usize::from(lattice.width()), usize::from(lattice.height()));
        let half = h as f64 / 2.0;
        self.samples.clear();
        self.unlit.clear();
        for j in 0..h {
            for i in 0..w {
                let x = (i as f64 + 0.5 - w as f64 / 2.0) / half;
                let y = (j as f64 + 0.5 - half) / half;
                let rho = x.hypot(y);
                let cavity = ease::smoothstep((rho - CAVITY * 0.5) / CAVITY);
                let vignette = 1.0 - ease::smoothstep((rho - VIGNETTE_AT) / VIGNETTE_SPAN);
                let mut cell = [Sample::default(); N_SHEETS];
                let mut total = 0.0;
                for (k, (s, &z0)) in cell.iter_mut().zip(&SHEETS).enumerate() {
                    let sk = salt ^ (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                    // Swirly density: domain-warped value noise, with lanes and voids.
                    let (u, v) = (x * 3.0, y * 3.0);
                    let wx = value_noise(sk ^ 1, u, v, 1.3) - 0.5;
                    let wy = value_noise(sk ^ 2, u, v, 1.3) - 0.5;
                    let n = 0.6 * value_noise(sk, u + 1.6 * wx, v + 1.6 * wy, 0.9)
                        + 0.4 * value_noise(sk ^ 3, u + 2.2 * wy, v - 2.2 * wx, 0.4);
                    let density = ((n - 0.38) / 0.62).clamp(0.0, 1.0).powf(1.6) * cavity;
                    let z = z0 + RIPPLE * (value_noise(sk ^ 4, x * 1.6, y * 1.6, 1.0) - 0.5);
                    let r2 = rho * rho + z * z;
                    s.lag = lag(rho, z) as f32;
                    s.weight = (vignette * density * henyey_greenstein(z / r2.sqrt())
                        / r2.powf(DILUTION / 2.0)) as f32;
                    total += density;
                }
                self.samples.push(cell);
                let d = total / N_SHEETS as f64;
                let l = DUST_LEVEL * (d / 0.35).min(1.0);
                self.unlit.push(if l >= ink::MIN_VISIBLE_LEVEL {
                    l as u8
                } else {
                    0
                });
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct LightEcho {
    seeded: bool,
    /// Loop time: `−PRE` at the loop's start, 0 at the flash.
    t: f64,
    salt: u64,
    dust: Dust,
    /// `L` and `T` tabulated over `[0, BURST]`.
    curve: Vec<(f32, f32)>,
    /// Echo brightness and colour temperature per node this frame.
    light: Grid<f32>,
    kelvin: Grid<f32>,
    /// The blurred echo (brightness, and brightness × temperature) and blur scratch.
    soft: Grid<f32>,
    soft_t: Vec<f32>,
    scratch: Vec<f32>,
    map: SampleMap,
    tone: Tone,
    /// Toned level → drawn level (contrast gamma, scaled to `HIGHLIGHT_MAX`): built once,
    /// so no `powf` per pixel.
    contrast: Vec<f32>,
    /// The smoothed exposure gain (see `EXPOSE`).
    gain: f32,
    pixels: Pixels,
    centre: (f64, f64),
    half_px: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl LightEcho {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.salt = ctx.rng.random();
        self.curve = (0..LUT)
            .map(|i| {
                let te = BURST * i as f64 / (LUT - 1) as f64;
                (light_curve(te) as f32, temperature(te) as f32)
            })
            .collect();
        self.tone = Tone::asinh(0.15, WHITE);
        self.contrast = (0..=255)
            .map(|l| ((f64::from(l) / 255.0).powf(ECHO_GAMMA) * f64::from(HIGHLIGHT_MAX)) as f32)
            .collect();
        self.t = ctx.rng.random_range(4.0..8.0);
    }

    /// `L` and `T` at `t_e`, linearly interpolated in the table (0 outside the burst).
    fn curve_at(&self, te: f64) -> (f32, f32) {
        lerp_curve(&self.curve, te)
    }

    /// The exposure gain the echo at time `t` asks for: from the peak scattering weight
    /// lit now over a fixed subsample of nodes, at the light curve's envelope (see
    /// `EXPOSE`).
    fn target_gain(&self) -> f32 {
        let t = self.t;
        let mut peak = 0.0f32;
        for cell in self.dust.samples.iter().step_by(EXPOSE_STRIDE) {
            let mut sum = 0.0f32;
            for &(age, weight) in &GENERATIONS {
                for s in cell {
                    let te = t + age - f64::from(s.lag) / C;
                    if (0.0..=BURST).contains(&te) && s.weight > 0.0 {
                        sum += s.weight * weight;
                    }
                }
            }
            peak = peak.max(sum);
        }
        if peak > 0.0 {
            (EXPOSE * WHITE / peak).clamp(ECHO_GAIN.0, ECHO_GAIN.1)
        } else {
            ECHO_GAIN.1
        }
    }

    /// Echo brightness and temperature at every node for the current time: the newest
    /// flash's echo plus the fading echoes of the flashes before it (`GENERATIONS`).
    fn illuminate(&mut self) {
        let t = self.t;
        let (light, kelvin) = (self.light.as_mut_slice(), self.kelvin.as_mut_slice());
        for ((cell, l), k) in self.dust.samples.iter().zip(light).zip(kelvin) {
            let (mut sum, mut tsum) = (0.0f32, 0.0f32);
            for &(age, weight) in &GENERATIONS {
                for s in cell {
                    let te = t + age - f64::from(s.lag) / C;
                    if (0.0..=BURST).contains(&te) && s.weight > 0.0 {
                        let (lc, tc) = lerp_curve(&self.curve, te);
                        let v = s.weight * lc * weight;
                        sum += v;
                        tsum += v * tc;
                    }
                }
            }
            *l = sum;
            *k = if sum > 0.0 { tsum / sum } else { 0.0 };
        }
    }

    /// Low-pass the echo (ECHO-A7): brightness and brightness-weighted temperature are
    /// blurred together, so the colour of a softened edge is its light's mean temperature.
    fn soften(&mut self) {
        let (w, h) = (
            usize::from(self.light.width()),
            usize::from(self.light.height()),
        );
        let soft = self.soft.as_mut_slice();
        soft.copy_from_slice(self.light.as_slice());
        for ((st, &l), &k) in self
            .soft_t
            .iter_mut()
            .zip(self.light.as_slice())
            .zip(self.kelvin.as_slice())
        {
            *st = l * k;
        }
        for _ in 0..BLUR_PASSES {
            blur121(soft, &mut self.scratch, w, h);
            blur121(&mut self.soft_t, &mut self.scratch, w, h);
        }
    }

    fn raster(&mut self) {
        self.illuminate();
        self.soften();
        let (map, soft, soft_t, tone) = (&self.map, &self.soft, &self.soft_t, &self.tone);
        let contrast = &self.contrast;
        let gain = self.gain;
        let unlit = &self.dust.unlit;
        let direct = soft.size() == self.pixels.pixel_size();
        let w = usize::from(soft.width());
        let kelvin_at = |i: usize| {
            let l = soft.as_slice()[i];
            if l > 1e-9 { soft_t[i] / l } else { 0.0 }
        };
        self.pixels.shade(|x, y| {
            let (v, kv, dust) = if direct {
                let i = y as usize * w + x as usize;
                (soft.as_slice()[i], kelvin_at(i), unlit[i])
            } else {
                let (nx, ny) = map.nearest(x, y);
                let i = ny as usize * w + nx as usize;
                (map.sample(soft, x, y), kelvin_at(i), unlit[i])
            };
            // Highlights roll off on the asinh curve, scaled into the muted range, so the
            // shells keep their gradation and fade smoothly into the dark.
            let level = f64::from(contrast[usize::from(tone.level(gain * v))]);
            if kv > 0.0 {
                if let Some(ink) = echo_ink(level, kv >= HOT_FROM) {
                    return ink;
                }
            }
            if dust > 0 { DUST } else { Ink::BLANK }
        });
        self.star();
    }

    /// The star: a white point, and during the outburst's peaks a small plus.
    fn star(&mut self) {
        let (l, _) = self.curve_at(self.t);
        // No glow disc: a point, the frame's brightest accent.
        let (x, y) = (self.centre.0.floor() as i32, self.centre.1.floor() as i32);
        let ink = STAR;
        self.pixels.set(x, y, ink);
        if l > 0.3 {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                self.pixels.set(x + dx, y + dy, ink);
            }
        }
    }
}

impl Generator for LightEcho {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let lattice = lattice_for(px, MAX_NODES, 1.0);
        {
            let (d, salt) = (&mut self.dust, self.salt);
            d.build(salt, lattice);
        }
        self.light.resize(lattice, 0.0);
        self.kelvin.resize(lattice, 0.0);
        self.soft.resize(lattice, 0.0);
        let n = usize::from(lattice.width()) * usize::from(lattice.height());
        self.soft_t.clear();
        self.soft_t.resize(n, 0.0);
        self.scratch.clear();
        self.scratch.resize(n, 0.0);
        self.map.rebuild(lattice, px);
        self.centre = (f64::from(px.width()) / 2.0, f64::from(px.height()) / 2.0);
        self.half_px = f64::from(px.height()) / 2.0;
        // The exposure starts where the light is, with no ramp after a resize.
        self.gain = self.target_gain();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        while self.t >= ECHO {
            self.t -= LOOP;
        }
        let k = 1.0 - (-dt / EXPOSE_TAU).exp();
        self.gain += (self.target_gain() - self.gain) * k as f32;
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

    fn built(size: Size) -> (LightEcho, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = LightEcho::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn echo_a1_scattering_points_lie_on_the_paraboloid() {
        for &t in &[0.5, 3.0, 12.0, 30.0] {
            let ct = C * t;
            for i in 0..50 {
                let rho = f64::from(i) * 0.05;
                let z = rho * rho / (2.0 * ct) - ct / 2.0;
                assert!(
                    (lag(rho, z) - ct).abs() < 1e-9 * ct.max(1.0),
                    "t={t} ρ={rho}"
                );
            }
        }
    }

    #[test]
    fn echo_a2_rings_grow_apparently_faster_than_light_at_first() {
        let z0 = 0.3;
        let mut last = 0.0;
        for i in 1..=400 {
            let t = f64::from(i) * 0.1;
            let r = ring_radius(t, z0);
            assert!(r > last);
            last = r;
            // The ring is where the lag equals ct.
            assert!((lag(r, z0) - C * t).abs() < 1e-9);
        }
        let speed = (ring_radius(0.2, z0) - ring_radius(0.1, z0)) / 0.1;
        assert!(speed > C, "early apparent speed {speed} vs c {C}");
    }

    #[test]
    fn echo_a3_the_dust_never_moves() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let dust = g.dust.unlit.clone();
        let samples: Vec<f32> = g.dust.samples.iter().map(|c| c[1].lag).collect();
        for _ in 0..600 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(g.dust.unlit, dust);
        let again: Vec<f32> = g.dust.samples.iter().map(|c| c[1].lag).collect();
        assert_eq!(again, samples);
    }

    #[test]
    fn echo_a6_highlights_roll_off_and_hot_light_is_blue_white() {
        let (mut g, _) = built(Size::new(120, 40));
        // ADR-0018: hot light in blue steps (Cold, Wave), the rest warm (Hot, Sun).
        let (mut ice, mut ember) = (0, 0);
        for t in [1.0, 3.0, 6.0, 12.0] {
            g.t = t;
            // A jump in time: the exposure settles where the light is (`update` eases it).
            g.gain = g.target_gain();
            g.raster();
            let px = g.pixels.pixel_size();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let ink = g.pixels.get(x, y).unwrap();
                    match rank(ink) {
                        Some((_, true)) => ice += 1,
                        Some((_, false)) => ember += 1,
                        None => assert!(
                            matches!(ink.role, Role::Background | Role::Star),
                            "{ink:?} at {x},{y} t={t}"
                        ),
                    }
                }
            }
        }
        // Both the hot (early) and the cool (late) light show, the cool light dominating.
        assert!(ice > 10 && ember > ice, "ice {ice}, ember {ember}");
    }

    /// Step (1–3) and hotness of an echo ink, `None` for anything else.
    fn rank(ink: Ink) -> Option<(u8, bool)> {
        [false, true].into_iter().find_map(|hot| {
            (1u8..=3)
                .find(|&k| echo_ink([20.0, 100.0, 200.0][usize::from(k - 1)], hot) == Some(ink))
                .map(|k| (k, hot))
        })
    }

    /// Mean level step between 4-neighbouring echo pixels, and the share of echo pixels
    /// with fewer than two echo 8-neighbours (ragged specks; M14: 8-neighbours, since a
    /// thin shell running diagonally touches its neighbours only at corners).
    fn roughness(g: &LightEcho) -> (f64, f64) {
        let px = g.pixels.pixel_size();
        let echo = |x: i32, y: i32| {
            // One flat step is 40 levels' worth of change.
            g.pixels
                .get(x, y)
                .and_then(rank)
                .map(|(k, _)| 40.0 * f64::from(k))
        };
        let (mut step, mut pairs, mut lit, mut specks) = (0.0, 0, 0, 0);
        for y in 0..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let Some(l) = echo(x, y) else { continue };
                lit += 1;
                let mut nb = 0;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    if let Some(m) = echo(x + dx, y + dy) {
                        nb += 1;
                        if dx + dy > 0 {
                            step += (l - m).abs();
                            pairs += 1;
                        }
                    }
                }
                for (dx, dy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                    nb += usize::from(echo(x + dx, y + dy).is_some());
                }
                if nb < 2 {
                    specks += 1;
                }
            }
        }
        (
            step / f64::from(pairs.max(1)),
            f64::from(specks) / f64::from(lit.max(1)),
        )
    }

    #[test]
    fn echo_a7_lit_dust_is_smooth_not_blocky_or_ragged() {
        let (mut g, _) = built(Size::new(120, 40));
        for t in [3.0, 8.0, 14.0, 20.0] {
            g.t = t;
            // A jump in time: the exposure settles where the light is (`update` eases it).
            g.gain = g.target_gain();
            g.raster();
            let (step, specks) = roughness(&g);
            // M14: thin shells (1–3 px) rise and fall within a few pixels, so the bound
            // is looser than for the old broad bands; ragged specks stay rare.
            assert!(step <= 40.0, "t={t}: mean neighbour step {step:.1}");
            assert!(
                specks <= 0.025,
                "t={t}: {:.1} % ragged specks",
                100.0 * specks
            );
        }
    }

    #[test]
    fn echo_a8_exposure_is_smooth_in_time_and_cadence_free() {
        // M14: the gain eases with time constant `EXPOSE_TAU` in simulated time: one step
        // moves it by at most `1 − e^{−dt/τ}` of the way, and stepping 1/60 s or 1/30 s
        // lands on the same gain within a few per cent after a second.
        let size = Size::new(120, 40);
        let (mut a, mut ca) = built(size);
        let (mut b, mut cb) = built(size);
        let k = 1.0 - (-SIM_DT / EXPOSE_TAU).exp();
        for _ in 0..600 {
            let (g0, target) = (a.gain, a.target_gain());
            a.update(SIM_DT, &mut ca);
            let bound = (f64::from((a.target_gain() - g0).abs())
                .max(f64::from((target - g0).abs()))
                * k) as f32;
            assert!((a.gain - g0).abs() <= bound + 1e-4, "{g0} -> {}", a.gain);
        }
        for _ in 0..300 {
            b.update(2.0 * SIM_DT, &mut cb);
        }
        let rel = (a.gain - b.gain).abs() / a.gain.max(1e-6);
        assert!(rel < 0.05, "{} vs {}", a.gain, b.gain);
    }

    #[test]
    fn echo_a4_colour_is_the_outburst_temperature_at_emission() {
        let mut last = f64::INFINITY;
        for i in 0..=300 {
            let te = BURST * f64::from(i) / 300.0;
            let t = temperature(te);
            assert!(t <= last);
            last = t;
        }
        // A node lit by a single sheet carries that sheet's T(t_e).
        let (mut g, _) = built(Size::new(80, 24));
        g.t = 12.0;
        g.illuminate();
        let dust = &g.dust;
        let mut checked = 0;
        for (i, cell) in dust.samples.iter().enumerate() {
            // Every lit (sheet, flash) pair; a node with exactly one shows its colour.
            let mut lit = Vec::new();
            for &(age, _) in &GENERATIONS {
                for s in cell {
                    let te = g.t + age - f64::from(s.lag) / C;
                    if (0.0..=BURST).contains(&te) && s.weight > 0.0 && g.curve_at(te).0 > 0.0 {
                        lit.push(te);
                    }
                }
            }
            if let [te] = lit[..] {
                let want = f64::from(g.curve_at(te).1);
                let got = f64::from(g.kelvin.as_slice()[i]);
                assert!((got - want).abs() <= 1e-3 * want, "{got} vs {want}");
                checked += 1;
            }
        }
        assert!(checked > 20);
    }

    #[test]
    fn echo_a5_nothing_is_lit_by_a_flash_before_it_happens() {
        // Before loop time 0 the newest flash has not happened: every node's emission
        // time for it is negative, so it adds nothing; only older echoes can show.
        let (mut g, mut ctx) = built(Size::new(80, 24));
        while g.t > -PRE + 0.1 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        while g.t < 0.0 {
            for cell in &g.dust.samples {
                for s in cell {
                    assert_eq!(g.curve_at(g.t - f64::from(s.lag) / C).0, 0.0);
                }
            }
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(lerp_curve(&g.curve, -1e-9), (0.0, 0.0));
    }
}
