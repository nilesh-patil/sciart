//! Semantic inks (CORE-R8) and the colormap-field helpers (CORE-R46, ADR-0011).

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
