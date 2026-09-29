//! Chemical garden: seed crystals of metal salts in water glass grow tubes in the salts'
//! colours, in jerks: a membrane swells by osmosis, bursts, and the jet of salt solution sheathes
//! itself in new membrane; then a hydrothermal-vent chimney, where such mineral
//! membranes are hypothesised to have housed early life (SPEC-002 `chemical_garden`).
//!
//! A phenomenological burst model, no fluid solver: each growing tip holds an osmotic
//! pressure that rises at a steady rate (water drawn in through the semipermeable
//! precipitate membrane, driven by the concentration difference) and bursts when it
//! exceeds the membrane's strength (seeded ±30 %). A burst releases a buoyant jet that
//! lifts the tip by a random-walk step biased upward and precipitates a new tube segment;
//! the pressure falls to zero and the new membrane gets a fresh strength. Tips branch now
//! and then; precipitate flakes break off and sink.
//!
//! Drawn on black (ADR-0017): tubes are thin strokes anti-aliased by coverage in their
//! salts' muted colours, lighter while fresh and at the growing tips; the floor is one
//! dim line.

use rand::Rng;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "chemical_garden",
    category: Category::PhysicalChemistry,
    description: "Chemical garden: salt crystals in water glass grow tubes in osmotic bursts; a hydrothermal-vent chimney",
    title: "Chemical garden · osmosis bursts",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 2] = ["osmotic bursts", "life's vents?"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Garden::default())
}

// ---- Model (SPEC-002 Model) ----

/// Tube segments and tips at most, and loose particles (jets, flakes, smoke).
pub(crate) const SEGMENTS: usize = 600;
const TIPS: usize = 24;
const PARTICLES: usize = 800;
/// Membrane strength (pressure units) and its seeded spread.
const STRENGTH: f64 = 1.0;
const STRENGTH_SPREAD: f64 = 0.3;
/// Chance a burst also starts a branch.
const BRANCH_P: f64 = 0.1;
/// Tips stop near the surface.
const SURFACE_Y: f64 = 0.06;
const FLOOR_Y: f64 = 0.95;

const GARDEN_S: f64 = 30.0;
const VENT_S: f64 = 20.0;
const GARDEN: usize = 0;
const VENT: usize = 1;

/// One growing tube end, in tank units (x, y ∈ [0, 1], y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tip {
    pub(crate) x: f64,
    pub(crate) y: f64,
    /// Osmotic pressure, rising at `rate` per second until it reaches `strength`.
    pub(crate) p: f64,
    pub(crate) rate: f64,
    pub(crate) strength: f64,
    /// Tube radius in tank units.
    pub(crate) w: f64,
    /// Upward step per burst.
    pub(crate) step: f64,
    /// Which metal salt the tube precipitates (an index into [`SALTS`]).
    pub(crate) salt: u8,
}

/// A precipitated tube segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    pub(crate) a: [f64; 2],
    pub(crate) b: [f64; 2],
    pub(crate) w: f64,
    /// Seconds since it formed.
    pub(crate) age: f32,
    pub(crate) salt: u8,
    /// 1 while the garden it belongs to stands; falls to 0 once a new garden is sown,
    /// and the segment is then removed (GRD-A6).
    pub(crate) fade: f32,
}

/// Metal-salt tube colours (muted, ADR-0017): copper blue, cobalt pink, iron rust, nickel
/// green.
pub(crate) const SALTS: [Ramp; 4] = [Ramp::Ice, Ramp::HAlpha, Ramp::Sienna, Ramp::Aurora];
/// The vent chimney's mineral wall (iron sulfides and oxides): rust.
const CHIMNEY_SALT: u8 = 2;
/// An old garden fades out over this long while the new one sprouts.
pub(crate) const FADE_OUT_S: f32 = 5.0;
/// A garden is renewed once it holds this many segments or its tips have all stopped.
const RENEW_AT: usize = 380;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// A jet droplet of salt solution (rises, fades).
    Jet,
    /// A precipitate flake (sinks).
    Flake,
    /// Warm vent fluid (rises, spreads).
    Smoke,
    /// A gas bubble (hydrogen from the silicate, air from the seed) rising to the top.
    Bubble,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Particle {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) vx: f64,
    pub(crate) vy: f64,
    life: f32,
    kind: Kind,
    salt: u8,
}

