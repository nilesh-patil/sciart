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
    category: Category::Astronomy,
    description: "An ancient globular cluster of live gravitating stars, seen by a slowly turning telescope",
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
const RH_PER_A: f64 = 1.3048;
/// Allowed body counts; the largest is the ADR-0012 cap.
const BUCKETS: [usize; 7] = [64, 96, 128, 192, 256, 384, 500];
const N_MAX: usize = BUCKETS[BUCKETS.len() - 1];
const MAX_GIANTS: usize = 10;
/// Camera eye distance from the origin, model units (SPEC-002 Motion).
const EYE: f64 = 10.0;
/// Re-centring passes onto the inner half's mean position (GC-A2).
const DENSITY_PASSES: usize = 4;
/// Seconds of the `r_h` smoothing that drives the glow radius.
const RH_TAU: f64 = 4.0;
/// Model radius filling the frame's fit radius (≈ 93 % of the projected mass).
const VIEW_RADIUS: f64 = 2.2;
/// The glow ends where `p` reaches 0: `g = 0.08` at `R = √(√12.5 − 1) a_g ≈ 1.5923 a_g`.
const GLOW_EDGE: f64 = 1.5924;
const GIANT: Glyph = Glyph::new('✦', '*');
/// A giant stands out only where the unresolved glow lights less than this fraction
/// of the dots (`R > 0.83 a_g`); nearer the core it is lost in the glare.
const GIANT_MAX_GLOW: f64 = 0.5;
/// Giant level `GIANT_MID ± GIANT_SWING`, twinkling with a per-giant period drawn
/// from `GIANT_PERIODS` seconds.
const GIANT_MID: f64 = 200.0;
const GIANT_SWING: f64 = 30.0;
const GIANT_PERIODS: (f64, f64) = (5.0, 9.0);
/// The label's name part, also the label on canvases too narrow for the counts.
const NAME: &str = "globular cluster";

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
    let (s, c) = (TAU * rng.random::<f64>()).sin_cos();
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

/// `(R/a_g)²` of a point `(dx, dy)` cells from the centre, given `k2 = (scale·a_g)²`.
fn x2_over(dx: f64, dy: f64, k2: f64) -> f64 {
    ((dx / geom::ASPECT).powi(2) + dy * dy) / k2
}

/// Plummer surface brightness `g = (1 + x²)^{−2}`, from `x² = (R/a_g)²`.
fn surface(x2: f64) -> f64 {
    let d = 1.0 + x2;
    1.0 / (d * d)
}

/// Probability that glow of surface brightness `g` lights a dot (GC-A8): 1 in the
/// saturated core (`R ≤ 0.52 a_g`), falling to 0 at `g = 0.08` (`R = 1.59 a_g`).
fn lit_p(g: f64) -> f64 {
    (1.7 * (g - 0.08) / 0.92).clamp(0.0, 1.0)
}

