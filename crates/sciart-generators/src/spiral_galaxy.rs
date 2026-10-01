//! A grand-design spiral as a Kalnajs kinematic density wave (SPEC-002 `spiral_galaxy`).
//!
//! Every star moves on a closed oval about the centre, `r = r_g (1 − e sin ψ)` with
//! `ψ = m(α + ln r_g / tan p)`, `m = 4`, where `α` is its angle in the frame turning with
//! the pattern. The ovals are nested with an orientation that turns with `ln r_g`, so
//! they crowd into four trailing logarithmic arms while every star keeps flowing through
//! them. Positions are closed form in time (no integration); lengths are in units of the
//! disc radius `R`, angles in radians. Drawn as star streams (M16): every star a braille
//! dot with a short thread along its orbit.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::InverseCdf;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "spiral_galaxy",
    category: Category::DeepCosmos,
    description: "Stars streaming through the four density-wave arms of a grand-design spiral",
    title: "Spiral galaxy · density waves",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SpiralGalaxy::default())
}

/// Number of arms (the oval's azimuthal order).
pub(crate) const ARMS: f64 = 4.0;
/// Arm pitch angle (22°, open arms for `m = 4`) as `1 / tan p`, the winding rate of
/// `ln r`.
const COT_PITCH: f64 = 2.475_086_853_416_296;
/// Pattern speed: one turn in 120 s.
pub(crate) const OMEGA_P: f64 = TAU / 120.0;
/// Corotation radius, where `Ω(r) = Ω_p`.
#[cfg(test)]
const COROTATION: f64 = 0.6;
/// Rotation-curve softening radius.
const SOFT: f64 = 0.1;
/// Flat circular speed, set so that corotation sits at `COROTATION`.
const V_FLAT: f64 = OMEGA_P * 0.608_276_253_029_821_9; // √(0.6² + 0.1²)
/// Stellar disc: exponential scale length, inner and outer guiding radii.
const SCALE_LENGTH: f64 = 0.3;
const R_IN: f64 = 0.04;
const R_OUT: f64 = 1.02;
/// Oval eccentricities: the crowding `1/(1 − e·m·cot p)` stays below orbit crossing
/// (`e·m·cot p ≈ 0.6–0.8` for `m = 4`).
const ECC: (f64, f64) = (0.06, 0.08);
/// Bulge: share of the stars, Gaussian σ (R).
const BULGE_SHARE: f64 = 0.08;
const BULGE_SIGMA: f64 = 0.07;
/// Star cap and stars per cell (ADR-0012).
const MAX_STARS: usize = 12_000;
const STARS_PER_CELL: f64 = 0.4;
/// View: inclination (25°) and the position angle of the major axis on screen.
const COS_INC: f64 = 0.906_307_787_036_649_9;
const PA: f64 = 0.35;
/// Stream thread: each star's thread is the chord back along its circular motion over
/// `STREAM_S` seconds (≈ 3 dots at 200×50).
const STREAM_S: f64 = 0.6;
/// Dust lane: centre and half-width in arm phase (SPG-A4); a star whose dust weight is
/// above `DUST_GAP` is hidden, so the lanes are gaps.
const DUST_PHI: f64 = -0.4;
const DUST_HALF: f64 = 0.25;
const DUST_GAP: f64 = 0.4;
/// H II knots: pool, lifetime (s), spawn rate (per s), the arm phase they are born at
/// (SPG-A5) and their guiding-radius range.
pub(crate) const KNOTS: usize = 48;
const KNOT_LIFE: f64 = 3.0;
const KNOT_RATE: f64 = 13.0;
const KNOT_PHI: f64 = 0.25;
const KNOT_R: (f64, f64) = (0.35, 0.92);
/// An H II region shows as two `Accent` dots, in two flat steps over its life, and only
/// the larger ones (`size ≥ KNOT_SHOWN`, about a third).
const KNOT_INKS: [Ink; 2] = [Ink::new(Role::Accent, 150), Ink::new(Role::Accent, 235)];
const KNOT_SHOWN: f64 = 0.87;
/// Young blue stars: share of the disc and the arm-phase band (just downstream of the
/// crest, the leading edge) they shine in.
const YOUNG_SHARE: f64 = 0.4;
const YOUNG_BAND: (f64, f64) = (-0.1, 0.9);
/// Inks (SPEC-002 Look, M16), `(head, stream)`: old disc stars yellow `Sun`, the old
/// bulge `Hot` and `Sun`, young stars on the arms blue `Cold`. A brighter population
/// wins a shared braille cell.
pub(crate) const DISC_INKS: (Ink, Ink) = (Ink::new(Role::Sun, 150), Ink::new(Role::Sun, 70));
pub(crate) const BULGE_INKS: [(Ink, Ink); 2] = [
    (Ink::new(Role::Hot, 215), Ink::new(Role::Hot, 130)),
    (Ink::new(Role::Sun, 235), Ink::new(Role::Sun, 140)),
];
pub(crate) const YOUNG_INKS: (Ink, Ink) = (Ink::new(Role::Cold, 255), Ink::new(Role::Cold, 160));
/// Foreground stars: dim and bright, one dot each.
const FIELD_INKS: [Ink; 2] = [Ink::new(Role::Dim, 120), Ink::new(Role::Star, 170)];
/// Field stars of our own Galaxy in front, per cell.
const FIELD_PER_CELL: f64 = 1.0 / 90.0;

