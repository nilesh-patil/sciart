//! Belousov–Zhabotinsky dish: spiral and target oxidation waves sweeping a dark medium
//! (SPEC-002 `belousov_zhabotinsky`).
//!
//! Barkley's (1991) excitable medium on a no-flux rectangle:
//!
//! ```text
//! ∂u/∂t = ∇²u + ε⁻¹ u (1 − u)(u − u_th),   u_th = (v + b)/a
//! ∂v/∂t = u − v
//! ```
//!
//! `u` stands for the autocatalyst HBrO₂ and `v` for the oxidised catalyst (ferriin),
//! whose reduction releases the Br⁻ inhibitor: the two-variable Oregonator picture.
//! With `a = 0.75`, `b = 0.06`, `ε = 0.02` (the EZ-Spiral default) broken fronts curl
//! into rigidly rotating spirals.
//!
//! ## Discretisation
//!
//! Node spacing `h = 0.5` su, sub-step `τ = 0.0125` tu, Lie-split into three stages:
//! explicit diffusion with the isotropic 9-point Laplacian (`grid::laplacian9`, Neumann
//! walls), the reaction with Barkley's (1991) implicit update on the decaying branch
//! (`u < u_th`) and an explicit one on the growing branch, which stays bounded because
//! `τ/ε ≤ 1` (Barkley and EZ-Spiral take that branch implicitly too), then
//! `v ← v + τ(u − v)`. Since `(10/3)·τ/h² ≈ 0.17 ≤ 1`,
//! `τ/ε = 0.625 ≤ 1` and `τ ≤ 1`, every stage is a convex combination or a shrinking
//! factor, so `u, v ∈ [0, 1]` holds exactly for any `b ≥ 0` without clamps (BZ-A1).
//!
//! ## Clock
//!
//! The simulation runs on an integer sub-step counter at 120 sub-steps per simulated
//! second (1.5 tu/s): the quench, the pacemakers and the reseeding are scheduled on that
//! counter, so how frames chop time never changes what happens.

use std::f64::consts::TAU as TWO_PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::title;
use sciart_core::{draw, ease};

use crate::shared::excitable::{self, Barkley, Front, FrontStroke, unit};

pub static META: Meta = Meta {
    name: "belousov_zhabotinsky",
    category: Category::Chemistry,
    description: "Belousov-Zhabotinsky dish: spiral and target oxidation waves sweeping a dark medium",
    title: "Chemical waves · fire, then rest",
    captions: &CAPTIONS,
};

/// The quench's caption; the waves show the title's own mechanism (CORE-R71).
const CAPTIONS: [&str; 1] = ["light quench"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Bz::default())
}

// ---- Model (SPEC-002 Model) ----

const A: f32 = 0.75;
const B0: f32 = 0.06;
/// `b` at the height of the light quench: `u_th ≥ (0 + 0.45)/0.75 = 0.6 > ½`, so no
/// front can sustain itself.
const B_QUENCH: f32 = 0.45;
const EPS: f32 = 0.02;
/// Node spacing in space units.
const H: f32 = 0.5;
/// Sub-step in time units.
const TAU: f32 = 0.0125;
/// Sub-steps per simulated second: `RATE = 1.5` tu/s over `τ`.
const SUB_PER_S: u64 = 120;
/// Nagumo front speed at `v = 0`, `√(2/ε)(½ − b/a)` su/tu, the BZ-A3 reference.
#[cfg(test)]
const C_NAGUMO: f32 = 4.2;

// Every stage must be a convex combination / shrinking factor (module docs, BZ-A1).
const _: () = assert!(MODEL.is_bounded());

// ---- Story (SPEC-002 Model, Motion) ----

const WAVES_S: u64 = 60;
/// The dish is dark ≈ 1 s into the quench; one more second of dark reads as a pause,
/// not a dead scene.
const QUENCH_S: u64 = 2;
const CYCLE_STEPS: u64 = (WAVES_S + QUENCH_S) * SUB_PER_S;
const WAVES_STEPS: u64 = WAVES_S * SUB_PER_S;
/// Seconds over which `b` ramps up at the start of the quench.
const QUENCH_RAMP_S: f64 = 0.5;
/// Beat index of the quench in the timeline.
const QUENCH: usize = 1;

/// Pacemakers first fire 1 tu into a cycle, then every `T_pm = 6` tu (≈ 1.5 spiral
/// periods, so the spirals win the competition, Zaikin & Zhabotinsky 1970).
const PM_FIRST: u64 = 80;
const PM_PERIOD: u64 = 480;
const PM_RADIUS: f64 = 2.5;
/// A pacemaker only re-excites nodes that have recovered this far.
const PM_V_MAX: f32 = 0.1;

// ---- Seeding (SPEC-002 Model) ----

/// Spiral wavelength in nodes, the unit of every seeding length.
const LAMBDA: f64 = 25.0;
const NODES_PER_FRONT: f64 = 2000.0;
const MAX_FRONTS: usize = 4;
const SEED_TRIES: usize = 32;
/// Free segment ends keep this many nodes from the walls, so each has room to curl.
const WALL_MARGIN: f64 = 4.0;
const ANCHOR_P: f64 = 0.3;
/// Excited strip width and the recovering tail behind it, in nodes.
const FRONT_W: f64 = 2.0;
const TAIL_W: f64 = 6.0;
/// `v` along the tail: every value ≥ 0.315 keeps `u_th > ½`, so the tail blocks
/// backward propagation, and the ramp already shades as a tail at t = 0.
const TAIL_V: (f64, f64) = (0.69, 0.36);
/// A second pacemaker once the lattice holds this many nodes.
const TWO_PACERS: usize = 8000;
/// Seed segments keep half a wavelength apart, so no two seeded comets (each ≤ 6 nodes
/// from its segment) touch or cross (BZ-A7).
const SEED_GAP: f64 = 0.5 * LAMBDA;
/// A fallback segment is never shrunk below this many nodes: shorter ones cannot launch
/// a wave (BZ-A2's radius-0.5 case), and the dish would sit dark until the pacemaker.
const MIN_FALLBACK_LEN: f64 = 3.0;

// ---- Display (SPEC-002 Resize, Inks, ASCII, Label) ----

