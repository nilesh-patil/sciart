# SPEC-002: Generators & registry

**Crate**: `crates/sciart-generators` · **Milestones**: M1 (registry, starfield), M4–M7 (batches), M10 (Batch E), M11 (titles, taxonomy), M13/M21 (new scenes) · **Status**: Implemented (GEN-R1..GEN-R26); the registry holds 85 scenes in 11 categories

Ports the Python prototype's stability contract (its rule R1) to Rust
and extends it for real-time animation.

## Registry
- **GEN-R1** `registry() -> &'static [Entry]` lists every generator; `Entry { meta: &'static Meta, build: fn() -> Box<dyn Generator> }`. Names are unique, `snake_case`, and sorted.
- **GEN-R2** `find(name) -> Option<&'static Entry>` is exact-match on `meta.name`, or on a retired name (GEN-R14).
- **GEN-R3** Adding a generator requires exactly: one module file + one line in the registry macro. The contract suite picks it up automatically. Models shared by several scene crates live in `crates/sciart-shared`; a model with a single scene crate's users lives there as a crate-private helper module instead (`mod <name>;` in that crate's `lib.rs`, `pub(crate)` items, never registered). Helper modules are named in a source-rules allowlist, so a stray file still fails the rule. *(M22, ADR-0023: scenes live in five category crates, `crates/sciart-scenes-{cosmos,earth,physics,matter,life}/src/<name>.rs`, and shared models in `crates/sciart-shared`; `sciart-generators` is the registry facade whose `registry![…]` lists every scene, so adding a scene is one module file in its category's crate (declared `pub mod` in that crate's `lib.rs`) + one registry line, and `registry()`/`find` and every test suite are unchanged. A source-rules test asserts every scene file in those crates is registered, every registered scene lives in the crate its `Meta.category` maps to, and the only non-scene files are the allowlisted helpers: `physics` keeps `lbm`, `matter` keeps `cahn_hilliard` and `laplacian_growth`.)*

