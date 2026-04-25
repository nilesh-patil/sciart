//! Three equal suns dancing periodic three-body orbits: figure-8, Lagrange, Broucke, moth (SPEC-002 `three_body`).
//!
//! A gallery of five published periodic solutions of the planar equal-mass problem,
//! each replayed from its initial conditions for a few periods on a fixed leapfrog
//! lattice, so the state is a pure function of the substep count (TB-A5) and the
//! unstable orbits never get the time to show their instability.

use sciart_core::ink::{self, MIN_VISIBLE_LEVEL};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{geom, ode};

pub static META: Meta = Meta {
    name: "three_body",
    category: Category::Astronomy,
    description: "Three equal suns dancing periodic three-body orbits: figure-8, Lagrange, Broucke, moth",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ThreeBody::new())
}

/// Trail samples per body: exactly one period, so each orbit closes on itself.
const SAMPLES: usize = 480;
/// Leapfrog substeps per model time unit, at least; `qᵢ = ⌈RES·Tᵢ/SAMPLES⌉` (SPEC-002).
const RES: f64 = 4096.0;
/// Seconds each orbit fades out while its physics keeps running.
const FADE: f64 = 1.2;
/// Seconds an orbit (heads, trails, triangle) takes to fade in at each run start, so no
/// orbit starts hard-on.
const FADE_IN: f64 = 0.3;
/// Fraction of its period the moth's trail spans at `MOTH_FULL_SCALE` and above: its
/// single period folds into a knot, so only the newest loops are shown and the older
/// ones dissolve.
const MOTH_TRAIL: f64 = 0.45;
/// Rows per model unit from which the moth shows its full `MOTH_TRAIL` (its 80×24 view
/// is 8.6); smaller views shorten the span in proportion, down to `MOTH_MIN_TRAIL`.
const MOTH_FULL_SCALE: f64 = 8.0;
const MOTH_MIN_TRAIL: f64 = 0.25;
/// Level of Lagrange's triangle once its bodies are in (SPEC-002 Inks).
const TRIANGLE_LEVEL: f64 = 80.0;
/// Body `i` is drawn in `ROLES[i]` (SPEC-002 Inks).
const ROLES: [Role; 3] = [Role::Hot, Role::Cold, Role::Accent];
const HEAD: Glyph = Glyph::new('●', '@');
/// Index of the Lagrange orbit, the one that gets its triangle drawn.
const LAGRANGE: usize = 1;

/// One periodic solution: published initial conditions plus how to show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Orbit {
    pub label: &'static str,
    pub short: &'static str,
    /// Interleaved `(x, y)` of bodies 1..3, COM-centred.
    pub pos: [f64; 6],
    pub vel: [f64; 6],
    /// Period `T` in model time units.
    pub period: f64,
    /// Fraction of the period the trail spans (`1` = the whole period, SPEC-002 Inks);
    /// below 1 it is the span at full size, see [`trail_span`].
    pub trail: f64,
    /// Periods shown per run (`n`).
    pub periods: u32,
    /// Model time units per simulated second (`k`).
    pub rate: f64,
    /// `[x_min, x_max, y_min, y_max]` over run plus fade, padded (TB-A6).
    pub bbox: [f64; 4],
}

impl Orbit {
    /// Substeps per trail sample, `q = ⌈RES·T/480⌉`, so `h = T/(480 q) ≤ 1/RES`.
    pub fn q(&self) -> u64 {
        (RES * self.period / SAMPLES as f64).ceil() as u64
    }

    /// The leapfrog step `h = T/(480 q)`: a whole number of steps per period.
    pub fn h(&self) -> f64 {
        self.period / (SAMPLES as u64 * self.q()) as f64
    }

    /// Seconds of the `run` phase, `n·T/k`.
    pub fn run_secs(&self) -> f64 {
        f64::from(self.periods) * self.period / self.rate
    }

    /// Substeps due `tau` seconds after the run started: `⌊τ·k/h⌋` (TB-A5).
    pub fn substeps_at(&self, tau: f64) -> u64 {
        // Saturating cast: negative or NaN gives 0.
        (tau * self.rate / self.h()).floor() as u64
    }
}

/// Lagrange's homographic solution with eccentricity `e` (SPEC-002): bodies at
/// apocentre on the unit circle at `90° + 120°·i`, moving perpendicular to the radius
/// at `v_a = √(μ(1 − e))`, `μ = 1/√3` being the net central pull `μ/r²` on each body.
/// Returns `(pos, vel, period)` with `T = 2π√(a³/μ)`, `a = 1/(1 + e)`.
pub(crate) fn lagrange_ics(e: f64) -> ([f64; 6], [f64; 6], f64) {
    let mu = 1.0 / 3f64.sqrt();
    let v = (mu * (1.0 - e)).sqrt();
    let (mut pos, mut vel) = ([0.0; 6], [0.0; 6]);
    for i in 0..3 {
        let (s, c) = (90f64 + 120.0 * i as f64).to_radians().sin_cos();
        pos[2 * i] = c;
        pos[2 * i + 1] = s;
        vel[2 * i] = -s * v;
        vel[2 * i + 1] = c * v;
    }
    let a = 1.0 / (1.0 + e);
    (pos, vel, std::f64::consts::TAU * (a * a * a / mu).sqrt())
}

