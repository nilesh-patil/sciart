//! How a slingshot works (SPEC-002 `gravity_assist`).
//!
//! Model: patched conics in model units (`GM_sun = 1`, Jupiter's orbit radius 1, so its
//! speed and angular speed are 1). The probe climbs a heliocentric ellipse from an
//! Earth-like perihelion and meets Jupiter where it crosses Jupiter's orbit. In
//! Jupiter's frame it arrives with `v∞ = v_in − v_J`, swings round on a hyperbola with
//! periapsis `r_p` and leaves with the same `|v∞|`, turned by `δ`,
//! `sin(δ/2) = 1 / (1 + r_p v∞² / μ_J)`; back in the Sun's frame
//! `v_out = v_J + R(δ)(v_in − v_J)`. The pass is on Jupiter's trailing side, so `v∞` is
//! turned towards Jupiter's motion and the probe gains speed and energy. Every orbit is a
//! closed-form Kepler conic (elliptic or hyperbolic), timed by Kepler's equation.
//!
//! Look (GEN-R25): two panels split by a dotted `Dim` divider. Left, Jupiter's frame:
//! the hyperbola, inbound half `Cold` and outbound half `Hot`, round a small `Planet`
//! Jupiter atom, `|v - vJ| in = out` above it (plain ASCII, so every font shows it).
//! Right, the Sun's frame: the old orbit from launch to the encounter `Cold`, the new one
//! `Hot` (once the flyby has happened), Jupiter a small atom on a dotted orbit, the Sun a
//! small solid atom. The probe is a small solid `Accent` atom with a ≈ 0.7 s afterglow.
//! Approach and departure run on one Sun-frame clock; it stops during the flyby, while the
//! left panel shows the swing round Jupiter.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom;
use sciart_core::prelude::*;
use sciart_core::title;

