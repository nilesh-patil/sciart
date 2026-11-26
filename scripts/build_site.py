#!/usr/bin/env python3
"""Build the docs site into site/dist/ (WEB-R5, ADR-0034).

    python3 scripts/build_site.py          # pixi run site-build (after pixi run web-build)

The site is plain files from three sources and nothing else:

- the user guide, docs/guide/ (chapters.toml lists the chapters in order), built as a book;
  `<!-- include: README.md#Install -->` copies the README's Install section in and
  `<!-- include: docs/config.example.toml -->` the example configuration, so neither can
  drift from its source (WEB-R7);
- the gallery, from the scene registry (`sciart list --json`), the stills in
  samples/current/ and the showcase manifest, docs/showcase.toml (WEB-R1): one entry per
  registered scene;
- the live player: site/static/player.js and the WebAssembly module from
  `pixi run web-build` (WEB-R3).

Everything the pages load is copied into site/dist/, so the site loads nothing from another
origin (WEB-R6). scripts/site_check.py checks the result (WEB-R4, WEB-R6, WEB-R7). The
Markdown the guide uses is a small subset (headings, paragraphs, lists, tables, fenced code,
inline code, links, emphasis), converted here with the standard library.
"""

from __future__ import annotations

import html
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

import gallery
import showcase

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "site" / "dist"
STATIC = ROOT / "site" / "static"
GUIDE = ROOT / "docs" / "guide"
README = ROOT / "README.md"
STILLS = ROOT / "samples" / "current"
WASM = ROOT / "web" / "target" / "wasm32-unknown-unknown" / "release" / "sciart_web.wasm"
REPOSITORY = "https://github.com/nilesh-patil/sciart"

# The still frames' size in pixels (scripts/gallery.py renders 160x48 cells at 8x16 px).
STILL_W, STILL_H = 1280, 768
SITE_NAME = "sciart"
# WEB-R9: the project's one description, for the lede and every page's meta description.
DESCRIPTION = "Real-time generative science art for the terminal: physics, chemistry, biology, Earth and space"


def fail(message: str) -> NoReturn:
    sys.exit(f"site: {message}")


# ---- The binary --------------------------------------------------------------------------


def sciart_command() -> list[str]:
    """A built `sciart` if there is one (as scripts/gallery.py looks for it), else cargo."""
    for relative in ("target/ci/sciart", "target/debug/sciart", "target/release/sciart"):
        binary = ROOT / relative
        if binary.is_file() and os.access(binary, os.X_OK):
            return [str(binary)]
    return ["cargo", "run", "-q", "-p", "sciart", "--"]


def sciart(*args: str) -> str:
    proc = subprocess.run(
        [*sciart_command(), *args], cwd=ROOT, capture_output=True, text=True
    )
    if proc.returncode != 0:
        fail(f"`sciart {' '.join(args)}` failed:\n{proc.stderr.strip()}")
    return proc.stdout


def catalogue() -> list[dict]:
    """The registered scenes, as `sciart list --json` prints them (CLI-R16)."""
    data = json.loads(sciart("list", "--json"))
    if data.get("schema") != 1:
        fail(f"unexpected `list --json` schema {data.get('schema')!r}")
    return data["scenes"]


def build_label() -> str:
    """`sciart 0.1.0 (abc1234)`: the version and the commit the site was built from."""
    version = sciart("--version").strip()
    proc = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True
    )
    commit = proc.stdout.strip() if proc.returncode == 0 else "unknown"
    return f"{version} ({commit})"


# ---- Markdown ------------------------------------------------------------------------------


def slug(text: str) -> str:
    """A heading's id: lower case, words joined by dashes, punctuation dropped."""
    text = re.sub(r"<[^>]+>", "", text).lower()
    text = re.sub(r"[^\w\s-]", "", text)
    return re.sub(r"[\s_]+", "-", text).strip("-")


def inline(text: str, link) -> str:
    """Inline Markdown: `code`, [text](url), **strong**, *emphasis*; everything escaped."""
    out = []
    for i, part in enumerate(re.split(r"(`[^`]*`)", text)):
        if i % 2:
            out.append(f"<code>{html.escape(part[1:-1])}</code>")
            continue
        part = html.escape(part, quote=False)
        part = re.sub(
            r"\[([^\]]+)\]\(([^)\s]+)\)",
            lambda m: f'<a href="{html.escape(link(html.unescape(m.group(2))))}">{m.group(1)}</a>',
            part,
        )
        part = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", part)
        part = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"<em>\1</em>", part)
        out.append(part)
    return "".join(out)


