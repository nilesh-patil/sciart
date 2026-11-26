#!/usr/bin/env python3
"""Tooling schema checks (SPEC-006 TEST-R9, TEST-R10, TEST-R11, TEST-R12, TEST-R13;
SPEC-007 LIB-R4, LIB-R7, LIB-R8; SPEC-005 CLI-R15; SPEC-008 DIST-R5; SPEC-009 WEB-R3..R8).

Asserts the shape of the build/test tooling the gate depends on: pixi tasks use the
documented profiles, the cargo `ci`/`test` profiles keep their opt-levels and checks,
nextest never filters or retries, sccache stays the shared rustc wrapper, the local gate
runs CI's NO_COLOR pass, CI covers the platform tiers on supported runner images, a
workflow that installs a toolchain other than stable pins it with RUSTUP_TOOLCHAIN, the
fuzz run names its `--target`, the browser player and the docs site build in the gate and
in CI, and GitHub Pages is deployed only by hand or on a published release.

Successor of the config half of `crates/sciart-generators/tests/source_rules.rs`; wired
into `pixi run rs-spec-check` and CI.
"""

from __future__ import annotations

import re
import subprocess
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from yaml_subset import YamlError, parse  # noqa: E402

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


NOCOLOR_TASK = "rs-test-nocolor"


def nocolor_problems(pixi: dict, ci: str) -> list[str]:
    """TEST-R12: CI's `test` job re-runs the colour-sensitive suites with NO_COLOR=1, and the
    local gate runs the same command with the same environment (`rs-test-nocolor`, a
    dependency of `rs-ci`), so a test that only fails under NO_COLOR fails locally too."""
    problems = []
    task = pixi.get("tasks", {}).get(NOCOLOR_TASK)
    if not isinstance(task, dict):
        return [f"TEST-R12: pixi.toml has a `{NOCOLOR_TASK}` task with `cmd` and `env`"]
    if (task.get("env") or {}).get("NO_COLOR") != "1":
        problems.append(f"TEST-R12: `{NOCOLOR_TASK}` sets `env = {{ NO_COLOR = \"1\" }}`")
    if NOCOLOR_TASK not in pixi_task(pixi, "rs-ci")[1]:
        problems.append(f"TEST-R12: rs-ci depends on `{NOCOLOR_TASK}`")
    try:
        data = parse(ci)
    except YamlError as e:
        return problems + [f"TEST-R12: cannot read ci.yml: {e}"]
    test = (data.get("jobs") or {}).get("test") if isinstance(data, dict) else None
    steps = test.get("steps") if isinstance(test, dict) else None
    runs = [s for s in steps or [] if isinstance(s, dict) and "NO_COLOR" in (s.get("env") or {})]
    if len(runs) != 1:
        return problems + ["TEST-R12: ci.yml `test` has one step with NO_COLOR in its env"]
    step = runs[0]
    if str(step["env"]["NO_COLOR"]) != "1":
        problems.append("TEST-R12: the ci.yml NO_COLOR step sets NO_COLOR to \"1\"")
    if " ".join(str(step.get("run", "")).split()) != " ".join(str(task.get("cmd", "")).split()):
        problems.append(f"TEST-R12: the ci.yml NO_COLOR step and `{NOCOLOR_TASK}` run the same command: "
                        f"{step.get('run')!r} vs {task.get('cmd')!r}")
    return problems


def check_r12(problems: list[str]) -> None:
    for problem in nocolor_problems(toml_file("pixi.toml"), (ROOT / ".github/workflows/ci.yml").read_text()):
        fail(problems, problem)


# TEST-R13 / ADR-0029: platform tiers.
TIER_1 = {"aarch64-apple-darwin", "x86_64-unknown-linux-gnu"}
TIER_2 = {"aarch64-unknown-linux-gnu"}
# Frames are pinned here only: the snapshots and the M11 fixtures were reviewed on it.
REFERENCE_PLATFORM = "aarch64-apple-darwin"
PIXI_PLATFORMS = ("osx-arm64", "osx-64", "linux-64", "linux-aarch64")
PINNING_SUITES = (
    "crates/sciart-generators/tests/snapshots.rs",
    "crates/sciart-generators/tests/m11_fidelity.rs",
)
PINNING_SKIP = "tier::skip_off_reference_platform("

