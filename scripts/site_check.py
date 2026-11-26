#!/usr/bin/env python3
"""Check the built docs site in site/dist/ (WEB-R3, WEB-R4, WEB-R5, WEB-R6, WEB-R7, WEB-R9).

    python3 scripts/site_check.py    # pixi run site-check (builds the module and the site first)

WEB-R3: the player module is a WebAssembly module that imports nothing (no JavaScript glue,
no host functions, so no clock and no operating-system RNG reach it) and exports the player.
The pixel equality of a player frame and the capture renderer is a Rust test in
web/tests/player.rs (`pixi run web-test`).
WEB-R4: the module is at most 3 MiB before compression, and the player page loads only the
site's own stylesheet, script and module.
WEB-R5: every guide chapter, the gallery and the player page are built; the player page has
its clip fallback, its reduced-motion start and its keys; every image has alt text.
WEB-R6: no page, stylesheet or script loads a script, style, font, image, frame or other
resource from another origin, however it is written: a fetching attribute on any element
(SVG `href`/`xlink:href` included), a style rule in any letter case or `image-set`, the
player's data (every address in it is a relative path), a `<base>` element, a refresh
redirect, `ping` or an import map. The site's scripts load only literal relative paths or
values from the player's data; a computed address passed to `fetch`, `import()`, a worker,
`WebSocket`, `EventSource`, `sendBeacon`, `open` or assigned to `src`/`href`/`location`
fails. Nothing names an analytics or tracking service. Ordinary links (`<a href>`) to other
sites are allowed. `scripts/site_check_test.py` proves each case.
WEB-R7: every registered scene is in the gallery; every internal link and asset resolves
(fragments included); the install commands on the site are the README's Install section's,
and its one-line `curl … | sh` follows redirects with its https-only flags (DIST-R7); and
the guide names no subcommand, flag or flag value the binary does not have (checked
against `sciart completions bash`).
WEB-R2: the gallery's headings list the categories in the README's domain order, each under
its domain heading (`DOMAINS` in scripts/showcase.py); the player's data carries the same
category order and player.js groups its scene picker by it, so the picker and `n`/`p` walk
the scenes in the gallery's order.
WEB-R9: the README's opening sentence, the crate `description`, the command line's `about`
(or its short form), the site's lede and every page's meta description, and the opening
lines of the architecture guide and the user guide carry the one description; the Homebrew
`desc` is its part before the colon, within 80 characters; and no user-facing page or built
site page describes the project as "science & astronomy" or "science and astronomy".
`scripts/site_check_test.py` proves each case.
"""

from __future__ import annotations

import html
import json
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

import build_site as site_build
from installer_check import one_liner_problems

ROOT = Path(__file__).resolve().parent.parent
DIST = site_build.DIST
GUIDE = site_build.GUIDE

# WEB-R4: may only be lowered without a spec change.
MODULE_BUDGET = 3 * 1024 * 1024
MODULE = DIST / "assets" / "sciart_web.wasm"
PLAYER_EXPORTS = {
    "memory",
    "input",
    "output_ptr",
    "output_len",
    "scenes_json",
    "theme_names",
    "player_new",
    "player_free",
    "player_advance",
    "player_resize",
    "player_frame",
    "player_width",
    "player_height",
    "player_scene",
}

# WEB-R6: names that only an analytics or tracking setup would contain.
TRACKERS = (
    "google-analytics",
    "googletagmanager",
    "gtag(",
    "analytics.js",
    "plausible",
    "matomo",
    "piwik",
    "segment.com",
    "segment.io",
    "mixpanel",
    "hotjar",
    "clarity.ms",
    "umami",
    "usefathom",
    "cloudflareinsights",
    "doubleclick",
    "fbq(",
    "connect.facebook",
    "sendbeacon",
    "statcounter",
    "quantserve",
)
# Attributes whose value the browser fetches, on any element. `href` is a link only on `<a>`
# and `<area>`; everywhere else (`<link>`, SVG `<use>`, `<image>`, `<feImage>`) it loads.
FETCHED_ATTRS = {"src", "srcset", "imagesrcset", "poster", "data", "background", "href", "xlink:href"}
LINK_TAGS = {"a", "area"}
# Keys in the player's data whose values are addresses (WEB-R6: every one is relative).
URL_KEYS = {"module", "src", "href", "url", "srcset", "poster"}


def is_external(url: str) -> bool:
    """A reference to another origin: an absolute URL with a scheme, or `//host/…`."""
    url = url.strip()
    return bool(re.match(r"^[a-z][a-z0-9+.-]*:", url, re.I)) or url.startswith("//")


def is_relative_path(url: str) -> bool:
    """A literal relative path: no scheme, no `//host`, not rooted at `/`."""
    url = url.strip()
    return bool(url) and not is_external(url) and not url.startswith(("/", "\\"))


