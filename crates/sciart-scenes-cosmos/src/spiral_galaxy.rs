//! A grand-design spiral as Kalnajs' nested six-lobed orbit rosettes (SPEC-002
//! `spiral_galaxy`).
//!
//! M18 (orbital-thread style with six arms, GEN-R26): the density-wave picture made
//! visible. Thirty-six nested star orbits, `r = r_j (1 − ε cos 6(α − θ_j))` in the frame
//! turning with the pattern, each turned a little more than the one inside it
//! (`θ_j = cot p · ln r_j`), are drawn as faint dotted threads; where they crowd, six
//! trailing arms appear. Stars ride the rosettes faster than the pattern turns (inside
//! corotation), so the arms are a traffic jam, not material. The crowding of the orbits
//! is closed form, `1/|∂r/∂r_j| = 1/((1 − ε cos ψ) − 6ε cot p sin ψ)` with
//! `ψ = 6(α − θ_j)`, and stays finite because `6ε cot p < 1` (the orbits never cross):
//! young stars shine blue `Cold` where it is highest, H II knots flare `Accent` on the
//! ridge, and a dust lane is a dark gap on each arm's inner edge. The bulge is a
//! `Sun`/`Hot` stipple with fixed per-dot thresholds (CORE-R107). Lengths are in units of
//! the disc radius `R`, angles in radians; positions are closed form in time (no
//! integration).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::stipple_keep;
use sciart_core::rng::{cell_key, hash01};

pub static META: Meta = Meta {
    name: "spiral_galaxy",
    category: Category::DeepCosmos,
    description: "Nested star orbits crowding into the six arms of a grand-design spiral",
    title: "Spiral galaxy · density waves",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SpiralGalaxy::default())
}

/// Number of arms: the orbits are `ARMS`-lobed rosettes in the pattern frame.
pub(crate) const ARMS: f64 = 6.0;
/// Orbits: count, innermost and outermost guiding radius (log-spaced, so each is turned
/// by the same angle from the one inside it).
pub(crate) const ORBITS: usize = 36;
const R_IN: f64 = 0.22;
const R_OUT: f64 = 1.0;
/// Rosette amplitude. With `COT_PITCH` it sets `6ε cot p = 0.70` (≤ 0.75): the orbits
/// never cross and the crowding peaks at `1/(1 − ε√(1 + 36 cot²p)) ≈ 3.4`.
pub(crate) const ECC: f64 = 0.09;
/// Winding: `cot p` of the arms (pitch 37.6°), open enough that each arm winds about
/// 110° from the inner orbit to the rim and the six read as a pinwheel, not rings.
pub(crate) const COT_PITCH: f64 = 1.3;
/// Pattern speed (one turn in 150 s) and corotation just outside the disc, so every
/// star overtakes the pattern.
pub(crate) const OMEGA_P: f64 = TAU / 150.0;
#[cfg(test)]
pub(crate) const COROTATION: f64 = 1.12;
/// Rotation-curve softening radius and the flat speed that puts corotation at
/// `COROTATION`.
const SOFT: f64 = 0.1;
const V_FLAT: f64 = OMEGA_P * 1.124_455_423_7; // √(1.12² + 0.1²)
/// Thread samples per orbit: one every ~`THREAD_GAP` dots of circumference, capped.
const THREAD_GAP: f64 = 2.6;
const THREAD_MIN: usize = 24;
const THREAD_MAX: usize = 256;
/// Neighbouring drawn orbit threads are at least this many dots apart at 0.5 R (small
/// frames draw every second, third or fourth orbit).
const THREAD_SPACING: f64 = 3.5;
/// Stars per orbit (fixed per orbit; more on the outer ones, which are longer).
const STARS_MIN: usize = 60;
const STARS_MAX: usize = 220;
/// Crowding above which a stretch of orbit is on the arm ridge (threads brighter, young
/// stars lit) and the crowding above which an H II knot flares.
pub(crate) const RIDGE: f64 = 2.5;
const KNOT_CROWD: f64 = 3.1;
const KNOT_SHARE: f64 = 0.1;
/// Dust lane: the arm's inner edge, `ψ = ψ* + DUST_OFFSET ± DUST_HALF` (ψ* the crowding
/// peak; larger ψ is smaller radius at fixed angle), where threads and stars are hidden.
pub(crate) const DUST_OFFSET: f64 = 0.55;
const DUST_HALF: f64 = 0.22;
/// Young stars: share of the stars that are young (they shine only on the ridge).
const YOUNG_SHARE: f64 = 0.45;
/// Inks: the orbit threads faint (`Dim`, brighter on the ridge), the stars brighter, the
/// bulge `Sun` core and `Hot` body.
pub(crate) const THREAD_INKS: [Ink; 2] = [Ink::new(Role::Dim, 50), Ink::new(Role::Dim, 110)];
pub(crate) const OLD_INK: Ink = Ink::new(Role::Sun, 130);
/// Old stars show only where the orbits crowd at least this much (the arms' flanks), so
/// the inter-arm disc is nearly black.
const OLD_CROWD: f64 = 1.7;
pub(crate) const YOUNG_INK: Ink = Ink::new(Role::Cold, 255);
pub(crate) const KNOT_INK: Ink = Ink::new(Role::Accent, 235);
pub(crate) const BULGE_INKS: [Ink; 2] = [Ink::new(Role::Sun, 235), Ink::new(Role::Hot, 205)];
/// Bulge stipple: keep `BULGE_P0·exp(−r/BULGE_SCALE)` within `BULGE_R` (units of R).
const BULGE_R: f64 = 0.14;
const BULGE_P0: f64 = 0.95;
const BULGE_SCALE: f64 = 0.04;
const BULGE_CORE: f64 = 0.04;
const BULGE_SALT: u64 = 0xB01_6E00_0000_0001;
/// View: inclination (cos 25°) and the position angle of the major axis on screen.
const COS_INC: f64 = 0.906_307_787_036_649_9;
const PA: f64 = 0.35;
/// Foreground stars of our own Galaxy, per cell: dim and bright, one dot each.
const FIELD_INKS: [Ink; 2] = [Ink::new(Role::Dim, 120), Ink::new(Role::Star, 170)];
const FIELD_PER_CELL: f64 = 1.0 / 110.0;

