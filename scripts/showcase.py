#!/usr/bin/env python3
"""Record and check the animated showcase from docs/showcase.toml (WEB-R1, WEB-R2).

Every animated capture in the README and the gallery is a clip in the manifest, recorded by
`sciart record` with exactly the manifest's parameters:

    python3 scripts/showcase.py            # record missing clips, then write docs/GALLERY.md
    python3 scripts/showcase.py --force    # re-record every clip, then write the gallery
    python3 scripts/showcase.py --check    # WEB-R2; exit 1 when anything is off

WEB-R1: the manifest lists each clip's id, scene, seed, size, start, seconds, fps, theme and
output path under docs/media/; a GIF is at most 1.5 MB and the set at most 16 MB; the set
covers at least 12 scenes from at least 8 categories, including the thread idiom (GEN-R25),
the orbital-thread stipple idiom (GEN-R26) and a half-block field scene.

WEB-R2 (`--check`): fails when a manifest clip is missing, a file under docs/media/ is not in
the manifest, a clip or the set is over its size limit, a manifest scene is not registered,
the WEB-R1 coverage is not met, or README.md or docs/GALLERY.md references a media file that
is not in the manifest. README.md shows the one `readme = "hero"` clip and every
`readme = "grid"` clip (at least eight), each with alt text naming the scene; docs/GALLERY.md
(written by scripts/gallery.py) shows every clip.

Clips are recorded through `cargo run --profile ci -p sciart --features record -- record`,
the same build as `pixi run rs-record`. This is the only script that writes docs/media/.
"""

from __future__ import annotations

import argparse
import html
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

import gallery

# Locate the repo root from this file: <root>/scripts/showcase.py.
ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "docs" / "showcase.toml"
MEDIA_DIR = ROOT / "docs" / "media"
README = ROOT / "README.md"
GALLERY = ROOT / "docs" / "GALLERY.md"
# GEN-R25 / GEN-R26 scene lists: the test suites' own source of truth.
IDIOM_LISTS = ROOT / "crates" / "sciart-generators" / "tests" / "common" / "gen_r25.rs"
SCENE_SOURCES = ROOT / "crates"

# WEB-R1 limits, in decimal megabytes (the stricter reading of "MB").
CLIP_LIMIT = 1_500_000
SET_LIMIT = 16_000_000
MIN_SCENES = 12
MIN_CATEGORIES = 8
MIN_GRID = 8

THEMES = ("astro", "rainbow", "mono", "aurora", "colorblind")
FIELDS = {
    "id": str,
    "scene": str,
    "seed": int,
    "size": str,
    "start": (int, float),
    "seconds": (int, float),
    "fps": int,
    "theme": str,
    "out": str,
    "alt": str,
}
OPTIONAL = {"readme": str}
READMES = ("hero", "grid")

RECORD = [
    "cargo",
    "run",
    "-q",
    "--profile",
    "ci",
    "-p",
    "sciart",
    "--features",
    "record",
    "--",
    "record",
]

# A media reference in Markdown or HTML: `docs/media/x.gif`, `media/x.gif`, `../docs/media/…`.
MEDIA_REF = re.compile(r"""(?P<path>(?:\.\./)*(?:docs/)?media/[^\s"'()<>]+)""")
# `<img … src="…" … alt="…">` in either attribute order, and `![alt](src)`.
IMG_TAG = re.compile(r"<img\b[^>]*>", re.IGNORECASE)
ATTR = re.compile(r"""(\w+)\s*=\s*"([^"]*)\"""")
MD_IMAGE = re.compile(r"!\[(?P<alt>[^\]]*)\]\((?P<src>[^)\s]+)\)")


@dataclass(frozen=True)
class Clip:
    id: str
    scene: str
    seed: int
    size: str
    start: float
    seconds: float
    fps: int
    theme: str
    out: str
    alt: str
    readme: str | None

    @property
    def path(self) -> Path:
        return ROOT / self.out

    def command(self) -> list[str]:
        """The exact `sciart record` invocation for this clip (WEB-R1)."""
        return [
            *RECORD,
            self.scene,
            "--out",
            str(self.path),
            "--seed",
            str(self.seed),
            "--size",
            self.size,
            "--start",
            f"{self.start:g}",
            "--seconds",
            f"{self.seconds:g}",
            "--fps",
            str(self.fps),
            "--theme",
            self.theme,
        ]


