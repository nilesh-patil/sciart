//! A ratatui widget that draws a [`Stage`] (feature `widget`, LIB-R1, LIB-R5).
//!
//! The only facade file that names ratatui types (LIB-R2). Built against ratatui 0.30: a
//! ratatui release with a new major version needs a new sciart major version or a new,
//! versioned feature.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::{Stage, Theme};

/// Draws a stage's current frame into a ratatui area (ratatui 0.30).
///
/// Rendering resizes the stage to the area, draws a frame and writes the theme's exact
/// colours into the area's cells; cells outside the area are untouched. It never advances
/// time: call [`Stage::advance`] between draws. A failed stage draws nothing.
///
/// ```
/// use ratatui::{Terminal, backend::TestBackend};
/// use sciart::{Options, SceneWidget, Stage, Theme};
///
/// let mut stage = Stage::new("comet_tails", 1, 1, Options::default())?;
/// stage.advance(3.0)?;
/// let mut terminal = Terminal::new(TestBackend::new(30, 10)).expect("a test terminal");
/// terminal
///     .draw(|frame| frame.render_widget(SceneWidget::new(&mut stage).theme(Theme::Rainbow), frame.area()))
///     .expect("draw");
/// assert_eq!(stage.size(), (30, 10));
/// # Ok::<(), sciart::Error>(())
/// ```
#[derive(Debug)]
pub struct SceneWidget<'a> {
    stage: &'a mut Stage,
    theme: Theme,
}

impl<'a> SceneWidget<'a> {
    /// A widget for `stage` in the default theme.
    pub fn new(stage: &'a mut Stage) -> Self {
        Self {
            stage,
            theme: Theme::default(),
        }
    }

    /// Draw in `theme`.
    #[must_use]
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }
}

impl Widget for SceneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        if area.is_empty() || self.stage.resize(area.width, area.height).is_err() {
            return;
        }
        if let Ok(canvas) = self.stage.draw() {
            sciart_render::paint(canvas, self.theme.render(), area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;
    use ratatui::style::Color;

    use super::*;
    use crate::Options;
    use crate::stage::tests::{PanicIn, fragile};

    /// Draw `stage` into `area` of a `width` × `height` test terminal.
    fn draw(stage: &mut Stage, theme: Theme, width: u16, height: u16, area: Rect) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(SceneWidget::new(stage).theme(theme), area))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn lib_r5_the_widget_resizes_its_stage_to_the_area() {
        let mut stage = Stage::new("solar_system", 3, 3, Options::default()).unwrap();
        draw(&mut stage, Theme::Astro, 50, 20, Rect::new(5, 2, 31, 11));
        assert_eq!(stage.size(), (31, 11));
        draw(&mut stage, Theme::Astro, 10, 4, Rect::new(0, 0, 50, 50));
        assert_eq!(
            stage.size(),
            (10, 4),
            "an area larger than the buffer is clipped"
        );
    }

    #[test]
    fn lib_r5_the_widget_writes_the_exact_theme_colours_only_inside_its_area() {
        let mut stage = Stage::new("aurora", 1, 1, Options::default()).unwrap();
        stage.advance(5.0).unwrap();
        let area = Rect::new(6, 3, 40, 12);
        let buf = draw(&mut stage, Theme::Aurora, 60, 20, area);

        // The same frame painted by the renderer the binary uses, at the origin.
        let canvas = stage.draw().unwrap().clone();
        let mut expected = Buffer::empty(Rect::new(0, 0, 40, 12));
        sciart_render::paint(
            &canvas,
            Theme::Aurora.render(),
            expected.area,
            &mut expected,
        );

        let blank = Cell::default();
        let mut lit = 0;
        for y in 0..20 {
            for x in 0..60 {
                let cell = &buf[(x, y)];
                if area.contains((x, y).into()) {
                    assert_eq!(cell, &expected[(x - area.x, y - area.y)], "({x}, {y})");
                    lit += usize::from(cell.fg != Color::Reset);
                } else {
                    assert_eq!(cell, &blank, "({x}, {y}) is outside the area");
                }
            }
        }
        assert!(lit > 0, "the scene drew something");
    }

    #[test]
    fn lib_r6_a_failed_stage_draws_nothing() {
        let mut stage = fragile(PanicIn::Render).unwrap();
        let buf = draw(&mut stage, Theme::Astro, 8, 4, Rect::new(0, 0, 8, 4));
        assert!(stage.is_failed());
        assert_eq!(buf, Buffer::empty(Rect::new(0, 0, 8, 4)));
    }
}