/// `Ω(r)`: flat rotation curve, softened inside `SOFT` (rad/s).
pub(crate) fn omega(r: f64) -> f64 {
    V_FLAT / (r * r + SOFT * SOFT).sqrt()
}

/// Which population a star belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Pop {
    #[default]
    Disc,
    /// Disc star young enough to shine blue while it crosses an arm.
    Young,
    /// Bulge star, `Hot` (0) or `Sun` (1).
    Bulge(u8),
}

/// One star: guiding radius, pattern-frame angle at `t = 0`, eccentricity, population,
/// and the per-star constants of its orbit (`Ω − Ω_p`, `Ω`, `m·ln r_g·cot p`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Star {
    r: f32,
    alpha0: f32,
    ecc: f32,
    pop: Pop,
    slip: f32,
    omega: f32,
    wind: f32,
}

impl Star {
    fn new(r: f64, alpha0: f64, ecc: f64, pop: Pop) -> Self {
        Self {
            r: r as f32,
            alpha0: alpha0 as f32,
            ecc: ecc as f32,
            pop,
            slip: (omega(r) - OMEGA_P) as f32,
            omega: omega(r) as f32,
            wind: (ARMS * r.ln() * COT_PITCH) as f32,
        }
    }

    /// Galaxy-plane position at `t` and the arm phase there, from the star's own orbit
    /// phase `ψ`: `φ = m(α + ln r / tan p) ≈ ψ + m·cot p·ln(1 − e sin ψ)`, without a
    /// per-star `atan2` or `ln` (the log's series to second order: `e ≤ 0.08`).
    fn at(&self, t: f64) -> ((f64, f64), f64) {
        let (r, e) = (f64::from(self.r), f64::from(self.ecc));
        let alpha = f64::from(self.alpha0) + f64::from(self.slip) * t;
        let psi = ARMS * alpha + f64::from(self.wind);
        let u = e * psi.sin_det();
        let rad = r * (1.0 - u);
        let (s, c) = (alpha + OMEGA_P * t).sin_cos_det();
        let phi = psi - ARMS * COT_PITCH * (u + 0.5 * u * u);
        ((rad * c, rad * s), sciart_core::geom::wrap_angle(phi))
    }
}

