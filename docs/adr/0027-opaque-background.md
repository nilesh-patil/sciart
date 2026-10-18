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

The **app's** live painter defaults to an **opaque black** background (`#000000`) for every
cell, including cells whose ink is `Role::Background`. A new public `Background` enum selects
`Opaque` or `Terminal`; `--background opaque|terminal` (CLI-R11) and
`background = "opaque"|"terminal"` (CFG-R11) expose it. `Terminal` restores the old
`Color::Reset` behaviour for users who want terminal transparency or their own
background.

The **library** default stays `Background::Terminal`: `paint()`, `Painter::new()` and
`Painter::indexed()` callers (previews, capture/record, cost probes, encoder tests) keep the
RND-R3/RND-R7 stream byte-for-byte. Only `Screen` opts into `Opaque` from `Settings`, which
defaults to `opaque`. This keeps the visual fix in the live app without silently changing
headless output or the art-metric bands.

Explicit per-cell background inks (half blocks, `Canvas::put_bg`) are preserved. The
indexed painter emits the nearest xterm-256 black. Recordings and previews already render
an opaque black image, so they need no new flag.

## Alternatives

- **Terminal default only (status quo)**: no new bytes and keeps transparency, but the
  band and any wallpaper/selection artefact remain; different terminals show different
  frames.
- **Theme background colour**: more cohesive per theme, but still an explicit colour and
  would change every theme's ground; the reported band is not a theme colour. Pure black
  matches the art's existing terminal-black ground and the PNG capture pipeline's opaque
  image.
- **Clear the screen every frame / force a full repaint**: more bytes, does not hide a
  terminal selection overlay, and does not address transparency; only useful if a stale
  diff row were the cause, which the pty capture ruled out.
- **Fix it in the terminal profile**: works for one user, not for the next; the app's
  default should not depend on the terminal's wallpaper.

## Consequences

- The live frame is deterministic and does not let terminal wallpaper, transparency or
  selection highlights leak into the art by default; the PNG capture pipeline was already
  opaque.
- Users who want transparency set `background = "terminal"`.
- The app's first frame emits one extra background SGR; the encoder groups equal pens, so
  the cost is not one SGR per cell. Headless callers are unchanged.
- RND-R3/RND-R7 keep their library wording; the app's `Background::Opaque` mode overrides
  `Role::Background` at the `Screen`, and `Background::Terminal` is the library default.
  RND-R15 is unchanged: a blank still has no foreground. New tests: RND-R20 (paint),
  CLI-R11/CFG-R11 (CLI/config), and the pty smoke test.
- `record` remains opaque black in every mode; the flag is terminal-only.
