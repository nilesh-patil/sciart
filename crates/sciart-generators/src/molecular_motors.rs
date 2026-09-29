//! Motor proteins: two molecular machines fuelled by ATP. Kinesin walks hand over hand
//! along a microtubule, one 8 nm step per ATP, jiggling in thermal noise between steps;
//! then ATP synthase's rotor turns in 120° steps, each split into 80° and 40° substeps
//! (SPEC-002 `molecular_motors`).
//!
//! Kinesin: two heads on the microtubule's 8 nm lattice of binding sites. Dwell times are
//! exponential with the Michaelis–Menten rate `k = k_cat[ATP]/(K_M + [ATP])`; each step
//! the trailing head unbinds, diffuses (overdamped Langevin) in a potential pulling it
//! forward (neck-linker docking) and binds 16 nm ahead, so the motor's centre moves
//! exactly 8 nm and the heads alternate. A bound head jiggles in a harmonic well (an
//! Ornstein–Uhlenbeck process with no drift). The cargo bead trails on a spring.
//! `[ATP]` steps down a `Timeline`; dwells are slowed [`SLOW`]× for the eye.
//!
//! ATP synthase, seen from the membrane's top as in Noji's single-molecule films: the
//! α₃β₃ ring round the γ rotor, with an actin filament stuck on γ to make its turns
//! visible. Each 120° turn is an 80° substep after an ATP-binding dwell and a 40°
//! substep after a catalytic dwell, both exponential; each turn releases one ATP.

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::{draw, ease, ink, raster, title};

pub static META: Meta = Meta {
    name: "molecular_motors",
    category: Category::LivingMatter,
    description: "Kinesin walks hand over hand, one 8 nm step per ATP; ATP synthase turns in 120-degree steps",
    title: "Motor proteins · fuelled by ATP",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 2] = ["kinesin walks", "synthase turns"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Motors::default())
}

// ---- Kinesin (SPEC-002 Model) ----

/// Binding-site spacing along the microtubule (nm).
pub(crate) const SITE_NM: f64 = 8.0;
/// Kinesin's catalytic rate (steps per second at saturating ATP) and Michaelis constant.
pub(crate) const K_CAT: f64 = 100.0;
pub(crate) const K_M_UM: f64 = 60.0;
/// Display slow-down of the dwells.
pub(crate) const SLOW: f64 = 40.0;
/// Bound-head well stiffness (per second) and diffusion (nm²/s), display time.
const WELL_K: f64 = 30.0;
const WELL_D: f64 = 6.0;
/// Free-head pull towards its next site and diffusion (display time).
const SWING_K: f64 = 9.0;
const SWING_D: f64 = 20.0;
/// A free head binds within this distance of its site (nm).
const CAPTURE_NM: f64 = 1.2;
/// Model sub-step (s).
const H: f64 = 1.0 / 240.0;

/// The Michaelis–Menten stepping rate at `atp_um` (per second, real time).
pub(crate) fn rate(atp_um: f64) -> f64 {
    K_CAT * atp_um / (K_M_UM + atp_um)
}

/// An exponential dwell with rate `k`.
pub(crate) fn dwell(rng: &mut impl Rng, k: f64) -> f64 {
    let u: f64 = rng.random_range(f64::MIN_POSITIVE..1.0);
    -u.ln() / k
}

/// One step of an overdamped particle in a harmonic well about `centre`.
fn langevin(x: f64, centre: f64, k: f64, d: f64, h: f64, rng: &mut impl Rng) -> f64 {
    x - k * (x - centre) * h + gaussian(rng, 0.0, (2.0 * d * h).sqrt())
}

/// Two heads on a lattice of sites: `site[i]` is head `i`'s site, `free` the head in
/// flight (if any) with its position.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Kinesin {
    pub(crate) site: [i64; 2],
    /// Jiggle of each bound head about its site (nm), and its height offset.
    pub(crate) jig: [[f64; 2]; 2],
    pub(crate) free: Option<(usize, [f64; 2])>,
    /// Seconds (display) until the next step starts.
    pub(crate) wait: f64,
    pub(crate) steps: u64,
    /// Head that moved last.
    pub(crate) last_moved: Option<usize>,
}