/// The gallery in timeline order (SPEC-002 table; ICs quoted as published).
///
/// The periods of A1, R4 and moth I are refined from the published values (6.283213,
/// 5.396, 14.8939) to the period that minimises the return error of these ICs on the
/// orbit's own leapfrog lattice (TB-A3), so every trail closes on itself.
// Keeps one IC array per line, as the table is published.
#[rustfmt::skip]
pub(crate) fn orbits() -> [Orbit; 5] {
    let (lx, lv, lt) = lagrange_ics(0.3);
    // Figure-8 (Simó's ICs): r₁ = −r₂, r₃ = 0, v₁ = v₂ = −v₃/2.
    let (x1, y1) = (0.970_004_36, -0.243_087_53);
    let (vx3, vy3) = (-0.932_407_37, -0.864_731_46);
    // Moth I: r₁ = (−1, 0), r₂ = (1, 0), r₃ = 0, v₁ = v₂, v₃ = −2 v₁.
    let (mvx, mvy) = (0.464_445, 0.396_060);
    [
        Orbit {
            label: "figure-8  Moore 1993",
            short: "figure-8",
            pos: [x1, y1, -x1, -y1, 0.0, 0.0],
            vel: [-vx3 / 2.0, -vy3 / 2.0, -vx3 / 2.0, -vy3 / 2.0, vx3, vy3],
            period: 6.325_913_98,
            trail: 1.0,
            periods: 3,
            rate: 1.0,
            bbox: [-1.09, 1.09, -0.36, 0.36],
        },
        Orbit {
            label: "Lagrange triangle  1772",
            short: "Lagrange",
            pos: lx,
            vel: lv,
            period: lt,
            trail: 1.0,
            periods: 2,
            rate: 1.0,
            bbox: [-0.97, 0.97, -0.87, 1.01],
        },
        Orbit {
            label: "Broucke A1  1975",
            short: "Broucke A1",
            pos: [-0.989_262_004_3, 0.0, 2.209_617_724_1, 0.0, -1.220_355_719_7, 0.0],
            vel: [0.0, 1.916_924_418_5, 0.0, 0.191_026_873_8, 0.0, -2.107_951_292_4],
            period: 6.283_247,
            trail: 1.0,
            periods: 2,
            rate: 1.0,
            bbox: [-2.22, 2.22, -0.57, 0.57],
        },
        Orbit {
            label: "Broucke R4  1975",
            short: "Broucke R4",
            pos: [0.873_304_709_1, 0.0, -0.625_403_028_8, 0.0, -0.247_901_680_3, 0.0],
            vel: [0.0, 1.010_776_443_6, 0.0, -1.683_353_345_8, 0.0, 0.672_576_902_2],
            period: 5.396_075,
            trail: 1.0,
            periods: 2,
            rate: 1.0,
            bbox: [-0.88, 0.88, -0.88, 0.88],
        },
        Orbit {
            label: "moth I  Suvakov 2013",
            short: "moth I",
            pos: [-1.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            vel: [mvx, mvy, mvx, mvy, -2.0 * mvx, -2.0 * mvy],
            period: 14.894_089,
            trail: MOTH_TRAIL,
            periods: 1,
            rate: 1.5,
            bbox: [-1.13, 1.13, -1.15, 1.15],
        },
    ]
}

/// Newtonian accelerations of three unit masses (`G = 1`, unsoftened) at interleaved
/// `pos`, into `acc`. The symmetric pair loop applies each pair force with opposite
/// signs, so momentum and angular momentum are conserved to rounding (TB-A2).
pub(crate) fn accel3(pos: &[f64], acc: &mut [f64]) {
    acc.fill(0.0);
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let dx = pos[2 * j] - pos[2 * i];
        let dy = pos[2 * j + 1] - pos[2 * i + 1];
        let r2 = dx * dx + dy * dy;
        let inv3 = 1.0 / (r2 * r2.sqrt());
        acc[2 * i] += dx * inv3;
        acc[2 * i + 1] += dy * inv3;
        acc[2 * j] -= dx * inv3;
        acc[2 * j + 1] -= dy * inv3;
    }
}

/// Positions, velocities and the accelerations at `pos` (the leapfrog carry).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bodies {
    pub pos: [f64; 6],
    pub vel: [f64; 6],
    acc: [f64; 6],
}

impl Bodies {
    /// Exactly the orbit's published state (TB-A5 compares bitwise).
    pub fn at_ics(orbit: &Orbit) -> Self {
        let mut acc = [0.0; 6];
        accel3(&orbit.pos, &mut acc);
        Self {
            pos: orbit.pos,
            vel: orbit.vel,
            acc,
        }
    }

    /// One KDK leapfrog step of size `h` (CORE-R44).
    pub fn step(&mut self, h: f64) {
        ode::verlet_step(&mut self.pos, &mut self.vel, &mut self.acc, h, accel3);
    }

    /// Body `i`'s position.
    pub fn body(&self, i: usize) -> (f64, f64) {
        (self.pos[2 * i], self.pos[2 * i + 1])
    }
}

/// Model-to-screen mapping for one orbit at one canvas size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    /// Rows per model unit (x is stretched by `ASPECT`).
    scale: f64,
    /// Canvas centre, in continuous cell coordinates.
    centre: (f64, f64),
    /// Model point drawn at the canvas centre: the bounding box centre.
    origin: (f64, f64),
}

