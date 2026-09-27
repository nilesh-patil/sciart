# ADR-0014: Science titles drawn centrally by `Stage`

- **Status**: Accepted
- **Date**: 2026-09-27

## Context
Viewers see a scene without knowing what it shows. A handful of scenes wrote their own names or
parameter readouts, each in its own position, ink and fallback, and several on the bottom-left.
Titles have to appear on every current and future scene, stay allocation-free (GEN-R11), work in
ASCII mode (GEN-R10), and be visible to snapshots, contract, alloc and bench suites.

## Decision
- **Data, not code, per scene.** `Meta.title` is a required `Phenomenon · mechanism` string, with
  width ≤ 32 cells and a phenomenon ≤ 20 cells; the phenomenon doubles as the short form. Scenes
  with acts list their captions in `Meta.captions` and return the live one from
  `Generator::caption()` (default `None`), which replaces the mechanism. All `&'static str`
  (CORE-R71, GEN-R12).
- **One drawing path.** `Stage::render` draws the title after the scene, so every consumer of
  `Stage` sees the same frame (CORE-R75). It sits on row `h − 1` from column 1, is fitted
  full → phenomenon → hidden (never clipped mid-word, hidden when `h < 6`), clears only its own
  cells plus a gutter and a pad, is two-tone (`Role::Text` 170/110) and dims to 90 after 8 s of
  scene time (CORE-R73, CORE-R74).
- **Narrow charset.** ASCII plus `·`, `–`, `á`, `é`, `ö`, each with a one-cell ASCII fallback
  (CORE-R72). No formulas.
- **One text owner per corner.** Scenes keep the title span free of text and right-align their
  readouts with `draw::label_right` (GEN-R13, CORE-R76).
- **HUD merge.** With the HUD on, the canvas title hides and the HUD's name slot shows the title
  instead of the id (RT-R12, RND-R9). Rainbow maps `Role::Text` to a fixed grey (RND-R8).
- **Controls.** Config `title = true|false` and `--no-title` (CFG-R6, CLI-R7); no runtime key.

## Consequences
+ A new scene cannot compile without a title, and the contract suite checks its content.
+ Titles cost at most ~34 `put`s per frame, with no allocation, and are covered by every suite.
− All snapshots change once when titles turn on; twelve scenes move their readouts.
− A title is one static string per act: live numbers stay in scene readouts, not in the title.

## Alternatives considered
Each scene drawing its own title (41 copies that drift, no compile-time guarantee); drawing it as
a ratatui widget in the binary (invisible to snapshots and alloc tests, and without the glyph
mode); a title row above the HUD (costs a row of art); a separate caption line; a runtime toggle
key (one more key and pty test for little use).
