//! A diffusion cloud chamber in a magnetic field (SPEC-002 `cloud_chamber`).
//!
//! Invisible charged particles cross a supersaturated vapour and leave trails of droplets:
//! short fat alpha streaks, thin electrons curling into tightening spirals, long straight
//! cosmic muons (now and then knocking out a curly delta ray) and, rarely, the `V` of an
//! electron–positron pair converting from an unseen photon.
//!
//! ## Model
//! Events of each species arrive as Poisson processes at fixed mean rates (`ctx.rng`).
//! Every track is integrated once, when it is born, into a fixed slot of droplets:
//! - **alpha**: straight, its range drawn from the radon-chain alpha lines with 2 %
//!   straggling; droplets spaced by a Bragg-like `√(remaining range)` law, so the end is
//!   denser, in two lanes (2 dots thick);
//! - **beta**: the projection of a helix in a uniform out-of-screen `B`: the curvature is
//!   `q / r` with the gyroradius `r = p / (|q| B)`, positive charges turning clockwise on
//!   screen. The electron loses momentum as it ionises, `dr/ds = −κ (1 + (r_c/r)²)` (a
//!   constant loss plus a low-energy rise), so the circle tightens into a spiral, with a
//!   multiple-scattering kick `∝ 1/p` per step that makes the slow end curly;
//! - **muon**: enters at an edge and crosses the chamber nearly straight (its gyroradius
//!   dwarfs the chamber); with probability [`DELTA_P`] it knocks out a delta ray, a short,
//!   slow electron branch;
//! - **pair**: a vertex with two opposite-curling tracks of a shared momentum.
//!
//! Each droplet condenses as the particle passes (the trail draws in over ~0.1–0.4 s),
//! then all of a track's droplets sag and diffuse a little, and fade: from [`HOLD_S`] to
//! [`LIFE_S`] the track's level falls in flat steps and its density falls linearly, a
//! droplet staying only while its fixed threshold is below the density, so a fading
//! track only loses droplets (CLD-A5). A track's slot frees at [`LIFE_S`] (CLD-A3).
//!
//! ## Look (GEN-R25)
//! Everything is braille dots on black: alpha `Hot`, betas, positrons and delta rays
//! `Cold`, muons `Accent`. No vapour backdrop.
//!
//! Stylisation: rates and ranges are illustrative; tracks are 2-D projections; the field is
//! exaggerated so betas curl on screen.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::particles;
use sciart_core::prelude::*;
use sciart_core::rng::hash01;

pub static META: Meta = Meta {
    name: "cloud_chamber",
    category: Category::Physics,
    description: "Alpha, beta and muon tracks condense and fade in a cloud chamber inside a magnetic field",
    title: "Cloud chamber · trails of ions",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CloudChamber::default())
}

// ---- Species and rates (CLD-A4) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Alpha,
    Beta,
    Muon,
    Pair,
}

const KINDS: [Kind; 4] = [Kind::Alpha, Kind::Beta, Kind::Muon, Kind::Pair];
/// Mean events per simulated second at the reference chamber (200×50 cells), in [`KINDS`]
/// order: ≈ 9 live tracks there. Rates scale gently with the chamber (see
/// [`rate_scale`]), so a small chamber still looks alive and a big one is not a crowd.
pub(crate) const RATES: [f64; 4] = [1.2, 1.8, 0.95, 0.15];
/// Exponent of the rate's dependence on the chamber scale `S`: `S^0.4` gives ≈ 6 live
/// tracks at 80×24, ≈ 8 at 120×40, ≈ 9 at 200×50 and ≈ 14 at 500×150.
const RATE_EXPONENT: f64 = 0.4;
/// The fewest live tracks: below this an extra event arrives at once (a floor on Poisson
/// lulls; rare above 120×40).
const MIN_LIVE: usize = 4;
/// The reference chamber scale `S` in dots (200×50 cells = 400×200 dots).
const S_REF: f64 = 282.842_712_474_619;
/// Chance that a muon knocks out a visible delta ray.
const DELTA_P: f64 = 0.35;

// ---- Geometry (dot units; `S` = √(dot width · dot height), the chamber scale) ----

/// Dots per centimetre of chamber, as a fraction of `S` (a ~40 cm chamber).
const CM_PER_S: f64 = 1.0 / 40.0;
/// Alpha ranges in air (cm) of the radon chain: ²²²Rn, ²¹⁸Po, ²¹⁴Po, ²¹⁰Po.
pub(crate) const ALPHA_RANGES_CM: [f64; 4] = [4.05, 4.7, 6.9, 3.9];
/// Range straggling (fraction, 1σ), clipped at 3σ.
pub(crate) const STRAGGLE: f64 = 0.02;
/// Momentum (MeV/c) whose gyroradius is [`R_REF`]·S: fixes the (exaggerated) field.
const P_REF: f64 = 1.0;
const R_REF: f64 = 0.055;
/// Beta momenta, log-uniform (MeV/c): starting radii up to ≈ 0.08·S (≈ 8 cells across
/// at 120×40).
const BETA_P: (f64, f64) = (0.7, 1.5);
/// Delta-ray momenta (MeV/c): short curls a few dots across.
const DELTA_P_RANGE: (f64, f64) = (0.2, 0.4);
/// Pair total momentum (MeV/c).
const PAIR_P: (f64, f64) = (1.4, 3.2);
/// Radius loss per dot of path and the low-energy radius (dots) below which the loss
/// climbs: the turning angle to the end is `ln((r0² + R_LOW²)/(R_END² + R_LOW²)) / 2κ`,
/// about 1.5 turns for the fastest betas, so a track curves gently and then tightens
/// into a curl.
const KAPPA: f64 = 0.11;
const R_LOW: f64 = 6.0;
/// A beta stops below this radius (dots).
const R_END: f64 = 1.2;
/// Multiple-scattering strength: kick σ = `SCATTER · √ds / r` radians.
const SCATTER: f64 = 0.3;
/// Droplet spacing along an electron's path (dots).
pub(crate) const BETA_SPACING: f64 = 1.0;
/// Integration step along a path (dots).
const STEP: f64 = 0.5;

