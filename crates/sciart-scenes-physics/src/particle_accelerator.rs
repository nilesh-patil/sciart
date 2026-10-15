//! A collider ring: two counter-rotating bunches ramp up, meet at the detector and burst
//! into curling bubble-chamber tracks (SPEC-002 `particle_accelerator`).
//!
//! Units: lengths in rows (screen column = `x · ASPECT`), time in s, mass 1, so a shower
//! particle's momentum is its speed and the field `B` is its cyclotron frequency.

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::ops::Range;

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::particles::plot;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};

pub static META: Meta = Meta {
    name: "particle_accelerator",
    category: Category::Physics,
    description: "Counter-rotating bunches colliding into curling particle showers",
    title: "Collider · magnets curl tracks",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ParticleAccelerator::new())
}

/// Full fill cycle, s.
const CYCLE: f64 = 20.0;
/// Ramp duration, s.
const RAMP: f64 = 10.0;
/// `dφ/dt` at injection and after the ramp, rad/s.
const OMEGA0: f64 = 0.5;
const OMEGA1: f64 = 6.0;
/// Ring radius as a fraction of `fit_radius`.
const RING_FRAC: f64 = 0.8;

/// Detector field = cyclotron frequency `qB/m`, rad/s (m = 1, |q| = 1).
const B_FIELD: f64 = 3.5;
/// Momentum loss rate `−(dp/dt)/p`, 1/s (ionisation in the bubble-chamber liquid).
const LOSS: f64 = 0.8;
/// Shower multiplicity per collision.
const MULT_MIN: u32 = 20;
const MULT_MAX: u32 = 40;
/// Track lifetime range, s (fade out within 2 s).
const LIFE_MIN: f64 = 1.2;
const LIFE_MAX: f64 = 2.0;
/// Initial momentum range as a fraction of `B·R` (so radii span 12–90 % of the ring).
const P_LO: f64 = 0.12;
const P_HI: f64 = 0.9;
/// Live tracks: at ω = 6 rad/s detector meets are ≥ 1.04 s apart and tracks live ≤ 2 s, so
/// at most two showers (≤ 80 tracks) overlap; the rest is headroom.
const TRACK_CAP: usize = 128;
const MAX_TRACK_SAMPLES: usize = 600;
/// Drawn length of each track, s of flight.
const TAIL: f64 = 0.7;

const BUNCH_TRAIL: usize = 10;
const FLASH: f64 = 0.35;
const GLINT: f64 = 0.15;

const RING: Glyph = Glyph::new('·', '.');
const BARREL: Glyph = Glyph::new('∙', '.');
const BUNCH: Glyph = Glyph::new('●', 'O');
const BUNCH_TAIL: Glyph = Glyph::new('•', 'o');
const BUNCH_TAIL_DIM: Glyph = Glyph::new('∙', '.');
const TRACK: Glyph = Glyph::new('∙', '.');
const TRACK_HEAD: Glyph = Glyph::new('•', 'o');
const IP: Glyph = Glyph::new('◎', '@');
const BURST: Glyph = Glyph::new('✶', '*');
const LABEL: &str = "DETECTOR";

/// `dφ/dt` at cycle time `tau` (s): linear 0.5 → 6 rad/s over the ramp, then flat.
pub(crate) fn ramp_omega(tau: f64) -> f64 {
    OMEGA0 + (OMEGA1 - OMEGA0) * (tau / RAMP).clamp(0.0, 1.0)
}

/// `φ` accumulated from the start of a cycle to cycle time `tau`: `∫₀^τ ω`.
pub(crate) fn ramp_phase(tau: f64) -> f64 {
    let tau = tau.max(0.0);
    let r = tau.min(RAMP);
    let ramp = OMEGA0 * r + 0.5 * (OMEGA1 - OMEGA0) * r * r / RAMP;
    ramp + OMEGA1 * (tau - r)
}

/// Bunch angles `(−π/2 + φ, −π/2 − φ)`, y down so `−π/2` is the top of the ring (ACC-A1).
pub(crate) fn bunch_angles(phi: f64) -> (f64, f64) {
    (-FRAC_PI_2 + phi, -FRAC_PI_2 - phi)
}

