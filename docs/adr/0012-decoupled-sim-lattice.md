# ADR-0012: Capped simulation lattices, decoupled from the display, resampled on resize

- **Status**: Accepted
- **Date**: 2026-09-27

## Context
Grid and field scenes (LBM, Gray–Scott, Ising) and N-body cost O(cells) or O(N²). Simulating
at display resolution ties cost to terminal size: a 400×100 terminal would break GEN-R8.
Rebuilding state on resize (the M4–M7 default) throws away minutes of evolved pattern.

## Decision
1. **Capped lattice.** A stateful field scene simulates on its own lattice whose node count
   is capped by a per-scene constant, independent of the terminal. The display samples it
   (bilinear or nearest). N-body caps `N`.
2. **Aspect buckets.** The lattice shape is chosen from the display aspect. Within the same
   bucket and cap, a resize only rescales the view and leaves the simulation untouched.
3. **Resample on bucket change.** When the lattice must change, state is resampled with
   `grid::resample_bilinear` (continuous fields) or `grid::resample_nearest` (discrete, such as
   Ising spins) (CORE-R43). LBM resamples only macroscopic `ρ, u` and rebuilds
   `f = f_eq(ρ, u)`; resampling non-equilibrium populations is unstable.
4. Single-threaded and deterministic. No rayon (it would need float-reduction ordering care
   and a new dependency).

## Consequences
+ Bounded cost for every scene at any terminal size; patterns survive resizes.
− Very large terminals show upsampled (softer) fields rather than more detail.
− Scenes carry a small bucket/resample state machine; the helpers keep it short.

## Alternatives considered
Native resolution (unbounded cost); reset on resize (loses state); rayon (non-determinism
risk, a dependency).