// ---- Droplets and time ----

/// Seconds for the particle's condensation front to cross one `S` of path; capped.
const DRAW_S_PER_S: f64 = 0.12;
const DRAW_MAX_S: f64 = 0.4;
/// Full strength until `HOLD_S`, then fading in flat steps until `LIFE_S`.
pub(crate) const HOLD_S: f64 = 0.7;
pub(crate) const LIFE_S: f64 = 2.3;
/// Fade levels after `HOLD_S` (flat steps, M11 idiom).
const FADE_LEVELS: [u8; 4] = [225, 190, 155, 120];
/// Sag (dots per second, starting after condensation; a slight nod to a side view, see
/// the stylisation note) and diffusion (dots per √s).
const SAG: f64 = 0.8;
const DIFFUSE: f64 = 0.7;
/// Track slots; a full pool reuses the oldest (≈ 22 live at 500×150).
const SLOTS: usize = 28;
/// Vapour specks: a faint drizzle of `Dim` droplets, this share of the dots (≤ 1 %),
/// falling at [`DRIZZLE`] dots per second.
const SPECKS: f64 = 0.001;
const DRIZZLE: f64 = 1.5;
const SPECK: Ink = Ink::new(Role::Dim, 110);

/// Inks by droplet role byte.
const ROLES: [Role; 3] = [Role::Hot, Role::Cold, Role::Accent];
const HOT: u8 = 0;
const COLD: u8 = 1;
const ACCENT: u8 = 2;
/// Every track's level until [`HOLD_S`].
const FULL: u8 = 255;

/// One condensation droplet, relative to its track's birth.
#[derive(Debug, Clone, Copy, Default)]
struct Drop {
    x: f32,
    y: f32,
    /// Seconds after the track's birth when it condenses.
    appear: f32,
    /// Fixed fade threshold in `[0, 1)` (CLD-A5).
    thr: f32,
    /// Fixed diffusion direction, `[-1, 1]` scaled by 127.
    jx: i8,
    jy: i8,
    role: u8,
}

#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    live: bool,
    birth: f64,
    len: usize,
}

/// The track's droplet density and ink level at `age` seconds, `None` once gone
/// (CLD-A3, CLD-A5): full until [`HOLD_S`], then the density falls linearly and the level
/// in flat steps.
pub(crate) fn fade(age: f64) -> Option<(f64, u8)> {
    if !(0.0..LIFE_S).contains(&age) {
        return None;
    }
    if age < HOLD_S {
        return Some((1.0, FULL));
    }
    let u = (age - HOLD_S) / (LIFE_S - HOLD_S);
    let step = ((u * FADE_LEVELS.len() as f64) as usize).min(FADE_LEVELS.len() - 1);
    Some((1.0 - u, FADE_LEVELS[step]))
}

/// Gyroradius `r = p / (|q| B)` (CLD-A1).
pub(crate) fn gyro_radius(p: f64, charge: f64, b: f64) -> f64 {
    p / (charge.abs() * b)
}

/// A charged particle's launch and medium for [`curl_path`] (dot units, screen
/// coordinates with y down).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Path {
    pub start: (f64, f64),
    pub theta: f64,
    /// Initial gyroradius.
    pub r0: f64,
    /// Charge sign.
    pub charge: f64,
    /// Radius loss per dot of path.
    pub kappa: f64,
    /// Multiple-scattering strength (0 = none).
    pub scatter: f64,
    pub max_len: f64,
}

/// Integrate a charged particle's path in the out-of-screen field: curvature
/// `dθ/ds = charge / r` (positive charges clockwise on screen), the radius shrinking by
/// `dr/ds = −kappa (1 + (R_LOW/r)²)`, plus a scattering kick of σ = `scatter · √ds / r`
/// when `scatter > 0`. Calls `f(x, y, s, r)` every [`STEP`] and stops when `f` returns
/// false, the radius falls below [`R_END`] or `s > max_len`.
pub(crate) fn curl_path(
    path: Path,
    rng: &mut impl Rng,
    mut f: impl FnMut(f64, f64, f64, f64) -> bool,
) {
    let Path {
        start: (mut x, mut y),
        mut theta,
        r0,
        charge,
        kappa,
        scatter,
        max_len,
    } = path;
    let mut r = r0.max(R_END);
    let mut s = 0.0;
    let sq = STEP.sqrt();
    while s <= max_len && r >= R_END {
        if !f(x, y, s, r) {
            return;
        }
        // Midpoint rule for the heading: exact for a circle of fixed radius.
        let turn = charge.signum() * STEP / r;
        let (sn, cs) = (theta + 0.5 * turn).sin_cos_det();
        x += cs * STEP;
        y += sn * STEP;
        theta += turn;
        if scatter > 0.0 {
            // Sum of two uniforms: a cheap, bounded bell for the kick.
            let u: f64 = rng.random::<f64>() + rng.random::<f64>() - 1.0;
            theta += u * 2.45 * scatter * sq / r;
        }
        r -= kappa * STEP * (1.0 + (R_LOW / r).powi(2));
        s += STEP;
    }
}

