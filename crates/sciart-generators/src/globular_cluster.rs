//! An ancient globular cluster of live gravitating stars, seen by a slowly turning telescope (SPEC-002 `globular_cluster`).
//!
//! Direct softened N-body (N ≤ 500) in Hénon units from a Plummer sphere, leapfrogged in
//! real time. Each body stands for ~10³ stars, so its unresolved light is deposited on a
//! fixed model-plane lattice and blurred into a glow that follows the bodies (and breathes
//! as they cross the core); on top of it each body is drawn as one representative resolved
//! star of a colour class (faint turn-off, red-giant branch, blue horizontal branch, or a
//! bright giant).

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, draw, geom, ink, ode, raster, rng};

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
/// The glow lattice: `LAT × LAT` cells over the model plane `[−SPAN, SPAN]²` at the
/// origin's depth. It is fixed in model units, so it is the same (and capped, ADR-0012) at
/// every terminal size and costs the same to blur.
const LAT: usize = 64;
const SPAN: f64 = 2.0;
/// Box-blur radius (lattice cells) and passes: three passes approximate a Gaussian of
/// σ ≈ 2.4 cells ≈ 0.15 model units, a quarter of the Plummer scale radius, which smooths
/// 500 bodies into light while the core still visibly shifts at each crossing (GC-A7).
const BLUR_R: usize = 2;
const BLUR_PASSES: usize = 3;
/// The glow's soft knee (share of the central surface brightness): `g²/(g + p)`, so light
/// well below it fades quadratically to black instead of being cut. One blurred body peaks
/// at ≈ 0.013, so lone halo bodies (drawn as stars) leave only a trace.
const GLOW_KNEE_P: f64 = 0.04;
/// Share of the glow taken from the bodies' own light at the centre, tapering as
/// `1/(1 + R²/a²)` outwards; the rest is the smooth Plummer surface brightness
/// `(1 + R²/a²)⁻²` about the light's centre, so the outline stays round while the core
/// still shifts and breathes with the bodies (GC-A7).
const BODY_LIGHT: f64 = 0.5;
/// Glow levels below this dissolve (CORE-R89): only the ramp's faint floor.
const GLOW_FLOOR: u8 = 16;
/// The halo resolves into stars (GC-A7): each body stands for ~10³ stars, so it carries
/// `COMPANIONS` faint `GRAIN` dots at fixed 3-D offsets within `COMPANION_R` model units,
/// drawn where the glow is below `GRAIN_TOP`. They orbit, turn and fade with their body.
const COMPANIONS: usize = 3;
const COMPANION_R: f64 = 0.2;
const GRAIN_TOP: u8 = 150;
const GRAIN: Ink = Ink::new(Role::Star, 40);
/// Motion streaks (GC-A10): each coloured star outside the bright glow trails a faint
/// line back to where it was `TRAIL_T` N-body time units ago (≈ 1 s, a seventh of a
/// crossing). **Stylised**: a long exposure; the eye sees no trails.
const TRAIL_T: f64 = 0.4;
const TRAIL: Ink = Ink::new(Role::Star, 26);
const TRAIL_MAX_G: f64 = 0.12;
/// Seconds per turn of the telescope round the cluster.
const TURN: f64 = 48.0;
const GLOW_RAMP: Role = Role::Ramp(Ramp::Starlight);
/// Tone curve of the glow, in units of the Plummer central surface brightness: the asinh
/// knee keeps the inner halo visible; white sits just below the centre's (blurred) value,
/// so only the innermost core reaches the pale top (GC-A7).
const GLOW_KNEE: f32 = 0.3;
const GLOW_WHITE: f32 = 0.9;
/// Contrast of the glow: above 1 it keeps the faint outskirts near black, so the halo reads
/// as resolved stars on the night sky rather than a disc.
const GLOW_GAMMA: f64 = 1.25;
/// Stars are lost in the core's glare where the glow exceeds `GLARE_G` of the central
/// surface brightness (`R ≈ a`), each at its own hashed depth into the glare, so the core
/// has no hard rim of stars.
const GLARE_G: f64 = 0.25;
/// A giant shows as a `✦` only where the glow is below `GIANT_G`; nearer the core it is an
/// ordinary star.
const GIANT_G: f64 = 0.25;
const GIANT: Glyph = Glyph::new('✦', '*');
/// Temperatures (K): the red giants, and the colour classes of the other bodies (a visual
/// tag; the dynamics stay equal-mass), as `(cumulative fraction, lowest, highest)`:
/// turn-off and subgiant stars, the red-giant branch, the blue horizontal branch (GC-A8).
const GIANT_K: f64 = 3900.0;
/// `Star` level of a turn-off star's single dot: the faint majority.
const TURNOFF_LEVEL: u8 = 140;
const CLASSES: [(f64, f64, f64); 3] = [
    (0.76, 5600.0, 6400.0),
    (0.93, 4300.0, 4900.0),
    (1.0, 9000.0, 12_000.0),
];

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

