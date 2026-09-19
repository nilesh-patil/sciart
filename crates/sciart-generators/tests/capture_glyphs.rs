//! CLI-R18: the capture renderer (`sciart record`, the preview example, the gallery) draws
//! every glyph the registered scenes emit as that glyph, never as its ASCII stand-in.

use std::collections::BTreeMap;

use sciart_capture::draws_natively;
use sciart_core::prelude::*;
use sciart_generators::registry;
use unicode_width::UnicodeWidthChar;

#[test]
fn cli_r18_capture_draws_every_scene_glyph_natively() {
    // glyph -> the scenes that emit it, for a useful failure message.
    let mut missing: BTreeMap<char, Vec<&str>> = BTreeMap::new();
    for entry in registry() {
        let mut stage = Stage::new((entry.build)(), 42, Size::new(80, 24), GlyphMode::Unicode);
        let mut elapsed = 0;
        for secs in [0, 5, 20] {
            while elapsed < secs * 60 {
                stage.step(SIM_DT);
                elapsed += 1;
            }
            for cell in stage.render().cells() {
                let ch = cell.ch;
                // Glyphs wider than one cell keep the core's ASCII fallback (GEN-R10).
                if ch.width() != Some(1) || draws_natively(ch) {
                    continue;
                }
                let scenes = missing.entry(ch).or_default();
                if !scenes.contains(&entry.meta.name) {
                    scenes.push(entry.meta.name);
                }
            }
        }
    }
    let report: Vec<String> = missing
        .iter()
        .map(|(ch, scenes)| format!("{ch} U+{:04X} in {}", u32::from(*ch), scenes.join(", ")))
        .collect();
    assert!(
        report.is_empty(),
        "CLI-R18: the capture has no native drawing for {} glyph(s):\n  {}",
        report.len(),
        report.join("\n  ")
    );
}

/// The sampled times above can miss a glyph a scene shows only later (an act, a rare event),
/// so every single-width `Glyph::new('…', …)` in the scene and shared sources is checked too.
#[test]
fn cli_r18_capture_draws_every_declared_scene_glyph_natively() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut missing: BTreeMap<char, Vec<String>> = BTreeMap::new();
    let mut files = 0;
    for dir in std::fs::read_dir(&crates).expect("crates/ is readable") {
        let dir = dir.expect("a crates/ entry").path();
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with("sciart-scenes-") || name == "sciart-shared") {
            continue;
        }
        for file in std::fs::read_dir(dir.join("src")).expect("src/ is readable") {
            let path = file.expect("a src/ entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).expect("a readable source file");
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            for (i, _) in text.match_indices("Glyph::new(") {
                let rest = text[i + "Glyph::new(".len()..].trim_start();
                let mut chars = rest.chars();
                let (Some('\''), Some(ch), Some('\'')) = (chars.next(), chars.next(), chars.next())
                else {
                    continue;
                };
                if ch.width() != Some(1) || draws_natively(ch) {
                    continue;
                }
                let scenes = missing.entry(ch).or_default();
                if !scenes.iter().any(|s| s == stem) {
                    scenes.push(stem.to_owned());
                }
            }
        }
    }
    assert!(
        files > 80,
        "found only {files} scene sources under {crates:?}"
    );
    let report: Vec<String> = missing
        .iter()
        .map(|(ch, files)| format!("{ch} U+{:04X} in {}", u32::from(*ch), files.join(", ")))
        .collect();
    assert!(
        report.is_empty(),
        "CLI-R18: the capture has no native drawing for {} declared glyph(s):\n  {}",
        report.len(),
        report.join("\n  ")
    );
}