/// Electron rest mass (MeV/c²).
const M_E: f64 = 0.511;

/// Emission angle (radians from the muon) of a delta ray of momentum `p` (MeV/c) knocked
/// out by a fast heavy particle: `cos θ = T / p` (β → 1), `T` its kinetic energy.
pub(crate) fn delta_angle(p: f64) -> f64 {
    let t = (p * p + M_E * M_E).sqrt() - M_E;
    (t / p).clamp(-1.0, 1.0).acos()
}

/// Event-rate factor for a chamber of scale `scale` dots relative to the reference:
/// `(S / S_ref)^0.4`. Track lengths scale with `S`, so a smaller chamber's tracks cover a
/// little more of it and a bigger one's a little less, which reads better than equal
/// coverage: equal coverage leaves a small chamber with two or three tracks.
pub(crate) fn rate_scale(scale: f64) -> f64 {
    (scale / S_REF).powf(RATE_EXPONENT)
}

/// Alpha-track range in dots for line `line` and straggling deviate `z` (clipped to ±3).
pub(crate) fn alpha_range(line: usize, z: f64, scale: f64) -> f64 {
    let cm = ALPHA_RANGES_CM[line % ALPHA_RANGES_CM.len()];
    cm * (1.0 + STRAGGLE * z.clamp(-3.0, 3.0)) * scale * CM_PER_S
}

/// Writes one track's droplets into its slot.
struct Sink<'a> {
    drops: &'a mut [Drop],
    n: usize,
    salt: u64,
    /// Seconds per dot of path for the condensation front.
    draw_rate: f64,
}

impl Sink<'_> {
    fn push(&mut self, x: f64, y: f64, s: f64, role: u8) {
        let Some(d) = self.drops.get_mut(self.n) else {
            return;
        };
        let key = self.salt ^ (self.n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let j = |k: u64| ((hash01(key ^ k) * 2.0 - 1.0) * 127.0) as i8;
        *d = Drop {
            x: x as f32,
            y: y as f32,
            appear: (s * self.draw_rate).min(DRAW_MAX_S) as f32,
            thr: hash01(key) as f32,
            jx: j(0xA1),
            jy: j(0xB2),
            role,
        };
        self.n += 1;
    }

    fn full(&self) -> bool {
        self.n >= self.drops.len()
    }

    /// Fixed coin for gaps in thin tracks, keyed by the caller (e.g. the path step).
    fn coin(&self, salt: u64) -> f64 {
        hash01(self.salt ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93))
    }
}

/// The chamber in dot units.
#[derive(Debug, Clone, Copy)]
struct Chamber {
    w: f64,
    h: f64,
    scale: f64,
}

impl Chamber {
    fn inside(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= -margin && y >= -margin && x <= self.w + margin && y <= self.h + margin
    }

    /// Field strength in MeV/c per dot of gyroradius.
    fn field(&self) -> f64 {
        P_REF / (R_REF * self.scale).max(1e-9)
    }
}

/// A thin electron's launch: position, heading, momentum (MeV/c), charge sign, and the
/// path length `s0` its condensation front starts from.
#[derive(Debug, Clone, Copy)]
struct Launch {
    start: (f64, f64),
    theta: f64,
    p: f64,
    charge: f64,
    s0: f64,
}

/// Integrate a thin electron (beta, delta ray, pair arm) into `sink`.
fn electron(sink: &mut Sink, ch: Chamber, at: Launch, rng: &mut impl Rng) {
    let path = Path {
        start: at.start,
        theta: at.theta,
        r0: gyro_radius(at.p, at.charge, ch.field()),
        charge: at.charge,
        kappa: KAPPA,
        scatter: SCATTER,
        max_len: 8.0 * ch.scale + 64.0,
    };
    let s0 = at.s0;
    // A droplet every dot of path: the track reads as one continuous thread.
    let mut next = 0.0;
    curl_path(path, rng, |x, y, s, _| {
        if sink.full() || !ch.inside(x, y, 4.0) {
            return false;
        }
        if s >= next {
            next = s + BETA_SPACING - 0.5 * STEP;
            sink.push(x, y, s0 + s, COLD);
        }
        true
    });
}

