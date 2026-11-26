# ADR-0033: Four install channels, one version, gated publishing

- **Status**: Accepted; amended in 1.0.1 (the conda channel withdrawn, below)
- **Date**: 2026-11-25
- **Amends**: ADR-0024 (hosted CI and release), ADR-0007 (pixi)

## Context

Before 1.0 the only install was `cargo install --git`. A release workflow built three
tarballs when a tag was pushed and published the GitHub release at once; crates.io was
deferred (ADR-0024); pixi was a task runner only (ADR-0007); the Cargo version had stayed at
0.1.0 while tags reached v0.6.0, so a released binary printed the wrong version.

Version 1.0 is to be installable as a binary and through pixi, by every channel on
offer: GitHub Releases with an installer script, a conda package for pixi, crates.io and a
Homebrew tap, for macOS and Linux, with one facade crate carrying the stability promise.

## Decision

- **One version.** The root `Cargo.toml` holds the version of `sciart`; the tag, the
  CHANGELOG heading, the conda recipe and `--version` are checked against it.
- **crates.io.** `sciart` and the eleven crates it needs are published. The internal crates
  keep a `0.x` version of their own, are labelled internal, and are pinned with `=`, so
  `sciart 1.y.z` always builds against the internals it was tested with and their API can
  keep changing.
- **Binaries.** The existing hand-written workflow is extended to the six targets of the
  platform tiers, with checksums, provenance attestations, completions and a man page.
- **pixi.** A rattler-build recipe and a `dist` pixi environment build the conda package;
  it is published to the project's own channel.
- **Homebrew.** A formula rendered from a template and the release checksums, in a tap.
- **Gates.** A tag produces a draft release. Publishing the release, crates.io, the channel
  upload and the tap push each wait for a maintainer's approval.

## Consequences

- An install can be verified (checksums, attestation, the dependency list in the binary).
- Twelve crates on crates.io, and two version streams that a script keeps in step.
- Publishing needs a newer cargo than the MSRV on the maintainer's machine
  (`cargo publish --workspace`); users are unaffected.
- The installer, binstall, the tap and attestations need a public repository.
- A second workflow and four small checker scripts to maintain.

## Alternatives considered

- **One shared version for every crate**: simpler tooling, but an internal crate at 1.0.0
  reads as stable.
- **Merge the crates into one or two**: fewer packages to publish, at the cost of the
  parallel builds of ADR-0023.
- **A release tool that generates the workflow and installers**: replaces a workflow that
  exists and that the tiers, smoke tests and formula template make project-specific.
- **conda-forge**: the widest reach for pixi users; review takes time, so it follows 1.0.
- **Skip crates.io**: `cargo install --git` and binstall already work, but the library
  facade needs a registry.

## Amendment, 1.0.1 (2026-11-26)

The conda channel is withdrawn before its first upload. A self-hosted channel is one more
credential and one more install path to keep working, and no other channel depends on it.
The channels are the release assets with the installer script, crates.io and the Homebrew
tap. It can return through a community channel without changing the others. DIST-R8 is
withdrawn and its ID is not reused; pixi remains the project's task runner (ADR-0007).
