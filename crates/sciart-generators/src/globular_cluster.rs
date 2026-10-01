//! An ancient globular cluster of live gravitating stars, seen by a slowly turning telescope (SPEC-002 `globular_cluster`).
//!
//! Direct softened N-body (N ≤ 500) in Hénon units from a Plummer sphere, leapfrogged in
//! real time. Each body stands for ~10³ stars, so the unresolved core is drawn from the
//! Plummer surface-brightness law, sized by the bodies' own half-mass radius.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::ink::{self, MIN_VISIBLE_LEVEL};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{draw, geom, ode, rng};

pub static META: Meta = Meta {
    name: "globular_cluster",
    category: Category::DeepCosmos,
    description: "An ancient globular cluster of live gravitating stars, seen by a slowly turning telescope",
    title: "Star cluster · stars pull stars",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(GlobularCluster::new())
}

/// Plummer softening length in Hénon units, 0.085 of the scale radius `3π/16`.
const EPS: f64 = 0.05;
const EPS2: f64 = EPS * EPS;
/// N-body time units per simulated second: one force evaluation per frame at 60 Hz.
const RATE: f64 = 0.4;
/// Largest leapfrog step, N-body units (≈ t_cr/360).
const H_MAX: f64 = 1.0 / 128.0;
/// Crossing time `2√2` in Hénon units (`G = M = 1`, `E = −1/4`).
const T_CR: f64 = 2.0 * std::f64::consts::SQRT_2;
/// Mass fraction cut of the AHW sampler, so `r ≤ 38.7 a`.
const MASS_CUT: f64 = 0.999;
/// Half-mass radius over scale radius for a Plummer sphere, `1/√(2^{2/3} − 1)`.
#[cfg(test)]
const RH_PER_A: f64 = 1.3048;
/// Allowed body counts; the largest is the ADR-0012 cap.
const BUCKETS: [usize; 7] = [64, 96, 128, 192, 256, 384, 500];
const N_MAX: usize = BUCKETS[BUCKETS.len() - 1];
const MAX_GIANTS: usize = 10;
/// Camera eye distance from the origin, model units (SPEC-002 Motion).
const EYE: f64 = 10.0;
/// Re-centring passes onto the inner half's mean position (GC-A2).
const DENSITY_PASSES: usize = 4;
/// Model radius filling the frame's fit radius (≈ 93 % of the projected mass).
const VIEW_RADIUS: f64 = 2.2;
/// Giant level `GIANT_MID ± GIANT_SWING`, twinkling with a per-giant period drawn
/// from `GIANT_PERIODS` seconds.
const GIANT_MID: f64 = 245.0;
const GIANT_SWING: f64 = 10.0;
const GIANT_PERIODS: (f64, f64) = (5.0, 9.0);
/// Main-sequence display masses (M☉): the hydrogen-burning limit to the turn-off of a
/// 12 Gyr population.
const MS_MASS: (f64, f64) = (0.15, 0.8);
/// Display-mass steps: turn-off (≈ 14 % of the main sequence) and upper (≈ 29 %).
const TURNOFF_M: f64 = 0.6;
const UPPER_M: f64 = 0.35;
/// Blue stragglers are picked among bodies starting inside this radius (≈ 0.65 r_h).
const STRAGGLER_R: f64 = 0.5;
/// Trail: one stored position every `TRAIL_EVERY` updates, `TRAIL_N` kept (≈ 0.13 s).
const TRAIL_EVERY: u32 = 4;
const TRAIL_N: usize = 2;
/// Trail level as a fraction of the head's: one flat step down.
const TRAIL_FRACTION: f64 = 0.75;
/// Longest trail segment drawn (dots, Chebyshev); a longer jump (a fast near-eye star)
/// is left out rather than streaked.
const MAX_SEGMENT: i32 = 12;

/// Timeline phase indices.
const FADE: usize = 1;
const RISE: usize = 2;

/// Body count for `size`: the largest bucket `≤ max(area/5, 64)` (GC-A6).
pub(crate) fn n_for(size: Size) -> usize {
    let budget = (size.area() / 5).max(BUCKETS[0]);
    BUCKETS
        .into_iter()
        .rev()
        .find(|&n| n <= budget)
        .unwrap_or(BUCKETS[0])
}

/// Red giants among `n` bodies, `g_n = clamp(N/50, 3, 10)` (SPEC-002 Inks).
pub(crate) fn giant_count(n: usize) -> usize {
    (n / 50).clamp(3, MAX_GIANTS)
}

/// An isotropic unit vector (`z = 1 − 2u`, `φ = 2πu`).
fn unit_vec(rng: &mut impl Rng) -> [f64; 3] {
    let z = 1.0 - 2.0 * rng.random::<f64>();
    let (s, c) = (TAU * rng.random::<f64>()).sin_cos_det();
    let rho = (1.0 - z * z).max(0.0).sqrt();
    [rho * c, rho * s, z]
}

/// Append `n` raw Plummer stars in `G = M = a = 1`, per Aarseth, Hénon & Wielen (1974).
pub(crate) fn plummer_append(rng: &mut impl Rng, n: usize, pos: &mut Vec<f64>, vel: &mut Vec<f64>) {
    for _ in 0..n {
        // Inverse of the mass profile M(r) = r³/(1 + r²)^{3/2}, cut at MASS_CUT.
        let x1 = MASS_CUT * rng.random::<f64>();
        let r = (x1.powf(-2.0 / 3.0) - 1.0).max(0.0).sqrt().recip();
        // Speed fraction q from g(q) = q²(1 − q²)^{7/2} by rejection under 0.1.
        let q = loop {
            let (q, y): (f64, f64) = (rng.random(), 0.1 * rng.random::<f64>());
            if y < q * q * (1.0 - q * q).powf(3.5) {
                break q;
            }
        };
        let v = q * std::f64::consts::SQRT_2 * (1.0 + r * r).powf(-0.25);
        pos.extend(unit_vec(rng).map(|c| r * c));
        vel.extend(unit_vec(rng).map(|c| v * c));
    }
}

/// Mean of the `(x, y, z)` triples in `v` (equal masses).
fn mean3(v: &[f64]) -> [f64; 3] {
    let n = (v.len() / 3).max(1) as f64;
    v.chunks_exact(3)
        .fold([0.0; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]])
        .map(|c| c / n)
}

fn sub3(v: &mut [f64], d: [f64; 3]) {
    for p in v.chunks_exact_mut(3) {
        p[0] -= d[0];
        p[1] -= d[1];
        p[2] -= d[2];
    }
}

fn scale(v: &mut [f64], k: f64) {
    v.iter_mut().for_each(|c| *c *= k);
}

