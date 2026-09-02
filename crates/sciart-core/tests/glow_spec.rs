//! SPEC-001 CORE-R85: `Glow`, a long exposure at `Pixels` resolution.

use proptest::prelude::*;
use sciart_core::prelude::*;
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
fn core_r92_bilinear_splat_on_a_pixel_centre_lights_only_it() {
    let mut g = Glow::new(Size::new(7, 3));
    g.splat_bilinear(3.0, 2.0, 1.0);
    assert_eq!(g.get(3, 2), Some(1.0));
    assert_eq!(g.values().iter().sum::<f32>(), 1.0);
    // Half-way between two pixels: an even split, not a jump to the nearest.
    let mut g = Glow::new(Size::new(7, 3));
    g.splat_bilinear(3.5, 2.0, 1.0);
    assert_eq!((g.get(3, 2), g.get(4, 2)), (Some(0.5), Some(0.5)));
    // Off the buffer, non-finite: nothing, no panic.
    let mut g = Glow::new(Size::new(7, 3));
    for (x, y) in [(-3.0, 1.0), (1.0, 99.0), (f64::NAN, 1.0), (1e300, -1e300)] {
        g.splat_bilinear(x, y, 1.0);
    }
    g.splat_bilinear(1.0, 1.0, f32::INFINITY);
    assert!(g.values().iter().all(|&v| v == 0.0));
}

proptest! {
 #[test]
 fn core_r92_bilinear_splat_conserves_in_bounds_weight(
 w in 2u16..20, h in 2u16..10,
 fx in 0.0f64..1.0, fy in 0.0f64..1.0,
 s in 0.0f32..10.0,
 ) {
 let mut g = Glow::new(Size::new(w, h));
 let size = g.pixel_size();
 // Anywhere whose four neighbours are all on the buffer.
 let x = fx * f64::from(size.width() - 1);
 let y = fy * f64::from(size.height() - 1);
 g.splat_bilinear(x, y, s);
 let total: f32 = g.values().iter().sum();
 prop_assert!((total - s).abs() <= 1e-5 * (1.0 + s));
 prop_assert!(g.values().iter().all(|&v| v >= 0.0));
 // Only the four pixels around (x, y) are touched.
 for yy in 0..i32::from(size.height()) {
 for xx in 0..i32::from(size.width()) {
 if (f64::from(xx) - x).abs() >= 1.0 || (f64::from(yy) - y).abs() >= 1.0 {
 prop_assert_eq!(g.get(xx, yy), Some(0.0));
 }
 }
 }
 }

 #[test]
 fn core_r92_bilinear_splat_never_panics(
 x in -1e6f64..1e6, y in -1e6f64..1e6, s in -10.0f32..10.0,
 ) {
 let mut g = Glow::new(Size::new(5, 3));
 g.splat_bilinear(x, y, s);
 prop_assert!(g.values().iter().all(|v| v.is_finite()));
 }
}
