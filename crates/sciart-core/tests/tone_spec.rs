//! SPEC-001 CORE-R81: the asinh tone curve.

use proptest::prelude::*;
use sciart_core::raster::Tone;

/// The exact curve, `255 · asinh(v / knee) / asinh(white / knee)` clamped to `[0, 255]`.
fn exact(v: f32, knee: f32, white: f32) -> f64 {
    let (v, knee, white) = (f64::from(v), f64::from(knee), f64::from(white));
    if v.is_nan() || v <= 0.0 {
        return 0.0;
    }
    (255.0 * (v / knee).asinh() / (white / knee).asinh()).min(255.0)
}

#[test]
fn core_r81_tone_ends_are_black_and_white() {
    let tone = Tone::asinh(0.5, 40.0);
    assert_eq!(tone.level(0.0), 0);
    assert_eq!(tone.level(-3.0), 0);
    assert_eq!(tone.level(f32::NAN), 0);
    assert_eq!(tone.level(40.0), 255);
    assert_eq!(tone.level(1e9), 255);
    assert_eq!(tone.level(f32::INFINITY), 255);
    assert_eq!(Tone::LUT, 1024);
}

#[test]
fn core_r81_tone_is_linear_when_faint_and_logarithmic_when_bright() {
    let tone = Tone::asinh(1.0, 1000.0);
    // Faint: doubling the value roughly doubles the level (asinh x ≈ x).
    let (a, b) = (f64::from(tone.level(0.2)), f64::from(tone.level(0.4)));
    assert!((b / a - 2.0).abs() < 0.25, "{a} {b}");
    // Bright: each doubling adds about 255 · ln 2 / asinh(1000) ≈ 23 levels (asinh x ≈ ln 2x).
    let steps: Vec<i32> = [100.0, 200.0, 400.0, 800.0]
        .windows(2)
        .map(|w| i32::from(tone.level(w[1])) - i32::from(tone.level(w[0])))
        .collect();
    assert!(steps.iter().all(|&s| (21..=25).contains(&s)), "{steps:?}");
}

#[test]
fn core_r81_bad_parameters_fall_back_and_the_knee_is_clamped() {
    for (knee, white) in [
        (f32::NAN, 1.0),
        (1.0, f32::NAN),
        (0.0, -2.0),
        (f32::INFINITY, 1.0),
    ] {
        let tone = Tone::asinh(knee, white);
        let fallback = Tone::asinh(1.0, 1.0);
        for v in [0.1, 0.5, 0.9, 1.0] {
            assert_eq!(tone.level(v), fallback.level(v), "{knee} {white} at {v}");
        }
    }
    // A knee below white / 1000 behaves as white / 1000.
    let (steep, clamped) = (Tone::asinh(1e-9, 10.0), Tone::asinh(0.01, 10.0));
    for v in [1e-4, 1e-3, 0.01, 0.1, 1.0, 9.0] {
        assert_eq!(steep.level(v), clamped.level(v), "at {v}");
    }
}

#[test]
fn core_r81_default_tone_is_the_fallback_curve() {
    let (d, f) = (Tone::default(), Tone::asinh(1.0, 1.0));
    for v in [0.0, 0.25, 0.5, 1.0] {
        assert_eq!(d.level(v), f.level(v));
    }
}

proptest! {
    #[test]
    fn core_r81_tone_is_monotone_and_within_two_levels(
        knee in 1e-3f32..10.0,
        white in 0.1f32..1e4,
        a in 0.0f32..1.2,
        b in 0.0f32..1.2,
    ) {
        let tone = Tone::asinh(knee, white);
        let knee = knee.max(white / 1000.0);
        let (lo, hi) = if a <= b { (a * white, b * white) } else { (b * white, a * white) };
        prop_assert!(tone.level(lo) <= tone.level(hi));
        for v in [lo, hi] {
            let err = (f64::from(tone.level(v)) - exact(v, knee, white)).abs();
            prop_assert!(err <= 2.0, "v {v}: {} vs {:.2}", tone.level(v), exact(v, knee, white));
        }
    }
}
