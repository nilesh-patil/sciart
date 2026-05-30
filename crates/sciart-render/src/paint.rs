//! Canvas → ratatui buffer (RND-R3, RND-R7, RND-R14, RND-R15).
//!
//! [`paint`] is stateless: the exact theme colours, for tests, the art metrics and the
//! probe. The app paints through a [`Painter`], which adds colour hysteresis (RND-R14):
//! it remembers what it last emitted per cell and keeps an imperceptibly different
//! colour, so ratatui's diff sends nothing for a cell whose field drifted by a level.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use sciart_core::{Canvas, Ink, Role};

use crate::Theme;
use crate::theme::{HuePos, hue_pos, lut, per_cell, rainbow_at, role_index};

/// Largest per-channel change from the emitted colour that is not re-sent (RND-R14): two
/// 8-bit levels, below what a viewer can see on a moving field, small enough that a slow
/// fade still steps.
pub const HYSTERESIS: u8 = 2;

/// Write every canvas cell that fits inside `area` into `buf` (RND-R3), with the exact
/// theme colours.
///
/// A `Role::Background` ink means "terminal default", for the foreground (RND-R3) and
/// for the cell background (RND-R7, CORE-R40). Attributes that cannot change what the
/// cell looks like are left at the default (RND-R15): a blank has no foreground, and a
/// half block whose two halves are one colour is drawn as a blank in that background.
pub fn paint(canvas: &Canvas, theme: Theme, area: Rect, buf: &mut Buffer) {
    paint_with(canvas, theme, area, buf, &[], &mut []);
}

/// A packed cell colour: 0 is the terminal default, `RGB | 1 << 24` a colour. One
/// compare per colour in the hot loop instead of matching `Color`.
type Packed = u32;
const DEFAULT: Packed = 0;
const SET: Packed = 1 << 24;

fn pack(c: crate::Rgb) -> Packed {
    SET | u32::from(c.0) << 16 | u32::from(c.1) << 8 | u32::from(c.2)
}

fn unpack(c: Packed) -> Color {
    if c == DEFAULT {
        Color::Reset
    } else {
        Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
    }
}

/// What was last emitted for one cell (RND-R14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Emitted {
    ch: char,
    fg: Packed,
    bg: Packed,
}

impl Emitted {
    /// Matches no painted cell (`'\0'` is never a canvas glyph), so the first frame after
    /// a reset is emitted exactly.
    const NONE: Emitted = Emitted {
        ch: '\0',
        fg: DEFAULT,
        bg: DEFAULT,
    };
}

/// Paints with colour hysteresis (RND-R14) and per-size tables; owned by the app for the
/// lifetime of the terminal.
///
/// The memory resets by itself when the theme or the canvas size changes; the app calls
/// [`Painter::reset`] when the scene changes or is reset.
#[derive(Debug, Default)]
pub struct Painter {
    theme: Option<Theme>,
    width: u16,
    height: u16,
    emitted: Vec<Emitted>,
    /// Rainbow's per-position hue parts (RND-R15), rebuilt on resize.
    hues: Vec<HuePos>,
    hysteresis: bool,
}

impl Painter {
    /// A painter with hysteresis on.
    pub fn new() -> Self {
        Self {
            hysteresis: true,
            ..Self::default()
        }
    }

    /// A painter that always emits the exact colours (for comparisons).
    pub fn exact() -> Self {
        Self::default()
    }

    /// Forget every emitted colour: the next frame is painted exactly (scene change or
    /// reset, RND-R14).
    pub fn reset(&mut self) {
        self.emitted.fill(Emitted::NONE);
    }

    pub fn paint(&mut self, canvas: &Canvas, theme: Theme, area: Rect, buf: &mut Buffer) {
        let size = canvas.size();
        let (w, h) = (size.width(), size.height());
        if (w, h) != (self.width, self.height) {
            (self.width, self.height) = (w, h);
            let n = usize::from(w) * usize::from(h);
            self.emitted.clear();
            self.emitted.resize(n, Emitted::NONE);
            self.hues.clear();
            self.hues
                .extend((0..h).flat_map(|y| (0..w).map(move |x| hue_pos(x, y))));
        } else if self.theme != Some(theme) {
            self.reset();
        }
        self.theme = Some(theme);
        let memory: &mut [Emitted] = if self.hysteresis {
            &mut self.emitted
        } else {
            &mut []
        };
        paint_with(canvas, theme, area, buf, &self.hues, memory);
    }
}

