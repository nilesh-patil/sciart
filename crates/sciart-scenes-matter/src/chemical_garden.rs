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
//! ## Look (M18, GEN-R26: precipitate bead chains)
//!
//! Braille only, on a black tank. Every burst precipitates one bead, a small Gaussian
//! stipple atom (`Dots::stipple_puff`, CORE-R108), bonded to the bead it grew from by a
//! 1-px thread, so each tube is a branched molecule in its salt's colour that grows upward
//! in jerks. The tip bead swells with its osmotic pressure and, when it bursts, the jet
//! that overshoots the new tip shows as a short thread for 0.3 s. Flakes are single
//! sinking dots; the vent's chimney is a column of stacked beads venting warm fluid as
//! rising dots with short afterglow threads.

use rand::Rng;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::rng::hash01;

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

/// Tube segments at most, growing tips, loose particles (jets, flakes, smoke) and jet
/// threads.
pub(crate) const SEGMENTS: usize = 600;
const TIPS: usize = 24;
const PARTICLES: usize = 800;
const JETS: usize = 32;
/// Membrane strength (pressure units) and its seeded spread.
const STRENGTH: f64 = 1.0;
const STRENGTH_SPREAD: f64 = 0.3;
/// Chance a burst also starts a branch: tubes in the garden; the chimney's crown
/// strands rarely split, so the crown stays one column.
const BRANCH_P: f64 = 0.1;
const VENT_BRANCH_P: f64 = 0.03;
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
    /// The bead (or seed) the tip sits on: the parent of the next bead (GRD-A9).
    pub(crate) bead: u32,
}

/// A precipitated tube segment: the bond `a → b` and the bead at `b`.
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
    /// The bead's id, and the id of the bead (or seed) it grew from (GRD-A9).
    pub(crate) id: u32,
    pub(crate) parent: u32,
}

/// Metal-salt tube colours in M11's semantic roles (GEN-R23): copper blue, cobalt pink,
/// iron rust, nickel green.
pub(crate) const SALTS: [Role; 4] = [Role::Cold, Role::Accent, Role::Hot, Role::Particle];
/// The vent chimney's mineral wall (iron sulfides and oxides): rust.
const CHIMNEY_SALT: u8 = 2;
/// An old garden fades out over this long while the new one sprouts.
pub(crate) const FADE_OUT_S: f32 = 5.0;
/// A garden is renewed once it holds this many segments or its tips have all stopped.
const RENEW_AT: usize = 380;
/// The chimney: layers of mineral wall, and its two walls (bead columns, either side of
/// the fluid conduit) as offsets of the half-width.
const CHIMNEY_LAYERS: u32 = 24;
const CHIMNEY_COLS: [f64; 2] = [-1.0, 1.0];
/// The chimney's crown (tank y).
const CROWN_Y: f64 = 0.55;

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
    /// A spawn counter: which vent parcels are drawn is fixed per parcel.
    tag: u32,
}

/// The jet of a burst, shooting on past the new tip before its sheath precipitates:
/// drawn as a short thread for [`JET_S`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Jet {
    pub(crate) from: [f64; 2],
    pub(crate) to: [f64; 2],
    pub(crate) age: f32,
    pub(crate) salt: u8,
}

/// A jet thread's afterglow, and the part of it drawn bright.
const JET_S: f32 = 0.3;
const JET_BRIGHT_S: f32 = 0.15;
/// How far the jet overshoots the new tip, as a share of the burst step.
const JET_OVERSHOOT: f64 = 0.9;

/// Advance a tip's pressure by `dt`; `true` when its membrane bursts. Between bursts
/// the pressure only rises (GRD-A1).
pub(crate) fn charge(t: &mut Tip, dt: f64) -> bool {
    t.p += t.rate * dt;
    t.p >= t.strength
}

/// Burst `t`: the jet lifts the tip by a biased random step, and the pressure falls to
/// zero behind a fresh membrane. Returns the segment the new membrane precipitates, a
/// bead `id` bonded to the bead the tip sat on, which becomes the tip's bead.
pub(crate) fn burst(t: &mut Tip, id: u32, rng: &mut impl Rng) -> Segment {
    let a = [t.x, t.y];
    let dy = t.step * rng.random_range(0.6..1.4);
    let dx = gaussian(rng, 0.0, 0.35 * t.step);
    t.x = (t.x + dx).clamp(0.01, 0.99);
    t.y -= dy;
    t.p = 0.0;
    t.strength = STRENGTH * (1.0 + rng.random_range(-STRENGTH_SPREAD..=STRENGTH_SPREAD));
    let parent = t.bead;
    t.bead = id;
    Segment {
        a,
        b: [t.x, t.y],
        w: t.w,
        age: 0.0,
        salt: t.salt,
        fade: 1.0,
        id,
        parent,
    }
}

