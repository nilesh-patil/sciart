# sciart

[![CI](https://github.com/nilesh-patil/sciart/actions/workflows/ci.yml/badge.svg)](https://github.com/nilesh-patil/sciart/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](rust-toolchain.toml)

Real-time generative science & astronomy art for the terminal: **85 animated scenes** drawn in
braille and half-block cells at 30 FPS — and it keeps your machine awake while you watch.

<p align="center">
  <img src="samples/current/aurora.png" alt="Aurora: electrons light the air — green curtains over a starry sky" width="880">
</p>

## Gallery

<p align="center">
  <a href="samples/current/black_hole.png"><img src="samples/current/black_hole.png" alt="Black hole: infalling gas glows" width="270"></a>
  <a href="samples/current/solar_system.png"><img src="samples/current/solar_system.png" alt="Solar system: farther planets go slower" width="270"></a>
  <a href="samples/current/saturn_rings.png"><img src="samples/current/saturn_rings.png" alt="Saturn rings: moons carve gaps" width="270"></a>
</p>

<p align="center"><sub>Black hole · Solar system · Saturn rings — <a href="docs/GALLERY.md">full gallery</a></sub></p>

## Install

Requires [Rust](https://rustup.rs) 1.88 or newer.

```sh
cargo install --git https://github.com/nilesh-patil/sciart sciart --locked
```

The default build includes the headless recorder (`sciart record`); add
`--no-default-features --features cli` for a binary without the image encoders.

Working on the project? Clone it and install through the pixi task instead:

```sh
git clone https://github.com/nilesh-patil/sciart
cd sciart
pixi run rs-install
```

Release tarballs and Homebrew formulae are planned but **not available yet**.

## Run

```sh
sciart                  # cycle all 85 scenes (the default `sim` mode)
sciart run aurora       # one scene, full-screen
sciart list             # every scene: name, category, description
sciart list --json      # the same catalogue for scripts
sciart record aurora --seconds 5 --out aurora.gif   # GIF, PNG frames or an asciicast
sciart frame aurora --t 5 --size 80x24              # one frame as ANSI text, for a MOTD or a log
```

85 scenes across **11 categories**. Five themes — `astro` (default), `rainbow`, `mono`,
`aurora` and the colour-blind-safe `colorblind` — cycle with `t`. The default frame rate is
30 FPS (`--fps 1..=120`).

## Keys

| Key | Action |
| --- | --- |
| `n` / `→` | next scene |
| `p` / `←` | previous scene |
| `space` | pause / resume |
| `r` | reset the current scene |
| `h` | toggle the HUD |
| `t` | cycle theme: astro → rainbow → mono → aurora → colorblind |
| `?` | help overlay (`Esc` closes it) |
| `f` | favourite the current scene |
| `v` | show only favourites |
| `[` / `]` | previous / next category |
| `s` | shuffle the cycle order |
| `q` / `Esc` / `Ctrl-C` | quit |

## Configure

The config file is `~/.config/sciart/config.toml` (or
`$XDG_CONFIG_HOME/sciart/config.toml`). Every key is optional; command-line flags override
the file. See [`docs/config.example.toml`](docs/config.example.toml) for every key, including
`favorites`, `speed`, `color` and the low-power `[low_power]` table.

## Colour

Truecolor is the default. `--color 256` quantizes every cell to the xterm-256 palette
(deterministic, for tmux and older terminals), and `--color mono` uses the greyscale theme.
`NO_COLOR=1` selects mono automatically (with one warning); an explicit
`--color truecolor|256|mono` overrides it.

For colour-blind viewers, `--theme colorblind` uses the Okabe–Ito palette: vermillion and
sky blue stand in for the red/green pair, and the field ramps run blue ↔ vermillion rather
than red ↔ green. `t` cycles to it after `aurora`.

## Platform

macOS and Linux are supported. Keep-awake uses `caffeinate` on macOS and, on Linux, a
`systemd-inhibit` (or `elogind-inhibit`) lock held for as long as sciart runs; over SSH it
defaults to `off`, since keeping the remote machine awake is rarely what you want. Choose
explicitly with `--keep-awake caffeinate|inhibit|mouse|off`. The optional `mouse` jiggle
backend is behind the `mouse` cargo feature and needs macOS Accessibility permission.

## Use it as a library

The `sciart` crate is also a library: run a scene without a terminal, list the scenes, turn a
frame into text, ANSI or a PNG, or draw a scene inside your own ratatui application. **Turn
the default features off**, or you also get the command line's dependencies:

```toml
[dependencies]
sciart = { git = "https://github.com/nilesh-patil/sciart", default-features = false, features = ["png", "widget"] }
```

```rust
use sciart::{Options, Stage, Theme};

let mut stage = Stage::new("aurora", 80, 24, Options::default().seed(7))?;
stage.advance(5.0)?;                        // simulated seconds, in fixed 1/60 s steps
print!("{}", stage.render_ansi(Theme::Astro)?);  // the bytes `sciart frame` prints
```

`png` adds `Stage::render_png`; `widget` adds `SceneWidget` for ratatui 0.30. Only the `sciart`
crate's library carries a stability promise; the crates behind it are internal. The same build,
scene, seed and size give the same frames on the same platform, not across platforms or
versions. Writing your own scene is possible behind `unstable-generator`, with no promise.
See `examples/headless.rs` and `examples/widget.rs` in `crates/sciart`.

## How it works

```text
sciart-core        canvas, glyphs, inks, generator contract, fixed-step clock, seeded RNG
  → sciart-scenes-{cosmos,earth,physics,matter,life}   one module per scene, by category
  → sciart-generators   the registry facade over the scene crates (+ sciart-shared models)
  → sciart-render       themes, HUD, ratatui painting
  → sciart              the library facade (Stage, scenes, themes, widget) + CLI, TOML config,
                        runtime, event loop
14 crates · deterministic fixed 1/60 s step · seeded RNG · snapshot-tested
```

## Documentation

- [Master specification](docs/specs/sciart-specification.md) and the
  [component specs](docs/specs/) (`SPEC-001` … `SPEC-006`)
- [Roadmap](docs/ROADMAP.md) and [architecture decisions](docs/adr/)
- [Improvement proposal](docs/IMPROVEMENTS.md) and [changelog](CHANGELOG.md)
- [Full gallery](docs/GALLERY.md)

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). The gate is `pixi run rs-ci`; changes are spec-first
and test-first, and snapshots are reviewed by a person before they are accepted.

## License

MIT — see [LICENSE](LICENSE).
