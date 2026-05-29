//! SPEC-001 CORE-R49..R59: shared M10 scene helpers (text, cached projector, dot lines,
//! sample maps, 9-point Laplacian, spatial hash, density buffer, soft clip, SplitMix64,
//! inverse CDF, tridiagonal solver).

use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};
use std::fmt::Write as _;

use proptest::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use sciart_core::grid::{self, SampleMap};
use sciart_core::particles::{InverseCdf, SpatialHash};
use sciart_core::prelude::*;
use sciart_core::raster::Density;
use sciart_core::rng::SplitMix64;
use sciart_core::text::StackText;
use sciart_core::{draw, geom, ink, ode};

const A: Ink = Ink::new(Role::Star, 200);

fn lit(dots: &Dots) -> BTreeSet<(i32, i32)> {
    let s = dots.dot_size();
    let mut out = BTreeSet::new();
    for y in 0..i32::from(s.height()) {
        for x in 0..i32::from(s.width()) {
            if dots.is_lit(x, y) {
                out.insert((x, y));
            }
        }
    }
    out
}

// ---------------------------------------------------------------- CORE-R49

#[test]
fn core_r49_stack_text_formats_like_a_string() {
    let mut buf = StackText::<32>::new();
    assert!(buf.is_empty());
    write!(buf, "T/Tc {:.2}", 2.269_18).unwrap();
    assert_eq!(buf.as_str(), "T/Tc 2.27");
    assert_eq!(buf.len(), 9);
    write!(buf, " N {}", 42).unwrap();
    assert_eq!(buf.as_str(), "T/Tc 2.27 N 42");
    buf.clear();
    assert_eq!((buf.as_str(), buf.len()), ("", 0));
    write!(buf, "again").unwrap();
    assert_eq!(buf.as_str(), "again");
}

#[test]
fn core_r49_stack_text_truncates_silently_on_a_char_boundary() {
    // 'é' is 2 bytes: "abcé" is 5 bytes.
    let mut five = StackText::<5>::new();
    assert!(write!(five, "abcé").is_ok());
    assert_eq!(five.as_str(), "abcé");
    let mut four = StackText::<4>::new();
    assert!(write!(four, "abcé").is_ok());
    assert_eq!((four.as_str(), four.len()), ("abc", 3));
    // Once truncated, later pieces are dropped too, so the text stays a prefix.
    write!(four, "d").unwrap();
    assert_eq!(four.as_str(), "abc");
    four.clear();
    write!(four, "wxyz!").unwrap();
    assert_eq!(four.as_str(), "wxyz");
    let mut zero = StackText::<0>::new();
    write!(zero, "anything {}", 1).unwrap();
    assert_eq!(zero.as_str(), "");
}

proptest! {
    #[test]
    fn core_r49_stack_text_is_a_bounded_prefix(pieces in prop::collection::vec(".{0,12}", 0..8)) {
        let mut buf = StackText::<24>::new();
        let mut full = String::new();
        for p in &pieces {
            write!(buf, "{p}").unwrap();
            full.push_str(p);
        }
        prop_assert!(buf.len() <= 24);
        prop_assert!(full.starts_with(buf.as_str()));
        prop_assert_eq!(buf.as_str().len(), buf.len());
        // Truncation keeps as much as fits: the next char would not have fitted.
        if buf.len() < full.len() {
            let next = full[buf.len()..].chars().next().unwrap();
            prop_assert!(buf.len() + next.len_utf8() > 24);
        }
    }
}

#[test]
fn core_r49_text_mode_picks_the_string_for_the_glyph_mode() {
    let ink = Ink::new(Role::Text, 150);
    let mut c = Canvas::new(Size::new(10, 2), GlyphMode::Unicode);
    assert_eq!(draw::text_mode(&mut c, 1, 0, "ψ²=1", "psi^2=1", ink), 5);
    assert_eq!(c.to_lines()[0], " ψ²=1     ");
    assert_eq!(c.get(1, 0).unwrap().ink, ink);

    let mut c = Canvas::new(Size::new(10, 2), GlyphMode::Ascii);
    assert_eq!(draw::text_mode(&mut c, 1, 1, "ψ²=1", "psi^2=1", ink), 8);
    assert_eq!(c.to_lines()[1], " psi^2=1  ");
}

#[test]
fn core_r49_text_mode_clips_at_both_edges_and_never_panics() {
    let ink = Ink::new(Role::Text, 150);
    let mut c = Canvas::new(Size::new(4, 1), GlyphMode::Ascii);
    assert_eq!(draw::text_mode(&mut c, -2, 0, "", "abcdef", ink), 4);
    assert_eq!(c.to_lines()[0], "cdef");
    assert_eq!(draw::text_mode(&mut c, 3, 5, "", "xyz", ink), 6);
    assert_eq!(
        draw::text_mode(&mut c, i32::MAX - 1, 0, "", "xyz", ink),
        i32::MAX
    );
    assert_eq!(
        draw::text_mode(&mut c, i32::MIN, 0, "", "xyz", ink),
        i32::MIN + 3
    );
    assert_eq!(c.to_lines()[0], "cdef");
}