/// A shower track in a uniform field `B` with momentum loss `dp/dt = −k p` (ACC-A2).
///
/// With m = 1 the velocity obeys `v̇ = (−k + i q B) v` in screen coordinates (y down, so a
/// positive angular rate is clockwise on screen), i.e. `v(t) = v₀ e^{−λt}` with
/// `λ = k − i q B`, and the path is exact: `z(t) = z₀ + v₀ (1 − e^{−λt}) / λ` — a spiral
/// whose curvature radius `|v| / B = p / B` shrinks as the particle slows. Closed form means
/// the track is independent of the step size and rendering needs no stored history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Track {
    /// Birth point, rows.
    x0: f64,
    y0: f64,
    /// `c = v₀ / λ`: the spiral converges on `z₀ + c`.
    cx: f64,
    cy: f64,
    p0: f64,
    q: f64,
    pub(crate) age: f64,
    pub(crate) life: f64,
}

/// `v / λ` for velocity `(vx, vy)` and `λ = k − i q B`.
fn div_lambda(vx: f64, vy: f64, q: f64) -> (f64, f64) {
    // v · conj(λ) / |λ|², conj(λ) = k + i q B.
    let (lr, li) = (LOSS, q * B_FIELD);
    let n = LOSS * LOSS + B_FIELD * B_FIELD;
    ((vx * lr - vy * li) / n, (vx * li + vy * lr) / n)
}

impl Track {
    pub(crate) fn new(x: f64, y: f64, p0: f64, dir: f64, q: f64, life: f64) -> Self {
        let (s, c) = dir.sin_cos_det();
        let (cx, cy) = div_lambda(p0 * c, p0 * s, q);
        Self {
            x0: x,
            y0: y,
            cx,
            cy,
            p0,
            q,
            age: 0.0,
            life,
        }
    }

    /// `e^{−λ a}` as `(re, im)`.
    fn decay(&self, age: f64) -> (f64, f64) {
        let m = (-LOSS * age).exp();
        let (s, c) = (self.q * B_FIELD * age).sin_cos_det();
        (m * c, m * s)
    }

    /// Position at `age`, rows.
    pub(crate) fn pos(&self, age: f64) -> (f64, f64) {
        let (er, ei) = self.decay(age);
        // z₀ + c (1 − E) = z₀ + c − c·E
        (
            self.x0 + self.cx - (self.cx * er - self.cy * ei),
            self.y0 + self.cy - (self.cx * ei + self.cy * er),
        )
    }

    /// Momentum (= speed) at `age`.
    #[cfg(test)]
    pub(crate) fn p(&self, age: f64) -> f64 {
        self.p0 * (-LOSS * age).exp()
    }
}

#[derive(Debug)]
pub struct ParticleAccelerator {
    /// Cycle time in `[0, CYCLE)`.
    tau: f64,
    /// Accumulated bunch phase, unwrapped (so bunches never jump at a cycle restart).
    phi: f64,
    /// Ring centre (rows) and radius.
    cx: f64,
    cy: f64,
    r: f64,
    height: u16,
    trail: Trail<f64>,
    tracks: ParticlePool<Track>,
    flash: f64,
    glint: f64,
    detector_meets: u64,
    bottom_meets: u64,
    showers: u64,
    last_shower_tau: f64,
    last_shower_phi: f64,
    expired_last: usize,
}

impl ParticleAccelerator {
    fn new() -> Self {
        Self {
            tau: 0.0,
            phi: 0.0,
            cx: 0.0,
            cy: 0.0,
            r: 0.0,
            height: 0,
            trail: Trail::new(BUNCH_TRAIL),
            tracks: ParticlePool::with_capacity(TRACK_CAP),
            flash: 0.0,
            glint: 0.0,
            detector_meets: 0,
            bottom_meets: 0,
            showers: 0,
            last_shower_tau: 0.0,
            last_shower_phi: 0.0,
            expired_last: 0,
        }
    }

    /// The two bunches' positions (rows).
    fn bunch_positions(&self) -> ((f64, f64), (f64, f64)) {
        let (a, b) = bunch_angles(self.phi);
        (self.ring_point(a), self.ring_point(b))
    }

