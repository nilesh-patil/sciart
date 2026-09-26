//! Dense 2-D fields for automata, lattices and PDEs (SPEC-001 CORE-R33..R36).
//!
//! One contiguous row-major `Vec<T>`: rows are cache-line friendly for the
//! `for y { for x { .. } }` sweeps every grid scene does, and the same layout as
//! [`Canvas`](crate::Canvas), so a grid maps onto the screen cell for cell.
//!
//! Coordinates are `i32` like the rest of the drawing API, so callers can probe
//! `x - 1` at the edge without casts; anything off-grid is reported as `None` /
//! `false`, never a panic (GEN-R4). Index arithmetic is done in `usize` after the
//! bounds check, because a `u16 × u16` area does not fit in `i32`.
//!
//! No `Index<(i32, i32)>` impl on purpose: `grid[(x - 1, y)]` reads naturally but
//! panics on the edge, which is exactly the GEN-R4 failure mode. [`Grid::get`]
//! plus `unwrap_or` is barely longer and cannot crash a scene.

use std::iter::FusedIterator;

use crate::Size;

/// A dense `width × height` field of `T`, stored row-major (CORE-R33).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid<T> {
    // `u16` like `Size`, but kept separately because a grid may be 0×0 while a
    // `Size` is always at least 1×1 (CORE-R1).
    width: u16,
    height: u16,
    cells: Vec<T>,
}

/// The 0×0 grid (CORE-R33). Does not allocate, so it is the natural initial value
/// for a scene field or a [`Grid::step`] scratch buffer before the first resize.
impl<T> Default for Grid<T> {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            cells: Vec::new(),
        }
    }
}

impl<T: Clone> Grid<T> {
    /// A `size` grid with every cell set to `fill` (CORE-R33).
    pub fn new(size: Size, fill: T) -> Self {
        Self {
            width: size.width(),
            height: size.height(),
            cells: vec![fill; size.area()],
        }
    }

    /// Set every cell to `v` (CORE-R33).
    pub fn fill(&mut self, v: T) {
        self.cells.fill(v);
    }

    /// Change the dimensions, keeping every cell whose `(x, y)` exists in both the
    /// old and the new size; new cells become `fill` (CORE-R34).
    ///
    /// Works in place: surviving cells are moved to their new row-major slot by
    /// `swap`, which needs neither `Copy` nor extra clones. The buffer only grows
    /// (never shrinks), so a shrink-then-grow on terminal resize does not allocate.
    pub fn resize(&mut self, size: Size, fill: T) {
        let (ow, oh) = (usize::from(self.width), usize::from(self.height));
        let (nw, nh) = (usize::from(size.width()), usize::from(size.height()));
        if (ow, oh) == (nw, nh) {
            return;
        }
        let (mw, mh) = (ow.min(nw), oh.min(nh));
        let new_len = nw * nh;

        // Make room first so every destination index is valid during the moves.
        if new_len > self.cells.len() {
            self.cells.resize(new_len, fill.clone());
        }

        // Move each kept cell from `y*ow + x` to `y*nw + x`. The iteration order
        // guarantees a source slot is never clobbered before it is read: when rows
        // get narrower every destination is <= its source, so walk forwards; when
        // they get wider every destination is >= its source, so walk backwards.
        // The value swapped *into* a source slot is garbage that is later either
        // overwritten by `fill` below or truncated away.
        let mut shift = |y: usize, x: usize| self.cells.swap(y * ow + x, y * nw + x);
        if nw <= ow {
            (0..mh).for_each(|y| (0..mw).for_each(|x| shift(y, x)));
        } else {
            (0..mh)
                .rev()
                .for_each(|y| (0..mw).rev().for_each(|x| shift(y, x)));
        }

        self.cells.truncate(new_len);
        // Everything outside the kept `mw × mh` rectangle is new.
        if mw < nw {
            for y in 0..mh {
                self.cells[y * nw + mw..(y + 1) * nw].fill(fill.clone());
            }
        }
        self.cells[mh * nw..].fill(fill);
        self.width = size.width();
        self.height = size.height();
    }
}

