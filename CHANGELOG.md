# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-10-18

### Added

- Colour capability: `--color auto|truecolor|256|mono` and the `color` config key. `256`
  quantizes to the xterm-256 palette deterministically; `mono` uses the greyscale theme.
- `NO_COLOR` support: a non-empty `NO_COLOR` selects mono automatically with one warning;
  an explicit `--color truecolor|256|mono` overrides it.
- The colour-blind-safe `colorblind` theme (Okabe–Ito palette, viridis-like fields and a
  blue ↔ vermillion diverging ramp), selectable with `--theme colorblind` and cycled by `t`
  after `aurora` (ADR-0026).

## [0.2.0] - 2026-10-18

### Added

- `sciart record` (behind the `record` feature): render a scene headlessly to a looping GIF,
  a PNG frame sequence, or an asciicast v2 stream, with deterministic frame timing. The
  renderer is shared with the preview tool through the new `sciart-capture` crate (ADR-0025).
- In-app help overlay on `?`: every binding, the current scene, title and category, the
  config path and the project URL; `Esc` closes it instead of quitting (`RT-R18`).
- Favourites: `f` toggles the current scene, `v` shows only favourites, seeded from the new
  config `favorites` list (`RT-R19`).
- Category navigation (`[`/`]`), shuffle (`s`), scene filters (`--category`, `--only`,
  `--exclude`) and a simulation speed multiplier (`--speed`, config `speed`) (`RT-R19`,
  `CLI-R9`, `CFG-R9`).
- `sciart list --json`, `--categories` and `--plain`; a terminal gets an aligned table
  (`CLI-R10`).

## [0.1.0] - 2026-10-18

First public release.

### Added

- 85 scientific and astronomy scenes across 11 categories: Deep Cosmos, Sun & Planets,
  Earth & Sky, Voyages, Forces & Waves, Reactions, Living Systems, States of Matter,
  Physics of Life, Molecules of Life and Life from Matter.
- Four themes: `astro` (default), `rainbow`, `mono` and `aurora`, cycled with `t`.
- Keep-awake backends: `caffeinate` on macOS (default), an opt-in `mouse` jiggler behind the
  `mouse` cargo feature, and `off`.
- A HUD showing the science title, measured FPS, theme, seed, keep-awake state, low-power and
  pause flags.
- TOML configuration at `~/.config/sciart/config.toml` (or
  `$XDG_CONFIG_HOME/sciart/config.toml`), with command-line flags overriding the file.
- Deterministic seeded runs: the same seed reproduces the same frames, guarded by contract and
  snapshot tests.
- 30 FPS terminal rendering (configurable 1–120), Unicode glyphs with an ASCII fallback,
  half-block pixels and braille sub-cell art.
- Keyboard control: next/previous scene, pause, reset, HUD, theme and quit.
- A low-power draw rate after two idle minutes (or while the terminal is unfocused), with
  simulation speed preserved.

### Changed

- Lean build and runtime (M22, ADR-0023): tests run under `cargo nextest` on an optimised test
  profile; a separate `ci` profile builds the snapshot and cost probes; sccache backs the build
  tasks; `sciart-generators` split into a shared-models crate and five scene crates behind a
  registry facade; an idle-aware frame loop and a shorter terminal encoder cut CPU and bytes
  per frame; the low-power idle default dropped from 10 minutes to 2; per-scene memory
  ceilings were lowered to their measured values plus a margin. Visuals are unchanged.

[Unreleased]: https://github.com/nilesh-patil/sciart/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nilesh-patil/sciart/releases/tag/v0.1.0
