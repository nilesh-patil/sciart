//! A bright comet rounding the Sun, with a curved dust tail and a straight ion tail
//! (SPEC-002 `comet_tails`).
//!
//! Dust grains leave the nucleus with small velocity spreads and then feel the Sun's
//! gravity reduced by radiation pressure, `μ(1 − β)`, with β log-uniform in
//! `[0.04, 0.9]`. Each grain follows its own exact two-body orbit, propagated in closed
//! form with the universal-variable Kepler solution warm-started from the previous frame.
//! On the shown arc β > r/2a ≈ 0.015–0.03 everywhere, so every grain is on a hyperbolic
//! orbit. Grains of one β trace a syndyne, grains of one release time a synchrone, and
//! together they fan into a curved tail that lags behind the orbit. The ion tail is
//! kinematic: a fan of fine plasma streamers accelerated straight away from the Sun,
//! rippled by travelling solar-wind kinks; every so often the tail disconnects at the
//! coma, drifts off, and a new one regrows.
//!
//! Units: AU and years inside the physics (`μ_☉ = 4π²`), one simulated second = 1.7 days.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::Tone;
use sciart_core::particles::ParticlePool;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01};

pub static META: Meta = Meta {
    name: "comet_tails",
    category: Category::SunPlanets,
    description: "A comet rounds the Sun: a curved dust tail pushed by light, a straight ion tail",
    title: "Comet tails · light pushes dust",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CometTails::new())
}

/// Solar gravitational parameter, AU³/yr².
pub(crate) const MU: f64 = 4.0 * PI * PI;
/// The comet's orbit: perihelion distance (AU) and eccentricity.
const Q: f64 = 0.35;
const ECC: f64 = 0.97;
/// True anomalies at the start and end of one loop (the comet enters from behind the
/// Sun, off frame, and leaves through the top), and the loop length (simulated seconds).
const NU_START: f64 = -90.0 * PI / 180.0;
const NU_END: f64 = 95.0 * PI / 180.0;
const LOOP: f64 = 30.0;
/// Grains drawn (then thinned by activity) for the tail a new comet arrives with.
const BACKLOG: usize = 7000;
/// Dust offsets from the nucleus are drawn this many times longer than they are: the
/// real tail grows over weeks, more than the loop can show. **Exaggerated**.
const TAIL_EXAG: f64 = 5.0;
/// Radiation-pressure range (log-uniform) — kept below 1 (CMT-A5).
pub(crate) const BETA: (f64, f64) = (0.04, 0.9);
/// Pools (CMT-A5).
pub(crate) const DUST: usize = 8000;
/// Room for a whole attached tail (`ION_RATE · ION_LIFE`) plus a whole disconnected one,
/// so a cut-off tail never starves the regrowing one (CMT-A6).
pub(crate) const IONS: usize = 3000;
/// Dust release per simulated second at 1 AU; the rate goes as r^{−3/2}.
const DUST_RATE: f64 = 70.0;
/// Grains are dropped after this many simulated seconds or beyond this distance (AU).
const DUST_LIFE: f64 = 20.0;
const DUST_REACH: f64 = 6.0;
/// Ion-tail launch speed (AU/s) and acceleration (AU/s², simulated seconds), parcel life
/// and release rate.
const ION_V0: f64 = 0.25;
const ION_ACCEL: f64 = 0.16;
const ION_LIFE: f64 = 5.0;
const ION_RATE: f64 = 280.0;
/// Kink amplitude as a share of the distance down the tail, its temporal frequency (Hz)
/// and wavenumber (rad/AU): a ripple that runs down the tail.
const KINK: f64 = 0.04;
const KINK_HZ: f64 = 0.35;
const KINK_K: f64 = 5.0;
/// The ion tail is a fan of `STREAMERS` fine rays, `STREAM_SPREAD` rad apart, the middle
/// ones brightest (CMT-A7).
pub(crate) const STREAMERS: usize = 5;
const STREAM_SPREAD: f64 = 0.026;
const STREAM_W: [f32; STREAMERS] = [0.4, 0.8, 1.0, 0.65, 0.35];
/// Stroke segments per streamer (denser near the coma).
const STREAM_SEGS: usize = 48;
/// Ion light falls off down the tail as `1 / (1 + s / ION_FALL)` (s in AU) (CMT-A7).
const ION_FALL: f64 = 0.45;
/// A disconnection every `DISCONNECT` seconds, with release paused for `PAUSE` seconds;
/// the first comes `FIRST_CUT` seconds in (scene time 16 s after the 4 s pre-roll).
pub(crate) const DISCONNECT: f64 = 20.0;
pub(crate) const PAUSE: f64 = 1.6;
const FIRST_CUT: f64 = 20.0;
/// A disconnected tail fades out over this many seconds after the cut.
const DETACH_FADE: f64 = 4.0;
/// Anti-solar speed the solar wind gives a disconnected tail, AU per simulated second.
const DETACH_KICK: f64 = 0.3;
/// Box-blur radius (pixels) of the far-dust layer, applied twice (near-Gaussian).
const FAR_BLUR: usize = 2;
/// Kepler solve: Newton iterations per frame (warm start).
const NEWTON_WARM: usize = 3;
/// One pixel: the ion rays and their knots are drawn without a spread (critic round 2).
const POINT: [(i32, i32, f32); 1] = [(0, 0, 1.0)];
/// The head's radius in pixels ([`coma_ink`]); no Sun glow (it sat cut off in a corner).
const COMA_PX: f64 = 4.5;
/// A new comet arrives this far into its approach, with its tails seeded (GEN-R19: no
/// warm-up steps in `resize`).
const ARRIVE: f64 = 4.0;

