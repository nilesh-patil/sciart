//! The DNA double helix turning in space, unzipped and copied by a replication fork
//! (SPEC-002 `dna_helix`).
//!
//! B-DNA in Å with the axis along world +x: two backbones wind right-handed around a
//! ladder of coloured base pairs while the duplex spins. A 24 s story then runs a
//! replication fork through it: a helicase ring unzips the ladder, the leading strand is
//! copied continuously and the lagging strand in Okazaki fragments, and two daughter
//! helices remain, each half old (grey) and half new (magenta).
//!
//! Nothing is integrated: every quantity is a closed-form function of scene time
//! ([`Story::at`] and the synthesis-time functions), so the story cannot drift or desync
//! on resize. Backbones and the helicase collar are rasterised into two braille layers
//! (far and near) at the end of `update` and `resize`; `render` only blits them around
//! the depth-sorted rungs.

use std::f64::consts::{FRAC_PI_2, TAU};

use rand::Rng;
use rand::seq::SliceRandom;
use sciart_core::ease;
use sciart_core::geom::{self, ASPECT};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "dna_helix",
    category: Category::Biology,
    description: "The DNA double helix turning in space, unzipped and copied by a replication fork",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(DnaHelix::new())
}

// ---------------------------------------------------------------------------------------
// B-DNA geometry (DNA-A1).
// ---------------------------------------------------------------------------------------

/// Base pairs per turn (Wang 1979).
const BP_PER_TURN: f64 = 10.5;
/// Twist per base pair, rad: 34.29°.
pub(crate) const TWIST: f64 = TAU / BP_PER_TURN;
/// Rise per base pair, Å.
pub(crate) const RISE: f64 = 3.4;
/// Helical pitch, Å: 35.7.
pub(crate) const PITCH: f64 = BP_PER_TURN * RISE;
/// Phosphorus backbone radius, Å (Arnott & Hukins 1972).
pub(crate) const R: f64 = 8.9;
/// Crick's backbone leads Watson's by this angle, from C1′–C1′ = 10.5 Å at r = 5.9 Å
/// (Saenger 1984); it sets the minor (12.5 Å) and major (23.2 Å) grooves.
pub(crate) const DELTA: f64 = 2.2;
/// Pairs beyond each screen edge, so the camera sway never exposes a blunt end.
const MARGIN: usize = 4;

/// World x of base pair `u` (fractional between pairs). The view keeps x = 0 at column
/// 0, which falls midway between bp `MARGIN` and the next: a rung standing exactly on
/// the left edge would read as a window border.
fn bp_x(u: f64) -> f64 {
    (u - MARGIN as f64 - 0.5) * RISE
}

// ---------------------------------------------------------------------------------------
// Sequence (DNA-A3).
// ---------------------------------------------------------------------------------------

/// Pairs laid out at most: enough for the width at any aspect ratio (a floor in
/// [`View::new`]); 512 B of sequence on the stack.
pub(crate) const N_MAX: usize = 512;
pub(crate) type Seq = [u8; N_MAX];

pub(crate) const A: u8 = 0;
pub(crate) const C: u8 = 1;
pub(crate) const G: u8 = 2;
pub(crate) const T: u8 = 3;

/// Watson–Crick complement: A↔T, C↔G (the encoding makes it `3 − b`).
pub(crate) fn comp(b: u8) -> u8 {
    3 - b
}

fn letter(b: u8) -> char {
    ['A', 'C', 'G', 'T'][usize::from(b & 3)]
}

/// Four-dye sequencing colours: A green, C blue, G yellow, T red.
fn base_role(b: u8) -> Role {
    [Role::Particle, Role::Cold, Role::Sun, Role::Hot][usize::from(b & 3)]
}

/// Fewest and most G/C positions: a realised GC fraction within [0.35, 0.65] (DNA-A3).
const GC_MIN: usize = (35 * N_MAX).div_ceil(100); // ⌈0.35·N_MAX⌉
const GC_MAX: usize = 65 * N_MAX / 100; // ⌊0.65·N_MAX⌋

/// A fresh Watson sequence: a seeded GC count, each GC position G or C and each AT
/// position A or T by a coin, then a Fisher–Yates shuffle (DNA-A3).
pub(crate) fn draw_sequence(rng: &mut impl Rng, seq: &mut Seq) {
    let gc = rng.random_range(GC_MIN..=GC_MAX);
    for (i, b) in seq.iter_mut().enumerate() {
        let coin = rng.random::<bool>();
        *b = match (i < gc, coin) {
            (true, true) => G,
            (true, false) => C,
            (false, true) => A,
            (false, false) => T,
        };
    }
    seq.shuffle(rng);
}

/// One nucleotide of a duplex and whether it is parental (grey backbone) or new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Nt {
    pub base: u8,
    pub parental: bool,
}

/// The pair at `i` in the upper daughter (Watson template + new Crick copy) and the
/// lower one (new Watson copy + Crick template), as `(watson slot, crick slot)`:
/// semiconservative replication (DNA-A3).
pub(crate) fn daughter_pairs(seq: &Seq, i: usize) -> [(Nt, Nt); 2] {
    let w = seq[i];
    let old = |base| Nt {
        base,
        parental: true,
    };
    let new = |base| Nt {
        base,
        parental: false,
    };
    // The lower daughter's new Watson base is the complement of the Crick template.
    [(old(w), new(comp(w))), (new(w), old(comp(w)))]
}

/// The intact parental pair at `i`, as `(watson, crick)`.
pub(crate) fn parental_pair(seq: &Seq, i: usize) -> (Nt, Nt) {
    let old = |base| Nt {
        base,
        parental: true,
    };
    (old(seq[i]), old(comp(seq[i])))
}

// ---------------------------------------------------------------------------------------
// Replication kinematics (DNA-A4, DNA-A5). τ is the time since `unwind` began.
// ---------------------------------------------------------------------------------------

/// Fork speed, bp/s: ≈ 6–17× slower than human forks, ≈ 300× slower than E. coli.
const V_FORK: f64 = 3.0;
/// The leading polymerase trails the helicase by this many bp.
const LEAD_LAG: f64 = 2.0;
/// Okazaki fragment length, bp (scaled down from 100–2000 nt).
const OKAZAKI: usize = 8;
/// A fragment is primed once the fork is this far past its low end.
const PRIME_AT: f64 = OKAZAKI as f64 + 1.0;
/// Lagging polymerase speed, bp/s: faster than the fork, so every fragment finishes
/// before the next is primed.
const V_POL: f64 = 8.0;
/// Ligase seals a nick this long after the fragment completes, s.
const LIGASE: f64 = 0.5;
/// Fresh nucleotides glow at full level and fade to normal over this long, s.
const FRESH: f64 = 0.4;
/// Arms open to ±`ARM·R` over `ARM_SPAN` bp behind the apex.
const ARM: f64 = 1.7;
const ARM_SPAN: f64 = 6.0;

/// Apex position in bp: `f(τ) = 3τ`.
pub(crate) fn fork(tau: f64) -> f64 {
    V_FORK * tau
}

/// Arm displacement `Y(s) = 1.7R·smoothstep(s/6)` at `s` bp behind the apex: 0 ahead of
/// it (the duplex is intact there).
pub(crate) fn arm_offset(s: f64) -> f64 {
    ARM * R * ease::smoothstep(s / ARM_SPAN)
}

/// When leading base `j` is made: once `f − 2 = j` (5′→3′ along +x).
pub(crate) fn leading_time(j: f64) -> f64 {
    (j + LEAD_LAG) / V_FORK
}

/// When Okazaki fragment `k` (bases `[8k, 8k + 8)`) is primed at its high end.
pub(crate) fn primed_time(k: usize) -> f64 {
    ((OKAZAKI * k) as f64 + PRIME_AT) / V_FORK
}

/// When the lagging strand reaches pair position `u` (base `j` at `u = j`): fragment
/// `k = ⌊u/8⌋` grows 5′→3′ toward decreasing x from its high end. The one formula for
/// synthesis, rung pairing and the freshness glow.
pub(crate) fn lagging_time(u: f64) -> f64 {
    let k = (u.max(0.0) / OKAZAKI as f64).floor();
    primed_time(k as usize) + (OKAZAKI as f64 * k + (OKAZAKI - 1) as f64 - u) / V_POL
}

/// When fragment `k` reaches its low end.
pub(crate) fn complete_time(k: usize) -> f64 {
    primed_time(k) + (OKAZAKI - 1) as f64 / V_POL
}

/// When the nick between fragment `k` and its predecessor (bp `8k − 1`/`8k`) is sealed.
pub(crate) fn sealed_time(k: usize) -> f64 {
    complete_time(k) + LIGASE
}

// ---------------------------------------------------------------------------------------
// The story (DNA-A6).
// ---------------------------------------------------------------------------------------

/// The story's beats, in timeline order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Helix,
    Unwind,
    Hold,
    Reset,
}

