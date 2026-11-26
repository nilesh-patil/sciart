#!/usr/bin/env python3
"""Release configuration checks (SPEC-008 DIST-R1, DIST-R3..DIST-R6, DIST-R8..DIST-R10).

Without arguments it checks, from the files alone:

- DIST-R1 one version: the root Cargo.toml's version of `sciart`, the internal crates'
  shared 0.x version and their `=` pins, Cargo.lock, no version in pixi.toml, the conda
  recipe reading SCIART_VERSION;
- DIST-R3 the published set and the labels of the internal crates;
- DIST-R4 the packaging rules (excludes, licence and changelog links, publish order);
- DIST-R5 / DIST-R6 the release workflow: targets, runners, build command, smoke tests,
  size budgets, packing, checksums, attestations, and the binstall metadata;
- DIST-R8 the conda recipe and its pixi environment;
- DIST-R9 the Homebrew formula rendered from a fixture (and `ruby -c` when Ruby is
  installed; `--require-ruby` makes it mandatory);
- DIST-R10 the gates: a draft release on a pushed tag, and every upload behind the
  `release` environment.

Other modes:

    --tag v1.2.3        also: the tag is v{version} and CHANGELOG.md has its heading (DIST-R1)
    --crates DIR        the .crate files `cargo publish --dry-run` left in DIR (DIST-R4)
    --notes 1.2.3       print the CHANGELOG section of a version (the release notes)
    --print-version     print the version of `sciart`
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_assets  # noqa: E402
import render_formula  # noqa: E402
from tooling_check import TIER_1, TIER_2, runner_target, workflow_jobs  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
FACADE = "sciart"
# DIST-R4: the publish order, each crate after everything it depends on.
INTERNAL = (
    "sciart-core",
    "sciart-shared",
    "sciart-scenes-cosmos",
    "sciart-scenes-earth",
    "sciart-scenes-physics",
    "sciart-scenes-matter",
    "sciart-scenes-life",
    "sciart-generators",
    "sciart-render",
    "sciart-keepawake",
    "sciart-capture",
)
PUBLISH_ORDER = INTERNAL + (FACADE,)
NEVER_PUBLISHED = ("sciart-testalloc", "sciart-qa")
# Internal crates whose crate-level doc notice is not written yet. May only shrink.
PENDING_NOTICE = {"sciart-generators"}
NOTICE = ("Internal crate of", "no semver promise")
EXCLUDED = ("tests/", "benches/", "examples/")
MAX_CRATE_BYTES = 1 << 20
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?$")
OLDEST_UBUNTU = "ubuntu-22.04"  # glibc 2.35 (DIST-R5)
CONDA_PLATFORMS = {"osx-arm64", "osx-64", "linux-64", "linux-aarch64"}
PACKAGE_CHECK = ("cargo publish --workspace --dry-run --locked && "
                 "python3 scripts/release_check.py --crates target/package")
BUILD = "cargo auditable build --release --locked -p sciart --bin sciart --target ${{ matrix.target }}"
UPLOADS = ("cargo publish --workspace --locked", "rattler-build upload", "git push")


def toml_file(rel: str) -> dict:
    return tomllib.loads((ROOT / rel).read_text())


def members() -> dict[str, tuple[Path, dict]]:
    out = {}
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text())
        out[data["package"]["name"]] = (manifest.parent, data)
    return out


def sciart_version() -> str:
    return toml_file("Cargo.toml")["workspace"]["package"]["version"]


def internal_version(crates: dict[str, tuple[Path, dict]]) -> object:
    return crates["sciart-core"][1]["package"].get("version")


def deps(manifest: dict, kinds: tuple[str, ...] = ("dependencies", "build-dependencies")) -> set[str]:
    """Names of a manifest's dependencies of `kinds`, target-specific ones included."""
    names: set[str] = set()
    tables = [manifest] + list(manifest.get("target", {}).values())
    for table in tables:
        for kind in kinds:
            names |= set(table.get(kind, {}))
    return names


