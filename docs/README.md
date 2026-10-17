# sciart documentation

Start with the [project README](../README.md) to install and run the art, and
[CONTRIBUTING](../CONTRIBUTING.md) for the engineering rules. This page is the map of the docs tree:
what each document is for, and which one is authoritative.

## Start here

| If you want to… | Read |
| --- | --- |
| install, run, or see the art | [README](../README.md) · [gallery](GALLERY.md) |
| work on the code | [CONTRIBUTING](../CONTRIBUTING.md) |
| know what the project promises | [master specification](specs/sciart-specification.md) |
| know why a decision was made | [ADRs](adr/) |
| know what is still open | [backlog](BACKLOG.md) · [roadmap](ROADMAP.md) |
| see the improvement programme | [IMPROVEMENTS](IMPROVEMENTS.md) |

## Specifications

The spec is the current contract: if code and spec disagree, the spec is the bug report. ADRs
are records of decisions, including historical ones — they are not amended retroactively.

| Document | What it covers |
| --- | --- |
| [Master specification](specs/sciart-specification.md) | Goals, architecture, F1–F9 scope, performance/testing constraints |
| [SPEC-001 core](specs/SPEC-001-core.md) | Core types and helpers: canvas, generator contract, fixed step, geometry, raster, RNG, ODE |
| [SPEC-002 generators](specs/SPEC-002-generators.md) | Generator/registry rules, titles and taxonomy, and the per-scene requirements; the scene sections are being split into category files ([backlog B-01](BACKLOG.md)) |
| [SPEC-003 runtime & render](specs/SPEC-003-runtime-render.md) | Runtime state machine, keys, help/favourites, HUD, themes, encoder |
| [SPEC-004 keep-awake](specs/SPEC-004-keepawake.md) | `caffeinate`/mouse/off backends and the jiggler |
| [SPEC-005 CLI & config](specs/SPEC-005-cli-config.md) | Commands, flags, TOML precedence, colour/speed/filter settings |
| [SPEC-006 testing](specs/SPEC-006-testing.md) | Test layers, the `rs-ci` gate, manual checks and traceability |

## Architecture decision records

Each ADR is a point-in-time decision with context, alternatives and consequences. Later ADRs
amend or supersede earlier ones; the index notes the relationship.

| ADR | Decision |
| --- | --- |
| [0000](adr/0000-template.md) | Template for a new ADR |
| [0001](adr/0001-cargo-workspace.md) | Cargo workspace (the five-crate count is historical; ADR-0023 split the workspace further) |
| [0002](adr/0002-ratatui-crossterm.md) | ratatui + crossterm for terminal output |
| [0003](adr/0003-fixed-step-simulation.md) | Fixed-step simulation decoupled from the frame rate |
| [0004](adr/0004-semantic-palettes.md) | Semantic inks and themes instead of per-character colours |
| [0005](adr/0005-keep-awake-backends.md) | Selectable keep-awake backends, `caffeinate` by default |
| [0006](adr/0006-panic-quarantine.md) | `catch_unwind` and per-session quarantine for generator panics |
| [0007](adr/0007-pixi-task-runner.md) | pixi as the single task runner |
| [0008](adr/0008-injected-rng.md) | Injected seeded RNG, no global randomness |
| [0009](adr/0009-allocation-tests.md) | Allocation tests through a test-only counting allocator |
| [0010](adr/0010-layered-raster.md) | Braille dots and half-block pixels as layered sub-cell rasters |
| [0011](adr/0011-colormap-roles.md) | Sequential and diverging colormap roles (amends ADR-0004) |
| [0012](adr/0012-decoupled-sim-lattice.md) | Capped simulation lattices, decoupled from the display |
| [0013](adr/0013-pxcxb-taxonomy.md) | Physics × chemistry × biology taxonomy (supersedes the early categories) |
| [0014](adr/0014-central-science-titles.md) | Science titles drawn centrally by `Stage` |
| [0015](adr/0015-phase-blackbody-named-ramps.md) | Phase, blackbody and named ramp roles (amends ADR-0004/0011) |
| [0016](adr/0016-profile-independent-trig.md) | Profile-independent trigonometry, so frames do not depend on codegen |
| [0017](adr/0017-clean-minimal-style.md) | Clean minimal style (superseded by ADR-0018) |
| [0018](adr/0018-m11-look-restored.md) | The M11 look restored (supersedes ADR-0017) |
| [0019](adr/0019-thread-style.md) | Thread style for selected scenes (amends ADR-0018) |
| [0020](adr/0020-orbital-thread-style.md) | Orbital-thread style (amends ADR-0019) |
| [0021](adr/0021-orbital-molecules.md) | Molecules in the orbital-thread style (amends ADR-0020) |
| [0022](adr/0022-aurora-theme.md) | The `aurora` colour theme |
| [0023](adr/0023-lean-build-and-runtime.md) | Lean build and runtime: nextest, `ci` profile, sccache, category crates |
| [0024](adr/0024-hosted-ci-and-release.md) | Hosted CI and the tagged release pipeline |
| [0025](adr/0025-capture-and-record.md) | Headless capture and `sciart record` |
| [0026](adr/0026-colour-capability.md) | Colour capability, `NO_COLOR`, and the colour-blind-safe theme |

