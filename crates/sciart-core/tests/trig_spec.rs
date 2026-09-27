//! SPEC-001 CORE-R90: trigonometry that does not depend on the optimiser.
//!
//! The reference values are libm's `sin`/`cos` evaluated alone, each behind its own
//! non-inlined function and a `black_box`, so LLVM cannot fuse them into a `sincos`.
#![allow(clippy::disallowed_methods)] // the reference must call libm directly

use std::hint::black_box;

use proptest::prelude::*;
use sciart_core::prelude::*;
use sciart_core::trig;

#[inline(never)]
fn lone_sin(x: f64) -> f64 {
    black_box(x).sin()
}

#[inline(never)]
fn lone_cos(x: f64) -> f64 {
    black_box(x).cos()
}

#[inline(never)]
fn lone_sinf(x: f32) -> f32 {
    black_box(x).sin()
}

#[inline(never)]
fn lone_cosf(x: f32) -> f32 {
    black_box(x).cos()
}

/// Bit patterns, so NaN compares equal to NaN and -0.0 differs from 0.0.
fn bits(p: (f64, f64)) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

fn bitsf(p: (f32, f32)) -> (u32, u32) {
    (p.0.to_bits(), p.1.to_bits())
}

/// A fixed sweep dense enough to hit the ~0.2 % of arguments where Apple's fused
/// `__sincos_stret` sine differs from `sin` (tens of thousands of them at this count).
fn sweep() -> impl Iterator<Item = f64> {
    let mut s = SplitMix64::new(42);
    (0..2_000_000).map(move |i| {
        let u = (s.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        // Mostly phases in [-4π, 4π], plus some large sim-time products.
        if i % 16 == 0 {
            (u - 0.5) * 2.0e5
        } else {
            (u - 0.5) * 8.0 * std::f64::consts::PI
        }
    })
}

#[test]
fn core_r90_sin_cos_is_bitwise_the_lone_sin_and_cos() {
    let mut bad = 0usize;
    for x in sweep() {
        let want = (lone_sin(x), lone_cos(x));
        if bits(trig::sin_cos(x)) != bits(want)
            || bits(x.sin_cos_det()) != bits(want)
            || bits((x.sin_det(), x.cos_det())) != bits(want)
            || bits((trig::sin(x), trig::cos(x))) != bits(want)
        {
            bad += 1;
        }
    }
    assert_eq!(
        bad, 0,
        "CORE-R90: sin_cos must equal the unfused libm pair bit for bit"
    );
}

#[test]
fn core_r90_f32_matches_the_lone_libm_calls() {
    let mut bad = 0usize;
    for x in sweep() {
        let x = x as f32;
        let want = (lone_sinf(x), lone_cosf(x));
        if bitsf(trig::sin_cosf(x)) != bitsf(want)
            || bitsf(x.sin_cos_det()) != bitsf(want)
            || bitsf((x.sin_det(), x.cos_det())) != bitsf(want)
            || bitsf((trig::sinf(x), trig::cosf(x))) != bitsf(want)
        {
            bad += 1;
        }
    }
    assert_eq!(
        bad, 0,
        "CORE-R90: f32 trig must equal the unfused libm calls bit for bit"
    );
}

#[test]
fn core_r90_special_values_follow_libm() {
    for x in [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        1e300,
        -1e-300,
    ] {
        assert_eq!(
            bits(x.sin_cos_det()),
            bits((lone_sin(x), lone_cos(x))),
            "x = {x}"
        );
    }
}

proptest! {
    #[test]
    fn core_r90_any_f64_agrees_with_lone_calls(x in any::<f64>()) {
        prop_assert_eq!(bits(x.sin_cos_det()), bits((lone_sin(x), lone_cos(x))));
    }
}
