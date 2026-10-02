//! SPEC-001 CORE-R40..R42, R82: cell background, braille `Dots` (and its disc) and half-block `Pixels`
//! (ADR-0010).

use proptest::prelude::*;
use sciart_core::prelude::*;
use sciart_core::raster::{self, Dots, Pixels};

const A: Ink = Ink::new(Role::Star, 200);
const B: Ink = Ink::new(Role::Hot, 90);
const C: Ink = Ink::new(Role::Cold, 200);
const X: Glyph = Glyph::ascii('x');

fn cell(c: &Canvas, x: i32, y: i32) -> Cell {
    *c.get(x, y).expect("in bounds")
}

// ---------------------------------------------------------------- CORE-R40

#[test]
fn core_r40_blank_cells_have_no_background() {
    assert_eq!(Cell::BLANK.bg, Ink::BLANK);
    let mut c = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
    assert!(c.cells().iter().all(|cell| cell.bg == Ink::BLANK));
    c.put_bg(0, 0, X, A, B);
    c.clear();
    assert!(c.cells().iter().all(|cell| *cell == Cell::BLANK));
}

#[test]
fn core_r40_put_bg_sets_both_inks_and_put_resets_bg() {
    let mut c = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
    c.put_bg(1, 1, Glyph::new('▀', '#'), A, B);
    let got = cell(&c, 1, 1);
    assert_eq!((got.ch, got.ink, got.bg), ('▀', A, B));
    assert_eq!(c.to_lines(), vec!["   ".to_owned(), " ▀ ".to_owned()]);

    // Overdraw with plain `put` must not leave a stale background behind.
    c.put(1, 1, X, C);
    assert_eq!(
        cell(&c, 1, 1),
        Cell {
            ch: 'x',
            ink: C,
            bg: Ink::BLANK
        }
    );
}

#[test]
fn core_r40_put_bg_follows_bounds_and_glyph_mode_rules() {
    let mut c = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
    for (x, y) in [(-1, 0), (0, -1), (3, 0), (0, 2), (i32::MIN, i32::MAX)] {
        c.put_bg(x, y, X, A, B); // CORE-R3: silent no-op
    }
    assert!(c.cells().iter().all(|cell| *cell == Cell::BLANK));

    // CORE-R4: wide glyphs fall back to ASCII in Unicode mode, ASCII mode stores ASCII.
    c.put_bg(0, 0, Glyph::new('🌍', 'o'), A, B);
    assert_eq!(cell(&c, 0, 0).ch, 'o');
    c.set_mode(GlyphMode::Ascii);
    c.put_bg(2, 1, Glyph::new('▀', '#'), A, B);
    assert_eq!((cell(&c, 2, 1).ch, cell(&c, 2, 1).bg), ('#', B));
}

// ---------------------------------------------------------------- CORE-R41

/// Standard Unicode braille numbering: dots 1-2-3-7 down the left column, 4-5-6-8 down
/// the right, and dot `n` is bit `n - 1`.
const STANDARD_BIT: [((i32, i32), u8); 8] = [
    ((0, 0), 0x01),
    ((0, 1), 0x02),
    ((0, 2), 0x04),
    ((1, 0), 0x08),
    ((1, 1), 0x10),
    ((1, 2), 0x20),
    ((0, 3), 0x40),
    ((1, 3), 0x80),
];

fn blit_one(dots: &Dots, size: Size, mode: GlyphMode) -> Canvas {
    let mut c = Canvas::new(size, mode);
    dots.blit(&mut c);
    c
}

#[test]
fn core_r41_lattice_is_two_by_four_dots_per_cell() {
    assert_eq!(Dots::new(Size::new(3, 2)).dot_size(), Size::new(6, 8));
    assert_eq!(Dots::new(Size::new(1, 1)).dot_size(), Size::new(2, 4));
    assert_eq!((Dots::CELL_W, Dots::CELL_H), (2, 4));
}

#[test]
fn core_r41_each_dot_lights_its_standard_braille_bit() {
    for ((dx, dy), bit) in STANDARD_BIT {
        let mut dots = Dots::new(Size::new(2, 2));
        // Use the second cell so the cell → dot offset is exercised too.
        dots.set(2 + dx, 4 + dy, A);
        assert!(dots.is_lit(2 + dx, 4 + dy));
        assert_eq!(raster::dot_bit(2 + dx, 4 + dy), bit);
        let c = blit_one(&dots, Size::new(2, 2), GlyphMode::Unicode);
        let expected = char::from_u32(0x2800 + u32::from(bit)).unwrap();
        assert_eq!(
            cell(&c, 1, 1),
            Cell {
                ch: expected,
                ink: A,
                bg: Ink::BLANK
            },
            "{dx},{dy}"
        );
        assert_eq!(cell(&c, 0, 0), Cell::BLANK);
    }
}

#[test]
fn core_r41_all_dots_make_the_full_braille_cell() {
    let mut dots = Dots::new(Size::new(1, 1));
    for y in 0..4 {
        for x in 0..2 {
            dots.set(x, y, A);
        }
    }
    assert_eq!(
        cell(&blit_one(&dots, Size::new(1, 1), GlyphMode::Unicode), 0, 0).ch,
        '⣿'
    );
}

