//! Three equal suns dancing periodic three-body orbits, each body a cloud of shadow copies
//! that shows the chaos (SPEC-002 `three_body`).
//!
//! M17 (orbital-thread style, GEN-R26): a gallery of seven periodic solutions of the planar
//! equal-mass problem (`G = m = 1`): the figure-8, Šuvakov & Dmitrašinović's (2013)
//! butterfly I, moth I, goggles, dragonfly and yin-yang Ia, and Lagrange's (1772)
//! triangle. Every trail is a strange-attractor thread with an afterglow, coloured by
//! speed (`Accent` slow → `Hot` → `Sun` fast), so each knot draws itself. Each body is a
//! tiny hydrogen-style cloud of 48 shadow copies started 10⁻⁵ apart on the same energy
//! shell: on stable orbits the cloud stays a dot, on unstable ones (goggles, Lagrange) it
//! smears along the orbit before the scene moves on.
//!
//! Integrator: the logarithmic-Hamiltonian leapfrog (Mikkola & Tanikawa 1999; Preto &
//! Tremaine 1999) composed to fourth order (Yoshida 1990). Time is transformed by
//! `dt = ds/|U|`, so a fixed step `ds` takes small physical steps through the close
//! encounters (the dragonfly passes within 0.007) and large ones elsewhere, and the
//! two-body part of an encounter is followed without the usual energy kick. The state is
//! a pure function of the step count; a frame shows each system at the frame's model time
//! by a drift of less than one step from its last step.

use std::sync::OnceLock;

use sciart_core::ink::{self, MIN_VISIBLE_LEVEL};
use sciart_core::prelude::*;
use sciart_core::rng::SplitMix64;
use sciart_core::timeline::ease;
use sciart_core::geom;

pub static META: Meta = Meta {
    name: "three_body",
    category: Category::Physics,
    description: "Three equal suns on periodic three-body orbits, their shadow clouds showing chaos",
    title: "Three bodies · periodic orbits",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ThreeBody::new())
}

/// The gallery's captions (orbit and discovery year, GEN-R12), in cycle order.
const CAPTIONS: [&str; ORBITS] = [
    "moth I 2013",
    "butterfly I 2013",
    "goggles 2013",
    "dragonfly 2013",
    "figure-8 1993",
    "yin-yang 2013",
    "Lagrange 1772",
];
const ORBITS: usize = 7;
/// Fictitious-time step of the main bodies and of the shadow copies.
pub(crate) const H_MAIN: f64 = 0.004;
const H_GHOST: f64 = 0.008;
/// Shadow copies per body and their initial scatter (each coordinate, model units).
pub(crate) const GHOSTS: usize = 48;
pub(crate) const GHOST_EPS: f64 = 1e-5;
/// Seconds each orbit fades out while its physics keeps running, and fades in.
const FADE: f64 = 1.2;
const FADE_IN: f64 = 0.3;
/// Trail: a sample whenever a body has moved `TRAIL_GAP` (model units) or `1/TRAIL_MIN`
/// of a period has passed; one period long, and pre-rolled: every run starts with the
/// trail of the period that ends at its ICs (the orbit is periodic), so each knot is
/// drawn whole from the first frame. The ring holds one period plus `TRAIL_SLACK`.
const TRAIL_GAP: f64 = 0.02;
const TRAIL_MIN: f64 = 300.0;
const TRAIL_SLACK: usize = 600;
/// Shadow copies are drawn only within `GHOST_CAP` (model units) of their body, so an
/// unstable orbit shows a comet-like smear along the orbit rather than copies flung
/// across the screen; within `GHOST_NEAR` they are at the brighter level.
const GHOST_CAP: f64 = 0.35;
const GHOST_NEAR: f64 = 0.04;
/// Speed bands in units of the orbit's rms speed `√(−2E/3)` (virial): below `SLOW`
/// `Accent`, above `FAST` `Sun`, `Hot` between (TB-A11).
const SLOW: f64 = 0.7;
const FAST: f64 = 1.25;
/// Inks: the shadow cloud `Cold`, the body itself a small white `Star` disc.
const GHOST_LEVEL: f64 = 225.0;
const GHOST_FAR_LEVEL: f64 = 150.0;
const BODY_LEVEL: f64 = 255.0;
const BODY_R: f64 = 1.2;
/// Yoshida's fourth-order weights.
const W1: f64 = 1.351_207_191_959_657_8;
const W0: f64 = -1.702_414_383_919_315_5;

/// One periodic solution: initial conditions plus how to show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Orbit {
    pub caption: &'static str,
    /// Interleaved `(x, y)` of bodies 1..3, and their velocities.
    pub pos: [f64; 6],
    pub vel: [f64; 6],
    /// Period in model time units: the published one, and the one the scene uses.
    pub published: f64,
    pub period: f64,
    /// Periods shown per run (`n`) and model time units per simulated second (`k`).
    pub periods: f64,
    pub rate: f64,
}