# TEST-R13 / DIST-R5: the GitHub-hosted runner images a workflow job may name. A `-latest`
# label changes meaning when the hosting service moves it, and a retired image rejects the
# job, so every job names one of these. When an image is retired, change it here and in the
# workflows together (docs/RELEASING.md).
RUNNER_IMAGES = ("macos-15", "ubuntu-24.04", "ubuntu-24.04-arm", "ubuntu-22.04", "ubuntu-22.04-arm")
# The oldest images, for the release builds' glibc baseline only (DIST-R5).
RELEASE_ONLY_IMAGES = ("ubuntu-22.04", "ubuntu-22.04-arm")
RELEASE_WORKFLOWS = ("release.yml", "publish.yml")


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


def matrix_values(body: str, key: str) -> list[str] | None:
    """Every value `key` takes in the job's `strategy.matrix` (plain lists and `include`
    entries), or None when the matrix cannot be read that simply (an `exclude`, or no
    matrix at all), so the caller fails closed."""
    block = re.search(r"^      matrix:\n((?:(?:        .*)?\n)*)", body, re.M)
    if block is None or "exclude:" in block.group(1):
        return None
    text = block.group(1)
    values = re.findall(rf"^\s+(?:-\s+)?{re.escape(key)}:\s*([A-Za-z0-9._-]+)\s*$", text, re.M)
    for listed in re.findall(rf"^\s+{re.escape(key)}:\s*\[([^\]]*)\]", text, re.M):
        values += [v.strip() for v in listed.split(",") if v.strip()]
    return values


def job_targets(body: str) -> set[str]:
    """Targets a job actually runs on, read from its `runs-on`: a literal label, or every
    value of the matrix key it names (`${{ matrix.os }}`). Matrix values that `runs-on`
    does not use count for nothing."""
    runs_on = re.search(r"^    runs-on:\s*(.+?)\s*$", body, re.M)
    if runs_on is None:
        return set()
    value = runs_on.group(1).strip("'\"")
    from_matrix = re.fullmatch(r"\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}", value)
    if from_matrix:
        labels = matrix_values(body, from_matrix.group(1)) or []
    elif re.fullmatch(r"[A-Za-z0-9._-]+", value):
        labels = [value]
    else:
        labels = []  # any other expression: not checkable, so it proves nothing
    return {t for t in map(runner_target, labels) if t}


def runner_labels(job: dict) -> list[str] | None:
    """Every runner label a parsed job can run on: a literal `runs-on`, or every value of
    the matrix key it names (plain lists and `include` entries). None when that cannot be
    read so simply (another expression, a label list, an `exclude`, no value at all), so
    the caller fails closed."""
    runs_on = job.get("runs-on")
    if not isinstance(runs_on, str):
        return None
    value = runs_on.strip()
    from_matrix = re.fullmatch(r"\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}", value)
    if from_matrix is None:
        return [value] if re.fullmatch(r"[A-Za-z0-9._-]+", value) else None
    key = from_matrix.group(1)
    strategy = job.get("strategy")
    matrix = strategy.get("matrix") if isinstance(strategy, dict) else None
    if not isinstance(matrix, dict) or "exclude" in matrix:
        return None
    listed = matrix.get(key, [])
    labels = list(listed) if isinstance(listed, list) else [listed]
    include = matrix.get("include", [])
    if not isinstance(include, list):
        return None
    labels += [entry[key] for entry in include if isinstance(entry, dict) and key in entry]
    if not labels or not all(isinstance(label, str) for label in labels):
        return None
    return labels


def runner_image_problems(texts: dict[str, str]) -> list[str]:
    """TEST-R13 / DIST-R5 over every workflow ({file name: text}): each job runs on an
    image from RUNNER_IMAGES, the oldest ones only in the release workflows."""
    problems = []
    for name, text in sorted(texts.items()):
        try:
            data = parse(text)
        except YamlError as e:
            problems.append(f"TEST-R13: cannot read {name} to check its runner images: {e}")
            continue
        jobs = data.get("jobs") if isinstance(data, dict) else None
        for job_name, job in (jobs.items() if isinstance(jobs, dict) else []):
            labels = runner_labels(job) if isinstance(job, dict) else None
            if labels is None:
                problems.append(f"TEST-R13: {name} `{job_name}` names no runner image the check can read "
                                f"(runs-on: {job.get('runs-on') if isinstance(job, dict) else job!r})")
                continue
            for label in dict.fromkeys(labels):
                if label not in RUNNER_IMAGES:
                    problems.append(f"TEST-R13: {name} `{job_name}` runs on `{label}`, not a supported "
                                    f"runner image ({', '.join(RUNNER_IMAGES)})")
                elif label in RELEASE_ONLY_IMAGES and name not in RELEASE_WORKFLOWS:
                    problems.append(f"TEST-R13: {name} `{job_name}` runs on `{label}`, an image kept for "
                                    "the release builds only")
    return problems


