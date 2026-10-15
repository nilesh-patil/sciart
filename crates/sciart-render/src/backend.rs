//! A ratatui backend that sends each frame as one write (RT-R15) in as few bytes as the
//! diff allows (RND-R15, RND-R17).
//!
//! `ratatui`'s `CrosstermBackend` over `stdout()` goes through std's `LineWriter`, which
//! hands the kernel 1 KiB at a time: a 100 KB field frame became ~100 `write` calls, and a
//! terminal that repaints on each read could show a torn frame. It also re-sends the
//! foreground *and* background whenever either changes, moves the cursor with an absolute
//! `CUP` for every gap, and ends even an empty frame with three reset sequences.
//!
//! [`FrameBackend`] appends everything to one reused `Vec<u8>` and hands it to the writer
//! in a single `write_all` + `flush` from [`Backend::flush`], which `Terminal::draw` calls
//! once per frame. An empty diff writes nothing and does not flush (RT-R17). A non-empty
//! frame is wrapped in a synchronized update (DEC private mode 2026) so terminals that
//! support it swap the whole frame at once; others ignore the unknown mode. The cells
//! themselves go through [`crate::encode`]: only the SGR parameters that changed, the
//! shortest cursor move, and the cheaper of row-major and style-grouped order (RND-R17).

use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use unicode_width::UnicodeWidthStr;

use crate::encode::{self, Cursor, Pen, Pending, Plan, State};

const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
/// Reset the newline mode (LNM), so VT moves straight down (RND-R17); sent once.
const RESET_LNM: &[u8] = b"\x1b[20l";

/// A ratatui [`Backend`] that buffers a whole frame and writes it once (RT-R15).
#[derive(Debug)]
pub struct FrameBackend<W: Write> {
    out: W,
    /// The pending frame; its capacity is kept, so steady-state frames never allocate.
    buf: Vec<u8>,
    /// The frame in the other cell order (RND-R17), swapped with `buf` when shorter; the
    /// same capacity as `buf`.
    alt: Vec<u8>,
    /// The diff's cells in row-major order, copied so they can be sent in another order
    /// (RND-R17); capacity kept.
    cells: Vec<Pending>,
    /// Symbols of `cells` longer than 4 bytes (only in free text; canvas and HUD cells are
    /// one `char`, kept inline). Not reserved: a frame with such a grapheme may grow it once.
    syms: Vec<u8>,
    /// Scratch for the style-grouped order: indices into `cells`; capacity kept.
    order: Vec<u32>,
    /// Which cell order the last probe frame found shorter (RND-R17).
    choice: encode::Choice,
    /// Fixed size for tests and the cost probe; `None` asks the terminal.
    size: Option<Size>,
    /// Wrap frames in a synchronized update (DEC mode 2026).
    sync: bool,
    /// A `BEGIN_SYNC` was written and its `END_SYNC` is still due.
    in_sync: bool,
    /// `RESET_LNM` was sent.
    lnm_reset: bool,
    cursor_hidden: bool,
    /// The terminal's current rendition. It carries over from frame to frame, so a frame
    /// that starts in the colour the last one ended in sends no SGR.
    pen: Pen,
    /// The terminal size as last seen by `size()` (which `Terminal::draw` calls first
    /// every frame): what [`FrameBackend::reserve_frame`] sizes the buffers for.
    seen: std::cell::Cell<Size>,
    /// First and last row the last `draw` changed; `None` for an empty diff (RT-R17).
    rows: Option<(u16, u16)>,
}