/// All stars are drawn from `KEEP_FULL_R` dots of disc radius up, a share in proportion
/// below (at least `KEEP_MIN`).
const KEEP_FULL_R: f64 = 75.0;
const KEEP_MIN: f64 = 0.25;
/// The old disc is drawn in full from `OLD_FULL_R` dots of disc radius up, and in
/// proportion below (at least `OLD_MIN_KEEP`).
const OLD_FULL_R: f64 = 100.0;
const OLD_MIN_KEEP: f64 = 0.2;

/// Angle of thread sample `i` of `n` on orbit `j`: each orbit's samples are offset by a
/// golden-ratio fraction of a step, so the dotted hints of neighbouring orbits never line
/// up into spokes.
fn sample_angle(j: usize, i: usize, n: usize) -> f64 {
    let offset = (j as f64 * 0.618_033_988_749_895).fract();
    TAU * (i as f64 + offset) / n as f64
}

/// `Ω(r)`: flat rotation curve, softened inside `SOFT` (rad/s).
pub(crate) fn omega(r: f64) -> f64 {
    V_FLAT / (r * r + SOFT * SOFT).sqrt()
}

/// Guiding radius of orbit `j`.
pub(crate) fn orbit_radius(j: usize) -> f64 {
    R_IN * (R_OUT / R_IN).powf(j as f64 / (ORBITS - 1) as f64)
}

/// Orientation of orbit `j`'s lobes in the pattern frame: `cot p · ln r_j`, so the
/// rosettes turn steadily outward and crowd along logarithmic spirals.
pub(crate) fn orbit_turn(j: usize) -> f64 {
    COT_PITCH * orbit_radius(j).ln()
}

/// The orbit phase `ψ*` where the rosettes crowd most: `tan ψ* = 6 cot p`.
pub(crate) fn ridge_phase() -> f64 {
    (ARMS * COT_PITCH).atan()
}

/// The crowding (surface density of orbits relative to an unperturbed disc) at orbit
/// phase `ψ = 6(α − θ_j)`, from `(cos ψ, sin ψ)`.
pub(crate) fn crowding(cos_psi: f64, sin_psi: f64) -> f64 {
    let d = (1.0 - ECC * cos_psi) - ARMS * ECC * COT_PITCH * sin_psi;
    1.0 / d.max(1e-3)
}

/// Whether orbit phase `ψ` lies in the dust lane on the arm's inner edge.
pub(crate) fn in_lane(cos_psi: f64, sin_psi: f64) -> bool {
    let (s, c) = (ridge_phase() + DUST_OFFSET).sin_cos_det();
    // cos(ψ − ψ_lane) > cos(DUST_HALF)
    cos_psi * c + sin_psi * s > DUST_HALF.cos_det()
}