    fn ring_point(&self, theta: f64) -> (f64, f64) {
        let (s, c) = theta.sin_cos_det();
        (self.cx + self.r * c, self.cy + self.r * s)
    }

    /// Spawn a shower at the detector; each track is already `age` old (ACC-A3).
    fn collide(&mut self, age: f64, ctx: &mut Ctx) {
        let (ix, iy) = self.ring_point(-FRAC_PI_2);
        let pmax = (P_HI * B_FIELD * self.r).max(0.5);
        let pmin = (P_LO * B_FIELD * self.r).clamp(0.1, pmax);
        let n = ctx.rng.random_range(MULT_MIN..=MULT_MAX);
        for _ in 0..n {
            let q = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
            // Log-uniform momenta: a steeply falling spectrum, many tight curls, few stiff tracks.
            let p0 = pmin * (pmax / pmin).powf(ctx.rng.random::<f64>());
            let dir = ctx.rng.random_range(0.0..TAU);
            let life = ctx.rng.random_range(LIFE_MIN..=LIFE_MAX);
            let mut tr = Track::new(ix, iy, p0, dir, q, life);
            tr.age = age;
            self.tracks.spawn(tr);
        }
        self.flash = FLASH;
    }

    /// Glyph at a point in rows, if on screen.
    fn put(canvas: &mut Canvas, x: f64, y: f64, g: Glyph, ink: Ink) {
        plot(canvas, x * ASPECT, y, g, ink);
    }

    /// The bunches and their streaks: arcs back through the trail of recent phases.
    fn render_bunches(&self, canvas: &mut Canvas) {
        // ≤ 0.5 cells between arc samples keeps the streak connected.
        let step = 0.5 / (self.r * ASPECT).max(0.5);
        let n = self.trail.len().max(1) as f64;
        let mut prev: Option<f64> = None;
        for (i, &phi) in self.trail.iter().enumerate() {
            if let Some(p0) = prev {
                let frac = i as f64 / n;
                let level = (50.0 + 170.0 * frac) as u8;
                let g = if frac > 0.6 {
                    BUNCH_TAIL
                } else {
                    BUNCH_TAIL_DIM
                };
                let span = phi - p0;
                let k = ((span.abs() / step).ceil() as usize).min(2000);
                for j in 0..k {
                    let (a, b) = bunch_angles(p0 + span * j as f64 / k as f64);
                    for th in [a, b] {
                        let (x, y) = self.ring_point(th);
                        Self::put(canvas, x, y, g, Ink::new(Role::Accent, level));
                    }
                }
            }
            prev = Some(phi);
        }
        let ((x1, y1), (x2, y2)) = self.bunch_positions();
        Self::put(canvas, x1, y1, BUNCH, Ink::new(Role::Accent, 255));
        Self::put(canvas, x2, y2, BUNCH, Ink::new(Role::Accent, 255));
    }

    fn render_track(canvas: &mut Canvas, tr: &Track) {
        let fade = (1.0 - tr.age / tr.life).clamp(0.0, 1.0);
        if fade <= 0.0 || tr.p0 <= 0.0 {
            return;
        }
        let role = if tr.q > 0.0 { Role::Hot } else { Role::Cold };
        // The last TAIL seconds of the path: long enough to show the curl, short enough that a
        // 40-track shower stays legible. Uniform time samples ≤ 0.5 cells apart at the
        // fastest (oldest) speed; w = e^{−λ dt} is constant, so each sample is one complex
        // multiply of u = c·e^{−λa} (position = z₀ + c − u).
        let a0 = (tr.age - TAIL).max(0.0);
        let span = tr.age - a0;
        let dt = (0.5 / (tr.p0 * (-LOSS * a0).exp() * ASPECT)).max(1e-3);
        let n = ((span / dt).ceil() as usize).clamp(1, MAX_TRACK_SAMPLES);
        let (wr, wi) = tr.decay(span / n as f64);
        let (er, ei) = tr.decay(a0);
        let (mut ur, mut ui) = (tr.cx * er - tr.cy * ei, tr.cx * ei + tr.cy * er);
        let (zx, zy) = (tr.x0 + tr.cx, tr.y0 + tr.cy);
        let mut last = (i32::MIN, i32::MIN);
        for j in 0..=n {
            let (x, y) = (zx - ur, zy - ui);
            let cell = ((x * ASPECT).round() as i32, y.round() as i32);
            if cell != last {
                last = cell;
                // Older bubbles are dimmer, so the track reads as moving.
                let along = j as f64 / n as f64;
                let level = 30.0 + 210.0 * fade * along * along;
                Self::put(canvas, x, y, TRACK, Ink::new(role, level as u8));
            }
            (ur, ui) = (ur * wr - ui * wi, ur * wi + ui * wr);
        }
        let (x, y) = tr.pos(tr.age);
        Self::put(
            canvas,
            x,
            y,
            TRACK_HEAD,
            Ink::new(role, (80.0 + 175.0 * fade) as u8),
        );
    }
}

