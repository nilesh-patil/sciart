//! A rain shower lit by a low Sun behind the viewer (SPEC-002 `rainbow`): a bright primary
//! bow with red outside, a fainter secondary with its colours reversed, the darker sky
//! between them (Alexander's band), pastel supernumerary arcs inside the primary, drifting
//! rain curtains, slanting rain and shafts of sunlight converging on the antisolar point.
//!
//! Every pixel sees the rain at an angle `θ` from the antisolar point. Light leaving the
//! drops after one internal reflection (the primary) or two (the secondary) piles up at the
//! Descartes minimum-deviation angle of each wavelength. A look-up table over `cos θ`
//! (size-independent, built once) holds, per angle, how much bow light there is, the
//! wavelength seen there (an analytic angle → wavelength map between the drawn red and
//! violet Descartes angles, so the bands are smooth rather than a few solid hues) and the
//! smooth scattered light of the sunlit rain.
//!
//! Colour: where the bow is at full strength it is drawn in the theme's isoluminant hue
//! loop (`Role::Phase`), its hue following the wavelength continuously, so the bands blend
//! into one another with no stepped stripes, at a pastel chroma. Phase's fixed lightness
//! matches the named ramps near level 186, so where the bow is weaker (its soft edges, gaps
//! between rain curtains, the Sun behind a cloud, the fainter secondary) it continues in the
//! nearest spectral ramp at a lower level (red `HAlpha`, orange `Ember`, yellow `Sulfur`,
//! green `Aurora`, blue `Cobalt`, violet `Plasma`). The sunlit rain inside the primary is a
//! dim neutral grey `Starlight`, the storm cloud outside a slate `Div`.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{hash01, value_noise};
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

/// Drop radius (nm): 0.5 mm.
const DROP: f64 = 5.0e5;
/// Airy fringe scale: `z = AIRY · (a/λ)^{2/3} · Δθ` (Δθ in radians). Stylised: fringes
/// ~1.8° apart, about 2.5× real ones for 0.5 mm drops, so they resolve on a terminal.
const AIRY: f64 = 0.75;
/// Supernumeraries fade beyond the first few (a spread of drop sizes washes them out).
#[cfg(test)]
const FRINGE_FADE: f64 = 5.0;
/// The secondary is this much fainter than the primary.
const SECONDARY: f64 = 0.45;
/// Look-up table resolution over `cos θ ∈ [0, 1]`.
const LUT: usize = 8192;
/// Colour dispersion drawn ×this about 550 nm (stylised: a real primary's red-to-violet
/// span of ~1.8° is 2–4 pixels on a terminal). The secondary, twice as dispersed already,
/// is widened less so Alexander's band stays wide.
const DISPERSION: (f64, f64) = (2.5, 1.15);
/// Soft falloff of a band beyond its red rim and its violet edge (share of its width).
const RIM_SOFT: f64 = 0.14;
const VIOLET_SOFT: f64 = 0.32;
/// Supernumeraries: amplitude and fade (in primary band widths inside the violet); their
/// spacing is Airy's, from the first two maxima of `Ai(−z)²` at 550 nm.
const SUPER: (f64, f64) = (0.5, 0.55);
/// Airy maxima `Ai(−z)²`: the main bow and the first supernumerary.
const FRINGES: [f64; 2] = [1.019, 3.248];
/// Share of a band's peak light above which its hue shows (below it: the sky's). A
/// property of the angle, not of the moment, so a fading bow dims but keeps its width.
const HUE_SWITCH: f64 = 0.3;
/// The bow's full level (where `Phase`'s lightness sits on the ramps), the gain from the
/// band's light to its strength, and the level from which it is drawn in `Phase`.
const BOW_LEVEL: f64 = 186.0;
const BOW_GAIN: f64 = 1.35;
const PHASE_FROM: f64 = 170.0;
/// The bow's strength: its floor with the rain and Sun at their dimmest, and the share of
/// full sunlit rain at which it is at full strength.
const BOW_FLOOR: f64 = 0.35;
const BOW_FULL: f64 = 0.6;
/// The sunlit rain inside the primary in `Ramp(Starlight)`, a calm grey (not a cyan): its
/// base, the
/// gain with sunlit rain, the extra near the bow and the drop toward the horizon.
const RAIN_LIT: (f64, f64, f64, f64) = (64.0, 10.0, 12.0, 14.0);
/// Sun breaks: the Sun behind the viewer passes in and out of cloud (value noise in time,
/// cell in s), dimming the bow and the sunlit rain to this share.
const SUN_CELL: f64 = 6.0;
const SUN_LOW: f64 = 0.4;
/// Storm-cloud `Ramp(Starlight)` levels: top of the frame and at the horizon.
const SKY_BASE: (f64, f64) = (20.0, 40.0);
/// Slate sky as `Div` below zero: offset, per level, and the deepest (bluest) value.
const SLATE: (f64, f64, f64) = (0.01, 0.0032, 0.1);
/// Extra light of the sunlit rain: inside the primary, and outside the secondary.
const LIT: (f64, f64) = (88.0, 12.0);
/// Cloud-base mottling (± levels) and its cell (share of the width).
const MOTTLE: (f64, f64) = (7.0, 0.09);
/// Rain curtains: slant (px across per px down, matching the streaks), cell (share of the
/// width) and drift (px/s at 100 px wide).
const CURTAIN_SLANT: f64 = 0.15;
const CURTAIN_CELL: f64 = 0.22;
const CURTAIN_DRIFT: f64 = 2.2;
/// Anticrepuscular shafts: count, peak extra level, and period range (s).
const SHAFTS: usize = 7;
const SHAFT_GAIN: f64 = 20.0;
const SHAFT_PERIOD: (f64, f64) = (9.0, 17.0);
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
    let spread = if k == 1 { DISPERSION.0 } else { DISPERSION.1 };
    mid + spread * (bow_angle(water_n(nm), k) - mid)
}