impl Kinesin {
    pub(crate) fn new() -> Self {
        Self {
            site: [0, 1],
            jig: [[0.0; 2]; 2],
            free: None,
            wait: 0.5,
            steps: 0,
            last_moved: None,
        }
    }

    /// The motor's centre (nm): the midpoint of its two sites.
    pub(crate) fn centre(&self) -> f64 {
        0.5 * (self.site[0] + self.site[1]) as f64 * SITE_NM
    }

    /// Advance one sub-step at `[ATP]`.
    pub(crate) fn step(&mut self, atp_um: f64, rng: &mut impl Rng) {
        for i in 0..2 {
            if self.free.is_none_or(|(f, _)| f != i) {
                for a in 0..2 {
                    self.jig[i][a] = langevin(self.jig[i][a], 0.0, WELL_K, WELL_D * 0.02, H, rng);
                }
            }
        }
        match self.free {
            None => {
                self.wait -= H;
                if self.wait <= 0.0 {
                    // ATP binds the leading head; the trailing head lets go.
                    let trail = if self.site[0] < self.site[1] { 0 } else { 1 };
                    let pos = [self.site[trail] as f64 * SITE_NM, 0.0];
                    self.free = Some((trail, pos));
                }
            }
            Some((i, mut p)) => {
                let lead = self.site[1 - i];
                let target = (lead + 1) as f64 * SITE_NM;
                // Pulled forward and held above the track by the docked neck linker.
                let arc = -4.0 * (1.0 - ((p[0] - target + SITE_NM) / SITE_NM).powi(2)).max(0.0);
                p[0] = langevin(p[0], target, SWING_K, SWING_D, H, rng);
                p[1] = langevin(p[1], arc, SWING_K * 2.0, SWING_D * 0.2, H, rng);
                if (p[0] - target).abs() < CAPTURE_NM && p[1].abs() < 2.0 {
                    self.site[i] = lead + 1;
                    self.jig[i] = [p[0] - target, 0.0];
                    self.free = None;
                    self.steps += 1;
                    self.last_moved = Some(i);
                    self.wait = dwell(rng, rate(atp_um)) * SLOW;
                } else {
                    self.free = Some((i, p));
                }
            }
        }
    }

    /// Position of head `i` (nm; y up is negative).
    pub(crate) fn head(&self, i: usize) -> [f64; 2] {
        match self.free {
            Some((f, p)) if f == i => p,
            _ => [
                self.site[i] as f64 * SITE_NM + self.jig[i][0],
                self.jig[i][1],
            ],
        }
    }
}

// ---- ATP synthase ----

/// Mean dwells (display seconds) before the 80° and the 40° substeps.
const BIND_DWELL: f64 = 0.45;
const CAT_DWELL: f64 = 0.25;
/// Seconds a substep's rotation takes on screen.
const TURN_S: f64 = 0.08;

/// The rotor: committed angle (degrees, a multiple of 40), and the substep in progress.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rotor {
    /// Angle reached by completed substeps (degrees).
    pub(crate) angle: f64,
    /// Next substep is the 80° one.
    pub(crate) next_big: bool,
    pub(crate) wait: f64,
    /// Seconds into the current substep's rotation, if turning.
    pub(crate) turning: Option<f64>,
    pub(crate) turns: u64,
}

impl Rotor {
    pub(crate) fn new() -> Self {
        Self {
            angle: 0.0,
            next_big: true,
            wait: BIND_DWELL,
            turning: None,
            turns: 0,
        }
    }

    fn substep_deg(&self) -> f64 {
        if self.next_big { 80.0 } else { 40.0 }
    }

    /// Advance by `h`; returns true when a 120° turn completes (one ATP made).
    pub(crate) fn step(&mut self, h: f64, rng: &mut impl Rng) -> bool {
        if let Some(t) = self.turning {
            let t = t + h;
            if t >= TURN_S {
                self.angle += self.substep_deg();
                let done = !self.next_big;
                self.next_big = !self.next_big;
                self.turning = None;
                let mean = if self.next_big { BIND_DWELL } else { CAT_DWELL };
                self.wait = dwell(rng, 1.0 / mean);
                if done {
                    self.turns += 1;
                }
                return done;
            }
            self.turning = Some(t);
        } else {
            self.wait -= h;
            if self.wait <= 0.0 {
                self.turning = Some(0.0);
            }
        }
        false
    }

