#!/usr/bin/env python3
"""Move SPEC-002's per-scene sections into the five category files (GEN-R15, IMP-12).

The monolith `docs/specs/SPEC-002-generators.md` keeps the rules, the template and the
batch notes. Every registered scene section (`### <name>  (<category label>, …)`, heading
plus body) is moved into `docs/specs/SPEC-002-scenes-<crate>.md`, chosen by the heading's
category label, and sorted by scene name inside each file.

The split is idempotent: sections already in a category file are read back and rewritten,
so a second run changes nothing. It is a dry run unless `--apply` is given; `--apply` writes
the category files and the slimmed monolith, then regenerates the scene index by running
`scripts/spec_index.py` (GEN-R15).

    python3 scripts/spec_split.py            # show the plan, write nothing
    python3 scripts/spec_split.py --apply    # write the category files and monolith, then regenerate the index
    python3 scripts/spec_split.py --apply --specs /tmp/specs-copy   # try it on a copy
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

# Locate the repo root from this file: <root>/scripts/spec_split.py.
ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SPECS = ROOT / "docs" / "specs"
MONOLITH = "SPEC-002-generators.md"

# GEN-R15: each scene section's heading label selects its category file (the same mapping
# as `Category`'s scene crate, ADR-0023).
LABEL_TO_SUFFIX = {
    "Deep Cosmos": "cosmos",
    "Sun & Planets": "cosmos",
    "Earth & Sky": "earth",
    "Voyages": "earth",
    "Forces & Waves": "physics",
    "Reactions": "matter",
    "States of Matter": "matter",
    "Living Systems": "life",
    "Physics of Life": "life",
    "Molecules of Life": "life",
    "Life from Matter": "life",
}
SUFFIXES = ("cosmos", "earth", "physics", "matter", "life")
LABELS_IN_FILE = {
    suffix: [label for label, mapped in LABEL_TO_SUFFIX.items() if mapped == suffix]
    for suffix in SUFFIXES
}

# `Category::id()` (what `sciart list` prints) -> `Category::label()` (the heading label).
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

# `### name  (label, milestones)`; the template's `### <name>  (category label, batch)` is
# not a registered scene and stays in the monolith.
SCENE_HEADING = re.compile(r"^### (?P<name>\S+)  \((?P<label>[^,()]+),")


def fail(message: str) -> NoReturn:
    sys.exit(f"spec-split: {message}")


def scene_file(specs: Path, suffix: str) -> Path:
    return specs / f"SPEC-002-scenes-{suffix}.md"


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
            f"spec-split: {relative} list failed ({proc.stderr.strip()}), "
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


def is_heading(line: str) -> bool:
    return line.startswith("## ") or line.startswith("### ")


def read_lines(path: Path) -> list[str]:
    try:
        return path.read_text(encoding="utf-8").split("\n")
    except OSError as error:
        fail(f"reading {path}: {error}")


def extract_sections(lines: list[str], registered: set[str], source: Path):
    """Split `lines` into kept layout lines and scene sections.

    A section is its heading plus the body up to the next `##`/`###`; trailing blank lines
    are layout (dropped) and a trailing `---` separator stays in the kept lines.
    """
    kept: list[str] = []
    sections: dict[str, tuple[str, str, list[str]]] = {}
    i = 0
    while i < len(lines):
        match = SCENE_HEADING.match(lines[i])
        if match and match.group("name") in registered:
            name = match.group("name")
            if name in sections:
                fail(f"{source}: {name} has more than one scene section")
            label = match.group("label").strip()
            j = i + 1
            while j < len(lines) and not is_heading(lines[j]):
                j += 1
            body = lines[i + 1 : j]
            while body and body[-1].strip() == "":
                body.pop()
            trailing: list[str] = []
            while body and body[-1].strip() == "---":
                trailing.insert(0, body.pop())
                while body and body[-1].strip() == "":
                    body.pop()
            sections[name] = (lines[i], label, body)
            kept.extend(trailing)
            i = j
            continue
        kept.append(lines[i])
        i += 1
    return kept, sections


def render_category(suffix: str, sections: dict[str, tuple[str, str, list[str]]]) -> str:
    """The deterministic category-file text: one header, then sections sorted by name."""
    labels = ", ".join(LABELS_IN_FILE[suffix])
    lines = [
        f"# SPEC-002: scenes — {suffix}",
        "",
        f"Scene sections for {labels} (GEN-R15), moved out of `{MONOLITH}` by",
        "`scripts/spec_split.py`. Edit a scene here and regenerate the scene index with",
        "`python3 scripts/spec_index.py`.",
    ]
    for name in sorted(sections):
        heading, _label, body = sections[name]
        lines.append("")
        lines.append(heading)
        lines.extend(body)
    lines.append("")
    return "\n".join(lines)


def merge_category_file(
    path: Path,
    suffix: str,
    registered: dict[str, str],
    moved: dict[str, tuple[str, str, list[str]]],
) -> dict[str, tuple[str, str, list[str]]]:
    """Existing sections for scenes not being moved, plus the moved ones (moved wins)."""
    existing: dict[str, tuple[str, str, list[str]]] = {}
    if path.is_file():
        _kept, existing = extract_sections(read_lines(path), set(registered), path)
    for name, (_heading, label, _body) in sorted(existing.items()):
        if label != registered[name]:
            fail(
                f"{path}: {name} has heading label {label!r}, "
                f"the registry says {registered[name]!r}"
            )
    wanted = {name for name, label in registered.items() if LABEL_TO_SUFFIX.get(label) == suffix}
    for name, section in moved.items():
        if LABEL_TO_SUFFIX[section[1]] == suffix:
            existing[name] = section
    unknown = sorted(existing.keys() - wanted)
    if unknown:
        fail(f"{path}: sections name no registered scene in this category: {unknown}")
    return {name: existing[name] for name in existing if name in wanted}


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Move SPEC-002 scene sections into the category files (GEN-R15)."
    )
    parser.add_argument(
        "--apply", action="store_true", help="write the files (default is a dry run)"
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

    monolith_lines = read_lines(monolith_path)
    kept, moved = extract_sections(monolith_lines, set(registered), monolith_path)
    if not moved:
        print(
            "spec-split: no scene sections left in the monolith; "
            "the split has already run"
        )
    # Every moved section must carry the label the registry gives the scene (GEN-R15).
    problems = []
    for name, (_heading, label, _body) in sorted(moved.items()):
        want = registered[name]
        if label != want:
            problems.append(f"{name}: heading label {label!r}, registry says {want!r}")
        if label not in LABEL_TO_SUFFIX:
            problems.append(f"{name}: unknown category label {label!r}")
    if problems:
        fail("bad scene headings (run the GEN-R15 test):\n  " + "\n  ".join(problems))

    category: dict[str, dict[str, tuple[str, str, list[str]]]] = {}
    for suffix in SUFFIXES:
        category[suffix] = merge_category_file(
            scene_file(specs, suffix), suffix, registered, moved
        )
    # After the merge every registered scene must live in exactly one category file.
    placed = {name for sections in category.values() for name in sections}
    missing = sorted(set(registered) - placed)
    if missing:
        fail(f"{len(missing)} registered scene(s) have no section: {missing}")
    extra = sorted(placed - set(registered))
    if extra:
        fail(f"category files name scenes that are not registered: {extra}")

    total = sum(len(sections) for sections in category.values())
    print(f"spec-split: {len(moved)} scene section(s) in {monolith_path}")
    for suffix in SUFFIXES:
        print(
            f"  {suffix:<8} {len(category[suffix]):>2} scene(s) -> "
            f"{scene_file(specs, suffix).name}"
        )
    print(f"  total    {total} registered scene(s)")

    monolith_text = "\n".join(kept).rstrip("\n") + "\n"
    if not args.apply:
        print("spec-split: dry run; rerun with `--apply` to write the files")
        return 0

    wrote = 0
    for suffix in SUFFIXES:
        path = scene_file(specs, suffix)
        text = render_category(suffix, category[suffix])
        if not path.is_file() or path.read_text(encoding="utf-8") != text:
            path.write_text(text, encoding="utf-8")
            wrote += 1
    if moved or monolith_path.read_text(encoding="utf-8") != monolith_text:
        monolith_path.write_text(monolith_text, encoding="utf-8")
        wrote += 1
    print(f"spec-split: wrote {wrote} file(s); regenerating the scene index", flush=True)

    index = Path(__file__).with_name("spec_index.py")
    proc = subprocess.run(
        [sys.executable, str(index), "--specs", str(specs)], cwd=ROOT
    )
    return proc.returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
