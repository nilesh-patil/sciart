//! The changed-cell pipeline (RND-R22, ADR-0030): paint, diff and encode only the canvas
//! cells that changed since the last drawn frame.
//!
//! `ratatui::Terminal::draw` paints a whole frame into a cleared buffer and diffs every
//! cell of it against the previous one, so a frame costs time in proportion to the screen
//! (about 20 ns a cell at M25), however little changed. [`Compositor`] keeps the buffer the
//! terminal shows and a copy of the canvas it was painted from. A frame compares the new
//! canvas with that copy (a cheap word compare per cell, a row at a time), paints only the
//! cells that differ into the shown buffer, and hands the backend exactly the cells whose
//! terminal cell changed, in row-major order. For one-column cells that is the set and the
//! order ratatui's diff yields, so the bytes on the wire are the same.
//!
//! The full-area path is kept as the reference: the first frame, a resize, a new theme,
//! painter epoch (colour memory reset, background) or overlay, a caller's
//! [`Compositor::invalidate`] (title or help toggles), a canvas that does not fill the
//! screen, and any changed cell whose glyph is not one column wide all draw the way
//! `Terminal::draw` does: paint everything into a reset buffer and diff it in full.
//!
//! The row under an overlay (the HUD or the RT-R4 error line) is painted and compared in
//! full every frame, since the overlay reverses whatever canvas cells it does not cover.

use ratatui::backend::{Backend, ClearType};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use sciart_core::{Canvas, Cell as CanvasCell};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{Painter, Theme};

/// Draws into a buffer covering (part of) the screen area it is given.
pub type Draw<'a> = &'a mut dyn FnMut(Rect, &mut Buffer);

/// What one frame shows.
pub enum Content<'a> {
    /// A canvas through `painter`; `overlay`, when given, draws on the last row over it
    /// (the HUD, or the RT-R4 error line) and must touch no other row.
    Canvas {
        canvas: &'a Canvas,
        theme: Theme,
        painter: &'a mut Painter,
        overlay: Option<Draw<'a>>,
    },
    /// Anything else (the help page), drawn into a reset buffer and diffed in full.
    Full(Draw<'a>),
}

impl std::fmt::Debug for Content<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Canvas { theme, overlay, .. } => f
                .debug_struct("Canvas")
                .field("theme", theme)
                .field("overlay", &overlay.is_some())
                .finish_non_exhaustive(),
            Content::Full(_) => f.write_str("Full"),
        }
    }
}

/// The work one frame did (RND-R22), for tests and the cost probe.
///
/// `painted` and `encoded` are counted where each stage visits a cell, not derived from
/// `changed`: the painter counts every canvas cell it maps ([`Painter::mapped`]), the
/// encoder every cell it considers sending. On the changed-cell path each is at most
/// `changed + rows`; a stage that scans the screen again breaks that bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Work {
    /// The frame took the full-area path.
    pub full: bool,
    /// Screen rows of the frame (the per-row bookkeeping the bound allows).
    pub rows: usize,
    /// Cells that changed: the canvas cells that differ from the last painted canvas,
    /// plus the overlay row, which is repainted under the overlay every frame. The whole
    /// area on the full-area path.
    pub changed: usize,
    /// Canvas cells the painter mapped to terminal cells this frame.
    pub painted: usize,
    /// Cells the encoder considered sending: the whole area on the full-area path, whose
    /// diff compares every cell.
    pub encoded: usize,
    /// Cells handed to the backend.
    pub sent: usize,
}

/// What [`Compositor::compose`] left for [`Compositor::emit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Nothing,
    /// Diff `next` against `screen` in full, after the clears a resize queued.
    Full {
        clears: u8,
    },
    /// Send the `dirty` cells of `screen`.
    Changed,
}

