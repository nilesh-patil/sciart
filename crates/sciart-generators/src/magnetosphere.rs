//! Earth's magnetosphere in the solar wind (SPEC-002 `magnetosphere`).
//!
//! Model, in Earth radii (R_E) on the noon–midnight meridian plane (Sun to the left,
//! `x` antisunward, `z` north): Earth's dipole (moment pointing south, as Earth's does),
//! plus a Chapman–Ferraro image dipole at `x = −2 r_mp` (the same moment, so `B_x = 0` on
//! the plane `x = −r_mp`: the dayside is squeezed), plus a tail current-sheet term
//! `B_x ∝ −tanh(z/L)` that stretches the night side into lobes (a drawn stretch, not a
//! divergence-free sheet). The standoff follows pressure balance, `r_mp = r₀ (p/p₀)^{−1/6}`;
//! the bow shock is a parabola with its nose at `1.3 r_mp`. Field lines are traced by RK4
//! from fixed northern footpoints, outward against `B̂` (the model is symmetric under
//! `z → −z`, so a closed line comes back at the mirror latitude and an open lobe line has
//! a mirror twin); a line is closed when it returns to Earth. Retracing is amortised: at
//! most [`RETRACE_PER_STEP`] lines per step, and only when the parameters moved (MAG-A6).
//!
//! Acts (`Timeline`, 30 s): quiet → gust (pressure ×4, the field compresses) →
//! reconnection (an X-line in the tail: the near tail dipolarises and the innermost lobe
//! lines close, MAG-A7) → substorm (at onset the plasmoid leaves down the tail and the
//! auroral ovals flare at both poles; the wind relaxes).
//!
//! Look (GEN-R25): 1-px braille field lines, closed `Cold` and open `Hot` in three flat
//! levels by footpoint latitude; the wind as sparse dim `Particle` dust with short streaks
//! (a drawn stand-in that flows round the shock and magnetopause, not solved); the bow
//! shock a dotted `Dim` thread; Earth a tiny half-block `Planet` disc, day side lit; the
//! aurora as bright `Particle` arcs and short curtains over each polar cap (the high
//! lines give way to them near Earth while they show).

use rand::Rng;
use sciart_core::ease::smoothstep;
use sciart_core::prelude::*;
use sciart_core::rng::hash01;

use sciart_core::raster::{self, half_block};

pub static META: Meta = Meta {
    name: "magnetosphere",
    category: Category::EarthSky,
    description: "The solar wind meets Earth's magnetic field: bow shock, magnetotail, reconnection and aurora",
    title: "Magnetosphere · wind meets field",
    captions: &["quiet", "gust", "reconnection", "substorm"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Magnetosphere::new())
}

// ---- model --------------------------------------------------------------------------

/// Quiet-time standoff distance, R_E.
const R0: f64 = 10.0;
/// Bow-shock nose over the magnetopause standoff.
pub(crate) const SHOCK_RATIO: f64 = 1.3;
/// Gust dynamic-pressure factor.
const GUST: f64 = 4.0;
/// Tail current-sheet field at quiet pressure (in units where the dipole is 1 at 1 R_E
/// on the equator, so ≈ 25 nT for 30 000 nT): enough to open the lines above ≈ 70°.
const TAIL_B: f64 = 8.0e-4;
/// Current-sheet half-thickness, R_E.
const SHEET_L: f64 = 1.5;
/// The sheet term grows over `x ∈ [TAIL_X0, TAIL_X0 + TAIL_RAMP]` (zero near Earth).
const TAIL_X0: f64 = 3.0;
const TAIL_RAMP: f64 = 8.0;
/// Near-tail X-line, R_E (the tail is shortened ~3× to fit).
const X_LINE: f64 = 16.0;
/// How much of the sheet field reconnection removes earthward of the X-line.
const DIPOLARISE: f64 = 0.8;
/// Seconds over which reconnection builds up (from 0.5 s into its act): the innermost
/// lobe lines close (MAG-A7) before substorm onset.
const RECON_RAMP: f64 = 4.0;

/// The view, R_E: Sun side, tail end and half-height. The trace box is the view.
const VIEW_X0: f64 = -18.0;
const VIEW_X1: f64 = 42.0;
const VIEW_Z: f64 = 15.5;

/// Footpoint latitudes (degrees) on the day and night sides.
const DAY_LATS: [f64; 5] = [45.0, 58.0, 66.0, 71.0, 74.0];
const NIGHT_LATS: [f64; 8] = [45.0, 58.0, 66.0, 70.0, 72.0, 73.5, 75.5, 78.0];
const LINES: usize = DAY_LATS.len() + NIGHT_LATS.len();
/// Most RK4 points per traced line.
const MAX_POINTS: usize = 700;
/// Lines retraced per step at most (MAG-A6): all fifteen refresh in 5 steps (< 0.1 s).
pub(crate) const RETRACE_PER_STEP: usize = 3;
/// Parameter change that triggers a retrace.
const RETRACE_EPS: f64 = 2e-3;