impl Generator for ParticleAccelerator {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        // Whole-cell centre and radius so the interaction points land exactly on cells.
        self.cx = f64::from(size.width().saturating_sub(1) / 2) / ASPECT;
        self.cy = f64::from(size.height().saturating_sub(1) / 2);
        let r = draw::fit_radius(size, RING_FRAC);
        self.r = if r >= 1.0 { r.round() } else { r };
        self.height = size.height();
        // Tracks live in the old geometry; a fresh detector is simpler than rescaling them.
        self.tracks.clear();
        self.tracks.set_capacity(TRACK_CAP);
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.flash = (self.flash - dt).max(0.0);
        self.glint = (self.glint - dt).max(0.0);

        let mut expired = 0;
        self.tracks.retain(|tr| {
            tr.age += dt;
            let alive = tr.age < tr.life;
            expired += usize::from(!alive);
            alive
        });
        self.expired_last = expired;

        // Advance the phase exactly (closed-form Φ), splitting at cycle restarts, and find
        // every meeting φ = kπ passed in this step (ACC-A1).
        let mut done = 0.0;
        while done < dt {
            let h = (dt - done).min(CYCLE - self.tau);
            let (phi0, tau0) = (self.phi, self.tau);
            let dphi = ramp_phase(tau0 + h) - ramp_phase(tau0);
            let phi1 = phi0 + dphi;
            let first = (phi0 / PI).floor() as i64 + 1;
            let last = (phi1 / PI).floor() as i64;
            for k in first..=last {
                let phi_k = k as f64 * PI;
                // ω is linear in τ, so the exact crossing time solves a quadratic; ω changes
                // by < 0.14 rad/s within h ≤ 0.25 s, so linear interpolation in φ is within
                // a few ms — ample for gating showers on the end of the ramp.
                let tau_k = tau0 + h * (phi_k - phi0) / dphi;
                if k.rem_euclid(2) == 0 {
                    self.detector_meets += 1;
                    if tau_k >= RAMP {
                        self.showers += 1;
                        self.last_shower_tau = tau_k;
                        self.last_shower_phi = phi_k;
                        let age = dt - done - (tau_k - tau0);
                        self.collide(age.max(0.0), ctx);
                    }
                } else {
                    // The second, quieter interaction point: a glint, no detector.
                    self.bottom_meets += 1;
                    if tau_k >= RAMP {
                        self.glint = GLINT;
                    }
                }
            }
            self.phi = phi1;
            self.tau += h;
            if self.tau >= CYCLE {
                self.tau = 0.0;
            }
            done += h;
        }
        self.trail.push(self.phi);
    }

    fn render(&self, canvas: &mut Canvas) {
        let (ix, iy) = self.ring_point(-FRAC_PI_2);
        let flash = self.flash / FLASH;

        // Beam pipe: the ring brightens briefly with each collision.
        draw::orbit(
            canvas,
            self.cx * ASPECT,
            self.cy,
            self.r,
            RING,
            Ink::new(Role::Structure, (95.0 + 90.0 * flash) as u8),
        );

        // Detector barrel: a thin layer around the interaction point.
        if self.r >= 4.0 {
            draw::orbit(
                canvas,
                ix * ASPECT,
                iy,
                1.5,
                BARREL,
                Ink::new(Role::Structure, (80.0 + 80.0 * flash) as u8),
            );
        }

        for tr in &self.tracks {
            Self::render_track(canvas, tr);
        }

        self.render_bunches(canvas);

        // The event: a burst at the IP and an expanding shock ring.
        if flash > 0.0 {
            draw::orbit(
                canvas,
                ix * ASPECT,
                iy,
                0.5 + 3.5 * (1.0 - flash),
                Glyph::ascii('*'),
                Ink::new(Role::Accent, (120.0 + 135.0 * flash) as u8),
            );
            Self::put(canvas, ix, iy, BURST, Ink::new(Role::Accent, 255));
        } else {
            Self::put(canvas, ix, iy, IP, Ink::new(Role::Text, 200));
        }
        if self.glint > 0.0 {
            let (bx, by) = self.ring_point(FRAC_PI_2);
            let lvl = 90.0 + 120.0 * self.glint / GLINT;
            Self::put(
                canvas,
                bx,
                by,
                Glyph::ascii('+'),
                Ink::new(Role::Accent, lvl as u8),
            );
        }

        // Label above the detector when there is room, otherwise below.
        let w = usize::from(canvas.size().width());
        if self.height >= 8 && w >= LABEL.len() + 2 {
            let row = iy.round() as i32;
            let y = if row >= 2 { row - 2 } else { row + 3 };
            let x = (ix * ASPECT).round() as i32 - LABEL.len() as i32 / 2;
            canvas.text(x, y, LABEL, Ink::new(Role::Text, 190));
        }

        // Beam-energy gauge along the bottom row: fills during the ramp.
        if let Some(cols) = gauge_cols(canvas.size(), &Title::of(self)) {
            let y = i32::from(self.height) - 1;
            let cells = f64::from(cols.end - cols.start);
            let fill = (ramp_omega(self.tau) - OMEGA0) / (OMEGA1 - OMEGA0);
            for x in cols.clone() {
                let (g, ink) = if f64::from(x - cols.start) + 0.5 <= fill * cells {
                    (Glyph::new('━', '='), Ink::new(Role::Accent, 170))
                } else {
                    (Glyph::new('─', '-'), Ink::new(Role::Dim, 90))
                };
                canvas.put(x, y, g, ink);
            }
        }
    }
}

