//! Hydrogen and oxygen in a vessel below and above the first explosion limit: radicals
//! wander and die on the walls, until the pressure is high enough that branching outruns
//! the walls, the population doubles again and again and the vessel flashes pale blue
//! (SPEC-002 `radical_chain`).
//!
//! ## Model
//!
//! Radicals (a pool of [`CAPACITY`]) random-walk in a rounded vessel. Per step each one
//!
//! - **branches** (H + O₂ → OH + O, O + H₂ → OH + H: one radical becomes two) with rate
//!   `k_b = K_B0 · fuel · exp(E_A (1 − 1/T))`, Arrhenius in the local temperature `T`
//!   (units of the wall temperature);
//! - **terminates at the wall** on contact (the first explosion limit's mechanism), or
//! - **recombines in the gas** at `K_T3 · p` (three-body) plus `K_RR · n/CAPACITY`
//!   (radical–radical, the flame's own quench), leaving water.
//!
//! The diffusion coefficient falls as `1/p` (mean free path), so at low pressure the
//! walls catch radicals faster than they branch and nothing happens; at `p = 1` (the
//! limit) wall loss `D π² (1 + 1/A²)` of the vessel's slowest mode equals `K_B0`. Heat
//! from each event goes into a coarse temperature grid (ADR-0012) that relaxes toward the
//! wall and diffuses, feeding back into `k_b`: thermal runaway. Fuel is consumed per
//! branching, so the flash burns itself out; the vessel is then vented and refilled.
//!
//! Pressure follows a looping [`Timeline`] in simulated seconds: slow reaction below the
//! limit, a rise across it, the explosion, a vent. Captions follow the state.

use std::fmt::Write as _;

use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::{self, STROKE_KERNEL};
use sciart_core::title;
use sciart_core::{Tone, draw, ease, ink};

pub static META: Meta = Meta {
    name: "radical_chain",
    category: Category::Chemistry,
    description: "Hydrogen-oxygen chain branching: radicals die on the walls below the explosion limit and multiply into a flash above it",
    title: "Explosion · radicals multiply",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(RadicalChain::default())
}

const CAPTIONS: [&str; 3] = ["slow reaction", "chain branching", "explosion"];

/// Radical pool size (the flash saturates it).
pub(crate) const CAPACITY: usize = 4000;
/// Branching rate at the wall temperature with full fuel (per simulated second).
pub(crate) const K_B0: f64 = 3.0;
/// Activation energy in units of the wall temperature.
const E_A: f64 = 4.0;
/// Three-body recombination per unit pressure and radical–radical recombination at a
/// full pool (per second).
const K_T3: f64 = 0.15;
const K_RR: f64 = 4.0;
/// Initiation (H₂ + O₂ → HO₂ + H) rate at full fuel, radicals per second.
const K_INIT: f64 = 6.0;
/// Vessel aspect (width/height, vessel units) the limit is calibrated for.
const A_REF: f64 = 1.8;
/// Vessel corner radius, in vessel heights.
const CORNER: f64 = 0.22;
/// Branchings one vessel of fuel allows.
const FUEL: f64 = 60_000.0;
/// Temperature grid cap (ADR-0012), relaxation toward the wall and diffusion
/// (cells²/s), and heat per branching and per recombination (grid-cell units).
const MAX_CELLS: usize = 3000;
const BUCKET: f64 = 1.25;
const K_COOL: f64 = 2.0;
const T_DIFF: f32 = 6.0;
const HEAT_BRANCH: f32 = 0.06;
const HEAT_RECOMB: f32 = 0.10;
/// Cap on the temperature (units of the wall's): keeps the Arrhenius factor finite.
const T_MAX: f32 = 6.0;
/// Water haze decay while venting (per second).
const VENT_RATE: f32 = 1.5;
/// Longest internal step (the runtime's dt is 1/60 s).
const MAX_H: f64 = 1.0 / 60.0;
const MAX_SUBSTEPS: usize = 15;

/// Timeline: (label, seconds); pressure in units of the first explosion limit.
const PHASES: [(&str, f64); 4] = [("slow", 2.5), ("rise", 1.5), ("burn", 10.0), ("vent", 6.0)];
const P_LOW: f64 = 0.6;
const P_HIGH: f64 = 2.2;

