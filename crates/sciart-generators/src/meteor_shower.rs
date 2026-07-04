//! The Perseids at their peak (SPEC-002 `meteor_shower`): meteors flash all over a dark
//! summer sky, yet every streak points back to one radiant; bright ones leave green
//! glowing trains that linger and twist in the upper-atmosphere wind.
//!
//! Every meteoroid of the stream moves along one velocity vector, so their straight paths
//! through the 110–75 km ablation layer are parallel lines in space. The camera is
//! gnomonic (a rectilinear lens), which maps straight lines to straight lines and sends
//! parallel lines to one vanishing point: the radiant (MET-A1). A path seen close to the
//! radiant is foreshortened, so meteors there are short (MET-A2).

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::{self, Glow, Tone};
use sciart_core::timeline::ease;

use crate::shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "meteor_shower",
    category: Category::EarthSky,
    description: "Perseid meteors fan out of one radiant; bright ones leave twisting green trains",
    title: "Meteor shower · comet dust burns",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(MeteorShower::default())
}

/// Pool capacities (MET-A5).
const MAX_METEORS: usize = 48;
const MAX_TRAINS: usize = 12;
/// Population index `r`: each magnitude step fainter holds `r` times more meteors (MET-A3).
const POP_INDEX: f64 = 2.2;
/// The faintest meteor drawn; the brightest allowed (a fireball); trains from meteors at
/// least this bright.
const MAG_FAINT: f64 = 3.5;
const MAG_BRIGHT: f64 = -4.0;
const TRAIN_MAG: f64 = 0.6;
/// Mean meteors per second (far above a real ZHR of ~100/h), slowly swelling and ebbing.
const RATE: f64 = 4.0;
const RATE_SWELL: (f64, f64) = (0.45, 37.0);
/// Perseid entry speed (km/s), sped up ×2.2 (M14; was ×3.5) so a (lengthened) streak crosses in about
/// half a second, as the eye sees one.
const SPEED: f64 = 59.0 * 2.2;
/// Paths are drawn this much longer than the ablation layer allows, so a streak spans a
/// readable stretch of a terminal sky (stylised).
const STRETCH: f64 = 5.0;
/// Ablation heights of Perseids (km): begin (brighter meteors start higher), end
/// (brighter ones burn deeper).
const BEGIN_KM: (f64, f64) = (105.0, 125.0);
const END_KM: (f64, f64) = (80.0, 95.0);
/// Share of meteors that start within `NEAR_RADIANT` degrees of the radiant: short,
/// foreshortened streaks that make it visible as the point they all fan out of.
const NEAR_SHARE: f64 = 0.3;
const NEAR_RADIANT: f64 = 22.0;
/// The radiant's mark: a faint ring of dots (radius in dots) and its level.
const RING: (f64, u8) = (3.5, 80);
/// M14 (ADR-0017): the radiant's ring and label are not drawn.
const SHOW_RADIANT: bool = false;
/// The streak's glow: green light stroked along the tail into the trains' layer.
const WAKE_GLOW: f32 = 0.22;
/// M14 (ADR-0017): the half-block wake glow is off; the 1-px braille streak carries it.
const SHOW_WAKE: bool = false;
/// M14: a meteor trail's level at the head end (the head itself is white 255).
const TRAIL_TOP: f64 = 200.0;
/// M14: persistent trains are not drawn (they read as blocky smears at cell size); the
/// model still ages them (MET-A4, MET-A8).
const SHOW_TRAINS: bool = false;
/// M14: stars are neutral `Starlight` at most this level, so meteors are the brightest.
const STAR_MAX: f64 = 110.0;
/// Framing: half field of view across (deg), the horizon's row as a share of the height,
/// the radiant's height below the top edge (share of the height) and its offset from the
/// centre (share of the width).
const HALF_FOV: f64 = 31.0;
const HORIZON_ROW: f64 = 0.86;
const RADIANT_ROW: f64 = 0.14;
const RADIANT_OFF: (f64, f64) = (0.1, 0.25);
/// A streak is drawn in braille: the head and a tail behind it covering this share of the
/// path (more for bright meteors), tapering by thinning toward its end.
const TAIL: (f64, f64) = (0.4, 0.75);
/// Recent tracks: how many are kept, their fade time (s) and peak level (drawn in two dim
/// grey steps, `TRACK`): a faint composite as in a stacked photograph of a shower: at any
/// moment several tracks show, all pointing back to the radiant.
const MAX_TRACKS: usize = 32;
const MEMORY_TAU: f64 = 3.5;
const MEMORY_PEAK: f64 = 70.0;
const TRACK: [Ink; 2] = [Ink::new(Role::Structure, 0), Ink::new(Role::Structure, 60)];
/// The white-hot head of a streak.
const HEAD: Ink = Ink::new(Role::Star, 255);
/// A train's one faint green step.
const TRAIN_LEVEL: u8 = 30;
/// A meteor's light is `flux^LIGHT_POW` (relative to the faintest drawn).
const LIGHT_POW: f64 = 0.55;
/// Trains widen with this time constant (s).
const TRAIN_SPREAD: f64 = 0.8;
/// Train: points along the track, life range (s), fade time scale, and the wind: two
/// layers' amplitude (km/s, time-lapsed) and vertical wavelength (km).
const TRAIN_POINTS: usize = 24;
const TRAIN_LIFE: (f64, f64) = (1.8, 3.2);
const WIND: [(f64, f64); 2] = [(3.5, 7.0), (2.2, 12.5)];
/// The trains' tone (knee, white); below `TRAIN_FLOOR` a train is not drawn, and for
/// `DITHER` levels above it, dithered into the dark.
const TRAIN_TONE: (f32, f32) = (0.1, 3.0);
const DITHER: u8 = 28;
const TRAIN_FLOOR: u8 = 22;
/// Trains are drawn at this share of their toned level: faint, so the streaks are the hero.
const TRAIN_DIM: (u16, u16) = (1, 3);
/// The head's share of a streak, drawn white-hot (`Star` 255); behind it the wake is
/// green (`Particle`) down to `WAKE_SPLIT` of the tail, then yellow (`Sun`) to its end,
/// each in two flat steps (ADR-0018).
const HOT: f64 = 0.88;
const WAKE_SPLIT: f64 = 0.45;
/// Stars: per cell, spectral mix, and the brightest stars' level.
const STAR_DENSITY: f64 = 0.07;
const CLASSES: [(f64, f64, f64); 5] = [
    (0.20, 3300.0, 4200.0),
    (0.50, 4300.0, 5400.0),
    (0.65, 5500.0, 6500.0),
    (0.88, 7000.0, 10_000.0),
    (1.00, 11_000.0, 22_000.0),
];
/// The far shore: a spruce ridge.
const LAND: Terrain = Terrain {
    ridge: (0.9, 2.6),
    trees: Some((1.4, 5.2)),
    gap: (0.7, 2.9),
    clearing: 0.4,
};

