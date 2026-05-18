# ADR-0016: Profile-independent trigonometry

- **Status**: Accepted
- **Date**: 2026-09-28

## Context
GEN-R6 promises byte-identical frames for the same seed and call sequence. M12 found
`molecular_dynamics` rendering a different frame at t = 5 s (seed 42, 80×24) in a release
build than in the dev build its snapshot was recorded with.

Bisecting the build settings showed the result tracked codegen partitioning (incremental,
codegen units, LTO), not optimisation level, debug assertions or overflow checks. A per-step
state hash of the scene diverged already after `resize`, in the start velocities. Those come
from `particles::gaussian2` → Box–Muller → `(TAU * u2).sin_cos()`. `f64::sin_cos` is
`(self.sin(), self.cos())`; when both calls land in one function, LLVM's DAG combiner replaces
them with one `sincos` libcall, which on Apple platforms is `__sincos_stret`. Its sine differs
from libm's `sin` in the last bit for about 0.2 % of arguments (22 379 of 10⁷ uniform phases;
the cosine agreed). Whether the pair shares a function after inlining depends on the build, so
the same source gave two answers, and the Lennard-Jones gas amplified one ulp into a different
frame within seconds. The ~370 other trig calls in the workspace carry the same latent hazard.

Rust itself never contracts float expressions to FMA or reassociates them, and an all-scene
frame-hash sweep (41 scenes × 80×24 and 200×50 × t = 1, 5, 20, 40 s) found no other
profile-dependent result.

## Decision
- `sciart_core::trig` holds `sin`, `cos` (and `f32` `sinf`, `cosf`) as `#[inline(never)]`
  wrappers around exactly one libm call, plus `sin_cos(x) = (sin(x), cos(x))`. No function
  ever holds both halves of a pair, so the combine cannot fire. The prelude trait `Trig` gives
  method syntax: `x.sin_det()`, `x.cos_det()`, `x.sin_cos_det()` (CORE-R90).
- `clippy.toml` bans `f64::{sin, cos, sin_cos}` and `f32::{sin, cos, sin_cos}` workspace-wide;
  only `trig.rs` allows them, and `source_rules.rs` asserts the bans (GEN-R6).
- `rs-ci` also runs the snapshot suite in release (`rs-snap-release`, TEST-R9), so the
  optimiser can never again change a frame unnoticed.

## Consequences
- Dev, release, incremental and fat-LTO builds give identical frames for every scene (checked
  by the hash sweep); no snapshot changed, because the dev builds that recorded them happened
  not to fuse the affected pairs.
- A sin/cos pair costs two libm calls instead of one `sincos` plus a tail-call branch each:
  a few nanoseconds per pair, well inside the frame budget (bench before and after).
- Cross-platform bit-identity (glibc vs Apple libm) is still not promised; this ADR only
  removes the dependence on the optimiser within one platform.

## Alternatives considered
- **A pure-Rust sin/cos (musl kernels / the `libm` crate)**: also platform-independent, but a
  new dependency in `sciart-core` (or ~200 lines of range reduction to own) for a guarantee no
  test asks for yet.
- **`black_box` around arguments**: blocks the combine only where remembered, and pessimises
  surrounding code.
- **Fix only Box–Muller**: removes today's symptom and leaves the other ~370 call sites to
  break the next time inlining shifts.
- **Weaken the snapshot test to debug only**: hides the bug; the shipped binary is release.