/// Pattern-frame radius of orbit `j` at angle `α`.
pub(crate) fn orbit_r(j: usize, alpha: f64) -> f64 {
    orbit_radius(j) * (1.0 - ECC * (ARMS * (alpha - orbit_turn(j))).cos_det())
}

/// Pattern-frame position of a point at angle `α` on orbit `j`.
pub(crate) fn orbit_point(j: usize, alpha: f64) -> (f64, f64) {
    let r = orbit_r(j, alpha);
    let (s, c) = alpha.sin_cos_det();
    (r * c, r * s)
}

/// The per-orbit constants a star needs: guiding radius, lobe turn and the rate at which
/// it overtakes the pattern, `Ω(r_j) − Ω_p`.
#[derive(Debug, Clone, Copy)]
struct Orbit {
    r: f64,
    turn: f64,
    rate: f64,
}

impl Orbit {
    fn of(j: usize) -> Self {
        let r = orbit_radius(j);
        Self {
            r,
            turn: orbit_turn(j),
            rate: omega(r) - OMEGA_P,
        }
    }
}

/// One star: its orbit, its pattern-frame angle at `t = 0`, and a fixed uniform that
/// decides its population (young if below `YOUNG_SHARE`) and whether it flares as a knot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Star {
    orbit: u8,
    alpha0: f32,
    u: f32,
    /// A second fixed uniform: the star is drawn while it is below the frame's keep share.
    keep: f32,
}

impl Star {
    /// Pattern-frame angle at `t`: the star overtakes the pattern at `Ω(r_j) − Ω_p`.
    #[cfg(test)]
    pub(crate) fn alpha(&self, t: f64) -> f64 {
        let r = orbit_radius(usize::from(self.orbit));
        f64::from(self.alpha0) + (omega(r) - OMEGA_P) * t
    }

    /// Pattern-frame position and `(cos ψ, sin ψ)` at `t`.
    #[cfg(test)]
    fn state(&self, t: f64) -> ((f64, f64), (f64, f64)) {
        self.state_on(&Orbit::of(usize::from(self.orbit)), t)
    }

    /// [`state`](Self::state) with the orbit's constants precomputed.
    #[inline]
    fn state_on(&self, o: &Orbit, t: f64) -> ((f64, f64), (f64, f64)) {
        let alpha = f64::from(self.alpha0) + o.rate * t;
        let (s, c) = alpha.sin_cos_det();
        let (sp, cp) = (ARMS * (alpha - o.turn)).sin_cos_det();
        let r = o.r * (1.0 - ECC * cp);
        ((r * c, r * s), (cp, sp))
    }

    /// The ink of the star now, or `None` (hidden in a lane, or a young star off the
    /// ridge).
    pub(crate) fn ink(&self, cp: f64, sp: f64, old_keep: f64) -> Option<Ink> {
        if in_lane(cp, sp) {
            return None;
        }
        let crowd = crowding(cp, sp);
        let u = f64::from(self.u);
        if u < YOUNG_SHARE {
            if crowd >= KNOT_CROWD && u < KNOT_SHARE * YOUNG_SHARE {
                Some(KNOT_INK)
            } else {
                (crowd >= RIDGE).then_some(YOUNG_INK)
            }
        } else {
            // Small frames show a share of the old disc, so it stays a sprinkle.
            (crowd >= OLD_CROWD && (u - YOUNG_SHARE) / (1.0 - YOUNG_SHARE) < old_keep)
                .then_some(OLD_INK)
        }
    }
}

#[derive(Debug, Default)]
pub struct SpiralGalaxy {
    seeded: bool,
    t: f64,
    stars: Vec<Star>,
    /// Thread samples: pattern-frame points of every orbit (`f32`), `thread_n` per orbit,
    /// and each segment's ink (`None` in a lane); built in `resize`.
    thread: Vec<(f32, f32)>,
    thread_ink: Vec<Option<Ink>>,
    thread_n: usize,
    /// Every `thread_stride`-th orbit thread is drawn (small frames thin them).
    thread_stride: usize,
    /// Bulge stipple, dot positions and inks (fixed for a size).
    bulge: Vec<(i32, i32, Ink)>,
    /// Field stars, dot positions and inks (stable for a size).
    field: Vec<(i32, i32, Ink)>,
    /// Screen: centre and R in dots, and the view's rotation `(sin, cos)`.
    centre: (f64, f64),
    r_px: f64,
    pa: (f64, f64),
    dots: Dots,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl SpiralGalaxy {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.stars.clear();
        for j in 0..ORBITS {
            let f = j as f64 / (ORBITS - 1) as f64;
            let n = STARS_MIN + ((STARS_MAX - STARS_MIN) as f64 * f) as usize;
            for _ in 0..n {
                self.stars.push(Star {
                    orbit: j as u8,
                    alpha0: ctx.rng.random_range(0.0..TAU) as f32,
                    u: ctx.rng.random_range(0.0..1.0) as f32,
                    keep: ctx.rng.random_range(0.0..1.0) as f32,
                });
            }
        }
        self.t = ctx.rng.random_range(0.0..150.0);
    }

