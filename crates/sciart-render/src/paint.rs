use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use sciart_core::{Canvas, Role};

use crate::Theme;

/// Write every canvas cell that fits inside `area` into `buf` (RND-R3).
pub fn paint(canvas: &Canvas, theme: Theme, area: Rect, buf: &mut Buffer) {
    let width = usize::from(canvas.size().width());
    for (i, cell) in canvas.cells().iter().enumerate() {
        let (cx, cy) = ((i % width) as u16, (i / width) as u16);
        if cx >= area.width || cy >= area.height {
            continue;
        }
        let Some(out) = buf.cell_mut((area.x + cx, area.y + cy)) else {
            continue;
        };
        if cell.ink.role == Role::Background {
            out.set_char(cell.ch).set_style(Style::reset());
        } else {
            let rgb = theme.color(cell.ink, cx, cy);
            out.set_char(cell.ch)
                .set_style(Style::reset().fg(Color::Rgb(rgb.0, rgb.1, rgb.2)));
        }
    }
}

#[cfg(test)]
mod tests {
    use sciart_core::{Glyph, GlyphMode, Ink, Size};

    use super::*;

    #[test]
    fn rnd_r3_paints_glyphs_and_colors_background_as_reset() {
        let mut canvas = Canvas::new(Size::new(3, 2), GlyphMode::Unicode);
        canvas.put(1, 1, Glyph::new('★', '*'), Ink::new(Role::Star, 255));
        let area = Rect::new(0, 0, 3, 2);
        let mut buf = Buffer::empty(area);
        buf[(0, 0)].set_char('X');

        paint(&canvas, Theme::Mono, area, &mut buf);

        assert_eq!(
            buf[(0, 0)].symbol(),
            " ",
            "blank cells overwrite stale content"
        );
        assert_eq!(buf[(0, 0)].fg, Color::Reset);
        assert_eq!(buf[(1, 1)].symbol(), "★");
        assert_eq!(buf[(1, 1)].fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn rnd_r3_canvas_larger_than_area_is_clipped() {
        let mut canvas = Canvas::new(Size::new(10, 10), GlyphMode::Ascii);
        canvas.put(9, 9, Glyph::ascii('x'), Ink::new(Role::Star, 1));
        let area = Rect::new(2, 1, 4, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 6));
        paint(&canvas, Theme::Astro, area, &mut buf); // must not panic
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }
}
