# SPEC-006: Testing & quality gates

**Status**: M1 implemented (all four layers wired for starfield)

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

- **TEST-R1** Every implemented requirement ID is cited by ≥ 1 test name (`core_r10_…`), or has a non-test verification listed here:

  | ID | Verified by |
  |----|-------------|
  | GEN-R3 | Process: `registry!` macro + contract suite iterates `registry()` |
  | GEN-R7 | `clippy.toml` `disallowed-methods` + `-D warnings` in `rs-lint` (checked by injecting `Instant::now`/`rand::random` into starfield: both flagged) |
  | GEN-R8 | `pixi run rs-bench` (starfield: 18 µs @ 200×50, 71 µs @ 400×100 on M-series, 2026-09-26) |
  | RND-R5 | `crates/sciart/tests/pty_smoke.rs::rnd_r5_run_draws_stars_and_restores_the_terminal` (80×24 pty: stars on screen, `q` ⇒ exit 0, alt-screen enter/leave balanced) |
  | TEST-R* | Process (this document) |
- **TEST-R2** Snapshot changes are reviewed with `cargo insta review`; never blindly accepted.
- **TEST-R3** `pixi run rs-ci` = `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`. Must pass before every commit.
- **TEST-R4** Proptest case counts are kept modest by default (64) so `rs-test` stays < 60 s; `PROPTEST_CASES` can raise it locally.
