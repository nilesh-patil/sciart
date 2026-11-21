#!/usr/bin/env python3
"""Unit tests for scripts/site_check.py's origin rule (WEB-R6): a built site that would load
anything from another origin fails, however the reference is written, and the site's own
pages and player script pass.

Usage: python3 scripts/site_check_test.py
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import site_check  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

PAGE = """<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>t</title>
<link rel="stylesheet" href="assets/style.css">{head}</head>
<body>{body}
<script type="application/json" id="player-data">{data}</script>
<script src="assets/player.js" defer></script>
</body>
</html>
"""
DATA = '{"module": "assets/sciart_web.wasm", "default_scene": "aurora", "clips": {"aurora": {"src": "media/aurora.gif", "alt": "Aurora: curtains"}}}'
CSS = "body { background: #000; font-family: system-ui, \"SF Mono\"; }\n"
JS = 'fetch(data.module).then((r) => r.arrayBuffer());\nimg.src = data.clips[scene].src;\n'


class Site:
    """A minimal built site: one page, a stylesheet, a script and the files they name."""

    def __init__(self, root: Path, head: str = "", body: str = "", data: str = DATA,
                 css: str = CSS, js: str = JS):
        self.root = root
        (root / "assets").mkdir(parents=True)
        (root / "media").mkdir()
        (root / "index.html").write_text(PAGE.format(head=head, body=body, data=data))
        (root / "assets" / "style.css").write_text(css)
        (root / "assets" / "player.js").write_text(js)
        (root / "assets" / "sciart_web.wasm").write_bytes(b"\0asm\x01\0\0\0")
        (root / "media" / "aurora.gif").write_bytes(b"GIF89a")


class OriginRule(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.n = 0

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def site(self, **parts: str) -> Path:
        self.n += 1
        root = Path(self.tmp.name) / f"site{self.n}"
        Site(root, **parts)
        return root

    def fails(self, **parts: str) -> None:
        problems = site_check.origin_problems(self.site(**parts))
        self.assertTrue(problems, f"expected a WEB-R6 problem for {parts}")
        self.assertTrue(all(p.startswith("WEB-R6") for p in problems), problems)

    def test_web_r6_a_self_contained_site_passes(self) -> None:
        self.assertEqual(site_check.origin_problems(self.site()), [])

    def test_web_r6_ordinary_links_to_other_sites_pass(self) -> None:
        body = '<a href="https://github.com/nilesh-patil/sciart">Source</a>'
        self.assertEqual(site_check.origin_problems(self.site(body=body)), [])

    # The player's data.

    def test_web_r6_a_module_on_another_origin_fails(self) -> None:
        self.fails(data=DATA.replace('"assets/sciart_web.wasm"', '"https://host/sciart_web.wasm"'))

    def test_web_r6_a_protocol_relative_module_fails(self) -> None:
        self.fails(data=DATA.replace('"assets/sciart_web.wasm"', '"//host/sciart_web.wasm"'))

    def test_web_r6_a_clip_on_another_origin_fails(self) -> None:
        self.fails(data=DATA.replace('"media/aurora.gif"', '"https://host/aurora.gif"'))

    def test_web_r6_player_data_that_is_not_json_fails(self) -> None:
        self.fails(data='{"module": ')

    # Elements and attributes.

    def test_web_r6_any_base_element_fails(self) -> None:
        self.fails(head='<base href="https://host/">')
        self.fails(head='<base href="./">')

    def test_web_r6_a_refresh_redirect_fails(self) -> None:
        self.fails(head='<meta http-equiv="refresh" content="0; url=https://host/">')
        self.fails(head="<META HTTP-EQUIV='Refresh' CONTENT='5;URL=https://host/'>")

    def test_web_r6_a_ping_fails(self) -> None:
        self.fails(body='<a href="guide/index.html" ping="https://host/track">Guide</a>')

    def test_web_r6_svg_references_to_another_origin_fail(self) -> None:
        self.fails(body='<svg aria-hidden="true"><image xlink:href="https://host/a.png"/></svg>')
        self.fails(body='<svg aria-hidden="true"><use href="https://host/sprite.svg#a"/></svg>')

    def test_web_r6_an_import_map_fails(self) -> None:
        self.fails(body='<script type="importmap">{"imports": {"x": "./x.js"}}</script>')

    def test_web_r6_a_script_from_another_origin_fails(self) -> None:
        self.fails(head="<script src='https://cdn.host/x.js'></script>")
        self.fails(head="<link rel=preload href=//cdn.host/font.woff2 as=font>")

    # Style rules.

    def test_web_r6_css_urls_in_any_letter_case_fail(self) -> None:
        self.fails(css="body { background: URL(https://host/a.png); }")
        self.fails(css="body { background: Url( 'https://host/a.png' ); }")
        self.fails(body='<p style="background:URL(https://host/a.png)">x</p>')
        self.fails(head="<style>@IMPORT 'https://host/a.css';</style>")

    def test_web_r6_image_set_with_a_bare_string_fails(self) -> None:
        self.fails(css='body { background-image: image-set("https://host/a.png" 1x); }')
        self.fails(css="body { background-image: -webkit-image-set('//host/a.png' 2x); }")

    # Scripts: only literal relative paths, or a value from the player's data.

    def test_web_r6_a_computed_fetch_fails(self) -> None:
        self.fails(js='fetch("ht" + "tps:/" + "/host");\n')
        self.fails(js='fetch(location.protocol + "/" + "/host");\n')
        self.fails(js="const u = base + 'x.wasm';\nfetch(u);\n")
        self.fails(js="fetch(`${origin}/x`);\n")

    def test_web_r6_every_load_sink_is_checked(self) -> None:
        for js in (
            'import("https://host/m.js");\n',
            "new Worker(workerUrl);\n",
            'new SharedWorker("https://host/w.js");\n',
            'new WebSocket("wss://host/socket");\n',
            "new EventSource(feed);\n",
            'navigator.sendBeacon("https://host/b", payload);\n',
            'xhr.open("GET", url);\n',
            'importScripts("https://host/s.js");\n',
            'img.src = "https://host/a.png";\n',
            "link.href = prefix + 'a.css';\n",
            'location = "https://host/";\n',
            'window.location.href = "https://host/";\n',
            'location.assign("https://host/");\n',
            'window.open("https://host/");\n',
            'import x from "https://host/m.js";\n',
        ):
            with self.subTest(js=js):
                self.fails(js=js)

    def test_web_r6_a_root_relative_path_is_not_relative(self) -> None:
        self.fails(js='fetch("/assets/sciart_web.wasm");\n')

    def test_web_r6_literal_relative_paths_and_player_data_pass(self) -> None:
        js = (
            'fetch("../assets/sciart_web.wasm");\n'
            "fetch(data.module);\n"
            "img.src = data.clips[scene].src;\n"
            'import("./more.js");\n'
            'xhr.open("GET", "data.json");\n'
            "// fetch(\"https://host/\") in a comment is not code\n"
            "if (a == b) { x = 1; }\n"
        )
        self.assertEqual(site_check.origin_problems(self.site(js=js)), [])

    def test_web_r6_an_inline_script_follows_the_same_rule(self) -> None:
        self.fails(body='<script>fetch("ht" + "tps://host/");</script>')

    def test_web_r6_trackers_fail(self) -> None:
        self.fails(body="<script>gtag('config', 'G-1');</script>")

    # The site's own files meet the rule as they are.

    def test_web_r6_the_player_script_meets_the_rule(self) -> None:
        script = (ROOT / "site" / "static" / "player.js").read_text(encoding="utf-8")
        self.assertEqual(site_check.script_problems(script, "player.js"), [])

    def test_web_r6_the_site_stylesheet_meets_the_rule(self) -> None:
        css = (ROOT / "site" / "static" / "style.css").read_text(encoding="utf-8")
        self.assertEqual(site_check.css_problems(css, "style.css"), [])


if __name__ == "__main__":
    unittest.main()