const FLAME: Ramp = Ramp::Ice;
const WATER: Ramp = Ramp::Starlight;
const WATER_MAX: f64 = 80.0 / 255.0;
/// Radicals in the vessel at the start, so the first frame already has sparks.
const SEED_RADICALS: usize = 25;
/// Ice level of the gas at the wall temperature: a faint ambient so the vessel never
/// reads as empty.
const AMBIENT: u8 = 10;
/// Flame levels below this are drawn in violet (`Ramp(Plasma)`), the flame's cool
/// fringe; above it the pale blue of `FLAME` rises to a white-hot core.
const VIOLET_TOP: u8 = 45;
/// The ignition spark's temperature excess and radius (grid cells), and where in the
/// vessel it may strike (fractions of the width and height from the centre).
const SPARK_HEAT: f32 = 1.6;
const SPARK_R: f32 = 2.5;
const SPARK_SPREAD: f64 = 0.3;
/// Glow kept per frame: short fading trails behind the sparks.
const TRAIL_KEEP: f32 = 0.72;
/// Spark radius (dots) while the pool is sparse; single dots once it crowds.
const DOT_R: f64 = 0.9;
const SPARSE: usize = 300;
const OUTSIDE: u8 = 0;
const ON_WALL: u8 = 1;
const INSIDE: u8 = 2;
const WALL: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 150);
const RADICAL: Ink = Ink::new(Role::Ramp(Ramp::Ice), 255);
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// Pressure (units of the first limit) at a beat of the loop.
pub(crate) fn pressure(beat: &Beat) -> f64 {
    match beat.index {
        0 => P_LOW,
        1 => ease::lerp(P_LOW, P_HIGH, ease::smoothstep(beat.progress)),
        2 => P_HIGH,
        _ => ease::lerp(P_HIGH, P_LOW, ease::smoothstep(beat.progress)),
    }
}

/// Diffusion coefficient (vessel heights²/s) at pressure `p`: wall loss of the slowest
/// mode of an `A_REF` vessel equals `K_B0` at `p = 1`.
pub(crate) fn diffusion(p: f64) -> f64 {
    let d_limit = K_B0 / (std::f64::consts::PI.powi(2) * (1.0 + 1.0 / (A_REF * A_REF)));
    d_limit / p.max(0.05)
}

/// The vessel: a rounded rectangle `[0, a] × [0, 1]` in vessel units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Vessel {
    pub a: f64,
}