/// Pair potential `W = −Σ_{i<j} m²/√(r²_ij + ε²)`; `eps2 = 0` gives the unsoftened one.
pub(crate) fn potential(pos: &[f64], m: f64, eps2: f64) -> f64 {
    let mut w = 0.0;
    for (i, pi) in pos.chunks_exact(3).enumerate() {
        for pj in pos[3 * (i + 1)..].chunks_exact(3) {
            let (dx, dy, dz) = (pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]);
            w -= 1.0 / (dx * dx + dy * dy + dz * dz + eps2).sqrt();
        }
    }
    w * m * m
}

/// Kinetic energy `½ m Σ v²`.
pub(crate) fn kinetic(vel: &[f64], m: f64) -> f64 {
    0.5 * m * vel.iter().map(|v| v * v).sum::<f64>()
}

/// Softened accelerations `aᵢ = Σ_j m (r_j − r_i)/(r² + ε²)^{3/2}` into `acc`.
///
/// One Newton-3 pass over the `i < j` pairs, applying each pair term with opposite
/// signs, so `Σ m a = 0` to rounding (GC-A3) at half the cost of a per-star sum.
pub(crate) fn accel(pos: &[f64], acc: &mut [f64], m: f64, eps2: f64) {
    acc.fill(0.0);
    if pos.len() != acc.len() {
        return;
    }
    for i in 0..pos.len() / 3 {
        let (pi, pos_rest) = pos[3 * i..].split_at(3);
        let (ai, acc_rest) = acc[3 * i..].split_at_mut(3);
        let (mut ax, mut ay, mut az) = (0.0, 0.0, 0.0);
        for (pj, aj) in pos_rest.chunks_exact(3).zip(acc_rest.chunks_exact_mut(3)) {
            let (dx, dy, dz) = (pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]);
            let r2 = dx * dx + dy * dy + dz * dz + eps2;
            let k = m / (r2 * r2.sqrt());
            ax += dx * k;
            ay += dy * k;
            az += dz * k;
            aj[0] -= dx * k;
            aj[1] -= dy * k;
            aj[2] -= dz * k;
        }
        ai[0] += ax;
        ai[1] += ay;
        ai[2] += az;
    }
}

/// Zero the momentum, then scale velocities so `2K = |W_ε|` (virial equilibrium in the
/// softened potential). Returns the velocity factor applied.
pub(crate) fn virialise(pos: &[f64], vel: &mut [f64], m: f64) -> f64 {
    let drift = mean3(vel);
    sub3(vel, drift);
    let k = kinetic(vel, m);
    let f = if k > 0.0 {
        (potential(pos, m, EPS2).abs() / (2.0 * k)).sqrt()
    } else {
        1.0
    };
    scale(vel, f);
    f
}

/// Radius holding half the bodies, about the origin; `scratch` is reused.
pub(crate) fn half_mass_radius(pos: &[f64], scratch: &mut Vec<f64>) -> f64 {
    scratch.clear();
    scratch.extend(
        pos.chunks_exact(3)
            .map(|p| p[0] * p[0] + p[1] * p[1] + p[2] * p[2]),
    );
    if scratch.is_empty() {
        return 0.0;
    }
    let k = scratch.len() / 2;
    let (_, r2, _) = scratch.select_nth_unstable_by(k, f64::total_cmp);
    r2.sqrt()
}

/// Brightness factor of a body at camera depth `z` (larger = nearer): 1 up to
/// `z = EYE/4`, 0 from `z = EYE/3`, where perspective would magnify it ≥ 1.5× (SPEC-002
/// Inks). The eye sits inside the far halo (`r ≤ 38.7 a ≈ 23` units), so without this
/// a near-side halo star would streak across the screen like a meteor.
pub(crate) fn eye_fade(z: f64) -> f64 {
    let (near, gone) = (EYE / 4.0, EYE / 3.0);
    1.0 - ease::smoothstep((z - near) / (gone - near))
}

/// Mean position of the bodies within the half-mass radius about the origin: a cheap
/// density-centre estimate, robust to the heavy Plummer tail; `scratch` is reused.
pub(crate) fn density_centre(pos: &[f64], scratch: &mut Vec<f64>) -> [f64; 3] {
    let rh = half_mass_radius(pos, scratch);
    let (mut sum, mut n) = ([0.0; 3], 0.0);
    for p in pos.chunks_exact(3) {
        if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= rh * rh {
            sum = [sum[0] + p[0], sum[1] + p[1], sum[2] + p[2]];
            n += 1.0;
        }
    }
    if n > 0.0 { sum.map(|s| s / n) } else { sum }
}

/// A body's place on the colour–magnitude diagram (SPEC-002 Look, M16): a render-only
/// tag, the dynamics stay equal-mass. Ordered by display mass, so a heavier kind is drawn
/// at a higher level and wins a shared braille cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    /// Lower main sequence (`m < 0.35 M☉`): dim yellow.
    Lower,
    /// Upper main sequence (`0.35 ≤ m < 0.6`): yellow.
    Upper,
    /// Turn-off and subgiants (`m ≥ 0.6`): white.
    Turnoff,
    /// Blue stragglers (merged binaries, ≈ 1.2–1.5 M☉): blue, in the core.
    Straggler,
    /// Red giants: the most luminous, a 2 × 2 dot block.
    Giant,
}

impl Kind {
    /// Head role and level at full brightness; giants twinkle about theirs.
    pub(crate) fn head(self) -> (Role, f64) {
        match self {
            Kind::Lower => (Role::Sun, 150.0),
            Kind::Upper => (Role::Sun, 190.0),
            Kind::Turnoff => (Role::Star, 230.0),
            Kind::Straggler => (Role::Cold, 230.0),
            Kind::Giant => (Role::Hot, GIANT_MID),
        }
    }
}

/// Display mass (M☉) of a main-sequence body from a fixed uniform `u`: the inverse CDF
/// of `dN/dm ∝ m^{−1.3}` (Kroupa's low-mass slope) on `MS_MASS`.
pub(crate) fn ms_mass(u: f64) -> f64 {
    let (lo, hi) = (MS_MASS.0.powf(-0.3), MS_MASS.1.powf(-0.3));
    (lo + u.clamp(0.0, 1.0) * (hi - lo)).powf(-1.0 / 0.3)
}

/// Main-sequence kind of display mass `m` (brightness steps by mass).
pub(crate) fn ms_kind(m: f64) -> Kind {
    if m >= TURNOFF_M {
        Kind::Turnoff
    } else if m >= UPPER_M {
        Kind::Upper
    } else {
        Kind::Lower
    }
}

/// Blue stragglers among `n` bodies: `clamp(N/100, 2, 5)`.
pub(crate) fn straggler_count(n: usize) -> usize {
    (n / 100).clamp(2, 5)
}

