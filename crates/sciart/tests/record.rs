//! CLI-R8: `sciart record` end to end, through the real binary.
#![cfg(feature = "record")]

use std::path::Path;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sciart"))
        .args(args)
        .output()
        .expect("spawn the sciart binary")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn gif_frames(path: &Path) -> (u16, u16, usize) {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options
        .read_info(std::fs::File::open(path).expect("open the GIF"))
        .expect("read the GIF header");
    let (width, height) = (decoder.width(), decoder.height());
    let mut frames = 0;
    while decoder.read_next_frame().expect("decode a frame").is_some() {
        frames += 1;
    }
    (width, height, frames)
}

fn png_size(path: &Path) -> (u32, u32) {
    let decoder = png::Decoder::new(std::fs::File::open(path).expect("open the PNG"));
    let reader = decoder.read_info().expect("read the PNG header");
    (reader.info().width, reader.info().height)
}

#[test]
fn cli_r8_record_writes_a_gif_a_png_sequence_and_an_asciicast() {
    let dir = tempfile::tempdir().unwrap();

    // GIF (the default format, inferred from `.gif`), 20x8 cells = 160x128 px.
    let gif = dir.path().join("starfield.gif");
    let out = run(&[
        "record",
        "starfield",
        "--out",
        gif.to_str().unwrap(),
        "--frames",
        "3",
        "--fps",
        "10",
        "--size",
        "20x8",
        "--seed",
        "42",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(gif_frames(&gif), (160, 128, 3));

    // PNG: `--format png` writes frame-000.png, … into the directory PATH.
    let png_dir = dir.path().join("frames");
    let out = run(&[
        "record",
        "starfield",
        "--out",
        png_dir.to_str().unwrap(),
        "--format",
        "png",
        "--frames",
        "2",
        "--fps",
        "10",
        "--size",
        "10x4",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(png_dir.join("frame-000.png").is_file());
    assert!(png_dir.join("frame-001.png").is_file());
    assert!(!png_dir.join("frame-002.png").exists());
    assert_eq!(png_size(&png_dir.join("frame-000.png")), (80, 64));

    // Asciicast: inferred from `.cast`, asciicast v2, one event per frame at k / fps.
    let cast = dir.path().join("a.cast");
    let out = run(&[
        "record",
        "starfield",
        "--out",
        cast.to_str().unwrap(),
        "--frames",
        "2",
        "--fps",
        "10",
        "--size",
        "10x4",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(&cast).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[0].contains("\"version\": 2"), "{}", lines[0]);
    assert!(lines[0].contains("\"width\": 10"), "{}", lines[0]);
    assert!(lines[1].starts_with("[0.000000, \"o\", \""), "{}", lines[1]);
    assert!(lines[2].starts_with("[0.100000, \"o\", \""), "{}", lines[2]);
}

#[test]
fn cli_r8_bad_record_requests_exit_2_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("x.gif");
    let out = out_path.to_str().unwrap();
    for (args, needle) in [
        (vec!["record", "nope", "--out", out], "unknown generator"),
        (
            vec!["record", "starfield", "--out", out, "--fps", "7"],
            "divide 60",
        ),
        (
            vec!["record", "starfield", "--out", out, "--frames", "36001"],
            "36001",
        ),
        (
            vec!["record", "starfield", "--out", out, "--seconds", "1201"],
            "36000",
        ),
        (
            vec!["record", "starfield", "--out", out, "--size", "0x0"],
            "WxH",
        ),
    ] {
        let output = run(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(needle),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(!out_path.exists(), "no file for a bad request: {args:?}");
    }
}

#[test]
fn cli_r8_io_failure_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where a GIF file must go: `File::create` fails, which is an I/O error.
    let target = dir.path().join("a-directory");
    std::fs::create_dir(&target).unwrap();
    let output = run(&[
        "record",
        "starfield",
        "--out",
        target.to_str().unwrap(),
        "--frames",
        "1",
        "--format",
        "gif",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("record:"), "{}", stderr(&output));
}