// ---------------------------------------------------------------- CORE-R50

fn same_projection(a: Option<(f64, f64, f64)>, b: Option<(f64, f64, f64)>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.0.to_bits() == b.0.to_bits()
                && a.1.to_bits() == b.1.to_bits()
                && a.2.to_bits() == b.2.to_bits()
        }
        _ => false,
    }
}

#[test]
fn core_r50_rotate_returns_camera_space_coordinates() {
    let cam = Camera {
        yaw: FRAC_PI_2,
        ..Camera::default()
    };
    let [x, y, z] = cam.projector().rotate([1.0, 0.0, 0.0]);
    // yaw = π/2 turns +x onto the depth axis (away from the eye), as in CORE-R45.
    assert!(x.abs() < 1e-12 && y.abs() < 1e-12 && (z + 1.0).abs() < 1e-12);
    let pitched = Camera {
        pitch: FRAC_PI_2,
        ..Camera::default()
    };
    let [_, y, z] = pitched.projector().rotate([0.0, 1.0, 0.0]);
    assert!(y.abs() < 1e-12 && (z - 1.0).abs() < 1e-12, "{y} {z}");
}

#[test]
fn core_r50_projector_matches_camera_on_edge_cases() {
    for cam in [
        Camera::default(),
        Camera {
            yaw: 0.3,
            pitch: -0.7,
            distance: 4.0,
        },
        Camera {
            yaw: f64::NAN,
            pitch: 0.0,
            distance: 3.0,
        },
    ] {
        let pr = cam.projector();
        for p in [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 4.0],
            [0.0, 0.0, 10.0],
            [f64::NAN, 0.0, 0.0],
            [f64::INFINITY, 1.0, 0.0],
            [1e308, 1e308, 0.0],
        ] {
            let (want, got) = (
                cam.project(p, (5.0, 6.0), 2.0),
                pr.project(p, (5.0, 6.0), 2.0),
            );
            assert!(
                same_projection(want, got),
                "{cam:?} {p:?}: {want:?} vs {got:?}"
            );
        }
    }
}

proptest! {
    #[test]
    fn core_r50_projector_is_bit_identical_to_camera_project(
        yaw in -2.0 * PI..2.0 * PI,
        pitch in -PI..PI,
        distance in prop_oneof![Just(f64::INFINITY), 0.5f64..50.0],
        p in prop::array::uniform3(-20.0f64..20.0),
        cx in -50.0f64..50.0,
        scale in 0.0f64..20.0,
    ) {
        let cam = Camera { yaw, pitch, distance };
        let want = cam.project(p, (cx, 3.0), scale);
        let got = cam.projector().project(p, (cx, 3.0), scale);
        prop_assert!(same_projection(want, got), "{:?} vs {:?}", want, got);
        // `rotate` is the camera-space point whose depth `project` reports.
        let rotated = cam.projector().rotate(p);
        if let Some((_, _, depth)) = want {
            prop_assert_eq!(depth.to_bits(), rotated[2].to_bits());
        }
    }
}

// ---------------------------------------------------------------- CORE-R51

#[test]
fn core_r51_line_is_the_inclusive_bresenham_segment() {
    let cells = Size::new(10, 5); // 20 × 20 dots
    for (x0, y0, x1, y1) in [
        (1, 2, 17, 9),
        (17, 9, 1, 2),
        (3, 3, 3, 3),
        (0, 19, 19, 0),
        (5, 0, 5, 19),
    ] {
        let mut dots = Dots::new(cells);
        dots.line(
            f64::from(x0),
            f64::from(y0),
            f64::from(x1),
            f64::from(y1),
            A,
        );
        let want: BTreeSet<_> = geom::line(x0, y0, x1, y1).collect();
        assert_eq!(lit(&dots), want, "({x0},{y0})-({x1},{y1})");
        assert!(dots.is_lit(x0, y0) && dots.is_lit(x1, y1));
    }
    // Fractional endpoints are rounded first.
    let mut dots = Dots::new(cells);
    dots.line(0.6, 1.4, 4.2, 1.49, A);
    assert_eq!(lit(&dots), geom::line(1, 1, 4, 1).collect());
}

#[test]
fn core_r51_line_clips_to_the_lattice_and_caps_wild_coordinates() {
    let cells = Size::new(10, 5); // 20 × 20 dots
    let mut dots = Dots::new(cells);
    dots.line(-1e12, 5.0, 1e12, 5.0, A);
    let want: BTreeSet<_> = (0..20).map(|x| (x, 5)).collect();
    assert_eq!(lit(&dots), want);

    // Partly outside: the visible part is the same as for the unclipped segment.
    let mut clipped = Dots::new(cells);
    clipped.line(-10.0, -5.0, 30.0, 15.0, A);
    let want: BTreeSet<_> = geom::line(-10, -5, 30, 15)
        .filter(|&(x, y)| (0..20).contains(&x) && (0..20).contains(&y))
        .collect();
    assert_eq!(lit(&clipped), want);

    let mut wild = Dots::new(cells);
    wild.line(-3e9, -7e9, 8e9, 2e9, A);
    wild.line(f64::MAX, f64::MIN, f64::MIN, f64::MAX, A);
    assert!(lit(&wild).len() <= 4 * (20 + 20));
}