impl View {
    /// Continuous cell coordinates of model point `(x, y)` (cell `i` spans `[i, i+1)`).
    pub fn cell(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.centre.0 + (x - self.origin.0) * self.scale * geom::ASPECT,
            self.centre.1 - (y - self.origin.1) * self.scale,
        )
    }

    /// Braille dot coordinates of `(x, y)` (CORE-R64), so the dot a point rounds to
    /// lies in its cell.
    fn dot(&self, x: f64, y: f64) -> (f64, f64) {
        let (cx, cy) = self.cell(x, y);
        Dots::from_cells(cx, cy)
    }
}

/// The largest view fitting `orbit`'s bounding box in 90 % of the canvas inside a
/// one-cell margin, centred on the box (SPEC-002 Resize).
pub(crate) fn view_for(size: Size, orbit: &Orbit) -> View {
    let [x0, x1, y0, y1] = orbit.bbox;
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    let fit_x = (w / 2.0 - 1.0) / (geom::ASPECT * (x1 - x0) / 2.0);
    let fit_y = (h / 2.0 - 1.0) / ((y1 - y0) / 2.0);
    View {
        scale: (0.9 * fit_x.min(fit_y)).max(0.25),
        centre: (w / 2.0, h / 2.0),
        origin: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
    }
}

/// The orbit's label for a `w`-column canvas: the full form if it fits in `w − 2`
/// columns, else the short form, else none (SPEC-002 Label).
pub(crate) fn label_for(w: u16, orbit: &Orbit) -> Option<&'static str> {
    let room = usize::from(w.saturating_sub(2));
    [orbit.label, orbit.short]
        .into_iter()
        .find(|s| s.len() <= room)
}

/// The fraction of its period `orbit`'s trail spans in a view of `scale` rows per unit.
/// Full-period trails always span 1. A partial one (the moth) keeps its span from
/// `MOTH_FULL_SCALE` up and shortens in proportion below, to no less than
/// `MOTH_MIN_TRAIL`, so a small view shows one clean stroke rather than a knot (TB-A7).
pub(crate) fn trail_span(orbit: &Orbit, scale: f64) -> f64 {
    if orbit.trail >= 1.0 {
        return 1.0;
    }
    (orbit.trail * scale / MOTH_FULL_SCALE).clamp(MOTH_MIN_TRAIL.min(orbit.trail), orbit.trail)
}

/// Trail brightness for a sample of age fraction `f ∈ [0, 1)` (0 = newest) on a trail
/// spanning `span` of the period. A full-period trail keeps a floor of 40, so the whole
/// orbit stays sketched; a shorter one tapers to 0 at `f = span` and its old loops
/// dissolve (SPEC-002 Inks).
fn trail_level(f: f64, span: f64) -> f64 {
    if span >= 1.0 {
        return 40.0 + 160.0 * (1.0 - f).max(0.0).powf(1.5);
    }
    let rest = (1.0 - f / span).max(0.0);
    160.0 * rest.powf(1.5) + 40.0 * rest
}

/// Ink multiplier of everything an orbit draws: faded in over the first `FADE_IN`
/// seconds of a run and out with the orbit (visual only; the physics is untouched).
fn orbit_fade(phase: &Phase) -> f64 {
    ease::smoothstep((phase.tau / FADE_IN).min(1.0)) * phase.fade
}

/// Head brightness, `255·orbit_fade`.
fn head_level(phase: &Phase) -> f64 {
    255.0 * orbit_fade(phase)
}

/// Lagrange's triangle level, `80·orbit_fade`: it rises with the heads, so it never
/// shows before the bodies at its corners (TB-A7).
fn triangle_level(phase: &Phase) -> f64 {
    TRIANGLE_LEVEL * orbit_fade(phase)
}

/// Where the looping gallery stands at sim time `t` (SPEC-002 Motion).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Phase {
    /// Orbit index.
    pub seg: usize,
    /// Seconds since this orbit's run started (the physics keeps going in the fade).
    pub tau: f64,
    /// Ink multiplier: 1 while running, `1 − smoothstep(u)` over the fade.
    pub fade: f64,
}

