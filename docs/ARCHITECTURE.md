# Architecture

How sciart is put together: the crates and what each one owns, the generator contract every
scene follows, the look rules, and which test suite enforces which requirement. For setup,
commands and the contribution workflow, read [CONTRIBUTING](../CONTRIBUTING.md); for the full
contract, the [specifications](specs/) and the [architecture decision records](adr/).

## Overview

sciart is real-time generative science art for the terminal: physics, chemistry, biology,
Earth and space. Its 85 animated scenes are drawn at 30 FPS with ratatui, and it keeps the
machine awake while it runs. It replaced a Python prototype at the M8 cutover.

A frame travels in one direction:

1. The binary's event loop (`app`) translates key, resize and focus events for the pure
   `Runtime` state machine (auto-cycle, history, pause, panic quarantine, run deadline).
2. `Stage` drives the current scene through the generator contract: fixed simulation steps of
   1/60 s (ADR-0003), one `prepare` and one `render` per drawn frame, then the science title.
3. The scene writes glyphs with semantic inks (`Ink { role, level }`, never RGB, ADR-0004) into
   a `Canvas`.
4. `sciart-render` maps inks to colours through the active theme, paints the canvas into a
   ratatui buffer and diffs and encodes only the cells that changed (RND-R22, ADR-0030).

Simulation is deterministic: no wall clock and no global RNG inside scenes, a seeded RNG in
`Ctx`, and trigonometry that does not depend on the optimiser. The same seed and call
sequence give byte-identical frames on a given platform.

| What | Where |
| --- | --- |
| Implementation | `crates/` (the workspace), `web/` and `site/` (the browser player and docs site) |
| Master specification (read first) | [docs/specs/sciart-specification.md](specs/sciart-specification.md) |
| Component specs with requirement IDs | [docs/specs/](specs/) (`SPEC-001-core.md` … `SPEC-009-web-docs.md`) |
| Architecture decisions | [docs/adr/](adr/) (ADR-0001 … ADR-0035) |
| Plan and progress | [docs/ROADMAP.md](ROADMAP.md); open items in [docs/BACKLOG.md](BACKLOG.md) |

## Crate map