    /// The angle shown (degrees): committed plus the eased part of the substep in flight.
    pub(crate) fn shown(&self) -> f64 {
        self.angle
            + self
                .turning
                .map_or(0.0, |t| self.substep_deg() * ease::in_out_cubic(t / TURN_S))
    }
}

// ---- Scene ----

const KINESIN_S: f64 = 24.0;
const SYNTHASE_S: f64 = 16.0;
const KINESIN: usize = 0;
/// `[ATP]` (µM) over the kinesin act: saturating, near `K_M`, scarce.
const ATP_STEPS: [(f64, f64); 3] = [(0.0, 1000.0), (8.0, 60.0), (16.0, 15.0)];
const ATP_DOTS: usize = 90;
/// Footprints kept, and the seconds one takes to fade.
const TRAIL: usize = 8;
const TRAIL_S: f64 = 5.0;
/// Cargo vesicle radius and its tether rest height (nm).
const CARGO_R: f64 = 9.0;
const CARGO_Y: f64 = -18.0;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

#[derive(Debug, Clone, Copy, Default)]
struct Dot {
    x: f64,
    y: f64,
    /// Seconds left flying out of the synthase (0: drifting).
    fly: f64,
    vx: f64,
    vy: f64,
}

#[derive(Debug)]
struct Motors {
    kinesin: Kinesin,
    rotor: Rotor,
    /// Cargo bead position (nm), lagging on its spring.
    cargo: [f64; 2],
    /// Camera centre (nm), easing after the motor.
    cam: f64,
    dots: Vec<Dot>,
    /// Sites heads bound to (nm) and when, a ring buffer.
    trail: [(f64, f64); TRAIL],
    trail_len: usize,
    trail_next: usize,
    time: f64,
    owed: f64,
    act: usize,
    timeline: Timeline,
    pixels: Pixels,
    cells: Size,
    readout_room: usize,
    rng: SplitMix64,
}

impl Default for Motors {
    fn default() -> Self {
        Self {
            kinesin: Kinesin::new(),
            rotor: Rotor::new(),
            cargo: [-14.0, CARGO_Y],
            cam: 0.0,
            dots: Vec::with_capacity(ATP_DOTS),
            trail: [(0.0, 0.0); TRAIL],
            trail_len: 0,
            trail_next: 0,
            time: 0.0,
            owed: 0.0,
            act: KINESIN,
            timeline: Timeline::new(&[(CAPTIONS[0], KINESIN_S), (CAPTIONS[1], SYNTHASE_S)]),
            pixels: Pixels::default(),
            cells: Size::new(1, 1),
            readout_room: 0,
            rng: SplitMix64::new(1),
        }
    }
}

/// `[ATP]` at kinesin-act time `t`.
fn atp_at(t: f64) -> f64 {
    ATP_STEPS
        .iter()
        .rev()
        .find(|(from, _)| t >= *from)
        .map_or(ATP_STEPS[0].1, |s| s.1)
}

/// Blend `level` in `role` into pixel `(x, y)` by `cover`: over blank or same-ramp pixels
/// the levels mix (the ramps run to near-black, so this reads as partial coverage); over
/// another ramp the pixel is taken only when mostly covered.
fn blend(px: &mut Pixels, x: i32, y: i32, cover: f64, role: Role, level: f64) {
    let Some(under) = px.get(x, y) else {
        return;
    };
    let base = if under.role == role {
        f64::from(under.level)
    } else if under.role == Role::Background || cover >= 0.5 {
        0.0
    } else {
        return;
    };
    let cover = if under.role == role || under.role == Role::Background {
        cover
    } else {
        1.0
    };
    px.set(
        x,
        y,
        Ink::new(role, ink::level(base + (level - base) * cover)),
    );
}

/// An antialiased, shaded disc: `level(shade, d)` with `shade` in `[0, 1]` (lit from the
/// upper left) and `d` the distance from the centre over `r`.
fn soft_disc(
    px: &mut Pixels,
    c: (f64, f64),
    r: f64,
    role: Role,
    mut level: impl FnMut(f64, f64) -> f64,
) {
    raster::disc_pixels(c, r, |x, y, cover| {
        let (ox, oy) = (f64::from(x) + 0.5 - c.0, f64::from(y) + 0.5 - c.1);
        let shade = 0.5 + 0.5 * (-(ox + oy) / (r * 1.5 + 1.0)).clamp(-1.0, 1.0);
        let l = level(shade, ox.hypot(oy) / r.max(0.5));
        blend(px, x, y, cover, role, l);
    });
}