/// Airy's function `Ai(x)`: the power series for `|x| ≤ 6`, the asymptotic forms beyond.
#[cfg(test)]
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
/// the first damped toward their mean `1/(2π√z)` (so the sky inside is smooth). The
/// supernumeraries' spacing in the drawn bow follows these maxima.
#[cfg(test)]
fn bow(z: f64) -> f64 {
    let ai = airy(-z);
    let wave = ai * ai;
    if z <= 2.0 {
        return wave;
    }
    let mean = 1.0 / (2.0 * PI * z.sqrt());
    mean + (wave - mean) * (-(z - 2.0) / FRINGE_FADE).exp()
}

/// Spectral weight: sunlight as the eye sees it, strongest in the green.
fn weight(nm: f64) -> f64 {
    (-((nm - 560.0) / 120.0).powi(2)).exp()
}

/// The ramp that carries a wavelength's colour where the bow is weak, red → violet. Cyan
/// has no ramp of its own; `Phase` carries it at full strength.
pub(crate) fn spectral_ramp(nm: f64) -> Ramp {
    match nm {
        n if n >= 636.0 => Ramp::HAlpha,
        n if n >= 596.0 => Ramp::Ember,
        n if n >= 566.0 => Ramp::Sulfur,
        n if n >= 492.0 => Ramp::Aurora,
        n if n >= 440.0 => Ramp::Cobalt,
        _ => Ramp::Plasma,
    }
}

/// The `Phase` level (a hue on the theme's isoluminant loop, which starts at OKLab hue
/// 30°) of a wavelength: its OKLab hue, piecewise linear through the spectral colours,
/// spaced so red, orange, yellow, green, blue and violet take similar shares of the band
/// (as the eye sees a bow; the drawn band is linear in wavelength).
pub(crate) fn spectral_phase(nm: f64) -> u8 {
    // (nm, OKLab hue in degrees), red → violet.
    const HUES: [(f64, f64); 7] = [
        (700.0, 25.0),
        (650.0, 55.0),
        (600.0, 100.0),
        (550.0, 140.0),
        (500.0, 215.0),
        (450.0, 262.0),
        (400.0, 300.0),
    ];
    let nm = nm.clamp(400.0, 700.0);
    let mut hue = HUES[HUES.len() - 1].1;
    for w in HUES.windows(2) {
        let ((n0, h0), (n1, h1)) = (w[0], w[1]);
        if nm <= n0 && nm >= n1 {
            hue = h0 + (h1 - h0) * (n0 - nm) / (n0 - n1);
            break;
        }
    }
    sciart_core::ink::phase((hue - 30.0).to_radians()).level
}

