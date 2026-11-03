# sciart documentation

Start with the [project README](../README.md) to install and run the art. This page is the
map of the docs tree: what each document is for, and which one is authoritative.

## Start here

| If you want to… | Read |
| --- | --- |
| install, run, or see the art | [README](../README.md) · [user guide](guide/index.md) · [gallery](GALLERY.md) |
| work on the code | [CONTRIBUTING](../CONTRIBUTING.md) · [ARCHITECTURE](ARCHITECTURE.md) |
| know what the project promises | [master specification](specs/sciart-specification.md) |
| know why a decision was made | [ADRs](adr/) |
| know what is still open | [backlog](BACKLOG.md) · [roadmap](ROADMAP.md) |
| see what changed in a release | [changelog](../CHANGELOG.md) |

## User guide

The guide in [docs/guide/](guide/) is built into the docs site in the order of
[chapters.toml](guide/chapters.toml) (WEB-R5).

| Chapter | What it covers |
| --- | --- |
| [Introduction](guide/index.md) | What sciart is and where to start |
| [Installing](guide/installing.md) | Getting the binary, shell completions and the manual page |
| [Running](guide/running.md) | Cycling the scenes or running one, choosing what to cycle, themes, seed, speed and frame rate |
| [Keys](guide/keys.md) | The runtime keys |
| [Configuration](guide/configuration.md) | The config file, with a few recipes |
| [Colour and terminals](guide/terminals.md) | Colour modes, colour-blind viewing, the background, fonts, keep-awake and platforms |
| [Scripting](guide/scripting.md) | `--for`, `while`, `frame`, `list --json`, completions and man pages |
| [Recording](guide/recording.md) | `sciart record`: GIF, PNG and asciicast |
| [The library](guide/library.md) | Using the `sciart` crate from Rust |

## Contributing and architecture

| Document | What it covers |
| --- | --- |
| [Contributing](../CONTRIBUTING.md) | Setup, every command, the `rs-ci` gate, spec-first and test-first work, adding a scene, snapshot and scene review, conventions, backlog review |
| [Architecture](ARCHITECTURE.md) | The crate map, dependency direction, the generator contract and its rules, the look rules, which test suite enforces what, platform tiers and environment notes |

## Specifications

The spec is the current contract: if code and spec disagree, the spec is the bug report. ADRs
are records of decisions, including historical ones — they are not amended retroactively.

| Document | What it covers |
| --- | --- |
| [Master specification](specs/sciart-specification.md) | Goals, architecture, F1–F9 scope, performance/testing constraints |
| [SPEC-001 core](specs/SPEC-001-core.md) | Core types and helpers: canvas, generator contract, fixed step, geometry, raster, RNG, ODE |
| [SPEC-002 generators](specs/SPEC-002-generators.md) | Registry/stability rules, titles, taxonomy, the look rules and the generated scene index |
| [SPEC-002 scenes](specs/SPEC-002-scenes-cosmos.md) | Per-scene sections by category: [cosmos](specs/SPEC-002-scenes-cosmos.md) · [earth](specs/SPEC-002-scenes-earth.md) · [physics](specs/SPEC-002-scenes-physics.md) · [matter](specs/SPEC-002-scenes-matter.md) · [life](specs/SPEC-002-scenes-life.md) |
| [SPEC-003 runtime & render](specs/SPEC-003-runtime-render.md) | Runtime state machine, keys, help/favourites, HUD, themes, the changed-cell pipeline and encoder |
| [SPEC-004 keep-awake](specs/SPEC-004-keepawake.md) | `caffeinate`, Linux inhibitor, mouse and off backends, the jiggler |
| [SPEC-005 CLI & config](specs/SPEC-005-cli-config.md) | Commands, flags, TOML precedence, colour/speed/filter settings, scriptable modes |
| [SPEC-006 testing](specs/SPEC-006-testing.md) | Test layers, the `rs-ci` gate, platform tiers, manual checks and traceability |
| [SPEC-007 library](specs/SPEC-007-library.md) | The `sciart` library facade: `Stage`, scenes, text/ANSI/PNG output, the ratatui widget |
| [SPEC-009 web and docs](specs/SPEC-009-web-docs.md) | The animated showcase, the browser player, the web gallery and the docs site |

## Architecture decision records

Each ADR is a point-in-time decision with context, alternatives and consequences. Later ADRs
amend or supersede earlier ones; the index notes the relationship.

