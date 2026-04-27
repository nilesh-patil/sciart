//! An emission nebula with dust pillars where gas falls onto protostars that ignite and
//! launch bipolar jets (SPEC-002 `stellar_nursery`, NUR-A1..A3).
//!
//! Positions are in rows (x on screen = `x · ASPECT`). The nebula is a domain-warped sum of
//! sines whose warp is chosen *separable* (x is warped by a function of y and vice versa),
//! so every mode's phase splits into a column part plus a row part and a frame costs
//! O((w + h)·modes) trig calls instead of O(w·h·modes).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::particles::{kepler_speed, plot};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "stellar_nursery",
    category: Category::DeepCosmos,
    description: "A glowing nebula where gas streams onto protostars that ignite",
    title: "Star birth · gas clouds collapse",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(StellarNursery::default())
}

/// Sine terms in the nebula field (spec: ≤ 6).
const MODES: usize = 6;
/// Largest internal integration step; `update` sub-steps larger `dt`s.
const MAX_STEP: f64 = 1.0 / 60.0;
/// Ignition mass (normalized units).
pub(crate) const M_IGN: f64 = 1.0;
/// Gas within this many rows of its protostar is absorbed.
pub(crate) const R_ABS: f64 = 0.8;
/// Gravitational softening length (rows).
const EPS: f64 = 1.0;
/// Brightness ramp after ignition (s).
pub(crate) const IGNITION_RAMP: f64 = 1.0;
/// Main-sequence display time before a star fades (s).
pub(crate) const SHINE: f64 = 25.0;
/// Fade-out before replacement (s).
pub(crate) const FADE: f64 = 2.5;
const GAS_PER_AREA: f64 = 0.03;
pub(crate) const GAS_MAX: usize = 600;
pub(crate) const JET_MAX: usize = 240;
/// Target free-fall time from the feeding radius (s); fixes `gm` per size.
const T_FF: f64 = 3.0;
/// Time to fill an empty gas pool (s).
const T_FILL: f64 = 2.0;
/// Typical infall time of a parcel (s), used to normalize the parcel mass.
const T_ARRIVE: f64 = 2.5;
/// Typical accretion time from zero mass to ignition (s).
const ACC_TIME: f64 = 16.0;
/// Closest spawn distance to a protostar (rows).
const R_SPAWN_MIN: f64 = 3.0;
/// Jet particles per second per freshly ignited star.
const JET_RATE: f64 = 90.0;
/// e-folding time of jet activity after ignition (s): outflows weaken as accretion stops.
const JET_DECAY: f64 = 9.0;
/// Period of episodic ejection, which is what makes Herbig–Haro knots (s).
const JET_PULSE: f64 = 0.8;

/// Nebula z-score below which nothing is drawn, and the span mapped to full brightness.
const NEB_THRESHOLD: f64 = -0.1;
const NEB_SPAN: f64 = 2.4;

const NEBULA: [Glyph; 4] = [
    Glyph::new('·', '.'),
    Glyph::new('░', ':'),
    Glyph::new('▒', '+'),
    Glyph::new('▓', '#'),
];
const DUST: Glyph = Glyph::new('█', '#');
const DUST_THIN: Glyph = Glyph::new('▓', '#');
const RIM_TOP: Glyph = Glyph::new('▒', '%');
const RIM_SIDE: Glyph = Glyph::new('░', ':');
const GAS: [Glyph; 3] = [
    Glyph::new('·', '.'),
    Glyph::new('∙', '.'),
    Glyph::new('•', 'o'),
];
const EMBEDDED: [Glyph; 2] = [Glyph::new('∙', '.'), Glyph::new('•', 'o')];
const STAR: Glyph = Glyph::new('✦', '*');
const SPIKE_H: Glyph = Glyph::new('─', '-');
const SPIKE_V: Glyph = Glyph::new('│', '|');
const JET_DIRS: [Glyph; 4] = [
    Glyph::new('─', '-'),
    Glyph::new('╲', '\\'),
    Glyph::new('│', '|'),
    Glyph::new('╱', '/'),
];
const KNOT: Glyph = Glyph::new('•', 'o');

/// Radial acceleration toward a protostar, `gm / (d² + ε²)` (Plummer-softened).
#[inline]
pub(crate) fn gravity_accel(gm: f64, d: f64) -> f64 {
    gm / (d * d + EPS * EPS)
}

