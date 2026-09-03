//! SPEC-001 CORE-R93..R95: stellar-disc helpers shared by the Sun & Planets scenes.

use sciart_core::raster;
use sciart_core::stellar;

#[test]
fn core_r93_limb_darkening_law() {
    let u = (0.62, 0.12);
    assert!((stellar::limb_darkening(1.0, u) - 1.0).abs() < 1e-15);
    assert!((stellar::limb_darkening(0.0, u) - (1.0 - 0.62 - 0.12)).abs() < 1e-15);
    // Linear law is the quadratic with u2 = 0.
    assert!((stellar::limb_darkening(0.5, (0.6, 0.0)) - 0.7).abs() < 1e-15);
    // μ is clamped onto [0, 1]; NaN reads as the limb, never NaN.
    assert_eq!(stellar::limb_darkening(2.0, u), 1.0);
    assert_eq!(
        stellar::limb_darkening(-1.0, u),
        stellar::limb_darkening(0.0, u)
    );
    assert!(stellar::limb_darkening(f64::NAN, u).is_finite());
}

#[test]
fn core_r95_disc_cover_is_the_centre_distance_estimate() {
    // Pixel (x, y) spans [x, x + 1) × [y, y + 1); cover = clamp(r + ½ − d, 0, 1).
    assert_eq!(raster::disc_cover(3, 4, (3.5, 4.5), 2.0), 1.0);
    assert_eq!(raster::disc_cover(10, 4, (3.5, 4.5), 2.0), 0.0);
    let c = raster::disc_cover(5, 4, (3.5, 4.5), 1.8);
    assert!((c - 0.3).abs() < 1e-12, "{c}");
    assert_eq!(raster::disc_cover(0, 0, (f64::NAN, 0.0), 1.0), 0.0);
}

#[test]
fn core_r95_disc_pixels_visits_every_covered_pixel_once_row_by_row() {
    for &(cx, cy, r) in &[(10.3, 7.7, 0.4), (10.5, 7.5, 2.0), (20.0, 11.2, 5.3)] {
        let mut seen = Vec::new();
        raster::disc_pixels((cx, cy), r, |x, y, cover| seen.push((x, y, cover)));
        // Exactly the pixels with positive cover in a generous box, each once, with the
        // cover `disc_cover` gives, in row-major order.
        let mut want = Vec::new();
        for y in -5..40 {
            for x in -5..60 {
                let c = raster::disc_cover(x, y, (cx, cy), r);
                if c > 0.0 {
                    want.push((x, y, c));
                }
            }
        }
        assert_eq!(seen, want, "({cx}, {cy}) r={r}");
    }
    // Degenerate input visits nothing and never loops forever.
    let mut n = 0;
    raster::disc_pixels((f64::NAN, 1.0), 3.0, |_, _, _| n += 1);
    raster::disc_pixels((1.0, 1.0), f64::INFINITY, |_, _, _| n += 1);
    raster::disc_pixels((1.0, 1.0), -2.0, |_, _, _| n += 1);
    assert_eq!(n, 0);
}