#[test]
fn core_r51_non_finite_endpoints_draw_nothing() {
    let mut dots = Dots::new(Size::new(4, 4));
    for (x0, y0, x1, y1) in [
        (f64::NAN, 0.0, 3.0, 3.0),
        (0.0, 0.0, f64::INFINITY, 3.0),
        (0.0, f64::NEG_INFINITY, 3.0, 3.0),
        (0.0, 0.0, 3.0, f64::NAN),
    ] {
        dots.line(x0, y0, x1, y1, A);
    }
    assert!(lit(&dots).is_empty());
}

// ---------------------------------------------------------------- CORE-R52

fn ramp_grid(size: Size) -> Grid<f32> {
    let mut g = Grid::new(size, 0.0f32);
    for (i, v) in g.as_mut_slice().iter_mut().enumerate() {
        *v = ((i * 37 % 101) as f32).sin_det() * 3.0;
    }
    g
}

#[test]
fn core_r52_empty_map_and_off_range_coordinates_are_safe() {
    let map = SampleMap::default();
    let src = ramp_grid(Size::new(4, 3));
    assert_eq!(map.sample(&src, 0, 0), 0.0);
    assert_eq!(map.nearest(5, -5), (0, 0));

    let mut map = SampleMap::default();
    map.rebuild(Size::new(4, 3), Size::new(9, 7));
    assert_eq!(
        map.sample(&src, -3, 100).to_bits(),
        map.sample(&src, 0, 6).to_bits()
    );
    assert_eq!(
        map.sample(&src, 1000, -1).to_bits(),
        map.sample(&src, 8, 0).to_bits()
    );
    assert_eq!(map.nearest(-1, 99), map.nearest(0, 6));
    // A source of another size is a caller bug; it must still not panic.
    let other = ramp_grid(Size::new(2, 2));
    assert_eq!(map.sample(&other, 3, 3), 0.0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn core_r52_sample_matches_resample_bilinear_and_nearest(
        sw in 1u16..40, sh in 1u16..40, dw in 1u16..60, dh in 1u16..60,
    ) {
        let (ss, ds) = (Size::new(sw, sh), Size::new(dw, dh));
        let src = ramp_grid(ss);
        let mut dst = Grid::new(ds, 0.0f32);
        grid::resample_bilinear(&src, &mut dst);
        // Source cell ids, to read back which cell resample_nearest picked.
        let mut ids = Grid::new(ss, (0, 0));
        for y in 0..i32::from(sh) {
            for x in 0..i32::from(sw) {
                ids.set(x, y, (x, y));
            }
        }
        let mut picked = Grid::new(ds, (-1, -1));
        grid::resample_nearest(&ids, &mut picked);

        let mut map = SampleMap::default();
        map.rebuild(ss, ds);
        for ((x, y), &want) in dst.iter() {
            prop_assert_eq!(map.sample(&src, x, y).to_bits(), want.to_bits(), "({}, {})", x, y);
            prop_assert_eq!(Some(map.nearest(x, y)), picked.get(x, y).copied());
        }
    }

    #[test]
    fn core_r104_row_sampler_is_bit_identical_to_sample(
        sw in 1u16..40, sh in 1u16..40, dw in 1u16..60, dh in 1u16..60, seed in any::<u64>(),
    ) {
        let (ss, ds) = (Size::new(sw, sh), Size::new(dw, dh));
        let mut src = Grid::new(ss, 0.0f32);
        let mut s = seed;
        for v in src.as_mut_slice() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *v = (s >> 40) as f32 / 4096.0 - 2048.0;
        }
        let mut map = SampleMap::default();
        map.rebuild(ss, ds);
        // Off-range rows and columns clamp exactly as `sample` does.
        for y in -2..i32::from(dh) + 2 {
            let row = map.row(&src, y);
            for x in -2..i32::from(dw) + 2 {
                prop_assert_eq!(
                    row.at(x).to_bits(), map.sample(&src, x, y).to_bits(), "({}, {})", x, y
                );
            }
        }
    }
}

#[test]
fn core_r104_row_sampler_of_an_empty_map_or_wrong_source_is_zero() {
    let src = ramp_grid(Size::new(4, 3));
    assert_eq!(SampleMap::default().row(&src, 0).at(0), 0.0);
    let mut map = SampleMap::default();
    map.rebuild(Size::new(4, 3), Size::new(9, 7));
    let other = ramp_grid(Size::new(2, 2));
    assert_eq!(map.row(&other, 3).at(3), 0.0);
}

// ---------------------------------------------------------------- CORE-R102

fn noise_grid(size: Size, seed: u64) -> Grid<f32> {
    let mut g = Grid::new(size, 0.0f32);
    let mut s = seed;
    for v in g.as_mut_slice() {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *v = (s >> 40) as f32 / (1u64 << 24) as f32 * 4.0 - 2.0;
    }
    g
}