const PHASE_ORDER: [Phase; 4] = [Phase::Helix, Phase::Unwind, Phase::Hold, Phase::Reset];
const PHASES: [(&str, f64); 4] = [
    ("helix", 4.0),
    ("unwind", 14.0),
    ("hold", 4.0),
    ("reset", 2.0),
];

/// Length of one beat, s.
const fn secs(phase: Phase) -> f64 {
    PHASES[phase as usize].1
}

const UNWIND_S: f64 = secs(Phase::Unwind);
const RESET_HALF: f64 = secs(Phase::Reset) / 2.0;
/// Zoom during `helix`; eases to 1 over the first `DOLLY` s of `unwind`. `FIT_R` budgets
/// the open fork's ±2.7R, but the intact helix needs only ±R plus the pitch shear and
/// sway, so zoomed in it fills about two thirds of rows [1, h) and still fits them
/// (DNA-A4).
const ZOOM_IN: f64 = 2.2;
const DOLLY: f64 = 3.0;

/// The whole story state at one instant (DNA-A6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Snapshot {
    pub phase: Phase,
    /// Time since `unwind` began while a fork exists (frozen during the first half of
    /// `reset`); `None` for an intact helix.
    pub tau: Option<f64>,
    pub zoom: f64,
    /// Level multiplier: 1, except while `reset` fades out and back in.
    pub fade: f64,
    /// Sequences drawn after the first: one per `reset` midpoint passed.
    pub draws: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct Story {
    timeline: Timeline,
}

impl Story {
    pub fn new() -> Self {
        Self {
            timeline: Timeline::new(&PHASES),
        }
    }

    pub fn at(&self, t: f64) -> Snapshot {
        let beat = self.timeline.at(t);
        let cycle = self.timeline.cycle(t);
        let l = beat.local;
        let phase = PHASE_ORDER[beat.index.min(PHASE_ORDER.len() - 1)];
        let (tau, zoom, fade) = match phase {
            Phase::Helix => (None, ZOOM_IN, 1.0),
            Phase::Unwind => {
                let zoom = ease::lerp(ZOOM_IN, 1.0, ease::in_out_cubic(l / DOLLY));
                (Some(l), zoom, 1.0)
            }
            Phase::Hold => (Some(UNWIND_S + l), 1.0, 1.0),
            // Daughters fade out, then (after the new sequence) the intact helix fades in.
            Phase::Reset if l < RESET_HALF => {
                let tau = UNWIND_S + secs(Phase::Hold);
                (Some(tau), 1.0, 1.0 - l / RESET_HALF)
            }
            Phase::Reset => (None, ZOOM_IN, (l - RESET_HALF) / RESET_HALF),
        };
        let past_midpoint = phase == Phase::Reset && l >= RESET_HALF;
        Snapshot {
            phase,
            tau,
            zoom,
            fade,
            draws: cycle + i64::from(past_midpoint),
        }
    }
}

/// Spin rate that unwinds exactly as fast as the fork moves, so the apex angle is
/// fixed: `ω = 2π·v_f/10.5` = 1.7952 rad/s.
pub(crate) const SPIN: f64 = TAU * V_FORK / BP_PER_TURN;
/// Story length, s.
const LOOP: f64 = secs(Phase::Helix) + secs(Phase::Unwind) + secs(Phase::Hold) + secs(Phase::Reset);
/// Watson's angle at the apex: π/2 − Δ/2, where Watson stands highest above Crick
/// (+0.89R against −0.89R, both at z = +0.45R). Behind the apex the two then climb and
/// drop into their arms without crossing, and the helicase sits on the upper strand.
const APEX_THETA: f64 = FRAC_PI_2 - DELTA / 2.0;
/// Spin phase at loop-local time 0: `θ_apex = φ₀ − ω·4 s − 4Ω` (the helix phase and the
/// margin pairs), so `φ₀ = θ_apex + 16Ω` = 3.762 rad (mod 2π).
const PHI0: f64 = APEX_THETA + SPIN * secs(Phase::Helix) + TWIST * MARGIN as f64;

/// Watson's phase `φ = φ₀ − ω·t_loop` (mod 2π), on loop-local time that wraps at the
/// `reset` midpoint, where every level is 0: the apex angle is the same in every loop.
pub(crate) fn spin_phase(t: f64) -> f64 {
    let local = (t + RESET_HALF).rem_euclid(LOOP) - RESET_HALF;
    (PHI0 - SPIN * local).rem_euclid(TAU)
}

// ---------------------------------------------------------------------------------------
// View and camera (DNA-A2, DNA-A4).
// ---------------------------------------------------------------------------------------

/// Vertical budget in units of R: the open fork's ±2.7R, turned by the pitch, plus the
/// sway's edge depth sheared by the pitch and the near side's perspective (DNA-A4).
const FIT_R: f64 = 6.6;
const YAW_SWAY: f64 = 0.2;
const YAW_PERIOD: f64 = 19.0;
const PITCH_TILT: f64 = 0.18;
/// Smallest eye distance, Å: gentle perspective. The depth cue is carried by the
/// `z_local` levels, not by perspective.
const EYE: f64 = 40.0 * R;
/// The eye also stays this many visible half-widths away, so on wide views the screen
/// edges keep a perspective factor ≈ 1 instead of splaying.
const EYE_PER_HALF_WIDTH: f64 = 6.0;
/// Largest depth swing of the screen edges under the yaw sway, Å. It caps the sway on
/// wide views (it binds only beyond ≈ 65 Å half-widths, wider than 200×50), so the far
/// end never recedes past its margin pairs and the pitch never shears the duplex.
const SWAY_DEPTH: f64 = 13.0;
/// Largest perspective factor of any backbone point: the nearest lies within
/// `SWAY_DEPTH + R + 2.7R·sin(pitch)` ≈ 26 Å of the axis plane, and the eye ≥ 40R away.
const MAX_MAGNIFY: f64 = 1.1;
/// Rungs are drawn at least this many columns apart, so the ladder keeps its rhythm.
const RUNG_GAP: f64 = 2.0;

/// Per-size framing (rebuilt in `resize`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub w: f64,
    pub h: f64,
    /// Rows per Å at zoom 1.
    pub scale: f64,
    /// Pairs laid out, `MARGIN` beyond each edge.
    pub n: usize,
    /// World width (Å) visible at zoom 1.
    pub width: f64,
    /// Every `stride`-th rung is drawn: 1 unless pairs are under `RUNG_GAP` columns
    /// apart at zoom 1.
    pub stride: usize,
}

impl View {
    pub fn new(size: Size) -> Self {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let fit = (h - 2.0).max(1.0) / (FIT_R * R);
        // Wide views: never need more than N_MAX pairs across. On extreme aspect ratios
        // (w/h beyond ≈ 50) that floor would outgrow the height fit and push the strands
        // off rows [1, h), so it is capped there and the duplex ends short of the right
        // edge instead (DNA-A4).
        let floor = w / (ASPECT * RISE * (N_MAX - 2 * MARGIN - 1) as f64);
        let scale = fit.min(w / (ASPECT * PITCH)).max(floor.min(fit));
        let columns_per_bp = ASPECT * RISE * scale;
        // Pairs from column 0 to the right edge, counting the half-pair offset of the
        // layout ([`bp_x`]).
        let across = (w / columns_per_bp + 0.5).ceil() as usize;
        Self {
            w,
            h,
            scale,
            n: (across + 2 * MARGIN).min(N_MAX),
            width: w / (ASPECT * scale),
            stride: (RUNG_GAP / columns_per_bp).ceil().max(1.0) as usize,
        }
    }

    /// Pairs on screen: the layout minus its margins.
    pub fn visible(&self) -> std::ops::Range<usize> {
        MARGIN..self.n.saturating_sub(MARGIN)
    }
}

/// The camera at time `t` and zoom `zoom`: a gentle yaw sway, a fixed pitch, and an eye
/// far enough away that the visible width never splays in perspective.
pub(crate) fn camera(view: &View, t: f64, zoom: f64) -> Camera {
    let half = view.width / (2.0 * zoom);
    let sway = YAW_SWAY.min((SWAY_DEPTH / half).min(1.0).asin());
    Camera {
        yaw: sway * (TAU * t / YAW_PERIOD).sin(),
        pitch: PITCH_TILT,
        distance: EYE.max(EYE_PER_HALF_WIDTH * half),
    }
}

/// One of the four backbones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strand {
    /// Parental Watson (5′→3′ along +x): the upper arm, the lagging template.
    Watson,
    /// Parental Crick: the lower arm, the leading template.
    Crick,
    /// New strand on the upper arm, in Crick's angular slot (lagging, Okazaki fragments).
    Lagging,
    /// New strand on the lower arm, in Watson's slot (leading, continuous).
    Leading,
}

