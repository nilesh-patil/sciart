//! Source-level rules that the type system cannot express (GEN-R3, GEN-R6, GEN-R7,
//! GEN-R15, TEST-R8, TEST-R9, TEST-R10, TEST-R11).
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

fn read_toml(rel: &str) -> toml::Table {
    let text =
        fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    text.parse()
        .unwrap_or_else(|e| panic!("{rel} is not valid TOML: {e}"))
}

/// A `[tasks]` entry of pixi.toml: its command ("" for a pure `depends-on` task) and the
/// tasks it depends on. A task is either a command string or a table.
fn pixi_task(pixi: &toml::Table, name: &str) -> (String, Vec<String>) {
    let task = pixi
        .get("tasks")
        .and_then(|t| t.get(name))
        .unwrap_or_else(|| panic!("pixi.toml has no `{name}` task"));
    match task {
        toml::Value::String(cmd) => (cmd.clone(), Vec::new()),
        toml::Value::Table(t) => (
            t.get("cmd")
                .and_then(toml::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            t.get("depends-on")
                .and_then(toml::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        other => panic!("pixi task `{name}` has an unexpected shape: {other}"),
    }
}

/// `[profile.<name>]` of the root Cargo.toml.
fn cargo_profile<'a>(cargo: &'a toml::Table, name: &str) -> &'a toml::Table {
    cargo
        .get("profile")
        .and_then(|p| p.get(name))
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("Cargo.toml has no [profile.{name}]"))
}

/// `[profile.<name>.package]`: the per-crate overrides of a profile.
fn profile_packages(cargo: &toml::Table, name: &str) -> toml::Table {
    cargo_profile(cargo, name)
        .get("package")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default()
}

fn opt_level(t: &toml::Table) -> Option<i64> {
    t.get("opt-level").and_then(toml::Value::as_integer)
}

#[test]
fn test_r9_rs_ci_runs_the_snapshot_suite_in_the_optimised_ci_profile() {
    // TEST-R9 (amended M22, ADR-0023): the snapshot suite also runs at opt-level 3 in the
    // `ci` profile (release codegen, many codegen units, no LTO, unwinding), so a frame
    // that changes with the optimiser fails the gate; installs keep `release`.
    let pixi = read_toml("pixi.toml");
    let (snap, _) = pixi_task(&pixi, "rs-snap-release");
    assert!(
        snap.contains("--profile ci") && snap.contains("--test snapshots"),
        "TEST-R9: rs-snap-release must run the snapshot suite in the `ci` profile: {snap}"
    );
    let (_, deps) = pixi_task(&pixi, "rs-ci");
    assert!(
        deps.iter().any(|d| d == "rs-snap-release"),
        "TEST-R9: rs-ci must depend on rs-snap-release: {deps:?}"
    );
    for probe in ["rs-cost", "rs-worst"] {
        let (cmd, _) = pixi_task(&pixi, probe);
        assert!(
            cmd.contains("--profile ci"),
            "TEST-R9: {probe} builds with the `ci` profile: {cmd}"
        );
    }
    for install in ["rs-install", "rs-install-mouse"] {
        let (cmd, _) = pixi_task(&pixi, install);
        assert!(
            !cmd.contains("--profile") && !cmd.contains("--debug"),
            "TEST-R9: installs keep the `release` profile: {cmd}"
        );
    }

    let cargo = read_toml("Cargo.toml");
    let ci = cargo_profile(&cargo, "ci");
    let str_of = |k: &str| ci.get(k).and_then(toml::Value::as_str);
    assert_eq!(
        str_of("inherits"),
        Some("release"),
        "TEST-R9: ci inherits release"
    );
    assert_eq!(opt_level(ci), Some(3), "TEST-R9: ci builds at opt-level 3");
    assert_eq!(
        str_of("panic"),
        Some("unwind"),
        "TEST-R9 / ADR-0006: ci unwinds"
    );
    let lto_off =
        str_of("lto") == Some("off") || ci.get("lto").and_then(toml::Value::as_bool) == Some(false);
    assert!(lto_off, "TEST-R9: ci has no LTO: {:?}", ci.get("lto"));
    assert!(
        ci.get("codegen-units")
            .and_then(toml::Value::as_integer)
            .is_some_and(|n| n >= 16),
        "TEST-R9: ci uses >= 16 codegen units"
    );
    // `ci` inherits `release`; neither may lower a crate's opt-level per package.
    for profile in ["ci", "release"] {
        for (krate, over) in &profile_packages(&cargo, profile) {
            assert!(
                over.as_table().and_then(opt_level).is_none_or(|o| o >= 3),
                "TEST-R9: [profile.{profile}.package.{krate}] lowers the opt-level"
            );
        }
    }
}

