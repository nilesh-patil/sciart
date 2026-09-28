//! Charged-particle tracks curled by magnetic fields, in two acts (SPEC-002
//! `particle_accelerator`):
//!
//! 1. **Collider event**: the end view of a general-purpose LHC-style detector, to scale in
//!    the transverse plane (metres). Each bunch crossing throws out charged tracks that are
//!    exact circles, radius `pT / (0.3 B)`; they stop at the electromagnetic calorimeter,
//!    where towers rise (ECAL for electrons and photons, HCAL for hadrons), except muons,
//!    which cross the solenoid and bend back the other way in the return yoke's opposite
//!    field. Loopers too soft to reach the calorimeter circle in the tracker. A K⁰_S flies
//!    unseen and decays into a π⁺π⁻ vee. The previous event fades out as the next arrives,
//!    so at most two share the screen.
//! 2. **Bubble chamber**: a 1960s heavy-liquid chamber photograph. A negative beam enters
//!    from the left; one beam particle interacts with a nucleus in the liquid and throws out
//!    a star of curling secondaries; photons convert into e⁺e⁻ pairs that spiral inward as
//!    they lose energy; knock-on electrons (δ rays) curl off the beam tracks. Tracks are
//!    strings of bubbles, denser for slower, more ionising particles, on grainy sepia film
//!    with fiducial crosses.
//!
//! Both acts share one exact path, [`Spiral`]: with m = 1 a particle's velocity obeys
//! `v̇ = (−k + i q B) v`, so `z(t) = z₀ + v₀ (1 − e^{−λt}) / λ` with `λ = k − i q B` — a
//! circle of radius `p / B` when `k = 0`, an inward spiral when it loses momentum at rate
//! `k`. The collider uses `k = 0` and `B → 0.3 B` so that `p` is `pT` in GeV and lengths are
//! in metres.
//!
//! Stylisation (stated in SPEC-002): tracks are drawn growing outward over a fraction of a
//! second (real ones take nanoseconds); crossings come every 2.2 s (really 25 ns) with the
//! previous event left fading, where an event display shows one; energies are illustrative,
//! and colours are false.
//!
//! Raster convention: everything is drawn into `Dots` (lines, tracks, bubbles) and `Pixels`
//! (the collider glow) at the end of `update` and `resize`; `render` blits and writes the
//! readout.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::rng::hash01;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, draw, ink};

pub static META: Meta = Meta {
    name: "particle_accelerator",
    category: Category::Physics,
    description: "Collider event display and a 1960s bubble-chamber photograph",
    title: "Particle tracks · bent by fields",
    captions: &[CAPTION_COLLIDER, CAPTION_CHAMBER],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ParticleAccelerator::default())
}

const CAPTION_COLLIDER: &str = "collider event";
const CAPTION_CHAMBER: &str = "bubble chamber";

// ------------------------------------------------------------------------------ timing

/// Act lengths, s; the loop is their sum (ACC-A4).
const ACT_COLLIDER: f64 = 26.0;
const ACT_CHAMBER: f64 = 18.0;
/// Each act fades in and out over this long.
const ACT_FADE: f64 = 0.8;
/// A bunch crossing every `EVENT_PERIOD` s from `FIRST_EVENT`. An event stays bright for
/// `EVENT_HOLD` s, then fades with time constant `EVENT_FADE`, so it is at about a quarter
/// when the next crossing arrives and nearly gone half a second later. Each crossing drops
/// the events at least `EVENT_DROP` s old, so only the new event and the previous one
/// (`EVENT_PERIOD` old) ever share the screen (ACC-A7).
const FIRST_EVENT: f64 = 0.05;
const EVENT_PERIOD: f64 = 2.2;
const EVENT_HOLD: f64 = 1.6;
const EVENT_FADE: f64 = 0.45;
const EVENT_DROP: f64 = 3.8;
// A crossing keeps the previous event (EVENT_PERIOD old) and drops the one before it.
const _: () = assert!(EVENT_PERIOD < EVENT_DROP && EVENT_DROP <= 2.0 * EVENT_PERIOD);
/// A track is drawn growing from its start over this long (stylised: really nanoseconds).
const GROW: f64 = 0.35;
/// One bubble-chamber photograph every `PHOTO` s; the frame goes dark for `PHOTO_DARK` s
/// before the next.
const PHOTO: f64 = 3.0;
const PHOTO_DARK: f64 = 0.25;

// ------------------------------------------------------------------ collider geometry

/// Solenoid field and the return yoke's opposite field, T.
const B_SOLENOID: f64 = 3.8;
const B_RETURN: f64 = -1.8;
/// `pT [GeV] = 0.3 B [T] ρ [m]`.
const GEV_PER_TM: f64 = 0.3;
/// Detector radii, m (a general-purpose LHC detector, transverse plane).
/// Tracker barrel layers drawn: the pixel detector's outer layer (Phase-1, 0.16 m) and
/// three strip layers.
const TRACKER_LAYERS: [f64; 4] = [0.16, 0.43, 0.8, 1.08];
const ECAL: (f64, f64) = (1.29, 1.75);
const HCAL: (f64, f64) = (1.81, 2.95);
const COIL: (f64, f64) = (2.98, 3.18);
/// Muon stations (inner, outer radius, m) sandwiched in the iron yoke.
const MUON_STATIONS: [(f64, f64); 4] = [(4.0, 4.35), (4.9, 5.25), (5.9, 6.25), (6.95, 7.3)];
/// Muons are drawn to here.
const MUON_REACH: f64 = 7.3;
/// The view: `VIEW_R.1` metres span `VIEW_FRAC` of the stage's fit radius once that is at
/// least `VIEW_ROWS.1` rows, so the tracker fills the middle, the calorimeters and coil
/// show to the sides of a wide terminal, and muons run off the edges. Smaller stages zoom
/// in, down to `VIEW_R.0` metres at `VIEW_ROWS.0` rows, so the tracker never shrinks to a
/// few cells (the outer calorimeters are cropped instead).
const VIEW_R: (f64, f64) = (1.8, 2.6);
const VIEW_ROWS: (f64, f64) = (10.0, 20.0);
const VIEW_FRAC: f64 = 0.96;
/// Tower length: `depth · ln(1 + E/E0) / ln(1 + E_MAX/E0)`, capped at the depth.
const TOWER_E0: f64 = 0.5;
const TOWER_EMAX: f64 = 80.0;
/// Angular half-width of a tower, rad: one 5° HCAL tower (0.087 rad), and an ECAL tower
/// drawn as the 5 × 5 crystals (1° each) in front of it.
const TOWER_HALF: f64 = 0.0435;
/// K⁰_S and π± masses (GeV) and the K⁰_S mean decay length cτ (m), PDG.
const M_K: f64 = 0.497_611;
const M_PI: f64 = 0.139_570;
const CTAU_K: f64 = 0.026_84;

const TRACK_CAP: usize = 192;
/// Charged hadrons softer than this (GeV) are loopers, drawn thin and dim (ACC-A14).
const SOFT_PT: f64 = 1.0;
/// Ramp value of the softest charged hadron (ACC-A12, ACC-A14).
const SOFT_V: f64 = 0.45;
const TOWER_CAP: usize = 144;
const MAX_SAMPLES: usize = 900;

// ---------------------------------------------------------------- chamber geometry

/// Chamber units: the stage height is 1 (x in the same units, so circles are round).
/// Field as a curling rate `B` (radius = p / B).
const B_CHAMBER: f64 = 1.0;
const BEAMS: (u32, u32) = (5, 8);
const CHAMBER_CAP: usize = 64;
/// A converted electron keeps this fraction of its radius per turn (bremsstrahlung and
/// ionisation in a heavy liquid), so a pair winds a few visibly tightening turns:
/// `k = −ln(PAIR_TURN)·B/2π`, drawn for `PAIR_TURNS` turns.
const PAIR_TURN: f64 = 0.6;
const PAIR_TURNS: f64 = 4.0;
/// The primary vertex flashes for this long as its star appears.
const VERTEX_FLASH: f64 = 0.5;

// ------------------------------------------------------------------------------ inks

