//! Source-level rules that the type system cannot express (GEN-R3, GEN-R7).
//!
//! These read the crate's own sources at test time, so they fail the moment a
//! module is added without a registry line or a generator reaches for the clock.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sciart_generators::registry;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_dir().join("../..")
}

fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    out.sort();
    out
}

#[test]
fn gen_r3_every_scene_module_file_is_registered_and_vice_versa() {
    // GEN-R3: one module file + one registry line. A file without a registry line
    // would silently escape the contract/property/snapshot suites.
    let files: BTreeSet<String> = rust_sources(&crate_dir().join("src"))
        .iter()
        .filter_map(|p| p.file_stem()?.to_str().map(str::to_owned))
        .filter(|stem| stem != "lib")
        .collect();
    let registered: BTreeSet<String> = registry().iter().map(|e| e.meta.name.to_owned()).collect();
    assert_eq!(
        files, registered,
        "GEN-R3: src/<name>.rs files and registry![...] entries must match one-to-one"
    );
}

#[test]
fn gen_r7_clippy_config_bans_wall_clock_and_global_rng() {
    let clippy = fs::read_to_string(workspace_root().join("clippy.toml")).expect("clippy.toml");
    for path in [
        "std::time::Instant::now",
        "std::time::SystemTime::now",
        "rand::rng",
        "rand::thread_rng",
        "rand::random",
    ] {
        assert!(
            clippy.contains(&format!("path = \"{path}\"")),
            "GEN-R7: clippy.toml disallowed-methods must ban `{path}`"
        );
    }
}

#[test]
fn gen_r7_core_and_generators_never_touch_clock_or_global_rng() {
    // Belt and braces for the clippy ban: also catches the escape hatch
    // (`allow(clippy::disallowed_methods)`) and APIs clippy's list does not name.
    const FORBIDDEN: &[&str] = &[
        "Instant::",
        "SystemTime",
        "thread_rng",
        "ThreadRng",
        "OsRng",
        "rand::rng(",
        "rand::random",
        "disallowed_methods",
    ];
    let dirs = [
        crate_dir().join("src"),
        workspace_root().join("crates/sciart-core/src"),
    ];
    let mut hits = Vec::new();
    for file in dirs.iter().flat_map(|d| rust_sources(d)) {
        let text = fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            for bad in FORBIDDEN.iter().filter(|b| line.contains(**b)) {
                hits.push(format!("{}:{}: `{bad}`", file.display(), n + 1));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "GEN-R7: generators/core must use dt/ctx.time and ctx.rng only:\n{}",
        hits.join("\n")
    );
}