/// Tag every body (GC-A8): the first `g_n` (sample order is random) are red giants, the
/// first `s_n` of the rest that start inside `STRAGGLER_R` are blue stragglers (heavy
/// merger products sink to the core), every other body a main-sequence star whose
/// display mass is a fixed hash of `(salt, index)`.
pub(crate) fn assign_kinds(pos: &[f64], salt: u64, kinds: &mut Vec<Kind>) {
    let n = pos.len() / 3;
    let (g_n, mut s_left) = (giant_count(n), straggler_count(n));
    kinds.clear();
    for (k, p) in pos.chunks_exact(3).enumerate() {
        let r2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        let kind = if k < g_n {
            Kind::Giant
        } else if s_left > 0 && r2 < STRAGGLER_R * STRAGGLER_R {
            s_left -= 1;
            Kind::Straggler
        } else {
            ms_kind(ms_mass(rng::hash01(
                salt ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            )))
        };
        kinds.push(kind);
    }
}

/// The N-body system: interleaved `(x, y, z)` per body, plus the leapfrog carry.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cluster {
    pub pos: Vec<f64>,
    pub vel: Vec<f64>,
    acc: Vec<f64>,
    /// Per-body squared radii for the half-mass radius, reused every frame.
    scratch: Vec<f64>,
    pub m: f64,
    /// Length and velocity factors that took the last AHW sample to Hénon units;
    /// growing a sample reuses them so new stars match the old (SPEC-002 Resize).
    scales: (f64, f64),
}

impl Cluster {
    fn preallocated() -> Self {
        Self {
            pos: Vec::with_capacity(3 * N_MAX),
            vel: Vec::with_capacity(3 * N_MAX),
            acc: Vec::with_capacity(3 * N_MAX),
            scratch: Vec::with_capacity(N_MAX),
            ..Self::default()
        }
    }

    pub fn n(&self) -> usize {
        self.pos.len() / 3
    }

    /// A fresh `n`-star Plummer sphere in Hénon units (GC-A2): at rest, density centre
    /// at the origin, unsoftened `W = −1/2`, then `2K = |W_ε|`, so `E ≈ −0.248`.
    pub fn plummer(&mut self, rng: &mut impl Rng, n: usize) {
        self.pos.clear();
        self.vel.clear();
        plummer_append(rng, n, &mut self.pos, &mut self.vel);
        self.m = 1.0 / n.max(1) as f64;
        let com = mean3(&self.pos);
        sub3(&mut self.pos, com);
        let len = -2.0 * potential(&self.pos, self.m, 0.0);
        scale(&mut self.pos, len);
        // The heavy halo tail puts the COM ≈ r_rms/√N (≈ 0.13 a at N = 500) off the
        // body core; the view is centred on the origin, so move the
        // density centre there (a translation: W and the dynamics are unchanged).
        for _ in 0..DENSITY_PASSES {
            let centre = density_centre(&self.pos, &mut self.scratch);
            sub3(&mut self.pos, centre);
        }
        let v = virialise(&self.pos, &mut self.vel, self.m);
        self.scales = (len, v);
        self.prime();
    }

    /// Change to `n` bodies without a visible reset (GC-A6): keep the first
    /// `min(n, N)` positions, append fresh stars at the build scales, then re-zero the
    /// momentum and re-virialise. Positions are not re-centred, which would jump the
    /// whole cluster; the COM offset of a subsample is carried by the sparse halo.
    pub fn rebucket(&mut self, rng: &mut impl Rng, n: usize) {
        let old = self.n();
        if n <= old {
            self.pos.truncate(3 * n);
            self.vel.truncate(3 * n);
        } else {
            plummer_append(rng, n - old, &mut self.pos, &mut self.vel);
            scale(&mut self.pos[3 * old..], self.scales.0);
            scale(&mut self.vel[3 * old..], self.scales.1);
        }
        self.m = 1.0 / n.max(1) as f64;
        virialise(&self.pos, &mut self.vel, self.m);
        self.prime();
    }

    /// Accelerations at the current positions (the `verlet` contract).
    fn prime(&mut self) {
        self.acc.clear();
        self.acc.resize(self.pos.len(), 0.0);
        accel(&self.pos, &mut self.acc, self.m, EPS2);
    }

    /// Advance by `dt_nbody` N-body units in leapfrog substeps `≤ H_MAX` (CORE-R44).
    pub fn advance(&mut self, dt_nbody: f64) {
        let m = self.m;
        ode::verlet_integrate(
            &mut self.pos,
            &mut self.vel,
            &mut self.acc,
            dt_nbody,
            H_MAX,
            |p, a| accel(p, a, m, EPS2),
        );
    }
}

/// Body `k`'s trail snapshot `age` slots before the newest slot `head` (see
/// `GlobularCluster::trail`).
fn snapshot(trail: &[f64], head: usize, k: usize, age: usize) -> [f64; 3] {
    let slot = (head + TRAIL_N - age % TRAIL_N) % TRAIL_N;
    let at = 3 * N_MAX * slot + 3 * k;
    [trail[at], trail[at + 1], trail[at + 2]]
}

/// A giant's level at time `t` before the fade: a slow, quiet twinkle `245 ± 10` with
/// its own `(period, phase)` (visual only).
fn giant_level((period, phase): (f64, f64), t: f64) -> f64 {
    GIANT_MID + GIANT_SWING * (TAU * t / period + phase).sin_det()
}

pub(crate) struct GlobularCluster {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    cluster: Cluster,
    timeline: Timeline,
    /// Own accumulated simulated time (GEN-R7): drives the camera and the timeline.
    t: f64,
    /// Rises passed so far: a change means "reseed now".
    epoch: i64,
    /// N-body time since the last reseed, for the label.
    age: f64,
    /// Salt of the per-body display masses and the giants' twinkle, drawn once.
    salt: u64,
    size: Size,
    /// Rows per model unit at the origin.
    scale: f64,
    dots: Dots,
    /// Per-body colour–magnitude tag (capacity `N_MAX`).
    kinds: Vec<Kind>,
    /// `TRAIL_N` snapshots of every position, slot-major with stride `3·N_MAX`, a ring
    /// whose newest slot is `trail_head`; stored in `update`, so trails do not depend on
    /// the draw cadence (GEN-R20).
    trail: Vec<f64>,
    trail_head: usize,
    /// Valid snapshots, `1..=TRAIL_N` (the first is the body's position at a reset).
    trail_len: usize,
    /// Updates since the last snapshot.
    tick: u32,
    /// Per-giant twinkle `(period s, phase rad)`, drawn from the salt.
    twinkle: [(f64, f64); MAX_GIANTS],
    label: StackText<48>,
}

impl GlobularCluster {
    fn new() -> Self {
        Self {
            dirty: false,
            cluster: Cluster::preallocated(),
            timeline: Timeline::new(&[("evolve", 120.0), ("fade", 1.5), ("rise", 1.5)]),
            t: 0.0,
            epoch: 0,
            age: 0.0,
            salt: 0,
            size: Size::new(1, 1),
            scale: 1.0,
            dots: Dots::default(),
            kinds: Vec::with_capacity(N_MAX),
            trail: vec![0.0; 3 * N_MAX * TRAIL_N],
            trail_head: 0,
            trail_len: 1,
            tick: 0,
            twinkle: [(GIANT_PERIODS.0, 0.0); MAX_GIANTS],
            label: StackText::new(),
        }
    }