/// Build one track of `kind` into `sink` (droplets relative to the chamber origin).
fn spawn_track(kind: Kind, sink: &mut Sink, ch: Chamber, rng: &mut impl Rng) {
    let (w, h) = (ch.w, ch.h);
    match kind {
        Kind::Alpha => {
            let line = rng.random_range(0..ALPHA_RANGES_CM.len());
            let z = particles::gaussian(rng, 0.0, 1.0);
            let range = alpha_range(line, z, ch.scale);
            let (x0, y0) = (rng.random_range(0.0..=w), rng.random_range(0.0..=h));
            let (sn, cs) = rng.random_range(0.0..TAU).sin_cos_det();
            // Bragg-like spacing: denser toward the end, ∝ √(remaining + R/4).
            let mut s = 0.0;
            let mut lane = 0.5;
            let tail = 0.25 * range;
            while s <= range && !sink.full() {
                let jit = (sink.coin(0xA7 ^ (s * 64.0) as u64) - 0.5) * 0.5;
                let d = lane + jit;
                sink.push(x0 + cs * s - sn * d, y0 + sn * s + cs * d, s, HOT);
                lane = -lane;
                s += 0.45 * ((range - s + tail) / (range + tail)).sqrt();
            }
        }
        Kind::Beta => {
            let start = (rng.random_range(0.0..=w), rng.random_range(0.0..=h));
            let p = log_uniform(rng, BETA_P);
            let charge = if rng.random_bool(0.85) { -1.0 } else { 1.0 };
            let theta = rng.random_range(0.0..TAU);
            let at = Launch {
                start,
                theta,
                p,
                charge,
                s0: 0.0,
            };
            electron(sink, ch, at, rng);
        }
        Kind::Muon => {
            // A line through a point of the central part of the chamber at a random
            // angle, started one diagonal back so it enters at an edge and crosses.
            let diag = (w * w + h * h).sqrt();
            let mid = (
                rng.random_range(0.2..0.8) * w,
                rng.random_range(0.2..0.8) * h,
            );
            let theta = rng.random_range(0.0..PI) + if rng.random_bool(0.5) { PI } else { 0.0 };
            let (sn, cs) = theta.sin_cos_det();
            let (x0, y0) = (mid.0 - cs * diag, mid.1 - sn * diag);
            let mut entered = None::<f64>;
            let charge = if rng.random_bool(0.55) { 1.0 } else { -1.0 };
            // A muon of ~1 GeV/c: a gyroradius of ~100 S, nearly straight.
            let r0 = 100.0 * ch.scale.max(1.0);
            let delta_at = rng
                .random_bool(DELTA_P)
                .then(|| diag * rng.random_range(0.6..1.4));
            let mut delta = None;
            let mut next = 0.0;
            let path = Path {
                start: (x0, y0),
                theta,
                r0,
                charge,
                kappa: 0.0,
                scatter: 0.0,
                max_len: 2.0 * diag + 8.0,
            };
            curl_path(path, rng, |x, y, s, _| {
                let inside = ch.inside(x, y, 1.0);
                if sink.full() || (entered.is_some() && !inside) {
                    return false;
                }
                if inside && entered.is_none() {
                    entered = Some(s);
                }
                if !inside {
                    return true;
                }
                if s >= next {
                    next = s + 1.0;
                    if sink.coin(0x1111 ^ (s * 8.0) as u64) < 0.85 {
                        sink.push(x, y, s - entered.unwrap_or(0.0), ACCENT);
                    }
                }
                if delta.is_none() && delta_at.is_some_and(|d| s >= d) {
                    delta = Some((x, y, s - entered.unwrap_or(0.0)));
                }
                true
            });
            if let Some((x, y, s)) = delta {
                let side = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                let p = rng.random_range(DELTA_P_RANGE.0..DELTA_P_RANGE.1);
                let dir = theta + side * delta_angle(p);
                let at = Launch {
                    start: (x, y),
                    theta: dir,
                    p,
                    charge: -1.0,
                    s0: s,
                };
                electron(sink, ch, at, rng);
            }
        }
        Kind::Pair => {
            let start = (
                rng.random_range(0.15 * w..=0.85 * w.max(0.0)),
                rng.random_range(0.15 * h..=0.85 * h.max(0.0)),
            );
            let theta = rng.random_range(0.0..TAU);
            let p = rng.random_range(PAIR_P.0..PAIR_P.1);
            let f = rng.random_range(0.3..0.7);
            let open = rng.random_range(0.03..0.08);
            for (side, share, charge) in [(-1.0, f, -1.0), (1.0, 1.0 - f, 1.0)] {
                let at = Launch {
                    start,
                    theta: theta + side * open,
                    p: share * p,
                    charge,
                    s0: 0.0,
                };
                electron(sink, ch, at, rng);
            }
        }
    }
}

fn log_uniform(rng: &mut impl Rng, (lo, hi): (f64, f64)) -> f64 {
    lo * (hi / lo).powf(rng.random_range(0.0..1.0))
}

/// Number of events of a Poisson process with mean `lambda` (inversion; `lambda` small).
fn poisson(rng: &mut impl Rng, lambda: f64) -> u32 {
    if lambda.is_nan() || lambda <= 0.0 {
        return 0;
    }
    let mut p = (-lambda).exp();
    let mut cdf = p;
    let u: f64 = rng.random_range(0.0..1.0);
    let mut k = 0;
    while u > cdf && k < 32 {
        k += 1;
        p *= lambda / f64::from(k);
        cdf += p;
    }
    k
}

#[derive(Debug)]
pub struct CloudChamber {
    size: Size,
    time: f64,
    chamber: Chamber,
    /// Droplets per slot.
    cap: usize,
    slots: [Slot; SLOTS],
    drops: Vec<Drop>,
    /// Events so far per species, in [`KINDS`] order (CLD-A4).
    events: [u64; 4],
    /// Extra events the live-track floor added ([`MIN_LIVE`]).
    floor_events: u64,
    /// Event rates for this chamber (per second, [`KINDS`] order).
    rates: [f64; 4],
    /// Vapour specks in this chamber.
    specks: usize,
    dots: Dots,
    dirty: bool,
}

impl Default for CloudChamber {
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            time: 0.0,
            chamber: Chamber {
                w: 1.0,
                h: 1.0,
                scale: 1.0,
            },
            cap: 0,
            slots: [Slot::default(); SLOTS],
            drops: Vec::new(),
            events: [0; 4],
            floor_events: 0,
            rates: RATES,
            specks: 0,
            dots: Dots::default(),
            dirty: true,
        }
    }
}

