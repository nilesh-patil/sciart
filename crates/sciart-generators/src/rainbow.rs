//! A rain shower lit by a low Sun behind the viewer (SPEC-002 `rainbow`): a bright primary
//! bow with red outside, a fainter secondary with its colours reversed, the darker sky
//! between them (Alexander's band), pastel supernumerary arcs inside the primary, and
//! slanting rain.
//!
//! Every pixel sees the rain at an angle `θ` from the antisolar point. Light leaving the
//! drops after one internal reflection (the primary) or two (the secondary) piles up at the
//! Descartes minimum-deviation angle of each wavelength; near it, Airy's wave theory turns
//! the caustic into `Ai²` fringes, the supernumeraries. A look-up table over `cos θ`
//! (size-independent, built once) holds, per angle, how much bow light there is above the
//! smooth sky, its dominant wavelength as a hue, and the smooth scattered-light sky.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

use crate::shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "rainbow",
    category: Category::EarthSky,
    description: "Primary and reversed secondary bows, Alexander's dark band and supernumeraries",
    title: "Rainbow · raindrops split light",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Rainbow::default())
}

/// Wavelengths sampled (nm), 400–700.
const WAVES: usize = 12;
/// Drop radius (nm): 0.5 mm.
const DROP: f64 = 5.0e5;
/// Airy fringe scale: `z = AIRY · (a/λ)^{2/3} · Δθ` (Δθ in radians). Stylised: fringes
/// ~1.8° apart, about 2.5× real ones for 0.5 mm drops, so they resolve on a terminal.
const AIRY: f64 = 0.75;
/// Supernumeraries fade beyond the first few (a spread of drop sizes washes them out).
const FRINGE_FADE: f64 = 5.0;
/// The secondary is this much fainter than the primary.
const SECONDARY: f64 = 0.43;
/// Look-up table resolution over `cos θ ∈ [0, 1]`.
const LUT: usize = 8192;
/// Bow light above which a pixel is all colour, and the level for none.
const BOW_FULL: f64 = 0.06;
const BOW_NONE: f64 = 0.02;
/// Sky `Ramp(Starlight)` levels: the rain cloud's base (brighter toward the horizon) and
/// the extra from light scattered by the drops, scaled so the bright inside of the
/// primary stands out from Alexander's band.
const SKY_BASE: (f64, f64) = (20.0, 38.0);
const SKY_SCATTER: f64 = 420.0;
/// The sunlit rain's glow inside the primary (`Ramp(Starlight)` level before scatter).
const INNER_GLOW: f64 = 66.0;
/// Colour dispersion drawn ×this about 550 nm (stylised: a real bow's red-to-violet span
/// of ~1.8° is 2–4 pixels on a terminal).
const DISPERSION: f64 = 2.0;
/// Where the drawn red begins, in Airy units beyond red's own maximum.
const RED_RIM: f64 = 0.45;
/// Rain streaks: pool capacity, per cell, `Ramp(Starlight)` level range, fall speed
/// (cells/s) and slant (cells across per cell down).
const RAIN_CAP: usize = 1500;
const RAIN_DENSITY: f64 = 0.28;
const RAIN_LEVEL: (f64, f64) = (60.0, 95.0);
const RAIN_SPEED: (f64, f64) = (26.0, 40.0);
const SLANT: f64 = 0.3;
/// Sun elevation over one loop (deg): it sinks, so the bow rises.
const SUN_ELEV: (f64, f64) = (18.0, 8.0);
const LOOP: f64 = 50.0;
/// Framing: the horizon's row (share of the height) and the sky shown above it (deg).
const HORIZON_ROW: f64 = 0.9;
const SKY_SPAN: f64 = 40.0;
/// Hills the bow's feet stand on.
const HILLS: Terrain = Terrain {
    ridge: (0.6, 2.5),
    trees: Some((0.8, 2.2)),
    gap: (0.3, 1.2),
    clearing: 0.35,
};

