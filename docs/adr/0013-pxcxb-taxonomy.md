# ADR-0013: Physics × chemistry × biology taxonomy (supersedes the M1/M10 categories)

- **Status**: Accepted; amended 2026-09-29 (M13, see Addendum)
- **Date**: 2026-09-27

## Context
Categories grew by accretion: `Astronomy`, `SpaceExploration`, `PhysicsChemistry`, `Basic`, then
`Biology` and `Chemistry` (CORE-R47). `PhysicsChemistry` kept scenes that are really about matter
changing phase next to pure mechanics; `Basic` held only `starfield`; `Astronomy` held 17 scenes.
The categories drive `sciart list` and, from M11, the auto-cycle order (RT-R13), and M13 adds two
scenes per category. That needs a rule that puts any scene, new or old, in exactly one category.

## Decision
Replace `Category` with eleven variants (CORE-R77): four sky settings, plus every non-empty
combination of the physics (P), chemistry (C) and biology (B) axes. Classify by the first rule
that matches:

1. **Setting** (checked in this order). A human spacecraft or mission → Voyages; otherwise a
   view from Earth's surface or atmosphere, or of Earth's weather or atmosphere from above
   (M13 addendum) → Earth & Sky; otherwise the Sun, or any planet, moon
   or comet (ours or another star's) → Sun & Planets; otherwise a deep-space subject → Deep
   Cosmos. The order settles scenes that match several settings: moon_landing and mars_rover
   are missions before they are the Moon or Mars, moon_phases is a view from Earth before it is
   the Moon, and exoplanet_transit is a planet before it is deep space.
2. **Axes.** Tag the model that fills most of the screen time (an act never moves a scene):
   - **B**: a named organism, tissue, cell, working biomolecule, proto-cell or artificial life;
   - **C**: kinetics that transform chemical species (atoms regrouping into molecules or ions;
     nuclear transmutation is P), fuel turnover, molecular or electronic structure as the
     subject, or a phase change (including ordering transitions such as magnetisation);
   - **P**: a law beyond diffusion that only carries a chemistry (diffusion counts as P when
     the transport itself sets the shape, as in Laplacian growth): force and mechanics, flow,
     circuits and fields, free-energy or stat-mech sampling, quantum mechanics, waves and optics,
     or deterministic chaos in a continuous flow (an ODE system in phase space).

   Behavioural rules are neither P nor C. Drop any tag whose removal leaves the scene working.
3. **Map.** P → Forces & Waves · C → Reactions · B → Living Systems · P×C → States of Matter ·
   P×B → Physics of Life · C×B → Molecules of Life · P×C×B → Life from Matter.

Each variant has a kebab-case ASCII `id()` for machine output and an evocative `label()` for
people. SPEC-002 headings record every scene's label, and GEN-R15 checks them against the code.
The assignment of the 41 existing scenes is the table in `docs/design/M11-titles-taxonomy.md` §2.

## Consequences
+ Every future scene has one category by rule, not by taste; the lattice shows where the gaps
  are (Life from Matter is empty until M13).
+ The auto-cycle can interleave categories so consecutive scenes contrast (RT-R13).
− `sciart list` output changes (category column), and `Basic`, `PhysicsChemistry`,
  `Astronomy` and `SpaceExploration` disappear, so anything parsing the old ids breaks.
− Borderline scenes need the rule applied carefully (e.g. `electron_orbitals` is C by
  "electronic structure as the subject", plus P for quantum mechanics → States of Matter;
  `fractal_growth` is P (Laplacian growth: the walkers sample the diffusion field's harmonic
  measure, and that transport, not a reaction, sets the fractal) × C (deposition, a phase
  change) → States of Matter, while `liesegang_rings` stays C because its bands are set by
  reaction and nucleation thresholds, with diffusion only supplying the species;
  `ising_model` is P (Glauber sampling) × C (an ordering transition) → States of Matter; `brownian_motion` is P only (Langevin
  mechanics: no species, fuel, structure or phase change) → Forces & Waves; and
  `radioactive_decay` is P, since nuclear transmutation is not chemistry).
− Categories are unequal in size (Forces & Waves and Deep Cosmos hold 9 scenes each, most
  others 1–4), so the round-robin cycle order (RT-R13) ends each lap alternating between the
  two big ones, and the wrap from the last scene to the first may repeat a category. Accepted
  for M11. If it matters as M13 grows the big categories, deal scenes by proportional spacing
  instead: sort by `(rank_in_group + 0.5) / group_len`, ties broken by the shuffled category
  order; that keeps the permutation, determinism and adjacency properties.

## Alternatives considered
The P×C×B lattice alone, with astronomy as P (it would swamp Forces & Waves); the lattice plus a
"Patterns & Chaos" category (overlaps P and has no rule of its own); keeping the old categories
with more added (no rule, so every new scene is a judgement call).

## Addendum (2026-09-29, M13): Earth seen from above

The Earth & Sky setting rule now reads: a view from Earth's surface or atmosphere, **or of Earth's
weather or atmosphere from above** → Earth & Sky. `hurricane` (a storm seen from orbit) is the
case that prompted it: its subject is Earth's atmosphere, not a planet as an astronomical body, so
it sits with aurora and noctilucent clouds rather than in Sun & Planets. The order of the setting
check is unchanged: a scene about a human spacecraft or mission is still Voyages first.
