//! Source-level rules that the type system cannot express (GEN-R3, GEN-R6, GEN-R7,
//! GEN-R15, GEN-R16, TEST-R8, TEST-R9, TEST-R10, TEST-R11).
//!
//! These read the scene crates' sources, the manifests and SPEC-002 at test time, so they
//! fail the moment a module is added without a registry line or in the wrong category
//! crate, a generator reaches for the clock, or a scene's category drifts from its spec
//! heading.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use sciart_core::Category;
use sciart_generators::registry;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_dir().join("../..")
}

/// Every `docs/specs/SPEC-002*.md`, sorted (GEN-R15: the rules monolith plus the five
/// category files after `scripts/spec_split.py` has moved the scene sections).
fn spec_002_files() -> Vec<PathBuf> {
    let dir = workspace_root().join("docs/specs");
    let mut out: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("SPEC-002") && name.ends_with(".md"))
        })
        .collect();
    out.sort();
    out
}

/// The concatenation of every SPEC-002 file in path order, so a scene heading may live in
/// the monolith (before the split) or in a category file (after it).
fn spec_002_text() -> String {
    let mut text = String::new();
    for path in spec_002_files() {
        text.push_str(
            &fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
        );
        text.push('\n');
    }
    text
}

/// The scene crates (ADR-0023), by the suffix of `crates/sciart-scenes-<suffix>`.
const SCENE_CRATES: [&str; 5] = ["cosmos", "earth", "physics", "matter", "life"];

/// GEN-R3 / ADR-0023: the crate a scene of each category lives in.
fn crate_for(category: Category) -> &'static str {
    match category {
        Category::DeepCosmos | Category::SunPlanets => "cosmos",
        Category::EarthSky | Category::Voyages => "earth",
        Category::Physics => "physics",
        Category::Chemistry | Category::PhysicalChemistry => "matter",
        Category::Biology
        | Category::Biophysics
        | Category::Biochemistry
        | Category::LivingMatter => "life",
    }
}

fn scene_crate_dir(suffix: &str) -> PathBuf {
    workspace_root().join(format!("crates/sciart-scenes-{suffix}"))
}

