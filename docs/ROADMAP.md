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
- [ ] Second-round review follow-ups (minor, non-blocking): aurora red-crown ramp, thinner spruce ridge, gentler lake ripple, `Glow::red` doc and `ASCII_LAKE_BRIGHT` const, top-fade spec wording · solar_surface 2 px polar margin at small sizes, ASCII loop glyphs by pixel pair, interlaced-granulation spec note, `SpatialHash::reserve` (CORE-R54 amendment) to replace the dummy rebuild · gray_scott ASCII `HALO_DOT` 13→20 · electron_orbitals fade window on the framing weight, shorter label outside the beat · ising_model ASCII polarity follows the majority · liesegang_rings legend leaves with the first ratio · three_body `MOTH_FULL_SCALE`, triangle clear of adjacent trail cells · dna_helix zoom-aware rung stride · molecule_viewer rear-bond dotting threshold, tilted planar molecules · physarum third inoculum blur pass · action_potential dendrite spacing and deterministic third primary · belousov_zhabotinsky free-end spacing in `plan_seeds`
- [ ] Follow-up proposals (unscheduled): `sciart record` (GIF/asciicast export for taste review); GitHub Actions CI (+ cargo-deny); `Role::Spectrum` for true spectral line colour

## M11 — Science titles, P×C×B taxonomy, leak guards
Design: [docs/design/M11-titles-taxonomy.md](design/M11-titles-taxonomy.md) §1–§4 · ADR-0013, ADR-0014 · branch `feat/m11-sciart-titles`.
- [x] `title` module (split, charset, fit, span, two-tone draw with the 8–10 s dim), `Meta.title`/`captions`, `Generator::caption`, `Stage::title`/`set_title_visible`, `draw::label_right` [CORE-R71..R76]
- [x] `Category` rebuilt: split sky + Voyages + the P×C×B lattice (11 categories), every scene re-filed; `list` prints category ids [CORE-R77, CLI-R4]
- [x] Titles for all 41 scenes (design §3; brownian_motion from the curator proposal); strange_attractors names the shown attractor through `caption()` [GEN-R12]
- [x] Title contract over the registry; contract and alloc suites draw the title [GEN-R12, GEN-R11]
- [x] Renames `chemical_reaction` → `gray_scott`, `crystal_growth` → `fractal_growth`, old names kept as aliases [GEN-R2, GEN-R14]
- [x] SPEC-002 scene headings carry each scene's category label [GEN-R15]
- [x] Bottom-left kept free of scene text: readout moves to `draw::label_right`, label clamps [GEN-R13]
- [x] Runtime: title visible iff setting on and HUD off, category-interleaved cycle order [RT-R5, RT-R12, RT-R13]
- [x] HUD shows the title; rainbow text is grey [RND-R8, RND-R9]
- [x] `title` config key, `--no-title`, retired names in `disabled` [CFG-R6, CFG-R7, CLI-R7]
- [x] Leak guards: live-bytes churn and ceiling, clippy bans on `Box::leak`/`mem::forget` [TEST-R6..R8]
- [x] Titles on by default; all snapshots re-recorded and reviewed [CORE-R75, TEST-R2]
- [x] Review round 1: 4-cell title–readout gap, fade restarts per caption, HUD title in the canvas title's columns, setting precedence and narrowed C/P clauses (brownian_motion → Forces & Waves, ising_model → States of Matter), category ids spelled from labels, title wording, per-scene TEST-R7 ceiling, wider leak bans [CORE-R71, CORE-R74..R77, GEN-R13, RND-R9, TEST-R7, TEST-R8]
- [x] Review round 2: gauges and readouts clear of the title, per-act captions (moon_landing, strange_attractors), title wording, 2-cycle warm-up + 8 checked cycles in the churn guard [CORE-R76, GEN-R12, GEN-R13, TEST-R2, TEST-R6]
- [x] the contributor guide updated (taxonomy, titles in "Adding a generator", leak-guard suites); benches at 200×50 max 0.46 ms (aurora; title drawing included) [GEN-R8]
- [x] Review round 3 follow-ups (done in M12; readouts and the dimmed title share `title::READOUT_LEVEL` 90; wave_interference keeps 3 blank cells, WAVE-A3) [CORE-R74, CORE-R76, GEN-R12, GEN-R13, TEST-R2, TEST-R6, TEST-R8]:
  - Titles (update design §3 and re-record snapshots): molecular_dynamics → "Condensation · cool atoms cling" (the cycle both melts and condenses, mostly below the 2-D triple point) · solar_surface → "Solar surface · convection cells" (not boiling) · constellation_map → "Constellations · star patterns" (Crux is not Ptolemaic) · action_potential → "Nerve spike · ion channels open" · spacetime_grid → "Spacetime waves · orbits shrink" · moon_phases → "Moon phases · half always sunlit" · optional boids_flocking → "Murmuration · each bird tracks 7"
  - Title-row readouts sit at `title::READOUT_LEVEL` 110, brighter than the faded title (90): set it to `title::DIM_LEVEL`, or reword CORE-R76 and the constant's doc [CORE-R76]
  - Act name shown twice once the caption fits: three_body's top label keeps only the attribution and molecule_viewer's only `CH4  16.04 g/mol` when `Title::fit` gives `Form::Full` (both variants built in `resize`)
  - wave_interference: the field's lowest `.` level starts one pad cell after the title and reads as an ellipsis; leave the 3 cells after `title::span(..).end` blank on row h−1, or note it in SPEC-002
  - SPEC-002 solar_surface Label still says "(`Text` 150)"; it is `Text` at `title::READOUT_LEVEL` (CORE-R76)
  - `crates/sciart/src/app.rs`: the file-level `#![allow(clippy::disallowed_methods)]` also exempts the event loop from the TEST-R8 leak bans; move the allow onto the three `Instant::now` call sites
  - `leak_free.rs`: one-line note at the TEST-R6 assertion that live bytes see capacity, not length (growth inside a `Vec`'s capacity is invisible until it doubles)

## M12 — Scene upgrades (planned)
Design: [docs/design/M11-titles-taxonomy.md](design/M11-titles-taxonomy.md) §5, §7 · branch `feat/m12-upgrades`. Research: `docs/design/M11-research/curator-proposal.md` §4, `cross-domain-taxonomy.md` §5.
- [x] Ink ADR-0015: `Role::Phase` (cyclic), `Role::Blackbody` (temperature in `level`), `Role::Ramp(Ramp)` with six named ramps (Ember, Ice, Ferroin, HAlpha, Aurora, Plasma) mapped by every theme; Mono keeps its floors [CORE-R78..R80, RND-R10..R12]
- [x] `raster::Tone` (asinh LUT, built on resize) + `Density::to_pixels` [CORE-R81]
- [ ] Upgrade all 41 scenes from the per-scene lists (art first, honest science, stylisation stated in SPEC-002); rescore each in truecolor frames before and after
- [x] doppler_effect: two acts, acoustic with Mach cone and relativistic light with red/blue shift, each with its own caption [DOP-A1..A10]
- [ ] particle_accelerator: two acts, LHC-style event display and 1960s bubble chamber, each with its own caption
- [x] States of Matter group: molecular_dynamics (blackbody balls, ice streaks, MB inset), fractal_growth (frost + cathode forest, no halo), ising_model (Kawasaki spinodal act, chamfer-lit domains, thermometer), molecule_viewer (per-pixel z-buffered ball-and-stick with bond orders) [MD-A5..A9, CRY-A1..A8, ISING-A6..A9, MOL-A3..A7]
- [ ] Matter review follow-ups (minor): ising_model one chamfer-depth curve for both spins with a darker, calmer crimson interior · molecular_dynamics streak cap scaled by the atom's pixel radius at 80×24 · fractal_growth pre-grown opening seed and a frost tip age gradient · shared `geom::LIGHT` + `geom::lambert` for molecular_dynamics and molecule_viewer (new CORE-R ID) · re-measure ising_model/200x50 on a quiet machine (1.27 ms worst at load ≈ 30)
- [ ] Every upgraded scene < 2 ms worst frame (step + render) at 200×50, allocation-free `render`/steady-state `update` [GEN-R8, GEN-R11]
- [x] Cap fractal_growth's DLA lattice with `grid::lattice_for` (ADR-0012) and drop its TEST-R7 exception (14.5 → 2.7 MiB at 500×150) [CRY-A5, TEST-R7]
- [x] radioactive_decay: size the nucleus lattice to rows `0..h − 1`, so the title is not punched through its last row [RAD-A4]
- [x] Title-row readout polish [GEN-R13]: brownian_motion `D fit 4.63  true 2.25 rows^2/s`; double_pendulum long form padded at the front, not inside (`{growth:.1}x`, form chosen by its maximum width)
- [ ] Review (art · science · Rust/perf) with fix loops; local `--no-ff` merge into `main`

## M13 — New scenes (planned)
Design: [docs/design/M11-titles-taxonomy.md](design/M11-titles-taxonomy.md) §6, §7 · branch `feat/m13-new-scenes` (may split per category group).
- [ ] Curator + critic pass over `M11-research/` plus a fresh astronomy research pass; confirm the picks, including Reactions and the 12 extra sky scenes
- [ ] Shared `pub(crate)` models in sciart-generators: excitable medium (BZ, heart, dicty), Laplacian growth (lichtenberg, fractal_growth), Cahn–Hilliard (spinodal, protocell); lift `lbm` into core only with kelvin_helmholtz
- [ ] Deep Cosmos: spiral_galaxy, cosmic_web + 4 astronomy picks
- [ ] Sun & Planets: saturn_rings, comet_tails + 4 (jupiter_storms, solar_eruption, …)
- [ ] Earth & Sky: total_solar_eclipse, rainbow + 4 (hurricane, meteor shower, …)
- [ ] Voyages: jwst_unfolding, orbital_rendezvous
- [ ] Forces & Waves: kelvin_helmholtz, quantum_double_slit
- [ ] Reactions: 2 C-only scenes chosen by the curator
- [ ] Living Systems: lenia, firefly_sync
- [ ] States of Matter: spinodal_decomposition, lichtenberg_discharge
- [ ] Physics of Life: heart_fibrillation, motility_clustering
- [ ] Molecules of Life: dicty_aggregation, seashell_pigmentation
- [ ] Life from Matter: chemical_garden, molecular_motors
- [ ] Every new scene: SPEC-002 section, title, category by ADR-0013, < 2 ms worst frame at 200×50, leak guards green; review; local `--no-ff` merge into `main`

## M9 — Future (unscheduled)
- `sciart record`, GitHub Actions + cargo-deny, Windows/Linux keep-awake backends, per-generator config (the braille hi-res canvas landed in M10 as `raster::Dots`).
