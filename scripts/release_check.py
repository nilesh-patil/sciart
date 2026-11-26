#!/usr/bin/env python3
"""Release configuration checks (SPEC-008 DIST-R1, DIST-R3..DIST-R6, DIST-R9, DIST-R10).

Without arguments it checks, from the files alone:

- DIST-R1 one version: the root Cargo.toml's version of `sciart`, the internal crates'
  shared 0.x version and their `=` pins, Cargo.lock, no version in pixi.toml;
- DIST-R3 the published set and the labels of the internal crates;
- DIST-R4 the packaging rules (excludes, licence and changelog links, publish order);
- DIST-R5 / DIST-R6 the packing of a fixture release and the binstall metadata;
- DIST-R9 the Homebrew formula rendered from a fixture (and `ruby -c` when Ruby is
  installed; `--require-ruby` makes it mandatory);
- DIST-R5 / DIST-R6 / DIST-R10 every workflow, read as data
  (scripts/release_workflows.py): pinned actions, secrets on steps only, no event data in
  scripts, the release chain and its checks, and every upload behind the gated publish
  step; and no pixi task that uploads.

Other modes:

    --tag v1.2.3        also: the tag is v{version} and CHANGELOG.md has its heading (DIST-R1)
    --crates DIR        the .crate files `cargo publish --dry-run` left in DIR (DIST-R4)
    --notes 1.2.3       print the CHANGELOG section of a version (the release notes)
    --print-version     print the version of `sciart`
"""

from __future__ import annotations

import os
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
import release_workflows  # noqa: E402
from tooling_check import TIER_1, TIER_2  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
FACADE = "sciart"
# DIST-R4: the publish order, each crate after every workspace crate it needs from the
# registry, dev-dependencies included (`publish_order_problems` checks it against the
# manifests): sciart-generators' tests use sciart-render and sciart-capture.
INTERNAL = (
    "sciart-core",
    "sciart-shared",
    "sciart-scenes-cosmos",
    "sciart-scenes-earth",
    "sciart-scenes-physics",
    "sciart-scenes-matter",
    "sciart-scenes-life",
    "sciart-render",
    "sciart-keepawake",
    "sciart-capture",
    "sciart-generators",
)
PUBLISH_ORDER = INTERNAL + (FACADE,)
NEVER_PUBLISHED = ("sciart-testalloc", "sciart-qa")
# Internal crates whose crate-level doc notice is not written yet. May only shrink.
PENDING_NOTICE: set[str] = set()
NOTICE = ("Internal crate of", "no semver promise")
EXCLUDED = ("tests/", "benches/", "examples/")
MAX_CRATE_BYTES = 1 << 20
SEMVER = release_assets.SEMVER
PACKAGE_CHECK = ("cargo publish --workspace --dry-run --locked && "
                 "python3 scripts/release_check.py --crates target/package")


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


LOCK_SKIP = {"target", ".git", ".pixi", "node_modules"}


def lock_files(root: Path) -> list[Path]:
    """Every Cargo.lock in the repository that names a sciart crate (the workspace's,
    fuzz/'s, web/'s, ...), found by search, not by a list."""
    found = []
    for folder, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in LOCK_SKIP)
        if "Cargo.lock" in files:
            path = Path(folder) / "Cargo.lock"
            if 'name = "sciart' in path.read_text():
                found.append(path)
    return sorted(found)


def lock_versions(version: str, internal: str) -> dict[str, str]:
    """The version every workspace crate has in a lock file (as a path package)."""
    return {FACADE: version} | {n: internal for n in INTERNAL} | {n: version for n in NEVER_PUBLISHED}


def deps(manifest: dict, kinds: tuple[str, ...] = ("dependencies", "build-dependencies")) -> set[str]:
    """Names of a manifest's dependencies of `kinds`, target-specific ones included."""
    names: set[str] = set()
    tables = [manifest] + list(manifest.get("target", {}).values())
    for table in tables:
        for kind in kinds:
            names |= set(table.get(kind, {}))
    return names


DEP_KINDS = ("dependencies", "build-dependencies", "dev-dependencies")


def versioned_deps(manifest: dict, workspace: dict, members: set[str]) -> set[str]:
    """The workspace crates `manifest` depends on with a version requirement, in any of
    `DEP_KINDS`, target-specific tables included. `workspace = true` reads the workspace
    dependency table. A path-only dependency has no version: cargo strips it from the
    published manifest, so it needs nothing on the registry."""
    out: set[str] = set()
    tables = [manifest] + [t for t in manifest.get("target", {}).values() if isinstance(t, dict)]
    for table in tables:
        for kind in DEP_KINDS:
            for key, spec in table.get(kind, {}).items():
                if isinstance(spec, dict) and spec.get("workspace") is True:
                    inherited = workspace.get(key, {})
                    spec = dict(inherited) if isinstance(inherited, dict) else {"version": inherited}
                name = spec.get("package", key) if isinstance(spec, dict) else key
                has_version = isinstance(spec, str) or (isinstance(spec, dict) and "version" in spec)
                if name in members and has_version:
                    out.add(name)
    return out