/// One semi-implicit Euler step of radial infall: kick `vr` (inward speed, ≥ 0) with the
/// softened pull, then drift inward. Distance can only shrink (NUR-A1) because the
/// inward speed only grows and the drift subtracts it.
pub(crate) fn infall_step(r: f64, vr: f64, gm: f64, dt: f64) -> (f64, f64) {
    let vr = vr.max(0.0) + gravity_accel(gm, r) * dt.max(0.0);
    ((r - vr * dt.max(0.0)).max(0.0), vr)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Phase {
    /// Embedded protostar gathering gas.
    Accreting,
    /// Ignited; the value is seconds since ignition.
    Shining(f64),
    /// Dying away; seconds since the fade began.
    Fading(f64),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Protostar {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) mass: f64,
    pub(crate) phase: Phase,
    /// Jet axis angle (rad), in row units.
    jet: f64,
    /// Rotation sense of the infalling envelope (±1).
    spin: f64,
    /// Bumped on replacement so stale gas can recognise its target is gone.
    pub(crate) generation: u32,
    jet_acc: f64,
    /// Directions of the filaments that feed this hub (rad).
    feeds: [f64; 3],
    /// Relative gas supply of this hub.
    richness: f64,
}

impl Protostar {
    fn new(x: f64, y: f64, rng: &mut impl Rng, generation: u32, mass: f64) -> Self {
        Self {
            x,
            y,
            mass,
            phase: Phase::Accreting,
            jet: rng.random_range(0.0..TAU),
            spin: if rng.random_bool(0.5) { 1.0 } else { -1.0 },
            generation,
            jet_acc: 0.0,
            feeds: {
                let a: f64 = rng.random_range(0.0..TAU);
                [
                    a,
                    a + rng.random_range(1.6..=2.6),
                    a - rng.random_range(1.6..=2.6),
                ]
            },
            richness: rng.random_range(0.5..=1.5),
        }
    }

    /// Add absorbed gas mass; returns `true` iff this call ignites the star — i.e. exactly
    /// when the mass first reaches `M_IGN` (NUR-A2). Negative input is ignored so the mass
    /// never decreases.
    pub(crate) fn accrete(&mut self, dm: f64) -> bool {
        if dm > 0.0 {
            self.mass += dm;
        }
        if self.phase == Phase::Accreting && self.mass >= M_IGN {
            self.phase = Phase::Shining(0.0);
            true
        } else {
            false
        }
    }

    /// Visible brightness in `[0, 1]`: 0 while embedded, smoothstep over the 1 s
    /// ignition ramp, linear fade at the end.
    pub(crate) fn brightness(&self) -> f64 {
        match self.phase {
            Phase::Accreting => 0.0,
            Phase::Shining(a) => {
                let s = (a / IGNITION_RAMP).clamp(0.0, 1.0);
                s * s * (3.0 - 2.0 * s)
            }
            Phase::Fading(a) => (1.0 - a / FADE).clamp(0.0, 1.0),
        }
    }

    fn is_accreting(&self) -> bool {
        self.phase == Phase::Accreting
    }
}

/// Infalling gas, stored in polar coordinates about its target so its distance to the
/// target is exactly `r` (NUR-A1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gas {
    /// Stable identity, so tests can follow a parcel across `swap_remove` compaction.
    #[cfg_attr(not(test), expect(dead_code, reason = "only read by the NUR-A1 tests"))]
    pub(crate) id: u32,
    pub(crate) star: usize,
    pub(crate) generation: u32,
    pub(crate) r: f64,
    theta: f64,
    /// Inward radial speed (rows/s), ≥ 0.
    vr: f64,
    /// Specific angular momentum (rows²/s); gives the streams a gentle spiral.
    l: f64,
    age: f64,
}

impl Gas {
    pub(crate) fn pos(&self, s: &Protostar) -> (f64, f64) {
        let (sn, cs) = self.theta.sin_cos();
        (s.x + self.r * cs, s.y + self.r * sn)
    }
}

/// A ballistic jet particle (rows, rows/s).
#[derive(Debug, Clone, Copy)]
struct Jet {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
    life: f64,
    dir: u8,
}

/// One nebula plane wave; `k` in rad/row, `omega` in rad/s.
#[derive(Debug, Clone, Copy)]
struct Mode {
    kx: f64,
    ky: f64,
    phase: f64,
    omega: f64,
    weight: f64,
}

/// A dust pillar as fractions of the screen, so it survives resizes.
#[derive(Debug, Clone, Copy)]
struct Pillar {
    base: f64,
    lean: f64,
    height: f64,
    base_half: f64,
    tip_half: f64,
    wobble: f64,
}

/// Separable domain warp: x is displaced by `amp·sin(qy·y + …)`, y by `amp·sin(qx·x + …)`.
#[derive(Debug, Clone, Copy, Default)]
struct Warp {
    amp: f64,
    qx: f64,
    qy: f64,
    px: f64,
    py: f64,
    nu: f64,
}

/// Mask values.
const M_DUST: u8 = 1;
const M_RIM_TOP: u8 = 2;
const M_RIM_SIDE: u8 = 3;

#[derive(Debug)]
pub struct StellarNursery {
    t: f64,
    w: usize,
    h: usize,
    modes: Vec<Mode>,
    warp: Warp,
    /// 1/σ of the raw field (so the sum reads as a z-score).
    inv_sigma: f64,
    col: Vec<[(f64, f64); MODES]>,
    row: Vec<[(f64, f64); MODES]>,
    /// Illumination gradient: the ionizing cluster sits above the frame.
    row_bias: Vec<f64>,
    pillars: Vec<Pillar>,
    mask: Vec<u8>,
    tips: Vec<(f64, f64)>,
    stars: Vec<Protostar>,
    gas: ParticlePool<Gas>,
    jets: ParticlePool<Jet>,
    gm: f64,
    r_feed: f64,
    r_hii: f64,
    gas_mass: f64,
    spawn_acc: f64,
    next_id: u32,
}

