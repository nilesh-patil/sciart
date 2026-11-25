#!/usr/bin/env python3
"""Size and weight budget of the release binary (SPEC-003 RT-R22, ADR-0035).

Checks, after `cargo build --release -p sciart --bin sciart` (no `record` or `mouse`
feature; `pixi run rs-size` builds it first):

- the binary is the one this checkout's sources build: cargo's dep-info file next to it
  (`sciart.d`) lists only sources of this checkout, none of them newer than the binary,
  and none of the sources only the `record` or `mouse` feature compiles. A stale or
  feature-built binary is refused, not measured;
- the binary is at most `BINARY_BUDGET` bytes, on `aarch64-apple-darwin`, the platform the
  budget was measured on (elsewhere the size is printed, not enforced);
- the default binary's dependency graph (`cargo tree -p sciart -e normal`, one entry per
  crate name and version) has at most `CRATE_BUDGET` crates;
- `ratatui`, `clap` and `toml` are declared with `default-features = false`, so they are
  built with only the features the code uses, and the release profile strips symbols.

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

# RT-R22 budgets (M25, ADR-0035: measured 3 213 264 bytes and 93 crates; with the CLI-R17
# completion and manual generators, 3 600 256 bytes and 96 crates).
BINARY_BUDGET = 3_604_480  # bytes (55 x 64 KiB), aarch64-apple-darwin, release profile
CRATE_BUDGET = 96  # crates in `cargo tree -p sciart -e normal`

TRIMMED = ("clap", "ratatui", "toml")

# Sources that only a feature build compiles: their presence in the dep-info file means
# the binary was built with that feature (RT-R22 measures the binary without either).
FEATURE_SOURCES = {
    "record": ("crates/sciart-capture/", "crates/sciart/src/record.rs"),
    "mouse": ("crates/sciart-keepawake/src/mouse.rs",),
}


def enforced_platform() -> bool:
    return sys.platform == "darwin" and platform.machine() == "arm64"


def parse_depinfo(text: str) -> tuple[str, list[str]]:
    """The target and the prerequisites of a cargo dep-info (Makefile) rule.

    Spaces inside paths are escaped as `\\ `; the target ends at the first unescaped
    `: `. Only the first rule counts: cargo writes the binary's rule first.
    """
    for line in text.splitlines():
        if not line.strip():
            continue
        tokens: list[str] = []
        current = []
        i = 0
        while i < len(line):
            c = line[i]
            if c == "\\" and i + 1 < len(line):
                current.append(line[i + 1])
                i += 2
                continue
            if c == " ":
                if current:
                    tokens.append("".join(current))
                    current = []
            else:
                current.append(c)
            i += 1
        if current:
            tokens.append("".join(current))
        if not tokens or not tokens[0].endswith(":"):
            raise ValueError(f"not a dep-info rule: {line[:80]!r}")
        return tokens[0][:-1], tokens[1:]
    raise ValueError("empty dep-info file")


def judge_binary(binary: Path, root: Path) -> list[str]:
    """Why `binary` cannot stand for this checkout's default release build (RT-R22), or
    nothing when it can."""
    depinfo = binary.with_name(binary.name + ".d")
    if not depinfo.is_file():
        return [f"RT-R22: no dep-info file {depinfo}: cannot tell what {binary} was built from"]
    try:
        _, sources = parse_depinfo(depinfo.read_text())
    except ValueError as e:
        return [f"RT-R22: {depinfo}: {e}"]
    root = root.resolve()
    problems: list[str] = []
    relative: list[str] = []
    for src in sources:
        path = Path(src).resolve()
        try:
            relative.append(path.relative_to(root).as_posix())
        except ValueError:
            # Registry crates are recorded only for local packages' sources; anything else
            # outside the checkout means the binary came from another tree.
            problems.append(f"RT-R22: {binary} was built from another checkout ({src})")
            break
    for feature, prefixes in FEATURE_SOURCES.items():
        hit = next((r for r in relative if any(r.startswith(p) for p in prefixes)), None)
        if hit:
            problems.append(f"RT-R22: {binary} was built with the `{feature}` feature ({hit})")
    built = binary.stat().st_mtime
    for rel in relative:
        path = root / rel
        if not path.exists():
            problems.append(f"RT-R22: {binary} is stale: {rel} no longer exists")
            break
        if path.stat().st_mtime > built:
            problems.append(f"RT-R22: {binary} is stale: {rel} is newer; rebuild it")
            break
    return problems


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


def manifest_problems(manifest: dict) -> list[str]:
    problems = []
    deps = manifest.get("workspace", {}).get("dependencies", {})
    for name in TRIMMED:
        spec = deps.get(name)
        if not isinstance(spec, dict) or spec.get("default-features", True):
            problems.append(f"RT-R22: workspace dependency `{name}` must set default-features = false")
    strip = manifest.get("profile", {}).get("release", {}).get("strip")
    if strip not in (True, "symbols"):
        problems.append(f"RT-R22: [profile.release] must strip symbols (strip = true), found {strip!r}")
    return problems


def main() -> int:
    problems: list[str] = []
    binary = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "release" / "sciart"
    if not binary.is_file():
        print(f"RT-R22: {binary} not found; build it with `cargo build --release -p sciart --bin sciart`")
        return 1
    refused = judge_binary(binary, ROOT)
    if refused:
        for p in refused:
            print(p, file=sys.stderr)
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

    problems += manifest_problems(tomllib.loads((ROOT / "Cargo.toml").read_text()))

    for p in problems:
        print(p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
