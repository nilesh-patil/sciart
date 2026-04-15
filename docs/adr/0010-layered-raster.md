# ADR-0010: Layered sub-cell raster — braille dots and half-block pixels

- **Status**: Accepted
- **Date**: 2026-09-27

## Context
A cell holds one glyph with one foreground ink. Fine structure (attractor filaments, N-body
cores, aurora curtains, fluid vorticity) needs more resolution than 1 sample per cell.
`crystal_growth` already renders on a private braille lattice; every new scene would copy it.
Continuous fields also want real colour resolution, not only more dots.

## Decision
Add two reusable rasters to `sciart_core::raster` (CORE-R41, CORE-R42) plus a cell background
ink (CORE-R40):

- **`Dots`** — a 2×4 braille dot lattice per cell (square dots with `ASPECT = 2`). One ink per
  cell (the brightest dot wins). ASCII fallback: a density glyph by dot count.
- **`Pixels`** — two vertically stacked square pixels per cell rendered as `▀` with
  `fg = top`, `bg = bottom`. This needs `Cell.bg`, painted by the renderer (RND-R7). ASCII
  fallback is **scene-specific**: `blit` takes a closure, so a scene cannot forget it
  (GEN-R10).

Scenes pick one raster per region. Mixing both in the same cell is not supported, because a
cell has one glyph.

## Consequences
+ 8× (dots) or 2× with two colours (pixels) resolution, shared, allocation-free after resize.
+ `crystal_growth` migrates to `Dots`, which proves the module against an existing snapshot.
− `Cell` grows by one `Ink` (2 bytes). `Canvas::put` resets `bg`, so overdraw stays predictable.
− In ASCII mode (snapshots) half-block scenes are only as good as their fallback closure.

## Alternatives considered
Braille only (no colour resolution for fields); half-block only (no fine line art);
per-scene rasters (duplication, which is exactly what this ADR removes).