def steps(job_body: str) -> list[str]:
    """The steps of a job, each as its text (six-space `- ` items under `steps:`)."""
    text = job_body.split("\n    steps:\n", 1)
    if len(text) != 2:
        return []
    out: list[str] = []
    for line in text[1].splitlines():
        if line.startswith("      #"):
            continue
        if line.startswith("      - "):
            out.append(line + "\n")
        elif out and (line.startswith("        ") or not line.strip()):
            out[-1] += line + "\n"
        elif line.strip():
            break
    return out


def matrix_entries(job_body: str) -> list[dict[str, str]]:
    """The `include:` entries of a job's matrix as {key: value}."""
    block = re.search(r"^        include:\n((?:(?:          .*)?\n)*)", job_body, re.M)
    if block is None:
        return []
    entries: list[dict[str, str]] = []
    for line in block.group(1).splitlines():
        found = re.match(r"^          (- )?\s*([A-Za-z0-9_-]+):\s*(.+?)\s*$", line)
        if not found or line.strip().startswith("#"):
            continue
        if found.group(1):
            entries.append({})
        if entries:
            entries[-1][found.group(2)] = found.group(3)
    return entries


# --- DIST-R1 -------------------------------------------------------------------------------

def check_r1(problems: list[str], tag: str | None) -> None:
    crates = members()
    version = sciart_version()
    internal = internal_version(crates)
    if not SEMVER.match(version):
        problems.append(f"DIST-R1: the version of sciart is not semver: {version!r}")
    if not isinstance(internal, str) or not SEMVER.match(internal) or not internal.startswith("0."):
        problems.append(f"DIST-R1 / DIST-R3: the internal crates' version is a 0.x semver: {internal!r}")
    if crates[FACADE][1]["package"].get("version") != {"workspace": True}:
        problems.append("DIST-R1: sciart takes `version.workspace = true` (the root Cargo.toml)")
    for name in INTERNAL:
        got = crates[name][1]["package"].get("version")
        if got != internal:
            problems.append(f"DIST-R1: {name} has version {got!r}, not the internal version {internal!r}")
    workspace_deps = toml_file("Cargo.toml")["workspace"]["dependencies"]
    for name in INTERNAL:
        spec = workspace_deps.get(name, {})
        if not isinstance(spec, dict) or spec.get("version") != f"={internal}":
            problems.append(f"DIST-R1: the workspace pins {name} with version = \"={internal}\"")
    for name in NEVER_PUBLISHED:
        spec = workspace_deps.get(name)
        if isinstance(spec, dict) and "version" in spec:
            problems.append(f"DIST-R3: {name} is never published, so its workspace entry has no version")
    pixi = toml_file("pixi.toml")
    if "version" in pixi.get("workspace", pixi.get("project", {})):
        problems.append("DIST-R1: pixi.toml carries no version of its own")
    if 'version: ${{ env.get("SCIART_VERSION") }}' not in (ROOT / "packaging/conda/recipe.yaml").read_text():
        problems.append("DIST-R1: the conda recipe takes its version from SCIART_VERSION")
    expected = {FACADE: version} | {n: internal for n in INTERNAL} | {n: version for n in NEVER_PUBLISHED}
    for lock, names in (("Cargo.lock", expected), ("fuzz/Cargo.lock", {"sciart-core": internal,
                                                                        "sciart-render": internal})):
        path = ROOT / lock
        if not path.exists():
            continue
        packages = tomllib.loads(path.read_text()).get("package", [])
        found = {p["name"]: p["version"] for p in packages if "source" not in p}
        for name, want in names.items():
            if name in found and found[name] != want:
                problems.append(f"DIST-R1: {lock} has {name} {found[name]}, not {want} "
                                f"(`scripts/bump_version.py` updates it)")
    if tag is not None:
        if tag != f"v{version}":
            problems.append(f"DIST-R1: tag {tag} is not v{version}, the version in Cargo.toml")
        heading = re.compile(rf"^## \[{re.escape(version)}\] - \d{{4}}-\d{{2}}-\d{{2}}$", re.M)
        if not heading.search((ROOT / "CHANGELOG.md").read_text()):
            problems.append(f"DIST-R1: CHANGELOG.md has no `## [{version}] - YYYY-MM-DD` heading")