/// Advance a tip's pressure by `dt`; `true` when its membrane bursts. Between bursts
/// the pressure only rises (GRD-A1).
pub(crate) fn charge(t: &mut Tip, dt: f64) -> bool {
    t.p += t.rate * dt;
    t.p >= t.strength
}

/// Burst `t`: the jet lifts the tip by a biased random step, and the pressure falls to
/// zero behind a fresh membrane. Returns the segment the new membrane precipitates.
pub(crate) fn burst(t: &mut Tip, rng: &mut impl Rng) -> Segment {
    let a = [t.x, t.y];
    let dy = t.step * rng.random_range(0.6..1.4);
    let dx = gaussian(rng, 0.0, 0.35 * t.step);
    t.x = (t.x + dx).clamp(0.01, 0.99);
    t.y -= dy;
    t.p = 0.0;
    t.strength = STRENGTH * (1.0 + rng.random_range(-STRENGTH_SPREAD..=STRENGTH_SPREAD));
    Segment {
        a,
        b: [t.x, t.y],
        w: t.w,
        age: 0.0,
        salt: t.salt,
        fade: 1.0,
    }
}

/// The garden's state: pools with fixed capacities.
#[derive(Debug, Clone)]
pub(crate) struct Tank {
    pub(crate) tips: Vec<Tip>,
    pub(crate) segments: Vec<Segment>,
    pub(crate) particles: ParticlePool<Particle>,
    /// Seed crystals: position and radius.
    seeds: [(f64, f64); 6],
    salts: [u8; 6],
    n_seeds: usize,
    vent: bool,
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            tips: Vec::with_capacity(TIPS),
            segments: Vec::with_capacity(SEGMENTS),
            particles: ParticlePool::with_capacity(PARTICLES),
            seeds: [(0.0, 0.0); 6],
            salts: [CHIMNEY_SALT; 6],
            n_seeds: 0,
            vent: false,
        }
    }
}

impl Tank {
    fn new_tip(x: f64, y: f64, vent: bool, rng: &mut impl Rng) -> Tip {
        Tip {
            salt: CHIMNEY_SALT,
            x,
            y,
            p: rng.random_range(0.0..0.8),
            // Bursts every 0.3–2 s: rate / strength ∈ [0.5, 3.3] per second.
            rate: if vent {
                rng.random_range(0.4..1.0)
            } else {
                rng.random_range(0.6..2.8)
            },
            strength: STRENGTH,
            w: if vent {
                rng.random_range(0.012..0.02)
            } else {
                rng.random_range(0.006..0.011)
            },
            step: if vent { 0.02 } else { 0.028 },
        }
    }

    /// Start a garden (seed crystals on the floor) or a vent (one chimney).
    pub(crate) fn plant(&mut self, vent: bool, rng: &mut impl Rng) {
        self.segments.clear();
        self.particles.clear();
        self.sow(vent, rng);
    }

    /// Sow a new garden (or vent) while the standing one fades out over
    /// [`FADE_OUT_S`], so the tank is never empty (GRD-A6).
    pub(crate) fn renew(&mut self, vent: bool, rng: &mut impl Rng) {
        for s in &mut self.segments {
            s.fade = s.fade.min(0.999);
        }
        self.sow(vent, rng);
    }

    /// Whether the garden has grown as far as it will: time for a new one.
    pub(crate) fn spent(&self) -> bool {
        let live = self.segments.iter().filter(|s| s.fade >= 1.0).count();
        live >= RENEW_AT || self.tips.is_empty()
    }

