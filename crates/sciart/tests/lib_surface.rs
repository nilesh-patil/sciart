//! LIB-R1, LIB-R2, LIB-R3, LIB-R6 and LIB-R8: the library's public surface, read from its
//! sources.
//!
//! The facade is small and hand-written, so its public items can be listed from the source
//! text: every `pub` item at the top level of a facade file, every `pub` method of an
//! inherent `impl`, every enum variant and every trait `impl`, each with its attributes and
//! the feature it needs. The list is a reviewed snapshot (LIB-R8): a change to it is a
//! change to the promise and needs a person to look at the diff, as a scene snapshot does
//! (TEST-R2). Record a reviewed change with
//! `INSTA_UPDATE=always cargo test -p sciart --test lib_surface`.

use std::path::{Path, PathBuf};

/// A facade source file and the feature it compiles under.
struct Facade {
    name: String,
    feature: Option<String>,
    code: String,
}

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// `#[cfg(feature = "x")]` → `x`.
fn feature_of(attr: &str) -> Option<String> {
    let rest = attr.trim().strip_prefix("#[cfg(feature = \"")?;
    Some(rest.split('"').next()?.to_owned())
}

/// The source without its unit tests (they sit at the end of each file).
fn without_tests(text: &str) -> String {
    [
        "#[cfg(test)]\nmod tests",
        "#[cfg(test)]\npub(crate) mod tests",
    ]
    .iter()
    .filter_map(|marker| text.find(marker))
    .min()
    .map_or_else(|| text.to_owned(), |i| text[..i].to_owned())
}

/// `lib.rs` and every module it declares, with the module's feature gate.
fn facade() -> Vec<Facade> {
    let lib = std::fs::read_to_string(src().join("lib.rs")).expect("read lib.rs");
    let mut files = vec![Facade {
        name: "lib.rs".to_owned(),
        feature: None,
        code: without_tests(&lib),
    }];
    let mut gate = None;
    for line in lib.lines() {
        let line = line.trim();
        if line.starts_with("#[cfg(feature") {
            gate = feature_of(line);
            continue;
        }
        let module = line
            .strip_prefix("pub mod ")
            .or_else(|| line.strip_prefix("mod "))
            .and_then(|m| m.strip_suffix(';'));
        if let Some(module) = module {
            let name = format!("{module}.rs");
            let text = std::fs::read_to_string(src().join(&name)).expect("read a facade module");
            files.push(Facade {
                name,
                feature: gate.take(),
                code: without_tests(&text),
            });
        } else if !line.starts_with("#[") {
            gate = None;
        }
    }
    files
}

/// Lines of code with comments and doc comments removed.
fn code_lines(code: &str) -> impl Iterator<Item = &str> {
    code.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .filter(|l| !l.trim().is_empty())
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// One normalised signature from `lines[start..]`: up to the opening brace or semicolon.
fn signature(lines: &[&str], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i].trim();
        text.push(' ');
        text.push_str(line);
        if line.ends_with('{') || line.ends_with(';') || line.ends_with('}') {
            break;
        }
        i += 1;
    }
    let mut sig = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [("( ", "("), (", )", ")"), (",)", ")")] {
        sig = sig.replace(from, to);
    }
    let sig = sig
        .trim_end_matches("{}")
        .trim_end_matches('{')
        .trim_end_matches(';')
        .trim()
        .to_owned();
    (sig, i)
}

/// The public surface, one line per item.
fn surface(files: &[Facade]) -> Vec<String> {
    let mut out = Vec::new();
    for file in files {
        let lines: Vec<&str> = code_lines(&file.code).collect();
        let file_gate = file.feature.as_deref();
        let mut attrs: Vec<String> = Vec::new();
        // The enclosing block at indent 0: an inherent impl, a public enum, or neither.
        let mut block: Option<(String, bool)> = None; // (owner, is_enum)
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();
            let depth = indent(line);
            if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
                if !trimmed.starts_with("#![") && !trimmed.starts_with("#[allow") {
                    attrs.push(trimmed.to_owned());
                }
                i += 1;
                continue;
            }
            let gate = attrs
                .iter()
                .find_map(|a| feature_of(a))
                .or_else(|| file_gate.map(str::to_owned));
            let shown: Vec<_> = attrs
                .iter()
                .filter(|a| !a.starts_with("#[cfg(") && !a.starts_with("#[must_use"))
                .cloned()
                .collect();
            let label = |item: &str| {
                let gate = gate.as_ref().map(|g| format!("[{g}] ")).unwrap_or_default();
                let attrs = if shown.is_empty() {
                    String::new()
                } else {
                    format!("{} ", shown.join(" "))
                };
                format!("{}: {gate}{attrs}{item}", file.name)
            };
            if depth == 0 {
                if trimmed == "}" {
                    block = None;
                } else if trimmed.starts_with("pub ") {
                    let (sig, end) = signature(&lines, i);
                    let is_enum = sig.starts_with("pub enum ");
                    if (is_enum || sig.starts_with("pub struct ")) && trimmed.ends_with('{') {
                        let name = sig.split_whitespace().nth(2).unwrap_or("").to_owned();
                        block = Some((name, is_enum));
                    }
                    out.push(label(&sig));
                    i = end;
                } else if trimmed.starts_with("impl") {
                    let (sig, end) = signature(&lines, i);
                    if sig.contains(" for ") {
                        out.push(label(&sig));
                        block = None;
                    } else {
                        block = Some((sig, false));
                    }
                    i = end;
                }
                attrs.clear();
            } else if depth == 4 {
                match &block {
                    Some((owner, true)) => {
                        let variant = trimmed.trim_end_matches(',');
                        out.push(label(&format!("{owner}::{variant}")));
                    }
                    Some((owner, false)) if trimmed.starts_with("pub ") => {
                        let (sig, end) = signature(&lines, i);
                        out.push(label(&format!("{owner}: {sig}")));
                        i = end;
                    }
                    _ => {}
                }
                attrs.clear();
            }
            i += 1;
        }
    }
    out
}