impl Default for StellarNursery {
    fn default() -> Self {
        Self {
            t: 0.0,
            w: 0,
            h: 0,
            modes: Vec::new(),
            warp: Warp::default(),
            inv_sigma: 0.0,
            col: Vec::new(),
            row: Vec::new(),
            row_bias: Vec::new(),
            pillars: Vec::new(),
            mask: Vec::new(),
            tips: Vec::new(),
            stars: Vec::new(),
            gas: ParticlePool::with_capacity(0),
            jets: ParticlePool::with_capacity(0),
            gm: 0.0,
            r_feed: 0.0,
            r_hii: 0.0,
            gas_mass: 0.0,
            spawn_acc: 0.0,
            next_id: 0,
        }
    }
}

impl StellarNursery {
    fn init_sky(&mut self, rng: &mut impl Rng) {
        self.modes = (0..MODES)
            .map(|_| {
                let wavelength = rng.random_range(10.0..=42.0);
                let dir: f64 = rng.random_range(0.0..TAU);
                let k = TAU / wavelength;
                Mode {
                    kx: k * dir.cos(),
                    ky: k * dir.sin(),
                    phase: rng.random_range(0.0..TAU),
                    omega: rng.random_range(0.04..=0.14)
                        * if rng.random_bool(0.5) { 1.0 } else { -1.0 },
                    weight: rng.random_range(0.5..=1.0),
                }
            })
            .collect();
        // Independent random phases: variance of Σ aᵢ sin(·) is Σ aᵢ² / 2.
        let var: f64 = self.modes.iter().map(|m| m.weight * m.weight).sum::<f64>() / 2.0;
        self.inv_sigma = 1.0 / var.sqrt();
        self.warp = Warp {
            amp: rng.random_range(3.0..=5.0),
            qx: TAU / rng.random_range(18.0..=34.0),
            qy: TAU / rng.random_range(10.0..=20.0),
            px: rng.random_range(0.0..TAU),
            py: rng.random_range(0.0..TAU),
            nu: rng.random_range(0.08..=0.16),
        };
        let n = rng.random_range(2..=3);
        let slot = 1.0 / f64::from(n);
        self.pillars = (0..n)
            .map(|i| Pillar {
                base: slot * (f64::from(i) + rng.random_range(0.3..=0.7)),
                lean: rng.random_range(-0.05..=0.05),
                height: rng.random_range(0.35..=0.7),
                base_half: rng.random_range(0.035..=0.06),
                tip_half: rng.random_range(0.7..=1.0),
                wobble: rng.random_range(0.0..TAU),
            })
            .collect();
    }