def markdown(text: str, link) -> str:
    """The guide's Markdown subset as HTML. `link` rewrites each link target."""
    lines = text.split("\n")
    out: list[str] = []
    i = 0
    paragraph: list[str] = []

    def flush() -> None:
        if paragraph:
            out.append(f"<p>{inline(' '.join(paragraph), link)}</p>")
            paragraph.clear()

    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        fence = re.match(r"^(```|~~~)\s*([\w+-]*)\s*$", stripped)
        if fence:
            flush()
            marker, lang = fence.groups()
            body = []
            i += 1
            while i < len(lines) and lines[i].strip() != marker:
                body.append(lines[i])
                i += 1
            cls = f' class="language-{lang}"' if lang else ""
            out.append(f"<pre><code{cls}>{html.escape(chr(10).join(body))}</code></pre>")
            i += 1
            continue
        heading = re.match(r"^(#{1,6})\s+(.*?)\s*#*$", stripped)
        if heading:
            flush()
            level = len(heading.group(1))
            body = inline(heading.group(2), link)
            out.append(f'<h{level} id="{slug(heading.group(2))}">{body}</h{level}>')
            i += 1
            continue
        if stripped.startswith("|") and i + 1 < len(lines) and re.match(
            r"^\|?\s*:?-{3,}", lines[i + 1].strip()
        ):
            flush()
            cells = lambda row: [c.strip() for c in row.strip().strip("|").split("|")]
            header = cells(line)
            rows = []
            i += 2
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append(cells(lines[i]))
                i += 1
            head = "".join(f"<th>{inline(c, link)}</th>" for c in header)
            body = "".join(
                "<tr>" + "".join(f"<td>{inline(c, link)}</td>" for c in row) + "</tr>"
                for row in rows
            )
            out.append(f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>")
            continue
        item = re.match(r"^(\s*)([-*]|\d+\.)\s+(.*)$", line)
        if item and not item.group(1):
            flush()
            ordered = item.group(2)[0].isdigit()
            items: list[list[str]] = []
            while i < len(lines):
                m = re.match(r"^([-*]|\d+\.)\s+(.*)$", lines[i])
                if m:
                    items.append([m.group(2)])
                elif lines[i].startswith("  ") and lines[i].strip() and items:
                    items[-1].append(lines[i].strip())
                else:
                    break
                i += 1
            tag = "ol" if ordered else "ul"
            body = "".join(f"<li>{inline(' '.join(it), link)}</li>" for it in items)
            out.append(f"<{tag}>{body}</{tag}>")
            continue
        if not stripped:
            flush()
        elif stripped.startswith("\x00"):
            flush()
            out.append(stripped)
        else:
            paragraph.append(stripped)
        i += 1
    flush()
    return "\n".join(out)


def readme_section(title: str) -> str:
    """The body of the README's `## <title>` section (up to the next `## ` heading)."""
    text = README.read_text(encoding="utf-8")
    match = re.search(rf"^## {re.escape(title)}\s*\n(.*?)(?=^## |\Z)", text, re.S | re.M)
    if not match:
        fail(f"README.md has no `## {title}` section")
    return match.group(1).strip("\n")


def repository_link(target: str) -> str:
    """A README link as seen from the site: repository files point at the repository."""
    if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
        return target
    return f"{REPOSITORY}/blob/main/{target}"


def chapter_link(target: str) -> str:
    """A guide link: `x.md#frag` is the built page `x.html#frag`."""
    return re.sub(r"^([\w-]+)\.md(#|$)", r"\1.html\2", target)


def expand_includes(text: str) -> tuple[str, list[str]]:
    """Replace include directives by placeholders; returns the text and the HTML for each."""
    blocks: list[str] = []

    def include(match: re.Match) -> str:
        source = match.group(1).strip()
        path, _, section = source.partition("#")
        if path == "README.md" and section:
            body = markdown(readme_section(section), repository_link)
            html_block = (
                f'<section class="readme-include" data-source="{html.escape(source)}">'
                f"\n{body}\n</section>"
            )
        else:
            file = ROOT / path
            if not file.is_file():
                fail(f"include of a missing file: {source}")
            lang = {".toml": "toml", ".rs": "rust", ".sh": "sh"}.get(file.suffix, "")
            cls = f' class="language-{lang}"' if lang else ""
            html_block = (
                f'<pre data-source="{html.escape(source)}"><code{cls}>'
                f"{html.escape(file.read_text(encoding='utf-8').rstrip())}</code></pre>"
            )
        blocks.append(html_block)
        return f"\n\x00include{len(blocks) - 1}\x00\n"

    text = re.sub(r"^<!--\s*include:\s*(.+?)\s*-->\s*$", include, text, flags=re.M)
    return text, blocks


# ---- Pages ---------------------------------------------------------------------------------


@dataclass(frozen=True)
class Page:
    path: str  # relative to site/dist, e.g. "guide/running.html"
    title: str
    section: str  # "home", "play", "gallery" or "guide"
    body: str


def page_html(page: Page, build: str) -> str:
    depth = page.path.count("/")
    root = "../" * depth
    nav = [
        ("play", "play/index.html", "Player"),
        ("gallery", "gallery/index.html", "Gallery"),
        ("guide", "guide/index.html", "Guide"),
    ]
    links = "".join(
        f'<a href="{root}{href}"{" aria-current=\"page\"" if page.section == key else ""}>{label}</a>'
        for key, href, label in nav
    )
    title = SITE_NAME if page.section == "home" else f"{page.title} · {SITE_NAME}"
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{DESCRIPTION}. 85 scenes, drawn in braille and half-block cells.">
<link rel="icon" href="{root}assets/icon.svg" type="image/svg+xml">
<link rel="stylesheet" href="{root}assets/style.css">
</head>
<body>
<header class="site">
<a class="mark" href="{root}index.html">{SITE_NAME}</a>
<nav aria-label="Site">{links}<a href="{REPOSITORY}">Source</a></nav>
</header>
<main>
{page.body}
</main>
<footer>
<p>{html.escape(build)} · MIT licence · Stills, clips and the player are drawn by sciart itself, at 8×16 pixels per cell.</p>
</footer>
</body>
</html>
"""


def clip_img(clip: showcase.Clip, root: str, width: int | None = None) -> str:
    cols, rows = (int(n) for n in clip.size.split("x"))
    w, h = cols * 8, rows * 16
    style = f' style="width:{width}px"' if width else ""
    name = Path(clip.out).name
    return (
        f'<img src="{root}media/{name}" alt="{html.escape(clip.alt)}" width="{w}" '
        f'height="{h}"{style}>'
    )


def home_tiles(clips: list[showcase.Clip], scenes: list[dict], count: int = 6) -> list[showcase.Clip]:
    """`count` README grid clips taken in turn from each domain (WEB-R2's order), so the home
    page shows every science, not the first few clips of the manifest."""
    category = {s["name"]: s["category"] for s in scenes}
    queues = [
        [c for c in clips if c.readme == "grid" and category.get(c.scene) in members]
        for _, members in showcase.DOMAINS
    ]
    tiles: list[showcase.Clip] = []
    while len(tiles) < count and any(queues):
        for queue in queues:
            if queue and len(tiles) < count:
                tiles.append(queue.pop(0))
    return tiles


def home(clips: list[showcase.Clip], scenes: list[dict]) -> Page:
    hero = next(clip for clip in clips if clip.readme == "hero")
    tiles = "\n".join(
        f'<li><figure>{clip_img(clip, "")}<figcaption>{html.escape(clip.scene)}</figcaption></figure></li>'
        for clip in home_tiles(clips, scenes)
    )
    categories = len({s["category"] for s in scenes})
    body = f"""<div class="home">
<figure class="hero">{clip_img(hero, "")}<figcaption>{html.escape(hero.scene)} · recorded with <code>sciart record</code></figcaption></figure>
<h1>{SITE_NAME}</h1>
<p class="lede">{DESCRIPTION}. {len(scenes)} scenes in {categories} categories, each a small
simulation drawn in braille dots and half-block cells at 30 frames per second. It keeps your
machine awake while you watch.</p>
<ul class="doors">
<li><a href="play/index.html">Play a scene in your browser</a></li>
<li><a href="gallery/index.html">See all {len(scenes)} scenes</a></li>
<li><a href="guide/installing.html">Install it</a></li>
<li><a href="guide/index.html">Read the guide</a></li>
</ul>
<h2 id="in-motion">In motion</h2>
<ul class="clips">
{tiles}
</ul>
</div>"""
    return Page("index.html", SITE_NAME, "home", body)


def guide_pages() -> list[Page]:
    chapters = tomllib.loads((GUIDE / "chapters.toml").read_text(encoding="utf-8"))["chapters"]
    names = [Path(ch["file"]).stem for ch in chapters]
    pages = []
    for n, chapter in enumerate(chapters):
        source = GUIDE / chapter["file"]
        if not source.is_file():
            fail(f"docs/guide/chapters.toml lists a missing chapter {chapter['file']}")
        text, blocks = expand_includes(source.read_text(encoding="utf-8"))
        body = markdown(text, chapter_link)
        for k, block in enumerate(blocks):
            body = body.replace(f"\x00include{k}\x00", block)
        toc = "".join(
            f'<li><a href="{name}.html"{" aria-current=\"page\"" if name == names[n] else ""}>'
            f"{html.escape(ch['title'])}</a></li>"
            for name, ch in zip(names, chapters)
        )
        pager = []
        if n > 0:
            pager.append(f'<a href="{names[n - 1]}.html" rel="prev">← {html.escape(chapters[n - 1]["title"])}</a>')
        else:
            pager.append("<span></span>")
        if n + 1 < len(chapters):
            pager.append(f'<a href="{names[n + 1]}.html" rel="next">{html.escape(chapters[n + 1]["title"])} →</a>')
        page_body = f"""<div class="book">
<nav class="chapters" aria-label="Guide"><ol>{toc}</ol></nav>
<article>
{body}
<nav class="pager" aria-label="Chapter">{"".join(pager)}</nav>
</article>
</div>"""
        pages.append(Page(f"guide/{names[n]}.html", chapter["title"], "guide", page_body))
    return pages


def gallery_page(scenes: list[dict], clips: list[showcase.Clip]) -> Page:
    by_category: dict[str, list[dict]] = {}
    for scene in sorted(scenes, key=lambda s: s["name"]):
        by_category.setdefault(scene["category"], []).append(scene)
    order = [c for c in gallery.CATEGORIES if c in by_category] + sorted(
        set(by_category) - set(gallery.CATEGORIES)
    )
    toc = "".join(
        f'<li><a href="#{cat}">{html.escape(by_category[cat][0]["category_label"])}</a></li>'
        for cat in order
    )
    clip_items = "\n".join(
        f'<li><figure>{clip_img(clip, "../")}<figcaption>'
        f'<a href="#{html.escape(clip.scene)}">{html.escape(clip.scene)}</a> · '
        f't = {clip.start:g}–{clip.start + clip.seconds:g} s</figcaption></figure></li>'
        for clip in clips
    )
    sections = []
    for cat in order:
        items = []
        for scene in by_category[cat]:
            name = scene["name"]
            if not (STILLS / f"{name}.png").is_file():
                fail(f"no still for {name}; run `python3 scripts/gallery.py`")
            words = name.replace("_", " ")
            alt = f"{words}, a still at 20 seconds: {scene['description']}"
            items.append(
                f'<li id="{name}"><figure>'
                f'<a href="../play/index.html?scene={name}"><img src="../stills/{name}.png" '
                f'alt="{html.escape(alt)}" width="{STILL_W}" height="{STILL_H}" loading="lazy"></a>'
                f'<figcaption><p class="name"><a href="../play/index.html?scene={name}">{name}</a></p>'
                f'<p class="desc">{html.escape(scene["title"])}. {html.escape(scene["description"])}.</p>'
                f"</figcaption></figure></li>"
            )
        label = html.escape(by_category[cat][0]["category_label"])
        sections.append(
            f'<h2 id="{cat}">{label}</h2>\n<ul class="scenes">\n' + "\n".join(items) + "\n</ul>"
        )
    body = f"""<h1>Gallery</h1>
<p class="lede">Every one of the {len(scenes)} scenes, as sciart draws it at 160×48 cells after 20
simulated seconds (seed 42, astro theme). Select a scene to play it live.</p>
<ul class="toc">{toc}</ul>
<h2 id="in-motion">In motion</h2>
<ul class="clips">
{clip_items}
</ul>
{chr(10).join(sections)}"""
    return Page("gallery/index.html", "Gallery", "gallery", body)


def player_page(clips: list[showcase.Clip]) -> Page:
    hero = next(clip for clip in clips if clip.readme == "hero")
    data = {
        "module": "../assets/sciart_web.wasm",
        "default_scene": hero.scene,
        "clips": {
            clip.scene: {"src": f"../media/{Path(clip.out).name}", "alt": clip.alt}
            for clip in clips
        },
    }
    # `</` cannot end the JSON block early: escape it.
    payload = json.dumps(data, ensure_ascii=False).replace("</", "<\\/")
    others = "".join(
        f'<li><a href="../media/{Path(clip.out).name}">{html.escape(clip.scene)}</a></li>'
        for clip in clips
    )
    body = f"""<h1>Player</h1>
<p class="lede">The scenes running in your browser: the same code as the terminal program,
compiled to WebAssembly, drawn at 8×16 pixels per cell. Choose a scene, a theme and a seed.</p>
<section class="player" id="player" aria-label="Scene player">
<div class="player-stage" id="stage">
<figure class="fallback" id="fallback">{clip_img(hero, "../")}
<figcaption id="fallback-note">A recorded clip. The live player needs JavaScript and WebAssembly.</figcaption></figure>
<canvas id="screen" hidden tabindex="0" aria-label="The selected scene, animated; keys work while it has focus"></canvas>
</div>
<form class="controls" id="controls" hidden>
<label>Scene <select id="scene"></select></label>
<label>Theme <select id="theme"></select></label>
<label>Seed <input id="seed" type="number" min="0" step="1" inputmode="numeric"></label>
<button type="button" id="play">Pause</button>
<button type="button" id="restart">Restart</button>
<button type="button" id="reseed">New seed</button>
</form>
<p class="status" id="status"></p>
</section>
<p class="note">Keys, once you click the picture or tab into the player: <kbd>space</kbd> pause,
<kbd>n</kbd> and <kbd>p</kbd> (or the arrows) next and previous scene, <kbd>t</kbd> next theme,
<kbd>r</kbd> restart. With reduced motion turned on in your system, the player starts paused.
The browser is its own platform: frames can differ from a terminal's in the last bits of the
arithmetic.</p>
<details>
<summary>Recorded clips</summary>
<ul>{others}</ul>
</details>
<script type="application/json" id="player-data">{payload}</script>
<script src="../assets/player.js" defer></script>"""
    return Page("play/index.html", "Player", "play", body)


def copy_assets(clips: list[showcase.Clip], scenes: list[dict]) -> None:
    assets = DIST / "assets"
    assets.mkdir(parents=True)
    for name in ("style.css", "player.js", "icon.svg"):
        shutil.copy2(STATIC / name, assets / name)
    if not WASM.is_file():
        fail(f"{WASM.relative_to(ROOT)} is missing; run `pixi run web-build`")
    shutil.copy2(WASM, assets / "sciart_web.wasm")
    media = DIST / "media"
    media.mkdir()
    for clip in clips:
        shutil.copy2(clip.path, media / Path(clip.out).name)
    stills = DIST / "stills"
    stills.mkdir()
    for scene in scenes:
        shutil.copy2(STILLS / f"{scene['name']}.png", stills / f"{scene['name']}.png")
    # GitHub Pages: serve the files as they are.
    (DIST / ".nojekyll").write_text("", encoding="utf-8")


def main(argv: list[str]) -> int:
    if argv:
        fail("takes no arguments")
    scenes = catalogue()
    clips = showcase.load_manifest()
    build = build_label()
    pages = [home(clips, scenes), gallery_page(scenes, clips), player_page(clips), *guide_pages()]
    if DIST.exists():
        shutil.rmtree(DIST)
    DIST.mkdir(parents=True)
    copy_assets(clips, scenes)
    for page in pages:
        target = DIST / page.path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(page_html(page, build), encoding="utf-8")
    total = sum(p.stat().st_size for p in DIST.rglob("*") if p.is_file())
    print(
        f"site: wrote {len(pages)} pages, {len(scenes)} gallery scenes and {len(clips)} clips "
        f"to {DIST.relative_to(ROOT)} ({total / 1e6:.1f} MB), {build}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
