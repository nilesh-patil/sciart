//! The Sun's limb in extreme ultraviolet: a loop arcade, a rising flux rope, a flare and
//! a coronal mass ejection (SPEC-002 `solar_eruption`).
//!
//! The corona is a 2.5-D potential field over a flat photosphere, drawn onto a gently
//! curved limb: four photospheric sources (line "charges" below the surface) make the
//! arcade, and a flux rope is a line current with its image below the surface (which
//! keeps the field line-tied). Field lines are traced with RK2 from footpoints and cached;
//! at most `RETRACE` of them are re-traced per frame. As the rope rises (a torus-instability
//! takeoff, `h ∝ e^{aτ²}`) an X-point opens beneath it; field lines closing under the
//! X-point are the freshly reconnected post-flare loops, hot then cooling, and their
//! footpoints — the flare ribbons — move apart as the X-point climbs. The rope leaves as a
//! three-part coronal mass ejection (bright front, dark cavity, bright core), and cooled
//! plasma rains back down the new loops.
//!
//! Units: lengths in `L` (≈ 50 Mm), simulated seconds, hours of evolution compressed.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Tone, ease, ink};

use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "solar_eruption",
    category: Category::SunPlanets,
    description: "Loops rise and snap: a flux rope erupts, flare ribbons flash, a CME escapes",
    title: "Solar eruption · loops snap",
    captions: &["loops rise", "flare", "mass ejection", "coronal rain"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SolarEruption::new())
}

const ACTS: [(&str, f64); 5] = [
    ("quiet", 6.0),
    ("loops rise", 8.0),
    ("flare", 6.0),
    ("mass ejection", 10.0),
    ("coronal rain", 12.0),
];
const QUIET: usize = 0;
const RISE: usize = 1;
const FLARE: usize = 2;
const CME: usize = 3;
const RAIN: usize = 4;

/// Photospheric sources: `(x, depth, strength)`.
const SOURCES: [(f64, f64, f64); 4] = [
    (-0.42, 0.28, 1.0),
    (0.42, 0.28, -1.0),
    (-1.15, 0.35, 0.55),
    (1.15, 0.35, -0.55),
];
/// Rope current (negative: its field opposes the arcade's beneath it) and its starting
/// height; the takeoff `h = H0·e^{a τ²}` over the rise and flare.
const ROPE_I: f64 = -1.6;
const H0: f64 = 0.3;
const TAKEOFF: f64 = 0.0125;
/// Radius of the limb's curvature in `L` (a gentle arc on screen).
const LIMB_R: f64 = 4.0;
/// Field lines: arcade footpoints, rope lines, and the per-frame re-trace budget
/// (ERUP-A5).
const ARCADE: usize = 32;
const ROPE_LINES: usize = 10;
pub(crate) const LINES: usize = ARCADE + ROPE_LINES;
pub(crate) const RETRACE: usize = 20;
const MAX_PTS: usize = 180;
const DS: f64 = 0.025;
/// Domain of the traced field.
const X_MAX: f64 = 2.6;
const Y_MAX: f64 = 3.5;
/// Rain blobs.
const BLOBS: usize = 200;
/// Post-flare loops cool over this many seconds.
const COOL: f64 = 9.0;

/// Point in the corona: `x` along the limb, `y` height, in `L`.
type P = [f64; 2];

/// The field of a 2-D line source of `q` at `(x0, y0)`.
#[inline]
fn source(p: P, x0: f64, y0: f64, q: f64) -> P {
    let (dx, dy) = (p[0] - x0, p[1] - y0);
    let r2 = (dx * dx + dy * dy).max(1e-6);
    [q * dx / r2, q * dy / r2]
}

/// The field of a line current `i` at `(x0, y0)` (counter-clockwise for `i > 0`).
#[inline]
fn current(p: P, x0: f64, y0: f64, i: f64) -> P {
    let (dx, dy) = (p[0] - x0, p[1] - y0);
    let r2 = (dx * dx + dy * dy).max(1e-4);
    [-i * dy / r2, i * dx / r2]
}

