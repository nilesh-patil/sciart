//! A canvas as ANSI text without a terminal session (CLI-R8, CLI-R13, LIB-R5).
//!
//! The one writer behind `sciart frame`, the library's `Stage::render_ansi` and the asciicast
//! recorder: every row starts with no colour state, sets a colour only when it changes and
//! ends with an SGR reset, so a row can be printed on its own. The two framings only differ
//! in what goes around the rows: [`AnsiFraming::Lines`] ends every row with `\n` (for
//! stdout, a file or a log), [`AnsiFraming::Screen`] homes the cursor first and joins the
//! rows with `\r\n` (an asciicast event replayed on a terminal in raw mode).

use std::fmt::Write as _;

use sciart_core::{Canvas, Role};

use crate::color::quantize;
use crate::{Rgb, Theme};

/// The foreground written for a glyph whose ink is `Role::Background`: the light grey a
/// terminal uses by default, so the text stays readable on the terminal's own background.
pub const DEFAULT_FG: Rgb = Rgb(204, 204, 204);

/// How colours are written (RND-R18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnsiColor {
    /// 24-bit `38;2;r;g;b` / `48;2;r;g;b`.
    #[default]
    Truecolor,
    /// The nearest xterm-256 index, `38;5;n` / `48;5;n`, through the live painter's table.
    Indexed,
}

/// What goes around the rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiFraming {
    /// Every row followed by `\n` (CLI-R13).
    Lines,
    /// `ESC[0m ESC[H`, then the rows joined by `\r\n` (CLI-R8 asciicast events).
    Screen,
}

/// Append `canvas` to `out` as ANSI text. Cells whose background ink is
/// `Role::Background` are left on the terminal's own background (`49`), as in a
/// recording; colours come from [`Theme::color`], so the output is a pure function of the
/// canvas, the theme and the colour mode.
pub fn write_ansi(
    canvas: &Canvas,
    theme: Theme,
    color: AnsiColor,
    framing: AnsiFraming,
    out: &mut String,
) {
    let width = usize::from(canvas.size().width()).max(1);
    if framing == AnsiFraming::Screen {
        out.push_str("\x1b[0m\x1b[H");
    }
    for (y, row) in canvas.cells().chunks(width).enumerate() {
        if y > 0 && framing == AnsiFraming::Screen {
            out.push_str("\r\n");
        }
        let mut fg: Option<Rgb> = None;
        // `Some(None)` means the terminal's default background was requested.
        let mut bg: Option<Option<Rgb>> = None;
        for (x, cell) in row.iter().enumerate() {
            let (x, y) = (x as u16, y as u16);
            let fg_colour = if cell.ink.role == Role::Background {
                DEFAULT_FG
            } else {
                theme.color(cell.ink, x, y)
            };
            if fg != Some(fg_colour) {
                sgr(out, 38, fg_colour, color);
                fg = Some(fg_colour);
            }
            let bg_colour = (cell.bg.role != Role::Background).then(|| theme.color(cell.bg, x, y));
            if bg != Some(bg_colour) {
                match bg_colour {
                    Some(colour) => sgr(out, 48, colour, color),
                    None => out.push_str("\x1b[49m"),
                }
                bg = Some(bg_colour);
            }
            out.push(cell.ch);
        }
        out.push_str("\x1b[0m");
        if framing == AnsiFraming::Lines {
            out.push('\n');
        }
    }
}

/// One colour as an SGR sequence: `layer` is 38 (foreground) or 48 (background).
fn sgr(out: &mut String, layer: u8, Rgb(r, g, b): Rgb, color: AnsiColor) {
    let _ = match color {
        AnsiColor::Truecolor => write!(out, "\x1b[{layer};2;{r};{g};{b}m"),
        AnsiColor::Indexed => write!(out, "\x1b[{layer};5;{}m", quantize(Rgb(r, g, b))),
    };
}

#[cfg(test)]
mod tests {
    use sciart_core::{Glyph, GlyphMode, Ink, Size};

    use super::*;

    fn two_rows() -> Canvas {
        let mut canvas = Canvas::new(Size::new(2, 2), GlyphMode::Unicode);
        canvas.put(0, 0, Glyph::new('A', '#'), Ink::new(Role::Hot, 255));
        canvas.put(1, 0, Glyph::new('B', '#'), Ink::new(Role::Hot, 255));
        canvas.put_bg(
            0,
            1,
            Glyph::new('▀', '#'),
            Ink::new(Role::Cold, 200),
            Ink::new(Role::Hot, 90),
        );
        canvas
    }

    fn ansi(canvas: &Canvas, color: AnsiColor, framing: AnsiFraming) -> String {
        let mut out = String::new();
        write_ansi(canvas, Theme::Astro, color, framing, &mut out);
        out
    }

    #[test]
    fn cli_r13_lines_framing_is_one_reset_terminated_line_per_row() {
        let text = ansi(&two_rows(), AnsiColor::Truecolor, AnsiFraming::Lines);
        let lines: Vec<_> = text.split_inclusive('\n').collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        for line in &lines {
            assert!(line.ends_with("\x1b[0m\n"), "{line:?}");
        }
        assert!(!text.contains('\r') && !text.contains("\x1b[H"), "{text:?}");
        assert!(lines[0].contains("AB"));
        assert_eq!(
            lines[0].matches("\x1b[38;2;").count(),
            1,
            "one SGR for one colour"
        );
    }

    #[test]
    fn cli_r8_r13_screen_framing_is_the_same_rows_homed_and_crlf_joined() {
        let canvas = two_rows();
        let lines = ansi(&canvas, AnsiColor::Truecolor, AnsiFraming::Lines);
        let screen = ansi(&canvas, AnsiColor::Truecolor, AnsiFraming::Screen);
        let rows: Vec<_> = lines.lines().collect();
        assert_eq!(screen, format!("\x1b[0m\x1b[H{}", rows.join("\r\n")));
    }

    #[test]
    fn cli_r13_indexed_colour_uses_the_painters_xterm_table() {
        let text = ansi(&two_rows(), AnsiColor::Indexed, AnsiFraming::Lines);
        assert!(!text.contains(";2;"), "{text:?}");
        let hot = Theme::Astro.color(Ink::new(Role::Hot, 255), 0, 0);
        assert!(
            text.contains(&format!("\x1b[38;5;{}m", quantize(hot))),
            "{text:?}"
        );
        assert!(
            text.contains("\x1b[48;5;"),
            "an explicit background is indexed too"
        );
    }

    #[test]
    fn cli_r13_cells_without_a_background_stay_on_the_terminal_background() {
        let mut canvas = Canvas::new(Size::new(3, 1), GlyphMode::Unicode);
        canvas.put(1, 0, Glyph::new('*', '*'), Ink::new(Role::Star, 255));
        let text = ansi(&canvas, AnsiColor::Truecolor, AnsiFraming::Lines);
        assert!(text.contains("\x1b[49m"), "{text:?}");
        assert!(!text.contains("\x1b[48;"), "{text:?}");
    }
}
