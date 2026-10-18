# ADR-0027: Opaque black background by default

**Status**: Accepted · **Date**: 2026-10-18 · **Milestone**: M24

## Context

A horizontal dark-blue band appeared across the art in a live terminal, at the same
screen row in every scene. It was not in the generator output: `rs-preview` and
`sciart record` produced clean frames, and a real pty capture decoded by `vt100` showed
every cell with the terminal-default background — the app emitted no background colour
at all. The band disappeared when the terminal profile used a solid black background.

`Role::Background` has meant "terminal default" since RND-R3/RND-R7. That is the right
choice for a transparent terminal, but it also lets terminal wallpaper, transparency,
selection highlights and cursor-line overlays show through the art. A default install
should be deterministic: the same frame should look the same in every terminal.

## Decision

The live painter defaults to an **opaque black** background (`#000000`) for every cell,
including cells whose ink is `Role::Background`. A new public `Background` enum selects
`Opaque` (default) or `Terminal`; `--background opaque|terminal` (CLI-R11) and
`background = "opaque"|"terminal"` (CFG-R11) expose it. `Terminal` restores the old
`Color::Reset` behaviour for users who want terminal transparency or their own
background.

Explicit per-cell background inks (half blocks, `Canvas::put_bg`) are preserved. The
indexed painter emits the nearest xterm-256 black. Recordings and previews already render
an opaque black image, so they need no new flag.

## Alternatives

- **Terminal default only (status quo)**: no new bytes and keeps transparency, but the
  band and any wallpaper/selection artefact remain; different terminals show different
  frames.
- **Theme background colour**: more cohesive per theme, but still an explicit colour and
  would change every theme's ground; the reported band is not a theme colour. Pure black
  matches the art's existing black ground and the previews.
- **Clear the screen every frame / force a full repaint**: more bytes, does not hide a
  terminal selection overlay, and does not address transparency; only useful if a stale
  diff row were the cause, which the pty capture ruled out.
- **Fix it in the terminal profile**: works for one user, not for the next; the app's
  default should not depend on the terminal's wallpaper.

## Consequences

- The live frame is deterministic and matches the PNG previews; terminal wallpaper,
  transparency and selection highlights cannot leak into the art by default.
- Users who want transparency set `background = "terminal"`.
- The first frame emits one extra SGR (black background) and each cell's background is
  explicit; the encoder already groups equal pens, so the per-frame cost is one SGR, not
  one per cell. Hysteresis and the RT-R15 byte budget are unchanged in practice.
- RND-R3/RND-R7 are amended for the default mode; `Background::Terminal` keeps their
  original wording. New tests: RND-R20 (paint), CLI-R11/CFG-R11 (CLI/config), and the
  pty smoke test in `crates/sciart/tests/pty_smoke.rs`.
- `record` remains opaque black in every mode; the flag is terminal-only.