/// The coronal field with the rope at height `h` carrying `i`.
pub(crate) fn field(p: P, h: f64, i: f64) -> P {
    let mut b = [0.0, 0.0];
    for &(x0, d, q) in &SOURCES {
        let s = source(p, x0, -d, q);
        b[0] += s[0];
        b[1] += s[1];
    }
    if i != 0.0 {
        let r = current(p, 0.0, h, i);
        let m = current(p, 0.0, -h, -i);
        b[0] += r[0] + m[0];
        b[1] += r[1] + m[1];
    }
    b
}

fn unit(b: P) -> P {
    let n = b[0].hypot(b[1]);
    if n > 1e-12 {
        [b[0] / n, b[1] / n]
    } else {
        [0.0, 0.0]
    }
}

/// Rope height at time `tau` seconds after the rise began: monotonic, with an increasing
/// log-slope `2aτ` (ERUP-A2).
pub(crate) fn rope_height(tau: f64) -> f64 {
    H0 * (TAKEOFF * tau.max(0.0) * tau.max(0.0)).min(30.0).exp()
}

/// A traced field line.
#[derive(Debug, Clone)]
struct Line {
    pts: [[f32; 2]; MAX_PTS],
    len: usize,
    /// Apex height, footpoints (`x` where it starts and ends on the surface, NaN if it
    /// leaves the domain) and whether it closes on itself (a rope line).
    apex: f64,
    foot: (f64, f64),
    closed: bool,
    /// Seconds since the X-point last passed over it (∞: never reconnected).
    age: f64,
    /// Relative brightness of this strand.
    gain: f32,
}

impl Default for Line {
    fn default() -> Self {
        Self {
            pts: [[0.0; 2]; MAX_PTS],
            len: 0,
            apex: 0.0,
            foot: (f64::NAN, f64::NAN),
            closed: false,
            age: f64::INFINITY,
            gain: 1.0,
        }
    }
}

/// Trace from `start` along `sign·B` with RK2 steps of `DS`, writing into `line`.
fn trace(line: &mut Line, start: P, sign: f64, h: f64, i: f64, rope_seed: bool) {
    let mut p = start;
    line.len = 0;
    line.apex = start[1];
    line.foot = (start[0], f64::NAN);
    line.closed = false;
    for k in 0..MAX_PTS {
        line.pts[k] = [p[0] as f32, p[1] as f32];
        line.len = k + 1;
        line.apex = line.apex.max(p[1]);
        let d1 = unit(field(p, h, i));
        let mid = [
            p[0] + 0.5 * DS * sign * d1[0],
            p[1] + 0.5 * DS * sign * d1[1],
        ];
        let d2 = unit(field(mid, h, i));
        let next = [p[0] + DS * sign * d2[0], p[1] + DS * sign * d2[1]];
        if d2 == [0.0, 0.0] {
            break;
        }
        if next[1] < 0.0 {
            // Land exactly on the surface.
            let f = p[1] / (p[1] - next[1]);
            let end = [p[0] + f * (next[0] - p[0]), 0.0];
            if line.len < MAX_PTS {
                line.pts[line.len] = [end[0] as f32, 0.0];
                line.len += 1;
            }
            line.foot.1 = end[0];
            return;
        }
        if next[0].abs() > X_MAX || next[1] > Y_MAX {
            return;
        }
        if rope_seed && k > 20 && (next[0] - start[0]).hypot(next[1] - start[1]) < 0.7 * DS {
            line.closed = true;
            line.pts[line.len.min(MAX_PTS - 1)] = [start[0] as f32, start[1] as f32];
            line.len = (line.len + 1).min(MAX_PTS);
            return;
        }
        p = next;
    }
}

