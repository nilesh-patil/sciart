//! Golden frames per generator (SPEC-006, TEST-R2). Review changes with `cargo insta review`.
//! Seed 42, 80x24, ASCII mode, after 0 s / 1 s / 5 s of simulated time. Pinned on the
//! reference platform (aarch64-apple-darwin) only (TEST-R13, ADR-0029).

#[path = "common/tier.rs"]
mod tier;

use sciart_core::prelude::*;
use sciart_generators::registry;

#[test]
fn snapshots_seed42_80x24_ascii() {
    if tier::skip_off_reference_platform("snapshots") {
        return;
    }
    for entry in registry() {
        let mut stage = Stage::new((entry.build)(), 42, Size::new(80, 24), GlyphMode::Ascii);
        let mut elapsed = 0;
        for secs in [0, 1, 5] {
            while elapsed < secs * 60 {
                stage.step(SIM_DT);
                elapsed += 1;
            }
            let frame = stage.render().to_lines().join("\n");
            insta::assert_snapshot!(format!("{}_t{secs}", entry.meta.name), frame);
        }
    }
}

/// TEST-R13: frames are pinned on exactly one platform, Apple Silicon macOS.
#[test]
fn test_r13_frames_are_pinned_on_apple_silicon_macos_only() {
    let expected = matches!(
        (std::env::consts::OS, std::env::consts::ARCH),
        ("macos", "aarch64")
    );
    assert_eq!(tier::FRAMES_PINNED, expected);
    assert_eq!(tier::skip_off_reference_platform("snapshots"), !expected);
}
