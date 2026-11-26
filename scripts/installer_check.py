#!/usr/bin/env python3
"""The installer script against a local fixture release (SPEC-008 DIST-R7).

Builds a fixture release with the real packing code (`release_assets.py`, fake binaries
that name their target), then runs `packaging/install.sh` with a hermetic PATH: symlinks
to the real tools it needs, plus fakes that make it see another OS, machine, userland,
glibc, Rosetta or `gh`, and a `curl` wrapper that logs every call (an https fixture host
is served from the same files, so the https path runs offline). Each case checks the exit
status, the message, what was (or was not) installed, that no request was made when the
installer had to refuse first, that every download used `--proto`, `--proto-redir` and
`--tlsv1.2`, and that no temporary file outlived the run, interrupted or not. At the end
the fake home's start-up files must be untouched.

Then a mutation table breaks the installer in the ways a careless edit could (checksum
skipped on the latest path, glibc floor lowered, Rosetta ignored, version not validated,
TLS or redirect flags dropped, cleanup traps removed, sudo half-detected, ...) and each
broken installer must fail at least one of the cases named for it.

The documented one-line command (`curl … | sh` in the README and the installer's header)
must follow redirects and keep its https-only flags; it is also run against a local fixture
host that redirects `latest/download/` the way the release host does, and a second table of
command variants (no `-L`, no `-f`, ...) must each be judged right.

shellcheck runs when it is installed (`--require-shellcheck` makes it mandatory, as the
release workflow does).

    python3 scripts/installer_check.py [--require-shellcheck]
"""

from __future__ import annotations

import http.server
import os
import re
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_assets  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "packaging" / "install.sh"
MARKER = "Real-time generative science & astronomy art"
LATEST, OLDER = "0.6.0", "0.5.0"
HTTPS_FIXTURE = "https://fixture.invalid/releases"
TOOLS = ("sh", "cut", "sed", "head", "mktemp", "rm", "tar", "gzip", "grep", "mkdir",
         "cp", "chmod", "cat", "sleep", "touch")
CHECKSUM_TOOLS = ("sha256sum", "shasum")
STARTUP_FILES = (".profile", ".bashrc", ".bash_profile", ".zshrc", ".zprofile")

FAKE_UNAME = """#!/bin/sh
case "$1" in
-m) echo "$FAKE_UNAME_M" ;;
*) echo "$FAKE_UNAME_S" ;;
esac
"""
FAKE_GETCONF = """#!/bin/sh
case "$1" in
LONG_BIT) echo "${FAKE_LONG_BIT:-64}" ;;
GNU_LIBC_VERSION) [ -n "${FAKE_GLIBC:-}" ] || exit 1; echo "glibc $FAKE_GLIBC" ;;
*) exit 1 ;;
esac
"""
FAKE_SYSCTL = """#!/bin/sh
[ "$1 $2" = "-n sysctl.proc_translated" ] || exit 1
echo "${FAKE_ROSETTA:-0}"
"""
FAKE_GH = """#!/bin/sh
case "$1 ${2:-}" in
"attestation --help") [ "$FAKE_GH" != old ] ;;
"auth status") [ "$FAKE_GH" != signed-out ] ;;
"attestation verify") [ "$FAKE_GH" = ok ] ;;
*) exit 2 ;;
esac
"""
# Logs each call (one line of arguments), optionally stalls (for the interrupt case), and
# serves the https fixture host from the fixture files with the same flags.
CURL = """#!/bin/sh
printf '%s\\n' "$*" >> "$CURL_LOG"
if [ -n "${CURL_STALL:-}" ]; then
    case "$*" in *"$CURL_STALL"*) touch "$CURL_STARTED"; sleep 30 ;; esac
fi
for a in "$@"; do last="$a"; done
case "$last" in
https://fixture.invalid/*)
    url="file://$FIXTURE_ROOT/${last#https://fixture.invalid/releases/}"
    count=$#
    n=0
    for a in "$@"; do
        n=$((n + 1))
        if [ "$n" -lt "$count" ]; then
            [ "$a" = "=https" ] && a="=file"
            set -- "$@" "$a"
        fi
    done
    shift "$count"
    exec "$REAL_CURL" "$@" "$url"
    ;;
esac
exec "$REAL_CURL" "$@"
"""
FAKE_MV_FAIL = """#!/bin/sh
exit 1
"""