# --- DIST-R3 -------------------------------------------------------------------------------

def check_r3(problems: list[str]) -> None:
    crates = members()
    published = {n for n, (_, m) in crates.items() if m["package"].get("publish", True) is not False}
    if published != set(PUBLISH_ORDER):
        problems.append(f"DIST-R3: the published crates are sciart and its {len(INTERNAL)} internal crates; "
                        f"extra {sorted(published - set(PUBLISH_ORDER))}, "
                        f"missing {sorted(set(PUBLISH_ORDER) - published)}")
    for name in NEVER_PUBLISHED:
        if name in crates and crates[name][1]["package"].get("publish") is not False:
            problems.append(f"DIST-R3: {name} has publish = false")
    if (ROOT / "fuzz/Cargo.toml").exists() and toml_file("fuzz/Cargo.toml")["package"].get("publish") is not False:
        problems.append("DIST-R3: fuzz/ has publish = false")
    # Everything published is needed by sciart, and nothing it needs is unpublished.
    reach, todo = set(), [FACADE]
    while todo:
        for dep in deps(crates[todo.pop()][1]):
            if dep in crates and dep not in reach:
                reach.add(dep)
                todo.append(dep)
    if reach != set(INTERNAL):
        problems.append(f"DIST-R3: sciart needs exactly the internal crates; it needs {sorted(reach)}")
    for name in PUBLISH_ORDER:
        for dep in deps(crates[name][1]):
            if dep in NEVER_PUBLISHED:
                problems.append(f"DIST-R3: published {name} depends on the unpublished {dep}")
    for name in INTERNAL:
        folder, manifest = crates[name]
        package = manifest["package"]
        if not str(package.get("description", "")).startswith("Internal:"):
            problems.append(f"DIST-R3: the description of {name} starts with `Internal:`")
        for key in ("keywords", "categories"):
            if key in package:
                problems.append(f"DIST-R3: {name} has no {key}")
        if package.get("readme") != "README.md":
            problems.append(f"DIST-R3: {name} has its own readme = \"README.md\"")
        readme = folder / "README.md"
        if not readme.is_file() or not all(m in readme.read_text() for m in NOTICE):
            problems.append(f"DIST-R3: {name}/README.md says it is an internal crate with no semver promise")
        doc = "".join(line + "\n" for line in (folder / "src/lib.rs").read_text().splitlines()
                      if line.startswith("//!"))
        has_notice = all(m in doc for m in NOTICE)
        if name in PENDING_NOTICE:
            if has_notice:
                problems.append(f"DIST-R3: {name} has its crate-level notice; remove it from PENDING_NOTICE")
        elif not has_notice:
            problems.append(f"DIST-R3: the crate docs of {name} (src/lib.rs `//!`) carry the internal notice")
    for name in PUBLISH_ORDER:
        docs = crates[name][1]["package"].get("metadata", {}).get("docs", {}).get("rs", {})
        if docs.get("all-features") or "mouse" in docs.get("features", []):
            problems.append(f"DIST-R3: the docs.rs build of {name} never enables `mouse`")
    facade_docs = crates[FACADE][1]["package"].get("metadata", {}).get("docs", {}).get("rs")
    if not facade_docs:
        problems.append("DIST-R3: sciart has [package.metadata.docs.rs]")


# --- DIST-R4 -------------------------------------------------------------------------------

