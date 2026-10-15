//! The frame encoder behind [`crate::FrameBackend`] (RND-R15, RND-R17).
//!
//! A frame's changed cells are independent once every one of them is a single-width
//! symbol, so the order in which they reach the terminal is free (a frame with a wide
//! symbol keeps the diff's order). Of two orders the shorter is sent: row-major is written,
//! the grouped order is costed with a dry run that only counts bytes and replaces it when
//! strictly shorter ([`encode_best`]):
//!
//! - **row-major** (the diff's own order): adjacent cells need no cursor move at all,
//!   which wins on full-screen fields whose neighbouring cells differ in colour anyway;
//! - **style-grouped** (sorted by fg, bg, modifiers, then position): one SGR per distinct
//!   style instead of one per colour change along the row. Scenes draw with stepped ink
//!   levels, so a frame of scattered braille dots has a few dozen distinct colours over
//!   hundreds of cells; a 24-bit SGR costs 15–19 bytes, a short cursor move 1–5.
//!
//! Cursor moves pick the shortest of an absolute `CUP` and the relative forms (LF, `CUD`,
//! `CUU`, `CUF`, `CUB`, backspace, CR, `CHA`). The encoder tracks the pending-wrap state
//! after the last column (a print there leaves the cursor *on* the last column with a wrap
//! pending), from which only CR and `CUP` are used, since terminals disagree on what the
//! relative moves do there. LF moves straight down because raw mode clears `OPOST`, and the
//! backend resets the newline mode (`LNM`) once; LF is only used to move to a row below the
//! cursor, so it never scrolls.

use ratatui::style::{Color, Modifier};

/// Where the terminal cursor is, as far as the encoder knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cursor {
    Unknown,
    At(u16, u16),
    /// On the last column of row `y` with a wrap pending (after printing there).
    Wrap(u16),
}

/// The terminal's current rendition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub modifier: Modifier,
}

impl Pen {
    pub const DEFAULT: Self = Self {
        fg: Color::Reset,
        bg: Color::Reset,
        modifier: Modifier::empty(),
    };
}

/// Encoder state that carries from cell to cell (and the pen from frame to frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct State {
    pub pen: Pen,
    pub cursor: Cursor,
    /// Terminal width in columns; 0 when unknown (then every move is absolute).
    pub width: u16,
}

/// A changed cell, copied out of the diff so it can be reordered. Its symbol lives in the
/// frame's symbol arena at `sym`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    /// Sort key of the style: fg, bg and modifiers packed (see [`style_key`]).
    pub key: u64,
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub sym: (u32, u32),
    pub pen: Pen,
}

/// Byte sink: a real buffer, or a counter for the dry runs.
pub(crate) trait Out {
    fn bytes(&mut self, b: &[u8]);
    fn byte(&mut self, b: u8) {
        self.bytes(&[b]);
    }
}

impl Out for Vec<u8> {
    #[inline]
    fn bytes(&mut self, b: &[u8]) {
        self.extend_from_slice(b);
    }
    #[inline]
    fn byte(&mut self, b: u8) {
        self.push(b);
    }
}

/// Counts bytes without writing them.
#[derive(Debug, Default)]
pub(crate) struct Count(pub usize);

impl Out for Count {
    #[inline]
    fn bytes(&mut self, b: &[u8]) {
        self.0 += b.len();
    }
    #[inline]
    fn byte(&mut self, _: u8) {
        self.0 += 1;
    }
}

/// Packs a style into a sort key: equal styles have equal keys and vice versa.
pub(crate) fn style_key(pen: Pen) -> u64 {
    fn color(c: Color) -> u64 {
        match c {
            Color::Reset => 0,
            Color::Indexed(i) => (2 << 24) | u64::from(i),
            Color::Rgb(r, g, b) => {
                (3 << 24) | (u64::from(r) << 16) | (u64::from(g) << 8) | u64::from(b)
            }
            named => (1 << 24) | u64::from(named_code(named)),
        }
    }
    (color(pen.fg) << 38) | (color(pen.bg) << 12) | u64::from(pen.modifier.bits() & 0x0fff)
}