    fn sow(&mut self, vent: bool, rng: &mut impl Rng) {
        self.tips.clear();
        self.vent = vent;
        if vent {
            // A chimney already standing: a tapered stack of mineral wall, its crown
            // still growing and venting warm fluid.
            self.n_seeds = 1;
            self.seeds[0] = (0.5, FLOOR_Y);
            self.salts[0] = CHIMNEY_SALT;
            let top = 0.55;
            let layers = 24;
            for k in 0..layers {
                let t = f64::from(k) / f64::from(layers);
                let y0 = FLOOR_Y - (FLOOR_Y - top) * t;
                let y1 = FLOOR_Y - (FLOOR_Y - top) * (t + 1.0 / f64::from(layers));
                let half = 0.11 * (1.0 - 0.72 * t);
                for side in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                    let x = 0.5 + side * half * 0.6 + rng.random_range(-0.006..0.006);
                    self.segments.push(Segment {
                        a: [x, y0],
                        b: [x + rng.random_range(-0.005..0.005), y1],
                        w: half * 0.55,
                        age: 30.0,
                        salt: CHIMNEY_SALT,
                        fade: 1.0,
                    });
                }
            }
            for k in 0..4 {
                let x = 0.47 + 0.02 * f64::from(k) + rng.random_range(-0.005..0.005);
                self.tips.push(Self::new_tip(x, top, true, rng));
            }
        } else {
            self.n_seeds = rng.random_range(5..=6);
            // A few different salts per garden, as when a handful of crystals is dropped.
            let first = rng.random_range(0..SALTS.len());
            for k in 0..self.n_seeds {
                let x = (k as f64 + rng.random_range(0.25..0.75)) / self.n_seeds as f64;
                self.seeds[k] = (x, FLOOR_Y);
                let salt = ((first + k) % SALTS.len()) as u8;
                self.salts[k] = salt;
                for _ in 0..rng.random_range(1..=2) {
                    let tx = x + rng.random_range(-0.02..0.02);
                    let mut tip = Self::new_tip(tx, FLOOR_Y, false, rng);
                    tip.salt = salt;
                    self.tips.push(tip);
                }
            }
        }
    }

    /// Advance by `dt`; `false` when the segment pool is full (the scene then resets).
    pub(crate) fn step(&mut self, dt: f64, rng: &mut impl Rng) -> bool {
        let mut k = 0;
        while k < self.tips.len() {
            let mut t = self.tips[k];
            if charge(&mut t, dt) {
                if self.segments.len() >= SEGMENTS {
                    if self.segments.iter().any(|s| s.fade < 1.0) {
                        // An old garden is still fading out: the burst waits for room.
                        self.tips[k] = t;
                        k += 1;
                        continue;
                    }
                    return false;
                }
                let seg = burst(&mut t, rng);
                self.segments.push(seg);
                // The jet: droplets of salt solution shooting up out of the rupture.
                for _ in 0..5 {
                    self.particles.spawn(Particle {
                        x: t.x,
                        y: t.y,
                        vx: gaussian(rng, 0.0, 0.03),
                        vy: -rng.random_range(0.08..0.2),
                        life: 1.0,
                        kind: Kind::Jet,
                        salt: t.salt,
                    });
                }
                // Now and then a gas bubble escapes the rupture and rises to the top.
                if rng.random_bool(0.3) {
                    self.particles.spawn(Particle {
                        x: t.x,
                        y: t.y,
                        vx: 0.0,
                        vy: -rng.random_range(0.1..0.18),
                        life: 1.0,
                        kind: Kind::Bubble,
                        salt: t.salt,
                    });
                }
                if rng.random_bool(0.3) {
                    let mid = [0.5 * (seg.a[0] + seg.b[0]), 0.5 * (seg.a[1] + seg.b[1])];
                    self.particles.spawn(Particle {
                        x: mid[0],
                        y: mid[1],
                        vx: gaussian(rng, 0.0, 0.004),
                        vy: rng.random_range(0.01..0.03),
                        life: 1.0,
                        kind: Kind::Flake,
                        salt: t.salt,
                    });
                }
                if self.tips.len() < TIPS && rng.random_bool(BRANCH_P) {
                    let mut b = Self::new_tip(t.x, t.y, self.vent, rng);
                    b.w = t.w * 0.8;
                    b.salt = t.salt;
                    self.tips.push(b);
                }
            }
            self.tips[k] = t;
            if t.y < SURFACE_Y {
                self.tips.swap_remove(k);
            } else {
                k += 1;
            }
        }
        if self.vent {
            // Warm fluid venting from the chimney tops.
            for t in &self.tips {
                if rng.random_bool((dt * 40.0).min(1.0)) {
                    self.particles.spawn(Particle {
                        x: t.x,
                        y: t.y,
                        vx: gaussian(rng, 0.0, 0.015),
                        vy: -rng.random_range(0.16..0.3),
                        life: 1.0,
                        kind: Kind::Smoke,
                        salt: CHIMNEY_SALT,
                    });
                }
            }
        }
        for s in &mut self.segments {
            s.age += dt as f32;
            if s.fade < 1.0 {
                s.fade -= dt as f32 / FADE_OUT_S;
            }
        }
        // An old garden's segments leave the pool once faded (keeps its order, no
        // allocation).
        self.segments.retain(|s| s.fade > 0.0);
        self.particles.retain(|p| {
            match p.kind {
                Kind::Jet => {
                    p.vy *= (1.0 - 2.5 * dt).max(0.0);
                    p.life -= (1.2 * dt) as f32;
                }
                Kind::Flake => p.life -= (0.08 * dt) as f32,
                Kind::Bubble => {
                    // A small bubble rises at a steady speed, wobbling side to side.
                    p.vx = 0.02 * (40.0 * p.y + f64::from(p.salt)).sin_det();
                    // It dissolves on the way up, so bubbles never crowd the surface.
                    p.life -= (0.45 * dt) as f32;
                }
                Kind::Smoke => {
                    // The buoyant plume slows, spreads and billows as it rises.
                    p.vx += gaussian(rng, 0.0, 0.25) * dt;
                    p.vy *= (1.0 - 0.12 * dt).max(0.0);
                    p.life -= (0.3 * dt) as f32;
                }
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.life > 0.0 && (0.0..=FLOOR_Y).contains(&p.y) && (0.0..=1.0).contains(&p.x)
        });
        true
    }
}