impl Orbit {
    /// Seconds of the `run` phase, `n·T/k`.
    pub fn run_secs(&self) -> f64 {
        self.periods * self.period / self.rate
    }
}

/// Lagrange's homographic solution with eccentricity `e`: bodies at apocentre on the unit
/// circle at `90° + 120°·i`, moving perpendicular to the radius at `v_a = √(μ(1 − e))`,
/// `μ = 1/√3` the net central pull `μ/r²` on each. Returns `(pos, vel, period)` with
/// `T = 2π√(a³/μ)`, `a = 1/(1 + e)`.
pub(crate) fn lagrange_ics(e: f64) -> ([f64; 6], [f64; 6], f64) {
    let mu = 1.0 / 3f64.sqrt();
    let v = (mu * (1.0 - e)).sqrt();
    let (mut pos, mut vel) = ([0.0; 6], [0.0; 6]);
    for i in 0..3 {
        let (s, c) = (90f64 + 120.0 * i as f64).to_radians().sin_cos_det();
        pos[2 * i] = c;
        pos[2 * i + 1] = s;
        vel[2 * i] = -s * v;
        vel[2 * i + 1] = c * v;
    }
    let a = 1.0 / (1.0 + e);
    (pos, vel, std::f64::consts::TAU * (a * a * a / mu).sqrt())
}

/// A Šuvakov–Dmitrašinović zero-angular-momentum orbit: bodies at `(−1, 0)`, `(1, 0)`,
/// `(0, 0)` with velocities `(p₁, p₂)`, `(p₁, p₂)`, `(−2p₁, −2p₂)`.
fn suvakov(p1: f64, p2: f64) -> ([f64; 6], [f64; 6]) {
    (
        [-1.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [p1, p2, p1, p2, -2.0 * p1, -2.0 * p2],
    )
}

/// The gallery in cycle order. Periods: the published `T` (Šuvakov & Dmitrašinović 2013,
/// PRL 110 114301, 4–5 digits) and the one used, refined to the return minimum of these
/// six-digit ICs (TB-A8); the figure-8 uses Simó's ICs and period.
// One IC set per line, as the tables are published.
#[rustfmt::skip]
pub(crate) fn orbits() -> [Orbit; ORBITS] {
    let (lx, lv, lt) = lagrange_ics(0.3);
    let (x1, y1) = (0.970_004_36, -0.243_087_53);
    let (vx3, vy3) = (-0.932_407_37, -0.864_731_46);
    let o = |caption, (pos, vel): ([f64; 6], [f64; 6]), published, period, periods, rate| Orbit {
        caption, pos, vel, published, period, periods, rate,
    };
    // Each run holds 15–20 s; goggles and Lagrange show 3 periods, so their shadow
    // clouds (×30–100 per period) smear in the last one.
    [
        o(CAPTIONS[0], suvakov(0.464_445, 0.396_060), 14.8939, 14.894_293, 1.5, 1.4),
        o(CAPTIONS[1], suvakov(0.306_893, 0.125_507), 6.2356, 6.234_674, 2.0, 0.8),
        o(CAPTIONS[2], suvakov(0.083_300, 0.127_889), 10.4668, 10.464_854, 3.0, 1.7),
        o(CAPTIONS[3], suvakov(0.080_584, 0.588_836), 21.2710, 21.272_330, 1.5, 2.0),
        o(CAPTIONS[4], ([x1, y1, -x1, -y1, 0.0, 0.0],
                        [-vx3 / 2.0, -vy3 / 2.0, -vx3 / 2.0, -vy3 / 2.0, vx3, vy3]),
          6.325_913_98, 6.325_913_98, 2.0, 0.8),
        o(CAPTIONS[5], suvakov(0.513_938, 0.304_736), 17.3284, 17.328_807, 1.5, 1.6),
        o(CAPTIONS[6], (lx, lv), lt, lt, 3.0, 1.0),
    ]
}

/// Accelerations and potential `U = −Σ 1/rᵢⱼ` of three unit masses at `pos` (unsoftened).
/// The symmetric pair loop conserves momentum and angular momentum to rounding.
pub(crate) fn accel3(pos: &[f64; 6], acc: &mut [f64; 6]) -> f64 {
    *acc = [0.0; 6];
    let mut u = 0.0;
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let dx = pos[2 * j] - pos[2 * i];
        let dy = pos[2 * j + 1] - pos[2 * i + 1];
        let r2 = dx * dx + dy * dy;
        let r = r2.sqrt();
        u -= 1.0 / r;
        let inv3 = 1.0 / (r2 * r);
        acc[2 * i] += dx * inv3;
        acc[2 * i + 1] += dy * inv3;
        acc[2 * j] -= dx * inv3;
        acc[2 * j + 1] -= dy * inv3;
    }
    u
}