const T_QUIET: f64 = 8.0;
const T_GUST: f64 = 7.0;
const T_RECON: f64 = 7.0;
const T_SUBSTORM: f64 = 8.0;
const LOOP_S: f64 = T_QUIET + T_GUST + T_RECON + T_SUBSTORM;
const TIMELINE: [(&str, f64); 4] = [
    ("quiet", T_QUIET),
    ("gust", T_GUST),
    ("reconnection", T_RECON),
    ("substorm", T_SUBSTORM),
];

/// Solar-wind dust.
const WIND: usize = 130;
/// Upstream wind speed, R_E per (scene) second.
const WIND_V: f64 = 9.0;
/// Seconds for a sheath grain to ease onto its lane.
const LANE_TAU: f64 = 0.3;
/// Streak length, seconds of travel.
const STREAK_S: f64 = 0.05;
/// Plasmoid speed, R_E/s, and half-axes.
const PLASMOID_V: f64 = 5.0;
const PLASMOID_A: f64 = 4.0;
const PLASMOID_B: f64 = 1.4;
/// Aurora: latitude band (deg) and curtain radii (R_E, exaggerated to clear the disc).
/// Aurora: the cap arc spans ±AURORA_CAP° about each pole, curtains ±AURORA_RAYS°.
const AURORA_CAP: f64 = 35.0;
const AURORA_RAYS: f64 = 25.0;
/// Arc radius and curtain tips, R_E (exaggerated to clear the tiny disc).
const AURORA_R: (f64, f64) = (1.6, 2.3);
/// Below this width every other night-side line is drawn.
const THIN_BELOW: u16 = 100;
/// Random spread of a grain's sheath lane, so the flanks are not one dense lane.
const LANE_JITTER: f64 = 0.25;

const SHOCK_INK: Ink = Ink::new(Role::Dim, 160);
/// Dimmer than the aurora (same green), so the aurora is the brightest green.
const WIND_INK: Ink = Ink::new(Role::Particle, 150);
const PLASMOID_INK: Ink = Ink::new(Role::Accent, 220);
const EARTH_DAY: Ink = Ink::new(Role::Planet, 255);
const EARTH_NIGHT: Ink = Ink::new(Role::Planet, 110);
const AURORA_SALT: u64 = 0xA0_0A;
/// Oxygen green, at full level so it wins the cells it shares with field lines.
const AURORA_INK: Ink = Ink::new(Role::Particle, 255);

/// Standoff distance `r_mp = r₀ (p/p₀)^{−1/6}` for pressure `p` in units of quiet (MAG-A1).
pub(crate) fn standoff(p: f64) -> f64 {
    R0 * p.max(1e-6).powf(-1.0 / 6.0)
}

/// The slowly varying model parameters, with what `field` needs precomputed (the
/// standoff's `powf` and the sheet's `√p` are per step, not per RK4 evaluation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Params {
    /// Dynamic pressure over quiet.
    pub(crate) p: f64,
    /// Reconnection progress, 0..1.
    pub(crate) recon: f64,
    /// Standoff `r_mp(p)`, R_E.
    rmp: f64,
    /// Current-sheet field `TAIL_B · √p`.
    sheet: f64,
}

impl Params {
    pub(crate) fn new(p: f64, recon: f64) -> Self {
        Self {
            p,
            recon,
            rmp: standoff(p),
            sheet: TAIL_B * p.max(0.0).sqrt(),
        }
    }

    fn quiet() -> Self {
        Self::new(1.0, 0.0)
    }

    fn differs(self, o: Params) -> bool {
        (self.p - o.p).abs() > RETRACE_EPS || (self.recon - o.recon).abs() > RETRACE_EPS
    }
}

/// The field at `(x, z)` of a unit dipole at the origin pointing south (`−z`), as
/// Earth's does: field lines leave the southern hemisphere and enter the northern one.
pub(crate) fn dipole(x: f64, z: f64) -> [f64; 2] {
    let r2 = x * x + z * z;
    let r5 = r2 * r2 * r2.sqrt();
    [-3.0 * x * z / r5, -(3.0 * z * z - r2) / r5]
}

/// The model field at `(x, z)` R_E for `prm`: dipole + Chapman–Ferraro image + tail sheet.
pub(crate) fn field(x: f64, z: f64, prm: Params) -> [f64; 2] {
    let [ex, ez] = dipole(x, z);
    let [ix, iz] = dipole(x + 2.0 * prm.rmp, z);
    // Tail sheet: odd in z (the model keeps its north–south mirror symmetry), pointing
    // earthward in the northern lobe and away in the southern, as the dipole's own lines
    // do there. A drawn stretch, not a divergence-free current-sheet solution.
    let ramp = smoothstep((x - TAIL_X0) / TAIL_RAMP);
    let near = 1.0 - smoothstep((x - (X_LINE - 3.0)) / 4.0);
    let strength = prm.sheet * (1.0 - DIPOLARISE * prm.recon * near);
    let tail = -strength * ramp * (z / SHEET_L).tanh();
    [ex + ix + tail, ez + iz]
}

