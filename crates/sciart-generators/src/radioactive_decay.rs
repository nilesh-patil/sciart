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
//! Each A → B emits an α (short, straight, mono-energetic track); each B → C emits a β⁻
//! (longer, faint, strongly curling in the chamber's magnetic field, with a continuous
//! energy spectrum). An A → C within one step emits both.

use std::f64::consts::{LN_2, TAU};

use rand::Rng;

use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "radioactive_decay",
    category: Category::PhysicsChemistry,
    description: "A lattice of nuclei decaying through a chain beside live Bateman curves",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(RadioactiveDecay::default())
}

/// Half-lives, s (spec).
const T_A: f64 = 8.0;
const T_B: f64 = 3.0;
/// Start over when fewer than this fraction of the nuclei are still radioactive (spec).
const RESET_FRAC: f64 = 0.02;
/// The plot's time axis spans this multiple of the expected (analytic) reset time, so a
/// run that ends a little late still lands on the axis.
const AXIS_SLACK: f64 = 1.08;

/// Plot panel: ≈ 38 % of the width on the right, shown only when it gets ≥ 24×10 cells.
const PANEL_FRAC: f64 = 0.38;
const PANEL_MIN_W: i32 = 24;
const PANEL_MIN_H: i32 = 10;

/// Decay flash and afterglow, s.
const FLASH: f32 = 0.14;
const GLOW: f32 = 0.9;

/// α: mono-energetic, so every α has the same range; barely bent by the field.
const ALPHA_SPEED: f64 = 11.0; // rows/s
const ALPHA_RUN: f64 = 0.3; // s → range 3.3 rows
const ALPHA_OMEGA: f64 = 0.6; // rad/s
const ALPHA_LIFE: f64 = 0.8;
/// β⁻: continuous spectrum (speed drawn per decay), opposite charge ⇒ curls the other way.
const BETA_SPEED: (f64, f64) = (10.0, 30.0);
const BETA_RUN: f64 = 0.35;
const BETA_OMEGA: f64 = -5.5;
const BETA_LIFE: f64 = 0.9;

/// Ink role per species `[A, B, C]`, shared by the lattice and the measured plot points.
const SPECIES_ROLE: [Role; 3] = [Role::Hot, Role::Accent, Role::Dim];