impl<T> Grid<T> {
    /// The grid's dimensions (CORE-R33).
    ///
    /// A `Size` cannot be 0×0 (CORE-R1), so for the empty [`Grid::default`] this
    /// reports 1×1; use [`width`](Self::width)/[`height`](Self::height) or
    /// [`is_empty`](Self::is_empty) when the grid may be empty.
    #[inline]
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Number of columns; `0` only for the empty grid (CORE-R33).
    #[inline]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Number of rows; `0` only for the empty grid (CORE-R33).
    #[inline]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Number of cells, `width * height`.
    #[inline]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Row-major buffer index of `(x, y)`, or `None` off-grid.
    ///
    /// The bounds check happens in `i32` (so negative and huge values are simply
    /// out of range) and only the validated values are widened to `usize` for the
    /// multiply, which cannot overflow: `(65535 - 1) * 65535 + 65534 < usize::MAX`.
    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let in_x = (0..i32::from(self.width)).contains(&x);
        let in_y = (0..i32::from(self.height)).contains(&y);
        (in_x && in_y).then(|| y as usize * usize::from(self.width) + x as usize)
    }

    /// The cell at `(x, y)`, or `None` off-grid; never panics (CORE-R33).
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<&T> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// Mutable access to the cell at `(x, y)`, or `None` off-grid (CORE-R33).
    #[inline]
    pub fn get_mut(&mut self, x: i32, y: i32) -> Option<&mut T> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    /// Store `v` at `(x, y)`; returns `false` (dropping `v`) off-grid (CORE-R33).
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, v: T) -> bool {
        match self.get_mut(x, y) {
            Some(cell) => {
                *cell = v;
                true
            }
            None => false,
        }
    }

    /// Every cell with its coordinates, in row-major order (CORE-R33).
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            cells: self.cells.iter(),
            width: i32::from(self.width),
            x: 0,
            y: 0,
        }
    }

    /// The row-major buffer: cell `(x, y)` is at `y * width + x` (CORE-R33).
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.cells
    }

    /// Mutable row-major buffer, for bulk per-cell updates that need no coordinates.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.cells
    }

    /// In-grid neighbour coordinates of `(x, y)`, excluding `(x, y)` itself
    /// (CORE-R35). Yields nothing if `(x, y)` is off-grid.
    ///
    /// Order follows the offset table (row-major around the cell). With
    /// [`Edges::Wrap`] on a grid narrower or shorter than 3, several offsets can
    /// land on the same cell or on the cell itself; those are yielded once / skipped.
    /// Allocation-free: a fixed offset table and an 8-slot inline "seen" list.
    pub fn neighbors(&self, x: i32, y: i32, hood: Hood, edges: Edges) -> Neighbors {
        let offsets: &'static [(i32, i32)] = match hood {
            Hood::VonNeumann => &VON_NEUMANN,
            Hood::Moore => &MOORE,
        };
        let (w, h) = (i32::from(self.width), i32::from(self.height));
        Neighbors {
            x,
            y,
            w,
            h,
            edges,
            // An off-grid centre has no neighbourhood; starting exhausted also
            // keeps the arithmetic below within `-1..=w` (no overflow possible).
            offsets: if self.index(x, y).is_some() {
                offsets
            } else {
                &[]
            },
            // Only small wrapped grids can alias; skip the de-dup work otherwise.
            dedup: edges == Edges::Wrap && (w < 3 || h < 3),
            seen: [(0, 0); 8],
            n_seen: 0,
        }
    }

    /// One synchronous update: every cell of the next generation is
    /// `rule(&old, x, y)`, computed from the old grid only, then the buffers are
    /// swapped (CORE-R36). Afterwards `scratch` holds the previous generation.
    ///
    /// `scratch` is rebuilt to this grid's size by clear-and-push, which keeps its
    /// capacity, so after the first call the two buffers just ping-pong with no
    /// allocation. This also means `T` needs no `Clone`/`Default` for the scratch.
    pub fn step(&mut self, scratch: &mut Grid<T>, mut rule: impl FnMut(&Grid<T>, i32, i32) -> T) {
        scratch.cells.clear();
        scratch.cells.reserve_exact(self.cells.len());
        for y in 0..i32::from(self.height) {
            for x in 0..i32::from(self.width) {
                scratch.cells.push(rule(self, x, y));
            }
        }
        scratch.width = self.width;
        scratch.height = self.height;
        std::mem::swap(self, scratch);
    }
}

/// Which cells count as neighbours (CORE-R35).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hood {
    /// The 4 orthogonal cells.
    VonNeumann,
    /// The 8 orthogonal and diagonal cells.
    Moore,
}

/// What happens at the border (CORE-R35).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edges {
    /// Off-grid neighbours are dropped.
    Clip,
    /// The grid is a torus: coordinates wrap around both axes.
    Wrap,
}

// Row-major around the centre, so iteration order is stable and predictable.
const VON_NEUMANN: [(i32, i32); 4] = [(0, -1), (-1, 0), (1, 0), (0, 1)];
const MOORE: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Row-major iterator over `((x, y), &cell)`, from [`Grid::iter`] (CORE-R33).
///
/// Tracks `x`/`y` incrementally instead of dividing the flat index each step.
#[derive(Debug, Clone)]
pub struct Iter<'a, T> {
    cells: std::slice::Iter<'a, T>,
    width: i32,
    x: i32,
    y: i32,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = ((i32, i32), &'a T);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cell = self.cells.next()?;
        let pos = (self.x, self.y);
        self.x += 1;
        if self.x == self.width {
            self.x = 0;
            self.y += 1;
        }
        Some((pos, cell))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.cells.size_hint()
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}
impl<T> FusedIterator for Iter<'_, T> {}

/// Neighbour coordinates of one cell, from [`Grid::neighbors`] (CORE-R35).
///
/// Holds no borrow of the grid, so a caller can iterate neighbours while mutating
/// the grid (e.g. to scatter into them).
#[derive(Debug, Clone)]
pub struct Neighbors {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    edges: Edges,
    offsets: &'static [(i32, i32)],
    dedup: bool,
    seen: [(i32, i32); 8],
    n_seen: usize,
}

impl Iterator for Neighbors {
    type Item = (i32, i32);

    fn next(&mut self) -> Option<(i32, i32)> {
        while let Some((&(dx, dy), rest)) = self.offsets.split_first() {
            self.offsets = rest;
            // The centre is in-grid (checked at construction), so these are in
            // `-1..=w` / `-1..=h` and cannot overflow.
            let (mut nx, mut ny) = (self.x + dx, self.y + dy);
            match self.edges {
                Edges::Clip => {
                    if !(0..self.w).contains(&nx) || !(0..self.h).contains(&ny) {
                        continue;
                    }
                }
                Edges::Wrap => {
                    nx = nx.rem_euclid(self.w);
                    ny = ny.rem_euclid(self.h);
                }
            }
            if self.dedup {
                let p = (nx, ny);
                if p == (self.x, self.y) || self.seen[..self.n_seen].contains(&p) {
                    continue;
                }
                self.seen[self.n_seen] = p;
                self.n_seen += 1;
            }
            return Some((nx, ny));
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.offsets.len()))
    }
}

impl FusedIterator for Neighbors {}
