#!/usr/bin/env python3
"""Run the compiled player module outside a browser (WEB-R3).

    python3 scripts/web_smoke.py [MODULE]     # pixi run web-smoke (after pixi run web-build)

The native tests (web/tests/player.rs) prove the player's logic on the host; this runs the
`wasm32-unknown-unknown` build itself, through wasmtime (the `wasmtime-py` package in the
pixi environment), so a fault that only the compiled module has cannot reach the site:

- the module imports nothing, so it instantiates with no host functions at all;
- `scenes()` returns exactly `sciart list --json` (CLI-R16), so it lists exactly the
  registered scenes;
- every scene is created, advanced one second and returns a frame of
  `columns·8 × rows·16 × 4` bytes that is not all one colour.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import build_site

ROOT = Path(__file__).resolve().parent.parent
MODULE = ROOT / "web" / "target" / "wasm32-unknown-unknown" / "release" / "sciart_web.wasm"
COLUMNS, ROWS, SEED = 40, 12, 42


def fail(message: str) -> None:
    sys.exit(f"web_smoke: {message}")


def main(argv: list[str]) -> int:
    try:
        from wasmtime import Engine, Instance, Module, Store
    except ImportError:
        fail("the wasmtime package is missing; run through pixi (`pixi run web-smoke`)")
    path = Path(argv[0]) if argv else MODULE
    if not path.is_file():
        fail(f"{path} is missing; run `pixi run web-build`")

    engine = Engine()
    store = Store(engine)
    module = Module.from_file(engine, str(path))
    if module.imports:
        names = ", ".join(f"{i.module}.{i.name}" for i in module.imports)
        fail(f"WEB-R3: the module imports {names}; it must need nothing from its host")
    exports = Instance(store, module, []).exports(store)
    memory = exports["memory"]

    def call(name: str, *args: int | float) -> int:
        return exports[name](store, *args)

    def output() -> str:
        start = call("output_ptr")
        return bytes(memory.read(store, start, start + call("output_len"))).decode("utf-8")

    def write_input(text: str) -> None:
        data = text.encode("utf-8")
        memory.write(store, data, call("input", len(data)))

    problems: list[str] = []

    # scenes() is `sciart list --json`, so it lists exactly the registered scenes.
    call("scenes_json")
    listed = output()
    expected = build_site.sciart("list", "--json")
    if listed != expected:
        problems.append("WEB-R3: the module's scenes() differs from `sciart list --json`")
    names = [scene["name"] for scene in json.loads(listed)["scenes"]]
    registered = [scene["name"] for scene in json.loads(expected)["scenes"]]
    if sorted(names) != sorted(registered):
        problems.append(
            f"WEB-R3: scenes() lists {sorted(set(names) ^ set(registered))} differently "
            "from the registry"
        )

    call("theme_names")
    themes = output().split("\n")
    if themes[:1] != ["astro"] or len(themes) != len(set(themes)):
        problems.append(f"WEB-R3: unexpected theme names {themes}")

    # Every scene: created, advanced one second, a full frame that is not blank.
    width, height = COLUMNS * 8, ROWS * 16
    for name in registered:
        write_input(name)
        handle = call("player_new", COLUMNS, ROWS, 0, SEED)
        if handle == 0:
            problems.append(f"WEB-R3: {name}: the module has no such scene")
            continue
        try:
            if call("player_advance", handle, 1.0) != 1:
                problems.append(f"WEB-R3: {name}: advance(1.0) was refused")
            for index, theme in enumerate(themes):
                ptr = call("player_frame", handle, index)
                size = (call("player_width", handle), call("player_height", handle))
                if size != (width, height):
                    problems.append(f"WEB-R3: {name}: a {size[0]}x{size[1]} frame, not {width}x{height}")
                    break
                pixels = bytes(memory.read(store, ptr, ptr + width * height * 4))
                if len(pixels) != width * height * 4:
                    problems.append(f"WEB-R3: {name}: {len(pixels)} frame bytes, not {width * height * 4}")
                    break
                if len({pixels[i : i + 4] for i in range(0, len(pixels), 4)}) < 2:
                    problems.append(f"WEB-R3: {name}: the {theme} frame after one second is blank")
        except Exception as err:  # a trap in the module
            problems.append(f"WEB-R3: {name}: the module failed ({err})")
        finally:
            call("player_free", handle)

    if problems:
        print(f"web_smoke: {len(problems)} problem(s):", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    print(
        f"web_smoke: ok, {path.stat().st_size / 2**20:.2f} MiB module with no imports; "
        f"{len(registered)} scenes listed, created, advanced 1 s and drawn non-blank at "
        f"{COLUMNS}x{ROWS} in {len(themes)} themes"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
