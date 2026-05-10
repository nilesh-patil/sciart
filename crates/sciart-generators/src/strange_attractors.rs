//! A turning gallery of chaotic attractors traced by swarms of glowing threads
//! (SPEC-002 `strange_attractors`).
//!
//! Six classic flows (Lorenz, Aizawa, Thomas, Chen, Halvorsen, Rössler) are integrated in
//! f64 with RK4: chaos turns f32 rounding into platform-dependent trajectories. Each ensemble of
//! 1024 particles is sampled from one long "scout" orbit, stored at bit-reversed indices so
//! every power-of-two prefix is spread along the attractor; only a prefix that grows
//! linearly with the lattice is simulated and drawn.
//!
//! Every particle leaves a short thread in a decaying [`Density`] field at braille
//! resolution (a segment from its previous dot to its current one), so the flow itself
//! builds the shape; a resize replays the flow backwards over a unit deposit's lifetime
//! (τ·ln(1/D_MIN) ≈ 0.25 s, 16 updates) so the first frame already shows whole threads.
//!
//! Between attractors the swarm morphs: both ensembles keep flowing while each displayed
//! point is interpolated between a pair matched by azimuth sector, then height band, then
//! cylindrical radius, and the blend is stretched per axis so the swarm keeps its extent.
//!
//! Raster convention (M10): the density is decayed, deposited and thresholded into
//! [`Dots`] at the end of `update` and `resize`; `render` only blits and writes the label.

