//! One moving source, two kinds of wave (SPEC-002 `doppler_effect`): sound, as the source
//! speeds up through the sound barrier into a Mach cone, then light, blueshifted and
//! beamed ahead of a relativistic source.

use std::collections::VecDeque;
use std::f64::consts::TAU;
use std::fmt::Write as _;

use sciart_core::geom::{self, ASPECT};
use sciart_core::ink::{self, Ramp};
use sciart_core::prelude::*;
use sciart_core::text::StackText;
use sciart_core::title::{self, Title};
use sciart_core::{Timeline, draw, ease};

pub static META: Meta = Meta {
    name: "doppler_effect",
    category: Category::Physics,
    description: "A moving source: sound bunches up into a Mach cone, then light shifts blue ahead and red behind",
    title: "Doppler effect · waves bunch up",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 4] = [
    "pitch up ahead",
    "sound barrier",
    "Mach cone",
    "blueshift ahead",
];

pub fn build() -> Box<dyn Generator> {
    Box::new(DopplerEffect::new())
}

/// Wave speed in both acts, rows/s (light is slowed to it for display).
const C: f64 = 10.0;
/// Emission period of the sound act, and the light source's proper period, s.
const PERIOD: f64 = 0.3;
/// The light act's speed β = v/c and its Lorentz factor.
const BETA: f64 = 0.8;
const GAMMA: f64 = 5.0 / 3.0;
/// The light source's rest temperature, K: a Sun-like star.
const T_REST: f64 = 5800.0;
/// The acts' beats (s): sound at Mach 0.5, through Mach 1 to 1.6, then light.
const BEATS: [(&str, f64); 6] = [
    ("cruise", 6.0),
    ("speed up", 5.0),
    ("barrier", 3.0),
    ("break through", 4.0),
    ("supersonic", 8.0),
    ("light", 22.0),
];
/// Mach numbers of the holds.
const M_CRUISE: f64 = 0.5;
const M_FAST: f64 = 1.6;
/// Half-width of the Mach number band captioned as the sound barrier.
const BARRIER: f64 = 0.03;
/// Where the camera keeps the source, as a fraction of the width: subsonic and light, and
/// supersonic (room for the cone); the anchor relaxes between them over `ANCHOR_TAU` s.
const ANCHOR_SLOW: f64 = 0.4;
const ANCHOR_FAST: f64 = 0.72;
const ANCHOR_TAU: f64 = 1.2;
/// Fronts are dropped once they can no longer be seen, and in any case past this multiple
/// of the screen diagonal, which bounds their number (DOP-A2).
const REACH: f64 = 3.0;
/// Seconds over which the other act's fronts fade out when an act begins.
const ACT_FADE: f64 = 2.0;
/// Once the source passes Mach 1 (the "break through" beat), the fronts it emitted below
/// Mach 1 fade out over this many seconds, so the zone of silence ahead of the cone reads
/// dark (DOP-A9).
const SUBSONIC_FADE: f64 = 3.0;
/// Dot spacing along a front (dots are square, 4 per row).
const DOTS_PER_ROW: f64 = 4.0;
const SPACING: f64 = 0.7;
const MAX_SAMPLES: usize = 12_000;
/// Entries of the light act's rest-angle → Blackbody-level table.
const LUT: usize = 129;
const SOUND_RAMP: Ramp = Ramp::Aurora;
const SOUND_SOURCE: Glyph = Glyph::new('◉', '@');
const LIGHT_SOURCE: Glyph = Glyph::new('✦', '*');
const WAKE: [Glyph; 2] = [Glyph::new('━', '='), Glyph::new('─', '-')];

/// Which wave a front carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wave {
    Sound,
    Light,
}

/// One circular wavefront: emitted at `(x0, y0)` (rows, in the medium) at time `t0` by a
/// source moving at `m` times the wave speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Front {
    x0: f64,
    y0: f64,
    t0: f64,
    m: f64,
    wave: Wave,
}

impl Front {
    fn radius(&self, t: f64) -> f64 {
        C * (t - self.t0)
    }
}

/// Mach number (sound) or β (light) of the source at scene time `t`, and its act.
fn speed(timeline: &Timeline, t: f64) -> (f64, Wave) {
    let beat = timeline.at(t);
    let p = ease::smoothstep(beat.progress);
    let m = match beat.index {
        0 => M_CRUISE,
        1 => ease::lerp(M_CRUISE, 1.0, p),
        2 => 1.0,
        3 => ease::lerp(1.0, M_FAST, p),
        4 => M_FAST,
        _ => return (BETA, Wave::Light),
    };
    (m, Wave::Sound)
}

/// The emission period in the lab frame: time-dilated by γ for the light source.
fn period(wave: Wave) -> f64 {
    match wave {
        Wave::Sound => PERIOD,
        Wave::Light => GAMMA * PERIOD,
    }
}

/// Half-angle of the Mach cone, `asin(1/M)`, or `None` at or below Mach 1 (DOP-A3).
pub(crate) fn mach_half_angle(m: f64) -> Option<f64> {
    (m > 1.0).then(|| (1.0 / m).asin())
}

