//! LIB-R1, LIB-R2, LIB-R3, LIB-R6 and LIB-R8: the library's public surface, read from its
//! sources.
//!
//! The scan starts at `lib.rs` and follows every module declaration into its file (a
//! `#[path]` attribute, a nested directory or `mod.rs`), and inline `mod x { .. }` blocks,
//! tracking braces rather than indentation. It lists every `pub` item it meets, wherever it
//! is: top-level items, inherent methods, enum variants, public fields, trait impls and any
//! `pub` item nested in a body, each with its attributes and the feature it needs. The list
//! is a reviewed snapshot (LIB-R8): a change to it is a change to the promise and needs a
//! person to look at the diff, as a scene snapshot does (TEST-R2). Record a reviewed change
//! with `INSTA_UPDATE=always cargo test -p sciart --test lib_surface`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// `#[cfg(feature = "x")]` → `x`.
fn feature_of(attr: &str) -> Option<String> {
    let rest = attr.trim().strip_prefix("#[cfg(feature = \"")?;
    Some(rest.split('"').next()?.to_owned())
}

/// `#[path = "p"]` → `p`.
fn path_of(attr: &str) -> Option<String> {
    let rest = attr.trim().strip_prefix("#[path = \"")?;
    Some(rest.split('"').next()?.to_owned())
}