#[test]
fn core_r41_ascii_fallback_is_by_dot_count() {
    let expected = ['.', ':', '+', '+', '#', '#', '#', '#'];
    for n in 1..=8 {
        let mut dots = Dots::new(Size::new(1, 1));
        for &((x, y), _) in &STANDARD_BIT[..n] {
            dots.set(x, y, A);
        }
        let got = cell(&blit_one(&dots, Size::new(1, 1), GlyphMode::Ascii), 0, 0).ch;
        assert_eq!(got, expected[n - 1], "{n} dots");
    }
    for mask in 0..=255u8 {
        let g = raster::braille(mask);
        assert_eq!(g.unicode as u32, 0x2800 + u32::from(mask));
        assert!(g.ascii.is_ascii());
    }
}

#[test]
fn core_r41_cell_keeps_the_brightest_dot_ink_and_ties_go_to_the_last() {
    let mut dots = Dots::new(Size::new(1, 1));
    dots.set(0, 0, A); // level 200
    dots.set(1, 0, B); // level 90: dimmer, ignored
    let got = cell(&blit_one(&dots, Size::new(1, 1), GlyphMode::Unicode), 0, 0);
    assert_eq!(got.ink, A);
    dots.set(1, 1, C); // level 200 again: tie, last set wins
    let got = cell(&blit_one(&dots, Size::new(1, 1), GlyphMode::Unicode), 0, 0);
    assert_eq!(got.ink, C);
    assert_eq!(got.ch, char::from_u32(0x2800 + 0x01 + 0x08 + 0x10).unwrap());
}

#[test]
fn core_r41_div_intensity_is_distance_from_zero_so_negative_lobes_keep_their_peak() {
    let ink_of = |dots: &Dots| cell(&blit_one(dots, Size::new(1, 1), GlyphMode::Unicode), 0, 0).ink;
    // A strong negative value (level 10, |10 − 128| = 118) beats a weak positive one
    // (level 150, |150 − 128| = 22), whichever order they are set in.
    let strong_neg = Ink::new(Role::Div, 10);
    let weak_pos = Ink::new(Role::Div, 150);
    for (first, second) in [(strong_neg, weak_pos), (weak_pos, strong_neg)] {
        let mut dots = Dots::new(Size::new(1, 1));
        dots.set(0, 0, first);
        dots.set(1, 0, second);
        assert_eq!(ink_of(&dots), strong_neg, "{first:?} then {second:?}");
    }
    // Equal distance from zero is a tie: the last set wins.
    let mut dots = Dots::new(Size::new(1, 1));
    dots.set(0, 0, Ink::new(Role::Div, 28)); // |28 − 128| = 100
    dots.set(1, 0, Ink::new(Role::Div, 228)); // |228 − 128| = 100
    assert_eq!(ink_of(&dots), Ink::new(Role::Div, 228));
    // Other roles still compare by level: 10 < 90.
    let mut dots = Dots::new(Size::new(1, 1));
    dots.set(0, 0, B);
    dots.set(1, 0, Ink::new(Role::Seq, 10));
    assert_eq!(ink_of(&dots), B);
}

#[test]
fn core_r41_blit_leaves_unlit_cells_untouched() {
    let mut c = Canvas::new(Size::new(3, 1), GlyphMode::Unicode);
    for x in 0..3 {
        c.put(x, 0, X, B);
    }
    let mut dots = Dots::new(Size::new(3, 1));
    dots.set(2, 1, A); // cell 1
    dots.blit(&mut c);
    assert_eq!(cell(&c, 0, 0).ch, 'x');
    assert_eq!(cell(&c, 1, 0).ch, '⠂');
    assert_eq!(cell(&c, 2, 0).ch, 'x');
}

#[test]
fn core_r41_off_lattice_and_non_finite_points_are_ignored() {
    let mut dots = Dots::new(Size::new(3, 2));
    for (x, y) in [(-1, 0), (0, -1), (6, 0), (0, 8), (i32::MIN, i32::MAX)] {
        dots.set(x, y, A);
        assert!(!dots.is_lit(x, y));
    }
    for (x, y) in [
        (f64::NAN, 1.0),
        (1.0, f64::INFINITY),
        (f64::NEG_INFINITY, 0.0),
        (1e300, 1.0),
        (-0.6, 1.0),
        (5.6, 1.0),
    ] {
        dots.plot(x, y, A);
    }
    let c = blit_one(&dots, Size::new(3, 2), GlyphMode::Unicode);
    assert!(c.cells().iter().all(|cell| *cell == Cell::BLANK));

    // `plot` rounds to the nearest dot.
    dots.plot(1.4, 2.6, A);
    assert!(dots.is_lit(1, 3));
    dots.plot(-0.4, 7.4, A);
    assert!(dots.is_lit(0, 7));
}