/// The terminal's picture, kept across frames so only changed cells are painted, diffed
/// and encoded (RND-R22).
#[derive(Debug)]
pub struct Compositor {
    /// The area the terminal had at the last frame (`Terminal`'s `last_known_area`);
    /// `None` before the first frame.
    area: Option<Rect>,
    /// What the terminal shows.
    screen: Buffer,
    /// The full-area path's frame; also the scratch row under an overlay.
    next: Buffer,
    /// The canvas cells `screen` was painted from, valid when `valid`.
    last: Vec<CanvasCell>,
    /// Canvas indices that differ from `last` (scratch, capacity kept).
    changed: Vec<u32>,
    /// Buffer indices to send, row-major (scratch, capacity kept).
    dirty: Vec<u32>,
    valid: bool,
    theme: Option<Theme>,
    painter_epoch: u64,
    overlay: bool,
    pending: Pending,
    work: Work,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            area: None,
            screen: Buffer::empty(Rect::ZERO),
            next: Buffer::empty(Rect::ZERO),
            last: Vec::new(),
            changed: Vec::new(),
            dirty: Vec::new(),
            valid: false,
            theme: None,
            painter_epoch: 0,
            overlay: false,
            pending: Pending::Nothing,
            work: Work::default(),
        }
    }

    /// The next frame takes the full-area path (the caller changed something the
    /// compositor cannot see, such as the title or the help page).
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// The last frame's work.
    pub fn work(&self) -> Work {
        self.work
    }

    /// What the terminal shows after the last emitted frame.
    pub fn screen(&self) -> &Buffer {
        &self.screen
    }

    /// Paint the frame for a terminal of `area` (its size, at the origin). Writes nothing:
    /// [`Compositor::emit`] sends it.
    pub fn compose(&mut self, area: Rect, content: Content<'_>) {
        let clears = self.track_area(area);
        self.work = Work::default();
        match content {
            Content::Full(draw) => {
                self.full(area, clears, |buf| draw(area, buf));
                // The next canvas frame must repaint what the page covered.
                self.valid = false;
                self.overlay = false;
            }
            Content::Canvas {
                canvas,
                theme,
                painter,
                mut overlay,
            } => {
                let mapped = painter.mapped();
                let fits =
                    canvas.size().width() == area.width && canvas.size().height() == area.height;
                let overlay_on = overlay.is_some();
                let same = self.valid
                    && clears == 0
                    && fits
                    && self.theme == Some(theme)
                    && self.painter_epoch == painter.epoch()
                    && self.overlay == overlay_on;
                if !(same && self.changed_cells(area, canvas, theme, painter, &mut overlay)) {
                    self.full(area, clears, |buf| {
                        painter.paint(canvas, theme, area, buf);
                        if let Some(draw) = overlay.as_mut() {
                            draw(area, buf);
                        }
                    });
                    // A wide glyph anywhere makes ratatui's diff look at its neighbours:
                    // such a canvas stays on the full-area path.
                    self.valid = fits && canvas.cells().iter().all(|c| c.ch.width() == Some(1));
                    if self.valid {
                        self.last.clear();
                        self.last.extend_from_slice(canvas.cells());
                    }
                }
                self.work.painted = (painter.mapped() - mapped) as usize;
                self.theme = Some(theme);
                self.painter_epoch = painter.epoch();
                self.overlay = overlay_on;
            }
        }
    }

    /// `Terminal::autoresize`: a new area clears the screen (twice when it got narrower,
    /// as ratatui does) and blanks the shown buffer. Returns the clears to queue.
    fn track_area(&mut self, area: Rect) -> u8 {
        let Some(old) = self.area else {
            // `Terminal::new`: buffers of the first size, no clear.
            self.area = Some(area);
            self.screen.resize(area);
            self.screen.reset();
            self.reserve(area);
            return 0;
        };
        if old == area {
            return 0;
        }
        let clears = if area.width < old.width { 2 } else { 1 };
        self.area = Some(area);
        self.screen.resize(area);
        self.screen.reset();
        self.reserve(area);
        self.valid = false;
        clears
    }

    /// Size the scratch lists once per area, so steady frames never allocate (RT-R14).
    fn reserve(&mut self, area: Rect) {
        let cells = area.area() as usize;
        for list in [&mut self.changed, &mut self.dirty] {
            if list.capacity() < cells {
                list.reserve_exact(cells - list.len());
            }
        }
        if self.last.capacity() < cells {
            self.last.reserve_exact(cells - self.last.len());
        }
    }

    /// The reference path: everything into a reset buffer, diffed in full on emit.
    fn full(&mut self, area: Rect, clears: u8, draw: impl FnOnce(&mut Buffer)) {
        if self.next.area != area {
            self.next.resize(area);
        }
        self.next.reset();
        draw(&mut self.next);
        self.pending = Pending::Full { clears };
        let cells = area.area() as usize;
        // `painted` is the painter's count, which `compose` fills in; `encoded` and `sent`
        // are filled in by `emit`.
        self.work = Work {
            full: true,
            rows: usize::from(area.height),
            changed: cells,
            ..Work::default()
        };
    }

    /// The changed-cell path; `false` (with nothing touched) when this frame must take the
    /// full-area path instead.
    fn changed_cells(
        &mut self,
        area: Rect,
        canvas: &Canvas,
        theme: Theme,
        painter: &mut Painter,
        overlay: &mut Option<Draw<'_>>,
    ) -> bool {
        let width = usize::from(area.width);
        let height = usize::from(area.height);
        let rows = if overlay.is_some() {
            height.saturating_sub(1)
        } else {
            height
        };
        let cells = canvas.cells();
        self.changed.clear();
        for y in 0..rows {
            let span = y * width..(y + 1) * width;
            let (new, old) = (&cells[span.clone()], &self.last[span]);
            if new == old {
                continue;
            }
            for (x, (n, o)) in new.iter().zip(old).enumerate() {
                if n != o {
                    // ratatui's diff treats a wide glyph's neighbours specially; only
                    // one-column glyphs keep the two paths identical.
                    if n.ch.width() != Some(1) || o.ch.width() != Some(1) {
                        return false;
                    }
                    self.changed.push((y * width + x) as u32);
                }
            }
        }
        // The overlay row: the canvas row painted in full, the overlay over it.
        let overlay_row = if let Some(draw) = overlay.as_mut() {
            let y = area.bottom() - 1;
            let row = Rect::new(area.x, y, area.width, 1);
            if self.next.area != row {
                self.next.resize(row);
            }
            self.next.reset();
            painter.paint_row(canvas, theme, y, &mut self.next);
            draw(area, &mut self.next);
            let start = usize::from(y) * width;
            let shown = &self.screen.content[start..start + width];
            if self
                .next
                .content
                .iter()
                .chain(shown)
                .any(|c| c.symbol().width() != 1)
            {
                return false;
            }
            Some(y)
        } else {
            None
        };
        self.dirty.clear();
        painter.paint_cells(
            canvas,
            theme,
            &self.changed,
            &mut self.screen,
            &mut self.dirty,
        );
        for &i in &self.changed {
            let i = i as usize;
            self.last[i] = cells[i];
        }
        let mut changed = self.changed.len();
        if let Some(y) = overlay_row {
            let start = usize::from(y) * width;
            let shown = &mut self.screen.content[start..start + width];
            for (x, (new, old)) in self.next.content.iter().zip(shown).enumerate() {
                if new != old {
                    *old = new.clone();
                    self.dirty.push((start + x) as u32);
                }
            }
            changed += width;
        }
        self.pending = Pending::Changed;
        // `painted` is the painter's count, which `compose` fills in; `encoded` and `sent`
        // are filled in by `emit`.
        self.work = Work {
            full: false,
            rows: height,
            changed,
            ..Work::default()
        };
        true
    }

    /// Send the composed frame the way `Terminal::draw` does: the queued clears, the
    /// changed cells, hide the cursor, flush (one write, RT-R15).
    pub fn emit<B: Backend>(&mut self, backend: &mut B) -> Result<(), B::Error> {
        match std::mem::replace(&mut self.pending, Pending::Nothing) {
            Pending::Nothing => {}
            Pending::Full { clears } => {
                for _ in 0..clears {
                    backend.clear_region(ClearType::All)?;
                }
                let mut sent = 0;
                backend.draw(self.screen.diff_iter(&self.next).inspect(|_| sent += 1))?;
                // ratatui's diff compares every cell of the area.
                self.work.encoded = self.next.content.len();
                self.work.sent = sent;
                backend.hide_cursor()?;
                backend.flush()?;
                std::mem::swap(&mut self.screen, &mut self.next);
            }
            Pending::Changed => {
                let width = usize::from(self.screen.area.width.max(1));
                let (x0, y0) = (self.screen.area.x, self.screen.area.y);
                let screen = &self.screen;
                let (mut encoded, mut sent) = (0, 0);
                backend.draw(
                    self.dirty
                        .iter()
                        .map(|&i| {
                            // RND-R22's encoder counter: every cell considered, sent or not.
                            encoded += 1;
                            let i = i as usize;
                            let (x, y) = ((i % width) as u16, (i / width) as u16);
                            (x0 + x, y0 + y, &screen.content[i])
                        })
                        .inspect(|_| sent += 1),
                )?;
                self.work.encoded = encoded;
                self.work.sent = sent;
                backend.hide_cursor()?;
                backend.flush()?;
            }
        }
        Ok(())
    }

    /// [`Compositor::compose`] then [`Compositor::emit`], with the area from the backend.
    pub fn draw<B: Backend>(
        &mut self,
        backend: &mut B,
        content: Content<'_>,
    ) -> Result<(), B::Error> {
        let size = backend.size()?;
        self.compose(Rect::new(0, 0, size.width, size.height), content);
        self.emit(backend)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::style::Style;
    use sciart_core::{Glyph, GlyphMode, Ink, Role, Size};

    use super::*;
    use crate::FrameBackend;

    /// One frame of a script: the canvas, an optional reversed message on the last row.
    struct Step {
        canvas: Canvas,
        message: Option<&'static str>,
    }

    fn message_line(msg: &str) -> impl FnMut(Rect, &mut Buffer) + '_ {
        move |area: Rect, buf: &mut Buffer| {
            let row = Rect::new(area.x, area.bottom() - 1, area.width, 1).intersection(buf.area);
            buf.set_style(row, Style::new().reversed());
            buf.set_stringn(row.x, row.y, msg, usize::from(row.width), Style::new());
        }
    }

    /// The bytes `Terminal::draw` and the compositor send for the same frames.
    fn streams(steps: &[Step]) -> (Vec<u8>, Vec<u8>, Vec<Work>) {
        let size = steps[0].canvas.size();
        let backend = || FrameBackend::with_size(Vec::new(), size.width(), size.height());
        let mut reference = Terminal::new(backend()).expect("terminal");
        let mut new = backend();
        let (mut p_ref, mut p_new) = (Painter::new(), Painter::new());
        let mut compositor = Compositor::new();
        let mut work = Vec::new();
        for step in steps {
            let s = step.canvas.size();
            reference
                .backend_mut()
                .set_fixed_size(s.width(), s.height());
            new.set_fixed_size(s.width(), s.height());
            reference
                .draw(|f| {
                    let area = f.area();
                    p_ref.paint(&step.canvas, Theme::Astro, area, f.buffer_mut());
                    if let Some(msg) = step.message {
                        message_line(msg)(area, f.buffer_mut());
                    }
                })
                .expect("draw");
            let mut line = step.message.map(message_line);
            let overlay = line
                .as_mut()
                .map(|l| l as &mut dyn FnMut(Rect, &mut Buffer));
            compositor
                .draw(
                    &mut new,
                    Content::Canvas {
                        canvas: &step.canvas,
                        theme: Theme::Astro,
                        painter: &mut p_new,
                        overlay,
                    },
                )
                .expect("draw");
            work.push(compositor.work());
        }
        (
            reference.backend().writer().clone(),
            new.writer().clone(),
            work,
        )
    }

    fn canvas(size: Size, marks: &[(i32, i32, char)]) -> Canvas {
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        for &(x, y, ch) in marks {
            c.text(x, y, &ch.to_string(), Ink::new(Role::Star, 200));
        }
        c
    }

    #[test]
    fn rnd_r22_wide_glyphs_keep_the_full_area_path_and_its_bytes() {
        let size = Size::new(12, 4);
        let steps = [
            Step {
                canvas: canvas(size, &[(1, 1, 'a')]),
                message: None,
            },
            Step {
                canvas: canvas(size, &[(1, 1, 'b')]),
                message: None,
            },
            // `Canvas::text` writes a wide glyph as it is: ratatui then skips its neighbour.
            Step {
                canvas: canvas(size, &[(1, 1, '世'), (2, 1, 'x')]),
                message: None,
            },
            Step {
                canvas: canvas(size, &[(1, 1, 'c'), (2, 1, 'y')]),
                message: None,
            },
            Step {
                canvas: canvas(size, &[(1, 1, 'd'), (2, 1, 'y')]),
                message: None,
            },
        ];
        let (reference, new, work) = streams(&steps);
        assert_eq!(new, reference);
        assert!(!work[1].full, "a narrow change is a changed-cell frame");
        assert!(work[2].full && work[3].full, "{work:?}");
        assert!(
            !work[4].full,
            "back on the changed-cell path once the screen is narrow"
        );
    }

    #[test]
    fn rnd_r22_overlay_row_and_narrowing_resize_match_the_full_area_bytes() {
        let (big, small) = (Size::new(16, 5), Size::new(10, 4));
        let mut steps = Vec::new();
        for (i, msg) in [
            None,
            Some("error: x"),
            Some("error: y"),
            Some("err 世界"),
            Some("ok"),
        ]
        .into_iter()
        .enumerate()
        {
            steps.push(Step {
                canvas: canvas(big, &[(i as i32, 4, '*'), (3, 2, '+')]),
                message: msg,
            });
        }
        steps.push(Step {
            canvas: canvas(small, &[(1, 1, 'q')]),
            message: None,
        });
        steps.push(Step {
            canvas: canvas(small, &[(2, 1, 'q')]),
            message: None,
        });
        steps.push(Step {
            canvas: canvas(big, &[(2, 1, 'q')]),
            message: None,
        });
        let (reference, new, work) = streams(&steps);
        assert_eq!(new, reference);
        assert!(
            !work[2].full,
            "a steady overlay is a changed-cell frame: {work:?}"
        );
        assert!(work[3].full, "a wide glyph on the overlay row: {work:?}");
        assert!(work[5].full && work[7].full, "resizes: {work:?}");
        assert!(!work[6].full, "{work:?}");
    }

    #[test]
    fn rnd_r22_an_unchanged_frame_paints_and_sends_nothing() {
        let size = Size::new(20, 6);
        let mut c = canvas(size, &[(3, 3, '*')]);
        c.put_bg(
            5,
            5,
            Glyph::new('▀', '#'),
            Ink::new(Role::Seq, 9),
            Ink::new(Role::Div, 200),
        );
        let steps = [
            Step {
                canvas: c.clone(),
                message: None,
            },
            Step {
                canvas: c.clone(),
                message: None,
            },
            Step {
                canvas: c,
                message: Some("hud"),
            },
        ];
        let (reference, new, work) = streams(&steps);
        assert_eq!(new, reference);
        let w = work[1];
        assert_eq!((w.changed, w.painted, w.encoded, w.sent), (0, 0, 0, 0));
        assert!(!w.full);
        // With an overlay, the row under it is repainted in full and nothing else.
        let w = work[2];
        assert!(w.full, "the overlay appears: {w:?}");
    }

    #[test]
    fn rnd_r22_counters_bound_the_painter_and_the_encoder_with_an_overlay() {
        let size = Size::new(20, 6);
        let steps: Vec<Step> = (0..4)
            .map(|i| Step {
                canvas: canvas(size, &[(i, 2, '*'), (5, 5, '+')]),
                message: Some("hud"),
            })
            .collect();
        let (reference, new, work) = streams(&steps);
        assert_eq!(new, reference);
        for w in &work[1..] {
            assert!(!w.full, "{w:?}");
            // Two canvas cells move a column per frame; the overlay row is 20 cells.
            assert_eq!(w.changed, 2 + 20, "{w:?}");
            assert!(w.painted <= w.changed + w.rows, "{w:?}");
            assert!(w.encoded <= w.changed + w.rows, "{w:?}");
            assert!(w.sent <= w.encoded, "{w:?}");
        }
    }
}
