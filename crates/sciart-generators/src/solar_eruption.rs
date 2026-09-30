//! The Sun's limb in extreme ultraviolet: a loop arcade, a rising flux rope, a flare and
//! a coronal mass ejection (SPEC-002 `solar_eruption`).
//!
//! The corona is a 2.5-D potential field over a flat photosphere, drawn onto a gently
//! curved limb: four photospheric sources (line "charges" below the surface) make the
//! arcade, and a flux rope is a line current with its image below the surface (which
//! keeps the field line-tied). Field lines are traced with RK2 from footpoints and cached;
//! at most `RETRACE` of them are re-traced per frame. As the rope rises (a torus-instability
//! takeoff, `h ∝ e^{aτ²}`) an X-point opens beneath it; field lines closing under the
//! X-point are the freshly reconnected post-flare loops, hot then cooling, and their
//! footpoints — the flare ribbons — move apart as the X-point climbs. The rope leaves as a
//! three-part coronal mass ejection (bright front, dark cavity, bright core), and cooled
//! plasma rains back down the new loops.
//!
//! Units: lengths in `L` (≈ 50 Mm), simulated seconds, hours of evolution compressed.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::hash01;
use sciart_core::{Tone, ease, ink};

use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "solar_eruption",
    category: Category::SunPlanets,
    description: "Loops rise and snap: a flux rope erupts, flare ribbons flash, a CME escapes",
    title: "Solar eruption · loops snap",
    captions: &[
        "quiet arcade",
        "loops rise",
        "flare",
        "mass ejection",
        "coronal rain",
    ],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SolarEruption::new())
}

const ACTS: [(&str, f64); 5] = [
    ("quiet arcade", 6.0),
    ("loops rise", 8.0),
    ("flare", 6.0),
    ("mass ejection", 10.0),
    ("coronal rain", 12.0),
];
const QUIET: usize = 0;
const RISE: usize = 1;
const FLARE: usize = 2;
const CME: usize = 3;
const RAIN: usize = 4;

/// Photospheric sources: `(x, depth, strength)`.
const SOURCES: [(f64, f64, f64); 4] = [
    (-0.42, 0.28, 1.0),
    (0.42, 0.28, -1.0),
    (-1.15, 0.35, 0.55),
    (1.15, 0.35, -0.55),
];
/// Rope current (negative: its field opposes the arcade's beneath it) and its starting
/// height; the takeoff `h = H0·e^{a τ²}` over the rise and flare.
const ROPE_I: f64 = -1.6;
const H0: f64 = 0.3;
const TAKEOFF: f64 = 0.0125;
/// Radius of the limb's curvature in `L` (a gentle arc on screen).
const LIMB_R: f64 = 4.0;
/// Field lines: arcade footpoints, rope lines, and the per-frame re-trace budget
/// (ERUP-A5).
const ARCADE: usize = 32;
const ROPE_LINES: usize = 10;
pub(crate) const LINES: usize = ARCADE + ROPE_LINES;
pub(crate) const RETRACE: usize = 20;
const MAX_PTS: usize = 180;
const DS: f64 = 0.025;
/// Domain of the traced field.
const X_MAX: f64 = 2.6;
const Y_MAX: f64 = 3.5;
/// Rain blobs.
const BLOBS: usize = 200;
/// Post-flare loops cool over this many seconds.
const COOL: f64 = 9.0;
/// ADR-0017 (M14; AIA's green was false colour): the hot loops glow in EUV, which has no
/// visible colour, so they are a neutral `Ramp(Starlight)` up to `LOOP_TOP`; the cool
/// plasma that shines in H-alpha (the prominence rope and its core, coronal rain, the flare
/// ribbons) is `Ramp(HAlpha)` up to `HA_TOP`, the scene's one accent. A toned glow `g` is
/// drawn at `top·smoothstep(g/EDGE)·(BODY + (1 − BODY)·g)`: a thin stroke's rim fades in
/// from black within a pixel and its core stays legible (GEN-R21).
/// The CME front is white light (Thomson-scattered), a dim `Ramp(Starlight)` shell up to
/// `FRONT_TOP`. The disc is black with a thin `HAlpha` chromosphere at the limb.
const LOOP_TOP: f64 = 235.0;
/// Exposure of the loop glow: the thin (3 × 3) strokes gather less light per pixel than
/// the old wide ones.
const LOOP_GAIN: f64 = 1.5;
/// H-alpha material (critic round 2): the rope's strands at most `HA_TOP`, only the
/// densest core (toned above `CORE_FROM`) rising to `HA_PEAK`, so the rope is no longer
/// the largest saturated fill.
const HA_TOP: f64 = 150.0;
const HA_PEAK: f64 = 210.0;
const CORE_FROM: f64 = 0.85;
const EDGE: f64 = 0.35;
const BODY: f64 = 0.5;

/// The level of toned glow `g` (0–1) in a ramp whose top is `top`.
fn stroke_level(g: f64, top: f64) -> f64 {
    top * ease::smoothstep(g / EDGE) * (BODY + (1.0 - BODY) * g)
}
const FRONT_TOP: f64 = 120.0;
const RIM_LEVEL: f64 = 55.0;
/// Rim width (pixels) of the limb's chromosphere line.
const RIM_PX: f64 = 0.8;

/// Point in the corona: `x` along the limb, `y` height, in `L`.
type P = [f64; 2];

/// The field of a 2-D line source of `q` at `(x0, y0)`.
#[inline]
fn source(p: P, x0: f64, y0: f64, q: f64) -> P {
    let (dx, dy) = (p[0] - x0, p[1] - y0);
    let r2 = (dx * dx + dy * dy).max(1e-6);
    [q * dx / r2, q * dy / r2]
}

/// The field of a line current `i` at `(x0, y0)` (counter-clockwise for `i > 0`).
#[inline]
fn current(p: P, x0: f64, y0: f64, i: f64) -> P {
    let (dx, dy) = (p[0] - x0, p[1] - y0);
    let r2 = (dx * dx + dy * dy).max(1e-4);
    [-i * dy / r2, i * dx / r2]
}

/// The coronal field with the rope at height `h` carrying `i`.
pub(crate) fn field(p: P, h: f64, i: f64) -> P {
    let mut b = [0.0, 0.0];
    for &(x0, d, q) in &SOURCES {
        let s = source(p, x0, -d, q);
        b[0] += s[0];
        b[1] += s[1];
    }
    if i != 0.0 {
        let r = current(p, 0.0, h, i);
        let m = current(p, 0.0, -h, -i);
        b[0] += r[0] + m[0];
        b[1] += r[1] + m[1];
    }
    b
}

fn unit(b: P) -> P {
    let n = b[0].hypot(b[1]);
    if n > 1e-12 {
        [b[0] / n, b[1] / n]
    } else {
        [0.0, 0.0]
    }
}