/// An antialiased stroke of half-width `hw` pixels from `a` to `b`: coverage from the
/// distance to the segment, so a thin rod is smooth at any angle.
fn soft_segment(px: &mut Pixels, a: (f64, f64), b: (f64, f64), hw: f64, ink: Ink) {
    let reach = hw + 1.0;
    let (x0, x1) = (a.0.min(b.0) - reach, a.0.max(b.0) + reach);
    let (y0, y1) = (a.1.min(b.1) - reach, a.1.max(b.1) + reach);
    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
        return;
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = (dx * dx + dy * dy).max(1e-9);
    for y in y0.floor() as i32..=y1.ceil() as i32 {
        for x in x0.floor() as i32..=x1.ceil() as i32 {
            let (qx, qy) = (f64::from(x) + 0.5 - a.0, f64::from(y) + 0.5 - a.1);
            let t = ((qx * dx + qy * dy) / len2).clamp(0.0, 1.0);
            let d = (qx - t * dx).hypot(qy - t * dy);
            let cover = (hw + 0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                blend(px, x, y, cover, ink.role, f64::from(ink.level));
            }
        }
    }
}

/// Screen direction of γ at rotor angle `deg`, viewed from the membrane (Fo) side with
/// y down: 0° points up and the angle grows clockwise on screen, the synthesis sense.
pub(crate) fn gamma_dir(deg: f64) -> (f64, f64) {
    let (s, c) = (deg.to_radians() - PI / 2.0).sin_cos_det();
    (c, s)
}

impl Motors {
    fn local(&self) -> f64 {
        self.time % self.timeline.total()
    }

    fn scale(&self) -> f64 {
        let ps = self.pixels.pixel_size();
        (f64::from(ps.width()) / 60.0)
            .min(f64::from(ps.height()) / 32.0)
            .max(0.4)
    }

    fn scatter_dots(&mut self, n: usize) {
        self.dots.clear();
        for _ in 0..n.min(ATP_DOTS) {
            let (x, y) = (
                self.rng.random_range(-1.0..1.0),
                self.rng.random_range(-1.0..1.0),
            );
            self.dots.push(Dot {
                x,
                y,
                ..Dot::default()
            });
        }
    }

    fn dots_for(atp: f64) -> usize {
        ((atp.log10() - 0.8) * 30.0).clamp(8.0, ATP_DOTS as f64) as usize
    }

    fn step_model(&mut self, h: f64) {
        let local = self.local();
        let act = self.timeline.at(local).index;
        if act != self.act {
            self.act = act;
            if act == KINESIN {
                self.kinesin = Kinesin::new();
                self.cargo = [-14.0, CARGO_Y];
                self.cam = 0.0;
                self.trail_len = 0;
                self.scatter_dots(Self::dots_for(atp_at(0.0)));
            } else {
                self.rotor = Rotor::new();
                self.dots.clear();
            }
        }
        if act == KINESIN {
            let atp = atp_at(local);
            let want = Self::dots_for(atp);
            if self.dots.len() > want {
                self.dots.truncate(want);
            }
            let mut rng = self.rng.clone();
            let before = self.kinesin.steps;
            self.kinesin.step(atp, &mut rng);
            self.rng = rng;
            if self.kinesin.steps != before
                && let Some(i) = self.kinesin.last_moved
            {
                self.trail[self.trail_next] = (self.kinesin.site[i] as f64 * SITE_NM, self.time);
                self.trail_next = (self.trail_next + 1) % TRAIL;
                self.trail_len = (self.trail_len + 1).min(TRAIL);
            }
            // The cargo lags behind on its tether.
            let c = self.kinesin.centre();
            self.cargo[0] += (c - 16.0 - self.cargo[0]) * (2.0 * h).min(1.0);
            self.cargo[1] += (CARGO_Y - self.cargo[1]) * (2.0 * h).min(1.0);
            self.cargo[0] += gaussian(&mut self.rng, 0.0, 0.15);
            // A lazy camera: the motor walks across the frame, and the view pans only
            // once it strays 14 nm from the centre, so every step shows.
            let off = c - self.cam;
            let zone = (0.1 * f64::from(self.pixels.pixel_size().width()) / self.scale()).min(14.0);
            if off.abs() > zone {
                self.cam += (off - zone * off.signum()) * (2.0 * h).min(1.0);
            }
        } else {
            let mut rng = self.rng.clone();
            let made = self.rotor.step(h, &mut rng);
            self.rng = rng;
            if made && self.dots.len() < ATP_DOTS {
                // A fresh ATP leaves the catalytic site the rotor just passed.
                let (c, s) = gamma_dir(self.rotor.angle - 30.0);
                self.dots.push(Dot {
                    x: 0.55 * c,
                    y: 0.55 * s,
                    fly: 1.5,
                    vx: 0.5 * c,
                    vy: 0.5 * s,
                });
            }
        }
        for d in &mut self.dots {
            if d.fly > 0.0 {
                d.fly -= h;
                d.x += d.vx * h;
                d.y += d.vy * h;
            } else {
                d.x += gaussian(&mut self.rng, 0.0, 0.02);
                d.y += gaussian(&mut self.rng, 0.0, 0.02);
            }
            d.x = (d.x + 1.0).rem_euclid(2.0) - 1.0;
            d.y = (d.y + 1.0).rem_euclid(2.0) - 1.0;
        }
    }