impl Vessel {
    /// Signed distance to the wall (negative inside).
    pub(crate) fn sdf(&self, x: f64, y: f64) -> f64 {
        let r = CORNER.min(0.5 * self.a).min(0.5);
        let qx = (x - 0.5 * self.a).abs() - (0.5 * self.a - r);
        let qy = (y - 0.5).abs() - (0.5 - r);
        let (ox, oy) = (qx.max(0.0), qy.max(0.0));
        (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
    }

    pub(crate) fn inside(&self, x: f64, y: f64) -> bool {
        self.sdf(x, y) < 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Radical {
    pub x: f64,
    pub y: f64,
    /// Position before the last step, for the trail.
    pub px: f64,
    pub py: f64,
}

impl Radical {
    pub(crate) fn at(x: f64, y: f64) -> Self {
        Self { x, y, px: x, py: y }
    }
}

/// Rates of one step, for the model and its tests.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rates {
    /// Branching rate at the wall temperature with full fuel.
    pub k_b0: f64,
    /// Gas termination rate (per second) on top of radical–radical recombination.
    pub k_t: f64,
    pub diff: f64,
    /// Whether events heat the gas.
    pub heat: bool,
}

/// Radicals, temperature, water and fuel.
#[derive(Debug, Clone)]
pub(crate) struct Chain {
    pub vessel: Vessel,
    pub pool: ParticlePool<Radical>,
    births: Vec<Radical>,
    pub temp: Grid<f32>,
    pub water: Grid<f32>,
    scratch: Grid<f32>,
    lap: Grid<f32>,
    pub fuel: f64,
    /// Branching attempts refused because the pool was full (CHN-A4).
    pub refused: u64,
    init_acc: f64,
}

impl Chain {
    pub(crate) fn new(vessel: Vessel, grid: Size) -> Self {
        Self {
            vessel,
            pool: ParticlePool::with_capacity(CAPACITY),
            births: Vec::with_capacity(CAPACITY),
            temp: Grid::new(grid, 1.0),
            water: Grid::new(grid, 0.0),
            scratch: Grid::new(grid, 0.0),
            lap: Grid::new(grid, 0.0),
            fuel: 1.0,
            refused: 0,
            init_acc: 0.0,
        }
    }

    /// A uniform point inside the vessel (rejection sampling, ≤ 16 tries).
    fn random_point(&self, rng: &mut SplitMix64) -> Option<Radical> {
        for _ in 0..16 {
            let (x, y) = (rng.next_f64() * self.vessel.a, rng.next_f64());
            if self.vessel.inside(x, y) {
                return Some(Radical::at(x, y));
            }
        }
        None
    }

    /// Spawn initiation radicals for `h` seconds.
    pub(crate) fn initiate(&mut self, h: f64, rng: &mut SplitMix64) {
        self.init_acc += K_INIT * self.fuel * h;
        while self.init_acc >= 1.0 {
            self.init_acc -= 1.0;
            if let Some(r) = self.random_point(rng) {
                self.pool.spawn(r);
            }
        }
    }

    /// One step of `h` seconds: walk, wall loss, gas termination, branching.
    pub(crate) fn step(&mut self, h: f64, rates: Rates, rng: &mut SplitMix64) {
        // Uniform steps of variance 2Dh per axis: no libm call per radical.
        let half = (6.0 * rates.diff * h).sqrt();
        let n_frac = self.pool.len() as f64 / CAPACITY as f64;
        let p_term = ((rates.k_t + K_RR * n_frac) * h).min(1.0);
        let (vessel, fuel) = (self.vessel, self.fuel);
        let (gw, gh) = (
            usize::from(self.temp.width()),
            usize::from(self.temp.height()),
        );
        let cell = |x: f64, y: f64| {
            let gx = ((x / vessel.a) * gw as f64).clamp(0.0, (gw - 1) as f64) as usize;
            let gy = (y * gh as f64).clamp(0.0, (gh - 1) as f64) as usize;
            gy * gw + gx
        };
        let temp = self.temp.as_mut_slice();
        let water = self.water.as_mut_slice();
        let births = &mut self.births;
        births.clear();
        let mut branched = 0u64;
        self.pool.retain(|r| {
            (r.px, r.py) = (r.x, r.y);
            r.x += half * (2.0 * rng.next_f64() - 1.0);
            r.y += half * (2.0 * rng.next_f64() - 1.0);
            if !vessel.inside(r.x, r.y) {
                return false;
            }
            let c = cell(r.x, r.y);
            if rng.next_f64() < p_term {
                water[c] += 1.0;
                if rates.heat {
                    temp[c] = (temp[c] + HEAT_RECOMB).min(T_MAX);
                }
                return false;
            }
            let t = f64::from(temp[c]).max(0.1);
            let k_b = rates.k_b0 * fuel * (E_A * (1.0 - 1.0 / t)).exp();
            if rng.next_f64() < (k_b * h).min(1.0) {
                branched += 1;
                water[c] += 0.5;
                if rates.heat {
                    temp[c] = (temp[c] + HEAT_BRANCH).min(T_MAX);
                }
                if births.len() < births.capacity() {
                    births.push(*r);
                }
            }
            true
        });
        for &b in &self.births {
            if !self.pool.spawn(b) {
                self.refused += 1;
            }
        }
        self.fuel = (self.fuel - branched as f64 / FUEL).max(0.0);
    }

    /// Relax the temperature toward the wall's and diffuse it for `h` seconds.
    pub(crate) fn cool(&mut self, h: f64) {
        let k = (1.0 - (-K_COOL * h).exp()) as f32;
        grid::laplacian9(&self.temp, &mut self.lap, Edges::Clip);
        let d = (T_DIFF * h as f32).min(0.2);
        for (t, l) in self.temp.as_mut_slice().iter_mut().zip(self.lap.as_slice()) {
            let v = *t + d * l;
            *t = (v + (1.0 - v) * k).clamp(0.0, T_MAX);
        }
    }

    pub(crate) fn max_temp(&self) -> f32 {
        self.temp.as_slice().iter().copied().fold(1.0, f32::max)
    }

    /// Carry the grids onto a new lattice and the radicals into a vessel of aspect `a`.
    fn regrid(&mut self, vessel: Vessel, size: Size) {
        let sx = vessel.a / self.vessel.a;
        self.vessel = vessel;
        for r in self.pool.iter_mut() {
            r.x *= sx;
        }
        self.pool.retain(|r| vessel.inside(r.x, r.y));
        if self.temp.size() != size {
            grid::regrid_bilinear(&mut self.temp, size, &mut self.scratch);
            grid::regrid_bilinear(&mut self.water, size, &mut self.scratch);
            self.lap.resize(size, 0.0);
        }
    }
}

/// Where the vessel sits on the pixel buffer.
#[derive(Debug, Clone, Copy, Default)]
struct Frame {
    x0: i32,
    y0: i32,
    /// Pixels per vessel unit.
    scale: f64,
    w: i32,
    h: i32,
}

/// The flame's ink for a toned level: violet fringe, pale blue body, white-hot core.
pub(crate) fn flame_ink(lv: u8) -> Ink {
    if lv < VIOLET_TOP {
        Ink::new(Role::Ramp(Ramp::Plasma), lv)
    } else {
        Ink::new(Role::Ramp(FLAME), lv)
    }
}

/// Strike the ignition spark: a Gaussian hot spot at a random point near the centre.
fn spark(chain: &mut Chain, rng: &mut SplitMix64) {
    let (w, h) = (
        usize::from(chain.temp.width()),
        usize::from(chain.temp.height()),
    );
    let jitter = |rng: &mut SplitMix64| (rng.next_f64() - 0.5) * 2.0 * SPARK_SPREAD;
    let cx = (0.5 + 0.5 * jitter(rng)) * w as f64;
    let cy = (0.5 + 0.5 * jitter(rng)) * h as f64;
    for (i, t) in chain.temp.as_mut_slice().iter_mut().enumerate() {
        let (dx, dy) = ((i % w.max(1)) as f64 - cx, (i / w.max(1)) as f64 - cy);
        let g = (-(dx * dx + dy * dy) as f32 / (SPARK_R * SPARK_R)).exp();
        *t = (*t + SPARK_HEAT * g).min(T_MAX);
    }
}

#[derive(Debug)]
pub struct RadicalChain {
    size: Size,
    chain: Option<Chain>,
    rng: SplitMix64,
    schedule: Timeline,
    t: f64,
    cycle: i64,
    /// Latched once the flash comes in the current loop.
    exploded: bool,
    /// The loop whose ignition spark has been struck.
    sparked: i64,
    frame: Frame,
    pixels: Pixels,
    dots: Dots,
    glow: Glow,
    tone: Tone,
    map: SampleMap,
    readout_room: usize,
    /// Per pixel: outside, on the wall or inside the vessel (built in `resize`).
    mask: Vec<u8>,
    /// Dots of the vessel outline (built in `resize`).
    wall_dots: Vec<(i32, i32)>,
}

impl Default for RadicalChain {
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            chain: None,
            rng: SplitMix64::new(0),
            schedule: Timeline::new(&PHASES),
            t: 0.0,
            cycle: 0,
            exploded: false,
            sparked: -1,
            frame: Frame::default(),
            pixels: Pixels::new(Size::new(1, 1)),
            dots: Dots::new(Size::new(1, 1)),
            glow: Glow::new(Size::new(1, 1)),
            tone: Tone::asinh(0.6, 12.0),
            map: SampleMap::default(),
            readout_room: 0,
            mask: Vec::new(),
            wall_dots: Vec::new(),
        }
    }
}