/// Five `run` phases of `nᵢTᵢ/kᵢ` seconds, each followed by a `FADE` (SPEC-002).
fn timeline(orbits: &[Orbit; 5]) -> Timeline {
    let phases: Vec<(&'static str, f64)> = orbits
        .iter()
        .flat_map(|o| [("run", o.run_secs()), ("fade", FADE)])
        .collect();
    Timeline::new(&phases)
}

pub(crate) struct ThreeBody {
    orbits: [Orbit; 5],
    timeline: Timeline,
    /// Own accumulated simulated time (GEN-R7).
    t: f64,
    phase: Phase,
    /// Substeps taken since the current run started.
    n_done: u64,
    bodies: Bodies,
    trails: [Trail<(f64, f64)>; 3],
    size: Size,
    view: View,
    /// Trails (and Lagrange's triangle), rasterised in `update`/`resize`.
    dots: Dots,
}

impl ThreeBody {
    fn new() -> Self {
        let orbits = orbits();
        let size = Size::new(1, 1);
        Self {
            timeline: timeline(&orbits),
            t: 0.0,
            phase: Phase {
                seg: 0,
                tau: 0.0,
                fade: 1.0,
            },
            n_done: 0,
            bodies: Bodies::at_ics(&orbits[0]),
            trails: std::array::from_fn(|_| Trail::new(SAMPLES)),
            view: view_for(size, &orbits[0]),
            size,
            dots: Dots::default(),
            orbits,
        }
    }

    pub fn phase_at(&self, t: f64) -> Phase {
        let beat = self.timeline.at(t);
        let seg = (beat.index / 2).min(self.orbits.len() - 1);
        if beat.index % 2 == 0 {
            Phase {
                seg,
                tau: beat.local,
                fade: 1.0,
            }
        } else {
            Phase {
                seg,
                tau: self.orbits[seg].run_secs() + beat.local,
                fade: 1.0 - ease::smoothstep(beat.progress),
            }
        }
    }

    /// Restart orbit `seg` bitwise from its ICs with empty trails (TB-A5), and frame it.
    fn start_run(&mut self, seg: usize) {
        self.bodies = Bodies::at_ics(&self.orbits[seg]);
        self.n_done = 0;
        for trail in &mut self.trails {
            trail.clear();
        }
        self.view = view_for(self.size, &self.orbits[seg]);
    }

    /// Step the current orbit until `target` substeps have been taken since its run
    /// started, pushing one trail sample every `q` substeps.
    fn advance_to(&mut self, target: u64) {
        let orbit = self.orbits[self.phase.seg];
        let (h, q) = (orbit.h(), orbit.q());
        while self.n_done < target {
            self.bodies.step(h);
            self.n_done += 1;
            if self.n_done % q == 0 {
                for (i, trail) in self.trails.iter_mut().enumerate() {
                    trail.push(self.bodies.body(i));
                }
            }
        }
    }

    /// Draw the trails (and Lagrange's triangle) onto `dots` (the M10 raster convention).
    fn rasterise(&mut self) {
        self.dots.clear();
        self.trails_onto_dots();
        if self.phase.seg == LAGRANGE {
            self.triangle();
        }
    }

    /// Each body's trail as dot-lattice segments, dimming with age.
    fn trails_onto_dots(&mut self) {
        let (view, fade) = (self.view, orbit_fade(&self.phase));
        let span = trail_span(&self.orbits[self.phase.seg], view.scale);
        for (i, trail) in self.trails.iter().enumerate() {
            let len = trail.len();
            let head = self.bodies.body(i);
            // Consecutive samples, then newest sample → the head itself.
            let pairs = trail.iter().zip(trail.iter().skip(1).chain([&head]));
            for (k, (&a, &b)) in pairs.enumerate() {
                let level = trail_level((len - 1 - k) as f64 / SAMPLES as f64, span) * fade;
                if level < MIN_VISIBLE_LEVEL {
                    continue;
                }
                let (p, q) = (view.dot(a.0, a.1), view.dot(b.0, b.1));
                self.dots
                    .line(p.0, p.1, q.0, q.1, Ink::new(ROLES[i], ink::level(level)));
            }
        }
    }

    /// Lagrange's faint equilateral triangle joining the bodies: one dot per cell, and
    /// only in cells no trail has lit, so it reads as a clean dotted line (a `.` per
    /// cell in ASCII) and never thickens a trail.
    fn triangle(&mut self) {
        let level = triangle_level(&self.phase);
        if level < MIN_VISIBLE_LEVEL {
            return;
        }
        let ink = Ink::new(Role::Dim, ink::level(level));
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (self.bodies.body(i), self.bodies.body(j));
            let (p, q) = (self.view.dot(a.0, a.1), self.view.dot(b.0, b.1));
            // Positions stay inside the orbit's bbox (TB-A6) and the scale is bounded by
            // the canvas, so the rounded ends are small; off-lattice dots are ignored by
            // `Dots::set` (and read as lit here, so they are skipped).
            let side = geom::line(
                p.0.round() as i32,
                p.1.round() as i32,
                q.0.round() as i32,
                q.1.round() as i32,
            );
            for (x, y) in side {
                // Once this side lights a dot in a cell, the cell counts as taken.
                if !self.cell_lit(x, y) {
                    self.dots.set(x, y, ink);
                }
            }
        }
    }

    /// Whether any dot of the cell holding dot `(x, y)` is lit (off-lattice: `true`).
    fn cell_lit(&self, x: i32, y: i32) -> bool {
        let (cx, cy) = (x.div_euclid(Dots::CELL_W), y.div_euclid(Dots::CELL_H));
        self.dots.cell_mask(cx, cy).is_none_or(|m| m != 0)
    }
}