/// A stable value in `[0, 1)` for a star at dot `(x, y)` (its brightness).
fn hash_level(x: f64, y: f64) -> f64 {
    sciart_core::rng::hash01(sciart_core::rng::cell_key(x as i32, y as i32))
}

/// A magnitude for a uniform `u ∈ (0, 1]`: `P(m ≤ M) = r^{M − MAG_FAINT}` (MET-A3),
/// clamped at a fireball.
pub(crate) fn sample_mag(u: f64) -> f64 {
    (MAG_FAINT + u.max(1e-12).ln() / POP_INDEX.ln()).max(MAG_BRIGHT)
}

/// Relative flux of magnitude `m` against the faintest drawn.
fn flux(m: f64) -> f64 {
    10f64.powf(-0.4 * (m - MAG_FAINT))
}

/// Light curve along the path, `u ∈ [0, 1]`: rises, peaks at two thirds, then burns out
/// (`√(27/4 · u²(1 − u))`, 1 at the peak).
pub(crate) fn light_curve(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    (6.75 * u * u * (1.0 - u)).sqrt()
}

/// Horizontal wind (km/s, east and north) at height `z` km: two sheared layers.
pub(crate) fn wind(z: f64, phases: [f64; 4]) -> (f64, f64) {
    let mut w = (0.0, 0.0);
    for (i, &(amp, lambda)) in WIND.iter().enumerate() {
        let s = amp * (TAU * z / lambda + phases[2 * i]).sin_det();
        let (sa, ca) = phases[2 * i + 1].sin_cos_det();
        w.0 += s * ca;
        w.1 += s * sa;
    }
    w
}

/// Train brightness at `age` of a train living `life` seconds: an exponential fade whose
/// last second eases to zero, so it never snaps off (MET-A4).
pub(crate) fn train_fade(age: f64, life: f64) -> f64 {
    if !(0.0..life).contains(&age) {
        return 0.0;
    }
    (-2.5 * age / life).exp() * ease::smoothstep(life - age)
}

/// Stroke the dot-space segment `a → b` into `glow` as a thin anti-aliased thread: bilinear
/// splats every half pixel (`Glow::splat_bilinear`), so it is one to two pixels wide
/// (a pixel is a cell across) with no stair steps, `scale` per pixel of length.
fn thin_stroke(glow: &mut Glow, a: (f64, f64), b: (f64, f64), scale: f32) {
    // Dot `d` sits in the pixel whose centre is `(d − 0.5) / 2`.
    let (a, b) = (
        ((a.0 - 0.5) / 2.0, (a.1 - 0.5) / 2.0),
        ((b.0 - 0.5) / 2.0, (b.1 - 0.5) / 2.0),
    );
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    if !len.is_finite() {
        return;
    }
    let n = (2.0 * len).ceil().clamp(1.0, 4096.0) as usize;
    let w = scale * len.max(0.5) as f32 / n as f32;
    for k in 0..n {
        let u = (k as f64 + 0.5) / n as f64;
        glow.splat_bilinear(a.0 + u * (b.0 - a.0), a.1 + u * (b.1 - a.1), w);
    }
}

/// A rectilinear camera facing north, pitched up, in pixel units.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Lens {
    f: f64,
    centre: (f64, f64),
    /// sin and cos of the pitch.
    pitch: (f64, f64),
}

impl Lens {
    fn new(px_w: f64, px_h: f64) -> Self {
        let f = (0.5 * px_w / HALF_FOV.to_radians().tan()).max(1.0);
        let centre = (0.5 * px_w, 0.5 * px_h);
        let pitch = ((HORIZON_ROW - 0.5) * px_h / f).atan();
        Self {
            f,
            centre,
            pitch: pitch.sin_cos_det(),
        }
    }

