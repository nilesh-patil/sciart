//! Platform tiers (SPEC-006 TEST-R13, ADR-0029). Frames are deterministic per platform,
//! not across platforms: the last bit of a libm call can differ between glibc, musl and
//! Apple's libm on each architecture. So the suites that pin frames (the snapshot suite and
//! `m11_fidelity`) run on tier 1 only; the same-seed determinism tests run everywhere.

/// Tier 1: `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`, where frames are pinned.
pub const FRAMES_PINNED: bool = cfg!(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    ),
));

/// `true` (after saying why on stderr) when a frame-pinning test must skip itself because
/// this target is outside tier 1 (TEST-R13).
pub fn skip_off_tier_1(suite: &str) -> bool {
    if FRAMES_PINNED {
        return false;
    }
    eprintln!(
        "{suite}: skipped on {}-{}: frames are pinned on tier 1 only \
         (aarch64-apple-darwin, x86_64-unknown-linux-gnu; TEST-R13, ADR-0029)",
        std::env::consts::ARCH,
        std::env::consts::OS,
    );
    true
}
