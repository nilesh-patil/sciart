# Science claims

The [claims table](claims.md) is a per-scene audit of what each title says, what the model
actually does, and where the art exaggerates or simplifies it. It exists so the project's
science honesty is reviewable at a glance, not buried in review threads.

## Status

- **85 rows** — every registered scene, generated from `sciart list`, each scene's
  `META.title`, and the SPEC-002 category files.
- **20 audited** (review date 2026-10-18): `black_hole`, `cosmic_microwave_background`,
  `gravitational_lensing`, `spacetime_grid`, `pulsar`, `quantum_double_slit`,
  `molecular_dynamics`, `ribosome_translation`, `binary_inspiral`, `solar_eruption`,
  `magnetosphere`, `solar_surface`, `aurora`, `action_potential`, `ising_model`,
  `exoplanet_transit`, `moon_phases`, `total_solar_eclipse`, `rainbow`, `solar_system`.
- **65 unaudited** — their model sentence comes from the spec, and `—` marks the caveat and
  review date as not yet reviewed. `—` is acceptable **only** outside the audited 20; an
  audited row always has a caveat and a date.

The table is a review artefact, not a spec. If a scene's model or title changes, update its row
in the same change; do not edit a SPEC to match this table.

## Flagged titles

Two audited titles claim more than the model does. The proposed wording is in the row and
repeated here; the spec is intentionally untouched (title changes go through the spec):

| Scene | Current title | Problem | Proposed |
| --- | --- | --- | --- |
| `solar_surface` | Solar surface · boiling granules | Granulation is convection, not boiling | Solar surface · convection cells |
| `molecular_dynamics` | Gas to liquid · cool atoms cling | The particles are Lennard-Jones sites, not atoms | Condensation · cool particles cling |

Everything else in the audited set reads as a claim the model supports, with the caveat column
recording the stylisation (2-D models, compressed time, false colour, illustrative profiles).

## How to add or review a row

1. Take the scene's `META.title` from its source and its `Model`/`Stylisation` from the
   SPEC-002 category file (`docs/specs/SPEC-002-scenes-<crate>.md`).
2. Write one model sentence: the mechanism the scene actually simulates, not the phenomenon it
   evokes.
3. Write the caveat: what is 2-D, compressed, false-colour, illustrative, or deliberately
   exaggerated; if the title overclaims, say so and propose replacement wording.
4. Add the review date (`YYYY-MM-DD`).
5. Do not edit a SPEC to match the table; propose the wording in the row/report instead.

## Regenerating and checking

`scripts/science_claims.py` is a stdlib-only helper:

```sh
# print a fresh skeleton (one row per registered scene)
python3 scripts/science_claims.py

# write it; refuses to overwrite a filled table unless --force
python3 scripts/science_claims.py --write

# verify every registered scene still has a row (used by the doc review)
python3 scripts/science_claims.py --check
```

The `--check` mode only verifies coverage; it never rewrites the audited rows. Re-run it after
adding or renaming a scene, and at each milestone close ([backlog](../BACKLOG.md) B-86/B-88).

## Limitations

- This is a focused audited sample, not an external peer review. The unaudited rows still need
  the same treatment.
- The model sentence is from the spec/implementation; if they disagree, the spec is the bug
  report and the row should say so.
- The claims are about the visible model, not every physical effect the real phenomenon has;
  "stylisation" is where the difference lives.
- The table follows the registry: a renamed or removed scene is a new/removed row, and the
  `--check` script catches drift.
