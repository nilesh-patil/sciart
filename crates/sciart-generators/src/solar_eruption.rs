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
//! M16 (GEN-R25, thread style): every field line, the rope and the CME are braille
//! threads; closed loops `Hot`, open field `Cold`, the flare `Sun`/`Star`, the CME front
//! and its plasma `Accent`.
//!
//! Units: lengths in `L` (≈ 50 Mm), simulated seconds, hours of evolution compressed;
//! on screen, dots.

use std::cell::RefCell;
use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::ease;
use sciart_core::prelude::*;
use sciart_core::raster::stipple_keep;
use sciart_core::rng::hash01;

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
/// Inks (M16, GEN-R25): braille threads in flat steps. Closed loops `Hot`, open field
/// `Cold` (ERUP-A12), each segment's brightness `e` picking a step (`e ≥ from`), below the
/// last not drawn.
pub(crate) const CLOSED: [(f64, Ink); 3] = [
    (0.75, Ink::new(Role::Hot, 245)),
    (0.3, Ink::new(Role::Hot, 170)),
    (0.12, Ink::new(Role::Hot, 90)),
];
pub(crate) const OPEN: [(f64, Ink); 3] = [
    (0.75, Ink::new(Role::Cold, 245)),
    (0.3, Ink::new(Role::Cold, 170)),
    (0.12, Ink::new(Role::Cold, 90)),
];
/// The flare (ERUP-A11): loops hotter than `FLARE_HOT` are `FLARE_YELLOW`, their tops
/// (the middle `TOP_SHARE` of its points while hotter than `TOP_HOT`) `FLARE_WHITE`; the ribbons
/// the same pair.
const FLARE_HOT: f64 = 0.3;
const TOP_HOT: f64 = 0.8;
const TOP_SHARE: f64 = 0.12;
/// The share of the cool field lines (closed and open, each mirror twin on its own) drawn;
/// fewer while the loops rise (ERUP-A17: a calmer `loops rise`), eased in and out so lines
/// leave and return one by one, each at its own fixed threshold.
pub(crate) const LINE_KEEP: f64 = 0.7;
const RISE_KEEP: (f64, f64) = (0.2, 0.1);

/// Whether cool field line `k` (an index into the lines and their mirror twins) is drawn
/// at keep share `keep`.
pub(crate) fn drawn(k: usize, keep: f64) -> bool {
    hash01(0x7A1E ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) < keep
}
pub(crate) const FLARE_WHITE: Ink = Ink::new(Role::Star, 255);
pub(crate) const FLARE_YELLOW: Ink = Ink::new(Role::Sun, 240);
/// The flux rope (M17): helical strands `Accent` where they face the viewer, `Hot` behind,
/// faint `Hot` while it forms; its prominence core one bright `Hot` thread.
pub(crate) const ROPE_FRONT: Ink = Ink::new(Role::Accent, 230);
pub(crate) const ROPE_BACK: Ink = Ink::new(Role::Hot, 150);
pub(crate) const ROPE_FAINT: Ink = Ink::new(Role::Hot, 90);
pub(crate) const ROPE_CORE: Ink = Ink::new(Role::Hot, 240);
/// The CME front (M17, ERUP-A15): sparse white-light stipple, `Star` on top where it is
/// brightest, `Cold` down the flanks; golden-angle samples, `FRONT_FILL` per dot of the band
/// circumference, each at a fixed depth in the front's band and with a fixed threshold.
pub(crate) const FRONT_STAR: Ink = Ink::new(Role::Star, 230);
pub(crate) const FRONT_COLD: Ink = Ink::new(Role::Cold, 170);
const FRONT_SAMPLES: usize = 3000;
const FRONT_FILL: f64 = 0.5;
const FRONT_KEEP: f64 = 0.9;
/// Golden-angle fraction: any prefix of `frac(i·φ)` is evenly spread round the circle.
const GOLDEN: f64 = 0.618_033_988_749_895;
/// Loop plasma (M17): stipple beside the closed loops, kept with probability ∝ emission
/// measure (the loop brightness `e`, itself ∝ n²), in a band `PLASMA_REACH` dots either side of the thread; densest at the
/// hot loop tops in the flare.
const PLASMA_REACH: i32 = 2;
const PLASMA_EM: f64 = 0.22;
const PLASMA_CAP: f64 = 0.35;
const PLASMA_LOOP: Ink = Ink::new(Role::Hot, 140);
/// Loop brightness from which its plasma knots are drawn `Sun` (the densest knots).
const PLASMA_KNOT: f64 = 0.55;
const PLASMA_FLARE: Ink = Ink::new(Role::Sun, 220);
const PLASMA_TOP: Ink = Ink::new(Role::Star, 220);
/// The CME front's inner edge (share of the bubble radius); inside it is the cavity.
pub(crate) const FRONT_INNER: f64 = 0.9;
/// Coronal rain.
const RAIN_INK: Ink = Ink::new(Role::Hot, 240);
/// The limb: a thin arc just inside it.
pub(crate) const LIMB: Ink = Ink::new(Role::Hot, 100);