impl Strand {
    fn slot(self) -> f64 {
        match self {
            Strand::Watson | Strand::Leading => 0.0,
            Strand::Crick | Strand::Lagging => DELTA,
        }
    }

    fn arm(self) -> f64 {
        match self {
            Strand::Watson | Strand::Lagging => 1.0,
            Strand::Crick | Strand::Leading => -1.0,
        }
    }
}

/// One projected backbone point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sample {
    /// Cell coordinates.
    pub x: f64,
    pub y: f64,
    /// Camera-space depth (larger = nearer).
    pub depth: f64,
    /// Depth relative to the point's own duplex axis at the same x, Å.
    pub z_local: f64,
}

/// Everything needed to place a backbone point at one instant.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geo {
    proj: Projector,
    centre: (f64, f64),
    /// Rows per Å at this zoom.
    k: f64,
    /// World x the camera looks at: keeps x = 0 (between bp `MARGIN` and the next) at
    /// column 0 at any zoom.
    look_x: f64,
    phi: f64,
    /// Apex position in bp, while a fork exists.
    fork: Option<f64>,
}

impl Geo {
    pub fn new(view: &View, t: f64, snap: &Snapshot) -> Self {
        Self::with_camera(view, camera(view, t, snap.zoom), t, snap)
    }

    pub fn with_camera(view: &View, cam: Camera, t: f64, snap: &Snapshot) -> Self {
        Self {
            proj: cam.projector(),
            centre: (view.w / 2.0, view.h / 2.0),
            k: view.scale * snap.zoom,
            look_x: view.width / (2.0 * snap.zoom),
            phi: spin_phase(t),
            fork: snap.tau.map(fork),
        }
    }

    /// Duplex axis height of `strand` at pair position `u`.
    fn axis(&self, strand: Strand, u: f64) -> f64 {
        self.fork.map_or(0.0, |f| strand.arm() * arm_offset(f - u))
    }

    /// World position of `strand` at pair position `u`: `(x, y₀ + R cos θ, R sin θ)`.
    pub fn world(&self, strand: Strand, u: f64) -> [f64; 3] {
        let theta = self.phi + TWIST * (u - MARGIN as f64) + strand.slot();
        let (s, c) = theta.sin_cos();
        [bp_x(u), self.axis(strand, u) + R * c, R * s]
    }

    /// Cell coordinates and camera depth of world point `p`.
    fn project(&self, p: [f64; 3]) -> Option<(f64, f64, f64)> {
        self.proj
            .project([p[0] - self.look_x, p[1], p[2]], self.centre, self.k)
    }

    pub fn sample(&self, strand: Strand, u: f64) -> Option<Sample> {
        let p = self.world(strand, u);
        let y0 = self.axis(strand, u);
        let (x, y, depth) = self.project(p)?;
        let z_local = self.proj.rotate([0.0, p[1] - y0, p[2]])[2];
        Some(Sample {
            x,
            y,
            depth,
            z_local,
        })
    }
}

/// Backbone depth cue: 70 far … 230 near (DNA-A2).
pub(crate) fn backbone_level(z_local: f64) -> f64 {
    70.0 + 160.0 * ((z_local + R) / (2.0 * R)).clamp(0.0, 1.0)
}

/// Rung half level at its backbone end: `max(0.9 × backbone level, 110)`.
pub(crate) fn rung_level(z_local: f64) -> f64 {
    (RUNG * backbone_level(z_local)).max(RUNG_FLOOR)
}

/// A new nucleotide `age` s old: 255 when made, fading linearly to `level` over 0.4 s.
fn fresh(level: f64, age: f64) -> f64 {
    if age < FRESH {
        ease::lerp(255.0, level, (age / FRESH).max(0.0))
    } else {
        level
    }
}

/// One continuous run of backbone, in pair positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    pub strand: Strand,
    pub u0: f64,
    pub u1: f64,
}

/// When the new strand's backbone reached pair position `u` (continuous version of
/// the per-base times), for the freshness glow.
fn made_at(strand: Strand, u: f64) -> f64 {
    match strand {
        Strand::Leading => leading_time(u),
        Strand::Lagging => lagging_time(u),
        Strand::Watson | Strand::Crick => f64::NEG_INFINITY,
    }
}

/// Every backbone run that exists at `snap` over pairs `[0, n)`: the two parents, the
/// leading strand up to `f − 2`, and each primed Okazaki fragment down to its growing
/// 3′ end, joined to its predecessor once the nick is sealed.
pub(crate) fn for_each_segment(snap: &Snapshot, n: usize, mut f: impl FnMut(Segment)) {
    let last = n.saturating_sub(1) as f64;
    f(Segment {
        strand: Strand::Watson,
        u0: 0.0,
        u1: last,
    });
    f(Segment {
        strand: Strand::Crick,
        u0: 0.0,
        u1: last,
    });
    let Some(tau) = snap.tau else { return };
    let lead = (fork(tau) - LEAD_LAG).min(last);
    if lead >= 0.0 {
        f(Segment {
            strand: Strand::Leading,
            u0: 0.0,
            u1: lead,
        });
    }
    for k in 0..n.div_ceil(OKAZAKI) {
        let tk = primed_time(k);
        if tau < tk {
            break;
        }
        let (low, high) = ((OKAZAKI * k) as f64, (OKAZAKI * k + OKAZAKI - 1) as f64);
        let grown = (high - V_POL * (tau - tk)).max(low);
        let u0 = if k > 0 && tau >= sealed_time(k) {
            low - 1.0
        } else {
            grown
        };
        if u0 <= last {
            f(Segment {
                strand: Strand::Lagging,
                u0,
                u1: high.min(last),
            });
        }
    }
}

// ---------------------------------------------------------------------------------------
// Rungs and the helicase.
// ---------------------------------------------------------------------------------------

/// Rungs at this fraction of their backbone end's level, floored at `RUNG_FLOOR`, so
/// the base colours stay clear on the far side while near and far still differ.
const RUNG: f64 = 0.9;
const RUNG_FLOOR: f64 = 110.0;
/// Helicase collar radius, in R: snug around the strand, not a halo over the fork.
const RING: f64 = 0.35;
/// Points around the collar; at 200×50 its ≈ 20-dot circumference is fully joined.
const RING_SAMPLES: usize = 32;
const RING_LEVEL: f64 = 210.0;
const LABEL_LEVEL: f64 = 150.0;
const LABEL_BASE_LEVEL: f64 = 220.0;

/// A base-pair chord, split at its midpoint; `b` is `None` for a template base that is
/// not yet paired (a half-rung).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rung {
    a: (f64, f64),
    b: (f64, f64),
    ink_a: Ink,
    ink_b: Option<Ink>,
    depth: f64,
    index: u16,
}

/// The helicase: a circle of radius `RING·R` about Watson's apex point, in the plane
/// normal to the strand there, so the strand threads through it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ring {
    pub centre: [f64; 3],
    /// Orthonormal axes of the ring's plane (world).
    pub e1: [f64; 3],
    pub e2: [f64; 3],
    /// Camera depth of the centre: ring points behind it go to the far layer.
    pub depth: f64,
    pub ink: Ink,
}

/// Whether a rung chord of `(dx, dy)` cells is drawn straight down its middle column:
/// within 22.5° of vertical on screen, or shorter than a cell both ways.
fn rung_upright(dx: f64, dy: f64) -> bool {
    const TAN_22_5: f64 = 0.414_213_562_373_095;
    let (ax, ay) = (dx.abs(), dy.abs() * ASPECT);
    ax <= ay * TAN_22_5 || (dx.abs() < 1.0 && dy.abs() < 1.0)
}

/// The cell chord a rung is drawn along, or `None` for a pair seen nearly end-on at a
/// backbone crossing, where a real model hides it too: a slanted chord spanning under
/// two rows (a lone `╲`/`╱` scratch across the braille strands), over four rows, or
/// making more than one column step per two rows. Near a crossing the backbones there
/// run at the chord's own slope a column or two away, so a longer or flatter `╲`
/// staircase reads as a doubled strand or hatching, not a base pair; the upright `│`
/// comb keeps the ladder's rhythm and the rare short, steep `╲` still gives the 3-D cue.
fn rung_chord(a: (f64, f64), b: (f64, f64)) -> Option<[i32; 4]> {
    let (y0, y1) = (a.1.round() as i32, b.1.round() as i32);
    if rung_upright(b.0 - a.0, b.1 - a.1) {
        let mid = (0.5 * (a.0 + b.0)).round() as i32;
        return Some([mid, y0, mid, y1]);
    }
    let (x0, x1) = (a.0.round() as i32, b.0.round() as i32);
    let rows = (y1 - y0).abs();
    (rows >= 2 && 2 * (x1 - x0).abs() <= rows && rows <= 4).then_some([x0, y0, x1, y1])
}

