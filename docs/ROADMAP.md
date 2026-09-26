# sciart Roadmap

Spec: [docs/specs/sciart-specification.md](specs/sciart-specification.md) · ADRs: [docs/adr/](adr/)

Each milestone ends green on `pixi run rs-ci` and is one or more commits on `feat/rust-rewrite`.
Tick items as they land. Requirement IDs in brackets link each item to its spec.

## M0 — Scaffold ✅
- [x] Spec interview → master spec, SPEC-001..006, ADR-0001..0008
- [x] Cargo workspace (5 crates), `rust-toolchain.toml`, workspace lints, `clippy.toml` disallowed methods [GEN-R7]
- [x] pixi `rs-*` tasks [ADR-0007]

## M1 — Vertical slice: core + starfield end-to-end ✅
- [x] `Size`, `Glyph`, `GlyphMode`, `Ink`/`Role`, `Canvas` [CORE-R1..R8]
- [x] `FixedStep` [CORE-R9..R12] (+ proptest)
- [x] `Ctx` with seeded ChaCha8 [CORE-R13]; `Generator` trait + `Meta` [CORE-R14..R15]
- [x] `geom` helpers [CORE-R16..R18]
- [x] `Stage`: call protocol + double-buffered frame [CORE-R19..R21]
- [x] Registry + `find` [GEN-R1..R3]
- [x] Contract suite over the registry [GEN-R4..R6, R9, R10]; proptest op sequences; insta snapshots; criterion bench [GEN-R8]
- [x] `starfield` [STAR-A1, STAR-A2]
- [x] Themes `astro`/`rainbow`/`mono` + `paint` into ratatui buffer [RND-R1..R3]; `TerminalGuard` + panic hook [RND-R5]
- [x] `Runtime` with catch_unwind quarantine + panic-hook filter [RT-R1..R4, RT-R7]; "all failed" message row
- [x] CLI `run`, `list`, `sim` (random start; auto-cycle arrives in M2), `--fps --seed --theme --ascii` [CLI-R1..R4]
- [x] `sciart-keepawake::Mode` parse + platform default [KA-R1] (pulled forward from M3)
## M2 — Runtime & UX ✅
- [x] Auto-cycle with interval + no-repeat + history; render-on-switch [RT-R5, RT-R5a, RT-R7]
- [x] Interactive keys n/p/←/→/space/r/h/t/q/Esc/Ctrl-C [RT-R6, RT-R9]
- [x] HUD overlay + FPS meter [F4, RND-R4, RT-R10]; flags `--interval --hud` [CLI-R5]
- [x] TOML config + precedence + `deny_unknown_fields`; `--config` [CFG-R1..R4]; example in `docs/config.example.toml`
- [x] Automated pty end-to-end tests (portable-pty + vt100) replace the manual RND-R5 check

## M3 — Keep-awake & low-power ✅
- [x] `Backend` trait + `Guard` (fakes in tests); `ChildGuard`; `caffeinate` backend (macOS); `--keep-awake` flag + `keep_awake` config key [KA-R2, R3, R5, R6, CLI-R6, CFG-R5]
- [x] `Jiggler<Mover>` logic (always tested) + enigo `Mover` behind feature `mouse`; `pixi run rs-install-mouse` [KA-R4]
- [x] Low-power FPS on idle / focus loss via `Pacer`; `[low_power]` config; focus reporting in `TerminalGuard` [F5, RT-R8, RND-R5]
- [x] `MAX_STEPS` 5 → 15 so low-power frames keep real-time motion (ADR-0003 amended) [RT-R11]
- [x] HUD shows keep-awake status and LOW-POWER [RND-R4]; e2e pty tests for all of the above
- [ ] Manual check (can't be automated without moving the real cursor): `pixi run rs-install-mouse`, then `sciart --keep-awake mouse --hud`, grant Accessibility, confirm the HUD shows `mouse:on` and the cursor moves every 15 s

## M4 — Batch A: analytic / time-driven scenes (10)
Shared helpers first: orbit/ellipse drawing, disc/ring fill, sine fields.
- [x] `sciart_core::draw` helpers (orbit, disc, field, ramp, text_centered, fit_radius) [CORE-R22..R27]
- [x] solar_system · [x] pulsar · [x] moon_phases · [x] exoplanet_transit · [x] satellite_orbit
- [x] doppler_effect · [x] wave_interference · [x] cosmic_microwave_background · [x] gravitational_lensing · [x] constellation_map
- [x] Snapshots recorded and reviewed; all Batch A benches ≤ 0.3 ms at 200×50 [GEN-R8]
- [ ] Follow-up: HUD truncates on narrow terminals with long scene names (flags like `PAUSED` fall off the right edge) — prioritise flags or elide the name [RND-R4]
- [ ] Follow-up: CMB with 6 modes looks stripy; consider more modes / a power-spectrum weighting

## M5 — Batch B: particle systems (7)
Shared helpers first: `ParticlePool` (fixed-capacity, respawn), Keplerian speed, gaussian spawn.
- [x] `sciart_core::particles` — ParticlePool, Trail, gaussian, kepler_*, plot [CORE-R28..R32]
- [x] black_hole · [x] galaxy_collision · [x] stellar_nursery · [x] brownian_motion
- [x] molecular_dynamics · [x] particle_accelerator · [x] rocket_launch
- [x] Snapshots recorded and reviewed; all Batch B benches < 0.5 ms at 200×50 [GEN-R8]
- [ ] Follow-up: MD density (1σ ≈ 1.6 rows) makes the gas phase sparse on small terminals; consider denser packing
- [ ] Follow-up: the pool "no allocation" guarantee (CORE-R28) is by construction only — no counting-allocator test (`unsafe_code = forbid`)

## M6 — Batch C: grid / automata / stochastic (4)
Shared helpers first: `Grid<T>` with resize-rebuild, neighbourhood iteration.
- [x] `sciart_core::grid` — Grid, Hood, Edges, step [CORE-R33..R36]
- [x] crystal_growth · [x] radioactive_decay · [x] chemical_reaction · [x] spacetime_grid
- [x] Snapshots recorded and reviewed; benches < 1 ms at 200×50 [GEN-R8]

## M7 — Batch D: physics sims & narrative scenes (4)
Shared helpers first: RK4 integrator, scene timeline/state machine.
- [x] `sciart_core::{ode, timeline, ease}` — RK4, Timeline, easing [CORE-R37..R39]
- [x] double_pendulum · [x] mars_rover · [x] voyager_journey · [x] moon_landing
- [x] Snapshots recorded and reviewed; benches < 1 ms at 200×50 [GEN-R8]

## M8 — Parity & cutover
- [ ] All 26 registered; every requirement ID cited by a test [TEST-R1]
- [ ] All benches < 5 ms at 200×50 [GEN-R8]
- [ ] Remove `tools-py/`, `tests/` (Python), Python deps and `run_*` tasks from `pixi.toml`; update the contributor guide, `visuals.md`
- [ ] Open PR `feat/rust-rewrite` → `main`

## M9 — Future (unscheduled)
- `sciart record`, braille hi-res canvas, GitHub Actions + cargo-deny, Windows/Linux keep-awake backends, per-generator config.
