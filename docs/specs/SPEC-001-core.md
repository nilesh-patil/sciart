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
- **CORE-R28** `ParticlePool<P>::with_capacity(n)` holds at most `n` particles. `spawn(p) -> bool` adds `p` and returns `true`, or returns `false` and drops `p` when full. `retain(|&mut P| -> bool)` updates every particle in place and removes those for which the closure returns `false` (order is not preserved). `len`, `is_empty`, `capacity`, `iter`, `iter_mut`, `clear`, and `set_capacity(n)` (truncates when shrinking) complete the API. No operation other than `set_capacity` growing may allocate — verified with a counting allocator in `crates/sciart-core/tests/alloc_free.rs` (ADR-0009), as are CORE-R29 `push` and CORE-R36 `step` after its first call.
- **CORE-R29** `Trail<T: Copy>::new(n)` is a fixed-length history: `push(v)` appends and evicts the oldest when `n` entries exist; `iter()` yields oldest → newest; `len() <= n`; `n = 0` keeps nothing; `clear()` empties it.
- **CORE-R30** `gaussian(rng, mean, sd) -> f64` draws a normal variate (Box–Muller or `rand_distr`-free equivalent); always finite; `sd == 0` returns exactly `mean`; a large seeded sample has mean within 3 % of `sd` of `mean` and standard deviation within 3 % of `sd`; non-finite parameters are treated as 0 and a negative `sd` as `|sd|`. `gaussian2(rng, sd) -> (f64, f64)` draws an isotropic 2-D pair.
- **CORE-R31** `kepler_speed(gm, r) = √(gm / r)` and `kepler_omega(gm, r) = √(gm / r³)` for `r > 0`, `gm ≥ 0`; `0` for `r ≤ 0` or non-finite input (never NaN or ∞).
- **CORE-R32** `plot(canvas, x, y, glyph, ink)` takes **f64** cell coordinates, rounds to the nearest cell and ignores non-finite or off-canvas points (so particle code never casts NaN to `i32`).

### Grid helpers (`grid`, M6)
A dense 2-D field for automata, lattices and PDEs. Coordinates are `i32` cells `(x, y)`, row-major storage.
- **CORE-R33** `Grid<T: Clone>::new(size, fill)`; `size()`, `width()`, `height()`; `get(x, y) -> Option<&T>`, `get_mut`, `set(x, y, v) -> bool` return `None`/`false` off-grid (never panic for any `i32`); `fill(v)`; `iter()` yields `((x, y), &T)` in row-major order; `as_slice()` exposes the row-major buffer. A 0×0 grid (`Grid::default()`, since `Size` is at least 1×1) is valid and empty; use `width()`/`height()`/`is_empty()` for it. There is no panicking `Index` impl (GEN-R4); off-grid centres have no neighbours.
- **CORE-R34** `resize(size, fill)` keeps the values of cells whose coordinates exist in both sizes and sets new cells to `fill` (reusing the allocation when it is large enough).
- **CORE-R35** `neighbors(x, y, hood, edges)` iterates the in-grid neighbour coordinates of `(x, y)` (excluding itself) for `Hood::VonNeumann` (4) or `Hood::Moore` (8), with `Edges::Clip` (drop off-grid) or `Edges::Wrap` (toroidal; on a 1-wide or 1-tall grid a wrapped neighbour may equal the cell itself and is then skipped, and duplicates are yielded once). It does not allocate.
- **CORE-R36** `step(&mut self, scratch: &mut Grid<T>, rule)` performs one synchronous update: `scratch[x,y] = rule(&self, x, y)` for every cell, then swaps the buffers, so a rule never sees partially updated state. `scratch` is resized to match if needed and is otherwise not allocated.

