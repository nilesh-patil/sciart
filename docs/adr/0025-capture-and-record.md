# ADR-0025: Headless capture and `sciart record`

**Status**: Accepted (2026-10-18)

## Context

`examples/preview` already renders a scene's canvas through the real `Theme` into a PNG
(8×16 px cells, `font8x8` glyphs, exact braille/half-block drawing). It is a review tool: one
static frame per `--t`, run from a Cargo example.

The project now needs motion outside the terminal: the README hero, the gallery, a release
announcement, and a way to review a scene's timing without a terminal. A GIF (or a PNG
sequence for a video) is the useful artifact. Doing this inside `sciart` by scraping the
event loop would be fragile (terminal size, pacer, HUD), and shelling out to `ffmpeg` would
make the tool depend on a system binary.

## Decision

- Add a `sciart-capture` crate (`publish = false`) with a headless pipeline:
  `Stage` (seeded, sized) → fixed 1/60 s steps → `render` → `Painter` → RGBA (the
  `preview` renderer) → GIF, PNG sequence, or asciicast v2.
- `sciart record [SCENE] --out <PATH>` (CLI-R8) is compiled behind the `record` cargo
  feature. The default binary carries no image/GIF dependencies; installs that want the
  command use `--features record` (the README documents it). A build without the feature
  reports that and exits 2.
- Timing is exact and deterministic: frame *k* is the scene after exactly *k* / fps
  simulated seconds; `--fps` must divide 60, so each frame lands on whole fixed steps.
  Frame 0 must equal `rs-preview` at `t = 0` through the same renderer.
- Formats: `gif` (default, looping, 256-colour palette), `png` (a `frame-000.png` sequence in
  a directory), `asciicast` (v2, replayable, no dependency). Format is inferred from the
  output path when `--format` is omitted.
- Move the `preview` example onto `sciart-capture` so there is one renderer, not two. The
  example keeps its `--sheet` and `--swatches` modes.
- Limits are part of the contract: 5 s at 80×24 should produce a GIF under ~5 MB, and
  recording should not take more than about twice real time on an M-series Mac.

## Consequences

- **Positive**: the README/GIF workflow is reproducible from the binary itself; no terminal or
  `ffmpeg`; the frame timing is testable; one renderer for review and capture.
- **Negative**: a feature-gated subcommand is a second install variant to document; the
  capture crate adds `gif`, `png` and `font8x8` (already a dev-dependency) to the workspace.
- **Neutral**: `sciart record` never touches keep-awake or the runtime; it is a separate
  headless path over `Stage`.

## Alternatives considered

- **Asciicast only.** Small and dependency-free, and replayable in asciinema, but it cannot
  be embedded in a README and depends on the viewer's terminal.
- **PNG sequence + external `ffmpeg`.** Keeps the binary lean, but requires a system binary
  and a second documented step; kept as the `png` format for users who want video.
- **Capture inside the event loop.** Reuses the exact on-screen path but couples recording to
  a terminal, the pacer and the HUD, and is hard to make deterministic.

## References

- CLI-R8 in [SPEC-005](../specs/SPEC-005-cli-config.md)
- `crates/sciart-generators/examples/preview.rs` (the renderer being lifted)
- `docs/IMPROVEMENTS.md` IMP-07