/// Encode `cells` (all single-width, in any order) from `state` into `out`.
pub(crate) fn encode<'a, O: Out>(
    out: &mut O,
    state: &mut State,
    cells: impl Iterator<Item = &'a Pending>,
    syms: &[u8],
) {
    for p in cells {
        move_to(out, state, p.x, p.y);
        sgr(out, &mut state.pen, p.pen);
        let (off, len) = p.sym;
        let sym = &syms[off as usize..off as usize + len as usize];
        out.bytes(sym);
        let nx = p.x.saturating_add(p.width);
        state.cursor = if state.width == 0 {
            Cursor::Unknown
        } else if nx >= state.width {
            Cursor::Wrap(p.y)
        } else {
            Cursor::At(nx, p.y)
        };
    }
}

/// Encode `cells` (in row-major order, as the diff yields them) in whichever of row-major
/// and style-grouped order is shorter. `order` is scratch space (its capacity is kept).
///
/// Row-major is written first; the grouped order is costed with a dry run and replaces it
/// only when strictly shorter. Grouping sorts `(style key, row-major index)` pairs, so
/// within a style the cells stay in row-major order and `cells` itself never moves.
///
/// Frames of more than `max_grouped` cells try grouping only when they use at most
/// [`FEW_STYLES`] distinct styles: a big frame in many colours (a full-screen field, the
/// repaint after a theme change or resize) is cheapest in row-major order, and the sort
/// and second pass would cost the most exactly there, while a big frame of dots in a few
/// colours gains the most from grouping.
pub(crate) fn encode_best(
    out: &mut Vec<u8>,
    state: &mut State,
    cells: &[Pending],
    order: &mut Vec<(u64, u32)>,
    syms: &[u8],
    max_grouped: usize,
) {
    let start = (out.len(), *state);
    encode(out, state, cells.iter(), syms);
    let row_major = out.len() - start.0;
    if cells.len() < 2
        || cells.len() > u32::MAX as usize
        || (cells.len() > max_grouped && !few_styles(cells))
        || cells.iter().any(|p| p.width != 1)
    {
        return;
    }
    order.clear();
    order.extend(cells.iter().zip(0u32..).map(|(p, i)| (p.key, i)));
    order.sort_unstable();
    let grouped_cells = || order.iter().map(|&(_, i)| &cells[i as usize]);
    let mut n = Count(0);
    let mut dry = start.1;
    encode(&mut n, &mut dry, grouped_cells(), syms);
    if n.0 < row_major {
        out.truncate(start.0);
        *state = start.1;
        encode(out, state, grouped_cells(), syms);
    }
}

/// Distinct styles up to which a big frame still tries the grouped order.
pub(crate) const FEW_STYLES: usize = 64;

/// Whether `cells` use at most [`FEW_STYLES`] distinct styles: an open-addressing set on
/// the stack, given up as soon as it holds more.
fn few_styles(cells: &[Pending]) -> bool {
    // Twice the limit keeps probes short; `style_key` is never `u64::MAX` (its modifier
    // bits stop at 0x1ff), so that marks a free slot.
    const SLOTS: usize = 2 * FEW_STYLES;
    let mut set = [u64::MAX; SLOTS];
    let mut n = 0;
    for p in cells {
        // Fibonacci hashing onto the 7 top bits (SLOTS = 128).
        let mut i = (p.key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 57) as usize;
        loop {
            if set[i] == p.key {
                break;
            }
            if set[i] == u64::MAX {
                set[i] = p.key;
                n += 1;
                if n > FEW_STYLES {
                    return false;
                }
                break;
            }
            i = (i + 1) % SLOTS;
        }
    }
    true
}

/// Cursor movement to `(x, y)`: the shortest of `CUP` and the relative forms.
pub(crate) fn move_to<O: Out>(out: &mut O, state: &mut State, x: u16, y: u16) {
    let (from, prefix_cr) = match state.cursor {
        Cursor::At(cx, cy) if cx == x && cy == y => return,
        Cursor::At(cx, cy) => (Some((cx, cy)), false),
        // Only CR is trusted from the pending-wrap state; then move from column 0.
        Cursor::Wrap(cy) => (Some((0, cy)), true),
        Cursor::Unknown => (None, false),
    };
    let abs = cup_cost(x, y);
    if let Some((cx, cy)) = from {
        let v = vert_cost(cy, y);
        let h = horiz_plan(cx, x);
        if usize::from(prefix_cr) + v + h.cost() < abs {
            if prefix_cr {
                out.byte(b'\r');
            }
            vert(out, cy, y);
            h.emit(out);
            state.cursor = Cursor::At(x, y);
            return;
        }
    }
    cup(out, x, y);
    state.cursor = Cursor::At(x, y);
}

