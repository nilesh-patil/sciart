#!/usr/bin/env python3
"""Generate docs/GALLERY.md and the one current frame per registered scene (IMP-06).

The gallery is generated from the registry, never hand-edited:

    python3 scripts/gallery.py            # render missing frames, then write docs/GALLERY.md
    python3 scripts/gallery.py --check    # verify samples/current/; exit 1 if it is stale
    python3 scripts/gallery.py --force    # re-render every frame, then write the gallery

`samples/current/<scene>.png` is the scene's current look. Frames are rendered at 160x48
cells, astro theme, seed 42, t = 20 s, through `examples/preview`, which writes
`<out>-t<t>.png` and is renamed to `<scene>.png` here. This is the only script that writes
`samples/`.

The gallery also opens with an animated section (WEB-R2): every clip in docs/showcase.toml,
which scripts/showcase.py records into docs/media/ (WEB-R1) and checks.
"""

from __future__ import annotations

import argparse
import html
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

# Locate the repo root from this file: <root>/scripts/gallery.py.
ROOT = Path(__file__).resolve().parent.parent
CURRENT_DIR = ROOT / "samples" / "current"
GALLERY = ROOT / "docs" / "GALLERY.md"

# One current frame per scene. 160x48 cells at 8x16 px per cell is 1280x768 px. Most frames
# are 50-300 KiB; the busiest few reach ~1 MiB, for ~12 MiB across the 85 scenes.
SIZE = "160x48"
TIME = "20"
THEME = "astro"
# IMP-06 asks for a "reasonable" total; the current set is ~12 MiB, so this is a guard, not a
# target. Exceeding it means the size or theme changed by accident.
SIZE_BUDGET = 25 * 1024 * 1024

# Display names in Category::ALL order (GEN-R15, ADR-0013); keys are the ids `sciart list`
# prints (CLI-R4). Unknown ids are appended after these, titled from the id.
CATEGORIES = {
    "deep-cosmos": "Deep Cosmos",
    "sun-planets": "Sun & Planets",
    "earth-sky": "Earth & Sky",
    "voyages": "Voyages",
    "forces-waves": "Forces & Waves",
    "reactions": "Reactions",
    "living-systems": "Living Systems",
    "states-of-matter": "States of Matter",
    "physics-of-life": "Physics of Life",
    "molecules-of-life": "Molecules of Life",
    "life-from-matter": "Life from Matter",
}

PREVIEW = [
    "cargo",
    "run",
    "-q",
    "--profile",
    "ci",
    "-p",
    "sciart-generators",
    "--example",
    "preview",
    "--",
]


@dataclass(frozen=True)
class Scene:
    name: str
    category: str
    description: str


def fail(message: str) -> NoReturn:
    sys.exit(f"gallery: {message}")


