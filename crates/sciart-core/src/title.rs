//! Science titles: `Phenomenon · mechanism`, fitted and drawn at the bottom-left of every
//! scene by [`Stage`](crate::Stage) (SPEC-001 CORE-R71..R75, ADR-0014).
//!
//! Everything here works on `&'static str` slices of `Meta.title` and the captions, so
//! fitting and drawing never allocate (GEN-R11). The HUD (RND-R9) reuses [`Title::width`],
//! [`Title::chars`] and [`ascii_of`] to write the same forms.

use std::ops::Range;

use unicode_width::UnicodeWidthStr;

use crate::{Canvas, Generator, Glyph, Ink, Role, Size, ink};

/// Splits a title into phenomenon and mechanism (CORE-R71).
pub const SEPARATOR: &str = " · ";

/// Rows below which no title is shown: on tiny canvases the art needs every row (CORE-R73).
const MIN_HEIGHT: u16 = 6;
/// Ink levels (CORE-R74): the phenomenon reads first, the mechanism recedes.
const PHENOMENON_LEVEL: f64 = 170.0;
const MECHANISM_LEVEL: f64 = 110.0;
/// Ink level of every title-row readout (CORE-R76): the mechanism's tone, so the numbers
/// never outshine the scene's name.
pub const READOUT_LEVEL: u8 = 110;
/// After the viewer has had time to read it, the whole title fades to this level so it
/// stops competing with the art (CORE-R74).
const DIM_LEVEL: f64 = 90.0;
const DIM_FROM_S: f64 = 8.0;
const DIM_OVER_S: f64 = 2.0;
/// Blank cells between the title's text and a title-row readout's text (CORE-R76): with only
/// the two pad cells between them, two grey runs read as one sentence.
const READOUT_GAP: usize = 4;

/// Which part of a title is shown (CORE-R72, CORE-R73).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// The phenomenon alone: the short form.
    Phenomenon,
    /// `phenomenon · mechanism` (the phenomenon alone when the mechanism is empty).
    Full,
}

/// A scene's live title: `Meta.title` split at [`SEPARATOR`], with the mechanism replaced
/// by the live caption when there is one (CORE-R71).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Title {
    pub phenomenon: &'static str,
    /// Empty when the title has no separator and there is no caption.
    pub mechanism: &'static str,
}

impl Title {
    pub fn new(title: &'static str, caption: Option<&'static str>) -> Self {
        let (phenomenon, mechanism) = title.split_once(SEPARATOR).unwrap_or((title, ""));
        Self {
            phenomenon,
            mechanism: caption.unwrap_or(mechanism),
        }
    }

    /// The live title of `g`: its `Meta.title` with its current caption (CORE-R71).
    pub fn of(g: &(impl Generator + ?Sized)) -> Self {
        Self::new(g.meta().title, g.caption())
    }

    /// The pieces of `form` in reading order: phenomenon, separator, mechanism (the last
    /// two empty for the short form or an empty mechanism).
    fn parts(&self, form: Form) -> [&'static str; 3] {
        match form {
            Form::Full if !self.mechanism.is_empty() => {
                [self.phenomenon, SEPARATOR, self.mechanism]
            }
            _ => [self.phenomenon, "", ""],
        }
    }

    /// Display width of `form` in cells (CORE-R72).
    pub fn width(&self, form: Form) -> usize {
        self.parts(form).iter().map(|s| s.width()).sum()
    }

    /// The chars of `form` in reading order, for callers that write it themselves
    /// (the HUD maps them through [`ascii_of`]).
    pub fn chars(&self, form: Form) -> impl Iterator<Item = char> + use<> {
        let [a, b, c] = self.parts(form);
        a.chars().chain(b.chars()).chain(c.chars())
    }

    /// The longest form that fits a canvas of `size`, or `None` (CORE-R73). A form fits
    /// when it plus its gutter and pad cells take at most 3/5 of the width, so the title
    /// never dominates the art; forms are shown whole, never clipped.
    pub fn fit(&self, size: Size) -> Option<Form> {
        if size.height() < MIN_HEIGHT {
            return None;
        }
        let limit = usize::from(size.width()) * 3 / 5;
        [Form::Full, Form::Phenomenon]
            .into_iter()
            .find(|&form| self.width(form) + 2 <= limit)
    }
}

/// The title charset and its one-cell ASCII fallback (CORE-R72): printable ASCII maps to
/// itself, the separator dot and en dash to `-`, and the few accented vowels titles need
/// to their plain vowel. Anything else is `None`, i.e. not allowed in a title (GEN-R12).
pub const fn ascii_of(c: char) -> Option<char> {
    match c {
        ' '..='~' => Some(c),
        '·' | '–' => Some('-'),
        'á' => Some('a'),
        'é' => Some('e'),
        'ö' => Some('o'),
        _ => None,
    }
}

/// The row and columns [`draw`] writes for `title` on a canvas of `size` (gutter, text and
/// pad), or `None` when the title is hidden (CORE-R74). Scenes keep their text out of it
/// (GEN-R13).
pub fn span(size: Size, title: &Title) -> Option<(i32, Range<i32>)> {
    let form = title.fit(size)?;
    let end = i32::try_from(title.width(form) + 2).unwrap_or(i32::MAX);
    Some((i32::from(size.height()) - 1, 0..end))
}

/// The most chars a [`draw::label_right`](crate::draw::label_right) readout on the title
/// row can hold while keeping [`READOUT_GAP`] blank cells from the title's text (GEN-R13),
/// and at most half the row, so on narrow canvases the row stays art with a name and a
/// number rather than one line of text. Scenes drop or shorten parts to fit.
pub fn readout_room(size: Size, title: &Title) -> usize {
    let w = usize::from(size.width());
    // Cells left of the readout's text: the gutter and title text, then the gap; or only
    // the readout's leading blank when there is no title.
    let before = span(size, title).map_or(1, |(_, cols)| cols.len() - 1 + READOUT_GAP);
    // One more for the readout's trailing blank in the last column.
    w.saturating_sub(before + 1).min(w / 2)
}

/// Draw the fitted title on the bottom row, laid out like `draw::label(canvas, 0, h − 1, …)`,
/// dimming with its `age`, the scene seconds it has been on show (CORE-R74). Touches only
/// the [`span`] cells, so the art keeps the rest of the row.
pub fn draw(canvas: &mut Canvas, title: &Title, age: f64) {
    let Some(form) = title.fit(canvas.size()) else {
        return;
    };
    let age = if age.is_finite() { age } else { 0.0 };
    let fade = ((age - DIM_FROM_S) / DIM_OVER_S).clamp(0.0, 1.0);
    let ink_at = |base: f64| Ink::new(Role::Text, ink::level(base + (DIM_LEVEL - base) * fade));
    let (phenomenon, rest) = (ink_at(PHENOMENON_LEVEL), ink_at(MECHANISM_LEVEL));

    let y = i32::from(canvas.size().height()) - 1;
    let gap = Glyph::ascii(' ');
    canvas.put(0, y, gap, Ink::BLANK);
    let mut x = 1;
    for (part, ink) in title.parts(form).into_iter().zip([phenomenon, rest, rest]) {
        for c in part.chars() {
            // `?` only for chars outside the charset, which GEN-R12 keeps out of titles.
            canvas.put(x, y, Glyph::new(c, ascii_of(c).unwrap_or('?')), ink);
            x += 1;
        }
    }
    canvas.put(x, y, gap, Ink::BLANK);
}
