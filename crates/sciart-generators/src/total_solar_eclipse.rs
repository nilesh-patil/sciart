//! Totality seen from the ground (SPEC-002 `total_solar_eclipse`): the Moon slides over
//! the Sun, the last sunlight breaks into Baily's beads and a diamond ring, then the corona
//! blooms with long streamers and small pink prominences at the limb; then it all runs
//! backwards.
//!
//! Geometry is in solar radii about the Sun's centre. The Moon (1.03 R☉) moves along a
//! tilted chord through the Sun's centre; its offset `d(t)` is driven by a `Timeline`, so
//! the eclipse is symmetric in time (ECL-A1). The Moon's limb is a seeded valley profile:
//! once its mountain-top envelope covers the Sun, sunlight shows only through valleys whose
//! floors still dip inside the photosphere, which are the beads (ECL-A2).
//!
//! M16 (GEN-R25, thread style): the corona is braille streamer threads along a dipole-like
//! field (closed helmet loops `r = L sin²θ`, open field fanning out along the separatrix and
//! running radially past the cusp), stepping down in brightness outward; the photosphere a
//! checkerboard of braille dots; nothing on the Moon's black disc. Screen lengths are dots.

use std::cell::RefCell;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::stellar;
use sciart_core::timeline::ease;

use crate::shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "total_solar_eclipse",
    category: Category::EarthSky,
    description: "The Moon hides the Sun: Baily's beads, the diamond ring, the pearly corona",
    title: "Total eclipse · Moon hides Sun",
    captions: &["partial", "diamond ring", "totality"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Eclipse::default())
}

/// Moon radius (solar radii) and the depth of its deepest limb valley (stylised: real
/// lunar relief is ~0.002 R☉; deepened so beads last long enough to see).
const MOON_R: f64 = 1.03;
const VALLEY: f64 = 0.02;
/// Limb profile samples (one per degree) and valleys cut into it.
const LIMB_N: usize = 360;
const VALLEYS: usize = 14;
/// Where the Moon starts and ends, beyond first contact.
const START: f64 = MOON_R + 1.0 + 0.08;
/// The Moon's path, tilted from the horizontal (rad).
const PATH_TILT: f64 = -0.35;
/// Acts (s): partial, beads and diamond ring, totality, and back (ECL-A5).
const PHASES: [(&str, f64); 5] = [
    ("partial", 8.0),
    ("diamond ring", 4.0),
    ("totality", 14.0),
    ("diamond ring", 4.0),
    ("partial", 8.0),
];
/// Photosphere: limb darkening (quadratic, visible light).
const LIMB_DARK: (f64, f64) = (0.47, 0.23);
/// The corona shows once the uncovered fraction falls below this (ECL-A3).
const CORONA_F: f64 = 0.01;
/// The corona's field (ECL-A12): closed loops' `L` under each helmet (plus its cusp
/// `L_max`, seeded per side), open lines per pole and side (the last `STALK` crowd the
/// separatrix), how far out they run (R☉) and the brightness falloff along them.
const LOOPS: [f64; 5] = [1.15, 1.3, 1.5, 1.75, 2.05];
const CUSP: (f64, f64) = (2.2, 2.6);
const OPEN: usize = 9;
const STALK: [f64; 3] = [0.93, 0.96, 0.985];
const R_OUT: f64 = 4.0;
const FALLOFF: f64 = 2.2;
/// Line gains: seeded range, and the boosts of the stalk and the closed loops.
const GAIN: (f64, f64) = (0.5, 1.0);
const STALK_GAIN: f64 = 1.6;
const LOOP_GAIN: f64 = 1.3;
/// Segment length along a thread (dots).
const SEG: f64 = 2.0;
/// The corona's steps, brightest first: `(from, ink)`, `e ≥ from`.
pub(crate) const CORONA_STEPS: [(f64, Ink); 4] = [
    (0.55, Ink::new(Role::Star, 230)),
    (0.25, Ink::new(Role::Star, 110)),
    (0.1, Ink::new(Role::Structure, 100)),
    (0.035, Ink::new(Role::Structure, 0)),
];
/// The hills' top edge, the only lit land.
const SKYLINE: Ink = Ink::new(Role::Dim, 60);
/// Prominences (pink H-alpha, M11's `Accent`), beads, the Moon's limb in totality.
const PROMINENCE: Ink = Ink::new(Role::Accent, 200);
const BEAD_CORE: Ink = Ink::new(Role::Star, 255);
const MOON_RING: Ink = Ink::new(Role::Star, 200);
/// The diamond's spike inks and least reach in dots.
const SPIKE_AXIS: Ink = Ink::new(Role::Star, 230);
const SPIKE_DIAGONAL: Ink = Ink::new(Role::Sun, 200);
const SPIKE_DOTS: f64 = 8.0;
/// Prominences: count range, heights (solar radii).
const PROMS: (usize, usize) = (3, 6);
const PROM_H: (f64, f64) = (0.07, 0.16);
/// Beads are drawn this far outside the lunar limb (in half-block pixels, two dots).
const BEAD_LIFT: f64 = 0.8;
/// Hills on the horizon.
const HILLS: Terrain = Terrain {
    ridge: (0.8, 3.0),
    trees: Some((0.8, 2.0)),
    gap: (0.4, 1.5),
    clearing: 0.5,
};

