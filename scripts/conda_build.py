#!/usr/bin/env python3
"""Build the conda package with rattler-build (SPEC-008 DIST-R8).

Sets `SCIART_VERSION` from the root Cargo.toml (DIST-R1) and `SCIART_GIT_SHA` from the
checked-out commit (DIST-R2), then runs

    rattler-build build --recipe packaging/conda/recipe.yaml --output-dir target/conda <args>

The recipe builds the working tree, so a tree with uncommitted changes would produce a
package that no commit describes: it is refused unless `--allow-dirty` is given, and then
`--version` names no commit. Other arguments go to rattler-build
(`--target-platform linux-aarch64`). Uploading is not done here: only the publish
workflow uploads, after a maintainer's approval (DIST-R10).
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def uncommitted(root: Path) -> list[str] | None:
    """Changed and untracked files (ignored ones excepted), or None outside a git checkout."""
    try:
        out = subprocess.run(["git", "status", "--porcelain", "--untracked-files=normal"],
                             cwd=root, check=True, capture_output=True, text=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return [line[3:] for line in out.splitlines() if line.strip()]


def main(argv: list[str], root: Path = ROOT) -> int:
    allow_dirty = "--allow-dirty" in argv
    passed = [a for a in argv if a != "--allow-dirty"]
    changes = uncommitted(root)
    if changes is None and not allow_dirty:
        print("DIST-R8: not a git checkout, so the build cannot name its commit; "
              "pass --allow-dirty to build anyway", file=sys.stderr)
        return 1
    if changes and not allow_dirty:
        print(f"DIST-R8: uncommitted changes ({', '.join(changes[:5])}"
              f"{', ...' if len(changes) > 5 else ''}); commit them or pass --allow-dirty",
              file=sys.stderr)
        return 1
    tool = shutil.which("rattler-build")
    if tool is None:
        print("DIST-R8: rattler-build is not installed; run `pixi run -e dist rs-conda-build`",
              file=sys.stderr)
        return 1
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    env = dict(os.environ, SCIART_VERSION=version)
    env.pop("SCIART_GIT_SHA", None)
    if changes == []:
        env["SCIART_GIT_SHA"] = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True
        ).stdout.strip()
    command = [tool, "build", "--recipe", str(root / "packaging/conda/recipe.yaml"),
               "--output-dir", str(root / "target/conda"), *passed]
    print(f"SCIART_VERSION={version} {' '.join(command)}", flush=True)
    return subprocess.run(command, cwd=root, env=env).returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
