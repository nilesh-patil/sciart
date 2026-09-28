//! Twin double pendulums released 10⁻³ rad apart: an aurora-green twin and an Hα-crimson
//! twin, superposed as one two-tone pendulum while they are closer than a braille cell
//! (dashed rods and trail alternating the hues, crimson rims around the green bobs), then
//! visibly two, each lower bob tracing a 3 s fading path. A fan of ghost pendulums, all
//! started inside the 10⁻³ rad gap between the twins, is drawn as the material line joining
//! their lower bobs; chaos stretches and folds it across the whole swing (SPEC-002
//! `double_pendulum`).
//!
//! Everything is drawn on a braille [`Dots`] lattice rasterised in `update`/`resize` from
//! model-space state, so a resize keeps the trails; `render` only blits and writes the
//! readout.
//!
//! Physics: point masses `m₁, m₂` on massless rigid rods `L₁, L₂`, angles `θ₁, θ₂` measured
//! from the downward vertical. The state `(θ₁, θ₂, ω₁, ω₂)` obeys the exact Lagrangian
//! equations of motion ([`deriv`]) and is advanced with classical RK4 (`ode::integrate`) at
//! `h ≤ 1/240 s` (DP-A1). Bob positions are always derived from the angles ([`bobs`]), so
//! the rods are rigid by construction (DP-A4).
//!
//! Units: model lengths are in units of `L₁`; `g` is chosen so that the small-angle period of
//! the upper arm alone, `2π√(L₁/g)`, is 2.2 s. The dynamics are therefore independent of the
//! terminal size — a resize only rescales the view and never disturbs the run.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::{ASPECT, wrap_angle};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, ink, ode, raster};

pub static META: Meta = Meta {
    name: "double_pendulum",
    category: Category::Physics,
    description: "Twin double pendulums diverging chaotically from almost identical starts",
    title: "Double pendulum · twins diverge",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(DoublePendulum::default())
}

/// Physical parameters of one double pendulum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Params {
    pub m1: f64,
    pub m2: f64,
    pub l1: f64,
    pub l2: f64,
    pub g: f64,
}

/// Small-angle period of the upper arm on its own, s (spec: ≈ 2.2 s).
const UPPER_PERIOD: f64 = 2.2;
const SCENE_L1: f64 = 1.0;

/// The on-screen pendulum: a slightly heavier upper bob and a slightly shorter lower arm
/// (asymmetry makes the motion less regular-looking than the textbook 1:1:1:1 case).
pub(crate) const SCENE: Params = Params {
    m1: 1.2,
    m2: 1.0,
    l1: SCENE_L1,
    l2: 0.9,
    // g = L₁ (2π / T)², so that 2π√(L₁/g) = T.
    g: SCENE_L1 * (TAU / UPPER_PERIOD) * (TAU / UPPER_PERIOD),
};

/// Largest RK4 substep, s (spec: h ≤ 1/240 s).
pub(crate) const H_MAX: f64 = 1.0 / 240.0;
/// Initial offset of the twin's lower-arm angle, rad (spec).
pub(crate) const TWIN_OFFSET: f64 = 1e-3;
/// Each run lasts this long before restarting from new seeded angles, s (spec).
pub(crate) const RUN_SECONDS: f64 = 45.0;
/// Trail sampling tick and length: 3 s of lower-bob path at 120 Hz (DP-A7).
const TICK: f64 = 1.0 / 120.0;
pub(crate) const TRAIL_LEN: usize = 360;
/// Ghost pendulums whose starts fill the gap between the twins (DP-A6).
pub(crate) const GHOSTS: usize = 96;
/// Links in the ghost line: twin A → 96 ghosts → twin B is 98 samples.
const LINKS: usize = GHOSTS + 1;
/// Largest frame step honoured; bounds the per-frame cost whatever the caller passes.
const MAX_DT: f64 = 0.25;
/// Slack for accumulated-time comparisons: summing 2700 frame steps of 1/60 s misses
/// 45 s by ~1e-12, which must still count as "the run is over".
const TIME_SLACK: f64 = 1e-9;
/// While the twins' matching bobs have been closer than `MERGE_DOTS` on screen they share
/// braille cells (a cell is 2×4 dots and shows one ink), so they are drawn superposed:
/// dashes alternating the two hues, and twin B's bobs as the right half of a disc laid on
/// twin A's, so each bob is half green, half crimson. Twin B's disc grows back to whole
/// between `MERGE_DOTS` and `SPLIT_DOTS` (DP-A5).
const MERGE_DOTS: f64 = 4.0;
const SPLIT_DOTS: f64 = 8.0;
/// Dash length along a rod, dots, and along the trail, samples. Each dash is drawn with
/// one twin on top, alternating, so coinciding twins read as a two-tone stripe rather
/// than as whichever twin was drawn last (DP-A5).
const ROD_DASH: f64 = 6.0;
const TRAIL_DASH: usize = 4;

/// Twin A is aurora green, twin B Hα crimson: near-complementary false colours, and a
/// pair no other dynamics scene uses (DP-A5).
const RAMPS: [Ramp; 2] = [Ramp::Aurora, Ramp::HAlpha];
/// The glow under the line art (DP-A10): each twin's trail, rods and bobs expose a
/// half-block [`Glow`] of their own, rebuilt every frame from the model-space trail (so a
/// resize keeps it), tone-mapped by an asinh curve and drawn in the twin's ramp at up to
/// `GLOW_V`, under the trail's top value, so the lines stay the brightest thing in their
/// cells and the glow reads as light spilling from them. Each pixel takes the ramp of the
/// twin that exposed it more. Unicode only: ASCII has no half blocks.
const GLOW_V: f64 = 0.4;
const GLOW_KNEE: f32 = 0.4;
const GLOW_WHITE: f32 = 5.0;
/// Glow exposure per pixel of path: the trail by freshness², the rods evenly; a bob adds a
/// wide halo (`HALO`) at `BOB_GLOW`.
const TRAIL_GLOW: f32 = 0.45;
const ROD_GLOW: f32 = 0.4;
const BOB_GLOW: f32 = 2.5;
/// The bobs' radius-3 halo; strokes use the shared 3×3 `raster::STROKE_KERNEL` (CORE-R96).
const HALO: [(i32, i32, f32); 49] = raster::halo_kernel(3);
const _: () = assert!(GLOW_V < TRAIL_V);

/// Ramp values of the parts of a twin. The pendulum outshines its own trail (capped at
/// `TRAIL_V`), and the bodies sit in the saturated middle of the ramps (above ~0.8 both
/// wash out towards white, and the two-tone superposition with them); each bob gets a
/// one-dot core at 0.9 as a highlight (M12: lifted from 0.62 / 0.68 / 0.8 / 0.56 so the arms
/// read as lit over their glow).
const ROD_V: f64 = 0.66;
const BOB_V: f64 = 0.72;
const CORE_V: f64 = 0.9;
/// Trail ramp value: `TRAIL_FLOOR` at the tail up to `TRAIL_V` at the newest segment.
const TRAIL_FLOOR: f64 = 0.12;
const TRAIL_V: f64 = 0.6;
/// The pivot hinge, always visible.
const PIVOT_LEVEL: u8 = 230;
// The pendulum outshines its own trail, and its core outshines its body (DP-A5).
const _: () = assert!(TRAIL_V < ROD_V && ROD_V < BOB_V && BOB_V < CORE_V);
/// Bob and pivot radii, dots; the lower bob grows on tall terminals, where a 2.8-dot disc
/// reads as a speckle rather than a ball.
const UPPER_R: f64 = 1.8;
const LOWER_R: f64 = 2.8;
const LOWER_R_TALL: f64 = 3.5;
const TALL_ROWS: u16 = 40;
const PIVOT_R: f64 = 1.0;
/// The ghost line at full fade-in, as a ramp value: under the twins and their trails.
const GHOST_V: f64 = 0.45;
/// Ghost line segments longer than this (model units) are left out: where the line has
/// been stretched further than its samples resolve, a chord would be a straight streak
/// across the swing, not the material line (DP-A6, stated in SPEC-002).
const GHOST_LINK: f64 = 0.35;
/// The ghost line fades out as it breaks up: fully shown while at least 70 % of its links
/// are kept, gone at 30 % or fewer (smoothstep between), so a line stretched past its
/// samples dissolves instead of salting the frame with lone beads (DP-A6).
const GHOST_KEPT: (f64, f64) = (0.3, 0.7);