/// Collider palette (false colour, ACC-A13), after the event displays' own convention of
/// colouring tracks by charge: positive tracks `Ice` (cyan), negative `HAlpha` (rose),
/// each brighter with pT; muons near-white `Starlight`, the brightest thing on screen; the
/// K⁰_S flight a dim `Starlight` dash. Calorimeter energy is thin radial bars outside the
/// tracker, `Aurora` in the ECAL and `Plasma` in the HCAL, level by energy. The detector
/// is a faint `Dim` outline, below the dimmest live track, so even the mono
/// theme tells them apart and the tracker's centre stays dark.
const POS_RAMP: Ramp = Ramp::Ice;
const NEG_RAMP: Ramp = Ramp::HAlpha;
const MUON_RAMP: Ramp = Ramp::Starlight;
const NEUTRAL_RAMP: Ramp = Ramp::Starlight;
const ECAL_RAMP: Ramp = Ramp::Aurora;
const HCAL_RAMP: Ramp = Ramp::Plasma;
const STRUCTURE_LEVEL: f64 = 30.0;
/// The outline's role: `Dim`, the theme's quietest object colour, so the rings recede
/// to about 0.15 of full brightness behind the event.
const OUTLINE_ROLE: Role = Role::Dim;
/// A tower's drawn bar is this fraction of its 5° cell wide (at least one stroke), so
/// neighbouring towers read as separate bars rather than a solid block.
const BAR_WIDTH: f64 = 0.25;
/// The outer two muon wheels' share of the outline level.
const OUTER_WHEELS: f64 = 0.7;
/// Chamber palette: warm film, bubbles pale `Ember`; the window is black with a faint rim
/// and film grain, one dim speck per `1 / FILM_SPECKS` dots, new with each frame. Beam
/// tracks are dim and sparse, the star's secondaries the brightest bubbles on the film.
const FILM_RAMP: Ramp = Ramp::Ember;
/// Collider glow: the crossing's flash at the interaction point, `Ice`, at most `GLOW_MAX`
/// of the ramp, fading with time constant `GLOW_TAU` s: a bright pixel and a softer cross,
/// gone within a second so the tracker's centre is dark between crossings.
const GLOW_RAMP: Ramp = Ramp::Ice;
const GLOW_MAX: f64 = 0.6;
const GLOW_TAU: f64 = 0.3;
/// The glow's asinh tone curve, in splat weight per pixel: linear below `GLOW_KNEE`,
/// logarithmic above it, full at `GLOW_WHITE`.
const GLOW_KNEE: f32 = 1.5;
const GLOW_WHITE: f32 = 40.0;
const FLASH_KERNEL: [(i32, i32, f32); 5] = [
    (0, 0, 12.0),
    (-1, 0, 4.0),
    (1, 0, 4.0),
    (0, -1, 4.0),
    (0, 1, 4.0),
];
const FILM_SPECKS: f64 = 0.006;
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

// ============================================================================ physics

/// An exact charged path in a uniform field with momentum loss (ACC-A2): velocity
/// `v(t) = v₀ e^{−λt}`, `λ = k − i q B` (y down, so `q B > 0` turns clockwise on screen),
/// position `z(t) = z₀ + v₀(1 − e^{−λt})/λ`. Speed is momentum (m = 1), so the curvature
/// radius is `p(t)/B` with `p(t) = p₀ e^{−kt}`. Neutral (`B = 0`, `k = 0`) paths are lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Spiral {
    x0: f64,
    y0: f64,
    vx: f64,
    vy: f64,
    /// `q·B`, signed curling rate.
    qb: f64,
    k: f64,
}

impl Spiral {
    pub(crate) fn new(x: f64, y: f64, p: f64, dir: f64, qb: f64, k: f64) -> Self {
        let (s, c) = dir.sin_cos_det();
        Self {
            x0: x,
            y0: y,
            vx: p * c,
            vy: p * s,
            qb,
            k,
        }
    }

    fn straight(&self) -> bool {
        self.qb == 0.0 && self.k == 0.0
    }

    /// `e^{−λt}` as `(re, im)`.
    fn decay(&self, t: f64) -> (f64, f64) {
        let m = (-self.k * t).exp();
        let (s, c) = (self.qb * t).sin_cos_det();
        (m * c, m * s)
    }

    /// `c = v₀/λ`, the spiral's pole offset from `z₀`.
    fn pole(&self) -> (f64, f64) {
        let n = self.k * self.k + self.qb * self.qb;
        // v₀ · conj(λ) / |λ|², conj(λ) = k + i q B.
        (
            (self.vx * self.k - self.vy * self.qb) / n,
            (self.vx * self.qb + self.vy * self.k) / n,
        )
    }

    pub(crate) fn pos(&self, t: f64) -> (f64, f64) {
        if self.straight() {
            return (self.x0 + self.vx * t, self.y0 + self.vy * t);
        }
        let (cx, cy) = self.pole();
        let (er, ei) = self.decay(t);
        (
            self.x0 + cx - (cx * er - cy * ei),
            self.y0 + cy - (cx * ei + cy * er),
        )
    }

    pub(crate) fn vel(&self, t: f64) -> (f64, f64) {
        let (er, ei) = self.decay(t);
        (self.vx * er - self.vy * ei, self.vx * ei + self.vy * er)
    }

    pub(crate) fn p(&self, t: f64) -> f64 {
        self.vx.hypot(self.vy) * (-self.k * t).exp()
    }

    /// The same particle continuing from time `t` in a field of curling rate `qb`.
    fn continued(&self, t: f64, qb: f64) -> Self {
        let (x, y) = self.pos(t);
        let (vx, vy) = self.vel(t);
        Self {
            x0: x,
            y0: y,
            vx,
            vy,
            qb,
            k: self.k,
        }
    }
}

/// First time a circular path (`k = 0`, `qb ≠ 0`) starting inside radius `r` reaches it
/// (ACC-A5), from any start point (a V⁰ daughter starts off the IP), or `None` for a
/// looper whose whole circle stays inside `r`. The path is `z(t) = Z − c e^{i qb t}` about
/// the centre `Z = z₀ + c` (`c` the pole), radius `ρ = |c|`, so `|z| = r` where
/// `cos(θ − arg Z) = (r² − |Z|² − ρ²) / (2|Z|ρ)` (law of cosines), and the swept angle to
/// the nearer root, in the turning sense, divided by `|qb|` is the time.
pub(crate) fn exit_time(path: &Spiral, r: f64) -> Option<f64> {
    if path.qb == 0.0 || path.k != 0.0 {
        return None;
    }
    let (cx, cy) = path.pole();
    let (zx, zy) = (path.x0 + cx, path.y0 + cy);
    let (rho, d) = (cx.hypot(cy), zx.hypot(zy));
    if d + rho < r || d * rho == 0.0 {
        return None;
    }
    let half = ((r * r - d * d - rho * rho) / (2.0 * d * rho))
        .clamp(-1.0, 1.0)
        .acos();
    let (axis, start) = (zy.atan2(zx), (-cy).atan2(-cx));
    let sweep = |to: f64| (path.qb.signum() * (to - start)).rem_euclid(TAU);
    Some(sweep(axis + half).min(sweep(axis - half)) / path.qb.abs())
}

/// Tower length (m) in a calorimeter `depth` deep for energy `e` GeV (ACC-A6).
pub(crate) fn tower_len(e: f64, depth: f64) -> f64 {
    let f = (1.0 + e.max(0.0) / TOWER_E0).ln() / (1.0 + TOWER_EMAX / TOWER_E0).ln();
    depth * f.clamp(0.0, 1.0)
}

/// K⁰_S → π⁺π⁻ in the transverse plane (ACC-A10): a K⁰_S of momentum `p_k` along +x decays
/// with the π⁺ at angle `theta` from the flight axis in its rest frame (momentum p* =
/// 0.206 GeV back to back), boosted along x. Returns each pion's (along, across) momentum,
/// π⁺ first: they sum to (p_k, 0) and their invariant mass is m_K.
pub(crate) fn vee(p_k: f64, theta: f64) -> [(f64, f64); 2] {
    let (gamma, beta_gamma) = (p_k.hypot(M_K) / M_K, p_k / M_K);
    // Equal masses share the energy: E* = m_K / 2 each.
    let e_star = 0.5 * M_K;
    let p_star = (e_star * e_star - M_PI * M_PI).sqrt();
    let (s, c) = theta.sin_cos_det();
    [1.0, -1.0].map(|sign| {
        (
            gamma * sign * p_star * c + beta_gamma * e_star,
            sign * p_star * s,
        )
    })
}

/// What left a collider track, for its ink and its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavour {
    Hadron,
    Electron,
    Muon,
    /// A neutral flight (the K⁰_S before it decays): a faint dashed line.
    Neutral,
}

/// One collider track: its path in metres over `t ∈ [0, len]`, shown from `delay` s after
/// its event's crossing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CTrack {
    path: Spiral,
    len: f64,
    flavour: Flavour,
    /// Positive charge (its ink, ACC-A13); kept apart from `path.qb`, whose sign flips
    /// with the field in the muon's return yoke.
    positive: bool,
    pt: f64,
    born: f64,
    delay: f64,
}

/// A calorimeter tower: `len` m outward from radius `r0` at azimuth `phi`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tower {
    phi: f64,
    r0: f64,
    len: f64,
    ecal: bool,
    e: f64,
    born: f64,
    delay: f64,
}

/// One chamber track in chamber units, shown from `delay` s into its photograph; bubbles
/// are kept with probability `dens` (more ionising ⇒ denser).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BTrack {
    path: Spiral,
    len: f64,
    dens: f64,
    delay: f64,
    /// Hash salt for its bubbles.
    id: u64,
    kind: BKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BKind {
    Beam,
    Secondary,
    Pair,
    Delta,
}

impl BKind {
    /// Bubble brightness on the film: the star is the hero, the beam a dim backdrop.
    fn brightness(self) -> f64 {
        match self {
            BKind::Beam => 0.5,
            BKind::Delta => 0.75,
            BKind::Pair => 0.85,
            BKind::Secondary => 1.0,
        }
    }
}

/// Film exposure at `(x, y)` across a round `window` (centre, radius): 1 inside, easing to
/// 0 just past its rim.
fn vignette((cx, cy, r): (f64, f64, f64), x: f64, y: f64) -> f64 {
    ease::smoothstep((1.08 - (x - cx).hypot(y - cy) / r.max(1e-9)) / 0.25)
}