/// The step of corona brightness `e`: 4 brightest … 1 faintest, 0 not drawn.
pub(crate) fn corona_rank(e: f64) -> usize {
    CORONA_STEPS
        .iter()
        .position(|&(from, _)| e >= from)
        .map_or(0, |k| CORONA_STEPS.len() - k)
}

/// The ink of corona step `rank` (1–4).
fn rank_ink(rank: usize) -> Option<Ink> {
    (1..=CORONA_STEPS.len())
        .contains(&rank)
        .then(|| CORONA_STEPS[CORONA_STEPS.len() - rank].1)
}

/// The photosphere by its limb-darkened brightness `i` (0–1): a yellow-white centre,
/// yellow, an orange limb.
fn photosphere_ink(i: f64) -> Ink {
    match i {
        i if i > 0.93 => Ink::new(Role::Sun, 255),
        i if i > 0.8 => Ink::new(Role::Sun, 170),
        _ => Ink::new(Role::Hot, 200),
    }
}

/// Area of the lens where discs of radius `r1`, `r2` with centres `d` apart overlap.
pub(crate) fn overlap(r1: f64, r2: f64, d: f64) -> f64 {
    let d = d.abs();
    if d >= r1 + r2 {
        return 0.0;
    }
    if d <= (r1 - r2).abs() {
        let r = r1.min(r2);
        return PI * r * r;
    }
    let a1 = ((d * d + r1 * r1 - r2 * r2) / (2.0 * d * r1))
        .clamp(-1.0, 1.0)
        .acos();
    let a2 = ((d * d + r2 * r2 - r1 * r1) / (2.0 * d * r2))
        .clamp(-1.0, 1.0)
        .acos();
    let k = ((-d + r1 + r2) * (d + r1 - r2) * (d - r1 + r2) * (d + r1 + r2)).max(0.0);
    r1 * r1 * a1 + r2 * r2 * a2 - 0.5 * k.sqrt()
}

/// The Moon's seeded limb: valley depth (solar radii) per degree.
#[derive(Debug, Clone)]
pub(crate) struct Limb {
    depth: [f64; LIMB_N],
}

impl Default for Limb {
    fn default() -> Self {
        Self {
            depth: [0.0; LIMB_N],
        }
    }
}

impl Limb {
    fn seeded(rng: &mut impl Rng) -> Self {
        let mut depth = [0.0; LIMB_N];
        for _ in 0..VALLEYS {
            let at = rng.random_range(0.0..LIMB_N as f64);
            let half = rng.random_range(1.5..5.0);
            let deep = VALLEY * rng.random_range(0.35..1.0);
            for (i, d) in depth.iter_mut().enumerate() {
                let off = (i as f64 - at + LIMB_N as f64 * 1.5).rem_euclid(LIMB_N as f64)
                    - LIMB_N as f64 / 2.0;
                let v = deep * (1.0 - (off / half).powi(2)).max(0.0);
                *d = f64::max(*d, v);
            }
        }
        Self { depth }
    }

    /// Light through each valley, as `(angle, floor point, penetration)`: the valley floor
    /// at angle `θ` lies `MOON_R − depth` from the Moon's centre `m`; it lets sunlight
    /// through when it is inside the Sun, by `1 − |floor|` solar radii.
    fn beads(&self, m: (f64, f64)) -> impl Iterator<Item = (f64, (f64, f64), f64)> + '_ {
        self.depth.iter().enumerate().filter_map(move |(i, &v)| {
            let a = TAU * i as f64 / LIMB_N as f64;
            let (s, c) = a.sin_cos_det();
            let p = (m.0 + (MOON_R - v) * c, m.1 + (MOON_R - v) * s);
            let gap = 1.0 - p.0.hypot(p.1);
            (gap > 0.0).then_some((a, p, gap))
        })
    }
}

/// The share of the Sun's disc left uncovered with the Moon `d` from its centre: the
/// smooth envelope's crescent plus, once the envelope covers the Sun, the light through
/// the valleys (each one degree of limb wide).
pub(crate) fn uncovered(d: f64, limb: &Limb) -> f64 {
    let smooth = 1.0 - overlap(1.0, MOON_R, d) / PI;
    if smooth > 1e-12 {
        return smooth;
    }
    let (s, c) = PATH_TILT.sin_cos_det();
    let arc = TAU / LIMB_N as f64;
    limb.beads((d * c, d * s))
        .map(|(_, _, gap)| gap * arc)
        .sum::<f64>()
        / PI
}

/// The Moon's offset along its path at loop time `t` (ECL-A1): approach, the beads while
/// the valleys close, totality, and out again, symmetric about mid-totality.
pub(crate) fn moon_offset(timeline: &Timeline, t: f64) -> f64 {
    let beat = timeline.at(t);
    let contact = MOON_R - 1.0; // the envelope just covers the Sun
    let closed = contact - VALLEY; // the deepest valley has closed
    let p = beat.progress;
    match beat.index {
        0 => -START + (START - contact) * p,
        1 => -contact + (contact - closed) * p,
        2 => -closed + 2.0 * closed * p,
        3 => closed + (contact - closed) * p,
        _ => contact + (START - contact) * p,
    }
}

/// A corona thread's kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    /// A closed loop `r = L sin²θ`.
    Closed { l: f64 },
    /// An open line from footpoint colatitude `s·θ_c` (north pole, or south mirrored).
    Open { s: f64, south: bool, stalk: bool },
}

