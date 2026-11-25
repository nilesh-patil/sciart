# ADR-0035: Release profile and dependency trim

- **Status**: Accepted
- **Date**: 2026-11-26

## Context

The release binary was 3.64 MB with 108 crates in its dependency graph. About half of the
code is the scenes; the rest is the standard library, `clap`, `ratatui`, `crossterm` and
`toml`. `ratatui`'s default features bring every widget, including a calendar that pulls in
a date-and-time stack the program never uses; `clap` and `toml` likewise default to more
than the code calls. Version 1.0 is downloaded as a binary and built by `cargo install`, so
both the size and the number of crates to compile are paid by every user.

Each candidate was measured for size and for frame time on the release profile.

| Change | Size | Frame time |
|---|---|---|
| Strip symbols | −11.6 % | unchanged |
| Fat LTO instead of thin | −7.2 % | unchanged, slower build |
| 16 codegen units | +15.3 % | — |
| `opt-level = "s"` on the cold crates | +1.1 % | — |
| `opt-level = "z"` on the cold crates | +2.3 % | — |

## Decision

- `ratatui`, `clap` and `toml` are built with only the features the code uses: 108 crates
  become 93.
- The release profile strips symbols: 3 636 112 bytes become 3 213 264.
- Thin LTO and one codegen unit stay (ADR-0023). Fat LTO is not worth its build time, and
  size-optimising the cold crates makes the binary larger.
- A checker holds both numbers: at most 3.25 MiB and 93 crates for the build without the
  recorder (RT-R22). The budgets only go down.

## Consequences

- A stripped binary's backtrace has no function names. Panic messages keep their file and
  line, and the scene quarantine (ADR-0006) is unaffected: it needs unwinding, not symbols.
- A dependency that needs a trimmed feature fails to compile, not at run time, so a wrong
  trim cannot ship.
- Adding a crate or a feature to the default binary is now a visible decision: the checker
  fails until the budget is raised in the spec.

## Alternatives considered

- **`panic = "abort"`**: the largest single saving, and it removes the scene quarantine.
- **A smaller argument parser**: `clap` is about a tenth of the code; replacing it would
  cost the generated help, completions and man page.
- **Dropping thin LTO for build speed**: frames per second matter more to users than the
  maintainer's link time; the `ci` profile already serves fast local builds.
