# ADR-0001: Cargo workspace with five crates

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
The Python project mixes discovery, rendering, color, OS automation and art in one script plus a flat package. We want generators testable without a terminal, and boundaries enforced by the compiler.

## Decision
A virtual workspace under `crates/`:
`sciart-core` (primitives, no I/O) ← `sciart-generators` (26 scenes + registry) ← `sciart-render` (themes, ratatui) ← `sciart` (binary: CLI, config, runtime). `sciart-keepawake` is a leaf used only by the binary. Shared versions and lints live in `[workspace.package]`, `[workspace.dependencies]`, `[workspace.lints]`.

## Consequences
+ Generators cannot touch the terminal or OS.
+ Parallel compilation; focused test runs (`cargo test -p sciart-core`).
− More manifests; cross-crate refactors touch several `Cargo.toml` files.

## Alternatives considered
Single crate with modules (boundaries by convention only); one crate per generator (26 manifests for ~150-line modules).