#[derive(Debug, Clone, Copy)]
struct Blob {
    line: usize,
    /// Index position along the line's points (fractional) and speed, points/s.
    s: f64,
    speed: f64,
    /// −1 towards the start, +1 towards the end.
    dir: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct View {
    /// Screen position of the Sun's centre, pixels per `L`.
    cx: f64,
    cy: f64,
    scale: f64,
}

impl View {
    /// Pixel of coronal point `(x, y)`: `x` becomes an angle round the limb.
    fn px(&self, x: f64, y: f64) -> (f64, f64) {
        let (s, c) = (x / LIMB_R).sin_cos_det();
        let r = (LIMB_R + y) * self.scale;
        (self.cx + r * s, self.cy - r * c)
    }
}

#[derive(Debug)]
pub struct SolarEruption {
    t: f64,
    timeline: Timeline,
    lines: Vec<Line>,
    /// Next line to re-trace (round robin).
    cursor: usize,
    blobs: Vec<Blob>,
    view: View,
    glow: Glow,
    cme: Glow,
    tone: Tone,
    cme_tone: Tone,
    pixels: Pixels,
    /// Heights of the rope and the X-point; the ribbons' footpoint `x`.
    h: f64,
    current: f64,
    xpoint: Option<f64>,
    ribbon: f64,
    ribbon_seen: f64,
    size: Size,
}

impl SolarEruption {
    fn new() -> Self {
        let mut lines = Vec::with_capacity(LINES);
        lines.resize_with(LINES, Line::default);
        let mut g = Self {
            t: 0.0,
            timeline: Timeline::new(&ACTS),
            lines,
            cursor: 0,
            blobs: Vec::with_capacity(BLOBS),
            view: View::default(),
            glow: Glow::default(),
            cme: Glow::default(),
            tone: Tone::asinh(0.35, 4.0),
            cme_tone: Tone::asinh(0.5, 5.0),
            pixels: Pixels::default(),
            h: H0,
            current: 0.0,
            xpoint: None,
            ribbon: 0.0,
            ribbon_seen: 0.0,
            size: Size::new(1, 1),
        };
        for (k, line) in g.lines.iter_mut().enumerate() {
            line.gain = 0.55 + 0.45 * hash01(0x10E ^ k as u64) as f32;
        }
        g.drive();
        for k in 0..LINES {
            g.retrace(k);
        }
        g
    }

    /// Rope height and current for the present act.
    fn drive(&mut self) {
        let beat = self.timeline.at(self.t);
        let since_rise = match beat.index {
            RISE => beat.local,
            FLARE => ACTS[RISE].1 + beat.local,
            CME => ACTS[RISE].1 + ACTS[FLARE].1 + beat.local,
            _ => 0.0,
        };
        self.h = rope_height(since_rise);
        self.current = match beat.index {
            // The rope forms slowly during the quiet act (flux cancellation).
            QUIET => ROPE_I * ease::smoothstep(beat.progress),
            CME => ROPE_I * (1.0 - ease::smoothstep(beat.progress)),
            RAIN => 0.0,
            _ => ROPE_I,
        };
        self.xpoint = self.find_xpoint();
    }

    /// The X-point under the rope: where `B_x` on the axis changes sign below it.
    fn find_xpoint(&self) -> Option<f64> {
        if self.current == 0.0 {
            return None;
        }
        let bx = |y: f64| field([0.0, y], self.h, self.current)[0];
        let (mut lo, mut hi) = (0.02, 0.97 * self.h);
        if !(bx(lo).is_finite() && bx(lo) * bx(hi) < 0.0) {
            return None;
        }
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if bx(lo) * bx(mid) <= 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Some(0.5 * (lo + hi))
    }

    /// Footpoint of arcade line `k` (positive polarity, left of the inversion line), or
    /// the seed of rope line `k`.
    fn seed(&self, k: usize) -> (P, bool) {
        if k < ARCADE {
            let u = (k as f64 + 0.5) / ARCADE as f64;
            ([-0.02 - 1.55 * u * u, 0.0], false)
        } else {
            let j = (k - ARCADE) as f64 + 1.0;
            ([0.0, self.h + 0.035 * j * (1.0 + 0.3 * self.h)], true)
        }
    }

