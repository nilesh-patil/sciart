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

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Tone, ease};

use crate::shared::jovian;
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

/// The approach is short, so the tide is already visibly stretching the pile by t ≈ 5 s
/// (BRK-A6).
const ACTS: [(&str, f64); 5] = [
    ("approach", 2.0),
    ("Roche limit", 12.0),
    ("string of pearls", 12.0),
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
const T_START: f64 = -2.6;
const RATES: [f64; 3] = [0.7, 0.9, 1.1];
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
/// How long the radial glow at each strike lasts, seconds.
const FLASH: f64 = 2.5;
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
    /// Nuclei and dust tails, and the ion tails (toned, then stepped).
    glow: Glow,
    tone: Tone,
    ion: Glow,
    ion_tone: Tone,
    /// Impact fireballs and flashes.
    fire: Glow,
    fire_tone: Tone,
    /// When the last fragment struck (s).
    last_strike: f64,
    pixels: Pixels,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
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
            ion: Glow::default(),
            ion_tone: Tone::asinh(0.3, 3.0),
            fire: Glow::default(),
            fire_tone: Tone::asinh(0.4, 3.0),
            last_strike: f64::NEG_INFINITY,
            pixels: Pixels::default(),
            dirty: false,
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
        // Spacing on the run-in: the chain's own, shrunk if needed so the last fragment
        // strikes by 13 s into the act (BRK-A4).
        let span = lead - self.fragments[self.n_frag.max(1) - 1].along;
        let spacing = 0.06f64.min(4.3 / span.max(1e-9));
        // Distance from the site in R_J: the chain arrives at 0.35 R_J/s.
        let d = 0.25 + spacing * gap - 0.35 * local;
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
        // Half-way between Jupiter and the comet on the approach, so both are in frame
        // from the first second; onto the pile early in the breakup, where the stretching
        // shows (BRK-A6).
        let follow = match beat.index {
            0 => FOLLOW_APPROACH,
            BREAKUP => {
                let u = ease::smoothstep((beat.progress / 0.3).clamp(0.0, 1.0));
                FOLLOW_APPROACH + (1.0 - FOLLOW_APPROACH) * u
            }
            PEARLS => 1.0,
            _ => 0.0,
        };
        let home = [0.9, 0.0];
        // While following, zoom out as the chain lengthens so the whole string of pearls
        // stays in frame (never below 40 % of the home scale).
        let reach = self
            .pos
            .iter()
            .map(|p| p[0].hypot(p[1]))
            .fold(1.0, f64::max);
        let fit = 0.46 * w.min(1.8 * h) / (MAG * reach);
        let scale = scale + follow * (fit.clamp(0.4 * scale, scale) - scale);
        self.view = View {
            wx: home[0] + follow * (com[0] - home[0]),
            wy: home[1] + follow * (com[1] - home[1]),
            fx: (0.4 + 0.1 * follow) * w,
            fy: 0.5 * h,
            scale,
        };
    }

    fn compose(&mut self) {
        self.update_view();
        let v = self.view;
        let beat = self.timeline.at(self.t);
        self.glow.clear();
        self.ion.clear();
        const CORE: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
        const COMA: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
        let com = orbit(self.tau);
        // Tails behind every few grains (so each fragment trails its own), sampled once per
        // pixel of length and fading with distance: a broad, pale dust tail, and a longer,
        // straighter, fainter ion tail pointing straight away from the Sun.
        let to_px = |dir: [f64; 2], len: f64| [dir[0] * len * v.scale, -dir[1] * len * v.scale];
        let (dust_px, ion_px) = (to_px(TAIL_DIR, TAIL_LEN), to_px(ION_DIR, ION_LEN));
        let steps = |p: [f64; 2]| (p[0].hypot(p[1]).ceil() as usize).clamp(2, 256);
        let tail =
            |glow: &mut Glow, kernel: &[(i32, i32, f32)], to: [f64; 2], x: f64, y: f64, w: f64| {
                let n = steps(to);
                for j in 1..=n {
                    let s = j as f64 / n as f64;
                    let k = w * (1.0 - s).powf(1.5) / n as f64 * 14.0;
                    let (tx, ty) = (x + s * to[0], y + s * to[1]);
                    glow.splat(2.0 * tx, 2.0 * ty, kernel, k as f32);
                }
            };
        if beat.index <= PEARLS {
            for (k, p) in self.pos.iter().enumerate() {
                let w = [com[0] + MAG * p[0], com[1] + MAG * p[1]];
                let (x, y) = v.px(w);
                // Nucleus grains: tight, so only the dense core saturates.
                self.glow.splat(2.0 * x, 2.0 * y, &CORE, 0.14);
                if k % TAIL_EVERY == 0 {
                    tail(&mut self.glow, &CORE, dust_px, x, y, 0.04);
                    tail(&mut self.ion, &CORE, ion_px, x, y, 0.03);
                }
            }
        } else if beat.index == IMPACTS {
            for k in self.struck..self.n_frag {
                if let Some(p) = self.run_in(k, beat.local) {
                    let (x, y) = v.px(p);
                    let w = 0.4 + 2.0 * (self.fragments[k].size as f64 / N as f64).sqrt();
                    self.glow.splat(2.0 * x, 2.0 * y, &CORE, 0.8 * w as f32);
                    tail(&mut self.glow, &CORE, dust_px, x, y, 0.25 * w);
                    tail(&mut self.ion, &CORE, ion_px, x, y, 0.12 * w);
                }
            }
        }
        // Jupiter with scars, then the comet glow, the plumes and the Roche circle.
        let size = self.pixels.pixel_size();
        let (pw, ph) = (i32::from(size.width()), i32::from(size.height()));
        let (jx, jy) = v.px([0.0, 0.0]);
        let jr = v.scale;
        let (tone, ion_tone) = (&self.tone, &self.ion_tone);
        let (glow, ion) = (self.glow.values(), self.ion.values());
        for y in 0..ph {
            for x in 0..pw {
                let i = (y * pw + x) as usize;
                let lv = tone.level(glow[i]);
                let li = ion_tone.level(ion[i]);
                let (dx, dy) = (
                    (f64::from(x) + 0.5 - jx) / jr,
                    (f64::from(y) + 0.5 - jy) / jr,
                );
                let d2 = dx * dx + dy * dy;
                // M11 idiom (ADR-0018): white nuclei, a stepped yellow dust tail, the ion
                // tail in flat blue steps beneath it; no halo, no wash.
                let ink = if let Some(dust) = dust_ink(lv) {
                    dust
                } else if let Some(ion) = ion_ink(li) {
                    ion
                } else if d2 < 1.0 {
                    jupiter_cloud(dx, dy, d2, jr, self.t)
                } else if hash01(0x5E ^ cell_key(x, y)) < 0.0015 {
                    STAR
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
                    self.pixels.set(x, y, Ink::new(Role::Dim, 140));
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
                        // Scars step the clouds down (the red spot too), never the stars
                        // or the comet.
                        if dark > 0.25 && jovian::is_cloud(under) {
                            let n = if dark > 0.6 { 3 } else { 1 };
                            self.pixels.set(x, y, jovian::darker(under, n));
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
            // Young grains are all at the vent: ramp them in so they do not sum to a
            // saturated blob before the plume has spread.
            let spread = ease::smoothstep((f64::from(gr.age) / 0.4).min(1.0)) as f32;
            self.fire
                .splat(2.0 * x, 2.0 * y, &COMA, 0.035 * heat * spread);
        }
        let fw = usize::from(self.fire.pixel_size().width()).max(1);
        if (0.0..FLASH).contains(&since) {
            // A soft radial glow over the limb at the strike, bright at first and fading
            // (a Gaussian drawn directly, so it has no kernel's stepped edge).
            let (cx, cy) = v.px([site[0] * 1.03, site[1] * 1.03]);
            let r0 = (GLOW_R * v.scale).max(1.5);
            let k = 1.0 * (-since / GLOW_FADE).exp() * (1.0 - since / FLASH);
            let reach = 2.2 * r0;
            let fh = self.fire.values().len() / fw;
            let (x0, x1) = ((cx - reach).floor().max(0.0) as usize, (cx + reach).ceil());
            let (y0, y1) = ((cy - reach).floor().max(0.0) as usize, (cy + reach).ceil());
            let buf = self.fire.values_mut();
            for y in y0..(y1.max(0.0) as usize).min(fh) {
                for x in x0..(x1.max(0.0) as usize).min(fw) {
                    let (ddx, ddy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                    let g = (-(ddx * ddx + ddy * ddy) / (r0 * r0)).exp();
                    buf[y * fw + x] += (k * g) as f32;
                }
            }
        }
        // The fireball in flat M11 steps (ADR-0018): a white heart, then sun-yellow, then
        // hot orange; over the clouds a faint glow lifts them one step instead.
        for (i, &f) in self.fire.values().iter().enumerate() {
            let lv = self.fire_tone.level(f);
            if lv < FIRE_EDGE {
                continue;
            }
            let (x, y) = ((i % fw) as i32, (i / fw) as i32);
            let under = self.pixels.get(x, y).unwrap_or(Ink::BLANK);
            let ink = if lv >= FIRE_HEART {
                Ink::new(Role::Star, 255)
            } else if jovian::is_cloud(under) && lv < GLOW_ON_CLOUD {
                jovian::lighter(under)
            } else if lv >= 170 {
                Ink::new(Role::Sun, 220)
            } else if lv >= 90 {
                Ink::new(Role::Hot, 200)
            } else {
                Ink::new(Role::Hot, 70)
            };
            self.pixels.set(x, y, ink);
        }
        // The strike itself, a small white point at the limb for its first
        // `STRIKE_WHITE` s, the frame's brightest accent.
        if (0.0..STRIKE_WHITE).contains(&since) {
            let (cx, cy) = v.px([site[0] * 1.03, site[1] * 1.03]);
            let (x0, y0) = (cx.floor() as i32, cy.floor() as i32);
            for (dx, dy) in [(0, 0), (-1, 0), (0, -1), (-1, -1)] {
                self.pixels.set(x0 + dx, y0 + dy, Ink::new(Role::Star, 255));
            }
        }
    }
}

/// How long the strike's white point shows (s).
const STRIKE_WHITE: f64 = 0.6;

/// The Great Red Spot on the disc: planet longitude (rad), sine latitude, half-widths.
const GRS_LON: f64 = 0.9;
const GRS_S: f64 = -0.37;
const GRS_HALF: (f64, f64) = (0.24, 0.07);

/// Jupiter's clouds at disc point `(dx, dy)` (disc radii, `y` down, `d2 = dx² + dy²` < 1),
/// `jr` pixels per radius, `t` seconds: the shared belt/zone profile (the same stepped
/// palette as `jupiter_storms`), its edges made wavy and mottled by value noise fixed in
/// planet longitude so the texture turns with the planet (and the scars), the Great Red
/// Spot, limb darkening, and the outermost pixel antialiased into space.
fn jupiter_cloud(dx: f64, dy: f64, d2: f64, jr: f64, t: f64) -> Ink {
    let mu = (1.0 - d2).sqrt();
    let s = (-dy).clamp(-1.0, 1.0);
    let cl = (1.0 - s * s).max(1e-9).sqrt();
    let lon = (dx / cl).clamp(-1.0, 1.0).asin() + SPIN * t;
    let turb = value_noise(0x3B1, 2.5 * lon, 12.0 * s, 1.0) - 0.5;
    let fine = value_noise(0x3B2, 7.0 * lon, 45.0 * s, 1.0) - 0.5;
    let a = jovian::band_profile(s + 0.035 * turb) + 0.22 * fine;
    // Limb darkening all the way down (no floor), so the edge sinks into dark sienna and
    // then fades into space over its outer 1.5 pixels: no bright or orange rim.
    let aa = ease::smoothstep(((1.0 - d2.sqrt()) * jr / 1.5).min(1.0));
    let lit = mu.powf(0.9) * aa;
    let mut level = jovian::cloud_level(a, 0.0);
    // The Great Red Spot (a small oval in its collar, shared look with jupiter_storms).
    let dl = (lon - GRS_LON + PI).rem_euclid(TAU) - PI;
    let (sx, sy) = (dl / GRS_HALF.0, (s - GRS_S) / GRS_HALF.1);
    let rho = sx.hypot(sy);
    if rho < 2.0 {
        // At this scale the spot is a few pixels: feather in its own units (rim over
        // 0.7–1.3 radii, a faint collar out to 1.9) rather than in pixels.
        let inside = 1.0 - ease::smoothstep(((rho - 0.7) / 0.6).clamp(0.0, 1.0));
        let collar =
            0.6 * (1.0 - inside) * (1.0 - ease::smoothstep(((rho - 1.2) / 0.7).clamp(0.0, 1.0)));
        let spot = jovian::spot_level(0.5 + fine, a, 0.0);
        level = jovian::spot_blend(level, spot, inside, collar);
    }
    jovian::cloud_ink(level, lit)
}

/// How far the view leans from Jupiter towards the comet during the approach.
const FOLLOW_APPROACH: f64 = 0.55;
/// Dust tails: direction (unit, world, away from a Sun off to the lower left), length in
/// R_J, samples per pixel of length, and one tail per `TAIL_EVERY` grains.
const TAIL_DIR: [f64; 2] = [0.6, 0.8];
const TAIL_LEN: f64 = 0.8;
const TAIL_EVERY: usize = 3;
/// Ion tail: straight away from the Sun (unit, world) and length (R_J).
const ION_DIR: [f64; 2] = [0.2, 0.98];
const ION_LEN: f64 = 1.4;
/// A background star (M11: small white points, dim).
const STAR: Ink = Ink::new(Role::Star, 40);

/// Nuclei and dust in flat steps by tone level: white where grains crowd, pale then
/// deep sun-yellow down the dust tail, nothing below the faintest step.
fn dust_ink(lv: u8) -> Option<Ink> {
    match lv {
        170.. => Some(Ink::new(Role::Star, 255)),
        100.. => Some(Ink::new(Role::Star, 90)),
        50.. => Some(Ink::new(Role::Sun, 110)),
        22.. => Some(Ink::new(Role::Hot, 40)),
        _ => None,
    }
}

/// The ion tail in two flat blue steps (it stays fainter than the dust).
fn ion_ink(lv: u8) -> Option<Ink> {
    match lv {
        110.. => Some(Ink::new(Role::Cold, 120)),
        40.. => Some(Ink::new(Role::Cold, 20)),
        _ => None,
    }
}
/// The impact's radial glow: radius (R_J) and length of the flash (s, e-folding).
const GLOW_R: f64 = 0.16;
const GLOW_FADE: f64 = 0.7;
/// Below this tone level a glow over the clouds lightens them instead of replacing them.
const GLOW_ON_CLOUD: u8 = 140;
/// Tone level from which the glow is drawn white (its heart), and its faintest step.
const FIRE_HEART: u8 = 238;
const FIRE_EDGE: u8 = 30;

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
        self.ion.resize(size);
        self.fire.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        self.frame = (f64::from(px.width()), f64::from(px.height()));
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
        g.prepare();
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
    fn brk_a6_the_tide_visibly_stretches_the_pile_by_five_seconds() {
        let (mut g, mut rng) = fresh();
        run_to(&mut g, &mut rng, 5.0);
        let ratio = axis_ratio(&g.pos);
        assert!(ratio >= 2.0, "axis ratio {ratio} at t = 5 s");
    }

    #[test]
    fn brk_a6_the_comet_has_a_coma_and_tail_from_the_start() {
        let (mut g, mut rng) = fresh();
        run_to(&mut g, &mut rng, 0.1);
        g.compose();
        let v = g.view;
        let (nx, ny) = v.px(orbit(g.tau));
        let d = 0.45 * TAIL_LEN * v.scale;
        let at = |sx: f64, sy: f64| {
            g.pixels
                .get((nx + sx * d).floor() as i32, (ny - sy * d).floor() as i32)
                .unwrap_or(Ink::BLANK)
        };
        // A stepped sun-yellow/white dust tail away from the Sun (M11 roles, ADR-0018),
        // none (or only its faintest step) on the sunward side.
        let tail = at(TAIL_DIR[0], TAIL_DIR[1]);
        assert!(
            matches!(tail.role, Role::Sun | Role::Star | Role::Hot),
            "no dust tail: {tail:?}"
        );
        let sunward = at(-TAIL_DIR[0], -TAIL_DIR[1]);
        assert!(
            sunward == Ink::BLANK || sunward == Ink::new(Role::Hot, 40),
            "tail on the sunward side: {sunward:?}"
        );
        // A white nucleus and a blue ion tail further out along the anti-Sun line.
        let core = g
            .pixels
            .get(nx as i32, ny as i32)
            .expect("nucleus on screen");
        assert_eq!(core, Ink::new(Role::Star, 255), "nucleus {core:?}");
        let di = 0.5 * ION_LEN * v.scale;
        let ion = (-1..=1).filter_map(|o| {
            let x = (nx + ION_DIR[0] * di) as i32 + o;
            g.pixels.get(x, (ny - ION_DIR[1] * di) as i32)
        });
        let ion: Vec<_> = ion.collect();
        assert!(
            ion.iter().any(|i| i.role == Role::Cold),
            "no ion tail: {ion:?}"
        );
    }

    #[test]
    fn brk_a7_jupiter_shares_the_stepped_cloud_palette() {
        let (g, _) = fresh();
        let (jx, jy) = g.view.px([0.0, 0.0]);
        let r = g.view.scale;
        let mut sienna = 0;
        let mut total = 0;
        for y in (jy - 0.8 * r) as i32..(jy + 0.8 * r) as i32 {
            for x in (jx - 0.5 * r) as i32..(jx + 0.5 * r) as i32 {
                if let Some(i) = g.pixels.get(x, y) {
                    total += 1;
                    sienna += usize::from(jovian::is_cloud(i));
                }
            }
        }
        assert!(
            total > 20 && sienna * 10 >= total * 8,
            "{sienna} of {total}"
        );
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