## Stability contract (every registered generator)
- **GEN-R4** Never panics for any `Size` with `w, h >= 1` across any sequence of `resize`, `update(dt)` (`0 <= dt <= 0.25`), and `render` calls — including resizes between any two calls.
- **GEN-R5** Dimension-dependent state is rebuilt in `resize`; `update`/`render` must tolerate the smallest size (1×1).
- **GEN-R6** Determinism: same seed + same call sequence ⇒ byte-identical `Canvas::to_lines()`, in every build profile: a frame must not depend on the optimiser. Rust never contracts to FMA or reorders float operations, but LLVM does fuse `sin`/`cos` pairs by inlining context, so all trigonometry goes through `sciart_core::trig` (CORE-R90), enforced by `clippy.toml` `disallowed-methods`; the snapshot suite runs in the optimised `ci` profile (opt-level 3, no LTO; TEST-R9, ADR-0023).
- **GEN-R7** No wall-clock reads (`Instant`, `SystemTime`) and no global RNG (`rand::rng()`/`thread_rng`) inside generators; time comes from `dt`/`ctx.time`, randomness from `ctx.rng`. Enforced by `clippy.toml` `disallowed-methods` and a source scan of core and generators, `src/shared/` included.
- **GEN-R8** Performance: `update(1/60)` + `render` < 2 ms at 200×50 in release (criterion bench per generator). Means hide act hand-overs, so the bench also times the slowest known frames (group `worst`: the slowest of 3 frames around each hand-over or peak) and the resize hitches (group `resize`), and `examples/worst_frame` (`pixi run rs-worst`) replays every scene and reports its worst single frame: < 2 ms at 200×50 and < 5 ms at 500×150. A resize is a one-off and may take longer (black_hole's geodesic trace ≈ 14 ms); precompute there, or amortise over the frames before the result is needed, never in one frame of `update`.
- **GEN-R9** Motion: over 1 simulated second at 200×50 the rendered frame changes (a scene is an animation, not a still). Verified by the contract suite.
- **GEN-R10** Unicode glyphs must have an ASCII fallback (`Glyph::new(u, a)`); ASCII mode output is pure ASCII.
- **GEN-R11** After warm-up (build, resize, ≥ 1 step and one render), `render` performs zero heap allocations, and `update(1/60)` performs zero allocations in steady state unless the scene is listed with a reason in `UPDATE_ALLOCATES` of `crates/sciart-generators/tests/alloc_free.rs` (empty today). Resizing may allocate. Verified at 200×50 and 80×24, in both glyph modes, by that test (ADR-0009).

## Titles, categories and names (M11)
Decisions: ADR-0013 (taxonomy) · ADR-0014 (titles). Mechanism: CORE-R71..R77.
- **GEN-R12** Every registered scene has a valid science title (ADR-0014). The contract suite checks `meta.title` and every entry of `meta.captions`:
  - the title contains exactly one ` · `, with non-empty, trimmed text on both sides;
  - its width is ≤ 32 cells and the phenomenon's is ≤ 20 cells (`Title::width`, CORE-R72);
  - each caption is non-empty, trimmed and has no ` · `, and `phenomenon · caption` is ≤ 32 cells;
  - every char is in the title charset (`title::ascii_of(c).is_some()`), and none of `= ^ _ * / < >` appears (no formulas or markup);
  - titles are unique across the registry;
  - `caption()` sampled at t = 0, 1, 5 and 20 s (seed 42, 80×24) is `None` or an entry of `meta.captions`, and a scene with empty `captions` always returns `None`.

  A title never claims more than the model does; wording is signed off by the science review.
- **GEN-R13** The bottom-left belongs to the title. With the title hidden (`Stage::set_title_visible(false)`), at 80×24 and 40×12, seed 42, t = 0, 1 and 5 s, in Unicode glyph mode, no cell of row `h − 1` left of where the longest `title::readout_room` readout may start (the span of `Title::of(scene)` plus the 4-cell gap, CORE-R76) holds an alphanumeric char. Only text is excluded: half-block, braille and field art may fill the row. A scene that places a readout on row `h − 1` right-aligns it with `draw::label_right` (CORE-R76), inks it `Text` at `title::READOUT_LEVEL` (checked for every scene at 80×24, 40×12 and 200×50, t = 1 s), and shortens or drops it so that at least 4 blank cells separate it from the title's text: forms are chosen against `title::readout_room` (CORE-R76) with the live caption, whether or not the title is drawn. Other scene text (names, epithets) stays off row `h − 1` while the title is shown, so it never reads as part of the title. The moves below supersede the bottom-left placements written in the scene sections.

  | Scene | M11 change |
  |-------|-----------|
  | action_potential | Readout (`HH squid …`) right-aligned on row `h − 1`; its form follows the room right of the title. |
  | belousov_zhabotinsky | The name `Belousov-Zhabotinsky` is dropped (the title names the phenomenon); the beat word and `b=…` are right-aligned, forms chosen by the room. *Superseded by the M12 scene section.* |
  | boids_flocking | `N 192  k=7` right-aligned. |
  | brownian_motion | MSD readout right-aligned, parts two spaces apart; when it would reach the title, trailing parts are dropped (`t = …s` first, then the true D); hidden when the estimator alone does not fit. |
  | cosmic_microwave_background | `CMB  T=2.725K  dT~1e-5` right-aligned instead of centred. |
  | double_pendulum | Divergence and energy readout right-aligned instead of centred. |
  | doppler_effect | The legend describes sound and is right-aligned, longest form that fits: `<- lower pitch  v = 0.5 c_s  higher pitch ->`, `<- lower  v = 0.5 c_s  higher ->`, `v = 0.5 c_s`. *Superseded by the M12 scene section.* |
  | electron_orbitals | Orbital name and transition readouts right-aligned. |
  | fractal_growth | The phrase `diffusion-limited aggregation` is dropped (the title paraphrases it); `p_s …   N …   D_f …` right-aligned; whole parts are dropped from the left, D_f last. |
  | molecular_dynamics | `T 0.40 -> 0.25  U/N -2.63  N 240  LJ units` right-aligned (measured T, then the thermostat target), shortened by dropping `LJ units`, then `U/N … N …`, then `-> target`, else nothing. |
  | physarum | `SA … RA … SO …` right-aligned. |
  | solar_surface | `day …  Teff 5772 K` right-aligned (Teff still dropped first when narrow). |
  | strange_attractors | The attractor name becomes `caption()`; `meta.captions` = `Lorenz`, `Aizawa`, `Thomas`, `Halvorsen`, `Rössler`, `Chen` (`Strange attractor · Chen`, 23 cells), and `meta.title` is `Strange attractor · chaos`. The parameter set, when it fits, is right-aligned. |
  | constellation_map | The α-star label is clamped to rows ≤ `h − 2`; the constellation name and its epithet to rows ≤ `h − 3`, so a blank row separates them from the title. |
  | molecule_viewer | The shown molecule's name is `caption()`; `meta.captions` = `water`, `methane`, `ethanol`, `benzene`, `caffeine`, `buckminsterfullerene`. |
  | moon_landing | `caption()` follows the landing: `powered descent` until the footpads touch, `touchdown` until the flag beat, then `Tranquility Base`. The intro caption names only the craft: `LUNAR MODULE EAGLE`, `LM EAGLE`, `EAGLE`. |
  | moon_phases | The phase name is clamped to rows ≤ `h − 2`. |
  | particle_accelerator | The beam-energy gauge on row `h − 1` starts at least 4 blank cells after the title's text, is shortened to end before the last column, and is dropped below 8 cells. |
  | three_body | The shown orbit's short name is `caption()`; `meta.captions` = `figure-8`, `Lagrange`, `Broucke A1`, `Broucke R4`, `moth I`. |
- **GEN-R14** Renamed scenes keep their old names as aliases: `ALIASES: &[(&str, &str)]` maps `chemical_reaction → gray_scott` and `crystal_growth → fractal_growth`. `find(name)` tries `meta.name` first, then an alias, so `sciart run <old name>` and `disabled = ["<old name>"]` (CFG-R7) keep working. Every alias target is registered, and no alias equals a registered name. `registry()` and `sciart list` show only current names. Acceptance IDs keep their prefixes (`CHEM-A*`, `CRY-A*`).
- **GEN-R15** Scene sections live in the category files `docs/specs/SPEC-002-scenes-{cosmos,earth,physics,matter,life}.md`; this file keeps the rules, the template and a generated scene index. Each scene section heading reads `### <name>  (<category label>, <milestones>)`; every registered scene has exactly one such section across those files, and its label equals `meta.category.label()` (CORE-R77), which is how each scene's category (ADR-0013) is written down. Checked by a test that parses every `SPEC-002*.md` file and by the generated index (every scene appears once in it).
- **GEN-R16** No scene imports another scene: a scene module uses only `std`, `rand`, `sciart_core`, `sciart_shared::…` and its own crate's allowlisted helper modules (`crate::lbm::…` in physics, `crate::cahn_hilliard::…`/`crate::laplacian_growth::…` in matter; scene crates depend only on `sciart-core`, `sciart-shared` and `rand`). Code that several scenes need moves to `sciart-shared` (or into core with its own CORE-R ID); a model with one crate's users lives as that crate's helper module. A helper never imports a scene. (M13: `satellite_orbit`'s Earth globe, used by `moon_landing`, moved to `shared::earth_globe`.)
- ~~GEN-R17~~ *Superseded by GEN-R22–R24 (M15, ADR-0018).* (M14) Fine texture: a scene that simulates a field on a lattice coarser than the pixels it draws shows it through smooth upsampling to the pixel grid, not nearest-neighbour blocks: bilinear up to 2× magnification, separable Catmull-Rom (CORE-R102) above it, so capped lattices on large terminals show no diamond creases or soft plateaus; sprites and strokes use sub-pixel `Glow::stroke`/`halo_kernel`/braille, not cell-sized stamps; unless the lattice itself is the subject (cellular automata, discrete states, lattice proteins). Measured by the RND-R13 *blockiness* on the pixel grid (`▀` is fg over bg, `▄` bg over fg, `█` fg twice, a blank cell with a bg is that bg twice; any other glyph is a glyph pixel and never part of a flat square): the share of visible field pixels (luma > 0.08) covered by a flat 4×4 window of one colour that has, just outside its edge, a visible field pixel one *stair* step away (a different colour, max channel difference ≤ 64), so object edges against black do not count; at 200×50, t = 5 s and 20 s. The probe also reports the 2×2 variant (`blk2`), which is not ratcheted. `tests/art_metrics.rs` holds a per-scene ceiling table that may only be lowered.
- ~~GEN-R18~~ *Superseded by GEN-R22–R24 (M15, ADR-0018).* (M14) Dark ground: empty space is `Role::Background` (the terminal default, black on a dark terminal, and the cheapest cell to send). Full-frame fields and skies map their low end to near-black, and colours are graded to sit on black. Measured by the RND-R13 *darkness*: the share of ground pixels (half-block pixel colours, else the cell background) that are default or truly black, max(R, G, B) ≤ 28 (a navy or teal ground is not dark, however low its luma); the probe also reports mean ground luma. `tests/art_metrics.rs` holds a per-scene floor table that may only be raised. A scene whose subject is a lit backdrop uses a dim, dark-leaning one and says so in its section.
- **GEN-R19** (M14) Scene switches don't hitch: `resize` takes at most 10 ms at 500×150 in a release build (`pixi run rs-worst` reports it). Heavy tables are built once per size, amortised over the first frames, or shared through `OnceLock`.
- **GEN-R20** (M14) Rasterise once per drawn frame: `Generator::prepare(&mut self)` (default no-op) is called by `Stage::render` right before `render`. The runtime takes about two 60 Hz steps per 30 FPS frame, so a scene that rasterises in `update` wastes every raster but the last; such scenes rasterise in `prepare` behind a dirty flag instead. Frames are identical to rasterising in `update`: the raster depends only on simulation state, so snapshots do not change. `prepare` is allocation-free like `render` (GEN-R11) and must not advance the simulation or draw from `ctx.rng`.
- ~~GEN-R21~~ *Superseded by GEN-R22–R24 (M15, ADR-0018).* (M14, ADR-0017) Clean minimal style: scenes stay quiet and uncluttered. On the painted `astro` frame at 200×50, t = 5 s and 20 s, `tests/art_metrics.rs` holds per-scene ceilings for four measures. *Lit coverage* is the share of pixels with max channel > 28. *Chroma mass* is mean OKLCh chroma over all pixels, with unlit pixels counted as 0. *Edge weight* is lit pixels over boundary pixels on the 2×2-pooled lit mask; a thin line or point scores about 1–3, a filled blob tens. The fourth is a neon cap: the 90th percentile of OKLCh chroma over lit pixels, at most 0.20. Two floors keep a scene from going faint (ADR-0017 rule 7): the 99th percentile of lit-pixel luma is at least 0.6 in every scene (read from the colours the half-block pixel grid actually shows, glyph foreground or field colour, not the 2×2-pooled grid of the other clean measures, where one white braille dot pools to luma ≈ 0.28; scenes below it today are listed in a `KNOWN_FAINT` allowlist that may only shrink), and each scene has a lit-coverage floor. The ceilings and floors are seeded after the M14 redesign; a ceiling may only be lowered and a floor only raised, and moving one the other way needs a reviewed note in the scene's section. Colourfulness is deliberately not a quality floor. M12 raised it while the user judged the gallery worse (over-design, loud colour, blocks, thick edges). The user's references (`double_pendulum`, M11 `galaxy_collision` and `rocket_launch`) score coverage 0.01–0.08, chroma mass ≤ 0.006 and edge weight 1.0–2.8, against 0.90–1.00, 0.08–0.16 and 49–95 for the over-designed scenes, measured at 120×40 (at 200×50 the edge weight of filled shapes is about 1.7× higher).
- **GEN-R22** (M15, ADR-0018) M11 fidelity: each of the 41 scenes registered at M11 (the M11 merge) renders as it did at M11. Seed-42 frames at 80×24, in Unicode and in ASCII mode, after 0, 1 and 5 s of simulated time, match the M11 reference cell for cell (glyph, foreground and background ink role and level) in at least 99.5 % of the cells of every frame; the port to deterministic trigonometry (GEN-R6) may move a last bit. The reference is `crates/sciart-generators/tests/fixtures/m11_frames.txt`, written from the M11 tree by the `inkframes` example (the same example in both trees); exceptions are listed in the test with a reason. A reverted scene keeps its M11 SPEC-002 section, with today's heading and category (GEN-R15).
- **GEN-R23** (M15, ADR-0018) M11 palette: every scene inks only with the roles that existed at M11: `Background`, `Dim`, `Star`, `Sun`, `Planet`, `Particle`, `Wave`, `Hot`, `Cold`, `Structure`, `Accent`, `Text`, `Seq` and `Div`. Checked on seed-42 Unicode frames at 80×24 and 200×50 after 0, 1, 5 and 20 s: no drawn cell (glyph, foreground or half-block background) uses `Role::Ramp(_)`, `Role::Blackbody` or `Role::Phase`, which stay in the core and themes for later use.
- **GEN-R24** (M15, ADR-0018) M11 idiom for the scenes added after M11: a black ground; braille dots for lines, orbits, trails and particles; half-block pixels for bodies and fields, where flat, stepped fills are allowed; saturated semantic colours in complementary pairs (`Hot`/`Cold`, `Accent`); a small subject with much black around it; no tone-mapped glow washes, halos or gradient backdrops; labels and readouts as at M11. Each keeps its model, motion and title. The art review judges it against the M11 looks (the frames GEN-R22's fixture pins); `tests/art_metrics.rs` holds each reviewed look as a per-scene band (lit coverage, chroma mass and edge weight ceilings, darkness floor), re-seeded with `rs-cost -- <scene> --emit-limits` only after review.
- **GEN-R25** (M16, ADR-0019) Thread style, after `strange_attractors`, for `light_echo`, `catalytic_surface`, `saturn_rings`, `dicty_aggregation`, `total_solar_eclipse`, `radical_chain`, `comet_breakup`, `lenia`, `quantum_double_slit`, `belousov_zhabotinsky`, `jupiter_storms`, `globular_cluster`, `vortex_street`, `jwst_unfolding`, `rainbow`, `hurricane`, `relativistic_jet`, `planetary_nebula`, `cosmic_web`, `hp_protein_folding`, (M19) `galaxy_collision` and (M20) `solar_system`, `brownian_motion`, `action_potential` and (M21) `magnetosphere`, `gravity_assist`, `snowflake_growth` and `cloud_chamber`: braille dots (`raster::Dots`) for lines, waves, fields (drawn as contours or crests, not fills), particles and trails; half-block pixels only for one small solid body per scene (a disc, a planet, a mirror), in flat steps; M11 roles only (GEN-R23), coloured along the thread by a physical variable (age, sign, speed, temperature, species) in complementary pairs; decaying trails only for moving things; a black ground between threads. Each listed scene has a `Look (M16)` paragraph in its section, which supersedes its earlier inks where they disagree. The listed M11 scenes are no longer pinned by GEN-R22. Measured by `art_metrics` (per-scene bands re-seeded after review) and by a share of half-block cells at 200×50: at most 3 % of lit cells of a listed scene are half-block cells (the small body). *(M17: `supernova_remnant`, `ising_model`, `solar_eruption`, `spiral_galaxy`, `gravitational_lensing` and `spacetime_grid` moved to GEN-R26, whose `Look (M17)` supersedes their `Look (M16)`.)*
- **GEN-R26** (M17, ADR-0020) Orbital-thread style, a blend of `electron_orbitals` (hydrogen clouds) and `strange_attractors` (threads), for `gray_scott`, `molecular_motors`, `gravitational_lensing`, `io_volcanoes`, `spacetime_grid`, `solar_eruption`, `three_body`, `spiral_galaxy`, `supernova_remnant`, `ising_model`, `wave_interference`, `chemical_garden`, `physarum`, `firefly_sync`, `heart_fibrillation` and (M21) `binary_inspiral`, `chladni_plate`, `rayleigh_benard`, `mitosis`, `ribosome_translation` and `protocell_assembly`: everything GEN-R25 asks (braille first, at most 3 % half-block cells, M11 roles only, a black ground), plus two rules. (1) *Clouds*: bodies and media (cells, heads, plasma, ejecta, domains, the lens, the radiation pattern) are probability stipple: a braille dot is lit with probability equal to a physical density (|ψ|², a concentration, emission measure, column density, spin sign near a wall), coloured by the sign or phase of a physical variable in a complementary pair (`Hot`/`Cold`, as the two signs of an orbital lobe), and each dot's acceptance threshold is fixed (a stored per-sample uniform, or `raster::stipple_keep`, CORE-R107), so a still cloud holds still and a changing one changes only where its density crosses a threshold. (2) *Threads*: structure and motion are 1-px braille threads coloured along their length by a physical variable (speed, age, temperature), with a decaying afterglow only for moving things. (3) *Molecules* (M18, ADR-0021): discrete bodies (wave sources, precipitate beads, food flakes, fireflies, spiral tips) are small Gaussian stipple atoms (`Dots::stipple_puff`, CORE-R108), joined by 1-px bond threads only where a physical link exists (a tube's growth, a coupling pulse). Each listed scene has a `Look (M17)` or `Look (M18)` paragraph in its section that supersedes its earlier looks and inks where they disagree, and the listed M11 scenes are no longer pinned by GEN-R22. Measured at 200×50 by `tests/orbital_style.rs`: the list equals this one, at most 3 % of lit cells are half-block cells at t = 5 and 20 s, and on average at most 10 % of cells change between consecutive drawn frames (two 60 Hz steps) over t = 10–12 s (the hydrogen reference changes 3 %); the one exception is `spacetime_grid`, at most 11 %, because its M16 rubber sheet is kept exactly and the sheet's travelling crests alone change 10.1 %; `art_metrics` bands are re-seeded after review.