def workflow_texts() -> dict[str, str]:
    return {p.name: p.read_text() for p in sorted((ROOT / ".github/workflows").glob("*.yml"))}


def fuzz_target_problems(texts: dict[str, str]) -> list[str]:
    """IMP-14 over every workflow: each `cargo fuzz run` names its `--target`. The install
    action ships cargo-fuzz as a musl binary, which otherwise builds for musl, where the
    sanitizer cannot link (a statically linked libc)."""
    problems = []
    for name, text in sorted(texts.items()):
        joined = re.sub(r"\\\n\s*", " ", text)
        for command in re.findall(r"cargo\s+fuzz\s+run\b[^\n]*", joined):
            if not re.search(r"\s--target[\s=]", command.split(" -- ")[0] + " "):
                problems.append(f"IMP-14: {name} runs `{command.strip()}` without `--target` (the host triple)")
    return problems


TOOLCHAIN_ACTION = "dtolnay/rust-toolchain@"


def toolchain_problems(texts: dict[str, str]) -> list[str]:
    """ADR-0024 over every workflow ({file name: text}): `rust-toolchain.toml` floats on
    stable and wins over the toolchain an action installs, so a job that installs any other
    toolchain (`with: toolchain:`, or the version in the pinned action's comment) sets
    `RUSTUP_TOOLCHAIN` to it at job or workflow level; otherwise its cargo commands run on
    stable (and `cargo fuzz` refuses `-Z`)."""
    problems = []
    for name, text in sorted(texts.items()):
        try:
            data = parse(text)
        except YamlError as e:
            problems.append(f"ADR-0024: cannot read {name} to check its toolchains: {e}")
            continue
        if not isinstance(data, dict) or not isinstance(data.get("jobs"), dict):
            continue
        top = data.get("env") if isinstance(data.get("env"), dict) else {}
        for job_name, job in data["jobs"].items():
            if not isinstance(job, dict):
                continue
            env = dict(top, **(job.get("env") if isinstance(job.get("env"), dict) else {}))
            for step in job.get("steps") or []:
                uses = str(step.get("uses", "")) if isinstance(step, dict) else ""
                if not uses.startswith(TOOLCHAIN_ACTION):
                    continue
                chosen = (step.get("with") or {}).get("toolchain") if isinstance(step.get("with"), dict) else None
                if chosen is None:
                    comment = re.search(rf"uses:\s*{re.escape(uses)}\s+#\s*(\S+)", text)
                    chosen = comment.group(1) if comment else None
                if chosen is None:
                    problems.append(f"ADR-0024: {name} `{job_name}` installs a toolchain it does not name")
                elif str(chosen) != "stable" and str(env.get("RUSTUP_TOOLCHAIN")) != str(chosen):
                    problems.append(f"ADR-0024: {name} `{job_name}` installs the {chosen} toolchain but does not set "
                                    f"`RUSTUP_TOOLCHAIN: {chosen}` at job or workflow level, so "
                                    "rust-toolchain.toml (stable) wins")
    return problems


def forces_lf(lines: list[str], pattern: str) -> bool:
    for line in lines:
        fields = line.split()
        if fields and not fields[0].startswith("#") and fields[0].endswith(pattern) \
                and "eol=lf" in fields[1:]:
            return True
    return False


