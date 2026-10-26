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
resource from another origin, and nothing names an analytics or tracking service. Ordinary
links (`<a href>`) to other sites are allowed.
WEB-R7: every registered scene is in the gallery; every internal link and asset resolves
(fragments included); the install commands on the site are the README's Install section's;
and the guide names no subcommand, flag or flag value the binary does not have (checked
against `sciart completions bash`).
"""

from __future__ import annotations

import html
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
# HTML attributes that make the browser fetch something (as opposed to `<a href>`).
FETCHING = {
    ("script", "src"),
    ("img", "src"),
    ("img", "srcset"),
    ("source", "src"),
    ("source", "srcset"),
    ("video", "src"),
    ("video", "poster"),
    ("audio", "src"),
    ("track", "src"),
    ("iframe", "src"),
    ("frame", "src"),
    ("embed", "src"),
    ("object", "data"),
    ("input", "src"),
    ("image", "href"),
    ("use", "href"),
    ("link", "href"),
}


def is_external(url: str) -> bool:
    return bool(re.match(r"^[a-z][a-z0-9+.-]*:", url, re.I)) or url.startswith("//")


class Page(HTMLParser):
    """The references, ids and images of one HTML file."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.fetches: list[tuple[str, str]] = []  # (tag, url) the browser loads
        self.links: list[str] = []  # <a href>
        self.ids: set[str] = set()
        self.images: list[tuple[str, str | None]] = []  # (src, alt)
        self.inline_styles: list[str] = []
        self.scripts: list[str] = []
        self._in_script = False
        self._in_style = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        a = {k: (v or "") for k, v in attrs}
        if "id" in a:
            self.ids.add(a["id"])
        if tag == "a" and "name" in a:
            self.ids.add(a["name"])
        for key, value in a.items():
            if (tag, key) in FETCHING:
                urls = [c.split()[0] for c in value.split(",") if c.strip()] if key == "srcset" else [value]
                for url in urls:
                    self.fetches.append((tag, url))
            if key == "style":
                self.inline_styles.append(value)
        if tag == "a" and "href" in a:
            self.links.append(a["href"])
        if tag == "img":
            self.images.append((a.get("src", ""), dict(attrs).get("alt")))
        if tag == "script":
            self._in_script = True
        if tag == "style":
            self._in_style = True

    def handle_endtag(self, tag: str) -> None:
        if tag == "script":
            self._in_script = False
        if tag == "style":
            self._in_style = False

    def handle_data(self, data: str) -> None:
        if self._in_script:
            self.scripts.append(data)
        if self._in_style:
            self.inline_styles.append(data)


def css_urls(css: str) -> list[str]:
    urls = re.findall(r"url\(\s*['\"]?([^'\")]+)['\"]?\s*\)", css)
    urls += re.findall(r"@import\s+['\"]([^'\"]+)['\"]", css)
    return urls


def parse(path: Path) -> Page:
    page = Page()
    page.feed(path.read_text(encoding="utf-8"))
    return page


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
        # WEB-R6: everything the browser fetches is the site's own.
        for tag, url in page.fetches:
            if is_external(url) and not url.startswith("data:"):
                problems.append(f"WEB-R6: {name} loads <{tag}> {url} from another origin")
        for css in page.inline_styles:
            for url in css_urls(css):
                if is_external(url) and not url.startswith("data:"):
                    problems.append(f"WEB-R6: {name} loads {url} from another origin (inline style)")
        for script in page.scripts:
            if re.search(r"https?://|//[a-z0-9-]+\.[a-z]", script, re.I) and "application/json" not in script:
                if re.search(r"(fetch|import|XMLHttpRequest|WebSocket|EventSource)\b", script):
                    problems.append(f"WEB-R6: an inline script in {name} reaches another origin")
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

    for path in sorted(DIST.rglob("*")):
        if not path.is_file():
            continue
        text = None
        if path.suffix in (".css", ".js", ".html", ".svg", ".json"):
            text = path.read_text(encoding="utf-8")
        if text is None:
            continue
        rel = path.relative_to(DIST)
        lower = text.lower()
        for tracker in TRACKERS:
            if tracker in lower:
                problems.append(f"WEB-R6: {rel} mentions the tracker `{tracker}`")
        if path.suffix == ".css":
            for url in css_urls(text):
                if is_external(url) and not url.startswith("data:"):
                    problems.append(f"WEB-R6: {rel} loads {url} from another origin")
                elif not is_external(url) and not (path.parent / url.split("#")[0]).exists():
                    problems.append(f"WEB-R7: {rel}: {url} does not resolve")
        if path.suffix == ".js" and re.search(r"https?:|['\"`]//", text):
            problems.append(f"WEB-R6: {rel} names another origin")

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
