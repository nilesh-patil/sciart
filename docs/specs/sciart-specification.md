# Project Specification: sciart

**Status**: Approved (spec interview, 2026-09-26; current as of M23, 2026-10-18)
**Repository**: `https://github.com/nilesh-patil/sciart`
**Supersedes**: the Python prototype (removed at the M8 cutover)

## Executive Summary

`sciart` is real-time generative science and astronomy art for the terminal: **85 animated
scenes at 30 FPS**, drawn with ratatui + crossterm, while keeping the machine awake (macOS
`caffeinate` by default, optional mouse jiggle). It replaced the Python prototype, which
rendered one static frame every 15 seconds; the Python era ended at the M8 cutover.

The implementation is a Cargo workspace with a small, testable core (canvas, generator
trait, fixed-step clock, seeded RNG), five category scene crates behind a registry facade,
shared models used by several scenes, a renderer with semantic colour palettes, a
keep-awake crate, and a thin CLI binary. Development follows **SDD** (numbered
requirements in `docs/specs/SPEC-*.md`, decisions in `docs/adr/`) and **TDD** (tests cite
requirement IDs; contract, property, snapshot, end-to-end, allocation, leak and benchmark
layers).

Current scope: 85 scenes in 11 categories, 4 themes (`astro`, `rainbow`, `mono`, `aurora`),
85 scenes × 3 snapshot times, and a hosted CI gate that mirrors the local `rs-ci` task
(ADR-0024).

## Project Goals & Success Criteria

| Goal | Success criterion |
|------|-------------------|
| Smooth animation | Every scene animates at 30 FPS; worst update+render < 2 ms at 200×50 and < 5 ms at 500×150 (release/`ci` profiles); the gallery averages ≈ 0.5 ms and ≈ 4.3 KB per frame (RT-R16) |
| Stability | No generator panics for any `width, height >= 1`, any resize/dt sequence (contract + proptest). The runtime survives any generator panic via quarantine |
| Modularity | A scene depends only on `sciart-core`, `sciart-shared` and `rand`; scene crates never import each other; the core has no terminal/OS/I/O dependencies (ADR-0023, GEN-R16) |
| Determinism | `--seed N` reproduces a run; frames are byte-stable across dev, `ci` and `release` (ADR-0016); snapshot tests are stable |
| Scope | All 26 original Python scenes exist in Rust (M8), and the catalogue has grown to 85 scenes across 11 categories |
| Traceability | Every requirement ID (e.g. `CORE-R3`, `TEST-R12`) is cited by at least one test or listed as manual (TEST-R1) |
| Science honesty | Every scene carries a `Phenomenon · mechanism` title that never claims more than its model (GEN-R12, ADR-0014) |
| Shareability | A stranger can install, run and browse the catalogue from the README and gallery (M23) |

## Technical Architecture

### Technology Stack

| Concern | Choice | ADR |
|---------|--------|-----|
| Language | Rust, edition 2024, MSRV 1.88 | — |
| Structure | Cargo workspace, 12 crates: core, registry facade, five scene crates, shared, render, keep-awake, binary, test allocator | [ADR-0001](../adr/0001-cargo-workspace.md), [ADR-0023](../adr/0023-lean-build-and-runtime.md) |
| Terminal | ratatui + crossterm (alt screen, raw mode, truecolor) | [ADR-0002](../adr/0002-ratatui-crossterm.md) |
| Time | Fixed-step `update(dt)` with accumulator, dt = 1/60 s, `MAX_STEPS = 15` | [ADR-0003](../adr/0003-fixed-step-simulation.md) |
| Colour | Semantic inks mapped by themes; generators never pick RGB | [ADR-0004](../adr/0004-semantic-palettes.md) |
| Keep-awake | `caffeinate` (macOS default) \| `mouse` (enigo, cargo feature) \| `off` | [ADR-0005](../adr/0005-keep-awake-backends.md) |
| Fault isolation | `catch_unwind` + per-session quarantine | [ADR-0006](../adr/0006-panic-quarantine.md) |
| Task runner | pixi tasks wrapping cargo | [ADR-0007](../adr/0007-pixi-task-runner.md) |
| RNG | Injected, seedable `rand_chacha::ChaCha8Rng`; no global RNG in generators | [ADR-0008](../adr/0008-injected-rng.md) |
| Categories (M11) | Sky settings + physics × chemistry × biology lattice, 11 categories | [ADR-0013](../adr/0013-pxcxb-taxonomy.md) |
| Titles (M11) | One science title per scene, drawn by `Stage` | [ADR-0014](../adr/0014-central-science-titles.md) |
| Field colour (M12) | Phase, blackbody and named ramps as semantic roles | [ADR-0015](../adr/0015-phase-blackbody-named-ramps.md) |
| Determinism (M12) | Trig through non-inlined libm wrappers, so no `sincos` fusion changes a frame between builds | [ADR-0016](../adr/0016-profile-independent-trig.md) |
| CLI / config | clap (derive) + TOML file, flags override file | [SPEC-005](SPEC-005-cli-config.md) |
| Tests | built-in test + proptest + insta + nextest + criterion + counting allocator | [SPEC-006](SPEC-006-testing.md) |
| CI / release | GitHub Actions (macOS + Linux, `cargo-deny`, tagged tarballs); local `rs-ci` stays the pre-commit gate | [ADR-0024](../adr/0024-hosted-ci-and-release.md) |