/// Barker's mean-anomaly measure `D + D³/3` of true anomaly `nu` (parabolic time).
fn barker(nu: f64) -> f64 {
    let d = (0.5 * nu).tan();
    d + d * d * d / 3.0
}

/// Years per simulated second: the loop sweeps `NU_START..NU_END` in `LOOP` seconds
/// (Barker's equation: exact for e = 1, within a few per cent here; it only paces the
/// loop).
fn years_per_second() -> f64 {
    let k = (2.0 * Q * Q * Q / MU).sqrt();
    k * (barker(NU_END) - barker(NU_START)) / LOOP
}

/// Stumpff functions `C(z)`, `S(z)`.
fn stumpff(z: f64) -> (f64, f64) {
    if z > 1e-6 {
        let s = z.sqrt();
        ((1.0 - s.cos_det()) / z, (s - s.sin_det()) / (s * s * s))
    } else if z < -1e-6 {
        let s = (-z).sqrt();
        let (sh, ch) = (s.sinh(), s.cosh());
        ((ch - 1.0) / -z, (sh - s) / (s * s * s))
    } else {
        (0.5 - z / 24.0, 1.0 / 6.0 - z / 120.0)
    }
}

/// A two-body orbit about the Sun with gravitational parameter `mu`, from a state at
/// its epoch. `advance` returns the state `dt` years later, universal-variable form.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Orbit {
    pub r0: [f64; 2],
    pub v0: [f64; 2],
    pub mu: f64,
    /// Universal anomaly of the last solve (the warm start for the next one).
    pub chi: f64,
}

impl Orbit {
    pub fn new(r0: [f64; 2], v0: [f64; 2], mu: f64) -> Self {
        Self {
            r0,
            v0,
            mu,
            chi: 0.0,
        }
    }

    /// Position and velocity `dt` years after the epoch, after at most `iters` Newton
    /// steps on the universal Kepler equation from the stored `chi`.
    pub fn advance(&mut self, dt: f64, iters: usize) -> ([f64; 2], [f64; 2]) {
        let (r0v, v0v, mu) = (self.r0, self.v0, self.mu);
        let r0 = r0v[0].hypot(r0v[1]);
        let v2 = v0v[0] * v0v[0] + v0v[1] * v0v[1];
        let sqmu = mu.sqrt();
        let alpha = 2.0 / r0 - v2 / mu;
        let rv = (r0v[0] * v0v[0] + r0v[1] * v0v[1]) / sqmu;
        let mut chi = if self.chi == 0.0 {
            sqmu * dt / r0
        } else {
            self.chi
        };
        for _ in 0..iters {
            let z = alpha * chi * chi;
            let (c, s) = stumpff(z);
            let f = rv * chi * chi * c + (1.0 - alpha * r0) * chi * chi * chi * s + r0 * chi
                - sqmu * dt;
            let df = rv * chi * (1.0 - z * s) + (1.0 - alpha * r0) * chi * chi * c + r0;
            if !(df.is_finite() && df > 0.0) {
                break;
            }
            let step = f / df;
            chi -= step;
            if step.abs() < 1e-13 * (1.0 + chi.abs()) {
                break;
            }
        }
        self.chi = chi;
        let z = alpha * chi * chi;
        let (c, s) = stumpff(z);
        let f = 1.0 - chi * chi / r0 * c;
        let g = dt - chi * chi * chi / sqmu * s;
        let r = [f * r0v[0] + g * v0v[0], f * r0v[1] + g * v0v[1]];
        let rn = r[0].hypot(r[1]);
        let fd = sqmu / (rn * r0) * chi * (z * s - 1.0);
        let gd = 1.0 - chi * chi / rn * c;
        (r, [fd * r0v[0] + gd * v0v[0], fd * r0v[1] + gd * v0v[1]])
    }

    #[cfg(test)]
    /// Specific orbital energy of the reduced-μ problem.
    pub fn energy(&self, r: [f64; 2], v: [f64; 2]) -> f64 {
        0.5 * (v[0] * v[0] + v[1] * v[1]) - self.mu / r[0].hypot(r[1])
    }
}

/// The comet's heliocentric position and velocity at true anomaly `nu`: perihelion on
/// `+x`, moving counter-clockwise in `(x, y)` with `y` up.
pub(crate) fn comet_state(nu: f64) -> ([f64; 2], [f64; 2]) {
    let p = Q * (1.0 + ECC);
    let (s, c) = nu.sin_cos_det();
    let r = p / (1.0 + ECC * c);
    let k = (MU / p).sqrt();
    ([r * c, r * s], [-k * s, k * (ECC + c)])
}

/// True anomaly at loop time `t ∈ [0, LOOP)`, time-uniform (Barker's equation inverted
/// by Newton).
fn anomaly(t: f64) -> f64 {
    let m = barker(NU_START) + (barker(NU_END) - barker(NU_START)) * t / LOOP;
    let mut d = (0.5 * NU_START).tan();
    for _ in 0..12 {
        d -= (d + d * d * d / 3.0 - m) / (1.0 + d * d);
    }
    2.0 * d.atan()
}

/// Distance (AU) an attached ion parcel of `age` seconds has run down the tail.
fn ion_run(age: f64) -> f64 {
    ION_V0 * age + 0.5 * ION_ACCEL * age * age
}

