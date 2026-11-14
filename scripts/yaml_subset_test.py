#!/usr/bin/env python3
"""Tests of the workflow YAML parser (SPEC-008 DIST-R10): what it reads, what it refuses,
and that every workflow in `.github/workflows/` parses.

    python3 scripts/yaml_subset_test.py
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from yaml_subset import YamlError, parse  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

READS = [
    ("scalars", "a: 1\nb: true\nc: false\nd: ~\ne: plain text\nf: 'it''s'\ng: \"x\\ty\"\n",
     {"a": 1, "b": True, "c": False, "d": None, "e": "plain text", "f": "it's", "g": "x\ty"}),
    ("comments", "# head\na: 1 # trailing\nb: 'q # kept' # gone\nc: x#y\n",
     {"a": 1, "b": "q # kept", "c": "x#y"}),
    ("nesting", "on:\n  push:\n    tags: [\"v*\"]\n  workflow_dispatch:\n",
     {"on": {"push": {"tags": ["v*"]}, "workflow_dispatch": None}}),
    ("sequence of mappings",
     "steps:\n  - uses: a/b@c # v1\n    with:\n      x: 1\n  - name: two\n    run: echo hi\n",
     {"steps": [{"uses": "a/b@c", "with": {"x": 1}}, {"name": "two", "run": "echo hi"}]}),
    ("sequence at the key's indent", "a:\n- 1\n- two\nb: 3\n", {"a": [1, "two"], "b": 3}),
    ("literal block", "run: |\n  set -e\n  echo \"#not a comment\"\n\n  done\nnext: 1\n",
     {"run": "set -e\necho \"#not a comment\"\n\ndone\n", "next": 1}),
    ("literal block, strip", "run: |-\n  one\n    two\n", {"run": "one\n  two"}),
    ("folded block", "msg: >\n  one\n  two\n\n  three\n", {"msg": "one two\n\nthree\n"}),
    ("flow collections", "a: [x, 'y, z', 3]\nb: {}\nc: {k: v}\nd: []\n",
     {"a": ["x", "y, z", 3], "b": {}, "c": {"k": "v"}, "d": []}),
    ("expressions", "if: ${{ github.event_name == 'release' }}\nenv:\n  T: ${{ secrets.X }}\n",
     {"if": "${{ github.event_name == 'release' }}", "env": {"T": "${{ secrets.X }}"}}),
    ("colon in a value", "run: echo a: b\n", {"run": "echo a: b"}),
    ("empty item", "a:\n  -\n  - 2\n", {"a": [None, 2]}),
]

REFUSES = [
    ("anchor", "a: &x 1\nb: *x\n"),
    ("tag", "a: !!str 1\n"),
    ("complex key", "? a\n: b\n"),
    ("two documents", "a: 1\n---\nb: 2\n"),
    ("tab indentation", "a:\n\tb: 1\n"),
    ("nested flow", "a: [[1], 2]\n"),
    ("unclosed flow", "a: [1, 2\n"),
    ("bad indentation", "a: 1\n   b: 2\n"),
    ("not a mapping line", "a: 1\njust text\n"),
    ("duplicate key", "a: 1\na: 2\n"),
    ("unterminated quote", "a: 'open\n"),
    ("text after a quote", "a: 'x' y\n"),
]


def main() -> int:
    problems = []
    for name, text, want in READS:
        try:
            got = parse(text)
        except YamlError as e:
            problems.append(f"{name}: refused: {e}")
            continue
        if got != want:
            problems.append(f"{name}: got {got!r}, want {want!r}")
    for name, text in REFUSES:
        try:
            got = parse(text)
            problems.append(f"{name}: accepted as {got!r}")
        except YamlError:
            pass
    workflows = sorted((ROOT / ".github/workflows").glob("*.yml"))
    for path in workflows:
        try:
            data = parse(path.read_text())
        except YamlError as e:
            problems.append(f"{path.name}: {e}")
            continue
        if not isinstance(data, dict) or "jobs" not in data or "on" not in data:
            problems.append(f"{path.name}: no `on` and `jobs` at the top")
    if problems:
        print("DIST-R10 yaml subset:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"DIST-R10 yaml subset OK ({len(READS)} read, {len(REFUSES)} refused, "
          f"{len(workflows)} workflows parse)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