class Page(HTMLParser):
    """The references, ids, images, scripts and WEB-R6 hazards of one HTML or SVG file."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.fetches: list[tuple[str, str]] = []  # (tag, url) the browser loads
        self.links: list[str] = []  # <a href>
        self.ids: set[str] = set()
        self.images: list[tuple[str, str | None]] = []  # (src, alt)
        self.styles: list[str] = []  # inline `style` attributes and `<style>` bodies
        self.scripts: list[str] = []  # inline classic and module scripts
        self.data: list[tuple[str, str]] = []  # (id, text) of `application/json` scripts
        self.hazards: list[str] = []  # elements WEB-R6 forbids outright
        self._script: tuple[str, str] | None = None  # (type, id) while inside a script
        self._in_style = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        a = {k.lower(): (v or "") for k, v in attrs}
        if "id" in a:
            self.ids.add(a["id"])
        if tag == "a" and "name" in a:
            self.ids.add(a["name"])
        if tag == "base":
            self.hazards.append("a <base> element (it re-points every relative address)")
        if tag == "meta" and a.get("http-equiv", "").strip().lower() == "refresh":
            self.hazards.append(f"a refresh redirect (<meta http-equiv=refresh content={a.get('content', '')!r}>)")
        if "ping" in a:
            self.hazards.append(f"a ping to {a['ping']!r}")
        for key, value in a.items():
            if key in FETCHED_ATTRS and not (key == "href" and tag in LINK_TAGS):
                if key in ("srcset", "imagesrcset"):
                    urls = [c.split()[0] for c in value.split(",") if c.strip()]
                else:
                    urls = [value]
                for url in urls:
                    self.fetches.append((tag, url))
            if key == "style":
                self.styles.append(value)
        if tag in LINK_TAGS and "href" in a:
            self.links.append(a["href"])
        if tag == "img":
            self.images.append((a.get("src", ""), dict(attrs).get("alt")))
        if tag == "script":
            kind = a.get("type", "").strip().lower()
            if kind == "importmap":
                self.hazards.append("an import map")
            self._script = (kind, a.get("id", ""))
        if tag == "style":
            self._in_style = True

    def handle_endtag(self, tag: str) -> None:
        if tag == "script":
            self._script = None
        if tag == "style":
            self._in_style = False

    def handle_data(self, data: str) -> None:
        if self._script is not None and data.strip():
            kind, ident = self._script
            if kind in ("application/json", "application/ld+json"):
                self.data.append((ident, data))
            elif kind in ("", "text/javascript", "module", "application/javascript"):
                self.scripts.append(data)
        if self._in_style:
            self.styles.append(data)


def parse(path: Path) -> Page:
    page = Page()
    page.feed(path.read_text(encoding="utf-8"))
    return page


def css_urls(css: str) -> list[str]:
    """Every address a style sheet names: `url(…)` in any letter case and every quoted string
    (which covers `@import "…"` and `image-set("…" 1x)`)."""
    urls = [m.group(2) for m in re.finditer(r"url\(\s*(['\"]?)(.*?)\1\s*\)", css, re.I | re.S)]
    urls += [m.group(2) for m in re.finditer(r"(['\"])(.*?)\1", css, re.S)]
    return [u.strip() for u in urls]


def css_problems(css: str, where: str) -> list[str]:
    """WEB-R6: a style sheet that names another origin, however it is written."""
    return [
        f"WEB-R6: {where} loads {url} from another origin (style)"
        for url in css_urls(css)
        if is_external(url) and not url.lower().startswith("data:")
    ]


# ---- WEB-R6: scripts -----------------------------------------------------------------------------

# Calls whose argument the browser loads; `.open(` covers XMLHttpRequest and window.open.
LOAD_CALLS = re.compile(
    r"(?<![\w$])(?:fetch|import|importScripts|(?:new\s+)?(?:Shared)?Worker|(?:new\s+)?WebSocket"
    r"|(?:new\s+)?EventSource|sendBeacon|location\s*\.\s*(?:assign|replace))\s*\(|\.\s*open\s*\("
)
# Assignments of an address: `x.src = …`, `x.href = …`, `location = …`.
LOAD_ASSIGNMENTS = re.compile(
    r"(?:\.\s*(?:src|href|srcset|data|poster|action)|(?<![\w$.])location)\s*=(?![=>])"
)
STATIC_IMPORT = re.compile(r"(?<![\w$.])(?:import|export)\b[^;()]*?\bfrom\s*(['\"])(.*?)\1|(?<![\w$.])import\s*(['\"])(.*?)\3")
DATA_VALUE = re.compile(r"^data(?:\s*\.\s*[A-Za-z_$][\w$]*|\s*\[[^\[\]]+\])+$")
METHOD = re.compile(r"^(['\"])[A-Z]+\1$")


def strip_comments(js: str) -> str:
    """`js` with comments blanked and strings kept (enough for the site's own scripts)."""
    out = []
    i = 0
    while i < len(js):
        c = js[i]
        if c in "'\"`":
            j = i + 1
            while j < len(js) and js[j] != c:
                j += 2 if js[j] == "\\" else 1
            out.append(js[i : j + 1])
            i = j + 1
        elif js.startswith("//", i):
            j = js.find("\n", i)
            i = len(js) if j < 0 else j
        elif js.startswith("/*", i):
            j = js.find("*/", i + 2)
            i = len(js) if j < 0 else j + 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


def split_args(js: str, start: int) -> list[str]:
    """The top-level arguments of the call whose `(` is just before `start`."""
    args, depth, i, begin = [], 0, start, start
    while i < len(js):
        c = js[i]
        if c in "'\"`":
            j = i + 1
            while j < len(js) and js[j] != c:
                j += 2 if js[j] == "\\" else 1
            i = j + 1
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:
                args.append(js[begin:i].strip())
                return [a for a in args if a]
            depth -= 1
        elif c == "," and depth == 0:
            args.append(js[begin:i].strip())
            begin = i + 1
        i += 1
    args.append(js[begin:].strip())
    return [a for a in args if a]


def expression_at(js: str, start: int) -> str:
    """The expression from `start` to the end of its statement (`;`, a newline at depth 0)."""
    depth, i = 0, start
    while i < len(js):
        c = js[i]
        if c in "'\"`":
            j = i + 1
            while j < len(js) and js[j] != c:
                j += 2 if js[j] == "\\" else 1
            i = j + 1
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:
                break
            depth -= 1
        elif c in ";\n," and depth == 0:
            break
        i += 1
    return js[start:i].strip()


def allowed_address(expr: str) -> bool:
    """WEB-R6: a literal relative path, or a value read from the player's data."""
    expr = expr.strip()
    # One string literal and nothing else: `"a" + "b"` is a computed address.
    literal = (
        re.fullmatch(r'"((?:[^"\\\n]|\\.)*)"', expr)
        or re.fullmatch(r"'((?:[^'\\\n]|\\.)*)'", expr)
        or re.fullmatch(r"`([^`$\\]*)`", expr)
    )
    if literal:
        return is_relative_path(literal.group(literal.lastindex))
    return bool(DATA_VALUE.match(expr))


def script_problems(js: str, where: str) -> list[str]:
    """WEB-R6: a site script fetches and imports only literal relative paths (or values
    from the player's data); anything computed fails, as does any absolute address it
    names."""
    code = strip_comments(js)
    problems = []
    for match in LOAD_CALLS.finditer(code):
        call = re.sub(r"\s+", " ", match.group(0).rstrip("( "))
        args = split_args(code, match.end())
        if call.endswith("open"):
            args = [a for a in args if not METHOD.match(a)][:1]
        elif call.endswith("sendBeacon") or call.endswith("WebSocket") or call.endswith("Worker") \
                or call.endswith("EventSource") or call in ("fetch", "import", "importScripts") \
                or "location" in call:
            args = args[:1]
        for arg in args:
            if not allowed_address(arg):
                problems.append(f"WEB-R6: {where}: `{call}({arg})` loads an address that is not a literal relative path")
    for match in LOAD_ASSIGNMENTS.finditer(code):
        value = expression_at(code, match.end())
        if not allowed_address(value):
            target = re.sub(r"\s+", "", match.group(0))
            problems.append(f"WEB-R6: {where}: `{target} {value}` assigns an address that is not a literal relative path")
    for match in STATIC_IMPORT.finditer(code):
        path = match.group(2) if match.group(2) is not None else match.group(4)
        if not is_relative_path(path):
            problems.append(f"WEB-R6: {where}: imports {path!r}, not a relative path")
    for literal in re.findall(r"(['\"`])((?:[a-z][a-z0-9+.-]*:)?//[^'\"`]*)\1", code, re.I):
        problems.append(f"WEB-R6: {where} names another origin: {literal[1]!r}")
    return problems


def data_problems(ident: str, text: str, where: str) -> list[str]:
    """WEB-R6: the player's data parses, and every address in it is a relative path."""
    try:
        value = json.loads(text)
    except json.JSONDecodeError as err:
        return [f"WEB-R6: {where}: the data block #{ident} is not JSON ({err})"]
    problems = []

    def walk(node: object, key: str | None) -> None:
        if isinstance(node, dict):
            for k, v in node.items():
                walk(v, k)
        elif isinstance(node, list):
            for v in node:
                walk(v, key)
        elif isinstance(node, str):
            if key is not None and key.lower() in URL_KEYS and not is_relative_path(node):
                problems.append(f"WEB-R6: {where}: data #{ident} `{key}` is {node!r}, not a relative path")
            elif re.match(r"^(?:[a-z][a-z0-9+.-]*:)?//", node.strip(), re.I):
                problems.append(f"WEB-R6: {where}: data #{ident} names another origin: {node!r}")

    walk(value, None)
    return problems


def origin_problems(dist: Path) -> list[str]:
    """WEB-R6 over a built site: nothing loads from another origin, however it is written,
    and nothing names an analytics or tracking service."""
    problems: list[str] = []
    for path in sorted(dist.rglob("*")):
        if not path.is_file() or path.suffix not in (".html", ".svg", ".css", ".js", ".json", ".mjs"):
            continue
        rel = str(path.relative_to(dist))
        text = path.read_text(encoding="utf-8")
        lower = text.lower()
        for tracker in TRACKERS:
            if tracker in lower:
                problems.append(f"WEB-R6: {rel} mentions the tracker `{tracker}`")
        if path.suffix in (".html", ".svg"):
            page = Page()
            page.feed(text)
            for hazard in page.hazards:
                problems.append(f"WEB-R6: {rel} has {hazard}")
            for tag, url in page.fetches:
                if is_external(url) and not url.strip().lower().startswith("data:"):
                    problems.append(f"WEB-R6: {rel} loads <{tag}> {url} from another origin")
            for css in page.styles:
                problems += css_problems(css, rel)
            for script in page.scripts:
                problems += script_problems(script, f"{rel} (inline script)")
            for ident, data in page.data:
                problems += data_problems(ident, data, rel)
        elif path.suffix == ".css":
            problems += css_problems(text, rel)
        elif path.suffix in (".js", ".mjs"):
            problems += script_problems(text, rel)
        elif path.suffix == ".json":
            problems += data_problems(path.name, text, rel)
    return list(dict.fromkeys(problems))


def resolve(page: Path, url: str) -> tuple[Path, str]:
    """A relative reference from `page`: the file (a directory means its index.html) and
    the fragment."""
    parts = urlsplit(url)
    target = (page.parent / unquote(parts.path)).resolve() if parts.path else page.resolve()
    if target.is_dir():
        target = target / "index.html"
    return target, parts.fragment


# ---- WEB-R3 / WEB-R4: the module ---------------------------------------------------------------


def wasm_sections(data: bytes) -> dict[int, bytes]:
    def leb(pos: int) -> tuple[int, int]:
        result = shift = 0
        while True:
            byte = data[pos]
            pos += 1
            result |= (byte & 0x7F) << shift
            shift += 7
            if byte < 0x80:
                return result, pos

    sections: dict[int, bytes] = {}
    pos = 8
    while pos < len(data):
        sid = data[pos]
        size, pos = leb(pos + 1)
        sections.setdefault(sid, b"")
        sections[sid] += data[pos : pos + size]
        pos += size
    return sections


def wasm_exports(section: bytes) -> set[str]:
    def leb(pos: int) -> tuple[int, int]:
        result = shift = 0
        while True:
            byte = section[pos]
            pos += 1
            result |= (byte & 0x7F) << shift
            shift += 7
            if byte < 0x80:
                return result, pos

    count, pos = leb(0)
    names = set()
    for _ in range(count):
        length, pos = leb(pos)
        names.add(section[pos : pos + length].decode("utf-8"))
        pos += length + 1  # the kind byte
        _, pos = leb(pos)  # the index
    return names


def check_module(problems: list[str]) -> None:
    if not MODULE.is_file():
        problems.append("WEB-R3: site/dist/assets/sciart_web.wasm is missing (pixi run site-build)")
        return
    data = MODULE.read_bytes()
    if data[:8] != b"\0asm\x01\0\0\0":
        problems.append("WEB-R3: sciart_web.wasm is not a WebAssembly 1 module")
        return
    if len(data) > MODULE_BUDGET:
        problems.append(
            f"WEB-R4: the module is {len(data) / 2**20:.2f} MiB, over the "
            f"{MODULE_BUDGET / 2**20:.0f} MiB budget"
        )
    sections = wasm_sections(data)
    if sections.get(2):
        problems.append(
            "WEB-R3: the module imports from its host (it must need nothing: no clock, no RNG)"
        )
    missing = PLAYER_EXPORTS - wasm_exports(sections.get(7, b"\0"))
    if missing:
        problems.append(f"WEB-R3: the module does not export {', '.join(sorted(missing))}")


# ---- WEB-R5 / WEB-R6 / WEB-R7 over the built files -------------------------------------------


def check_files(problems: list[str], registered: list[str]) -> dict[Path, Page]:
    pages = {path.resolve(): parse(path) for path in sorted(DIST.rglob("*.html"))}
    chapters = site_build.tomllib.loads((GUIDE / "chapters.toml").read_text(encoding="utf-8"))
    expected = [DIST / "index.html", DIST / "gallery" / "index.html", DIST / "play" / "index.html"]
    expected += [DIST / "guide" / f"{Path(ch['file']).stem}.html" for ch in chapters["chapters"]]
    for path in expected:
        if path.resolve() not in pages:
            problems.append(f"WEB-R5: {path.relative_to(ROOT)} was not built")

    for path, page in pages.items():
        name = path.relative_to(DIST.resolve())
        # WEB-R5: every image has alt text.
        for src, alt in page.images:
            if alt is None or not alt.strip():
                problems.append(f"WEB-R5: {name}: <img src={src!r}> has no alt text")
        # WEB-R7: internal references resolve, fragments included.
        for kind, url in [("asset", u) for _, u in page.fetches] + [("link", u) for u in page.links]:
            if not url or is_external(url):
                continue
            target, fragment = resolve(path, url)
            if not target.exists():
                problems.append(f"WEB-R7: {name}: {kind} {url} does not resolve")
            elif fragment and target.suffix == ".html":
                ids = pages[target.resolve()].ids if target.resolve() in pages else set()
                if fragment not in ids:
                    problems.append(f"WEB-R7: {name}: {url} names no element #{fragment}")

    # WEB-R7: a style sheet's own references resolve.
    for path in sorted(DIST.rglob("*.css")):
        for url in re.findall(r"url\(\s*['\"]?([^'\")]+)['\"]?\s*\)", path.read_text(encoding="utf-8"), re.I):
            if not is_external(url) and not (path.parent / url.split("#")[0]).exists():
                problems.append(f"WEB-R7: {path.relative_to(DIST)}: {url} does not resolve")
    # WEB-R7: the player's data names files that exist.
    player_page = (DIST / "play" / "index.html").resolve()
    for ident, text in pages[player_page].data if player_page in pages else []:
        try:
            data = json.loads(text)
        except json.JSONDecodeError:
            continue  # WEB-R6 reports it
        clips = data.get("clips", {}) if isinstance(data, dict) else {}
        refs = [data.get("module", "")] + [c.get("src", "") for c in clips.values() if isinstance(c, dict)]
        for ref in refs:
            if is_relative_path(ref) and not resolve(player_page, ref)[0].exists():
                problems.append(f"WEB-R7: play/index.html: data #{ident} names {ref}, which does not resolve")

    # WEB-R7: one gallery entry per registered scene.
    gallery_page = pages.get((DIST / "gallery" / "index.html").resolve())
    if gallery_page:
        stills = {Path(src).stem for src, _ in gallery_page.images if "/stills/" in src}
        for scene in registered:
            if scene not in gallery_page.ids or scene not in stills:
                problems.append(f"WEB-R7: the gallery has no entry for {scene}")
    # WEB-R4 / WEB-R5: the player page.
    player = DIST / "play" / "index.html"
    if player.is_file():
        page = pages[player.resolve()]
        loaded = sorted({Path(url).name for _, url in page.fetches if not is_external(url)})
        allowed = {"style.css", "player.js", "icon.svg"} | {Path(src).name for src, _ in page.images}
        for name in loaded:
            if name not in allowed:
                problems.append(f"WEB-R4: the player page loads {name} besides the module")
        text = player.read_text(encoding="utf-8")
        if 'id="fallback"' not in text or "/media/" not in text:
            problems.append("WEB-R5: the player page has no recorded-clip fallback")
        script = (DIST / "assets" / "player.js").read_text(encoding="utf-8")
        for needle, what in (
            ("prefers-reduced-motion", "starts paused for reduced motion"),
            ("keydown", "can be driven from the keyboard"),
            ("sciart_web.wasm", "loads the module"),
        ):
            if needle not in script and needle not in text:
                problems.append(f"WEB-R5: the player no longer {what}")
    return pages


# ---- WEB-R7: install commands and the guide's command lines ---------------------------------


def shell_commands(markdown: str) -> list[str]:
    """The non-comment lines of the ```sh blocks in `markdown`."""
    commands = []
    for block in re.findall(r"^```(?:sh|bash|console)?\s*\n(.*?)^```", markdown, re.S | re.M):
        for line in block.splitlines():
            line = line.split(" #", 1)[0].strip()
            if line and not line.startswith("#"):
                commands.append(line)
    return commands


def check_install(problems: list[str]) -> None:
    readme = shell_commands(site_build.readme_section("Install"))
    installing = DIST / "guide" / "installing.html"
    if not installing.is_file():
        return
    text = installing.read_text(encoding="utf-8")
    sections = re.findall(r'<section class="readme-include" data-source="README.md#Install">(.*?)</section>', text, re.S)
    if not sections:
        problems.append("WEB-R7: the install page does not carry the README's Install section")
        return
    site = []
    for block in re.findall(r"<pre><code[^>]*>(.*?)</code></pre>", sections[0], re.S):
        for line in html.unescape(block).splitlines():
            line = line.split(" #", 1)[0].strip()
            if line and not line.startswith("#"):
                site.append(line)
    if site != readme:
        problems.append(
            "WEB-R7: the install commands on the site differ from README.md's Install section:\n"
            f"    site:   {site}\n    README: {readme}"
        )
    # No other page may give an install command the README does not.
    install_like = re.compile(r"^(cargo (install|binstall)|brew install|pixi (global )?(install|run rs-install)|curl .*\| *sh)")
    for path in sorted(DIST.rglob("*.html")):
        for block in re.findall(r"<pre><code[^>]*>(.*?)</code></pre>", path.read_text(encoding="utf-8"), re.S):
            for line in html.unescape(block).splitlines():
                line = line.split(" #", 1)[0].strip()
                if install_like.match(line) and line not in readme:
                    problems.append(f"WEB-R7: {path.relative_to(DIST)} gives an install command not in the README: {line}")
                # DIST-R7: the one-line command the site shows follows redirects and keeps
                # its https-only flags (the rule scripts/installer_check.py applies to the
                # README and the installer's header).
                if re.match(r"^curl .*\|\s*sh$", line):
                    problems += [f"DIST-R7: {path.relative_to(DIST)}: {p}" for p in one_liner_problems(line)]


# ---- WEB-R2: the gallery's domain order ---------------------------------------------------


def gallery_order_problems(page: Path, catalogue: list[dict]) -> list[str]:
    """WEB-R2: the built gallery's headings list the categories in the README's domain
    order, each under its domain heading (`showcase.DOMAINS`)."""
    if not page.is_file():
        return []  # WEB-R5 reports a gallery that was not built
    text = page.read_text(encoding="utf-8")
    headings = [squash(h) for h in re.findall(r"<h[23]\b[^>]*>(.*?)</h[23]>", text, re.S)]
    labels = {scene["category"]: scene["category_label"] for scene in catalogue}
    return [
        f"WEB-R2: {p}"
        for p in site_build.showcase.heading_order_problems(
            "the site's gallery", headings, labels, set(labels)
        )
    ]


PLAYER_DATA = re.compile(r'<script type="application/json" id="player-data">(.*?)</script>', re.S)
# The player script orders its scene picker's category groups by the page's data, not by
# their labels (`.sort(... localeCompare ...)` over the groups).
GROUP_LABEL_SORT = re.compile(r"\[\.\.\.groups\]\s*\.sort\([^)]*\)\s*=>\s*a\[0\]\.localeCompare")


def player_order_problems(page: Path, script: Path, catalogue: list[dict]) -> list[str]:
    """WEB-R2: the player's scene picker (and `n`/`p`) walks the categories in the gallery's
    domain order: the page's data carries that order as `category_order`, and the script
    groups the scenes by it."""
    if not page.is_file() or not script.is_file():
        return []  # WEB-R5 reports a player that was not built
    match = PLAYER_DATA.search(page.read_text(encoding="utf-8"))
    try:
        data = json.loads(match.group(1)) if match else {}
    except json.JSONDecodeError:
        return []  # WEB-R6 reports it
    want = site_build.showcase.ordered_categories({scene["category"] for scene in catalogue})
    got = data.get("category_order") if isinstance(data, dict) else None
    problems = []
    if got != want:
        problems.append(
            f"WEB-R2: the player's data gives the category order {got!r}, want the gallery's "
            f"domain order {want!r}"
        )
    code = strip_comments(script.read_text(encoding="utf-8"))
    if "data.category_order" not in code or GROUP_LABEL_SORT.search(code):
        problems.append(
            "WEB-R2: player.js does not order its scene groups by the data's `category_order`"
        )
    return problems


# ---- WEB-R9: one description, science first ------------------------------------------------

DESCRIPTION = "Real-time generative science art for the terminal: physics, chemistry, biology, Earth and space"
# The part before the colon: the Homebrew `desc` (at most 80 characters), and the command
# line's `about` when the full sentence would break a size budget (RT-R22).
SHORT_DESCRIPTION = DESCRIPTION.split(":", 1)[0]
HOMEBREW_DESC_LIMIT = 80
# How the project described itself before WEB-R9: astronomy beside science as its subject.
ASTRONOMY_FIRST = re.compile(r"science\s+(?:&|&amp;|and)\s+astronomy", re.IGNORECASE)
# Files whose text users read as the project's own description (the CHANGELOG keeps its
# history and docs/specs its record, so neither is here).
USER_FACING = (
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/GALLERY.md",
    "docs/guide/*.md",
    "crates/sciart/Cargo.toml",
    "crates/sciart/src/cli.rs",
    "crates/sciart/src/lib.rs",
    "crates/sciart/src/main.rs",
    "packaging/homebrew/sciart.rb.in",
)


def squash(text: str) -> str:
    """`text` with tags dropped, entities decoded and whitespace runs made single spaces."""
    return " ".join(html.unescape(re.sub(r"<[^>]+>", " ", text)).split())


def first_paragraph(markdown: str, after: str | None = None) -> str:
    """The first prose paragraph of `markdown` (after the heading line `after`, if given):
    headings, badge lines and blank lines before it are skipped."""
    lines = markdown.splitlines()
    if after is not None:
        if after not in lines:
            return ""
        lines = lines[lines.index(after) + 1 :]
    paragraph: list[str] = []
    for line in lines:
        stripped = line.strip()
        if not paragraph and (not stripped or stripped.startswith(("#", "[![", "<"))):
            continue
        if not stripped:
            break
        paragraph.append(stripped)
    return squash(" ".join(paragraph))


def carries(text: str, phrase: str = DESCRIPTION) -> bool:
    """`text` contains `phrase`, ignoring letter case (a sentence may start in lower case)."""
    return phrase.lower() in squash(text).lower()


def description_problems(root: Path, dist: Path) -> list[str]:
    """WEB-R9: every place that describes the project carries the one description, and no
    user-facing page presents the project as science and astronomy."""
    problems: list[str] = []

    def read(rel: str) -> str:
        path = root / rel
        if not path.is_file():
            problems.append(f"WEB-R9: {rel} is missing")
            return ""
        return path.read_text(encoding="utf-8")

    readme = first_paragraph(read("README.md"))
    if not readme.startswith(DESCRIPTION):
        problems.append(f"WEB-R9: README.md does not open with the description: {readme[:100]!r}")
    cargo = re.search(r'^description\s*=\s*"([^"]*)"', read("crates/sciart/Cargo.toml"), re.M)
    if not cargo or cargo.group(1) != DESCRIPTION:
        problems.append("WEB-R9: crates/sciart/Cargo.toml's `description` is not the description")
    about = re.search(r'^\s*about\s*=\s*"([^"]*)"', read("crates/sciart/src/cli.rs"), re.M)
    if not about or about.group(1) not in (DESCRIPTION, SHORT_DESCRIPTION):
        problems.append(
            "WEB-R9: crates/sciart/src/cli.rs's `about` is neither the description nor its "
            "part before the colon"
        )
    architecture = first_paragraph(read("docs/ARCHITECTURE.md"), after="## Overview")
    if not carries(architecture):
        problems.append("WEB-R9: docs/ARCHITECTURE.md's overview does not open with the description")
    guide = first_paragraph(read("docs/guide/index.md"))
    if not carries(guide):
        problems.append("WEB-R9: docs/guide/index.md does not open with the description")
    desc = re.search(r'^\s*desc\s+"([^"]*)"', read("packaging/homebrew/sciart.rb.in"), re.M)
    if not desc or desc.group(1) != SHORT_DESCRIPTION:
        problems.append(
            f"WEB-R9: the Homebrew formula's `desc` is not {SHORT_DESCRIPTION!r}"
        )
    elif len(desc.group(1)) > HOMEBREW_DESC_LIMIT:
        problems.append(f"WEB-R9: the Homebrew `desc` is over {HOMEBREW_DESC_LIMIT} characters")

    # The built site: the home page's lede and every page's meta description.
    home = dist / "index.html"
    if home.is_file():
        lede = re.search(r'<p class="lede">(.*?)</p>', home.read_text(encoding="utf-8"), re.S)
        if not lede or not squash(lede.group(1)).startswith(DESCRIPTION):
            problems.append("WEB-R9: the site's home page lede does not open with the description")
    else:
        problems.append("WEB-R9: site/dist/index.html was not built")
    for page in sorted(dist.rglob("*.html")):
        text = page.read_text(encoding="utf-8")
        meta = re.search(r'<meta\s+name="description"\s+content="([^"]*)"', text)
        if not meta or not squash(meta.group(1)).startswith(DESCRIPTION):
            problems.append(
                f"WEB-R9: {page.relative_to(dist)}: the meta description does not open with "
                "the description"
            )

    # Astronomy first: the old description anywhere users read it.
    pages = [path for pattern in USER_FACING for path in sorted(root.glob(pattern))]
    pages += sorted(dist.rglob("*.html"))
    for path in pages:
        if ASTRONOMY_FIRST.search(path.read_text(encoding="utf-8")):
            where = path.relative_to(dist if path.is_relative_to(dist) else root)
            problems.append(
                f"WEB-R9: {where} describes the project as science and astronomy, astronomy "
                "first; use the description"
            )
    return problems


def completions() -> tuple[set[str], dict[str, set[str]], dict[str, dict[str, set[str] | None]]]:
    """From `sciart completions bash`: the subcommands, each command's flags, and for each
    flag that takes a value, its allowed values (None when any value goes)."""
    script = site_build.sciart("completions", "bash")
    blocks = re.split(r"^        (sciart(?:__subcmd__[a-z]+)?)\)\n", script, flags=re.M)
    subcommands: set[str] = set()
    flags: dict[str, set[str]] = {}
    values: dict[str, dict[str, set[str] | None]] = {}
    for name, body in zip(blocks[1::2], blocks[2::2]):
        command = "" if name == "sciart" else name.split("__subcmd__")[1]
        if command:
            subcommands.add(command)
        opts = re.search(r'opts="([^"]*)"', body)
        flags[command] = {o for o in (opts.group(1).split() if opts else []) if o.startswith("-")}
        values[command] = {}
        for flag, rest in re.findall(r"^                (--?[\w-]+)\)\n(.*?);;", body, re.S | re.M):
            words = re.search(r'compgen -W "([^"]*)"', rest)
            values[command][flag] = set(words.group(1).split()) if words else None
    return subcommands, flags, values


def check_guide_commands(problems: list[str]) -> None:
    subcommands, flags, values = completions()
    every_flag = set().union(*flags.values())
    for source in sorted(GUIDE.glob("*.md")):
        text = source.read_text(encoding="utf-8")
        rel = source.relative_to(ROOT)
        lines = []
        for block in re.findall(r"^```(?:sh|bash|console)?\s*\n(.*?)^```", text, re.S | re.M):
            lines += [line.strip() for line in block.splitlines()]
        spans = re.findall(r"`([^`\n]+)`", re.sub(r"^```.*?^```", "", text, flags=re.S | re.M))
        lines += [span.strip() for span in spans if span.strip().startswith("sciart")]
        for line in lines:
            line = line.removeprefix("$ ").split(" #", 1)[0]
            if not re.match(r"^sciart(\s|$)", line):
                continue
            tokens = line.split()[1:]
            command = ""
            expect_value: str | None = None
            for token in tokens:
                if token == "--":
                    break
                if expect_value is not None:
                    allowed = values[command].get(expect_value) or values[""].get(expect_value)
                    if allowed and token not in allowed and not token.startswith("<"):
                        problems.append(f"WEB-R7: {rel}: `{line}`: {expect_value} has no value {token!r}")
                    expect_value = None
                    continue
                if token.startswith("-"):
                    flag = token.split("=", 1)[0]
                    if flag not in flags.get(command, set()) and flag not in flags[""]:
                        problems.append(f"WEB-R7: {rel}: `{line}`: sciart {command or ''} has no {flag}".replace("  ", " "))
                    elif "=" not in token and (flag in values.get(command, {}) or flag in values[""]):
                        expect_value = flag
                    continue
                if not command and token in subcommands:
                    command = token
                    continue
                if not command and not token.startswith("<"):
                    problems.append(f"WEB-R7: {rel}: `{line}`: sciart has no subcommand {token!r}")
                    break
        # A flag named on its own in the prose must exist somewhere.
        for span in spans:
            for flag in re.findall(r"^(--[a-z][a-z0-9-]*)", span.strip()):
                if flag not in every_flag:
                    problems.append(f"WEB-R7: {rel}: `{span}` names a flag sciart does not have")


def main(argv: list[str]) -> int:
    if argv:
        sys.exit("site_check: takes no arguments")
    if not DIST.is_dir():
        sys.exit("site_check: site/dist/ is missing; run `pixi run site-build`")
    problems: list[str] = []
    catalogue = site_build.catalogue()
    registered = [scene["name"] for scene in catalogue]
    check_module(problems)
    problems += origin_problems(DIST)
    pages = check_files(problems, registered)
    check_install(problems)
    check_guide_commands(problems)
    problems += description_problems(ROOT, DIST)
    problems += gallery_order_problems(DIST / "gallery" / "index.html", catalogue)
    problems += player_order_problems(
        DIST / "play" / "index.html", DIST / "assets" / "player.js", catalogue
    )
    problems = list(dict.fromkeys(problems))
    if problems:
        print(f"site_check: {len(problems)} problem(s):", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    size = MODULE.stat().st_size
    print(
        f"site_check: ok, {len(pages)} pages, {len(registered)} scenes in the gallery, "
        f"module {size / 2**20:.2f} MiB of {MODULE_BUDGET / 2**20:.0f} MiB, nothing from "
        "another origin"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