    /// Rises passed by time `t`: one per completed cycle, plus the current rise.
    fn epoch_at(&self, t: f64) -> i64 {
        self.timeline.cycle(t) + i64::from(self.timeline.at(t).index == RISE)
    }

    /// Ink multiplier: fades out before the reseed and back in after it.
    fn brightness(&self) -> f64 {
        let beat = self.timeline.at(self.t);
        match beat.index {
            FADE => 1.0 - ease::smoothstep(beat.progress),
            RISE => ease::smoothstep(beat.progress),
            _ => 1.0,
        }
    }

    fn reseed(&mut self, rng: &mut impl Rng) {
        self.cluster.plummer(rng, self.cluster.n().max(BUCKETS[0]));
        self.age = 0.0;
        self.retag();
    }

    /// Re-tag the bodies and restart their trails at the current positions (a reseed or
    /// a bucket change: the trails of a new sample would jump).
    fn retag(&mut self) {
        assign_kinds(&self.cluster.pos, self.salt, &mut self.kinds);
        let len = self.cluster.pos.len();
        for slot in self.trail.chunks_exact_mut(3 * N_MAX) {
            slot[..len].copy_from_slice(&self.cluster.pos);
        }
        (self.trail_head, self.trail_len, self.tick) = (0, 1, 0);
    }

    /// Store a trail snapshot every `TRAIL_EVERY` updates.
    fn record(&mut self) {
        self.tick += 1;
        if self.tick < TRAIL_EVERY {
            return;
        }
        self.tick = 0;
        self.trail_head = (self.trail_head + 1) % TRAIL_N;
        self.trail_len = (self.trail_len + 1).min(TRAIL_N);
        let len = self.cluster.pos.len();
        let at = 3 * N_MAX * self.trail_head;
        self.trail[at..at + len].copy_from_slice(&self.cluster.pos);
    }

    /// Canvas centre in continuous cell coordinates (cell `i` spans `[i, i + 1)`).
    fn centre(&self) -> (f64, f64) {
        (
            f64::from(self.size.width()) / 2.0,
            f64::from(self.size.height()) / 2.0,
        )
    }

    /// Draw label and stars for the current state (the M10 raster convention: `render`
    /// only blits). The label goes first, so the stars can keep clear of it.
    fn rasterise(&mut self) {
        self.dots.clear();
        self.write_label();
        let b = self.brightness();
        self.bodies(b);
    }

    /// Cells kept clear for the label on row 0: column 0, the label, and one trailing
    /// cell, so no star reads as part of the text (0 without a label).
    fn label_cells(&self) -> i32 {
        if self.label.is_empty() {
            0
        } else {
            self.label.len() as i32 + 2
        }
    }

    /// Project every body and its trail through the turning camera (SPEC-002 Look,
    /// M16): a one-dot head in its kind's ink (giants a small cross), a one-dot thread
    /// one step dimmer through its stored positions. No unresolved glow: the core is
    /// dense because its stars and their trails crowd it.
    fn bodies(&mut self, b: f64) {
        let proj = self.camera().projector();
        let centre = self.centre();
        let keep_out = self.label_cells();
        let scale = self.scale;
        let dot_of = |p: [f64; 3]| {
            proj.project(p, centre, scale).map(|(x, y, z)| {
                let (dx, dy) = Dots::from_cells(x, y);
                ((dx.round() as i32, dy.round() as i32), z)
            })
        };
        let put = |dots: &mut Dots, (x, y): (i32, i32), ink: Ink| {
            if !(y < Dots::CELL_H && x < keep_out * Dots::CELL_W) {
                dots.set(x, y, ink);
            }
        };
        let (t, trail_len, head_slot) = (self.t, self.trail_len, self.trail_head);
        let (trail, kinds, twinkle) = (&self.trail, &self.kinds, &self.twinkle);
        let dots = &mut self.dots;
        for (k, p) in self.cluster.pos.chunks_exact(3).enumerate() {
            let Some((head, z)) = dot_of([p[0], p[1], p[2]]) else {
                continue;
            };
            // Near-eye halo stars (perspective ≥ 1.33×) are left out instead of streaking.
            if eye_fade(z) < 0.5 {
                continue;
            }
            let kind = kinds.get(k).copied().unwrap_or(Kind::Lower);
            let (role, mut level) = kind.head();
            if kind == Kind::Giant {
                level = giant_level(twinkle[k.min(MAX_GIANTS - 1)], t);
            }
            level *= b;
            if level < MIN_VISIBLE_LEVEL {
                continue;
            }
            let trail_ink = Ink::new(role, ink::level(level * TRAIL_FRACTION));
            let mut from = head;
            for age in 0..trail_len {
                let Some((to, _)) = dot_of(snapshot(trail, head_slot, k, age)) else {
                    break;
                };
                if (to.0 - from.0).abs().max((to.1 - from.1).abs()) > MAX_SEGMENT {
                    break;
                }
                for q in geom::line(from.0, from.1, to.0, to.1).skip(1) {
                    put(dots, q, trail_ink);
                }
                from = to;
            }
            let head_ink = Ink::new(role, ink::level(level));
            put(dots, head, head_ink);
            // A giant, the most luminous kind, is the largest mark: a 2 × 2 dot block.
            if kind == Kind::Giant {
                for (dx, dy) in [(1, 0), (0, 1), (1, 1)] {
                    put(dots, (head.0 + dx, head.1 + dy), head_ink);
                }
            }
        }
    }

    /// Giant `k`'s level at time `t` before the fade (the tests' probe).
    #[cfg(test)]
    fn giant_level(&self, k: usize, t: f64) -> f64 {
        giant_level(self.twinkle[k], t)
    }

    /// Per-giant twinkle periods and phases, a pure function of the salt (no extra
    /// draws from `ctx.rng`).
    fn build_twinkle(&mut self) {
        let (lo, hi) = GIANT_PERIODS;
        let mut rng = SplitMix64::new(self.salt);
        for slot in &mut self.twinkle {
            *slot = (lo + (hi - lo) * rng.next_f64(), TAU * rng.next_f64());
        }
    }

    /// The slowly turning, gently nodding telescope (SPEC-002 Motion).
    fn camera(&self) -> Camera {
        Camera {
            yaw: TAU * self.t / 90.0,
            pitch: 0.30 + 0.15 * (TAU * self.t / 67.0).sin_det(),
            distance: EYE,
        }
    }