fn digits(n: u16) -> usize {
    match n {
        0..=9 => 1,
        10..=99 => 2,
        100..=999 => 3,
        1000..=9999 => 4,
        _ => 5,
    }
}

/// `CSI n <c>` with `n` omitted when 1.
fn csi_n_cost(n: u16) -> usize {
    if n == 1 { 3 } else { 3 + digits(n) }
}

fn csi_n<O: Out>(out: &mut O, n: u16, c: u8) {
    out.bytes(b"\x1b[");
    if n != 1 {
        push_u16(out, n);
    }
    out.byte(c);
}

fn cup_cost(x: u16, y: u16) -> usize {
    if x == 0 {
        // `CSI row H` (the column defaults to 1); `CSI H` for the origin.
        if y == 0 { 3 } else { 3 + digits(y + 1) }
    } else {
        4 + digits(y.saturating_add(1)) + digits(x.saturating_add(1))
    }
}

fn cup<O: Out>(out: &mut O, x: u16, y: u16) {
    out.bytes(b"\x1b[");
    if x == 0 {
        if y != 0 {
            push_u16(out, y + 1);
        }
    } else {
        push_u16(out, y.saturating_add(1));
        out.byte(b';');
        push_u16(out, x.saturating_add(1));
    }
    out.byte(b'H');
}

/// Up to this many rows down, repeated LFs are no longer than `CUD`.
const MAX_LF: u16 = 3;

fn vert_cost(cy: u16, y: u16) -> usize {
    match y.cmp(&cy) {
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater if y - cy <= MAX_LF => usize::from(y - cy),
        std::cmp::Ordering::Greater => csi_n_cost(y - cy),
        std::cmp::Ordering::Less => csi_n_cost(cy - y),
    }
}

fn vert<O: Out>(out: &mut O, cy: u16, y: u16) {
    match y.cmp(&cy) {
        std::cmp::Ordering::Equal => {}
        std::cmp::Ordering::Greater if y - cy <= MAX_LF => {
            for _ in cy..y {
                out.byte(b'\n');
            }
        }
        std::cmp::Ordering::Greater => csi_n(out, y - cy, b'B'),
        std::cmp::Ordering::Less => csi_n(out, cy - y, b'A'),
    }
}

/// A horizontal move on the cursor's row.
#[derive(Debug, Clone, Copy)]
enum Horiz {
    None,
    /// `CUF n`
    Right(u16),
    /// `CUB n`
    Left(u16),
    /// `n` backspaces
    Back(u16),
    /// CR, then `CUF n` if `n > 0`
    Cr(u16),
    /// `CHA col` (0-based column)
    Cha(u16),
}

/// Up to this many columns left, backspaces are no longer than `CUB`.
const MAX_BS: u16 = 3;

impl Horiz {
    fn cost(self) -> usize {
        match self {
            Self::None => 0,
            Self::Right(n) | Self::Left(n) => csi_n_cost(n),
            Self::Back(n) => usize::from(n),
            Self::Cr(n) => 1 + if n == 0 { 0 } else { csi_n_cost(n) },
            Self::Cha(x) => csi_n_cost(x + 1),
        }
    }

    fn emit<O: Out>(self, out: &mut O) {
        match self {
            Self::None => {}
            Self::Right(n) => csi_n(out, n, b'C'),
            Self::Left(n) => csi_n(out, n, b'D'),
            Self::Back(n) => {
                for _ in 0..n {
                    out.byte(0x08);
                }
            }
            Self::Cr(n) => {
                out.byte(b'\r');
                if n > 0 {
                    csi_n(out, n, b'C');
                }
            }
            Self::Cha(x) => csi_n(out, x + 1, b'G'),
        }
    }
}

fn horiz_plan(cx: u16, x: u16) -> Horiz {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let shorter = |a: Horiz, b: Horiz| if b.cost() < a.cost() { b } else { a };
    match x.cmp(&cx) {
        Equal => Horiz::None,
        Greater => shorter(Horiz::Right(x - cx), Horiz::Cha(x)),
        Less => {
            let n = cx - x;
            let back = if n <= MAX_BS {
                Horiz::Back(n)
            } else {
                Horiz::Left(n)
            };
            shorter(shorter(back, Horiz::Cr(x)), Horiz::Cha(x))
        }
    }
}

