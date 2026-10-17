#!/usr/bin/env python3
"""Regenerate the SPEC-002 scene index (GEN-R15, IMP-12).

`docs/specs/SPEC-002-generators.md` keeps a generated table of every registered scene,
its category label and the category file it lives in, between

    <!-- scene-index:start -->
    | Scene | Category | Spec |
    |---|---|---|
    ...
    <!-- scene-index:end -->

The table is rebuilt from the five `docs/specs/SPEC-002-scenes-*.md` files, sorted by scene
name, and checked against the registry: every registered scene must appear exactly once and
no other scene may appear. If the markers are missing (before the first split) the section is
inserted before the first `## Batch …` heading.

    python3 scripts/spec_index.py            # regenerate (writes only when it changed)
    python3 scripts/spec_index.py --check    # exit 1 if the index is stale or wrong
    python3 scripts/spec_index.py --check --specs /tmp/specs-copy
"""

from __future__ import annotations

import argparse
import difflib
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

# Locate the repo root from this file: <root>/scripts/spec_index.py.
ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SPECS = ROOT / "docs" / "specs"
MONOLITH = "SPEC-002-generators.md"
START = "<!-- scene-index:start -->"
END = "<!-- scene-index:end -->"
SECTION_HEADING = "## Scene index"

# `Category::id()` (what `sciart list` prints) -> `Category::label()` (the heading label).
# Keep in step with `Category::label()` and with scripts/spec_split.py.
ID_TO_LABEL = {
    "deep-cosmos": "Deep Cosmos",
    "sun-planets": "Sun & Planets",
    "earth-sky": "Earth & Sky",
    "voyages": "Voyages",
    "forces-waves": "Forces & Waves",
    "reactions": "Reactions",
    "living-systems": "Living Systems",
    "states-of-matter": "States of Matter",
    "physics-of-life": "Physics of Life",
    "molecules-of-life": "Molecules of Life",
    "life-from-matter": "Life from Matter",
}

SCENE_HEADING = re.compile(r"^### (?P<name>\S+)  \((?P<label>[^,()]+),")


def fail(message: str) -> NoReturn:
    sys.exit(f"spec-index: {message}")


