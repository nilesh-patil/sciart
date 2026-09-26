# SPEC-002: Generators & registry

**Crate**: `crates/sciart-generators` · **Milestones**: M1 (registry, starfield), M4–M7 (batches) · **Status**: M1, M4–M7 (Batches A–D) implemented

Ports the Python stability contract (`docs/archive/issue-01-simulator-stability.md`, R1) to Rust
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
- **GEN-R11** After warm-up (build, resize, ≥ 1 step and one render), `render` performs zero heap allocations, and `update(1/60)` performs zero allocations in steady state unless the scene is listed with a reason in `UPDATE_ALLOCATES` of `crates/sciart-generators/tests/alloc_free.rs` (empty today). Resizing may allocate. Verified at 200×50 and 80×24, in both glyph modes, by that test (ADR-0009).

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
- **Model**: an isotropic Gaussian random field by spectral synthesis, `f = (1/N) Σ sin(kᵢ·(x/ASPECT, y) + φᵢ + ωᵢ t)` with N = 96 equal-power modes; `|kᵢ|` stratified-sampled so their density in `ln ℓ` follows `D_ℓ·B_ℓ²` — `D_ℓ` an approximate Planck 2018 ΛCDM TT spectrum (Sachs–Wolfe plateau, first acoustic peak at ℓ = 220 ≈ 5× plateau, trough, 2nd/3rd peaks, damping), `B_ℓ² = exp(−(ℓ/400)²)` a "cell beam", ℓ ∈ [10, 760] — mapped to the screen as `|k| = (2π/6)·ℓ/220` rad/row (first peak = 6-row wavelength); directions from a jittered golden-ratio sequence over [0, π) along the ℓ-sorted modes; phases uniform; `|ωᵢ| ∈ [0.08, 0.35]` rad/s with random sign (slow shimmer, no drift); display saturates at `|f| = 2.7σ`, `σ = 1/√(2N)` (Planck colour bar ±300 μK ≈ ±2.7 σ_CMB); drawn only inside an ellipse covering 94 % of the screen (Mollweide-like); caption `CMB  T=2.725K  dT~1e-5` on the last row when `h > 6`.
- **Inks**: `f > 0` ⇒ `Hot`, else `Cold`, level ∝ `min(|f|/2.7σ, 1)`.
- **Acceptance**: `CMB-A1` `|f| ≤ 1`; `CMB-A2` smooth — horizontally adjacent cells differ by at most `Σ|kₓᵢ| / (ASPECT·N)`, and adjacent-cell correlation is ≥ 0.9 horizontally and ≥ 0.7 vertically; `CMB-A3` a 200×50 frame contains both hot and cold cells; `CMB-A4` isotropic, not stripy — the direction tensor `Σ k̂k̂ᵀ` has anisotropy ≤ 0.05 and the two-point correlation at equal aspect-corrected lags agrees across directions within 0.1 (mode ensemble at 0°/90°/±45°, 1–6 rows; and sampled on a 200×50 screen, horizontal vs vertical, 1–3 rows); `CMB-A5` CMB-like spectrum — `D_ℓ` peaks at ℓ ∈ [200, 240] with `D_220/D_10 ∈ [4.5, 6]`, a trough then a second peak, and damping; the most populated Δln ℓ = 0.5 band of modes is centred at ℓ ∈ [140, 300].

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

---
## Batch B — particle systems (M5)
Shared helpers: `sciart_core::particles` (CORE-R28..R32). Pools are sized in `resize` from the area and are
bounded (GEN-R8); particles are respawned rather than reallocated. Positions are in **rows** unless stated
(x on screen = `cx + x·ASPECT`). Physics uses `dt` from `update` only.