### Integrator & timeline helpers (`ode`, `timeline`, M7)
For deterministic physics and scripted, looping narratives. Pure, allocation-free.
- **CORE-R37** `ode::rk4_step(y: &mut [f64; N], t, h, f)` advances `y' = f(t, y)` (with `f(t, &[f64; N]) -> [f64; N]`) by one classical 4th-order Runge–Kutta step; `ode::integrate(y, t, dt, h_max, f)` covers `dt` with `ceil(dt / h_max)` equal steps (none for `dt ≤ 0` or non-finite `dt`; `h_max` clamped to `MIN_STEP = 1e-6` and the step count capped at `MAX_STEPS = 4096` to protect GEN-R8; returns the step count). RK4 is exact for solutions that are polynomials of degree ≤ 4 in t. Global error on `y' = −y` over 1 s is `O(h⁴)` (halving `h` reduces it ≥ 12×); non-finite results are left for the caller to detect.
- **CORE-R38** `Timeline::new(&[(label, seconds)])` is a looping sequence of phases (non-positive durations are treated as zero-length and skipped); `at(t) -> Beat { index, label, local, progress }` with `local ∈ [0, duration)`, `progress = local / duration ∈ [0, 1)`, wrapping at the total length (any finite `t`, including negative, maps into the cycle); `total()`; `cycle(t)` returns the completed-cycle count. An empty or all-zero timeline returns index 0 with progress 0 and never panics.
- **CORE-R39** easing: `ease::smoothstep(x)`, `ease::in_out_cubic(x)`, `ease::out_quad(x)` map `[0, 1] → [0, 1]` monotonically with `f(0) = 0`, `f(1) = 1`, clamping inputs outside the range (NaN ⇒ 0); `ease::lerp(a, b, t)`.

