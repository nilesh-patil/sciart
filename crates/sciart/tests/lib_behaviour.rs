//! LIB-R5: what a library user can rely on from `Stage`.

use sciart::{ErrorKind, Options, Stage, Theme};

fn stage(scene: &str, w: u16, h: u16) -> Stage {
    Stage::new(scene, w, h, Options::default()).expect("a registered scene")
}

#[test]
fn lib_r5_default_options_are_seed_42_unicode_and_title_on() {
    // Seed 42 with the title on is what `sciart record` and the snapshot suite draw.
    let mut default = stage("galaxy_collision", 60, 20);
    let mut explicit = Stage::new(
        "galaxy_collision",
        60,
        20,
        Options::default().seed(42).ascii(false).title(true),
    )
    .unwrap();
    default.advance(3.0).unwrap();
    explicit.advance(3.0).unwrap();
    assert_eq!(
        default.render_text().unwrap(),
        explicit.render_text().unwrap()
    );

    let mut other = Stage::new("galaxy_collision", 60, 20, Options::default().seed(43)).unwrap();
    other.advance(3.0).unwrap();
    assert_ne!(default.render_text().unwrap(), other.render_text().unwrap());

    assert!(
        !default.render_text().unwrap().is_ascii(),
        "Unicode glyphs by default"
    );
    let mut ascii = Stage::new("galaxy_collision", 60, 20, Options::default().ascii(true)).unwrap();
    ascii.advance(3.0).unwrap();
    assert!(ascii.render_text().unwrap().is_ascii());

    let title = default.scene().title();
    let mechanism = title.split(" · ").next().unwrap();
    assert!(
        default.render_text().unwrap().contains(mechanism),
        "the title is drawn"
    );
    let mut untitled =
        Stage::new("galaxy_collision", 60, 20, Options::default().title(false)).unwrap();
    untitled.advance(3.0).unwrap();
    assert!(!untitled.render_text().unwrap().contains(mechanism));
}

#[test]
fn lib_r5_sizes_are_clamped_to_at_least_one_cell() {
    let mut s = stage("starfield", 0, 0);
    assert_eq!(s.size(), (1, 1));
    assert_eq!(
        s.render_text().unwrap().chars().count(),
        2,
        "one glyph and a newline"
    );
    s.resize(0, 7).unwrap();
    assert_eq!(s.size(), (1, 7));
    s.resize(33, 9).unwrap();
    assert_eq!(s.size(), (33, 9));
    assert_eq!(s.render_text().unwrap().lines().count(), 9);
}

#[test]
fn lib_r5_advance_takes_whole_steps_of_the_total() {
    let mut once = stage("double_pendulum", 50, 16);
    let mut sixty = stage("double_pendulum", 50, 16);
    once.advance(1.0).unwrap();
    for _ in 0..60 {
        sixty.advance(1.0 / 60.0).unwrap();
    }
    assert_eq!(once.time(), 1.0);
    assert_eq!(sixty.time(), 1.0);
    assert_eq!(
        once.render_ansi(Theme::Astro).unwrap(),
        sixty.render_ansi(Theme::Astro).unwrap()
    );

    // A part of a step carries over to the next call.
    let mut halves = stage("double_pendulum", 50, 16);
    halves.advance(0.5 / 60.0).unwrap();
    assert_eq!(halves.time(), 0.0);
    halves.advance(0.5 / 60.0).unwrap();
    assert_eq!(halves.time(), 1.0 / 60.0);

    halves.advance(0.0).unwrap();
    assert_eq!(halves.time(), 1.0 / 60.0, "zero seconds take no step");
}

#[test]
fn lib_r5_the_same_calls_give_the_same_bytes() {
    let run = |seed: u64| {
        let mut s = Stage::new("galaxy_collision", 70, 22, Options::default().seed(seed)).unwrap();
        s.advance(4.0).unwrap();
        s.resize(48, 14).unwrap();
        s.advance(2.5).unwrap();
        let text = s.render_text().unwrap();
        let ansi = s.render_ansi(Theme::Aurora).unwrap();
        let mut png = Vec::new();
        s.render_png(Theme::Aurora, &mut png).unwrap();
        (text, ansi, png)
    };
    let a = run(5);
    assert_eq!(a, run(5));
    assert_ne!(a.0, run(6).0);
}

#[test]
fn lib_r5_drawing_does_not_move_time() {
    let mut s = stage("aurora", 40, 12);
    s.advance(2.0).unwrap();
    let first = s.render_ansi(Theme::Astro).unwrap();
    let _ = s.render_text().unwrap();
    let mut png = Vec::new();
    s.render_png(Theme::Mono, &mut png).unwrap();
    assert_eq!(s.render_ansi(Theme::Astro).unwrap(), first);
    assert_eq!(s.time(), 2.0);
}

#[test]
fn lib_r5_ansi_is_line_framed_with_a_reset_per_row() {
    let mut s = stage("solar_system", 44, 13);
    s.advance(1.0).unwrap();
    let ansi = s.render_ansi(Theme::Astro).unwrap();
    let rows: Vec<_> = ansi.split_inclusive('\n').collect();
    assert_eq!(rows.len(), 13);
    assert!(rows.iter().all(|r| r.ends_with("\x1b[0m\n")));
    let text = s.render_text().unwrap();
    let plain: String = ansi
        .split('\x1b')
        .map(|s| s.split_once('m').map_or(s, |(_, rest)| rest))
        .collect();
    assert_eq!(plain, text, "the same glyphs as the text frame");
}

#[test]
fn lib_r5_png_is_8_by_16_pixels_per_cell() {
    let mut s = stage("pulsar", 12, 5);
    let mut png = Vec::new();
    s.render_png(Theme::Astro, &mut png).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((width, height), (12 * 8, 5 * 16));
}

#[test]
fn lib_r5_unknown_scenes_are_errors() {
    let err = Stage::new("no_such_scene", 10, 10, Options::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnknownScene);
    assert_eq!(err.scene(), Some("no_such_scene"));
    let renamed = Stage::new("crystal_growth", 10, 10, Options::default()).unwrap();
    assert_eq!(renamed.scene().name(), "fractal_growth");
}

#[test]
fn lib_r5_sizes_are_clamped_to_at_most_1024_cells() {
    // One axis at a time, on thin stages: a full 1024 × 1024 stage is a million cells per
    // buffer, which only slows the suite down (the clamp itself is a unit test in stage.rs).
    let mut wide = stage("starfield", u16::MAX, 3);
    assert_eq!(wide.size(), (1024, 3));
    wide.resize(1025, 2).unwrap();
    assert_eq!(wide.size(), (1024, 2));
    let mut tall = stage("starfield", 2, u16::MAX);
    assert_eq!(tall.size(), (2, 1024));
    tall.resize(3, 2000).unwrap();
    assert_eq!(tall.size(), (3, 1024));
    assert_eq!(tall.render_text().unwrap().lines().count(), 1024);
}

#[test]
fn lib_r5_bad_advances_are_invalid_input_and_step_nothing() {
    let mut s = stage("double_pendulum", 30, 10);
    s.advance(1.0).unwrap();
    let before = s.render_text().unwrap();
    for bad in [
        -1.0,
        -0.0001,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        3600.5,
        f64::MAX,
    ] {
        let err = s.advance(bad).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad}");
        assert_eq!(s.time(), 1.0, "{bad} stepped nothing");
        assert!(!s.is_failed(), "{bad} does not fail the stage");
    }
    assert_eq!(s.render_text().unwrap(), before);
    s.advance(0.0).unwrap();
    assert_eq!(s.time(), 1.0);
}