/// The traced line's outcome.
fn inside_view(x: f64, z: f64) -> bool {
    (VIEW_X0..=VIEW_X1).contains(&x) && z.abs() <= VIEW_Z
}

/// Trace the field line from the northern footpoint at `lat_deg` (day side `x < 0` when
/// `day`) against `B̂` (outward, as the field enters the north) with RK4 into `out` (cleared; at most [`MAX_POINTS`]). Returns
/// whether it closes on Earth; then its last point is on the surface.
pub(crate) fn trace(lat_deg: f64, day: bool, prm: Params, out: &mut Vec<[f64; 2]>) -> bool {
    out.clear();
    let (s, c) = lat_deg.to_radians().sin_cos_det();
    let mut y = [if day { -c } else { c }, s];
    out.push(y);
    let mut left_earth = false;
    while out.len() < MAX_POINTS {
        let r = y[0].hypot(y[1]);
        let h = (0.06 * r).clamp(0.04, 0.5);
        sciart_core::ode::rk4_step(&mut y, 0.0, h, |_, p| {
            let [bx, bz] = field(p[0], p[1], prm);
            let n = bx.hypot(bz);
            if n > 0.0 && n.is_finite() {
                [-bx / n, -bz / n]
            } else {
                [0.0, 0.0]
            }
        });
        if !(y[0].is_finite() && y[1].is_finite()) {
            return false;
        }
        let r = y[0].hypot(y[1]);
        if r > 1.1 {
            left_earth = true;
        }
        if left_earth && r <= 1.0 {
            out.push([y[0] / r, y[1] / r]);
            return true;
        }
        out.push(y);
        if !inside_view(y[0], y[1]) {
            return false;
        }
    }
    false
}

/// Ink of a field line by topology (MAG-A3) and footpoint latitude (three flat levels).
pub(crate) fn line_ink(closed: bool, lat_deg: f64) -> Ink {
    let level = if lat_deg < 60.0 {
        150
    } else if lat_deg < 70.0 {
        200
    } else {
        255
    };
    Ink::new(if closed { Role::Cold } else { Role::Hot }, level)
}

/// What the story asks for at loop time `t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Story {
    pub(crate) prm: Params,
    /// Plasmoid centre `x` (R_E) when one is on show.
    pub(crate) plasmoid: Option<f64>,
    /// Aurora brightness 0..1 (only during `substorm`).
    pub(crate) aurora: f64,
}

pub(crate) fn story(timeline: &Timeline, t: f64) -> Story {
    let beat = timeline.at(t);
    let l = beat.local;
    let (p, recon, aurora) = match beat.index {
        0 => (1.0, 0.0, 0.0),
        1 => (1.0 + (GUST - 1.0) * smoothstep(l / 3.0), 0.0, 0.0),
        2 => (GUST, smoothstep((l - 0.5) / RECON_RAMP), 0.0),
        _ => {
            // Onset: the plasmoid's release and the auroral flare together.
            let flare = if l < 0.3 {
                l / 0.3
            } else {
                (-(l - 0.3) / 2.5).exp()
            };
            (
                GUST - (GUST - 1.0) * smoothstep((l - 2.0) / 5.0),
                1.0 - smoothstep((l - 3.0) / 4.0),
                flare,
            )
        }
    };
    // At substorm onset the stretched lines' tailward halves leave the X-line as a
    // plasmoid that slides down the tail.
    let plasmoid = (beat.index == 3)
        .then_some(X_LINE + PLASMOID_A + PLASMOID_V * l)
        .filter(|&x| x - PLASMOID_A < VIEW_X1);
    Story {
        prm: Params::new(p, recon),
        plasmoid,
        aurora,
    }
}

/// Bow-shock nose distance for pressure `p`.
fn shock_nose(p: f64) -> f64 {
    SHOCK_RATIO * standoff(p)
}

/// The paraboloid `x = −s + z² / (2s)` with nose at `−s`: the bow shock (`s` = nose) and,
/// inside it, the sheath streamlines. `s` of the member through `(x, z)`.
fn parabola_s(x: f64, z: f64) -> f64 {
    (-x + (x * x + 2.0 * z * z).sqrt()) / 2.0
}

/// The wind's geometry at one pressure, computed once per step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Wind {
    /// Upstream speed, R_E/s.
    v0: f64,
    /// Bow-shock nose and magnetopause standoff, R_E.
    shock: f64,
    rmp: f64,
}

impl Wind {
    pub(crate) fn at(p: f64) -> Self {
        Self {
            v0: WIND_V * p.max(0.0).powf(0.25),
            shock: shock_nose(p),
            rmp: standoff(p),
        }
    }
}

/// The wind's velocity at `(x, z)`: straight downstream outside the shock; inside it,
/// along the sheath parabola through the point, slowed near the nose. A drawn stand-in.
pub(crate) fn wind_velocity(x: f64, z: f64, w: &Wind) -> [f64; 2] {
    let s = parabola_s(x, z);
    if s >= w.shock || !s.is_finite() || s <= 0.0 {
        return [w.v0, 0.0];
    }
    sheath_velocity(s, z, w.v0)
}