#[test]
fn test_r10_rs_test_runs_every_test_under_nextest_on_an_optimised_checked_build() {
    // TEST-R10 / ADR-0023: rs-test = every workspace test under nextest (doctests through
    // `cargo test --doc`), nothing filtered, skipped or retried, on an optimised test build
    // that keeps debug assertions and overflow checks. `pixi run rs-test-parity` proves
    // the test-name set equals `cargo test --workspace -- --list`.
    let pixi = read_toml("pixi.toml");
    let (cmd, _) = pixi_task(&pixi, "rs-test");
    assert!(
        cmd.contains("cargo nextest run --workspace")
            && cmd.contains("cargo test --workspace --doc"),
        "TEST-R10: rs-test runs the workspace under nextest plus the doctests: {cmd}"
    );
    for narrowing in [
        "--skip",
        "-E ",
        "--filterset",
        "--ignored",
        "--run-ignored",
        "--retries",
        "--exclude",
        "--lib",
        "--profile",
        "--cargo-profile",
    ] {
        assert!(
            !cmd.contains(narrowing),
            "TEST-R10: rs-test must not narrow or re-profile the run (`{narrowing}`): {cmd}"
        );
    }
    let (_, ci_deps) = pixi_task(&pixi, "rs-ci");
    assert!(
        ci_deps.iter().any(|d| d == "rs-test"),
        "TEST-R10: rs-ci runs rs-test"
    );

    let cargo = read_toml("Cargo.toml");
    let test = cargo_profile(&cargo, "test");
    assert!(
        opt_level(test).is_some_and(|o| o >= 2),
        "TEST-R10: the test profile is opt-level >= 2"
    );
    for key in ["debug-assertions", "overflow-checks"] {
        assert_eq!(
            test.get(key).and_then(toml::Value::as_bool),
            Some(true),
            "TEST-R10: the test profile states `{key} = true`"
        );
    }
    // `test` inherits `dev`, per-package overrides included, so every crate that dev pins
    // to an opt-level needs a test override, and no test override may go below 2.
    let test_pkgs = profile_packages(&cargo, "test");
    for (krate, over) in &profile_packages(&cargo, "dev") {
        if over.as_table().and_then(opt_level).is_some() {
            let level = test_pkgs
                .get(krate)
                .and_then(toml::Value::as_table)
                .and_then(opt_level);
            assert!(
                level.is_some_and(|o| o >= 2),
                "TEST-R10: [profile.dev.package.{krate}] sets an opt-level that the test \
                 profile inherits; add [profile.test.package.{krate}] opt-level >= 2"
            );
        }
    }
    for (krate, over) in &test_pkgs {
        let t = over.as_table().expect("package override table");
        assert!(
            opt_level(t).is_none_or(|o| o >= 2),
            "TEST-R10: [profile.test.package.{krate}] is opt-level >= 2"
        );
        for key in ["debug-assertions", "overflow-checks"] {
            assert_ne!(
                t.get(key).and_then(toml::Value::as_bool),
                Some(false),
                "TEST-R10: [profile.test.package.{krate}] keeps `{key}`"
            );
        }
    }

    // The nextest config may schedule and time out tests, never filter or retry them.
    let nextest = read_toml(".config/nextest.toml");
    let profiles = nextest
        .get("profile")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    for (name, profile) in &profiles {
        let profile = profile.as_table().expect("nextest profile table");
        let overrides = profile
            .get("overrides")
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tables = std::iter::once(profile)
            .chain(overrides.iter().filter_map(toml::Value::as_table))
            .collect::<Vec<_>>();
        for t in tables {
            for key in ["default-filter", "retries", "run-ignored"] {
                assert!(
                    !t.contains_key(key),
                    "TEST-R10: nextest profile `{name}` must not set `{key}`"
                );
            }
        }
    }
}

#[test]
fn test_r11_every_pixi_task_uses_sccache_when_installed() {
    // TEST-R11 / ADR-0023: the pixi environment ships sccache and its activation script
    // makes it the rustc wrapper, skipped when sccache is missing (same build, slower).
    let pixi = read_toml("pixi.toml");
    assert!(
        pixi.get("dependencies")
            .and_then(|d| d.get("sccache"))
            .is_some(),
        "TEST-R11: pixi.toml [dependencies] provides sccache"
    );
    let scripts: Vec<&str> = pixi
        .get("activation")
        .and_then(|a| a.get("scripts"))
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(toml::Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        !scripts.is_empty(),
        "TEST-R11: pixi.toml [activation] scripts sets the wrapper"
    );
    let text: String = scripts
        .iter()
        .map(|s| {
            fs::read_to_string(workspace_root().join(s)).unwrap_or_else(|e| panic!("{s}: {e}"))
        })
        .collect();
    assert!(
        text.contains("export RUSTC_WRAPPER=sccache") && text.contains("command -v sccache"),
        "TEST-R11: activation exports RUSTC_WRAPPER=sccache, guarded by `command -v sccache`"
    );
    // A task that sets its own wrapper would silently bypass the shared cache.
    let tasks = pixi
        .get("tasks")
        .and_then(toml::Value::as_table)
        .expect("pixi.toml [tasks]");
    for name in tasks.keys() {
        let (cmd, _) = pixi_task(&pixi, name);
        assert!(
            !cmd.contains("RUSTC_WRAPPER"),
            "TEST-R11: task `{name}` overrides RUSTC_WRAPPER"
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