const MAX_NODES: usize = 20_000;
const BUCKET: f64 = 1.25;
/// `v` scale for the excitation-cycle phase: `a − b₀`, the unquenched value, so the
/// quench fades tails instead of brightening them.
const R_SCALE: f32 = A - B0;
/// Phases below this show as the resting medium (the terminal background).
const S_MIN: f32 = 0.06;
/// Reseeded fronts brighten over this many seconds (display only, SPEC-002 Motion).
const FADE_IN_S: f64 = 0.5;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// Everything one cycle's seeding draws (BZ-A7): fronts and normalised pacemakers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Seeds {
    pub(crate) fronts: [Option<Front>; MAX_FRONTS],
    pub(crate) pacers: [[f64; 2]; 2],
    pub(crate) n_pacers: usize,
}

#[derive(Debug)]
struct Bz {
    u: Grid<f32>,
    v: Grid<f32>,
    /// Laplacian scratch for `bz_step`.
    lap: Grid<f32>,
    /// Lattice → pixel sampling tables, rebuilt in `resize`.
    map: SampleMap,
    /// Shaded in `update`/`resize`; `render` only blits it.
    pixels: Pixels,
    timeline: Timeline,
    /// Normalised pacemaker positions in `[0, 1]²`.
    pacers: [[f64; 2]; 2],
    n_pacers: usize,
    /// Sub-steps run so far: the scene clock.
    step: u64,
    /// Fractional sub-steps owed to the clock.
    owed: f64,
    /// The cycle whose seeds are painted.
    seeded_cycle: u64,
    /// Sub-step of the latest cycle reseed, for the display fade-in; `None` for the
    /// first seeding, which shows at once.
    reseeded_at: Option<u64>,
    /// Live `b`, for the label.
    b: f32,
    cells: Size,
    /// `title::stable_readout_room` at this size: the label never changes form with
    /// the caption (CORE-R83).
    readout_room: usize,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Bz {
    fn default() -> Self {
        Self {
            dirty: true,
            u: Grid::default(),
            v: Grid::default(),
            lap: Grid::default(),
            map: SampleMap::default(),
            pixels: Pixels::default(),
            timeline: Timeline::new(&[("waves", WAVES_S as f64), ("quench", QUENCH_S as f64)]),
            pacers: [[0.5; 2]; 2],
            n_pacers: 0,
            step: 0,
            owed: 0.0,
            seeded_cycle: 0,
            reseeded_at: None,
            b: B0,
            cells: Size::new(1, 1),
            readout_room: 0,
        }
    }
}

/// Barkley's model with the EZ-Spiral constants (module docs), from the shared
/// excitable-medium kernel.
const MODEL: Barkley = Barkley {
    a: A,
    eps: EPS,
    h: H,
    tau: TAU,
};

/// One Lie-split sub-step of Barkley's model in place (module docs). `lap` is scratch.
pub(crate) fn bz_step(u: &mut Grid<f32>, v: &mut Grid<f32>, lap: &mut Grid<f32>, b: f32) {
    MODEL.step(u, v, lap, |_| b);
}

/// `b` at cycle-local sub-step `local`: `b₀` while the waves run, then a smoothstep
/// ramp to `B_QUENCH` over the first half second of the quench.
fn b_at(timeline: &Timeline, local: u64) -> f32 {
    let beat = timeline.at(local as f64 / SUB_PER_S as f64);
    if beat.index == QUENCH {
        let k = ease::smoothstep(beat.local / QUENCH_RAMP_S);
        ease::lerp(f64::from(B0), f64::from(B_QUENCH), k) as f32
    } else {
        B0
    }
}

/// Whether the pacemakers fire at cycle-local sub-step `local` (never in the quench).
pub(crate) fn pacemaker_fires(local: u64) -> bool {
    (PM_FIRST..WAVES_STEPS).contains(&local) && (local - PM_FIRST) % PM_PERIOD == 0
}

/// Excite every recovered node (`v < 0.1`) within 2.5 nodes of `(cx, cy)`.
pub(crate) fn fire_pacemaker(u: &mut Grid<f32>, v: &Grid<f32>, cx: f64, cy: f64) {
    excitable::excite_disc(u, v, (cx, cy), PM_RADIUS, PM_V_MAX);
}

/// Paint `front` into the fields with BZ's strip widths (the shared
/// [`excitable::paint_front`]).
pub(crate) fn paint_front(u: &mut Grid<f32>, v: &mut Grid<f32>, f: &Front) {
    excitable::paint_front(u, v, f, &BZ_STROKE);
}

/// BZ's painted front: `FRONT_W` excited nodes backed by a `TAIL_W`-node tail.
const BZ_STROKE: FrontStroke = FrontStroke {
    front_w: FRONT_W,
    tail_w: TAIL_W,
    tail_v: TAIL_V,
};

/// Distance from `p` to the segment `a → b`.
fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (d, len) = unit(a, b);
    let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]).clamp(0.0, len);
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}