fn kinetic(vel: &[f64; 6]) -> f64 {
    0.5 * vel.iter().map(|v| v * v).sum::<f64>()
}

/// Total energy `Σ v²/2 − Σ 1/rᵢⱼ`.
pub(crate) fn energy(pos: &[f64; 6], vel: &[f64; 6]) -> f64 {
    let mut acc = [0.0; 6];
    kinetic(vel) + accel3(pos, &mut acc)
}

/// One three-body system on the log-Hamiltonian integrator: positions, velocities, model
/// time, and its binding `B = −E₀` (the time transform `T + B = −U` on shell).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sys {
    pub pos: [f64; 6],
    pub vel: [f64; 6],
    pub t: f64,
    b: f64,
}

impl Sys {
    pub fn new(pos: [f64; 6], vel: [f64; 6]) -> Self {
        Self {
            pos,
            vel,
            t: 0.0,
            b: -energy(&pos, &vel),
        }
    }

    /// Drift for fictitious time `s`: `dt = s/(T + B)`.
    fn drift(&mut self, s: f64) {
        let dt = s / (kinetic(&self.vel) + self.b);
        for (p, v) in self.pos.iter_mut().zip(&self.vel) {
            *p += dt * v;
        }
        self.t += dt;
    }

    /// One log-H leapfrog of fictitious step `s`: half drift, kick `s/(−U)`, half drift.
    fn leap(&mut self, s: f64) {
        self.drift(0.5 * s);
        let mut acc = [0.0; 6];
        let u = accel3(&self.pos, &mut acc);
        let k = s / (-u);
        for (v, a) in self.vel.iter_mut().zip(&acc) {
            *v += k * a;
        }
        self.drift(0.5 * s);
    }

    /// One fourth-order step (Yoshida's triple composition).
    pub fn step(&mut self, s: f64) {
        self.leap(W1 * s);
        self.leap(W0 * s);
        self.leap(W1 * s);
    }

    /// Body `i`'s position drifted to model time `t` (within one step of `self.t`).
    pub fn body_at(&self, i: usize, t: f64) -> (f64, f64) {
        let dt = t - self.t;
        (
            self.pos[2 * i] + dt * self.vel[2 * i],
            self.pos[2 * i + 1] + dt * self.vel[2 * i + 1],
        )
    }

    pub fn speed(&self, i: usize) -> f64 {
        self.vel[2 * i].hypot(self.vel[2 * i + 1])
    }

    fn is_finite(&self) -> bool {
        self.t.is_finite() && self.pos.iter().chain(&self.vel).all(|v| v.is_finite())
    }
}

/// A system and its next step, so a frame can stop at the last step before its time
/// without computing a step twice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Runner {
    pub cur: Sys,
    next: Sys,
    h: f64,
}

impl Runner {
    pub fn new(sys: Sys, h: f64) -> Self {
        let mut next = sys;
        next.step(h);
        Self { cur: sys, next, h }
    }

    /// Step while the next step ends at or before model time `target`, calling
    /// `on_step` after each. A system that blew up (an escape to non-finite values) stops.
    pub fn advance(&mut self, target: f64, mut on_step: impl FnMut(&Sys)) {
        while self.next.t <= target && self.next.is_finite() {
            self.cur = self.next;
            on_step(&self.cur);
            self.next.step(self.h);
        }
    }
}

/// The 48 shadow copies of an orbit: positions scattered by up to `GHOST_EPS` with the
/// centre of mass kept, velocities rescaled onto the orbit's energy shell, from a fixed
/// per-orbit seed (the same cloud every loop).
pub(crate) fn ghost_ics(orbit: &Orbit, seg: usize) -> [Sys; GHOSTS] {
    let mut rng = SplitMix64::new(0x3B0D_5EED ^ (seg as u64).wrapping_mul(0x9E37_79B9));
    let e0 = energy(&orbit.pos, &orbit.vel);
    std::array::from_fn(|_| {
        let mut d = [0.0; 6];
        for v in &mut d {
            *v = GHOST_EPS * (2.0 * rng.next_f64() - 1.0);
        }
        let (mx, my) = ((d[0] + d[2] + d[4]) / 3.0, (d[1] + d[3] + d[5]) / 3.0);
        let mut pos = orbit.pos;
        for i in 0..3 {
            pos[2 * i] += d[2 * i] - mx;
            pos[2 * i + 1] += d[2 * i + 1] - my;
        }
        let mut acc = [0.0; 6];
        let u = accel3(&pos, &mut acc);
        let k = ((e0 - u) / kinetic(&orbit.vel)).max(0.0).sqrt();
        let vel = orbit.vel.map(|v| v * k);
        Sys::new(pos, vel)
    })
}

