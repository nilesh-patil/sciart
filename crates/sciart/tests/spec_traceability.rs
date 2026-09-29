//! Spec <-> test traceability (SPEC-006 TEST-R1, TEST-R4).
//!
//! Scans `docs/specs/*.md` for defined requirement IDs (bold `**CORE-R10**`) and scene
//! acceptance IDs (`` `SOL-A1` `` on a `- **Acceptance**` line), then scans every
//! `#[test]` function under `crates/` for citations. A test cites an ID through its
//! name, the comments/attributes directly above it, or its body (assert messages),
//! case-insensitively with `-`/`_` equivalence: `core_r10_…`, `[GEN-R4]`, and chains
//! such as `core_r9_r10_r12_…` or `cry_a1_a2_…`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Requirements verified outside `cargo test` (SPEC-006 TEST-R1 "non-test verification").
/// Keep this list short and honest: an entry here is a promise someone checks by hand.
const MANUAL: &[(&str, &str)] = &[
    (
        "GEN-R8",
        "release-mode timing budget: `pixi run rs-bench` (criterion); debug-mode tests cannot measure it reliably",
    ),
    (
        "GEN-R19",
        "release-mode resize timing: `pixi run rs-worst --size 500x150` reports resize ms; debug-mode tests cannot measure it reliably",
    ),
    (
        "TEST-R2",
        "process: snapshot diffs are reviewed by a human (`cargo insta review`)",
    ),
    (
        "TEST-R3",
        "process: `pixi run rs-ci` is the pre-commit gate (definition lives in pixi.toml)",
    ),
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `PREFIX-<kind>N`, e.g. `CORE-R10` (kind `R`) or `SOL-A1` (kind `A`).
fn is_id(s: &str, kind: char) -> bool {
    let Some((prefix, rest)) = s.split_once('-') else {
        return false;
    };
    let mut rest = rest.chars();
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && rest.next() == Some(kind)
        && {
            let digits = rest.as_str();
            !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
        }
}

/// Defined IDs -> spec file that defines them.
fn spec_ids() -> BTreeMap<String, String> {
    let dir = workspace_root().join("docs/specs");
    let mut ids = BTreeMap::new();
    for path in files_with_ext(&dir, "md") {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).expect("read spec");
        for line in text.lines() {
            // Bold definitions: segments between `**` pairs.
            for seg in line.split("**").skip(1).step_by(2) {
                if is_id(seg, 'R') {
                    ids.entry(seg.to_owned()).or_insert_with(|| file.clone());
                }
            }
            if line.trim_start().starts_with("- **Acceptance**") {
                for seg in line.split('`').skip(1).step_by(2) {
                    if is_id(seg, 'A') {
                        ids.entry(seg.to_owned()).or_insert_with(|| file.clone());
                    }
                }
            }
        }
    }
    ids
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap_or_else(|e| panic!("read {}: {e}", d.display())) {
            let path = entry.expect("dir entry").path();
            let name = path.file_name().unwrap().to_string_lossy();
            if path.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|x| x == ext) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// `(path::name, text)` for every `#[test]` fn: attached comments/attributes + signature + body.
fn test_fns() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    for path in files_with_ext(&root.join("crates"), "rs") {
        let text = fs::read_to_string(&path).expect("read source");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim() != "#[test]" {
                continue;
            }
            let mut start = i;
            while start > 0 {
                let above = lines[start - 1].trim_start();
                if above.starts_with("//") || above.starts_with("#[") {
                    start -= 1;
                } else {
                    break;
                }
            }
            let Some(fn_line) = (i..lines.len()).find(|&k| fn_name(lines[k]).is_some()) else {
                continue;
            };
            let name = fn_name(lines[fn_line]).unwrap();
            let (mut depth, mut opened, mut end) = (0i64, false, fn_line);
            while end < lines.len() {
                let l = lines[end];
                depth += l.matches('{').count() as i64 - l.matches('}').count() as i64;
                opened |= l.contains('{');
                if opened && depth <= 0 {
                    break;
                }
                end += 1;
            }
            let end = end.min(lines.len() - 1);
            let rel = path.strip_prefix(&root).unwrap_or(&path).display();
            out.push((format!("{rel}::{name}"), lines[start..=end].join("\n")));
        }
    }
    out
}

fn fn_name(line: &str) -> Option<&str> {
    let after = line.split("fn ").nth(1)?;
    let name = after
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()?;
    (!name.is_empty()).then_some(name)
}