## Per-generator spec template
Each scene gets a short section appended below when its batch starts:

```
### <name>  (category label, batch)
Intent: what the viewer should see/feel.
Model: state, physics/maths, parameters.
Motion: what changes over time and at what rate.
Resize: what is rebuilt.
Inks: roles used.
Acceptance: scene-specific assertions beyond GEN-R4..R10.
```

---

> The scene sections live in the category files (GEN-R15): `SPEC-002-scenes-{cosmos,earth,
> physics,matter,life}.md`. This file keeps the rules, the template and the index; the batch
> headings after the index keep their shared-helper and history notes, but their scene
> sections have moved.

## Scene index

<!-- scene-index:start -->
| Scene | Category | Spec |
|---|---|---|
| `action_potential` | Physics of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `aurora` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `belousov_zhabotinsky` | Reactions | [scenes-matter](SPEC-002-scenes-matter.md) |
| `binary_inspiral` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `black_hole` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `boids_flocking` | Living Systems | [scenes-life](SPEC-002-scenes-life.md) |
| `brownian_motion` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `catalytic_surface` | Reactions | [scenes-matter](SPEC-002-scenes-matter.md) |
| `chemical_garden` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `chladni_plate` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `cloud_chamber` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `comet_breakup` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `comet_tails` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `constellation_map` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `cosmic_microwave_background` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `cosmic_web` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `dicty_aggregation` | Molecules of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `dna_helix` | Molecules of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `doppler_effect` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `double_pendulum` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `electron_orbitals` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `exoplanet_transit` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `firefly_sync` | Living Systems | [scenes-life](SPEC-002-scenes-life.md) |
| `fractal_growth` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `galaxy_collision` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `globular_cluster` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `gravitational_lensing` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `gravity_assist` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `gray_scott` | Reactions | [scenes-matter](SPEC-002-scenes-matter.md) |
| `heart_fibrillation` | Physics of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `hp_protein_folding` | Life from Matter | [scenes-life](SPEC-002-scenes-life.md) |
| `hurricane` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `io_volcanoes` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `ising_model` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `jupiter_storms` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `jwst_unfolding` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `kelvin_helmholtz` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `lenia` | Living Systems | [scenes-life](SPEC-002-scenes-life.md) |
| `lichtenberg_discharge` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `liesegang_rings` | Reactions | [scenes-matter](SPEC-002-scenes-matter.md) |
| `light_echo` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `magnetosphere` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `mars_rover` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `meteor_shower` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `mitosis` | Physics of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `molecular_dynamics` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `molecular_motors` | Life from Matter | [scenes-life](SPEC-002-scenes-life.md) |
| `molecule_viewer` | Reactions | [scenes-matter](SPEC-002-scenes-matter.md) |
| `moon_landing` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `moon_phases` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `motility_clustering` | Physics of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `noctilucent_clouds` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `orbital_rendezvous` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `particle_accelerator` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `physarum` | Living Systems | [scenes-life](SPEC-002-scenes-life.md) |
| `planetary_nebula` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `protocell_assembly` | Life from Matter | [scenes-life](SPEC-002-scenes-life.md) |
| `pulsar` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `quantum_double_slit` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `radical_chain` | Reactions | [scenes-matter](SPEC-002-scenes-matter.md) |
| `radioactive_decay` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `rainbow` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `rayleigh_benard` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `relativistic_jet` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `ribosome_translation` | Molecules of Life | [scenes-life](SPEC-002-scenes-life.md) |
| `rocket_launch` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `satellite_orbit` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `saturn_rings` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `snowflake_growth` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `solar_eruption` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `solar_surface` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `solar_system` | Sun & Planets | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `spacetime_grid` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `spinodal_decomposition` | States of Matter | [scenes-matter](SPEC-002-scenes-matter.md) |
| `spiral_galaxy` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `star_trails` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `starfield` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `stellar_nursery` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `strange_attractors` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `supernova_remnant` | Deep Cosmos | [scenes-cosmos](SPEC-002-scenes-cosmos.md) |
| `three_body` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `total_solar_eclipse` | Earth & Sky | [scenes-earth](SPEC-002-scenes-earth.md) |
| `vortex_street` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
| `voyager_journey` | Voyages | [scenes-earth](SPEC-002-scenes-earth.md) |
| `wave_interference` | Forces & Waves | [scenes-physics](SPEC-002-scenes-physics.md) |
<!-- scene-index:end -->

