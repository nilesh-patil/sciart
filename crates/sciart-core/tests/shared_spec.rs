//! SPEC-001 CORE-R60..R70: helpers lifted out of the M10 scenes by the post-M10 audit
//! (3-vectors, angle wrap, stroked lines, disc coverage and magnification, dot mapping,
//! pixel shading, ink levels, labels, regridding, lattice shapes, stateless hashes).

use std::f64::consts::{PI, TAU};

use proptest::prelude::*;
use sciart_core::geom::{self, v3};
use sciart_core::prelude::*;
use sciart_core::rng::{self, SplitMix64};
use sciart_core::{draw, grid, ink};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

fn finite3() -> impl Strategy<Value = [f64; 3]> {
    prop::array::uniform3(-1e3f64..1e3)
}

// ---------------------------------------------------------------- CORE-R60

#[test]
fn core_r60_v3_basics() {
    let (a, b) = ([1.0, 2.0, 3.0], [4.0, -5.0, 6.0]);
    assert_eq!(v3::add(a, b), [5.0, -3.0, 9.0]);
    assert_eq!(v3::sub(a, b), [-3.0, 7.0, -3.0]);
    assert_eq!(v3::scale(a, 2.0), [2.0, 4.0, 6.0]);
    assert_eq!(v3::axpy(2.0, a, b), [6.0, -1.0, 12.0]);
    assert_eq!(v3::dot(a, b), 12.0);
    assert_eq!(v3::cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    assert_eq!(v3::norm([3.0, 4.0, 12.0]), 13.0);
    assert_eq!(v3::unit([0.0, 0.0, -2.0]), [0.0, 0.0, -1.0]);
}

proptest! {
    #[test]
    fn core_r60_cross_is_orthogonal_and_unit_is_unit(a in finite3(), b in finite3()) {
        let c = v3::cross(a, b);
        let scale = v3::norm(a) * v3::norm(b);
        prop_assert!(v3::dot(c, a).abs() <= 1e-9 * scale * v3::norm(a).max(1.0));
        prop_assert!(v3::dot(c, b).abs() <= 1e-9 * scale * v3::norm(b).max(1.0));
        // Anti-commutative, exactly: every component is a difference of the same products.
        prop_assert_eq!(v3::cross(b, a), v3::scale(c, -1.0));
        if v3::norm(a) > 1e-6 {
            prop_assert!(close(v3::norm(v3::unit(a)), 1.0));
        }
        prop_assert!(close(v3::dot(a, a), v3::norm(a).powi(2)));
    }
}

// ---------------------------------------------------------------- CORE-R61

#[test]
fn core_r61_wrap_angle_lands_in_minus_pi_to_pi() {
    assert_eq!(geom::wrap_angle(0.0), 0.0);
    assert!(close(geom::wrap_angle(TAU + 0.5), 0.5));
    assert!(close(geom::wrap_angle(-TAU - 0.5), -0.5));
    assert!(close(geom::wrap_angle(PI + 0.25), -PI + 0.25));
    assert!(geom::wrap_angle(f64::NAN).is_nan());
}

proptest! {
    #[test]
    fn core_r61_wrap_angle_is_the_same_direction(a in -1e4f64..1e4) {
        let w = geom::wrap_angle(a);
        prop_assert!((-PI..PI).contains(&w), "{} -> {}", a, w);
        prop_assert!((w.sin_det() - a.sin_det()).abs() < 1e-9);
        prop_assert!((w.cos_det() - a.cos_det()).abs() < 1e-9);
    }
}

// ---------------------------------------------------------------- CORE-R62

fn stroke(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32, i32, (i32, i32))> {
    let mut out = Vec::new();
    geom::stroked_line(x0, y0, x1, y1, |k, x, y, step| out.push((k, x, y, step)));
    out
}

#[test]
fn core_r62_stroked_line_hands_out_steps() {
    // A lone cell is a step down a column.
    assert_eq!(stroke(3, 4, 3, 4), [(0, 3, 4, (0, 1))]);
    // The first cell takes its outgoing step; the rest their incoming one.
    assert_eq!(
        stroke(0, 0, 3, 1),
        [
            (0, 0, 0, (1, 0)),
            (1, 1, 0, (1, 0)),
            (2, 2, 1, (1, 1)),
            (3, 3, 1, (1, 0))
        ]
    );
    let cells: Vec<(i32, i32)> = stroke(5, -2, -3, 7).iter().map(|c| (c.1, c.2)).collect();
    assert_eq!(cells, geom::line(5, -2, -3, 7).collect::<Vec<_>>());
}

#[test]
fn core_r62_step_glyph_per_direction() {
    let g = |dx, dy| draw::step_glyph(dx, dy).unicode;
    assert_eq!(g(1, 0), '─');
    assert_eq!(g(-1, 0), '─');
    assert_eq!(g(0, 1), '│');
    assert_eq!(g(0, -1), '│');
    assert_eq!(g(1, 1), '╲');
    assert_eq!(g(-1, -1), '╲');
    assert_eq!(g(1, -1), '╱');
    assert_eq!(g(-1, 1), '╱');
    assert_eq!(draw::step_glyph(1, 1).ascii, '\\');
    assert_eq!(draw::step_glyph(-1, 1).ascii, '/');
}

// ---------------------------------------------------------------- CORE-R63

#[test]
fn core_r63_disc_covers_exactly_the_cells_disc_paints() {
    let size = Size::new(40, 20);
    for &(cx, cy, r) in &[
        (10.3, 7.8, 3.2),
        (20.0, 10.0, 0.5),
        (5.5, 5.5, 1.7),
        (12.2, 9.6, 0.3),
        (0.4, 0.2, 2.5),
    ] {
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        draw::disc(&mut canvas, cx, cy, r, |_, _, _| {
            Some((Glyph::ascii('#'), Ink::new(Role::Star, 1)))
        });
        for y in 0..20 {
            for x in 0..40 {
                let painted = canvas.get(x, y).unwrap().ch == '#';
                assert_eq!(
                    draw::disc_covers(cx, cy, r, x, y),
                    painted,
                    "disc ({cx}, {cy}, {r}) at ({x}, {y})"
                );
            }
        }
    }
    assert!(!draw::disc_covers(f64::NAN, 1.0, 2.0, 0, 0));
    assert!(!draw::disc_covers(1.0, 1.0, 0.0, 1, 1));
    assert!(!draw::disc_covers(1.0, 1.0, f64::INFINITY, 1, 1));
}

#[test]
fn core_r63_magnification_matches_project() {
    let cam = Camera {
        yaw: 0.4,
        pitch: -0.3,
        distance: 6.0,
    };
    let proj = cam.projector();
    let p = [0.7, -1.1, 0.9];
    let (x, _, depth) = proj.project(p, (0.0, 0.0), 1.0).unwrap();
    let rotated = proj.rotate(p);
    assert!(close(
        x,
        rotated[0] * proj.magnification(depth) * geom::ASPECT
    ));
    assert!(proj.magnification(0.0) == 1.0);
    assert!(proj.magnification(3.0) > 1.0 && proj.magnification(-3.0) < 1.0);
    // Orthographic: no magnification at any depth.
    let ortho = Camera::default().projector();
    assert_eq!(ortho.magnification(5.0), 1.0);
    assert_eq!(ortho.magnification(-5.0), 1.0);
}

// ---------------------------------------------------------------- CORE-R64

#[test]
fn core_r64_from_cells_maps_into_the_holding_cell() {
    assert_eq!(Dots::from_cells(0.0, 0.0), (-0.5, -0.5));
    assert_eq!(Dots::from_cells(3.5, 2.5), (6.5, 9.5));
    let mut dots = Dots::new(Size::new(10, 5));
    // Any point strictly inside cell (cx, cy) lights a dot of that cell.
    for &(x, y) in &[(3.01, 2.01), (3.99, 2.99), (3.5, 2.5), (3.26, 2.74)] {
        dots.clear();
        let (dx, dy) = Dots::from_cells(x, y);
        dots.plot(dx, dy, Ink::new(Role::Star, 9));
        assert_ne!(dots.cell_mask(3, 2), Some(0), "({x}, {y})");
    }
}

#[test]
fn core_r64_cell_mask_reads_the_braille_bits() {
    let mut dots = Dots::new(Size::new(3, 2));
    assert_eq!(dots.cell_mask(1, 1), Some(0));
    dots.set(2, 4, Ink::new(Role::Star, 9)); // cell (1, 1), dot (0, 0)
    dots.set(3, 7, Ink::new(Role::Star, 9)); // cell (1, 1), dot (1, 3)
    assert_eq!(dots.cell_mask(1, 1), Some(0x01 | 0x80));
    assert_eq!(dots.cell_mask(0, 0), Some(0));
    assert_eq!(dots.cell_mask(3, 0), None);
    assert_eq!(dots.cell_mask(0, -1), None);
}

// ---------------------------------------------------------------- CORE-R65

#[test]
fn core_r65_shade_sets_every_pixel() {
    let mut px = Pixels::new(Size::new(4, 3));
    px.shade(|x, y| Ink::new(Role::Seq, (10 * y + x) as u8));
    let s = px.pixel_size();
    assert_eq!(s, Size::new(4, 6));
    for y in 0..6 {
        for x in 0..4 {
            assert_eq!(px.get(x, y), Some(Ink::new(Role::Seq, (10 * y + x) as u8)));
        }
    }
    px.shade(|_, _| Ink::BLANK);
    assert_eq!(px.get(3, 5), Some(Ink::BLANK));
}

// ---------------------------------------------------------------- CORE-R66

#[test]
fn core_r66_level_rounds_and_clamps() {
    assert_eq!(ink::level(0.0), 0);
    assert_eq!(ink::level(12.49), 12);
    assert_eq!(ink::level(12.5), 13);
    assert_eq!(ink::level(254.6), 255);
    assert_eq!(ink::level(1e9), 255);
    assert_eq!(ink::level(-3.0), 0);
    assert_eq!(ink::level(f64::NAN), 0);
    assert_eq!(ink::level(f64::INFINITY), 255);
    assert_eq!(ink::level(ink::MIN_VISIBLE_LEVEL), 12);
}

// ---------------------------------------------------------------- CORE-R67

#[test]
fn core_r67_label_has_a_blank_gutter_either_side() {
    let mut c = Canvas::new(Size::new(12, 1), GlyphMode::Unicode);
    for x in 0..12 {
        c.put(x, 0, Glyph::ascii('#'), Ink::new(Role::Star, 1));
    }
    let ink = Ink::new(Role::Text, 200);
    let next = draw::label(&mut c, 2, 0, "λ=5", ink);
    assert_eq!(next, 7);
    assert_eq!(c.to_lines()[0], "## λ=5 #####");
    assert_eq!(c.get(2, 0).unwrap().ink, Ink::BLANK);
    assert_eq!(c.get(3, 0).unwrap().ink, ink);
    assert_eq!(c.get(6, 0).unwrap().ink, Ink::BLANK);
    // Clipped at the edge without panicking.
    assert_eq!(draw::label(&mut c, 9, 0, "long", ink), 15);
    assert_eq!(c.to_lines()[0], "## λ=5 ## lo");
}

// ---------------------------------------------------------------- CORE-R76

#[test]
fn core_r76_label_right_mirrors_label() {
    let hashes = |w| {
        let mut c = Canvas::new(Size::new(w, 1), GlyphMode::Unicode);
        for x in 0..i32::from(w) {
            c.put(x, 0, Glyph::ascii('#'), Ink::new(Role::Star, 1));
        }
        c
    };
    let ink = Ink::new(Role::Text, 200);

    let mut c = hashes(12);
    assert_eq!(draw::label_right(&mut c, 0, "λ=5", ink), 7);
    assert_eq!(c.to_lines()[0], "####### λ=5 ");
    assert_eq!(c.get(7, 0).unwrap().ink, Ink::BLANK);
    assert_eq!(c.get(8, 0).unwrap().ink, ink);
    assert_eq!(c.get(10, 0).unwrap().ink, ink);
    assert_eq!(c.get(11, 0).unwrap().ink, Ink::BLANK);

    // Clipped at column 0; the returned column goes negative.
    let mut c = hashes(12);
    assert_eq!(draw::label_right(&mut c, 0, "a much longer text", ink), -8);
    assert_eq!(c.to_lines()[0], "longer text ");

    // Off-canvas rows and a 1-column canvas are harmless.
    let mut c = hashes(1);
    assert_eq!(draw::label_right(&mut c, 0, "ab", ink), -3);
    assert_eq!(c.to_lines()[0], " ");
    assert_eq!(draw::label_right(&mut c, 5, "ab", ink), -3);
}

// ---------------------------------------------------------------- CORE-R68

#[test]
fn core_r68_regrid_matches_resample_and_swaps() {
    let mut field = Grid::new(Size::new(6, 4), 0.0f32);
    for (i, v) in field.as_mut_slice().iter_mut().enumerate() {
        *v = i as f32 * 0.25;
    }
    let to = Size::new(9, 3);
    let mut expected = Grid::new(to, 0.0);
    grid::resample_bilinear(&field, &mut expected);
    let old = field.clone();
    let mut scratch = Grid::new(Size::new(2, 2), 7.0);
    grid::regrid_bilinear(&mut field, to, &mut scratch);
    assert_eq!(field, expected);
    assert_eq!(scratch, old);
    // Already at size: untouched.
    grid::regrid_bilinear(&mut field, to, &mut scratch);
    assert_eq!(field, expected);
    // Empty field: nothing to sample, stays empty.
    let mut empty = Grid::<f32>::default();
    grid::regrid_bilinear(&mut empty, to, &mut scratch);
    assert!(empty.is_empty());

    let mut spins = Grid::new(Size::new(5, 5), 1i8);
    spins.set(0, 0, -1);
    let mut want = Grid::new(Size::new(3, 7), 0i8);
    grid::resample_nearest(&spins, &mut want);
    let mut scratch = Grid::default();
    grid::regrid_nearest(&mut spins, Size::new(3, 7), &mut scratch);
    assert_eq!(spins, want);
}

#[test]
fn core_r68_centre_of_aligns_cell_centres() {
    assert_eq!(grid::centre_of(3.0, 10, 10), 3.0);
    // Cell centres of a 2-cell axis sit at the quarter points of a 4-cell one.
    assert_eq!(grid::centre_of(0.0, 2, 4), 0.5);
    assert_eq!(grid::centre_of(1.0, 2, 4), 2.5);
    // The outer edges map onto each other.
    assert_eq!(grid::centre_of(-0.5, 3, 7), -0.5);
    assert!(close(grid::centre_of(2.5, 3, 7), 6.5));
}

// ---------------------------------------------------------------- CORE-R69

#[test]
fn core_r69_lattice_for_aspect_is_the_uncapped_shape() {
    let s = grid::lattice_for_aspect(2.0, 20_000);
    assert_eq!(s, Size::new(200, 100));
    assert_eq!(grid::lattice_for_aspect(1.0, 0), Size::new(1, 1));
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let s = grid::lattice_for_aspect(bad, 5000);
        assert!(s.area() <= 5000, "{bad}: {s:?}");
    }
    // Saturates per axis instead of overflowing a u16.
    let wide = grid::lattice_for_aspect(1e6, usize::MAX / 4);
    assert_eq!(wide.width(), u16::MAX);
    // `lattice_for` is this shape clamped to the display.
    let display = Size::new(300, 150);
    assert_eq!(
        grid::lattice_for(display, 20_000, 1.0),
        grid::lattice_for_aspect(2.0, 20_000)
    );
}