def check_r13(problems: list[str]) -> None:
    jobs = workflow_jobs((ROOT / ".github/workflows/ci.yml").read_text())
    for job, targets in {"lint": TIER_1, "test": TIER_1 | TIER_2}.items():
        if job not in jobs:
            fail(problems, f"TEST-R13: ci.yml has no `{job}` job")
            continue
        missing = targets - job_targets(jobs[job])
        if missing:
            fail(problems, f"TEST-R13: ci.yml `{job}` does not run on {sorted(missing)}")
    for problem in snapshot_job_problems(jobs):
        fail(problems, problem)
    for problem in runner_image_problems(workflow_texts()):
        fail(problems, problem)
    for problem in toolchain_problems(workflow_texts()):
        fail(problems, problem)
    for problem in fuzz_target_problems(workflow_texts()):
        fail(problems, problem)
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
        if PINNING_SKIP not in (ROOT / suite).read_text():
            fail(problems, f"TEST-R13: {suite} skips itself off the reference platform (`{PINNING_SKIP}`)")
    tier = (ROOT / "crates/sciart-generators/tests/common/tier.rs").read_text()
    if "TEST-R13" not in tier:
        fail(problems, "TEST-R13: the reference-platform skip reason (tests/common/tier.rs) names TEST-R13")
    pinned = re.search(r"pub const FRAMES_PINNED: bool = cfg!\((.*?)\);", tier, re.S)
    if pinned is None or " ".join(pinned.group(1).split()) != 'all(target_os = "macos", target_arch = "aarch64")':
        fail(problems, f"TEST-R13: tests/common/tier.rs pins frames on {REFERENCE_PLATFORM} only "
                       "(`FRAMES_PINNED = cfg!(all(target_os = \"macos\", target_arch = \"aarch64\"))`)")


def snapshot_job_problems(jobs: dict[str, str]) -> list[str]:
    """TEST-R13 / TEST-R9: the optimised snapshot job runs on the reference platform, and
    only there (the suite skips itself elsewhere, so another runner would prove nothing)."""
    if "snapshots" not in jobs:
        return ["TEST-R13: ci.yml has no `snapshots` job"]
    body = jobs["snapshots"]
    if "cargo test --profile ci -p sciart-generators --test snapshots" not in body:
        return ["TEST-R9: ci.yml `snapshots` runs the snapshot suite in the `ci` profile"]
    targets = job_targets(body)
    if targets != {REFERENCE_PLATFORM}:
        return [f"TEST-R13: ci.yml `snapshots` runs on the reference platform ({REFERENCE_PLATFORM}) "
                f"only, not {sorted(targets)}"]
    return []


# LIB-R4 / CLI-R15: what the library pulls in without the binary's defaults.
CLI_ONLY_CRATES = ("clap", "anyhow", "toml", "serde", "gif", "sciart-keepawake")
LIB_DOC_TESTS = "cargo test -p sciart --doc --features png,widget,unstable-generator"
LIB_LINTS = (
    "cargo clippy -p sciart --all-targets --no-default-features -- -D warnings",
    "cargo clippy -p sciart --all-targets --no-default-features --features png,widget -- -D warnings",
)
LIB_TESTS = "cargo nextest run -p sciart --no-default-features --features png,widget,unstable-generator"
NO_RECORDER_TESTS = "cargo nextest run -p sciart --no-default-features --features cli"


