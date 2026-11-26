#!/usr/bin/env python3
"""Unit tests for scripts/site_check.py's origin rule (WEB-R6): a built site that would load
anything from another origin fails, however the reference is written, and the site's own
pages and player script pass; and for its description rule (WEB-R9): every place that
describes the project carries the one description, and astronomy-first wording fails.

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


D = site_check.DESCRIPTION
SHORT = site_check.SHORT_DESCRIPTION
# A tree that carries the description everywhere WEB-R9 names, keyed by path.
DESCRIBED = {
    "README.md": f"# sciart\n\n[![CI](x.svg)](y)\n\n{D}. **85 scenes**,\nat 30 FPS.\n",
    "crates/sciart/Cargo.toml": f'[package]\nname = "sciart"\ndescription = "{D}"\n',
    "crates/sciart/src/cli.rs": f'#[command(\n    name = "sciart",\n    about = "{D}",\n)]\n',
    "crates/sciart/src/lib.rs": f"//! {D}, as a library.\n",
    "crates/sciart/src/main.rs": "//! `sciart`, the binary.\n",
    "docs/ARCHITECTURE.md": f"# Architecture\n\n## Overview\n\nsciart is {D[0].lower()}{D[1:]}:\n85 scenes.\n",
    "docs/GALLERY.md": "# Gallery\n",
    "docs/guide/index.md": f"# Introduction\n\nsciart is {D[0].lower()}{D[1:]}.\nIt keeps your machine awake.\n",
    "packaging/homebrew/sciart.rb.in": f'class Sciart < Formula\n  desc "{SHORT}"\nend\n',
    "site/dist/index.html": f'<meta name="description" content="{D}: 85 scenes.">\n<p class="lede">{D}: 85\nscenes.</p>\n',
    "site/dist/guide/index.html": f'<meta name="description" content="{D}: 85 scenes.">\n',
}


class DescriptionRule(unittest.TestCase):
    """WEB-R9: one description, science first."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.n = 0

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def check(self, changed: dict[str, str | None]) -> list[str]:
        """`description_problems` over DESCRIBED with the files in `changed` replaced."""
        self.n += 1
        root = Path(self.tmp.name) / f"tree{self.n}"
        for rel, text in {**DESCRIBED, **changed}.items():
            if text is None:
                continue
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        return site_check.description_problems(root, root / "site" / "dist")

    def fails(self, changed: dict[str, str | None]) -> None:
        problems = self.check(changed)
        self.assertTrue(problems, f"expected a WEB-R9 problem for {changed}")
        self.assertTrue(all(p.startswith("WEB-R9") for p in problems), problems)

    def test_web_r9_a_tree_carrying_the_description_passes(self) -> None:
        self.assertEqual(self.check({}), [])

    def test_web_r9_the_short_about_passes(self) -> None:
        cli = f'#[command(\n    about = "{SHORT}",\n)]\n'
        self.assertEqual(self.check({"crates/sciart/src/cli.rs": cli}), [])

    def test_web_r9_the_description_is_under_eighty_characters_short(self) -> None:
        self.assertLessEqual(len(SHORT), site_check.HOMEBREW_DESC_LIMIT)
        self.assertEqual(SHORT, "Real-time generative science art for the terminal")

    def test_web_r9_a_readme_that_opens_otherwise_fails(self) -> None:
        self.fails({"README.md": f"# sciart\n\nArt for the terminal. {D}.\n"})

    def test_web_r9_a_different_crate_description_fails(self) -> None:
        self.fails({"crates/sciart/Cargo.toml": 'description = "Generative art in your terminal"\n'})

    def test_web_r9_a_different_about_fails(self) -> None:
        self.fails({"crates/sciart/src/cli.rs": 'about = "Generative art",\n'})

    def test_web_r9_the_guides_opening_lines_carry_it(self) -> None:
        self.fails({"docs/ARCHITECTURE.md": "# Architecture\n\n## Overview\n\nsciart draws art.\n"})
        self.fails({"docs/guide/index.md": "# Introduction\n\nsciart draws art.\n\n" + D + "\n"})

    def test_web_r9_the_homebrew_desc_is_the_short_form(self) -> None:
        self.fails({"packaging/homebrew/sciart.rb.in": f'  desc "{D}"\n'})
        self.fails({"packaging/homebrew/sciart.rb.in": '  desc "Terminal art"\n'})

    def test_web_r9_the_site_lede_and_every_meta_description_carry_it(self) -> None:
        self.fails({"site/dist/index.html": f'<meta name="description" content="{D}.">\n<p class="lede">Art.</p>\n'})
        self.fails({"site/dist/guide/index.html": '<meta name="description" content="Art.">\n'})
        self.fails({"site/dist/guide/index.html": "<p>No meta.</p>\n"})

    def test_web_r9_astronomy_first_wording_fails_wherever_users_read_it(self) -> None:
        for rel, text in (
            ("docs/GALLERY.md", "# Gallery\n\nScience & astronomy art.\n"),
            ("crates/sciart/src/main.rs", "//! real-time generative science and astronomy art\n"),
            ("crates/sciart/src/lib.rs", f"//! {D}.\n//! Was: science  and\tastronomy.\n"),
            ("README.md", f"# sciart\n\n{D}.\n\nScience &\nastronomy.\n"),
            ("docs/guide/keys.md", "# Keys\n\nsciart is science &amp; astronomy art.\n"),
            ("site/dist/gallery/index.html", f'<meta name="description" content="{D}.">\n<p>Science and Astronomy</p>\n'),
        ):
            with self.subTest(rel=rel):
                self.fails({rel: text})

    def test_web_r9_the_repository_meets_the_rule_outside_the_built_site(self) -> None:
        # The built site is checked by `pixi run site-check`; here, the sources it comes from.
        problems = site_check.description_problems(site_check.ROOT, Path(self.tmp.name) / "none")
        self.assertEqual(
            [p for p in problems if "site/dist" not in p and "index.html" not in p], []
        )


if __name__ == "__main__":
    unittest.main()