/// Position of a star at time `t` in the galaxy plane (units of R, inertial frame). The
/// oval is closed in the pattern frame (SPG-A1). The reference for [`Star::at`].
#[cfg(test)]
pub(crate) fn star_pos(r: f64, alpha0: f64, ecc: f64, t: f64) -> (f64, f64) {
    let alpha = alpha0 + (omega(r) - OMEGA_P) * t;
    let psi = ARMS * (alpha + r.ln() * COT_PITCH);
    let rad = r * (1.0 - ecc * psi.sin_det());
    let (s, c) = (alpha + OMEGA_P * t).sin_cos_det();
    (rad * c, rad * s)
}

/// Pattern-frame position (for the closure test): the inertial position turned back by
/// the pattern's angle.
#[cfg(test)]
pub(crate) fn pattern_pos(r: f64, alpha0: f64, ecc: f64, t: f64) -> (f64, f64) {
    let (x, y) = star_pos(r, alpha0, ecc, t);
    let (s, c) = (-OMEGA_P * t).sin_cos_det();
    (x * c - y * s, x * s + y * c)
}

/// Arm phase `φ = m(θ + ln r / tan p − Ω_p t)` wrapped onto `(−π, π]`: 0 on an arm crest,
/// negative on its inner (upstream) edge. The reference for [`Star::at`]'s phase.
#[cfg(test)]
pub(crate) fn arm_phase(x: f64, y: f64, t: f64) -> f64 {
    let r = x.hypot(y).max(1e-9);
    let phi = ARMS * (y.atan2(x) + r.ln() * COT_PITCH - OMEGA_P * t);
    sciart_core::geom::wrap_angle(phi)
}

/// Dust opacity at arm phase `φ`: a raised cosine on `[DUST_PHI ± DUST_HALF]` (SPG-A4).
pub(crate) fn dust(phi: f64) -> f64 {
    let d = (phi - DUST_PHI) / DUST_HALF;
    if d.abs() >= 1.0 {
        0.0
    } else {
        0.5 + 0.5 * (PI * d).cos_det()
    }
}

/// Radial weight of the dust: none in the bulge or beyond the bright disc.
fn dust_radial(r: f64) -> f64 {
    ease::smoothstep((r - 0.1) / 0.15) * (1.0 - ease::smoothstep((r - 0.8) / 0.25))
}

/// Whether a star at radius `r` and arm phase `φ` is hidden in a dust lane.
pub(crate) fn in_lane(r: f64, phi: f64) -> bool {
    dust(phi) * dust_radial(r) > DUST_GAP
}

/// Whether young star `n` at arm phase `φ` shines blue: inside the band, thinned towards
/// its upstream edge by a fixed per-star hash, so they scatter rather than line up.
pub(crate) fn young_shows(n: usize, phi: f64) -> bool {
    let band = (phi - YOUNG_BAND.0) / (YOUNG_BAND.1 - YOUNG_BAND.0);
    (0.0..=1.0).contains(&band) && hash01(n as u64) <= 1.0 - band * 0.6
}

/// An H II region: galaxy-plane guiding radius and angle at birth, and its age.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    r: f64,
    theta0: f64,
    age: f64,
    size: f64,
}

impl Knot {
    fn pos(&self) -> (f64, f64) {
        let (s, c) = (self.theta0 + omega(self.r) * self.age).sin_cos_det();
        (self.r * c, self.r * s)
    }

    /// Brightness envelope over its life: a quick flare, then a slow fade.
    fn envelope(&self) -> f64 {
        let x = self.age / KNOT_LIFE;
        ease::smoothstep(x / 0.15) * (1.0 - ease::smoothstep((x - 0.6) / 0.4))
    }
}

