#!/bin/sh
# shellcheck disable=SC2218
# (shellcheck 0.11.0 reports every call that precedes a command substitution in the same
# function as a use before definition; every function here is defined before main runs.)
# Install the sciart release binary (SPEC-008 DIST-R7).
#
#   curl --proto '=https' --tlsv1.2 -LsSf \
#     https://github.com/nilesh-patil/sciart/releases/latest/download/install.sh | sh
#   sh install.sh [--version X.Y.Z] [--to DIR] [--force]
#
# Picks the build for this machine (on Linux the glibc build when glibc is 2.35 or newer,
# else the static musl build; under Rosetta the native arm64 build), downloads it and the
# release's SHA256SUMS over https (TLS 1.2 or newer, https-only redirects), verifies the
# checksum, verifies the build-provenance attestation when `gh` has that command and is
# signed in, and copies the binary to $SCIART_INSTALL_DIR (default ~/.local/bin).
#
# It refuses: an OS other than macOS or Linux, a 32-bit machine or userland, a version
# that is not letters, digits, dots and hyphens, a missing checksum tool, a checksum
# mismatch, a URL that is neither https nor a local file:// mirror, running under sudo,
# and replacing a file that is not a sciart binary unless --force is given. It never edits
# shell start-up files, and removes its temporary files on failure or interrupt.
#
# SCIART_INSTALL_BASE_URL replaces https://github.com/nilesh-patil/sciart/releases (a
# mirror with the same layout: download/v<version>/<asset> and latest/download/<asset>).

set -eu

REPO="nilesh-patil/sciart"
BASE="${SCIART_INSTALL_BASE_URL:-https://github.com/$REPO/releases}"
GLIBC_FLOOR="2.35"
# Text every sciart binary contains: the start of its command-line description, which the
# 1.0.0 binaries and the later ones share.
MARKER="Real-time generative science"

say() { printf 'sciart-install: %s\n' "$*"; }
die() {
    printf 'sciart-install: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Install the sciart release binary.

Usage: install.sh [--version X.Y.Z] [--to DIR] [--force]

  --version X.Y.Z  the release to install (default: the latest)
  --to DIR         where to put the binary (default: $SCIART_INSTALL_DIR or ~/.local/bin)
  --force          replace DIR/sciart even if it is not a sciart binary
EOF
}

# The temporary directory and the staged binary, removed on any exit.
tmp=""
staged=""
cleanup() {
    if [ -n "$tmp" ]; then rm -rf "$tmp"; fi
    if [ -n "$staged" ]; then rm -f "$staged"; fi
}

# Whether $1 is a release version: letters, digits, dots and hyphens, starting with a
# digit, without `..` (it becomes part of a URL and of file names).
valid_version() {
    case "$1" in
    '' | *[!0-9A-Za-z.-]* | *..* | [!0-9]*) return 1 ;;
    esac
}

# Whether dotted version $1 is at least $2 (major.minor compared as numbers).
at_least() {
    set -- "$1.0.0" "$2.0.0"
    a1="${1%%.*}"
    rest="${1#*.}"
    a2="${rest%%.*}"
    b1="${2%%.*}"
    rest="${2#*.}"
    b2="${rest%%.*}"
    case "$a1$a2$b1$b2" in
    '' | *[!0-9]*) return 1 ;;
    esac
    [ "$a1" -gt "$b1" ] || { [ "$a1" -eq "$b1" ] && [ "$a2" -ge "$b2" ]; }
}

# The SHA-256 of file $1 (the caller has checked that one of the tools exists).
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

# Download $1 to $2, over the one protocol $proto allows, redirects included.
fetch() {
    curl --proto "$proto" --proto-redir "$proto" --tlsv1.2 -fsSL -o "$2" "$1" ||
        die "download failed: $1"
}