impl CloudChamber {
    /// Tracks still on show.
    fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.live).count()
    }

    /// A species drawn in proportion to its rate.
    fn pick_kind(&self, rng: &mut impl Rng) -> Kind {
        let total: f64 = self.rates.iter().sum();
        let mut u = rng.random_range(0.0..total);
        for (k, &r) in self.rates.iter().enumerate() {
            if u < r {
                return KINDS[k];
            }
            u -= r;
        }
        Kind::Beta
    }

    /// A free slot, or the oldest live one.
    fn free_slot(&self) -> usize {
        let mut best = 0;
        let mut oldest = f64::INFINITY;
        for (i, s) in self.slots.iter().enumerate() {
            if !s.live {
                return i;
            }
            if s.birth < oldest {
                oldest = s.birth;
                best = i;
            }
        }
        best
    }

    /// Start a track of `kind` born at `birth`; returns its slot.
    fn spawn(&mut self, kind: Kind, birth: f64, rng: &mut impl Rng) -> usize {
        let i = self.free_slot();
        let ch = self.chamber;
        let draw_rate = DRAW_S_PER_S / ch.scale.max(1.0);
        let salt = rng.random::<u64>();
        let mut sink = Sink {
            drops: &mut self.drops[i * self.cap..(i + 1) * self.cap],
            n: 0,
            salt,
            draw_rate,
        };
        spawn_track(kind, &mut sink, ch, rng);
        self.slots[i] = Slot {
            live: true,
            birth,
            len: sink.n,
        };
        i
    }

    /// Indices (within the slot) of the droplets of slot `i` visible at the current time.
    #[cfg(test)]
    fn visible(&self, i: usize, f: impl FnMut(usize, &Drop, f64, f64, Ink)) {
        visit(&self.slots, &self.drops, self.cap, self.time, i, f);
    }

    fn rasterise(&mut self) {
        let dots = &mut self.dots;
        dots.clear();
        // The drizzle: fixed-hash specks falling slowly, wrapping at the bottom.
        let (w, h) = (self.chamber.w, self.chamber.h);
        for i in 0..self.specks as u64 {
            let key = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD122_51E0;
            let x = (hash01(key) * w).floor();
            let y = (hash01(key ^ 0x51) * h + DRIZZLE * self.time).rem_euclid(h);
            dots.plot(x, y.floor(), SPECK);
        }
        for i in 0..SLOTS {
            visit(
                &self.slots,
                &self.drops,
                self.cap,
                self.time,
                i,
                |_, _, x, y, ink| {
                    dots.plot(x, y, ink);
                },
            );
        }
    }
}

/// Call `f(index, droplet, x, y, ink)` for each droplet of slot `i` visible at `time`
/// (CLD-A3, CLD-A5): condensed, not yet thinned out, sagged and diffused by its age.
fn visit(
    slots: &[Slot],
    drops: &[Drop],
    cap: usize,
    time: f64,
    i: usize,
    mut f: impl FnMut(usize, &Drop, f64, f64, Ink),
) {
    let slot = &slots[i];
    if !slot.live {
        return;
    }
    let age = time - slot.birth;
    let Some((density, level)) = fade(age) else {
        return;
    };
    let start = i * cap;
    for (k, d) in drops[start..start + slot.len].iter().enumerate() {
        let appear = f64::from(d.appear);
        if appear > age || f64::from(d.thr) >= density {
            continue;
        }
        let since = age - appear;
        let diff = DIFFUSE * since.sqrt() / 127.0;
        let x = f64::from(d.x) + f64::from(d.jx) * diff;
        let y = f64::from(d.y) + f64::from(d.jy) * diff + SAG * since;
        let r = usize::from(d.role.min(2));
        f(k, d, x, y, Ink::new(ROLES[r], level));
    }
}