/// One corona thread: its kind, the side of the dipole axis it lies on (±1) and its gain.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Thread {
    kind: Kind,
    side: f64,
    gain: f64,
}

/// The corona's field (ECL-A12): a dipole tilted to `equator`, its two helmets' cusps.
#[derive(Debug, Clone, Default)]
pub(crate) struct Corona {
    equator: f64,
    /// Cusp `L_max` on each side (−1, +1).
    lmax: [f64; 2],
    threads: Vec<Thread>,
}

impl Corona {
    fn seeded(rng: &mut impl Rng) -> Self {
        let equator = rng.random_range(-0.3..0.3);
        let lmax = [
            rng.random_range(CUSP.0..CUSP.1),
            rng.random_range(CUSP.0..CUSP.1),
        ];
        let mut threads = Vec::with_capacity(2 * (LOOPS.len() + 1) + 4 * OPEN);
        for side in [-1.0, 1.0] {
            let lm = lmax[usize::from(side > 0.0)];
            for l in LOOPS.iter().copied().chain([lm]) {
                threads.push(Thread {
                    kind: Kind::Closed { l },
                    side,
                    gain: LOOP_GAIN * rng.random_range(GAIN.0..GAIN.1),
                });
            }
            for south in [false, true] {
                let fan = OPEN - STALK.len();
                for k in 0..OPEN {
                    let (s, stalk) = if k < fan {
                        let s = (k as f64 + 0.5) / fan as f64 * 0.88;
                        (
                            (s + rng.random_range(-0.025..0.025)).clamp(0.02, 0.9),
                            false,
                        )
                    } else {
                        (STALK[k - fan], true)
                    };
                    let boost = if stalk { STALK_GAIN } else { 1.0 };
                    threads.push(Thread {
                        kind: Kind::Open { s, south, stalk },
                        side,
                        gain: boost * rng.random_range(GAIN.0..GAIN.1),
                    });
                }
            }
        }
        Self {
            equator,
            lmax,
            threads,
        }
    }

    /// The cusp on a thread's side.
    fn cusp(&self, t: &Thread) -> f64 {
        self.lmax[usize::from(t.side > 0.0)]
    }

    /// Point `k` of `n` along thread `t` as `(r, colatitude)`, from its first footpoint
    /// outward (open) or across (closed).
    pub(crate) fn point(&self, t: &Thread, k: usize, n: usize) -> (f64, f64) {
        let u = k as f64 / n.max(1) as f64;
        let lm = self.cusp(t);
        match t.kind {
            Kind::Closed { l } => {
                let tf = (1.0 / l.max(1.0)).sqrt().asin();
                let th = tf + (PI - 2.0 * tf) * u;
                let s = th.sin_det();
                (l * s * s, th)
            }
            Kind::Open { s, south, .. } => {
                // Inside the cusp: along the separatrix's colatitude φ ∈ [θ_c, π/2] scaled
                // by `s`; beyond it, radially out to `R_OUT`.
                let tc = (1.0 / lm).sqrt().asin();
                let split = (lm - 1.0) / (R_OUT - 1.0);
                let (r, sep) = if u <= split {
                    let phi = tc + (FRAC_PI_2 - tc) * (u / split.max(1e-9));
                    let sp = phi.sin_det();
                    (lm * sp * sp, phi)
                } else {
                    (lm + (R_OUT - lm) * (u - split) / (1.0 - split), FRAC_PI_2)
                };
                let th = s * sep;
                (r.max(1.0), if south { PI - th } else { th })
            }
        }
    }

    /// Screen angle (rad, y down) of colatitude `th` on side `side`.
    fn angle(&self, th: f64, side: f64) -> f64 {
        self.equator - FRAC_PI_2 + side * th
    }
}

#[derive(Debug, Clone, Copy)]
struct Prominence {
    angle: f64,
    half: f64,
    height: f64,
}

/// A precomputed corona segment: its ends (dots), radius at its middle (R☉), brightness
/// before the visibility (`gain·r^−FALLOFF`) and its thread.
#[derive(Debug, Clone, Copy)]
struct Seg {
    a: (f32, f32),
    b: (f32, f32),
    #[cfg(test)]
    r: f32,
    base: f32,
    thread: u16,
}

#[derive(Debug, Default)]
pub struct Eclipse {
    timeline: Option<Timeline>,
    limb: Limb,
    corona_model: Corona,
    proms: Vec<Prominence>,
    /// Picture geometry: Sun centre and solar radius, in dots.
    centre: (f64, f64),
    radius: f64,
    /// The corona's segments and the prominences' dots, built in `resize`.
    segs: Vec<Seg>,
    prom_dots: Vec<(i32, i32)>,
    /// Per dot column: the skyline's dot row.
    ridge: Vec<i32>,
    /// A few bright planets and stars that come out in totality (dots, level).
    lights: Vec<(i32, i32, u8)>,
    /// The braille lattice; a cell so `prepare` can draw into it while reading the
    /// rest of `self` (no placeholder lattice allocated per frame, GEN-R11).
    dots: RefCell<Dots>,
    t: f64,
    /// Uncovered fraction and Moon offset now.
    f: f64,
    d: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl Eclipse {
    fn timeline(&self) -> Option<&Timeline> {
        self.timeline.as_ref()
    }

    /// Solar radii → dot position.
    fn to_dot(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.centre.0 + p.0 * self.radius,
            self.centre.1 + p.1 * self.radius,
        )
    }

