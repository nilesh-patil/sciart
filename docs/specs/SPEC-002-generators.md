# SPEC-002: Generators & registry

**Crate**: `crates/sciart-generators` · **Milestones**: M1 (registry, starfield), M4–M7 (batches) · **Status**: M1 + M4 (Batch A) + M5 (Batch B) implemented

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
- **Model**: grain obeys the Langevin/Ornstein–Uhlenbeck equation `dv = −γ v dt + σ √dt ξ` (γ = 2 s⁻¹, σ chosen so the diffusion constant `D = σ²/(2γ²)` gives ~3 rows/√s), exact OU update for the velocity; reflecting walls; trail of the last 180 positions (`Trail`); `≈ 0.02·area` (max 400) visible solvent molecules moving ballistically with Maxwell–Boltzmann velocities and reflecting walls, occasionally flashing where they "hit" the grain; readout `D̂` from the time-averaged MSD at a 2 s lag with the exact OU inertial correction (a single grain's `r²/4t` from the start never converges). The joint position–velocity OU transition is sampled exactly; walls are exact specular reflections via folding (method of images).
- **Inks**: grain `Accent`, trail `Structure` fading by age, molecules `Particle`, readout `Text`.
- **Acceptance**: `BRO-A1` the stationary velocity variance per axis converges to `σ²/(2γ)` (seeded statistical test, 5 %); `BRO-A2` the ensemble MSD at time t is `4Dt` within 10 % for t ≫ 1/γ (seeded, many walkers); `BRO-A3` the grain and molecules always stay inside the box; the trail length is ≤ 180.

### molecular_dynamics  (Physics & Chemistry, M5)
- **Intent**: a Lennard-Jones fluid in a box that is slowly heated and cooled so it melts into a gas and condenses back into droplets/crystals.
- **Model**: reduced LJ units, `U(r) = 4(r⁻¹² − r⁻⁶)` shifted at cutoff 2.5; `N = clamp(area/45, 8, 160)` atoms at a density that fits the box (1 LJ σ ≈ 1.6 rows horizontally-corrected); velocity-Verlet with `dt_md ≤ 0.004` sub-steps per frame; cell list for O(N) forces; reflecting walls; atoms start on a triangular crystal; below `r = 0.8` the potential continues linearly so forces stay finite; a 30 s temperature cycle by gentle velocity rescaling (Berendsen, τ = 1 s) between T = 0.25 and 1.6.
- **Inks**: atoms `Cold`→`Hot` by kinetic energy; box `Structure`; readout `T = …` `Text`.
- **Acceptance**: `MD-A1` the force equals `−dU/dr` (finite-difference check) and vanishes beyond the cutoff; `MD-A2` with the thermostat off, total energy drifts < 1 % over 5 simulated seconds; `MD-A3` atoms always stay inside the box; `MD-A4` the thermostat moves temperature toward its target.

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