proptest! {
    #[test]
    fn core_r69_lattice_for_aspect_respects_the_cap(aspect in 0.05f64..20.0, max in 1usize..200_000) {
        let s = grid::lattice_for_aspect(aspect, max);
        prop_assert!(s.area() <= max);
        prop_assert!(s.width() >= 1 && s.height() >= 1);
    }
}

// ---------------------------------------------------------------- CORE-R70

#[test]
fn core_r70_hashes_are_the_splitmix_finaliser() {
    // `next_u64` is the finaliser of the Weyl sequence.
    let mut r = SplitMix64::new(0);
    assert_eq!(r.next_u64(), SplitMix64::mix(0x9E37_79B9_7F4A_7C15));
    assert_eq!(SplitMix64::mix(0), 0);
    let mut r = SplitMix64::new(41);
    assert_eq!(
        r.next_f64(),
        rng::hash01(41u64.wrapping_add(0x9E37_79B9_7F4A_7C15))
    );
    assert_eq!(rng::cell_key(0, 0), 0);
    assert_eq!(rng::cell_key(1, 2), (1 << 32) | 2);
    assert_eq!(rng::cell_key(-1, -1), u64::MAX);
    assert_ne!(rng::cell_key(1, 0), rng::cell_key(0, 1));
}

proptest! {
    #[test]
    fn core_r70_hash01_is_a_stable_unit_value(key in any::<u64>(), x in any::<i32>(), y in any::<i32>()) {
        let h = rng::hash01(key);
        prop_assert!((0.0..1.0).contains(&h));
        prop_assert_eq!(h, rng::hash01(key));
        let k = rng::cell_key(x, y);
        prop_assert_eq!((k >> 32) as u32 as i32, x);
        prop_assert_eq!(k as u32 as i32, y);
    }
}
