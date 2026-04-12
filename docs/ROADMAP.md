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

## M2 — Runtime & UX
- [ ] Auto-cycle with interval + no-repeat + history [RT-R5]
- [ ] Interactive keys n/p/space/r/h/t/q [RT-R6] (q/Esc/Ctrl-C already work)
- [ ] HUD overlay [F4, RND-R4]; flags `--interval --hud`
- [ ] TOML config + precedence + `deny_unknown_fields`; `--config` [CFG-R1..R4]
- [ ] Automated pty smoke test for the binary (replaces manual RND-R5 check)

## M3 — Keep-awake & low-power
- [ ] `KeepAwake` trait + fake; `caffeinate` backend (macOS); `--keep-awake` flag [KA-R2, R3, R5, R6]
- [ ] `mouse` backend behind feature (enigo) [KA-R4]
- [ ] Low-power FPS on idle / focus loss [F5, RT-R8]

## M4 — Batch A: analytic / time-driven scenes (10)
Shared helpers first: orbit/ellipse drawing, disc/ring fill, sine fields.
- [ ] solar_system · [ ] pulsar · [ ] moon_phases · [ ] exoplanet_transit · [ ] satellite_orbit
- [ ] doppler_effect · [ ] wave_interference · [ ] cosmic_microwave_background · [ ] gravitational_lensing · [ ] constellation_map

## M5 — Batch B: particle systems (7)
Shared helpers first: `ParticlePool` (fixed-capacity, respawn), Keplerian speed, gaussian spawn.
- [ ] black_hole · [ ] galaxy_collision · [ ] stellar_nursery · [ ] brownian_motion
- [ ] molecular_dynamics · [ ] particle_accelerator · [ ] rocket_launch

## M6 — Batch C: grid / automata / stochastic (4)
Shared helpers first: `Grid<T>` with resize-rebuild, neighbourhood iteration.
- [ ] crystal_growth · [ ] radioactive_decay · [ ] chemical_reaction · [ ] spacetime_grid

## M7 — Batch D: physics sims & narrative scenes (4)
Shared helpers first: RK4 integrator, scene timeline/state machine.
- [ ] double_pendulum · [ ] mars_rover · [ ] voyager_journey · [ ] moon_landing

## M8 — Parity & cutover
- [ ] All 26 registered; every requirement ID cited by a test [TEST-R1]
- [ ] All benches < 5 ms at 200×50 [GEN-R8]
- [ ] Remove `tools-py/`, `tests/` (Python), Python deps and `run_*` tasks from `pixi.toml`; update the contributor guide, `visuals.md`
- [ ] Open PR `feat/rust-rewrite` → `main`

## M9 — Future (unscheduled)
- `sciart record`, braille hi-res canvas, GitHub Actions + cargo-deny, Windows/Linux keep-awake backends, per-generator config.
