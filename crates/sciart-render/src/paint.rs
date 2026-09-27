use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use sciart_core::{Canvas, Ink, Role};

use crate::Theme;

/// Write every canvas cell that fits inside `area` into `buf` (RND-R3).
///
/// A `Role::Background` ink means "terminal default", for the foreground (RND-R3) and
/// for the cell background (RND-R7, CORE-R40) alike.
pub fn paint(canvas: &Canvas, theme: Theme, area: Rect, buf: &mut Buffer) {
    let width = usize::from(canvas.size().width());
    let rgb = |ink: Ink, x, y| {
        let c = theme.color(ink, x, y);
        Color::Rgb(c.0, c.1, c.2)
    };
    for (i, cell) in canvas.cells().iter().enumerate() {
        let (cx, cy) = ((i % width) as u16, (i / width) as u16);
        if cx >= area.width || cy >= area.height {
            continue;
        }
        let Some(out) = buf.cell_mut((area.x + cx, area.y + cy)) else {
            continue;
        };
        let mut style = Style::reset();
        if cell.ink.role != Role::Background {
            style = style.fg(rgb(cell.ink, cx, cy));
        }
        if cell.bg.role != Role::Background {
            style = style.bg(rgb(cell.bg, cx, cy));
        }
        out.set_char(cell.ch).set_style(style);
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
    fn rnd_r7_paints_the_cell_background_ink() {
        let mut canvas = Canvas::new(Size::new(3, 1), GlyphMode::Unicode);
        let (top, bottom) = (Ink::new(Role::Seq, 40), Ink::new(Role::Div, 250));
        canvas.put_bg(0, 0, Glyph::new('▀', '#'), top, bottom);
        canvas.put(1, 0, Glyph::ascii('x'), top);
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        buf[(1, 0)].set_bg(Color::Red); // stale background must be reset

        paint(&canvas, Theme::Astro, area, &mut buf);

        let rgb = |ink| {
            let c = Theme::Astro.color(ink, 0, 0);
            Color::Rgb(c.0, c.1, c.2)
        };
        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert_eq!(buf[(0, 0)].fg, rgb(top));
        assert_eq!(buf[(0, 0)].bg, rgb(bottom));
        assert_eq!(
            buf[(1, 0)].bg,
            Color::Reset,
            "no bg ink: default background"
        );
        assert_eq!(buf[(2, 0)].bg, Color::Reset);
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
