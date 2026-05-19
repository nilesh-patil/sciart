//! Shoemaker–Levy 9: a rubble-pile comet torn apart by Jupiter's tides, then the
//! fragments' impacts (SPEC-002 `comet_breakup`).
//!
//! The comet is 240 particles held together by softened self-gravity and soft, damped
//! contacts (a cohesionless rubble pile). Its centre of mass follows a parabolic orbit
//! round Jupiter with perijove at 1.33 R_J; the particles move in the comet's frame under
//! the linear tidal field of Jupiter (the Hill approximation), integrated by velocity
//! Verlet. Outside the Roche distance the pile holds; inside it the tide stretches it
//! into a chain that its own gravity gathers into clumps — a string of pearls, found by a
//! union-find over touching particles. Two years later (compressed to seconds) the
//! fragments strike one after another just behind Jupiter's limb: each raises a
//! ballistic fireball (the shared `plume` model) and leaves a dark scar that Jupiter's
//! rotation carries into view.
//!
//! Units: Jupiter radii for the orbit, pile radii for the comet (drawn magnified), and the
//! pile's dynamical time `τ` (`G·M_comet/ρ³ = 1`) for both.

use std::f64::consts::{FRAC_PI_2, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Tone, ease, ink};

use crate::shared::plume::Plume;
use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "comet_breakup",
    category: Category::SunPlanets,
    description: "Shoemaker-Levy 9: Jupiter's tides tear a comet into a string of pearls",
    title: "Comet breakup · tides pull apart",
    captions: &["Roche limit", "string of pearls", "impacts"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CometBreakup::new())
}

const ACTS: [(&str, f64); 5] = [
    ("approach", 6.0),
    ("Roche limit", 10.0),
    ("string of pearls", 10.0),
    ("impacts", 16.0),
    ("scars", 6.0),
];
const BREAKUP: usize = 1;
const PEARLS: usize = 2;
const IMPACTS: usize = 3;

/// Rubble pile: particles, their radius and softening (pile radii), contact stiffness and
/// damping (a contact lasts ≈ 0.08 τ).
pub(crate) const N: usize = 240;
const R_P: f64 = 0.062;
const SOFT: f64 = 0.1;
const K_CONTACT: f64 = 13.0;
const C_CONTACT: f64 = 0.1;
/// Jupiter: `G·M_J` in R_J³/τ², chosen so the Roche distance (tide = self-gravity at the
/// pile's edge, `3·GM_J/d³ = 1`) is 2.5 R_J; perijove at 1.33 R_J (as SL9 in 1992).
pub(crate) const GM_J: f64 = 2.5 * 2.5 * 2.5 / 3.0;
pub(crate) const ROCHE: f64 = 2.5;
const PERIJOVE: f64 = 1.33;
/// Direction of perijove on screen (rad, counter-clockwise from +x).
const ORBIT_TILT: f64 = 1.6;
/// Orbit time (τ, perijove at 0) at the start of the loop, and τ per simulated second in
/// each act.
const T_START: f64 = -3.0;
const RATES: [f64; 3] = [0.22, 0.45, 1.2];
/// Verlet sub-step (τ) and the most simulated τ one frame may advance.
const DT_SUB: f64 = 0.01;
const MAX_ADVANCE: f64 = 0.03;
/// Fragment table (BRK-A3): clusters of at least `MIN_FRAG` particles, largest first.
pub(crate) const FRAGMENTS: usize = 21;
const MIN_FRAG: usize = 3;
/// Drawn size of one pile radius, in R_J (the pile is ~10³× magnified).
const MAG: f64 = 0.16;
/// Impact plumes: capacity, height (R_J, exaggerated) and flight time (s).
const PLUME_CAP: usize = 3000;
const PLUME_H: f64 = 0.32;
const PLUME_FLIGHT: f64 = 3.0;
/// Length of the flash at each strike, seconds.
const FLASH: f64 = 0.8;
/// Jupiter's rotation rate on screen (time-lapsed), rad/s; the impacts' latitude.
pub(crate) const SPIN: f64 = 0.1;
const IMPACT_LAT: f64 = -0.5;

