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
use sciart_core::ease;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};

use sciart_shared::plume::Plume;

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
    /// When the last fragment struck (s).
    last_strike: f64,
    dots: Dots,
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
            last_strike: f64::NEG_INFINITY,
            dots: Dots::default(),
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
        // Spacing on the run-in: the chain's own, widened so each pearl reads as its own
        // body, and shrunk if needed so the last fragment strikes by 13 s into the act
        // (BRK-A4). 0.16 R_J per chain unit is ~4 dots apart at the home scale.
        let span = lead - self.fragments[self.n_frag.max(1) - 1].along;
        let spacing = 0.16f64.min(4.3 / span.max(1e-9));
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

    /// Draw the frame as braille threads (SPEC-002 Look, M16): Jupiter's thin limb and
    /// two belt lines, scars as small grey rings, the grains as white dots trailing dust
    /// (`Sun` → `Hot` with distance) and ion (`Cold`) threads, the fireball's grains
    /// coloured by heat, the flash as a starburst.
    fn compose(&mut self) {
        self.update_view();
        let v = self.view;
        let beat = self.timeline.at(self.t);
        self.dots.clear();
        // Pixel units (the view's) → dots: two dots per pixel each way.
        let dot = |(x, y): (f64, f64)| (2.0 * x, 2.0 * y);
        // Background stars: a fixed sparse hash per cell.
        let d = self.dots.dot_size();
        let (cw, ch) = (
            i32::from(d.width()) / Dots::CELL_W,
            i32::from(d.height()) / Dots::CELL_H,
        );
        for cy in 0..ch {
            for cx in 0..cw {
                if hash01(0x5E ^ cell_key(cx, cy)) < STAR_SHARE {
                    self.dots
                        .set(cx * Dots::CELL_W, cy * Dots::CELL_H + 1, STAR);
                }
            }
        }
        // Jupiter: the limb, two belt edges, the scars (BRK-A5).
        let (jx, jy) = dot(v.px([0.0, 0.0]));
        let jr = 2.0 * v.scale;
        let n = ((TAU * jr) as usize).clamp(16, 4096);
        for k in 0..n {
            let (s, c) = (TAU * k as f64 / n as f64).sin_cos_det();
            self.dots.plot(jx + jr * c, jy + jr * s, LIMB);
        }
        for s in BELTS {
            let half = jr * (1.0 - s * s).sqrt() * 0.97;
            let y = jy - s * jr;
            self.dots.line(jx - half, y, jx + half, y, BELT);
        }
        for sc in &self.scars[..self.n_scars] {
            if let Some(p) = self.scar_screen(sc) {
                let (sx, sy) = dot(p);
                let lon = sc.lon - SPIN * (self.t - sc.born);
                let fore = lon.cos_det().max(0.05);
                let (rx, ry) = (
                    (0.7 * sc.size * jr * fore).max(0.8),
                    (0.7 * sc.size * jr * 0.8).max(0.8),
                );
                let m = ((TAU * rx.max(ry)) as usize).clamp(8, 256);
                for k in 0..m {
                    let (s, c) = (TAU * k as f64 / m as f64).sin_cos_det();
                    self.dots.plot(sx + rx * c, sy + ry * s, SCAR);
                }
            }
        }
        // The Roche distance, dashed, while the pile passes inside it.
        if beat.index == BREAKUP {
            let rr = ROCHE * jr;
            let m = ((TAU * rr / 3.0) as usize).clamp(8, 4096);
            for k in (0..m).step_by(2) {
                let (s, c) = (TAU * k as f64 / m as f64).sin_cos_det();
                self.dots.plot(jx + rr * c, jy + rr * s, ROCHE_INK);
            }
        }
        // Tails: straight threads from a grain, dust away from the Sun, ion straight
        // anti-Sun, each in two steps by distance (the dust ages and reddens outward).
        // While the pile breaks up its many tails are capped, so they stay threads; in the
        // impacts act they are capped too, so the pearls stay separate bodies.
        let cap = if matches!(beat.index, BREAKUP | PEARLS | IMPACTS) {
            BREAK_TAIL_CAP
        } else {
            f64::INFINITY
        };
        let tail =
            |dots: &mut Dots, (x, y): (f64, f64), dir: [f64; 2], len: f64, inks: [Ink; 2]| {
                let len = len.min(cap / jr);
                let (tx, ty) = (dir[0] * len * jr, -dir[1] * len * jr);
                let (mx, my) = (x + 0.5 * tx, y + 0.5 * ty);
                dots.line(mx, my, x + tx, y + ty, inks[1]);
                dots.line(x, y, mx, my, inks[0]);
            };
        let com = orbit(self.tau);
        if beat.index <= PEARLS {
            for (k, p) in self.pos.iter().enumerate() {
                let at = dot(v.px([com[0] + MAG * p[0], com[1] + MAG * p[1]]));
                if k % ION_EVERY == 0 {
                    tail(&mut self.dots, at, ION_DIR, ION_LEN, ION_INKS);
                }
                if k % TAIL_EVERY == 0 {
                    tail(&mut self.dots, at, TAIL_DIR, TAIL_LEN, DUST_INKS);
                }
            }
            for p in self.pos.iter() {
                let (x, y) = dot(v.px([com[0] + MAG * p[0], com[1] + MAG * p[1]]));
                self.dots.plot(x, y, HEAD);
            }
        } else if beat.index == IMPACTS {
            for k in self.struck..self.n_frag {
                if let Some(p) = self.run_in(k, beat.local) {
                    let at = dot(v.px(p));
                    let w = (self.fragments[k].size as f64 / N as f64).sqrt();
                    tail(&mut self.dots, at, ION_DIR, ION_LEN * (0.4 + w), ION_INKS);
                    tail(
                        &mut self.dots,
                        at,
                        TAIL_DIR,
                        TAIL_LEN * (0.4 + w),
                        DUST_INKS,
                    );
                    self.dots.disc(at.0, at.1, 0.8 + 2.2 * w, HEAD);
                }
            }
        }
        // Impact fireballs: the plume's grains as dots coloured by heat, hidden behind
        // the disc; the flash a starburst of rays over the limb; the strike a white point.
        let site = impact_site();
        let (up, tangent) = (site, [-site[1], site[0]]);
        for gr in self.plume.iter() {
            let p = [
                site[0] + up[0] * f64::from(gr.pos[2]) + tangent[0] * f64::from(gr.pos[0]),
                site[1] + up[1] * f64::from(gr.pos[2]) + tangent[1] * f64::from(gr.pos[0]),
            ];
            if gr.pos[1] < 0.0 && p[0].hypot(p[1]) < 1.0 {
                continue;
            }
            let heat = (-f64::from(gr.age) / 1.4).exp();
            let ink = FIRE_INKS
                .iter()
                .find(|(from, _)| heat >= *from)
                .map_or(FIRE_INKS[2].1, |&(_, i)| i);
            let (x, y) = dot(v.px(p));
            self.dots.plot(x, y, ink);
        }
        // Each recent strike: an expanding shock ring on the limb, fading over FLASH
        // seconds, so the impacts read as events rather than as one more pale thread.
        let site_dot = dot(v.px([site[0] * 1.02, site[1] * 1.02]));
        for sc in &self.scars[..self.n_scars] {
            let age = self.t - sc.born;
            if !(0.0..FLASH).contains(&age) {
                continue;
            }
            let u = age / FLASH;
            // A shock ring about half a Jupiter radius across at its brightest, growing
            // outward as it fades: big enough to read against the limb at 80x24.
            let rr = (0.05 + 0.50 * u) * jr;
            let m = ((TAU * rr) as usize).clamp(12, 512);
            let ink = if u < 0.35 {
                FLASH_CORE
            } else if u < 0.7 {
                FLASH_INK
            } else {
                Ink::new(Role::Hot, 170)
            };
            for k in 0..m {
                let (s, c) = (TAU * k as f64 / m as f64).sin_cos_det();
                let (x, y) = (site_dot.0 + rr * c, site_dot.1 + rr * s);
                // Outside the limb: the ejecta rises off the planet, and the ring must not
                // paint over Jupiter's dark disc.
                if (x - jx).hypot(y - jy) >= jr {
                    self.dots.plot(x, y, ink);
                }
            }
        }
        // The newest strike also throws a cone of rays off the limb (the impact's ejecta),
        // bright for a moment and then fading; the strike point itself is briefly white.
        let since = self.t - self.last_strike;
        if (0.0..CONE_LIFE).contains(&since) {
            let fade = 1.0 - since / CONE_LIFE;
            let base = site[1].atan2(site[0]);
            for k in -2..=2 {
                let a = base + 0.22 * f64::from(k);
                let (s, c) = a.sin_cos_det();
                let len = (0.25 + 0.60 * fade) * jr;
                let ink = if k == 0 { FLASH_CORE } else { FLASH_INK };
                self.dots.line(
                    site_dot.0,
                    site_dot.1,
                    site_dot.0 + len * c,
                    site_dot.1 - len * s,
                    ink,
                );
            }
        }
        if (0.0..STRIKE_WHITE).contains(&since) {
            self.dots.disc(site_dot.0, site_dot.1, 2.5, HEAD);
        }
    }
}