/// The cells of a rung, as `(x, y, glyph, ink)`, one per row: an upright one straight
/// down its middle column, a slanted one as a Bresenham chord with `│` for a step down a
/// column and the chord's diagonal (`╲`/`╱`) for a step across, so it reads `│╲│` or a
/// `╲` staircase, never `─`; the half nearer `a` in `ink_a`, the rest in `ink_b` (only
/// if paired).
fn rung_cells(r: &Rung, mut f: impl FnMut(i32, i32, Glyph, Ink)) {
    let Some([x0, y0, x1, y1]) = rung_chord(r.a, r.b) else {
        return;
    };
    let len = (x1 - x0).abs().max((y1 - y0).abs());
    let diagonal = if (x1 > x0) == (y1 > y0) {
        Glyph::new('╲', '\\')
    } else {
        Glyph::new('╱', '/')
    };
    let mut cells = geom::line(x0, y0, x1, y1).peekable();
    let mut prev_x = None;
    let mut k = 0;
    while let Some((x, y)) = cells.next() {
        // An unpaired half stops at the chord's midpoint.
        let Some(ink) = (if 2 * k <= len { Some(r.ink_a) } else { r.ink_b }) else {
            return;
        };
        // A step down one column (or a lone cell) is `│`; any step across is diagonal.
        let upright = match prev_x {
            Some(px) => px == x,
            None => cells.peek().is_none_or(|&(nx, _)| nx == x),
        };
        f(
            x,
            y,
            if upright {
                Glyph::new('│', '|')
            } else {
                diagonal
            },
            ink,
        );
        prev_x = Some(x);
        k += 1;
    }
}

fn draw_rung(canvas: &mut Canvas, r: &Rung) {
    rung_cells(r, |x, y, glyph, ink| canvas.put(x, y, glyph, ink));
}