### System Architecture

```
sciart (bin): cli → config → Runtime (state machine) → event loop
  ├── sciart-generators (registry facade)
  │     ├── sciart-scenes-cosmos    (Deep Cosmos, Sun & Planets)
  │     ├── sciart-scenes-earth     (Earth & Sky, Voyages)
  │     ├── sciart-scenes-physics   (Forces & Waves)
  │     ├── sciart-scenes-matter    (Reactions, States of Matter)
  │     ├── sciart-scenes-life      (Living Systems, Physics of Life,
  │     │                            Molecules of Life, Life from Matter)
  │     ├── sciart-shared           (models shared by scenes; never registered)
  │     └── sciart-core             (canvas, Generator, Ctx, FixedStep, helpers)
  ├── sciart-render                 (themes, paint, HUD, terminal guard) → sciart-core
  └── sciart-keepawake              (caffeinate / mouse / off)
```

Dependency arrows point downward only. `sciart-core` has no terminal, OS, or I/O
dependencies. Scene crates depend only on `sciart-core`, `sciart-shared` and `rand`; a scene
never imports another scene (GEN-R16). `sciart-generators` is the only crate that knows the
whole registry.

Frame loop (per iteration of the binary's event loop):

1. Drain crossterm events → `Runtime::handle(Event)` (keys, resize, focus).
2. `steps = FixedStep::advance(real_elapsed)`; call `generator.update(dt, ctx)` `steps`
   times (clamped to `MAX_STEPS = 15` to avoid a spiral of death, RT-R11).
3. `generator.prepare()` once per drawn frame, then `generator.render(&mut canvas)` inside
   `catch_unwind` (GEN-R20; the runtime takes ~2 steps per drawn frame).
4. `sciart-render` maps the canvas through the active theme into the ratatui buffer, draws
   the HUD or error row if needed, and encodes the changed cells (RND-R15/RND-R17).
5. Write the frame as one buffered write; an unchanged frame writes nothing (RT-R15/RT-R17).
6. Sleep until the next frame deadline (30 FPS, or the low-power rate).

### API Design (core contract)

```rust
pub trait Generator: Send {
    fn meta(&self) -> &'static Meta;
    fn resize(&mut self, size: Size, ctx: &mut Ctx); // always called before first update
    fn update(&mut self, dt: f64, ctx: &mut Ctx);    // fixed dt = 1/60 s
    fn prepare(&mut self) {}                         // once per drawn frame, before render
    fn render(&self, canvas: &mut Canvas);           // canvas already sized & cleared
    fn caption(&self) -> Option<&'static str> { None } // live act, one of meta().captions
}
```

Full requirements: [SPEC-001 core](SPEC-001-core.md), [SPEC-002 generators](SPEC-002-generators.md).

### Data Model

- `Size { width: u16, height: u16 }` — both clamped to `>= 1` at construction.
- `Cell { ch: char, ink: Ink, bg: Ink }`; `Canvas { size, glyph_mode, cells: Vec<Cell> }` row-major.
- `Glyph { unicode: char, ascii: char }` — canvas stores whichever the `GlyphMode` selects;
  a unicode glyph that is not single-width falls back to `ascii`.
- `Ink { role: Role, level: u8 }` — semantic role (Star, Planet, Particle, Wave, Accent,
  Text, Hot, Cold, Dim, Seq, Div, …) and intensity 0..=255.
- `Theme` — `fn color(&self, ink: Ink, x: u16, y: u16) -> Rgb`; built-ins `astro`
  (default), `rainbow` (stable per-cell hue), `mono`, `aurora` (RND-R16).
- Config file `~/.config/sciart/config.toml` — see SPEC-005.

## Feature Specifications

### F1: Single-generator mode — `sciart run <name>`
- **Story**: As a user I run one scene full-screen to enjoy/debug it.
- **Acceptance**: unknown name → exit code 2 with the list of valid names; `q`/`Esc`/`Ctrl-C` quits and restores the terminal.

### F2: Simulation mode — `sciart sim` (default subcommand)
- **Story**: As a user I leave it running; scenes change automatically.
- **Acceptance**: auto-advances every `--interval` seconds (default 15) to the next
  non-quarantined generator in a seeded, category-interleaved order (RT-R13, M11), never the same
  one twice in a row (when >1 available).
- Keys: `n`/`→` next, `p`/`←` previous (history), `space` pause/resume the auto-cycle **and** the
  simulation, `r` reset current generator, `h` toggle HUD, `t` cycle theme, `q`/`Esc`/`Ctrl-C` quit
  (RT-R9). The same keys work in `run` mode.

### F3: `sciart list`
- Prints `name  category  description` for every registered generator, sorted by name (category id since M11, CLI-R4).

### F4: HUD overlay
- Toggleable one-line status bar: science title (M11, RND-R9; the id when titles are off), measured FPS, theme, keep-awake state,
  paused flag, quarantine count. Off by default; `--hud` or `h` enables.

### F5: Low-power mode
- After `low_power.idle_after` (default 2 min since M22) without a key press, drop to
  `low_power.fps` (default 5). Any key restores full FPS. Simulation time still advances
  at the fixed dt, so motion stays physically consistent (fewer rendered frames, same speed).
- A picture that stops changing (paused, or a held scene) also steps the draw rate down
  (RT-R17); a resize or a changed frame restores it.

### F6: Keep-awake
- `--keep-awake caffeinate|mouse|off`, default `caffeinate` on macOS, `off` elsewhere.
- Failures never stop the art (logged in HUD); see SPEC-004.

### F7: Fault isolation
- Generator panics are caught; the generator is quarantined for the session, the last good
  frame is kept, and in `sim` mode the runtime advances to the next generator. In `run`
  mode a panic shows an error message and waits for `q` / `r` (reset).

### F8: The 85 scenes
- The 26 original Python scenes were reimagined for continuous animation by M8; the
  catalogue has since grown through M10–M21 to 85 scenes in 11 categories (see the
  [ROADMAP](../ROADMAP.md) and SPEC-002). Each scene is seeded, resize-safe, snapshot-tested,
  and inside the GEN-R8/GEN-R24 performance and art bands.
- The look follows ADR-0018 (M11 idiom) plus the thread (ADR-0019), orbital-thread
  (ADR-0020) and orbital-molecule (ADR-0021) idioms; the M11 scenes are pinned by GEN-R22.

### F9: Science titles (M11)
- Every scene shows a short `Phenomenon · mechanism` title at the bottom-left, fitted to the width and
  dimmed after 8 s; it moves into the HUD while the HUD is on. `title = false` or `--no-title` hides it.
  See ADR-0014, CORE-R71..R76, GEN-R12/R13, RT-R12, RND-R8/R9, CFG-R6, CLI-R7.

## UI/UX Specifications

- Alternate screen + raw mode + hidden cursor; always restored on exit **and** on panic
  (panic hook restores the terminal before printing).
- Unicode glyphs by default (`★ ✦ · ░ ▒ ▓ ☉ ● ◐`); `--ascii` forces pure ASCII.
- Terminal aspect ratio: cells are ~2:1 tall; `core::geom::ASPECT = 2.0` scales x for round shapes.
- No per-frame colour randomization (no strobing).
- The frame encoder writes the shortest sequences that reproduce the painted buffer, never
  an LF (so a frame reaches line-buffered stdout as one write), and skips unchanged frames
  (RND-R17, RT-R15, RT-R17).

## Performance Requirements

- Target 30 FPS (configurable `--fps 1..=120`); worst update+render < 2 ms at 200×50 and
  < 5 ms at 500×150 (GEN-R8).
- No per-frame heap allocation in steady state for the canvas (reused buffer; GEN-R11,
  RT-R14).
- `MAX_STEPS` per frame = 15 (drop simulation time rather than stall; ADR-0003 amended).
- Gallery mean ≤ 4,700 terminal bytes and ≤ 480 changed cells per frame at 200×50 (RT-R16);
  no scene exceeds its art bands (GEN-R24).

## Testing Strategy

See [SPEC-006](SPEC-006-testing.md). Layers:

1. Unit tests next to the code (core types, runtime state machine, themes, scene physics).
2. Contract over `registry()` (GEN-R4..R10, GEN-R12) and proptest over sizes/resizes/dt.
3. Snapshots (insta, seed 42, 80×24 ASCII at t = 0/1/5 s) in the optimised `ci` profile (TEST-R9).
4. End-to-end pty tests of the real binary (keys, HUD, exit codes, keep-awake cleanup).
5. Allocation, live-bytes and leak guards with the counting allocator (ADR-0009, TEST-R5..R7).
6. Art-metric and resource budgets, M11 fidelity, thread/orbital-style rules (GEN-R22..R26, RT-R16).
7. Requirement traceability: every ID is cited by a test or listed as manual (TEST-R1).
8. Benchmarks (criterion) and the `rs-worst`/`rs-cost` probes (GEN-R8, RND-R13).

TDD: a failing test that cites a requirement ID precedes each implementation. The suite is
hermetic with respect to terminal colour configuration (TEST-R12).

## DevOps & Deployment

- pixi tasks: `rs-build`, `rs-test` (nextest + doctests), `rs-fmt`, `rs-lint`, `rs-ci`
  (lint + test + optimised snapshots), `rs-bench`, `rs-worst`, `rs-cost`, `rs-list`,
  `rs-sim`, `rs-run`, `rs-preview`, `rs-install`, `rs-gallery`.
- Hosted CI: GitHub Actions on macOS 14 and Ubuntu runs the same lint/test/snapshot/deny
  jobs (ADR-0024); `pixi run rs-ci` remains the pre-commit gate.
- Platforms: macOS arm64 primary; Linux must build and pass tests. Windows builds
  best-effort; no keep-awake backend there yet.
- Install: `cargo install --git https://github.com/nilesh-patil/sciart sciart --locked`
  (or `pixi run rs-install` in a checkout); release tarballs are built by the tag workflow.
  crates.io publishing is deferred.

## Error Handling & Edge Cases

| Case | Behavior |
|------|----------|
| Terminal 1×1 or 0×0 reported | Size clamps to ≥1; generators must still render |
| Resize mid-scene | `resize()` is called; generator reinitializes dimension-dependent state |
| Long stall (laptop sleep) | FixedStep clamps to `MAX_STEPS = 15`; accumulator is reset |
| Generator panic | Quarantine (F7) |
| All generators quarantined | Show error screen; keep running until quit (keep-awake stays active) |
| `caffeinate` missing / fails | Log in HUD, continue without keep-awake |
| Mouse backend lacks accessibility permission | Log, continue |
| Invalid config file | Exit code 2 with file path + line/col of the TOML error |
| Wide / zero-width unicode glyph | Canvas substitutes the ASCII fallback |
| `NO_COLOR` set in the developer environment | Tests still pass (TEST-R12); the art is unaffected until colour capability lands (planned) |

## Constraints & Assumptions

- The Python implementation was kept until M8 and then removed (M8 cutover).
- Hosted CI mirrors `pixi run rs-ci`; the local gate is still the contract (ADR-0024).
- The registry is the source of truth for the scene count and categories (85 scenes, 11
  categories at M23); docs and the gallery are generated or checked against it.
- macOS is the supported keep-awake experience; Linux is a supported build/test target.

## Future Considerations (M23 improvement programme)

- `sciart record` (GIF/PNG/asciicast) so scenes can leave the terminal.
- In-app help overlay, favourites and category filtering.
- Colour capability negotiation (`auto`/`truecolor`/`256`/`mono`), `NO_COLOR` → mono, and a
  colour-blind-safe theme.
- Per-scene configuration and a speed multiplier.
- Windows keep-awake (`SetThreadExecutionState`) and Linux (`systemd-inhibit`), if the
  project becomes cross-platform.
- Splitting `SPEC-002` by category crate and generating the scene index.

## Open Questions

- Should the light-background `paper` theme be part of the colour-capability work or a
  separate accessibility review? (Proposed: separate.)
- Should `NO_COLOR` select `mono` automatically, or only via `--color mono`? (Proposed:
  automatic, with one warning line, overridable by an explicit `--color`/`--theme`.)
