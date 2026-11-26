#!/usr/bin/env python3
"""Mutation tests of the workflow checks (SPEC-008 DIST-R6, DIST-R10).

Each row breaks the real workflows in one way a careless edit or a hostile change could,
and the check must report it; the unmodified workflows must pass. A row whose text no
longer appears in the workflow fails too, so the table cannot rot silently.

Before them, the release notes (`release_check.py --notes`): relative links pinned to the
tag, everything else left alone, and no relative link target in the real 1.0.0 notes.

    python3 scripts/release_check_test.py
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_check  # noqa: E402
import release_workflows  # noqa: E402

GATED = "        if: github.event_name == 'release'\n"
PUBLISH_STEP = """      - name: Publish
        if: github.event_name == 'release'
        env:
          RUSTUP_TOOLCHAIN: stable
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        run: python3 scripts/publish_crates.py
"""
SNEAK = "      # DIST-R4: one crate at a time"
CHECKOUT = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"
INSTALL = "taiki-e/install-action@893d6ee8d714b6c7dfe2bcd94e5759e9a7958a7e # v2.90.10"
PRIVATE_GUARD = """      - name: A release tag needs a public repository
        if: startsWith(github.ref, 'refs/tags/v') && github.event.repository.private
"""
ATTEST_IF = "        if: github.event.repository.private == false\n"
GATE_NEEDS = "    needs: environment-gate\n"
HOMEBREW_HEAD = "    name: Homebrew tap\n" + GATE_NEEDS
TAG_STEP_PUBLISH = """      - name: Tag matches the version (DIST-R1)
        run: python3 scripts/release_check.py --tag "$TAG"
"""
TAG_STEP_RELEASE = """      - name: Tag matches the version
        if: startsWith(github.ref, 'refs/tags/v')
        run: python3 scripts/release_check.py --tag "${GITHUB_REF_NAME}"
"""
INSTALLER_STEP = """      - name: Installer against a fixture release (DIST-R7)
        run: python3 scripts/installer_check.py --require-shellcheck
"""
JSON_CHECK = """assert json.load(sys.stdin)["scenes"]'"""

