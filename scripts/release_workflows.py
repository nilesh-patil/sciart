#!/usr/bin/env python3
"""The release and publish workflows, read as data (SPEC-008 DIST-R5, DIST-R6, DIST-R10).

`problems(texts)` takes every file under `.github/workflows/` as {name: text}, parses each
with `yaml_subset` (a file it cannot read is a problem, never skipped) and reports:

- every workflow: a job on a runner image outside `tooling_check.RUNNER_IMAGES` (TEST-R13);
  an action not pinned to a full commit SHA with its version in a comment, or from an
  owner outside the allow-list; an install-action step that names no tool; a secret at
  workflow or job level, or inside a
  `run:` script; an expression from `github.event.*`, `inputs.*` or `github.head_ref`
  inside a `run:` script; a command that creates or pushes a tag; an upload
  (`cargo publish` without `--dry-run`, `scripts/publish_crates.py` without
  `--dry-run`, `git push`) anywhere but a gated publish step;
- release.yml: triggers, permissions per job against an allow-list, a tag push in a
  private repository refused first, the chain
  verify -> build -> package -> release, the six targets on their runners, the build
  command, the tag check, the installer check, the configuration checks, the smoke test
  and the size budgets present and able to fail the job, the packing, the checksum check,
  the attestation of every asset in a public repository (a notice when skipped), a draft
  release on the pushed tag;
- publish.yml: triggers, a concurrency group, an environment gate outside the `release`
  environment that every publish job needs, exactly the crates.io and Homebrew jobs in
  the `release` environment with a tag check each, uploads only in steps gated on the
  release event, the crates published one at a time by the resumable script, a rehearsal,
  a tap commit that tolerates no change.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_assets  # noqa: E402
from tooling_check import RELEASE_ONLY_IMAGES, runner_image_problems, runner_target  # noqa: E402
from yaml_subset import YamlError, parse  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

ALLOWED_OWNERS = {"actions", "dtolnay", "Swatinem", "taiki-e", "EmbarkStudios"}
PIN = re.compile(r"^(?P<owner>[A-Za-z0-9_.-]+)/[A-Za-z0-9_.-]+(/[^@\s]*)?@[0-9a-f]{40}$")
# The runner images themselves are listed once, in tooling_check.RUNNER_IMAGES (TEST-R13).
OLDEST_UBUNTU = "ubuntu-22.04"  # glibc 2.35 (DIST-R5)
MACOS_IMAGE = "macos-15"  # both macOS targets (DIST-R5)
assert OLDEST_UBUNTU in RELEASE_ONLY_IMAGES and OLDEST_UBUNTU + "-arm" in RELEASE_ONLY_IMAGES
BUILD = 'cargo auditable build --release --locked -p sciart --bin sciart --target "$TARGET"'
GATE = "github.event_name == 'release'"
TAG_IF = "startsWith(github.ref, 'refs/tags/v')"
# DIST-R6: attestations are signed for public repositories only. `repository.private` is
# in the payload of both triggers (a pushed tag, a manual run).
PRIVATE = "github.event.repository.private"
PRIVATE_TAG_IF = f"{TAG_IF} && {PRIVATE}"
PUBLIC_IF = f"{PRIVATE} == false"
# DIST-R10: the job that reads the `release` environment before any publish job starts.
GATE_JOB = "environment-gate"
PUBLISH_JOBS = ("crates-io", "homebrew")
LEVELS = {"none": 0, "read": 1, "write": 2}
# DIST-R10: what each job may be granted (a job without `permissions` gets the workflow's,
# which must be `contents: read`).
PERMISSIONS = {
    ("release.yml", "package"): {"contents": "read", "id-token": "write", "attestations": "write"},
    ("release.yml", "release"): {"contents": "write"},
    # Reading an environment's protection rules is an `actions: read` endpoint.
    ("publish.yml", GATE_JOB): {"actions": "read"},
}
ATTESTED = ("dist/*.tar.gz", "dist/install.sh", "dist/SHA256SUMS")
# A command that cannot fail the job.
SWALLOWED = re.compile(r"\|\|\s*(true|:|exit\s+0)\b|;\s*true\s*$|\bset\s+\+e\b", re.M)
INJECTED = re.compile(r"\$\{\{[^}]*\b(github\.event\.|inputs\.|github\.head_ref)")


def load(root: Path = ROOT) -> dict[str, str]:
    return {p.name: p.read_text() for p in sorted((root / ".github/workflows").glob("*.yml"))}


def norm(expr: object) -> str:
    """An `if:` condition without `${{ }}` and with single spaces."""
    text = str(expr if expr is not None else "").strip()
    found = re.fullmatch(r"\$\{\{(.*)\}\}", text, re.S)
    return " ".join((found.group(1) if found else text).split())


def commands(run: str) -> list[str]:
    """A `run:` script split into commands (lines, `&&`, `||`, `;`), continuations joined."""
    joined = re.sub(r"\\\n\s*", " ", run)
    return [c.strip() for c in re.split(r"\n|&&|\|\||;", joined) if c.strip()]


def uploads(run: str) -> list[str]:
    out = []
    for c in commands(run):
        if re.search(r"\bcargo\s+publish\b", c) and "--dry-run" not in c:
            out.append(c)
        elif "publish_crates.py" in c and "--dry-run" not in c:
            out.append(c)
        elif re.search(r"\bgit\s+push\b", c):
            out.append(c)
    return out


def jobs_of(data: dict) -> dict[str, dict]:
    jobs = data.get("jobs")
    return jobs if isinstance(jobs, dict) else {}


def steps_of(job: dict) -> list[dict]:
    steps = job.get("steps")
    return [s for s in steps if isinstance(s, dict)] if isinstance(steps, list) else []


def needs(job: dict) -> set[str]:
    value = job.get("needs", [])
    return {value} if isinstance(value, str) else set(value or [])


def can_fail(step: dict, job: dict) -> bool:
    return not step.get("continue-on-error") and not job.get("continue-on-error") and \
        not SWALLOWED.search(str(step.get("run", "")))


def find(job: dict, needle: str) -> list[dict]:
    return [s for s in steps_of(job) if needle in str(s.get("run", ""))]


def check_every_workflow(name: str, text: str, data: dict, out: list[str]) -> None:
    lines = text.splitlines()
    if isinstance(data.get("env"), dict) and "secrets." in str(data["env"]):
        out.append(f"DIST-R10: {name} sets a secret at workflow level; set it on the one step that needs it")
    for job_name, job in jobs_of(data).items():
        if not isinstance(job, dict):
            out.append(f"DIST-R10: {name} job {job_name} is not a mapping")
            continue
        if "secrets." in str(job.get("env", "")):
            out.append(f"DIST-R10: {name} `{job_name}` sets a secret at job level; set it on the step")
        if "uses" in job:
            out.append(f"DIST-R6: {name} `{job_name}` calls a reusable workflow; inline it")
        for step in steps_of(job):
            uses = step.get("uses")
            if uses is not None:
                found = PIN.match(str(uses))
                if not found:
                    out.append(f"DIST-R6: {name} `{job_name}` uses `{uses}`, not pinned to a full commit SHA")
                elif found.group("owner") not in ALLOWED_OWNERS:
                    out.append(f"DIST-R6: {name} `{job_name}` uses `{uses}`: owner not in the allow-list")
                elif not any(re.search(rf"uses:\s*{re.escape(str(uses))}\s+#\s*\S", line) for line in lines):
                    out.append(f"DIST-R6: {name} `{job_name}` uses `{uses}` without its version in a comment")
            # DIST-R6: without `tool:` the install action installs nothing, and the step
            # that needs the tool fails later with an unrelated message.
            if str(uses or "").startswith("taiki-e/install-action@"):
                tool = (step.get("with") or {}).get("tool") if isinstance(step.get("with"), dict) else None
                if not isinstance(tool, str) or not tool.strip():
                    out.append(f"DIST-R6: {name} `{job_name}` uses taiki-e/install-action without naming "
                               "its tool (`with: tool:`)")
            if str(uses or "").startswith("actions/checkout@") and \
                    (step.get("with") or {}).get("persist-credentials") is not False:
                out.append(f"DIST-R10: {name} `{job_name}` checks out with stored credentials "
                           "(set persist-credentials: false)")
            if name in ("release.yml", "publish.yml") and \
                    str(uses or "").startswith(("Swatinem/rust-cache@", "actions/cache@")):
                out.append(f"DIST-R10: {name} `{job_name}` restores a build cache into a release")
            run = str(step.get("run", ""))
            if "secrets." in run:
                out.append(f"DIST-R10: {name} `{job_name}` puts a secret inside a run script; pass it in env")
            if INJECTED.search(run):
                out.append(f"DIST-R10: {name} `{job_name}` interpolates github.event.*/inputs.* into a "
                           "run script; pass it in env")
            for cmd in commands(run):
                if re.search(r"\bgit\s+tag\b", cmd) or re.search(r"git\s+push\b.*(--tags|refs/tags|\bv\d)", cmd):
                    out.append(f"DIST-R10: {name} `{job_name}` creates or pushes a tag: {cmd}")
                if re.search(r"gh\s+release\s+(create|edit)\b", cmd) and "--draft=false" in cmd:
                    out.append(f"DIST-R10: {name} `{job_name}` publishes a release: {cmd}")
            if "softprops/action-gh-release" in str(uses or ""):
                out.append(f"DIST-R10: {name} `{job_name}` publishes a release directly")
            if name != "publish.yml" and uploads(run):
                out.append(f"DIST-R10: {name} `{job_name}` uploads ({uploads(run)[0]}); only publish.yml does")


def check_permissions(name: str, data: dict, out: list[str]) -> None:
    if data.get("permissions") != {"contents": "read"}:
        out.append(f"DIST-R10: {name} defaults to `permissions: contents: read`")
    for job_name, job in jobs_of(data).items():
        if not isinstance(job, dict) or "permissions" not in job:
            continue
        granted = job["permissions"]
        allowed = PERMISSIONS.get((name, job_name), {})
        if not isinstance(granted, dict):
            out.append(f"DIST-R10: {name} `{job_name}` has permissions {granted!r}, beyond its allow-list")
            continue
        for scope, level in granted.items():
            if LEVELS.get(str(level), 3) > LEVELS.get(allowed.get(scope, "none"), 0):
                out.append(f"DIST-R10: {name} `{job_name}` has permissions {scope}: {level}, beyond its allow-list")


def check_release(data: dict, out: list[str]) -> None:
    name = "release.yml"
    on = data.get("on")
    if on != {"push": {"tags": ["v*"]}, "workflow_dispatch": None}:
        out.append(f"DIST-R10: release.yml runs on a pushed v* tag and by hand only: {on!r}")
    check_permissions(name, data, out)
    concurrency = data.get("concurrency")
    if not isinstance(concurrency, dict) or not concurrency.get("group") or concurrency.get("cancel-in-progress"):
        out.append("DIST-R10: release.yml has a concurrency group that does not cancel a running release")
    jobs = jobs_of(data)
    for job, want in (("build", {"verify"}), ("package", {"build"}), ("release", {"package"})):
        if job not in jobs:
            out.append(f"DIST-R10: release.yml has a `{job}` job")
        elif not want <= needs(jobs[job]):
            out.append(f"DIST-R10: release.yml `{job}` needs {sorted(want)} (build -> package -> release)")
    verify = jobs.get("verify", {})
    first = steps_of(verify)[:1]
    if not first or norm(first[0].get("if")) != PRIVATE_TAG_IF or \
            "exit 1" not in str(first[0].get("run", "")) or "::error::" not in str(first[0].get("run", "")) or \
            not can_fail(first[0], verify):
        out.append(f"DIST-R6: release.yml `verify` first fails a tag push in a private repository with an "
                   f"error (if: {PRIVATE_TAG_IF}; `::error::` and `exit 1`), before anything is built")
    for needle, what, condition in (
        ("release_check.py --tag", "tag check", TAG_IF),
        ("installer_check.py --require-shellcheck", "installer check", None),
        ("release_check.py --require-ruby", "release configuration check", None),
        ("release_check_test.py", "workflow mutation tests", None),
    ):
        hits = find(verify, needle)
        if not hits:
            out.append(f"DIST-R10: release.yml `verify` runs the {what} (`{needle}`)")
        for step in hits:
            if condition is not None and norm(step.get("if")) != condition:
                out.append(f"DIST-R10: the {what} runs on every tag build (if: {condition})")
            elif condition is None and "if" in step:
                out.append(f"DIST-R10: the {what} runs unconditionally")
            if not can_fail(step, verify):
                out.append(f"DIST-R10: the {what} cannot fail the job")
    build = jobs.get("build", {})
    matrix = build.get("strategy", {}).get("matrix", {}) if isinstance(build.get("strategy"), dict) else {}
    entries = matrix.get("include", []) if isinstance(matrix, dict) else []
    by_target = {e.get("target"): e for e in entries if isinstance(e, dict)}
    if set(by_target) != set(release_assets.TARGETS) or len(entries) != len(release_assets.TARGETS) \
            or set(matrix) != {"include"}:
        out.append(f"DIST-R5: release.yml builds exactly the six targets: {sorted(map(str, by_target))}")
    if norm(build.get("runs-on")) != "matrix.os":
        out.append("DIST-R5: the build job runs on the matrix's runner")
    for target, entry in by_target.items():
        if target is None:
            continue
        os_label = str(entry.get("os", ""))
        arch = target.split("-")[0]
        if target.endswith("-apple-darwin"):
            if os_label != MACOS_IMAGE:
                out.append(f"DIST-R5: {target} builds on {MACOS_IMAGE}, not {os_label}")
        else:
            want = OLDEST_UBUNTU + ("-arm" if arch == "aarch64" else "")
            if os_label != want:
                out.append(f"DIST-R5: {target} builds on {want} (glibc 2.35), not {os_label}")
        native = runner_target(os_label)
        runs_here = native is not None and native.split("-")[0] == arch and \
            native.endswith("darwin") == target.endswith("darwin")
        if entry.get("smoke") is not runs_here:
            out.append(f"DIST-R5: {target} is smoke-tested exactly when it runs on its runner "
                       f"({os_label}): smoke = {entry.get('smoke')!r}")
    builds = find(build, "cargo auditable build")
    if len(builds) != 1 or str(builds[0].get("run", "")).strip() != BUILD:
        out.append(f"DIST-R5 / DIST-R6: the build step runs `{BUILD}` (default features)")
    else:
        env = builds[0].get("env", {}) or {}
        if norm(env.get("TARGET")) != "matrix.target" or norm(env.get("SCIART_GIT_SHA")) != "github.sha":
            out.append("DIST-R2 / DIST-R5: the build step gets TARGET and SCIART_GIT_SHA from env")
    smoke = [s for s in steps_of(build) if norm(s.get("if")) == "matrix.smoke"]
    if len(smoke) != 1:
        out.append("DIST-R5: the build job has one smoke-test step (if: matrix.smoke)")
    else:
        run = str(smoke[0].get("run", ""))
        for probe in ('"$bin" -V', '"$bin" --version', '"$bin" list --json'):
            if probe not in run:
                out.append(f"DIST-R5: the smoke test runs `{probe}`")
        if smoke[0].get("shell") != "bash" or "set -euo pipefail" not in run:
            out.append("DIST-R5: the smoke test runs under bash with `set -euo pipefail`")
        if not can_fail(smoke[0], build):
            out.append("DIST-R5 / DIST-R10: the smoke test cannot fail the job")
    size = find(build, "size_check.py")
    if len(size) != 1 or norm(size[0].get("if")) != "matrix.target == 'aarch64-apple-darwin'" \
            or "--build lean" not in str(size[0].get("run")) or "--build default" not in str(size[0].get("run")):
        out.append("DIST-R5 / RT-R22: the aarch64-apple-darwin build checks both size budgets")
    elif not can_fail(size[0], build):
        out.append("DIST-R5 / RT-R22: the size check cannot fail the job")
    package = jobs.get("package", {})
    pack = find(package, "scripts/release_assets.py")
    if len(pack) != 1 or "--tool bins/x86_64-unknown-linux-gnu/sciart" not in str(pack[0].get("run")):
        out.append("DIST-R5: the package job packs with release_assets.py, generating the "
                   "completions and manual with the x86_64 Linux binary")
    sums = find(package, "sha256sum --check --strict SHA256SUMS")
    if len(sums) != 1 or not can_fail(sums[0], package):
        out.append("DIST-R6: the package job verifies SHA256SUMS")
    attest = [s for s in steps_of(package) if str(s.get("uses", "")).startswith("actions/attest-build-provenance@")]
    subjects = str((attest[0].get("with") or {}).get("subject-path", "")).split() if attest else []
    if len(attest) != 1 or sorted(subjects) != sorted(ATTESTED):
        out.append(f"DIST-R6: the package job attests {', '.join(ATTESTED)}: {subjects}")
    elif norm(attest[0].get("if")) != PUBLIC_IF:
        out.append(f"DIST-R6: the attestation runs only in a public repository (if: {PUBLIC_IF})")
    notice = [s for s in steps_of(package) if norm(s.get("if")) == PRIVATE]
    if len(notice) != 1 or "::notice::" not in str(notice[0].get("run", "")):
        out.append(f"DIST-R6: the package job says with a `::notice::` when it skips the attestation "
                   f"(if: {PRIVATE})")
    release = jobs.get("release", {})
    if norm(release.get("if")) != TAG_IF:
        out.append("DIST-R10: the release job runs only for a tag")
    create = [c for s in steps_of(release) for c in commands(str(s.get("run", ""))) if "gh release create" in c]
    if len(create) != 1 or "--draft" not in create[0] or "--verify-tag" not in create[0]:
        out.append("DIST-R10: the release job creates a draft (`--draft`) on the pushed tag (`--verify-tag`)")


def check_publish(data: dict, out: list[str]) -> None:
    name = "publish.yml"
    on = data.get("on")
    if not isinstance(on, dict) or set(on) != {"release", "workflow_dispatch"} or \
            on.get("release") != {"types": ["published"]}:
        out.append(f"DIST-R10: publish.yml runs on a published release or by hand only: {on!r}")
    check_permissions(name, data, out)
    concurrency = data.get("concurrency")
    if not isinstance(concurrency, dict) or not concurrency.get("group") or concurrency.get("cancel-in-progress"):
        out.append("DIST-R10: publish.yml has a concurrency group that does not cancel a running publish")
    jobs = jobs_of(data)
    gate = jobs.get(GATE_JOB)
    if not isinstance(gate, dict):
        out.append(f"DIST-R10: publish.yml has an `{GATE_JOB}` job that reads the `release` environment")
    else:
        if gate.get("environment") is not None:
            out.append(f"DIST-R10: publish.yml `{GATE_JOB}` runs outside the `release` environment "
                       "(naming it would create a missing one)")
        reads = find(gate, "environments/release")
        run = str(reads[0].get("run", "")) if len(reads) == 1 else ""
        if len(reads) != 1 or "gh api" not in run or "required_reviewers" not in run or \
                "GITHUB_EVENT_NAME" not in run or "exit 1" not in run:
            out.append(f"DIST-R10: publish.yml `{GATE_JOB}` reads the environment with `gh api` and, on the "
                       "release event (GITHUB_EVENT_NAME), fails (`exit 1`) unless it has `required_reviewers`")
        elif "if" in reads[0] or not can_fail(reads[0], gate):
            out.append(f"DIST-R10: publish.yml `{GATE_JOB}`'s environment check runs unconditionally and can "
                       "fail the job")
    for job_name in PUBLISH_JOBS:
        job = jobs.get(job_name)
        if not isinstance(job, dict):
            out.append(f"DIST-R10: publish.yml has a `{job_name}` job")
            continue
        if GATE_JOB not in needs(job):
            out.append(f"DIST-R10: publish.yml `{job_name}` needs `{GATE_JOB}`")
        env = job.get("environment")
        if (env.get("name") if isinstance(env, dict) else env) != "release":
            out.append(f"DIST-R10: publish.yml `{job_name}` runs in the `release` environment")
        tags = find(job, "release_check.py --tag")
        if not tags or any("if" in s or not can_fail(s, job) for s in tags):
            out.append(f"DIST-R10: publish.yml `{job_name}` has a tag check that can fail the job")
    for job_name, job in jobs.items():
        if not isinstance(job, dict):
            continue
        env = job.get("environment")
        gated_job = (env.get("name") if isinstance(env, dict) else env) == "release"
        for step in steps_of(job):
            found = uploads(str(step.get("run", "")))
            if found and (not gated_job or norm(step.get("if")) != GATE):
                out.append(f"DIST-R10: publish.yml `{job_name}` uploads ({found[0]}) outside the gated publish "
                           f"step (environment release, if: {GATE})")
            if found and not can_fail(step, job):
                out.append(f"DIST-R10: publish.yml `{job_name}` has an upload that cannot fail the job")
    crates = jobs.get("crates-io", {}) if isinstance(jobs.get("crates-io"), dict) else {}
    real = [c for s in steps_of(crates) for c in uploads(str(s.get("run", "")))]
    if real != ["python3 scripts/publish_crates.py"]:
        out.append(f"DIST-R4 / DIST-R10: crates.io gets one resumable publish, `python3 scripts/publish_crates.py`: {real}")
    if not find(crates, "cargo publish --workspace --dry-run --locked") or \
            not find(crates, "publish_crates.py --dry-run"):
        out.append("DIST-R4: the crates.io job rehearses (`cargo publish --workspace --dry-run`, "
                   "`publish_crates.py --dry-run`)")
    if set(jobs) != {GATE_JOB, *PUBLISH_JOBS}:
        out.append(f"DIST-R10: publish.yml has exactly the jobs {sorted({GATE_JOB, *PUBLISH_JOBS})}, "
                   f"not {sorted(jobs)}")
    brew = jobs.get("homebrew", {}) if isinstance(jobs.get("homebrew"), dict) else {}
    push = [s for s in steps_of(brew) if uploads(str(s.get("run", "")))]
    if len(push) != 1 or "git diff --cached --quiet" not in str(push[0].get("run")):
        out.append("DIST-R10: the tap commit tolerates a formula that did not change")


def problems(texts: dict[str, str]) -> list[str]:
    out: list[str] = []
    parsed: dict[str, dict] = {}
    for name, text in sorted(texts.items()):
        try:
            data = parse(text)
        except YamlError as e:
            out.append(f"DIST-R10: cannot read {name} as workflow data: {e}")
            continue
        if not isinstance(data, dict) or not isinstance(data.get("jobs"), dict):
            out.append(f"DIST-R10: {name} has no jobs mapping")
            continue
        parsed[name] = data
        check_every_workflow(name, text, data, out)
    # TEST-R13 / DIST-R5: every job names a supported runner image.
    out += runner_image_problems({n: t for n, t in texts.items() if n in parsed})
    for name, check in (("release.yml", check_release), ("publish.yml", check_publish)):
        if name not in texts:
            out.append(f"DIST-R10: .github/workflows/{name} exists")
        elif name in parsed:
            check(parsed[name], out)
    return out


if __name__ == "__main__":
    found = problems(load())
    for p in found:
        print(p, file=sys.stderr)
    raise SystemExit(1 if found else 0)