const NUC_A: Glyph = Glyph::new('●', '@');
const NUC_B: Glyph = Glyph::new('◐', 'o');
const NUC_C: Glyph = Glyph::new('○', '.');
const FLASH_ALPHA: Glyph = Glyph::new('✸', '*');
const FLASH_BETA: Glyph = Glyph::new('✦', '+');
const BETA_DOT: Glyph = Glyph::new('·', '.');
/// Track / curve segments by screen direction: horizontal, falling, vertical, rising.
const SEG_H: Glyph = Glyph::new('─', '-');
const SEG_FALL: Glyph = Glyph::new('╲', '\\');
const SEG_V: Glyph = Glyph::new('│', '|');
const SEG_RISE: Glyph = Glyph::new('╱', '/');
/// Sub-row placement of a nearly flat curve inside its cell: upper, middle, lower third.
const SUB_ROW: [Glyph; 3] = [
    Glyph::new('‾', '-'),
    Glyph::new('─', '-'),
    Glyph::new('_', '_'),
];
const AXIS_CORNER: Glyph = Glyph::new('└', '+');
const AXIS_TICK_X: Glyph = Glyph::new('┴', '+');
const AXIS_TICK_Y: Glyph = Glyph::new('┤', '+');
/// `A ─α→ B ─β→ C`.
const TITLE: [Glyph; 13] = [
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

/// A charged-particle track in a cloud chamber with a uniform magnetic field: a circular
/// arc of angular speed `omega`, run for `run` seconds (the particle's range), then left to
/// fade until `life`. Position is closed-form in `age`, so it is exact for any `dt`.
/// Coordinates are in rows (x already divided by [`ASPECT`]) so circles are round.
#[derive(Debug, Clone, Copy)]
struct Track {
    x0: f64,
    y0: f64,
    theta: f64,
    speed: f64,
    omega: f64,
    run: f64,
    life: f64,
    age: f64,
    alpha: bool,
    /// Origin cell, which the track never paints over (that is where the nucleus sits).
    cell: (i32, i32),
}

impl Track {
    /// Position after moving for `s` seconds: the arc
    /// `x = x₀ + (v/ω)(sin(θ + ωs) − sin θ)`, `y = y₀ − (v/ω)(cos(θ + ωs) − cos θ)`.
    fn pos(&self, s: f64) -> (f64, f64) {
        let r = self.speed / self.omega;
        let phi = self.theta + self.omega * s;
        (
            self.x0 + r * (phi.sin() - self.theta.sin()),
            self.y0 - r * (phi.cos() - self.theta.cos()),
        )
    }

    /// Brightness factor: full while moving, then a linear fade.
    fn fade(&self) -> f64 {
        if self.age <= self.run {
            1.0
        } else {
            (1.0 - (self.age - self.run) / (self.life - self.run)).clamp(0.0, 1.0)
        }
    }
}

/// Segment glyph for a screen-space direction `(dx, dy)` (y down).
fn segment(dx: f64, dy: f64) -> Glyph {
    // Fold the angle into [0, π): 0 = horizontal, π/2 = vertical.
    let a = dy.atan2(dx).rem_euclid(std::f64::consts::PI);
    let eighth = std::f64::consts::FRAC_PI_8;
    if !(eighth..7.0 * eighth).contains(&a) {
        SEG_H
    } else if a < 3.0 * eighth {
        SEG_FALL
    } else if a < 5.0 * eighth {
        SEG_V
    } else {
        SEG_RISE
    }
}

/// Write `n` in decimal at `(x, y)` without allocating, clipped at `x_max` (exclusive);
/// returns the column after the last digit.
fn put_num(canvas: &mut Canvas, x: i32, y: i32, n: u32, ink: Ink, x_max: i32) -> i32 {
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    let mut v = n;
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let mut cx = x;
    for &b in &buf[i..] {
        if cx < x_max {
            canvas.put(cx, y, Glyph::ascii(char::from(b)), ink);
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

    /// Fractional row of a population fraction `f ∈ [0, 1]` (1 at `top`, 0 at `bottom`).
    fn row_of(&self, f: f64) -> f64 {
        f64::from(self.bottom) - f.clamp(0.0, 1.0) * f64::from(self.bottom - self.top)
    }
}

#[derive(Debug)]
pub struct RadioactiveDecay {
    /// Time since the current run started, s.
    t: f64,
    /// One nucleus per two columns (terminal cells are ~2:1 tall, so this is square).
    lattice: Grid<Nucleus>,
    /// Screen column of lattice column 0, and the lattice's exclusive right edge.
    x_off: i32,
    lat_w: i32,
    counts: [u32; 3],
    n0: u32,
    panel: Option<Panel>,
    /// Plot time span, s.
    t_axis: f64,
    /// Measured `[N_A, N_B, N_C]` sampled at each plot column's time.
    hist: Vec<[u32; 3]>,
    tracks: ParticlePool<Track>,
    /// Decays of the current step, `(cell index, from, to)`; reserved in `resize`.
    events: Vec<(u32, Kind, Kind)>,
    /// Completed runs (for tests / debugging).
    resets: u32,
}

impl Default for RadioactiveDecay {
    fn default() -> Self {
        Self {
            t: 0.0,
            lattice: Grid::default(),
            x_off: 0,
            lat_w: 0,
            counts: [0; 3],
            n0: 0,
            panel: None,
            t_axis: 1.0,
            hist: Vec::new(),
            tracks: ParticlePool::with_capacity(0),
            events: Vec::new(),
            resets: 0,
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
        self.record();
    }

    /// Time of plot column `c`.
    fn col_time(&self, c: usize, cols: usize) -> f64 {
        self.t_axis * c as f64 / (cols.max(2) - 1) as f64
    }

    /// Append measured samples for every plot column whose time has been reached.
    fn record(&mut self) {
        let Some(p) = self.panel else { return };
        while self.hist.len() < p.cols && self.col_time(self.hist.len(), p.cols) <= self.t + 1e-9 {
            self.hist.push(self.counts);
        }
    }

    /// Screen cell of lattice index `i`.
    fn cell_xy(&self, i: usize) -> (i32, i32) {
        let gw = usize::from(self.lattice.width()).max(1);
        (self.x_off + 2 * (i % gw) as i32, (i / gw) as i32)
    }

    fn spawn_tracks(&mut self, ctx: &mut Ctx) {
        for k in 0..self.events.len() {
            let (i, from, to) = self.events[k];
            let (cx, cy) = self.cell_xy(i as usize);
            let alpha = from == Kind::A;
            let beta = to == Kind::C;
            // A → B: α; B → C: β⁻; A → C within one step: both.
            for is_alpha in [true, false] {
                if (is_alpha && !alpha) || (!is_alpha && !beta) {
                    continue;
                }
                let theta = ctx.rng.random::<f64>() * TAU;
                let (speed, run, omega, life) = if is_alpha {
                    (ALPHA_SPEED, ALPHA_RUN, ALPHA_OMEGA, ALPHA_LIFE)
                } else {
                    let v = ctx.rng.random_range(BETA_SPEED.0..BETA_SPEED.1);
                    (v, BETA_RUN, BETA_OMEGA, BETA_LIFE)
                };
                self.tracks.spawn(Track {
                    x0: f64::from(cx) / ASPECT,
                    y0: f64::from(cy),
                    theta,
                    speed,
                    omega,
                    run,
                    life,
                    age: 0.0,
                    alpha: is_alpha,
                    cell: (cx, cy),
                });
            }
        }
    }

    fn draw_track(&self, canvas: &mut Canvas, tr: &Track) {
        let s_end = tr.age.min(tr.run);
        let len_cells = tr.speed * s_end * ASPECT;
        let n = ((len_cells * 2.0).ceil() as usize).clamp(1, 64);
        let fade = tr.fade();
        let h = i32::from(canvas.size().height());
        let mut last = tr.cell;
        for k in 1..=n {
            let s = s_end * k as f64 / n as f64;
            let (x, y) = tr.pos(s);
            let (col, row) = ((x * ASPECT).round(), y.round());
            if !(0.0..f64::from(self.lat_w)).contains(&col) || !(0.0..f64::from(h)).contains(&row) {
                continue;
            }
            let cell = (col as i32, row as i32);
            if cell == last || cell == tr.cell {
                continue;
            }
            last = cell;
            // Brighter toward the head, where the ionisation is freshest.
            let along = 0.45 + 0.55 * k as f64 / n as f64;
            let (glyph, base, span) = if tr.alpha {
                let phi = tr.theta + tr.omega * s;
                (segment(phi.cos() * ASPECT, phi.sin()), 70.0, 185.0)
            } else {
                (BETA_DOT, 50.0, 150.0)
            };
            let level = (base + span * fade * along) * fade.max(0.35);
            canvas.put(cell.0, cell.1, glyph, Ink::new(Role::Particle, level as u8));
        }
    }

    fn draw_nucleus(&self, canvas: &mut Canvas, i: usize, n: &Nucleus) {
        let (x, y) = self.cell_xy(i);
        // Afterglow ∈ [0, 1]: 1 right after the change, 0 once settled.
        let glow = f64::from((1.0 - n.since / GLOW).clamp(0.0, 1.0));
        // A little fixed per-site texture so the lattice reads as matter, not a pattern.
        let grain =
            ((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)) % 37;
        let grain = f64::from(grain);
        let (glyph, ink) = match n.kind {
            Kind::A => (
                NUC_A,
                Ink::new(
                    SPECIES_ROLE[0],
                    (165.0 + grain + 50.0 * glow).min(255.0) as u8,
                ),
            ),
            Kind::B if n.since < FLASH => (FLASH_ALPHA, Ink::new(Role::Hot, 255)),
            Kind::B => (
                NUC_B,
                Ink::new(SPECIES_ROLE[1], (150.0 + grain + 70.0 * glow) as u8),
            ),
            Kind::C if n.since < FLASH => (FLASH_BETA, Ink::new(Role::Accent, 255)),
            Kind::C => (
                NUC_C,
                Ink::new(SPECIES_ROLE[2], (55.0 + 0.6 * grain + 150.0 * glow) as u8),
            ),
        };
        canvas.put(x, y, glyph, ink);
    }

    fn draw_panel(&self, canvas: &mut Canvas, p: &Panel) {
        let n0 = f64::from(self.n0.max(1));
        let text = Ink::new(Role::Text, 200);
        let faint = Ink::new(Role::Text, 120);
        let axis = Ink::new(Role::Structure, 110);

        // Title and the live populations.
        for (k, g) in TITLE.iter().enumerate() {
            let x = p.x0 + k as i32;
            if x < p.x1 {
                canvas.put(x, 0, *g, text);
            }
        }
        let secs = self.t.max(0.0) as u32;
        let digits = if secs >= 10 { 2 } else { 1 };
        let tx = p.x1 - 2 - digits - 2;
        if tx > p.x0 + TITLE.len() as i32 + 1 {
            canvas.text(tx, 0, "t=", faint);
            let end = put_num(canvas, tx + 2, 0, secs, faint, p.x1);
            canvas.text(end, 0, "s", faint);
        }
        let mut x = p.x0;
        for (k, g) in [NUC_A, NUC_B, NUC_C].into_iter().enumerate() {
            if x + 1 >= p.x1 {
                break;
            }
            // The legend key glyph wears its series' ink; the count stays Text.
            canvas.put(x, 1, g, Ink::new(SPECIES_ROLE[k], 230));
            x = put_num(canvas, x + 1, 1, self.counts[k], text, p.x1) + 2;
        }

        // Axes: y axis with a tick at N₀/2 (read the half-life off the A curve), x axis
        // with ticks every T_A.
        for y in p.top..=p.bottom {
            canvas.put(p.x0, y, SEG_V, axis);
        }
        canvas.put(p.x0, p.row_of(0.5).round() as i32, AXIS_TICK_Y, axis);
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

        // Analytic Bateman curves (thin lines), then measured points on top.
        let half = 0.5 / scale;
        for c in 0..p.cols {
            let t = c as f64 / scale;
            let (l, m, r) = (bateman(t - half), bateman(t), bateman(t + half));
            for (fl, fm, fr, level) in [
                (l.2, m.2, r.2, 80.0),
                (l.1, m.1, r.1, 125.0),
                (l.0, m.0, r.0, 125.0),
            ] {
                self.curve_cell(
                    canvas,
                    p,
                    c,
                    (fl, fm, fr),
                    Ink::new(Role::Structure, level as u8),
                );
            }
        }
        // Measured points in their species' lattice ink, so each series matches its nuclei.
        let marks = [NUC_A, NUC_B, NUC_C];
        let levels = [200u8, 185, 150];
        for (c, h) in self.hist.iter().enumerate() {
            for k in [2, 1, 0] {
                let y = p.row_of(f64::from(h[k]) / n0).round() as i32;
                canvas.put(
                    p.col_x(c),
                    y,
                    marks[k],
                    Ink::new(SPECIES_ROLE[k], levels[k]),
                );
            }
        }
        // The live point at the current time, brightest.
        let c = ((self.t * scale).round() as usize).min(p.cols.saturating_sub(1));
        for k in [2, 1, 0] {
            let y = p.row_of(f64::from(self.counts[k]) / n0).round() as i32;
            canvas.put(p.col_x(c), y, marks[k], Ink::new(SPECIES_ROLE[k], 255));
        }
    }

    /// One column of an analytic curve: the rows it spans across the column's width, drawn
    /// as a sub-row tick when flat and as slope segments when steep.
    fn curve_cell(&self, canvas: &mut Canvas, p: &Panel, c: usize, f: (f64, f64, f64), ink: Ink) {
        let x = p.col_x(c);
        let (yl, ym, yr) = (p.row_of(f.0), p.row_of(f.1), p.row_of(f.2));
        let (lo, hi) = (yl.min(yr).round() as i32, yl.max(yr).round() as i32);
        if lo == hi || (yr - yl).abs() < 0.9 {
            let cell = ym.round();
            let off = ym - cell; // ∈ [−0.5, 0.5]; negative = upper part of the cell
            let g = SUB_ROW[if off < -1.0 / 6.0 {
                0
            } else if off > 1.0 / 6.0 {
                2
            } else {
                1
            }];
            canvas.put(x, cell as i32, g, ink);
        } else {
            let g = if hi - lo >= 2 {
                SEG_V
            } else if yr > yl {
                SEG_FALL
            } else {
                SEG_RISE
            };
            for y in lo..=hi {
                canvas.put(x, y, g, ink);
            }
        }
    }
}

impl Generator for RadioactiveDecay {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        self.panel = Panel::new(w, h);
        // Lattice to the left of the panel with one blank gutter column.
        self.lat_w = self.panel.map_or(w, |p| p.x0 - 2).max(1);
        let gw = ((self.lat_w + 1) / 2).max(1);
        self.x_off = (self.lat_w - (2 * gw - 1)) / 2;
        self.lattice = Grid::new(Size::new(gw as u16, h as u16), Nucleus::FRESH);
        self.n0 = self.lattice.len() as u32;

        self.t_axis = AXIS_SLACK * reset_time();
        let cols = self.panel.map_or(0, |p| p.cols);
        self.hist = Vec::with_capacity(cols);
        // Peak decay rate is N₀ λ_A ≈ 0.087 N₀ /s; tracks live < 1 s. Cap for GEN-R8.
        let cap = (self.n0 as usize / 6).clamp(8, 480);
        self.tracks.set_capacity(cap);
        self.tracks.clear();
        self.events = Vec::with_capacity(cap);
        self.restart();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt > 0.0 && dt.is_finite()) {
            return;
        }
        let tr = Transition::new(dt);
        let counts = &mut self.counts;
        let events = &mut self.events;
        let cap = events.capacity();
        events.clear();
        step_nuclei(
            self.lattice.as_mut_slice(),
            &tr,
            dt,
            &mut ctx.rng,
            |i, from, to| {
                counts[from as usize] -= 1;
                counts[to as usize] += 1;
                // Tracks are decoration: beyond the budget a decay just flashes.
                if events.len() < cap {
                    events.push((i as u32, from, to));
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
    }

    fn render(&self, canvas: &mut Canvas) {
        let cells = self.lattice.as_slice();
        // The lattice, then the tracks streaking across it like a cloud chamber, then the
        // decay flashes on top so every decay is seen at its own nucleus.
        for (i, n) in cells.iter().enumerate() {
            self.draw_nucleus(canvas, i, n);
        }
        for tr in self.tracks.iter() {
            self.draw_track(canvas, tr);
        }
        for (i, n) in cells.iter().enumerate() {
            if n.kind != Kind::A && n.since < FLASH {
                self.draw_nucleus(canvas, i, n);
            }
        }
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
    fn rad_a3_populations_always_sum_to_n0() {
        let mut stage = Stage::new(build(), 3, Size::new(90, 24), GlyphMode::Unicode);
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
        stage.step(SIM_DT);
        let _ = stage.render();
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
        let last = g.cell_xy(g.lattice.len() - 1);
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
            // Nothing but the panel is drawn right of the lattice: tracks are clipped.
            let panel = g.panel.take();
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut canvas);
            g.panel = panel;
            for y in 0..36 {
                for x in g.lat_w..120 {
                    assert_eq!(canvas.get(x, y).map(|c| c.ch), Some(' '), "({x},{y})");
                }
            }
        }
        assert!(seen > 0, "decays emit tracks");
    }
}