use std::f64::consts::{FRAC_1_SQRT_2, PI, SQRT_2, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{draw, geom, ink, ode};

pub static META: Meta = Meta {
    name: "strange_attractors",
    category: Category::Physics,
    description: "A turning gallery of chaotic attractors traced by swarms of glowing threads",
    title: "Strange attractor · chaos",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(StrangeAttractors::default())
}

pub(crate) type V3 = [f64; 3];

/// Particles per ensemble; a power of two so bit-reversed prefixes stay spread out.
const N: usize = 1024;
const LOG2_N: u32 = N.trailing_zeros();
/// Timeline: hold then morph per attractor, 6 × 12 s = a 72 s loop (ATT-A6).
const HOLD: f64 = 8.0;
const MORPH: f64 = 4.0;
/// Scout burn-in and sample spacing, in simulated seconds of flow (× `rate` tu/s).
const BURN_S: f64 = 40.0;
const STRIDE_S: f64 = 0.15;
/// Seeded Gaussian offset of the scout's fixed start (native units).
const START_JITTER: f64 = 1e-3;
/// Scout restarts after an escape (each from the start offset by `START_JITTER·i`) before
/// it gives up and fills its slot from the samples it has, so `run` always terminates.
const MAX_RESTARTS: u32 = 8;
/// Scout RK4 steps per 1/60 s while the next slot fills in the background.
const SCOUT_STEPS_PER_TICK: f64 = 400.0;
/// Blow-up guard: a particle beyond `|x̂|∞ = ESCAPE` in the pre-rotation normalised
/// frame `x̂ = (x − c)/s` (the reference boxes are native-axis) is replaced by a jittered
/// clone of particle `(i + n/2) mod n`; the jitter (× `half`) lets chaos separate the
/// clone from its source instead of shadowing it forever.
const ESCAPE: f64 = 3.0;
const CLONE_JITTER: f64 = 1e-6;
/// Camera: yaw period, pitch wobble, eye distance (world units; |p| ≤ 1). The loop (72 s)
/// is two yaw turns, so each member's hold always sweeps the same 80° of yaw, offset by
/// its [`Spec::yaw`].
const YAW_PERIOD: f64 = 36.0;
/// Default yaw offset: two thirds of a turn, the same views as none for the 3-fold flows
/// (Thomas, Halvorsen). With holds starting every 120° of yaw it gives Aizawa's hold 0°–80°
/// (it is nearly axisymmetric about its up axis, so only the tube's side changes) and
/// Rössler's 120°–200° (the disc seen obliquely with the fold spike rising off its near
/// side), both checked in the previews.
const YAW_PHASE: f64 = 240.0 * PI / 180.0;
/// Yaw offset of the two-wing flows (Lorenz, Chen). Their projected width peaks near
/// 125°–130° of yaw (wings side by side) and falls to a third of that near 45° (a narrow
/// V, one wing behind the other); a hold sweeps 0°–80° plus the offset, so 88° keeps the
/// wings at least 70 % open throughout (ATT-A10). Both flows are odd in (x, y) about their
/// up axis, so the view at ψ + 180° is the view at ψ: 268° shows the same wings while
/// sitting only 28° from `YAW_PHASE`, so a morph beside them changes the 10 °/s turn rate
/// by at most about 1× instead of whipping the camera round (or backwards).
const WINGS_OPEN: f64 = 268.0 * PI / 180.0;
const PITCH_WOBBLE: f64 = 0.10;
const PITCH_PERIOD: f64 = 29.0;
const DISTANCE: f64 = 4.5;
/// Fractions of the [`stage_rows`] (and of the width) that each attractor's projected
/// [`Frame`] fills, centred on the stage. The frame is a 99.5th percentile, so the rare
/// excursions beyond it may be clipped at the frame edge; row h−1 is never drawn while the
/// label shows.
const FILL_H: f64 = 0.88;
const FILL_W: f64 = 0.92;
/// Thread density: decay time (s), lit threshold, top of the colour ramp.
const TAU_S: f64 = 0.1;
const D_MIN: f32 = 0.08;
const D_TOP: f64 = 4.0;
/// Thread density → ramp value (ATT-A9): `FLOOR + (TOP − FLOOR)·u^GAMMA` for the
/// log-compressed density `u ∈ [0, 1]`. The floor lifts the sparsest strands into each
/// ramp's hue (not its near-black foot); the gamma keeps the bulk saturated and lets only
/// the densest cores climb towards the near-white top, so figures glow at their cores
/// without washing every palette to the same pale grey.
const PALETTE_FLOOR: f64 = 0.26;
/// The floor for the crimson, violet and wine ramps: at equal OKLab lightness their deep
/// hues read darker on a black terminal than orange, cyan or green do.
const PALETTE_FLOOR_DEEP: f64 = 0.34;
const PALETTE_TOP: f64 = 0.95;
const PALETTE_GAMMA: f64 = 1.6;
/// Longest segment (dots, Chebyshev) drawn as a thread; longer jumps deposit only the end.
const MAX_SEGMENT: i32 = 16;
/// Deposit per dot `FAR + NEAR·near`, `near = (depth + 1)/2`: nearer threads glow brighter.
const DEPOSIT_FAR: f64 = 0.6;
const DEPOSIT_NEAR: f64 = 0.4;
/// Mid-morph dimming: deposits scale by `1 − MORPH_DIM·sin(π w)`. Light, because the
/// lerped velocity already shortens and fades the blended threads.
const MORPH_DIM: f64 = 0.2;
/// Morph pairing: equal-count azimuth sectors, each cut into equal-count height bands,
/// each band ordered by cylindrical radius. Both divide every prefix size (powers of
/// two ≥ `N_MIN`).
const SECTORS: usize = 8;
const BANDS: usize = 4;
/// Power of the spread `(⟨|p − m|^P⟩)^{1/P}` that a morph keeps at the lerp of the
/// ensembles' ([`stretch`]): high, so it tracks the silhouette rather than the bulk.
const SPREAD_P: i32 = 16;
/// `N_eff = clamp(pow2_floor(6·min(h, w/2)), 64, 1024)`: linear in the lattice size.
const N_PER_ROW: f64 = 6.0;
const N_MIN: usize = 64;
/// Flow replayed backwards at a resize, in updates of `SIM_DT`: the lifetime of a unit
/// deposit, `τ·ln(1/D_MIN)` ≈ 0.253 s (its segment `j` updates old is lit iff
/// `e^{−j·dt/τ} ≥ D_MIN`, i.e. `j ≤ 15`), so frame 0 already shows full-length threads.
const PRIME_STEPS: usize = 16;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// Parameters of the gallery flows (SPEC-002 Model).
const LORENZ: (f64, f64, f64) = (10.0, 28.0, 8.0 / 3.0);
const AIZAWA: [f64; 6] = [0.95, 0.7, 0.6, 3.5, 0.25, 0.1];
const THOMAS_B: f64 = 0.18;
const HALVORSEN_A: f64 = 1.27;
const ROSSLER: (f64, f64, f64) = (0.2, 0.2, 5.7);
/// Chen's two-wing attractor (Chen & Ueta 1999), `(a, b, c)`.
const CHEN: (f64, f64, f64) = (35.0, 3.0, 28.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attractor {
    Lorenz,
    Aizawa,
    Thomas,
    Halvorsen,
    Rossler,
    Chen,
}

/// Gallery order (ATT-A6).
pub(crate) const GALLERY: [Attractor; 6] = [
    Attractor::Lorenz,
    Attractor::Aizawa,
    Attractor::Thomas,
    Attractor::Chen,
    Attractor::Halvorsen,
    Attractor::Rossler,
];

/// Title captions (GEN-R12), indexed by `Attractor as usize`: the shown attractor names
/// the title's mechanism, `Strange attractor · Lorenz`.
const CAPTIONS: [&str; 6] = ["Lorenz", "Aizawa", "Thomas", "Halvorsen", "Rössler", "Chen"];

/// Which native direction becomes the camera's up axis (+y).
#[derive(Debug, Clone, Copy)]
enum Up {
    /// Native z (Lorenz, Aizawa, Rössler).
    Z,
    /// The (1, 1, 1) axis, so yaw spins the cyclic flows about their 3-fold axis.
    Diagonal,
}

/// Per-attractor constants (SPEC-002 Model); the tests below re-measure each one.
#[derive(Debug)]
struct Spec {
    /// Largest RK4 step (tu) and model rate (tu per simulated second, ≈ 1.2 half-widths/s).
    h_max: f64,
    rate: f64,
    /// Reference box centre and largest half-extent: `x̃ = Q(x − c)/s`.
    centre: V3,
    half: f64,
    /// `max|x̃|`, so world points `x̃/ρ` stay within the unit ball.
    rho: f64,
    up: Up,
    /// Fixed in-basin start (Aizawa has a coexisting non-chaotic attractor).
    start: V3,
    /// Camera elevation (rad) and projected framing box over the camera loop.
    pitch: f64,
    frame: Frame,
    params: &'static str,
    params_ascii: &'static str,
    /// Thread colours: a named false-colour ramp (ATT-A9).
    palette: Ramp,
    /// Extra scale on the fitted frame: below 1 when rare excursions reach far beyond
    /// the 99.5 % frame (Chen's outer loops, ATT-A4), so they mostly stay on screen.
    zoom: f64,
    /// Camera yaw offset (rad), so the hold shows the attractor's most telling side.
    yaw: f64,
}

/// A 99.5th-percentile box of the projected ensemble over the camera loop (perspective
/// included; screen y points down): `|x|/ASPECT ≤ x` and `top ≤ y − s(ψ) ≤ bottom`, world
/// units, with `s(ψ) = swing·cos 3ψ` at camera yaw ψ.
#[derive(Debug, Clone, Copy)]
struct Frame {
    x: f64,
    top: f64,
    bottom: f64,
    /// Third-harmonic swing of the box's vertical midpoint with yaw. Seen from above its
    /// 3-fold axis, Halvorsen turns like a triangle: every 120° of yaw a blade tip points
    /// up, then down, and the box midpoint swings by ±0.17 of the half-height while the
    /// centroid stays put. A fixed centre left 4 blank rows on one side and none on the
    /// other within one hold at 80×24; following the swing keeps the margins even and
    /// narrows the frame from 1.89 to 1.72. Zero for the others (swing ≤ 0.06 half-heights,
    /// with no 3-fold harmonic).
    swing: f64,
}

impl Frame {
    /// Vertical midpoint of the box at camera yaw `yaw` (world units).
    fn mid(&self, yaw: f64) -> f64 {
        (self.top + self.bottom) / 2.0 + self.swing * (3.0 * yaw).cos()
    }
}

const fn spec(k: Attractor) -> &'static Spec {
    match k {
        Attractor::Lorenz => &Spec {
            h_max: 0.01,
            rate: 0.33,
            centre: [0.0, 0.0, 24.5],
            half: 26.0,
            rho: 1.24,
            up: Up::Z,
            start: [1.0, 1.0, 1.0],
            pitch: 0.20,
            frame: Frame {
                x: 0.90,
                top: -0.66,
                bottom: 0.66,
                swing: 0.0,
            },
            params: "σ=10 ρ=28 β=8/3",
            params_ascii: "s=10 r=28 b=8/3",
            palette: Ramp::Ember,
            zoom: 1.0,
            yaw: WINGS_OPEN,
        },
        Attractor::Aizawa => &Spec {
            h_max: 0.02,
            rate: 0.69,
            centre: [0.0, 0.0, 0.74],
            half: 1.49,
            rho: 1.02,
            up: Up::Z,
            start: [0.1, 0.0, 0.0],
            pitch: 0.48,
            frame: Frame {
                x: 1.01,
                top: -0.82,
                bottom: 0.94,
                swing: 0.0,
            },
            // Its six parameters exceed the 16-char cap: none are quoted (ATT-A7).
            params: "",
            params_ascii: "",
            palette: Ramp::Ice,
            zoom: 1.0,
            yaw: YAW_PHASE,
        },
        Attractor::Thomas => &Spec {
            h_max: 0.1,
            rate: 6.5,
            centre: [0.0; 3],
            half: 4.2,
            rho: 1.13,
            up: Up::Diagonal,
            start: [0.1, 0.0, 0.0],
            pitch: 0.55,
            frame: Frame {
                x: 0.87,
                top: -1.01,
                bottom: 0.98,
                swing: 0.0,
            },
            params: "b=0.18",
            params_ascii: "b=0.18",
            palette: Ramp::Aurora,
            zoom: 1.0,
            yaw: YAW_PHASE,
        },
        Attractor::Halvorsen => &Spec {
            h_max: 0.005,
            rate: 0.31,
            centre: [-3.54; 3],
            half: 9.8,
            rho: 1.32,
            up: Up::Diagonal,
            start: [-1.0, 0.0, 0.0],
            pitch: 1.35,
            frame: Frame {
                x: 0.94,
                top: -0.80,
                bottom: 0.92,
                swing: 0.12,
            },
            params: "a=1.27",
            params_ascii: "a=1.27",
            palette: Ramp::Plasma,
            zoom: 1.0,
            yaw: YAW_PHASE,
        },
        Attractor::Rossler => &Spec {
            h_max: 0.02,
            rate: 1.6,
            centre: [1.17, -1.47, 11.36],
            half: 11.35,
            rho: 1.38,
            up: Up::Z,
            start: [1.0, 1.0, 0.0],
            pitch: 0.50,
            frame: Frame {
                x: 0.65,
                top: -0.74,
                bottom: 1.02,
                swing: 0.0,
            },
            params: "a=.2 b=.2 c=5.7",
            params_ascii: "a=.2 b=.2 c=5.7",
            palette: Ramp::Ferroin,
            zoom: 1.0,
            yaw: YAW_PHASE,
        },
        Attractor::Chen => &Spec {
            h_max: 0.004,
            rate: 0.229,
            centre: [0.0, 0.0, 28.5],
            half: 31.5,
            rho: 1.22,
            up: Up::Z,
            start: [-10.0, 0.0, 37.0],
            pitch: 0.20,
            frame: Frame {
                x: 0.86,
                top: -0.49,
                bottom: 0.60,
                swing: 0.0,
            },
            params: "a=35 b=3 c=28",
            params_ascii: "a=35 b=3 c=28",
            palette: Ramp::HAlpha,
            zoom: 0.85,
            yaw: WINGS_OPEN,
        },
    }
}

// ---------------------------------------------------------------------------------------
// Flows (ATT-A3; their Jacobians live with the tests)
// ---------------------------------------------------------------------------------------

/// Thomas' cyclically symmetric flow `ẋ = sin y − bx` (and cyclic).
pub(crate) fn thomas(b: f64, [x, y, z]: V3) -> V3 {
    [y.sin() - b * x, z.sin() - b * y, x.sin() - b * z]
}

/// Halvorsen's cyclically symmetric flow `ẋ = −ax − 4y − 4z − y²` (and cyclic).
pub(crate) fn halvorsen(a: f64, [x, y, z]: V3) -> V3 {
    [
        -a * x - 4.0 * y - 4.0 * z - y * y,
        -a * y - 4.0 * z - 4.0 * x - z * z,
        -a * z - 4.0 * x - 4.0 * y - x * x,
    ]
}

/// The gallery vector field `ẋ = f(x)` in native units.
pub(crate) fn field(k: Attractor, p: V3) -> V3 {
    let [x, y, z] = p;
    match k {
        Attractor::Lorenz => {
            let (s, r, b) = LORENZ;
            [s * (y - x), x * (r - z) - y, x * y - b * z]
        }
        Attractor::Aizawa => {
            let [a, b, c, d, e, f] = AIZAWA;
            [
                (z - b) * x - d * y,
                d * x + (z - b) * y,
                c + a * z - z * z * z / 3.0 - (x * x + y * y) * (1.0 + e * z) + f * z * x * x * x,
            ]
        }
        Attractor::Thomas => thomas(THOMAS_B, p),
        Attractor::Halvorsen => halvorsen(HALVORSEN_A, p),
        Attractor::Rossler => {
            let (a, b, c) = ROSSLER;
            [-y - z, x + a * y, b + z * (x - c)]
        }
        Attractor::Chen => {
            let (a, b, c) = CHEN;
            [a * (y - x), (c - a) * x - x * z + c * y, x * y - b * z]
        }
    }
}

// ---------------------------------------------------------------------------------------
// Normalisation and world coordinates
// ---------------------------------------------------------------------------------------

/// Box-normalised coordinates `(x − c)/s`, in native axis order (ATT-A4 extents).
pub(crate) fn normalised(k: Attractor, x: V3) -> V3 {
    let s = spec(k);
    std::array::from_fn(|i| (x[i] - s.centre[i]) / s.half)
}

/// `Q_k`: a rotation taking the attractor's symmetry axis onto world +y.
fn orient(up: Up, [x, y, z]: V3) -> V3 {
    match up {
        Up::Z => [x, z, -y],
        Up::Diagonal => {
            let (r3, r6) = (1.0 / 3f64.sqrt(), 1.0 / 6f64.sqrt());
            [
                (x - y) * FRAC_1_SQRT_2,
                (x + y + z) * r3,
                (2.0 * z - x - y) * r6,
            ]
        }
    }
}

/// World position `Q(x − c)/(s·ρ)`: inside the unit ball for every attractor.
pub(crate) fn world(k: Attractor, x: V3) -> V3 {
    let rho = spec(k).rho;
    orient(spec(k).up, normalised(k, x)).map(|v| v / rho)
}

/// Blow-up test: non-finite, or beyond `ESCAPE` box half-widths on some axis.
fn escaped(k: Attractor, x: V3) -> bool {
    !normalised(k, x).iter().all(|v| v.abs() <= ESCAPE)
}

/// Component-wise `lerp`, exact at `w = 0` and `w = 1` (ATT-A6).
pub(crate) fn blend(a: V3, b: V3, w: f64) -> V3 {
    std::array::from_fn(|i| ease::lerp(a[i], b[i], w))
}

/// Index `i` with its low `LOG2_N` bits reversed.
fn bit_reversed(i: usize) -> usize {
    (i as u32).reverse_bits() as usize >> (u32::BITS - LOG2_N)
}

// ---------------------------------------------------------------------------------------
// Scout: one long orbit sampled into an ensemble
// ---------------------------------------------------------------------------------------

/// Samples an ensemble from one orbit: burn-in, then one sample every `STRIDE_S` s of flow,
/// stored at bit-reversed indices. Fixed RK4 steps, so the result does not depend on how
/// the work is split across updates.
#[derive(Debug)]
pub(crate) struct Scout {
    k: Attractor,
    x: V3,
    steps: usize,
    filled: usize,
    restarts: u32,
}

impl Scout {
    pub(crate) fn new(k: Attractor, rng: &mut impl Rng) -> Self {
        let start = spec(k).start;
        Self {
            k,
            x: start.map(|v| gaussian(rng, v, START_JITTER)),
            steps: 0,
            filled: 0,
            restarts: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn done(&self) -> bool {
        self.filled == N
    }

    /// Advance by at most `budget` RK4 steps, writing samples into `slot` (length `N`).
    pub(crate) fn run(&mut self, slot: &mut [V3], budget: usize) {
        let k = self.k;
        self.run_field(slot, budget, |p| field(k, p));
    }

    /// [`Scout::run`] with the vector field as a parameter (the tests inject escapes).
    fn run_field(&mut self, slot: &mut [V3], mut budget: usize, mut f: impl FnMut(V3) -> V3) {
        let (k, s) = (self.k, spec(self.k));
        let burn = (BURN_S * s.rate / s.h_max).round() as usize;
        let stride = ((STRIDE_S * s.rate / s.h_max).round() as usize).max(1);
        while budget > 0 && self.filled < N {
            ode::rk4_step(&mut self.x, 0.0, s.h_max, |_, p| f(*p));
            budget -= 1;
            if escaped(k, self.x) {
                self.restarts += 1;
                if self.restarts > MAX_RESTARTS {
                    // Unreachable with the gallery's in-basin starts, but `run(.., MAX)`
                    // must terminate: repeat the samples taken (the start if none).
                    for i in self.filled..N {
                        slot[bit_reversed(i)] = match self.filled {
                            0 => s.start,
                            m => slot[bit_reversed(i % m)],
                        };
                    }
                    self.filled = N;
                    return;
                }
                // Restart, burn-in included, so no transient sample is kept; each restart
                // is offset, so a start that escaped is not replayed exactly.
                let offset = START_JITTER * f64::from(self.restarts);
                (self.x, self.steps) = (s.start.map(|v| v + offset), 0);
                continue;
            }
            self.steps += 1;
            if self.steps > burn && (self.steps - burn) % stride == 0 {
                slot[bit_reversed(self.filled)] = self.x;
                self.filled += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Gallery: two ensemble slots driven by the Timeline
// ---------------------------------------------------------------------------------------

/// The simulation: the current ensemble, the next one (filled by the scout during the
/// hold, co-simulated during the morph) and the looping Timeline. Display-independent.
#[derive(Debug)]
pub(crate) struct Gallery {
    timeline: Timeline,
    /// Timeline clock (s), starting at the seeded attractor's hold.
    t: f64,
    /// Native positions; `slots[cur]` holds `GALLERY[at]`, the other `GALLERY[at + 1]`.
    slots: [Vec<V3>; 2],
    cur: usize,
    at: usize,
    morphing: bool,
    /// Smoothstepped morph progress (0 in a hold).
    weight: f64,
    scout: Scout,
    /// Simulated and displayed prefix `N_eff`, and the value to adopt at the next hold (a
    /// running morph keeps its pairing intact).
    n: usize,
    pending: usize,
    /// Per world axis, the centre and gain applied to the blend at the current weight
    /// ([`stretch`]), measured after every morph step.
    stretch: [(f64, f64); 3],
    /// Pairing buffers and the stretch's world positions (sized at build so a morph does
    /// not allocate).
    keys: [Vec<Key>; 2],
    ranks: [Vec<u32>; 2],
    scratch: [Vec<V3>; 2],
}

/// Pairing key of a point: azimuth about the vertical axis, height, cylindrical radius.
type Key = [f64; 3];

impl Gallery {
    /// Start at the beginning of `GALLERY[first]`'s hold with display prefix `n_eff`; the
    /// current slot is filled synchronously.
    pub(crate) fn new(first: usize, n_eff: usize, rng: &mut impl Rng) -> Self {
        let first = first % GALLERY.len();
        let phases: Vec<_> = GALLERY
            .iter()
            .flat_map(|&k| [(CAPTIONS[k as usize], HOLD), ("morph", MORPH)])
            .collect();
        let mut slots = [vec![[0.0; 3]; N], vec![[0.0; 3]; N]];
        Scout::new(GALLERY[first], rng).run(&mut slots[0], usize::MAX);
        let n_eff = n_eff.clamp(1, N);
        Self {
            timeline: Timeline::new(&phases),
            t: (HOLD + MORPH) * first as f64,
            slots,
            cur: 0,
            at: first,
            morphing: false,
            weight: 0.0,
            scout: Scout::new(GALLERY[(first + 1) % GALLERY.len()], rng),
            n: n_eff,
            pending: n_eff,
            stretch: [(0.0, 1.0); 3],
            keys: [vec![[0.0; 3]; N], vec![[0.0; 3]; N]],
            ranks: [vec![0; N], vec![0; N]],
            scratch: [vec![[0.0; 3]; N], vec![[0.0; 3]; N]],
        }
    }

    fn next(&self) -> usize {
        (self.at + 1) % GALLERY.len()
    }

    /// Timeline clock (s). It is also the camera's clock, so every member's hold is seen
    /// from the same yaws whatever the seeded first member.
    pub(crate) fn t(&self) -> f64 {
        self.t
    }

    /// Simulated (and displayed) prefix.
    pub(crate) fn n(&self) -> usize {
        self.n
    }

    /// Change the display prefix now, or at the next hold if a morph is running.
    pub(crate) fn set_n(&mut self, n_eff: usize) {
        self.pending = n_eff.clamp(1, N);
        if !self.morphing {
            self.n = self.pending;
        }
    }

    /// `(from, to, w)`: the attractors being shown and the smoothstepped morph weight.
    pub(crate) fn pair(&self) -> (Attractor, Attractor, f64) {
        (GALLERY[self.at], GALLERY[self.next()], self.weight)
    }

    /// World position of displayed particle `i < n`.
    pub(crate) fn displayed(&self, i: usize) -> V3 {
        self.shown(self.slots[self.cur][i], self.slots[1 - self.cur][i])
    }

    /// World position of a particle at native positions `a` (current) and `b` (next).
    fn shown(&self, a: V3, b: V3) -> V3 {
        let (from, to, w) = self.pair();
        let a = world(from, a);
        if self.morphing && w > 0.0 && w < 1.0 {
            let p = blend(a, world(to, b), w);
            std::array::from_fn(|j| {
                let (centre, gain) = self.stretch[j];
                centre + gain * (p[j] - centre)
            })
        } else if self.morphing {
            // The endpoints are exact (ATT-A6).
            blend(a, world(to, b), w)
        } else {
            a
        }
    }

    /// Measure [`Gallery::stretch`] over the displayed prefix at the current weight (a
    /// morph only).
    fn measure(&mut self) {
        let (from, to, w) = self.pair();
        let n = self.n;
        let [wa, wb] = &mut self.scratch;
        for (dst, &p) in wa[..n].iter_mut().zip(&self.slots[self.cur][..n]) {
            *dst = world(from, p);
        }
        for (dst, &p) in wb[..n].iter_mut().zip(&self.slots[1 - self.cur][..n]) {
            *dst = world(to, p);
        }
        self.stretch = stretch(&wa[..n], &wb[..n], w);
    }

    /// Displayed positions of particle `i` at `1..=steps` earlier updates of `dt`, newest
    /// first: the flow integrated backwards from the current state (weight, stretch and
    /// camera held). Stops early if a backward orbit leaves the box.
    pub(crate) fn rewind(&self, i: usize, dt: f64, steps: usize, mut f: impl FnMut(V3)) {
        let (from, to, _) = self.pair();
        let (mut a, mut b) = (self.slots[self.cur][i], self.slots[1 - self.cur][i]);
        for _ in 0..steps {
            let a_ok = backward(&mut a, from, dt);
            let b_ok = !self.morphing || backward(&mut b, to, dt);
            if !(a_ok && b_ok) {
                return;
            }
            f(self.shown(a, b));
        }
    }

    /// Advance by `dt > 0` s (finite; the scene's `update` filters every other value).
    pub(crate) fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        debug_assert!(dt.is_finite() && dt > 0.0, "dt = {dt}");
        self.t += dt;
        let beat = self.sync(rng);
        self.weight = if self.morphing {
            ease::smoothstep(beat.progress)
        } else {
            0.0
        };

        let (from, to, n) = (GALLERY[self.at], GALLERY[self.next()], self.n);
        let (cur, other) = slots_mut(&mut self.slots, self.cur);
        advance(&mut cur[..n], from, dt, rng);
        if self.morphing {
            advance(&mut other[..n], to, dt, rng);
            self.measure();
        } else {
            let budget = (SCOUT_STEPS_PER_TICK * dt / SIM_DT).ceil() as usize;
            self.scout.run(other, budget);
        }
    }

    /// Walk the state machine to the Timeline's current phase (one transition per call
    /// at dt ≤ 0.25, but correct for any jump); returns that phase's beat.
    fn sync(&mut self, rng: &mut impl Rng) -> Beat {
        let beat = self.timeline.at(self.t);
        let target = (beat.index / 2, beat.index % 2 == 1);
        for _ in 0..2 * GALLERY.len() {
            if (self.at, self.morphing) == target {
                break;
            }
            if self.morphing {
                self.cur = 1 - self.cur;
                self.at = self.next();
                self.morphing = false;
                self.n = self.pending;
                self.scout = Scout::new(GALLERY[self.next()], rng);
            } else {
                self.begin_morph();
            }
        }
        beat
    }

    /// Complete the next ensemble and pair the two.
    fn begin_morph(&mut self) {
        let (_, other) = slots_mut(&mut self.slots, self.cur);
        self.scout.run(other, usize::MAX);
        self.pair_ensembles();
        self.morphing = true;
        self.measure();
    }

    /// Reorder the next ensemble's prefix so that its `j`-th point in pairing order sits at
    /// the index of the current ensemble's `j`-th point: the display index `i` then pairs
    /// `a_i ↔ b_i` without re-indexing the points already on screen.
    ///
    /// Pairing order ([`pairing_order`]): equal-count azimuth sectors, equal-count height
    /// bands within each, radius within each band. Equal counts keep sector `j` of one
    /// attractor paired with sector `j` of the other however unevenly their mass is spread
    /// (Lorenz's two wings), and matching height and radius ranks starts each lerped pair
    /// near both of its ends. That holds only at the morph start: both ensembles keep
    /// flowing, so the pairs decorrelate within about a second, and the size is kept by
    /// [`stretch`], not by the pairing.
    fn pair_ensembles(&mut self) {
        let n = self.n;
        let (from, to, _) = self.pair();
        let (a, b) = slots_mut(&mut self.slots, self.cur);
        let [keys_a, keys_b] = &mut self.keys;
        let [rank_a, rank_b] = &mut self.ranks;
        pairing_order(from, &a[..n], &mut keys_a[..n], &mut rank_a[..n]);
        pairing_order(to, &b[..n], &mut keys_b[..n], &mut rank_b[..n]);
        let scratch = &mut self.scratch[0][..n];
        for (dst, &j) in scratch.iter_mut().zip(&rank_b[..n]) {
            *dst = b[j as usize];
        }
        for (&i, &p) in rank_a[..n].iter().zip(scratch.iter()) {
            b[i as usize] = p;
        }
    }
}

/// Per world axis, the centre and gain that keep a blend `lerp(a_i, b_i, w)` as wide as
/// the ensembles it blends: the lerped mean, and the factor that brings the blend's
/// spread `(⟨|p − mean|^P⟩)^{1/P}` (`P = SPREAD_P`) to the lerp of the ensembles' own.
///
/// Both ensembles keep flowing, so within about a second of the morph start most pairs
/// are uncorrelated, and the lerp of two independent clouds is narrower (its spread falls
/// to `√((1−w)² + w²)` ≈ 0.71 at w = ½ for equal variances): a knot. A variance match
/// (P = 2) overshoots, because the extremes decorrelate last (pairs at the top of both
/// stay there while the bulk mixes) and a blend has thinner tails than the shapes it
/// averages: it pushed Aizawa → Thomas past the frame (80×24: 5.6 edge-row cells per
/// morph frame, against 0.07 in holds). A high power tracks the silhouette instead (0.7
/// at P = 16) while staying smooth in every point, where an order statistic jumped by 0.3
/// in one update when a fast particle overtook. Per axis, since one gain for all axes
/// turns uncorrelated extremes into corners the shapes never have (4.8 edge cells at
/// P = 8). The gain is clamped to `[1, √2]`, so `|p − mean|` stays within √2 of the
/// ensembles' (ATT-A5).
fn stretch(a: &[V3], b: &[V3], w: f64) -> [(f64, f64); 3] {
    let n = a.len() as f64;
    std::array::from_fn(|j| {
        let mean = |p: &[V3]| p.iter().map(|q| q[j]).sum::<f64>() / n;
        let (ma, mb) = (mean(a), mean(b));
        let centre = ease::lerp(ma, mb, w);
        let (mut sa, mut sb, mut sm) = (0.0, 0.0, 0.0);
        for (p, q) in a.iter().zip(b) {
            sa += (p[j] - ma).powi(SPREAD_P);
            sb += (q[j] - mb).powi(SPREAD_P);
            sm += (ease::lerp(p[j], q[j], w) - centre).powi(SPREAD_P);
        }
        let norm = |s: f64| (s / n).powf(1.0 / f64::from(SPREAD_P));
        let (qa, qb, qm) = (norm(sa), norm(sb), norm(sm));
        let gain = ease::lerp(qa, qb, w) / qm;
        let gain = if gain.is_finite() {
            gain.clamp(1.0, SQRT_2)
        } else {
            1.0
        };
        (centre, gain)
    })
}

/// Fill `rank` with the indices of `slot` in pairing order: by azimuth, then within each
/// of `SECTORS` equal-count runs by height, then within each of `BANDS` equal-count bands
/// by cylindrical radius; ties fall to the index. Keys are computed once per point, and
/// `sort_unstable_by` does not allocate.
fn pairing_order(k: Attractor, slot: &[V3], keys: &mut [Key], rank: &mut [u32]) {
    for (i, ((key, r), &x)) in keys.iter_mut().zip(rank.iter_mut()).zip(slot).enumerate() {
        let [x, y, z] = world(k, x);
        *key = [z.atan2(x), y, x.hypot(z)];
        *r = i as u32;
    }
    let keys: &[Key] = keys;
    let by = |axis: usize| {
        move |&i: &u32, &j: &u32| {
            let (p, q) = (keys[i as usize][axis], keys[j as usize][axis]);
            p.total_cmp(&q).then(i.cmp(&j))
        }
    };
    let sector = (rank.len() / SECTORS).max(1);
    let band = (sector / BANDS).max(1);
    rank.sort_unstable_by(by(0));
    for run in rank.chunks_mut(sector) {
        run.sort_unstable_by(by(1));
        for run in run.chunks_mut(band) {
            run.sort_unstable_by(by(2));
        }
    }
}

/// `(current, other)` ensemble slots.
fn slots_mut(slots: &mut [Vec<V3>; 2], cur: usize) -> (&mut [V3], &mut [V3]) {
    let [a, b] = slots;
    if cur == 0 { (a, b) } else { (b, a) }
}

/// Integrate one particle backwards by `dt` s of flow (the reversed field, forwards);
/// false if it left the box. Short spans only: the reversed flow expands volumes.
fn backward(p: &mut V3, k: Attractor, dt: f64) -> bool {
    let s = spec(k);
    ode::integrate(p, 0.0, dt * s.rate, s.h_max, |_, x| {
        field(k, *x).map(|v| -v)
    });
    !escaped(k, *p)
}

/// Integrate an ensemble prefix for `dt` s, then replace escaped particles.
fn advance(slot: &mut [V3], k: Attractor, dt: f64, rng: &mut impl Rng) {
    let s = spec(k);
    for p in slot.iter_mut() {
        ode::integrate(p, 0.0, dt * s.rate, s.h_max, |_, x| field(k, *x));
    }
    let n = slot.len();
    for i in 0..n {
        if escaped(k, slot[i]) {
            // The partner half an ensemble away, else the next finite particle; the start
            // only if all have escaped (it is off the attractor, e.g. by Lorenz's saddle).
            let src = std::iter::once((i + n / 2) % n)
                .chain((1..n).map(|d| (i + d) % n))
                .map(|j| slot[j])
                .find(|&p| !escaped(k, p));
            slot[i] = match src {
                Some(p) => p.map(|v| gaussian(rng, v, CLONE_JITTER * s.half)),
                None => s.start,
            };
        }
    }
}

// ---------------------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------------------

/// Simulated prefix for a lattice: linear in its size (threads grow with it).
pub(crate) fn n_eff(size: Size) -> usize {
    let rows = f64::from(size.height()).min(f64::from(size.width()) / 2.0);
    let n = (N_PER_ROW * rows).clamp(1.0, N as f64) as usize;
    (1usize << n.ilog2()).clamp(N_MIN, N)
}

/// Rows per world unit that fit attractor `k`'s [`Frame`] into the [`stage_rows`]; positive
/// even at zero area, so the morph's log-lerp of two fits stays finite.
fn fit(k: Attractor, size: Size) -> f64 {
    let f = spec(k).frame;
    let (w, rows) = (f64::from(size.width()), f64::from(stage_rows(size)));
    (spec(k).zoom * (FILL_H * rows / (f.bottom - f.top)).min(FILL_W * w / (2.0 * ASPECT * f.x)))
        .max(f64::MIN_POSITIVE)
}

/// Whether the label shows (and row h−1 is kept free of threads).
fn label_shown(size: Size) -> bool {
    size.width() >= 12 && size.height() >= 8
}

/// The rows the figure is framed in and centred on: all of them, less the label's.
fn stage_rows(size: Size) -> u16 {
    size.height() - u16::from(label_shown(size))
}

/// Thread colour: density log-compressed by `ln_top = ln(1 + D_TOP/D_MIN)` onto the shown
/// attractor's palette (ATT-A9).
fn thread_ink(palette: Ramp, d: f32, ln_top: f64) -> Ink {
    let u = ((1.0 + f64::from(d) / f64::from(D_MIN)).ln() / ln_top).clamp(0.0, 1.0);
    let floor = match palette {
        Ramp::HAlpha | Ramp::Plasma | Ramp::Ferroin => PALETTE_FLOOR_DEEP,
        Ramp::Ember | Ramp::Ice | Ramp::Aurora | Ramp::Starlight => PALETTE_FLOOR,
    };
    ink::ramp(
        palette,
        floor + (PALETTE_TOP - floor) * u.powf(PALETTE_GAMMA),
    )
}

/// Camera yaw (rad) at gallery clock `t`, `w` of the way from `from`'s offset to `to`'s.
fn camera_yaw(t: f64, from: Attractor, to: Attractor, w: f64) -> f64 {
    TAU * t / YAW_PERIOD + ease::lerp(spec(from).yaw, spec(to).yaw, w)
}

/// Where and how a particle deposits this update.
#[derive(Debug, Clone, Copy)]
struct Pen {
    proj: Projector,
    centre: (f64, f64),
    scale: f64,
    /// Morph dimming factor.
    dim: f64,
    /// First dot row that is not drawn (the label row), or `i32::MAX`.
    floor: i32,
}

impl Pen {
    /// Dot and deposit weight of a world point, if it is in front of the eye.
    fn mark(&self, p: V3) -> Option<((i32, i32), f64)> {
        let (x, y, depth) = self.proj.project(p, self.centre, self.scale)?;
        let (dx, dy) = Dots::from_cells(x, y);
        let dot = (dx.round() as i32, dy.round() as i32);
        let near = ((depth + 1.0) / 2.0).clamp(0.0, 1.0);
        Some((dot, self.dim * (DEPOSIT_FAR + DEPOSIT_NEAR * near)))
    }

    /// Deposit the thread from `from` (deposited earlier, so skipped unless the particle is
    /// still on it: dwelling brightens a dot, as a long exposure would) to `to`. Longer
    /// jumps than `MAX_SEGMENT` deposit only their end; the label row is never drawn.
    fn thread(&self, density: &mut Density, from: Option<(i32, i32)>, to: (i32, i32), w: f32) {
        let mut put = |(x, y): (i32, i32)| {
            if y < self.floor {
                density.deposit(f64::from(x), f64::from(y), w);
            }
        };
        match from {
            Some(p) if chebyshev(p, to) <= MAX_SEGMENT => {
                let skip = usize::from(p != to);
                geom::line(p.0, p.1, to.0, to.1).skip(skip).for_each(put);
            }
            _ => put(to),
        }
    }
}

#[derive(Debug)]
struct StrangeAttractors {
    /// Built at the first resize (the seeded start needs `ctx.rng`).
    gallery: Option<Gallery>,
    size: Size,
    density: Density,
    dots: Dots,
    /// Previous dot of each displayed particle (`None`: forgotten, no thread segment).
    prev: Vec<Option<(i32, i32)>>,
}

impl Default for StrangeAttractors {
    fn default() -> Self {
        Self {
            gallery: None,
            size: Size::new(1, 1),
            density: Density::default(),
            dots: Dots::default(),
            prev: vec![None; N],
        }
    }
}

impl StrangeAttractors {
    /// Camera, centre and scale for the current morph state: the scale is log-lerped and
    /// the frame's (yaw-dependent) middle held mid-stage.
    fn view(&self, g: &Gallery) -> (Projector, (f64, f64), f64) {
        let (from, to, w) = g.pair();
        let pitch = ease::lerp(spec(from).pitch, spec(to).pitch, w)
            + PITCH_WOBBLE * (TAU * g.t() / PITCH_PERIOD).sin();
        let scale = ease::lerp(fit(from, self.size).ln(), fit(to, self.size).ln(), w).exp();
        let yaw = camera_yaw(g.t(), from, to, w);
        let mid = |k: Attractor| spec(k).frame.mid(yaw);
        let cam = Camera {
            yaw,
            pitch,
            distance: DISTANCE,
        };
        let centre = (
            f64::from(self.size.width()) / 2.0,
            f64::from(stage_rows(self.size)) / 2.0 - scale * ease::lerp(mid(from), mid(to), w),
        );
        (cam.projector(), centre, scale)
    }

    fn pen(&self, g: &Gallery) -> Pen {
        let (proj, centre, scale) = self.view(g);
        let floor = if label_shown(self.size) {
            Dots::CELL_H * (i32::from(self.size.height()) - 1)
        } else {
            i32::MAX
        };
        Pen {
            proj,
            centre,
            scale,
            dim: 1.0 - MORPH_DIM * (PI * g.pair().2).sin(),
            floor,
        }
    }

    /// Deposit each particle's thread segment, then threshold the density into `Dots`.
    fn rasterise(&mut self) {
        let Some(g) = &self.gallery else {
            return;
        };
        // Zero area: nothing to plot (the density holds no dots), so skip the projections.
        if self.size.area() == 0 {
            return;
        }
        let pen = self.pen(g);
        for (i, prev) in self.prev[..g.n()].iter_mut().enumerate() {
            let Some((dot, w)) = pen.mark(g.displayed(i)) else {
                *prev = None;
                continue;
            };
            pen.thread(&mut self.density, *prev, dot, w as f32);
            *prev = Some(dot);
        }
        self.threshold();
    }

    /// Rebuild the threads after a resize: each particle's last `PRIME_STEPS` updates are
    /// replayed by integrating its flow backwards, and each segment is deposited with the
    /// decay it would have had since, so the first frame already shows threads.
    fn prime(&mut self) {
        let Some(g) = &self.gallery else {
            return;
        };
        if self.size.area() == 0 {
            return;
        }
        let pen = self.pen(g);
        let fade = (-SIM_DT / TAU_S).exp();
        for (i, prev) in self.prev[..g.n()].iter_mut().enumerate() {
            let Some((head, w)) = pen.mark(g.displayed(i)) else {
                *prev = None;
                continue;
            };
            let (mut to, mut age) = (head, 1.0);
            let density = &mut self.density;
            g.rewind(i, SIM_DT, PRIME_STEPS, |p| {
                if let Some((from, _)) = pen.mark(p) {
                    pen.thread(density, Some(from), to, (w * age) as f32);
                    (to, age) = (from, age * fade);
                }
            });
            // The oldest dot is the start of a segment never deposited (as in `draw`).
            *prev = Some(head);
        }
        self.threshold();
    }

    fn threshold(&mut self) {
        self.dots.clear();
        let Some(k) = self.shown() else {
            return;
        };
        let (palette, ln_top) = (spec(k).palette, (1.0 + D_TOP / f64::from(D_MIN)).ln());
        self.density
            .to_dots(&mut self.dots, D_MIN, |d| thread_ink(palette, d, ln_top));
    }

    /// The attractor the swarm mostly shows: the target once a morph is half done.
    fn shown(&self) -> Option<Attractor> {
        let (from, to, weight) = self.gallery.as_ref()?.pair();
        Some(if weight < 0.5 { from } else { to })
    }

    /// The shown attractor's parameter set, bottom-right and clear of the title, whose
    /// caption already names the attractor (GEN-R13); whole or not at all.
    fn label(&self, canvas: &mut Canvas) {
        if !label_shown(canvas.size()) {
            return;
        }
        let Some(k) = self.shown() else {
            return;
        };
        let s = spec(k);
        let room = title::readout_room(canvas.size(), &Title::of(self));
        // Both variants have the same char count (ATT-A7), so one check serves both modes.
        if s.params.is_empty() || s.params_ascii.chars().count() > room {
            return;
        }
        let params = match canvas.mode() {
            GlyphMode::Unicode => s.params,
            GlyphMode::Ascii => s.params_ascii,
        };
        let y = i32::from(canvas.size().height()) - 1;
        draw::label_right(canvas, y, params, LABEL_INK);
    }
}

fn chebyshev(a: (i32, i32), b: (i32, i32)) -> i32 {
    a.0.abs_diff(b.0)
        .max(a.1.abs_diff(b.1))
        .try_into()
        .unwrap_or(i32::MAX)
}

impl Generator for StrangeAttractors {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.density.resize(size);
        self.dots.resize(size);
        let n = n_eff(size);
        match &mut self.gallery {
            Some(g) => g.set_n(n),
            None => {
                let first = ctx.rng.random_range(0..GALLERY.len());
                self.gallery = Some(Gallery::new(first, n, &mut ctx.rng));
            }
        }
        self.prime();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let Some(g) = &mut self.gallery else {
            return;
        };
        // The one dt guard: it also keeps the gallery (camera) clock and the decay finite.
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let n = g.n();
        g.step(dt, &mut ctx.rng);
        // Particles that join the prefix (at a hold start) have no thread yet; their
        // stale previous dots must not draw a jump.
        if g.n() != n {
            self.prev[n.min(g.n())..].fill(None);
        }
        self.density.decay((-dt / TAU_S).exp() as f32);
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        self.label(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        self.shown().map(|k| CAPTIONS[k as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type M3 = [[f64; 3]; 3];

    fn thomas_jacobian(b: f64, [x, y, z]: V3) -> M3 {
        [[-b, y.cos(), 0.0], [0.0, -b, z.cos()], [x.cos(), 0.0, -b]]
    }

    fn halvorsen_jacobian(a: f64, [x, y, z]: V3) -> M3 {
        [
            [-a, -4.0 - 2.0 * y, -4.0],
            [-4.0, -a, -4.0 - 2.0 * z],
            [-4.0 - 2.0 * x, -4.0, -a],
        ]
    }

    /// Analytic Jacobian `∂f_i/∂x_j` of [`field`] (test-only: it validates the flows).
    fn jacobian(k: Attractor, p: V3) -> M3 {
        let [x, y, z] = p;
        match k {
            Attractor::Lorenz => {
                let (s, r, b) = LORENZ;
                [[-s, s, 0.0], [r - z, -1.0, -x], [y, x, -b]]
            }
            Attractor::Aizawa => {
                let [a, b, _, d, e, f] = AIZAWA;
                [
                    [z - b, -d, x],
                    [d, z - b, y],
                    [
                        -2.0 * x * (1.0 + e * z) + 3.0 * f * z * x * x,
                        -2.0 * y * (1.0 + e * z),
                        a - z * z - e * (x * x + y * y) + f * x * x * x,
                    ],
                ]
            }
            Attractor::Thomas => thomas_jacobian(THOMAS_B, p),
            Attractor::Halvorsen => halvorsen_jacobian(HALVORSEN_A, p),
            Attractor::Rossler => {
                let (a, _, c) = ROSSLER;
                [[0.0, -1.0, -1.0], [1.0, a, 0.0], [z, 0.0, x - c]]
            }
            Attractor::Chen => {
                let (a, b, c) = CHEN;
                [[-a, a, 0.0], [c - a - z, c, -x], [y, x, -b]]
            }
        }
    }

    fn mat_vec(m: &M3, v: V3) -> V3 {
        std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
    }

    /// Largest Lyapunov exponent by the tangent-vector method (Benettin 1980): RK4 on the
    /// 6-D (state, tangent) system with step `h`, renormalising the tangent after every step
    /// and averaging `ln|v|` over `t` tu after `burn` tu (ATT-A1, ATT-A2).
    fn lyapunov_max(
        f: impl Fn(V3) -> V3,
        jac: impl Fn(V3) -> M3,
        start: V3,
        h: f64,
        burn: f64,
        t: f64,
    ) -> f64 {
        let v0 = 1.0 / 3f64.sqrt();
        let mut y = [start[0], start[1], start[2], v0, v0, v0];
        let flow = |_: f64, s: &[f64; 6]| {
            let x = [s[0], s[1], s[2]];
            let fx = f(x);
            let jv = mat_vec(&jac(x), [s[3], s[4], s[5]]);
            [fx[0], fx[1], fx[2], jv[0], jv[1], jv[2]]
        };
        let burn_steps = (burn / h).round() as usize;
        let steps = (t / h).round() as usize;
        let mut sum = 0.0;
        for i in 0..burn_steps + steps {
            ode::rk4_step(&mut y, 0.0, h, &flow);
            let norm = (y[3] * y[3] + y[4] * y[4] + y[5] * y[5]).sqrt();
            if i >= burn_steps {
                sum += norm.ln();
            }
            for v in &mut y[3..] {
                *v /= norm;
            }
        }
        sum / (steps as f64 * h)
    }

    fn rng(seed: u64) -> impl Rng {
        Ctx::new(seed, Size::new(1, 1)).rng
    }

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
    }

    fn trace(m: &M3) -> f64 {
        m[0][0] + m[1][1] + m[2][2]
    }

    fn cyc([x, y, z]: V3) -> V3 {
        [y, z, x]
    }

    #[test]
    fn att_a1_lorenz_lyapunov_matches_viswanath() {
        let starts = [[1.0, 1.0, 1.0], [-3.0, 2.0, 20.0], [5.0, -5.0, 30.0]];
        let lorenz = |p| field(Attractor::Lorenz, p);
        let jac = |p| jacobian(Attractor::Lorenz, p);
        let mean = starts
            .iter()
            .map(|&s| lyapunov_max(lorenz, jac, s, 0.005, 20.0, 400.0))
            .sum::<f64>()
            / 3.0;
        assert!((0.86..=0.95).contains(&mean), "λ₁ = {mean} (ref 0.9056)");
    }

    #[test]
    fn att_a2_gallery_is_chaotic_and_the_periodic_controls_are_not() {
        let runs = [
            (Attractor::Lorenz, 50.0, 400.0),
            (Attractor::Aizawa, 50.0, 600.0),
            (Attractor::Thomas, 200.0, 1500.0),
            (Attractor::Halvorsen, 50.0, 400.0),
            (Attractor::Rossler, 100.0, 1500.0),
            (Attractor::Chen, 50.0, 400.0),
        ];
        for (k, burn, t) in runs {
            let h = if k == Attractor::Lorenz {
                0.005
            } else {
                spec(k).h_max
            };
            let l = lyapunov_max(
                |p| field(k, p),
                |p| jacobian(k, p),
                spec(k).start,
                h,
                burn,
                t,
            );
            assert!(l > 0.015, "{k:?}: λ₁ = {l}");
        }
        let halvorsen = lyapunov_max(
            |p| halvorsen(1.89, p),
            |p| halvorsen_jacobian(1.89, p),
            spec(Attractor::Halvorsen).start,
            0.005,
            50.0,
            400.0,
        );
        assert!(
            halvorsen.abs() < 0.005,
            "Halvorsen a=1.89: λ₁ = {halvorsen}"
        );
        let thomas = lyapunov_max(
            |p| thomas(0.19, p),
            |p| thomas_jacobian(0.19, p),
            spec(Attractor::Thomas).start,
            0.1,
            200.0,
            1500.0,
        );
        assert!(thomas.abs() < 0.005, "Thomas b=0.19: λ₁ = {thomas}");
    }

    #[test]
    fn att_a3_lorenz_vanishes_at_its_nontrivial_equilibria() {
        let q = 72f64.sqrt();
        for p in [[q, q, 27.0], [-q, -q, 27.0]] {
            let f = field(Attractor::Lorenz, p);
            assert!(f.iter().all(|v| v.abs() < 1e-12), "{f:?}");
        }
    }

    #[test]
    fn att_a3_jacobians_match_central_differences_and_divergences() {
        let mut r = rng(3);
        for k in GALLERY {
            let s = spec(k);
            for _ in 0..64 {
                let p: V3 =
                    std::array::from_fn(|i| s.centre[i] + s.half * r.random_range(-1.0..1.0));
                let j = jacobian(k, p);
                for col in 0..3 {
                    let d = 1e-5 * p[col].abs().max(1.0);
                    let (mut lo, mut hi) = (p, p);
                    lo[col] -= d;
                    hi[col] += d;
                    let (fl, fh) = (field(k, lo), field(k, hi));
                    for row in 0..3 {
                        let num = (fh[row] - fl[row]) / (2.0 * d);
                        assert!(
                            close(num, j[row][col], 1e-6),
                            "{k:?} J[{row}][{col}] at {p:?}"
                        );
                    }
                }
                let [x, y, z] = p;
                let div = match k {
                    Attractor::Lorenz => -41.0 / 3.0,
                    Attractor::Aizawa => {
                        let [a, b, _, _, e, f] = AIZAWA;
                        2.0 * (z - b) + a - z * z - e * (x * x + y * y) + f * x * x * x
                    }
                    Attractor::Thomas => -3.0 * THOMAS_B,
                    Attractor::Halvorsen => -3.0 * HALVORSEN_A,
                    Attractor::Rossler => ROSSLER.0 + x - ROSSLER.2,
                    Attractor::Chen => CHEN.2 - CHEN.0 - CHEN.1,
                };
                assert!(close(trace(&j), div, 1e-12), "{k:?} trace at {p:?}");
            }
        }
    }

    #[test]
    fn att_a3_symmetries_are_exact() {
        let mut r = rng(4);
        for _ in 0..64 {
            let p: V3 = std::array::from_fn(|_| r.random_range(-20.0..20.0));
            let f = field(Attractor::Lorenz, p);
            let g = field(Attractor::Lorenz, [-p[0], -p[1], p[2]]);
            assert_eq!(g, [-f[0], -f[1], f[2]], "Lorenz is odd in (x, y)");
            for k in [Attractor::Thomas, Attractor::Halvorsen] {
                assert_eq!(field(k, cyc(p)), cyc(field(k, p)), "{k:?} is cyclic");
            }
        }
    }

    /// A full scout ensemble for attractor `k`.
    fn ensemble(k: Attractor, seed: u64) -> Vec<V3> {
        let mut slot = vec![[0.0; 3]; N];
        let mut scout = Scout::new(k, &mut rng(seed));
        scout.run(&mut slot, usize::MAX);
        assert!(scout.done());
        slot
    }

    #[test]
    fn att_a4_scout_ensembles_fill_their_boxes_and_frames() {
        // In `Attractor` order; Chen's is the median scout box of seeds 0–7 (its rare long
        // excursions make a long orbit's box a poor reference).
        let boxes = [
            [1.44, 1.95, 1.70],
            [2.00, 2.00, 1.49],
            [2.0; 3],
            [2.0; 3],
            [1.81, 1.64, 2.00],
            [1.45, 1.63, 1.13],
        ];
        for k in GALLERY {
            let reference = boxes[k as usize];
            let s = spec(k);
            // Lorenz 0.8, not the 0.9 of the others: a 1024 × 0.05 tu = 51 tu scout rarely
            // reaches the outer spiral (z is the short axis; minimum 0.825 over seeds 0–7,
            // 0.806 over 0–63, where 35 of 64 seeds are below 0.9). Aizawa keeps 0.9, which
            // its coexisting inner attractor (z: 0.82) fails.
            let fill = if k == Attractor::Lorenz { 0.8 } else { 0.9 };
            // A heavy tail: Chen's outer spiral is reached by some 1024-sample scouts and
            // not others, so its radius varies more by seed (0.86–1.03 ρ), and its rarest
            // points reach 1.38× the 99.5 % frame (clipped at the edge while they are out).
            let (radius_floor, tail) = match k {
                Attractor::Chen => (0.85, 1.4),
                _ => (0.9, 1.2),
            };
            let mut reach = [0.0f64; 3];
            for seed in 0..8 {
                let pts = ensemble(k, seed);
                let (mut lo, mut hi, mut rmax) = ([f64::MAX; 3], [f64::MIN; 3], 0.0f64);
                for &p in &pts {
                    let q = normalised(k, p);
                    assert!(
                        q.iter().all(|v| v.abs() <= 1.05),
                        "{k:?} seed {seed}: {q:?}"
                    );
                    for i in 0..3 {
                        lo[i] = lo[i].min(q[i]);
                        hi[i] = hi[i].max(q[i]);
                    }
                    rmax = rmax.max(q.iter().map(|v| v * v).sum::<f64>().sqrt());
                }
                for i in 0..3 {
                    assert!(
                        hi[i] - lo[i] >= fill * reference[i],
                        "{k:?} seed {seed} axis {i}: {}",
                        hi[i] - lo[i]
                    );
                }
                assert!(
                    (radius_floor * s.rho..=1.03 * s.rho).contains(&rmax),
                    "{k:?} seed {seed}: max radius {rmax}"
                );
                // The frame is the 99.5th percentile of every pose over the camera loop,
                // measured about its yaw-dependent midpoint, and tight (some pose of some
                // seed reaches 0.97 of it on every side); nothing lies beyond 1.2× it, so
                // clipped excursions stay short.
                let f = s.frame;
                let q = N - N / 200;
                for step in 0..72 {
                    for dp in [-0.10, -0.05, 0.0, 0.05, 0.10] {
                        let yaw = f64::from(step) * 5f64.to_radians();
                        let cam = Camera {
                            yaw,
                            pitch: s.pitch + dp,
                            distance: DISTANCE,
                        };
                        let pr = cam.projector();
                        let swing = f.swing * (3.0 * yaw).cos();
                        let (mut xs, mut ys) = (Vec::with_capacity(N), Vec::with_capacity(N));
                        for &p in &pts {
                            let (x, y, _) = pr.project(world(k, p), (0.0, 0.0), 1.0).unwrap();
                            xs.push((x / ASPECT).abs());
                            ys.push(y - swing);
                        }
                        xs.sort_by(f64::total_cmp);
                        ys.sort_by(f64::total_cmp);
                        let (x, top, bottom) = (xs[q], ys[N - 1 - q], ys[q]);
                        assert!(
                            x <= f.x && top >= f.top && bottom <= f.bottom,
                            "{k:?} seed {seed}: 99.5 % box ({x}, {top}, {bottom})"
                        );
                        for (r, v) in
                            reach
                                .iter_mut()
                                .zip([x / f.x, top / f.top, bottom / f.bottom])
                        {
                            *r = r.max(v);
                        }
                        let (x, top, bottom) = (xs[N - 1], ys[0], ys[N - 1]);
                        assert!(
                            x <= tail * f.x && top >= tail * f.top && bottom <= tail * f.bottom,
                            "{k:?} seed {seed}: extremes ({x}, {top}, {bottom})"
                        );
                    }
                }
            }
            assert!(
                reach.iter().all(|&r| r >= 0.97),
                "{k:?}: frame is loose {reach:?}"
            );
        }
    }

    #[test]
    fn att_a4_rates_give_1_2_half_widths_per_second() {
        // The model rates r_k are set so the mean normalised speed |f|·r_k/s_k is 1.2 box
        // half-widths per simulated second for every attractor, peaking at ≈ 9/s in
        // Rössler's fold (measured 1.20–1.22; peak 9.06).
        for k in GALLERY {
            let s = spec(k);
            let mut x = s.start;
            let burn = (BURN_S * s.rate / s.h_max).round() as usize;
            let span = if k == Attractor::Thomas {
                1000.0
            } else {
                100.0
            };
            let steps = (span / s.h_max).round() as usize;
            let (mut sum, mut peak) = (0.0, 0.0f64);
            for i in 0..burn + steps {
                ode::rk4_step(&mut x, 0.0, s.h_max, |_, p| field(k, *p));
                if i >= burn {
                    let v = field(k, x);
                    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() * s.rate / s.half;
                    sum += speed;
                    peak = peak.max(speed);
                }
            }
            let mean = sum / steps as f64;
            assert!(
                (1.1..=1.3).contains(&mean),
                "{k:?}: mean {mean} half-widths/s"
            );
            if k == Attractor::Rossler {
                assert!((8.0..=10.0).contains(&peak), "Rössler peak {peak}");
            }
        }
    }

    #[test]
    fn att_a4_aizawa_fill_threshold_rejects_its_inner_attractor() {
        // Negative control for the 0.9 fill bound: from (0.2, 0.1, 0.5) Aizawa settles on
        // its coexisting non-chaotic attractor, whose z extent is only 0.82 of the box
        // (measured 1.226 of 1.49), while the in-basin scout ensembles reach ≥ 0.9 of it.
        let k = Attractor::Aizawa;
        let (h, z_box) = (0.02, 1.49);
        let mut x = [0.2, 0.1, 0.5];
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 0..(600.0 / h) as usize {
            ode::rk4_step(&mut x, 0.0, h, |_, p| field(k, *p));
            if f64::from(i as u32) * h >= 100.0 {
                let z = normalised(k, x)[2];
                (lo, hi) = (lo.min(z), hi.max(z));
            }
        }
        assert!(
            hi - lo < 0.9 * z_box,
            "inner attractor z extent {}",
            hi - lo
        );
        assert!(
            hi - lo > 0.7 * z_box,
            "inner attractor z extent {}",
            hi - lo
        );
        let pts = ensemble(k, 0);
        let z = pts.iter().map(|&p| normalised(k, p)[2]);
        let (lo, hi) = z.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        assert!(hi - lo >= 0.9 * z_box, "chaotic z extent {}", hi - lo);
    }

    #[test]
    fn att_a5_a_scout_that_always_escapes_still_terminates() {
        // A start that escapes every time (here: a field that is never finite) must not
        // spin forever in `run(.., usize::MAX)`: after MAX_RESTARTS restarts from offset
        // starts the scout fills its slot and stops.
        let mut slot = vec![[0.0; 3]; N];
        let mut scout = Scout::new(Attractor::Lorenz, &mut rng(9));
        scout.run_field(&mut slot, usize::MAX, |_| [f64::NAN; 3]);
        assert!(scout.done());
        assert!(slot.iter().flatten().all(|v| v.is_finite()));

        // Escapes that stop once the start is offset: samples resume after a fresh burn-in.
        let k = Attractor::Lorenz;
        let mut scout = Scout::new(k, &mut rng(9));
        let mut calls = 0usize;
        scout.run_field(&mut slot, usize::MAX, |p| {
            calls += 1;
            if calls <= 4 {
                [f64::NAN; 3]
            } else {
                field(k, p)
            }
        });
        assert!(scout.done());
        assert!(slot.iter().all(|&p| !escaped(k, p)));
    }

    #[test]
    fn att_a8_zero_area_sizes_keep_the_view_finite() {
        // A zero-width (or zero-height) terminal fits no rows: the scale is clamped rather
        // than log-lerping ln 0 = −∞ into NaN, and nothing is plotted.
        for (w, h) in [(0, 24), (80, 0), (0, 0)] {
            let (mut s, mut ctx, _) = scene(w, h, GlyphMode::Unicode, 0);
            for _ in 0..8 {
                s.update(0.25, &mut ctx);
                let g = s.gallery.as_ref().unwrap();
                let (_, centre, scale) = s.view(g);
                assert!(
                    scale.is_finite()
                        && scale > 0.0
                        && centre.0.is_finite()
                        && centre.1.is_finite(),
                    "{w}x{h}: scale {scale}, centre {centre:?}"
                );
            }
        }
    }

    #[test]
    fn att_a5_gallery_stays_bounded_and_replaces_a_nan_particle() {
        let mut r = rng(5);
        let mut g = Gallery::new(0, 64, &mut r);
        for _ in 0..2 * 240 {
            g.step(0.25, &mut r);
            for i in 0..64 {
                let p = g.displayed(i);
                let norm = p.iter().map(|v| v * v).sum::<f64>().sqrt();
                assert!(norm.is_finite() && norm <= 1.5, "particle {i}: {p:?}");
            }
        }
        let mut g = Gallery::new(0, 64, &mut r);
        g.slots[g.cur][3] = [f64::NAN; 3];
        g.step(SIM_DT, &mut r);
        let (a, b) = (g.displayed(3), g.displayed(3 + 32));
        let d = (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0, f64::max);
        assert!(d <= 1e-5, "clone is {d} from its source");

        // A particle whose clone source has escaped too clones the next finite particle,
        // never the fixed start (Lorenz's sits by the origin saddle, off the attractor).
        let mut g = Gallery::new(0, 64, &mut r);
        g.slots[g.cur][3] = [f64::NAN; 3];
        g.slots[g.cur][3 + 32] = [f64::NAN; 3];
        g.step(SIM_DT, &mut r);
        let neighbour = g.displayed(4);
        for i in [3, 3 + 32] {
            let p = g.displayed(i);
            let d = (0..3)
                .map(|j| (p[j] - neighbour[j]).abs())
                .fold(0.0, f64::max);
            assert!(d <= 1e-5, "particle {i} is {d} from its finite neighbour");
        }

        // A scout that escapes restarts its burn-in: no transient samples are kept.
        let k = Attractor::Lorenz;
        let (sp, mut slot) = (spec(k), vec![[0.0; 3]; N]);
        let burn = (BURN_S * sp.rate / sp.h_max).round() as usize;
        let stride = (STRIDE_S * sp.rate / sp.h_max).round() as usize;
        let mut scout = Scout::new(k, &mut r);
        scout.run(&mut slot, burn + 10 * stride);
        assert_eq!(scout.filled, 10);
        scout.x = [f64::NAN; 3];
        scout.run(&mut slot, burn);
        assert_eq!(scout.filled, 10, "sampled before a fresh burn-in");
        scout.run(&mut slot, 2 * stride);
        assert!(scout.filled > 10);
    }

    #[test]
    fn att_a6_timeline_order_endpoints_and_continuity() {
        let a = [0.1, -0.2, 0.3];
        let b = [-0.7, 0.4, 0.9];
        assert_eq!(blend(a, b, 0.0), a);
        assert_eq!(blend(a, b, 1.0), b);

        let mut r = rng(6);
        // At every morph and display prefix: the Gallery itself shows slot A at s = 0 and
        // slot B at s = 1 after pairing, and the pairing matches the two ensembles rank for
        // rank in (azimuth sector, height band, radius) order, recomputed here with stable
        // sorts, so the mid-morph swarm is a blend, not a scramble.
        let order = |pts: &[V3]| {
            let key = |p: &V3| [p[2].atan2(p[0]), p[1], p[0].hypot(p[2])];
            let mut idx: Vec<usize> = (0..pts.len()).collect();
            let by = |axis: usize| {
                move |&i: &usize, &j: &usize| key(&pts[i])[axis].total_cmp(&key(&pts[j])[axis])
            };
            idx.sort_by(by(0));
            for sector in idx.chunks_mut(pts.len() / SECTORS) {
                sector.sort_by(by(1));
                for band in sector.chunks_mut(pts.len() / SECTORS / BANDS) {
                    band.sort_by(by(2));
                }
            }
            idx
        };
        for first in 0..GALLERY.len() {
            for n_eff in [64, 256] {
                let mut g = Gallery::new(first, n_eff, &mut r);
                g.begin_morph();
                let (from, to, _) = g.pair();
                let n = g.n();
                assert_eq!(n, n_eff);
                let (a, b) = (g.cur, 1 - g.cur);
                let pa: Vec<V3> = (0..n).map(|i| world(from, g.slots[a][i])).collect();
                let pb: Vec<V3> = (0..n).map(|i| world(to, g.slots[b][i])).collect();
                assert_eq!(order(&pa), order(&pb), "{from:?} -> {to:?}, n = {n}");

                for (w, k, slot) in [(0.0, from, a), (1.0, to, b)] {
                    g.weight = w;
                    for i in 0..n {
                        let want = world(k, g.slots[slot][i]);
                        assert_eq!(g.displayed(i), want, "{from:?} w = {w}, i = {i}");
                    }
                }
            }
        }

        let mut g = Gallery::new(0, 64, &mut r);
        assert_eq!(g.timeline.total(), 72.0);
        let mut seen = vec![(g.at, g.morphing, 0.0)];
        let mut last: Vec<V3> = (0..64).map(|i| g.displayed(i)).collect();
        let mut worst = 0.0f64;
        // Mid-morph the swarm keeps its body: along every world axis its spread (the 16-power
        // mean deviation, nearly the extent) is the lerp of the two ensembles', unless that
        // needs more than √2× the plain blend's (which narrows to ≈ 0.75 at w = ½), and it
        // is never narrower than the plain blend. Recomputed here in `powf`.
        let spread = |pts: &[V3]| -> V3 {
            std::array::from_fn(|j| {
                let mean = pts.iter().map(|p| p[j]).sum::<f64>() / 64.0;
                let sum: f64 = pts.iter().map(|p| (p[j] - mean).abs().powf(16.0)).sum();
                (sum / 64.0).powf(1.0 / 16.0)
            })
        };
        let (mut body, mut stretched) = (0.0f64, 0);
        let steps = 4321;
        for s in 1..=steps {
            g.step(SIM_DT, &mut r);
            if (g.at, g.morphing) != (seen.last().unwrap().0, seen.last().unwrap().1) {
                seen.push((g.at, g.morphing, f64::from(s) * SIM_DT));
            }
            if g.morphing && g.weight > 0.0 && g.weight < 1.0 {
                let (from, to, w) = g.pair();
                let a: Vec<V3> = g.slots[g.cur][..64]
                    .iter()
                    .map(|&p| world(from, p))
                    .collect();
                let b: Vec<V3> = g.slots[1 - g.cur][..64]
                    .iter()
                    .map(|&p| world(to, p))
                    .collect();
                let plain: Vec<V3> = a.iter().zip(&b).map(|(&p, &q)| blend(p, q, w)).collect();
                let shown: Vec<V3> = (0..64).map(|i| g.displayed(i)).collect();
                let (s_a, s_b, s_p, s) = (spread(&a), spread(&b), spread(&plain), spread(&shown));
                for j in 0..3 {
                    let want = ease::lerp(s_a[j], s_b[j], w).clamp(s_p[j], SQRT_2 * s_p[j]);
                    body = body.max((s[j] / want - 1.0).abs());
                    stretched += usize::from(s[j] > 1.2 * s_p[j]);
                }
            }
            for (i, prev) in last.iter_mut().enumerate() {
                let p = g.displayed(i);
                let d = (0..3).map(|j| (p[j] - prev[j]).powi(2)).sum::<f64>().sqrt();
                worst = worst.max(d);
                *prev = p;
            }
        }
        assert!(worst <= 0.2, "a displayed point jumped {worst}");
        assert!(body <= 1e-9, "mid-morph spread is off by {body:e}");
        assert!(
            stretched > 100,
            "the stretch did not act ({stretched} axis-steps)"
        );
        let order: Vec<_> = seen.iter().map(|&(at, m, _)| (at, m)).collect();
        let expected: Vec<_> = (0..GALLERY.len())
            .flat_map(|k| [(k, false), (k, true)])
            .chain([(0, false)])
            .collect();
        assert_eq!(order, expected);
        for (i, &(_, _, t)) in seen.iter().enumerate().skip(1) {
            let want = 12.0 * ((i / 2) as f64) + if i % 2 == 1 { HOLD } else { 0.0 };
            assert!(
                (t - want).abs() <= SIM_DT + 1e-9,
                "transition {i} at {t}, want {want}"
            );
        }
    }

    fn scene(w: u16, h: u16, mode: GlyphMode, first: usize) -> (StrangeAttractors, Ctx, Canvas) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(42, size);
        let mut s = StrangeAttractors::default();
        s.resize(size, &mut ctx);
        s.gallery = Some(Gallery::new(first, n_eff(size), &mut ctx.rng));
        s.resize(size, &mut ctx);
        (s, ctx, Canvas::new(size, mode))
    }

    fn label_row(s: &StrangeAttractors, c: &mut Canvas) -> String {
        c.clear();
        s.render(c);
        c.to_lines().pop().unwrap()
    }

    #[test]
    fn gen_r12_caption_names_the_shown_attractor() {
        for (i, k) in GALLERY.into_iter().enumerate() {
            let (s, _, _) = scene(80, 24, GlyphMode::Unicode, i);
            assert_eq!(s.caption(), Some(CAPTIONS[k as usize]), "{k:?}");
        }
        assert_eq!(
            StrangeAttractors::default().caption(),
            None,
            "before resize"
        );
    }

    #[test]
    fn att_a7_gen_r13_parameters_sit_right_and_the_name_is_the_caption() {
        for k in GALLERY {
            let s = spec(k);
            assert_eq!(
                s.params.chars().count(),
                s.params_ascii.chars().count(),
                "{k:?}"
            );
            assert!(s.params_ascii.is_ascii());
        }
        // The name is the title's caption, so the row holds the parameters alone,
        // right-aligned with a blank either side (Aizawa: nothing).
        for (i, k) in GALLERY.into_iter().enumerate() {
            for (mode, params) in [
                (GlyphMode::Ascii, spec(k).params_ascii),
                (GlyphMode::Unicode, spec(k).params),
            ] {
                let (s, _, mut c) = scene(80, 24, mode, i);
                let row = label_row(&s, &mut c);
                assert_eq!(row.trim(), params, "{k:?} {mode:?}");
                assert!(row.ends_with(' '), "{row:?}");
                assert!(row.is_ascii() || mode == GlyphMode::Unicode);
            }
        }
        // Every set is restrained (≤ 16 chars) and quoted whole or, when it does not fit
        // (Aizawa's six), not at all: one parameter out of several would read as the
        // attractor's only one.
        assert!(
            GALLERY
                .iter()
                .all(|&k| spec(k).params.chars().count() <= 16)
        );
        for (k, params) in GALLERY.into_iter().zip([3, 6, 1, 3, 1, 3]) {
            let quoted = spec(k).params.matches('=').count();
            assert!(
                quoted == params || quoted == 0,
                "{k:?} quotes {quoted} of {params}"
            );
        }
        // Lorenz's 15-char set beside "Strange attractor" alone (from 32 wide, the full
        // title from 47), where the room is w − 23: dropped at 37 columns, shown at 38.
        for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
            let (s, _, mut c) = scene(37, 12, mode, 0);
            assert_eq!(label_row(&s, &mut c).trim(), "", "{mode:?}");
            let (s, _, mut c) = scene(38, 12, mode, 0);
            assert!(label_row(&s, &mut c).contains("8/3"), "{mode:?}");
        }
        for (w, h) in [(11, 24), (80, 7)] {
            let (s, _, mut c) = scene(w, h, GlyphMode::Ascii, 0);
            assert!(!label_row(&s, &mut c).contains("8/3"), "{w}x{h}");
        }
    }

    #[test]
    fn att_a8_lorenz_fills_the_frame_and_plots_stay_finite() {
        let (mut s, mut ctx, _) = scene(80, 24, GlyphMode::Unicode, 0);
        for _ in 0..60 {
            s.update(SIM_DT, &mut ctx);
        }
        let lit = |x: i32, y: i32| (0..8).any(|d| s.dots.is_lit(2 * x + d % 2, 4 * y + d / 2));
        let rows = (0..24).filter(|&y| (0..80).any(|x| lit(x, y))).count();
        let cols = (0..80).filter(|&x| (0..24).any(|y| lit(x, y))).count();
        assert!(rows >= 14 && cols >= 24, "{rows} rows × {cols} cols");
        assert!((0..80).all(|x| !lit(x, 23)), "row h−1 belongs to the label");

        // Row h−1 stays free for the whole loop (80×24: in the composition test below).
        let (mut s, mut ctx, _) = scene(40, 12, GlyphMode::Unicode, 0);
        for step in 0..4320 {
            s.update(SIM_DT, &mut ctx);
            let x = (0..40).find(|&x| cell_lit(&s, x, 11));
            assert!(
                x.is_none(),
                "40x12 step {step}: dot on the label row at {x:?}"
            );
        }

        for (w, h) in [(1, 1), (3, 2), (80, 24), (200, 50)] {
            let (mut s, mut ctx, _) = scene(w, h, GlyphMode::Unicode, 0);
            for _ in 0..240 {
                s.update(0.25, &mut ctx);
                let g = s.gallery.as_ref().unwrap();
                let (pr, centre, scale) = s.view(g);
                for i in 0..g.n() {
                    let (x, y, _) = pr
                        .project(g.displayed(i), centre, scale)
                        .expect("in front of the eye");
                    assert!(x.is_finite() && y.is_finite());
                }
            }
        }
    }

    #[test]
    fn att_a8_without_a_label_the_figure_uses_every_row() {
        // The label row is reserved only while the label shows (ATT-A7): below 8 rows the
        // figure is framed in, and centred on, all of them.
        assert_eq!(stage_rows(Size::new(80, 24)), 23);
        for (w, h) in [(80, 7), (11, 24)] {
            assert_eq!(stage_rows(Size::new(w, h)), h);
        }
        let (mut s, mut ctx, _) = scene(80, 7, GlyphMode::Unicode, 0);
        let mut bottom = false;
        for _ in 0..240 {
            s.update(SIM_DT, &mut ctx);
            bottom |= (0..80).any(|x| cell_lit(&s, x, 6));
        }
        assert!(bottom, "80x7: row 6 never drawn");
    }

    fn cell_lit(s: &StrangeAttractors, x: i32, y: i32) -> bool {
        (0..8).any(|d| s.dots.is_lit(2 * x + d % 2, 4 * y + d / 2))
    }

    /// Lit cells and the number of rows between the first and last lit row.
    fn coverage(s: &StrangeAttractors) -> (usize, usize) {
        let (w, h) = (i32::from(s.size.width()), i32::from(s.size.height()));
        let rows: Vec<i32> = (0..h)
            .filter(|&y| (0..w).any(|x| cell_lit(s, x, y)))
            .collect();
        let lit = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| cell_lit(s, x, y))
            .count();
        let span = rows.last().map_or(0, |&b| (b - rows[0] + 1) as usize);
        (lit, span)
    }

    #[test]
    fn att_a8_resize_primes_a_unit_deposits_whole_lifetime() {
        // Segment j updates old carries e^{−j·dt/τ} of a unit deposit: lit up to j = 15, so
        // the replay spans exactly the segments 0..PRIME_STEPS that can still be lit.
        let life = TAU_S * (1.0 / f64::from(D_MIN)).ln() / SIM_DT;
        assert_eq!(
            PRIME_STEPS,
            life.floor() as usize + 1,
            "lifetime {life} updates"
        );
    }

    #[test]
    fn att_a8_holds_fill_the_frame_morphs_keep_their_body_and_frame_0_has_threads() {
        // One loop of the real scene at 80×24 (seed 42 opens on Aizawa). Per hold: mean lit
        // cells and mean silhouette height; per morph: lit cells at s = 0.5.
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(42, size);
        let mut s = StrangeAttractors::default();
        s.resize(size, &mut ctx);
        let first = coverage(&s).0;
        let mut holds: Vec<(Attractor, f64, f64)> = Vec::new();
        let mut mids: Vec<(Attractor, Attractor, f64, f64)> = Vec::new();
        let (mut lit, mut span, mut frames) = (0.0, 0.0, 0.0);
        let mut uneven = 0i32;
        for step in 0..4320 {
            s.update(SIM_DT, &mut ctx);
            let x = (0..80).find(|&x| cell_lit(&s, x, 23));
            assert!(x.is_none(), "step {step}: dot on the label row at {x:?}");
            let g = s.gallery.as_ref().unwrap();
            let (from, to, w) = g.pair();
            let (l, r) = coverage(&s);
            if !g.morphing {
                (lit, span, frames) = (lit + l as f64, span + r as f64, frames + 1.0);
                if from == Attractor::Halvorsen {
                    // Blank rows above the figure minus those below it (rows 0..=22).
                    let rows: Vec<i32> = (0..23)
                        .filter(|&y| (0..80).any(|x| cell_lit(&s, x, y)))
                        .collect();
                    let margins = rows[0] - (22 - rows[rows.len() - 1]);
                    uneven = uneven.max(margins.abs());
                }
            } else if frames > 0.0 {
                holds.push((from, lit / frames, span / frames));
                (lit, span, frames) = (0.0, 0.0, 0.0);
            }
            if g.morphing && w >= 0.5 && mids.last().is_none_or(|m| m.0 != from) {
                mids.push((from, to, l as f64, r as f64));
            }
        }
        let n = GALLERY.len();
        assert_eq!((holds.len(), mids.len()), (n, n));
        let lit_of = |k: Attractor| holds.iter().find(|h| h.0 == k).unwrap().1;
        let span_of = |k: Attractor| holds.iter().find(|h| h.0 == k).unwrap().2;

        // Halvorsen stays centred through its hold: its blade tips swing the silhouette's
        // box by ±0.17 half-heights over 120° of yaw, and the frame follows (Frame::swing;
        // a fixed centre gave 4 rows here and 9 of 49 at 200×50, where it is now ≤ 5).
        assert!(uneven <= 2, "Halvorsen's margins differ by {uneven} rows");

        // The bounds below hold for seeds 0–23 as well as 42 (chaos makes the per-seed
        // numbers platform-sensitive, so no bound leans on seed 42's own values), except the
        // mid-morph span, which Halvorsen → Rössler misses for about half of those seeds
        // (0.72–0.85; see below), as the Halvorsen margins above do for seeds 6, 22 and 23
        // (the seed figures below are for the other 21).
        //
        // Frame 0 already shows threads: 0.96 of the first hold's mean (seeds 0–23: ≥ 0.87).
        let opening = lit_of(holds[0].0);
        assert!(
            first as f64 >= 0.85 * opening,
            "frame 0 lights {first} cells, the hold {opening:.0}"
        );
        // Holds fill the frame: silhouettes span ≥ 0.72 of the rows above the label, 0.8 on
        // average (seeds 0–23: ≥ 0.75, mean ≥ 0.84).
        for &(k, _, rows) in &holds {
            assert!(rows >= 0.72 * 23.0, "{k:?} spans {rows:.1} rows");
        }
        let mean = holds.iter().map(|h| h.2).sum::<f64>() / n as f64;
        assert!(mean >= 0.8 * 23.0, "holds span {mean:.1} rows on average");
        // Aizawa's shell covers its disc where the others are sheets and bands, so it lights
        // more cells, but never twice as many (1.33× their mean; seeds 0–23: 1.40–1.75×).
        let others = GALLERY[..].iter().filter(|&&k| k != Attractor::Aizawa);
        let others = others.map(|&k| lit_of(k)).sum::<f64>() / (n - 1) as f64;
        let aizawa = lit_of(Attractor::Aizawa);
        assert!(aizawa <= 2.0 * others, "Aizawa {aizawa:.0} vs {others:.0}");
        // Mid-morph the swarm keeps its body: ≥ 0.55 of the smaller neighbour's hold. The
        // lowest is Lorenz → Aizawa (0.66; seeds 0–23: ≥ 0.59): both ends are dense holds,
        // and Lorenz's count swings with the yaw it is held at.
        // Nor does it shrink into a knot: its lit rows span ≥ 0.85 of the smaller neighbouring
        // hold's mean span (a plain lerp of the decorrelated pairs gave 0.65–0.83 here).
        // Seeds 0–7 at 80×24 and 200×50: ≥ 0.88, except Halvorsen → Rössler at some yaws
        // (0.75), where the camera swings from above the propeller to beside the band and
        // the half-way pitch foreshortens both.
        for (from, to, l, rows) in mids {
            let lo = lit_of(from).min(lit_of(to));
            assert!(l >= 0.55 * lo, "{from:?} -> {to:?}: {l} lit vs {lo:.0}");
            let lo = span_of(from).min(span_of(to));
            assert!(
                rows >= 0.85 * lo,
                "{from:?} -> {to:?}: spans {rows} rows vs {lo:.1}"
            );
        }
    }

    #[test]
    fn att_a9_threads_take_the_shown_attractors_palette() {
        // Every member has its own palette, so each reads as itself (false colour).
        for (i, a) in GALLERY.iter().enumerate() {
            for b in &GALLERY[i + 1..] {
                assert_ne!(spec(*a).palette, spec(*b).palette, "{a:?} {b:?}");
            }
        }
        for (i, k) in GALLERY.into_iter().enumerate() {
            let (mut s, mut ctx, mut c) = scene(80, 24, GlyphMode::Unicode, i);
            for _ in 0..30 {
                s.update(SIM_DT, &mut ctx);
            }
            s.render(&mut c);
            let roles: Vec<Role> = c.cells()[..80 * 23]
                .iter()
                .filter(|cell| cell.ch != ' ')
                .map(|cell| cell.ink.role)
                .collect();
            assert!(roles.len() > 50, "{k:?}: {} cells", roles.len());
            assert!(
                roles.iter().all(|&r| r == Role::Ramp(spec(k).palette)),
                "{k:?}"
            );
        }
        // Sparse strands sit in the hue, not the ramp's black foot; the bulk stays
        // saturated; only the densest cores approach the near-white top.
        let ln_top = (1.0 + D_TOP / f64::from(D_MIN)).ln();
        let at = |d: f64| thread_ink(Ramp::HAlpha, d as f32, ln_top);
        assert_eq!(at(f64::from(D_MIN)).role, Role::Ramp(Ramp::HAlpha));
        assert!(at(f64::from(D_MIN)).level >= ink::seq(0.3).level);
        assert!(at(D_TOP / 8.0).level <= ink::seq(0.75).level);
        assert_eq!(at(D_TOP).level, ink::seq(0.95).level);
        assert_eq!(at(1e9), at(D_TOP));
        // The deep crimson, violet and wine ramps start higher, so their sparse strands
        // are no darker on screen than the others'.
        for ramp in Ramp::ALL {
            let floor = thread_ink(ramp, 0.0, ln_top).level;
            let deep = matches!(ramp, Ramp::HAlpha | Ramp::Plasma | Ramp::Ferroin);
            let want = if deep {
                PALETTE_FLOOR_DEEP
            } else {
                PALETTE_FLOOR
            };
            assert_eq!(floor, ink::seq(want).level, "{ramp:?}");
        }
    }

    /// Horizontal extent of `k`'s attractor seen from camera yaw `yaw` (world units), from
    /// a 60 000-step RK4 orbit after a transient.
    fn width_at(path: &[V3], yaw: f64) -> f64 {
        let proj = Camera {
            yaw,
            pitch: 0.0,
            distance: f64::INFINITY,
        }
        .projector();
        let xs = path.iter().map(|&p| proj.rotate(p)[0]);
        xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min)
    }

    #[test]
    fn att_a10_two_wing_flows_open_their_wings_through_the_hold() {
        // Seen side-on a two-wing flow is a narrow V (a third of its widest); seen face-on
        // its wings open side by side and the figure is at its widest. Every yaw of
        // Lorenz's and Chen's hold, whichever member the seed starts at, shows them at
        // least 70 % open.
        for k in [Attractor::Lorenz, Attractor::Chen] {
            let mut y = spec(k).start;
            let h = spec(k).h_max;
            let mut path = Vec::new();
            for n in 0..62_000 {
                ode::rk4_step(&mut y, 0.0, h, |_, p| field(k, *p));
                if n >= 2_000 && n % 4 == 0 {
                    path.push(world(k, y));
                }
            }
            let widest = (0..180)
                .map(|d| width_at(&path, f64::from(d).to_radians()))
                .fold(0.0, f64::max);
            // The camera clock runs with the gallery's, where member `i`'s hold starts at
            // 12·i s (mod 72 s = two yaw turns).
            let i = GALLERY.iter().position(|&g| g == k).unwrap();
            let start = (HOLD + MORPH) * i as f64;
            for s in 0..=8 {
                let yaw = TAU * (start + HOLD * f64::from(s) / 8.0) / YAW_PERIOD + spec(k).yaw;
                let open = width_at(&path, yaw) / widest;
                assert!(open >= 0.7, "{k:?} {s}/8 of the hold: {open:.2} open");
            }
        }
    }

    #[test]
    fn att_a10_opening_the_wings_never_whips_or_reverses_the_camera() {
        // The wing flows' offset must not turn the steady 10 °/s turntable into a whip pan
        // at the morphs next to them: over a whole loop the yaw rate stays within
        // [−1, 25] °/s (the base rate is 10 °/s).
        let base = 360.0 / YAW_PERIOD;
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for (i, &from) in GALLERY.iter().enumerate() {
            let to = GALLERY[(i + 1) % GALLERY.len()];
            let start = (HOLD + MORPH) * i as f64;
            let yaw = |t: f64| {
                let w = ease::smoothstep(((t - start - HOLD) / MORPH).clamp(0.0, 1.0));
                camera_yaw(t, from, to, w)
            };
            let mut t = start;
            while t < start + HOLD + MORPH {
                let rate = (yaw(t + SIM_DT) - yaw(t)).to_degrees() / SIM_DT;
                (lo, hi) = (lo.min(rate), hi.max(rate));
                t += SIM_DT;
            }
        }
        assert!(
            lo >= -1.0 && hi <= 2.5 * base,
            "yaw rate {lo:.1}..{hi:.1} °/s"
        );
    }
}