```
crates/
  sciart-core/        Size, Canvas, Glyph/GlyphMode, Ink/Role, Ctx (seeded RNG + sim time), FixedStep,
                      Generator trait + Meta/Category, Stage (call protocol, draws the science title),
                      title (Title, Form, fit, span, draw, ascii_of). Helpers for scenes: geom (Camera/Projector,
                      wrap_angle, stroked_line, v3), draw (orbit, disc, disc_covers, field, ramp, text,
                      text_mode, label, label_right, step_glyph), particles (ParticlePool, Trail, gaussian, kepler_*,
                      plot, SpatialHash, InverseCdf), grid (Grid, Hood, Edges, step, lattice_for[_aspect],
                      resample_*, regrid_*, centre_of, SampleMap, laplacian9), ode (rk4_step, integrate,
                      verlet_*, tridiag_solve[_c], C64), timeline (Timeline, Beat, ease), raster (Dots, Pixels,
                      Density, Tone, Glow with splat/splat_bilinear,
                      stronger, dissolve, ascii_ramp_cell, disc_cover/disc_pixels, stipple_keep), stellar (limb_darkening),
                      ink (seq, div, ramp, phase, blackbody, class_kelvin, soft_clip, *_value, level), text (StackText, StackStr),
                      rng (SplitMix64, hash01, cell_key, value_noise), trig (Trig: sin_det/cos_det/sin_cos_det). No I/O.
  sciart-generators/  The registry facade: registry![<scene> in sciart_scenes_<crate>, …] (alphabetical), re-exporting
                      each scene module, plus registry()/find/ALIASES. All scene test suites, benches and examples
                      live here. Depends on core and the five scene crates (ADR-0023).
  sciart-scenes-{cosmos,earth,physics,matter,life}/  One module per scene (src/<name>.rs, `pub mod` in lib.rs), placed
                      by Meta.category: cosmos = Deep Cosmos + Sun & Planets; earth = Earth & Sky + Voyages; physics =
                      Forces & Waves; matter = Reactions + States of Matter; life = the four life categories. A scene
                      never imports another scene (GEN-R16); a model with users in one crate only is a crate-private
                      helper module there (physics: lbm; matter: cahn_hilliard, laplacian_growth), never registered.
                      Depend only on sciart-core, sciart-shared and rand.
  sciart-shared/      Models shared by several scene crates (excitable, plume, skyline, vortex_advect); never
                      registered; depends only on sciart-core and rand. Its `test-support`
                      feature holds helpers the scenes' unit tests use (enabled only via dev-dependencies).
  sciart-render/      Theme (Ink -> RGB: astro (default), rainbow, mono, aurora, colorblind), paint(Canvas -> ratatui Buffer),
                      pipeline (changed cells only: one equality pass, then paint, diff and encode visit just those
                      cells; the full-area path is the tested reference, RND-R22, ADR-0030), backend+encode (diffed
                      terminal writes, RND-R15/R17), Hud (priority fitting), TerminalGuard.
  sciart-keepawake/   Mode + default_for(os, env), Backend trait + Guard, ChildGuard (caffeinate on macOS; a piped
                      systemd-inhibit/elogind-inhibit helper on Linux, ADR-0028), Jiggler<Mover>
                      (pure mouse logic); the enigo Mover only with feature `mouse`.
  sciart-capture/     Headless capture behind the binary's `record` feature and the library's `png` feature
                      (ADR-0025): Canvas -> RGB pixels (braille/half-block exact, other glyphs via font8x8)
                      -> PNG/GIF/asciicast; used by `sciart record`, `Stage::render_png` and the preview
                      example. `record` is on by default (CLI-R15); a `--no-default-features --features cli`
                      binary carries no image dependencies.
  sciart/             Binary + library: the lib target is the 1.0 facade (SPEC-007, ADR-0032: `Stage`, `scenes`/`find`,
                      `SceneInfo`, `Category`, `Theme`, `Options`, `Error`/`ErrorKind`; `SceneWidget` behind `widget`,
                      `render_png` behind `png`, `experimental` behind `unstable-generator`; its public surface is a
                      reviewed snapshot, `tests/lib_surface.rs`). Features: `default = ["cli", "record"]`; the binary
                      needs `cli`. Binary modules: cli (clap), config (TOML + precedence), runtime (pure state machine:
                      auto-cycle, history, pause, panic quarantine, run deadline), keys, fps, pacer, app (event loop),
                      signals (SIGTERM/SIGHUP watcher), hint, child (the `while` command's process and spool),
                      frame (one frame as ANSI text, CLI-R13), record (feature `record`, on by default, CLI-R15).
  sciart-qa/          Unpublished QA crate: the art metrics, their per-scene bands (`tests/art_metrics.rs`) and the
                      `cost` probe behind `rs-cost`.
  sciart-testalloc/   Dev-only counting allocator for allocation tests; the one `unsafe` exception (ADR-0009).
web/                  `sciart-web`, outside the workspace: the scenes for `wasm32-unknown-unknown` as plain exported
                      functions (no terminal, no clock, no OS RNG), wrapped by `site/static/player.js` (ADR-0034).
site/                 Static assets of the docs site; `scripts/build_site.py` writes `site/dist/` from them, the guide,
                      the scene registry, the gallery stills and the showcase manifest.
```

## Dependency direction

Dependencies point strictly downward: the `sciart` crate depends on the generators facade,
render, capture and keep-awake; the facade on the five scene crates; the scene crates on
`sciart-shared` and `sciart-core`; `sciart-capture` on `sciart-render`, and both on
`sciart-core`. `sciart-core` must never gain terminal or OS dependencies, and no new dependency
enters `sciart-core` without an ADR. Scenes import only `sciart_core`, `sciart_shared` and
`rand` (GEN-R16), and never another scene.

## The generator contract

SPEC-002 is the contract; this is its working summary.

```rust
pub trait Generator: Send {
    fn meta(&self) -> &'static Meta;
    fn resize(&mut self, size: Size, ctx: &mut Ctx);  // rebuild size-dependent state here
    fn update(&mut self, dt: f64, ctx: &mut Ctx);     // fixed dt = 1/60 s (ADR-0003)
    fn prepare(&mut self) {} // once per drawn frame, before render: rasterise here (GEN-R20)
    fn render(&self, canvas: &mut Canvas);            // canvas is pre-cleared, correctly sized
    fn caption(&self) -> Option<&'static str> { None } // live act, one of meta().captions
}
```

