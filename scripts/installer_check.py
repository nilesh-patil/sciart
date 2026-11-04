#!/usr/bin/env python3
"""The installer script against a local fixture release (SPEC-008 DIST-R7).

Builds a fixture release with the real packing code (`release_assets.py`, fake binaries
that name their target), then runs `packaging/install.sh` with a hermetic PATH: symlinks
to the real tools it needs, plus fake `uname`, `getconf` and `gh` that make it see another
OS, machine, glibc or attestation result. Each case checks the exit status, the message
and what was (or was not) installed; at the end the fake home's start-up files must be
untouched. shellcheck runs when it is installed (`--require-shellcheck` makes it
mandatory, as the release workflow does).

    python3 scripts/installer_check.py [--require-shellcheck]
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_assets  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "packaging" / "install.sh"
MARKER = "Real-time generative science & astronomy art"
LATEST, OLDER = "0.6.0", "0.5.0"
TOOLS = ("sh", "curl", "cut", "sed", "head", "mktemp", "rm", "tar", "gzip", "grep", "mkdir",
         "cp", "chmod", "mv", "cat")
CHECKSUM_TOOLS = ("sha256sum", "shasum")
STARTUP_FILES = (".profile", ".bashrc", ".bash_profile", ".zshrc", ".zprofile")

FAKE_UNAME = """#!/bin/sh
case "$1" in
-s) echo "$FAKE_UNAME_S" ;;
-m) echo "$FAKE_UNAME_M" ;;
*) echo "$FAKE_UNAME_S" ;;
esac
"""
FAKE_GETCONF = """#!/bin/sh
[ "$1" = GNU_LIBC_VERSION ] && [ -n "${FAKE_GLIBC:-}" ] || exit 1
echo "glibc $FAKE_GLIBC"
"""
FAKE_GH = """#!/bin/sh
case "$1 $2" in
"auth status") [ "$FAKE_GH" != signed-out ] ;;
"attestation verify") [ "$FAKE_GH" = ok ] ;;
*) exit 2 ;;
esac
"""


def fake_binary(version: str, target: str) -> bytes:
    return f"#!/bin/sh\n# {MARKER}\necho 'sciart {version} {target}'\n".encode()


FAKE_EXTRA = {"completions/sciart.bash": b"# bash\n", "completions/_sciart": b"#compdef sciart\n",
              "completions/sciart.fish": b"# fish\n", "man/sciart.1": b".TH SCIART 1\n"}


def write_release(base: Path, version: str, tamper: str | None = None) -> None:
    """`base/download/v<version>/` with every target's tarball, install.sh and SHA256SUMS;
    `tamper` names a target whose tarball changes after the sums are written."""
    out = base / "download" / f"v{version}"
    out.mkdir(parents=True)
    assets = {release_assets.asset_name(version, t):
              release_assets.pack(version, t, fake_binary(version, t), FAKE_EXTRA, 0)
              for t in release_assets.TARGETS}
    assets["install.sh"] = INSTALLER.read_bytes()
    (out / release_assets.SUMS).write_text(release_assets.sha256_sums(assets))
    if tamper:
        name = release_assets.asset_name(version, tamper)
        assets[name] = release_assets.pack(version, tamper, b"#!/bin/sh\necho evil\n", FAKE_EXTRA, 0)
    for name, data in assets.items():
        (out / name).write_bytes(data)


@dataclass
class Case:
    name: str
    os: str = "Darwin"
    machine: str = "arm64"
    glibc: str = ""
    args: list[str] = field(default_factory=list)
    base: str = "good"
    env: dict[str, str] = field(default_factory=dict)
    gh: str | None = None  # None: no gh on PATH; else ok | fail | signed-out
    checksum: bool = True
    existing: bytes | None = None  # what DIR/sciart holds before the run
    ok: bool = True
    says: str = ""
    installs: str | None = None  # "<version> <target>" the installed fake binary prints


CASES = [
    Case("macOS arm64, pinned version", args=["--version", LATEST],
         installs=f"{LATEST} aarch64-apple-darwin", says="checksum verified"),
    Case("macOS arm64, latest", installs=f"{LATEST} aarch64-apple-darwin"),
    Case("an older version, with a v prefix", args=["--version", f"v{OLDER}"],
         installs=f"{OLDER} aarch64-apple-darwin"),
    Case("macOS x86_64", machine="x86_64", installs=f"{LATEST} x86_64-apple-darwin"),
    Case("Linux x86_64, glibc 2.35", os="Linux", machine="x86_64", glibc="2.35",
         installs=f"{LATEST} x86_64-unknown-linux-gnu"),
    Case("Linux aarch64, glibc 2.39", os="Linux", machine="aarch64", glibc="2.39",
         installs=f"{LATEST} aarch64-unknown-linux-gnu"),
    Case("Linux, glibc 2.31 is too old", os="Linux", machine="x86_64", glibc="2.31",
         installs=f"{LATEST} x86_64-unknown-linux-musl", says="static build"),
    Case("Linux without glibc", os="Linux", machine="aarch64",
         installs=f"{LATEST} aarch64-unknown-linux-musl"),
    Case("checksum mismatch", base="tampered", args=["--version", LATEST], ok=False,
         says="checksum mismatch"),
    Case("Windows", os="MINGW64_NT-10.0", machine="x86_64", ok=False, says="unsupported OS"),
    Case("FreeBSD", os="FreeBSD", machine="amd64", ok=False, says="unsupported OS"),
    Case("32-bit x86", os="Linux", machine="i686", glibc="2.35", ok=False, says="unsupported machine"),
    Case("32-bit ARM", os="Linux", machine="armv7l", glibc="2.35", ok=False, says="unsupported machine"),
    Case("plain http", env={"SCIART_INSTALL_BASE_URL": "http://example.invalid/releases"},
         ok=False, says="non-https"),
    Case("under sudo", env={"SUDO_USER": "someone"}, ok=False, says="sudo"),
    Case("no checksum tool", checksum=False, ok=False, says="sha256sum or shasum"),
    Case("a version that does not exist", args=["--version", "9.9.9"], ok=False, says="download failed"),
    Case("replacing another program", existing=b"#!/bin/sh\necho other\n", ok=False, says="--force"),
    Case("replacing another program with --force", existing=b"#!/bin/sh\necho other\n",
         args=["--force"], installs=f"{LATEST} aarch64-apple-darwin"),
    Case("upgrading an installed sciart", existing=fake_binary(OLDER, "aarch64-apple-darwin"),
         installs=f"{LATEST} aarch64-apple-darwin"),
    Case("attestation verifies", gh="ok", installs=f"{LATEST} aarch64-apple-darwin",
         says="attestation verified"),
    Case("attestation fails", gh="fail", ok=False, says="does not verify"),
    Case("gh signed out", gh="signed-out", installs=f"{LATEST} aarch64-apple-darwin",
         says="not checked"),
]


def hermetic_path(where: Path, case: Case) -> Path:
    bin_dir = where / f"bin-{abs(hash(case.name))}"
    bin_dir.mkdir()
    tools = TOOLS + (CHECKSUM_TOOLS if case.checksum else ())
    for tool in tools:
        real = shutil.which(tool)
        if real:
            (bin_dir / tool).symlink_to(real)
    fakes = {"uname": FAKE_UNAME, "getconf": FAKE_GETCONF}
    if case.gh is not None:
        fakes["gh"] = FAKE_GH
    for name, text in fakes.items():
        path = bin_dir / name
        path.write_text(text)
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return bin_dir


def run_case(work: Path, home: Path, case: Case) -> list[str]:
    dest_dir = work / f"dest-{abs(hash(case.name))}"
    dest_dir.mkdir()
    dest = dest_dir / "sciart"
    if case.existing is not None:
        dest.write_bytes(case.existing)
        dest.chmod(0o755)
    bin_dir = hermetic_path(work, case)
    env = {
        "PATH": str(bin_dir),
        "HOME": str(home),
        "SCIART_INSTALL_DIR": str(dest_dir),
        "SCIART_INSTALL_BASE_URL": (work / case.base).as_uri(),
        "FAKE_UNAME_S": case.os,
        "FAKE_UNAME_M": case.machine,
        "FAKE_GLIBC": case.glibc,
        "FAKE_GH": case.gh or "",
    }
    env.update(case.env)
    run = subprocess.run([str(bin_dir / "sh"), str(INSTALLER), *case.args], env=env,
                         capture_output=True, text=True, timeout=60)
    output = run.stdout + run.stderr
    problems = []
    if (run.returncode == 0) != case.ok:
        problems.append(f"exit {run.returncode}, expected {'success' if case.ok else 'a refusal'}")
    if case.says and case.says not in output:
        problems.append(f"output lacks {case.says!r}")
    if case.installs is not None:
        if not dest.is_file():
            problems.append("nothing installed")
        else:
            printed = subprocess.run(["/bin/sh", str(dest)], capture_output=True, text=True).stdout.strip()
            if printed != f"sciart {case.installs}":
                problems.append(f"installed {printed!r}, expected 'sciart {case.installs}'")
            if not os.access(dest, os.X_OK):
                problems.append("installed binary is not executable")
    elif not case.ok:
        after = dest.read_bytes() if dest.exists() else None
        if after != case.existing:
            problems.append("a refused install changed the destination")
    leftovers = [p.name for p in dest_dir.iterdir() if p.name != "sciart"]
    if leftovers:
        problems.append(f"left files behind: {leftovers}")
    return [f"{case.name}: {p}\n    {output.strip()}" for p in problems]


def shellcheck(required: bool) -> list[str]:
    tool = shutil.which("shellcheck")
    if tool is None:
        if required:
            return ["DIST-R7: shellcheck is not installed"]
        print("DIST-R7: shellcheck not installed; skipped (the release workflow requires it)")
        return []
    run = subprocess.run([tool, "--shell=sh", str(INSTALLER)], capture_output=True, text=True)
    return [] if run.returncode == 0 else [f"DIST-R7: shellcheck:\n{run.stdout}{run.stderr}"]


def main() -> int:
    problems: list[str] = []
    required = "--require-shellcheck" in sys.argv[1:]
    if not INSTALLER.read_text().startswith("#!/bin/sh\n"):
        problems.append("DIST-R7: packaging/install.sh starts with #!/bin/sh (POSIX sh)")
    problems += shellcheck(required)
    with tempfile.TemporaryDirectory(prefix="sciart-installer-") as tmp:
        work = Path(tmp)
        for version in (OLDER, LATEST):
            write_release(work / "good", version)
        latest = work / "good" / "latest" / "download"
        latest.mkdir(parents=True)
        shutil.copy(work / "good" / "download" / f"v{LATEST}" / release_assets.SUMS, latest)
        write_release(work / "tampered", LATEST, tamper="aarch64-apple-darwin")
        home = work / "home"
        home.mkdir()
        for name in STARTUP_FILES:
            (home / name).write_text(f"# {name}\n")
        before = sorted(p.relative_to(home) for p in home.rglob("*"))
        for case in CASES:
            problems += [f"DIST-R7: {p}" for p in run_case(work, home, case)]
        for name in STARTUP_FILES:
            if (home / name).read_text() != f"# {name}\n":
                problems.append(f"DIST-R7: the installer edited ~/{name}")
        after = sorted(p.relative_to(home) for p in home.rglob("*"))
        if after != before:
            problems.append(f"DIST-R7: the installer wrote into the home directory: "
                            f"{sorted(set(map(str, after)) - set(map(str, before)))}")
    help_run = subprocess.run(["/bin/sh", str(INSTALLER), "--help"], capture_output=True, text=True)
    if help_run.returncode != 0 or "--version" not in help_run.stdout:
        problems.append("DIST-R7: install.sh --help prints the usage and exits 0")
    if problems:
        print("installer:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"installer OK ({len(CASES)} cases against a fixture release)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
