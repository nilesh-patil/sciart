#!/usr/bin/env python3
"""The Homebrew formula of a release (SPEC-008 DIST-R9).

Fills `packaging/homebrew/sciart.rb.in` with the version, the release download URL and,
for each macOS and Linux architecture, the tarball's checksum from the release's
`SHA256SUMS`. Every placeholder must be filled and every checksum must be in the file, or
nothing is written.

    python3 scripts/render_formula.py --version 1.0.0 --sums SHA256SUMS [--base URL] [--out FILE]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_assets  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = ROOT / "packaging" / "homebrew" / "sciart.rb.in"
BASE = "https://github.com/nilesh-patil/sciart/releases"
# The Linux bottles are the glibc builds: Homebrew on Linux brings a recent glibc.
FORMULA_TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin",
                   "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu")


def parse_sums(text: str) -> dict[str, str]:
    sums = {}
    for line in text.splitlines():
        found = re.fullmatch(r"([0-9a-f]{64})  (\S+)", line.strip())
        if found:
            sums[found.group(2)] = found.group(1)
    return sums


def render(version: str, sums_text: str, base: str = BASE, template: str | None = None) -> str:
    if not release_assets.SEMVER.match(version):
        raise ValueError(f"not a semantic version: {version!r}")
    text = template if template is not None else TEMPLATE.read_text()
    sums = parse_sums(sums_text)
    text = text.replace("@VERSION@", version).replace("@BASE@", base.rstrip("/"))
    for target in FORMULA_TARGETS:
        name = release_assets.asset_name(version, target)
        if name not in sums:
            raise ValueError(f"SHA256SUMS lists no {name}")
        text = text.replace(f"@SHA256:{target}@", sums[name])
    left = re.findall(r"@[A-Z0-9:_a-z-]+@", text)
    if left:
        raise ValueError(f"unfilled placeholders: {sorted(set(left))}")
    return text


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--sums", type=Path, required=True)
    parser.add_argument("--base", default=BASE)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        formula = render(args.version, args.sums.read_text(), args.base)
    except ValueError as e:
        print(f"DIST-R9: {e}", file=sys.stderr)
        return 1
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(formula)
    else:
        sys.stdout.write(formula)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