### Raster, fields, symplectic & 3-D helpers (M10)
Sub-cell rendering (ADR-0010), colormap roles (ADR-0011), capped/resampled lattices (ADR-0012). Pure, deterministic, allocation-free after construction/resize.
- **CORE-R40** `Cell` gains `bg: Ink` (`Cell::BLANK.bg == Ink::BLANK`, meaning "no background"). `Canvas::put` resets `bg` to `Ink::BLANK`; `Canvas::put_bg(x, y, glyph, fg, bg)` sets both, with the same bounds and glyph-mode rules as `put` (CORE-R3, CORE-R4). `to_lines` is unchanged.
- **CORE-R41** `raster::Dots::new(cells: Size)` is a braille lattice of `2·w × 4·h` dots (`dot_size()`). `set(x: i32, y: i32, ink)` lights a dot; `plot(x: f64, y: f64, ink)` rounds and ignores non-finite points; off-lattice dots are no-ops. Each cell keeps the ink of its most intense lit dot (ties: last set wins), where intensity is `level`, except for `Role::Div`, where it is `|level − 128|` (a negative lobe keeps its strongest dot). `clear()`, `resize(cells)` (reallocates only when the size changes), `is_lit(x, y)`. `blit(&self, canvas)` writes every cell that has ≥ 1 lit dot as the braille glyph `U+2800 + mask` (standard dot order: bits 0–2 left column rows 0–2, bits 3–5 right column rows 0–2, bit 6 left row 3, bit 7 right row 3). Its ASCII fallback is by dot count: 1 ⇒ `.`, 2 ⇒ `:`, 3–4 ⇒ `+`, ≥ 5 ⇒ `#`. Cells with no lit dots are left untouched, so `Dots` layers over other drawing. `set`, `plot`, `clear` and `blit` never allocate.
- **CORE-R42** `raster::Pixels::new(cells: Size)` is a `w × 2·h` buffer of `Ink` (`Ink::BLANK` = empty), with square pixels at `ASPECT = 2`. It provides `set(x, y, ink)`, `get(x, y) -> Option<Ink>`, `fill`, `clear`, `resize(cells)` and `pixel_size()`. `blit(&self, canvas, ascii)`: in Unicode mode, both pixels empty ⇒ untouched; only top ⇒ `▀` fg top; only bottom ⇒ `▄` fg bottom; both ⇒ `▀` with fg top and bg bottom (`put_bg`). In ASCII mode it calls `ascii(x, y, top, bottom) -> Option<(char, Ink)>` per cell and writes the result, so every half-block scene has to supply its own fallback (GEN-R10). `blit` never allocates.
- **CORE-R43** `grid::resample_bilinear(src: &Grid<f32>, dst: &mut Grid<f32>)` fills `dst` (keeping its size) by cell-centre-aligned bilinear sampling of `src` with edge clamping. It is identity for equal sizes, preserves constants exactly and preserves linear ramps in the interior. `grid::resample_nearest<T: Clone>(src, dst)` does the same with nearest-neighbour sampling, so only values present in `src` appear in `dst`. Both are no-ops on an empty `src` or `dst` and do not allocate.
- **CORE-R44** `ode::verlet_step(pos: &mut [f64], vel: &mut [f64], acc: &mut [f64], h, accel)` performs one kick–drift–kick leapfrog step, where `accel(pos, acc_out)` writes accelerations and `acc` must hold `a(pos)` on entry (it holds `a(pos')` on exit). Mismatched slice lengths make it a no-op. `ode::verlet_integrate(pos, vel, acc, dt, h_max, accel) -> usize` covers `dt` with equal substeps under the same count rules as `integrate` (CORE-R37: `MIN_STEP`, `MAX_STEPS`, no steps for `dt ≤ 0` or non-finite). Properties: exact for constant acceleration; time-reversible (step with `h`, negate `vel`, step again ⇒ start within 1e-12 relative); for the harmonic oscillator the energy error stays bounded (no secular drift) over 10⁴ periods at `h = T/50`. It does not allocate.
- **CORE-R45** `geom::Camera { yaw, pitch, distance }` (radians; `distance = f64::INFINITY` ⇒ orthographic). `project(&self, p: [f64; 3], centre: (f64, f64), scale: f64) -> Option<(f64, f64, f64)>` rotates `p` by yaw (about y) then pitch (about x) and returns screen `(x, y)` in cell units, with x stretched by `ASPECT`, y pointing down, `scale` in rows per world unit and perspective factor `d / (d − z)`. It also returns `depth = z` (larger = nearer). It returns `None` for non-finite input or points at or behind the eye (`z ≥ distance − 1e-9`). The origin maps to `centre`; `yaw = π/2` maps +x onto the depth axis.
- **CORE-R46** `Role` gains `Seq` (sequential field: level 0 = low, 255 = high) and `Div` (signed field: level 128 = zero, 0 = most negative, 255 = most positive) (ADR-0011). `ink::seq(v)` maps `v ∈ [0, 1]` and `ink::div(v)` maps `v ∈ [−1, 1]` onto those levels (clamped, NaN ⇒ low/zero).
- **CORE-R47** `Category` gains `Biology` (label `"biology"`) and `Chemistry` (label `"chemistry"`). `PhysicsChemistry` keeps its label for existing scenes.
- **CORE-R48** `grid::lattice_for(display: Size, max_nodes: usize, bucket: f64) -> Size` picks a capped simulation lattice for a display region in square units (ADR-0012). The aspect `display.w / display.h` is quantised to the nearest power of `bucket` (`bucket > 1`, for example `1.25`; otherwise treated as exact). It returns the largest lattice with that aspect and `w·h ≤ max_nodes`, never larger than `display` in either dimension, at least 1×1, and never panicking for any input (`max_nodes = 0` ⇒ 1×1). Two displays in the same aspect bucket whose sizes are both at or above the cap map to the same lattice, so resizing within a bucket does not touch simulation state.

