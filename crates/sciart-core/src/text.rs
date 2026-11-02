//! Allocation-free text formatting for `render` (SPEC-001 CORE-R49, GEN-R11).
//!
//! `format!` allocates a `String`, which GEN-R11 forbids in `render`. [`StackText`] is a
//! fixed-capacity buffer on the stack that implements [`core::fmt::Write`], so
//! `write!(buf, "T/Tc {:.2}", t)` formats a readout with no heap traffic.

use core::fmt;

/// A fixed-capacity UTF-8 string of at most `N` bytes, living inline (CORE-R49).
///
/// Writes that do not fit are truncated silently on a char boundary, and once a write
/// has been truncated every later write is dropped, so the contents are always a prefix
/// of everything written. A HUD label that overflows loses its tail, never its validity.
#[derive(Clone)]
pub struct StackText<const N: usize> {
    buf: [u8; N],
    len: usize,
    full: bool,
}

impl<const N: usize> StackText<N> {
    /// An empty buffer.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            full: false,
        }
    }

    /// The text written so far.
    #[inline]
    pub fn as_str(&self) -> &str {
        // `write_str` only ever copies whole chars of a `&str`, so the prefix is valid
        // UTF-8 and this never takes the fallback; it exists to stay panic- and
        // `unsafe`-free.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    /// Length in bytes (`≤ N`).
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Capacity in bytes.
    #[inline]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Empty the buffer (and accept writes again after a truncation).
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
        self.full = false;
    }
}

impl<const N: usize> Default for StackText<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Write for StackText<N> {
    /// Append as much of `s` as fits, cut on a char boundary; always `Ok`, so a
    /// truncated `write!` still formats (and drops) the remaining arguments quietly.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.full {
            return Ok(());
        }
        let room = N - self.len;
        let mut take = s.len().min(room);
        if take < s.len() {
            self.full = true;
            // `is_char_boundary(0)` is true, so this stops.
            while !s.is_char_boundary(take) {
                take -= 1;
            }
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

impl<const N: usize> fmt::Display for StackText<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> fmt::Debug for StackText<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl<const N: usize> AsRef<str> for StackText<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A fixed-capacity UTF-8 string of at most `N` bytes that *errors* on overflow
/// (CORE-R109): unlike [`StackText`] a write that does not fit returns [`fmt::Error`]
/// and leaves the buffer unchanged, so a caller can fall back to a shorter form
/// (`voyager_journey`'s readout fit ladder) instead of silently losing the tail.
#[derive(Clone, Debug)]
pub struct StackStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Default for StackStr<N> {
    fn default() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> StackStr<N> {
    /// The text written so far (whole `&str` pieces only, so always valid UTF-8).
    #[inline]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    /// Bytes written so far (width-fitting decisions compare it against the canvas).
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing has been written.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> fmt::Write for StackStr<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        if end > self.buf.len() {
            return Err(fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
