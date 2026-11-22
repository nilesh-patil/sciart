# Releasing sciart

How a version reaches its four install channels (SPEC-008, ADR-0033): GitHub Releases
(tarballs and the installer script), crates.io, the conda channel for pixi, and the
Homebrew tap. A pushed tag builds everything into a **draft** release; nothing is published
until a maintainer publishes the draft and approves each publish job.

## One-time setup

These need the repository owner's accounts and cannot be done from a checkout.

1. **Make the repository public.** The installer, `cargo binstall`, the Homebrew formula
   and `gh attestation verify` all download from public release assets, and artifact
   attestations need a public repository (or an Enterprise plan).
2. **Environment `release`** (Settings → Environments): add the maintainers as required
   reviewers, and restrict it to tags `v*`. Every publish job waits here for approval.
3. **crates.io**: sign in with GitHub, create an API token scoped to `publish-new` and
   `publish-update` for the crates named `sciart` and `sciart-*`, and store it as the
   environment secret `CARGO_REGISTRY_TOKEN`. After the first release, trusted publishing
   from this repository can replace the token. New crates are rate-limited, so the first
   publish of twelve crates can take a while.
4. **Conda channel**: create a channel on prefix.dev, store its name as the repository
   variable `CONDA_CHANNEL` and an API key with upload rights as the environment secret
   `PREFIX_API_KEY`.
5. **Homebrew tap**: create a public repository named `homebrew-sciart` (users then run
   `brew tap <owner>/sciart`), store `<owner>/homebrew-sciart` as the repository variable
   `HOMEBREW_TAP`, and a fine-grained token with *Contents: read and write* on that one
   repository as the environment secret `HOMEBREW_TAP_TOKEN`.
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
   pixi run -e dist rs-conda-build    # optional: the conda package for this machine
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

6. **Publish the draft.** This starts the `Publish` workflow. Approve its three jobs in the
   `release` environment: crates.io (one crate at a time, in dependency order), the conda
   channel (four platforms) and the Homebrew tap.
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
- **The conda upload stopped part-way:** re-run the `conda` job; the upload skips the
  packages the channel already has (`--skip-existing`).
- **The Homebrew job failed after the push:** re-run it; with nothing to commit it
  succeeds without a new commit.
- **The GitHub release itself** is not affected by any of these: it stays published, and
  the assets and their attestations stay as they are.

## Install instructions

The README's Install section and the user guide's installing chapter
(`docs/guide/installing.md`) must be kept in step: `pixi run site-check` compares the install
commands the site shows with the README's (WEB-R7).