    fn draw_kinesin(&mut self) {
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()) - 2.0);
        let s = self.scale();
        let track_y = ph * 0.66;
        let to_px = |x: f64, y: f64, cam: f64| (pw / 2.0 + (x - cam) * s, track_y + y * s);
        let cam = self.cam;
        // ATP molecules drifting in solution.
        for d in &self.dots {
            let (x, y) = (pw * (d.x + 1.0) / 2.0, ph * (d.y + 1.0) / 2.0);
            if (y - track_y).abs() > 7.0 * s {
                self.pixels
                    .set(x as i32, y as i32, Ink::new(Role::Ramp(Ramp::Sulfur), 150));
            }
        }
        // Microtubule: a dim tube body (lit from above) so the lattice has no black gaps,
        // then three protofilaments of αβ-tubulin (4 nm beads, α darker, β paler), each
        // row shifted along the axis as in the lattice's helical pitch, the far rows
        // dimmer; value noise gives each dimer its own shade, so the texture moves with
        // the track. Beads are antialiased and low in contrast: a soft, woven surface.
        let tube = Role::Ramp(Ramp::Starlight);
        let (y_top, y_bot) = (to_px(0.0, 2.4, cam).1, to_px(0.0, 14.6, cam).1);
        for py in y_top.floor() as i32..=y_bot.ceil() as i32 {
            let fy = f64::from(py) + 0.5;
            let cover = ((fy - y_top).min(y_bot - fy) + 0.5).clamp(0.0, 1.0);
            let t = ((fy - y_top) / (y_bot - y_top).max(1.0)).clamp(0.0, 1.0);
            let lvl = cover * (26.0 + 30.0 * (1.0 - t));
            for px in 0..i32::from(ps.width()) {
                self.pixels.set(px, py, Ink::new(tube, ink::level(lvl)));
            }
        }
        let first = ((cam - pw / 2.0 / s) / 4.0).floor() as i64 - 3;
        let last = ((cam + pw / 2.0 / s) / 4.0).ceil() as i64 + 3;
        for (row, (y, shift, depth)) in [(12.5, 2.6, 0.62), (8.5, 1.3, 0.8), (4.5, 0.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            for k in first..=last {
                let x = k as f64 * 4.0 + 2.0 + shift;
                let beta = k.rem_euclid(2) == 0;
                let grain = sciart_core::rng::value_noise(11 + row as u64, x, 0.0, 9.0);
                let lvl = depth * (if beta { 132.0 } else { 100.0 }) * (0.88 + 0.24 * grain);
                soft_disc(&mut self.pixels, to_px(x, y, cam), 2.1 * s, tube, |l, d| {
                    lvl * (0.62 + 0.38 * l) * (1.0 - 0.25 * d * d)
                });
            }
        }
        // Footprints: each site a head bound to glows faintly and fades, so the walk
        // leaves a trail of 8 nm steps along the track.
        for &(x, at) in &self.trail[..self.trail_len] {
            let age = (self.time - at) / TRAIL_S;
            if (0.0..1.0).contains(&age) {
                let glow = 60.0 + 110.0 * (1.0 - age).powi(2);
                soft_disc(
                    &mut self.pixels,
                    to_px(x, 3.4, cam),
                    1.0 * s,
                    Role::Ramp(Ramp::Aurora),
                    |_, d| glow * (1.0 - 0.4 * d),
                );
            }
        }
        // Cargo, tether (coiled coil) and heads.
        let k = self.kinesin;
        let heads = [k.head(0), k.head(1)];
        let hip = (
            0.5 * (heads[0][0] + heads[1][0]),
            0.5 * (heads[0][1] + heads[1][1]) - 6.5,
        );
        let (cx, cy) = to_px(self.cargo[0], self.cargo[1], cam);
        // The cargo: a vesicle. A lipid bilayer (a bright, cool membrane), a dark lumen
        // speckled with its cargo molecules, and a soft highlight at the upper left.
        let r = CARGO_R * s;
        raster::disc_pixels((cx, cy), r, |x, y, cover| {
            let (ox, oy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
            let d = ox.hypot(oy) / r;
            let light = 0.5 + 0.5 * (-(ox + oy) / (1.5 * r)).clamp(-1.0, 1.0);
            let bump = |c: f64, w: f64| (-((d - c) / w).powi(2)).exp();
            let leaflet = bump(0.91, 0.075);
            if leaflet > 0.3 {
                let lvl = (80.0 + 90.0 * light) * leaflet.min(1.0);
                blend(&mut self.pixels, x, y, cover, Role::Ramp(Ramp::Ice), lvl);
            } else {
                // Two octaves of soft noise: a grainy, translucent lumen.
                let fine = sciart_core::rng::value_noise(5, ox / s, oy / s, 0.8);
                let coarse = sciart_core::rng::value_noise(6, ox / s, oy / s, 2.6);
                let granule = ease::smoothstep((0.6 * fine + 0.4 * coarse - 0.35) / 0.4);
                let sheen = (-(((ox / r + 0.4).powi(2) + (oy / r + 0.4).powi(2)) / 0.03)).exp();
                let lvl =
                    22.0 + 26.0 * light + 50.0 * granule * (0.6 + 0.4 * light) + 110.0 * sheen;
                blend(&mut self.pixels, x, y, cover, tube, lvl);
            }
        });
        let tether = Ink::new(Role::Ramp(Ramp::Aurora), 130);
        let hip_px = to_px(hip.0, hip.1, cam);
        soft_segment(
            &mut self.pixels,
            hip_px,
            (cx + 0.55 * r, cy + 0.72 * r),
            0.45,
            tether,
        );
        for (i, h) in heads.iter().enumerate() {
            let bound = k.free.is_none_or(|(f, _)| f != i);
            let (x, y) = to_px(h[0], h[1], cam);
            soft_segment(&mut self.pixels, hip_px, (x, y), 0.45, tether);
            let top = if bound { 235.0 } else { 185.0 };
            soft_disc(
                &mut self.pixels,
                (x, y),
                4.0 * s,
                Role::Ramp(Ramp::Aurora),
                |l, _| top * (0.55 + 0.45 * l),
            );
        }
    }

    fn draw_synthase(&mut self) {
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()) - 2.0);
        let c = (pw / 2.0, ph / 2.0);
        let r = (ph * 0.22).min(pw * 0.2);
        // The α₃β₃ ring.
        for j in 0..6 {
            let a = f64::from(j) * PI / 3.0 + PI / 6.0;
            let (s, co) = a.sin_cos_det();
            let beta = j % 2 == 0;
            let lvl = if beta { 175.0 } else { 120.0 };
            soft_disc(
                &mut self.pixels,
                (c.0 + r * co, c.1 + r * s),
                r * 0.52,
                Role::Ramp(Ramp::Starlight),
                |l, _| lvl * (0.5 + 0.5 * l),
            );
        }
        // γ and the actin filament stuck to it (Noji et al. 1997), turning clockwise as
        // seen from the membrane side: the synthesis sense.
        let (gx, gy) = gamma_dir(self.rotor.shown());
        let rotor = Ink::new(Role::Ramp(Ramp::Plasma), 150);
        soft_disc(
            &mut self.pixels,
            c,
            r * 0.3,
            Role::Ramp(Ramp::Plasma),
            |l, _| 90.0 + 90.0 * l,
        );
        let tip = (c.0 + 2.6 * r * gx, c.1 + 2.6 * r * gy);
        // While γ turns, a fading ghost of the filament sweeps behind it (motion blur).
        if let Some(t) = self.rotor.turning {
            let (bx, by) = gamma_dir(self.rotor.shown() - 25.0 * (1.0 - t / TURN_S));
            let ghost = Ink::new(Role::Ramp(Ramp::Plasma), 70);
            soft_segment(
                &mut self.pixels,
                c,
                (c.0 + 2.6 * r * bx, c.1 + 2.6 * r * by),
                0.8,
                ghost,
            );
        }
        soft_segment(&mut self.pixels, c, tip, 1.0, rotor);
        // Tick marks every 40°, the substep positions.
        for k in 0..9 {
            let (dx, dy) = gamma_dir(f64::from(k) * 40.0);
            let lvl = if k % 3 == 0 { 120 } else { 60 };
            self.pixels.set(
                (c.0 + 2.9 * r * dx) as i32,
                (c.1 + 2.9 * r * dy) as i32,
                Ink::new(Role::Ramp(Ramp::Starlight), lvl),
            );
        }
        for d in &self.dots {
            let (x, y) = (c.0 + d.x * pw * 0.5, c.1 + d.y * ph * 0.5);
            self.pixels
                .set(x as i32, y as i32, Ink::new(Role::Ramp(Ramp::Sulfur), 190));
        }
    }

    fn rasterise(&mut self) {
        self.pixels.clear();
        if self.act == KINESIN {
            self.draw_kinesin();
        } else {
            self.draw_synthase();
        }
    }
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let lit = |i: Ink| i.role != Role::Background;
    let ink = match (lit(top), lit(bottom)) {
        (false, false) => return None,
        (true, false) => top,
        (false, true) => bottom,
        (true, true) => {
            if top.level >= bottom.level {
                top
            } else {
                bottom
            }
        }
    };
    let ch = match ink.role {
        Role::Ramp(Ramp::Aurora) if ink.level > 150 => '@',
        Role::Ramp(Ramp::Aurora) => '|',
        Role::Ramp(Ramp::Sulfur) => '*',
        Role::Ramp(Ramp::Plasma) => '#',
        _ if ink.level > 110 => 'o',
        _ => '.',
    };
    Some((ch, ink))
}