main() {
    version=""
    pinned=0
    dir="${SCIART_INSTALL_DIR:-}"
    force=0
    help=0
    while [ $# -gt 0 ]; do
        case "$1" in
        --version)
            [ $# -ge 2 ] || die "--version needs a value"
            version="$2"
            pinned=1
            shift 2
            ;;
        --version=*)
            version="${1#--version=}"
            pinned=1
            shift
            ;;
        --to)
            [ $# -ge 2 ] || die "--to needs a directory"
            dir="$2"
            shift 2
            ;;
        --to=*)
            dir="${1#--to=}"
            shift
            ;;
        --force)
            force=1
            shift
            ;;
        -h | --help)
            help=1
            shift
            ;;
        *) die "unknown argument: $1 (see --help)" ;;
        esac
    done
    if [ "$help" -eq 1 ]; then
        usage
        exit 0
    fi
    version="${version#v}"
    if [ "$pinned" -eq 1 ] && ! valid_version "$version"; then
        die "not a version: $version"
    fi
    if [ -z "$dir" ]; then
        [ -n "${HOME:-}" ] || die "HOME is not set; pass --to DIR"
        dir="$HOME/.local/bin"
    fi

    if [ -n "${SUDO_USER:-}" ] || [ -n "${SUDO_UID:-}" ]; then
        die "refusing to run under sudo; install into a directory you own (--to DIR)"
    fi

    case "$BASE" in
    https://*) proto="=https" ;;
    # A local mirror: nothing travels over the network.
    file://*) proto="=file" ;;
    *) die "refusing a URL that is neither https nor file://: $BASE" ;;
    esac

    # The build for this machine.
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    *) die "unsupported OS: $os (release binaries are for macOS and Linux)" ;;
    esac
    case "$arch" in
    arm64 | aarch64) arch=aarch64 ;;
    x86_64 | amd64) arch=x86_64 ;;
    *) die "unsupported machine: $arch (64-bit x86 or ARM only)" ;;
    esac
    if [ "$os" = darwin ] && [ "$arch" = x86_64 ] &&
        [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
        # A shell under Rosetta on Apple Silicon: the native build is the right one.
        arch=aarch64
    fi

    if [ "$os" = darwin ]; then
        target="$arch-apple-darwin"
    else
        # A 32-bit userland on a 64-bit kernel cannot run the 64-bit binaries.
        bits="$(getconf LONG_BIT 2>/dev/null || echo 64)"
        [ "$bits" = 64 ] || die "a ${bits}-bit userland (64-bit binaries only)"
        glibc="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"
        case "$glibc" in
        "glibc "*) glibc="${glibc#glibc }" ;;
        *) glibc="" ;;
        esac
        if [ -n "$glibc" ] && at_least "$glibc" "$GLIBC_FLOOR"; then
            target="$arch-unknown-linux-gnu"
        else
            target="$arch-unknown-linux-musl"
            say "glibc ${glibc:-not found} (the glibc build needs $GLIBC_FLOOR or newer): using the static build"
        fi
    fi

    # The tools, before anything is downloaded.
    if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
        die "needs sha256sum or shasum to verify the download"
    fi
    command -v curl >/dev/null 2>&1 || die "needs curl"

    trap cleanup EXIT
    trap 'exit 1' INT TERM HUP
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/sciart-install.XXXXXX")"

    if [ "$pinned" -eq 0 ]; then
        fetch "$BASE/latest/download/SHA256SUMS" "$tmp/SHA256SUMS"
        version="$(sed -n "s/^[0-9a-f]\{64\}  sciart-\(.*\)-$target\.tar\.gz\$/\1/p" "$tmp/SHA256SUMS" | head -n 1)"
        [ -n "$version" ] || die "the latest release has no build for $target"
        valid_version "$version" || die "the latest release names an invalid version: $version"
    else
        fetch "$BASE/download/v$version/SHA256SUMS" "$tmp/SHA256SUMS"
    fi

    asset="sciart-$version-$target.tar.gz"
    pattern="$(printf '%s' "$asset" | sed 's/[.]/\\&/g')"
    want="$(sed -n "s/^\([0-9a-f]\{64\}\)  $pattern\$/\1/p" "$tmp/SHA256SUMS" | head -n 1)"
    [ -n "$want" ] || die "SHA256SUMS of $version lists no $asset"
    fetch "$BASE/download/v$version/$asset" "$tmp/$asset"
    got="$(sha256 "$tmp/$asset")"
    [ "$got" = "$want" ] || die "checksum mismatch for $asset: expected $want, got $got"
    say "checksum verified: $asset"

    if ! command -v gh >/dev/null 2>&1; then
        say "gh is not installed: provenance attestation not checked (the checksum was)"
    elif ! gh attestation --help >/dev/null 2>&1; then
        say "this gh has no attestation command: provenance attestation not checked (the checksum was)"
    elif ! gh auth status >/dev/null 2>&1; then
        say "gh is not signed in: provenance attestation not checked (the checksum was)"
    else
        gh attestation verify "$tmp/$asset" --repo "$REPO" >/dev/null 2>&1 ||
            die "the build-provenance attestation of $asset does not verify"
        say "provenance attestation verified"
    fi

    tar -xzf "$tmp/$asset" -C "$tmp"
    binary="$tmp/sciart-$version-$target/sciart"
    [ -f "$binary" ] || die "$asset holds no sciart binary"

    dest="$dir/sciart"
    [ ! -d "$dest" ] || die "$dest is a directory"
    if [ -e "$dest" ] && [ "$force" -ne 1 ] && ! LC_ALL=C grep -q -F "$MARKER" "$dest" 2>/dev/null; then
        die "$dest exists and is not a sciart binary; pass --force to replace it"
    fi
    mkdir -p "$dir"
    staged="$dir/.sciart.install.$$"
    cp "$binary" "$staged"
    chmod 755 "$staged"
    mv -f "$staged" "$dest"
    staged=""

    say "installed sciart $version ($target) as $dest"
    case ":${PATH:-}:" in
    *":$dir:"*) ;;
    *) say "$dir is not on your PATH; add it in your shell's start-up file" ;;
    esac
    say "shell completions: sciart completions <bash|zsh|fish>; manual: sciart man"
}

main "$@"
