# SPEC-009: Showcase, web gallery and docs site

**Milestone**: M25 · **Status**: WEB-R1/R2 (showcase) in progress; WEB-R3..R8 (web gallery, docs site) are specified with phase F2

The README, the gallery and the docs site show the art moving. Every animated capture is
recorded from the binary, from a manifest, so it can be regenerated and never drifts from
what the program draws.

- **WEB-R1** (M25) Showcase manifest: `docs/showcase.toml` lists the animated captures. Each entry has an `id`, the scene name, seed, size in cells, seconds, fps, theme and the output path under `docs/media/`. Every clip is produced by `sciart record` with exactly those parameters; `pixi run rs-showcase` records the missing ones (and all of them with `--force`). No clip is edited by hand or captured by another tool. A GIF clip is at most 1.5 MB and the whole set at most 16 MB. The set covers at least 12 scenes from at least 8 categories and includes the thread idiom (GEN-R25), the orbital-thread stipple idiom (GEN-R26) and a half-block field scene.
- **WEB-R2** (M25) `scripts/showcase.py --check` (`pixi run rs-showcase-check`, and CI) fails when a manifest clip is missing; a file under `docs/media/` is not in the manifest; a clip or the set is over its size limit; a manifest scene is not registered; the coverage of WEB-R1 is not met; or `README.md` or a generated docs page references a media file that is not in the manifest. `README.md` shows one hero clip and a grid of at least eight clips, each with alt text naming the scene and what it shows; `docs/GALLERY.md` has an animated section, written by its generator, with every clip.