impl Generator for Motors {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.local()).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        self.pixels.resize(size);
        if self.dots.is_empty() && self.time == 0.0 {
            self.rng = SplitMix64::from_rng(&mut ctx.rng);
            self.scatter_dots(Self::dots_for(atp_at(0.0)));
        }
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.owed += dt / H;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        for _ in 0..n as u32 {
            self.time += H;
            self.step_model(H);
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        let h = i32::from(self.cells.height());
        if h < 4 {
            return;
        }
        let mut label = StackText::<32>::new();
        if self.act == KINESIN {
            let atp = atp_at(self.local());
            let k = rate(atp);
            let _ = match self.readout_room {
                24.. => write!(label, "ATP {atp:>4.0} uM  {k:>3.0} steps/s"),
                9.. => write!(label, "ATP {atp:>4.0}"),
                _ => return,
            };
        } else {
            let turns = self.rotor.turns;
            let _ = match self.readout_room {
                16.. => write!(label, "ATP made {turns:>4}"),
                5.. => write!(label, "{turns:>4}"),
                _ => return,
            };
        }
        draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kin_a1_mean_dwell_matches_michaelis_menten() {
        let mut rng = SplitMix64::new(1);
        for atp in [1000.0, 20.0] {
            let k = rate(atp);
            let mean = (0..2000).map(|_| dwell(&mut rng, k)).sum::<f64>() / 2000.0;
            assert!(
                (mean * k - 1.0).abs() < 0.1,
                "[ATP] {atp}: mean {mean}, 1/k {}",
                1.0 / k
            );
        }
        assert!(rate(1000.0) > 2.0 * rate(20.0));
    }

    #[test]
    fn kin_a2_steps_are_8_nm_and_hand_over_hand() {
        let mut rng = SplitMix64::new(2);
        let mut k = Kinesin::new();
        let (mut last_centre, mut last_steps, mut movers) = (k.centre(), 0, Vec::new());
        for _ in 0..(240 * 60) {
            k.step(1000.0, &mut rng);
            if k.steps != last_steps {
                assert_eq!(k.centre() - last_centre, SITE_NM, "one 8 nm step");
                assert_eq!(
                    (k.site[0] - k.site[1]).abs(),
                    1,
                    "heads on neighbouring sites"
                );
                movers.push(k.last_moved.unwrap_or(9));
                last_centre = k.centre();
                last_steps = k.steps;
            }
        }
        assert!(movers.len() >= 20, "{} steps", movers.len());
        assert!(
            movers.windows(2).all(|w| w[0] != w[1]),
            "heads alternate: {movers:?}"
        );
    }

    #[test]
    fn kin_a3_rotor_turns_forward_in_80_40_substeps() {
        let mut rng = SplitMix64::new(3);
        let mut r = Rotor::new();
        let (mut shown, mut committed) = (r.shown(), vec![r.angle]);
        for _ in 0..(240 * 30) {
            r.step(H, &mut rng);
            assert!(r.shown() >= shown, "never turns back");
            shown = r.shown();
            if *committed.last().unwrap_or(&0.0) != r.angle {
                committed.push(r.angle);
            }
        }
        let steps: Vec<f64> = committed.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.len() >= 6);
        for (i, s) in steps.iter().enumerate() {
            assert_eq!(*s, if i % 2 == 0 { 80.0 } else { 40.0 });
        }
        assert_eq!(r.turns, (committed.len() as u64 - 1) / 2);
    }

    #[test]
    fn kin_a4_bound_jiggle_has_no_drift() {
        let mut rng = SplitMix64::new(4);
        let (mut x, mut sum) = (0.0, 0.0);
        let n = 200_000;
        for _ in 0..n {
            x = langevin(x, 0.0, WELL_K, WELL_D * 0.02, H, &mut rng);
            sum += x;
        }
        assert!(
            (sum / f64::from(n)).abs() < 0.02,
            "mean {}",
            sum / f64::from(n)
        );
    }

    #[test]
    fn kin_a6_gamma_turns_clockwise_seen_from_the_membrane() {
        // Screen y points down, so a positive cross product of successive directions is
        // a clockwise turn on screen (viewed from the Fo side: the synthesis sense).
        for k in 0..9 {
            let (a, b) = (
                gamma_dir(f64::from(k) * 40.0),
                gamma_dir(f64::from(k + 1) * 40.0),
            );
            assert!(a.0 * b.1 - a.1 * b.0 > 0.5, "clockwise at {}°", k * 40);
        }
        assert!(gamma_dir(0.0).1 < -0.99, "0° points up");
    }

    #[test]
    fn kin_a7_each_step_leaves_a_footprint_at_its_site() {
        let mut ctx = Ctx::new(7, Size::new(120, 40));
        let mut g = Motors::default();
        g.resize(Size::new(120, 40), &mut ctx);
        let mut seen = 0;
        while g.time < 8.0 {
            let before = g.kinesin.steps;
            g.update(1.0 / 60.0, &mut ctx);
            let new = (g.kinesin.steps - before) as usize;
            seen += new;
            if new > 0 {
                let last = g.trail[(g.trail_next + TRAIL - 1) % TRAIL];
                let i = g.kinesin.last_moved.unwrap_or(0);
                assert_eq!(last.0, g.kinesin.site[i] as f64 * SITE_NM);
            }
        }
        assert!(seen >= 3, "{seen} steps");
        assert_eq!(g.trail_len, seen.min(TRAIL));
    }

    #[test]
    fn kin_a5_captions_follow_the_acts() {
        let mut ctx = Ctx::new(5, Size::new(80, 24));
        let mut g = Motors::default();
        g.resize(Size::new(80, 24), &mut ctx);
        assert_eq!(g.caption(), Some("kinesin walks"));
        while g.time < KINESIN_S + 1.0 {
            g.update(0.25, &mut ctx);
        }
        assert_eq!(g.caption(), Some("synthase turns"));
        while g.time < KINESIN_S + SYNTHASE_S + 1.0 {
            g.update(0.25, &mut ctx);
        }
        assert_eq!(g.caption(), Some("kinesin walks"));
    }
}
