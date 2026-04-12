# ADR-0005: Selectable keep-awake backends, caffeinate by default

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
The original purpose is to stop the screen sleeping. Mouse movement needs macOS accessibility permission and trips pyautogui's fail-safe; `caffeinate` needs neither.

## Decision
`--keep-awake caffeinate|mouse|off`. Default `caffeinate` on macOS (child process `caffeinate -dimsu -w <pid>`), `off` elsewhere. `mouse` (enigo) sits behind the opt-in cargo feature `mouse`. Failures are never fatal.

## Consequences
+ Works out of the box without permissions.
− `caffeinate` doesn't reset "idle/away" status in chat apps; users who need that opt into `mouse`.

## Alternatives considered
Mouse only (the Python behavior); dropping the feature.
