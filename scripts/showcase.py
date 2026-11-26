#!/usr/bin/env python3
"""Record and check the animated showcase from docs/showcase.toml (WEB-R1, WEB-R2).

Every animated capture in the README and the gallery is a clip in the manifest, recorded by
`sciart record` with exactly the manifest's parameters:

    python3 scripts/showcase.py            # record missing and stale clips, write the gallery
    python3 scripts/showcase.py --force    # re-record every clip, then write the gallery
    python3 scripts/showcase.py --check    # WEB-R2; exit 1 when anything is off

WEB-R1: the manifest lists each clip's id, scene, seed, size, start, seconds, fps, theme and
output path under docs/media/; docs/media/showcase.lock records the parameters each clip was
recorded with, and a clip whose manifest entry differs is stale; a GIF is at most 1.5 MB and
the set at most 16 MB; the set covers every category of the registry with at least one clip
and at least 12 scenes in all, including the thread idiom (GEN-R25), the orbital-thread
stipple idiom (GEN-R26) and a half-block field scene.

WEB-R2 (`--check`): fails when a manifest clip is missing, stale, or not a GIF with at least
one frame; a file under docs/media/ is neither a clip nor the lock; a clip or the set is over
its size limit; a manifest scene is not registered; the WEB-R1 coverage is not met;
README.md or docs/GALLERY.md references an animated image that is not a manifest clip
(wherever it lives, however the tag is written) or an image that does not resolve to a file;
or the animated section of docs/GALLERY.md differs from what scripts/gallery.py writes.
README.md shows the one `readme = "hero"` clip and every `readme = "grid"` clip (at least
eight), each with alt text naming the scene; docs/GALLERY.md shows every clip.

The README's layout (WEB-R2): its `## Gallery` is grouped under the `### <domain>` headings of
DOMAINS, in that order, each clip under its category's domain, with a clip from every
category; its `## What's inside` table has one row per category with the registry's scene
count (example scenes named in a row belong to that category); and the `sciart` commands in
its shell blocks name scenes from at least five categories, with real scenes and category ids
after `run`, `record`, `frame`, `--only`, `--exclude` and `--category`.

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
MIN_GRID = 8
# WEB-R2: the README's example commands name scenes from at least this many categories.
MIN_COMMAND_CATEGORIES = 5

# The README's science domains (WEB-R2), in the order it presents them: the sciences first,
# then Earth and space. Every registry category belongs to exactly one.
DOMAINS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("Physics", ("forces-waves", "states-of-matter")),
    ("Chemistry", ("reactions",)),
    ("Life", ("living-systems", "physics-of-life", "molecules-of-life", "life-from-matter")),
    ("Earth", ("earth-sky",)),
    ("Space", ("sun-planets", "deep-cosmos", "voyages")),
)
DOMAIN_OF = {category: domain for domain, categories in DOMAINS for category in categories}
# `sciart` subcommands whose first argument is a scene name.
SCENE_COMMANDS = ("run", "record", "frame")

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

LOCK = MEDIA_DIR / "showcase.lock"
# The parameters a recording depends on; the lock keeps them per output path (WEB-R1).
RECORDED = ("scene", "seed", "size", "start", "seconds", "fps", "theme")

# HTML image tags (`<img src>`, `<source srcset>`) with double, single or no quotes, and
# Markdown images `![alt](src "title")`.
TAG = re.compile(r"<(?P<name>img|source)\b(?P<attrs>[^>]*)>", re.IGNORECASE)
ATTR = re.compile(r"""([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))""")
MD_IMAGE = re.compile(r"!\[(?P<alt>[^\]]*)\]\(\s*<?(?P<src>[^)\s>]+)>?(?:\s+[\"'][^)]*)?\)")
# Any reference to an animated image, in a tag, a link or plain text.
ANIMATED = re.compile(
    r"""(?P<ref>[^\s"'()<>=,\[\]`]+\.(?:gif|apng|webp|avif|mp4|webm|mov))(?=[\s"'()<>,?#\]]|$)""",
    re.IGNORECASE,
)
# Code is not a reference: fenced blocks and inline spans are blanked before scanning.
FENCE = re.compile(r"^(```|~~~).*?^\1[^\n]*$", re.MULTILINE | re.DOTALL)
CODE_SPAN = re.compile(r"`[^`\n]*`")


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

    def recorded(self) -> dict[str, object]:
        """The parameters this clip is recorded with, as the lock stores them (WEB-R1)."""
        return {key: getattr(self, key) for key in RECORDED}

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


def read_lock() -> dict[str, dict[str, object]]:
    """docs/media/showcase.lock: output path -> the parameters it was recorded with."""
    if not LOCK.is_file():
        return {}
    try:
        data = tomllib.loads(LOCK.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as err:
        fail(f"cannot read {LOCK.relative_to(ROOT)}: {err}")
    lock = {}
    for entry in data.get("clip", []):
        out = entry.get("out")
        if isinstance(out, str):
            lock[out] = {key: entry.get(key) for key in RECORDED}
    return lock


def write_lock(clips: list[Clip], lock: dict[str, dict[str, object]]) -> None:
    """Write the lock for the manifest's clips, in manifest order (WEB-R1)."""

    def value(v: object) -> str:
        if isinstance(v, str):
            return '"' + v.replace("\\", "\\\\").replace('"', '\\"') + '"'
        if isinstance(v, float):
            return repr(v)
        return str(v)

    lines = [
        "# The parameters each clip in this directory was recorded with (WEB-R1).",
        "# Written by scripts/showcase.py after each recording; do not edit by hand.",
    ]
    for clip in clips:
        params = lock.get(clip.out)
        if params is None:
            continue
        lines += ["", "[[clip]]", f"out = {value(clip.out)}"]
        lines += [f"{key} = {value(params[key])}" for key in RECORDED]
    LOCK.write_text("\n".join(lines) + "\n", encoding="utf-8")


def stale_reason(clip: Clip, lock: dict[str, dict[str, object]]) -> str | None:
    """Why `clip` no longer matches its recording, or None when it does (WEB-R1)."""
    recorded = lock.get(clip.out)
    if recorded is None:
        return "not in docs/media/showcase.lock"
    wanted = clip.recorded()
    changed = [
        f"{key} {recorded[key]!r} -> {wanted[key]!r}"
        for key in RECORDED
        if recorded[key] != wanted[key]
    ]
    return ("recorded with " + ", ".join(changed)) if changed else None


def record(clips: list[Clip], force: bool) -> list[tuple[str, str]]:
    """Record every missing or stale (with `force`, every) clip. Returns failures."""
    MEDIA_DIR.mkdir(parents=True, exist_ok=True)
    lock = read_lock()
    todo = [
        clip
        for clip in clips
        if force or not clip.path.is_file() or stale_reason(clip, lock)
    ]
    if not todo:
        print(f"showcase: all {len(clips)} clips present and current")
        write_lock(clips, lock)
        return []
    print(f"showcase: recording {len(todo)} clip(s)")
    failed = []
    for number, clip in enumerate(todo, 1):
        # A failed recording must not leave the old clip looking current.
        lock.pop(clip.out, None)
        proc = subprocess.run(clip.command(), cwd=ROOT)
        if proc.returncode != 0 or not clip.path.is_file():
            failed.append((clip.id, f"sciart record exited {proc.returncode}"))
            print(f"  [{number}/{len(todo)}] {clip.id}: FAILED", file=sys.stderr)
        else:
            lock[clip.out] = clip.recorded()
            print(
                f"  [{number}/{len(todo)}] {clip.out} "
                f"({clip.path.stat().st_size / 1e6:.2f} MB)"
            )
        write_lock(clips, lock)
    return failed


def gif_problem(path: Path) -> str | None:
    """Why `path` is not a well-formed GIF with at least one frame, or None (WEB-R2)."""
    data = path.read_bytes()
    if data[:6] not in (b"GIF87a", b"GIF89a"):
        return "is not a GIF (bad header)"
    if len(data) < 13:
        return "is a truncated GIF (no screen descriptor)"
    pos = 13
    if data[10] & 0x80:  # global colour table
        pos += 3 * (2 << (data[10] & 0x07))

    def skip_sub_blocks(p: int) -> int:
        while p < len(data) and data[p] != 0:
            p += 1 + data[p]
        return p + 1

    frames = 0
    while pos < len(data):
        block = data[pos]
        if block == 0x3B:  # trailer
            return None if frames else "is a GIF with no frames"
        if block == 0x21:  # extension: label, then data sub-blocks
            pos = skip_sub_blocks(pos + 2)
        elif block == 0x2C:  # image descriptor, optional local table, LZW data
            if pos + 10 > len(data):
                break
            flags = data[pos + 9]
            pos += 10
            if flags & 0x80:
                pos += 3 * (2 << (flags & 0x07))
            pos = skip_sub_blocks(pos + 1)
            frames += 1
        else:
            return f"is a corrupt GIF (unknown block 0x{block:02x} at byte {pos})"
    return "is a truncated GIF (no trailer)"


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


def is_external(ref: str) -> bool:
    """A URL (a badge, say) rather than a file in the repository."""
    return bool(re.match(r"^[a-z][a-z0-9+.-]*:", ref, re.IGNORECASE)) or ref.startswith("//")


def resolve(page: Path, ref: str) -> Path:
    """`ref` (query and fragment dropped) resolved against `page`'s directory."""
    ref = html.unescape(ref).split("#", 1)[0].split("?", 1)[0]
    return (page.parent / ref).resolve()


def blank_code(text: str) -> str:
    """`text` with fenced code blocks and inline code spans blanked out."""
    text = FENCE.sub(lambda m: "\n" * m.group(0).count("\n"), text)
    return CODE_SPAN.sub("", text)


def prose(page: Path) -> str:
    """`page` with fenced code blocks and inline code spans blanked out."""
    return blank_code(page.read_text(encoding="utf-8"))


def images(page: Path) -> list[tuple[str, str]]:
    """`(src, alt)` for every <img>, every <source srcset> candidate and every Markdown
    image in `page`, however its attributes are quoted."""
    return images_in(prose(page))


def images_in(text: str) -> list[tuple[str, str]]:
    """`images` over text whose code is already blanked."""
    found = []
    for tag in TAG.finditer(text):
        attrs = {}
        for m in ATTR.finditer(tag.group("attrs")):
            attrs[m.group(1).lower()] = next(v for v in m.groups()[1:] if v is not None)
        alt = html.unescape(attrs.get("alt", ""))
        if "src" in attrs:
            found.append((attrs["src"], alt))
        for candidate in attrs.get("srcset", "").split(","):
            if candidate.strip():
                found.append((candidate.split()[0], alt))
    for match in MD_IMAGE.finditer(text):
        found.append((match.group("src"), match.group("alt")))
    return found


def reference_problems(page: Path, clips: set[Path]) -> list[str]:
    """WEB-R2: every animated reference is a manifest clip; every image resolves to a file."""
    name = page.relative_to(ROOT)
    problems = []
    for match in ANIMATED.finditer(prose(page)):
        ref = match.group("ref")
        if is_external(ref) or resolve(page, ref) not in clips:
            problems.append(f"{name} references animated {ref}, which is not a manifest clip")
    for src, _ in images(page):
        if not is_external(src) and not resolve(page, src).is_file():
            problems.append(f"{name} shows {src}, which does not resolve to a file")
    return sorted(set(problems))


def sections(text: str, level: int) -> list[tuple[str, str]]:
    """`(heading, body)` for each heading of `level` in `text` (code already blanked); the
    text before the first heading comes first, with the heading ""."""
    parts = re.split(rf"^{'#' * level} +(.+?)\s*$", text, flags=re.MULTILINE)
    return [("", parts[0])] + [
        (parts[i].strip(), parts[i + 1]) for i in range(1, len(parts), 2)
    ]


def section(text: str, heading: str) -> str | None:
    """The body of the level-2 section `heading` (code already blanked), or None."""
    return next((body for title, body in sections(text, 2) if title == heading), None)


def plain(cell: str) -> str:
    """A Markdown table cell without emphasis, code marks or links' targets."""
    cell = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", cell)
    return re.sub(r"[*_`]", "", cell).strip()


def readme_layout_problems(
    text: str, page: Path, clips: list[Clip], scenes: list[gallery.Scene]
) -> list[str]:
    """WEB-R2: the README's gallery is grouped by domain with a clip from every category, its
    "What's inside" table has every category with the registry's count, and its example
    commands name scenes from at least five categories."""
    problems: list[str] = []
    name = page.name
    category_of = {scene.name: scene.category for scene in scenes}
    counts: dict[str, int] = {}
    for scene in scenes:
        counts[scene.category] = counts.get(scene.category, 0) + 1
    for category in sorted(set(counts) - set(DOMAIN_OF)):
        problems.append(
            f"category `{category}` belongs to no domain; add it to DOMAINS in scripts/showcase.py"
        )
    label = gallery.category_label
    domain_names = [domain for domain, _ in DOMAINS]
    by_path = {clip.path.resolve(): clip for clip in clips}

    # Gallery: `### <domain>` headings in DOMAINS order, each clip under its own domain.
    prose_text = blank_code(text)
    body = section(prose_text, "Gallery")
    if body is None:
        problems.append(f"{name} has no `## Gallery` section")
    else:
        parts = sections(body, 3)
        headings = [heading for heading, _ in parts[1:]]
        if headings != domain_names:
            problems.append(
                f"{name}: the gallery's `###` headings are {headings}, want the domains "
                f"{domain_names} in that order"
            )
        shown: set[str] = set()
        in_gallery: set[Path] = set()
        for heading, part in parts:
            for src, _ in images_in(part):
                clip = by_path.get(resolve(page, src))
                if clip is None:
                    continue
                in_gallery.add(clip.path.resolve())
                category = category_of.get(clip.scene)
                if category is None:
                    continue  # reported as an unregistered manifest scene
                shown.add(category)
                want = DOMAIN_OF.get(category)
                if heading != want:
                    where = f"under `### {heading}`" if heading else "before the first domain heading"
                    problems.append(
                        f"{name}: the gallery shows {clip.out} ({label(category)}) {where}, "
                        f"want `### {want}`"
                    )
        for category in sorted(set(counts) - shown):
            problems.append(f"{name}: the gallery shows no clip from {label(category)}")
        for clip in clips:
            if clip.readme == "grid" and clip.path.resolve() not in in_gallery:
                problems.append(f"{name}: grid clip {clip.out} is not in the `## Gallery` section")

    # What's inside: one row per category, with the registry's count. Code spans stay: the
    # example scenes are written as code.
    body = section(FENCE.sub(lambda m: "\n" * m.group(0).count("\n"), text), "What's inside")
    if body is None:
        problems.append(f"{name} has no `## What's inside` section")
    else:
        rows = [
            [cell.strip() for cell in line.strip().strip("|").split("|")]
            for line in body.splitlines()
            if line.strip().startswith("|")
        ]
        rows = [row for row in rows if not all(re.fullmatch(r":?-+:?", c) for c in row)]
        header = [plain(cell).lower() for cell in rows[0]] if rows else []
        if "category" not in header or "scenes" not in header:
            problems.append(
                f"{name}: `## What's inside` has no table with `Category` and `Scenes` columns"
            )
        else:
            by_label = {label(category): category for category in counts}
            col_category, col_scenes = header.index("category"), header.index("scenes")
            seen: list[str] = []
            for row in rows[1:]:
                cell = plain(row[col_category]) if col_category < len(row) else ""
                category = by_label.get(cell)
                if category is None:
                    problems.append(f"{name}: `What's inside` names an unknown category {cell!r}")
                    continue
                seen.append(category)
                count = plain(row[col_scenes]) if col_scenes < len(row) else ""
                if count != str(counts[category]):
                    problems.append(
                        f"{name}: `What's inside` gives {cell} {count!r} scenes; the "
                        f"registry has {counts[category]}"
                    )
                for scene in re.findall(r"`([a-z0-9_]+)`", "|".join(row)):
                    if category_of.get(scene) != category:
                        problems.append(
                            f"{name}: `What's inside` names `{scene}` under {cell}, which is "
                            "not a scene of that category"
                        )
            for category in sorted(set(counts) - set(seen)):
                problems.append(f"{name}: `What's inside` has no row for {label(category)}")
            for category in sorted({c for c in seen if seen.count(c) > 1}):
                problems.append(f"{name}: `What's inside` has {label(category)} more than once")
            order = [domain_names.index(DOMAIN_OF[c]) for c in seen if c in DOMAIN_OF]
            if order != sorted(order):
                problems.append(
                    f"{name}: `What's inside` is not grouped by domain in the order {domain_names}"
                )

    # Example commands: real scenes and categories, from at least five categories.
    named: set[str] = set()
    categories = set(counts)
    for block in re.findall(r"^```(?:sh|bash|console)\s*\n(.*?)^```", text, re.S | re.M):
        for line in block.splitlines():
            line = line.split(" #", 1)[0].strip().removeprefix("$ ")
            tokens = line.split()
            if not tokens or tokens[0] != "sciart":
                continue
            tokens = tokens[1 : tokens.index("--")] if "--" in tokens else tokens[1:]
            for i, token in enumerate(tokens):
                value = tokens[i + 1] if i + 1 < len(tokens) else ""
                if token in ("--only", "--exclude") and value not in category_of:
                    problems.append(f"{name}: `{line}`: {token} {value!r} is not a scene")
                if token == "--category" and value not in categories:
                    problems.append(f"{name}: `{line}`: --category {value!r} is not a category id")
                if token in category_of:
                    named.add(category_of[token])
            if tokens and tokens[0] in SCENE_COMMANDS:
                first = tokens[1] if len(tokens) > 1 else ""
                if first not in category_of and not first.startswith("<"):
                    problems.append(f"{name}: `{line}`: {first!r} is not a scene")
    if len(named) < MIN_COMMAND_CATEGORIES:
        problems.append(
            f"{name}: the example commands name scenes from {len(named)} categories, want at "
            f"least {MIN_COMMAND_CATEGORIES}"
        )
    return problems


def check(clips: list[Clip], scenes: list[gallery.Scene]) -> int:
    """WEB-R2: the manifest, docs/media/, the README and the gallery agree (WEB-R1 limits)."""
    problems: list[str] = []
    registered = {scene.name: scene for scene in scenes}

    # Clips present, current, real GIFs, within the limits; no orphans in docs/media/.
    total = 0
    lock = read_lock()
    for clip in clips:
        if not clip.path.is_file():
            problems.append(f"missing {clip.out} (clip `{clip.id}`)")
            continue
        reason = stale_reason(clip, lock)
        if reason:
            problems.append(f"stale {clip.out}: {reason}")
        bad = gif_problem(clip.path)
        if bad:
            problems.append(f"{clip.out} {bad}")
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
            if path.is_file() and path.resolve() not in wanted | {LOCK.resolve()}:
                problems.append(f"orphan {path.relative_to(ROOT)} is not in the manifest")
    for out in sorted(set(lock) - {clip.out for clip in clips}):
        problems.append(f"docs/media/showcase.lock lists {out}, which is not in the manifest")

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
    for category in sorted({scene.category for scene in scenes} - categories):
        problems.append(f"coverage: no clip from {gallery.category_label(category)}")
    if not known & thread:
        problems.append("coverage: no thread-idiom scene (GEN-R25)")
    if not known & stipple:
        problems.append("coverage: no orbital-thread stipple scene (GEN-R26)")
    if not fields:
        problems.append("coverage: no half-block field scene")

    # README and gallery: only manifest clips animate, every image resolves, the hero and
    # grid are shown with alt text, and the gallery's animated section is the generator's.
    for page in (README, GALLERY):
        if not page.is_file():
            problems.append(f"{page.relative_to(ROOT)} is missing")
            continue
        problems.extend(reference_problems(page, wanted))
    if GALLERY.is_file():
        section = "\n".join(gallery.motion_section(scenes))
        if section not in GALLERY.read_text(encoding="utf-8"):
            problems.append(
                "docs/GALLERY.md: the animated section differs from what scripts/gallery.py "
                "writes; run `python3 scripts/gallery.py`"
            )
    heroes = [clip for clip in clips if clip.readme == "hero"]
    grid = [clip for clip in clips if clip.readme == "grid"]
    if len(heroes) != 1:
        problems.append(f"the manifest marks {len(heroes)} hero clips, want exactly one")
    if len(grid) < MIN_GRID:
        problems.append(f"the manifest marks {len(grid)} grid clips, want at least {MIN_GRID}")
    if README.is_file():
        shown = [(resolve(README, src), alt) for src, alt in images(README)]
        for clip in heroes + grid:
            alts = [alt for src, alt in shown if src == clip.path.resolve()]
            if not alts:
                problems.append(f"README.md does not show {clip.out} ({clip.readme})")
            elif not all(alt_names_scene(alt, clip.scene) for alt in alts):
                problems.append(
                    f"README.md shows {clip.out} without alt text naming "
                    f"`{scene_words(clip.scene)}` and what it shows"
                )
        problems.extend(
            readme_layout_problems(README.read_text(encoding="utf-8"), README, clips, scenes)
        )
    if GALLERY.is_file():
        shown_in_gallery = {resolve(GALLERY, src) for src, _ in images(GALLERY)}
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
            "missing and stale clips; edit docs/showcase.toml or README.md for the rest",
            file=sys.stderr,
        )
        return 1
    print(
        f"showcase: ok, {len(clips)} clips ({total / 1e6:.2f} MB), {len(known)} scenes in "
        f"all {len(categories)} categories; thread {len(known & thread)}, stipple "
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
        help="re-record every clip instead of only the missing and stale ones",
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
