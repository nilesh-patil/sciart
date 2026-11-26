# Contributing to sciart

Thanks for helping. sciart is a Rust workspace of 85 terminal scenes; the rules below keep it
deterministic, fast and honest about what it models.

This is the contributor guide: setup, commands, the gate, the spec-first workflow, adding a
scene and reviewing changes. How the code is organised (the crate map, the generator contract,
the look rules and which suite enforces which requirement) is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The documentation map is
[docs/README.md](docs/README.md). Open follow-ups live in [docs/BACKLOG.md](docs/BACKLOG.md),
not in the roadmap.

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
pixi tasks when in doubt. Extra arguments after a task name are passed to the command
(`pixi run rs-sim --hud`).

## Commands

| Task | Command |
|------|---------|
| Full gate — run before every commit | `pixi run rs-ci` (fmt check + clippy `-D warnings` with and without `--all-features` + all tests under cargo-nextest, TEST-R10 + `pty_smoke` and `encoder_fidelity` again with `NO_COLOR=1`, `rs-test-nocolor`, TEST-R12 + the snapshot suite again in the optimised `ci` profile, TEST-R9) |
| Library and feature sets (LIB-R4/R7, CLI-R15; part of `rs-ci`) | `pixi run rs-test-features` (the binary without the recorder, the library's doctests, and the library-only tests with `png,widget,unstable-generator`); `rs-lint` also lints `--no-default-features` and `png,widget` |
| Tests only / build | `pixi run rs-test` (nextest on the opt-level 2 `test` profile with debug assertions; extra args go to nextest: `pixi run rs-test contract`) / `pixi run rs-build`; `pixi run rs-test-parity` proves rs-test runs exactly `cargo test --workspace`'s tests |
| Format | `pixi run rs-fmt` |
| Benchmarks (GEN-R8) | `pixi run rs-bench` (or one scene: `cargo bench -p sciart-generators --bench frame -- <name>/200x50`); worst single frames: `pixi run rs-worst [<name> ...] [--size 200x50,500x150] [--over 2]` |
| Resource cost + art metrics (RND-R13) | `pixi run rs-cost [<name> ...] [--size WxH] [--sort bytes\|cpu\|cov\|cmass\|edge\|p99l]` (the real 30 FPS pipeline: µs per stage, bytes/frame, blockiness, darkness, lit coverage, chroma mass, edge weight, p99 luma; `--emit-limits` prints `art_metrics` rows) |
| Size and weight budget (RT-R22) | `pixi run rs-size` (release build without `record`/`mouse`: bytes and crate count against the budgets in SPEC-003; the budgets only go down without a spec change) |
| Browser player and docs site (WEB-R3..R8; part of `rs-ci`) | `pixi run web-build` (the WebAssembly module from `web/`, its own `web/target`), `web-test` (a player frame equals the capture renderer's), `web-smoke` (the compiled module under wasmtime: every scene draws), `site-build` (guide, gallery and player into `site/dist/`), `site-check` (all of these, then module weight, no other origin, links, install text and guide flags against the binary). Guide chapters: `docs/guide/` in the order of `chapters.toml` |
| Release configuration and packaging (DIST-R1..R10; steps in [docs/RELEASING.md](docs/RELEASING.md)) | `scripts/release_check.py`, `scripts/installer_check.py` and their tests run in `rs-spec-check`; `pixi run rs-package-check` (crates.io dry run of the twelve published crates, then their packed files; needs a cargo with `publish --workspace`); `pixi run -e dist rs-installer-check` (`packaging/install.sh` against a fixture release, shellcheck required); `python3 scripts/bump_version.py <sciart-version> <internal-version>` sets both version streams and every lock file |
| List scenes | `pixi run rs-list` |
| Cycle all scenes | `pixi run rs-sim` (e.g. `pixi run rs-sim --hud --ascii --fps 60 --seed 7`) |
| Run one scene | `pixi run rs-run <name>` (same flags) |
| Review frames as text / truecolor PNG | `cargo run -q -p sciart-generators --example frames -- <name> --inks` / `pixi run rs-preview <name> --size 120x40 --t 0,5,20 --theme astro --out /tmp/p/<name>` (`--sheet` stacks the times; `--swatches` shows every field palette) |
| Install the binary | `pixi run rs-install` (the default build, headless recorder included; `rs-install-record` is its older name), or `rs-install-mouse` for the mouse keep-awake backend; then `sciart sim` |
| Record a scene | `pixi run rs-record <name> --out out.gif [--start S] [--seconds N]` (feature `record`, on by default; PNG/asciicast too; the capture draws every scene glyph natively, CLI-R18: a new glyph in a scene needs a bitmap in `sciart-capture/src/glyphs.rs`) |
| Animated showcase (WEB-R1/R2) | `pixi run rs-showcase` records missing or stale clips from `docs/showcase.toml` into `docs/media/` (`--force` after a look change); `pixi run rs-showcase-check` (in `rs-ci`) verifies clips, lock, sizes, coverage and README/gallery references |

`rs-sim`, `rs-run`, `rs-preview`, `rs-cost` and `rs-worst` build the `ci` profile (opt-level 3,
16 codegen units, no LTO; frames are identical to `release`, GEN-R6); only `rs-install` builds
`release` (thin LTO, one codegen unit). Benches keep `bench`/`release`.

Focused loops are fine while iterating, but finish with `pixi run rs-ci`:

```sh
cargo nextest run -p sciart-generators <scene>::      # or: cargo test -p sciart-generators --lib <scene>::
cargo test -p sciart-scenes-<crate> --lib <scene>::   # one scene's unit tests in its own crate
cargo run -q -p sciart-generators --example frames -- <scene> --inks
```

Both `nextest` and `cargo test` build the optimised `test` profile that `rs-test` reuses.

### Running the art

- **Keys**: `n`/`→` next · `p`/`←` previous · `space` pause · `r` reset scene · `h` HUD · `t`
  theme (astro → rainbow → mono → aurora → colorblind) · `?` help · `f` favourite · `v`
  favourites only · `[`/`]` category · `s` shuffle · `q`/`Esc`/`Ctrl-C` quit.
- **Config file**: `~/.config/sciart/config.toml` (see
  [docs/config.example.toml](docs/config.example.toml)); flags override it.
- **Scriptable modes** (ADR-0031): `--for <duration>` ends a run on a wall-clock deadline held
  by the pure `Runtime` (RT-R23); `sciart while -- <command>` runs a command beside the art with
  its output spooled and replayed, exiting with its status (124 deadline, 127 cannot start, 130
  Ctrl-C; CLI-R14); `sciart list --json` is a versioned object (CLI-R16);
  `sciart completions <shell>` and `sciart man [subcommand]` are generated from the clap
  definition (CLI-R17).

The [user guide](docs/guide/index.md) covers the same ground for people running sciart.

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
   `core_r10_advance_clamps_to_max_steps`; `rt_r3_…`; chains like `core_r9_r10_…` count).
   Watch it fail.
3. **Implement** the smallest thing that passes, then refactor.
4. **Gate and commit.** `pixi run rs-ci`, then a small conventional commit with the ID:
   `feat(core): clamp fixed-step backlog [CORE-R10]`.

`pixi run rs-spec-check` (`scripts/spec_check.py`) fails if a requirement ID has no citing
test or checker script and is not listed in its `MANUAL` table (and SPEC-006's TEST-R1 table)
with a reason; it also fails on citations of undefined IDs. Keep it green rather than bending
it.

## Adding a scene

A new scene touches five places, in this order:

1. **Spec.** Write the scene section in the category file
   `docs/specs/SPEC-002-scenes-<crate>.md`, headed `### <name>  (<category label>, <milestone>)`
   (GEN-R15): intent, model, motion, resize, inks, title and acceptance IDs (`XXX-An`). Add the
   scene to the generated index in `docs/specs/SPEC-002-generators.md` with
   `python3 scripts/spec_index.py`.
2. **Module.** Create `crates/sciart-scenes-<crate>/src/<name>.rs` in the crate its category
   maps to (ADR-0023: cosmos = Deep Cosmos + Sun & Planets; earth = Earth & Sky + Voyages;
   physics = Forces & Waves; matter = Reactions + States of Matter; life = the four life
   categories), and add `pub mod <name>;` to that crate's `lib.rs`. Export
   `pub static META: Meta` (category, `title`, and `captions` + `caption()` if it has acts;
   GEN-R12), `pub fn build() -> Box<dyn Generator>`, pure `pub(crate)` physics functions, and
   failing unit tests named after the acceptance IDs (`xxx_a1_…`). Leave the bottom-left title
   span free of text (GEN-R13).
3. **Registration.** Add `<name> in sciart_scenes_<crate>` to `registry![...]` in
   `crates/sciart-generators/src/lib.rs`, keeping the list alphabetical. Focused loop:
   `cargo test -p sciart-scenes-<crate> --lib <name>::`.
4. **Implement until green**, then record and review the new `.snap` files and run
   `pixi run rs-ci`. Contract, props, snapshots, source rules, allocation, leak and bench
   suites pick the scene up automatically.
5. **Bench and tick.** `cargo bench -p sciart-generators --bench frame -- <name>/200x50` and
   `pixi run rs-worst <name>`: a new scene stays under 2 ms worst frame at 200×50 and aims for
   under 1 ms. Then tick the roadmap item.

Good references (under `crates/sciart-scenes-*/src/`): `starfield.rs` (minimal),
`solar_system.rs` / `doppler_effect.rs` (house style), `cosmic_microwave_background.rs` (cheap
per-cell fields via per-resize sin/cos tables), `moon_landing.rs` (`Timeline` + `ode`). The
rules every scene must follow are in
[ARCHITECTURE: the generator contract](docs/ARCHITECTURE.md#the-generator-contract).

## Snapshot review

Snapshots are seed-42, 80×24 ASCII frames at t = 0, 1 and 5 s for every scene. Inspect the
`crates/sciart-generators/tests/snapshots/*.snap` diffs (or `cargo insta review` if
cargo-insta is installed). An intentional change is recorded with

```sh
INSTA_UPDATE=always cargo test -p sciart-generators --test snapshots
```

**Never accept a snapshot you have not looked at** (TEST-R2). A diff that is not explained by
your change is a bug, not an update.

## Reviewing a changed scene

A scene whose look or model changed gets a short review on top of the gate before its roadmap
box is ticked. Render it at the gallery size, in ASCII and at a small size:

```sh
pixi run rs-preview <scene> --size 160x48 --t 0,5,20 --theme astro --sheet --out /tmp/review/<scene>
pixi run rs-preview <scene> --size 80x24 --t 0,5,20 --theme astro --ascii --sheet --out /tmp/review/<scene>-ascii
pixi run rs-preview <scene> --size 40x12 --t 5 --theme astro --sheet --out /tmp/review/<scene>-small
```

Compare against the scene's current frame in `samples/current/`. For the RND-R19 set,
`pixi run rs-colorblind-review` renders the colour-blind sheets.

- **Art.** The subject is the first thing seen at 160×48; the ground is black and, in the
  default opaque mode, every cell has an explicit black background; the main contrast is a
  complementary pair (`Hot`/`Cold`, or `Accent` on a neutral ground); nothing important leaves
  the frame at 160×48, 80×24 or 40×12; the bottom-left title keeps ≥ 4 blank cells before any
  right-aligned readout (GEN-R13); the `--ascii` frame still reads; 1×1 does not panic
  (GEN-R4).
- **Science.** The title is a plain `Phenomenon · mechanism` line that claims no more than the
  model simulates (GEN-R12); stylisation (exaggerated scale, compressed time, a kinematic
  stand-in) is stated in the scene's *Stylisation* line; captions follow the `Timeline`; the
  acceptance tests still test the stated property; the section's heading label matches the
  category and `python3 scripts/spec_index.py --check` passes. Update the scene's row in
  [docs/science/claims.md](docs/science/claims.md).
- **Performance.** `pixi run rs-worst <scene> --size 200x50,500x150` stays under 2 ms at
  200×50 and 5 ms at 500×150 (GEN-R8); `pixi run rs-cost <scene>` bytes and changed cells are
  in family with its peers; re-seed the `art_metrics` band (`rs-cost -- <scene> --emit-limits`)
  only after the look is reviewed; `alloc_free` and `leak_free` stay green.
- **Gallery.** Regenerate the changed scenes' frames with `pixi run rs-gallery`;
  `python3 scripts/gallery.py --check` passes and `git status samples/current docs/GALLERY.md`
  shows only those scenes.

## Determinism and allocation rules

The full list is in [ARCHITECTURE](docs/ARCHITECTURE.md#the-generator-contract); the ones that
most often bite:

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

## Conventions

- Rust 2024 edition, MSRV 1.88; workspace lints in the root `Cargo.toml`
  (`unsafe_code = "forbid"`; sole exception: the dev-only `sciart-testalloc`, ADR-0009).
  Outside the workspace, `web/src/wasm.rs` carries the `#[unsafe(no_mangle)]` export
  attributes and nothing else unsafe (ADR-0034).
- Proptest case counts: ≤ 64 (`ProptestConfig::with_cases`) in `sciart-generators` and
  `sciart`; cheap pure properties in `sciart-core` may use the default (TEST-R4).
- Errors: library crates don't panic on user input; the binary uses `anyhow` and exits with
  code 2 for usage errors (`cli::UsageError`: unknown generator, bad/missing config,
  everything disabled).
- Keep terminal I/O out of `runtime.rs`: new behaviour goes into the pure `Runtime`
  (unit-tested), with `app.rs` only translating events and drawing.
- The GEN-R7 clock ban is workspace-wide; binary or test code that truly needs wall time uses
  `#[allow(clippy::disallowed_methods)]` with a reason.
- Comments explain *why*; cite requirement and ADR IDs where a rule comes from a spec.
- No new dependencies in `sciart-core` without an ADR.

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

## Environment notes

- Every pixi task routes rustc through `sccache` from the pixi environment
  (`scripts/pixi-activate.sh`, TEST-R11; `sccache --show-stats`). Plain `cargo` outside pixi
  does not use it; `RUSTC_WRAPPER= pixi run …` turns it off.
- Platforms (TEST-R13, ADR-0029): macOS arm64 and Linux x86_64 run lint and the full suite;
  Linux aarch64 runs the full suite; Intel macOS and Linux musl get release builds only.
  Frames are deterministic per platform, not across platforms, so they are pinned on macOS
  arm64 only: the snapshot suite and `m11_fidelity` skip themselves elsewhere. Record and
  review snapshots there. A unit test of a chaotic model asserts a margin that holds over
  many seeds (measure 64 or more), never a threshold one platform's rounding decides.
- Keep-awake defaults to `caffeinate` on macOS (ADR-0005) and to `inhibit` on Linux for a local
  session, `off` over SSH (KA-R7/R8, ADR-0028). The `mouse` backend is an opt-in cargo feature
  (`pixi run rs-install-mouse`) and needs Accessibility permission.
- Tests that spawn `caffeinate` must leave nothing running: `pgrep -fl 'caffeinate -dimsu'`
  should be empty after `rs-test`.
- The pty tests use an 80×24 terminal; the HUD fits itself to the width (RND-R4), so assert on
  flags like `| PAUSED`, not on trailing separators.
- `molecular_dynamics`' condensation test takes about 10 s in a debug build; that is expected.
- More detail (build profiles, signals, low-power behaviour) is in
  [ARCHITECTURE: platforms and environment](docs/ARCHITECTURE.md#platforms-and-environment).

## Commits and pull requests

- Small commits per roadmap item, conventional style, with requirement IDs:
  `fix(render): clamp the HUD to the last row [RND-R4]`.
- Stage explicitly: pending `git rm`/`git mv` changes ride along with the next commit.
- Keep `docs/ROADMAP.md` checkboxes truthful and update the changelog for user-visible changes.
- Open work goes in the [backlog](docs/BACKLOG.md), not into a longer roadmap sentence.
- In a pull request, say what changed, which requirement IDs it serves and which checks ran.
- New to the codebase? Start from [docs/README.md](docs/README.md), then
  `docs/specs/SPEC-001-core.md` (the core types and the generator contract) and
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (the crate map, the rules and the test suites).
