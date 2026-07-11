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
//!   `k_b = K_B0 · p · fuel · exp(E_A (1 − 1/T))` (H + O₂ is a collision with O₂, so ∝ p),
//!   Arrhenius in the local temperature `T` (units of the wall temperature);
//! - **terminates at the wall** on contact (the first explosion limit's mechanism), or
//! - **recombines in the gas** at `K_T3 · p²` (three-body, H + O₂ + M: ∝ [O₂][M]) plus
//!   `K_RR · n/CAPACITY` (radical–radical, the flame's own quench), leaving water.
//!
//! The diffusion coefficient falls as `1/p` (mean free path), so at low pressure the
//! walls catch radicals faster than they branch and nothing happens; at `p = 1` (the
//! limit) wall loss `D π² (1 + 1/A²)` of the vessel's slowest mode equals `K_B0`. Above
//! it the radicals both branch faster and spread slower, so the flame front is thin and
//! slow (Fisher–KPP speed `2√(D k)`, width `√(D/k)`). Heat
//! from each event goes into a coarse temperature grid (ADR-0012) that relaxes toward the
//! wall and diffuses, feeding back into `k_b`: thermal runaway. Fuel is consumed per
//! branching, globally and in a coarse *unburnt gas* grid: behind the flame the gas is
//! spent, radicals there find nothing to branch on and recombine, so the explosion runs
//! as a front from the ignition spark (a Fisher–KPP pulse) rather than flaring
//! everywhere at once. The vessel is then vented and refilled.
//!
//! Pressure follows a looping [`Timeline`] in simulated seconds: slow reaction below the
//! limit, a rise across it, the explosion, a vent. Captions follow the state.

use std::fmt::Write as _;

use sciart_core::grid;
use sciart_core::prelude::*;
use sciart_core::rng::hash01;
use sciart_core::title;
use sciart_core::{draw, ease};

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
/// Branching rate at the wall temperature with full fuel at the limit pressure (per
/// simulated second; it scales with `p`).
pub(crate) const K_B0: f64 = 1.2;
/// Activation energy in units of the wall temperature.
const E_A: f64 = 4.0;
/// Three-body recombination per unit pressure squared (it overtakes branching, ∝ p, at
/// the second limit p = K_B0 / K_T3 = 12, far above the sweep) and radical–radical
/// recombination at a full pool (per second).
const K_T3: f64 = 0.1;
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
const K_COOL: f64 = 2.5;
const T_DIFF: f32 = 6.0;
const HEAT_BRANCH: f32 = 0.5;
const HEAT_RECOMB: f32 = 0.2;
/// Cap on the temperature (units of the wall's): keeps the Arrhenius factor finite.
const T_MAX: f32 = 6.0;
/// Water haze decay while venting (per second): it thins over the whole vent.
const VENT_RATE: f32 = 0.6;
/// Branchings that spend one grid cell's gas, and the extra recombination rate
/// (per second) in fully burnt gas.
const CELL_FUEL: f32 = 4.5;
const K_BURNT: f64 = 6.0;
/// Fresh gas flowing in once the vent has dropped below the limit (per second).
const REFILL: f64 = 1.5;
/// Radicals the ignition spark releases at its hot spot.
const SPARK_RADICALS: usize = 30;
/// Longest internal step (the runtime's dt is 1/60 s).
const MAX_H: f64 = 1.0 / 60.0;
const MAX_SUBSTEPS: usize = 15;

/// Timeline: (label, seconds); pressure in units of the first explosion limit.
/// M16: the burn is 5 s (was 6.5 s): with no drawn afterglow the vessel would sit
/// empty after the flame.
const PHASES: [(&str, f64); 4] = [("slow", 2.5), ("rise", 1.5), ("burn", 5.0), ("vent", 5.5)];
/// Index of the burn beat in [`PHASES`]: the spark strikes as it begins.
const BURN: usize = 2;
/// The vent drops the pressure over the first `1/VENT_FAST` of its beat, so fresh gas
/// (and its slow chains) is back well before the loop ends.
const VENT_FAST: f64 = 3.0;
const P_LOW: f64 = 0.6;
const P_HIGH: f64 = 2.2;

