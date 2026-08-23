//! IMP-14: fuzz `Theme::from_name` (RND-R2, RND-R19) on arbitrary UTF-8.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sciart_render::Theme;

fuzz_target!(|s: &str| {
    let _ = Theme::from_name(s);
    for theme in Theme::ALL {
        let _ = theme.name();
        let _ = Theme::from_name(theme.name());
    }
});