/// Refractive index of water at `nm` (Cauchy fit through 404 nm and 700 nm).
pub(crate) fn water_n(nm: f64) -> f64 {
    1.324_96 + 2961.0 / (nm * nm)
}

/// Descartes' rainbow angle (rad from the antisolar point) after `k` internal reflections:
/// the minimum-deviation ray has `cos² i = (n² − 1) / (k (k + 2))`.
pub(crate) fn bow_angle(n: f64, k: u32) -> f64 {
    let k = f64::from(k);
    let cos_i = ((n * n - 1.0) / (k * (k + 2.0))).sqrt();
    let i = cos_i.acos();
    let r = (i.sin_det() / n).asin();
    let dev = 2.0 * (i - r) + k * (PI - 2.0 * r);
    // Primary: θ = π − D; secondary: θ = D − π (the light comes back from the other side).
    if k < 1.5 { PI - dev } else { dev - PI }
}

/// The bow angle as drawn: Descartes' angle with its spread about 550 nm widened by
/// [`DISPERSION`].
fn drawn_angle(nm: f64, k: u32) -> f64 {
    let mid = bow_angle(water_n(550.0), k);
    mid + DISPERSION * (bow_angle(water_n(nm), k) - mid)
}

/// Airy's function `Ai(x)`: the power series for `|x| ≤ 6`, the asymptotic forms beyond.
pub(crate) fn airy(x: f64) -> f64 {
    const C1: f64 = 0.355_028_053_887_817;
    const C2: f64 = 0.258_819_403_792_807;
    if x < -6.0 {
        let z = -x;
        let zeta = 2.0 / 3.0 * z.powf(1.5);
        return (zeta + PI / 4.0).sin_det() / (PI.sqrt() * z.powf(0.25));
    }
    if x > 6.0 {
        let zeta = 2.0 / 3.0 * x.powf(1.5);
        return (-zeta).exp() / (2.0 * PI.sqrt() * x.powf(0.25));
    }
    let x3 = x * x * x;
    let (mut f, mut g) = (1.0, x);
    let (mut tf, mut tg) = (1.0, x);
    for k in 0..60 {
        let k = f64::from(k);
        tf *= x3 / ((3.0 * k + 2.0) * (3.0 * k + 3.0));
        tg *= x3 / ((3.0 * k + 3.0) * (3.0 * k + 4.0));
        f += tf;
        g += tg;
        if tf.abs() < 1e-17 && tg.abs() < 1e-17 {
            break;
        }
    }
    C1 * f - C2 * g
}

/// One bow's intensity `z` Airy units onto its bright side: `Ai(−z)²`, with fringes past
/// the first damped toward their mean `1/(2π√z)` (so the sky inside is smooth).
fn bow(z: f64) -> f64 {
    let ai = airy(-z);
    let wave = ai * ai;
    if z <= 2.0 {
        return wave;
    }
    let mean = 1.0 / (2.0 * PI * z.sqrt());
    mean + (wave - mean) * (-(z - 2.0) / FRINGE_FADE).exp()
}

/// The smooth part of a bow's light on its bright side (its fringes averaged out).
fn bow_mean(z: f64) -> f64 {
    if z <= 0.0 {
        0.0
    } else {
        1.0 / (2.0 * PI * z.max(1.0).sqrt())
    }
}

fn wavelength(i: usize) -> f64 {
    400.0 + 300.0 * i as f64 / (WAVES - 1) as f64
}

/// Spectral weight: sunlight as the eye sees it, strongest in the green.
fn weight(nm: f64) -> f64 {
    (-((nm - 560.0) / 120.0).powi(2)).exp()
}

/// Per angle: bow light above the sky, its hue (a `Phase` level) and the smooth sky light.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Sample {
    pub bow: f32,
    pub hue: u8,
    pub sky: f32,
}

