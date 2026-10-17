# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
