#!/usr/bin/env python3
"""Publish the crates to crates.io, one at a time in dependency order (SPEC-008 DIST-R4,
DIST-R10).

For each crate of `PUBLISH_ORDER` it asks the crates.io sparse index whether this version
is already there; if it is, the crate is skipped, otherwise it runs
`cargo publish -p <crate> --locked` (which waits until the index has the new version
before the next crate needs it). A run that stopped half-way, for a network error or a
rejected crate, is finished by running it again: what was published is skipped. A
published version can be yanked but never replaced, so nothing here retries a crate that
crates.io accepted. A crate that crates.io turns away for its new-crate rate limit (429,
"Please try again after <date>") is tried again at that time, within a bounded number of
tries and minutes of waiting.

    python3 scripts/publish_crates.py [--dry-run] [--index URL]

`--dry-run` only lists what would be published or skipped; no token is needed for it or
for the index queries. `--index` replaces https://index.crates.io (tests use a file://
directory with the same layout).
"""

from __future__ import annotations

import email.utils
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from datetime import datetime, timezone
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
    return publish_all(left)


# crates.io lets a burst of new crates through, then about one every ten minutes; past that
# it answers 429 with "Please try again after <HTTP date>". The run waits that out, within
# these bounds, instead of stopping for a re-run.
RETRY_AFTER = re.compile(r"try again after ([A-Z][a-z]{2}, \d{1,2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} "
                         r"(?:GMT|UTC|[+-]\d{4}))")
RETRY_MARGIN = 5.0  # seconds after the time crates.io names
MAX_TRIES = 12  # per crate, the first attempt included
MAX_WAIT = 90 * 60.0  # seconds of waiting over the whole run


def cargo_publish(name: str) -> tuple[int, str]:
    """`cargo publish -p <name> --locked`, its output shown and returned."""
    run = subprocess.run(["cargo", "publish", "-p", name, "--locked"], cwd=ROOT, capture_output=True, text=True)
    sys.stdout.write(run.stdout)
    sys.stderr.write(run.stderr)
    sys.stdout.flush()
    return run.returncode, run.stdout + run.stderr


def retry_at(output: str) -> datetime | None:
    """When crates.io says a rate-limited publish may be tried again, or None when the
    failure is anything else."""
    if "429" not in output:
        return None
    found = RETRY_AFTER.search(output)
    if found is None:
        return None
    try:
        return email.utils.parsedate_to_datetime(found.group(1))
    except (TypeError, ValueError):
        return None


def publish_all(left: list[tuple[str, str]],
                publish: Callable[[str], tuple[int, str]] = cargo_publish,
                sleep: Callable[[float], None] = time.sleep,
                now: Callable[[], datetime] = lambda: datetime.now(timezone.utc)) -> int:
    """Publish `left` in order. A rate-limited crate is tried again once crates.io allows
    it (at most MAX_TRIES times, at most MAX_WAIT seconds of waiting in all); any other
    failure stops the run, which a re-run finishes."""
    waited = 0.0
    for name, ver in left:
        for attempt in range(1, MAX_TRIES + 1):
            print(f"::group::cargo publish -p {name} ({ver})", flush=True)
            code, output = publish(name)
            print("::endgroup::", flush=True)
            if code == 0:
                break
            when = retry_at(output)
            if when is None:
                print(f"DIST-R4: publishing {name} {ver} failed; fix the cause and run this again: "
                      f"the crates already published are skipped", file=sys.stderr)
                return 1
            wait = max(0.0, (when - now()).total_seconds()) + RETRY_MARGIN
            if attempt == MAX_TRIES or waited + wait > MAX_WAIT:
                print(f"DIST-R4: crates.io still limits new crates at {name} {ver} (until "
                      f"{when.isoformat()}); the wait would pass {MAX_TRIES} tries or "
                      f"{MAX_WAIT / 60:.0f} minutes. Run this again later: the crates already "
                      f"published are skipped", file=sys.stderr)
                return 1
            print(f"crates.io rate limit for {name} {ver}: waiting {wait / 60:.1f} minutes, until "
                  f"{when.isoformat()} (try {attempt + 1} of {MAX_TRIES})", flush=True)
            sleep(wait)
            waited += wait
    print(f"published {len(left)} crates" + (f" (waited {waited / 60:.1f} minutes for the rate limit)"
                                             if waited else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
