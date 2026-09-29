//! A starling murmuration rippling away from a stooping falcon
//! (SPEC-002 `boids_flocking`).
//!
//! Reynolds (1987) boids on the square braille dot lattice (1 dot ≈ 0.3 m at σ = 1),
//! each bird steering by its `K = 7` topological nearest neighbours within a metric
//! horizon (Ballerini et al. 2008; Hemelrijk & Hildenbrandt 2011). A falcon stoops every
//! 12 s; birds near it turn away and their neighbours copy the escape manoeuvre a couple
//! of substeps later (Hemelrijk et al. 2015), so the agitation wave outruns the flock, as
//! Potts (1984) observed in dunlin flocks when he found manoeuvres passing between
//! neighbours faster than a bird's startle reaction (his chorus-line hypothesis).
//! Stylised: the front runs ≈ 2–3× the 13 m/s mean Procaccini et al. (2011) measured in
//! starlings (range 3.7–25 m/s), so it crosses this small flock in about a second.
//! Drawn as the M11 murmuration on black (ADR-0017): two pale grey braille dots per bird,
//! a cell a little brighter the more birds it holds, so folds read dense and edges
//! sparse; the escape wave shows only in the flock's motion. The falcon is a small pale 3 × 2-dot body
//! with a faint streak.
//!
//! Integration is synchronous semi-implicit Euler at a fixed `h = 1/60 s`: every
//! acceleration comes from the old state, so the result does not depend on update order.
//! The flock and the falcon's streak are rasterised into braille `Dots` in `prepare`
//! (GEN-R20), with one foreground ink per cell.

