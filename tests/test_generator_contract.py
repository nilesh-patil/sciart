"""R1: every generator must survive any call sequence with width/height >= 1.

See docs/issue-01-simulator-stability.md.
"""

import importlib
import pkgutil
import random
import re
from pathlib import Path

import pytest

import art_generators


def _discover():
    names = []
    for _, name, _ in pkgutil.iter_modules(art_generators.__path__):
        module = importlib.import_module(f"art_generators.{name}")
        if hasattr(module, "generate"):
            names.append(name)
    return sorted(names)


GENERATOR_NAMES = _discover()


def _fresh(name):
    """Return the generator with its cached animation state cleared."""
    gen = importlib.import_module(f"art_generators.{name}").generate
    for attr in list(vars(gen)):
        delattr(gen, attr)
    random.seed(99)
    return gen


def _check_frame(out, width, height):
    assert isinstance(out, str), f"expected str, got {type(out)}"
    lines = out.split("\n")
    assert len(lines) == height, f"expected {height} lines, got {len(lines)}"
    too_wide = [(i, len(line)) for i, line in enumerate(lines) if len(line) > width]
    assert not too_wide, f"lines wider than {width}: {too_wide[:3]}"


@pytest.mark.parametrize("name", GENERATOR_NAMES)
def test_steady_long_run(name):
    gen = _fresh(name)
    for _ in range(200):
        _check_frame(gen(200, 50), 200, 50)


@pytest.mark.parametrize("name", GENERATOR_NAMES)
def test_resize_sequence(name):
    gen = _fresh(name)
    for w, h in [(200, 50), (80, 24), (20, 6), (3, 2), (1, 1), (120, 40), (200, 50)]:
        for _ in range(3):
            _check_frame(gen(w, h), w, h)


@pytest.mark.parametrize("name", GENERATOR_NAMES)
def test_random_resize_fuzz(name):
    gen = _fresh(name)
    rng = random.Random(1234)
    for _ in range(150):
        w, h = rng.randint(1, 260), rng.randint(1, 70)
        _check_frame(gen(w, h), w, h)


@pytest.mark.parametrize("name", GENERATOR_NAMES)
def test_minimal_dims(name):
    gen = _fresh(name)
    for w, h in [(1, 1), (1, 8), (8, 1), (2, 2)]:
        _check_frame(gen(w, h), w, h)


def test_all_advertised_generators_exist():
    """R3: every run_<name> pixi task resolves to a real generator module."""
    toml = (Path(__file__).resolve().parent.parent / "pixi.toml").read_text()
    advertised = re.findall(r"^run_(\w+)\s*=", toml, flags=re.M)
    missing = sorted(set(advertised) - set(GENERATOR_NAMES))
    assert not missing, f"pixi tasks reference missing generators: {missing}"
