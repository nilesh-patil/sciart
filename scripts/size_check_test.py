#!/usr/bin/env python3
"""Unit tests for scripts/size_check.py (RT-R22): the budget check refuses a binary it
cannot vouch for (stale, feature-built, from another checkout) instead of measuring it.

Usage: python3 scripts/size_check_test.py
"""

from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import size_check  # noqa: E402


class Tree:
    """A fake checkout with sources, a binary and its dep-info file."""

    def __init__(self, root: Path, sources: list[str]):
        self.root = root
        self.binary = root / "target" / "release" / "sciart"
        self.binary.parent.mkdir(parents=True)
        for rel in sources:
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// source\n")
            os.utime(path, (1_000, 1_000))
        self.binary.write_bytes(b"\0" * 16)
        os.utime(self.binary, (2_000, 2_000))
        self.write_depinfo([str(root / rel) for rel in sources])

    def write_depinfo(self, paths: list[str]) -> None:
        def esc(p: object) -> str:
            return str(p).replace(" ", "\\ ")

        depinfo = self.binary.with_name("sciart.d")
        depinfo.write_text(f"{esc(self.binary)}: {' '.join(esc(p) for p in paths)}\n\n")


DEFAULT = ["crates/sciart/src/main.rs", "crates/sciart-keepawake/src/lib.rs"]


class SizeCheck(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "check out"  # a space, as dep-info escapes it

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_rt_r22_a_fresh_default_build_is_judged(self) -> None:
        tree = Tree(self.root, DEFAULT)
        self.assertEqual(size_check.judge_binary(tree.binary, self.root), [])

    def test_rt_r22_a_binary_older_than_a_source_is_refused(self) -> None:
        tree = Tree(self.root, DEFAULT)
        os.utime(self.root / DEFAULT[1], (3_000, 3_000))
        problems = size_check.judge_binary(tree.binary, self.root)
        self.assertTrue(any("stale" in p and DEFAULT[1] in p for p in problems), problems)

    def test_rt_r22_a_deleted_source_makes_the_binary_stale(self) -> None:
        tree = Tree(self.root, DEFAULT)
        (self.root / DEFAULT[0]).unlink()
        problems = size_check.judge_binary(tree.binary, self.root)
        self.assertTrue(any("stale" in p for p in problems), problems)

    def test_rt_r22_record_and_mouse_builds_are_refused(self) -> None:
        for feature, extra in [
            ("record", "crates/sciart-capture/src/lib.rs"),
            ("record", "crates/sciart/src/record.rs"),
            ("mouse", "crates/sciart-keepawake/src/mouse.rs"),
        ]:
            with self.subTest(extra=extra):
                root = self.root / feature / Path(extra).stem
                tree = Tree(root, DEFAULT + [extra])
                problems = size_check.judge_binary(tree.binary, root)
                self.assertTrue(any(f"`{feature}`" in p for p in problems), problems)

    def test_rt_r22_a_binary_from_another_checkout_is_refused(self) -> None:
        tree = Tree(self.root, DEFAULT)
        other = Path(self.tmp.name) / "other" / "crates" / "sciart" / "src" / "main.rs"
        other.parent.mkdir(parents=True)
        other.write_text("// elsewhere\n")
        tree.write_depinfo([str(other)])
        problems = size_check.judge_binary(tree.binary, self.root)
        self.assertTrue(any("another checkout" in p for p in problems), problems)

    def test_rt_r22_a_binary_without_dep_info_is_refused(self) -> None:
        tree = Tree(self.root, DEFAULT)
        tree.binary.with_name("sciart.d").unlink()
        problems = size_check.judge_binary(tree.binary, self.root)
        self.assertTrue(any("dep-info" in p for p in problems), problems)

    def test_rt_r22_the_manifest_must_trim_features_and_strip(self) -> None:
        trimmed = {name: {"version": "1", "default-features": False} for name in size_check.TRIMMED}
        good = {"workspace": {"dependencies": trimmed}, "profile": {"release": {"strip": True}}}
        self.assertEqual(size_check.manifest_problems(good), [])
        unstripped = {"workspace": {"dependencies": trimmed}, "profile": {"release": {}}}
        self.assertTrue(any("strip" in p for p in size_check.manifest_problems(unstripped)))
        untrimmed = {"workspace": {"dependencies": {}}, "profile": {"release": {"strip": True}}}
        self.assertEqual(len(size_check.manifest_problems(untrimmed)), len(size_check.TRIMMED))

    def test_rt_r22_cli_r15_the_default_build_must_carry_the_recorder(self) -> None:
        tree = Tree(self.root, DEFAULT)
        problems = size_check.judge_binary(tree.binary, self.root, "default")
        self.assertTrue(any("without the `record` feature" in p for p in problems), problems)
        root = self.root / "recorder"
        tree = Tree(root, DEFAULT + ["crates/sciart/src/record.rs"])
        self.assertEqual(size_check.judge_binary(tree.binary, root, "default"), [])
        root = self.root / "mouse"
        tree = Tree(root, DEFAULT + ["crates/sciart/src/record.rs", "crates/sciart-keepawake/src/mouse.rs"])
        problems = size_check.judge_binary(tree.binary, root, "default")
        self.assertTrue(any("`mouse`" in p for p in problems), problems)

    def test_cli_r15_the_default_build_budgets_are_the_spec_values(self) -> None:
        lines = [
            next(l for l in (size_check.ROOT / "docs" / "specs" / name).read_text().splitlines() if l.startswith(head))
            for name, head in (("SPEC-003-runtime-render.md", "- **RT-R22**"), ("SPEC-005-cli-config.md", "- **CLI-R15**"))
        ]
        size = f"{size_check.DEFAULT_BINARY_BUDGET:,}".replace(",", " ")
        crates = f"at most {size_check.DEFAULT_CRATE_BUDGET} crates"
        self.assertTrue(any(size in l and crates in l for l in lines), (size, crates))

    def test_rt_r22_budgets_are_the_spec_values(self) -> None:
        spec = (size_check.ROOT / "docs" / "specs" / "SPEC-003-runtime-render.md").read_text()
        line = next(l for l in spec.splitlines() if l.startswith("- **RT-R22**"))
        self.assertIn(f"{size_check.BINARY_BUDGET:,}".replace(",", " "), line)
        self.assertIn(f"at most {size_check.CRATE_BUDGET} crates", line)


if __name__ == "__main__":
    unittest.main()