def list_scenes() -> tuple[list[Scene], str]:
    """Scene metadata from a built binary if present, else `cargo run -q -p sciart -- list`."""
    for relative in ("target/ci/sciart", "target/debug/sciart", "target/release/sciart"):
        binary = ROOT / relative
        if not (binary.is_file() and os.access(binary, os.X_OK)):
            continue
        proc = subprocess.run(
            [str(binary), "list"], cwd=ROOT, capture_output=True, text=True
        )
        if proc.returncode == 0:
            return parse_list(proc.stdout), relative
        print(
            f"gallery: {relative} list failed ({proc.stderr.strip()}), trying the next binary",
            file=sys.stderr,
        )
    proc = subprocess.run(
        ["cargo", "run", "-q", "-p", "sciart", "--", "list"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        fail(f"`cargo run -q -p sciart -- list` failed:\n{proc.stderr.strip()}")
    return parse_list(proc.stdout), "cargo run -q -p sciart -- list"


def parse_list(tsv: str) -> list[Scene]:
    """Parse `sciart list`'s `name<TAB>category id<TAB>description` lines (CLI-R4)."""
    scenes = []
    for line in tsv.splitlines():
        if not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) < 3:
            fail(
                "bad `sciart list` line (want name<TAB>category<TAB>description): "
                f"{line!r}"
            )
        name, category, description = fields[0], fields[1], "\t".join(fields[2:])
        scenes.append(Scene(name, category, description))
    if not scenes:
        fail("`sciart list` printed no scenes")
    names = [scene.name for scene in scenes]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        fail(f"`sciart list` repeats {', '.join(duplicates)}")
    return scenes


def frame_path(scene: Scene) -> Path:
    return CURRENT_DIR / f"{scene.name}.png"


def check(scenes: list[Scene]) -> int:
    """IMP-06: every registered scene has one current frame and there are no orphans."""
    names = {scene.name for scene in scenes}
    missing = [scene.name for scene in scenes if not frame_path(scene).is_file()]
    orphans = []
    if CURRENT_DIR.is_dir():
        orphans = sorted(
            path.name
            for path in CURRENT_DIR.glob("*.png")
            if path.stem not in names
        )
    if missing:
        print(
            f"gallery: {len(missing)} of {len(scenes)} registered scenes have no current frame:",
            file=sys.stderr,
        )
        for name in missing:
            print(f"  missing samples/current/{name}.png", file=sys.stderr)
    if orphans:
        print(
            f"gallery: {len(orphans)} current frame(s) name no registered scene:",
            file=sys.stderr,
        )
        for name in orphans:
            print(f"  orphan  samples/current/{name}", file=sys.stderr)
    if missing or orphans:
        print(
            "gallery: run `python3 scripts/gallery.py` (pixi run rs-gallery) to fix this",
            file=sys.stderr,
        )
        return 1
    print(f"gallery: ok, {len(scenes)} frames for {len(scenes)} registered scenes")
    return 0


def render(scenes: list[Scene], force: bool) -> list[tuple[str, str]]:
    """Render every missing (or, with `force`, every) current frame. Returns failures."""
    CURRENT_DIR.mkdir(parents=True, exist_ok=True)
    todo = [
        scene
        for scene in scenes
        if force or not frame_path(scene).is_file()
    ]
    if not todo:
        print(f"gallery: all {len(scenes)} current frames present")
        return []
    print(
        f"gallery: rendering {len(todo)} frame(s) at {SIZE}, t = {TIME} s, theme {THEME}"
    )
    written = []
    failed = []
    for number, scene in enumerate(todo, 1):
        # A stale intermediate from an interrupted run would confuse the rename below.
        for stale in list(CURRENT_DIR.glob(f"{scene.name}-t*.png")):
            stale.unlink()
        prefix = CURRENT_DIR / scene.name
        cmd = [
            *PREVIEW,
            scene.name,
            "--size",
            SIZE,
            "--t",
            TIME,
            "--theme",
            THEME,
            "--out",
            str(prefix),
        ]
        proc = subprocess.run(cmd, cwd=ROOT)
        if proc.returncode != 0:
            failed.append((scene.name, f"preview exited {proc.returncode}"))
            print(f"  [{number}/{len(todo)}] {scene.name}: FAILED", file=sys.stderr)
            continue
        # `preview` writes `<out>-t<t>.png`; the gallery wants `<scene>.png`.
        produced = sorted(CURRENT_DIR.glob(f"{scene.name}-t*.png"))
        if len(produced) != 1:
            failed.append(
                (
                    scene.name,
                    f"expected one {scene.name}-t*.png, found {len(produced)}",
                )
            )
            print(f"  [{number}/{len(todo)}] {scene.name}: FAILED (bad output name)", file=sys.stderr)
            continue
        target = frame_path(scene)
        produced[0].replace(target)
        written.append(target)
        print(
            f"  [{number}/{len(todo)}] samples/current/{scene.name}.png "
            f"({target.stat().st_size / 1024:.0f} KiB)"
        )
    total = sum(path.stat().st_size for path in written)
    print(
        f"gallery: {len(written)} new frame(s), {total / (1024 * 1024):.2f} MiB total"
    )
    if total > SIZE_BUDGET:
        print(
            f"gallery: warning: {total / (1024 * 1024):.1f} MiB exceeds the "
            f"{SIZE_BUDGET / (1024 * 1024):.0f} MiB budget; consider a smaller --size",
            file=sys.stderr,
        )
    return failed


def category_label(category: str) -> str:
    return CATEGORIES.get(category, category.replace("-", " ").title())


def motion_section(scenes: list[Scene]) -> list[str]:
    """WEB-R2: the animated section, one entry per docs/showcase.toml clip (WEB-R1).

    The clips themselves are recorded by scripts/showcase.py; this only lays them out, in
    manifest order, with the parameters they were recorded with.
    """
    # Imported here: showcase imports this module for the scene list.
    import showcase

    clips = showcase.load_manifest()
    categories = {scene.name: scene.category for scene in scenes}
    lines = [
        "## In motion",
        "",
        (
            f"{len(clips)} clips recorded by `sciart record` from "
            "[`docs/showcase.toml`](showcase.toml) (`pixi run rs-showcase`); each runs "
            "from the simulated time shown, at the size, seed and frame rate shown."
        ),
        "",
    ]
    for clip in clips:
        out = Path(clip.out)
        source = out.relative_to("docs") if out.parts[0] == "docs" else Path("..") / out
        start = f"{clip.start:g}"
        end = f"{clip.start + clip.seconds:g}"
        category = category_label(categories.get(clip.scene, "unknown"))
        lines.append(f"### `{clip.scene}` in motion")
        lines.append("")
        lines.append(f'<img src="{source.as_posix()}" alt="{html.escape(clip.alt)}">')
        lines.append("")
        lines.append(
            f"{category} · {clip.size} cells · t = {start}–{end} s · {clip.fps} fps · "
            f"seed {clip.seed} · {clip.theme}"
        )
        lines.append("")
    return lines


def write_gallery(scenes: list[Scene]) -> None:
    """docs/GALLERY.md: title, one-line legend, category sections, one image per scene."""
    sections: dict[str, list[Scene]] = {category: [] for category in CATEGORIES}
    for scene in scenes:
        sections.setdefault(scene.category, []).append(scene)
    lines = [
        "# Gallery",
        "",
        (
            f"Every one of the {len(scenes)} registered scenes, one current frame each: astro "
            f"theme at {SIZE} cells (8x16 px per cell), seed 42, t = {TIME} s. A compare link, "
            "where present, shows the scene's historical looks side by side."
        ),
        "",
        *motion_section(scenes),
    ]
    for category, items in sections.items():
        if not items:
            continue
        lines.append(f"## {category_label(category)}")
        lines.append("")
        for scene in sorted(items, key=lambda item: item.name):
            lines.append(f"### `{scene.name}`")
            lines.append("")
            lines.append(f"![{scene.name}](../samples/current/{scene.name}.png)")
            lines.append("")
            lines.append(scene.description)
            lines.append("")
    lines.append(
        "Generated by `python3 scripts/gallery.py` (`pixi run rs-gallery`); "
        "do not edit by hand."
    )
    lines.append("")
    GALLERY.write_text("\n".join(lines), encoding="utf-8")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Generate docs/GALLERY.md and samples/current/<scene>.png (IMP-06)."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify every registered scene has a current frame and no orphans exist",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="re-render every current frame instead of only the missing ones",
    )
    args = parser.parse_args(argv)
    scenes, source = list_scenes()
    print(f"gallery: {len(scenes)} registered scenes ({source})")
    if args.check:
        return check(scenes)
    failed = render(scenes, args.force)
    if failed:
        for name, reason in failed:
            print(f"gallery: {name}: {reason}", file=sys.stderr)
        return 1
    write_gallery(scenes)
    print(f"gallery: wrote {GALLERY.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
