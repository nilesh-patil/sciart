//! IMP-14: fuzz the HUD priority-fitting ladder (RND-R4, RND-R9) on arbitrary UTF-8.
//!
//! `Hud` borrows its name and keep-awake status, so this is the target that feeds
//! arbitrary runtime text through the fitting code (title fitting itself only sees static
//! `Meta.title` slices; see title_fit).
#![no_main]

use libfuzzer_sys::fuzz_target;
use sciart_core::title::Title;
use sciart_render::{Hud, Theme};

fuzz_target!(|s: &str| {
    let b = s.as_bytes();
    let mut seed = [0u8; 8];
    for (dst, src) in seed.iter_mut().zip(b.iter().cycle()) {
        *dst = *src;
    }
    let theme = Theme::ALL[usize::from(b.first().copied().unwrap_or(0)) % Theme::ALL.len()];
    let widths = [
        0,
        1,
        s.chars().count(),
        usize::MAX,
        usize::from(b.get(6).copied().unwrap_or(40)) * 4,
    ];
    for title in [
        None,
        Some(Title::new("Parallax · near stars rush by", None)),
        Some(Title::new("Deep Cosmos · a live caption", Some("act two"))),
    ] {
        let hud = Hud {
            generator: s,
            title,
            fps: f64::from(b.get(2).copied().unwrap_or(30)),
            theme,
            seed: u64::from_le_bytes(seed),
            keep_awake: s,
            low_power: b.get(3).is_some_and(|x| x & 1 == 1),
            paused: b.get(4).is_some_and(|x| x & 1 == 1),
            quarantined: usize::from(b.get(5).copied().unwrap_or(0)),
        };
        for width in widths {
            let _ = hud.fit(width);
            let mut out = String::new();
            hud.fit_into(width, &mut out);
        }
        let _ = hud.line();
    }

    // Non-finite measurements and a saturated quarantine count must also format.
    let hud = Hud {
        generator: s,
        title: None,
        fps: f64::NAN,
        theme,
        seed: u64::MAX,
        keep_awake: s,
        low_power: true,
        paused: true,
        quarantined: usize::MAX,
    };
    let _ = hud.fit(80);
    let _ = hud.line();
});