def fake_binary(version: str, target: str) -> bytes:
    return f"#!/bin/sh\n# {MARKER}\necho 'sciart {version} {target}'\n".encode()


FAKE_EXTRA = {"completions/sciart.bash": b"# bash\n", "completions/_sciart": b"#compdef sciart\n",
              "completions/sciart.fish": b"# fish\n", "man/sciart.1": b".TH SCIART 1\n"}


def write_release(base: Path, version: str, tamper: str | None = None, latest: bool = False) -> None:
    """`base/download/v<version>/` with every target's tarball, install.sh and SHA256SUMS
    (and `base/latest/download/SHA256SUMS` with `latest`); `tamper` names a target whose
    tarball changes after the sums are written."""
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
    if latest:
        (base / "latest" / "download").mkdir(parents=True)
        shutil.copy(out / release_assets.SUMS, base / "latest" / "download")


@dataclass
class Case:
    name: str
    os: str = "Darwin"
    machine: str = "arm64"
    glibc: str = ""
    long_bit: str = "64"
    rosetta: str = "0"
    args: list[str] = field(default_factory=list)
    base: str = "good"
    env: dict[str, str] = field(default_factory=dict)
    gh: str | None = None  # None: no gh on PATH; else ok | fail | signed-out | old
    checksum: bool = True
    mv_fails: bool = False
    existing: bytes | None = None  # what DIR/sciart holds before the run
    ok: bool = True
    says: str = ""
    installs: str | None = None  # "<version> <target>" the installed fake binary prints
    no_request: bool = False  # refused before any download
    proto: str = "=file"  # the protocol every download must be restricted to
    interrupt: str = ""  # send SIGTERM while downloading the file named here