    /// `globular cluster  N=500  3 crossings` (no crossings clause before the first) if
    /// it takes at most half the width, else `globular cluster` if that fits in `w − 2`
    /// columns, else nothing (SPEC-002 Label). The age counts whole crossing times
    /// since the last reseed, so it changes only every 7.1 s.
    fn write_label(&mut self) {
        self.label.clear();
        let w = usize::from(self.size.width());
        // `age ≥ 0`, and ≤ 17 crossings per evolve phase: the cast is exact.
        let crossings = (self.age / T_CR).floor() as u32;
        let n = self.cluster.n();
        let _ = write!(self.label, "N={n}");
        if crossings > 0 {
            let plural = if crossings == 1 { "" } else { "s" };
            let _ = write!(self.label, "  {crossings} crossing{plural}");
        }
        // The title already names the scene: the label is only the counts, the
        // crossings where the label takes at most half the width.
        if 2 * self.label.len() > w {
            self.label.clear();
            let _ = write!(self.label, "N={n}");
            if self.label.len() + 2 > w {
                self.label.clear();
            }
        }
    }
}

impl Generator for GlobularCluster {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let n = n_for(size);
        if self.cluster.n() == 0 {
            self.salt = ctx.rng.random();
            self.build_twinkle();
            self.cluster.plummer(&mut ctx.rng, n);
            self.retag();
        } else if n != self.cluster.n() {
            self.cluster.rebucket(&mut ctx.rng, n);
            self.retag();
        }
        self.size = size;
        self.scale = draw::fit_radius(size, 0.92) / VIEW_RADIUS;
        self.dots.resize(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        self.t += dt;
        let epoch = self.epoch_at(self.t);
        if epoch != self.epoch {
            self.epoch = epoch;
            self.reseed(&mut ctx.rng);
        } else {
            self.cluster.advance(dt * RATE);
            self.age += dt * RATE;
            self.record();
        }
        self.dirty = true;
    }

    /// GEN-R20: rasterise once per drawn frame, only after the state changed.
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        canvas.text(1, 0, self.label.as_str(), Ink::new(Role::Text, 140));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> impl Rng {
        Ctx::new(seed, Size::new(1, 1)).rng
    }

    fn momentum(c: &Cluster) -> f64 {
        let p = mean3(&c.vel).map(|v| v * c.n() as f64 * c.m);
        p[0].hypot(p[1]).hypot(p[2])
    }