/// Centre-of-mass position (R_J) at orbit time `t` (τ, perijove at 0): a parabola with
/// perijove at angle `ORBIT_TILT`, moving counter-clockwise (y up).
pub(crate) fn orbit(t: f64) -> [f64; 2] {
    let m = t / (2.0 * PERIJOVE * PERIJOVE * PERIJOVE / GM_J).sqrt();
    let mut d = m.cbrt();
    for _ in 0..30 {
        d -= (d + d * d * d / 3.0 - m) / (1.0 + d * d);
    }
    let nu = 2.0 * d.atan();
    let r = 2.0 * PERIJOVE / (1.0 + nu.cos_det());
    let (s, c) = nu.sin_cos_det();
    // The orbit is turned so the comet comes in from the right of the frame.
    let (sr, cr) = ORBIT_TILT.sin_cos_det();
    [r * (c * cr - s * sr), r * (s * cr + c * sr)]
}

/// Tidal acceleration on relative position `rho` for a centre at `r` (Hill, linear):
/// `(GM/r³)·(3(r̂·ρ)r̂ − ρ)`.
fn tide(r: [f64; 2], rho: [f64; 2]) -> [f64; 2] {
    let d = r[0].hypot(r[1]);
    let k = GM_J / (d * d * d);
    let (ux, uy) = (r[0] / d, r[1] / d);
    let along = 3.0 * (ux * rho[0] + uy * rho[1]);
    [k * (along * ux - rho[0]), k * (along * uy - rho[1])]
}