/// Right-hand side of the exact Lagrangian equations of motion (DP-A1).
///
/// With `Δ = θ₁ − θ₂` and `D = 2m₁ + m₂ − m₂ cos 2Δ` (≥ 2m₁ > 0, so never singular):
/// - `θ₁'' = [−g(2m₁+m₂) sin θ₁ − m₂ g sin(θ₁ − 2θ₂) − 2 sin Δ · m₂ (ω₂² L₂ + ω₁² L₁ cos Δ)] / (L₁ D)`
/// - `θ₂'' = [2 sin Δ (ω₁² L₁ (m₁+m₂) + g (m₁+m₂) cos θ₁ + ω₂² L₂ m₂ cos Δ)] / (L₂ D)`
pub(crate) fn deriv(p: &Params, y: &[f64; 4]) -> [f64; 4] {
    let [th1, th2, w1, w2] = *y;
    let Params { m1, m2, l1, l2, g } = *p;
    let delta = th1 - th2;
    let (sd, cd) = delta.sin_cos_det();
    let den = 2.0 * m1 + m2 - m2 * (2.0 * delta).cos_det();
    let a1 = (-g * (2.0 * m1 + m2) * th1.sin_det()
        - m2 * g * (th1 - 2.0 * th2).sin_det()
        - 2.0 * sd * m2 * (w2 * w2 * l2 + w1 * w1 * l1 * cd))
        / (l1 * den);
    let a2 = 2.0
        * sd
        * (w1 * w1 * l1 * (m1 + m2) + g * (m1 + m2) * th1.cos_det() + w2 * w2 * l2 * m2 * cd)
        / (l2 * den);
    [w1, w2, a1, a2]
}

/// Total mechanical energy measured from the resting configuration, so it is ≥ 0 and
/// vanishes only at rest (a safe denominator for relative drift, DP-A1).
///
/// `T = ½(m₁+m₂)L₁²ω₁² + ½m₂L₂²ω₂² + m₂L₁L₂ω₁ω₂ cos Δ`,
/// `V = (m₁+m₂) g L₁ (1 − cos θ₁) + m₂ g L₂ (1 − cos θ₂)`.
pub(crate) fn energy(p: &Params, y: &[f64; 4]) -> f64 {
    let [th1, th2, w1, w2] = *y;
    let Params { m1, m2, l1, l2, g } = *p;
    let m = m1 + m2;
    let kinetic = 0.5 * m * l1 * l1 * w1 * w1
        + 0.5 * m2 * l2 * l2 * w2 * w2
        + m2 * l1 * l2 * w1 * w2 * (th1 - th2).cos_det();
    // 1 − cos θ = 2 sin²(θ/2) keeps small-angle energies accurate (no cancellation).
    let lift = |th: f64| 2.0 * (0.5 * th).sin_det().powi(2);
    let potential = m * g * l1 * lift(th1) + m2 * g * l2 * lift(th2);
    kinetic + potential
}

/// Positions of the upper and lower bobs relative to the pivot, model units, y down
/// (DP-A4): always derived from the angles, so both rods are rigid by construction.
pub(crate) fn bobs(p: &Params, th1: f64, th2: f64) -> [(f64, f64); 2] {
    let (s1, c1) = th1.sin_cos_det();
    let (s2, c2) = th2.sin_cos_det();
    let b1 = (p.l1 * s1, p.l1 * c1);
    [b1, (b1.0 + p.l2 * s2, b1.1 + p.l2 * c2)]
}

/// The widest each readout form (long, mid, short) can be for any separation in
/// `[0, π√2]`, start separation `1e-3` and finite energy drift (`dp_gen_r13_form_widths_
/// bound_every_value`). `render` picks a form by this width, not its current length, so
/// the choice never flips as the numbers change width.
const FORM_WIDTHS: [usize; 3] = [84, 34, 14];
/// Front padding for the readout forms.
const PAD: &str =
    "                                                                                        ";

/// The readout forms, unpadded: long
/// `|dtheta| 3.1e-2 rad = 31.0x start (3.4 e-folds)   energy drift 2e-9   t 12/45s`, mid
/// `|dtheta| 3.1e-2 (3.4 e-folds)` (no drift: its width varies most), short `e-folds 3.4`.
pub(crate) fn write_forms(out: &mut [String; 3], sep: f64, sep0: f64, drift: f64, t: f64) {
    let growth = sep / sep0.max(1e-300);
    let folds = growth.max(1e-300).ln();
    let [long, mid, short] = out;
    long.clear();
    mid.clear();
    short.clear();
    let _ = write!(
        long,
        "|dtheta| {sep:.1e} rad = {growth:.1}x start ({folds:.1} e-folds)   \
         energy drift {drift:.0e}   t {t:.0}/{RUN_SECONDS:.0}s"
    );
    let _ = write!(mid, "|dtheta| {sep:.1e} ({folds:.1} e-folds)");
    let _ = write!(short, "e-folds {folds:.1}");
}

/// Angular separation of two states, `√(Δθ₁² + Δθ₂²)` with each difference wrapped to
/// `[−π, π)` (DP-A3) — a full turn apart is the same configuration.
pub(crate) fn separation(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    wrap_angle(a[0] - b[0]).hypot(wrap_angle(a[1] - b[1]))
}

/// Advance one pendulum by `dt` seconds of RK4 substeps no longer than [`H_MAX`].
pub(crate) fn step(p: &Params, y: &mut [f64; 4], dt: f64) {
    ode::integrate(y, 0.0, dt, H_MAX, |_, s| deriv(p, s));
}

/// A seeded high-energy release from rest: both arms above the horizontal, each on a
/// random side, with variety in how close to inverted they start (spec).
pub(crate) fn seeded_start(rng: &mut impl Rng) -> [f64; 4] {
    let mut arm = |lo: f64, hi: f64| {
        let side = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
        side * PI * rng.random_range(lo..hi)
    };
    let th1 = arm(0.55, 0.92);
    let th2 = arm(0.52, 0.97);
    [th1, th2, 0.0, 0.0]
}

/// How far the twins have split (DP-A5), from the most their matching bobs have been
/// apart this run, in dots: 0 up to `MERGE_DOTS`, 1 from `SPLIT_DOTS`, smoothstepped
/// between. It only grows during a run, so twins that later cross are never drawn superposed again.
pub(crate) fn split(apart_dots: f64) -> f64 {
    ease::smoothstep((apart_dots - MERGE_DOTS) / (SPLIT_DOTS - MERGE_DOTS))
}

/// Ghost line visibility (DP-A6): it fades in with the twins' split, since before that
/// it lies under them, and out as its links break (`kept` = fraction of links joined).
pub(crate) fn ghost_fade(split: f64, kept: f64) -> f64 {
    let (lo, hi) = GHOST_KEPT;
    split * ease::smoothstep((kept - lo) / (hi - lo))
}

/// Ghost `k`'s start (DP-A6): twin A's, with the lower arm turned `(k + 1)/(GHOSTS + 1)` of
/// the way to twin B's, so the fan spans exactly the gap between the twins.
pub(crate) fn ghost_start(a: &[f64; 4], k: usize) -> [f64; 4] {
    let mut y = *a;
    y[1] += TWIN_OFFSET * (k + 1) as f64 / (GHOSTS + 1) as f64;
    y
}

/// Screen mapping: pivot at `(cx, cy)` cells, `scale` rows per model unit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct View {
    cx: f64,
    cy: f64,
    scale: f64,
}

