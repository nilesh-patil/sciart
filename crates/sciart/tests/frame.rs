//! CLI-R13 and LIB-R5: `sciart frame` through the real binary, against the library.

use std::process::{Command, Output};

use sciart::{Options, Stage, Theme};

fn frame(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sciart"));
    command.arg("frame").args(args).env_remove("NO_COLOR");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("spawn the sciart binary")
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).expect("UTF-8 output")
}

#[test]
fn cli_r13_prints_one_reset_terminated_line_per_row() {
    let text = stdout(&frame(&["aurora", "--size", "48x14"], &[]));
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    assert_eq!(lines.len(), 14, "piped stdout: the given size");
    for line in &lines {
        assert!(line.ends_with("\x1b[0m\n"), "{line:?}");
    }
    assert!(text.contains("\x1b[38;2;"), "truecolor by default");
}

#[test]
fn cli_r13_off_a_terminal_the_default_size_is_80x24() {
    let text = stdout(&frame(&["starfield"], &[]));
    assert_eq!(text.lines().count(), 24);
}

#[test]
fn cli_r13_never_enters_a_terminal_session() {
    let text = stdout(&frame(&["black_hole", "--size", "30x8"], &[]));
    for session in ["\x1b[?1049h", "\x1b[?25l", "\x1b[H", "\x1b[2J", "\r"] {
        assert!(!text.contains(session), "{session:?}");
    }
}

#[test]
fn cli_r13_the_same_arguments_give_the_same_bytes() {
    let args = [
        "galaxy_collision",
        "--size",
        "50x16",
        "--t",
        "7.25",
        "--seed",
        "3",
    ];
    let a = frame(&args, &[]);
    let b = frame(&args, &[]);
    assert_eq!(stdout(&a), stdout(&b));
    let other_seed = frame(
        &[
            "galaxy_collision",
            "--size",
            "50x16",
            "--t",
            "7.25",
            "--seed",
            "4",
        ],
        &[],
    );
    assert_ne!(
        stdout(&a),
        stdout(&other_seed),
        "the seed reaches the scene"
    );
}

#[test]
fn cli_r13_lib_r5_frame_prints_the_librarys_ansi() {
    for (scene, theme, seed, t, title) in [
        ("aurora", Theme::Astro, 42, 5.0, true),
        ("double_pendulum", Theme::Aurora, 9, 2.5, false),
        ("moon_landing", Theme::Rainbow, 1, 12.0, true),
    ] {
        let t_arg = t.to_string();
        let seed_arg = seed.to_string();
        let mut args = vec![
            scene,
            "--size",
            "64x20",
            "--t",
            &t_arg,
            "--seed",
            &seed_arg,
            "--theme",
            theme.name(),
        ];
        if !title {
            args.push("--no-title");
        }
        let cli = stdout(&frame(&args, &[]));
        let mut stage =
            Stage::new(scene, 64, 20, Options::default().seed(seed).title(title)).unwrap();
        stage.advance(t).unwrap();
        assert_eq!(cli, stage.to_ansi(theme).unwrap(), "{scene}");
    }
}

#[test]
fn cli_r13_colour_modes() {
    let indexed = stdout(&frame(
        &["aurora", "--size", "40x12", "--color", "256"],
        &[],
    ));
    assert!(
        indexed.contains("\x1b[38;5;") && !indexed.contains(";2;"),
        "{indexed:?}"
    );

    let mono = stdout(&frame(
        &["aurora", "--size", "40x12", "--color", "mono"],
        &[],
    ));
    let mut stage = Stage::new("aurora", 40, 12, Options::default()).unwrap();
    stage.advance(5.0).unwrap();
    assert_eq!(mono, stage.to_ansi(Theme::Mono).unwrap());

    let no_color = frame(&["aurora", "--size", "40x12"], &[("NO_COLOR", "1")]);
    assert_eq!(stdout(&no_color), mono, "NO_COLOR selects mono");
    let warning = String::from_utf8_lossy(&no_color.stderr);
    assert_eq!(warning.matches("warning").count(), 1, "{warning}");
}

#[test]
fn cli_r13_glyph_and_title_flags() {
    let ascii = stdout(&frame(&["saturn_rings", "--size", "60x18", "--ascii"], &[]));
    let visible: String = ascii
        .split('\x1b')
        .map(|s| s.split_once('m').map_or(s, |(_, rest)| rest))
        .collect();
    assert!(visible.is_ascii(), "{visible:?}");
    let titled = stdout(&frame(&["saturn_rings", "--size", "60x18"], &[]));
    let untitled = stdout(&frame(
        &["saturn_rings", "--size", "60x18", "--no-title"],
        &[],
    ));
    assert_ne!(titled, untitled);
}

#[test]
fn cli_r13_usage_errors_exit_2() {
    for args in [
        &["no_such_scene"][..],
        &["aurora", "--t", "-1"],
        &["aurora", "--t", "3600.01"],
        &["aurora", "--t", "inf"],
        &["aurora", "--size", "0x10"],
    ] {
        let output = frame(args, &[]);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn cli_r13_never_reads_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "theme = \"mono\"\nthis_key_is_unknown = 1\n").unwrap();
    let path = config.to_str().unwrap();
    let with = stdout(&frame(
        &["aurora", "--size", "40x12", "--config", path],
        &[],
    ));
    let without = stdout(&frame(&["aurora", "--size", "40x12"], &[]));
    assert_eq!(with, without);
}
