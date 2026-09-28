//! Stellar-disc helpers (SPEC-001 CORE-R93, CORE-R94): the limb-darkening law and the
//! brightness temperature that colours a limb-darkened disc truthfully on
//! [`Role::Blackbody`](crate::Role::Blackbody). Lifted out of `solar_system`,
//! `solar_surface` and `exoplanet_transit`, which each had a copy.

/// `λ / c₂` at 550 nm (the middle of the V band), in 1/K: the Wien step of
/// [`brightness_temperature`].
pub const WIEN_550: f64 = 550.0 / 1.438_776_9e7;

/// Quadratic limb darkening `I(μ)/I(1) = 1 − u1(1 − μ) − u2(1 − μ)²` (CORE-R93), where
/// `μ = cos θ` is 1 at the disc centre and 0 at the limb; the linear law is `u2 = 0`.
/// `μ` is clamped onto `[0, 1]` and NaN reads as the limb.
#[inline]
pub fn limb_darkening(mu: f64, (u1, u2): (f64, f64)) -> f64 {
    let mu = if mu.is_nan() { 0.0 } else { mu.clamp(0.0, 1.0) };
    let m = 1.0 - mu;
    1.0 - u1 * m - u2 * m * m
}

/// The 550 nm brightness temperature (K) of a surface element whose intensity is `i` of
/// the disc centre's, when the centre radiates at `t_centre` (CORE-R94): Wien's law,
/// `1/T = 1/t_centre − (λ/c₂)·ln i`. `i` is clamped onto `[10⁻⁶, 1]` (NaN ⇒ 10⁻⁶), so the
/// result is finite, positive and never hotter than the centre.
#[inline]
pub fn brightness_temperature(t_centre: f64, i: f64) -> f64 {
    let i = if i.is_nan() { 1e-6 } else { i.clamp(1e-6, 1.0) };
    1.0 / (1.0 / t_centre - WIEN_550 * i.ln())
}
