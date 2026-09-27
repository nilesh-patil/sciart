//! SPEC-001 CORE-R81: the asinh tone curve and `Density::to_pixels`.

use proptest::prelude::*;
use sciart_core::prelude::*;
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

const INK: fn(u8) -> Ink = |level| Ink::new(Role::Seq, level);

#[test]
fn core_r81_to_pixels_sums_each_two_by_two_dot_block() {
    // 2×1 cells: 4×4 dots, 2×2 pixels.
    let size = Size::new(2, 1);
    let mut density = Density::new(size);
    let tone = Tone::asinh(1.0, 8.0);
    // Pixel (0, 0) ⇐ dots (0..=1, 0..=1); pixel (1, 1) ⇐ dots (2..=3, 2..=3).
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        density.deposit(f64::from(x), f64::from(y), 2.0);
    }
    density.deposit(3.0, 3.0, 0.5);
    let mut px = Pixels::new(size);
    px.fill(Ink::new(Role::Hot, 9)); // stale content must be overwritten
    density.to_pixels(&mut px, &tone, INK);
    assert_eq!(px.get(0, 0), Some(INK(tone.level(8.0))));
    assert_eq!(px.get(0, 0), Some(INK(255)));
    assert_eq!(px.get(1, 1), Some(INK(tone.level(0.5))));
    assert_eq!(px.get(1, 0), Some(Ink::BLANK), "an empty block is empty");
    assert_eq!(px.get(0, 1), Some(Ink::BLANK));
}

#[test]
fn core_r81_to_pixels_leaves_level_zero_empty_and_ignores_extra_pixels() {
    let mut density = Density::new(Size::new(1, 1)); // 2×4 dots ⇒ 1×2 pixels
    density.deposit(0.0, 0.0, 1e-9); // rounds to level 0
    density.deposit(1.0, 3.0, 5.0);
    let tone = Tone::asinh(1.0, 5.0);
    let mut px = Pixels::new(Size::new(3, 2)); // larger than the density
    px.fill(Ink::new(Role::Hot, 9));
    let mut calls = 0;
    density.to_pixels(&mut px, &tone, |l| {
        calls += 1;
        INK(l)
    });
    assert_eq!(calls, 1, "ink is called for lit pixels only");
    assert_eq!(px.get(0, 0), Some(Ink::BLANK));
    assert_eq!(px.get(0, 1), Some(INK(255)));
    for (x, y) in [(1, 0), (2, 0), (0, 2), (2, 3)] {
        assert_eq!(px.get(x, y), Some(Ink::BLANK), "no block at ({x}, {y})");
    }
}
