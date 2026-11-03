# ADR-0026: Colour capability, `NO_COLOR`, and a colour-blind-safe theme

**Status**: Accepted (2026-10-18)

## Context

sciart's palette is its art: themes are semantic ink tables mapped to 24-bit RGB, and the
frame encoder emits `38;2;r;g;b` unconditionally. That is correct on a truecolor terminal and
wrong everywhere else:

- a 256-colour terminal (tmux, some ssh clients, older emulators) shows whatever its own
  nearest-colour approximation produces, with no control over the result;
- a user who sets `NO_COLOR=1` (the cross-tool convention, <https://no-color.org>) still gets
  full colour, unlike every other terminal tool on their machine;
- the default `Hot` red / `Cold` green pair is the most common confusion pair for
  deuteranopia and protanopia, and the roadmap has carried a colour-blind-safe variant since
  M19/M20.

The renderer already has `Color::Indexed` support in the encoder and a pure `Theme::color`
(RND-R1), so this is a paint-time mapping, not a palette rewrite.

## Decision

- Add a colour mode: `auto`, `truecolor`, `256`, `mono` (`--color`, config `color`,
  CFG-R10). `auto` is truecolor unless `NO_COLOR` is non-empty, which selects `mono`; an
  explicit `--color` always wins. `NO_COLOR` prints one warning line unless the mode was
  explicit.
- `mono` reuses `Theme::Mono` (no new palette); `truecolor` is byte-identical to the M22
  encoder; `256` quantizes every painted RGB to the nearest xterm-256 index through a
  precomputed 32³ RGB5 lookup table (standard 6×6×6 cube + 24 greys), built once, and the
  existing encoder emits `38;5;n`/`48;5;n`. Hysteresis stores the emitted (quantized) colour.
- Do **not** guess the colour depth from `TERM`/`COLORTERM`: modern terminals advertise
  poorly and a wrong guess degrades the art. `auto` therefore means "truecolor unless
  `NO_COLOR`"; users on a 256-colour terminal choose `--color 256` explicitly.
- Add a fifth theme, `colorblind`, built from the Okabe–Ito palette: `Hot` vermillion
  `#D55E00`, `Cold` sky blue `#56B4E9`, `Accent` reddish purple `#CC79A7`, with a
  viridis-like `Seq` and a blue↔vermillion `Div`. It cycles after `aurora` and is selected by
  `--theme colorblind` / `theme = "colorblind"`.
- Keep `Theme::color` pure and RGB; the colour mode is a paint/encoder concern. `paint()` (the
  stateless reference used by tests, metrics and probes) stays truecolor.
- Review the theme on the five busiest complementary-pair scenes at 80×24 and 200×50 before
  calling it done.

## Consequences

- **Positive**: the tool respects `NO_COLOR`; 256-colour terminals get a deterministic,
  art-directed result; a colour-blind-safe option exists without changing the default; the
  encoder already supports indexed colours.
- **Negative**: a second install/flag dimension, a 32 KiB LUT, and a new theme to review and
  document. Indexed output is lower-fidelity by definition.
- **Neutral**: the default (`auto` + astro) is unchanged on a truecolor terminal without
  `NO_COLOR`; snapshots and art metrics (which use `paint`) are unchanged.

## Alternatives considered

- **Detect from `TERM`/`COLORTERM`.** Rejected: unreliable across tmux/ssh and a wrong guess
  silently degrades every frame; explicit `--color 256` is predictable.
- **Let crossterm approximate colours.** It would put the mapping in a dependency, outside
  the tested theme/painter, and vary by crossterm version.
- **A 16-colour mode.** Far too lossy for the art; not worth a mode.
- **A separate binary for `NO_COLOR` / a11y.** Worse than a flag and a theme.

## References

- RND-R18, RND-R19 in [SPEC-003](../specs/SPEC-003-runtime-render.md)
- CFG-R10 in [SPEC-005](../specs/SPEC-005-cli-config.md)
- M23 improvement items IMP-10, IMP-11 ([roadmap](../ROADMAP.md))
