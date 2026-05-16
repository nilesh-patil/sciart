//! Semantic inks (CORE-R8), ink levels (CORE-R66), the colormap-field helpers
//! (CORE-R46, CORE-R56, ADR-0011) and the phase, blackbody and named-ramp roles
//! (CORE-R78..R80, ADR-0015).

use std::f64::consts::TAU;

/// What a cell *means*; themes turn roles into colors (ADR-0004, CORE-R8).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Background,
    Dim,
    Star,
    Sun,
    Planet,
    Particle,
    Wave,
    Hot,
    Cold,
    Structure,
    Accent,
    Text,
    /// Sequential scalar field: level 0 = low, 255 = high (CORE-R46). Build with [`seq`].
    Seq,
    /// Signed scalar field: level 128 = zero, 0 = most negative, 255 = most positive
    /// (CORE-R46). Build with [`div`].
    Div,
    /// Cyclic field: level is a phase angle, one turn over 256 levels, so 255 sits next
    /// to 0 (CORE-R78). Build with [`phase`].
    Phase,
    /// Temperature on a log scale, 1000 K at level 0 to 30 000 K at 255, drawn in the
    /// colour of a blackbody (CORE-R79). Build with [`blackbody`].
    Blackbody,
    /// Sequential field in a named ramp: level 0 = low, 255 = high (CORE-R80). Build with
    /// [`ramp()`].
    Ramp(Ramp),
}

/// A named sequential ramp for [`Role::Ramp`] (CORE-R80, ADR-0015): one hue family each,
/// near-black at level 0 and pale at 255, so field scenes keep their own identity under
/// one theme. Named palettes are false colour; SPEC-002 says so where a scene uses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ramp {
    /// Oxblood → scarlet → orange → pale gold: glowing matter, fire, plumes, growth age.
    Ember,
    /// Navy → teal-blue → cyan → white: cold gas, frost, Cherenkov and young-star blue.
    Ice,
    /// Wine → crimson → violet → azure: BZ ferroin (red, reduced) to ferriin (blue).
    Ferroin,
    /// Crimson → rose → pale pink: emission nebulae (Hα 656 nm), prominences.
    HAlpha,
    /// Deep teal → emerald → lime-white: auroral O I 557.7 nm, fluorescence.
    Aurora,
    /// Indigo → violet → lavender-white: discharges, lightning, synchrotron glow.
    Plasma,
    /// Near-black → neutral grey → pale warm white: unresolved starlight, the nearly
    /// colourless integrated light of an old stellar population.
    Starlight,
    /// Near-black olive → chrome yellow → pale lemon: the plasmodium of *Physarum
    /// polycephalum*, sulfur and pollen yellows (a yellow that is not Ember's orange).
    Sulfur,
    /// Deep ultramarine → cobalt blue → pale silver-blue: cobalt glass and pigments, a
    /// cool precipitate (M12, `liesegang_rings`), a blue that is not Ice's cyan.
    Cobalt,
    /// Wine-rose → eosin rose-pink → pale pink-lilac: the eosin of a histology stain, for
    /// patterns that read as tissue (M12, `gray_scott`), a rose that is neither Ferroin's
    /// plum, Plasma's violet nor HAlpha's crimson.
    Eosin,
}

impl Ramp {
    /// Every ramp, in declaration order.
    pub const ALL: [Ramp; 10] = [
        Ramp::Ember,
        Ramp::Ice,
        Ramp::Ferroin,
        Ramp::HAlpha,
        Ramp::Aurora,
        Ramp::Plasma,
        Ramp::Starlight,
        Ramp::Sulfur,
        Ramp::Cobalt,
        Ramp::Eosin,
    ];

    /// A unique lower-case name.
    pub const fn name(self) -> &'static str {
        match self {
            Ramp::Ember => "ember",
            Ramp::Ice => "ice",
            Ramp::Ferroin => "ferroin",
            Ramp::HAlpha => "halpha",
            Ramp::Aurora => "aurora",
            Ramp::Plasma => "plasma",
            Ramp::Starlight => "starlight",
            Ramp::Sulfur => "sulfur",
            Ramp::Cobalt => "cobalt",
            Ramp::Eosin => "eosin",
        }
    }
}

/// Semantic style of a cell: a role plus an intensity `0..=255`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ink {
    pub role: Role,
    pub level: u8,
}

impl Ink {
    pub const BLANK: Ink = Ink::new(Role::Background, 0);

    pub const fn new(role: Role, level: u8) -> Self {
        Self { role, level }
    }
}

impl Default for Ink {
    fn default() -> Self {
        Self::BLANK
    }
}

/// Levels below this should not be drawn (CORE-R66): every theme keeps level 0 visible
/// (a dim ink is still a glyph), so a fade must end by removing ink, not by dimming it to
/// nothing. Compare the real-valued level before converting it with [`level`].
pub const MIN_VISIBLE_LEVEL: f64 = 12.0;

/// An ink level from a real value: rounded to the nearest integer and clamped to
/// `0..=255`; NaN ⇒ 0 (CORE-R66).
#[inline]
pub fn level(v: f64) -> u8 {
    // `clamp` keeps NaN, and the saturating `as u8` maps it to 0.
    v.round().clamp(0.0, 255.0) as u8
}

