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
use sciart_core::raster::{ascii_ramp_cell, disc_pixels};
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
const PEAKS: [(f64, f64, f64); 3] = [(0.7, 0.55, 1.0), (2.4, 0.8, 0.7), (4.2, 0.95, 0.8)];
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
/// Unlit dust: brightest level (SPEC-002: ≤ 20).
const DUST_LEVEL: f64 = 20.0;
/// Flashes whose echoes are drawn: age offset (s) and weight. The newest is weight 1;
/// the one before keeps expanding, fading, while the next flash lights up.
const GENERATIONS: [(f64, f32); 3] = [(0.0, 1.0), (LOOP, 0.55), (2.0 * LOOP, 0.25)];
/// Light-curve tables.
const LUT: usize = 512;
/// Echo inks: the highest level (short of white), the faintest echo drawn (fainter light
/// leaves the dust's own grey, so the far rings end cleanly instead of in dark-red
/// streaks), and the emission temperature from which the light is blue-white (Ice).
const HIGHLIGHT_MAX: u8 = 236;
const ECHO_FLOOR: u8 = 38;
pub(crate) const HOT_FROM: f32 = 5_500.0;
/// Echo weight fades out beyond this sky radius (half-heights) over `VIGNETTE_SPAN`.
const VIGNETTE_AT: f64 = 0.95;
const VIGNETTE_SPAN: f64 = 0.5;
/// Inverse-square dilution, softened (stylised): weight `∝ 1 / r^DILUTION`.
const DILUTION: f64 = 1.0;

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
    map: SampleMap,
    tone: Tone,
    pixels: Pixels,
    centre: (f64, f64),
    half_px: f64,
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
        self.tone = Tone::asinh(0.15, 2.2);
        self.t = ctx.rng.random_range(4.0..8.0);
    }

    /// `L` and `T` at `t_e`, linearly interpolated in the table (0 outside the burst).
    fn curve_at(&self, te: f64) -> (f32, f32) {
        lerp_curve(&self.curve, te)
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

    fn raster(&mut self) {
        self.illuminate();
        let (map, light, kelvin, tone) = (&self.map, &self.light, &self.kelvin, &self.tone);
        let unlit = &self.dust.unlit;
        let direct = light.size() == self.pixels.pixel_size();
        let w = usize::from(light.width());
        self.pixels.shade(|x, y| {
            let (v, kv, dust) = if direct {
                let i = y as usize * w + x as usize;
                (light.as_slice()[i], kelvin.as_slice()[i], unlit[i])
            } else {
                let (nx, ny) = map.nearest(x, y);
                let i = ny as usize * w + nx as usize;
                (map.sample(light, x, y), kelvin.as_slice()[i], unlit[i])
            };
            // Highlights roll off on the asinh curve and stop short of white, so the
            // brightest bands keep their gradation instead of clipping to flat steps.
            let level = tone.level(v).min(HIGHLIGHT_MAX);
            if level >= ECHO_FLOOR && kv > 0.0 {
                // Colour is the outburst's temperature when this light left the star: the
                // hot first peak blue-white (Ice), the cooling later light amber (Ember).
                let ramp = if kv >= HOT_FROM {
                    Ramp::Ice
                } else {
                    Ramp::Ember
                };
                return Ink::new(Role::Ramp(ramp), level.max(dust));
            }
            if dust > 0 {
                Ink::new(Role::Ramp(Ramp::Starlight), dust)
            } else {
                Ink::BLANK
            }
        });
        self.star();
    }

    /// The star: blazing during the outburst, then a red supergiant.
    fn star(&mut self) {
        let (l, k) = self.curve_at(self.t);
        let kelvin = if self.t < 0.0 {
            5_500.0
        } else if self.t <= BURST {
            f64::from(k)
        } else {
            3_000.0
        };
        let glow = f64::from(l);
        let radius = 0.8 + 0.05 * self.half_px * glow;
        let (pixels, c) = (&mut self.pixels, self.centre);
        if glow > 0.02 {
            disc_pixels(c, radius, |x, y, cover| {
                let d = (f64::from(x) + 0.5 - c.0).hypot(f64::from(y) + 0.5 - c.1) / radius;
                let v = 255.0 * cover * (1.0 - d.min(1.0)).powf(1.5) * glow.min(1.0);
                if v >= ink::MIN_VISIBLE_LEVEL {
                    pixels.set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), ink::level(v)));
                }
            });
        }
        let core = if glow > 0.3 { 1.3 } else { 0.7 };
        disc_pixels(c, core, |x, y, cover| {
            if cover > 0.3 {
                pixels.set(x, y, ink::blackbody(kelvin));
            }
        });
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
        self.map.rebuild(lattice, px);
        self.centre = (f64::from(px.width()) / 2.0, f64::from(px.height()) / 2.0);
        self.half_px = f64::from(px.height()) / 2.0;
        self.raster();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        while self.t >= ECHO {
            self.t -= LOOP;
        }
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

    fn built(size: Size) -> (LightEcho, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = LightEcho::default();
        g.resize(size, &mut ctx);
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
        }
        assert_eq!(g.dust.unlit, dust);
        let again: Vec<f32> = g.dust.samples.iter().map(|c| c[1].lag).collect();
        assert_eq!(again, samples);
    }

    #[test]
    fn echo_a6_highlights_roll_off_and_hot_light_is_blue_white() {
        let (mut g, _) = built(Size::new(120, 40));
        let (mut ice, mut ember) = (0, 0);
        for t in [1.0, 3.0, 6.0, 12.0] {
            g.t = t;
            g.raster();
            let px = g.pixels.pixel_size();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let ink = g.pixels.get(x, y).unwrap();
                    match ink.role {
                        Role::Ramp(Ramp::Ice) => ice += 1,
                        Role::Ramp(Ramp::Ember) => ember += 1,
                        _ => continue,
                    }
                    assert!(ink.level <= HIGHLIGHT_MAX, "clipped at {x},{y} t={t}");
                }
            }
        }
        // Both the hot (early) and the cool (late) light show, the cool light dominating.
        assert!(ice > 50 && ember > ice, "ice {ice}, ember {ember}");
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
        }
        while g.t < 0.0 {
            for cell in &g.dust.samples {
                for s in cell {
                    assert_eq!(g.curve_at(g.t - f64::from(s.lag) / C).0, 0.0);
                }
            }
            g.update(SIM_DT, &mut ctx);
        }
        assert_eq!(lerp_curve(&g.curve, -1e-9), (0.0, 0.0));
    }
}