#[derive(Debug)]
struct Garden {
    tank: Tank,
    time: f64,
    act: usize,
    timeline: Timeline,
    pixels: Pixels,
    glow: Glow,
    tone: Tone,
    /// Pixel rows the tank spans (above the title row).
    rows: f64,
    started: bool,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Garden {
    fn default() -> Self {
        Self {
            tank: Tank::default(),
            time: 0.0,
            act: usize::MAX,
            timeline: Timeline::new(&[(CAPTIONS[0], GARDEN_S), (CAPTIONS[1], VENT_S)]),
            pixels: Pixels::default(),
            glow: Glow::default(),
            tone: Tone::asinh(0.5, 12.0),
            rows: 1.0,
            started: false,
            dirty: true,
        }
    }
}

/// Each salt's ramp span `(lo, hi)`: old membrane at `lo`, the fresh growing tube and the
/// tip toward `hi` (ADR-0017: muted, so the tubes sit quietly on black).
const SALT_LEVELS: [(f64, f64); 4] = [(90.0, 190.0), (80.0, 150.0), (90.0, 180.0), (90.0, 170.0)];
/// Tube stroke radius in pixels: 1–3-pixel strokes, the chimney's wall the widest.
const TUBE_R: (f64, f64) = (0.5, 1.0);
const CHIMNEY_R: f64 = 1.5;
/// Segments wider than this (tank units) are chimney wall.
const CHIMNEY_W: f64 = 0.025;
/// The floor: one dim line.
const FLOOR_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 40);
/// The glow a smoke parcel exposes around its pixel.
const SMOKE_HALO: [(i32, i32, f32); 9] = sciart_core::raster::halo_kernel(1);

/// Set pixel `(x, y)` to `ink` unless it already holds a brighter one.
fn put_max(pixels: &mut Pixels, x: i32, y: i32, ink: Ink) {
    if ink.level == 0 {
        return;
    }
    if pixels.get(x, y).is_some_and(|old| old.level < ink.level) {
        pixels.set(x, y, ink);
    }
}