def check_r4(problems: list[str]) -> None:
    crates = members()
    for name in PUBLISH_ORDER:
        folder, manifest = crates[name]
        exclude = manifest["package"].get("exclude", [])
        wanted = EXCLUDED if name in INTERNAL else ("tests/",)
        for path in wanted:
            if path not in exclude:
                problems.append(f"DIST-R4: {name} excludes `{path}` from the published crate")
        links = ("LICENSE", "CHANGELOG.md") if name == FACADE else ("LICENSE",)
        for link in links:
            path = folder / link
            if not path.is_file() or path.resolve() != (ROOT / link).resolve():
                problems.append(f"DIST-R4: {name}/{link} is a link to the root {link}")
    readme = crates[FACADE][1]["package"].get("readme")
    if not readme or (crates[FACADE][0] / readme).resolve() != (ROOT / "README.md").resolve():
        problems.append("DIST-R4: sciart ships the root README.md")
    for i, name in enumerate(PUBLISH_ORDER):
        later = set(PUBLISH_ORDER[i + 1:])
        for dep in deps(crates[name][1]) & later:
            problems.append(f"DIST-R4: {name} depends on {dep}, which is published after it")
    task = toml_file("pixi.toml").get("tasks", {}).get("rs-package-check")
    if task != PACKAGE_CHECK:
        problems.append(f"DIST-R4: pixi task rs-package-check is `{PACKAGE_CHECK}`")


def check_crates(problems: list[str], folder: Path) -> None:
    """DIST-R4 on the packaged .crate files."""
    crates = members()
    version, internal = sciart_version(), internal_version(crates)
    rows = []
    for name in PUBLISH_ORDER:
        ver = version if name == FACADE else internal
        # `cargo package` leaves the files in DIR; `cargo publish --workspace --dry-run`
        # in DIR/tmp-crate.
        candidates = [folder / f"{name}-{ver}.crate", folder / "tmp-crate" / f"{name}-{ver}.crate"]
        path = next((c for c in candidates if c.is_file()), candidates[0])
        if not path.is_file():
            problems.append(f"DIST-R4: no {path} (run `cargo publish --workspace --dry-run --locked`)")
            continue
        size = path.stat().st_size
        rows.append((name, size))
        if size > MAX_CRATE_BYTES:
            problems.append(f"DIST-R4: {path.name} is {size} bytes, over {MAX_CRATE_BYTES}")
        with tarfile.open(path) as tar:
            files = {m.name.split("/", 1)[1] for m in tar.getmembers() if "/" in m.name}
        if name in INTERNAL:
            shipped = sorted(f for f in files if f.startswith(EXCLUDED))
            if shipped:
                problems.append(f"DIST-R4: {path.name} ships {shipped[:3]}")
        else:
            for doc in ("LICENSE", "README.md", "CHANGELOG.md"):
                if doc not in files:
                    problems.append(f"DIST-R4: {path.name} lacks {doc}")
    for name, size in rows:
        print(f"DIST-R4: {name:24} {size:>9} bytes")


# --- DIST-R5 / DIST-R6 ---------------------------------------------------------------------

def render_binstall(template: str, version: str, target: str, repo: str) -> str:
    values = {"repo": repo, "name": "sciart", "version": version, "target": target,
              "archive-suffix": ".tar.gz", "bin": "sciart", "binary-ext": ""}
    return re.sub(r"\{\s*([a-z-]+)\s*\}", lambda m: values.get(m.group(1), m.group(0)), template)


