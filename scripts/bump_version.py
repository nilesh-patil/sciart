#!/usr/bin/env python3
"""Set the two version streams (SPEC-008 DIST-R1, DIST-R3).

    python3 scripts/bump_version.py <sciart-version> <internal-version> [--root DIR]

writes the version of `sciart` (`[workspace.package] version` in the root Cargo.toml) and
the internal crates' shared 0.x version (each internal crate's `version` line and its `=`
pin in `[workspace.dependencies]`), and the versions of the workspace crates in every
Cargo.lock of the repository that names one (found by search: the workspace's, fuzz/'s,
web/'s, ...; registry packages are left alone). Nothing is built or fetched. It does not
touch CHANGELOG.md, which needs a `## [<version>] - YYYY-MM-DD` heading before the tag
(`scripts/release_check.py --tag v<version>` checks both). `--root` is for tests.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_check import INTERNAL, SEMVER, lock_files, lock_versions  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def replace_once(path: Path, pattern: str, repl: str, root: Path) -> None:
    text = path.read_text()
    new, count = re.subn(pattern, repl, text, count=1, flags=re.M)
    if count != 1:
        raise SystemExit(f"{path.relative_to(root)}: no match for {pattern!r}")
    path.write_text(new)


def bump_lock(text: str, versions: dict[str, str]) -> str:
    """Set the version of each path package (no `source`) that `versions` names."""
    def entry(found: re.Match[str]) -> str:
        name, rest = found.group(2), found.group(4)
        if name not in versions or rest.startswith("source = "):
            return found.group(0)
        return f'{found.group(1)}"{versions[name]}"\n{rest}'
    return re.sub(r'^(\[\[package\]\]\nname = "([^"]+)"\nversion = )("[^"]*")\n(.*)$', entry, text, flags=re.M)


def main(argv: list[str]) -> int:
    root = ROOT
    if "--root" in argv:
        i = argv.index("--root")
        root = Path(argv[i + 1]).resolve()
        argv = argv[:i] + argv[i + 2:]
    if len(argv) != 2:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    version, internal = argv[0].removeprefix("v"), argv[1].removeprefix("v")
    if not SEMVER.match(version) or not SEMVER.match(internal):
        print(f"not semver: {version!r} / {internal!r}", file=sys.stderr)
        return 2
    if not internal.startswith("0."):
        print(f"the internal crates stay at 0.x (DIST-R3): {internal!r}", file=sys.stderr)
        return 2
    manifest = root / "Cargo.toml"
    replace_once(manifest, r'^(\[workspace\.package\]\n(?:#.*\n)*version = )"[^"]*"', rf'\g<1>"{version}"', root)
    for name in INTERNAL:
        replace_once(manifest, rf'^({re.escape(name)} = \{{ path = "[^"]*", version = )"=[^"]*"',
                     rf'\g<1>"={internal}"', root)
        replace_once(root / "crates" / name / "Cargo.toml", r'^version = "[^"]*"$', f'version = "{internal}"', root)
    versions = lock_versions(version, internal)
    for lock in lock_files(root):
        lock.write_text(bump_lock(lock.read_text(), versions))
        print(f"updated {lock.relative_to(root)}")
    print(f"sciart {version}, internal crates {internal}. Next: the CHANGELOG heading "
          f"`## [{version}] - YYYY-MM-DD`, then `python3 scripts/release_check.py --tag v{version}`.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