#[test]
fn core_r41_clear_and_resize_leave_an_empty_lattice() {
    let mut dots = Dots::new(Size::new(3, 2));
    dots.set(1, 1, A);
    dots.clear();
    assert!(!dots.is_lit(1, 1));
    dots.set(1, 1, A);
    dots.resize(Size::new(3, 2));
    assert!(!dots.is_lit(1, 1));
    dots.resize(Size::new(5, 1));
    assert_eq!(dots.dot_size(), Size::new(10, 4));
    dots.set(9, 3, A);
    assert!(dots.is_lit(9, 3));
    assert!(!dots.is_lit(1, 5));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn core_r41_blit_mask_matches_lit_dots(
        w in 1u16..8, h in 1u16..6,
        pts in prop::collection::vec((-4i32..20, -4i32..28, 0u8..=255), 0..40),
    ) {
        let size = Size::new(w, h);
        let mut dots = Dots::new(size);
        for &(x, y, level) in &pts {
            dots.set(x, y, Ink::new(Role::Particle, level));
        }
        let c = blit_one(&dots, size, GlyphMode::Unicode);
        for cy in 0..i32::from(h) {
            for cx in 0..i32::from(w) {
                let mut mask = 0u32;
                for ((dx, dy), bit) in STANDARD_BIT {
                    if dots.is_lit(cx * 2 + dx, cy * 4 + dy) {
                        mask |= u32::from(bit);
                    }
                }
                let got = cell(&c, cx, cy);
                if mask == 0 {
                    prop_assert_eq!(got, Cell::BLANK);
                } else {
                    prop_assert_eq!(got.ch as u32, 0x2800 + mask);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- CORE-R42

#[test]
fn core_r42_pixel_buffer_is_width_by_twice_height() {
    let mut px = Pixels::new(Size::new(4, 3));
    assert_eq!(px.pixel_size(), Size::new(4, 6));
    assert_eq!(px.get(3, 5), Some(Ink::BLANK));
    for (x, y) in [(-1, 0), (0, -1), (4, 0), (0, 6), (i32::MAX, i32::MIN)] {
        px.set(x, y, A); // no-op, no panic
        assert_eq!(px.get(x, y), None);
    }
    px.set(3, 5, A);
    assert_eq!(px.get(3, 5), Some(A));
    px.fill(B);
    assert_eq!(px.get(0, 0), Some(B));
    px.clear();
    assert_eq!(px.get(0, 0), Some(Ink::BLANK));
    px.set(1, 1, A);
    px.resize(Size::new(2, 1));
    assert_eq!(px.pixel_size(), Size::new(2, 2));
    assert_eq!(px.get(1, 1), Some(Ink::BLANK));
    assert_eq!(px.get(3, 5), None);
}

#[test]
fn core_r42_unicode_blit_uses_upper_and_lower_half_blocks() {
    // Cells: 0 = both empty, 1 = top only, 2 = bottom only, 3 = both.
    let mut px = Pixels::new(Size::new(4, 1));
    px.set(1, 0, A);
    px.set(2, 1, B);
    px.set(3, 0, A);
    px.set(3, 1, B);
    let mut c = Canvas::new(Size::new(4, 1), GlyphMode::Unicode);
    c.put(0, 0, X, C);
    px.blit(&mut c, |_, _, _, _| {
        panic!("the ASCII closure is only for ASCII mode")
    });
    assert_eq!(
        cell(&c, 0, 0),
        Cell {
            ch: 'x',
            ink: C,
            bg: Ink::BLANK
        },
        "untouched"
    );
    assert_eq!(
        cell(&c, 1, 0),
        Cell {
            ch: '▀',
            ink: A,
            bg: Ink::BLANK
        }
    );
    assert_eq!(
        cell(&c, 2, 0),
        Cell {
            ch: '▄',
            ink: B,
            bg: Ink::BLANK
        }
    );
    assert_eq!(
        cell(&c, 3, 0),
        Cell {
            ch: '▀',
            ink: A,
            bg: B
        }
    );
}

#[test]
fn core_r42_ascii_blit_delegates_every_cell_to_the_scene_fallback() {
    let mut px = Pixels::new(Size::new(3, 2));
    px.set(1, 0, A); // cell (1, 0) top
    px.set(2, 3, B); // cell (2, 1) bottom
    let mut c = Canvas::new(Size::new(3, 2), GlyphMode::Ascii);
    c.put(0, 1, X, C);
    let mut calls = Vec::new();
    px.blit(&mut c, |x, y, top, bottom| {
        calls.push((x, y, top, bottom));
        match (top != Ink::BLANK, bottom != Ink::BLANK) {
            (false, false) => None,
            (true, _) => Some(('^', top)),
            (false, true) => Some(('v', bottom)),
        }
    });
    assert_eq!(calls.len(), 6, "once per cell: {calls:?}");
    assert!(calls.contains(&(1, 0, A, Ink::BLANK)));
    assert!(calls.contains(&(2, 1, Ink::BLANK, B)));
    assert_eq!(c.to_lines(), vec![" ^ ".to_owned(), "x v".to_owned()]);
    assert_eq!(cell(&c, 1, 0).ink, A);
    assert_eq!(cell(&c, 0, 1).ink, C, "None leaves the cell untouched");
}

#[test]
fn core_r42_background_role_pixels_count_as_empty() {
    // A `Role::Background` fg is painted as "no colour" (RND-R3), so a half block drawn
    // with it would show up in the terminal's default colour: treat it as empty.
    let mut px = Pixels::new(Size::new(1, 1));
    px.set(0, 0, Ink::new(Role::Background, 77));
    px.set(0, 1, A);
    let mut c = Canvas::new(Size::new(1, 1), GlyphMode::Unicode);
    px.blit(&mut c, |_, _, _, _| None);
    assert_eq!(
        cell(&c, 0, 0),
        Cell {
            ch: '▄',
            ink: A,
            bg: Ink::BLANK
        }
    );
}

// ---------------------------------------------------------------- CORE-R84

#[test]
fn core_r84_cell_bg_is_the_more_intense_lit_pixel_of_the_cell() {
    let mut px = Pixels::new(Size::new(3, 1));
    // Cell 0: both lit, the bottom brighter; cell 1: top only; cell 2: both empty.
    px.set(0, 0, B);
    px.set(0, 1, A);
    px.set(1, 0, C);
    assert_eq!(px.cell_bg(0, 0), A);
    assert_eq!(px.cell_bg(1, 0), C);
    assert_eq!(px.cell_bg(2, 0), Ink::BLANK);
    // A tie goes to the top; a lone bottom pixel stands for its cell.
    px.set(2, 0, Ink::new(Role::Star, 90));
    px.set(2, 1, B);
    assert_eq!(px.cell_bg(2, 0), Ink::new(Role::Star, 90));
    px.set(1, 0, Ink::BLANK);
    px.set(1, 1, B);
    assert_eq!(px.cell_bg(1, 0), B);
    // Off the buffer: blank.
    assert_eq!(px.cell_bg(3, 0), Ink::BLANK);
    assert_eq!(px.cell_bg(0, 1), Ink::BLANK);
    assert_eq!(px.cell_bg(-1, 0), Ink::BLANK);
}

#[test]
fn core_r84_blit_over_puts_the_pixels_behind_lit_cells_only() {
    let mut dots = Dots::new(Size::new(3, 2));
    dots.set(0, 0, A);
    dots.set(5, 7, C);
    let mut under = Pixels::new(Size::new(3, 2));
    under.set(0, 1, B);
    under.set(1, 0, C);
    let mut plain = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
    dots.blit(&mut plain);
    let mut c = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
    c.put_bg(1, 0, X, A, B);
    dots.blit_over(&mut c, &under);
    for (x, y) in [(0, 0), (2, 1)] {
        let (got, want) = (cell(&c, x, y), cell(&plain, x, y));
        assert_eq!((got.ch, got.ink), (want.ch, want.ink), "same dots as blit");
        assert_eq!(got.bg, under.cell_bg(x, y));
    }
    assert_eq!(cell(&c, 0, 0).bg, B);
    assert_eq!(cell(&c, 2, 1).bg, Ink::BLANK);
    // Unlit cells keep what was drawn before, even over a lit pixel.
    assert_eq!(
        cell(&c, 1, 0),
        Cell {
            ch: 'x',
            ink: A,
            bg: B
        }
    );
    // ASCII mode has no half blocks: plain `blit`.
    let mut ascii = Canvas::new(Size::new(3, 2), GlyphMode::Ascii);
    let mut ascii_plain = Canvas::new(Size::new(3, 2), GlyphMode::Ascii);
    dots.blit_over(&mut ascii, &under);
    dots.blit(&mut ascii_plain);
    assert_eq!(ascii.cells(), ascii_plain.cells());
}

#[test]
fn core_r42_half_block_is_the_unicode_cell_rule_of_blit() {
    // Every (top, bottom) pair: `half_block` on one cell equals `blit` of that pair.
    for (top, bottom) in [
        (Ink::BLANK, Ink::BLANK),
        (A, Ink::BLANK),
        (Ink::BLANK, B),
        (A, B),
    ] {
        let mut px = Pixels::new(Size::new(1, 1));
        px.set(0, 0, top);
        px.set(0, 1, bottom);
        let mut blitted = Canvas::new(Size::new(1, 1), GlyphMode::Unicode);
        blitted.put(0, 0, X, C);
        let mut direct = blitted.clone();
        px.blit(&mut blitted, |_, _, _, _| None);
        raster::half_block(&mut direct, 0, 0, top, bottom);
        assert_eq!(
            cell(&direct, 0, 0),
            cell(&blitted, 0, 0),
            "{top:?} {bottom:?}"
        );
    }
}

// ---------------------------------------------------------------- CORE-R82

fn lit(d: &Dots) -> Vec<(i32, i32)> {
    let s = d.dot_size();
    let (w, h) = (i32::from(s.width()), i32::from(s.height()));
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| d.is_lit(x, y))
        .collect()
}

#[test]
fn core_r82_disc_lights_the_dots_within_its_radius() {
    let mut d = Dots::new(Size::new(10, 5));
    d.disc(9.3, 9.6, 2.0, A);
    // Every dot of the 20 × 20 lattice whose centre is within the radius.
    let want: Vec<_> = (0..20)
        .flat_map(|y| (0..20).map(move |x| (x, y)))
        .filter(|&(x, y)| (f64::from(x) - 9.3).hypot(f64::from(y) - 9.6) <= 2.0)
        .collect();
    assert_eq!(lit(&d), want);
    assert!(want.len() >= 12, "{}", want.len());
}

#[test]
fn core_r82_small_disc_is_the_nearest_dot_and_bad_input_lights_nothing() {
    let mut d = Dots::new(Size::new(4, 2));
    d.disc(2.4, 3.6, 0.2, A);
    assert_eq!(lit(&d), vec![(2, 4)]);
    let mut d = Dots::new(Size::new(4, 2));
    for (x, y, r) in [
        (f64::NAN, 1.0, 1.0),
        (1.0, f64::INFINITY, 1.0),
        (1.0, 1.0, -0.5),
        (1.0, 1.0, f64::NAN),
    ] {
        d.disc(x, y, r, A);
    }
    assert!(lit(&d).is_empty());
}

#[test]
fn core_r82_disc_is_clipped_to_the_lattice() {
    let mut d = Dots::new(Size::new(3, 2));
    d.disc(-1.0, -1.0, 2.0, A);
    assert_eq!(lit(&d), vec![(0, 0)]);
    // A huge disc fills the lattice and nothing more.
    let mut d = Dots::new(Size::new(3, 2));
    d.disc(2.0, 3.0, 1e12, A);
    assert_eq!(lit(&d).len(), 6 * 8);
}

// ---------------------------------------------------------------- CORE-R86

#[test]
fn core_r86_backdrop_puts_the_pixels_behind_glyphs_drawn_over_them() {
    let size = Size::new(4, 2);
    let mut px = Pixels::new(size);
    // Cell (0, 0): both pixels set; cell (1, 0): top only; cell (2, 0): none.
    px.set(0, 0, A);
    px.set(0, 1, B);
    px.set(1, 0, C);
    let mut c = Canvas::new(size, GlyphMode::Unicode);
    px.blit(&mut c, |_, _, _, _| None);
    for x in 0..3 {
        c.put(x, 0, Glyph::new('─', '-'), A);
    }
    // A label's blank over the pixels counts too.
    px.set(3, 1, B);
    c.put(3, 0, Glyph::ascii(' '), A);
    px.backdrop(&mut c);
    assert_eq!(cell(&c, 3, 0).bg, B, "a label gap takes the backdrop");
    assert_eq!(
        cell(&c, 0, 0).bg,
        A,
        "the cell's `cell_bg`, its more intense pixel"
    );
    assert_eq!(cell(&c, 1, 0).bg, C, "top pixel when the bottom is empty");
    assert_eq!(cell(&c, 2, 0).bg, Ink::BLANK, "no pixels, no backdrop");
    assert_eq!(
        (cell(&c, 0, 0).ch, cell(&c, 0, 0).ink),
        ('─', A),
        "glyph kept"
    );
    // ASCII canvases have no half blocks to match: untouched.
    let mut a = Canvas::new(size, GlyphMode::Ascii);
    a.put(0, 0, X, A);
    px.backdrop(&mut a);
    assert_eq!(cell(&a, 0, 0).bg, Ink::BLANK);
}

// ---------------------------------------------------------------- CORE-R87

#[test]
fn core_r87_stronger_keeps_the_more_intense_ink() {
    assert_eq!(raster::stronger(B, A), A);
    assert_eq!(raster::stronger(A, B), A);
    assert_eq!(raster::stronger(A, A), A, "the first on a tie");
    assert_eq!(raster::stronger(Ink::BLANK, B), B, "an empty ink loses");
    assert_eq!(raster::stronger(B, Ink::BLANK), B);
    assert_eq!(raster::stronger(Ink::BLANK, Ink::BLANK), Ink::BLANK);
    // A signed field's zero (128) is weaker than a deep negative lobe.
    let (zero, lobe) = (Ink::new(Role::Div, 128), Ink::new(Role::Div, 10));
    assert_eq!(raster::stronger(zero, lobe), lobe);
}

// ---------------------------------------------------------------- CORE-R91

#[test]
fn core_r91_ascii_ramp_cell_draws_the_stronger_pixel_on_the_ramp() {
    assert_eq!(raster::ascii_ramp_cell(Ink::BLANK, Ink::BLANK), None);
    // The stronger pixel's ink, top on a tie; an empty pixel loses.
    assert_eq!(raster::ascii_ramp_cell(A, B), Some(('%', A)));
    assert_eq!(raster::ascii_ramp_cell(B, A), Some(('%', A)));
    assert_eq!(raster::ascii_ramp_cell(A, C), Some(('%', A)));
    assert_eq!(raster::ascii_ramp_cell(Ink::BLANK, B), Some(('=', B)));
    // Level `l` maps to ramp entry `l · 9 / 256`: both ends and every step boundary.
    let ramp = raster::ASCII_RAMP;
    for level in 0..=255u8 {
        let ink = Ink::new(Role::Blackbody, level);
        let (ch, got) = raster::ascii_ramp_cell(ink, Ink::BLANK).expect("drawn");
        assert_eq!(got, ink);
        assert_eq!(ch, ramp[usize::from(level) * 9 / 256], "level {level}");
    }
    assert_eq!(
        raster::ascii_ramp_cell(Ink::new(Role::Star, 0), Ink::BLANK).map(|c| c.0),
        Some('.')
    );
    assert_eq!(
        raster::ascii_ramp_cell(Ink::new(Role::Star, 255), Ink::BLANK).map(|c| c.0),
        Some('@')
    );
}

// ---------------------------------------------------------------- CORE-R89

/// Every pixel of a `n × n`-pixel buffer filled with `ink` and dissolved at `floor`.
fn dissolved(ink: Ink, floor: u8, n: u16) -> Vec<Ink> {
    let mut p = Pixels::new(Size::new(n, n / 2));
    p.fill(ink);
    p.dissolve(floor);
    let n = i32::from(n);
    (0..n)
        .flat_map(|y| (0..n).map(move |x| (x, y)))
        .map(|(x, y)| p.get(x, y).expect("in range"))
        .collect()
}

/// The one ink a dissolved uniform field holds (CORE-R89: no pattern).
fn dissolved_uniform(ink: Ink, floor: u8) -> Ink {
    let px = dissolved(ink, floor, 16);
    assert!(
        px.iter().all(|&p| p == px[0]),
        "{ink:?} at {floor}: not uniform"
    );
    px[0]
}

#[test]
fn core_r89_dissolve_fades_the_fringe_smoothly_to_black() {
    let seq = |l: u8| Ink::new(Role::Seq, l);
    // At or above the floor: untouched; below half of it: gone; floor 0 is a no-op.
    assert_eq!(dissolved_uniform(seq(40), 40), seq(40));
    assert_eq!(dissolved_uniform(seq(200), 40), seq(200));
    assert_eq!(dissolved_uniform(seq(19), 40), Ink::BLANK);
    assert_eq!(dissolved_uniform(seq(0), 40), Ink::BLANK);
    assert_eq!(dissolved_uniform(seq(1), 0), seq(1));
    // In the band the level is `i·smoothstep`: half-way it is halved, and over every
    // level the output never falls as the input rises (a monotone fade, no cliff).
    assert_eq!(dissolved_uniform(seq(30), 40), seq(15));
    let out: Vec<u8> = (0..=255u8)
        .map(|l| dissolved_uniform(seq(l), 40))
        .map(|p| if p == Ink::BLANK { 0 } else { p.level })
        .collect();
    assert!(out.windows(2).all(|w| w[1] >= w[0]), "{out:?}");
    assert!((20..38).all(|l| out[l] < l as u8), "the band is dimmed");
    // Div fades its contrast from 128 on either side; Phase has no intensity.
    assert_eq!(
        dissolved_uniform(Ink::new(Role::Div, 158), 40),
        Ink::new(Role::Div, 143)
    );
    assert_eq!(
        dissolved_uniform(Ink::new(Role::Div, 98), 40),
        Ink::new(Role::Div, 113)
    );
    let phase = Ink::new(Role::Phase, 3);
    assert_eq!(dissolved_uniform(phase, 40), phase);
}

#[test]
fn core_r89_dissolve_has_no_pattern() {
    // No ordered or noise dither: a uniform field stays uniform at every band level, so
    // nothing repeats, and nothing salts the fringe.
    for l in 0..60u8 {
        dissolved_uniform(Ink::new(Role::Seq, l), 40);
        dissolved_uniform(Ink::new(Role::Ramp(Ramp::Aurora), l), 40);
    }
}

// ---------------------------------------------------------------- CORE-R105

/// A `w × h` field of distances from `(cx, cy)` (node units).
fn cone(w: usize, h: usize, cx: f64, cy: f64) -> Vec<f32> {
    (0..w * h)
        .map(|k| ((k % w) as f64 - cx).hypot((k / w) as f64 - cy) as f32)
        .collect()
}

fn lit_dots(d: &Dots) -> Vec<(i32, i32)> {
    let s = d.dot_size();
    let mut out = Vec::new();
    for y in 0..i32::from(s.height()) {
        for x in 0..i32::from(s.width()) {
            if d.is_lit(x, y) {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn core_r105_circle_contour_is_closed_and_on_its_radius() {
    // 32×16 cells = 64×64 dots; one node per dot.
    let mut d = Dots::new(Size::new(32, 16));
    let (w, h, c, r) = (64, 64, 31.5, 20.0);
    let field = cone(w, h, c, c);
    d.contour(&field, w, h, r as f32, |x, y| (x, y), A);
    let lit = lit_dots(&d);
    assert!(lit.len() > 100, "{} dots", lit.len());
    for &(x, y) in &lit {
        let rho = (f64::from(x) - c).hypot(f64::from(y) - c);
        assert!((rho - r).abs() <= 1.0, "dot {x},{y} at radius {rho}");
        // Closed: every dot continues the curve on both sides.
        let nb = (-1..=1)
            .flat_map(|a| (-1..=1).map(move |b| (a, b)))
            .filter(|&(a, b)| (a, b) != (0, 0) && d.is_lit(x + a, y + b))
            .count();
        assert!(nb >= 2, "open end at {x},{y}");
    }
    // Every direction meets the ring: a lit dot within one dot of the circle's point.
    for k in 0..360 {
        let (s, co) = f64::from(k).to_radians().sin_cos_det();
        let (px, py) = ((c + r * co).round() as i32, (c + r * s).round() as i32);
        let hit = (-1..=1).any(|a| (-1..=1).any(|b| d.is_lit(px + a, py + b)));
        assert!(hit, "gap at {k}°");
    }
}

#[test]
fn core_r105_to_dot_maps_nodes_and_contour_by_picks_ink_per_cell() {
    // Nodes every 2 dots: the circle of radius 6 nodes lands at radius 12 dots.
    let (w, h) = (20, 20);
    let field = cone(w, h, 9.5, 9.5);
    let mut d = Dots::new(Size::new(20, 10));
    d.contour_by(
        &field,
        w,
        h,
        6.0,
        |x, y| (2.0 * x, 2.0 * y),
        |i, _| (i >= 10).then_some(B),
    );
    let lit = lit_dots(&d);
    assert!(!lit.is_empty());
    for &(x, y) in &lit {
        let rho = (f64::from(x) - 19.0).hypot(f64::from(y) - 19.0);
        assert!((rho - 12.0).abs() <= 1.5, "{x},{y}: {rho}");
        // Only the right half's cells were drawn.
        assert!(x >= 20, "{x},{y}");
    }
    let mut c = Canvas::new(Size::new(20, 10), GlyphMode::Unicode);
    d.blit(&mut c);
    assert!(c.cells().iter().all(|cell| cell.ch == ' ' || cell.ink == B));
}

#[test]
fn core_r106_refined_cells_trace_the_bilinear_level_set() {
    // One cell 40 dots wide with corners 0, 1, 0, 1 (clockwise from top-left): the
    // bilinear surface v = a + b − 2ab has a curved (hyperbolic) level set; refined,
    // every dot lies on it.
    let corners = [0.0f32, 1.0, 0.0, 1.0];
    let level = 0.3f32;
    let wide = |x: f64, y: f64| (40.0 * x + 2.0, 40.0 * y + 2.0);
    let mut d = Dots::new(Size::new(24, 12));
    d.contour_cell_refined(corners, level, (0, 0), 8, wide, A);
    let lit = lit_dots(&d);
    assert!(lit.len() > 20, "{} dots", lit.len());
    // Bilinear value at a dot.
    let bil = |x: i32, y: i32| {
        let (a, b) = ((f64::from(x) - 2.0) / 40.0, (f64::from(y) - 2.0) / 40.0);
        let top = 0.0 + a;
        let bottom = 1.0 - a;
        top + (bottom - top) * b
    };
    for &(x, y) in &lit {
        // Within a dot of the level set: some neighbour straddles it.
        let around: Vec<f64> = (-1..=1)
            .flat_map(|p| (-1..=1).map(move |q| (p, q)))
            .map(|(p, q)| bil(x + p, y + q))
            .collect();
        let (lo, hi) = around
            .iter()
            .fold((f64::MAX, f64::MIN), |m, &v| (m.0.min(v), m.1.max(v)));
        assert!(
            lo <= 0.3 + 1e-9 && hi >= 0.3 - 1e-9,
            "dot {x},{y} off the curve"
        );
    }
    // Plain contour_cell draws straight chords instead: the refined curve differs.
    let mut plain = Dots::new(Size::new(24, 12));
    plain.contour_cell(corners, level, (0, 0), wide, A);
    assert_ne!(lit_dots(&plain), lit);
    // k ≤ 1 is exactly contour_cell.
    let mut one = Dots::new(Size::new(24, 12));
    one.contour_cell_refined(corners, level, (0, 0), 1, wide, A);
    assert_eq!(lit_dots(&one), lit_dots(&plain));
    // A level outside the corners draws nothing.
    let mut none = Dots::new(Size::new(24, 12));
    none.contour_cell_refined(corners, 2.0, (0, 0), 8, wide, A);
    assert!(lit_dots(&none).is_empty());
}

#[test]
fn core_r105_saddles_and_single_cells() {
    // A saddle: two diagonal corners above the level, two below: two pieces.
    let mut d = Dots::new(Size::new(8, 4));
    let wide = |x: f64, y: f64| (8.0 * x + 2.0, 8.0 * y + 2.0);
    d.contour_cell([1.0, 0.0, 1.0, 0.0], 0.5, (0, 0), wide, A);
    assert!(lit_dots(&d).len() >= 8, "{:?}", lit_dots(&d));
    // No crossing, no dots.
    let mut e = Dots::new(Size::new(8, 4));
    e.contour_cell([1.0, 2.0, 3.0, 4.0], 0.5, (0, 0), |x, y| (x, y), A);
    assert!(lit_dots(&e).is_empty());
    // A straight level in one cell: one piece through its middle.
    e.contour_cell(
        [0.0, 1.0, 1.0, 0.0],
        0.5,
        (0, 0),
        |x, y| (8.0 * x, 8.0 * y),
        A,
    );
    let lit = lit_dots(&e);
    assert!(lit.iter().all(|&(x, _)| x == 4), "{lit:?}");
    assert_eq!(lit.len(), 9);
}

#[test]
fn core_r105_degenerate_fields_draw_nothing() {
    let mut d = Dots::new(Size::new(8, 4));
    let field = cone(8, 8, 3.5, 3.5);
    let id = |x: f64, y: f64| (x, y);
    d.contour(&field, 8, 9, 2.0, id, A); // shorter than w·h
    d.contour(&field[..8], 8, 1, 2.0, id, A); // one row
    d.contour(&field, 1, 8, 2.0, id, A); // one column
    d.contour(&[f32::NAN; 16], 4, 4, 2.0, id, A);
    d.contour_cell([0.0, f32::INFINITY, 1.0, 0.0], 0.5, (0, 0), id, A);
    assert!(lit_dots(&d).is_empty());
}

// ---- CORE-R107 (M17, ADR-0020): fixed-threshold stipple ----

#[test]
fn core_r107_stipple_keep_is_deterministic_and_bounded() {
    for (x, y) in [(0, 0), (3, -7), (1000, 20)] {
        assert_eq!(
            raster::stipple_keep(9, x, y, 0.4),
            raster::stipple_keep(9, x, y, 0.4)
        );
        assert!(!raster::stipple_keep(9, x, y, 0.0), "p = 0 keeps nothing");
        assert!(!raster::stipple_keep(9, x, y, -1.0), "p < 0 keeps nothing");
        assert!(
            !raster::stipple_keep(9, x, y, f64::NAN),
            "NaN keeps nothing"
        );
        assert!(raster::stipple_keep(9, x, y, 1.0), "p = 1 keeps every dot");
    }
}

fn kept_share(f: impl Fn(i32, i32) -> bool) -> f64 {
    let kept = (0..200)
        .flat_map(|x| (0..200).map(move |y| (x, y)))
        .filter(|&(x, y)| f(x, y))
        .count();
    kept as f64 / 40_000.0
}

#[test]
fn core_r107_stipple_keep_rate_matches_the_density() {
    for p in [0.05, 0.25, 0.5, 0.9] {
        let rate = kept_share(|x, y| raster::stipple_keep(42, x, y, p));
        assert!((rate - p).abs() < 0.01, "p {p}: kept {rate}");
    }
}

#[test]
fn core_r107_salts_decorrelate_layers() {
    let rate =
        kept_share(|x, y| raster::stipple_keep(1, x, y, 0.5) && raster::stipple_keep(2, x, y, 0.5));
    assert!(
        (rate - 0.25).abs() < 0.01,
        "independent salts overlap at p² = 0.25, got {rate}"
    );
}

proptest! {
    /// No shimmer: raising the density only adds dots, so a cloud whose density changes
    /// smoothly changes only where it crosses a dot's fixed threshold.
    #[test]
    fn core_r107_stipple_is_monotone_in_density(
        salt in any::<u64>(), x in -500i32..500, y in -500i32..500,
        p in 0.0f64..1.0, dp in 0.0f64..1.0,
    ) {
        if raster::stipple_keep(salt, x, y, p) {
            prop_assert!(raster::stipple_keep(salt, x, y, p + dp));
        }
    }
}

#[test]
fn core_r107_dots_stipple_lights_exactly_the_kept_dots() {
    let mut dots = Dots::new(Size::new(10, 5));
    let ink = Ink::new(Role::Hot, 200);
    for x in 0..20 {
        for y in 0..20 {
            dots.stipple(x, y, 0.3, 7, ink);
        }
    }
    for x in 0..20 {
        for y in 0..20 {
            assert_eq!(
                dots.is_lit(x, y),
                raster::stipple_keep(7, x, y, 0.3),
                "dot {x},{y}"
            );
        }
    }
    dots.stipple(-1, 3, 1.0, 7, ink); // off-lattice: ignored, no panic
}

fn puff_dots(cx: f64, cy: f64, sigma: f64, peak: f64) -> Dots {
    let mut dots = Dots::new(Size::new(20, 10));
    dots.stipple_puff(cx, cy, sigma, peak, 11, Ink::new(Role::Sun, 255));
    dots
}

fn lit_count(dots: &Dots) -> usize {
    (0..40)
        .flat_map(|x| (0..40).map(move |y| (x, y)))
        .filter(|&(x, y)| dots.is_lit(x, y))
        .count()
}

#[test]
fn core_r108_stipple_puff_lights_the_kept_dots_within_three_sigma() {
    let (cx, cy, sigma, peak) = (20.3, 19.6, 2.0, 0.9);
    let dots = puff_dots(cx, cy, sigma, peak);
    for x in 0..40 {
        for y in 0..40 {
            let r2 = (f64::from(x) - cx).powi(2) + (f64::from(y) - cy).powi(2);
            let p = peak * (-r2 / (2.0 * sigma * sigma)).exp();
            let want = (x, y) == (20, 20)
                || (r2 <= (3.0 * sigma).powi(2) && raster::stipple_keep(11, x, y, p));
            assert_eq!(dots.is_lit(x, y), want, "dot {x},{y}");
        }
    }
    assert!(lit_count(&dots) >= 8, "a σ = 2 atom is a cloud, not a dot");
}

#[test]
fn core_r108_stipple_puff_never_vanishes_and_rejects_bad_input() {
    // A tiny or degenerate atom is still its centre dot.
    for sigma in [1e-6, 0.0, -1.0, f64::NAN] {
        let dots = puff_dots(5.4, 6.6, sigma, 0.9);
        assert_eq!(lit_count(&dots), 1, "sigma {sigma}");
        assert!(dots.is_lit(5, 7));
    }
    for (cx, peak) in [(f64::NAN, 0.9), (5.0, 0.0), (5.0, -1.0)] {
        assert_eq!(
            lit_count(&puff_dots(cx, 5.0, 2.0, peak)),
            0,
            "cx {cx} peak {peak}"
        );
    }
    // Off-lattice and huge atoms are clipped, not a panic or a hang.
    let _ = puff_dots(-50.0, 900.0, 3.0, 1.0);
    let _ = puff_dots(10.0, 10.0, 1e9, 1.0);
}

proptest! {
    /// A swelling atom only gains dots: monotone in σ and peak.
    #[test]
    fn core_r108_stipple_puff_is_monotone(
        cx in 0.0f64..40.0, cy in 0.0f64..40.0,
        sigma in 0.3f64..4.0, ds in 0.0f64..2.0, peak in 0.05f64..1.0, dp in 0.0f64..0.5,
    ) {
        let small = puff_dots(cx, cy, sigma, peak);
        let big = puff_dots(cx, cy, sigma + ds, (peak + dp).min(1.0));
        for x in 0..40 {
            for y in 0..40 {
                if small.is_lit(x, y) {
                    prop_assert!(big.is_lit(x, y), "dot {},{} lost", x, y);
                }
            }
        }
    }
}