#[test]
fn core_r102_cubic_is_exact_at_source_centres_for_equal_sizes() {
    let src = noise_grid(Size::new(13, 7), 3);
    let mut map = SampleMap::default();
    map.rebuild(src.size(), src.size());
    for ((x, y), &v) in src.iter() {
        assert_eq!(
            map.sample_cubic(&src, x, y).to_bits(),
            v.to_bits(),
            "({x}, {y})"
        );
        assert_eq!(map.row_cubic(&src, y).at(x).to_bits(), v.to_bits());
    }
}

#[test]
fn core_r102_cubic_reproduces_a_linear_ramp_away_from_the_edges() {
    let size = Size::new(16, 12);
    let mut src = Grid::new(size, 0.0f32);
    for y in 0..12 {
        for x in 0..16 {
            src.set(x, y, 0.75 * x as f32 - 0.5 * y as f32 + 2.0);
        }
    }
    let dst = Size::new(80, 60); // 5× magnification
    let mut map = SampleMap::default();
    map.rebuild(size, dst);
    for y in 0..60 {
        for x in 0..80 {
            let sx = grid::centre_of(f64::from(x), 80, 16);
            let sy = grid::centre_of(f64::from(y), 60, 12);
            // Taps two cells in from every edge are unclamped.
            if !(1.0..=13.0).contains(&sx) || !(1.0..=9.0).contains(&sy) {
                continue;
            }
            let want = (0.75 * sx - 0.5 * sy + 2.0) as f32;
            let got = map.sample_cubic(&src, x, y);
            assert!((got - want).abs() < 1e-4, "({x}, {y}): {got} vs {want}");
        }
    }
}

#[test]
fn core_r102_cubic_of_an_empty_map_or_wrong_source_is_zero() {
    let src = ramp_grid(Size::new(4, 3));
    assert_eq!(SampleMap::default().sample_cubic(&src, 0, 0), 0.0);
    assert_eq!(SampleMap::default().row_cubic(&src, 0).at(0), 0.0);
    let mut map = SampleMap::default();
    map.rebuild(Size::new(4, 3), Size::new(9, 7));
    let other = ramp_grid(Size::new(2, 2));
    assert_eq!(map.sample_cubic(&other, 3, 3), 0.0);
    assert_eq!(map.row_cubic(&other, 3).at(3), 0.0);
    // Off-range coordinates clamp onto the region, as `sample` does.
    assert_eq!(
        map.sample_cubic(&src, -3, 100).to_bits(),
        map.sample_cubic(&src, 0, 6).to_bits()
    );
}

#[test]
fn core_r102_magnification_is_the_larger_axis_ratio() {
    let mut map = SampleMap::default();
    assert_eq!(map.magnification(), 0.0);
    map.rebuild(Size::new(50, 40), Size::new(200, 100));
    assert_eq!(map.magnification(), 4.0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn core_r102_cubic_never_leaves_the_central_four_values(
        sw in 1u16..30, sh in 1u16..30, dw in 1u16..90, dh in 1u16..90, seed in any::<u64>(),
    ) {
        let (ss, ds) = (Size::new(sw, sh), Size::new(dw, dh));
        let src = noise_grid(ss, seed);
        let mut map = SampleMap::default();
        map.rebuild(ss, ds);
        let at = |x: usize, y: usize| src.as_slice()[y * usize::from(sw) + x];
        let taps = |d: i32, dn: u16, sn: u16| {
            let s = grid::centre_of(f64::from(d), usize::from(dn), usize::from(sn))
                .clamp(0.0, f64::from(sn - 1));
            let i0 = s as usize;
            (i0, (i0 + 1).min(usize::from(sn) - 1))
        };
        for y in 0..i32::from(dh) {
            let row = map.row_cubic(&src, y);
            for x in 0..i32::from(dw) {
                let v = map.sample_cubic(&src, x, y);
                prop_assert_eq!(v.to_bits(), row.at(x).to_bits());
                let ((x0, x1), (y0, y1)) = (taps(x, dw, sw), taps(y, dh, sh));
                let c = [at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)];
                let lo = c.iter().copied().fold(f32::INFINITY, f32::min);
                let hi = c.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                prop_assert!(v.is_finite() && lo <= v && v <= hi, "({}, {}): {} not in [{}, {}]", x, y, v, lo, hi);
            }
        }
    }
}

// ---------------------------------------------------------------- CORE-R53

fn lap(src: &Grid<f32>, edges: Edges) -> Grid<f32> {
    let mut dst = Grid::default();
    grid::laplacian9(src, &mut dst, edges);
    dst
}

#[test]
fn core_r53_point_source_gives_the_nine_point_stencil() {
    let mut src = Grid::new(Size::new(5, 5), 0.0f32);
    src.set(2, 2, 6.0);
    for edges in [Edges::Wrap, Edges::Clip] {
        let dst = lap(&src, edges);
        assert_eq!(dst.size(), src.size());
        assert_eq!(dst.get(2, 2), Some(&-20.0));
        for (x, y) in [(1, 2), (3, 2), (2, 1), (2, 3)] {
            assert_eq!(dst.get(x, y), Some(&4.0), "{edges:?} ({x},{y})");
        }
        for (x, y) in [(1, 1), (3, 1), (1, 3), (3, 3)] {
            assert_eq!(dst.get(x, y), Some(&1.0), "{edges:?} ({x},{y})");
        }
        assert_eq!(dst.get(0, 0), Some(&0.0));
    }
}