/// Weighted union-find root with path halving.
fn root(parent: &mut [u16], mut i: usize) -> usize {
    while usize::from(parent[i]) != i {
        let p = usize::from(parent[i]);
        parent[i] = parent[p];
        i = usize::from(parent[i]);
    }
    i
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Fragment {
    /// Centre (pile units, comet frame), particle count, and distance along the chain.
    pub centre: [f64; 2],
    pub size: usize,
    pub along: f64,
}

#[derive(Debug, Clone, Copy)]
struct Scar {
    /// Longitude (rad) at impact, when it struck (s), size.
    lon: f64,
    born: f64,
    size: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct View {
    /// World point at the frame's focus pixel, pixels per R_J, frame size.
    wx: f64,
    wy: f64,
    fx: f64,
    fy: f64,
    scale: f64,
}

impl View {
    fn px(&self, p: [f64; 2]) -> (f64, f64) {
        (
            self.fx + self.scale * (p[0] - self.wx),
            self.fy - self.scale * (p[1] - self.wy),
        )
    }
}

#[derive(Debug)]
pub struct CometBreakup {
    t: f64,
    timeline: Timeline,
    cycle: i64,
    /// Orbit time of the pile (τ).
    tau: f64,
    pos: Box<[[f64; 2]]>,
    vel: Box<[[f64; 2]]>,
    acc: Box<[[f64; 2]]>,
    parent: Box<[u16]>,
    fragments: [Fragment; FRAGMENTS],
    n_frag: usize,
    /// Fragments that have struck, in order.
    struck: usize,
    impact_order: [usize; FRAGMENTS],
    plume: Plume,
    scars: [Scar; FRAGMENTS],
    n_scars: usize,
    seed: u64,
    frame: (f64, f64),
    view: View,
    glow: Glow,
    tone: Tone,
    /// Impact fireballs and flashes.
    fire: Glow,
    fire_tone: Tone,
    /// When the last fragment struck (s).
    last_strike: f64,
    pixels: Pixels,
}

impl CometBreakup {
    fn new() -> Self {
        let mut g = Self {
            t: 0.0,
            timeline: Timeline::new(&ACTS),
            cycle: 0,
            tau: T_START,
            pos: vec![[0.0; 2]; N].into_boxed_slice(),
            vel: vec![[0.0; 2]; N].into_boxed_slice(),
            acc: vec![[0.0; 2]; N].into_boxed_slice(),
            parent: (0..N as u16).collect(),
            fragments: [Fragment::default(); FRAGMENTS],
            n_frag: 0,
            struck: 0,
            impact_order: [0; FRAGMENTS],
            plume: Plume::with_capacity(PLUME_CAP),
            scars: [Scar {
                lon: 0.0,
                born: 0.0,
                size: 0.0,
            }; FRAGMENTS],
            n_scars: 0,
            seed: 0x5139,
            frame: (1.0, 1.0),
            view: View::default(),
            glow: Glow::default(),
            tone: Tone::asinh(0.5, 6.0),
            fire: Glow::default(),
            fire_tone: Tone::asinh(0.4, 3.0),
            last_strike: f64::NEG_INFINITY,
            pixels: Pixels::default(),
        };
        g.reset();
        g
    }

    /// A settled pile: hexagonal packing inside the unit disc, a little jitter.
    fn reset(&mut self) {
        let mut rng = SplitMix64::new(self.seed);
        // Candidates on a hexagonal lattice of touching grains; keep the N nearest the
        // centre (a fixed stack array: `reset` runs inside `update` at each loop).
        let sp = 2.0 * R_P;
        let mut cand = [[0.0f64; 2]; 41 * 41];
        for (k, c) in cand.iter_mut().enumerate() {
            let (i, row) = ((k % 41) as f64 - 20.0, (k / 41) as i32 - 20);
            let off = if row.rem_euclid(2) == 1 {
                0.5 * sp
            } else {
                0.0
            };
            *c = [i * sp + off, f64::from(row) * sp * 0.866];
        }
        cand.sort_unstable_by(|a, b| a[0].hypot(a[1]).total_cmp(&b[0].hypot(b[1])));
        for (p, c) in self.pos.iter_mut().zip(cand.iter()) {
            let j = 0.1 * R_P * (rng.next_f64() - 0.5);
            *p = [c[0] + j, c[1] - j];
        }
        // Remove the centre-of-mass offset.
        let c = centroid(&self.pos);
        for p in self.pos.iter_mut() {
            p[0] -= c[0];
            p[1] -= c[1];
        }
        self.vel.fill([0.0; 2]);
        self.tau = T_START;
        self.forces();
        self.n_frag = 0;
        self.struck = 0;
        self.n_scars = 0;
        self.plume.clear();
    }

    /// Self-gravity, contacts and the tide into `acc`.
    fn forces(&mut self) {
        let m = 1.0 / N as f64;
        let com = orbit(self.tau);
        for (a, p) in self.acc.iter_mut().zip(self.pos.iter()) {
            *a = tide(com, *p);
        }
        let (pos, vel, acc) = (&self.pos, &self.vel, &mut self.acc);
        for i in 0..N {
            let (pi, vi) = (pos[i], vel[i]);
            let mut ai = [0.0, 0.0];
            for j in i + 1..N {
                let d = [pos[j][0] - pi[0], pos[j][1] - pi[1]];
                let r2 = d[0] * d[0] + d[1] * d[1];
                let s2 = r2 + SOFT * SOFT;
                let mut f = m / (s2 * s2.sqrt());
                if r2 < 4.0 * R_P * R_P {
                    // Soft, damped contact along the line of centres.
                    let r = r2.sqrt().max(1e-9);
                    let overlap = 2.0 * R_P - r;
                    let dv = [vel[j][0] - vi[0], vel[j][1] - vi[1]];
                    let closing = (dv[0] * d[0] + dv[1] * d[1]) / r;
                    f -= (K_CONTACT * overlap - C_CONTACT * closing) / (m * r);
                }
                ai[0] += f * d[0];
                ai[1] += f * d[1];
                acc[j][0] -= f * d[0];
                acc[j][1] -= f * d[1];
            }
            acc[i][0] += ai[0];
            acc[i][1] += ai[1];
        }
    }

    fn advance(&mut self, dt_tau: f64) {
        let n = (dt_tau / DT_SUB).ceil().max(1.0);
        let h = dt_tau / n;
        for _ in 0..n as usize {
            for k in 0..N {
                self.vel[k][0] += 0.5 * h * self.acc[k][0];
                self.vel[k][1] += 0.5 * h * self.acc[k][1];
                self.pos[k][0] += h * self.vel[k][0];
                self.pos[k][1] += h * self.vel[k][1];
            }
            self.tau += h;
            self.forces();
            for k in 0..N {
                self.vel[k][0] += 0.5 * h * self.acc[k][0];
                self.vel[k][1] += 0.5 * h * self.acc[k][1];
            }
        }
    }

    /// Group touching particles (union-find) into fragments, largest `FRAGMENTS`, ordered
    /// along the chain's principal axis, leading end first.
    pub(crate) fn find_fragments(&mut self) {
        for (k, p) in self.parent.iter_mut().enumerate() {
            *p = k as u16;
        }
        let link = (2.6 * R_P) * (2.6 * R_P);
        for i in 0..N {
            for j in i + 1..N {
                let d = [
                    self.pos[j][0] - self.pos[i][0],
                    self.pos[j][1] - self.pos[i][1],
                ];
                if d[0] * d[0] + d[1] * d[1] < link {
                    let (a, b) = (root(&mut self.parent, i), root(&mut self.parent, j));
                    if a != b {
                        self.parent[a.max(b)] = a.min(b) as u16;
                    }
                }
            }
        }
        // Tally clusters by root without allocating: sizes in `acc` scratch (x = count,
        // y = unused), centres accumulated then divided.
        let mut sums = [[0.0f64; 3]; N];
        for i in 0..N {
            let r = root(&mut self.parent, i);
            sums[r][0] += self.pos[i][0];
            sums[r][1] += self.pos[i][1];
            sums[r][2] += 1.0;
        }
        self.n_frag = 0;
        let (axis, c) = principal_axis(&self.pos);
        for s in sums.iter().filter(|s| s[2] >= MIN_FRAG as f64) {
            let f = Fragment {
                centre: [s[0] / s[2], s[1] / s[2]],
                size: s[2] as usize,
                along: 0.0,
            };
            if self.n_frag < FRAGMENTS {
                self.fragments[self.n_frag] = f;
                self.n_frag += 1;
            } else if let Some(small) = (0..FRAGMENTS).min_by_key(|&k| self.fragments[k].size) {
                if self.fragments[small].size < f.size {
                    self.fragments[small] = f;
                }
            }
        }
        // Leading end: the direction of motion along the orbit.
        let v = {
            let (a, b) = (orbit(self.tau), orbit(self.tau + 0.01));
            [b[0] - a[0], b[1] - a[1]]
        };
        let sign = if axis[0] * v[0] + axis[1] * v[1] >= 0.0 {
            1.0
        } else {
            -1.0
        };
        for f in &mut self.fragments[..self.n_frag] {
            f.along = sign * ((f.centre[0] - c[0]) * axis[0] + (f.centre[1] - c[1]) * axis[1]);
        }
        self.fragments[..self.n_frag].sort_unstable_by(|a, b| b.along.total_cmp(&a.along));
        for (k, o) in self.impact_order.iter_mut().enumerate() {
            *o = k;
        }
    }

    /// Where fragment `k` is during the impacts act, in R_J: on a straight run-in to the
    /// impact site, spaced as along the chain; `None` once it has struck.
    fn run_in(&self, k: usize, local: f64) -> Option<[f64; 2]> {
        let lead = self.fragments.first()?.along;
        let gap = (lead - self.fragments[k].along).max(0.0);
        // Distance from the site in R_J: the chain arrives at 0.35 R_J/s.
        let d = 0.25 + 0.06 * gap - 0.35 * local;
        if d <= 0.0 {
            return None;
        }
        let site = impact_site();
        let dir = [0.8, 0.6];
        Some([site[0] + d * dir[0], site[1] + d * dir[1]])
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        self.t += dt;
        let cycle = self.timeline.cycle(self.t);
        if cycle != self.cycle {
            self.cycle = cycle;
            self.seed = rng.random();
            self.reset();
        }
        let beat = self.timeline.at(self.t);
        if beat.index <= PEARLS {
            let adv = (RATES[beat.index] * dt).min(MAX_ADVANCE);
            self.advance(adv);
        }
        if beat.index == PEARLS
            && beat.progress > 0.5
            && (self.n_frag == 0 || beat.local % 1.0 < dt)
        {
            self.find_fragments();
        }
        if beat.index == IMPACTS {
            if self.n_frag == 0 {
                self.find_fragments();
            }
            // Strike in order (BRK-A4): the leading fragment first.
            while self.struck < self.n_frag && self.run_in(self.struck, beat.local).is_none() {
                let size = (self.fragments[self.struck].size as f64 / N as f64).sqrt();
                let v0 = (4.0 * PLUME_H / PLUME_FLIGHT) as f32;
                for _ in 0..(40.0 + 400.0 * size) as usize {
                    self.plume.launch(rng, self.struck as u8, v0, (0.0, 0.7));
                }
                if self.n_scars < FRAGMENTS {
                    self.scars[self.n_scars] = Scar {
                        lon: 1.72,
                        born: self.t,
                        size: 0.05 + 0.2 * size,
                    };
                    self.n_scars += 1;
                }
                self.struck += 1;
                self.last_strike = self.t;
            }
        }
        // Fireball grains fly in display units (R_J, s): a vertical launch at `v0` rises
        // `PLUME_H` and falls back after `PLUME_FLIGHT`.
        self.plume.step(dt as f32, plume_g() as f32, |_| {});
    }

    fn scar_screen(&self, s: &Scar) -> Option<(f64, f64)> {
        let lon = s.lon - SPIN * (self.t - s.born);
        if lon.abs() >= FRAC_PI_2 {
            return None;
        }
        let sl = lon.sin_det();
        let (sb, cb) = IMPACT_LAT.sin_cos_det();
        Some(self.view.px([cb * sl, sb]))
    }

    fn update_view(&mut self) {
        let (w, h) = self.frame;
        let scale = 0.26 * h.min(0.6 * w);
        let beat = self.timeline.at(self.t);
        // Follow the chain out during the pearls act, then come home for the impacts.
        let com = orbit(self.tau);
        let follow = match beat.index {
            BREAKUP => ease::smoothstep(((beat.progress - 0.35) / 0.5).clamp(0.0, 1.0)),
            PEARLS => 1.0,
            _ => 0.0,
        };
        let home = [0.9, 0.0];
        self.view = View {
            wx: home[0] + follow * (com[0] - home[0]),
            wy: home[1] + follow * (com[1] - home[1]),
            fx: 0.4 * w,
            fy: 0.5 * h,
            scale,
        };
    }

    fn compose(&mut self) {
        self.update_view();
        let v = self.view;
        let beat = self.timeline.at(self.t);
        self.glow.clear();
        const COMA: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
        let com = orbit(self.tau);
        if beat.index <= PEARLS {
            for p in self.pos.iter() {
                let w = [com[0] + MAG * p[0], com[1] + MAG * p[1]];
                let (x, y) = v.px(w);
                self.glow.splat(2.0 * x, 2.0 * y, &COMA, 0.25);
            }
        } else if beat.index == IMPACTS {
            for k in self.struck..self.n_frag {
                if let Some(p) = self.run_in(k, beat.local) {
                    let (x, y) = v.px(p);
                    let w = 0.4 + 2.0 * (self.fragments[k].size as f64 / N as f64).sqrt();
                    self.glow.splat(2.0 * x, 2.0 * y, &COMA, w as f32);
                }
            }
        }
        // Jupiter with scars, then the comet glow, the plumes and the Roche circle.
        let size = self.pixels.pixel_size();
        let (pw, ph) = (i32::from(size.width()), i32::from(size.height()));
        let (jx, jy) = v.px([0.0, 0.0]);
        let jr = v.scale;
        let tone = &self.tone;
        let glow = self.glow.values();
        for y in 0..ph {
            for x in 0..pw {
                let i = (y * pw + x) as usize;
                let lv = tone.level(glow[i]);
                let (dx, dy) = (
                    (f64::from(x) + 0.5 - jx) / jr,
                    (f64::from(y) + 0.5 - jy) / jr,
                );
                let d2 = dx * dx + dy * dy;
                let ink = if lv > 14 {
                    ink::ramp(Ramp::Ice, f64::from(lv) / 255.0)
                } else if d2 < 1.0 {
                    // Jupiter: pale zones (Starlight) and tan-brown belts (Ember), with
                    // wavy, turbulent edges; limb-darkened.
                    let mu = (1.0 - d2).sqrt();
                    let wave = value_noise(0x3B1, 9.0 * dx + 0.6 * self.t, 5.0 * dy, 1.0) - 0.5;
                    let lat = -dy + 0.015 * wave;
                    let bands = (lat * 10.5 + 0.3).sin_det() + 0.35 * (lat * 23.0 + 1.0).sin_det();
                    let lit = 0.35 + 0.65 * mu.powf(0.5);
                    if bands > -0.45 {
                        ink::ramp(Ramp::Starlight, (0.6 + 0.12 * bands) * lit)
                    } else {
                        ink::ramp(Ramp::Ember, (0.42 + 0.05 * bands) * lit)
                    }
                } else if hash01(0x5E ^ cell_key(x, y)) < 0.0015 {
                    ink::ramp(Ramp::Starlight, 0.3)
                } else {
                    Ink::BLANK
                };
                self.pixels.set(x, y, ink);
            }
        }
        // The Roche distance, dashed, while the pile passes inside it.
        if beat.index == BREAKUP {
            let steps = (TAU * ROCHE * jr / 3.0) as usize;
            for k in (0..steps).step_by(2) {
                let a = TAU * k as f64 / steps as f64;
                let (s, c) = a.sin_cos_det();
                let (x, y) = v.px([ROCHE * c, ROCHE * s]);
                let (x, y) = (x.floor() as i32, y.floor() as i32);
                if self.pixels.get(x, y) == Some(Ink::BLANK) {
                    self.pixels.set(x, y, ink::ramp(Ramp::Starlight, 0.3));
                }
            }
        }
        // Scars: a dark core inside a dark crescent (the ejecta ring falls back on the
        // side away from the incoming fragment), carried round by the rotation (BRK-A5).
        // They darken the clouds under them rather than replacing them, so edges are soft.
        for s in &self.scars[..self.n_scars] {
            if let Some((sx, sy)) = self.scar_screen(s) {
                let lon = s.lon - SPIN * (self.t - s.born);
                let fore = lon.cos_det().max(0.05);
                let (rx, ry) = (1.6 * s.size * jr * fore, 1.6 * s.size * jr * 0.8);
                let (x0, x1) = ((sx - rx).floor() as i32, (sx + rx).ceil() as i32);
                let (y0, y1) = ((sy - ry).floor() as i32, (sy + ry).ceil() as i32);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let (ex, ey) = (
                            (f64::from(x) + 0.5 - sx) / rx.max(0.5),
                            (f64::from(y) + 0.5 - sy) / ry.max(0.5),
                        );
                        let e = (ex * ex + ey * ey).sqrt();
                        let core = (1.0 - e / 0.4).max(0.0);
                        let side = (0.5 + 0.5 * ey).clamp(0.0, 1.0);
                        let ring = (1.0 - ((e - 0.72) / 0.22).abs()).max(0.0) * side;
                        let dark = (0.9 * core.sqrt()).max(0.75 * ring);
                        let Some(under) = self.pixels.get(x, y) else {
                            continue;
                        };
                        if dark > 0.0
                            && matches!(under.role, Role::Ramp(Ramp::Starlight | Ramp::Ember))
                        {
                            let lv = f64::from(under.level) / 255.0 * (1.0 - dark);
                            let ramp = match under.role {
                                Role::Ramp(r) => r,
                                _ => Ramp::Starlight,
                            };
                            self.pixels.set(x, y, ink::ramp(ramp, lv.max(0.03)));
                        }
                    }
                }
            }
        }
        // Impact fireballs: a flash at each strike, then the plume's hot grains cooling
        // (temperature from the glow they make together), hidden behind the disc.
        // Nothing burning and no flash: skip the fireball pass entirely.
        let since = self.t - self.last_strike;
        if self.plume.iter().next().is_none() && !(0.0..FLASH).contains(&since) {
            return;
        }
        self.fire.clear();
        let site = impact_site();
        let up = [site[0], site[1]];
        let tangent = [-up[1], up[0]];
        const SPARK: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
        for gr in self.plume.iter() {
            let p = [
                site[0] + up[0] * f64::from(gr.pos[2]) + tangent[0] * f64::from(gr.pos[0]),
                site[1] + up[1] * f64::from(gr.pos[2]) + tangent[1] * f64::from(gr.pos[0]),
            ];
            if gr.pos[1] < 0.0 && p[0].hypot(p[1]) < 1.0 {
                continue;
            }
            let (x, y) = v.px(p);
            let heat = (-f64::from(gr.age) / 1.4).exp() as f32;
            self.fire.splat(2.0 * x, 2.0 * y, &SPARK, 0.6 * heat);
        }
        if (0.0..FLASH).contains(&since) {
            const FLASH_K: [(i32, i32, f32); 49] = halo_kernel::<49>(3);
            let (x, y) = v.px([site[0] * 1.03, site[1] * 1.03]);
            let k = (1.0 - since / FLASH) as f32;
            self.fire.splat(2.0 * x, 2.0 * y, &FLASH_K, 6.0 * k * k);
        }
        let fw = usize::from(self.fire.pixel_size().width()).max(1);
        for (i, &f) in self.fire.values().iter().enumerate() {
            let lv = self.fire_tone.level(f);
            if lv < 24 {
                continue;
            }
            let (x, y) = ((i % fw) as i32, (i / fw) as i32);
            let kelvin = 1500.0 + 6500.0 * f64::from(lv) / 255.0;
            self.pixels.set(x, y, ink::blackbody(kelvin));
        }
    }
}