HOSTILE = ["1.0.0;id", "../0.6.0", "0.6.0/../x", "..", "$(id)", "-rf", "0.6.0 x", "v", "0.6.0+evil"]
CASES = [
    Case("macOS arm64, pinned version", args=["--version", LATEST],
         installs=f"{LATEST} aarch64-apple-darwin", says="checksum verified"),
    Case("macOS arm64, latest", installs=f"{LATEST} aarch64-apple-darwin"),
    Case("an older version, with a v prefix", args=["--version", f"v{OLDER}"],
         installs=f"{OLDER} aarch64-apple-darwin"),
    Case("macOS x86_64", machine="x86_64", installs=f"{LATEST} x86_64-apple-darwin"),
    Case("Rosetta", machine="x86_64", rosetta="1", installs=f"{LATEST} aarch64-apple-darwin"),
    Case("Linux x86_64, glibc 2.35", os="Linux", machine="x86_64", glibc="2.35",
         installs=f"{LATEST} x86_64-unknown-linux-gnu"),
    Case("Linux aarch64, glibc 2.39", os="Linux", machine="aarch64", glibc="2.39",
         installs=f"{LATEST} aarch64-unknown-linux-gnu"),
    Case("Linux, glibc 2.34 is too old", os="Linux", machine="x86_64", glibc="2.34",
         installs=f"{LATEST} x86_64-unknown-linux-musl", says="static build"),
    Case("Linux, glibc 2.10 is too old", os="Linux", machine="x86_64", glibc="2.10",
         installs=f"{LATEST} x86_64-unknown-linux-musl"),
    Case("Linux without glibc", os="Linux", machine="aarch64",
         installs=f"{LATEST} aarch64-unknown-linux-musl"),
    Case("over https", base="https", args=["--version", LATEST], proto="=https",
         installs=f"{LATEST} aarch64-apple-darwin"),
    Case("over https, latest", base="https", proto="=https", installs=f"{LATEST} aarch64-apple-darwin"),
    Case("checksum mismatch, pinned", base="tampered", args=["--version", LATEST], ok=False,
         says="checksum mismatch"),
    Case("checksum mismatch, latest", base="tampered", ok=False, says="checksum mismatch"),
    Case("Windows", os="MINGW64_NT-10.0", machine="x86_64", ok=False, says="unsupported OS", no_request=True),
    Case("FreeBSD", os="FreeBSD", machine="amd64", ok=False, says="unsupported OS", no_request=True),
    Case("32-bit x86", os="Linux", machine="i686", glibc="2.35", ok=False, says="unsupported machine",
         no_request=True),
    Case("32-bit ARM", os="Linux", machine="armv7l", glibc="2.35", ok=False, says="unsupported machine",
         no_request=True),
    Case("32-bit userland", os="Linux", machine="x86_64", glibc="2.35", long_bit="32", ok=False,
         says="32-bit userland", no_request=True),
    Case("plain http", env={"SCIART_INSTALL_BASE_URL": "http://example.invalid/releases"},
         ok=False, says="neither https", no_request=True),
    Case("under sudo", env={"SUDO_USER": "someone"}, ok=False, says="sudo", no_request=True),
    Case("under sudo, SUDO_UID only", env={"SUDO_UID": "501"}, ok=False, says="sudo", no_request=True),
    Case("no checksum tool", checksum=False, ok=False, says="sha256sum or shasum", no_request=True),
    Case("a version that does not exist", args=["--version", "9.9.9"], ok=False, says="download failed"),
    *[Case(f"hostile version {v!r}", args=["--version", v], ok=False, says="not a version", no_request=True)
      for v in HOSTILE],
    Case("replacing another program", existing=b"#!/bin/sh\necho other\n", ok=False, says="--force"),
    Case("replacing another program with --force", existing=b"#!/bin/sh\necho other\n",
         args=["--force"], installs=f"{LATEST} aarch64-apple-darwin"),
    Case("upgrading an installed sciart", existing=fake_binary(OLDER, "aarch64-apple-darwin"),
         installs=f"{LATEST} aarch64-apple-darwin"),
    Case("attestation verifies", gh="ok", installs=f"{LATEST} aarch64-apple-darwin",
         says="attestation verified"),
    Case("attestation fails", gh="fail", ok=False, says="does not verify"),
    Case("gh signed out", gh="signed-out", installs=f"{LATEST} aarch64-apple-darwin", says="not signed in"),
    Case("gh without the attestation command", gh="old", installs=f"{LATEST} aarch64-apple-darwin",
         says="no attestation command"),
    Case("a failed move leaves no staged file", mv_fails=True, ok=False),
    Case("interrupted while downloading", args=["--version", LATEST], ok=False,
         interrupt=release_assets.asset_name(LATEST, "aarch64-apple-darwin")),
]
CASE_NAMES = {c.name for c in CASES}
ANY = "macOS arm64, pinned version"
# (label, [(text, replacement)], the cases of which at least one must fail)
MUTATIONS = [
    ("checksum skipped unless --version was given",
     [('        valid_version "$version" || die "the latest release names an invalid version: $version"\n',
       '        valid_version "$version" || die "the latest release names an invalid version: $version"\n'
       '        unpinned=1\n'),
      ('    [ "$got" = "$want" ] ||', '    [ -n "${unpinned:-}" ] || [ "$got" = "$want" ] ||')],
     ["checksum mismatch, latest"]),
    ("glibc 2.34 accepted", [('GLIBC_FLOOR="2.35"', 'GLIBC_FLOOR="2.34"')], ["Linux, glibc 2.34 is too old"]),
    ("glibc compared as text", [('at_least "$glibc" "$GLIBC_FLOOR"', '[ "$glibc" != "" ]')],
     ["Linux, glibc 2.34 is too old"]),
    ("no Rosetta switch", [("        arch=aarch64\n    fi\n", "    fi\n")], ["Rosetta"]),
    ("no --version validation", [('    if [ "$pinned" -eq 1 ] && ! valid_version "$version"; then\n'
                                   '        die "not a version: $version"\n    fi\n', "")],
     [c.name for c in CASES if c.name.startswith("hostile")]),
    ("a weaker version pattern", [("*[!0-9A-Za-z.-]* | *..* | [!0-9]*", "*[!0-9A-Za-z./-]*")],
     [c.name for c in CASES if c.name.startswith("hostile")]),
    ("http allowed on redirect", [('--proto-redir "$proto" --tlsv1.2 -fsSL', '--tlsv1.2 -fsSL')], [ANY]),
    ("TLS below 1.2", [('--proto-redir "$proto" --tlsv1.2 -fsSL', '--proto-redir "$proto" -fsSL')], [ANY]),
    ("any protocol", [('curl --proto "$proto" --proto-redir', "curl --proto-redir")], [ANY]),
    ("temporary files left on failure", [("    trap cleanup EXIT\n", "")], ["checksum mismatch, pinned"]),
    ("temporary files left on interrupt", [("    trap 'exit 1' INT TERM HUP\n", "")],
     ["interrupted while downloading"]),
    ("the staged binary left on failure", [('    if [ -n "$staged" ]; then rm -f "$staged"; fi\n', "")],
     ["a failed move leaves no staged file"]),
    ("SUDO_UID ignored", [(' || [ -n "${SUDO_UID:-}" ]', "")], ["under sudo, SUDO_UID only"]),
    ("sudo ignored", [('    if [ -n "${SUDO_USER:-}" ] || [ -n "${SUDO_UID:-}" ]; then\n'
                       '        die "refusing to run under sudo; install into a directory you own (--to DIR)"\n'
                       '    fi\n', "")], ["under sudo"]),
    ("32-bit userland ignored", [('        [ "$bits" = 64 ] || die "a ${bits}-bit userland (64-bit binaries only)"\n',
                                  "")], ["32-bit userland"]),
    ("a failed attestation tolerated", [('            die "the build-provenance attestation of $asset does not verify"',
                                         '            say "the build-provenance attestation of $asset does not verify"')],
     ["attestation fails"]),
    ("an old gh treated as a failure", [("    elif ! gh attestation --help >/dev/null 2>&1; then\n"
                                         "        say \"this gh has no attestation command: provenance attestation "
                                         "not checked (the checksum was)\"\n", "")],
     ["gh without the attestation command"]),
    ("http accepted", [("    https://*) proto=\"=https\" ;;\n", "    http*) proto=\"=https\" ;;\n")],
     ["plain http"]),
    ("the version checked after the download", [('    if [ "$pinned" -eq 1 ] && ! valid_version "$version"; then\n'
                                                 '        die "not a version: $version"\n    fi\n', ""),
                                                ('    asset="sciart-$version-$target.tar.gz"\n',
                                                 '    valid_version "$version" || die "not a version: $version"\n'
                                                 '    asset="sciart-$version-$target.tar.gz"\n')],
     [c.name for c in CASES if c.name.startswith("hostile")]),
    ("another program replaced", [('[ "$force" -ne 1 ] && ', '[ "$force" -ne 0 ] && ')], ["replacing another program"]),
]


