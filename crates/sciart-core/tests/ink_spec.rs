//! SPEC-001 CORE-R78..R80: phase, blackbody and named-ramp inks (ADR-0015).

use std::f64::consts::TAU;

use proptest::prelude::*;
use sciart_core::ink::{self, Ramp};
use sciart_core::raster::Dots;
use sciart_core::{Canvas, GlyphMode, Ink, Role, Size};

#[test]
fn core_r78_phase_levels_cover_one_turn() {
    assert_eq!(ink::phase(0.0), Ink::new(Role::Phase, 0));
    assert_eq!(ink::phase(TAU / 4.0), Ink::new(Role::Phase, 64));
    assert_eq!(ink::phase(TAU / 2.0), Ink::new(Role::Phase, 128));
    // A full turn, and anything within half a level of it, wraps to 0.
    assert_eq!(ink::phase(TAU), Ink::new(Role::Phase, 0));
    assert_eq!(ink::phase(TAU * 255.9 / 256.0), Ink::new(Role::Phase, 0));
    assert_eq!(ink::phase(TAU * 255.0 / 256.0), Ink::new(Role::Phase, 255));
    // Negative angles and angles past 2π wrap.
    assert_eq!(ink::phase(-TAU / 4.0), Ink::new(Role::Phase, 192));
    assert_eq!(
        ink::phase(7.0 * TAU + TAU / 2.0),
        Ink::new(Role::Phase, 128)
    );
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(ink::phase(bad), Ink::new(Role::Phase, 0), "{bad}");
    }
}

#[test]
fn core_r78_phase_value_inverts_phase_on_every_level() {
    for level in 0..=255u8 {
        let i = Ink::new(Role::Phase, level);
        let v = ink::phase_value(i);
        assert!((0.0..TAU).contains(&v), "{level}: {v}");
        assert_eq!(ink::phase(v), i, "level {level}");
    }
}

#[test]
fn core_r78_phase_dots_do_not_compete_by_level_the_last_one_wins() {
    let mut dots = Dots::new(Size::new(1, 1));
    dots.set(0, 0, ink::phase(3.0));
    dots.set(1, 0, ink::phase(0.1)); // a lower level, set last
    let mut canvas = Canvas::new(Size::new(1, 1), GlyphMode::Unicode);
    dots.blit(&mut canvas);
    assert_eq!(canvas.get(0, 0).unwrap().ink, ink::phase(0.1));
}

#[test]
fn core_r79_blackbody_levels_are_log_temperature() {
    assert_eq!(ink::BLACKBODY_MIN_K, 1000.0);
    assert_eq!(ink::BLACKBODY_MAX_K, 30_000.0);
    assert_eq!(ink::blackbody(1000.0), Ink::new(Role::Blackbody, 0));
    assert_eq!(ink::blackbody(30_000.0), Ink::new(Role::Blackbody, 255));
    // Geometric midpoint √(1000·30000) ≈ 5477 K sits at the middle level.
    assert_eq!(ink::blackbody(30_000_000f64.sqrt()).level, 128);
    // Clamped below and above; NaN is the low end.
    assert_eq!(ink::blackbody(300.0).level, 0);
    assert_eq!(ink::blackbody(0.0).level, 0);
    assert_eq!(ink::blackbody(-5.0).level, 0);
    assert_eq!(ink::blackbody(1e9).level, 255);
    assert_eq!(ink::blackbody(f64::INFINITY).level, 255);
    assert_eq!(ink::blackbody(f64::NAN).level, 0);
}

#[test]
fn core_r79_blackbody_kelvin_inverts_blackbody_on_every_level() {
    let mut last = 0.0;
    for level in 0..=255u8 {
        let i = Ink::new(Role::Blackbody, level);
        let k = ink::blackbody_kelvin(i);
        assert!(k > last, "increasing at {level}");
        last = k;
        assert_eq!(ink::blackbody(k), i, "level {level}");
    }
    assert!((ink::blackbody_kelvin(Ink::new(Role::Blackbody, 0)) - 1000.0).abs() < 1e-9);
    assert!((ink::blackbody_kelvin(Ink::new(Role::Blackbody, 255)) - 30_000.0).abs() < 1e-6);
}

#[test]
fn core_r80_ramps_are_named_and_listed() {
    assert_eq!(
        Ramp::ALL,
        [
            Ramp::Ember,
            Ramp::Ice,
            Ramp::Ferroin,
            Ramp::HAlpha,
            Ramp::Aurora,
            Ramp::Plasma
        ]
    );
    let names: Vec<_> = Ramp::ALL.iter().map(|r| r.name()).collect();
    assert_eq!(
        names,
        ["ember", "ice", "ferroin", "halpha", "aurora", "plasma"]
    );
}

#[test]
fn core_r80_ramp_maps_like_seq_and_seq_value_reads_it_back() {
    for r in Ramp::ALL {
        assert_eq!(ink::ramp(r, 0.0), Ink::new(Role::Ramp(r), 0));
        assert_eq!(ink::ramp(r, 1.0), Ink::new(Role::Ramp(r), 255));
        assert_eq!(ink::ramp(r, 2.0).level, 255);
        assert_eq!(ink::ramp(r, -1.0).level, 0);
        assert_eq!(ink::ramp(r, f64::NAN).level, 0);
        for level in 0..=255u8 {
            let i = Ink::new(Role::Ramp(r), level);
            assert_eq!(ink::ramp(r, ink::seq_value(i)), i, "{r:?} {level}");
            assert_eq!(
                ink::ramp(r, ink::seq_value(i)).level,
                ink::seq(ink::seq_value(i)).level
            );
        }
    }
}

#[test]
fn core_r80_ink_stays_two_bytes() {
    assert_eq!(std::mem::size_of::<Ink>(), 2);
}

proptest! {
    #[test]
    fn core_r78_phase_is_periodic(a in -1e6f64..1e6, k in -50i32..50) {
        // Whole turns never move the level by more than rounding at a level boundary.
        let (p, q) = (ink::phase(a).level, ink::phase(a + f64::from(k) * TAU).level);
        let d = u16::from(p.abs_diff(q)).min(256 - u16::from(p.abs_diff(q)));
        prop_assert!(d <= 1, "{a} + {k} turns: {p} vs {q}");
    }

    #[test]
    fn core_r79_blackbody_is_monotone_in_temperature(a in 0.0f64..1e5, b in 0.0f64..1e5) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(ink::blackbody(lo).level <= ink::blackbody(hi).level);
    }
}