    /// Kolmogorov–Smirnov distance of the sorted sample `xs` from the CDF `cdf`.
    fn ks(xs: &[f64], cdf: impl Fn(f64) -> f64) -> f64 {
        let n = xs.len() as f64;
        xs.iter()
            .enumerate()
            .map(|(i, &x)| {
                let c = cdf(x);
                (c - i as f64 / n).abs().max((c - (i + 1) as f64 / n).abs())
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn gc_a1_ahw_sampler_matches_the_truncated_plummer_profile() {
        let (mut pos, mut vel) = (Vec::new(), Vec::new());
        plummer_append(&mut rng(1), 4000, &mut pos, &mut vel);
        let mut r: Vec<f64> = pos
            .chunks_exact(3)
            .map(|p| p[0].hypot(p[1]).hypot(p[2]))
            .collect();
        r.sort_by(f64::total_cmp);
        let mass = |r: f64| r.powi(3) / (1.0 + r * r).powf(1.5) / MASS_CUT;
        let ks_r = ks(&r, mass);
        assert!(ks_r < 0.03, "KS distance {ks_r}");
        let median = r[r.len() / 2];
        assert!((median / RH_PER_A - 1.0).abs() < 0.04, "median {median}");
        // Speed fractions q = v/v_esc(r) follow g(q) ∝ q²(1 − q²)^{7/2}: KS against its
        // CDF (the regularised incomplete beta I_{q²}(3/2, 9/2)), integrated here by the
        // trapezoid rule on a fine grid.
        let mut q: Vec<f64> = pos
            .chunks_exact(3)
            .zip(vel.chunks_exact(3))
            .map(|(p, v)| {
                let r = p[0].hypot(p[1]).hypot(p[2]);
                v[0].hypot(v[1]).hypot(v[2])
                    / (std::f64::consts::SQRT_2 * (1.0 + r * r).powf(-0.25))
            })
            .collect();
        q.sort_by(f64::total_cmp);
        assert!(q[0] >= 0.0 && q[q.len() - 1] < 1.0, "unbound star");
        // KS distance of the sorted q from g(q) ∝ q²(1 − q²)^e.
        let ks_q = |e: f64| {
            const GRID: usize = 20_000;
            let step = 1.0 / GRID as f64;
            let g = |q: f64| q * q * (1.0 - q * q).powf(e);
            let mut cdf = vec![0.0; GRID + 1];
            for i in 1..=GRID {
                cdf[i] = cdf[i - 1] + 0.5 * step * (g((i - 1) as f64 * step) + g(i as f64 * step));
            }
            let big_g = |q: f64| cdf[(q * GRID as f64).round() as usize] / cdf[GRID];
            ks(&q, big_g)
        };
        assert!(ks_q(3.5) < 0.03, "KS distance of q {}", ks_q(3.5));
        // The test has teeth: a wrong exponent (5/2 or 9/2 for 7/2) fails it.
        for e in [2.5, 4.5] {
            assert!(ks_q(e) > 0.03, "exponent {e} not rejected: {}", ks_q(e));
        }

        let (mut pos, mut vel) = (Vec::new(), Vec::new());
        plummer_append(&mut rng(2), 1500, &mut pos, &mut vel);
        let m = 1.0 / 1500.0;
        let ratio = 2.0 * kinetic(&vel, m) / potential(&pos, m, 0.0).abs();
        assert!((0.9..=1.1).contains(&ratio), "raw virial ratio {ratio}");
    }

    #[test]
    fn gc_a2_plummer_ics_are_in_henon_units_and_virialised() {
        // Centred on the density centre, not the heavy-tailed COM (≈ r_rms/√N: 0.08
        // units at N = 500, 0.22 at N = 64), so the glow sits on the body core. The
        // mean-shift passes settle within a few hundredths (the inner half's own noise).
        for seed in 1..=6 {
            for n in [64, 256, 500] {
                let mut c = Cluster::preallocated();
                c.plummer(&mut rng(seed), n);
                let mut scratch = Vec::new();
                let dc = density_centre(&c.pos, &mut scratch);
                let off = dc[0].hypot(dc[1]).hypot(dc[2]);
                assert!(off < 0.03, "seed {seed}, N = {n}: density centre {off}");
                assert!(momentum(&c) < 1e-14);
            }
        }
        let mut c = Cluster::preallocated();
        c.plummer(&mut rng(3), 256);
        assert!((potential(&c.pos, c.m, 0.0) + 0.5).abs() < 1e-12);
        let (k, w) = (kinetic(&c.vel, c.m), potential(&c.pos, c.m, EPS2));
        assert!((2.0 * k + w).abs() < 1e-12);
        assert!((k + w + 0.25).abs() < 0.01, "E = {}", k + w);
    }

    /// Per-star O(N²) sum, the reference for the Newton-3 loop.
    fn accel_naive(pos: &[f64], m: f64, eps2: f64) -> Vec<f64> {
        let body = |i: usize| &pos[3 * i..3 * i + 3];
        (0..pos.len() / 3)
            .flat_map(|i| {
                let mut a = [0.0; 3];
                for j in (0..pos.len() / 3).filter(|&j| j != i) {
                    let d: Vec<f64> = (0..3).map(|c| body(j)[c] - body(i)[c]).collect();
                    let r2 = d.iter().map(|x| x * x).sum::<f64>() + eps2;
                    for c in 0..3 {
                        a[c] += m * d[c] / r2.powf(1.5);
                    }
                }
                a
            })
            .collect()
    }

    #[test]
    fn gc_a3_pair_forces_match_the_naive_sum_and_the_potential_gradient() {
        let mut c = Cluster::preallocated();
        c.plummer(&mut rng(4), 64);
        let mut acc = vec![0.0; c.pos.len()];
        accel(&c.pos, &mut acc, c.m, EPS2);
        let naive = accel_naive(&c.pos, c.m, EPS2);
        for (a, b) in acc.chunks_exact(3).zip(naive.chunks_exact(3)) {
            let err = (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]);
            assert!(
                err <= 1e-12 * b[0].hypot(b[1]).hypot(b[2]),
                "{a:?} vs {b:?}"
            );
        }
        let net = mean3(&acc).map(|x| x * 64.0 * c.m);
        assert!(
            net[0].hypot(net[1]).hypot(net[2]) < 1e-14,
            "Σ m a = {net:?}"
        );

        let h = 1e-5;
        for i in 0..c.n() {
            let grad: Vec<f64> = (0..3)
                .map(|k| {
                    let mut p = c.pos.clone();
                    p[3 * i + k] += h;
                    let up = potential(&p, c.m, EPS2);
                    p[3 * i + k] -= 2.0 * h;
                    (up - potential(&p, c.m, EPS2)) / (2.0 * h)
                })
                .collect();
            let a = &acc[3 * i..3 * i + 3];
            let err = (0..3)
                .map(|k| (a[k] + grad[k] / c.m).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(err < 1e-6 * a[0].hypot(a[1]).hypot(a[2]), "star {i}: {err}");
        }

        let pair = [0.3, -0.2, 0.1, 0.3, -0.2, 0.1];
        let mut a = [1.0; 6];
        accel(&pair, &mut a, 0.5, EPS2);
        assert_eq!(a, [0.0; 6]);
    }

    /// The N = 128 validation run: 10 N-body units at `h = 1/128`, sampled every 0.5.
    fn validation_run() -> (Vec<(f64, f64, f64)>, f64, f64) {
        let mut c = Cluster::preallocated();
        c.plummer(&mut rng(5), 128);
        let mut scratch = Vec::new();
        let e0 = kinetic(&c.vel, c.m) + potential(&c.pos, c.m, EPS2);
        let (mut max_de, mut max_p) = (0.0f64, 0.0f64);
        let mut samples = vec![(0.0, half_mass_radius(&c.pos, &mut scratch), 1.0)];
        for step in 1..=1280 {
            c.advance(H_MAX);
            let (k, w) = (kinetic(&c.vel, c.m), potential(&c.pos, c.m, EPS2));
            max_de = max_de.max(((k + w - e0) / e0).abs());
            max_p = max_p.max(momentum(&c));
            if step % 64 == 0 {
                let rh = half_mass_radius(&c.pos, &mut scratch);
                samples.push((f64::from(step) * H_MAX, rh, 2.0 * k / w.abs()));
            }
        }
        (samples, max_de, max_p)
    }

    #[test]
    fn gc_a4_gc_a5_leapfrog_conserves_energy_and_the_cluster_stays_in_equilibrium() {
        let (samples, max_de, max_p) = validation_run();
        assert!(max_p < 1e-14, "|Σ m v| = {max_p}");
        assert!(max_de < 5e-4, "max |dE/E| = {max_de}");
        assert_eq!(samples.len(), 21);
        let rh0 = samples[0].1;
        for &(t, rh, _) in &samples {
            assert!(
                (0.7..=1.3).contains(&(rh / rh0)),
                "r_h ratio {} at t = {t}",
                rh / rh0
            );
        }
        let mean = |f: fn(&(f64, f64, f64)) -> f64| samples.iter().map(f).sum::<f64>() / 21.0;
        let rh_mean = mean(|s| s.1) / rh0;
        assert!((0.9..=1.1).contains(&rh_mean), "mean r_h ratio {rh_mean}");
        let virial = samples[1..].iter().map(|s| s.2).sum::<f64>() / 20.0;
        assert!((0.9..=1.1).contains(&virial), "mean 2K/|W| {virial}");
    }

    #[test]
    fn gc_a6_body_count_follows_buckets_and_resizes_never_reset_the_cluster() {
        assert_eq!(n_for(Size::new(80, 24)), 384);
        assert_eq!(n_for(Size::new(200, 50)), 500);
        assert_eq!(n_for(Size::new(1, 1)), 64);
        assert_eq!(n_for(Size::new(0, 0)), 64);
        assert_eq!(n_for(Size::new(1000, 1000)), 500);

        let mut ctx = Ctx::new(6, Size::new(80, 24));
        let mut g = GlobularCluster::new();
        g.resize(Size::new(80, 24), &mut ctx);
        let before = g.cluster.clone();
        g.resize(Size::new(81, 24), &mut ctx);
        assert_eq!(g.cluster.pos, before.pos);
        assert_eq!(g.cluster.vel, before.vel);

        for (size, n) in [(Size::new(200, 50), 500), (Size::new(40, 12), 96)] {
            let old = g.cluster.pos.clone();
            g.resize(size, &mut ctx);
            let c = &g.cluster;
            assert_eq!(c.n(), n);
            let keep = 3 * n.min(old.len() / 3);
            assert_eq!(c.pos[..keep], old[..keep]);
            assert!(momentum(c) < 1e-14);
            let (k, w) = (kinetic(&c.vel, c.m), potential(&c.pos, c.m, EPS2));
            assert!((2.0 * k + w).abs() < 1e-12);
        }
    }

    #[test]
    fn gc_a6_reseed_reuses_the_preallocated_buffers() {
        // The reseed runs only at t = 121.5 s, beyond the alloc_free suite's reach: its
        // buffers must keep their build capacity (GEN-R11).
        let (mut g, mut ctx) = scene_at(Size::new(200, 50), 0.0);
        let caps = |c: &Cluster| {
            [
                c.pos.capacity(),
                c.vel.capacity(),
                c.acc.capacity(),
                c.scratch.capacity(),
            ]
        };
        let before = caps(&g.cluster);
        let old = g.cluster.pos.clone();
        g.t = 121.49;
        g.update(SIM_DT, &mut ctx);
        g.prepare();
        assert_eq!(g.epoch, 1, "no reseed");
        assert_ne!(g.cluster.pos, old);
        assert_eq!(caps(&g.cluster), before);
        assert_eq!(g.cluster.n(), 500);
    }

    #[test]
    fn gc_a7_stars_are_coloured_dots_with_a_dense_core_and_no_fog() {
        let size = Size::new(200, 50);
        let (g, _) = scene_at(size, 3.0);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        // Every lit cell is a braille star cell in one of the kinds' inks: a head level
        // or the trail step below it (giants twinkle, so their levels are a band).
        let heads: Vec<(Role, u8)> = [Kind::Lower, Kind::Upper, Kind::Turnoff, Kind::Straggler]
            .map(|k| (k.head().0, ink::level(k.head().1)))
            .to_vec();
        let mut hot = 0;
        for row in 1..50 {
            for col in 0..200 {
                let cell = canvas.get(col, row).unwrap();
                if cell.ch == ' ' {
                    continue;
                }
                assert!(('⠀'..='⣿').contains(&cell.ch), "{col},{row}: {:?}", cell.ch);
                let (role, level) = (cell.ink.role, cell.ink.level);
                let trail = |h: u8| ink::level(f64::from(h) * TRAIL_FRACTION);
                let ok = match role {
                    Role::Hot => {
                        hot += 1;
                        let lo = (GIANT_MID - GIANT_SWING) * TRAIL_FRACTION;
                        f64::from(level) >= lo.floor()
                    }
                    _ => heads
                        .iter()
                        .any(|&(r, h)| r == role && (level == h || level == trail(h))),
                };
                assert!(ok, "{col},{row}: {role:?} {level}");
            }
        }
        assert!(hot > 0, "no giant drawn");
        // Dense core, sparse halo: lit-dot density inside 0.4 model units of the
        // centre (projected) is ≥ 6× that between 1.2 and 1.8 units; no fog means the
        // core is not filled solid either (< 60 % of its dots).
        let (cx, cy) = Dots::from_cells(100.0, 25.0);
        let per_unit = 4.0 * g.scale;
        let (mut core, mut core_n, mut ring, mut ring_n) = (0.0, 0.0, 0.0, 0.0);
        let dots = g.dots.dot_size();
        for py in 0..i32::from(dots.height()) {
            for px in 0..i32::from(dots.width()) {
                let r = (f64::from(px) - cx).hypot(f64::from(py) - cy) / per_unit;
                let lit = f64::from(u8::from(g.dots.is_lit(px, py)));
                if r < 0.4 {
                    (core, core_n) = (core + lit, core_n + 1.0);
                } else if (1.2..1.8).contains(&r) {
                    (ring, ring_n) = (ring + lit, ring_n + 1.0);
                }
            }
        }
        let (core, ring) = (core / core_n, ring / ring_n);
        assert!(core >= 6.0 * ring, "core {core:.3} vs halo {ring:.3}");
        assert!(core < 0.6, "core filled solid: {core:.3}");
        // Near-eye halo stars (perspective ≥ 1.33×) are left out instead of streaking.
        for z in [-20.0, 0.0, 2.5] {
            assert_eq!(eye_fade(z), 1.0);
        }
        for z in [EYE / 3.0, 5.0, 9.9] {
            assert_eq!(eye_fade(z), 0.0);
        }
        assert!(eye_fade(2.7) > eye_fade(2.9) && eye_fade(2.9) > eye_fade(3.2));
    }

    #[test]
    fn gc_a8_kinds_follow_mass_and_stragglers_start_in_the_core() {
        assert_eq!(
            (giant_count(500), giant_count(384), giant_count(64)),
            (10, 7, 3)
        );
        assert_eq!(
            (
                straggler_count(500),
                straggler_count(384),
                straggler_count(64)
            ),
            (5, 3, 2)
        );
        // Display masses span MS_MASS, rising with u.
        assert!((ms_mass(0.0) - 0.15).abs() < 1e-12 && (ms_mass(1.0) - 0.8).abs() < 1e-12);
        let us: Vec<f64> = (0..=100).map(|i| f64::from(i) / 100.0).collect();
        assert!(us.windows(2).all(|w| ms_mass(w[1]) > ms_mass(w[0])));
        // Brightness steps by mass: heavier kinds are drawn brighter, and a trail is one
        // step below its head.
        let order = [Kind::Lower, Kind::Upper, Kind::Turnoff];
        assert!(order.windows(2).all(|w| w[1].head().1 > w[0].head().1));
        assert!(
            order
                .iter()
                .all(|k| matches!(k.head().0, Role::Sun | Role::Star))
        );
        assert_eq!(Kind::Straggler.head().0, Role::Cold);
        assert_eq!(Kind::Giant.head().0, Role::Hot);
        for k in order {
            assert!(k.head().1 * TRAIL_FRACTION < k.head().1 - 20.0);
        }

        let mut c = Cluster::preallocated();
        c.plummer(&mut rng(8), 500);
        let mut kinds = Vec::new();
        assign_kinds(&c.pos, 99, &mut kinds);
        assert_eq!(kinds.len(), 500);
        let count = |k: Kind| kinds.iter().filter(|&&x| x == k).count();
        assert_eq!(count(Kind::Giant), 10);
        assert!(kinds[..10].iter().all(|&k| k == Kind::Giant));
        assert_eq!(count(Kind::Straggler), 5);
        for (k, p) in c.pos.chunks_exact(3).enumerate() {
            if kinds[k] == Kind::Straggler {
                assert!(p[0].hypot(p[1]).hypot(p[2]) < STRAGGLER_R);
            }
        }
        // Main-sequence shares ≈ 14 / 29 / 57 % (the m^−1.3 law's steps).
        let ms = 485.0;
        for (kind, share) in [
            (Kind::Turnoff, 0.14),
            (Kind::Upper, 0.29),
            (Kind::Lower, 0.57),
        ] {
            let got = count(kind) as f64 / ms;
            assert!((got - share).abs() < 0.07, "{kind:?}: {got:.3}");
        }
        // Tags are a pure function of the positions and the salt.
        let mut again = Vec::new();
        assign_kinds(&c.pos, 99, &mut again);
        assert_eq!(kinds, again);
        assign_kinds(&c.pos, 100, &mut again);
        assert_ne!(kinds, again);
    }

    #[test]
    fn gc_a11_giants_are_the_most_luminous_and_largest_marks() {
        // The faintest giant twinkle is at least every other kind's head level.
        let lo = GIANT_MID - GIANT_SWING;
        for k in [Kind::Lower, Kind::Upper, Kind::Turnoff, Kind::Straggler] {
            assert!(lo >= k.head().1, "{k:?}");
        }
        // In the frame the mean level of the giants' cells is at least that of every
        // other kind's cells (their trails included), and each giant is a 2 × 2 block.
        let size = Size::new(200, 50);
        let (g, _) = scene_at(size, 3.0);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut sums: [(f64, f64); 4] = [(0.0, 0.0); 4];
        for y in 1..50 {
            for x in 0..200 {
                let c = canvas.get(x, y).unwrap();
                let k = match c.ink.role {
                    Role::Hot => 0,
                    Role::Star => 1,
                    Role::Sun => 2,
                    Role::Cold => 3,
                    _ => continue,
                };
                sums[k].0 += f64::from(c.ink.level);
                sums[k].1 += 1.0;
            }
        }
        let mean = |k: usize| sums[k].0 / sums[k].1.max(1.0);
        assert!(sums[0].1 > 0.0, "no giant");
        for k in 1..4 {
            assert!(
                mean(0) >= mean(k),
                "giants {} < kind {k} {}",
                mean(0),
                mean(k)
            );
        }
        let proj = g.camera().projector();
        for (k, p) in g
            .cluster
            .pos
            .chunks_exact(3)
            .enumerate()
            .take(giant_count(500))
        {
            let Some((x, y, z)) = proj.project([p[0], p[1], p[2]], g.centre(), g.scale) else {
                continue;
            };
            let (dx, dy) = Dots::from_cells(x, y);
            let (px, py) = (dx.round() as i32, dy.round() as i32);
            if eye_fade(z) < 0.5 || py < 2 * Dots::CELL_H || px < 0 || py >= 49 * Dots::CELL_H {
                continue;
            }
            for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                assert!(
                    g.dots.is_lit(px + ox, py + oy),
                    "giant {k} block at {px},{py}"
                );
            }
        }
    }

    #[test]
    fn gc_a10_trails_are_short_and_independent_of_the_draw_cadence() {
        let size = Size::new(120, 40);
        let run = |every: usize| {
            let mut ctx = Ctx::new(42, size);
            let mut g = GlobularCluster::new();
            g.resize(size, &mut ctx);
            for step in 1..=120 {
                g.update(SIM_DT, &mut ctx);
                if step % every == 0 {
                    g.prepare();
                }
            }
            g
        };
        let (a, b) = (run(1), run(2));
        let dots = a.dots.dot_size();
        for py in 0..i32::from(dots.height()) {
            for px in 0..i32::from(dots.width()) {
                assert_eq!(a.dots.is_lit(px, py), b.dots.is_lit(px, py), "{px},{py}");
            }
        }
        // A trail spans at most TRAIL_N snapshots, TRAIL_EVERY updates apart (≈ 0.27 s).
        assert_eq!(a.trail_len, TRAIL_N);
        assert!(f64::from(TRAIL_EVERY) * TRAIL_N as f64 * SIM_DT <= 0.3);
        // Trails restart at a reseed: one snapshot, at the new positions.
        let (mut g, mut ctx) = scene_at(size, 1.0);
        g.t = 121.49;
        g.update(SIM_DT, &mut ctx);
        assert_eq!(g.trail_len, 1);
        assert_eq!(
            snapshot(&g.trail, g.trail_head, 3, 0).to_vec(),
            g.cluster.pos[9..12]
        );
    }

    /// A scene resized to `size` and run for `t` simulated seconds.
    fn scene_at(size: Size, t: f64) -> (GlobularCluster, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = GlobularCluster::new();
        g.resize(size, &mut ctx);
        for _ in 0..(t / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        (g, ctx)
    }

    #[test]
    fn gc_a9_label_is_plain_words_in_whole_crossings_and_fits() {
        let (mut g, _) = scene_at(Size::new(80, 24), 0.0);
        g.age = 4.5 * T_CR;
        g.write_label();
        assert_eq!(g.label.as_str(), "N=384  4 crossings");
        g.age = 1.99 * T_CR;
        g.write_label();
        assert_eq!(g.label.as_str(), "N=384  1 crossing");
        // No "0 crossings" debug counter: the clause appears after the first crossing.
        g.age = 0.99 * T_CR;
        g.write_label();
        assert_eq!(g.label.as_str(), "N=384");
        // The crossings show only where the label takes at most half the width, so it
        // never turns into a banner; then the bare count while it fits in `w − 2`.
        g.age = 4.5 * T_CR;
        g.write_label();
        let full = g.label.as_str().len() as u16;
        for (w, expect) in [
            (2 * full, "N=384  4 crossings"),
            (2 * full - 1, "N=384"),
            (7, "N=384"),
            (6, ""),
            (1, ""),
        ] {
            g.size = Size::new(w, 24);
            g.write_label();
            assert_eq!(g.label.as_str(), expect, "w = {w}");
        }
        g.age = 0.0;
        for (w, expect) in [(10, "N=384"), (7, "N=384"), (6, "")] {
            g.size = Size::new(w, 24);
            g.write_label();
            assert_eq!(g.label.as_str(), expect, "w = {w}");
        }
        // Over a whole 120 s evolve phase the age never outgrows the buffer.
        g.size = Size::new(80, 24);
        g.age = 120.0 * RATE;
        g.write_label();
        assert!(
            g.label.as_str().ends_with(" crossings"),
            "{}",
            g.label.as_str()
        );
    }

    #[test]
    fn gc_a9_nothing_touches_the_label_and_giants_glint() {
        for (size, times) in [
            (Size::new(30, 10), [0.0, 5.0, 12.0]),
            (Size::new(40, 12), [0.0, 5.0, 12.0]),
            (Size::new(80, 24), [0.0, 5.0, 12.0]),
        ] {
            let (mut g, mut ctx) = scene_at(size, 0.0);
            let mut now = 0.0;
            for t in times {
                while now < t {
                    g.update(SIM_DT, &mut ctx);
                    g.prepare();
                    now += SIM_DT;
                }
                let len = g.label.as_str().len() as i32;
                assert!(len > 0, "{size:?}");
                for px in 0..(len + 2) * Dots::CELL_W {
                    for py in 0..Dots::CELL_H {
                        assert!(
                            !g.dots.is_lit(px, py),
                            "dot {px},{py} by the label at {size:?}"
                        );
                    }
                }
            }
        }

        // Each giant slot twinkles slowly and independently between 235 and 255.
        let (g, _) = scene_at(Size::new(80, 24), 0.0);
        for (k, &(period, _)) in g.twinkle.iter().enumerate() {
            assert!((5.0..=9.0).contains(&period), "P = {period}");
            let levels: Vec<f64> = (0..=540)
                .map(|i| g.giant_level(k, f64::from(i) / 60.0))
                .collect();
            let lo = levels.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = levels.iter().copied().fold(0.0, f64::max);
            assert!(lo >= 235.0 && hi <= 255.0, "slot {k}: {lo}..{hi}");
            assert!(hi - lo >= 15.0, "slot {k} barely glints: {lo}..{hi}");
        }
        assert_ne!(g.twinkle[0], g.twinkle[1]);
    }
}
