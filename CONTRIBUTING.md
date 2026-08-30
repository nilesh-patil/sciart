# Contributing to sciart

Thanks for helping. sciart is a Rust workspace of 85 terminal scenes; the rules below keep it
deterministic, fast and honest about what it models. They apply to all contributors
alike.

The docs map is [docs/README.md](docs/README.md): specs, ADRs, design notes, the gallery and
the roadmap. Open follow-ups live in [docs/BACKLOG.md](docs/BACKLOG.md), not in the roadmap.

## Setup

The project uses [pixi](https://pixi.sh) for every task (ADR-0007); the Rust toolchain itself
comes from `rust-toolchain.toml` via rustup.

```sh
git clone https://github.com/nilesh-patil/sciart
cd sciart
pixi install
pixi run rs-build
pixi run rs-sim          # cycle the scenes; `pixi run rs-run aurora` for one
```

Plain `cargo build` / `cargo test` work too, but the gate is defined in `pixi.toml`; use the
pixi tasks when in doubt. Useful focused loops:

```sh
cargo test -p sciart-scenes-<crate> --lib <scene>::   # while iterating on one scene
cargo run -q -p sciart-generators --example frames -- <scene> --inks
```

## The gate

Run this before every commit:

```sh
pixi run rs-ci
```

It runs `cargo fmt --all --check`, clippy with `-D warnings` (with and without
`--all-features`), the whole test suite under `cargo nextest` (TEST-R10), and the snapshot
suite again in the optimised `ci` profile (TEST-R9). CI mirrors the same commands.

`pixi run rs-fmt` formats; `pixi run rs-test` runs tests only. A full `rs-test` takes about
40 s on a warm machine; the snapshot run is the slow part.

## Spec-driven, test-driven

1. **Spec first.** Behaviour changes start in `docs/specs/SPEC-*.md` with a numbered
   requirement (`CORE-R22`, `GEN-R4`, `RT-R9`, …). Significant design choices get an ADR
   (copy `docs/adr/0000-template.md`).
2. **Test first.** Write a failing test that cites the requirement ID in its name (e.g.
   `core_r10_advance_clamps_to_max_steps`; chains like `core_r9_r10_…` count). Watch it fail.
3. **Implement** the smallest thing that passes, then refactor.
4. **Gate and commit.** `pixi run rs-ci`, then a small conventional commit with the ID:
   `feat(core): clamp fixed-step backlog [CORE-R10]`.

`crates/sciart/tests/spec_traceability.rs` fails if a requirement ID has no citing test and is
not listed in SPEC-006's manual table; it also fails on citations of undefined IDs. Keep it
green rather than bending it.

## Backlog review

The [roadmap](docs/ROADMAP.md) is milestone-level; the detail for anything not done lives in
the [backlog](docs/BACKLOG.md). At every milestone close:

1. Walk every `open` row that belongs to the milestone and decide: fix it, narrow it, mark it
   `superseded`, or mark it `won't fix` with the reason.
2. No item survives **two milestones** without a decision. A row that is still open after two
   closes is either scheduled or closed — not silently carried.
3. Keep the statuses truthful: `done` rows stay for the roadmap box they let close;
   `superseded` rows name the later work that replaced them.
4. Replace the milestone's long follow-up sentence in `docs/ROADMAP.md` with a one-line link
   to the backlog, and tick or annotate the boxes the backlog has resolved.

The `samples/compare/` sheets and the historical samples table are part of this review: if a
milestone changes a scene's look, regenerate them before closing it.

Then run the [15-minute milestone review](docs/REVIEW.md) — the standing (art,
science, Rust/perf) checklist — and record its score in the milestone's design doc.

## Adding a scene

A new scene touches five places, in this order:

1. **Spec.** Add `### <name>  (<category label>, <milestone>)` to
   `docs/specs/SPEC-002-generators.md` with intent, model, motion, resize behaviour, inks,
   title and acceptance IDs (`XXX-A1`, …).
2. **Module.** Create `crates/sciart-scenes-<crate>/src/<name>.rs`, where the crate follows
   `Meta.category` (cosmos = Deep Cosmos + Sun & Planets; earth = Earth & Sky + Voyages;
   physics = Forces & Waves; matter = Reactions + States of Matter; life = the four life
   categories). Export `pub static META: Meta`, `pub fn build() -> Box<dyn Generator>`, the
   pure physics functions, and failing unit tests named after the acceptance IDs.
3. **Registration.** Add `pub mod <name>;` to that crate's `lib.rs`, then
   `<name> in sciart_scenes_<crate>` to `registry![...]` in
   `crates/sciart-generators/src/lib.rs`, keeping the list alphabetical.
4. **Tests and snapshots.** Implement until the contract, property, allocation and snapshot
   suites pass. New scenes are picked up by those suites automatically.
5. **Gate and bench.** `pixi run rs-ci`, then
   `cargo bench -p sciart-generators --bench frame -- <name>/200x50` (and `pixi run rs-worst`).
   A new scene should stay under 2 ms worst frame at 200×50 and aim for under 1 ms.

## Snapshot review

Snapshots are seed-42, 80×24 ASCII frames at t = 0, 1 and 5 s for every scene. An intentional
change is recorded with

```sh
INSTA_UPDATE=always cargo test -p sciart-generators --test snapshots
```

then reviewed (`cargo insta review`, or read the `.snap` diffs). **Never accept a snapshot you
have not looked at** (TEST-R2). A diff that is not explained by your change is a bug, not an
update.

## Determinism and allocation rules

- **No wall clock, no global RNG.** Use `dt`, the scene's accumulated time and `ctx.rng`.
  `clippy.toml` bans `Instant::now`, `SystemTime::now`, `rand::rng` and `rand::random`.
- **Trigonometry goes through `Trig`** (`x.sin_det()`, `x.cos_det()`, `x.sin_cos_det()`), never
  `f64::sin`/`cos`/`sin_cos`. LLVM can fuse a sin+cos pair differently between builds, which
  changed frames; clippy enforces this (GEN-R6, ADR-0016).
- **`update` and `render` must not allocate** in steady state (GEN-R11): size buffers in
  `resize`, write text into stack buffers or in pieces, and never `format!` in `render`.
  Rasterise in `prepare`, behind a dirty flag (GEN-R20).
- **No `unsafe`.** The workspace forbids it; the dev-only `sciart-testalloc` counting allocator
  is the single documented exception (ADR-0009).
- **Every Unicode glyph needs an ASCII fallback** (`Glyph::new('★', '*')`, GEN-R10).
- **No panics** for any size ≥ 1×1, resize order or `dt` in `[0, 0.25]` (GEN-R4).
- **`Meta.title` is a plain phenomenon · mechanism line**, ≤ 32 cells, no formulas, and never
  claims more than the model does (GEN-R12).
- Terminal cells are about 2:1 tall: use `geom::polar`/`geom::ASPECT` for round shapes, or the
  square braille lattice.



## Commits and pull requests

- Small commits, conventional style, with requirement IDs:
  `fix(render): clamp the HUD to the last row [RND-R4]`.
- Keep `docs/ROADMAP.md` checkboxes truthful and update the changelog for user-visible changes.
- Open work goes in the [backlog](docs/BACKLOG.md), not into a longer roadmap sentence.
- In a pull request, say what changed, which requirement IDs it serves and which checks ran.
- New to the codebase? Start from [docs/README.md](docs/README.md), then
  `docs/specs/SPEC-001-core.md` (the generator contract) and the contributor guide (the full engineering
  guide).
