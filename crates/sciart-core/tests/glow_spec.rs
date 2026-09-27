//! SPEC-001 CORE-R85: `Glow`, a long exposure at `Pixels` resolution.

use proptest::prelude::*;
use sciart_core::prelude::*;
use sciart_core::raster::Tone;

const INK: fn(u8) -> Ink = |level| Ink::new(Role::Seq, level);
const POINT: [(i32, i32, f32); 1] = [(0, 0, 1.0)];

#[test]
fn core_r85_glow_is_one_value_per_pixel() {
    let g = Glow::new(Size::new(7, 3));
    assert_eq!(g.pixel_size(), Size::new(7, 6));
    assert_eq!(g.pixel_size(), Pixels::new(Size::new(7, 3)).pixel_size());
    assert_eq!(g.get(6, 5), Some(0.0));
    assert_eq!(g.get(7, 0), None);
    assert_eq!(g.get(0, 6), None);
    assert_eq!(g.get(-1, 0), None);
    assert_eq!(Glow::default().pixel_size(), Size::new(1, 2));
}

#[test]
fn core_r85_splat_lands_in_the_pixel_holding_the_dot() {
    let mut g = Glow::new(Size::new(3, 2)); // 6×8 dots, 3×4 pixels
    // Pixel (1, 2) holds dots (2..=3, 4..=5); dots round to the nearest one first.
    for (x, y) in [(2.0, 4.0), (3.0, 5.0), (3.4, 4.6)] {
        g.splat(x, y, &POINT, 1.0);
    }
    assert_eq!(g.get(1, 2), Some(3.0));
    g.splat(3.6, 5.0, &POINT, 1.0); // rounds to dot 4 ⇒ pixel 2
    assert_eq!(g.get(2, 2), Some(1.0));
    // A kernel spreads about that pixel, scaled; off-buffer parts are dropped.
    let cross = [
        (0, 0, 0.5),
        (-1, 0, 0.125),
        (1, 0, 0.125),
        (0, -1, 0.125),
        (0, 1, 0.125),
    ];
    let mut g = Glow::new(Size::new(3, 2));
    g.splat(0.0, 0.0, &cross, 2.0);
    assert_eq!(g.get(0, 0), Some(1.0));
    assert_eq!(g.get(1, 0), Some(0.25));
    assert_eq!(g.get(0, 1), Some(0.25));
    let total: f32 = (0..4)
        .flat_map(|y| (0..3).map(move |x| (x, y)))
        .map(|(x, y)| g.get(x, y).unwrap())
        .sum();
    assert_eq!(total, 1.5, "the two neighbours off the buffer are dropped");
    // Non-finite input changes nothing.
    let before: Vec<_> = (0..3).map(|x| g.get(x, 0)).collect();
    g.splat(f64::NAN, 0.0, &POINT, 1.0);
    g.splat(0.0, f64::INFINITY, &POINT, 1.0);
    g.splat(0.0, 0.0, &POINT, f32::NAN);
    g.splat(1e300, -1e300, &POINT, 1.0);
    assert_eq!((0..3).map(|x| g.get(x, 0)).collect::<Vec<_>>(), before);
}

#[test]
fn core_r85_decay_fades_and_resize_clears() {
    let mut g = Glow::new(Size::new(2, 1));
    g.splat(0.0, 0.0, &POINT, 4.0);
    g.decay(0.5);
    assert_eq!(g.get(0, 0), Some(2.0));
    g.decay(f32::NAN);
    g.decay(1.5);
    assert_eq!(g.get(0, 0), Some(2.0), "NaN and k ≥ 1 leave it as is");
    g.decay(-3.0);
    assert_eq!(g.get(0, 0), Some(0.0), "k < 0 clamps to 0");
    g.splat(0.0, 0.0, &POINT, 4.0);
    g.clear();
    assert_eq!(g.get(0, 0), Some(0.0));
    g.splat(0.0, 0.0, &POINT, 4.0);
    g.resize(Size::new(4, 2));
    assert_eq!(g.pixel_size(), Size::new(4, 4));
    assert_eq!(g.get(0, 0), Some(0.0));
}

#[test]
fn core_r85_to_pixels_tone_maps_each_pixel_one_to_one() {
    let size = Size::new(2, 1);
    let mut g = Glow::new(size);
    g.splat(0.0, 0.0, &POINT, 8.0); // pixel (0, 0)
    g.splat(3.0, 3.0, &POINT, 0.5); // pixel (1, 1)
    g.splat(0.0, 2.0, &POINT, 1e-9); // pixel (0, 1): rounds to level 0
    let tone = Tone::asinh(1.0, 8.0);
    let mut px = Pixels::new(Size::new(3, 2)); // larger than the glow
    px.fill(Ink::new(Role::Hot, 9)); // stale content must be overwritten
    let mut calls = 0;
    g.to_pixels(&mut px, &tone, |l| {
        calls += 1;
        INK(l)
    });
    assert_eq!(calls, 2, "ink is called for lit pixels only");
    assert_eq!(px.get(0, 0), Some(INK(255)));
    assert_eq!(px.get(1, 1), Some(INK(tone.level(0.5))));
    for (x, y) in [(1, 0), (0, 1), (2, 0), (2, 1), (0, 2), (2, 3)] {
        assert_eq!(px.get(x, y), Some(Ink::BLANK), "({x}, {y})");
    }
    // The ink may itself leave a pixel empty.
    g.to_pixels(
        &mut px,
        &tone,
        |l| if l < 255 { Ink::BLANK } else { INK(l) },
    );
    assert_eq!(px.get(1, 1), Some(Ink::BLANK));
    assert_eq!(px.get(0, 0), Some(INK(255)));
}

proptest! {
    #[test]
    fn core_r85_splat_never_panics_and_only_touches_its_kernel(
        w in 1u16..20, h in 1u16..10,
        x in -100.0f64..100.0, y in -100.0f64..100.0,
        s in 0.0f32..10.0,
    ) {
        let mut g = Glow::new(Size::new(w, h));
        g.splat(x, y, &POINT, s);
        let (px, py) = ((x.round() as i32).div_euclid(2), (y.round() as i32).div_euclid(2));
        let size = g.pixel_size();
        for yy in 0..i32::from(size.height()) {
            for xx in 0..i32::from(size.width()) {
                let v = g.get(xx, yy).unwrap();
                prop_assert_eq!(v, if (xx, yy) == (px, py) { s } else { 0.0 });
            }
        }
    }
}