/// A random unit charge.
fn charge(rng: &mut impl Rng) -> f64 {
    if rng.random_bool(0.5) { 1.0 } else { -1.0 }
}

/// The collider view: stage centre (cells) and rows per metre.
#[derive(Debug, Clone, Copy)]
struct View {
    cx: f64,
    cy: f64,
    k: f64,
}

impl View {
    /// Dot coordinates of a point in metres (y down).
    fn dot(self, x: f64, y: f64) -> (f64, f64) {
        Dots::from_cells(self.cx + x * self.k * ASPECT, self.cy + y * self.k)
    }

    /// Dots per metre.
    fn dots_per_m(self) -> f64 {
        self.k * f64::from(Dots::CELL_H)
    }
}

/// Metres spanned by the view on a stage whose fit radius is `rows` rows (see `VIEW_R`).
pub(crate) fn view_radius(rows: f64) -> f64 {
    let s = ((rows - VIEW_ROWS.0) / (VIEW_ROWS.1 - VIEW_ROWS.0)).clamp(0.0, 1.0);
    ease::lerp(VIEW_R.0, VIEW_R.1, s)
}

/// Brightness of an event `age` s old: full while fresh, then fading (ACC-A7).
pub(crate) fn event_fade(age: f64) -> f64 {
    if age < EVENT_HOLD {
        1.0
    } else {
        (-(age - EVENT_HOLD) / EVENT_FADE).exp()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    Collider,
    Chamber,
}

/// The act at loop time `t` and the time into it.
pub(crate) fn act_at(t: f64) -> (Act, f64) {
    let t = t.rem_euclid(ACT_COLLIDER + ACT_CHAMBER);
    if t < ACT_COLLIDER {
        (Act::Collider, t)
    } else {
        (Act::Chamber, t - ACT_COLLIDER)
    }
}

/// Fade in and out at the act's ends.
fn act_fade(act: Act, t: f64) -> f64 {
    let len = match act {
        Act::Collider => ACT_COLLIDER,
        Act::Chamber => ACT_CHAMBER,
    };
    ease::smoothstep((t.min(len - t) / ACT_FADE).min(1.0))
}

// ============================================================================== scene

#[derive(Debug)]
pub struct ParticleAccelerator {
    /// Loop clock, s.
    t: f64,
    /// Stage (rows above the title row) and canvas sizes.
    stage: Size,
    size: Size,
    tracks: ParticlePool<CTrack>,
    towers: ParticlePool<Tower>,
    chamber: ParticlePool<BTrack>,
    /// Bunch crossings and photographs so far (readouts).
    events: u32,
    photos: u32,
    dots: Dots,
    /// The collider glow, tone-mapped from `exposure`.
    pixels: Pixels,
    /// Collider glow at pixel resolution (like `pixels`): the crossings' flashes and fresh
    /// energy deposits, fading with time constant `GLOW_TAU`.
    exposure: Glow,
    tone: Tone,
}

impl Default for ParticleAccelerator {
    fn default() -> Self {
        Self {
            t: 0.0,
            stage: Size::new(1, 1),
            size: Size::new(1, 1),
            tracks: ParticlePool::with_capacity(TRACK_CAP),
            towers: ParticlePool::with_capacity(TOWER_CAP),
            chamber: ParticlePool::with_capacity(CHAMBER_CAP),
            events: 0,
            photos: 0,
            dots: Dots::default(),
            pixels: Pixels::default(),
            exposure: Glow::default(),
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
        }
    }
}

/// The rows above the title row while the title shows (GEN-R13): the art never meets it.
fn stage_of(size: Size) -> Size {
    let titled = title::span(size, &Title::new(META.title, Some(CAPTION_COLLIDER))).is_some();
    Size::new(size.width(), size.height() - u16::from(titled))
}

impl ParticleAccelerator {
    fn act(&self) -> (Act, f64) {
        act_at(self.t)
    }

    /// The collider view of the current stage.
    fn view(&self) -> View {
        let rows = draw::fit_radius(self.stage, VIEW_FRAC);
        View {
            cx: f64::from(self.stage.width()) / 2.0,
            cy: f64::from(self.stage.height()) / 2.0,
            k: rows / view_radius(rows),
        }
    }

    /// Dot coordinates of a chamber point (stage height 1, x from the stage's left edge).
    fn chamber_dot(&self, x: f64, y: f64) -> (f64, f64) {
        let h = f64::from(self.stage.height());
        Dots::from_cells(x * h * ASPECT, y * h)
    }

    /// Chamber width in chamber units.
    fn chamber_w(&self) -> f64 {
        f64::from(self.stage.width()) / (ASPECT * f64::from(self.stage.height()).max(1.0))
    }

    // ---------------------------------------------------------------- collider events

    fn spawn_track(&mut self, path: Spiral, flavour: Flavour, pt: f64, delay: f64) {
        let qb = path.qb;
        let (len, end) = match flavour {
            Flavour::Neutral => (1.0, None),
            Flavour::Muon => {
                // Through the solenoid, then the yoke's opposite field (ACC-A5).
                let t1 = exit_time(&path, COIL.1).unwrap_or(TAU / qb.abs());
                (t1, Some(t1))
            }
            _ => match exit_time(&path, ECAL.0) {
                Some(t) => (t, Some(t)),
                None => (TAU / qb.abs(), None),
            },
        };
        let born = self.t;
        let positive = qb > 0.0;
        self.tracks.spawn(CTrack {
            path,
            len,
            flavour,
            positive,
            pt,
            born,
            delay,
        });
        if flavour == Flavour::Muon {
            let yoke = path.continued(len, qb * B_RETURN / B_SOLENOID);
            // Straight enough out here that the reach is the arc length to MUON_REACH.
            let reach = (MUON_REACH - COIL.1) / pt * 1.15;
            self.tracks.spawn(CTrack {
                path: yoke,
                len: reach,
                flavour,
                positive,
                pt,
                born,
                delay: delay + GROW,
            });
        }
        // A tower where a hadron or electron meets the calorimeter (ACC-A6): electrons
        // shower fully in the ECAL, hadrons leave a quarter there and the rest in the HCAL.
        if let (Some(t), Flavour::Hadron | Flavour::Electron) = (end, flavour) {
            let (x, y) = path.pos(t);
            let phi = y.atan2(x);
            let hadron = flavour == Flavour::Hadron;
            let ecal_e = if hadron { 0.25 * pt } else { pt };
            self.spawn_tower(phi, true, ecal_e, delay + GROW);
            if hadron {
                self.spawn_tower(phi, false, 0.75 * pt, delay + GROW);
            }
        }
    }

    fn spawn_tower(&mut self, phi: f64, ecal: bool, e: f64, delay: f64) {
        let (r0, r1) = if ecal { ECAL } else { HCAL };
        let len = tower_len(e, r1 - r0);
        if len < 0.03 {
            return;
        }
        self.towers.spawn(Tower {
            phi,
            r0,
            len,
            ecal,
            e,
            born: self.t,
            delay,
        });
    }

    /// One bunch crossing: soft tracks, maybe jets, a dimuon, an electron and a V⁰.
    fn crossing(&mut self, ctx: &mut Ctx) {
        self.events += 1;
        let rng = &mut ctx.rng;
        let qb = |q: f64| q * GEV_PER_TM * B_SOLENOID;
        let from_ip = |pt: f64, phi: f64, q: f64| Spiral::new(0.0, 0.0, pt, phi, qb(q), 0.0);
        // Soft underlying event: a steeply falling pT spectrum, many loopers.
        for _ in 0..rng.random_range(6..=12) {
            let pt = (0.45 * rng.random::<f64>().max(1e-3).powf(-0.5)).min(20.0);
            let q = charge(rng);
            let phi = rng.random_range(0.0..TAU);
            self.spawn_track(from_ip(pt, phi, q), Flavour::Hadron, pt, 0.0);
        }
        // A back-to-back dijet: collimated stiff tracks and neutral energy in the ECAL.
        if rng.random_bool(0.7) {
            let axis = rng.random_range(0.0..TAU);
            for jet in [axis, axis + PI + gaussian(rng, 0.0, 0.15)] {
                for _ in 0..rng.random_range(4..=7) {
                    let pt = rng.random_range(2.0..25.0);
                    let phi = jet + gaussian(rng, 0.0, 0.1);
                    let q = charge(rng);
                    self.spawn_track(from_ip(pt, phi, q), Flavour::Hadron, pt, 0.0);
                }
                for _ in 0..rng.random_range(2..=4) {
                    let phi = jet + gaussian(rng, 0.0, 0.1);
                    self.spawn_tower(phi, true, rng.random_range(3.0..20.0), GROW);
                }
            }
        }
        // A dimuon (as from a Z): two stiff muons roughly back to back.
        if rng.random_bool(0.3) {
            let phi = rng.random_range(0.0..TAU);
            let q = charge(rng);
            for (dphi, sign) in [(0.0, 1.0), (PI + gaussian(rng, 0.0, 0.25), -1.0)] {
                let pt = rng.random_range(20.0..45.0);
                self.spawn_track(from_ip(pt, phi + dphi, q * sign), Flavour::Muon, pt, 0.0);
            }
        }
        // An isolated electron: a stiff track and a tall ECAL tower.
        if rng.random_bool(0.3) {
            let pt = rng.random_range(8.0..40.0);
            let phi = rng.random_range(0.0..TAU);
            let q = charge(rng);
            self.spawn_track(from_ip(pt, phi, q), Flavour::Electron, pt, 0.0);
        }
        // A K⁰_S → π⁺π⁻ vee: it flies βγ·cτ·(−ln u) unseen (clamped to where the tracker
        // shows it), then decays as a true two-body decay (ACC-A10).
        if rng.random_bool(0.55) {
            let phi = rng.random_range(0.0..TAU);
            let p_k = rng.random_range(1.5..6.0);
            let u: f64 = rng.random::<f64>();
            let flight = (p_k / M_K * CTAU_K * -(1.0 - u).ln()).clamp(0.12, 0.45);
            let flown = Spiral::new(0.0, 0.0, flight, phi, 0.0, 0.0);
            self.spawn_track(flown, Flavour::Neutral, 0.0, 0.0);
            let (vx, vy) = flown.pos(1.0);
            let pions = vee(p_k, rng.random_range(0.0..TAU));
            for ((along, across), q) in pions.into_iter().zip([1.0, -1.0]) {
                let pt = along.hypot(across);
                let path = Spiral::new(vx, vy, pt, phi + across.atan2(along), qb(q), 0.0);
                self.spawn_track(path, Flavour::Hadron, pt, 0.5 * GROW);
            }
        }
        // The crossing's flash at the interaction point: a bright pixel and a softer cross.
        let (x, y) = self.view().dot(0.0, 0.0);
        self.exposure.splat(x, y, &FLASH_KERNEL, 1.0);
    }

    // ------------------------------------------------------------ chamber photographs

    /// One photograph: beam tracks with δ rays, an interaction star and converted photons.
    fn photograph(&mut self, ctx: &mut Ctx) {
        self.photos += 1;
        self.chamber.clear();
        let rng = &mut ctx.rng;
        let w = self.chamber_w();
        let nb = rng.random_range(BEAMS.0..=BEAMS.1);
        let star = rng.random_range(0..nb);
        let mut id = u64::from(self.photos) << 16;
        let mut next_id = || {
            id += 1;
            id
        };
        // A negative beam: stiff, nearly straight, entering from the left.
        let p_beam = 6.0 * w.max(1.0);
        for b in 0..nb {
            let y = 0.18 + 0.64 * (f64::from(b) + rng.random_range(0.2..0.8)) / f64::from(nb);
            let dir = gaussian(rng, 0.0, 0.01);
            let path = Spiral::new(-0.02, y, p_beam, dir, -B_CHAMBER, 0.0);
            let full = (w + 0.05) / p_beam;
            let len = if b == star {
                rng.random_range(0.35..0.6) * full
            } else {
                full
            };
            self.chamber.spawn(BTrack {
                path,
                len,
                dens: 0.3,
                delay: 0.0,
                id: next_id(),
                kind: BKind::Beam,
            });
            // Knock-on electrons: tight curls, all turning one way (they are all e⁻).
            for _ in 0..rng.random_range(0..=2) {
                let at = rng.random_range(0.1..0.95) * len;
                let (x, y) = path.pos(at);
                let side = charge(rng);
                let d = Spiral::new(
                    x,
                    y,
                    rng.random_range(0.04..0.1),
                    dir + side * rng.random_range(1.0..1.4),
                    -B_CHAMBER,
                    1.0,
                );
                self.chamber.spawn(BTrack {
                    path: d,
                    len: 3.0,
                    dens: 0.8,
                    delay: at / len * GROW,
                    id: next_id(),
                    kind: BKind::Delta,
                });
            }
            if b == star {
                let (vx, vy) = path.pos(len);
                // The star on a nucleus of the heavy liquid (break-up protons make any prong
                // count and net charge possible): slower secondaries denser and curlier.
                for _ in 0..rng.random_range(3..=6) {
                    let p = 0.06 + 0.5 * rng.random::<f64>().powi(2);
                    let q = charge(rng);
                    let dir = rng.random_range(-1.3..1.3);
                    self.chamber.spawn(BTrack {
                        path: Spiral::new(vx, vy, p, dir, q * B_CHAMBER, 0.12),
                        len: 5.0,
                        dens: (0.45 + 0.05 / p).min(1.0),
                        delay: GROW,
                        id: next_id(),
                        kind: BKind::Secondary,
                    });
                }
                // Photons (π⁰ → γγ) fly unseen and convert: e⁺e⁻ spiralling inward.
                let k = -PAIR_TURN.ln() * B_CHAMBER / TAU;
                for _ in 0..rng.random_range(1..=2) {
                    let dir: f64 = rng.random_range(-0.9..0.9);
                    let d = rng.random_range(0.08..0.3);
                    let (cx, cy) = (vx + d * dir.cos_det(), vy + d * dir.sin_det());
                    let e = rng.random_range(0.15..0.45);
                    let share = rng.random_range(0.25..0.75);
                    for (q, p) in [(1.0, e * share), (-1.0, e * (1.0 - share))] {
                        self.chamber.spawn(BTrack {
                            path: Spiral::new(cx, cy, p, dir + q * 0.04, q * B_CHAMBER, k),
                            len: PAIR_TURNS * TAU / B_CHAMBER,
                            dens: 0.55,
                            delay: 1.6 * GROW,
                            id: next_id(),
                            kind: BKind::Pair,
                        });
                    }
                }
            }
        }
    }

    // ----------------------------------------------------------------------- raster

    fn rasterise(&mut self) {
        self.dots.clear();
        let (act, t) = self.act();
        let fade = act_fade(act, t);
        match act {
            Act::Collider => self.rasterise_collider(fade),
            Act::Chamber => {
                self.pixels.clear();
                self.rasterise_chamber(t, fade);
            }
        }
    }

    fn rasterise_collider(&mut self, fade: f64) {
        let view = self.view();
        // The glow of the crossings' flashes; the detector itself is line art on black,
        // like an event display.
        self.exposure
            .to_pixels(&mut self.pixels, &self.tone, |level| {
                ink::ramp(GLOW_RAMP, GLOW_MAX * fade * f64::from(level) / 255.0)
            });

        // Detector outlines.
        let structure = Ink::new(OUTLINE_ROLE, (STRUCTURE_LEVEL * fade) as u8);
        let dots = &mut self.dots;
        for r in TRACKER_LAYERS {
            circle(dots, view, r, structure);
        }
        for r in [ECAL.0, ECAL.1, HCAL.0, HCAL.1, COIL.0, COIL.1] {
            circle(dots, view, r, structure);
        }
        // The outer two muon wheels a step dimmer, so the frame's edges stay quiet where
        // a wide terminal shows them.
        let outer = Ink::new(OUTLINE_ROLE, (OUTER_WHEELS * STRUCTURE_LEVEL * fade) as u8);
        for (k, (a, b)) in MUON_STATIONS.into_iter().enumerate() {
            let ink = if k >= 2 { outer } else { structure };
            polygon(dots, view, a, 12, ink);
            polygon(dots, view, b, 12, ink);
        }

        // Towers, then tracks over them.
        let t = self.t;
        for tw in self.towers.iter() {
            draw_tower(dots, view, t, tw, fade);
        }
        for tr in self.tracks.iter() {
            draw_ctrack(dots, view, t, tr, fade);
        }
    }

    /// The chamber's round window in chamber units: centre and radius (ACC-A8), its rim
    /// (at 1.02 r) clear of the stage's top and bottom rows.
    fn window(&self) -> (f64, f64, f64) {
        let w = self.chamber_w();
        let rows = f64::from(self.stage.height()).max(1.0);
        let fit = (1.0 - 2.0 / rows).max(0.5);
        (w / 2.0, 0.5, 0.5 * w.min(fit) / 1.02)
    }

    /// The chamber is line art on black: only `Dots`, no `Pixels`.
    fn rasterise_chamber(&mut self, t: f64, fade: f64) {
        let age = t.rem_euclid(PHOTO);
        // The frame goes dark while the film advances.
        let dark =
            ease::smoothstep((PHOTO - age) / PHOTO_DARK).min(ease::smoothstep(age / PHOTO_DARK));
        let lit = fade * dark;
        let frame = u64::from(self.photos);
        // Film grain: dim specks that change with each frame, over the lit window.
        let window = self.window();
        let size = self.dots.dot_size();
        let (dw, dh) = (f64::from(size.width()), f64::from(size.height()));
        let per_unit = f64::from(Dots::CELL_H) * f64::from(self.stage.height()).max(1.0);
        let specks = (FILM_SPECKS * dw * dh) as u64;
        for i in 0..specks {
            let key = (frame << 32) ^ i;
            let (x, y) = (hash01(key) * dw, hash01(key ^ 0xA5A5_A5A5) * dh);
            let v = (0.25 + 0.3 * hash01(key ^ 0x3C3C))
                * lit
                * vignette(window, x / per_unit, y / per_unit);
            let ink = ink::ramp(FILM_RAMP, v);
            if ink.level > 20 {
                self.dots.plot(x, y, ink);
            }
        }
        // The window's rim and the fiducial crosses etched on it.
        let (cx, cy, r) = window;
        let rim = ink::ramp(FILM_RAMP, 0.3 * lit);
        let n = ((TAU * r * per_unit).ceil() as usize).clamp(8, 4000);
        for i in 0..n {
            let a = TAU * i as f64 / n as f64;
            let (x, y) = self.chamber_dot(cx + 1.02 * r * a.cos_det(), cy + 1.02 * r * a.sin_det());
            self.dots.plot(x, y, rim);
        }
        let cross = ink::ramp(FILM_RAMP, 0.4 * lit);
        for (i, j) in [
            (-1.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
            (0.0, 0.0),
            (0.0, -1.2),
            (0.0, 1.2),
        ] {
            let (x, y) = self.chamber_dot(cx + 0.45 * r * i, cy + 0.45 * r * j);
            self.dots.line(x - 2.0, y, x + 2.0, y, cross);
            self.dots.line(x, y - 2.0, x, y + 2.0, cross);
        }
        // Bubbles, only where the window is lit.
        let h_rows = f64::from(self.stage.height());
        let dots = &mut self.dots;
        for tr in self.chamber.iter() {
            let a = age - tr.delay;
            if a <= 0.0 {
                continue;
            }
            let (x0, y0) = (tr.path.x0 * h_rows * ASPECT, tr.path.y0 * h_rows);
            // The primary vertex flashes as its star appears: a small bright burst.
            if tr.kind == BKind::Secondary && a < VERTEX_FLASH {
                let flash = ink::ramp(FILM_RAMP, (1.0 - a / VERTEX_FLASH) * lit);
                let (dx, dy) = Dots::from_cells(x0, y0);
                for (ox, oy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1), (-2, 0), (2, 0)] {
                    dots.plot(dx + f64::from(ox), dy + f64::from(oy), flash);
                }
            }
            let grow = if tr.kind == BKind::Beam {
                GROW
            } else {
                2.0 * GROW
            };
            let shown = tr.len * ease::out_quad(a / grow);
            let (id, dens, bright) = (tr.id, tr.dens, tr.kind.brightness());
            trace(
                &tr.path,
                shown,
                tr.path.p(0.0) * per_unit,
                dots,
                |j, x, y| {
                    let key = id.wrapping_mul(0x100_0193) ^ j as u64;
                    if hash01(key) >= dens {
                        return None;
                    }
                    // Bubbles vary in size and brightness on the film.
                    let v =
                        bright * (0.7 + 0.3 * hash01(key ^ 0x5555)) * lit * vignette(window, x, y);
                    let ink = ink::ramp(FILM_RAMP, v);
                    (ink.level > 40)
                        .then(|| (Dots::from_cells(x * h_rows * ASPECT, y * h_rows), ink))
                },
            );
        }
    }

    /// Readout on the title row, right-aligned clear of the title (GEN-R13): the long form,
    /// else the short one, else nothing.
    fn readout(&self, canvas: &mut Canvas) {
        if self.stage == self.size {
            return;
        }
        let room = title::readout_room(self.size, &Title::of(self));
        let (act, _) = self.act();
        let mut text = StackText::<32>::new();
        for long in [true, false] {
            text.clear();
            // Each form is under 32 bytes, so writing never fails.
            let _ = match (act, long) {
                (Act::Collider, true) => write!(text, "event {}  B {B_SOLENOID} T", self.events),
                (Act::Collider, false) => write!(text, "B {B_SOLENOID} T"),
                (Act::Chamber, true) => write!(text, "film frame {}", self.photos),
                (Act::Chamber, false) => write!(text, "frame {}", self.photos),
            };
            if text.len() <= room {
                let y = i32::from(self.size.height()) - 1;
                draw::label_right(canvas, y, text.as_str(), READOUT_INK);
                return;
            }
        }
    }
}

/// A detector circle of radius `r` m.
fn circle(dots: &mut Dots, view: View, r: f64, ink: Ink) {
    let n = ((TAU * r * view.dots_per_m()).ceil() as usize).clamp(8, 4000);
    for i in 0..n {
        let a = TAU * i as f64 / n as f64;
        let (x, y) = view.dot(r * a.cos_det(), r * a.sin_det());
        dots.plot(x, y, ink);
    }
}

/// A regular `sides`-gon of inradius `r` m (muon wheels are dodecagons).
fn polygon(dots: &mut Dots, view: View, r: f64, sides: u32, ink: Ink) {
    let rc = r / (PI / f64::from(sides)).cos_det();
    let corner = |i: u32| {
        let a = TAU * (f64::from(i) + 0.5) / f64::from(sides);
        view.dot(rc * a.cos_det(), rc * a.sin_det())
    };
    for i in 0..sides {
        let ((x0, y0), (x1, y1)) = (corner(i), corner(i + 1));
        dots.line(x0, y0, x1, y1, ink);
    }
}

fn draw_tower(dots: &mut Dots, view: View, t: f64, tw: &Tower, fade: f64) {
    let age = t - tw.born - tw.delay;
    if age <= 0.0 {
        return;
    }
    let ink = tower_ink(tw, event_fade(t - tw.born) * fade);
    if ink.level == 0 {
        return;
    }
    let len = tw.len * ease::out_quad(age / GROW);
    // A thin bar down the tower's axis (ACC-A13): radial strokes about one dot apart at
    // its outer edge across `BAR_WIDTH` of the cell, one stroke on a small stage.
    let half = BAR_WIDTH * TOWER_HALF;
    let outer = (tw.r0 + len) * view.dots_per_m();
    let strokes = ((2.0 * half * outer).floor() as i32).clamp(0, 1);
    for s in 0..=strokes {
        let a = if strokes == 0 {
            tw.phi
        } else {
            tw.phi + half * (2.0 * f64::from(s) / f64::from(strokes) - 1.0)
        };
        let (sn, c) = a.sin_cos_det();
        let (x0, y0) = view.dot(tw.r0 * c, tw.r0 * sn);
        let (x1, y1) = view.dot((tw.r0 + len) * c, (tw.r0 + len) * sn);
        dots.line(x0, y0, x1, y1, ink);
    }
}

/// A tower's ink at event brightness `bright` (ACC-A13): its calorimeter's ramp, from 0.5
/// for the softest deposit to 0.95 at 30 GeV and above.
fn tower_ink(tw: &Tower, bright: f64) -> Ink {
    let v = (0.5 + 0.45 * (tw.e / 30.0).min(1.0)) * bright;
    ink::ramp(if tw.ecal { ECAL_RAMP } else { HCAL_RAMP }, v)
}

/// A collider track's ramp and value at full event brightness (ACC-A12, ACC-A13): all clear
/// of the detector outline. Charged tracks take their charge's hue, for the
/// softest loopers at `SOFT_V` (so the tracker's centre stays calm and the stiff jet
/// tracks carry the event's axis) to 0.82 for stiff ones, where each hue is still
/// saturated; muons are near-white; the K⁰_S flight is a dim neutral dash.
fn fresh_tone(flavour: Flavour, pt: f64, positive: bool) -> (Ramp, f64) {
    let charged = if positive { POS_RAMP } else { NEG_RAMP };
    match flavour {
        Flavour::Muon => (MUON_RAMP, 1.0),
        Flavour::Neutral => (NEUTRAL_RAMP, 0.45),
        Flavour::Electron => (charged, 0.82),
        Flavour::Hadron => (charged, SOFT_V + (0.82 - SOFT_V) * (pt / 10.0).min(1.0)),
    }
}

/// Whether a track is a soft looper (ACC-A14): a charged hadron under [`SOFT_PT`] curls up
/// inside the tracker, so it is drawn as a thin dotted trace (every third sample, about
/// two dots apart) at the
/// dim end of its ramp, and the stiff tracks read as the jet's axis through the clutter.
fn is_looper(tr: &CTrack) -> bool {
    tr.flavour == Flavour::Hadron && tr.pt < SOFT_PT
}

fn draw_ctrack(dots: &mut Dots, view: View, t: f64, tr: &CTrack, fade: f64) {
    let age = t - tr.born - tr.delay;
    if age <= 0.0 {
        return;
    }
    let (ramp, v) = fresh_tone(tr.flavour, tr.pt, tr.positive);
    let ink = ink::ramp(ramp, v * event_fade(t - tr.born) * fade);
    if ink.level == 0 {
        return;
    }
    let dashed = tr.flavour == Flavour::Neutral;
    let sparse = is_looper(tr);
    let shown = tr.len * ease::out_quad(age / GROW);
    let speed = tr.path.p(0.0) * view.dots_per_m();
    trace(&tr.path, shown, speed, dots, |j, x, y| {
        let on = if dashed {
            (j / 3) % 2 == 0
        } else {
            !sparse || j % 3 == 0
        };
        on.then(|| (view.dot(x, y), ink))
    });
}

/// Walk `path` over `t ∈ [0, len]` with samples ≤ 0.7 dots apart at speed `dots_per_t`,
/// calling `at(sample index, x, y)` for each; its `Some((dot, ink))` is plotted. The
/// spiral is advanced by one complex multiply per sample (`e^{−λh}` is constant).
fn trace(
    path: &Spiral,
    len: f64,
    dots_per_t: f64,
    dots: &mut Dots,
    mut at: impl FnMut(usize, f64, f64) -> Option<((f64, f64), Ink)>,
) {
    if !(len > 0.0 && dots_per_t.is_finite() && dots_per_t > 0.0) {
        return;
    }
    let n = ((len * dots_per_t / 0.7).ceil() as usize).clamp(1, MAX_SAMPLES);
    let h = len / n as f64;
    let mut emit = |j: usize, x: f64, y: f64, dots: &mut Dots| {
        if let Some(((dx, dy), ink)) = at(j, x, y) {
            dots.plot(dx, dy, ink);
        }
    };
    if path.straight() {
        for j in 0..=n {
            let (x, y) = path.pos(h * j as f64);
            emit(j, x, y, dots);
        }
        return;
    }
    let (cx, cy) = path.pole();
    let (zx, zy) = (path.x0 + cx, path.y0 + cy);
    let (wr, wi) = path.decay(h);
    let (mut ur, mut ui) = (cx, cy);
    for j in 0..=n {
        emit(j, zx - ur, zy - ui, dots);
        (ur, ui) = (ur * wr - ui * wi, ur * wi + ui * wr);
    }
}

impl Generator for ParticleAccelerator {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(match self.act().0 {
            Act::Collider => CAPTION_COLLIDER,
            Act::Chamber => CAPTION_CHAMBER,
        })
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.stage = stage_of(size);
        self.dots.resize(self.stage);
        self.pixels.resize(self.stage);
        // Collider tracks live in metres and chamber tracks in stage heights, so both
        // survive a resize; only the glow is in pixels, and restarts dark.
        self.exposure.resize(self.stage);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let (act0, t0) = self.act();
        self.t += dt;
        let (act, t1) = self.act();
        if act != act0 {
            self.tracks.clear();
            self.towers.clear();
            self.chamber.clear();
            self.exposure.clear();
        }
        // A new act starts its own clock at 0, so its first crossing or photograph is due.
        let from = if act == act0 { t0 } else { 0.0 };
        match act {
            Act::Collider => {
                // Every crossing passed in this step (at most one at dt ≤ 0.25); before the
                // first, k0 = −1.
                let k0 = ((from - FIRST_EVENT) / EVENT_PERIOD).floor();
                let k1 = ((t1 - FIRST_EVENT) / EVENT_PERIOD).floor();
                if k1 > k0 {
                    let t = self.t;
                    self.tracks.retain(|tr| t - tr.born < EVENT_DROP);
                    self.towers.retain(|tw| t - tw.born < EVENT_DROP);
                    self.crossing(ctx);
                }
                self.exposure.decay((-dt / GLOW_TAU).exp() as f32);
            }
            Act::Chamber => {
                if act != act0 || (t1 / PHOTO).floor() > (from / PHOTO).floor() {
                    self.photograph(ctx);
                }
            }
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels.blit(canvas, |_, _, _, _| None);
        self.dots.blit_over(canvas, &self.pixels);
        self.readout(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(w: u16, h: u16) -> (ParticleAccelerator, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(7, size);
        let mut g = ParticleAccelerator::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// Run `g` to loop time `t` with steps cycling through `dts`.
    fn run_to(g: &mut ParticleAccelerator, ctx: &mut Ctx, t: f64, dts: &[f64]) {
        let mut i = 0;
        while g.t < t {
            g.update(dts[i % dts.len()].min(t - g.t + 1e-9), ctx);
            i += 1;
        }
    }

    #[test]
    fn acc_a2_curvature_radius_is_p_over_b_and_turns_with_charge() {
        let mut rng = Ctx::new(11, Size::new(1, 1)).rng;
        for i in 0..48 {
            let q = if i % 2 == 0 { 1.0 } else { -1.0 };
            let b = rng.random_range(0.5..4.0);
            let k = [0.0, 0.3, 1.2][i % 3];
            let p0 = rng.random_range(0.5..20.0);
            let dir = rng.random_range(0.0..TAU);
            let sp = Spiral::new(3.0, -2.0, p0, dir, q * b, k);
            let (x, y) = sp.pos(0.0);
            assert!((x - 3.0).abs() < 1e-12 && (y + 2.0).abs() < 1e-12);
            for t in [0.05, 0.4, 1.1] {
                // Central differences on the closed-form path.
                let h = 1e-4;
                let (x0, y0) = sp.pos(t - h);
                let (x1, y1) = sp.pos(t);
                let (x2, y2) = sp.pos(t + h);
                let (vx, vy) = ((x2 - x0) / (2.0 * h), (y2 - y0) / (2.0 * h));
                let (ax, ay) = (
                    (x2 - 2.0 * x1 + x0) / (h * h),
                    (y2 - 2.0 * y1 + y0) / (h * h),
                );
                let speed = vx.hypot(vy);
                // Signed curvature in screen coordinates (y down): κ > 0 is clockwise.
                let kappa = (vx * ay - vy * ax) / speed.powi(3);
                let expect = sp.p(t) / b;
                assert!(
                    (1.0 / kappa.abs() - expect).abs() < 1e-3 * expect,
                    "r vs p/B"
                );
                assert_eq!(kappa.signum(), q, "turning sign follows the charge");
                assert!(
                    (speed - sp.p(t)).abs() < 1e-4 * sp.p(t),
                    "speed is momentum"
                );
                let (vx, vy) = sp.vel(t);
                assert!((vx.hypot(vy) - sp.p(t)).abs() < 1e-9 * p0);
            }
            if k > 0.0 {
                assert!(sp.p(1.0) < sp.p(0.5), "losing momentum, it spirals inward");
            }
        }
        // A neutral path is a straight line.
        let n = Spiral::new(0.0, 0.0, 2.0, 0.3, 0.0, 0.0);
        let (x, y) = n.pos(1.5);
        assert!(
            (x - 3.0 * 0.3f64.cos_det()).abs() < 1e-12
                && (y - 3.0 * 0.3f64.sin_det()).abs() < 1e-12
        );
    }

    #[test]
    fn acc_a4_two_acts_each_with_its_caption() {
        let period = ACT_COLLIDER + ACT_CHAMBER;
        assert_eq!(act_at(0.0), (Act::Collider, 0.0));
        assert_eq!(act_at(ACT_COLLIDER).0, Act::Chamber);
        assert_eq!(act_at(period + 1.0), (Act::Collider, 1.0));
        assert_eq!(META.captions, [CAPTION_COLLIDER, CAPTION_CHAMBER]);
        let (mut g, mut ctx) = fresh(120, 40);
        let mut seen = [0u32; 2];
        for i in 0..(2.0 * period / 0.1) as usize {
            g.update(if i % 3 == 0 { 0.25 } else { 0.05 }, &mut ctx);
            let (act, _) = act_at(g.t);
            let want = if act == Act::Collider {
                CAPTION_COLLIDER
            } else {
                CAPTION_CHAMBER
            };
            assert_eq!(g.caption(), Some(want), "t = {}", g.t);
            // Each act keeps only its own things alive.
            match act {
                Act::Collider => assert!(g.chamber.is_empty()),
                Act::Chamber => assert!(g.tracks.is_empty() && g.towers.is_empty()),
            }
            seen[act as usize] += 1;
        }
        assert!(seen[0] > 0 && seen[1] > 0);
    }

    #[test]
    fn acc_a5_collider_tracks_are_circles_stopped_by_the_calorimeter() {
        // Exit times from the IP: the radius reaches r exactly, and a looper (2ρ < r) never
        // leaves.
        for (pt, q) in [(0.5, 1.0), (1.2, -1.0), (8.0, 1.0), (40.0, -1.0)] {
            let qb = q * GEV_PER_TM * B_SOLENOID;
            let rho = pt / qb.abs();
            let sp = Spiral::new(0.0, 0.0, pt, 1.0, qb, 0.0);
            match exit_time(&sp, ECAL.0) {
                Some(t) => {
                    assert!(2.0 * rho >= ECAL.0);
                    let (x, y) = sp.pos(t);
                    assert!((x.hypot(y) - ECAL.0).abs() < 1e-9, "pt {pt}");
                    // The chord formula of a circle through the origin agrees.
                    let chord = 2.0 * (ECAL.0 * qb.abs() / (2.0 * pt)).asin() / qb.abs();
                    assert!((t - chord).abs() < 1e-9, "pt {pt}");
                }
                None => assert!(2.0 * rho < ECAL.0, "pt {pt} loops"),
            }
        }
        // From a displaced start, any direction: the first crossing of r, never earlier.
        let mut rng = Ctx::new(5, Size::new(1, 1)).rng;
        for _ in 0..500 {
            let (r0, a) = (rng.random_range(0.0..1.0), rng.random_range(0.0..TAU));
            let pt = rng.random_range(0.05..30.0);
            let qb = charge(&mut rng) * GEV_PER_TM * B_SOLENOID;
            let sp = Spiral::new(
                r0 * a.cos_det(),
                r0 * a.sin_det(),
                pt,
                rng.random_range(0.0..TAU),
                qb,
                0.0,
            );
            let rho = pt / qb.abs();
            let (cx, cy) = sp.pole();
            let d = (sp.x0 + cx).hypot(sp.y0 + cy);
            let radius = |t: f64| {
                let (x, y) = sp.pos(t);
                x.hypot(y)
            };
            match exit_time(&sp, ECAL.0) {
                Some(t) => {
                    assert!((radius(t) - ECAL.0).abs() < 1e-9);
                    assert!((0..100).all(|j| radius(t * f64::from(j) / 100.0) <= ECAL.0 + 1e-9));
                }
                None => assert!(d + rho < ECAL.0),
            }
        }
        // Radius in metres: pT = 0.3 B ρ, so a 1 GeV track in 3.8 T curls with ρ = 0.877 m.
        assert!((1.0 / (GEV_PER_TM * B_SOLENOID) - 0.877).abs() < 1e-3);

        // Generated crossings: every charged hadron or electron, IP or V⁰ daughter, stops on
        // the ECAL face or is a whole looper circle inside it.
        let (mut g, mut ctx) = fresh(160, 48);
        let (mut muons, mut seen, mut displaced) = (0, 0, 0);
        for _ in 0..40 {
            g.tracks.clear();
            g.towers.clear();
            g.crossing(&mut ctx);
            let tracks: Vec<CTrack> = g.tracks.iter().copied().collect();
            for (i, tr) in tracks.iter().enumerate() {
                let r_end = {
                    let (x, y) = tr.path.pos(tr.len);
                    x.hypot(y)
                };
                match tr.flavour {
                    Flavour::Hadron | Flavour::Electron => {
                        seen += 1;
                        displaced += usize::from((tr.path.x0, tr.path.y0) != (0.0, 0.0));
                        let full = TAU / tr.path.qb.abs();
                        if (tr.len - full).abs() > 1e-12 {
                            assert!((r_end - ECAL.0).abs() < 1e-9, "stops at the ECAL face");
                        } else {
                            // A looper stays inside the tracker volume.
                            let (cx, cy) = tr.path.pole();
                            let d = (tr.path.x0 + cx).hypot(tr.path.y0 + cy);
                            assert!(d + tr.pt / tr.path.qb.abs() < ECAL.0);
                        }
                    }
                    Flavour::Muon if (tr.path.x0, tr.path.y0) == (0.0, 0.0) => {
                        muons += 1;
                        assert!((r_end - COIL.1).abs() < 1e-9, "leaves through the coil");
                        // The yoke segment continues from there, bending the other way.
                        let yoke = tracks[i + 1];
                        assert_eq!(yoke.flavour, Flavour::Muon);
                        assert_eq!((yoke.path.x0, yoke.path.y0), tr.path.pos(tr.len));
                        let ratio = yoke.path.qb / tr.path.qb;
                        assert!((ratio - B_RETURN / B_SOLENOID).abs() < 1e-12 && ratio < 0.0);
                    }
                    _ => {}
                }
            }
        }
        assert!(
            seen > 200 && displaced > 10 && muons > 0,
            "{seen} tracks, {displaced} displaced, {muons} muons"
        );
    }

    #[test]
    fn acc_a6_towers_rise_where_tracks_meet_the_calorimeter() {
        assert_eq!(tower_len(0.0, 0.5), 0.0);
        let mut last = 0.0;
        for e in [0.5, 2.0, 10.0, 50.0, 80.0, 500.0] {
            let l = tower_len(e, 0.46);
            assert!(l >= last && l <= 0.46, "monotone and within the depth");
            last = l;
        }
        assert_eq!(tower_len(TOWER_EMAX, 1.15), 1.15);
        // A tower is one 5° HCAL tower wide.
        assert!((2.0 * TOWER_HALF - 5f64.to_radians()).abs() < 1e-3);
        let (mut g, mut ctx) = fresh(160, 48);
        let mut electrons = 0;
        for _ in 0..30 {
            g.tracks.clear();
            g.towers.clear();
            g.crossing(&mut ctx);
            for tw in g.towers.iter() {
                let (r0, r1) = if tw.ecal { ECAL } else { HCAL };
                assert_eq!(tw.r0, r0);
                assert!(tw.len > 0.0 && tw.len <= r1 - r0 + 1e-12);
            }
            for tr in g.tracks.iter().filter(|t| t.flavour == Flavour::Electron) {
                electrons += 1;
                let (x, y) = tr.path.pos(tr.len);
                let phi = y.atan2(x);
                let hit = g
                    .towers
                    .iter()
                    .any(|tw| tw.ecal && (tw.phi - phi).abs() < 1e-12 && tw.e == tr.pt);
                assert!(
                    hit,
                    "an electron's full energy lands in the ECAL at its exit"
                );
            }
        }
        assert!(electrons > 0);
    }

    #[test]
    fn acc_a7_the_previous_event_fades_and_at_most_two_share_the_screen() {
        assert_eq!(event_fade(0.0), 1.0);
        assert_eq!(event_fade(EVENT_HOLD - 1e-9), 1.0);
        let mut last = 1.0;
        for i in 0..100 {
            let f = event_fade(f64::from(i) * 0.1);
            assert!(f <= last && f > 0.0);
            last = f;
        }
        // When the next crossing arrives the previous event is at about a quarter, and
        // half a second later under a tenth.
        let at_next = event_fade(EVENT_PERIOD);
        assert!((0.15..=0.35).contains(&at_next), "{at_next}");
        assert!(event_fade(EVENT_PERIOD + 0.5) < 0.1);
        assert!(event_fade(EVENT_DROP) < 0.01, "dropped events are gone");
        let (mut g, mut ctx) = fresh(120, 40);
        let dts = [SIM_DT, SIM_DT, 0.25, 0.1, SIM_DT];
        let mut i = 0;
        let mut max_gen = 0;
        while g.t < ACT_COLLIDER - 0.01 {
            g.update(dts[i % dts.len()], &mut ctx);
            i += 1;
            assert!(g.tracks.len() <= g.tracks.capacity());
            assert!(g.towers.len() <= g.towers.capacity());
            let mut borns: Vec<u64> = g.tracks.iter().map(|t| t.born.to_bits()).collect();
            borns.sort_unstable();
            borns.dedup();
            assert!(borns.len() <= 2, "{} events at t = {}", borns.len(), g.t);
            max_gen = max_gen.max(borns.len());
        }
        assert_eq!(max_gen, 2, "the previous event lingers while fading");
        // One crossing every EVENT_PERIOD from FIRST_EVENT.
        let want = ((ACT_COLLIDER - 0.01 - FIRST_EVENT) / EVENT_PERIOD).floor() as u32 + 1;
        assert_eq!(g.events, want);
    }

    #[test]
    fn acc_a8_chamber_photographs_show_beams_stars_pairs_and_delta_rays() {
        let (mut g, mut ctx) = fresh(120, 40);
        let (mut pairs, mut deltas) = (0, 0);
        for _ in 0..20 {
            g.photograph(&mut ctx);
            let tracks: Vec<BTrack> = g.chamber.iter().copied().collect();
            let beams: Vec<&BTrack> = tracks.iter().filter(|t| t.kind == BKind::Beam).collect();
            assert!((BEAMS.0 as usize..=BEAMS.1 as usize).contains(&beams.len()));
            let p_beam = beams[0].path.p(0.0);
            for b in &beams {
                // A negative beam from the left edge, the stiffest tracks in the frame.
                assert!(b.path.x0 < 0.0 && b.path.qb < 0.0 && b.path.k == 0.0);
                assert!(
                    tracks
                        .iter()
                        .all(|t| t.path.p(0.0) <= p_beam * (1.0 + 1e-12))
                );
            }
            for w in tracks.windows(2) {
                if w[0].kind == BKind::Pair
                    && w[1].kind == BKind::Pair
                    && (w[0].path.x0, w[0].path.y0) == (w[1].path.x0, w[1].path.y0)
                {
                    pairs += 1;
                    // e⁺ and e⁻ from one point, curling opposite ways and spiralling in over
                    // several turns, each 20–50 % tighter than the last.
                    assert!(w[0].path.qb * w[1].path.qb < 0.0);
                    for e in w {
                        let turn = TAU / e.path.qb.abs();
                        let shrink = e.path.p(turn) / e.path.p(0.0);
                        assert!((0.5..=0.8).contains(&shrink), "{shrink}");
                        assert!(e.len >= 3.0 * turn);
                    }
                }
            }
            for d in tracks.iter().filter(|t| t.kind == BKind::Delta) {
                deltas += 1;
                // Knock-on electrons: negative, soft, born on a beam track.
                assert!(d.path.qb < 0.0 && d.path.p(0.0) < 0.12 && d.path.k > 0.0);
                let on_beam = beams.iter().any(|b| {
                    (0..=400).any(|j| {
                        let (x, y) = b.path.pos(b.len * f64::from(j) / 400.0);
                        let (dx, dy) = (d.path.x0, d.path.y0);
                        (x - dx).hypot(y - dy) < 0.02
                    })
                });
                assert!(on_beam, "δ ray off the beam");
            }
            // Slower secondaries bubble more densely.
            let mut sec: Vec<&BTrack> = tracks
                .iter()
                .filter(|t| t.kind == BKind::Secondary)
                .collect();
            sec.sort_by(|a, b| a.path.p(0.0).total_cmp(&b.path.p(0.0)));
            for w in sec.windows(2) {
                assert!(w[0].dens >= w[1].dens);
            }
        }
        assert!(pairs >= 20 && deltas > 20, "{pairs} pairs, {deltas} δ rays");
        // The star outshines the beam, which is also sparser.
        assert!(BKind::Secondary.brightness() > BKind::Pair.brightness());
        assert!(BKind::Pair.brightness() > BKind::Beam.brightness());
        // The round window, rim included, fits between the stage's top and bottom rows.
        for (w, h) in [(80u16, 24u16), (120, 40), (200, 50), (60, 40)] {
            let (g, _) = fresh(w, h);
            let (_, cy, r) = g.window();
            let rows = f64::from(g.stage.height());
            assert!(cy - 1.02 * r >= 1.0 / rows - 1e-12, "{w}x{h}");
            assert!(cy + 1.02 * r <= 1.0 - 1.0 / rows + 1e-12, "{w}x{h}");
        }
    }

    #[test]
    fn acc_a9_the_art_leaves_the_title_row_to_the_title_and_readout() {
        let (mut g, mut ctx) = fresh(80, 24);
        assert_eq!(g.stage, Size::new(80, 23));
        for t in [1.0, 5.0, 12.0, 27.0, 30.0, 33.0] {
            run_to(&mut g, &mut ctx, t, &[SIM_DT, 0.25]);
            let mut c = Canvas::new(g.size, GlyphMode::Unicode);
            g.render(&mut c);
            let row = &c.to_lines()[23];
            let text = row.trim_start();
            let start = row.chars().count() - text.chars().count();
            let room = title::readout_room(g.size, &Title::of(&g));
            assert!(text.chars().count() <= room, "{row:?}");
            assert!(start > 40, "only the right-aligned readout: {row:?}");
            assert!(
                text.starts_with("event ") || text.starts_with("film frame "),
                "{row:?}"
            );
        }
        // Tiny and odd sizes never panic.
        for (w, h) in [(1, 1), (2, 1), (3, 40), (200, 2), (80, 24)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            for i in 0..200 {
                g.update(if i % 5 == 0 { 0.25 } else { SIM_DT }, &mut ctx);
            }
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut c);
            assert!(c.to_lines().iter().all(|l| l.is_ascii()));
        }
    }

    #[test]
    fn acc_a10_the_k0s_vee_conserves_momentum_and_mass() {
        let energy = |(a, b): (f64, f64)| (a * a + b * b + M_PI * M_PI).sqrt();
        for p_k in [1.5, 3.0, 6.0] {
            for i in 0..24 {
                let [plus, minus] = vee(p_k, TAU * f64::from(i) / 24.0);
                assert!(
                    (plus.0 + minus.0 - p_k).abs() < 1e-12,
                    "momentum along the flight"
                );
                assert!((plus.1 + minus.1).abs() < 1e-12, "no momentum across it");
                let e = energy(plus) + energy(minus);
                let mass = (e * e - p_k * p_k).sqrt();
                assert!((mass - M_K).abs() < 1e-9, "invariant mass {mass}");
            }
        }
        // p* = 0.206 GeV.
        let [rest, _] = vee(0.0, 0.0);
        assert!((rest.0 - 0.206).abs() < 1e-3);
        // Generated vees: daughters start where the dashed flight ends, 0.12–0.45 m out.
        let (mut g, mut ctx) = fresh(160, 48);
        let mut vees = 0;
        for _ in 0..40 {
            g.tracks.clear();
            g.towers.clear();
            g.crossing(&mut ctx);
            let tracks: Vec<CTrack> = g.tracks.iter().copied().collect();
            for (i, k) in tracks.iter().enumerate() {
                if k.flavour != Flavour::Neutral {
                    continue;
                }
                vees += 1;
                let end = k.path.pos(k.len);
                assert!((0.12 - 1e-12..=0.45 + 1e-12).contains(&end.0.hypot(end.1)));
                for d in &tracks[i + 1..i + 3] {
                    assert_eq!((d.path.x0, d.path.y0), end);
                }
                assert!(tracks[i + 1].path.qb > 0.0 && tracks[i + 2].path.qb < 0.0);
            }
        }
        assert!(vees > 10, "{vees} vees");
    }

    #[test]
    fn acc_a11_small_stages_zoom_in_on_the_tracker() {
        assert_eq!(view_radius(5.0), VIEW_R.0);
        assert_eq!(view_radius(40.0), VIEW_R.1);
        let tracker_cols = |w, h| {
            let (g, _) = fresh(w, h);
            let v = g.view();
            2.0 * ECAL.0 * v.k * ASPECT
        };
        // At 80×24 the tracker is ≥ 30 columns across; at 200×50 the full 2.6 m view.
        assert!(tracker_cols(80, 24) >= 30.0, "{}", tracker_cols(80, 24));
        let (g, _) = fresh(200, 50);
        let rows = draw::fit_radius(g.stage, VIEW_FRAC);
        assert_eq!(view_radius(rows), VIEW_R.1);
    }

    /// ACC-A13: the collider act is coloured like an event display: charged tracks by
    /// charge (positive `Ice`, negative `HAlpha`, never through the muon yoke's field flip),
    /// muons `Starlight`, towers as thin `Aurora` (ECAL) and `Plasma` (HCAL) bars, the
    /// outline `Dim`, and no `Ember` anywhere in the act.
    #[test]
    fn acc_a14_soft_loopers_are_thin_so_the_jet_axis_reads() {
        // Draw each track of a collider event alone, whole and fresh: a looper (a charged
        // hadron under SOFT_PT) lights about half the dots of the same path drawn solid,
        // and stiff tracks are drawn solid.
        let (mut g, mut ctx) = fresh(120, 40);
        run_to(&mut g, &mut ctx, 3.0, &[SIM_DT]);
        let tracks: Vec<CTrack> = g.tracks.iter().copied().collect();
        let lit = |tr: &CTrack| {
            let mut d = Dots::default();
            d.resize(g.stage);
            let t = tr.born + tr.delay + GROW;
            draw_ctrack(&mut d, g.view(), t, tr, 1.0);
            let s = d.dot_size();
            (0..i32::from(s.height()))
                .flat_map(|y| (0..i32::from(s.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| d.is_lit(x, y))
                .count()
        };
        let (mut loopers, mut stiff) = (0, 0);
        for tr in &tracks {
            if tr.flavour != Flavour::Hadron {
                continue;
            }
            let solid = CTrack {
                pt: SOFT_PT.max(tr.pt),
                ..*tr
            };
            let (a, b) = (lit(tr), lit(&solid));
            if b < 40 {
                continue;
            }
            if is_looper(tr) {
                loopers += 1;
                assert!(
                    (a as f64) < 0.7 * b as f64,
                    "looper pT {:.2}: {a} of {b}",
                    tr.pt
                );
            } else {
                stiff += 1;
                assert_eq!(a, b);
            }
        }
        assert!(loopers > 0 && stiff > 0, "{loopers} loopers, {stiff} stiff");
    }

    #[test]
    fn acc_a13_collider_inks_follow_charge_and_calorimeter() {
        let (mut g, mut ctx) = fresh(120, 40);
        let allowed = [
            Role::Ramp(POS_RAMP),
            Role::Ramp(NEG_RAMP),
            Role::Ramp(MUON_RAMP),
            Role::Ramp(ECAL_RAMP),
            Role::Ramp(HCAL_RAMP),
            Role::Ramp(GLOW_RAMP),
            OUTLINE_ROLE,
            Role::Text,
            Role::Background,
        ];
        let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
        let (mut pos, mut neg) = (0, 0);
        let mut t = 0.5;
        while t < ACT_COLLIDER - 0.5 {
            run_to(&mut g, &mut ctx, t, &[SIM_DT]);
            for tr in g.tracks.iter() {
                let (ramp, _) = fresh_tone(tr.flavour, tr.pt, tr.positive);
                match tr.flavour {
                    Flavour::Hadron | Flavour::Electron => {
                        assert_eq!(tr.positive, tr.path.qb > 0.0);
                        assert_eq!(ramp, if tr.positive { POS_RAMP } else { NEG_RAMP });
                        if tr.positive { pos += 1 } else { neg += 1 }
                    }
                    Flavour::Muon => assert_eq!(ramp, MUON_RAMP),
                    Flavour::Neutral => assert_eq!(ramp, NEUTRAL_RAMP),
                }
            }
            canvas.clear();
            g.render(&mut canvas);
            for cell in canvas.cells() {
                assert!(allowed.contains(&cell.ink.role), "{:?} at t={t}", cell.ink);
                assert!(allowed.contains(&cell.bg.role), "{:?} at t={t}", cell.bg);
            }
            t += 0.37;
        }
        assert!(pos > 20 && neg > 20, "{pos} positive, {neg} negative");
    }

    #[test]
    fn acc_a12_the_detector_is_drawn_below_every_fresh_track_even_in_mono() {
        use sciart_render::{Rgb, Theme};
        let luma = |Rgb(r, g, b): Rgb| {
            0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)
        };
        let structure = Ink::new(OUTLINE_ROLE, STRUCTURE_LEVEL as u8);
        // Every flavour's fresh ink, at its softest.
        for flavour in [
            Flavour::Hadron,
            Flavour::Electron,
            Flavour::Muon,
            Flavour::Neutral,
        ] {
            for positive in [true, false] {
                let (ramp, v) = fresh_tone(flavour, 0.0, positive);
                let track = ink::ramp(ramp, v);
                for theme in [Theme::Astro, Theme::Mono] {
                    let (s, t) = (
                        luma(theme.color(structure, 0, 0)),
                        luma(theme.color(track, 0, 0)),
                    );
                    assert!(
                        s + 10.0 < t,
                        "{theme:?} {flavour:?}: structure {s}, track {t}"
                    );
                }
            }
        }
    }
}