### Shared scene helpers (M10, from the spec critics)
Each was requested independently by several M10 scene authors. They are pure, deterministic, allocation-free after construction/resize, and never panic for any input.
- **CORE-R49** `text::StackText<const N: usize>` is a fixed-capacity UTF-8 buffer implementing `core::fmt::Write`. It truncates silently at `N` bytes on a char boundary and provides `as_str()`, `clear()` and `len()`, and it never allocates, so `write!(buf, "T/Tc {:.2}", t)` is safe in `render` (GEN-R11). `draw::text_mode(canvas, x, y, unicode: &str, ascii: &str, ink) -> i32` writes `unicode` or `ascii` depending on the canvas glyph mode (left-aligned, clipped) and returns the column after the last char.
- **CORE-R50** `Camera::projector(&self) -> Projector` caches the yaw/pitch sines and cosines. `Projector::rotate(p) -> [f64; 3]` returns camera-space coordinates, and `Projector::project(p, centre, scale)` returns results bit-identical to `Camera::project`.
- **CORE-R51** `Dots::line(x0, y0, x1, y1, ink)` (f64 dot coordinates, rounded) lights a Bresenham segment on the dot lattice with both endpoints inclusive and clipped to the lattice. Non-finite endpoints draw nothing, and the length is capped (≤ 4·(dot width + dot height) dots) so wild coordinates stay cheap.
- **CORE-R52** `grid::SampleMap::rebuild(src: Size, dst: Size)` builds per-axis source-index and weight tables using the same cell-centre, edge-clamp convention as `resample_bilinear` (CORE-R43), and allocates only when it grows. `sample(&self, src: &Grid<f32>, x, y) -> f32` returns the bilinear value at destination cell `(x, y)` (identical to `resample_bilinear`'s output for that cell). `nearest(x, y) -> (i32, i32)` returns the nearest source cell. Off-range destination coordinates are clamped.
- **CORE-R53** `grid::laplacian9(src: &Grid<f32>, dst: &mut Grid<f32>, edges)` writes the isotropic 9-point Laplacian `(4·(N+S+E+W) + (NE+NW+SE+SW) − 20·c) / 6` (unit grid spacing). `Edges::Wrap` is periodic, and `Edges::Clip` is zero-flux (Neumann, ghost cells mirror the edge). Mirror pairs are summed first (`(W+E)`, `(N+S)`, …), so a left-right mirrored input gives a bitwise mirrored output. A constant field gives exactly 0, and on a periodic grid `Σ dst = 0` within rounding. `dst` is resized to `src` if needed.
- **CORE-R54** `particles::SpatialHash::new(width, height, cell)` is a uniform cell list over `[0, width) × [0, height)` built by counting sort. `rebuild(points: &[[f64; 2]])` keeps a stable index order within a bucket and clamps out-of-range or non-finite points to the border bucket. `for_each_within(p, r, |i| ..)` visits every index whose point lies within distance `r` of `p` (and possibly others from the overlapped buckets), each exactly once, in deterministic order. It allocates only when the point count or bucket count grows.
- **CORE-R55** `raster::Density::new(cells: Size)` is an `f32` long-exposure buffer at `Dots` resolution. It provides `decay(k)` (multiply all by `k ∈ [0, 1]`), `deposit(x, y, w)` (f64 dot coordinates, nearest dot, non-finite ignored), `deposit_line(x0, y0, x1, y1, w)` (along a CORE-R51 segment), `resize`, `clear`, and `to_dots(&self, dots, threshold, map: impl Fn(f32) -> Ink)`, which lights every dot whose value is `≥ threshold` with `map(value)` in one pass.
- **CORE-R56** `ink::soft_clip(x)` is the Padé tanh `x(27 + x²)/(27 + 9x²)`, clamped to `[−1, 1]`: monotone, odd, exactly ±1 at |x| ≥ 3, libm-free, NaN ⇒ 0. `ink::seq_value(Ink) -> f64` and `ink::div_value(Ink) -> f64` are exact inverses of `ink::seq`/`ink::div` on their levels, so `ink::seq(ink::seq_value(i)) == i` and likewise for `div`.
- **CORE-R57** `rng::SplitMix64` is a tiny deterministic generator for hot inner loops, seeded from `ctx.rng` so GEN-R7 still holds. It provides `new(seed)`, `from_rng(&mut impl Rng)`, `next_u64()` (reference: seed 0 ⇒ first output `0xE220A8397B1DCDAF`), `next_f64() ∈ [0, 1)` with 53 random bits, and `below(n: u32)`.
- **CORE-R58** `particles::InverseCdf<const K: usize>::build(a, b, density: impl Fn(f64) -> f64)` tabulates a non-negative density on `[a, b]` into `K` quantiles, with no allocation. `sample(u)` is monotone non-decreasing in `u ∈ [0, 1]` and lies in `[a, b]`, and it is exact for a uniform density (`a + u·(b − a)`). A density that is zero or non-finite everywhere falls back to uniform.
- **CORE-R59** `ode::tridiag_solve(sub, diag, sup, rhs, scratch) -> bool` solves a tridiagonal system in place into `rhs` (Thomas algorithm, `scratch` of the same length). Mismatched lengths or a zero pivot return `false`, leave `rhs` unchanged and never panic.
