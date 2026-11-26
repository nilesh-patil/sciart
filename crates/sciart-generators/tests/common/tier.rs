//! The reference platform (SPEC-006 TEST-R13, ADR-0029). Frames are deterministic per
//! platform, not across platforms: the last bit of a libm call can differ between glibc,
//! musl and Apple's libm on each architecture. So the suites that pin frames (the snapshot
//! suite and `m11_fidelity`) run on the reference platform, where every look was reviewed,
//! and skip themselves elsewhere; the same-seed determinism tests run everywhere.

/// The reference platform, `aarch64-apple-darwin`: the only target where frames are pinned.
pub const FRAMES_PINNED: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

/// `true` (after saying why on stderr) when a frame-pinning test must skip itself because
/// this target is not the reference platform (TEST-R13).
pub fn skip_off_reference_platform(suite: &str) -> bool {
    if FRAMES_PINNED {
        return false;
    }
    eprintln!(
        "{suite}: skipped on {}-{}: frames are pinned on the reference platform only \
         (aarch64-apple-darwin; TEST-R13, ADR-0029)",
        std::env::consts::ARCH,
        std::env::consts::OS,
    );
    true
}
