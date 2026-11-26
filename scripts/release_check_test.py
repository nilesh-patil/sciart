#!/usr/bin/env python3
"""Mutation tests of the workflow checks (SPEC-008 DIST-R6, DIST-R10).

Each row breaks the real workflows in one way a careless edit or a hostile change could,
and the check must report it; the unmodified workflows must pass. A row whose text no
longer appears in the workflow fails too, so the table cannot rot silently.

    python3 scripts/release_check_test.py
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
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
CHECKOUT = "actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0"
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
    ("the conda job outside the release environment", "publish.yml",
     "    runs-on: ${{ matrix.os }}\n    environment: release\n", "    runs-on: ${{ matrix.os }}\n",
     "`conda` runs in the `release` environment"),
    ("a channel upload that fails on a rerun", "publish.yml", "upload prefix --skip-existing", "upload prefix",
     "--skip-existing"),
    ("a tap commit that fails on a rerun", "publish.yml", "          if git diff --cached --quiet; then\n"
     "            echo \"the tap already has this formula\"\n          else\n            git commit -m \"sciart ${TAG#v}\"\n"
     "            git push\n          fi\n", "          git commit -m \"sciart ${TAG#v}\"\n          git push\n",
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
    ("package may write contents", "release.yml", "      contents: read\n      id-token: write\n",
     "      contents: write\n      id-token: write\n", "beyond its allow-list"),
    ("the workflow default widened", "release.yml", "permissions:\n  contents: read\n\nenv:",
     "permissions:\n  contents: write\n\nenv:", "contents: read"),
    ("j: release does not need package", "release.yml", "    needs: [verify, package]\n", "    needs: [verify]\n",
     "build -> package -> release"),
    ("k: package does not need build", "release.yml", "    needs: [verify, build]\n", "    needs: [verify]\n",
     "build -> package -> release"),
    ("l: a third-party action swapped in", "release.yml",
     "taiki-e/install-action@893d6ee8d714b6c7dfe2bcd94e5759e9a7958a7e # v2.90.10",
     "someone-else/install-action@main", "not pinned"),
    ("l: a pinned action from an unknown owner", "release.yml",
     "taiki-e/install-action@893d6ee8d714b6c7dfe2bcd94e5759e9a7958a7e # v2.90.10",
     "someone-else/install-action@893d6ee8d714b6c7dfe2bcd94e5759e9a7958a7e # v2.90.10", "allow-list"),
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


def main() -> int:
    texts = release_workflows.load()
    problems = []
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
    print(f"DIST-R10 workflow mutations OK: {caught} of {len(MUTATIONS)} caught, the real workflows pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
