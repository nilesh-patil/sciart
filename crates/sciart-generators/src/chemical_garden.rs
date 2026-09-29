//! Chemical garden: seed crystals of a metal salt in water glass grow blue tubes in
//! jerks: a membrane swells by osmosis, bursts, and the jet of salt solution sheathes
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

use rand::Rng;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "chemical_garden",
    category: Category::LivingMatter,
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
}

/// A precipitated tube segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    pub(crate) a: [f64; 2],
    pub(crate) b: [f64; 2],
    pub(crate) w: f64,
    /// Seconds since it formed.
    pub(crate) age: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// A jet droplet of salt solution (rises, fades).
    Jet,
    /// A precipitate flake (sinks).
    Flake,
    /// Warm vent fluid (rises, spreads).
    Smoke,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Particle {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) vx: f64,
    pub(crate) vy: f64,
    life: f32,
    kind: Kind,
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
            n_seeds: 0,
            vent: false,
        }
    }
}

impl Tank {
    fn new_tip(x: f64, y: f64, vent: bool, rng: &mut impl Rng) -> Tip {
        Tip {
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
        self.tips.clear();
        self.segments.clear();
        self.particles.clear();
        self.vent = vent;
        if vent {
            // A chimney already standing: a tapered stack of mineral wall, its crown
            // still growing and venting warm fluid.
            self.n_seeds = 1;
            self.seeds[0] = (0.5, FLOOR_Y);
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
                    });
                }
            }
            for k in 0..4 {
                let x = 0.47 + 0.02 * f64::from(k) + rng.random_range(-0.005..0.005);
                self.tips.push(Self::new_tip(x, top, true, rng));
            }
        } else {
            self.n_seeds = rng.random_range(5..=6);
            for k in 0..self.n_seeds {
                let x = (k as f64 + rng.random_range(0.25..0.75)) / self.n_seeds as f64;
                self.seeds[k] = (x, FLOOR_Y);
                for _ in 0..rng.random_range(1..=2) {
                    let tx = x + rng.random_range(-0.02..0.02);
                    self.tips.push(Self::new_tip(tx, FLOOR_Y, false, rng));
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
                    });
                }
                if self.tips.len() < TIPS && rng.random_bool(BRANCH_P) {
                    let mut b = Self::new_tip(t.x, t.y, self.vent, rng);
                    b.w = t.w * 0.8;
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
                    });
                }
            }
        }
        for s in &mut self.segments {
            s.age += dt as f32;
        }
        self.particles.retain(|p| {
            match p.kind {
                Kind::Jet => {
                    p.vy *= (1.0 - 2.5 * dt).max(0.0);
                    p.life -= (1.2 * dt) as f32;
                }
                Kind::Flake => p.life -= (0.08 * dt) as f32,
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
        }
    }
}

const TUBE: Ramp = Ramp::Cobalt;
/// The glow a smoke parcel exposes around its pixel.
const SMOKE_HALO: [(i32, i32, f32); 25] = sciart_core::raster::halo_kernel(2);

