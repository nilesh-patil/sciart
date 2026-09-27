# Project Specification: sciart

**Status**: Approved (spec interview, 2026-09-26)
**Branch**: `feat/rust-rewrite`
**Supersedes**: the Python prototype (removed at the M8 cutover; its design notes are archived in `docs/archive/python-prototype-spec.md`)

## Executive Summary

`sciart` is a Rust rewrite of the generative ASCII science/astronomy art project. The
Python version renders a single static frame every 15 seconds; `sciart` renders
**real-time animations at 30 FPS** in the terminal using ratatui + crossterm, while
keeping the machine awake (macOS `caffeinate` by default, optional mouse jiggle).

The rewrite is organized as a Cargo workspace with a small, testable core (canvas,
generator trait, fixed-step clock, seeded RNG), a crate holding all 26 generators, a
renderer with semantic color palettes, a keep-awake crate, and a thin CLI binary.
Development follows **SDD** (numbered requirements in `docs/specs/SPEC-*.md`, decisions
in `docs/adr/`) and **TDD** (tests cite requirement IDs; contract, property, snapshot and
benchmark layers).

## Project Goals & Success Criteria

| Goal | Success criterion |
|------|-------------------|
| Smooth animation | Every generator animates at 30 FPS; update+render < 5 ms at 200×50 (release, M-series Mac) — enforced by criterion benches |
| Stability | No generator panics for any `width, height >= 1`, any resize/dt sequence (contract + proptest). Runtime survives any generator panic via quarantine |
| Modularity | Generators depend only on `sciart-core`; they are testable without a terminal |
| Determinism | `--seed N` reproduces a run; snapshot tests are byte-stable |
| Parity of scope | All 26 Python scenes exist in Rust (reimagined for motion) before Python is removed |
| Traceability | Each requirement ID (e.g. `CORE-R3`) is referenced by at least one test |

## Technical Architecture

### Technology Stack

| Concern | Choice | ADR |
|---------|--------|-----|
| Language | Rust, edition 2024, MSRV 1.88 | — |
| Structure | Cargo workspace, 5 crates | [ADR-0001](../adr/0001-cargo-workspace.md) |
| Terminal | ratatui + crossterm (alt screen, raw mode, truecolor) | [ADR-0002](../adr/0002-ratatui-crossterm.md) |
| Time | Fixed-step `update(dt)` with accumulator, dt = 1/60 s | [ADR-0003](../adr/0003-fixed-step-simulation.md) |
| Color | Semantic inks mapped by themes; generators never pick RGB | [ADR-0004](../adr/0004-semantic-palettes.md) |
| Keep-awake | `caffeinate` (default) \| `mouse` (enigo, cargo feature) \| `off` | [ADR-0005](../adr/0005-keep-awake-backends.md) |
| Fault isolation | `catch_unwind` + per-session quarantine | [ADR-0006](../adr/0006-panic-quarantine.md) |
| Task runner | pixi tasks wrapping cargo (no hosted CI yet) | [ADR-0007](../adr/0007-pixi-task-runner.md) |
| RNG | Injected, seedable `rand_chacha::ChaCha8Rng`; no global RNG in generators | [ADR-0008](../adr/0008-injected-rng.md) |
| Categories (M11) | Sky settings + physics × chemistry × biology lattice, 11 categories | [ADR-0013](../adr/0013-pxcxb-taxonomy.md) |
| Titles (M11) | One science title per scene, drawn by `Stage` | [ADR-0014](../adr/0014-central-science-titles.md) |
| Field colour (M12) | Phase, blackbody and six named ramps as semantic roles | [ADR-0015](../adr/0015-phase-blackbody-named-ramps.md) |
| CLI / config | clap (derive) + TOML file, flags override file | [SPEC-005](SPEC-005-cli-config.md) |
| Tests | built-in test + proptest + insta + criterion | [SPEC-006](SPEC-006-testing.md) |

### System Architecture

```
                  ┌──────────────────────── sciart (bin, crates/sciart) ───────────────────────┐
                  │ cli (clap) → config (TOML merge) → Runtime (state machine) → event loop    │
                  └──────┬───────────────────────┬────────────────────────┬────────────────────┘
                         │                       │                        │
          ┌──────────────▼─────────┐   ┌─────────▼──────────┐   ┌─────────▼──────────┐
          │ sciart-generators      │   │ sciart-render      │   │ sciart-keepawake   │
          │ registry + 26 scenes   │   │ themes, HUD,       │   │ caffeinate / mouse │
          │                        │   │ Canvas→ratatui     │   │ / off              │
          └──────────────┬─────────┘   └─────────┬──────────┘   └────────────────────┘
                         │                       │
                  ┌──────▼───────────────────────▼──────┐
                  │ sciart-core                          │
                  │ Size, Canvas, Glyph, Ink, Generator, │
                  │ Ctx(rng,time), FixedStep, geom       │
                  └──────────────────────────────────────┘
```

Dependency rule: arrows point downward only. `sciart-core` has no terminal, OS, or
I/O dependencies. `sciart-generators` depends only on `sciart-core`.