#[derive(Debug, Default)]
pub struct SpiralGalaxy {
    seeded: bool,
    t: f64,
    /// Stars in units of R (kept across resizes), and the count in use.
    all_stars: Vec<Star>,
    n_stars: usize,
    knots: ParticlePool<Knot>,
    knot_debt: f64,
    /// Field stars, dot positions and inks (stable for a size).
    field: Vec<(i32, i32, Ink)>,
    /// Screen: centre and R in dots, and the view's rotation.
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
        let cdf = InverseCdf::<256>::build(R_IN, R_OUT, |r| {
            r * (-r / SCALE_LENGTH).exp() * (1.0 - ease::smoothstep((r - 0.85) / 0.17))
        });
        // Interleave the populations so every prefix (the stars in use at a size) has
        // the same mix.
        self.all_stars = (0..MAX_STARS)
            .map(|_| {
                let alpha0 = ctx.rng.random_range(0.0..TAU);
                if ctx.rng.random_bool(BULGE_SHARE) {
                    // A Rayleigh radius (a 2-D Gaussian), on near-circular orbits.
                    let u: f64 = ctx.rng.random_range(1e-9..1.0);
                    let r = (BULGE_SIGMA * (-2.0 * u.ln()).sqrt()).clamp(0.005, 0.3);
                    let tint = u8::from(ctx.rng.random_bool(0.5));
                    Star::new(r, alpha0, 0.0, Pop::Bulge(tint))
                } else {
                    let r = cdf.sample(ctx.rng.random_range(0.0..1.0));
                    let ecc = ctx.rng.random_range(ECC.0..ECC.1);
                    let pop = if ctx.rng.random_bool(YOUNG_SHARE) {
                        Pop::Young
                    } else {
                        Pop::Disc
                    };
                    Star::new(r, alpha0, ecc, pop)
                }
            })
            .collect();
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.t = ctx.rng.random_range(0.0..120.0);
    }

    /// Dot centre → galaxy plane (units of R): the inverse of `raster`'s projection.
    #[cfg(test)]
    fn to_plane(&self, px: i32, py: i32) -> (f64, f64) {
        let sx = (f64::from(px) - self.centre.0) / self.r_px;
        let sy = -(f64::from(py) - self.centre.1) / self.r_px;
        let (s, c) = self.pa;
        let (u, v) = (sx * c + sy * s, -sx * s + sy * c);
        (u, v / COS_INC)
    }

    fn spawn_knots(&mut self, dt: f64, ctx: &mut Ctx) {
        self.knot_debt += KNOT_RATE * dt;
        while self.knot_debt >= 1.0 {
            self.knot_debt -= 1.0;
            let r = ctx.rng.random_range(KNOT_R.0..KNOT_R.1);
            let arm = f64::from(ctx.rng.random_range(0..4u8)) * TAU / ARMS;
            // θ where φ = KNOT_PHI at radius r now.
            let theta0 = KNOT_PHI / ARMS - r.ln() * COT_PITCH + OMEGA_P * self.t + arm;
            let size = ctx.rng.random_range(0.6..1.0);
            self.knots.spawn(Knot {
                r,
                theta0,
                age: 0.0,
                size,
            });
        }
    }

    /// The inks a star is drawn in now, `(head, stream)`, or `None` in a dust lane.
    fn inks(n: usize, s: &Star, r: f64, phi: f64) -> Option<(Ink, Ink)> {
        match s.pop {
            Pop::Bulge(tint) => Some(BULGE_INKS[usize::from(tint)]),
            _ if in_lane(r, phi) => None,
            // Massive young stars live only just downstream of the arm that made them.
            Pop::Young if young_shows(n, phi) => Some(YOUNG_INKS),
            Pop::Young => None,
            Pop::Disc => Some(DISC_INKS),
        }
    }

    /// Every star as a dot with a short stream thread, then knots and field stars
    /// (SPEC-002 Look, M16).
    fn raster(&mut self) {
        self.dots.clear();
        let t = self.t;
        let (sn, c) = self.pa;
        let (cx, cy, k) = (self.centre.0, self.centre.1, self.r_px);
        let screen = |x: f64, y: f64| {
            let (u, v) = (x, y * COS_INC);
            (cx + (u * c - v * sn) * k, cy - (u * sn + v * c) * k)
        };
        for (n, s) in self.all_stars[..self.n_stars].iter().enumerate() {
            let ((x, y), phi) = s.at(t);
            let r = f64::from(s.r);
            let Some((head, stream)) = Self::inks(n, s, r, phi) else {
                continue;
            };
            // The stream: back along the circular motion over STREAM_S (a chord).
            let back = f64::from(s.omega) * STREAM_S;
            let (bx, by) = (x + back * y, y - back * x);
            let (hx, hy) = screen(x, y);
            let (tx, ty) = screen(bx, by);
            self.dots.line(tx, ty, hx, hy, stream);
            self.dots.plot(hx, hy, head);
        }
        // H II regions: a pink point each, the larger ones only, fading with their life.
        for kn in self.knots.iter() {
            let env = kn.envelope();
            if kn.size < KNOT_SHOWN || env < 0.15 {
                continue;
            }
            let (x, y) = kn.pos();
            let (px, py) = screen(x, y);
            let ink = KNOT_INKS[usize::from(env >= 0.5)];
            self.dots.plot(px, py, ink);
            self.dots.plot(px + 1.0, py, ink);
        }
        for &(x, y, ink) in &self.field {
            if !self.dots.is_lit(x, y) {
                self.dots.set(x, y, ink);
            }
        }
    }
}