def publish_order_problems(order: tuple[str, ...], manifests: dict[str, dict], workspace: dict) -> list[str]:
    """DIST-R4: each published crate comes after every workspace crate it needs from the
    registry: normal, build and dev-dependencies with a version (`cargo publish` resolves
    all three against the registry; a workspace dry run resolves them locally and cannot
    see a wrong order). `manifests` maps every workspace crate to its manifest."""
    problems = []
    published = {n for n, m in manifests.items() if m.get("package", {}).get("publish", True) is not False}
    for name in sorted(published - set(order)):
        problems.append(f"DIST-R4: {name} is published but not in the publish order")
    edges = {name: versioned_deps(manifests[name], workspace, set(manifests)) for name in published}
    position = {name: i for i, name in enumerate(order)}
    for name in order:
        for dep in sorted(edges.get(name, set())):
            if dep not in published:
                problems.append(f"DIST-R4: published {name} depends with a version on {dep}, which is "
                                "never published (a path-only dependency would be stripped)")
            elif dep in position and position[dep] > position[name]:
                problems.append(f"DIST-R4: {name} depends on {dep}, which is published after it")
    # Cycles: a crate that needs itself through others can be published in no order.
    state: dict[str, int] = {}
    stack: list[str] = []
    cycles: list[list[str]] = []

    def visit(node: str) -> None:
        state[node] = 1
        stack.append(node)
        for dep in sorted(edges.get(node, set()) & published):
            if state.get(dep) == 1:
                cycles.append(stack[stack.index(dep):] + [dep])
            elif dep not in state:
                visit(dep)
        stack.pop()
        state[node] = 2

    for node in sorted(published):
        if node not in state:
            visit(node)
    for cycle in cycles:
        problems.append(f"DIST-R4: a dependency cycle among published crates: {' -> '.join(cycle)}")
    return problems


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
    expected = lock_versions(version, str(internal))
    locks = lock_files(ROOT)
    if ROOT / "Cargo.lock" not in locks:
        problems.append("DIST-R1: the workspace Cargo.lock names the sciart crates")
    for path in locks:
        packages = tomllib.loads(path.read_text()).get("package", [])
        found = {p["name"]: p["version"] for p in packages if "source" not in p}
        for name, want in expected.items():
            if name in found and found[name] != want:
                problems.append(f"DIST-R1: {path.relative_to(ROOT)} has {name} {found[name]}, not {want} "
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
    workspace = toml_file("Cargo.toml")["workspace"]["dependencies"]
    problems += publish_order_problems(PUBLISH_ORDER, {n: m for n, (_, m) in crates.items()}, workspace)
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
    for bad in ("1.2", "1.2.3\" do system 'x' end #", "v1.2.3", "1.2.3+build", "01.2.3", ""):
        try:
            render_formula.render(bad, sums)
            problems.append(f"DIST-R9: rendering refuses the version {bad!r} (not a semantic version)")
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

def check_r10(problems: list[str]) -> None:
    """DIST-R5 / R6 / R10 in the workflows, read as data (release_workflows.py); no pixi
    task uploads anything (only the publish workflow does)."""
    problems += release_workflows.problems(release_workflows.load(ROOT))
    pixi = toml_file("pixi.toml")
    tasks = [*pixi.get("tasks", {}).items()]
    for feature in pixi.get("feature", {}).values():
        tasks += [*feature.get("tasks", {}).items()]
    for name, task in tasks:
        if "upload" in str(task):
            problems.append(f"DIST-R10: pixi task {name} uploads; only the publish workflow does")


# --- modes ---------------------------------------------------------------------------------

# DIST-R10: Markdown link targets, inline (`[text](target "title")`) and reference
# definitions (`[label]: target`); code spans and fenced blocks are left as they are.
INLINE_LINK = re.compile(r"(\]\()([^)\s]+)((?:\s+\"[^\"]*\")?\))")
REFERENCE = re.compile(r"^(\s{0,3}\[[^\]\n]+\]:\s*)(\S+)", re.M)
CODE = re.compile(r"(```.*?```|`[^`\n]*`)", re.S)


def is_relative(target: str) -> bool:
    """A link target that resolves against the page it is on: not a URL with a scheme
    (`https:`, `mailto:`, ...), not protocol-relative, not an anchor on the same page."""
    return not re.match(r"[A-Za-z][A-Za-z0-9+.-]*:|//|#", target)


def pinned_links(text: str, repo: str, version: str) -> str:
    """`text` with every relative link target made absolute in the repository's tree at the
    tag `v<version>`, its `#fragment` kept: on a release page a relative link resolves under
    /releases/tag/ and is dead."""
    base = f"{repo.removesuffix('.git').rstrip('/')}/blob/v{version}/"

    def pin(target: str) -> str:
        if not is_relative(target):
            return target
        path = re.sub(r"^(\./)+", "", target).lstrip("/")
        return base + path

    def outside_code(part: str) -> str:
        part = INLINE_LINK.sub(lambda m: m.group(1) + pin(m.group(2)) + m.group(3), part)
        return REFERENCE.sub(lambda m: m.group(1) + pin(m.group(2)), part)

    return "".join(part if CODE.fullmatch(part) else outside_code(part) for part in CODE.split(text))


def relative_targets(text: str) -> list[str]:
    """The relative link targets left in `text` (outside code)."""
    prose = "".join(part for part in CODE.split(text) if not CODE.fullmatch(part))
    targets = [m.group(2) for m in INLINE_LINK.finditer(prose)] + [m.group(2) for m in REFERENCE.finditer(prose)]
    return [t for t in targets if is_relative(t)]


def notes(version: str) -> str | None:
    """The release notes of `version`: its CHANGELOG section, with relative links pinned to
    the tag (DIST-R10). The repository is the workspace manifest's `repository`."""
    text = (ROOT / "CHANGELOG.md").read_text()
    found = re.search(rf"^## \[{re.escape(version)}\] - [^\n]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S)
    if not found:
        return None
    repo = toml_file("Cargo.toml")["workspace"]["package"]["repository"]
    return pinned_links(found.group(1).strip(), repo, version) + "\n"


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
