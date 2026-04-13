# ADR-0002: ratatui + crossterm for terminal output

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
30 FPS full-screen output needs diffed writes (only changed cells), truecolor, resize/key/focus events, and reliable terminal restore.

## Decision
ratatui (buffer diffing, widgets for the HUD) on the crossterm backend. The canvas is painted cell-by-cell into ratatui's `Buffer`; ratatui is confined to `sciart-render` and the binary.

## Consequences
+ Mature stack; frames are cheap because of diffing.
− ratatui API churn between minor versions — pinned in `[workspace.dependencies]`.

## Alternatives considered
Raw crossterm (hand-rolled diffing); a `Renderer` trait (unnecessary: the pure `Canvas` already gives terminal-free tests).