    fn moon(&self) -> (f64, f64) {
        let (s, c) = PATH_TILT.sin_cos_det();
        (self.d * c, self.d * s)
    }

    /// Whether the Timeline is in one of the two diamond-ring acts.
    fn diamond_act(&self) -> bool {
        self.timeline()
            .is_some_and(|tl| matches!(tl.at(self.t).index, 1 | 3))
    }

    /// Corona visibility: 0 while the photosphere's glare drowns it, 1 in totality.
    fn corona_vis(&self) -> f64 {
        ease::smoothstep((CORONA_F - self.f) / CORONA_F)
    }

    /// Whether dot `p` lies under the land (below the skyline).
    fn under_land(&self, p: (f64, f64)) -> bool {
        let x = p.0.round();
        if !(x >= 0.0 && (x as usize) < self.ridge.len()) {
            return false;
        }
        p.1 >= f64::from(self.ridge[x as usize])
    }

    /// Build the corona's segments and the prominences' dots for this geometry.
    fn build_corona(&mut self) {
        self.segs.clear();
        let r_dots = self.radius.max(1e-9);
        let model = &self.corona_model;
        for (ti, t) in model.threads.iter().enumerate() {
            let len = match t.kind {
                Kind::Closed { l } => 2.2 * l,
                Kind::Open { .. } => R_OUT,
            } * r_dots;
            let n = (len / SEG).ceil().max(8.0) as usize;
            let at = |k: usize| {
                let (r, th) = model.point(t, k, n);
                let (s, c) = model.angle(th, t.side).sin_cos_det();
                (
                    r,
                    (
                        self.centre.0 + r * r_dots * c,
                        self.centre.1 + r * r_dots * s,
                    ),
                )
            };
            let mut prev = at(0);
            for k in 1..=n {
                let next = at(k);
                let r = 0.5 * (prev.0 + next.0);
                let mid = (0.5 * (prev.1.0 + next.1.0), 0.5 * (prev.1.1 + next.1.1));
                if !self.under_land(mid) {
                    self.segs.push(Seg {
                        a: (prev.1.0 as f32, prev.1.1 as f32),
                        b: (next.1.0 as f32, next.1.1 as f32),
                        #[cfg(test)]
                        r: r as f32,
                        base: (t.gain * r.powf(-FALLOFF)) as f32,
                        thread: ti as u16,
                    });
                }
                prev = next;
            }
        }
        // Prominences: tiny dotted arches on the limb.
        self.prom_dots.clear();
        for pr in &self.proms {
            let n = (2.0 * pr.half * r_dots * 1.5).ceil().max(4.0) as i32;
            for k in 0..=n {
                let off = 2.0 * f64::from(k) / f64::from(n) - 1.0;
                let rr = 1.0 + pr.height * (1.0 - off * off).max(0.0).sqrt();
                let (s, c) = (pr.angle + off * pr.half).sin_cos_det();
                let p = (
                    (self.centre.0 + rr * r_dots * c).round() as i32,
                    (self.centre.1 + rr * r_dots * s).round() as i32,
                );
                // Every other dot along the arch.
                if k % 2 == 0 && self.prom_dots.last() != Some(&p) {
                    self.prom_dots.push(p);
                }
            }
        }
    }

    /// Every corona segment drawn now: `(a, b, rank, thread)` (ECL-A3, ECL-A6): stepped by
    /// its brightness × the visibility, one step darker in the diamond-ring acts, hidden
    /// behind the Moon.
    fn corona(&self, mut f: impl FnMut((f64, f64), (f64, f64), usize, usize)) {
        let vis = self.corona_vis();
        if vis <= 0.0 {
            return;
        }
        let hold = usize::from(self.diamond_act());
        let m = self.to_dot(self.moon());
        let mr = MOON_R * self.radius;
        for s in &self.segs {
            let rank = corona_rank(f64::from(s.base) * vis).saturating_sub(hold);
            if rank == 0 {
                continue;
            }
            let (a, b) = (
                (f64::from(s.a.0), f64::from(s.a.1)),
                (f64::from(s.b.0), f64::from(s.b.1)),
            );
            let mid = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
            if (mid.0 - m.0).hypot(mid.1 - m.1) <= mr + 0.5 {
                continue;
            }
            f(a, b, rank, usize::from(s.thread));
        }
    }

