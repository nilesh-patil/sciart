#!/usr/bin/env python3
"""Unit tests for scripts/tooling_check.py's web rules: the Pages workflow deploys only on a
published release or a manual run from the default branch (WEB-R8), and the CI `web` job
builds the binary first, then the module, its smoke test and the site (WEB-R3). Its platform
rules: every workflow job names a supported runner image, and the optimised snapshot job
runs on the reference platform only (TEST-R13).

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


WORKFLOWS = {p.name: p.read_text() for p in sorted((ROOT / ".github" / "workflows").glob("*.yml"))}


class RunnerImages(unittest.TestCase):
    """TEST-R13 / DIST-R5: every job names a supported runner image."""

    def problems(self, name: str, old: str, new: str) -> list[str]:
        self.assertIn(old, WORKFLOWS[name], "the text to mutate is gone; update the test")
        return tooling_check.runner_image_problems(dict(WORKFLOWS, **{name: WORKFLOWS[name].replace(old, new, 1)}))

    def test_test_r13_every_workflow_passes(self) -> None:
        self.assertEqual(tooling_check.runner_image_problems(WORKFLOWS), [])

    def test_test_r13_retired_and_moving_labels_fail(self) -> None:
        for name, old, new in (
            ("ci.yml", "          - os: macos-15\n", "          - os: macos-14\n"),
            ("ci.yml", "    runs-on: ubuntu-24.04\n", "    runs-on: ubuntu-latest\n"),
            ("release.yml", "            os: macos-15\n", "            os: macos-latest\n"),
            ("fuzz.yml", "    runs-on: ubuntu-24.04\n", "    runs-on: windows-2025\n"),
        ):
            with self.subTest(name=name, new=new.strip()):
                found = self.problems(name, old, new)
                self.assertTrue(any(new.split(":")[-1].strip() in p for p in found), found)

    def test_test_r13_the_oldest_images_are_for_release_builds_only(self) -> None:
        found = self.problems("pages.yml", "    runs-on: ubuntu-24.04\n", "    runs-on: ubuntu-22.04\n")
        self.assertTrue(any("release builds only" in p for p in found), found)

    def test_test_r13_a_runner_the_check_cannot_read_fails(self) -> None:
        for new in ("    runs-on: ${{ inputs.os }}\n", "    runs-on: [self-hosted, linux]\n"):
            with self.subTest(new=new.strip()):
                self.assertTrue(self.problems("ci.yml", "    runs-on: ubuntu-24.04\n", new))


class SnapshotJob(unittest.TestCase):
    """TEST-R13: the optimised snapshot job runs on the reference platform only."""

    def test_test_r13_the_snapshot_job_passes(self) -> None:
        self.assertEqual(tooling_check.snapshot_job_problems(tooling_check.workflow_jobs(CI)), [])

    def test_test_r13_a_linux_snapshot_row_fails(self) -> None:
        old = "          - os: macos-15\n            tier: 1\n    steps:"
        self.assertIn(old, CI)
        text = CI.replace(old, "          - os: macos-15\n            tier: 1\n          - os: ubuntu-24.04\n"
                               "            tier: 1\n    steps:")
        self.assertTrue(tooling_check.snapshot_job_problems(tooling_check.workflow_jobs(text)))

    def test_test_r13_a_snapshot_job_off_the_reference_platform_fails(self) -> None:
        old = "          # TEST-R13: the reference platform only; the suite skips itself elsewhere.\n          - os: macos-15\n"
        self.assertIn(old, CI)
        text = CI.replace(old, "          - os: ubuntu-24.04\n")
        self.assertTrue(tooling_check.snapshot_job_problems(tooling_check.workflow_jobs(text)))


class Toolchains(unittest.TestCase):
    """ADR-0024: a job that installs a toolchain other than stable pins RUSTUP_TOOLCHAIN."""

    def test_adr_0024_every_workflow_passes(self) -> None:
        self.assertEqual(tooling_check.toolchain_problems(WORKFLOWS), [])

    def test_adr_0024_nightly_without_rustup_toolchain_fails(self) -> None:
        old = "    env:\n      RUSTUP_TOOLCHAIN: nightly\n"
        self.assertIn(old, WORKFLOWS["fuzz.yml"])
        found = tooling_check.toolchain_problems(dict(WORKFLOWS, **{"fuzz.yml": WORKFLOWS["fuzz.yml"].replace(old, "")}))
        self.assertTrue(any("nightly" in p for p in found), found)

    def test_adr_0024_the_msrv_without_rustup_toolchain_fails(self) -> None:
        old = '  RUSTUP_TOOLCHAIN: "1.88.0"\n'
        self.assertIn(old, WORKFLOWS["ci.yml"])
        found = tooling_check.toolchain_problems(dict(WORKFLOWS, **{"ci.yml": WORKFLOWS["ci.yml"].replace(old, "")}))
        self.assertTrue(any("1.88.0" in p for p in found), found)

    def test_adr_0024_a_named_toolchain_input_counts(self) -> None:
        old = "          targets: wasm32-unknown-unknown\n"
        self.assertIn(old, WORKFLOWS["pages.yml"])
        text = WORKFLOWS["pages.yml"].replace(old, "          toolchain: beta\n" + old, 1)
        found = tooling_check.toolchain_problems(dict(WORKFLOWS, **{"pages.yml": text}))
        self.assertTrue(any("beta" in p for p in found), found)


PIXI = tooling_check.toml_file("pixi.toml")


class NoColorParity(unittest.TestCase):
    """TEST-R12: `rs-ci` runs CI's NO_COLOR pass, with the same command and environment."""

    STEP = "        run: cargo nextest run -p sciart --test pty_smoke --test encoder_fidelity\n"

    def pixi_with(self, **task: object) -> dict:
        tasks = dict(PIXI["tasks"])
        tasks[tooling_check.NOCOLOR_TASK] = dict(tasks[tooling_check.NOCOLOR_TASK], **task)
        return dict(PIXI, tasks=tasks)

    def test_test_r12_the_task_and_the_ci_step_agree(self) -> None:
        self.assertEqual(tooling_check.nocolor_problems(PIXI, CI), [])

    def test_test_r12_a_different_command_in_ci_fails(self) -> None:
        self.assertIn(self.STEP, CI)
        text = CI.replace(self.STEP, self.STEP.replace(" --test encoder_fidelity", ""))
        self.assertTrue(tooling_check.nocolor_problems(PIXI, text))

    def test_test_r12_a_different_command_in_the_task_fails(self) -> None:
        pixi = self.pixi_with(cmd="cargo nextest run -p sciart --test pty_smoke")
        self.assertTrue(tooling_check.nocolor_problems(pixi, CI))

    def test_test_r12_the_task_without_no_color_fails(self) -> None:
        self.assertTrue(tooling_check.nocolor_problems(self.pixi_with(env={}), CI))

    def test_test_r12_rs_ci_without_the_task_fails(self) -> None:
        tasks = dict(PIXI["tasks"])
        deps = [d for d in tasks["rs-ci"]["depends-on"] if d != tooling_check.NOCOLOR_TASK]
        tasks["rs-ci"] = dict(tasks["rs-ci"], **{"depends-on": deps})
        self.assertTrue(tooling_check.nocolor_problems(dict(PIXI, tasks=tasks), CI))

    def test_test_r12_ci_without_the_no_color_step_fails(self) -> None:
        old = '        env:\n          NO_COLOR: "1"\n'
        self.assertIn(old, CI)
        self.assertTrue(tooling_check.nocolor_problems(PIXI, CI.replace(old, "")))


if __name__ == "__main__":
    unittest.main()
