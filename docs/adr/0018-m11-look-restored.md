# ADR-0018: M11 look restored (supersedes ADR-0017; amends ADR-0011, ADR-0015)

- **Status**: Accepted
- **Date**: 2026-09-30

## Context
Every scene was rendered at each milestone (`samples/<scene>-<version>.png`, side by side in
`samples/compare/`). The user judged the **M11** look better in most cases than M12's rich, filled
and haloed upgrades, and better than M14's clean but greyed redesign (ADR-0017). Measured over the
41 scenes that existed at M11 (160×48, t = 20 s): M11 has the highest lit chroma (0.35, against
0.31 at M12 and 0.25 at M14) and far less blob fill than M12 (0.24 against 0.67), on a black ground.

Asked what "M11 style" should mean (2026-09-30), the user chose:

- a **literal revert** of every scene that existed at M11, not an M11-inspired palette on the
  M14 geometry;
- M11's flat, stepped fills **exactly as they were** (gray_scott and belousov_zhabotinsky fills,
  ising_model's domains, mars_rover's sky, the stepped exoplanet star);
- M11's **sparseness** for scenes that were small or faint then (black_hole, galaxy_collision,
  gravitational_lensing, pulsar, fractal_growth, solar_system), not the later richer subjects;
- the **35 scenes added at M13** restyled in M11's idiom, keeping their models and compositions;
- M14's **performance gains kept**.

The M11 scene files compile against today's core unchanged; the only port they need is the
deterministic trigonometry (GEN-R6, ADR-0016), plus a lattice cap where M11's memory exceeds the
TEST-R7 ceiling (fractal_growth).

## Decision
1. **The 41 M11 scenes render as they did at M11** (GEN-R22). Each is M11's scene module
   (`627cde6`), ported only as far as today's rules require: `Trig` instead of std `sin`/`cos`,
   today's `Meta.category` (ADR-0013), the allocation and live-memory rules (GEN-R11, TEST-R6/R7),
   the `prepare` hook where it saves work (GEN-R20), and the resource budgets (GEN-R8, RT-R16).
   Their SPEC-002 sections return to their M11 text. What M12–M14 added to these scenes (models,
   acts, readouts, palettes) is dropped with their looks.
2. **M11's palette only** (GEN-R23). Scenes ink with the semantic roles and the two field roles
   that existed at M11: `Background`, `Dim`, `Star`, `Sun`, `Planet`, `Particle`, `Wave`, `Hot`,
   `Cold`, `Structure`, `Accent`, `Text`, `Seq`, `Div`. `Role::Ramp(_)`, `Role::Blackbody` and
   `Role::Phase` stay in the core and the themes (ADR-0015) but no scene uses them.
3. **M11's idiom for the scenes added later** (GEN-R24). Black ground; braille dots for lines,
   orbits and particles; half-block pixels for bodies and fields, with flat, stepped fills allowed;
   saturated semantic colours in complementary pairs (Hot/Cold, Accent); a small subject with
   much black around it; no tone-mapped glow washes, halos or gradient backdrops; labels and
   readouts as at M11. Each keeps its M14 model, motion and title.
4. **M14's pipeline and budgets stay.** The `prepare` hook, the one-write frame backend, the LUT
   paint, colour hysteresis and the allocation-free loop are look-independent and unchanged. The
   budgets are the M14 measurements plus headroom (RT-R16, GEN-R8).

ADR-0017 is superseded: its rules 1 (black ground) and 7 (minimal is cheap) carry over in spirit;
rule 2 (muted colour), rule 3 (no flat blocks, Catmull-Rom magnification in scenes) and rule 6
(legible-subject floor) do not.

## Consequences
- GEN-R17, GEN-R18 and GEN-R21 (M14's look rules) are superseded by GEN-R22–R24; `art_metrics`
  becomes a per-scene regression band around the reviewed M15 looks, without the global neon cap
  or the legible-subject floor.
- The M12–M14 per-scene improvements to the 41 scenes (science readouts, acts, colour work) are
  lost; that is the user's choice, recorded here. Science stays honest: a restored M11 title
  or caption that overclaims is corrected (GEN-R12).
- Core helpers lifted at M12–M14 (ramps, blackbody, `Glow`/`Tone`, `SampleMap` Catmull-Rom)
  remain in the core for the scenes that still use them within GEN-R23 (e.g. `Glow` toned into
  `Seq`) and for future work.
- The fidelity fixture (M11 frames, glyph + ink) pins the 41 scenes; changing one later is a
  spec change, as with the snapshots.

## Alternatives considered
- **M11 palette on M14 geometry** (thin colourful lines, the `M15-thin-colour-style` proposal):
  not chosen by the user.
- **Per-scene best of M11 and M14**: not chosen.
- **Rebuilding the M11 looks inside the M14 scene code**: far more work and risk than reverting
  the files, which compile and pass the contract and allocation suites as they are.