/// Speed band ink of a trail segment at speed `v` of an orbit with rms speed `v_ref`:
/// `Accent` slow, `Hot`, `Sun` fast (TB-A11).
pub(crate) fn speed_role(v: f64, v_ref: f64) -> Role {
    let q = v / v_ref.max(1e-9);
    if q < SLOW {
        Role::Accent
    } else if q < FAST {
        Role::Hot
    } else {
        Role::Sun
    }
}

/// Afterglow level of a trail point of age fraction `f ∈ [0, 1)` (0 newest).
fn trail_level(f: f64) -> f64 {
    40.0 + 200.0 * (1.0 - f).max(0.0).powf(1.5)
}

/// Per-orbit tables measured once by integrating the orbits: the bodies' bounding box
/// `[x_min, x_max, y_min, y_max]` over run plus fade, padded by 0.5 % (TB-A6), and the
/// pre-rolled trail: the samples of the first period, with times shifted back by `T`.
struct Table {
    bbox: [f64; 4],
    pre: [Vec<TrailPt>; 3],
}

fn tables() -> &'static [Table; ORBITS] {
    static TABLES: OnceLock<[Table; ORBITS]> = OnceLock::new();
    TABLES.get_or_init(|| {
        let all = orbits();
        std::array::from_fn(|k| {
            let o = &all[k];
            let mut b = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
            let mut pre: [Vec<TrailPt>; 3] = Default::default();
            let mut last = f64::NEG_INFINITY;
            let mut on_step = |s: &Sys| {
                for i in 0..3 {
                    let (x, y) = (s.pos[2 * i], s.pos[2 * i + 1]);
                    b = [b[0].min(x), b[1].max(x), b[2].min(y), b[3].max(y)];
                }
                if s.t <= o.period {
                    sample(&mut pre, &mut last, o.period, s, -o.period);
                }
            };
            let sys = Sys::new(o.pos, o.vel);
            on_step(&sys);
            let mut r = Runner::new(sys, H_MAIN);
            r.advance(o.run_secs() * o.rate + FADE * o.rate, &mut on_step);
            let pad = 0.005 * (b[1] - b[0]).max(b[3] - b[2]);
            Table {
                bbox: [b[0] - pad, b[1] + pad, b[2] - pad, b[3] + pad],
                pre,
            }
        })
    })
}

/// The bounding box of orbit `k`.
fn bbox(k: usize) -> &'static [f64; 4] {
    &tables()[k].bbox
}

/// Push a trail sample of all three bodies of `s` (time shifted by `shift`) when one of
/// them has moved `TRAIL_GAP` since the last sample or `1/TRAIL_MIN` of a period passed.
fn sample<T: TrailSink>(trails: &mut [T; 3], last: &mut f64, period: f64, s: &Sys, shift: f64) {
    let moved = trails.iter().enumerate().any(|(i, tr)| {
        tr.newest().is_none_or(|p| {
            (s.pos[2 * i] - f64::from(p.x)).hypot(s.pos[2 * i + 1] - f64::from(p.y)) >= TRAIL_GAP
        })
    });
    if moved || s.t - *last >= period / TRAIL_MIN {
        *last = s.t;
        for (i, tr) in trails.iter_mut().enumerate() {
            tr.put(TrailPt {
                x: s.pos[2 * i] as f32,
                y: s.pos[2 * i + 1] as f32,
                t: (s.t + shift) as f32,
                v: s.speed(i) as f32,
            });
        }
    }
}

/// Where trail samples go: the pre-roll tables (`Vec`) or the scene's rings (`Trail`).
trait TrailSink {
    fn newest(&self) -> Option<&TrailPt>;
    fn put(&mut self, p: TrailPt);
}

impl TrailSink for Vec<TrailPt> {
    fn newest(&self) -> Option<&TrailPt> {
        self.last()
    }
    fn put(&mut self, p: TrailPt) {
        self.push(p);
    }
}

impl TrailSink for Trail<TrailPt> {
    fn newest(&self) -> Option<&TrailPt> {
        self.iter().next_back()
    }
    fn put(&mut self, p: TrailPt) {
        self.push(p);
    }
}

/// Model-to-screen mapping for one orbit at one canvas size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    /// Rows per model unit (x is stretched by `ASPECT`).
    scale: f64,
    centre: (f64, f64),
    origin: (f64, f64),
}

impl View {
    /// Continuous cell coordinates of model point `(x, y)`.
    pub fn cell(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.centre.0 + (x - self.origin.0) * self.scale * geom::ASPECT,
            self.centre.1 - (y - self.origin.1) * self.scale,
        )
    }

    fn dot(&self, x: f64, y: f64) -> (f64, f64) {
        let (cx, cy) = self.cell(x, y);
        Dots::from_cells(cx, cy)
    }
}

