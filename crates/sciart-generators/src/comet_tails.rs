//! A bright comet rounding the Sun, with a curved dust tail and a straight ion tail
//! (SPEC-002 `comet_tails`).
//!
//! Dust grains leave the nucleus with small velocity spreads and then feel the Sun's
//! gravity reduced by radiation pressure, `μ(1 − β)`, with β log-uniform in
//! `[0.01, 0.9]`. Each grain follows its own exact two-body orbit (elliptic or, for most
//! grains released near perihelion, hyperbolic), propagated in closed form with the
//! universal-variable Kepler solution warm-started from the previous frame; grains of one
//! β trace a syndyne, grains of one release time a synchrone, and together they fan into
//! a curved tail that lags behind the orbit. The ion tail is kinematic: plasma parcels
//! accelerated straight away from the Sun with small solar-wind kinks; every so often
//! the tail disconnects at the coma, drifts off, and a new one regrows.
//!
//! Units: AU and years inside the physics (`μ_☉ = 4π²`), one simulated second = 1.7 days.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::particles::ParticlePool;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Tone, ink};

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
pub(crate) const IONS: usize = 1500;
/// Dust release per simulated second at 1 AU; the rate goes as r⁻².
const DUST_RATE: f64 = 70.0;
/// Grains are dropped after this many simulated seconds or beyond this distance (AU).
const DUST_LIFE: f64 = 20.0;
const DUST_REACH: f64 = 6.0;
/// Ion-tail acceleration (AU/s² in simulated seconds), parcel life and kink amplitude
/// as a share of the distance down the tail.
const ION_ACCEL: f64 = 0.16;
const ION_LIFE: f64 = 5.0;
const ION_RATE: f64 = 280.0;
const KINK: f64 = 0.035;
/// A disconnection every `DISCONNECT` seconds, with release paused for `PAUSE` seconds.
pub(crate) const DISCONNECT: f64 = 20.0;
pub(crate) const PAUSE: f64 = 1.6;
/// Anti-solar speed the solar wind gives a disconnected tail, AU per simulated second.
const DETACH_KICK: f64 = 0.3;
/// Kepler solve: Newton iterations per frame (warm start) and from scratch.
const NEWTON_WARM: usize = 3;

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
    /// Kink phase.
    phase: f64,
    /// Detached parcels fly free from `pos` with `vel` (AU, AU/s).
    attached: bool,
    pos: [f64; 2],
    vel: [f64; 2],
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
    ion_glow: Glow,
    dust_tone: Tone,
    ion_tone: Tone,
    pixels: Pixels,
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
            next_disconnect: 0.6 * DISCONNECT,
            view: View::default(),
            dust_glow: Glow::default(),
            ion_glow: Glow::default(),
            dust_tone: Tone::asinh(0.12, 7.0),
            ion_tone: Tone::asinh(0.3, 8.0),
            pixels: Pixels::default(),
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
            // Activity ∝ r⁻², as in `release_dust`.
            if rng.random::<f64>() < (0.2 / (r * r.sqrt())).min(1.0) {
                self.emit(rng, self.t - tau, n, v);
            }
        }
        let parcels = (ION_RATE * ION_LIFE) as usize;
        for k in 0..parcels.min(IONS) {
            self.ions.spawn(Ion {
                age: ION_LIFE * k as f64 / parcels as f64,
                phase: rng.random_range(0.0..TAU),
                attached: true,
                pos: self.nucleus,
                vel: [0.0; 2],
            });
        }
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        let before = (self.t / LOOP).floor();
        self.t += dt;
        if (self.t / LOOP).floor() != before {
            let (n, v) = comet_state(anomaly(self.t.rem_euclid(LOOP)));
            self.nucleus = n;
            self.velocity = v;
            self.seed_backlog(rng);
        }
        let (n, v) = comet_state(anomaly(self.t.rem_euclid(LOOP)));
        self.nucleus = n;
        self.velocity = v;
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
            let (nucleus, cv) = (self.nucleus, self.velocity);
            for ion in self.ions.iter_mut().filter(|i| i.attached) {
                let s = 0.5 * ION_ACCEL * ion.age * ion.age;
                // The cut-off plasmoid is swept away by the solar wind.
                let sp = ION_ACCEL * ion.age + DETACH_KICK;
                ion.pos = [nucleus[0] + anti[0] * s, nucleus[1] + anti[1] * s];
                ion.vel = [cv[0] * yps + anti[0] * sp, cv[1] * yps + anti[1] * sp];
                ion.attached = false;
            }
        }
        for ion in self.ions.iter_mut() {
            ion.age += dt;
            if !ion.attached {
                let r = ion.pos[0].hypot(ion.pos[1]).max(1e-6);
                let a = [ion.pos[0] / r * ION_ACCEL, ion.pos[1] / r * ION_ACCEL];
                ion.vel = [ion.vel[0] + a[0] * dt, ion.vel[1] + a[1] * dt];
                ion.pos = [ion.pos[0] + ion.vel[0] * dt, ion.pos[1] + ion.vel[1] * dt];
            }
        }
        self.ions
            .retain(|i| i.age < if i.attached { ION_LIFE } else { 2.0 * ION_LIFE });
        if self.t >= self.ion_resume {
            self.owed_ions += ION_RATE * dt;
            while self.owed_ions >= 1.0 {
                self.owed_ions -= 1.0;
                let ion = Ion {
                    age: 0.0,
                    phase: rng.random_range(0.0..TAU),
                    attached: true,
                    pos: self.nucleus,
                    vel: [0.0; 2],
                };
                if !self.ions.spawn(ion) {
                    self.owed_ions = 0.0;
                    break;
                }
            }
        }
    }

    /// Where an attached parcel is: down the anti-solar axis, kinked a little sideways.
    fn ion_pos(&self, ion: &Ion) -> [f64; 2] {
        if !ion.attached {
            return ion.pos;
        }
        let anti = self.anti_solar();
        let s = 0.5 * ION_ACCEL * ion.age * ion.age + 0.01 * ion.age;
        let kink = KINK * s * (TAU * 0.35 * self.t + 3.0 * s + ion.phase).sin_det();
        [
            self.nucleus[0] + anti[0] * s - anti[1] * kink,
            self.nucleus[1] + anti[1] * s + anti[0] * kink,
        ]
    }

    /// Follow the comet: the nucleus sits off-centre towards the Sun, so the tails
    /// have the frame; the view keeps its orientation, so the tails swing round.
    fn follow(&mut self) {
        let anti = self.anti_solar();
        let v = &mut self.view;
        let off = 0.32 * v.w.min(2.0 * v.h);
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
        self.dust_glow.clear();
        self.ion_glow.clear();
        for g in self.dust.iter() {
            let n = self.nucleus;
            let (x, y) = v.px([
                n[0] + TAIL_EXAG * (g.pos[0] - n[0]),
                n[1] + TAIL_EXAG * (g.pos[1] - n[1]),
            ]);
            // Fine grains (large β) scatter more light per unit mass and fill the fan.
            let w = (0.5 + g.beta) * (1.0 - (self.t - g.born) / DUST_LIFE).max(0.0);
            self.dust_glow
                .splat(2.0 * x, 2.0 * y, &SOFT, 0.45 * w as f32);
        }
        for ion in self.ions.iter() {
            let (x, y) = v.px(self.ion_pos(ion));
            let fade = if ion.attached {
                1.0
            } else {
                (1.0 - ion.age / (2.0 * ION_LIFE)).max(0.0)
            };
            self.ion_glow
                .splat(2.0 * x, 2.0 * y, &SOFT, 0.6 * fade as f32);
        }
        // Coma: a bright, compact halo round the nucleus.
        const COMA: [(i32, i32, f32); 49] = halo_kernel::<49>(3);
        let (nx, ny) = v.px(self.nucleus);
        self.dust_glow.splat(2.0 * nx, 2.0 * ny, &COMA, 30.0);

        let (dt, it) = (&self.dust_tone, &self.ion_tone);
        let size = self.pixels.pixel_size();
        let w = usize::from(size.width());
        let (dust, ions) = (self.dust_glow.values(), self.ion_glow.values());
        // The Sun just off the left edge: a warm glow, hotter toward the edge.
        let r_sun = self.nucleus[0].hypot(self.nucleus[1]).max(0.1);
        let glow_r = 0.3 * v.h * (0.35 / r_sun).sqrt();
        for (i, px) in (0..dust.len().min(ions.len())).zip(0..) {
            let (x, y) = ((px % w) as i32, (px / w) as i32);
            let (ld, li) = (dt.level(env * dust[i]), it.level(env * ions[i]));
            let ink = if li > 20 && u16::from(li) + 30 > u16::from(ld) {
                ink::ramp(Ramp::Ice, f64::from(li) / 255.0)
            } else if ld > 12 {
                ink::ramp(Ramp::Starlight, f64::from(ld) / 255.0)
            } else {
                let d = (f64::from(x) + 0.5 - v.sx).hypot(f64::from(y) + 0.5 - v.sy);
                // The Sun: a white-hot core, then scattered light falling off smoothly
                // (Ember to black) rather than ending in a hard rim.
                let q = d / glow_r;
                let sun = (-7.0 * q * q).exp();
                if q < 0.18 {
                    ink::blackbody(5800.0 - 6000.0 * q)
                } else if sun * 255.0 * 0.9 >= ink::MIN_VISIBLE_LEVEL {
                    ink::ramp(Ramp::Ember, 0.9 * sun)
                } else if hash01(0x57A ^ cell_key(x, y)) < 0.0015 {
                    ink::ramp(Ramp::Starlight, 0.3 + 0.3 * hash01(0x57B ^ cell_key(x, y)))
                } else {
                    Ink::BLANK
                }
            };
            self.pixels.set(x, y, ink);
        }
    }
}