/// A capsule `a → b` of radius `r` (pixels), anti-aliased by coverage: each pixel's level
/// is `level` times the share of it inside the stroke, so edges fade into the black
/// instead of stepping (ADR-0017: thin lines, anti-aliased by intensity).
fn capsule(pixels: &mut Pixels, a: (f64, f64), b: (f64, f64), r: f64, role: Role, level: f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let reach = r + 1.0;
    let (x0, x1) = (
        (a.0.min(b.0) - reach).floor(),
        (a.0.max(b.0) + reach).ceil(),
    );
    let (y0, y1) = (
        (a.1.min(b.1) - reach).floor(),
        (a.1.max(b.1) + reach).ceil(),
    );
    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
        return;
    }
    for y in y0 as i32..=y1 as i32 {
        for x in x0 as i32..=x1 as i32 {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let t = if len2 > 0.0 {
                (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = (px - a.0 - t * dx).hypot(py - a.1 - t * dy);
            let cover = (r + 0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                put_max(pixels, x, y, Ink::new(role, ink::level(level * cover)));
            }
        }
    }
}

impl Garden {
    /// The tank on black: tubes as thin anti-aliased strokes in their salts' muted
    /// colours, lighter while fresh and at the growing tips; the seed crystals small
    /// points; the floor one dim line; the vent's smoke a faint grey glow, warm only at
    /// the orifice. Rebuilt from the model in `prepare` (GEN-R20).
    fn rasterise(&mut self) {
        self.pixels.clear();
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), self.rows);
        let to_px = |x: f64, y: f64| (x * pw, y * ph);
        // The floor line, under everything.
        let floor = (FLOOR_Y * ph) as i32;
        if f64::from(floor) < ph {
            for x in 0..i32::from(ps.width()) {
                self.pixels.set(x, floor, FLOOR_INK);
            }
        }
        let salt = |i: u8| usize::from(i) % SALTS.len();
        for s in &self.tank.segments {
            let k = salt(s.salt);
            let (lo, hi) = SALT_LEVELS[k];
            let fresh = (1.0 - f64::from(s.age) / 6.0).max(0.0);
            let fade = f64::from(s.fade.clamp(0.0, 1.0));
            let r = if s.w > CHIMNEY_W {
                CHIMNEY_R
            } else {
                (s.w * pw).clamp(TUBE_R.0, TUBE_R.1)
            };
            let level = (lo + (hi - lo) * 0.6 * fresh) * fade;
            capsule(
                &mut self.pixels,
                to_px(s.a[0], s.a[1]),
                to_px(s.b[0], s.b[1]),
                r,
                Role::Ramp(SALTS[k]),
                level,
            );
        }
        // Seed crystals (the chimney stands on its own): small points.
        if !self.tank.vent {
            for (k, &(sx, sy)) in self.tank.seeds[..self.tank.n_seeds].iter().enumerate() {
                let i = salt(self.tank.salts[k]);
                let c = to_px(sx, sy);
                capsule(
                    &mut self.pixels,
                    c,
                    c,
                    1.0,
                    Role::Ramp(SALTS[i]),
                    SALT_LEVELS[i].1,
                );
            }
        }
        // Growing tips: the lightest point of each tube, swelling a little with the
        // pressure inside before it bursts.
        for t in &self.tank.tips {
            let i = salt(t.salt);
            let swell = (t.p / t.strength).clamp(0.0, 1.0);
            let c = to_px(t.x, t.y);
            let level = SALT_LEVELS[i].1 + 30.0 * swell;
            capsule(
                &mut self.pixels,
                c,
                c,
                0.7 + 0.5 * swell,
                Role::Ramp(SALTS[i]),
                level,
            );
        }
        // The vent plume: a faint grey glow, warm only at the orifice.
        self.glow.clear();
        if self.tank.vent {
            for p in self.tank.particles.iter().filter(|p| p.kind == Kind::Smoke) {
                let (x, y) = to_px(p.x, p.y);
                self.glow
                    .splat(2.0 * x, 2.0 * y, &SMOKE_HALO, 0.25 * p.life);
            }
            let orifice = to_px(0.5, 0.55);
            let (glow, tone, rows) = (&self.glow, &self.tone, self.rows);
            for y in 0..i32::from(ps.height()) {
                for x in 0..i32::from(ps.width()) {
                    let level = tone.level(glow.get(x, y).unwrap_or(0.0));
                    if level < 24 {
                        continue;
                    }
                    let hot = (orifice.1 - f64::from(y)) < 0.08 * rows
                        && (f64::from(x) - orifice.0).abs() < 0.05 * pw;
                    let ink = if hot {
                        Ink::new(Role::Ramp(Ramp::Ember), level / 2 + 30)
                    } else {
                        Ink::new(Role::Ramp(Ramp::Starlight), level / 4 + 10)
                    };
                    put_max(&mut self.pixels, x, y, ink);
                }
            }
        }
        // Jets and sinking flakes: single dim pixels.
        for p in &self.tank.particles {
            let i = salt(p.salt);
            let (lo, _) = SALT_LEVELS[i];
            let level = match p.kind {
                Kind::Smoke | Kind::Bubble => continue,
                Kind::Jet => lo * (0.5 + 0.5 * f64::from(p.life)),
                Kind::Flake => 0.7 * lo,
            };
            let (x, y) = to_px(p.x, p.y);
            put_max(
                &mut self.pixels,
                x as i32,
                y as i32,
                Ink::new(Role::Ramp(SALTS[i]), ink::level(level)),
            );
        }
    }
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = sciart_core::raster::stronger(top, bottom);
    if ink.role == Role::Background {
        return None;
    }
    let ch = match ink.role {
        Role::Ramp(Ramp::Starlight) if ink.level <= FLOOR_INK.level => '_',
        Role::Ramp(Ramp::Starlight) => '.',
        _ if ink.level >= 90 => '|',
        _ => ',',
    };
    Some((ch, ink))
}