## Batch A — analytic / time-driven scenes (M4)
Shared helpers: `sciart_core::draw` (CORE-R22..R27). All lengths below are in **rows**; x is stretched by `ASPECT`. `R = draw::fit_radius(size, frac)`.

---
## Batch B — particle systems (M5)
Shared helpers: `sciart_core::particles` (CORE-R28..R32). Pools are sized in `resize` from the area and are
bounded (GEN-R8); particles are respawned rather than reallocated. Positions are in **rows** unless stated
(x on screen = `cx + x·ASPECT`). Physics uses `dt` from `update` only.

---
## Batch C — grids, automata & stochastic processes (M6)
Shared helpers: `sciart_core::grid` (CORE-R33..R36) plus `particles` and `draw`. Grids are rebuilt (or `resize`d) in `resize`.

## Batch D — physics sims & narrative scenes (M7)
Shared helpers: `ode` (RK4), `timeline` + `ease` (CORE-R37..R39) plus `particles`, `grid`, `draw`. Narrative scenes loop forever: every phase has a fixed duration in simulated seconds, so a run is deterministic per seed and resizes never desynchronise the story.

---
## Batch E — scientific art: fields, gravity, chemistry & biology (M10)
Decisions: ADR-0010 (layered raster: `raster::Dots` braille, `raster::Pixels` half-block with `Cell.bg`), ADR-0011 (colormap roles `Seq`/`Div`), ADR-0012 (simulation lattice decoupled from the display, `grid::lattice_for`).
New core helpers (CORE-R40..R59, RND-R6/R7): `raster::{Dots, Pixels, Density}`, `grid::{resample_bilinear, resample_nearest, lattice_for, SampleMap, laplacian9}`, `ode::{verlet_step, verlet_integrate, tridiag_solve}`, `geom::{Camera, Projector}`, `ink::{seq, div, soft_clip, seq_value, div_value}`, `text::StackText`, `draw::text_mode`, `particles::{SpatialHash, InverseCdf}`, `rng::SplitMix64`, `Category::{Biology, Chemistry}`; the post-M10 audit lifted more shared code into core (CORE-R60..R70); plus the existing `draw`, `particles`, `grid`, `ode`, `timeline`/`ease` and `geom` helpers.
The `gray_scott` upgrade is specified in place in Batch C above (`CHEM-A5`..`CHEM-A8`).