/// Upper bound on the bytes one changed cell can add to a frame (RT-R14, RT-R15):
/// cursor move `CSI 65535;65535H` (14; a relative move is only chosen when shorter) +
/// one SGR with a modifier reset, all nine modifier codes and both 24-bit colours
/// `CSI 0;1;…;9;38;2;255;255;255;48;2;255;255;255m` (56) + the symbol, 4 bytes for any
/// one `char` (canvas cells and the ASCII HUD). A longer grapheme (only possible in free
/// text) may still grow the buffer once.
pub const MAX_CELL_BYTES: usize = 14 + 56 + 4;
/// Per-frame bytes outside the cells: sync begin/end (16), cursor hide/show (12), a clear
/// (4), a rendition reset (4) and the one-off LNM reset (5), rounded up.
pub const FRAME_OVERHEAD_BYTES: usize = 64;

/// The most bytes a frame of `cells` cells can take ([`MAX_CELL_BYTES`] per cell).
pub const fn frame_bound(cells: usize) -> usize {
    cells * MAX_CELL_BYTES + FRAME_OVERHEAD_BYTES
}

impl<W: Write> FrameBackend<W> {
    /// A backend on a real terminal: its size is queried, frames are synchronized.
    pub fn new(out: W) -> Self {
        Self {
            out,
            buf: Vec::new(),
            alt: Vec::new(),
            cells: Vec::new(),
            syms: Vec::new(),
            order: Vec::new(),
            choice: encode::Choice::default(),
            size: None,
            sync: true,
            in_sync: false,
            lnm_reset: false,
            cursor_hidden: false,
            pen: Pen::DEFAULT,
            seen: std::cell::Cell::new(Size {
                width: 0,
                height: 0,
            }),
            rows: None,
        }
    }

    /// A backend of a fixed size (tests, the RND-R13 probe): no terminal is queried.
    pub fn with_size(out: W, width: u16, height: u16) -> Self {
        Self {
            size: Some(Size { width, height }),
            ..Self::new(out)
        }
    }

    /// Change a fixed-size backend's size, as a terminal resize would (tests, the RND-R13
    /// probe); like `ratatui::Terminal::resize`, follow it with a clear and a full redraw.
    pub fn set_fixed_size(&mut self, width: u16, height: u16) {
        self.size = Some(Size { width, height });
    }

    /// Switch the DEC 2026 synchronized-update wrapper on or off.
    pub fn with_sync(mut self, on: bool) -> Self {
        self.sync = on;
        self
    }

    pub fn writer(&self) -> &W {
        &self.out
    }

    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.out
    }

    /// The first and last row the last frame changed, or `None` when its diff was empty
    /// (RT-R17: the app steps its draw rate down on a run of those).
    pub fn changed_rows(&self) -> Option<(u16, u16)> {
        self.rows
    }

    /// Bytes queued for the next flush.
    pub fn pending(&self) -> &[u8] {
        &self.buf
    }

    fn current_size(&self) -> Size {
        self.size.unwrap_or_else(|| self.seen.get())
    }

    /// Grow the buffers, once per terminal size, to hold the worst-case frame, so no draw
    /// can reallocate mid-run however much of the screen changes (RT-R14).
    fn reserve_frame(&mut self) {
        let s = self.current_size();
        let cells = usize::from(s.width) * usize::from(s.height);
        let bound = frame_bound(cells);
        for buf in [&mut self.buf, &mut self.alt] {
            if buf.capacity() < bound {
                buf.reserve_exact(bound - buf.len());
            }
        }
        if self.cells.capacity() < cells {
            self.cells.reserve_exact(cells - self.cells.len());
        }
        if self.order.capacity() < cells {
            self.order.reserve_exact(cells - self.order.len());
        }
    }

    /// Bytes the frame buffer can hold without growing.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    fn begin_frame(&mut self) {
        if self.sync && !self.in_sync {
            self.buf.extend_from_slice(BEGIN_SYNC);
            self.in_sync = true;
        }
        if !self.lnm_reset {
            self.buf.extend_from_slice(RESET_LNM);
            self.lnm_reset = true;
        }
    }

    /// Queue an SGR reset when the rendition is not the default; `ED`/`EL` erase with the
    /// current background, and the shell after exit expects the default.
    fn reset_pen(&mut self) {
        if self.pen != Pen::DEFAULT {
            self.buf.extend_from_slice(b"\x1b[0m");
            self.pen = Pen::DEFAULT;
        }
    }

    /// Put the terminal back in the default rendition and write out everything pending:
    /// call before leaving the alternate screen.
    pub fn finish(&mut self) -> io::Result<()> {
        self.reset_pen();
        Backend::flush(self)
    }

    fn write_now(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buf.extend_from_slice(bytes);
        Backend::flush(self)
    }
}

