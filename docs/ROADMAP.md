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
- [x] Follow-up: HUD truncates on narrow terminals with long scene names — priority fitting keeps flags, elides the name with `~` [RND-R4]
- [x] Follow-up: CMB is now a 96-mode isotropic Gaussian random field with a Planck-like D_ℓ (plateau + acoustic peaks + cell beam) [CMB-A4, CMB-A5]

## M5 — Batch B: particle systems (7)
Shared helpers first: `ParticlePool` (fixed-capacity, respawn), Keplerian speed, gaussian spawn.
- [x] `sciart_core::particles` — ParticlePool, Trail, gaussian, kepler_*, plot [CORE-R28..R32]
- [x] black_hole · [x] galaxy_collision · [x] stellar_nursery · [x] brownian_motion
- [x] molecular_dynamics · [x] particle_accelerator · [x] rocket_launch
- [x] Snapshots recorded and reviewed; all Batch B benches < 0.5 ms at 200×50 [GEN-R8]
- [x] Follow-up: MD density — per-terminal σ scale and N at ρ = 0.25 in the 2-D LJ coexistence dome; 80×24 now 159 atoms with droplets in the cold phase [MD-A5]; Brownian readout label now names its estimator
- [x] Follow-up: pool/trail/grid allocation-freedom and allocation-free `render`/`update` for every scene tested with a counting allocator (`sciart-testalloc`, ADR-0009); fixed the one offender (`moon_phases` label) [CORE-R28, CORE-R29, CORE-R36, GEN-R11]

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
- [x] All 26 registered; every requirement ID cited by a test — 166/169 cited, 3 manual; enforced by `crates/sciart/tests/spec_traceability.rs` [TEST-R1]
- [x] All benches < 5 ms at 200×50 — max 0.55 ms (chemical_reaction) [GEN-R8]
- [x] Remove `tools-py/`, `tests/` (Python), Python deps and `run_*` tasks from `pixi.toml`; update the contributor guide; Python-era docs (`visuals.md`, `specs.md`, improvement notes, issue-01) moved to `docs/archive/`
- [x] Open PR `feat/rust-rewrite` → `main` (draft)

## M10 — Batch E: scientific art (16)
Design: [docs/design/M10-scientific-art-batch.md](design/M10-scientific-art-batch.md) · ADR-0010..0012 · scene specs in SPEC-002 "Batch E" (15 new scenes + the `chemical_reaction` upgrade).
Shared helpers first: layered raster, colormap roles, capped lattices, leapfrog, 3-D camera.
- [x] Core foundation — `Cell.bg`/`put_bg`, `raster::{Dots, Pixels}`, `grid::{resample_bilinear, resample_nearest, lattice_for}`, `ode::{verlet_step, verlet_integrate}`, `geom::Camera`, `Role::{Seq, Div}` + `ink::{seq, div}`, `Category::{Biology, Chemistry}`; themes and `paint` backgrounds; `rs-ci` green [CORE-R40..R48, RND-R6, RND-R7]
- [x] Shared scene helpers — `text::StackText` + `draw::text_mode`, `Camera::projector`, `Dots::line`, `grid::{SampleMap, laplacian9}`, `particles::{SpatialHash, InverseCdf}`, `raster::Density`, `ink::{soft_clip, seq_value, div_value}`, `rng::SplitMix64`, `ode::tridiag_solve`; CORE-R41 Div intensity rule [CORE-R41, CORE-R49..R59]
- [x] Post-M10 audit: duplicated scene code lifted into core — `geom::{v3, wrap_angle, stroked_line}`, `Projector::magnification`, `draw::{disc_covers, label, step_glyph}`, `Dots::{from_cells, cell_mask}`, `Pixels::shade`, `ink::{level, MIN_VISIBLE_LEVEL}`, `grid::{regrid_*, centre_of, lattice_for_aspect}`, `rng::{hash01, cell_key}` + `SplitMix64::mix`; dead `Density::deposit_line` and `SampleMap::{src,dst}_size` removed; raster passes all named `rasterise`; strange_attractors and globular_cluster snapshots re-recorded (half-dot mapping fix, shared dot hash) [CORE-R60..R70]
- [x] `crystal_growth` migrated onto `raster::Dots`, snapshots byte-identical [CRY-A1..A4, CORE-R41]
- [x] Physics fields: vortex_street [VOR-A1..A8] · ising_model [ISING-A1..A6] · strange_attractors [ATT-A1..A8]
- [x] Astronomy: aurora [AUR-A1..A8] · solar_surface [SUN-A1..A7] · three_body [TB-A1..A7] · globular_cluster [GC-A1..A9]
- [x] Chemistry: chemical_reaction upgrade (Pixels + `Seq`, preset walk, capped lattice; moves to `Category::Chemistry`) [CHEM-A1..A8] · belousov_zhabotinsky [BZ-A1..A8]
- [x] Chemistry: electron_orbitals [ORB-A1..A7] · molecule_viewer [MOL-A1..A6] · liesegang_rings [LIES-A1..A8]
- [x] Biology: boids_flocking [BOID-A1..A7] · physarum [PHY-A1..A7] · dna_helix [DNA-A1..A6] · action_potential [AP-A1..A9]
- [x] Snapshots recorded and reviewed for all 16; benches < 1 ms at 200×50 (max 0.47 ms, aurora; hard limit 5 ms) [GEN-R8, GEN-R11]; every new ID cited by a test [TEST-R1]; taste, physics and performance review per scene; SPEC-002 brought in line with the delivered code
- [x] `rs-fmt`/`rs-lint` also run `rustfmt` on each scene file (modules declared inside `registry!` are invisible to `cargo fmt`)
- [x] Review follow-ups: solar_surface spot granulation gated out of the penumbra + umbra pixel floor, aurora patch floor and ridge glow, physarum later-bloom exposure, vortex_street start-up specks, mono-theme `Seq`/`Div` floors (ADR-0011, RND-R6), liesegang ASCII haze class, three_body orbit fade-in (triangle included), pinning tests (ATT rates, ORB dipole table, GC reseed buffers); all 16 scenes polished over a second art/science/Rust review round, snapshots re-recorded and reviewed, max bench unchanged (aurora 0.47 ms) [AUR-A8, LIES-A8, AP-A9]
- [ ] Second-round review follow-ups (minor, non-blocking): aurora red-crown ramp, thinner spruce ridge, gentler lake ripple, `Glow::red` doc and `ASCII_LAKE_BRIGHT` const, top-fade spec wording · solar_surface 2 px polar margin at small sizes, ASCII loop glyphs by pixel pair, interlaced-granulation spec note, `SpatialHash::reserve` (CORE-R54 amendment) to replace the dummy rebuild · chemical_reaction ASCII `HALO_DOT` 13→20 · electron_orbitals fade window on the framing weight, shorter label outside the beat · ising_model ASCII polarity follows the majority · liesegang_rings legend leaves with the first ratio · three_body `MOTH_FULL_SCALE`, triangle clear of adjacent trail cells · dna_helix zoom-aware rung stride · molecule_viewer rear-bond dotting threshold, tilted planar molecules · physarum third inoculum blur pass · action_potential dendrite spacing and deterministic third primary · belousov_zhabotinsky free-end spacing in `plan_seeds`
- [ ] Follow-up proposals (unscheduled): `sciart record` (GIF/asciicast export for taste review); GitHub Actions CI (+ cargo-deny); `Role::Spectrum` for true spectral line colour