#[test]
fn lib_r1_r8_the_public_surface_is_the_reviewed_one() {
    insta::assert_snapshot!("public_api", surface(&facade()).join("\n"));
}

#[test]
fn lib_r1_exports_exactly_the_promised_names() {
    let files = facade();
    let lib = &files[0].code;
    let mut exported: Vec<&str> = Vec::new();
    for line in code_lines(lib) {
        if let Some(list) = line.trim().strip_prefix("pub use ") {
            let list = list.trim_end_matches(';');
            let names = match list.split_once('{') {
                Some((_, names)) => names.trim_end_matches('}').split(',').collect::<Vec<_>>(),
                None => vec![list.rsplit("::").next().unwrap_or(list)],
            };
            exported.extend(names.into_iter().map(str::trim).filter(|n| !n.is_empty()));
        }
    }
    exported.sort_unstable();
    assert_eq!(
        exported,
        [
            "Category",
            "Error",
            "Options",
            "SceneInfo",
            "SceneWidget",
            "Stage",
            "Theme",
            "find",
            "scenes"
        ]
    );
    let modules: Vec<_> = code_lines(lib)
        .filter(|l| l.trim().starts_with("pub mod "))
        .map(str::trim)
        .collect();
    assert_eq!(modules, ["pub mod experimental;"]);
    let experimental = files.iter().find(|f| f.name == "experimental.rs").unwrap();
    assert_eq!(experimental.feature.as_deref(), Some("unstable-generator"));
    let widget = files.iter().find(|f| f.name == "widget.rs").unwrap();
    assert_eq!(widget.feature.as_deref(), Some("widget"));
}

#[test]
fn lib_r1_nothing_internal_is_re_exported_outside_experimental() {
    for file in facade().iter().filter(|f| f.name != "experimental.rs") {
        for line in code_lines(&file.code) {
            assert!(
                !line.trim().starts_with("pub use sciart_"),
                "{}: {line}",
                file.name
            );
        }
    }
}

#[test]
fn lib_r2_no_dependency_types_outside_the_widget() {
    const BANNED: [&str; 5] = ["rand::", "rand_chacha", "ratatui", "crossterm", "clap"];
    for file in facade().iter().filter(|f| f.name != "widget.rs") {
        for line in code_lines(&file.code) {
            for banned in BANNED {
                assert!(
                    !line.contains(banned),
                    "{}: `{banned}` in {line}",
                    file.name
                );
            }
        }
    }
}

#[test]
fn lib_r3_enums_are_non_exhaustive_and_structs_opaque() {
    let mut enums = 0;
    let mut structs = 0;
    for file in facade() {
        let lines: Vec<&str> = code_lines(&file.code).collect();
        for (i, line) in lines.iter().enumerate() {
            let attrs: Vec<&str> = lines[..i]
                .iter()
                .rev()
                .take_while(|l| l.trim().starts_with("#["))
                .map(|l| l.trim())
                .collect();
            if line.starts_with("pub enum ") {
                enums += 1;
                assert!(
                    attrs.contains(&"#[non_exhaustive]"),
                    "{}: {line}",
                    file.name
                );
            }
            if line.starts_with("pub struct ") {
                structs += 1;
                assert!(!line.contains("(pub "), "{}: {line}", file.name);
                if line.ends_with('{') {
                    for field in lines[i + 1..].iter().take_while(|l| l.trim() != "}") {
                        assert!(
                            !field.trim().starts_with("pub "),
                            "{}: public field {field}",
                            file.name
                        );
                    }
                }
            }
            assert!(
                !line.trim_start().starts_with("pub trait "),
                "{}: {line}",
                file.name
            );
        }
    }
    assert!(
        enums >= 3 && structs >= 4,
        "the scan found the facade's types"
    );
}

#[test]
fn lib_r6_the_library_never_touches_the_terminal_or_the_panic_hook() {
    const BANNED: [&str; 6] = [
        "set_hook",
        "take_hook",
        "enable_raw_mode",
        "EnterAlternateScreen",
        "?1049",
        "TerminalGuard",
    ];
    for file in facade() {
        for line in code_lines(&file.code) {
            for banned in BANNED {
                assert!(
                    !line.contains(banned),
                    "{}: `{banned}` in {line}",
                    file.name
                );
            }
        }
    }
}

#[test]
fn lib_r6_every_failing_entry_point_documents_the_abort_caveat() {
    let lib = std::fs::read_to_string(src().join("lib.rs")).unwrap();
    let stage = std::fs::read_to_string(src().join("stage.rs")).unwrap();
    for (name, text) in [("lib.rs", &lib), ("stage.rs", &stage)] {
        assert!(text.contains("# Panics"), "{name}");
        assert!(text.contains("panic = \"abort\""), "{name}");
    }
}
