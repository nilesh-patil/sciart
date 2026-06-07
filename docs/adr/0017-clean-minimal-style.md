# ADR-0017: Clean minimal style (amends ADR-0010, ADR-0011, ADR-0015)

- **Status**: Accepted
- **Date**: 2026-09-29

## Context
After M12 and M13 the gallery reads worse, even though colourfulness and contrast went up. The
user named the causes: **over-design, loud colours, large blocks, thick edges**. Scenes grew extra
elements, saturated full-frame grounds, cell-sized stamps and multi-cell outlines. The user's
references for the wanted look are `double_pendulum` and the **M11** versions of
`galaxy_collision` and `rocket_launch`: sparse, fine, calm scenes on black. Colourfulness is not
a measure of quality here, and chasing it caused part of the regression.

## Decision
Every scene follows these rules. SPEC-002 GEN-R17, GEN-R18 and GEN-R21 test the measurable parts,
and the art review judges the rest.

1. **Black ground.** Empty space is `Role::Background`. Fields map their low end to true black
   (max channel ≤ 28), and skies are black or near-black. A lit backdrop needs a reason in the
   scene's SPEC-002 section and stays dim.
2. **Physics-true colour, muted.** Colours keep their physical meaning: spectra, blackbody,
   emission lines, chemistry. They are desaturated and dimmed so they sit quietly on black. A
   saturated colour is kept for the one thing the eye should find, and bright means white-hot, not
   neon. `rainbow`-theme hues are unchanged; this applies to `astro`, the default.
3. **Small pixels, thin lines.** Structure is drawn at sub-cell resolution: braille dots,
   half-block pixels, or 1-pixel `Glow::stroke` lines anti-aliased by intensity, not by thickness.
   There are no cell-sized stamps for small things, no multi-cell outlines or borders, and no flat
   blocks where a field varies (GEN-R17). Glows have tight falloff, so a point stays a point. Values fade smoothly to black (smoothstep); no ordered, checker or stipple dither, which reads as a mechanical screen pattern and frays edges (added after M14 review of cosmic_microwave_background).
4. **Less.** One subject per scene. Remove decorative frames, boxes, legends, redundant particles,
   extra captions and ornamental layers that don't explain the science. The bottom-left science
   title stays (ADR-0014). A label or readout stays only where it explains the science (a planet's
   name, a measured quantity), in dim grey (`Role::Text`, low level).
5. **Calm motion.** Motion shows the physics. No flashing, strobing or busy background animation.
6. **Legible subject: clean is not faint.** The scene's one subject reads at a glance at 80×24 and 200×50.
  Its brightest pixels reach near-white or the accent (L* ≥ 70), a hero object is solid (filled, or
  strokes of 2 pixels or more), and every act shows it. Cut the other elements so the subject stands out;
  never dim or thin the subject itself. (Added after M14 review round 1, where a rocket drawn as a faint
  dotted line and clouds that peaked at L* 30 were clean but lifeless.)
7. **Minimal is also cheap.** Black cells cost nothing to send, fewer layers cost less CPU, and a
   quiet frame changes fewer cells per frame. Style and the M14 resource goals point the same way.

For the 41 scenes that existed at M11, the art review compares the M11 look (`627cde6`) with
today's. Where M11 is cleaner, the scene returns to that look and keeps everything else later
milestones added: the M11 titles, GEN-R6 trig, allocation-free rendering, `prepare`, and the model
fixes that aren't visual.

## Consequences
- M12's single-hue ramps stay available, but a scene uses one only where it fits the physics, and
  its floor must be black.
- GEN-R21 no longer sets colourfulness or contrast floors. It sets ceilings on the chroma of lit
  pixels and on lit coverage, so a scene can't slide back to loud or crowded.
- Many snapshots change. Each is reviewed against this ADR, with a before/after sheet.

## Alternatives considered
- **Keep the M12/M13 look and only fix blocks and grounds.** Rejected: the user named over-design
  and loud colour as primary.
- **Monochrome per scene.** Rejected by the user in favour of physics-true colour, muted.
- **Revert all scenes to M11 wholesale.** Rejected: M13 has no M11 version, and several M12 changes
  are real improvements (e.g. `wave_interference`, `double_pendulum`). The choice is per scene.