def fail(message: str) -> NoReturn:
    sys.exit(f"showcase: {message}")


def load_manifest() -> list[Clip]:
    """Parse and validate docs/showcase.toml; any malformed entry is fatal (WEB-R1)."""
    try:
        data = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as err:
        fail(f"cannot read {MANIFEST.relative_to(ROOT)}: {err}")
    entries = data.get("clip")
    if not isinstance(entries, list) or not entries:
        fail("the manifest has no [[clip]] entries")
    unknown_top = set(data) - {"clip"}
    if unknown_top:
        fail(f"unknown top-level key(s) {', '.join(sorted(unknown_top))}")
    clips = []
    problems = []
    for number, entry in enumerate(entries, 1):
        label = f"clip {number} ({entry.get('id', '?')})"
        extra = set(entry) - set(FIELDS) - set(OPTIONAL)
        if extra:
            problems.append(f"{label}: unknown key(s) {', '.join(sorted(extra))}")
        missing = [key for key in FIELDS if key not in entry]
        if missing:
            problems.append(f"{label}: missing {', '.join(missing)}")
            continue
        for key, kind in {**FIELDS, **OPTIONAL}.items():
            if key in entry and (
                not isinstance(entry[key], kind) or isinstance(entry[key], bool)
            ):
                problems.append(f"{label}: `{key}` has the wrong type")
        if problems:
            continue
        clip = Clip(
            id=entry["id"],
            scene=entry["scene"],
            seed=entry["seed"],
            size=entry["size"],
            start=float(entry["start"]),
            seconds=float(entry["seconds"]),
            fps=entry["fps"],
            theme=entry["theme"],
            out=entry["out"],
            alt=entry["alt"].strip(),
            readme=entry.get("readme"),
        )
        problems.extend(f"{label}: {p}" for p in validate(clip))
        clips.append(clip)
    for key in ("id", "out"):
        values = [getattr(clip, key) for clip in clips]
        repeated = sorted({v for v in values if values.count(v) > 1})
        if repeated:
            problems.append(f"repeated `{key}`: {', '.join(repeated)}")
    if problems:
        fail("bad manifest:\n  " + "\n  ".join(problems))
    return clips


def validate(clip: Clip) -> list[str]:
    """The `sciart record` ranges (CLI-R8) and the WEB-R1 output rules for one clip."""
    problems = []
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", clip.id):
        problems.append("`id` is lower-case letters, digits and dashes")
    match = re.fullmatch(r"([1-9][0-9]*)x([1-9][0-9]*)", clip.size)
    if not match:
        problems.append(f"`size` is WxH cells, not {clip.size!r}")
    if clip.fps < 1 or 60 % clip.fps:
        problems.append(f"`fps` must divide 60, not {clip.fps}")
    if not 0 <= clip.start <= 1200:
        problems.append("`start` is in 0..=1200 s")
    if not 0 < clip.seconds <= 60:
        problems.append("`seconds` is in (0, 60]")
    if clip.theme not in THEMES:
        problems.append(f"`theme` is one of {', '.join(THEMES)}")
    out = Path(clip.out)
    if out.is_absolute() or out.parent != Path("docs/media") or out.suffix != ".gif":
        problems.append(f"`out` is docs/media/<name>.gif, not {clip.out!r}")
    if clip.readme is not None and clip.readme not in READMES:
        problems.append(f"`readme` is one of {', '.join(READMES)} or absent")
    if not alt_names_scene(clip.alt, clip.scene):
        problems.append("`alt` names the scene and says what the clip shows")
    return problems


def scene_words(scene: str) -> str:
    return scene.replace("_", " ").lower()


def alt_names_scene(alt: str, scene: str) -> bool:
    """WEB-R2: alt text names the scene (its name as words) and says more than the name."""
    words = re.sub(r"[-_]", " ", alt).lower()
    name = scene_words(scene)
    return name in words and len(words) >= len(name) + 20


