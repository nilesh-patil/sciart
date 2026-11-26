# ADR-0007: pixi as the single task runner

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
The repo already uses pixi for Python. During the transition both implementations live side-by-side.

## Decision
Rust commands are pixi tasks prefixed `rs-` (`rs-ci`, `rs-test`, `rs-lint`, `rs-bench`, `rs-run`, `rs-sim`, …) wrapping cargo. The toolchain is pinned by `rust-toolchain.toml` (rustup), not conda, so cargo and rust-analyzer agree. No hosted CI for now; `pixi run rs-ci` is the pre-commit gate.

## Consequences
+ One entry point (`pixi run …`) for every contributor and for CI.
− Contributors need rustup as well as pixi.

## Update (M8)
The Python prototype was removed; pixi now only runs the `rs-*` tasks and has no conda/PyPI dependencies.

## Alternatives considered
justfile; GitHub Actions (deferred — see spec Future Considerations).