def check_r5_r6(problems: list[str]) -> None:
    if set(release_assets.TIER_1) != TIER_1 or set(release_assets.TIER_2) != TIER_2:
        problems.append("DIST-R5: release_assets.py and tooling_check.py agree on the platform tiers")
    text = (ROOT / ".github/workflows/release.yml").read_text()
    jobs = workflow_jobs(text)
    build = jobs.get("build", "")
    entries = matrix_entries(build)
    by_target = {e.get("target"): e for e in entries}
    if set(by_target) != set(release_assets.TARGETS) or len(entries) != len(release_assets.TARGETS):
        problems.append(f"DIST-R5: release.yml builds exactly the six targets: {sorted(by_target, key=str)}")
    for target, entry in by_target.items():
        if target is None:
            continue
        os_label = entry.get("os", "")
        arch = target.split("-")[0]
        native = runner_target(os_label)
        if target.endswith("-apple-darwin"):
            if not os_label.startswith("macos-"):
                problems.append(f"DIST-R5: {target} builds on a macOS runner, not {os_label}")
        else:
            want = OLDEST_UBUNTU + ("-arm" if arch == "aarch64" else "")
            if os_label != want:
                problems.append(f"DIST-R5: {target} builds on {want} (glibc 2.35), not {os_label}")
        runs_here = native is not None and native.split("-")[0] == arch and \
            native.endswith("darwin") == target.endswith("darwin")
        if (entry.get("smoke") == "true") != runs_here:
            problems.append(f"DIST-R5: {target} is smoke-tested exactly when it runs on its runner "
                            f"({os_label}): smoke = {entry.get('smoke')}")
    build_steps = steps(build)
    command = [s for s in build_steps if "cargo auditable build" in s]
    if len(command) != 1 or f"run: {BUILD}" not in command[0]:
        problems.append(f"DIST-R5 / DIST-R6: the build step runs `{BUILD}` (default features)")
    elif "SCIART_GIT_SHA: ${{ github.sha }}" not in command[0]:
        problems.append("DIST-R2 / DIST-R5: the release build is given SCIART_GIT_SHA")
    for step in build_steps:
        if "cargo auditable build" in step and re.search(r"--(no-default-|all-)?features", step):
            problems.append("DIST-R5: release binaries are built with the default features")
    smoke = next((s for s in build_steps if "if: matrix.smoke" in s), "")
    for probe in ('"$bin" -V', '"$bin" --version', '"$bin" list --json'):
        if probe not in smoke:
            problems.append(f"DIST-R5: the smoke test runs `{probe}`")
    size = next((s for s in build_steps if "size_check.py" in s), "")
    if "matrix.target == 'aarch64-apple-darwin'" not in size or "--build lean" not in size \
            or "--build default" not in size:
        problems.append("DIST-R5 / RT-R22: the aarch64-apple-darwin build checks both size budgets")
    package = jobs.get("package", "")
    if "scripts/release_assets.py" not in package or \
            "--tool bins/x86_64-unknown-linux-gnu/sciart" not in package:
        problems.append("DIST-R5: the package job packs with release_assets.py, generating the "
                        "completions and manual with the x86_64 Linux binary")
    # DIST-R6
    if not re.search(r"^      id-token: write$", package, re.M) or \
            not re.search(r"^      attestations: write$", package, re.M):
        problems.append("DIST-R6: the package job may write attestations (id-token, attestations)")
    if not re.search(r"uses: actions/attest-build-provenance@v\d+\n\s+with:\n\s+subject-path: dist/\*\.tar\.gz",
                     package):
        problems.append("DIST-R6: every tarball gets a build-provenance attestation")
    if "sha256sum --check --strict SHA256SUMS" not in package:
        problems.append("DIST-R6: the package job verifies SHA256SUMS")
    check_assets(problems)
    # binstall (DIST-R5)
    manifest = members()[FACADE][1]
    binstall = manifest["package"].get("metadata", {}).get("binstall", {})
    repo = toml_file("Cargo.toml")["workspace"]["package"]["repository"]
    if binstall.get("pkg-fmt") != "tgz":
        problems.append("DIST-R5: binstall pkg-fmt is tgz")
    for target in release_assets.TARGETS:
        url = render_binstall(binstall.get("pkg-url", ""), "1.2.3", target, repo)
        want = f"{repo}/releases/download/v1.2.3/{release_assets.asset_name('1.2.3', target)}"
        if url != want:
            problems.append(f"DIST-R5: binstall pkg-url gives {url}, not {want}")
        path = render_binstall(binstall.get("bin-dir", ""), "1.2.3", target, repo)
        if path != f"{release_assets.asset_dir('1.2.3', target)}/sciart":
            problems.append(f"DIST-R5: binstall bin-dir gives {path}")


