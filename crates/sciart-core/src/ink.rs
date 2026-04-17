//! Semantic inks (CORE-R8) and the colormap-field helpers (CORE-R46, CORE-R56, ADR-0011).

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

/// A [`Role::Seq`] ink for `v ∈ [0, 1]`: 0 ⇒ level 0, 1 ⇒ 255; clamped, NaN ⇒ 0
/// (CORE-R46).
#[inline]
pub fn seq(v: f64) -> Ink {
    // `clamp` keeps NaN, and a saturating `as u8` maps NaN to 0: the low end.
    Ink::new(Role::Seq, (v.clamp(0.0, 1.0) * 255.0).round() as u8)
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
