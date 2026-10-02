# ADR-0021: Molecules in the orbital-thread style (amends ADR-0020)

- **Status**: Accepted
- **Date**: 2026-10-02

## Context
After M17 the user asked for six more scenes to be redesigned "into the same theme as a mix of
hydrogen atoms, strange attractors and molecules": `wave_interference`, `chemical_garden`,
`physarum`, `firefly_sync`, `heart_fibrillation`, and `spiral_galaxy` again, now with six arms.
ADR-0020 gives clouds and threads. The new ingredient is the molecule, as `molecule_viewer` draws
it: small round atoms joined by thin bonds. Four of the six scenes have discrete bodies (wave
sources, precipitate beads, oat flakes, fireflies, spiral tips) that had been drawn as single
pixels, glyphs or flat blocks. The user chose one design per scene from previews; see
`docs/design/M18-orbital-molecules.md`.

## Decision
1. GEN-R26 gains a third rule, *molecules*: discrete bodies are small Gaussian stipple atoms,
   joined by 1-px bond threads only where a physical link exists (a tube's growth, a coupling
   pulse).
2. **CORE-R108** `Dots::stipple_puff` is the one atom primitive. It is a CORE-R107 stipple of a
   Gaussian, with the centre dot always lit. A static atom holds still, and a swelling one only
   gains dots.
3. The five new scenes join GEN-R26's list. Its half-block and stillness limits apply unchanged,
   and GEN-R22 no longer pins `wave_interference` or `physarum`.

## Consequences
- Scenes stop writing their own Gaussian puff loops. A puff costs one hash per dot within 3σ,
  which is a few dozen dots per atom.
- An atom's thresholds are keyed by absolute dot position. A moving atom therefore re-samples as
  it moves, which suits flashes, beads and tips. A large body that moves smoothly should key its
  thresholds to its own frame instead, as `three_body`'s ghosts do.

## Alternatives considered
- Half-block discs for atoms: rejected by GEN-R26's ≤ 3 % half-block rule and the M17 look.
- Per-scene puff code: the copies that M12 lifted out (CORE-R84..R96) show where that leads.