# (label, workflow, text, replacement, a phrase the report must contain)
MUTATIONS = [
    # publish.yml
    ("a: cargo publish, flags reordered, in an ungated step", "publish.yml", SNEAK,
     "      - name: Sneak\n        run: cargo publish --locked --workspace\n" + SNEAK, "outside the gated publish step"),
    ("b: one crate published outside the gate", "publish.yml", SNEAK,
     "      - name: Sneak\n        run: cargo publish -p sciart-core\n" + SNEAK, "outside the gated publish step"),
    ("c: the gate weakened", "publish.yml", PUBLISH_STEP,
     PUBLISH_STEP.replace("'release'\n", "'release' || true\n"), "outside the gated publish step"),
    ("d: the registry token at job level", "publish.yml",
     "    environment: release\n    steps:\n      - uses: " + CHECKOUT + "\n        with:\n          ref: ${{ env.TAG }}\n"
     "          persist-credentials: false\n      - name: Tag matches the version (DIST-R1)\n"
     "        run: python3 scripts/release_check.py --tag \"$TAG\"\n      # `cargo publish --workspace`",
     "    environment: release\n    env:\n      CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}\n    steps:\n"
     "      - uses: " + CHECKOUT + "\n        with:\n          ref: ${{ env.TAG }}\n          persist-credentials: false\n"
     "      - name: Tag matches the version (DIST-R1)\n        run: python3 scripts/release_check.py --tag \"$TAG\"\n"
     "      # `cargo publish --workspace`", "secret at job level"),
    ("e: the crates.io tag check deleted", "publish.yml",
     TAG_STEP_PUBLISH + "      # `cargo publish --workspace`", "      # `cargo publish --workspace`",
     "`crates-io` has a tag check"),
    ("f: the release body interpolated into a script", "publish.yml",
     "        run: python3 scripts/publish_crates.py\n",
     "        run: echo \"${{ github.event.release.body }}\" && python3 scripts/publish_crates.py\n",
     "interpolates github.event"),
    ("the token inside the script", "publish.yml", "        run: python3 scripts/publish_crates.py\n",
     "        run: CARGO_REGISTRY_TOKEN=${{ secrets.CARGO_REGISTRY_TOKEN }} python3 scripts/publish_crates.py\n",
     "secret inside a run script"),
    ("a workspace publish instead of the resumable one", "publish.yml",
     "        run: python3 scripts/publish_crates.py\n", "        run: cargo publish --workspace --locked\n",
     "one resumable publish"),
    ("no rehearsal", "publish.yml", "          python3 scripts/publish_crates.py --dry-run\n", "", "rehearses"),
    ("the Homebrew job outside the release environment", "publish.yml",
     HOMEBREW_HEAD + "    runs-on: ubuntu-24.04\n    environment: release\n",
     HOMEBREW_HEAD + "    runs-on: ubuntu-24.04\n", "`homebrew` runs in the `release` environment"),
    ("a third publish job", "publish.yml", "  homebrew:\n",
     "  extra:\n    needs: environment-gate\n    runs-on: ubuntu-24.04\n    environment: release\n"
     "    steps:\n      - run: echo\n\n  homebrew:\n", "exactly the jobs"),
    ("the oldest image in the publish workflow", "publish.yml", "    runs-on: ubuntu-24.04\n",
     "    runs-on: ubuntu-22.04\n", "release builds only"),
    ("a tap commit that fails on a rerun", "publish.yml", "          if git diff --cached --quiet; then\n"
     "            echo \"the tap already has this formula\"\n          else\n", "          if true; then\n",
     "tolerates a formula"),
    ("no concurrency group", "publish.yml", "concurrency:\n  group:", "x-concurrency:\n  group:", "concurrency group"),
    ("an upload that cannot fail", "publish.yml", "        run: python3 scripts/publish_crates.py\n",
     "        run: python3 scripts/publish_crates.py || true\n", "cannot fail"),
    ("published by a pull request", "publish.yml", "  release:\n    types: [published]\n",
     "  release:\n    types: [published]\n  pull_request:\n", "published release or by hand"),
    # release.yml
    ("g: the tag check deleted", "release.yml", TAG_STEP_RELEASE, "", "tag check"),
    ("the tag check skipped on tags", "release.yml", "if: startsWith(github.ref, 'refs/tags/v')\n        run: python3 "
     "scripts/release_check.py --tag", "if: false\n        run: python3 scripts/release_check.py --tag", "tag check"),
    ("the tag check allowed to fail", "release.yml", TAG_STEP_RELEASE,
     TAG_STEP_RELEASE.replace("        if:", "        continue-on-error: true\n        if:"), "tag check cannot fail"),
    ("h: the installer check deleted", "release.yml", INSTALLER_STEP, "", "installer check"),
    ("the mutation tests deleted", "release.yml", "          python3 scripts/release_check_test.py\n", "",
     "workflow mutation tests"),
    ("i: write-all on build", "release.yml", "    needs: verify\n", "    needs: verify\n    permissions: write-all\n",
     "beyond its allow-list"),
    ("package may write contents", "release.yml", "      contents: read\n      id-token: write",
     "      contents: write\n      id-token: write", "beyond its allow-list"),
    ("the workflow default widened", "release.yml", "permissions:\n  contents: read\n\n# One release build",
     "permissions:\n  contents: write\n\n# One release build", "contents: read"),
    ("a release workflow without concurrency", "release.yml", "concurrency:\n  group: release-",
     "x-concurrency:\n  group: release-", "concurrency group"),
    ("a build cache in the release build", "release.yml", "      # DIST-R6: the dependency list is embedded",
     "      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3\n"
     "      # DIST-R6: the dependency list is embedded", "restores a build cache"),
    ("credentials persisted by a checkout", "release.yml", "        with:\n          persist-credentials: false\n",
     "", "persist-credentials"),
    ("the tap token persisted by its checkout", "publish.yml", "          path: tap\n          persist-credentials: false\n",
     "          path: tap\n", "persist-credentials"),
    ("j: release does not need package", "release.yml", "    needs: [verify, package]\n", "    needs: [verify]\n",
     "build -> package -> release"),
    ("k: package does not need build", "release.yml", "    needs: [verify, build]\n", "    needs: [verify]\n",
     "build -> package -> release"),
    ("l: a third-party action swapped in", "release.yml", INSTALL, "someone-else/install-action@main", "not pinned"),
    ("l: a pinned action from an unknown owner", "release.yml", INSTALL,
     INSTALL.replace("taiki-e/", "someone-else/"), "allow-list"),
    ("an install step that names no tool", "fuzz.yml", INSTALL + "\n        with:\n          tool: cargo-fuzz\n",
     INSTALL + "\n", "without naming its tool"),
    ("an install step with an empty tool", "ci.yml", "          tool: nextest\n", "          tool: \"\"\n",
     "without naming its tool"),
    # runner images (TEST-R13, DIST-R5)
    ("a retired macOS image", "ci.yml", "          - os: macos-15\n", "          - os: macos-14\n",
     "runs on `macos-14`"),
    ("a moving ubuntu label", "pages.yml", "    runs-on: ubuntu-24.04\n", "    runs-on: ubuntu-latest\n",
     "runs on `ubuntu-latest`"),
    ("a moving macOS label in a matrix", "release.yml", "            os: macos-15\n", "            os: macos-latest\n",
     "runs on `macos-latest`"),
    ("a retired image for a release build", "release.yml", "            os: macos-15\n", "            os: macos-14\n",
     "runs on `macos-14`"),
    ("the oldest image outside the release builds", "ci.yml", "    runs-on: ubuntu-24.04\n",
     "    runs-on: ubuntu-22.04\n", "release builds only"),
    ("a runner the check cannot read", "fuzz.yml", "    runs-on: ubuntu-24.04\n",
     "    runs-on: ${{ github.event.inputs.runner }}\n", "names no runner image"),
    # attestations need a public repository (DIST-R6)
    ("the private-repository guard deleted", "release.yml", PRIVATE_GUARD, "      - name: Anything\n        if: false\n",
     "private repository"),
    ("the private-repository guard weakened", "release.yml", PRIVATE_GUARD,
     PRIVATE_GUARD.replace("        if: ", "        continue-on-error: true\n        if: "), "private repository"),
    ("the attestation unguarded", "release.yml", ATTEST_IF, "", "only in a public repository"),
    ("the skipped attestation without a notice", "release.yml",
     "          echo \"::notice::No build-provenance", "          echo \"No build-provenance", "`::notice::`"),
    # the release environment gate (DIST-R10)
    ("the environment gate deleted", "publish.yml", "  environment-gate:\n", "  x-environment-gate:\n",
     "`environment-gate` job"),
    ("a publish job that does not need the gate", "publish.yml", HOMEBREW_HEAD, "    name: Homebrew tap\n",
     "`homebrew` needs `environment-gate`"),
    ("the gate inside the environment it checks", "publish.yml",
     "    permissions:\n      actions: read", "    environment: release\n    permissions:\n      actions: read",
     "outside the `release` environment"),
    ("a gate that never fails", "publish.yml", "                exit 1\n", "", "required_reviewers"),
    ("a gate that cannot fail", "publish.yml", "          set -euo pipefail\n          if rules=",
     "          set +e\n          if rules=", "can fail the job"),
    ("the gate may write", "publish.yml", "      actions: read #", "      actions: write #", "beyond its allow-list"),
    ("m: the smoke JSON check swallowed", "release.yml", JSON_CHECK, JSON_CHECK + " || true", "cannot fail"),
    ("the smoke test without -V", "release.yml", '          test "$("$bin" -V)" = "sciart $VERSION"\n', "",
     '"$bin" -V'),
    ("n: an action by tag", "release.yml", CHECKOUT, "actions/checkout@v4", "not pinned"),
    ("n: an action by tag in another workflow", "ci.yml", CHECKOUT, "actions/checkout@v4", "not pinned"),
    ("a pin without its version", "fuzz.yml", CHECKOUT, CHECKOUT.split(" #")[0], "version in a comment"),
    ("not a draft", "release.yml", "--draft --verify-tag", "--verify-tag", "creates a draft"),
    ("a tag created", "release.yml", "--draft --verify-tag", "--draft", "creates a draft"),
    ("an attestation missing install.sh", "release.yml", "            dist/install.sh\n", "", "attests"),
    ("a git tag in CI", "ci.yml", "    steps:\n", "    steps:\n      - run: git tag v9.9.9\n", "creates or pushes a tag"),
    ("an upload from CI", "ci.yml", "    steps:\n", "    steps:\n      - run: cargo publish -p sciart\n",
     "only publish.yml does"),
    ("a secret at workflow level", "release.yml", "  CARGO_TERM_COLOR: always\n",
     "  CARGO_TERM_COLOR: always\n  TOKEN: ${{ secrets.X }}\n", "workflow level"),
    ("triggered by pull requests", "release.yml", "  workflow_dispatch:\n", "  workflow_dispatch:\n  pull_request:\n",
     "pushed v* tag"),
    ("a size check that cannot fail", "release.yml", "python3 scripts/size_check.py --build default\n",
     "python3 scripts/size_check.py --build default || true\n", "size check cannot fail"),
    ("a build with other features", "release.yml", '--bin sciart --target "$TARGET"',
     '--bin sciart --target "$TARGET" --features mouse', "default features"),
    ("a seventh target", "release.yml", "          - target: aarch64-unknown-linux-musl\n",
     "          - target: i686-unknown-linux-gnu\n            os: ubuntu-22.04\n            tier: 3\n"
     "            smoke: false\n          - target: aarch64-unknown-linux-musl\n", "six targets"),
    ("a workflow the parser cannot read", "fuzz.yml", "jobs:\n", "anchor: &x 1\njobs:\n", "cannot read"),
]