/// Relativistic Doppler factor `D = 1/(γ(1 − β cos θ))` for lab direction θ (DOP-A4).
pub(crate) fn doppler_factor(beta: f64, cos_lab: f64) -> f64 {
    let gamma = 1.0 / (1.0 - beta * beta).sqrt();
    1.0 / (gamma * (1.0 - beta * cos_lab))
}

/// Aberration: the lab-frame cosine of a photon emitted at rest-frame cosine `cos_rest`
/// by a source moving at β along +x, `(cos θ′ + β)/(1 + β cos θ′)` (DOP-A4).
pub(crate) fn aberrate(beta: f64, cos_rest: f64) -> f64 {
    (cos_rest + beta) / (1.0 + beta * cos_rest)
}

/// The rest-frame angle of a photon seen at lab angle `lab` (radians from the motion),
/// the inverse of [`aberrate`]: `cos θ′ = (cos θ − β)/(1 − β cos θ)`, same side (DOP-A4).
pub(crate) fn rest_angle(beta: f64, lab: f64) -> f64 {
    let (s, c) = lab.sin_cos_det();
    let cos_rest = ((c - beta) / (1.0 - beta * c)).clamp(-1.0, 1.0);
    cos_rest.acos().copysign(s)
}

/// Where the fronts cross the source's axis at time `t`: `(ahead, behind)`, each sorted by
/// distance from the source (DOP-A1). Crossings are `x0 ± c(t − t0)`.
#[cfg(test)]
pub(crate) fn axis_crossings<'a>(
    fronts: impl IntoIterator<Item = &'a Front>,
    t: f64,
    source_x: f64,
) -> (Vec<f64>, Vec<f64>) {
    let (mut ahead, mut behind) = (Vec::new(), Vec::new());
    for f in fronts {
        let r = f.radius(t);
        if f.x0 + r > source_x {
            ahead.push(f.x0 + r);
        }
        if f.x0 - r < source_x {
            behind.push(f.x0 - r);
        }
    }
    ahead.sort_by(f64::total_cmp);
    behind.sort_by(|a, b| b.total_cmp(a));
    (ahead, behind)
}

/// Light a front point `p` (dot units) two dots thick, the second one dot inward along the
/// front's outward direction `(c, s)`: single-dot braille lines read as haze.
fn stroke(dots: &mut Dots, p: (f64, f64), (c, s): (f64, f64), ink: Ink) {
    dots.plot(p.0, p.1, ink);
    dots.plot(p.0 - c, p.1 - s, ink);
}

/// How far up the sound act's ramp a front point sits: the pitch heard in its direction,
/// `f'/f = 1/|1 − M cos θ|`, through `q = tanh(ln(f'/f)/2) = (1 − s)/(1 + s)` with
/// `s = |1 − M cos θ|`, as `0.45 + 1.2 q` in `[0.1, 1]` (DOP-A8). Unshifted is 0.45; at
/// Mach 0.5 ahead (×2) is 0.85 and behind (×0.67) 0.21; the Mach-1 pile-up and the cone
/// (s → 0) saturate at 1.
pub(crate) fn pitch_value(m: f64, cos: f64) -> f64 {
    let s = (1.0 - m * cos).abs();
    (0.45 + 1.2 * (1.0 - s) / (1.0 + s)).clamp(0.1, 1.0)
}

/// Upper bound on live fronts for a screen diagonal of `diag` rows (DOP-A2): the fronts
/// out to [`REACH`] diagonals of both acts, as both are live while one fades out.
pub(crate) fn max_fronts(diag: f64) -> f64 {
    REACH * diag / C * (1.0 / period(Wave::Sound) + 1.0 / period(Wave::Light)) + 2.0
}

#[derive(Debug)]
pub struct DopplerEffect {
    timeline: Timeline,
    t: f64,
    /// Source position in the medium, rows (x already divided by ASPECT).
    sx: f64,
    sy: f64,
    /// Camera: the source's fraction of the width, relaxing toward the act's anchor.
    anchor: f64,
    /// Screen width in rows, height in cells, diagonal in rows.
    width: f64,
    height: u16,
    diag: f64,
    /// Time since the last emission.
    acc: f64,
    fronts: VecDeque<Front>,
    /// The light act's Blackbody ink by rest-frame cosine, `-1..=1` over `LUT` entries.
    light_inks: [Ink; LUT],
    dots: Dots,
    ready: bool,
}

impl DopplerEffect {
    fn new() -> Self {
        Self {
            timeline: Timeline::new(&BEATS),
            t: 0.0,
            sx: 0.0,
            sy: 0.0,
            anchor: ANCHOR_SLOW,
            width: 0.0,
            height: 0,
            diag: 0.0,
            acc: 0.0,
            fronts: VecDeque::new(),
            light_inks: std::array::from_fn(|i| {
                let cos_rest = 2.0 * i as f64 / (LUT - 1) as f64 - 1.0;
                // D = γ(1 + β cos θ′), and a Doppler-shifted blackbody is a blackbody at
                // D·T (DOP-A4).
                ink::blackbody(GAMMA * (1.0 + BETA * cos_rest) * T_REST)
            }),
            dots: Dots::default(),
            ready: false,
        }
    }

    fn speed(&self) -> (f64, Wave) {
        speed(&self.timeline, self.t)
    }

    /// The camera's left edge in the medium, rows.
    fn cam(&self) -> f64 {
        self.sx - self.anchor * self.width
    }

