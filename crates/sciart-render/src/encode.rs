//! The frame encoder behind [`crate::FrameBackend`] (RND-R15, RND-R17).
//!
//! A frame's changed cells are independent once every one of them is a single-width
//! symbol, so the order in which they reach the terminal is free (a frame with a wide
//! symbol keeps the diff's order). Of two orders the shorter is sent; both are written,
//! into two reused buffers, only on probe frames ([`Choice`]):
//!
//! - **row-major** (the diff's own order): adjacent cells need no cursor move at all,
//!   which wins on full-screen fields whose neighbouring cells differ in colour anyway;
//! - **style-grouped** (sorted by fg, bg, modifiers, then position): one SGR per distinct
//!   style instead of one per colour change along the row. Scenes draw with stepped ink
//!   levels, so a frame of scattered braille dots has a few dozen distinct colours over
//!   hundreds of cells; a 24-bit SGR costs 15–19 bytes, a short cursor move 1–5.
//!
//! Cursor moves pick the shortest of an absolute `CUP` and the relative forms (`CUD`,
//! `CUU`, `CUF`, `CUB`, VT, backspace, CR, `CHA`). Never LF: stdout's `LineWriter` would
//! split the frame at the last '\n' into two writes (RT-R15); VT moves down exactly as LF
//! does (xterm documents it so; the backend resets the newline mode `LNM` once, which VT
//! follows like LF). The encoder tracks the pending-wrap
//! state after the last column (a print there leaves the cursor *on* the last column with
//! a wrap pending), from which only CR and `CUP` are used, since terminals disagree on what
//! the relative moves do there; and after a symbol of ambiguous width it sets the column
//! absolutely (`CHA`, CR or `CUP`, [`Cursor::Loose`]).

use ratatui::style::{Color, Modifier};

/// Where the terminal cursor is, as far as the encoder knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cursor {
    Unknown,
    At(u16, u16),
    /// After a symbol whose width terminals may disagree on (neither ASCII nor braille):
    /// probably at `(x, y)`, so printing on there is as safe as it ever was, but a relative
    /// move from here could land off by a column; any other target gets a `CUP`.
    Loose(u16, u16),
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

/// A changed cell, copied out of the diff so it can be reordered (28 bytes). Its symbol
/// is inline when it fits in 4 bytes (any one `char`: every canvas and HUD cell), else in
/// the frame's symbol arena ([`Pending::new`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    /// 1..=4: the symbol is `sym[..len]`; 0: it is in the arena at `long`.
    len: u8,
    sym: [u8; 4],
    long: u32,
    pub pen: Pen,
    /// The style's slot in [`worth_grouping`]'s set, which groups the cells.
    pub slot: u16,
}

/// Byte sink of the encoder (the frame buffers).
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

impl Pending {
    /// A cell printing `symbol`; a symbol longer than 4 bytes is appended to `arena`
    /// (length as two bytes, then the bytes).
    pub(crate) fn new(
        (x, y, width): (u16, u16, u16),
        pen: Pen,
        symbol: &[u8],
        arena: &mut Vec<u8>,
    ) -> Self {
        let mut p = Self {
            y,
            x,
            width,
            len: 0,
            sym: [0; 4],
            long: 0,
            pen,
            slot: 0,
        };
        if let Some(inline) = p.sym.get_mut(..symbol.len()).filter(|_| !symbol.is_empty()) {
            inline.copy_from_slice(symbol);
            p.len = symbol.len() as u8; // 1..=4
        } else {
            let len = u16::try_from(symbol.len()).unwrap_or(u16::MAX);
            p.long = u32::try_from(arena.len()).unwrap_or(u32::MAX);
            arena.extend_from_slice(&len.to_le_bytes());
            arena.extend_from_slice(&symbol[..usize::from(len)]);
        }
        p
    }

