//! ADR-0009: `sciart-testalloc` cannot inherit `[workspace.lints]` (a `forbid` cannot be
//! relaxed by `#[allow]`), so it carries a copy. This test keeps the copy honest: the
//! only permitted difference is `unsafe_code = "deny"` (instead of `"forbid"`) plus
//! extra, stricter unsafe-hygiene lints.

use std::fs;
use std::path::PathBuf;

use toml::{Table, Value};

fn manifest(path: PathBuf) -> Table {
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    text.parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn table<'a>(t: &'a Table, keys: &[&str]) -> &'a Table {
    keys.iter().fold(t, |t, k| {
        t.get(*k)
            .and_then(Value::as_table)
            .unwrap_or_else(|| panic!("missing [{}]", keys.join(".")))
    })
}

#[test]
fn test_r5_crate_lints_mirror_workspace_lints_except_unsafe_code() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest(dir.join("../../Cargo.toml"));
    let own = manifest(dir.join("Cargo.toml"));

    let ws = table(&root, &["workspace", "lints"]);
    let mine = table(&own, &["lints"]);
    assert!(
        own.get("lints").and_then(|l| l.get("workspace")).is_none(),
        "must not inherit workspace lints (forbid(unsafe_code) cannot be allowed locally)"
    );

    for group in ["rust", "clippy"] {
        let (ws_g, my_g) = (table(ws, &[group]), table(mine, &[group]));
        for (name, level) in ws_g {
            let expected = if group == "rust" && name == "unsafe_code" {
                assert_eq!(
                    level.as_str(),
                    Some("forbid"),
                    "workspace unsafe_code changed"
                );
                Value::from("deny")
            } else {
                level.clone()
            };
            assert_eq!(
                my_g.get(name),
                Some(&expected),
                "[lints.{group}] {name} must mirror [workspace.lints.{group}]"
            );
        }
    }
    for group in table(ws, &[]).keys() {
        assert!(mine.contains_key(group), "lint group [{group}] missing");
    }
}