impl View {
    /// Pivot centred in `rows × cols`, scaled so the full reach `L₁ + L₂` in every direction
    /// stays inside (aspect-corrected) with half a row to spare for the lower bob's disc.
    /// The rounded extremes stay within `0` and `size − 1`.
    fn fit(p: &Params, cols: f64, rows: f64) -> Self {
        let (cx, cy) = (((cols - 1.0) / 2.0).max(0.0), ((rows - 1.0) / 2.0).max(0.0));
        let reach = p.l1 + p.l2;
        let scale = ((cy - 0.5).min((cx - 0.5) / ASPECT) / reach).max(0.0);
        Self { cx, cy, scale }
    }

    /// Model point (y down) → fractional cell coordinates.
    #[inline]
    fn map(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (self.cx + self.scale * x * ASPECT, self.cy + self.scale * y)
    }

    /// Model point → dot coordinates (CORE-R64); the cell centre of a point is its
    /// fractional cell coordinate plus ½.
    #[inline]
    fn dot(&self, p: (f64, f64)) -> (f64, f64) {
        let (x, y) = self.map(p);
        Dots::from_cells(x + 0.5, y + 0.5)
    }

    /// Dots per model unit, the same both ways: `scale` rows of 4 dots vertically,
    /// `scale · ASPECT` columns of 2 dots horizontally.
    fn dots_per_unit(&self) -> f64 {
        self.scale * f64::from(Dots::CELL_H)
    }
}

/// Whether neighbouring ghost bobs `a`, `b` (model units) are close enough to join (DP-A6).
pub(crate) fn ghost_link(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).hypot(a.1 - b.1) <= GHOST_LINK
}

/// A part of twin `i` at ramp value `v`: twin A aurora, twin B Hα (DP-A5).
fn twin_ink(i: usize, v: f64) -> Ink {
    ink::ramp(RAMPS[i], v)
}

/// Which twin is drawn on top in dash `k`: they alternate, so neither wins every cell
/// they share (DP-A5).
fn dash_order(k: usize) -> [usize; 2] {
    if k % 2 == 0 { [1, 0] } else { [0, 1] }
}

/// Both twins' segments `from[i] → to[i]` (dots), cut into matching dashes of about
/// `ROD_DASH` dots, the twins alternating on top dash by dash (DP-A5).
fn dashed(dots: &mut Dots, from: [(f64, f64); 2], to: [(f64, f64); 2], v: f64) {
    let len = |i: usize| (to[i].0 - from[i].0).hypot(to[i].1 - from[i].1);
    let span = len(0).max(len(1));
    // A rod is at most the screen's dot diagonal long; the cap only guards NaN or ∞.
    let n = if span.is_finite() {
        (span / ROD_DASH).ceil().clamp(1.0, 4096.0) as usize
    } else {
        1
    };
    let at = |i: usize, u: f64| {
        (
            from[i].0 + u * (to[i].0 - from[i].0),
            from[i].1 + u * (to[i].1 - from[i].1),
        )
    };
    for k in 0..n {
        let (u0, u1) = (k as f64 / n as f64, (k + 1) as f64 / n as f64);
        for i in dash_order(k) {
            let (p, q) = (at(i, u0), at(i, u1));
            dots.line(p.0, p.1, q.0, q.1, twin_ink(i, v));
        }
    }
}

/// The dots of the disc of radius `r` about `c` (dots) that lie right of `c.x − cut`:
/// the whole disc for `cut ≥ r`, its right half for `cut = 0` (DP-A5). Lattice-clipped
/// by [`Dots::set`]; a non-finite centre draws nothing.
fn clipped_disc(dots: &mut Dots, c: (f64, f64), r: f64, cut: f64, ink: Ink) {
    if !(c.0.is_finite() && c.1.is_finite()) {
        return;
    }
    let (x0, x1) = ((c.0 - r.min(cut)).ceil(), (c.0 + r).floor());
    let (y0, y1) = ((c.1 - r).ceil(), (c.1 + r).floor());
    let mut y = y0;
    while y <= y1 {
        let mut x = x0;
        while x <= x1 {
            if (x - c.0).hypot(y - c.1) <= r {
                dots.set(x as i32, y as i32, ink);
            }
            x += 1.0;
        }
        y += 1.0;
    }
}

/// A trail sample: both twins' lower bobs, model units.
#[derive(Debug, Clone, Copy)]
struct Sample {
    lows: [(f64, f64); 2],
}

#[derive(Debug)]
pub struct DoublePendulum {
    /// The twins: A (aurora) and B (Hα), 10⁻³ rad apart in `θ₂`.
    twins: [[f64; 4]; 2],
    /// The ghost fan between them (DP-A6).
    ghosts: [[f64; 4]; GHOSTS],
    started: bool,
    /// Time since the current release, s.
    run_t: f64,
    tick_clock: f64,
    /// Energy of each twin at release (DP-A1 readout).
    e0: [f64; 2],
    /// Largest relative energy drift seen this run.
    drift: f64,
    /// Current and initial angular separation (DP-A3 readout).
    sep: f64,
    sep0: f64,
    /// Farthest the twins' matching bobs have been apart this run, model units (DP-A5).
    apart: f64,
    /// Lower-bob positions of both twins, oldest → newest (DP-A7).
    trail: Trail<Sample>,
    /// Trail samples pushed this run, so each keeps its dash as the trail scrolls.
    pushed: usize,
    view: View,
    label_row: Option<i32>,
    /// Lower bob radius for the current size, dots.
    lower_r: f64,
    /// Everything but the readout, rasterised in `update`/`resize`.
    dots: Dots,
    /// Each twin's glow exposure and the toned half-block layer under the dots (DP-A10).
    glow: [Glow; 2],
    halo: Pixels,
    tone: Tone,
    /// Readouts in decreasing length; render picks the longest that fits.
    labels: [String; 3],
}

impl Default for DoublePendulum {
    fn default() -> Self {
        Self {
            twins: [[0.0; 4]; 2],
            ghosts: [[0.0; 4]; GHOSTS],
            started: false,
            run_t: 0.0,
            tick_clock: 0.0,
            e0: [0.0; 2],
            drift: 0.0,
            sep: 0.0,
            sep0: TWIN_OFFSET,
            apart: 0.0,
            trail: Trail::new(TRAIL_LEN),
            pushed: 0,
            view: View::default(),
            label_row: None,
            lower_r: LOWER_R,
            dots: Dots::default(),
            glow: [Glow::default(), Glow::default()],
            halo: Pixels::default(),
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
            labels: std::array::from_fn(|_| String::with_capacity(96)),
        }
    }
}

impl DoublePendulum {
    /// Release both twins and the ghost fan from a new seeded start.
    fn release(&mut self, ctx: &mut Ctx) {
        let a = seeded_start(&mut ctx.rng);
        let mut b = a;
        b[1] += TWIN_OFFSET;
        self.twins = [a, b];
        self.ghosts = std::array::from_fn(|k| ghost_start(&a, k));
        self.e0 = self.twins.map(|y| energy(&SCENE, &y));
        self.drift = 0.0;
        self.sep0 = separation(&a, &b);
        self.sep = self.sep0;
        self.apart = 0.0;
        self.run_t = 0.0;
        self.tick_clock = 0.0;
        self.trail.clear();
        self.pushed = 0;
        self.push_trail();
        self.started = true;
        self.refresh_labels();
    }