/// How long the strike's white point shows (s).
const STRIKE_WHITE: f64 = 0.6;

/// How far the view leans from Jupiter towards the comet during the approach.
const FOLLOW_APPROACH: f64 = 0.55;
/// Dust tails: direction (unit, world, away from a Sun off to the lower left), length in
/// R_J, and one dust thread per `TAIL_EVERY` grains (so a few threads per fragment,
/// not a filled band).
const TAIL_DIR: [f64; 2] = [0.6, 0.8];
const TAIL_LEN: f64 = 0.8;
const TAIL_EVERY: usize = 16;
/// Ion tail: straight away from the Sun (unit, world), length (R_J), one streamer per
/// `ION_EVERY` grains.
const ION_DIR: [f64; 2] = [0.2, 0.98];
const ION_LEN: f64 = 1.4;
const ION_EVERY: usize = 40;
/// Longest tail thread (dots) during the breakup and the string of pearls.
pub(crate) const BREAK_TAIL_CAP: f64 = 12.0;
/// Inks (SPEC-002 Look, M16). Grains and fragments white; dust threads `Sun` near the
/// nucleus, `Hot` in the outer half; ion threads `Cold`, fainter outward.
pub(crate) const HEAD: Ink = Ink::new(Role::Star, 255);
pub(crate) const DUST_INKS: [Ink; 2] = [Ink::new(Role::Sun, 220), Ink::new(Role::Hot, 160)];
pub(crate) const ION_INKS: [Ink; 2] = [Ink::new(Role::Cold, 200), Ink::new(Role::Cold, 120)];
/// Jupiter: a thin `Sun` limb and two `Hot` belt edges (sine latitudes); scars grey rings.
pub(crate) const LIMB: Ink = Ink::new(Role::Sun, 190);
pub(crate) const BELT: Ink = Ink::new(Role::Hot, 110);
const BELTS: [f64; 2] = [0.18, -0.24];
pub(crate) const SCAR: Ink = Ink::new(Role::Dim, 200);
const ROCHE_INK: Ink = Ink::new(Role::Dim, 140);
/// Fireball grains by heat `exp(−age/1.4 s)`: white, then yellow, then orange.
const FIRE_INKS: [(f64, Ink); 3] = [
    (0.6, Ink::new(Role::Star, 240)),
    (0.3, Ink::new(Role::Sun, 220)),
    (0.0, Ink::new(Role::Hot, 170)),
];
const FLASH_INK: Ink = Ink::new(Role::Sun, 230);
/// The hottest part of the impact flash (the ring's first frame, the cone's axis).
const FLASH_CORE: Ink = Ink::new(Role::Star, 255);
/// A background star (one dot, dim) in this share of cells.
const STAR: Ink = Ink::new(Role::Star, 60);
const STAR_SHARE: f64 = 0.004;
/// The impact cone's life (s).
const CONE_LIFE: f64 = 1.6;

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
        self.dots.resize(size);
        // The view works in half-block pixel units (w × 2h, square), two dots each.
        self.frame = (f64::from(size.width()), 2.0 * f64::from(size.height()));
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
        self.dots.blit(canvas);
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

    /// The inks of the cells within one cell of dot `(x, y)` in a rendered frame.
    fn inks_near(canvas: &Canvas, (x, y): (f64, f64)) -> Vec<Ink> {
        let (cx, cy) = ((x / 2.0).floor() as i32, (y / 4.0).floor() as i32);
        let mut out = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(c) = canvas.get(cx + dx, cy + dy) {
                    if c.ch != ' ' {
                        out.push(c.ink);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn brk_a6_the_comet_has_threaded_tails_from_the_start() {
        let (mut g, mut rng) = fresh();
        run_to(&mut g, &mut rng, 0.1);
        g.compose();
        let mut canvas = Canvas::new(Size::new(100, 30), GlyphMode::Unicode);
        g.render(&mut canvas);
        let v = g.view;
        let (nx, ny) = v.px(orbit(g.tau));
        let (nx, ny) = (2.0 * nx, 2.0 * ny);
        let jr = 2.0 * v.scale;
        let along = |dir: [f64; 2], s: f64| (nx + dir[0] * s * jr, ny - dir[1] * s * jr);
        // A white nucleus; the dust thread `Sun` near it and `Hot` in its outer half; no
        // dust on the sunward side; a blue ion streamer half its length out.
        assert!(inks_near(&canvas, (nx, ny)).contains(&HEAD), "no nucleus");
        let near = inks_near(&canvas, along(TAIL_DIR, 0.25 * TAIL_LEN));
        assert!(near.contains(&DUST_INKS[0]), "near dust {near:?}");
        let far = inks_near(&canvas, along(TAIL_DIR, 0.8 * TAIL_LEN));
        assert!(far.contains(&DUST_INKS[1]), "far dust {far:?}");
        let sunward = inks_near(&canvas, along(TAIL_DIR, -0.45 * TAIL_LEN));
        assert!(
            !sunward.iter().any(|i| DUST_INKS.contains(i)),
            "dust on the sunward side: {sunward:?}"
        );
        let ion = inks_near(&canvas, along(ION_DIR, 0.5 * ION_LEN));
        assert!(
            ion.iter().any(|i| i.role == Role::Cold),
            "no ion tail: {ion:?}"
        );
        // Every lit cell is a braille thread cell (no half blocks).
        for y in 0..30 {
            for x in 0..100 {
                let ch = canvas.get(x, y).unwrap().ch;
                assert!(ch == ' ' || ('⠀'..='⣿').contains(&ch), "{x},{y}: {ch:?}");
            }
        }
    }

    #[test]
    fn brk_a7_jupiter_is_a_thin_limb_with_belt_lines() {
        let (g, _) = fresh();
        let (jx, jy) = g.view.px([0.0, 0.0]);
        let (jx, jy, jr) = (2.0 * jx, 2.0 * jy, 2.0 * g.view.scale);
        // The limb: nearly every point of the circle is lit.
        let (mut on, mut n) = (0, 0);
        for k in 0..360 {
            let (s, c) = (TAU * f64::from(k) / 360.0).sin_cos_det();
            let (x, y) = ((jx + jr * c).round() as i32, (jy + jr * s).round() as i32);
            let d = g.dots.dot_size();
            if (0..i32::from(d.width())).contains(&x) && (0..i32::from(d.height())).contains(&y) {
                n += 1;
                on += usize::from(g.dots.is_lit(x, y));
            }
        }
        assert!(n > 100 && on * 10 >= n * 9, "limb {on} of {n}");
        // Inside, black but for the belt lines (and the odd star).
        let (mut lit, mut inside) = (0, 0);
        for y in (jy - 0.85 * jr) as i32..(jy + 0.85 * jr) as i32 {
            if BELTS
                .iter()
                .any(|s| (f64::from(y) - (jy - s * jr)).abs() < 1.5)
            {
                continue;
            }
            for x in (jx - 0.5 * jr) as i32..(jx + 0.5 * jr) as i32 {
                inside += 1;
                lit += usize::from(g.dots.is_lit(x, y));
            }
        }
        assert!(
            inside > 200 && lit * 50 <= inside,
            "{lit} of {inside} lit inside"
        );
    }

    #[test]
    fn brk_a8_tails_are_capped_during_the_breakup() {
        // In the breakup and the string of pearls no tail thread runs more than
        // BREAK_TAIL_CAP dots from the grain it leaves (checked on the lit dots beyond
        // the pile: every one lies within the cap of some grain).
        let (mut g, mut rng) = fresh();
        for t in [6.0, 20.0] {
            run_to(&mut g, &mut rng, t);
            g.compose();
            let v = g.view;
            let com = orbit(g.tau);
            let grains: Vec<(f64, f64)> = g
                .pos
                .iter()
                .map(|p| {
                    let (x, y) = v.px([com[0] + MAG * p[0], com[1] + MAG * p[1]]);
                    (2.0 * x, 2.0 * y)
                })
                .collect();
            let d = g.dots.dot_size();
            let (jx, jy) = v.px([0.0, 0.0]);
            let (jx, jy, jr) = (2.0 * jx, 2.0 * jy, 2.0 * v.scale);
            for y in 0..i32::from(d.height()) {
                for x in 0..i32::from(d.width()) {
                    let (fx, fy) = (f64::from(x), f64::from(y));
                    // Leave out Jupiter, the Roche circle and the stars.
                    let rj = (fx - jx).hypot(fy - jy) / jr;
                    if !g.dots.is_lit(x, y) || rj < 1.05 || (rj - ROCHE).abs() < 0.1 {
                        continue;
                    }
                    let near = grains
                        .iter()
                        .map(|&(gx, gy)| (gx - fx).hypot(gy - fy))
                        .fold(f64::INFINITY, f64::min);
                    let star = hash01(0x5E ^ cell_key(x / 2, y / 4)) < STAR_SHARE;
                    assert!(
                        star || near <= BREAK_TAIL_CAP + 1.5,
                        "t {t}: dot ({x}, {y}) {near:.1} dots from the pile"
                    );
                }
            }
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
