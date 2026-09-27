//! Colour tables for the ADR-0015 field roles: the astro phase loop (RND-R10), the Planck
//! locus (RND-R11) and the named ramps (RND-R12).
//!
//! Every entry needs cube roots, `exp` and colour matrices, which is too costly per painted
//! cell, so each table is built once on first use and indexed by `level`.

use std::sync::LazyLock;

use sciart_core::{Ink, Ramp, Role, Trig, ink};

use crate::theme::Rgb;

pub(crate) struct Tables {
    /// `astro`'s isoluminant hue loop for `Role::Phase`.
    pub phase: [Rgb; 256],
    /// The Planck locus for `Role::Blackbody`, shared by `astro` and `rainbow`.
    pub blackbody: [Rgb; 256],
    /// One table per [`Ramp`], indexed `ramp as usize`.
    pub ramps: [[Rgb; 256]; Ramp::ALL.len()],
}

pub(crate) fn tables() -> &'static Tables {
    static TABLES: LazyLock<Tables> = LazyLock::new(|| Tables {
        phase: std::array::from_fn(|i| phase_loop(i as f64 / 256.0)),
        // `i < 256`: the casts are exact.
        blackbody: std::array::from_fn(|i| blackbody(Ink::new(Role::Blackbody, i as u8))),
        ramps: Ramp::ALL.map(|r| std::array::from_fn(|i| ramp(r, i as f64 / 255.0))),
    });
    &TABLES
}

// ---------------------------------------------------------------------------------------
// Named ramps (RND-R12)
// ---------------------------------------------------------------------------------------

/// OKLab lightness of level 0 (L* ≈ 4, near-black) and level 255 (L* ≈ 94, pale).
const RAMP_L: (f64, f64) = (0.16, 0.955);

/// A ramp in OKLCh: the hue runs linearly from `h0` to `h1` (degrees, either direction)
/// while lightness rises evenly; `chroma` is the peak, reached a quarter of the way up.
struct RampSpec {
    h0: f64,
    h1: f64,
    chroma: f64,
}

const fn spec(r: Ramp) -> RampSpec {
    match r {
        // oxblood → scarlet → orange → pale gold
        Ramp::Ember => RampSpec {
            h0: 20.0,
            h1: 95.0,
            chroma: 0.20,
        },
        // navy → teal-blue → cyan → white
        Ramp::Ice => RampSpec {
            h0: 265.0,
            h1: 200.0,
            chroma: 0.14,
        },
        // wine → crimson → violet → azure (the hue turns backwards through magenta)
        Ramp::Ferroin => RampSpec {
            h0: 15.0,
            h1: -115.0,
            chroma: 0.19,
        },
        // crimson → rose → pale pink
        Ramp::HAlpha => RampSpec {
            h0: 20.0,
            h1: 2.0,
            chroma: 0.21,
        },
        // deep teal → emerald → lime-white
        Ramp::Aurora => RampSpec {
            h0: 200.0,
            h1: 125.0,
            chroma: 0.18,
        },
        // indigo → violet → lavender-white
        Ramp::Plasma => RampSpec {
            h0: 275.0,
            h1: 310.0,
            chroma: 0.19,
        },
        // near-black → neutral grey → pale warm white: unresolved starlight
        Ramp::Starlight => RampSpec {
            h0: 75.0,
            h1: 70.0,
            chroma: 0.035,
        },
    }
}

fn ramp(r: Ramp, t: f64) -> Rgb {
    let RampSpec { h0, h1, chroma } = spec(r);
    let l = RAMP_L.0 + (RAMP_L.1 - RAMP_L.0) * t;
    // A tinted dark end, full colour through the middle, and a pale but still hued top,
    // so the ramps stay apart at both ends. Starlight alone stays grey while faint: an old
    // population's integrated light (B−V ≈ 0.7) is pale yellow-white, so a faint halo or
    // Milky Way must not read as sepia; its warmth shows only where it is bright.
    let grey = if r == Ramp::Starlight {
        (2.0 * t).min(1.0)
    } else {
        1.0
    };
    let c = chroma * grey * (0.3 + 2.8 * t).min(1.0) * (1.0 - 0.7 * t * t);
    oklch(l, c, h0 + (h1 - h0) * t)
}

// ---------------------------------------------------------------------------------------
// Phase loop (RND-R10)
// ---------------------------------------------------------------------------------------

/// OKLab lightness and chroma of astro's phase loop: bright enough to read on a black
/// terminal, soft enough that every hue fits in gamut at one lightness.
const PHASE_L: f64 = 0.74;
const PHASE_C: f64 = 0.125;

fn phase_loop(turns: f64) -> Rgb {
    // Start at red (OKLab hue ≈ 30°) so phase 0 reads as the "warm" reference.
    oklch(PHASE_L, PHASE_C, 30.0 + 360.0 * turns)
}

// ---------------------------------------------------------------------------------------
// Blackbody (RND-R11)
// ---------------------------------------------------------------------------------------

/// CIE L* of the coolest level; the dim end of a glow, still distinct from black.
const BLACKBODY_DARK_L: f64 = 12.0;
/// Where the glow reaches its full, unscaled colour (the white point, ~6500 K).
const BLACKBODY_WHITE_K: f64 = 6500.0;

/// The sRGB colour of a blackbody at the ink's temperature: the chromaticity of Planck's
/// law under the CIE 1931 2° observer, normalised to its brightest channel, then dimmed
/// below the white point so that lightness climbs from `BLACKBODY_DARK_L` to the full colour.
fn blackbody(ink: Ink) -> Rgb {
    encode(blackbody_linear(ink))
}