/// Within [`HYSTERESIS`] in every channel of the emitted colour `a`: both set and close,
/// or equal.
#[inline]
fn near(a: Packed, b: Packed) -> bool {
    // Branch-free: on a moving field "near" is a coin flip per cell, so `&&` chains
    // mispredict. `(x − y + H) as u32 ≤ 2H` ⇔ `|x − y| ≤ H`.
    let both_set = (a & b) >> 24 == 1;
    let h = i32::from(HYSTERESIS);
    let d = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as i32, ((b >> shift) & 0xFF) as i32);
        (x - y + h) as u32 <= 2 * h as u32
    };
    (a == b) | (both_set & d(16) & d(8) & d(0))
}

/// The painting loop: `hues` (per-position rainbow parts) and `memory` (hysteresis) may
/// be empty, in which case rainbow hues are computed per cell and colours are exact.
fn paint_with(
    canvas: &Canvas,
    theme: Theme,
    area: Rect,
    buf: &mut Buffer,
    hues: &[HuePos],
    memory: &mut [Emitted],
) {
    let width = usize::from(canvas.size().width());
    let table = lut(theme);
    let color = |ink: Ink, i: usize, x: u16, y: u16| -> Packed {
        if ink.role == Role::Background {
            return DEFAULT;
        }
        let c = match role_index(ink.role) {
            Some(r) if !per_cell(theme, ink.role) => table.colors[r][usize::from(ink.level)],
            _ => hues
                .get(i)
                .filter(|_| per_cell(theme, ink.role))
                .map_or_else(|| theme.color(ink, x, y), |&p| rainbow_at(p, ink.level)),
        };
        pack(c)
    };
    // `paint` is only ever given an area inside its buffer; clip to both anyway.
    if area.x < buf.area.x || area.y < buf.area.y {
        return;
    }
    let right = area.right().min(buf.area.right());
    let bottom = area.bottom().min(buf.area.bottom());
    let cols = usize::from(right - area.x).min(width);
    let rows = usize::from(bottom - area.y).min(canvas.cells().len() / width.max(1));
    for cy in 0..rows {
        let src = &canvas.cells()[cy * width..cy * width + cols];
        let start = buf.index_of(area.x, area.y + cy as u16);
        let dst = &mut buf.content[start..start + cols];
        for (cx, (cell, out)) in src.iter().zip(dst).enumerate() {
            let i = cy * width + cx;
            let (x, y) = (cx as u16, cy as u16);
            let mut ch = cell.ch;
            let mut fg = color(cell.ink, i, x, y);
            let mut bg = color(cell.bg, i, x, y);
            // Invisible attributes (RND-R15): a blank shows no foreground, and a half
            // block of one colour is a blank in that colour.
            if fg == bg && bg != DEFAULT && matches!(ch, '▀' | '▄') {
                ch = ' ';
            }
            if ch == ' ' {
                fg = DEFAULT;
            }
            if let Some(m) = memory.get_mut(i) {
                if m.ch == ch {
                    if near(m.fg, fg) {
                        fg = m.fg;
                    }
                    if near(m.bg, bg) {
                        bg = m.bg;
                    }
                }
                let e = Emitted { ch, fg, bg };
                if *m != e {
                    *m = e;
                }
            }
            out.set_char(ch);
            out.fg = unpack(fg);
            out.bg = unpack(bg);
            out.modifier = Modifier::empty();
        }
    }
}

#[cfg(test)]
mod tests {
    use sciart_core::{Glyph, GlyphMode, Ink, Size};

    use super::*;

    fn rgb(theme: Theme, ink: Ink) -> Color {
        let c = theme.color(ink, 0, 0);
        Color::Rgb(c.0, c.1, c.2)
    }

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

        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert_eq!(buf[(0, 0)].fg, rgb(Theme::Astro, top));
        assert_eq!(buf[(0, 0)].bg, rgb(Theme::Astro, bottom));
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

    #[test]
    fn rnd_r15_invisible_attributes_are_dropped() {
        let mut canvas = Canvas::new(Size::new(3, 1), GlyphMode::Unicode);
        let ink = Ink::new(Role::Seq, 90);
        // A blank with a background ink and a (meaningless) foreground ink.
        canvas.put_bg(0, 0, Glyph::ascii(' '), Ink::new(Role::Star, 200), ink);
        // A half block whose halves are one colour.
        canvas.put_bg(1, 0, Glyph::new('▀', '#'), ink, ink);
        canvas.put_bg(2, 0, Glyph::new('▄', '#'), ink, ink);
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        paint(&canvas, Theme::Astro, area, &mut buf);
        for x in 0..3 {
            let cell = &buf[(x, 0)];
            assert_eq!(cell.symbol(), " ", "cell {x}");
            assert_eq!(cell.fg, Color::Reset, "cell {x}: a blank has no fg");
            assert_eq!(cell.bg, rgb(Theme::Astro, ink), "cell {x}: same look");
        }
    }