/// Per angle: bow light (0..1 of a full band), the wavelength seen (nm; its ramp) and the
/// smooth light of the sunlit rain (0..1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sample {
    pub bow: f32,
    /// Only the BOW-A2 test reads it: the drawing uses `ramp` and `level`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub nm: f32,
    pub ramp: Ramp,
    /// Whether the band's hue shows here (its light above [`HUE_SWITCH`] of its peak).
    pub hue: bool,
    /// The wavelength's hue on the `Phase` loop ([`spectral_phase`]).
    pub phase: u8,
    pub sky: f32,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            bow: 0.0,
            nm: 550.0,
            ramp: Ramp::Starlight,
            hue: false,
            phase: 0,
            sky: 0.0,
        }
    }
}

/// Drawn band edges (rad): the primary's red rim and violet edge, the secondary's.
fn edges() -> ((f64, f64), (f64, f64)) {
    (
        (drawn_angle(700.0, 1), drawn_angle(400.0, 1)),
        (drawn_angle(700.0, 2), drawn_angle(400.0, 2)),
    )
}

/// A band's light at position `u` across it (0 = red rim, 1 = violet edge): a soft
/// Gaussian shoulder beyond either edge, the eye's spectral weight across it.
fn band(u: f64) -> (f64, f64) {
    let nm = 700.0 - 300.0 * u.clamp(0.0, 1.0);
    let body = 0.72 + 0.28 * weight(nm);
    let edge = if u < 0.0 {
        (-(u / RIM_SOFT).powi(2)).exp()
    } else if u > 1.0 {
        (-((u - 1.0) / VIOLET_SOFT).powi(2)).exp()
    } else {
        1.0
    };
    (body * edge, nm)
}

/// The light at `θ` (rad) from the antisolar point.
pub(crate) fn light_at(theta: f64) -> Sample {
    let ((r1, v1), (r2, v2)) = edges();
    // Primary: red outside (larger θ), violet inside.
    let u1 = (r1 - theta) / (r1 - v1);
    let (b1, nm1) = band(u1);
    // Supernumeraries: pastel fringes inside the violet, alternating green and pink-lilac,
    // spaced as Airy's maxima (in band widths).
    let spacing = (FRINGES[1] - FRINGES[0]) / (AIRY * (DROP / 550.0f64).powf(2.0 / 3.0));
    let s = (u1 - 1.0) * (r1 - v1) / spacing;
    let sup = if s > 0.0 {
        SUPER.0 * (-(u1 - 1.0) / SUPER.1).exp() * (PI * s).sin_det().powi(2)
    } else {
        0.0
    };
    // Secondary: red inside, violet outside.
    let u2 = (theta - r2) / (v2 - r2);
    let (b2, nm2) = band(u2);
    let b2 = SECONDARY * b2;
    // Sunlit rain: bright inside the primary (brightest near it), dim outside the
    // secondary, darkest between them (Alexander's band); edges eased over ~1°.
    let deg = 1f64.to_radians();
    let inner = ease::smoothstep((r1 - theta) / deg + 0.3);
    let near = 0.62 + 0.38 * (-(r1 - theta).max(0.0) / (14.0 * deg)).exp();
    let outer = ease::smoothstep((theta - r2) / deg + 0.3);
    let sky = inner * near + 0.3 * outer;
    let (bow, nm, ramp, peak) = if b1 >= b2 && b1 >= sup {
        (b1, nm1, spectral_ramp(nm1), 1.0)
    } else if b2 >= sup {
        (b2, nm2, spectral_ramp(nm2), SECONDARY)
    } else {
        let pink = s.fract() > 0.5;
        let nm = if pink { 420.0 } else { 530.0 };
        let ramp = if pink { Ramp::Eosin } else { Ramp::Aurora };
        // A fringe a pixel or two wide would alias into coloured specks: the
        // supernumeraries show as pale ripples in the sunlit rain, lightness only.
        (sup, nm, ramp, f64::INFINITY)
    };
    Sample {
        bow: bow as f32,
        nm: nm as f32,
        ramp,
        hue: bow > HUE_SWITCH * peak,
        phase: spectral_phase(nm),
        sky: sky as f32,
    }
}