def lean_tree(features: str = "") -> list[str] | None:
    """Crate names in `sciart`'s normal dependency tree with no default features (plus
    `features`), or None when cargo cannot resolve it."""
    extra = ["--features", features] if features else []
    try:
        out = subprocess.run(
            ["cargo", "tree", "-p", "sciart", "--no-default-features", *extra, "-e", "normal",
             "--prefix", "none", "--locked"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return sorted({line.split()[0] for line in out.splitlines() if line.strip()})


def check_lib(problems: list[str]) -> None:
    manifest = toml_file("crates/sciart/Cargo.toml")
    features = manifest.get("features", {})
    if features.get("default") != ["cli", "record"]:
        fail(problems, f"LIB-R4 / CLI-R15: sciart's default features are cli and record: {features.get('default')}")
    for name in ("cli", "record", "png", "widget", "unstable-generator"):
        if name not in features:
            fail(problems, f"LIB-R4: sciart has a `{name}` feature")
    if "png" not in features.get("record", []):
        fail(problems, "CLI-R15: `record` enables `png` (the capture pipeline)")
    bins = manifest.get("bin", [])
    if not any(b.get("name") == "sciart" and b.get("required-features") == ["cli"] for b in bins):
        fail(problems, "LIB-R4: the `sciart` binary has required-features = [\"cli\"]")
    docs = manifest.get("package", {}).get("metadata", {}).get("docs", {}).get("rs", {})
    if sorted(docs.get("features", [])) != ["png", "unstable-generator", "widget"] or "mouse" in str(docs):
        fail(problems, f"LIB-R4: docs.rs builds png, widget and unstable-generator, never mouse: {docs}")
    tree = lean_tree()
    if tree is None:
        fail(problems, "LIB-R4: `cargo tree -p sciart --no-default-features` did not resolve")
    else:
        for crate in CLI_ONLY_CRATES:
            if crate in tree:
                fail(problems, f"LIB-R4: `{crate}` is in the library's dependency tree without default features")
    png_tree = lean_tree("png")
    if png_tree is None:
        fail(problems, "LIB-R4: `cargo tree -p sciart --no-default-features --features png` did not resolve")
    else:
        if "gif" in png_tree:
            fail(problems, "LIB-R4: `png` must not bring the GIF encoder (only `record` enables it)")
        for crate in ("png", "font8x8"):
            if crate not in png_tree:
                fail(problems, f"LIB-R4: `png` brings `{crate}`")
    if "sciart-capture/gif" not in features.get("record", []):
        fail(problems, "LIB-R4 / CLI-R15: `record` turns on the capture crate's `gif` feature")
    pixi = toml_file("pixi.toml")
    lint, _ = pixi_task(pixi, "rs-lint")
    for command in LIB_LINTS:
        if command not in lint:
            fail(problems, f"LIB-R4: rs-lint runs `{command}`")
    tests, _ = pixi_task(pixi, "rs-test-features")
    for command, req in ((LIB_DOC_TESTS, "LIB-R7"), (LIB_TESTS, "LIB-R7"), (NO_RECORDER_TESTS, "CLI-R15")):
        if command not in tests:
            fail(problems, f"{req}: rs-test-features runs `{command}`")
    if "rs-test-features" not in pixi_task(pixi, "rs-ci")[1]:
        fail(problems, "LIB-R7: rs-ci runs rs-test-features")
    workflow = (ROOT / ".github/workflows/ci.yml").read_text()
    jobs = workflow_jobs(workflow)
    for command in LIB_LINTS:
        if command not in jobs.get("lint", ""):
            fail(problems, f"LIB-R4: ci.yml `lint` runs `{command}`")
    for command in (LIB_DOC_TESTS, LIB_TESTS, NO_RECORDER_TESTS):
        if command not in jobs.get("test", ""):
            fail(problems, f"LIB-R7 / CLI-R15: ci.yml `test` runs `{command}`")
    for problem in semver_problems(workflow):
        fail(problems, problem)
    if any("semver-checks" in pixi_task(pixi, name)[0] for name in pixi.get("tasks", {})):
        fail(problems, "LIB-R8: cargo-semver-checks runs in hosted CI only, not in a pixi task")


SEMVER_BASELINE = "--baseline-rev v1.0.0"


def semver_problems(ci: str) -> list[str]:
    """LIB-R8 in ci.yml: a job runs `cargo semver-checks check-release` against the `v1.0.0`
    tag (`--baseline-rev`, built from this repository; never a registry version, since 1.0.0
    is not on crates.io), and its checkout has the full history and the tags
    (`fetch-depth: 0`) without stored credentials."""
    try:
        data = parse(ci)
    except YamlError as e:
        return [f"LIB-R8: cannot read ci.yml: {e}"]
    jobs = data.get("jobs") if isinstance(data, dict) else None
    found = []
    for name, job in (jobs.items() if isinstance(jobs, dict) else []):
        steps = [s for s in (job.get("steps") or []) if isinstance(s, dict)] if isinstance(job, dict) else []
        runs = [str(s.get("run", "")) for s in steps if "cargo semver-checks check-release" in str(s.get("run", ""))]
        if runs:
            found.append((name, steps, runs))
    if len(found) != 1:
        return ["LIB-R8: exactly one ci.yml job runs `cargo semver-checks check-release`"]
    name, steps, runs = found[0]
    problems = []
    command = " ".join(" ".join(runs).replace("\\\n", " ").split())
    if SEMVER_BASELINE not in command or "--baseline-version" in command:
        problems.append(f"LIB-R8: ci.yml `{name}` compares with the v1.0.0 tag (`{SEMVER_BASELINE}`), "
                        "not a registry version")
    checkouts = [s for s in steps if str(s.get("uses", "")).startswith("actions/checkout@")]
    options = (checkouts[0].get("with") or {}) if len(checkouts) == 1 else {}
    if len(checkouts) != 1 or options.get("fetch-depth") != 0 or options.get("persist-credentials") is not False:
        problems.append(f"LIB-R8: ci.yml `{name}` checks out the full history and tags (`fetch-depth: 0`, "
                        "`persist-credentials: false`), or the v1.0.0 tag is missing")
    return problems


WEB_BUILD = "cargo build --release --locked --manifest-path web/Cargo.toml --target wasm32-unknown-unknown"
WEB_TEST = "cargo test --locked --manifest-path web/Cargo.toml"


def check_web(problems: list[str]) -> None:
    """WEB-R3..R7: the player builds for the browser through pixi and in CI on every push,
    and the site is built and checked in the gate."""
    pixi = toml_file("pixi.toml")
    for task, command in (("web-build", WEB_BUILD), ("web-test", WEB_TEST)):
        if pixi_task(pixi, task)[0] != command:
            fail(problems, f"WEB-R3: the `{task}` task runs `{command}`")
    cmd, deps = pixi_task(pixi, "site-build")
    if "scripts/build_site.py" not in cmd or "web-build" not in deps:
        fail(problems, "WEB-R5: `site-build` runs scripts/build_site.py after web-build")
    cmd, deps = pixi_task(pixi, "site-check")
    if "scripts/site_check.py" not in cmd or not {"site-build", "web-test"} <= set(deps):
        fail(problems, "WEB-R4/R7: `site-check` runs scripts/site_check.py after site-build and web-test")
    if "site-check" not in pixi_task(pixi, "rs-ci")[1]:
        fail(problems, "WEB-R4/R7: rs-ci runs site-check")
    web = toml_file("web/Cargo.toml")
    if "workspace" not in web:
        fail(problems, "WEB-R3: web/Cargo.toml is its own workspace, outside the product's")
    members = toml_file("Cargo.toml").get("workspace", {}).get("members", [])
    if any(m.startswith("web") for m in members):
        fail(problems, "WEB-R3: web/ is not a member of the product workspace")
    toolchain = toml_file("rust-toolchain.toml").get("toolchain", {})
    if "wasm32-unknown-unknown" not in toolchain.get("targets", []):
        fail(problems, "WEB-R3: rust-toolchain.toml installs the wasm32-unknown-unknown target")
    cmd, deps = pixi_task(pixi, "web-smoke")
    if "scripts/web_smoke.py" not in cmd or "web-build" not in deps:
        fail(problems, "WEB-R3: `web-smoke` runs scripts/web_smoke.py after web-build")
    if "web-smoke" not in pixi_task(pixi, "site-check")[1]:
        fail(problems, "WEB-R3: `site-check` (and so rs-ci) runs web-smoke")
    if "wasmtime-py" not in pixi.get("dependencies", {}):
        fail(problems, "WEB-R3: the pixi environment provides wasmtime-py for web-smoke")
    for problem in ci_web_problems((ROOT / ".github/workflows/ci.yml").read_text()):
        fail(problems, problem)


WEB_SMOKE = "python3 scripts/web_smoke.py"
SCIART_CI_BUILD = "cargo build --locked --profile ci -p sciart"


def ci_web_problems(ci: str) -> list[str]:
    """WEB-R3..R7 in ci.yml: on every push a job builds the `sciart` binary first (the
    player tests, the smoke test and the site compare against it), then the module, runs
    the module's smoke test and the player tests, and builds and checks the site."""
    problems = []
    if not re.search(r"^on:\n  push:", ci, re.M):
        problems.append("WEB-R3: ci.yml runs on push")
    web_jobs = [body for body in workflow_jobs(ci).values() if WEB_BUILD in body]
    if not web_jobs:
        return problems + [f"WEB-R3: a ci.yml job runs `{WEB_BUILD}`"]
    body = web_jobs[0]
    if "wasm32-unknown-unknown" not in body.split(WEB_BUILD)[0]:
        problems.append("WEB-R3: the ci.yml web job installs the wasm32-unknown-unknown target")
    order = [SCIART_CI_BUILD, WEB_BUILD, WEB_SMOKE, WEB_TEST, "python3 scripts/build_site.py",
             "python3 scripts/site_check.py"]
    positions = [body.find(step) for step in order]
    for step, at in zip(order, positions):
        if at < 0:
            problems.append(f"WEB-R3..R7: the ci.yml web job runs `{step}`")
    found = [at for at in positions if at >= 0]
    if found != sorted(found):
        problems.append(
            "WEB-R3: the ci.yml web job builds the binary, then the module, then runs the "
            "smoke test, the player tests and the site, in that order"
        )
    return problems


def workflow_on(text: str) -> dict[str, str]:
    """The workflow's triggers as {event: body}: two-space-indented keys under `on:`."""
    block = re.search(r"^on:\s*\n((?:(?:  .*)?\n)*)", text, re.M)
    events: dict[str, str] = {}
    if block is None:
        return events
    name = None
    for line in block.group(1).splitlines():
        head = re.match(r"^  ([A-Za-z_]+):\s*(.*)$", line)
        if head:
            name = head.group(1)
            events[name] = head.group(2) + "\n"
        elif name is not None:
            events[name] += line + "\n"
    return events


# WEB-R8: the deploy job's guard. A manual run from any other branch builds and stops.
DEPLOY_CONDITION = (
    "github.event_name == 'release' || "
    "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)"
)


def pages_problems(text: str) -> list[str]:
    """WEB-R8 over pages.yml: deploys only on a published release or a manual run from the
    default branch, through the `github-pages` environment; a push never deploys."""
    problems = []
    events = workflow_on(text)
    if set(events) != {"workflow_dispatch", "release"}:
        problems.append(f"WEB-R8: pages.yml runs on workflow_dispatch and release only, not {sorted(events)}")
    release = events.get("release", "")
    types = re.search(r"types:\s*\[([^\]]*)\]", release)
    if types is None or [t.strip() for t in types.group(1).split(",")] != ["published"]:
        problems.append("WEB-R8: pages.yml deploys on a release only when it is published")
    top = re.search(r"^permissions:\s*\n((?:  .*\n)*)", text, re.M)
    if top is None or "pages:" in top.group(1) or "id-token:" in top.group(1):
        problems.append("WEB-R8: pages.yml grants pages and id-token only to the deploy job")
    deploys = [body for body in workflow_jobs(text).values() if "actions/deploy-pages@" in body]
    if len(deploys) != 1:
        return problems + ["WEB-R8: exactly one pages.yml job deploys (actions/deploy-pages)"]
    body = deploys[0]
    if not re.search(r"^    environment:\s*\n      name:\s*github-pages\s*$", body, re.M):
        problems.append("WEB-R8: the deploy job runs in the `github-pages` environment")
    for grant in ("pages: write", "id-token: write"):
        if grant not in body:
            problems.append(f"WEB-R8: the deploy job has `{grant}`")
    guard = re.search(r"^    if:\s*(.+?)\s*$", body, re.M)
    condition = guard.group(1) if guard else ""
    wrapped = re.fullmatch(r"\$\{\{\s*(.*?)\s*\}\}", condition)
    if wrapped:
        condition = wrapped.group(1)
    if re.sub(r"\s+", " ", condition) != DEPLOY_CONDITION:
        problems.append(f"WEB-R8: the deploy job runs only `if: {DEPLOY_CONDITION}`, not {condition!r}")
    if "path: site/dist" not in text:
        problems.append("WEB-R8: pages.yml publishes site/dist (the output of scripts/build_site.py)")
    return problems


def check_web_r8(problems: list[str]) -> None:
    path = ROOT / ".github/workflows/pages.yml"
    if not path.is_file():
        fail(problems, "WEB-R8: .github/workflows/pages.yml is missing")
        return
    for problem in pages_problems(path.read_text()):
        fail(problems, problem)
    for workflow in (ROOT / ".github/workflows").glob("*.yml"):
        if workflow.name != "pages.yml" and "deploy-pages" in workflow.read_text():
            fail(problems, f"WEB-R8: only pages.yml deploys to Pages, not {workflow.name}")


def main() -> int:
    problems: list[str] = []
    for check in (check_r9, check_r10, check_r11, check_r12, check_r13, check_lib, check_web, check_web_r8):
        check(problems)
    if problems:
        print("tooling schema:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print("tooling schema OK (pixi tasks, cargo profiles, nextest, sccache, platform tiers, "
          "library features, web player, Pages)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
