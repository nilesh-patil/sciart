//! Twin double pendulums released 10⁻³ rad apart: they swing as one, then diverge
//! chaotically, each lower bob tracing a glowing, fading path (SPEC-002 `double_pendulum`).
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
use sciart_core::geom::{self, ASPECT};
use sciart_core::ode;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "double_pendulum",
    category: Category::PhysicsChemistry,
    description: "Twin double pendulums diverging chaotically from almost identical starts",
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
/// Trail sampling tick and length (~2 s of lower-bob path, spec).
const TICK: f64 = 1.0 / 60.0;
const TRAIL_LEN: usize = 120;
/// The opening caption is shown for this long after each release, s.
const CAPTION_SECONDS: f64 = 6.0;
/// Largest frame step honoured; bounds the per-frame cost whatever the caller passes.
const MAX_DT: f64 = 0.25;
/// Slack for accumulated-time comparisons: summing 2700 frame steps of 1/60 s misses
/// 45 s by ~1e-12, which must still count as "the run is over".
const TIME_SLACK: f64 = 1e-9;

const PIVOT: Glyph = Glyph::new('◆', '+');
const UPPER_BOB: Glyph = Glyph::new('•', 'o');
const LOWER_BOB: Glyph = Glyph::new('●', 'O');
const ROD_H: Glyph = Glyph::new('─', '-');
const ROD_V: Glyph = Glyph::new('│', '|');
const ROD_UP: Glyph = Glyph::new('╱', '/');
const ROD_DOWN: Glyph = Glyph::new('╲', '\\');
/// Trail glyphs, oldest → newest.
const TRAIL_GLYPHS: [Glyph; 3] = [
    Glyph::new('·', '.'),
    Glyph::new('∙', ':'),
    Glyph::new('•', 'o'),
];

/// Right-hand side of the exact Lagrangian equations of motion (DP-A1).
///
/// With `Δ = θ₁ − θ₂` and `D = 2m₁ + m₂ − m₂ cos 2Δ` (≥ 2m₁ > 0, so never singular):
/// - `θ₁'' = [−g(2m₁+m₂) sin θ₁ − m₂ g sin(θ₁ − 2θ₂) − 2 sin Δ · m₂ (ω₂² L₂ + ω₁² L₁ cos Δ)] / (L₁ D)`
/// - `θ₂'' = [2 sin Δ (ω₁² L₁ (m₁+m₂) + g (m₁+m₂) cos θ₁ + ω₂² L₂ m₂ cos Δ)] / (L₂ D)`
pub(crate) fn deriv(p: &Params, y: &[f64; 4]) -> [f64; 4] {
    let [th1, th2, w1, w2] = *y;
    let Params { m1, m2, l1, l2, g } = *p;
    let delta = th1 - th2;
    let (sd, cd) = delta.sin_cos();
    let den = 2.0 * m1 + m2 - m2 * (2.0 * delta).cos();
    let a1 = (-g * (2.0 * m1 + m2) * th1.sin()
        - m2 * g * (th1 - 2.0 * th2).sin()
        - 2.0 * sd * m2 * (w2 * w2 * l2 + w1 * w1 * l1 * cd))
        / (l1 * den);
    let a2 =
        2.0 * sd * (w1 * w1 * l1 * (m1 + m2) + g * (m1 + m2) * th1.cos() + w2 * w2 * l2 * m2 * cd)
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
        + m2 * l1 * l2 * w1 * w2 * (th1 - th2).cos();
    // 1 − cos θ = 2 sin²(θ/2) keeps small-angle energies accurate (no cancellation).
    let lift = |th: f64| 2.0 * (0.5 * th).sin().powi(2);
    let potential = m * g * l1 * lift(th1) + m2 * g * l2 * lift(th2);
    kinetic + potential
}

