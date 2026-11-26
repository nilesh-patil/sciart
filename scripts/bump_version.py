#!/usr/bin/env python3
"""Set the two version streams (SPEC-008 DIST-R1, DIST-R3).

    python3 scripts/bump_version.py <sciart-version> <internal-version>

writes the version of `sciart` (`[workspace.package] version` in the root Cargo.toml) and
the internal crates' shared 0.x version (each internal crate's `version` line and its `=`
pin in `[workspace.dependencies]`), then refreshes the workspace entries of Cargo.lock
(`cargo update --workspace --offline`) and of fuzz/Cargo.lock. It does not touch
CHANGELOG.md, which needs a `## [<version>] - YYYY-MM-DD` heading before the tag
(`scripts/release_check.py --tag v<version>` checks both).
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_check import INTERNAL, SEMVER  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def replace_once(path: Path, pattern: str, repl: str) -> None:
    text = path.read_text()
    new, count = re.subn(pattern, repl, text, count=1, flags=re.M)
    if count != 1:
        raise SystemExit(f"{path.relative_to(ROOT)}: no match for {pattern!r}")
    path.write_text(new)


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    version, internal = sys.argv[1].removeprefix("v"), sys.argv[2].removeprefix("v")
    if not SEMVER.match(version) or not SEMVER.match(internal):
        print(f"not semver: {version!r} / {internal!r}", file=sys.stderr)
        return 2
    if not internal.startswith("0."):
        print(f"the internal crates stay at 0.x (DIST-R3): {internal!r}", file=sys.stderr)
        return 2
    root = ROOT / "Cargo.toml"
    replace_once(root, r'^(\[workspace\.package\]\n(?:#.*\n)*version = )"[^"]*"', rf'\g<1>"{version}"')
    for name in INTERNAL:
        replace_once(root, rf'^({re.escape(name)} = \{{ path = "[^"]*", version = )"=[^"]*"',
                     rf'\g<1>"={internal}"')
        replace_once(ROOT / "crates" / name / "Cargo.toml", r'^version = "[^"]*"$', f'version = "{internal}"')
    fuzz = ROOT / "fuzz" / "Cargo.lock"
    if fuzz.exists():
        for name in ("sciart-core", "sciart-render"):
            replace_once(fuzz, rf'^(name = "{name}"\nversion = )"[^"]*"', rf'\g<1>"{internal}"')
    subprocess.run(["cargo", "update", "--workspace", "--offline"], cwd=ROOT, check=True)
    print(f"sciart {version}, internal crates {internal}. Next: the CHANGELOG heading "
          f"`## [{version}] - YYYY-MM-DD`, then `python3 scripts/release_check.py --tag v{version}`.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