/// Sheath velocity at height `z` on the parabola with nose distance `s`: along its
/// tangent `(z/s, 1)·sign(z)`, away from the nose, slowed near it (post-shock).
pub(crate) fn sheath_velocity(s: f64, z: f64, v0: f64) -> [f64; 2] {
    let s = s.max(1e-6);
    let sign = if z >= 0.0 { 1.0 } else { -1.0 };
    let (tx, tz) = (z.abs() / s, sign);
    let n = tx.hypot(tz);
    let speed = v0 * (0.35 + 0.65 * (z.abs() / (1.5 * s)).min(1.0));
    [speed * tx / n, speed * tz / n]
}

// ---- scene --------------------------------------------------------------------------

#[derive(Debug)]
struct Line {
    lat: f64,
    day: bool,
    pts: Vec<[f64; 2]>,
    closed: bool,
    traced: Params,
}

#[derive(Debug, Clone, Copy)]
struct Grain {
    x: f64,
    z: f64,
    /// Upstream: < 0. In the sheath: the grain's lane, 0 (magnetopause) to 1 (shock),
    /// from how far off the Sun–Earth line it crossed the shock.
    lane: f64,
    /// In the sheath: the parabola (nose distance) the grain is on, easing to its lane.
    s: f64,
}

#[derive(Debug)]
pub struct Magnetosphere {
    size: Size,
    time: f64,
    timeline: Timeline,
    lines: Vec<Line>,
    /// Round-robin start for retracing.
    cursor: usize,
    /// Lines retraced by the last `update` (MAG-A6).
    retraced: usize,
    wind: Vec<Grain>,
    seeded: bool,
    /// Screen mapping: dot = (ox + x·scale, oy − z·scale).
    ox: f64,
    oy: f64,
    scale: f64,
    dots: Dots,
    /// Earth's half-block cells: (x, y, top, bottom), filled in `prepare`.
    earth: Vec<(i32, i32, Ink, Ink)>,
    dirty: bool,
}

impl Magnetosphere {
    fn new() -> Self {
        let mut lines = Vec::with_capacity(LINES);
        let lats = DAY_LATS
            .iter()
            .map(|&l| (l, true))
            .chain(NIGHT_LATS.iter().map(|&l| (l, false)));
        for (lat, day) in lats {
            let mut pts = Vec::with_capacity(MAX_POINTS);
            let closed = trace(lat, day, Params::quiet(), &mut pts);
            lines.push(Line {
                lat,
                day,
                pts,
                closed,
                traced: Params::quiet(),
            });
        }
        Self {
            size: Size::new(1, 1),
            time: 0.0,
            timeline: Timeline::new(&TIMELINE),
            lines,
            cursor: 0,
            retraced: 0,
            wind: Vec::with_capacity(WIND),
            seeded: false,
            ox: 0.0,
            oy: 0.0,
            scale: 1.0,
            dots: Dots::default(),
            earth: Vec::new(),
            dirty: true,
        }
    }

    fn loop_time(&self) -> f64 {
        self.time.rem_euclid(LOOP_S)
    }

    fn story(&self) -> Story {
        story(&self.timeline, self.loop_time())
    }

    fn to_dots(&self, x: f64, z: f64) -> (f64, f64) {
        (self.ox + x * self.scale, self.oy - z * self.scale)
    }

    /// Retrace up to [`RETRACE_PER_STEP`] lines whose parameters are stale.
    fn retrace(&mut self, prm: Params) -> usize {
        let mut done = 0;
        for k in 0..LINES {
            if done == RETRACE_PER_STEP {
                break;
            }
            let i = (self.cursor + k) % LINES;
            let line = &mut self.lines[i];
            if line.traced.differs(prm) {
                line.closed = trace(line.lat, line.day, prm, &mut line.pts);
                line.traced = prm;
                done += 1;
                self.cursor = (i + 1) % LINES;
            }
        }
        done
    }