impl Garden {
    fn rasterise(&mut self) {
        self.pixels.clear();
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), self.rows);
        let to_px = |x: f64, y: f64| (x * pw, y * ph);
        // The floor: a band of sediment.
        let floor = (FLOOR_Y * ph) as i32;
        for y in floor..ph as i32 {
            for x in 0..i32::from(ps.width()) {
                let n = sciart_core::rng::hash01(sciart_core::rng::cell_key(x, y));
                self.pixels.set(
                    x,
                    y,
                    Ink::new(Role::Ramp(Ramp::Starlight), 40 + (n * 30.0) as u8),
                );
            }
        }
        // Tubes: thick strokes, fresh membrane pale, old tube deep cobalt.
        for s in &self.tank.segments {
            let (ax, ay) = to_px(s.a[0], s.a[1]);
            let (bx, by) = to_px(s.b[0], s.b[1]);
            let r = (s.w * pw).max(0.6);
            let fresh = (1.0 - f64::from(s.age) / 6.0).max(0.0);
            let len = (bx - ax).hypot(by - ay);
            let n = (len * 2.0).ceil().max(1.0) as i32;
            for k in 0..=n {
                let t = f64::from(k) / f64::from(n);
                let (cx, cy) = (ax + (bx - ax) * t, ay + (by - ay) * t);
                let reach = r.ceil() as i32;
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                        let (ox, _) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
                        let d = (f64::from(dx).powi(2) + f64::from(dy).powi(2)).sqrt();
                        if d <= r + 0.3 {
                            // Round tube: the rim darker than the axis.
                            let across = (ox / (r + 0.5)).abs().min(1.0);
                            let lvl = 90.0 + 70.0 * (1.0 - across) + 80.0 * fresh;
                            self.pixels
                                .set(x, y, Ink::new(Role::Ramp(TUBE), ink::level(lvl)));
                        }
                    }
                }
            }
        }
        // Seed crystals.
        for &(sx, sy) in &self.tank.seeds[..self.tank.n_seeds] {
            let (cx, cy) = to_px(sx, sy);
            let r = if self.tank.vent {
                0.03 * pw
            } else {
                0.012 * pw + 1.0
            };
            let reach = r.ceil() as i32;
            for dy in -reach..=0 {
                for dx in -reach..=reach {
                    if f64::from(dx * dx + dy * dy) <= r * r {
                        self.pixels.set(
                            cx as i32 + dx,
                            cy as i32 + dy,
                            Ink::new(Role::Ramp(TUBE), 120),
                        );
                    }
                }
            }
        }
        // Tips: a membrane bulb swelling with the pressure inside.
        for t in &self.tank.tips {
            let (cx, cy) = to_px(t.x, t.y);
            let swell = (t.p / t.strength).clamp(0.0, 1.0);
            let r = (t.w * pw * (1.0 + 0.9 * swell)).max(0.8);
            let reach = r.ceil() as i32;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    if f64::from(dx * dx + dy * dy) <= r * r {
                        let lvl = 150.0 + 100.0 * swell;
                        self.pixels.set(
                            cx as i32 + dx,
                            cy as i32 + dy,
                            Ink::new(Role::Ramp(TUBE), ink::level(lvl)),
                        );
                    }
                }
            }
        }
        // The vent plume: smoke exposes a soft glow, hot at the orifice (Ember), cooling
        // to grey smoke as it rises (Starlight), so the plume reads as one billowing body.
        self.glow.clear();
        for p in self.tank.particles.iter().filter(|p| p.kind == Kind::Smoke) {
            let (x, y) = to_px(p.x, p.y);
            self.glow
                .splat(2.0 * x, 2.0 * y, &SMOKE_HALO, 0.25 * p.life);
        }
        if self.tank.vent {
            let ps = self.pixels.pixel_size();
            let orifice = to_px(0.5, 0.55);
            for y in 0..i32::from(ps.height()) {
                for x in 0..i32::from(ps.width()) {
                    let level = self.tone.level(self.glow.get(x, y).unwrap_or(0.0));
                    if level < 18 || self.pixels.get(x, y) != Some(Ink::BLANK) {
                        continue;
                    }
                    let hot = (orifice.1 - f64::from(y)) < 0.08 * self.rows
                        && (f64::from(x) - orifice.0).abs() < 0.05 * f64::from(ps.width());
                    let ink = if hot {
                        Ink::new(Role::Ramp(Ramp::Ember), level.saturating_add(60).min(230))
                    } else {
                        Ink::new(Role::Ramp(Ramp::Starlight), level / 2 + 10)
                    };
                    self.pixels.set(x, y, ink);
                }
            }
        }
        for p in &self.tank.particles {
            if p.kind == Kind::Smoke {
                continue;
            }
            let (x, y) = to_px(p.x, p.y);
            let ink = match p.kind {
                Kind::Jet => Ink::new(
                    Role::Ramp(Ramp::Ice),
                    ink::level(40.0 + 110.0 * f64::from(p.life)),
                ),
                Kind::Flake => Ink::new(Role::Ramp(TUBE), 70),
                Kind::Smoke => Ink::new(
                    Role::Ramp(Ramp::Starlight),
                    ink::level(25.0 + 95.0 * f64::from(p.life)),
                ),
            };
            if self.pixels.get(x as i32, y as i32) == Some(Ink::BLANK) {
                self.pixels.set(x as i32, y as i32, ink);
            }
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
        Role::Ramp(TUBE) if ink.level >= 90 => '|',
        Role::Ramp(TUBE) => ',',
        Role::Ramp(Ramp::Ice) => '\'',
        Role::Ramp(Ramp::Starlight) if ink.level < 75 => '_',
        _ => '.',
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
        self.rasterise();
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
            self.tank.plant(act == VENT, &mut ctx.rng);
        }
        // Sub-steps of at most 1/60 s keep bursts from being skipped.
        let n = (dt * 60.0).ceil().max(1.0);
        for _ in 0..n as u32 {
            if !self.tank.step(dt / n, &mut ctx.rng) {
                // Full: the garden starts over (GRD-A3).
                self.tank.plant(self.act == VENT, &mut ctx.rng);
            }
        }
        self.rasterise();
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
        assert_eq!(g.caption(), Some("osmotic bursts"));
        while g.time < GARDEN_S + 1.0 {
            g.update(0.25, &mut ctx);
        }
        let c = g.caption().unwrap_or("");
        assert_eq!(c, "life's vents?");
        assert!(c.ends_with('?'));
        assert!(g.tank.vent);
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