fn shared_crate_dir() -> PathBuf {
    workspace_root().join("crates/sciart-shared")
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

/// The modules a crate's `src/lib.rs` declares with `pub mod <name>;`.
fn declared_modules(krate: &Path) -> BTreeSet<String> {
    let lib = fs::read_to_string(krate.join("src/lib.rs")).expect("lib.rs");
    lib.lines()
        .filter_map(|l| l.trim().strip_prefix("pub mod ")?.strip_suffix(';'))
        .map(str::to_owned)
        .collect()
}

/// The `[dependencies]` names of a crate's manifest (dev-dependencies excluded).
fn dependencies(krate: &Path) -> BTreeSet<String> {
    let text = fs::read_to_string(krate.join("Cargo.toml")).expect("Cargo.toml");
    let manifest: toml::Table = text.parse().expect("Cargo.toml is TOML");
    manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .map(|d| d.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn gen_r3_every_scene_module_file_is_registered_in_its_category_crate() {
    // GEN-R3 (amended M22, ADR-0023): one module file in its category's crate + one
    // registry line. A file without a registry line would silently escape the
    // contract/property/snapshot suites; a scene in the wrong crate breaks the split.
    let registered: BTreeMap<&str, Category> = registry()
        .iter()
        .map(|e| (e.meta.name, e.meta.category))
        .collect();
    let mut home: BTreeMap<String, &str> = BTreeMap::new();
    for suffix in SCENE_CRATES {
        let dir = scene_crate_dir(suffix);
        let mut files = stems(&rust_sources(&dir.join("src")));
        files.remove("lib");
        assert_eq!(
            files,
            declared_modules(&dir),
            "GEN-R3: sciart-scenes-{suffix}: src/<name>.rs files and `pub mod` lines must match"
        );
        assert!(
            rust_sources(&dir.join("src")).len() == files.len() + 1
                && rust_sources_deep(&dir.join("src")).len() == files.len() + 1,
            "GEN-R3: sciart-scenes-{suffix}/src holds only lib.rs and scene files"
        );
        for file in files {
            if let Some(other) = home.insert(file.clone(), suffix) {
                panic!("GEN-R3: scene `{file}` exists in both sciart-scenes-{other} and -{suffix}");
            }
        }
    }
    let homes: BTreeSet<&str> = home.keys().map(String::as_str).collect();
    let names: BTreeSet<&str> = registered.keys().copied().collect();
    assert_eq!(
        homes, names,
        "GEN-R3: scene files in crates/sciart-scenes-*/src and registry![...] entries must match one-to-one"
    );
    let misplaced: Vec<String> = registered
        .iter()
        .filter(|(name, category)| home[**name] != crate_for(**category))
        .map(|(name, category)| {
            format!(
                "  {name}: {:?} belongs in sciart-scenes-{}, found in sciart-scenes-{}",
                category,
                crate_for(*category),
                home[*name]
            )
        })
        .collect();
    assert!(
        misplaced.is_empty(),
        "GEN-R3: each scene lives in the crate its Meta.category maps to:\n{}",
        misplaced.join("\n")
    );
    let shared = stems(&rust_sources_deep(&shared_crate_dir().join("src")));
    assert!(
        shared.iter().all(|m| !names.contains(m.as_str())),
        "GEN-R3: a shared model cannot also be a registered scene"
    );
    // The facade holds the registry only: no scene modules of its own.
    let facade = stems(&rust_sources_deep(&crate_dir().join("src")));
    assert_eq!(
        facade,
        BTreeSet::from(["lib".to_owned()]),
        "GEN-R3: sciart-generators/src is the registry facade (lib.rs only)"
    );
}

#[test]
fn gen_r16_no_scene_imports_another_scene() {
    // GEN-R16 (M22): a scene uses only std, rand, sciart_core and sciart_shared. Scene
    // modules in one crate are siblings, so any `crate::` path from a scene reaches another
    // scene (or lib.rs); a path into another scene crate or the facade likewise. The
    // manifests enforce the same at the dependency level.
    const FOREIGN: [&str; 2] = ["sciart_scenes_", "sciart_generators"];
    let mut hits = Vec::new();
    let mut scan = |file: &Path, crate_paths_ok: bool| {
        let text = fs::read_to_string(file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let bad = FOREIGN
                .iter()
                .copied()
                .chain((!crate_paths_ok).then_some("crate::"))
                .filter(|b| line.contains(*b));
            for b in bad {
                hits.push(format!("{}:{}: `{b}`", file.display(), n + 1));
            }
        }
    };
    for suffix in SCENE_CRATES {
        for file in rust_sources_deep(&scene_crate_dir(suffix).join("src")) {
            if !file.ends_with("src/lib.rs") {
                scan(&file, false);
            }
        }
    }
    // Shared models may use their own crate's modules, never a scene.
    for file in rust_sources_deep(&shared_crate_dir().join("src")) {
        scan(&file, true);
    }
    assert!(
        hits.is_empty(),
        "GEN-R16: scenes may import only sciart_core, sciart_shared and rand:\n{}",
        hits.join("\n")
    );

    let allowed = |names: &[&str]| {
        names
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>()
    };
    for suffix in SCENE_CRATES {
        let deps = dependencies(&scene_crate_dir(suffix));
        assert!(
            deps.is_subset(&allowed(&["sciart-core", "sciart-shared", "rand"])),
            "GEN-R16: sciart-scenes-{suffix} depends only on sciart-core, sciart-shared, rand: {deps:?}"
        );
    }
    let deps = dependencies(&shared_crate_dir());
    assert!(
        deps.is_subset(&allowed(&["sciart-core", "rand"])),
        "GEN-R16: sciart-shared depends only on sciart-core and rand (never a scene): {deps:?}"
    );
}

#[test]
fn gen_r6_gen_r7_scene_crates_share_the_workspace_lints_and_clippy_config() {
    // GEN-R6 / GEN-R7 / TEST-R8 hold for the split crates only if clippy reads the root
    // clippy.toml for them (clippy looks in the crate directory first, so a crate-level
    // clippy.toml would shadow the bans) and they inherit the workspace lints.
    let crates: Vec<PathBuf> = SCENE_CRATES
        .iter()
        .map(|s| scene_crate_dir(s))
        .chain([shared_crate_dir(), crate_dir()])
        .collect();
    for krate in &crates {
        for name in ["clippy.toml", ".clippy.toml"] {
            assert!(
                !krate.join(name).exists(),
                "GEN-R6: {} must not shadow the root clippy.toml",
                krate.join(name).display()
            );
        }
        let text = fs::read_to_string(krate.join("Cargo.toml")).expect("Cargo.toml");
        let manifest: toml::Table = text.parse().expect("Cargo.toml is TOML");
        assert_eq!(
            manifest
                .get("lints")
                .and_then(|l| l.get("workspace"))
                .and_then(toml::Value::as_bool),
            Some(true),
            "GEN-R6: {} must set `[lints] workspace = true`",
            krate.display()
        );
    }
}

#[test]
fn gen_r15_spec_headings_carry_each_scenes_category_label() {
    // SPEC-002 is where each scene's category (ADR-0013) is written down; this keeps the
    // headings `### <name>  (<category label>, <milestones>)` in step with `Meta.category`.
    // GEN-R15: the sections may be in the monolith or in the category files, so every
    // `SPEC-002*.md` is concatenated.
    let spec = spec_002_text();
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

/// GEN-R15: once the scene sections live in the category files, the monolith's generated
/// index must list every registered scene exactly once, and nothing else. Before the split
/// there are no markers and no category files, so the check is inactive; a half-done split
/// (category files without an index) fails.
#[test]
fn gen_r15_scene_index_lists_every_registered_scene_once() {
    let dir = workspace_root().join("docs/specs");
    let monolith =
        fs::read_to_string(dir.join("SPEC-002-generators.md")).expect("read SPEC-002 monolith");
    let category_files: Vec<PathBuf> = spec_002_files()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != "SPEC-002-generators.md")
        })
        .collect();
    let (start, end) = match (
        monolith.find("<!-- scene-index:start -->"),
        monolith.find("<!-- scene-index:end -->"),
    ) {
        (Some(start), Some(end)) => (start, end),
        (None, None) => {
            assert!(
                category_files.is_empty(),
                "GEN-R15: the scene sections are in {category_files:?} but \
                 SPEC-002-generators.md has no scene-index markers; run \
                 `python3 scripts/spec_index.py`"
            );
            return;
        }
        _ => panic!("GEN-R15: both scene-index markers are required"),
    };
    assert!(start < end, "GEN-R15: scene-index markers are out of order");
    let mut listed: BTreeMap<&str, usize> = BTreeMap::new();
    for line in monolith[start..end].lines() {
        // A row is `| \`name\` | Category | [spec](SPEC-002-scenes-…md) |`.
        if let Some(name) = line.split('`').nth(1) {
            *listed.entry(name).or_default() += 1;
        }
    }
    let problems: Vec<_> = registry()
        .iter()
        .filter_map(|e| {
            let found = listed.get(e.meta.name).copied().unwrap_or(0);
            (found != 1)
                .then(|| format!("{}: listed {found} times in the scene index", e.meta.name))
        })
        .collect();
    assert!(problems.is_empty(), "GEN-R15:\n{}", problems.join("\n"));
    let known: BTreeSet<&str> = registry().iter().map(|e| e.meta.name).collect();
    let stale: Vec<&str> = listed
        .keys()
        .copied()
        .filter(|name| !known.contains(name))
        .collect();
    assert!(
        stale.is_empty(),
        "GEN-R15: the scene index lists unknown scenes: {stale:?}"
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
    // Every crate a frame depends on (ADR-0023): core, the shared models, the five scene
    // crates and the registry facade.
    let dirs: Vec<PathBuf> = [
        crate_dir().join("src"),
        workspace_root().join("crates/sciart-core/src"),
        shared_crate_dir().join("src"),
    ]
    .into_iter()
    .chain(SCENE_CRATES.iter().map(|s| scene_crate_dir(s).join("src")))
    .collect();
    let mut hits = Vec::new();
    // Recursive, so any submodule is held to the same rule.
    let files: Vec<PathBuf> = dirs.iter().flat_map(|d| rust_sources_deep(d)).collect();
    // The facade (first) is lib.rs alone; every other crate has modules to scan.
    for dir in dirs.iter().skip(1) {
        assert!(
            files
                .iter()
                .any(|f| f.starts_with(dir) && !f.ends_with("lib.rs")),
            "GEN-R7: the scan must reach the modules in {}",
            dir.display()
        );
    }
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
        "GEN-R7: core, shared models and scenes must use dt/ctx.time and ctx.rng only:\n{}",
        hits.join("\n")
    );
}