def check_assets(problems: list[str]) -> None:
    """Pack a fixture release with a fake tool: the layout of DIST-R5, the sums of DIST-R6."""
    with tempfile.TemporaryDirectory(prefix="sciart-assets-") as tmp:
        work = Path(tmp)
        tool = work / "tool"
        tool.write_text("#!/bin/sh\necho \"generated $*\"\n")
        tool.chmod(0o755)
        for target in release_assets.TARGETS:
            (work / "bins" / target).mkdir(parents=True)
            (work / "bins" / target / "sciart").write_bytes(b"\x7fELF fake")
        try:
            names = release_assets.build("1.2.3", work / "bins", tool, work / "out",
                                         list(release_assets.TARGETS), 0)
        except (OSError, ValueError, subprocess.CalledProcessError) as e:
            problems.append(f"DIST-R5: release_assets.py fails on a fixture: {e}")
            return
        sums = render_formula.parse_sums((work / "out" / release_assets.SUMS).read_text())
        if set(sums) != set(names) - {release_assets.SUMS}:
            problems.append(f"DIST-R6: SHA256SUMS covers every other asset: {sorted(sums)} vs {names}")
        if "install.sh" not in sums:
            problems.append("DIST-R6: install.sh is a release asset listed in SHA256SUMS")
        top = release_assets.asset_dir("1.2.3", "x86_64-unknown-linux-gnu")
        with tarfile.open(work / "out" / release_assets.asset_name("1.2.3", "x86_64-unknown-linux-gnu")) as tar:
            files = {m.name for m in tar.getmembers() if m.isfile()}
            mode = tar.getmember(f"{top}/sciart").mode
        want = {f"{top}/{n}" for n in ("sciart", *release_assets.DOCS, "man/sciart.1")} | \
            {f"{top}/completions/{n}" for n in release_assets.COMPLETIONS.values()}
        if not want <= files:
            problems.append(f"DIST-R5: a tarball holds {sorted(want - files)}")
        if not release_assets.man_subcommands() or \
                not any(f.startswith(f"{top}/man/sciart-") for f in files):
            problems.append("DIST-R5: the tarball has the subcommands' manual pages")
        if mode & 0o111 == 0:
            problems.append("DIST-R5: the packed binary is executable")
        again = release_assets.pack("1.2.3", "x86_64-unknown-linux-gnu", b"\x7fELF fake",
                                    {"man/sciart.1": b"x"}, 0)
        if again != release_assets.pack("1.2.3", "x86_64-unknown-linux-gnu", b"\x7fELF fake",
                                        {"man/sciart.1": b"x"}, 0):
            problems.append("DIST-R5: packing is reproducible")


# --- DIST-R8 -------------------------------------------------------------------------------

def check_r8(problems: list[str]) -> None:
    recipe = (ROOT / "packaging/conda/recipe.yaml").read_text()
    for needle, why in (
        ("  name: sciart\n", "the package is named sciart"),
        ('${{ compiler("rust") }}', "it builds with the conda Rust compiler"),
        ("cargo-bundle-licenses", "it bundles the third-party licences"),
        ("    - THIRDPARTY.yml\n", "the bundled licences are a licence file"),
        ("    - LICENSE\n", "sciart's licence is a licence file"),
        ("--locked", "it builds with the lock file"),
        ("SCIART_GIT_SHA", "--version names the commit (DIST-R2)"),
        ("      - sciart --version\n", "it tests `sciart --version`"),
        ("      - sciart list\n", "it tests `sciart list`"),
        ("  path: ../..\n", "it builds this checkout"),
    ):
        if needle not in recipe:
            problems.append(f"DIST-R8: recipe.yaml: {why} (`{needle.strip()}`)")
    pixi = toml_file("pixi.toml")
    feature = pixi.get("feature", {}).get("dist", {})
    if "rattler-build" not in feature.get("dependencies", {}):
        problems.append("DIST-R8: the pixi `dist` feature provides rattler-build")
    if feature.get("tasks", {}).get("rs-conda-build") != "python3 scripts/conda_build.py":
        problems.append("DIST-R8: `rs-conda-build` (dist feature) runs scripts/conda_build.py")
    if "dist" not in pixi.get("environments", {}):
        problems.append("DIST-R8: pixi has a `dist` environment")
    for name, task in [*pixi.get("tasks", {}).items(), *feature.get("tasks", {}).items()]:
        if "upload" in str(task):
            problems.append(f"DIST-R8 / DIST-R10: pixi task {name} uploads; only the publish workflow does")
    conda = workflow_jobs((ROOT / ".github/workflows/publish.yml").read_text()).get("conda", "")
    platforms = {e.get("platform") for e in matrix_entries(conda)}
    if platforms != CONDA_PLATFORMS:
        problems.append(f"DIST-R8: publish.yml builds the conda package for {sorted(CONDA_PLATFORMS)}: "
                        f"{sorted(map(str, platforms))}")
    if "rs-conda-build --target-platform ${{ matrix.platform }}" not in conda:
        problems.append("DIST-R8: publish.yml builds each platform with rs-conda-build")


