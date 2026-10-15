# Sourced by pixi for every task and `pixi shell` (pixi.toml [activation]). POSIX sh.
#
# TEST-R11 / ADR-0023: route rustc through sccache so a new checkout or workspace, which
# starts with an empty target/, reuses crates compiled by any other tree on this machine.
# - Skipped when sccache is not on PATH (the build is the same, only slower).
# - An explicit RUSTC_WRAPPER wins; `RUSTC_WRAPPER= pixi run ...` (empty) turns it off.
# - Incremental builds stay on for dev/test (focused loops rebuild only what changed).
#   sccache cannot cache an incremental compile, so it passes workspace crates straight
#   to rustc there and caches the dependencies (never incremental) and every `ci`/`release`
#   build (non-incremental).
if [ -z "${RUSTC_WRAPPER+set}" ] && command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
fi