/// Rope height at time `tau` seconds after the rise began: monotonic, with an increasing
/// log-slope `2aτ` (ERUP-A2).
pub(crate) fn rope_height(tau: f64) -> f64 {
    H0 * (TAKEOFF * tau.max(0.0) * tau.max(0.0)).min(30.0).exp()
}

/// A traced field line.
#[derive(Debug, Clone)]
struct Line {
    pts: [[f32; 2]; MAX_PTS],
    len: usize,
    /// Apex height, footpoints (`x` where it starts and ends on the surface, NaN if it
    /// leaves the domain) and whether it closes on itself (a rope line).
    apex: f64,
    foot: (f64, f64),
    closed: bool,
    /// Seconds since the X-point last passed over it (∞: never reconnected).
    age: f64,
    /// Relative brightness of this strand.
    gain: f32,
}

impl Default for Line {
    fn default() -> Self {
        Self {
            pts: [[0.0; 2]; MAX_PTS],
            len: 0,
            apex: 0.0,
            foot: (f64::NAN, f64::NAN),
            closed: false,
            age: f64::INFINITY,
            gain: 1.0,
        }
    }
}

/// Trace from `start` along `sign·B` with RK2 steps of `DS`, writing into `line`.
fn trace(line: &mut Line, start: P, sign: f64, h: f64, i: f64, rope_seed: bool) {
    let mut p = start;
    line.len = 0;
    line.apex = start[1];
    line.foot = (start[0], f64::NAN);
    line.closed = false;
    for k in 0..MAX_PTS {
        line.pts[k] = [p[0] as f32, p[1] as f32];
        line.len = k + 1;
        line.apex = line.apex.max(p[1]);
        let d1 = unit(field(p, h, i));
        let mid = [
            p[0] + 0.5 * DS * sign * d1[0],
            p[1] + 0.5 * DS * sign * d1[1],
        ];
        let d2 = unit(field(mid, h, i));
        let next = [p[0] + DS * sign * d2[0], p[1] + DS * sign * d2[1]];
        if d2 == [0.0, 0.0] {
            break;
        }
        if next[1] < 0.0 {
            // Land exactly on the surface.
            let f = p[1] / (p[1] - next[1]);
            let end = [p[0] + f * (next[0] - p[0]), 0.0];
            if line.len < MAX_PTS {
                line.pts[line.len] = [end[0] as f32, 0.0];
                line.len += 1;
            }
            line.foot.1 = end[0];
            return;
        }
        if next[0].abs() > X_MAX || next[1] > Y_MAX {
            return;
        }
        if rope_seed && k > 20 && (next[0] - start[0]).hypot(next[1] - start[1]) < 0.7 * DS {
            line.closed = true;
            line.pts[line.len.min(MAX_PTS - 1)] = [start[0] as f32, start[1] as f32];
            line.len = (line.len + 1).min(MAX_PTS);
            return;
        }
        p = next;
    }
}

/// How an arcade loop sits on screen (ERUP-A6). The traced field is a 2.5-D slice, so
/// every line lies in one plane and a naive drawing is a nest of concentric arches. A real
/// active region is a 3-D arcade seen at the limb: loops lean out of the vertical
/// (`shear`), stand at different places along the neutral line (`shift`), differ in
/// height (`stretch`), and many are rooted behind the limb so their feet are occulted
/// (`drop`). Each line gets a fixed pose from its index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    shear: f64,
    drop: f64,
    shift: f64,
    stretch: f64,
}

pub(crate) fn loop_pose(k: usize) -> Pose {
    let h = |salt: u64| hash01(0x9053_1A7E ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt);
    let shift = 1.2 * (h(3) - 0.5);
    Pose {
        shear: 0.6 * (h(1) - 0.5),
        // The neutral line curves away from the viewer toward its ends, so loops rooted far
        // along it stand further behind the limb.
        drop: 0.02 + 0.08 * h(2) + 0.45 * shift * shift,
        shift,
        stretch: 0.62 + 0.6 * h(4),
    }
}

/// Whether arcade loop `k` glows in the quiet corona (ERUP-A6): about half do, so the
/// arcade is a few loops with dark corona between them; the rest light up only when
/// reconnection heats them.
pub(crate) fn quiet_lit(k: usize) -> bool {
    hash01(0x0717_E5B2 ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) < 0.45
}

/// Rope strands drawn at view scale `scale` (pixels per `L`): two at small sizes, where
/// more would merge into a blob (ERUP-A7).
pub(crate) fn strands_for(scale: f64) -> usize {
    if scale < 26.0 { 2 } else { STRANDS }
}

impl Pose {
    /// Corona point `p` as drawn, with the pose applied at strength `m ∈ [0, 1]` (a hot
    /// post-flare loop is drawn upright on its ribbons, `m = 0`, and relaxes into its pose
    /// as it cools).
    fn apply(&self, m: f64, p: P) -> P {
        [
            p[0] + m * (self.shift + self.shear * p[1]),
            p[1] * (1.0 + m * (self.stretch - 1.0)) - m * self.drop,
        ]
    }
}

/// Brightness along loop `k` at arc length `s` (`L`) and time `t`: knots of denser plasma
/// a few tenths of `L` long that drift along the loop (siphon flows), so no loop is a
/// uniform band (ERUP-A6).
pub(crate) fn along(k: usize, s: f64, t: f64) -> f64 {
    let n = value_noise(0xA10E ^ k as u64, s / 0.28 - 0.35 * t, 0.5, 1.0);
    0.22 + 1.1 * n * n
}

/// Twisted strands in the flux rope, turns of twist along it, and its footpoints `|x|`.
pub(crate) const STRANDS: usize = 4;
const TWIST: f64 = 2.5;
const ROPE_FOOT: f64 = 0.8;
/// `sin θ + SHEAR·sin 2θ` peaks at this value (θ ≈ 61°), used to put the apex at `hr`.
const ROPE_SHEAR: f64 = 0.45;
const ROPE_PEAK: f64 = 1.256_6;

/// Axis of the flux rope with apex height `hr`, at `u ∈ [0, 1]` from one footpoint to the
/// other: a sheared arch (one shoulder high, the other low, the apex leaning over), the
/// limb view of a sigmoid's S (ERUP-A7).
pub(crate) fn rope_axis(u: f64, hr: f64) -> P {
    let th = std::f64::consts::PI * u.clamp(0.0, 1.0);
    let (s, c) = th.sin_cos_det();
    let s2 = (2.0 * th).sin_det();
    let y = (hr * (s + ROPE_SHEAR * s2) / ROPE_PEAK).max(0.0);
    [-ROPE_FOOT * c + 0.4 * ROPE_FOOT * s2, y]
}