    /// Draw line `i` (mirrored to the south when `mirror`), leaving out the points within
    /// `clear_r` R_E of Earth's centre (the aurora's room while it shows).
    fn polyline(&mut self, i: usize, mirror: bool, ink: Ink, clear_r: f64) {
        let zs = if mirror { -1.0 } else { 1.0 };
        let (ox, oy, sc) = (self.ox, self.oy, self.scale);
        let pts = &self.lines[i].pts;
        let mut prev: Option<(f64, f64)> = None;
        for p in pts {
            if p[0].hypot(p[1]) < clear_r {
                prev = None;
                continue;
            }
            let q = (ox + p[0] * sc, oy - zs * p[1] * sc);
            if let Some(a) = prev {
                self.dots.line(a.0, a.1, q.0, q.1, ink);
            }
            prev = Some(q);
        }
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        let st = self.story();
        let wind = Wind::at(st.prm.p);

        // Bow shock: one dot every ~3 along the parabola.
        let s = wind.shock;
        let step = 3.0 / self.scale.max(1e-9);
        let mut z = -VIEW_Z;
        while z <= VIEW_Z {
            let x = -s + z * z / (2.0 * s);
            if x <= VIEW_X1 {
                let (dx, dy) = self.to_dots(x, z);
                // Keep the dotted look even where the parabola runs steeply.
                let ds = (z / s).hypot(1.0);
                self.dots.plot(dx, dy, SHOCK_INK);
                z += step / ds;
            } else {
                z += step;
            }
        }

        // Wind dust with short streaks.
        for g in 0..self.wind.len() {
            let Grain { x, z, .. } = self.wind[g];
            let [vx, vz] = wind_velocity(x, z, &wind);
            let a = self.to_dots(x, z);
            let b = self.to_dots(x - vx * STREAK_S, z - vz * STREAK_S);
            self.dots.line(b.0, b.1, a.0, a.1, WIND_INK);
        }

        // Field lines: closed loops once; open lobe lines and their southern twins. On
        // narrow canvases every other night-side line, so the night fan stays lines.
        let clear_r = if st.aurora > 0.0 {
            AURORA_R.1 + 0.3
        } else {
            0.0
        };
        let thin = self.size.width() < THIN_BELOW;
        for i in 0..LINES {
            if thin && i >= DAY_LATS.len() && (i - DAY_LATS.len()) % 2 == 1 {
                continue;
            }
            let (closed, lat) = (self.lines[i].closed, self.lines[i].lat);
            let ink = line_ink(closed, lat);
            // Only the high-latitude lines run through the ovals; the low loops stay whole.
            let clear_r = if lat >= 90.0 - AURORA_CAP {
                clear_r
            } else {
                0.0
            };
            self.polyline(i, false, ink, clear_r);
            if !closed {
                self.polyline(i, true, ink, clear_r);
            }
        }

        // Plasmoid: a detached loop sliding down the tail.
        if let Some(px) = st.plasmoid {
            let n = 40;
            let mut prev = self.to_dots(px + PLASMOID_A, 0.0);
            for k in 1..=n {
                let (sn, cs) = (f64::from(k) * std::f64::consts::TAU / f64::from(n)).sin_cos_det();
                let q = self.to_dots(px + PLASMOID_A * cs, PLASMOID_B * sn);
                self.dots.line(prev.0, prev.1, q.0, q.1, PLASMOID_INK);
                prev = q;
            }
        }

        // Aurora: over each polar cap a 1-px arc ±AURORA_CAP° around the pole, clear of
        // the disc, and short radial curtains over its middle, as many as the flare is
        // bright (each has a fixed threshold, so a fading aurora loses curtains, it does
        // not shimmer); the night side brighter.
        if st.aurora > 0.0 {
            let samples: i32 = 15;
            let top = AURORA_R.0 + (AURORA_R.1 - AURORA_R.0) * (0.5 + 0.5 * st.aurora);
            for (hi, hemi) in [1.0, -1.0].into_iter().enumerate() {
                // `phi` from the pole, positive towards the night side.
                let at = |phi: f64, r: f64| {
                    let (sn, cs) = phi.to_radians().sin_cos_det();
                    (r * sn, hemi * r * cs)
                };
                let mut prev: Option<(f64, f64)> = None;
                for k in 0..samples {
                    let phi = AURORA_CAP * (2.0 * f64::from(k) / f64::from(samples - 1) - 1.0);
                    let (x0, z0) = at(phi, AURORA_R.0);
                    let a = self.to_dots(x0, z0);
                    if let Some(b) = prev {
                        self.dots.line(b.0, b.1, a.0, a.1, AURORA_INK);
                    }
                    prev = Some(a);
                    let gain = if phi >= 0.0 { 1.0 } else { 0.6 };
                    let key = hi as u64 * 64 + k as u64;
                    if phi.abs() <= AURORA_RAYS && hash01(AURORA_SALT ^ key) < st.aurora * gain {
                        let (x1, z1) = at(phi, top);
                        let b = self.to_dots(x1, z1);
                        self.dots.line(a.0, a.1, b.0, b.1, AURORA_INK);
                    }
                }
            }
        }

        // Earth: a tiny half-block disc, day side (towards the Sun, left) lit.
        self.earth.clear();
        let (ex, ey) = self.to_dots(0.0, 0.0);
        // Pixels are 2×2 dots.
        let (pcx, pcy) = ((ex + 0.5) / 2.0, (ey + 0.5) / 2.0);
        let pr = (self.scale / 2.0).max(0.6);
        let w = i32::from(self.size.width());
        let h = i32::from(self.size.height());
        let (x0, x1) = (
            (pcx - pr - 1.0).floor() as i32,
            (pcx + pr + 1.0).ceil() as i32,
        );
        let (y0, y1) = (
            ((pcy - pr - 1.0) / 2.0).floor() as i32,
            ((pcy + pr + 1.0) / 2.0).ceil() as i32,
        );
        for cy in y0.max(0)..=y1.min(h - 2) {
            for cx in x0.max(0)..=x1.min(w - 1) {
                let px = |py: i32| {
                    let cover = raster::disc_cover(cx, py, (pcx, pcy), pr);
                    if cover < 0.5 {
                        Ink::BLANK
                    } else if f64::from(cx) + 0.5 <= pcx {
                        EARTH_DAY
                    } else {
                        EARTH_NIGHT
                    }
                };
                let (top, bottom) = (px(2 * cy), px(2 * cy + 1));
                if (top != Ink::BLANK || bottom != Ink::BLANK)
                    && self.earth.len() < self.earth.capacity()
                {
                    self.earth.push((cx, cy, top, bottom));
                }
            }
        }
    }
}