impl Generator for ThreeBody {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        // Physics and trails live in model units: only the view changes (SPEC-002).
        self.size = size;
        self.view = view_for(size, &self.orbits[self.phase.seg]);
        self.dots.resize(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        self.t += dt;
        let phase = self.phase_at(self.t);
        let target = self.orbits[phase.seg].substeps_at(phase.tau);
        // A new orbit, or the same one again after a full loop.
        let restart = phase.seg != self.phase.seg || target < self.n_done;
        self.phase = phase;
        if restart {
            self.start_run(phase.seg);
        }
        self.advance_to(target);
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        let level = head_level(&self.phase);
        if level >= MIN_VISIBLE_LEVEL {
            for (i, role) in ROLES.iter().enumerate() {
                let (x, y) = self.bodies.body(i);
                let (cx, cy) = self.view.cell(x, y);
                canvas.put(
                    cx.floor() as i32,
                    cy.floor() as i32,
                    HEAD,
                    Ink::new(*role, ink::level(level)),
                );
            }
        }
        let orbit = &self.orbits[self.phase.seg];
        if let Some(label) = label_for(canvas.size().width(), orbit) {
            canvas.text(1, 0, label, Ink::new(Role::Text, 150));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The conserved quantities the acceptance tests track.
    impl Bodies {
        /// Total energy `Σ v²/2 − Σ 1/rᵢⱼ`.
        fn energy(&self) -> f64 {
            let kinetic: f64 = self.vel.iter().map(|v| 0.5 * v * v).sum();
            let potential: f64 = [(0, 1), (0, 2), (1, 2)]
                .iter()
                .map(|&(i, j)| -1.0 / self.dist(i, j))
                .sum();
            kinetic + potential
        }

        /// Total momentum `Σ vᵢ` (unit masses).
        fn momentum(&self) -> (f64, f64) {
            (0..3).fold((0.0, 0.0), |(px, py), i| {
                (px + self.vel[2 * i], py + self.vel[2 * i + 1])
            })
        }

        /// Total angular momentum `Σ xᵢ v_yᵢ − yᵢ v_xᵢ` about the origin.
        fn ang_mom(&self) -> f64 {
            (0..3)
                .map(|i| {
                    self.pos[2 * i] * self.vel[2 * i + 1] - self.pos[2 * i + 1] * self.vel[2 * i]
                })
                .sum()
        }

        /// Distance between bodies `i` and `j`.
        fn dist(&self, i: usize, j: usize) -> f64 {
            let (a, b) = (self.body(i), self.body(j));
            (b.0 - a.0).hypot(b.1 - a.1)
        }
    }

    /// Substeps covering an orbit's run plus fade.
    fn run_and_fade_steps(o: &Orbit) -> u64 {
        o.substeps_at(o.run_secs() + FADE)
    }

    /// Max position distance from the ICs.
    fn ic_error(b: &Bodies, o: &Orbit) -> f64 {
        (0..3)
            .map(|i| (b.pos[2 * i] - o.pos[2 * i]).hypot(b.pos[2 * i + 1] - o.pos[2 * i + 1]))
            .fold(0.0, f64::max)
    }

    #[test]
    fn tb_a1_figure_eight_closes_after_one_period_and_conserves_energy() {
        let o = orbits()[0];
        assert_eq!(o.q(), 54);
        let mut b = Bodies::at_ics(&o);
        let e0 = b.energy();
        assert!((e0 + 1.287_142).abs() < 1e-6, "E0 = {e0}");
        assert!(b.ang_mom().abs() < 1e-12);
        let per_period = SAMPLES as u64 * o.q();
        assert_eq!(per_period, 25_920);
        let mut max_de = 0.0f64;
        for n in 1..=3 * per_period {
            b.step(o.h());
            max_de = max_de.max(((b.energy() - e0) / e0).abs());
            if n == per_period {
                assert!(ic_error(&b, &o) < 1e-5, "return {}", ic_error(&b, &o));
            }
        }
        assert!(max_de < 1e-6, "max dE/E = {max_de}");
    }

    #[test]
    fn tb_a2_every_orbit_conserves_momentum_angular_momentum_and_energy() {
        let table = [
            (-1.287_142, 0.0, 1e-5),
            (-1.125_833, 1.907_172, 1e-5),
            (-0.854_131, 1.098_206, 1e-5),
            (-2.054_282, 1.768_757, 1e-5),
            (-1.382_282, 0.0, 1e-3),
        ];
        for (o, (e_ref, l_ref, de_tol)) in orbits().iter().zip(table) {
            let mut b = Bodies::at_ics(o);
            let (px, py) = b.momentum();
            assert!(px.hypot(py) < 1e-9, "{}: P0 = ({px}, {py})", o.short);
            let (e0, l0) = (b.energy(), b.ang_mom());
            assert!((e0 - e_ref).abs() < 1e-6, "{}: E0 = {e0}", o.short);
            assert!((l0 - l_ref).abs() < 1e-6, "{}: L0 = {l0}", o.short);
            let (mut max_dp, mut max_dl, mut max_de) = (0.0f64, 0.0f64, 0.0f64);
            for _ in 0..run_and_fade_steps(o) {
                b.step(o.h());
                let (qx, qy) = b.momentum();
                max_dp = max_dp.max((qx - px).hypot(qy - py));
                max_dl = max_dl.max((b.ang_mom() - l0).abs());
                max_de = max_de.max(((b.energy() - e0) / e0).abs());
            }
            assert!(max_dp < 1e-12, "{}: dP = {max_dp}", o.short);
            assert!(max_dl < 1e-12, "{}: dL = {max_dl}", o.short);
            assert!(max_de < de_tol, "{}: dE/E = {max_de}", o.short);
        }
    }

    /// Max return error over the orbit's shown periods on its own lattice.
    fn return_error(o: &Orbit) -> f64 {
        let mut b = Bodies::at_ics(o);
        let per_period = SAMPLES as u64 * o.q();
        let mut worst = 0.0f64;
        for n in 1..=u64::from(o.periods) * per_period {
            b.step(o.h());
            if n % per_period == 0 {
                worst = worst.max(ic_error(&b, o));
            }
        }
        worst
    }

    #[test]
    fn tb_a3_orbits_return_each_period_and_never_collide() {
        let qs: Vec<u64> = orbits().iter().map(Orbit::q).collect();
        assert_eq!(qs, [54, 48, 54, 47, 128]);
        // Per-orbit return tolerances, ≈ 1.2–3× the measured error.
        let tol = [2e-6, 1e-6, 8e-5, 2e-5, 3e-4];
        for (o, tol) in orbits().iter().zip(tol) {
            let err = return_error(o);
            assert!(err < tol, "{}: return error {err}", o.short);
            let mut b = Bodies::at_ics(o);
            let mut dmin = f64::INFINITY;
            for _ in 1..=run_and_fade_steps(o) {
                b.step(o.h());
                dmin = dmin.min(b.dist(0, 1).min(b.dist(0, 2)).min(b.dist(1, 2)));
            }
            assert!(dmin >= 0.07, "{}: closest approach {dmin}", o.short);
        }
        // The refined periods of A1, R4 and moth minimise the return error on their
        // lattice: nudging T either way by 2e-5 does not close the orbit better.
        for seg in [2, 3, 4] {
            let o = orbits()[seg];
            let err = return_error(&o);
            for dt in [-2e-5, 2e-5] {
                let nudged = Orbit {
                    period: o.period + dt,
                    ..o
                };
                let e = return_error(&nudged);
                assert!(e > err, "{}: T{dt:+} gives {e} ≤ {err}", o.short);
            }
        }
        // The published (rounded) periods fail those tolerances: measured 1.1e-4 (A1),
        // 2.5e-4 (R4) and 3.6e-4 (moth, a small margin: its lattice error is mostly
        // leapfrog truncation at the 0.079 close approach, not the period).
        for (seg, published) in [(2, 6.283_213), (3, 5.396), (4, 14.893_9)] {
            let o = Orbit {
                period: published,
                ..orbits()[seg]
            };
            let e = return_error(&o);
            assert!(e > tol[seg], "{} at T = {published}: {e}", o.short);
        }
        let (_, _, t) = lagrange_ics(0.3);
        let a = 1.0 / 1.3f64;
        assert!((t - std::f64::consts::TAU * (a.powi(3) * 3f64.sqrt()).sqrt()).abs() < 1e-12);
    }

    /// `max side / min side − 1` of the triangle.
    fn side_deviation(b: &Bodies) -> f64 {
        let s = [b.dist(0, 1), b.dist(1, 2), b.dist(2, 0)];
        s.iter().copied().fold(0.0, f64::max) / s.iter().copied().fold(f64::INFINITY, f64::min)
            - 1.0
    }

    #[test]
    fn tb_a4_lagrange_stays_equilateral_while_shown_then_diverges() {
        let o = orbits()[LAGRANGE];
        let mut b = Bodies::at_ics(&o);
        let per_period = SAMPLES as u64 * o.q();
        let mut dev = [0.0f64; 7]; // max side deviation within each period (1-based)
        for n in 0..6 * per_period {
            b.step(o.h());
            let p = (n / per_period + 1) as usize;
            dev[p] = dev[p].max(side_deviation(&b));
            if p <= 2 {
                let r = b.body(0).0.hypot(b.body(0).1);
                assert!((0.538_462 - 1e-6..=1.0 + 1e-6).contains(&r), "r = {r}");
            }
        }
        assert!(dev[1].max(dev[2]) < 1e-8, "shown periods: {dev:?}");
        let growth = (dev[6] / dev[3]).powf(1.0 / 3.0);
        assert!((30.0..=300.0).contains(&growth), "growth {growth}, {dev:?}");
    }

    #[test]
    fn tb_a5_timeline_loops_and_substeps_are_frame_rate_independent() {
        let g = ThreeBody::new();
        let expected: f64 = g.orbits.iter().map(Orbit::run_secs).sum::<f64>() + 5.0 * FADE;
        assert!((g.timeline.total() - expected).abs() < 1e-9);
        assert!(
            (g.timeline.total() - 69.42).abs() < 0.01,
            "{}",
            g.timeline.total()
        );
        assert_eq!(g.phase_at(3.0), g.phase_at(3.0 + g.timeline.total()));

        let mut ctx = Ctx::new(0, Size::new(80, 24));
        let (mut fine, mut coarse) = (ThreeBody::new(), ThreeBody::new());
        fine.resize(Size::new(80, 24), &mut ctx);
        coarse.resize(Size::new(80, 24), &mut ctx);
        for _ in 0..70 {
            for _ in 0..60 {
                fine.update(SIM_DT, &mut ctx);
            }
            for _ in 0..4 {
                let (seg, before) = (coarse.phase.seg, coarse.n_done);
                coarse.update(0.25, &mut ctx);
                let taken = if coarse.phase.seg == seg && coarse.n_done >= before {
                    coarse.n_done - before
                } else {
                    coarse.n_done
                };
                assert!(taken <= 1548, "one update(0.25) took {taken} substeps");
            }
            assert_eq!(
                (fine.phase.seg, fine.n_done),
                (coarse.phase.seg, coarse.n_done)
            );
            assert_eq!(fine.bodies.pos, coarse.bodies.pos);
        }
    }

    #[test]
    fn tb_a5_every_run_start_restores_the_ics_bitwise_with_empty_trails() {
        let mut ctx = Ctx::new(0, Size::new(80, 24));
        let mut g = ThreeBody::new();
        g.resize(Size::new(80, 24), &mut ctx);
        let mut start = 0.0;
        for seg in 0..=5 {
            // Land just inside run `seg` (seg 5 = the figure-8 again, next loop), from
            // deep inside the previous fade.
            g.update(1.0, &mut ctx);
            g.t = start - 0.5;
            g.update(0.5 + 1e-6, &mut ctx);
            let o = g.orbits[seg % 5];
            assert_eq!(g.phase.seg, seg % 5);
            assert_eq!(g.n_done, 0);
            assert_eq!(g.bodies, Bodies::at_ics(&o));
            assert!(g.trails.iter().all(Trail::is_empty));
            start += o.run_secs() + FADE;
        }
    }

    #[test]
    fn tb_a6_orbits_stay_in_their_boxes_and_on_canvas_with_labels_that_fit() {
        for o in &orbits() {
            let [x0, x1, y0, y1] = o.bbox;
            let views = [Size::new(80, 24), Size::new(200, 50)].map(|s| (s, view_for(s, o)));
            let mut b = Bodies::at_ics(o);
            for _ in 0..run_and_fade_steps(o) {
                b.step(o.h());
                for i in 0..3 {
                    let (x, y) = b.body(i);
                    assert!(
                        (x0..=x1).contains(&x) && (y0..=y1).contains(&y),
                        "{}",
                        o.short
                    );
                    for (size, view) in views {
                        let (cx, cy) = view.cell(x, y);
                        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
                        assert!(cx >= 1.0 && cx < w - 1.0, "{}: x {cx} at {size:?}", o.short);
                        assert!(cy >= 1.0 && cy < h - 1.0, "{}: y {cy} at {size:?}", o.short);
                    }
                }
            }
            assert!(o.label.is_ascii() && o.short.is_ascii());
            assert_eq!(label_for(80, o), Some(o.label));
            let full = o.label.len() as u16 + 2;
            assert_eq!(label_for(full, o), Some(o.label));
            assert_eq!(label_for(full - 1, o), Some(o.short));
            assert_eq!(label_for(o.short.len() as u16 + 1, o), None);
        }
        let mut stage = Stage::new(build(), 1, Size::new(80, 24), GlyphMode::Unicode);
        let row: String = stage.render().to_lines()[0].chars().take(22).collect();
        assert_eq!(row, " figure-8  Moore 1993 ");
    }

    #[test]
    fn tb_a7_moth_shows_only_its_newest_loops_and_the_others_a_full_period() {
        let spans: Vec<f64> = orbits().iter().map(|o| o.trail).collect();
        assert_eq!(spans, [1.0, 1.0, 1.0, 1.0, MOTH_TRAIL]);
        // The moth keeps its full span from 80×24 up; smaller views shorten it (never
        // below 0.25) so it stays one stroke, not a knot. Other orbits always span 1.
        let moth = orbits()[4];
        let span_at = |w, h| trail_span(&moth, view_for(Size::new(w, h), &moth).scale);
        assert_eq!(span_at(80, 24), MOTH_TRAIL);
        assert_eq!(span_at(200, 50), MOTH_TRAIL);
        assert_eq!(span_at(40, 12), MOTH_MIN_TRAIL);
        let mid = span_at(60, 18);
        assert!(MOTH_MIN_TRAIL < mid && mid < MOTH_TRAIL, "60×18 span {mid}");
        assert!(span_at(40, 12) <= span_at(50, 15) && span_at(50, 15) <= mid);
        for o in &orbits()[..4] {
            assert_eq!(trail_span(o, 0.25), 1.0);
        }
        // Full-period trails keep the published ramp, floor 40 at the oldest sample.
        for i in 0..SAMPLES {
            let f = i as f64 / SAMPLES as f64;
            let expect = 40.0 + 160.0 * (1.0 - f).powf(1.5);
            assert!((trail_level(f, 1.0) - expect).abs() < 1e-12);
        }
        // The moth's ramp starts at 200, falls strictly and is gone by the span's end.
        assert_eq!(trail_level(0.0, MOTH_TRAIL), 200.0);
        let fs: Vec<f64> = (0..=100)
            .map(|i| f64::from(i) / 100.0 * MOTH_TRAIL)
            .collect();
        for w in fs.windows(2) {
            assert!(trail_level(w[1], MOTH_TRAIL) < trail_level(w[0], MOTH_TRAIL));
        }
        for f in [MOTH_TRAIL, 0.8, 0.999] {
            assert!(trail_level(f, MOTH_TRAIL) < MIN_VISIBLE_LEVEL, "f = {f}");
        }

        // At the end of the moth's run every lit dot lies near a sample younger than
        // the span: the old loops are gone, not just dim.
        let mut ctx = Ctx::new(0, Size::new(80, 24));
        let mut g = ThreeBody::new();
        g.resize(Size::new(80, 24), &mut ctx);
        let before: f64 = g.orbits[..4].iter().map(|o| o.run_secs() + FADE).sum();
        g.t = before + g.orbits[4].run_secs() - 0.05 - SIM_DT;
        g.update(SIM_DT, &mut ctx);
        assert_eq!(g.phase.seg, 4);
        let mut young = Vec::new();
        for trail in &g.trails {
            let len = trail.len();
            for (k, &(x, y)) in trail.iter().enumerate() {
                if ((len - 1 - k) as f64) < MOTH_TRAIL * SAMPLES as f64 {
                    young.push(g.view.dot(x, y));
                }
            }
        }
        let dots = g.dots.dot_size();
        let mut lit = 0;
        for py in 0..i32::from(dots.height()) {
            for px in 0..i32::from(dots.width()) {
                if g.dots.is_lit(px, py) {
                    lit += 1;
                    let (x, y) = (f64::from(px), f64::from(py));
                    let near = young.iter().any(|&(u, v)| (u - x).hypot(v - y) < 6.0);
                    assert!(near, "old moth dot at {px},{py}");
                }
            }
        }
        assert!(lit > 100, "{lit} lit dots");
    }

    #[test]
    fn tb_a7_heads_and_lagrange_triangle_fade_in_at_every_run_start() {
        let run = |tau: f64, fade: f64| head_level(&Phase { seg: 0, tau, fade });
        assert_eq!(run(0.0, 1.0), 0.0);
        assert!(run(0.1, 1.0) > 0.0 && run(0.1, 1.0) < run(0.2, 1.0));
        assert_eq!(run(FADE_IN, 1.0), 255.0);
        assert_eq!(run(5.0, 1.0), 255.0);
        assert_eq!(run(5.0, 0.5), 127.5);
        // The triangle rises with the heads, to 80, and fades with the orbit.
        let tri = |tau: f64, fade: f64| {
            triangle_level(&Phase {
                seg: LAGRANGE,
                tau,
                fade,
            })
        };
        assert_eq!(tri(0.0, 1.0), 0.0);
        assert!(tri(0.1, 1.0) > 0.0 && tri(0.1, 1.0) < tri(0.2, 1.0));
        assert!((tri(FADE_IN, 1.0) - 80.0).abs() < 1e-12);
        assert!((tri(5.0, 0.5) - 40.0).abs() < 1e-12);
        // Seen in dots: nothing is lit on the Lagrange run's first frame (empty trails,
        // triangle still dark); by 0.3 s the triangle is there.
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(0, size);
        let mut g = ThreeBody::new();
        g.resize(size, &mut ctx);
        let start = g.orbits[0].run_secs() + FADE;
        let lit = |d: &Dots| {
            let s = d.dot_size();
            (0..i32::from(s.height()))
                .flat_map(|y| (0..i32::from(s.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| d.is_lit(x, y))
                .count()
        };
        g.t = start;
        g.update(SIM_DT, &mut ctx);
        assert_eq!((g.phase.seg, lit(&g.dots)), (LAGRANGE, 0));
        g.t = start + FADE_IN - SIM_DT;
        g.update(SIM_DT, &mut ctx);
        g.dots.clear();
        g.trails_onto_dots();
        let trails = lit(&g.dots);
        g.triangle();
        assert!(lit(&g.dots) >= trails + 20, "no triangle at τ = 0.3 s");

        // Seen in frames: no head on the scene's first frame, all three by 0.3 s.
        let mut stage = Stage::new(build(), 1, Size::new(80, 24), GlyphMode::Ascii);
        let heads =
            |lines: Vec<String>| -> usize { lines.iter().map(|l| l.matches('@').count()).sum() };
        assert_eq!(heads(stage.render().to_lines()), 0);
        for _ in 0..18 {
            stage.step(SIM_DT);
        }
        assert_eq!(heads(stage.render().to_lines()), 3);
    }

    /// Lit dots in cell `(cx, cy)`.
    fn cell_dots(d: &Dots, cx: i32, cy: i32) -> usize {
        (0..8)
            .filter(|k| d.is_lit(2 * cx + k % 2, 4 * cy + k / 2))
            .count()
    }

    #[test]
    fn tb_a7_lagrange_triangle_is_one_dot_per_empty_cell() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(0, size);
        let mut g = ThreeBody::new();
        g.resize(size, &mut ctx);
        let start = g.orbits[0].run_secs() + FADE;
        for t in [1.0, 4.0, 9.0] {
            g.t = start + t - SIM_DT;
            g.update(SIM_DT, &mut ctx);
            assert_eq!(g.phase.seg, LAGRANGE);
            g.dots.clear();
            g.trails_onto_dots();
            let trails = g.dots.clone();
            g.triangle();
            let mut added = 0;
            for cy in 0..24 {
                for cx in 0..80 {
                    let before = cell_dots(&trails, cx, cy);
                    let after = cell_dots(&g.dots, cx, cy);
                    if before > 0 {
                        assert_eq!(after, before, "triangle merged into trail cell {cx},{cy}");
                    } else {
                        assert!(after <= 1, "{after} triangle dots in cell {cx},{cy}");
                        added += after;
                    }
                }
            }
            assert!(added >= 20, "only {added} triangle dots at t = {t}");
        }
    }
}