    /// The symbol's bytes.
    pub(crate) fn symbol<'a>(&'a self, arena: &'a [u8]) -> &'a [u8] {
        if self.len > 0 {
            &self.sym[..usize::from(self.len)]
        } else {
            let at = self.long as usize;
            let len = usize::from(u16::from_le_bytes([arena[at], arena[at + 1]]));
            &arena[at + 2..at + 2 + len]
        }
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

/// Print one cell at `(x, y)`: the move there, the SGR delta, the symbol; then track where
/// the cursor is.
#[inline]
pub(crate) fn put<O: Out>(
    out: &mut O,
    state: &mut State,
    (x, y, width): (u16, u16, u16),
    pen: Pen,
    sym: &[u8],
) {
    move_to(out, state, x, y);
    sgr(out, &mut state.pen, pen);
    out.bytes(sym);
    let nx = x.saturating_add(width);
    state.cursor = if state.width == 0 {
        Cursor::Unknown
    } else if nx >= state.width {
        Cursor::Wrap(y)
    } else if unambiguous(sym) {
        Cursor::At(nx, y)
    } else {
        Cursor::Loose(nx, y)
    };
}

/// Encode `cells` (all single-width, in any order) from `state` into `out`.
pub(crate) fn encode<'a, O: Out>(
    out: &mut O,
    state: &mut State,
    cells: impl Iterator<Item = &'a Pending>,
    syms: &[u8],
) {
    for p in cells {
        put(out, state, (p.x, p.y, p.width), p.pen, p.symbol(syms));
    }
}

/// How the next frame is encoded (RND-R17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Straight from the diff, as it comes: no copy, no sort.
    RowMajor,
    /// Collect the cells, write them grouped by style (row-major if they can't gain).
    Grouped,
    /// Write row-major and collect; also write grouped, keep the shorter, remember which.
    Probe,
}

/// Which cell order recent frames found shorter (RND-R17). A scene's consecutive frames
/// are alike, so both orders are written only on a probe frame, every [`PROBE_EVERY`]
/// frames or right after the frame size jumps; the frames in between write just the
/// order the last probe chose, once. Either order reproduces the screen exactly, so a
/// stale choice costs bytes, never fidelity.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Choice {
    grouped: bool,
    /// Frames until the next probe.
    countdown: u8,
    /// Cells in the last probe frame.
    probed_cells: usize,
}

/// Frames per probe: one a second at 30 FPS.
pub(crate) const PROBE_EVERY: u8 = 30;

impl Choice {
    pub(crate) fn plan(&self) -> Plan {
        if self.countdown == 0 {
            Plan::Probe
        } else if self.grouped {
            Plan::Grouped
        } else {
            Plan::RowMajor
        }
    }

    /// After a frame of `cells` cells that was not a probe. A frame more than twice or
    /// less than half the size of the last probe (a scene switch, a burst) makes the next
    /// frame a probe.
    pub(crate) fn tick(&mut self, cells: usize) {
        self.countdown = self.countdown.saturating_sub(1);
        if cells > 2 * self.probed_cells || 2 * cells < self.probed_cells {
            self.countdown = 0;
        }
    }

    /// After a probe frame of `cells` cells that found `grouped` shorter.
    pub(crate) fn probed(&mut self, grouped: bool, cells: usize) {
        *self = Self {
            grouped,
            countdown: PROBE_EVERY - 1,
            probed_cells: cells,
        };
    }
}

/// A [`Plan::Grouped`] frame: `cells` grouped by style into `out`, or row-major when they
/// can't gain from it ([`worth_grouping`]).
pub(crate) fn encode_grouped(
    out: &mut Vec<u8>,
    state: &mut State,
    cells: &mut [Pending],
    order: &mut Vec<u32>,
    syms: &[u8],
) {
    if worth_grouping(cells) {
        group(cells, order);
        encode(out, state, order.iter().map(|&i| &cells[i as usize]), syms);
    } else {
        encode(out, state, cells.iter(), syms);
    }
}