    /// The galaxy-plane → dot map at time `t`: turn by the pattern angle, tilt, then turn
    /// to the position angle and scale (`x' = m₀x + m₁y + cx`, `y' = m₂x + m₃y + cy`).
    fn matrix(&self, t: f64) -> [f64; 6] {
        let (ss, sc) = (OMEGA_P * t).sin_cos_det();
        let (ps, pc) = self.pa;
        let k = self.r_px;
        // Pattern rotation, then (u, v·cos i), then the position angle; y down on screen.
        let (a, b, c, d) = (sc, -ss, COS_INC * ss, COS_INC * sc);
        [
            k * (pc * a - ps * c),
            k * (pc * b - ps * d),
            -k * (ps * a + pc * c),
            -k * (ps * b + pc * d),
            self.centre.0,
            self.centre.1,
        ]
    }

    /// Dot centre → galaxy plane (pattern frame, units of R) at time `t`.
    #[cfg(test)]
    fn to_pattern(&self, px: i32, py: i32, t: f64) -> (f64, f64) {
        let m = self.matrix(t);
        let (x, y) = (f64::from(px) - m[4], f64::from(py) - m[5]);
        let det = m[0] * m[3] - m[1] * m[2];
        ((m[3] * x - m[1] * y) / det, (-m[2] * x + m[0] * y) / det)
    }

    /// Thread samples and inks of every orbit for the current size.
    fn build_threads(&mut self) {
        let circ = TAU * self.r_px;
        self.thread_n = ((circ / THREAD_GAP) as usize).clamp(THREAD_MIN, THREAD_MAX);
        // Spacing of neighbouring orbits at 0.5 R, in dots.
        let ln_step = (R_OUT / R_IN).ln() / (ORBITS - 1) as f64;
        let spacing = 0.5 * self.r_px * ln_step;
        self.thread_stride = ((THREAD_SPACING / spacing).ceil() as usize).clamp(1, 4);
        let n = self.thread_n;
        self.thread.clear();
        self.thread_ink.clear();
        for j in 0..ORBITS {
            for i in 0..n {
                let alpha = sample_angle(j, i, n);
                let (x, y) = orbit_point(j, alpha);
                self.thread.push((x as f32, y as f32));
                // The segment i → i + 1, inked at its middle.
                let mid = sample_angle(j, i, n) + 0.5 * TAU / n as f64;
                let (sp, cp) = (ARMS * (mid - orbit_turn(j))).sin_cos_det();
                let ink =
                    (!in_lane(cp, sp)).then(|| THREAD_INKS[usize::from(crowding(cp, sp) >= RIDGE)]);
                self.thread_ink.push(ink);
            }
        }
    }