### black_hole  (Astronomy, M5)
- **Intent**: an Interstellar-style black hole: a dark shadow, a thin bright photon ring, and a tilted accretion disk whose far side is lensed up over the shadow; the approaching side glows brighter.
- **Model**: shadow radius `rs = R(0.14)`; ISCO `r_in = 2.2 rs`; disk out to `R(0.95)`; inclination 78° (disk ellipse flattening `cos i`); particles on circular orbits `ω(r) = kepler_omega(gm, r)` with the ISCO period fixed at 2.5 s; relativistic beaming for shading only: level ∝ `(1 + 0.55·β_los)³` with `β_los` the line-of-sight velocity fraction. Particles slowly drift inward (`dr/dt = −0.02 r`); inside `r_in` they plunge and are respawned at the outer edge. The far half of the disk is drawn at its weak-field lensed image positions (`θE² = 2·R_s·depth`): the bright primary is lifted into an arc over the shadow, a faint secondary appears below; there is no separate unlensed copy. Pool `clamp(area/6, 24, 1600)`, initially uniform in `ln r` (the steady state of the drift).
- **Inks**: disk `Hot` (level from radius and beaming), photon ring `Accent`, lensed arc `Hot` dimmer, shadow = blank.
- **Acceptance**: `BH-A1` ω strictly decreases with r and the ISCO period is 2.5 s; `BH-A2` no disk particle is drawn inside the shadow when it is behind the hole (far side, projected distance < rs); `BH-A3` the approaching half is brighter on average than the receding half; `BH-A4` the pool size stays constant after warm-up (respawn, no growth).

### galaxy_collision  (Astronomy, M5)
- **Intent**: two spiral galaxies passing each other and pulling out tidal tails and bridges (Toomre & Toomre 1972).
- **Model**: restricted three-body: two cores (masses `M1 = 1`, `M2 = 0.8`) on a bound, eccentric, softened (`ε = 0.3` of disk radius) orbit integrated with kick-drift-kick leapfrog at `dt` sub-stepped to ≤ 1/120 s; massless test stars (`≈ 0.04·area`, max 900, split ∝ mass) start on circular orbits `v = √(G M r² / (r² + ε²)^{3/2})` in disks of radius `Rd`, one prograde and one inclined/retrograde, projected to the screen. Cores start at apocentre of an orbit with `a = 3.5 Rd`, `e = 0.63` (Kepler period mapped to 24 s). A cycle is 40 s (approach, pericentre ≈ 12 s, tails, second close passage ≈ 36 s — a conservative restricted three-body model cannot merge) then restarts from a re-seeded configuration (galaxy 2 spin 40–140°); the camera eases to frame both cores.
- **Inks**: galaxy 1 stars `Cold`, galaxy 2 `Hot`, cores `Accent`; star level by local density or distance to core.
- **Acceptance**: `GAL-A1` the core pair conserves energy within 1 % over a full cycle (leapfrog); `GAL-A2` initial star speeds equal the softened circular speed; `GAL-A3` after pericentre some stars are farther than `2·Rd` from both cores (tails form); `GAL-A4` the cycle restarts at 40 s.

### stellar_nursery  (Astronomy, M5)
- **Intent**: a glowing emission nebula with dark dust pillars where gas streams onto protostars that ignite, flare bipolar jets, then shine.
- **Model**: nebula brightness = slowly drifting domain-warped sum of sines (≤ 6 terms, drawn only where above a threshold); 2–3 dust pillars as tapered vertical shapes that occlude; 3–5 protostars at seeded positions; a gas-particle pool (`≈ 0.03·area`, max 600) spawned in the nebula, accelerating toward the nearest protostar (`a = gm / (d² + ε²)`), absorbed within 0.8 rows, adding mass; a protostar ignites when mass ≥ `M_ign` (brightness ramps over 1 s) and then emits short-lived jet particles along a seeded axis; after 25 s of shining it fades (2.5 s) and is replaced by a new protostar elsewhere. Gas is stored in polar coordinates about its target: only the radial motion follows the softened gravity, the swirl is kinematic (pure rotation), which keeps NUR-A1 exact; new gas targets the nearest *accreting* protostar. Ignited stars ionise a growing bubble of nebula; pillar rims glow `Hot` as ionisation fronts; jets are pulsed (Herbig–Haro knots); jet pool `clamp(area/60, 4, 240)`.
- **Inks**: nebula `Wave`/`Hot` blend by level, pillars `Structure` (dark), gas `Particle`, protostars `Star`, jets `Accent`.
- **Acceptance**: `NUR-A1` a gas particle's distance to its target protostar is non-increasing until absorption (for dt ≤ 1/60); `NUR-A2` protostar mass is non-decreasing while it accretes and ignition happens exactly when mass first reaches `M_ign`; `NUR-A3` particle pools never exceed capacity.