    /// Paints one cell whose background is `Seq` at `level`, through `painter`.
    fn paint_level(painter: &mut Painter, theme: Theme, level: u8) -> Color {
        let mut canvas = Canvas::new(Size::new(1, 1), GlyphMode::Unicode);
        canvas.put_bg(
            0,
            0,
            Glyph::new('▀', '#'),
            Ink::new(Role::Seq, 0),
            Ink::new(Role::Seq, level),
        );
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        painter.paint(&canvas, theme, area, &mut buf);
        buf[(0, 0)].bg
    }

    fn channels(c: Color) -> [u8; 3] {
        match c {
            Color::Rgb(r, g, b) => [r, g, b],
            other => panic!("not rgb: {other:?}"),
        }
    }

    #[test]
    fn rnd_r14_small_changes_keep_the_emitted_colour_and_error_is_bounded() {
        let mut painter = Painter::new();
        let first = paint_level(&mut painter, Theme::Astro, 100);
        assert_eq!(
            first,
            rgb(Theme::Astro, Ink::new(Role::Seq, 100)),
            "exact at first"
        );
        let mut emitted = first;
        let mut kept = 0;
        // A slow ramp up and back: every frame's error stays within HYSTERESIS of the
        // true colour, and the output still follows the fade (it cannot drift or stick).
        for level in (100..=160u8).chain((100..160u8).rev()) {
            let truth = channels(rgb(Theme::Astro, Ink::new(Role::Seq, level)));
            let got = paint_level(&mut painter, Theme::Astro, level);
            let c = channels(got);
            for k in 0..3 {
                assert!(
                    c[k].abs_diff(truth[k]) <= HYSTERESIS,
                    "level {level}: {c:?} vs {truth:?}"
                );
            }
            if got == emitted {
                kept += 1;
            }
            emitted = got;
        }
        assert!(kept > 0, "some frames keep the emitted colour");
        let changed = 121 - kept;
        assert!(changed > 20, "the fade still moves: {changed} changes");
    }

    #[test]
    fn rnd_r14_large_changes_glyph_changes_and_resets_are_exact() {
        let mut painter = Painter::new();
        let seq = |l| rgb(Theme::Astro, Ink::new(Role::Seq, l));
        paint_level(&mut painter, Theme::Astro, 100);
        assert_eq!(
            paint_level(&mut painter, Theme::Astro, 200),
            seq(200),
            "big step"
        );
        // A one-level nudge is kept...
        let nudged = paint_level(&mut painter, Theme::Astro, 201);
        assert_eq!(nudged, seq(200));
        // ...until the memory resets: scene change / reset, theme change, resize.
        painter.reset();
        assert_eq!(paint_level(&mut painter, Theme::Astro, 201), seq(201));
        paint_level(&mut painter, Theme::Mono, 201);
        assert_eq!(
            paint_level(&mut painter, Theme::Astro, 200),
            seq(200),
            "theme change resets"
        );
        let mut exact = Painter::exact();
        paint_level(&mut exact, Theme::Astro, 200);
        assert_eq!(paint_level(&mut exact, Theme::Astro, 201), seq(201));
    }

    #[test]
    fn rnd_r14_r15_painter_matches_paint_without_hysteresis() {
        let mut canvas = Canvas::new(Size::new(7, 3), GlyphMode::Unicode);
        let roles = [Role::Star, Role::Seq, Role::Div, Role::Text, Role::Planet];
        for (i, role) in roles.iter().enumerate() {
            let ink = Ink::new(*role, (i * 50) as u8);
            canvas.put_bg(
                i as i32,
                0,
                Glyph::new('▀', '#'),
                ink,
                Ink::new(Role::Seq, 9),
            );
            canvas.put(i as i32, 1, Glyph::new('★', '*'), ink);
        }
        let area = Rect::new(0, 0, 7, 3);
        for theme in Theme::ALL {
            let (mut a, mut b) = (Buffer::empty(area), Buffer::empty(area));
            paint(&canvas, theme, area, &mut a);
            Painter::exact().paint(&canvas, theme, area, &mut b);
            assert_eq!(a, b, "{theme:?}");
        }
    }
}