/// The largest view fitting `bbox` in 90 % of the canvas inside a one-cell margin.
pub(crate) fn view_for(size: Size, bbox: &[f64; 4]) -> View {
    let [x0, x1, y0, y1] = *bbox;
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    let fit_x = (w / 2.0 - 1.0) / (geom::ASPECT * (x1 - x0) / 2.0);
    let fit_y = (h / 2.0 - 1.0) / ((y1 - y0) / 2.0);
    View {
        scale: (0.9 * fit_x.min(fit_y)).max(0.25),
        centre: (w / 2.0, h / 2.0),
        origin: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
    }
}

/// Where the looping gallery stands at sim time `t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Phase {
    pub seg: usize,
    /// Seconds since this orbit's run started (the physics keeps going in the fade).
    pub tau: f64,
    /// Ink multiplier: 1 while running, `1 − smoothstep(u)` over the fade.
    pub fade: f64,
}

/// Ink multiplier of everything an orbit draws: faded in over `FADE_IN` s and out with
/// the orbit (visual only).
fn orbit_fade(phase: &Phase) -> f64 {
    ease::smoothstep((phase.tau / FADE_IN).min(1.0)) * phase.fade
}

/// One trail sample: position, model time and speed (`f32`: the trail is only drawn).
#[derive(Debug, Clone, Copy, Default)]
struct TrailPt {
    x: f32,
    y: f32,
    t: f32,
    v: f32,
}

pub(crate) struct ThreeBody {
    dirty: bool,
    orbits: [Orbit; ORBITS],
    timeline: Timeline,
    /// Own accumulated simulated time (GEN-R7).
    t: f64,
    phase: Phase,
    /// The model time shown now (`τ·k`).
    target: f64,
    main: Runner,
    ghosts: [Runner; GHOSTS],
    trails: [Trail<TrailPt>; 3],
    /// Time of the last trail sample.
    last_sample: f64,
    size: Size,
    view: View,
    dots: Dots,
}

