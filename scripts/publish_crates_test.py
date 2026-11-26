#!/usr/bin/env python3
"""Tests of the resumable crates.io publish (SPEC-008 DIST-R4, DIST-R10), against a fake
sparse index on disk and a fake `cargo` that records what it is asked to publish.

    python3 scripts/publish_crates_test.py
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import publish_crates  # noqa: E402
from release_check import FACADE, PUBLISH_ORDER, internal_version, members, sciart_version  # noqa: E402

SCRIPT = Path(__file__).resolve().parent / "publish_crates.py"
FAKE_CARGO = """#!/bin/sh
# Records `cargo publish -p <crate> --locked`, then adds the crate to the fake index,
# unless it is the one FAIL_ON names.
[ "$1" = publish ] && [ "$2" = -p ] && [ "$4" = --locked ] || { echo "unexpected: $*" >&2; exit 3; }
echo "$3" >> "$CARGO_LOG"
[ "$3" = "${FAIL_ON:-}" ] && exit 101
python3 - "$3" <<'EOF'
import json, os, sys
sys.path.insert(0, os.environ["SCRIPTS"])
import publish_crates
name = sys.argv[1]
path = os.path.join(os.environ["INDEX_DIR"], publish_crates.index_path(name))
os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, "a") as f:
    f.write(json.dumps({"name": name, "vers": os.environ["VERS_" + name.replace("-", "_")]}) + "\\n")
EOF
"""


def versions() -> dict[str, str]:
    internal = str(internal_version(members()))
    return {n: (sciart_version() if n == FACADE else internal) for n in PUBLISH_ORDER}


def put(index: Path, name: str, *vers: str) -> None:
    path = index / publish_crates.index_path(name)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps({"name": name, "vers": v}) + "\n" for v in vers))


def run(index: Path, work: Path, *args: str, fail_on: str = "") -> tuple[int, str, list[str]]:
    bin_dir = work / "bin"
    bin_dir.mkdir(exist_ok=True)
    cargo = bin_dir / "cargo"
    cargo.write_text(FAKE_CARGO)
    cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
    log = work / "cargo.log"
    env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", CARGO_LOG=str(log),
               FAIL_ON=fail_on, INDEX_DIR=str(index), SCRIPTS=str(SCRIPT.parent))
    env.update({"VERS_" + n.replace("-", "_"): v for n, v in versions().items()})
    done = subprocess.run([sys.executable, str(SCRIPT), "--index", index.as_uri(), *args],
                          env=env, capture_output=True, text=True)
    calls = log.read_text().split() if log.exists() else []
    if log.exists():
        log.unlink()
    return done.returncode, done.stdout + done.stderr, calls


def main() -> int:
    problems: list[str] = []
    check = lambda cond, msg: None if cond else problems.append(msg)  # noqa: E731
    vers = versions()
    check([publish_crates.index_path(n) for n in ("a", "ab", "abc", "sciart", "SciArt-Core")]
          == ["1/a", "2/ab", "3/a/abc", "sc/ia/sciart", "sc/ia/sciart-core"],
          "DIST-R4: sparse-index paths follow the registry layout")
    with tempfile.TemporaryDirectory(prefix="sciart-publish-") as tmp:
        work = Path(tmp)
        index = work / "index"
        index.mkdir()
        # Two crates are on the index at this version, one only at an older one.
        put(index, "sciart-core", "0.0.1", vers["sciart-core"])
        put(index, "sciart-shared", vers["sciart-shared"])
        put(index, "sciart-render", "0.0.1")
        planned = publish_crates.plan(index.as_uri())
        check([n for n, _, _ in planned] == list(PUBLISH_ORDER), "DIST-R4: the plan follows PUBLISH_ORDER")
        done = {n for n, _, d in planned if d}
        check(done == {"sciart-core", "sciart-shared"},
              f"DIST-R4: exactly the versions on the index are skipped: {sorted(done)}")
        code, out, calls = run(index, work, "--dry-run")
        check(code == 0 and not calls, f"DIST-R4: --dry-run publishes nothing (exit {code}, calls {calls})")
        check("skip    sciart-core" in out and "publish sciart-render" in out and
              f"{len(PUBLISH_ORDER) - 2} of {len(PUBLISH_ORDER)} crates to publish (dry run)" in out,
              f"DIST-R4: --dry-run lists what it would do:\n{out}")
        # A publish that fails half-way, then a rerun that finishes the rest.
        code, out, calls = run(index, work, fail_on="sciart-generators")
        stop = PUBLISH_ORDER.index("sciart-generators")
        want = [n for n in PUBLISH_ORDER[: stop + 1] if n not in {"sciart-core", "sciart-shared"}]
        check(code != 0 and calls == want, f"DIST-R4: a failure stops the run there: exit {code}, {calls}")
        check("run this again" in out, "DIST-R4: a failed run says how to finish")
        code, out, calls = run(index, work)
        check(code == 0 and calls == list(PUBLISH_ORDER[stop:]),
              f"DIST-R10: the rerun publishes only what is left: exit {code}, {calls}")
        code, out, calls = run(index, work)
        check(code == 0 and not calls and "0 of" in out,
              f"DIST-R10: a third run has nothing to do: exit {code}, {calls}")

    def down(url: str) -> str | None:
        raise OSError(f"cannot reach {url}")

    # An index that cannot be read is an error, never "not published yet".
    try:
        publish_crates.published_versions("sciart", "https://index.invalid", down)
        problems.append("DIST-R4: an index error is raised, not read as 'not published'")
    except OSError:
        pass
    if problems:
        print("publish_crates:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print("publish_crates OK (plan, dry run, a failed run and its rerun)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