def registry_scenes() -> dict[str, str]:
    """Registered scene name -> category label, from `sciart list` (or a cargo fallback)."""
    for relative in ("target/ci/sciart", "target/debug/sciart", "target/release/sciart"):
        binary = ROOT / relative
        if not (binary.is_file() and os.access(binary, os.X_OK)):
            continue
        proc = subprocess.run(
            [str(binary), "list"], cwd=ROOT, capture_output=True, text=True
        )
        if proc.returncode == 0:
            return parse_registry(proc.stdout)
        print(
            f"spec-index: {relative} list failed ({proc.stderr.strip()}), "
            "trying the next binary",
            file=sys.stderr,
        )
    proc = subprocess.run(
        ["cargo", "run", "-q", "-p", "sciart", "--", "list"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        fail(f"`cargo run -q -p sciart -- list` failed:\n{proc.stderr.strip()}")
    return parse_registry(proc.stdout)


def parse_registry(tsv: str) -> dict[str, str]:
    scenes: dict[str, str] = {}
    for line in tsv.splitlines():
        if not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) < 3:
            fail(f"bad `sciart list` line: {line!r}")
        name, category = fields[0], fields[1]
        if name in scenes:
            fail(f"`sciart list` repeats {name}")
        scenes[name] = ID_TO_LABEL.get(category, category)
    if not scenes:
        fail("`sciart list` printed no scenes")
    return scenes


def collect(specs: Path, registered: dict[str, str]) -> tuple[dict[str, tuple[str, str]], list[str]]:
    """Scene name -> (category label, category file name) from the category files."""
    sections: dict[str, tuple[str, str]] = {}
    problems: list[str] = []
    paths = sorted(specs.glob("SPEC-002-scenes-*.md"))
    if not paths:
        problems.append(
            f"no SPEC-002-scenes-*.md files in {specs}; "
            "run `python3 scripts/spec_split.py --apply` first"
        )
    for path in paths:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            fail(f"reading {path}: {error}")
        for line in text.splitlines():
            match = SCENE_HEADING.match(line)
            if not match:
                continue
            name = match.group("name")
            label = match.group("label").strip()
            if name in sections:
                previous = sections[name][1]
                if previous == path.name:
                    problems.append(f"{name}: appears more than once in {path.name}")
                else:
                    problems.append(f"{name}: appears in both {previous} and {path.name}")
            sections[name] = (label, path.name)
    for name, (label, file) in sorted(sections.items()):
        if name not in registered:
            problems.append(f"{name}: in {file} but not in the registry")
        elif label != registered[name]:
            problems.append(
                f"{name}: heading label {label!r}, registry says {registered[name]!r}"
            )
    missing = sorted(set(registered) - set(sections))
    if missing:
        problems.append(
            f"{len(missing)} registered scene(s) have no section: {', '.join(missing)}"
        )
    return sections, problems


def table(sections: dict[str, tuple[str, str]]) -> str:
    """The generated table: `Scene | Category | Spec`, sorted by scene name."""
    lines = ["| Scene | Category | Spec |", "|---|---|---|"]
    for name in sorted(sections):
        label, file = sections[name]
        stem = file.removeprefix("SPEC-002-").removesuffix(".md")
        lines.append(f"| `{name}` | {label} | [{stem}]({file}) |")
    return "\n".join(lines)


def block(sections: dict[str, tuple[str, str]]) -> str:
    return f"{START}\n{table(sections)}\n{END}"


def current_block(monolith: str) -> tuple[str, int, int] | None:
    """The existing index region (markers included), or None when there are no markers."""
    start = monolith.find(START)
    end = monolith.find(END)
    if start == -1 and end == -1:
        return None
    if start == -1 or end == -1:
        fail(f"{MONOLITH}: both {START} and {END} are required")
    if start > end:
        fail(f"{MONOLITH}: {START} comes after {END}")
    end += len(END)
    return monolith[start:end], start, end


def with_index(monolith: str, generated: str) -> str:
    """Replace the region between the markers, or insert a `## Scene index` section."""
    existing = current_block(monolith)
    if existing is not None:
        _old, start, end = existing
        return monolith[:start] + generated + monolith[end:]
    if SECTION_HEADING in monolith:
        fail(
            f"{MONOLITH} has a `{SECTION_HEADING}` heading but no {START}/{END} markers"
        )
    section = f"{SECTION_HEADING}\n\n{generated}\n\n"
    # Keep the index next to the rules: immediately before the first batch section.
    marker = "\n## Batch "
    pos = monolith.find(marker)
    if pos == -1:
        return monolith.rstrip("\n") + "\n\n" + section
    return monolith[:pos].rstrip("\n") + "\n\n" + section + monolith[pos + 1 :]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Regenerate the SPEC-002 scene index (GEN-R15)."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; exit 1 if the index is stale or the scene set is wrong",
    )
    parser.add_argument(
        "--specs",
        type=Path,
        default=DEFAULT_SPECS,
        help=f"spec directory (default: {DEFAULT_SPECS})",
    )
    args = parser.parse_args(argv)
    specs: Path = args.specs
    monolith_path = specs / MONOLITH
    if not monolith_path.is_file():
        fail(f"{monolith_path} not found")
    registered = registry_scenes()
    sections, problems = collect(specs, registered)
    if problems:
        fail("the scene set does not match the registry:\n  " + "\n  ".join(problems))

    generated = block(sections)
    monolith = monolith_path.read_text(encoding="utf-8")
    existing = current_block(monolith)

    if args.check:
        if existing is None:
            fail(
                f"{MONOLITH} has no scene index; run `python3 scripts/spec_index.py` "
                "(or `spec_split.py --apply`)"
            )
        old, _start, _end = existing
        if old != generated:
            diff = list(
                difflib.unified_diff(
                    old.splitlines(),
                    generated.splitlines(),
                    fromfile="index (current)",
                    tofile="index (generated)",
                    lineterm="",
                )
            )
            shown = "\n".join(diff[:40])
            fail(f"the scene index is stale; run `python3 scripts/spec_index.py`\n{shown}")
        print(f"spec-index: ok, {len(sections)} scenes")
        return 0

    updated = with_index(monolith, generated)
    if updated == monolith:
        print(f"spec-index: up to date, {len(sections)} scenes")
        return 0
    monolith_path.write_text(updated, encoding="utf-8")
    print(f"spec-index: wrote {len(sections)} scenes to {monolith_path.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