    fn target_anchor(&self) -> f64 {
        match self.speed() {
            (m, Wave::Sound) => {
                ease::lerp(ANCHOR_SLOW, ANCHOR_FAST, ease::smoothstep((m - 1.0) / 0.6))
            }
            (_, Wave::Light) => ANCHOR_SLOW,
        }
    }

    /// The screen in the medium, rows: `(left, right, top, bottom)`.
    fn screen(&self) -> (f64, f64, f64, f64) {
        let cam = self.cam();
        (cam, cam + self.width, 0.0, f64::from(self.height))
    }

    /// Drop the fronts that can never be seen again (DOP-A2): those faded out
    /// ([`Self::shown`]), those past [`REACH`] diagonals, and those whose rightmost point
    /// is left of `sx − ANCHOR_FAST·width`, the leftmost the camera can reach (the camera
    /// can step left while the anchor rises). Only fronts emitted above Mach 1 fall behind
    /// that line, and it then moves at v ≥ c, so they never catch up; any left over into
    /// the light act go with its act fade. A circle that holds the whole screen is kept: a
    /// supersonic source later overtakes it.
    fn prune(&mut self) {
        let (t, reach) = (self.t, REACH * self.diag);
        let left = self.sx - ANCHOR_FAST * self.width;
        let sound = [self.shown(Wave::Sound, 0.0), self.shown(Wave::Sound, 1.0)];
        let light = self.shown(Wave::Light, BETA);
        self.fronts.retain(|f| {
            let shown = match f.wave {
                Wave::Sound => sound[usize::from(f.m >= 1.0)],
                Wave::Light => light,
            };
            let r = f.radius(t);
            shown > 0.0 && r <= reach && f.x0 + r >= left
        });
    }

    /// Seconds since the current act began.
    fn act_age(&self) -> f64 {
        match self.speed().1 {
            Wave::Sound => self.t.rem_euclid(self.timeline.total()),
            Wave::Light => self.timeline.at(self.t).local,
        }
    }

