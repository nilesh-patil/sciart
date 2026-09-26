//! SPEC-001 CORE-R33..R36: grid helpers.

use proptest::prelude::*;
use sciart_core::prelude::*;

/// A grid whose value at `(x, y)` is `y * 100 + x`, so moved cells are identifiable.
fn labelled(w: u16, h: u16) -> Grid<i32> {
    let mut g = Grid::new(Size::new(w, h), 0);
    for y in 0..i32::from(h) {
        for x in 0..i32::from(w) {
            assert!(g.set(x, y, y * 100 + x));
        }
    }
    g
}

fn hood(g: &Grid<i32>, x: i32, y: i32, hood: Hood, edges: Edges) -> Vec<(i32, i32)> {
    let mut v: Vec<(i32, i32)> = g.neighbors(x, y, hood, edges).collect();
    v.sort_unstable();
    v
}

fn sorted(mut v: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    v.sort_unstable();
    v
}

// ---------------------------------------------------------------- CORE-R33

#[test]
fn core_r33_new_reports_dimensions_and_fill() {
    let g = Grid::new(Size::new(3, 2), 7u8);
    assert_eq!(g.size(), Size::new(3, 2));
    assert_eq!((g.width(), g.height()), (3, 2));
    assert_eq!(g.len(), 6);
    assert!(!g.is_empty());
    assert_eq!(g.as_slice(), &[7; 6]);
}

#[test]
fn core_r33_get_set_in_grid_round_trip() {
    let mut g = Grid::new(Size::new(4, 3), 0);
    assert!(g.set(3, 2, 5));
    assert_eq!(g.get(3, 2), Some(&5));
    *g.get_mut(1, 1).unwrap() = 9;
    assert_eq!(g.get(1, 1), Some(&9));
    // Row-major: (x, y) lives at y * width + x.
    assert_eq!(g.as_slice()[2 * 4 + 3], 5);
    assert_eq!(g.as_slice()[4 + 1], 9);
}

#[test]
fn core_r33_off_grid_access_never_panics_for_extreme_i32() {
    let mut g = Grid::new(Size::new(4, 3), 1);
    let probes = [
        (-1, 0),
        (0, -1),
        (4, 0),
        (0, 3),
        (i32::MIN, 0),
        (0, i32::MIN),
        (i32::MAX, 0),
        (0, i32::MAX),
        (i32::MIN, i32::MIN),
        (i32::MAX, i32::MAX),
        (i32::MAX, i32::MIN),
    ];
    for (x, y) in probes {
        assert_eq!(g.get(x, y), None, "get({x},{y})");
        assert!(g.get_mut(x, y).is_none(), "get_mut({x},{y})");
        assert!(!g.set(x, y, 9), "set({x},{y})");
    }
    assert!(g.as_slice().iter().all(|&v| v == 1), "off-grid set wrote");
}

#[test]
fn core_r33_max_width_far_corner() {
    // Index math is done in usize (a u16 x u16 area can exceed i32::MAX); a full
    // 65535 x 65535 grid is too big for a unit test, so exercise the widest row.
    let mut g = Grid::new(Size::new(u16::MAX, 2), 0u8);
    assert!(g.set(i32::from(u16::MAX) - 1, 1, 3));
    assert_eq!(g.as_slice()[g.len() - 1], 3);
    assert_eq!(g.get(i32::from(u16::MAX), 1), None);
}

#[test]
fn core_r33_zero_by_zero_grid_is_valid_and_empty() {
    let mut g = Grid::<u8>::default();
    assert_eq!((g.width(), g.height()), (0, 0));
    assert!(g.is_empty());
    assert_eq!(g.len(), 0);
    assert!(g.as_slice().is_empty());
    assert_eq!(g.iter().count(), 0);
    assert_eq!(g.get(0, 0), None);
    assert!(!g.set(0, 0, 1));
    g.fill(3);
    assert_eq!(g.neighbors(0, 0, Hood::Moore, Edges::Wrap).count(), 0);
}

