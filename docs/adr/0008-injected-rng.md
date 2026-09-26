# ADR-0008: Injected seeded RNG, no global randomness

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
Snapshot and property tests need byte-identical output; users want `--seed` to reproduce a scene.

## Decision
`Ctx` owns a `ChaCha8Rng` seeded from `--seed` (random when absent, shown in the HUD). Generators use only `ctx.rng`. `clippy.toml` disallows `rand::rng`, `rand::thread_rng`, `rand::random`, `Instant::now` and `SystemTime::now`; the binary crate, which legitimately needs the clock, opts out locally with `#[allow(clippy::disallowed_methods)]` at the call site.

## Consequences
+ Fully deterministic generators; reproducible bug reports.
− Helpers must thread `ctx` through.
