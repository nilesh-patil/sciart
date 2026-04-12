# SPEC-002: Generators & registry

**Crate**: `crates/sciart-generators` · **Milestones**: M1 (registry, starfield), M4–M7 (batches) · **Status**: M1 + M4 (Batch A) implemented

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

---
## Batch A — analytic / time-driven scenes (M4)
Shared helpers: `sciart_core::draw` (CORE-R22..R27). All lengths below are in **rows**; x is stretched by `ASPECT`. `R = draw::fit_radius(size, frac)`.

### solar_system  (Astronomy, M4)
- **Intent**: the eight planets circling the Sun at physically ordered speeds, with an asteroid belt.
- **Model**: orbit radii `R(0.95) · {.14,.22,.30,.40,.60,.74,.87,1.0}`; Kepler's third law `ω(a) = (2π/12 s) · (a/a⊕)^-1.5` (Earth year = 12 s); random initial phases; 60 belt particles at `0.48–0.54` of the outer radius.
- **Motion**: every body advances `ω·dt`. **Resize**: radii recomputed, phases kept.
- **Inks**: Sun `Sun`, planets `Planet`, orbits `Dim`, belt `Particle`.
- **Acceptance**: `SOL-A1` ω strictly decreases outward and Earth's period is 12 s; `SOL-A2` every planet stays on-canvas at any time (200×50); `SOL-A3` the Sun's core sits on the center cell.

### pulsar  (Astronomy, M4)
- **Intent**: a lighthouse neutron star — two opposite radiation cones sweeping, dipole field loops, a flash each time a beam faces the viewer.
- **Model**: spin period 2.5 s, beams at `θ(t)` and `θ+π`; cone half-angle 0.12 rad; dipole field lines `r = L·sin²φ` about the beam (magnetic) axis for `L ∈ {0.35, 0.6, 0.85}·R(1)`; the viewer sits at angle `-π/2` (screen up) and a flash lasts while a beam is within 0.2 rad of it.
- **Inks**: beams `Wave` (level falls off with distance and off-axis angle), core `Hot`, field `Dim`, flash halo `Accent`.
- **Acceptance**: `PUL-A1` the beams are exactly π apart and advance by `2π/2.5` rad/s; `PUL-A2` exactly two flashes per rotation; `PUL-A3` field-line points satisfy `r = L sin²φ`.

### moon_phases  (Astronomy, M4)
- **Intent**: a lit sphere going through a full lunation, with craters and the phase name.
- **Model**: synodic cycle 30 s, phase `p ∈ [0,1)`; light direction `L = (sin 2πp, 0, −cos 2πp)`; surface normal `n = (nx, ny, √(1−nx²−ny²))`; illumination `n·L`; illuminated fraction `(1 − cos 2πp)/2`. Scene time starts at a seeded offset in `[0, 30)` s so runs open on different phases (still deterministic per seed).
- **Inks**: lit `Star` (level ∝ illumination), limb of the dark side `Dim`, label `Text`.
- **Acceptance**: `MOON-A1` new moon (p=0) has no lit point on the visible hemisphere, full moon (p=0.5) lights every interior point; `MOON-A2` names: 0 New Moon, .25 First Quarter, .5 Full Moon, .75 Last Quarter, .97 New Moon; `MOON-A3` illuminated fraction is 0, ½, 1, ½ at p = 0, ¼, ½, ¾.

### exoplanet_transit  (Astronomy, M4)
- **Intent**: a planet crossing a limb-darkened star while the light curve draws the dip.
- **Model**: star radius `Rs`, planet `rp = max(0.12 Rs, 0.5)`, impact parameter `b = 0.3 Rs`, period 10 s along `u ∈ [−2.2 Rs, 2.2 Rs]`; flux `1 − A(d; Rs, rp)/(π Rs²)` with `A` the exact circle–circle overlap area; limb darkening `I(μ) = 1 − 0.6(1 − μ)` for shading only. The light-curve panel (right 38 % of the screen, only if ≥ 12×6 cells) plots one full period with a marker at the current phase.
- **Inks**: star `Sun`, planet `Planet` (off-disk) / blank silhouette (on-disk), curve `Accent`, axes `Structure`.
- **Acceptance**: `TRAN-A1` flux = 1 when the disks don't touch; `TRAN-A2` flux = `1 − (rp/Rs)²` when the planet is fully inside; `TRAN-A3` overlap is symmetric in its radii and non-increasing in `d`.

### satellite_orbit  (Space Exploration, M4)
- **Intent**: a rotating Earth (day/night, oceans, continents, ice caps) with satellites on inclined orbits passing in front of and behind it.
- **Model**: Earth radius `Re = R(0.45)`, spin period 20 s, procedural continents from seeded sinusoids on (lon, lat); 5 satellites with `a ∈ [1.25, 2.2]·Re`, inclination `∈ [0.2, 1.2]` rad from the screen plane (radii and inclinations drawn stratified so orbits spread out), node angle random, `ω = (2π/6 s)·(a/Re)^-1.5`. 3-D position `(a cosθ, a sinθ cos i, a sinθ sin i)` rotated by the node angle in the screen plane; `z < 0` is behind.
- **Inks**: ocean `Cold`, land `Particle`, ice `Text`, night side dimmed, satellites `Accent`, orbit paths `Dim`; extras: sun-side atmosphere glow `Cold`, sparse background stars `Dim`.
- **Acceptance**: `SAT-A1` a satellite is hidden iff `z < 0` and its screen distance from Earth's center is `< Re`; `SAT-A2` ω decreases with `a`; `SAT-A3` `|position| = a`.