pub static META: Meta = Meta {
    name: "gravity_assist",
    category: Category::Voyages,
    description: "A probe swings past Jupiter: the same speed in Jupiter's frame, much faster in the Sun's",
    title: "Gravity assist · borrowed speed",
    captions: &["approach", "flyby", "faster away"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(GravityAssist::new())
}

// ---- model --------------------------------------------------------------------------

/// Jupiter's `GM` over the Sun's.
pub(crate) const MU_J: f64 = 9.55e-4;
/// Perihelion of the pre-flyby orbit (Earth's orbit over Jupiter's).
const R_PERI: f64 = 0.192;
/// Pre-flyby aphelion range, per loop (beyond Jupiter, so the orbit crosses it).
const APO: (f64, f64) = (1.15, 1.3);
/// Jupiter-frame periapsis range in model units: ≈ 6–11 Jupiter radii, Voyager-like.
const RP: (f64, f64) = (5.5e-4, 1.0e-3);
/// Where on Jupiter's orbit the encounter happens (radians, per loop).
const ENC_ANGLE: (f64, f64) = (1.55, 2.05);
/// Jupiter's orbital speed in km/s: the scale of the readout.
pub(crate) const KMS: f64 = 13.07;

const T_APPROACH: f64 = 4.5;
const T_FLYBY: f64 = 4.5;
const T_AWAY: f64 = 8.0;
/// One clock for the Sun's frame (model time per scene second) in `approach` and
/// `faster away`: Jupiter turns at this many radians per second in both, the probe climbs
/// from perihelion to the encounter in ≈ 3 s and runs out to the panel's edge in ≈ 5–6 s.
/// The clock stops during `flyby` (the sphere of influence is a point in this frame).
pub(crate) const HELIO_RATE: f64 = 0.33;
const TIMELINE: [(&str, f64); 3] = [
    ("approach", T_APPROACH),
    ("flyby", T_FLYBY),
    ("faster away", T_AWAY),
];

/// The right panel shows the Sun's frame out to this radius (Jupiter's orbit = 1).
const R_VIEW: f64 = 2.6;
/// The left panel shows the hyperbola out to this many periapsis distances.
const R_LEFT: f64 = 6.0;
/// Jupiter's atom in the left panel: σ as a share of the panel's shorter side (dots).
/// A marker, not to scale (the model's `r_p` is 6–11 R_J).
const JUP_SIGMA_FRAC: f64 = 0.012;
/// The hyperbola's legs end this many dots inside the left panel (≈ 2 cells).
const LEG_INSET: (f64, f64) = (4.0, 8.0);

/// Smallest canvas that gets both panels (ASST-A5).
const MIN_W: u16 = 60;
const MIN_H: u16 = 14;
/// Share of the width the planet-frame panel takes.
const LEFT_SHARE: f64 = 0.4;
/// Blank cells kept right of the divider: the Sun panel's drawing (the escaping probe and
/// its conic) stops short of it rather than running into it.
const DIVIDER_GAP: i32 = 3;

const SUN_INK: Ink = Ink::new(Role::Sun, 255);
/// Atoms: σ (dots) and peak density (> 1 makes the core solid).
const SUN_SIGMA: f64 = 1.0;
const ATOM_PEAK: f64 = 2.0;
const JUP_SIGMA: f64 = 1.0;
const PROBE_SIGMA: f64 = 0.7;
const JUP_INK: Ink = Ink::new(Role::Planet, 255);
const PRE_INK: Ink = Ink::new(Role::Cold, 200);
const POST_INK: Ink = Ink::new(Role::Hot, 230);
const ORBIT_INK: Ink = Ink::new(Role::Dim, 120);
const DIVIDER_INK: Ink = Ink::new(Role::Dim, 110);
const PROBE_INK: Ink = Ink::new(Role::Accent, 255);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// Afterglow: this many earlier positions, this far apart in scene seconds.
const GLOW_N: usize = 12;
const GLOW_DT: f64 = 0.06;

/// Turn angle of a hyperbolic flyby: `sin(δ/2) = 1 / (1 + r_p v∞² / μ)` (ASST-A2).
pub(crate) fn turn_angle(rp: f64, vinf: f64, mu: f64) -> f64 {
    2.0 * (1.0 / (1.0 + rp * vinf * vinf / mu)).asin()
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn norm(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

fn unit(a: [f64; 2]) -> [f64; 2] {
    let n = norm(a).max(1e-300);
    [a[0] / n, a[1] / n]
}

fn rotate(a: [f64; 2], angle: f64) -> [f64; 2] {
    let (s, c) = angle.sin_cos_det();
    [c * a[0] - s * a[1], s * a[0] + c * a[1]]
}

/// A Kepler conic (ellipse or hyperbola) about a body of `GM = mu`, timed from an epoch
/// at which the body is at mean anomaly `m0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Conic {
    mu: f64,
    e: f64,
    /// Semi-major axis; negative for a hyperbola.
    a: f64,
    /// Argument of periapsis.
    w: f64,
    /// +1 prograde (counter-clockwise), −1 retrograde.
    dir: f64,
    m0: f64,
    /// Mean motion.
    n: f64,
}

impl Conic {
    /// The conic through position `r` with velocity `v` at epoch `t = 0`.
    pub(crate) fn from_state(mu: f64, r: [f64; 2], v: [f64; 2]) -> Self {
        let rn = norm(r);
        let v2 = dot(v, v);
        let rv = dot(r, v);
        let k = v2 - mu / rn;
        let ev = [(k * r[0] - rv * v[0]) / mu, (k * r[1] - rv * v[1]) / mu];
        let e = norm(ev);
        let w = ev[1].atan2(ev[0]);
        let a = 1.0 / (2.0 / rn - v2 / mu);
        let dir = if cross(r, v) >= 0.0 { 1.0 } else { -1.0 };
        let nu = dir * geom::wrap_angle(r[1].atan2(r[0]) - w);
        let (sn, cn) = nu.sin_cos_det();
        let (m0, n) = if e < 1.0 {
            let ea = ((1.0 - e * e).sqrt() * sn).atan2(e + cn);
            (ea - e * ea.sin_det(), (mu / (a * a * a)).sqrt())
        } else {
            let f = ((e * e - 1.0).sqrt() * sn / (1.0 + e * cn)).asinh();
            (e * f.sinh() - f, (mu / (-a * -a * -a)).sqrt())
        };
        Self {
            mu,
            e,
            a,
            w,
            dir,
            m0,
            n,
        }
    }

    pub(crate) fn hyperbolic(&self) -> bool {
        self.e >= 1.0
    }

    /// Eccentric (ellipse) or hyperbolic anomaly at mean anomaly `m`, by Newton.
    fn anomaly(&self, m: f64) -> f64 {
        let e = self.e;
        if self.hyperbolic() {
            let mut f = (m / e).asinh();
            for _ in 0..50 {
                let d = (e * f.sinh() - f - m) / (e * f.cosh() - 1.0);
                f -= d;
                if d.abs() < 1e-14 * (1.0 + f.abs()) {
                    break;
                }
            }
            f
        } else {
            let m = geom::wrap_angle(m);
            let mut ea = if e > 0.8 { PI.copysign(m) } else { m };
            for _ in 0..50 {
                let (s, c) = ea.sin_cos_det();
                let d = (ea - e * s - m) / (1.0 - e * c);
                ea -= d;
                if d.abs() < 1e-14 {
                    break;
                }
            }
            ea
        }
    }

    /// Position at anomaly `x` (eccentric or hyperbolic).
    pub(crate) fn at_anomaly(&self, x: f64) -> [f64; 2] {
        let (e, a) = (self.e, self.a);
        let (px, py) = if self.hyperbolic() {
            let b = -a;
            (b * (e - x.cosh()), b * (e * e - 1.0).sqrt() * x.sinh())
        } else {
            let (s, c) = x.sin_cos_det();
            (a * (c - e), a * (1.0 - e * e).sqrt() * s)
        };
        rotate([px, self.dir * py], self.w)
    }

    /// Position at time `t` after the epoch.
    pub(crate) fn at(&self, t: f64) -> [f64; 2] {
        self.at_anomaly(self.anomaly(self.m0 + self.n * t))
    }

    /// Hyperbolic anomaly at which the radius reaches `r` (hyperbola only).
    fn anomaly_at_radius(&self, r: f64) -> f64 {
        ((r / -self.a + 1.0) / self.e).max(1.0).acosh()
    }
}

/// One loop's flyby, everything in closed form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Flyby {
    /// The encounter point (Jupiter's position then) and its polar angle.
    pub(crate) enc: [f64; 2],
    pub(crate) enc_angle: f64,
    pub(crate) v_j: [f64; 2],
    pub(crate) v_in: [f64; 2],
    pub(crate) v_out: [f64; 2],
    pub(crate) rp: f64,
    pub(crate) delta: f64,
    /// Heliocentric orbits before and after (epoch: the encounter).
    pub(crate) pre: Conic,
    pub(crate) post: Conic,
    /// The Jupiter-frame hyperbola (epoch: periapsis), in model units.
    pub(crate) hyp: Conic,
    /// Time from perihelion to the encounter, and from the encounter to `R_VIEW`.
    t_back: f64,
    t_out: f64,
    /// Hyperbolic anomaly and mean anomaly at the left panel's edge.
    f_max: f64,
    m_max: f64,
}

impl Flyby {
    /// The flyby for a pre-flyby aphelion `apo`, Jupiter-frame periapsis `rp` and
    /// encounter angle `enc_angle` (ASST-A3: always a trailing-side, speed-gaining pass).
    pub(crate) fn new(apo: f64, rp: f64, enc_angle: f64) -> Self {
        let a = (R_PERI + apo) / 2.0;
        let e = (apo - R_PERI) / (apo + R_PERI);
        // At r = 1, outbound: vis-viva speed, angular momentum √(a(1 − e²)).
        let v = (2.0 - 1.0 / a).sqrt();
        let vt = (a * (1.0 - e * e)).sqrt();
        let vr = (v * v - vt * vt).max(0.0).sqrt();
        let (s, c) = enc_angle.sin_cos_det();
        let (rhat, that) = ([c, s], [-s, c]);
        let enc = rhat;
        let v_in = [vr * rhat[0] + vt * that[0], vr * rhat[1] + vt * that[1]];
        let v_j = that;
        let vinf_in = [v_in[0] - v_j[0], v_in[1] - v_j[1]];
        let vinf = norm(vinf_in);
        let delta = turn_angle(rp, vinf, MU_J);
        // Trailing side: turn v∞ towards Jupiter's velocity.
        let sign = if cross(vinf_in, v_j) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let vinf_out = rotate(vinf_in, sign * delta);
        let v_out = [v_j[0] + vinf_out[0], v_j[1] + vinf_out[1]];
        let pre = Conic::from_state(1.0, enc, v_in);
        let post = Conic::from_state(1.0, enc, v_out);
        // Jupiter's frame: periapsis opposite the bisector of the asymptotes.
        let (u_in, u_out) = (unit(vinf_in), unit(vinf_out));
        let p_hat = unit([u_in[0] - u_out[0], u_in[1] - u_out[1]]);
        let v_hat = unit([u_in[0] + u_out[0], u_in[1] + u_out[1]]);
        let vp = (vinf * vinf + 2.0 * MU_J / rp).sqrt();
        let hyp = Conic::from_state(
            MU_J,
            [rp * p_hat[0], rp * p_hat[1]],
            [vp * v_hat[0], vp * v_hat[1]],
        );
        let t_back = if pre.hyperbolic() {
            0.0
        } else {
            geom::wrap(pre.m0, TAU) / pre.n
        };
        // Bisection for the time the new orbit reaches the view's edge.
        let (mut lo, mut hi) = (0.0, 30.0);
        if norm(post.at(hi)) < R_VIEW {
            hi = PI / post.n; // bound ellipse that never leaves: half a period
        } else {
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if norm(post.at(mid)) < R_VIEW {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
        }
        let f_max = hyp.anomaly_at_radius(R_LEFT * rp);
        let m_max = hyp.e * f_max.sinh() - f_max;
        Self {
            enc,
            enc_angle,
            v_j,
            v_in,
            v_out,
            rp,
            delta,
            pre,
            post,
            hyp,
            t_back,
            t_out: hi,
            f_max,
            m_max,
        }
    }

    /// The Sun-frame clock (model time, 0 at the encounter) at loop time `lt`: one rate,
    /// [`HELIO_RATE`], before and after, stopped during the flyby (the planet's sphere of
    /// influence is a point in the Sun's frame).
    pub(crate) fn helio_clock(lt: f64) -> f64 {
        if lt < T_APPROACH {
            -HELIO_RATE * (T_APPROACH - lt)
        } else if lt < T_APPROACH + T_FLYBY {
            0.0
        } else {
            HELIO_RATE * (lt - T_APPROACH - T_FLYBY)
        }
    }

    /// The probe's Sun-frame time: the clock, but waiting at perihelion (launch) until the
    /// clock reaches it, and stopped a little past the panel's edge.
    pub(crate) fn helio_time(&self, lt: f64) -> f64 {
        Self::helio_clock(lt).clamp(-self.t_back, 1.5 * self.t_out)
    }

    /// Scene seconds the probe takes from the encounter to the panel's edge (tests).
    #[cfg(test)]
    pub(crate) fn away_secs(&self) -> f64 {
        self.t_out / HELIO_RATE
    }

    /// The probe in the Sun's frame at loop time `lt`.
    pub(crate) fn probe_helio(&self, lt: f64) -> [f64; 2] {
        let t = self.helio_time(lt);
        if t < 0.0 {
            self.pre.at(t)
        } else if t == 0.0 {
            self.enc
        } else {
            self.post.at(t)
        }
    }

    /// Jupiter in the Sun's frame at loop time `lt` (angular speed 1 in model time).
    pub(crate) fn jupiter(&self, lt: f64) -> [f64; 2] {
        let (s, c) = (self.enc_angle + Self::helio_clock(lt)).sin_cos_det();
        [c, s]
    }

    /// The probe in Jupiter's frame (units of `r_p`) at loop time `lt`: it runs the
    /// hyperbola leg to leg during the flyby, timed by Kepler's equation (periapsis at
    /// mid-flyby), and is not shown otherwise.
    pub(crate) fn probe_planet(&self, lt: f64) -> Option<[f64; 2]> {
        let from = T_APPROACH;
        if !(from..from + T_FLYBY).contains(&lt) {
            return None;
        }
        let u = (lt - from) / T_FLYBY;
        let m = -self.m_max + 2.0 * self.m_max * u;
        let p = self.hyp.at(m / self.hyp.n);
        Some([p[0] / self.rp, p[1] / self.rp])
    }

    /// Heliocentric speeds before and after, km/s (Voyager-like scale).
    pub(crate) fn speeds_kms(&self) -> (f64, f64) {
        (norm(self.v_in) * KMS, norm(self.v_out) * KMS)
    }
}

// ---- scene --------------------------------------------------------------------------

/// A panel: centre and scale in dots, and its clip box in dots.
#[derive(Debug, Clone, Copy, Default)]
struct Panel {
    cx: f64,
    cy: f64,
    scale: f64,
    clip: (i32, i32, i32, i32),
}

impl Panel {
    /// Shorter side of the clip box, dots.
    fn side(&self) -> f64 {
        let (x0, x1, y0, y1) = self.clip;
        f64::from((x1 - x0).min(y1 - y0) + 1)
    }

    fn to_dots(self, p: [f64; 2]) -> (f64, f64) {
        (self.cx + p[0] * self.scale, self.cy - p[1] * self.scale)
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        let (x0, x1, y0, y1) = self.clip;
        (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
    }
}

/// A dot segment from `a` to `b`, only the dots inside `panel`'s clip. `Dots::line` and
/// `Dots::plot` clip to the whole lattice only; the two panels share one lattice, so each
/// panel's drawing must be clipped to its own box (or the escaping conic would cross into
/// the other panel).
fn segment(dots: &mut Dots, panel: &Panel, a: (f64, f64), b: (f64, f64), ink: Ink) {
    if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
        return;
    }
    let n = (b.0 - a.0)
        .abs()
        .max((b.1 - a.1).abs())
        .ceil()
        .clamp(1.0, 2048.0) as usize;
    for k in 0..=n {
        let u = k as f64 / n as f64;
        let (x, y) = (
            (a.0 + (b.0 - a.0) * u).round() as i32,
            (a.1 + (b.1 - a.1) * u).round() as i32,
        );
        if panel.contains(x, y) {
            dots.set(x, y, ink);
        }
    }
}

fn plot(dots: &mut Dots, panel: &Panel, p: (f64, f64), ink: Ink) {
    if p.0.is_finite() && p.1.is_finite() {
        let (x, y) = (p.0.round() as i32, p.1.round() as i32);
        if panel.contains(x, y) {
            dots.set(x, y, ink);
        }
    }
}

/// The probe: a small solid `Accent` atom, only while the whole atom (3σ) is inside its
/// panel.
fn probe(dots: &mut Dots, panel: &Panel, p: (f64, f64)) {
    let (x, y) = (p.0.round() as i32, p.1.round() as i32);
    let reach = (3.0 * PROBE_SIGMA).ceil() as i32;
    let inside = [(-reach, 0), (reach, 0), (0, -reach), (0, reach)]
        .iter()
        .all(|&(dx, dy)| panel.contains(x + dx, y + dy));
    if inside {
        dots.stipple_puff(p.0, p.1, PROBE_SIGMA, ATOM_PEAK, 0x5C, PROBE_INK);
    }
}

#[derive(Debug)]
pub struct GravityAssist {
    size: Size,
    time: f64,
    timeline: Timeline,
    cycle: i64,
    flyby: Flyby,
    seeded: bool,
    sun_panel: Panel,
    /// The planet-frame panel, when there is room (ASST-A5).
    planet_panel: Option<Panel>,
    /// Divider column.
    divider: Option<i32>,
    readout_room: usize,
    dots: Dots,
    dirty: bool,
}

impl GravityAssist {
    fn new() -> Self {
        Self {
            size: Size::new(1, 1),
            time: 0.0,
            timeline: Timeline::new(&TIMELINE),
            cycle: 0,
            flyby: Flyby::new(1.22, 8e-4, 0.6),
            seeded: false,
            sun_panel: Panel::default(),
            planet_panel: None,
            divider: None,
            readout_room: 0,
            dots: Dots::default(),
            dirty: true,
        }
    }

    fn loop_time(&self) -> f64 {
        self.time.rem_euclid(self.timeline.total())
    }

    /// The readout: `v 10 km/s` until the flyby, then `v 10 → 20 km/s`; `short` is the
    /// current speed alone, for narrow canvases.
    fn readout(&self, unicode: bool, full: &mut StackText<32>, short: &mut StackText<16>) {
        let (vin, vout) = self.flyby.speeds_kms();
        let before = self.loop_time() < T_APPROACH;
        let arrow = if unicode { "→" } else { "->" };
        full.clear();
        short.clear();
        if before {
            let _ = write!(full, "v {vin:.0} km/s");
            let _ = write!(short, "{vin:.0} km/s");
        } else {
            let _ = write!(full, "v {vin:.0} {arrow} {vout:.0} km/s");
            let _ = write!(short, "{vout:.0} km/s");
        }
    }

    fn draw_flyby(rng: &mut impl Rng) -> Flyby {
        Flyby::new(
            rng.random_range(APO.0..APO.1),
            rng.random_range(RP.0..RP.1),
            rng.random_range(ENC_ANGLE.0..ENC_ANGLE.1),
        )
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        let lt = self.loop_time();
        let fb = self.flyby;
        let sp = self.sun_panel;
        let after = lt >= T_APPROACH;

        if let Some(col) = self.divider {
            let x = col * Dots::CELL_W;
            let rows = i32::from(self.size.height()).saturating_sub(1) * Dots::CELL_H;
            for y in (0..rows).step_by(3) {
                self.dots.set(x, y, DIVIDER_INK);
            }
        }

        // Sun's frame: Jupiter's orbit, the old orbit, the new one once it exists.
        let n = ((TAU * sp.scale / 4.0).round() as usize).max(8);
        for k in 0..n {
            let (s, c) = (k as f64 * TAU / n as f64).sin_cos_det();
            plot(&mut self.dots, &sp, sp.to_dots([c, s]), ORBIT_INK);
        }
        // The old orbit from perihelion (launch) to the encounter only.
        let segs = 64;
        let mut a = sp.to_dots(fb.pre.at(-fb.t_back));
        for k in 1..=segs {
            let t = -fb.t_back * (1.0 - f64::from(k) / f64::from(segs));
            let b = sp.to_dots(fb.pre.at(t));
            segment(&mut self.dots, &sp, a, b, PRE_INK);
            a = b;
        }
        if after {
            let mut a = sp.to_dots(fb.enc);
            for k in 1..=segs {
                let b = sp.to_dots(fb.post.at(1.5 * fb.t_out * f64::from(k) / f64::from(segs)));
                segment(&mut self.dots, &sp, a, b, POST_INK);
                a = b;
            }
        }
        let sun = sp.to_dots([0.0, 0.0]);
        self.dots
            .stipple_puff(sun.0, sun.1, SUN_SIGMA, ATOM_PEAK, 0x5A, SUN_INK);
        let jup = sp.to_dots(fb.jupiter(lt));
        self.dots
            .stipple_puff(jup.0, jup.1, JUP_SIGMA, ATOM_PEAK, 0x5B, JUP_INK);
        // The probe and its afterglow, in this loop only.
        for k in (1..=GLOW_N).rev() {
            let t = lt - k as f64 * GLOW_DT;
            if t < 0.0 {
                continue;
            }
            let level = 230.0 - 160.0 * (k as f64 - 1.0) / GLOW_N as f64;
            let ink = Ink::new(Role::Accent, level.round() as u8);
            let (a, b) = (fb.probe_helio(t), fb.probe_helio(t + GLOW_DT));
            segment(&mut self.dots, &sp, sp.to_dots(a), sp.to_dots(b), ink);
        }
        probe(&mut self.dots, &sp, sp.to_dots(fb.probe_helio(lt)));

        // Jupiter's frame: Jupiter centred, both legs ending inside the panel.
        if let Some(pp) = self.planet_panel {
            let segs = 96;
            let to = |x: f64| {
                let p = fb.hyp.at_anomaly(x);
                pp.to_dots([p[0] / fb.rp, p[1] / fb.rp])
            };
            let mut a = to(-fb.f_max);
            for k in 1..=segs {
                let x = -fb.f_max + 2.0 * fb.f_max * f64::from(k) / f64::from(segs);
                let b = to(x);
                let ink = if x <= 0.0 { PRE_INK } else { POST_INK };
                segment(&mut self.dots, &pp, a, b, ink);
                a = b;
            }
            for k in (1..=GLOW_N).rev() {
                let t = lt - k as f64 * GLOW_DT;
                if let (Some(a), Some(b)) = (fb.probe_planet(t), fb.probe_planet(t + GLOW_DT)) {
                    let level = 230.0 - 160.0 * (k as f64 - 1.0) / GLOW_N as f64;
                    let ink = Ink::new(Role::Accent, level.round() as u8);
                    segment(&mut self.dots, &pp, pp.to_dots(a), pp.to_dots(b), ink);
                }
            }
            if let Some(p) = fb.probe_planet(lt) {
                probe(&mut self.dots, &pp, pp.to_dots(p));
            }
            let sigma = (JUP_SIGMA_FRAC * pp.side()).max(1.0);
            self.dots
                .stipple_puff(pp.cx, pp.cy, sigma, ATOM_PEAK, 0x5D, JUP_INK);
        }
    }
}

impl Generator for GravityAssist {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        if !self.seeded {
            self.seeded = true;
            self.flyby = Self::draw_flyby(&mut ctx.rng);
            self.cycle = self.timeline.cycle(self.time);
        }
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let rows = (h - 1).max(1);
        let (cw, ch) = (Dots::CELL_W, Dots::CELL_H);
        let two = size.width() >= MIN_W && size.height() >= MIN_H;
        let split = if two {
            (f64::from(w) * LEFT_SHARE).round() as i32
        } else {
            0
        };
        // Sun's frame: the right part (or everything), rows above the title row.
        let x0 = if two {
            (split + 1 + DIVIDER_GAP) * cw
        } else {
            0
        };
        let x1 = w * cw - 1;
        let (y0, y1) = (0, rows * ch - 1);
        let (pw, ph) = (f64::from(x1 - x0 + 1), f64::from(y1 - y0 + 1));
        self.sun_panel = Panel {
            cx: f64::from(x0) + pw / 2.0 - 0.5,
            cy: f64::from(y0) + ph / 2.0 - 0.5,
            scale: (pw.min(ph) / (2.0 * R_VIEW)).max(0.1),
            clip: (x0, x1, y0, y1),
        };
        self.divider = two.then_some(split);
        self.planet_panel = two.then(|| {
            // Row 0 holds the label; keep a row of air above the title row.
            let (x0, x1) = (0, (split - 1) * cw - 1);
            let (y0, y1) = (ch, (rows - 1) * ch - 1);
            let (pw, ph) = (f64::from(x1 - x0 + 1), f64::from(y1 - y0 + 1));
            // Both legs end at R_LEFT·r_p, so this leaves them LEG_INSET inside.
            let reach = (pw / 2.0 - LEG_INSET.0).min(ph / 2.0 - LEG_INSET.1);
            Panel {
                cx: f64::from(x0) + pw / 2.0 - 0.5,
                cy: f64::from(y0) + ph / 2.0 - 0.5,
                scale: (reach / R_LEFT).max(0.1),
                clip: (x0, x1, y0, y1),
            }
        });
        self.readout_room = title::stable_readout_room(size, &META);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.time += dt;
        let cycle = self.timeline.cycle(self.time);
        if cycle != self.cycle {
            self.cycle = cycle;
            self.flyby = Self::draw_flyby(&mut ctx.rng);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
        let unicode = canvas.mode() == GlyphMode::Unicode;
        let h = i32::from(self.size.height());
        if let (Some(pp), Some(split)) = (self.planet_panel, self.divider) {
            let (uni, asc) = ("|v - vJ| in = out", "|v - vJ| in = out");
            let len = if unicode {
                uni.chars().count()
            } else {
                asc.len()
            };
            let len = i32::try_from(len).unwrap_or(i32::MAX);
            if len + 2 < split {
                let x = ((pp.cx / 2.0).round() as i32 - len / 2).max(1);
                draw::text_mode(canvas, x, 0, uni, asc, LABEL_INK);
            }
        }
        let mut full = StackText::<32>::new();
        let mut short = StackText::<16>::new();
        self.readout(unicode, &mut full, &mut short);
        let text = if full.as_str().chars().count() <= self.readout_room {
            Some(full.as_str())
        } else if short.len() <= self.readout_room {
            Some(short.as_str())
        } else {
            None
        };
        if let Some(t) = text {
            draw::label_right(canvas, h - 1, t, LABEL_INK);
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.timeline.at(self.loop_time()).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flybys() -> Vec<Flyby> {
        let mut out = Vec::new();
        for seed in 0..40u64 {
            let mut rng = SplitMix64::new(seed);
            for _ in 0..5 {
                out.push(GravityAssist::draw_flyby(&mut rng));
            }
        }
        out
    }

    #[test]
    fn asst_a1_planet_frame_speed_in_equals_out() {
        for fb in flybys() {
            let vin = norm([fb.v_in[0] - fb.v_j[0], fb.v_in[1] - fb.v_j[1]]);
            let vout = norm([fb.v_out[0] - fb.v_j[0], fb.v_out[1] - fb.v_j[1]]);
            assert!((vin - vout).abs() < 1e-9, "{vin} vs {vout}");
        }
    }

    #[test]
    fn asst_a2_turn_angle_formula_and_the_drawn_hyperbola_turns_by_it() {
        assert!((turn_angle(1.0, 1.0, 1.0) - 2.0 * 0.5f64.asin()).abs() < 1e-12);
        for fb in flybys().into_iter().step_by(7) {
            let vinf = norm([fb.v_in[0] - fb.v_j[0], fb.v_in[1] - fb.v_j[1]]);
            let want = 1.0 / (1.0 + fb.rp * vinf * vinf / MU_J);
            assert!(((fb.delta / 2.0).sin_det() - want).abs() < 1e-12);
            // The hyperbola's asymptotic directions, far out, differ by δ and match v∞.
            let far = 12.0;
            let (p1, p0) = (fb.hyp.at_anomaly(far), fb.hyp.at_anomaly(far - 1e-3));
            let (q1, q0) = (fb.hyp.at_anomaly(-far + 1e-3), fb.hyp.at_anomaly(-far));
            let d_out = unit([p1[0] - p0[0], p1[1] - p0[1]]);
            let d_in = unit([q1[0] - q0[0], q1[1] - q0[1]]);
            let turn = cross(d_in, d_out).atan2(dot(d_in, d_out)).abs();
            assert!((turn - fb.delta).abs() < 1e-6, "{turn} vs {}", fb.delta);
            let u_in = unit([fb.v_in[0] - fb.v_j[0], fb.v_in[1] - fb.v_j[1]]);
            let u_out = unit([fb.v_out[0] - fb.v_j[0], fb.v_out[1] - fb.v_j[1]]);
            assert!(dot(d_in, u_in) > 1.0 - 1e-9 && dot(d_out, u_out) > 1.0 - 1e-9);
            // Periapsis at r_p.
            assert!((norm(fb.hyp.at(0.0)) / fb.rp - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn asst_a3_trailing_pass_gains_speed_and_energy_every_loop_and_seed() {
        for fb in flybys() {
            let (vi, vo) = (norm(fb.v_in), norm(fb.v_out));
            assert!(vo > vi + 0.2, "{vi} -> {vo}");
            let energy = |v: f64| v * v / 2.0 - 1.0;
            assert!(energy(vo) > energy(vi));
            // Trailing side: the probe passes behind Jupiter's motion, i.e. its
            // periapsis lies on the side Jupiter is moving away from.
            let peri = fb.hyp.at(0.0);
            assert!(dot(peri, fb.v_j) < 0.0, "periapsis behind Jupiter");
            let (a, b) = fb.speeds_kms();
            assert!(
                (8.0..13.0).contains(&a) && (16.0..26.0).contains(&b),
                "{a} → {b}"
            );
        }
    }

    #[test]
    fn asst_a4_new_orbit_passes_through_the_encounter_with_v_out() {
        for fb in flybys().into_iter().step_by(5) {
            for (c, v) in [(fb.post, fb.v_out), (fb.pre, fb.v_in)] {
                let p = c.at(0.0);
                assert!(norm([p[0] - fb.enc[0], p[1] - fb.enc[1]]) < 1e-9);
                let e = 1e-6;
                let (a, b) = (c.at(-e), c.at(e));
                let vel = [(b[0] - a[0]) / (2.0 * e), (b[1] - a[1]) / (2.0 * e)];
                assert!(
                    norm([vel[0] - v[0], vel[1] - v[1]]) < 1e-5,
                    "{vel:?} vs {v:?}"
                );
            }
            // The pre-flyby orbit starts at the Earth-like perihelion.
            assert!((norm(fb.pre.at(-fb.t_back)) - R_PERI).abs() < 1e-6);
        }
    }

    #[test]
    fn asst_a5_below_60x14_only_the_sun_panel() {
        let mut ctx = Ctx::new(1, Size::new(59, 30));
        for (size, two) in [((59, 30), false), ((80, 13), false), ((60, 14), true)] {
            let size = Size::new(size.0, size.1);
            let mut g = GravityAssist::new();
            g.resize(size, &mut ctx);
            assert_eq!(g.planet_panel.is_some(), two, "{size:?}");
            assert_eq!(g.divider.is_some(), two);
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            // With two panels, Jupiter's atom sits at the left panel's centre.
            if let Some(pp) = g.planet_panel {
                let (x, y) = (pp.cx.round() as i32, pp.cy.round() as i32);
                assert!(g.dots.is_lit(x, y), "{size:?}");
            }
        }
    }

    #[test]
    fn asst_a6_captions_follow_the_acts_and_loop() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(3, size);
        let mut g = GravityAssist::new();
        g.resize(size, &mut ctx);
        let mut seen = vec![g.caption().unwrap()];
        let first = g.flyby;
        for _ in 0..(20.0 * 60.0) as usize + 30 {
            g.update(SIM_DT, &mut ctx);
            let c = g.caption().unwrap();
            if seen.last() != Some(&c) {
                seen.push(c);
            }
        }
        assert_eq!(seen, ["approach", "flyby", "faster away", "approach"]);
        assert_ne!(first, g.flyby, "the geometry varies per loop");
    }

    #[test]
    fn asst_probe_meets_jupiter_at_the_encounter() {
        let fb = Flyby::new(1.22, 8e-4, 0.6);
        let j = fb.jupiter(T_APPROACH - 1e-9);
        let p = fb.probe_helio(T_APPROACH - 1e-9);
        assert!(norm([j[0] - p[0], j[1] - p[1]]) < 1e-6);
        // In Jupiter's frame the periapsis falls at mid-flyby.
        let mid = T_APPROACH + T_FLYBY / 2.0;
        let p = fb.probe_planet(mid).unwrap();
        assert!((norm(p) - 1.0).abs() < 1e-6);
    }

    /// GEN-R25's ≤ 3 % half-block cells, for every seed (the flyby, so Jupiter's drawn
    /// size, varies per loop), as `thread_style.rs` measures it.
    #[test]
    fn asst_all_braille_no_half_blocks_for_any_seed() {
        let size = Size::new(200, 50);
        let mut worst = 0.0_f64;
        for seed in 0..12 {
            let mut stage = Stage::new(build(), seed, size, GlyphMode::Unicode);
            let mut steps = 0;
            for t in [5, 20, 31] {
                while steps < t * 60 {
                    stage.step(SIM_DT);
                    steps += 1;
                }
                let frame = stage.render();
                let body = &frame.cells()[..200 * 49];
                let (mut lit, mut block) = (0usize, 0usize);
                for c in body {
                    let is_block = c.bg != Ink::BLANK || matches!(c.ch, '▀' | '▄' | '█');
                    if is_block || c.ch != ' ' {
                        lit += 1;
                        block += usize::from(is_block);
                    }
                }
                worst = worst.max(block as f64 / lit.max(1) as f64);
            }
        }
        assert!(worst == 0.0, "half-block share {worst}");
    }

    #[test]
    fn asst_one_sun_frame_clock_in_approach_and_away_stopped_in_flyby() {
        let fb = Flyby::new(1.22, 8e-4, 0.6);
        let rate = |lt: f64| {
            let (a, b) = (fb.jupiter(lt - 1e-4), fb.jupiter(lt + 1e-4));
            let d = cross(a, b).atan2(dot(a, b));
            d / 2e-4
        };
        for lt in [0.5, 2.0, 4.0] {
            assert!((rate(lt) - HELIO_RATE).abs() < 1e-6, "approach {lt}");
        }
        for lt in [T_APPROACH + 1.0, T_APPROACH + 4.0] {
            assert!(rate(lt).abs() < 1e-9, "flyby {lt}");
        }
        for lt in [T_APPROACH + T_FLYBY + 0.5, T_APPROACH + T_FLYBY + 6.0] {
            assert!((rate(lt) - HELIO_RATE).abs() < 1e-6, "away {lt}");
        }
    }

    #[test]
    fn asst_approach_sweeps_perihelion_to_encounter_away_reaches_the_edge_in_about_5_s() {
        for fb in flybys() {
            // Launch: the act opens with the probe at perihelion, then it climbs.
            assert!((norm(fb.probe_helio(0.0)) - R_PERI).abs() < 1e-9);
            let climb = fb.t_back / HELIO_RATE;
            assert!((2.0..T_APPROACH).contains(&climb), "climb {climb} s");
            let mut r = 0.0;
            let mut lt = T_APPROACH - climb;
            while lt < T_APPROACH {
                let now = norm(fb.probe_helio(lt));
                assert!(now >= r - 1e-12, "r rises during the approach");
                r = now;
                lt += 0.05;
            }
            let away = fb.away_secs();
            assert!((4.0..7.0).contains(&away), "away {away} s");
        }
    }

    #[test]
    fn asst_left_probe_swings_round_during_the_flyby() {
        let fb = Flyby::new(1.22, 8e-4, 0.6);
        assert!(fb.probe_planet(T_APPROACH - 0.01).is_none());
        assert!(fb.probe_planet(T_APPROACH + T_FLYBY + 0.01).is_none());
        let mut prev = fb.probe_planet(T_APPROACH).unwrap();
        assert!(
            (norm(prev) - R_LEFT).abs() < 1e-6,
            "enters at the leg's end"
        );
        let mut lt = T_APPROACH + 0.25;
        while lt < T_APPROACH + T_FLYBY {
            let p = fb.probe_planet(lt).unwrap();
            assert!(
                norm([p[0] - prev[0], p[1] - prev[1]]) > 0.05,
                "moving at {lt}"
            );
            prev = p;
            lt += 0.25;
        }
    }

    #[test]
    fn asst_left_panel_centres_jupiter_with_symmetric_legs_inside() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(4, size);
        let mut g = GravityAssist::new();
        g.resize(size, &mut ctx);
        let pp = g.planet_panel.unwrap();
        let (x0, x1, y0, y1) = pp.clip;
        assert!((pp.cx - f64::from(x0 + x1) / 2.0).abs() <= 0.5);
        assert!((pp.cy - f64::from(y0 + y1) / 2.0).abs() <= 0.5);
        let fb = g.flyby;
        for f in [-fb.f_max, fb.f_max] {
            let q = fb.hyp.at_anomaly(f);
            let (x, y) = pp.to_dots([q[0] / fb.rp, q[1] / fb.rp]);
            assert!(
                x >= f64::from(x0) + 3.5 && x <= f64::from(x1) - 3.5,
                "leg x {x}"
            );
            assert!(
                y >= f64::from(y0) + 7.5 && y <= f64::from(y1) - 7.5,
                "leg y {y}"
            );
            let r = (x - pp.cx).hypot(y - pp.cy);
            assert!(
                (r - R_LEFT * pp.scale).abs() < 1e-6,
                "legs end at equal radii"
            );
        }
    }

    #[test]
    fn asst_readout_shows_the_old_speed_then_the_gain() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(4, size);
        let mut g = GravityAssist::new();
        g.resize(size, &mut ctx);
        let (mut full, mut short) = (StackText::<32>::new(), StackText::<16>::new());
        g.readout(true, &mut full, &mut short);
        let (vin, vout) = g.flyby.speeds_kms();
        assert_eq!(full.as_str(), format!("v {vin:.0} km/s"));
        for _ in 0..(T_APPROACH * 60.0) as usize + 2 {
            g.update(SIM_DT, &mut ctx);
        }
        g.readout(true, &mut full, &mut short);
        assert_eq!(full.as_str(), format!("v {vin:.0} → {vout:.0} km/s"));
        g.readout(false, &mut full, &mut short);
        assert!(full.as_str().is_ascii() && full.as_str().contains("->"));
    }

    #[test]
    fn asst_the_escaping_probe_stops_short_of_the_divider() {
        for seed in 0..6 {
            let size = Size::new(200, 50);
            let mut ctx = Ctx::new(seed, size);
            let mut g = GravityAssist::new();
            g.resize(size, &mut ctx);
            let split = g.divider.unwrap();
            let mut lt = 0.0;
            while lt < g.timeline.total() {
                g.update(0.1, &mut ctx);
                g.prepare();
                lt += 0.1;
                for x in split + 1..split + 1 + DIVIDER_GAP {
                    for y in 0..49 {
                        let m = g.dots.cell_mask(x, y).unwrap();
                        assert_eq!(m, 0, "seed {seed} t={lt:.1}: drawing at ({x},{y})");
                    }
                }
            }
        }
    }

    #[test]
    fn asst_tiny_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (2, 3), (59, 13), (60, 14), (500, 150)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(9, size);
            let mut g = GravityAssist::new();
            g.resize(size, &mut ctx);
            for dt in [0.0, 0.25, SIM_DT] {
                g.update(dt, &mut ctx);
                g.prepare();
            }
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut c);
        }
    }
}
