//! Asciicast v2 output (CLI-R8): the same frames as ANSI text, replayable in asciinema.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use sciart_core::prelude::*;
use sciart_render::{AnsiColor, AnsiFraming, Theme, write_ansi};

use crate::RenderOptions;

/// The stage's current frame as an ANSI truecolor screen, for an asciicast event.
pub fn render_ansi(stage: &mut Stage, options: &RenderOptions) -> String {
    let previous = stage.glyph_mode();
    stage.set_glyph_mode(if options.ascii {
        GlyphMode::Ascii
    } else {
        GlyphMode::Unicode
    });
    let text = ansi(stage.render(), options.theme);
    stage.set_glyph_mode(previous);
    text
}

/// One frame, starting at the home position: only colour changes emit an SGR sequence, so a
/// cast stays small. The rows are the shared ANSI writer's (LIB-R5): `sciart frame` and the
/// library print the same rows with line framing.
fn ansi(canvas: &Canvas, theme: Theme) -> String {
    let mut out = String::with_capacity(canvas.cells().len() * 4);
    write_ansi(
        canvas,
        theme,
        AnsiColor::Truecolor,
        AnsiFraming::Screen,
        &mut out,
    );
    out
}

/// Write an asciicast v2 file: a header, then one output event per frame at `k / fps`.
/// Frames are consumed and written one at a time, so a long recording never holds them all.
pub fn write_asciicast<I>(path: &Path, size: Size, frames: I, fps: u16) -> std::io::Result<()>
where
    I: IntoIterator<Item = String>,
{
    if fps == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "fps must be at least 1",
        ));
    }
    let mut frames = frames.into_iter().peekable();
    if frames.peek().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no frames to write",
        ));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    // Asciicast v2 header (the `env` block is optional but makes replays faithful).
    writeln!(
        out,
        "{{\"version\": 2, \"width\": {}, \"height\": {}, \"title\": \"sciart record\", \
         \"env\": {{\"TERM\": \"xterm-256color\", \"SHELL\": \"/bin/sh\"}}}}",
        size.width(),
        size.height()
    )?;
    for (k, frame) in frames.enumerate() {
        let time = k as f64 / f64::from(fps);
        writeln!(out, "[{time:.6}, \"o\", \"{}\"]", json_escape(&frame))?;
    }
    out.flush()
}

/// JSON string escaping for the event payload.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_r8_ansi_frame_uses_truecolor_and_the_glyphs() {
        let mut canvas = Canvas::new(Size::new(2, 1), GlyphMode::Unicode);
        canvas.put(0, 0, Glyph::new('A', '#'), Ink::new(Role::Hot, 255));
        canvas.put(1, 0, Glyph::new('B', '#'), Ink::new(Role::Hot, 255));
        let text = ansi(&canvas, Theme::Astro);
        assert!(text.starts_with("\x1b[0m\x1b[H"));
        assert_eq!(
            text.matches("\x1b[38;2;").count(),
            1,
            "one SGR for one colour"
        );
        assert!(text.contains("AB"));
        assert!(text.ends_with("\x1b[0m"));
    }

    #[test]
    fn cli_r8_the_cast_and_the_png_share_the_default_foreground() {
        assert_eq!(crate::image::TERM_FG, sciart_render::DEFAULT_FG);
    }

    #[test]
    fn cli_r8_asciicast_has_the_v2_header_and_frame_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.cast");
        let frames = ["\x1b[Hone".to_owned(), "\x1b[Htwo".to_owned()];
        write_asciicast(&path, Size::new(80, 24), frames, 10).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("\"version\": 2"));
        assert!(lines[0].contains("\"width\": 80"));
        assert!(lines[0].contains("\"height\": 24"));
        assert!(lines[1].starts_with("[0.000000, \"o\", \""), "{}", lines[1]);
        assert!(lines[2].starts_with("[0.100000, \"o\", \""), "{}", lines[2]);
        assert!(lines[2].contains("two"));
    }

    #[test]
    fn cli_r8_asciicast_escapes_the_event_payload() {
        assert_eq!(json_escape("a\"b\\c\nd\x1b"), "a\\\"b\\\\c\\nd\\u001b");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("escape.cast");
        write_asciicast(&path, Size::new(1, 1), ["\"\\".to_owned()], 60).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[0.000000, \"o\", \"\\\"\\\\\"]"), "{text}");
    }

    #[test]
    fn cli_r8_asciicast_rejects_empty_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.cast");
        assert!(write_asciicast(&path, Size::new(1, 1), std::iter::empty(), 30).is_err());
        assert!(write_asciicast(&path, Size::new(1, 1), ["x".to_owned()], 0).is_err());
    }
}
