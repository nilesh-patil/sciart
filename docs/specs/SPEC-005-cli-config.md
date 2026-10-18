# SPEC-005: CLI & configuration

**Crate**: `crates/sciart` · **Milestones**: M1 (CLI subset), M2 (config file), M23 (record, filters, list formats, colour capability), M24 (background) · **Status**: Implemented through M22 (CLI-R7/CFG-R6/R7 in M11; CFG-R8 in M22); CLI-R8..R10 and CFG-R9 in M23; CFG-R10 in M23; CLI-R11/CFG-R11 in M24.

## CLI (clap derive)
```
sciart [OPTIONS] [COMMAND]
  sim            (default) auto-cycling simulation
  run <NAME>     single generator
  list           list generators (`--json`, `--categories`, `--plain`, `--category <ID>`; M23)
  record [SCENE] render a scene to GIF/PNG/asciicast (M23)
Options:
  --fps <1..=120>          default 30
  --interval <SECS>        default 15 (sim only)
  --theme <astro|rainbow|mono|aurora|colorblind>
  --color <auto|truecolor|256|mono>  colour output (M23)
  --keep-awake <caffeinate|mouse|off>
  --seed <u64>             default: random
  --ascii                  pure-ASCII glyphs
  --hud                    start with HUD visible
  --no-title               hide the science title (M11)
  --category <ID>          repeatable; sim/list filter (M23)
  --only <NAME>            repeatable; sim filter (M23)
  --exclude <NAME>         repeatable; sim filter (M23)
  --speed <0.1..=4>        simulation speed (M23)
  --config <PATH>          default ~/.config/sciart/config.toml
```
- **CLI-R1** No subcommand ⇒ `sim`.
- **CLI-R2** `run <unknown>` exits with code 2 and prints the valid names.
- **CLI-R3** `--fps` outside `1..=120` is rejected by clap with code 2.
- **CLI-R4** `list` output is one line per generator, `name<TAB>category<TAB>description`, sorted by name when stdout is not a terminal (or with `--plain`). On a terminal it uses the aligned form of CLI-R10 instead. Since M11 the category column is `category.id()` (kebab-case ASCII, CORE-R77), and retired names (GEN-R14) are not listed.

