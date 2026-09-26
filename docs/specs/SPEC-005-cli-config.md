# SPEC-005: CLI & configuration

**Crate**: `crates/sciart` · **Milestones**: M1 (CLI subset), M2 (config file) · **Status**: M1 implemented: `sim|run|list`, `--fps --seed --theme --ascii`. `--interval --hud --config` land in M2, `--keep-awake` in M3.

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
- **CFG-R1** Precedence: CLI flag > config file > built-in default.
- **CFG-R2** Missing default config file is not an error; missing file given via `--config` is (code 2).
- **CFG-R3** Unknown keys are rejected (`deny_unknown_fields`) with the file path and TOML error location.
- **CFG-R4** `disabled` names that are not registered produce a warning, not an error.