/// A tip bead's size: σ (dots) from [`TIP_SIGMA`]`.0` at rest to `.1` at the burst
/// threshold, rising with the osmotic pressure (GRD-A10).
pub(crate) fn tip_sigma(p: f64, strength: f64) -> f64 {
    let f = if strength > 0.0 {
        (p / strength).clamp(0.0, 1.0)
    } else {
        1.0
    };
    TIP_SIGMA.0 + (TIP_SIGMA.1 - TIP_SIGMA.0) * f
}

/// The garden's state: pools with fixed capacities.
#[derive(Debug, Clone)]
pub(crate) struct Tank {
    pub(crate) tips: Vec<Tip>,
    pub(crate) segments: Vec<Segment>,
    pub(crate) particles: ParticlePool<Particle>,
    /// Jet threads, a ring (oldest overwritten).
    pub(crate) jets: [Jet; JETS],
    next_jet: usize,
    /// Seed crystals: position, salt and bead id.
    seeds: [(f64, f64); 6],
    salts: [u8; 6],
    pub(crate) seed_ids: [u32; 6],
    pub(crate) n_seeds: usize,
    next_id: u32,
    vent: bool,
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            tips: Vec::with_capacity(TIPS),
            segments: Vec::with_capacity(SEGMENTS),
            particles: ParticlePool::with_capacity(PARTICLES),
            jets: [Jet {
                from: [0.0; 2],
                to: [0.0; 2],
                age: JET_S,
                salt: 0,
            }; JETS],
            next_jet: 0,
            seeds: [(0.0, 0.0); 6],
            salts: [CHIMNEY_SALT; 6],
            seed_ids: [0; 6],
            n_seeds: 0,
            next_id: 0,
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
            bead: 0,
        }
    }

    fn fresh_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Start a garden (seed crystals on the floor) or a vent (one chimney).
    pub(crate) fn plant(&mut self, vent: bool, rng: &mut impl Rng) {
        self.segments.clear();
        self.particles.clear();
        for j in &mut self.jets {
            j.age = JET_S;
        }
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
            // A chimney already standing: a tapered stack of mineral wall, two bead
            // columns bonded layer to layer on one footing, its crown still growing and
            // venting warm fluid.
            self.n_seeds = 1;
            self.seeds[0] = (0.5, FLOOR_Y);
            self.salts[0] = CHIMNEY_SALT;
            let foot = self.fresh_id();
            self.seed_ids[0] = foot;
            let mut below = [foot; CHIMNEY_COLS.len()];
            let mut tops = [[0.5, CROWN_Y]; CHIMNEY_COLS.len()];
            let mut prev_x = [0.5; CHIMNEY_COLS.len()];
            let layers = f64::from(CHIMNEY_LAYERS);
            for k in 0..CHIMNEY_LAYERS {
                let t = f64::from(k) / layers;
                let y0 = FLOOR_Y - (FLOOR_Y - CROWN_Y) * t;
                let y1 = FLOOR_Y - (FLOOR_Y - CROWN_Y) * (t + 1.0 / layers);
                let half = 0.11 * (1.0 - 0.72 * t);
                for (c, side) in CHIMNEY_COLS.iter().enumerate() {
                    let x = 0.5 + side * half * 0.6 + 0.4 * rng.random_range(-0.006..0.006);
                    let a = [if k == 0 { x } else { prev_x[c] }, y0];
                    let b = [x + 0.4 * rng.random_range(-0.005..0.005), y1];
                    let id = self.fresh_id();
                    self.segments.push(Segment {
                        a,
                        b,
                        w: half * 0.55,
                        age: 30.0,
                        salt: CHIMNEY_SALT,
                        fade: 1.0,
                        id,
                        parent: below[c],
                    });
                    below[c] = id;
                    prev_x[c] = b[0];
                    tops[c] = b;
                }
            }
            // The crown keeps growing: two tips on each wall's top bead (a branch point).
            for c in [0, 1, 0, 1] {
                let _ = rng.random_range(-0.005..0.005);
                let mut tip = Self::new_tip(tops[c][0], tops[c][1], true, rng);
                tip.bead = below[c];
                self.tips.push(tip);
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
                let seed = self.fresh_id();
                self.seed_ids[k] = seed;
                for _ in 0..rng.random_range(1..=2) {
                    // Twin tubes root a little apart on the crystal, so they rise as a V.
                    let tx = x + rng.random_range(-0.015..0.015);
                    let mut tip = Self::new_tip(tx, FLOOR_Y, false, rng);
                    tip.salt = salt;
                    tip.bead = seed;
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
                let id = self.fresh_id();
                let seg = burst(&mut t, id, rng);
                self.segments.push(seg);
                // The jet overshoots the new tip along the burst before it sheathes.
                let (dx, dy) = (seg.b[0] - seg.a[0], seg.b[1] - seg.a[1]);
                self.jets[self.next_jet] = Jet {
                    from: seg.b,
                    to: [seg.b[0] + JET_OVERSHOOT * dx, seg.b[1] + JET_OVERSHOOT * dy],
                    age: 0.0,
                    salt: t.salt,
                };
                self.next_jet = (self.next_jet + 1) % JETS;
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
                        tag: 0,
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
                        tag: 0,
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
                        tag: 0,
                    });
                }
                let branch_p = if self.vent { VENT_BRANCH_P } else { BRANCH_P };
                if self.tips.len() < TIPS && rng.random_bool(branch_p) {
                    // A branch: the new bead gets a second child (GRD-A9).
                    let mut b = Self::new_tip(t.x, t.y, self.vent, rng);
                    b.w = t.w * 0.8;
                    b.salt = t.salt;
                    b.bead = t.bead;
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
            for k in 0..self.tips.len() {
                let t = self.tips[k];
                if rng.random_bool((dt * 40.0).min(1.0)) {
                    let tag = self.fresh_id();
                    self.particles.spawn(Particle {
                        x: t.x,
                        y: t.y,
                        vx: gaussian(rng, 0.0, 0.015),
                        vy: -rng.random_range(0.16..0.3),
                        life: 1.0,
                        kind: Kind::Smoke,
                        salt: CHIMNEY_SALT,
                        tag,
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
        for j in &mut self.jets {
            j.age = (j.age + dt as f32).min(JET_S);
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

// ---- Look (SPEC-002 Look (M18)) ----

/// M15's stepped levels, kept by the beads (GRD-A11): an old membrane, a fresh one
/// (younger than [`FRESH_S`]), the growing tip, and a garden fading out.
const OLD: u8 = 120;
const FRESH: u8 = 200;
const TIP: u8 = 255;
const FADING: u8 = 60;
const FRESH_S: f32 = 4.0;
/// Bead levels from brightest down (GRD-A11).
#[cfg(test)]
const STEPS: [u8; 4] = [TIP, FRESH, OLD, FADING];
/// Stick levels: under a tip or fresh bead, and under an old one (a fading garden's
/// sticks share its beads' level).
const FRESH_BOND: u8 = 200;
const OLD_BOND: u8 = 110;
/// Loose things: flakes, and a jet thread's afterglow.
const LOOSE: u8 = 90;
/// Tube beads: σ (dots) and peak of the Gaussian atom.
const BEAD_SIGMA: f64 = 1.0;
const BEAD_PEAK: f64 = 2.5;
/// Path length (dots) between drawn atoms along a tube.
const BEAD_EVERY: f32 = 7.0;
/// Dots between an atom's centre and its bond's end.
const BOND_GAP: f64 = 2.0;
/// Chimney wall beads, a little heavier.
const WALL_SIGMA: f64 = 0.9;
const WALL_PEAK: f64 = 2.5;
/// A tip bead's σ from rest to the burst threshold (GRD-A10).
const TIP_SIGMA: (f64, f64) = (0.7, 1.6);
const TIP_PEAK: f64 = 2.0;
/// Seed crystals.
const SEED_SIGMA: f64 = 1.6;
/// Segments wider than this (tank units) are chimney wall.
const CHIMNEY_W: f64 = 0.025;
/// The floor: one dim thread.
const FLOOR_INK: Ink = Ink::new(Role::Dim, 60);
/// Precipitate flakes: single dim cobalt dots.
const FLAKE_INK: Ink = Ink::new(Role::Accent, LOOSE);
/// Vent fluid: warm at the orifice, cooling to dim as it rises.
const SMOKE_INK: Ink = Ink::new(Role::Dim, 110);
const WARM_INK: Ink = Ink::new(Role::Hot, 150);
/// Vent parcels older than this (life below) are no longer drawn, and parcels younger
/// than `WARM_LIFE` (life above) are still warm.
const SMOKE_LIFE: f32 = 0.55;
const WARM_LIFE: f32 = 0.75;
/// A smoke parcel's afterglow thread: its path over the last this-many seconds.
const SMOKE_TRAIL_S: f64 = 0.06;
/// The share of vent parcels drawn (fixed per parcel), so the plume stays a sparse
/// spray of rising dots rather than a grey wash.
const SMOKE_DRAWN: f64 = 0.15;
const SMOKE_SALT: u64 = 0x5EED_5A0C_E000_0001;

/// A bead's level: fresh for its first [`FRESH_S`], then old; a garden fading out drops
/// to old, then to the fading step for its last half (GRD-A11).
fn bead_level(s: &Segment) -> u8 {
    if s.fade < 0.5 {
        FADING
    } else if s.fade < 1.0 || s.age > FRESH_S {
        OLD
    } else {
        FRESH
    }
}

/// A bond's level, a fixed legible step for its child bead's (GRD-A11): never brighter
/// than the bead, and bright enough that an old tube still reads as atoms on sticks.
fn bond_level(child: u8) -> u8 {
    match child {
        TIP | FRESH => FRESH_BOND,
        OLD => OLD_BOND,
        _ => FADING,
    }
}

/// A piece of bond thread along `a → b`, stopped `gap_a` / `gap_b` dots short of its
/// ends, so a black gap sets each drawn atom apart from its sticks (ball-and-stick); a
/// piece too short for its gaps is left out.
fn bond(dots: &mut Dots, a: (f64, f64), b: (f64, f64), gaps: (f64, f64), ink: Ink) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len.is_nan() || len <= gaps.0 + gaps.1 + 0.5 {
        return;
    }
    let (fa, fb) = (gaps.0 / len, gaps.1 / len);
    dots.line(
        a.0 + fa * dx,
        a.1 + fa * dy,
        b.0 - fb * dx,
        b.1 - fb * dy,
        ink,
    );
}

/// Per segment, for drawing: its parent segment (`NO_PARENT` for a seed or the chimney's
/// footing), its child count, the path length (dots) since the last drawn atom, and
/// whether its bead is drawn as an atom.
#[derive(Debug, Clone, Copy)]
struct Link {
    parent: u16,
    kids: u8,
    since: f32,
    atom: bool,
}

const NO_PARENT: u16 = u16::MAX;

/// A bead's fixed stipple salt (CORE-R107): per bead, so neighbouring atoms differ.
fn bead_salt(id: u32) -> u64 {
    u64::from(id).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xC6A4_A793_5BD1_E995
}

#[derive(Debug)]
struct Garden {
    tank: Tank,
    time: f64,
    act: usize,
    timeline: Timeline,
    dots: Dots,
    /// Dot columns across and dot rows the tank spans (above the title row).
    span: (f64, f64),
    /// Per segment drawing scratch (capacity [`SEGMENTS`], never grows).
    links: Vec<Link>,
    started: bool,
    /// The dots are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Garden {
    fn default() -> Self {
        Self {
            tank: Tank::default(),
            time: 0.0,
            act: usize::MAX,
            timeline: Timeline::new(&[(CAPTIONS[0], GARDEN_S), (CAPTIONS[1], VENT_S)]),
            dots: Dots::default(),
            span: (1.0, 1.0),
            links: Vec::with_capacity(SEGMENTS),
            started: false,
            dirty: true,
        }
    }
}

impl Garden {
    /// Walk the tubes' segment chains (Look (M18)): a bead is drawn as an atom once the
    /// path since the last atom reaches [`BEAD_EVERY`] dots, and always at a tip, a
    /// branch point and a tube's end, so each tube reads as round atoms joined by sticks.
    /// Segments are stored in the order they formed (ids rising), so parents come first.
    fn link_chains(&mut self) {
        let segs = &self.tank.segments;
        self.links.clear();
        for (k, s) in segs.iter().enumerate() {
            let parent = segs[..k]
                .binary_search_by_key(&s.parent, |p| p.id)
                .map_or(NO_PARENT, |i| i as u16);
            if parent != NO_PARENT {
                let p = &mut self.links[usize::from(parent)];
                p.kids = p.kids.saturating_add(1);
            }
            self.links.push(Link {
                parent,
                kids: 0,
                since: 0.0,
                atom: false,
            });
        }
        for (k, s) in segs.iter().enumerate() {
            let (a, b) = (self.to_dot(s.a[0], s.a[1]), self.to_dot(s.b[0], s.b[1]));
            let len = (b.0 - a.0).hypot(b.1 - a.1) as f32;
            let l = self.links[k];
            let before = match l.parent {
                NO_PARENT => 0.0,
                p if self.links[usize::from(p)].atom => 0.0,
                p => self.links[usize::from(p)].since,
            };
            let since = before + len;
            let atom = l.kids != 1 || since >= BEAD_EVERY;
            self.links[k].atom = atom;
            self.links[k].since = if atom { 0.0 } else { since };
        }
    }

    /// Tank units to dot coordinates.
    fn to_dot(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.span.0 - 0.5, y * self.span.1 - 0.5)
    }

    /// The tank on black, as braille (Look (M18)): the floor thread; every bead a small
    /// stipple atom in its salt's colour bonded to its parent by a thread one step
    /// dimmer; the tips swelling with pressure; jet threads; flakes; the vent's fluid.
    /// Rebuilt from the model in `prepare` (GEN-R20).
    fn rasterise(&mut self) {
        self.dots.clear();
        let salt = |i: u8| SALTS[usize::from(i) % SALTS.len()];
        // The floor, under everything.
        let (_, fy) = self.to_dot(0.0, FLOOR_Y);
        self.dots
            .line(0.0, fy.round(), self.span.0 - 1.0, fy.round(), FLOOR_INK);
        self.link_chains();
        // Bonds first, so the atoms sit on them: each segment is a piece of the stick
        // between two drawn atoms, gapped where it meets one.
        for (s, l) in self.tank.segments.iter().zip(&self.links) {
            let ink = Ink::new(salt(s.salt), bond_level(bead_level(s)));
            let from_atom = l.parent == NO_PARENT || self.links[usize::from(l.parent)].atom;
            let gaps = (
                if from_atom { BOND_GAP } else { 0.0 },
                if l.atom { BOND_GAP } else { 0.0 },
            );
            let mut a = self.to_dot(s.a[0], s.a[1]);
            // A tube's first stick grows out of its seed crystal.
            if l.parent == NO_PARENT && !self.tank.vent {
                let n = self.tank.n_seeds;
                if let Some(k) = self.tank.seed_ids[..n]
                    .iter()
                    .position(|&id| id == s.parent)
                {
                    let (sx, sy) = self.tank.seeds[k];
                    let c = self.to_dot(sx, sy);
                    a = (c.0, c.1 - 1.0);
                }
            }
            let b = self.to_dot(s.b[0], s.b[1]);
            bond(&mut self.dots, a, b, gaps, ink);
        }
        for (s, l) in self.tank.segments.iter().zip(&self.links) {
            // Tip beads are drawn swelling, below.
            if !l.atom || self.tank.tips.iter().any(|t| t.bead == s.id) {
                continue;
            }
            let ink = Ink::new(salt(s.salt), bead_level(s));
            let (sigma, peak) = if self.tank.vent || s.w > CHIMNEY_W {
                (WALL_SIGMA, WALL_PEAK)
            } else {
                (BEAD_SIGMA, BEAD_PEAK)
            };
            let b = self.to_dot(s.b[0], s.b[1]);
            self.dots
                .stipple_puff(b.0, b.1, sigma, peak, bead_salt(s.id), ink);
        }
        // Seed crystals (the chimney stands on its own footing).
        if !self.tank.vent {
            for k in 0..self.tank.n_seeds {
                let (sx, sy) = self.tank.seeds[k];
                let c = self.to_dot(sx, sy);
                let ink = Ink::new(salt(self.tank.salts[k]), TIP);
                let id = self.tank.seed_ids[k];
                self.dots
                    .stipple_puff(c.0, c.1 - 1.0, SEED_SIGMA, TIP_PEAK, bead_salt(id), ink);
            }
        }
        // Growing tips swell with their osmotic pressure (GRD-A10).
        for t in &self.tank.tips {
            let c = self.to_dot(t.x, t.y);
            let ink = Ink::new(salt(t.salt), TIP);
            let sigma = tip_sigma(t.p, t.strength);
            self.dots
                .stipple_puff(c.0, c.1, sigma, TIP_PEAK, bead_salt(t.bead), ink);
        }
        // Jet threads, bright, then a dim afterglow, then gone.
        for j in &self.tank.jets {
            if j.age >= JET_S {
                continue;
            }
            let level = if j.age < JET_BRIGHT_S { FRESH } else { LOOSE };
            let (a, b) = (
                self.to_dot(j.from[0], j.from[1]),
                self.to_dot(j.to[0], j.to[1]),
            );
            self.dots
                .line(a.0, a.1, b.0, b.1, Ink::new(salt(j.salt), level));
        }
        for p in self.tank.particles.iter() {
            match p.kind {
                Kind::Flake => {
                    let (x, y) = self.to_dot(p.x, p.y);
                    self.dots.plot(x, y, FLAKE_INK);
                }
                Kind::Smoke
                    if p.life >= SMOKE_LIFE
                        && hash01(u64::from(p.tag) ^ SMOKE_SALT) < SMOKE_DRAWN =>
                {
                    let ink = if p.life > WARM_LIFE {
                        WARM_INK
                    } else {
                        SMOKE_INK
                    };
                    let (x, y) = self.to_dot(p.x, p.y);
                    let (x0, y0) =
                        self.to_dot(p.x - p.vx * SMOKE_TRAIL_S, p.y - p.vy * SMOKE_TRAIL_S);
                    self.dots.line(x0, y0, x, y, ink);
                }
                // Droplets are the jet thread; bubbles are simulated, not drawn.
                _ => {}
            }
        }
    }
}

impl Generator for Garden {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.time).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.dots.resize(size);
        // The tank fills the rows above the title row.
        self.span = (
            f64::from(size.width()) * f64::from(Dots::CELL_W),
            f64::from(size.height().saturating_sub(1).max(1)) * f64::from(Dots::CELL_H),
        );
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
        self.dots.blit(canvas);
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
                let _ = burst(&mut t, bursts + 1, &mut rng);
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

    /// Run a garden at `size` (seed 7) to each of `times` and hand over the frame.
    fn frames(size: Size, times: &[f64], mut f: impl FnMut(f64, &Garden, &Canvas)) {
        let mut ctx = Ctx::new(7, size);
        let mut g = Garden::default();
        g.resize(size, &mut ctx);
        for &t in times {
            while g.time < t {
                g.update(1.0 / 60.0, &mut ctx);
            }
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            f(t, &g, &canvas);
        }
    }

    #[test]
    // GRD-A7 (M15 stepped capsule tubes) is superseded by GRD-A8..A11 (M18 bead chains);
    // its tank-coverage band (0.2–20 %) carries over here, on lit dots.
    fn grd_a7_a8_braille_only_and_a_sparse_tank() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            frames(size, &[5.0, 20.0, 40.0], |t, g, canvas| {
                let (mut lit_cells, mut blocks) = (0, 0);
                for c in canvas.cells() {
                    if c.ch == ' ' && c.bg == Ink::BLANK {
                        continue;
                    }
                    lit_cells += 1;
                    if c.bg != Ink::BLANK || matches!(c.ch, '▀' | '▄' | '█') {
                        blocks += 1;
                    }
                }
                assert!(
                    f64::from(blocks) <= 0.03 * f64::from(lit_cells),
                    "{size:?} t {t}: {blocks} half-block cells of {lit_cells}"
                );
                let (dw, dh) = (g.span.0 as i32, g.span.1 as i32);
                let lit = (0..dh)
                    .flat_map(|y| (0..dw).map(move |x| (x, y)))
                    .filter(|&(x, y)| g.dots.is_lit(x, y))
                    .count();
                let share = lit as f64 / f64::from(dw * dh);
                assert!(
                    share > 0.002 && share < 0.2,
                    "{size:?} t {t}: lit dots {share:.4}"
                );
            });
        }
    }

    #[test]
    fn grd_a9_every_bead_has_one_parent_and_branches_have_two_children() {
        use std::collections::HashMap;
        for vent in [false, true] {
            let mut rng = SplitMix64::new(9);
            let mut tank = Tank::default();
            tank.plant(vent, &mut rng);
            let mut branched = 0;
            for _ in 0..(20 * 60) {
                if !tank.step(1.0 / 60.0, &mut rng) {
                    break; // full: the scene would start over (GRD-A3)
                }
                let mut known: HashMap<u32, usize> = tank.seed_ids[..tank.n_seeds]
                    .iter()
                    .map(|&id| (id, 0))
                    .collect();
                for s in &tank.segments {
                    assert!(known.insert(s.id, 0).is_none(), "bead ids are unique");
                }
                for s in &tank.segments {
                    let n = known.get_mut(&s.parent).expect("every bead has its parent");
                    *n += 1;
                }
                let seeds = &tank.seed_ids[..tank.n_seeds];
                for (id, &n) in &known {
                    if !seeds.contains(id) {
                        assert!(n <= 2, "bead {id}: {n} children");
                        branched += usize::from(n == 2);
                    }
                }
                // Every tip sits on a bead or a seed.
                assert!(tank.tips.iter().all(|t| known.contains_key(&t.bead)));
            }
            if !vent {
                assert!(branched > 0, "the garden branches");
            }
        }
    }

    #[test]
    fn grd_a10_a_tip_swells_with_pressure_and_shrinks_after_bursting() {
        let mut rng = SplitMix64::new(10);
        let mut t = Tank::new_tip(0.5, FLOOR_Y, false, &mut rng);
        let mut last = tip_sigma(t.p, t.strength);
        let mut bursts = 0;
        for k in 0..600 {
            if charge(&mut t, 1.0 / 60.0) {
                let before = tip_sigma(t.p, t.strength);
                let _ = burst(&mut t, k, &mut rng);
                let after = tip_sigma(t.p, t.strength);
                assert!(after < before, "σ {before:.2} → {after:.2} on bursting");
                assert_eq!(after, TIP_SIGMA.0);
                bursts += 1;
            } else {
                let s = tip_sigma(t.p, t.strength);
                assert!(s >= last, "σ rises with pressure: {last:.3} → {s:.3}");
            }
            last = tip_sigma(t.p, t.strength);
        }
        assert!(bursts >= 3);
        assert_eq!(tip_sigma(0.0, 1.0), 0.7);
        assert_eq!(tip_sigma(1.0, 1.0), 1.6);
    }

    #[test]
    fn grd_a11_beads_in_salt_steps_and_bonds_dimmer() {
        // Sticks are never brighter than their bead, and dimmer under the tip and old
        // beads (round 2: fixed legible steps 200 / 200 / 110 / 60).
        for level in [TIP, FRESH, OLD, FADING] {
            assert!(bond_level(level) <= level, "bond under {level}");
        }
        assert!(bond_level(TIP) < TIP && bond_level(OLD) < OLD);
        assert_eq!(
            [TIP, FRESH, OLD, FADING].map(bond_level),
            [200, 200, 110, 60]
        );
        let s = |age: f32, fade: f32| Segment {
            a: [0.0; 2],
            b: [0.0; 2],
            w: 0.01,
            age,
            salt: 0,
            fade,
            id: 1,
            parent: 0,
        };
        assert_eq!(bead_level(&s(1.0, 1.0)), FRESH);
        assert_eq!(bead_level(&s(9.0, 1.0)), OLD);
        assert_eq!(bead_level(&s(1.0, 0.8)), OLD);
        assert_eq!(bead_level(&s(1.0, 0.3)), FADING);
        // Every lit cell is a salt at a bead or bond step, the floor, a flake, a jet or
        // the vent's fluid.
        let mut levels: Vec<u8> = STEPS.to_vec();
        levels.extend(STEPS.iter().map(|&l| bond_level(l)));
        levels.push(LOOSE);
        frames(Size::new(200, 50), &[5.0, 20.0, 40.0], |t, _, canvas| {
            for c in canvas.cells() {
                if c.ch == ' ' || c.ink == Ink::BLANK || c.ink.role == Role::Text {
                    continue;
                }
                let ok = [FLOOR_INK, FLAKE_INK, SMOKE_INK, WARM_INK].contains(&c.ink)
                    || (SALTS.contains(&c.ink.role) && levels.contains(&c.ink.level));
                assert!(ok, "t {t}: {:?}", c.ink);
            }
        });
    }
}
