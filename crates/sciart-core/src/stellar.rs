//! Stellar-disc helpers (SPEC-001 CORE-R93): the quadratic limb-darkening law, shared by
//! `solar_system`-era scenes (`total_solar_eclipse`, `solar_surface`, `exoplanet_transit`).

/// Quadratic limb darkening `I(μ)/I(1) = 1 − u1(1 − μ) − u2(1 − μ)²` (CORE-R93), where
/// `μ = cos θ` is 1 at the disc centre and 0 at the limb; the linear law is `u2 = 0`.
/// `μ` is clamped onto `[0, 1]` and NaN reads as the limb.
#[inline]
pub fn limb_darkening(mu: f64, (u1, u2): (f64, f64)) -> f64 {
    let mu = if mu.is_nan() { 0.0 } else { mu.clamp(0.0, 1.0) };
    let m = 1.0 - mu;
    1.0 - u1 * m - u2 * m * m
}