/// Radicals in the vessel at the start, so the first frame already has sparks.
const SEED_RADICALS: usize = 25;
/// The ignition spark's temperature excess and radius (grid cells), and where in the
/// vessel it may strike (fractions of the width and height from the centre).
const SPARK_HEAT: f32 = 1.6;
const SPARK_R: f32 = 2.5;
const SPARK_SPREAD: f64 = 0.3;
/// M16 (GEN-R25, strange_attractors' thread idiom): each radical is a bright `Sun` dot
/// heading a thread through its last `TRAIL_N` positions (sampled every `TRAIL_DT` s),
/// coloured by age: `Hot` 230 fresh, `Hot` 150, then a faint `Accent` 90 tail. A child is
/// born on its parent with the parent's trail, so every branching forks a thread.
pub(crate) const TRAIL_N: usize = 8;
const TRAIL_DT: f64 = 0.06;
/// Time constant of the drawn position's smoothing (s).
const SMOOTH_S: f64 = 0.12;
const THREAD_INKS: [Ink; 3] = [
    Ink::new(Role::Hot, 230),
    Ink::new(Role::Hot, 150),
    Ink::new(Role::Accent, 90),
];
const HEAD: Ink = Ink::new(Role::Sun, 255);
/// At most this many threads are drawn (a fixed hashed share of the pool beyond it).
pub(crate) const MAX_THREADS: usize = 240;
/// A radical lost at the wall: a brief `Cold` dot there for `DEATH_S` seconds (a ring of
/// `N_DEATHS`).
const DEATH: Ink = Ink::new(Role::Cold, 200);
const DEATH_S: f32 = 0.4;
const N_DEATHS: usize = 160;
/// Wall deaths one update records at most.
const MAX_WALL_HITS: usize = 512;
/// The wall: a faint dotted line, one dot in `WALL_EVERY`.
const WALL: Ink = Ink::new(Role::Dim, 90);
const WALL_EVERY: usize = 3;
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// Pressure (units of the first limit) at a beat of the loop.
pub(crate) fn pressure(beat: &Beat) -> f64 {
    match beat.index {
        0 => P_LOW,
        1 => ease::lerp(P_LOW, P_HIGH, ease::smoothstep(beat.progress)),
        2 => P_HIGH,
        _ => ease::lerp(
            P_HIGH,
            P_LOW,
            ease::smoothstep((beat.progress * VENT_FAST).min(1.0)),
        ),
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
    /// Position before the last step (the last point inside, for a wall death).
    pub px: f64,
    pub py: f64,
    /// Where it is drawn: its position smoothed over `SMOOTH_S` (a random walk drawn
    /// raw is a scribble; smoothed, a thread).
    pub shown: (f32, f32),
    /// Its past drawn positions, newest first (the thread, M16).
    pub trail: [(f32, f32); TRAIL_N],
    /// A stable identity: which share of the pool is drawn when it is crowded.
    pub id: u32,
}

impl Radical {
    pub(crate) fn at(x: f64, y: f64) -> Self {
        let id = (x.to_bits() ^ y.to_bits().rotate_left(29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        Self {
            x,
            y,
            px: x,
            py: y,
            shown: (x as f32, y as f32),
            trail: [(x as f32, y as f32); TRAIL_N],
            id: (id >> 32) as u32,
        }
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
    /// Unburnt share of the gas per cell (1 fresh, 0 spent).
    pub gas: Grid<f32>,
    scratch: Grid<f32>,
    lap: Grid<f32>,
    pub fuel: f64,
    /// Branching attempts refused because the pool was full (CHN-A4).
    pub refused: u64,
    init_acc: f64,
    /// Where radicals left through the wall since the scene last looked (their last
    /// point inside), at most `MAX_WALL_HITS`.
    pub wall_hits: Vec<(f64, f64)>,
    /// Births so far: mixed into each child's id.
    births_n: u32,
}

impl Chain {
    pub(crate) fn new(vessel: Vessel, grid: Size) -> Self {
        Self {
            vessel,
            pool: ParticlePool::with_capacity(CAPACITY),
            births: Vec::with_capacity(CAPACITY),
            temp: Grid::new(grid, 1.0),
            water: Grid::new(grid, 0.0),
            gas: Grid::new(grid, 1.0),
            scratch: Grid::new(grid, 0.0),
            lap: Grid::new(grid, 0.0),
            fuel: 1.0,
            refused: 0,
            init_acc: 0.0,
            wall_hits: Vec::with_capacity(MAX_WALL_HITS),
            births_n: 0,
        }
    }

    /// Ease every radical's drawn position towards it over `dt` seconds.
    pub(crate) fn smooth(&mut self, dt: f64) {
        let k = (1.0 - (-dt / SMOOTH_S).exp()) as f32;
        for r in self.pool.iter_mut() {
            r.shown.0 += (r.x as f32 - r.shown.0) * k;
            r.shown.1 += (r.y as f32 - r.shown.1) * k;
        }
    }

    /// Push every radical's drawn position onto its trail (newest first).
    pub(crate) fn sample_trails(&mut self) {
        for r in self.pool.iter_mut() {
            r.trail.copy_within(0..TRAIL_N - 1, 1);
            r.trail[0] = r.shown;
        }
    }

    /// Fresh gas everywhere.
    pub(crate) fn refuel(&mut self) {
        self.fuel = 1.0;
        self.gas.fill(1.0);
    }

    /// Let fresh gas flow in for `h` seconds (the vent's refill).
    fn refill(&mut self, h: f64) {
        let k = 1.0 - (-REFILL * h).exp();
        self.fuel += (1.0 - self.fuel) * k;
        let k = k as f32;
        self.gas
            .as_mut_slice()
            .iter_mut()
            .for_each(|g| *g += (1.0 - *g) * k);
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
        let gas = self.gas.as_mut_slice();
        let births = &mut self.births;
        births.clear();
        let hits = &mut self.wall_hits;
        let mut branched = 0u64;
        self.pool.retain(|r| {
            (r.px, r.py) = (r.x, r.y);
            r.x += half * (2.0 * rng.next_f64() - 1.0);
            r.y += half * (2.0 * rng.next_f64() - 1.0);
            if !vessel.inside(r.x, r.y) {
                if hits.len() < MAX_WALL_HITS {
                    hits.push((r.px, r.py));
                }
                return false;
            }
            let c = cell(r.x, r.y);
            let unburnt = gas[c];
            let p_die = p_term + K_BURNT * f64::from(1.0 - unburnt) * h;
            if rng.next_f64() < p_die {
                water[c] += 1.0;
                if rates.heat {
                    temp[c] = (temp[c] + HEAT_RECOMB).min(T_MAX);
                }
                return false;
            }
            let t = f64::from(temp[c]).max(0.1);
            let k_b = rates.k_b0 * fuel * f64::from(unburnt) * (E_A * (1.0 - 1.0 / t)).exp();
            if rng.next_f64() < (k_b * h).min(1.0) {
                branched += 1;
                gas[c] = (unburnt - 1.0 / CELL_FUEL).max(0.0);
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
            // The child shares its parent's past (the fork) under a new identity.
            self.births_n = self.births_n.wrapping_add(1);
            let id = (u64::from(b.id) << 32 | u64::from(self.births_n))
                .wrapping_mul(0xd1b5_4a32_d192_ed03);
            let b = Radical {
                id: (id >> 32) as u32,
                ..b
            };
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
            grid::regrid_bilinear(&mut self.gas, size, &mut self.scratch);
            self.lap.resize(size, 0.0);
        }
    }
}

/// Where the vessel sits, in half-block pixel units (a dot is half a pixel).
#[derive(Debug, Clone, Copy, Default)]
struct Frame {
    x0: i32,
    y0: i32,
    /// Pixels per vessel unit.
    scale: f64,
    w: i32,
    h: i32,
}

impl Frame {
    /// Dot coordinates of the vessel point `(x, y)`.
    fn dot(&self, x: f64, y: f64) -> (f64, f64) {
        (
            2.0 * (f64::from(self.x0) + x * self.scale),
            2.0 * (f64::from(self.y0) + y * self.scale),
        )
    }
}

/// The ink of a trail segment `k` samples old (0 the newest): hot and fresh near the
/// radical, cooling to a faint `Accent` at its tail (M16: colour along the thread by age).
pub(crate) fn thread_ink(k: usize) -> Ink {
    match k {
        0..=1 => THREAD_INKS[0],
        2..=4 => THREAD_INKS[1],
        _ => THREAD_INKS[2],
    }
}

/// Whether radical `id` is drawn with `n` alive: all of them while there are at most
/// `MAX_THREADS`, else a fixed hashed share, so a drawn thread stays drawn while the
/// population holds and the crowd of the flash stays a tree, not a wash.
pub(crate) fn drawn(id: u32, n: usize) -> bool {
    n <= MAX_THREADS || hash01(u64::from(id)) * n as f64 <= MAX_THREADS as f64
}

/// Strike the ignition spark: a Gaussian hot spot at a random point near the centre,
/// releasing a burst of radicals there. The gas's stray radicals are swept aside (the
/// ignition's own chains outrun them), so the flame grows from this one seed as a front.
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
    chain.pool.clear();
    let (x, y) = (cx / w.max(1) as f64 * chain.vessel.a, cy / h.max(1) as f64);
    let r = f64::from(SPARK_R) / h.max(1) as f64;
    for _ in 0..SPARK_RADICALS {
        let (dx, dy) = ((rng.next_f64() - 0.5) * r, (rng.next_f64() - 0.5) * r);
        if chain.vessel.inside(x + dx, y + dy) {
            chain.pool.spawn(Radical::at(x + dx, y + dy));
        }
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
    dots: Dots,
    readout_room: usize,
    /// Dots of the vessel outline (built in `resize`).
    wall_dots: Vec<(i32, i32)>,
    /// Recent wall deaths: vessel position and seconds left to show, a ring.
    deaths: [(f32, f32, f32); N_DEATHS],
    death_next: usize,
    /// Seconds since the trails were last sampled.
    trail_clock: f64,
    /// The dots are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
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
            dots: Dots::new(Size::new(1, 1)),
            readout_room: 0,
            wall_dots: Vec::new(),
            deaths: [(0.0, 0.0, 0.0); N_DEATHS],
            death_next: 0,
            trail_clock: 0.0,
            dirty: true,
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
            k_b0: K_B0 * p,
            k_t: K_T3 * p * p,
            diff: diffusion(p),
            heat: true,
        }
    }

    /// Age the wall deaths by `dt` and add the chain's new ones.
    fn record_deaths(&mut self, dt: f64) {
        for d in &mut self.deaths {
            d.2 = (d.2 - dt as f32).max(0.0);
        }
        let Some(chain) = &mut self.chain else {
            return;
        };
        for &(x, y) in &chain.wall_hits {
            self.deaths[self.death_next] = (x as f32, y as f32, DEATH_S);
            self.death_next = (self.death_next + 1) % N_DEATHS;
        }
        chain.wall_hits.clear();
    }

    /// The vessel on black (M16, GEN-R25): every drawn radical a bright `Sun` dot at the
    /// head of its thread (its last positions, coloured by age), so each branching forks
    /// a thread and the explosion grows as a tree; wall deaths brief `Cold` dots; the
    /// wall a faint dotted line.
    fn rasterise(&mut self) {
        let Some(chain) = &self.chain else {
            return;
        };
        let f = self.frame;
        self.dots.clear();
        for &(x, y) in &self.wall_dots {
            self.dots.set(x, y, WALL);
        }
        let n = chain.pool.len();
        for r in chain.pool.iter() {
            if !drawn(r.id, n) {
                continue;
            }
            let mut a = f.dot(f64::from(r.shown.0), f64::from(r.shown.1));
            for (k, &(tx, ty)) in r.trail.iter().enumerate() {
                let b = f.dot(f64::from(tx), f64::from(ty));
                self.dots.line(a.0, a.1, b.0, b.1, thread_ink(k));
                a = b;
            }
        }
        // Heads last: a cell holding a head shows the head's colour (one ink per cell).
        for r in chain.pool.iter() {
            if drawn(r.id, n) {
                let (x, y) = f.dot(f64::from(r.shown.0), f64::from(r.shown.1));
                self.dots.plot(x, y, HEAD);
            }
        }
        for &(x, y, life) in &self.deaths {
            if life > 0.0 {
                let (x, y) = f.dot(f64::from(x), f64::from(y));
                self.dots.disc(x, y, 0.8, DEATH);
            }
        }
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

    /// The caption follows the state (CHN-A9): below the limit the chains die out
    /// (`slow reaction`); above it they multiply (`chain branching`) until the explosion
    /// has latched (`explosion`), which lasts until the vent takes p back below the limit.
    fn caption(&self) -> Option<&'static str> {
        Some(if pressure(&self.beat()) < 1.0 {
            CAPTIONS[0]
        } else if self.exploded {
            CAPTIONS[2]
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
        self.dots.resize(size);
        // The wall: every dot whose centre lies within ~0.3 px of the vessel's outline,
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
        // Every `WALL_EVERY`-th dot along the outline: a faint dotted wall.
        let mut k = 0;
        self.wall_dots.retain(|_| {
            k += 1;
            k % WALL_EVERY == 0
        });
        self.readout_room = title::stable_readout_room(size, &META);
        self.dirty = true;
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
            chain.refuel();
            chain.water.fill(0.0);
        }
        if beat.index == BURN && self.sparked != cycle {
            // The ignition spark, at the start of the burn (so the explosion caption and
            // the flash coincide): one hot spot off-centre, so the runaway starts there and
            // the flame front propagates across the vessel instead of flaring everywhere.
            self.sparked = cycle;
            spark(chain, &mut self.rng);
        }
        // Initiation is slow chemistry: it seeds the slow phase and the refilled vessel,
        // but in the rise and the burn only the spark's chains run, so the flame is a
        // front.
        let venting = beat.index == 3 && p < 1.0;
        let initiating = beat.index == 0 || venting;
        for _ in 0..steps {
            if initiating {
                chain.initiate(h, &mut self.rng);
            }
            if venting {
                chain.refill(h);
            }
            chain.step(h, rates, &mut self.rng);
            chain.cool(h);
        }
        if beat.index == 3 {
            let k = (-VENT_RATE * dt as f32).exp();
            chain.water.as_mut_slice().iter_mut().for_each(|w| *w *= k);
        }
        if chain.pool.len() > CAPACITY / 8 || chain.max_temp() > 1.5 + SPARK_HEAT {
            self.exploded = true;
        }
        // The threads: every `TRAIL_DT` each radical's position joins its trail, so a
        // trail spans `TRAIL_N · TRAIL_DT` simulated seconds whatever the draw cadence.
        chain.smooth(dt);
        self.trail_clock += dt;
        if self.trail_clock >= TRAIL_DT {
            self.trail_clock %= TRAIL_DT;
            chain.sample_trails();
        }
        self.record_deaths(dt);
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
            k_t: K_T3 * p * p,
            diff: diffusion(p),
            heat: false,
        }
    }

    #[test]
    fn chn_a8_three_body_loss_grows_as_p_squared_and_relative_to_branching_as_p() {
        let g = RadicalChain::default();
        for p in [0.3, P_LOW, 1.0, P_HIGH] {
            let (r1, r2) = (g.rates(p), g.rates(2.0 * p));
            let close = |a: f64, b: f64| (a - b).abs() <= 1e-12 * b.abs();
            // H + O₂ + M: one power of p for O₂, one for the third body.
            assert!(
                close(r2.k_t, 4.0 * r1.k_t),
                "p {p}: k_t {} -> {}",
                r1.k_t,
                r2.k_t
            );
            assert!(close(r2.k_b0, 2.0 * r1.k_b0), "p {p}: branching is ∝ p");
            assert!(close(r2.k_t / r2.k_b0, 2.0 * (r1.k_t / r1.k_b0)), "p {p}");
        }
        // The second limit (three-body loss = branching) lies far above the sweep.
        const { assert!(K_B0 / K_T3 > 3.0 * P_HIGH) };
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
            c.step(SIM_DT, cold(K_B0 * p, p), &mut r);
            c.refuel();
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
        let loop_s: f64 = PHASES.iter().map(|p| p.1).sum();
        for k in 0..(loop_s / SIM_DT) as usize - 1 {
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
    fn chn_a9_captions_follow_the_state_and_the_spark_strikes_at_the_burn() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(42, size);
        let mut g = RadicalChain::default();
        g.resize(size, &mut ctx);
        assert_eq!(g.caption(), Some("slow reaction"));
        let loop_s: f64 = PHASES.iter().map(|p| p.1).sum();
        let mut seen = [false; 3];
        let mut latched_in = Vec::new();
        let mut was = false;
        for _ in 0..(2.0 * loop_s / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let beat = g.beat();
            let p = pressure(&beat);
            let c = g.caption().expect("caption");
            let want = if p < 1.0 {
                "slow reaction"
            } else if g.exploded {
                "explosion"
            } else {
                "chain branching"
            };
            assert_eq!(c, want, "t {:.2}: p {p:.2}, exploded {}", g.t, g.exploded);
            // The spark belongs to the burn: none before it in a loop.
            if beat.index < BURN {
                assert_ne!(g.sparked, g.cycle, "t {:.2}: sparked before the burn", g.t);
            }
            if g.exploded && !was {
                latched_in.push(beat.index);
            }
            was = g.exploded;
            seen[CAPTIONS.iter().position(|&x| x == c).expect("listed")] = true;
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(
            latched_in,
            vec![BURN, BURN],
            "the explosion latches in each burn"
        );
    }

    #[test]
    fn chn_a6_the_flame_runs_as_a_front_from_the_spark_and_fades_over_seconds() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = RadicalChain::default();
        g.resize(size, &mut ctx);
        let share = |c: &Chain, f: &dyn Fn(f32, f32) -> bool| {
            let n = c.gas.len() as f32;
            c.gas
                .as_slice()
                .iter()
                .zip(c.temp.as_slice())
                .filter(|&(&g, &t)| f(g, t))
                .count() as f32
                / n
        };
        let (mut at_latch, mut hot_max) = (None, 0.0f32);
        for _ in 0..(14.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let c = g.chain.as_ref().expect("chain");
            let hot = share(c, &|_, t| t > 1.3);
            let burnt = share(c, &|g, _| g < 0.5);
            if g.exploded && at_latch.is_none() {
                at_latch = Some((hot, burnt));
            }
            hot_max = hot_max.max(hot);
        }
        let (hot, burnt) = at_latch.expect("the flash comes");
        // A kernel at the spark, not the whole vessel at once, which the front then sweeps.
        assert!(
            hot < 0.25 && burnt < 0.1,
            "at the latch: hot {hot} burnt {burnt}"
        );
        assert!(hot_max > 0.3, "the front sweeps the vessel ({hot_max})");
    }

    /// Every lit cell of a frame at `t` s (120×40), with the canvas.
    fn frame_at(t: f64) -> (RadicalChain, Canvas) {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = RadicalChain::default();
        g.resize(size, &mut ctx);
        while g.t < t {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        (g, canvas)
    }

    #[test]
    fn chn_a7_threads_on_black_coloured_by_age() {
        // M16: the ground is black and every lit cell (the readout aside) is braille: a
        // thread (`Hot` fresh, `Accent` tail), a radical (`Sun`), a wall death (`Cold`)
        // or the faint wall (`Dim`); no flash fill.
        let allowed = [
            THREAD_INKS[0],
            THREAD_INKS[1],
            THREAD_INKS[2],
            HEAD,
            DEATH,
            WALL,
        ];
        for t in [1.5, 4.5, 6.0, 12.5] {
            let (_, canvas) = frame_at(t);
            let mut threads = 0;
            for c in canvas.cells() {
                if c.ch == ' ' || c.ink.role == Role::Text {
                    continue;
                }
                assert!(
                    c.bg == Ink::BLANK && ('\u{2800}'..='\u{28ff}').contains(&c.ch),
                    "t = {t}: {c:?}"
                );
                assert!(allowed.contains(&c.ink), "t = {t}: {c:?}");
                threads += usize::from(c.ink.role == Role::Hot);
            }
            assert!(threads > 0, "t = {t}: no threads");
        }
        // Colour by age: fresh segments hot, the tail faint.
        assert_eq!(thread_ink(0), THREAD_INKS[0]);
        assert!(thread_ink(3).level < thread_ink(0).level);
        assert_eq!(thread_ink(TRAIL_N - 1).role, Role::Accent);
        // Burnt gas quenches radicals: branching needs unburnt gas.
        let (mut c, mut r) = chain_with(400, 9);
        c.gas.fill(0.0);
        let hot = cold(50.0, 1.0);
        for _ in 0..60 {
            c.step(SIM_DT, hot, &mut r);
        }
        assert!(c.pool.len() < 400, "{}", c.pool.len());
    }

    #[test]
    fn chn_a10_branching_forks_a_thread_and_wall_deaths_flash_cold() {
        // A branching child is born on its parent with the parent's past: one thread
        // splits in two, the child under a new identity.
        let (mut c, mut r) = chain_with(50, 3);
        for _ in 0..TRAIL_N {
            c.step(SIM_DT, cold(0.0, 1.0), &mut r);
            c.sample_trails();
        }
        let before: Vec<Radical> = c.pool.iter().copied().collect();
        let n0 = c.pool.len();
        c.step(SIM_DT, cold(30.0, 1.0), &mut r);
        let born: Vec<&Radical> = c.pool.iter().skip(n0).collect();
        assert!(!born.is_empty(), "something branched");
        for b in born {
            let parent = before
                .iter()
                .find(|p| p.trail == b.trail)
                .expect("a child shares a parent's trail");
            assert_ne!(parent.id, b.id);
        }
        // Wall deaths: recorded where the radical last was inside, and drawn briefly.
        let mut c = Chain::new(Vessel { a: A_REF }, Size::new(30, 16));
        c.pool.spawn(Radical::at(0.9, 0.999_999));
        let mut r = rng(4);
        for _ in 0..30 {
            c.step(SIM_DT, cold(0.0, 1.0), &mut r);
        }
        assert!(c.wall_hits.iter().all(|&(x, y)| c.vessel.inside(x, y)));
        let (mut g, _) = frame_at(1.0);
        let deaths = |g: &RadicalChain| g.deaths.iter().filter(|d| d.2 > 0.0).count();
        assert!(deaths(&g) > 0, "slow chains die on the walls");
        g.chain.as_mut().expect("chain").pool.clear();
        let mut ctx = Ctx::new(1, Size::new(120, 40));
        for _ in 0..(f64::from(DEATH_S) / SIM_DT).ceil() as usize + 1 {
            g.update(SIM_DT, &mut ctx);
            g.chain.as_mut().expect("chain").pool.clear();
        }
        assert_eq!(deaths(&g), 0, "deaths fade within {DEATH_S} s");
        // A crowded pool draws a bounded, fixed share of threads.
        let shown = (0..4000u32).filter(|&id| drawn(id, 4000)).count();
        assert!(shown < MAX_THREADS + MAX_THREADS / 5, "{shown}");
        assert!((0..100u32).all(|id| drawn(id, 100)));
    }
}