| ADR | Decision |
| --- | --- |
| [0000](adr/0000-template.md) | Template for a new ADR |
| [0001](adr/0001-cargo-workspace.md) | Cargo workspace (the five-crate count is historical; ADR-0023 split the workspace further) |
| [0002](adr/0002-ratatui-crossterm.md) | ratatui + crossterm for terminal output |
| [0003](adr/0003-fixed-step-simulation.md) | Fixed-step simulation, decoupled from the frame rate |
| [0004](adr/0004-semantic-palettes.md) | Semantic inks and themes instead of per-character random colour |
| [0005](adr/0005-keep-awake-backends.md) | Selectable keep-awake backends, `caffeinate` by default |
| [0006](adr/0006-panic-quarantine.md) | `catch_unwind` and quarantine for generator panics |
| [0007](adr/0007-pixi-task-runner.md) | pixi as the single task runner |
| [0008](adr/0008-injected-rng.md) | Injected seeded RNG, no global randomness |
| [0009](adr/0009-allocation-tests.md) | Allocation tests via a test-only counting allocator crate |
| [0010](adr/0010-layered-raster.md) | Layered sub-cell raster: braille dots and half-block pixels |
| [0011](adr/0011-colormap-roles.md) | Sequential and diverging colormap roles (amends ADR-0004) |
| [0012](adr/0012-decoupled-sim-lattice.md) | Capped simulation lattices, decoupled from the display, resampled on resize |
| [0013](adr/0013-pxcxb-taxonomy.md) | Physics × chemistry × biology taxonomy (supersedes the M1/M10 categories) |
| [0014](adr/0014-central-science-titles.md) | Science titles drawn centrally by `Stage` |
| [0015](adr/0015-phase-blackbody-named-ramps.md) | Phase, blackbody and named ramp roles (amends ADR-0004, ADR-0011) |
| [0016](adr/0016-profile-independent-trig.md) | Profile-independent trigonometry |
| [0017](adr/0017-clean-minimal-style.md) | Clean minimal style (superseded by ADR-0018) |
| [0018](adr/0018-m11-look-restored.md) | M11 look restored (supersedes ADR-0017; amends ADR-0011, ADR-0015) |
| [0019](adr/0019-thread-style.md) | Thread style for selected scenes (amends ADR-0018) |
| [0020](adr/0020-orbital-thread-style.md) | Orbital-thread style: hydrogen clouds × strange-attractor threads (amends ADR-0019) |
| [0021](adr/0021-orbital-molecules.md) | Molecules in the orbital-thread style (amends ADR-0020) |
| [0022](adr/0022-aurora-theme.md) | Aurora colour theme |
| [0023](adr/0023-lean-build-and-runtime.md) | Lean build and runtime: nextest, `ci` profile, sccache, category crates |
| [0024](adr/0024-hosted-ci-and-release.md) | Hosted CI and a tagged release pipeline |
| [0025](adr/0025-capture-and-record.md) | Headless capture and `sciart record` |
| [0026](adr/0026-colour-capability.md) | Colour capability, `NO_COLOR`, and a colour-blind-safe theme |
| [0027](adr/0027-opaque-background.md) | Opaque black background by default |
| [0028](adr/0028-linux-inhibitor-helper.md) | Linux keep-awake through an inhibitor helper process |
| [0029](adr/0029-platform-tiers-and-determinism.md) | Platform tiers and per-platform determinism |
| [0030](adr/0030-changed-cell-pipeline.md) | Changed-cell paint and diff; event-driven pause |
| [0031](adr/0031-scriptable-modes.md) | Scriptable modes: a deadline, one frame, and running beside a command |
| [0032](adr/0032-library-facade.md) | The `sciart` library facade |
| [0034](adr/0034-web-gallery-and-docs-site.md) | A web gallery and a docs site |
| [0035](adr/0035-release-profile-and-dependency-trim.md) | Release profile and dependency trim |

ADR-0033 (distribution) is reserved for the open distribution phase of M25.

## Reference

| Document | What it covers |
| --- | --- |
| [Gallery](GALLERY.md) | One current frame per registered scene, grouped by category, and the animated clips (generated by `scripts/gallery.py`) |
| [Science claims](science/README.md) | Per-scene title/model/caveat audit ([claims table](science/claims.md); 20 audited, 65 pending) |
| [Roadmap](ROADMAP.md) | Milestones M0–M25, done and not done, with requirement IDs and measurements |
| [Backlog](BACKLOG.md) | Open follow-ups and stale claims, with status and priority |
| [Changelog](../CHANGELOG.md) | User-visible changes per release |
| [Showcase manifest](showcase.toml) | Every animated clip in the README and the gallery, with its recording parameters (WEB-R1); the clips are in [docs/media/](media/) |
| [Config example](config.example.toml) | Every config key with its default and a comment |

## Keeping the docs true

- A behaviour change starts in a spec ([CONTRIBUTING](../CONTRIBUTING.md#spec-driven-test-driven)).
- ADRs are records: do not edit a past decision; add a new ADR that amends or supersedes it.
- The roadmap is milestone-level. Open detail lives in the [backlog](BACKLOG.md); every
  milestone close reviews it, and no item survives two milestones without a decision.
- The gallery, the scene index in SPEC-002 and the claims table are generated or checked
  against the registry; regenerate them rather than editing by hand.