impl Generator for CloudChamber {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let ds = self.dots.dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        let scale = (w * h).sqrt().max(1.0);
        self.chamber = Chamber { w, h, scale };
        // Room for the longest spiral or a muon with its delta ray.
        self.cap = (5.0 * scale) as usize + 256;
        let k = rate_scale(scale);
        self.rates = RATES.map(|r| r * k);
        self.specks = (w * h * SPECKS) as usize;
        self.drops.clear();
        self.drops.resize(SLOTS * self.cap, Drop::default());
        // Live tracks are cleared; the pool is warmed with the steady state's mix of
        // species and ages, so the first frame already looks lived-in.
        self.slots = [Slot::default(); SLOTS];
        let total: f64 = self.rates.iter().sum();
        let warm = ((total * LIFE_S).round() as usize).min(SLOTS);
        for _ in 0..warm {
            let kind = self.pick_kind(&mut ctx.rng);
            let age = ctx.rng.random_range(0.0..LIFE_S);
            self.spawn(kind, self.time - age, &mut ctx.rng);
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.time += dt;
        for s in &mut self.slots {
            if s.live && self.time - s.birth >= LIFE_S {
                s.live = false;
            }
        }
        for (k, &kind) in KINDS.iter().enumerate() {
            let n = poisson(&mut ctx.rng, self.rates[k] * dt);
            for _ in 0..n {
                self.events[k] += 1;
                // Born at a uniform instant within the step.
                let birth = self.time - ctx.rng.random_range(0.0..dt);
                self.spawn(kind, birth, &mut ctx.rng);
            }
        }
        // The floor: a Poisson lull that would leave the chamber nearly empty is cut
        // short by an extra event at once (disclosed in SPEC-002's As built; counted
        // apart from the Poisson events, CLD-A4).
        while self.live() < MIN_LIVE {
            let kind = self.pick_kind(&mut ctx.rng);
            self.floor_events += 1;
            self.spawn(kind, self.time, &mut ctx.rng);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(r0: f64, charge: f64) -> Vec<(f64, f64)> {
        let mut rng = Ctx::new(1, Size::new(1, 1)).rng;
        let mut pts = Vec::new();
        let path = Path {
            start: (0.0, 0.0),
            theta: 0.0,
            r0,
            charge,
            kappa: 0.0,
            scatter: 0.0,
            max_len: r0,
        };
        curl_path(path, &mut rng, |x, y, _, _| {
            pts.push((x, y));
            true
        });
        pts
    }

    /// Signed turn (cross product sign) and circumradius of three path points.
    fn circle(p: &[(f64, f64)]) -> (f64, f64) {
        let (a, b, c) = (p[0], p[p.len() / 2], p[p.len() - 1]);
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        let d = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1);
        let r = d(a, b) * d(b, c) * d(c, a) / (2.0 * cross.abs());
        (cross, r)
    }

    #[test]
    fn cld_a1_beta_curl_flips_with_charge_and_radius_tracks_momentum() {
        let field = Chamber {
            w: 400.0,
            h: 200.0,
            scale: 283.0,
        }
        .field();
        for p in [0.2, 0.5, 1.0] {
            let r = gyro_radius(p, -1.0, field);
            assert!((r - p * R_REF * 283.0 / P_REF).abs() < 1e-9, "r ∝ p");
            let (neg, rn) = circle(&path(r, -1.0));
            let (pos, rp) = circle(&path(r, 1.0));
            assert!(neg * pos < 0.0, "curl flips with charge");
            // Positive charges turn clockwise on screen (y down): cross > 0.
            assert!(pos > 0.0);
            for measured in [rn, rp] {
                assert!((measured / r - 1.0).abs() < 0.01, "r {measured} vs {r}");
            }
        }
        // Doubling the momentum doubles the radius.
        assert!((gyro_radius(0.8, 1.0, field) / gyro_radius(0.4, -1.0, field) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn cld_a1_energy_loss_tightens_the_spiral() {
        let mut rng = Ctx::new(3, Size::new(1, 1)).rng;
        let mut radii = Vec::new();
        let path = Path {
            start: (0.0, 0.0),
            theta: 0.0,
            r0: 30.0,
            charge: -1.0,
            kappa: KAPPA,
            scatter: 0.0,
            max_len: 1e4,
        };
        curl_path(path, &mut rng, |_, _, _, r| {
            radii.push(r);
            true
        });
        assert!(radii.windows(2).all(|w| w[1] < w[0]));
        assert!(*radii.last().unwrap() < 2.0, "ends as a tight curl");
    }

    fn sink_track(kind: Kind, seed: u64) -> (Vec<Drop>, Chamber) {
        let ch = Chamber {
            w: 400.0,
            h: 200.0,
            scale: 283.0,
        };
        let mut drops = vec![Drop::default(); 4000];
        let mut rng = Ctx::new(seed, Size::new(1, 1)).rng;
        let mut sink = Sink {
            drops: &mut drops,
            n: 0,
            salt: seed,
            draw_rate: 0.0,
        };
        spawn_track(kind, &mut sink, ch, &mut rng);
        let n = sink.n;
        drops.truncate(n);
        (drops, ch)
    }

    #[test]
    fn cld_a2_alpha_tracks_are_straight_and_ranges_in_distribution() {
        let lo = ALPHA_RANGES_CM.iter().copied().fold(f64::MAX, f64::min);
        let hi = ALPHA_RANGES_CM.iter().copied().fold(0.0, f64::max);
        let k = 283.0 * CM_PER_S;
        for seed in 0..40 {
            let (d, _) = sink_track(Kind::Alpha, seed);
            assert!(d.len() > 20);
            let (a, b) = (d[0], d[d.len() - 1]);
            // Axis through the lane midpoints of the first and last pairs.
            let (x0, y0) = (f64::from(a.x + d[1].x) / 2.0, f64::from(a.y + d[1].y) / 2.0);
            let m = d.len() - 2;
            let (x1, y1) = (f64::from(d[m].x + b.x) / 2.0, f64::from(d[m].y + b.y) / 2.0);
            let len = (x1 - x0).hypot(y1 - y0);
            let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
            let mut along_max = 0.0f64;
            for p in &d {
                let (dx, dy) = (f64::from(p.x) - x0, f64::from(p.y) - y0);
                let off = (dx * uy - dy * ux).abs();
                assert!(off <= 1.0, "seed {seed}: deviation {off}");
                along_max = along_max.max(dx * ux + dy * uy);
            }
            let range_lo = lo * (1.0 - 3.0 * STRAGGLE) * k - 1.0;
            let range_hi = hi * (1.0 + 3.0 * STRAGGLE) * k + 1.0;
            assert!(
                (range_lo..=range_hi).contains(&along_max),
                "seed {seed}: length {along_max}"
            );
            assert!(d.iter().all(|p| p.role == HOT));
        }
        // Denser at the end (Bragg-like): the last fifth holds more droplets than the first.
        let (d, _) = sink_track(Kind::Alpha, 7);
        let n = d.len();
        let span = |a: usize, b: usize| f64::from((d[b].x - d[a].x).hypot(d[b].y - d[a].y));
        assert!(span(n - n / 5 - 1, n - 1) < span(0, n / 5), "end denser");
    }

    fn chamber_at(seed: u64) -> (CloudChamber, Ctx) {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(seed, size);
        let mut g = CloudChamber::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn cld_a3_track_gone_within_lifetime_and_slot_reused() {
        let (mut g, mut ctx) = chamber_at(5);
        g.slots = [Slot::default(); SLOTS];
        let slot = g.spawn(Kind::Beta, g.time, &mut ctx.rng);
        // Hold the Poisson stream off by stepping a private clock.
        let steps = (LIFE_S / SIM_DT).ceil() as usize + 1;
        for _ in 0..steps {
            g.time += SIM_DT;
            for s in &mut g.slots {
                if s.live && g.time - s.birth >= LIFE_S {
                    s.live = false;
                }
            }
        }
        let mut n = 0;
        g.visible(slot, |_, _, _, _, _| n += 1);
        assert_eq!(n, 0);
        assert!(!g.slots[slot].live);
        assert_eq!(g.free_slot(), 0);
        let again = g.spawn(Kind::Alpha, g.time, &mut ctx.rng);
        assert_eq!(again, slot, "first free slot reused");
        assert!(fade(LIFE_S).is_none() && fade(LIFE_S - 1e-9).is_some());
    }

    #[test]
    fn cld_a4_event_counts_match_rates_within_3_sigma() {
        for seed in [1, 2, 3] {
            let (mut g, mut ctx) = chamber_at(seed);
            let secs = 60.0;
            for _ in 0..(secs / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
            }
            for (k, &rate) in g.rates.iter().enumerate() {
                let mean = rate * secs;
                let got = g.events[k] as f64;
                assert!(
                    (got - mean).abs() <= 3.0 * mean.sqrt(),
                    "seed {seed} kind {k}: {got} vs {mean}"
                );
            }
        }
    }

    #[test]
    fn cld_a5_fading_droplets_are_a_subset_of_earlier_ones() {
        let (mut g, mut ctx) = chamber_at(9);
        g.slots = [Slot::default(); SLOTS];
        let slot = g.spawn(Kind::Muon, g.time, &mut ctx.rng);
        let ids = |g: &CloudChamber| {
            let mut v = Vec::new();
            g.visible(slot, |k, _, _, _, _| v.push(k));
            v
        };
        g.time += DRAW_MAX_S + 0.01;
        let mut prev = ids(&g);
        assert!(!prev.is_empty());
        while g.time - g.slots[slot].birth < LIFE_S {
            g.time += SIM_DT;
            let now = ids(&g);
            assert!(now.iter().all(|k| prev.contains(k)), "only loses droplets");
            prev = now;
        }
        // And the level only falls, in flat steps.
        let mut last = FULL;
        let mut levels = Vec::new();
        let mut t = 0.0;
        while let Some((_, level)) = fade(t) {
            assert!(level <= last);
            if level != last || levels.is_empty() {
                levels.push(level);
            }
            last = level;
            t += 0.01;
        }
        assert_eq!(levels.len(), 1 + FADE_LEVELS.len(), "{levels:?}");
    }

    #[test]
    fn spawned_tracks_are_drawn_in_their_roles() {
        for (kind, role) in [(Kind::Beta, COLD), (Kind::Muon, ACCENT), (Kind::Pair, COLD)] {
            let (d, _) = sink_track(kind, 11);
            assert!(!d.is_empty(), "{kind:?}");
            assert!(d.iter().any(|p| p.role == role), "{kind:?}");
        }
    }

    #[test]
    fn tiny_and_huge_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (3, 2), (500, 150)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(1, size);
            let mut g = CloudChamber::default();
            g.resize(size, &mut ctx);
            for _ in 0..120 {
                g.update(0.25, &mut ctx);
                g.prepare();
            }
        }
    }

    #[test]
    fn muons_cross_the_chamber_as_dotted_threads() {
        let mut total = 0.0;
        for seed in 0..20 {
            let (d, ch) = sink_track(Kind::Muon, seed);
            let mu: Vec<&Drop> = d.iter().filter(|p| p.role == ACCENT).collect();
            let gap = |a: &Drop, b: &Drop| f64::from((b.x - a.x).hypot(b.y - a.y));
            assert!(
                mu.windows(2).all(|w| gap(w[0], w[1]) < 12.0),
                "seed {seed}: no long gaps"
            );
            total += gap(mu[0], mu[mu.len() - 1]) / ch.h;
        }
        assert!(total / 20.0 > 0.8, "mean crossing {}", total / 20.0);
    }

    fn live(g: &CloudChamber) -> usize {
        g.slots
            .iter()
            .filter(|s| s.live && g.time - s.birth < LIFE_S)
            .count()
    }

    #[test]
    fn busy_chamber_from_the_first_frame_at_200x50() {
        let (mut g, mut ctx) = chamber_at(2);
        assert!(live(&g) >= 5, "warm start: {}", live(&g));
        let (mut sum, mut n) = (0usize, 0usize);
        for step in 0..(30.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            if step % 6 == 0 {
                sum += live(&g);
                n += 1;
            }
        }
        let mean = sum as f64 / n as f64;
        assert!((6.0..=10.0).contains(&mean), "mean live tracks {mean}");
    }

    /// Mean live tracks over 30 s, and the lowest 1-s mean, for one chamber.
    fn liveliness(w: u16, h: u16, seed: u64) -> (f64, f64) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = CloudChamber::default();
        g.resize(size, &mut ctx);
        let per_s = (1.0 / SIM_DT).round() as usize;
        let (mut total, mut n, mut low) = (0usize, 0usize, f64::INFINITY);
        for _ in 0..30 {
            let mut window = 0usize;
            for _ in 0..per_s {
                g.update(SIM_DT, &mut ctx);
                window += live(&g);
            }
            total += window;
            n += per_s;
            low = low.min(window as f64 / per_s as f64);
        }
        (total as f64 / n as f64, low)
    }

    #[test]
    fn chamber_is_alive_at_every_size() {
        // Mean live tracks over 30 s: lively on small terminals, not a crowd on big ones.
        let bands = [
            ((80, 24), 5.0, 8.5),
            ((120, 40), 5.5, 9.0),
            ((200, 50), 8.0, 10.5),
            ((500, 150), 10.0, 16.0),
        ];
        for ((w, h), lo, hi) in bands {
            for seed in [1, 2, 3] {
                let (mean, _) = liveliness(w, h, seed);
                assert!((lo..=hi).contains(&mean), "{w}x{h} seed {seed}: {mean}");
            }
        }
        // No 1-s stretch of a near-empty chamber at 120×40.
        for seed in [1, 2, 3] {
            let (_, low) = liveliness(120, 40, seed);
            assert!(
                low >= 3.0,
                "seed {seed}: a 1-s window averaged {low} live tracks"
            );
        }
    }

    #[test]
    fn the_live_track_floor_only_trims_rare_lulls() {
        for (w, h, most) in [(80, 24, 0.15), (120, 40, 0.06), (200, 50, 0.02)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(4, size);
            let mut g = CloudChamber::default();
            g.resize(size, &mut ctx);
            for _ in 0..(60.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                assert!(g.live() >= MIN_LIVE);
            }
            let poisson: u64 = g.events.iter().sum();
            let share = g.floor_events as f64 / (poisson + g.floor_events) as f64;
            assert!(share <= most, "{w}x{h}: floor share {share}");
        }
    }

    #[test]
    fn rates_follow_a_gentle_power_of_the_chamber_scale() {
        let rates = |w: u16, h: u16| {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(1, size);
            let mut g = CloudChamber::default();
            g.resize(size, &mut ctx);
            (g.rates, g.chamber.scale)
        };
        let (r1, s1) = rates(200, 50);
        let (r2, s2) = rates(500, 150);
        for k in 0..4 {
            assert!((r1[k] - RATES[k]).abs() < 1e-9);
            assert!((r2[k] / r1[k] - (s2 / s1).powf(RATE_EXPONENT)).abs() < 1e-9);
        }
        assert!(rates(80, 24).0[0] < r1[0]);
    }

    #[test]
    fn delta_rays_leave_at_the_kinematic_angle() {
        let (lo, hi) = DELTA_P_RANGE;
        for k in 0..=10 {
            let p = lo + (hi - lo) * f64::from(k) / 10.0;
            let t = (p * p + M_E * M_E).sqrt() - M_E;
            let deg = delta_angle(p).to_degrees();
            assert!((deg - (t / p).acos().to_degrees()).abs() < 1e-9);
            assert!((65.0..=81.0).contains(&deg), "p {p}: {deg}°");
        }
        // Slower deltas leave closer to perpendicular.
        assert!(delta_angle(lo) > delta_angle(hi));
    }

    #[test]
    fn betas_are_continuous_threads_that_curl_within_two_turns() {
        // Droplets at most a dot apart along the path.
        for seed in 0..30 {
            let (d, ch) = sink_track(Kind::Beta, seed);
            let gap = |a: &Drop, b: &Drop| f64::from((b.x - a.x).hypot(b.y - a.y));
            assert!(
                d.windows(2).all(|w| gap(&w[0], &w[1]) <= 1.0 + 1e-3),
                "seed {seed}"
            );
            // Starting radius at most ≈ 0.08·S (8 cells across at 120×40).
            let r0 = gyro_radius(BETA_P.1, 1.0, ch.field());
            assert!(r0 <= 0.085 * ch.scale, "{r0}");
        }
        // The fastest beta turns through 1–2 turns before it stops.
        let field = Chamber {
            w: 240.0,
            h: 160.0,
            scale: (240.0f64 * 160.0).sqrt(),
        }
        .field();
        let r0 = gyro_radius(BETA_P.1, -1.0, field);
        let mut turned = 0.0;
        let mut rng = Ctx::new(1, Size::new(1, 1)).rng;
        let path = Path {
            start: (0.0, 0.0),
            theta: 0.0,
            r0,
            charge: -1.0,
            kappa: KAPPA,
            scatter: 0.0,
            max_len: 1e4,
        };
        curl_path(path, &mut rng, |_, _, _, r| {
            turned += STEP / r;
            true
        });
        let turns = turned / TAU;
        assert!((1.0..=2.0).contains(&turns), "{turns} turns from r0 {r0}");
    }

    #[test]
    fn vapour_specks_stay_under_one_percent_of_dots() {
        let (mut g, _) = chamber_at(3);
        g.slots = [Slot::default(); SLOTS];
        g.prepare();
        let ds = g.dots.dot_size();
        let mut lit = 0usize;
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                lit += usize::from(g.dots.is_lit(x, y));
            }
        }
        let share = lit as f64 / f64::from(ds.width()) / f64::from(ds.height());
        assert!(share > 0.0 && share <= 0.01, "{share}");
    }
}