def record(clips: list[Clip], force: bool) -> list[tuple[str, str]]:
    """Record every missing (or, with `force`, every) clip. Returns failures."""
    MEDIA_DIR.mkdir(parents=True, exist_ok=True)
    todo = [clip for clip in clips if force or not clip.path.is_file()]
    if not todo:
        print(f"showcase: all {len(clips)} clips present")
        return []
    print(f"showcase: recording {len(todo)} clip(s)")
    failed = []
    for number, clip in enumerate(todo, 1):
        proc = subprocess.run(clip.command(), cwd=ROOT)
        if proc.returncode != 0 or not clip.path.is_file():
            failed.append((clip.id, f"sciart record exited {proc.returncode}"))
            print(f"  [{number}/{len(todo)}] {clip.id}: FAILED", file=sys.stderr)
            continue
        print(
            f"  [{number}/{len(todo)}] {clip.out} ({clip.path.stat().st_size / 1e6:.2f} MB)"
        )
    return failed


def idiom_lists() -> tuple[set[str], set[str]]:
    """The GEN-R25 (thread) and GEN-R26 (orbital-thread stipple) scene lists."""
    text = IDIOM_LISTS.read_text(encoding="utf-8")
    lists = []
    for name in ("GEN_R25", "GEN_R26"):
        match = re.search(rf"pub const {name}: &\[&str\] = &\[(.*?)\];", text, re.DOTALL)
        if not match:
            fail(f"cannot find `{name}` in {IDIOM_LISTS.relative_to(ROOT)}")
        lists.append(set(re.findall(r'"([a-z0-9_]+)"', match.group(1))))
    return lists[0], lists[1]


def is_half_block_field(scene: str, thread: set[str], stipple: set[str]) -> bool:
    """A scene outside both later idioms that draws with half-block `Pixels` (ADR-0010)."""
    if scene in thread or scene in stipple:
        return False
    sources = list(SCENE_SOURCES.glob(f"sciart-scenes-*/src/{scene}.rs"))
    return any("Pixels" in path.read_text(encoding="utf-8") for path in sources)


def media_refs(path: Path) -> set[Path]:
    """Every docs/media/ file `path` refers to, resolved against the file's directory."""
    refs = set()
    for match in MEDIA_REF.finditer(path.read_text(encoding="utf-8")):
        target = (path.parent / match.group("path")).resolve()
        if MEDIA_DIR.resolve() in target.parents:
            refs.add(target)
    return refs


def images(path: Path) -> list[tuple[Path, str]]:
    """`(resolved src, alt)` for every <img> tag and Markdown image in `path`."""
    text = path.read_text(encoding="utf-8")
    found = []
    for tag in IMG_TAG.findall(text):
        attrs = {key.lower(): value for key, value in ATTR.findall(tag)}
        if "src" in attrs:
            alt = html.unescape(attrs.get("alt", ""))
            found.append(((path.parent / html.unescape(attrs["src"])).resolve(), alt))
    for match in MD_IMAGE.finditer(text):
        found.append(((path.parent / match.group("src")).resolve(), match.group("alt")))
    return found