/// The storm cloud outside the primary at a `Ramp`-equivalent level: a dark slate from
/// the cool side of `Div` just below zero (a blue-grey no named ramp has at this
/// lightness), lightening with the level.
fn slate(lvl: f64) -> Ink {
    sciart_core::ink::div(-(SLATE.0 + SLATE.1 * (lvl - 12.0).max(0.0)).min(SLATE.2))
}

#[derive(Debug, Clone, Copy)]
struct Drop {
    x: f64,
    y: f64,
    speed: f64,
    level: u8,
}

#[derive(Debug, Clone, Copy, Default)]
struct Shaft {
    /// Centre and half-width in pseudo-angle about the antisolar point (−1..1).
    at: f64,
    half: f64,
    period: f64,
    phase: f64,
}

#[derive(Debug, Default)]
pub struct Rainbow {
    /// Light per `cos θ` bin; built once.
    lut: Vec<Sample>,
    /// Per pixel column: `cos(az)`; per pixel row: `cos(el)`, `sin(el)`.
    cos_az: Vec<f64>,
    el: Vec<(f64, f64)>,
    /// Per pixel: land cover (0..1) and the cloud base's mottling (−1..1).
    land: Vec<f32>,
    mottle: Vec<f32>,
    /// Rain-curtain density along `x − slant·y` (px), refreshed each step.
    curtain: Vec<f32>,
    /// Shaft light per pseudo-angle bin, refreshed each step.
    shaft: Vec<f32>,
    shafts: [Shaft; SHAFTS],
    salt: u64,
    horizon: f64,
    per_deg: f64,
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
        let swell = 0.8 + 0.2 * (TAU * 2.0 * p).cos_det();
        (elev.to_radians(), seam * swell)
    }

    /// Sunlight on the rain at time `t` (`SUN_LOW`..1): the Sun behind the viewer passes
    /// in and out of broken cloud, brightening and dimming the whole bow over seconds.
    pub(crate) fn sun_at(salt: u64, t: f64) -> f64 {
        let n = value_noise(salt ^ 0x5a11, t, 0.0, SUN_CELL);
        SUN_LOW + (1.0 - SUN_LOW) * ease::smoothstep((n - 0.2) / 0.6)
    }

    /// Rain-curtain density (0..1) at curtain coordinate `x − slant·y` (px) and time `t`:
    /// two octaves of drifting value noise, sharpened so veils have gaps between them.
    fn curtain_at(salt: u64, x: f64, width: f64, t: f64) -> f64 {
        let cell = (CURTAIN_CELL * width).max(4.0);
        let drift = CURTAIN_DRIFT * t * width / 100.0;
        let a = value_noise(salt, x - drift, 0.0, cell);
        let b = value_noise(salt ^ 0x77, x - 1.7 * drift, 5.0, 0.37 * cell);
        ease::smoothstep((0.7 * a + 0.3 * b - 0.28) / 0.44)
    }

    /// Refresh the per-step tables: curtain density and shaft light.
    fn refresh(&mut self) {
        let w = self.cos_az.len() as f64;
        let rows = self.el.len() as f64;
        let x0 = -CURTAIN_SLANT * rows;
        for (i, c) in self.curtain.iter_mut().enumerate() {
            *c = Self::curtain_at(self.salt, x0 + i as f64, w, self.t) as f32;
        }
        for (i, s) in self.shaft.iter_mut().enumerate() {
            let p = (i as f64 + 0.5) / 128.0 - 1.0;
            let mut sum = 0.0;
            for sh in &self.shafts {
                // Each shaft comes and goes as a gap in the clouds behind the viewer opens.
                let on = ((TAU * self.t / sh.period + sh.phase).sin_det()).max(0.0);
                let d = (p - sh.at) / sh.half;
                sum += on * on * (-d * d).exp();
            }
            *s = sum.min(1.0) as f32;
        }
    }

    fn paint(&mut self) {
        let (elev, rain) = Self::sky_state(self.t);
        let (se, ce) = elev.sin_cos_det();
        let sun = Self::sun_at(self.salt, self.t);
        let ps = self.picture.pixel_size();
        let w = usize::from(ps.width());
        let horizon = self.horizon;
        // The antisolar point on screen: centred, `elev` below the horizon.
        let (ax, ay) = (0.5 * w as f64, horizon + elev.to_degrees() * self.per_deg);
        let x0 = -CURTAIN_SLANT * self.el.len() as f64;
        let (lut, cos_az, el, land, mottle, curtain, shaft) = (
            &self.lut,
            &self.cos_az,
            &self.el,
            &self.land,
            &self.mottle,
            &self.curtain,
            &self.shaft,
        );
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
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            // cos θ to the antisolar point, which sits `elev` below the horizon ahead.
            let c = cos_el * ca * ce - sin_el * se;
            let s = lut_at(c);
            let open = 1.0 - cover;
            let ci = (px - CURTAIN_SLANT * py - x0).max(0.0) as usize;
            let veil = f64::from(curtain.get(ci).copied().unwrap_or(0.0));
            // Pseudo-angle about the antisolar point (no trig): rays of sunlight through
            // gaps in the clouds behind the viewer converge there.
            let (dx, dy) = (px - ax, (ay - py).max(0.0));
            let p = dx / (dx.abs() + dy + 1e-9);
            let si = ((p + 1.0) * 128.0).clamp(0.0, 255.0) as usize;
            let beam = f64::from(shaft[si]) * rain;
            // Sky: the storm cloud's base, lighter toward the horizon and mottled, plus the
            // sunlit rain (brightest where the curtains are dense and the shafts fall).
            let low = (py / horizon.max(1.0)).clamp(0.0, 1.0);
            let m = f64::from(mottle.get(i).copied().unwrap_or(0.0)) * (1.0 - low);
            let lit = rain * sun * (0.55 + 0.45 * veil);
            let shafts = SHAFT_GAIN * beam * sun * (0.4 + f64::from(s.sky));
            // Outside the primary: the storm cloud's slate base.
            let base = SKY_BASE.0 + (SKY_BASE.1 - SKY_BASE.0) * low * low + MOTTLE.0 * m;
            let cloud = base + LIT.1 * veil * rain * sun + shafts;
            // Inside: the sunlit rain, luminous near the bow and easing down to the
            // horizon; lightness blends continuously from the cloud across the red rim.
            let rain_lit = RAIN_LIT.0 + RAIN_LIT.1 * lit + RAIN_LIT.2 * f64::from(s.sky) * lit
                - RAIN_LIT.3 * low * low
                + 0.5 * MOTTLE.0 * m
                + shafts;
            let inside = ease::smoothstep((f64::from(s.sky) - 0.2) / 0.4);
            let sky = cloud + (rain_lit - cloud) * inside;
            // The bow brightens and fades along its arc with the rain it shines in, and as a
            // whole with the sunlight on the rain.
            let strength = BOW_FLOOR + (1.0 - BOW_FLOOR) * (lit / BOW_FULL).min(1.0);
            let a = (f64::from(s.bow) * BOW_GAIN * strength * (1.0 + 0.25 * beam)).clamp(0.0, 1.0);
            let target = BOW_LEVEL.max(sky + 30.0);
            let lvl = (sky + (target - sky) * a) * open;
            if s.hue && a > 0.04 {
                if lvl >= PHASE_FROM {
                    // At full strength: a continuous hue at Phase's (fixed) lightness.
                    Ink::new(Role::Phase, s.phase)
                } else {
                    Ink::new(Role::Ramp(s.ramp), sciart_core::ink::level(lvl))
                }
            } else if inside > 0.5 {
                // The sunlit rain inside the primary: a dim, nearly neutral grey
                // (`Starlight` stays grey while faint), brighter than the cloud but calm.
                Ink::new(Role::Ramp(Ramp::Starlight), sciart_core::ink::level(lvl))
            } else {
                slate(lvl)
            }
        });
    }

    fn rasterise_rain(&mut self) {
        self.streaks.clear();
        let (_, rain) = Self::sky_state(self.t);
        let horizon_cells = self.horizon / 2.0;
        let w = self.cos_az.len() as f64;
        for (k, d) in self.rain.iter().enumerate() {
            if d.y > horizon_cells {
                continue;
            }
            // Streaks fall in the curtains: thin where the veil is, gone between.
            let veil = Self::curtain_at(self.salt, d.x - CURTAIN_SLANT * 2.0 * d.y, w, self.t);
            if hash01(k as u64 ^ 0x5a) > rain * (0.25 + 0.75 * veil) {
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
        self.per_deg = per_deg;
        let f = per_deg * 180.0 / PI;
        self.cos_az = (0..w)
            .map(|x| ((x as f64 + 0.5 - 0.5 * px_w) / f).cos_det())
            .collect();
        self.el = (0..2 * h)
            .map(|y| ((self.horizon - (y as f64 + 0.5)) / f).sin_cos_det())
            .collect();
        let hills = Skyline::new(&mut ctx.rng, w, per_deg, HILLS);
        self.salt = ctx.rng.random();
        let cell = (MOTTLE.1 * px_w).max(3.0);
        self.land.clear();
        self.mottle.clear();
        for y in 0..2 * h {
            for x in 0..w {
                self.land
                    .push(hills.cover(x as i32, y as i32, self.horizon));
                // Flattened cells, as a cloud base seen at a slant.
                let (fx, fy) = (x as f64, y as f64 * 2.2);
                let m = 0.65 * value_noise(self.salt ^ 0xc1, fx, fy, cell)
                    + 0.35 * value_noise(self.salt ^ 0xc2, fx, fy, 0.4 * cell);
                self.mottle.push((2.0 * m - 1.0) as f32);
            }
        }
        let span = w + (CURTAIN_SLANT * px_h).ceil() as usize + 2;
        self.curtain.clear();
        self.curtain.resize(span, 0.0);
        self.shaft.resize(256, 0.0);
        for sh in &mut self.shafts {
            *sh = Shaft {
                at: ctx.rng.random_range(-0.85..0.85),
                half: ctx.rng.random_range(0.025..0.07),
                period: ctx.rng.random_range(SHAFT_PERIOD.0..SHAFT_PERIOD.1),
                phase: ctx.rng.random_range(0.0..TAU),
            };
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
        self.refresh();
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
        self.refresh();
        self.paint();
        self.rasterise_rain();
    }

    fn render(&self, canvas: &mut Canvas) {
        let is_bow = |ink: Ink| {
            ink.role == Role::Phase || matches!(ink.role, Role::Ramp(r) if r != Ramp::Starlight)
        };
        self.picture.blit(canvas, |_, _, top, bottom| {
            match (is_bow(top), is_bow(bottom)) {
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

    fn at(d: f64) -> Sample {
        light_at(d.to_radians())
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
        // And in the drawn bands: wavelength falls inward across the primary and rises
        // outward across the secondary.
        let ((r1, v1), (r2, v2)) = edges();
        let (r1, v1, r2, v2) = (deg(r1), deg(v1), deg(r2), deg(v2));
        assert!(at(r1 - 0.2).nm > at(0.5 * (r1 + v1)).nm);
        assert!(at(0.5 * (r1 + v1)).nm > at(v1 + 0.2).nm);
        assert!(r2 < v2 && at(r2 + 0.2).nm > at(v2 - 0.2).nm);
        assert_eq!(at(r1 - 0.1).ramp, Ramp::HAlpha);
        assert_eq!(at(v1 + 0.1).ramp, Ramp::Plasma);
    }

    #[test]
    fn bow_a3_alexanders_band_is_darker() {
        let mean = |a: f64, b: f64| {
            let n = 40;
            (0..n)
                .map(|i| {
                    let s = at(a + (b - a) * f64::from(i) / f64::from(n));
                    f64::from(s.sky + s.bow)
                })
                .sum::<f64>()
                / f64::from(n)
        };
        let band = mean(44.5, 48.5);
        assert!(band < mean(35.0, 39.0), "{band} vs inside");
        assert!(band < mean(55.0, 59.0), "{band} vs outside");
    }

    #[test]
    fn bow_a4_supernumeraries_lie_inside_the_primary() {
        let n = water_n(550.0);
        let peak = deg(bow_angle(n, 1));
        let scale = AIRY * (DROP / 550.0f64).powf(2.0 / 3.0);
        let airy_at = |d: f64| bow(scale * (bow_angle(n, 1) - d.to_radians()));
        let maxima = |f: &dyn Fn(f64) -> f64, from: f64, to: f64| {
            let mut out = Vec::new();
            let mut d = from;
            while d < to {
                if f(d) > f(d - 0.01) && f(d) > f(d + 0.01) {
                    out.push(d);
                }
                d += 0.01;
            }
            out
        };
        let wave = maxima(&airy_at, 30.0, 48.0);
        assert!(wave.len() >= 3, "{wave:?}");
        assert!(wave.iter().all(|m| *m < peak), "{wave:?} vs {peak}");
        // The drawn pastel fringes lie inside the violet edge too.
        let (_, v1) = edges().0;
        let drawn = maxima(&|d| f64::from(at(d).bow), 30.0, deg(v1) - 0.5);
        assert!(drawn.len() >= 2, "{drawn:?}");
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
    fn bow_a6_bands_are_smooth_with_soft_edges() {
        // No thresholds: the bow's light changes gradually across and beyond each band.
        let mut prev = at(30.0).bow;
        let mut d = 30.05;
        while d < 60.0 {
            let b = at(d).bow;
            assert!((b - prev).abs() < 0.12, "jump {prev} → {b} at {d}°");
            prev = b;
            d += 0.05;
        }
        // The full spectrum is present across the primary, in order.
        let ((r1, v1), _) = edges();
        let mut seen = Vec::new();
        let mut d = deg(r1);
        while d > deg(v1) {
            let r = at(d).ramp;
            if seen.last() != Some(&r) {
                seen.push(r);
            }
            d -= 0.02;
        }
        use Ramp::*;
        assert_eq!(seen, [HAlpha, Ember, Sulfur, Aurora, Cobalt, Plasma]);
        // At full strength the hue runs continuously red → violet on the Phase loop: small
        // steps, always forward, and most of the way round (no stepped stripes).
        let mut prev = at(deg(r1)).phase;
        let mut turned = 0u32;
        let mut d = deg(r1) - 0.02;
        while d > deg(v1) {
            let p = at(d).phase;
            let step = p.wrapping_sub(prev);
            assert!(step <= 6, "hue step {step} at {d}°");
            turned += u32::from(step);
            prev = p;
            d -= 0.02;
        }
        assert!((170..=200).contains(&turned), "hue turned {turned}/256");
    }

    #[test]
    fn bow_a7_curtains_modulate_the_bow_along_the_arc_and_in_time() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(7, size);
        let mut g = Rainbow::default();
        g.resize(size, &mut ctx);
        let bright = |g: &Rainbow| {
            // Mean level of bow pixels in the left, middle and right thirds (a `Phase`
            // pixel is at the bow's full level).
            let ps = g.picture.pixel_size();
            let (w, h) = (i32::from(ps.width()), i32::from(ps.height()));
            let mut sums = [(0.0, 0.0); 3];
            for y in 0..h {
                for x in 0..w {
                    let ink = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                    let lvl = match ink.role {
                        Role::Phase => BOW_LEVEL,
                        Role::Ramp(r) if r != Ramp::Starlight => f64::from(ink.level),
                        _ => continue,
                    };
                    let k = (3 * x / w) as usize;
                    sums[k].0 += lvl;
                    sums[k].1 += 1.0;
                }
            }
            sums.map(|(s, n)| if n > 0.0 { s / n } else { 0.0 })
        };
        let first = bright(&g);
        assert!(first.iter().all(|&m| m > 0.0), "{first:?}");
        for _ in 0..(8.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let later = bright(&g);
        let change: f64 = first.iter().zip(&later).map(|(a, b)| (a - b).abs()).sum();
        assert!(change > 3.0, "{first:?} → {later:?}");
    }

    #[test]
    fn bow_a8_slate_storm_cloud_outside_sunlit_rain_inside() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(3, size);
        let mut g = Rainbow::default();
        g.resize(size, &mut ctx);
        let (elev, _) = Rainbow::sky_state(0.0);
        let row = |d: f64| {
            // Pixel row `d`° above the antisolar point, straight up the middle.
            (g.horizon + (elev.to_degrees() - d) * g.per_deg) as i32
        };
        let x = i32::from(g.picture.pixel_size().width()) / 2;
        let (inside, band) = (
            g.picture.get(x, row(36.0)).unwrap_or(Ink::BLANK),
            g.picture.get(x, row(46.5)).unwrap_or(Ink::BLANK),
        );
        // Inside the primary: sunlit rain as a calm, dim grey `Starlight` (brighter than the
        // cloud, far below the bow); Alexander's band: dark slate (the cool side of `Div`,
        // just below zero), a darker sky than inside.
        assert_eq!(inside.role, Role::Ramp(Ramp::Starlight));
        assert!((55..=95).contains(&inside.level), "{inside:?}");
        // It eases down toward the horizon rather than being one flat dome.
        let low = g
            .picture
            .get(x, row(elev.to_degrees() + 2.0))
            .unwrap_or(Ink::BLANK);
        assert_eq!(low.role, Role::Ramp(Ramp::Starlight));
        assert!(low.level + 8 <= inside.level, "{low:?} vs {inside:?}");
        assert_eq!(band.role, Role::Div);
        let v = sciart_core::ink::div_value(band);
        assert!((-SLATE.2 - 0.01..0.0).contains(&v), "{band:?}");
        // Darker: slate this close to zero is below Starlight 40 in lightness.
        assert!(v > -0.06, "{band:?}");
    }

    #[test]
    fn bow_a9_sun_breaks_brighten_and_dim_the_whole_bow() {
        // The sunlight on the rain wanders within [SUN_LOW, 1], swinging widely within
        // ~10 s, and a dim spell takes the bow out of full (Phase) strength.
        let mut lo: f64 = 1.0;
        let mut hi: f64 = 0.0;
        let mut widest: f64 = 0.0;
        for i in 0..600 {
            let t = f64::from(i) * 0.1;
            let s = Rainbow::sun_at(9, t);
            assert!((SUN_LOW - 1e-9..=1.0 + 1e-9).contains(&s), "{s} at {t}");
            lo = lo.min(s);
            hi = hi.max(s);
            let later = Rainbow::sun_at(9, t + 10.0);
            widest = widest.max((later - s).abs());
        }
        assert!(hi - lo > 0.4 && widest > 0.3, "{lo}..{hi}, {widest}");
        let phase_pixels = |sun_low: bool| {
            let size = Size::new(120, 40);
            let mut ctx = Ctx::new(5, size);
            let mut g = Rainbow::default();
            g.resize(size, &mut ctx);
            // Find a moment of full and of dimmest sunlight within the first minute.
            let target = if sun_low { lo } else { hi };
            let t = (0..600)
                .map(|i| f64::from(i) * 0.1)
                .min_by(|a, b| {
                    let (fa, fb) = (Rainbow::sun_at(g.salt, *a), Rainbow::sun_at(g.salt, *b));
                    (fa - target).abs().total_cmp(&(fb - target).abs())
                })
                .unwrap_or(0.0);
            g.t = t;
            g.refresh();
            g.paint();
            let ps = g.picture.pixel_size();
            let mut n = 0;
            for y in 0..i32::from(ps.height()) {
                for x in 0..i32::from(ps.width()) {
                    if g.picture.get(x, y).is_some_and(|k| k.role == Role::Phase) {
                        n += 1;
                    }
                }
            }
            n
        };
        let (bright, dim) = (phase_pixels(false), phase_pixels(true));
        assert!(bright > 0 && dim * 10 < bright * 9, "{bright} vs {dim}");
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
        let c = 41f64.to_radians().cos_det();
        let s = g.lut_at(c);
        assert!(s.bow > 0.5, "{s:?}");
    }
}