impl Generator for SpiralGalaxy {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.n_stars = ((size.area() as f64 * STARS_PER_CELL) as usize).clamp(1, MAX_STARS);
        self.dots.resize(size);
        let d = self.dots.dot_size();
        let (w, h) = (f64::from(d.width()), f64::from(d.height()));
        self.centre = (w / 2.0, h / 2.0);
        self.r_px = 0.5 * w.min(h / COS_INC) * 0.94;
        self.pa = PA.sin_cos_det();
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

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        for k in self.knots.iter_mut() {
            k.age += dt;
        }
        self.knots.retain(|k| k.age < KNOT_LIFE);
        self.spawn_knots(dt, ctx);
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

    fn built(size: Size, seed: u64) -> (SpiralGalaxy, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = SpiralGalaxy::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn spg_a1_orbits_close_after_one_pattern_frame_period() {
        for &(r, a0, e) in &[(0.2, 0.3, 0.05), (0.45, 2.0, 0.04), (0.9, 5.0, 0.056)] {
            let period = TAU / (omega(r) - OMEGA_P).abs();
            let p0 = pattern_pos(r, a0, e, 1.5);
            let p1 = pattern_pos(r, a0, e, 1.5 + period);
            assert!(
                (p0.0 - p1.0).abs() < 1e-9 && (p0.1 - p1.1).abs() < 1e-9,
                "r={r}"
            );
        }
        // The per-star fast path matches the closed form and the arm phase.
        let s = Star::new(0.45, 2.0, 0.05, Pop::Disc);
        for t in [0.0, 13.7, 88.0] {
            let ((x, y), phi) = s.at(t);
            let (x0, y0) = star_pos(0.45, f64::from(s.alpha0), f64::from(s.ecc), t);
            assert!((x - x0).abs() < 1e-6 && (y - y0).abs() < 1e-6);
            let exact = arm_phase(x, y, t);
            assert!(
                sciart_core::geom::wrap_angle(phi - exact).abs() < 0.02,
                "{phi} {exact}"
            );
        }
    }

    /// Disc stars binned by arm phase at time `t`, `bins` bins over `(−π, π]`, radii in
    /// `r_range`.
    fn phase_hist(g: &SpiralGalaxy, t: f64, bins: usize, r_range: (f64, f64)) -> Vec<f64> {
        let mut h = vec![0.0; bins];
        for s in &g.all_stars[..g.n_stars] {
            if matches!(s.pop, Pop::Bulge(_)) {
                continue;
            }
            let (x, y) = star_pos(f64::from(s.r), f64::from(s.alpha0), f64::from(s.ecc), t);
            let r = x.hypot(y);
            if r < r_range.0 || r > r_range.1 {
                continue;
            }
            let phi = arm_phase(x, y, t);
            let b = (((phi + PI) / TAU) * bins as f64) as usize % bins;
            h[b] += 1.0;
        }
        h
    }

    #[test]
    fn spg_a2_star_density_peaks_on_the_arm_with_contrast() {
        let (g, _) = built(Size::new(200, 50), 42);
        let h = phase_hist(&g, g.t, 21, (0.15, 0.95));
        let (imax, &max) = h
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        let centre = -PI + (imax as f64 + 0.5) * TAU / 21.0;
        assert!(centre.abs() <= 0.3 + PI / 21.0, "peak at φ = {centre}");
        let min = h.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(max / min >= 1.8, "contrast {}", max / min);
    }

    /// Angle of the density peak in the annulus around `r`, via the m = 4 Fourier phase.
    fn arm_angle(g: &SpiralGalaxy, t: f64, r0: f64) -> f64 {
        let (mut c, mut s) = (0.0, 0.0);
        for st in &g.all_stars[..g.n_stars] {
            let (x, y) = star_pos(f64::from(st.r), f64::from(st.alpha0), f64::from(st.ecc), t);
            if (x.hypot(y) - r0).abs() < 0.03 {
                let th = ARMS * y.atan2(x);
                c += th.cos_det();
                s += th.sin_det();
            }
        }
        s.atan2(c) / ARMS
    }

    #[test]
    fn spg_a3_pattern_turns_at_omega_p_while_stars_do_not() {
        let (g, _) = built(Size::new(200, 50), 7);
        let dt = 4.0;
        let a0 = arm_angle(&g, 0.0, 0.5);
        let a1 = arm_angle(&g, dt, 0.5);
        let rate = sciart_core::geom::wrap_angle(ARMS * (a1 - a0)) / ARMS / dt;
        assert!(
            (rate - OMEGA_P).abs() <= 0.05 * OMEGA_P,
            "rate {rate} vs {OMEGA_P}"
        );
        assert!(omega(0.3) > OMEGA_P && omega(0.9) < OMEGA_P);
        assert!((omega(COROTATION) - OMEGA_P).abs() < 1e-12);
    }

    /// Lit dots and dots in the disc annulus `0.3–0.7 R` whose arm phase satisfies `pick`.
    fn annulus_lit(g: &SpiralGalaxy, pick: impl Fn(f64) -> bool) -> (usize, usize) {
        let d = g.dots.dot_size();
        let (mut lit, mut n) = (0, 0);
        for py in 0..i32::from(d.height()) {
            for px in 0..i32::from(d.width()) {
                let (x, y) = g.to_plane(px, py);
                if !(0.3..0.7).contains(&x.hypot(y)) || !pick(arm_phase(x, y, g.t)) {
                    continue;
                }
                n += 1;
                lit += usize::from(g.dots.is_lit(px, py));
            }
        }
        (lit, n)
    }

    #[test]
    fn spg_a4_dust_lanes_sit_on_the_inner_edge_as_gaps() {
        // The dust window lies at negative arm phase, and at fixed θ smaller radii have
        // smaller φ: the inner side of the crest.
        let mut sum = 0.0;
        let mut n = 0.0;
        for i in 0..2000 {
            let phi = -PI + TAU * (f64::from(i) + 0.5) / 2000.0;
            let d = dust(phi);
            sum += d * phi;
            n += d;
        }
        assert!(sum / n < 0.0);
        let (x, y) = (0.5, 0.0);
        let (x2, y2) = (0.45, 0.0);
        assert!(arm_phase(x2, y2, 0.0) < arm_phase(x, y, 0.0));
        assert!(in_lane(0.5, DUST_PHI) && !in_lane(0.5, 0.0) && !in_lane(0.02, DUST_PHI));
        // In the frame, the lane's dots are sparser than the crest's at the same radii.
        let (g, _) = built(Size::new(200, 50), 3);
        let (lane, lane_n) = annulus_lit(&g, |phi| (phi - DUST_PHI).abs() < 0.08);
        let (crest, crest_n) = annulus_lit(&g, |phi| (phi - 0.2).abs() < 0.08);
        let (lane, crest) = (lane as f64 / lane_n as f64, crest as f64 / crest_n as f64);
        assert!(lane < 0.5 * crest, "lane {lane:.3} vs crest {crest:.3}");
    }

    #[test]
    fn spg_a6_star_streams_on_black_with_dark_gaps_between_the_arms() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size, 3);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut inks = vec![DISC_INKS.0, DISC_INKS.1, YOUNG_INKS.0, YOUNG_INKS.1];
        inks.extend(BULGE_INKS.iter().flat_map(|p| [p.0, p.1]));
        inks.extend(KNOT_INKS);
        inks.extend(FIELD_INKS);
        let mut young = 0;
        for y in 0..50 {
            for x in 0..200 {
                let cell = canvas.get(x, y).unwrap();
                if cell.ch == ' ' {
                    continue;
                }
                assert!(('⠀'..='⣿').contains(&cell.ch), "{x},{y}: {:?}", cell.ch);
                assert!(inks.contains(&cell.ink), "{x},{y}: {:?}", cell.ink);
                young += usize::from(cell.ink == YOUNG_INKS.0);
            }
        }
        assert!(young > 20, "{young} young-star cells");
        // Between the arms (|φ| near π) the dots are sparse; on the crests they crowd.
        let (between, between_n) = annulus_lit(&g, |phi| (phi.abs() - PI).abs() < 0.4);
        let (crest, crest_n) = annulus_lit(&g, |phi| (phi - 0.2).abs() < 0.4);
        assert!(between_n > 400, "{between_n} inter-arm dots");
        let (between, crest) = (
            between as f64 / between_n as f64,
            crest as f64 / crest_n as f64,
        );
        assert!(between < 0.3, "inter-arm lit share {between:.3}");
        assert!(
            crest >= 1.5 * between,
            "crest {crest:.3} vs between {between:.3}"
        );
    }

    #[test]
    fn spg_a7_young_stars_light_the_leading_edges_and_the_bulge_is_old() {
        let (g, _) = built(Size::new(200, 50), 5);
        let mut bulge_max: f64 = 0.0;
        for (n, s) in g.all_stars[..g.n_stars].iter().enumerate() {
            let ((x, y), phi) = s.at(g.t);
            let r = f64::from(s.r);
            match (s.pop, SpiralGalaxy::inks(n, s, r, phi)) {
                (Pop::Bulge(_), inks) => {
                    bulge_max = bulge_max.max(x.hypot(y));
                    let (head, _) = inks.unwrap();
                    assert!(matches!(head.role, Role::Hot | Role::Sun));
                }
                (_, Some(inks)) if inks == YOUNG_INKS => {
                    assert!((YOUNG_BAND.0..=YOUNG_BAND.1).contains(&phi), "φ = {phi}");
                }
                (_, Some(inks)) => assert_eq!(inks, DISC_INKS),
                (Pop::Young, None) => assert!(in_lane(r, phi) || !young_shows(n, phi)),
                (_, None) => assert!(in_lane(r, phi)),
            }
        }
        assert!(bulge_max <= 0.3 + 1e-9, "bulge star at {bulge_max}");
    }

    #[test]
    fn spg_a5_knots_are_born_downstream_and_the_pool_is_capped() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size, 11);
        for _ in 0..1200 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            assert!(g.knots.len() <= KNOTS);
            for k in g.knots.iter() {
                let (x, y) = k.pos();
                let phi = arm_phase(x, y, g.t);
                assert!((0.0..=0.8).contains(&phi), "knot at φ = {phi}");
            }
        }
        assert!(g.knots.len() > KNOTS / 2);
    }
}
