//! IMP-14: fuzz the title-fitting path (CORE-R71..R76, RND-R9).
//!
//! `Title` holds `&'static str` by contract — a title is a slice of `Meta.title`, never a
//! runtime string — so instead of leaking an owned title per case the fuzzer picks a
//! varied set of static titles (empty, one-part, separator, caption, accented, CJK) with
//! the input and fuzzes the canvas size. Arbitrary runtime text reaches the same fitting
//! logic through `Hud::fit`'s borrowed `generator`/`keep_awake` fields (hud_fit).
#![no_main]

use libfuzzer_sys::fuzz_target;
use sciart_core::title::{self, Form, Title};
use sciart_core::{Category, Meta, Size};

const TITLES: [(&str, Option<&str>); 7] = [
    ("", None),
    ("Starfield", None),
    ("Parallax · near stars rush by", None),
    (
        "Aurora · charged particles excite the atmosphere",
        Some("solar wind couples to the magnetosphere"),
    ),
    ("Café · naïve", None),
    ("Deep Cosmos · 深宇宙", None),
    ("Separator · ", None),
];

static META: Meta = Meta {
    name: "fuzz",
    category: Category::Physics,
    description: "fuzz fixture",
    title: "Phenomenon · mechanism",
    captions: &["first act", "a much longer second act for the same title"],
};

fuzz_target!(|s: &str| {
    // CORE-R72: the title charset mapping must be total over arbitrary chars.
    for c in s.chars() {
        let _ = title::ascii_of(c);
    }

    let bytes = s.as_bytes();
    let pick = bytes
        .iter()
        .fold(0usize, |a, &b| a.wrapping_add(usize::from(b)))
        % TITLES.len();
    let (raw, caption) = TITLES[pick];
    let title = Title::new(raw, caption);

    let sizes = [
        Size::new(0, 0),
        Size::new(1, 1),
        Size::new(u16::MAX, u16::MAX),
        Size::new(
            1 + u16::from(bytes.first().copied().unwrap_or(0)) % 200,
            1 + u16::from(bytes.get(1).copied().unwrap_or(0)) % 60,
        ),
    ];
    for size in sizes {
        let _ = title.fit(size);
        let _ = title.width(Form::Full);
        let _ = title.width(Form::Phenomenon);
        let _ = title.chars(Form::Full).count();
        let _ = title.full_fits_from();
        let _ = title::span(size, &title);
        let _ = title::readout_room(size, &title);
        let _ = title::stable_readout_room(size, &META);
    }
});