    fn retrace(&mut self, k: usize) {
        let (start, rope) = self.seed(k);
        let (h, i) = (self.h, self.current);
        let line = &mut self.lines[k];
        if rope && i.abs() < 0.3 * ROPE_I.abs() {
            line.len = 0;
            return;
        }
        trace(line, start, 1.0, h, i, rope);
        if rope && !line.closed {
            // Only closed strands belong to the rope; an open one is arcade flux.
            line.len = 0;
        }
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        self.t += dt;
        self.drive();
        for _ in 0..RETRACE {
            let k = self.cursor;
            self.cursor = (self.cursor + 1) % LINES;
            self.retrace(k);
        }
        // Reconnection: a line closing below the X-point with footpoints inside the
        // ribbons has just been reconnected.
        self.ribbon = self.ribbon_footpoint();
        let act = self.timeline.at(self.t).index;
        for line in &mut self.lines {
            line.age += dt;
            let fresh = matches!(act, FLARE | CME)
                && !line.closed
                && line.foot.1.is_finite()
                && line.foot.0.abs() <= self.ribbon
                && line.foot.0.abs() >= self.ribbon - 0.12;
            if fresh {
                line.age = 0.0;
            }
            if act == QUIET && line.age > 3.0 * COOL {
                line.age = f64::INFINITY;
            }
        }
        if matches!(act, FLARE | CME) {
            self.ribbon_seen = self.ribbon;
        }
        self.rain(dt, act, rng);
    }

    /// `|x|` of the flare ribbons: footpoint of the separatrix through the X-point.
    fn ribbon_footpoint(&self) -> f64 {
        let Some(y) = self.xpoint else {
            return 0.0;
        };
        let mut line = Line::default();
        // Just below the X-point, follow −B down to the positive-polarity surface.
        trace(
            &mut line,
            [0.0, 0.98 * y],
            -1.0,
            self.h,
            self.current,
            false,
        );
        if line.foot.1.is_finite() {
            line.foot.1.abs()
        } else {
            0.0
        }
    }

    fn rain(&mut self, dt: f64, act: usize, rng: &mut impl Rng) {
        // Condensations form near the tops of cooled post-flare loops and slide down.
        // Rain pours off the cooled post-flare loops; a light drizzle falls from the
        // quiet arcade too (thermal non-equilibrium keeps it going between flares).
        let rate = match act {
            RAIN => 25.0,
            QUIET => 5.0,
            _ => 0.0,
        };
        if rate > 0.0 && self.blobs.len() < BLOBS && rng.random_bool((dt * rate).min(1.0)) {
            let loops = self.lines.iter().enumerate().filter(|(_, l)| {
                (act == QUIET || (l.age.is_finite() && l.age > 2.0)) && l.len > 10 && !l.closed
            });
            let n = loops.clone().count();
            if n > 0 {
                let pick = rng.random_range(0..n);
                if let Some((k, l)) = loops.clone().nth(pick) {
                    let top = (0..l.len)
                        .max_by(|&a, &b| l.pts[a][1].total_cmp(&l.pts[b][1]))
                        .unwrap_or(0);
                    // Start just off the apex on a random side, heading downhill.
                    let dir = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                    self.blobs.push(Blob {
                        line: k,
                        s: top as f64 + dir * rng.random_range(0.5..3.0),
                        speed: 0.0,
                        dir,
                    });
                }
            }
        }
        let lines = &self.lines;
        self.blobs.retain_mut(|b| {
            let l = &lines[b.line];
            let k = (b.s.max(0.0) as usize).min(l.len.saturating_sub(2));
            if l.len < 3 {
                return false;
            }
            // Gravity's component along the loop pulls the blob down (ERUP-A4).
            let (a, c) = (l.pts[k], l.pts[k + 1]);
            let mut dy = f64::from(c[1] - a[1]) * b.dir;
            if dy > 0.0 {
                // Uphill this way (the blob sits just across the apex): turn round.
                b.dir = -b.dir;
                dy = -dy;
            }
            let slope = -dy / DS;
            b.speed = (b.speed + 60.0 * slope.max(0.05) * dt).min(40.0);
            b.s += b.dir * b.speed * dt;
            b.s > 0.0 && b.s < (l.len - 1) as f64
        });
        if act != RAIN && act != QUIET {
            self.blobs.clear();
        }
    }

