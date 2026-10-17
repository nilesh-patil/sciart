#!/usr/bin/env python3
"""Generate or check docs/science/claims.md (IMP-17).

The skeleton has one row per registered scene. The scene list comes from `sciart list`
(which is the registry), the title from the scene's `pub static META` in its source file,
and the model sentence from the scene's section in the SPEC-002 category files.

The generated rows are a starting point: the audited scenes are then filled in by hand
(caveat and review date). `--check` verifies that every registered scene still has a row
without overwriting the filled-in file. Stdlib only; run from anywhere.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SPECS = ROOT / "docs" / "specs"
OUT = ROOT / "docs" / "science" / "claims.md"


def scene_list() -> list[tuple[str, str, str]]:
    """`(name, category, description)` for every registered scene, sorted by name."""
    result = subprocess.run(
        ["cargo", "run", "-q", "-p", "sciart", "--", "list"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    rows = []
    for line in result.stdout.splitlines():
        name, category, description = line.split("\t", 2)
        rows.append((name, category, description))
    rows.sort()
    return rows


def spec_models() -> dict[str, str]:
    """First Model sentence per scene, from the SPEC-002 category files."""
    models: dict[str, str] = {}
    for path in sorted(SPECS.glob("SPEC-002-scenes-*.md")):
        text = path.read_text(encoding="utf-8")
        for match in re.finditer(r"(?ms)^### (\S+)\b.*?(?=^### |\Z)", text):
            name = match.group(1)
            body = match.group(0)
            model = re.search(r"(?m)^- \*\*Model\*\*: (.+)$", body)
            if model:
                models[name] = model.group(1).strip()
    return models


def source_title(scene: str) -> str:
    """The `title:` in the scene's `pub static META`."""
    for path in (ROOT / "crates").glob(f"sciart-scenes-*/src/{scene}.rs"):
        text = path.read_text(encoding="utf-8")
        match = re.search(r'title:\s*"([^"]+)"', text)
        if match:
            return match.group(1)
    return "—"


def first_sentence(text: str) -> str:
    """One sentence, safe for a Markdown table cell."""
    text = text.strip().replace("|", "/")
    cut = text.find(". ")
    if cut != -1:
        text = text[: cut + 1]
    return text or "—"


def row(scene: str, category: str, title: str, model: str) -> str:
    return (
        f"| `{scene}` | {title} | `{category}` | {first_sentence(model)} "
        "| — | — |"
    )


def skeleton() -> str:
    models = spec_models()
    lines = [
        "# Science claims",
        "",
        "One row per registered scene: the title, the model it claims, and what the art",
        "exaggerates or simplifies. The audited rows are filled in; `—` in an unaudited row",
        "means the caveat has not been reviewed yet (see [README](README.md)).",
        "",
        "Generated from `sciart list`, each scene's `META.title`, and the",
        "SPEC-002 category files (`scripts/science_claims.py`); checked by",
        "`python3 scripts/science_claims.py --check`.",
        "",
        "| Scene | Title | Category | Model (one sentence) | Stylisation / caveat | Reviewed |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for scene, category, description in scene_list():
        model = models.get(scene, "—")
        if model in ("", "—"):
            model = description
        lines.append(row(scene, category, source_title(scene), model))
    lines.append("")
    return "\n".join(lines)


def check() -> int:
    if not OUT.exists():
        print(f"missing {OUT}", file=sys.stderr)
        return 1
    text = OUT.read_text(encoding="utf-8")
    rows = set(re.findall(r"(?m)^\| `([a-z0-9_]+)` \|", text))
    scenes = {name for name, _, _ in scene_list()}
    missing = sorted(scenes - rows)
    extra = sorted(rows - scenes)
    if missing:
        print("missing rows: " + ", ".join(missing), file=sys.stderr)
    if extra:
        print("rows for unregistered scenes: " + ", ".join(extra), file=sys.stderr)
    return 1 if missing or extra else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write",
        action="store_true",
        help="write the skeleton (fails if the file exists unless --force)",
    )
    parser.add_argument("--force", action="store_true", help="overwrite an existing file")
    parser.add_argument("--check", action="store_true", help="verify every scene has a row")
    args = parser.parse_args()
    if args.check:
        return check()
    if not args.write:
        print(skeleton())
        return 0
    if OUT.exists() and not args.force:
        print(f"{OUT} exists; use --force to overwrite", file=sys.stderr)
        return 1
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(skeleton(), encoding="utf-8")
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
