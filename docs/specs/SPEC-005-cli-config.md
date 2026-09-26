# SPEC-005: CLI & configuration

**Crate**: `crates/sciart` · **Milestones**: M1 (CLI subset), M2 (config file) · **Status**: M2 implemented; `--keep-awake` and the `keep_awake` / `[low_power]` config keys land in M3 (until then they are rejected as unknown keys).

## CLI (clap derive)
```
sciart [OPTIONS] [COMMAND]
  sim            (default) auto-cycling simulation
  run <NAME>     single generator
  list           list generators
Options:
  --fps <1..=120>          default 30
  --interval <SECS>        default 15 (sim only)
  --theme <astro|rainbow|mono>
  --keep-awake <caffeinate|mouse|off>
  --seed <u64>             default: random
  --ascii                  pure-ASCII glyphs
  --hud                    start with HUD visible
  --config <PATH>          default ~/.config/sciart/config.toml
```
- **CLI-R1** No subcommand ⇒ `sim`.
- **CLI-R2** `run <unknown>` exits with code 2 and prints the valid names.
- **CLI-R3** `--fps` outside `1..=120` is rejected by clap with code 2.
- **CLI-R4** `list` output: one line per generator, `name<TAB>category<TAB>description`, sorted by name.

## Config file (M2)
```toml
fps = 30
interval = 15
theme = "astro"
keep_awake = "caffeinate"
ascii = false
hud = false
disabled = ["chemical_reaction"]     # excluded from sim cycling

[low_power]
idle_after = 600   # seconds
fps = 5
```
- **CFG-R1** Precedence: CLI flag > config file > built-in default (`fps 30`, `interval 15`, `theme astro`, `ascii false`, `hud false`, `disabled []`). Boolean flags (`--ascii`, `--hud`) can only turn a setting on.
- **CFG-R2** Default path: `$XDG_CONFIG_HOME/sciart/config.toml`, else `$HOME/.config/sciart/config.toml`. A missing default file is not an error; a missing file given via `--config` is (code 2).
- **CFG-R3** Unknown keys are rejected (`deny_unknown_fields`) with the file path and TOML error location (code 2). Values are validated like the CLI: `fps ∈ 1..=120`, `interval > 0`.
- **CFG-R4** `disabled` names that are not registered produce a warning (printed to stderr before the TUI starts), not an error. `disabled` only filters `sim`; `run <name>` always works. If `disabled` removes every generator, `sim` exits with code 2.
- **CLI-R5** `--interval <SECS>` must be a finite number > 0 (code 2 otherwise).
