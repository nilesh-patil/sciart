#!/usr/bin/env python3
"""Check the built docs site in site/dist/ (WEB-R3, WEB-R4, WEB-R5, WEB-R6, WEB-R7).

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
(fragments included); the install commands on the site are the README's Install section's;
and the guide names no subcommand, flag or flag value the binary does not have (checked
against `sciart completions bash`).
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
    registered = [scene["name"] for scene in site_build.catalogue()]
    check_module(problems)
    problems += origin_problems(DIST)
    pages = check_files(problems, registered)
    check_install(problems)
    check_guide_commands(problems)
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