impl Generator for Magnetosphere {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let rows = size.height().saturating_sub(1).max(1);
        let wd = f64::from(size.width()) * f64::from(Dots::CELL_W);
        let hd = f64::from(rows) * f64::from(Dots::CELL_H);
        let (vw, vh) = (VIEW_X1 - VIEW_X0, 2.0 * VIEW_Z);
        self.scale = (wd / vw).min(hd / vh).max(0.05);
        // Centre the view box.
        self.ox = (wd - vw * self.scale) / 2.0 - VIEW_X0 * self.scale - 0.5;
        self.oy = hd / 2.0 - 0.5;
        // Earth's cell list: at most the disc's bounding box.
        let pr = (self.scale / 2.0).max(0.6);
        let cells = ((2.0 * pr + 4.0) * (pr + 3.0)).ceil() as usize;
        self.earth = Vec::with_capacity(cells);
        if !self.seeded {
            self.seeded = true;
            for _ in 0..WIND {
                // Upstream of the shock only (rejection sampling, bounded).
                let (mut x, mut z) = (VIEW_X0, 0.0);
                for _ in 0..16 {
                    x = ctx.rng.random_range(VIEW_X0..VIEW_X1);
                    z = ctx.rng.random_range(-VIEW_Z..VIEW_Z);
                    if parabola_s(x, z) >= shock_nose(1.0) {
                        break;
                    }
                    x = VIEW_X0;
                }
                self.wind.push(Grain {
                    x,
                    z,
                    lane: -1.0,
                    s: 0.0,
                });
            }
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.time += dt;
        let st = self.story();
        self.retraced = self.retrace(st.prm);
        let wind = Wind::at(st.prm.p);
        let (shock, rmp) = (wind.shock, wind.rmp);
        let relax = 1.0 - (-dt / LANE_TAU).exp();
        for g in &mut self.wind {
            if g.lane < 0.0 {
                g.x += wind.v0 * dt;
                let s = parabola_s(g.x, g.z);
                if s < shock {
                    // Crossing the shock: near the nose the flow is pushed close to
                    // the magnetopause, on the flanks it stays nearer the shock; the
                    // jitter spreads the lanes.
                    let jitter = ctx.rng.random_range(-LANE_JITTER..LANE_JITTER);
                    g.lane = (g.z.abs() / (1.2 * shock) + jitter).clamp(0.05, 0.95);
                    g.s = s;
                }
            } else {
                // Along the sheath parabola through the grain, easing onto its lane.
                g.s += (rmp + (shock - rmp) * g.lane - g.s) * relax;
                let [_, vz] = sheath_velocity(g.s, g.z, wind.v0);
                g.z += vz * dt;
                g.x = -g.s + g.z * g.z / (2.0 * g.s);
            }
            if !inside_view(g.x, g.z) || (g.lane >= 0.0 && g.s < rmp) {
                *g = Grain {
                    x: VIEW_X0,
                    z: ctx.rng.random_range(-VIEW_Z..VIEW_Z),
                    lane: -1.0,
                    s: 0.0,
                };
            }
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
        for &(x, y, top, bottom) in &self.earth {
            if unicode {
                half_block(canvas, x, y, top, bottom);
            } else {
                let ink = raster::stronger(top, bottom);
                let ch = if ink == EARTH_DAY { '@' } else { 'o' };
                canvas.put(x, y, Glyph::ascii(ch), ink);
            }
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.timeline.at(self.loop_time()).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(size: Size) -> (Magnetosphere, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Magnetosphere::new();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn run(g: &mut Magnetosphere, ctx: &mut Ctx, secs: f64) {
        for _ in 0..(secs * 60.0).round() as usize {
            g.update(SIM_DT, ctx);
        }
        g.prepare();
    }

    #[test]
    fn mag_a1_standoff_scales_as_p_to_minus_one_sixth() {
        let ratio = standoff(GUST) / standoff(1.0);
        assert!((ratio / 0.794 - 1.0).abs() < 0.01, "{ratio}");
        // And the field really is tangential there: B_x = 0 on x = −r_mp.
        for p in [1.0, 2.0, GUST] {
            let rmp = standoff(p);
            for z in [-6.0, -2.0, 0.5, 3.0, 7.0] {
                let [bx, _] = field(-rmp, z, Params::new(p, 0.0));
                assert!(bx.abs() < 1e-12, "p={p} z={z}: {bx}");
            }
        }
    }

    #[test]
    fn mag_a2_near_earth_the_field_is_a_dipole_within_5_percent() {
        for prm in [Params::quiet(), Params::new(GUST, 1.0)] {
            for k in 0..24 {
                let a = f64::from(k) * std::f64::consts::TAU / 24.0;
                for r in [1.0, 1.5, 2.0] {
                    let (s, c) = a.sin_cos_det();
                    let (x, z) = (r * c, r * s);
                    let [bx, bz] = field(x, z, prm);
                    let [dx, dz] = dipole(x, z);
                    let err = (bx - dx).hypot(bz - dz) / dx.hypot(dz);
                    assert!(err < 0.05, "{prm:?} r={r} a={a}: {err}");
                }
            }
        }
    }

    #[test]
    fn mag_a3_closed_lines_cold_with_both_feet_on_earth_open_lines_hot() {
        let (mut g, mut ctx) = scene(Size::new(200, 50));
        let (mut closed, mut open) = (0, 0);
        for secs in [4.0, 6.0, 7.0, 9.0] {
            run(&mut g, &mut ctx, secs);
            for line in &g.lines {
                let first = line.pts[0];
                assert!((first[0].hypot(first[1]) - 1.0).abs() < 1e-9);
                if line.closed {
                    closed += 1;
                    let last = *line.pts.last().unwrap();
                    assert!((last[0].hypot(last[1]) - 1.0).abs() < 1e-9, "foot on Earth");
                    assert!(last[1] < 0.0, "a closed line ends in the south");
                    assert_eq!(line_ink(true, line.lat).role, Role::Cold);
                } else {
                    open += 1;
                    let last = *line.pts.last().unwrap();
                    assert!(last[0].hypot(last[1]) > 1.5, "open lines leave");
                    assert_eq!(line_ink(false, line.lat).role, Role::Hot);
                }
            }
        }
        assert!(closed > 0 && open > 0, "closed {closed}, open {open}");
        // Low-latitude lines are closed; the highest night-side ones are open (lobes).
        assert!(g.lines[0].closed && !g.lines[LINES - 1].closed);
        // The drawn frame carries both colours.
        let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let has = |r: Role| canvas.cells().iter().any(|c| c.ink.role == r);
        assert!(has(Role::Cold) && has(Role::Hot) && has(Role::Planet));
    }

    #[test]
    fn mag_a4_aurora_only_in_substorm_and_near_polar_footpoints() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = scene(size);
        let start = T_QUIET + T_GUST + T_RECON;
        let mut seen_aurora = false;
        let mut t = 0.0;
        while t < LOOP_S {
            run(&mut g, &mut ctx, 0.5);
            t += 0.5;
            let st = g.story();
            let in_substorm = g.caption() == Some("substorm");
            assert_eq!(
                st.aurora > 0.0,
                in_substorm && g.loop_time() > start,
                "t={t}"
            );
            // Aurora dots: Particle ink at the curtain's levels (wind dust is 170).
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            // Polar footpoints: the highest-latitude closed line's feet on both sides.
            let feet: Vec<(f64, f64)> = g
                .lines
                .iter()
                .filter(|l| l.closed)
                .flat_map(|l| [l.pts[0], *l.pts.last().unwrap()])
                .map(|p| g.to_dots(p[0], p[1]))
                .map(|(x, y)| (x / 2.0, y / 4.0))
                .collect();
            for (i, c) in canvas.cells().iter().enumerate() {
                if c.ink == AURORA_INK {
                    assert!(in_substorm, "aurora outside substorm at t={t}");
                    seen_aurora = true;
                    let (x, y) = ((i % 200) as f64 + 0.5, (i / 200) as f64 + 0.5);
                    let near = feet
                        .iter()
                        .map(|&(fx, fy)| (fx - x).abs().max((fy - y).abs()))
                        .fold(f64::MAX, f64::min);
                    assert!(
                        near <= 3.0,
                        "aurora cell ({x},{y}) {near} cells from a foot"
                    );
                }
            }
        }
        assert!(seen_aurora);
    }

    #[test]
    fn mag_a5_captions_follow_the_acts_and_loop() {
        let (mut g, mut ctx) = scene(Size::new(80, 24));
        let mut seen = vec![g.caption().unwrap()];
        for _ in 0..(LOOP_S * 60.0) as usize + 60 {
            g.update(SIM_DT, &mut ctx);
            let c = g.caption().unwrap();
            if seen.last() != Some(&c) {
                seen.push(c);
            }
        }
        assert_eq!(seen, ["quiet", "gust", "reconnection", "substorm", "quiet"]);
    }

    #[test]
    fn mag_a6_retracing_is_amortised_and_idle_when_quiet() {
        let (mut g, mut ctx) = scene(Size::new(120, 40));
        let mut total = 0;
        for step in 0..(LOOP_S * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            assert!(
                g.retraced <= RETRACE_PER_STEP,
                "step {step}: {}",
                g.retraced
            );
            total += g.retraced;
            // Mid-quiet nothing changes, so nothing is retraced.
            if (120..400).contains(&step) {
                assert_eq!(g.retraced, 0, "step {step}");
            }
        }
        assert!(total > 50, "the field does follow the story ({total})");
        // The gust compresses the dayside: the subsolar closed line moves in.
        let nose = |prm: Params| {
            let mut pts = Vec::new();
            trace(DAY_LATS[4], true, prm, &mut pts);
            pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min)
        };
        assert!(nose(Params::new(GUST, 0.0)) > nose(Params::quiet()) + 0.3);
    }

    #[test]
    fn mag_a7_reconnection_closes_the_innermost_lobe_lines() {
        let tl = Timeline::new(&TIMELINE);
        let before = story(&tl, T_QUIET + T_GUST - 0.01).prm;
        let after = story(&tl, T_QUIET + T_GUST + T_RECON - 0.01).prm;
        let mut pts = Vec::new();
        let flipped: Vec<f64> = NIGHT_LATS
            .iter()
            .copied()
            .filter(|&lat| {
                !trace(lat, false, before, &mut pts) && trace(lat, false, after, &mut pts)
            })
            .collect();
        assert!(
            !flipped.is_empty(),
            "no lobe line closes during reconnection"
        );
        // Only the innermost lobe lines (the lowest open latitudes) close.
        let open_before: Vec<f64> = NIGHT_LATS
            .iter()
            .copied()
            .filter(|&lat| !trace(lat, false, before, &mut pts))
            .collect();
        assert_eq!(flipped[0], open_before[0], "{flipped:?} of {open_before:?}");
        assert!(flipped.len() <= 2);
        // And the scene's traced lines show it (amortised retracing has caught up).
        let (mut g, mut ctx) = scene(Size::new(200, 50));
        run(&mut g, &mut ctx, T_QUIET + T_GUST - 0.05);
        let open_then: Vec<bool> = g.lines.iter().map(|l| !l.closed).collect();
        run(&mut g, &mut ctx, T_RECON);
        let closed_now = g
            .lines
            .iter()
            .zip(&open_then)
            .filter(|(l, was_open)| **was_open && l.closed && !l.day)
            .count();
        assert!(closed_now >= 1);
        assert_eq!(g.caption(), Some("reconnection"));
    }

    #[test]
    fn mag_aurora_caps_span_several_cells_at_each_pole() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = scene(size);
        run(&mut g, &mut ctx, T_QUIET + T_GUST + T_RECON + 0.5);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let (_, ey) = g.to_dots(0.0, 0.0);
        let earth_row = (ey / 4.0) as usize;
        for north in [true, false] {
            let cols: Vec<usize> = canvas
                .cells()
                .iter()
                .enumerate()
                .filter(|(i, c)| c.ink == AURORA_INK && ((i / 200) < earth_row) == north)
                .map(|(i, _)| i % 200)
                .collect();
            let (lo, hi) = (*cols.iter().min().unwrap(), *cols.iter().max().unwrap());
            assert!(
                hi - lo + 1 >= 5,
                "north={north}: aurora spans {} columns",
                hi - lo + 1
            );
            // One unbroken cap over the pole, not separate day and night fans.
            for x in lo..=hi {
                assert!(cols.contains(&x), "north={north}: gap at column {x}");
            }
        }
        // And none of it under the Earth's disc cells.
        assert!(
            g.earth
                .iter()
                .all(|&(x, y, _, _)| { canvas.get(x, y).unwrap().ink.role == Role::Planet })
        );
    }

    #[test]
    fn mag_wind_flows_round_the_shock_never_into_the_magnetopause() {
        let (mut g, mut ctx) = scene(Size::new(120, 40));
        for _ in 0..1800 {
            g.update(SIM_DT, &mut ctx);
            let p = g.story().prm.p;
            for w in &g.wind {
                assert!(w.x == VIEW_X0 || parabola_s(w.x, w.z) >= standoff(p) - 1e-9);
            }
        }
    }

    #[test]
    fn mag_earth_is_tiny_and_tiny_sizes_do_not_panic() {
        let (mut g, mut ctx) = scene(Size::new(200, 50));
        run(&mut g, &mut ctx, 1.0);
        assert!(
            !g.earth.is_empty() && g.earth.len() <= 30,
            "{}",
            g.earth.len()
        );
        for (w, h) in [(1, 1), (3, 2), (1, 7), (500, 150)] {
            let (mut g, mut ctx) = scene(Size::new(w, h));
            for dt in [0.0, 0.25, SIM_DT] {
                g.update(dt, &mut ctx);
                g.prepare();
            }
            let mut c = Canvas::new(Size::new(w, h), GlyphMode::Ascii);
            g.render(&mut c);
        }
    }
}
