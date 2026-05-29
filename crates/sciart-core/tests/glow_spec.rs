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
    g.values_mut()[0] = 2.0;
    assert_eq!(g.get(0, 0), Some(2.0));
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

// ---------------------------------------------------------------- CORE-R96

#[test]
fn core_r96_stroke_kernel_is_a_normalised_3x3_splat() {
    let k = sciart_core::raster::STROKE_KERNEL;
    let sum: f32 = k.iter().map(|&(_, _, w)| w).sum();
    assert!((sum - 1.0).abs() < 1e-6, "{sum}");
    assert!(k.iter().all(|&(dx, dy, _)| dx.abs() <= 1 && dy.abs() <= 1));
    assert_eq!(k[0], (0, 0, 0.36));
}

#[test]
fn core_r96_halo_kernel_is_round_and_falls_from_one_to_zero() {
    const HALO: [(i32, i32, f32); 49] = sciart_core::raster::halo_kernel(3);
    let w = |dx: i32, dy: i32| {
        HALO.iter()
            .find(|&&(x, y, _)| (x, y) == (dx, dy))
            .map(|&(_, _, w)| w)
            .expect("offset in the kernel")
    };
    assert_eq!(w(0, 0), 1.0);
    // Symmetric under the square's eight symmetries, falling outward, 0 in the corners.
    for (dx, dy) in [(1, 0), (2, 1), (3, 0), (2, 2)] {
        let v = w(dx, dy);
        for (x, y) in [(dx, dy), (-dx, dy), (dy, dx), (-dy, -dx), (dx, -dy)] {
            assert_eq!(w(x, y), v);
        }
    }
    assert!(w(0, 0) > w(1, 0) && w(1, 0) > w(2, 0) && w(2, 0) > w(3, 0));
    assert!(w(3, 0) > 0.0);
    assert_eq!(w(3, 3), 0.0);
    let expect = (1.0 - 5.0 / (3.5f32 * 3.5)).powi(2);
    assert!((w(2, 1) - expect).abs() < 1e-6);
    let one: [(i32, i32, f32); 1] = sciart_core::raster::halo_kernel(0);
    assert_eq!(one, [(0, 0, 1.0)]);
}

#[test]
fn core_r96_stroke_exposes_one_splat_per_pixel_of_length() {
    let point = [(0, 0, 1.0f32)];
    // 20 dots = 10 pixels: ten splats of 1 along the row.
    let mut g = Glow::new(Size::new(20, 4));
    g.stroke((0.0, 2.0), (20.0, 2.0), &point, 1.0);
    let total: f32 = g.values().iter().sum();
    assert_eq!(total, 10.0);
    // A zero-length stroke is one splat; a non-finite one draws nothing.
    let mut g = Glow::new(Size::new(20, 4));
    g.stroke((5.0, 2.0), (5.0, 2.0), &point, 2.0);
    assert_eq!(g.values().iter().sum::<f32>(), 2.0);
    let mut g = Glow::new(Size::new(20, 4));
    g.stroke((0.0, 0.0), (f64::INFINITY, 1.0), &point, 1.0);
    g.stroke((f64::NAN, 0.0), (1.0, 1.0), &point, 1.0);
    assert!(g.values().iter().all(|&v| v == 0.0));
    // A runaway segment is capped at 2048 splats, never a hang or a panic.
    let mut g = Glow::new(Size::new(4, 2));
    g.stroke((0.0, 0.0), (1e12, 0.0), &point, 1.0);
    assert!(g.values().iter().sum::<f32>() <= 2048.0);
}

/// Reference `[1 2 1]/4 ⊗ [1 2 1]/4` with clamped edges, out of place (CORE-R103).
fn blur_reference(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let at = |x: isize, y: isize| {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        src[y * w + x]
    };
    let mut rows = vec![0.0; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            rows[y as usize * w + x as usize] =
                0.25 * (at(x - 1, y) + at(x + 1, y)) + 0.5 * at(x, y);
        }
    }
    let at = |x: usize, y: isize| rows[y.clamp(0, h as isize - 1) as usize * w + x];
    let mut out = vec![0.0; w * h];
    for y in 0..h as isize {
        for x in 0..w {
            out[y as usize * w + x] = 0.25 * (at(x, y - 1) + at(x, y + 1)) + 0.5 * at(x, y);
        }
    }
    out
}

#[test]
fn core_r103_blur121_spreads_an_impulse_into_the_binomial_kernel() {
    let mut g = Glow::new(Size::new(5, 3)); // 5×6 pixels
    g.values_mut()[2 * 5 + 2] = 16.0;
    g.blur121();
    let want = [[1.0, 2.0, 1.0], [2.0, 4.0, 2.0], [1.0, 2.0, 1.0]];
    for y in 0..6i32 {
        for x in 0..5i32 {
            let (dx, dy) = (x - 2, y - 2);
            let v = if dx.abs() <= 1 && dy.abs() <= 1 {
                want[(dy + 1) as usize][(dx + 1) as usize]
            } else {
                0.0
            };
            assert_eq!(g.get(x, y), Some(v), "pixel ({x}, {y})");
        }
    }
    // Interior mass is conserved; a constant field is a fixed point (clamped edges).
    assert_eq!(g.values().iter().sum::<f32>(), 16.0);
    let mut flat = Glow::new(Size::new(4, 2));
    flat.values_mut().fill(3.0);
    flat.blur121();
    assert!(flat.values().iter().all(|&v| v == 3.0));
}

#[test]
fn core_r103_blur121_degenerate_sizes_do_not_panic() {
    for (w, h) in [(1, 1), (1, 5), (7, 1)] {
        let mut g = Glow::new(Size::new(w, h));
        g.values_mut().fill(2.0);
        g.blur121();
        assert!(g.values().iter().all(|&v| v == 2.0));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn core_r103_blur121_matches_the_separable_reference(
        w in 1u16..24, h in 1u16..10, seed in any::<u64>(), shrink in 1u16..24,
    ) {
        let mut g = Glow::new(Size::new(w, h));
        // Resize down after a larger size: the scratch row must follow the width.
        g.resize(Size::new(w.min(shrink), h));
        let px = g.pixel_size();
        let (pw, ph) = (usize::from(px.width()), usize::from(px.height()));
        let mut s = seed;
        for v in g.values_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *v = (s >> 40) as f32 / (1u64 << 24) as f32 * 8.0;
        }
        let want = blur_reference(g.values(), pw, ph);
        g.blur121();
        for (i, (&a, &b)) in g.values().iter().zip(&want).enumerate() {
            prop_assert!((a - b).abs() <= 1e-5 * b.abs().max(1.0), "pixel {i}: {a} vs {b}");
        }
    }
}
