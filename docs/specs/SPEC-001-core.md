# SPEC-001: sciart-core

**Crate**: `crates/sciart-core` · **Milestone**: M1 · **Status**: Implemented (M1)

Pure, terminal-agnostic primitives. No I/O, no OS calls, no global state.

## Requirements

### Size
- **CORE-R1** `Size::new(w, h)` clamps both dimensions to `>= 1`. `Size::area()` returns `w*h` as `usize`.

### Glyph & Canvas
- **CORE-R2** `Canvas::new(size, mode)` allocates exactly `width*height` cells, all blank (`' '`, `Ink::BLANK`).
- **CORE-R3** `Canvas::put(x, y, glyph, ink)` with `x`/`y` outside the canvas (including negative `i32`) is a silent no-op — never panics.
- **CORE-R4** `GlyphMode::Ascii` stores `glyph.ascii`; `GlyphMode::Unicode` stores `glyph.unicode` unless its display width ≠ 1, in which case it stores `glyph.ascii`.
- **CORE-R5** `Canvas::resize(size)` reallocates only when the size changes, and leaves a fully blank canvas. `Canvas::clear()` blanks all cells without reallocating.
- **CORE-R6** `Canvas::to_lines()` returns exactly `height` strings, each exactly `width` chars (used by snapshot tests and `--ascii` debugging).
- **CORE-R7** `Canvas::get(x, y)` returns `Option<&Cell>`.

### Ink
- **CORE-R8** `Ink { role: Role, level: u8 }`; `Ink::BLANK` is `Role::Background` level 0. `Role` is a closed `#[non_exhaustive]` enum of semantic roles; generators must not encode RGB.

### Fixed-step clock
- **CORE-R9** `FixedStep::new(dt, max_steps)`; `advance(elapsed_secs) -> u32` adds `elapsed` to an accumulator and returns `floor(acc / dt)` steps, keeping the remainder.
- **CORE-R10** When more than `max_steps` steps are due, `advance` returns `max_steps` and discards the excess accumulator (no spiral of death).
- **CORE-R11** Negative or non-finite `elapsed` is treated as 0.
- **CORE-R12** `alpha()` returns `acc / dt` in `[0, 1)` for optional interpolation.

### Context & RNG
- **CORE-R13** `Ctx::new(seed, size)` owns a `ChaCha8Rng` seeded from `seed`, the current `Size`, and `time` (simulated seconds, advanced only by the runtime via `Ctx::advance(dt)`). Same seed + same call sequence ⇒ identical RNG output.

### Generator trait
- **CORE-R14** Trait (object-safe, `Send`):
  `meta() -> &'static Meta`, `resize(Size, &mut Ctx)`, `update(f64, &mut Ctx)`, `render(&self, &mut Canvas)`.
  `Meta { name: &'static str, category: Category, description: &'static str }`.
- **CORE-R15** Call protocol guaranteed by callers: `resize` before the first `update`/`render`, and again whenever size changes; `render` receives a cleared canvas of the same size last passed to `resize`.

### Geometry helpers
- **CORE-R16** `geom::ASPECT = 2.0`; `geom::polar(cx, cy, r, theta) -> (i32, i32)` applies `ASPECT` to x.
- **CORE-R17** `geom::line(x0, y0, x1, y1)` yields Bresenham points inclusive of both endpoints.
- **CORE-R18** `geom::wrap(v, max)` returns `v` modulo `max` in `[0, max)` for floats, for any finite `v` and `max > 0`.

### Stage (the single implementation of the call protocol)
- **CORE-R19** `Stage::new(generator, seed, size, mode)` builds a `Ctx`, calls `resize` once, and owns a front and back `Canvas`. `Stage::resize(size)` forwards to the generator only when the size changes.
- **CORE-R20** `Stage::step(dt)` calls `update(dt)` then `ctx.advance(dt)`.
- **CORE-R21** `Stage::render()` clears the back canvas, lets the generator draw into it, then swaps it to the front. If the generator panics mid-render, `Stage::frame()` still returns the previous complete frame (supports RT-R3).