    /// Pixel position of world direction `v` (east, north, up), if in front of the lens.
    pub(crate) fn project(&self, v: [f64; 3]) -> Option<(f64, f64)> {
        let (s, c) = self.pitch;
        let fwd = v[1] * c + v[2] * s;
        let up = -v[1] * s + v[2] * c;
        (fwd > 1e-6).then(|| {
            (
                self.centre.0 + self.f * v[0] / fwd,
                self.centre.1 - self.f * up / fwd,
            )
        })
    }

    /// The world direction (unit) seen at pixel position `(x, y)`.
    fn unproject(&self, x: f64, y: f64) -> [f64; 3] {
        let (s, c) = self.pitch;
        let (cx, cy) = ((x - self.centre.0) / self.f, (self.centre.1 - y) / self.f);
        // Camera axes: right = east, up = (0, −s, c), forward = (0, c, s).
        let v = [cx, c - cy * s, s + cy * c];
        geom::v3::unit(v)
    }

    /// The horizon's row (a great circle: a straight row in a gnomonic view).
    fn horizon(&self) -> f64 {
        let (s, c) = self.pitch;
        self.centre.1 + self.f * s / c
    }

    fn px_per_deg(&self) -> f64 {
        self.f * PI / 180.0
    }
}

/// A recent meteor's track (px) and its light, fading as it ages.
#[derive(Debug, Clone, Copy)]
struct Track {
    a: (f64, f64),
    b: (f64, f64),
    light: f64,
    age: f64,
}

#[derive(Debug, Clone, Copy)]
struct Meteor {
    /// Start of the luminous path (km, observer at the origin, z up) and its direction.
    start: [f64; 3],
    dir: [f64; 3],
    /// Path length (km), time flown (s), magnitude.
    len: f64,
    age: f64,
    mag: f64,
    /// Head position (px) last step.
    last: Option<(f64, f64)>,
}

impl Meteor {
    fn point(&self, s: f64) -> [f64; 3] {
        geom::v3::axpy(s, self.dir, self.start)
    }

    fn progress(&self) -> f64 {
        (SPEED * self.age / self.len.max(1e-9)).min(1.0)
    }
}

#[derive(Debug, Clone, Copy)]
struct Train {
    pts: [[f64; 3]; TRAIN_POINTS],
    /// Relative brightness along the train, age and life (s), and wind phases.
    amp: f64,
    age: f64,
    life: f64,
    wind: [f64; 4],
}

