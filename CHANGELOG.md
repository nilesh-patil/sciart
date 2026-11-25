# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A library: the `sciart` crate now exposes a small, stable API to run a scene without a
  terminal, list the scenes, render a frame as text, ANSI or PNG, and embed a scene in a
  ratatui application (`SceneWidget`). Use it with `default-features = false`
  ([ADR-0032](docs/adr/0032-library-facade.md), SPEC-007).
- `sciart frame <scene>` prints one frame as ANSI text, for a login message, a tmux pane or
  a CI log (CLI-R13).
- `sciart record` is part of the default build; `--no-default-features --features cli`
  builds without the image encoders (CLI-R15).

- `--for <duration>` ends a run by itself (`90s`, `25m`, `1.5h`), also while paused; the HUD
  shows the time left (CLI-R12, RT-R23).
- `sciart while -- <command>` shows the art and keeps the machine awake until the command
  exits, then replays its output and exits with its status
  ([ADR-0031](docs/adr/0031-scriptable-modes.md), CLI-R14).
- `sciart completions <shell>` and `sciart man [subcommand]` print completion scripts and
  manual pages generated from the command line's own definition (CLI-R17).

- Keep-awake on Linux: `--keep-awake inhibit` holds a `systemd-inhibit` (or
  `elogind-inhibit`) lock for as long as sciart runs, and is the default for a local Linux
  session; over SSH the default is `off` ([ADR-0028](docs/adr/0028-linux-inhibitor-helper.md),
  KA-R7..R9).
- `SIGTERM` and `SIGHUP` restore the terminal and exit with status 143 or 129 instead of
  leaving the shell in raw mode (RT-R20).
- A one-line hint on exit when the terminal is unlikely to render truecolor braille (the
  Linux console, Apple Terminal without `COLORTERM`, `screen`/`tmux`), unless a colour mode
  was chosen explicitly (RND-R21).
- Animated captures in the README and the gallery: 18 clips recorded by `sciart record`
  from the manifest `docs/showcase.toml`; `pixi run rs-showcase` regenerates them and
  `rs-showcase-check` fails on a missing, stale or unlisted clip (WEB-R1, WEB-R2).
- `sciart record --start <seconds>` simulates that long before the first frame, so a
  recording can open on a scene's best moment (CLI-R8).

### Fixed

- Recordings and gallery frames draw every glyph the scenes use (`•`, `●`, `☉`, arrows,
  planet symbols, superscripts and more) instead of an ASCII stand-in; 22 gallery frames
  were re-rendered (CLI-R18).

### Changed

- **Breaking**: `sciart list --json` is now a versioned object, `{"schema": 1, "scenes": [...]}`,
  and each scene also carries `category_label`, `title`, `captions` and `aliases`; fields
  are only added from here on (CLI-R16). Scripts that read the old array need `.scenes`.
- The terminal pipeline paints, diffs and encodes only the cells that changed since the last
  frame, with identical output: about a third less CPU per frame at 200×50 and more than
  half at 500×150 ([ADR-0030](docs/adr/0030-changed-cell-pipeline.md), RND-R22).
- A paused scene and the help page wait for the next key instead of redrawing five times a
  second (RT-R21).
- Smaller and lighter to build: unused `ratatui`, `clap` and `toml` features are off (108 →
  93 crates) and the release binary is stripped (3.64 → 3.27 MB), both held by
  `pixi run rs-size` ([ADR-0035](docs/adr/0035-release-profile-and-dependency-trim.md), RT-R22).
- Platform tiers: macOS arm64 and Linux x86_64 run the whole suite with pinned frames;
  Linux aarch64 runs it without the frame-pinning suites; frames are deterministic per
  platform ([ADR-0029](docs/adr/0029-platform-tiers-and-determinism.md), TEST-R13).
- The pixi workspace is named `sciart` and also covers Intel macOS and Linux.

## [0.6.0] - 2026-10-18

### Added

- `--background <opaque|terminal>` and the `background` config key. `opaque` is the default:
  the live painter fills every cell with black, so a terminal wallpaper, transparency or
  selection band cannot show through the art ([ADR-0027](docs/adr/0027-opaque-background.md),
  RND-R20, CLI-R11, CFG-R11).

### Changed

- The help page also paints the opaque background; `--background terminal` keeps the previous
  terminal-default behaviour for users who want transparency.

## [0.5.0] - 2026-10-18

### Changed

- Art polish pass over the eleven scenes that were below 8/10: `io_volcanoes` now has a
  living eruption cycle, `rainbow` a continuous dithered bow, `motility_clustering` a
  populated opening, `firefly_sync` soft glowing points, `meteor_shower` a structured
  Milky Way, `orbital_rendezvous` larger craft, `comet_breakup` visible impacts,
  `comet_tails` a contained composition, `total_solar_eclipse` a stippled corona,
  `noctilucent_clouds` ice-blue clouds, and `catalytic_surface` graded reaction fronts.
  Ten of eleven now score ≥ 8/10; `catalytic_surface`'s late uniform-CO beat is tracked as
  a spec decision ([B-92](docs/BACKLOG.md)).
- Re-recorded the seed-42 snapshots and re-seeded the art bands for the reviewed looks.

### Added

- A 85-row science-claims audit ([docs/science/claims.md](docs/science/claims.md), 20
  audited) and the standing 15-minute milestone review
  ([docs/REVIEW.md](docs/REVIEW.md)).

## [0.4.0] - 2026-10-18

### Changed

- Split `SPEC-002` into the rules/index plus five category scene files, with a generated
  scene index checked by `scripts/spec_index.py` (GEN-R15).
- Added `docs/BACKLOG.md` (91 triaged follow-ups) and `docs/README.md`; the roadmap now
  points at the backlog.
- Added scheduled `cargo-fuzz` targets and made the `cargo-deny` advisory policy explicit.

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
