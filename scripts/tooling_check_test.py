#!/usr/bin/env python3
"""Unit tests for scripts/tooling_check.py's web rules: the Pages workflow deploys only on a
published release or a manual run from the default branch (WEB-R8), and the CI `web` job
builds the binary first, then the module, its smoke test and the site (WEB-R3).

Usage: python3 scripts/tooling_check_test.py
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import tooling_check  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
PAGES = (ROOT / ".github" / "workflows" / "pages.yml").read_text()
CI = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
CONDITION = (
    "github.event_name == 'release' || "
    "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)"
)


class PagesWorkflow(unittest.TestCase):
    def test_web_r8_the_pages_workflow_passes(self) -> None:
        self.assertEqual(tooling_check.pages_problems(PAGES), [])

    def test_web_r8_a_push_trigger_fails(self) -> None:
        text = PAGES.replace("  workflow_dispatch:\n", "  workflow_dispatch:\n  push:\n    branches: [main]\n")
        self.assertTrue(tooling_check.pages_problems(text))

    def test_web_r8_a_release_that_is_not_published_fails(self) -> None:
        self.assertTrue(tooling_check.pages_problems(PAGES.replace("[published]", "[created]")))

    def test_web_r8_another_environment_fails(self) -> None:
        self.assertTrue(tooling_check.pages_problems(PAGES.replace("name: github-pages", "name: production")))

    def test_web_r8_pages_write_outside_the_deploy_job_fails(self) -> None:
        text = PAGES.replace("permissions:\n  contents: read\n", "permissions:\n  contents: read\n  pages: write\n", 1)
        self.assertTrue(tooling_check.pages_problems(text))

    def test_web_r8_a_deploy_without_the_default_branch_condition_fails(self) -> None:
        self.assertIn(CONDITION, PAGES)
        for changed in (
            PAGES.replace(f"    if: {CONDITION}\n", ""),
            PAGES.replace(CONDITION, "github.event_name == 'release' || github.event_name == 'workflow_dispatch'"),
            PAGES.replace(CONDITION, "github.event_name == 'release'  ||  true"),
            PAGES.replace(CONDITION, "always()"),
        ):
            with self.subTest(changed=changed[:0]):
                self.assertTrue(tooling_check.pages_problems(changed))

    def test_web_r8_the_condition_may_be_wrapped_in_an_expression(self) -> None:
        text = PAGES.replace(f"    if: {CONDITION}\n", "    if: ${{ " + CONDITION + " }}\n")
        self.assertEqual(tooling_check.pages_problems(text), [])


class CiWebJob(unittest.TestCase):
    def test_web_r3_the_ci_web_job_passes(self) -> None:
        self.assertEqual(tooling_check.ci_web_problems(CI), [])

    def test_web_r3_a_missing_smoke_test_fails(self) -> None:
        text = CI.replace("python3 scripts/web_smoke.py", "true")
        self.assertTrue(tooling_check.ci_web_problems(text))

    def test_web_r3_the_binary_must_be_built_before_the_player_tests(self) -> None:
        build = "      - name: The binary the site is built from (list, completions)\n        run: cargo build --locked --profile ci -p sciart\n"
        self.assertIn(build, CI)
        moved = CI.replace(build, "").replace("      - name: Docs site\n", build + "      - name: Docs site\n")
        self.assertTrue(tooling_check.ci_web_problems(moved))


if __name__ == "__main__":
    unittest.main()