#[test]
fn core_r53_quadratic_has_constant_laplacian_in_the_interior() {
    // ∇²(x² + y²) = 4, exact for the 9-point stencil.
    let mut src = Grid::new(Size::new(8, 6), 0.0f32);
    for y in 0..6 {
        for x in 0..8 {
            src.set(x, y, (x * x + y * y) as f32);
        }
    }
    let dst = lap(&src, Edges::Clip);
    for y in 1..5 {
        for x in 1..7 {
            assert_eq!(dst.get(x, y), Some(&4.0), "({x},{y})");
        }
    }
}

#[test]
fn core_r53_clip_is_zero_flux_with_ghosts_mirroring_the_edge() {
    // A 3×1 row [0, 0, 1]: at x = 2 the ghosts E, N, S (and NE, SE) copy edge cells,
    // so 4·(W + E + N + S) + (NW + NE + SW + SE) − 20·c = 4·3 + 2 − 20 = −6.
    let mut src = Grid::new(Size::new(3, 1), 0.0f32);
    src.set(2, 0, 1.0);
    let dst = lap(&src, Edges::Clip);
    assert_eq!(dst.as_slice(), &[0.0, 1.0, -1.0]);
    // Zero flux: no mass leaves through the walls.
    assert_eq!(dst.as_slice().iter().sum::<f32>(), 0.0);
    // One cell has no neighbours but itself.
    let one = Grid::new(Size::new(1, 1), 7.5f32);
    assert_eq!(lap(&one, Edges::Clip).as_slice(), &[0.0]);
    assert_eq!(lap(&one, Edges::Wrap).as_slice(), &[0.0]);
}