#[derive(Debug, Default)]
pub struct MeteorShower {
    lens: Lens,
    radiant: [f64; 3],
    /// Canvas size in cells.
    cells: (u16, u16),
    t: f64,
    next_at: f64,
    meteors: ParticlePool<Meteor>,
    trains: ParticlePool<Train>,
    /// Recent tracks, drawn faintly (the stacked-photograph composite).
    tracks: ParticlePool<Track>,
    /// The heads' soft glare, redrawn each step, under the braille streaks.
    /// The trains' light, redrawn every step as they drift.
    train_light: Glow,
    train_tone: Tone,
    /// Fixed sky: `Star` level per pixel (empty: the sky is black), 0 = dark.
    sky: Vec<u8>,
    /// First pixel row of land per pixel column: light at or below it is hidden.
    land: Vec<u16>,
    picture: Pixels,
    stars: Dots,
    /// Streaks and recent tracks, in braille: thin and sub-cell.
    streaks: Dots,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl MeteorShower {
    /// A new meteor of the stream, starting in view.
    fn spawn(&mut self, rng: &mut impl Rng) {
        let ps = self.picture.pixel_size();
        let (w, h) = (f64::from(ps.width()), f64::from(ps.height()));
        let horizon = self.lens.horizon().min(h);
        // A start seen anywhere above the land (plus a margin off the frame's sides).
        let (mut x, mut y) = (
            rng.random_range(-0.1 * w..1.1 * w),
            rng.random_range(-0.1 * h..(horizon - 0.06 * h).max(1.0)),
        );
        if rng.random_bool(NEAR_SHARE)
            && let Some(r) = self.lens.project(self.radiant)
        {
            // Close to the radiant, in any direction from it.
            let (s, c) = rng.random_range(0.0..TAU).sin_cos_det();
            let d = self.lens.px_per_deg() * NEAR_RADIANT * rng.random::<f64>().sqrt();
            (x, y) = (r.0 + d * c, (r.1 + d * s).min(horizon - 0.06 * h));
        }
        let d = self.lens.unproject(x, y);
        if d[2] < 0.05 {
            return;
        }
        let mag = sample_mag(1.0 - rng.random::<f64>());
        let bright = ((MAG_FAINT - mag) / (MAG_FAINT - MAG_BRIGHT)).clamp(0.0, 1.0);
        let begin =
            BEGIN_KM.0 + (BEGIN_KM.1 - BEGIN_KM.0) * (0.5 * bright + 0.5 * rng.random::<f64>());
        let end = END_KM.1 - (END_KM.1 - END_KM.0) * (0.5 * bright + 0.5 * rng.random::<f64>());
        let dir = geom::v3::scale(self.radiant, -1.0);
        // The path drops `begin − end` km at the stream's descent rate.
        let len = STRETCH * (begin - end) / (-dir[2]).max(0.05);
        let start = geom::v3::scale(d, begin / d[2]);
        self.meteors.spawn(Meteor {
            start,
            dir,
            len,
            age: 0.0,
            mag,
            last: None,
        });
    }

    fn spawn_train(&mut self, m: &Meteor, rng: &mut impl Rng) {
        let pts = std::array::from_fn(|i| {
            let u = 0.35 + 0.45 * i as f64 / (TRAIN_POINTS - 1) as f64;
            m.point(u * m.len)
        });
        let life = rng.random_range(TRAIN_LIFE.0..TRAIN_LIFE.1)
            * (1.0 + 0.15 * (TRAIN_MAG - m.mag).max(0.0)).min(1.6);
        self.trains.spawn(Train {
            pts,
            amp: 0.5 * flux(m.mag).powf(0.25),
            age: 0.0,
            life: life.min(TRAIN_LIFE.1 * 1.4),
            wind: std::array::from_fn(|_| rng.random_range(0.0..TAU)),
        });
    }

    fn fly(&mut self, dt: f64, rng: &mut impl Rng) {
        let mut ended: [Option<Meteor>; MAX_METEORS] = [None; MAX_METEORS];
        let mut n_ended = 0;
        let lens = self.lens;
        self.meteors.retain(|m| {
            m.age += dt;
            let u = m.progress();
            let head = lens.project(m.point(u * m.len));
            m.last = head;
            if u >= 1.0 {
                ended[n_ended] = Some(*m);
                n_ended += 1;
                return false;
            }
            true
        });
        for m in ended[..n_ended].iter().flatten() {
            if m.mag <= TRAIN_MAG {
                self.spawn_train(m, rng);
            }
            // Its whole track joins the composite (the oldest makes room).
            if let (Some(a), Some(b)) = (lens.project(m.point(0.05 * m.len)), m.last) {
                if self.tracks.len() == MAX_TRACKS {
                    let oldest = self.tracks.iter().map(|t| t.age).fold(0.0, f64::max);
                    self.tracks.retain(|t| t.age < oldest);
                }
                self.tracks.spawn(Track {
                    a,
                    b,
                    light: flux(m.mag).powf(LIGHT_POW),
                    age: 0.0,
                });
            }
        }
        self.tracks.retain(|t| {
            t.age += dt;
            MEMORY_PEAK * (0.4 + t.light / 3.0).min(1.0) * (-t.age / MEMORY_TAU).exp() >= 14.0
        });
        self.rasterise();
    }

    /// Draw the streaks and tracks in braille.
    fn rasterise(&mut self) {
        self.streaks.clear();
        // Nothing is drawn over the land: dots at or below its top are skipped.
        let land = &self.land;
        let above = |x: f64, y: f64| {
            let px = ((x - 0.5) / 2.0).round();
            px >= 0.0
                && land
                    .get(px as usize)
                    .is_some_and(|&top| (y - 0.5) / 2.0 < f64::from(top) - 0.5)
        };
        let dot = |p: (f64, f64)| (2.0 * p.0 + 0.5, 2.0 * p.1 + 0.5);
        for t in self.tracks.iter() {
            let level = MEMORY_PEAK * (0.4 + t.light / 3.0).min(1.0) * (-t.age / MEMORY_TAU).exp();
            let (a, b) = (dot(t.a), dot(t.b));
            let ink = TRACK[usize::from(level >= 0.5 * MEMORY_PEAK)];
            let n = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).clamp(1, 2048);
            for i in 0..=n {
                let f = i as f64 / n as f64;
                let (x, y) = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
                if above(x, y) {
                    self.streaks.plot(x, y, ink);
                }
            }
        }
        let lens = self.lens;
        for m in self.meteors.iter() {
            let u = m.progress();
            let light = flux(m.mag).powf(LIGHT_POW) * light_curve(u);
            let bright = ((MAG_FAINT - m.mag) / (MAG_FAINT - MAG_BRIGHT)).clamp(0.0, 1.0);
            let tail = (u - (TAIL.0 + (TAIL.1 - TAIL.0) * bright)).max(0.0);
            let (Some(a), Some(b)) = (
                lens.project(m.point(tail * m.len)),
                lens.project(m.point(u * m.len)),
            ) else {
                continue;
            };
            let (a, b) = (dot(a), dot(b));
            // A soft green glow a cell or two wide along the bright end of the tail.
            let mid = (a.0 + (b.0 - a.0) * 0.45, a.1 + (b.1 - a.1) * 0.45);
            if SHOW_WAKE && above(mid.0, mid.1) && above(b.0, b.1) {
                let g = WAKE_GLOW * (light as f32).min(3.0);
                thin_stroke(&mut self.train_light, mid, b, g);
            }
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            let n = (len.ceil() as usize).clamp(1, 2048);
            // Brighter meteors keep a longer solid stretch behind the head.
            let solid = (0.72 + 0.25 * light).min(0.97);
            for i in 0..=n {
                let f = i as f64 / n as f64;
                let (x, y) = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
                // A tapering tail (M14, no dither): every dot drawn, its level fading
                // smoothly toward the tail's end.
                let keep = (f.powf(0.7) * solid + (1.0 - solid) * f * f).clamp(0.0, 1.0);
                // White-hot at the head, then the green of magnesium and oxygen, cooling to
                // the yellow of sodium and iron toward the tail's end.
                let ink = if f >= HOT {
                    HEAD
                } else {
                    // M14: a trail tapering from 200 at the head to black at its end,
                    // green (Mg, O) then the yellow of sodium toward the end.
                    let level = TRAIL_TOP * f.powf(1.2) * (0.5 + 0.5 * keep);
                    let role = if f >= WAKE_SPLIT {
                        Role::Particle
                    } else {
                        Role::Sun
                    };
                    Ink::new(role, if level >= 100.0 { 220 } else { 110 })
                };
                if !above(x, y)
                    || f.powf(1.2) * (0.5 + 0.5 * keep) * TRAIL_TOP < ink::MIN_VISIBLE_LEVEL
                {
                    continue;
                }
                self.streaks.plot(x, y, ink);
            }
            // The head: a bright meteor's is a 2 × 2 dot blaze.
            if !above(b.0, b.1) {
                continue;
            }
            let head = HEAD;
            self.streaks.plot(b.0, b.1, head);
            {
                for (dx, dy) in [(1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    self.streaks.plot(b.0 + dx, b.1 + dy, head);
                }
            }
        }
    }

    fn drift(&mut self, dt: f64) {
        self.trains.retain(|t| {
            t.age += dt;
            for p in &mut t.pts {
                let (u, v) = wind(p[2], t.wind);
                p[0] += u * dt;
                p[1] += v * dt;
            }
            t.age < t.life
        });
        self.train_light.clear();
        let lens = self.lens;
        for t in self.trains.iter() {
            let fade = train_fade(t.age, t.life);
            if fade <= 0.0 {
                continue;
            }
            // The train spreads as it ages: its light moves from a thin core to a wide halo.
            let spread = (1.0 - (-t.age / TRAIN_SPREAD).exp()) as f32;
            let scale = (t.amp * fade) as f32;
            let mut prev: Option<(f64, f64)> = None;
            for (i, p) in t.pts.iter().enumerate() {
                let q = lens.project(*p);
                // Ends taper: a train is brightest in its middle.
                let taper = (PI * i as f64 / (TRAIN_POINTS - 1) as f64)
                    .sin_det()
                    .max(0.15) as f32;
                if let (Some(a), Some(b)) = (prev, q) {
                    let (a, b) = (
                        (2.0 * a.0 + 0.5, 2.0 * a.1 + 0.5),
                        (2.0 * b.0 + 0.5, 2.0 * b.1 + 0.5),
                    );
                    // A faint thread a pixel or two wide, dimming as it spreads.
                    thin_stroke(
                        &mut self.train_light,
                        a,
                        b,
                        scale * taper * (1.0 - 0.4 * spread),
                    );
                }
                prev = q;
            }
        }
    }

    /// Composite sky, trains and streaks into the picture.
    fn compose(&mut self) {
        let ps = self.picture.pixel_size();
        let w = usize::from(ps.width());
        let tt = &self.train_tone;
        let trains = self.train_light.values();
        let (sky, land) = (&self.sky, &self.land);
        self.picture.shade(|x, y| {
            let i = y as usize * w + x as usize;
            // Below the land's top only the sky's anti-aliased edge shows (its level is
            // already weighted by how much of the pixel is open): trains and glare stop.
            if land.get(x as usize).is_none_or(|&top| y as u16 >= top) {
                return match sky.get(i).copied().unwrap_or(0) {
                    0 => Ink::BLANK,
                    l => Ink::new(Role::Star, l),
                };
            }
            let t = tt.level(trains.get(i).copied().unwrap_or(0.0));
            // M14: no dither; the faint end fades smoothly to black over `DITHER` levels.
            let u = (f64::from(t.saturating_sub(TRAIN_FLOOR)) / f64::from(DITHER)).min(1.0);
            let faded = (f64::from(t) * u * u * (3.0 - 2.0 * u)).round() as u16;
            // The head is braille only (its 2 × 2-dot blaze): no half-block glare.
            if SHOW_TRAINS && faded * TRAIN_DIM.0 / TRAIN_DIM.1 >= ink::MIN_VISIBLE_LEVEL as u16 {
                Ink::new(Role::Particle, TRAIN_LEVEL)
            } else {
                match sky.get(i).copied().unwrap_or(0) {
                    0 => Ink::BLANK,
                    l => Ink::new(Role::Star, l),
                }
            }
        });
    }

    /// The fixed sky: Milky Way and horizon airglow, and the stars.
    fn paint_sky(&mut self, rng: &mut impl Rng) {
        let ps = self.picture.pixel_size();
        let (w, h) = (usize::from(ps.width()), usize::from(ps.height()));
        let lens = self.lens;
        let deg = lens.px_per_deg();
        let horizon = lens.horizon();
        let skyline = Skyline::new(rng, w, deg, LAND);
        // The Milky Way rises steeply out of the horizon, a little left of the radiant.
        let band_x = rng.random_range(0.25..0.5) * w as f64;
        let tilt = rng.random_range(0.15..0.45);
        let clumps: [f64; 3] = std::array::from_fn(|_| rng.random_range(0.0..TAU));
        let dust_seed: u64 = rng.random();
        let band_w = 0.09 * w as f64;
        self.sky.clear();
        self.land.clear();
        self.land.extend((0..w).map(|x| {
            let top = horizon - skyline.height_at(x as f64 + 0.5);
            top.clamp(0.0, f64::from(u16::MAX)) as u16
        }));
        // M14 (ADR-0017): a black sky, no Milky Way or airglow haze; `sky` stays empty,
        // so there is no per-pixel pass at resize.
        let _ = (h, band_x, tilt, clumps, dust_seed, band_w, deg);
        // Stars, more of them in the Milky Way.
        let size = Size::new(self.cells.0, self.cells.1);
        let cells = size.area() as f64;
        let n = (STAR_DENSITY * cells) as usize;
        self.stars.resize(size);
        self.streaks.resize(size);
        let dsize = self.stars.dot_size();
        for _ in 0..n {
            let x = rng.random_range(0.0..f64::from(dsize.width()));
            let y = rng.random_range(0.0..f64::from(dsize.height()));
            let (px, py) = ((x - 0.5) / 2.0, (y - 0.5) / 2.0);
            let top = self.land.get(px.max(0.0) as usize).copied().unwrap_or(0);
            if py >= f64::from(top) - 0.5 {
                continue;
            }
            let k = ink::class_kelvin(&CLASSES, rng.random(), rng.random());
            // The draws are kept so the sky stays the same per seed; M14 draws a neutral
            // point at most `STAR_MAX`, no bright crosses.
            let _ = k;
            let bright = rng.random_bool(1.0 / 15.0);
            let level = if bright {
                STAR_MAX
            } else {
                60.0 + 40.0 * hash_level(x, y)
            };
            self.stars.plot(
                x,
                y,
                Ink::new(Role::Star, if level >= STAR_MAX { 200 } else { 50 }),
            );
        }
        // The radiant's mark: a faint ring of dots round the point every streak leads to.
        if let Some(r) = lens.project(self.radiant).filter(|_| SHOW_RADIANT) {
            let ink = Ink::new(Role::Accent, RING.1);
            for k in 0..16 {
                let (sn, cs) = (TAU * f64::from(k) / 16.0).sin_cos_det();
                self.stars
                    .plot(2.0 * r.0 + RING.0 * cs, 2.0 * r.1 + RING.0 * sn, ink);
            }
        }
    }
}

impl Generator for MeteorShower {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = (size.width(), size.height());
        let (px_w, px_h) = (f64::from(size.width()), 2.0 * f64::from(size.height()));
        self.lens = Lens::new(px_w, px_h);
        // The radiant, in frame: `RADIANT_ROW` down from the top, off to one side.
        let side = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
        let off = ctx.rng.random_range(RADIANT_OFF.0..RADIANT_OFF.1);
        self.radiant = self
            .lens
            .unproject((0.5 + side * off) * px_w, RADIANT_ROW * px_h);
        self.meteors = ParticlePool::with_capacity(MAX_METEORS);
        self.trains = ParticlePool::with_capacity(MAX_TRAINS);
        self.tracks = ParticlePool::with_capacity(MAX_TRACKS);
        self.train_light.resize(size);
        self.picture.resize(size);
        self.train_tone = Tone::asinh(TRAIN_TONE.0, TRAIN_TONE.1);
        self.paint_sky(&mut ctx.rng);
        // The shower is under way: a train from a fireball a moment ago, and the next
        // meteor due at once.
        self.t = 0.0;
        self.next_at = 0.25;
        // Its track is picked to lie well inside the frame, above the land.
        let (w, h) = (px_w, self.lens.horizon().min(px_h));
        // Twice the longest of a few candidate tracks well inside the frame: one for the
        // train, one for a meteor already flying, so the first second is never empty.
        let mut opening = None;
        for role in 0..2 {
            let mut best: Option<(f64, Meteor)> = None;
            for _ in 0..48 {
                self.meteors.clear();
                self.spawn(&mut ctx.rng);
                let Some(m) = self.meteors.iter().next().copied() else {
                    continue;
                };
                let ends = [0.35, 0.8].map(|u| self.lens.project(m.point(u * m.len)));
                if let [Some(a), Some(b)] = ends {
                    let framed = [a, b]
                        .iter()
                        .all(|&(x, y)| (0.0..w).contains(&x) && (0.0..0.85 * h).contains(&y));
                    let len = (b.0 - a.0).hypot(b.1 - a.1);
                    if framed && best.is_none_or(|(l, _)| len > l) {
                        best = Some((len, m));
                    }
                }
            }
            match (role, best) {
                (0, Some((_, mut m))) => {
                    m.mag = -1.5;
                    self.spawn_train(&m, &mut ctx.rng);
                    if let Some(t) = self.trains.iter_mut().next() {
                        t.age = 0.4;
                        t.life = TRAIN_LIFE.1;
                    }
                }
                (_, Some((_, mut m))) => {
                    m.mag = m.mag.min(1.0);
                    opening = Some(m);
                }
                _ => {}
            }
        }
        self.meteors.clear();
        if let Some(m) = opening {
            self.meteors.spawn(m);
        }
        self.drift(0.0);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        while self.t >= self.next_at {
            self.spawn(&mut ctx.rng);
            let rate = RATE * (1.0 + RATE_SWELL.0 * (TAU * self.t / RATE_SWELL.1).sin_det());
            self.next_at += -(1.0 - ctx.rng.random::<f64>()).ln() / rate.max(0.05);
        }
        // Trains first (they redraw their layer), then the meteors, whose tails add their
        // glow to it.
        self.drift(dt);
        self.fly(dt, &mut ctx.rng);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.compose();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let unicode = canvas.mode() == GlyphMode::Unicode;
        self.picture.blit(canvas, |_, _, top, bottom| {
            // ASCII: trains by brightness; the sky stays dark and streaks are drawn below.
            let pick = raster::stronger(top, bottom);
            (pick.role == Role::Particle)
                .then(|| raster::ascii_ramp_cell(top, bottom))
                .flatten()
        });
        self.stars.blit_over(canvas, &self.picture);
        // The radiant, named beside its ring.
        if let Some(r) = self.lens.project(self.radiant).filter(|_| SHOW_RADIANT) {
            let (x, y) = ((r.0 + RING.0 / 2.0 + 1.5) as i32, (r.1 / 2.0) as i32);
            if x + 9 < i32::from(canvas.size().width()) {
                draw::label(canvas, x, y, "radiant", Ink::new(Role::Text, 60));
            }
        }
        if unicode {
            self.streaks.blit_over(canvas, &self.picture);
            return;
        }
        // ASCII streaks: the last stretch of each head's path as `-/\|`, the head `*`.
        for m in self.meteors.iter() {
            let u = m.progress();
            let back = (u - 0.25).max(0.0);
            let (Some(a), Some(b)) = (
                self.lens.project(m.point(back * m.len)),
                self.lens.project(m.point(u * m.len)),
            ) else {
                continue;
            };
            let (a, b) = ((a.0, a.1 / 2.0), (b.0, b.1 / 2.0));
            let mut prev = (a.0 as i32, a.1 as i32);
            for (x, y) in geom::line(prev.0, prev.1, b.0 as i32, b.1 as i32) {
                let g = draw::step_glyph(x - prev.0, y - prev.1);
                if (x, y) != prev {
                    canvas.put(x, y, g, Ink::new(Role::Sun, 150));
                }
                prev = (x, y);
            }
            canvas.put(b.0 as i32, b.1 as i32, Glyph::ascii('*'), HEAD);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (MeteorShower, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = MeteorShower::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    /// Angle (deg) between the line through `a → b` (px) and the direction from `b` to
    /// the radiant's pixel.
    fn miss_deg(a: (f64, f64), b: (f64, f64), r: (f64, f64)) -> f64 {
        let (tx, ty) = (a.0 - b.0, a.1 - b.1);
        let (rx, ry) = (r.0 - b.0, r.1 - b.1);
        let cross = tx * ry - ty * rx;
        let dot = tx * rx + ty * ry;
        cross.atan2(dot).abs().to_degrees()
    }

    #[test]
    fn met_a1_every_track_points_back_to_the_radiant() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let r = g.lens.project(g.radiant).expect("radiant in front");
        let mut checked = 0;
        for _ in 0..200 {
            g.spawn(&mut ctx.rng);
            for m in g.meteors.iter() {
                let (Some(a), Some(b)) =
                    (g.lens.project(m.point(0.0)), g.lens.project(m.point(m.len)))
                else {
                    continue;
                };
                // Backward from the end through the start leads to the radiant, on screen and
                // on the sky (the lens is gnomonic, so screen lines are great circles).
                assert!(miss_deg(a, b, r) < 1.0, "{}", miss_deg(a, b, r));
                let sky = geom::v3::dot(geom::v3::unit(m.point(0.0)), g.radiant);
                assert!(sky < 1.0);
                checked += 1;
            }
            g.meteors.clear();
        }
        assert!(checked > 100, "{checked}");
    }

    #[test]
    fn met_a2_meteors_near_the_radiant_are_short() {
        let (g, _) = built(Size::new(200, 50));
        // The same physical path length placed at 10° and at 60° from the radiant.
        let screen_len = |sep_deg: f64| {
            let mut total = 0.0;
            for k in 0..12 {
                let turn = TAU * f64::from(k) / 12.0;
                // A direction `sep` from the radiant, around it.
                let r = g.radiant;
                let e = geom::v3::unit(geom::v3::cross(r, [0.0, 0.0, 1.0]));
                let n = geom::v3::cross(e, r);
                let (s, c) = sep_deg.to_radians().sin_cos_det();
                let (st, ct) = turn.sin_cos_det();
                let d = geom::v3::add(
                    geom::v3::scale(r, c),
                    geom::v3::add(geom::v3::scale(e, s * ct), geom::v3::scale(n, s * st)),
                );
                if d[2] < 0.1 {
                    continue;
                }
                let start = geom::v3::scale(d, 105.0 / d[2]);
                let end = geom::v3::axpy(-25.0, r, start);
                if let (Some(a), Some(b)) = (g.lens.project(start), g.lens.project(end)) {
                    total += (b.0 - a.0).hypot(b.1 - a.1);
                }
            }
            total
        };
        let near = screen_len(8.0);
        let far = screen_len(60.0);
        assert!(near > 0.0 && far > 0.0);
        assert!(near / far < 0.5, "{near} / {far}");
    }

    #[test]
    fn met_a3_magnitudes_follow_the_population_index() {
        let mut rng = SplitMix64::new(7);
        let mut bins = [0u32; 4];
        for _ in 0..1000 {
            let m = sample_mag(1.0 - rng.next_f64());
            let k = (MAG_FAINT - m).floor();
            if (0.0..4.0).contains(&k) {
                bins[k as usize] += 1;
            }
        }
        for k in 0..2 {
            let ratio = f64::from(bins[k]) / f64::from(bins[k + 1]);
            assert!((ratio / POP_INDEX - 1.0).abs() < 0.2, "{bins:?}");
        }
    }

    #[test]
    fn met_a4_trains_fade_monotonically_and_drift() {
        let mut prev = f64::INFINITY;
        for i in 0..=100 {
            let f = train_fade(f64::from(i) * 0.08, 8.0);
            assert!(f <= prev);
            prev = f;
        }
        assert_eq!(train_fade(8.0, 8.0), 0.0);
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let before = g.trains.iter().next().expect("a train at the start").pts;
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let after = g.trains.iter().next().expect("still there").pts;
        let moved = before
            .iter()
            .zip(after.iter())
            .map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]))
            .fold(0.0, f64::max);
        assert!(moved > 0.3, "{moved} km");
        // Sheared wind: points at different heights move differently, so the train bends.
        let spread = before
            .iter()
            .zip(after.iter())
            .map(|(a, b)| b[0] - a[0])
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| {
                (lo.min(d), hi.max(d))
            });
        assert!(spread.1 - spread.0 > 0.1, "{spread:?}");
    }

    #[test]
    fn met_a8_trains_are_thin_faint_short_lived_and_stop_at_the_treeline() {
        // Thin: a stroked thread lights at most two pixel rows across its length.
        let mut glow = Glow::new(Size::new(40, 10));
        thin_stroke(&mut glow, (10.0, 20.3), (60.0, 20.3), 1.0);
        let rows = (0..20)
            .filter(|&y| (0..40).any(|x| glow.values()[y * 40 + x] > 1e-3))
            .count();
        assert!((1..=2).contains(&rows), "{rows} rows");
        // Short-lived: a train fades out within ~4.5 s (the O I afterglow, time-lapsed).
        assert!(TRAIN_LIFE.1 * 1.4 <= 4.5);
        // Faint and never over the land, over several seconds of the shower.
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for step in 0..(6.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if step % 20 != 0 {
                continue;
            }
            let ps = g.picture.pixel_size();
            for y in 0..i32::from(ps.height()) {
                for x in 0..i32::from(ps.width()) {
                    let Some(ink) = g.picture.get(x, y) else {
                        continue;
                    };
                    if ink.role != Role::Particle {
                        continue;
                    }
                    assert!(ink.level <= 128, "bright train {ink:?}");
                    let top = g.land[x as usize];
                    assert!((y as u16) < top, "train over the land at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn met_a6_perseid_ablation_heights() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let mut n = 0;
        for _ in 0..400 {
            g.meteors.clear();
            g.spawn(&mut ctx.rng);
            if let Some(m) = g.meteors.iter().next() {
                // Real heights: the drawn path is `STRETCH` × the ablation layer's.
                let (begin, end) = (m.start[2], m.point(m.len / STRETCH)[2]);
                assert!(
                    (BEGIN_KM.0 - 1e-9..=BEGIN_KM.1 + 1e-9).contains(&begin),
                    "{begin}"
                );
                assert!((END_KM.0 - 1e-6..=END_KM.1 + 1e-6).contains(&end), "{end}");
                n += 1;
            }
        }
        assert!(n > 300, "{n}");
    }

    #[test]
    fn met_a7_visible_radiant_and_hot_head_green_yellow_tails() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        let r = g.lens.project(g.radiant).expect("radiant in front");
        assert!(
            r.0 > 0.0 && r.0 < 120.0 && r.1 > 0.0 && r.1 < g.lens.horizon(),
            "{r:?}"
        );
        let (mut green, mut yellow) = (0, 0);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        for _ in 0..(4.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            canvas.clear();
            g.render(&mut canvas);
            for c in canvas.cells() {
                if ('\u{2801}'..='\u{28ff}').contains(&c.ch) {
                    match c.ink.role {
                        Role::Particle => green += 1,
                        Role::Sun => yellow += 1,
                        Role::Star | Role::Structure | Role::Accent => {}
                        other => panic!("{other:?}"),
                    }
                }
            }
        }
        assert!(green > 20 && yellow > 5, "{green} {yellow}");
    }

    #[test]
    fn met_a5_pools_never_exceed_capacity() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for _ in 0..200 {
            g.spawn(&mut ctx.rng);
        }
        assert!(g.meteors.len() <= MAX_METEORS);
        for _ in 0..3600 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            assert!(g.meteors.len() <= MAX_METEORS && g.trains.len() <= MAX_TRAINS);
            assert!(g.meteors.capacity() == MAX_METEORS && g.trains.capacity() == MAX_TRAINS);
        }
    }

    #[test]
    fn met_light_curve_peaks_late_and_burns_out() {
        assert_eq!(light_curve(0.0), 0.0);
        assert_eq!(light_curve(1.0), 0.0);
        assert!((light_curve(2.0 / 3.0) - 1.0).abs() < 1e-12);
    }
}
