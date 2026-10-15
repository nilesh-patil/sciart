#!/usr/bin/env python3
"""TEST-R10 evidence: the tests `pixi run rs-test` runs == the tests `cargo test --workspace` lists.

Runs both listers in the workspace root and compares (test binary, test name) pairs:
  * libtest: `cargo test --workspace -- --list --format terse` (doctests included),
  * rs-test: `cargo nextest list --workspace` (JSON) plus `cargo test --workspace --doc -- --list`.
Binaries are matched by their file name without the `-<hash>` suffix, so both sides may build
with different profiles. Exits non-zero and prints the difference when the sets differ.

Usage (from the workspace root): pixi run python3 scripts/test_set_parity.py
"""

import json
import re
import subprocess
import sys

RUNNING = re.compile(r"^\s*(?:Running .*\((?P<bin>[^)]+)\)|Doc-tests (?P<doc>\S+))\s*$")


def libtest_list(extra):
    """(binary, name) pairs from libtest's terse `--list` output (stderr carries the headers)."""
    out = subprocess.run(
        ["cargo", "test", "--workspace", *extra, "--", "--list", "--format", "terse"],
        capture_output=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=True,
    ).stdout
    pairs, binary = set(), None
    for line in out.splitlines():
        m = RUNNING.match(line)
        if m:
            binary = norm(m["bin"]) if m["bin"] else "doc:" + m["doc"]
        elif line.endswith(": test") or line.endswith(": bench"):
            assert binary, f"test before any binary header: {line}"
            pairs.add((binary, line.rsplit(": ", 1)[0]))
    return pairs


def norm(path):
    name = path.rsplit("/", 1)[-1]
    return re.sub(r"-[0-9a-f]{16}$", "", name)


def nextest_list():
    out = subprocess.run(
        ["cargo", "nextest", "list", "--workspace", "--message-format", "json"],
        stdout=subprocess.PIPE,
        text=True,
        check=True,
    ).stdout
    suites = json.loads(out)["rust-suites"]
    return {
        (norm(s["binary-path"]), name)
        for s in suites.values()
        for name in s["testcases"]
    }


def main():
    want = libtest_list([])
    got = nextest_list() | libtest_list(["--doc"])
    missing, extra = sorted(want - got), sorted(got - want)
    print(f"cargo test --workspace lists {len(want)} tests; rs-test runs {len(got)}")
    for b, n in missing:
        print(f"  missing from rs-test: {b} :: {n}")
    for b, n in extra:
        print(f"  only in rs-test:      {b} :: {n}")
    return 1 if missing or extra else 0


if __name__ == "__main__":
    sys.exit(main())