    /// The bulge: a fixed-threshold stipple, densest at the centre (SPG-A12).
    fn build_bulge(&mut self) {
        self.bulge.clear();
        let d = self.dots.dot_size();
        let r = BULGE_R * self.r_px;
        let (cx, cy) = self.centre;
        let (ps, pc) = self.pa;
        let y0 = (cy - r).floor().max(0.0) as i32;
        let y1 = (cy + r).ceil().min(f64::from(d.height()) - 1.0) as i32;
        let x0 = (cx - r).floor().max(0.0) as i32;
        let x1 = (cx + r).ceil().min(f64::from(d.width()) - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (sx, sy) = (
                    (f64::from(x) - cx) / self.r_px,
                    -(f64::from(y) - cy) / self.r_px,
                );
                let (u, v) = (sx * pc + sy * ps, (-sx * ps + sy * pc) / COS_INC);
                let rr = u.hypot(v);
                if rr > BULGE_R {
                    continue;
                }
                if stipple_keep(BULGE_SALT, x, y, bulge_density(rr)) {
                    self.bulge
                        .push((x, y, BULGE_INKS[usize::from(rr > BULGE_CORE)]));
                }
            }
        }
    }

    /// Orbit threads, stars, the bulge and the field (SPEC-002 Look, M18).
    fn raster(&mut self) {
        self.dots.clear();
        let m = self.matrix(self.t);
        let screen = |(x, y): (f64, f64)| (m[0] * x + m[1] * y + m[4], m[2] * x + m[3] * y + m[5]);
        // Every drawn orbit is a sparse dotted thread (one dot per sample, ~2.6 dots
        // apart), so the rosettes read as distinct thin lines with black between them.
        let n = self.thread_n;
        for j in (0..ORBITS).step_by(self.thread_stride.max(1)) {
            let base = j * n;
            for i in 0..n {
                if let Some(ink) = self.thread_ink[base + i] {
                    let p = screen(f2(self.thread[base + i]));
                    self.dots.plot(p.0, p.1, ink);
                }
            }
        }
        for &(x, y, ink) in &self.bulge {
            self.dots.set(x, y, ink);
        }
        let t = self.t;
        let old_keep = (self.r_px / OLD_FULL_R).clamp(OLD_MIN_KEEP, 1.0);
        // Small frames draw a share of the stars, so the arms stay crisp, not crowded:
        // the share falls with the disc's area, as the dots available to the arms do.
        let keep = (self.r_px / KEEP_FULL_R).powi(2).clamp(KEEP_MIN, 1.0);
        let orbits: [Orbit; ORBITS] = std::array::from_fn(Orbit::of);
        for s in &self.stars {
            if f64::from(s.keep) >= keep {
                continue;
            }
            let (p, (cp, sp)) = s.state_on(&orbits[usize::from(s.orbit)], t);
            if let Some(ink) = s.ink(cp, sp, old_keep) {
                let (x, y) = screen(p);
                self.dots.plot(x, y, ink);
                if ink == KNOT_INK {
                    self.dots.plot(x + 1.0, y, ink);
                }
            }
        }
        for &(x, y, ink) in &self.field {
            if !self.dots.is_lit(x, y) {
                self.dots.set(x, y, ink);
            }
        }
    }
}

/// Bulge keep probability at radius `r` (units of R): falls monotonically, 0 outside.
pub(crate) fn bulge_density(r: f64) -> f64 {
    if !(0.0..=BULGE_R).contains(&r) {
        return 0.0;
    }
    BULGE_P0 * (-r / BULGE_SCALE).exp()
}

fn f2((x, y): (f32, f32)) -> (f64, f64) {
    (f64::from(x), f64::from(y))
}