/// One SGR for everything that changed from `pen` to `want`. A modifier change resets
/// first (`0`), which also resets both colours, so they are re-sent if set.
pub(crate) fn sgr<O: Out>(out: &mut O, pen: &mut Pen, want: Pen) {
    if *pen == want {
        return;
    }
    let mut s = Sgr { out, any: false };
    if want.modifier != pen.modifier {
        s.param(0);
        for (flag, code) in MODIFIER_CODES {
            if want.modifier.contains(flag) {
                s.param(code);
            }
        }
        pen.modifier = want.modifier;
        pen.fg = Color::Reset;
        pen.bg = Color::Reset;
    }
    if want.fg != pen.fg {
        s.color(want.fg, false);
        pen.fg = want.fg;
    }
    if want.bg != pen.bg {
        s.color(want.bg, true);
        pen.bg = want.bg;
    }
    s.end();
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
struct Sgr<'a, O: Out> {
    out: &'a mut O,
    any: bool,
}

impl<O: Out> Sgr<'_, O> {
    fn sep(&mut self) {
        if self.any {
            self.out.byte(b';');
        } else {
            self.out.bytes(b"\x1b[");
            self.any = true;
        }
    }

    fn param(&mut self, n: u16) {
        self.sep();
        push_u16(self.out, n);
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
            self.out.byte(b'm');
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
pub(crate) fn push_u16<O: Out>(out: &mut O, n: u16) {
    // Colour channels and most coordinates are below 256: one table lookup (RND-R17
    // costs every frame twice, so this is on the hot path).
    if let Some(&(len, ref d)) = DECIMAL.get(usize::from(n)) {
        out.bytes(&d[..usize::from(len)]);
        return;
    }
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
    out.bytes(&digits[i..]);
}

/// Decimal digits of 0..=255: `(length, digits)`.
static DECIMAL: [(u8, [u8; 3]); 256] = {
    let mut t = [(0u8, [0u8; 3]); 256];
    let mut n = 0;
    while n < 256 {
        let (h, te, o) = ((n / 100) as u8, (n / 10 % 10) as u8, (n % 10) as u8);
        t[n] = if n >= 100 {
            (3, [b'0' + h, b'0' + te, b'0' + o])
        } else if n >= 10 {
            (2, [b'0' + te, b'0' + o, 0])
        } else {
            (1, [b'0' + o, 0, 0])
        };
        n += 1;
    }
    t
};

#[cfg(test)]
mod tests {
    use super::*;

    fn pen(fg: Color, bg: Color, modifier: Modifier) -> Pen {
        Pen { fg, bg, modifier }
    }

    fn colors() -> Vec<Color> {
        let mut v = vec![Color::Reset, Color::Red, Color::DarkGray, Color::White];
        v.extend([0u8, 7, 255].map(Color::Indexed));
        v.extend(
            [(0, 0, 0), (255, 255, 255), (1, 2, 3), (3, 2, 1)].map(|(r, g, b)| Color::Rgb(r, g, b)),
        );
        v
    }

    #[test]
    fn rnd_r17_style_keys_are_equal_exactly_when_styles_are() {
        let mods = [Modifier::empty(), Modifier::REVERSED, Modifier::all()];
        let mut pens = Vec::new();
        for &fg in &colors() {
            for &bg in &colors() {
                for &m in &mods {
                    pens.push(pen(fg, bg, m));
                }
            }
        }
        for a in &pens {
            assert_ne!(
                style_key(*a),
                u64::MAX,
                "MAX marks a free slot in few_styles"
            );
            for b in &pens {
                assert_eq!(style_key(*a) == style_key(*b), a == b, "{a:?} {b:?}");
            }
        }
    }

    fn cells(styles: usize, n: usize) -> Vec<Pending> {
        (0..n)
            .map(|i| {
                let s = u8::try_from(i % styles).expect("< 256 styles");
                let p = pen(Color::Rgb(s, 0, 0), Color::Reset, Modifier::empty());
                Pending {
                    key: style_key(p),
                    y: 0,
                    x: 0,
                    width: 1,
                    sym: (0, 1),
                    pen: p,
                }
            })
            .collect()
    }

    #[test]
    fn rnd_r17_few_styles_counts_distinct_styles_up_to_the_limit() {
        assert!(few_styles(&[]));
        assert!(few_styles(&cells(1, 5000)));
        assert!(few_styles(&cells(FEW_STYLES, 5000)));
        assert!(!few_styles(&cells(FEW_STYLES + 1, 5000)));
        assert!(!few_styles(&cells(255, 5000)));
    }
}
