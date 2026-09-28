//! A lattice of unstable nuclei decaying through the chain `A → B → C` beside live
//! population curves and the analytic Bateman solution (SPEC-002 `radioactive_decay`).
//!
//! Physics: each nucleus is an independent continuous-time Markov chain with rates
//! `λ_A = ln 2 / T_A`, `λ_B = ln 2 / T_B` and C stable. A frame step `h` is sampled from
//! the chain's *exact* transition matrix ([`Transition`]) with one uniform draw per live
//! nucleus:
//!
//! ```text
//!          A                  B                                   C
//!   A  [ e^{−λ_A h}   λ_A/(λ_B−λ_A)(e^{−λ_A h} − e^{−λ_B h})   rest ]
//!   B  [ 0            e^{−λ_B h}                                rest ]
//!   C  [ 0            0                                         1    ]
//! ```
//!
//! The single-species decay probability is `p = 1 − 2^{−h/T}` (RAD-A1), and the A row
//! already contains the paths `A → B → C` completed *inside* one step (the "rest" of the A
//! row), so a long step (dt = 0.25 s, or the 6 s step in the tests) is not biased by
//! forbidding two hops per step. Because the matrix is exact, `M(h₁)M(h₂) = M(h₁ + h₂)`
//! (Chapman–Kolmogorov) and the populations are exactly binomial with the Bateman
//! fractions at every time, for any sequence of step sizes (RAD-A2).
//!
//! Each nucleus is a square of 2 × 2 half-block pixels on a pitch of 3, so the lattice reads
//! as separate nuclei on a quiet black ground rather than as one-pixel noise (RAD-A5). The
//! parent and daughter differ in hue and grey level: A a warm `HAlpha` rose, B a teal
//! `Ice`, while stable C sinks to a near-black `Plasma` ember of the ground, so the
//! lattice fades out as it decays. Events are
//! the brightest thing on the lattice (RAD-A8): a decay flashes its nucleus at the top of
//! the daughter's ramp and eases back to the daughter's colour over [`FLASH`] s, long
//! enough for the eye to follow, and a uniform sample of decays also burst (a small ring)
//! and leave a braille track, drawn as if in a cloud chamber in a magnetic field: an α
//! after A → B (short, straight, thick, mono-energetic), a β⁻ after B → C (always longer,
//! thin, curling the other way and knocked about by scattering, with a continuous energy
//! spectrum). An A → C within one step emits both.
//! Part-way through each run the plot's y axis eases from linear to logarithmic, where
//! every exponential is a straight line and B's line turns parallel to A's (transient
//! equilibrium, RAD-A6). The measured counts are drawn bright over the dim analytic curves.
//!
//! Stylisation (stated in SPEC-002): the chain is fictional, with half-lives of seconds;
//! track ranges, speeds, scattering and the field are display values, only a sample of
//! decays is tracked, and colours are false.
//!
//! Raster convention: the nuclei go into `Pixels`, tracks, bursts and curves into `dots`,
//! at the end of `update` and `resize`; `render` blits them (the dots over the lattice,
//! which carries on as their cells' background) and writes the panel text. ASCII draws
//! the lattice per cell instead.

use std::f64::consts::{LN_2, TAU};
use std::fmt::Write as _;

use rand::Rng;

use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::rng::hash01;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};

pub static META: Meta = Meta {
    name: "radioactive_decay",
    category: Category::Physics,
    description: "A lattice of nuclei decaying through a chain beside live Bateman curves",
    title: "Decay chain · random yet lawful",
    captions: &[CAPTION_LINEAR, CAPTION_LOG],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(RadioactiveDecay::default())
}

/// Act captions (RAD-A6): the linear plot, then the log plot.
const CAPTION_LINEAR: &str = "random yet lawful";
const CAPTION_LOG: &str = "on a log scale";

/// Half-lives, s (spec).
const T_A: f64 = 8.0;
const T_B: f64 = 3.0;
/// Start over when fewer than this fraction of the nuclei are still radioactive (spec).
const RESET_FRAC: f64 = 0.02;
/// The plot's time axis spans this multiple of the expected (analytic) reset time, so a
/// run that ends a little late still lands on the axis.
const AXIS_SLACK: f64 = 1.08;
/// The y axis eases from linear to log over `LOG_EASE` s from `LOG_FROM` s into a run
/// (RAD-A6), spanning `DECADES` decades below N₀.
const LOG_FROM: f64 = 18.0;
const LOG_EASE: f64 = 2.0;
const DECADES: f64 = 3.0;

/// Plot panel: 30 % of the width on the right, shown only when it gets ≥ 24×10 cells.
const PANEL_FRAC: f64 = 0.30;
const PANEL_MIN_W: i32 = 24;
const PANEL_MIN_H: i32 = 10;

/// A decay flashes its nucleus at the top of the daughter's ramp and eases back to the
/// daughter's settled ink over `FLASH` s (quadratic ease-out), so the eye can follow each
/// event; for the first `FLASH_HOT` s it is bright (≥ `BRIGHT` levels) and ASCII shows `*`
/// (RAD-A8).
const FLASH: f32 = 0.3;
const FLASH_HOT: f32 = 0.15;
/// Nuclei are `NUC × NUC` pixel squares on a pitch of `PITCH` pixels (RAD-A5): a one-pixel
/// black gap keeps every nucleus a separate dot.
const NUC: i32 = 2;
const PITCH: i32 = 3;

/// Tracks in pixels (square: one pixel is half a row tall and one column wide), each gone
/// within `*_LIFE` s. α: mono-energetic, so every α has the same range, and heavy, so it
/// runs nearly straight and is drawn two dots thick.
const ALPHA_SPEED: f64 = 25.0; // px/s
const ALPHA_RUN: f64 = 0.2; // s → range 5 px
const ALPHA_OMEGA: f64 = 0.6; // rad/s
const ALPHA_LIFE: f64 = 0.35;
/// β⁻: continuous spectrum (speed drawn per decay), always longer than an α (10–22.5 px),
/// opposite charge ⇒ curls the other way, and light, so it is knocked about (multiple
/// scattering): its heading takes a fixed random kick of up to ±`BETA_SCATTER` rad every
/// `KINK` s of flight.
const BETA_SPEED: (f64, f64) = (40.0, 90.0);
const BETA_RUN: f64 = 0.25;
const BETA_OMEGA: f64 = -5.0;
const BETA_SCATTER: f64 = 0.35;
const BETA_LIFE: f64 = 0.4;
/// Flight time per straight piece of a track.
const KINK: f64 = 0.02;
/// Lattice pixels (not nuclei) per track slot: most decays only flash, so each tracked one
/// stands out.
/// Which decays get a free slot is a uniform sample of the step's decays (RAD-A8).
const TRACKS_PER_PIXEL: usize = 800;
/// A tracked decay also bursts: a braille ring growing from `BURST_R` dots to
/// `BURST_R + BURST_GROW` over `BURST` s, fading as it grows.
const BURST: f64 = 0.12;
const BURST_R: f64 = 2.5;
const BURST_GROW: f64 = 2.0;

/// Species palette (RAD-A5; false colour), ordered A, B, C: parent A a warm `HAlpha` rose
/// at 0.6, daughter B a teal `Ice` at 0.7 (a step brighter, so they part in mono too), and
/// stable C near the ground, a barely-there `Plasma` at 0.07: the lattice fades out as it
/// decays, and only the flashes carry real brightness. C's curve on the plot is the same
/// `Plasma` at [`MEASURED`], a clear lavender. No amber: that hue is the gallery's most used.
const RAMPS: [Ramp; 3] = [Ramp::HAlpha, Ramp::Ice, Ramp::Plasma];
const SETTLED: [f64; 3] = [0.6, 0.7, 0.07];
/// Tracks in hues no species uses: α `Aurora` (green), β `Starlight` (near white), at
/// `TRACK_TOP` of the ramp while moving, fading to `TRACK_FLOOR` of that.
const ALPHA_RAMP: Ramp = Ramp::Aurora;
const BETA_RAMP: Ramp = Ramp::Starlight;
const TRACK_TOP: f64 = 0.95;
const TRACK_FLOOR: f64 = 0.7;
/// Ramp value of the plot's measured curves (the species' hues, bright on black), and the
/// analytic reference curves' dim `Structure` level. Measured curves are two dots thick.
const MEASURED: f64 = 0.9;
const ANALYTIC_LEVEL: u8 = 55;
/// The legend chip of C, which the lattice does not draw: a dim tile of its ramp.
const C_CHIP: f64 = 0.3;

/// Legend chips per species: a nucleus in its lattice ink (ASCII its ASCII lattice glyph),
/// then its curve's stroke (solid, dashed, dotted, as [`stroke_on`] draws them, so the
/// species read apart in mono by stroke as well as grey).
const MARKS: [Glyph; 3] = [
    Glyph::new('█', '-'),
    Glyph::new('█', 'o'),
    Glyph::new('█', ','),
];
const STROKES: [Glyph; 3] = [
    Glyph::new('─', '-'),
    Glyph::new('╌', '='),
    Glyph::new('┈', '.'),
];
const SEG_H: Glyph = Glyph::new('─', '-');
const SEG_V: Glyph = Glyph::new('│', '|');
const AXIS_CORNER: Glyph = Glyph::new('└', '+');
const AXIS_TICK_X: Glyph = Glyph::new('┴', '+');
const AXIS_TICK_Y: Glyph = Glyph::new('┤', '+');
/// The y-axis ticks as population fractions with their labels (RAD-A6): N₀/2 on the
/// linear plot (read the half-life off the A curve), each decade on the log plot. The
/// labels sit just inside the axis, over the curves (a crossing curve loses a cell or two; the label stays legible) (a panel has ≥ 22 data
/// columns, so a 3-character label always fits).
const TICKS_LINEAR: [(f64, &str); 1] = [(0.5, "50%")];
const TICKS_LOG: [(f64, &str); 2] = [(0.1, "10%"), (0.01, "1%")];
/// `A ─α→ B ─β→ C`.
const HEADER: [Glyph; 13] = [
    Glyph::ascii('A'),
    Glyph::ascii(' '),
    Glyph::new('─', '-'),
    Glyph::new('α', 'a'),
    Glyph::new('→', '>'),
    Glyph::ascii(' '),
    Glyph::ascii('B'),
    Glyph::ascii(' '),
    Glyph::new('─', '-'),
    Glyph::new('β', 'b'),
    Glyph::new('→', '>'),
    Glyph::ascii(' '),
    Glyph::ascii('C'),
];

