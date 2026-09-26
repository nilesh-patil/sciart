# ADR-0009: Allocation tests via a test-only counting allocator crate

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
SPEC-001 promises that `ParticlePool` never allocates after construction except when
`set_capacity` grows (CORE-R28), that `Trail::push` does not allocate (CORE-R29), and that
`Grid::step` does not allocate after its first call (CORE-R36). The house rule behind
the GEN-R8 frame budget is that `Generator::render` never allocates in steady state.
None of this was tested: counting allocations needs a `#[global_allocator]` whose type
implements `unsafe trait GlobalAlloc`, and the workspace sets `unsafe_code = "forbid"`,
which an inner `#[allow]` cannot relax.

Constraints: a binary has exactly one global allocator, so allocation tests need their
own integration-test binaries; libtest runs tests on parallel threads, so a
process-wide counter is polluted by other tests; product crates must keep `forbid`.

## Decision
Add `crates/sciart-testalloc` (`publish = false`, used only as a **dev-dependency**):
- `CountingAlloc`: forwards every call to `std::alloc::System` and bumps **per-thread**
  counters (`const`-initialised `thread_local!` `Cell<u64>`s: no lazy init, no TLS
  destructor, so no re-entrant allocation). This `unsafe impl GlobalAlloc` is the only
  `unsafe` in the workspace, with a `// SAFETY:` comment on the impl and on every block.
- `measure(f) -> (R, Allocs)` counts `alloc`/`alloc_zeroed`/`realloc`/`dealloc` and bytes
  on the current thread during `f`. It panics if `CountingAlloc` is not installed, so a
  forgotten `#[global_allocator]` cannot yield a vacuous zero.
- The crate does not install itself. Each allocation test is its own file that declares
  `#[global_allocator] static GLOBAL: CountingAlloc = CountingAlloc;` (safe code):
  `sciart-core/tests/alloc_free.rs`, `sciart-generators/tests/alloc_free.rs`.
- Lints: this crate cannot use `[lints] workspace = true`. It copies the workspace table
  with `unsafe_code = "deny"` (so the one impl opts in with `#[allow(unsafe_code)]`) and
  adds `unsafe_op_in_unsafe_fn = "deny"` and `clippy::undocumented_unsafe_blocks = "deny"`.
  `tests/lints_mirror.rs` fails if the copy drifts from `[workspace.lints]`.
- The generator suite runs every registered scene (one scoped thread each) through
  `Stage` at 200×50 and 80×24, measures `step` and `render` separately after warm-up,
  prints a per-scene table, and fails on any allocation not in an explicit allowlist
  (`RENDER_ALLOCATES`, `UPDATE_ALLOCATES`); a stale allowlist entry fails too (ratchet).

## Consequences
+ CORE-R28/R29/R36 and render allocation-freedom are now executable, with ~50 lines of
  audited `unsafe` confined to a crate that never reaches a published artifact or the
  `sciart` binary. Product crates keep `unsafe_code = "forbid"`.
+ Zero third-party dependencies; per-thread counting keeps tests parallel.
+ Mutation-checked: replacing the pool's/trail's reserved `Vec` with `Vec::new()` or
  reallocating the grid scratch per step makes the core tests fail.
− Allocations made on other threads during `measure` are invisible (fine: generators
  are single-threaded by contract).
− Counts are debug-build counts; a release-only optimisation that removes an allocation
  is not credited (conservative).
− One more crate whose lints must be kept in sync (guarded by a test).

## Alternatives considered
- **`allocation-counter` / `stats_alloc` / `assert_no_alloc` dev-dependency.** Not in the
  offline registry; network fetch works but adds a third-party dependency (and its
  `unsafe`) to audit. `stats_alloc` counts process-wide, so parallel tests pollute it;
  `allocation-counter` installs its own global allocator from a library, which then
  applies to every test target and bench linking it unless feature-gated;
  `assert_no_alloc` aborts/warns rather than counting, which cannot produce the
  per-scene report. The code we need is ~50 lines; owning it is cheaper than auditing.
- **Relax `unsafe_code` to `deny` workspace-wide.** Weakens the guarantee for product
  crates to make tests possible; rejected.
- **`dhat` heap profiling in tests.** Heavier, process-wide, and needs its own allocator
  anyway.
