# Backlog

Open work that is **not** a milestone: follow-ups from the M10–M25 milestones, stale
roadmap claims, and reviewed-but-deferred polish. The roadmap stays milestone-level; this file
holds the detail, so an item cannot disappear into a ticked box.

**Last triaged**: 2026-10-18 (M23, IMP-13).

**Review rule**: no item survives two milestones without a decision — fix it, narrow it,
mark it superseded, or mark it `won't fix`. At each milestone close, update the status here
and keep the roadmap line truthful ([CONTRIBUTING](../CONTRIBUTING.md#backlog-review)).

## How to read

| Field | Meaning |
| --- | --- |
| **Status** | `open` — still true and undecided · `done` — resolved (kept so the roadmap box can close) · `superseded` — replaced by later work/spec, no action · `won't fix` — considered and declined, reason in the note |
| **Priority** | **P1** correctness, a truth/claim problem, or blocks release · **P2** quality/robustness worth scheduling · **P3** opportunistic polish |
| **Source** | the roadmap box or milestone follow-up that raised it; code links when they make the item concrete |

A row is one item. Where an old roadmap sentence bundled several independent items, the rows
below split it so each can close on its own.

## Summary

| Status | Count |
| --- | ---: |
| open | 64 |
| done | 16 |
| superseded | 13 |
| won't fix | 3 |
| **total** | **96** |

Counted from the tables below (M25); re-count after edits.

## Docs truth (stale claims)

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-01 | docs / SPEC-002 | GEN-R15 and the contributor guide point at `docs/specs/SPEC-002-scenes-{cosmos,earth,physics,matter,life}.md` and `scripts/spec_index.py`, but only `SPEC-002-generators.md` exists and there is no `spec_index.py`. | [SPEC-002](specs/SPEC-002-generators.md) | done | P1 | M23: the split is applied, the index is generated, and `spec_index.py --check` plus GEN-R15/GEN-R15-index pass. |
| B-02 | ROADMAP / M12 | "Local `--no-ff` merge of `feat/m12-upgrades` into `main`" is still an open box; the merge is long done. | [ROADMAP](ROADMAP.md) | done | P1 | Close the box (merged; M12 is marked done). |
| B-03 | ROADMAP / M12 | "snapshots pass only in the debug profile (`molecular_dynamics_t5` differs in release)" is stale. | [ROADMAP](ROADMAP.md) | superseded | P1 | ADR-0016 removed the `sincos` fusion; M22 runs snapshots in the `ci` profile; `rs-snap-release` is green. |
| B-04 | ROADMAP / M12 | The "near-ceiling live bytes at 500×150 (TEST-R7 4 MiB)" list is stale. | [ROADMAP](ROADMAP.md) | superseded | P1 | M22 set the default ceiling to 2.75 MiB with measured per-scene exceptions; the 4 MiB figures are historical. |
| B-05 | ROADMAP / M14 | "22 scenes > 2 ms worst at 500×150" is stale. | [ROADMAP](ROADMAP.md), M14 follow-ups | superseded | P1 | M15 measured 17; later style milestones revised the set. GEN-R8 now allows < 5 ms at 500×150; the per-scene 200×50 < 2 ms rule is the enforced one. |
| B-06 | ROADMAP / M10 | "`sciart record` (GIF/asciicast export)" is still listed unscheduled. | [ROADMAP](ROADMAP.md) | done | P1 | Landed in M23 (ADR-0025, CLI-R8); `sciart record` is implemented. |
| B-07 | ROADMAP / M10 | "GitHub Actions CI (+ cargo-deny)" is still listed unscheduled. | [ROADMAP](ROADMAP.md) | done | P1 | Landed in M23 (IMP-04): `.github/workflows/ci.yml` plus `deny.toml` and the CI deny job. |
| B-08 | Aurora / RND-R19 | "Follow-up: a colour-blind-safe variant" is still open. | [ROADMAP](ROADMAP.md) | done | P1 | `Theme::Colorblind` landed in M23 (RND-R19, ADR-0026); reviewed on five scenes. |
| B-09 | ROADMAP / M12 | The M12 closing sentence bundles stale sub-items (500×150 list, 4 MiB list, three stale test names) with live art minors. | [ROADMAP](ROADMAP.md) | open | P1 | Split: B-30/B-31/B-32 are stale/done; the aurora–dna_helix art rows (B-26–B-29) stay open. The roadmap sentence should be rewritten as a link. |
| B-10 | docs index | There is no `docs/README.md`, so the specs and ADRs have no index. | IMP-13 | done | P2 | [docs/README.md](README.md) is the index. |
| B-11 | CONTRIBUTING | CONTRIBUTING has no docs/backlog links and no backlog-review rule. | IMP-13 | done | P2 | CONTRIBUTING links both and has the review rule. |
| B-12 | ROADMAP / M12 | Stale test names `dp_a5_…half_green_half_crimson_bobs`, `orb_a8_…over_the_glow`, and MD-A9/MD-A11 split. | [ROADMAP](ROADMAP.md) | done | P3 | Verified absent from the current scene crates/specs; the names were retired with the M12–M22 renames. |
| B-13 | ROADMAP / M12 | M12 closing sentence says "near-ceiling live bytes" and "500×150 worst frames" as if current. | [ROADMAP](ROADMAP.md) | superseded | P2 | Folded into B-04/B-05/B-30; remove the numbers from ROADMAP. |

## M3 — keep-awake

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-14 | keep-awake / mouse | Manual Accessibility check: `pixi run rs-install-mouse`, run with `--keep-awake mouse --hud`, grant permission, confirm `mouse:on` and a cursor move every 15 s. | [ROADMAP](ROADMAP.md) | open | P2 | Cannot be automated without moving the real cursor; needs a desktop session. |

## M10 — scientific art

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-15 | aurora | Red-crown ramp, thinner spruce ridge, gentler lake ripple, `Glow::red` doc + `ASCII_LAKE_BRIGHT` const, top-fade spec wording. | [ROADMAP](ROADMAP.md) | open | P3 | Six small art/doc items from the second review round. |
| B-16 | solar_surface / core | 2 px polar margin at small sizes, ASCII loop glyphs by pixel pair, interlaced-granulation spec note, and `SpatialHash::reserve` (CORE-R54 amendment) to replace the dummy rebuild. | [ROADMAP](ROADMAP.md) | open | P2 | The `SpatialHash` item is an API change; the rest is scene polish/spec wording. |
| B-17 | smaller scenes | gray_scott ASCII `HALO_DOT` 13→20; electron_orbitals fade window; ising_model ASCII polarity follows the majority; liesegang legend leaves with the first ratio; three_body `MOTH_FULL_SCALE` + triangle clearance; dna_helix zoom-aware rung stride; molecule_viewer rear-bond dotting + tilted planar molecules; physarum third inoculum blur; action_potential dendrite spacing + deterministic third primary; belousov_zhabotinsky free-end spacing. | [ROADMAP](ROADMAP.md) | open | P3 | Ten small, independent art/robustness fixes. |
| B-18 | core / Role | `Role::Spectrum` for true spectral line colour. | [ROADMAP](ROADMAP.md), [ADR-0015](adr/0015-phase-blackbody-named-ramps.md) | won't fix | P3 | Considered for M10/M13; no current scene needs it (ADR-0015 records the decision). Revisit only with a new dispersion scene. |

## M12 — scene upgrades

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-19 | matter scenes | ising_model one chamfer-depth curve for both spins + darker crimson interior; molecular_dynamics streak cap scaled by atom pixel radius at 80×24; fractal_growth pre-grown opening seed + frost tip age gradient; shared `geom::LIGHT`/`geom::lambert` for molecular_dynamics and molecule_viewer. | [ROADMAP](ROADMAP.md) | open | P3 | The Lambert lift needs a fourth scene before it earns a core API (same condition as B-20). |
| B-20 | dynamics scenes | Lift the Lambert sphere shading into core when a fourth scene needs it; the debug-only snapshot item is B-03. | [ROADMAP](ROADMAP.md) | open | P3 | Deferred API lift, not a bug. |
| B-21 | life scenes | physarum thinner channels (raised lattice cap or braille raster) + a pulse when food is reached; action_potential longer/wider ion streaks at ≥ 120 columns; dna_helix helicase rotation tick; boids_flocking braille reeds, Vicsek travelling-band act, and a dot→cell helper once a second scene needs it. | [ROADMAP](ROADMAP.md) | open | P3 | Four art/model upgrades, each needing a look review. |
| B-22 | quanta scenes | mono `Role::Phase` floor near L* 40 so phase-0 dots stay visible; radioactive_decay busy mono lattice at t ≈ 5 s; electron_orbitals glow outline steps per cell under dots; merge `Pixels::cell_bg` with `Pixels::backdrop`. | [ROADMAP](ROADMAP.md) | open | P3 | `Pixels::backdrop` landed (CORE-R86); `cell_bg` is still separate. |
| B-23 | cosmos A | spacetime_grid is a quiet purple lattice between bursts and SPT-A10's "merger + 1 s" became a 3 s window; black_hole mid-disk stays bronze below 6 500 K in `Blackbody`; core `raster::ascii_ramp` + `Tone::value_at` with new CORE-R IDs and a round-trip test. | [ROADMAP](ROADMAP.md) | open | P2 | First two are art/science decisions; the core lifts are still absent (`ascii_ramp_cell` exists, `ascii_ramp`/`value_at` do not). |
| B-24 | voyages | Draw mars_rover's rover/lander/dust/devils and the other Voyages sprites as pixels instead of glyphs; move `Globe` out of satellite_orbit into a shared module (needs a GEN-R3 rule for non-scene files); a fourth review pass for the round-3 art fixes. | [ROADMAP](ROADMAP.md) | open | P2 | `Pixels::backdrop` landed; the sprite and module work did not. |
| B-25 | cosmos B | stellar_nursery pillar heads are geometric semicircles; pulsar radial spokes when the beam turns away; galaxy_collision t = 60 s tails read as strokes; lift the 2-D value noise from stellar_nursery/moon_phases/constellation_map into core; one bilinear four-tap deposit helper for galaxy_collision/globular_cluster. | [ROADMAP](ROADMAP.md) | open | P2 | `rng::value_noise` and `Glow::splat_bilinear` now exist; the noise lift is done, the rest is open. |
| B-26 | closing art | aurora crimson fringe stair-steps at t ≈ 5 s; globular_cluster motion; exoplanet_transit star reads flat; pulsar spikes at t = 5 s. | [ROADMAP](ROADMAP.md) | open | P3 | Four 7.5–8/10 re-scores from the M12 closing review. |
| B-27 | closing art | satellite_orbit label crowding; constellation_map Cassiopeia wash. | [ROADMAP](ROADMAP.md) | open | P3 | Small layout/contrast fixes. |
| B-28 | closing art | double_pendulum twin B's bob is still clipped to its right half while merged; the code and SPEC-002 agree with each other but not the round-2 intent — decide and align. | [ROADMAP](ROADMAP.md) | open | P3 | Needs an art decision: full bob, or keep the half-disc and reword the spec. |
| B-29 | closing art | dna_helix zoom transition at t ≈ 6–8 s. | [ROADMAP](ROADMAP.md) | open | P3 | One transition seam. |
| B-30 | perf list | The M12 "500×150 worst frames over 2 ms" list (doppler_effect 4.7 ms … mars_rover 2.1 ms). | [ROADMAP](ROADMAP.md) | superseded | P1 | GEN-R8's 500×150 limit is < 5 ms; later milestones (M15–M22) re-measured. Keep the 200×50 < 2 ms rule only. |
| B-31 | memory list | The M12 "near-ceiling live bytes (TEST-R7 4 MiB)" list. | [ROADMAP](ROADMAP.md) | superseded | P1 | M22 replaced the 4 MiB ceiling with 2.75 MiB + measured exceptions (B-04). |
| B-32 | stale tests | `dp_a5_…half_green_half_crimson_bobs`, `orb_a8_…over_the_glow`, MD-A9/MD-A11 split. | [ROADMAP](ROADMAP.md) | done | P3 | Verified absent; see B-12. |
| B-33 | merge | Local `--no-ff` merge of `feat/m12-upgrades`. | [ROADMAP](ROADMAP.md) | done | P1 | Merged; M12 is marked done (B-02). |

## M13 — new scenes

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-34 | core / blur | Lift the 1-2-1 blur duplicated by motility_clustering and dicty_aggregation into core. | [ROADMAP](ROADMAP.md) | done | P3 | `raster::Glow::blur121` landed in M14 (CORE-R103); supernova_remnant/light_echo keep their float-different blur. |
| B-35 | radical_chain | CHN-A5 (vent cool-down) passes only for `K_T3 = 0.1` and one seed; make it robust across seeds and nearby constants. | [ROADMAP](ROADMAP.md) | open | P2 | A seed-sensitive acceptance test is a gate risk. |
| B-36 | catalytic_surface | CAT-A6 production stalls on some seeds (seed 4 seen); make the loop robust across seeds. | [ROADMAP](ROADMAP.md) | open | P2 | Same class of gate risk as B-35. |
| B-37 | seashell_pigmentation | Brown false-colour `Blackbody` at 1.6–2.5 kK should become `Ramp::Sienna`. | [ROADMAP](ROADMAP.md) | superseded | P3 | The scene was deleted in M16 (GEN-R25 restyle); no action. |
| B-38 | 13 scene scores | orbital_rendezvous 7, comet_tails 7.5, io_volcanoes 6.5, comet_breakup 7, rainbow 6.5, total_solar_eclipse 7.5, meteor_shower 7.5, noctilucent_clouds 7, catalytic_surface 7.5, motility_clustering 7.5, firefly_sync 7; plus stair-step outlines in heart_fibrillation/hurricane/chemical_garden/jupiter_storms and hp_protein_folding's C tag. | [ROADMAP](ROADMAP.md) | open | P2 | IMP-15 art batch (M23); seashell_pigmentation was deleted in M16. Many were restyled later but not re-scored in one pass. |

## M14 — resource diet

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-39 | 500×150 perf | 22 scenes over 2 ms worst at 500×150 (the M14 follow-up). | M14 follow-ups, [ROADMAP](ROADMAP.md) | superseded | P1 | M15 measured 17; GEN-R8 accepts < 5 ms at 500×150. The 200×50 < 2 ms rule is the one still enforced. |
| B-40 | mars_rover | Sky band and ground still score blockiness 0.10. | M14 follow-ups | open | P2 | One scene over the blockiness ceiling; needs a flat-field/gradient treatment. |
| B-41 | logged art minors | Hurricane eyewall, jupiter_storms limb AA, rainbow band-edge blend, spinodal line beading, molecule_viewer H < C, meteor trail brightness near the head. | M14 follow-ups | open | P3 | Six minor art notes; no blocker. |
| B-42 | low-hanging CPU | dicty_aggregation, molecular_dynamics and heart_fibrillation at ~0.8 ms could only go lower by changing pixels. | M14 follow-ups | won't fix | P3 | Explicitly declined: the cost is the look; no invisible cut exists. |
| B-43 | bench | The bench `worst` table was not revised in M14. | M14 follow-ups | done | P3 | `benches/frame.rs` now has the `WORST` table (8 entries: aurora, chemical_garden, constellation_map, doppler ×2, electron_orbitals, liesegang, radioactive_decay). |

## M15 — M11 look restored

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-44 | 500×150 perf | 17 scenes still exceed 2 ms worst at 500×150 (all < 5 ms). | M15 follow-ups | open | P3 | Within GEN-R8's 5 ms limit; review only if the 500×150 budget is tightened. |
| B-45 | art minors | Braille instead of half-block fronts in heart_fibrillation/dicty_aggregation; io_volcanoes 4 surface tones; comet_tails sunward grain puff at t = 0–5 s; solar_eruption loop tops one Sun step brighter. | M15 follow-ups | open | P3 | Four small art notes. |
| B-46 | science | The reverted scenes lose M12–M14's model/readout improvements by design. | M15 follow-ups | won't fix | P3 | ADR-0018 deliberately restored the M11 look; the model text follows the code. |

## M16 — thread style

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-47 | dicty_aggregation | Stream skeleton holds its layout from ~28 s to the 45 s reset; arms never shorten into mounds; mounds are solid dot blocks. | M16 follow-ups | open | P3 | One scene, three linked look issues. |
| B-48 | spiral_galaxy | Reads as two long arms + inner arcs rather than four; dust lanes faint. | M16 follow-ups | superseded | P3 | M18 redesigned it as six grand-design rosettes; re-check against that look. |
| B-49 | globular_cluster | Blue stragglers are 1–2 single dots; few giants. | M16 follow-ups | open | P3 | Make stragglers 2×2 `Cold` 230. |
| B-50 | saturn_rings | At 80×24 the far-side ring drops out; ansae fragment into arcs. | M16 follow-ups | open | P3 | Small-size framing. |
| B-51 | comet_breakup | Initial coma is a filled dot blob (shrinking it changes MAG/BRK-A6). | M16 follow-ups | open | P3 | Needs a spec/test change, not just art. |
| B-52 | rainbow | In-drop ribbon 3–4 dots wide and hatched; 42°/51° guides faint. | M16 follow-ups | open | P3 | Optical diagram legibility. |
| B-53 | jwst_unfolding | At 80×24 the mirror is a solid gold blob (seam threshold); bus/tower lines fragmentary. | M16 follow-ups | open | P3 | Small-size threshold. |
| B-54 | radical_chain | t ≈ 12 s is sparse; `Cold` wall-death specks line the walls. | M16 follow-ups | open | P3 | Densify the middle act. |
| B-55 | hurricane | Single-thread bands slightly broken; infrared act differs modestly. | M16 follow-ups | open | P3 | One scene, two notes. |
| B-56 | total_solar_eclipse | Long diamond-ring spikes cross the corona at t ≈ 30 s. | M16 follow-ups | open | P3 | Occultation/overdraw. |
| B-57 | solar_eruption | "loops rise" is still busy. | M16 follow-ups | superseded | P3 | M17 restyled the CME; its remaining "faint front at 80×24" is B-67. |
| B-58 | lenia | A few short hatched clumps on creature flanks. | M16 follow-ups | open | P3 | Dot hatch. |
| B-59 | ising_model | The coarsened phase is ~95 % black with 2–3 wall pairs (by design, sparse). | M16 follow-ups | superseded | P3 | M17 moved ising_model to the orbital-thread style; re-evaluate the sparse phase there. |
| B-60 | quantum_double_slit | The transmitted tunnelling wave is exposed like the incident one. | M16 follow-ups | open | P3 | One amplitude/exposure fix. |
| B-61 | spacetime_grid | Vertical lines carry 1-dot jogs; 500×150 worst 2.2 ms. | M16 follow-ups | superseded | P3 | M17 restyled it; the remaining 3.05 ms frame is B-64. |
| B-62 | catalytic_surface | The O-poisoned beat is the sparsest stretch (~2.5k lit px vs 6–18k). | M16 follow-ups | open | P3 | Densify the poisoned beat. |
| B-63 | bench | Add the slowest M16 frames to the `WORST` table if they become a regression risk. | M16 follow-ups | open | P3 | The current `WORST` table covers 8 non-M16 frames; review after the next bench pass. |

## M17 — orbital-thread style

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-64 | core / stipple | A branch-free stipple-mask API (u8 threshold masks) shared by every GEN-R26 scene. | M17 follow-ups | open | P2 | A core lift for CORE-R107/R108 users; needs a CORE-R ID and tests. |
| B-65 | perf | spacetime_grid 500×150 worst 3.05 ms (M16 2.2 ms); ising_model update + render is the costliest of the 10 (0.69 ms). | M17 follow-ups | open | P3 | Both within GEN-R8's 5 ms/2 ms limits; monitor. |
| B-66 | three_body | At the end of Lagrange a few shadow copies scatter around the bodies. | M17 follow-ups | open | P3 | One orbit-end visual. |
| B-67 | gray_scott | Young mitosis spots can look slightly square (the field itself is). | M17 follow-ups | open | P3 | Pixel-field artefact. |
| B-68 | solar_eruption / supernova | CME front faint at 80×24; supernova_remnant clumps form small dense patches. | M17 follow-ups | open | P3 | Two scenes, two small look issues. |
| B-69 | samples | The per-milestone comparison sheets and the samples table were not regenerated for M17, M18, M19/M20 or M21. | M17 follow-ups, M21 follow-ups | superseded | P2 | The per-milestone image archive was retired before 1.0; `docs/GALLERY.md` and `samples/current/` hold one current frame per scene (M23). |

## M18 — orbital molecules

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-70 | firefly_sync | Phase-circle arc faint at R ≈ 1; its 500×150 resize (5.9 ms) is dominated by the salt search. | M18 follow-ups | open | P3 | 5.9 ms is under the ≤ 10 ms resize guidance; the arc is the art issue. |
| B-71 | heart_fibrillation | 500×150 worst frame 3.4 ms (freshly shocked organ); front thread ~1.5 KB/frame. | M18 follow-ups | open | P3 | Within budget; monitor bytes. |
| B-72 | wave_interference | Resize at 500×150 is 4.4 ms (was 1.7 ms). | M18 follow-ups | open | P3 | One-off resize; below the 10 ms guidance. |
| B-73 | chemical_garden | Atoms crowd at 80×24; the vent plume could carry more of the hypothesis' story. | M18 follow-ups | open | P3 | Small-size crowding + one narrative beat. |

## M19 / M20 — star dust, planets, Brownian motion, nerve spike

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-74 | galaxy_collision | Busiest gallery scene (829 changed cells/frame); trim the dust level or star cap if RT-R16's ≤ 880-cell mean gets tight. | M19/M20 follow-ups | open | P2 | 829 < 880, but the headroom is small; revisit if RT-R16 is tightened. |

## M21 — ten new scenes

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-75 | cloud_chamber | Sparse at 120×40 (2–3 tracks). | M21 follow-ups | done | P3 | Post-merge fix: rate scaling + live-track floor, art band re-seeded. |
| B-76 | ribosome_translation | At sample size the folded α-helix reads as a beaded zigzag; a tighter/more frontal projection would read better. | M21 follow-ups | open | P2 | Representation, not just polish. |
| B-77 | protocell_assembly / binary_inspiral | Protocell is the busiest new scene (10.7 KB, 672 cells at 200×50; 26 KB at 500×150); binary_inspiral is heaviest at 500×150 (42 KB, 4.7k cells); no test covers 500×150. | M21 follow-ups | open | P2 | Add a 500×150 budget probe/test or cap the scenes. |
| B-78 | magnetosphere | Keeps its own small half-block Earth disc; lift `raster::disc_cells` to core only if a second scene needs it. | M21 follow-ups | open | P3 | Conditional API lift. |
| B-79 | perf | Quiet-machine `rs-worst` re-check. | M21 follow-ups | open | P2 | The M21 numbers were measured on a loaded machine. |

## M22 — lean build and runtime

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-80 | encoder | Manual encoder check in iTerm2, Terminal.app and tmux: VT down-moves and colour grouping without synchronized output. | M22 follow-ups, [ROADMAP](ROADMAP.md) | open | P2 | Needs real terminals; pty tests cannot cover sync-update behaviour. |
| B-81 | encoder perf | Bypass ratatui's buffer diff (~90 % of diff+encode at 500×150) with our own larger cut. | M22 follow-ups | open | P3 | A larger change, only if 500×150 becomes a target. |
| B-82 | CI/platform | Add `linux-64` to the pixi platforms (`pixi.toml` currently `osx-arm64` only); both sccache and nextest exist on conda-forge. | M22 follow-ups | open | P3 | CI already tests Linux with cargo; this makes local pixi tasks work there too. |

## M23 — improvement programme

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-83 | SPEC-002 split | Split the scene sections into five category files with a generated index and a `scripts/spec_index.py`. | [ROADMAP](ROADMAP.md) | done | P1 | M23: 25/18/14/14/14 sections moved; the monolith keeps the rules and an 85-row index. |
| B-84 | backlog triage | Triage the open roadmap boxes and M12–M22 follow-ups into this file; close/annotate stale boxes. | [ROADMAP](ROADMAP.md), IMP-13 | done | P1 | This file is the triage; the roadmap boxes link here. |
| B-85 | fuzzing | `cargo-fuzz` targets for `config::parse`, `title::fit`/`title::split`, `Hud::fit` and `Theme::from_name`; weekly job, crashes become regression tests. | IMP-14 | done | P2 | M23: `fuzz/` with `title_fit`, `hud_fit`, `theme_name` + weekly workflow; config has a 64-case proptest (binary-only module). The first hosted run is pending. |
| B-86 | M23 art | Re-score and polish the below-8 scenes. | [ROADMAP](ROADMAP.md) | open | P2 | IMP-15; the M13 art list (B-38) is the input. |
| B-87 | M23 gallery check | A look check of every changed scene at each milestone close. | [ROADMAP](ROADMAP.md) | open | P2 | IMP-16; the checklist is in [CONTRIBUTING](../CONTRIBUTING.md#reviewing-a-changed-scene). |
| B-88 | M23 science | Science-claims audit table for every scene. | [ROADMAP](ROADMAP.md) | open | P2 | IMP-17; every scene gets a row and a review date. |

## M9 — future

| ID | Area | Issue | Source | Status | Priority | Note |
| --- | --- | --- | --- | --- | --- | --- |
| B-89 | keep-awake | Windows keep-awake backend. | [ROADMAP](ROADMAP.md) | open | P3 | Linux landed in M25 (KA-R7..R9); Windows is out of scope for 1.0. |
| B-96 | keep-awake | Run `--keep-awake inhibit` on a Linux desktop with logind: the lock appears in `systemd-inhibit --list`, blocks suspend, and is gone after `q`, `kill` and `kill -9`. | [ROADMAP](ROADMAP.md) | open | P1 | No test can fail on the real helper (it needs logind and polkit); the helper choice, the pipe and the defaults are tested with a stand-in. |
| B-90 | themes | Light-background theme. | [ROADMAP](ROADMAP.md) | open | P3 | Not scheduled. |
| B-91 | release | Per-OS release packaging beyond the 1.0 channels. | [ROADMAP](ROADMAP.md) | open | P3 | M25 added an installer script, a conda package and a Homebrew tap beside the tarballs (SPEC-008); distribution packages such as apt or rpm remain. |
| B-92 | art / spec | `catalytic_surface`'s late uniform-CO beat is a rust ring around a black interior; a dim interior stipple would read better but CAT-A9 forbids it. | M23 art pass | open | P1 | Needs a spec decision (allow a dim interior after the opening, or accept the ring), not a parameter tweak. |
| B-93 | science / GEN-R12 | `solar_surface` title says "boiling granules"; granulation is convection. | [claims](science/claims.md) | open | P2 | Proposed "Solar surface · convection cells"; the scene is M11-pinned, so a title change needs a GEN-R22 exception. |
| B-94 | science / GEN-R12 | `molecular_dynamics` title says "cool atoms"; the model is Lennard-Jones sites. | [claims](science/claims.md) | open | P2 | Proposed "Condensation · cool particles cling"; M11-pinned, same fixture decision as B-93. |
| B-95 | art | `io_volcanoes`' faint fixed Pele guide dome is a static element under the live grains. | M23 art pass | open | P3 | Deliberate silhouette aid; remove if a future review prefers pure live grains. |

## Notes

- **B-01/B-83 closed in M23.** The SPEC-002 split landed with a generated index; the docs
  index can link the category files, and `scripts/spec_index.py --check` guards the table.
- **B-35/B-36 next.** Seed-dependent acceptance tests are a gate risk; they are cheap to make
  robust and expensive to rediscover at a release.
- **B-14/B-80** are the only items that require a real desktop/terminal; everything else can be
  done from CI or a pty.
- The per-milestone image archive is retired (B-69); the current-frame gallery is generated
  (`docs/GALLERY.md`).
