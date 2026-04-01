# Art Project: Science and Astronomy

This file outlines 25 ideas for generative ASCII art visualizations to be implemented in the `python/art_generators` directory. Each generator creates dynamic, scientifically-inspired animations rendered in the terminal.

## Implementation Guidelines

All generators must:
- Implement a `generate(width, height) -> str` function
- Return plain ASCII/Unicode text (colors applied by orchestrator)
- Handle terminal resizing gracefully
- Use function attributes for persistent state between frames
- Account for 2.5x aspect ratio correction for circular shapes

## Astronomy & Cosmology

### 1. Solar System Orrery ✅
*   **Goal:** Create a visually appealing and educational representation of our solar system with realistic orbital mechanics.
*   **Scope/Features:**
    *   Central sun with corona effect
    *   All 8 planets plus dwarf planets
    *   Realistic orbital speeds (Kepler's laws)
    *   Asteroid belt between Mars and Jupiter
    *   Comet with tail pointing away from sun
    *   Kuiper belt objects
    *   Motion trails for planets
*   **Specifications:**
    *   Sun: Central stationary object with multi-character corona (`@`, `O`, `o`)
    *   Planets: Unicode symbols (☿♀⊕♂♃♄⛢♆) with distance-based speeds
    *   Orbital paths: Faint dots showing elliptical orbits
    *   Asteroid belt: Random moving particles at ~17 AU
    *   Comet: Elliptical orbit with dynamic tail length based on solar distance
    *   Aspect ratio: Multiply x-coordinates by 2.5 for circular orbits
*   **Physics:**
    *   Orbital speed ∝ 1/√distance (Kepler's third law)
    *   Comet tail always points away from sun (solar wind)
    *   Trail fading showing recent positions
*   **Tests:**
    *   **Unit Test:** Planet position calculation returns correct coordinates
    *   **Visual Test:** All planets orbit at different speeds
    *   **Visual Test:** Comet tail elongates when approaching sun
*   **Complexity:** Medium-High
*   **Status:** Done (Enhanced)
*   **File:** `solar_system.py`

### 2. Galaxy Collision ✅
*   **Goal:** Dramatic animation of two spiral galaxies merging over time.
*   **Scope/Features:**
    *   Two distinct spiral galaxies with arm structure
    *   Gravitational attraction between galaxy centers
    *   Individual stars follow galaxy movement
    *   Rotation of spiral arms
    *   Tidal distortion during close approach
*   **Specifications:**
    *   Galaxy class with position, velocity, rotation rate
    *   Stars generated in logarithmic spiral patterns
    *   Gaussian distribution for realistic star density
    *   Gravitational force: F ∝ 1/r² affects galaxy velocities
    *   Rotation: Each galaxy rotates at different rate
    *   Reset when galaxies exit screen boundaries
*   **Physics:**
    *   Two-body gravitational interaction
    *   Conservation of momentum
    *   Angular momentum in spiral structure
*   **Tests:**
    *   **Unit Test:** Galaxy.update() correctly applies gravitational force
    *   **Visual Test:** Spiral arm structure visible in both galaxies
    *   **Visual Test:** Galaxies distort as they pass through each other
*   **Complexity:** High
*   **Status:** Done
*   **File:** `galaxy_collision.py`

### 3. Black Hole Accretion Disk ✅
*   **Goal:** Mesmerizing visualization of a black hole with accretion disk and relativistic jets.
*   **Scope/Features:**
    *   Central event horizon (dark region)
    *   Glowing ergosphere
    *   Rotating accretion disk with 800+ particles
    *   Keplerian velocity (faster inner orbits)
    *   3D perspective with z-depth
    *   Relativistic jets from magnetic poles
    *   Jet turbulence particles
    *   Particle spiraling inward
*   **Specifications:**
    *   Event horizon: 4-unit radius, rendered as `@`
    *   Ergosphere: 7-unit radius, glowing halo
    *   Disk particles: Position (r, θ, z), speed ∝ 1/√r
    *   Brightness varies with z-height (3D effect)
    *   Jets: Vertical beams with pulsing intensity
    *   Jet particles: Ejected from poles with turbulent motion
*   **Physics:**
    *   Keplerian rotation: v = √(GM/r)
    *   Particles spiral inward gradually (0.99x per decay step)
    *   Relativistic beaming in jets
*   **Tests:**
    *   **Unit Test:** Particle speed correctly proportional to 1/√r
    *   **Visual Test:** Inner disk rotates visibly faster than outer
    *   **Visual Test:** Jets pulse and emit particles
*   **Complexity:** High
*   **Status:** Done (Enhanced)
*   **File:** `black_hole.py`

### 4. Pulsar Beacon ✅
*   **Goal:** Hypnotic rotating neutron star with sweeping lighthouse beams.
*   **Scope/Features:**
    *   Compact neutron star core
    *   Two opposing radio beams rotating rapidly
    *   Curved magnetic field lines
    *   Beam cone expansion with distance
    *   Intensity falloff with distance
*   **Specifications:**
    *   Core: Multi-character neutron star (`@`, `#`, `o`)
    *   Beams: Cone-shaped, expanding as they extend
    *   Magnetic fields: Curved lines radiating outward
    *   Rotation: 3 rad/s for visible motion
    *   Beam characters: Intensity-graded (`#`, `=`, `:`, `.`)
*   **Physics:**
    *   Dipole magnetic field structure
    *   Beam intensity ∝ 1/distance²
*   **Tests:**
    *   **Unit Test:** Beam position calculation at given angle
    *   **Visual Test:** Beams rotate smoothly around pulsar
    *   **Visual Test:** Beams expand into cone shape
*   **Complexity:** Medium
*   **Status:** Done
*   **File:** `pulsar.py`

### 5. Gravitational Lensing ✅
*   **Goal:** Stunning demonstration of Einstein's general relativity bending light.
*   **Scope/Features:**
    *   Massive foreground galaxy cluster
    *   Background quasars and galaxies
    *   Perfect Einstein ring from aligned source
    *   Multiple lensed images of same object
    *   Gravitational arcs (partial rings)
    *   Light deflection calculation
*   **Specifications:**
    *   Central lens: Massive galaxy cluster (elliptical shape)
    *   Background sources: 80+ distant objects
    *   Einstein ring: Perfect circle at radius R_E
    *   Lensing equation: deflection ∝ M/r²
    *   Multiple images: Primary (magnified) and secondary (demagnified)
    *   Arcs: Partial rings from misaligned sources
*   **Physics:**
    *   Deflection angle: α = 4GM/c²r
    *   Einstein radius: R_E = √(4GM*D_LS/(c²*D_L*D_S))
    *   Magnification factor for lensed images
*   **Tests:**
    *   **Unit Test:** Lensing calculation produces correct deflection
    *   **Visual Test:** Einstein ring visible around central mass
    *   **Visual Test:** Multiple images of same background source
*   **Complexity:** High
*   **Status:** Done (Enhanced)
*   **File:** `gravitational_lensing.py`

### 6. Constellation Map ✅
*   **Goal:** Educational starfield showing recognizable constellation patterns.
*   **Scope/Features:**
    *   6 major constellations (Orion, Ursa Major, Cassiopeia, Leo, Scorpius, Cygnus)
    *   Background star field for depth
    *   Constellation lines drawn with Bresenham's algorithm
    *   Constellation name label
    *   Periodic constellation changes
*   **Specifications:**
    *   Constellations: Normalized coordinates (0-1) scaled to screen
    *   Stars: Bright asterisks at constellation vertices
    *   Lines: Bresenham's line algorithm connecting stars
    *   Background: ~2% of screen filled with dim stars
    *   Label: Constellation name at bottom
    *   Change interval: ~1% chance per frame
*   **Implementation:**
    *   Pre-defined star positions and line connections
    *   Static background stars generated once
    *   Smooth line rendering without gaps
*   **Tests:**
    *   **Visual Test:** Constellation shapes recognizable
    *   **Visual Test:** Background stars provide depth
    *   **Visual Test:** Lines connect stars smoothly
*   **Complexity:** Medium
*   **Status:** Done (Enhanced)
*   **File:** `constellation_map.py`

### 7. Moon Phases ✅
*   **Goal:** Realistic lunar phase cycle with proper 3D shading.
*   **Scope/Features:**
    *   Spherical moon with limb darkening
    *   Accurate terminator line (day/night boundary)
    *   Lambertian reflection shading
    *   8 named phases (New, Crescent, Quarter, Gibbous, Full)
    *   60-second complete cycle
*   **Specifications:**
    *   Moon: Sphere rendered with distance-from-center check
    *   Lighting: Directional light rotating around moon
    *   Shading: Dot product of surface normal and light direction
    *   Characters: 8 levels from dark (`.`) to bright (`@`)
    *   Phase labels: Displayed below moon
    *   3D math: √(1-x²-y²) for z-coordinate
*   **Physics:**
    *   Lambertian reflectance: I ∝ cos(θ)
    *   Spherical geometry for surface normals
    *   Limb darkening effect
*   **Tests:**
    *   **Visual Test:** Moon appears spherical, not flat
    *   **Visual Test:** Smooth transition through all 8 phases
    *   **Visual Test:** Terminator line is curved
*   **Complexity:** Low-Medium
*   **Status:** Done
*   **File:** `moon_phases.py`

### 8. Exoplanet Transit ✅
*   **Goal:** Educational visualization of exoplanet detection method.
*   **Scope/Features:**
    *   Large host star with limb darkening
    *   Dark exoplanet transiting across star
    *   Light curve graph showing brightness dip
    *   Real-time plot of dimming during transit
    *   Transit duration: 40 seconds
*   **Specifications:**
    *   Star: Gradient-shaded circle (limb darkening)
    *   Planet: Dark circle (blocking star light)
    *   Transit path: Horizontal across star center
    *   Light curve: Real-time graph on right side
    *   Axes: Brightness (vertical) vs Time (horizontal)
    *   Current position marker on curve
*   **Physics:**
    *   Brightness reduction ∝ (R_planet/R_star)²
    *   Limb darkening: I(r) ∝ 1 - (r/R)²
    *   Contact points calculation
*   **Tests:**
    *   **Visual Test:** Planet visibly crosses star
    *   **Visual Test:** Light curve shows clear dip during transit
    *   **Visual Test:** Marker tracks current position on graph
*   **Complexity:** Medium
*   **Status:** Done
*   **File:** `exoplanet_transit.py`

### 9. Cosmic Microwave Background ✅
*   **Goal:** Artistic representation of the universe's oldest light.
*   **Scope/Features:**
    *   Multi-scale Perlin-like noise
    *   Temperature fluctuations at multiple spatial scales
    *   Primordial hot and cold spots
    *   Acoustic peak structures
    *   Scale-invariant power spectrum
*   **Specifications:**
    *   Noise generation: Custom smooth noise function
    *   Octaves: 4 scales (20, 10, 5, 2 pixel wavelengths)
    *   Amplitude: Decreasing with scale (0.5, 0.3, 0.15, 0.05)
    *   Characters: 9 levels representing temperature
    *   Hot spots: Bright characters (`@`, `#`, `*`)
    *   Cold spots: Dim characters (` `, `.`, `:`)
    *   Spot radius: 3-8 units with Gaussian falloff
*   **Physics:**
    *   CMB temperature: ~2.725K with μK fluctuations
    *   Acoustic peaks from early universe sound waves
    *   Scale-invariant Harrison-Zel'dovich spectrum
*   **Tests:**
    *   **Visual Test:** Non-uniform, cloudy appearance
    *   **Visual Test:** Structures at multiple scales
    *   **Visual Test:** Not purely random - shows coherent regions
*   **Complexity:** Medium-High
*   **Status:** Done (Enhanced)
*   **File:** `cosmic_microwave_background.py`

### 10. Stellar Nursery (Nebula) ✅
*   **Goal:** Dynamic nebula with swirling gas and star formation.
*   **Scope/Features:**
    *   Swirling gas clouds using time-based waves
    *   Pillars of Creation-style structures
    *   Proto-stars forming and fading
    *   Radial density gradient
    *   Multi-density rendering
*   **Specifications:**
    *   Gas clouds: 3-wave interference pattern
    *   Time evolution: Slowly shifting patterns
    *   Pillars: 3 vertical structures getting narrower upward
    *   Proto-stars: Random spawning, brightness lifecycle
    *   Star glow: Cross pattern around bright stars
    *   Density levels: 5 character levels (` `, `.`, `:`, `+`, `#`)
*   **Physics:**
    *   Turbulent flow in gas clouds
    *   Gravitational collapse forming stars
    *   Radiation pressure from new stars
*   **Tests:**
    *   **Visual Test:** Swirling, organic cloud patterns
    *   **Visual Test:** Pillars visible as vertical structures
    *   **Visual Test:** Stars appear and fade over time
*   **Complexity:** High
*   **Status:** Done
*   **File:** `stellar_nursery.py`

## Space Exploration

### 11. Rocket Launch Sequence ✅
*   **Goal:** Exciting vertical rocket launch with staging.
*   **Scope/Features:**
    *   3-second countdown on launchpad
    *   Multi-stage rocket (stages 1 & 2)
    *   Accelerating upward motion
    *   Booster separation at T+6s with physics
    *   Animated exhaust flames with cycling
    *   Expanding smoke cloud
    *   Ground smoke effects
*   **Specifications:**
    *   Countdown: Numbers 3, 2, 1 displayed
    *   Rocket: Multi-line ASCII art (5 lines)
    *   Stage 1: Wider base with boosters (|||)
    *   Stage 2: Smaller upper stage after separation
    *   Boosters: Fall with gravity and horizontal drift
    *   Exhaust: Animated characters (`▲`, `△`, `^`, `*`, `+`, `×`)
    *   Motion: y(t) = y₀ - ½at² (quadratic acceleration)
*   **Physics:**
    *   Constant acceleration: a = 2 units/s²
    *   Booster separation: Outward velocity ±0.3
    *   Gravity on boosters: g = 0.1
*   **Tests:**
    *   **Visual Test:** Rocket accelerates smoothly upward
    *   **Visual Test:** Boosters separate and fall
    *   **Visual Test:** Flames animate convincingly
*   **Complexity:** Medium
*   **Status:** Done (Enhanced)
*   **File:** `rocket_launch.py`

### 12. Mars Rover Explorer ✅
*   **Goal:** Autonomous rover traversing realistic Martian terrain.
*   **Scope/Features:**
    *   Rover with wheels and scientific instruments
    *   Procedurally generated terrain (hills, craters, rocks)
    *   Scrolling landscape (rover stays centered)
    *   Distance/altitude HUD display
    *   Dust particles in thin atmosphere
    *   Rock samples and landmarks
*   **Specifications:**
    *   Rover: 3-5 line ASCII art with wheels
    *   Terrain: Perlin noise for height map
    *   Craters: Circular depressions with raised rims
    *   Rocks: Random boulder placement
    *   Scroll speed: 1-2 chars per frame
    *   HUD: Distance traveled, current altitude
    *   Dust: Occasional particles drifting
*   **Implementation:**
    *   Height map: 1D Perlin noise for ground level
    *   Crater generation: Gaussian profile
    *   Rock density: Sparse (~5% coverage)
    *   Non-repeating: Continuous procedural generation
*   **Tests:**
    *   **Visual Test:** Terrain scrolls smoothly left
    *   **Visual Test:** Rover appears to drive over terrain
    *   **Visual Test:** Craters and rocks visible
*   **Complexity:** Medium-High
*   **Status:** Done
*   **File:** `mars_rover.py`

### 13. Satellite Orbit ✅
*   **Goal:** Earth from space with multiple orbiting satellites.
*   **Scope/Features:**
    *   Textured Earth with continents and oceans
    *   4 satellites (GPS, Weather, Communications, ISS)
    *   Different orbital altitudes and speeds
    *   Visible orbital paths
    *   Satellite trails showing recent positions
    *   Atmospheric glow around Earth
*   **Specifications:**
    *   Earth: Procedural continent patterns using modulo
    *   Characters: Land (`▓`), coast (`░`), water (`~`)
    *   Satellites: Labeled characters (G, W, C, █ for ISS)
    *   Orbits: Circular paths at different radii
    *   Trails: 8 fading positions behind each satellite
    *   Atmosphere: Glow ring one unit beyond surface
*   **Physics:**
    *   Orbital velocity: v ∝ 1/√r
    *   ISS: Low orbit, fastest speed
    *   Geosynchronous: High orbit, slower speed
*   **Tests:**
    *   **Visual Test:** Satellites orbit at different speeds
    *   **Visual Test:** Earth has recognizable features
    *   **Visual Test:** Trails follow satellites
*   **Complexity:** Medium
*   **Status:** Done (Enhanced)
*   **File:** `satellite_orbit.py`

### 14. Voyager's Journey ✅
*   **Goal:** Historic grand tour of the outer solar system.
*   **Scope/Features:**
    *   Voyager probe with golden record
    *   Jupiter flyby with Great Red Spot
    *   Saturn flyby with ring system
    *   Trajectory path showing gravity assists
    *   Distance and velocity readout
    *   Background stars for depth
*   **Specifications:**
    *   Probe: Small spacecraft ASCII art
    *   Jupiter: Large planet (15+ chars diameter) with bands
    *   Great Red Spot: Oval feature
    *   Saturn: Planet with horizontal ring lines
    *   Rings: Multiple concentric lines with gaps
    *   Trajectory: Curved path showing gravity assist
    *   Timeline: 60s for complete journey
*   **Implementation:**
    *   Planets scroll past fixed probe
    *   Jupiter approach: T=0-20s
    *   Saturn approach: T=30-50s
    *   Gravity assist: Probe accelerates during flyby
    *   HUD: Distance, velocity, planet name
*   **Tests:**
    *   **Visual Test:** Probe passes close to both planets
    *   **Visual Test:** Jupiter's Great Red Spot visible
    *   **Visual Test:** Saturn's rings clearly shown
*   **Complexity:** Medium-High
*   **Status:** Done
*   **File:** `voyager_journey.py`

### 15. Landing on the Moon ✅
*   **Goal:** First-person lunar landing experience.
*   **Scope/Features:**
    *   Lunar surface approaching (fractal terrain)
    *   Descent HUD with altitude, velocity, fuel
    *   Engine burn indicator
    *   Dust kicked up by exhaust
    *   Touchdown animation
    *   Shadow appearing as you descend
*   **Specifications:**
    *   Surface: Fractal/noise-based crater field
    *   Scaling: Surface detail increases as you approach
    *   HUD elements:
        - Altitude: Decreasing from 1000m to 0m
        - Vertical velocity: 0 to -50 m/s
        - Fuel: Percentage remaining
        - Engine thrust indicator
    *   Dust effect: Particles spreading from center
    *   Touchdown: "CONTACT LIGHT" message
*   **Implementation:**
    *   Surface zoom: Characters scale up over time
    *   Craters: Circular patterns with depth shading
    *   Descent rate: Exponential approach
    *   Dust: Radial particle system
    *   Duration: 30 seconds to touchdown
*   **Tests:**
    *   **Visual Test:** Surface appears to approach
    *   **Visual Test:** Altitude decreases to zero
    *   **Visual Test:** Dust effect visible near landing
*   **Complexity:** Medium-High
*   **Status:** Done
*   **File:** `moon_landing.py`

## Physics & Chemistry

### 16. Particle Accelerator ✅
*   **Goal:** Dramatic particle collision in circular collider.
*   **Scope/Features:**
    *   Large circular/oval accelerator ring
    *   Two particle beams circulating opposite directions
    *   Increasing velocity indicators
    *   Collision point with particle shower
    *   Detector segments around ring
    *   Energy readout in TeV
*   **Specifications:**
    *   Ring: Ellipse spanning most of screen
    *   Particles: Proton (P) and antiproton (P̄)
    *   Acceleration: Speed increases each lap
    *   Collision: When particles meet, create shower
    *   Shower: 20-30 particles radiating outward
    *   Detectors: Segments lighting up on impact
    *   Energy: Displayed in TeV units
*   **Physics:**
    *   Relativistic energy: E = γmc²
    *   Momentum conservation in collision
    *   Particle decay products
*   **Implementation:**
    *   Ring: Parametric ellipse equation
    *   Collision detection: Check angular positions
    *   Shower: Random angles with exponential decay
    *   Duration: 20s per collision cycle
*   **Tests:**
    *   **Visual Test:** Particles race around ring
    *   **Visual Test:** Collision creates visible shower
    *   **Visual Test:** Detectors register hits
*   **Complexity:** Medium-High
*   **Status:** Done
*   **File:** `particle_accelerator.py`

### 17. Molecular Dynamics ✅
*   **Goal:** Gas molecules demonstrating kinetic theory.
*   **Scope/Features:**
    *   30 bouncing molecules in box
    *   Elastic collisions (particle-particle and walls)
    *   Temperature visualization via speed
    *   Temperature legend
    *   Realistic physics simulation
*   **Specifications:**
    *   Box: Bordered container with corners
    *   Particles: Position, velocity for each
    *   Collision: Simple elastic (swap velocities)
    *   Temperature: Speed mapped to character
    *   Characters: `.` (cold) < `o` < `O` < `@` (hot)
    *   Legend: Temperature scale displayed
*   **Physics:**
    *   Kinetic energy: KE = ½mv²
    *   Temperature ∝ average kinetic energy
    *   Elastic collisions conserve energy
*   **Tests:**
    *   **Visual Test:** Particles bounce realistically
    *   **Visual Test:** Different characters show speed variety
    *   **Visual Test:** No particles escape box
*   **Complexity:** Medium
*   **Status:** Done (Enhanced)
*   **File:** `molecular_dynamics.py`

### 18. Wave Interference ✅
*   **Goal:** Classic double-slit interference pattern.
*   **Scope/Features:**
    *   Two point sources emitting waves
    *   Concentric circular waves
    *   Constructive interference (bright bands)
    *   Destructive interference (dark bands)
    *   Time-animated wave propagation
*   **Specifications:**
    *   Sources: Two points separated vertically
    *   Waves: Expanding circles from each source
    *   Interference: Sum amplitudes at each point
    *   Characters: Amplitude mapped to brightness
    *   Bright: `@`, `#`, `*` (constructive)
    *   Dark: ` `, `.`, `:` (destructive)
    *   Phase: Time-dependent wave equation
*   **Physics:**
    *   Wave equation: A = A₀ sin(kr - ωt)
    *   Superposition principle: A_total = A₁ + A₂
    *   Path difference determines interference
*   **Implementation:**
    *   For each point: calculate distance to both sources
    *   Phase = k*distance - ω*time
    *   Sum waves and map to character
*   **Tests:**
    *   **Visual Test:** Clear bright/dark bands visible
    *   **Visual Test:** Pattern is symmetric about centerline
    *   **Visual Test:** Waves animate outward
*   **Complexity:** High
*   **Status:** Done
*   **File:** `wave_interference.py`

### 19. Double Pendulum ✅
*   **Goal:** Mesmerizing chaotic motion demonstration.
*   **Scope/Features:**
    *   Two connected pendulum arms
    *   Realistic physics integration
    *   Path tracing for end point
    *   Fading trail showing chaos
    *   Energy conservation
*   **Specifications:**
    *   Arms: Two line segments connected
    *   Joints: Pivot point and middle joint
    *   Trail: Last 200 positions of end mass
    *   Fade: Trail alpha decreases with age
    *   Initial: Small initial displacement
    *   Integration: Runge-Kutta 4th order
*   **Physics:**
    *   Lagrangian mechanics for equations of motion
    *   Two coupled ODEs: θ₁''(θ₁, θ₂, θ₁', θ₂')
    *   Highly sensitive to initial conditions
    *   Energy: E = KE + PE conserved
*   **Implementation:**
    *   State vector: [θ₁, θ₂, ω₁, ω₂]
    *   RK4 integration with dt = 0.05
    *   Line drawing: Bresenham's algorithm
    *   Trail: Circular buffer of positions
*   **Tests:**
    *   **Unit Test:** Energy conservation within 1%
    *   **Visual Test:** Chaotic, unpredictable motion
    *   **Visual Test:** Trail shows complex paths
*   **Complexity:** High
*   **Status:** Done
*   **File:** `double_pendulum.py`

### 20. Brownian Motion ✅
*   **Goal:** Statistical mechanics visualization with multiple particles.
*   **Scope/Features:**
    *   8 particles of varying sizes
    *   Random walk with collision detection
    *   Size-dependent diffusion rates
    *   Fading trails for each particle
    *   Boundary reflections
*   **Specifications:**
    *   Particles: Sizes 1-3 units
    *   Step size: Inversely proportional to mass
    *   Trails: 40-60 positions depending on size
    *   Collisions: Particles bounce off each other
    *   Characters: `@`, `O`, `o` for different sizes
    *   Trail fade: `.` → `·` → `:` → `*`
*   **Physics:**
    *   Einstein relation: D = kT/γ
    *   Diffusion coefficient ∝ 1/radius
    *   Mean squared displacement: <r²> ∝ t
*   **Tests:**
    *   **Visual Test:** Larger particles move slower
    *   **Visual Test:** Random, jagged paths
    *   **Visual Test:** Particles repel on collision
*   **Complexity:** Medium
*   **Status:** Done (Enhanced)
*   **File:** `brownian_motion.py`

### 21. Spacetime Grid ✅
*   **Goal:** General relativity visualization with warped spacetime.
*   **Scope/Features:**
    *   2D grid representing spacetime fabric
    *   Massive object creating curvature
    *   Grid warping smoothly toward mass
    *   Optional: Test particle following geodesic
    *   Depth perception via character density
*   **Specifications:**
    *   Grid: Regular lattice of dots/plus signs
    *   Mass: Central sphere causing distortion
    *   Warp function: Δr = -GM/(r² + a²)
    *   Grid spacing: Denser near mass
    *   Characters: `+` for grid intersections
    *   Depth: Line density indicates curvature
*   **Physics:**
    *   Schwarzschild metric for curvature
    *   Geodesic equation for particle paths
    *   Proper time vs coordinate time
*   **Implementation:**
    *   Unwarped grid: Regular spacing
    *   For each point: Calculate radial distance to mass
    *   Apply radial displacement toward center
    *   Connect neighboring grid points
*   **Tests:**
    *   **Visual Test:** Grid curves inward near mass
    *   **Visual Test:** Spacing compresses near center
    *   **Visual Test:** Symmetric distortion pattern
*   **Complexity:** High
*   **Status:** Done
*   **File:** `spacetime_grid.py`

### 22. Radioactive Decay ✅
*   **Goal:** Statistical decay process with real-time graphing.
*   **Scope/Features:**
    *   Sample of ~60% of screen filled with isotopes
    *   Probabilistic decay following half-life
    *   Visible gamma radiation particles
    *   Real-time decay curve graph
    *   Exponential decay verification
*   **Specifications:**
    *   Isotopes: Grid of 'U' characters (Uranium)
    *   Decay: Random check each frame
    *   Products: `.`, `·`, `+` characters
    *   Radiation: γ particles with velocity
    *   Graph: Right 40% of screen
    *   Curve: Theoretical exponential vs actual
    *   Half-life: 15 seconds
*   **Physics:**
    *   Decay probability: p = 1 - 0.5^(dt/t₁/₂)
    *   Number remaining: N(t) = N₀ * 0.5^(t/t₁/₂)
    *   Stochastic process matches expectation
*   **Tests:**
    *   **Visual Test:** Decay rate slows over time
    *   **Visual Test:** Graph matches exponential curve
    *   **Visual Test:** Radiation particles emitted
*   **Complexity:** Medium-High
*   **Status:** Done (Enhanced)
*   **File:** `radioactive_decay.py`

### 23. Doppler Effect ✅
*   **Goal:** Frequency shift demonstration with moving source.
*   **Scope/Features:**
    *   Wave source moving horizontally
    *   Compressed wavefronts ahead (blue-shift)
    *   Stretched wavefronts behind (red-shift)
    *   Wave density visualization
    *   Labeled regions for clarity
*   **Specifications:**
    *   Source: Moving @ symbol with arrow
    *   Waves: 20 concentric circles emitted over time
    *   Ahead: Characters `≈` and `~` (compressed)
    *   Behind: Characters `~` and `·` (stretched)
    *   Density bars: Show local wave concentration
    *   Labels: "Compressed (blue-shift)" and "Stretched (red-shift)"
*   **Physics:**
    *   Observed frequency: f' = f(v_wave)/(v_wave ± v_source)
    *   Wavelength: λ' = λ(1 ± v_source/v_wave)
    *   Sonic boom at v_source = v_wave
*   **Tests:**
    *   **Visual Test:** Clear compression/rarefaction visible
    *   **Visual Test:** Density bars higher in front
    *   **Visual Test:** Symmetric pattern when source stops
*   **Complexity:** Medium-High
*   **Status:** Done (Enhanced)
*   **File:** `doppler_effect.py`

### 24. Crystal Growth ✅
*   **Goal:** Snowflake formation with hexagonal symmetry.
*   **Scope/Features:**
    *   Dendritic (branching) growth from center
    *   6-fold rotational symmetry
    *   Primary arms and secondary branches
    *   Different characters for structure types
    *   Background frost effects
*   **Specifications:**
    *   Seed: Central snowflake symbol (❄)
    *   Growth directions: 6 symmetric angles (60° apart)
    *   Main branches: Strong growth along axes
    *   Side branches: Perpendicular offshoots
    *   Characters: Core `❄`, main `*`/`✦`/`❖`, branches `+`/`·`
    *   Growth probability: Higher along symmetry axes
*   **Physics:**
    *   Diffusion-limited aggregation
    *   Hexagonal ice crystal structure
    *   Temperature-dependent branching
*   **Implementation:**
    *   Dictionary storage for sparse growth
    *   Angle alignment check for directional preference
    *   Distance-limited growth (finite size)
    *   Age tracking for character variation
*   **Tests:**
    *   **Visual Test:** Clear 6-fold symmetry
    *   **Visual Test:** Dendritic branching pattern
    *   **Visual Test:** Resembles natural snowflake
*   **Complexity:** Medium-High
*   **Status:** Done (Enhanced)
*   **File:** `crystal_growth.py`

### 25. Chemical Reaction ✅
*   **Goal:** Dynamic molecular reaction visualization.
*   **Scope/Features:**
    *   Multiple molecule types with distinct symbols
    *   Brownian motion for all molecules
    *   Collision-triggered reactions
    *   Reversible and irreversible reactions
    *   Concentration tracking over time
    *   Equilibrium establishment
*   **Specifications:**
    *   Molecules: A, B, C with different chars/sizes
    *   Reactions: A + B ⇌ C (reversible)
    *   Collision: Distance < threshold triggers check
    *   Rate constants: k_forward, k_reverse
    *   Concentration: Count each type, display as graph
    *   Equilibrium: K_eq = [C]/([A][B])
*   **Physics:**
    *   Collision theory: Rate ∝ collision frequency
    *   Activation energy: Probability of reaction
    *   Le Chatelier's principle at equilibrium
    *   Mass action law
*   **Implementation:**
    *   Particles: Position, velocity, type
    *   Update: Move + check all pairs for collisions
    *   React: Random check with rate constant
    *   Display: Symbols for each type + count graph
*   **Tests:**
    *   **Visual Test:** Molecules move and collide
    *   **Visual Test:** Product C forms over time
    *   **Visual Test:** System reaches equilibrium
    *   **Unit Test:** Mass conservation (A + B count constant)
*   **Complexity:** High
*   **Status:** Done
*   **File:** `chemical_reaction.py`

## Summary Statistics

**Total Visualizations:** 25
- **Astronomy & Cosmology:** 10/10 ✅
- **Space Exploration:** 5/5 ✅
- **Physics & Chemistry:** 10/10 ✅

**Completed:** 25/25 (100%) ✅

**By Complexity:**
- Low: 2/2 done ✅
- Medium: 12/12 done ✅
- Medium-High: 7/7 done ✅
- High: 4/4 done ✅

## Project Complete

All 25 visualizations have been successfully implemented! The project includes a comprehensive collection of scientifically-accurate ASCII art generators covering astronomy, cosmology, space exploration, and physics/chemistry topics.
