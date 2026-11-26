#!/usr/bin/env python3
"""Build the conda package with rattler-build (SPEC-008 DIST-R8).

Sets `SCIART_VERSION` from the root Cargo.toml (DIST-R1) and `SCIART_GIT_SHA` from the
checked-out commit (DIST-R2), then runs

    rattler-build build --recipe packaging/conda/recipe.yaml --output-dir target/conda <args>

Extra arguments go to rattler-build (`--target-platform linux-aarch64`). Uploading is not
done here: only the publish workflow uploads, after a maintainer's approval (DIST-R10).
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    tool = shutil.which("rattler-build")
    if tool is None:
        print("DIST-R8: rattler-build is not installed; run `pixi run -e dist rs-conda-build`",
              file=sys.stderr)
        return 1
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    env = dict(os.environ, SCIART_VERSION=version)
    try:
        env["SCIART_GIT_SHA"] = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, check=True, capture_output=True, text=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        env.pop("SCIART_GIT_SHA", None)  # not a git checkout: --version without a commit
    command = [tool, "build", "--recipe", str(ROOT / "packaging/conda/recipe.yaml"),
               "--output-dir", str(ROOT / "target/conda"), *sys.argv[1:]]
    print(f"SCIART_VERSION={version} {' '.join(command)}", flush=True)
    return subprocess.run(command, cwd=ROOT, env=env).returncode


if __name__ == "__main__":
    raise SystemExit(main())