# --- DIST-R9 -------------------------------------------------------------------------------

def check_r9(problems: list[str], require_ruby: bool) -> None:
    fake = {release_assets.asset_name("1.2.3", t): f"{i:064x}" for i, t in enumerate(release_assets.TARGETS)}
    sums = "".join(f"{h}  {n}\n" for n, h in fake.items())
    try:
        formula = render_formula.render("1.2.3", sums)
    except ValueError as e:
        problems.append(f"DIST-R9: the formula does not render from a complete SHA256SUMS: {e}")
        return
    for target in render_formula.FORMULA_TARGETS:
        name = release_assets.asset_name("1.2.3", target)
        if f'url "{render_formula.BASE}/download/v1.2.3/{name}"\n      sha256 "{fake[name]}"' not in formula:
            problems.append(f"DIST-R9: the formula downloads {name} with its checksum")
    for block in ("on_macos do", "on_linux do", "on_arm do", "on_intel do"):
        if block not in formula:
            problems.append(f"DIST-R9: the formula has `{block}`")
    installs = ['bin.install "sciart"', 'man1.install Dir["man/*.1"]',
                f'bash_completion.install "completions/{release_assets.COMPLETIONS["bash"]}" => "sciart"',
                f'zsh_completion.install "completions/{release_assets.COMPLETIONS["zsh"]}"',
                f'fish_completion.install "completions/{release_assets.COMPLETIONS["fish"]}"']
    for line in installs:
        if line not in formula:
            problems.append(f"DIST-R9: the formula runs `{line}`")
    if not re.search(r'test do\n\s+assert_match "sciart #\{version\}", shell_output\("#\{bin\}/sciart --version"\)',
                     formula):
        problems.append("DIST-R9: the formula's test checks `sciart --version`")
    partial = "".join(line for line in sums.splitlines(keepends=True) if "x86_64-apple-darwin" not in line)
    try:
        render_formula.render("1.2.3", partial)
        problems.append("DIST-R9: rendering refuses a SHA256SUMS that lacks a formula target")
    except ValueError:
        pass
    ruby = shutil.which("ruby")
    if ruby is None:
        if require_ruby:
            problems.append("DIST-R9: ruby is not installed (needed for `ruby -c` on the formula)")
        else:
            print("DIST-R9: ruby not installed; `ruby -c` skipped (the release workflow requires it)")
        return
    with tempfile.NamedTemporaryFile("w", suffix=".rb", delete=False) as f:
        f.write(formula)
    run = subprocess.run([ruby, "-c", f.name], capture_output=True, text=True)
    Path(f.name).unlink()
    if run.returncode != 0:
        problems.append(f"DIST-R9: `ruby -c` rejects the formula: {run.stderr.strip()}")


# --- DIST-R10 ------------------------------------------------------------------------------

def on_block(text: str) -> str:
    found = re.search(r"^on:\n((?:  .*\n|\n)*)", text, re.M)
    return found.group(1) if found else ""