impl ThreeBody {
    fn new() -> Self {
        let orbits = orbits();
        let phases: Vec<(&'static str, f64)> = orbits
            .iter()
            .flat_map(|o| [("run", o.run_secs()), ("fade", FADE)])
            .collect();
        let size = Size::new(1, 1);
        let main = Runner::new(Sys::new(orbits[0].pos, orbits[0].vel), H_MAIN);
        let ghosts = ghost_ics(&orbits[0], 0).map(|s| Runner::new(s, H_GHOST));
        let mut g = Self {
            dirty: false,
            timeline: Timeline::new(&phases),
            t: 0.0,
            phase: Phase {
                seg: 0,
                tau: 0.0,
                fade: 1.0,
            },
            target: 0.0,
            main,
            ghosts,
            trails: std::array::from_fn(|_| {
                let longest = tables().iter().map(|t| t.pre[0].len()).max().unwrap_or(0);
                Trail::new(longest + TRAIL_SLACK)
            }),
            last_sample: f64::NEG_INFINITY,
            view: view_for(size, bbox(0)),
            size,
            dots: Dots::default(),
            orbits,
        };
        // The first run starts like every other: from its ICs with its pre-rolled trail.
        g.start_run(0);
        g
    }

    pub fn phase_at(&self, t: f64) -> Phase {
        let beat = self.timeline.at(t);
        let seg = (beat.index / 2).min(ORBITS - 1);
        if beat.index % 2 == 0 {
            Phase {
                seg,
                tau: beat.local,
                fade: 1.0,
            }
        } else {
            Phase {
                seg,
                tau: self.orbits[seg].run_secs() + beat.local,
                fade: 1.0 - ease::smoothstep(beat.progress),
            }
        }
    }

    /// Restart orbit `seg` from its ICs (and its fixed shadow cloud) with empty trails.
    fn start_run(&mut self, seg: usize) {
        let o = &self.orbits[seg];
        self.main = Runner::new(Sys::new(o.pos, o.vel), H_MAIN);
        self.ghosts = ghost_ics(o, seg).map(|s| Runner::new(s, H_GHOST));
        // The pre-rolled period ending at the ICs: the knot is whole from the first frame.
        let pre = &tables()[seg].pre;
        for (trail, pre) in self.trails.iter_mut().zip(pre) {
            trail.clear();
            for &p in pre {
                trail.push(p);
            }
        }
        self.last_sample = pre[0].last().map_or(f64::NEG_INFINITY, |p| f64::from(p.t));
        self.target = 0.0;
        self.view = view_for(self.size, bbox(seg));
    }

    /// Advance every system to model time `target`, sampling the main bodies' trails.
    fn advance_to(&mut self, target: f64) {
        self.target = target;
        let period = self.orbits[self.phase.seg].period;
        let (trails, last) = (&mut self.trails, &mut self.last_sample);
        self.main.advance(target, |s| sample(trails, last, period, s, 0.0));
        for g in &mut self.ghosts {
            g.advance(target, |_| {});
        }
    }

    /// Trails, shadow clouds and bodies onto the dot lattice.
    fn rasterise(&mut self) {
        self.dots.clear();
        let (view, fade) = (self.view, orbit_fade(&self.phase));
        if fade * GHOST_LEVEL < MIN_VISIBLE_LEVEL {
            return;
        }
        let orbit = self.orbits[self.phase.seg];
        let v_ref = (-2.0 * energy(&orbit.pos, &orbit.vel) / 3.0).sqrt();
        let now = self.target;
        for (i, trail) in self.trails.iter().enumerate() {
            let (hx, hy) = self.main.cur.body_at(i, now);
            let mut prev = (view.dot(hx, hy), self.main.cur.speed(i));
            for p in trail.iter().rev() {
                let f = (now - f64::from(p.t)) / orbit.period;
                if f >= 1.0 {
                    break;
                }
                let level = trail_level(f) * fade;
                let d = view.dot(f64::from(p.x), f64::from(p.y));
                let v = f64::from(p.v);
                if level >= MIN_VISIBLE_LEVEL {
                    let role = speed_role(0.5 * (v + prev.1), v_ref);
                    self.dots
                        .line(prev.0.0, prev.0.1, d.0, d.1, Ink::new(role, ink::level(level)));
                }
                prev = (d, v);
            }
        }
        let near = Ink::new(Role::Cold, ink::level(GHOST_LEVEL * fade));
        let far = Ink::new(Role::Cold, ink::level(GHOST_FAR_LEVEL * fade));
        let bodies = [0, 1, 2].map(|i| self.main.cur.body_at(i, now));
        for g in &self.ghosts {
            for (i, b) in bodies.iter().enumerate() {
                let (x, y) = g.cur.body_at(i, now);
                let d = (x - b.0).hypot(y - b.1);
                if d > GHOST_CAP {
                    continue;
                }
                let (px, py) = view.dot(x, y);
                self.dots.plot(px, py, if d < GHOST_NEAR { near } else { far });
            }
        }
        let body = Ink::new(Role::Star, ink::level(BODY_LEVEL * fade));
        for i in 0..3 {
            let (x, y) = self.main.cur.body_at(i, now);
            let (px, py) = view.dot(x, y);
            self.dots.disc(px, py, BODY_R, body);
        }
    }
}

impl Generator for ThreeBody {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        // Physics and trails live in model units: only the view changes.
        self.size = size;
        self.view = view_for(size, bbox(self.phase.seg));
        self.dots.resize(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        self.t += dt;
        let phase = self.phase_at(self.t);
        let restart = phase.seg != self.phase.seg || phase.tau < self.phase.tau;
        self.phase = phase;
        if restart {
            self.start_run(phase.seg);
        }
        self.advance_to(phase.tau * self.orbits[phase.seg].rate);
        self.dirty = true;
    }

    /// GEN-R20: rasterise once per drawn frame, only after the state changed.
    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.orbits[self.phase.seg].caption)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The system integrated to exactly model time `t`: log-H steps to the last one
    /// before `t`, then RK4 over the remainder (less than one step).
    fn at_time(sys: Sys, h: f64, t: f64) -> Sys {
        let mut r = Runner::new(sys, h);
        r.advance(t, |_| {});
        let mut y = [0.0; 12];
        y[..6].copy_from_slice(&r.cur.pos);
        y[6..].copy_from_slice(&r.cur.vel);
        let rem = t - r.cur.t;
        let n = 40;
        for _ in 0..n {
            sciart_core::ode::rk4_step(&mut y, 0.0, rem / f64::from(n), |_, y| {
                let mut p = [0.0; 6];
                p.copy_from_slice(&y[..6]);
                let mut a = [0.0; 6];
                accel3(&p, &mut a);
                let mut d = [0.0; 12];
                d[..6].copy_from_slice(&y[6..]);
                d[6..].copy_from_slice(&a);
                d
            });
        }
        let mut out = r.cur;
        out.pos.copy_from_slice(&y[..6]);
        out.vel.copy_from_slice(&y[6..]);
        out.t = t;
        out
    }