    /// Record the trail sample for now, after widening `apart` to the twins' current gap.
    fn push_trail(&mut self) {
        let [a, b] = self.twins.map(|y| bobs(&SCENE, y[0], y[1]));
        let gap = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1);
        self.apart = self.apart.max(gap(a[0], b[0])).max(gap(a[1], b[1]));
        self.trail.push(Sample { lows: [a[1], b[1]] });
        self.pushed += 1;
    }

    fn rel_drift(&self) -> f64 {
        let rel =
            |i: usize| (energy(&SCENE, &self.twins[i]) - self.e0[i]).abs() / self.e0[i].max(1e-12);
        rel(0).max(rel(1))
    }

    fn refresh_labels(&mut self) {
        write_forms(
            &mut self.labels,
            self.sep,
            self.sep0,
            self.drift,
            self.run_t,
        );
        for (label, width) in self.labels.iter_mut().zip(FORM_WIDTHS) {
            // Padded at the front, so the readout's left edge holds still while its
            // numbers change width; capacity 96 ≥ every width, so this never reallocates.
            let pad = width.saturating_sub(label.len());
            label.insert_str(0, &PAD[..pad.min(PAD.len())]);
        }
    }

    /// Draw everything onto `dots`, dimmest first; a cell keeps its brightest dot (CORE-R41).
    fn rasterise(&mut self) {
        self.dots.clear();
        let view = self.view;
        let split_of = |apart: f64| split(apart * view.dots_per_unit());
        let parted = split_of(self.apart);

        // Ghost fan: the material line of starts between the twins, joined in start order
        // from twin A's lower bob through every ghost's to twin B's (A's colour for the
        // first half, B's for the second), with a one-dot bead per ghost.
        let lower = |y: &[f64; 4]| bobs(&SCENE, y[0], y[1])[1];
        let mut line = [(0.0, 0.0); LINKS + 1];
        line[0] = lower(&self.twins[0]);
        line[LINKS] = lower(&self.twins[1]);
        for (p, y) in line[1..LINKS].iter_mut().zip(&self.ghosts) {
            *p = lower(y);
        }
        let kept = line.windows(2).filter(|l| ghost_link(l[0], l[1])).count();
        let fade = ghost_fade(parted, kept as f64 / LINKS as f64);
        if 255.0 * GHOST_V * fade >= ink::MIN_VISIBLE_LEVEL {
            for (k, l) in line.windows(2).enumerate() {
                let ink = ink::ramp(RAMPS[2 * k / LINKS], GHOST_V * fade);
                let (d0, d1) = (view.dot(l[0]), view.dot(l[1]));
                if ghost_link(l[0], l[1]) {
                    self.dots.line(d0.0, d0.1, d1.0, d1.1, ink);
                }
                if k < GHOSTS {
                    self.dots.plot(d1.0, d1.1, ink);
                }
            }
        }

        // Trails: an aurora path and an Hα path, in dashes of `TRAIL_DASH` segments that
        // alternate which twin is on top, so the pair reads two-tone while they coincide.
        let n = self.trail.len();
        let now = Sample {
            lows: [line[0], line[LINKS]],
        };
        let newer = self.trail.iter().skip(1).chain([&now]);
        for (k, (old, new)) in self.trail.iter().zip(newer).enumerate() {
            // Age 0 = newest segment, → 1 at the tail.
            let age = n - 1 - k;
            let fresh = 1.0 - age as f64 / TRAIL_LEN as f64;
            let v = TRAIL_FLOOR + (TRAIL_V - TRAIL_FLOOR) * fresh * fresh;
            // Dashes by the sample's run-wide index, so each stays on its stretch of path.
            for i in dash_order((self.pushed.saturating_sub(n) + k) / TRAIL_DASH) {
                let (d0, d1) = (view.dot(old.lows[i]), view.dot(new.lows[i]));
                self.dots.line(d0.0, d0.1, d1.0, d1.1, twin_ink(i, v));
            }
        }

        // The twins: dashed rods, then bobs with a bright core. Twin A's discs go first,
        // then twin B's on top, cut to their right half while the twins coincide (a
        // half-green, half-crimson bob) and whole again once they have parted.
        let arms = self
            .twins
            .map(|y| bobs(&SCENE, y[0], y[1]).map(|p| view.dot(p)));
        let pivot = view.dot((0.0, 0.0));
        let [[a1, a2], [b1, b2]] = arms;
        dashed(&mut self.dots, [pivot, pivot], [a1, b1], ROD_V);
        dashed(&mut self.dots, [a1, b1], [a2, b2], ROD_V);
        for (i, [p1, p2]) in arms.into_iter().enumerate() {
            for (p, r) in [(p1, UPPER_R), (p2, self.lower_r)] {
                // Twin B keeps the dots right of `p.x − cut`: its right half while the
                // twins coincide, the whole disc once parted; twin A is always whole.
                let cut = if i == 0 { r } else { 2.0 * r * parted - 0.5 };
                clipped_disc(&mut self.dots, p, r, cut, twin_ink(i, BOB_V));
                self.dots.plot(p.0, p.1, twin_ink(i, CORE_V));
            }
        }
        self.dots
            .disc(pivot.0, pivot.1, PIVOT_R, Ink::new(Role::Text, PIVOT_LEVEL));
        self.expose();
    }

    /// Rebuild the glow under the line art from the trail, rods and bobs (DP-A10). The
    /// trail lives in model space, so this is exact after a resize too.
    fn expose(&mut self) {
        let view = self.view;
        let n = self.trail.len();
        for g in &mut self.glow {
            g.clear();
        }
        let arms = self.twins.map(|y| bobs(&SCENE, y[0], y[1]));
        let now = arms.map(|a| a.map(|p| view.dot(p)));
        // The newest trail segment joins the bobs' current positions, as the trail does.
        let head = Sample {
            lows: [arms[0][1], arms[1][1]],
        };
        let newer = self.trail.iter().skip(1).chain([&head]);
        for (k, (old, new)) in self.trail.iter().zip(newer).enumerate() {
            let age = n - 1 - k;
            let fresh = 1.0 - age as f32 / TRAIL_LEN as f32;
            for (i, glow) in self.glow.iter_mut().enumerate() {
                let (a, b) = (view.dot(old.lows[i]), view.dot(new.lows[i]));
                glow.stroke(a, b, &raster::STROKE_KERNEL, TRAIL_GLOW * fresh * fresh);
            }
        }
        let pivot = view.dot((0.0, 0.0));
        for (glow, [b1, b2]) in self.glow.iter_mut().zip(now) {
            glow.stroke(pivot, b1, &raster::STROKE_KERNEL, ROD_GLOW);
            glow.stroke(b1, b2, &raster::STROKE_KERNEL, ROD_GLOW);
            glow.splat(b1.0, b1.1, &HALO, 0.6 * BOB_GLOW);
            glow.splat(b2.0, b2.1, &HALO, BOB_GLOW);
        }
        let (tone, [ga, gb]) = (&self.tone, &self.glow);
        let (va, vb) = (ga.values(), gb.values());
        let w = i32::from(ga.pixel_size().width()).max(1);
        self.halo.shade(|x, y| {
            let i = (y * w + x) as usize;
            let (a, b) = (va.get(i).copied(), vb.get(i).copied());
            let (a, b) = (a.unwrap_or(0.0), b.unwrap_or(0.0));
            // Where both twins expose a pixel about equally (superposed), alternate them
            // on a checkerboard, the glow's analogue of the dashed two-tone lines.
            let close = (a - b).abs() <= 0.35 * a.max(b);
            let twin = if close {
                ((x + y) & 1) as usize
            } else {
                usize::from(b > a)
            };
            let v = a.max(b) + 0.5 * a.min(b);
            let level = tone.level(v);
            let value = GLOW_V * f64::from(level) / 255.0;
            if 255.0 * value < ink::MIN_VISIBLE_LEVEL {
                Ink::BLANK
            } else {
                twin_ink(twin, value)
            }
        });
    }
}

