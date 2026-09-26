# SPEC-002: Generators & registry

**Crate**: `crates/sciart-generators` · **Milestones**: M1 (registry, starfield), M4–M7 (batches) · **Status**: M1 implemented

Ports the Python stability contract (`docs/issue-01-simulator-stability.md`, R1) to Rust
and extends it for real-time animation.

## Registry
- **GEN-R1** `registry() -> &'static [Entry]` lists every generator; `Entry { meta: &'static Meta, build: fn() -> Box<dyn Generator> }`. Names are unique, `snake_case`, and sorted.
- **GEN-R2** `find(name) -> Option<&'static Entry>` is exact-match on `meta.name`.
- **GEN-R3** Adding a generator requires exactly: one module file + one line in the registry macro. The contract suite picks it up automatically.

## Stability contract (every registered generator)
- **GEN-R4** Never panics for any `Size` with `w, h >= 1` across any sequence of `resize`, `update(dt)` (`0 <= dt <= 0.25`), and `render` calls — including resizes between any two calls.
- **GEN-R5** Dimension-dependent state is rebuilt in `resize`; `update`/`render` must tolerate the smallest size (1×1).
- **GEN-R6** Determinism: same seed + same call sequence ⇒ byte-identical `Canvas::to_lines()`.
- **GEN-R7** No wall-clock reads (`Instant`, `SystemTime`) and no global RNG (`rand::rng()`/`thread_rng`) inside generators; time comes from `dt`/`ctx.time`, randomness from `ctx.rng`. Enforced by `clippy.toml` `disallowed-methods`.
- **GEN-R8** Performance: `update(1/60)` + `render` < 5 ms at 200×50 in release (criterion bench per generator).
- **GEN-R9** Motion: over 1 simulated second at 200×50 the rendered frame changes (a scene is an animation, not a still). Verified by the contract suite.
- **GEN-R10** Unicode glyphs must have an ASCII fallback (`Glyph::new(u, a)`); ASCII mode output is pure ASCII.

## Per-generator spec template
Each scene gets a short section appended below when its batch starts:

```
### <name>  (category, batch)
Intent: what the viewer should see/feel.
Model: state, physics/maths, parameters.
Motion: what changes over time and at what rate.
Resize: what is rebuilt.
Inks: roles used.
Acceptance: scene-specific assertions beyond GEN-R4..R10.
```

### starfield  (Basic, M1)
- **Intent**: drifting parallax starfield with twinkling.
- **Model**: `N = clamp(area * 0.08, 1, 4000)` stars with `x, y ∈ [0, size)`, `layer ∈ {1..4}` (4 = nearest), `phase ∈ [0, 2π)`, `rate ∈ [1.5, 4]` rad/s.
- **Motion**: `x += layer * SPEED * dt` wrapping at width (parallax: nearer = faster); brightness = layer base + twinkle `sin(phase + rate * t)`.
- **Resize**: star field regenerated for the new area.
- **Inks**: `Role::Star`, level from brightness.
- **Acceptance**: (`STAR-A1`) star count follows the formula; (`STAR-A2`) nearer layers move farther than farther layers over the same dt.