/// Fireball gravity in R_J/s² for the drawn height and flight time.
fn plume_g() -> f64 {
    8.0 * PLUME_H / (PLUME_FLIGHT * PLUME_FLIGHT)
}

/// Where the fragments strike: just behind Jupiter's right limb (R_J).
fn impact_site() -> [f64; 2] {
    let (s, c) = 0.35f64.sin_cos_det();
    [c, s]
}

fn centroid(p: &[[f64; 2]]) -> [f64; 2] {
    let n = p.len().max(1) as f64;
    let s = p.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0], a[1] + q[1]]);
    [s[0] / n, s[1] / n]
}

/// The chain's principal axis (unit) and centroid.
fn principal_axis(p: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    let c = centroid(p);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for q in p {
        let (x, y) = (q[0] - c[0], q[1] - c[1]);
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
    }
    let a = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (s, co) = a.sin_cos_det();
    ([co, s], c)
}

#[cfg(test)]
/// Ratio of the long to the short principal axis (square roots of the covariance
/// eigenvalues).
pub(crate) fn axis_ratio(p: &[[f64; 2]]) -> f64 {
    let c = centroid(p);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for q in p {
        let (x, y) = (q[0] - c[0], q[1] - c[1]);
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
    }
    let tr = sxx + syy;
    let det = sxx * syy - sxy * sxy;
    let disc = (0.25 * tr * tr - det).max(0.0).sqrt();
    ((0.5 * tr + disc) / (0.5 * tr - disc).max(1e-12)).sqrt()
}

