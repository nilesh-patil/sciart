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

## Decision

**Tiers.** Tier 1 (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`): the whole suite in
CI, frames pinned. Tier 2 (`aarch64-unknown-linux-gnu`): the whole suite in CI except the
frame-pinning suites. Tier 3 (`x86_64-apple-darwin`, both Linux musl targets): release
builds only. Windows is not supported in 1.0.

**Determinism is per platform.** The promise is: the same binary, seed and size produce the
same frames. Nothing is promised across platforms or across versions. The frame-pinning
suites run on tier 1 only and skip themselves elsewhere; the same-seed determinism tests run
everywhere the suite runs.

**Terminals are advised, not detected.** The colour mode is not guessed from `TERM`
(ADR-0026). Where the environment says the default output cannot render, the binary prints
one line of advice on exit and changes nothing itself.

## Consequences

- No snapshot or M11 fixture is re-recorded, and the M11 look does not move.
- A recording made on Linux may differ in a few cells from one made on macOS with the same
  seed; documentation and the showcase state which platform produced a clip only where it
  matters.
- The web gallery (WebAssembly) is its own platform under the same promise.
- A tier-2 or tier-3 target can be promoted by pinning frames there, which means reviewing
  the differences first.

## Alternatives considered

- **Identical frames everywhere through the pure-Rust `libm` crate**: one behaviour on every
  target and in the browser, at the cost of a new core dependency, re-recording every
  snapshot and the M11 fixtures, and slower transcendental functions in the hot scenes.
  Rejected for 1.0 in favour of the per-platform promise.
- **Pin frames on every target**: five sets of expected frames to review and keep.
- **Detect terminal capability and switch modes**: rejected in ADR-0026; a wrong guess is
  worse than a hint.