/// Nuclear species along the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    A,
    B,
    C,
}

fn lambda(t_half: f64) -> f64 {
    LN_2 / t_half
}

/// Probability that a nucleus with half-life `t_half` decays within `dt`:
/// `1 − 2^{−dt/T} = −expm1(−λ dt)` (RAD-A1). `expm1` keeps full relative precision for
/// the tiny per-frame probabilities (≈ 1.4·10⁻³ for A at 60 FPS).
///
/// Degenerate input stays a probability: `dt ≤ 0` or NaN ⇒ 0, `T ≤ 0` ⇒ 1.
pub(crate) fn decay_p(dt: f64, t_half: f64) -> f64 {
    if dt.is_nan() || dt <= 0.0 {
        return 0.0;
    }
    if t_half.is_nan() || t_half <= 0.0 {
        return 1.0;
    }
    (-(-dt * lambda(t_half)).exp_m1()).clamp(0.0, 1.0)
}

/// Bateman fractions `(f_A, f_B, f_C)` at time `t` for a pure-A start (RAD-A2):
/// `f_A = e^{−λ_A t}`, `f_B = λ_A/(λ_B − λ_A)(e^{−λ_A t} − e^{−λ_B t})`, `f_C = 1 − f_A − f_B`.
///
/// The difference of exponentials is taken as a difference of `expm1`s, which is exact
/// cancellation-free at small `t`. `t ≤ 0`/NaN ⇒ `(1, 0, 0)`; `t = ∞` ⇒ `(0, 0, 1)`.
pub(crate) fn bateman(t: f64) -> (f64, f64, f64) {
    if t.is_nan() || t <= 0.0 {
        return (1.0, 0.0, 0.0);
    }
    let (la, lb) = (lambda(T_A), lambda(T_B));
    let (ea, eb) = ((-la * t).exp_m1(), (-lb * t).exp_m1());
    let fa = 1.0 + ea;
    let fb = (la / (lb - la) * (ea - eb)).max(0.0);
    let fc = (1.0 - fa - fb).max(0.0);
    (fa, fb, fc)
}