def check(clips: list[Clip], scenes: list[gallery.Scene]) -> int:
    """WEB-R2: the manifest, docs/media/, the README and the gallery agree (WEB-R1 limits)."""
    problems: list[str] = []
    registered = {scene.name: scene for scene in scenes}

    # Clips present, within the limits, and no orphans in docs/media/.
    total = 0
    for clip in clips:
        if not clip.path.is_file():
            problems.append(f"missing {clip.out} (clip `{clip.id}`)")
            continue
        size = clip.path.stat().st_size
        total += size
        if size > CLIP_LIMIT:
            problems.append(
                f"{clip.out} is {size / 1e6:.2f} MB, over the {CLIP_LIMIT / 1e6:.1f} MB clip limit"
            )
    if total > SET_LIMIT:
        problems.append(
            f"the set is {total / 1e6:.2f} MB, over the {SET_LIMIT / 1e6:.0f} MB limit"
        )
    wanted = {clip.path.resolve() for clip in clips}
    if MEDIA_DIR.is_dir():
        for path in sorted(MEDIA_DIR.rglob("*")):
            if path.is_file() and path.resolve() not in wanted:
                problems.append(f"orphan {path.relative_to(ROOT)} is not in the manifest")

    # Registered scenes and WEB-R1 coverage.
    unknown = sorted({clip.scene for clip in clips} - set(registered))
    for name in unknown:
        problems.append(f"manifest scene `{name}` is not registered")
    known = {clip.scene for clip in clips if clip.scene in registered}
    categories = {registered[name].category for name in known}
    thread, stipple = idiom_lists()
    fields = {name for name in known if is_half_block_field(name, thread, stipple)}
    if len(known) < MIN_SCENES:
        problems.append(f"coverage: {len(known)} scenes, want at least {MIN_SCENES}")
    if len(categories) < MIN_CATEGORIES:
        problems.append(
            f"coverage: {len(categories)} categories, want at least {MIN_CATEGORIES}"
        )
    if not known & thread:
        problems.append("coverage: no thread-idiom scene (GEN-R25)")
    if not known & stipple:
        problems.append("coverage: no orbital-thread stipple scene (GEN-R26)")
    if not fields:
        problems.append("coverage: no half-block field scene")

    # README and gallery: only manifest media, the hero and grid shown with alt text.
    for page in (README, GALLERY):
        if not page.is_file():
            problems.append(f"{page.relative_to(ROOT)} is missing")
            continue
        for ref in sorted(media_refs(page) - wanted):
            problems.append(
                f"{page.relative_to(ROOT)} references {ref.relative_to(ROOT.resolve())}, "
                "which is not in the manifest"
            )
    heroes = [clip for clip in clips if clip.readme == "hero"]
    grid = [clip for clip in clips if clip.readme == "grid"]
    if len(heroes) != 1:
        problems.append(f"the manifest marks {len(heroes)} hero clips, want exactly one")
    if len(grid) < MIN_GRID:
        problems.append(f"the manifest marks {len(grid)} grid clips, want at least {MIN_GRID}")
    if README.is_file():
        shown = images(README)
        for clip in heroes + grid:
            alts = [alt for src, alt in shown if src == clip.path.resolve()]
            if not alts:
                problems.append(f"README.md does not show {clip.out} ({clip.readme})")
            elif not all(alt_names_scene(alt, clip.scene) for alt in alts):
                problems.append(
                    f"README.md shows {clip.out} without alt text naming "
                    f"`{scene_words(clip.scene)}` and what it shows"
                )
    if GALLERY.is_file():
        shown_in_gallery = {src for src, _ in images(GALLERY)}
        for clip in clips:
            if clip.path.resolve() not in shown_in_gallery:
                problems.append(
                    f"docs/GALLERY.md does not show {clip.out}; run `python3 scripts/gallery.py`"
                )

    if problems:
        print(f"showcase: {len(problems)} problem(s):", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "showcase: run `python3 scripts/showcase.py` (pixi run rs-showcase) to record "
            "missing clips; edit docs/showcase.toml or README.md for the rest",
            file=sys.stderr,
        )
        return 1
    print(
        f"showcase: ok, {len(clips)} clips ({total / 1e6:.2f} MB), {len(known)} scenes in "
        f"{len(categories)} categories; thread {len(known & thread)}, stipple "
        f"{len(known & stipple)}, half-block field {len(fields)}"
    )
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Record docs/media/ from docs/showcase.toml and check it (WEB-R1, WEB-R2)."
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument(
        "--check",
        action="store_true",
        help="verify clips, limits, coverage and the README/gallery references (WEB-R2)",
    )
    group.add_argument(
        "--force",
        action="store_true",
        help="re-record every clip instead of only the missing ones",
    )
    args = parser.parse_args(argv)
    clips = load_manifest()
    scenes, source = gallery.list_scenes()
    print(f"showcase: {len(clips)} manifest clips, {len(scenes)} registered scenes ({source})")
    if args.check:
        return check(clips, scenes)
    unknown = sorted({clip.scene for clip in clips} - {scene.name for scene in scenes})
    if unknown:
        fail(f"manifest scene(s) not registered: {', '.join(unknown)}")
    failed = record(clips, args.force)
    if failed:
        for name, reason in failed:
            print(f"showcase: {name}: {reason}", file=sys.stderr)
        return 1
    gallery.write_gallery(scenes)
    print(f"showcase: wrote {GALLERY.relative_to(ROOT)}")
    return check(clips, scenes)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