/// Three-pass box blur of the `LAT × LAT` lattice `v` in place, rows then columns, with
/// `tmp` as scratch; cells beyond the edge count as empty, so light is not piled up there.
/// Each pass keeps the total except what spills over the edge.
pub(crate) fn blur(v: &mut [f32], tmp: &mut [f32]) {
    let norm = 1.0 / (2 * BLUR_R + 1) as f32;
    // One running-sum pass along a line of `LAT` cells `stride` apart.
    let line = |src: &[f32], dst: &mut [f32], start: usize, stride: usize| {
        let mut sum: f32 = (0..BLUR_R.min(LAT)).map(|i| src[start + i * stride]).sum();
        for i in 0..LAT {
            if i + BLUR_R < LAT {
                sum += src[start + (i + BLUR_R) * stride];
            }
            if i > BLUR_R {
                sum -= src[start + (i - BLUR_R - 1) * stride];
            }
            dst[start + i * stride] = sum * norm;
        }
    };
    for _ in 0..BLUR_PASSES {
        for r in 0..LAT {
            line(v, tmp, r * LAT, 1);
        }
        for c in 0..LAT {
            line(tmp, v, c, LAT);
        }
    }
}

/// A fixed uniform in `[0, 1)` per body `k` and `stream`, a pure function of the salt, so
/// a star's colour and its place in a fade never flicker.
pub(crate) fn body_hash(salt: u64, k: usize, stream: i32) -> f64 {
    rng::hash01(salt ^ rng::cell_key(k as i32, stream))
}