    fn max_diff(a: &[f64; 6], b: &[f64; 6]) -> f64 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn tb_a1_a3_a8_orbits_return_after_their_period() {
        for (k, o) in orbits().iter().enumerate() {
            if !matches!(k, 0 | 1 | 2 | 3 | 5) {
                continue;
            }
            let end = at_time(Sys::new(o.pos, o.vel), H_MAIN, o.period);
            let err = max_diff(&end.pos, &o.pos);
            assert!(err < 1e-4, "{}: return error {err:.2e}", o.caption);
            // The published period, rounded to 4–5 digits, is within 2e-3 of a return
            // (the dragonfly's within 1.6e-3: its T is 1.3e-3 short).
            let pub_err = max_diff(&at_time(Sys::new(o.pos, o.vel), H_MAIN, o.published).pos, &o.pos);
            assert!(pub_err < 2e-3, "{}: published-T return {pub_err:.2e}", o.caption);
            assert!((o.period - o.published).abs() < 2e-3);
        }
        // The figure-8 and Lagrange close too.
        for k in [4, 6] {
            let o = orbits()[k];
            let end = at_time(Sys::new(o.pos, o.vel), H_MAIN, o.period);
            assert!(max_diff(&end.pos, &o.pos) < 1e-5, "{}", o.caption);
        }
    }

    #[test]
    fn tb_a2_a9_energy_momentum_and_angular_momentum_are_conserved() {
        for o in orbits() {
            let sys = Sys::new(o.pos, o.vel);
            let e0 = energy(&o.pos, &o.vel);
            let mut worst: f64 = 0.0;
            let (mut p_err, mut l_err): (f64, f64) = (0.0, 0.0);
            Runner::new(sys, H_MAIN).advance(o.period, |s| {
                worst = worst.max(((energy(&s.pos, &s.vel) - e0) / e0).abs());
                let px: f64 = (0..3).map(|i| s.vel[2 * i]).sum();
                let py: f64 = (0..3).map(|i| s.vel[2 * i + 1]).sum();
                let l: f64 = (0..3)
                    .map(|i| s.pos[2 * i] * s.vel[2 * i + 1] - s.pos[2 * i + 1] * s.vel[2 * i])
                    .sum();
                let l0: f64 = (0..3)
                    .map(|i| o.pos[2 * i] * o.vel[2 * i + 1] - o.pos[2 * i + 1] * o.vel[2 * i])
                    .sum();
                p_err = p_err.max(px.hypot(py));
                l_err = l_err.max((l - l0).abs());
            });
            assert!(worst < 1e-6, "{}: max |ΔE/E| {worst:.2e}", o.caption);
            assert!(p_err < 1e-12 && l_err < 1e-11, "{}: P {p_err:.1e} L {l_err:.1e}", o.caption);
        }
    }

    /// The shadow cloud's spread (largest distance of a copy's body from the main body)
    /// after `periods` periods of orbit `k`.
    fn spread(k: usize, periods: f64) -> f64 {
        let o = orbits()[k];
        let t = periods * o.period;
        let main = at_time(Sys::new(o.pos, o.vel), H_MAIN, t);
        ghost_ics(&o, k)
            .iter()
            .map(|g| max_diff(&at_time(*g, H_MAIN, t).pos, &main.pos))
            .fold(0.0, f64::max)
    }

    #[test]
    fn tb_a10_shadow_clouds_smear_on_unstable_orbits_only() {
        // The copies start within 1e-5, on the orbit's energy shell, with its COM.
        let o = orbits()[2];
        let e0 = energy(&o.pos, &o.vel);
        for g in ghost_ics(&o, 2) {
            assert!(max_diff(&g.pos, &o.pos) <= 2.0 * GHOST_EPS);
            assert!(((energy(&g.pos, &g.vel) - e0) / e0).abs() < 1e-12);
        }
        // Goggles (the most unstable in the cycle) against the KAM-stable figure-8.
        let (fig8, goggles) = (spread(4, 1.0), spread(2, 1.0));
        assert!(goggles > fig8, "1 T: goggles {goggles:.2e} vs figure-8 {fig8:.2e}");
        let (fig8, goggles) = (spread(4, 2.0), spread(2, 2.0));
        assert!(goggles > 10.0 * fig8, "2 T: goggles {goggles:.2e} vs figure-8 {fig8:.2e}");
        assert!(fig8 < 0.01, "the figure-8 cloud stays a dot ({fig8:.2e})");
    }

