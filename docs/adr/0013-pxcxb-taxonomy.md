# ADR-0013: Physics × chemistry × biology taxonomy (supersedes the M1/M10 categories)

- **Status**: Accepted
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

1. **Setting.** A deep-space subject → Deep Cosmos; the Sun, planets, moons or comets → Sun &
   Planets; seen from Earth's surface or atmosphere → Earth & Sky; a human spacecraft or mission
   → Voyages.
2. **Axes.** Tag the model that fills most of the screen time (an act never moves a scene):
   - **B**: a named organism, tissue, cell, working biomolecule, proto-cell or artificial life;
   - **C**: species-transforming kinetics, fuel turnover, molecular or electronic structure as
     the subject, or a phase change;
   - **P**: a law beyond the diffusion that carries a chemistry: force and mechanics, flow,
     circuits and fields, free-energy or stat-mech sampling, quantum mechanics, waves and optics.

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
  "electronic structure as the subject", plus P for quantum mechanics → States of Matter).

## Alternatives considered
The P×C×B lattice alone, with astronomy as P (it would swamp Forces & Waves); the lattice plus a
"Patterns & Chaos" category (overlaps P and has no rule of its own); keeping the old categories
with more added (no rule, so every new scene is a judgement call).
