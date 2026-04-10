"""R2: the orchestrator must isolate per-frame faults.

See docs/issue-01-simulator-stability.md. pyautogui is stubbed so tests never
touch the real mouse and can simulate its failure modes.
"""

import sys
import types

import pytest


@pytest.fixture
def activate(monkeypatch):
    fake = types.ModuleType("pyautogui")

    class FailSafeException(Exception):
        pass

    fake.FailSafeException = FailSafeException
    fake.size = lambda: (1440, 900)
    fake.position = lambda: (100, 100)
    fake.moves = []
    fake.move_error = None

    def move_to(x, y, duration=0.0):
        if fake.move_error is not None:
            raise fake.move_error
        fake.moves.append((x, y))

    fake.moveTo = move_to

    monkeypatch.setitem(sys.modules, "pyautogui", fake)
    sys.modules.pop("activate", None)
    module = __import__("activate")
    module._fake_pyautogui = fake
    yield module
    sys.modules.pop("activate", None)


def test_move_mouse_moves_within_margin(activate):
    fake = activate._fake_pyautogui
    activate.move_mouse()
    assert len(fake.moves) == 1
    x, y = fake.moves[0]
    assert 50 <= x <= 1440 - 50
    assert 50 <= y <= 900 - 50


def test_move_mouse_swallows_failsafe(activate):
    """R2.1: cursor parked in a screen corner must not kill the simulation."""
    fake = activate._fake_pyautogui
    fake.move_error = fake.FailSafeException("fail-safe triggered")
    activate.move_mouse()  # must not raise


def test_move_mouse_swallows_os_errors(activate):
    """R2.1: e.g. macOS accessibility permission revoked mid-run."""
    fake = activate._fake_pyautogui
    fake.move_error = OSError("accessibility permission denied")
    activate.move_mouse()  # must not raise


def test_safe_generate_passes_through_art(activate):
    assert activate.safe_generate(lambda w, h: "ok", 80, 24) == "ok"


def test_safe_generate_contains_generator_crash(activate):
    """R2.2: a crashing generator yields None instead of an exception."""

    def boom(w, h):
        raise ZeroDivisionError("division by zero")

    assert activate.safe_generate(boom, 80, 24) is None


def test_safe_generate_clamps_degenerate_dimensions(activate):
    """R2.2: dimensions are clamped to >= 1 before reaching the generator."""
    seen = {}

    def recorder(w, h):
        seen["dims"] = (w, h)
        return ""

    activate.safe_generate(recorder, 0, -7)
    assert seen["dims"] == (1, 1)


def test_color_text_preserves_spaces_and_content(activate):
    out = activate.generate_random_color_text("a b")
    assert " " in out
    assert "a" in out and "b" in out
    assert "[rgb(" in out
