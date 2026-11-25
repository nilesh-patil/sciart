#!/usr/bin/env python3
"""Tooling schema checks (SPEC-006 TEST-R9, TEST-R10, TEST-R11, TEST-R13).

Asserts the shape of the build/test tooling the gate depends on: pixi tasks use the
documented profiles, the cargo `ci`/`test` profiles keep their opt-levels and checks,
nextest never filters or retries, sccache stays the shared rustc wrapper, and CI covers
the platform tiers.

Successor of the config half of `crates/sciart-generators/tests/source_rules.rs`; wired
into `pixi run rs-spec-check` and CI.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def toml_file(rel: str) -> dict:
    return tomllib.loads((ROOT / rel).read_text())


def pixi_task(pixi: dict, name: str) -> tuple[str, list[str]]:
    task = pixi.get("tasks", {}).get(name)
    if task is None:
        raise KeyError(f"pixi.toml has no `{name}` task")
    if isinstance(task, str):
        return task, []
    return task.get("cmd", ""), list(task.get("depends-on", []))


def fail(problems: list[str], msg: str) -> None:
    problems.append(msg)


def check_r9(problems: list[str]) -> None:
    pixi = toml_file("pixi.toml")
    snap, _ = pixi_task(pixi, "rs-snap-release")
    if "--profile ci" not in snap or "--test snapshots" not in snap:
        fail(problems, f"TEST-R9: rs-snap-release must run the snapshot suite in the `ci` profile: {snap}")
    if "rs-snap-release" not in pixi_task(pixi, "rs-ci")[1]:
        fail(problems, "TEST-R9: rs-ci must depend on rs-snap-release")
    for probe in ("rs-cost", "rs-worst"):
        if "--profile ci" not in pixi_task(pixi, probe)[0]:
            fail(problems, f"TEST-R9: {probe} builds with the `ci` profile")
    for install in ("rs-install", "rs-install-mouse"):
        cmd = pixi_task(pixi, install)[0]
        if "--profile" in cmd or "--debug" in cmd:
            fail(problems, f"TEST-R9: installs keep the `release` profile: {cmd}")
    cargo = toml_file("Cargo.toml")
    ci = cargo.get("profile", {}).get("ci", {})
    if ci.get("inherits") != "release":
        fail(problems, "TEST-R9: ci inherits release")
    if ci.get("opt-level") != 3:
        fail(problems, "TEST-R9: ci builds at opt-level 3")
    if ci.get("panic") != "unwind":
        fail(problems, "TEST-R9 / ADR-0006: ci unwinds")
    if ci.get("lto") not in ("off", False):
        fail(problems, f"TEST-R9: ci has no LTO: {ci.get('lto')!r}")
    if not isinstance(ci.get("codegen-units"), int) or ci["codegen-units"] < 16:
        fail(problems, "TEST-R9: ci uses >= 16 codegen units")
    for profile in ("ci", "release"):
        for krate, over in cargo.get("profile", {}).get(profile, {}).get("package", {}).items():
            if isinstance(over, dict) and isinstance(over.get("opt-level"), int) and over["opt-level"] < 3:
                fail(problems, f"TEST-R9: [profile.{profile}.package.{krate}] lowers the opt-level")


def check_r10(problems: list[str]) -> None:
    pixi = toml_file("pixi.toml")
    cmd, _ = pixi_task(pixi, "rs-test")
    if "cargo nextest run --workspace" not in cmd or "cargo test --workspace --doc" not in cmd:
        fail(problems, f"TEST-R10: rs-test runs the workspace under nextest plus the doctests: {cmd}")
    for narrowing in ("--skip", "-E ", "--filterset", "--ignored", "--run-ignored",
                      "--retries", "--exclude", "--lib", "--profile", "--cargo-profile"):
        if narrowing in cmd:
            fail(problems, f"TEST-R10: rs-test must not narrow or re-profile the run (`{narrowing}`): {cmd}")
    if "rs-test" not in pixi_task(pixi, "rs-ci")[1]:
        fail(problems, "TEST-R10: rs-ci runs rs-test")
    cargo = toml_file("Cargo.toml")
    profiles = cargo.get("profile", {})
    test = profiles.get("test", {})
    if not isinstance(test.get("opt-level"), int) or test["opt-level"] < 2:
        fail(problems, "TEST-R10: the test profile is opt-level >= 2")
    for key in ("debug-assertions", "overflow-checks"):
        if test.get(key) is not True:
            fail(problems, f"TEST-R10: the test profile states `{key} = true`")
    dev_pkgs = profiles.get("dev", {}).get("package", {})
    test_pkgs = profiles.get("test", {}).get("package", {})
    for krate, over in dev_pkgs.items():
        if isinstance(over, dict) and "opt-level" in over:
            level = test_pkgs.get(krate, {}).get("opt-level")
            if not isinstance(level, int) or level < 2:
                fail(problems, f"TEST-R10: [profile.dev.package.{krate}] sets an opt-level that the test "
                               f"profile inherits; add [profile.test.package.{krate}] opt-level >= 2")
    for krate, over in test_pkgs.items():
        if isinstance(over, dict):
            if "opt-level" in over and over["opt-level"] < 2:
                fail(problems, f"TEST-R10: [profile.test.package.{krate}] is opt-level >= 2")
            for key in ("debug-assertions", "overflow-checks"):
                if over.get(key) is False:
                    fail(problems, f"TEST-R10: [profile.test.package.{krate}] keeps `{key}`")
    nextest = toml_file(".config/nextest.toml")
    for name, profile in nextest.get("profile", {}).items():
        tables = [profile] + list(profile.get("overrides", []))
        for t in tables:
            for key in ("default-filter", "retries", "run-ignored"):
                if key in t:
                    fail(problems, f"TEST-R10: nextest profile `{name}` must not set `{key}`")


def check_r11(problems: list[str]) -> None:
    pixi = toml_file("pixi.toml")
    if "sccache" not in pixi.get("dependencies", {}):
        fail(problems, "TEST-R11: pixi.toml [dependencies] provides sccache")
    scripts = pixi.get("activation", {}).get("scripts", [])
    if not scripts:
        fail(problems, "TEST-R11: pixi.toml [activation] scripts sets the wrapper")
    text = "".join((ROOT / s).read_text() for s in scripts)
    if "export RUSTC_WRAPPER=sccache" not in text or "command -v sccache" not in text:
        fail(problems, "TEST-R11: activation exports RUSTC_WRAPPER=sccache, guarded by `command -v sccache`")
    for name in pixi.get("tasks", {}):
        if "RUSTC_WRAPPER" in pixi_task(pixi, name)[0]:
            fail(problems, f"TEST-R11: task `{name}` overrides RUSTC_WRAPPER")


# TEST-R13 / ADR-0029: platform tiers.
TIER_1 = {"aarch64-apple-darwin", "x86_64-unknown-linux-gnu"}
TIER_2 = {"aarch64-unknown-linux-gnu"}
PIXI_PLATFORMS = ("osx-arm64", "osx-64", "linux-64", "linux-aarch64")
PINNING_SUITES = (
    "crates/sciart-generators/tests/snapshots.rs",
    "crates/sciart-generators/tests/m11_fidelity.rs",
)


def runner_target(label: str) -> str | None:
    """The target a GitHub-hosted runner label builds natively."""
    if label.startswith("macos-") and not label.endswith("-intel") and label != "macos-13":
        return "aarch64-apple-darwin"
    if label.startswith("ubuntu-"):
        if label.endswith("-arm"):
            return "aarch64-unknown-linux-gnu"
        return "x86_64-unknown-linux-gnu"
    return None


def workflow_jobs(text: str) -> dict[str, str]:
    """Top-level jobs of a workflow as {name: body}: two-space-indented keys under `jobs:`."""
    parts = text.split("\njobs:\n", 1)
    jobs: dict[str, str] = {}
    if len(parts) != 2:
        return jobs
    name = None
    for line in parts[1].splitlines():
        head = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if head:
            name = head.group(1)
            jobs[name] = ""
        elif name is not None:
            jobs[name] += line + "\n"
    return jobs


def job_targets(body: str) -> set[str]:
    """Targets a job runs on: a literal `runs-on`, or every `os` of its matrix."""
    labels = re.findall(r"^\s+runs-on:\s*([A-Za-z0-9._-]+)\s*$", body, re.M)
    labels += re.findall(r"^\s+(?:-\s+)?os:\s*([A-Za-z0-9._-]+)\s*$", body, re.M)
    for listed in re.findall(r"^\s+os:\s*\[([^\]]*)\]", body, re.M):
        labels += [label.strip() for label in listed.split(",") if label.strip()]
    return {t for t in map(runner_target, labels) if t}


def forces_lf(lines: list[str], pattern: str) -> bool:
    for line in lines:
        fields = line.split()
        if fields and not fields[0].startswith("#") and fields[0].endswith(pattern) \
                and "eol=lf" in fields[1:]:
            return True
    return False


def check_r13(problems: list[str]) -> None:
    jobs = workflow_jobs((ROOT / ".github/workflows/ci.yml").read_text())
    for job, targets in {"lint": TIER_1, "test": TIER_1 | TIER_2, "snapshots": TIER_1}.items():
        if job not in jobs:
            fail(problems, f"TEST-R13: ci.yml has no `{job}` job")
            continue
        missing = targets - job_targets(jobs[job])
        if missing:
            fail(problems, f"TEST-R13: ci.yml `{job}` does not run on {sorted(missing)}")
    if "test" in jobs and not re.search(r"run: cargo nextest run --workspace\s*$", jobs["test"], re.M):
        fail(problems, "TEST-R13: ci.yml `test` runs the whole suite (`cargo nextest run --workspace`)")
    pixi = toml_file("pixi.toml")
    workspace = pixi.get("workspace", pixi.get("project", {}))
    if workspace.get("name") != "sciart":
        fail(problems, f"TEST-R13: pixi.toml is named `sciart`, not {workspace.get('name')!r}")
    platforms = set(workspace.get("platforms", []))
    for platform in PIXI_PLATFORMS:
        if platform not in platforms:
            fail(problems, f"TEST-R13: pixi.toml lists the `{platform}` platform")
    attributes = (ROOT / ".gitattributes").read_text().splitlines()
    for pattern in ("*.snap", "tests/fixtures/**"):
        if not forces_lf(attributes, pattern):
            fail(problems, f"TEST-R13: .gitattributes forces `eol=lf` for `{pattern}`")
    for suite in PINNING_SUITES:
        if "tier::skip_off_tier_1(" not in (ROOT / suite).read_text():
            fail(problems, f"TEST-R13: {suite} skips itself off tier 1 (`tier::skip_off_tier_1`)")
    if "TEST-R13" not in (ROOT / "crates/sciart-generators/tests/common/tier.rs").read_text():
        fail(problems, "TEST-R13: the tier-1 skip reason (tests/common/tier.rs) names TEST-R13")


def main() -> int:
    problems: list[str] = []
    for check in (check_r9, check_r10, check_r11, check_r13):
        check(problems)
    if problems:
        print("tooling schema:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print("tooling schema OK (pixi tasks, cargo profiles, nextest, sccache, platform tiers)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
