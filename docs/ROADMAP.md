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

## M12 — Scene upgrades (done)
Design: [docs/design/M11-titles-taxonomy.md](design/M11-titles-taxonomy.md) §5, §7 · branch `feat/m12-upgrades`. Research: `docs/design/M11-research/curator-proposal.md` §4, `cross-domain-taxonomy.md` §5.
- [x] Ink ADR-0015: `Role::Phase` (cyclic), `Role::Blackbody` (temperature in `level`), `Role::Ramp(Ramp)` with named ramps mapped by every theme (six at first: Ember, Ice, Ferroin, HAlpha, Aurora, Plasma; M12 later added Starlight, Sulfur, Cobalt, Eosin); Mono keeps its floors [CORE-R78..R80, RND-R10..R12]
- [x] `raster::Tone` (asinh LUT, built on resize) + `Density::to_pixels` [CORE-R81]
- [x] Upgrade all 41 scenes from the per-scene lists (art first, honest science, stylisation stated in SPEC-002) in ten groups, each reviewed in truecolor frames; gallery rescore after: every scene 6–8/10
- [x] doppler_effect: two acts, acoustic with Mach cone and relativistic light with red/blue shift, each with its own caption [DOP-A1..A10]
- [x] particle_accelerator: two acts, LHC-style event display and 1960s bubble chamber, each with its own caption [ACC-A4..A12]
- [x] Reactions group: gray_scott (`Ramp(Aurora)` with a live 97th-percentile exposure, regime captions, Pearson readout; inset dropped), belousov_zhabotinsky (`Ramp(Ferroin)` through a phase curve, stylised ferroin/Ru hybrid stated), liesegang_rings (second act: Liesegang's 1896 silver-drop dish, `Ramp(Ember)`, part-grown openings) [CHEM-A4, CHEM-A7..A9, BZ-A7..A9, LIES-A1, LIES-A6..A13]
- [x] Waves group: wave_interference (`Ramp(Ice)` ripple tank, sweeping separation, detector screen), vortex_street (age-graded `Ramp(Plasma)` dye streaklines via `raster::half_block`); `ParticlePool::default()` allocation-free [WAVE-A1..A7, VOR-A9..A12, CORE-R28, CORE-R42]
- [x] Sun & Sky group: solar_system (Wien limb, night floor, ring occlusion), solar_surface (slit jaws, thin S filament, Doppler hysteresis), exoplanet_transit (anti-aliased limb), aurora (box-filtered lake, crimson crown), constellation_map (braille Milky Way with star clouds, Great Rift, Coalsack), moon_phases (lit floor 60, earthshine ≤ 30); mono maps `Blackbody` to its astro lightness [SOL-A1..A10, SUN-A4, SUN-A8, SUN-A9, TRAN-A1..A9, AUR-A7, AUR-A9, CON-A1..A9, MOON-A1..A9, RND-R11, RND-R12]
- [x] States of Matter group: molecular_dynamics (blackbody balls, ice streaks, MB inset), fractal_growth (frost + cathode forest, no halo), ising_model (Kawasaki spinodal act, chamfer-lit domains, thermometer), molecule_viewer (per-pixel z-buffered ball-and-stick with bond orders) [MD-A5..A9, CRY-A1..A8, ISING-A6..A9, MOL-A3..A7]
- [ ] Matter review follow-ups (minor): ising_model one chamfer-depth curve for both spins with a darker, calmer crimson interior · molecular_dynamics streak cap scaled by the atom's pixel radius at 80×24 · fractal_growth pre-grown opening seed and a frost tip age gradient · shared `geom::LIGHT` + `geom::lambert` for molecular_dynamics and molecule_viewer (new CORE-R ID)
- [x] Dynamics group: double_pendulum (white merge that splits into Aurora/Hα twins, 96-ghost material line), strange_attractors (Chen added, one palette per attractor, wings-open yaw at 268° with no camera whip), three_body (per-body ramps, comet tails, fade-in from 0.35), brownian_motion (dark-field grain sphere, OU rms ring, 22 s ember trail, `D 1-path` readout); `Dots::disc`; worst frames 0.10 / 0.39 / 0.12 / 0.11 ms at 200×50 [DP-A5..A8, ATT-A9, ATT-A10, TB-A7, TB-A8, BRO-A4..A7, CORE-R82]
- [ ] Dynamics review follow-ups (minor): lift the Lambert sphere shading into core when a fourth scene needs it · snapshots pass only in the debug profile (`molecular_dynamics_t5` differs in release)
- [x] Life group: action_potential (voltage on the `Div` map, gold spike with a quiet blue wake, Na⁺/K⁺ ion streaks, amber scope, beat captions), dna_helix (Ice/Hα backbones as the hero, Aurora/Plasma rungs with near `▐` slabs, `Hot` DnaB-style helicase, view-driven captions), boids_flocking (dusk `Seq` sky, dark birds shaded by density, reed-bank silhouette, pale-front/dark-band escape wave, pale `Ice` falcon, 12 s attack cycle, wave-following captions), physarum (`Ramp(Ember)` tubes with an oxblood halo, stylised streaming, oat `Blackbody` flakes, captions); `title::stable_readout_room` and `Title::full_fits_from`; worst frames 0.10 / 0.14 / 0.87 / 0.35 ms at 200×50 [AP-A10..A12, DNA-A7, DNA-A8, BOID-A8..A10, PHY-A8..A10, CORE-R73, CORE-R83]
- [ ] Life review follow-ups (minor): physarum thinner channels (needs a raised lattice cap or braille raster) and a pulse when food is reached · action_potential longer/wider ion streaks at ≥ 120 columns · dna_helix helicase rotation tick · boids_flocking braille reed stems, the Vicsek travelling-band act, and a dot→cell helper in core once a second scene needs it
- [x] Quanta group: electron_orbitals (bounded colour gauge where only the beat phase ωt is physical, hold-only `Ramp(Ice)` exposure glow from r₅₀ to r₉₀ under the phase dots), particle_accelerator (collider event display and bubble chamber acts, asinh-toned collider glow, muon wheels), radioactive_decay (half-block Ember/Aurora lattice, uniformly sampled α/β tracks, two-dot measured curves over Bateman); `Pixels::cell_bg`, `Dots::blit_over` and `raster::Glow`; worst frames 0.63 / 0.24 / 0.30 ms at 200×50 [ORB-A8, ORB-A9, ACC-A4..A12, RAD-A5..A8, CORE-R84, CORE-R85]
- [ ] Quanta review follow-ups (minor): mono `Role::Phase` floor near L* 40 so phase-0 dots stay visible (electron_orbitals transitions, solar_surface) [RND-R10] · radioactive_decay busy mono lattice at t ≈ 5 s · electron_orbitals glow outline steps per cell under dots · merge `Pixels::cell_bg` with the proposed `Pixels::backdrop` when that lands
- [x] Cosmos A group: black_hole (Binet-traced Luminet disk with order-1 lensing ring and lower arc, sheared log-normal clumps tapered at the rim, continuous ember envelope, one-pixel point stars that never drop out between texels, no haze), gravitational_lensing (compact gold-cored lens with a dark gap to the critical curve, Bayer halftone edge), spacetime_grid (`Seq` mesh whose crests and burst energy density light up the sheet, local rim cap, halo rings, merger and ringdown captions); worst frames 0.31 / 0.43 / 0.34 ms at 200×50 [BH-A1..A9, LENS-A1..A8, SPT-A1..A10]
- [ ] Cosmos A review follow-ups: spacetime_grid is still a mostly quiet purple lattice between bursts, and the reviewer's "merger + 1 s ≥ 1.4 × inspiral" became a 3 s window in SPT-A10 because the burst shell covers only ~5 % of the sheet at +1 s (art, major) · black_hole mid-disk stays bronze below 6 500 K in the `Blackbody` palette (theme or ink decision, RND-R11) · core `raster::ascii_ramp` (black_hole, gravitational_lensing, gray_scott) and `Tone::value_at` (inverse of level, replaces lensing's `faintest_visible`) with new CORE-R IDs and a round-trip test
- [x] Voyages group: satellite_orbit (shared `Globe`: outward limb glow, day-side-only sand, mid-Ember city lights, label chips), voyager_journey (log-compressed map stated as stylisation, symmetric planet stamps, two-dot recent trail), rocket_launch (pad flame splash, lozenge shock diamonds, stippled top of the air, fading smoke trail, vacuum plume as a short core plus braille gas), moon_landing (seasonal Earth, footpads on the regolith at touchdown), mars_rover (one-ramp `Blackbody` sky with a pale aureole brightest at the Sun, dust devils capped below the white knee, ridges dim at night); `Pixels::backdrop` behind glyphs drawn over half-block pictures (supersedes cosmos-b's proposed `backdrop(cx, cy)`, which is `cell_bg`); worst frames 0.18 / 0.11 / 0.36 / 0.09 / 0.22 ms at 200×50 [SAT-A9, SAT-A12, VOY-A4, VOY-A5, ROC-A5, ROC-A6, ML-A6, ML-A8, MR-A4, MR-A8, MR-A10, CORE-R86]
- [ ] Voyages review follow-ups: draw mars_rover's rover, lander, dust and devils and the other Voyages sprites as pixels instead of glyphs · move `Globe` out of `satellite_orbit` into a shared module (needs a GEN-R3 rule for non-scene files) · a fourth review pass to confirm the round-3 art fixes (touchdown, vacuum plume, planet stamps, aureole, devil veil)
- [x] Cosmos B group: galaxy_collision (per-host long exposure with a turning density-wave spiral, per-star tail exposures, clumped overlap stipple, Hα knots that outlast the encounter), globular_cluster (glow from the bodies' own light blended with a round Plummer profile and a soft knee, three companion dots riding with each body, coloured RGB/BHB stars), pulsar (CP 1919 oblique rotator, `Glow` beams that fade to zero at the cone edge and dim when aimed at us, feathered veil under the *Unknown Pleasures* pulse stack), stellar_nursery (HOO narrowband nebula, lobed pillars with broad heads, knobs, textured and limb-brightened dust, soft `Ember` embedded protostars), starfield (1/d parallax with exposure streaks, narrow soft Milky Way with a star cloud and a dust lane), cosmic_microwave_background (`Div` pixel sky with a sweeping `D_ℓ` inset); core `raster::stronger`, `ink::class_kelvin`, `Pixels::dissolve` and a seventh ramp, `Starlight`, grey while faint; worst frames at 200×50 ≤ 0.63 ms [GAL-A5..A8, GC-A7..A9, PUL-A1..A6, NUR-A4..A7, STAR-A2..A6, CMB-A3, CMB-A6, CMB-A7, CORE-R80, CORE-R84, CORE-R87..R89, RND-R12]
- [ ] Cosmos B review follow-ups: stellar_nursery pillar heads are geometric semicircles (art) · pulsar shows faint radial spokes briefly as the beam turns away from us · galaxy_collision's t = 60 s tails still read as strokes at 200×50 · lift the 2-D value noise now in stellar_nursery, moon_phases and constellation_map into core (new CORE-R ID; the latter two wrap in x) · one bilinear four-tap deposit helper (e.g. `Glow::splat_bilinear`) for galaxy_collision and globular_cluster
- [x] Gallery review: rocket_launch's two countdown clocks agree, action_potential readout uses the house two-space separator [ROC-A8, GEN-R13]
- [x] Gallery review follow-ups (art; resolved in the M12 closing rounds — each scene re-scored ≥ 8 or listed under the closing follow-ups below): too much amber (molecular_dynamics has no physical reason for it; recolour first) · weaker frames: solar_system tiny planets in mostly empty orbits, electron_orbitals small muddy cloud at t = 5 s, molecule_viewer all-grey methane, radioactive_decay flat maroon opening, brownian_motion sparse with `D 1-path …` until it has a fit, double_pendulum twins overlap as one early · pulsar pulse stack busy at 120×40 · ising_model low spin contrast in mono · belousov_zhabotinsky readout leads with the phase name · aurora flat purple lake stripes, no 630 nm red tops
- [x] Perf follow-ups: doppler_effect walks only the on-screen parts of each front (DOP-A11; 500×150 worst 6.5 → 4.6 ms, 400×100 3.7 → 2.5 ms, 200×50 0.97 → 0.72 ms); constellation_map builds its six charts lazily and redraws its static sky only while fading (CON-A10; resize 72.6 → 12.6 ms at 500×150, 10.6 → 1.9 ms at 200×50; mean 1.2 → 0.36 ms); radioactive_decay draws from a per-step SplitMix64 (RAD-A9; 2.3 → 1.9 ms at 500×150); black_hole looks each geodesic row up once (resize 17.7 → 13.6 ms at 200×50, one-off); rocket_launch 1.9–2.5 ms and mars_rover 1.9 ms at 500×150 are steady per-pixel cost, not spikes. Every scene < 1 ms worst frame at 200×50 (max 0.96 ms, ising_model) and < 5 ms at 500×150 (max 4.6 ms, doppler_effect); `pixi run rs-worst` probe and criterion `worst`/`resize` bench groups [GEN-R8, DOP-A11, CON-A10, RAD-A9]
- [x] Determinism across build profiles: LLVM's sin/cos → `sincos` fusion made molecular_dynamics' release frames differ from dev; all trig through `sciart_core::trig`, clippy-enforced, snapshots also run in release in `rs-ci` (ADR-0016) [GEN-R6, CORE-R90, TEST-R9]
- [x] Every upgraded scene < 2 ms worst frame (step + render) at 200×50, allocation-free `render`/steady-state `update`, leak guards green: worst 0.97 ms (doppler_effect act hand-over; per-frame minimum of 3 replays over 180 s), criterion mean max 0.53 ms (stellar_nursery) [GEN-R8, GEN-R11, TEST-R6, TEST-R7]
- [x] Cap fractal_growth's DLA lattice with `grid::lattice_for` (ADR-0012) and drop its TEST-R7 exception (14.5 → 2.7 MiB at 500×150) [CRY-A5, TEST-R7]
- [x] radioactive_decay: size the nucleus lattice to rows `0..h − 1`, so the title is not punched through its last row [RAD-A4]
- [x] Title-row readout polish [GEN-R13]: brownian_motion `D fit 4.63  true 2.25 rows^2/s`; double_pendulum long form padded at the front, not inside (`{growth:.1}x`, form chosen by its maximum width)
- [x] Review (art · science · Rust/perf) with fix loops: the closing review integrated ten groups' fixes, then three implementer rounds (sky & space, physics & matter, chemistry & life) with the chief scientist's wording corrections; every acceptance ID traced, snapshots inspected (dev and release) [TEST-R1, TEST-R2, TEST-R9]
- [x] Core lifts from the closing review: `raster::ascii_ramp_cell` (black_hole, gravitational_lensing), `Glow::splat_bilinear` (galaxy_collision, pulsar), `stellar::limb_darkening` / `brightness_temperature` and `raster::disc_cover` / `disc_pixels` (exoplanet_transit), `Glow::stroke` + `STROKE_KERNEL` + `halo_kernel` (double_pendulum, three_body); ramps `Sulfur`, `Cobalt`, `Eosin` [CORE-R80, CORE-R91..R96, RND-R12]
- [x] galaxy_collision back under the TEST-R7 ceiling (6.24 → 3.95 MiB at 500×150, pre-blurred splats); no TEST-R7 exceptions remain [GAL-A5, TEST-R7]
- [x] Closing bench (`rs-worst`, release, all 41 scenes): worst frame at 200×50 ≤ 0.98 ms (ising_model 0.98, electron_orbitals 0.97, boids_flocking 0.89); at 500×150 max 4.7 ms (doppler_effect); criterion `frame` mean max 0.56 ms at 200×50 (electron_orbitals), `worst` group max 0.89 ms [GEN-R8]
- [ ] Local `--no-ff` merge of `feat/m12-upgrades` into `main`
- [ ] M12 closing follow-ups (minor): aurora's crimson fringe is continuous but stair-steps along the curtain base at t ≈ 5 s (review 7.5–8/10; smooth it sub-cell) · globular_cluster motion (review 7.5/10) · exoplanet_transit star reads flat (7.5) · pulsar spikes at t = 5 s (7.5) · satellite_orbit label crowding · constellation_map Cassiopeia wash · double_pendulum: round 2 intended twin B's bob to stop being a half-disc while merged, but the code (and SPEC-002, which follows it) still clips it to its right half (decide and align) · dna_helix zoom transition at t ≈ 6–8 s · 500×150 worst frames over 2 ms: doppler_effect 4.7, aurora 4.1, stellar_nursery 4.0, moon_phases 3.9, cosmic_microwave_background 3.7, solar_surface 3.2, strange_attractors 2.7, molecule_viewer 2.6, rocket_launch 2.4, wave_interference 2.3, liesegang_rings 2.2, electron_orbitals 2.1, spacetime_grid 2.2, mars_rover 2.1 ms · near-ceiling live bytes at 500×150 (TEST-R7 4 MiB): galaxy_collision 3.95, solar_surface 3.93, electron_orbitals 3.86 MiB · stale test names `dp_a5_…half_green_half_crimson_bobs`, `orb_a8_…over_the_glow`, and MD-A9 covering both caption and relative colour (MD-A10 is now the atom bodies and halo; split the caption out to MD-A11 with a test rename)

## M13 — New scenes ✅
Design: [docs/design/M11-titles-taxonomy.md](design/M11-titles-taxonomy.md) §6, §7 · branch `feat/m13-new-scenes` (may split per category group).
- [x] Curator + critic pass over `M11-research/` plus a fresh astronomy research pass; confirm the picks, including Reactions and the 12 extra sky scenes
- [x] Shared `pub(crate)` models in `src/shared/` (GEN-R3, GEN-R16): excitable (belousov_zhabotinsky, heart_fibrillation, dicty_aggregation), laplacian_growth (lichtenberg_discharge), cahn_hilliard (spinodal_decomposition), vortex_advect, plume, skyline, earth_globe; `lbm` lifted into core with kelvin_helmholtz (CORE-R98..R100). fractal_growth keeps its own DLA walker.
- [x] Deep Cosmos: spiral_galaxy, cosmic_web, supernova_remnant, relativistic_jet, light_echo, planetary_nebula
- [x] Sun & Planets: saturn_rings, comet_tails, jupiter_storms, solar_eruption, io_volcanoes, comet_breakup
- [x] Earth & Sky: total_solar_eclipse, rainbow, hurricane, meteor_shower, noctilucent_clouds, star_trails
- [x] Voyages: jwst_unfolding, orbital_rendezvous
- [x] Forces & Waves: kelvin_helmholtz, quantum_double_slit
- [x] Reactions: catalytic_surface, radical_chain
- [x] Living Systems: lenia, firefly_sync
- [x] States of Matter: spinodal_decomposition, lichtenberg_discharge, chemical_garden (moved from Life from Matter in the Life A round: the model is precipitation and osmosis, P×C)
- [x] Physics of Life: heart_fibrillation, motility_clustering
- [x] Molecules of Life: dicty_aggregation, seashell_pigmentation
- [x] Life from Matter: molecular_motors, hp_protein_folding (added in the Life B round)
- [x] Every new scene: SPEC-002 section, title, category by ADR-0013, < 2 ms worst frame at 200×50, leak guards green; review (final art pass: scores and follow-ups below; see [docs/design/M13-scenes.md](design/M13-scenes.md))
- [x] Local `--no-ff` merge of `feat/m13-new-scenes` into `main`
- [ ] M13 follow-ups: lift the 1-2-1 blur duplicated in motility_clustering and dicty_aggregation (`blur121`) into core `Glow` (done in M14, CORE-R103; supernova_remnant and light_echo keep their float-different blur) · radical_chain: CHN-A5 (vent cool-down) passes only for K_T3 = 0.1 and on one seed; make it robust across seeds and nearby constants · catalytic_surface: CAT-A6 production stalls on some seeds (seed 4 seen while tuning the mid-window opening); make the loop robust across seeds · seashell_pigmentation draws its browns as false-colour Blackbody at 1.6–2.5 kK; switch to `Ramp::Sienna` (added in 7b6a9bf)
  - Art follow-ups from the final art pass (scenes below 8/10; scene, score, weakness):
    - orbital_rendezvous 7: the hero craft is small in an empty frame
    - comet_tails 7.5: the ion tail and the Sun clip at the frame edge
    - io_volcanoes 6.5: static over t = 1..30 s, speckled plume canopy, blocky ring band
    - comet_breakup 7: the impact is not visible, fragments smeared
    - rainbow 6.5: stair-stepped bands, rain streaks read as noise (the garish cyan interior was fixed in 0d16892: a dim `Starlight` grey)
    - total_solar_eclipse 7.5: blocky corona star with a hard edge
    - meteor_shower 7.5: small meteors, flat Milky Way
    - noctilucent_clouds 7: saturated cyan, ripple texture, hard orange horizon
    - catalytic_surface 7.5: rust swallows the frame by t = 30 s
    - motility_clustering 7.5: dim and sparse over t = 1..5 s
    - firefly_sync 7: flashes are per-cell speckle, not glowing points
    - lenia: re-scored 8 after the LNA-A9 cross-fade (no blank frames between acts)
  - Minor: stair-stepped outlines in heart_fibrillation (heart), seashell_pigmentation, hurricane arms, chemical_garden tubes, jupiter_storms limb · hp_protein_folding: reword the spec's C tag to "C: molecular structure as the subject"

## M14 — Resource diet ✅
Design: [docs/design/M14-resource-diet.md](design/M14-resource-diet.md) · branch `perf/m14-resource-diet`.
- [x] Cost probe `rs-cost` with bytes/frame, paint and diff time, blockiness and darkness metrics; baseline table (RND-R13)
- [x] Colour hysteresis in `paint` (RND-R14) · table-driven paint, no invisible escapes (RND-R15)
- [x] Allocation-free frame loop (RT-R14) · one buffered write per frame, synchronized update (RT-R15)
- [x] Fine texture: smooth upsampling of coarse lattices, blockiness ceilings (GEN-R17)
- [x] Dark ground: black backgrounds across scenes, darkness floors (GEN-R18)
- [x] Scene hot spots: every scene < 2 ms worst at 200×50 (max 1.63 ms) and < 5 ms at 500×150 (max 4.36 ms, was 13.8); mean 0.37 → 0.28 ms; resize ≤ 10 ms (GEN-R8, GEN-R19). Not met: 22 scenes still > 2 ms worst at 500×150 (follow-up)
- [x] Art + perf review (≤ 2 rounds), before/after tables, local `--no-ff` merge
- [x] Clean minimal redesign of all 76 scenes (ADR-0017; user redirect: over-design, loud colour, large blocks and thick edges were the regression, not colourfulness); legible-subject rule and art_metrics guards (GEN-R21)
- [x] Results: −49 % CPU per frame, −68 % bytes per frame, 57 → 1 writes per frame, blockiness halved, dark ground 0.68 → 0.89 (docs/design/M14-resource-diet.md)
- [ ] M14 follow-ups: see "Follow-ups" in docs/design/M14-resource-diet.md (22 scenes > 2 ms worst at 500×150, mars_rover sky band blocks, logged art minors, bench `worst` table)

## M15 — M11 look restored ✅
Design: [docs/design/M15-m11-look.md](design/M15-m11-look.md) · [ADR-0018](adr/0018-m11-look-restored.md) · branch `feat/m15-m11-style`. Evidence: `samples/` (every scene at every milestone).
- [x] Samples of every scene at M8, M10–M14; user verdict: M11's look was better (literal revert, flat fills, M11 sparseness, M13 scenes in M11's idiom, keep M14's perf)
- [x] Tests first: M11 ink-frame fixture + fidelity (GEN-R22), M11 palette (GEN-R23), gallery resource budget (RT-R16); `art_metrics` as GEN-R24 per-scene bands
- [x] Revert the 41 M11 scenes (Trig port, fractal_growth lattice cap, prepare hooks; frames identical) and their SPEC-002 sections
- [x] Colour hysteresis band 2 → 4 (RND-R14): gallery back below M14's bytes per frame, inks unchanged
- [x] Restyle the 35 M13 scenes in M11's idiom (critic review ≤ 2 rounds)
- [x] Budgets and results: 0.56 ms CPU, 12.5 KB and 600 changed cells per frame (M14: 0.63 ms, 18.8 KB, 806); every scene < 2 ms worst at 200×50, < 5 ms at 500×150; samples at M15 (`samples/*-m15.png`)
- [x] Local `--no-ff` merge of `feat/m15-m11-style` into main
- [ ] M15 follow-ups: see "Follow-ups" in docs/design/M15-m11-look.md

## M16 — Thread style
Design: [docs/design/M16-thread-style.md](design/M16-thread-style.md) · [ADR-0019](adr/0019-thread-style.md) · GEN-R25.
- [x] light_echo as braille crest rings coloured by age (ECHO-A6..A11)
- [x] Infra: core iso-contour helper lifted from light_echo; delete seashell_pigmentation; GEN-R22 skips GEN-R25 scenes; GEN-R25 half-block share test
- [x] Restyle 25 scenes in the thread style (per-scene choices in the design doc), spec sections first, TDD; core `Dots::contour_cell_refined` (CORE-R106); `shared::jovian` removed
- [x] Critic review (art + science): round 1 sent 12 scenes back, round 2 accepted 11, spacetime_grid in a 3rd two-constant round; budgets: every worst frame < 1.2 ms at 200×50 and < 2.8 ms at 500×150
- [x] Results: gallery 0.535 ms CPU, 9.9 KB and 576 changed cells per frame (M15: 0.56 ms, 12.5 KB, 600); the 26 thread scenes 13.7 → 5.8 KB per frame
- [x] Samples at M16 (`samples/*-m16.png`, M16 column in `samples/README.md` and `samples/compare/`)
- [x] Local `--no-ff` merge of `feat/m16-thread-style` into main
- [ ] M16 follow-ups: see "Follow-ups" in docs/design/M16-thread-style.md

## M17 — Orbital-thread style ✅
Design: [docs/design/M17-orbital-thread.md](design/M17-orbital-thread.md) · [ADR-0020](adr/0020-orbital-thread-style.md) · GEN-R26, CORE-R107.
- [x] Visual options per scene (user picked all recommended designs); spec first: GEN-R26, CORE-R107, `Look (M17)` + acceptance IDs in 10 sections
- [x] Infra: `raster::stipple_keep` / `Dots::stipple` (CORE-R107); GEN-R26 list, half-block and stillness test; GEN-R22 skips GEN-R26 scenes
- [x] Redesign gray_scott, ising_model, molecular_motors (team A)
- [x] Redesign gravitational_lensing, spacetime_grid, three_body, spiral_galaxy (team B)
- [x] Redesign io_volcanoes, solar_eruption, supernova_remnant (team C)
- [x] Lead review (art + science, ≤ 2 rounds per team); budgets: worst frame ≤ 1.27 ms at 200×50 and ≤ 3.05 ms at 500×150; the 10 scenes 9.2 → 4.2 KB and 517 → 320 changed cells per frame
- [x] Results (gallery 0.544 ms, 9.2 KB, 549 changed cells per frame), samples at M17, the contributor guide
- [x] Local `--no-ff` merge into main
- [ ] M17 follow-ups: see "Follow-ups" in docs/design/M17-orbital-thread.md

## M18 — Orbital molecules ✅
Design: [docs/design/M18-orbital-molecules.md](design/M18-orbital-molecules.md) · [ADR-0021](adr/0021-orbital-molecules.md) · GEN-R26 (molecules rule), CORE-R108.
- [x] Visual options per scene (user picked the recommended design for all but chemical_garden: bead chains); spec first: GEN-R26 list + molecules rule, CORE-R108, `Look (M18)` + acceptance IDs in 6 sections
- [x] Infra: `Dots::stipple_puff` (CORE-R108); GEN-R26 test list
- [x] Redesign wave_interference, spiral_galaxy (6 arms) (team A)
- [x] Redesign physarum, firefly_sync (team B)
- [x] Redesign chemical_garden, heart_fibrillation (team C)
- [x] Lead review (art + science, ≤ 2 rounds per team); budgets: worst frame ≤ 0.72 ms at 200×50 and ≤ 3.38 ms at 500×150; the 6 scenes 25.9 → 3.7 KB and 1121 → 257 changed cells per frame
- [x] Results (gallery 0.539 ms, 7.4 KB, 480 changed cells per frame), samples at M18, the contributor guide
- [x] Local `--no-ff` merge into main
- [ ] M18 follow-ups: see "Follow-ups" in docs/design/M18-orbital-molecules.md

## M19 — galaxy_collision star dust ✅
GEN-R25 (list + `Look (M19)` in galaxy_collision), GAL-A5..A8. User picked: star dust + afterglow tails, two-arm grand-design disks.
- [x] Spec first: GEN-R25 list, `Look (M19)` + GAL-A5..A8
- [x] Failing tests GAL-A5..A8, then the redesign (4× stars as braille dust; afterglow threads only for tidal material beyond 1.15 Rd, after a first all-star version filled the disks); snapshots re-recorded and reviewed; art_metrics row re-seeded after review
- [x] Budgets: worst frame 0.26 ms at 200×50 and 0.88 ms at 500×150 (`rs-worst`); 8.1 KB and 829 changed cells per frame at 200×50 (`rs-cost`)

## M9 — Future (unscheduled)
- `sciart record`, GitHub Actions + cargo-deny, Windows/Linux keep-awake backends, per-generator config (the braille hi-res canvas landed in M10 as `raster::Dots`).