/// CIE L* of the [`blackbody`] colour at the ink's level: what `mono` paints, so a star
/// is as bright in grey as in colour (RND-R11).
pub(crate) fn blackbody_lightness(ink: Ink) -> f64 {
    lightness(luminance(blackbody_linear(ink)))
}

/// The linear sRGB behind [`blackbody`].
fn blackbody_linear(ink: Ink) -> [f64; 3] {
    let kelvin = ink::blackbody_kelvin(ink);
    let [x, y, z] = planck_xyz(kelvin);
    let rgb = xyz_to_linear_srgb([x, y, z]).map(|c| c.max(0.0));
    let peak = rgb[0].max(rgb[1]).max(rgb[2]);
    let rgb = rgb.map(|c| c / peak);
    let full = lightness(luminance(rgb));
    // Linear in log T, the same scale as the level, up to the white point.
    let span = (BLACKBODY_WHITE_K / ink::BLACKBODY_MIN_K).ln();
    let s = ((kelvin / ink::BLACKBODY_MIN_K).ln() / span).clamp(0.0, 1.0);
    let target = BLACKBODY_DARK_L + (full - BLACKBODY_DARK_L) * s;
    let k = (luminance_of(target) / luminance(rgb)).min(1.0);
    rgb.map(|c| c * k)
}

/// Tristimulus XYZ of Planck's law at `kelvin`, up to a common scale: a 5 nm Riemann sum
/// over 380–780 nm of the spectral radiance times the CIE 1931 colour-matching functions.
fn planck_xyz(kelvin: f64) -> [f64; 3] {
    /// Second radiation constant `hc/k`, in nm·K.
    const C2: f64 = 1.438_776_9e7;
    let mut xyz = [0.0; 3];
    for nm in (380..=780).step_by(5) {
        let l = f64::from(nm);
        // λ⁻⁵ in units of 1000 nm keeps the sum well-scaled at every temperature.
        let radiance = (l / 1000.0).powi(-5) / (C2 / (l * kelvin)).exp_m1();
        let [xb, yb, zb] = cmf(l);
        xyz[0] += radiance * xb;
        xyz[1] += radiance * yb;
        xyz[2] += radiance * zb;
    }
    xyz
}

/// The CIE 1931 2° colour-matching functions at `nm`, from the multi-lobe piecewise
/// Gaussian fit of Wyman, Sloan and Shirley (JCGT 2(2), 2013): within ~1 % of the table,
/// and no 471-row table in the source.
fn cmf(nm: f64) -> [f64; 3] {
    let g = |mu: f64, s1: f64, s2: f64| {
        let t = (nm - mu) / if nm < mu { s1 } else { s2 };
        (-0.5 * t * t).exp()
    };
    [
        1.056 * g(599.8, 37.9, 31.0) + 0.362 * g(442.0, 16.0, 26.7) - 0.065 * g(501.1, 20.4, 26.2),
        0.821 * g(568.8, 46.9, 40.5) + 0.286 * g(530.9, 16.3, 31.1),
        1.217 * g(437.0, 11.8, 36.0) + 0.681 * g(459.0, 26.0, 13.8),
    ]
}

/// CIE XYZ (D65) to linear sRGB (IEC 61966-2-1).
fn xyz_to_linear_srgb([x, y, z]: [f64; 3]) -> [f64; 3] {
    [
        3.2406 * x - 1.5372 * y - 0.4986 * z,
        -0.9689 * x + 1.8758 * y + 0.0415 * z,
        0.0557 * x - 0.2040 * y + 1.0570 * z,
    ]
}

// ---------------------------------------------------------------------------------------
// Colour-space helpers
// ---------------------------------------------------------------------------------------

/// OKLCh (Ottosson 2020) to sRGB, lowering the chroma until the colour is in gamut, so
/// hue and lightness are kept exactly and only saturation gives way.
fn oklch(l: f64, c: f64, hue_deg: f64) -> Rgb {
    let (sin, cos) = hue_deg.to_radians().sin_cos_det();
    let at = |c: f64| oklab_to_linear_srgb(l, c * cos, c * sin);
    let in_gamut = |rgb: [f64; 3]| rgb.iter().all(|v| (-1e-9..=1.0 + 1e-9).contains(v));
    let (mut lo, mut hi) = (0.0, c);
    if !in_gamut(at(hi)) {
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if in_gamut(at(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi = lo;
    }
    encode(at(hi))
}

fn oklab_to_linear_srgb(l: f64, a: f64, b: f64) -> [f64; 3] {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    [
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_,
    ]
}

/// Relative luminance Y of linear sRGB.
fn luminance([r, g, b]: [f64; 3]) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// CIE L* of luminance `y ∈ [0, 1]`.
fn lightness(y: f64) -> f64 {
    if y > 216.0 / 24389.0 {
        116.0 * y.cbrt() - 16.0
    } else {
        y * 24389.0 / 27.0
    }
}

/// Luminance Y of CIE lightness `l` (the inverse of [`lightness`]).
fn luminance_of(l: f64) -> f64 {
    if l > 8.0 {
        ((l + 16.0) / 116.0).powi(3)
    } else {
        l * 27.0 / 24389.0
    }
}

/// Gamma-encode linear sRGB and round to 8 bits.
fn encode(rgb: [f64; 3]) -> Rgb {
    let e = |c: f64| {
        let c = c.clamp(0.0, 1.0);
        let v = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Rgb(e(rgb[0]), e(rgb[1]), e(rgb[2]))
}