# ---- The documented one-line command -----------------------------------------------------
#
# The release host answers `releases/latest/download/<asset>` with a redirect (to
# `download/v<version>/<asset>`, then to its asset host). curl without -L prints the empty
# redirect body, so `curl ... | sh` reads nothing and exits 0: nothing installed, nothing
# said. Two checks divide the work. The static rule reads the command verbatim wherever it
# is documented (the README, the installer's header; the site check applies it to the built
# site) and needs every flag: https only (`--proto '=https'`), TLS 1.2 or newer, follow
# redirects, fail on an HTTP error, show errors. The fixture run serves a release over local
# plain http that redirects like the real host and runs the README's curl flags against it,
# with the one https-only value swapped (`=https` -> `=http`): it proves the redirect is
# followed and that the script fetched that way installs.

DOC_URL = "https://github.com/nilesh-patil/sciart/releases/latest/download/install.sh"
README = ROOT / "README.md"


def logical_lines(text: str, comment: bool = False) -> list[str]:
    """`text` as shell lines: with `comment`, a leading `#` and its spaces are dropped first;
    a line ending in a backslash continues on the next."""
    out, pending = [], ""
    for line in text.splitlines():
        if comment:
            line = re.sub(r"^#\s*", "", line)
        line = line.strip()
        if line.endswith("\\"):
            pending += line[:-1].strip() + " "
            continue
        out.append(pending + line)
        pending = ""
    return out


