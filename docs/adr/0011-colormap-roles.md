# ADR-0011: Sequential and diverging colormap roles (amends ADR-0004)

- **Status**: Accepted
- **Date**: 2026-09-27

## Context
ADR-0004 maps a role to one hue scaled by level. That is right for objects (a star, a
planet) but wrong for scalar fields: vorticity, spin, concentration and temperature need
perceptual multi-hue ramps, and signed quantities need a neutral zero. Composing
`Hot`/`Cold` by threshold bands and loses the sign's magnitude.

## Decision
Add two roles to `sciart_core::Role` (CORE-R46), still semantic and never RGB:

- **`Role::Seq`** — sequential field; level 0 = low, 255 = high.
- **`Role::Div`** — signed field; level 128 = zero, 0 = most negative, 255 = most positive.

Themes map them to multi-stop gradients (RND-R6): `astro` uses an inferno-like `Seq` and a
dark-centred cyan↔orange `Div` (the centre sits near the terminal background, so zero reads as
"nothing"); `rainbow` uses a turbo-like `Seq` and ignores its per-cell hash for these roles;
`mono` uses grey ramps linear in CIE L*: `Seq` from near-black to white, and `Div` from a
near-black zero up to white for positive values and only to a dim grey for negative ones.

## Consequences
+ Field scenes (`chemical_reaction`, `vortex_street`, `ising_model`, `solar_surface`) share
  one colour vocabulary and switch themes correctly.
− The RND-R1 luminance-monotonicity property changes for `Div`: it is monotone in
  `|level − 128|`, not in `level`.
− `mono` has one channel, so its `Div` cannot be both symmetric in `|v|` and signed. It
  keeps zero dark and luminance monotone in `|v|` and gives the sign a different slope
  (negative tops out at L* 45, positive at L* 100). Opposite lobes read as dim and bright,
  but a strong negative matches a weak positive (|v| ≈ 0.43). Scenes that need the exact
  sign should also pick glyphs by sign.

## Alternatives considered
A single `Field` role (no signed fields); scenes composing existing roles (banding, weaker
semantics).