/// The light at `θ` (rad) from the antisolar point, summed over wavelengths.
pub(crate) fn light_at(theta: f64) -> Sample {
    let (mut bow_sum, mut sky, mut norm) = (0.0, 0.0, 0.0);
    for i in 0..WAVES {
        let nm = wavelength(i);
        let scale = AIRY * (DROP / nm).powf(2.0 / 3.0);
        let w = weight(nm);
        let z1 = scale * (drawn_angle(nm, 1) - theta);
        let z2 = scale * (theta - drawn_angle(nm, 2));
        sky += w * (bow_mean(z1) + SECONDARY * bow_mean(z2));
        // The bow is as bright as the strongest band here: every wavelength's main
        // fringe is as bright as the next, so the bow is solid from red to violet.
        bow_sum = f64::max(
            bow_sum,
            w.sqrt() * ((bow(z1) - bow_mean(z1)) + SECONDARY * (bow(z2) - bow_mean(z2))),
        );
        norm += w;
    }
    let (nm, t) = dominant_nm(theta);
    Sample {
        // Beyond the red rim the bow ends within a short fade.
        bow: (bow_sum * ease::smoothstep(1.0 + t / 0.06)) as f32,
        hue: hue_level(nm),
        sky: (sky / norm) as f32,
    }
}

/// Airy maxima `Ai(−z)²`: the main bow and the first two supernumeraries.
const FRINGES: [f64; 3] = [1.019, 3.248, 4.820];

/// The wavelength whose bright fringe lies at `θ`: each wavelength's main maximum sits
/// `z₁` Airy units inside its Descartes angle (outside, for the secondary), red outermost
/// in the primary and innermost in the secondary. Between the samples the angle is
/// interpolated; inside the primary the nearest supernumerary repeats the sequence.
pub(crate) fn dominant_nm(theta: f64) -> (f64, f64) {
    let peak = |nm: f64, k: u32, z: f64| {
        let scale = AIRY * (DROP / nm).powf(2.0 / 3.0);
        let base = drawn_angle(nm, k);
        if k == 1 {
            base - z / scale
        } else {
            base + z / scale
        }
    };
    // Position `t` along red (0) → violet (1) of one fringe family, unclamped.
    // The red end is taken at the outer edge of red's lobe (half an Airy unit beyond its
    // peak), so red is the band's rim rather than a third of its width.
    let along = |k: u32, z: f64| {
        let rim = RED_RIM / (AIRY * (DROP / 700.0).powf(2.0 / 3.0));
        let outward = if k == 1 { rim } else { -rim };
        let (red, violet) = (peak(700.0, k, z) + outward, peak(400.0, k, z));
        (theta - red) / (violet - red)
    };
    let secondary = theta > 0.5 * (peak(400.0, 1, 0.0) + peak(700.0, 2, 0.0));
    let t = if secondary {
        along(2, FRINGES[0])
    } else {
        // The family whose red-to-violet span is nearest.
        FRINGES
            .iter()
            .map(|&z| along(1, z))
            .min_by(|a, b| {
                let out = |t: f64| (t - t.clamp(0.0, 1.0)).abs();
                out(*a).total_cmp(&out(*b))
            })
            .unwrap_or(0.0)
    };
    (700.0 - 300.0 * t.clamp(0.0, 1.0), t)
}

/// `Phase` level of a wavelength: 700 nm → 0 (red) through 400 nm → 200 (violet), never
/// wrapping back to red.
pub(crate) fn hue_level(nm: f64) -> u8 {
    // The spectral colours' hues (OKLab degrees), red to violet: the loop's hue at level
    // `l` is `30° + 360° · l / 256`, so this places yellow and green where the eye sees
    // them instead of spreading red and orange over half the band.
    const HUES: [(f64, f64); 13] = [
        (700.0, 30.0),
        (640.0, 34.0),
        (610.0, 46.0),
        (590.0, 66.0),
        (575.0, 95.0),
        (560.0, 118.0),
        (540.0, 138.0),
        (520.0, 150.0),
        (500.0, 190.0),
        (480.0, 232.0),
        (460.0, 260.0),
        (430.0, 284.0),
        (400.0, 300.0),
    ];
    let nm = nm.clamp(400.0, 700.0);
    let k = HUES
        .iter()
        .position(|&(l, _)| l <= nm)
        .unwrap_or(HUES.len() - 1)
        .max(1);
    let ((l0, h0), (l1, h1)) = (HUES[k - 1], HUES[k]);
    let h = h0 + (h1 - h0) * (l0 - nm) / (l0 - l1);
    ((h - 30.0) / 360.0 * 256.0).round().clamp(0.0, 255.0) as u8
}

