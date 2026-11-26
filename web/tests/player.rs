//! WEB-R3: the browser player draws exactly what the capture renderer draws, steps time by
//! the rule of LIB-R5, and lists the scenes as `sciart list --json` does (CLI-R16).

use std::path::{Path, PathBuf};
use std::process::Command;

use sciart_capture::{RenderOptions, render_frame};
use sciart_core::prelude::*;
use sciart_render::Theme;
use sciart_web::{Player, scenes, themes};

/// The capture renderer's RGB pixels for `scene` after exactly `steps` fixed steps.
fn reference(scene: &str, size: Size, seed: u64, steps: u64, theme: Theme) -> Vec<u8> {
    let entry = sciart_generators::find(scene).expect("a registered scene");
    let mut stage = Stage::new((entry.build)(), seed, size, GlyphMode::Unicode);
    for _ in 0..steps {
        stage.step(SIM_DT);
    }
    let options = RenderOptions {
        theme,
        ascii: false,
    };
    render_frame(&mut stage, &options).rgb().to_vec()
}

/// RGBA → RGB, checking that every pixel is opaque.
fn rgb(rgba: &[u8]) -> Vec<u8> {
    assert!(
        rgba.chunks_exact(4).all(|px| px[3] == 255),
        "every pixel is opaque"
    );
    rgba.chunks_exact(4)
        .flat_map(|px| [px[0], px[1], px[2]])
        .collect()
}

#[test]
fn web_r3_player_frame_equals_the_capture_renderers_pixels_for_every_scene() {
    let size = Size::new(40, 12);
    for entry in sciart_generators::registry() {
        let name = entry.meta.name;
        let mut player = Player::new(name, 40, 12, 7).expect("a registered scene");
        assert_eq!(
            (player.width(), player.height()),
            (40 * 8, 12 * 16),
            "{name}"
        );
        // t = 0, then two uneven advances: 0.5 s and 1.25 s are 30 and 105 steps in all.
        assert_eq!(
            rgb(player.frame(Theme::Astro)),
            reference(name, size, 7, 0, Theme::Astro),
            "{name} t=0"
        );
        assert!(player.advance(0.5));
        assert!(player.advance(0.75));
        for theme in [Theme::Astro, Theme::Rainbow] {
            assert_eq!(
                rgb(player.frame(theme)),
                reference(name, size, 7, 75, theme),
                "{name} t=1.25 {theme:?}"
            );
        }
    }
}

#[test]
fn web_r3_player_follows_the_lib_r5_step_rule() {
    // advance(1.0) equals sixty advances of 1/60 s; both are sixty steps.
    let mut one = Player::new("double_pendulum", 30, 10, 42).expect("registered");
    let mut sixty = Player::new("double_pendulum", 30, 10, 42).expect("registered");
    assert!(one.advance(1.0));
    for _ in 0..60 {
        assert!(sixty.advance(1.0 / 60.0));
    }
    assert_eq!(one.steps(), 60);
    assert_eq!(sixty.steps(), 60);
    assert_eq!(
        one.frame(Theme::Astro).to_vec(),
        sixty.frame(Theme::Astro).to_vec()
    );
    // Animation frames of 1/144 s accumulate: floor(60 · total + 1e-6) steps in all.
    let mut fine = Player::new("double_pendulum", 30, 10, 42).expect("registered");
    for _ in 0..144 {
        assert!(fine.advance(1.0 / 144.0));
    }
    assert_eq!(fine.steps(), 60);
    // A negative, non-finite or over-long advance steps nothing.
    for bad in [-0.1, f64::NAN, f64::INFINITY, 3600.5] {
        assert!(!one.advance(bad), "{bad}");
        assert_eq!(one.steps(), 60, "{bad}");
    }
}

#[test]
fn web_r3_player_resizes_and_clamps_like_the_library() {
    let mut player = Player::new("aurora", 0, 5000, 1).expect("registered");
    assert_eq!((player.columns(), player.rows()), (1, 1024));
    player.resize(50, 20);
    assert_eq!((player.columns(), player.rows()), (50, 20));
    assert_eq!(player.frame(Theme::Mono).len(), 50 * 8 * 20 * 16 * 4);
    // A resized player draws what a stage resized at the same step draws.
    let mut player = Player::new("solar_system", 30, 10, 3).expect("registered");
    assert!(player.advance(0.5));
    player.resize(36, 12);
    assert!(player.advance(0.5));
    let entry = sciart_generators::find("solar_system").expect("registered");
    let mut stage = Stage::new((entry.build)(), 3, Size::new(30, 10), GlyphMode::Unicode);
    for _ in 0..30 {
        stage.step(SIM_DT);
    }
    stage.resize(Size::new(36, 12));
    for _ in 0..30 {
        stage.step(SIM_DT);
    }
    let options = RenderOptions {
        theme: Theme::Aurora,
        ascii: false,
    };
    assert_eq!(
        rgb(player.frame(Theme::Aurora)),
        render_frame(&mut stage, &options).rgb()
    );
}

#[test]
fn web_r3_player_resolves_names_and_aliases() {
    assert!(Player::new("no_such_scene", 10, 10, 1).is_none());
    let renamed = Player::new("chemical_reaction", 10, 10, 1).expect("a retired name resolves");
    assert_eq!(renamed.scene(), "gray_scott");
    let names: Vec<&str> = themes().to_vec();
    assert_eq!(
        names,
        Theme::ALL.iter().map(|t| t.name()).collect::<Vec<_>>()
    );
}

/// The `sciart` binary of the product workspace: a built one if there is one, else
/// `cargo run` (the same lookup as `scripts/gallery.py`).
fn sciart_list_json() -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for profile in ["ci", "debug", "release"] {
        let binary = root.join("target").join(profile).join("sciart");
        if binary.is_file() {
            let out = Command::new(&binary)
                .args(["list", "--json"])
                .output()
                .expect("sciart runs");
            if out.status.success() {
                return String::from_utf8(out.stdout).expect("UTF-8");
            }
        }
    }
    let out = Command::new(env!("CARGO"))
        .args(["run", "-q", "-p", "sciart", "--", "list", "--json"])
        .current_dir(&root)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("UTF-8")
}

#[test]
fn web_r3_scenes_is_the_json_of_sciart_list() {
    assert_eq!(scenes(), sciart_list_json());
}