    /// The uncovered photosphere as a checkerboard of dots, by limb darkening (ECL-A9).
    fn photosphere(&self, mut f: impl FnMut(i32, i32, Ink)) {
        if self.f <= 0.0 {
            return;
        }
        let (c, r) = (self.centre, self.radius);
        let m = self.to_dot(self.moon());
        let mr = MOON_R * r;
        let (x0, x1) = ((c.0 - r).floor() as i32, (c.0 + r).ceil() as i32);
        let (y0, y1) = ((c.1 - r).floor() as i32, (c.1 + r).ceil() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if (x + y) % 2 != 0 {
                    continue;
                }
                let (fx, fy) = (f64::from(x), f64::from(y));
                let rr = (fx - c.0).hypot(fy - c.1) / r;
                if rr > 1.0 || (fx - m.0).hypot(fy - m.1) <= mr {
                    continue;
                }
                let mu = (1.0 - rr * rr).max(0.0).sqrt();
                f(
                    x,
                    y,
                    photosphere_ink(stellar::limb_darkening(mu, LIMB_DARK).sqrt()),
                );
            }
        }
    }

    /// Baily's beads as small dot discs beyond the lunar limb, and the diamond ring
    /// (ECL-A11): through its acts a bigger bead at the contact point with 8 braille spikes,
    /// white along the axes and yellow on the diagonals, kept off the Moon.
    fn beads(&self, dots: &mut Dots) {
        let m = self.moon();
        let (mc, mr) = (self.to_dot(m), MOON_R * self.radius);
        let clear = |x: f64, y: f64| (x - mc.0).hypot(y - mc.1) > mr + 0.5;
        let lift = 2.0 * BEAD_LIFT / self.radius.max(1e-9);
        if 1.0 - overlap(1.0, MOON_R, self.d) / PI <= 1e-12 {
            for (_, p, gap) in self.limb.beads(m) {
                let (ox, oy) = (p.0 - m.0, p.1 - m.1);
                let len = ox.hypot(oy).max(1e-9);
                let k = (MOON_R - len + lift) / len;
                let (x, y) = self.to_dot((p.0 + ox * k, p.1 + oy * k));
                let rad = 0.6 + 1.4 * gap / VALLEY;
                let ri = rad.ceil() as i32;
                for dy in -ri..=ri {
                    for dx in -ri..=ri {
                        let (px, py) = (x.round() + f64::from(dx), y.round() + f64::from(dy));
                        if (px - x).hypot(py - y) <= rad && clear(px, py) {
                            dots.set(px as i32, py as i32, BEAD_CORE);
                        }
                    }
                }
            }
        }
        if !self.diamond_act() {
            return;
        }
        let len = m.0.hypot(m.1).max(1e-9);
        let out = 1.0 + lift;
        let (bx, by) = self.to_dot((-m.0 / len * out, -m.1 / len * out));
        for dy in -2..=2 {
            for dx in -2..=2 {
                let (px, py) = (bx.round() + f64::from(dx), by.round() + f64::from(dy));
                if (px - bx).hypot(py - by) <= 2.0 && clear(px, py) {
                    dots.set(px as i32, py as i32, BEAD_CORE);
                }
            }
        }
        let reach = SPIKE_DOTS.max(1.6 * 0.5 * self.radius);
        for k in 0..8 {
            let (s, c) = (f64::from(k) * TAU / 8.0).sin_cos_det();
            let (r, ink) = if k % 2 == 0 {
                (reach, SPIKE_AXIS)
            } else {
                (0.55 * reach, SPIKE_DIAGONAL)
            };
            let steps = (2.0 * r) as i32;
            for j in 6..=steps {
                let d = 0.5 * f64::from(j);
                let (x, y) = (bx + d * c, by + d * s);
                if clear(x, y) {
                    dots.plot(x, y, ink);
                }
            }
        }
    }

    fn paint(&mut self) {
        let mut dots = self.dots.borrow_mut();
        dots.clear();
        // The land's skyline: one dot per column.
        for (x, &y) in self.ridge.iter().enumerate() {
            dots.set(x as i32, y, SKYLINE);
        }
        self.stars(&mut dots);
        self.corona(|a, b, rank, _| {
            if let Some(ink) = rank_ink(rank) {
                dots.line(a.0, a.1, b.0, b.1, ink);
            }
        });
        let vis = self.corona_vis();
        let m = self.to_dot(self.moon());
        let mr = MOON_R * self.radius;
        if vis > 0.3 {
            for &(x, y) in &self.prom_dots {
                if (f64::from(x) - m.0).hypot(f64::from(y) - m.1) > mr + 0.5 {
                    dots.set(x, y, PROMINENCE);
                }
            }
        }
        if vis > 0.5 && !self.diamond_act() {
            let r = mr + 0.6;
            let n = (TAU * r).ceil().max(8.0) as usize;
            for k in 0..n {
                let (s, c) = (TAU * k as f64 / n as f64).sin_cos_det();
                let p = (m.0 + r * c, m.1 + r * s);
                if !self.under_land(p) {
                    dots.plot(p.0, p.1, MOON_RING);
                }
            }
        }
        self.photosphere(|x, y, ink| dots.set(x, y, ink));
        self.beads(&mut dots);
        drop(dots);
    }

    /// Planets and bright stars come out as the sky darkens: Venus first (brightest), then
    /// the others, as the light fails.
    fn stars(&self, dots: &mut Dots) {
        let show = ease::smoothstep((0.12 - self.f) / 0.12);
        if show <= 0.0 {
            return;
        }
        for &(x, y, level) in &self.lights {
            let l = f64::from(level) * show.powf(1.0 + (255.0 - f64::from(level)) / 40.0);
            if l >= 40.0 {
                dots.set(
                    x,
                    y,
                    Ink::new(Role::Star, if l >= 150.0 { 255 } else { 110 }),
                );
            }
        }
    }
}