impl RadicalChain {
    fn beat(&self) -> Beat {
        self.schedule.at(self.t)
    }

    /// Vessel placement for a display: 2 columns of margin, 1 row on top, the title row
    /// and one more below.
    fn frame_for(size: Size) -> Frame {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let x0 = if w > 12 { 2 } else { 0 };
        let y0 = if h > 6 { 2 } else { 0 };
        let pw = (w - 2 * x0).max(1);
        let ph = (2 * h - y0 - if h > 6 { 4 } else { 0 }).max(1);
        Frame {
            x0,
            y0,
            scale: f64::from(ph),
            w: pw,
            h: ph,
        }
    }

    fn rates(&self, p: f64) -> Rates {
        Rates {
            k_b0: K_B0,
            k_t: K_T3 * p,
            diff: diffusion(p),
            heat: true,
        }
    }

    fn rasterise(&mut self) {
        let Some(chain) = &self.chain else {
            return;
        };
        let f = self.frame;
        self.glow.decay(TRAIL_KEEP);
        self.dots.clear();
        let full = chain.pool.len() as f32 / CAPACITY as f32;
        // Many radicals means many overlapping splats: dim each so the flash stays graded.
        let scale = 1.0 / (1.0 + 3.0 * full);
        for r in chain.pool.iter() {
            let (px, py) = (
                f64::from(f.x0) + r.x * f.scale,
                f64::from(f.y0) + r.y * f.scale,
            );
            // Sparse: a stroke from the last position draws a fading trail. Crowded (the
            // flash), the glow's own persistence (TRAIL_KEEP) is trail enough and a splat
            // keeps the frame cheap.
            if chain.pool.len() < SPARSE {
                let (qx, qy) = (
                    f64::from(f.x0) + r.px * f.scale,
                    f64::from(f.y0) + r.py * f.scale,
                );
                let a = (2.0 * qx, 2.0 * qy);
                self.glow
                    .stroke(a, (2.0 * px, 2.0 * py), &STROKE_KERNEL, 2.0 * scale);
            } else {
                self.glow.splat(2.0 * px, 2.0 * py, &STROKE_KERNEL, scale);
            }
            // Keep the wall line clean: a radical touching it is about to die there. In
            // the crowd of the flash single dots would only speckle the flame, so only
            // the glow shows them.
            if chain.pool.len() < SPARSE && chain.vessel.sdf(r.x, r.y) < -1.0 / f.scale {
                self.dots.disc(2.0 * px, 2.0 * py, DOT_R, RADICAL);
            }
        }
        for &(x, y) in &self.wall_dots {
            self.dots.set(x, y, WALL);
        }
        let water_top = chain
            .water
            .as_slice()
            .iter()
            .copied()
            .fold(40.0f32, f32::max);
        // Skip the per-pixel bilinear samples of fields that are flat this frame.
        let (hot, wet) = (
            chain.max_temp() > 1.002,
            chain.water.as_slice().iter().any(|&w| w > 0.0),
        );
        let (map, glow, tone, mask) = (&self.map, &self.glow, &self.tone, &self.mask);
        let pw = i32::from(self.size.width());
        self.pixels.shade(|x, y| {
            match mask.get((y * pw + x) as usize) {
                Some(&INSIDE) => {}
                _ => return Ink::BLANK,
            }
            let (lx, ly) = (x - f.x0, y - f.y0);
            let heat = if hot {
                map.sample(&chain.temp, lx, ly) - 1.0
            } else {
                0.0
            };
            let light = glow.get(x, y).unwrap_or(0.0) + 3.0 * heat.max(0.0);
            let lv = tone.level(light);
            let wv = if wet {
                map.sample(&chain.water, lx, ly) / water_top
            } else {
                0.0
            };
            let water = if wv > 0.004 {
                ink::ramp(WATER, WATER_MAX * f64::from(wv.sqrt()))
            } else {
                Ink::BLANK
            };
            raster::stronger(flame_ink(lv.max(AMBIENT)), water)
        });
    }