def documented_commands(text: str, comment: bool = False) -> list[str]:
    """The one-line install commands in `text`: a curl of the installer piped to a shell."""
    return [line for line in logical_lines(text, comment) if line.startswith("curl ") and DOC_URL in line]


def _short_groups(tokens: list[str]) -> str:
    return "".join(t[1:] for t in tokens if re.fullmatch(r"-[A-Za-z0-9]+", t))


def one_liner_problems(command: str) -> list[str]:
    """DIST-R7 over one documented command, read as the shell would split it."""
    fetch, _, rest = command.partition("|")
    try:
        tokens = shlex.split(fetch)
    except ValueError as e:
        return [f"cannot read the command ({e}): {command}"]
    short = _short_groups(tokens)
    problems = []
    if not tokens or tokens[0] != "curl" or DOC_URL not in tokens:
        problems.append(f"it does not curl {DOC_URL}")
    if rest.strip() != "sh":
        problems.append("it does not pipe the script to `sh`")
    if _after(tokens, "--proto") != "=https":
        problems.append("it does not restrict curl to https (`--proto '=https'`)")
    if "--tlsv1.2" not in tokens:
        problems.append("it does not require TLS 1.2 or newer (`--tlsv1.2`)")
    for letter, long, what in (("L", "--location", "follow redirects (the release host redirects "
                                "latest/download, and without it `sh` reads nothing and exits 0)"),
                               ("f", "--fail", "fail on an HTTP error"),
                               ("S", "--show-error", "show errors")):
        if letter not in short and long not in tokens:
            problems.append(f"curl does not {what}: `-{letter}` or `{long}`")
    return [f"{p}: {command}" for p in problems]


def documented_problems(readme: str, installer: str) -> list[str]:
    problems = []
    for where, commands in (("README.md", documented_commands(readme)),
                            ("packaging/install.sh", documented_commands(installer, comment=True))):
        if not commands:
            problems.append(f"DIST-R7: {where} documents no one-line install command ({DOC_URL} | sh)")
        problems += [f"DIST-R7: {where}: {p}" for c in commands for p in one_liner_problems(c)]
    return problems


class _Release(http.server.BaseHTTPRequestHandler):
    """A release host over plain http that redirects like the real one: `latest/download/`
    to `download/v<latest>/`, and that to an asset path, which serves the file."""

    root = Path()

    def do_GET(self) -> None:  # noqa: N802 (the handler's name)
        path = urllib.parse.urlparse(self.path).path
        latest, versioned = "/releases/latest/download/", "/releases/download/"
        if path.startswith(latest):
            self._redirect(f"{versioned}v{LATEST}/{path[len(latest):]}")
        elif path.startswith(versioned):
            self._redirect(f"/assets/{path[len(versioned):]}")
        elif path.startswith("/assets/") and (self.root / "download" / path[len("/assets/"):]).is_file():
            data = (self.root / "download" / path[len("/assets/"):]).read_bytes()
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        else:
            self.send_error(404)

    def _redirect(self, location: str) -> None:
        self.send_response(302)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *_: object) -> None:
        pass


def fetch_like(command: str, base: str) -> bytes | None:
    """The body the documented command's curl gives at the fixture host `base` (http), with
    `=https` swapped for `=http`; None when curl fails."""
    tokens = shlex.split(command.partition("|")[0])
    path = DOC_URL[DOC_URL.index("/releases/"):]
    tokens = [base + path if t == DOC_URL else ("=http" if t == "=https" else t) for t in tokens]
    run = subprocess.run(tokens, capture_output=True, timeout=30)
    return run.stdout if run.returncode == 0 else None