    /// How much of a front of `wave`, emitted at speed `m`, is still shown. Fronts left
    /// over from the other act fade out over [`ACT_FADE`] s while the new act opens on its
    /// own steady state (DOP-A10). In the sound act, once the source breaks through Mach 1
    /// its subsonic fronts fade out over [`SUBSONIC_FADE`] s (DOP-A9).
    fn shown(&self, wave: Wave, m: f64) -> f64 {
        let (now, age) = (self.speed().1, self.act_age());
        if wave != now {
            return (1.0 - age / ACT_FADE).clamp(0.0, 1.0);
        }
        let t_break: f64 = BEATS[..3].iter().map(|b| b.1).sum();
        if wave == Wave::Sound && m < 1.0 && age > t_break {
            (1.0 - (age - t_break) / SUBSONIC_FADE).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// The arc of a circle of radius `r` centred at `(cx, cy)` that can meet the screen,
    /// as a start angle and a span: none when the circle passes inside the screen's
    /// nearest point or outside its farthest corner; else the whole circle when the centre
    /// is on screen, else the cone the screen subtends from the centre (under π, as the
    /// screen is convex).
    fn visible_arc(&self, cx: f64, cy: f64, r: f64) -> (f64, f64) {
        let (x0, x1, y0, y1) = self.screen();
        let near = (cx.clamp(x0, x1) - cx).hypot(cy.clamp(y0, y1) - cy);
        let far = (cx - x0).abs().max((cx - x1).abs());
        let far = far.hypot((cy - y0).abs().max((cy - y1).abs()));
        if r < near || r > far {
            return (0.0, 0.0);
        }
        if near == 0.0 {
            return (0.0, TAU);
        }
        let base = (0.5 * (y0 + y1) - cy).atan2(0.5 * (x0 + x1) - cx);
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let d = geom::wrap_angle((y - cy).atan2(x - cx) - base);
            (lo, hi) = (lo.min(d), hi.max(d));
        }
        (base + lo, hi - lo)
    }

    /// Add the fronts a source that had been moving forever at the current speed would
    /// have emitted, so the first frame and each act open on the steady-state pattern
    /// (DOP-A10).
    fn prefill(&mut self) {
        let (m, wave) = self.speed();
        let t = period(wave);
        let n = (REACH * self.diag / (C * t)).floor() as u32;
        for k in (0..=n).rev() {
            let back = self.acc + f64::from(k) * t;
            self.fronts.push_back(Front {
                x0: self.sx - m * C * back,
                y0: self.sy,
                t0: self.t - back,
                m,
                wave,
            });
        }
        self.prune();
    }

    /// Draw every front onto the dot lattice.
    fn rasterise(&mut self) {
        self.dots.clear();
        let (cam, t) = (self.cam(), self.t);
        // Fronts fade out linearly as their radius approaches [`REACH`], where they go.
        let reach = (REACH * self.diag).max(1e-9);
        for f in &self.fronts {
            let r = f.radius(t);
            if r < 0.25 {
                continue;
            }
            let (fade, shown) = ((1.0 - r / reach).clamp(0.0, 1.0), self.shown(f.wave, f.m));
            // Samples per radian of the circle, and the lab arc that can meet the screen.
            let per_rad = r * DOTS_PER_ROW / SPACING;
            let (start, span) = self.visible_arc(f.x0, f.y0, r);
            if shown <= 0.0 || span <= 0.0 {
                continue;
            }
            let dot = |x: f64, y: f64| Dots::from_cells((x - cam) * ASPECT, y);
            match f.wave {
                Wave::Sound => {
                    let n = ((span * per_rad).ceil() as usize).clamp(2, MAX_SAMPLES);
                    let (ds, dc) = (span / n as f64).sin_cos_det();
                    let (mut s, mut c) = start.sin_cos_det();
                    for _ in 0..=n {
                        // Up the ramp with the pitch heard that way (DOP-A8). A fading
                        // front loses its dots one by one, never all at once.
                        let level = shown * (25.0 + 230.0 * fade * pitch_value(f.m, c));
                        if level >= ink::MIN_VISIBLE_LEVEL {
                            let (x, y) = dot(f.x0 + r * c, f.y0 + r * s);
                            let ink = ink::ramp(SOUND_RAMP, level / 255.0);
                            stroke(&mut self.dots, (x, y), (c, s), ink);
                        }
                        (c, s) = (c * dc - s * ds, s * dc + c * ds);
                    }
                }
                Wave::Light => {
                    // Photons evenly spaced in the source's rest frame, placed at their
                    // aberrated lab angles: they crowd forward (beaming, DOP-A5). The
                    // visible lab arc maps to a rest-frame arc (aberration keeps order).
                    let (start, span) = if span >= TAU {
                        (0.0, TAU)
                    } else {
                        let a = rest_angle(BETA, start);
                        (a, (rest_angle(BETA, start + span) - a).rem_euclid(TAU))
                    };
                    let density = per_rad * fade.max(0.3) * shown;
                    let n = ((span * density).ceil() as usize).clamp(2, MAX_SAMPLES);
                    let (ds, dc) = (span / n as f64).sin_cos_det();
                    let (mut s, mut c) = start.sin_cos_det();
                    for _ in 0..=n {
                        let den = 1.0 + BETA * c;
                        let (lc, ls) = (aberrate(BETA, c), s / (GAMMA * den));
                        let i = ((c + 1.0) * 0.5 * (LUT - 1) as f64).round() as usize;
                        let (x, y) = dot(f.x0 + r * lc, f.y0 + r * ls);
                        stroke(
                            &mut self.dots,
                            (x, y),
                            (lc, ls),
                            self.light_inks[i.min(LUT - 1)],
                        );
                        (c, s) = (c * dc - s * ds, s * dc + c * ds);
                    }
                }
            }
        }
    }

    /// The bottom-right readout, longest form that fits `room` (GEN-R13).
    fn readout(&self, room: usize) -> StackText<48> {
        let (m, wave) = self.speed();
        let mut forms = [StackText::<48>::new(), StackText::new(), StackText::new()];
        let [long, mid, short] = &mut forms;
        match wave {
            Wave::Sound => {
                let _ = write!(short, "M {m:.2}");
                if m < 1.0 - BARRIER {
                    let (ahead, behind) = (1.0 / (1.0 - m), 1.0 / (1.0 + m));
                    let _ = write!(
                        long,
                        "M {m:.2}  pitch x{ahead:.2} ahead  x{behind:.2} behind"
                    );
                    let _ = write!(mid, "M {m:.2}  x{ahead:.2} ahead");
                } else if let Some(a) = mach_half_angle(m).filter(|_| m > 1.0 + BARRIER) {
                    let _ = write!(long, "M {m:.2}  cone {:.0} deg", a.to_degrees());
                }
            }
            Wave::Light => {
                let hot = (doppler_factor(BETA, 1.0) * T_REST / 100.0).round() * 100.0;
                let cool = (doppler_factor(BETA, -1.0) * T_REST / 100.0).round() * 100.0;
                let _ = write!(long, "v {BETA:.2}c  {hot:.0} K ahead  {cool:.0} K behind");
                let _ = write!(mid, "v {BETA:.2}c  {hot:.0} K ahead");
                let _ = write!(short, "v {BETA:.2}c");
            }
        }
        forms
            .into_iter()
            .find(|f| !f.is_empty() && f.len() <= room)
            .unwrap_or_else(StackText::new)
    }
}

impl Generator for DopplerEffect {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.width = f64::from(size.width()) / ASPECT;
        self.height = size.height();
        self.diag = self.width.hypot(f64::from(size.height()));
        // Keep the pattern centred on the source's row.
        let sy = 0.5 * f64::from(size.height());
        for f in &mut self.fronts {
            f.y0 += sy - self.sy;
        }
        self.sy = sy;
        self.dots.resize(size);
        if self.ready {
            self.prune();
        } else {
            self.ready = true;
            self.prefill();
        }
        let bound = max_fronts(self.diag) as usize;
        self.fronts.reserve(bound.saturating_sub(self.fronts.len()));
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let act = self.speed().1;
        self.t += dt;
        let (m, wave) = self.speed();
        self.sx += m * C * dt;
        self.acc += dt;
        if wave != act {
            self.prefill();
        }
        // Back-date each emission by the overshoot so fronts are exactly T apart in time
        // and vT apart in space regardless of the step size (DOP-A1).
        let t_emit = period(wave);
        while self.acc >= t_emit {
            self.acc -= t_emit;
            self.fronts.push_back(Front {
                x0: self.sx - m * C * self.acc,
                y0: self.sy,
                t0: self.t - self.acc,
                m,
                wave,
            });
        }
        self.anchor += (self.target_anchor() - self.anchor) * (1.0 - (-dt / ANCHOR_TAU).exp());
        self.prune();
        debug_assert!(self.fronts.len() as f64 <= max_fronts(self.diag));
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        // Two clear cells after the title, so fronts never run into the caption's tail.
        if let Some((y, cols)) = title::span(canvas.size(), &Title::of(self)) {
            for x in cols.end..cols.end + 2 {
                canvas.put(x, y, Glyph::ascii(' '), Ink::BLANK);
            }
        }

