# SPEC-005: CLI & configuration

**Crate**: `crates/sciart` · **Milestones**: M1 (CLI subset), M2 (config file) · **Status**: Implemented through M3; CLI-R7, CFG-R6 and CFG-R7 specified for M11.

## CLI (clap derive)
```
sciart [OPTIONS] [COMMAND]
  sim            (default) auto-cycling simulation
  run <NAME>     single generator
  list           list generators
Options:
  --fps <1..=120>          default 30
  --interval <SECS>        default 15 (sim only)
  --theme <astro|rainbow|mono|aurora>
  --keep-awake <caffeinate|mouse|off>
  --seed <u64>             default: random
  --ascii                  pure-ASCII glyphs
  --hud                    start with HUD visible
  --no-title               hide the science title (M11)
  --config <PATH>          default ~/.config/sciart/config.toml
```
- **CLI-R1** No subcommand ⇒ `sim`.
- **CLI-R2** `run <unknown>` exits with code 2 and prints the valid names.
- **CLI-R3** `--fps` outside `1..=120` is rejected by clap with code 2.
- **CLI-R4** `list` output: one line per generator, `name<TAB>category<TAB>description`, sorted by name. Since M11 the category column is `category.id()` (kebab-case ASCII, CORE-R77), and retired names (GEN-R14) are not listed.

## Config file (M2)
```toml
fps = 30
interval = 15
theme = "astro"
keep_awake = "caffeinate"
ascii = false
hud = false
title = true                          # science title at the bottom-left (M11)
disabled = ["gray_scott"]            # excluded from sim cycling

[low_power]
idle_after = 600   # seconds
fps = 5
```
- **CFG-R1** Precedence: CLI flag > config file > built-in default (`fps 30`, `interval 15`, `theme astro`, `ascii false`, `hud false`, `title true`, `disabled []`). Boolean flags (`--ascii`, `--hud`) can only turn a setting on; `--no-title` (M11, CLI-R7) is the one that can only turn its setting off.
- **CFG-R2** Default path: `$XDG_CONFIG_HOME/sciart/config.toml`, else `$HOME/.config/sciart/config.toml`. A missing default file is not an error; a missing file given via `--config` is (code 2).
- **CFG-R3** Unknown keys are rejected (`deny_unknown_fields`) with the file path and TOML error location (code 2). Values are validated like the CLI: `fps ∈ 1..=120`, `interval > 0`.
- **CFG-R4** `disabled` names that are not registered produce a warning (printed to stderr before the TUI starts), not an error. `disabled` only filters `sim`; `run <name>` always works. If `disabled` removes every generator, `sim` exits with code 2.
- **CLI-R5** `--interval <SECS>` must be a finite number > 0 (code 2 otherwise).
- **CLI-R6** (M3) `--keep-awake <caffeinate|mouse|off>`; default `Mode::platform_default()` (KA-R1). A backend that can't start (e.g. `mouse` without the cargo feature) is a warning, not an error (KA-R2).
- **CFG-R5** (M3) Config keys `keep_awake = "caffeinate"|"mouse"|"off"` and table `[low_power]` with `idle_after` (seconds, finite > 0, default 600) and `fps` (1..=120, default 5); unknown keys inside `[low_power]` are rejected too. Precedence as CFG-R1 (`keep_awake` also has the CLI flag; low-power settings are file-only).
- **CLI-R7** (M11) `--no-title` hides the science title (canvas and HUD name slot fall back as RT-R12/RND-R9 describe: the canvas shows no title and the HUD shows the id). Like `--ascii`/`--hud` it can only turn its setting one way: it overrides `title = true` in the config file, and its absence leaves the file's value.
- **CFG-R6** (M11) Config key `title = true|false` (default `true`) controls the science title (RT-R12). Precedence as CFG-R1, with the CLI side being `--no-title` (CLI-R7). There is no runtime key for it.
- **CFG-R7** (M11) A retired scene name in `disabled` (GEN-R14) resolves to the renamed scene and disables it without the CFG-R4 warning.