# (label, the README command's text, its replacement, whether the rule must reject it,
# whether the fixture fetch must come back empty)
DOC_MUTATIONS = [
    ("the command without -L", "-LsSf", "-sSf", True, True),
    ("without -f", "-LsSf", "-LsS", True, False),
    ("without -S", "-LsSf", "-Lsf", True, False),
    ("without https only", "--proto '=https' ", "", True, False),
    ("without TLS 1.2", "--tlsv1.2 ", "", True, False),
    ("not piped to sh", "| sh", "", True, False),
    ("long flags", "-LsSf", "--location --silent --show-error --fail", False, False),
    ("separate short flags", "-LsSf", "-L -s -S -f", False, False),
]


def fixture_problems(work: Path, fixtures: Path, home: Path, command: str) -> tuple[list[str], int]:
    """The README's command against a redirecting fixture host, then the mutation rows."""
    problems: list[str] = []
    _Release.root = fixtures / "good"
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Release)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_address[1]}"
    try:
        body = fetch_like(command, base)
        if body != INSTALLER.read_bytes():
            problems.append(f"DIST-R7: the README's command through the redirecting fixture host gives "
                            f"{len(body or b'')} bytes, not the installer: {command}")
        else:
            fetched = work / "fetched-install.sh"
            fetched.write_bytes(body)
            latest = next(c for c in CASES if c.name == "macOS arm64, latest")
            problems += [f"DIST-R7: the script fetched by the README's command: {p}"
                         for p in run_cases(work, fixtures, home, [latest], fetched)]
        # Without -L, the real host's failure: exit 0 and an empty body, which `sh` runs as
        # nothing. Anything else means the fixture does not redirect as the host does.
        without = fetch_like(command.replace("-LsSf", "-sSf"), base)
        if without != b"":
            problems.append("DIST-R7: the fixture host does not redirect latest/download like the release host "
                            f"(without -L: {'a failure' if without is None else f'{len(without)} bytes'})")
        caught = 0
        for label, old, new, rejected, empty in DOC_MUTATIONS:
            if old not in command:
                problems.append(f"DIST-R7: documented-command mutation {label!r} no longer applies: {old!r}")
                continue
            variant = command.replace(old, new, 1)
            if bool(one_liner_problems(variant)) != rejected:
                problems.append(f"DIST-R7: the rule {'accepts' if rejected else 'rejects'} {label!r}: {variant}")
                continue
            if empty and fetch_like(variant, base) != b"":
                problems.append(f"DIST-R7: {label!r} does not reproduce the silent empty fetch at the fixture host")
                continue
            caught += 1
    finally:
        server.shutdown()
        server.server_close()
    return problems, caught