/// Ink of body `k`'s resolved star (not a giant): turn-off stars, the faint majority, as a
/// plain dim `Star` dot; the brighter red-giant-branch and horizontal-branch stars in
/// their blackbody colour (GC-A8).
fn class_ink(salt: u64, k: usize) -> Ink {
    let u = body_hash(salt, k, 1);
    if u < CLASSES[0].0 {
        Ink::new(Role::Star, TURNOFF_LEVEL)
    } else {
        ink::blackbody(ink::class_kelvin(&CLASSES, u, body_hash(salt, k, 2)))
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
    /// N-body time since the last reseed, for the readout.
    age: f64,
    /// Half-mass radius smoothed over `RH_TAU`, sizing the glow.
    rh: f64,
    /// Salt of the per-body hashes (colour class, fade order), drawn once from `ctx.rng`.
    salt: u64,
    size: Size,
    /// Rows per model unit at the origin.
    scale: f64,
    tone: Tone,
    /// Bodies' light on the model-plane lattice (`LAT²`, row-major), blurred, and the
    /// blur's scratch; both fixed-size, allocated once in `new`.
    light: Box<[f32]>,
    blur: Box<[f32]>,
    /// The toned glow level of each lattice cell, for the current brightness.
    toned: Box<[f32]>,
    /// Factor from `light` to units of the Plummer central surface brightness.
    light_unit: f64,
    /// Unresolved light.
    pixels: Pixels,
    /// Resolved stars, laid over the glow.
    dots: Dots,
    /// Giant glyph cells, rasterised with the dots.
    giants: [(i32, i32); MAX_GIANTS],
    n_giants: usize,
    /// Title-row readout, e.g. `N=500  3 crossings` (empty when it does not fit).
    readout: StackText<32>,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

/// ASCII stand-in for a glow pixel pair, by the brighter pixel's level: the classic `#`
/// core thinning through `+` and `:` to `.`.
fn ascii_glow(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = raster::stronger(top, bottom);
    let ch = match ink.level {
        _ if ink.role == Role::Background => return None,
        200.. => '#',
        130.. => '+',
        80.. => ':',
        40.. => '.',
        _ => return None,
    };
    Some((ch, ink))
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
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
            light: vec![0.0; LAT * LAT].into_boxed_slice(),
            blur: vec![0.0; LAT * LAT].into_boxed_slice(),
            toned: vec![0.0; LAT * LAT].into_boxed_slice(),
            light_unit: 0.0,
            pixels: Pixels::default(),
            dots: Dots::default(),
            giants: [(0, 0); MAX_GIANTS],
            n_giants: 0,
            readout: StackText::new(),
            dirty: false,
        }
    }

    /// Rises passed by time `t`: one per completed cycle, plus the current rise.
    fn epoch_at(&self, t: f64) -> i64 {
        self.timeline.cycle(t) + i64::from(self.timeline.at(t).index == RISE)
    }

    /// Brightness multiplier: fades out before the reseed and back in after it.
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

    /// Readout, glow, stars and giants for the current state (the M10 raster
    /// convention: `render` only blits).
    fn rasterise(&mut self) {
        self.write_readout();
        let b = self.brightness();
        self.expose();
        self.glow(b);
        self.bodies(b);
    }

    /// Continuous lattice coordinates of the cell-space point `(x, y)`.
    fn lattice_xy(&self, x: f64, y: f64) -> (f64, f64) {
        let (cx, cy) = self.centre();
        let k = LAT as f64 / (2.0 * SPAN);
        let u = (x - cx) / (geom::ASPECT * self.scale);
        let v = (y - cy) / self.scale;
        ((u + SPAN) * k - 0.5, (v + SPAN) * k - 0.5)
    }

    /// Deposit every body's light onto the lattice (bilinear, weighted by its near-eye
    /// fade) and blur it: the unresolved light of the ~10³ stars each body stands for, so
    /// the glow follows the bodies and breathes as they cross the core (GC-A7).
    fn expose(&mut self) {
        self.light.fill(0.0);
        let proj = self.camera().projector();
        let centre = self.centre();
        for p in self.cluster.pos.chunks_exact(3) {
            let Some((x, y, z)) = proj.project([p[0], p[1], p[2]], centre, self.scale) else {
                continue;
            };
            let (gx, gy) = self.lattice_xy(x, y);
            if !(gx.is_finite() && gy.is_finite()) {
                continue;
            }
            let (x0, y0) = (gx.floor(), gy.floor());
            let (fx, fy) = (gx - x0, gy - y0);
            let w = eye_fade(z);
            for (dx, dy, k) in [
                (0, 0, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                let (i, j) = (x0 as i64 + dx, y0 as i64 + dy);
                if (0..LAT as i64).contains(&i) && (0..LAT as i64).contains(&j) {
                    self.light[j as usize * LAT + i as usize] += (w * k) as f32;
                }
            }
        }
        blur(&mut self.light, &mut self.blur);
        // Bodies per lattice cell → surface density (M = 1) in units of the Plummer
        // central surface density `1/(π a²)`, `a` from the smoothed half-mass radius.
        let cell = 2.0 * SPAN / LAT as f64;
        let a = self.a_g();
        self.light_unit =
            std::f64::consts::PI * a * a / (self.cluster.n().max(1) as f64 * cell * cell);
    }

    /// The glow at cell-space point `(x, y)`, in units of the Plummer central surface
    /// brightness (≈ 1 at the centre of a fresh cluster), sampled bilinearly.
    fn glow_at(&self, x: f64, y: f64) -> f64 {
        self.sample(&self.light, x, y) * self.light_unit
    }

    /// A `LAT²` lattice sampled bilinearly at cell-space point `(x, y)` (0 off it).
    fn sample(&self, lattice: &[f32], x: f64, y: f64) -> f64 {
        let (gx, gy) = self.lattice_xy(x, y);
        if !(gx.is_finite() && gy.is_finite()) {
            return 0.0;
        }
        let (x0, y0) = (gx.floor(), gy.floor());
        let (fx, fy) = (gx - x0, gy - y0);
        let at = |i: f64, j: f64| {
            if (0.0..LAT as f64).contains(&i) && (0.0..LAT as f64).contains(&j) {
                f64::from(lattice[j as usize * LAT + i as usize])
            } else {
                0.0
            }
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Tone every lattice cell once per frame, so the pixels interpolate levels instead of
    /// each paying a `powf`. The glow `g` blends the bodies' light with the round Plummer
    /// profile about the light's centre (weighted by light², so the core, not a stray halo
    /// body, sets it); `tone((knee(g)·b)^GLOW_GAMMA)` with the soft knee `g²/(g + p)`.
    fn tone_lattice(&mut self, b: f64) {
        let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
        for (i, &l) in self.light.iter().enumerate() {
            let w = f64::from(l) * f64::from(l);
            sx += w * (i % LAT) as f64;
            sy += w * (i / LAT) as f64;
            sw += w;
        }
        let half = (LAT as f64 - 1.0) / 2.0;
        let (ci, cj) = if sw > 0.0 {
            (sx / sw, sy / sw)
        } else {
            (half, half)
        };
        // Lattice cells per model unit, over the Plummer scale radius.
        let k_a = LAT as f64 / (2.0 * SPAN) * self.a_g();
        for (i, (t, &l)) in self.toned.iter_mut().zip(self.light.iter()).enumerate() {
            let (di, dj) = (((i % LAT) as f64 - ci) / k_a, ((i / LAT) as f64 - cj) / k_a);
            let q = 1.0 / (1.0 + di * di + dj * dj);
            // The bodies' share tapers outwards (as the Plummer core does), where 500
            // bodies are too few to trace a smooth edge.
            let body = BODY_LIGHT * q;
            let g = body * f64::from(l) * self.light_unit + (1.0 - body) * q * q;
            let v = g * g / (g + GLOW_KNEE_P);
            *t = f32::from(self.tone.level((v * b).powf(GLOW_GAMMA) as f32));
        }
    }

    /// The toned glow level at cell-space point `(x, y)`.
    fn glow_level(&self, x: f64, y: f64) -> u8 {
        ink::level(self.sample(&self.toned, x, y))
    }

    /// The cluster's unresolved light in the Starlight ramp (GC-A7): a globular's
    /// integrated colour is nearly uniform (B − V ≈ 0.7), so the glow has brightness but no
    /// hue gradient. Each pixel shows its toned lattice level, the faint floor dissolved.
    fn glow(&mut self, b: f64) {
        self.tone_lattice(b);
        self.pixels.clear();
        self.dots.clear();
        let (cx, cy) = self.centre();
        let reach = SPAN * self.scale; // rows
        let px = self.pixels.pixel_size();
        let (w, h) = (i32::from(px.width()), i32::from(px.height()));
        let xs = ((cx - reach * geom::ASPECT).floor() as i32).max(0)
            ..=((cx + reach * geom::ASPECT).ceil() as i32).min(w - 1);
        let ys = ((2.0 * (cy - reach)).floor() as i32).max(0)
            ..=((2.0 * (cy + reach)).ceil() as i32).min(h - 1);
        for py in ys {
            let y = (f64::from(py) + 0.5) / 2.0;
            for px in xs.clone() {
                let level = self.glow_level(f64::from(px) + 0.5, y);
                if level > 0 {
                    self.pixels.set(px, py, Ink::new(GLOW_RAMP, level));
                }
            }
        }
        self.pixels.dissolve(GLOW_FLOOR);
    }

    /// Project every body through the turning camera as a star in its colour class; the
    /// first `g_n` are red giants (a render-only tag: the dynamics stay equal-mass).
    /// Stars in the core's glare are not drawn, and a fade thins the stars in a fixed
    /// per-body order rather than dimming them.
    fn bodies(&mut self, b: f64) {
        self.n_giants = 0;
        let proj = self.camera().projector();
        let centre = self.centre();
        let n_giants = giant_count(self.cluster.n());
        for k in 0..self.cluster.n() {
            let p = [
                self.cluster.pos[3 * k],
                self.cluster.pos[3 * k + 1],
                self.cluster.pos[3 * k + 2],
            ];
            let Some((x, y, z)) = proj.project(p, centre, self.scale) else {
                continue;
            };
            if body_hash(self.salt, k, 0) >= b * eye_fade(z) {
                continue;
            }
            // The body's unresolved companions, in the halo only (the glow covers the core).
            for j in 0..COMPANIONS {
                let stream = 4 + 3 * j as i32;
                let o = [0, 1, 2]
                    .map(|a| COMPANION_R * (2.0 * body_hash(self.salt, k, stream + a) - 1.0));
                let q = [p[0] + o[0], p[1] + o[1], p[2] + o[2]];
                if let Some((qx, qy, _)) = proj.project(q, centre, self.scale)
                    && self.glow_level(qx, qy) < GRAIN_TOP
                {
                    let (dx, dy) = Dots::from_cells(qx, qy);
                    self.dots.plot(dx, dy, GRAIN);
                }
            }
            let g = self.glow_at(x, y);
            if g >= GLARE_G * (1.0 + 1.5 * body_hash(self.salt, k, 3)) {
                continue;
            }
            let ink = if k < n_giants {
                if g < GIANT_G {
                    self.giants[self.n_giants] = (x.floor() as i32, y.floor() as i32);
                    self.n_giants += 1;
                    continue;
                }
                ink::blackbody(GIANT_K)
            } else {
                class_ink(self.salt, k)
            };
            let (dx, dy) = Dots::from_cells(x, y);
            // A faint streak back along its velocity over the last `TRAIL_T` (GC-A10), in
            // the halo only: a long exposure that shows the orbits turning.
            let v = [
                self.cluster.vel[3 * k],
                self.cluster.vel[3 * k + 1],
                self.cluster.vel[3 * k + 2],
            ];
            let back = [0, 1, 2].map(|a| p[a] - TRAIL_T * v[a]);
            if g < TRAIL_MAX_G
                && ink.role == Role::Blackbody
                && let Some((bx, by, _)) = proj.project(back, centre, self.scale)
            {
                let (tx, ty) = Dots::from_cells(bx, by);
                self.dots.line(tx, ty - 0.5, dx, dy - 0.5, TRAIL);
            }
            self.dots.plot(dx, dy - 0.5, ink);
            // Coloured stars are a 2 × 2 dot block: a lone braille dot is too thin to show
            // a colour, and they are the brighter stars; a turn-off star is one faint dot.
            if ink.role == Role::Blackbody {
                self.dots.plot(dx, dy + 0.5, ink);
                self.dots.plot(dx + 1.0, dy - 0.5, ink);
                self.dots.plot(dx + 1.0, dy + 0.5, ink);
            }
        }
    }

    /// The slowly turning, gently nodding telescope (SPEC-002 Motion): once round in
    /// `TURN` s, so the halo visibly swings past the core within a minute (GC-A10).
    fn camera(&self) -> Camera {
        Camera {
            yaw: TAU * self.t / TURN,
            pitch: 0.30 + 0.15 * (TAU * self.t / 67.0).sin_det(),
            distance: EYE,
        }
    }

    /// `N=500  3 crossings` (no crossings clause before the first), else `N=500`, else
    /// nothing, whichever fits the title row's readout room (GC-A9, GEN-R13). The age
    /// counts whole crossing times since the last reseed, so it changes every 7.1 s.
    fn write_readout(&mut self) {
        let room = title::readout_room(self.size, &Title::of(self));
        self.readout.clear();
        // `age ≥ 0`, and ≤ 17 crossings per evolve phase: the cast is exact.
        let crossings = (self.age / T_CR).floor() as u32;
        let _ = write!(self.readout, "N={}", self.cluster.n());
        if crossings > 0 {
            let plural = if crossings == 1 { "" } else { "s" };
            let _ = write!(self.readout, "  {crossings} crossing{plural}");
        }
        if self.readout.len() > room {
            self.readout.clear();
            let _ = write!(self.readout, "N={}", self.cluster.n());
            if self.readout.len() > room {
                self.readout.clear();
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
            self.cluster.plummer(&mut ctx.rng, n);
            self.rh = self.cluster.half_mass_radius();
        } else if n != self.cluster.n() {
            self.cluster.rebucket(&mut ctx.rng, n);
        }
        self.size = size;
        self.scale = draw::fit_radius(size, 0.92) / VIEW_RADIUS;
        self.pixels.resize(size);
        self.dots.resize(size);
        self.dirty = true;
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
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_glow(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
        let unicode = canvas.mode() == GlyphMode::Unicode;
        let giant = ink::blackbody(GIANT_K);
        for &(x, y) in &self.giants[..self.n_giants] {
            let bg = if unicode {
                self.pixels.cell_bg(x, y)
            } else {
                Ink::BLANK
            };
            canvas.put_bg(x, y, GIANT, giant, bg);
        }
        if !self.readout.is_empty() {
            let y = i32::from(self.size.height()) - 1;
            let ink = Ink::new(Role::Text, title::READOUT_LEVEL);
            draw::label_right(canvas, y, self.readout.as_str(), ink);
        }
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
        g.prepare();
        let before = g.cluster.clone();
        g.resize(Size::new(81, 24), &mut ctx);
        g.prepare();
        assert_eq!(g.cluster.pos, before.pos);
        assert_eq!(g.cluster.vel, before.vel);

        for (size, n) in [(Size::new(200, 50), 500), (Size::new(40, 12), 96)] {
            let old = g.cluster.pos.clone();
            g.resize(size, &mut ctx);
            g.prepare();
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

    /// Level-weighted mean pixel position of the glow.
    fn glow_centroid(g: &GlobularCluster) -> (f64, f64) {
        let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
        let px = g.pixels.pixel_size();
        for py in 0..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let w = f64::from(g.pixels.get(x, py).unwrap().level);
                (sx, sy, sw) = (sx + w * f64::from(x), sy + w * f64::from(py), sw + w);
            }
        }
        (sx / sw, sy / sw)
    }

    #[test]
    fn gc_a7_glow_follows_the_bodies_without_hue_or_rim_and_stars_are_coloured() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(42, size);
        let mut g = GlobularCluster::new();
        g.resize(size, &mut ctx);
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let (cx, cy) = g.centre();
        // One hue everywhere: every glow pixel, core to fringe, is the neutral Starlight
        // ramp (a globular's integrated colour has no radial gradient).
        let (mut lit, mut edge) = (0, 0);
        for py in 0..100 {
            for px in 0..200 {
                let ink = g.pixels.get(px, py).unwrap();
                if ink == Ink::BLANK {
                    continue;
                }
                lit += 1;
                assert_eq!(ink.role, GLOW_RAMP, "glow pixel {px},{py}");
                // No rim: a lit pixel next to the empty sky is faint.
                let sky = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|&(dx, dy)| g.pixels.get(px + dx, py + dy) == Some(Ink::BLANK));
                if sky {
                    edge += 1;
                    assert!(ink.level < 90, "rim pixel {px},{py} at {}", ink.level);
                }
            }
        }
        assert!(lit > 500 && edge > 50, "{lit} lit, {edge} at the edge");
        // A bright centre, fading outwards: the centre pixel beats the mean at R ≈ a.
        let centre = g.pixels.get(cx as i32, (2.0 * cy) as i32).unwrap().level;
        assert!(centre >= 180, "centre at {centre}");
        let ring = (g.a_g() * g.scale * geom::ASPECT) as i32;
        let at_a = g
            .pixels
            .get(cx as i32 + ring, (2.0 * cy) as i32)
            .unwrap()
            .level;
        assert!(at_a < centre, "{at_a} at R = a vs {centre}");
        // Every star is a faint turn-off dot, or (braille cell or giant) a blackbody
        // between the giants' 3900 K and the blue horizontal branch's 12 000 K.
        let (mut stars, mut coloured) = (0, 0);
        for cell in canvas.cells() {
            if ('\u{2801}'..='\u{28ff}').contains(&cell.ch) || cell.ch == '✦' {
                stars += 1;
                // Turn-off stars and the halo's grain are plain `Star` dots.
                if cell.ink.role == Role::Star {
                    continue;
                }
                coloured += 1;
                assert_eq!(cell.ink.role, Role::Blackbody, "{cell:?}");
                let k = ink::blackbody_kelvin(cell.ink);
                assert!((3800.0..12_500.0).contains(&k), "{k} K");
            }
        }
        assert!(coloured > 20, "only {coloured} coloured star cells");
        assert!(stars > 150, "only {stars} star cells");
        // Giants: at most g_n, none in the bright core.
        assert!(g.n_giants <= giant_count(g.cluster.n()));
        for &(x, y) in &g.giants[..g.n_giants] {
            let glow = g.glow_at(f64::from(x) + 0.5, f64::from(y) + 0.5);
            assert!(glow < 2.0 * GIANT_G, "giant in the core at {x},{y}: {glow}");
        }
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
        // The glow is the bodies' light: moving every body moves the glow with it.
        let (x0, y0) = glow_centroid(&g);
        for p in g.cluster.pos.chunks_exact_mut(3) {
            p[0] += 0.4;
        }
        g.rasterise();
        let (x1, y1) = glow_centroid(&g);
        assert!(
            (x1 - x0).abs() + (y1 - y0).abs() > 3.0,
            "glow did not follow the bodies: ({x0:.1}, {y0:.1}) → ({x1:.1}, {y1:.1})"
        );
    }

    #[test]
    fn gc_a7_glow_is_round_and_fades_out_softly() {
        // The roundest object in the sky: from the glow's centroid, the distance out to
        // the floor contour varies by at most 1.3× round the compass, and along each
        // ray the light fades over several pixels rather than stepping off a cliff.
        for t in [1.0, 10.0, 30.0] {
            let (g, _) = scene_at(Size::new(120, 40), t);
            let (cx, cy) = glow_centroid(&g);
            let level = |x: f64, y: f64| {
                g.pixels
                    .get(x.floor() as i32, y.floor() as i32)
                    .map_or(0, |i| if i == Ink::BLANK { 0 } else { i.level })
            };
            let (mut lo, mut hi) = (f64::INFINITY, 0.0_f64);
            for k in 0..32 {
                let (s, c) = (TAU * f64::from(k) / 32.0).sin_cos_det();
                // Walk out to the last lit pixel (the fringe dissolves, so allow gaps).
                let (mut r, mut last, mut contour) = (0.0, 0.0, 0.0);
                let mut ramp = Vec::new();
                while r < 60.0 {
                    let l = level(cx + r * c, cy + r * s);
                    if l > 0 {
                        last = r;
                    }
                    // The floor contour: pixels at or above it are never dissolved.
                    if l >= GLOW_FLOOR {
                        contour = r;
                    }
                    ramp.push(l);
                    r += 0.5;
                }
                lo = lo.min(contour);
                hi = hi.max(contour);
                // From four times the floor to nothing takes ≥ 3 pixels (6 half-pixel steps).
                let from = ramp.iter().rposition(|&l| l >= 4 * GLOW_FLOOR);
                if let Some(i) = from {
                    let end = (last / 0.5) as usize;
                    assert!(end >= i + 6, "t {t}, ray {k}: a cliff at {}", i / 2);
                }
            }
            assert!(hi <= 1.3 * lo, "t {t}: radius {lo:.1}..{hi:.1}");
        }
    }

    #[test]
    fn gc_a7_halo_grain_rides_with_the_bodies() {
        // The grain is the unresolved stars each body stands for: faint `GRAIN` dots that
        // travel with their body (not a mask fixed to the screen), so every cell of grain
        // lies within a companion's reach of a projected body.
        let size = Size::new(120, 40);
        let (g, _) = scene_at(size, 10.0);
        let proj = g.camera().projector();
        let bodies: Vec<(f64, f64)> = g
            .cluster
            .pos
            .chunks_exact(3)
            .filter_map(|p| proj.project([p[0], p[1], p[2]], g.centre(), g.scale))
            .map(|(x, y, _)| (x, y))
            .collect();
        // A companion's offset is at most √3·COMPANION_R model units (rows at `scale`),
        // plus the cell it lands in.
        let reach = 3.0_f64.sqrt() * COMPANION_R * g.scale + 1.5;
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut grain = 0;
        for y in 0..40 {
            for x in 0..120 {
                let cell = canvas.get(x, y).unwrap();
                if !('\u{2801}'..='\u{28ff}').contains(&cell.ch) {
                    continue;
                }
                // A cell of grain alone (a star in it would outshine the grain).
                if cell.ink != GRAIN {
                    continue;
                }
                grain += 1;
                let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                assert!(
                    bodies.iter().any(|&(bx, by)| {
                        (bx - cx).abs() <= reach * geom::ASPECT && (by - cy).abs() <= reach
                    }),
                    "grain cell {x},{y} far from every body"
                );
            }
        }
        assert!(grain > 40, "only {grain} grain cells");
    }

    #[test]
    fn gc_a10_the_cluster_visibly_moves_between_frames_seconds_apart() {
        const { assert!(TURN <= 60.0) };
        let size = Size::new(120, 40);
        let mut stage = Stage::new(build(), 42, size, GlyphMode::Unicode);
        let frame = |stage: &mut Stage| -> Vec<(char, Ink)> {
            stage
                .render()
                .cells()
                .iter()
                .map(|c| (c.ch, c.ink))
                .collect()
        };
        for _ in 0..300 {
            stage.step(SIM_DT);
        }
        let a = frame(&mut stage);
        for _ in 0..(25 * 60) {
            stage.step(SIM_DT);
        }
        let b = frame(&mut stage);
        // Among the cells lit in either frame, most change: the halo swings round.
        let lit = |c: &(char, Ink)| c.0 != ' ';
        let either = a.iter().zip(&b).filter(|(x, y)| lit(x) || lit(y)).count();
        let changed = a
            .iter()
            .zip(&b)
            .filter(|(x, y)| (lit(x) || lit(y)) && x != y)
            .count();
        assert!(
            changed * 2 > either,
            "{changed} of {either} lit cells changed"
        );
        // Trails: the long exposure lights more of the halo than the stars alone.
        let mut g = GlobularCluster::new();
        g.resize(size, &mut Ctx::new(42, size));
        g.prepare();
        let halo_dots = |g: &GlobularCluster| {
            let d = g.dots.dot_size();
            (0..i32::from(d.height()))
                .flat_map(|y| (0..i32::from(d.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| g.dots.is_lit(x, y))
                .count()
        };
        let with = halo_dots(&g);
        g.cluster.vel.iter_mut().for_each(|v| *v = 0.0);
        g.rasterise();
        let without = halo_dots(&g);
        assert!(with > without + 20, "{with} vs {without} lit dots");
    }

    #[test]
    fn gc_a8_colour_classes_follow_the_old_population_mix() {
        let n = 4000;
        let (mut turnoff, mut rgb, mut bhb) = (0, 0, 0);
        for k in 0..n {
            let ink = class_ink(7, k);
            if ink == Ink::new(Role::Star, TURNOFF_LEVEL) {
                turnoff += 1;
                continue;
            }
            assert_eq!(ink.role, Role::Blackbody);
            // Levels are log-spaced ~1.3 % apart in temperature: allow one level.
            match ink::blackbody_kelvin(ink) {
                4250.0..=4960.0 => rgb += 1,
                8850.0..=12_200.0 => bhb += 1,
                kelvin => panic!("{kelvin} K is in no class"),
            }
        }
        let frac = |c: usize| c as f64 / n as f64;
        assert!((frac(turnoff) - 0.76).abs() < 0.03, "{}", frac(turnoff));
        assert!((frac(rgb) - 0.17).abs() < 0.03, "{}", frac(rgb));
        assert!((frac(bhb) - 0.07).abs() < 0.03, "{}", frac(bhb));
        assert_eq!(body_hash(3, 5, 1), body_hash(3, 5, 1));
        assert_ne!(body_hash(3, 5, 1), body_hash(3, 5, 2));
        assert_ne!(body_hash(3, 5, 1), body_hash(4, 5, 1));
    }

    /// A scene resized to `size` and run for `t` simulated seconds.
    fn scene_at(size: Size, t: f64) -> (GlobularCluster, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = GlobularCluster::new();
        g.resize(size, &mut ctx);
        g.prepare();
        for _ in 0..(t / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        (g, ctx)
    }

    #[test]
    fn gc_a9_readout_counts_whole_crossings_and_fits_the_title_row() {
        let (mut g, _) = scene_at(Size::new(80, 24), 0.0);
        g.age = 4.5 * T_CR;
        g.write_readout();
        assert_eq!(g.readout.as_str(), "N=384  4 crossings");
        g.age = 1.99 * T_CR;
        g.write_readout();
        assert_eq!(g.readout.as_str(), "N=384  1 crossing");
        // No "0 crossings" debug counter: the clause appears after the first crossing.
        g.age = 0.99 * T_CR;
        g.write_readout();
        assert_eq!(g.readout.as_str(), "N=384");
        // Shortened, then dropped, as the room right of the title shrinks.
        g.age = 4.5 * T_CR;
        for (w, h, expect) in [
            (80, 24, "N=384  4 crossings"),
            (40, 12, "N=384  4 crossings"),
            (30, 10, "N=384"),
            (8, 24, ""),
        ] {
            g.size = Size::new(w, h);
            g.write_readout();
            assert_eq!(g.readout.as_str(), expect, "{w}x{h}");
            let room = title::readout_room(g.size, &Title::of(&g));
            assert!(g.readout.len() <= room);
        }
        // Over a whole 120 s evolve phase the age never outgrows the buffer.
        g.size = Size::new(200, 50);
        g.age = 120.0 * RATE;
        g.write_readout();
        assert!(g.readout.as_str().ends_with(" crossings"));

        // Drawn right-aligned on the title row at the readout level.
        let (g, _) = scene_at(Size::new(80, 24), 8.0);
        let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
        g.render(&mut canvas);
        let last = &canvas.to_lines()[23];
        assert!(last.ends_with(" N=384  1 crossing "), "{last:?}");
        let n = canvas.get(79 - 17, 23).unwrap();
        assert_eq!((n.ch, n.ink.level), ('N', title::READOUT_LEVEL));
    }
}