/// The source without its unit-test modules.
fn without_tests(text: &str) -> String {
    let mut out = String::new();
    let mut skipping: Option<i64> = None;
    let mut pending_test = false;
    for line in text.lines() {
        if let Some(depth) = skipping.as_mut() {
            *depth += braces(&scrub(line));
            if *depth <= 0 {
                skipping = None;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            pending_test = true;
            continue;
        }
        if pending_test && trimmed.contains("mod tests") && trimmed.ends_with('{') {
            pending_test = false;
            skipping = Some(1);
            continue;
        }
        if pending_test {
            out.push_str("#[cfg(test)]\n");
            pending_test = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A line with comments removed and string and char literals emptied, for counting braces.
fn scrub(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => break,
            '"' => {
                out.push('"');
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
                out.push('"');
            }
            '\'' => {
                // A char literal ('x', '\n', '{'), or a lifetime ('a), which has no closing quote.
                let rest: String = chars.clone().take(12).collect();
                let close = if let Some(escaped) = rest.strip_prefix('\\') {
                    escaped.find('\'').map(|i| i + 2)
                } else {
                    rest.char_indices()
                        .nth(1)
                        .filter(|&(_, c)| c == '\'')
                        .map(|_| 2)
                };
                if let Some(n) = close {
                    for _ in 0..n {
                        chars.next();
                    }
                    out.push_str("' '");
                } else {
                    out.push('\'');
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn braces(scrubbed: &str) -> i64 {
    scrubbed.chars().fold(0, |n, c| match c {
        '{' => n + 1,
        '}' => n - 1,
        _ => n,
    })
}

/// A facade source file and the feature it compiles under.
#[derive(Debug)]
struct Facade {
    /// Path relative to `src/`.
    name: String,
    feature: Option<String>,
    code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Use,
    Mod,
    Fn,
    Enum,
    Struct,
    Trait,
    Other,
    Method,
    Variant,
    Field,
    TraitImpl,
    Nested,
}

#[derive(Debug)]
struct Item {
    file: String,
    gate: Option<String>,
    attrs: Vec<String>,
    kind: Kind,
    /// `impl X`, `Enum`, `Struct` or a module path the item sits in.
    owner: String,
    sig: String,
}

impl Item {
    fn line(&self) -> String {
        let gate = self
            .gate
            .as_ref()
            .map(|g| format!("[{g}] "))
            .unwrap_or_default();
        let attrs: Vec<_> = self
            .attrs
            .iter()
            .filter(|a| {
                !a.starts_with("#[cfg(") && !a.starts_with("#[must_use") && !a.starts_with("#[path")
            })
            .cloned()
            .collect();
        let attrs = if attrs.is_empty() {
            String::new()
        } else {
            format!("{} ", attrs.join(" "))
        };
        let owner = match self.kind {
            Kind::Method | Kind::Field => format!("{}: ", self.owner),
            Kind::Variant => format!("{}::", self.owner),
            Kind::Nested => format!("(nested in {}) ", self.owner),
            _ if !self.owner.is_empty() => format!("mod {}: ", self.owner),
            _ => String::new(),
        };
        format!("{}: {gate}{attrs}{owner}{}", self.file, self.sig)
    }
}

/// What an open brace belongs to.
#[derive(Debug, Clone)]
enum Block {
    /// A module (the file itself or an inline `mod x {}`), with its path and gate.
    Module(Vec<String>, Option<String>),
    Impl(String, Option<String>),
    /// An enum or struct body: name, gate, and whether the type is `pub`.
    Enum(String, Option<String>, bool),
    Struct(String, Option<String>, bool),
    Trait,
    Body(String),
}

/// A `mod x;` declaration: the inline module path it sits in, its name and attributes.
struct ModDecl {
    inline: Vec<String>,
    name: String,
    path: Option<String>,
    gate: Option<String>,
}

fn normalise(text: &str) -> String {
    let mut sig = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [
        ("( ", "("),
        (", )", ")"),
        (",)", ")"),
        ("< ", "<"),
        (" >", ">"),
    ] {
        sig = sig.replace(from, to);
    }
    let sig = sig.trim_end_matches("{}").trim();
    let sig = sig.strip_suffix('{').unwrap_or(sig);
    sig.trim_end_matches(';')
        .trim_end_matches(',')
        .trim()
        .to_owned()
}

/// Every `pub` item and module declaration in one file.
fn scan(file: &Facade) -> (Vec<Item>, Vec<ModDecl>) {
    let mut items = Vec::new();
    let mut mods = Vec::new();
    let mut stack = vec![Block::Module(Vec::new(), file.feature.clone())];
    let mut attrs: Vec<String> = Vec::new();
    let mut attr_open: Option<String> = None;
    // A signature that spans lines: its text so far.
    let mut sig: Option<String> = None;

    for raw in file.code.lines() {
        let scrubbed = scrub(raw);
        let code = scrubbed.trim();
        if code.is_empty() {
            continue;
        }
        if let Some(open) = attr_open.as_mut() {
            open.push(' ');
            open.push_str(raw.trim());
            if open.matches('[').count() <= open.matches(']').count() {
                attrs.push(normalise(attr_open.take().as_deref().unwrap_or("")));
            }
            continue;
        }
        let top = stack.last().cloned().unwrap();
        let in_items = !matches!(top, Block::Body(_));
        if sig.is_none() && in_items && (code.starts_with("#[") || code.starts_with("#![")) {
            if code.starts_with("#![") {
                continue;
            }
            if code.matches('[').count() > code.matches(']').count() {
                attr_open = Some(raw.trim().to_owned());
            } else {
                attrs.push(raw.trim().to_owned());
            }
            continue;
        }
        let text = match sig.take() {
            Some(mut so_far) => {
                so_far.push(' ');
                so_far.push_str(raw.trim());
                so_far
            }
            None => raw.trim().to_owned(),
        };
        // Items in a module or impl may span lines: a `use` runs to its `;`, anything else to
        // the brace that opens its body or the `;` that ends it. Variants, fields and lines
        // in a body are taken a line at a time.
        let starts_item = matches!(top, Block::Module(..) | Block::Impl(..));
        let is_use = text.split_whitespace().take(2).any(|w| w == "use");
        let ends = if is_use {
            code.ends_with(';')
        } else {
            scrub(&text).contains('{') || code.ends_with(';') || code.ends_with('}')
        };
        if starts_item && !ends {
            sig = Some(text);
            continue;
        }
        let first = text.trim_start();
        let public = first.starts_with("pub ");
        let gate = attrs
            .iter()
            .find_map(|a| feature_of(a))
            .or_else(|| match &top {
                Block::Module(_, g)
                | Block::Impl(_, g)
                | Block::Enum(_, g, _)
                | Block::Struct(_, g, _) => g.clone(),
                _ => None,
            });
        let path = match &top {
            Block::Module(p, _) => p.clone(),
            _ => Vec::new(),
        };
        let words: Vec<&str> = first.split_whitespace().collect();
        let after_vis = if public { &words[1..] } else { &words[..] };
        let keyword = after_vis.first().copied().unwrap_or("");
        let normalised = normalise(&text);
        let mut opens: Option<Block> = None;
        let item = |kind: Kind, owner: String, sig: String, attrs: &[String]| Item {
            file: file.name.clone(),
            gate: gate.clone(),
            attrs: attrs.to_vec(),
            kind,
            owner,
            sig,
        };
        match &top {
            Block::Module(..) => {
                let name = after_vis
                    .get(1)
                    .map(|n| {
                        n.trim_end_matches([';', '{'])
                            .split(['<', '('])
                            .next()
                            .unwrap_or("")
                            .to_owned()
                    })
                    .unwrap_or_default();
                let kind = match (keyword, after_vis.get(1).copied()) {
                    ("use", _) => Kind::Use,
                    ("mod", _) => Kind::Mod,
                    ("fn", _) | ("const" | "async" | "unsafe", Some("fn")) => Kind::Fn,
                    ("enum", _) => Kind::Enum,
                    ("struct", _) => Kind::Struct,
                    ("trait", _) | ("unsafe", Some("trait")) => Kind::Trait,
                    _ => Kind::Other,
                };
                if keyword == "impl" || first.starts_with("impl<") || first.starts_with("impl ") {
                    if normalised.contains(" for ") {
                        items.push(item(
                            Kind::TraitImpl,
                            path.join("::"),
                            normalised.clone(),
                            &attrs,
                        ));
                        opens = Some(Block::Body(normalised.clone()));
                    } else {
                        opens = Some(Block::Impl(normalised.clone(), gate.clone()));
                    }
                } else {
                    if public {
                        items.push(item(kind, path.join("::"), normalised.clone(), &attrs));
                    }
                    opens = Some(match kind {
                        Kind::Mod => {
                            if code.ends_with(';') {
                                mods.push(ModDecl {
                                    inline: path.clone(),
                                    name: name.clone(),
                                    path: attrs.iter().find_map(|a| path_of(a)),
                                    gate: gate.clone(),
                                });
                            }
                            let mut inner = path.clone();
                            inner.push(name.clone());
                            Block::Module(inner, gate.clone())
                        }
                        Kind::Enum => Block::Enum(name.clone(), gate.clone(), public),
                        Kind::Struct => Block::Struct(name.clone(), gate.clone(), public),
                        Kind::Trait => Block::Trait,
                        _ => Block::Body(name.clone()),
                    });
                }
            }
            Block::Impl(header, _) => {
                if public {
                    items.push(item(
                        Kind::Method,
                        header.clone(),
                        normalised.clone(),
                        &attrs,
                    ));
                }
                opens = Some(Block::Body(header.clone()));
            }
            Block::Enum(name, _, true) if first.starts_with(|c: char| c.is_ascii_uppercase()) => {
                items.push(item(
                    Kind::Variant,
                    name.clone(),
                    normalised.clone(),
                    &attrs,
                ));
            }
            Block::Struct(name, _, true) if public => {
                items.push(item(Kind::Field, name.clone(), normalised.clone(), &attrs));
            }
            Block::Body(owner) if public => {
                items.push(item(
                    Kind::Nested,
                    owner.clone(),
                    normalised.clone(),
                    &attrs,
                ));
            }
            _ => {}
        }
        attrs.clear();
        for c in scrubbed.chars() {
            match c {
                '{' => stack.push(opens.take().unwrap_or_else(|| Block::Body(String::new()))),
                '}' if stack.len() > 1 => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    (items, mods)
}

/// The directory a file's child modules live in.
fn child_dir(rel: &Path) -> PathBuf {
    let parent = rel.parent().map(Path::to_path_buf).unwrap_or_default();
    match rel.file_name().and_then(|n| n.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => parent,
        _ => parent.join(rel.file_stem().unwrap()),
    }
}

/// Every file reachable from `root` (relative to `src/`), with its feature gate, and the
/// items of each.
fn walk(root: &str) -> (Vec<Facade>, Vec<Item>) {
    let mut files = Vec::new();
    let mut items = Vec::new();
    let mut queue = vec![(PathBuf::from(root), None::<String>)];
    while let Some((rel, feature)) = queue.pop() {
        let text = std::fs::read_to_string(src().join(&rel))
            .unwrap_or_else(|e| panic!("read {}: {e}", rel.display()));
        let file = Facade {
            name: rel.to_string_lossy().replace('\\', "/"),
            feature,
            code: without_tests(&text),
        };
        let (found, mods) = scan(&file);
        items.extend(found);
        for decl in mods {
            let gate = decl.gate.or_else(|| file.feature.clone());
            let base = rel.parent().map(Path::to_path_buf).unwrap_or_default();
            let child = match decl.path {
                Some(path) => decl.inline.iter().fold(base, |d, m| d.join(m)).join(path),
                None => {
                    let dir = decl.inline.iter().fold(child_dir(&rel), |d, m| d.join(m));
                    let flat = dir.join(format!("{}.rs", decl.name));
                    if src().join(&flat).exists() {
                        flat
                    } else {
                        dir.join(&decl.name).join("mod.rs")
                    }
                }
            };
            queue.push((child, gate));
        }
        files.push(file);
    }
    files.sort_by(|a, b| (a.name != "lib.rs", &a.name).cmp(&(b.name != "lib.rs", &b.name)));
    (files, items)
}

fn facade() -> (Vec<Facade>, Vec<Item>) {
    walk("lib.rs")
}

fn surface() -> Vec<String> {
    let (files, items) = facade();
    let order: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    let mut lines: Vec<(usize, usize, String)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            (
                order.iter().position(|f| *f == item.file).unwrap(),
                i,
                item.line(),
            )
        })
        .collect();
    lines.sort();
    lines.into_iter().map(|(_, _, line)| line).collect()
}

/// Lines of code with comments removed.
fn code_lines(code: &str) -> impl Iterator<Item = &str> {
    code.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .filter(|l| !l.trim().is_empty())
}

#[test]
fn lib_r1_r8_the_public_surface_is_the_reviewed_one() {
    insta::assert_snapshot!("public_api", surface().join("\n"));
}

#[test]
fn lib_r8_every_source_file_belongs_to_the_library_or_the_binary() {
    let (lib, _) = walk("lib.rs");
    let (bin, _) = walk("main.rs");
    let reached: BTreeSet<String> = lib.iter().chain(&bin).map(|f| f.name.clone()).collect();
    let mut stack = vec![src()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(src())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                assert!(
                    reached.contains(&rel),
                    "{rel} is in neither the library's nor the binary's module tree"
                );
            }
        }
    }
    let lib_names: BTreeSet<_> = lib.iter().map(|f| f.name.clone()).collect();
    for file in &bin {
        assert!(
            !lib_names.contains(&file.name),
            "{} is in both module trees",
            file.name
        );
    }
}

#[test]
fn lib_r1_exports_exactly_the_promised_names() {
    let (_, items) = facade();
    let mut exported: Vec<String> = Vec::new();
    for item in items
        .iter()
        .filter(|i| i.file == "lib.rs" && i.kind == Kind::Use)
    {
        let list = item.sig.trim_start_matches("pub use ");
        match list.split_once('{') {
            Some((_, names)) => exported.extend(
                names
                    .trim_end_matches('}')
                    .split(',')
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty()),
            ),
            None => exported.push(list.rsplit("::").next().unwrap_or(list).to_owned()),
        }
    }
    exported.sort_unstable();
    assert_eq!(
        exported,
        [
            "Category",
            "Error",
            "ErrorKind",
            "Options",
            "SceneInfo",
            "SceneWidget",
            "Stage",
            "Theme",
            "find",
            "scenes"
        ]
    );
    let modules: Vec<_> = items
        .iter()
        .filter(|i| i.kind == Kind::Mod)
        .map(|i| (i.file.as_str(), i.sig.as_str(), i.gate.as_deref()))
        .collect();
    assert_eq!(
        modules,
        [("lib.rs", "pub mod experimental", Some("unstable-generator"))]
    );
    let (files, _) = facade();
    let widget = files.iter().find(|f| f.name == "widget.rs").unwrap();
    assert_eq!(widget.feature.as_deref(), Some("widget"));
}

#[test]
fn lib_r1_nothing_internal_is_re_exported_outside_experimental() {
    let (_, items) = facade();
    for item in &items {
        if item.file != "experimental.rs" {
            assert!(!item.sig.starts_with("pub use sciart_"), "{}", item.line());
        }
    }
}

#[test]
fn lib_r2_no_dependency_types_outside_the_widget_and_experimental() {
    const BANNED: [&str; 5] = ["rand::", "rand_chacha", "ratatui", "crossterm", "clap"];
    let (files, _) = facade();
    for file in files
        .iter()
        .filter(|f| f.name != "widget.rs" && f.name != "experimental.rs")
    {
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
    let (_, items) = facade();
    let enums: Vec<_> = items.iter().filter(|i| i.kind == Kind::Enum).collect();
    let structs: Vec<_> = items.iter().filter(|i| i.kind == Kind::Struct).collect();
    assert!(
        enums.len() >= 3 && structs.len() >= 5,
        "the scan found the facade's types"
    );
    for e in enums {
        assert!(
            e.attrs.iter().any(|a| a == "#[non_exhaustive]"),
            "{}",
            e.line()
        );
    }
    for s in structs {
        assert!(
            !s.sig.contains("(pub ") && !s.sig.contains("{ pub "),
            "{}",
            s.line()
        );
    }
    for item in &items {
        assert!(item.kind != Kind::Field, "public field: {}", item.line());
        assert!(item.kind != Kind::Trait, "public trait: {}", item.line());
    }
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
    let (files, _) = facade();
    for file in &files {
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
    for name in ["lib.rs", "stage.rs"] {
        let text = std::fs::read_to_string(src().join(name)).unwrap();
        assert!(text.contains("# Panics"), "{name}");
        assert!(text.contains("panic = \"abort\""), "{name}");
    }
}

#[test]
fn lib_r8_the_scanner_finds_items_wherever_they_are() {
    let file = Facade {
        name: "probe.rs".to_owned(),
        feature: None,
        code: without_tests(
            "pub fn top() {}\n\
             mod hidden {\n\
             \x20       pub fn inline_item() {}\n\
             \x20   pub struct Deep { pub field: u8 }\n\
             }\n\
             impl Probe {\n\
             pub fn flush_left(&self) {}\n\
             \x20       pub fn over_indented(&self) {}\n\
             }\n\
             fn body() {\n\
             \x20   let s = \"{ not a brace\";\n\
             \x20   pub struct Nested;\n\
             }\n\
             #[path = \"elsewhere.rs\"]\n\
             mod moved;\n\
             #[cfg(test)]\n\
             mod tests {\n\
             \x20   pub fn ignored() {}\n\
             }\n",
        ),
    };
    let (items, mods) = scan(&file);
    let lines: Vec<_> = items.iter().map(Item::line).collect();
    assert_eq!(
        lines,
        [
            "probe.rs: pub fn top()",
            "probe.rs: mod hidden: pub fn inline_item()",
            "probe.rs: mod hidden: pub struct Deep { pub field: u8 }",
            "probe.rs: impl Probe: pub fn flush_left(&self)",
            "probe.rs: impl Probe: pub fn over_indented(&self)",
            "probe.rs: (nested in body) pub struct Nested",
        ]
    );
    assert_eq!(mods.len(), 1);
    assert_eq!(
        (mods[0].name.as_str(), mods[0].path.as_deref()),
        ("moved", Some("elsewhere.rs"))
    );
}
