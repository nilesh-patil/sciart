# SPEC-006: Testing & quality gates

**Status**: Implemented (all layers, including `alloc_free` and `spec_traceability`, cover all 41 scenes); TEST-R6..R8 (leak guards) specified for M11

## TDD workflow
1. Pick the next unchecked item in `docs/ROADMAP.md`.
2. Add/extend requirement IDs in the relevant SPEC if behavior is new.
3. Write a failing test whose name or doc comment cites the ID (e.g. `core_r10_advance_clamps_to_max_steps`).
4. Implement the minimum to pass; refactor; `pixi run rs-ci`.
5. Tick the roadmap item; commit (`feat(core): … [CORE-R10]`).

## Layers
| Layer | Tool | Location | Scope |
|-------|------|----------|-------|
| Unit | `#[test]` | next to code (`mod tests`) | core types, runtime state machine, themes |
| Contract | `#[test]` table over `registry()` | `crates/sciart-generators/tests/contract.rs` | GEN-R4..R10 for **every** generator: long run, fixed resize sequence, 1×1/1×8/8×1/2×2, motion, determinism, ASCII purity |
| Property | `proptest` | `crates/sciart-generators/tests/props.rs`, core `tests/` | random `(size, dt)` op sequences per generator; FixedStep invariants |
| Snapshot | `insta` | `crates/sciart-generators/tests/snapshots.rs` | seed 42, 80×24, ASCII mode, frames after 0 s / 1 s / 5 s |
| End-to-end | `portable-pty` | `crates/sciart/tests/pty_smoke.rs` | real binary in an 80×24 pty: draws, restores the terminal (RND-R5), keys work, HUD visible |
| Bench | `criterion` | `crates/sciart-generators/benches/frame.rs` | update(1/60)+render at 200×50 and 400×100 per generator |
| Leak (M11) | `sciart_testalloc` live bytes | `crates/sciart-generators/tests/leak_free.rs` | resize churn and a live-bytes ceiling per generator (TEST-R6, TEST-R7) |

- **TEST-R1** Every requirement ID defined in `docs/specs` (bold `**X-RN**` requirements and backticked `X-AN` IDs on `- **Acceptance**` lines) is cited by ≥ 1 test — in its name (chains like `core_r9_r10_…` count), the comments/attributes directly above it, or its body — or is listed here as manual. Enforced by `crates/sciart/tests/spec_traceability.rs`, which also rejects citations of undefined IDs and stale manual entries (its `MANUAL` list must match this table):

  | ID | Verified by |
  |----|-------------|
  | GEN-R8 | `pixi run rs-bench` (release timing; all 41 scenes ≤ 0.55 ms @ 200×50 on M-series: the 25 unchanged scenes 2026-09-26, the 16 M10 scenes ≤ 0.47 ms (max: aurora), 2026-09-27) |
  | TEST-R2 | Process: snapshot diffs reviewed by a person |
  | TEST-R3 | Process: `pixi run rs-ci` before every commit |
- **TEST-R2** Snapshot changes are reviewed with `cargo insta review`; never blindly accepted.
- **TEST-R3** `pixi run rs-ci` = `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`. Must pass before every commit.
- **TEST-R5** Allocation guarantees (CORE-R28, CORE-R29, CORE-R36, GEN-R11) are verified by counting heap operations on the current thread with `sciart_testalloc::CountingAlloc` (ADR-0009). Each such test file is its own integration-test binary declaring `#[global_allocator]`. `sciart-testalloc` is `publish = false`, only ever a dev-dependency, the only crate allowed `unsafe` (a single `#[allow(unsafe_code)]` on `impl GlobalAlloc`), and mirrors `[workspace.lints]` except `unsafe_code = "deny"` (enforced by its `tests/lints_mirror.rs`).
- **TEST-R4** Proptest case counts are kept modest (≤ 64 via `ProptestConfig::with_cases`) in `sciart-generators` and `sciart` so `rs-test` stays < 60 s; cheap pure properties in `sciart-core` may use proptest's default (256). `PROPTEST_CASES` can raise counts locally.
- **TEST-R6** (M11) `sciart-testalloc` also tracks live heap bytes on the current thread: `live_bytes()` is bytes allocated minus bytes freed, with a `realloc` counted as its size delta; a failed (null) allocation or `realloc` changes nothing. `crates/sciart-generators/tests/leak_free.rs` (its own test binary, TEST-R5) drives every registered scene through `Stage`: build at 80×24, then ten cycles of resizing 80×24 → 200×50 → 1×1 → 80×24, with 30 steps of 1/60 s and one render at each size. Live bytes after each of cycles 3..10 equal live bytes after cycle 2, so slow amortised growth (one small element per cycle) must cross a capacity doubling and show. The first two cycles are warm-up: they settle every buffer's largest capacity, including both halves of a double buffer swapped by `grid::regrid_*` (an odd number of regrids per cycle alternates their roles). Any difference is a leak or unbounded growth.
- **TEST-R7** (M11) For every scene, the live bytes held after building it through `Stage` at 500×150 (canvases included), stepping 1 simulated second and rendering once, measured from just before the build, stay under their ceiling in `leak_free.rs`: `LIVE_CEILING` = 4 MiB (M11 measurement: every scene but the exceptions holds < 4 MiB), or the scene's own entry in `EXCEPTIONS`. Ceilings target ≤ 8 MiB and may only be lowered; raising one or adding an exception needs a spec change that names the scene and the reason here. Exceptions: fractal_growth, 15 MiB (14.5 MiB measured: its braille-resolution DLA lattice is uncapped); capping that lattice (ADR-0012, M12) removes the entry. A per-scene exception keeps one outlier from loosening the guard for every other scene.
- **TEST-R8** (M11) `clippy.toml` `disallowed-methods` bans `std::boxed::Box::leak`, `std::vec::Vec::leak`, `std::string::String::leak`, `std::mem::forget` and `std::mem::ManuallyDrop::new` workspace-wide, so memory cannot be leaked on purpose; a source-rules test asserts every entry, as GEN-R7's test does for the clock bans. Code that truly needs one uses `#[allow(clippy::disallowed_methods)]` with a reason (none today).