#[derive(Debug, Clone, Copy)]
struct Drop {
    x: f64,
    y: f64,
    speed: f64,
    level: u8,
}

#[derive(Debug, Default)]
pub struct Rainbow {
    /// Light per `cos θ` bin; built once.
    lut: Vec<Sample>,
    /// Per pixel column: `cos(az)`; per pixel row: `cos(el)`, `sin(el)`.
    cos_az: Vec<f64>,
    el: Vec<(f64, f64)>,
    /// Per pixel: land cover (0..1).
    land: Vec<f32>,
    horizon: f64,
    picture: Pixels,
    rain: ParticlePool<Drop>,
    streaks: Dots,
    /// Canvas size (cells).
    cells: (f64, f64),
    t: f64,
}

impl Rainbow {
    #[cfg(test)]
    fn lut_at(&self, c: f64) -> Sample {
        let i = (c.clamp(0.0, 1.0) * (LUT - 1) as f64).round() as usize;
        self.lut.get(i).copied().unwrap_or_default()
    }

    /// Sun elevation (rad) and the rain curtain's strength at loop time `t`.
    pub(crate) fn sky_state(t: f64) -> (f64, f64) {
        // The scene opens with the shower already under way, past the seam.
        let p = (t / LOOP + 0.12).rem_euclid(1.0);
        let elev = SUN_ELEV.0 + (SUN_ELEV.1 - SUN_ELEV.0) * p;
        // The shower thins at the loop's seam, so the Sun's jump back is never seen, and
        // swells and ebbs in between.
        let seam = ease::smoothstep(p / 0.1) * ease::smoothstep((1.0 - p) / 0.1);
        let swell = 0.78 + 0.22 * (TAU * 2.0 * p).cos_det();
        (elev.to_radians(), seam * swell)
    }

    fn paint(&mut self) {
        let (elev, rain) = Self::sky_state(self.t);
        let (se, ce) = elev.sin_cos_det();
        let ps = self.picture.pixel_size();
        let w = usize::from(ps.width());
        let horizon = self.horizon;
        // cos θ of the primary's drawn middle (under its solid band): nearer is inside.
        let inside = drawn_angle(560.0, 1).cos_det();
        let (lut, cos_az, el, land) = (&self.lut, &self.cos_az, &self.el, &self.land);
        let lut_at = |c: f64| {
            let i = (c.clamp(0.0, 1.0) * (LUT - 1) as f64).round() as usize;
            lut.get(i).copied().unwrap_or_default()
        };
        self.picture.shade(|x, y| {
            let i = y as usize * w + x as usize;
            let cover = f64::from(land.get(i).copied().unwrap_or(1.0));
            if cover >= 0.999 {
                return Ink::BLANK;
            }
            let (Some(&ca), Some(&(sin_el, cos_el))) = (cos_az.get(x as usize), el.get(y as usize))
            else {
                return Ink::BLANK;
            };
            // cos θ to the antisolar point, which sits `elev` below the horizon ahead.
            let c = cos_el * ca * ce - sin_el * se;
            let s = lut_at(c);
            let open = 1.0 - cover;
            let noise = hash01(cell_key(x, y) ^ 0xb0);
            let bow = ((f64::from(s.bow) - BOW_NONE) / (BOW_FULL - BOW_NONE)).clamp(0.0, 1.0);
            // Solid where the bow is strong; its faint fringes and supernumeraries thin out
            // pixel by pixel (the scattered pastel of real ones).
            let strength = bow * rain * open;
            if strength >= 0.3 || strength > 0.08 + 0.5 * noise {
                return Ink::new(Role::Phase, s.hue);
            }
            let low = ((f64::from(y) + 0.5) / horizon.max(1.0)).clamp(0.0, 1.0);
            let base = SKY_BASE.0 + (SKY_BASE.1 - SKY_BASE.0) * low * low;
            // Inside the primary the sunlit rain glows (`Starlight`, brightest next to the
            // bow); beyond it the dark slate rain curtain (`Cobalt`), darkest in Alexander's
            // band. The hue changes behind the bow itself.
            let lit = SKY_SCATTER * f64::from(s.sky) * rain;
            let (ramp, level) = if c > inside {
                (Ramp::Starlight, INNER_GLOW * rain.sqrt() + 1.6 * lit)
            } else {
                (Ramp::Cobalt, base + 0.35 * lit)
            };
            Ink::new(
                Role::Ramp(ramp),
                sciart_core::ink::level((level * open).max(12.0)),
            )
        });
    }