/// Angular frequencies `(ω_slow, ω_fast)` of the linearised normal modes (DP-A2).
///
/// Linearising about the rest state gives `M θ'' = −K θ` with
/// `M = [[(m₁+m₂)L₁², m₂L₁L₂], [m₂L₁L₂, m₂L₂²]]`, `K = diag((m₁+m₂)gL₁, m₂gL₂)`.
/// `det(K − λM) = 0` with `λ = ω²` reduces to
/// `m₁L₁L₂ λ² − (m₁+m₂) g (L₁+L₂) λ + (m₁+m₂) g² = 0`.
pub(crate) fn normal_modes(p: &Params) -> (f64, f64) {
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
pub(crate) fn mode_ratio(p: &Params, omega: f64) -> f64 {
    let l = omega * omega;
    (p.m1 + p.m2) * (p.g - l * p.l1) / (l * p.m2 * p.l2)
}

/// Positions of the upper and lower bobs relative to the pivot, model units, y down
/// (DP-A4): always derived from the angles, so both rods are rigid by construction.
pub(crate) fn bobs(p: &Params, th1: f64, th2: f64) -> [(f64, f64); 2] {
    let (s1, c1) = th1.sin_cos();
    let (s2, c2) = th2.sin_cos();
    let b1 = (p.l1 * s1, p.l1 * c1);
    [b1, (b1.0 + p.l2 * s2, b1.1 + p.l2 * c2)]
}

/// "small swings: normal modes 2.78 s (in step) & 1.22 s (opposed) -- large: chaos":
/// the linear prediction the chaotic motion departs from (DP-A2).
fn modes_caption(p: &Params) -> String {
    let (slow, fast) = normal_modes(p);
    let kind = |w: f64| {
        if mode_ratio(p, w) > 0.0 {
            "in step"
        } else {
            "opposed"
        }
    };
    format!(
        "small swings: normal modes {:.2} s ({}) & {:.2} s ({}) -- large swings: chaos",
        TAU / slow,
        kind(slow),
        TAU / fast,
        kind(fast)
    )
}

/// `a` wrapped into `(−π, π]`.
fn wrap_pi(a: f64) -> f64 {
    a - TAU * (a / TAU).round()
}

/// Angular separation of two states, `√(Δθ₁² + Δθ₂²)` with each difference wrapped to
/// `(−π, π]` (DP-A3) — a full turn apart is the same configuration.
pub(crate) fn separation(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    wrap_pi(a[0] - b[0]).hypot(wrap_pi(a[1] - b[1]))
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

/// Screen mapping: pivot at `(cx, cy)` cells, `scale` rows per model unit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct View {
    cx: f64,
    cy: f64,
    scale: f64,
}

impl View {
    /// Pivot centred in `rows × cols`, scaled so the full reach `L₁ + L₂` in every direction
    /// stays inside (aspect-corrected). The rounded extremes land on `0` and `size − 1`.
    fn fit(p: &Params, cols: f64, rows: f64) -> Self {
        let (cx, cy) = (((cols - 1.0) / 2.0).max(0.0), ((rows - 1.0) / 2.0).max(0.0));
        let reach = p.l1 + p.l2;
        let scale = (cy.min(cx / ASPECT) / reach).max(0.0);
        Self { cx, cy, scale }
    }

    /// Model point (y down) → fractional cell coordinates.
    #[inline]
    fn map(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (self.cx + self.scale * x * ASPECT, self.cy + self.scale * y)
    }
}

#[derive(Debug)]
pub struct DoublePendulum {
    /// The reference pendulum (`Cold`) and its twin (`Hot`).
    a: [f64; 4],
    b: [f64; 4],
    started: bool,
    /// Time since the current release, s.
    run_t: f64,
    tick_clock: f64,
    /// Energy of each twin at release (DP-A1 readout).
    e0: [f64; 2],
    /// Largest relative energy drift seen this run.
    drift: f64,
    /// Current and initial separation (DP-A3 readout).
    sep: f64,
    sep0: f64,
    trail_a: Trail<(f64, f64)>,
    trail_b: Trail<(f64, f64)>,
    view: View,
    label_row: Option<i32>,
    /// Readouts in decreasing length; render picks the longest that fits.
    labels: [String; 3],
    /// What small swings would do, from the linear analysis (DP-A2).
    modes_caption: String,
}

impl Default for DoublePendulum {
    fn default() -> Self {
        Self {
            a: [0.0; 4],
            b: [0.0; 4],
            started: false,
            run_t: 0.0,
            tick_clock: 0.0,
            e0: [0.0; 2],
            drift: 0.0,
            sep: 0.0,
            sep0: TWIN_OFFSET,
            trail_a: Trail::new(TRAIL_LEN),
            trail_b: Trail::new(TRAIL_LEN),
            view: View::default(),
            label_row: None,
            labels: std::array::from_fn(|_| String::with_capacity(96)),
            modes_caption: modes_caption(&SCENE),
        }
    }
}

impl DoublePendulum {
    /// Release both twins from a new seeded start.
    fn release(&mut self, ctx: &mut Ctx) {
        self.a = seeded_start(&mut ctx.rng);
        self.b = self.a;
        self.b[1] += TWIN_OFFSET;
        self.e0 = [energy(&SCENE, &self.a), energy(&SCENE, &self.b)];
        self.drift = 0.0;
        self.sep0 = separation(&self.a, &self.b);
        self.sep = self.sep0;
        self.run_t = 0.0;
        self.tick_clock = 0.0;
        self.trail_a.clear();
        self.trail_b.clear();
        self.push_trails();
        self.started = true;
        self.refresh_labels();
    }

    fn push_trails(&mut self) {
        self.trail_a.push(bobs(&SCENE, self.a[0], self.a[1])[1]);
        self.trail_b.push(bobs(&SCENE, self.b[0], self.b[1])[1]);
    }

    fn rel_drift(&self) -> f64 {
        let da = (energy(&SCENE, &self.a) - self.e0[0]).abs() / self.e0[0].max(1e-12);
        let db = (energy(&SCENE, &self.b) - self.e0[1]).abs() / self.e0[1].max(1e-12);
        da.max(db)
    }

    fn refresh_labels(&mut self) {
        let growth = self.sep / self.sep0.max(1e-300);
        let folds = growth.max(1e-300).ln();
        let (sep, drift, t) = (self.sep, self.drift, self.run_t);
        let [long, mid, short] = &mut self.labels;
        long.clear();
        mid.clear();
        short.clear();
        let _ = write!(
            long,
            "|dtheta| {sep:.1e} rad = {growth:>7.1}x start ({folds:>4.1} e-folds)   \
             energy drift {drift:.0e}   t {t:>2.0}/{RUN_SECONDS:.0}s"
        );
        let _ = write!(
            mid,
            "|dtheta| {sep:.1e} ({folds:>4.1} e-folds)  dE/E {drift:.0e}"
        );
        let _ = write!(short, "e-folds {folds:>4.1}");
    }

    /// Draw a rod from `p` to `q` (cells), endpoints excluded, with a glyph that follows
    /// its on-screen slope.
    fn rod(canvas: &mut Canvas, p: (f64, f64), q: (f64, f64), ink: Ink) {
        let (x0, y0, x1, y1) = (
            p.0.round() as i32,
            p.1.round() as i32,
            q.0.round() as i32,
            q.1.round() as i32,
        );
        let (dx, dy) = (q.0 - p.0, q.1 - p.1);
        let glyph = if dy.abs() < 0.4 * dx.abs() {
            ROD_H
        } else if dx.abs() < 0.5 * dy.abs() {
            ROD_V
        } else if (dx > 0.0) == (dy < 0.0) {
            ROD_UP
        } else {
            ROD_DOWN
        };
        for (x, y) in geom::line(x0, y0, x1, y1) {
            if (x, y) != (x0, y0) && (x, y) != (x1, y1) {
                canvas.put(x, y, glyph, ink);
            }
        }
    }

    /// Put a glyph for twin B; where it lands on twin A's ink the cell turns `Accent`, so
    /// the twins read as one while they are in step.
    fn put_b(canvas: &mut Canvas, x: f64, y: f64, glyph: Glyph, level: u8) {
        let (x, y) = (x.round(), y.round());
        let size = canvas.size();
        if !((0.0..f64::from(size.width())).contains(&x)
            && (0.0..f64::from(size.height())).contains(&y))
        {
            return;
        }
        let (xi, yi) = (x as i32, y as i32);
        let role = match canvas.get(xi, yi) {
            Some(c) if c.ink.role == Role::Cold => Role::Accent,
            _ => Role::Hot,
        };
        canvas.put(xi, yi, glyph, Ink::new(role, level));
    }

    fn put_a(canvas: &mut Canvas, x: f64, y: f64, glyph: Glyph, level: u8) {
        sciart_core::particles::plot(canvas, x, y, glyph, Ink::new(Role::Cold, level));
    }

    /// Fading, glowing trail: consecutive samples joined by interpolated dots.
    fn trail(
        &self,
        canvas: &mut Canvas,
        trail: &Trail<(f64, f64)>,
        put: fn(&mut Canvas, f64, f64, Glyph, u8),
    ) {
        let n = trail.len();
        let mut prev: Option<(f64, f64)> = None;
        for (i, &p) in trail.iter().enumerate() {
            let q = self.view.map(p);
            // 0 = oldest, 1 = newest.
            let fresh = (i + 1) as f64 / n.max(1) as f64;
            let level = (30.0 + 200.0 * fresh.powf(1.6)) as u8;
            let glyph = TRAIL_GLYPHS[((fresh * 3.0) as usize).min(2)];
            if let Some(o) = prev {
                let (dc, dr) = (q.0 - o.0, q.1 - o.1);
                let steps = dc.abs().max(dr.abs()).ceil().clamp(1.0, 64.0) as usize;
                for k in 1..=steps {
                    let f = k as f64 / steps as f64;
                    put(canvas, o.0 + dc * f, o.1 + dr * f, glyph, level);
                }
            } else {
                put(canvas, q.0, q.1, glyph, level);
            }
            prev = Some(q);
        }
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
        // The dynamics are in model units, so a resize never restarts a run.
        if !self.started {
            self.release(ctx);
        }
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
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
            step(&SCENE, &mut self.a, h);
            step(&SCENE, &mut self.b, h);
            left -= h;
            self.run_t += h;
            self.tick_clock += h;
            if self.tick_clock >= TICK - TIME_SLACK {
                self.tick_clock = 0.0;
                self.push_trails();
            }
            let broken = !(self.a.iter().chain(&self.b).all(|v| v.is_finite()));
            if self.run_t >= RUN_SECONDS - TIME_SLACK || broken {
                self.release(ctx);
            }
        }
        self.sep = separation(&self.a, &self.b);
        self.drift = self.drift.max(self.rel_drift());
        self.refresh_labels();
    }

    fn render(&self, canvas: &mut Canvas) {
        let width = i32::from(canvas.size().width());

        // Opening caption, fading out.
        if self.run_t < CAPTION_SECONDS && canvas.size().height() >= 14 {
            let fade = 1.0 - self.run_t / CAPTION_SECONDS;
            let ink = Ink::new(Role::Text, (60.0 + 170.0 * fade) as u8);
            for s in [
                "two double pendulums, released 0.001 rad apart",
                "released 0.001 rad apart",
            ] {
                if s.len() as i32 + 2 <= width {
                    sciart_core::draw::text_centered(canvas, 0, s, ink);
                    break;
                }
            }
            // Only on tall terminals: row 1 is otherwise too often under the pendulum.
            if canvas.size().height() >= 24 && self.modes_caption.len() as i32 + 2 <= width {
                let ink = Ink::new(Role::Text, (40.0 + 120.0 * fade) as u8);
                sciart_core::draw::text_centered(canvas, 1, &self.modes_caption, ink);
            }
        }

        self.trail(canvas, &self.trail_a, Self::put_a);
        self.trail(canvas, &self.trail_b, Self::put_b);

        let pivot = self.view.map((0.0, 0.0));
        for (state, lvl) in [(&self.a, 150u8), (&self.b, 120u8)] {
            let [p1, p2] = bobs(&SCENE, state[0], state[1]);
            let (p1, p2) = (self.view.map(p1), self.view.map(p2));
            Self::rod(canvas, pivot, p1, Ink::new(Role::Structure, lvl));
            Self::rod(canvas, p1, p2, Ink::new(Role::Structure, lvl));
        }
        for (state, put) in [
            (&self.a, Self::put_a as fn(&mut Canvas, f64, f64, Glyph, u8)),
            (&self.b, Self::put_b),
        ] {
            let [p1, p2] = bobs(&SCENE, state[0], state[1]);
            let (p1, p2) = (self.view.map(p1), self.view.map(p2));
            put(canvas, p1.0, p1.1, UPPER_BOB, 220);
            put(canvas, p2.0, p2.1, LOWER_BOB, 255);
        }
        sciart_core::particles::plot(canvas, pivot.0, pivot.1, PIVOT, Ink::new(Role::Text, 230));

        if let Some(row) = self.label_row
            && let Some(label) = self.labels.iter().find(|l| l.len() as i32 + 2 <= width)
        {
            sciart_core::draw::text_centered(canvas, row, label, Ink::new(Role::Text, 170));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn dp_a2_caption_reports_in_phase_slow_and_anti_phase_fast_modes() {
        let c = modes_caption(&SCENE);
        let (slow, fast) = normal_modes(&SCENE);
        assert!(c.contains(&format!("{:.2} s (in step)", TAU / slow)), "{c}");
        assert!(c.contains(&format!("{:.2} s (opposed)", TAU / fast)), "{c}");
    }

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
    fn dp_scene_upper_arm_small_angle_period_is_2_2_s() {
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
        assert!((d[2] + p.g / p.l1 * 0.7f64.sin()).abs() < 1e-9, "{d:?}");
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
                for s in [g.a, g.b] {
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
    fn dp_bobs_stay_on_screen_at_any_size_and_angle() {
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
    fn dp_view_is_aspect_correct() {
        let v = View::fit(&SCENE, 100.0, 30.0);
        let (x, _) = v.map((1.0, 0.0));
        let (_, y) = v.map((0.0, 1.0));
        assert!(((x - v.cx) - ASPECT * (y - v.cy)).abs() < 1e-12);
    }

    #[test]
    fn dp_starts_are_high_energy_and_twins_differ_only_in_theta2() {
        let mut ctx = seeded(5);
        for _ in 0..200 {
            let y = seeded_start(&mut ctx.rng);
            assert!(y[0].abs() > FRAC_PI_2 && y[0].abs() < PI, "{y:?}");
            assert!(y[1].abs() > FRAC_PI_2 && y[1].abs() < PI, "{y:?}");
            assert_eq!((y[2], y[3]), (0.0, 0.0));
        }
        let mut g = DoublePendulum::default();
        g.resize(Size::new(80, 24), &mut seeded(1));
        assert_eq!((g.a[0], g.a[2], g.a[3]), (g.b[0], g.b[2], g.b[3]));
        assert!((g.b[1] - g.a[1] - TWIN_OFFSET).abs() < 1e-15);
    }

    #[test]
    fn dp_separation_wraps_full_turns() {
        let a = [0.1, -3.1, 0.0, 0.0];
        let b = [0.1 + TAU, 3.1, 5.0, 5.0];
        let d = separation(&a, &b);
        assert!((d - (TAU - 6.2)).abs() < 1e-12, "{d}");
    }

    #[test]
    fn dp_restarts_every_45_s_with_new_seeded_angles() {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(2);
        g.resize(Size::new(80, 24), &mut ctx);
        let first = g.a;
        for _ in 0..(RUN_SECONDS / SIM_DT).round() as usize - 1 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.run_t > RUN_SECONDS - 0.05);
        g.update(SIM_DT, &mut ctx);
        assert!(g.run_t < 1e-9, "{}", g.run_t);
        assert_ne!(g.a[..2], first[..2]);
        assert_eq!((g.a[2], g.a[3]), (0.0, 0.0));
        assert!((separation(&g.a, &g.b) - TWIN_OFFSET).abs() < 1e-12);
        assert_eq!(g.trail_a.len(), 1);
    }

    #[test]
    fn dp_resize_keeps_the_run() {
        let mut g = DoublePendulum::default();
        let mut ctx = seeded(4);
        g.resize(Size::new(80, 24), &mut ctx);
        for _ in 0..90 {
            g.update(SIM_DT, &mut ctx);
        }
        let (a, t) = (g.a, g.run_t);
        g.resize(Size::new(33, 11), &mut ctx);
        assert_eq!((g.a, g.run_t), (a, t));
    }
}