    /// Rasterise the pillars into `mask` and find their tips (rows).
    fn build_mask(&mut self) {
        let (w, h) = (self.w, self.h);
        self.mask.clear();
        self.mask.resize(w * h, 0);
        self.tips.clear();
        let (wf, hf) = (w as f64, h as f64);
        for p in &self.pillars {
            let bottom = hf - 1.0;
            let tip_y = (bottom - p.height * hf).max(0.0);
            let base_half = (p.base_half * wf).max(1.0);
            let tip_half = (base_half * p.tip_half).max(0.8);
            let centre = |y: f64| {
                let s = ((y - tip_y) / (bottom - tip_y).max(1.0)).clamp(0.0, 1.0);
                (p.base + p.lean * (1.0 - s)) * wf
            };
            // A bulbous head (the dense globule that shields the column), a narrower neck,
            // and a base that flares; the two edges wobble independently.
            let head = tip_half / ASPECT;
            let neck = 0.7 * tip_half;
            let ph = p.wobble;
            for y in 0..h {
                let yf = y as f64;
                let (half_l, half_r) = if yf >= tip_y {
                    let s = ((yf - tip_y) / (bottom - tip_y).max(1.0)).clamp(0.0, 1.0);
                    let core = neck + (base_half - neck) * s.powf(1.5);
                    let bulb = tip_half * (-(s / 0.14).powi(2)).exp();
                    let half = core.max(bulb);
                    (
                        half * (1.0
                            + 0.18 * (0.9 * yf + ph).sin()
                            + 0.08 * (2.7 * yf + 2.0 * ph).sin()),
                        half * (1.0
                            + 0.18 * (0.7 * yf + ph + 2.0).sin()
                            + 0.08 * (3.1 * yf + ph).sin()),
                    )
                } else {
                    let d = (tip_y - yf) / head.max(0.5);
                    if d > 1.0 {
                        continue;
                    }
                    let half = tip_half * (1.0 - d * d).sqrt();
                    (half, half)
                };
                let c = centre(yf);
                let x0 = (c - half_l).ceil().max(0.0);
                let x1 = (c + half_r).floor().min(wf - 1.0);
                if x0 > x1 {
                    continue;
                }
                for x in x0 as usize..=x1 as usize {
                    self.mask[y * w + x] = M_DUST;
                }
            }
            self.tips.push((centre(tip_y) / ASPECT, tip_y + 0.5));
        }
        // Ionization fronts: dust surfaces facing the (overhead) ionizing stars glow.
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if self.mask[i] == 0 {
                    continue;
                }
                let open = |xx: usize, yy: usize| self.mask[yy * w + xx] == 0;
                if y == 0 || open(x, y - 1) {
                    self.mask[i] = M_RIM_TOP;
                } else if (x > 0 && open(x - 1, y)) || (x + 1 < w && open(x + 1, y)) {
                    self.mask[i] = M_RIM_SIDE;
                }
            }
        }
    }

    /// Fill the per-column and per-row `(sin, cos)` tables for the current time.
    fn refresh_tables(&mut self) {
        let (t, wp) = (self.t, self.warp);
        for (x, c) in self.col.iter_mut().enumerate() {
            let xr = x as f64 / ASPECT;
            let dy = wp.amp * (wp.qx * xr + wp.px + wp.nu * t).sin();
            for (o, m) in c.iter_mut().zip(&self.modes) {
                *o = (m.kx * xr + m.ky * dy + m.phase + m.omega * t).sin_cos();
            }
        }
        for (y, r) in self.row.iter_mut().enumerate() {
            let yf = y as f64;
            let dx = wp.amp * (wp.qy * yf + wp.py - 0.7 * wp.nu * t).sin();
            for (o, m) in r.iter_mut().zip(&self.modes) {
                *o = (m.ky * yf + m.kx * dx).sin_cos();
            }
        }
    }

    /// Nebula z-score at cell `(x, y)`: `Σ wᵢ sin(Aᵢ(x) + Bᵢ(y)) / σ` plus the gradient.
    #[inline]
    fn field(&self, x: usize, y: usize) -> f64 {
        let (c, r) = (&self.col[x], &self.row[y]);
        let mut f = 0.0;
        for i in 0..MODES {
            f += self.modes[i].weight * (c[i].0 * r[i].1 + c[i].1 * r[i].0);
        }
        f * self.inv_sigma + self.row_bias[y]
    }

    fn in_bounds(&self, x: f64, y: f64) -> bool {
        let xmax = (self.w as f64 - 1.0) / ASPECT;
        (0.0..=xmax).contains(&x) && (0.0..=self.h as f64 - 1.0).contains(&y)
    }

    fn cell(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (cx, cy) = ((x * ASPECT).round(), y.round());
        ((0.0..self.w as f64).contains(&cx) && (0.0..self.h as f64).contains(&cy))
            .then_some((cx as usize, cy as usize))
    }

    /// A protostar position away from the others (and inside the frame).
    fn place_star(&self, rng: &mut impl Rng, skip: Option<usize>) -> (f64, f64) {
        let (wr, hf) = (
            (self.w as f64 - 1.0).max(0.0) / ASPECT,
            (self.h as f64 - 1.0).max(0.0),
        );
        let min_sep = 0.22 * wr.min(hf * 2.0).max(1.0);
        let mut best = (wr / 2.0, hf / 2.0);
        let mut best_d = -1.0;
        for _ in 0..24 {
            let x = rng.random_range(0.12..=0.88) * wr;
            let y = rng.random_range(0.15..=0.8) * hf;
            let d = self
                .stars
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != skip)
                .map(|(_, s)| (s.x - x).hypot(s.y - y))
                .fold(f64::INFINITY, f64::min);
            if d > best_d {
                best = (x, y);
                best_d = d;
            }
            if d >= min_sep {
                break;
            }
        }
        best
    }

    fn nearest_accreting(&self, x: f64, y: f64) -> Option<usize> {
        self.stars
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_accreting())
            .map(|(i, s)| (i, (s.x - x).hypot(s.y - y)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Spawn one gas parcel in the nebula around an accreting protostar. Returns `false`
    /// when there is no room or nowhere to put it.
    fn spawn_gas(&mut self, rng: &mut impl Rng) -> bool {
        let accreting = self.stars.iter().filter(|s| s.is_accreting()).count();
        if accreting == 0 || self.gas.len() >= self.gas.capacity() {
            return false;
        }
        // Hubs differ in how rich their surroundings are, which desynchronizes ignitions.
        let total: f64 = self
            .stars
            .iter()
            .filter(|s| s.is_accreting())
            .map(|s| s.richness)
            .sum();
        let mut pick = rng.random_range(0.0..1.0) * total;
        let mut host = None;
        for s in self.stars.iter().filter(|s| s.is_accreting()) {
            host = Some(*s);
            pick -= s.richness;
            if pick < 0.0 {
                break;
            }
        }
        let Some(host) = host else {
            return false;
        };
        let (mut px, mut py) = (host.x, host.y);
        let mut found = false;
        for attempt in 0..6 {
            // Gas arrives along a hub's filaments rather than isotropically.
            let feed = host.feeds[rng.random_range(0..host.feeds.len())];
            let th = feed + rng.random_range(-0.22..=0.22);
            let u: f64 = rng.random();
            let r = R_SPAWN_MIN + (self.r_feed - R_SPAWN_MIN).max(0.0) * u.sqrt();
            let (x, y) = (host.x + r * th.cos(), host.y + r * th.sin());
            let Some((cx, cy)) = self.cell(x, y).filter(|_| self.in_bounds(x, y)) else {
                continue;
            };
            // Prefer the bright (dense) parts of the cloud; dust pillars count as dense too.
            if self.mask[cy * self.w + cx] != 0
                || self.field(cx, cy) > NEB_THRESHOLD
                || attempt == 5
            {
                (px, py, found) = (x, y, true);
                break;
            }
        }
        if !found {
            return false;
        }
        let Some(target) = self.nearest_accreting(px, py) else {
            return false;
        };
        let s = self.stars[target];
        let r = (px - s.x).hypot(py - s.y);
        if r <= R_ABS + 0.5 {
            return false;
        }
        let v_circ = kepler_speed(self.gm, r);
        let gas = Gas {
            id: self.next_id,
            star: target,
            generation: s.generation,
            r,
            theta: (py - s.y).atan2(px - s.x),
            vr: rng.random_range(0.0..=0.15) * v_circ,
            l: s.spin * rng.random_range(0.1..=0.3) * r * v_circ,
            age: 0.0,
        };
        self.next_id = self.next_id.wrapping_add(1);
        self.gas.spawn(gas)
    }

    fn emit_jets(&mut self, i: usize, h: f64, rng: &mut impl Rng) {
        let s = self.stars[i];
        let Phase::Shining(age) = s.phase else { return };
        let pulse = (0.5 + 0.5 * (TAU * age / JET_PULSE).sin()).powi(2);
        let strength = s.brightness() * (-age / JET_DECAY).exp() * pulse;
        let mut acc = s.jet_acc + JET_RATE * strength * h;
        let (sn, cs) = s.jet.sin_cos();
        // Screen-space orientation of the axis picks the line glyph.
        let ang = sn.atan2(cs * ASPECT).rem_euclid(std::f64::consts::PI);
        let dir = ((ang / (std::f64::consts::PI / 4.0)).round() as u8) % 4;
        while acc >= 1.0 {
            acc -= 1.0;
            let sign = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
            let speed = rng.random_range(6.0..=10.0);
            let spread = rng.random_range(-0.08..=0.08);
            let (a_s, a_c) = (s.jet + spread).sin_cos();
            let jet = Jet {
                x: s.x + sign * 0.6 * a_c,
                y: s.y + sign * 0.6 * a_s,
                vx: sign * speed * a_c,
                vy: sign * speed * a_s,
                age: 0.0,
                life: rng.random_range(0.9..=1.5),
                dir,
            };
            if !self.jets.spawn(jet) {
                acc = 0.0;
                break;
            }
        }
        self.stars[i].jet_acc = acc;
    }

    fn substep(&mut self, h: f64, rng: &mut impl Rng) {
        self.t += h;
        // Protostar life cycle.
        for i in 0..self.stars.len() {
            match self.stars[i].phase {
                Phase::Accreting => {}
                Phase::Shining(a) => {
                    self.stars[i].phase = if a + h >= SHINE {
                        Phase::Fading(0.0)
                    } else {
                        Phase::Shining(a + h)
                    };
                    self.emit_jets(i, h, rng);
                }
                Phase::Fading(a) if a + h < FADE => self.stars[i].phase = Phase::Fading(a + h),
                Phase::Fading(_) => {
                    let (x, y) = self.place_star(rng, Some(i));
                    let generation = self.stars[i].generation.wrapping_add(1);
                    let mass = rng.random_range(0.0..=0.4) * M_IGN;
                    self.stars[i] = Protostar::new(x, y, rng, generation, mass);
                }
            }
        }
        // Gas infall and accretion.
        let (gm, m_p) = (self.gm, self.gas_mass);
        let stars = &mut self.stars;
        self.gas.retain(|g| {
            let Some(s) = stars
                .get_mut(g.star)
                .filter(|s| s.generation == g.generation)
            else {
                return false;
            };
            let (r, vr) = infall_step(g.r, g.vr, gm, h);
            if r <= R_ABS {
                s.accrete(m_p);
                return false;
            }
            // ω = L / r², capped so a parcel never whips round in one step.
            g.theta += (g.l / (r * r) * h).clamp(-0.25, 0.25);
            (g.r, g.vr, g.age) = (r, vr, g.age + h);
            true
        });
        // Replenish the cloud.
        let accreting = self.stars.iter().filter(|s| s.is_accreting()).count();
        if accreting > 0 && self.gas.len() < self.gas.capacity() {
            // Each hub is fed at the same rate, so accretion time is independent of how
            // many hubs are active.
            let share = accreting as f64 / self.stars.len() as f64;
            self.spawn_acc += share * self.gas.capacity() as f64 / T_FILL * h;
            while self.spawn_acc >= 1.0 {
                self.spawn_acc -= 1.0;
                if !self.spawn_gas(rng) {
                    self.spawn_acc = 0.0;
                    break;
                }
            }
        } else {
            self.spawn_acc = 0.0;
        }
        // Jets coast outward, decelerating as they sweep up ambient gas.
        let drag = (1.0 - 0.5 * h).max(0.0);
        self.jets.retain(|j| {
            j.x += j.vx * h;
            j.y += j.vy * h;
            j.vx *= drag;
            j.vy *= drag;
            j.age += h;
            j.age < j.life
        });
    }

    fn render_nebula(&self, canvas: &mut Canvas) {
        // HII regions: each shining star ionizes a growing bubble (Strömgren sphere).
        let mut lamps = [(0.0, 0.0, 0.0, 0.0); 5];
        let mut n = 0;
        for s in &self.stars {
            let b = s.brightness();
            let age = match s.phase {
                Phase::Shining(a) => a,
                Phase::Fading(_) => SHINE,
                Phase::Accreting => continue,
            };
            let r = self.r_hii * (1.0 - (-age / 3.0).exp());
            if b > 0.0 && r > 0.5 && n < lamps.len() {
                lamps[n] = (s.x * ASPECT, s.y, r, b);
                n += 1;
            }
        }
        let lamps = &lamps[..n];
        for y in 0..self.h {
            let yf = y as f64;
            for x in 0..self.w {
                let m = self.mask[y * self.w + x];
                let z = self.field(x, y);
                match m {
                    M_DUST => {
                        let v = ((z + 1.5) / 4.0).clamp(0.0, 1.0);
                        // Thinner, back-lit dust where the nebula behind is bright.
                        let g = if v > 0.7 { DUST_THIN } else { DUST };
                        canvas.put(
                            x as i32,
                            y as i32,
                            g,
                            Ink::new(Role::Structure, (8.0 + 40.0 * v) as u8),
                        );
                        continue;
                    }
                    M_RIM_TOP => {
                        canvas.put(x as i32, y as i32, RIM_TOP, Ink::new(Role::Hot, 210));
                        continue;
                    }
                    M_RIM_SIDE => {
                        canvas.put(x as i32, y as i32, RIM_SIDE, Ink::new(Role::Hot, 120));
                        continue;
                    }
                    _ => {}
                }
                let mut glow = 0.0;
                for &(lx, ly, r, b) in lamps {
                    let (dx, dy) = ((x as f64 - lx) / ASPECT, yf - ly);
                    let q = 1.0 - (dx * dx + dy * dy) / (r * r);
                    if q > 0.0 {
                        glow += 2.2 * b * q * q;
                    }
                }
                let z = z + glow;
                if z <= NEB_THRESHOLD {
                    continue;
                }
                let v = ((z - NEB_THRESHOLD) / NEB_SPAN).min(1.0);
                // Hubble palette: faint [O III] teal outskirts, bright Hα/[S II] cores.
                let role = if v < 0.5 { Role::Wave } else { Role::Hot };
                let ink = Ink::new(role, (30.0 + 225.0 * v) as u8);
                canvas.put(x as i32, y as i32, draw::ramp(v, &NEBULA), ink);
            }
        }
    }
}

impl Generator for StellarNursery {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.modes.is_empty() {
            self.init_sky(&mut ctx.rng);
        }
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        (self.w, self.h) = (w, h);
        self.col.clear();
        self.col.resize(w, [(0.0, 1.0); MODES]);
        self.row.clear();
        self.row.resize(h, [(0.0, 1.0); MODES]);
        self.row_bias.clear();
        let hf = (h as f64).max(1.0);
        self.row_bias
            .extend((0..h).map(|y| 0.7 * (0.5 - y as f64 / hf)));
        self.build_mask();
        self.refresh_tables();

        let (wr, hr) = (w as f64 / ASPECT, h as f64);
        let span = wr.min(hr).max(1.0);
        self.r_feed = (0.45 * span).clamp(R_SPAWN_MIN + 1.0, 18.0);
        self.r_hii = (0.3 * span).clamp(2.0, 9.0);
        // Free fall from rest: t_ff = (π/2)·√(r³ / 2gm) (ε ≪ r) ⇒ gm fixes the timescale.
        let half_pi = std::f64::consts::FRAC_PI_2;
        self.gm = half_pi * half_pi * self.r_feed.powi(3) / (2.0 * T_FF * T_FF);

        let cap = ((GAS_PER_AREA * size.area() as f64) as usize).clamp(8, GAS_MAX);
        self.gas.set_capacity(cap);
        self.gas.clear();
        let jet_cap = (size.area() / 60).clamp(4, JET_MAX);
        self.jets.set_capacity(jet_cap);
        self.jets.clear();
        self.spawn_acc = 0.0;

        self.stars.clear();
        let n = ctx.rng.random_range(3..=5);
        for k in 0..n {
            // Some protostars sit in the pillar heads (evaporating gaseous globules).
            let (x, y) = match self.tips.get(k) {
                Some(&tip) if k + 1 < n && self.in_bounds(tip.0, tip.1) => tip,
                _ => self.place_star(&mut ctx.rng, None),
            };
            let mass = ctx.rng.random_range(0.0..=0.8) * M_IGN;
            let mut s = Protostar::new(x, y, &mut ctx.rng, 0, mass);
            // One star is already shining, so the life cycles are staggered from the start.
            if k + 1 == n {
                s.mass = M_IGN;
                s.phase = Phase::Shining(ctx.rng.random_range(2.0..=10.0));
            }
            self.stars.push(s);
        }
        // Each hub keeps ≈ cap/n parcels in flight, each arriving after ≈ T_ARRIVE, so it
        // accretes ≈ (cap/n)·m/T_ARRIVE per second; m gives an ACC_TIME build-up at any size.
        self.gas_mass = M_IGN * T_ARRIVE * n as f64 / (ACC_TIME * cap as f64);
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.w == 0 || self.h == 0 {
            return;
        }
        let n = (dt / MAX_STEP).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            self.substep(h, &mut ctx.rng);
        }
        self.refresh_tables();
    }

    fn render(&self, canvas: &mut Canvas) {
        if self.w == 0 || self.h == 0 || self.col.len() != self.w || self.row.len() != self.h {
            return;
        }
        self.render_nebula(canvas);

        for g in &self.gas {
            let Some(s) = self.stars.get(g.star) else {
                continue;
            };
            let (x, y) = g.pos(s);
            let close = (1.0 - g.r / self.r_feed).clamp(0.0, 1.0);
            let fade_in = (g.age / 0.4).min(1.0);
            let ink = Ink::new(Role::Particle, ((80.0 + 175.0 * close) * fade_in) as u8);
            plot(canvas, x * ASPECT, y, draw::ramp(close, &GAS), ink);
        }

        for j in &self.jets {
            let f = 1.0 - j.age / j.life;
            let glyph = if j.age < 0.6 * j.life {
                JET_DIRS[usize::from(j.dir)]
            } else {
                KNOT
            };
            plot(
                canvas,
                j.x * ASPECT,
                j.y,
                glyph,
                Ink::new(Role::Accent, (70.0 + 185.0 * f) as u8),
            );
        }

        for s in &self.stars {
            let (cx, cy) = (s.x * ASPECT, s.y);
            match s.phase {
                Phase::Accreting => {
                    let m = (s.mass / M_IGN).clamp(0.0, 1.0);
                    let g = draw::ramp(m, &EMBEDDED);
                    plot(
                        canvas,
                        cx,
                        cy,
                        g,
                        Ink::new(Role::Star, (70.0 + 110.0 * m) as u8),
                    );
                }
                _ => {
                    let b = s.brightness();
                    if b <= 0.02 {
                        continue;
                    }
                    // Brief flash at ignition: longer diffraction spikes for ~2 s.
                    let flash = match s.phase {
                        Phase::Shining(a) => (-(a - IGNITION_RAMP).max(0.0) / 1.2).exp() * b,
                        _ => 0.0,
                    };
                    let spike = Ink::new(Role::Star, (60.0 + 150.0 * b) as u8);
                    let arm = (1.0 + 3.0 * flash).round() as i32;
                    if b > 0.3 {
                        for k in 1..=arm {
                            let k = f64::from(k);
                            plot(canvas, cx - k, cy, SPIKE_H, spike);
                            plot(canvas, cx + k, cy, SPIKE_H, spike);
                        }
                        let varm = if flash > 0.3 { 2.0 } else { 1.0 };
                        for k in 1..=varm as i32 {
                            let k = f64::from(k);
                            plot(canvas, cx, cy - k, SPIKE_V, spike);
                            plot(canvas, cx, cy + k, SPIKE_V, spike);
                        }
                    }
                    plot(
                        canvas,
                        cx,
                        cy,
                        STAR,
                        Ink::new(Role::Star, (80.0 + 175.0 * b) as u8),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn scene(seed: u64, w: u16, h: u16) -> (StellarNursery, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = StellarNursery::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn nur_a1_infall_step_never_increases_distance() {
        for gm in [0.0, 1.0, 50.0, 500.0, 5000.0] {
            for r0 in [0.81, 1.0, 3.0, 10.0, 40.0] {
                for dt in [1e-4, 1.0 / 120.0, 1.0 / 60.0, 0.25] {
                    let (mut r, mut vr) = (r0, 0.0);
                    for _ in 0..200 {
                        let (r1, vr1) = infall_step(r, vr, gm, dt);
                        assert!(r1 <= r, "gm {gm} r {r} -> {r1}");
                        assert!(vr1 >= vr, "inward speed must not drop");
                        assert!(r1 >= 0.0 && r1.is_finite());
                        (r, vr) = (r1, vr1);
                    }
                }
            }
        }
    }

    #[test]
    fn nur_a1_infall_matches_softened_gravity() {
        // One kick of dt: Δv = gm/(r²+ε²)·dt exactly, then drift by the new speed.
        let (r1, vr1) = infall_step(5.0, 1.0, 26.0, 0.01);
        let a = 26.0 / (25.0 + EPS * EPS);
        assert!((vr1 - (1.0 + a * 0.01)).abs() < 1e-12);
        assert!((r1 - (5.0 - vr1 * 0.01)).abs() < 1e-12);
        assert!((gravity_accel(26.0, 5.0) - a).abs() < 1e-12);
    }

    #[test]
    fn nur_a1_gas_distance_to_target_is_non_increasing_in_the_scene() {
        for seed in [1, 7] {
            let (mut g, mut ctx) = scene(seed, 160, 48);
            let mut seen = 0;
            for _ in 0..900 {
                let before: HashMap<u32, (usize, u32, f64)> = g
                    .gas
                    .iter()
                    .map(|p| {
                        let (x, y) = p.pos(&g.stars[p.star]);
                        let s = &g.stars[p.star];
                        (p.id, (p.star, p.generation, (x - s.x).hypot(y - s.y)))
                    })
                    .collect();
                g.update(SIM_DT, &mut ctx);
                for p in &g.gas {
                    if let Some(&(star, generation, d0)) = before.get(&p.id) {
                        assert_eq!((star, generation), (p.star, p.generation), "target changed");
                        let s = &g.stars[p.star];
                        let (x, y) = p.pos(s);
                        let d1 = (x - s.x).hypot(y - s.y);
                        assert!(d1 <= d0 + 1e-9, "seed {seed}: {d0} -> {d1}");
                        assert!(d1 > R_ABS, "unabsorbed particle inside R_ABS");
                        seen += 1;
                    }
                }
            }
            assert!(seen > 10_000, "too few particles tracked: {seen}");
        }
    }

    #[test]
    fn nur_a2_accrete_ignites_exactly_when_mass_first_reaches_m_ign() {
        let mut rng = Ctx::new(3, Size::new(10, 10)).rng;
        let mut s = Protostar::new(0.0, 0.0, &mut rng, 0, 0.0);
        let mut ignitions = 0;
        let mut last = s.mass;
        for k in 1..=40 {
            let crossed_now = last < M_IGN && last + 0.07 >= M_IGN;
            let ignited = s.accrete(0.07);
            assert!(s.mass >= last, "mass decreased");
            assert_eq!(
                ignited, crossed_now,
                "step {k}: ignition must coincide with crossing"
            );
            if ignited {
                ignitions += 1;
                assert_eq!(s.phase, Phase::Shining(0.0));
            }
            last = s.mass;
        }
        assert_eq!(ignitions, 1);
        // Negative or NaN mass is ignored: never decreases, never ignites spuriously.
        let mut t = Protostar::new(0.0, 0.0, &mut rng, 0, 0.5);
        assert!(!t.accrete(-1.0));
        assert!(!t.accrete(f64::NAN));
        assert_eq!(t.mass, 0.5);
        assert!(t.accrete(0.5), "exactly M_IGN ignites");
    }

    #[test]
    fn nur_a2_scene_mass_is_monotone_and_ignition_is_at_threshold() {
        for seed in [2, 11] {
            let (mut g, mut ctx) = scene(seed, 160, 48);
            let mut ignitions = 0;
            for _ in 0..(60 * 60) {
                let before = g.stars.clone();
                g.update(SIM_DT, &mut ctx);
                for (a, b) in before.iter().zip(&g.stars) {
                    if a.generation != b.generation {
                        assert_eq!(b.phase, Phase::Accreting, "replacement starts embedded");
                        assert!(b.mass < M_IGN);
                        continue;
                    }
                    assert!(b.mass >= a.mass, "seed {seed}: mass decreased");
                    match (a.phase, b.phase) {
                        (Phase::Accreting, Phase::Accreting) => assert!(b.mass < M_IGN),
                        (Phase::Accreting, Phase::Shining(_)) => {
                            assert!(a.mass < M_IGN && b.mass >= M_IGN);
                            ignitions += 1;
                        }
                        (Phase::Accreting, Phase::Fading(_)) => panic!("skipped shining"),
                        _ => {}
                    }
                }
            }
            assert!(
                ignitions >= 3,
                "seed {seed}: only {ignitions} ignitions in 60 s"
            );
        }
    }

    #[test]
    fn nur_a3_pools_never_exceed_capacity() {
        let (mut g, mut ctx) = scene(5, 200, 50);
        let sizes = [
            (200, 50),
            (400, 100),
            (30, 8),
            (1, 1),
            (260, 80),
            (0, 0),
            (120, 40),
        ];
        for (k, &(w, h)) in sizes.iter().enumerate() {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            let gas_cap = ((GAS_PER_AREA * size.area() as f64) as usize).clamp(8, GAS_MAX);
            assert_eq!(g.gas.capacity(), gas_cap);
            assert!(g.jets.capacity() <= JET_MAX);
            for i in 0..600 {
                let dt = if (i + k) % 7 == 0 { 0.25 } else { SIM_DT };
                g.update(dt, &mut ctx);
                assert!(g.gas.len() <= g.gas.capacity() && g.gas.capacity() <= GAS_MAX);
                assert!(g.jets.len() <= g.jets.capacity());
            }
        }
    }

    #[test]
    fn nur_ignition_ramps_over_one_second_then_fades_and_is_replaced() {
        let (mut g, mut ctx) = scene(4, 120, 40);
        g.stars[0].mass = M_IGN - 1e-9;
        assert!(g.stars[0].accrete(1e-9));
        let (x0, gen0) = (g.stars[0].x, g.stars[0].generation);
        let mut b = Vec::new();
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            b.push(g.stars[0].brightness());
        }
        assert!(b.windows(2).all(|w| w[1] >= w[0]), "ramp must be monotone");
        assert!(b[29] > 0.2 && b[29] < 0.8);
        assert!((b[59] - 1.0).abs() < 1e-9);
        assert!(!g.jets.is_empty(), "an ignited star launches jets");
        for _ in 0..((SHINE + FADE) * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let s = g.stars[0];
        assert_eq!(s.generation, gen0 + 1, "replaced after shining and fading");
        assert_eq!(s.phase, Phase::Accreting);
        let _ = x0;
    }

    #[test]
    fn nur_pillars_occlude_the_nebula() {
        let (g, _) = scene(8, 160, 48);
        let mut canvas = Canvas::new(Size::new(160, 48), GlyphMode::Unicode);
        g.render(&mut canvas);
        let dust = g.mask.iter().filter(|&&m| m == M_DUST).count();
        assert!(dust > 50, "pillars too small: {dust}");
        for (i, &m) in g.mask.iter().enumerate() {
            if m == M_DUST {
                let c = canvas.cells()[i];
                assert!(
                    c.ink.role == Role::Structure || c.ink.role == Role::Star,
                    "nebula drawn through a pillar at {i}"
                );
            }
        }
    }
}