Frame loop (per iteration of the binary's event loop):

1. Drain crossterm events → `Runtime::handle(Event)` (keys, resize).
2. `steps = FixedStep::advance(real_elapsed)`; call `generator.update(dt, ctx)` `steps`
   times (clamped to `max_steps` to avoid a spiral of death).
3. `generator.render(&mut canvas)` inside `catch_unwind`.
4. `sciart-render` maps the canvas through the active theme into the ratatui buffer,
   draws the HUD if enabled, and flushes (ratatui diffs cells).
5. Sleep until the next frame deadline (30 FPS, or low-power FPS).

### API Design (core contract)

```rust
pub trait Generator: Send {
    fn meta(&self) -> &'static Meta;                 // name, category, description
    fn resize(&mut self, size: Size, ctx: &mut Ctx);  // always called before first update
    fn update(&mut self, dt: f64, ctx: &mut Ctx);     // fixed dt, seconds
    fn render(&self, canvas: &mut Canvas);            // canvas already sized & cleared
}
```

Full requirements: [SPEC-001 core](SPEC-001-core.md), [SPEC-002 generators](SPEC-002-generators.md).

### Data Model

- `Size { width: u16, height: u16 }` — both clamped to `>= 1` at construction.
- `Cell { ch: char, ink: Ink }`; `Canvas { size, glyph_mode, cells: Vec<Cell> }` row-major.
- `Glyph { unicode: char, ascii: char }` — canvas stores whichever the `GlyphMode` selects;
  a unicode glyph that is not single-width falls back to `ascii`.
- `Ink { role: Role, level: u8 }` — semantic role (Star, Planet, Particle, Wave, Accent,
  Text, Hot, Cold, Dim, …) and intensity 0..=255.
- `Theme` — `fn color(&self, ink: Ink, x: u16, y: u16) -> Rgb`; built-ins `astro`
  (default), `rainbow` (stable per-cell hue), `mono`.
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
- After `low_power.idle_after` (default 10 min) without a key press, drop to
  `low_power.fps` (default 5). Any key restores full FPS. Simulation time still advances
  at the fixed dt, so motion stays physically consistent (fewer rendered frames, same speed).

### F6: Keep-awake
- `--keep-awake caffeinate|mouse|off`, default `caffeinate` on macOS, `off` elsewhere.
- Failures never stop the art (logged in HUD); see SPEC-004.

### F7: Fault isolation
- Generator panics are caught; the generator is quarantined for the session, the last good
  frame is kept, and in `sim` mode the runtime advances to the next generator. In `run`
  mode a panic shows an error message and waits for `q` / `r` (reset).

### F8: The 26 scenes
Reimagined for continuous animation (Python output is inspiration, not oracle). Ported in
technique batches — see [ROADMAP](../ROADMAP.md). Each scene: seeded, resize-safe,
< 5 ms/frame, snapshot-tested.

### F9: Science titles (M11)
- Every scene shows a short `Phenomenon · mechanism` title at the bottom-left, fitted to the width and
  dimmed after 8 s; it moves into the HUD while the HUD is on. `title = false` or `--no-title` hides it.
  See ADR-0014, CORE-R71..R76, GEN-R12/R13, RT-R12, RND-R8/R9, CFG-R6, CLI-R7.

## UI/UX Specifications

- Alternate screen + raw mode + hidden cursor; always restored on exit **and** on panic
  (panic hook restores the terminal before printing).
- Unicode glyphs by default (`★ ✦ · ░ ▒ ▓ ☉ ● ◐`); `--ascii` forces pure ASCII.
- Terminal aspect ratio: cells are ~2:1 tall; `core::geom::ASPECT = 2.0` scales x for round shapes.
- No per-frame color randomization (no strobing).

## Performance Requirements

- Target 30 FPS (configurable `--fps 1..=120`), budget < 5 ms update+render at 200×50.
- No per-frame heap allocation in steady state for the canvas (reused buffer).
- `max_steps` per frame = 5 (drop simulation time rather than stall).

## Testing Strategy

See [SPEC-006](SPEC-006-testing.md). Four layers: contract (every registered generator),
property (proptest over sizes/resizes/dt), snapshot (insta, seeded), benchmarks
(criterion). TDD: a failing test that cites a requirement ID precedes each implementation.

## DevOps & Deployment

- pixi tasks: `rs-build`, `rs-test`, `rs-lint` (fmt --check + clippy -D warnings),
  `rs-fmt`, `rs-bench`, `rs-ci` (fmt + clippy + test), `rs-run`, `rs-sim`.
- Platforms: macOS arm64 primary; Linux must build and pass tests.
- Install: `cargo install --path crates/sciart`.

## Error Handling & Edge Cases

| Case | Behavior |
|------|----------|
| Terminal 1×1 or 0×0 reported | Size clamps to ≥1; generators must still render |
| Resize mid-scene | `resize()` is called; generator reinitializes dimension-dependent state |
| Long stall (laptop sleep) | FixedStep clamps to `max_steps`; accumulator is reset |
| Generator panic | Quarantine (F7) |
| All generators quarantined | Show error screen; keep running until quit (keep-awake stays active) |
| `caffeinate` missing / fails | Log in HUD, continue without keep-awake |
| Mouse backend lacks accessibility permission | Log, continue |
| Invalid config file | Exit code 2 with file path + line/col of the TOML error |
| Wide / zero-width unicode glyph | Canvas substitutes the ASCII fallback |

## Constraints & Assumptions

- The Python implementation was kept until M8 and then removed (M8 cutover).
- No hosted CI; pixi `rs-ci` is the gate (run before every commit).
- Generator count and categories match the registry (26 incl. starfield).

## Future Considerations

- `sciart record <name> --frames N` to dump frames (README GIFs).
- Braille 2×4 sub-cell "hi-res" canvas for smooth curves.
- Hosted CI (GitHub Actions) mirroring `rs-ci`; `cargo-deny`.
- Windows support; per-OS keep-awake backends (`SetThreadExecutionState`, systemd-inhibit).
- Per-generator parameters in config.

## Open Questions

- Should the `mouse` backend be compiled in by default (feature on) or opt-in? (Proposed: opt-in feature `mouse`.)
- Low-power detection of *terminal focus loss* (crossterm `FocusLost`) — include in F5 or keep idle-timer only? (Proposed: both, focus events where the terminal supports them.)
