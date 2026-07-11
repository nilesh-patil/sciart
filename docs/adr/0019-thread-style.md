# ADR-0019: Thread style for selected scenes (amends ADR-0018)

- **Status**: Accepted
- **Date**: 2026-10-01

## Context
After M15 the user picked `strange_attractors` as the look to follow: fine braille threads on
black, colour along the thread, density where threads cross, no fills. `light_echo` was
restyled that way first and accepted. The user then listed 25 more scenes to restyle (several
redesigned, some heavily) and asked for `seashell_pigmentation` to be removed. The choices,
made scene by scene, are recorded in `docs/design/M16-thread-style.md`.

Six of the scenes are M11 scenes that GEN-R22 pins to their M11 ink frames
(`belousov_zhabotinsky`, `ising_model`, `globular_cluster`, `vortex_street`,
`gravitational_lensing`, `spacetime_grid`).

## Decision
1. **Thread style (GEN-R25)** for the scenes listed in GEN-R25: braille-first (lines, waves,
   fields as contours or crests, particles, trails as `Dots`); half-block pixels only for a
   small solid body in flat steps; M11 roles only (GEN-R23) with colour by a physical variable;
   decaying trails only for moving things; black between threads.
2. **GEN-R22 no longer pins those M11 scenes**: `m11_fidelity` skips the scenes listed in
   GEN-R25. The other M11 scenes stay pinned.
3. **`seashell_pigmentation` is removed** from the gallery.
4. Iso-contour drawing (marching squares into `Dots`), first written in `light_echo`, is lifted
   into the core, so the contour scenes share one allocation-free implementation.

## Consequences
- The gallery has two idioms: M11 (GEN-R22/R24) and thread (GEN-R25). Both use M11's palette
  and a black ground.
- Thread scenes usually change more cells per frame than flat fills, so RT-R16's gallery
  budget must be re-checked; per-scene `art_metrics` bands are re-seeded after review.
