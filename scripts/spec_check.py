#!/usr/bin/env python3
"""Spec <-> test traceability (SPEC-006 TEST-R1, TEST-R4).

Scans `docs/specs/*.md` for defined requirement IDs (bold `**CORE-R10**`) and scene
acceptance IDs (`` `SOL-A1` `` on a `- **Acceptance**` line), then scans every `#[test]`
function under `crates/` and every checker under `scripts/` for citations. A test cites
an ID through its name, the comments/attributes directly above it, or its body (assert
messages), case-insensitively with `-`/`_` equivalence: `core_r10_...`, `[GEN-R4]`, and
chains such as `core_r9_r10_r12_...` or `cry_a1_a2_...`.

Also enforces TEST-R4: every `proptest!` block outside the pure `sciart-core` pins
`with_cases(n <= 64)`.

Successor of `crates/sciart/tests/spec_traceability.rs` (same enforcement, no Rust
parser to compile); wired into `pixi run rs-spec-check` and CI.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Requirements verified outside `cargo test` (SPEC-006 TEST-R1 "non-test verification").
# Keep this list short and honest: an entry here is a promise someone checks by hand.
MANUAL: dict[str, str] = {
    "GEN-R8": "release-mode timing budget: `pixi run rs-bench` (criterion); "
    "debug-mode tests cannot measure it reliably",
    "TEST-R2": "process: snapshot diffs are reviewed by a human (`cargo insta review`)",
    "TEST-R3": "process: `pixi run rs-ci` is the pre-commit gate (definition lives in pixi.toml)",
}

ID_RE = re.compile(r"^[A-Z]+-[RA][0-9]+$")


def is_id(s: str, kind: str) -> bool:
    return bool(ID_RE.match(s)) and s.split("-")[1][0] == kind


def spec_ids() -> dict[str, str]:
    """Defined IDs -> spec file that defines them."""
    ids: dict[str, str] = {}
    for path in sorted((ROOT / "docs" / "specs").rglob("*.md")):
        for line in path.read_text().splitlines():
            # Bold definitions: segments between `**` pairs.
            for seg in line.split("**")[1::2]:
                if is_id(seg, "R"):
                    ids.setdefault(seg, path.name)
            if line.lstrip().startswith("- **Acceptance**"):
                for seg in line.split("`")[1::2]:
                    if is_id(seg, "A"):
                        ids.setdefault(seg, path.name)
    return ids


def sources() -> list[tuple[str, str]]:
    """`(label, text)` of every #[test] fn under crates/ and every checker in scripts/."""
    out: list[tuple[str, str]] = []
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        # Judge only the parts below the repo root: a checkout that itself lives under a
        # dot-directory must not hide every source file.
        rel = path.relative_to(ROOT).parts
        if any(part == "target" or part.startswith(".") for part in rel):
            continue
        lines = path.read_text().splitlines()
        for i, line in enumerate(lines):
            if line.strip() != "#[test]":
                continue
            start = i
            while start > 0 and (
                lines[start - 1].lstrip().startswith("//")
                or lines[start - 1].lstrip().startswith("#[")
            ):
                start -= 1
            m = next(
                (re.search(r"fn (\w+)", l) for l in lines[i:] if re.search(r"fn (\w+)", l)),
                None,
            )
            if m is None:
                continue
            depth, opened, end = 0, False, i
            while end < len(lines):
                l = lines[end]
                depth += l.count("{") - l.count("}")
                opened = opened or "{" in l
                if opened and depth <= 0:
                    break
                end += 1
            rel = path.relative_to(ROOT)
            out.append((f"{rel}::{m.group(1)}", "\n".join(lines[start : min(end, len(lines) - 1) + 1])))
    for path in sorted((ROOT / "scripts").glob("*.py")):
        out.append((f"scripts/{path.name}", path.read_text()))
    return out


def cited_ids(text: str, prefixes: set[str]) -> set[str]:
    """IDs (upper-case, `-` separated) cited in `text` for the given lower-case prefixes."""
    found: set[str] = set()
    words = re.split(r"[^\w-]+", text.lower())
    for word in words:
        prefix = None
        for tok in re.split(r"[_-]", word):
            is_num = len(tok) > 1 and tok[0] in "ra" and tok[1:].isdigit()
            if prefix is not None and is_num:
                found.add(f"{prefix}-{tok}".upper())
            elif tok in prefixes:
                prefix = tok
            else:
                prefix = None
    return found


def check_traceability() -> list[str]:
    ids = spec_ids()
    problems: list[str] = []
    if "TEST-R1" not in ids or len(ids) <= 100:
        return [f"spec scan looks broken: found {len(ids)} IDs"]
    prefixes = {pid.split("-")[0].lower() for pid in ids}
    corpus = sources()
    cited: dict[str, list[str]] = {}
    # Manual entries go stale only when a *test* cites the ID again (a checker script
    # merely listing the MANUAL table must not un-manual an entry).
    cited_by_tests: set[str] = set()
    for name, text in corpus:
        found = cited_ids(text, prefixes)
        for pid in found:
            cited.setdefault(pid, []).append(name)
        if ".rs::" in name:
            cited_by_tests |= found
    uncited = [f"  {pid} ({file})" for pid, file in ids.items()
               if pid not in cited and pid not in MANUAL]
    undefined = [f"  {pid} cited by {', '.join(by)}" for pid, by in cited.items()
                 if pid not in ids]
    stale = [f"  {pid}" for pid in MANUAL if pid not in ids or pid in cited_by_tests]
    if uncited:
        problems.append(
            "uncited (add a test citing it, e.g. `core_r10_...`, or a MANUAL entry with a reason):\n"
            + "\n".join(uncited))
    if undefined:
        problems.append(
            "cited but not defined in docs/specs (typo or renumbered spec?):\n" + "\n".join(undefined))
    if stale:
        problems.append("MANUAL entries that are undefined or now cited (remove them):\n" + "\n".join(stale))
    return problems


def check_proptest_cap() -> list[str]:
    """TEST-R4: proptest blocks outside sciart-core pin `with_cases(n <= 64)`."""
    core = ROOT / "crates" / "sciart-core"
    offenders: list[str] = []
    blocks = 0
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        if core in path.parents:
            continue
        lines = path.read_text().splitlines()
        for i, line in enumerate(lines):
            if not line.lstrip().startswith("proptest!"):
                continue
            blocks += 1
            config = next((l.strip() for l in lines[i + 1:] if l.strip()), "")
            m = re.search(r"with_cases\((\d+)\)", config)
            if m is None or int(m.group(1)) > 64:
                offenders.append(f"{path.relative_to(ROOT)}:{i + 1}")
    if blocks == 0:
        return ["expected at least the generator property suite"]
    if offenders:
        return ["TEST-R4: proptest blocks must start with "
                "`#![proptest_config(ProptestConfig::with_cases(n))]`, n <= 64:\n" + "\n".join(offenders)]
    return []


def main() -> int:
    ids = spec_ids()
    problems = check_traceability() + check_proptest_cap()
    if problems:
        print(f"TEST-R1/R4 traceability ({len(ids)} IDs):", file=sys.stderr)
        for p in problems:
            print(p, file=sys.stderr)
        return 1
    print(f"spec traceability OK ({len(ids)} IDs)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