        let x = ((self.sx - self.cam()) * ASPECT).floor() as i32;
        let y = self.sy.floor() as i32;
        let source = match self.speed().1 {
            Wave::Sound => (SOUND_SOURCE, Ink::new(Role::Accent, 255)),
            Wave::Light => (LIGHT_SOURCE, ink::blackbody(T_REST)),
        };
        canvas.put(x, y, source.0, source.1);
        canvas.put(x - 1, y, WAKE[0], Ink::new(Role::Accent, 150));
        canvas.put(x - 2, y, WAKE[1], Ink::new(Role::Accent, 90));

        // Bottom-right, clear of the title (GEN-R13).
        if self.height > 8 {
            let room = title::readout_room(canvas.size(), &Title::of(self));
            let text = self.readout(room);
            if !text.is_empty() {
                let y = i32::from(self.height) - 1;
                let ink = Ink::new(Role::Text, title::READOUT_LEVEL);
                draw::label_right(canvas, y, text.as_str(), ink);
            }
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(match self.speed() {
            (_, Wave::Light) => CAPTIONS[3],
            (m, _) if m < 1.0 - BARRIER => CAPTIONS[0],
            (m, _) if m <= 1.0 + BARRIER => CAPTIONS[1],
            _ => CAPTIONS[2],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(w: u16, h: u16) -> (DopplerEffect, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(1, size);
        let mut g = DopplerEffect::new();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// Consecutive gaps of a sorted crossing list.
    fn gaps(v: &[f64]) -> Vec<f64> {
        v.windows(2).map(|p| (p[1] - p[0]).abs()).collect()
    }

    const V: f64 = M_CRUISE * C;

    #[test]
    fn dop_a1_axis_spacing_is_c_minus_v_ahead_and_c_plus_v_behind() {
        let (mut g, mut ctx) = fresh(400, 50);
        // Step with a dt that does not divide T, so emission back-dating matters; the
        // cruise beat (Mach 0.5) lasts 6 s.
        for _ in 0..(4.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let stepped: Vec<_> = g.fronts.iter().filter(|f| f.t0 > 0.0).copied().collect();
        assert!(stepped.len() >= 10, "{} fronts", stepped.len());
        let (ahead, behind) = axis_crossings(&stepped, g.t, g.sx);
        assert_eq!(ahead.len(), stepped.len());
        assert_eq!(behind.len(), stepped.len());
        for gap in gaps(&ahead) {
            assert!((gap - (C - V) * PERIOD).abs() < 1e-9, "ahead gap {gap}");
        }
        for gap in gaps(&behind) {
            assert!((gap - (C + V) * PERIOD).abs() < 1e-9, "behind gap {gap}");
        }
    }

    #[test]
    fn dop_a1_holds_for_the_prefilled_steady_state_and_coarse_steps() {
        let (mut g, mut ctx) = fresh(400, 50);
        for _ in 0..12 {
            g.update(0.23, &mut ctx);
        }
        let (ahead, behind) = axis_crossings(&g.fronts, g.t, g.sx);
        assert!(ahead.len() >= 5);
        for gap in gaps(&ahead) {
            assert!((gap - (C - V) * PERIOD).abs() < 1e-9, "ahead gap {gap}");
        }
        for gap in gaps(&behind) {
            assert!((gap - (C + V) * PERIOD).abs() < 1e-9, "behind gap {gap}");
        }
    }

    #[test]
    fn dop_a2_live_fronts_are_bounded_by_the_diagonal() {
        let (mut g, mut ctx) = fresh(200, 50);
        for (w, h) in [(200, 50), (80, 24), (3, 2), (1, 1), (0, 0), (300, 80)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            // 3000 steps × ≈ 1/40 s covers the whole 48 s loop, Mach 1 included.
            for i in 0..3000 {
                g.update(if i % 7 == 0 { 0.25 } else { SIM_DT }, &mut ctx);
                assert!(
                    g.fronts.len() as f64 <= max_fronts(g.diag),
                    "{w}x{h}: {} fronts > {}",
                    g.fronts.len(),
                    max_fronts(g.diag)
                );
            }
        }
    }

    #[test]
    fn dop_a3_supersonic_fronts_are_tangent_to_the_mach_cone() {
        assert_eq!(mach_half_angle(1.0), None);
        assert_eq!(mach_half_angle(0.5), None);
        let (mut g, mut ctx) = fresh(400, 50);
        // Into the supersonic hold (18–26 s), then 5 s more at constant Mach 1.6.
        while g.t < 23.0 {
            g.update(SIM_DT, &mut ctx);
        }
        let (m, wave) = g.speed();
        assert_eq!((m, wave), (M_FAST, Wave::Sound));
        let a = mach_half_angle(m).unwrap();
        assert!((a.sin_det() - 1.0 / 1.6).abs() < 1e-12);
        let mut tangent = 0;
        for f in g.fronts.iter().filter(|f| f.t0 > 18.0) {
            let r = f.radius(g.t);
            // Distance from the front's centre to both cone lines through the source.
            let (dx, dy) = (g.sx - f.x0, g.sy - f.y0);
            for side in [1.0, -1.0] {
                let (ux, uy) = (-a.cos_det(), side * a.sin_det());
                let d = (dx * uy - dy * ux).abs();
                assert!((d - r).abs() < 1e-9, "front {:.2}: {d} vs {r}", f.t0);
            }
            tangent += 1;
        }
        assert!(tangent >= 10, "{tangent} fronts");
    }

    #[test]
    fn dop_a4_relativistic_doppler_and_aberration() {
        let b = BETA;
        let d = |c| doppler_factor(b, c);
        assert!((d(1.0) - ((1.0 + b) / (1.0 - b)).sqrt()).abs() < 1e-12);
        assert!((d(-1.0) - ((1.0 - b) / (1.0 + b)).sqrt()).abs() < 1e-12);
        assert!((d(0.0) - 1.0 / GAMMA).abs() < 1e-12, "transverse redshift");
        assert!((aberrate(b, 1.0) - 1.0).abs() < 1e-12 && (aberrate(b, -1.0) + 1.0).abs() < 1e-12);
        let mut last = -2.0;
        for i in 0..=100 {
            let c = -1.0 + 0.02 * f64::from(i);
            let lab = aberrate(b, c);
            assert!(lab > last, "monotone");
            last = lab;
            assert!((GAMMA * (1.0 + b * c) - d(lab)).abs() < 1e-9);
            // `rest_angle` inverts it, on both sides of the axis.
            for side in [1.0, -1.0] {
                let rest = side * c.acos();
                let lab_angle = side * lab.acos();
                assert!((rest_angle(b, lab_angle) - rest).abs() < 1e-6, "{c} {side}");
            }
        }
        // The table holds blackbody(D·5800 K) by rest-frame cosine.
        let g = DopplerEffect::new();
        assert_eq!(g.light_inks[LUT - 1], ink::blackbody(d(1.0) * T_REST));
        assert_eq!(g.light_inks[0], ink::blackbody(d(-1.0) * T_REST));
        assert_eq!(g.light_inks[LUT / 2], ink::blackbody(T_REST * GAMMA));
    }

    /// Lit dots and mean Blackbody level of the light fronts' dots ahead of and behind the
    /// source, skipping the 4 columns either side of it where the fronts converge.
    fn halves(g: &DopplerEffect) -> ((usize, f64), (usize, f64)) {
        let w = (g.width * ASPECT) as u16;
        let mut canvas = Canvas::new(Size::new(w, g.height), GlyphMode::Unicode);
        g.render(&mut canvas);
        let src = ((g.sx - g.cam()) * ASPECT) as i32;
        let (mut ahead, mut behind) = ((0, 0.0), (0, 0.0));
        for (i, cell) in canvas.cells().iter().enumerate() {
            if !('\u{2801}'..='\u{28ff}').contains(&cell.ch) || cell.ink.role != Role::Blackbody {
                continue;
            }
            let cx = (i % usize::from(w)) as i32;
            let half = if cx > src + 4 {
                &mut ahead
            } else if cx < src - 4 {
                &mut behind
            } else {
                continue;
            };
            let n = (cell.ch as u32 - 0x2800).count_ones() as usize;
            half.0 += n;
            half.1 += f64::from(cell.ink.level) * n as f64;
        }
        let mean = |(n, s): (usize, f64)| (n, s / n.max(1) as f64);
        (mean(ahead), mean(behind))
    }

    #[test]
    fn dop_a5_light_is_beamed_and_bluer_ahead() {
        let (mut g, mut ctx) = fresh(200, 50);
        // 15 s into the light act: its fronts fill the screen, the sound ones are gone.
        while g.t < 26.0 + 15.0 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.fronts.iter().all(|f| f.wave == Wave::Light));
        let ((n_ahead, t_ahead), (n_behind, t_behind)) = halves(&g);
        assert!(
            n_ahead >= 2 * n_behind,
            "dots {n_ahead} ahead vs {n_behind} behind"
        );
        assert!(t_ahead > t_behind + 30.0, "levels {t_ahead} vs {t_behind}");
    }

    #[test]
    fn dop_a6_acts_walk_the_captions_and_inks() {
        let (mut g, mut ctx) = fresh(120, 40);
        let mut seen: Vec<&str> = Vec::new();
        for _ in 0..(2.0 * 48.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let c = g.caption().unwrap();
            if seen.last() != Some(&c) {
                seen.push(c);
            }
            let (m, wave) = g.speed();
            let beat = g.timeline.at(g.t).index;
            if beat == 0 {
                assert_eq!(m, 0.5);
            } else if beat == 2 {
                assert_eq!(m, 1.0);
            } else if beat == 4 {
                assert_eq!(m, 1.6);
            }
            assert_eq!(wave == Wave::Light, beat == BEATS.len() - 1);
        }
        let cycle = [CAPTIONS[0], CAPTIONS[1], CAPTIONS[2], CAPTIONS[3]];
        assert_eq!(seen, [&cycle[..], &cycle[..]].concat());
        // Fronts carry their act's palette.
        let mut stage = Stage::new(build(), 3, Size::new(120, 40), GlyphMode::Unicode);
        for (t, role) in [(3.0, Role::Ramp(Ramp::Aurora)), (40.0, Role::Blackbody)] {
            while stage.ctx().time() < t {
                stage.step(SIM_DT);
            }
            let frame = stage.render();
            let braille = frame
                .cells()
                .iter()
                .filter(|c| ('\u{2801}'..='\u{28ff}').contains(&c.ch));
            let roles: Vec<Role> = braille.map(|c| c.ink.role).collect();
            assert!(roles.len() > 200, "t = {t}: {} braille cells", roles.len());
            assert!(roles.iter().all(|&r| r == role), "t = {t}");
        }
    }

    #[test]
    fn dop_a7_camera_keeps_the_source_on_screen() {
        for (w, h) in [(200, 50), (80, 24), (40, 12)] {
            let (mut g, mut ctx) = fresh(w, h);
            for i in 0..(50.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                let frac = (g.sx - g.cam()) / g.width;
                assert!(
                    (ANCHOR_SLOW - 1e-9..=ANCHOR_FAST + 1e-9).contains(&frac),
                    "{frac}"
                );
                if i % 60 == 0 {
                    let mut canvas = Canvas::new(Size::new(w, h), GlyphMode::Ascii);
                    g.render(&mut canvas);
                    let lines = canvas.to_lines();
                    let row = &lines[usize::from(h / 2)];
                    assert!(
                        row.contains('@') || row.contains('*'),
                        "t = {:.1}: {row}",
                        g.t
                    );
                }
            }
        }
    }

    #[test]
    fn dop_gen_r13_readout_names_the_speed_and_sits_right_of_the_title() {
        // At t = 0 (Mach 0.5) the caption is "pitch up ahead": "Doppler effect · pitch up
        // ahead" is 31 cells, so the room is min(w − 37, w/2).
        let long = "M 0.50  pitch x2.00 ahead  x0.67 behind";
        let mid = "M 0.50  x2.00 ahead";
        for (w, h, readout) in [
            (78, 24, long),
            (77, 24, mid),
            (56, 24, mid),
            (55, 24, "M 0.50"),
            (40, 12, mid),
        ] {
            let mut stage = Stage::new(build(), 3, Size::new(w, h), GlyphMode::Ascii);
            let row = stage.render().to_lines().pop().unwrap();
            assert!(row.ends_with(&format!(" {readout} ")), "{w}x{h}: {row:?}");
        }
        let mut stage = Stage::new(build(), 3, Size::new(200, 50), GlyphMode::Ascii);
        while stage.ctx().time() < 30.0 {
            stage.step(SIM_DT);
        }
        let row = stage.render().to_lines().pop().unwrap();
        assert!(
            row.ends_with(" v 0.80c  17400 K ahead  1900 K behind "),
            "{row:?}"
        );
        while stage.ctx().time() < 48.0 + 22.0 {
            stage.step(SIM_DT);
        }
        let row = stage.render().to_lines().pop().unwrap();
        assert!(row.ends_with(" M 1.60  cone 39 deg "), "{row:?}");
    }

    /// Braille cells of `role` in `frame`, as `(column, level)`.
    fn braille(frame: &Canvas, role: Role) -> Vec<(i32, u8)> {
        let w = usize::from(frame.size().width());
        frame
            .cells()
            .iter()
            .enumerate()
            .filter(|(_, c)| ('\u{2801}'..='\u{28ff}').contains(&c.ch) && c.ink.role == role)
            .map(|(i, c)| ((i % w) as i32, c.ink.level))
            .collect()
    }

    fn source_col(g: &DopplerEffect) -> i32 {
        ((g.sx - g.cam()) * ASPECT).floor() as i32
    }

    #[test]
    fn dop_a8_sound_fronts_are_coloured_by_the_heard_pitch() {
        // f'/f = 1/|1 − M cos θ| through tanh(ln(f'/f)/2): high ahead, low behind.
        let v = pitch_value;
        assert!(v(0.5, 1.0) >= 0.8, "ahead {}", v(0.5, 1.0));
        assert!(v(0.5, -1.0) <= 0.3, "behind {}", v(0.5, -1.0));
        assert!(
            (0.35..0.55).contains(&v(0.5, 0.0)),
            "sideways {}",
            v(0.5, 0.0)
        );
        assert_eq!(v(1.0, 1.0), 1.0, "the Mach-1 pile-up saturates");
        let mut last = 0.0;
        for i in 0..=40 {
            let x = v(0.5, -1.0 + 0.05 * f64::from(i));
            assert!(x >= last && (0.0..=1.0).contains(&x));
            last = x;
        }
        // On screen at Mach 0.5, fronts ahead are much higher on the ramp than behind.
        let (mut g, mut ctx) = fresh(120, 40);
        while g.t < 3.0 {
            g.update(SIM_DT, &mut ctx);
        }
        let mut frame = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut frame);
        let src = source_col(&g);
        let cells = braille(&frame, Role::Ramp(SOUND_RAMP));
        let mean = |keep: &dyn Fn(i32) -> bool| {
            let v: Vec<f64> = cells
                .iter()
                .filter(|c| keep(c.0))
                .map(|c| f64::from(c.1))
                .collect();
            assert!(v.len() > 50, "{} cells", v.len());
            v.iter().sum::<f64>() / v.len() as f64
        };
        let (ahead, behind) = (mean(&|x| x > src + 4), mean(&|x| x < src - 4));
        assert!(
            ahead > behind + 60.0,
            "ahead {ahead:.0} vs behind {behind:.0}"
        );
    }

    #[test]
    fn dop_a9_zone_of_silence_ahead_of_the_supersonic_source_is_dark() {
        for (w, h) in [(200, 50), (120, 40)] {
            let (mut g, mut ctx) = fresh(w, h);
            while g.t < 26.0 - SIM_DT {
                g.update(SIM_DT, &mut ctx);
                if g.t < 18.0 {
                    continue;
                }
                // Past the fade only the fronts emitted at Mach ≥ 1 are left.
                assert!(g.fronts.iter().all(|f| f.m >= 1.0), "t = {:.2}", g.t);
                let mut frame = Canvas::new(Size::new(w, h), GlyphMode::Unicode);
                g.render(&mut frame);
                let src = source_col(&g);
                let ahead = braille(&frame, Role::Ramp(SOUND_RAMP))
                    .into_iter()
                    .filter(|c| c.0 > src + 1)
                    .count();
                assert_eq!(ahead, 0, "{w}x{h} t = {:.2}", g.t);
            }
        }
    }

    #[test]
    fn dop_a10_each_act_opens_on_its_steady_state() {
        let mut stage = Stage::new(build(), 3, Size::new(120, 40), GlyphMode::Unicode);
        for (t, role) in [(26.3, Role::Blackbody), (48.3, Role::Ramp(SOUND_RAMP))] {
            while stage.ctx().time() < t {
                stage.step(SIM_DT);
            }
            let n = braille(stage.render(), role).len();
            assert!(n > 200, "t = {t}: {n} braille cells of {role:?}");
        }
    }

    #[test]
    fn dop_a2_left_pruned_fronts_never_come_back_on_screen() {
        for (w, h) in [(400, 60), (80, 24)] {
            let (mut g, mut ctx) = fresh(w, h);
            // Every step's time and camera, and the fronts pruned while still live.
            let (mut cams, mut pruned) = (Vec::new(), Vec::new());
            for _ in 0..(48.0 / SIM_DT) as usize {
                let before: Vec<Front> = g.fronts.iter().copied().collect();
                g.update(SIM_DT, &mut ctx);
                cams.push((g.t, g.cam(), g.speed().1));
                for f in before {
                    let live = f.radius(g.t) <= REACH * g.diag && g.shown(f.wave, f.m) > 0.0;
                    if live && !g.fronts.contains(&f) {
                        pruned.push((cams.len(), f));
                    }
                }
            }
            for (i, f) in pruned {
                for &(t, cam, wave) in &cams[i..] {
                    if wave != f.wave || f.radius(t) > REACH * g.diag {
                        break;
                    }
                    assert!(
                        f.x0 + f.radius(t) < cam,
                        "{w}x{h}: front {:.2} back at t {t:.2}",
                        f.t0
                    );
                }
            }
        }
    }

    #[test]
    fn dop_a2_visible_arc_is_empty_when_the_circle_misses_the_screen() {
        let (g, _) = fresh(80, 24);
        let (x0, x1, y0, y1) = g.screen();
        let (cx, cy) = (0.5 * (x0 + x1), 0.5 * (y0 + y1));
        let far = (x1 - cx).hypot(y1 - cy);
        assert_eq!(
            g.visible_arc(cx, cy, far + 0.1).1,
            0.0,
            "holds the whole screen"
        );
        assert_eq!(g.visible_arc(cx, cy, 1.0).1, TAU);
        // Centred left of the screen: too small to reach it, or reaching it.
        assert_eq!(g.visible_arc(x0 - 10.0, cy, 9.0).1, 0.0);
        assert!(g.visible_arc(x0 - 10.0, cy, 11.0).1 > 0.0);
    }

    #[test]
    fn dop_gen_r13_two_cells_after_the_title_stay_clear() {
        // The title itself is Stage's; the scene keeps the two cells after its span clear.
        for (w, h) in [(80, 24), (120, 40)] {
            let (mut g, mut ctx) = fresh(w, h);
            while g.t < 48.0 {
                for _ in 0..6 {
                    g.update(SIM_DT, &mut ctx);
                }
                let mut frame = Canvas::new(Size::new(w, h), GlyphMode::Unicode);
                g.render(&mut frame);
                let (y, cols) = title::span(frame.size(), &Title::of(&g)).unwrap();
                for x in cols.end..cols.end + 2 {
                    let c = frame.get(x, y).unwrap();
                    assert_eq!(c.ch, ' ', "{w}x{h} t = {:.1}", g.t);
                }
            }
        }
    }
}