#[test]
fn core_r53_dst_follows_src_including_empty() {
    let mut dst = Grid::new(Size::new(9, 9), 1.0f32);
    grid::laplacian9(&Grid::new(Size::new(3, 2), 1.0), &mut dst, Edges::Wrap);
    assert_eq!((dst.width(), dst.height()), (3, 2));
    grid::laplacian9(&Grid::default(), &mut dst, Edges::Wrap);
    assert!(dst.is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn core_r53_constant_is_exactly_zero(
        w in 1u16..12, h in 1u16..12, c in -1e30f32..1e30, wrap in any::<bool>(),
    ) {
        let edges = if wrap { Edges::Wrap } else { Edges::Clip };
        let dst = lap(&Grid::new(Size::new(w, h), c), edges);
        prop_assert!(dst.as_slice().iter().all(|&v| v == 0.0));
    }

    #[test]
    fn core_r53_mirrored_input_gives_bitwise_mirrored_output(
        w in 1u16..10, h in 1u16..10, seed in any::<u64>(), wrap in any::<bool>(),
    ) {
        let edges = if wrap { Edges::Wrap } else { Edges::Clip };
        let mut rng = SplitMix64::new(seed);
        let mut src = Grid::new(Size::new(w, h), 0.0f32);
        for v in src.as_mut_slice() {
            *v = (rng.next_f64() * 2.0 - 1.0) as f32;
        }
        let (wi, hi) = (i32::from(w), i32::from(h));
        let mut lr = src.clone();
        let mut ud = src.clone();
        for ((x, y), &v) in src.iter() {
            lr.set(wi - 1 - x, y, v);
            ud.set(x, hi - 1 - y, v);
        }
        let (out, out_lr, out_ud) = (lap(&src, edges), lap(&lr, edges), lap(&ud, edges));
        for ((x, y), v) in out.iter() {
            prop_assert_eq!(out_lr.get(wi - 1 - x, y).map(|v| v.to_bits()), Some(v.to_bits()));
            prop_assert_eq!(out_ud.get(x, hi - 1 - y).map(|v| v.to_bits()), Some(v.to_bits()));
        }
        if wrap {
            let sum: f64 = out.as_slice().iter().map(|&v| f64::from(v)).sum();
            prop_assert!(sum.abs() < 1e-4, "Σ = {}", sum);
        }
    }
}

// ---------------------------------------------------------------- CORE-R54

fn within(points: &[[f64; 2]], p: [f64; 2], r: f64) -> BTreeSet<usize> {
    points
        .iter()
        .enumerate()
        .filter(|(_, q)| (q[0] - p[0]).hypot(q[1] - p[1]) <= r)
        .map(|(i, _)| i)
        .collect()
}

fn visits(hash: &SpatialHash, p: [f64; 2], r: f64) -> Vec<usize> {
    let mut out = Vec::new();
    hash.for_each_within(p, r, |i| out.push(i));
    out
}

#[test]
fn core_r54_bucket_order_is_stable() {
    let mut hash = SpatialHash::new(10.0, 10.0, 2.0);
    let pts = [[1.0, 1.0]; 5];
    hash.rebuild(&pts);
    assert_eq!(visits(&hash, [1.0, 1.0], 0.5), vec![0, 1, 2, 3, 4]);
    assert_eq!(hash.len(), 5);
}

#[test]
fn core_r54_out_of_range_and_non_finite_points_clamp_to_the_border() {
    let mut hash = SpatialHash::new(10.0, 10.0, 1.0);
    let pts = [
        [-5.0, 3.0],
        [f64::NAN, f64::INFINITY],
        [99.0, 99.0],
        [5.0, 5.0],
    ];
    hash.rebuild(&pts);
    // Point 0 lives in the left border bucket, so a query near it still finds it.
    assert!(visits(&hash, [-5.0, 3.0], 0.1).contains(&0));
    assert!(visits(&hash, [0.1, 3.0], 0.2).contains(&0));
    assert!(visits(&hash, [99.0, 99.0], 0.1).contains(&2));
    // Every index is somewhere, exactly once.
    let mut all = visits(&hash, [5.0, 5.0], f64::INFINITY);
    all.sort_unstable();
    assert_eq!(all, vec![0, 1, 2, 3]);
    // Degenerate queries and constructors never panic.
    assert!(visits(&hash, [f64::NAN, 1.0], 3.0).is_empty());
    assert!(visits(&hash, [1.0, 1.0], -1.0).is_empty());
    for (w, h, c) in [
        (0.0, 0.0, 0.0),
        (f64::NAN, 1.0, 1.0),
        (1e300, 1e300, 1e-300),
        (5.0, 5.0, f64::NAN),
    ] {
        let mut hash = SpatialHash::new(w, h, c);
        hash.rebuild(&pts);
        let mut all = visits(&hash, [0.0, 0.0], f64::INFINITY);
        all.sort_unstable();
        assert_eq!(all, vec![0, 1, 2, 3], "{w} {h} {c}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn core_r54_visits_every_neighbour_exactly_once_in_a_fixed_order(
        pts in prop::collection::vec(prop::array::uniform2(-5.0f64..45.0), 0..120),
        p in prop::array::uniform2(-10.0f64..50.0),
        r in 0.0f64..12.0,
        cell in 0.5f64..8.0,
    ) {
        let mut hash = SpatialHash::new(40.0, 30.0, cell);
        hash.rebuild(&pts);
        let got = visits(&hash, p, r);
        let unique: BTreeSet<usize> = got.iter().copied().collect();
        prop_assert_eq!(unique.len(), got.len(), "duplicate visits");
        prop_assert!(within(&pts, p, r).is_subset(&unique));
        prop_assert!(got.iter().all(|&i| i < pts.len()));
        prop_assert_eq!(visits(&hash, p, r), got.clone());
        // Rebuilding from the same points gives the same order.
        hash.rebuild(&pts);
        prop_assert_eq!(visits(&hash, p, r), got);
    }
}

// ---------------------------------------------------------------- CORE-R55

#[test]
fn core_r55_density_deposits_decays_and_thresholds_into_dots() {
    let cells = Size::new(3, 2);
    let mut d = Density::new(cells);
    assert_eq!(d.dot_size(), Size::new(6, 8));
    d.deposit(1.4, 2.6, 1.0);
    d.deposit(1.0, 3.0, 0.5);
    d.deposit(5.0, 7.0, 0.25);
    d.deposit(f64::NAN, 1.0, 9.0);
    d.deposit(1.0, f64::INFINITY, 9.0);
    d.deposit(-1.0, 0.0, 9.0);
    d.deposit(6.0, 0.0, 9.0);
    assert_eq!(d.get(1, 3), Some(1.5));
    assert_eq!(d.get(5, 7), Some(0.25));
    assert_eq!(d.get(6, 0), None);
    d.decay(0.5);
    assert_eq!(d.get(1, 3), Some(0.75));
    assert_eq!(d.get(5, 7), Some(0.125));

    let mut dots = Dots::new(cells);
    d.to_dots(&mut dots, 0.125, |v| ink::seq(f64::from(v)));
    assert_eq!(lit(&dots), BTreeSet::from([(1, 3), (5, 7)]));
    let c = {
        let mut c = Canvas::new(cells, GlyphMode::Unicode);
        dots.blit(&mut c);
        c
    };
    assert_eq!(c.get(0, 0).unwrap().ink, ink::seq(0.75));

    d.clear();
    assert_eq!(d.get(1, 3), Some(0.0));
    d.deposit(0.0, 0.0, 1.0);
    d.resize(Size::new(1, 1));
    assert_eq!(d.dot_size(), Size::new(2, 4));
    assert_eq!(d.get(0, 0), Some(0.0));
}

#[test]
fn core_r55_decay_is_clamped_to_the_unit_interval() {
    let mut d = Density::new(Size::new(1, 1));
    d.deposit(0.0, 0.0, 2.0);
    d.decay(3.0);
    assert_eq!(d.get(0, 0), Some(2.0));
    d.decay(-1.0);
    assert_eq!(d.get(0, 0), Some(0.0));
}

// ---------------------------------------------------------------- CORE-R56

#[test]
fn core_r56_soft_clip_saturates_exactly_and_tracks_tanh() {
    for x in [3.0, 3.5, 100.0, f64::INFINITY] {
        assert_eq!(ink::soft_clip(x), 1.0);
        assert_eq!(ink::soft_clip(-x), -1.0);
    }
    assert_eq!(ink::soft_clip(0.0), 0.0);
    assert_eq!(ink::soft_clip(f64::NAN), 0.0);
    for i in -300..=300 {
        let x = f64::from(i) * 0.01;
        assert!((ink::soft_clip(x) - x.tanh()).abs() < 0.03, "{x}");
    }
}

#[test]
fn core_r56_value_functions_invert_seq_and_div_on_every_level() {
    for level in 0..=255u8 {
        let s = Ink::new(Role::Seq, level);
        let v = ink::seq_value(s);
        assert!((0.0..=1.0).contains(&v));
        assert_eq!(ink::seq(v), s, "seq level {level}");
        let d = Ink::new(Role::Div, level);
        let v = ink::div_value(d);
        assert!((-1.0..=1.0).contains(&v));
        assert_eq!(ink::div(v), d, "div level {level}");
    }
    assert_eq!(ink::seq_value(ink::seq(1.0)), 1.0);
    assert_eq!(ink::div_value(ink::div(-1.0)), -1.0);
    assert_eq!(ink::div_value(ink::div(0.0)), 0.0);
    assert_eq!(ink::div_value(ink::div(1.0)), 1.0);
}

proptest! {
    #[test]
    fn core_r56_soft_clip_is_odd_bounded_and_monotone(a in -10.0f64..10.0, b in -10.0f64..10.0) {
        let (fa, fb) = (ink::soft_clip(a), ink::soft_clip(b));
        prop_assert!((-1.0..=1.0).contains(&fa));
        prop_assert_eq!(ink::soft_clip(-a), -fa);
        if a <= b {
            prop_assert!(fa <= fb, "{} -> {}, {} -> {}", a, fa, b, fb);
        }
    }
}

// ---------------------------------------------------------------- CORE-R57

#[test]
fn core_r57_splitmix64_matches_the_reference_stream() {
    let mut r = SplitMix64::new(0);
    assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
    // Same seed, same stream.
    let (mut a, mut b) = (SplitMix64::new(99), SplitMix64::new(99));
    assert!((0..100).all(|_| a.next_u64() == b.next_u64()));
}

#[test]
fn core_r57_next_f64_uses_the_top_53_bits() {
    for seed in [0, 1, 0xDEAD_BEEF, u64::MAX] {
        let bits = SplitMix64::new(seed).next_u64() >> 11;
        let v = SplitMix64::new(seed).next_f64();
        assert_eq!(v, bits as f64 / (1u64 << 53) as f64);
        assert!((0.0..1.0).contains(&v));
    }
}

#[test]
fn core_r57_from_rng_is_deterministic_and_below_is_bounded_and_even() {
    let mut a = SplitMix64::from_rng(&mut ChaCha8Rng::seed_from_u64(7));
    let mut b = SplitMix64::from_rng(&mut ChaCha8Rng::seed_from_u64(7));
    assert!((0..50).all(|_| a.next_u64() == b.next_u64()));

    assert_eq!(a.below(0), 0);
    assert_eq!(a.below(1), 0);
    let mut counts = [0u32; 10];
    for _ in 0..100_000 {
        let k = a.below(10);
        assert!(k < 10);
        counts[k as usize] += 1;
    }
    // 10 000 expected per bin; sd ≈ 95, so ±500 is > 5 sd.
    assert!(
        counts.iter().all(|&c| c.abs_diff(10_000) < 500),
        "{counts:?}"
    );
    assert!(a.below(u32::MAX) < u32::MAX);
}

// ---------------------------------------------------------------- CORE-R58

#[test]
fn core_r58_uniform_density_is_exact() {
    let icdf = InverseCdf::<64>::build(-2.0, 3.0, |_| 0.7);
    for i in 0..=100 {
        let u = f64::from(i) / 100.0;
        let want = -2.0 + u * 5.0;
        assert_eq!(icdf.sample(u), want.clamp(-2.0, 3.0), "u = {u}");
    }
}

#[test]
fn core_r58_degenerate_densities_fall_back_to_uniform() {
    for density in [
        (|_| 0.0) as fn(f64) -> f64,
        |_| f64::NAN,
        |_| f64::INFINITY,
        |_| -1.0,
    ] {
        let icdf = InverseCdf::<16>::build(0.0, 4.0, density);
        assert_eq!(icdf.sample(0.25), 1.0);
        assert_eq!(icdf.sample(0.5), 2.0);
    }
    // Degenerate tables and inputs never panic and stay in range.
    let tiny = InverseCdf::<1>::build(0.0, 1.0, |x| x);
    assert!((0.0..=1.0).contains(&tiny.sample(0.3)));
    let empty = InverseCdf::<0>::build(0.0, 1.0, |x| x);
    assert!((0.0..=1.0).contains(&empty.sample(0.3)));
    let icdf = InverseCdf::<8>::build(0.0, 1.0, |x| x);
    assert_eq!(icdf.sample(f64::NAN), icdf.sample(0.0));
    assert_eq!(icdf.sample(-3.0), icdf.sample(0.0));
    assert_eq!(icdf.sample(7.0), icdf.sample(1.0));
    let swapped = InverseCdf::<8>::build(5.0, 1.0, |_| 1.0);
    assert!((1.0..=5.0).contains(&swapped.sample(0.5)));
}

#[test]
fn core_r58_linear_density_inverts_to_a_square_root() {
    // p(x) = 2x on [0, 1] ⇒ F(x) = x² ⇒ F⁻¹(u) = √u.
    let icdf = InverseCdf::<256>::build(0.0, 1.0, |x| 2.0 * x);
    for i in 0..=200 {
        let u = f64::from(i) / 200.0;
        let got = icdf.sample(u);
        assert!(
            (got - u.sqrt()).abs() < 0.01,
            "u = {u}: {got} vs {}",
            u.sqrt()
        );
    }
}

#[test]
fn core_r58_samples_avoid_zero_density_regions() {
    let icdf = InverseCdf::<64>::build(0.0, 1.0, |x| if x > 0.5 { 1.0 } else { 0.0 });
    for i in 0..=100 {
        let got = icdf.sample(f64::from(i) / 100.0);
        assert!((0.5..=1.0).contains(&got), "{got}");
    }
}

proptest! {
    #[test]
    fn core_r58_sample_is_monotone_and_in_range(
        k in 0.5f64..20.0, c in 0.0f64..2.0, a in -5.0f64..5.0, len in 0.0f64..10.0,
        u in 0.0f64..=1.0, v in 0.0f64..=1.0,
    ) {
        let b = a + len;
        let icdf = InverseCdf::<32>::build(a, b, |x| (k * x).sin_det().abs() + c);
        let (su, sv) = (icdf.sample(u), icdf.sample(v));
        prop_assert!((a..=b).contains(&su), "{} not in [{}, {}]", su, a, b);
        if u <= v {
            prop_assert!(su <= sv, "{} -> {}, {} -> {}", u, su, v, sv);
        }
    }
}

// ---------------------------------------------------------------- CORE-R59

#[test]
fn core_r59_tridiag_solves_a_diagonally_dominant_system() {
    let n = 50;
    let mut rng = SplitMix64::new(3);
    let mut r = || rng.next_f64() * 2.0 - 1.0;
    let sub: Vec<f64> = (0..n).map(|_| r()).collect();
    let sup: Vec<f64> = (0..n).map(|_| r()).collect();
    let diag: Vec<f64> = (0..n).map(|_| 3.0 + r()).collect();
    let x_true: Vec<f64> = (0..n).map(|_| r()).collect();
    let rhs: Vec<f64> = (0..n)
        .map(|i| {
            let left = if i > 0 { sub[i] * x_true[i - 1] } else { 0.0 };
            let right = if i + 1 < n {
                sup[i] * x_true[i + 1]
            } else {
                0.0
            };
            left + diag[i] * x_true[i] + right
        })
        .collect();
    let mut x = rhs.clone();
    let mut scratch = vec![0.0; n];
    assert!(ode::tridiag_solve(&sub, &diag, &sup, &mut x, &mut scratch));
    for (got, want) in x.iter().zip(&x_true) {
        assert!((got - want).abs() < 1e-12, "{got} vs {want}");
    }
    // One unknown: x = rhs / diag.
    let mut one = [6.0];
    assert!(ode::tridiag_solve(
        &[9.0],
        &[2.0],
        &[9.0],
        &mut one,
        &mut [0.0]
    ));
    assert_eq!(one, [3.0]);
    // No unknowns: trivially solved.
    assert!(ode::tridiag_solve(&[], &[], &[], &mut [], &mut []));
}

#[test]
fn core_r59_mismatched_lengths_and_zero_pivots_leave_rhs_unchanged() {
    let orig = [1.0, 2.0, 3.0];
    let mut rhs = orig;
    let three = [1.0; 3];
    assert!(!ode::tridiag_solve(
        &three,
        &three,
        &[1.0; 2],
        &mut rhs,
        &mut [0.0; 3]
    ));
    assert!(!ode::tridiag_solve(
        &three,
        &three,
        &three,
        &mut rhs,
        &mut [0.0; 2]
    ));
    assert!(!ode::tridiag_solve(
        &[1.0; 4],
        &three,
        &three,
        &mut rhs,
        &mut [0.0; 3]
    ));
    assert_eq!(rhs, orig);
    // First pivot zero.
    assert!(!ode::tridiag_solve(
        &three,
        &[0.0, 1.0, 1.0],
        &three,
        &mut rhs,
        &mut [0.0; 3]
    ));
    assert_eq!(rhs, orig);
    // Second pivot zero: 1 − sub[1]·sup[0]/diag[0] = 1 − 1 = 0.
    assert!(!ode::tridiag_solve(
        &three,
        &three,
        &three,
        &mut rhs,
        &mut [0.0; 3]
    ));
    assert_eq!(rhs, orig);
}