## Config file (M2)
```toml
fps = 30
interval = 15
theme = "astro"
color = "auto"                         # auto | truecolor | 256 | mono (M23)
keep_awake = "caffeinate"
ascii = false
hud = false
title = true                          # science title at the bottom-left (M11)
disabled = ["gray_scott"]            # excluded from sim cycling
speed = 1.0                          # simulated seconds per wall second (M23)
favorites = []                       # scene names, seeded into the session set (M23)

[low_power]
idle_after = 120   # seconds (2 min; CFG-R8, M22)
fps = 5
```
- **CFG-R1** Precedence: CLI flag > config file > built-in default (`fps 30`, `interval 15`, `theme astro`, `color auto`, `ascii false`, `hud false`, `title true`, `disabled []`, `speed 1.0`, `favorites []`). Boolean flags (`--ascii`, `--hud`) can only turn a setting on; `--no-title` (M11, CLI-R7) is the one that can only turn its setting off.
- **CFG-R2** Default path: `$XDG_CONFIG_HOME/sciart/config.toml`, else `$HOME/.config/sciart/config.toml`. A missing default file is not an error; a missing file given via `--config` is (code 2).
- **CFG-R3** Unknown keys are rejected (`deny_unknown_fields`) with the file path and TOML error location (code 2). Values are validated like the CLI: `fps ∈ 1..=120`, `interval > 0`.
- **CFG-R4** `disabled` names that are not registered produce a warning (printed to stderr before the TUI starts), not an error. `disabled` only filters `sim`; `run <name>` always works. If `disabled` removes every generator, `sim` exits with code 2.
- **CLI-R5** `--interval <SECS>` must be a finite number > 0 (code 2 otherwise).
- **CLI-R6** (M3) `--keep-awake <caffeinate|mouse|off>`; default `Mode::platform_default()` (KA-R1). A backend that can't start (e.g. `mouse` without the cargo feature) is a warning, not an error (KA-R2).
- **CFG-R5** (M3) Config keys `keep_awake = "caffeinate"|"mouse"|"off"` and table `[low_power]` with `idle_after` (seconds, finite > 0, default 120 since M22, CFG-R8) and `fps` (1..=120, default 5); unknown keys inside `[low_power]` are rejected too. Precedence as CFG-R1 (`keep_awake` also has the CLI flag; low-power settings are file-only).
- **CLI-R7** (M11) `--no-title` hides the science title (canvas and HUD name slot fall back as RT-R12/RND-R9 describe: the canvas shows no title and the HUD shows the id). Like `--ascii`/`--hud` it can only turn its setting one way: it overrides `title = true` in the config file, and its absence leaves the file's value.
- **CFG-R6** (M11) Config key `title = true|false` (default `true`) controls the science title (RT-R12). Precedence as CFG-R1, with the CLI side being `--no-title` (CLI-R7). There is no runtime key for it.
- **CFG-R7** (M11) A retired scene name in `disabled` (GEN-R14) resolves to the renamed scene and disables it without the CFG-R4 warning.
- **CFG-R8** (M22, ADR-0023) The default `low_power.idle_after` is 120 s (was 600 s): after 2 minutes without a key press, or while unfocused, the draw rate drops to `low_power.fps` (default 5). The default `fps` stays 30. `docs/config.example.toml` shows the new default.
- **CLI-R8** (M23, ADR-0025) `sciart record [SCENE] --out <PATH> [--seconds N | --frames N] [--fps F] [--size WxH] [--theme T] [--ascii] [--seed N] [--format gif|png|asciicast]` renders a scene headlessly to `PATH` (no terminal, no keep-awake) and exits 0. SCENE is required and must be registered; `--seconds` defaults to 5, `--fps` to 30, `--size` to 80×24, `--theme` to `astro`, `--seed` to 42, and `--format` is inferred from the path (`.gif`, `.png`, `.cast`/`.json`, else gif). `--frames` overrides `--seconds`; `--fps` must divide 60 (1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30, 60) so every frame lands on fixed steps, and the frame count is capped at 36,000 (20 minutes at 30 fps); a `--seconds` request that would exceed that exits 2. Frame k is the scene after exactly k/fps simulated seconds, so a recording is deterministic and frame 0 equals `rs-preview` at t = 0. GIF writes one looping 256-colour animation, PNG writes `frame-000.png`, … into the directory PATH, and asciicast writes an asciicast v2 stream. An unknown scene or value exits 2; an I/O failure exits 1. The subcommand is compiled with the `record` feature; without it the binary reports the missing feature and exits 2.
- **CLI-R9** (M23) Scene filters and speed: `--category <ID>` (repeatable), `--only <NAME>` (repeatable) and `--exclude <NAME>` (repeatable) filter `sim`; `list` applies only `--category` and ignores `--only`/`--exclude`; `run` ignores every filter flag, including unknown names. `--speed <0.1..=4>` scales simulated time per wall second for `sim` and `run`: the fixed step stays 1/60 s, frames stay deterministic, and the auto-cycle interval remains wall time. Filters compose with `disabled` (CFG-R4). An unknown category or name (where the command uses it), an empty selection, or a non-finite/out-of-range speed exits 2.
- **CLI-R10** (M23) `list` formats, in precedence order: `--json` (a JSON array of `{name, category, description}` sorted by name), `--categories` (`id<TAB>label<TAB>count` for each non-empty category, sorted by label), TSV (with `--plain` or when stdout is not a terminal; the CLI-R4 form), otherwise the aligned `NAME CATEGORY DESCRIPTION` table with a header row. Filters narrow every format, including the `--categories` counts, and every format lists the same scene set.
- **CFG-R9** (M23) Config keys `speed` (finite, 0.1..=4, default 1.0; `--speed` overrides) and `favorites` (array of registered names or retired aliases, default empty; unknown names warn like `disabled`, CFG-R4). Favourites seed the session set (RT-R19); `f` does not write the file.
- **CFG-R10** (M23, ADR-0026) Config key `color = "auto"|"truecolor"|"256"|"mono"` (default `auto`); `--color` overrides. `auto` (default or explicit) honours `NO_COLOR` as RND-R18 describes; an explicit `truecolor`/`256`/`mono` in the file or flag wins over the environment.
- **CLI-R11** (M24, ADR-0027) `--background <opaque|terminal>` is a global flag (available to `sim` and `run`), default `opaque`. `opaque` paints black behind every cell (RND-R20); `terminal` restores the terminal-default background for `Role::Background`. An invalid value exits 2. `record` is unaffected: its PNG/GIF/asciicast frames already have an opaque black image/background.
- **CFG-R11** (M24, ADR-0027) Config key `background = "opaque"|"terminal"` (default `opaque`); `--background` overrides. Invalid values are rejected like any CFG-R3 value (exit 2).