impl Generator for DoublePendulum {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        // Bottom row is the readout when there is room for it.
        self.label_row = (size.height() >= 10).then(|| i32::from(size.height()) - 1);
        let rows = if self.label_row.is_some() { h - 1.0 } else { h };
        self.view = View::fit(&SCENE, w, rows);
        self.lower_r = if size.height() >= TALL_ROWS {
            LOWER_R_TALL
        } else {
            LOWER_R
        };
        self.dots.resize(size);
        self.halo.resize(size);
        for g in &mut self.glow {
            g.resize(size);
        }
        // The dynamics and trails are in model units, so a resize never restarts a run.
        if !self.started {
            self.release(ctx);
        }
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        if !self.started {
            self.release(ctx);
        }
        let mut left = dt.min(MAX_DT);
        // Split the frame at trail ticks and at the run's end, so trail samples land on
        // exact multiples of TICK and the restart happens at exactly RUN_SECONDS.
        while left > TIME_SLACK {
            let h = left
                .min(TICK - self.tick_clock)
                .min(RUN_SECONDS - self.run_t)
                .max(0.0);
            for y in self.twins.iter_mut().chain(&mut self.ghosts) {
                step(&SCENE, y, h);
            }
            left -= h;
            self.run_t += h;
            self.tick_clock += h;
            if self.tick_clock >= TICK - TIME_SLACK {
                self.tick_clock = 0.0;
                self.push_trail();
            }
            let broken = !(self.twins.iter().flatten().all(|v| v.is_finite()));
            if self.run_t >= RUN_SECONDS - TIME_SLACK || broken {
                self.release(ctx);
            }
        }
        self.sep = separation(&self.twins[0], &self.twins[1]);
        self.drift = self.drift.max(self.rel_drift());
        self.refresh_labels();
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        // The glow first (Unicode only), then the line art with the glow carried on as
        // its cells' background (DP-A10).
        self.halo.blit(canvas, |_, _, _, _| None);
        self.dots.blit_over(canvas, &self.halo);
        // Right-aligned, clear of the title (GEN-R13).
        let room = title::readout_room(canvas.size(), &Title::of(self));
        if let Some(row) = self.label_row
            && let Some((label, _)) = self
                .labels
                .iter()
                .zip(FORM_WIDTHS)
                .find(|&(l, width)| width <= room && l.len() <= room)
        {
            sciart_core::draw::label_right(
                canvas,
                row,
                label,
                Ink::new(Role::Text, title::READOUT_LEVEL),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, SQRT_2};

    fn seeded(seed: u64) -> Ctx {
        Ctx::new(seed, Size::new(1, 1))
    }

    /// Times of the upward zero crossings of `θ₁`, linearly interpolated between samples.
    fn upward_crossings(p: &Params, y0: [f64; 4], seconds: f64) -> Vec<f64> {
        let mut y = y0;
        let mut t = 0.0;
        let mut out = Vec::new();
        for _ in 0..(seconds / SIM_DT).round() as usize {
            let before = y[0];
            step(p, &mut y, SIM_DT);
            if before < 0.0 && y[0] >= 0.0 {
                out.push(t + SIM_DT * (-before) / (y[0] - before));
            }
            t += SIM_DT;
        }
        out
    }

    /// Angular frequencies `(ω_slow, ω_fast)` of the linearised normal modes (DP-A2).
    ///
    /// Linearising about the rest state gives `M θ'' = −K θ` with
    /// `M = [[(m₁+m₂)L₁², m₂L₁L₂], [m₂L₁L₂, m₂L₂²]]`, `K = diag((m₁+m₂)gL₁, m₂gL₂)`.
    /// `det(K − λM) = 0` with `λ = ω²` reduces to
    /// `m₁L₁L₂ λ² − (m₁+m₂) g (L₁+L₂) λ + (m₁+m₂) g² = 0`.
    fn normal_modes(p: &Params) -> (f64, f64) {
        let Params { m1, m2, l1, l2, g } = *p;
        let m = m1 + m2;
        let a = m1 * l1 * l2;
        let b = m * g * (l1 + l2);
        let c = m * g * g;
        // b² − 4ac = (m₁+m₂) g² [(m₁+m₂)(L₁−L₂)² + 4m₂L₁L₂] ≥ 0, written as a sum of
        // non-negative terms so rounding can never make it negative.
        let disc = m * g * g * (m * (l1 - l2).powi(2) + 4.0 * m2 * l1 * l2);
        let fast = (b + disc.sqrt()) / (2.0 * a);
        // Vieta (λ₊λ₋ = c/a) instead of b − √disc avoids cancellation in the slow root.
        let slow = c / (a * fast);
        (slow.sqrt(), fast.sqrt())
    }

    /// Mode shape `θ₂/θ₁` of the normal mode with angular frequency `omega` (DP-A2), from the
    /// first row of `(K − ω²M) v = 0`: `θ₂/θ₁ = (m₁+m₂)(g − ω²L₁) / (ω² m₂ L₂)`.
    fn mode_ratio(p: &Params, omega: f64) -> f64 {
        let l = omega * omega;
        (p.m1 + p.m2) * (p.g - l * p.l1) / (l * p.m2 * p.l2)
    }

    const PARAM_SETS: [Params; 4] = [
        SCENE,
        Params {
            m1: 1.0,
            m2: 1.0,
            l1: 1.0,
            l2: 1.0,
            g: 9.81,
        },
        Params {
            m1: 3.0,
            m2: 0.5,
            l1: 1.0,
            l2: 2.0,
            g: 9.81,
        },
        Params {
            m1: 0.5,
            m2: 2.0,
            l1: 1.5,
            l2: 0.7,
            g: 4.0,
        },
    ];

    #[test]
    fn dp_a2_upper_arm_small_angle_period_is_2_2_s() {
        let period = TAU * (SCENE.l1 / SCENE.g).sqrt();
        assert!((period - UPPER_PERIOD).abs() < 1e-12, "{period}");
    }

    #[test]
    fn dp_a1_energy_drift_below_half_percent_over_20_s() {
        for seed in 0..8 {
            let mut y = seeded_start(&mut seeded(seed).rng);
            let e0 = energy(&SCENE, &y);
            assert!(e0 > 0.0);
            let mut worst: f64 = 0.0;
            for _ in 0..(20.0 / SIM_DT) as usize {
                step(&SCENE, &mut y, SIM_DT);
                worst = worst.max((energy(&SCENE, &y) - e0).abs() / e0);
            }
            // Spec bound; RK4 at h = 1/240 s actually does ~1e-6 or better here.
            assert!(worst < 5e-3, "seed {seed}: drift {worst:e}");
        }
    }

    #[test]
    fn dp_a1_generator_energy_readout_stays_below_half_percent() {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(3);
        g.resize(Size::new(100, 30), &mut ctx);
        for _ in 0..(20.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.drift < 5e-3, "{}", g.drift);
    }

    #[test]
    fn dp_a1_rest_state_is_an_equilibrium_with_zero_energy() {
        let y = [0.0; 4];
        assert_eq!(deriv(&SCENE, &y), [0.0; 4]);
        assert_eq!(energy(&SCENE, &y), 0.0);
        // Inverted is also an equilibrium, with potential 2(m₁+m₂)gL₁ + 2m₂gL₂.
        let up = [PI, PI, 0.0, 0.0];
        assert!(deriv(&SCENE, &up).iter().all(|v| v.abs() < 1e-12));
        let p = SCENE;
        let v_up = 2.0 * (p.m1 + p.m2) * p.g * p.l1 + 2.0 * p.m2 * p.g * p.l2;
        assert!((energy(&SCENE, &up) - v_up).abs() < 1e-12);
    }

    #[test]
    fn dp_a1_single_pendulum_limit_matches_simple_pendulum() {
        // With the lower mass → 0 the upper arm is a simple pendulum: θ₁'' = −(g/L₁) sin θ₁.
        let p = Params { m2: 1e-12, ..SCENE };
        let y = [0.7, 0.2, 0.3, -0.4];
        let d = deriv(&p, &y);
        assert!((d[2] + p.g / p.l1 * 0.7f64.sin_det()).abs() < 1e-9, "{d:?}");
    }

    #[test]
    fn dp_a2_equal_case_matches_textbook_2_pm_sqrt2() {
        let p = PARAM_SETS[1];
        let (slow, fast) = normal_modes(&p);
        let g_over_l = p.g / p.l1;
        assert!((slow * slow - (2.0 - 2f64.sqrt()) * g_over_l).abs() < 1e-12);
        assert!((fast * fast - (2.0 + 2f64.sqrt()) * g_over_l).abs() < 1e-12);
        // In-phase mode θ₂ = √2 θ₁, anti-phase θ₂ = −√2 θ₁.
        assert!((mode_ratio(&p, slow) - 2f64.sqrt()).abs() < 1e-12);
        assert!((mode_ratio(&p, fast) + 2f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn dp_a2_normal_modes_solve_the_characteristic_equation() {
        for p in PARAM_SETS {
            let (slow, fast) = normal_modes(&p);
            assert!(0.0 < slow && slow < fast, "{p:?}");
            for w in [slow, fast] {
                let l = w * w;
                // det(K − λM), normalised by the size of its terms.
                let k11 = (p.m1 + p.m2) * p.g * p.l1 - l * (p.m1 + p.m2) * p.l1 * p.l1;
                let k22 = p.m2 * p.g * p.l2 - l * p.m2 * p.l2 * p.l2;
                let k12 = -l * p.m2 * p.l1 * p.l2;
                let scale = ((p.m1 + p.m2) * p.g * p.l1).powi(2);
                assert!((k11 * k22 - k12 * k12).abs() / scale < 1e-12, "{p:?}");
            }
        }
    }

    #[test]
    fn dp_a2_measured_small_angle_frequencies_match_linear_analysis_within_1_percent() {
        // Released from rest in a pure mode shape at 10⁻³ rad, each mode oscillates at its
        // own frequency; the amplitude-dependent shift is O(ε²) ≈ 10⁻⁷, far below 1 %.
        for p in PARAM_SETS {
            let (slow, fast) = normal_modes(&p);
            for w in [slow, fast] {
                let eps = 1e-3;
                let y0 = [eps, eps * mode_ratio(&p, w), 0.0, 0.0];
                let crossings = upward_crossings(&p, y0, 12.0 * TAU / slow);
                assert!(crossings.len() >= 8, "{p:?}: {} crossings", crossings.len());
                let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
                let measured = TAU * (crossings.len() - 1) as f64 / (last - first);
                let err = (measured - w).abs() / w;
                assert!(err < 0.01, "{p:?}: ω {w} measured {measured} ({err:e})");
            }
        }
    }

    #[test]
    fn dp_a3_twins_diverge_at_least_100x_within_30_s() {
        for seed in 0..8 {
            let mut g = DoublePendulum::default();
            let mut ctx = seeded(seed);
            g.resize(Size::new(80, 24), &mut ctx);
            let sep0 = g.sep0;
            assert!((sep0 - TWIN_OFFSET).abs() < 1e-12, "{sep0}");
            let mut max_growth: f64 = 1.0;
            for _ in 0..(30.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                max_growth = max_growth.max(g.sep / sep0);
            }
            assert!(max_growth >= 100.0, "seed {seed}: growth {max_growth}");
        }
    }

    #[test]
    fn dp_a4_rods_keep_their_lengths_exactly() {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(11);
        g.resize(Size::new(80, 24), &mut ctx);
        for i in 0..600 {
            g.update(SIM_DT, &mut ctx);
            if i % 37 == 0 {
                for s in g.twins {
                    let [(x1, y1), (x2, y2)] = bobs(&SCENE, s[0], s[1]);
                    assert!((x1.hypot(y1) - SCENE.l1).abs() < 1e-12);
                    assert!(((x2 - x1).hypot(y2 - y1) - SCENE.l2).abs() < 1e-12);
                }
            }
        }
        // Angles are measured from the downward vertical, y down.
        let [b1, b2] = bobs(&SCENE, 0.0, FRAC_PI_2);
        assert!((b1.0).abs() < 1e-15 && (b1.1 - SCENE.l1).abs() < 1e-15);
        assert!((b2.0 - SCENE.l2).abs() < 1e-15 && (b2.1 - SCENE.l1).abs() < 1e-15);
    }

    #[test]
    fn dp_a8_bobs_stay_on_screen_at_any_size_and_angle() {
        for (w, h) in [
            (1, 1),
            (2, 3),
            (7, 40),
            (80, 24),
            (100, 30),
            (300, 20),
            (41, 9),
        ] {
            let size = Size::new(w, h);
            let mut g = DoublePendulum::default();
            g.resize(size, &mut seeded(0));
            let rows = f64::from(g.label_row.unwrap_or(i32::from(h)));
            for i in 0..64 {
                for j in 0..64 {
                    let (t1, t2) = (TAU * f64::from(i) / 64.0, TAU * f64::from(j) / 64.0);
                    for p in bobs(&SCENE, t1, t2) {
                        let (x, y) = g.view.map(p);
                        let (x, y) = (x.round(), y.round());
                        assert!((0.0..f64::from(w)).contains(&x), "{w}x{h}: x {x}");
                        assert!((0.0..rows).contains(&y), "{w}x{h}: y {y}");
                    }
                }
            }
        }
    }

    #[test]
    fn dp_gen_r13_readout_is_the_longest_form_right_of_the_title() {
        // Long, mid and short forms: the room right of the title decides.
        for (size, form) in [
            (Size::new(200, 50), 0),
            (Size::new(80, 24), 1),
            (Size::new(40, 12), 2),
        ] {
            let mut g = DoublePendulum::default();
            let mut ctx = seeded(3);
            g.resize(size, &mut ctx);
            // At the start and once the energy drift is non-zero: the form must not
            // change with the width of the numbers.
            for _ in 0..2 {
                let mut canvas = Canvas::new(size, GlyphMode::Ascii);
                g.render(&mut canvas);
                let bottom = &canvas.to_lines()[usize::from(size.height()) - 1];
                let label = &g.labels[form];
                assert!(
                    bottom.ends_with(&format!(" {label} ")),
                    "{size:?}: {bottom:?}"
                );
                for _ in 0..60 {
                    g.update(SIM_DT, &mut ctx);
                }
            }
        }
    }

    #[test]
    fn dp_gen_r13_form_widths_bound_every_value() {
        let seps = [0.0, 5e-324, 1e-300, 1.2e-10, 1e-3, 0.123, 4.4, PI * SQRT_2];
        let drifts = [0.0, 5e-324, 1e-16, 3e-7, 0.5, 9.9, 1e308, f64::MAX];
        let mut widest = [0; 3];
        let mut forms: [String; 3] = Default::default();
        for sep in seps {
            for drift in drifts {
                for t in [0.0, 9.4, 44.6, RUN_SECONDS] {
                    write_forms(&mut forms, sep, 1e-3, drift, t);
                    for (w, f) in widest.iter_mut().zip(&forms) {
                        *w = (*w).max(f.len());
                    }
                }
            }
        }
        assert_eq!(widest, FORM_WIDTHS, "the widths are tight bounds");
        assert!(PAD.len() >= FORM_WIDTHS[0]);
    }

    #[test]
    fn dp_gen_r13_readout_is_padded_at_the_front_only() {
        let mut g = DoublePendulum::default();
        g.resize(Size::new(200, 50), &mut seeded(3));
        for (label, width) in g.labels.iter().zip(FORM_WIDTHS) {
            assert_eq!(label.len(), width, "{label:?}");
            let text = label.trim_start();
            assert!(!text.contains("x start") || text.contains(" = "), "{text}");
            // No run of padding inside the text: separators are at most 3 spaces.
            assert!(!text.contains("    "), "inner padding: {text:?}");
        }
    }

    #[test]
    fn dp_a4_view_is_aspect_correct() {
        let v = View::fit(&SCENE, 100.0, 30.0);
        let (x, _) = v.map((1.0, 0.0));
        let (_, y) = v.map((0.0, 1.0));
        assert!(((x - v.cx) - ASPECT * (y - v.cy)).abs() < 1e-12);
    }

    #[test]
    fn dp_a9_starts_are_high_energy_and_twins_differ_only_in_theta2() {
        let mut ctx = seeded(5);
        for _ in 0..200 {
            let y = seeded_start(&mut ctx.rng);
            assert!(y[0].abs() > FRAC_PI_2 && y[0].abs() < PI, "{y:?}");
            assert!(y[1].abs() > FRAC_PI_2 && y[1].abs() < PI, "{y:?}");
            assert_eq!((y[2], y[3]), (0.0, 0.0));
        }
        let mut g = DoublePendulum::default();
        g.resize(Size::new(80, 24), &mut seeded(1));
        let [a, b] = g.twins;
        assert_eq!((a[0], a[2], a[3]), (b[0], b[2], b[3]));
        assert!((b[1] - a[1] - TWIN_OFFSET).abs() < 1e-15);
    }

    #[test]
    fn dp_a3_separation_wraps_full_turns() {
        let a = [0.1, -3.1, 0.0, 0.0];
        let b = [0.1 + TAU, 3.1, 5.0, 5.0];
        let d = separation(&a, &b);
        assert!((d - (TAU - 6.2)).abs() < 1e-12, "{d}");
    }

    #[test]
    fn dp_a9_restarts_every_45_s_with_new_seeded_angles() {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(2);
        g.resize(Size::new(80, 24), &mut ctx);
        let first = g.twins[0];
        for _ in 0..(RUN_SECONDS / SIM_DT).round() as usize - 1 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.run_t > RUN_SECONDS - 0.05);
        g.update(SIM_DT, &mut ctx);
        assert!(g.run_t < 1e-9, "{}", g.run_t);
        let [a, b] = g.twins;
        assert_ne!(a[..2], first[..2]);
        assert_eq!((a[2], a[3]), (0.0, 0.0));
        assert!((separation(&a, &b) - TWIN_OFFSET).abs() < 1e-12);
        assert_eq!(g.trail.len(), 1);
    }

    #[test]
    fn dp_a9_resize_keeps_the_run() {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(4);
        g.resize(Size::new(80, 24), &mut ctx);
        for _ in 0..90 {
            g.update(SIM_DT, &mut ctx);
        }
        let (a, t) = (g.twins, g.run_t);
        g.resize(Size::new(33, 11), &mut ctx);
        assert_eq!((g.twins, g.run_t), (a, t));
    }

    /// Scene `seed` at `size` after `seconds` of simulated time.
    #[test]
    fn dp_a10_each_twin_glows_in_its_own_ramp_under_brighter_line_art() {
        let size = Size::new(120, 40);
        let g = run(0, size, 30.0);
        // Round each lower bob: a halo in that twin's ramp, from the first frame after a
        // resize too (the glow is rebuilt from the model-space trail).
        let mut resized = run(0, Size::new(80, 24), 30.0);
        resized.resize(size, &mut seeded(0));
        for s in [&g, &resized] {
            for (i, (y, ramp)) in s.twins.iter().zip(RAMPS).enumerate() {
                let (dx, dy) = s.view.dot(bobs(&SCENE, y[0], y[1])[1]);
                let (px, py) = ((dx / 2.0) as i32, (dy / 2.0) as i32);
                let own = (-3..=3)
                    .flat_map(|oy| (-3..=3).map(move |ox| (px + ox, py + oy)))
                    .filter_map(|(x, y)| s.halo.get(x, y))
                    .filter(|p| p.role == Role::Ramp(ramp))
                    .count();
                assert!(own >= 12, "twin {i}: {own} halo pixels");
            }
        }
        // The glow stays under the line art: no pixel above GLOW_V, and every dotted
        // cell's ink is brighter than the glow behind it.
        let cap = ink::ramp(RAMPS[0], GLOW_V).level;
        let mut glowing = 0;
        for y in 0..80 {
            for x in 0..120 {
                let p = g.halo.get(x, y).unwrap();
                assert!(p.level <= cap, "({x},{y}) {p:?}");
                glowing += usize::from(p.level > 0);
            }
        }
        assert!(glowing > 300, "{glowing} glowing pixels");
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        for cell in c.cells() {
            let braille = ('\u{2801}'..='\u{28FF}').contains(&cell.ch);
            if braille
                && matches!(cell.bg.role, Role::Ramp(_))
                && matches!(cell.ink.role, Role::Ramp(_))
            {
                assert!(cell.ink.level > cell.bg.level, "{cell:?}");
            }
        }
        // ASCII has no half blocks, so no glow: only the line art's own cells.
        let mut a = Canvas::new(size, GlyphMode::Ascii);
        g.render(&mut a);
        assert!(a.cells().iter().all(|c| c.bg == Ink::BLANK));
        let mut lines = Canvas::new(size, GlyphMode::Ascii);
        g.dots.blit(&mut lines);
        let w = usize::from(size.width());
        assert_eq!(a.cells()[..39 * w], lines.cells()[..39 * w]);
    }

    fn run(seed: u64, size: Size, seconds: f64) -> DoublePendulum {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(seed);
        g.resize(size, &mut ctx);
        for _ in 0..(seconds / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g
    }

    /// Roles of the drawn cells above the readout row.
    fn roles(g: &DoublePendulum, size: Size) -> Vec<Role> {
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let label = usize::from(size.height()) - 1;
        canvas
            .cells()
            .chunks(usize::from(size.width()))
            .take(label)
            .flatten()
            .filter(|c| c.ch != ' ')
            .map(|c| c.ink.role)
            .collect()
    }

    /// Lit cells in each twin's ramp.
    fn hue_counts(lit: &[Role]) -> [usize; 2] {
        RAMPS.map(|r| lit.iter().filter(|&&l| l == Role::Ramp(r)).count())
    }

    #[test]
    fn dp_a5_both_twins_show_in_their_own_hues_from_the_first_frame() {
        let size = Size::new(80, 24);
        for seed in 0..4 {
            for t in [0.0, 1.0] {
                let g = run(seed, size, t);
                // Still merged: closer than a braille cell.
                assert_eq!(split(g.apart * g.view.dots_per_unit()), 0.0, "seed {seed}");
                let lit = roles(&g, size);
                // Only the pivot hinge (≤ 2 cells) is white; the rest is the two twins in
                // comparable amounts.
                let text = lit.iter().filter(|&&r| r == Role::Text).count();
                assert!(text <= 2, "seed {seed} t {t}: {text} white cells");
                let [a, b] = hue_counts(&lit);
                assert!(a.min(b) * 3 >= a.max(b), "seed {seed} t {t}: {a} vs {b}");
            }
            let late = run(seed, size, 30.0);
            assert_eq!(split(late.apart * late.view.dots_per_unit()), 1.0);
            let [a, b] = hue_counts(&roles(&late, size));
            assert!(a > 0 && b > 0, "seed {seed}: {a} vs {b}");
        }
    }

    /// The scene at 200×50 with twin B turned rigidly about the pivot so that the lower
    /// bobs are `dots` apart on screen, and a fresh trail.
    fn posed(dots: f64) -> (DoublePendulum, Size) {
        let size = Size::new(200, 50);
        let mut g = run(1, size, 0.0);
        let a = [0.4, 0.9, 0.0, 0.0];
        let reach = bobs(&SCENE, a[0], a[1])[1];
        let turn = dots / (g.view.dots_per_unit() * reach.0.hypot(reach.1));
        g.twins = [a, [a[0] + turn, a[1] + turn, 0.0, 0.0]];
        g.apart = 0.0;
        g.trail.clear();
        g.push_trail();
        g.rasterise();
        (g, size)
    }

    #[test]
    fn dp_a5_superposed_twins_are_two_tone_with_half_green_half_crimson_bobs() {
        assert_eq!(split(MERGE_DOTS), 0.0);
        assert_eq!(split(SPLIT_DOTS), 1.0);
        for gap in [0.0, 2.0] {
            let (mut g, size) = posed(gap);
            // A swept trail as well as the rods: both hues, neither drowning the other.
            g.trail.clear();
            for j in 0..40 {
                let a = [0.4 + 0.02 * f64::from(j), 0.9, 0.0, 0.0];
                let d = g.twins[1][0] - g.twins[0][0];
                g.twins = [a, [a[0] + d, a[1] + d, 0.0, 0.0]];
                g.push_trail();
            }
            g.rasterise();
            let [a, b] = hue_counts(&roles(&g, size));
            assert!(
                a.min(b) * 3 >= a.max(b),
                "gap {gap}: aurora {a} vs halpha {b}"
            );
            // The lower bob: half twin A's green, half twin B's crimson.
            let tip = g.view.dot(bobs(&SCENE, g.twins[0][0], g.twins[0][1])[1]);
            let (cx, cy) = ((tip.0 / 2.0) as i32, (tip.1 / 4.0) as i32);
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let [a, b] = [0, 1].map(|i| {
                (-2..=2)
                    .flat_map(|dy| (-2..=2).map(move |dx| (cx + dx, cy + dy)))
                    .filter_map(|(x, y)| canvas.get(x, y))
                    .filter(|c| c.ink == twin_ink(i, BOB_V))
                    .count()
            });
            assert!(
                a >= 1 && b >= 1,
                "gap {gap}: bob cells aurora {a}, halpha {b}"
            );
        }
        // Ten dots apart both hues show in comparable amounts.
        let (g, size) = posed(10.0);
        let [a, b] = hue_counts(&roles(&g, size));
        assert!(a.min(b) * 3 >= a.max(b) * 2, "aurora {a} vs halpha {b}");
    }

    #[test]
    fn dp_a5_dashes_alternate_the_twin_on_top_and_stay_on_their_stretch_of_path() {
        assert_eq!(dash_order(0), [1, 0]);
        assert_eq!(dash_order(1), [0, 1]);
        // Coinciding rods: every dash is drawn by both twins, each on top in turn.
        let mut dots = Dots::default();
        dots.resize(Size::new(40, 10));
        let (p, q) = ((2.0, 20.0), (70.0, 20.0));
        dashed(&mut dots, [p, p], [q, q], ROD_V);
        let mut canvas = Canvas::new(Size::new(40, 10), GlyphMode::Unicode);
        dots.blit(&mut canvas);
        let hues: Vec<_> = (1..35)
            .filter_map(|x| canvas.get(x, 5))
            .map(|c| c.ink.role)
            .collect();
        let [a, b] = hue_counts(&hues);
        assert!(a >= 8 && b >= 8, "{a} {b}");
        // Full trail: the run-wide sample index picks the dash, so one more push does not
        // move a sample's dash.
        let mut g = run(2, Size::new(80, 24), 5.0);
        let base = |g: &DoublePendulum| g.pushed - g.trail.len();
        let before = base(&g);
        g.push_trail();
        assert_eq!(base(&g), before + 1);
        // Trail samples and the dash index agree after a restart.
        g.release(&mut seeded(2));
        assert_eq!((g.pushed, g.trail.len()), (1, 1));
    }

    #[test]
    fn dp_a5_trail_rods_bobs_and_cores_rise_in_brightness() {
        // The pendulum outshines its own trail (also a compile-time assertion).
        let levels = [TRAIL_V, ROD_V, BOB_V, CORE_V].map(|v| twin_ink(0, v).level);
        assert!(levels.is_sorted_by(|a, b| a < b), "{levels:?}");
    }

    #[test]
    fn dp_a6_ghost_starts_fill_the_gap_between_the_twins() {
        let g = run(7, Size::new(80, 24), 0.0);
        let [a, b] = g.twins;
        let mut last = a[1];
        for y in g.ghosts {
            assert_eq!((y[0], y[2], y[3]), (a[0], a[2], a[3]));
            assert!(last < y[1] && y[1] < b[1], "{last} {} {}", y[1], b[1]);
            last = y[1];
        }
    }

    #[test]
    fn dp_a6_ghost_fan_shows_after_the_twins_part_and_dissolves_as_it_breaks() {
        assert_eq!(ghost_fade(0.0, 1.0), 0.0);
        assert_eq!(ghost_fade(1.0, 1.0), 1.0);
        assert_eq!(ghost_fade(0.5, GHOST_KEPT.1), 0.5);
        assert_eq!(ghost_fade(1.0, GHOST_KEPT.0), 0.0);
        assert_eq!(ghost_fade(1.0, 0.0), 0.0);
        let mut last = 0.0;
        for i in 0..=60 {
            let f = ghost_fade(1.0, f64::from(i) / 60.0);
            assert!(f >= last, "not monotone at {i}");
            last = f;
        }
        // Chaos spreads the fan: once the twins have parted, the ghosts' lower bobs span
        // much of the reach.
        for seed in 0..4 {
            let g = run(seed, Size::new(80, 24), 30.0);
            let lows: Vec<_> = g
                .ghosts
                .iter()
                .map(|y| bobs(&SCENE, y[0], y[1])[1])
                .collect();
            let spread = lows
                .iter()
                .flat_map(|p| lows.iter().map(move |q| (p.0 - q.0).hypot(p.1 - q.1)))
                .fold(0.0, f64::max);
            assert!(spread > 0.5, "seed {seed}: fan spread {spread}");
        }
    }

    #[test]
    fn dp_a6_ghost_line_joins_neighbours_and_breaks_where_stretched() {
        assert!(ghost_link((0.0, 0.0), (0.0, GHOST_LINK)));
        assert!(!ghost_link((0.0, 0.0), (0.0, GHOST_LINK + 1e-9)));
        // Once the fan has fully faded in, the line is still continuous: every neighbour
        // pair along twin A → ghosts → twin B is joined (it is not yet stretched past its
        // samples), so the fan reads as one curve.
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(1);
        g.resize(Size::new(80, 24), &mut ctx);
        for _ in 0..(RUN_SECONDS / SIM_DT) as usize {
            if split(g.apart * g.view.dots_per_unit()) >= 1.0 {
                break;
            }
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.run_t > 0.0 && split(g.apart * g.view.dots_per_unit()) >= 1.0);
        let lower = |y: &[f64; 4]| bobs(&SCENE, y[0], y[1])[1];
        let line: Vec<_> = std::iter::once(lower(&g.twins[0]))
            .chain(g.ghosts.iter().map(lower))
            .chain([lower(&g.twins[1])])
            .collect();
        assert!(line.windows(2).all(|w| ghost_link(w[0], w[1])));
    }

    #[test]
    fn dp_a5_parted_twins_keep_their_hue_at_the_heads() {
        // The bob discs sit in the saturated middle of their ramps, below the near-white
        // top, with only a one-dot core above it.
        let g = run(0, Size::new(120, 40), 30.0);
        let tip = g.view.dot(bobs(&SCENE, g.twins[1][0], g.twins[1][1])[1]);
        let (cx, cy) = ((tip.0 / 2.0) as i32, (tip.1 / 4.0) as i32);
        let mut canvas = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut canvas);
        let hued = [-1, 0, 1]
            .iter()
            .flat_map(|dy| [-1, 0, 1].map(|dx| (cx + dx, cy + dy)))
            .filter_map(|(x, y)| canvas.get(x, y))
            .filter(|c| {
                c.ink.role == Role::Ramp(RAMPS[1])
                    && (ink::seq(ROD_V).level..=ink::seq(BOB_V).level).contains(&c.ink.level)
            })
            .count();
        assert!(
            hued >= 2,
            "{hued} saturated cells around twin B's lower bob"
        );
    }

    #[test]
    fn dp_a7_trails_are_three_seconds_in_model_space_and_survive_a_resize() {
        let mut g = run(3, Size::new(80, 24), 5.0);
        assert_eq!(g.trail.len(), TRAIL_LEN);
        assert!((TRAIL_LEN as f64 * TICK - 3.0).abs() < 1e-12);
        let size = Size::new(120, 40);
        g.resize(size, &mut seeded(3));
        let lit = roles(&g, size).len();
        // Two bobs, two rods and a pivot light well under 60 cells at 120×40; the rest is
        // the trail, redrawn from model space.
        assert!(lit > 100, "{lit} cells lit after the resize");
    }

    #[test]
    fn dp_a8_gen_r13_no_dots_on_the_readout_row() {
        for (w, h) in [(80, 24), (40, 12), (200, 50)] {
            let size = Size::new(w, h);
            let mut g = DoublePendulum::default();
            let mut ctx = seeded(5);
            g.resize(size, &mut ctx);
            for i in 0..(45.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                if i % 7 == 0 {
                    let row = i32::from(h) - 1;
                    assert!(
                        (0..i32::from(w)).all(|x| g.dots.cell_mask(x, row) == Some(0)),
                        "{w}x{h}: dots on row {row} at step {i}"
                    );
                }
            }
        }
    }
}
