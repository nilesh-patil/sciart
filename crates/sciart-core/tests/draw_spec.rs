//! SPEC-001 CORE-R22..R27: drawing helpers.

use proptest::prelude::*;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;

const G: Glyph = Glyph::ascii('#');
const INK: Ink = Ink::new(Role::Structure, 200);

fn drawn(c: &Canvas) -> Vec<(i32, i32)> {
    let w = i32::from(c.size().width());
    c.cells()
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.ch != ' ')
        .map(|(i, _)| (i as i32 % w, i as i32 / w))
        .collect()
}

fn norm_dist(x: i32, y: i32, cx: f64, cy: f64) -> f64 {
    (((f64::from(x) - cx) / ASPECT).powi(2) + (f64::from(y) - cy).powi(2)).sqrt()
}

#[test]
fn core_r22_orbit_cells_lie_on_the_curve_without_gaps() {
    let mut c = Canvas::new(Size::new(80, 30), GlyphMode::Ascii);
    let (cx, cy, r) = (40.0, 15.0, 10.0);
    draw::orbit(&mut c, cx, cy, r, G, INK);
    let pts = drawn(&c);
    assert!(pts.len() > 40, "outline too sparse: {}", pts.len());
    for &(x, y) in &pts {
        // within one cell of the ideal curve (one cell = 1/ASPECT rows horizontally)
        assert!(
            (norm_dist(x, y, cx, cy) - r).abs() <= 1.0,
            "({x},{y}) off curve"
        );
    }
    // 8-connected: every drawn cell has a drawn neighbour
    for &(x, y) in &pts {
        let has_neighbour = pts
            .iter()
            .any(|&(a, b)| (a, b) != (x, y) && (a - x).abs() <= 1 && (b - y).abs() <= 1);
        assert!(has_neighbour, "gap at ({x},{y})");
    }
}

#[test]
fn core_r22_degenerate_orbits_draw_nothing_and_never_panic() {
    let mut c = Canvas::new(Size::new(10, 5), GlyphMode::Ascii);
    for r in [0.0, -3.0, f64::NAN, f64::INFINITY] {
        draw::orbit(&mut c, 5.0, 2.0, r, G, INK);
    }
    assert!(drawn(&c).is_empty());
    draw::orbit(&mut c, -1e9, 1e9, 3.0, G, INK);
    draw::orbit(&mut c, f64::NAN, 2.0, 3.0, G, INK);
}

#[test]
fn core_r23_disc_visits_exactly_the_inside_with_normalized_args() {
    let mut c = Canvas::new(Size::new(60, 24), GlyphMode::Ascii);
    let (cx, cy, r) = (30.0, 12.0, 6.0);
    let mut calls = 0;
    draw::disc(&mut c, cx, cy, r, |d, nx, ny| {
        calls += 1;
        assert!((0.0..=1.0).contains(&d), "d={d}");
        assert!((-1.0..=1.0).contains(&nx) && (-1.0..=1.0).contains(&ny));
        Some((G, INK))
    });
    let pts = drawn(&c);
    assert_eq!(calls, pts.len());
    for &(x, y) in &pts {
        assert!(norm_dist(x, y, cx, cy) <= r + 1e-9);
    }
    // every cell inside is drawn
    for y in 0..24 {
        for x in 0..60 {
            if norm_dist(x, y, cx, cy) <= r - 1e-9 {
                assert!(pts.contains(&(x, y)), "missed ({x},{y})");
            }
        }
    }
}

#[test]
fn core_r23_disc_none_leaves_cells_and_tiny_radius_hits_center() {
    let mut c = Canvas::new(Size::new(9, 5), GlyphMode::Ascii);
    draw::disc(&mut c, 4.0, 2.0, 3.0, |_, _, _| None);
    assert!(drawn(&c).is_empty());

    let mut calls = vec![];
    draw::disc(&mut c, 4.0, 2.0, 0.2, |d, _, _| {
        calls.push(d);
        Some((G, INK))
    });
    assert_eq!(calls, vec![0.0]);
    assert_eq!(drawn(&c), vec![(4, 2)]);

    for r in [0.0, -1.0, f64::NAN] {
        draw::disc(&mut c, 4.0, 2.0, r, |_, _, _| panic!("must not be called"));
    }
}

#[test]
fn core_r23_disc_far_off_canvas_is_cheap_and_safe() {
    let mut c = Canvas::new(Size::new(10, 5), GlyphMode::Ascii);
    let mut calls = 0;
    draw::disc(&mut c, 1e12, -1e12, 1e6, |_, _, _| {
        calls += 1;
        None
    });
    assert_eq!(calls, 0);
}

#[test]
fn core_r24_field_visits_every_cell_once() {
    let mut c = Canvas::new(Size::new(7, 3), GlyphMode::Ascii);
    let mut seen = [0u8; 21];
    draw::field(&mut c, |x, y| {
        seen[(y * 7 + x) as usize] += 1;
        (x == y).then_some((G, INK))
    });
    assert!(seen.iter().all(|&n| n == 1));
    assert_eq!(drawn(&c), vec![(0, 0), (1, 1), (2, 2)]);
}

#[test]
fn core_r25_ramp_maps_and_clamps() {
    let gs = [Glyph::ascii('a'), Glyph::ascii('b'), Glyph::ascii('c')];
    assert_eq!(draw::ramp(0.0, &gs), gs[0]);
    assert_eq!(draw::ramp(0.5, &gs), gs[1]);
    assert_eq!(draw::ramp(1.0, &gs), gs[2]);
    assert_eq!(draw::ramp(-4.0, &gs), gs[0]);
    assert_eq!(draw::ramp(9.0, &gs), gs[2]);
    assert_eq!(draw::ramp(f64::NAN, &gs), gs[0]);
}

#[test]
fn core_r26_text_centered_is_centered_and_clipped() {
    let mut c = Canvas::new(Size::new(9, 2), GlyphMode::Ascii);
    draw::text_centered(&mut c, 0, "abc", INK);
    assert_eq!(c.to_lines()[0], "   abc   ");
    draw::text_centered(&mut c, 1, "0123456789ABC", INK);
    assert_eq!(c.to_lines()[1], "23456789A");
    draw::text_centered(&mut c, 5, "off", INK);
}

#[test]
fn core_r27_fit_radius() {
    // width-limited: 40 / (2*2) = 10 ; height 50/2 = 25
    assert!((draw::fit_radius(Size::new(40, 50), 1.0) - 10.0).abs() < 1e-9);
    // height-limited
    assert!((draw::fit_radius(Size::new(200, 20), 0.5) - 5.0).abs() < 1e-9);
    assert_eq!(draw::fit_radius(Size::new(1, 1), -2.0), 0.0);
}

proptest! {
    #[test]
    fn core_r22_r23_never_panic(
        w in 1u16..60, h in 1u16..30,
        cx in -100.0f64..100.0, cy in -100.0f64..100.0, r in -5.0f64..80.0,
    ) {
        let mut c = Canvas::new(Size::new(w, h), GlyphMode::Unicode);
        draw::orbit(&mut c, cx, cy, r, G, INK);
        draw::disc(&mut c, cx, cy, r, |d, _, _| Some((draw::ramp(d, &[G, Glyph::ascii('.')]), INK)));
        prop_assert_eq!(c.cells().len(), usize::from(w) * usize::from(h));
    }
}