## Batch M13 — new scenes
Every scene here: < 2 ms worst frame (step + render) at 200×50 (`pixi run rs-worst`), `render` and steady-state `update` allocation-free (GEN-R11), growing structures in fixed-capacity pools, live heap under the TEST-R7 ceiling at 500×150, trigonometry through `Trig` (GEN-R6), title per GEN-R12 and category per ADR-0013. Models shared across scene crates live in `crates/sciart-shared` (GEN-R3, GEN-R16): `vortex_advect` (jupiter_storms, hurricane), `plume` (io_volcanoes, comet_breakup), `excitable` (heart_fibrillation, dicty_aggregation; belousov_zhabotinsky's Barkley step and pacemaker call it) and `skyline` (total_solar_eclipse, rainbow, meteor_shower, noctilucent_clouds, star_trails). Single-crate models are crate-private helpers: `lbm` in physics (kelvin_helmholtz, vortex_street; LBM-A1/A2), `cahn_hilliard` (spinodal_decomposition) and `laplacian_growth` (lichtenberg_discharge) in matter.

## Batch M21 — ten new scenes
Every look below was chosen from previews. GEN-R25 (thread) lists `magnetosphere`, `gravity_assist`, `snowflake_growth`, `cloud_chamber`; GEN-R26 (orbital-thread, with molecules) lists the other six. All M21 scenes: M11 roles only (GEN-R23), a black ground, ≤ 3 % half-block cells, stipple peak density ≤ 0.5 with small atoms that have a solid core, worst frame < 2 ms at 200×50 (aim < 1 ms) and < 5 ms at 500×150, `resize` < 10 ms at 500×150 (GEN-R19), rasterise in `prepare` behind a dirty flag (GEN-R20), allocation-free `render`/steady `update` (GEN-R11). A GEN-R26 scene that cannot hold the 10 % changed-cells rule gets a documented, reviewed exception in `orbital_style.rs`, as `spacetime_grid` has (none was needed). `cloud_chamber`, `mitosis` and `protocell_assembly` peak at a p99 lit luma of ≈ 0.52–0.54, the astro luma of `Hot`, `Cold` and `Accent` at full level.