def check_r10(problems: list[str]) -> None:
    release = (ROOT / ".github/workflows/release.yml").read_text()
    publish = (ROOT / ".github/workflows/publish.yml").read_text()
    triggers = set(re.findall(r"^  ([a-z_]+):", on_block(release), re.M))
    if triggers != {"push", "workflow_dispatch"} or 'tags: ["v*"]' not in on_block(release):
        problems.append(f"DIST-R10: release.yml runs on a pushed v* tag and by hand only: {sorted(triggers)}")
    if not re.search(r"^permissions:\n  contents: read$", release, re.M) or \
            not re.search(r"^permissions:\n  contents: read$", publish, re.M):
        problems.append("DIST-R10: both workflows default to read-only permissions")
    jobs = workflow_jobs(release)
    draft = jobs.get("release", "")
    if "if: startsWith(github.ref, 'refs/tags/v')" not in draft:
        problems.append("DIST-R10: the release job runs only for a tag")
    create = [s for s in steps(draft) if "gh release create" in s]
    if len(create) != 1 or "--draft" not in create[0] or "--verify-tag" not in create[0]:
        problems.append("DIST-R10: the release job creates a draft (`--draft`) on the pushed tag (`--verify-tag`)")
    for name, body in jobs.items():
        if name != "release" and "contents: write" in body:
            problems.append(f"DIST-R10: only the release job of release.yml writes contents ({name})")
    if not re.search(r"^  release:\n    types: \[published\]$", on_block(publish), re.M):
        problems.append("DIST-R10: publish.yml runs when a release is published")
    triggers = set(re.findall(r"^  ([a-z_]+):", on_block(publish), re.M))
    if triggers != {"release", "workflow_dispatch"}:
        problems.append(f"DIST-R10: publish.yml runs on a published release or by hand only: {sorted(triggers)}")
    pub_jobs = workflow_jobs(publish)
    for name in ("crates-io", "conda", "homebrew"):
        body = pub_jobs.get(name)
        if body is None:
            problems.append(f"DIST-R10: publish.yml has a `{name}` job")
        elif not re.search(r"^    environment: release$", body, re.M):
            problems.append(f"DIST-R10: publish.yml `{name}` runs in the `release` environment")
    for name, body in pub_jobs.items():
        gated = re.search(r"^    environment: release$", body, re.M) is not None
        for step in steps(body):
            if any(u in step for u in UPLOADS):
                if not gated or "if: github.event_name == 'release'" not in step:
                    problems.append(f"DIST-R10: publish.yml `{name}` uploads only on a published release, "
                                    f"in the `release` environment")
    for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
        text = workflow.read_text()
        for pattern, why in ((r"\bgit tag\b", "creates a tag"), (r"git push[^\n]*(--tags|refs/tags| v\d)", "pushes a tag"),
                             (r"softprops/action-gh-release", "publishes a release directly"),
                             (r"gh release (edit|create)[^\n]*--draft=false", "publishes a release")):
            if re.search(pattern, text):
                problems.append(f"DIST-R10: {workflow.name} {why}")
        if workflow.name != "publish.yml":
            for upload in UPLOADS:
                if upload in text:
                    problems.append(f"DIST-R10: {workflow.name} runs `{upload}`; only publish.yml uploads")


# --- modes ---------------------------------------------------------------------------------

def notes(version: str) -> str | None:
    text = (ROOT / "CHANGELOG.md").read_text()
    found = re.search(rf"^## \[{re.escape(version)}\] - [^\n]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S)
    return found.group(1).strip() + "\n" if found else None


def main() -> int:
    args = sys.argv[1:]

    def value(flag: str) -> str | None:
        if flag in args:
            i = args.index(flag)
            if i + 1 >= len(args):
                raise SystemExit(f"{flag} needs a value")
            return args[i + 1]
        return None

    if "--print-version" in args:
        print(sciart_version())
        return 0
    if (version := value("--notes")) is not None:
        text = notes(version)
        if text is None:
            print(f"DIST-R1: CHANGELOG.md has no section for {version}", file=sys.stderr)
            return 1
        sys.stdout.write(text)
        return 0
    problems: list[str] = []
    crates_dir = value("--crates")
    if crates_dir is not None:
        check_crates(problems, ROOT / crates_dir)
        label = "packaged crates"
    else:
        check_r1(problems, value("--tag"))
        check_r3(problems)
        check_r4(problems)
        check_r5_r6(problems)
        check_r8(problems)
        check_r9(problems, "--require-ruby" in args)
        check_r10(problems)
        label = "release configuration"
    if problems:
        print(f"{label}:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"{label} OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
