# ADR-0029: Platform tiers and per-platform determinism

- **Status**: Accepted
- **Date**: 2026-11-25

## Context

Until 1.0 the project was built and reviewed on Apple Silicon, with Linux x86_64 running the
tests in CI. Version 1.0 ships binaries for macOS and Linux on both architectures, plus
static musl builds. Two questions needed an answer before calling a platform supported.

First, what is tested where. Linux aarch64, musl and Intel macOS had never been built.

Second, what "deterministic" means. Frames are pinned by snapshots and by the M11 fidelity
fixtures. Sine and cosine go through `Trig` so a frame does not depend on the optimiser
(ADR-0016), but `Trig`, like the roughly thousand `exp`, `powf`, `ln`, `atan2` and `hypot`
calls in the scenes, still ends in the platform's maths library. glibc, musl, Apple's libm
on each architecture, and a browser's WebAssembly runtime may round the last bit differently.

This is measured, not hypothetical. On x86_64 Linux the suite differs from Apple Silicon in
one snapshot (`molecular_dynamics` at t = 5 s, where a Lennard-Jones gas has amplified a
last-bit difference into different atom positions) and in two statistical thresholds that
had been tuned on one platform.

## Decision

**Tiers.** Tier 1 (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`): lint and the whole
suite in CI. Tier 2 (`aarch64-unknown-linux-gnu`): the whole suite in CI. Tier 3
(`x86_64-apple-darwin`, both Linux musl targets): release builds only. Windows is not
supported in 1.0.

**Frames are pinned on one reference platform**, `aarch64-apple-darwin`, where every look
was reviewed. The frame-pinning suites (the snapshots and the M11 fidelity fixtures) run
there and skip themselves on every other target. Everything else runs wherever the suite
runs, including the same-seed determinism tests. A test of a chaotic model asserts a
property with a margin that holds for any seed, not a threshold that one platform's
rounding decides.

**Determinism is per platform.** The promise is: the same binary, seed and size produce the
same frames. Nothing is promised across platforms or across versions.

**Terminals are advised, not detected.** The colour mode is not guessed from `TERM`
(ADR-0026). Where the environment says the default output cannot render, the binary prints
one line of advice on exit and changes nothing itself.

## Consequences

- No snapshot or M11 fixture is re-recorded, and the M11 look does not move.
- A recording made on Linux may differ in a few cells from one made on macOS with the same
  seed; documentation and the showcase state which platform produced a clip only where it
  matters.
- The web gallery (WebAssembly) is its own platform under the same promise.
- Another target can pin frames too. That means recording a second set of expected frames
  there and reviewing every difference first.

## Alternatives considered

- **Identical frames everywhere through the pure-Rust `libm` crate**: one behaviour on every
  target and in the browser, at the cost of a new core dependency, re-recording every
  snapshot and the M11 fixtures, and slower transcendental functions in the hot scenes.
  Rejected for 1.0 in favour of the per-platform promise.
- **Pin frames on every target**: five sets of expected frames to review and keep.
- **Pin frames on x86_64 Linux as well**: a second set of 255 snapshots, re-recorded and
  reviewed with every look change. Left for after 1.0.
- **Detect terminal capability and switch modes**: rejected in ADR-0026; a wrong guess is
  worse than a hint.