/// A [`Plan::Probe`] frame: `out` holds the frame row-major (its first `prefix` bytes
/// lead either order) and `state` is where that left the terminal; `start` is the state
/// before the cells. Writes the grouped order into `alt` and, when shorter, swaps it in.
/// Returns whether the grouped order won. `alt` and `order` keep their capacity.
#[allow(clippy::too_many_arguments)] // the frame's reused buffers, passed apart to borrow apart
pub(crate) fn probe(
    out: &mut Vec<u8>,
    alt: &mut Vec<u8>,
    prefix: usize,
    start: State,
    state: &mut State,
    cells: &mut [Pending],
    order: &mut Vec<u32>,
    syms: &[u8],
) -> bool {
    if !worth_grouping(cells) {
        return false;
    }
    group(cells, order);
    alt.clear();
    alt.extend_from_slice(&out[..prefix]);
    let mut grouped = start;
    encode(
        alt,
        &mut grouped,
        order.iter().map(|&i| &cells[i as usize]),
        syms,
    );
    let won = alt.len() < out.len();
    if won {
        std::mem::swap(out, alt);
        *state = grouped;
    }
    won
}

/// Frames up to this many cells try grouping whenever their styles repeat.
pub(crate) const GROUP_ANY_CELLS: usize = 4096;
/// Bigger frames try it only with at most this many distinct styles.
pub(crate) const FEW_STYLES: usize = 64;
/// Distinct styles counted at most (the set below is twice this size).
const STYLE_CAP: usize = 256;

/// Whether the grouped order can be the shorter one, cheaply: every cell single-width
/// (reordering is only free then), and styles that repeat: on average at least twice per
/// style, and for a frame of more than [`GROUP_ANY_CELLS`] cells at most [`FEW_STYLES`] of
/// them. A frame in many colours (a full-screen field, the repaint after a theme change or
/// resize) is cheapest row-major, and the sort and second pass would cost the most there.
///
/// When it returns `true`, every cell's `slot` holds its style's slot in the set.
pub(crate) fn worth_grouping(cells: &mut [Pending]) -> bool {
    // Open addressing on the stack; `style_key` is never `u64::MAX` (its modifier bits stop
    // at 0x1ff), so that marks a free slot.
    const SLOTS: usize = 2 * STYLE_CAP;
    if cells.len() < 2 || cells.len() > u32::MAX as usize {
        return false;
    }
    let limit = if cells.len() > GROUP_ANY_CELLS {
        FEW_STYLES
    } else {
        STYLE_CAP
    };
    let limit = limit.min(cells.len() / 2);
    let mut set = [u64::MAX; SLOTS];
    let mut n = 0;
    for p in cells.iter_mut() {
        if p.width != 1 {
            return false;
        }
        let key = style_key(p.pen);
        // Fibonacci hashing onto the 9 top bits (SLOTS = 512).
        let mut i = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 55) as usize;
        loop {
            if set[i] == key {
                break;
            }
            if set[i] == u64::MAX {
                set[i] = key;
                n += 1;
                if n > limit {
                    return false;
                }
                break;
            }
            i = (i + 1) % SLOTS;
        }
        // `i < SLOTS = 512`.
        p.slot = i as u16;
    }
    true
}

/// `order` = the indices of `cells` grouped by style slot (set by [`worth_grouping`]),
/// row-major within each group: a stable counting sort, no allocation once `order` has
/// the capacity.
fn group(cells: &[Pending], order: &mut Vec<u32>) {
    const SLOTS: usize = 2 * STYLE_CAP;
    let mut start = [0u32; SLOTS];
    for p in cells {
        start[usize::from(p.slot)] += 1;
    }
    let mut sum = 0;
    for s in &mut start {
        let n = *s;
        *s = sum;
        sum += n;
    }
    order.clear();
    order.resize(cells.len(), 0);
    for (p, i) in cells.iter().zip(0u32..) {
        let s = &mut start[usize::from(p.slot)];
        order[*s as usize] = i;
        *s += 1;
    }
}

/// One column wide on every terminal: ASCII, or braille (U+2800..=U+28FF, UTF-8
/// `E2 A0..=A3 xx`), which is never ambiguous-width. Half blocks, stars and arrows are
/// East Asian "ambiguous" and double on terminals set to treat them so.
fn unambiguous(sym: &[u8]) -> bool {
    match sym {
        [b] => b.is_ascii(),
        [0xE2, b, _] => (0xA0..=0xA3).contains(b),
        _ => false,
    }
}