### brownian_motion  (Physics & Chemistry, M5)
- **Intent**: a large pollen grain jittering among fast solvent molecules, leaving a fading random-walk trail, with a live mean-squared-displacement readout.
- **Model**: grain obeys the Langevin/Ornstein–Uhlenbeck equation `dv = −γ v dt + σ √dt ξ` (γ = 2 s⁻¹, σ chosen so the diffusion constant `D = σ²/(2γ²)` gives ~3 rows/√s), exact OU update for the velocity; reflecting walls; trail of the last 180 positions (`Trail`); `≈ 0.02·area` (max 400) visible solvent molecules moving ballistically with Maxwell–Boltzmann velocities and reflecting walls, occasionally flashing where they "hit" the grain; readout `D^ (MSD@2s) = …   D = … rows^2/s   t = …s` (ASCII): `D̂` from the time-averaged MSD at a 2 s lag with the exact OU inertial correction (a single grain's `r²/4t` from the start never converges); on narrow terminals trailing parts (t, then the true D) are dropped, never clipped, and the readout is hidden when even `D^ (MSD@2s) = …` does not fit. The joint position–velocity OU transition is sampled exactly; walls are exact specular reflections via folding (method of images).
- **Inks**: grain `Accent`, trail `Structure` fading by age, molecules `Particle`, readout `Text`.
- **Acceptance**: `BRO-A1` the stationary velocity variance per axis converges to `σ²/(2γ)` (seeded statistical test, 5 %); `BRO-A2` the ensemble MSD at time t is `4Dt` within 10 % for t ≫ 1/γ (seeded, many walkers); `BRO-A3` the grain and molecules always stay inside the box; the trail length is ≤ 180.

### molecular_dynamics  (Physics & Chemistry, M5)
- **Intent**: a Lennard-Jones fluid in a box that is slowly heated and cooled so it melts into a gas and condenses back into droplets/crystals.
- **Model**: reduced LJ units, `U(r) = 4(r⁻¹² − r⁻⁶)` shifted at cutoff 2.5, in **2-D**; constant reduced density ρ = 0.25, inside the 2-D LJ vapour–liquid dome (r_c = 2.5 truncated-shifted: T_c ≈ 0.46, ρ_c ≈ 0.33–0.35, triple point ≈ 0.40); per-terminal layout: rows per σ `s = clamp(√(ρ·A₁/240), 1.1, 2.5)` and `N = clamp(round(ρ·A₁/s²), 8, 240)` with `A₁` the interior area in σ² at one row per σ (neighbouring crystal rows stay ≥ 1 row apart; 80×24 gets N ≈ 159; above ≈ 250×50 the scale clamps and ρ falls); one simulated second = 6 τ; velocity-Verlet with `dt_md ≤ 0.004` sub-steps per frame; cell list for O(N) forces; reflecting walls; atoms start on a triangular crystal; below `r = 0.8` the potential continues linearly so forces stay finite; a 30 s temperature cycle `T = 0.25 + 0.75·u²`, `u = (1 − cos 2πt/30)/2` (below T_c for about half the cycle, ≈ 2 T_c at the peak), by gentle velocity rescaling (Berendsen, τ = 0.5 s).
- **Inks**: atoms `Cold`→`Hot` by kinetic energy; box `Structure`; readout `T = …` `Text`.
- **Acceptance**: `MD-A1` the force equals `−dU/dr` (finite-difference check) and vanishes beyond the cutoff; `MD-A2` with the thermostat off, total energy drifts < 1 % over 5 simulated seconds; `MD-A3` atoms always stay inside the box; `MD-A4` the thermostat moves temperature toward its target; `MD-A5` the layout keeps ρ within 5 % of 0.25 wherever `s` is not clamped, and at 80×24 (seeded) the mean coordination within 1.5 σ in the cold phase (28–33 s) is > 2× that around the peak (14–19 s) and > 3.5.

### particle_accelerator  (Physics & Chemistry, M5)
- **Intent**: a collider ring where two counter-rotating bunches accelerate, meet at the detector, and burst into a shower of curling tracks.
- **Model**: ring = ellipse `R(0.8)`; bunches at angles `−π/2 + φ(t)` and `−π/2 − φ(t)` (mirror images about the vertical axis), so they meet only when `φ ≡ 0` (the detector, top) or `φ ≡ π` (a second, quieter interaction point, bottom); `dφ/dt` ramps linearly from 0.5 to 6 rad/s over 10 s; after the ramp the bunches collide at the detector each time they meet; each collision spawns 20–40 shower tracks with random momentum `p`, charge ±1, moving in a uniform magnetic field so each follows a circle of radius `r = p/B` (curling clockwise for +, counter-clockwise for −), losing momentum as `dp/dt = −0.8 p` (closed-form inward spiral, exact for any dt; non-relativistic: speed = p, radius = p/B) and fading after ≤ 2 s. Pool capped at 128. Collisions flash; the bottom interaction point only glints. Cycle 20 s; the first shower comes after the 10 s ramp.
- **Inks**: ring `Structure`, bunches `Accent` with short trails, tracks `Hot` (+) / `Cold` (−), detector `Text` label.
- **Acceptance**: `ACC-A1` the two bunches are always mirror images about the vertical axis and meet only at the two interaction points; `ACC-A2` a shower track's curvature radius equals `p/B` and its turning sign equals its charge; `ACC-A3` showers only spawn at the detector after the ramp and the pool never exceeds capacity.

### rocket_launch  (Space Exploration, M5)
- **Intent**: a launch from countdown to orbit: vapour venting, ignition, a rising rocket with a flickering flame, billowing exhaust that cools into smoke, and stage separation.
- **Model**: 22 s cycle — countdown 4 s (vapour puffs, "T−3 … 1" text), ignition 1 s (hold-down, exhaust spreads along the pad), flight: first stage burns 8 s with thrust `F`, exhaust speed `ve`, mass flow `ṁ = F/ve`, `a = F/m − g`; stage separation (the spent stage tumbles and falls away), second stage burns until the rocket exits the top; the view is fixed on the pad until altitude exceeds 60 % of the height, then the camera follows (ground scrolls away). Exhaust particles (`≈ 0.08·area`, max 1500) emitted backward with spread, slowed by drag, rising buoyantly as they cool from `Hot` to grey smoke. Rocket drawn from a small ASCII sprite (≥ 5 rows tall at h ≥ 20, a 1-cell glyph when tiny). Units are scaled for the screen (toy vehicle, `ve` ≈ 110–165 m/s; rows-per-metre chosen so 60 % height is reached 5 s after liftoff); the rocket equation is exact with `t` and `m0` measured from liftoff. Countdown vapour and the separation puff use a separate vapour pool, so "exhaust only while burning" holds strictly. Captions `LIFTOFF`, `STAGE SEPARATION`, `STAGE 2 IGNITION` and a telemetry line (T+, ALT, VEL) are shown.
- **Inks**: rocket body `Text`, flame `Hot`/`Accent`, smoke `Dim`, ground/tower `Structure`, countdown `Text`.
- **Acceptance**: `ROC-A1` during first-stage burn the velocity matches the rocket equation `v = ve ln(m0/m) − g t` within 1 %; `ROC-A2` exhaust spawns only while an engine burns, and the pool never exceeds capacity; `ROC-A3` altitude is non-decreasing from liftoff until stage separation and the cycle restarts at 22 s.

---
## Batch C — grids, automata & stochastic processes (M6)
Shared helpers: `sciart_core::grid` (CORE-R33..R36) plus `particles` and `draw`. Grids are rebuilt (or `resize`d) in `resize`.

### crystal_growth  (Physics & Chemistry, M6)
- **Intent**: a frost/mineral dendrite growing from a seed by diffusion-limited aggregation (Witten–Sander), glowing at its growing tips, then melting away and regrowing.
- **Model**: DLA on the square **braille sub-cell lattice** (2×4 dots per cell = 1/4 row per side with `ASPECT = 2`), so walks, contact distances and particle cost are all isotropic (weighting steps on the 2:1 cell grid only fixes diffusion and grows vertical needles, D_f ≈ 1.44); 8-neighbour sticking; far-field and near-field jump accelerations (Meakin / Ball–Brady; no measurable effect on D_f); walkers launch on a circle just outside the cluster radius and are discarded beyond `2×` it; sticking probability `p_s ∈ [0.3, 1]` (seeded per run: lower = denser); a bounded walker budget per update (`≤ 4000` walker steps per frame; paced so growth takes ~13–22 s at any size). Melting removes sites by graph distance from the seed (tips first, remainder stays connected). A live `D_f` readout is shown. Each crystal cell stores its attach time. When the cluster reaches `R(0.85)` (or after 60 s) it holds 3 s, then melts outside-in over 3 s and a new seed starts (optionally 2–3 seeds competing).
- **Inks**: fresh cells `Hot`/`Accent` fading by age to `Cold`; walkers faint `Dim` (optional); melting `Dim`.
- **Acceptance**: `CRY-A1` the cluster is always 8-connected to a seed; `CRY-A2` during growth cells are only added (count non-decreasing, attach times never change); `CRY-A3` walkers stay within the kill radius and the grid; `CRY-A4` the mean mass–radius fractal dimension of a small seeded ensemble is in `[1.45, 1.95]` (single clusters scatter ±0.08; finite-size fits read ≈ 1.63 vs the asymptotic 1.71).

### radioactive_decay  (Physics & Chemistry, M6)
- **Intent**: a lattice of unstable nuclei flickering through a decay chain while live population curves trace the exponential law.
- **Model**: chain `A → B → C` (C stable) with half-lives `T_A = 8 s`, `T_B = 3 s`; every nucleus decays within a step with probability `1 − 2^{−dt/T}` (exact for any dt); each decay emits a short-lived α/β track particle (`ParticlePool`) and a flash. Lattice on the left (≈ 62 % width, one nucleus per cell or per 2 cells for readability), plot panel on the right (only if ≥ 24×10 cells) with measured `N_A, N_B, N_C` and the analytic Bateman curves `N_A = N₀ 2^{−t/T_A}`, `N_B = N₀ λ_A/(λ_B − λ_A)(e^{−λ_A t} − e^{−λ_B t})`. Reset when `N_A + N_B < 2 %` of N₀ (≈ 50.6 s). A→B→C within one step is sampled exactly from the 3×3 transition matrix. The panel itself must be ≥ 24×10 (≈ terminals ≥ 63 columns); a resize starts a fresh run.
- **Inks**: A `Hot`, B `Accent`, C `Dim`, tracks `Particle`, curves: measured points in their species ink (A `Hot`, B `Accent`, C `Dim`, legend likewise), analytic Bateman curves `Structure`, axes/labels `Text`.
- **Acceptance**: `RAD-A1` per-step decay probability is `1 − 2^{−dt/T}` and composes exactly (`p(dt₁) ∘ p(dt₂) = p(dt₁ + dt₂)`); `RAD-A2` a seeded 20 000-nucleus run keeps `N_A(t)` within 4σ of `N₀ 2^{−t/T_A}` and `N_B(t)` within 4σ of Bateman; `RAD-A3` `N_A + N_B + N_C = N₀` at all times.

### chemical_reaction  (Physics & Chemistry, M6)
- **Intent**: living Turing patterns — spots that divide, labyrinths and coral — from the Gray–Scott reaction–diffusion system `U + 2V → 3V, V → P`.
- **Model**: `∂u/∂t = D_u∇²u − uv² + F(1 − u)`, `∂v/∂t = D_v∇²v + uv² − (F + k)v`, `D_u = 2 D_v`, `D_v = 0.03` sim-cells²/unit (smaller than Pearson's so several wavelengths fit on screen; only the length scale changes); isotropic 9-point Laplacian; Euler step τ = 0.5 within the derived positivity bound `τ(10/3·D + 1 + F + k) ≤ 1`; periodic boundaries; the simulation grid is aspect-corrected (e.g. one sim cell per 2×1 terminal cells) so patterns look isotropic; explicit Euler sub-steps within the stability bound, a fixed number of reaction time units per real second; presets (e.g. mitosis F=.0367 k=.0649, coral F=.0545 k=.062, worms F=.078 k=.061, spots F=.035 k=.065) cycle every 40 s (30 s hold + 10 s smoothstep blend of (F, k)); seeded perturbation patches (re-seeded if the field dies out). Caption with the preset name and `U + 2V → 3V` when h > 8.
- **Inks**: `v` level → glyph ramp; `Cold` background to `Hot` at high v, `Accent` on fronts (large |∇v|).
- **Acceptance**: `CHEM-A1` u, v stay finite and in `[0, 1]` for any dt ≤ 0.25; `CHEM-A2` the uniform state `(u, v) = (1, 0)` is a fixed point; `CHEM-A3` with the reaction terms off, diffusion conserves `Σu` and `Σv` (periodic); `CHEM-A4` from the seeded start a 200×50 run develops structure (spatial variance of v above a threshold) within 10 s.

### spacetime_grid  (Astronomy, M6)
- **Intent**: the rubber-sheet picture of general relativity: a perspective grid dimpled by two orbiting masses, with gravitational-wave ripples spreading outward.
- **Model**: a `(nx × ny)` vertex mesh (`Grid<[f64; 3]>` or similar) on the plane, depth `z = −Σ mᵢ / √(rᵢ² + ε²)` plus a quadrupole wave term `A·cos(2(φ − Ω(t − r/c)))·f(r)` with retarded time (`c` = wave speed in grid units, amplitude decaying ~ `1/√r` and tapered near the sources); binary on a circular orbit (angular speed Ω, masses `m₁ ≥ m₂` about the centre of mass), inspiralling on a Newtonian (Peters) chirp `a = a₀(1−τ/τc)^{1/4}` with Kepler Ω and closed-form phase, reset at `a = 0.55 a₀` with continuous phase (a circular mode is kept for SPT-A2); wave amplitude grows ∝ `1/a`; `c` is artistic (λ comparable to the orbit so the spiral fits the sheet); the mesh is viewed from a tilted camera (pitch ≈ 55°) and projected with perspective, drawn as polylines with slope-aware glyphs, drawn far-band to near (painter's order) so near lines win; crossings merge into `┼`/`╳`; masses drawn as bright spheres above their wells.
- **Inks**: grid lines `Structure` (level by depth/nearness), wells `Cold`, masses `Sun`/`Star`, wave crests `Wave`.
- **Acceptance**: `SPT-A1` for a single mass the depth is radially symmetric and strictly increases in magnitude toward the mass; `SPT-A2` the binary keeps its centre of mass fixed and its separation constant (non-inspiral mode); `SPT-A3` the wave term has period π in the azimuth and its phase at radius r equals the source phase at `t − r/c`.

---
## Batch D — physics sims & narrative scenes (M7)
Shared helpers: `ode` (RK4), `timeline` + `ease` (CORE-R37..R39) plus `particles`, `grid`, `draw`. Narrative scenes loop forever: every phase has a fixed duration in simulated seconds, so a run is deterministic per seed and resizes never desynchronise the story.

### double_pendulum  (Physics & Chemistry, M7)
- **Intent**: chaos made visible — two double pendulums released from almost identical angles swing together, then diverge wildly, each tracing a glowing path.
- **Model**: exact Lagrangian equations for point masses `m₁, m₂` on massless rods `L₁, L₂` (rows, aspect-correct on screen), `g` scaled so the upper arm alone has a small-angle period of 2.2 s (the coupled slow mode is ≈ 2.78 s); physics runs in model units (`L₁ = 1`) with a separate screen scale, so motion is identical at any terminal size; both twins share one pivot; state `(θ₁, θ₂, ω₁, ω₂)` advanced with `ode::integrate` at `h ≤ 1/240 s`; the twin starts with `Δθ₂ = 10⁻³` rad; high-energy seeded initial angles (both arms above horizontal ± variety); trails of the lower bobs (`Trail`, ~2 s); a divergence readout (`|Δθ| e-folding`) and energy drift; restart every 45 s with new seeded angles.
- **Inks**: pendulum A `Cold`, B `Hot`, rods `Structure`, pivots `Text`, trails fading by age.
- **Acceptance**: `DP-A1` total energy drifts < 0.5 % over 20 s; `DP-A2` for small angles the two normal-mode frequencies match the linearised analytic values within 1 %; `DP-A3` from a seeded chaotic start the twins' separation (max so far — it saturates near π) grows ≥ 100× within 30 s; `DP-A4` rods keep their lengths exactly (bob positions derived from angles).

### mars_rover  (Space Exploration, M7)
- **Intent**: a Perseverance-style rover crossing rust-red dunes under a butterscotch sky: parallax ridges, dust devils, a stop to drill a sample and beam data to an orbiter, as the sol passes from day to dusk (blue Martian sunset).
- **Model**: world-x terrain from seeded fractal value noise (≥ 3 octaves) that is a pure function of world x (deterministic, continuous); 3 parallax layers; rocker–bogie suspension: 6 wheels each placed on the terrain surface, body height/pitch from the rocker and bogie pivots; a `Timeline` sol: drive (18 s) → stop & deploy arm (3 s) → drill with dust (4 s) → stow (2 s) → transmit to an orbiter crossing the sky (4 s) → drive … ; dust particles from wheels and drill (`ParticlePool`); 0–2 dust devils drifting; Phobos crossing fast; sky/sun tint changes over a slower 90 s sol cycle (solar elevation at Jezero's latitude; Phobos on its real synodic period, scaled). Pose is solved by Newton iteration on the three wheel-contact residuals (rigid links, body pitch = rocker angle); far-side wheels coincide with the near ones in the side view. Dust over solid fills lightens rather than replaces the cell (a "veil" compositor), since cells have no background colour.
- **Inks**: terrain `Hot` (near) → `Dim` (far layers), day haze `Sun` (low levels), blue sunset glow `Cold`, dusk band/orbiter `Accent`, dust devils `Hot`, rover body `Text`, wheels/arm `Structure`, dust `Particle`, signal `Wave`.
- **Acceptance**: `MR-A1` every wheel contact point lies on the terrain surface (within 0.5 row); `MR-A2` terrain height is a pure deterministic function of world x — identical across scrolls, resizes and chunk boundaries, and continuous (adjacent columns differ by ≤ a bound); `MR-A3` the sol timeline visits its phases in order with their durations and loops.

### voyager_journey  (Space Exploration, M7)
- **Intent**: Voyager 2's Grand Tour, 1977–1989 in 48 s: launch from Earth, gravity assists at Jupiter, Saturn, Uranus and Neptune with flyby flashes and captions, then out past the heliopause to a closing "Pale Blue Dot" moment.
- **Model**: heliocentric top-down map with a monotone radial compression (e.g. `ρ = R·log(1 + r/r₀)/log(1 + r_max/r₀)`, AU→rows) so Earth…Neptune all fit; planets on circular orbits with real mean motions and J2000-ish mean longitudes; real dates: launch 1977-08-20, Jupiter 1979-07-09, Saturn 1981-08-26, Uranus 1986-01-24, Neptune 1989-08-25; the spacecraft path is a smooth curve through the planets' positions at those dates (e.g. Hermite/polar interpolation in (r, φ) between encounter points with monotone r), time compression with slow-downs near encounters (via `ease`), a date/distance readout; 48 s mission + 8 s epilogue, then loop. Planet elements from JPL/Standish "Keplerian Elements for Approximate Positions" (J2000); the path is a monotone cubic Hermite (PCHIP) curve through each planet's model position on its encounter date (a shape, not a propagated trajectory). Captions sit at the top (Uranus/Neptune are near the bottom in the real geometry). The epilogue credits the Pale Blue Dot to Voyager 1 (which took it); the sight line from Voyager 2 is artistic licence.
- **Inks**: Sun `Sun`, orbits `Dim`, planets `Planet`, spacecraft `Accent` with `Structure` trail, captions `Text`, flyby flash `Hot`.
- **Acceptance**: `VOY-A1` at each encounter date the spacecraft coincides with the planet (same model position, and within 0.5 cell on screen); `VOY-A2` planet angular speeds follow Kepler (period ratios `(a₁/a₂)^{3/2}`) and match real periods within 1 %; `VOY-A3` mission time → calendar date is monotone and prints the exact encounter dates at encounters; `VOY-A4` the radial mapping is strictly monotone and fits Neptune's orbit on screen.

### moon_landing  (Space Exploration, M7)
- **Intent**: Apollo 11's powered descent: the LM pitching over, throttling its descent engine above a cratered Mare Tranquillitatis, kicking up dust, "CONTACT LIGHT", "The Eagle has landed", then the flag and an earthrise.
- **Model**: 2-D point-mass LM with thrust `T ≤ T_max` along its attitude, `ṁ = −T/(I_sp g₀)` (I_sp 311 s), lunar `g = 1.62 m/s²`, scaled distances; a guidance law (e.g. gravity-turn then a constant-deceleration / polynomial guidance to a target point with touchdown speed ≤ 1 m/s) that always lands for the seeded start set; cratered terrain (seeded crater field on a gently rolling surface, pure function of x); dust `ParticlePool` when altitude < ~30 m (scaled) blown outward from beneath the engine; `Timeline`: descent (≈ 22 s) → touchdown & dust settles (4 s) → flag & caption (5 s) → earthrise hold (4 s) → loop with a new seeded approach; readout of altitude, vertical speed and fuel %, "CONTACT LIGHT" at probe contact. Guidance: P64 closed-form E-guidance to a gate 8 m above the site, then P66 sink-rate profile with position hold. "Touchdown" is footpad contact; the engine runs until then (a historical cut at CONTACT LIGHT would free-fall 1.7 m to ≈ 2.4 m/s and break ML-A1). Earth appears only in the earthrise phase.
- **Inks**: LM `Text`/`Structure`, flame `Hot`, dust `Particle`, terrain `Dim`/`Structure`, Earth `Cold`, captions `Text`.
- **Acceptance**: `ML-A1` for a seeded set of start conditions the guidance lands with vertical speed ≤ 1 m/s, horizontal ≤ 0.5 m/s and fuel ≥ 0; `ML-A2` with constant thrust the integrated velocity matches the rocket equation `Δv = I_sp g₀ ln(m₀/m) − g t` (vertical burn) within 0.5 %; `ML-A3` the LM never penetrates the terrain; `ML-A4` the timeline phases occur in order and loop.