## M11 — Science titles, P×C×B taxonomy, leak guards
Design: [docs/design/M11-titles-taxonomy.md](design/M11-titles-taxonomy.md) §1–§4 · ADR-0013, ADR-0014 · branch `feat/m11-sciart-titles`.
- [x] `title` module (split, charset, fit, span, two-tone draw with the 8–10 s dim), `Meta.title`/`captions`, `Generator::caption`, `Stage::title`/`set_title_visible` (hidden until the snapshots are re-recorded), `draw::label_right` [CORE-R71..R76]
- [x] `Category` rebuilt: split sky + Voyages + the P×C×B lattice (11 categories), every scene re-filed; `list` prints category ids [CORE-R77, CLI-R4]
- [x] Titles for all 41 scenes (design §3; brownian_motion from the curator proposal); strange_attractors names the shown attractor through `caption()` [GEN-R12]
- [x] Title contract over the registry; contract and alloc suites draw the title [GEN-R12, GEN-R11]
- [ ] Renames `chemical_reaction` → `gray_scott`, `crystal_growth` → `fractal_growth`, old names kept as aliases [GEN-R2, GEN-R14]
- [ ] SPEC-002 scene headings carry each scene's category label [GEN-R15]
- [ ] Bottom-left kept free of scene text: readout moves to `draw::label_right`, label clamps [GEN-R13]
- [ ] Runtime: title visible iff setting on and HUD off, category-interleaved cycle order [RT-R5, RT-R12, RT-R13]
- [ ] HUD shows the title; rainbow text is grey [RND-R8, RND-R9]
- [ ] `title` config key, `--no-title`, retired names in `disabled` [CFG-R6, CFG-R7, CLI-R7]
- [ ] Leak guards: live-bytes churn and ceiling, clippy bans on `Box::leak`/`mem::forget` [TEST-R6..R8]
- [ ] Titles on by default; all snapshots re-recorded and reviewed [CORE-R75, TEST-R2]

## M9 — Future (unscheduled)
- `sciart record`, GitHub Actions + cargo-deny, Windows/Linux keep-awake backends, per-generator config (the braille hi-res canvas landed in M10 as `raster::Dots`).