    fn blob_pos(&self, b: &Blob) -> P {
        let l = &self.lines[b.line];
        let k = (b.s.max(0.0) as usize).min(l.len.saturating_sub(2));
        let f = (b.s - k as f64).clamp(0.0, 1.0) as f32;
        let (a, c) = (l.pts[k], l.pts[(k + 1).min(l.len - 1)]);
        [
            f64::from(a[0] + f * (c[0] - a[0])),
            f64::from(a[1] + f * (c[1] - a[1])),
        ]
    }

    fn compose(&mut self) {
        let v = self.view;
        let beat = self.timeline.at(self.t);
        self.glow.clear();
        self.cme.clear();
        // Loops: quiet arcade strands glow and flicker (each heats and cools on its own
        // cycle, as active-region loops do); reconnected loops hot, then cooling. Wide
        // soft strokes let neighbouring strands merge into a glowing arcade instead of
        // beating against the pixel grid (moiré) at small sizes.
        const LOOP_KERNEL: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
        for (k, line) in self.lines.iter().enumerate() {
            if line.len < 2 {
                continue;
            }
            let hot = if line.age.is_finite() {
                (-line.age / COOL).exp()
            } else {
                0.0
            };
            let w = if line.closed {
                // The rope: a bright core during the eruption.
                if matches!(beat.index, FLARE | CME) {
                    1.0
                } else {
                    0.12
                }
            } else {
                let phase = TAU * hash01(0xF11C ^ k as u64);
                let flicker = 0.75 + 0.35 * (0.9 * self.t + phase).sin_det();
                f64::from(line.gain) * (0.7 * flicker + 2.2 * hot)
            };
            let mut prev = v.px(f64::from(line.pts[0][0]), f64::from(line.pts[0][1]));
            for p in &line.pts[1..line.len] {
                let q = v.px(f64::from(p[0]), f64::from(p[1]));
                self.glow.stroke(
                    (2.0 * prev.0, 2.0 * prev.1),
                    (2.0 * q.0, 2.0 * q.1),
                    &LOOP_KERNEL,
                    (0.3 * w) as f32,
                );
                prev = q;
            }
            // The sources are mirror-antisymmetric about the inversion line, so the field
            // is mirror-symmetric: a line that does not close back on the far polarity
            // (it leaves the domain) has a twin on the other side, drawn by reflection.
            if !line.closed && !line.foot.1.is_finite() {
                let mut prev = v.px(-f64::from(line.pts[0][0]), f64::from(line.pts[0][1]));
                for p in &line.pts[1..line.len] {
                    let q = v.px(-f64::from(p[0]), f64::from(p[1]));
                    self.glow.stroke(
                        (2.0 * prev.0, 2.0 * prev.1),
                        (2.0 * q.0, 2.0 * q.1),
                        &LOOP_KERNEL,
                        (0.3 * w) as f32,
                    );
                    prev = q;
                }
            }
        }
        // CME: a bright front round a dark cavity, expanding self-similarly.
        if matches!(beat.index, FLARE | CME) && self.h > 0.9 {
            // The front is drawn at a height that climbs steadily through the frame (the
            // rope itself accelerates away far faster than the view could follow).
            let hc = if beat.index == CME {
                self.h.min(0.7 + 0.9 * beat.progress)
            } else {
                self.h.min(0.7)
            };
            let radius = 0.55 * hc;
            let fade = if beat.index == CME {
                1.0 - 0.7 * beat.progress
            } else {
                ((self.h - 0.9) / 0.8).min(1.0)
            };
            let steps = 160;
            for k in 0..steps {
                let a = TAU * f64::from(k) / f64::from(steps);
                let (s, c) = a.sin_cos_det();
                let (x, y) = (radius * c, hc + radius * s);
                if y < 0.05 || s < -0.35 {
                    continue;
                }
                let (px, py) = v.px(x, y);
                // Brighter on top where the front piles up coronal material.
                let w = (0.6 + 0.4 * s.max(0.0)) * fade;
                const FRONT: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
                self.cme.splat(2.0 * px, 2.0 * py, &FRONT, (2.5 * w) as f32);
            }
        }
        let tone = &self.tone;
        let cme_tone = &self.cme_tone;
        let size = self.pixels.pixel_size();
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let (glow, cme) = (self.glow.values(), self.cme.values());
        let limb_r = LIMB_R * v.scale;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let (dx, dy) = (f64::from(x) + 0.5 - v.cx, f64::from(y) + 0.5 - v.cy);
                let r = dx.hypot(dy);
                let above = r - limb_r;
                let lv = tone.level(glow[i]);
                let lc = cme_tone.level(cme[i]);
                let ink = if lc > 30 && lc > lv {
                    ink::ramp(Ramp::Cobalt, f64::from(lc) / 255.0)
                } else if lv > 16 {
                    ink::ramp(Ramp::Aurora, f64::from(lv) / 255.0)
                } else if above < 0.0 {
                    // The disc in 131 Å: dark, mottled with faint moss.
                    let m = value_noise(0xD15C, f64::from(x) * 0.18, f64::from(y) * 0.36, 1.0);
                    let limb = (1.0 + above / (0.08 * limb_r)).clamp(0.0, 1.0);
                    ink::ramp(Ramp::Aurora, 0.07 + 0.08 * m + 0.12 * limb)
                } else {
                    // The diffuse corona, fading with height.
                    let c = 0.2 * (-above / (0.35 * v.scale)).exp();
                    if c * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                        ink::ramp(Ramp::Aurora, c)
                    } else if hash01(0xC0 ^ cell_key(x, y)) < 0.0008 {
                        ink::ramp(Ramp::Starlight, 0.35)
                    } else {
                        Ink::BLANK
                    }
                };
                self.pixels.set(x, y, ink);
            }
        }
        // Flare ribbons: two bright strips at the footpoints, flashing in the flare.
        let flash = match beat.index {
            FLARE => 1.0,
            CME => (1.0 - beat.progress).max(0.25),
            RAIN => 0.25 * (1.0 - beat.progress),
            _ => 0.0,
        };
        if flash > 0.0 && self.ribbon_seen > 0.0 {
            for side in [-1.0, 1.0] {
                for k in -3i32..=3 {
                    let x = side * (self.ribbon_seen + 0.02 * f64::from(k));
                    let (px, py) = v.px(x, 0.0);
                    let lvl = flash * (1.0 - 0.1 * f64::from(k.abs()));
                    for dy in 0..2 {
                        self.pixels.set(
                            px.floor() as i32,
                            py.floor() as i32 - dy,
                            ink::ramp(Ramp::Starlight, lvl.clamp(0.3, 1.0)),
                        );
                    }
                }
            }
        }
        for b in &self.blobs {
            let p = self.blob_pos(b);
            let (px, py) = v.px(p[0], p[1]);
            self.pixels.set(
                px.floor() as i32,
                py.floor() as i32,
                ink::ramp(Ramp::Starlight, 0.95),
            );
        }
    }
}