use std::f64::consts::{FRAC_PI_4, PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::particles::{gaussian, gaussian2};
use sciart_core::prelude::*;
use sciart_core::raster::braille;
use sciart_core::timeline::ease;
use sciart_core::title;
use sciart_core::{draw, geom, ink};

pub static META: Meta = Meta {
    name: "boids_flocking",
    category: Category::Biology,
    description: "A starling murmuration rippling away from a stooping falcon",
    title: "Murmuration · each bird tracks 7",
    captions: &CAPTIONS,
};

/// Captions: the stoop beat, the escape wave (while any bird is alarmed and `WAVE_HOLD`
/// after, whatever the beat), and pursue and climb. Patrol shows the title's own
/// mechanism.
const CAPTIONS: [&str; 3] = ["a falcon stoops", "escape wave runs", "the flock closes"];
/// Seconds the wave caption stays up after the last bird calms: a wave lasts ≈ 1 s, too
/// short to read a caption that leaves with it.
const WAVE_HOLD: f64 = 1.0;

pub fn build() -> Box<dyn Generator> {
    Box::new(Boids::new())
}

/// Topological neighbours per bird (Ballerini et al. 2008: `n_c ≈ 6.5`).
const K: usize = 7;
/// Population bounds: bird arrays are allocated once at `N_MAX` (GEN-R11).
const N_MIN: usize = 24;
const N_MAX: usize = 1000;

// Model constants at scale σ = 1, in dots and seconds; lengths, speeds and
// accelerations are multiplied by σ (SPEC-002 `boids_flocking` Model).
/// Metric horizon of the neighbourhood (3.6 m).
const R: f64 = 12.0;
/// Separation radius (1.2 m) and strength.
const R_S: f64 = 4.0;
const A_S: f64 = 150.0;
/// Alignment rate (s⁻¹) and cohesion stiffness (s⁻²).
const BETA: f64 = 5.0;
const GAMMA: f64 = 1.0;
/// Cruise speed (9 m/s, a starling's) and its relaxation time.
const V0: f64 = 30.0;
const TAU_V: f64 = 0.5;
/// Roost stiffness (s⁻²), acting on the flock's centroid: the flock swings around the
/// roost every 2π/√κ ≈ 9.4 s. A polarised flock cruising at v₀ orbits the roost at
/// radius `v₀/√κ`, which must fit the 80×24 sky (96 dots tall): 45 dots here, where
/// 0.3 would give 55 (SPEC-002 `boids_flocking` Model term 5).
const KAPPA: f64 = 0.45;
/// Per-bird pull toward the flock's own centroid (s⁻²) at N = 1000, and the population
/// up to which it equals κ (see [`trap_stiffness`]).
const KAPPA_I: f64 = 0.03;
const N_TRAP: f64 = 300.0;
/// Soft-bound strength, and the margin as a fraction of the shorter side (not scaled by
/// σ). A wide, gentle cushion turns the flock back through the central part of the sky
/// instead of piling it against the frame.
const A_B: f64 = 200.0;
const MARGIN: f64 = 0.25;
/// A small sky (see [`room`]) scales σ by down to `SIGMA_SMALL`, stiffens the roost to
/// `KAPPA_SMALL` and narrows the margin to `MARGIN_SMALL`, while the roost stops
/// wandering.
const SMALL_SKY: (f64, f64) = (44.0, 60.0);
const SIGMA_SMALL: f64 = 0.7;
const KAPPA_SMALL: f64 = 2.0;
const MARGIN_SMALL: f64 = 0.2;
/// Cap on the total acceleration.
const A_MAX: f64 = 250.0;
/// Rotational diffusion of the heading (rad²/s).
const D_THETA: f64 = 0.1;
/// Speeds are clamped to `[0.5, 2]·v₀`.
const V_MIN: f64 = 0.5;
const V_MAX: f64 = 2.0;

/// The fixed substep, and the most substeps one update may take (dt ≤ 0.25 s).
const H: f64 = SIM_DT;
const MAX_SUBSTEPS: u32 = 15;

/// Gravity for the coordinated-turn bank angle (9.81 m/s²) and the bank's EMA time.
const G: f64 = 32.7;
const BANK_TAU: f64 = 0.1;

// Escape wave (Hemelrijk et al. 2015).
/// A bird this close to the falcon fires directly (and flees).
const R_FLEE: f64 = 12.0;
/// Contagion lag: a neighbour must have fired at least `LAG` substeps (33 ms) earlier.
/// The front then runs ≈ 30–45 m/s, ≈ 2–3× the measured starling mean (SPEC-002
/// Stylisation), so the wave crosses the small on-screen flock in about a second.
const LAG: u32 = 2;
/// Alarm duration in substeps (0.35 s).
const T_A: u32 = 21;
/// Lateral escape turn and flee accelerations; alarmed birds cruise at 1.3 v₀.
const A_ALARM: f64 = 160.0;
const A_FLEE: f64 = 200.0;
const ALARM_SPEED: f64 = 1.3;

/// The falcon's 12 s attack cycle (BOID-A6): the attack fills more than half of it.
const BEATS: [(&str, f64); 4] = [
    ("patrol", 5.0),
    ("stoop", 2.5),
    ("pursue", 2.0),
    ("climb", 2.5),
];
const PATROL: usize = 0;
const STOOP: usize = 1;
const PURSUE: usize = 2;
/// Patrol circles take 4 s.
const PATROL_PERIOD: f64 = 4.0;
/// Streak length in substeps (0.3 s), and its ink levels from tail to head: it tapers.
const STREAK: usize = 18;
const STREAK_LEVELS: (f64, f64) = (60.0, 140.0);

/// Inks (SPEC-002 Inks, ADR-0017): a black sky; birds pale grey starlight, brighter only
/// where a cell holds more of them; the falcon the one pale cold accent.
const BIRD_ROLE: Role = Role::Ramp(Ramp::Starlight);
const BIRD_LEVELS: (f64, f64) = (170.0, 235.0);
const FALCON: Ramp = Ramp::Ice;
const FALCON_INK: Ink = Ink::new(Role::Ramp(FALCON), 235);
/// ASCII birds are grey specks.
const BIRD_ASCII: Ink = Ink::new(Role::Structure, 150);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// The falcon glyph by heading octant, 0 = east, clockwise on screen (y down).
const FALCON_GLYPH: [Glyph; 8] = [
    Glyph::new('→', '>'),
    Glyph::new('↘', '\\'),
    Glyph::new('↓', 'v'),
    Glyph::new('↙', '/'),
    Glyph::new('←', '<'),
    Glyph::new('↖', '\\'),
    Glyph::new('↑', '^'),
    Glyph::new('↗', '/'),
];

type V2 = [f64; 2];

#[inline]
fn add(a: V2, b: V2) -> V2 {
    [a[0] + b[0], a[1] + b[1]]
}

#[inline]
fn sub(a: V2, b: V2) -> V2 {
    [a[0] - b[0], a[1] - b[1]]
}

#[inline]
fn scale(a: V2, k: f64) -> V2 {
    [a[0] * k, a[1] * k]
}

#[inline]
fn dot(a: V2, b: V2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

#[inline]
fn norm(a: V2) -> f64 {
    // Plain sqrt, not `hypot`: values are small, and libm `hypot` dominated the
    // force loop.
    dot(a, a).sqrt()
}

/// The unit vector 90° clockwise on screen (y down) from `a`.
#[inline]
fn perp(a: V2) -> V2 {
    [-a[1], a[0]]
}

/// Unit heading of a velocity; east for a (never expected) zero velocity.
#[inline]
fn heading(v: V2) -> V2 {
    let s = norm(v);
    if s > 1e-12 {
        scale(v, 1.0 / s)
    } else {
        [1.0, 0.0]
    }
}

/// Population and length scale for a terminal size (BOID-A2): `N = clamp(0.1·w·h)`, and
/// σ keeps the flock's share of the sky constant once `N` is capped.
pub(crate) fn population(size: Size) -> (usize, f64) {
    let area = f64::from(size.width()) * f64::from(size.height());
    let n = (0.1 * area).round().clamp(N_MIN as f64, N_MAX as f64) as usize;
    let sigma = (8.0 * area / (80.0 * n as f64)).sqrt().clamp(0.5, 4.0);
    (n, sigma)
}

/// How roomy a `w × sky` dot sky is: 0 when its shorter side is at most `SMALL_SKY.0`
/// dots (a 12-row terminal with its label), 1 from `SMALL_SKY.1` (16 rows) up. At a full
/// size the flock crosses the small sky in about a second and is pressed against its
/// walls, so a small sky gets a smaller, slower flock held nearer the middle.
pub(crate) fn room(w: f64, sky: f64) -> f64 {
    let (lo, hi) = SMALL_SKY;
    ((w.min(sky) - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Stiffness `κ_i` of each bird's pull toward the flock centroid (s⁻²; SPEC-002 Model term
/// 5): κ up to `N_TRAP` birds, where it holds a small flock together as one, falling
/// log-linearly in N to `KAPPA_I` at `N_MAX`. A trap as stiff as κ on every bird of a
/// large flock squeezes it into a round, evenly grainy ball; a weak one lets it stretch
/// into sheets and folds.
pub(crate) fn trap_stiffness(n: usize) -> f64 {
    let f = ((n as f64 / N_TRAP).ln() / (N_MAX as f64 / N_TRAP).ln()).clamp(0.0, 1.0);
    KAPPA * (KAPPA_I / KAPPA).powf(f)
}

/// `(d², index)` ordering for neighbour lists: nearest first, ties by index.
#[inline]
fn closer(a: (f64, u16), b: (f64, u16)) -> bool {
    a.0 < b.0 || (a.0 == b.0 && a.1 < b.1)
}

/// The ≤ `K` nearest other points within `r` of point `i`, sorted by `(d², index)`,
/// written to `out`; returns how many (BOID-A1). `hash` must hold `pos`.
///
/// The core cell list visits every point whose bucket overlaps the query disc (exact
/// even for points outside its domain, which it clamps into edge buckets). A dense core
/// holds hundreds of birds within `r`, so the disc of radius `hint` is tried first: once
/// a disc holds `K` birds, everything outside it is strictly farther, so the answer is
/// exact for any `hint`, and a good one (see [`reach`]) makes it a single small query.
pub(crate) fn k_nearest(
    hash: &SpatialHash,
    pos: &[V2],
    i: usize,
    r: f64,
    hint: f64,
    out: &mut [u16; K],
) -> usize {
    if hint < r && k_nearest_within(hash, pos, i, hint, out) == K {
        return K;
    }
    k_nearest_within(hash, pos, i, r, out)
}

/// A disc around bird `i` that surely holds `K` other birds: out to the farthest of
/// its previous `K` neighbours at their current positions (∞ without a full list).
/// Neighbours change little per substep, so this is barely wider than the answer.
///
/// The radius is padded by 1e-9 relative: `k_nearest_within` squares it again, and a
/// sqrt-then-square round trip often lands one ulp below the farthest `d²`, which would
/// drop that neighbour and send the query out to the full horizon (BOID-A1).
fn reach(pos: &[V2], i: usize, prev: &[u16]) -> f64 {
    if prev.len() < K {
        return f64::INFINITY;
    }
    let far = prev.iter().fold(0.0f64, |m, &j| {
        pos.get(usize::from(j)).map_or(f64::INFINITY, |&q| {
            let d = sub(pos[i], q);
            m.max(dot(d, d))
        })
    });
    far.sqrt() * (1.0 + 1e-9)
}

/// [`k_nearest`] over a single disc of radius `r`, keeping the best `K` in a fixed
/// insertion array (no allocation).
fn k_nearest_within(hash: &SpatialHash, pos: &[V2], i: usize, r: f64, out: &mut [u16; K]) -> usize {
    let p = pos[i];
    let r2 = r * r;
    let mut best = [(f64::INFINITY, u16::MAX); K];
    let mut n = 0;
    hash.for_each_within(p, r, |j| {
        if j == i {
            return;
        }
        let d = sub(p, pos[j]);
        let key = (dot(d, d), j as u16);
        if key.0 > r2 || (n == K && !closer(key, best[K - 1])) {
            return;
        }
        let mut slot = if n < K {
            n += 1;
            n - 1
        } else {
            K - 1
        };
        while slot > 0 && closer(key, best[slot - 1]) {
            best[slot] = best[slot - 1];
            slot -= 1;
        }
        best[slot] = key;
    });
    for (o, b) in out.iter_mut().zip(&best[..n]) {
        *o = b.1;
    }
    n
}

/// Per-bird alarm records for the escape wave (BOID-A5). Firing is decided from stored
/// fire steps only, so the outcome does not depend on the order birds are visited.
#[derive(Debug, Clone)]
pub(crate) struct Alarm {
    /// Substep at which the bird last fired.
    fire_step: Vec<u32>,
    /// Attack in which it last fired; 0 = never (attack ids start at 1).
    fire_attack: Vec<u16>,
    /// Escape-turn sign, ±1: the lateral push is `sign · perp(v̂)`.
    sign: Vec<i8>,
    /// Whether that firing was a direct sighting of the falcon rather than contagion.
    direct: Vec<bool>,
}

impl Alarm {
    fn new(n: usize) -> Self {
        Self {
            fire_step: vec![0; n],
            fire_attack: vec![0; n],
            sign: vec![1; n],
            direct: vec![false; n],
        }
    }

    /// Forget bird `i`'s alarm history (a newly cloned bird starts calm).
    fn calm(&mut self, i: usize) {
        self.fire_attack[i] = 0;
        self.direct[i] = false;
    }

    /// Substeps since bird `i` fired, while it is still alarmed.
    #[inline]
    fn age(&self, i: usize, step: u32) -> Option<u32> {
        let age = step.wrapping_sub(self.fire_step[i]);
        (self.fire_attack[i] != 0 && age < T_A).then_some(age)
    }

    /// One substep of the excitable alarm wave (BOID-A5). A bird that has not yet fired
    /// in `attack` fires if it sees the falcon (`exposure[i] = ±1`, the turn away from
    /// it), or else if a topological neighbour fired at least `LAG` substeps ago and is
    /// still alarmed, copying the sign of the lowest-index such neighbour.
    pub(crate) fn step(
        &mut self,
        nbr: &[[u16; K]],
        nbr_n: &[u8],
        exposure: &[i8],
        step: u32,
        attack: u16,
    ) {
        for i in 0..nbr.len() {
            if self.fire_attack[i] == attack {
                continue; // at most once per attack
            }
            let sign = if exposure[i] != 0 {
                Some((exposure[i], true))
            } else {
                nbr[i][..usize::from(nbr_n[i])]
                    .iter()
                    .map(|&j| usize::from(j))
                    .filter(|&j| {
                        let age = step.wrapping_sub(self.fire_step[j]);
                        self.fire_attack[j] == attack && (LAG..T_A).contains(&age)
                    })
                    .min()
                    .map(|j| (self.sign[j], false))
            };
            if let Some((sign, direct)) = sign {
                self.fire_step[i] = step;
                self.fire_attack[i] = attack;
                self.sign[i] = sign;
                self.direct[i] = direct;
            }
        }
    }
}

/// Attack ids are 1-based per falcon cycle, so 0 can mean "never fired".
fn attack_id(cycle: i64) -> u16 {
    (cycle.rem_euclid(i64::from(u16::MAX)) + 1) as u16
}

/// Rotate `from` toward `to` (radians) by at most `max`.
fn turn_toward(from: f64, to: f64, max: f64) -> f64 {
    from + geom::wrap_angle(to - from).clamp(-max, max)
}

/// Soft-bound push along one axis: `min(((m − d)/m)², 4)` inward within `m` of a wall.
fn soft_bound(x: f64, hi: f64, m: f64) -> f64 {
    let push = |d: f64| {
        if d < m {
            ((m - d) / m).powi(2).min(4.0)
        } else {
            0.0
        }
    };
    push(x) - push(hi - x)
}

/// Specular reflection at the walls `0` and `hi` (a safety net, BOID-A4).
fn reflect(x: &mut f64, v: &mut f64, hi: f64) -> bool {
    if *x < 0.0 {
        *x = (-*x).min(hi);
        *v = v.abs();
        true
    } else if *x > hi {
        *x = (2.0 * hi - *x).max(0.0);
        *v = -v.abs();
        true
    } else {
        false
    }
}

/// The falcon: a turn-limited glider on the 12 s patrol/stoop/pursue/climb cycle. It never
/// removes birds; it only frightens them.
#[derive(Debug, Clone)]
struct Falcon {
    pos: V2,
    /// Heading in radians, y down.
    angle: f64,
    /// Patrol over the upper-right corner (else upper-left).
    right: bool,
    streak: Trail<[f32; 2]>,
}

impl Falcon {
    fn new() -> Self {
        Self {
            pos: [0.0, 0.0],
            angle: 0.0,
            right: false,
            streak: Trail::new(STREAK),
        }
    }

    /// Radius of the 4 s patrol circle flown at 0.8 v₀.
    fn radius(sigma: f64) -> f64 {
        0.8 * V0 * sigma * PATROL_PERIOD / TAU
    }

    /// Centre of the patrol circle over the left or right upper corner of a `w × h` dot
    /// sky, inset so the whole circle plus a 4σ cushion clears the walls (never past the
    /// middle of a small sky).
    fn corner(right: bool, w: f64, h: f64, sigma: f64) -> V2 {
        let inset = Self::radius(sigma) + 4.0 * sigma;
        let x = (0.12 * w).max(inset).min(0.5 * w);
        [
            if right { w - x } else { x },
            (0.2 * h).max(inset).min(0.5 * h),
        ]
    }

    fn anchor(&self, w: f64, h: f64, sigma: f64) -> V2 {
        Self::corner(self.right, w, h, sigma)
    }

    /// Patrol over whichever upper corner is farther from the flock.
    fn pick_corner(&mut self, flock: V2, w: f64, h: f64, sigma: f64) {
        let left = norm(sub(flock, Self::corner(false, w, h, sigma)));
        let right = norm(sub(flock, Self::corner(true, w, h, sigma)));
        self.right = right > left;
    }

    /// One substep of flight toward `prey` (the flock centroid 0.5 s ahead) in a
    /// `w × h` dot sky.
    fn fly(&mut self, beat: &Beat, prey: V2, sigma: f64, w: f64, h: f64) {
        let v0 = V0 * sigma;
        let unit = |to: V2| heading(sub(to, self.pos));
        // Fly onto the patrol circle of radius r_c around the anchor, then round it
        // clockwise: the climb joins the circle tangentially instead of overshooting its
        // centre into the corner.
        let circle = || {
            let r_c = Self::radius(sigma);
            let to = sub(self.anchor(w, h, sigma), self.pos);
            let d = norm(to);
            if d > 1e-9 {
                let inward = scale(to, 1.0 / d);
                let err = ((d - r_c) / r_c.max(1e-9)).clamp(-1.0, 1.0);
                heading(add(perp(inward), scale(inward, err)))
            } else {
                [self.angle.cos_det(), self.angle.sin_det()]
            }
        };
        let (speed, turn, goal) = match beat.index {
            PATROL => (0.8, 2.0, circle()),
            STOOP => (
                ease::lerp(0.8, 3.0, ease::in_out_cubic(beat.progress)),
                3.0,
                unit(prey),
            ),
            PURSUE => (2.0, 1.5, unit(prey)),
            _ => (1.5, 2.0, circle()),
        };
        // The falcon heeds the walls as the birds do: within one patrol radius of a wall
        // it turns away, and it pulls up (down to patrol speed) while heading into it, so
        // a stoop that overshoots the flock does not end sliding along the frame.
        let m = Self::radius(sigma);
        let wall = [soft_bound(self.pos[0], w, m), soft_bound(self.pos[1], h, m)];
        let into = (-dot([self.angle.cos_det(), self.angle.sin_det()], wall)).max(0.0);
        let goal = add(goal, scale(wall, 3.0));
        let speed = (speed / (1.0 + 2.0 * into)).max(0.8);
        self.angle = turn_toward(self.angle, goal[1].atan2(goal[0]), turn * H);
        let step = speed * v0 * H;
        let mut v = [step * self.angle.cos_det(), step * self.angle.sin_det()];
        let mut p = add(self.pos, v);
        // Should it still reach a wall, it glances off (a specular pull-up) rather than
        // sliding along the frame.
        let hx = reflect(&mut p[0], &mut v[0], w);
        let hy = reflect(&mut p[1], &mut v[1], h);
        if hx || hy {
            self.angle = v[1].atan2(v[0]);
        }
        self.pos = p;
        self.streak.push([self.pos[0] as f32, self.pos[1] as f32]);
    }

    /// The 8-way heading glyph: `round(atan2(vy, vx) / (π/4))`.
    fn glyph(&self) -> Glyph {
        FALCON_GLYPH[((self.angle / FRAC_PI_4).round() as i64).rem_euclid(8) as usize]
    }
}

/// The murmuration scene: bird arrays of length `N_MAX`, of which the first `n` fly.
#[derive(Debug)]
pub(crate) struct Boids {
    n: usize,
    sigma: f64,
    /// Box in dots: `[0, w] × [0, h]`.
    w: f64,
    h: f64,
    /// Height of the open sky, the box minus the label row when the label is drawn: the
    /// soft bounds, the specular wall and the drawing all keep the flock in it.
    sky: f64,
    /// `room(w, sky)` and the soft-bound margin (dots): they depend on the box alone, so
    /// `resize` computes them once instead of for every bird every substep.
    room: f64,
    margin: f64,
    /// Per-bird pull toward the flock centroid (s⁻²): `KAPPA_I` scaled down with N.
    kappa_i: f64,
    /// Per-substep weight of the bank EMA.
    bank_k: f32,
    pos: Vec<V2>,
    vel: Vec<V2>,
    vel_next: Vec<V2>,
    /// Signed bank angle (radians, + = right wing down), an EMA of the coordinated-turn
    /// roll.
    bank: Vec<f32>,
    nbr: Vec<[u16; K]>,
    nbr_n: Vec<u8>,
    /// Per-substep falcon sightings for `Alarm::step`.
    exposure: Vec<i8>,
    alarm: Alarm,
    hash: SpatialHash,
    falcon: Falcon,
    /// Always true in the scene; BOID-A3 switches it off to watch order emerge.
    falcon_enabled: bool,
    timeline: Timeline,
    /// Substeps taken; scene time is `step · h`.
    step: u64,
    clock: FixedStep,
    prev_beat: usize,
    /// Scene time of the latest rasterisation with any bird alarmed (for the caption).
    wave_at: f64,
    /// Bird dots, head and tail, over the sky cells.
    birds: Dots,
    /// The falcon's tapering streak, and per sky cell the ink of its brightest dot.
    streak: Dots,
    /// The falcon's 3 × 2-dot body, drawn over birds and streak.
    hawk: Dots,
    streak_ink: Vec<Ink>,
    /// Per sky cell, row-major: its birds' ink at the latest rasterisation.
    bird_ink: Vec<Ink>,
    cells: Size,
    label: StackText<16>,
    /// `title::stable_readout_room` at this size: the label never flickers with the beat.
    readout_room: usize,
    ready: bool,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Boids {
    fn new() -> Self {
        Self {
            dirty: true,
            n: 0,
            sigma: 1.0,
            w: 1.0,
            h: 1.0,
            sky: 1.0,
            room: 1.0,
            margin: 1.0,
            kappa_i: KAPPA_I,
            bank_k: (1.0 - (-H / BANK_TAU).exp()) as f32,
            pos: vec![[0.0; 2]; N_MAX],
            vel: vec![[0.0; 2]; N_MAX],
            vel_next: vec![[0.0; 2]; N_MAX],
            bank: vec![0.0; N_MAX],
            nbr: vec![[0; K]; N_MAX],
            nbr_n: vec![0; N_MAX],
            exposure: vec![0; N_MAX],
            alarm: Alarm::new(N_MAX),
            hash: SpatialHash::default(),
            falcon: Falcon::new(),
            falcon_enabled: true,
            timeline: Timeline::new(&BEATS),
            step: 0,
            clock: FixedStep::new(H, MAX_SUBSTEPS),
            prev_beat: PATROL,
            wave_at: f64::NEG_INFINITY,
            birds: Dots::default(),
            streak: Dots::default(),
            hawk: Dots::default(),
            streak_ink: Vec::new(),
            bird_ink: Vec::new(),
            cells: Size::new(1, 1),
            label: StackText::new(),
            readout_room: 0,
            ready: false,
        }
    }

    fn time(&self) -> f64 {
        self.step as f64 * H
    }

    /// The label is drawn only on terminals of at least 32×10 cells.
    fn shows_label(size: Size) -> bool {
        size.width() >= 32 && size.height() >= 10
    }

    /// Radius of the flock's disc at its natural spacing (12 dots² per bird at σ = 1).
    fn flock_radius(&self) -> f64 {
        (12.0 * self.n as f64 / PI).sqrt() * self.sigma
    }

    /// The roost wanders on a slow Lissajous path, so the flock never settles. Each
    /// amplitude is capped by the free sky, so a flock at the roost clears the soft-bound
    /// margin instead of being flattened against a wall, and it shrinks to 0 in a small
    /// sky, where the flock should wheel about the middle.
    fn roost(&self, t: f64) -> V2 {
        let (r, m, room) = (self.flock_radius(), self.margin, self.room);
        let amp = |len: f64, frac: f64| room * (frac * len).min((0.5 * len - r - m).max(0.0));
        [
            0.5 * self.w + amp(self.w, 0.25) * (TAU * t / 23.0).sin_det(),
            0.5 * self.sky + amp(self.sky, 0.2) * (TAU * t / 37.0).sin_det(),
        ]
    }

    fn centroid(&self) -> (V2, V2) {
        let n = self.n.max(1) as f64;
        let (p, v) = self.pos[..self.n]
            .iter()
            .zip(&self.vel[..self.n])
            .fold(([0.0; 2], [0.0; 2]), |(p, v), (&x, &u)| {
                (add(p, x), add(v, u))
            });
        (scale(p, 1.0 / n), scale(v, 1.0 / n))
    }

    /// Scatter the flock over a disc at the roost with random headings at v₀, so each
    /// visit opens with order emerging from chaos.
    fn scatter(&mut self, rng: &mut impl Rng) {
        let radius = self.flock_radius().min(0.3 * self.w.min(self.sky));
        let c = self.roost(self.time());
        for i in 0..self.n {
            let r = radius * rng.random::<f64>().sqrt();
            let (s, co) = (TAU * rng.random::<f64>()).sin_cos_det();
            let (vs, vc) = (TAU * rng.random::<f64>()).sin_cos_det();
            self.pos[i] = [
                (c[0] + r * co).clamp(0.0, self.w),
                (c[1] + r * s).clamp(0.0, self.sky),
            ];
            self.vel[i] = scale([vc, vs], V0 * self.sigma);
            self.bank[i] = 0.0;
        }
    }

    /// Carry the flock over to a new box (BOID-A2): scale positions and velocities, keep
    /// the first `n'` birds, and clone extra ones from bird `j mod n` with a bounded
    /// jitter of `r_s·σ'` so they land beside their source and start calm.
    fn rescale(&mut self, n: usize, sigma: f64, w: f64, h: f64, rng: &mut impl Rng) {
        let (kx, ky, kv) = (w / self.w, h / self.h, sigma / self.sigma);
        for i in 0..self.n {
            self.pos[i] = [self.pos[i][0] * kx, self.pos[i][1] * ky];
            self.vel[i] = scale(self.vel[i], kv);
        }
        let jitter = R_S * sigma;
        for i in self.n..n {
            let j = i % self.n;
            let (dx, dy) = gaussian2(rng, jitter);
            // Truncate the tail so a clone never lands far from its source.
            let d = scale([dx, dy], (2.5 * jitter / dx.hypot(dy).max(1e-12)).min(1.0));
            self.pos[i] = [
                (self.pos[j][0] + d[0]).clamp(0.0, w),
                (self.pos[j][1] + d[1]).clamp(0.0, h),
            ];
            self.vel[i] = self.vel[j];
            self.bank[i] = 0.0;
            self.alarm.calm(i);
        }
        self.falcon.pos = [self.falcon.pos[0] * kx, self.falcon.pos[1] * ky];
        self.falcon.streak.clear();
    }

    /// Acceleration of bird `i` from the old state (SPEC-002 Model, terms 1–5, plus the
    /// alarm turn, the flee and the soft bounds), capped at `A_MAX·σ`, and the bird's
    /// heading. `pull = κ(x_roost − x̄)` is shared by the whole flock, so the roost steers
    /// the flock without squeezing it; `centre` is the centroid `x̄`.
    fn accel(&self, i: usize, pull: V2, centre: V2, step: u32) -> (V2, V2) {
        let s = self.sigma;
        let (p, v) = (self.pos[i], self.vel[i]);
        let speed = norm(v);
        let dir = heading(v);
        let nn = usize::from(self.nbr_n[i]);
        let rs = R_S * s;
        let mut a = [0.0; 2];
        let (mut sum_v, mut sum_x) = ([0.0; 2], [0.0; 2]);
        for &j in &self.nbr[i][..nn] {
            let j = usize::from(j);
            let d = sub(p, self.pos[j]);
            let dist = norm(d);
            if dist < 1e-6 {
                // Coincident birds split along x, the lower index going −x.
                a[0] += A_S * s * if i < j { -1.0 } else { 1.0 };
            } else if dist < rs {
                a = add(a, scale(d, A_S * s * (1.0 - dist / rs) / dist));
            }
            sum_v = add(sum_v, self.vel[j]);
            sum_x = add(sum_x, self.pos[j]);
        }
        if nn > 0 {
            let inv = 1.0 / nn as f64;
            a = add(a, scale(sub(scale(sum_v, inv), v), BETA));
            a = add(a, scale(sub(scale(sum_x, inv), p), GAMMA));
        }
        let alarm = self.alarm.age(i, step);
        let cruise = if alarm.is_some() { ALARM_SPEED } else { 1.0 } * V0 * s;
        a = add(a, scale(dir, (cruise - speed) / TAU_V));
        if alarm.is_some() {
            a = add(
                a,
                scale(perp(dir), A_ALARM * s * f64::from(self.alarm.sign[i])),
            );
        }
        if self.falcon_enabled {
            // Any bird within reach of the talons veers away, which opens the vacuole
            // the flock closes behind the attack.
            let d = sub(p, self.falcon.pos);
            let dist = norm(d);
            let r = R_FLEE * s;
            if dist > 1e-9 && dist < r {
                a = add(a, scale(d, A_FLEE * s * (1.0 - dist / r) / dist));
            }
        }
        a = add(a, add(pull, scale(sub(centre, p), self.kappa_i)));
        let m = self.margin;
        let bound = [soft_bound(p[0], self.w, m), soft_bound(p[1], self.sky, m)];
        a = add(a, scale(bound, A_B * s));
        let mag = norm(a);
        let cap = A_MAX * s;
        (if mag > cap { scale(a, cap / mag) } else { a }, dir)
    }

    /// The falcon's sighting of bird `i`: the turn sign away from it, or 0 out of reach.
    fn sighting(&self, i: usize) -> i8 {
        let to = sub(self.falcon.pos, self.pos[i]);
        if norm(to) >= R_FLEE * self.sigma {
            return 0;
        }
        if dot(perp(heading(self.vel[i])), to) > 0.0 {
            -1
        } else {
            1
        }
    }

    /// One synchronous substep of the whole scene; returns how many birds the specular
    /// wall had to reflect (BOID-A4 counts these).
    fn substep(&mut self, rng: &mut impl Rng) -> usize {
        let n = self.n;
        let t = self.time();
        let step = self.step as u32;
        let beat = self.timeline.at(t);

        self.hash.rebuild(&self.pos[..n]);
        let pos = &self.pos[..n];
        for i in 0..n {
            let hint = reach(pos, i, &self.nbr[i][..usize::from(self.nbr_n[i])]);
            let found = k_nearest(&self.hash, pos, i, R * self.sigma, hint, &mut self.nbr[i]);
            self.nbr_n[i] = found as u8;
        }

        let (centre, mean_v) = self.centroid();
        if beat.index == PATROL && self.prev_beat != PATROL {
            self.falcon
                .pick_corner(centre, self.w, self.sky, self.sigma);
        }
        self.prev_beat = beat.index;
        let prey = add(centre, scale(mean_v, 0.5));
        self.falcon.fly(&beat, prey, self.sigma, self.w, self.sky);

        // Alarms fire only inside an attack's stoop and pursue windows.
        if self.falcon_enabled && matches!(beat.index, STOOP | PURSUE) {
            for i in 0..n {
                self.exposure[i] = self.sighting(i);
            }
            let attack = attack_id(self.timeline.cycle(t));
            self.alarm.step(
                &self.nbr[..n],
                &self.nbr_n[..n],
                &self.exposure[..n],
                step,
                attack,
            );
        }

        let kappa = ease::lerp(KAPPA_SMALL, KAPPA, self.room);
        let pull = scale(sub(self.roost(t), centre), kappa);
        let noise = (2.0 * D_THETA * H).sqrt();
        for i in 0..n {
            let (a, dir) = self.accel(i, pull, centre, step);
            let v = add(self.vel[i], scale(a, H));
            // Heading noise as a small-angle rotation (θ has sd ≈ 0.06 rad): dividing out
            // the `(1 + θ²)^½` stretch keeps the speed, and the heading error is O(θ³),
            // without a sin_cos per bird.
            let theta = gaussian(rng, 0.0, noise);
            let v = scale(
                add(v, scale(perp(v), theta)),
                1.0 / (1.0 + theta * theta).sqrt(),
            );
            let speed = norm(v);
            let clamped = speed.clamp(V_MIN * V0 * self.sigma, V_MAX * V0 * self.sigma);
            self.vel_next[i] = scale(heading(v), clamped);
            // Coordinated-turn roll from the lateral acceleration. The roll is signed
            // (left or right wing down) and smoothed as such, so the flickering pushes
            // of separation average out and only a sustained turn banks a bird.
            let roll = dot(a, perp(dir)).atan2(G * self.sigma) as f32;
            self.bank[i] += (roll - self.bank[i]) * self.bank_k;
        }

        let mut hits = 0;
        for i in 0..n {
            let v = &mut self.vel_next[i];
            let p = &mut self.pos[i];
            p[0] += v[0] * H;
            p[1] += v[1] * H;
            let hx = reflect(&mut p[0], &mut v[0], self.w);
            let hy = reflect(&mut p[1], &mut v[1], self.sky);
            hits += usize::from(hx || hy);
        }
        std::mem::swap(&mut self.vel, &mut self.vel_next);
        self.step += 1;
        hits
    }

    /// Stamp `wave_at` while any bird is alarmed (the caption's clock). Called wherever the
    /// state changes, so the caption does not depend on how often frames are drawn.
    fn note_alarm(&mut self) {
        let step = self.step as u32;
        if (0..self.n).any(|i| self.alarm.age(i, step).is_some()) {
            self.wave_at = self.time();
        }
    }

    /// Two braille dots per bird (head and tail), and per bird cell a pale grey that brightens a little
    /// with the birds it holds, so a fold reads denser than a sparse edge (ADR-0017: the
    /// M11 murmuration on black). The escape wave shows only as motion. The falcon is a
    /// pale 3 × 2-dot body with a faint streak behind it. Everything is clipped to the sky, so
    /// nothing reaches the label row.
    fn rasterise(&mut self) {
        self.birds.clear();
        self.streak.clear();
        self.hawk.clear();
        self.streak_ink.fill(Ink::BLANK);
        let (w, rows) = (i32::from(self.cells.width()), self.sky_rows());
        // Dots round to the nearest row, and rows from `sky` on are the label's.
        let sky = self.sky;
        let on_grid = move |x: f64, y: f64| {
            y.round() < sky && y >= 0.0 && (0.0..2.0 * f64::from(w)).contains(&x.round())
        };
        // Head and tail, as in M11: two dots give each bird its heading (at least 1.5 dots
        // apart, so they stay two on small screens).
        let tail = (1.5 * self.sigma).max(1.5);
        for (p, v) in self.pos[..self.n].iter().zip(&self.vel) {
            for q in [*p, sub(*p, scale(heading(*v), tail))] {
                if on_grid(q[0], q[1]) {
                    self.birds.plot(q[0], q[1], Ink::BLANK);
                }
            }
        }
        for cy in 0..rows {
            for cx in 0..w {
                let lit = self.birds.cell_mask(cx, cy).unwrap_or(0).count_ones();
                if lit > 0 {
                    self.bird_ink[(cy * w + cx) as usize] = bird_ink(lit);
                }
            }
        }
        if !self.falcon_enabled {
            return;
        }
        let cell = |x: i32, y: i32| {
            let cx = x.div_euclid(Dots::CELL_W).clamp(0, w - 1);
            let cy = y.div_euclid(Dots::CELL_H).clamp(0, rows - 1);
            (cy * w + cx) as usize
        };
        let last = self.falcon.streak.len().saturating_sub(1).max(1) as f64;
        let mut prev: Option<(i32, i32)> = None;
        for (k, &[x, y]) in self.falcon.streak.iter().enumerate() {
            let (lo, hi) = STREAK_LEVELS;
            let ink = Ink::new(
                Role::Ramp(FALCON),
                ink::level(ease::lerp(lo, hi, k as f64 / last)),
            );
            let q = (f64::from(x).round() as i32, f64::from(y).round() as i32);
            let (x0, y0) = prev.unwrap_or(q);
            for (dx, dy) in geom::line(x0, y0, q.0, q.1) {
                if on_grid(f64::from(dx), f64::from(dy)) {
                    self.streak.set(dx, dy, ink);
                    let slot = &mut self.streak_ink[cell(dx, dy)];
                    if ink.level >= slot.level {
                        *slot = ink;
                    }
                }
            }
            prev = Some(q);
        }
        let [fx, fy] = self.falcon.pos;
        if on_grid(fx, fy) {
            // A 3 × 2-dot body: the accent reads at a glance, still within about a cell.
            for (dx, dy) in [
                (-1.0, 0.0),
                (0.0, 0.0),
                (1.0, 0.0),
                (-1.0, 1.0),
                (0.0, 1.0),
                (1.0, 1.0),
            ] {
                if on_grid(fx + dx, fy + dy) {
                    self.hawk.plot(fx + dx, fy + dy, FALCON_INK);
                }
            }
        }
    }

    /// Cell rows of the sky (the label row excluded).
    fn sky_rows(&self) -> i32 {
        ((self.sky / f64::from(Dots::CELL_H)).ceil() as i32).max(1)
    }
}

/// A bird cell's ink by its lit dots (1 to 8): dim grey, `BIRD_LEVELS` from one bird to a
/// full cell.
fn bird_ink(lit: u32) -> Ink {
    let (lo, hi) = BIRD_LEVELS;
    let k = f64::from(lit.clamp(1, 8) - 1) / 7.0;
    Ink::new(BIRD_ROLE, ink::level(ease::lerp(lo, hi, k)))
}

impl Generator for Boids {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        if self.time() - self.wave_at < WAVE_HOLD {
            return Some(CAPTIONS[1]);
        }
        match self.timeline.at(self.time()).index {
            PATROL => None,
            STOOP => Some(CAPTIONS[0]),
            _ => Some(CAPTIONS[2]),
        }
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (
            f64::from(size.width()) * f64::from(Dots::CELL_W),
            f64::from(size.height()) * f64::from(Dots::CELL_H),
        );
        let label_row = if Self::shows_label(size) {
            f64::from(Dots::CELL_H)
        } else {
            0.0
        };
        let sky = (h - label_row).max(1.0);
        let (n, sigma) = population(size);
        // A small sky gets a smaller, slower flock (see `room`).
        let room = room(w, sky);
        let sigma = (sigma * ease::lerp(SIGMA_SMALL, 1.0, room)).clamp(0.5, 4.0);
        if self.ready {
            self.rescale(n, sigma, w, h, &mut ctx.rng);
        }
        (self.n, self.sigma, self.w, self.h, self.sky) = (n, sigma, w, h, sky);
        // The soft-bound margin is a fraction of the sky's shorter side, narrower in a
        // small sky, where a wide cushion leaves the flock no room between the walls.
        self.room = room;
        self.margin = ease::lerp(MARGIN_SMALL, MARGIN, room) * w.min(sky);
        self.kappa_i = trap_stiffness(n);
        // Scaling can carry a bird past the top of a new label row; the wall puts it back.
        for i in 0..n {
            reflect(&mut self.pos[i][1], &mut self.vel[i][1], self.sky);
        }
        self.falcon.pos[1] = self.falcon.pos[1].min(self.sky);
        if !self.ready {
            self.scatter(&mut ctx.rng);
            let (centre, _) = self.centroid();
            self.falcon.pick_corner(centre, w, self.sky, sigma);
            let anchor = self.falcon.anchor(w, self.sky, sigma);
            self.falcon.pos = add(anchor, [Falcon::radius(sigma), 0.0]);
            // Already on the circle, heading along it (clockwise on screen).
            self.falcon.angle = -PI / 2.0;
            self.ready = true;
        }
        // Buckets of R/4 fit the tight hinted queries of `k_nearest`.
        self.hash.resize(w, h, 0.25 * R * sigma);
        self.cells = size;
        let sky_cells = Size::new(size.width(), self.sky_rows() as u16);
        self.birds.resize(sky_cells);
        self.streak.resize(sky_cells);
        self.hawk.resize(sky_cells);
        for layer in [&mut self.bird_ink, &mut self.streak_ink] {
            layer.clear();
            layer.resize(sky_cells.area(), Ink::BLANK);
        }
        self.label.clear();
        let _ = write!(self.label, "N {n}  k={K}");
        self.readout_room = title::stable_readout_room(size, &META);
        self.note_alarm();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !self.ready {
            return;
        }
        let substeps = self.clock.advance(dt);
        if substeps == 0 {
            return;
        }
        let mut rng = SplitMix64::from_rng(&mut ctx.rng);
        for _ in 0..substeps {
            self.substep(&mut rng);
        }
        self.note_alarm();
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
        let unicode = canvas.mode() == GlyphMode::Unicode;
        let w = i32::from(self.cells.width());
        for cy in 0..self.sky_rows() {
            for cx in 0..w {
                let bird = self.birds.cell_mask(cx, cy).unwrap_or(0);
                let streak = self.streak.cell_mask(cx, cy).unwrap_or(0);
                let hawk = if unicode {
                    self.hawk.cell_mask(cx, cy).unwrap_or(0)
                } else {
                    0
                };
                if bird | streak | hawk == 0 {
                    continue;
                }
                let i = (cy * w + cx) as usize;
                // One ink per cell: the falcon owns its cell, birds win over the streak (it
                // breaks behind them).
                let (glyph, ink, ascii) = if hawk != 0 {
                    (braille(hawk | bird), FALCON_INK, ('.', FALCON_INK))
                } else if bird == 0 {
                    (
                        braille(streak),
                        self.streak_ink[i],
                        ('.', self.streak_ink[i]),
                    )
                } else {
                    let g = braille(bird);
                    (g, self.bird_ink[i], (g.ascii, BIRD_ASCII))
                };
                if unicode {
                    canvas.put(cx, cy, glyph, ink);
                } else {
                    canvas.put(cx, cy, Glyph::ascii(ascii.0), ascii.1);
                }
            }
        }
        if self.falcon_enabled && !unicode {
            let [x, y] = self.falcon.pos;
            let (cx, cy) = (
                ((x / f64::from(Dots::CELL_W)).floor() as i32).clamp(0, w - 1),
                ((y / f64::from(Dots::CELL_H)).floor() as i32).clamp(0, self.sky_rows() - 1),
            );
            // ASCII has no single dots: the falcon keeps its heading glyph.
            canvas.put(cx, cy, self.falcon.glyph(), FALCON_INK);
        }
        if Self::shows_label(self.cells) && self.label.len() <= self.readout_room {
            // Right of the title (GEN-R13), with one dark cell either side, so the label
            // never looks glued to a bird.
            let y = i32::from(self.cells.height()) - 1;
            draw::label_right(canvas, y, self.label.as_str(), LABEL_INK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(size: Size, seed: u64, falcon: bool) -> (Boids, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut b = Boids::new();
        b.falcon_enabled = falcon;
        b.resize(size, &mut ctx);
        b.prepare();
        (b, ctx)
    }

    /// Global polarisation `Φ = |Σ v̂| / N` (BOID-A3): 1 when every bird flies the same way.
    fn polarisation(vel: &[V2]) -> f64 {
        let sum = vel.iter().fold([0.0, 0.0], |s, &v| add(s, heading(v)));
        norm(sum) / vel.len().max(1) as f64
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    /// `⟨v̂_i · mean v̂_{N_i}⟩` over birds with neighbours (BOID-A3).
    fn local_alignment(b: &Boids) -> f64 {
        let mut sum = 0.0;
        let mut count = 0;
        for i in 0..b.n {
            let nn = usize::from(b.nbr_n[i]);
            if nn == 0 {
                continue;
            }
            let mean = b.nbr[i][..nn]
                .iter()
                .fold([0.0; 2], |s, &j| add(s, heading(b.vel[usize::from(j)])));
            // The mean of the unit headings, not renormalised (the spec's quantity).
            sum += dot(heading(b.vel[i]), scale(mean, 1.0 / nn as f64));
            count += 1;
        }
        sum / f64::from(count.max(1))
    }

    /// Milling parameter `M = |Σ r̂_i × v̂_i| / N` about the centroid (BOID-A3): 1 for a
    /// flock circling its centre, which is ordered although `Φ ≈ 0`.
    fn milling(pos: &[V2], vel: &[V2]) -> f64 {
        let n = pos.len().max(1) as f64;
        let c = scale(pos.iter().fold([0.0; 2], |s, &p| add(s, p)), 1.0 / n);
        let m: f64 = pos
            .iter()
            .zip(vel)
            .map(|(&p, &v)| {
                let (r, u) = (heading(sub(p, c)), heading(v));
                r[0] * u[1] - r[1] * u[0]
            })
            .sum();
        m.abs() / n
    }

    /// `√(λ_max / λ_min)` of the positions' covariance (BOID-A7): 1 for a round cloud.
    fn axis_ratio(pos: &[V2]) -> f64 {
        let n = pos.len().max(1) as f64;
        let c = scale(pos.iter().fold([0.0; 2], |s, &p| add(s, p)), 1.0 / n);
        let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
        for &p in pos {
            let d = sub(p, c);
            (xx, yy, xy) = (xx + d[0] * d[0], yy + d[1] * d[1], xy + d[0] * d[1]);
        }
        let (half, det) = ((xx + yy) / (2.0 * n), (xx * yy - xy * xy) / (n * n));
        let root = (half * half - det).max(0.0).sqrt();
        ((half + root) / (half - root).max(1e-12)).sqrt()
    }

    /// Median distance to the true nearest neighbour at the current positions (brute
    /// force: the stored lists date from the start of the substep).
    fn nearest_neighbour_median(b: &Boids) -> f64 {
        let pos = &b.pos[..b.n];
        let d: Vec<f64> = pos
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                pos.iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i)
                    .map(|(_, &q)| norm(sub(p, q)))
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        median(d)
    }

    #[test]
    fn boid_a1_k_nearest_matches_brute_force() {
        let (w, h, r) = (160.0, 96.0, 12.0);
        let mut rng = SplitMix64::new(7);
        for n in [1usize, 7, 8, 500] {
            let mut pos: Vec<V2> = (0..n)
                .map(|_| {
                    [
                        rng.random_range(-8.0..w + 8.0),
                        rng.random_range(-8.0..h + 8.0),
                    ]
                })
                .collect();
            // Coincident points and a dense clump stress ties and pruning.
            for i in (0..n).step_by(5).skip(1) {
                pos[i] = pos[i - 1];
            }
            for p in pos.iter_mut().take(n / 3) {
                *p = [
                    40.0 + rng.random_range(0.0..6.0),
                    30.0 + rng.random_range(0.0..6.0),
                ];
            }
            let mut hash = SpatialHash::new(w, h, r / 4.0);
            hash.rebuild(&pos);
            for i in 0..n {
                // Any hint gives the exact answer: none, too small, random, or from a
                // previous neighbour list of arbitrary birds.
                let prev: Vec<u16> = (0..K).map(|_| rng.random_range(0..n as u16)).collect();
                let hints = [
                    f64::INFINITY,
                    0.5,
                    rng.random_range(0.0..2.0 * r),
                    reach(&pos, i, &prev),
                ];
                let mut want: Vec<(f64, u16)> = (0..n)
                    .filter(|&j| j != i)
                    .map(|j| {
                        let d = sub(pos[i], pos[j]);
                        (dot(d, d), j as u16)
                    })
                    .filter(|&(d2, _)| d2 <= r * r)
                    .collect();
                want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let want: Vec<u16> = want.iter().take(K).map(|x| x.1).collect();
                for hint in hints {
                    let mut out = [0u16; K];
                    let found = k_nearest(&hash, &pos, i, r, hint, &mut out);
                    assert_eq!(&out[..found], &want[..], "n = {n}, bird {i}, hint {hint}");
                }
            }
        }
    }

    #[test]
    fn boid_a1_reach_of_the_previous_list_holds_all_k() {
        // The hint disc must contain every previous neighbour, including the farthest:
        // a sqrt-then-square round trip that lands one ulp short would drop it and send
        // the query to the full horizon.
        let (w, h, r) = (400.0, 200.0, 12.0);
        let mut rng = SplitMix64::new(13);
        let pos: Vec<V2> = (0..1000)
            .map(|_| {
                let (dx, dy) = gaussian2(&mut rng, 25.0);
                [200.0 + dx, 100.0 + dy]
            })
            .collect();
        let mut hash = SpatialHash::new(w, h, r / 4.0);
        hash.rebuild(&pos);
        let mut full = 0;
        for i in 0..pos.len() {
            let mut list = [0u16; K];
            if k_nearest(&hash, &pos, i, r, f64::INFINITY, &mut list) < K {
                continue;
            }
            full += 1;
            let hint = reach(&pos, i, &list);
            let mut out = [0u16; K];
            assert_eq!(
                k_nearest_within(&hash, &pos, i, hint, &mut out),
                K,
                "bird {i}, hint {hint}"
            );
        }
        assert!(full > 900, "{full} birds with a full list");
    }

    #[test]
    fn boid_a2_population_and_sigma_follow_formulas() {
        for ((w, h), n, sigma) in [
            ((80, 24), 192, 1.0),
            ((200, 50), 1000, 1.0),
            ((400, 100), 1000, 2.0),
            ((1, 1), 24, 0.5),
        ] {
            let (got_n, got_sigma) = population(Size::new(w, h));
            assert_eq!(got_n, n, "{w}x{h}");
            assert!((got_sigma - sigma).abs() < 1e-12, "{w}x{h}: σ {got_sigma}");
        }
        let (mut b, mut ctx) = scene(Size::new(80, 24), 3, true);
        for _ in 0..120 {
            b.update(H, &mut ctx);
            b.prepare();
            assert_eq!(b.n, 192);
        }
    }

    #[test]
    fn boid_a2_resize_keeps_prefix_and_clones_near_source() {
        let (mut b, mut ctx) = scene(Size::new(200, 50), 5, true);
        for _ in 0..60 {
            b.update(H, &mut ctx);
            b.prepare();
        }
        let before: Vec<V2> = b.pos[..192].to_vec();
        let (kx, ky) = (160.0 / 400.0, 96.0 / 200.0);
        b.resize(Size::new(80, 24), &mut ctx);
        b.prepare();
        assert_eq!(b.n, 192);
        for (p, q) in b.pos[..192].iter().zip(&before) {
            assert!((p[0] - q[0] * kx).abs() < 1e-9 && (p[1] - q[1] * ky).abs() < 1e-9);
        }

        let small: Vec<V2> = b.pos[..192].to_vec();
        b.resize(Size::new(200, 50), &mut ctx);
        b.prepare();
        assert_eq!(b.n, 1000);
        let (kx, ky) = (400.0 / 160.0, 200.0 / 96.0);
        let reach = 3.0 * R_S * b.sigma;
        for i in 192..1000 {
            let src = small[i % 192];
            let d = norm(sub(b.pos[i], [src[0] * kx, src[1] * ky]));
            assert!(d <= reach, "clone {i} is {d} dots from its source");
            assert!(b.alarm.age(i, b.step as u32).is_none());
        }
    }

    #[test]
    fn boid_a3_order_emerges_without_falcon() {
        // ~3 s in a debug build. The order parameter is max(Φ, M): a flock milling
        // around the roost is ordered although its polarisation Φ is low. Seeds are the
        // first three, not picked: a flock bent part-way into a mill (Φ ≈ 0.5, M ≈ 0.6)
        // can miss the global bound on some seeds, while local alignment holds on all.
        for seed in [0, 1, 2] {
            let (mut b, _) = scene(Size::new(100, 30), seed, false);
            assert_eq!(b.n, 300);
            assert!(polarisation(&b.vel[..b.n]) < 0.2, "seed {seed}");
            let mut rng = SplitMix64::new(seed);
            let (mut order, mut align) = (Vec::new(), Vec::new());
            for s in 1..=20 * 60 {
                b.substep(&mut rng);
                if s >= 8 * 60 && s % 60 == 0 {
                    let (pos, vel) = (&b.pos[..b.n], &b.vel[..b.n]);
                    order.push(polarisation(vel).max(milling(pos, vel)));
                    align.push(local_alignment(&b));
                }
            }
            let (order, align) = (median(order), median(align));
            assert!(align >= 0.9, "seed {seed}: median local alignment {align}");
            assert!(order >= 0.75, "seed {seed}: median order max(Φ, M) {order}");
        }
    }

    #[test]
    fn boid_a7_trap_stiffness_falls_with_population() {
        assert_eq!(trap_stiffness(24), KAPPA);
        assert_eq!(trap_stiffness(300), KAPPA);
        assert!((trap_stiffness(N_MAX) - KAPPA_I).abs() < 1e-12);
        let mut last = KAPPA;
        for n in 300..=N_MAX {
            let k = trap_stiffness(n);
            assert!(k <= last && k >= KAPPA_I - 1e-12, "n = {n}: {k}");
            last = k;
        }
    }

    #[test]
    fn boid_a7_large_flock_is_not_a_round_ball() {
        // At 200×50 (N = 1000) the flock stretches into sheets and folds instead of a
        // round, evenly grainy ball, and its ASCII frames stay a haze with dense ridges
        // rather than a solid `+`/`#` mass.
        let size = Size::new(200, 50);
        let (mut b, mut ctx) = scene(size, 42, true);
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        let (mut ratios, mut dense, mut lit) = (Vec::new(), 0, 0);
        while b.time() < 40.0 {
            b.update(H, &mut ctx);
            b.prepare();
            if b.time() >= 8.0 && b.step % 30 == 0 {
                ratios.push(axis_ratio(&b.pos[..b.n]));
                canvas.clear();
                b.render(&mut canvas);
                for c in canvas.cells() {
                    dense += usize::from(matches!(c.ch, '+' | '#'));
                    lit += usize::from(matches!(c.ch, '.' | ':' | '+' | '#'));
                }
            }
        }
        let (ratio, share) = (median(ratios), dense as f64 / lit as f64);
        assert!(ratio >= 1.5, "median principal-axis ratio {ratio}");
        assert!(share <= 0.4, "`+`/`#` share of lit ASCII cells {share}");
    }

    #[test]
    fn boid_a4_birds_stay_bounded_and_spaced() {
        // ~1.2 s in a debug build.
        let (mut b, _) = scene(Size::new(80, 24), 42, true);
        let mut rng = SplitMix64::new(42);
        let (lo, hi) = (V_MIN * V0 * b.sigma, V_MAX * V0 * b.sigma);
        let (mut hits, mut bird_steps) = (0, 0);
        let (mut clear, mut samples, mut falcon_edge) = (0, 0, 0);
        let (cw, ch) = (f64::from(Dots::CELL_W), f64::from(Dots::CELL_H));
        for s in 1..=20 * 60 {
            let h = b.substep(&mut rng);
            for i in 0..b.n {
                let (p, v) = (b.pos[i], norm(b.vel[i]));
                assert!(p[0].is_finite() && p[1].is_finite());
                // The label row is not sky: the wall sits at its top.
                assert!(b.sky < b.h);
                assert!((0.0..=b.w).contains(&p[0]) && (0.0..=b.sky).contains(&p[1]));
                assert!(v >= lo - 1e-9 && v <= hi + 1e-9, "speed {v}");
            }
            let [fx, fy] = b.falcon.pos;
            if fx.min(fy) < 1.0 || fx > b.w - 1.0 || fy > b.sky - 1.0 {
                falcon_edge += 1;
            }
            if s > 5 * 60 {
                hits += h;
                bird_steps += b.n;
                if s % 30 == 0 {
                    // The flock's 2–98 % extent keeps a cell clear of the sky's edges.
                    let q = |axis: usize, f: f64| {
                        let mut v: Vec<f64> = b.pos[..b.n].iter().map(|p| p[axis]).collect();
                        v.sort_by(f64::total_cmp);
                        v[((b.n - 1) as f64 * f) as usize]
                    };
                    let inside = q(0, 0.02) >= cw
                        && q(0, 0.98) <= b.w - cw
                        && q(1, 0.02) >= ch
                        && q(1, 0.98) <= b.sky - ch;
                    clear += usize::from(inside);
                    samples += 1;
                }
                if s % 60 == 0 {
                    let nn = nearest_neighbour_median(&b);
                    let rs = R_S * b.sigma;
                    assert!(
                        (0.35 * rs..=rs).contains(&nn),
                        "t = {} s: median nn {nn}",
                        s / 60
                    );
                }
            }
        }
        let frac = hits as f64 / bird_steps as f64;
        assert!(frac < 0.02, "specular wall on {frac} of bird-substeps");
        assert!(
            clear * 10 >= samples * 9,
            "flock clear of the edges in {clear} of {samples}"
        );
        let edge = f64::from(falcon_edge) / (20.0 * 60.0);
        assert!(
            edge < 0.03,
            "falcon on the frame edge in {edge} of substeps"
        );
    }

    #[test]
    fn boid_a4_small_sky_keeps_the_flock_off_the_walls_and_the_label_row() {
        // ~2 s in a debug build. A small sky gets a smaller, slower flock held near the
        // middle, and nothing but the label shows on its row. Over seeds 0–2, 60 s each,
        // the mean share of birds within 4 dots of a wall of the sky: at full size and σ
        // (room = 1) it is ≈ 12 % at 40×12 and ≈ 21 % at 32×10.
        assert_eq!(room(80.0, 44.0), 0.0);
        assert_eq!(room(160.0, 92.0), 1.0);
        for (size, bound) in [(Size::new(40, 12), 0.07), (Size::new(32, 10), 0.16)] {
            let (mut edge, mut samples) = (0.0, 0.0);
            for seed in 0..3 {
                let (mut b, mut ctx) = scene(size, seed, true);
                assert!((b.sigma - SIGMA_SMALL).abs() < 1e-12, "σ {}", b.sigma);
                let y = i32::from(size.height()) - 1;
                let label = format!(" {} ", b.label.as_str());
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                while b.time() < 60.0 {
                    b.update(2.0 * H, &mut ctx);
                    b.prepare();
                    if b.step % 30 != 0 {
                        continue;
                    }
                    // Birds within 4 dots of an edge of the sky (13 % at full size and σ).
                    let near = b.pos[..b.n]
                        .iter()
                        .filter(|p| p[0].min(p[1]) < 4.0 || p[0] > b.w - 4.0 || p[1] > b.sky - 4.0)
                        .count();
                    edge += near as f64 / b.n as f64;
                    samples += 1.0;
                    canvas.clear();
                    b.render(&mut canvas);
                    let row: String = (0..i32::from(size.width()))
                        .map(|x| canvas.get(x, y).map_or(' ', |c| c.ch))
                        .collect();
                    assert_eq!(
                        row.trim_start(),
                        label.trim_start(),
                        "{size:?} t = {}: label row {row:?}",
                        b.time()
                    );
                    let w = i32::from(size.width());
                    for x in [w - label.len() as i32, w - 1] {
                        assert_eq!(canvas.get(x, y).copied(), Some(Cell::BLANK), "x = {x}");
                    }
                }
            }
            let edge = edge / samples;
            assert!(edge <= bound, "{size:?}: {edge} of birds at a wall");
        }
    }

    #[test]
    fn boid_a5_alarm_is_an_excitable_wave_on_a_chain() {
        let n = 12;
        let mut alarm = Alarm::new(n);
        let mut nbr = vec![[0u16; K]; n];
        let mut nbr_n = vec![0u8; n];
        for j in 1..n {
            nbr[j][0] = (j - 1) as u16;
            nbr_n[j] = 1;
        }
        let mut exposure = vec![0i8; n];
        for step in 0..40u32 {
            // Bird 0 sees the falcon at step 0 and again (re-exposure) at step 30.
            exposure[0] = if step == 0 || step == 30 { -1 } else { 0 };
            exposure[3] = if step == 30 { 1 } else { 0 };
            alarm.step(&nbr, &nbr_n, &exposure, step, 1);
        }
        for j in 0..=10 {
            assert_eq!(alarm.fire_step[j], j as u32 * LAG, "bird {j}");
            assert_eq!(alarm.sign[j], -1, "bird {j} sign");
            assert_eq!(alarm.fire_attack[j], 1);
        }
        assert!(alarm.direct[0] && !alarm.direct[1..].iter().any(|&d| d));
        // A new attack re-arms every bird.
        exposure.fill(0);
        exposure[5] = 1;
        alarm.step(&nbr, &nbr_n, &exposure, 100, 2);
        assert_eq!((alarm.fire_step[5], alarm.fire_attack[5]), (100, 2));
        assert_eq!(alarm.fire_attack[6], 1);
    }

    #[test]
    fn boid_a5_wave_sweeps_the_flock_mostly_by_contagion() {
        let (mut b, _) = scene(Size::new(100, 30), 21, true);
        let mut rng = SplitMix64::new(21);
        while !b.alarm.fire_attack[..b.n].contains(&1) {
            b.substep(&mut rng);
            assert!(b.time() < 9.5, "no alarm during the first attack");
        }
        let first = b.step;
        let origin = b.pos[(0..b.n).find(|&i| b.alarm.fire_attack[i] == 1).unwrap()];
        // (seconds after the first alarm, distance from where it fired) of every firing.
        let mut front: Vec<(f64, f64)> = Vec::new();
        let mut seen = vec![false; b.n];
        loop {
            for (i, seen) in seen.iter_mut().enumerate() {
                if b.alarm.fire_attack[i] == 1 && !*seen {
                    *seen = true;
                    // It fired in the substep just taken.
                    let t = (b.step - first) as f64 * H;
                    front.push((t, norm(sub(b.pos[i], origin))));
                }
            }
            if b.step >= first + 60 {
                break;
            }
            b.substep(&mut rng);
        }
        // The front's speed, the least-squares slope of distance on time, beats the
        // fastest a bird can fly in the model (2 v₀): the wave outruns the flock.
        let n = front.len() as f64;
        let (mt, md) = front
            .iter()
            .fold((0.0, 0.0), |(a, c), &(t, d)| (a + t / n, c + d / n));
        let (sxy, sxx) = front.iter().fold((0.0, 0.0), |(a, c), &(t, d)| {
            (a + (t - mt) * (d - md), c + (t - mt) * (t - mt))
        });
        let speed = sxy / sxx;
        assert!(
            speed > V_MAX * V0 * b.sigma,
            "front speed {speed} dots/s vs birds ≤ {}",
            V_MAX * V0 * b.sigma
        );
        let fired: Vec<usize> = (0..b.n).filter(|&i| b.alarm.fire_attack[i] == 1).collect();
        let direct = fired.iter().filter(|&&i| b.alarm.direct[i]).count();
        assert!(
            fired.len() * 10 >= b.n * 8,
            "{} of {} fired",
            fired.len(),
            b.n
        );
        assert!(
            direct * 10 <= fired.len() * 2,
            "{direct} of {} direct",
            fired.len()
        );
    }

    #[test]
    fn boid_a6_falcon_beats_windows_and_strikes() {
        let tl = Timeline::new(&BEATS);
        for (t, beat) in [
            (0.0, 0),
            (4.99, 0),
            (5.0, 1),
            (7.49, 1),
            (7.5, 2),
            (9.49, 2),
            (9.5, 3),
            (11.99, 3),
            (12.0, 0),
            (17.0, 1),
        ] {
            assert_eq!(tl.at(t).index, beat, "t = {t}");
        }
        let (mut b, _) = scene(Size::new(100, 30), 42, true);
        let mut rng = SplitMix64::new(42);
        let mut fired_at: Vec<Vec<u32>> = vec![Vec::new(); b.n];
        let mut last = vec![0u16; b.n];
        let mut start_dist = [0.0f64; 2];
        let mut min_dist = [f64::INFINITY; 2];
        while b.time() < 30.0 {
            let t = b.time();
            b.substep(&mut rng);
            for i in 0..b.n {
                if b.alarm.fire_attack[i] != last[i] {
                    last[i] = b.alarm.fire_attack[i];
                    fired_at[i].push(b.alarm.fire_step[i]);
                }
            }
            let attack = (t / 12.0) as usize;
            let beat = tl.at(t).index;
            if attack < 2 && matches!(beat, STOOP | PURSUE) {
                let d = norm(sub(b.falcon.pos, b.centroid().0));
                if start_dist[attack] == 0.0 {
                    start_dist[attack] = d;
                }
                min_dist[attack] = min_dist[attack].min(d);
            }
        }
        for step in fired_at.iter().flatten() {
            let beat = tl.at(f64::from(*step) * H).index;
            assert!(matches!(beat, STOOP | PURSUE), "fired at step {step}");
        }
        for a in 0..2 {
            assert!(
                min_dist[a] < 0.25 * start_dist[a],
                "attack {a}: {} vs {}",
                min_dist[a],
                start_dist[a]
            );
        }
    }

    /// The scene at `size` (seed 42) after `t` simulated seconds, rendered in `mode`.
    fn frame(size: Size, t: f64, mode: GlyphMode) -> (Boids, Canvas) {
        let (mut b, mut ctx) = scene(size, 42, true);
        while b.time() < t - 1e-9 {
            b.update(H, &mut ctx);
            b.prepare();
        }
        let mut canvas = Canvas::new(size, mode);
        b.render(&mut canvas);
        (b, canvas)
    }

    #[test]
    fn boid_a9_captions_follow_the_falcon() {
        // The wave caption is up exactly while birds are alarmed and for `WAVE_HOLD` after,
        // whichever beat that falls in, so it never promises a wave that is off screen.
        let (mut b, mut ctx) = scene(Size::new(100, 30), 42, true);
        let (mut seen, mut last_alarm, mut run, mut runs) = (Vec::new(), f64::MIN, 0.0, 0);
        while b.time() < 36.0 {
            b.update(H, &mut ctx);
            b.prepare();
            let step = b.step as u32;
            if (0..b.n).any(|i| b.alarm.age(i, step).is_some()) {
                last_alarm = b.time();
            }
            let want = if b.time() - last_alarm < WAVE_HOLD {
                Some("escape wave runs")
            } else {
                match b.timeline.at(b.time()).index {
                    PATROL => None,
                    STOOP => Some("a falcon stoops"),
                    _ => Some("the flock closes"),
                }
            };
            assert_eq!(b.caption(), want, "t = {}", b.time());
            if want == Some("escape wave runs") {
                run += H;
            } else if run > 0.0 {
                // Long enough to read.
                assert!(
                    run >= 1.5,
                    "wave caption for {run:.2} s at t = {}",
                    b.time()
                );
                (run, runs) = (0.0, runs + 1);
            }
            if let Some(c) = want {
                assert!(META.captions.contains(&c));
                if !seen.contains(&c) {
                    seen.push(c);
                }
            }
        }
        assert_eq!(seen.len(), 3);
        assert!(runs >= 3, "one wave per attack, got {runs}");
        // A folding flock has dense cells: 4 or more birds, inked brighter than a lone bird.
        let (b, canvas) = frame(Size::new(200, 50), 6.5, GlyphMode::Unicode);
        let w = i32::from(b.cells.width());
        let dense = (0..b.sky_rows())
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let lit = b.birds.cell_mask(x, y).unwrap_or(0).count_ones();
                let ink = canvas.get(x, y).map_or(Ink::BLANK, |c| c.ink);
                lit >= 4 && ink.role == BIRD_ROLE && ink.level > bird_ink(1).level
            })
            .count();
        assert!(dense > 0, "no dense fold at t = 6.5 s");
    }

    #[test]
    fn boid_a8_black_sky_with_dim_grey_birds() {
        // ADR-0017: no sky, no reeds; every bird cell is pale grey on the
        // terminal's own black, brighter only where it holds more birds.
        for (w, h) in [(80, 24), (200, 50)] {
            let size = Size::new(w, h);
            for t in [5.0, 20.0] {
                let (_, canvas) = frame(size, t, GlyphMode::Unicode);
                let (mut birds, mut lit) = (0, 0);
                for cell in canvas.cells() {
                    assert_eq!(cell.bg, Ink::BLANK, "{size:?} t {t}: a lit background");
                    if cell.ink.role == BIRD_ROLE {
                        birds += 1;
                        assert!((170..=235).contains(&cell.ink.level), "{:?}", cell.ink);
                    }
                    lit += usize::from(cell.ch != ' ');
                }
                assert!(birds > 0, "{size:?} t {t}: no birds");
                // The flock is a sparse scatter: most of the frame stays black.
                assert!(
                    lit * 4 < canvas.cells().len(),
                    "{size:?} t {t}: {lit} lit cells"
                );
            }
        }
        assert_eq!(bird_ink(1).level, 170);
        assert_eq!(bird_ink(8).level, 235);
        assert!(bird_ink(4).level > bird_ink(2).level);
    }

    #[test]
    fn boid_a8_two_dots_per_bird() {
        let size = Size::new(200, 50);
        let (b, _) = frame(size, 3.0, GlyphMode::Unicode);
        let dots: u32 = (0..b.sky_rows())
            .flat_map(|cy| (0..200).map(move |cx| (cx, cy)))
            .map(|(cx, cy)| b.birds.cell_mask(cx, cy).unwrap_or(0).count_ones())
            .sum();
        // Head and tail; birds closer than a dot share one, so at most 2N dots.
        let n = b.n;
        assert!(
            dots as usize <= 2 * n && dots as usize * 10 >= n * 14,
            "{dots} dots, {n} birds"
        );
    }

    #[test]
    fn boid_a10_a11_the_wave_is_motion_and_the_falcon_a_pale_accent() {
        // Through a whole attack, only two inks appear on the sky: dim grey birds and the
        // falcon's pale Ice (its body at 235, its streak ≤ 140); the falcon is ≤ 6 dots.
        let size = Size::new(120, 40);
        let (mut b, mut ctx) = scene(size, 42, true);
        let mut seen_falcon = false;
        while b.time() < 14.0 {
            b.update(H, &mut ctx);
            b.prepare();
            let dots: u32 = (0..b.sky_rows())
                .flat_map(|cy| (0..120).map(move |cx| (cx, cy)))
                .map(|(cx, cy)| b.hawk.cell_mask(cx, cy).unwrap_or(0).count_ones())
                .sum();
            assert!(dots <= 6, "t {}: falcon of {dots} dots", b.time());
            seen_falcon |= dots >= 4;
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            b.render(&mut canvas);
            for cell in canvas.cells().iter().filter(|c| c.ch != ' ') {
                let ok = cell.ink.role == BIRD_ROLE
                    || cell.ink == FALCON_INK
                    || (cell.ink.role == Role::Ramp(FALCON) && cell.ink.level <= 140)
                    || cell.ink == LABEL_INK
                    || cell.ink.role == Role::Text;
                assert!(ok, "t {}: {:?}", b.time(), cell);
            }
        }
        assert!(seen_falcon, "the falcon never showed");
    }
}