/// A [`Role::Seq`] ink for `v ∈ [0, 1]`: 0 ⇒ level 0, 1 ⇒ 255; clamped, NaN ⇒ 0
/// (CORE-R46).
#[inline]
pub fn seq(v: f64) -> Ink {
    // `clamp` keeps NaN, and a saturating `as u8` maps NaN to 0: the low end.
    Ink::new(Role::Seq, (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// A [`Role::Ramp`] ink for `v ∈ [0, 1]` in `ramp`, mapped exactly like [`seq`] and read
/// back with [`seq_value`] (CORE-R80).
#[inline]
pub fn ramp(ramp: Ramp, v: f64) -> Ink {
    Ink::new(Role::Ramp(ramp), seq(v).level)
}

/// A [`Role::Phase`] ink for an angle in radians, wrapped onto one turn:
/// `round(frac(a / 2π) · 256) mod 256`; NaN or ±∞ ⇒ level 0 (CORE-R78).
#[inline]
pub fn phase(radians: f64) -> Ink {
    let turns = radians / TAU;
    let frac = if turns.is_finite() {
        turns - turns.floor()
    } else {
        0.0
    };
    // `frac ∈ [0, 1]` (it can round up to exactly 1), so the product is in [0, 256] and
    // the wrap folds 256 onto 0.
    Ink::new(Role::Phase, ((frac * 256.0).round() as u32 % 256) as u8)
}

/// The angle in `[0, 2π)` a [`Role::Phase`] level stands for, `level · 2π / 256`: the
/// exact inverse of [`phase`] on its levels (CORE-R78). Reads `level` whatever the role.
#[inline]
pub fn phase_value(ink: Ink) -> f64 {
    f64::from(ink.level) * TAU / 256.0
}

/// Temperature at [`Role::Blackbody`] level 0 (CORE-R79).
pub const BLACKBODY_MIN_K: f64 = 1000.0;
/// Temperature at [`Role::Blackbody`] level 255 (CORE-R79).
pub const BLACKBODY_MAX_K: f64 = 30_000.0;

/// A [`Role::Blackbody`] ink for a temperature in kelvin: level `255 · ln(T / 1000) / ln 30`,
/// rounded and clamped; NaN ⇒ level 0 (CORE-R79). The log scale spends about half the
/// levels on the red-to-white part, where the colour changes fastest.
#[inline]
pub fn blackbody(kelvin: f64) -> Ink {
    let t = (kelvin / BLACKBODY_MIN_K).ln() / (BLACKBODY_MAX_K / BLACKBODY_MIN_K).ln();
    // `ln` of a non-positive ratio is −∞ or NaN: both land on level 0 via `level`.
    Ink::new(Role::Blackbody, level(t * 255.0))
}

/// The temperature in kelvin a [`Role::Blackbody`] level stands for,
/// `1000 · 30^(level / 255)`: the inverse of [`blackbody`] on its levels (CORE-R79). Reads
/// `level` whatever the role.
#[inline]
pub fn blackbody_kelvin(ink: Ink) -> f64 {
    BLACKBODY_MIN_K * (BLACKBODY_MAX_K / BLACKBODY_MIN_K).powf(f64::from(ink.level) / 255.0)
}

/// A temperature drawn from a spectral-class mix (CORE-R88): `classes` holds
/// `(cumulative fraction, lowest K, highest K)` rows in increasing fraction; a uniform `u`
/// picks the first row with `u < fraction` (the last row past the end) and `v ∈ [0, 1]`
/// places the temperature within it, `lo + (hi − lo)·v`. NaN for an empty table.
#[inline]
pub fn class_kelvin(classes: &[(f64, f64, f64)], u: f64, v: f64) -> f64 {
    let Some(&last) = classes.last() else {
        return f64::NAN;
    };
    let (_, lo, hi) = classes
        .iter()
        .copied()
        .find(|&(cum, _, _)| u < cum)
        .unwrap_or(last);
    lo + (hi - lo) * v
}

/// A [`Role::Div`] ink for `v ∈ [−1, 1]`: −1 ⇒ 0, 0 ⇒ 128, 1 ⇒ 255; clamped, NaN ⇒ 128
/// (CORE-R46).
///
/// The two halves have different slopes (128 levels below zero, 127 above) so that
/// both ends of the range and an exact zero are representable.
#[inline]
pub fn div(v: f64) -> Ink {
    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
    let half = if v < 0.0 { 128.0 } else { 127.0 };
    Ink::new(Role::Div, (128.0 + v * half).round() as u8)
}

/// The value `v ∈ [0, 1]` a [`Role::Seq`] level stands for: `level / 255`, the exact
/// inverse of [`seq`] on its levels (CORE-R56). Reads `level` whatever the role.
#[inline]
pub fn seq_value(ink: Ink) -> f64 {
    f64::from(ink.level) / 255.0
}

/// The value `v ∈ [−1, 1]` a [`Role::Div`] level stands for, the exact inverse of
/// [`div`] on its levels (same two slopes either side of 128) (CORE-R56). Reads `level`
/// whatever the role.
#[inline]
pub fn div_value(ink: Ink) -> f64 {
    let d = f64::from(ink.level) - 128.0;
    if d < 0.0 { d / 128.0 } else { d / 127.0 }
}

/// A cheap `tanh`-like squash onto `[−1, 1]` for mapping unbounded fields to colour
/// (CORE-R56): the Padé approximant `x(27 + x²) / (27 + 9x²)`, which is odd, monotone
/// (its derivative is `9(x² − 9)² / (27 + 9x²)² ≥ 0`), meets ±1 with zero slope at
/// `|x| = 3`, and is exactly ±1 beyond. No libm call, so it is bit-reproducible across
/// platforms; NaN ⇒ 0.
#[inline]
pub fn soft_clip(x: f64) -> f64 {
    if x.is_nan() {
        0.0
    } else if x >= 3.0 {
        1.0
    } else if x <= -3.0 {
        -1.0
    } else {
        let x2 = x * x;
        (x * (27.0 + x2) / (27.0 + 9.0 * x2)).clamp(-1.0, 1.0)
    }
}