/// IDs (upper-case, `-` separated) cited in `text` for the given lower-case prefixes.
fn cited_ids(text: &str, prefixes: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let lower = text.to_ascii_lowercase();
    let words = lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    for word in words {
        let mut prefix: Option<&str> = None;
        for tok in word.split(['_', '-']) {
            let is_num = tok.len() > 1
                && (tok.starts_with('r') || tok.starts_with('a'))
                && tok[1..].chars().all(|c| c.is_ascii_digit());
            match prefix {
                Some(p) if is_num => {
                    found.insert(format!("{p}-{tok}").to_ascii_uppercase());
                }
                _ if prefixes.contains(tok) => prefix = Some(tok),
                _ => prefix = None,
            }
        }
    }
    found
}

#[test]
fn test_r1_every_spec_id_is_cited_by_a_test_or_listed_as_manual() {
    let ids = spec_ids();
    assert!(
        ids.contains_key("TEST-R1") && ids.len() > 100,
        "spec scan looks broken: found {} IDs",
        ids.len()
    );
    let prefixes: BTreeSet<String> = ids
        .keys()
        .map(|id| id.split('-').next().unwrap().to_ascii_lowercase())
        .collect();
    let tests = test_fns();
    let mut cited: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (name, text) in &tests {
        for id in cited_ids(text, &prefixes) {
            cited.entry(id).or_default().push(name);
        }
    }
    let manual: BTreeMap<&str, &str> = MANUAL.iter().copied().collect();

    let uncited: Vec<_> = ids
        .iter()
        .filter(|(id, _)| !cited.contains_key(*id) && !manual.contains_key(id.as_str()))
        .map(|(id, file)| format!("  {id} ({file})"))
        .collect();
    let undefined: Vec<_> = cited
        .iter()
        .filter(|(id, _)| !ids.contains_key(*id))
        .map(|(id, by)| format!("  {id} cited by {}", by.join(", ")))
        .collect();
    let stale_manual: Vec<_> = MANUAL
        .iter()
        .filter(|(id, _)| !ids.contains_key(*id) || cited.contains_key(*id))
        .map(|(id, _)| format!("  {id}"))
        .collect();

    assert!(
        uncited.is_empty() && undefined.is_empty() && stale_manual.is_empty(),
        "TEST-R1 traceability ({} IDs, {} tests scanned)\n\
         uncited (add a test citing it, e.g. `core_r10_…`, or a MANUAL entry with a reason):\n{}\n\
         cited but not defined in docs/specs (typo or renumbered spec?):\n{}\n\
         MANUAL entries that are undefined or now cited (remove them):\n{}",
        ids.len(),
        tests.len(),
        uncited.join("\n"),
        undefined.join("\n"),
        stale_manual.join("\n"),
    );
}

#[test]
fn test_r1_citation_parser_accepts_chains_and_rejects_prefix_collisions() {
    // Uses a prefix no spec defines, so this test cannot cite real IDs by accident.
    let prefixes: BTreeSet<String> = ["xq".to_owned()].into();
    let ids = |text: &str| -> Vec<String> { cited_ids(text, &prefixes).into_iter().collect() };
    assert_eq!(ids("fn xq_r1_does_things()"), ["XQ-R1"]);
    assert_eq!(ids("// [XQ-R10] and xq_a2"), ["XQ-A2", "XQ-R10"]);
    assert_eq!(
        ids("fn xq_r9_r10_r12_chain()"),
        ["XQ-R10", "XQ-R12", "XQ-R9"]
    );
    assert!(ids("fn xq_resize_r1()").is_empty(), "chain breaks on words");
    assert!(
        ids("fn axq_r1()").is_empty(),
        "prefix must be a whole token"
    );
    assert!(ids("XQ-Rx XQ-A XQ R1").is_empty());
}

#[test]
fn test_r4_generator_and_binary_proptests_cap_cases_at_64() {
    // TEST-R4: whole-scene property runs are the expensive ones, so every `proptest!`
    // block outside the pure, microsecond-cheap `sciart-core` pins `with_cases(n <= 64)`.
    let root = workspace_root();
    let core = root.join("crates/sciart-core");
    let mut offenders = Vec::new();
    let mut blocks = 0;
    for path in files_with_ext(&root.join("crates"), "rs") {
        if path.starts_with(&core) {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read source");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("proptest!") {
                continue;
            }
            blocks += 1;
            let config = lines[i + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .unwrap_or("");
            let cases = config
                .split_once("with_cases(")
                .and_then(|(_, rest)| rest.split(')').next())
                .and_then(|n| n.trim().parse::<u32>().ok());
            if cases.is_none_or(|n| n > 64) {
                offenders.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    assert!(blocks > 0, "expected at least the generator property suite");
    assert!(
        offenders.is_empty(),
        "TEST-R4: proptest blocks must start with \
         `#![proptest_config(ProptestConfig::with_cases(n))]`, n <= 64:\n{}",
        offenders.join("\n")
    );
}
