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
