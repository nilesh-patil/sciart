# Releasing sciart

How a version reaches its three install channels (SPEC-008, ADR-0033): GitHub Releases
(tarballs and the installer script), crates.io and the Homebrew tap. A pushed tag builds everything into a **draft** release; nothing is published
until a maintainer publishes the draft and approves each publish job.

## One-time setup

These need the repository owner's accounts and cannot be done from a checkout.

1. **Make the repository public.** The installer, `cargo binstall`, the Homebrew formula
   and `gh attestation verify` all download from public release assets, and artifact
   attestations need a public repository (or an Enterprise plan). A release tag pushed in
   a private repository fails in the first job of the `Release` workflow, before anything
   is built (DIST-R6); a manual run there builds and checks everything and skips the
   attestations with a notice.
2. **Environment `release`** (Settings → Environments), **before the first publish**: add
   the maintainers as required reviewers, and restrict it to tags `v*`. Every publish job
   waits here for approval. GitHub creates a missing environment, without reviewers, the
   first time a job names it, so the `Publish` workflow's first job, `environment-gate`,
   reads the environment and fails a published release unless it requires a reviewer
   (DIST-R10). A manual rehearsal shows what it found.
3. **crates.io**: sign in with GitHub, create an API token scoped to `publish-new` and
   `publish-update` for the crates named `sciart` and `sciart-*`, and store it as the
   environment secret `CARGO_REGISTRY_TOKEN`. After the first release, trusted publishing
   from this repository can replace the token. New crates are rate-limited, so the first
   publish of twelve crates can take a while.
4. **Homebrew tap**: create a public repository named `homebrew-sciart` (users then run
   `brew tap <owner>/sciart`), store `<owner>/homebrew-sciart` as the repository variable
   `HOMEBREW_TAP`, and a fine-grained token with *Contents: read and write* on that one
   repository as the environment secret `HOMEBREW_TAP_TOKEN`.
5. **Docs site** (Settings → Pages): set the source to *GitHub Actions*. The `Pages`
   workflow deploys on a published release, whose ref is the tag, while the `github-pages`
   environment that GitHub creates allows the default branch only: add the tag rule `v*`
   to it (Settings → Environments → github-pages → Deployment branches and tags), or the
   release's deployment is rejected. A manual run of the workflow from the default branch
   deploys at any time.
6. **Tag signing** (recommended): configure `git` to sign tags (`gpg.format = ssh` and
   `user.signingkey`), and add the key to the GitHub account as a signing key, so the tag
   shows as verified.

## Every release

1. **Versions.** Pick the `sciart` version (semver of the CLI, config keys, scene names,
   `list --json` and the library facade; see the 1.0 promise in the README) and the
   internal crates' next `0.x` version (bump the minor on every release; their API has no
   promise), then:

   ```sh
   python3 scripts/bump_version.py 1.0.0 0.7.0
   ```

2. **CHANGELOG.** Rename `## [Unreleased]` to `## [1.0.0] - YYYY-MM-DD` (and start a new
   empty `[Unreleased]`). Its text becomes the release notes.
3. **Check locally.**

   ```sh
   pixi run rs-ci && pixi run rs-size
   python3 scripts/release_check.py --tag v1.0.0
   pixi run rs-package-check          # needs cargo 1.90+ (`rustup update stable`)
   pixi run -e dist rs-installer-check
   ```

4. **Commit, tag, push.**

   ```sh
   git commit -am "chore(release): 1.0.0"
   git tag -s v1.0.0 -m "sciart 1.0.0"
   git push origin main v1.0.0
   ```

5. **Review the draft.** The `Release` workflow builds the six targets, smoke-tests them,
   packs the tarballs and `install.sh`, writes `SHA256SUMS`, attests the tarballs and
   creates a draft release. Check the assets and the notes, and optionally:

   ```sh
   gh release download v1.0.0 --pattern 'sciart-1.0.0-aarch64-apple-darwin.tar.gz' --pattern SHA256SUMS
   shasum -a 256 -c SHA256SUMS --ignore-missing
   gh attestation verify sciart-1.0.0-aarch64-apple-darwin.tar.gz --repo <owner>/sciart
   ```

6. **Publish the draft.** This starts the `Publish` workflow. Approve its two jobs in the
   `release` environment: crates.io (one crate at a time, in dependency order) and the
   Homebrew tap.
7. **Verify** an install through each channel on a clean machine (ROADMAP, manual).

A rehearsal of the publish jobs without uploading: run the `Publish` workflow by hand with
the tag (`workflow_dispatch`); each job still waits for approval, builds and checks, and
skips its upload step. `python3 scripts/publish_crates.py --dry-run` lists, from the
public index and without a token, which crates of this version are already on crates.io.

## When a publish fails half-way

**A crates.io version is permanent.** It can be *yanked* (`cargo yank --version X -p
<crate>`: new lockfiles stop picking it, existing ones keep working) but never deleted or
replaced, and the same version number can never be uploaded again. So a mistake found
after a crate is up is fixed by a new patch release, not by re-uploading.

- **crates.io stopped part-way** (network error, a rejected crate, the new-crate rate
  limit): fix the cause if there is one, then *re-run the failed `crates.io` job* from
  the workflow run. `scripts/publish_crates.py` skips every crate whose version is
  already on the index and publishes the rest in order. Do not bump versions to "start
  over": the crates already published are correct, and the facade pins exactly these
  internal versions.
- **A crate was published with a defect:** yank that version of the affected crates and
  of `sciart` if it depends on them, then release a patch version (internal crates move
  to their next `0.x`, `sciart` to `X.Y.(Z+1)`).
- **The Homebrew job failed after the push:** re-run it; with nothing to commit it
  succeeds without a new commit.
- **The GitHub release itself** is not affected by any of these: it stays published, and
  the assets and their attestations stay as they are.

## Runner images

Every workflow job names an explicit GitHub-hosted runner image; `-latest` labels are
refused because their meaning moves (TEST-R13, DIST-R5). The list lives in one place,
`RUNNER_IMAGES` in `scripts/tooling_check.py`, which both `scripts/tooling_check.py` and
`scripts/release_check.py` read:

| Image | Used for |
| --- | --- |
| `macos-15` | CI on the reference platform (lint, suite, snapshots), both macOS release builds |
| `ubuntu-24.04` | CI on Linux x86_64 and every Linux job that builds nothing for release |
| `ubuntu-24.04-arm` | CI on Linux aarch64 |
| `ubuntu-22.04`, `ubuntu-22.04-arm` | release builds only (`release.yml`): the oldest images, so the gnu binaries need glibc 2.35 |

When GitHub announces that an image is retired, change it in `RUNNER_IMAGES` and in the
workflows in one commit, and run `pixi run rs-spec-check`. The `ubuntu-22.04` images are
removed on 2027-04-17; the next glibc baseline has to be chosen before then
([BACKLOG](BACKLOG.md) B-97).

## Install instructions

The README's Install section and the user guide's installing chapter
(`docs/guide/installing.md`) must be kept in step: `pixi run site-check` compares the install
commands the site shows with the README's (WEB-R7).