/// Strand `j` at `u` and time `t`: its offset from the axis in units of the rope radius
/// (it winds `TWIST` turns round the axis and slowly rotates as the rope untwists
/// upward), and how much it faces the viewer (`0` behind, `1` in front).
pub(crate) fn strand(u: f64, j: usize, t: f64) -> (f64, f64) {
    let a = TAU * (TWIST * u + j as f64 / STRANDS as f64) + 0.9 * t;
    let (s, c) = a.sin_cos_det();
    (c, 0.5 + 0.5 * s)
}

/// Rope brightness through the story: faint while it forms, brightening as it rises,
/// brightest in the flare, fading as the CME carries it off the frame
/// (ERUP-A7), gone by 60 % of the act so no stretched legs are left in the frame.
pub(crate) fn rope_gain(act: usize, p: f64) -> f64 {
    match act {
        QUIET => 0.18 * ease::smoothstep(p),
        RISE => 0.18 + 0.72 * ease::smoothstep(p),
        FLARE => 0.9 + 0.1 * ease::smoothstep(p / 0.3),
        CME => 1.0 - ease::smoothstep((p / 0.6).min(1.0)),
        _ => 0.0,
    }
}

/// CME bubble radius over its centre height (self-similar expansion).
pub(crate) const CME_ASPECT: f64 = 0.55;

/// The CME bubble `(centre height, radius)` in `L`: it swells out of the rope during the
/// flare and accelerates off the top of the frame through the CME act (ERUP-A8).
pub(crate) fn bubble(act: usize, p: f64) -> Option<(f64, f64)> {
    let hc = match act {
        FLARE => 0.78 + 0.27 * ease::smoothstep(p),
        CME => 1.05 * (1.75 * p).exp(),
        _ => return None,
    };
    Some((hc, CME_ASPECT * hc))
}

/// The bright core — the erupting rope — rides low in the cavity.
pub(crate) fn core_height(hc: f64) -> f64 {
    hc - 0.25 * CME_ASPECT * hc
}

/// Brightness of the CME front at normalised distance `d` from the bubble centre: a
/// shell of piled-up coronal plasma with a crisp outer rim that fades softly inward into
/// the cavity (a white-light "lightbulb", not a ring).
pub(crate) fn front(d: f64) -> f64 {
    let x = (d - 1.0) / if d < 1.0 { 0.24 } else { 0.09 };
    (-x * x).exp()
}