/// Cursor movement to `(x, y)`: the shortest of `CUP` and the relative forms.
pub(crate) fn move_to<O: Out>(out: &mut O, state: &mut State, x: u16, y: u16) {
    let (from, prefix_cr) = match state.cursor {
        Cursor::At(cx, cy) if cx == x && cy == y => return,
        // The common case in grouped order, decided without costing: on the same row to
        // the right, `CUF n` is the shortest (`CHA` needs at least as many digits, `CUP`
        // more).
        Cursor::At(cx, cy) if cy == y && cx < x => {
            csi_n(out, x - cx, b'C');
            state.cursor = Cursor::At(x, y);
            return;
        }
        Cursor::At(cx, cy) => (Some((cx, cy)), false),
        Cursor::Loose(cx, cy) if cx == x && cy == y => return,
        // The row is certain, the column is not: a vertical move keeps whatever column the
        // terminal has, then CR or `CHA` sets it absolutely.
        Cursor::Loose(_, cy) => {
            let h = if x == 0 { Horiz::Cr(0) } else { Horiz::Cha(x) };
            if vert_cost(cy, y) + h.cost() < cup_cost(x, y) {
                vert(out, cy, y);
                h.emit(out);
            } else {
                cup(out, x, y);
            }
            state.cursor = Cursor::At(x, y);
            return;
        }
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
        if y == 0 {
            3
        } else {
            3 + digits(y.saturating_add(1))
        }
    } else {
        4 + digits(y.saturating_add(1)) + digits(x.saturating_add(1))
    }
}

fn cup<O: Out>(out: &mut O, x: u16, y: u16) {
    out.bytes(b"\x1b[");
    if x == 0 {
        if y != 0 {
            push_u16(out, y.saturating_add(1));
        }
    } else {
        push_u16(out, y.saturating_add(1));
        out.byte(b';');
        push_u16(out, x.saturating_add(1));
    }
    out.byte(b'H');
}

/// Up to this many rows down, repeated VTs are no longer than `CUD`.
const MAX_VT: u16 = 3;

fn vert_cost(cy: u16, y: u16) -> usize {
    match y.cmp(&cy) {
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater if y - cy <= MAX_VT => usize::from(y - cy),
        std::cmp::Ordering::Greater => csi_n_cost(y - cy),
        std::cmp::Ordering::Less => csi_n_cost(cy - y),
    }
}