/// The step of `steps` that brightness `e` reaches, if any.
fn step_ink(e: f64, steps: &[(f64, Ink)]) -> Option<Ink> {
    steps.iter().find(|(from, _)| e >= *from).map(|s| s.1)
}

/// What a segment belongs to (ERUP-A12, ERUP-A13).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Part {
    /// An arcade field line, closed (both feet on the surface) or open.
    Loop {
        closed: bool,
    },
    /// A loop segment drawn in the flare's inks.
    Flare,
    Ribbon,
    Rope,
    Front,
    /// Loop plasma stipple (a single dot).
    Plasma,
    Rain,
}

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

/// Rope strands drawn at view scale `scale` (half-block pixels per `L`): two at small sizes, where
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
pub(crate) const STRANDS: usize = 3;
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

/// M16: the front is drawn as a thread on the bubble's rim (where this profile peaks),
/// so only the three-part test reads the profile (ERUP-A8).
#[cfg(test)]
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

/// Plasma stipple beside the thread from `a` to `b` (dots): candidate dots up to
/// `PLASMA_REACH` dots either side (one per dot of length), each kept with probability
/// `em`, falling off with distance from the thread, against its fixed threshold in layer
/// `salt` (CORE-R107), so a still loop's plasma holds still (GEN-R26).
fn plasma_dots(a: (f64, f64), b: (f64, f64), em: f64, salt: u64, mut emit: impl FnMut((f64, f64))) {
    if em <= 0.0 {
        return;
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if !(len > 1e-9 && len.is_finite()) {
        return;
    }
    let (nx, ny) = (-dy / len, dx / len);
    let n = len.ceil() as usize;
    for i in 0..n {
        let f = (i as f64 + 0.5) / n as f64;
        let (cx, cy) = (a.0 + f * dx, a.1 + f * dy);
        for o in (-PLASMA_REACH..=PLASMA_REACH).filter(|&o| o != 0) {
            let fall = 1.0 - f64::from(o.abs()) / f64::from(PLASMA_REACH + 1);
            let (x, y) = (
                (cx + f64::from(o) * nx).round(),
                (cy + f64::from(o) * ny).round(),
            );
            if stipple_keep(salt, x as i32, y as i32, em * fall) {
                emit((x, y));
            }
        }
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
    /// Screen position of the Sun's centre, dots per `L`.
    cx: f64,
    cy: f64,
    scale: f64,
}

impl View {
    /// Dot of coronal point `(x, y)`: `x` becomes an angle round the limb.
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
    /// The view in dots (square), and its scale in half-block pixels per `L` (the unit
    /// `strands_for` and the crowding guard are tuned in).
    view: View,
    px_scale: f64,
    /// The braille lattice; a cell so `prepare` can draw into it while reading the
    /// rest of `self` (no placeholder lattice allocated per frame, GEN-R11).
    dots: RefCell<Dots>,
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
            px_scale: 1.0,
            dots: RefCell::default(),
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

    /// A blob's position, or where it was `back` points up its loop.
    fn blob_at(&self, b: &Blob, back: f64) -> P {
        let l = &self.lines[b.line];
        let s = (b.s - b.dir * back).clamp(0.0, (l.len.max(2) - 1) as f64);
        let k = (s as usize).min(l.len.saturating_sub(2));
        let f = (s - k as f64).clamp(0.0, 1.0) as f32;
        let (a, c) = (l.pts[k], l.pts[(k + 1).min(l.len - 1)]);
        [
            f64::from(a[0] + f * (c[0] - a[0])),
            f64::from(a[1] + f * (c[1] - a[1])),
        ]
    }

    fn blob_pos(&self, b: &Blob) -> P {
        self.blob_at(b, 0.0)
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

    /// The CME bubble now in dots: `(centre, radius, amplitude)`.
    fn shell(&self) -> Option<((f64, f64), f64, f64)> {
        let beat = self.timeline.at(self.t);
        bubble(beat.index, beat.progress).map(|(hc, r)| {
            let amp = match beat.index {
                FLARE => ease::smoothstep(((beat.progress - 0.15) / 0.6).clamp(0.0, 1.0)),
                // Gone by 55 % of the act: the front leaves as a whole, with no flanks
                // left standing at the frame's top edge.
                _ => 1.0 - ease::smoothstep((beat.progress / 0.55).min(1.0)),
            };
            (self.view.px(0.0, hc), r * self.view.scale, amp)
        })
    }

    /// The cavity's dimming at dot `p` (1 outside the bubble or on the disc).
    fn dim_at(&self, shell: Option<((f64, f64), f64, f64)>, p: (f64, f64)) -> f64 {
        let Some((c, r, amp)) = shell else {
            return 1.0;
        };
        let d = (p.0 - c.0).hypot(p.1 - c.1) / r.max(1e-9);
        if d >= 1.0 {
            1.0
        } else {
            1.0 - amp * (1.0 - cavity(d))
        }
    }

    /// Whether dot `p` lies in the CME's cavity while the front is up: inside the front's
    /// inner edge and off the disc, where only the core (the flux rope) and the front
    /// itself are drawn (ERUP-A14).
    fn in_cavity(&self, shell: Option<((f64, f64), f64, f64)>, p: (f64, f64)) -> bool {
        let Some((c, r, amp)) = shell else {
            return false;
        };
        let v = self.view;
        amp > 0.0
            && (p.0 - c.0).hypot(p.1 - c.1) < FRONT_INNER * r
            && (p.0 - v.cx).hypot(p.1 - v.cy) > LIMB_R * v.scale + 1.0
    }

    /// The shares of cool closed and open field lines drawn now: fewer while the loops
    /// rise (ERUP-A17), eased in over 0.8 s; the closed loops return over the flare's
    /// first 1.5 s, the open field (which the rising rope stretches out) only over the
    /// CME's, so the flare stays uncluttered.
    fn line_keep(&self) -> (f64, f64) {
        let beat = self.timeline.at(self.t);
        let ease_in = ease::smoothstep((beat.local / 0.8).min(1.0));
        let ease_out = 1.0 - ease::smoothstep((beat.local / 1.5).min(1.0));
        let (closed, open) = match beat.index {
            RISE => (ease_in, ease_in),
            FLARE => (ease_out, 1.0),
            CME => (0.0, ease_out),
            _ => (0.0, 0.0),
        };
        (
            ease::lerp(LINE_KEEP, RISE_KEEP.0, closed),
            ease::lerp(LINE_KEEP, RISE_KEEP.1, open),
        )
    }

    /// Every segment `(from, to, ink, part)` in dots, in drawing order; single dots
    /// (stipple, ribbons) come as `from == to`.
    fn segments(&self, mut f: impl FnMut((f64, f64), (f64, f64), Ink, Part)) {
        let v = self.view;
        let beat = self.timeline.at(self.t);
        let t = self.t;
        let shell = self.shell();
        let keep = self.line_keep();
        // At small sizes the loops crowd into fewer dots: thin them.
        let dense = (self.px_scale / 28.0).clamp(0.6, 1.0);
        // Loops: each quiet strand heats and cools on its own cycle; reconnected loops
        // are hot, then cooling. The rope's own closed field lines are not drawn: the
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
            let closed = line.foot.1.is_finite();
            let mut draw = |pose: Pose, flip: f64, kk: usize| {
                // Thinned: a fixed share of the cool lines is not drawn, so the arcade is
                // not a tangle; hot post-flare loops always are.
                let share = if closed { keep.0 } else { keep.1 };
                if hot <= FLARE_HOT && !drawn(kk, share) {
                    return;
                }
                let salt = 0x9A5_0000 ^ (kk as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let at = |p: &[f32; 2]| pose.apply(m, [flip * f64::from(p[0]), f64::from(p[1])]);
                let mut prev = at(&pts[0]);
                let mut a = v.px(prev[0], prev[1]);
                for (j, p) in pts.iter().enumerate().skip(1) {
                    let q = at(p);
                    let b = v.px(q[0], q[1]);
                    // Behind the limb (a posed loop rooted on the far side): occulted; in
                    // the CME's cavity: emptied.
                    let shown = prev[1] >= 0.0
                        && q[1] >= 0.0
                        && !self.in_cavity(shell, a)
                        && !self.in_cavity(shell, b);
                    if shown {
                        if hot > FLARE_HOT {
                            let mid = j as f64 / pts.len() as f64 - 0.5;
                            let top = hot > TOP_HOT && mid.abs() < 0.5 * TOP_SHARE;
                            f(
                                a,
                                b,
                                if top { FLARE_WHITE } else { FLARE_YELLOW },
                                Part::Flare,
                            );
                            // Evaporated plasma fills the new loop; its emission measure
                            // peaks at the loop top.
                            let crown = (-(mid / 0.14) * (mid / 0.14)).exp();
                            let em = hot * (0.1 + 0.3 * crown);
                            let ink = if crown > 0.5 && hot > TOP_HOT {
                                PLASMA_TOP
                            } else {
                                PLASMA_FLARE
                            };
                            plasma_dots(a, b, em, salt, |p| {
                                if !self.in_cavity(shell, p) {
                                    f(p, p, ink, Part::Plasma);
                                }
                            });
                        } else {
                            let mid = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
                            let e = w * along(kk, j as f64 * DS, t) * self.dim_at(shell, mid);
                            let steps = if closed { &CLOSED } else { &OPEN };
                            if let Some(ink) = step_ink(e, steps) {
                                f(a, b, ink, Part::Loop { closed });
                                if closed {
                                    let em = (PLASMA_EM * e).min(PLASMA_CAP);
                                    // The densest knots glow brightest (EM ∝ n²).
                                    let ink = if e >= PLASMA_KNOT {
                                        PLASMA_FLARE
                                    } else {
                                        PLASMA_LOOP
                                    };
                                    plasma_dots(a, b, em, salt, |p| {
                                        if !self.in_cavity(shell, p) {
                                            f(p, p, ink, Part::Plasma);
                                        }
                                    });
                                }
                            }
                        }
                    }
                    (prev, a) = (q, b);
                }
            };
            draw(loop_pose(k), 1.0, k);
            // The sources are mirror-antisymmetric about the inversion line, so the field
            // is mirror-symmetric: a line that does not close back on the far polarity
            // (it leaves the domain) has a twin on the other side, drawn by reflection
            // (with a pose of its own).
            if !closed {
                draw(loop_pose(k + LINES), -1.0, k + LINES);
            }
        }
        // Flare ribbons (ERUP-A16): dotted `Sun` lines along the limb at the post-flare
        // loops' feet, moving apart as the X-point climbs.
        let flash = match beat.index {
            FLARE => 1.0,
            CME => (1.0 - beat.progress).max(0.25),
            RAIN => 0.25 * (1.0 - beat.progress),
            _ => 0.0,
        };
        if flash > 0.0 && self.ribbon_seen > 0.0 {
            let span = 0.3 * v.scale;
            let n = (span / 2.0).ceil().max(3.0) as i32;
            for side in [-1.0, 1.0] {
                for k in 0..=n {
                    let u = f64::from(k) / f64::from(n);
                    let taper = (std::f64::consts::PI * u).sin_det().max(0.35) * (1.0 - 0.5 * u);
                    if flash * taper < 0.3 {
                        continue;
                    }
                    let x = side * (self.ribbon_seen - 0.2 + 0.3 * u);
                    let p = v.px(x, 0.6 / v.scale.max(1e-9));
                    f(p, p, FLARE_YELLOW, Part::Ribbon);
                }
            }
        }
        // Coronal rain: bright knots sliding down the loops they condensed on, each with
        // a short trail up the loop.
        for b in &self.blobs {
            let l = &self.lines[b.line];
            let hot = if l.age.is_finite() {
                (-l.age / COOL).exp()
            } else {
                0.0
            };
            let pose = loop_pose(b.line);
            let (p, q) = (
                pose.apply(1.0 - 0.55 * hot, self.blob_pos(b)),
                pose.apply(1.0 - 0.55 * hot, self.blob_at(b, 3.0)),
            );
            let (pd, qd) = (v.px(p[0], p[1]), v.px(q[0], q[1]));
            if p[1] >= 0.0 && q[1] >= 0.0 && !self.in_cavity(shell, pd) && !self.in_cavity(shell, qd) {
                f(qd, pd, RAIN_INK, Part::Rain);
            }
        }
        // The flux rope: helical strands round a sheared arch, rising and brightening; in
        // the CME it is the bright core low in the cavity, its legs still rooted.
        let gain = rope_gain(beat.index, beat.progress);
        let hr = self.rope_apex();
        if gain > 0.05 && hr > 0.0 {
            const N: usize = 90;
            let radius = (0.045 + 0.08 * hr).min(0.22);
            for j in 0..strands_for(self.px_scale) {
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
                    let q = v.px(a[0] + r * nx, (a[1] + r * ny).max(0.0));
                    if let Some(p) = prev {
                        let ink = if gain < 0.3 {
                            ROPE_FAINT
                        } else if face >= 0.5 {
                            ROPE_FRONT
                        } else {
                            ROPE_BACK
                        };
                        f(p, q, ink, Part::Rope);
                    }
                    prev = Some(q);
                }
            }
            // The core: dense prominence plasma cradled in the rope's dips, the CME's
            // bright third part, as one bright thread along the axis.
            if matches!(beat.index, FLARE | CME) {
                let mut prev = None;
                for i in 0..=12 {
                    let a = rope_axis(0.28 + 0.0275 * f64::from(i), hr);
                    let q = v.px(a[0], a[1]);
                    if let Some(p) = prev {
                        f(p, q, ROPE_CORE, Part::Rope);
                    }
                    prev = Some(q);
                }
            }
        }
        // The CME front (ERUP-A15): sparse stipple in a thin band at the bubble's rim,
        // brightest on top and streaky along it, riding the self-similar expansion.
        if let Some((c, r, amp)) = shell {
            let spread = 1.6
                + if beat.index == CME {
                    3.0 * beat.progress
                } else {
                    0.0
                };
            let limb_r = LIMB_R * v.scale;
            let band = (r * (1.0 - FRONT_INNER)).max(2.0);
            let front_boost = (2.0 - self.px_scale / 40.0).clamp(1.0, 1.7);
            let n =((TAU * r * band * FRONT_FILL) as usize).min(FRONT_SAMPLES);
            for i in 0..n {
                let key = 0xF207_0000 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let (s, co) = (TAU * (i as f64 * GOLDEN).fract()).sin_cos_det();
                // Brightest on top, fading down the flanks; streaky along the front.
                let (up, below) = ((-s).max(0.0), s.max(0.0));
                let ux = ((co + 1.0) * 31.5).clamp(0.0, 63.0);
                let kk = (ux as usize).min(62);
                let fr = ux - kk as f64;
                let streak = f64::from(self.front_tex[kk]) * (1.0 - fr)
                    + f64::from(self.front_tex[kk + 1]) * fr;
                let flank = (0.04 + 0.96 * up.powf(spread)) * (1.0 - 2.0 * below).max(0.0);
                // Small screens have few dots on the rim: boost the share kept there.
                let lit = amp * flank * (0.25 + 1.8 * streak * streak) * front_boost;
                if hash01(key ^ 0x5) >= FRONT_KEEP * lit.min(1.0) {
                    continue;
                }
                let d = r - band * hash01(key);
                let p = (c.0 + d * co, c.1 + d * s);
                if (p.0 - v.cx).hypot(p.1 - v.cy) >= limb_r {
                    let ink = if lit >= 0.8 { FRONT_STAR } else { FRONT_COLD };
                    f(p, p, ink, Part::Front);
                }
            }
        }
    }

    fn compose(&mut self) {
        let mut dots = self.dots.borrow_mut();
        dots.clear();
        // The limb: a thin arc just inside it.
        let v = self.view;
        let r = LIMB_R * v.scale - 1.0;
        let n = (TAU * r).ceil().max(8.0) as usize;
        for k in 0..n {
            let (s, c) = (TAU * k as f64 / n as f64).sin_cos_det();
            dots.plot(v.cx + r * s, v.cy - r * c, LIMB);
        }
        self.segments(|a, b, ink, part| match part {
            Part::Ribbon | Part::Plasma | Part::Front => dots.plot(b.0, b.1, ink),
            _ => dots.line(a.0, a.1, b.0, b.1, ink),
        });
        drop(dots);
    }
}

impl Generator for SolarEruption {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.dots.get_mut().resize(size);
        let ds = self.dots.get_mut().dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        let scale = (w / 4.2).min(h / 1.9).max(1.0);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.84 * h + LIMB_R * scale,
            scale,
        };
        // Two dots per half-block pixel.
        self.px_scale = 0.5 * scale;
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
        self.dots.borrow().blit(canvas);
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
    }

    /// The scene built at `size` and run to loop time `until`.
    fn run_to(size: Size, until: f64) -> (SolarEruption, Ctx) {
        let mut g = SolarEruption::new();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        while g.t < until {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        (g, ctx)
    }

    fn canvas_of(g: &SolarEruption, size: Size) -> Canvas {
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        canvas
    }

    fn count(canvas: &Canvas, ink: Ink) -> usize {
        canvas
            .cells()
            .iter()
            .filter(|c| c.ch != ' ' && c.ink == ink)
            .count()
    }

    #[test]
    fn erup_a8_m17_inks_mid_cme() {
        // M17: the M16 inks less the `Accent` front and plasma, plus the stipple front
        // (`Star`/`Cold`), the loop plasma, and the rope's `Accent` front strands.
        let size = Size::new(80, 24);
        let mid = ACTS[..CME].iter().map(|a| a.1).sum::<f64>() + 3.0;
        let (g, _) = run_to(size, mid);
        let canvas = canvas_of(&g, size);
        let inks: Vec<Ink> = CLOSED
            .iter()
            .chain(&OPEN)
            .map(|s| s.1)
            .chain([
                FLARE_WHITE,
                FLARE_YELLOW,
                ROPE_FRONT,
                ROPE_BACK,
                ROPE_FAINT,
                ROPE_CORE,
                FRONT_STAR,
                FRONT_COLD,
                PLASMA_LOOP,
                PLASMA_FLARE,
                PLASMA_TOP,
                RAIN_INK,
                LIMB,
            ])
            .collect();
        for c in canvas.cells() {
            if c.ch == ' ' {
                continue;
            }
            assert_eq!(c.bg, Ink::BLANK, "half block");
            assert!(('\u{2800}'..='\u{28ff}').contains(&c.ch), "{:?}", c.ch);
            assert!(inks.contains(&c.ink), "{:?}", c.ink);
        }
        assert!(
            count(&canvas, ROPE_FRONT) + count(&canvas, ROPE_CORE) > 0,
            "the rope shows"
        );
    }

    #[test]
    fn erup_a10_the_disc_is_black_under_a_thin_limb() {
        let size = Size::new(120, 40);
        let mut g = SolarEruption::new();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let v = g.view;
        let limb = LIMB_R * v.scale;
        let total: f64 = ACTS.iter().map(|a| a.1).sum();
        for k in 0..10 {
            let until = total * f64::from(k) / 10.0;
            while g.t < until {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            let canvas = canvas_of(&g, size);
            let mut rim = 0;
            for y in 0..40 {
                for x in 0..120 {
                    // The cell's dot farthest from the Sun's centre.
                    let far = (0..2)
                        .flat_map(|i| (0..4).map(move |j| (2 * x + i, 4 * y + j)))
                        .map(|(dx, dy)| (f64::from(dx) - v.cx).hypot(f64::from(dy) - v.cy))
                        .fold(0.0, f64::max);
                    let c = canvas.get(x, y).unwrap();
                    if far < limb - 2.0 {
                        assert_eq!(c.ch, ' ', "t {:.1} ({x}, {y})", g.t);
                    }
                    rim += usize::from(c.ch != ' ' && c.ink == LIMB);
                }
            }
            assert!(rim > 20, "t {:.1}: {rim} limb cells", g.t);
        }
    }

    #[test]
    fn erup_a11_the_flare_lights_white_loop_tops_and_ribbons() {
        let size = Size::new(120, 40);
        let (g, _) = run_to(size, 0.0);
        let canvas = canvas_of(&g, size);
        // Quiet: no flare.
        assert_eq!(
            count(&canvas, FLARE_WHITE) + count(&canvas, FLARE_YELLOW),
            0
        );
        // Mid-flare: white and yellow loop tops and ribbons.
        let mid = ACTS[..FLARE].iter().map(|a| a.1).sum::<f64>() + 0.6 * ACTS[FLARE].1;
        let (g, _) = run_to(size, mid);
        let canvas = canvas_of(&g, size);
        let (white, yellow) = (count(&canvas, FLARE_WHITE), count(&canvas, FLARE_YELLOW));
        assert!(white > 10 && yellow > 10, "white {white}, yellow {yellow}");
    }

    #[test]
    fn erup_a12_closed_loops_hot_open_field_cold() {
        let size = Size::new(120, 40);
        let total: f64 = ACTS.iter().map(|a| a.1).sum();
        let (mut closed, mut open) = (0, 0);
        for k in 0..6 {
            let (g, _) = run_to(size, total * f64::from(k) / 6.0);
            g.segments(|_, _, ink, part| {
                if let Part::Loop { closed: c } = part {
                    if c {
                        assert!(CLOSED.iter().any(|s| s.1 == ink), "{ink:?}");
                        closed += 1;
                    } else {
                        assert!(OPEN.iter().any(|s| s.1 == ink), "{ink:?}");
                        open += 1;
                    }
                }
                if part == Part::Flare {
                    assert!(ink == FLARE_WHITE || ink == FLARE_YELLOW);
                }
            });
        }
        assert!(closed > 100 && open > 100, "closed {closed}, open {open}");
    }

    #[test]
    fn erup_a13_the_cme_front_rides_the_rim_and_expands() {
        // M17: the front is stipple in a thin band at the rim (the M16 plasma dots, which
        // sat in what is now the dark cavity, are gone); a fixed front sample rides the
        // self-similar expansion outward.
        let size = Size::new(120, 40);
        let start = ACTS[..CME].iter().map(|a| a.1).sum::<f64>();
        let mut tops = Vec::new();
        for dt in [0.5, 1.5, 3.0] {
            let (g, _) = run_to(size, start + dt);
            let (c, r, _) = g.shell().expect("a bubble in the CME act");
            let band = (r * (1.0 - FRONT_INNER)).max(2.0);
            let mut front = 0;
            let mut top = f64::MAX;
            g.segments(|a, _, _, part| {
                if part == Part::Front {
                    let d = (a.0 - c.0).hypot(a.1 - c.1);
                    assert!(d <= r + 1e-9 && d >= r - band - 1e-9, "front at {d} vs {r}");
                    top = top.min(a.1);
                    front += 1;
                }
            });
            assert!(front > 10, "front {front} at {dt}");
            tops.push(top);
        }
        assert!(tops.windows(2).all(|w| w[1] < w[0]), "the front rises: {tops:?}");
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

    /// Corona `x` (`L` along the limb) of dot `p`: the inverse of `View::px`.
    fn corona_x(g: &SolarEruption, p: (f64, f64)) -> f64 {
        let v = g.view;
        (p.0 - v.cx).atan2(v.cy - p.1) * LIMB_R
    }

    /// Whether dot `p` is off the disc (outside the limb by more than a dot).
    fn off_disc(g: &SolarEruption, p: (f64, f64)) -> bool {
        let v = g.view;
        (p.0 - v.cx).hypot(p.1 - v.cy) > LIMB_R * v.scale + 1.0
    }

    #[test]
    fn erup_a14_the_cme_cavity_is_dark() {
        let size = Size::new(120, 40);
        let start = ACTS[..CME].iter().map(|a| a.1).sum::<f64>();
        for dt in [0.3, 1.0, 2.0, 3.5] {
            let (g, _) = run_to(size, start + dt);
            let (c, r, _) = g.shell().expect("a bubble in the CME act");
            let mut inside = 0;
            g.segments(|a, b, ink, part| {
                for p in [a, b] {
                    let d = (p.0 - c.0).hypot(p.1 - c.1) / r;
                    if d < FRONT_INNER && off_disc(&g, p) {
                        // Only the core (the flux rope) lives in the cavity.
                        assert!(
                            matches!(part, Part::Rope | Part::Front),
                            "{part:?} {ink:?} in the cavity at d {d:.2}"
                        );
                        inside += 1;
                    }
                }
            });
            assert!(inside > 0 || dt > 2.5, "the core shows in the cavity");
        }
    }

    #[test]
    fn erup_a15_the_cme_front_is_sparse_stipple() {
        let size = Size::new(120, 40);
        let start = ACTS[..CME].iter().map(|a| a.1).sum::<f64>();
        for dt in [0.3, 1.0, 2.0] {
            let (g, _) = run_to(size, start + dt);
            let (c, r, _) = g.shell().expect("a bubble");
            let mut dots = Dots::new(size);
            let mut n = 0;
            g.segments(|a, b, ink, part| {
                if part == Part::Front {
                    assert_eq!(a, b, "front dots, not strokes");
                    assert!(matches!(ink.role, Role::Star | Role::Cold), "{ink:?}");
                    let d = (a.0 - c.0).hypot(a.1 - c.1);
                    assert!(d <= r + 1.0 && d >= FRONT_INNER * r - 2.5, "front at {d:.1} of {r:.1}");
                    dots.plot(a.0, a.1, ink);
                    n += 1;
                }
            });
            assert!(n > 20, "{n} front dots");
            // Along the front's band (the annulus over the arc where it is drawn), at most
            // half the dots are lit.
            let ds = dots.dot_size();
            let (mut band, mut lit) = (0usize, 0usize);
            for y in 0..i32::from(ds.height()) {
                for x in 0..i32::from(ds.width()) {
                    let p = (f64::from(x), f64::from(y));
                    let d = (p.0 - c.0).hypot(p.1 - c.1);
                    if p.1 < c.1 && d >= FRONT_INNER * r - 2.5 && d <= r + 1.0 && off_disc(&g, p) {
                        band += 1;
                        lit += usize::from(dots.is_lit(x, y));
                    }
                }
            }
            assert!(lit * 2 <= band, "{lit} of {band} band dots lit");
        }
    }

    #[test]
    fn erup_a16_sun_ribbons_at_the_loop_feet_separate() {
        let size = Size::new(120, 40);
        let start = ACTS[..FLARE].iter().map(|a| a.1).sum::<f64>();
        let mut seps = Vec::new();
        for dt in [1.5, 3.0, 5.5] {
            let (g, _) = run_to(size, start + dt);
            let feet: Vec<f64> = g
                .lines
                .iter()
                .filter(|l| l.age.is_finite() && l.foot.1.is_finite())
                .map(|l| l.foot.0.abs())
                .collect();
            assert!(!feet.is_empty(), "post-flare loops at {dt}");
            let (mut sum, mut n) = (0.0, 0);
            g.segments(|_, b, ink, part| {
                if part == Part::Ribbon {
                    assert_eq!(ink.role, Role::Sun, "{ink:?}");
                    let x = corona_x(&g, b).abs();
                    let near = feet.iter().map(|f| (f - x).abs()).fold(f64::MAX, f64::min);
                    assert!(near < 0.3, "ribbon dot {x:.2} far from the feet ({near:.2})");
                    sum += x;
                    n += 1;
                }
            });
            assert!(n > 4, "{n} ribbon dots at {dt}");
            seps.push(sum / f64::from(n));
        }
        assert!(seps.windows(2).all(|w| w[1] > w[0]), "separation {seps:?}");
    }

    #[test]
    fn erup_a17_loops_rise_is_calmer_than_m16() {
        // M16's lit cells in `loops rise` (seed 42, 200×50, the scene alone), measured at
        // the base commit 75f5552 before the M17 redesign.
        const M16: [(f64, usize); 4] = [(7.0, 1094), (9.0, 973), (11.0, 1391), (13.0, 1830)];
        let size = Size::new(200, 50);
        let mut g = SolarEruption::new();
        let mut ctx = Ctx::new(42, size);
        g.resize(size, &mut ctx);
        for (t, m16) in M16 {
            while g.t < t {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            assert_eq!(g.caption(), Some("loops rise"));
            let canvas = canvas_of(&g, size);
            let lit = canvas.cells().iter().filter(|c| c.ch != ' ').count();
            assert!(lit * 10 <= m16 * 7, "t {t}: {lit} lit cells vs M16 {m16}");
        }
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
