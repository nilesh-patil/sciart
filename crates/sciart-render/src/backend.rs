//! A ratatui backend that sends each frame as one write (RT-R15) in as few bytes as the
//! diff allows (RND-R15).
//!
//! `ratatui`'s `CrosstermBackend` over `stdout()` goes through std's `LineWriter`, which
//! hands the kernel 1 KiB at a time: a 100 KB field frame became ~100 `write` calls, and a
//! terminal that repaints on each read could show a torn frame. It also re-sends the
//! foreground *and* background whenever either changes, moves the cursor with an absolute
//! `CUP` for every gap, and ends even an empty frame with three reset sequences.
//!
//! [`FrameBackend`] appends everything to one reused `Vec<u8>` and hands it to the writer
//! in a single `write_all` + `flush` from [`Backend::flush`], which `Terminal::draw` calls
//! once per frame. Inside, it sends only the SGR parameters that changed, moves right on
//! the same row with a relative `CUF`, sends nothing at all for an empty diff, and wraps a
//! non-empty frame in a synchronized update (DEC private mode 2026) so terminals that
//! support it swap the whole frame at once; others ignore the unknown mode.

use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

/// A ratatui [`Backend`] that buffers a whole frame and writes it once (RT-R15).
#[derive(Debug)]
pub struct FrameBackend<W: Write> {
    out: W,
    /// The pending frame; its capacity is kept, so steady-state frames never allocate.
    buf: Vec<u8>,
    /// Fixed size for tests and the cost probe; `None` asks the terminal.
    size: Option<Size>,
    /// Wrap frames in a synchronized update (DEC mode 2026).
    sync: bool,
    /// A `BEGIN_SYNC` was written and its `END_SYNC` is still due.
    in_sync: bool,
    cursor_hidden: bool,
    /// The terminal's current rendition (fg, bg, modifiers). It carries over from frame
    /// to frame, so a frame that starts in the colour the last one ended in sends no SGR.
    pen: (Color, Color, Modifier),
}

impl<W: Write> FrameBackend<W> {
    /// A backend on a real terminal: its size is queried, frames are synchronized.
    pub fn new(out: W) -> Self {
        Self {
            out,
            buf: Vec::new(),
            size: None,
            sync: true,
            in_sync: false,
            cursor_hidden: false,
            pen: (Color::Reset, Color::Reset, Modifier::empty()),
        }
    }

