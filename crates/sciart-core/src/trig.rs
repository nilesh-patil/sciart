//! Sine and cosine whose bits do not depend on the optimiser (SPEC-001 CORE-R90, ADR-0016).
//!
//! Rust never contracts float expressions into FMAs or reorders them, so arithmetic is
//! reproducible across build profiles. Library calls are not quite: when a `sin` and a
//! `cos` of the same argument end up in one function, LLVM replaces the pair with one
//! `sincos` call (`__sincos_stret` on Apple platforms), and that sine differs from
//! `sin` in the last bit for about 0.2 % of arguments. Whether a pair shares a function
//! is an inlining decision, so dev and release builds of the same scene diverged, and a
//! chaotic scene turned the one-ulp difference into a different frame.
//!
//! Each function here is a non-inlined wrapper around exactly one libm call (a tail
//! call, so the cost is one branch), so no function ever holds both halves of a pair and
//! the combine never fires. `clippy.toml` bans the `std` methods everywhere else (GEN-R6).
#![allow(clippy::disallowed_methods)] // the one place that may call libm's sin/cos

/// `x.sin()`, never fused with a cosine.
#[inline(never)]
#[must_use]
pub fn sin(x: f64) -> f64 {
    x.sin()
}

/// `x.cos()`, never fused with a sine.
#[inline(never)]
#[must_use]
pub fn cos(x: f64) -> f64 {
    x.cos()
}

/// `(sin(x), cos(x))`: bitwise the two lone calls, whatever inlines around it.
#[inline]
#[must_use]
pub fn sin_cos(x: f64) -> (f64, f64) {
    (sin(x), cos(x))
}

/// `x.sin()` for `f32`, never fused with a cosine.
#[inline(never)]
#[must_use]
pub fn sinf(x: f32) -> f32 {
    x.sin()
}

/// `x.cos()` for `f32`, never fused with a sine.
#[inline(never)]
#[must_use]
pub fn cosf(x: f32) -> f32 {
    x.cos()
}

/// `(sinf(x), cosf(x))`.
#[inline]
#[must_use]
pub fn sin_cosf(x: f32) -> (f32, f32) {
    (sinf(x), cosf(x))
}

/// Method syntax for the functions above, so `a.sin_cos()` reads `a.sin_cos_det()`.
/// In the prelude. The names differ from `std`'s because inherent methods win.
pub trait Trig: Copy {
    /// Sine, bit-identical in every build profile.
    #[must_use]
    fn sin_det(self) -> Self;
    /// Cosine, bit-identical in every build profile.
    #[must_use]
    fn cos_det(self) -> Self;
    /// `(sin_det, cos_det)`.
    #[must_use]
    fn sin_cos_det(self) -> (Self, Self);
}

impl Trig for f64 {
    #[inline]
    fn sin_det(self) -> f64 {
        sin(self)
    }
    #[inline]
    fn cos_det(self) -> f64 {
        cos(self)
    }
    #[inline]
    fn sin_cos_det(self) -> (f64, f64) {
        sin_cos(self)
    }
}

impl Trig for f32 {
    #[inline]
    fn sin_det(self) -> f32 {
        sinf(self)
    }
    #[inline]
    fn cos_det(self) -> f32 {
        cosf(self)
    }
    #[inline]
    fn sin_cos_det(self) -> (f32, f32) {
        sin_cosf(self)
    }
}
