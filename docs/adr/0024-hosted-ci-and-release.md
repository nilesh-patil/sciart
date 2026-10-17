# ADR-0024: Hosted CI and a tagged release pipeline

**Status**: Accepted (2026-10-18)
**Supersedes**: the "no hosted CI for now" clause of [ADR-0007](0007-pixi-task-runner.md)

## Context

ADR-0007 chose pixi tasks as the single build/test gate and explicitly deferred hosted CI.
That was the right trade-off for a solo, fast-moving rewrite: `pixi run rs-ci` is fast, the
toolchain is pinned by `rust-toolchain.toml`, and there was no release to publish.

The project now has 85 scenes, 1,552 tests, a 12-crate workspace and a public repository.
Two things changed the calculus:

1. **A clean second environment matters.** The `NO_COLOR` regression (TEST-R12) turned the
   local gate red on any machine that exports `NO_COLOR`, while passing on machines that do
   not. A gate that depends on the developer's shell is not a gate.
2. **There is something to ship.** The README, gallery and install story need a
   downloadable binary and a visible green badge, and Linux must keep building and passing
   tests even though macOS is the supported experience.

## Decision

- Add GitHub Actions workflows under `.github/workflows/`:
  - `ci.yml`: `lint` on macOS 14 (fmt + clippy with and without all features), `test` on
    macOS 14 and Ubuntu (doctests + nextest, plus a `NO_COLOR=1` run), `snapshots` on
    macOS 14 in the optimised `ci` profile (TEST-R9), and `cargo-deny` on Ubuntu.
  - `release.yml`: on a `v*` tag (or manual dispatch), build the `sciart` binary for
    `aarch64-apple-darwin`, `x86_64-apple-darwin` and `x86_64-unknown-linux-gnu`, package
    `sciart` + `README.md` + `LICENSE`, write a SHA-256 file, and attach the tarballs to the
    GitHub release.
- CI runs **plain cargo + cargo-nextest**, not pixi. This keeps the Linux job free of conda
  platform/lock work and makes the workflow readable. The commands are a copy of the
  `rs-ci` definition and must be kept in step with `pixi.toml`; `pixi run rs-ci` remains the
  pre-commit gate and the local source of truth.
- `cargo-deny` is part of the gate (licenses, advisories, bans, sources); the allowlist is
  deliberately narrow and grows only with a reviewed dependency.
- Release tags are created locally by the maintainer and pushed deliberately; CI never
  creates a tag. The first tag is `v0.1.0`.
- crates.io publishing stays deferred: the workspace has 12 interdependent crates, and
  `cargo install --git` / release tarballs cover installation for now.

## Consequences

- **Positive**: PRs and direct pushes get a second, clean environment; Linux build/test is
  proven continuously; a tag produces reproducible, checksummed binaries; the README can
  show a real CI badge.
- **Negative**: macOS runner minutes cost money and can be slower than local; two
  definitions of "green" exist (pixi locally, cargo in CI) and must be kept in step; action
  versions must be pinned.
- **Neutral**: `NO_COLOR` remains a test-environment concern, not an application feature,
  until colour capability lands (planned).

## Alternatives considered

- **Run pixi in CI.** Elegant (one task definition) but needs `linux-64` in the pixi lock,
  a conda solve on every runner, and makes the workflow harder to debug. Revisit if the
  command duplication causes drift.
- **macOS-only CI.** Cheaper, but drops the Linux build/test guarantee that the contributor guide has
  required since M0.
- **No releases.** Keeps the maintenance surface small, but leaves `cargo install --git`
  as the only install path and gives the README no artifact to point at.

## References

- [ADR-0007](0007-pixi-task-runner.md) (task runner; the deferred-CI clause is superseded)
- TEST-R3, TEST-R9, TEST-R10, TEST-R11, TEST-R12 in [SPEC-006](../specs/SPEC-006-testing.md)
- `docs/IMPROVEMENTS.md` IMP-01, IMP-04, IMP-05