impl Generator for SpiralGalaxy {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.dots.resize(size);
        let d = self.dots.dot_size();
        let (w, h) = (f64::from(d.width()), f64::from(d.height()));
        self.centre = (w / 2.0, h / 2.0);
        // The rosettes reach 1 + ε of R.
        self.r_px = 0.5 * w.min(h / COS_INC) * 0.84 / (1.0 + ECC);
        self.pa = PA.sin_cos_det();
        self.build_threads();
        self.build_bulge();
        // Field stars: hashed positions, stable for a size.
        self.field.clear();
        let count = (size.area() as f64 * FIELD_PER_CELL) as u64;
        for k in 0..count {
            let x = (hash01(cell_key(k as i32, 17)) * w) as i32;
            let y = (hash01(cell_key(k as i32, 91)) * h) as i32;
            let bright = hash01(cell_key(k as i32, 5)) > 0.8;
            self.field.push((x, y, FIELD_INKS[usize::from(bright)]));
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn built(size: Size, seed: u64) -> (SpiralGalaxy, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = SpiralGalaxy::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    /// Orbit phase `ψ` and guiding radius of the rosette through pattern-frame point
    /// `(x, y)`: the guiding radius solving `r = r_g(1 − ε cos ψ)` by fixed point.
    fn orbit_phase_at(x: f64, y: f64) -> (f64, f64) {
        let (r, a) = (x.hypot(y), y.atan2(x));
        let mut rg = r;
        for _ in 0..40 {
            let psi = ARMS * (a - COT_PITCH * rg.ln());
            rg = r / (1.0 - ECC * psi.cos_det());
        }
        (
            sciart_core::geom::wrap_angle(ARMS * (a - COT_PITCH * rg.ln())),
            rg,
        )
    }

    #[test]
    fn spg_a1_orbits_are_closed_rosettes_in_the_pattern_frame() {
        // A star returns to the same pattern-frame point after one relative period.
        for (k, &(j, a0)) in [(5usize, 0.3f64), (20, 2.0), (35, 5.0)].iter().enumerate() {
            let s = Star {
                orbit: j as u8,
                alpha0: a0 as f32,
                u: 0.9,
                keep: 0.0,
            };
            let period = TAU / (omega(orbit_radius(j)) - OMEGA_P);
            let (p0, _) = s.state(1.5 + k as f64);
            let (p1, _) = s.state(1.5 + k as f64 + period);
            assert!(
                (p0.0 - p1.0).abs() < 1e-5 && (p0.1 - p1.1).abs() < 1e-5,
                "orbit {j}"
            );
            // and lies on its rosette.
            let alpha = s.alpha(1.5);
            let (q, _) = s.state(1.5);
            let e = orbit_point(j, alpha);
            assert!((q.0 - e.0).abs() < 1e-6 && (q.1 - e.1).abs() < 1e-6);
        }
        // Each rosette turned by the same angle from the one inside it.
        let d0 = orbit_turn(1) - orbit_turn(0);
        for j in 1..ORBITS {
            assert!((orbit_turn(j) - orbit_turn(j - 1) - d0).abs() < 1e-9);
        }
        assert!(d0 > 0.0 && (30..=40).contains(&ORBITS));
        // At least six orbits per arm spacing in ln r, 2π/(6 cot p).
        let ln_step = (R_OUT / R_IN).ln() / (ORBITS - 1) as f64;
        assert!(TAU / (ARMS * COT_PITCH) >= 6.0 * ln_step);
        // Open arms: pitch 35–40°.
        let pitch = COT_PITCH.recip().atan().to_degrees();
        assert!((35.0..=40.0).contains(&pitch), "pitch {pitch:.1}°");
        // The crowding peak is 3–4.
        let psi = ridge_phase();
        let peak = crowding(psi.cos_det(), psi.sin_det());
        assert!((3.0..=4.0).contains(&peak), "peak crowding {peak:.2}");
    }

    #[test]
    fn spg_a14_rosettes_never_cross() {
        let crossing = ARMS * ECC * COT_PITCH;
        assert!(crossing <= 0.75, "6ε cot p = {crossing:.3}");
        for j in 1..ORBITS {
            for k in 0..2048 {
                let alpha = TAU * f64::from(k) / 2048.0;
                assert!(
                    orbit_r(j, alpha) > orbit_r(j - 1, alpha),
                    "orbit {j} at α {alpha:.4}"
                );
            }
        }
    }

    /// Lit-dot share on the ridge and between the arms (pattern frame), 0.35–0.75 R.
    fn ridge_vs_interarm(g: &SpiralGalaxy) -> (f64, f64) {
        let d = g.dots.dot_size();
        let (mut ridge, mut ridge_n, mut inter, mut inter_n) = (0usize, 0usize, 0usize, 0usize);
        let psi_r = ridge_phase();
        for py in 0..i32::from(d.height()) {
            for px in 0..i32::from(d.width()) {
                let (x, y) = g.to_pattern(px, py, g.t);
                let (psi, rg) = orbit_phase_at(x, y);
                if !(0.35..0.75).contains(&rg) {
                    continue;
                }
                let off = sciart_core::geom::wrap_angle(psi - psi_r);
                let lit = usize::from(g.dots.is_lit(px, py));
                if off.abs() < 0.3 {
                    (ridge, ridge_n) = (ridge + lit, ridge_n + 1);
                } else if (off.abs() - PI).abs() < 0.6 {
                    (inter, inter_n) = (inter + lit, inter_n + 1);
                }
            }
        }
        (ridge as f64 / ridge_n as f64, inter as f64 / inter_n as f64)
    }

    #[test]
    fn spg_a2_a8_a9_stars_and_threads_crowd_on_the_arm_ridge() {
        for seed in [42, 3] {
            let (g, _) = built(Size::new(200, 50), seed);
            let (ridge, inter) = ridge_vs_interarm(&g);
            // SPG-A8: the arms stand out by at least 2.5×.
            assert!(
                ridge >= 2.5 * inter,
                "ridge {ridge:.3} vs inter-arm {inter:.3}"
            );
        }
        // The stars alone: their density on the ridge is ≥ 2× that between the arms.
        let (g, _) = built(Size::new(200, 50), 42);
        let (mut on, mut off) = (0usize, 0usize);
        let psi_r = ridge_phase();
        for k in 0..40 {
            let t = g.t + f64::from(k) * 3.7;
            for s in &g.stars {
                let j = usize::from(s.orbit);
                if !(11..=28).contains(&j) {
                    continue;
                }
                let (_, (cp, sp)) = s.state(t);
                let off_r = sciart_core::geom::wrap_angle(sp.atan2(cp) - psi_r).abs();
                // Equal phase windows; position density = count / area ∝ count · crowding.
                if off_r < 0.3 {
                    on += 1;
                } else if (off_r - PI).abs() < 0.3 {
                    off += 1;
                }
            }
        }
        // Stars are uniform in orbit angle, so per window counts are similar; the arm
        // ridge squeezes them into a strip `crowding` times narrower.
        let c_on = crowding(psi_r.cos_det(), psi_r.sin_det());
        let anti = psi_r + PI;
        let c_off = crowding(anti.cos_det(), anti.sin_det());
        let ratio = (on as f64 * c_on) / (off as f64 * c_off);
        assert!(ratio >= 2.0, "density ratio {ratio:.2} ({on} / {off})");
    }

    #[test]
    fn spg_a13_six_fold_mode_dominates_at_mid_radius() {
        // Drawn stars in the annulus 0.45–0.55 R (pattern frame), binned in angle over
        // many frames: the m = 6 Fourier mode is at least twice any of m = 1…5.
        for seed in [42, 3] {
            let (g, _) = built(Size::new(200, 50), seed);
            let mut modes = [(0.0f64, 0.0f64); 7];
            let mut n = 0;
            for k in 0..60 {
                let t = g.t + f64::from(k) * 2.3;
                for s in &g.stars {
                    let ((x, y), (cp, sp)) = s.state(t);
                    let r = x.hypot(y);
                    if !(0.45..0.55).contains(&r) || s.ink(cp, sp, 1.0).is_none() {
                        continue;
                    }
                    let a = y.atan2(x);
                    n += 1;
                    for (m, c) in modes.iter_mut().enumerate().skip(1) {
                        let (sm, cm) = (m as f64 * a).sin_cos_det();
                        (c.0, c.1) = (c.0 + cm, c.1 + sm);
                    }
                }
            }
            let amp: Vec<f64> = modes
                .iter()
                .map(|c| c.0.hypot(c.1) / f64::from(n))
                .collect();
            let worst = amp[1..6].iter().copied().fold(0.0, f64::max);
            assert!(amp[6] >= 2.0 * worst, "seed {seed}: modes {amp:.3?}");
        }
    }

    #[test]
    fn spg_a10_stars_overtake_the_pattern_inside_corotation() {
        for j in 0..ORBITS {
            let r = orbit_radius(j);
            assert!(r * (1.0 + ECC) < COROTATION);
            assert!(omega(r) > OMEGA_P, "orbit {j}");
        }
        assert!((omega(COROTATION) - OMEGA_P).abs() < 1e-9);
        // A star's inertial angle advances faster than the pattern's.
        let s = Star {
            orbit: 20,
            alpha0: 0.0,
            u: 0.9,
            keep: 0.0,
        };
        let dt = 2.0;
        let turned = s.alpha(dt) + OMEGA_P * dt - s.alpha(0.0);
        assert!(turned > OMEGA_P * dt);
    }

    #[test]
    fn spg_a6_a11_orbit_threads_are_fainter_than_stars() {
        let stars = [OLD_INK, YOUNG_INK, KNOT_INK];
        for t in THREAD_INKS {
            for s in stars {
                assert!(t.level < s.level);
            }
        }
        // In the frame, every lit cell is braille in one of the listed inks.
        let (g, _) = built(Size::new(200, 50), 42);
        let mut c = Canvas::new(Size::new(200, 50), GlyphMode::Unicode);
        g.render(&mut c);
        let all = [THREAD_INKS[0], THREAD_INKS[1], OLD_INK, YOUNG_INK, KNOT_INK];
        let mut young = 0;
        for cell in c.cells() {
            if cell.ch == ' ' {
                continue;
            }
            assert!(
                ('\u{2801}'..='\u{28FF}').contains(&cell.ch),
                "{:?}",
                cell.ch
            );
            assert!(
                all.contains(&cell.ink)
                    || BULGE_INKS.contains(&cell.ink)
                    || FIELD_INKS.contains(&cell.ink),
                "{:?}",
                cell.ink
            );
            young += usize::from(cell.ink == YOUNG_INK);
        }
        assert!(young > 20, "{young} young-star cells");
    }

    #[test]
    fn spg_a7_a12_bulge_stipple_thins_with_radius_and_holds_still() {
        assert!(bulge_density(0.0) > bulge_density(0.05));
        assert!(bulge_density(0.05) > bulge_density(0.15));
        assert_eq!(bulge_density(0.3), 0.0);
        let (mut g, mut ctx) = built(Size::new(200, 50), 42);
        let mut counts = [(0usize, 0usize); 3];
        let (cx, cy) = g.centre;
        let rmax = BULGE_R * g.r_px;
        for &(x, y, ink) in &g.bulge {
            assert!(BULGE_INKS.contains(&ink));
            let r = (f64::from(x) - cx).hypot(f64::from(y) - cy);
            counts[((3.0 * r / rmax) as usize).min(2)].0 += 1;
        }
        for (k, c) in counts.iter_mut().enumerate() {
            // Dots available in each third (ring areas ∝ 1, 3, 5).
            c.1 = 2 * k + 1;
        }
        let dens: Vec<f64> = counts.iter().map(|&(n, a)| n as f64 / a as f64).collect();
        assert!(dens[0] > dens[1] && dens[1] > dens[2], "{dens:?}");
        let before = g.bulge.clone();
        g.update(1.0, &mut ctx);
        g.prepare();
        assert_eq!(before, g.bulge, "fixed thresholds: the bulge holds still");
    }

    #[test]
    fn spg_a4_dust_lanes_are_gaps_on_the_inner_edge() {
        // The lane sits at larger ψ than the ridge: at a fixed angle, smaller radius.
        let psi_l = ridge_phase() + DUST_OFFSET;
        assert!(in_lane(psi_l.cos_det(), psi_l.sin_det()));
        let psi_r = ridge_phase();
        assert!(!in_lane(psi_r.cos_det(), psi_r.sin_det()));
        let (a, j) = (0.7, 20);
        let psi = |rg: f64| ARMS * (a - COT_PITCH * rg.ln());
        assert!(psi(orbit_radius(j) * 0.98) > psi(orbit_radius(j)));
        // Stars there are hidden, threads too.
        let s = Star {
            orbit: 20,
            alpha0: 0.0,
            u: 0.9,
            keep: 0.0,
        };
        assert_eq!(s.ink(psi_l.cos_det(), psi_l.sin_det(), 1.0), None);
        let (g, _) = built(Size::new(200, 50), 42);
        let n = g.thread_n;
        for jj in 0..ORBITS {
            for i in 0..n {
                let mid = sample_angle(jj, i, n) + 0.5 * TAU / n as f64;
                let p = ARMS * (mid - orbit_turn(jj));
                assert_eq!(
                    g.thread_ink[jj * n + i].is_none(),
                    in_lane(p.cos_det(), p.sin_det())
                );
            }
        }
    }

    #[test]
    fn spg_a5_knots_flare_only_on_the_ridge() {
        // A knot star is drawn as a knot only where the orbits crowd most, and is never
        // in the dust lane; elsewhere it is an ordinary young star or hidden.
        let s = Star {
            orbit: 20,
            alpha0: 0.0,
            u: 0.01,
            keep: 0.0,
        };
        for k in 0..720 {
            let psi = TAU * f64::from(k) / 720.0;
            let (cp, sp) = (psi.cos_det(), psi.sin_det());
            if s.ink(cp, sp, 1.0) == Some(KNOT_INK) {
                assert!(
                    crowding(cp, sp) >= KNOT_CROWD && !in_lane(cp, sp),
                    "ψ {psi:.3}"
                );
            }
        }
        let p = ridge_phase();
        assert_eq!(s.ink(p.cos_det(), p.sin_det(), 1.0), Some(KNOT_INK));
        // About a tenth of the young stars are knot stars.
        let (g, _) = built(Size::new(200, 50), 42);
        let young = g
            .stars
            .iter()
            .filter(|s| f64::from(s.u) < YOUNG_SHARE)
            .count();
        let knots = g
            .stars
            .iter()
            .filter(|s| f64::from(s.u) < KNOT_SHARE * YOUNG_SHARE)
            .count();
        assert!(
            knots * 20 > young && knots * 5 < young,
            "{knots} of {young}"
        );
    }

    #[test]
    fn spg_a3_the_pattern_turns_at_omega_p() {
        // The ridge point of an orbit moves with the pattern: its screen position after Δt
        // is the pattern-frame point under the matrix turned by Ω_p Δt.
        let (g, _) = built(Size::new(200, 50), 7);
        let (x, y) = orbit_point(20, 1.0);
        for dt in [0.0, 5.0, 40.0] {
            let m = g.matrix(g.t + dt);
            let (px, py) = (m[0] * x + m[1] * y + m[4], m[2] * x + m[3] * y + m[5]);
            let back = g.to_pattern(px.round() as i32, py.round() as i32, g.t + dt);
            assert!((back.0 - x).hypot(back.1 - y) < 2.0 / g.r_px);
        }
        let m0 = g.matrix(0.0);
        let m1 = g.matrix(150.0);
        for k in 0..6 {
            assert!((m0[k] - m1[k]).abs() < 1e-6, "one turn in 150 s");
        }
    }
}