/// The analytic reset time: when `f_A + f_B` first drops below [`RESET_FRAC`] (≈ 50.6 s).
fn reset_time() -> f64 {
    let (mut lo, mut hi) = (0.0, 1000.0);
    // f_A + f_B is strictly decreasing, so bisection is safe; 60 halvings reach f64 limits.
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        let (a, b, _) = bateman(mid);
        if a + b > RESET_FRAC {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// Exact one-step transition of the chain over `h` (RAD-A1): the first two rows of the
/// matrix in the module docs. C is stable, so its row is the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Transition {
    /// P(A → A) = `e^{−λ_A h}`.
    stay_a: f64,
    /// P(A → B) over the whole step (decayed once, not yet twice) = Bateman `f_B(h)`.
    a_to_b: f64,
    /// P(B → B) = `2^{−h/T_B}`.
    stay_b: f64,
}

impl Transition {
    pub(crate) fn new(h: f64) -> Self {
        let (stay_a, a_to_b, _) = bateman(h);
        Self {
            stay_a,
            a_to_b,
            stay_b: 1.0 - decay_p(h, T_B),
        }
    }

    /// Where a nucleus of `kind` ends up for the uniform draw `u ∈ [0, 1)`: the unit
    /// interval is split into `[0, P(→A)) [.., +P(→B)) [.., 1)` — inverse-CDF sampling of
    /// the matrix row, so one draw decides the whole step including a double decay.
    #[inline]
    pub(crate) fn sample(&self, kind: Kind, u: f64) -> Kind {
        match kind {
            Kind::A if u < self.stay_a => Kind::A,
            Kind::A if u < self.stay_a + self.a_to_b => Kind::B,
            Kind::A => Kind::C,
            Kind::B if u < self.stay_b => Kind::B,
            Kind::B | Kind::C => Kind::C,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Nucleus {
    kind: Kind,
    /// Seconds since this nucleus last changed species (drives the flash and afterglow).
    since: f32,
}

impl Nucleus {
    const FRESH: Self = Self {
        kind: Kind::A,
        since: 0.0,
    };
}

/// Advance every nucleus by one exact step of `dt`; `on_decay(index, from, to)` fires for
/// each nucleus that changed species. Stable C nuclei consume no random numbers.
pub(crate) fn step_nuclei(
    cells: &mut [Nucleus],
    tr: &Transition,
    dt: f64,
    rng: &mut impl Rng,
    mut on_decay: impl FnMut(usize, Kind, Kind),
) {
    let dts = dt as f32;
    for (i, n) in cells.iter_mut().enumerate() {
        n.since = (n.since + dts).min(1.0e3);
        if n.kind == Kind::C {
            continue;
        }
        let to = tr.sample(n.kind, rng.random::<f64>());
        if to != n.kind {
            on_decay(i, n.kind, to);
            *n = Nucleus {
                kind: to,
                since: 0.0,
            };
        }
    }
}

/// Population counts `[N_A, N_B, N_C]` by a full recount (the scene keeps them
/// incrementally; tests cross-check the two).
#[cfg(test)]
fn census(cells: &[Nucleus]) -> [u32; 3] {
    let mut n = [0u32; 3];
    for c in cells {
        n[c.kind as usize] += 1;
    }
    n
}

/// Height of a population fraction `f` on the plot, 0 at the bottom and 1 at the top, for
/// the axis blend `m` (0 linear, 1 log over `DECADES` decades; RAD-A6). On the log axis
/// every exponential `e^{−λt}` is a straight line; `f ≤ 10^{−DECADES}` sits on the bottom.
pub(crate) fn plot_y(f: f64, m: f64) -> f64 {
    let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) };
    let log = if f > 0.0 {
        (1.0 + f.log10() / DECADES).max(0.0)
    } else {
        0.0
    };
    ease::lerp(f, log, m.clamp(0.0, 1.0))
}

/// The axis blend at run time `t` (RAD-A6): linear until `LOG_FROM`, then a smoothstep to
/// log over `LOG_EASE` s.
pub(crate) fn log_blend(t: f64) -> f64 {
    ease::smoothstep(((t - LOG_FROM) / LOG_EASE).clamp(0.0, 1.0))
}

/// The settled ink of species `kind` (RAD-A5): stable C is not drawn at all, so the lattice
/// fades out to the ground as it decays (its flash eases toward C's near-ground level).
pub(crate) fn settled_ink(kind: Kind) -> Ink {
    if kind == Kind::C {
        return Ink::BLANK;
    }
    ink::ramp(RAMPS[kind as usize], SETTLED[kind as usize])
}

/// The ink of nucleus `n`'s 2 × 2 pixels (RAD-A5, RAD-A8): its species' settled ink, or,
/// within [`FLASH`] s of a decay, the daughter's ramp eased from its top back down to the
/// settled level. A never flashes: nothing decays into it, and a restart is not a decay.
pub(crate) fn nucleus_ink(n: &Nucleus) -> Ink {
    let k = n.kind as usize;
    if n.kind == Kind::A || n.since >= FLASH {
        return settled_ink(n.kind);
    }
    let u = 1.0 - f64::from(n.since / FLASH);
    ink::ramp(RAMPS[k], SETTLED[k] + (1.0 - SETTLED[k]) * u * u)
}

/// Whether a nucleus is in the hot (bright) part of its flash (RAD-A8).
fn flashing(n: &Nucleus) -> bool {
    n.kind != Kind::A && n.since < FLASH_HOT
}

/// A charged-particle track in a cloud chamber with a uniform magnetic field: it turns at
/// `omega` and takes a fixed random heading kick of up to ±`scatter` every [`KINK`] s,
/// runs for `run` seconds (its range), then fades until `life`. The path depends only on
/// the track, not on the frame steps, so it is the same for any `dt`. Coordinates are
/// lattice pixels, which are square, so arcs are round.
#[derive(Debug, Clone, Copy)]
struct Track {
    x0: f64,
    y0: f64,
    theta: f64,
    speed: f64,
    omega: f64,
    scatter: f64,
    run: f64,
    life: f64,
    age: f64,
    /// Seeds the scattering kicks.
    key: u64,
    alpha: bool,
}

impl Track {
    /// Brightness factor: full while moving, then a linear fade.
    fn fade(&self) -> f64 {
        if self.age <= self.run {
            1.0
        } else {
            (1.0 - (self.age - self.run) / (self.life - self.run)).clamp(0.0, 1.0)
        }
    }

    /// The track's ink at its current age: its ramp near the top while moving, fading to
    /// [`TRACK_FLOOR`] of that.
    fn ink(&self) -> Ink {
        let ramp = if self.alpha { ALPHA_RAMP } else { BETA_RAMP };
        let f = TRACK_FLOOR + (1.0 - TRACK_FLOOR) * self.fade();
        ink::ramp(ramp, TRACK_TOP * f)
    }

    /// Call `f` with the path's corners (pixels) from the origin up to the distance flown
    /// so far: one per [`KINK`] s, the last one where the particle is now.
    fn walk(&self, mut f: impl FnMut(f64, f64)) {
        let flown = self.age.min(self.run);
        let (mut x, mut y, mut theta) = (self.x0, self.y0, self.theta);
        f(x, y);
        let mut s = 0.0;
        let mut k = 0u64;
        while s < flown {
            let ds = KINK.min(flown - s);
            let kick = self.scatter * (2.0 * hash01(self.key ^ k.wrapping_mul(0x9E37)) - 1.0);
            theta += self.omega * ds + kick;
            x += self.speed * ds * theta.cos_det();
            y += self.speed * ds * theta.sin_det();
            f(x, y);
            s += ds;
            k += 1;
        }
    }

    /// Where the particle is now (pixels).
    #[cfg(test)]
    fn head(&self) -> (f64, f64) {
        let mut p = (self.x0, self.y0);
        self.walk(|x, y| p = (x, y));
        p
    }
}

/// Dot position of pixel coordinates `(x, y)`, the centre of pixel `(x, y)`'s 2 × 2 dots
/// when both are integers: pixel `(x, y)` is cell `x`, half-row `y` (CORE-R64).
fn px_dot(x: f64, y: f64) -> (f64, f64) {
    Dots::from_cells(x + 0.5, (y + 0.5) / 2.0)
}

/// Draw `tr` over the lattice's `w × h` pixels, clipped to them: while young, a burst
/// ring around its nucleus; then its path as a braille stroke (α two dots thick), starting
/// a pixel out so the flash stays visible.
fn draw_track(dots: &mut Dots, tr: &Track, w: i32, h: i32) {
    if tr.fade() <= 0.0 {
        return;
    }
    let (dw, dh) = (f64::from(2 * w), f64::from(2 * h));
    let inside =
        |(x, y): (f64, f64)| (-0.5..dw - 0.5).contains(&x) && (-0.5..dh - 0.5).contains(&y);
    let (cx, cy) = px_dot(tr.x0, tr.y0);
    if tr.age < BURST {
        let u = tr.age / BURST;
        let r = BURST_R + BURST_GROW * u;
        let ring = ink::ramp(if tr.alpha { RAMPS[1] } else { RAMPS[2] }, 1.0 - 0.3 * u);
        for k in 0..12 {
            let a = TAU * f64::from(k) / 12.0;
            let p = (cx + r * a.cos_det(), cy + r * a.sin_det());
            if inside(p) {
                dots.plot(p.0, p.1, ring);
            }
        }
    }
    let ink = tr.ink();
    let mut prev: Option<(f64, f64)> = None;
    tr.walk(|x, y| {
        let p = px_dot(x, y);
        if !inside(p) || (x - tr.x0).hypot(y - tr.y0) < 1.5 {
            prev = None;
            return;
        }
        if let Some(q) = prev {
            dots.line(q.0, q.1, p.0, p.1, ink);
            if tr.alpha {
                // The second dot row, one dot across the heading.
                let (nx, ny) = (q.1 - p.1, p.0 - q.0);
                let n = nx.hypot(ny).max(1e-9);
                let (ox, oy) = (nx / n, ny / n);
                let (a, b) = ((q.0 + ox, q.1 + oy), (p.0 + ox, p.1 + oy));
                if inside(a) && inside(b) {
                    dots.line(a.0, a.1, b.0, b.1, ink);
                }
            }
        }
        prev = Some(p);
    });
}

/// ASCII for lattice cell `(x, y)` from the nuclei under its two pixels (GEN-R10): a hot
/// flash `*`, then B `o`, then A `-` (light glyphs, so the ground stays quiet); settled C
/// (near ground), gaps and off-lattice cells are blank, and no nucleus uses the braille
/// fallbacks
/// `.` `:` `+` `#`, so tracks never read as nuclei.
fn ascii_cell(lattice: &Grid<Nucleus>, geo: &LatticeGeo, x: i32, y: i32) -> Option<(char, Ink)> {
    let mut best: Option<&Nucleus> = None;
    let rank = |n: &Nucleus| match n.kind {
        _ if flashing(n) => 4,
        Kind::B => 3,
        Kind::A => 2,
        Kind::C => 1,
    };
    for py in [2 * y, 2 * y + 1] {
        if let Some(n) = geo.nucleus_at(x, py).and_then(|(i, j)| lattice.get(i, j))
            && best.is_none_or(|b| rank(n) > rank(b))
        {
            best = Some(n);
        }
    }
    let n = best?;
    let ch = match rank(n) {
        4 => '*',
        3 => 'o',
        2 => '-',
        _ => return None,
    };
    Some((ch, nucleus_ink(n)))
}

/// Where the nuclei sit on the lattice's pixels (RAD-A5): `nx × ny` squares of `NUC`
/// pixels on a pitch of `PITCH`, the block centred in the `lat_w × lat_h` pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LatticeGeo {
    nx: i32,
    ny: i32,
    ox: i32,
    oy: i32,
}

impl LatticeGeo {
    fn new(lat_w: i32, lat_h: i32) -> Self {
        let fit = |len: i32| ((len - NUC) / PITCH + 1).max(1);
        let (nx, ny) = (fit(lat_w), fit(lat_h));
        let span = |n: i32| PITCH * (n - 1) + NUC;
        Self {
            nx,
            ny,
            ox: ((lat_w - span(nx)) / 2).max(0),
            oy: ((lat_h - span(ny)) / 2).max(0),
        }
    }

    /// Top-left pixel of nucleus `(i, j)`.
    fn origin(&self, i: i32, j: i32) -> (i32, i32) {
        (self.ox + PITCH * i, self.oy + PITCH * j)
    }

    /// The nucleus covering pixel `(x, y)`, if any (gaps have none).
    fn nucleus_at(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (dx, dy) = (x - self.ox, y - self.oy);
        if dx < 0 || dy < 0 || dx % PITCH >= NUC || dy % PITCH >= NUC {
            return None;
        }
        let (i, j) = (dx / PITCH, dy / PITCH);
        (i < self.nx && j < self.ny).then_some((i, j))
    }
}

/// Whether the measured curve of species `k` is drawn at plot dot column `j`: A solid, B
/// dashed, C dotted, so the three read apart where their hues cannot (mono).
fn stroke_on(k: usize, j: usize) -> bool {
    match k {
        0 => true,
        1 => j % 6 < 4,
        // Pairs of dots, not single ones: a lone braille dot per column reads as faint.
        _ => j % 4 < 2,
    }
}

/// Write `n` in decimal at `(x, y)` without allocating, clipped at `x_max` (exclusive);
/// returns the column after the last digit.
fn put_num(canvas: &mut Canvas, x: i32, y: i32, n: u32, ink: Ink, x_max: i32) -> i32 {
    let mut text = StackText::<10>::new();
    // A u32 has at most 10 digits, so this never fails.
    let _ = write!(text, "{n}");
    let mut cx = x;
    for ch in text.as_str().chars() {
        if cx < x_max {
            canvas.put(cx, y, Glyph::ascii(ch), ink);
        }
        cx += 1;
    }
    cx
}

/// The plot panel on the right; all coordinates are canvas cells.
#[derive(Debug, Clone, Copy)]
struct Panel {
    /// Left edge (the y-axis column) and exclusive right edge.
    x0: i32,
    x1: i32,
    /// Plot rows `top..=bottom` hold data; the x axis is on `bottom + 1`, labels below it.
    top: i32,
    bottom: i32,
    /// Number of data columns, starting at `x0 + 1`.
    cols: usize,
}

impl Panel {
    fn new(w: i32, h: i32) -> Option<Self> {
        let pw = (f64::from(w) * PANEL_FRAC).round() as i32;
        (pw >= PANEL_MIN_W && h >= PANEL_MIN_H).then(|| {
            let x0 = w - pw;
            Self {
                x0,
                x1: w,
                top: if h >= 16 { 3 } else { 2 },
                bottom: h - 3,
                // One blank column of right margin.
                cols: (pw - 2) as usize,
            }
        })
    }

    /// Screen column of data column `c`.
    fn col_x(&self, c: usize) -> i32 {
        self.x0 + 1 + c as i32
    }

    /// Braille dot columns across the data columns (two per cell).
    fn dot_cols(&self) -> usize {
        2 * self.cols
    }

    /// Dot x of dot column `j`.
    fn dot_x(&self, j: usize) -> f64 {
        f64::from(Dots::CELL_W * (self.x0 + 1)) + j as f64
    }

    /// Dot y of plot height `y ∈ [0, 1]` (1 on the top data row's top dot).
    fn dot_y(&self, y: f64) -> f64 {
        let top = f64::from(Dots::CELL_H * self.top);
        let bottom = f64::from(Dots::CELL_H * (self.bottom + 1) - 1);
        bottom - y.clamp(0.0, 1.0) * (bottom - top)
    }
}

#[derive(Debug)]
pub struct RadioactiveDecay {
    /// Time since the current run started, s.
    t: f64,
    /// One nucleus per `NUC × NUC` pixel square on a `PITCH` grid (RAD-A5).
    lattice: Grid<Nucleus>,
    geo: LatticeGeo,
    /// Lattice size in pixels.
    lat_w: i32,
    lat_h: i32,
    counts: [u32; 3],
    n0: u32,
    panel: Option<Panel>,
    /// Plot time span, s.
    t_axis: f64,
    /// Measured `[N_A, N_B, N_C]` sampled at each plot dot column's time.
    hist: Vec<[u32; 3]>,
    tracks: ParticlePool<Track>,
    /// A uniform sample of the current step's decays, `(nucleus index, from, to)`, at most
    /// the track budget; reserved in `resize`.
    events: Vec<(u32, Kind, Kind)>,
    /// Completed runs (for tests / debugging).
    resets: u32,
    /// The nuclei as half-block pixels; the tracks and the plot's curves over it in `dots`.
    pixels: Pixels,
    dots: Dots,
}

impl Default for RadioactiveDecay {
    fn default() -> Self {
        Self {
            t: 0.0,
            lattice: Grid::default(),
            geo: LatticeGeo::default(),
            lat_w: 0,
            lat_h: 0,
            counts: [0; 3],
            n0: 0,
            panel: None,
            t_axis: 1.0,
            hist: Vec::new(),
            tracks: ParticlePool::with_capacity(0),
            events: Vec::new(),
            resets: 0,
            pixels: Pixels::default(),
            dots: Dots::default(),
        }
    }
}

impl RadioactiveDecay {
    #[cfg(test)]
    fn n0(&self) -> u32 {
        self.n0
    }

    /// Fresh run: every nucleus is A again.
    fn restart(&mut self) {
        self.lattice.fill(Nucleus::FRESH);
        self.counts = [self.n0, 0, 0];
        self.t = 0.0;
        self.hist.clear();
        self.tracks.clear();
        self.record();
    }

    /// Time of plot dot column `j` of `cols`.
    fn col_time(&self, j: usize, cols: usize) -> f64 {
        self.t_axis * j as f64 / (cols.max(2) - 1) as f64
    }

    /// Append measured samples for every plot dot column whose time has been reached.
    fn record(&mut self) {
        let Some(p) = self.panel else { return };
        let cols = p.dot_cols();
        while self.hist.len() < cols && self.col_time(self.hist.len(), cols) <= self.t + 1e-9 {
            self.hist.push(self.counts);
        }
    }

    /// Top-left lattice pixel of nucleus `i`.
    fn pixel_of(&self, i: usize) -> (i32, i32) {
        let gw = usize::from(self.lattice.width()).max(1);
        self.geo.origin((i % gw) as i32, (i / gw) as i32)
    }

    /// Give the free track slots to decays drawn uniformly from `events` (a partial
    /// Fisher–Yates shuffle), so a full pool favours no part of the lattice. Slots are
    /// counted per track: an `A → C` event needs two, and when only one is left it keeps
    /// its α and loses its β.
    fn spawn_tracks(&mut self, ctx: &mut Ctx) {
        let n = self.events.len();
        let mut free = self.tracks.capacity() - self.tracks.len();
        for k in 0..n {
            if free == 0 {
                break;
            }
            let j = ctx.rng.random_range(k..n);
            self.events.swap(k, j);
            let (i, from, to) = self.events[k];
            let (px, py) = self.pixel_of(i as usize);
            let alpha = from == Kind::A;
            let beta = to == Kind::C;
            // A → B: α; B → C: β⁻; A → C within one step: both.
            for is_alpha in [true, false] {
                if (is_alpha && !alpha) || (!is_alpha && !beta) || free == 0 {
                    continue;
                }
                free -= 1;
                let theta = ctx.rng.random::<f64>() * TAU;
                let (speed, omega, scatter, run, life) = if is_alpha {
                    (ALPHA_SPEED, ALPHA_OMEGA, 0.0, ALPHA_RUN, ALPHA_LIFE)
                } else {
                    let v = ctx.rng.random_range(BETA_SPEED.0..BETA_SPEED.1);
                    (v, BETA_OMEGA, BETA_SCATTER, BETA_RUN, BETA_LIFE)
                };
                // From the nucleus's centre, the corner its four pixels share.
                let half = 0.5 * f64::from(NUC - 1);
                self.tracks.spawn(Track {
                    x0: f64::from(px) + half,
                    y0: f64::from(py) + half,
                    theta,
                    speed,
                    omega,
                    scatter,
                    run,
                    life,
                    age: 0.0,
                    key: ctx.rng.random(),
                    alpha: is_alpha,
                });
            }
        }
    }

    /// Draw the nuclei, their tracks and the plot's curves (the only per-frame raster work).
    fn rasterise(&mut self) {
        self.pixels.clear();
        let gw = usize::from(self.lattice.width()).max(1);
        for (i, n) in self.lattice.as_slice().iter().enumerate() {
            let (x, y) = self.geo.origin((i % gw) as i32, (i / gw) as i32);
            let ink = nucleus_ink(n);
            for dy in 0..NUC {
                for dx in 0..NUC {
                    if x + dx < self.lat_w && y + dy < self.lat_h {
                        self.pixels.set(x + dx, y + dy, ink);
                    }
                }
            }
        }
        self.dots.clear();
        for tr in self.tracks.iter() {
            draw_track(&mut self.dots, tr, self.lat_w, self.lat_h);
        }
        if let Some(p) = self.panel {
            self.plot_curves(&p);
        }
    }

    /// Analytic Bateman curves as dim reference lines, then the measured counts as bright
    /// curves two dots thick in their species' inks and strokes, on the current axis blend
    /// (RAD-A6, RAD-A7).
    fn plot_curves(&mut self, p: &Panel) {
        let m = log_blend(self.t);
        let n0 = f64::from(self.n0.max(1));
        let cols = p.dot_cols();
        let analytic = Ink::new(Role::Structure, ANALYTIC_LEVEL);
        let mut prev: Option<[f64; 3]> = None;
        for j in 0..cols {
            let (a, b, c) = bateman(self.col_time(j, cols));
            let ys = [a, b, c].map(|f| p.dot_y(plot_y(f, m)));
            let x = p.dot_x(j);
            if let Some(py) = prev {
                for k in 0..3 {
                    self.dots.line(x - 1.0, py[k], x, ys[k], analytic);
                }
            }
            prev = Some(ys);
        }
        // The second dot row goes toward the plot's inside, so it never leaves the panel.
        let mid = p.dot_y(0.5);
        let thick = |y: f64| if y > mid { y - 1.0 } else { y + 1.0 };
        for k in [2, 1, 0] {
            let ink = ink::ramp(RAMPS[k], MEASURED);
            let mut prev: Option<f64> = None;
            for (j, h) in self.hist.iter().enumerate() {
                // Nothing left of a species has no place on a log axis.
                let y = (h[k] > 0 || m == 0.0).then(|| p.dot_y(plot_y(f64::from(h[k]) / n0, m)));
                if let (Some(y), true) = (y, stroke_on(k, j)) {
                    let x = p.dot_x(j);
                    match prev {
                        Some(py) if j > 0 && stroke_on(k, j - 1) => {
                            self.dots.line(x - 1.0, py, x, y, ink);
                            self.dots.line(x - 1.0, thick(py), x, thick(y), ink);
                        }
                        _ => {
                            self.dots.plot(x, y, ink);
                            self.dots.plot(x, thick(y), ink);
                        }
                    }
                }
                prev = y;
            }
        }
    }

    /// The y-axis ticks on the current axis blend: `(row, label)` (RAD-A6).
    fn y_ticks(&self, p: &Panel) -> impl Iterator<Item = (i32, &'static str)> {
        let m = log_blend(self.t);
        let ticks: &'static [(f64, &str)] = if m < 0.5 { &TICKS_LINEAR } else { &TICKS_LOG };
        ticks.iter().map(move |&(f, label)| {
            let row = (p.dot_y(plot_y(f, m)) / f64::from(Dots::CELL_H)).floor() as i32;
            (row.clamp(p.top, p.bottom), label)
        })
    }

    fn draw_panel(&self, canvas: &mut Canvas, p: &Panel) {
        let text = Ink::new(Role::Text, 200);
        // The time-axis labels can land on the title row, so they share its readout tone.
        let faint = Ink::new(Role::Text, title::READOUT_LEVEL);
        let axis = Ink::new(Role::Structure, 110);

        // Header and the live populations.
        for (k, g) in HEADER.iter().enumerate() {
            let x = p.x0 + k as i32;
            if x < p.x1 {
                canvas.put(x, 0, *g, text);
            }
        }
        let secs = self.t.max(0.0) as u32;
        let digits = if secs >= 10 { 2 } else { 1 };
        let tx = p.x1 - 2 - digits - 2;
        if tx > p.x0 + HEADER.len() as i32 + 1 {
            canvas.text(tx, 0, "t=", faint);
            let end = put_num(canvas, tx + 2, 0, secs, faint, p.x1);
            canvas.text(end, 0, "s", faint);
        }
        // Legend: each species as it looks on the lattice, then its curve's stroke in the
        // curve's ink, then the count (Text), one space on.
        // C is not drawn on the lattice, so its chip is a dim tile of its curve's ramp.
        let lattice_ink = [
            settled_ink(Kind::A),
            settled_ink(Kind::B),
            ink::ramp(RAMPS[2], C_CHIP),
        ];
        let mut x = p.x0;
        for k in 0..3 {
            if x + 2 >= p.x1 {
                break;
            }
            canvas.put(x, 1, MARKS[k], lattice_ink[k]);
            canvas.put(x + 1, 1, STROKES[k], ink::ramp(RAMPS[k], MEASURED));
            x = put_num(canvas, x + 3, 1, self.counts[k], text, p.x1) + 2;
        }

        // Axes: the y axis with ticks at N₀/2 (read the half-life off the A curve) on the
        // linear plot and at each decade on the log plot; the x axis with ticks every T_A.
        for y in p.top..=p.bottom {
            canvas.put(p.x0, y, SEG_V, axis);
        }
        for (row, label) in self.y_ticks(p) {
            canvas.put(p.x0, row, AXIS_TICK_Y, axis);
            canvas.text(p.col_x(0), row, label, faint);
        }
        let ax_row = p.bottom + 1;
        canvas.put(p.x0, ax_row, AXIS_CORNER, axis);
        for c in 0..p.cols {
            canvas.put(p.col_x(c), ax_row, SEG_H, axis);
        }
        let scale = (p.cols.max(2) - 1) as f64 / self.t_axis;
        let mut label_end = p.x0;
        let mut k = 0u32;
        loop {
            let t = f64::from(k) * T_A;
            let c = (t * scale).round() as usize;
            if c >= p.cols {
                break;
            }
            let x = p.col_x(c);
            if k > 0 {
                canvas.put(x, ax_row, AXIS_TICK_X, axis);
            }
            let v = t as u32;
            let w = if v >= 10 { 2 } else { 1 };
            let lx = x - (w - 1) / 2 - i32::from(k > 0 && w == 2);
            if lx > label_end && lx + w < p.x1 {
                label_end = put_num(canvas, lx, ax_row + 1, v, faint, p.x1);
            }
            k += 1;
        }
        if label_end + 1 < p.x1 {
            canvas.text(label_end + 1, ax_row + 1, "s", faint);
        }
    }
}

impl Generator for RadioactiveDecay {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(if log_blend(self.t) >= 0.5 {
            CAPTION_LOG
        } else {
            CAPTION_LINEAR
        })
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        self.panel = Panel::new(w, h);
        // Lattice to the left of the panel with one blank gutter column.
        self.lat_w = self.panel.map_or(w, |p| p.x0 - 2).max(1);
        // Row h − 1 belongs to the title while it shows (RAD-A4, GEN-R13). The longer
        // caption decides, so the lattice never changes with the act.
        let titled = title::span(size, &Title::new(META.title, Some(CAPTION_LINEAR))).is_some();
        let rows = if titled { h - 1 } else { h }.max(1);
        self.lat_h = 2 * rows;
        self.geo = LatticeGeo::new(self.lat_w, self.lat_h);
        let lattice = Size::new(self.geo.nx as u16, self.geo.ny as u16);
        self.lattice = Grid::new(lattice, Nucleus::FRESH);
        self.n0 = self.lattice.len() as u32;
        self.pixels.resize(size);
        self.dots.resize(size);

        self.t_axis = AXIS_SLACK * reset_time();
        let cols = self.panel.map_or(0, |p| p.dot_cols());
        self.hist = Vec::with_capacity(cols);
        // Peak decay rate is N₀ λ_A ≈ 0.087 N₀ /s; tracks are a sample of it, budgeted by
        // lattice area so their density on screen does not depend on the nucleus pitch.
        let area = (self.lat_w * self.lat_h).max(0) as usize;
        // Even, so a full step of `A → C` doubles fills it exactly (RAD-A8).
        let cap = (area / TRACKS_PER_PIXEL).clamp(4, 60) & !1;
        self.tracks.set_capacity(cap);
        self.events = Vec::with_capacity(cap);
        self.restart();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt > 0.0 && dt.is_finite()) {
            return;
        }
        let tr = Transition::new(dt);
        // Keys this step's reservoir draws: the closure cannot borrow `ctx.rng` while
        // `step_nuclei` does.
        let salt: u64 = ctx.rng.random();
        // A lattice sweep draws once per live nucleus: a SplitMix64 seeded from `ctx.rng`
        // keeps that cheap and deterministic (CORE-R57, RAD-A9).
        let mut draws = SplitMix64::from_rng(&mut ctx.rng);
        let counts = &mut self.counts;
        let events = &mut self.events;
        let cap = events.capacity();
        events.clear();
        let mut seen = 0u64;
        step_nuclei(
            self.lattice.as_mut_slice(),
            &tr,
            dt,
            &mut draws,
            |i, from, to| {
                counts[from as usize] -= 1;
                counts[to as usize] += 1;
                // Tracks are decoration: beyond the budget a decay just flashes. The kept
                // decays are a uniform sample (reservoir sampling, Algorithm R), not the
                // first rows scanned, so tracks start anywhere on the lattice.
                seen += 1;
                let e = (i as u32, from, to);
                if events.len() < cap {
                    events.push(e);
                } else {
                    let j = (hash01(salt ^ i as u64) * seen as f64) as usize;
                    if j < cap {
                        events[j] = e;
                    }
                }
            },
        );
        self.t += dt;

        self.tracks.retain(|tk| {
            tk.age += dt;
            tk.age < tk.life
        });
        self.spawn_tracks(ctx);
        self.record();

        let live = f64::from(self.counts[0] + self.counts[1]);
        if live < RESET_FRAC * f64::from(self.n0) {
            self.resets += 1;
            self.restart();
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        // ASCII has nothing finer than a cell, so there the lattice is drawn per cell
        // from the nuclei themselves.
        self.pixels.blit(canvas, |x, y, _, _| {
            ascii_cell(&self.lattice, &self.geo, x, y)
        });
        self.dots.blit_over(canvas, &self.pixels);
        if let Some(p) = self.panel {
            self.draw_panel(canvas, &p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(seed: u64) -> Ctx {
        Ctx::new(seed, Size::new(1, 1))
    }

    fn lam(t_half: f64) -> f64 {
        LN_2 / t_half
    }

    /// Binomial 4σ band check for `n` successes out of `n0` with probability `p`.
    fn within_4_sigma(n: u32, n0: u32, p: f64) -> (bool, f64, f64) {
        let mean = f64::from(n0) * p;
        let sd = (f64::from(n0) * p * (1.0 - p)).sqrt();
        // A 0.5 continuity allowance keeps p ≈ 0 or 1 (sd ≈ 0) from failing on rounding.
        ((f64::from(n) - mean).abs() <= 4.0 * sd + 0.5, mean, sd)
    }

    #[test]
    fn rad_a1_decay_probability_is_one_minus_two_pow_minus_dt_over_t() {
        assert_eq!(decay_p(0.0, T_A), 0.0);
        assert!((decay_p(T_A, T_A) - 0.5).abs() < 1e-15);
        assert!((decay_p(2.0 * T_B, T_B) - 0.75).abs() < 1e-15);
        for dt in [1e-6, SIM_DT, 0.1, 0.25, 1.0, 5.0] {
            for t in [T_A, T_B] {
                let want = 1.0 - 2f64.powf(-dt / t);
                assert!((decay_p(dt, t) - want).abs() < 1e-15, "dt={dt} T={t}");
            }
        }
        // Degenerate input never yields a non-probability.
        for (dt, t) in [
            (-1.0, T_A),
            (f64::NAN, T_A),
            (1.0, 0.0),
            (f64::INFINITY, T_A),
        ] {
            let p = decay_p(dt, t);
            assert!((0.0..=1.0).contains(&p), "dt={dt} T={t} p={p}");
        }
    }

    #[test]
    fn rad_a1_decay_probability_composes_exactly() {
        // Survival multiplies: (1 − p(a))(1 − p(b)) = 1 − p(a + b).
        for a in [SIM_DT, 0.05, 0.25, 1.7] {
            for b in [SIM_DT, 0.2, 3.0] {
                for t in [T_A, T_B] {
                    let lhs = (1.0 - decay_p(a, t)) * (1.0 - decay_p(b, t));
                    let rhs = 1.0 - decay_p(a + b, t);
                    assert!((lhs - rhs).abs() < 1e-15, "a={a} b={b} T={t}");
                }
            }
        }
        // 60 frames of 1/60 s are exactly one second.
        let s: f64 = (0..60).map(|_| 1.0 - decay_p(SIM_DT, T_A)).product();
        assert!((s - (1.0 - decay_p(1.0, T_A))).abs() < 1e-14);
    }

    /// Row-stochastic 3×3 matrix of the chain over `h`, built from [`Transition`].
    fn matrix(h: f64) -> [[f64; 3]; 3] {
        let tr = Transition::new(h);
        let (pa, pb) = (tr.stay_a, tr.a_to_b);
        [
            [pa, pb, 1.0 - pa - pb],
            [0.0, tr.stay_b, 1.0 - tr.stay_b],
            [0.0, 0.0, 1.0],
        ]
    }

    #[test]
    fn rad_a1_chain_transition_obeys_chapman_kolmogorov() {
        for (h1, h2) in [(SIM_DT, SIM_DT), (0.1, 0.25), (2.0, 5.0), (8.0, 3.0)] {
            let (m1, m2, m12) = (matrix(h1), matrix(h2), matrix(h1 + h2));
            for i in 0..3 {
                for j in 0..3 {
                    let prod: f64 = (0..3).map(|k| m1[i][k] * m2[k][j]).sum();
                    assert!(
                        (prod - m12[i][j]).abs() < 1e-14,
                        "h1={h1} h2={h2} [{i}][{j}]"
                    );
                }
                let row: f64 = m12[i].iter().sum();
                assert!((row - 1.0).abs() < 1e-15);
                assert!(m12[i].iter().all(|p| (0.0..=1.0).contains(p)));
            }
        }
        // The A row over t *is* the Bateman solution; the B row is plain decay.
        let tr = Transition::new(5.0);
        let (fa, fb, _) = bateman(5.0);
        assert!((tr.stay_a - fa).abs() < 1e-15 && (tr.a_to_b - fb).abs() < 1e-15);
        assert!((tr.stay_b - (1.0 - decay_p(5.0, T_B))).abs() < 1e-15);
    }

    #[test]
    fn rad_a1_sample_partitions_the_unit_interval() {
        let tr = Transition::new(4.0);
        let (pa, pb) = (tr.stay_a, tr.a_to_b);
        assert_eq!(tr.sample(Kind::A, 0.0), Kind::A);
        assert_eq!(tr.sample(Kind::A, pa * 0.999), Kind::A);
        assert_eq!(tr.sample(Kind::A, pa + 0.5 * pb), Kind::B);
        assert_eq!(tr.sample(Kind::A, (pa + pb) * 1.001), Kind::C);
        assert_eq!(tr.sample(Kind::B, tr.stay_b * 0.999), Kind::B);
        assert_eq!(tr.sample(Kind::B, tr.stay_b * 1.001), Kind::C);
        assert_eq!(tr.sample(Kind::C, 0.999), Kind::C);
        // dt = 0 is the identity.
        let id = Transition::new(0.0);
        for u in [0.0, 0.5, 0.999_999] {
            assert_eq!(id.sample(Kind::A, u), Kind::A);
            assert_eq!(id.sample(Kind::B, u), Kind::B);
        }
    }

    #[test]
    fn rad_a1_one_long_step_samples_a_to_c_exactly() {
        // A single 6 s step: a naive "one hop per step" sampler would give N_C = 0 here.
        let n0 = 20_000u32;
        let mut cells = vec![Nucleus::FRESH; n0 as usize];
        let mut ctx = seeded(11);
        let mut double = 0;
        step_nuclei(
            &mut cells,
            &Transition::new(6.0),
            6.0,
            &mut ctx.rng,
            |_, f, t| {
                if (f, t) == (Kind::A, Kind::C) {
                    double += 1;
                }
            },
        );
        let n = census(&cells);
        let (fa, fb, fc) = bateman(6.0);
        for (k, p) in [(0, fa), (1, fb), (2, fc)] {
            let (ok, mean, sd) = within_4_sigma(n[k], n0, p);
            assert!(ok, "species {k}: {} vs {mean:.1} ± {sd:.1}", n[k]);
        }
        assert_eq!(double, n[2], "every C came straight from A in one step");
    }

    #[test]
    fn rad_a2_bateman_solves_the_rate_equations() {
        let (la, lb) = (lam(T_A), lam(T_B));
        let (fa, fb, fc) = bateman(0.0);
        assert_eq!((fa, fb, fc), (1.0, 0.0, 0.0));
        assert!((bateman(T_A).0 - 0.5).abs() < 1e-15);
        // Residuals of dA = −λ_A A, dB = λ_A A − λ_B B, dC = λ_B B (central differences).
        let e = 1e-5;
        for t in [0.3, 2.0, 5.0, 11.0, 30.0] {
            let (a0, b0, c0) = bateman(t - e);
            let (a1, b1, c1) = bateman(t + e);
            let (a, b, c) = bateman(t);
            assert!(((a1 - a0) / (2.0 * e) + la * a).abs() < 1e-8, "t={t}");
            assert!(
                ((b1 - b0) / (2.0 * e) - (la * a - lb * b)).abs() < 1e-8,
                "t={t}"
            );
            assert!(((c1 - c0) / (2.0 * e) - lb * b).abs() < 1e-8, "t={t}");
            assert!((a + b + c - 1.0).abs() < 1e-15);
        }
        // N_B peaks at t* = ln(λ_B/λ_A)/(λ_B − λ_A), where λ_A A = λ_B B.
        let ts = (lb / la).ln() / (lb - la);
        let (a, b, _) = bateman(ts);
        assert!((la * a - lb * b).abs() < 1e-15);
    }

    /// Seeded 20 000-nucleus run with a varying frame step, checked once per second.
    fn run_rad_a2(seed: u64, dts: &[f64]) {
        let n0 = 20_000u32;
        let mut cells = vec![Nucleus::FRESH; n0 as usize];
        let mut ctx = seeded(seed);
        let mut t = 0.0;
        let mut next_check = 1.0;
        let mut i = 0;
        while t < 20.0 {
            let dt = dts[i % dts.len()];
            i += 1;
            step_nuclei(
                &mut cells,
                &Transition::new(dt),
                dt,
                &mut ctx.rng,
                |_, _, _| {},
            );
            t += dt;
            if t + 1e-9 >= next_check {
                next_check += 1.0;
                let n = census(&cells);
                assert_eq!(n.iter().sum::<u32>(), n0, "RAD-A3");
                let (fa, fb, _) = bateman(t);
                let (ok_a, ma, sa) = within_4_sigma(n[0], n0, fa);
                let (ok_b, mb, sb) = within_4_sigma(n[1], n0, fb);
                assert!(ok_a, "t={t:.2} N_A={} vs {ma:.1} ± {sa:.1}", n[0]);
                assert!(ok_b, "t={t:.2} N_B={} vs {mb:.1} ± {sb:.1}", n[1]);
            }
        }
    }

    #[test]
    fn rad_a2_seeded_20000_nucleus_run_follows_bateman_within_4_sigma() {
        // Each nucleus is independent, so N_A(t), N_B(t) are exactly binomial with the
        // Bateman fractions — the 4σ band is the binomial standard deviation.
        // Uneven steps from the frame step up to the GEN-R4 maximum: because the transition
        // is exact, the step size must not matter. 20 s covers the N_B peak (6.8 s) and
        // 2.5 half-lives of A; checked once per simulated second (20 × 2 binomial checks).
        run_rad_a2(2024, &[0.25, SIM_DT, 0.17, 0.25, 0.05, 0.25]);
    }

    #[test]
    fn rad_a9_the_scene_itself_follows_bateman_within_4_sigma() {
        let size = Size::new(200, 50);
        let mut ctx = seeded(42);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let n0 = g.n0();
        assert!(n0 > 1_000, "{n0} nuclei");
        for step in 1..=(20.0 / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
            if step % 60 == 0 {
                let (fa, fb, _) = bateman(g.t);
                let (ok_a, ma, sa) = within_4_sigma(g.counts[0], n0, fa);
                let (ok_b, mb, sb) = within_4_sigma(g.counts[1], n0, fb);
                assert!(
                    ok_a,
                    "t={:.1} N_A={} vs {ma:.1} ± {sa:.1}",
                    g.t, g.counts[0]
                );
                assert!(
                    ok_b,
                    "t={:.1} N_B={} vs {mb:.1} ± {sb:.1}",
                    g.t, g.counts[1]
                );
            }
        }
    }

    #[test]
    fn rad_a3_populations_always_sum_to_n0() {
        let mut ctx = seeded(5);
        let mut g = RadioactiveDecay::default();
        for (k, size) in [Size::new(90, 24), Size::new(7, 3), Size::new(200, 50)]
            .into_iter()
            .enumerate()
        {
            g.resize(size, &mut ctx);
            for i in 0..240 {
                let dt = [SIM_DT, 0.25, 0.0, 0.11][(i + k) % 4];
                g.update(dt, &mut ctx);
                assert_eq!(g.counts.iter().sum::<u32>(), g.n0(), "RAD-A3");
                assert_eq!(g.counts, census(g.lattice.as_slice()));
            }
        }
    }

    #[test]
    fn rad_reset_below_two_percent_restarts_all_a() {
        // Analytic reset time ≈ 50.6 s: f_A + f_B = 2 %.
        let tr = reset_time();
        let (a, b, _) = bateman(tr);
        assert!((a + b - RESET_FRAC).abs() < 1e-12 && (45.0..56.0).contains(&tr));

        let mut ctx = seeded(9);
        let mut g = RadioactiveDecay::default();
        g.resize(Size::new(100, 30), &mut ctx);
        let n0 = f64::from(g.n0());
        for _ in 0..(90.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
            // Invariant after every update: never left below the threshold (RAD-A3 too).
            let live = f64::from(g.counts[0] + g.counts[1]);
            assert!(live >= RESET_FRAC * n0);
            assert_eq!(g.counts.iter().sum::<u32>(), g.n0());
            if g.resets > 0 && g.t == 0.0 {
                assert_eq!(g.counts, [g.n0(), 0, 0]);
            }
        }
        assert!(g.resets >= 1, "a ~3 000-nucleus run resets by ~51 s");
    }

    #[test]
    fn rad_panel_only_when_it_gets_24x10_cells() {
        let mut ctx = seeded(1);
        let mut g = RadioactiveDecay::default();
        g.resize(Size::new(100, 30), &mut ctx);
        let p = g.panel.expect("100×30 has a plot panel");
        assert!(p.x1 - p.x0 >= PANEL_MIN_W);
        // The lattice stays left of the panel with a gutter.
        let last = g.pixel_of(g.lattice.len() - 1);
        assert!(last.0 < p.x0 - 1 && g.lat_w < p.x0);
        for size in [Size::new(50, 30), Size::new(100, 9), Size::new(1, 1)] {
            g.resize(size, &mut ctx);
            assert!(g.panel.is_none(), "{size:?}");
            assert!(g.n0() >= 1);
        }
    }

    #[test]
    fn rad_tracks_stay_in_the_lattice_and_under_budget() {
        let size = Size::new(120, 36);
        let mut ctx = seeded(4);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let mut seen = 0;
        for _ in 0..120 {
            g.update(0.25, &mut ctx);
            assert!(g.tracks.len() <= g.tracks.capacity());
            seen += g.tracks.len();
            // Tracks are braille strokes clipped to the lattice: no pixel right of it or
            // below it is lit, and every dot left of the plot lies over the lattice.
            for y in 0..72 {
                for x in 0..120 {
                    if x >= g.lat_w || y >= g.lat_h {
                        assert_eq!(g.pixels.get(x, y), Some(Ink::BLANK), "({x},{y})");
                    }
                }
            }
            let panel_x = g.panel.map_or(i32::MAX, |p| p.x0);
            let dots = g.dots.dot_size();
            for y in 0..i32::from(dots.height()) {
                for x in 0..i32::from(dots.width()) {
                    if g.dots.is_lit(x, y) && x / Dots::CELL_W < panel_x {
                        assert!(
                            x / Dots::CELL_W < g.lat_w && y / 2 < g.lat_h,
                            "dot ({x},{y})"
                        );
                    }
                }
            }
        }
        assert!(seen > 0, "decays emit tracks");
    }

    #[test]
    fn rad_a4_the_title_row_is_left_to_the_title() {
        let mut ctx = seeded(6);
        for (w, h) in [(80u16, 24u16), (120, 36), (200, 50), (40, 5), (7, 3)] {
            let size = Size::new(w, h);
            let mut g = RadioactiveDecay::default();
            g.resize(size, &mut ctx);
            let titled = title::span(size, &Title::new(META.title, Some(CAPTION_LINEAR))).is_some();
            let rows = if titled { h - 1 } else { h };
            assert_eq!(g.lat_h, 2 * i32::from(rows), "{w}x{h}");
            let last = i32::from(h) - 1;
            for _ in 0..40 {
                g.update(0.25, &mut ctx);
                let panel = g.panel.take();
                let mut canvas = Canvas::new(size, GlyphMode::Ascii);
                g.render(&mut canvas);
                g.panel = panel;
                if titled {
                    let row = &canvas.to_lines()[last as usize];
                    assert!(row.trim().is_empty(), "{w}x{h}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn rad_a5_each_nucleus_is_a_spaced_2x2_square_in_its_species_ink() {
        let size = Size::new(100, 30);
        let mut ctx = seeded(3);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let p = g.panel.unwrap();
        // Lattice pixels lat_w × 2·29 (29 rows above the title row), nuclei on a pitch of 3.
        assert_eq!((g.lat_w, g.lat_h), (p.x0 - 2, 58));
        assert_eq!(
            (i32::from(g.lattice.width()), i32::from(g.lattice.height())),
            ((g.lat_w - NUC) / PITCH + 1, (58 - NUC) / PITCH + 1)
        );
        assert_eq!(g.n0() as usize, g.lattice.len());
        for _ in 0..240 {
            g.update(SIM_DT, &mut ctx);
        }
        g.rasterise();
        // Every nucleus fills its 2×2 pixels with its ink; every gap pixel is empty.
        let mut seen = [0; 3];
        for (i, n) in g.lattice.as_slice().iter().enumerate() {
            let (x, y) = g.pixel_of(i);
            seen[n.kind as usize] += 1;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                assert_eq!(g.pixels.get(x + dx, y + dy), Some(nucleus_ink(n)));
            }
            if n.since >= FLASH {
                assert_eq!(nucleus_ink(n), settled_ink(n.kind));
            }
        }
        assert!(seen.iter().all(|&n| n > 30), "{seen:?}");
        for y in 0..g.lat_h {
            for x in 0..g.lat_w {
                if g.geo.nucleus_at(x, y).is_none() {
                    assert_eq!(g.pixels.get(x, y), Some(Ink::BLANK), "gap ({x},{y})");
                }
            }
        }
        // A flash starts at the top of the daughter's ramp; fresh A (a restart) never flashes.
        let at = |kind, since| Nucleus { kind, since };
        assert_eq!(nucleus_ink(&Nucleus::FRESH), settled_ink(Kind::A));
        for kind in [Kind::B, Kind::C] {
            let flash = nucleus_ink(&at(kind, 0.0));
            assert_eq!(
                (flash.role, flash.level),
                (Role::Ramp(RAMPS[kind as usize]), 255)
            );
        }
        // Unicode draws half blocks and braille; ASCII mode is pure ASCII (GEN-R10), a
        // hot flash `*` over B `o` over A `-`; settled C and gaps blank.
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        assert!(matches!(
            c.get(0, 0).unwrap().ch,
            '▀' | '▄' | '\u{2800}'..='\u{28FF}' | ' '
        ));
        let mut a = Canvas::new(size, GlyphMode::Ascii);
        g.tracks.clear();
        g.rasterise();
        g.render(&mut a);
        assert!(a.to_lines().iter().all(|l| l.is_ascii()));
        for y in 0..29 {
            for x in 0..g.lat_w {
                let ch = a.get(x, y).unwrap().ch;
                let ns: Vec<&Nucleus> = [2 * y, 2 * y + 1]
                    .iter()
                    .filter_map(|&py| g.geo.nucleus_at(x, py))
                    .filter_map(|(i, j)| g.lattice.get(i, j))
                    .collect();
                let has = |k| ns.iter().any(|n| n.kind == k);
                let want = match () {
                    _ if ns.iter().any(|n| flashing(n)) => '*',
                    _ if has(Kind::B) => 'o',
                    _ if has(Kind::A) => '-',
                    _ => ' ',
                };
                assert_eq!(ch, want, "({x},{y})");
            }
        }
    }

    #[test]
    fn rad_a10_species_read_apart_and_the_lattice_fades_as_it_decays() {
        // A and B are the lattice's two lit species, at 0.6–0.7 and a step apart so mono
        // parts them; stable C sinks to the ground (≤ 0.08), so a decayed lattice is dark
        // and only flashes and tracks carry brightness.
        let lv = [Kind::A, Kind::B, Kind::C].map(|k| f64::from(settled_ink(k).level) / 255.0);
        assert!(
            (0.58..=0.72).contains(&lv[0]) && (0.58..=0.72).contains(&lv[1]),
            "{lv:?}"
        );
        assert!(lv[0] + 0.08 <= lv[1], "{lv:?}");
        assert!(lv[2] <= 0.08, "C is near ground: {lv:?}");
        // A warm, B teal, C a faint violet whose curve is lavender: no amber ramps and no
        // grey measured curve anywhere in the scene.
        assert_eq!(RAMPS, [Ramp::HAlpha, Ramp::Ice, Ramp::Plasma]);
        assert!(
            !RAMPS.contains(&Ramp::Starlight),
            "only the theory curves are grey"
        );
        for r in RAMPS.iter().chain(&[ALPHA_RAMP, BETA_RAMP]) {
            assert!(!matches!(r, Ramp::Ember | Ramp::Sulfur), "{r:?}");
        }
        // The plot's curves are bright: ≥ 0.8 of their species' ramps.
        const { assert!(MEASURED >= 0.8) };
    }

    /// A lattice cell at or above this level reads as an event, not as ground (RAD-A8).
    const BRIGHT_LEVEL: u8 = 215;

    #[test]
    fn rad_a8_the_ground_is_dim_and_only_events_are_bright() {
        // A flash eases from the top of the ramp back to the settled ink over FLASH s: it is
        // bright only while hot, and it only ever dims.
        let b = |since: f32| {
            nucleus_ink(&Nucleus {
                kind: Kind::B,
                since,
            })
            .level
        };
        assert_eq!(b(0.0), 255);
        assert!(b(FLASH_HOT) < BRIGHT_LEVEL && b(FLASH_HOT * 0.5) >= BRIGHT_LEVEL);
        assert_eq!(b(FLASH), settled_ink(Kind::B).level);
        assert!((1..30).all(|k| b(FLASH * k as f32 / 30.0) <= b(FLASH * (k - 1) as f32 / 30.0)));
        const { assert!(FLASH >= 0.25, "long enough for the eye to follow") };
        // Tracks stand clear of the settled nuclei: fresh ≥ 1.2× B's (the brightest
        // species), and at every age above A's.
        let ground_a = settled_ink(Kind::A).level;
        let ground_b = settled_ink(Kind::B).level;
        let mut ctx = seeded(2);
        let mut g = RadioactiveDecay::default();
        g.resize(Size::new(120, 40), &mut ctx);
        while g.tracks.len() < 2 || g.tracks.iter().all(|t| t.alpha) {
            g.update(SIM_DT, &mut ctx);
        }
        for tr in g.tracks.iter() {
            let fresh = Track { age: 0.0, ..*tr }.ink().level;
            let faded = Track {
                age: tr.life - 1e-6,
                ..*tr
            }
            .ink()
            .level;
            assert!(f64::from(fresh) >= 1.2 * f64::from(ground_b), "{fresh}");
            assert!(faded > ground_a, "{faded}");
        }
        // β tracks are longer than α tracks, every one of them.
        let range = |alpha: bool, speed: f64| {
            let tr = Track {
                x0: 0.0,
                y0: 0.0,
                theta: 0.3,
                speed,
                omega: if alpha { ALPHA_OMEGA } else { BETA_OMEGA },
                scatter: if alpha { 0.0 } else { BETA_SCATTER },
                run: if alpha { ALPHA_RUN } else { BETA_RUN },
                life: 1.0,
                age: 1.0,
                key: 77,
                alpha,
            };
            let mut len = 0.0;
            let mut prev = (0.0, 0.0);
            tr.walk(|x, y| {
                len += (x - prev.0).hypot(y - prev.1);
                prev = (x, y);
            });
            len
        };
        let alpha = range(true, ALPHA_SPEED);
        assert!((alpha - ALPHA_SPEED * ALPHA_RUN).abs() < 1e-9);
        assert!(range(false, BETA_SPEED.0) > 1.9 * alpha);
        // The α runs straight; the β is knocked about.
        let a = Track {
            x0: 0.0,
            y0: 0.0,
            theta: 0.0,
            speed: ALPHA_SPEED,
            omega: ALPHA_OMEGA,
            scatter: 0.0,
            run: ALPHA_RUN,
            life: 1.0,
            age: 1.0,
            key: 1,
            alpha: true,
        };
        let (hx, hy) = a.head();
        assert!(hy.atan2(hx).abs() < 0.1 && (hx.hypot(hy) - alpha).abs() < 0.05);
        // Over a whole run at 120×40 fewer than 5 % of the lattice cells are bright on
        // average (and never more than 10 %), tracks, bursts and flashes included.
        let size = Size::new(120, 40);
        let mut ctx = seeded(9);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let (w, rows) = (g.lat_w, g.lat_h / 2);
        let cells = f64::from(w * rows);
        let (mut sum, mut worst, mut frames) = (0.0, 0.0f64, 0);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        for i in 0..(50.0 / 0.1) as usize {
            g.update(if i % 2 == 0 { SIM_DT } else { 0.1 }, &mut ctx);
            c.clear();
            g.render(&mut c);
            let bright = (0..rows)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let cell = c.get(x, y).unwrap();
                    cell.ink.level >= BRIGHT_LEVEL || cell.bg.level >= BRIGHT_LEVEL
                })
                .count();
            let f = bright as f64 / cells;
            sum += f;
            worst = worst.max(f);
            frames += 1;
        }
        let mean = sum / f64::from(frames);
        assert!(
            mean < 0.05 && worst < 0.1 && mean > 0.0,
            "mean {mean:.3}, worst {worst:.3}"
        );
        // Settled nuclei are never bright: only events are.
        assert!(SETTLED.iter().all(|&v| ink::level(v) < BRIGHT_LEVEL));
        // Tracks are in hues no species uses.
        for ramp in [ALPHA_RAMP, BETA_RAMP] {
            assert!(!RAMPS.contains(&ramp));
        }
        assert_ne!(ALPHA_RAMP, BETA_RAMP);
    }

    #[test]
    fn rad_a8_tracked_decays_are_spread_over_the_whole_lattice() {
        // At 200×50 the track budget is full most of the time, so which decays get a slot
        // decides where tracks start: over 20 s their rows must centre on the lattice and
        // fill every third of it, not hug the first rows scanned.
        let mut ctx = seeded(42);
        let mut g = RadioactiveDecay::default();
        g.resize(Size::new(200, 50), &mut ctx);
        let h = f64::from(g.lat_h);
        let mut rows = Vec::new();
        for _ in 0..(20.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            // Tracks spawned this step have not aged yet.
            rows.extend(g.tracks.iter().filter(|t| t.age == 0.0).map(|t| t.y0 / h));
        }
        let n = rows.len() as f64;
        assert!(n > 500.0, "{n} tracks");
        let mean = rows.iter().sum::<f64>() / n;
        assert!(
            (mean - 0.5).abs() < 0.05,
            "mean row {mean:.3} of the height"
        );
        for third in 0..3 {
            let lo = f64::from(third) / 3.0;
            let share = rows
                .iter()
                .filter(|&&y| (lo..lo + 1.0 / 3.0).contains(&y))
                .count();
            assert!(share as f64 >= 0.2 * n, "third {third}: {share} of {n}");
        }
    }

    #[test]
    fn rad_a8_the_lattice_fades_out_as_it_decays() {
        // Settled C is not drawn, so late in a run most sites are dark ground: the mean
        // level of the nucleus pixels falls well below its fresh value.
        let size = Size::new(120, 40);
        let mut ctx = seeded(5);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let mean = |g: &RadioactiveDecay| {
            let n = g.lattice.len();
            let sum: f64 = (0..n)
                .map(|i| {
                    let (x, y) = g.pixel_of(i);
                    let px = g.pixels.get(x, y).unwrap();
                    let n = g.lattice.as_slice()[i];
                    if n.kind == Kind::C && n.since >= FLASH {
                        assert_eq!(px, Ink::BLANK, "settled C {i} is not drawn");
                    } else {
                        assert_ne!(px, Ink::BLANK, "nucleus {i} is drawn");
                    }
                    if px == Ink::BLANK {
                        0.0
                    } else {
                        f64::from(px.level)
                    }
                })
                .sum();
            sum / n as f64
        };
        let fresh = mean(&g);
        assert_eq!(fresh, f64::from(settled_ink(Kind::A).level));
        for _ in 0..(30.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let c = f64::from(g.counts[2]) / f64::from(g.n0());
        assert!(
            c > 0.6 && mean(&g) < 0.4 * fresh,
            "C {c:.2}, mean {}",
            mean(&g)
        );
    }

    #[test]
    fn rad_a5_nuclei_are_separate_dots_on_a_quiet_ground() {
        // At t = 0 every nucleus is an A square; neighbouring squares are one blank pixel
        // apart in both directions and the block is centred in the lattice.
        let size = Size::new(120, 40);
        let mut ctx = seeded(4);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let geo = g.geo;
        assert!(geo.nx >= 20 && geo.ny >= 20, "{geo:?}");
        let (x0, y0) = geo.origin(0, 0);
        let (x1, y1) = geo.origin(geo.nx - 1, geo.ny - 1);
        assert!((x0 - (g.lat_w - (x1 + NUC))).abs() <= 1, "centred across");
        assert!((y0 - (g.lat_h - (y1 + NUC))).abs() <= 1, "centred down");
        for j in 0..geo.ny {
            for i in 0..geo.nx {
                let (x, y) = geo.origin(i, j);
                assert_eq!(g.pixels.get(x, y), Some(settled_ink(Kind::A)));
                assert_eq!(g.pixels.get(x + NUC, y), Some(Ink::BLANK));
                assert_eq!(g.pixels.get(x, y + NUC), Some(Ink::BLANK));
            }
        }
        // Nuclei cover about 4/9 of the lattice's pixels: most of the ground stays black.
        let lit = (0..g.lat_h)
            .flat_map(|y| (0..g.lat_w).map(move |x| (x, y)))
            .filter(|&(x, y)| g.pixels.get(x, y) != Some(Ink::BLANK))
            .count() as f64;
        let share = lit / f64::from(g.lat_w * g.lat_h);
        assert!((0.35..0.5).contains(&share), "{share}");
    }

    #[test]
    fn rad_a8_legend_shows_each_species_as_on_the_lattice_and_its_stroke() {
        let size = Size::new(120, 36);
        let mut ctx = seeded(1);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let p = g.panel.unwrap();
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let row: String = (p.x0..p.x1).map(|x| c.get(x, 1).unwrap().ch).collect();
        assert!(row.starts_with("█─ "), "{row:?}");
        assert!(row.contains("█╌ 0") && row.contains("█┈ 0"), "{row:?}");
        // The A chip wears the lattice's ink, its stroke the curve's.
        assert_eq!(c.get(p.x0, 1).unwrap().ink, settled_ink(Kind::A));
        assert_eq!(
            c.get(p.x0 + 1, 1).unwrap().ink,
            ink::ramp(RAMPS[0], MEASURED)
        );
        // Strokes: A solid, B dashed, C dotted, all different.
        let pattern = |k| (0..12).map(|j| stroke_on(k, j)).collect::<Vec<_>>();
        assert!(pattern(0).iter().all(|&on| on));
        assert!(pattern(1) != pattern(0) && pattern(2) != pattern(1) && pattern(2) != pattern(0));
        // The panel is at most 30 % of the width.
        assert!(f64::from(p.x1 - p.x0) <= 0.3 * 120.0 + 0.5);
    }

    #[test]
    fn rad_a6_the_log_act_straightens_every_exponential() {
        // Linear first, then eased to log over LOG_EASE s from LOG_FROM.
        assert_eq!(log_blend(0.0), 0.0);
        assert_eq!(log_blend(LOG_FROM), 0.0);
        assert_eq!(log_blend(LOG_FROM + LOG_EASE), 1.0);
        assert!((log_blend(LOG_FROM + 0.5 * LOG_EASE) - 0.5).abs() < 1e-12);
        for f in [0.0, 0.2, 0.5, 1.0] {
            assert!((plot_y(f, 0.0) - f).abs() < 1e-15, "linear axis");
        }
        assert_eq!(plot_y(1.0, 1.0), 1.0);
        assert!(
            (plot_y(0.1, 1.0) - 2.0 / 3.0).abs() < 1e-12,
            "one decade = 1/3"
        );
        assert_eq!(plot_y(1e-4, 1.0), 0.0);
        assert_eq!(plot_y(0.0, 1.0), 0.0);
        assert_eq!(plot_y(f64::NAN, 0.5), 0.0);
        // On the log axis N_A is a straight line of slope −λ_A/(DECADES ln 10) …
        let slope = |f: &dyn Fn(f64) -> f64, t: f64| {
            (plot_y(f(t + 0.5), 1.0) - plot_y(f(t - 0.5), 1.0)) / 1.0
        };
        let fa = |t: f64| bateman(t).0;
        let fb = |t: f64| bateman(t).1;
        let want = -lam(T_A) / (DECADES * std::f64::consts::LN_10);
        for t in [2.0, 10.0, 25.0, 40.0] {
            assert!((slope(&fa, t) - want).abs() < 1e-9, "A at t={t}");
        }
        // … and once B is in transient equilibrium (T_B < T_A) its line runs parallel.
        for t in [36.0, 44.0] {
            assert!((slope(&fb, t) / want - 1.0).abs() < 0.02, "B at t={t}");
        }
        // The caption follows the axis, and a new run starts linear again.
        let mut ctx = seeded(8);
        let mut g = RadioactiveDecay::default();
        g.resize(Size::new(100, 30), &mut ctx);
        let mut seen = [false; 2];
        for _ in 0..(60.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
            let want = if g.t >= LOG_FROM + 0.5 * LOG_EASE {
                CAPTION_LOG
            } else {
                CAPTION_LINEAR
            };
            assert_eq!(g.caption(), Some(want), "t = {}", g.t);
            seen[usize::from(want == CAPTION_LOG)] = true;
        }
        assert!(g.resets >= 1 && seen == [true, true]);
        assert!(META.captions.contains(&CAPTION_LINEAR) && META.captions.contains(&CAPTION_LOG));
    }

    #[test]
    fn rad_a6_y_ticks_are_labelled_as_fractions_of_n0() {
        // Linear plot: `50%` on the N₀/2 tick; log plot: `10%` and `1%` on the decades.
        // Labels sit just inside the axis on their tick's row, over the curves.
        let size = Size::new(120, 36);
        let mut ctx = seeded(6);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let p = g.panel.unwrap();
        let text_at = |g: &RadioactiveDecay, want: &[&str]| {
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            // Only the panel text: no curves over the labels.
            let bare = RadioactiveDecay {
                t: g.t,
                panel: g.panel,
                t_axis: g.t_axis,
                ..RadioactiveDecay::default()
            };
            bare.render(&mut c);
            for (row, label) in g.y_ticks(&p) {
                let got: String = (0..label.len() as i32)
                    .map(|k| c.get(p.col_x(0) + k, row).unwrap().ch)
                    .collect();
                assert_eq!(got, label);
                assert_eq!(c.get(p.x0, row).unwrap().ch, '+', "tick at row {row}");
            }
            let labels: Vec<_> = g.y_ticks(&p).map(|(_, l)| l).collect();
            assert_eq!(labels, want);
        };
        text_at(&g, &["50%"]);
        for _ in 0..(25.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
        }
        text_at(&g, &["10%", "1%"]);
        // Decades land a third of the plot apart on the log axis.
        let rows: Vec<_> = g.y_ticks(&p).map(|(r, _)| r).collect();
        let third = f64::from(p.bottom - p.top + 1) / 3.0;
        assert!(
            (f64::from(rows[1] - rows[0]) - third).abs() <= 1.0,
            "{rows:?}"
        );
    }

    #[test]
    fn rad_a8_track_slots_are_counted_per_track() {
        // Every event in a full step is an A → C double decay (two tracks each): the pool
        // fills to exactly its capacity, and the last event keeps its α when one slot is left.
        let mut ctx = seeded(7);
        let mut g = RadioactiveDecay::default();
        g.resize(Size::new(120, 40), &mut ctx);
        let cap = g.tracks.capacity();
        assert!(cap % 2 == 0 && cap >= 4);
        g.tracks.clear();
        // One pre-existing track leaves an odd number of free slots.
        g.events.clear();
        g.events.push((0, Kind::A, Kind::B));
        g.spawn_tracks(&mut ctx);
        assert_eq!(g.tracks.len(), 1);
        g.events.clear();
        for i in 0..cap as u32 {
            g.events.push((i, Kind::A, Kind::C));
        }
        g.spawn_tracks(&mut ctx);
        assert_eq!(g.tracks.len(), cap);
        let alphas = g.tracks.iter().filter(|t| t.alpha).count();
        // 1 + (cap − 1) slots: ⌈(cap − 1)/2⌉ doubles, the last with only its α.
        assert_eq!(alphas, 1 + (cap - 1).div_ceil(2));
    }

    #[test]
    fn rad_a7_measured_points_sit_on_the_axis_blend_in_their_species_ink() {
        let size = Size::new(120, 36);
        let mut ctx = seeded(12);
        let mut g = RadioactiveDecay::default();
        g.resize(size, &mut ctx);
        let p = g.panel.unwrap();
        for steps in [600, 900] {
            for _ in 0..steps {
                g.update(SIM_DT, &mut ctx);
            }
            let m = log_blend(g.t);
            let n0 = f64::from(g.n0());
            for (j, h) in g.hist.iter().enumerate() {
                for (k, &count) in h.iter().enumerate() {
                    if (count == 0 && m > 0.0) || !stroke_on(k, j) {
                        continue;
                    }
                    let y = p.dot_y(plot_y(f64::from(count) / n0, m)).round() as i32;
                    let x = p.dot_x(j) as i32;
                    assert!(g.dots.is_lit(x, y), "series {k} column {j}");
                }
            }
            // Curves stay inside the plot's data rows and columns (dots left of the panel
            // are tracks over the lattice).
            let dots = g.dots.dot_size();
            for y in 0..i32::from(dots.height()) {
                for x in Dots::CELL_W * p.x0..i32::from(dots.width()) {
                    if g.dots.is_lit(x, y) {
                        let (cx, cy) = (x / Dots::CELL_W, y / Dots::CELL_H);
                        assert!(cx > p.x0 && cx < p.x1 && (p.top..=p.bottom).contains(&cy));
                    }
                }
            }
        }
        // The measured cells wear the species ramps at the measured value.
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let ramps = (p.top..=p.bottom)
            .flat_map(|y| (p.x0 + 1..p.x1).map(move |x| (x, y)))
            .filter_map(|(x, y)| c.get(x, y))
            .filter(|cell| matches!(cell.ink.role, Role::Ramp(_)))
            .count();
        assert!(ramps > 20, "{ramps} measured cells");
    }
}