REPO = "https://github.com/someone/project"
# DIST-R10: the release notes are the CHANGELOG section; on the release page a relative link
# resolves under /releases/tag/, so every relative target is pinned to the tag's tree.
# (label, the CHANGELOG text, the notes text)
NOTES_CASES = [
    ("a relative file link", "see [ADR-0032](docs/adr/0032-library-facade.md).",
     f"see [ADR-0032]({REPO}/blob/v1.2.3/docs/adr/0032-library-facade.md)."),
    ("a relative link with a fragment", "[guide](docs/guide/running.md#keys)",
     f"[guide]({REPO}/blob/v1.2.3/docs/guide/running.md#keys)"),
    ("a ./ link and a title", '[x](./README.md "Read me")', f'[x]({REPO}/blob/v1.2.3/README.md "Read me")'),
    ("a reference definition", "[adr]: docs/adr/0001-x.md", f"[adr]: {REPO}/blob/v1.2.3/docs/adr/0001-x.md"),
    ("an absolute link left alone", "[spec](https://keepachangelog.com/en/1.1.0/)",
     "[spec](https://keepachangelog.com/en/1.1.0/)"),
    ("an anchor-only link left alone", "[above](#upgrading-from-06)", "[above](#upgrading-from-06)"),
    ("a mailto link left alone", "[mail](mailto:someone@example.invalid)", "[mail](mailto:someone@example.invalid)"),
    ("code that looks like a link left alone", "`[a](b)` in code", "`[a](b)` in code"),
]


