use unicode_width::UnicodeWidthChar;

use crate::{Ink, Size};

/// A glyph with a pure-ASCII fallback (GEN-R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub unicode: char,
    pub ascii: char,
}

impl Glyph {
    pub const fn new(unicode: char, ascii: char) -> Self {
        Self { unicode, ascii }
    }

    /// A glyph that is already ASCII in both modes.
    pub const fn ascii(ch: char) -> Self {
        Self::new(ch, ch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphMode {
    #[default]
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    /// Foreground ink of `ch`.
    pub ink: Ink,
    /// Background ink; [`Ink::BLANK`] means "no background" (terminal default). Only
    /// half-block pixels set it (CORE-R40, ADR-0010).
    pub bg: Ink,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        ink: Ink::BLANK,
        bg: Ink::BLANK,
    };
}

/// A row-major grid of cells that generators draw into.
#[derive(Debug, Clone)]
pub struct Canvas {
    size: Size,
    mode: GlyphMode,
    cells: Vec<Cell>,
}

impl Canvas {
    pub fn new(size: Size, mode: GlyphMode) -> Self {
        Self {
            size,
            mode,
            cells: vec![Cell::BLANK; size.area()],
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn mode(&self) -> GlyphMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: GlyphMode) {
        self.mode = mode;
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Reallocate only when the size actually changes; always leaves a blank canvas (CORE-R5).
    pub fn resize(&mut self, size: Size) {
        if size != self.size {
            self.size = size;
            self.cells = vec![Cell::BLANK; size.area()];
        } else {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::BLANK);
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        ((0..w).contains(&x) && (0..h).contains(&y))
            .then(|| y as usize * usize::from(self.size.width()) + x as usize)
    }

    pub fn get(&self, x: i32, y: i32) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// Draw one glyph; out-of-bounds coordinates are ignored (CORE-R3, CORE-R4).
    ///
    /// Clears the cell's background, so overdrawing a half-block pixel never leaves a
    /// stale background colour behind (CORE-R40).
    pub fn put(&mut self, x: i32, y: i32, glyph: Glyph, ink: Ink) {
        self.put_bg(x, y, glyph, ink, Ink::BLANK);
    }

    /// Draw one glyph with a foreground and a background ink, under the same bounds
    /// and glyph-mode rules as [`put`](Self::put) (CORE-R40).
    pub fn put_bg(&mut self, x: i32, y: i32, glyph: Glyph, fg: Ink, bg: Ink) {
        let Some(i) = self.index(x, y) else { return };
        let ch = match self.mode {
            GlyphMode::Ascii => glyph.ascii,
            GlyphMode::Unicode if glyph.unicode.width() == Some(1) => glyph.unicode,
            GlyphMode::Unicode => glyph.ascii,
        };
        self.cells[i] = Cell { ch, ink: fg, bg };
    }

    /// Draw a string left-to-right; each char is its own ASCII fallback.
    pub fn text(&mut self, x: i32, y: i32, s: &str, ink: Ink) {
        for (i, ch) in s.chars().enumerate() {
            self.put(x.saturating_add(i as i32), y, Glyph::ascii(ch), ink);
        }
    }

    /// Exactly `height` lines of exactly `width` chars (CORE-R6).
    pub fn to_lines(&self) -> Vec<String> {
        self.cells
            .chunks(usize::from(self.size.width()))
            .map(|row| row.iter().map(|c| c.ch).collect())
            .collect()
    }
}