impl Generator for Eclipse {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.dots.get_mut().resize(size);
        let ds = self.dots.get_mut().dot_size();
        let (dw, dh) = (f64::from(ds.width()), f64::from(ds.height()));
        if self.timeline.is_none() {
            self.timeline = Some(Timeline::new(&PHASES));
        }
        self.limb = Limb::seeded(&mut ctx.rng);
        self.corona_model = Corona::seeded(&mut ctx.rng);
        let n = ctx.rng.random_range(PROMS.0..=PROMS.1);
        self.proms.clear();
        self.proms.reserve_exact(PROMS.1);
        self.proms.extend((0..n).map(|_| Prominence {
            angle: ctx.rng.random_range(0.0..TAU),
            half: ctx.rng.random_range(0.04..0.12),
            height: ctx.rng.random_range(PROM_H.0..PROM_H.1),
        }));
        // The M15 geometry, in dots (two per half-block pixel each way).
        self.radius = (0.1 * dw).min(0.14 * dh).max(1.0);
        self.centre = (0.5 * dw, 0.44 * dh);
        let deg = (0.5 * dh / 30.0).max(0.1);
        let hills = Skyline::new(&mut ctx.rng, ds.width().into(), deg, HILLS);
        let horizon = 0.97 * dh;
        self.ridge.clear();
        self.ridge.extend(
            (0..usize::from(ds.width()))
                .map(|x| (horizon - hills.height_at(x as f64 + 0.5)).round().max(0.0) as i32),
        );
        // Venus low beside the Sun, Jupiter and a couple of bright stars.
        self.lights.clear();
        for (fx, fy, level) in [
            (0.25, 0.62, 250u8),
            (0.78, 0.3, 200),
            (0.12, 0.2, 130),
            (0.9, 0.7, 120),
        ] {
            self.lights
                .push(((fx * dw) as i32, (fy * dh) as i32, level));
        }
        self.build_corona();
        let tl = self
            .timeline
            .take()
            .unwrap_or_else(|| Timeline::new(&PHASES));
        self.d = moon_offset(&tl, self.t);
        self.f = uncovered(self.d, &self.limb);
        self.timeline = Some(tl);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        if let Some(tl) = self.timeline() {
            self.d = moon_offset(tl, self.t);
        }
        self.f = uncovered(self.d, &self.limb);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.paint();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.borrow().blit(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        self.timeline().map(|tl| tl.at(self.t).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tl() -> Timeline {
        Timeline::new(&PHASES)
    }

    fn built(size: Size) -> (Eclipse, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Eclipse::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    /// Advance `g` to loop time `t`.
    fn run_to(g: &mut Eclipse, ctx: &mut Ctx, t: f64) {
        while g.t < t {
            g.update(SIM_DT, ctx);
        }
        g.prepare();
    }

    fn canvas_of(g: &Eclipse, size: Size) -> Canvas {
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        canvas
    }

    #[test]
    fn ecl_a1_uncovered_fraction_runs_one_to_zero_symmetrically() {
        let limb = Limb::seeded(&mut SplitMix64::new(3));
        let tl = tl();
        let total = tl.total();
        assert_eq!(uncovered(moon_offset(&tl, 0.0), &limb), 1.0);
        assert_eq!(uncovered(moon_offset(&tl, total / 2.0), &limb), 0.0);
        for i in 0..=200 {
            let t = total * f64::from(i) / 200.0 * 0.999;
            let (da, db) = (moon_offset(&tl, t), moon_offset(&tl, total - t));
            let smooth = |d: f64| 1.0 - overlap(1.0, MOON_R, d) / PI;
            assert!((smooth(da) - smooth(db)).abs() < 1e-9, "t = {t}");
            // The beads differ at second and third contact (different valleys), by less
            // than their whole light.
            let (a, b) = (uncovered(da, &limb), uncovered(db, &limb));
            assert!((a - b).abs() < 0.005, "t = {t}: {a} vs {b}");
        }
    }

    #[test]
    fn ecl_a2_beads_only_as_the_last_light_and_on_the_moons_limb() {
        let limb = Limb::seeded(&mut SplitMix64::new(3));
        let tl = tl();
        let mut seen = 0;
        for i in 0..4600 {
            let t = f64::from(i) * 0.01;
            let d = moon_offset(&tl, t);
            let smooth = 1.0 - overlap(1.0, MOON_R, d) / PI;
            if smooth > 1e-12 {
                continue; // beads are drawn only once the envelope covers the Sun
            }
            let (s, c) = PATH_TILT.sin_cos_det();
            let m = (d * c, d * s);
            for (a, p, _) in limb.beads(m) {
                let f = uncovered(d, &limb);
                assert!(f > 0.0 && f < 0.005, "{f}");
                let from_moon = (p.0 - m.0).hypot(p.1 - m.1);
                assert!((from_moon - MOON_R).abs() <= VALLEY + 1e-12);
                assert!((0.0..TAU).contains(&a));
                seen += 1;
            }
        }
        assert!(seen > 20, "{seen}");
    }

    #[test]
    fn ecl_a3_corona_only_once_the_sun_is_nearly_covered() {
        let (mut g, mut ctx) = built(Size::new(80, 30));
        for _ in 0..(46.0 * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            if g.f >= CORONA_F {
                assert_eq!(g.corona_vis(), 0.0, "f = {}", g.f);
                let mut n = 0;
                g.corona(|_, _, _, _| n += 1);
                assert_eq!(n, 0, "f = {}", g.f);
            }
        }
    }

    #[test]
    fn ecl_a4_corona_fades_outward_along_every_thread() {
        let (g, _) = built(Size::new(120, 40));
        for s in &g.segs {
            let t = &g.corona_model.threads[usize::from(s.thread)];
            let want = t.gain * f64::from(s.r).powf(-FALLOFF);
            assert!((f64::from(s.base) - want).abs() < 1e-5 * want.max(1.0));
        }
        // So the step is a non-increasing function of r on every thread.
        for e in [0.03, 0.1, 0.3, 0.6, 1.0] {
            assert!(corona_rank(e) >= corona_rank(0.9 * e));
        }
    }

    #[test]
    fn ecl_a5_captions_follow_the_timeline() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        let tl = tl();
        for i in 0..(46 * 60) {
            let want = tl.at(g.t).label;
            assert_eq!(g.caption(), Some(want));
            assert!(META.captions.contains(&want), "{i}");
            g.update(SIM_DT, &mut ctx);
        }
    }

    #[test]
    fn ecl_a6_threads_step_down_one_step_at_a_time() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        run_to(&mut g, &mut ctx, 19.0);
        let vis = g.corona_vis();
        assert!(vis > 0.99);
        let mut last: Option<(u16, usize)> = None;
        let mut checked = 0;
        for s in &g.segs {
            let rank = corona_rank(f64::from(s.base) * vis);
            if let Some((t, l)) = last
                && t == s.thread
            {
                assert!(l.abs_diff(rank) <= 1, "thread {t}: step {l} → {rank}");
                checked += 1;
            }
            last = Some((s.thread, rank));
        }
        assert!(checked > 200, "{checked}");
    }

    #[test]
    fn ecl_a7_black_sky_and_the_moon_stays_whole() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        let mut t = 0.0;
        while t < 8.0 {
            t += 0.5;
            run_to(&mut g, &mut ctx, t);
            assert_eq!(canvas_of(&g, size).get(2, 1).unwrap().ch, ' ', "t = {t}");
        }
        // Through the beads and diamond ring nothing is lit on the Moon's disc.
        while g.t < 12.0 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let m = g.to_dot(g.moon());
            let rr = MOON_R * g.radius - 1.5;
            for y in (m.1 - rr) as i32..=(m.1 + rr) as i32 {
                for x in (m.0 - rr) as i32..=(m.0 + rr) as i32 {
                    if (f64::from(x) - m.0).hypot(f64::from(y) - m.1) < rr {
                        assert!(!g.dots.borrow().is_lit(x, y), "t = {} ({x},{y})", g.t);
                    }
                }
            }
        }
    }

    #[test]
    fn ecl_a8_stylised_timing_matches_the_spec() {
        // Partial phases 8 s against 14 s of totality, and the Moon moving ~180× faster in
        // the partial phases than through totality (the real motion is uniform).
        let contact = MOON_R - 1.0;
        let partial = (START - contact) / PHASES[0].1;
        let total = 2.0 * (contact - VALLEY) / PHASES[2].1;
        let ratio = partial / total;
        assert!((150.0..=210.0).contains(&ratio), "{ratio}");
        assert_eq!((PHASES[0].1, PHASES[2].1), (8.0, 14.0));
    }

    #[test]
    fn ecl_a9_photosphere_dots_stalks_reach_out_and_open_field_bends_to_the_equator() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        run_to(&mut g, &mut ctx, 6.0);
        let (c, r) = (g.centre, g.radius);
        let m = g.to_dot(g.moon());
        let mut n = 0;
        g.photosphere(|x, y, ink| {
            let (fx, fy) = (f64::from(x), f64::from(y));
            assert!((fx - c.0).hypot(fy - c.1) <= r && (fx - m.0).hypot(fy - m.1) > MOON_R * r);
            assert!(
                [
                    Ink::new(Role::Sun, 255),
                    Ink::new(Role::Sun, 170),
                    Ink::new(Role::Hot, 200)
                ]
                .contains(&ink)
            );
            n += 1;
        });
        assert!(n > 20, "{n} photosphere dots");
        // In totality the stalks run out past 3.5 R☉.
        run_to(&mut g, &mut ctx, 19.0);
        let model = &g.corona_model;
        let mut reach: f64 = 0.0;
        for s in &g.segs {
            let t = &model.threads[usize::from(s.thread)];
            if matches!(t.kind, Kind::Open { stalk: true, .. })
                && corona_rank(f64::from(s.base) * g.corona_vis()) > 0
            {
                reach = reach.max(f64::from(s.r));
            }
        }
        assert!(reach >= 3.5, "stalks reach {reach} R");
        // Open field: colatitude grows outward toward the equator, outside the helmet.
        for t in &model.threads {
            if let Kind::Open { south, .. } = t.kind {
                let lm = model.cusp(t);
                let mut last = 0.0;
                for k in 0..=100 {
                    let (rr, th) = model.point(t, k, 100);
                    let th = if south { PI - th } else { th };
                    assert!(th >= last - 1e-12, "colatitude fell");
                    if rr < lm {
                        let sep = (rr / lm).sqrt().asin();
                        assert!(th <= sep + 1e-9, "inside the helmet at r {rr}");
                    }
                    last = th;
                }
            }
        }
    }

    #[test]
    fn ecl_a10_black_overhead_land_and_a_clean_corona() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        let (c, r) = (g.centre, g.radius);
        let lights: Vec<(i32, i32)> = g
            .lights
            .iter()
            .map(|&(x, y, _)| (x / Dots::CELL_W, y / Dots::CELL_H))
            .collect();
        for t in [2.0, 6.0, 10.0, 19.0, 30.0, 36.0] {
            run_to(&mut g, &mut ctx, t);
            let canvas = canvas_of(&g, size);
            for y in 0..10 {
                for x in (0..12).chain(108..120) {
                    let cell = canvas.get(x, y).unwrap();
                    assert!(
                        cell.ch == ' ' || lights.contains(&(x, y)),
                        "t = {t}: ({x},{y}) not black"
                    );
                }
            }
            // The land: at most one lit cell per column below the skyline's top.
            let top = (*g.ridge.iter().min().unwrap() / Dots::CELL_H).max(0);
            for x in 0..120 {
                let lit = (top..40)
                    .filter(|&y| canvas.get(x, y).is_some_and(|c| c.ch != ' '))
                    .count();
                assert!(lit <= 2, "t = {t}: column {x} has {lit} land cells");
            }
            if t == 19.0 {
                let corona: Vec<Ink> = CORONA_STEPS.iter().map(|s| s.1).collect();
                for y in 0..40 {
                    for x in 0..120 {
                        let cell = canvas.get(x, y).unwrap();
                        let (fx, fy) = (f64::from(2 * x) + 1.0, f64::from(4 * y) + 2.0);
                        let d = (fx - c.0).hypot(fy - c.1) / r;
                        if cell.ch != ' ' && (1.3..3.0).contains(&d) {
                            assert!(
                                corona.contains(&cell.ink)
                                    || cell.ink == PROMINENCE
                                    || cell.ink.role == Role::Star,
                                "({x},{y}) {d:.2} R: {:?}",
                                cell.ink
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ecl_a11_the_diamond_ring_shows_a_bead_and_spikes_then_totality_does_not() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        let count = |g: &Eclipse, ink: Ink| {
            canvas_of(g, size)
                .cells()
                .iter()
                .filter(|c| c.ch != ' ' && c.ink == ink)
                .count()
        };
        run_to(&mut g, &mut ctx, 11.0);
        assert!(count(&g, BEAD_CORE) >= 1, "no diamond bead");
        let spikes = count(&g, SPIKE_AXIS) + count(&g, SPIKE_DIAGONAL);
        assert!(spikes >= 4, "{spikes} spike cells");
        let mut top = 0;
        g.corona(|_, _, rank, _| top += usize::from(rank == CORONA_STEPS.len()));
        assert_eq!(top, 0, "corona at full bloom in the diamond ring");
        run_to(&mut g, &mut ctx, 20.0);
        assert_eq!(count(&g, SPIKE_DIAGONAL), 0, "spikes in totality");
    }

    #[test]
    fn ecl_a12_threads_follow_the_field_thin_with_a_ringed_moon() {
        let model = Corona::seeded(&mut SplitMix64::new(5));
        for t in &model.threads {
            if let Kind::Closed { l } = t.kind {
                for k in 0..=50 {
                    let (r, th) = model.point(t, k, 50);
                    let s = th.sin_det();
                    assert!((r - l * s * s).abs() < 1e-9, "L {l} θ {th}");
                }
            }
        }
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size);
        run_to(&mut g, &mut ctx, 19.0);
        let ds = g.dots.borrow().dot_size();
        let mut only = Dots::new(size);
        let mut list = Vec::new();
        g.corona(|a, b, _, _| {
            only.line(a.0, a.1, b.0, b.1, Ink::new(Role::Star, 230));
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let n = dx.hypot(dy).max(1e-9);
            for (x, y) in sciart_core::geom::line(
                a.0.round() as i32,
                a.1.round() as i32,
                b.0.round() as i32,
                b.1.round() as i32,
            ) {
                if (0..i32::from(ds.width())).contains(&x)
                    && (0..i32::from(ds.height())).contains(&y)
                {
                    list.push((x, y, dx / n, dy / n));
                }
            }
        });
        let thin = list
            .iter()
            .filter(|&&(x, y, ux, uy)| {
                (-2..=2)
                    .filter(|&k| {
                        let k = f64::from(k);
                        only.is_lit(
                            (f64::from(x) - k * uy).round() as i32,
                            (f64::from(y) + k * ux).round() as i32,
                        )
                    })
                    .count()
                    <= 3
            })
            .count();
        let share = thin as f64 / list.len() as f64;
        assert!(
            list.len() > 300 && share >= 0.85,
            "thin {share:.3} of {}",
            list.len()
        );
        // The Moon's limb is a thin white ring in totality.
        let m = g.to_dot(g.moon());
        let r = MOON_R * g.radius + 0.6;
        let ring = (0..360)
            .filter(|&k| {
                let (s, c) = (f64::from(k).to_radians()).sin_cos_det();
                g.dots
                    .borrow()
                    .is_lit((m.0 + r * c).round() as i32, (m.1 + r * s).round() as i32)
            })
            .count();
        assert!(ring > 300, "ring {ring} of 360");
    }

    #[test]
    fn ecl_overlap_is_exact_at_the_limits() {
        assert_eq!(overlap(1.0, 1.03, 3.0), 0.0);
        assert!((overlap(1.0, 1.03, 0.0) - PI).abs() < 1e-12);
        // Half-way: equal discs one radius apart overlap by 2π/3 − √3/2.
        let lens = overlap(1.0, 1.0, 1.0);
        assert!((lens - (2.0 * PI / 3.0 - 3f64.sqrt() / 2.0)).abs() < 1e-12);
    }
}