def notes_problems() -> list[str]:
    problems = []
    for label, text, want in NOTES_CASES:
        got = release_check.pinned_links(text, REPO + ".git", "1.2.3")
        if got != want:
            problems.append(f"notes, {label}: {got!r}, expected {want!r}")
    real = release_check.notes("1.0.0") or ""
    if not real:
        problems.append("notes: CHANGELOG.md has no 1.0.0 section")
    left = release_check.relative_targets(real)
    if left:
        problems.append(f"notes: the 1.0.0 release notes keep relative link targets: {left}")
    repo = release_check.toml_file("Cargo.toml")["workspace"]["package"]["repository"]
    if f"{repo}/blob/v1.0.0/" not in real:
        problems.append(f"notes: the 1.0.0 release notes do not link into {repo}/blob/v1.0.0/")
    return problems


# DIST-R4: the publish order, judged from manifests. A fixture workspace: `a` <- `b` <- `c`,
# with a never-published `t`; the workspace table gives `a`, `b`, `c` a version, `t` a path.
WORKSPACE = {"a": {"path": "a", "version": "=0.1.0"}, "b": {"path": "b", "version": "=0.1.0"},
             "c": {"path": "c", "version": "=0.1.0"}, "t": {"path": "t"}}


def manifests(**extra: dict) -> dict[str, dict]:
    base = {
        "a": {"package": {"name": "a"}},
        "b": {"package": {"name": "b"}, "dependencies": {"a": {"workspace": True}}},
        "c": {"package": {"name": "c"}, "dependencies": {"b": {"workspace": True}}},
        "t": {"package": {"name": "t", "publish": False}},
    }
    for name, tables in extra.items():
        for kind, table in tables.items():
            base[name].setdefault(kind, {}).update(table)
    return base