/// A fixed per-dot uniform in `[0, 1)`, a pure function of `(x, y, salt)` (GC-A8), so
/// the glow speckle holds still from frame to frame.
pub(crate) fn dot_hash(x: i32, y: i32, salt: u64) -> f64 {
    rng::hash01(salt ^ rng::cell_key(x, y))
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

    /// Radius holding half the bodies, about the origin.
    pub fn half_mass_radius(&mut self) -> f64 {
        half_mass_radius(&self.pos, &mut self.scratch)
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
        // body core; the view and the glow are centred on the origin, so move the
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

pub(crate) struct GlobularCluster {
    cluster: Cluster,
    timeline: Timeline,
    /// Own accumulated simulated time (GEN-R7): drives the camera and the timeline.
    t: f64,
    /// Rises passed so far: a change means "reseed now".
    epoch: i64,
    /// N-body time since the last reseed, for the label.
    age: f64,
    /// Half-mass radius smoothed over `RH_TAU`, sizing the glow.
    rh: f64,
    /// Salt of the per-dot glow hash, drawn once from `ctx.rng`.
    salt: u64,
    size: Size,
    /// Rows per model unit at the origin.
    scale: f64,
    dots: Dots,
    /// Giant glyph cells and levels, rasterised with the dots.
    giants: [(i32, i32, u8); MAX_GIANTS],
    n_giants: usize,
    /// Per-giant twinkle `(period s, phase rad)`, drawn from the salt.
    twinkle: [(f64, f64); MAX_GIANTS],
    label: StackText<48>,
}

impl GlobularCluster {
    fn new() -> Self {
        Self {
            cluster: Cluster::preallocated(),
            timeline: Timeline::new(&[("evolve", 120.0), ("fade", 1.5), ("rise", 1.5)]),
            t: 0.0,
            epoch: 0,
            age: 0.0,
            rh: 0.0,
            salt: 0,
            size: Size::new(1, 1),
            scale: 1.0,
            dots: Dots::default(),
            giants: [(0, 0, 0); MAX_GIANTS],
            n_giants: 0,
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
        self.rh = self.cluster.half_mass_radius();
    }

    /// Canvas centre in continuous cell coordinates (cell `i` spans `[i, i + 1)`).
    fn centre(&self) -> (f64, f64) {
        (
            f64::from(self.size.width()) / 2.0,
            f64::from(self.size.height()) / 2.0,
        )
    }

    /// Glow radius `a_g`: the Plummer scale radius implied by the smoothed `r_h`.
    fn a_g(&self) -> f64 {
        (self.rh / RH_PER_A).max(1e-6)
    }

    /// `(scale·a_g)²`: `(R/a_g)² = ((dx/ASPECT)² + dy²)/k2` for cell offsets `(dx, dy)`.
    fn glow_k2(&self) -> f64 {
        let k = self.scale * self.a_g();
        k * k
    }

    /// `(R/a_g)²` of a point `(dx, dy)` cells from the centre (the tests' probe; the
    /// raster loops hoist `glow_k2`).
    #[cfg(test)]
    fn glow_x2(&self, dx: f64, dy: f64) -> f64 {
        x2_over(dx, dy, self.glow_k2())
    }

    /// Draw label, glow, bodies and giants for the current state (the M10 raster
    /// convention: `render` only blits). The label goes first, so the stars can keep
    /// clear of it.
    fn rasterise(&mut self) {
        self.dots.clear();
        self.write_label();
        let b = self.brightness();
        self.glow(b);
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

    /// Unresolved light: dot `d` is lit iff `p(R_d)·b > u(d)` (GC-A8). A cell whose
    /// centre lies in the saturated core (`p = 1`) is filled whole, as a saturated
    /// detector pixel is, so the core reads as a solid knot (GC-A7).
    fn glow(&mut self, b: f64) {
        let (cx, cy) = self.centre();
        let k2 = self.glow_k2();
        let keep_out = self.label_cells();
        let reach = GLOW_EDGE * self.a_g() * self.scale; // rows
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        let x_range = ((cx - reach * geom::ASPECT).floor() as i32).max(0)
            ..=((cx + reach * geom::ASPECT).ceil() as i32).min(w - 1);
        let y_range =
            ((cy - reach).floor() as i32).max(0)..=((cy + reach).ceil() as i32).min(h - 1);
        let (sub_w, sub_h) = (f64::from(Dots::CELL_W), f64::from(Dots::CELL_H));
        for row in y_range {
            for col in x_range.clone() {
                if row == 0 && col < keep_out {
                    continue;
                }
                let (fc, fr) = (f64::from(col), f64::from(row));
                let saturated = lit_p(surface(x2_over(fc + 0.5 - cx, fr + 0.5 - cy, k2))) >= 1.0;
                for sy in 0..Dots::CELL_H {
                    for sx in 0..Dots::CELL_W {
                        let dx = fc + (f64::from(sx) + 0.5) / sub_w - cx;
                        let dy = fr + (f64::from(sy) + 0.5) / sub_h - cy;
                        let g = surface(x2_over(dx, dy, k2));
                        let (px, py) = (col * Dots::CELL_W + sx, row * Dots::CELL_H + sy);
                        // A saturated dot is lit whatever its hash (`u < 1`): skip it.
                        let lit = if saturated && b >= 1.0 {
                            true
                        } else {
                            let p = if saturated { 1.0 } else { lit_p(g) } * b;
                            p > 0.0 && p > dot_hash(px, py, self.salt)
                        };
                        if lit {
                            let level = (90.0 + 165.0 * g) * b;
                            self.dots
                                .set(px, py, Ink::new(Role::Star, ink::level(level)));
                        }
                    }
                }
            }
        }
    }

    /// Project every body through the turning camera; the first `g_n` are red giants
    /// (a render-only tag: the dynamics stay equal-mass).
    fn bodies(&mut self, b: f64) {
        let proj = self.camera().projector();
        let centre = self.centre();
        let k2 = self.glow_k2();
        let keep_out = self.label_cells();
        let n_giants = giant_count(self.cluster.n());
        self.n_giants = 0;
        for (k, p) in self.cluster.pos.chunks_exact(3).enumerate() {
            let Some((x, y, z)) = proj.project([p[0], p[1], p[2]], centre, self.scale) else {
                continue;
            };
            let g = surface(x2_over(x - centre.0, y - centre.1, k2));
            // A giant in the bright core is lost in its glare, as in a real exposure:
            // it is drawn as an ordinary star there.
            if k < n_giants && lit_p(g) < GIANT_MAX_GLOW {
                let level = self.giant_level(k, self.t) * b;
                let (col, row) = (x.floor() as i32, y.floor() as i32);
                if level >= MIN_VISIBLE_LEVEL && !(row == 0 && col < keep_out) {
                    self.giants[self.n_giants] = (col, row, ink::level(level));
                    self.n_giants += 1;
                }
                continue;
            }
            let level =
                (100.0 + 140.0 * g.sqrt() + 20.0 * (z / 2.0).clamp(-1.0, 1.0)) * eye_fade(z) * b;
            let (dx, dy) = Dots::from_cells(x, y);
            // `Dots::plot` rounds to the nearest dot; round here to test the keep-out.
            let (px, py) = (dx.round() as i32, dy.round() as i32);
            let by_label = py < Dots::CELL_H && px < keep_out * Dots::CELL_W;
            if level >= MIN_VISIBLE_LEVEL && !by_label {
                self.dots
                    .plot(dx, dy, Ink::new(Role::Star, ink::level(level)));
            }
        }
    }

    /// Giant `k`'s level at time `t` before the fade: a slow, quiet twinkle
    /// `200 ± 30` with its own period and phase (visual only).
    fn giant_level(&self, k: usize, t: f64) -> f64 {
        let (period, phase) = self.twinkle[k];
        GIANT_MID + GIANT_SWING * (TAU * t / period + phase).sin()
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
            pitch: 0.30 + 0.15 * (TAU * self.t / 67.0).sin(),
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
        let _ = write!(self.label, "{NAME}  N={}", self.cluster.n());
        if crossings > 0 {
            let plural = if crossings == 1 { "" } else { "s" };
            let _ = write!(self.label, "  {crossings} crossing{plural}");
        }
        if 2 * self.label.len() > w {
            self.label.clear();
            if NAME.len() + 2 <= w {
                let _ = self.label.write_str(NAME);
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
            self.rh = self.cluster.half_mass_radius();
        } else if n != self.cluster.n() {
            self.cluster.rebucket(&mut ctx.rng, n);
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
            let rh = self.cluster.half_mass_radius();
            self.rh += (rh - self.rh) * (1.0 - (-dt / RH_TAU).exp());
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        for &(x, y, level) in &self.giants[..self.n_giants] {
            canvas.put(x, y, GIANT, Ink::new(Role::Hot, level));
        }
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
        assert_eq!(g.epoch, 1, "no reseed");
        assert_ne!(g.cluster.pos, old);
        assert_eq!(caps(&g.cluster), before);
        assert_eq!(g.cluster.n(), 500);
    }

    #[test]
    fn gc_a7_core_is_solid_glow_is_bounded_and_giants_are_few() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(42, size);
        let mut g = GlobularCluster::new();
        g.resize(size, &mut ctx);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let (cx, cy) = g.centre();
        let a_g = g.a_g();

        let proj = g.camera().projector();
        let body_dots: Vec<(i32, i32)> = g
            .cluster
            .pos
            .chunks_exact(3)
            .filter_map(|p| proj.project([p[0], p[1], p[2]], (cx, cy), g.scale))
            .map(|(x, y, _)| {
                (
                    (2.0 * x - 0.5).round() as i32,
                    (4.0 * y - 0.5).round() as i32,
                )
            })
            .collect();
        let dots = g.dots.dot_size();
        for py in 0..i32::from(dots.height()) {
            for px in 0..i32::from(dots.width()) {
                let x2 = g.glow_x2(
                    (f64::from(px) + 0.5) / 2.0 - cx,
                    (f64::from(py) + 0.5) / 4.0 - cy,
                );
                if x2 > 1.6 * 1.6 && g.dots.is_lit(px, py) {
                    assert!(body_dots.contains(&(px, py)), "stray glow dot at {px},{py}");
                }
            }
        }

        let (mut inner, mut outer) = (Vec::new(), Vec::new());
        let mut giants = 0;
        for row in 0..50 {
            for col in 0..200 {
                let cell = canvas.get(col, row).unwrap();
                let giant = cell.ink.role == Role::Hot;
                giants += usize::from(giant);
                let x2 = g.glow_x2(f64::from(col) + 0.5 - cx, f64::from(row) + 0.5 - cy);
                if x2 <= 0.5 * 0.5 {
                    assert!(
                        cell.ch == '⣿' || giant,
                        "core cell {col},{row} is {:?}",
                        cell.ch
                    );
                }
                if cell.ink.role == Role::Star {
                    if x2 <= 1.0 {
                        inner.push(f64::from(cell.ink.level));
                    } else if x2 > 3.0 * 3.0 {
                        outer.push(f64::from(cell.ink.level));
                    }
                }
            }
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!(!outer.is_empty());
        assert!(
            mean(&inner) >= mean(&outer) + 50.0,
            "{} vs {}",
            mean(&inner),
            mean(&outer)
        );
        assert_eq!(
            (giant_count(500), giant_count(384), giant_count(64)),
            (10, 7, 3)
        );
        // Near-eye halo stars (perspective ≥ 1.5×) fade out instead of streaking.
        for z in [-20.0, 0.0, 2.5] {
            assert_eq!(eye_fade(z), 1.0);
        }
        for z in [EYE / 3.0, 5.0, 9.9] {
            assert_eq!(eye_fade(z), 0.0);
        }
        assert!(eye_fade(2.7) > eye_fade(2.9) && eye_fade(2.9) > eye_fade(3.2));
        let g_n = giant_count(g.cluster.n());
        assert!(giants <= g_n, "{giants} giants");
        assert!(a_g > 0.3, "a_g = {a_g}");
        // A giant projected where the glow lights ≥ half the dots (R < 0.83 a_g) is lost
        // in the core's glare: no Hot glyph at its cell.
        let mut lost = 0;
        for p in g.cluster.pos.chunks_exact(3).take(g_n) {
            let (x, y, _) = proj.project([p[0], p[1], p[2]], (cx, cy), g.scale).unwrap();
            let x2 = g.glow_x2(x - cx, y - cy);
            if x2 < 0.83 * 0.83 {
                lost += 1;
                let cell = canvas.get(x.floor() as i32, y.floor() as i32).unwrap();
                assert_ne!(cell.ink.role, Role::Hot, "giant in the core at {x},{y}");
            }
        }
        assert!(
            lost > 0,
            "no giant in the core at seed 42 (the clause is untested)"
        );
    }

    #[test]
    fn gc_a8_glow_probability_and_dot_hash() {
        let glow_p = |x: f64| lit_p(surface(x * x));
        // p = 1 up to g = 0.08 + 0.92/1.7, i.e. R = 0.5185 a_g (the spec's "0.52").
        for i in 0..=518 {
            assert_eq!(glow_p(f64::from(i) / 1000.0), 1.0);
        }
        let xs: Vec<f64> = (0..=1000)
            .map(|i| 0.519 + 1.073 * f64::from(i) / 1000.0)
            .collect();
        for w in xs.windows(2) {
            assert!(glow_p(w[1]) < glow_p(w[0]), "not decreasing at {}", w[0]);
        }
        for x in [GLOW_EDGE, 2.0, 10.0] {
            assert_eq!(glow_p(x), 0.0);
        }
        assert_eq!(dot_hash(3, -7, 11), dot_hash(3, -7, 11));
        assert_ne!(dot_hash(3, -7, 11), dot_hash(3, -7, 12));
        assert!((0.0..1.0).contains(&dot_hash(-1, 5, 0)));
    }

    /// A scene resized to `size` and run for `t` simulated seconds.
    fn scene_at(size: Size, t: f64) -> (GlobularCluster, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = GlobularCluster::new();
        g.resize(size, &mut ctx);
        for _ in 0..(t / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
        }
        (g, ctx)
    }

    #[test]
    fn gc_a9_label_is_plain_words_in_whole_crossings_and_fits() {
        let (mut g, _) = scene_at(Size::new(80, 24), 0.0);
        g.age = 4.5 * T_CR;
        g.write_label();
        assert_eq!(g.label.as_str(), "globular cluster  N=384  4 crossings");
        g.age = 1.99 * T_CR;
        g.write_label();
        assert_eq!(g.label.as_str(), "globular cluster  N=384  1 crossing");
        // No "0 crossings" debug counter: the clause appears after the first crossing.
        g.age = 0.99 * T_CR;
        g.write_label();
        assert_eq!(g.label.as_str(), "globular cluster  N=384");
        // The counts show only where the label takes at most half the width, so it
        // never turns into a banner; then the bare name while it fits in `w − 2`.
        g.age = 4.5 * T_CR;
        g.write_label();
        let full = g.label.as_str().len() as u16;
        for (w, expect) in [
            (2 * full, "globular cluster  N=384  4 crossings"),
            (2 * full - 1, "globular cluster"),
            (40, "globular cluster"),
            (18, "globular cluster"),
            (17, ""),
            (1, ""),
        ] {
            g.size = Size::new(w, 24);
            g.write_label();
            assert_eq!(g.label.as_str(), expect, "w = {w}");
        }
        g.age = 0.0;
        for (w, expect) in [(46, "globular cluster  N=384"), (45, "globular cluster")] {
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
                for &(x, y, _) in &g.giants[..g.n_giants] {
                    assert!(y != 0 || x > len + 1, "giant at {x},{y} by the label");
                }
            }
        }

        // Each giant slot twinkles slowly and independently between 170 and 230.
        let (g, _) = scene_at(Size::new(80, 24), 0.0);
        for (k, &(period, _)) in g.twinkle.iter().enumerate() {
            assert!((5.0..=9.0).contains(&period), "P = {period}");
            let levels: Vec<f64> = (0..=540)
                .map(|i| g.giant_level(k, f64::from(i) / 60.0))
                .collect();
            let lo = levels.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = levels.iter().copied().fold(0.0, f64::max);
            assert!(lo >= 170.0 && hi <= 230.0, "slot {k}: {lo}..{hi}");
            assert!(hi - lo >= 40.0, "slot {k} barely glints: {lo}..{hi}");
        }
        assert_ne!(g.twinkle[0], g.twinkle[1]);
    }
}