/// A point `s` AU down streamer `k` of the ion tail of a nucleus at `nucleus` (with
/// lateral jitter `j`, AU per AU of run) at time `t`: along the anti-solar axis `anti`,
/// fanned by the streamer's angle and rippled by a kink that runs down the tail.
fn tail_point(nucleus: [f64; 2], anti: [f64; 2], t: f64, s: f64, k: u8, j: f64) -> [f64; 2] {
    let fan = STREAM_SPREAD * (f64::from(k) - 0.5 * (STREAMERS - 1) as f64);
    let phase = 1.7 * f64::from(k);
    let kink = KINK * s * (TAU * KINK_HZ * t - KINK_K * s + phase).sin_det();
    let side = s * (fan + j) + kink;
    [
        nucleus[0] + anti[0] * s - anti[1] * side,
        nucleus[1] + anti[1] * s + anti[0] * side,
    ]
}

/// In-place separable box blur of radius `r` over a row-major `w × h` buffer, using
/// `scratch` (at least `max(w, h)` long) for one line at a time; edges clamp. Mass is
/// conserved away from the edges. Allocation-free.
fn box_blur(buf: &mut [f32], w: usize, h: usize, r: usize, scratch: &mut [f32]) {
    if w == 0 || h == 0 || buf.len() < w * h || scratch.len() < w.max(h) {
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    // One line of `n` values at `start + i·stride`, blurred through `scratch`.
    let mut line = |buf: &mut [f32], start: usize, stride: usize, n: usize| {
        let at = |i: isize| start + (i.clamp(0, n as isize - 1) as usize) * stride;
        let mut acc = 0.0f32;
        for i in -(r as isize)..=(r as isize) {
            acc += buf[at(i)];
        }
        for (i, out) in scratch.iter_mut().take(n).enumerate() {
            *out = acc * norm;
            let ii = i as isize;
            acc += buf[at(ii + r as isize + 1)] - buf[at(ii - r as isize)];
        }
        for (i, s) in scratch.iter().take(n).enumerate() {
            buf[start + i * stride] = *s;
        }
    };
    for y in 0..h {
        line(buf, y * w, 1, w);
    }
    for x in 0..w {
        line(buf, x, w, h);
    }
}

#[derive(Debug, Clone, Copy)]
struct Grain {
    orbit: Orbit,
    /// Loop-independent release time, simulated seconds.
    born: f64,
    pos: [f64; 2],
    beta: f64,
}

#[derive(Debug, Clone, Copy)]
struct Ion {
    age: f64,
    /// Lateral jitter inside its streamer, `[-1, 1]`.
    jitter: f64,
    /// The ray of the fan it rides, `0..STREAMERS`.
    streamer: u8,
    /// Detached parcels fly free from `pos` with `vel` (AU, AU/s).
    attached: bool,
    /// Seconds since its tail was cut (detached parcels only).
    since_cut: f64,
    pos: [f64; 2],
    vel: [f64; 2],
}

impl Ion {
    fn new(age: f64, rng: &mut impl Rng) -> Self {
        Self {
            age,
            jitter: 2.0 * rng.random::<f64>() - 1.0,
            streamer: rng.random_range(0..STREAMERS) as u8,
            attached: true,
            since_cut: 0.0,
            pos: [0.0; 2],
            vel: [0.0; 2],
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct View {
    /// Pixel of the Sun and pixels per AU (`y` up in AU, down on screen).
    sx: f64,
    sy: f64,
    scale: f64,
    /// Frame size in pixels.
    w: f64,
    h: f64,
}

impl View {
    fn px(&self, p: [f64; 2]) -> (f64, f64) {
        (self.sx + self.scale * p[0], self.sy - self.scale * p[1])
    }
}

#[derive(Debug)]
pub struct CometTails {
    t: f64,
    yps: f64,
    nucleus: [f64; 2],
    velocity: [f64; 2],
    dust: ParticlePool<Grain>,
    ions: ParticlePool<Ion>,
    owed_dust: f64,
    owed_ions: f64,
    /// Release of ion parcels resumes at this time after a disconnection.
    ion_resume: f64,
    next_disconnect: f64,
    view: View,
    dust_glow: Glow,
    /// Grains older than 6 s, blurred before they join `dust_glow`.
    far_glow: Glow,
    /// One row or column of the blur (sized in `resize`).
    scratch: Vec<f32>,
    ion_glow: Glow,
    dust_tone: Tone,
    ion_tone: Tone,
    pixels: Pixels,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl CometTails {
    fn new() -> Self {
        let (nucleus, velocity) = comet_state(anomaly(0.0));
        Self {
            t: 0.0,
            yps: years_per_second(),
            nucleus,
            velocity,
            dust: ParticlePool::with_capacity(DUST),
            ions: ParticlePool::with_capacity(IONS),
            owed_dust: 0.0,
            owed_ions: 0.0,
            ion_resume: 0.0,
            next_disconnect: FIRST_CUT,
            view: View::default(),
            dust_glow: Glow::default(),
            far_glow: Glow::default(),
            scratch: Vec::new(),
            ion_glow: Glow::default(),
            dust_tone: Tone::asinh(0.3, 20.0),
            ion_tone: Tone::asinh(0.3, 8.0),
            pixels: Pixels::default(),
            dirty: false,
        }
    }

    /// Unit vector pointing away from the Sun at the nucleus.
    fn anti_solar(&self) -> [f64; 2] {
        let n = self.nucleus[0].hypot(self.nucleus[1]).max(1e-9);
        [self.nucleus[0] / n, self.nucleus[1] / n]
    }

    /// Release one grain at time `born` from a nucleus at `nucleus` moving at `velocity`.
    fn emit(
        &mut self,
        rng: &mut impl Rng,
        born: f64,
        nucleus: [f64; 2],
        velocity: [f64; 2],
    ) -> bool {
        let beta = (BETA.0.ln() + (BETA.1 / BETA.0).ln() * rng.random::<f64>()).exp();
        let speed = velocity[0].hypot(velocity[1]);
        let r = nucleus[0].hypot(nucleus[1]).max(1e-9);
        let sun = [nucleus[0] / r, nucleus[1] / r];
        // Ejection mostly sunward (the day side is active), a few per cent of v.
        let (a, b) = (rng.random::<f64>() - 0.5, rng.random::<f64>() - 0.5);
        let dv = 0.02 * speed;
        let v = [
            velocity[0] + dv * (2.0 * a - 0.8 * sun[0]),
            velocity[1] + dv * (2.0 * b - 0.8 * sun[1]),
        ];
        self.dust.spawn(Grain {
            orbit: Orbit::new(nucleus, v, MU * (1.0 - beta)),
            born,
            pos: nucleus,
            beta,
        })
    }

    fn release_dust(&mut self, dt: f64, rng: &mut impl Rng) {
        let r = self.nucleus[0].hypot(self.nucleus[1]);
        // Activity ∝ r^{−3/2}: water sublimation, flattened so the far legs still show.
        self.owed_dust += DUST_RATE * dt / (r * r.sqrt()).max(0.2);
        while self.owed_dust >= 1.0 {
            self.owed_dust -= 1.0;
            let (t, n, v) = (self.t, self.nucleus, self.velocity);
            if !self.emit(rng, t, n, v) {
                self.owed_dust = 0.0;
                break;
            }
        }
    }

    /// A new comet enters with the tails it grew on the way in: grains released over
    /// the last `DUST_LIFE` seconds from where the comet was then (its orbit extends to
    /// negative loop times), and a full ion tail. Grains are analytic orbits, so they
    /// need no history, only their release state.
    fn seed_backlog(&mut self, rng: &mut impl Rng) {
        self.dust.clear();
        self.ions.clear();
        let now = self.t.rem_euclid(LOOP);
        for _ in 0..BACKLOG {
            let tau = DUST_LIFE * rng.random::<f64>();
            let (n, v) = comet_state(anomaly(now - tau));
            let r = n[0].hypot(n[1]);
            // Activity ∝ r^{−3/2}, as in `release_dust`.
            if rng.random::<f64>() < (0.2 / (r * r.sqrt())).min(1.0) {
                self.emit(rng, self.t - tau, n, v);
            }
        }
        // A full attached tail, unless a cut is under way (then the regrowth handles it).
        if self.t >= self.ion_resume {
            let parcels = ((ION_RATE * ION_LIFE) as usize).min(IONS);
            for k in 0..parcels {
                let ion = Ion::new(ION_LIFE * k as f64 / parcels as f64, rng);
                self.ions.spawn(ion);
            }
        }
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        let before = (self.t / LOOP).floor();
        self.t += dt;
        let (n, v) = comet_state(anomaly(self.t.rem_euclid(LOOP)));
        self.nucleus = n;
        self.velocity = v;
        if (self.t / LOOP).floor() != before {
            self.seed_backlog(rng);
        }
        self.release_dust(dt, rng);
        let (t, yps) = (self.t, self.yps);
        self.dust.retain(|g| {
            let age = t - g.born;
            let (p, _) = g.orbit.advance(age * yps, NEWTON_WARM);
            g.pos = p;
            age < DUST_LIFE && p[0].hypot(p[1]) < DUST_REACH && p[0].is_finite()
        });

        // Ion tail.
        let anti = self.anti_solar();
        if self.t >= self.next_disconnect {
            self.next_disconnect += DISCONNECT;
            self.ion_resume = self.t + PAUSE;
            let (nucleus, cv, t) = (self.nucleus, self.velocity, self.t);
            for ion in self.ions.iter_mut().filter(|i| i.attached) {
                let s = ion_run(ion.age);
                // The cut-off plasmoid is swept away by the solar wind.
                let sp = ION_V0 + ION_ACCEL * ion.age + DETACH_KICK;
                // Keep its ray: the cut tail drifts off as the same fan of streamers.
                ion.pos = tail_point(nucleus, anti, t, s, ion.streamer, 0.003 * ion.jitter);
                ion.vel = [cv[0] * yps + anti[0] * sp, cv[1] * yps + anti[1] * sp];
                ion.attached = false;
            }
        }
        for ion in self.ions.iter_mut() {
            ion.age += dt;
            if !ion.attached {
                ion.since_cut += dt;
                let r = ion.pos[0].hypot(ion.pos[1]).max(1e-6);
                let a = [ion.pos[0] / r * ION_ACCEL, ion.pos[1] / r * ION_ACCEL];
                ion.vel = [ion.vel[0] + a[0] * dt, ion.vel[1] + a[1] * dt];
                ion.pos = [ion.pos[0] + ion.vel[0] * dt, ion.pos[1] + ion.vel[1] * dt];
            }
        }
        self.ions.retain(|i| {
            if i.attached {
                i.age < ION_LIFE
            } else {
                i.since_cut < DETACH_FADE
            }
        });
        if self.t >= self.ion_resume {
            self.owed_ions += ION_RATE * dt;
            while self.owed_ions >= 1.0 {
                self.owed_ions -= 1.0;
                if !self.ions.spawn(Ion::new(0.0, rng)) {
                    self.owed_ions = 0.0;
                    break;
                }
            }
        }
    }

    /// A point `s` AU down streamer `k` (with lateral jitter `j`, AU per AU of run):
    /// along the anti-solar axis, fanned by the streamer's angle and rippled by a kink
    /// that runs down the tail.
    fn tail_point(&self, s: f64, k: u8, j: f64) -> [f64; 2] {
        tail_point(self.nucleus, self.anti_solar(), self.t, s, k, j)
    }

    /// Where a parcel is: attached ones ride their streamer, detached ones fly free.
    fn ion_pos(&self, ion: &Ion) -> [f64; 2] {
        if !ion.attached {
            return ion.pos;
        }
        self.tail_point(ion_run(ion.age), ion.streamer, 0.003 * ion.jitter)
    }

    /// Follow the comet: the nucleus sits off-centre towards the Sun, so the tails
    /// have the frame; the view keeps its orientation, so the tails swing round.
    fn follow(&mut self) {
        let anti = self.anti_solar();
        let v = &mut self.view;
        let off = 0.38 * v.w.min(2.0 * v.h);
        let (nx, ny) = (0.5 * v.w - off * anti[0], 0.5 * v.h + 0.6 * off * anti[1]);
        v.sx = nx - v.scale * self.nucleus[0];
        v.sy = ny + v.scale * self.nucleus[1];
    }

    fn compose(&mut self) {
        self.follow();
        let v = self.view;
        // Fade in and out at the loop's ends, where the next comet takes over.
        let tl = self.t.rem_euclid(LOOP);
        let env = (tl / 0.4).min((LOOP - tl) / 0.4).clamp(0.0, 1.0) as f32;
        const SOFT: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
        const MID: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
        let sum = |k: &[(i32, i32, f32)]| k.iter().map(|e| e.2).sum::<f32>();
        let n_mid = sum(&SOFT) / sum(&MID);
        self.dust_glow.clear();
        self.far_glow.clear();
        self.ion_glow.clear();
        let n = self.nucleus;
        for g in self.dust.iter() {
            let (x, y) = v.px([
                n[0] + TAIL_EXAG * (g.pos[0] - n[0]),
                n[1] + TAIL_EXAG * (g.pos[1] - n[1]),
            ]);
            // Fine grains (large β) scatter more light per unit mass and fill the fan.
            let age = self.t - g.born;
            let w = 0.45 * ((0.5 + g.beta) * (1.0 - age / DUST_LIFE).max(0.0)) as f32;
            // Older grains have spread over more of the sky: they go into a layer that
            // is blurred below, so the sparse far tail reads as a smooth haze, not
            // speckle, while the young fan near the coma stays crisp.
            let (x, y) = (2.0 * x, 2.0 * y);
            if age < 2.0 {
                self.dust_glow.splat(x, y, &SOFT, w);
            } else if age < 6.0 {
                self.dust_glow.splat(x, y, &MID, w * n_mid);
            } else {
                self.far_glow.splat(x, y, &SOFT, w);
            }
        }
        let far_px = self.far_glow.pixel_size();
        let (fw, fh) = (usize::from(far_px.width()), usize::from(far_px.height()));
        let far = self.far_glow.values_mut();
        for _ in 0..2 {
            box_blur(far, fw, fh, FAR_BLUR, &mut self.scratch);
        }
        for (d, f) in self.dust_glow.values_mut().iter_mut().zip(far.iter()) {
            *d += f;
        }
        // The attached ion tail: each streamer stroked from the coma to the oldest
        // attached parcel's run, bright at the root and fading down the tail.
        let mut run = 0.0f64;
        for ion in self.ions.iter().filter(|i| i.attached) {
            run = run.max(ion_run(ion.age));
        }
        if run > 0.0 {
            for k in 0..STREAMERS as u8 {
                let mut a = v.px(self.tail_point(0.0, k, 0.0));
                for i in 1..=STREAM_SEGS {
                    let u = i as f64 / STREAM_SEGS as f64;
                    let s = run * u * u;
                    let b = v.px(self.tail_point(s, k, 0.0));
                    let w = STREAM_W[usize::from(k)] * (1.0 / (1.0 + s / ION_FALL)) as f32;
                    // A thin antialiased ray (M14: no soft body under it).
                    let len = (b.0 - a.0).hypot(b.1 - a.1);
                    let n = (2.0 * len).ceil().clamp(1.0, 1024.0) as usize;
                    let per = 1.6 * w * (len / n as f64) as f32;
                    for j in 0..n {
                        let f = (j as f64 + 0.5) / n as f64;
                        // Pixel `x` spans [x, x + 1); bilinear splats take centres.
                        let (x, y) = (a.0 + f * (b.0 - a.0) - 0.5, a.1 + f * (b.1 - a.1) - 0.5);
                        self.ion_glow
                            .splat(2.0 * (x + 0.5), 2.0 * (y + 0.5), &POINT, per);
                    }
                    a = b;
                }
            }
        }
        // Parcels on top: knots of plasma running out along the rays, and the
        // disconnected tail drifting off and fading.
        for ion in self.ions.iter() {
            let (x, y) = v.px(self.ion_pos(ion));
            let w = if ion.attached {
                0.22 / (1.0 + ion_run(ion.age) / ION_FALL)
            } else {
                0.5 * (1.0 - ion.since_cut / DETACH_FADE).max(0.0)
            };
            self.ion_glow.splat(2.0 * x, 2.0 * y, &POINT, w as f32);
        }
        // Coma: a bright, compact halo round the nucleus.
        const COMA: [(i32, i32, f32); 49] = halo_kernel::<49>(3);
        let (nx, ny) = v.px(self.nucleus);
        self.dust_glow.splat(2.0 * nx, 2.0 * ny, &COMA, 30.0);

        let (dt, it) = (&self.dust_tone, &self.ion_tone);
        let size = self.pixels.pixel_size();
        let w = usize::from(size.width());
        let (dust, ions) = (self.dust_glow.values(), self.ion_glow.values());
        // The nucleus's pixel (the glow splats above take dot units, 2 per pixel).
        let (cx, cy) = (nx, ny);
        // Loop envelope in three steps: the comet fades in and out step by step.
        let fading = env < 0.5;
        for (i, px) in (0..dust.len().min(ions.len())).zip(0..) {
            let (x, y) = ((px % w) as i32, (px / w) as i32);
            let (ld, li) = (dt.level(env * dust[i]), it.level(env * ions[i]));
            let d = (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy);
            let coma = (d < COMA_PX && env > 0.0).then(|| coma_ink(d, fading));
            let ink = if let Some(c) = coma {
                c
            } else if let Some(ion) = ion_ink(li).filter(|_| u16::from(li) + 20 > u16::from(ld)) {
                ion
            } else if let Some(dust) = dust_ink(ld) {
                dust
            } else if hash01(0x57A ^ cell_key(x, y)) < 0.0015 {
                STAR
            } else {
                Ink::BLANK
            };
            self.pixels.set(x, y, ink);
        }
    }
}

/// Background stars: small, dim white points.
const STAR: Ink = Ink::new(Role::Star, 40);

/// The head in flat rings (ADR-0018): a white nucleus, a pale coma, a sun-yellow rim;
/// one step dimmer while the comet fades in or out.
fn coma_ink(d: f64, fading: bool) -> Ink {
    let k = usize::from(d >= 1.2) + usize::from(d >= 3.0) + usize::from(fading);
    [
        Ink::new(Role::Star, 255),
        Ink::new(Role::Star, 150),
        Ink::new(Role::Sun, 140),
        Ink::new(Role::Sun, 40),
    ][k.min(3)]
}

/// The dust tail in flat steps by tone level: pale where the young fan crowds, then
/// sun-yellow, then a dim ochre edge.
fn dust_ink(level: u8) -> Option<Ink> {
    match level {
        185.. => Some(Ink::new(Role::Star, 120)),
        100.. => Some(Ink::new(Role::Sun, 110)),
        75.. => Some(Ink::new(Role::Sun, 10)),
        _ => None,
    }
}

/// The ion rays in flat blue steps: cyan knots, blue rays, a faint blue tip.
fn ion_ink(level: u8) -> Option<Ink> {
    match level {
        150.. => Some(Ink::new(Role::Wave, 190)),
        60.. => Some(Ink::new(Role::Cold, 170)),
        25.. => Some(Ink::new(Role::Cold, 40)),
        _ => None,
    }
}

impl Generator for CometTails {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.dust_glow.resize(size);
        self.far_glow.resize(size);
        let fp = self.far_glow.pixel_size();
        self.scratch.clear();
        self.scratch
            .resize(usize::from(fp.width().max(fp.height())), 0.0);
        self.ion_glow.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        // The orbit's visible arc spans ±0.75 AU vertically; the Sun sits just off the
        // left edge so the tails stream across the frame.
        let scale = (0.36 * w).min(0.8 * h).max(1.0);
        self.view = View {
            sx: 0.0,
            sy: 0.0,
            scale,
            w,
            h,
        };
        if self.t == 0.0 {
            // Arrive with grown tails, `ARRIVE` s into the approach: the grains are
            // analytic orbits, so seeding there is the same as having flown there, and one
            // step places them.
            self.t = ARRIVE;
            let (n, v) = comet_state(anomaly(ARRIVE));
            (self.nucleus, self.velocity) = (n, v);
            self.seed_backlog(&mut ctx.rng);
            self.step(SIM_DT, &mut ctx.rng);
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt, &mut ctx.rng);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.compose();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    #[test]
    fn cmt_a1_grains_conserve_reduced_mu_energy() {
        for beta in [0.01, 0.3, 0.9] {
            let (r0, v0) = comet_state(0.2);
            let mut orbit = Orbit::new(r0, [v0[0] * 1.01, v0[1] * 0.99], MU * (1.0 - beta));
            let e0 = orbit.energy(r0, [v0[0] * 1.01, v0[1] * 0.99]);
            for k in 1..=400 {
                let (r, v) = orbit.advance(f64::from(k) * 5e-4, 50);
                let e = orbit.energy(r, v);
                assert!(
                    (e - e0).abs() <= 1e-9 * e0.abs().max(1.0),
                    "β {beta}: {e} vs {e0}"
                );
            }
        }
    }

    #[test]
    fn cmt_a1_every_grain_on_the_shown_arc_is_hyperbolic() {
        // β > r/2a on the whole shown arc: even the heaviest grain (β = 0.04), released
        // with the nucleus's own velocity, has positive energy in the reduced-μ problem.
        let a = Q / (1.0 - ECC);
        for i in 0..=100 {
            let nu = NU_START + (NU_END - NU_START) * f64::from(i) / 100.0;
            let (r, v) = comet_state(nu);
            let rn = r[0].hypot(r[1]);
            assert!(BETA.0 > rn / (2.0 * a), "ν {nu}: r/2a = {}", rn / (2.0 * a));
            let orbit = Orbit::new(r, v, MU * (1.0 - BETA.0));
            assert!(orbit.energy(r, v) > 0.0, "ν {nu}");
        }
    }

    #[test]
    fn cmt_a9_far_dust_blur_spreads_and_conserves_light() {
        // The far-dust haze: one bright pixel mid-buffer spreads to a 5×5 plateau (radius
        // 2) with its total conserved, and a degenerate buffer is left alone.
        let (w, h) = (11, 9);
        let mut buf = vec![0.0f32; w * h];
        buf[4 * w + 5] = 25.0;
        let mut scratch = vec![0.0f32; 11];
        box_blur(&mut buf, w, h, 2, &mut scratch);
        let total: f32 = buf.iter().sum();
        assert!((total - 25.0).abs() < 1e-4, "{total}");
        assert!((buf[4 * w + 5] - 1.0).abs() < 1e-5 && (buf[2 * w + 3] - 1.0).abs() < 1e-5);
        assert_eq!(buf[w + 5], 0.0);
        box_blur(&mut buf, 0, 0, 2, &mut scratch);
    }

    fn run(seconds: f64) -> CometTails {
        let mut g = CometTails::new();
        let mut ctx = Ctx::new(42, Size::new(100, 30));
        g.resize(Size::new(100, 30), &mut ctx);
        g.prepare();
        let mut t = 0.0;
        while t < seconds {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            t += SIM_DT;
        }
        g
    }

    #[test]
    fn cmt_a2_dust_tail_curves_behind_the_orbit() {
        // Just past perihelion (the loop's middle).
        let g = run(0.6 * LOOP);
        let anti = g.anti_solar();
        let vn = g.velocity[0].hypot(g.velocity[1]);
        let v = [g.velocity[0] / vn, g.velocity[1] / vn];
        let (mut side, mut n) = (0.0, 0);
        for gr in g.dust.iter() {
            let d = [gr.pos[0] - g.nucleus[0], gr.pos[1] - g.nucleus[1]];
            let along = d[0] * anti[0] + d[1] * anti[1];
            if along <= 0.05 {
                continue;
            }
            let perp = [d[0] - along * anti[0], d[1] - along * anti[1]];
            side += perp[0] * v[0] + perp[1] * v[1];
            n += 1;
        }
        assert!(n > 100, "{n} grains down the tail");
        assert!(side < 0.0, "the tail leads the comet: {side}");
    }

    #[test]
    fn cmt_a3_ion_tail_points_away_from_the_sun_within_8_degrees() {
        let g = run(5.0);
        let anti = g.anti_solar();
        let mut n = 0;
        for ion in g.ions.iter().filter(|i| i.attached && i.age > 0.3) {
            let p = g.ion_pos(ion);
            let d = [p[0] - g.nucleus[0], p[1] - g.nucleus[1]];
            let cos = (d[0] * anti[0] + d[1] * anti[1]) / d[0].hypot(d[1]);
            assert!(
                cos > (8.0f64 * PI / 180.0).cos_det(),
                "{} deg off",
                cos.acos().to_degrees()
            );
            n += 1;
        }
        assert!(n > 50);
    }

    #[test]
    fn cmt_a4_a_disconnection_empties_the_tail_root_then_it_refills() {
        let mut g = CometTails::new();
        let mut ctx = Ctx::new(7, Size::new(80, 24));
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        while g.t < g.next_disconnect - 0.1 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let cut = g.next_disconnect;
        let near = |g: &CometTails| {
            g.ions
                .iter()
                .filter(|i| {
                    let p = g.ion_pos(i);
                    (p[0] - g.nucleus[0]).hypot(p[1] - g.nucleus[1]) < 0.08
                })
                .count()
        };
        assert!(near(&g) > 10);
        while g.t < cut + 1.3 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if g.t > cut + 0.3 {
                assert_eq!(near(&g), 0, "root refilled at {}", g.t - cut);
            }
        }
        while g.t < cut + PAUSE + 1.0 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert!(near(&g) > 10, "tail did not regrow");
    }

    #[test]
    fn cmt_a5_pools_stay_within_capacity_and_beta_below_one() {
        let mut g = CometTails::new();
        let mut ctx = Ctx::new(5, Size::new(40, 12));
        g.resize(Size::new(40, 12), &mut ctx);
        g.prepare();
        let mut rng = SplitMix64::new(1);
        for _ in 0..600 {
            g.step(0.25, &mut rng);
            assert!(g.dust.len() <= DUST && g.ions.len() <= IONS);
            assert!(g.dust.iter().all(|gr| gr.beta < 1.0 && gr.beta >= BETA.0));
        }
    }

    /// Ion light (tone level) at `s` AU down the tail's axis, or `None` off frame.
    fn ion_level_at(g: &CometTails, s: f64) -> Option<u8> {
        let (x, y) = g.view.px(g.tail_point(s, (STREAMERS / 2) as u8, 0.0));
        let px = g.dust_glow.pixel_size();
        let (w, h) = (i32::from(px.width()), i32::from(px.height()));
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        if !((0..w).contains(&xi) && (0..h).contains(&yi)) {
            return None;
        }
        // The brightest of the 3×3 pixels round the axis point (the kink wanders).
        let vals = g.ion_glow.values();
        let mut best = 0.0f32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (a, b) = (xi + dx, yi + dy);
                if (0..w).contains(&a) && (0..h).contains(&b) {
                    best = best.max(vals[(b * w + a) as usize]);
                }
            }
        }
        Some(g.ion_tone.level(best))
    }

    #[test]
    fn cmt_a6_ion_tail_stays_rooted_at_the_coma() {
        // Regression (M13 review): a cut-off tail used to fill the parcel pool, so the new
        // tail could not regrow for seconds and a detached stub floated alone. Outside a
        // cut's first PAUSE + 1 s the attached tail is whole and its light runs unbroken
        // from the coma down the axis to 0.4 AU (or the frame edge).
        let mut g = CometTails::new();
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let mut cuts = vec![];
        let mut last_cut = f64::NEG_INFINITY;
        let mut checked = 0;
        for k in 0..(70.0 / SIM_DT) as usize {
            let before = g.next_disconnect;
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if g.next_disconnect != before {
                last_cut = g.t;
                cuts.push(g.t);
            }
            let tl = g.t.rem_euclid(LOOP);
            let quiet = g.t - last_cut > PAUSE + 1.0 && tl > 0.5 && tl < LOOP - 0.5;
            if k % 15 != 0 || !quiet {
                continue;
            }
            let attached = g.ions.iter().filter(|i| i.attached).count();
            assert!(
                attached as f64 >= 0.6 * ION_RATE * ION_LIFE.min(g.t - last_cut - PAUSE),
                "t {:.2}: {attached} attached parcels",
                g.t
            );
            for i in 1..=20 {
                let s = 0.03 + 0.37 * f64::from(i) / 20.0;
                match ion_level_at(&g, s) {
                    Some(l) => assert!(l > 20, "t {:.2}: gap at {s:.2} AU (level {l})", g.t),
                    None => break,
                }
            }
            checked += 1;
        }
        assert!(cuts.len() >= 3 && checked > 150, "{cuts:?} {checked}");
    }

    #[test]
    fn cmt_a7_ion_tail_is_streamers_fading_with_distance() {
        let g = run(3.0);
        // Fading: the axis light near the root outshines the light 1 AU down.
        let near = ion_level_at(&g, 0.1).expect("root on frame");
        if let Some(far) = ion_level_at(&g, 0.9) {
            assert!(near > far + 25, "root {near} vs 1 AU {far}");
        }
        // Streamers: across the tail at 0.6 AU, the ion light has at least two separate
        // peaks (distinct rays, not one smooth band).
        let anti = g.anti_solar();
        let s = 0.6;
        let c = g.tail_point(s, (STREAMERS / 2) as u8, 0.0);
        let px = g.ion_glow.pixel_size();
        let w = i32::from(px.width());
        let vals = g.ion_glow.values();
        let mut prof = vec![];
        for i in -40..=40 {
            let d = f64::from(i) * 0.002;
            let p = [c[0] - anti[1] * d, c[1] + anti[0] * d];
            let (x, y) = g.view.px(p);
            let (xi, yi) = (x.floor() as i32, y.floor() as i32);
            if xi < 0 || yi < 0 || xi >= w || yi >= i32::from(px.height()) {
                continue;
            }
            prof.push(vals[(yi * w + xi) as usize]);
        }
        let top = prof.iter().cloned().fold(0.0f32, f32::max);
        let mut peaks = 0;
        for i in 1..prof.len().saturating_sub(1) {
            if prof[i] > 0.3 * top && prof[i] > prof[i - 1] && prof[i] >= prof[i + 1] {
                peaks += 1;
            }
        }
        assert!(peaks >= 2, "one band: {prof:?}");
    }

    #[test]
    fn cmt_a8_no_sun_glow_and_stepped_m11_tails() {
        // ADR-0018: no Sun glow (it sat cut off in a corner); a white head, the ion rays
        // in flat blue steps (Cold, Wave), the dust tail in sun-yellow and pale steps.
        let ion: Vec<Ink> = [255, 100, 30].into_iter().filter_map(ion_ink).collect();
        let dust: Vec<Ink> = [255, 150, 80].into_iter().filter_map(dust_ink).collect();
        for t in [5.0, 12.0, 20.0] {
            let g = run(t);
            let px = g.pixels.pixel_size();
            let (w, h) = (i32::from(px.width()), i32::from(px.height()));
            let (nx, ny) = g.view.px(g.nucleus);
            let (mut n_ion, mut n_dust) = (0, 0);
            for y in 0..h {
                for x in 0..w {
                    let ink = g.pixels.get(x, y).unwrap_or(Ink::BLANK);
                    let coma =
                        (f64::from(x) + 0.5 - nx).hypot(f64::from(y) + 0.5 - ny) < COMA_PX + 0.5;
                    if ion.contains(&ink) {
                        n_ion += 1;
                    } else if dust.contains(&ink) {
                        n_dust += 1;
                    } else {
                        assert!(
                            coma || ink == Ink::BLANK || ink == STAR,
                            "t {t} ({x}, {y}): {ink:?}"
                        );
                    }
                }
            }
            assert!(
                n_ion > 20 && n_dust > 20,
                "t {t}: ion {n_ion} dust {n_dust}"
            );
            let head = g.pixels.get(nx as i32, ny as i32);
            assert_eq!(head, Some(Ink::new(Role::Star, 255)), "t {t}: nucleus");
        }
    }
}