#[test]
fn core_r33_one_by_n_grid() {
    let mut g = Grid::new(Size::new(1, 5), 0);
    for y in 0..5 {
        assert!(g.set(0, y, y));
    }
    assert!(!g.set(1, 0, 9));
    assert_eq!(g.as_slice(), &[0, 1, 2, 3, 4]);
    let coords: Vec<_> = g.iter().map(|(c, _)| c).collect();
    assert_eq!(coords, vec![(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);
}

#[test]
fn core_r33_iter_is_row_major_with_coordinates() {
    let g = labelled(3, 2);
    let got: Vec<_> = g.iter().map(|((x, y), &v)| (x, y, v)).collect();
    assert_eq!(
        got,
        vec![
            (0, 0, 0),
            (1, 0, 1),
            (2, 0, 2),
            (0, 1, 100),
            (1, 1, 101),
            (2, 1, 102)
        ]
    );
    assert_eq!(g.iter().len(), 6);
}

#[test]
fn core_r33_fill_overwrites_every_cell() {
    let mut g = labelled(4, 3);
    g.fill(-1);
    assert!(g.as_slice().iter().all(|&v| v == -1));
}

// ---------------------------------------------------------------- CORE-R34

/// Expected result of resizing `labelled(ow, oh)` to `nw × nh` with fill `-1`.
fn expected_resize(ow: u16, oh: u16, nw: u16, nh: u16) -> Vec<i32> {
    let mut v = Vec::new();
    for y in 0..i32::from(nh) {
        for x in 0..i32::from(nw) {
            let kept = x < i32::from(ow) && y < i32::from(oh);
            v.push(if kept { y * 100 + x } else { -1 });
        }
    }
    v
}

#[test]
fn core_r34_resize_keeps_overlap_in_every_direction() {
    let shapes = [
        (4, 3, 6, 5), // grow both
        (6, 5, 4, 3), // shrink both
        (4, 3, 7, 3), // grow width only
        (7, 3, 4, 3), // shrink width only
        (4, 3, 4, 6), // grow height only
        (4, 6, 4, 3), // shrink height only
        (5, 3, 2, 6), // narrower and taller
        (2, 6, 5, 3), // wider and shorter
        (1, 1, 3, 3),
        (3, 3, 1, 1),
        (4, 3, 4, 3), // same size is a no-op
    ];
    for (ow, oh, nw, nh) in shapes {
        let mut g = labelled(ow, oh);
        g.resize(Size::new(nw, nh), -1);
        assert_eq!(g.size(), Size::new(nw, nh), "{ow}x{oh} -> {nw}x{nh}");
        assert_eq!(
            g.as_slice(),
            expected_resize(ow, oh, nw, nh).as_slice(),
            "{ow}x{oh} -> {nw}x{nh}"
        );
    }
}

#[test]
fn core_r34_resize_from_empty_fills_everything() {
    let mut g = Grid::<i32>::default();
    g.resize(Size::new(3, 2), 4);
    assert_eq!(g.size(), Size::new(3, 2));
    assert_eq!(g.as_slice(), &[4; 6]);
}

#[test]
fn core_r34_shrink_then_grow_reuses_the_allocation() {
    let mut g = labelled(8, 8);
    let ptr = g.as_slice().as_ptr();
    g.resize(Size::new(3, 2), -1);
    g.resize(Size::new(8, 8), -1);
    assert_eq!(g.as_slice().as_ptr(), ptr, "buffer was reallocated");
    assert_eq!(g.as_slice(), expected_resize(3, 2, 8, 8).as_slice());
}

#[test]
fn core_r34_resize_works_for_non_copy_values() {
    let mut g = Grid::new(Size::new(2, 2), String::from("a"));
    g.set(1, 1, "z".into());
    g.resize(Size::new(3, 1), "n".into());
    assert_eq!(g.as_slice(), &["a", "a", "n"]);
    g.resize(Size::new(2, 2), "m".into());
    assert_eq!(g.as_slice(), &["a", "a", "m", "m"]);
}

// ---------------------------------------------------------------- CORE-R35

#[test]
fn core_r35_interior_cell_has_full_hoods() {
    let g = labelled(5, 5);
    for edges in [Edges::Clip, Edges::Wrap] {
        assert_eq!(
            hood(&g, 2, 2, Hood::VonNeumann, edges),
            sorted(vec![(2, 1), (1, 2), (3, 2), (2, 3)])
        );
        assert_eq!(
            hood(&g, 2, 2, Hood::Moore, edges),
            sorted(vec![
                (1, 1),
                (2, 1),
                (3, 1),
                (1, 2),
                (3, 2),
                (1, 3),
                (2, 3),
                (3, 3)
            ])
        );
    }
}

#[test]
fn core_r35_clip_drops_off_grid_at_corners_and_edges() {
    let g = labelled(4, 3);
    // Top-left corner.
    assert_eq!(
        hood(&g, 0, 0, Hood::VonNeumann, Edges::Clip),
        sorted(vec![(1, 0), (0, 1)])
    );
    assert_eq!(
        hood(&g, 0, 0, Hood::Moore, Edges::Clip),
        sorted(vec![(1, 0), (0, 1), (1, 1)])
    );
    // Bottom-right corner.
    assert_eq!(
        hood(&g, 3, 2, Hood::VonNeumann, Edges::Clip),
        sorted(vec![(3, 1), (2, 2)])
    );
    assert_eq!(
        hood(&g, 3, 2, Hood::Moore, Edges::Clip),
        sorted(vec![(2, 1), (3, 1), (2, 2)])
    );
    // Top edge, not a corner.
    assert_eq!(
        hood(&g, 1, 0, Hood::VonNeumann, Edges::Clip),
        sorted(vec![(0, 0), (2, 0), (1, 1)])
    );
    assert_eq!(
        hood(&g, 1, 0, Hood::Moore, Edges::Clip),
        sorted(vec![(0, 0), (2, 0), (0, 1), (1, 1), (2, 1)])
    );
    // Right edge, not a corner.
    assert_eq!(
        hood(&g, 3, 1, Hood::Moore, Edges::Clip),
        sorted(vec![(2, 0), (3, 0), (2, 1), (2, 2), (3, 2)])
    );
}

#[test]
fn core_r35_wrap_is_toroidal_at_corners() {
    let g = labelled(4, 3);
    assert_eq!(
        hood(&g, 0, 0, Hood::VonNeumann, Edges::Wrap),
        sorted(vec![(0, 2), (3, 0), (1, 0), (0, 1)])
    );
    assert_eq!(
        hood(&g, 0, 0, Hood::Moore, Edges::Wrap),
        sorted(vec![
            (3, 2),
            (0, 2),
            (1, 2),
            (3, 0),
            (1, 0),
            (3, 1),
            (0, 1),
            (1, 1)
        ])
    );
    assert_eq!(
        hood(&g, 3, 2, Hood::VonNeumann, Edges::Wrap),
        sorted(vec![(3, 1), (2, 2), (0, 2), (3, 0)])
    );
}

#[test]
fn core_r35_wrap_on_1x1_has_no_neighbours() {
    let g = labelled(1, 1);
    for h in [Hood::VonNeumann, Hood::Moore] {
        for e in [Edges::Clip, Edges::Wrap] {
            assert_eq!(hood(&g, 0, 0, h, e), vec![], "{h:?} {e:?}");
        }
    }
}

#[test]
fn core_r35_wrap_on_1xn_skips_self_and_dedups() {
    // 1 wide, 4 tall: horizontal wraps land on the cell itself (skipped), and
    // Moore's diagonals collapse onto the vertical neighbours (yielded once).
    let g = labelled(1, 4);
    assert_eq!(
        hood(&g, 0, 0, Hood::VonNeumann, Edges::Wrap),
        vec![(0, 1), (0, 3)]
    );
    assert_eq!(
        hood(&g, 0, 0, Hood::Moore, Edges::Wrap),
        vec![(0, 1), (0, 3)]
    );
    assert_eq!(hood(&g, 0, 0, Hood::Moore, Edges::Clip), vec![(0, 1)]);
    // N x 1 is the transpose.
    let g = labelled(4, 1);
    assert_eq!(
        hood(&g, 2, 0, Hood::Moore, Edges::Wrap),
        vec![(1, 0), (3, 0)]
    );
    // 1 x 2: up and down wrap to the same cell.
    let g = labelled(1, 2);
    assert_eq!(hood(&g, 0, 0, Hood::Moore, Edges::Wrap), vec![(0, 1)]);
}

#[test]
fn core_r35_wrap_on_2x2_dedups_to_the_other_three() {
    let g = labelled(2, 2);
    assert_eq!(
        hood(&g, 0, 0, Hood::VonNeumann, Edges::Wrap),
        vec![(0, 1), (1, 0)]
    );
    assert_eq!(
        hood(&g, 0, 0, Hood::Moore, Edges::Wrap),
        vec![(0, 1), (1, 0), (1, 1)]
    );
    // Raw (unsorted) output must have no duplicates either.
    let raw: Vec<_> = g.neighbors(1, 1, Hood::Moore, Edges::Wrap).collect();
    assert_eq!(raw.len(), 3);
}

#[test]
fn core_r35_off_grid_centre_yields_nothing() {
    let g = labelled(3, 3);
    for (x, y) in [(-1, 0), (3, 1), (i32::MIN, i32::MAX), (i32::MAX, 0)] {
        for e in [Edges::Clip, Edges::Wrap] {
            assert_eq!(g.neighbors(x, y, Hood::Moore, e).count(), 0);
        }
    }
}

proptest! {
    #[test]
    fn core_r35_neighbours_in_grid_unique_self_excluded(
        w in 1u16..7,
        h in 1u16..7,
        x in 0i32..7,
        y in 0i32..7,
        moore in any::<bool>(),
        wrap in any::<bool>(),
    ) {
        let g = Grid::new(Size::new(w, h), 0u8);
        let (x, y) = (x % i32::from(w), y % i32::from(h));
        let hd = if moore { Hood::Moore } else { Hood::VonNeumann };
        let ed = if wrap { Edges::Wrap } else { Edges::Clip };
        let got: Vec<_> = g.neighbors(x, y, hd, ed).collect();
        let max = if moore { 8 } else { 4 };
        prop_assert!(got.len() <= max);
        let mut uniq = got.clone();
        uniq.sort_unstable();
        uniq.dedup();
        prop_assert_eq!(uniq.len(), got.len(), "duplicates in {:?}", got);
        for &(nx, ny) in &got {
            prop_assert!(g.get(nx, ny).is_some(), "({}, {}) off-grid", nx, ny);
            prop_assert!((nx, ny) != (x, y), "self yielded");
        }
        // On a grid at least 3 in both axes, Wrap always yields the full hood.
        if wrap && w >= 3 && h >= 3 {
            prop_assert_eq!(got.len(), max);
        }
    }
}

// ---------------------------------------------------------------- CORE-R36

#[test]
fn core_r36_step_is_synchronous() {
    // "Copy the left neighbour (0 at the edge)". Sequential in-place update would
    // smear the first value across the whole row; a synchronous step shifts it by one.
    let mut g = Grid::new(Size::new(5, 1), 0);
    for x in 0..5 {
        g.set(x, 0, x + 1);
    }
    let mut scratch = Grid::default();
    g.step(&mut scratch, |old, x, y| {
        old.get(x - 1, y).copied().unwrap_or(0)
    });
    assert_eq!(g.as_slice(), &[0, 1, 2, 3, 4]);
    g.step(&mut scratch, |old, x, y| {
        old.get(x - 1, y).copied().unwrap_or(0)
    });
    assert_eq!(g.as_slice(), &[0, 0, 1, 2, 3]);
}

#[test]
fn core_r36_rule_sees_coordinates_row_major_and_whole_old_grid() {
    let mut g = labelled(3, 2);
    let mut scratch = Grid::default();
    let mut visits = Vec::new();
    g.step(&mut scratch, |old, x, y| {
        visits.push((x, y));
        // Sum of the whole old grid is constant during the step.
        let total: i32 = old.as_slice().iter().sum();
        assert_eq!(total, 1 + 2 + 100 + 101 + 102);
        old.get(x, y).copied().unwrap_or_default() * 2
    });
    assert_eq!(visits, vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]);
    assert_eq!(g.as_slice(), &[0, 2, 4, 200, 202, 204]);
    assert_eq!(g.size(), Size::new(3, 2));
}

#[test]
fn core_r36_scratch_is_resized_to_match() {
    let mut g = labelled(4, 3);
    // Wrong-sized scratch, both larger and smaller.
    for s in [Size::new(9, 9), Size::new(1, 1)] {
        let mut scratch = Grid::new(s, 77);
        g.step(&mut scratch, |old, x, y| *old.get(x, y).unwrap());
        assert_eq!(g.size(), Size::new(4, 3));
        assert_eq!(g.as_slice(), labelled(4, 3).as_slice());
        // After the swap, scratch holds the previous generation at the grid's size.
        assert_eq!((scratch.width(), scratch.height()), (4, 3));
    }
}

#[test]
fn core_r36_steady_state_steps_do_not_reallocate() {
    let mut g = labelled(6, 4);
    let mut scratch = Grid::default();
    g.step(&mut scratch, |old, x, y| {
        old.get(x, y).copied().unwrap_or(0) + 1
    });
    // After one step both buffers exist; from now on they just ping-pong.
    let a = g.as_slice().as_ptr();
    let b = scratch.as_slice().as_ptr();
    for _ in 0..4 {
        g.step(&mut scratch, |old, x, y| {
            old.get(x, y).copied().unwrap_or(0) + 1
        });
        let now = g.as_slice().as_ptr();
        assert!(now == a || now == b, "step allocated a new buffer");
    }
    assert!(
        g.as_slice()
            .iter()
            .zip(labelled(6, 4).as_slice())
            .all(|(n, o)| *n == o + 5)
    );
}

#[test]
fn core_r36_step_on_empty_grid_is_a_no_op() {
    let mut g = Grid::<u8>::default();
    let mut scratch = Grid::new(Size::new(2, 2), 1);
    g.step(&mut scratch, |_, _, _| unreachable!("no cells"));
    assert!(g.is_empty());
}