## Design notes

Design notes explain how a milestone was built and why it looks the way it does. They are
historical context; the spec is still the contract.

| Document | What it covers |
| --- | --- |
| [M10 scientific art batch](design/M10-scientific-art-batch.md) | The 16-scene Batch E, colormap roles, capped lattices, 3-D cameras |
| [M11 titles & taxonomy](design/M11-titles-taxonomy.md) | Title system, P×C×B categories, the M12/M13 scene plans |
| [M11 research](design/M11-research/) | Curator proposals, taxonomy and title-system research (raw input, not a spec) |
| [M13 scenes](design/M13-scenes.md) | Proposals and implementer batches for the M13 scenes |
| [M14 briefs](design/M14-briefs.md) | Per-scene art briefs and the review log |
| [M14 resource diet](design/M14-resource-diet.md) | The CPU/bytes/memory diet: before/after measurements, follow-ups |
| [M15 M11 look](design/M15-m11-look.md) | The return to the M11 idiom, restyle rules and follow-ups |
| [M15 thin colour style](design/M15-thin-colour-style.md) | The earlier thin-colour proposal, not chosen (superseded by ADR-0018) |
| [M16 thread style](design/M16-thread-style.md) | Braille-thread restyle, GEN-R25, per-scene choices |
| [M17 orbital-thread](design/M17-orbital-thread.md) | Stipple + thread restyle, GEN-R26, orbit designs |
| [M18 orbital molecules](design/M18-orbital-molecules.md) | Molecule rule, stipple atoms and bond threads |
| [M19/M20](design/M19-M20.md) | Star dust, planets, Brownian motion, nerve spike, aurora theme, colour-blind follow-up |
| [M21 ten scenes](design/M21-ten-scenes.md) | Proposals, looks and follow-ups for the last ten scenes |
| [M22 lean build](design/M22-lean-build-runtime.md) | Build/runtime measurements, decisions and follow-ups |
| [M23 programme](design/M23-programme.md) | The 17 improvement items and the phase plan |

## Reference and process

| Document | What it covers |
| --- | --- |
| [Gallery](GALLERY.md) | One current frame per registered scene, grouped by category |
| [Roadmap](ROADMAP.md) | Milestones M0–M23, done/not-done, links to designs |
| [Backlog](BACKLOG.md) | Open follow-ups and stale claims, with status and priority |
| [Improvements](IMPROVEMENTS.md) | The 17-item improvement proposal (IMP-01..17) and its evidence |
| [Contributing](../CONTRIBUTING.md) | Setup, the `rs-ci` gate, SDD/TDD, adding a scene, snapshots, backlog review |
| [Config example](config.example.toml) | Every config key with its default and a comment |
| [Samples](../samples/README.md) | The historical per-scene/per-milestone PNG archive and compare sheets |
| [Archive](archive/) | Python-era documents (context only — not a spec) |

## Keeping the docs true

- A behaviour change starts in a spec, not in a design note ([CONTRIBUTING](../CONTRIBUTING.md)).
- ADRs are records: do not edit a past decision; add a new ADR that amends or supersedes it.
- The roadmap is milestone-level. Open detail lives in the [backlog](BACKLOG.md); every
  milestone close reviews it, and no item survives two milestones without a decision.
- If a document links to a file that does not exist yet, the split is in progress: see the
  backlog rather than trusting the link.