fn vert<O: Out>(out: &mut O, cy: u16, y: u16) {
    match y.cmp(&cy) {
        std::cmp::Ordering::Equal => {}
        // VT (0x0B), never LF: xterm, iTerm2, tmux and the vt100 crate treat VT exactly as
        // LF (one row down, column kept, with LNM reset), but a '\n' in the frame would make
        // stdout's `LineWriter` split it into two writes (RT-R15). Only used to move to a row
        // below the cursor, so it never scrolls.
        std::cmp::Ordering::Greater if y - cy <= MAX_VT => {
            for _ in cy..y {
                out.byte(0x0B);
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
                let s = i % styles;
                let (r, g) = (
                    u8::try_from(s % 256).expect("byte"),
                    u8::try_from(s / 256).expect("byte"),
                );
                let p = pen(Color::Rgb(r, g, 0), Color::Reset, Modifier::empty());
                Pending::new((0, 0, 1), p, b"x", &mut Vec::new())
            })
            .collect()
    }

    #[test]
    fn rnd_r17_grouping_is_tried_only_when_styles_repeat() {
        assert!(!worth_grouping(&mut []), "nothing to reorder");
        assert!(!worth_grouping(&mut cells(1, 1)));
        assert!(worth_grouping(&mut cells(1, 2)));
        // Small frames: up to half as many styles as cells, at most STYLE_CAP.
        assert!(worth_grouping(&mut cells(50, 100)));
        assert!(!worth_grouping(&mut cells(51, 100)));
        assert!(worth_grouping(&mut cells(STYLE_CAP, GROUP_ANY_CELLS)));
        assert!(!worth_grouping(&mut cells(STYLE_CAP + 1, GROUP_ANY_CELLS)));
        // Big frames: only in a few styles.
        assert!(worth_grouping(&mut cells(FEW_STYLES, 20_000)));
        assert!(!worth_grouping(&mut cells(FEW_STYLES + 1, 20_000)));
        // Any wide symbol keeps row-major order.
        let mut wide = cells(1, 10);
        wide[3].width = 2;
        assert!(!worth_grouping(&mut wide));
    }

    #[test]
    fn rnd_r17_grouping_keeps_row_major_order_within_each_style() {
        let mut c = cells(7, 300);
        assert!(worth_grouping(&mut c));
        let mut order = Vec::new();
        group(&c, &mut order);
        let mut seen = order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..300).collect::<Vec<u32>>(), "a permutation");
        for w in order.windows(2) {
            let (a, b) = (&c[w[0] as usize], &c[w[1] as usize]);
            if a.pen == b.pen {
                assert!(w[0] < w[1], "row-major within a style");
            }
        }
        let runs = order
            .windows(2)
            .filter(|w| c[w[0] as usize].pen != c[w[1] as usize].pen)
            .count();
        assert_eq!(runs, 6, "each style is one contiguous group");
    }

    #[test]
    fn rnd_r17_ambiguous_width_symbols_make_the_next_far_move_absolute() {
        assert!(unambiguous(b"x") && unambiguous(" ".as_bytes()));
        assert!(unambiguous("⠀".as_bytes()) && unambiguous("⣿".as_bytes()));
        for s in ["▀", "▄", "★", "·", "→", "∞", "█"] {
            assert!(!unambiguous(s.as_bytes()), "{s}");
        }
        // After '▀' at (0,0): the adjacent cell needs no move, a gap needs a CUP.
        let syms = "▀▀▀".as_bytes();
        let at = |x: u16, i: usize| {
            Pending::new(
                (x, 0, 1),
                Pen::DEFAULT,
                &syms[i * 3..i * 3 + 3],
                &mut Vec::new(),
            )
        };
        let mut out = Vec::new();
        let mut state = State {
            pen: Pen::DEFAULT,
            cursor: Cursor::Unknown,
            width: 40,
        };
        encode(
            &mut out,
            &mut state,
            [at(0, 0), at(1, 1), at(5, 2)].iter(),
            syms,
        );
        assert_eq!(
            out,
            "\x1b[H▀▀\x1b[6G▀".as_bytes(),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
        assert_eq!(state.cursor, Cursor::Loose(6, 0));
    }

    #[test]
    fn rnd_r17_symbols_up_to_four_bytes_are_inline_and_longer_ones_go_to_the_arena() {
        let mut arena = Vec::new();
        for s in ["x", "⣿", "▀", "\u{1F600}"] {
            let p = Pending::new((0, 0, 1), Pen::DEFAULT, s.as_bytes(), &mut arena);
            assert_eq!(p.symbol(&arena), s.as_bytes());
        }
        assert!(arena.is_empty(), "one char each: inline");
        let long = ["e\u{301}\u{302}", "\u{1F469}\u{200D}\u{1F52C}"];
        let ps: Vec<Pending> = long
            .iter()
            .map(|s| Pending::new((0, 0, 2), Pen::DEFAULT, s.as_bytes(), &mut arena))
            .collect();
        for (p, s) in ps.iter().zip(long) {
            assert_eq!(p.symbol(&arena), s.as_bytes());
        }
    }

    #[test]
    fn rnd_r17_rows_beyond_u16_saturate_instead_of_overflowing() {
        let mut out = Vec::new();
        cup(&mut out, 0, u16::MAX);
        assert_eq!(out, b"\x1b[65535H");
        assert_eq!(cup_cost(0, u16::MAX), out.len());
    }
}