/// The helicase at the apex, only while the apex is on screen (no stray ring dots at
/// an edge).
pub(crate) fn helicase(geo: &Geo, view: &View, snap: &Snapshot) -> Option<Ring> {
    let f = fork(snap.tau?);
    if f >= view.n as f64 {
        return None;
    }
    let s = geo.sample(Strand::Watson, f)?;
    if !(0.0..view.w).contains(&s.x.round()) {
        return None;
    }
    let centre = geo.world(Strand::Watson, f);
    let ahead = geo.world(Strand::Watson, f + 0.01);
    let t = unit(std::array::from_fn(|k| ahead[k] - centre[k]));
    // World up made normal to the tangent, and the third axis t × e1.
    let e1 = unit(std::array::from_fn(|k| [0.0, 1.0, 0.0][k] - t[1] * t[k]));
    let e2 = cross(t, e1);
    Some(Ring {
        centre,
        e1,
        e2,
        depth: s.depth,
        ink: Ink::new(Role::Wave, level(RING_LEVEL * snap.fade)),
    })
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / n)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// An ink level from a real value, rounded and clamped to `[0, 255]`.
fn level(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Sample one backbone run into the far (`z_local < 0`) or near braille layer, joining
/// consecutive samples with lines. The `τ = 0` fallback is inert: without a fork only
/// the parents exist, and a parent's freshness age `τ − (−∞)` is infinite at any τ.
fn trace(far: &mut Dots, near: &mut Dots, geo: &Geo, snap: &Snapshot, seg: Segment, per_bp: f64) {
    let steps = ((seg.u1 - seg.u0) * per_bp).ceil().max(0.0) as usize;
    let role = match seg.strand {
        Strand::Watson | Strand::Crick => Role::Structure,
        Strand::Lagging | Strand::Leading => Role::Accent,
    };
    let tau = snap.tau.unwrap_or(0.0);
    let mut prev: Option<(f64, f64)> = None;
    for i in 0..=steps {
        let u = if steps == 0 {
            seg.u0
        } else {
            seg.u0 + (seg.u1 - seg.u0) * i as f64 / steps as f64
        };
        let Some(s) = geo.sample(seg.strand, u) else {
            prev = None;
            continue;
        };
        let lit = fresh(backbone_level(s.z_local), tau - made_at(seg.strand, u));
        let ink = Ink::new(role, level(lit * snap.fade));
        let (dx, dy) = to_dots(s.x, s.y);
        let layer = if s.z_local < 0.0 {
            &mut *far
        } else {
            &mut *near
        };
        match prev {
            Some((px, py)) => layer.line(px, py, dx, dy, ink),
            None => layer.plot(dx, dy, ink),
        }
        prev = Some((dx, dy));
    }
}

/// Rasterise the helicase into the braille layers: the half behind the strand point
/// into `far` (under the rungs and near backbones), the rest into `near`, so the
/// strand visibly passes through the collar.
pub(crate) fn trace_ring(far: &mut Dots, near: &mut Dots, geo: &Geo, ring: &Ring) {
    let r = RING * R;
    let mut prev: Option<(f64, f64, bool)> = None;
    for i in 0..=RING_SAMPLES {
        let (s, c) = (TAU * i as f64 / RING_SAMPLES as f64).sin_cos();
        let p = std::array::from_fn(|k| ring.centre[k] + r * (c * ring.e1[k] + s * ring.e2[k]));
        let Some((x, y, depth)) = geo.project(p) else {
            prev = None;
            continue;
        };
        let (dx, dy) = to_dots(x, y);
        let behind = depth < ring.depth;
        let layer = if behind { &mut *far } else { &mut *near };
        match prev {
            Some((px, py, was_behind)) if was_behind == behind => {
                layer.line(px, py, dx, dy, ring.ink);
            }
            _ => layer.plot(dx, dy, ring.ink),
        }
        prev = Some((dx, dy, behind));
    }
}

/// The collar's ASCII stand-in: the cells of its top and bottom points (`centre ±
/// RING·R·e1`), which `render` marks `o`. ASCII's dot-count fallback (`. : + #`) cannot
/// draw a ring, so without them the helicase would read as more backbone.
pub(crate) fn ring_marks(geo: &Geo, ring: &Ring) -> Option<[(i32, i32); 2]> {
    let point = |sign: f64| {
        let p = std::array::from_fn(|k| ring.centre[k] + sign * RING * R * ring.e1[k]);
        geo.project(p)
            .map(|(x, y, _)| (x.round() as i32, y.round() as i32))
    };
    Some([point(1.0)?, point(-1.0)?])
}

/// Dot coordinates of cell point `(x, y)`: cell centres map to the middle of their
/// 2×4 dot block.
fn to_dots(x: f64, y: f64) -> (f64, f64) {
    (
        x * f64::from(Dots::CELL_W) + 0.5,
        y * f64::from(Dots::CELL_H) + 1.5,
    )
}

// ---------------------------------------------------------------------------------------
// The scene.
// ---------------------------------------------------------------------------------------

/// Label: this many Watson bases from this width up.
const LABEL_TIERS: [(f64, usize); 2] = [(60.0, 16), (30.0, 8)];
const LABEL_MIN_H: f64 = 6.0;

/// Label window: the Watson bases from the first on-screen pair (bp `MARGIN + 1`), 5′
/// end first like the strand below it; `None` when the screen is too small. It depends
/// on the size only, so the text changes only when a new sequence is drawn at the
/// `reset` midpoint, while every level is 0: a caption, not a ticker beside the fork.
fn label_window(view: &View) -> Option<(usize, usize)> {
    let len = LABEL_TIERS.iter().find(|(min_w, _)| view.w >= *min_w)?.1;
    let start = MARGIN + 1;
    (view.h >= LABEL_MIN_H && start + len <= view.visible().end).then_some((start, len))
}

#[derive(Debug)]
pub(crate) struct DnaHelix {
    story: Story,
    t: f64,
    seq: Seq,
    /// Sequences drawn after the first (tracks `Snapshot::draws`).
    draws: i64,
    seeded: bool,
    view: View,
    far: Dots,
    near: Dots,
    /// Sorted far → near.
    rungs: Vec<Rung>,
    /// The helicase's ASCII `o` cells and ink ([`ring_marks`]), while it is drawn.
    ring_marks: Option<([(i32, i32); 2], Ink)>,
    /// First Watson base of the label and how many.
    label: Option<(usize, usize)>,
    /// The `reset` fade, which the label follows like everything else.
    fade: f64,
}

impl DnaHelix {
    pub fn new() -> Self {
        Self {
            story: Story::new(),
            t: 0.0,
            seq: [A; N_MAX],
            draws: 0,
            seeded: false,
            view: View::new(Size::new(1, 1)),
            far: Dots::default(),
            near: Dots::default(),
            rungs: Vec::new(),
            ring_marks: None,
            label: None,
            fade: 1.0,
        }
    }

    /// Rebuild the backbone layers, rungs, helicase and label for `snap` at the
    /// current time.
    fn rasterise(&mut self, snap: &Snapshot) {
        let geo = Geo::new(&self.view, self.t, snap);
        self.far.clear();
        self.near.clear();
        // Samples ≤ 4 dots apart along the arc, joined by lines: on a backbone radius
        // of ≳ 10 dots the chords' sagitta stays under a quarter dot, so the curve is
        // as smooth as the lattice allows.
        let arc_per_bp = (R * TWIST).hypot(RISE);
        let dots_per_bp = f64::from(Dots::CELL_H) * geo.k * MAX_MAGNIFY * arc_per_bp;
        let per_bp = (dots_per_bp / 4.0).ceil().clamp(2.0, 32.0);
        let (far, near) = (&mut self.far, &mut self.near);
        for_each_segment(snap, self.view.n, |seg| {
            trace(far, near, &geo, snap, seg, per_bp);
        });
        self.ring_marks = helicase(&geo, &self.view, snap).and_then(|ring| {
            trace_ring(&mut self.far, &mut self.near, &geo, &ring);
            ring_marks(&geo, &ring).map(|cells| (cells, ring.ink))
        });
        self.build_rungs(&geo, snap);
        self.fade = snap.fade;
    }

    fn build_rungs(&mut self, geo: &Geo, snap: &Snapshot) {
        self.rungs.clear();
        for i in (0..self.view.n).step_by(self.view.stride) {
            let u = i as f64;
            // A fork (and so τ) exists from `unwind` to the `reset` midpoint.
            let tau = match (geo.fork, snap.tau) {
                (Some(f), Some(tau)) if u < f => tau,
                _ => {
                    let (w, c) = parental_pair(&self.seq, i);
                    // Parental bases never glow, so τ is not read.
                    let ends = [(Strand::Watson, w), (Strand::Crick, c)];
                    self.push_rung(geo, snap, 0.0, i, ends, true);
                    continue;
                }
            };
            let [(w_old, c_new), (w_new, c_old)] = daughter_pairs(&self.seq, i);
            let lagging_made = tau >= lagging_time(u);
            let leading_made = tau >= leading_time(u);
            let upper = [(Strand::Watson, w_old), (Strand::Lagging, c_new)];
            self.push_rung(geo, snap, tau, i, upper, lagging_made);
            let lower = [(Strand::Crick, c_old), (Strand::Leading, w_new)];
            self.push_rung(geo, snap, tau, i, lower, leading_made);
        }
        self.rungs
            .sort_unstable_by(|a, b| a.depth.total_cmp(&b.depth).then(a.index.cmp(&b.index)));
    }

    /// Add the chord at pair `i` between two strands; the second half only if `paired`.
    /// `tau` (time since `unwind` began) dates the glow of new bases.
    fn push_rung(
        &mut self,
        geo: &Geo,
        snap: &Snapshot,
        tau: f64,
        i: usize,
        ends: [(Strand, Nt); 2],
        paired: bool,
    ) {
        let u = i as f64;
        let [(sa, na), (sb, nb)] = ends;
        let (Some(a), Some(b)) = (geo.sample(sa, u), geo.sample(sb, u)) else {
            return;
        };
        if rung_chord((a.x, a.y), (b.x, b.y)).is_none() {
            return;
        }
        let ink = |strand: Strand, nt: Nt, s: &Sample| {
            let base = rung_level(s.z_local);
            let lit = if nt.parental {
                base
            } else {
                fresh(base, tau - made_at(strand, u))
            };
            Ink::new(base_role(nt.base), level(lit * snap.fade))
        };
        self.rungs.push(Rung {
            a: (a.x, a.y),
            b: (b.x, b.y),
            ink_a: ink(sa, na, &a),
            ink_b: paired.then(|| ink(sb, nb, &b)),
            depth: 0.5 * (a.depth + b.depth),
            index: self.rungs.len() as u16,
        });
    }
}

impl Generator for DnaHelix {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            draw_sequence(&mut ctx.rng, &mut self.seq);
            self.seeded = true;
        }
        self.view = View::new(size);
        self.label = label_window(&self.view);
        self.far.resize(size);
        self.near.resize(size);
        // Up to two daughter rungs per pair.
        self.rungs.reserve(2 * self.view.n);
        let snap = self.story.at(self.t);
        self.rasterise(&snap);
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        self.t += dt;
        let snap = self.story.at(self.t);
        // One new sequence per reset midpoint, drawn while every level is 0.
        while self.draws < snap.draws {
            draw_sequence(&mut ctx.rng, &mut self.seq);
            self.draws += 1;
        }
        self.rasterise(&snap);
    }

    fn render(&self, canvas: &mut Canvas) {
        self.far.blit(canvas);
        for rung in &self.rungs {
            draw_rung(canvas, rung);
        }
        self.near.blit(canvas);
        // ASCII's dot-count glyphs cannot draw the collar: mark it `o` above and below
        // the strand (SPEC-002 ASCII).
        if canvas.mode() == GlyphMode::Ascii
            && let Some((cells, ink)) = self.ring_marks
        {
            for (x, y) in cells {
                canvas.put(x, y, Glyph::ascii('o'), ink);
            }
        }
        if let Some((start, len)) = self.label {
            let text = Ink::new(Role::Text, level(LABEL_LEVEL * self.fade));
            let base = level(LABEL_BASE_LEVEL * self.fade);
            canvas.text(1, 0, "5'-", text);
            let x = 4;
            for (k, &b) in self.seq[start..start + len].iter().enumerate() {
                let ink = Ink::new(base_role(b), base);
                canvas.put(x + k as i32, 0, Glyph::ascii(letter(b)), ink);
            }
            canvas.text(x + len as i32, 0, "-3'", text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap_at(t: f64) -> Snapshot {
        Story::new().at(t)
    }

    #[test]
    fn dna_a1_b_dna_geometry() {
        assert!((TWIST.to_degrees() - 34.2857).abs() < 1e-4);
        assert_eq!(RISE, 3.4);
        assert!((PITCH - 35.7).abs() < 1e-12);
        let minor = DELTA / TAU * PITCH;
        let major = PITCH - minor;
        assert!((minor - 12.50).abs() < 0.05, "{minor}");
        assert!((major - 23.20).abs() < 0.05, "{major}");
        let chord = 2.0 * 5.9 * (DELTA / 2.0).sin();
        assert!((chord - 10.5).abs() < 0.1, "{chord}");
        // Right-handed: positive torsion of successive chords at 0.5 bp steps.
        let geo = Geo::new(&View::new(Size::new(80, 24)), 0.7, &snap_at(0.7));
        for strand in [Strand::Watson, Strand::Crick] {
            for i in 0..40 {
                let p: Vec<[f64; 3]> = (0..4)
                    .map(|k| geo.world(strand, f64::from(i) * 0.5 + f64::from(k) * 0.5))
                    .collect();
                let d = |a: [f64; 3], b: [f64; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let (a, b, c) = (d(p[0], p[1]), d(p[1], p[2]), d(p[2], p[3]));
                let cross = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                assert!(cross[0] * c[0] + cross[1] * c[1] + cross[2] * c[2] > 0.0);
            }
        }
    }

    #[test]
    fn dna_a2_projection_is_not_mirrored() {
        let view = View::new(Size::new(200, 50));
        let snap = Snapshot {
            phase: Phase::Helix,
            tau: None,
            zoom: 1.0,
            fade: 1.0,
            draws: 0,
        };
        for t in [0.0, 0.37, 1.9, 4.4] {
            // The scene's own camera at zoom 1, with the sway zeroed.
            let cam = Camera {
                yaw: 0.0,
                ..camera(&view, t, 1.0)
            };
            assert_eq!(cam.pitch, PITCH_TILT);
            let geo = Geo::with_camera(&view, cam, t, &snap);
            let mut near_segments = 0;
            for strand in [Strand::Watson, Strand::Crick] {
                let samples: Vec<Sample> = (0..400)
                    .map(|k| geo.sample(strand, f64::from(k) * 0.1).unwrap())
                    .collect();
                for w in samples.windows(2) {
                    if w[0].z_local >= 0.25 * R && w[1].z_local >= 0.25 * R {
                        assert!(w[1].x > w[0].x);
                        assert!(w[1].y > w[0].y, "near strand must slant like \\");
                        near_segments += 1;
                    }
                }
            }
            assert!(near_segments > 100);
            // At equal x, through the projection pipeline: the sample nearer the camera
            // relative to its own axis gets the higher level, and z_local has the sign
            // of its depth minus the axis's depth. Checked on the intact duplex and on
            // an open fork's upper arm (Watson template and its lagging copy).
            let open = Snapshot {
                tau: Some(8.0),
                ..snap
            };
            for (s, pair) in [
                (snap, [Strand::Watson, Strand::Crick]),
                (open, [Strand::Watson, Strand::Lagging]),
            ] {
                let geo = Geo::with_camera(&view, cam, t, &s);
                for k in 0..80 {
                    let u = f64::from(k) * 0.5;
                    let [a, b] = pair.map(|strand| {
                        let p = geo.sample(strand, u).unwrap();
                        let axis = [bp_x(u) - geo.look_x, geo.axis(strand, u), 0.0];
                        let axis_depth = geo.proj.rotate(axis)[2];
                        let rel = p.depth - axis_depth;
                        assert!((p.z_local - rel).abs() < 1e-9, "u = {u}: {p:?} vs {rel}");
                        p
                    });
                    if (a.z_local - b.z_local).abs() > 0.1 * R {
                        let (near, far) = if a.z_local > b.z_local {
                            (a, b)
                        } else {
                            (b, a)
                        };
                        assert!(
                            backbone_level(near.z_local) > backbone_level(far.z_local),
                            "u = {u}"
                        );
                        assert!(rung_level(near.z_local) >= rung_level(far.z_local));
                    }
                }
            }
            // World +y is up on screen.
            let up = geo
                .proj
                .project([0.0, 5.0, 0.0], geo.centre, geo.k)
                .unwrap();
            let down = geo
                .proj
                .project([0.0, -5.0, 0.0], geo.centre, geo.k)
                .unwrap();
            assert!(up.1 < down.1);
        }
        assert!(backbone_level(R) > backbone_level(-R));
        assert!(backbone_level(0.3) > backbone_level(-0.3));
    }

    #[test]
    fn dna_a3_watson_crick_pairing() {
        for b in [A, C, G, T] {
            assert_eq!(comp(comp(b)), b);
        }
        assert_eq!((comp(A), comp(G)), (T, C));
        let mut last: Option<Seq> = None;
        for seed in 0..64 {
            let mut rng = Ctx::new(seed, Size::new(1, 1)).rng;
            let mut seq = [A; N_MAX];
            draw_sequence(&mut rng, &mut seq);
            assert!(seq.iter().all(|&b| b <= T));
            let gc = seq.iter().filter(|&&b| b == G || b == C).count() as f64 / N_MAX as f64;
            assert!((0.35..=0.65).contains(&gc), "seed {seed}: {gc}");
            // Chargaff, over the Crick strand the rungs draw: #A = #T and #G = #C.
            let crick: Vec<u8> = (0..N_MAX).map(|i| parental_pair(&seq, i).1.base).collect();
            for i in 0..N_MAX {
                assert_eq!(crick[i], comp(seq[i]));
                assert_eq!(parental_pair(&seq, i).0.base, seq[i]);
            }
            let count = |s: &[u8], x: u8| s.iter().filter(|&&b| b == x).count();
            let duplex = |x: u8| count(&seq, x) + count(&crick, x);
            assert_eq!((duplex(A), duplex(G)), (duplex(T), duplex(C)));
            for i in 0..N_MAX {
                let [(w1, c1), (w2, c2)] = daughter_pairs(&seq, i);
                for (w, c) in [(w1, c1), (w2, c2)] {
                    assert_eq!((w.base, c.base), (seq[i], comp(seq[i])));
                    assert!(w.parental ^ c.parental, "one old strand per daughter");
                }
                assert!(w1.parental && c2.parental);
            }
            assert_ne!(last, Some(seq));
            last = Some(seq);
        }
    }

    #[test]
    fn dna_a4_fork_kinematics() {
        assert_eq!(fork(2.0), 6.0);
        assert!((SPIN - 1.795_20).abs() < 1e-5);
        let story = Story::new();
        let view = View::new(Size::new(200, 50));
        // Measured from the geometry: Watson's angle about its axis at the apex.
        let apex = |t: f64| {
            let snap = story.at(t);
            let geo = Geo::new(&view, t, &snap);
            let f = fork(snap.tau.unwrap());
            let p = geo.world(Strand::Watson, f);
            p[2].atan2(p[1] - geo.axis(Strand::Watson, f))
        };
        let a0 = apex(4.0);
        // Unwind and hold of the first and the second loop: the spin's loop-local time
        // keeps the apex angle the same in every loop, not only the first.
        for base in [4.0, 28.0] {
            for k in 0..=1080 {
                let t = base + f64::from(k) / 60.0;
                let d = (apex(t) - a0 + TAU / 2.0).rem_euclid(TAU) - TAU / 2.0;
                assert!(d.abs() < 1e-9, "t = {t}: {d}");
                let geo = Geo::new(&view, t, &story.at(t));
                let f = geo.fork.unwrap();
                for i in 0..view.n {
                    if i as f64 >= f {
                        assert_eq!(geo.axis(Strand::Watson, i as f64), 0.0);
                        assert_eq!(geo.axis(Strand::Crick, i as f64), 0.0);
                    }
                }
            }
        }
        // Watson sits at π/2 − Δ/2 at the apex, where it stands highest above Crick.
        let a = (a0 - (FRAC_PI_2 - DELTA / 2.0) + TAU / 2.0).rem_euclid(TAU) - TAU / 2.0;
        assert!(a.abs() < 1e-9, "{a}");
        assert!((PHI0.rem_euclid(TAU) - 3.762).abs() < 1e-3, "{PHI0}");
        assert_eq!(arm_offset(-1.0), 0.0);
        assert_eq!(arm_offset(0.0), 0.0);
        assert_eq!(arm_offset(6.0), ARM * R);
        assert_eq!(arm_offset(40.0), ARM * R);
        for k in 0..600 {
            assert!(arm_offset(f64::from(k + 1) * 0.01) >= arm_offset(f64::from(k) * 0.01));
        }
    }

    #[test]
    fn dna_a4_arms_split_as_a_clean_y_in_every_loop() {
        let story = Story::new();
        let view = View::new(Size::new(120, 36));
        for cycle in 0..2 {
            for tau in [2.0, 5.0, 8.0] {
                let t = 24.0 * f64::from(cycle) + secs(Phase::Helix) + tau;
                let snap = story.at(t);
                let geo = Geo::new(&view, t, &snap);
                let f = fork(snap.tau.unwrap());
                // Behind the apex Watson (upper arm) stays above Crick (lower arm): the
                // parental strands never cross on their way into the arms.
                for k in 0..=24 {
                    let s = f64::from(k) * 0.25;
                    let u = f - s;
                    let gap = geo.world(Strand::Watson, u)[1] - geo.world(Strand::Crick, u)[1];
                    assert!(
                        gap > 1.1 * R,
                        "cycle {cycle} τ = {tau} s = {s}: {}",
                        gap / R
                    );
                }
            }
        }
        // The spin wraps only at the reset midpoint, while every level is 0.
        let mut prev = spin_phase(0.0);
        for k in 1..=7200 {
            let t = f64::from(k) / 100.0;
            let phi = spin_phase(t);
            let step = (phi - prev + TAU / 2.0).rem_euclid(TAU) - TAU / 2.0;
            if (step + SPIN / 100.0).abs() > 1e-9 {
                assert!(story.at(t).fade < 0.02, "t = {t}");
            }
            prev = phi;
        }
    }

    #[test]
    fn dna_a4_open_fork_fits_the_frame() {
        let story = Story::new();
        for size in [
            Size::new(80, 24),
            Size::new(200, 50),
            Size::new(200, 10),
            Size::new(300, 12),
            Size::new(400, 8),
            Size::new(700, 8),
            Size::new(1000, 6),
            Size::new(40, 12),
        ] {
            let view = View::new(size);
            for k in 0..200 {
                let t = 24.0 * f64::from(k) / 200.0;
                let snap = story.at(t);
                let geo = Geo::new(&view, t, &snap);
                for_each_segment(&snap, view.n, |seg| {
                    for i in 0..=((seg.u1 - seg.u0) * 20.0) as usize {
                        let u = seg.u0 + f64::from(i as u32) * 0.05;
                        let s = geo.sample(seg.strand, u).unwrap();
                        // Off-screen columns (the 4-bp margins) are never drawn.
                        if !(0.0..view.w).contains(&s.x.round()) {
                            continue;
                        }
                        assert!(
                            s.y.round() >= 1.0 && s.y.round() < view.h,
                            "{size:?} t = {t} {seg:?} u = {u}: row {}",
                            s.y
                        );
                    }
                });
                // The laid-out pairs run past both edges: no blunt end on screen, except
                // on views so wide and short (w/h beyond ≈ 50) that N_MAX pairs at the
                // height's scale end short of the right edge; the fit above wins there.
                let last = (view.n - 1) as f64;
                let span = (N_MAX - 2 * MARGIN - 1) as f64 * ASPECT * RISE * view.scale;
                for strand in [Strand::Watson, Strand::Crick] {
                    let first = geo.sample(strand, 0.0).unwrap();
                    let end = geo.sample(strand, last).unwrap();
                    assert!(first.x < -0.5, "{size:?} t = {t}: left end at {}", first.x);
                    assert!(
                        span < view.w || end.x > view.w - 0.5,
                        "{size:?} t = {t}: right end at {}",
                        end.x
                    );
                }
                // The perspective bound the backbone sampling density relies on.
                let d = camera(&view, t, snap.zoom).distance;
                for u in (0..view.n).map(|i| i as f64) {
                    for strand in [Strand::Watson, Strand::Crick, Strand::Lagging] {
                        let depth = geo.sample(strand, u).unwrap().depth;
                        assert!(d / (d - depth) <= MAX_MAGNIFY, "{size:?} t = {t}");
                    }
                }
                // The helicase is drawn only around an on-screen apex.
                if let Some(ring) = helicase(&geo, &view, &snap) {
                    let (x, _, _) = geo.project(ring.centre).unwrap();
                    assert!((0.0..view.w).contains(&x.round()), "{size:?} t = {t}");
                }
            }
        }
    }

    #[test]
    fn dna_a4_opening_helix_fills_the_frame() {
        // The `helix` opening is zoomed in on the intact duplex, which needs only ±R (plus
        // the pitch shear and sway), not the open fork's budget: it fills about 70 % of
        // rows [1, h). The fit to rows [1, h) itself is `dna_a4_open_fork_fits_the_frame`.
        let story = Story::new();
        for size in [Size::new(80, 24), Size::new(200, 50), Size::new(120, 36)] {
            let view = View::new(size);
            for k in 0..40 {
                let t = 0.1 * f64::from(k);
                let snap = story.at(t);
                assert_eq!(snap.phase, Phase::Helix);
                let geo = Geo::new(&view, t, &snap);
                let (mut top, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY);
                for strand in [Strand::Watson, Strand::Crick] {
                    for i in 0..=(view.n - 1) * 20 {
                        let s = geo.sample(strand, f64::from(i as u32) * 0.05).unwrap();
                        if (0.0..view.w).contains(&s.x.round()) {
                            top = top.min(s.y);
                            bottom = bottom.max(s.y);
                        }
                    }
                }
                let fill = (bottom - top) / (view.h - 1.0);
                assert!(fill >= 0.62, "{size:?} t = {t}: fills {fill:.2}");
            }
        }
    }

    /// The scene at time `t` (rasterised), for rung checks.
    fn scene_at(size: Size, t: f64) -> DnaHelix {
        let mut scene = DnaHelix::new();
        scene.resize(size, &mut Ctx::new(42, size));
        scene.t = t;
        let snap = scene.story.at(t);
        scene.rasterise(&snap);
        scene
    }

    #[test]
    fn dna_a2_rungs_never_run_diagonals_along_a_row() {
        for size in [Size::new(80, 24), Size::new(200, 50), Size::new(200, 10)] {
            for k in 0..200 {
                let t = 24.0 * (f64::from(k) + 0.5) / 200.0;
                let scene = scene_at(size, t);
                assert!(!scene.rungs.is_empty());
                for rung in &scene.rungs {
                    // A slanted rung spans at least two rows: the 1-row diagonals at
                    // backbone crossings are end-on pairs and hidden.
                    let [x0, y0, x1, y1] = rung_chord(rung.a, rung.b).expect("culled rung");
                    assert!(
                        x0 == x1 || (y1 - y0).abs() >= 2,
                        "{size:?} t = {t}: {rung:?}"
                    );
                    // A slanted rung is steep and short: at most one column step per two
                    // rows, over at most four rows, so it never shadows a braille backbone
                    // of the same slope as a doubled strand.
                    let rows = (y1 - y0).abs();
                    assert!(
                        x0 == x1 || (2 * (x1 - x0).abs() <= rows && rows <= 4),
                        "{size:?} t = {t}: {rung:?}"
                    );
                    let mut rows = Vec::new();
                    rung_cells(rung, |x, y, glyph, _| {
                        // No flat strokes: an end-on pair is hidden, not drawn as `─`.
                        assert!(
                            matches!(glyph.unicode, '│' | '╲' | '╱'),
                            "{size:?} t = {t}: {glyph:?}"
                        );
                        // One cell per row: side-by-side `╱╱` never join into a stroke.
                        assert!(
                            !rows.contains(&y),
                            "{size:?} t = {t}: two cells on row {y} ({x})"
                        );
                        rows.push(y);
                    });
                }
            }
        }
    }

    #[test]
    fn dna_a2_column_0_falls_between_pairs() {
        // The landing frames (helix, zoomed in) never show an upright rung in column 0,
        // which would read as a window border.
        for size in [Size::new(80, 24), Size::new(200, 50), Size::new(120, 36)] {
            for k in 0..40 {
                let scene = scene_at(size, 0.1 * f64::from(k));
                for rung in &scene.rungs {
                    rung_cells(rung, |x, y, glyph, _| {
                        assert!(x != 0 || glyph.unicode != '│', "{size:?} k = {k} row {y}");
                    });
                }
            }
        }
        let view = View::new(Size::new(80, 24));
        let geo = Geo::new(&view, 0.0, &snap_at(0.0));
        let col = |u: f64| geo.sample(Strand::Watson, u).unwrap().x;
        assert!(col(MARGIN as f64) < -0.5 && col(MARGIN as f64 + 1.0) > 0.5);
    }

    #[test]
    fn dna_a6_label_is_static_until_the_reset_midpoint() {
        for size in [Size::new(80, 24), Size::new(200, 50), Size::new(40, 12)] {
            let mut ctx = Ctx::new(42, size);
            let mut scene = DnaHelix::new();
            scene.resize(size, &mut ctx);
            let (start, len) = scene.label.expect("label fits");
            let text = |s: &DnaHelix| s.seq[start..start + len].to_vec();
            let mut prev = text(&scene);
            for _ in 0..(50 * 60) {
                scene.update(SIM_DT, &mut ctx);
                assert_eq!(scene.label, Some((start, len)), "{size:?}");
                let now = text(&scene);
                if now != prev {
                    assert!(scene.fade < 0.02, "{size:?} t = {}", scene.t);
                }
                prev = now;
                // It starts at the first on-screen pair.
                let geo = Geo::new(&scene.view, scene.t, &scene.story.at(scene.t));
                let x = geo.sample(Strand::Watson, start as f64).unwrap().x;
                assert!(x > -0.5, "{size:?} t = {}: {x}", scene.t);
            }
        }
    }

    #[test]
    fn dna_a2_helicase_is_a_collar_threaded_on_the_upper_strand() {
        for size in [Size::new(80, 24), Size::new(120, 36), Size::new(200, 50)] {
            let view = View::new(size);
            for t in [6.0, 9.0, 12.0] {
                let snap = snap_at(t);
                let geo = Geo::new(&view, t, &snap);
                let ring = helicase(&geo, &view, &snap).expect("apex on screen");
                let f = fork(snap.tau.unwrap());
                // Centred on Watson at the apex, in the plane normal to its tangent.
                let w = geo.world(Strand::Watson, f);
                let tangent = sub(geo.world(Strand::Watson, f + 0.01), w);
                assert!(dot(ring.e1, tangent).abs() < 1e-3 * norm(tangent));
                assert!(dot(ring.e2, tangent).abs() < 1e-3 * norm(tangent));
                assert!((norm(ring.e1) - 1.0).abs() < 1e-12);
                assert!(dot(ring.e1, ring.e2).abs() < 1e-12);
                assert_eq!(ring.centre, w);
                // The strand passes through it: part of the ring lies behind the strand
                // point (far layer) and part in front (near layer).
                let (mut far, mut near) = (Dots::new(size), Dots::new(size));
                trace_ring(&mut far, &mut near, &geo, &ring);
                let lit = |d: &Dots| {
                    let s = d.dot_size();
                    (0..i32::from(s.width()))
                        .flat_map(|x| (0..i32::from(s.height())).map(move |y| (x, y)))
                        .filter(|&(x, y)| d.is_lit(x, y))
                        .count()
                };
                let (nf, nn) = (lit(&far), lit(&near));
                assert!(nf >= 4 && nn >= 4, "{size:?} t = {t}: far {nf} near {nn}");
                // A collar, not a halo: 0.35 R.
                assert!((RING - 0.35).abs() < 1e-12);
            }
        }
    }

    /// Cells of `canvas` showing `ch`.
    fn cells_with(canvas: &Canvas, ch: char) -> Vec<(i32, i32)> {
        let lines = canvas.to_lines();
        let mut out = Vec::new();
        for (y, line) in lines.iter().enumerate() {
            for (x, c) in line.chars().enumerate() {
                if c == ch {
                    out.push((x as i32, y as i32));
                }
            }
        }
        out
    }

    #[test]
    fn dna_a2_helicase_reads_as_an_o_ring_in_ascii() {
        for size in [Size::new(80, 24), Size::new(120, 36), Size::new(200, 50)] {
            for t in [6.0, 9.0, 12.0, 20.0] {
                let scene = scene_at(size, t);
                let snap = scene.story.at(t);
                let geo = Geo::new(&scene.view, t, &snap);
                let f = fork(snap.tau.unwrap());
                let mut ascii = Canvas::new(size, GlyphMode::Ascii);
                scene.render(&mut ascii);
                let rings = cells_with(&ascii, 'o');
                if helicase(&geo, &scene.view, &snap).is_none() {
                    // The apex has left the screen (`hold`): no ring.
                    assert!(rings.is_empty(), "{size:?} t = {t}: {rings:?}");
                    continue;
                }
                // Clamped on the upper strand at the apex: within the collar's reach.
                let apex = geo.sample(Strand::Watson, f).unwrap();
                let reach = RING * R * geo.k * MAX_MAGNIFY + 1.0;
                assert!(!rings.is_empty(), "{size:?} t = {t}");
                for (x, y) in rings {
                    assert!(
                        (f64::from(x) - apex.x).abs() <= reach,
                        "{size:?} t = {t}: {x}"
                    );
                    assert!(
                        (f64::from(y) - apex.y).abs() <= reach,
                        "{size:?} t = {t}: {y}"
                    );
                }
                // Unicode shows the braille collar instead.
                let mut unicode = Canvas::new(size, GlyphMode::Unicode);
                scene.render(&mut unicode);
                assert!(cells_with(&unicode, 'o').is_empty(), "{size:?} t = {t}");
            }
            // No fork, no helicase.
            let mut ascii = Canvas::new(size, GlyphMode::Ascii);
            scene_at(size, 2.0).render(&mut ascii);
            assert!(cells_with(&ascii, 'o').is_empty(), "{size:?}");
        }
    }

    fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|k| a[k] - b[k])
    }

    fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn norm(a: [f64; 3]) -> f64 {
        dot(a, a).sqrt()
    }

    #[test]
    fn dna_a2_ladder_keeps_two_columns_between_rungs() {
        for size in [Size::new(40, 12), Size::new(80, 24), Size::new(200, 10)] {
            let view = View::new(size);
            let gap = view.stride as f64 * ASPECT * RISE * view.scale;
            assert!(gap >= RUNG_GAP, "{size:?}: {gap}");
            // No thinning where the pairs are already spaced out.
            assert!(view.stride == 1 || gap < 2.0 * RUNG_GAP, "{size:?}");
        }
        assert_eq!(View::new(Size::new(80, 24)).stride, 1);
        assert_eq!(View::new(Size::new(40, 12)).stride, 2);
    }

    #[test]
    fn dna_a5_synthesis_is_5_to_3_only() {
        for j in 0..100 {
            let (a, b) = (leading_time(f64::from(j)), leading_time(f64::from(j + 1)));
            assert!(b > a);
            // Never within 2 bp of the apex.
            assert!(fork(a) - f64::from(j) >= 2.0 - 1e-12);
        }
        for k in 0..12 {
            let low = OKAZAKI * k;
            assert!(fork(primed_time(k)) >= (low + OKAZAKI + 1) as f64 - 1e-12);
            let lag = |j: usize| lagging_time(j as f64);
            for j in low..low + OKAZAKI - 1 {
                assert!(lag(j) > lag(j + 1), "grows toward −x");
            }
            assert!(lag(low + OKAZAKI - 1) >= primed_time(k));
            assert_eq!(lag(low), complete_time(k));
            assert!(complete_time(k) < primed_time(k + 1));
            assert!((sealed_time(k) - complete_time(k) - 0.5).abs() < 1e-12);
        }
        // Before reset every visible base of both daughters is made and every visible
        // nick sealed.
        let tau_end = UNWIND_S + secs(Phase::Hold);
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let view = View::new(size);
            for i in view.visible() {
                assert!(leading_time(i as f64) <= tau_end, "{size:?} {i}");
                assert!(lagging_time(i as f64) <= tau_end, "{size:?} {i}");
                if i % OKAZAKI == 0 && i > 0 {
                    assert!(sealed_time(i / OKAZAKI) <= tau_end, "{size:?} nick {i}");
                }
            }
        }
        // The drawn lagging backbone shows each nick until ligase seals it: no run spans
        // bp 8k − 1 → 8k before `sealed_time(k)`, and fragment k reaches back to 8k − 1
        // after.
        for k in 1..12 {
            let nick = (OKAZAKI * k) as f64;
            for (tau, sealed) in [
                (sealed_time(k) - 1e-6, false),
                (sealed_time(k) + 1e-6, true),
            ] {
                let snap = Snapshot {
                    phase: Phase::Unwind,
                    tau: Some(tau),
                    zoom: 1.0,
                    fade: 1.0,
                    draws: 0,
                };
                let mut lagging = Vec::new();
                for_each_segment(&snap, N_MAX, |seg| {
                    if seg.strand == Strand::Lagging {
                        lagging.push(seg);
                    }
                });
                let spans = lagging.iter().any(|s| s.u0 <= nick - 1.0 && s.u1 >= nick);
                assert_eq!(spans, sealed, "k = {k} τ = {tau}: {lagging:?}");
                let joined = lagging.iter().any(|s| s.u0 == nick - 1.0);
                assert_eq!(joined, sealed, "k = {k} τ = {tau}: {lagging:?}");
                // Fragment k itself is complete either way (sealing follows completion).
                assert!(lagging.iter().any(|s| s.u0 <= nick && s.u1 == nick + 7.0));
            }
        }
    }

    #[test]
    fn dna_a6_story_is_a_pure_function_of_time() {
        let story = Story::new();
        // The timeline: helix 4 → unwind 14 → hold 4 → reset 2, looping at 24 s.
        for (t, phase) in [
            (0.0, Phase::Helix),
            (4.0, Phase::Unwind),
            (18.0, Phase::Hold),
            (22.0, Phase::Reset),
            (24.0, Phase::Helix),
        ] {
            assert_eq!(story.at(t + 1e-9).phase, phase, "t = {t}");
        }
        // The zoom jumps only while the fade is 0.
        let mut prev = story.at(0.0);
        for k in 1..=4800 {
            let t = f64::from(k) / 100.0;
            let s = story.at(t);
            if (s.zoom - prev.zoom).abs() > 0.05 {
                assert!(s.fade.min(prev.fade) < 0.02, "t = {t}");
            }
            prev = s;
        }
        // Stepping at 1/60 s or at irregular dt gives the same state and RNG position.
        let size = Size::new(80, 24);
        let run = |dts: &mut dyn Iterator<Item = f64>| {
            let mut ctx = Ctx::new(42, size);
            let mut scene = DnaHelix::new();
            scene.resize(size, &mut ctx);
            for dt in dts {
                scene.update(dt, &mut ctx);
            }
            (scene, ctx)
        };
        let steps = 60 * 61 + 17; // past two reset midpoints
        let (a, ctx_a) = run(&mut std::iter::repeat_n(SIM_DT, steps));
        let mut rng = Ctx::new(9, size).rng;
        let mut t = 0.0;
        let mut dts = Vec::new();
        while t < a.t - 0.25 {
            let dt = rng.random_range(0.0..=0.25);
            dts.push(dt);
            t += dt;
        }
        dts.push(a.t - t);
        let (b, ctx_b) = run(&mut dts.into_iter());
        assert!((a.t - b.t).abs() < 1e-9);
        let (sa, sb) = (story.at(a.t), story.at(b.t));
        assert_eq!((sa.phase, sa.draws), (sb.phase, sb.draws));
        assert!((sa.tau.unwrap() - sb.tau.unwrap()).abs() < 1e-9);
        assert!((sa.zoom - sb.zoom).abs() < 1e-9 && (sa.fade - sb.fade).abs() < 1e-9);
        let tau = sa.tau.unwrap();
        let made = |tau: f64| {
            (0..N_MAX)
                .map(|j| (tau >= leading_time(j as f64), tau >= lagging_time(j as f64)))
                .collect::<Vec<_>>()
        };
        assert_eq!(made(tau), made(sb.tau.unwrap()));
        assert_eq!(a.seq, b.seq);
        assert_eq!(a.draws, 2);
        assert_eq!(ctx_a.rng, ctx_b.rng);

        // Exactly one new sequence per cycle, at the reset midpoint, and it differs.
        let mut ctx = Ctx::new(42, size);
        let mut scene = DnaHelix::new();
        scene.resize(size, &mut ctx);
        let mut seqs = vec![scene.seq];
        for step in 1..=(72 * 60) {
            scene.update(SIM_DT, &mut ctx);
            if scene.seq != *seqs.last().unwrap() {
                let at = story.at(scene.t);
                assert_eq!(at.phase, Phase::Reset);
                assert!(at.fade < 0.02, "drawn while dark (step {step})");
                seqs.push(scene.seq);
            }
        }
        assert_eq!(seqs.len(), 4);
        // Resizing keeps the time and the sequence.
        let before = (scene.t, scene.seq);
        scene.resize(Size::new(200, 50), &mut ctx);
        assert_eq!(before, (scene.t, scene.seq));
    }
}
