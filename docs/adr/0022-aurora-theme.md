# ADR-0022: Aurora colour theme

- **Status**: Accepted
- **Date**: 2026-10-13

## Context
The renderer has three themes (RND-R2): `astro` (the default, which every look rule and art
metric is measured in), `rainbow` (per-cell hashed hues) and `mono`. The user asked for a new
theme, *Aurora*. They chose all three options from previews: a fourth theme rather than a new
default, the oxygen green + red palette, and the "curtain" field ramps.

## Decision
1. `Theme::Aurora` is added after `mono` in `Theme::ALL`, so `t` cycles
   astro → rainbow → mono → aurora. `--theme aurora` and `theme = "aurora"` select it.
2. Its palette follows auroral emission (RND-R16): the O 557.7 nm green is `Cold`, the
   O 630 nm red is `Hot` (still a complementary pair, as M11's look needs), and N₂⁺ 427.8 nm violet
   is `Accent`. Other roles stay cool (teal, mint, ice blue, frost) on night blue, and `Sun`
   stays a pale gold so stars are not all green.
3. `Seq` is the curtain ramp night → violet → teal → green → mint. `Div` is violet ← dark → green.
   Both are monotone in luminance (RND-R6).
4. The roles that scenes may not use (`Phase`, `Blackbody`, `Ramp`, GEN-R23) reuse `astro`'s
   tables, so no new field tables are needed.
5. Object roles use `astro`'s level scaling (`0.3 + 0.7·level`) and a 256-entry table (RND-R15),
   so painting costs the same as `astro`.

## Consequences
- Scenes, snapshots and art metrics are unchanged: they are measured in `astro`.
- `preview --swatches` and `--theme aurora` show the theme without new tooling.

## Alternatives considered
- Making aurora the default: rejected by the user. Every art metric band was tuned in `astro`.
- Replacing `rainbow`: rejected. Existing configs naming `rainbow` would break (CFG-R3).
- A cool-only violet/green palette or a pastel one: both were offered, and the user chose the
  saturated red/green pair.