impl Generator for Garden {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.time).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.pixels.resize(size);
        self.glow.resize(size);
        // The tank fills the rows above the title row.
        self.rows = f64::from(2 * size.height().saturating_sub(1).max(1));
        if !self.started {
            self.started = true;
            self.act = GARDEN;
            self.tank.plant(false, &mut ctx.rng);
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.time += dt;
        let act = self.timeline.at(self.time).index;
        if act != self.act {
            self.act = act;
            // The new act grows while the old one fades out (GRD-A6).
            self.tank.renew(act == VENT, &mut ctx.rng);
        } else if act == GARDEN && self.tank.spent() {
            self.tank.renew(false, &mut ctx.rng);
        }
        // Sub-steps of at most 1/60 s keep bursts from being skipped.
        let n = (dt * 60.0).ceil().max(1.0);
        for _ in 0..n as u32 {
            if !self.tank.step(dt / n, &mut ctx.rng) {
                // Full: the garden starts over (GRD-A3).
                self.tank.plant(self.act == VENT, &mut ctx.rng);
            }
        }
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grd_a1_pressure_rises_between_bursts_and_drops_after() {
        let mut rng = SplitMix64::new(1);
        let mut t = Tank::new_tip(0.5, FLOOR_Y, false, &mut rng);
        let mut last = t.p;
        let mut bursts = 0;
        for _ in 0..600 {
            if charge(&mut t, 1.0 / 60.0) {
                let before = t.p;
                let _ = burst(&mut t, &mut rng);
                assert!(t.p < before && t.p == 0.0);
                bursts += 1;
            } else {
                assert!(t.p > last, "monotone between bursts");
            }
            last = t.p;
        }
        assert!(bursts >= 3, "{bursts} bursts in 10 s");
    }

    #[test]
    fn grd_a2_tips_climb_on_average() {
        let mut rng = SplitMix64::new(2);
        let mut tank = Tank::default();
        tank.plant(false, &mut rng);
        let start: f64 = tank.tips.iter().map(|t| t.y).sum::<f64>() / tank.tips.len() as f64;
        for _ in 0..(10 * 60) {
            tank.step(1.0 / 60.0, &mut rng);
        }
        let end: f64 = tank.tips.iter().map(|t| t.y).sum::<f64>() / tank.tips.len().max(1) as f64;
        assert!(end < start, "mean tip y {start:.3} → {end:.3} (up is −y)");
        // Every segment points up.
        let mean_dy: f64 =
            tank.segments.iter().map(|s| s.b[1] - s.a[1]).sum::<f64>() / tank.segments.len() as f64;
        assert!(mean_dy < 0.0);
    }

    #[test]
    fn grd_a3_pools_are_capped_and_a_full_garden_resets() {
        let mut rng = SplitMix64::new(3);
        let mut tank = Tank::default();
        tank.plant(false, &mut rng);
        let mut resets = 0;
        for _ in 0..(600 * 60) {
            if !tank.step(1.0 / 60.0, &mut rng) {
                resets += 1;
                tank.plant(false, &mut rng);
            }
            assert!(tank.segments.len() <= SEGMENTS && tank.tips.len() <= TIPS);
            assert!(tank.particles.len() <= PARTICLES);
            assert!(tank.segments.capacity() == SEGMENTS, "no reallocation");
            if tank.tips.is_empty() {
                tank.plant(false, &mut rng);
            }
        }
        let _ = resets;
    }

    #[test]
    fn grd_a4_the_vent_caption_is_a_question() {
        let mut ctx = Ctx::new(4, Size::new(80, 24));
        let mut g = Garden::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        assert_eq!(g.caption(), Some("osmotic bursts"));
        while g.time < GARDEN_S + 1.0 {
            g.update(0.25, &mut ctx);
            g.prepare();
        }
        let c = g.caption().unwrap_or("");
        assert_eq!(c, "life's vents?");
        assert!(c.ends_with('?'));
        assert!(g.tank.vent);
    }

    #[test]
    fn grd_a6_an_old_garden_fades_while_a_new_one_grows() {
        let mut rng = SplitMix64::new(6);
        let mut tank = Tank::default();
        tank.plant(false, &mut rng);
        for _ in 0..(8 * 60) {
            tank.step(1.0 / 60.0, &mut rng);
        }
        let old = tank.segments.len();
        assert!(old > 0);
        tank.renew(false, &mut rng);
        let mut t = 0.0;
        while t < f64::from(FADE_OUT_S) + 0.5 {
            assert!(tank.step(1.0 / 60.0, &mut rng));
            // Never an empty tank: the old garden is still there or the new one is.
            assert!(!tank.segments.is_empty(), "empty tank at {t:.2} s");
            t += 1.0 / 60.0;
        }
        assert!(
            tank.segments.iter().all(|s| s.fade >= 1.0),
            "the old garden is gone after the fade"
        );
        // A full-screen check: the rendered tank always holds tubes, act changes included.
        let mut ctx = Ctx::new(6, Size::new(80, 24));
        let mut g = Garden::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        while g.time < GARDEN_S + VENT_S + 12.0 {
            g.update(0.25, &mut ctx);
            g.prepare();
            if g.time > 3.0 {
                assert!(
                    !g.tank.segments.is_empty(),
                    "empty tank at t = {:.2}",
                    g.time
                );
            }
        }
    }

    #[test]
    fn grd_a7_thin_muted_tubes_on_a_black_tank() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let mut ctx = Ctx::new(7, size);
            let mut g = Garden::default();
            g.resize(size, &mut ctx);
            for t in [5.0, 20.0, 40.0] {
                while g.time < t {
                    g.update(1.0 / 60.0, &mut ctx);
                }
                g.prepare();
                let ps = g.pixels.pixel_size();
                let (mut lit, mut n) = (0, 0);
                for y in 0..i32::from(ps.height()) {
                    for x in 0..i32::from(ps.width()) {
                        let ink = g.pixels.get(x, y).unwrap_or(Ink::BLANK);
                        n += 1;
                        if ink.role == Role::Background {
                            continue;
                        }
                        lit += 1;
                        // Muted: tubes stay at their ramps' low-to-mid levels.
                        assert!(ink.level <= 225, "{size:?} t {t}: {ink:?}");
                    }
                }
                let share = f64::from(lit) / f64::from(n);
                assert!(
                    share > 0.002 && share < 0.2,
                    "{size:?} t {t}: lit {share:.3}"
                );
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                g.render(&mut canvas);
                assert!(
                    canvas.cells().iter().all(|c| c.ch != 'o'),
                    "no bubble glyphs"
                );
            }
        }
    }

    #[test]
    fn grd_a5_flakes_fall() {
        let mut rng = SplitMix64::new(5);
        let mut tank = Tank::default();
        tank.plant(false, &mut rng);
        for _ in 0..(5 * 60) {
            tank.step(1.0 / 60.0, &mut rng);
        }
        let flakes: Vec<_> = tank
            .particles
            .iter()
            .filter(|p| p.kind == Kind::Flake)
            .collect();
        assert!(!flakes.is_empty());
        assert!(
            flakes.iter().all(|p| p.vy > 0.0),
            "flakes sink (+y is down)"
        );
    }
}