### doppler_effect  (Physics & Chemistry, M4)
- **Intent**: a moving source compressing wavefronts ahead (blueshift) and stretching them behind (redshift).
- **Model**: wave speed `c = 10` rows/s, source speed `v = 0.5c` left→right (wraps with margin), one wavefront every `T = 0.35 s`, each front a circle of radius `c·(t − t₀)` around its emission point; fronts are dropped once larger than the screen diagonal.
- **Inks**: each front point is coloured by its direction θ from its own emission point — `Cold` (blueshifted, `cos θ > 0`) else `Hot`; on the axis this is exactly "ahead of the source ⇒ `Cold`", and it stays correct off-axis and after the source wraps; source `Accent`.
- **Acceptance**: `DOP-A1` in steady state the spacing of fronts along the axis is `(c − v)T` ahead and `(c + v)T` behind; `DOP-A2` the number of live fronts is bounded by `diag/(cT) + 2`.

### wave_interference  (Physics & Chemistry, M4)
- **Intent**: two coherent point sources whose ripples add up into bright and dark bands.
- **Model**: sources at `(0.25 w, h/2 ± s/2)`, `s = 0.35 h` rows; wavelength `λ = max(3, h/6)` rows; `ω = 3` rad/s; amplitude `a = sin(k d₁ − ωt) + sin(k d₂ − ωt)`, glyph from `(a + 2)/4`.
- **Inks**: `Wave`, level ∝ `(a + 2)/4`; sources `Accent`; title `Text` (only if `h > 8`).
- **Acceptance**: `WAVE-A1` on the perpendicular bisector the amplitude is `2 sin(kd − ωt)`; `WAVE-A2` where the path difference is `λ/2` the amplitude is ≈ 0 at all times.

### cosmic_microwave_background  (Astronomy, M4)
- **Intent**: the Planck-style oval sky map: smooth hot/cold anisotropies that slowly shimmer (not per-frame noise).
- **Model**: `f = Σ aᵢ sin(kᵢ·(x/ASPECT, y) + φᵢ + ωᵢ t) / Σ aᵢ` over 6 seeded modes (amplitudes `aᵢ ∈ [0.5, 1]`, wavelengths 3–20 rows, `ω ∈ [0.2, 0.8]` rad/s); drawn only inside an ellipse covering 94 % of the screen (Mollweide-like); caption `CMB  T=2.725K  dT~1e-5` on the last row when `h > 6`.
- **Inks**: `f > 0` ⇒ `Hot`, else `Cold`, level ∝ `|f|`.
- **Acceptance**: `CMB-A1` `|f| ≤ 1`; `CMB-A2` smooth — horizontally adjacent cells differ by at most `Σ aᵢ|kᵢ| / (ASPECT Σ aᵢ)`; `CMB-A3` a 200×50 frame contains both hot and cold cells.

### gravitational_lensing  (Astronomy, M4)
- **Intent**: background stars drifting behind a massive lens, each seen as two images, and a galaxy that periodically lines up into an Einstein ring.
- **Model**: point lens with Einstein radius `θE = R(0.35)`; lens equation `β = θ − θE² θ/|θ|²`; for a source at `β` the images are `θ± = β̂ (|β| ± √(|β|² + 4θE²))/2`; 160 seeded stars drifting at 0.6 rows/s (wrapping); an extended source (radius 1.5 rows) crossing near the lens every 12 s, rendered by inverse ray-shooting within `2.5 θE`. Images with `|μ| < 0.05` are not drawn (in practice only far minor images, which otherwise clutter the Einstein disc; real ones are ≈ `u⁻⁴` faint).
- **Inks**: images `Star` (level by magnification), ring/arcs `Accent`, lens `Structure`.
- **Acceptance**: `LENS-A1` both images map back to `β` through the lens equation; `LENS-A2` as `β → 0` both images approach radius `θE`; `LENS-A3` for `|β| ≫ θE` the primary image ≈ `β` and the secondary → 0.

### constellation_map  (Astronomy, M4)
- **Intent**: a star chart that traces one constellation at a time, then names it, over a twinkling sky.
- **Model**: 6 constellations (Orion, Ursa Major, Cassiopeia, Cygnus, Crux, Scorpius) as normalized star coordinates + edges, fitted to 80 % of the screen preserving shape; each is shown for 12 s: lines trace in over the first 4 s, the name appears after tracing; background stars `0.02·area` twinkle.
- **Data**: star positions from real J2000 RA/Dec and visual magnitudes, gnomonic projection about the figure centre (east left, north up); only stars on the conventional stick figure are included (e.g. ε Cru omitted).
- **Inks**: constellation stars `Star`, lines `Structure`, tracing pen tip `Accent`, name `Text`, background `Dim`.
- **Acceptance**: `CON-A1` every edge references valid stars and every star is on at least one edge; `CON-A2` tracing progress 0 draws no segment, 1 draws all, and drawn length is monotone in progress; `CON-A3` the constellation index advances every 12 s and wraps.