# (label, order, manifests, a phrase every report must contain, or None when accepted)
ORDER_CASES = [
    ("in order", ("a", "b", "c"), manifests(), None),
    ("a dependency published later", ("b", "a", "c"), manifests(), "published after it"),
    ("a dev-dependency published later", ("a", "b", "c"),
     manifests(a={"dev-dependencies": {"c": {"workspace": True}}}), "published after it"),
    ("a target-specific dev-dependency published later", ("a", "b", "c"),
     manifests(a={"target": {"cfg(unix)": {"dev-dependencies": {"b": {"workspace": True}}}}}),
     "published after it"),
    ("a direct version requirement published later", ("a", "b", "c"),
     manifests(a={"dev-dependencies": {"b": {"path": "../b", "version": "0.1"}}}), "published after it"),
    ("a renamed dependency published later", ("a", "b", "c"),
     manifests(a={"dev-dependencies": {"bee": {"package": "b", "path": "../b", "version": "0.1"}}}),
     "published after it"),
    ("a path-only dev-dependency on an unpublished crate", ("a", "b", "c"),
     manifests(a={"dev-dependencies": {"t": {"workspace": True}}}), None),
    ("a path-only dev-dependency published later", ("a", "b", "c"),
     manifests(a={"dev-dependencies": {"c": {"path": "../c"}}}), None),
    ("a versioned dependency on an unpublished crate", ("a", "b", "c"),
     manifests(b={"dev-dependencies": {"t": {"path": "../t", "version": "0.1"}}}), "never published"),
    ("a published crate missing from the order", ("a", "b"), manifests(), "not in the publish order"),
    ("a cycle", ("a", "b", "c"), manifests(a={"dev-dependencies": {"c": {"workspace": True}}}), "cycle"),
]


def order_problems() -> list[str]:
    problems = []
    for label, order, crates, expect in ORDER_CASES:
        found = release_check.publish_order_problems(order, crates, WORKSPACE)
        if expect is None and found:
            problems.append(f"order, {label}: rejected: {found}")
        elif expect is not None and not any(expect in p for p in found):
            problems.append(f"order, {label}: not rejected with {expect!r}: {found}")
    real = {name: manifest for name, (_, manifest) in release_check.members().items()}
    workspace = release_check.toml_file("Cargo.toml")["workspace"]["dependencies"]
    found = release_check.publish_order_problems(release_check.PUBLISH_ORDER, real, workspace)
    if found:
        problems.append(f"order: the real PUBLISH_ORDER is rejected: {found}")
    return problems


def main() -> int:
    texts = release_workflows.load()
    problems = order_problems() + notes_problems()
    base = release_workflows.problems(texts)
    if base:
        problems.append("the real workflows fail the check:\n    " + "\n    ".join(base))
    caught = 0
    for label, name, old, new, expect in MUTATIONS:
        text = texts.get(name, "")
        if text.count(old) < 1:
            problems.append(f"{label}: the text to mutate is not in {name} (update the table)")
            continue
        mutated = dict(texts, **{name: text.replace(old, new, 1)})
        found = release_workflows.problems(mutated)
        if not any(expect in p for p in found):
            problems.append(f"{label}: not caught (expected a report with {expect!r}; got {found})")
        else:
            caught += 1
    if problems:
        print("DIST-R10 workflow mutations:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"DIST-R10 workflow mutations OK: {caught} of {len(MUTATIONS)} caught, the real workflows pass; "
          f"release notes: {len(NOTES_CASES)} link cases, no relative link in the 1.0.0 notes; "
          f"publish order: {len(ORDER_CASES)} manifest cases, the real order accepted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