### Drawing helpers (`draw`, M4)
All take centers/radii in **rows** (x is stretched by `geom::ASPECT`), clip to the canvas, never panic for any finite or non-finite input, and cost time proportional to the *visible* area they touch.
- **CORE-R22** `draw::orbit(canvas, cx, cy, r, glyph, ink)` draws a closed ellipse outline of radius `r` rows: every drawn cell lies within 1 cell of the ideal curve, and consecutive samples are dense enough that the outline has no gaps (adjacent drawn cells are 8-connected). `r <= 0` draws nothing.
- **CORE-R23** `draw::disc(canvas, cx, cy, r, shader)` visits every cell whose aspect-corrected normalized distance `d = dist / r` is `<= 1`, calling `shader(d, nx, ny) -> Option<(Glyph, Ink)>` with `nx, ny ∈ [-1, 1]` the normalized offsets; `None` leaves the cell untouched. With `0 < r < 0.5` only the center cell is shaded (with `d = 0`); `r <= 0` or non-finite draws nothing.
- **CORE-R24** `draw::field(canvas, shader)` calls `shader(x, y) -> Option<(Glyph, Ink)>` exactly once per cell.
- **CORE-R25** `draw::ramp(v, glyphs)` maps `v ∈ [0, 1]` onto `glyphs` (0 ⇒ first, 1 ⇒ last), clamping outside values; NaN ⇒ first. Panics only for an empty slice (a programming error caught by tests).
- **CORE-R26** `draw::text_centered(canvas, y, s, ink)` writes `s` horizontally centered on row `y`, clipped at both edges.
- **CORE-R27** `draw::fit_radius(size, frac)` is the largest radius in rows such that a circle of that radius (aspect-stretched) centered on the canvas fits within `frac` of both dimensions: `frac * min(width / (2·ASPECT), height / 2)`, never negative.

### Particle helpers (`particles`, M5)
Shared by the particle-system scenes. Pure, allocation-free after construction, deterministic given `ctx.rng`.
- **CORE-R28** `ParticlePool<P>::with_capacity(n)` holds at most `n` particles. `spawn(p) -> bool` adds `p` and returns `true`, or returns `false` and drops `p` when full. `retain(|&mut P| -> bool)` updates every particle in place and removes those for which the closure returns `false` (order is not preserved). `len`, `is_empty`, `capacity`, `iter`, `iter_mut`, `clear`, and `set_capacity(n)` (truncates when shrinking) complete the API. No operation other than `set_capacity` growing may allocate.
- **CORE-R29** `Trail<T: Copy>::new(n)` is a fixed-length history: `push(v)` appends and evicts the oldest when `n` entries exist; `iter()` yields oldest → newest; `len() <= n`; `n = 0` keeps nothing; `clear()` empties it.
- **CORE-R30** `gaussian(rng, mean, sd) -> f64` draws a normal variate (Box–Muller or `rand_distr`-free equivalent); always finite; `sd == 0` returns exactly `mean`; a large seeded sample has mean within 3 % of `sd` of `mean` and standard deviation within 3 % of `sd`; non-finite parameters are treated as 0 and a negative `sd` as `|sd|`. `gaussian2(rng, sd) -> (f64, f64)` draws an isotropic 2-D pair.
- **CORE-R31** `kepler_speed(gm, r) = √(gm / r)` and `kepler_omega(gm, r) = √(gm / r³)` for `r > 0`, `gm ≥ 0`; `0` for `r ≤ 0` or non-finite input (never NaN or ∞).
- **CORE-R32** `plot(canvas, x, y, glyph, ink)` takes **f64** cell coordinates, rounds to the nearest cell and ignores non-finite or off-canvas points (so particle code never casts NaN to `i32`).
