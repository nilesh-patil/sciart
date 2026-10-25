# ADR-0034: A web gallery and a docs site

- **Status**: Accepted
- **Date**: 2026-11-26

## Context

sciart is art, and until 1.0 the only way to see it was to install it. The README shows
stills and a few recorded clips; the reference documentation lives in specs written for
maintainers. Someone deciding whether to install wants to see scenes move and to read how to
use the program, without a terminal.

The scenes are pure: a seed, fixed steps and a canvas of cells, with no clock and no I/O
(GEN-R7). The capture renderer already turns a canvas into pixels without a terminal
(ADR-0025). So the same code can run in a browser.

## Decision

- **A WebAssembly player.** A small crate in `web/`, outside the workspace, compiles the
  scenes and the capture renderer for the browser and exposes a `Player`. The page drives it
  from the animation frame and draws its pixels to a canvas. The browser is one more
  platform under the per-platform determinism promise (ADR-0029).
- **A static site.** A user guide written for users (not the specs), a gallery generated
  from the scene registry and the showcase manifest, and the live player. It is built by a
  pixi task into plain files.
- **Self-contained.** No scripts, fonts or images from other origins, no analytics.
- **Published by hand.** GitHub Pages is deployed by a manual run or a published release,
  never by a push.

## Consequences

- The render and capture crates gain feature gates so the browser build leaves out the
  terminal backend and the image encoders; the scene crates must not pull in an
  operating-system RNG.
- Frames in the browser may differ from a terminal's in the last bits of the mathematics;
  the page says which build it runs.
- The site shows 8×16 px cells, the recorder's look, not the visitor's terminal font.
- One more build target and one more workflow to keep green.

## Alternatives considered

- **Clips only**: no build target to maintain, and the site would show eighteen scenes, not
  eighty-five, with no way to change a seed or a theme.
- **A terminal emulator in the page running the real binary**: needs a WASI runtime and a
  terminal emulator in the browser for the same pictures.
- **A hosted documentation service**: third-party scripts and tracking on the project's
  front page.
- **The specs as the documentation**: they answer a maintainer's questions, not a user's.