/// Columns of the beam-energy gauge on the bottom row: a quarter of the width, centred,
/// but kept 4 blank cells clear of the title's text like a readout (GEN-R13, CORE-R76)
/// and shortened to end before the last column; `None` below 8 cells, so it is never a
/// stub.
fn gauge_cols(size: Size, title: &Title) -> Option<Range<i32>> {
    let w = i32::from(size.width());
    if size.height() < 12 || w < 24 {
        return None;
    }
    let start = title::span(size, title).map_or(1, |(_, cols)| cols.end + 3);
    let cells = (w / 4).clamp(8, 40);
    let x0 = ((w - cells) / 2).max(start);
    let end = (x0 + cells).min(w - 1);
    (end - x0 >= 8).then_some(x0..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(w: u16, h: u16) -> (ParticleAccelerator, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(7, size);
        let mut g = ParticleAccelerator::new();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn gen_r13_collider_gauge_clears_title_span() {
        let title = Title::new(META.title, None);
        // At 80×24 the centred gauge (columns 30..50) would sit under the title's text.
        let size = Size::new(80, 24);
        let (_, text) = title::span(size, &title).unwrap();
        let (g, _) = fresh(80, 24);
        let cols = gauge_cols(size, &title).unwrap();
        assert!(cols.start >= text.end + 3, "{cols:?} vs title {text:?}");
        assert_eq!(cols.len(), 20, "full length when it fits");
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let row: String = canvas.to_lines()[23]
            .chars()
            .skip(cols.start as usize)
            .collect();
        assert!(row.starts_with(&"─".repeat(20)), "{row:?}");
        // Wide canvases keep it centred; narrow ones shorten it or drop a stub.
        assert_eq!(gauge_cols(Size::new(200, 50), &title), Some(80..120));
        for w in 24..=120u16 {
            let size = Size::new(w, 24);
            if let Some(cols) = gauge_cols(size, &title) {
                let free = title::span(size, &title).map_or(1, |(_, c)| c.end + 3);
                assert!(cols.start >= free && cols.end < i32::from(w) && cols.len() >= 8);
            }
        }
    }

    /// Wrap an angle into (−π, π].
    fn wrap_pi(a: f64) -> f64 {
        let w = (a + PI).rem_euclid(2.0 * PI) - PI;
        if w <= -PI { w + 2.0 * PI } else { w }
    }

    #[test]
    fn acc_ramp_omega_is_linear_then_flat_and_phase_is_its_integral() {
        assert!((ramp_omega(0.0) - 0.5).abs() < 1e-12);
        assert!((ramp_omega(5.0) - 3.25).abs() < 1e-12);
        assert!((ramp_omega(10.0) - 6.0).abs() < 1e-12);
        assert!((ramp_omega(17.0) - 6.0).abs() < 1e-12);
        // Closed-form phase: 0.5·10 + 5.5·10/2 = 32.5 rad at the end of the ramp.
        assert!((ramp_phase(10.0) - 32.5).abs() < 1e-12);
        // dΦ/dτ = ω by central differences (exact on each linear piece up to roundoff).
        for tau in [0.3, 4.0, 9.5, 12.0, 19.0] {
            let h = 1e-5;
            let d = (ramp_phase(tau + h) - ramp_phase(tau - h)) / (2.0 * h);
            assert!((d - ramp_omega(tau)).abs() < 1e-6, "tau {tau}: {d}");
        }
    }

    #[test]
    fn acc_a1_bunches_are_mirror_images_and_meet_only_at_the_interaction_points() {
        // Pure geometry: the two bunch positions on a unit ring are mirror images about
        // the vertical axis, and their separation is 2|sin φ| — zero iff φ ≡ 0 (mod π).
        for i in 0..2000 {
            let phi = -7.0 + 0.0137 * f64::from(i);
            let (a, b) = bunch_angles(phi);
            let (x1, y1) = (a.cos_det(), a.sin_det());
            let (x2, y2) = (b.cos_det(), b.sin_det());
            assert!((x1 + x2).abs() < 1e-12, "mirror x at φ={phi}");
            assert!((y1 - y2).abs() < 1e-12, "same height at φ={phi}");
            let sep = (x1 - x2).hypot(y1 - y2);
            assert!((sep - 2.0 * phi.sin_det().abs()).abs() < 1e-12);
        }
        // At the meeting phases they sit at the detector (top) or the bottom point.
        for k in -4..=4 {
            let (a, b) = bunch_angles(f64::from(k) * PI);
            let expect = if k % 2 == 0 { -FRAC_PI_2 } else { FRAC_PI_2 };
            assert!(wrap_pi(a - expect).abs() < 1e-9 && wrap_pi(b - expect).abs() < 1e-9);
        }

        // The running generator: bunch positions stay mirrored for arbitrary dt, and every
        // meeting it registers is one of the two interaction points.
        let (mut g, mut ctx) = fresh(120, 40);
        let dts = [SIM_DT, 0.25, 0.1, 0.0, 0.037];
        for i in 0..2500 {
            g.update(dts[i % dts.len()], &mut ctx);
            let ((x1, y1), (x2, y2)) = g.bunch_positions();
            assert!(((x1 - g.cx) + (x2 - g.cx)).abs() < 1e-9, "step {i}");
            assert!((y1 - y2).abs() < 1e-9, "step {i}");
        }
        let crossings = (g.phi / PI).floor() as u64;
        assert_eq!(g.detector_meets + g.bottom_meets, crossings);
        assert!(g.detector_meets.abs_diff(g.bottom_meets) <= 1);
    }

    #[test]
    fn acc_a2_track_curvature_radius_is_p_over_b_and_turns_with_charge() {
        let mut rng = Ctx::new(11, Size::new(1, 1)).rng;
        for i in 0..40 {
            let q = if i % 2 == 0 { 1.0 } else { -1.0 };
            let p0 = rng.random_range(1.0..40.0);
            let dir = rng.random_range(0.0..TAU);
            let tr = Track::new(3.0, -2.0, p0, dir, q, 2.0);
            for age in [0.0, 0.4, 1.1, 1.9] {
                // Central differences on the closed-form path. h = 1e-4 s: truncation error
                // ~h²·|z''''|/12 and roundoff ~ε·|z|/h² are both ≲ 1e-6 relative, far inside
                // the 1e-4 tolerance.
                let h = 1e-4;
                let (x0, y0) = tr.pos(age - h);
                let (x1, y1) = tr.pos(age);
                let (x2, y2) = tr.pos(age + h);
                let (vx, vy) = ((x2 - x0) / (2.0 * h), (y2 - y0) / (2.0 * h));
                let (ax, ay) = (
                    (x2 - 2.0 * x1 + x0) / (h * h),
                    (y2 - 2.0 * y1 + y0) / (h * h),
                );
                let speed = vx.hypot(vy);
                // Signed curvature in screen coordinates (y down): κ > 0 is clockwise.
                let kappa = (vx * ay - vy * ax) / speed.powi(3);
                let r = 1.0 / kappa.abs();
                let expect = tr.p(age) / B_FIELD;
                assert!((r - expect).abs() < 1e-4 * expect, "r {r} vs p/B {expect}");
                assert_eq!(kappa.signum(), q, "turning sign follows the charge");
                // Momentum is the speed (m = 1 units).
                assert!((speed - tr.p(age)).abs() < 1e-4 * tr.p(age));
            }
            // Momentum loss: the radius shrinks monotonically (inward spiral).
            assert!(tr.p(1.0) < tr.p(0.5) && tr.p(0.5) < tr.p(0.0));
        }
    }

    #[test]
    fn acc_a3_showers_only_at_the_detector_after_the_ramp_and_pool_is_bounded() {
        let (mut g, mut ctx) = fresh(160, 48);
        let dts = [SIM_DT, SIM_DT, 0.25, SIM_DT, 0.1, 0.0];
        let mut t = 0.0;
        let mut per_cycle = [0u32; 3];
        let mut seen = 0;
        for i in 0..6000 {
            let dt = dts[i % dts.len()];
            let before = g.showers;
            let len_before = g.tracks.len();
            g.update(dt, &mut ctx);
            t += dt;
            if t > 60.0 {
                break;
            }
            assert!(g.tracks.len() <= g.tracks.capacity());
            if g.showers > before {
                assert_eq!(g.showers, before + 1, "at most one detector meet per step");
                assert!(
                    g.last_shower_tau >= RAMP,
                    "shower during ramp at τ={}",
                    g.last_shower_tau
                );
                let (a, b) = bunch_angles(g.last_shower_phi);
                assert!(wrap_pi(a + FRAC_PI_2).abs() < 1e-9);
                assert!(wrap_pi(b + FRAC_PI_2).abs() < 1e-9);
                let spawned = g.tracks.len() + g.expired_last - len_before;
                assert!((20..=40).contains(&spawned), "spawned {spawned}");
                per_cycle[((t - 1e-9) / CYCLE) as usize] += 1;
                seen += 1;
            }
        }
        assert!(seen > 0);
        // ω = 6 rad/s for 10 s: φ gains 60 rad ⇒ 9–10 detector meets per cycle.
        for n in per_cycle {
            assert!((9..=10).contains(&n), "{per_cycle:?}");
        }

        // A tiny budget is a hard cap.
        g.tracks.set_capacity(10);
        for _ in 0..1200 {
            g.update(SIM_DT, &mut ctx);
            assert!(g.tracks.len() <= 10);
        }
    }

    #[test]
    fn acc_tracks_fade_within_two_seconds() {
        let (mut g, mut ctx) = fresh(100, 30);
        for _ in 0..(40.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            for tr in &g.tracks {
                assert!(tr.age < tr.life && tr.life <= 2.0);
            }
        }
    }

    #[test]
    fn acc_tiny_and_odd_sizes_do_not_panic() {
        let (mut g, mut ctx) = fresh(1, 1);
        for (w, h) in [(1, 1), (2, 1), (0, 0), (3, 40), (200, 2), (80, 24)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            for i in 0..400 {
                g.update(if i % 5 == 0 { 0.25 } else { SIM_DT }, &mut ctx);
            }
            let mut c = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut c);
        }
    }
}
