# ADR-0020: Orbital-thread style (hydrogen clouds × strange-attractor threads; amends ADR-0019)

- **Status**: Accepted
- **Date**: 2026-10-02

## Context
After M16 the user asked for ten scenes to be redesigned "in the same theme as a mix of hydrogen
atoms and strange attractors": `gray_scott`, `molecular_motors`, `gravitational_lensing`,
`io_volcanoes`, `spacetime_grid` (central objects only), `solar_eruption`, `three_body`,
`spiral_galaxy`, `supernova_remnant` and `ising_model`. Three of them still drew flat half-block
fills (`gray_scott`, `molecular_motors`, `io_volcanoes`); six were M16 thread scenes the user wanted
improved. The user chose one design per scene from previews (all recommended options, recorded in
`docs/design/M17-orbital-thread.md`) and asked for the lowest CPU and memory cost.

`electron_orbitals` draws each orbital as a Monte-Carlo cloud whose points carry fixed uniforms,
so the cloud holds still and only changes where the density crosses a threshold. That is what
makes it calm, and cheap on the wire: 3 % of cells change per frame, against 12.8 % for
`strange_attractors`.

## Decision
1. **GEN-R26 orbital-thread style** for the ten scenes: GEN-R25's rules (braille first, ≤ 3 %
   half-block cells, M11 roles, black ground) plus *clouds* (bodies and media as probability stipple
   with fixed per-dot thresholds, coloured `Hot`/`Cold` by the sign or phase of a physical variable)
   and *threads* (structure and motion as 1-px threads coloured along their length, afterglow only for
   moving things).
2. The six M16 scenes move from GEN-R25 to GEN-R26; a scene is listed by one of the two. GEN-R22
   no longer pins the five M11 scenes involved (`gray_scott`, `three_body`, `gravitational_lensing`,
   `ising_model`, `spacetime_grid`).
3. **CORE-R107** `raster::stipple_keep` / `Dots::stipple` give every scene the same fixed-threshold
   stipple (a hash of the dot and a per-layer salt), so no scene grows its own copy and no cloud
   shimmers.
4. **Stillness is measured**: at 200×50, on average at most 10 % of cells may change per drawn
   frame over t = 10–12 s (`tests/orbital_style.rs`). The figure is under `strange_attractors` and
   near RT-R16's gallery budget.

## Consequences
- The gallery has three idioms on one palette: M11 (GEN-R22/R24), thread (GEN-R25) and
  orbital-thread (GEN-R26).
- Stipple costs one hash per candidate dot; scenes cull candidates to the cloud's bounding region
  and rasterise in `prepare` behind a dirty flag (GEN-R20), so the worst-frame budget
  (< 2 ms at 200×50) holds.
- `art_metrics` bands of the ten scenes are re-seeded after review.