def make_exec(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


# The installer runs under dash when it is installed: the strictest POSIX shell at hand,
# and the `sh` of Debian and Ubuntu, where `curl ... | sh` runs it. (bash, macOS's sh,
# also runs EXIT traps on fatal signals, which would hide a missing signal trap.)
SHELL = shutil.which("dash") or shutil.which("sh") or "/bin/sh"


def hermetic_path(where: Path, case: Case) -> Path:
    bin_dir = where / "bin"
    bin_dir.mkdir()
    tools = TOOLS + (CHECKSUM_TOOLS if case.checksum else ()) + (() if case.mv_fails else ("mv",))
    for tool in tools:
        real = SHELL if tool == "sh" else shutil.which(tool)
        if real:
            (bin_dir / tool).symlink_to(real)
    fakes = {"uname": FAKE_UNAME, "getconf": FAKE_GETCONF, "sysctl": FAKE_SYSCTL, "curl": CURL}
    if case.gh is not None:
        fakes["gh"] = FAKE_GH
    if case.mv_fails:
        fakes["mv"] = FAKE_MV_FAIL
    for name, text in fakes.items():
        make_exec(bin_dir / name, text)
    return bin_dir


def run_case(work: Path, fixtures: Path, home: Path, case: Case, installer: Path) -> list[str]:
    where = Path(tempfile.mkdtemp(dir=work))
    dest_dir, tmp_dir = where / "dest", where / "tmp"
    dest_dir.mkdir()
    tmp_dir.mkdir()
    dest = dest_dir / "sciart"
    if case.existing is not None:
        dest.write_bytes(case.existing)
        dest.chmod(0o755)
    bin_dir = hermetic_path(where, case)
    log, started = where / "curl.log", where / "curl.started"
    base = HTTPS_FIXTURE if case.base == "https" else (fixtures / case.base).as_uri()
    env = {
        "PATH": str(bin_dir), "HOME": str(home), "TMPDIR": str(tmp_dir),
        "SCIART_INSTALL_DIR": str(dest_dir), "SCIART_INSTALL_BASE_URL": base,
        "FAKE_UNAME_S": case.os, "FAKE_UNAME_M": case.machine, "FAKE_GLIBC": case.glibc,
        "FAKE_LONG_BIT": case.long_bit, "FAKE_ROSETTA": case.rosetta, "FAKE_GH": case.gh or "",
        "CURL_LOG": str(log), "CURL_STARTED": str(started), "CURL_STALL": case.interrupt,
        "REAL_CURL": shutil.which("curl") or "curl", "FIXTURE_ROOT": str(fixtures / "good"),
    }
    env.update(case.env)
    command = [str(bin_dir / "sh"), str(installer), *case.args]
    if case.interrupt:
        proc = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        deadline = time.monotonic() + 30
        while not started.exists() and proc.poll() is None and time.monotonic() < deadline:
            time.sleep(0.05)
        # The shell runs its trap when the stalled download returns; end that download too
        # (found first: once the shell is gone its children are no longer below it).
        children = _children(proc.pid)
        proc.send_signal(signal.SIGTERM)
        for child in children:
            try:
                os.kill(child, signal.SIGTERM)
            except ProcessLookupError:
                pass
        output, _ = proc.communicate(timeout=30)
        code = proc.returncode
    else:
        run = subprocess.run(command, env=env, capture_output=True, text=True, timeout=60)
        output, code = run.stdout + run.stderr, run.returncode
    problems = []
    if (code == 0) != case.ok:
        problems.append(f"exit {code}, expected {'success' if case.ok else 'a refusal'}")
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
    calls = log.read_text().splitlines() if log.exists() else []
    if case.no_request and calls:
        problems.append(f"refused only after a request: {calls[0]}")
    for call in calls:
        args = call.split()
        flags_ok = "--tlsv1.2" in args and _after(args, "--proto") == case.proto \
            and _after(args, "--proto-redir") == case.proto
        if not flags_ok:
            problems.append(f"a download without --proto {case.proto} --proto-redir {case.proto} --tlsv1.2: {call}")
            break
    leftovers = [p.name for p in dest_dir.iterdir() if p.name != "sciart"] + \
        [p.name for p in tmp_dir.iterdir()]
    if leftovers:
        problems.append(f"left files behind: {leftovers}")
    return [f"{case.name}: {p}\n    {output.strip()[-400:]}" for p in problems]


def _after(args: list[str], flag: str) -> str | None:
    return args[args.index(flag) + 1] if flag in args and args.index(flag) + 1 < len(args) else None


def _children(pid: int) -> list[int]:
    """Every descendant of `pid` (the stalled download and its `sleep`)."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid="], capture_output=True, text=True).stdout
    tree: dict[int, list[int]] = {}
    for line in out.split("\n"):
        parts = line.split()
        if len(parts) == 2:
            tree.setdefault(int(parts[1]), []).append(int(parts[0]))
    found, todo = [], [pid]
    while todo:
        for child in tree.get(todo.pop(), []):
            found.append(child)
            todo.append(child)
    return found


def shellcheck(required: bool) -> list[str]:
    tool = shutil.which("shellcheck")
    if tool is None:
        if required:
            return ["DIST-R7: shellcheck is not installed"]
        print("DIST-R7: shellcheck not installed; skipped (the release workflow requires it)")
        return []
    run = subprocess.run([tool, "--shell=sh", str(INSTALLER)], capture_output=True, text=True)
    return [] if run.returncode == 0 else [f"DIST-R7: shellcheck:\n{run.stdout}{run.stderr}"]


def run_cases(work: Path, fixtures: Path, home: Path, cases: list[Case], installer: Path) -> list[str]:
    with ThreadPoolExecutor(max_workers=8) as pool:
        results = pool.map(lambda c: run_case(work, fixtures, home, c, installer), cases)
    return [p for r in results for p in r]


def main() -> int:
    problems: list[str] = []
    required = "--require-shellcheck" in sys.argv[1:]
    script = INSTALLER.read_text()
    if not script.startswith("#!/bin/sh\n"):
        problems.append("DIST-R7: packaging/install.sh starts with #!/bin/sh (POSIX sh)")
    problems += shellcheck(required)
    readme = README.read_text()
    problems += documented_problems(readme, script)
    for label, _, names in MUTATIONS:
        unknown = sorted(set(names) - CASE_NAMES)
        if unknown or not names:
            problems.append(f"DIST-R7: mutation {label!r} names unknown cases {unknown}")
    with tempfile.TemporaryDirectory(prefix="sciart-installer-") as tmp:
        work = Path(tmp)
        fixtures = work / "fixtures"
        write_release(fixtures / "good", OLDER)
        write_release(fixtures / "good", LATEST, latest=True)
        write_release(fixtures / "tampered", LATEST, tamper="aarch64-apple-darwin", latest=True)
        home = work / "home"
        home.mkdir()
        for name in STARTUP_FILES:
            (home / name).write_text(f"# {name}\n")
        before = sorted(p.relative_to(home) for p in home.rglob("*"))
        problems += [f"DIST-R7: {p}" for p in run_cases(work, fixtures, home, CASES, INSTALLER)]
        for name in STARTUP_FILES:
            if (home / name).read_text() != f"# {name}\n":
                problems.append(f"DIST-R7: the installer edited ~/{name}")
        after = sorted(p.relative_to(home) for p in home.rglob("*"))
        if after != before:
            problems.append(f"DIST-R7: the installer wrote into the home directory: "
                            f"{sorted(set(map(str, after)) - set(map(str, before)))}")
        doc_caught = 0
        for command in documented_commands(readme)[:1]:
            found, doc_caught = fixture_problems(work, fixtures, home, command)
            problems += found
        caught = 0
        by_name = {c.name: c for c in CASES}
        for label, edits, names in MUTATIONS:
            text = script
            missing = [old for old, _ in edits if old not in text]
            if missing:
                problems.append(f"DIST-R7: mutation {label!r} no longer applies (update the table): {missing[0]!r}")
                continue
            for old, new in edits:
                text = text.replace(old, new, 1)
            broken = work / f"broken-{caught}-{abs(hash(label))}.sh"
            broken.write_text(text)
            if run_cases(work, fixtures, home, [by_name[n] for n in names], broken):
                caught += 1
            else:
                problems.append(f"DIST-R7: the installer with {label!r} passes {names}")
    help_run = subprocess.run(["/bin/sh", str(INSTALLER), "--help"], capture_output=True, text=True)
    if help_run.returncode != 0 or "--version" not in help_run.stdout:
        problems.append("DIST-R7: install.sh --help prints the usage and exits 0")
    if problems:
        print("installer:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"installer OK ({len(CASES)} cases under {Path(SHELL).name} against a fixture release; "
          f"{caught} of {len(MUTATIONS)} broken installers caught; the documented command follows the "
          f"fixture host's redirects and installs, {doc_caught} of {len(DOC_MUTATIONS)} command variants "
          "judged right)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
