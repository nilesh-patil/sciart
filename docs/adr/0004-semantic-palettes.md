# ADR-0004: Semantic inks + themes instead of per-character random color

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
Python assigns a random hue to every character on every frame. At 30 FPS that strobes.

## Decision
Generators emit `Ink { role, level }` (e.g. `Role::Star`, level 0–255). A `Theme` is a pure function `(ink, x, y) → Rgb`. Themes: `astro` (default), `rainbow` (stable per-cell hue — the old look without flicker), `mono`.

## Consequences
+ No flicker; color carries meaning; themes switch at runtime.
− Generators must choose roles and levels deliberately.

## Alternatives considered
Stable-hash rainbow only (loses semantics); generators pick RGB (no theming).