#[cfg(test)]
/// RMS radius of the pile.
pub(crate) fn rms_radius(p: &[[f64; 2]]) -> f64 {
    let c = centroid(p);
    let s: f64 = p
        .iter()
        .map(|q| (q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2))
        .sum();
    (s / p.len().max(1) as f64).sqrt()
}

impl Generator for CometBreakup {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.t == 0.0 && self.seed == 0x5139 {
            self.seed = ctx.rng.random();
            self.reset();
        }
        self.glow.resize(size);
        self.fire.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        self.frame = (f64::from(px.width()), f64::from(px.height()));
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

    fn caption(&self) -> Option<&'static str> {
        let beat = self.timeline.at(self.t);
        match beat.index {
            BREAKUP | PEARLS | IMPACTS => Some(beat.label),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (CometBreakup, SplitMix64) {
        let mut g = CometBreakup::new();
        let mut ctx = Ctx::new(42, Size::new(100, 30));
        g.resize(Size::new(100, 30), &mut ctx);
        (g, SplitMix64::new(9))
    }

    fn run_to(g: &mut CometBreakup, rng: &mut SplitMix64, t: f64) {
        while g.t < t {
            g.step(SIM_DT, rng);
        }
    }

    #[test]
    fn brk_a1_the_pile_holds_together_far_from_jupiter() {
        let (mut g, _) = fresh();
        // Far out on the orbit: let contacts settle, then compare while the pile is
        // beyond 3 Roche radii.
        g.tau = -16.0;
        g.advance(0.3);
        let r0 = rms_radius(&g.pos);
        while {
            let c = orbit(g.tau + 0.05);
            c[0].hypot(c[1]) > 3.0 * ROCHE
        } {
            g.advance(0.05);
            let r = rms_radius(&g.pos);
            assert!(
                (r / r0 - 1.0).abs() < 0.1,
                "radius {r0} -> {r} at tau {}",
                g.tau
            );
        }
        assert!(g.tau > -15.0, "never outside 3 Roche radii");
    }

    #[test]
    fn brk_a2_a_string_of_pearls_forms_after_perijove() {
        let (mut g, mut rng) = fresh();
        let end_pearls = ACTS[0].1 + ACTS[1].1 + ACTS[2].1 - 0.05;
        run_to(&mut g, &mut rng, end_pearls);
        g.find_fragments();
        assert!(g.n_frag >= 8, "{} fragments", g.n_frag);
        let ratio = axis_ratio(&g.pos);
        assert!(ratio >= 5.0, "axis ratio {ratio}");
    }

    #[test]
    fn brk_a3_pool_and_fragment_table_stay_within_capacity() {
        let (mut g, mut rng) = fresh();
        let total: f64 = ACTS.iter().map(|a| a.1).sum();
        while g.t < total + 5.0 {
            g.step(0.25, &mut rng);
            assert!(g.n_frag <= FRAGMENTS && g.n_scars <= FRAGMENTS);
            assert!(g.plume.iter().count() <= PLUME_CAP);
        }
    }

    #[test]
    fn brk_a4_fragments_strike_in_order() {
        let (mut g, mut rng) = fresh();
        let impacts = ACTS[0].1 + ACTS[1].1 + ACTS[2].1;
        run_to(&mut g, &mut rng, impacts + 0.05);
        let n = g.n_frag;
        assert!(n >= 2);
        let mut times = [f64::NAN; FRAGMENTS];
        let mut seen = 0;
        while g.t < impacts + ACTS[3].1 - 0.1 {
            g.step(SIM_DT, &mut rng);
            while seen < g.struck {
                times[seen] = g.t;
                seen += 1;
            }
        }
        assert_eq!(seen, n, "every fragment strikes");
        for k in 1..n {
            assert!(times[k] >= times[k - 1]);
            assert!(
                g.fragments[k].along <= g.fragments[k - 1].along,
                "leading first"
            );
        }
    }

    #[test]
    fn brk_a5_scars_turn_with_jupiter() {
        let (mut g, mut rng) = fresh();
        let impacts = ACTS[0].1 + ACTS[1].1 + ACTS[2].1;
        run_to(&mut g, &mut rng, impacts + ACTS[3].1 - 0.5);
        assert!(g.n_scars > 0);
        g.update_view();
        let first = g.scars[0];
        let a = g.scar_screen(&first).map(|p| p.0);
        let later = g.t + 1.0;
        run_to(&mut g, &mut rng, later);
        g.update_view();
        let b = g.scar_screen(&first).map(|p| p.0);
        let (a, b) = (a.expect("scar in view"), b.expect("scar in view"));
        assert!(b < a - 0.5, "scar did not move: {a} -> {b}");
    }
}