    fn readout(&self, canvas: &mut Canvas) {
        let Some(chain) = &self.chain else {
            return;
        };
        let n = chain.pool.len();
        let p = (pressure(&self.beat()) * 100.0).round() as u32;
        let mut text = StackText::<32>::new();
        let _ = match self.readout_room {
            26.. => write!(text, "p={}.{:02} limit  radicals {n}", p / 100, p % 100),
            13.. => write!(text, "radicals {n}"),
            6.. => write!(text, "n={n}"),
            _ => return,
        };
        let y = i32::from(self.size.height()) - 1;
        draw::label_right(canvas, y, text.as_str(), READOUT_INK);
    }
}

impl Generator for RadicalChain {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        let beat = self.beat();
        Some(if self.exploded {
            CAPTIONS[2]
        } else if beat.index == 0 || pressure(&beat) < 1.0 {
            CAPTIONS[0]
        } else {
            CAPTIONS[1]
        })
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.frame = Self::frame_for(size);
        let f = self.frame;
        let vessel = Vessel {
            a: (f64::from(f.w) / f64::from(f.h)).clamp(0.3, 6.0),
        };
        let grid = grid::lattice_for(Size::new(f.w as u16, f.h as u16), MAX_CELLS, BUCKET);
        match &mut self.chain {
            None => {
                self.rng = SplitMix64::from_rng(&mut ctx.rng);
                let mut chain = Chain::new(vessel, grid);
                // A few radicals from the start, so the first frame is not empty.
                for _ in 0..SEED_RADICALS {
                    if let Some(r) = chain.random_point(&mut self.rng) {
                        chain.pool.spawn(r);
                    }
                }
                self.chain = Some(chain);
            }
            Some(chain) => chain.regrid(vessel, grid),
        }
        self.pixels.resize(size);
        self.dots.resize(size);
        self.glow.resize(size);
        self.map.rebuild(grid, Size::new(f.w as u16, f.h as u16));
        let px = self.pixels.pixel_size();
        let (pw, ph) = (i32::from(px.width()), i32::from(px.height()));
        let wall = 0.9 / f.scale;
        self.mask.clear();
        self.mask.extend(
            (0..ph)
                .flat_map(|y| (0..pw).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let d = vessel.sdf(
                        (f64::from(x - f.x0) + 0.5) / f.scale,
                        (f64::from(y - f.y0) + 0.5) / f.scale,
                    );
                    // The whole pixel inside (half-diagonal ≈ 0.71 px), so the gas never
                    // pokes past the one-dot outline at the rounded corners.
                    if d < -0.75 / f.scale {
                        INSIDE
                    } else if d < wall {
                        ON_WALL
                    } else {
                        OUTSIDE
                    }
                }),
        );
        // The wall: every dot whose centre lies within ~0.4 dot of the vessel's outline,
        // a crisp one-dot braille line.
        let dots = self.dots.dot_size();
        let band = 0.3 / f.scale;
        self.wall_dots.clear();
        for y in 0..i32::from(dots.height()) {
            for x in 0..i32::from(dots.width()) {
                let d = vessel.sdf(
                    ((f64::from(x) + 0.5) / 2.0 - f64::from(f.x0)) / f.scale,
                    ((f64::from(y) + 0.5) / 2.0 - f64::from(f.y0)) / f.scale,
                );
                if d.abs() <= band {
                    self.wall_dots.push((x, y));
                }
            }
        }
        self.readout_room = title::stable_readout_room(size, &META);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.chain.is_none() {
            return;
        }
        self.t += dt;
        let beat = self.beat();
        let cycle = self.schedule.cycle(self.t);
        let p = pressure(&beat);
        let rates = self.rates(p);
        let steps = ((dt / MAX_H).ceil() as usize).clamp(1, MAX_SUBSTEPS);
        let h = dt / steps as f64;
        let Some(chain) = &mut self.chain else {
            return;
        };
        if cycle != self.cycle {
            // Fresh gas: the vessel was vented and refilled.
            self.cycle = cycle;
            self.exploded = false;
            chain.fuel = 1.0;
            chain.water.fill(0.0);
        }
        if beat.index >= 1 && self.sparked != cycle {
            // The ignition spark: one hot spot off-centre, so the runaway starts there and
            // the flame front propagates across the vessel instead of flaring everywhere.
            self.sparked = cycle;
            spark(chain, &mut self.rng);
        }
        for _ in 0..steps {
            chain.initiate(h, &mut self.rng);
            chain.step(h, rates, &mut self.rng);
            chain.cool(h);
        }
        if beat.index == 3 {
            let k = (-VENT_RATE * dt as f32).exp();
            chain.water.as_mut_slice().iter_mut().for_each(|w| *w *= k);
        }
        if chain.pool.len() > CAPACITY / 4 || chain.max_temp() > 1.5 + SPARK_HEAT {
            self.exploded = true;
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels.blit(canvas, |_, _, top, bottom| {
            raster::ascii_ramp_cell(top, bottom)
        });
        self.dots.blit_over(canvas, &self.pixels);
        self.readout(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> SplitMix64 {
        SplitMix64::from_rng(&mut Ctx::new(seed, Size::new(1, 1)).rng)
    }

    fn chain_with(n: usize, seed: u64) -> (Chain, SplitMix64) {
        let mut r = rng(seed);
        let mut c = Chain::new(Vessel { a: A_REF }, Size::new(30, 16));
        for _ in 0..n {
            let p = c.random_point(&mut r).expect("inside");
            c.pool.spawn(p);
        }
        (c, r)
    }

    fn cold(k_b0: f64, p: f64) -> Rates {
        Rates {
            k_b0,
            k_t: K_T3 * p,
            diff: diffusion(p),
            heat: false,
        }
    }

    #[test]
    fn chn_a1_without_branching_the_count_never_increases() {
        let (mut c, mut r) = chain_with(500, 1);
        let mut last = c.pool.len();
        for _ in 0..600 {
            c.step(SIM_DT, cold(0.0, 1.0), &mut r);
            assert!(c.pool.len() <= last);
            last = c.pool.len();
        }
        assert!(last < 500);
    }

    /// Slope of ln(count) against time over `secs`, by least squares.
    fn growth_rate(p: f64, seed: u64, secs: f64) -> f64 {
        let (mut c, mut r) = chain_with(200, seed);
        let steps = (secs / SIM_DT) as usize;
        let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for k in 0..steps {
            c.step(SIM_DT, cold(K_B0, p), &mut r);
            c.fuel = 1.0;
            let len = c.pool.len();
            if k > 30 && len > 0 && len < CAPACITY / 3 {
                let (x, y) = (k as f64 * SIM_DT, (len as f64).ln());
                (sx, sy, sxx, sxy, n) = (sx + x, sy + y, sxx + x * x, sxy + x * y, n + 1.0);
            }
        }
        (n * sxy - sx * sy) / (n * sxx - sx * sx)
    }

    #[test]
    fn chn_a2_above_the_limit_the_count_grows_exponentially_and_below_it_decays() {
        for seed in 0..3 {
            let up = growth_rate(P_HIGH, seed, 3.0);
            let down = growth_rate(P_LOW, seed, 1.0);
            assert!(up > 0.3, "seed {seed}: growth {up}");
            assert!(down < -0.5, "seed {seed}: decay {down}");
        }
    }

    #[test]
    fn chn_a3_a_radical_that_reaches_the_wall_is_removed() {
        let v = Vessel { a: A_REF };
        assert!(v.inside(0.9, 0.5) && !v.inside(-0.01, 0.5) && !v.inside(0.02, 0.02));
        assert!(v.sdf(0.9, 0.5) < 0.0 && v.sdf(0.9, 1.01) > 0.0);
        let mut c = Chain::new(v, Size::new(10, 6));
        c.pool.spawn(Radical::at(0.9, 0.999_999));
        c.pool.spawn(Radical::at(0.9, 0.5));
        let mut r = rng(4);
        // Huge diffusion: every step lands far outside, so both must go at once.
        let rates = Rates {
            k_b0: 0.0,
            k_t: 0.0,
            diff: 1e3,
            heat: false,
        };
        c.step(SIM_DT, rates, &mut r);
        assert!(c.pool.is_empty());
    }

    #[test]
    fn chn_a4_pool_never_exceeds_capacity_and_branching_stops_when_full() {
        let (mut c, mut r) = chain_with(3900, 5);
        let wild = Rates {
            k_b0: 500.0,
            k_t: 0.0,
            diff: 1e-4,
            heat: true,
        };
        for _ in 0..30 {
            c.step(SIM_DT, wild, &mut r);
            assert!(c.pool.len() <= CAPACITY);
        }
        assert!(c.refused > 0, "the full pool refused branchings");
        assert!(c.temp.as_slice().iter().all(|t| t.is_finite()));
    }

    #[test]
    fn chn_a5_temperature_stays_finite_and_relaxes_to_the_wall_after_the_flash() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = RadicalChain::default();
        g.resize(size, &mut ctx);
        let mut peak = 1.0f32;
        let mut exploded_at = None;
        for k in 0..(20.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let c = g.chain.as_ref().expect("chain");
            let m = c.max_temp();
            assert!(c.temp.as_slice().iter().all(|t| t.is_finite() && *t >= 0.0));
            peak = peak.max(m);
            if g.exploded && exploded_at.is_none() {
                exploded_at = Some(k);
            }
        }
        assert!(exploded_at.is_some(), "no flash in one loop");
        assert!(peak > 1.5, "the flash heats the gas (peak {peak})");
        // End of the vent: back to the wall temperature.
        let m = g.chain.as_ref().expect("chain").max_temp();
        assert!(m < 1.05, "T {m} after the vent");
    }

    #[test]
    fn chn_captions_follow_the_state() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(42, size);
        let mut g = RadicalChain::default();
        g.resize(size, &mut ctx);
        assert_eq!(g.caption(), Some("slow reaction"));
        let mut seen = [false; 3];
        for _ in 0..(20.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let c = g.caption().expect("caption");
            seen[CAPTIONS.iter().position(|&x| x == c).expect("listed")] = true;
        }
        assert_eq!(seen, [true; 3]);
    }
}
