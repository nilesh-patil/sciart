#!/usr/bin/env python3
"""Publish the crates to crates.io, one at a time in dependency order (SPEC-008 DIST-R4,
DIST-R10).

For each crate of `PUBLISH_ORDER` it asks the crates.io sparse index whether this version
is already there; if it is, the crate is skipped, otherwise it runs
`cargo publish -p <crate> --locked` (which waits until the index has the new version
before the next crate needs it). A run that stopped half-way, for a network error or a
rejected crate, is finished by running it again: what was published is skipped. A
published version can be yanked but never replaced, so nothing here retries a crate that
crates.io accepted.

    python3 scripts/publish_crates.py [--dry-run] [--index URL]

`--dry-run` only lists what would be published or skipped; no token is needed for it or
for the index queries. `--index` replaces https://index.crates.io (tests use a file://
directory with the same layout).
"""

from __future__ import annotations

import json
import subprocess
import sys
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_check import FACADE, PUBLISH_ORDER, internal_version, members, sciart_version  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
INDEX = "https://index.crates.io"


def index_path(name: str) -> str:
    """The sparse-index path of a crate (the registry index layout)."""
    name = name.lower()
    if len(name) <= 2:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def fetch_url(url: str) -> str | None:
    """The body at `url`, or None when the crate is not on the index (HTTP 404, or a
    missing file in a file:// index). Any other failure raises: never publish blind."""
    request = urllib.request.Request(url, headers={"User-Agent": "sciart-release-publish"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.read().decode()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise
    except urllib.error.URLError as e:
        if url.startswith("file:") and isinstance(e.reason, FileNotFoundError):
            return None
        raise


def published_versions(name: str, index: str, fetch: Callable[[str], str | None] = fetch_url) -> set[str]:
    body = fetch(f"{index.rstrip('/')}/{index_path(name)}")
    if body is None:
        return set()
    return {json.loads(line)["vers"] for line in body.splitlines() if line.strip()}


def plan(index: str, fetch: Callable[[str], str | None] = fetch_url) -> list[tuple[str, str, bool]]:
    """(crate, version, already published) for every crate, in publish order."""
    crates = members()
    version, internal = sciart_version(), str(internal_version(crates))
    out = []
    for name in PUBLISH_ORDER:
        ver = version if name == FACADE else internal
        out.append((name, ver, ver in published_versions(name, index, fetch)))
    return out


def main() -> int:
    args = sys.argv[1:]
    dry_run = "--dry-run" in args
    index = INDEX
    if "--index" in args:
        i = args.index("--index")
        if i + 1 >= len(args):
            print("--index needs a URL", file=sys.stderr)
            return 2
        index = args[i + 1]
    try:
        steps = plan(index)
    except (OSError, ValueError) as e:
        print(f"DIST-R4: cannot read the index at {index}: {e}", file=sys.stderr)
        return 1
    left = [(n, v) for n, v, done in steps if not done]
    for name, ver, done in steps:
        print(f"{'skip   ' if done else 'publish'} {name} {ver}"
              + (" (already on the index)" if done else ""))
    if dry_run or not left:
        print(f"{len(left)} of {len(steps)} crates to publish" + (" (dry run)" if dry_run else ""))
        return 0
    for name, ver in left:
        print(f"::group::cargo publish -p {name} ({ver})", flush=True)
        run = subprocess.run(["cargo", "publish", "-p", name, "--locked"], cwd=ROOT)
        print("::endgroup::", flush=True)
        if run.returncode != 0:
            print(f"DIST-R4: publishing {name} {ver} failed; fix the cause and run this again: "
                  f"the crates already published are skipped", file=sys.stderr)
            return 1
    print(f"published {len(left)} crates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