/// The cavity's dimming of whatever lies inside the bubble (low-density flux rope).
pub(crate) fn cavity(d: f64) -> f64 {
    if d >= 1.0 {
        1.0
    } else {
        0.12 + 0.88 * ease::smoothstep(((d - 0.72) / 0.26).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Clone, Copy)]
struct Blob {
    line: usize,
    /// Index position along the line's points (fractional) and speed, points/s.
    s: f64,
    speed: f64,
    /// −1 towards the start, +1 towards the end.
    dir: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct View {
    /// Screen position of the Sun's centre, pixels per `L`.
    cx: f64,
    cy: f64,
    scale: f64,
}

impl View {
    /// Pixel of coronal point `(x, y)`: `x` becomes an angle round the limb.
    fn px(&self, x: f64, y: f64) -> (f64, f64) {
        let (s, c) = (x / LIMB_R).sin_cos_det();
        let r = (LIMB_R + y) * self.scale;
        (self.cx + r * s, self.cy - r * c)
    }
}

#[derive(Debug)]
pub struct SolarEruption {
    t: f64,
    timeline: Timeline,
    lines: Vec<Line>,
    /// Next line to re-trace (round robin).
    cursor: usize,
    blobs: Vec<Blob>,
    view: View,
    glow: Glow,
    cme: Glow,
    tone: Tone,
    cme_tone: Tone,
    pixels: Pixels,
    /// Per pixel: the corona above the limb (≥ 0) or minus the disc level (< 0).
    sky: Vec<f32>,
    /// The static ink of each pixel where nothing is drawn over the disc or corona.
    sky_ink: Vec<Ink>,
    /// Heights of the rope and the X-point; the ribbons' footpoint `x`.
    h: f64,
    current: f64,
    xpoint: Option<f64>,
    ribbon: f64,
    ribbon_seen: f64,
    /// Streaks along the CME front, indexed by the front normal's `x` component.
    front_tex: [f32; 64],
    size: Size,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl SolarEruption {
    fn new() -> Self {
        let mut lines = Vec::with_capacity(LINES);
        lines.resize_with(LINES, Line::default);
        let mut g = Self {
            t: 0.0,
            timeline: Timeline::new(&ACTS),
            lines,
            cursor: 0,
            blobs: Vec::with_capacity(BLOBS),
            view: View::default(),
            glow: Glow::default(),
            cme: Glow::default(),
            tone: Tone::asinh(0.35, 4.0),
            cme_tone: Tone::asinh(0.5, 5.0),
            pixels: Pixels::default(),
            sky: Vec::new(),
            sky_ink: Vec::new(),
            h: H0,
            current: 0.0,
            xpoint: None,
            ribbon: 0.0,
            ribbon_seen: 0.0,
            front_tex: std::array::from_fn(|k| {
                value_noise(0xF207, k as f64 / 1.6, 0.5, 1.0) as f32
            }),
            size: Size::new(1, 1),
            dirty: false,
        };
        for (k, line) in g.lines.iter_mut().enumerate() {
            // A few bright loops over many faint ones, as in any EUV image of an arcade.
            let u = hash01(0x10E ^ k as u64) as f32;
            line.gain = 0.35 + 1.0 * u * u * u;
        }
        g.drive();
        for k in 0..LINES {
            g.retrace(k);
        }
        g
    }

    /// Rope height and current for the present act.
    fn drive(&mut self) {
        let beat = self.timeline.at(self.t);
        let since_rise = match beat.index {
            RISE => beat.local,
            FLARE => ACTS[RISE].1 + beat.local,
            CME => ACTS[RISE].1 + ACTS[FLARE].1 + beat.local,
            _ => 0.0,
        };
        self.h = rope_height(since_rise);
        self.current = match beat.index {
            // The rope forms slowly during the quiet act (flux cancellation).
            QUIET => ROPE_I * ease::smoothstep(beat.progress),
            CME => ROPE_I * (1.0 - ease::smoothstep(beat.progress)),
            RAIN => 0.0,
            _ => ROPE_I,
        };
        self.xpoint = self.find_xpoint();
    }

    /// The X-point under the rope: where `B_x` on the axis changes sign below it.
    fn find_xpoint(&self) -> Option<f64> {
        if self.current == 0.0 {
            return None;
        }
        let bx = |y: f64| field([0.0, y], self.h, self.current)[0];
        let (mut lo, mut hi) = (0.02, 0.97 * self.h);
        if !(bx(lo).is_finite() && bx(lo) * bx(hi) < 0.0) {
            return None;
        }
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if bx(lo) * bx(mid) <= 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Some(0.5 * (lo + hi))
    }

    /// Footpoint of arcade line `k` (positive polarity, left of the inversion line), or
    /// the seed of rope line `k`.
    fn seed(&self, k: usize) -> (P, bool) {
        if k < ARCADE {
            let u = (k as f64 + 0.5) / ARCADE as f64;
            ([-0.02 - 1.55 * u * u, 0.0], false)
        } else {
            let j = (k - ARCADE) as f64 + 1.0;
            ([0.0, self.h + 0.035 * j * (1.0 + 0.3 * self.h)], true)
        }
    }

    fn retrace(&mut self, k: usize) {
        let (start, rope) = self.seed(k);
        let (h, i) = (self.h, self.current);
        let line = &mut self.lines[k];
        if rope && i.abs() < 0.3 * ROPE_I.abs() {
            line.len = 0;
            return;
        }
        trace(line, start, 1.0, h, i, rope);
        if rope && !line.closed {
            // Only closed strands belong to the rope; an open one is arcade flux.
            line.len = 0;
        }
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        self.t += dt;
        self.drive();
        for _ in 0..RETRACE {
            let k = self.cursor;
            self.cursor = (self.cursor + 1) % LINES;
            self.retrace(k);
        }
        // Reconnection: a line closing below the X-point with footpoints inside the
        // ribbons has just been reconnected.
        self.ribbon = self.ribbon_footpoint();
        let act = self.timeline.at(self.t).index;
        for line in &mut self.lines {
            line.age += dt;
            let fresh = matches!(act, FLARE | CME)
                && !line.closed
                && line.foot.1.is_finite()
                && line.foot.0.abs() <= self.ribbon
                && line.foot.0.abs() >= self.ribbon - 0.12;
            if fresh {
                line.age = 0.0;
            }
            if act == QUIET && line.age > 3.0 * COOL {
                line.age = f64::INFINITY;
            }
        }
        if matches!(act, FLARE | CME) {
            self.ribbon_seen = self.ribbon;
        }
        self.rain(dt, act, rng);
    }

    /// `|x|` of the flare ribbons: footpoint of the separatrix through the X-point.
    fn ribbon_footpoint(&self) -> f64 {
        let Some(y) = self.xpoint else {
            return 0.0;
        };
        let mut line = Line::default();
        // Just below the X-point, follow −B down to the positive-polarity surface.
        trace(
            &mut line,
            [0.0, 0.98 * y],
            -1.0,
            self.h,
            self.current,
            false,
        );
        if line.foot.1.is_finite() {
            line.foot.1.abs()
        } else {
            0.0
        }
    }

    fn rain(&mut self, dt: f64, act: usize, rng: &mut impl Rng) {
        // Condensations form near the tops of cooled post-flare loops and slide down.
        // Rain pours off the cooled post-flare loops; a light drizzle falls from the
        // quiet arcade too (thermal non-equilibrium keeps it going between flares).
        let rate = match act {
            RAIN => 25.0,
            QUIET => 5.0,
            _ => 0.0,
        };
        if rate > 0.0 && self.blobs.len() < BLOBS && rng.random_bool((dt * rate).min(1.0)) {
            let loops = self.lines.iter().enumerate().filter(|&(k, l)| {
                ((act == QUIET && quiet_lit(k % ARCADE)) || (l.age.is_finite() && l.age > 2.0))
                    && l.len > 10
                    && !l.closed
            });
            let n = loops.clone().count();
            if n > 0 {
                let pick = rng.random_range(0..n);
                if let Some((k, l)) = loops.clone().nth(pick) {
                    let top = (0..l.len)
                        .max_by(|&a, &b| l.pts[a][1].total_cmp(&l.pts[b][1]))
                        .unwrap_or(0);
                    // Start just off the apex on a random side, heading downhill.
                    let dir = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                    self.blobs.push(Blob {
                        line: k,
                        s: top as f64 + dir * rng.random_range(0.5..3.0),
                        speed: 0.0,
                        dir,
                    });
                }
            }
        }
        let lines = &self.lines;
        self.blobs.retain_mut(|b| {
            let l = &lines[b.line];
            let k = (b.s.max(0.0) as usize).min(l.len.saturating_sub(2));
            if l.len < 3 {
                return false;
            }
            // Gravity's component along the loop pulls the blob down (ERUP-A4).
            let (a, c) = (l.pts[k], l.pts[k + 1]);
            let mut dy = f64::from(c[1] - a[1]) * b.dir;
            if dy > 0.0 {
                // Uphill this way (the blob sits just across the apex): turn round.
                b.dir = -b.dir;
                dy = -dy;
            }
            let slope = -dy / DS;
            b.speed = (b.speed + 60.0 * slope.max(0.05) * dt).min(40.0);
            b.s += b.dir * b.speed * dt;
            b.s > 0.0 && b.s < (l.len - 1) as f64
        });
        if act != RAIN && act != QUIET {
            self.blobs.clear();
        }
    }

    fn blob_pos(&self, b: &Blob) -> P {
        let l = &self.lines[b.line];
        let k = (b.s.max(0.0) as usize).min(l.len.saturating_sub(2));
        let f = (b.s - k as f64).clamp(0.0, 1.0) as f32;
        let (a, c) = (l.pts[k], l.pts[(k + 1).min(l.len - 1)]);
        [
            f64::from(a[0] + f * (c[0] - a[0])),
            f64::from(a[1] + f * (c[1] - a[1])),
        ]
    }

    /// Height of the rope's apex as drawn (ERUP-A7): the physical rope while it rises, then
    /// the CME's core once the bubble has formed (the rope takes off far faster than the
    /// view could follow, so the drawing follows the bubble instead).
    pub(crate) fn rope_apex(&self) -> f64 {
        let beat = self.timeline.at(self.t);
        match bubble(beat.index, beat.progress) {
            Some((hc, _)) => core_height(hc),
            None if beat.index == RAIN => 0.0,
            None => self.h,
        }
    }

    /// Stroke corona polyline `pts` (posed by `pose` at strength `m`, mirrored if `flip`)
    /// into the loop glow, weighting each segment by `w` times its knot brightness.
    #[allow(clippy::too_many_arguments)]
    fn stroke_loop(
        glow: &mut Glow,
        v: &View,
        pts: &[[f32; 2]],
        pose: Pose,
        m: f64,
        flip: f64,
        k: usize,
        t: f64,
        w: f64,
    ) {
        // 1–2 px strokes (ADR-0017): a 3 × 3 kernel at every size.
        // A line kernel (critic round 2): the neighbours across the stroke take a little
        // light, the corners none, so a loop is about a pixel thin with soft edges.
        const THIN: [(i32, i32, f32); 5] = [
            (0, 0, 1.0),
            (-1, 0, 0.15),
            (1, 0, 0.15),
            (0, -1, 0.15),
            (0, 1, 0.15),
        ];
        let (kernel, gain): (&[(i32, i32, f32)], f64) = (&THIN, 1.4);
        let at = |p: &[f32; 2]| pose.apply(m, [flip * f64::from(p[0]), f64::from(p[1])]);
        let mut prev = at(&pts[0]);
        let mut a = v.px(prev[0], prev[1]);
        for (j, p) in pts.iter().enumerate().skip(1) {
            let q = at(p);
            let b = v.px(q[0], q[1]);
            // Behind the limb (a posed loop rooted on the far side): occulted.
            if prev[1] >= 0.0 && q[1] >= 0.0 {
                let knot = along(k, j as f64 * DS, t);
                glow.stroke(
                    (2.0 * a.0, 2.0 * a.1),
                    (2.0 * b.0, 2.0 * b.1),
                    kernel,
                    (0.3 * w * gain * knot) as f32,
                );
            }
            (prev, a) = (q, b);
        }
    }

    fn compose(&mut self) {
        self.draw_glow();
        self.shade();
    }

    /// Stroke the loops, rain and rope into the glow buffers.
    fn draw_glow(&mut self) {
        let v = self.view;
        let beat = self.timeline.at(self.t);
        let t = self.t;
        // At small sizes the loops crowd into fewer pixels and pile up: thin them so the
        // arcade does not saturate.
        let dense = (v.scale / 28.0).clamp(0.6, 1.0);
        self.glow.clear();
        self.cme.clear();
        // Loops: each quiet strand heats and cools on its own cycle, as active-region loops
        // do; reconnected loops are hot, then cooling. Wide soft strokes let neighbouring
        // strands merge into a glowing arcade instead of beating against the pixel grid
        // (moiré) at small sizes. The rope's own closed field lines are not drawn: the
        // twisted strands below stand for them.
        for (k, line) in self.lines.iter().enumerate() {
            if line.len < 2 || line.closed {
                continue;
            }
            let hot = if line.age.is_finite() {
                (-line.age / COOL).exp()
            } else {
                0.0
            };
            // Unlit quiet loops are skipped outright until reconnection heats them.
            let lit = if quiet_lit(k % ARCADE) { 1.3 } else { 0.0 };
            if lit == 0.0 && hot < 0.03 {
                continue;
            }
            let phase = TAU * hash01(0xF11C ^ k as u64);
            let flicker = 0.75 + 0.35 * (0.9 * t + phase).sin_det();
            let w = dense * (f64::from(line.gain) * (0.7 * flicker * lit + 1.6 * hot) + 0.8 * hot);
            let pts = &line.pts[..line.len];
            let m = 1.0 - 0.55 * hot;
            Self::stroke_loop(&mut self.glow, &v, pts, loop_pose(k), m, 1.0, k, t, w);
            // The sources are mirror-antisymmetric about the inversion line, so the field
            // is mirror-symmetric: a line that does not close back on the far polarity
            // (it leaves the domain) has a twin on the other side, drawn by reflection
            // (with a pose of its own).
            if !line.foot.1.is_finite() {
                let pose = loop_pose(k + LINES);
                Self::stroke_loop(&mut self.glow, &v, pts, pose, m, -1.0, k + LINES, t, w);
            }
        }
        // Flare ribbons: two smooth bright lines along the limb at the post-flare loops'
        // feet, antialiased by the glow kernel, brightest at their inner edge (the newest
        // reconnection) and fading outward.
        let flash = match beat.index {
            FLARE => 1.0,
            CME => (1.0 - beat.progress).max(0.25),
            RAIN => 0.25 * (1.0 - beat.progress),
            _ => 0.0,
        };
        if flash > 0.0 && self.ribbon_seen > 0.0 {
            const RIBBON: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
            const N: i32 = 24;
            for side in [-1.0, 1.0] {
                for k in 0..=N {
                    let u = f64::from(k) / f64::from(N);
                    let x = side * (self.ribbon_seen - 0.14 + 0.2 * u);
                    let (px, py) = v.px(x, 0.012);
                    let taper = (std::f64::consts::PI * u).sin_det() * (1.0 - 0.5 * u);
                    self.cme
                        .splat(2.0 * px, 2.0 * py, &RIBBON, (0.9 * flash * taper) as f32);
                }
            }
        }
        // Coronal rain: bright knots sliding down the loops they condensed on.
        const RAIN_KERNEL: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
        for b in &self.blobs {
            let p = self.blob_pos(b);
            let l = &self.lines[b.line];
            let hot = if l.age.is_finite() {
                (-l.age / COOL).exp()
            } else {
                0.0
            };
            let q = loop_pose(b.line).apply(1.0 - 0.55 * hot, p);
            if q[1] >= 0.0 {
                let (px, py) = v.px(q[0], q[1]);
                // Cool, dense: H-alpha, with the rope (the `cme` buffer).
                self.cme.splat(2.0 * px, 2.0 * py, &RAIN_KERNEL, 0.9);
            }
        }
        // The flux rope: twisted strands round a sheared arch, rising and brightening; in
        // the CME it is the bright core low in the cavity, its legs still rooted.
        let gain = rope_gain(beat.index, beat.progress);
        let hr = self.rope_apex();
        if gain > 0.0 && hr > 0.0 {
            const ROPE_KERNEL: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
            const N: usize = 90;
            let radius = (0.045 + 0.08 * hr).min(0.22);
            for j in 0..strands_for(v.scale) {
                let mut prev: Option<(f64, f64)> = None;
                for i in 0..=N {
                    let u = i as f64 / N as f64;
                    let a = rope_axis(u, hr);
                    let b = rope_axis(u + 1e-3, hr);
                    let (tx, ty) = (b[0] - a[0], b[1] - a[1]);
                    let n = tx.hypot(ty).max(1e-12);
                    let (nx, ny) = (-ty / n, tx / n);
                    let swell = (std::f64::consts::PI * u).sin_det();
                    let (off, face) = strand(u, j, t);
                    let r = radius * (0.3 + 0.7 * swell) * off;
                    let (x, y) = (a[0] + r * nx, (a[1] + r * ny).max(0.0));
                    let q = v.px(x, y);
                    if let Some(p) = prev {
                        let w = gain * (0.12 + 0.88 * face * face) * (0.4 + 0.6 * swell);
                        self.cme.stroke(
                            (2.0 * p.0, 2.0 * p.1),
                            (2.0 * q.0, 2.0 * q.1),
                            &ROPE_KERNEL,
                            (0.9 * w) as f32,
                        );
                    }
                    prev = Some(q);
                }
            }
            // The core: dense prominence plasma cradled in the rope's dips, the CME's
            // bright third part.
            let core = match beat.index {
                FLARE => ease::smoothstep(beat.progress),
                CME => 1.0 - 0.4 * beat.progress,
                _ => 0.0,
            };
            if core > 0.0 {
                const CORE_KERNEL: [(i32, i32, f32); 49] = halo_kernel::<49>(3);
                const CORE_SMALL: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
                let kernel: &[(i32, i32, f32)] = if v.scale < 26.0 {
                    &CORE_SMALL
                } else {
                    &CORE_KERNEL
                };
                for i in 0..12 {
                    let a = rope_axis(0.28 + 0.03 * f64::from(i), hr);
                    let (px, py) = v.px(a[0], a[1]);
                    self.cme.splat(
                        2.0 * px,
                        2.0 * py,
                        kernel,
                        (1.6 * dense * gain * core) as f32,
                    );
                }
            }
        }
    }

    /// Tone-map the glow buffers over the disc and corona, add the CME front, ribbons.
    fn shade(&mut self) {
        let v = self.view;
        let beat = self.timeline.at(self.t);
        // CME: bright front, dark cavity (the rope's core drawn above), expanding
        // self-similarly off the top of the frame.
        let shell = bubble(beat.index, beat.progress).map(|(hc, r)| {
            let amp = match beat.index {
                FLARE => ease::smoothstep(((beat.progress - 0.15) / 0.6).clamp(0.0, 1.0)),
                // Gone by 55 % of the act: the front leaves as a whole, with no flanks
                // left standing at the frame's top edge.
                _ => 1.0 - ease::smoothstep((beat.progress / 0.55).min(1.0)),
            };
            let (cx, cy) = v.px(0.0, hc);
            (cx, cy, r * v.scale, amp)
        });
        let tone = &self.tone;
        let cme_tone = &self.cme_tone;
        let size = self.pixels.pixel_size();
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let (glow, cme) = (self.glow.values(), self.cme.values());
        let tex = &self.front_tex;
        let (sky, sky_ink) = (&self.sky, &self.sky_ink);
        // The bubble's bounding box (where the front and cavity can reach), in pixels.
        let (bx0, bx1, by0, by1) = shell.map_or((0.0, -1.0, 0.0, -1.0), |(cx, cy, r, _)| {
            (cx - 1.6 * r, cx + 1.6 * r, cy - 1.6 * r, cy + 1.6 * r)
        });
        for y in 0..h {
            let fy = f64::from(y) + 0.5;
            let row_in = fy >= by0 && fy <= by1;
            for x in 0..w {
                let i = (y * w + x) as usize;
                let fx = f64::from(x) + 0.5;
                let inside = row_in && fx >= bx0 && fx <= bx1;
                let (g, c) = (glow[i], cme[i]);
                // Most of the frame is empty corona or disc: its ink is static.
                if !inside && g <= 0.0 && c <= 0.0 {
                    self.pixels.set(x, y, sky_ink[i]);
                    continue;
                }
                // `sky`: the corona's brightness above the limb (≥ 0), or minus the disc's
                // ramp value on it (< 0), cached per pixel in `resize`.
                let base = f64::from(sky[i]);
                // Inside the bubble: dim the loops and corona (cavity), add the front.
                let (mut dim, mut lit) = (1.0, 0.0);
                if let Some((cx, cy, r, amp)) = shell
                    && inside
                    && base >= 0.0
                {
                    let (ex, ey) = (fx - cx, fy - cy);
                    let dn = ex.hypot(ey);
                    let d = dn / r;
                    if d < 1.6 {
                        dim = 1.0 - amp * (1.0 - cavity(d));
                        // Brightest on top, where the front piles up plasma, fading down
                        // the flanks; streaky along the front (a table in `ux`).
                        let dn = dn.max(1e-9);
                        let up = (-ey / dn).max(0.0);
                        let ux = ((ex / dn + 1.0) * 31.5).clamp(0.0, 63.0);
                        let k = (ux as usize).min(62);
                        let f = ux - k as f64;
                        let streak = f64::from(tex[k]) * (1.0 - f) + f64::from(tex[k + 1]) * f;
                        let below = (ey / dn).max(0.0);
                        // The flanks fade faster as the bubble grows (they thin out and
                        // leave the frame last).
                        let spread = 1.6
                            + if beat.index == CME {
                                3.0 * beat.progress
                            } else {
                                0.0
                            };
                        let flank = (0.04 + 0.96 * up.powf(spread)) * (1.0 - 2.0 * below).max(0.0);
                        // Filaments: narrow radial streaks of piled-up plasma, the front's
                        // brightness modulated strongly along it.
                        let fil = streak * streak;
                        lit = amp * front(d) * flank * (0.25 + 1.8 * fil);
                    }
                }
                // Loops (EUV, neutral), the H-alpha material and the front (white light):
                // the brightest owns the pixel.
                let (loops, cool, white) = if g > 0.0 || c > 0.0 || lit > 0.0 {
                    (
                        tone.level((LOOP_GAIN * f64::from(g) * dim) as f32),
                        cme_tone.level(c),
                        cme_tone.level((4.0 * lit) as f32),
                    )
                } else {
                    (0, 0, 0)
                };
                let gc = f64::from(cool) / 255.0;
                let ha = stroke_level(gc, HA_TOP)
                    + (HA_PEAK - HA_TOP) * ease::smoothstep((gc - CORE_FROM) / (1.0 - CORE_FROM));
                let euv = stroke_level(f64::from(loops) / 255.0, LOOP_TOP);
                let front = FRONT_TOP * f64::from(white) / 255.0;
                let ink = if cool >= loops && cool >= white && ha >= ink::MIN_VISIBLE_LEVEL {
                    Ink::new(Role::Ramp(Ramp::HAlpha), ink::level(ha))
                } else if loops >= white && euv >= ink::MIN_VISIBLE_LEVEL {
                    Ink::new(Role::Ramp(Ramp::Starlight), ink::level(euv))
                } else if front >= ink::MIN_VISIBLE_LEVEL {
                    Ink::new(Role::Ramp(Ramp::Starlight), ink::level(front))
                } else {
                    // The black disc with its chromosphere rim; the corona is not drawn.
                    sky_ink[i]
                };
                self.pixels.set(x, y, ink);
            }
        }
    }
}

impl Generator for SolarEruption {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.glow.resize(size);
        self.cme.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let scale = (w / 4.2).min(h / 1.9).max(0.5);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.84 * h + LIMB_R * scale,
            scale,
        };
        // The static background: the disc's mottled moss and the corona's fall-off.
        let v = self.view;
        let limb_r = LIMB_R * v.scale;
        let (pw, ph) = (i32::from(px.width()), i32::from(px.height()));
        self.sky.clear();
        self.sky.reserve(px.area());
        self.sky_ink.clear();
        self.sky_ink.reserve(px.area());
        for y in 0..ph {
            for x in 0..pw {
                let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let above = (fx - v.cx).hypot(fy - v.cy) - limb_r;
                // `sky` keeps its sign convention (< 0 on the disc) for the cavity test.
                let val = if above < 0.0 { -1.0 } else { 0.0 };
                self.sky.push(val as f32);
                // The chromosphere: a line about a pixel wide just inside the limb, fading
                // smoothly on both sides.
                let rim = (1.0 - (above + 0.5 * RIM_PX).abs() / RIM_PX).max(0.0);
                let level = RIM_LEVEL * rim * rim * (3.0 - 2.0 * rim);
                self.sky_ink.push(if level >= ink::MIN_VISIBLE_LEVEL {
                    Ink::new(Role::Ramp(Ramp::HAlpha), ink::level(level))
                } else {
                    Ink::BLANK
                });
            }
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

    fn caption(&self) -> Option<&'static str> {
        let beat = self.timeline.at(self.t);
        Some(beat.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    #[test]
    fn erup_a1_traced_lines_follow_the_field() {
        let mut line = Line::default();
        for (start, h, i) in [
            ([-0.5, 0.0], H0, ROPE_I),
            ([-1.0, 0.0], 1.2, ROPE_I),
            ([-0.3, 0.0], H0, 0.0),
        ] {
            trace(&mut line, start, 1.0, h, i, false);
            assert!(line.len > 10);
            for k in 0..line.len - 2 {
                let (a, b) = (line.pts[k], line.pts[k + 1]);
                let (tx, ty) = (f64::from(b[0] - a[0]), f64::from(b[1] - a[1]));
                let mid = [f64::from(a[0] + b[0]) * 0.5, f64::from(a[1] + b[1]) * 0.5];
                let bf = field(mid, h, i);
                let cos = (tx * bf[0] + ty * bf[1]) / (tx.hypot(ty) * bf[0].hypot(bf[1]));
                assert!(
                    cos > (2.0f64).to_radians().cos_det(),
                    "{}°",
                    cos.acos().to_degrees()
                );
            }
        }
    }

    #[test]
    fn erup_a2_the_rope_takes_off() {
        let (mut last_h, mut last_slope) = (0.0, 0.0);
        for k in 0..=80 {
            let tau = f64::from(k) * 0.1;
            let h = rope_height(tau);
            assert!(h >= last_h);
            let slope = (rope_height(tau + 0.05).ln() - rope_height(tau).ln()) / 0.05;
            assert!(slope >= last_slope - 1e-12, "log-slope fell at {tau}");
            (last_h, last_slope) = (h, slope);
        }
        assert!(rope_height(8.0) > 1.5 * H0);
    }

    #[test]
    fn erup_a3_ribbons_separate_during_the_flare() {
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(1);
        let start = ACTS[0].1 + ACTS[1].1;
        while g.t < start + 1.5 {
            g.step(SIM_DT, &mut rng);
        }
        let early = g.ribbon;
        while g.t < start + ACTS[2].1 - 0.5 {
            g.step(SIM_DT, &mut rng);
        }
        assert!(
            early > 0.0 && g.ribbon > early + 0.05,
            "{early} → {}",
            g.ribbon
        );
    }

    #[test]
    fn erup_a4_rain_falls_toward_the_surface() {
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(2);
        let rain = ACTS[..RAIN].iter().map(|a| a.1).sum::<f64>();
        while g.t < rain + 3.0 {
            g.step(SIM_DT, &mut rng);
        }
        assert!(!g.blobs.is_empty(), "no rain");
        // Follow the oldest blob (new ones are appended, so it stays first) to the ground.
        g.blobs.truncate(1);
        let first = g.blobs[0];
        let mut last = g.blob_pos(&first)[1];
        let mut steps = 0;
        while g.blobs.first().is_some_and(|b| b.line == first.line) && steps < 600 {
            g.step(SIM_DT, &mut rng);
            if let Some(b) = g.blobs.first().filter(|b| b.line == first.line) {
                let y = g.blob_pos(b)[1];
                assert!(y <= last + 1e-3, "a blob climbed: {last} → {y}");
                last = y;
            }
            steps += 1;
        }
        assert!(steps > 0 && last < 0.5, "blob stuck at {last}");
    }

    #[test]
    fn erup_a6_arcade_loops_vary_in_pose_and_along_their_length() {
        // Not a nest of concentric arches: loops lean both ways, some are rooted behind
        // the limb, their footpoints and heights differ, and a few outshine the rest.
        let poses: Vec<Pose> = (0..ARCADE).map(loop_pose).collect();
        let left = poses.iter().filter(|p| p.shear < -0.08).count();
        let right = poses.iter().filter(|p| p.shear > 0.08).count();
        assert!(
            left >= ARCADE / 5 && right >= ARCADE / 5,
            "{left} / {right}"
        );
        assert!(poses.iter().filter(|p| p.drop > 0.05).count() >= ARCADE / 4);
        let spread = |f: fn(&Pose) -> f64| {
            let v = poses.iter().map(f);
            v.clone().fold(f64::MIN, f64::max) - v.fold(f64::MAX, f64::min)
        };
        assert!(spread(|p| p.shift) > 0.3 && spread(|p| p.stretch) > 0.3);
        let g = SolarEruption::new();
        let gains = g.lines[..ARCADE].iter().map(|l| f64::from(l.gain));
        let (lo, hi) = (gains.clone().fold(9.0, f64::min), gains.fold(0.0, f64::max));
        assert!(hi > 3.0 * lo, "gain {lo} .. {hi}");
        // Brightness varies along each loop (knots), and the knots drift (flows).
        let mut varied = 0;
        for k in 0..ARCADE {
            let v: Vec<f64> = (0..60)
                .map(|j| along(k, f64::from(j) * 0.05, 0.0))
                .collect();
            let (lo, hi) = (
                v.iter().copied().fold(9.0, f64::min),
                v.iter().copied().fold(0.0, f64::max),
            );
            varied += usize::from(hi > 2.0 * lo);
        }
        assert!(varied * 4 >= ARCADE * 3, "{varied}");
        assert!((along(3, 1.0, 0.0) - along(3, 1.0, 2.0)).abs() > 1e-3);
        // A sparse quiet arcade: only some loops glow, with dark corona between them…
        let shown = (0..ARCADE).filter(|&k| quiet_lit(k)).count();
        assert!(
            (ARCADE * 3 / 10..=ARCADE * 6 / 10).contains(&shown),
            "{shown} of {ARCADE} lit"
        );
        // …rooted along a curved neutral line: loops far along it sit further behind the
        // limb than those near its middle.
        let mean_drop = |far: bool| {
            let v: Vec<f64> = poses
                .iter()
                .filter(|p| (p.shift.abs() > 0.35) == far)
                .map(|p| p.drop)
                .collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        };
        assert!(
            mean_drop(true) > mean_drop(false) + 0.08,
            "{} {}",
            mean_drop(true),
            mean_drop(false)
        );
        // Hot post-flare loops stand upright on the ribbons: the pose fades out with heat.
        let p = loop_pose(5);
        assert_eq!(p.apply(0.0, [0.4, 0.0]), [0.4, 0.0]);
    }

    #[test]
    fn erup_a7_the_rope_is_a_twisted_sheared_sigmoid_that_rises_and_brightens() {
        let hr = 0.6;
        // Rooted on the surface either side of the inversion line…
        let (a, b) = (rope_axis(0.0, hr), rope_axis(1.0, hr));
        assert!(a[1].abs() < 1e-9 && b[1].abs() < 1e-9 && a[0] < -0.5 && b[0] > 0.5);
        // …apex at `hr`, but sheared: one shoulder high, the other low, the apex off-centre.
        let top = (0..=100)
            .map(|k| rope_axis(f64::from(k) / 100.0, hr)[1])
            .fold(0.0, f64::max);
        assert!((top - hr).abs() < 0.02 * hr, "{top}");
        let (l, r) = (rope_axis(0.3, hr)[1], rope_axis(0.7, hr)[1]);
        assert!((l - r).abs() > 0.2 * hr, "{l} {r}");
        let apex = (0..=100)
            .map(|k| rope_axis(f64::from(k) / 100.0, hr))
            .max_by(|a, b| a[1].total_cmp(&b[1]))
            .expect("samples");
        assert!(apex[0].abs() > 0.1, "apex at x = {}", apex[0]);
        // Each strand winds round the axis: its offset changes sign at least four times.
        for j in 0..STRANDS {
            let mut flips = 0;
            let mut last = strand(0.0, j, 0.0).0;
            for k in 1..=200 {
                let o = strand(f64::from(k) / 200.0, j, 0.0).0;
                flips += usize::from(o * last < 0.0);
                last = o;
            }
            assert!(flips >= 4, "strand {j}: {flips}");
        }
        const { assert!(STRANDS >= 3) };
        // It rises and brightens through the rise and flare.
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(4);
        let (mut apex, mut gain) = (0.0, 0.0);
        let end = ACTS[..CME].iter().map(|a| a.1).sum::<f64>();
        while g.t < end - 0.1 {
            g.step(SIM_DT, &mut rng);
            let beat = g.timeline.at(g.t);
            if beat.index == QUIET {
                continue;
            }
            let (a, w) = (g.rope_apex(), rope_gain(beat.index, beat.progress));
            assert!(a >= apex - 1e-9 && w >= gain - 1e-9, "t {}: {a} {w}", g.t);
            (apex, gain) = (a, w);
        }
        assert!(apex > 0.8 && gain > 0.9, "{apex} {gain}");
        // Gone once the CME has carried it off: no legs left hanging in the frame.
        assert_eq!(rope_gain(CME, 0.65), 0.0);
        assert_eq!(rope_gain(RAIN, 0.0), 0.0);
        // At 80×24 only two strands, so the twist reads instead of a blob.
        assert_eq!(strands_for(20.0), 2);
        assert_eq!(strands_for(60.0), STRANDS);
    }

    #[test]
    fn erup_a8_the_cme_is_three_part_and_leaves_the_frame() {
        // Bright front, dark cavity, bright core.
        assert!(front(1.0) > 4.0 * front(0.7) && front(1.0) > 4.0 * front(1.3));
        assert!(cavity(0.5) < 0.3 && cavity(1.2) == 1.0);
        let mut last_top = 0.0;
        for k in 0..=40 {
            let p = f64::from(k) / 40.0;
            let (hc, r) = bubble(CME, p).expect("a bubble through the CME act");
            // Self-similar, and the core (the rope's apex) sits low in the cavity.
            assert!((r / hc - CME_ASPECT).abs() < 1e-9);
            let core = core_height(hc);
            assert!(core < hc && hc - core < 0.5 * r, "{core} {hc} {r}");
            assert!(hc + r > last_top);
            last_top = hc + r;
        }
        // Off the top of the frame by the act's end (the frame spans ≤ 2.2 L of height).
        assert!(last_top > 4.0, "{last_top}");
        assert!(bubble(RAIN, 0.5).is_none() && bubble(RISE, 0.5).is_none());
        // M14 palette: neutral loops and white-light front (`Starlight`), H-alpha for the
        // cool material and the chromosphere; nothing else, and nothing past its cap.
        let mut g = SolarEruption::new();
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let mid = ACTS[..CME].iter().map(|a| a.1).sum::<f64>() + 3.0;
        while g.t < mid {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut pink = 0;
        for c in canvas.cells() {
            for ink in [c.ink, c.bg] {
                match ink.role {
                    Role::Background => {}
                    Role::Ramp(Ramp::Starlight) => {
                        assert!(f64::from(ink.level) <= LOOP_TOP + 1.0, "{ink:?}");
                    }
                    Role::Ramp(Ramp::HAlpha) => {
                        assert!(f64::from(ink.level) <= HA_PEAK + 1.0, "{ink:?}");
                        pink += 1;
                    }
                    _ => panic!("{ink:?}"),
                }
            }
        }
        assert!(pink > 0, "the rope's core shows in H-alpha");
    }

    #[test]
    fn erup_a10_the_disc_is_black_under_a_thin_chromosphere() {
        // ADR-0017: no chromosphere fill. Every pixel more than 2 px inside the limb is
        // blank at every act; the rim just inside it is `HAlpha`, at most `RIM_LEVEL`.
        let size = Size::new(120, 40);
        let mut g = SolarEruption::new();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let v = g.view;
        let limb = LIMB_R * v.scale;
        let px = g.pixels.pixel_size();
        let total: f64 = ACTS.iter().map(|a| a.1).sum();
        let mut rim = 0;
        for k in 0..10 {
            let until = total * f64::from(k) / 10.0;
            while g.t < until {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let above = (fx - v.cx).hypot(fy - v.cy) - limb;
                    let ink = g.pixels.get(x, y).unwrap();
                    if above < -2.0 {
                        assert_eq!(ink, Ink::BLANK, "t {:.1} ({x}, {y})", g.t);
                    } else if above < 0.0 && ink.role == Role::Ramp(Ramp::HAlpha) {
                        rim += usize::from(k == 0);
                        assert!(f64::from(ink.level) <= HA_PEAK + 1.0);
                    }
                }
            }
        }
        assert!(rim > 20, "{rim} rim pixels");
    }

    #[test]
    fn erup_a9_every_act_is_captioned() {
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(6);
        let total = ACTS.iter().map(|a| a.1).sum::<f64>();
        while g.t < total {
            g.step(SIM_DT, &mut rng);
            let c = g.caption().expect("a caption in every act");
            assert!(META.captions.contains(&c), "{c}");
        }
        assert_eq!(ACTS[QUIET].0, META.captions[0]);
    }

    #[test]
    fn erup_a5_at_most_twenty_lines_retraced_per_frame() {
        const { assert!(RETRACE <= 20) };
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(3);
        let before = g.cursor;
        g.step(SIM_DT, &mut rng);
        let moved = (g.cursor + LINES - before) % LINES;
        assert!(moved <= 20);
    }
}