    fn rasterise_rain(&mut self) {
        self.streaks.clear();
        let (_, rain) = Self::sky_state(self.t);
        let horizon_cells = self.horizon / 2.0;
        for (k, d) in self.rain.iter().enumerate() {
            // The curtain thins by dropping streaks, a fixed share of them.
            if hash01(k as u64 ^ 0x5a) > rain {
                continue;
            }
            if d.y > horizon_cells {
                continue;
            }
            let (x, y) = Dots::from_cells(d.x, d.y);
            let len = 3.0;
            self.streaks.line(
                x,
                y,
                x - SLANT * len,
                y - len,
                Ink::new(Role::Ramp(Ramp::Starlight), d.level),
            );
        }
    }
}

impl Generator for Rainbow {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.lut.is_empty() {
            self.lut = (0..LUT)
                .map(|i| light_at((i as f64 / (LUT - 1) as f64).acos()))
                .collect();
        }
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let (px_w, px_h) = (w as f64, 2.0 * h as f64);
        self.cells = (px_w, h as f64);
        self.horizon = HORIZON_ROW * px_h;
        let per_deg = (self.horizon / SKY_SPAN).max(0.05);
        let f = per_deg * 180.0 / PI;
        self.cos_az = (0..w)
            .map(|x| ((x as f64 + 0.5 - 0.5 * px_w) / f).cos_det())
            .collect();
        self.el = (0..2 * h)
            .map(|y| ((self.horizon - (y as f64 + 0.5)) / f).sin_cos_det())
            .collect();
        let hills = Skyline::new(&mut ctx.rng, w, per_deg, HILLS);
        self.land.clear();
        for y in 0..2 * h {
            for x in 0..w {
                self.land
                    .push(hills.cover(x as i32, y as i32, self.horizon));
            }
        }
        self.picture.resize(size);
        self.streaks.resize(size);
        let n = ((RAIN_DENSITY * size.area() as f64) as usize).clamp(1, RAIN_CAP);
        self.rain = ParticlePool::with_capacity(RAIN_CAP);
        for _ in 0..n {
            self.rain.spawn(Drop {
                x: ctx.rng.random_range(0.0..px_w),
                y: ctx.rng.random_range(0.0..h as f64),
                speed: ctx.rng.random_range(RAIN_SPEED.0..RAIN_SPEED.1),
                level: ctx.rng.random_range(RAIN_LEVEL.0..RAIN_LEVEL.1) as u8,
            });
        }
        self.paint();
        self.rasterise_rain();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        let (w, h) = self.cells;
        for d in self.rain.iter_mut() {
            d.y += d.speed * dt;
            d.x += SLANT * d.speed * dt / 2.0;
            if d.y >= h {
                d.y -= h;
            }
            if d.x >= w {
                d.x -= w;
            }
        }
        self.paint();
        self.rasterise_rain();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.picture.blit(canvas, |_, _, top, bottom| {
            match (top.role == Role::Phase, bottom.role == Role::Phase) {
                (true, true) => Some(('#', top)),
                (true, false) => Some(('+', top)),
                (false, true) => Some(('+', bottom)),
                _ => None,
            }
        });
        // Streaks are braille-fine; as ASCII dot counts they would bury the bow.
        if canvas.mode() == GlyphMode::Unicode {
            self.streaks.blit_over(canvas, &self.picture);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(a: f64) -> f64 {
        a.to_degrees()
    }

    #[test]
    fn bow_a1_descartes_angles() {
        let na = water_n(589.0);
        let p = deg(bow_angle(na, 1));
        let s = deg(bow_angle(na, 2));
        assert!((p - 42.0).abs() <= 0.5, "primary {p}");
        assert!((50.5..=51.5).contains(&s), "secondary {s}");
    }

    #[test]
    fn bow_a2_red_outside_in_the_primary_inside_in_the_secondary() {
        let (red, violet) = (water_n(700.0), water_n(400.0));
        assert!(bow_angle(red, 1) > bow_angle(violet, 1));
        assert!(bow_angle(red, 2) < bow_angle(violet, 2));
        // And in the drawn hues: the outer edge of the primary is redder (lower level).
        let hue = |d: f64| light_at(d.to_radians()).hue;
        assert!(hue(42.2) < hue(40.9), "{} vs {}", hue(42.2), hue(40.9));
        assert!(hue(50.6) < hue(53.0), "{} vs {}", hue(50.6), hue(53.0));
    }

    #[test]
    fn bow_a3_alexanders_band_is_darker() {
        let mean = |a: f64, b: f64| {
            let n = 40;
            (0..n)
                .map(|i| {
                    let s = light_at((a + (b - a) * f64::from(i) / f64::from(n)).to_radians());
                    f64::from(s.sky + s.bow)
                })
                .sum::<f64>()
                / f64::from(n)
        };
        let band = mean(44.0, 49.0);
        assert!(band < mean(35.0, 39.0), "{band} vs inside");
        assert!(band < mean(55.0, 59.0), "{band} vs outside");
    }

    #[test]
    fn bow_a4_supernumeraries_lie_inside_the_primary() {
        let n = water_n(550.0);
        let peak = deg(bow_angle(n, 1));
        let scale = AIRY * (DROP / 550.0f64).powf(2.0 / 3.0);
        let at = |d: f64| bow(scale * (bow_angle(n, 1) - d.to_radians()));
        let mut maxima = Vec::new();
        let mut d = 30.0;
        while d < 48.0 {
            if at(d) > at(d - 0.01) && at(d) > at(d + 0.01) {
                maxima.push(d);
            }
            d += 0.01;
        }
        assert!(maxima.len() >= 3, "{maxima:?}");
        for m in &maxima {
            assert!(*m < peak, "{m} vs {peak}");
        }
    }

    #[test]
    fn bow_a5_rain_pool_stays_within_capacity() {
        for size in [Size::new(80, 24), Size::new(500, 150)] {
            let mut ctx = Ctx::new(42, size);
            let mut g = Rainbow::default();
            g.resize(size, &mut ctx);
            for _ in 0..120 {
                g.update(SIM_DT, &mut ctx);
            }
            assert!(g.rain.len() <= RAIN_CAP && g.rain.capacity() == RAIN_CAP);
        }
    }

    #[test]
    fn bow_airy_matches_known_values() {
        assert!((airy(0.0) - 0.355_028).abs() < 1e-6);
        assert!((airy(-1.018_793) - 0.535_656_6).abs() < 1e-5);
        assert!((airy(-7.0) - 0.184_08).abs() < 3e-3);
    }

    #[test]
    fn bow_lut_lookup_matches_direct() {
        let size = Size::new(40, 12);
        let mut ctx = Ctx::new(1, size);
        let mut g = Rainbow::default();
        g.resize(size, &mut ctx);
        let c = 42f64.to_radians().cos_det();
        let s = g.lut_at(c);
        assert!(s.bow > 0.0);
    }
}