impl<W: Write> Backend for FrameBackend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.reserve_frame();
        self.cells.clear();
        self.syms.clear();
        // RND-R17: the order is chosen before the diff is read (see `encode::Choice`), so a
        // row-major frame is encoded straight from the diff, and only grouped and probe
        // frames copy the cells out.
        let plan = self.choice.plan();
        let (inline, collect) = match plan {
            Plan::RowMajor => (true, false),
            Plan::Grouped => (false, true),
            Plan::Probe => (true, true),
        };
        let start = State {
            pen: self.pen,
            // Not carried across frames: something else may have moved it in between.
            cursor: Cursor::Unknown,
            width: self.current_size().width,
        };
        let mut state = start;
        // Everything queued before the first cell (a clear, the sync opener) leads either
        // cell order; `None` while the diff is empty.
        let mut prefix = None;
        let mut rows: Option<(u16, u16)> = None;
        let mut n = 0;
        for (x, y, cell) in content {
            if prefix.is_none() {
                self.begin_frame();
                prefix = Some(self.buf.len());
            }
            let symbol = match cell.symbol() {
                // An empty symbol would print nothing and leave the cursor behind.
                "" => " ",
                s => s,
            };
            // A width-0 symbol still occupies its cell once printed as a space would.
            let width = u16::try_from(symbol.width().max(1)).unwrap_or(1);
            let pen = Pen {
                fg: cell.fg,
                bg: cell.bg,
                modifier: cell.modifier,
            };
            if inline {
                encode::put(
                    &mut self.buf,
                    &mut state,
                    (x, y, width),
                    pen,
                    symbol.as_bytes(),
                );
            }
            if collect {
                let p = Pending::new((x, y, width), pen, symbol.as_bytes(), &mut self.syms);
                self.cells.push(p);
            }
            // The diff is row-major: the first cell has the first row, the last the last.
            rows = Some((rows.map_or(y, |r| r.0), y));
            n += 1;
        }
        self.rows = rows;
        let Some(prefix) = prefix else {
            return Ok(());
        };
        match plan {
            Plan::RowMajor => self.choice.tick(n),
            Plan::Grouped => {
                encode::encode_grouped(
                    &mut self.buf,
                    &mut state,
                    &mut self.cells,
                    &mut self.order,
                    &self.syms,
                );
                self.choice.tick(n);
            }
            Plan::Probe => {
                let grouped = encode::probe(
                    &mut self.buf,
                    &mut self.alt,
                    prefix,
                    start,
                    &mut state,
                    &mut self.cells,
                    &mut self.order,
                    &self.syms,
                );
                self.choice.probed(grouped, n);
            }
        }
        self.pen = state.pen;
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        // `Terminal::draw` calls this every frame, before its flush: queue it once.
        if !self.cursor_hidden {
            self.buf.extend_from_slice(HIDE_CURSOR);
            self.cursor_hidden = true;
        }
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        // Also called from `Terminal`'s `Drop`, with no flush after it: write through.
        self.cursor_hidden = false;
        self.write_now(SHOW_CURSOR)
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Backend::flush(self)?;
        crossterm::cursor::position().map(|(x, y)| Position { x, y })
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let Position { x, y } = position.into();
        let mut state = State {
            pen: self.pen,
            cursor: Cursor::Unknown,
            width: 0,
        };
        encode::move_to(&mut self.buf, &mut state, x, y);
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        let seq: &[u8] = match clear_type {
            ClearType::All => b"\x1b[2J",
            ClearType::AfterCursor => b"\x1b[J",
            ClearType::BeforeCursor => b"\x1b[1J",
            ClearType::CurrentLine => b"\x1b[2K",
            ClearType::UntilNewLine => b"\x1b[K",
        };
        self.reset_pen();
        self.buf.extend_from_slice(seq);
        Ok(())
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        for _ in 0..n {
            self.buf.push(b'\n');
        }
        Backend::flush(self)
    }

    fn size(&self) -> io::Result<Size> {
        match self.size {
            Some(size) => Ok(size),
            None => {
                let (width, height) = crossterm::terminal::size()?;
                self.seen.set(Size { width, height });
                Ok(Size { width, height })
            }
        }
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        let size = self.size()?;
        let pixels = match self.size {
            Some(_) => Size {
                width: 0,
                height: 0,
            },
            None => {
                let ws = crossterm::terminal::window_size()?;
                Size {
                    width: ws.width,
                    height: ws.height,
                }
            }
        };
        Ok(WindowSize {
            columns_rows: size,
            pixels,
        })
    }

    /// The frame's one write (RT-R15): everything queued since the last flush, closed by
    /// the end of the synchronized update, in a single `write_all`, then `flush`.
    fn flush(&mut self) -> io::Result<()> {
        if self.in_sync {
            self.buf.extend_from_slice(END_SYNC);
            self.in_sync = false;
        }
        if !self.buf.is_empty() {
            let result = self.out.write_all(&self.buf);
            self.buf.clear();
            result?;
            self.out.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use ratatui::style::{Color, Modifier};

    use super::*;
    use crate::encode::push_u16;

    /// Counts `write` and `flush` calls and keeps the bytes.
    #[derive(Debug, Default)]
    struct Counting {
        bytes: Vec<u8>,
        writes: usize,
        flushes: usize,
    }

    impl Write for Counting {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    fn terminal(w: u16, h: u16) -> Terminal<FrameBackend<Counting>> {
        Terminal::new(FrameBackend::with_size(Counting::default(), w, h)).expect("terminal")
    }

    fn fill(buf: &mut Buffer, t: u8) {
        for (i, cell) in buf.content.iter_mut().enumerate() {
            let v = (i as u8).wrapping_mul(7).wrapping_add(t);
            cell.set_char('▀')
                .set_fg(Color::Rgb(v, 10, 20))
                .set_bg(Color::Rgb(20, v, 10));
        }
    }

    /// Replays `bytes` into a vt100 screen and returns it.
    fn screen(w: u16, h: u16, bytes: &[u8]) -> vt100::Parser {
        let mut p = vt100::Parser::new(h, w, 0);
        p.process(bytes);
        p
    }

    /// Scattered dots in a few colours on rows far apart: the frame needs downward moves.
    fn dots(buf: &mut Buffer, t: u8) {
        for i in 0..60u16 {
            let (x, y) = ((i * 7 + u16::from(t)) % 40, (i * 5) % 12);
            buf[(x, y)]
                .set_char('⣿')
                .set_fg(Color::Rgb(200, 10 * (i % 3) as u8, 9));
        }
    }

    #[test]
    fn rt_r15_each_frame_is_one_write_and_one_flush() {
        // Behind a `LineWriter` like `stdout()`'s: a '\n' anywhere in the frame would make
        // it write up to the last newline and then the rest separately (RT-R15).
        let backend = FrameBackend::with_size(
            io::LineWriter::with_capacity(1024, Counting::default()),
            40,
            12,
        );
        let mut term = Terminal::new(backend).expect("terminal");
        for t in 0..6 {
            let before = {
                let c = term.backend().writer().get_ref();
                (c.writes, c.flushes)
            };
            if t % 2 == 0 {
                term.draw(|f| fill(f.buffer_mut(), t)).expect("draw");
            } else {
                term.draw(|f| dots(f.buffer_mut(), t)).expect("draw");
            }
            let c = term.backend().writer().get_ref();
            assert_eq!(c.writes - before.0, 1, "frame {t}: one write");
            assert_eq!(c.flushes - before.1, 1, "frame {t}: one flush");
        }
    }

    #[test]
    fn rt_r15_frames_are_wrapped_in_a_synchronized_update() {
        let mut term = terminal(10, 3);
        term.draw(|f| fill(f.buffer_mut(), 1)).expect("draw");
        let bytes = &term.backend().writer().bytes;
        assert!(bytes.starts_with(BEGIN_SYNC));
        assert!(bytes.ends_with(END_SYNC));
    }

    #[test]
    fn rnd_r15_an_unchanged_frame_sends_nothing() {
        let mut term = terminal(10, 3);
        term.draw(|f| fill(f.buffer_mut(), 1)).expect("draw");
        let n = term.backend().writer().bytes.len();
        let writes = term.backend().writer().writes;
        term.draw(|f| fill(f.buffer_mut(), 1)).expect("draw");
        assert_eq!(term.backend().writer().bytes.len(), n);
        assert_eq!(term.backend().writer().writes, writes, "no write at all");
    }

    #[test]
    fn rnd_r15_only_the_changed_colour_is_sent() {
        let mut term = terminal(4, 1);
        let paint = |buf: &mut Buffer, bg: u8| {
            for cell in &mut buf.content {
                cell.set_char('▀')
                    .set_fg(Color::Rgb(1, 2, 3))
                    .set_bg(Color::Rgb(bg, 5, 6));
            }
        };
        term.draw(|f| paint(f.buffer_mut(), 4)).expect("draw");
        let n = term.backend().writer().bytes.len();
        term.draw(|f| paint(f.buffer_mut(), 9)).expect("draw");
        let frame = &term.backend().writer().bytes[n..];
        let text = String::from_utf8_lossy(frame);
        assert!(text.contains("48;2;9;5;6"), "{text:?}");
        assert!(
            !text.contains("38;2;"),
            "fg unchanged, not re-sent: {text:?}"
        );
    }

    /// Whatever the encoding, the terminal must end up showing exactly the buffer.
    #[test]
    fn rnd_r15_rnd_r17_rt_r15_screen_matches_the_buffer_through_vt100() {
        let (w, h) = (23u16, 7u16);
        let mut term = terminal(w, h);
        let mut expected = Buffer::empty(Rect::new(0, 0, w, h));
        for t in 0..6u8 {
            term.draw(|f| {
                let buf = f.buffer_mut();
                for y in 0..h {
                    for x in 0..w {
                        // Sparse, uneven changes: gaps, row ends, style and modifier runs.
                        let k = (u32::from(x) * 31 + u32::from(y) * 17 + u32::from(t) * 13) % 11;
                        let cell = &mut buf[(x, y)];
                        match k {
                            0 => {
                                cell.set_char('▀')
                                    .set_fg(Color::Rgb(t, 200, 3))
                                    .set_bg(Color::Rgb(9, t, 90));
                            }
                            1 => {
                                cell.set_char('x').set_fg(Color::Rgb(250, 250, t));
                            }
                            2 => {
                                cell.set_char('#')
                                    .set_style(ratatui::style::Style::new().reversed());
                            }
                            3 if t % 2 == 0 => {
                                cell.set_char('⣿').set_fg(Color::Rgb(5, 5, 5));
                            }
                            _ => {
                                cell.set_char(' ').set_bg(Color::Rgb(0, 0, t));
                            }
                        }
                    }
                }
                expected = buf.clone();
            })
            .expect("draw");
            let p = screen(w, h, &term.backend().writer().bytes);
            for y in 0..h {
                for x in 0..w {
                    let want = &expected[(x, y)];
                    let got = p.screen().cell(y, x).expect("cell");
                    let norm = |s: &str| {
                        if s.is_empty() {
                            " ".to_owned()
                        } else {
                            s.to_owned()
                        }
                    };
                    assert_eq!(
                        norm(got.contents()),
                        norm(want.symbol()),
                        "frame {t} ({x},{y})"
                    );
                    let rgb = |c: Color| match c {
                        Color::Rgb(r, g, b) => vt100::Color::Rgb(r, g, b),
                        _ => vt100::Color::Default,
                    };
                    assert_eq!(got.fgcolor(), rgb(want.fg), "fg frame {t} ({x},{y})");
                    assert_eq!(got.bgcolor(), rgb(want.bg), "bg frame {t} ({x},{y})");
                    assert_eq!(
                        got.inverse(),
                        want.modifier.contains(Modifier::REVERSED),
                        "reverse frame {t} ({x},{y})"
                    );
                }
            }
        }
    }

    /// Every cell changes glyph, both colours (all three channels at three digits) and
    /// every modifier, against a previous frame that differs everywhere.
    fn worst(buf: &mut Buffer, t: u8) {
        let all = Modifier::all();
        for (i, cell) in buf.content.iter_mut().enumerate() {
            let v = 200 + ((i as u8).wrapping_add(t) % 50);
            cell.set_char(if (i + usize::from(t)) % 2 == 0 {
                '▀'
            } else {
                '⣿'
            })
            .set_fg(Color::Rgb(v, 255 - t, 100 + t))
            .set_bg(Color::Rgb(255 - t, v, 199 - t));
            cell.modifier = if (i + usize::from(t)) % 2 == 0 {
                all
            } else {
                Modifier::REVERSED
            };
        }
    }

    #[test]
    fn rt_r14_rt_r15_a_worst_case_frame_fits_the_reserved_buffer() {
        let (w, h) = (97u16, 31u16);
        let mut term = terminal(w, h);
        term.draw(|f| worst(f.buffer_mut(), 0)).expect("draw");
        let cap = term.backend().capacity();
        assert!(cap >= frame_bound(usize::from(w) * usize::from(h)));
        for t in 1..4 {
            let before = term.backend().writer().bytes.len();
            term.draw(|f| worst(f.buffer_mut(), t)).expect("draw");
            let frame = term.backend().writer().bytes.len() - before;
            assert!(frame <= cap, "frame {t}: {frame} bytes > reserved {cap}");
            assert_eq!(
                term.backend().capacity(),
                cap,
                "frame {t}: the buffer never grows"
            );
        }
    }

    #[test]
    fn rnd_r15_numbers_are_decimal() {
        let mut v = Vec::new();
        for n in [0u16, 7, 10, 255, 65535] {
            v.clear();
            push_u16(&mut v, n);
            assert_eq!(v, n.to_string().as_bytes());
        }
    }

    #[test]
    fn rt_r17_an_empty_diff_writes_nothing_and_does_not_flush() {
        let mut term = terminal(10, 3);
        term.draw(|f| fill(f.buffer_mut(), 1)).expect("draw");
        assert_eq!(term.backend().changed_rows(), Some((0, 2)));
        let c = term.backend().writer();
        let (bytes, writes, flushes) = (c.bytes.len(), c.writes, c.flushes);
        for _ in 0..3 {
            term.draw(|f| fill(f.buffer_mut(), 1)).expect("draw");
            assert_eq!(term.backend().changed_rows(), None);
        }
        let c = term.backend().writer();
        assert_eq!(
            c.bytes.len(),
            bytes,
            "no bytes: no cursor, colour or sync sequences"
        );
        assert_eq!(
            (c.writes, c.flushes),
            (writes, flushes),
            "no write and no flush"
        );
        term.draw(|f| {
            fill(f.buffer_mut(), 1);
            f.buffer_mut()[(4, 1)].set_char('x');
        })
        .expect("draw");
        assert_eq!(term.backend().changed_rows(), Some((1, 1)));
    }

    /// Puts a vt100 screen's cursor into `from`: `At` by an absolute move, `Wrap` by
    /// printing on the last column.
    fn enter(p: &mut vt100::Parser, from: Cursor, w: u16) {
        match from {
            Cursor::Unknown => p.process(b"\x1b[3;3H"),
            Cursor::At(x, y) | Cursor::Loose(x, y) => {
                p.process(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
            }
            Cursor::Wrap(y) => {
                p.process(format!("\x1b[{};{}Hw", y + 1, w).as_bytes());
            }
        }
    }

    #[test]
    fn rnd_r17_every_cursor_move_lands_on_its_target_and_is_never_longer_than_cup() {
        let (w, h) = (23u16, 7u16);
        let mut starts = vec![Cursor::Unknown];
        for y in 0..h {
            starts.push(Cursor::Wrap(y));
            for x in 0..w {
                starts.push(Cursor::At(x, y));
                starts.push(Cursor::Loose(x, y));
            }
        }
        let mut seq = Vec::new();
        for &from in &starts {
            for y in 0..h {
                for x in 0..w {
                    let mut p = vt100::Parser::new(h, w, 0);
                    p.process(b"\x1b[20l");
                    enter(&mut p, from, w);
                    seq.clear();
                    let mut state = State {
                        pen: Pen::DEFAULT,
                        cursor: from,
                        width: w,
                    };
                    encode::move_to(&mut seq, &mut state, x, y);
                    let mut cup = Vec::new();
                    let mut abs = State {
                        cursor: Cursor::Unknown,
                        ..state
                    };
                    encode::move_to(&mut cup, &mut abs, x, y);
                    assert!(
                        seq.len() <= cup.len(),
                        "{from:?} -> ({x},{y}): {seq:?} longer than {cup:?}"
                    );
                    p.process(&seq);
                    p.process(b"X");
                    let got = p.screen().cell(y, x).expect("cell").contents();
                    assert_eq!(got, "X", "{from:?} -> ({x},{y}) via {seq:?}");
                    assert!(
                        matches!(state.cursor, Cursor::At(a, b) | Cursor::Loose(a, b) if (a, b) == (x, y)),
                        "{from:?} -> ({x},{y}): {:?}",
                        state.cursor
                    );
                    if matches!(from, Cursor::Loose(lx, ly) if (lx, ly) != (x, y)) {
                        // The column is set absolutely (CUP, CHA or CR), never relatively.
                        assert!(
                            matches!(seq.last(), Some(b'H' | b'G' | b'\r')),
                            "from {from:?}: {seq:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rnd_r17_scattered_cells_of_one_colour_share_one_sgr() {
        // Braille dots in three ink colours scattered over the screen, interleaved in
        // row-major order: grouped by colour, each colour is set once.
        let mut term = terminal(40, 12);
        term.draw(|_| {}).expect("draw");
        let n = term.backend().writer().bytes.len();
        let inks = [
            Color::Rgb(255, 80, 40),
            Color::Rgb(40, 120, 255),
            Color::Rgb(250, 250, 250),
        ];
        term.draw(|f| {
            let buf = f.buffer_mut();
            for i in 0..36u16 {
                let (x, y) = ((i * 7) % 40, (i * 5) % 12);
                buf[(x, y)].set_char('⣿').set_fg(inks[usize::from(i) % 3]);
            }
        })
        .expect("draw");
        let text = String::from_utf8_lossy(&term.backend().writer().bytes[n..]).into_owned();
        assert_eq!(text.matches("38;2;").count(), 3, "{text:?}");
    }

    #[test]
    fn rnd_r17_a_row_of_distinct_colours_needs_no_cursor_moves_between_cells() {
        // A field row: every cell a different colour, adjacent. Row-major order is the
        // cheaper one, so there is one move for the whole row.
        let mut term = terminal(40, 3);
        term.draw(|_| {}).expect("draw");
        let n = term.backend().writer().bytes.len();
        term.draw(|f| {
            let buf = f.buffer_mut();
            for x in 0..40u16 {
                let v = u8::try_from(x * 6).expect("small");
                buf[(x, 1)]
                    .set_char('▀')
                    .set_fg(Color::Rgb(v, 0, 0))
                    .set_bg(Color::Rgb(0, v, 0));
            }
        })
        .expect("draw");
        let frame = &term.backend().writer().bytes[n..];
        let moves = frame
            .windows(3)
            .filter(|w| w[0] == 0x1b && w[1] == b'[' && w[2].is_ascii_digit())
            .count();
        let cup_or_cuf = String::from_utf8_lossy(frame)
            .split("\x1b[")
            .filter(|s| {
                let end = s.find(|c: char| !(c.is_ascii_digit() || c == ';'));
                end.is_some_and(|e| matches!(&s[e..=e], "H" | "C" | "G"))
            })
            .count();
        assert!(moves >= 40, "one SGR per cell");
        assert_eq!(cup_or_cuf, 1, "{:?}", String::from_utf8_lossy(frame));
    }

    /// What ratatui's own `CrosstermBackend` (the path before M14) leaves on the screen.
    fn reference(frames: &[Buffer], w: u16, h: u16) -> vt100::Parser {
        #[derive(Clone, Default)]
        struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let sink = Shared::default();
        let mut backend = ratatui::backend::CrosstermBackend::new(sink.clone());
        let mut prev = Buffer::empty(Rect::new(0, 0, w, h));
        for frame in frames {
            backend.draw(prev.diff_iter(frame)).expect("draw");
            Backend::flush(&mut backend).expect("flush");
            prev.clone_from(frame);
        }
        let mut p = vt100::Parser::new(h, w, 0);
        p.process(&sink.0.borrow());
        p
    }

    fn same_screen(a: &vt100::Screen, b: &vt100::Screen, w: u16, h: u16, what: &str) {
        for y in 0..h {
            for x in 0..w {
                let (ca, cb) = (a.cell(y, x).expect("cell"), b.cell(y, x).expect("cell"));
                assert_eq!(
                    (
                        ca.contents(),
                        ca.fgcolor(),
                        ca.bgcolor(),
                        ca.inverse(),
                        ca.bold()
                    ),
                    (
                        cb.contents(),
                        cb.fgcolor(),
                        cb.bgcolor(),
                        cb.inverse(),
                        cb.bold()
                    ),
                    "{what} ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn rnd_r17_wide_symbols_keep_row_major_order_and_the_reference_screen() {
        // Wide symbols (never on the canvas, GEN-R10, but possible in free text) disable
        // reordering; the screen still matches ratatui's own backend frame by frame.
        let (w, h) = (16u16, 4u16);
        let area = Rect::new(0, 0, w, h);
        let mut frames = Vec::new();
        for t in 0..4u8 {
            let mut buf = Buffer::empty(area);
            let s = if t % 2 == 0 { "日本ab語" } else { "xyz日" };
            buf.set_string(
                t.into(),
                1,
                s,
                ratatui::style::Style::new().fg(Color::Rgb(t, 9, 9)),
            );
            buf[(15, 3)].set_char('⣿').set_fg(Color::Rgb(1, 2, 3));
            buf[(0, 0)].set_char('⣿').set_fg(Color::Rgb(1, 2, 3));
            frames.push(buf);
        }
        let mut term = terminal(w, h);
        let mut p = vt100::Parser::new(h, w, 0);
        for (i, frame) in frames.iter().enumerate() {
            let n = term.backend().writer().bytes.len();
            term.draw(|f| f.buffer_mut().clone_from(frame))
                .expect("draw");
            p.process(&term.backend().writer().bytes[n..]);
            let r = reference(&frames[..=i], w, h);
            same_screen(p.screen(), r.screen(), w, h, &format!("frame {i}"));
        }
    }
}
