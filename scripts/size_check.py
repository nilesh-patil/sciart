#!/usr/bin/env python3
"""Size and weight budget of the release binary (SPEC-003 RT-R22).

Checks, after `cargo build --release -p sciart --bin sciart` (no `record` or `mouse`
feature; `pixi run rs-size` builds it first):

- the binary is at most `BINARY_BUDGET` bytes, on `aarch64-apple-darwin`, the platform the
  budget was measured on (elsewhere the size is printed, not enforced);
- the default binary's dependency graph (`cargo tree -p sciart -e normal`, one entry per
  crate name and version) has at most `CRATE_BUDGET` crates;
- `ratatui`, `clap` and `toml` are declared with `default-features = false`, so they are
  built with only the features the code uses.

The budgets may only be lowered without a spec change (RT-R22). Usage:

    python3 scripts/size_check.py [path/to/sciart]
"""

from __future__ import annotations

import platform
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# RT-R22 budgets (M25: the measured values plus a small margin).
BINARY_BUDGET = 3_670_016  # bytes, aarch64-apple-darwin, release profile
CRATE_BUDGET = 108  # crates in `cargo tree -p sciart -e normal`

TRIMMED = ("clap", "ratatui", "toml")


def enforced_platform() -> bool:
    return sys.platform == "darwin" and platform.machine() == "arm64"


def crate_count() -> tuple[int, list[str]]:
    out = subprocess.run(
        ["cargo", "tree", "-p", "sciart", "-e", "normal", "--prefix", "none"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    crates = set()
    for line in out.splitlines():
        line = line.replace(" (*)", "").strip()
        if not line:
            continue
        name, _, rest = line.partition(" ")
        crates.add(f"{name} {rest.split(' ')[0]}")
    return len(crates), sorted(crates)


def main() -> int:
    problems: list[str] = []
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "release" / "sciart"
    if not binary.is_file():
        print(f"RT-R22: {binary} not found; build it with `cargo build --release -p sciart --bin sciart`")
        return 1
    size = binary.stat().st_size
    if enforced_platform():
        verdict = "ok" if size <= BINARY_BUDGET else "OVER"
        print(f"RT-R22: binary {size} bytes (budget {BINARY_BUDGET}) {verdict}")
        if size > BINARY_BUDGET:
            problems.append(f"RT-R22: {binary} is {size} bytes, over the {BINARY_BUDGET} budget")
    else:
        print(f"RT-R22: binary {size} bytes (budget enforced on aarch64-apple-darwin only)")

    count, crates = crate_count()
    print(f"RT-R22: {count} crates in the default binary (budget {CRATE_BUDGET})")
    if count > CRATE_BUDGET:
        problems.append(f"RT-R22: {count} crates, over the {CRATE_BUDGET} budget:\n  " + "\n  ".join(crates))

    deps = tomllib.loads((ROOT / "Cargo.toml").read_text()).get("workspace", {}).get("dependencies", {})
    for name in TRIMMED:
        spec = deps.get(name)
        if not isinstance(spec, dict) or spec.get("default-features", True):
            problems.append(f"RT-R22: workspace dependency `{name}` must set default-features = false")

    for p in problems:
        print(p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
