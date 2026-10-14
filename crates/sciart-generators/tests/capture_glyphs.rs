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

/// The sampled times above miss glyphs a scene shows only later (an act, a label that comes
/// and goes) or picks from a table, so every non-ASCII character in a string or character
/// literal of the scene and shared sources is checked too (comments and test modules are
/// not code a scene draws).
#[test]
fn cli_r18_capture_draws_every_literal_glyph_in_the_scene_sources() {
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
            for ch in literal_chars(&text) {
                if ch.is_ascii() || ch.width() != Some(1) || draws_natively(ch) {
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
        "CLI-R18: the capture has no native drawing for {} literal glyph(s):\n  {}",
        report.len(),
        report.join("\n  ")
    );
}

#[test]
fn cli_r18_literal_scan_reads_strings_chars_and_escapes_but_not_comments_or_tests() {
    let src = r##"
        // a comment with ★ is not code
        /* nor ☉ in /* a nested */ block */
        const A: Glyph = Glyph::new('●', 'O');
        const B: &str = "slowed 10\u{2075} ⁶";
        const C: &[char] = &['▏', '\u{258E}', '{'];
        const D: &str = r#"raw ♃ "quoted""#;
        fn f<'a>(x: &'a str) -> &'a str { let _ = '\''; x }
        #[cfg(test)]
        mod tests {
            const T: &str = "✸ only in a test";
            fn g() { let _ = "{"; }
        }
        const E: char = '⛢';
    "##;
    let found: String = literal_chars(src)
        .into_iter()
        .filter(|c| !c.is_ascii())
        .collect();
    assert_eq!(found, "●\u{2075}⁶▏\u{258E}♃⛢");
}

/// Every character inside a string, raw-string or character literal of `src`, escapes
/// decoded, outside comments and outside items marked `#[cfg(test)]`.
fn literal_chars(src: &str) -> Vec<char> {
    let s: Vec<char> = src.chars().collect();
    let at = |i: usize| s.get(i).copied();
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = Vec::new();
    let mut keep = |c: char, skipping: bool| {
        if !skipping {
            out.push(c);
        }
    };
    let (mut i, mut depth) = (0, 0usize);
    // The brace depth a `#[cfg(test)]` item opened at, while its body is being skipped.
    let mut test_item: Option<usize> = None;
    let mut test_pending = false;
    let cfg_test: Vec<char> = "#[cfg(test)]".chars().collect();
    while i < s.len() {
        let skipping = test_item.is_some();
        let c = s[i];
        if c == '/' && at(i + 1) == Some('/') {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == Some('*') {
            let mut nest = 0;
            while i < s.len() {
                if s[i] == '/' && at(i + 1) == Some('*') {
                    nest += 1;
                    i += 2;
                } else if s[i] == '*' && at(i + 1) == Some('/') {
                    nest -= 1;
                    i += 2;
                    if nest == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if s[i..].starts_with(&cfg_test) {
            test_pending = true;
            i += cfg_test.len();
            continue;
        }
        // Raw strings: r"…", r#"…"#, br"…" (not an identifier ending in r).
        if c == 'r' && !ident(i.checked_sub(1).and_then(at).filter(|&p| p != 'b')) {
            let mut j = i + 1;
            while at(j) == Some('#') {
                j += 1;
            }
            if at(j) == Some('"') {
                let hashes = j - i - 1;
                j += 1;
                while j < s.len() {
                    if s[j] == '"' && (1..=hashes).all(|k| at(j + k) == Some('#')) {
                        break;
                    }
                    keep(s[j], skipping);
                    j += 1;
                }
                i = j + 1 + hashes;
                continue;
            }
        }
        match c {
            '"' => {
                i += 1;
                while i < s.len() && s[i] != '"' {
                    if s[i] == '\\' {
                        let (decoded, next) = escape(&s, i);
                        if let Some(d) = decoded {
                            keep(d, skipping);
                        }
                        i = next;
                    } else {
                        keep(s[i], skipping);
                        i += 1;
                    }
                }
            }
            '\'' if at(i + 1) == Some('\\') => {
                let (decoded, next) = escape(&s, i + 1);
                if let Some(d) = decoded {
                    keep(d, skipping);
                }
                i = next; // the closing quote
            }
            '\'' if at(i + 2) == Some('\'') => {
                keep(s[i + 1], skipping);
                i += 2; // the closing quote
            }
            '{' => {
                if test_pending && test_item.is_none() {
                    test_item = Some(depth);
                }
                test_pending = false;
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if test_item == Some(depth) {
                    test_item = None;
                }
            }
            ';' => test_pending = false,
            _ => {}
        }
        i += 1;
    }
    out
}

/// Decode the escape starting at the backslash `s[i]`: the character it stands for (none
/// for a line continuation) and the index just after it.
fn escape(s: &[char], i: usize) -> (Option<char>, usize) {
    match s.get(i + 1) {
        Some('u') if s.get(i + 2) == Some(&'{') => {
            let end = (i + 3..s.len())
                .find(|&j| s[j] == '}')
                .unwrap_or(s.len() - 1);
            let hex: String = s[i + 3..end].iter().filter(|&&c| c != '_').collect();
            let decoded = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
            (decoded, end + 1)
        }
        Some('x') => (None, i + 4),
        Some('\n') => {
            let mut j = i + 2;
            while s.get(j).is_some_and(|c| c.is_whitespace()) {
                j += 1;
            }
            (None, j)
        }
        Some(&c) => (Some(c), i + 2),
        None => (None, i + 1),
    }
}
