#!/usr/bin/env python3
"""Render the RND-R19 colour-blind review sheets.

RND-R19 requires sign-off on the five busiest complementary-pair scenes at 80x24 and
200x50. This script renders each as one `--sheet` PNG (the preview's default times,
0 / 5 / 20 s, stacked) through the `colorblind` theme into `/tmp/colorblind-review`,
then checks that every file exists and is non-empty and prints its path.

It needs the `colorblind` theme (RND-R19), so run it after that lands:

    python3 scripts/colorblind_review.py

Stdlib only; it shells out to the same `preview` example the gallery uses.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

# Locate the repo root from this file: <root>/scripts/colorblind_review.py.
ROOT = Path(__file__).resolve().parent.parent
OUT = Path("/tmp/colorblind-review")

# RND-R19's review set, in the order the spec lists them.
SCENES = (
    "wave_interference",
    "ising_model",
    "electron_orbitals",
    "solar_surface",
    "action_potential",
)
SIZES = ("80x24", "200x50")

PREVIEW = [
    "cargo",
    "run",
    "-q",
    "--profile",
    "ci",
    "-p",
    "sciart-generators",
    "--example",
    "preview",
    "--",
]


def render(scene: str, size: str) -> tuple[Path, list[str]]:
    """Render one sheet; return (path, error messages)."""
    path = OUT / f"{scene}-{size}.png"
    cmd = [
        *PREVIEW,
        scene,
        "--size",
        size,
        "--theme",
        "colorblind",
        "--sheet",
        "--out",
        str(OUT / f"{scene}-{size}"),
    ]
    proc = subprocess.run(cmd, cwd=ROOT)
    if proc.returncode != 0:
        return path, [f"{scene} at {size}: preview exited {proc.returncode}"]
    if not path.is_file():
        return path, [f"{scene} at {size}: {path} was not written"]
    if path.stat().st_size == 0:
        return path, [f"{scene} at {size}: {path} is empty"]
    return path, []


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    failures: list[str] = []
    for scene in SCENES:
        for size in SIZES:
            path, errors = render(scene, size)
            written.append(path)
            failures.extend(errors)
    for path in written:
        print(path)
    if failures:
        for message in failures:
            print(f"colorblind-review: {message}", file=sys.stderr)
        print(
            "colorblind-review: the `colorblind` theme must exist (RND-R19)",
            file=sys.stderr,
        )
        return 1
    print(f"colorblind-review: {len(written)} sheets in {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