/// Shortest distance between two seed segments: 0 if they cross, else the nearest
/// endpoint-to-segment distance.
pub(crate) fn segment_gap(f: &Front, g: &Front) -> f64 {
    let side = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let straddles = |f: &Front, g: &Front| side(g.a, f.a, f.b) * side(g.b, f.a, f.b) < 0.0;
    if straddles(f, g) && straddles(g, f) {
        return 0.0;
    }
    [
        point_segment(f.a, g.a, g.b),
        point_segment(f.b, g.a, g.b),
        point_segment(g.a, f.a, f.b),
        point_segment(g.b, f.a, f.b),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}

/// Whether `p` keeps `margin` nodes from every wall of a `w × h` lattice.
fn clear_of_walls(p: [f64; 2], w: f64, h: f64, margin: f64) -> bool {
    (margin..=w - 1.0 - margin).contains(&p[0]) && (margin..=h - 1.0 - margin).contains(&p[1])
}

/// Distance from `p` along unit `d` to the lattice edge, plus one node so the painted
/// strip reaches the wall nodes.
fn to_wall(p: [f64; 2], d: [f64; 2], w: f64, h: f64) -> f64 {
    let axis = |p: f64, d: f64, hi: f64| {
        if d > 0.0 {
            (hi - p) / d
        } else if d < 0.0 {
            -p / d
        } else {
            f64::INFINITY
        }
    };
    axis(p[0], d[0], w - 1.0)
        .min(axis(p[1], d[1], h - 1.0))
        .max(0.0)
        + 1.0
}

/// The segment of length `len` centred on `mid` along `d`, shrunk in 10 % steps (not
/// below `min_len`) until its free ends keep the wall margin and it passes `ok`; `None`
/// if it never does. An anchored segment extends its end nearer a wall to that wall
/// and returns it as `b`; only its free end `a` is checked against the walls.
fn fit(
    mid: [f64; 2],
    d: [f64; 2],
    len: f64,
    anchored: bool,
    min_len: f64,
    lat: (f64, f64),
    ok: impl Fn(&Front) -> bool,
) -> Option<Front> {
    let (w, h) = lat;
    let n = [-d[1], d[0]];
    let mut len = len;
    loop {
        let half = 0.5 * len;
        let mut a = [mid[0] - d[0] * half, mid[1] - d[1] * half];
        let mut b = [mid[0] + d[0] * half, mid[1] + d[1] * half];
        if anchored {
            // Anchor the end nearer its wall: extending the far end would draw a front
            // across the whole dish and leave no room for the others. `a` stays free.
            let back = [-d[0], -d[1]];
            let (reach_b, reach_a) = (to_wall(b, d, w, h), to_wall(a, back, w, h));
            if reach_a < reach_b {
                (a, b) = (b, [a[0] + back[0] * reach_a, a[1] + back[1] * reach_a]);
            } else {
                b = [b[0] + d[0] * reach_b, b[1] + d[1] * reach_b];
            }
        }
        let f = Front { a, b, n, anchored };
        let fits = clear_of_walls(a, w, h, WALL_MARGIN)
            && (anchored || clear_of_walls(b, w, h, WALL_MARGIN));
        if fits && ok(&f) {
            return Some(f);
        }
        if len <= min_len {
            return None;
        }
        len = (len * 0.9).max(min_len);
    }
}

/// Draw one cycle's fronts and pacemakers for a `lat`-node lattice (SPEC-002 Model,
/// BZ-A7): up to `k` broken fronts, free ends clear of the walls and segments `SEED_GAP`
/// apart, 30 % of them anchored to a wall. A front that finds no room is dropped, so a
/// crowded dish gets fewer fronts instead of two tangled ones.
pub(crate) fn plan_seeds(lat: Size, rng: &mut impl Rng) -> Seeds {
    let (w, h) = (f64::from(lat.width()), f64::from(lat.height()));
    let nodes = lat.area();
    let k = ((nodes as f64 / NODES_PER_FRONT).round() as usize).clamp(1, MAX_FRONTS);
    let mut seeds = Seeds {
        fronts: [None; MAX_FRONTS],
        pacers: [[0.0; 2]; 2],
        n_pacers: if nodes >= TWO_PACERS { 2 } else { 1 },
    };
    for i in 0..k {
        let apart = |f: &Front| {
            seeds.fronts[..i]
                .iter()
                .flatten()
                .all(|g| segment_gap(f, g) >= SEED_GAP)
        };
        let mut last = None;
        for _ in 0..SEED_TRIES {
            let len = rng.random_range(1.2..=2.0) * LAMBDA;
            let (sin, cos) = rng.random_range(0.0..TWO_PI).sin_cos_det();
            let mid = [
                w * rng.random_range(0.2..=0.8),
                h * rng.random_range(0.2..=0.8),
            ];
            let anchored = rng.random_bool(ANCHOR_P);
            let d = [cos, sin];
            last = Some((mid, d, len, anchored));
            if let Some(f) = fit(mid, d, len, anchored, 0.8 * LAMBDA, (w, h), apart) {
                seeds.fronts[i] = Some(f);
                break;
            }
        }
        if i == 0
            && seeds.fronts[0].is_none()
            && let Some((mid, d, len, anchored)) = last
        {
            // A dish too small for any draw still gets one front: the last draw shrunk
            // as far as needed (never so short it cannot launch a wave), else clipped
            // to the lattice.
            let f = fit(mid, d, len, anchored, MIN_FALLBACK_LEN, (w, h), |_| true).unwrap_or_else(
                || {
                    let clip = |p: [f64; 2]| [p[0].clamp(0.0, w - 1.0), p[1].clamp(0.0, h - 1.0)];
                    let half = 0.5 * len;
                    Front {
                        a: clip([mid[0] - d[0] * half, mid[1] - d[1] * half]),
                        b: clip([mid[0] + d[0] * half, mid[1] + d[1] * half]),
                        n: [-d[1], d[0]],
                        anchored: false,
                    }
                },
            );
            seeds.fronts[0] = Some(f);
        }
    }
    for p in &mut seeds.pacers {
        *p = [rng.random_range(0.15..=0.85), rng.random_range(0.15..=0.85)];
    }
    seeds
}

/// The excitation-cycle phase `s ∈ [0, 1]` of a node (SPEC-002 Inks): 1 at the fresh
/// front, falling over the excited plateau (`s ≥ 0.55`), then down the recovering tail
/// to 0 at rest.
fn phase(u: f32, v: f32) -> f32 {
    let r = (v / R_SCALE).clamp(0.0, 1.0);
    if u >= 0.5 {
        1.0 - 0.45 * r
    } else {
        0.55 * r * r
    }
}

/// Ferroin ramp position for a phase, piecewise linear between these knots (BZ-A9): the
/// leading edge (`s ≥ 0.92`, the front's first node or two) takes the pale-azure top,
/// the rest of the plateau saturated blue-violet, and the tail a dark crimson-to-wine
/// fade into the blank rest.
const FERROIN_KNOTS: [(f32, f32); 6] = [
    (S_MIN, 0.03),
    (0.30, 0.14),
    (0.55, 0.45),
    (0.88, 0.66),
    (0.92, 0.86),
    (1.00, 1.00),
];

/// The ferroin level of a phase `s ≥ S_MIN` (BZ-A9): never falling as `s` rises, 255 at 1.
const fn phase_level(s: f32) -> u8 {
    let mut i = 1;
    while i < FERROIN_KNOTS.len() - 1 && s > FERROIN_KNOTS[i].0 {
        i += 1;
    }
    let ((s0, t0), (s1, t1)) = (FERROIN_KNOTS[i - 1], FERROIN_KNOTS[i]);
    let t = t0 + (t1 - t0) * (s - s0) / (s1 - s0);
    // Round half up; the clamp keeps it in 0..=255 for s outside the knots.
    (t.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The phase as a colour in the ferroin ramp (BZ-A9). The ramp borrows the indicator's
/// colours (red ferroin, blue ferriin), so fronts read blue and the tail reddens as the
/// medium recovers, echoing the indicator; hue and lightness together encode the phase
/// `s`, not the ferriin concentration `v` (false colour), so the red comes earlier than
/// ferriin actually falls. The resting medium, ferroin-red in a real dish, is left to the
/// terminal background so the waves carry the colour.
fn phase_ink(s: f32) -> Ink {
    /// [`phase_level`] at `s = i/1023`, built at compile time: one lookup per pixel.
    const LEVELS: [u8; 1024] = {
        let mut t = [0; 1024];
        let mut i = 0;
        while i < t.len() {
            t[i] = phase_level(i as f32 / 1023.0);
            i += 1;
        }
        t
    };
    if s < S_MIN {
        Ink::BLANK
    } else {
        // `s ≤ 1` (phase's range), and the `min` guards the index anyway.
        Ink::new(
            Role::Ramp(Ramp::Ferroin),
            LEVELS[((s * 1023.0 + 0.5) as usize).min(1023)],
        )
    }
}

/// ASCII for a cell from the brighter of its two pixels (SPEC-002 ASCII): the M10
/// thresholds `s` = 0.85, 0.70, 0.40, 0.20, read through [`phase_level`]; the resting
/// medium is blank.
fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    const AT: [u8; 4] = [
        phase_level(0.85),
        phase_level(0.70),
        phase_level(0.40),
        phase_level(0.20),
    ];
    let lit = |i: Ink| i.role != Role::Background;
    let ink = match (lit(top), lit(bottom)) {
        (false, false) => return None,
        (true, false) => top,
        (false, true) => bottom,
        (true, true) if top.level >= bottom.level => top,
        (true, true) => bottom,
    };
    let ch = match ink.level {
        l if l >= AT[0] => '@',
        l if l >= AT[1] => '#',
        l if l >= AT[2] => '+',
        l if l >= AT[3] => ':',
        _ => '.',
    };
    Some((ch, ink))
}

impl Bz {
    /// Paint a fresh seed set over the (dark) field.
    fn reseed(&mut self, rng: &mut impl Rng) {
        let seeds = plan_seeds(self.u.size(), rng);
        for f in seeds.fronts.iter().flatten() {
            paint_front(&mut self.u, &mut self.v, f);
        }
        self.pacers = seeds.pacers;
        self.n_pacers = seeds.n_pacers;
    }

    /// One scheduled sub-step: reseed on a new cycle, set `b`, fire pacemakers, react.
    fn substep(&mut self, rng: &mut impl Rng) {
        let (cycle, local) = (self.step / CYCLE_STEPS, self.step % CYCLE_STEPS);
        if cycle != self.seeded_cycle {
            self.seeded_cycle = cycle;
            self.reseeded_at = Some(self.step);
            self.reseed(rng);
        }
        self.b = b_at(&self.timeline, local);
        if pacemaker_fires(local) {
            let (w, h) = (f64::from(self.u.width()), f64::from(self.u.height()));
            for p in &self.pacers[..self.n_pacers] {
                fire_pacemaker(&mut self.u, &self.v, p[0] * (w - 1.0), p[1] * (h - 1.0));
            }
        }
        bz_step(&mut self.u, &mut self.v, &mut self.lap, self.b);
        self.step += 1;
    }

    /// Shade the lattice into the pixel buffer: `u` and `v` sampled bilinearly per
    /// pixel (the identity when the lattice is the pixel grid), then the phase. Shading
    /// after sampling keeps the `u = ½` threshold sharp: interpolating the phase instead
    /// would put refractory-coloured pixels ahead of a front on a stretched lattice.
    fn rasterise(&mut self) {
        // Reseeded fronts brighten from dark (display only: the model is untouched).
        let fade = self.reseeded_at.map_or(1.0, |at| {
            let age = self.step.saturating_sub(at) as f64 / SUB_PER_S as f64;
            ease::smoothstep(age / FADE_IN_S) as f32
        });
        let (map, u, v) = (&self.map, &self.u, &self.v);
        // Row samplers, rebuilt when the shader reaches the next pixel row (CORE-R104).
        let (mut ry, mut ur, mut vr) = (-1, map.row(u, 0), map.row(v, 0));
        self.pixels.shade(|x, y| {
            if y != ry {
                (ry, ur, vr) = (y, map.row(u, y), map.row(v, y));
            }
            phase_ink(fade * phase(ur.at(x), vr.at(x)))
        });
    }
}

impl Generator for Bz {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        let beat = self
            .timeline
            .at((self.step % CYCLE_STEPS) as f64 / SUB_PER_S as f64);
        (beat.index == QUENCH).then_some(CAPTIONS[0])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let lat = grid::lattice_for(px, MAX_NODES, BUCKET);
        if self.u.is_empty() {
            self.u = Grid::new(lat, 0.0);
            self.v = Grid::new(lat, 0.0);
            self.seeded_cycle = self.step / CYCLE_STEPS;
            self.reseed(&mut ctx.rng);
        } else if lat != self.u.size() {
            // ADR-0012: carry the pattern over; bilinear weights are convex, so [0, 1]
            // survives, and the waves relax to their natural λ within a few turns.
            // Resampled through the Laplacian scratch, which `bz_step` reshapes anyway.
            for field in [&mut self.u, &mut self.v] {
                grid::regrid_bilinear(field, lat, &mut self.lap);
            }
        }
        self.map.rebuild(lat, px);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        // 1e-9 of slack so exact multiples (2 sub-steps per 1/60 s) never lose one.
        self.owed += dt.min(0.25) * SUB_PER_S as f64;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        for _ in 0..n as u32 {
            self.substep(&mut ctx.rng);
        }
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        let (w, h) = (self.cells.width(), self.cells.height());
        if h < 4 || w < 14 {
            return;
        }
        // Numbers only: the title names the reaction and its caption the beat. The room
        // is right of every caption's title (GEN-R13, CORE-R83) and at most half the row
        // (CORE-R76), so the label never competes with the waves. `b` is live: it reads
        // 0.45 in the quench, the sign that the quench is a loss of excitability.
        let b = self.b;
        let mut label = StackText::<40>::new();
        // Each threshold is its form's length. Writing into a StackText never fails.
        let _ = match self.readout_room {
            31.. => write!(label, "Barkley a=0.75 b={b:.2} eps=0.02"),
            6.. => write!(label, "b={b:.2}"),
            _ => return,
        };
        // label_right pads it with a blank cell each side, so it never touches the field.
        draw::label_right(canvas, i32::from(h) - 1, label.as_str(), LABEL_INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grids(w: u16, h: u16) -> (Grid<f32>, Grid<f32>, Grid<f32>) {
        let s = Size::new(w, h);
        (Grid::new(s, 0.0), Grid::new(s, 0.0), Grid::default())
    }

    fn run(u: &mut Grid<f32>, v: &mut Grid<f32>, lap: &mut Grid<f32>, steps: usize) {
        for _ in 0..steps {
            bz_step(u, v, lap, B0);
        }
    }

    /// Sub-steps in `tu` time units.
    fn steps(tu: f32) -> usize {
        (tu / TAU).round() as usize
    }

    fn max(g: &Grid<f32>) -> f32 {
        g.as_slice().iter().fold(0.0, |m, &x| m.max(x))
    }

    fn disc(u: &mut Grid<f32>, cx: f64, cy: f64, r: f64, val: f32) {
        let (w, h) = (i32::from(u.width()), i32::from(u.height()));
        for y in 0..h {
            for x in 0..w {
                if (f64::from(x) - cx).hypot(f64::from(y) - cy) <= r {
                    u.set(x, y, val);
                }
            }
        }
    }

    /// Farthest excited node (`u ≥ ½`) from `(cx, cy)`.
    fn reach(u: &Grid<f32>, cx: f64, cy: f64) -> f64 {
        u.iter()
            .filter(|(_, u)| **u >= 0.5)
            .map(|((x, y), _)| (f64::from(x) - cx).hypot(f64::from(y) - cy))
            .fold(0.0, f64::max)
    }

    #[test]
    fn bz_a1_step_keeps_fields_in_unit_interval_without_clamps() {
        let mut rng = SplitMix64::new(1);
        for b in [B0, B_QUENCH] {
            for _ in 0..64 {
                let (mut u, mut v, mut lap) = grids(24, 16);
                for x in u.as_mut_slice().iter_mut().chain(v.as_mut_slice()) {
                    *x = rng.random_range(0.0..=1.0);
                }
                for _ in 0..50 {
                    bz_step(&mut u, &mut v, &mut lap, b);
                    for &x in u.as_slice().iter().chain(v.as_slice()) {
                        assert!((0.0..=1.0).contains(&x), "b={b}: {x} left [0, 1]");
                    }
                }
            }
        }
        let (mut u, mut v, mut lap) = grids(24, 16);
        run(&mut u, &mut v, &mut lap, 10);
        assert!(u.as_slice().iter().chain(v.as_slice()).all(|&x| x == 0.0));
    }

    #[test]
    fn bz_a2_excitation_is_all_or_none() {
        let (mut u, mut v, mut lap) = grids(64, 64);
        disc(&mut u, 32.0, 32.0, 4.0, 0.9 * B0 / A);
        run(&mut u, &mut v, &mut lap, steps(2.0));
        assert!(
            max(&u) < 1e-3,
            "sub-threshold disc left max u = {}",
            max(&u)
        );

        // The spatial critical nucleus: above the kinetic threshold b/a = 0.08 but below
        // the radius-4 critical amplitude (≈ 0.126 on this scheme), diffusion wins; and a
        // single fully excited node (radius 0.5) cannot launch a ring either.
        for (r, amp) in [(4.0, 0.11), (0.5, 1.0)] {
            let (mut u, mut v, mut lap) = grids(64, 64);
            disc(&mut u, 32.0, 32.0, r, amp);
            run(&mut u, &mut v, &mut lap, steps(3.0));
            assert!(max(&u) < 1e-3, "r={r} u={amp}: max u = {}", max(&u));
        }

        for r in [1.5, 4.0] {
            let (mut u, mut v, mut lap) = grids(64, 64);
            disc(&mut u, 32.0, 32.0, r, 1.0);
            run(&mut u, &mut v, &mut lap, steps(2.0));
            let reach = reach(&u, 32.0, 32.0);
            assert!(reach >= 12.0, "r={r}: ring reached only {reach} nodes");
        }
    }

    /// Positions (nodes) of every front on the middle row of a strip: the leading
    /// `u = ½` crossing of each excited run, interpolated.
    fn fronts(u: &Grid<f32>) -> Vec<f64> {
        let row: Vec<f32> = (0..i32::from(u.width()))
            .map(|x| *u.get(x, 1).unwrap())
            .collect();
        let mut out: Vec<f64> = row
            .windows(2)
            .enumerate()
            .filter(|(_, p)| p[0] >= 0.5 && p[1] < 0.5)
            .map(|(x, p)| x as f64 + f64::from((p[0] - 0.5) / (p[0] - p[1])))
            .collect();
        out.sort_by(|a, b| b.total_cmp(a));
        out
    }

    #[test]
    fn bz_a3_planar_speed_and_pacemaker_spacing() {
        let (mut u, mut v, mut lap) = grids(160, 3);
        for y in 0..3 {
            for x in 0..3 {
                u.set(x, y, 1.0);
            }
        }
        // Least-squares slope of the front position over nodes 40–120.
        let mut pts = Vec::new();
        for k in 0..steps(30.0) {
            bz_step(&mut u, &mut v, &mut lap, B0);
            if let Some(&x) = fronts(&u).first()
                && (40.0..=120.0).contains(&x)
            {
                pts.push(((k + 1) as f64 * f64::from(TAU), x));
            }
        }
        let n = pts.len() as f64;
        let (mt, mx) = pts
            .iter()
            .fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
        let sxy: f64 = pts.iter().map(|p| (p.0 - mt) * (p.1 - mx)).sum();
        let sxx: f64 = pts.iter().map(|p| (p.0 - mt) * (p.0 - mt)).sum();
        let c = sxy / sxx * f64::from(H);
        let ratio = c / f64::from(C_NAGUMO);
        assert!(
            (0.75..=0.92).contains(&ratio),
            "c = {c:.3} su/tu = {ratio:.3} c_N"
        );

        // A 6-tu pacemaker at x = 0: trailing fronts sit c·T/h apart.
        let (mut u, mut v, mut lap) = grids(160, 3);
        for k in 0..steps(20.5) {
            if k % PM_PERIOD as usize == 0 {
                fire_pacemaker(&mut u, &v, 0.0, 1.0);
            }
            bz_step(&mut u, &mut v, &mut lap, B0);
        }
        let xs = fronts(&u);
        assert!(xs.len() >= 4, "fronts at {xs:?}");
        let expect = c * 6.0 / f64::from(H);
        for pair in xs[1..].windows(2) {
            let gap = pair[0] - pair[1];
            assert!(
                (gap - expect).abs() <= 0.1 * expect,
                "spacing {gap:.1} vs {expect:.1} (fronts {xs:?})"
            );
        }
    }

    /// The spiral of BZ-A4: a wall-anchored front from the top wall to the centre of a
    /// 64×64 lattice, propagating towards +x.
    fn spiral() -> (Grid<f32>, Grid<f32>, Grid<f32>) {
        let (mut u, mut v, lap) = grids(64, 64);
        let front = Front {
            a: [31.5, 32.0],
            b: [31.5, -1.0],
            n: [1.0, 0.0],
            anchored: true,
        };
        paint_front(&mut u, &mut v, &front);
        (u, v, lap)
    }

    /// Tip blocks: 2×2 blocks crossed by both `u = ½` and `v = a/2 − b` (EZ-Spiral).
    fn tips(u: &Grid<f32>, v: &Grid<f32>, out: &mut Vec<(i32, i32)>) {
        let v_tip = 0.5 * A - B0;
        let crosses = |g: &Grid<f32>, x: i32, y: i32, c: f32| {
            let q = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .map(|(dx, dy)| *g.get(x + dx, y + dy).unwrap() - c);
            q.iter().any(|&d| d >= 0.0) && q.iter().any(|&d| d < 0.0)
        };
        for y in 0..i32::from(u.height()) - 1 {
            for x in 0..i32::from(u.width()) - 1 {
                if crosses(u, x, y, 0.5) && crosses(v, x, y, v_tip) {
                    out.push((x, y));
                }
            }
        }
    }

    #[test]
    fn bz_a4_single_spiral_rotates_rigidly() {
        let (mut u, mut v, mut lap) = spiral();
        run(&mut u, &mut v, &mut lap, steps(20.0));
        let probe = (56, 52);
        let mut last_u = *u.get(probe.0, probe.1).unwrap();
        let mut ups = Vec::new();
        let mut window = Vec::new();
        let window_steps = steps(4.0);
        for k in 1..=steps(30.0) {
            bz_step(&mut u, &mut v, &mut lap, B0);
            let now = *u.get(probe.0, probe.1).unwrap();
            if last_u < 0.5 && now >= 0.5 {
                ups.push(k as f64 * f64::from(TAU));
            }
            last_u = now;
            if k % 8 == 0 {
                tips(&u, &v, &mut window);
            }
            // Every 4 tu window, and the trailing 48–50 tu one too.
            if k % window_steps == 0 || k == steps(30.0) {
                assert!(!window.is_empty(), "no tip found in window ending {k}");
                let span = |f: fn(&(i32, i32)) -> i32| {
                    window.iter().map(f).max().unwrap() - window.iter().map(f).min().unwrap()
                };
                let (sx, sy) = (span(|p| p.0), span(|p| p.1));
                // A block-origin span of s covers s + 2 nodes: the node box is ≤ 6×6.
                assert!(
                    sx <= 4 && sy <= 4,
                    "tip blocks spread {sx}×{sy}: {window:?}"
                );
                // The core drifts ≈ 1 node per turn: the probe must stay ≥ λ from it.
                let n = window.len() as f64;
                let (mx, my) = window.iter().fold((0.0, 0.0), |m, p| {
                    (m.0 + f64::from(p.0) / n, m.1 + f64::from(p.1) / n)
                });
                let far = (f64::from(probe.0) - mx).hypot(f64::from(probe.1) - my);
                assert!(far >= LAMBDA, "probe only {far:.1} nodes from the core");
                window.clear();
            }
        }
        assert!(ups.len() >= 5, "probe saw {} fronts", ups.len());
        for p in ups.windows(2) {
            let period = p[1] - p[0];
            assert!(
                (3.7..=4.5).contains(&period),
                "period {period:.2} tu ({ups:?})"
            );
        }
    }

    #[test]
    fn bz_a5_broken_front_becomes_mirror_symmetric_pair() {
        let (mut u, mut v, mut lap) = grids(72, 48);
        let front = Front {
            a: [15.5, 24.0],
            b: [55.5, 24.0],
            n: [0.0, 1.0],
            anchored: false,
        };
        paint_front(&mut u, &mut v, &front);
        let mirrored = |g: &Grid<f32>| {
            g.iter()
                .all(|((x, y), &a)| a.to_bits() == g.get(71 - x, y).unwrap().to_bits())
        };
        for k in 0..=steps(12.0) {
            assert!(
                mirrored(&u) && mirrored(&v),
                "asymmetric after {k} sub-steps"
            );
            bz_step(&mut u, &mut v, &mut lap, B0);
        }
        let behind = |left: bool| {
            u.iter()
                .any(|((x, y), &a)| a >= 0.5 && y < 24 && (x < 36) == left)
        };
        assert!(
            behind(true) && behind(false),
            "the free ends did not curl behind the seed"
        );
    }

    fn scene(size: Size, seed: u64) -> (Bz, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut bz = Bz::default();
        bz.resize(size, &mut ctx);
        bz.prepare();
        (bz, ctx)
    }

    fn in_unit(bz: &Bz) -> bool {
        bz.u.as_slice()
            .iter()
            .chain(bz.v.as_slice())
            .all(|x| (0.0..=1.0).contains(x))
    }

    #[test]
    fn bz_a6_lattice_is_capped_and_resize_keeps_state() {
        for w in [1u16, 2, 7, 40, 80, 133, 200, 257, 400] {
            for h in [1u16, 3, 12, 24, 50, 77, 120] {
                let (bz, _) = scene(Size::new(w, h), 3);
                let lat = bz.u.size();
                assert!(lat.area() <= MAX_NODES, "{w}x{h}: {lat:?}");
                assert!(
                    lat.width() <= w && lat.height() <= 2 * h,
                    "{w}x{h}: {lat:?}"
                );
            }
        }
        let (mut bz, mut ctx) = scene(Size::new(240, 60), 5);
        for _ in 0..30 {
            bz.update(SIM_DT, &mut ctx);
            bz.prepare();
        }
        let (u, v) = (bz.u.clone(), bz.v.clone());
        bz.resize(Size::new(250, 62), &mut ctx);
        bz.prepare();
        assert_eq!((&bz.u, &bz.v), (&u, &v), "same bucket must not touch state");

        let mut rng = SplitMix64::new(9);
        for _ in 0..12 {
            let size = Size::new(rng.random_range(1..=300), rng.random_range(1..=90));
            bz.resize(size, &mut ctx);
            bz.prepare();
            bz.update(0.1, &mut ctx);
            bz.prepare();
            assert!(in_unit(&bz), "fields left [0, 1] after resize to {size:?}");
        }
    }

    #[test]
    fn bz_a7_quench_clears_dish_and_cycles_reseed() {
        let (mut bz, mut ctx) = scene(Size::new(80, 24), 42);
        // Up to the end of the quench: 62 s in 0.25 s updates.
        for _ in 0..62 * 4 {
            bz.update(0.25, &mut ctx);
            bz.prepare();
        }
        assert_eq!(bz.step, CYCLE_STEPS, "clock at the cycle end");
        assert!(max(&bz.u) < 1e-3, "max u = {}", max(&bz.u));
        // v decays as e^{-t}: the 2 s (3 tu) quench leaves ≈ 0.047, far below both the
        // 0.1 a pacemaker needs and the ≈ 0.23 that still shades (SPEC-002 BZ-A7).
        assert!(max(&bz.v) < 0.06, "max v = {}", max(&bz.v));
        assert!(
            bz.pixels_iter().all(|i| i == Ink::BLANK),
            "the quenched dish is at rest"
        );
        assert!((WAVES_STEPS..CYCLE_STEPS).all(|s| !pacemaker_fires(s)));
        assert!(pacemaker_fires(PM_FIRST) && pacemaker_fires(PM_FIRST + PM_PERIOD));

        let lat = Size::new(80, 48);
        let plan = |seed| plan_seeds(lat, &mut SplitMix64::new(seed));
        assert_eq!(plan(7), plan(7), "same seed, same fronts and pacemakers");
        let mut rng = SplitMix64::new(7);
        assert_ne!(plan_seeds(lat, &mut rng), plan_seeds(lat, &mut rng));

        for (w, h) in [(64u16, 40u16), (80, 48), (197, 100), (100, 64)] {
            let (fw, fh) = (f64::from(w), f64::from(h));
            let k = ((fw * fh / NODES_PER_FRONT).round() as usize).clamp(1, MAX_FRONTS);
            let mut rng = SplitMix64::new(u64::from(w) * 31 + u64::from(h));
            let mut dropped = 0;
            for _ in 0..50 {
                let seeds = plan_seeds(Size::new(w, h), &mut rng);
                let fronts: Vec<&Front> = seeds.fronts.iter().flatten().collect();
                dropped += k - fronts.len();
                for (i, f) in fronts.iter().enumerate() {
                    assert!(clear_of_walls(f.a, fw, fh, WALL_MARGIN), "{w}x{h}: {f:?}");
                    assert!(
                        f.anchored || clear_of_walls(f.b, fw, fh, WALL_MARGIN),
                        "{w}x{h}: {f:?}"
                    );
                    // Seeded comets never touch or cross.
                    for g in &fronts[..i] {
                        let gap = segment_gap(f, g);
                        assert!(gap >= SEED_GAP, "{w}x{h}: gap {gap:.1}: {f:?} {g:?}");
                    }
                }
            }
            // The rule rarely costs a front: ≤ 1 in 10 is dropped for want of room.
            assert!(
                5 * dropped <= k * 50 / 2,
                "{w}x{h}: {dropped} of {} dropped",
                k * 50
            );
        }
        // A dish too small for the rules still gets its one front.
        let tiny = plan_seeds(Size::new(20, 8), &mut SplitMix64::new(3));
        assert_eq!(tiny.fronts.iter().flatten().count(), 1);
    }

    #[test]
    fn bz_a7_segment_gap_measures_crossings_and_clearance() {
        let seg = |a: [f64; 2], b: [f64; 2]| Front {
            a,
            b,
            n: [0.0, 1.0],
            anchored: false,
        };
        let h = seg([0.0, 0.0], [10.0, 0.0]);
        assert_eq!(segment_gap(&h, &seg([5.0, -3.0], [5.0, 3.0])), 0.0, "an X");
        assert!((segment_gap(&h, &seg([2.0, 4.0], [8.0, 4.0])) - 4.0).abs() < 1e-12);
        // Past the end: the nearest points are the two endpoints.
        let gap = segment_gap(&h, &seg([13.0, 4.0], [20.0, 4.0]));
        assert!((gap - 5.0).abs() < 1e-12, "{gap}");
        // A T: one end touching the other segment.
        assert!(segment_gap(&h, &seg([5.0, 0.0], [5.0, 7.0])).abs() < 1e-12);
    }

    #[test]
    fn bz_a8_composition_is_thin_fronts_on_dark_medium() {
        let (mut bz, mut ctx) = scene(Size::new(80, 24), 42);
        let mut t = 0;
        for until in [10, 20, 30] {
            while t < until * 60 {
                bz.update(SIM_DT, &mut ctx);
                bz.prepare();
                t += 1;
            }
            let px: Vec<Ink> = bz.pixels_iter().collect();
            let n = px.len() as f64;
            let share = |p: &dyn Fn(Ink) -> bool| px.iter().filter(|&&i| p(i)).count() as f64 / n;
            let lit = share(&|i| i.role != Role::Background);
            let pale = share(&|i| i.role != Role::Background && i.level >= phase_level(0.85));
            assert!((0.20..=0.70).contains(&lit), "t={until}: lit {lit:.3}");
            assert!((0.01..=0.12).contains(&pale), "t={until}: pale {pale:.3}");
        }
    }

    /// Brightest level on screen (0 for a resting dish).
    fn peak_level(bz: &Bz) -> u8 {
        bz.pixels_iter().map(|i| i.level).max().unwrap_or(0)
    }

    #[test]
    fn bz_a9_ferroin_inks_rise_with_phase_on_a_blank_rest() {
        let rest = Ink::BLANK;
        assert_eq!(phase_ink(phase(0.0, 0.0)), rest, "resting medium");
        let mut last = 0;
        for i in 6..=100 {
            let ink = phase_ink(i as f32 / 100.0);
            assert_eq!(ink.role, Role::Ramp(Ramp::Ferroin));
            assert!(ink.level >= last, "s {i}%");
            last = ink.level;
        }
        assert_eq!(last, 255);
        // A pale rim only at the leading edge, a saturated body, a dark tail.
        assert!(phase_level(0.92) >= 219, "rim {}", phase_level(0.92));
        assert!(phase_level(0.88) <= 170, "plateau {}", phase_level(0.88));
        assert!(phase_level(0.55) <= 115, "back of the plateau");
        assert!(phase_level(0.10) <= 16, "tail {}", phase_level(0.10));

        // On a planar wave the rim (s ≥ 0.92) is at most 2 nodes deep, behind a lit body
        // at least 5 times as deep.
        let (mut u, mut v, mut lap) = grids(160, 3);
        for y in 0..3 {
            for x in 0..3 {
                u.set(x, y, 1.0);
            }
        }
        run(&mut u, &mut v, &mut lap, steps(10.0));
        let row: Vec<f32> = (0..160)
            .map(|x| phase(*u.get(x, 1).unwrap(), *v.get(x, 1).unwrap()))
            .collect();
        let rim = row.iter().filter(|&&s| s >= 0.92).count();
        let body = row.iter().filter(|&&s| (S_MIN..0.92).contains(&s)).count();
        assert!((1..=2).contains(&rim), "rim {rim} nodes");
        assert!(body >= 5 * rim, "body {body} nodes");
        // The ASCII keeps the M10 thresholds on s, and the resting dish stays blank.
        let at = phase_ink;
        assert_eq!(ascii_cell(rest, rest), None);
        assert_eq!(ascii_cell(at(0.9), rest).map(|c| c.0), Some('@'));
        assert_eq!(ascii_cell(rest, at(0.75)).map(|c| c.0), Some('#'));
        assert_eq!(ascii_cell(at(0.5), at(0.3)).map(|c| c.0), Some('+'));
        assert_eq!(ascii_cell(at(0.25), rest).map(|c| c.0), Some(':'));
        assert_eq!(ascii_cell(at(0.1), rest).map(|c| c.0), Some('.'));
        let (bz, _) = scene(Size::new(80, 24), 42);
        let roles = [Role::Ramp(Ramp::Ferroin), Role::Background];
        assert!(bz.pixels_iter().all(|i| roles.contains(&i.role)));
    }

    #[test]
    fn bz_a7_reseeded_fronts_fade_in_without_touching_the_model() {
        let (mut bz, mut ctx) = scene(Size::new(80, 24), 42);
        let pale = phase_level(0.85);
        assert!(peak_level(&bz) >= pale, "the first seeding shows at once");
        for _ in 0..62 * 4 {
            bz.update(0.25, &mut ctx);
            bz.prepare();
        }
        // One sub-step past the wrap: fronts are painted at full strength in the model
        // but still dark on screen.
        bz.update(1.0 / SUB_PER_S as f64, &mut ctx);
        bz.prepare();
        assert_eq!(bz.step, CYCLE_STEPS + 1);
        assert!(max(&bz.u) > 0.9, "the model reseeds at full strength");
        assert!(
            peak_level(&bz) < phase_level(0.2),
            "reseed pops in at {}",
            peak_level(&bz)
        );
        let mut last = peak_level(&bz);
        for _ in 0..30 {
            bz.update(SIM_DT, &mut ctx);
            bz.prepare();
            let now = peak_level(&bz);
            assert!(now >= last, "fade-in dimmed {last} -> {now}");
            last = now;
        }
        assert!(last >= pale, "fronts at full brightness 0.5 s in: {last}");
    }

    #[test]
    fn bz_a9_leading_edge_has_no_refractory_halo_on_a_stretched_lattice() {
        let (mut bz, _) = scene(Size::new(200, 50), 1);
        assert_ne!(
            bz.u.size(),
            bz.pixels.pixel_size(),
            "want a stretched lattice"
        );
        // A fresh front: excited (u = 1, v = 0) left of node 90, resting medium right.
        bz.v.fill(0.0);
        for y in 0..i32::from(bz.u.height()) {
            for x in 0..i32::from(bz.u.width()) {
                bz.u.set(x, y, if x < 90 { 1.0 } else { 0.0 });
            }
        }
        bz.rasterise();
        for ink in bz.pixels_iter().filter(|&i| i != Ink::BLANK) {
            assert!(
                ink.level >= phase_level(0.85),
                "halo {ink:?} ahead of the front"
            );
        }
    }

    /// The bottom row at width `w`, `after` sub-steps into the scene (clock only: the
    /// label reads `b`, not the fields).
    fn label(w: u16, after: u64) -> String {
        let size = Size::new(w, 12);
        let (mut bz, _) = scene(size, 3);
        bz.step = after;
        bz.b = b_at(&bz.timeline, after % CYCLE_STEPS);
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        bz.render(&mut canvas);
        canvas.to_lines()[11].clone()
    }

    #[test]
    fn bz_gen_r13_label_is_numbers_only_right_aligned_and_never_clipped() {
        let quench = WAVES_STEPS + SUB_PER_S;
        for (w, after, want) in [
            (14u16, 0, "b=0.06"),
            (40, 0, "b=0.06"),
            (40, quench, "b=0.45"),
            (68, 0, "b=0.06"),
            (69, 0, "Barkley a=0.75 b=0.06 eps=0.02"),
            (80, quench, "Barkley a=0.75 b=0.45 eps=0.02"),
        ] {
            let row = label(w, after);
            // A blank cell each side, so it never touches the field.
            assert!(row.ends_with(&format!(" {want} ")), "w={w}: {row:?}");
            // The beat is the title's caption now, never a word in the readout.
            assert!(
                !row.contains("waves  ") && !row.contains("quench "),
                "w={w}: {row:?}"
            );
        }
        // The form follows the stable room (CORE-R83), so it never changes with the beat.
        for w in 14u16..=120 {
            let (waves, quenched) = (label(w, 0), label(w, quench));
            assert_eq!(
                waves.replace("0.06", "0.45"),
                quenched,
                "w={w}: the readout flickers with the beat"
            );
        }
        assert!(!label(13, 0).contains("b="), "no label below 14 columns");
    }

    #[test]
    fn bz_core_r71_quench_is_the_caption() {
        let (mut bz, _) = scene(Size::new(80, 24), 3);
        assert_eq!(bz.caption(), None, "waves: the title's own mechanism");
        bz.step = WAVES_STEPS;
        assert_eq!(bz.caption(), Some("light quench"));
        assert_eq!(META.captions, &["light quench"]);
        bz.step = CYCLE_STEPS;
        assert_eq!(bz.caption(), None, "the next cycle's waves");
    }

    impl Bz {
        fn pixels_iter(&self) -> impl Iterator<Item = Ink> + '_ {
            let px = self.pixels.pixel_size();
            let (w, h) = (i32::from(px.width()), i32::from(px.height()));
            (0..h).flat_map(move |y| (0..w).map(move |x| self.pixels.get(x, y).unwrap()))
        }
    }
}