impl Generator for CometTails {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.dust_glow.resize(size);
        self.ion_glow.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        // The orbit's visible arc spans ±0.75 AU vertically; the Sun sits just off the
        // left edge so the tails stream across the frame.
        let scale = (0.5 * w).min(1.1 * h).max(1.0);
        self.view = View {
            sx: 0.0,
            sy: 0.0,
            scale,
            w,
            h,
        };
        if self.t == 0.0 {
            // Arrive with grown tails, a few seconds into the approach.
            self.seed_backlog(&mut ctx.rng);
            for _ in 0..(4.0 / SIM_DT) as usize {
                self.step(SIM_DT, &mut ctx.rng);
            }
        }
        self.compose();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt, &mut ctx.rng);
        }
        self.compose();
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

    fn run(seconds: f64) -> CometTails {
        let mut g = CometTails::new();
        let mut ctx = Ctx::new(42, Size::new(100, 30));
        g.resize(Size::new(100, 30), &mut ctx);
        let mut t = 0.0;
        while t < seconds {
            g.update(SIM_DT, &mut ctx);
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
    fn cmt_a3_ion_tail_points_away_from_the_sun() {
        let g = run(5.0);
        let anti = g.anti_solar();
        let mut n = 0;
        for ion in g.ions.iter().filter(|i| i.attached && i.age > 0.3) {
            let p = g.ion_pos(ion);
            let d = [p[0] - g.nucleus[0], p[1] - g.nucleus[1]];
            let cos = (d[0] * anti[0] + d[1] * anti[1]) / d[0].hypot(d[1]);
            assert!(
                cos > (5.0f64 * PI / 180.0).cos_det(),
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
        while g.t < g.next_disconnect - 0.1 {
            g.update(SIM_DT, &mut ctx);
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
            if g.t > cut + 0.3 {
                assert_eq!(near(&g), 0, "root refilled at {}", g.t - cut);
            }
        }
        while g.t < cut + PAUSE + 1.0 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(near(&g) > 10, "tail did not regrow");
    }

    #[test]
    fn cmt_a5_pools_stay_within_capacity_and_beta_below_one() {
        let mut g = CometTails::new();
        let mut ctx = Ctx::new(5, Size::new(40, 12));
        g.resize(Size::new(40, 12), &mut ctx);
        let mut rng = SplitMix64::new(1);
        for _ in 0..600 {
            g.step(0.25, &mut rng);
            assert!(g.dust.len() <= DUST && g.ions.len() <= IONS);
            assert!(g.dust.iter().all(|gr| gr.beta < 1.0 && gr.beta >= BETA.0));
        }
    }
}
