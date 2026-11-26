# ADR-0023: Lean build and runtime

- **Status**: Accepted
- **Date**: 2026-10-15

## Context
The Rust build had become slow, and the goal was to cut build time and the CPU and memory used
while the art runs, without losing visual quality. Measured before M22
(M21 tree, 14 cores, 85 scenes, `sciart-generators` ≈ 96 k lines in one crate):

| Step | Time |
|---|---|
| clean release build (`-p sciart`) | 37 s (`sciart-generators` 16 s on one codegen unit, binary + thin LTO 15 s) |
| release rebuild after editing one scene | 31 s |
| debug test build, then after editing one scene | 28 s, 3 s |
| `rs-lint` / `rs-test` / `rs-snap-release` after editing one scene | 16 s / **173 s** / 35 s |

`rs-test` is dominated by running the test binaries one after another (generators unit tests
69 s at opt-level 1, `contract` 18 s, `resource_budget` 14 s, …). A fresh checkout starts from
an empty `target/` and reinstalls the pixi environment.

At runtime (200×50, 30 FPS, `sciart sim`) the process uses ≈ 2.4 % of a core and 6–9 MB RSS, but
writes ≈ 238 KB/s to the terminal, and iTerm2 spent ≈ 11 % CPU drawing it: the terminal is the
main cost.

Every option considered was adopted: faster test runs, an optimised CI profile,
a shared build cache, splitting the generators crate by category; an idle-aware frame loop, fewer
bytes per frame, earlier low-power mode (2 min idle, 30 FPS kept) and lower memory ceilings
(measured + margin).

## Decision
1. **Tests** run under `cargo nextest` (test binaries in parallel) with an optimised test build
   (opt-level ≥ 2, debug assertions and overflow checks kept) (TEST-R10).
2. **A `ci` profile** (opt-level 3, many codegen units, no LTO, `panic = "unwind"`) builds the
   release snapshot run and the `rs-cost`/`rs-worst` probes; `sciart` installs keep the `release`
   profile (thin LTO, one codegen unit) (TEST-R9 amended). Frames do not depend on codegen
   partitioning because trigonometry goes through `Trig` (GEN-R6, ADR-0016), so the snapshot run
   still guards optimiser-dependent frames.
3. **A shared compiler cache** (sccache from the pixi environment) is used by every task, so a
   second checkout of the repository reuses compiled crates (TEST-R11).
4. **Split `sciart-generators`** into a shared-models crate and five scene crates by category
   (cosmos, earth & voyages, physics, matter, life); `sciart-generators` stays the registry facade,
   so the binary, the test suites and `find`/`registry` are unchanged (GEN-R3, GEN-R16 amended).
5. **Idle-aware frame loop**: a frame whose diff is empty writes nothing, and a run of empty
   frames lowers the draw rate until something changes (RT-R17).
6. **Fewer bytes**: tighter terminal encoding for the same painted frame, and reviewed churn cuts
   in the busiest scenes (RND-R17, RT-R16 tightened).
7. **Low power** after 2 min without a key press (CFG-R8).
8. **Memory**: per-scene live-byte ceilings lowered to measured + margin (TEST-R7 amended).

## Consequences
- Visuals unchanged: snapshots and art-metric bands guard every frame; churn cuts are reviewed.
- More crates in the workspace; scene authors put a new scene in its category's crate.
- sccache is a new dev tool in the pixi environment (not a crate dependency).

## Alternatives considered
- Lowering `release` to many codegen units / no LTO: rejected for installs (runtime speed); used
  only for CI-style builds.
- 24 FPS by default: considered; 30 FPS was kept.
- One crate per scene: considered; rejected (workspace overhead).
- 256-colour output: not offered as a default; it changes colours.