impl Generator for SolarEruption {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.glow.resize(size);
        self.cme.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let scale = (w / 4.2).min(h / 1.9).max(0.5);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.84 * h + LIMB_R * scale,
            scale,
        };
        self.compose();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt, &mut ctx.rng);
        }
        self.compose();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }

    fn caption(&self) -> Option<&'static str> {
        let beat = self.timeline.at(self.t);
        (beat.index != QUIET).then_some(beat.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    #[test]
    fn erup_a1_traced_lines_follow_the_field() {
        let mut line = Line::default();
        for (start, h, i) in [
            ([-0.5, 0.0], H0, ROPE_I),
            ([-1.0, 0.0], 1.2, ROPE_I),
            ([-0.3, 0.0], H0, 0.0),
        ] {
            trace(&mut line, start, 1.0, h, i, false);
            assert!(line.len > 10);
            for k in 0..line.len - 2 {
                let (a, b) = (line.pts[k], line.pts[k + 1]);
                let (tx, ty) = (f64::from(b[0] - a[0]), f64::from(b[1] - a[1]));
                let mid = [f64::from(a[0] + b[0]) * 0.5, f64::from(a[1] + b[1]) * 0.5];
                let bf = field(mid, h, i);
                let cos = (tx * bf[0] + ty * bf[1]) / (tx.hypot(ty) * bf[0].hypot(bf[1]));
                assert!(
                    cos > (2.0f64).to_radians().cos_det(),
                    "{}°",
                    cos.acos().to_degrees()
                );
            }
        }
    }

    #[test]
    fn erup_a2_the_rope_takes_off() {
        let (mut last_h, mut last_slope) = (0.0, 0.0);
        for k in 0..=80 {
            let tau = f64::from(k) * 0.1;
            let h = rope_height(tau);
            assert!(h >= last_h);
            let slope = (rope_height(tau + 0.05).ln() - rope_height(tau).ln()) / 0.05;
            assert!(slope >= last_slope - 1e-12, "log-slope fell at {tau}");
            (last_h, last_slope) = (h, slope);
        }
        assert!(rope_height(8.0) > 1.5 * H0);
    }

    #[test]
    fn erup_a3_ribbons_separate_during_the_flare() {
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(1);
        let start = ACTS[0].1 + ACTS[1].1;
        while g.t < start + 1.5 {
            g.step(SIM_DT, &mut rng);
        }
        let early = g.ribbon;
        while g.t < start + ACTS[2].1 - 0.5 {
            g.step(SIM_DT, &mut rng);
        }
        assert!(
            early > 0.0 && g.ribbon > early + 0.05,
            "{early} → {}",
            g.ribbon
        );
    }

    #[test]
    fn erup_a4_rain_falls_toward_the_surface() {
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(2);
        let rain = ACTS[..RAIN].iter().map(|a| a.1).sum::<f64>();
        while g.t < rain + 3.0 {
            g.step(SIM_DT, &mut rng);
        }
        assert!(!g.blobs.is_empty(), "no rain");
        // Follow the oldest blob (new ones are appended, so it stays first) to the ground.
        g.blobs.truncate(1);
        let first = g.blobs[0];
        let mut last = g.blob_pos(&first)[1];
        let mut steps = 0;
        while g.blobs.first().is_some_and(|b| b.line == first.line) && steps < 600 {
            g.step(SIM_DT, &mut rng);
            if let Some(b) = g.blobs.first().filter(|b| b.line == first.line) {
                let y = g.blob_pos(b)[1];
                assert!(y <= last + 1e-3, "a blob climbed: {last} → {y}");
                last = y;
            }
            steps += 1;
        }
        assert!(steps > 0 && last < 0.5, "blob stuck at {last}");
    }

    #[test]
    fn erup_a5_at_most_twenty_lines_retraced_per_frame() {
        const { assert!(RETRACE <= 20) };
        let mut g = SolarEruption::new();
        let mut rng = SplitMix64::new(3);
        let before = g.cursor;
        g.step(SIM_DT, &mut rng);
        let moved = (g.cursor + LINES - before) % LINES;
        assert!(moved <= 20);
    }
}
