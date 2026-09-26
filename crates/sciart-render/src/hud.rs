//! One-line status overlay (SPEC-003 RND-R4, spec F4).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use crate::Theme;

#[derive(Debug, Clone)]
pub struct Hud<'a> {
    pub generator: &'a str,
    pub fps: f64,
    pub theme: Theme,
    pub seed: u64,
    pub paused: bool,
    pub quarantined: usize,
}

impl Hud<'_> {
    pub fn line(&self) -> String {
        let mut parts = vec![
            self.generator.to_string(),
            format!("{:.0} fps", self.fps),
            self.theme.name().to_string(),
            format!("seed {}", self.seed),
        ];
        if self.paused {
            parts.push("PAUSED".into());
        }
        if self.quarantined > 0 {
            parts.push(format!("{} quarantined", self.quarantined));
        }
        parts.push("n p space r h t q".into());
        parts.join(" | ")
    }
}

impl Widget for &Hud<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }
        let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        Line::from(self.line()).reversed().render(row, buf);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    fn hud() -> Hud<'static> {
        Hud {
            generator: "starfield",
            fps: 29.6,
            theme: Theme::Astro,
            seed: 42,
            paused: false,
            quarantined: 0,
        }
    }

    #[test]
    fn rnd_r4_line_format() {
        assert_eq!(
            hud().line(),
            "starfield | 30 fps | astro | seed 42 | n p space r h t q"
        );
        let h = Hud {
            paused: true,
            quarantined: 2,
            ..hud()
        };
        assert_eq!(
            h.line(),
            "starfield | 30 fps | astro | seed 42 | PAUSED | 2 quarantined | n p space r h t q"
        );
        assert!(h.line().is_ascii());
    }

    #[test]
    fn rnd_r4_renders_only_the_last_row_clipped_and_reversed() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(area);
        for y in 0..3 {
            buf[(0, y)].set_char('#');
        }
        hud().render(area, &mut buf);

        assert_eq!(buf[(0, 0)].symbol(), "#");
        assert_eq!(buf[(0, 1)].symbol(), "#");
        let bottom: String = (0..12).map(|x| buf[(x, 2)].symbol().to_string()).collect();
        assert_eq!(bottom, "starfield | ");
        assert!(buf[(0, 2)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn rnd_r4_zero_height_area_is_a_noop() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 1));
        hud().render(Rect::new(0, 0, 5, 0), &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }
}
