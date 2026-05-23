//! Source-level rules that the type system cannot express (GEN-R3, GEN-R6, GEN-R7,
//! GEN-R15, TEST-R8, TEST-R9).
//!
//! These read the crate's own sources and SPEC-002 at test time, so they fail the moment
//! a module is added without a registry line, a generator reaches for the clock, or a
//! scene's category drifts from its spec heading.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use sciart_generators::registry;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_dir().join("../..")
}

/// The `.rs` files directly in `dir` (not its subdirectories), sorted.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    out.sort();
    out
}

/// The `.rs` files in `dir` and every directory below it, sorted.
fn rust_sources_deep(dir: &Path) -> Vec<PathBuf> {
    let mut out = rust_sources(dir);
    let mut subdirs: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();
    for sub in subdirs {
        out.extend(rust_sources_deep(&sub));
    }
    out
}

fn stems(files: &[PathBuf]) -> BTreeSet<String> {
    files
        .iter()
        .filter_map(|p| p.file_stem()?.to_str().map(str::to_owned))
        .collect()
}

#[test]
fn gen_r3_every_scene_module_file_is_registered_and_vice_versa() {
    // GEN-R3: one module file + one registry line. A file without a registry line
    // would silently escape the contract/property/snapshot suites. Scenes are the files
    // directly in src/; models shared by several scenes live in src/shared/ and are never
    // registered.
    let mut files = stems(&rust_sources(&crate_dir().join("src")));
    files.remove("lib");
    let shared = stems(&rust_sources_deep(&crate_dir().join("src/shared")));
    let registered: BTreeSet<String> = registry().iter().map(|e| e.meta.name.to_owned()).collect();
    assert!(
        shared.is_disjoint(&registered),
        "GEN-R3: a shared model cannot also be a registered scene"
    );
    assert_eq!(
        files, registered,
        "GEN-R3: src/<name>.rs files and registry![...] entries must match one-to-one"
    );
}

#[test]
fn gen_r16_no_scene_imports_another_scene() {
    // GEN-R16: inside the crate, scenes and shared models may reach only `crate::shared`.
    // Any other `crate::<module>` path would be a scene (or lib.rs internals), which ties
    // one scene's look to another's edits.
    let mut hits = Vec::new();
    for file in rust_sources_deep(&crate_dir().join("src")) {
        if file.ends_with("src/lib.rs") {
            continue;
        }
        let text = fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let mut rest = line;
            while let Some(at) = rest.find("crate::") {
                let after = &rest[at + "crate::".len()..];
                let module: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if module != "shared" {
                    hits.push(format!("{}:{}: crate::{module}", file.display(), n + 1));
                }
                rest = after;
            }
        }
    }
    assert!(
        hits.is_empty(),
        "GEN-R16: scenes may import only sciart_core and crate::shared:\n{}",
        hits.join("\n")
    );
}

#[test]
fn gen_r15_spec_headings_carry_each_scenes_category_label() {
    // SPEC-002 is where each scene's category (ADR-0013) is written down; this keeps the
    // headings `### <name>  (<category label>, <milestones>)` in step with `Meta.category`.
    let spec = fs::read_to_string(workspace_root().join("docs/specs/SPEC-002-generators.md"))
        .expect("SPEC-002");
    let mut headings: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in spec.lines() {
        let Some((name, tail)) = line
            .strip_prefix("### ")
            .and_then(|rest| rest.split_once("  ("))
        else {
            continue;
        };
        let label = tail.split([',', ')']).next().unwrap_or_default();
        headings.entry(name).or_default().push(label);
    }
    let problems: Vec<_> = registry()
        .iter()
        .filter_map(|e| {
            let want = e.meta.category.label();
            let found = headings.get(e.meta.name).map(Vec::as_slice);
            (found != Some(&[want][..])).then(|| {
                format!(
                    "{}: want one `### {}  ({want}, …)` heading, found labels {found:?}",
                    e.meta.name, e.meta.name
                )
            })
        })
        .collect();
    assert!(problems.is_empty(), "GEN-R15:\n{}", problems.join("\n"));
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
fn test_r8_clippy_config_bans_deliberate_leaks() {
    let clippy = fs::read_to_string(workspace_root().join("clippy.toml")).expect("clippy.toml");
    for path in [
        "std::boxed::Box::leak",
        "std::vec::Vec::leak",
        "std::string::String::leak",
        "std::mem::forget",
        "std::mem::ManuallyDrop::new",
    ] {
        assert!(
            clippy.contains(&format!("path = \"{path}\"")),
            "TEST-R8: clippy.toml disallowed-methods must ban `{path}`"
        );
    }
}

#[test]
fn gen_r6_core_r90_clippy_config_bans_fusable_trig() {
    // A sin + cos pair in one function becomes one `sincos` whose sine differs in the last
    // bit, depending on inlining; so frames differed between dev and release builds.
    let clippy = fs::read_to_string(workspace_root().join("clippy.toml")).expect("clippy.toml");
    for path in [
        "f64::sin",
        "f64::cos",
        "f64::sin_cos",
        "f32::sin",
        "f32::cos",
        "f32::sin_cos",
    ] {
        assert!(
            clippy.contains(&format!("path = \"{path}\"")),
            "GEN-R6: clippy.toml disallowed-methods must ban `{path}` (use sciart_core::trig)"
        );
    }
}

#[test]
fn test_r9_rs_ci_runs_the_snapshot_suite_in_release() {
    let pixi = fs::read_to_string(workspace_root().join("pixi.toml")).expect("pixi.toml");
    let task = |name: &str| {
        pixi.lines()
            .find(|l| l.starts_with(&format!("{name} = ")))
            .unwrap_or_else(|| panic!("TEST-R9: pixi.toml has no `{name}` task"))
            .to_owned()
    };
    let snap = task("rs-snap-release");
    assert!(
        snap.contains("--release") && snap.contains("--test snapshots"),
        "TEST-R9: rs-snap-release must run the snapshot suite in release: {snap}"
    );
    let ci = task("rs-ci");
    assert!(
        ci.contains("\"rs-snap-release\""),
        "TEST-R9: rs-ci must depend on rs-snap-release: {ci}"
    );
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
    // Recursive, so src/shared/ (and any future submodule) is held to the same rule.
    let files: Vec<PathBuf> = dirs.iter().flat_map(|d| rust_sources_deep(d)).collect();
    assert!(
        files
            .iter()
            .any(|f| f.parent().is_some_and(|p| p.ends_with("src/shared"))),
        "GEN-R7: the scan must reach src/shared/"
    );
    for file in files {
        let text = fs::read_to_string(&file).expect("read source");
        // CORE-R90: `trig.rs` is the one module allowed to call libm's sin/cos.
        let trig = file.ends_with("sciart-core/src/trig.rs");
        for (n, line) in text.lines().enumerate() {
            let allowed = |b: &&&str| trig && **b == "disallowed_methods";
            for bad in FORBIDDEN
                .iter()
                .filter(|b| line.contains(**b) && !allowed(b))
            {
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