Rules every generator follows:

- **No panics.** Never panic for any size ≥ 1×1, any resize order, any `dt` in `[0, 0.25]`
  (GEN-R4). Sub-step internally when the physics needs a smaller step; prefer exact or
  closed-form updates.
- **No wall clock, no global RNG.** Use `dt`, the scene's own accumulated time and `ctx.rng`
  (GEN-R7; `clippy.toml` bans `Instant::now`, `SystemTime::now`, `rand::rng`, `rand::random`).
- **Titles and categories.** `Meta.title` is a plain `Phenomenon · mechanism` science title
  (≤ 32 cells, no formulas; GEN-R12) that never claims more than the model does; a scene with
  acts lists them in `Meta.captions` and returns the live one from `caption()`. `Stage` draws
  the title bottom-left (ADR-0014), so scenes put no text in `title::span` and right-align
  bottom-row readouts with `draw::label_right` (GEN-R13). `Meta.category` follows ADR-0013:
  setting first (Voyages → Earth & Sky → Sun & Planets → Deep Cosmos; Earth & Sky includes
  Earth's weather or atmosphere seen from above), otherwise the physics × chemistry × biology
  axes the main model needs (P Forces & Waves, C Reactions, B Living Systems, P×C States of
  Matter, P×B Physics of Life, C×B Molecules of Life, P×C×B Life from Matter).
- **Trigonometry through `Trig`** (`x.sin_det()`, `x.cos_det()`, `x.sin_cos_det()`, in the
  prelude), never `f64::sin`/`cos`/`sin_cos`: LLVM fuses a sin + cos pair into a `sincos` whose
  sine differs in the last bit depending on inlining, so frames differed between dev and
  release (GEN-R6, CORE-R90, ADR-0016; clippy enforces it).
- **Semantic inks.** Emit `Ink { role, level }`, never RGB (ADR-0004).
- **ASCII fallback.** Every Unicode glyph has one: `Glyph::new('★', '*')` (GEN-R10). Wide
  glyphs fall back automatically.
- **Motion and cost.** Visibly animate within 1 simulated second (GEN-R9); stay < 5 ms per
  step + render at 200×50 (GEN-R8). Every worst single frame stays < 2 ms at 200×50 and < 5 ms
  at 500×150; the whole pipeline averages about 0.36 ms and 4.5 KB per frame (M25,
  `rs-cost-release`: 293 µs scene, 43 paint, 23 diff + encode). New or upgraded scenes stay
  < 2 ms worst frame and aim for < 1 ms. Criterion's `frame` group reports means: check act
  hand-overs with `pixi run rs-worst` and add the slow ones to the bench's `worst` table;
  precompute in `resize` or amortise over frames, never in one frame.
- **No allocation in the frame loop.** `render` and steady-state `update` must not allocate
  (GEN-R11): size buffers in `resize`, no `format!`/`Vec` in `render`; write text in pieces or
  into a stack buffer (see `voyager_journey.rs`).
- **Sub-cell rendering.** `raster::Dots` (braille 2×4, line and particle art) or
  `raster::Pixels` (half-block, two coloured square pixels per cell, scene-specific ASCII
  fallback via the `blit` closure) (ADR-0010). Rasterise in `prepare` behind a dirty flag
  (GEN-R20: the runtime takes about 2 steps per drawn frame, so rasterising in `update` wastes
  half the work; frames must not depend on draw cadence, and per-step accumulations stay in
  `update`); `render(&self)` only blits and writes labels.
- **Inks for fields and bodies.** Fields use `Role::Seq`/`Role::Div` (ADR-0011) in flat,
  stepped levels; everything else uses the semantic roles (`Hot`, `Cold`, `Accent`, `Star`,
  `Sun`, `Planet`, `Particle`, `Wave`, `Structure`, `Dim`, `Text`). `Role::Phase`,
  `Role::Blackbody` and the `Role::Ramp(_)` palettes (ADR-0015) stay in the core but scenes
  must not use them (GEN-R23, ADR-0018).
- **Glows and lattices.** For glows with a large dynamic range, accumulate into
  `raster::Density` (braille dots) or `raster::Glow` (one value per pixel) and tone-map in the
  scene through `Glow::values()` + `Tone::level` (asinh LUT built in `resize`, CORE-R81).
  Stateful lattices are capped and resampled on resize via `grid::lattice_for` / `regrid_*` /
  `SampleMap` (ADR-0012).
- **Labels.** `text::StackText` + `draw::text_mode` / `draw::label`.
- **Reuse core helpers.** Before writing a small helper (vector maths, angle wrap, cell → dot
  mapping, ink levels, hashes, stroked lines), check SPEC-001 CORE-R60..R70 and the M12 lifts
  CORE-R84..R96 (`splat_bilinear`, `disc_cover`/`disc_pixels`, `stellar`, `ascii_ramp_cell`,
  `stronger`, `dissolve`, `class_kelvin`): those were lifted out of scenes that each had their
  own copy.
- **Aspect.** Terminal cells are about 2:1 tall: use `geom::polar`/`geom::ASPECT` for round
  shapes, or the square braille sub-cell lattice, as `fractal_growth` does.
- **Narrative scenes** are driven by a looping `Timeline` in simulated seconds, so resizes
  never desynchronise the story; captions are shortened or skipped, never clipped mid-word.

Reference scenes (under `crates/sciart-scenes-*/src/`): `starfield.rs` (minimal),
`solar_system.rs` / `doppler_effect.rs` (house style), `cosmic_microwave_background.rs` (cheap
per-cell fields via per-resize sin/cos tables), `moon_landing.rs` (`Timeline` + `ode`).

## The look

The house look is the **M11 look** (ADR-0018, M15). The 41 scenes that existed at M11 render
as they did then (GEN-R22 pins their ink frames). Every other scene follows M11's idiom
(GEN-R24): a black ground, braille dots for lines, orbits, trails and particles, half-block
pixels with flat, stepped fills for bodies and fields, saturated semantic colours in
complementary pairs (`Hot`/`Cold`, `Accent`), a small subject with much black around it, no
tone-mapped glow washes, halos or gradient backdrops, and M11-style labels. ADR-0017's
clean-minimal, muted rules are superseded.

Two later idioms use the same palette:

- **Thread** (GEN-R25, ADR-0019, after `strange_attractors`): braille threads on black, with
  colour along the thread.
- **Orbital-thread** (GEN-R26, ADR-0020, M17): bodies and media as fixed-threshold probability
  stipple coloured `Hot`/`Cold` by sign, as `electron_orbitals` does (`raster::stipple_keep` /
  `Dots::stipple`, CORE-R107), structure and motion as threads with afterglow. M18 (ADR-0021)
  adds *molecules*: discrete bodies as small Gaussian stipple atoms, `Dots::stipple_puff`
  (CORE-R108), joined by 1-px bond threads only where a physical link exists.

The scenes each style lists follow it. M19/M20 moved `galaxy_collision` (star dust with
afterglow on tidal material only), `solar_system` (speed trails), `brownian_motion` (rainbow
walk thread) and `action_potential` (teardrop neuron with ion dust) into GEN-R25. M21 added ten
scenes: four in GEN-R25 (`magnetosphere`, `gravity_assist`, `snowflake_growth`,
`cloud_chamber`) and six in GEN-R26 (`binary_inspiral`, `chladni_plate`, `rayleigh_benard`,
`mitosis`, `ribosome_translation`, `protocell_assembly`). In stipple, keep peak densities
≈ 0.5 or lower and atoms small with a solid core, or bodies read as solid discs or fuzzy
clouds.

What pins the look:

- the M11 ink frames in `crates/sciart-generators/tests/fixtures/m11_frames.txt` (GEN-R22);
- the M11 palette rule (GEN-R23) and the per-scene art-metric bands (GEN-R24);
- the thread and orbital-thread style suites (GEN-R25, GEN-R26);
- the decisions in ADR-0018, ADR-0019, ADR-0020 and ADR-0021, and each scene's `Look` entry in
  its SPEC-002 section;
- the current frame of every scene in `samples/current/` (the [gallery](GALLERY.md)).

## Test suites: what enforces what

| Suite | Enforces |
|-------|----------|
| `sciart-core/tests/*_spec.rs` | CORE-R* (unit + proptest) |
| `sciart-generators/tests/contract.rs`, `props.rs` | GEN-R1..R10 and GEN-R12 for every registered scene (no panic, determinism, motion, ASCII fallback, valid title) |
| `sciart-generators/tests/snapshots.rs` | Seed-42 80×24 ASCII frames at t = 0/1/5 s for every scene |
| `sciart-generators/tests/source_rules.rs` | GEN-R3 (every scene file in its category's crate is declared and registered), GEN-R16 (scenes import only `sciart_core`, `sciart_shared`, `rand`; checked in sources and manifests), GEN-R7 (no clock / global RNG in core, shared, scenes or facade), GEN-R6/R7 (no crate-level clippy.toml; workspace lints in every scene crate), GEN-R15 (SPEC-002 headings match `Meta.category`), GEN-R6 (`clippy.toml` bans std `sin`/`cos`/`sin_cos`), TEST-R8 (`clippy.toml` bans `Box::leak`, `Vec::leak`, `String::leak`, `mem::forget`, `ManuallyDrop::new`) |
| `*/tests/alloc_free.rs` | CORE-R28/R29/R36 and GEN-R11: pool/trail/grid and every scene's `render`/`update` are allocation-free (TEST-R5) |
| `sciart-generators/tests/leak_free.rs` | Live heap bytes per scene: return to the same value across resize churn (TEST-R6) and stay under `LIVE_CEILING` at 500×150 (TEST-R7; only lower it, exceptions need a spec change) |
| `sciart-qa/tests/art_metrics.rs` | GEN-R24 per-scene bands at 200×50: darkness floor and lit-coverage, chroma-mass and edge-weight ceilings. Re-seed a scene's row with `rs-cost -- <scene> --emit-limits` only after its look is reviewed |
| `sciart-generators/tests/m11_fidelity.rs` | GEN-R22: the 41 M11 scenes (less those GEN-R25/R26 restyle) match M11's ink frames (`tests/fixtures/m11_frames.txt`, glyph + inks, written at M11 by `examples/inkframes.rs`) in ≥ 99.5 % of cells; exceptions need a reason |
| `sciart-generators/tests/thread_style.rs` | GEN-R25: its scene list (`tests/common/gen_r25.rs`, also used by `m11_fidelity` to unpin those M11 scenes) equals SPEC-002's; ≤ 3 % of a listed scene's lit cells are half-block cells at 200×50, t = 5/20 s (`PENDING_GEN_R25` may only shrink) |
| `sciart-generators/tests/orbital_style.rs` | GEN-R26: its scene list (`tests/common/gen_r25.rs`, disjoint from GEN-R25's) equals SPEC-002's; ≤ 3 % half-block cells at 200×50, t = 5/20 s; stipple holds still: ≤ 10 % of cells change per drawn frame over t = 10–12 s |
| `sciart-generators/tests/palette.rs` | GEN-R23: every scene inks only with M11's 14 roles (no `Ramp`, `Blackbody`, `Phase`) |
| `sciart/tests/resource_budget.rs` | RT-R16: gallery mean ≤ 4 700 B and ≤ 480 changed cells per frame at 200×50 through the real pipeline |
| `sciart/tests/pty_smoke.rs` | The real binary in a pty (vt100): rendering, keys, HUD, exit codes, keep-awake cleanup |
| `scripts/spec_check.py` (`pixi run rs-spec-check`, CI) | TEST-R1: every requirement ID in `docs/specs` is cited by a test or checker script or listed in its `MANUAL` table (GEN-R8, TEST-R2, TEST-R3); also rejects citations of undefined IDs. TEST-R4: proptest case caps outside `sciart-core` |
| `scripts/tooling_check.py` (`pixi run rs-spec-check`, CI) | TEST-R9 (`rs-ci` runs the snapshots in the `ci` profile; probes use it, installs keep `release`), TEST-R10 (rs-test = nextest + doctests, nothing narrowed; test profile opt ≥ 2 with checks), TEST-R11 (sccache wrapper via pixi activation) |
| `scripts/release_check.py`, `scripts/release_workflows.py` (`rs-spec-check`, CI, the release workflow) | DIST-R1..R6, R8..R10: versions, tag and changelog, crate metadata and notices, the workflows read as data (gates, permissions, job order, pinned actions, no stored credentials), the conda recipe and the rendered formula. `scripts/release_check_test.py` holds the table of workflow mutations that must fail; `scripts/yaml_subset_test.py` tests the parser they read with |
| `scripts/installer_check.py` (`rs-spec-check`, CI; `pixi run -e dist rs-installer-check` with shellcheck) | DIST-R7: `packaging/install.sh` against a fixture release, tampered checksums, faked platforms and hostile versions, and a table of broken installers that must each fail |
| `scripts/publish_crates_test.py`, `scripts/conda_build_test.py`, `scripts/bump_version_test.py` (`rs-spec-check`, CI) | DIST-R4 (publish order, skipping versions already on the index), DIST-R8 (no build from a tree with uncommitted changes), DIST-R1 (both version streams and every lock file) |

New scenes are picked up by contract, props, snapshots, source rules, alloc, leak and bench
automatically.

## Platforms and environment

- **Platform tiers** (TEST-R13, ADR-0029): tier 1 = macOS arm64 and Linux x86_64 (lint and
  the whole suite); tier 2 = Linux aarch64 (the whole suite); tier 3 = Intel macOS and Linux
  musl (release builds only). Frames are deterministic per platform, not across platforms, so
  they are pinned on the reference platform, macOS arm64, only: the snapshot suite and
  `m11_fidelity` skip themselves elsewhere (`tests/common/tier.rs`). Every workflow job names
  a runner image from `RUNNER_IMAGES` in `scripts/tooling_check.py`. `pixi.toml` covers
  osx-arm64, osx-64, linux-64 and linux-aarch64.
- **Build cache.** Every pixi task routes rustc through `sccache` from the pixi environment
  (`scripts/pixi-activate.sh`, TEST-R11; `sccache --show-stats`). Plain `cargo` outside pixi
  does not use it; `RUSTC_WRAPPER= pixi run …` turns it off. Dev and test builds stay
  incremental, so sccache caches the dependencies and the `ci`/`release` builds; edited
  workspace crates rebuild incrementally. A cached workspace crate is reused only at the same
  path; a checkout at another path reuses only the dependencies.
- **Build profiles.** `rs-sim`, `rs-run`, `rs-preview`, `rs-cost` and `rs-worst` build the `ci`
  profile (opt-level 3, 16 codegen units, no LTO; frames are identical to `release`, GEN-R6);
  only `rs-install` builds `release` (thin LTO, one codegen unit). Benches keep
  `bench`/`release`.
- **Signals.** SIGTERM/SIGHUP restore the terminal from a watcher thread and exit 128+n within
  250 ms (RT-R20); on exit the binary may print a one-line terminal hint (RND-R21).
- **Keep-awake** defaults to `caffeinate` on macOS (ADR-0005) and to `inhibit` on Linux for a
  local session, `off` over SSH (KA-R7/R8, ADR-0028; the real `systemd-inhibit` path is a
  manual check, B-96). The `mouse` backend is an opt-in cargo feature
  (`pixi run rs-install-mouse`) and needs Accessibility permission; verifying it on a real
  desktop is still a manual roadmap item.
- **Low power** (idle or unfocused) lowers the frame rate only; simulation speed is preserved
  by `MAX_STEPS = 15` (ADR-0003).
- **Terminal I/O stays out of `runtime.rs`**: new behaviour goes into the pure `Runtime`
  (unit-tested), with `app.rs` only translating events and drawing.
- **Unsafe code.** The workspace forbids it (`unsafe_code = "forbid"`); the dev-only
  `sciart-testalloc` is the one exception (ADR-0009). Outside the workspace,
  `web/src/wasm.rs` carries the `#[unsafe(no_mangle)]` export attributes and nothing else
  unsafe (ADR-0034).
