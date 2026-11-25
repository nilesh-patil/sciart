# ADR-0030: Changed-cell paint and diff; event-driven pause

- **Status**: Accepted
- **Date**: 2026-11-25

## Context

M14 and M22 cut the scenes' own cost until the fixed cost of the terminal pipeline became
the larger part. Measured on the release profile (M4 Max, 85 scenes, 30 FPS):

| Size | Scene | Paint | Diff + encode | Pipeline |
|---|---|---|---|---|
| 200×50 | 293 µs | 96 µs | 143 µs | 532 µs |
| 500×150 | 777 µs | 699 µs | 965 µs | 2441 µs |

Paint and diff cost about 20 ns per cell whether or not the cell changed. A scene that
changes nine cells in a frame still pays about 200 µs at 200×50, and at 500×150 a scene
that needs 21 µs for itself pays 1.4 ms for the pipeline. The painter maps every canvas cell
to a ratatui cell, and ratatui's buffer diff then compares every cell again.

A paused scene, or the help page, has nothing to draw, yet the loop still wakes at the
low-power rate, repaints and diffs an unchanged screen.

## Decision

**Changed cells only.** `Stage` already keeps the previous canvas. The painter compares the
new canvas with it and visits only the cells that differ; the encoder receives those cells
and emits the same bytes as before. The first frame and anything that invalidates the whole
screen (resize, theme, colour mode, background, HUD, title, help) take the full-area path.
The full-area path stays in the code as the reference the new path is tested against, byte
for byte, for every scene.

**Block while there is nothing to draw.** Paused or showing help, once a frame's diff is
empty, the loop waits for the next terminal event or signal, with a timeout only long
enough to poll the keep-awake backend.

## Consequences

- Expected: about a third less pipeline time at 200×50 and more than half at 500×150; the
  quiet scenes gain most. The merge records the measured numbers.
- Frames, bytes and the M11 look do not change: the output stream is identical by test.
- The renderer owns its diff instead of relying on ratatui's `Buffer::diff`; ratatui remains
  the buffer type and the widget layer (HUD, help).
- Low-power mode still steps the simulation at full speed (ADR-0003); only pause and help
  stop the loop.

## Alternatives considered

- **Dirty rectangles reported by scenes**: scenes would have to track what they drew; the
  comparison against the previous canvas needs nothing from them.
- **Keep the full diff, make it faster (SIMD, row hashes)**: still pays per cell of screen.
- **Slow the simulation in low power**: saves the scene cost too, but changes what the user
  sees when they come back; rejected by ADR-0003.
