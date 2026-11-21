#!/usr/bin/env python3
"""Tests of the conda build wrapper's clean-tree rule (SPEC-008 DIST-R8), in a scratch git
repository and with no rattler-build on PATH, so nothing is built.

    python3 scripts/conda_build_test.py
"""

from __future__ import annotations

import contextlib
import io
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import conda_build  # noqa: E402

NOT_INSTALLED = "rattler-build is not installed"


def git(root: Path, *args: str) -> None:
    subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
                   cwd=root, check=True, capture_output=True)


def run(root: Path, *argv: str) -> tuple[int, str]:
    err = io.StringIO()
    path = os.environ.get("PATH", "")
    os.environ["PATH"] = "/usr/bin:/bin"  # git, but no rattler-build
    try:
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = conda_build.main(list(argv), root)
    finally:
        os.environ["PATH"] = path
    return code, err.getvalue()


def main() -> int:
    problems = []
    with tempfile.TemporaryDirectory(prefix="sciart-conda-") as tmp:
        root = Path(tmp)
        code, err = run(root)
        if code == 0 or "not a git checkout" not in err:
            problems.append(f"DIST-R8: outside a git checkout the build is refused: {code} {err!r}")
        git(root, "init", "-q")
        (root / ".gitignore").write_text("target/\n")
        (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n')
        git(root, "add", ".")
        git(root, "commit", "-q", "-m", "init")
        (root / "target").mkdir()
        (root / "target" / "ignored").write_text("x")
        code, err = run(root, "--target-platform", "linux-64")
        if NOT_INSTALLED not in err:
            problems.append(f"DIST-R8: a clean tree (ignored files aside) goes on to the build: {err!r}")
        (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.4"\n')
        code, err = run(root)
        if code == 0 or "uncommitted changes (Cargo.toml)" not in err:
            problems.append(f"DIST-R8: a modified file refuses the build: {code} {err!r}")
        git(root, "checkout", "--", "Cargo.toml")
        (root / "new.txt").write_text("x")
        code, err = run(root)
        if code == 0 or "new.txt" not in err:
            problems.append(f"DIST-R8: an untracked file refuses the build: {code} {err!r}")
        code, err = run(root, "--allow-dirty")
        if NOT_INSTALLED not in err:
            problems.append(f"DIST-R8: --allow-dirty builds a dirty tree: {err!r}")
    if problems:
        print("conda_build:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print("conda_build OK (outside git, clean, modified, untracked, --allow-dirty)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