    /// A backend of a fixed size (tests, the RND-R13 probe): no terminal is queried.
    pub fn with_size(out: W, width: u16, height: u16) -> Self {
        Self {
            size: Some(Size { width, height }),
            ..Self::new(out)
        }
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

    /// Bytes queued for the next flush.
    pub fn pending(&self) -> &[u8] {
        &self.buf
    }

    fn begin_frame(&mut self) {
        if self.sync && !self.in_sync {
            self.buf.extend_from_slice(BEGIN_SYNC);
            self.in_sync = true;
        }
    }

    fn move_to(&mut self, x: u16, y: u16) {
        self.buf.extend_from_slice(b"\x1b[");
        push_u16(&mut self.buf, y.saturating_add(1));
        self.buf.push(b';');
        push_u16(&mut self.buf, x.saturating_add(1));
        self.buf.push(b'H');
    }

    /// Queue an SGR reset when the rendition is not the default; `ED`/`EL` erase with the
    /// current background, and the shell after exit expects the default.
    fn reset_pen(&mut self) {
        if self.pen != (Color::Reset, Color::Reset, Modifier::empty()) {
            self.buf.extend_from_slice(b"\x1b[0m");
            self.pen = (Color::Reset, Color::Reset, Modifier::empty());
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
        let (mut fg, mut bg, mut modifier) = self.pen;
        // Where the terminal cursor is after the last print, when known.
        let mut cursor: Option<(u16, u16)> = None;
        let mut drew = false;
        for (x, y, cell) in content {
            if !drew {
                self.begin_frame();
                drew = true;
            }
            match cursor {
                Some((cx, cy)) if cy == y && cx == x => {}
                // Same row, further right: `CUF n` is shorter than `CUP y;x`.
                Some((cx, cy)) if cy == y && cx < x => {
                    self.buf.extend_from_slice(b"\x1b[");
                    let n = x - cx;
                    if n > 1 {
                        push_u16(&mut self.buf, n);
                    }
                    self.buf.push(b'C');
                }
                _ => self.move_to(x, y),
            }

            // One SGR for everything that changed. A modifier change resets first (`0`),
            // which also resets both colours, so they are re-sent if set.
            let mut sgr = Sgr::new(&mut self.buf);
            if cell.modifier != modifier {
                sgr.param(0);
                for (flag, code) in MODIFIER_CODES {
                    if cell.modifier.contains(flag) {
                        sgr.param(code);
                    }
                }
                modifier = cell.modifier;
                (fg, bg) = (Color::Reset, Color::Reset);
            }
            if cell.fg != fg {
                sgr.color(cell.fg, false);
                fg = cell.fg;
            }
            if cell.bg != bg {
                sgr.color(cell.bg, true);
                bg = cell.bg;
            }
            sgr.end();

            let symbol = cell.symbol();
            self.buf.extend_from_slice(symbol.as_bytes());
            // A width-0 symbol still occupies its cell once printed as a space would; use
            // at least 1 so the next relative move stays conservative-correct.
            let width = u16::try_from(symbol.width().max(1)).unwrap_or(1);
            cursor = Some((x.saturating_add(width), y));
        }
        self.pen = (fg, bg, modifier);
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
        self.move_to(x, y);
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

/// SGR codes of the modifiers ratatui can set.
const MODIFIER_CODES: [(Modifier, u16); 9] = [
    (Modifier::BOLD, 1),
    (Modifier::DIM, 2),
    (Modifier::ITALIC, 3),
    (Modifier::UNDERLINED, 4),
    (Modifier::SLOW_BLINK, 5),
    (Modifier::RAPID_BLINK, 6),
    (Modifier::REVERSED, 7),
    (Modifier::HIDDEN, 8),
    (Modifier::CROSSED_OUT, 9),
];

/// Builds one `CSI … m` from parameters; writes nothing when no parameter was added.
struct Sgr<'a> {
    buf: &'a mut Vec<u8>,
    any: bool,
}

impl<'a> Sgr<'a> {
    fn new(buf: &'a mut Vec<u8>) -> Self {
        Self { buf, any: false }
    }

    fn sep(&mut self) {
        if self.any {
            self.buf.push(b';');
        } else {
            self.buf.extend_from_slice(b"\x1b[");
            self.any = true;
        }
    }

    fn param(&mut self, n: u16) {
        self.sep();
        push_u16(self.buf, n);
    }

    /// The same parameters crossterm sends for each colour (`38;2;r;g;b`, `39`, …).
    fn color(&mut self, color: Color, background: bool) {
        let base = if background { 10 } else { 0 };
        match color {
            Color::Reset => self.param(39 + base),
            Color::Rgb(r, g, b) => {
                self.param(38 + base);
                for n in [2, u16::from(r), u16::from(g), u16::from(b)] {
                    self.param(n);
                }
            }
            Color::Indexed(i) => {
                self.param(38 + base);
                self.param(5);
                self.param(u16::from(i));
            }
            named => self.param(named_code(named) + base),
        }
    }

    fn end(self) {
        if self.any {
            self.buf.push(b'm');
        }
    }
}

/// SGR foreground code of a named colour (crossterm's mapping: `Gray` is 37, `DarkGray` 90).
fn named_code(color: Color) -> u16 {
    match color {
        Color::Black => 30,
        Color::Red => 31,
        Color::Green => 32,
        Color::Yellow => 33,
        Color::Blue => 34,
        Color::Magenta => 35,
        Color::Cyan => 36,
        Color::Gray => 37,
        Color::DarkGray => 90,
        Color::LightRed => 91,
        Color::LightGreen => 92,
        Color::LightYellow => 93,
        Color::LightBlue => 94,
        Color::LightMagenta => 95,
        Color::LightCyan => 96,
        Color::White => 97,
        _ => 39,
    }
}

/// Decimal digits of `n`, without `fmt` machinery.
fn push_u16(buf: &mut Vec<u8>, n: u16) {
    let mut digits = [0u8; 5];
    let mut i = digits.len();
    let mut n = n;
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    buf.extend_from_slice(&digits[i..]);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

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

    #[test]
    fn rt_r15_each_frame_is_one_write_and_one_flush() {
        let mut term = terminal(40, 12);
        for t in 0..5 {
            let before = {
                let c = term.backend().writer();
                (c.writes, c.flushes)
            };
            term.draw(|f| fill(f.buffer_mut(), t)).expect("draw");
            let c = term.backend().writer();
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
    fn rnd_r15_rt_r15_screen_matches_the_buffer_through_vt100() {
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

    #[test]
    fn rnd_r15_numbers_are_decimal() {
        let mut v = Vec::new();
        for n in [0u16, 7, 10, 255, 65535] {
            v.clear();
            push_u16(&mut v, n);
            assert_eq!(v, n.to_string().as_bytes());
        }
    }
}