    #[test]
    fn tb_a11_trails_are_inked_by_speed() {
        let rank = |r: Role| match r {
            Role::Accent => 0,
            Role::Hot => 1,
            Role::Sun => 2,
            _ => panic!("{r:?}"),
        };
        let mut last = 0;
        for k in 0..400 {
            let v = 0.01 * f64::from(k);
            let r = rank(speed_role(v, 1.0));
            assert!(r >= last, "v {v}: slower segments never get a hotter role");
            last = r;
        }
        assert_eq!(speed_role(0.1, 1.0), Role::Accent);
        assert_eq!(speed_role(3.0, 1.0), Role::Sun);
        // In a frame, trail cells use only the three speed roles, and there is a fast part.
        let size = Size::new(200, 50);
        let mut stage = Stage::new(build(), 42, size, GlyphMode::Unicode);
        for _ in 0..(8.0 / SIM_DT) as usize {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let roles: Vec<Role> = frame.cells()[..200 * 49]
            .iter()
            .filter(|c| c.ch != ' ')
            .map(|c| c.ink.role)
            .collect();
        assert!(roles.iter().all(|r| matches!(r, Role::Accent | Role::Hot | Role::Sun | Role::Cold | Role::Star)));
        assert!(roles.contains(&Role::Sun) && roles.contains(&Role::Hot));
    }

    #[test]
    fn tb_a12_the_cycle_holds_seven_distinct_orbits() {
        let all = orbits();
        assert!(all.len() >= 6);
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.caption, b.caption);
                assert!(max_diff(&a.vel, &b.vel) > 1e-3);
            }
            assert!(META.captions.contains(&a.caption));
        }
        // The scene visits each in turn, and its caption names it.
        let mut g = ThreeBody::new();
        let mut ctx = Ctx::new(1, Size::new(80, 24));
        g.resize(Size::new(80, 24), &mut ctx);
        let mut seen = Vec::new();
        for _ in 0..(130.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
            let c = g.caption().unwrap();
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
        assert_eq!(seen, CAPTIONS.to_vec());
    }

    #[test]
    fn tb_a4_lagrange_stays_equilateral_while_shown() {
        let o = orbits()[6];
        let mut worst: f64 = 0.0;
        Runner::new(Sys::new(o.pos, o.vel), H_MAIN).advance(o.periods * o.period, |s| {
            let d = |i: usize, j: usize| {
                (s.pos[2 * i] - s.pos[2 * j]).hypot(s.pos[2 * i + 1] - s.pos[2 * j + 1])
            };
            let sides = [d(0, 1), d(1, 2), d(2, 0)];
            let (lo, hi) = (sides.iter().copied().fold(f64::MAX, f64::min), sides.iter().copied().fold(0.0, f64::max));
            worst = worst.max(hi / lo - 1.0);
        });
        assert!(worst < 1e-5, "side deviation {worst:.2e}");
        let (_, _, t) = lagrange_ics(0.3);
        let a: f64 = 1.0 / 1.3;
        assert!((t - std::f64::consts::TAU * (a.powi(3) * 3f64.sqrt()).sqrt()).abs() < 1e-12);
    }

    #[test]
    fn tb_a5_timeline_loops_and_frames_do_not_depend_on_the_frame_rate() {
        let size = Size::new(80, 24);
        let run = |dt: f64, secs: usize| {
            let mut g = ThreeBody::new();
            let mut ctx = Ctx::new(1, size);
            g.resize(size, &mut ctx);
            let mut out = Vec::new();
            let per = (1.0 / dt).round() as usize;
            for _ in 0..secs {
                for _ in 0..per {
                    g.update(dt, &mut ctx);
                }
                out.push([0, 1, 2].map(|i| g.main.cur.body_at(i, g.target)));
            }
            out
        };
        let (a, b) = (run(SIM_DT, 40), run(0.25, 40));
        for (s, (pa, pb)) in a.iter().zip(&b).enumerate() {
            for i in 0..3 {
                let d = (pa[i].0 - pb[i].0).hypot(pa[i].1 - pb[i].1);
                assert!(d < 1e-6, "second {s}, body {i}: {d:.2e}");
            }
        }
        // The loop: the total is Σ nT/k + 7·FADE, and a run restarts from its ICs.
        let g = ThreeBody::new();
        let total: f64 = g.orbits.iter().map(|o| o.run_secs() + FADE).sum();
        assert!((g.timeline.total() - total).abs() < 1e-9);
        assert_eq!(g.phase_at(total + 0.01).seg, 0);
    }

    #[test]
    fn tb_a6_orbits_stay_on_canvas() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            for (k, b) in tables().iter().map(|t| &t.bbox).enumerate() {
                let v = view_for(size, b);
                for (x, y) in [(b[0], b[2]), (b[1], b[3])] {
                    let (cx, cy) = v.cell(x, y);
                    assert!(cx >= 1.0 && cx <= f64::from(size.width()) - 1.0, "orbit {k}");
                    assert!(cy >= 1.0 && cy <= f64::from(size.height()) - 1.0, "orbit {k}");
                }
            }
        }
    }

    #[test]
    fn tb_a7_every_run_fades_in() {
        let p = |tau| Phase { seg: 0, tau, fade: 1.0 };
        assert_eq!(orbit_fade(&p(0.0)), 0.0);
        assert!(orbit_fade(&p(0.1)) < orbit_fade(&p(0.2)));
        assert_eq!(orbit_fade(&p(FADE_IN)), 1.0);
    }
}
