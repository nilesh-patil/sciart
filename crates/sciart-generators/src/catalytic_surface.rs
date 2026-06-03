//! Carbon monoxide burning on a platinum surface, atom by atom: the Ziff–Gulari–Barshad
//! lattice model (SPEC-002 `catalytic_surface`).
//!
//! ## Model
//!
//! Ziff, Gulari & Barshad (1986): a square lattice of adsorption sites, each empty, O or
//! CO. One attempt picks a site at random; with probability `y` (the CO fraction of the
//! gas) a CO molecule tries to land there (the site must be empty), else an O₂ molecule
//! tries to land on it and a random neighbour, dissociating into two O atoms (both sites
//! must be empty). Every newly adsorbed particle checks its four neighbours, from a random
//! start, for a partner (CO for O, O for CO); a CO–O pair reacts at once to CO₂, which
//! leaves and frees both sites (the flash). Periodic edges.
//!
//! The model has two kinetic phase transitions: below `y₁ ≈ 0.39` the surface chokes on
//! oxygen, above `y₂ ≈ 0.525` (first order) on CO, and only between them does it keep
//! producing CO₂ (Ertl's Nobel-prize chemistry, stylised). Pure ZGB poisoning is
//! absorbing, so a small desorption channel ([`DESORB`]) lets the swept surface recover
//! when `y` returns into the reactive window: CO desorbs thermally in the real system,
//! and a rarer O loss stands in for the O-poisoned surface's slow reduction.
//!
//! The gas is fed as a jet onto the centre, so `y` falls toward the corners ([`GRADE`])
//! and each transition sits on a moving ring. The jet's `y` runs on a looping
//! [`Timeline`] in simulated seconds (36.5 s), so a
//! resize never desyncs it. Captions follow the *surface*, not `y`: poisoning lags the
//! gas by seconds, and the caption names what is on screen ([`regime`]).
//!
//! ## Picture
//!
//! The lattice is the art (ADR-0017): one half-block pixel per site, on black. Empty
//! platinum is the terminal's black, O a grey just off it (`Ramp(Starlight)` 20), CO a
//! muted rust (`Ramp(Sienna)`) that brightens on its islands' edges, where the reaction
//! runs, so islands, fronts and the moving poisoning rings read as fine texture. A sparse sample of the CO₂ events blinks as single near-white pixels that
//! fade in ≈ 0.3 s (the accent). Sites are sampled nearest (1:1 up to the cap).

use std::fmt::Write as _;

use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster;
use sciart_core::title;
use sciart_core::{draw, ease, rng};

pub static META: Meta = Meta {
    name: "catalytic_surface",
    category: Category::Chemistry,
    description: "CO oxidation on a platinum surface: the Ziff-Gulari-Barshad lattice model swept through its reactive window",
    title: "Catalysis · CO burns on platinum",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CatalyticSurface::default())
}

const CAPTIONS: [&str; 3] = ["oxygen-poisoned", "reactive", "CO-poisoned"];

/// Site states.
pub(crate) const EMPTY: u8 = 0;
pub(crate) const OX: u8 = 1;
pub(crate) const CO: u8 = 2;

/// `y` held in each regime of the sweep: below `y₁ ≈ 0.39`, inside the window, above
/// `y₂ ≈ 0.525` (Ziff et al. 1986; Evans & Miesch 1991).
const Y_REACTIVE: f64 = 0.46;
/// `y` at the jet (the centre of the surface) in the sweep's holds.
const Y_JET: f64 = 0.545;
const Y_CO: f64 = 0.65;
const Y_ADVANCE: f64 = 0.53;
const Y_OXYGEN: f64 = 0.43;
/// The gas is fed as a jet onto the centre and CO is used up on its way out, so `y`
/// falls with distance from the centre: `y = y_jet − GRADE·r`, `r` the distance over the
/// half-diagonal (0 at the centre, 1 in the corners). Each phase transition then sits on
/// a ring (where the local `y` crosses `y₁` or `y₂`) and moves as the sweep moves:
/// CO grows out from the centre as a disc, O closes in from the corners as a layer.
pub(crate) const GRADE: f64 = 0.14;
/// Coverage at which the caption calls the surface poisoned (CAT-A5).
pub(crate) const POISONED: f64 = 0.8;

/// Share of attempts that are desorption attempts instead of adsorptions, and the chance
/// an O picked by one leaves (CO always does): slow enough that the poisoned phases keep
/// ≥ 90 % coverage, fast enough that the surface recovers within a few seconds.
pub(crate) const DESORB: f64 = 0.004;
const O_DESORB: f64 = 0.1;

/// Lattice cap (ADR-0012): 200 × 100 sites, one per half-block pixel at 200 × 50.
const MAX_SITES: usize = 20_000;
const BUCKET: f64 = 1.25;
/// Attempts per simulated second in lattice sweeps, and the cap per update (CAT-A4).
const SWEEPS_PER_S: f64 = 16.0;
pub(crate) const MAX_ATTEMPTS: u32 = 12_000;
/// Sweeps run in the first resize, so the first frame already shows islands.
const PREROLL_SWEEPS: usize = 12;

/// The 36.5 s sweep of the jet's `y`: (label, seconds, y at the start, y at the end),
/// smoothstep between. CO islands nucleate at the jet and the CO disc grows out to the
/// corners; `y` then falls, O eats the disc back along a bright ring front, and the O
/// layer closes in from the corners to a brief poisoned beat before the jet recovers.
/// The local `y` stays inside the reactive window somewhere except in the two short
/// holds, so fronts are always moving.
const SWEEP: [(&str, f64, f64, f64); 8] = [
    ("CO nucleates", 5.0, Y_JET, Y_JET),
    ("CO grows", 7.0, Y_JET, Y_CO),
    ("CO", 2.5, Y_CO, Y_CO),
    ("O returns", 9.0, Y_CO, Y_ADVANCE),
    ("reactive", 5.0, Y_ADVANCE, Y_ADVANCE),
    ("O advances", 4.5, Y_ADVANCE, Y_OXYGEN),
    ("O-poisoned", 0.5, Y_OXYGEN, Y_OXYGEN),
    ("recover", 3.0, Y_OXYGEN, Y_JET),
];

/// Inks (ADR-0017): O a grey just off black (the Ice ramp has no near-black foot), CO a
/// muted rust, brightest on the island's edge, the CO₂ blinks near white.
const O_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 20);
const CO_INK: Ink = Ink::new(Role::Ramp(Ramp::Sienna), 80);
/// A CO site on its island's edge (a non-CO neighbour), where the reaction runs: the
/// edge carries the light.
const FRONT_INK: Ink = Ink::new(Role::Ramp(Ramp::Sienna), 185);
const FLASH_RAMP: Ramp = Ramp::Starlight;
/// A blink's level: `FLASH_LO + (FLASH_HI − FLASH_LO)·v` for its brightness `v`.
const FLASH_LEVELS: (f32, f32) = (170.0, 250.0);
/// The loop opens part-way through "O returns", where the jet's `y` is ≈ 0.59, on a
/// surface already split the way that `y` splits it: a CO disc about the jet out to where
/// the local `y` falls to `y₂` ([`Y_2`]; ≈ 0.46 of the half-diagonal, ≈ 40 % of the
/// sites) and O around it. Rust and teal domains then meet along a sparking front from the
/// first frame, and the disc recedes as `y` keeps falling, as it does every loop.
const OPEN_T: f64 = 19.0;
const Y_2: f64 = 0.525;
/// Share of sites filled in the opening domains (the rest are vacancies).
const OPEN_FILL: f64 = 0.9;
const OPEN_SALT: u64 = 0x0c0_5eed;
/// Share of CO₂ events drawn as sparks, their fade time (s: 5 % left after 0.3 s), and
/// the spark value below which a blink has faded out.
const SPARK_SHARE: f64 = 0.04;
const SPARK_TAU: f64 = 0.1;
const SPARK_MIN: f32 = 0.15;
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// The CO fraction `y` of the gas at a beat of the sweep: holds at the three regimes,
pub(crate) fn y_at(beat: &Beat) -> f64 {
    let (_, _, a, b) = SWEEP[beat.index.min(SWEEP.len() - 1)];
    ease::lerp(a, b, ease::smoothstep(beat.progress))
}

/// The ink of a site in state `site` with spark brightness `spark` (`front`: a CO site on
/// the island edge): a blink while it lasts, else the species' ink, black for empty
/// platinum.
pub(crate) fn site_ink(site: u8, spark: f32, front: bool) -> Ink {
    if spark >= SPARK_MIN {
        let (lo, hi) = FLASH_LEVELS;
        let v = ((spark - SPARK_MIN) / (1.0 - SPARK_MIN)).clamp(0.0, 1.0);
        return Ink::new(Role::Ramp(FLASH_RAMP), (lo + (hi - lo) * v) as u8);
    }
    match site {
        OX => O_INK,
        CO if front => FRONT_INK,
        CO => CO_INK,
        _ => Ink::BLANK,
    }
}

/// Regime of the surface from its `(O, CO)` coverages (CAT-A5): 0 O-poisoned, 1
/// reactive, 2 CO-poisoned.
pub(crate) fn regime((o, c): (f64, f64)) -> usize {
    if o >= POISONED {
        0
    } else if c >= POISONED {
        2
    } else {
        1
    }
}

/// The ZGB surface: site states, this update's CO₂ events and the CO₂ count.
#[derive(Debug, Clone, Default)]
pub(crate) struct Zgb {
    pub sites: Grid<u8>,
    /// Sites where CO₂ left since the last [`Self::clear_events`] (the CO's site), up to
    /// the buffer's capacity.
    pub events: Vec<u32>,
    /// CO₂ molecules made so far.
    pub produced: u64,
    /// Per-site offset of `y` (the gas gradient, [`GRADE`]); empty means uniform gas.
    pub grade: Vec<f32>,
}

impl Zgb {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            sites: Grid::new(size, EMPTY),
            // An attempt makes at most two molecules (an O₂ landing next to two CO).
            events: Vec::with_capacity(2 * MAX_ATTEMPTS as usize),
            produced: 0,
            grade: Vec::new(),
        }
    }

    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }

    /// Neighbour `dir` (E, S, W, N) of site `i`, periodic.
    #[inline]
    fn neighbour(&self, i: usize, dir: u32) -> usize {
        let w = usize::from(self.sites.width());
        let h = usize::from(self.sites.height());
        let (x, y) = (i % w, i / w);
        match dir {
            0 => y * w + (x + 1) % w,
            1 => ((y + 1) % h) * w + x,
            2 => y * w + (x + w - 1) % w,
            _ => ((y + h - 1) % h) * w + x,
        }
    }

    /// A freshly adsorbed particle at `i` looks for a partner among its neighbours from
    /// a random start; the first found reacts to CO₂ and both sites empty.
    fn react_from(&mut self, i: usize, rng: &mut SplitMix64) {
        let partner = match self.sites.as_slice()[i] {
            OX => CO,
            CO => OX,
            _ => return,
        };
        let start = rng.below(4);
        for k in 0..4 {
            let j = self.neighbour(i, (start + k) % 4);
            if j != i && self.sites.as_slice()[j] == partner {
                let s = self.sites.as_mut_slice();
                s[i] = EMPTY;
                s[j] = EMPTY;
                if self.events.len() < self.events.capacity() {
                    self.events.push(if partner == CO { j } else { i } as u32);
                }
                self.produced += 1;
                return;
            }
        }
    }

    /// Lay the radial gas gradient over the lattice: `y` falls by `grade`·r from the
    /// centre, `r` the distance over the half-diagonal (square sites, so rings are round).
    pub(crate) fn set_grade(&mut self, grade: f64) {
        let (w, h) = (
            f64::from(self.sites.width()),
            f64::from(self.sites.height()),
        );
        let half = (0.25 * (w * w + h * h)).sqrt().max(1e-9);
        let wi = usize::from(self.sites.width()).max(1);
        self.grade.clear();
        self.grade.extend((0..self.sites.len()).map(|i| {
            let (x, y) = (
                (i % wi) as f64 + 0.5 - 0.5 * w,
                (i / wi) as f64 + 0.5 - 0.5 * h,
            );
            (-grade * x.hypot(y) / half) as f32
        }));
    }

    /// One ZGB attempt at CO fraction `y` (plus the site's gradient offset) (CAT-A3):
    /// adsorption never overfills a site, and O₂ needs two empty neighbouring sites.
    pub(crate) fn attempt(&mut self, y: f64, rng: &mut SplitMix64) {
        let n = self.sites.len();
        if n == 0 {
            return;
        }
        let i = rng.below(n as u32) as usize;
        let r = rng.next_f64();
        if r < DESORB {
            let s = &mut self.sites.as_mut_slice()[i];
            if *s == CO || (*s == OX && rng.next_f64() < O_DESORB) {
                *s = EMPTY;
            }
            return;
        }
        if self.sites.as_slice()[i] != EMPTY {
            return;
        }
        let y = y + self.grade.get(i).map_or(0.0, |&g| f64::from(g));
        if (r - DESORB) / (1.0 - DESORB) < y {
            self.sites.as_mut_slice()[i] = CO;
            self.react_from(i, rng);
        } else {
            let j = self.neighbour(i, rng.below(4));
            if j == i || self.sites.as_slice()[j] != EMPTY {
                return;
            }
            let s = self.sites.as_mut_slice();
            s[i] = OX;
            s[j] = OX;
            self.react_from(i, rng);
            self.react_from(j, rng);
        }
    }

    /// `(O, CO)` coverages.
    pub(crate) fn coverage(&self) -> (f64, f64) {
        let n = self.sites.len().max(1) as f64;
        let (mut o, mut c) = (0usize, 0usize);
        for &s in self.sites.as_slice() {
            o += usize::from(s == OX);
            c += usize::from(s == CO);
        }
        (o as f64 / n, c as f64 / n)
    }
}

#[derive(Debug)]
pub struct CatalyticSurface {
    size: Size,
    zgb: Zgb,
    scratch: Grid<u8>,
    rng: SplitMix64,
    schedule: Option<Timeline>,
    t: f64,
    /// Fractional attempts carried to the next update.
    acc: f64,
    /// Attempts made by the last update (CAT-A4).
    last_attempts: u32,
    /// `(O, CO)` coverage after the last update, for the caption.
    cover: (f64, f64),
    pixels: Pixels,
    /// Per-site spark brightness (a sparse sample of the events), and the update count
    /// that keys the sampling.
    sparks: Grid<f32>,
    frame: u32,
    /// Site column of every pixel column and site row of every pixel row.
    cols: Vec<u16>,
    rows: Vec<u16>,
    readout_room: usize,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for CatalyticSurface {
    fn default() -> Self {
        Self {
            dirty: true,
            size: Size::new(1, 1),
            zgb: Zgb::default(),
            scratch: Grid::new(Size::new(1, 1), EMPTY),
            rng: SplitMix64::new(0),
            schedule: None,
            t: 0.0,
            acc: 0.0,
            last_attempts: 0,
            cover: (0.0, 0.0),
            pixels: Pixels::new(Size::new(1, 1)),
            sparks: Grid::new(Size::new(1, 1), 0.0),
            frame: 0,
            cols: Vec::new(),
            rows: Vec::new(),
            readout_room: 0,
        }
    }
}

impl CatalyticSurface {
    fn y(&self) -> f64 {
        self.schedule
            .as_ref()
            .map_or(Y_REACTIVE, |s| y_at(&s.at(self.t)))
    }

    /// Fade the sparks by `keep` and blink a sparse, deterministic sample of this
    /// update's CO₂ events, so they read as single bright points on a quiet surface, not a
    /// blizzard. Bound to updates, not frames (GEN-R20).
    fn expose_events(&mut self, keep: f32) {
        let size = self.zgb.sites.size();
        if self.sparks.size() != size {
            self.sparks.resize(size, 0.0);
        }
        let sp = self.sparks.as_mut_slice();
        sp.iter_mut().for_each(|v| *v *= keep);
        self.frame = self.frame.wrapping_add(1);
        for &e in &self.zgb.events {
            let key = rng::cell_key(e as i32, self.frame as i32);
            if rng::hash01(key) < SPARK_SHARE {
                if let Some(v) = sp.get_mut(e as usize) {
                    *v = 1.0;
                }
            }
        }
    }

    /// One pixel per site (nearest; the lattice is the art).
    fn rasterise(&mut self) {
        let (sites, sparks) = (self.zgb.sites.as_slice(), self.sparks.as_slice());
        let (cols, rows) = (&self.cols, &self.rows);
        let (w, h) = (
            usize::from(self.zgb.sites.width()),
            usize::from(self.zgb.sites.height()),
        );
        self.pixels.shade(|x, y| {
            let (sx, sy) = (cols[x as usize] as usize, rows[y as usize] as usize);
            let i = sy * w + sx;
            // The island edge: a CO site with a non-CO neighbour (periodic, as the model).
            let front = sites[i] == CO && {
                let (l, r) = ((sx + w - 1) % w, (sx + 1) % w);
                let (u, d) = ((sy + h - 1) % h, (sy + 1) % h);
                [sy * w + l, sy * w + r, u * w + sx, d * w + sx]
                    .iter()
                    .any(|&j| sites[j] != CO)
            };
            site_ink(sites[i], sparks[i], front)
        });
    }

    fn run(&mut self, attempts: u32, y: f64) {
        for _ in 0..attempts {
            self.zgb.attempt(y, &mut self.rng);
        }
    }

    /// Title-row readout: `y=0.46  CO2 12345`, or `y=0.46`.
    fn readout(&self, canvas: &mut Canvas) {
        let hundredths = (self.y() * 100.0).round() as u32;
        let mut text = StackText::<40>::new();
        let _ = match self.readout_room {
            // The CO₂ count is the turnover since the scene started.
            29.. => write!(
                text,
                "CO fraction y=0.{hundredths:02}  CO2 {}",
                self.zgb.produced
            ),
            6.. => write!(text, "y=0.{hundredths:02}"),
            _ => return,
        };
        let y = i32::from(self.size.height()) - 1;
        draw::label_right(canvas, y, text.as_str(), READOUT_INK);
    }
}

impl Generator for CatalyticSurface {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[regime(self.cover)])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        let pixels = Size::new(size.width().max(1), size.height().saturating_mul(2).max(1));
        let lattice = grid::lattice_for(pixels, MAX_SITES, BUCKET);
        if self.schedule.is_none() {
            let phases: [(&str, f64); SWEEP.len()] =
                std::array::from_fn(|i| (SWEEP[i].0, SWEEP[i].1));
            self.schedule = Some(Timeline::new(&phases));
            self.rng = SplitMix64::from_rng(&mut ctx.rng);
            self.zgb = Zgb::new(lattice);
            self.zgb.set_grade(GRADE);
            // Open mid-window, on a CO disc with O around it (see OPEN_T).
            self.t = OPEN_T;
            let y = self.y();
            for (i, (s, &g)) in self
                .zgb
                .sites
                .as_mut_slice()
                .iter_mut()
                .zip(&self.zgb.grade)
                .enumerate()
            {
                // The local y is the jet's plus the gradient offset (−GRADE·r); a fixed
                // per-site hash picks the vacancies.
                *s = if rng::hash01(OPEN_SALT ^ i as u64) >= OPEN_FILL {
                    EMPTY
                } else if y + f64::from(g) > Y_2 {
                    CO
                } else {
                    OX
                };
            }
            let n = self.zgb.sites.len() as u32;
            for _ in 0..PREROLL_SWEEPS {
                self.run(n, y);
                self.zgb.clear_events();
            }
            // A tenth of a second of frames, exposed (and cleared) as `update` does, so
            // the front already sparks in the first frame.
            for _ in 0..6 {
                self.zgb.clear_events();
                self.run((f64::from(n) * SWEEPS_PER_S * SIM_DT) as u32, y);
                self.expose_events((-SIM_DT / SPARK_TAU).exp() as f32);
            }
        } else if self.zgb.sites.size() != lattice {
            grid::regrid_nearest(&mut self.zgb.sites, lattice, &mut self.scratch);
            self.zgb.set_grade(GRADE);
        }
        self.zgb.clear_events();
        self.cover = self.zgb.coverage();
        self.pixels.resize(size);
        self.sparks.resize(lattice, 0.0);
        let px = self.pixels.pixel_size();
        let mut map = SampleMap::default();
        map.rebuild(lattice, px);
        self.cols.clear();
        self.cols
            .extend((0..i32::from(px.width())).map(|x| map.nearest(x, 0).0 as u16));
        self.rows.clear();
        self.rows
            .extend((0..i32::from(px.height())).map(|y| map.nearest(0, y).1 as u16));
        self.readout_room = title::stable_readout_room(size, &META);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.schedule.is_none() {
            return;
        }
        self.t += dt;
        let y = self.y();
        let n = self.zgb.sites.len() as f64;
        self.acc += n * SWEEPS_PER_S * dt;
        let attempts = self.acc.floor().min(f64::from(MAX_ATTEMPTS));
        self.acc = (self.acc - attempts).min(1.0);
        self.last_attempts = attempts as u32;
        self.zgb.clear_events();
        self.run(self.last_attempts, y);
        self.cover = self.zgb.coverage();
        self.expose_events((-dt / SPARK_TAU).exp() as f32);
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
        // ASCII: a density ramp by brightness (sparks brightest), not per-site letters.
        self.pixels.blit(canvas, |_, _, top, bottom| {
            raster::ascii_ramp_cell(top, bottom)
        });
        self.readout(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> SplitMix64 {
        SplitMix64::from_rng(&mut Ctx::new(seed, Size::new(1, 1)).rng)
    }

    fn relax(y: f64, sweeps: usize, seed: u64) -> Zgb {
        let mut z = Zgb::new(Size::new(64, 64));
        let mut r = rng(seed);
        for _ in 0..sweeps * 4096 {
            z.attempt(y, &mut r);
            if z.events.len() == z.events.capacity() {
                z.clear_events();
            }
        }
        z
    }

    #[test]
    fn cat_a1_low_y_poisons_with_oxygen_and_high_y_with_co() {
        for seed in 0..3 {
            let (o, _) = relax(0.30, 400, seed).coverage();
            assert!(o >= 0.9, "seed {seed}: O coverage {o}");
            let (_, c) = relax(0.60, 400, seed).coverage();
            assert!(c >= 0.9, "seed {seed}: CO coverage {c}");
        }
    }

    #[test]
    fn cat_a2_reactive_window_keeps_producing_with_mixed_coverage() {
        let mut z = relax(0.45, 200, 7);
        let mut r = rng(8);
        for _ in 0..10 {
            let before = z.produced;
            for _ in 0..20 * 4096 {
                z.attempt(0.45, &mut r);
            }
            z.clear_events();
            assert!(z.produced > before, "production stopped");
            let (o, c) = z.coverage();
            assert!(o < 0.9 && c < 0.9 && o + c < 0.99, "O {o} CO {c}");
        }
    }

    #[test]
    fn cat_a3_adsorption_never_overfills_and_o2_needs_two_empty_neighbours() {
        // A lone empty site in a CO sea: O₂ can never land (no empty neighbour) and a
        // CO landing there has no O to react with.
        let mut z = Zgb::new(Size::new(9, 7));
        z.sites.fill(CO);
        z.sites.as_mut_slice()[3 * 9 + 4] = EMPTY;
        let mut r = rng(1);
        for _ in 0..5000 {
            let before = z.sites.as_slice().to_vec();
            z.attempt(0.0, &mut r);
            let ox = z.sites.as_slice().iter().filter(|&&s| s == OX).count();
            assert_eq!(ox, 0, "O₂ adsorbed onto a single empty site");
            // Every change is a legal one: desorption (occupied → empty) only.
            for (a, b) in before.iter().zip(z.sites.as_slice()) {
                assert!(a == b || *b == EMPTY, "{a} → {b}");
            }
            if z.sites.as_slice().iter().filter(|&&s| s == EMPTY).count() > 1 {
                break;
            }
        }
        // Random play: states stay in range; events never outgrow their buffer.
        let mut z = Zgb::new(Size::new(16, 12));
        for k in 0..30_000 {
            z.attempt(0.5, &mut r);
            if k % 997 == 0 {
                assert!(z.sites.as_slice().iter().all(|&s| s <= CO));
                assert!(z.events.len() <= z.events.capacity());
                z.clear_events();
            }
        }
        // Degenerate lattices never panic.
        for size in [Size::new(1, 1), Size::new(1, 5), Size::new(4, 1)] {
            let mut z = Zgb::new(size);
            for _ in 0..200 {
                z.attempt(0.3, &mut r);
            }
        }
    }

    #[test]
    fn cat_a4_attempts_per_update_never_exceed_the_cap() {
        for size in [Size::new(200, 50), Size::new(500, 150), Size::new(80, 24)] {
            let mut ctx = Ctx::new(3, size);
            let mut g = CatalyticSurface::default();
            g.resize(size, &mut ctx);
            g.prepare();
            for dt in [0.25, SIM_DT, 0.25, 0.1] {
                g.update(dt, &mut ctx);
                g.prepare();
                assert!(
                    g.last_attempts <= MAX_ATTEMPTS,
                    "{size:?}: {}",
                    g.last_attempts
                );
            }
        }
    }

    #[test]
    fn cat_a7_sites_on_black_and_sparks_are_single_bright_blinks() {
        // ADR-0017: empty platinum is black, O and CO dim, and a CO₂ blink the only bright
        // thing, fading to its site's own ink.
        assert_eq!(site_ink(EMPTY, 0.0, false), Ink::BLANK);
        assert_eq!(site_ink(OX, 0.0, false), O_INK);
        assert_eq!(site_ink(CO, 0.0, false), CO_INK);
        // The front carries the light, and O stays just off black.
        assert!(site_ink(CO, 0.0, true).level > CO_INK.level);
        let hot = site_ink(EMPTY, 1.0, false);
        assert!(hot.role == Role::Ramp(FLASH_RAMP) && hot.level >= 230);
        assert!(site_ink(OX, 0.5, false).level < hot.level);
        assert_eq!(site_ink(CO, SPARK_MIN * 0.9, false), CO_INK);
    }

    #[test]
    fn cat_a8_the_loop_opens_mid_window_on_balanced_domains_with_sparks() {
        for seed in [1, 42] {
            let size = Size::new(80, 24);
            let mut ctx = Ctx::new(seed, size);
            let mut g = CatalyticSurface::default();
            g.resize(size, &mut ctx);
            g.prepare();
            let y = g.y();
            assert!((0.52..=0.6).contains(&y), "opening y {y}");
            let n = g.zgb.sites.len() as f64;
            let count = |s: u8| g.zgb.sites.as_slice().iter().filter(|&&v| v == s).count() as f64;
            let (co, ox) = (count(CO) / n, count(OX) / n);
            // Both domains are large: CO holds 35–55 % of the occupied sites.
            let share = co / (co + ox);
            assert!(
                (0.35..=0.55).contains(&share),
                "seed {seed}: CO share {share}"
            );
            assert!(
                g.sparks.as_slice().iter().any(|&s| s > 0.0),
                "sparks at t = 0"
            );
            assert_eq!(g.caption(), Some("reactive"));
        }
    }

    #[test]
    fn cat_a5_captions_follow_the_state_of_the_surface() {
        assert_eq!(regime((0.85, 0.0)), 0);
        assert_eq!(regime((0.3, 0.1)), 1);
        assert_eq!(regime((0.0, 0.95)), 2);
        let mut ctx = Ctx::new(1, Size::new(80, 24));
        let mut g = CatalyticSurface::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        let mut seen = [false; 3];
        for _ in 0..(40.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
            g.prepare();
            let r = regime(g.zgb.coverage());
            assert_eq!(g.caption(), Some(CAPTIONS[r]));
            seen[r] = true;
        }
        assert_eq!(seen, [true; 3], "the sweep reaches all three regimes");
    }

    #[test]
    fn cat_a6_poisoning_is_a_brief_beat_and_the_surface_never_stops_reacting() {
        for seed in [1, 42] {
            let size = Size::new(80, 24);
            let mut ctx = Ctx::new(seed, size);
            let mut g = CatalyticSurface::default();
            g.resize(size, &mut ctx);
            g.prepare();
            let loop_s: f64 = SWEEP.iter().map(|p| p.1).sum();
            let (mut poisoned_s, mut min_rate) = (0.0, f64::INFINITY);
            let mut last = g.zgb.produced;
            for _ in 0..(loop_s / 0.5) as usize {
                for _ in 0..30 {
                    g.update(SIM_DT, &mut ctx);
                    g.prepare();
                }
                if regime(g.zgb.coverage()) != 1 {
                    poisoned_s += 0.5;
                }
                // CO₂ per site per sweep over the last half second.
                let sweeps = SWEEPS_PER_S * 0.5 * g.zgb.sites.len() as f64;
                min_rate = min_rate.min((g.zgb.produced - last) as f64 / sweeps);
                last = g.zgb.produced;
            }
            // O-poisoned ≈ 5 s and CO-poisoned ≈ 7 s of the 36.5 s loop.
            assert!(poisoned_s <= 14.0, "seed {seed}: poisoned {poisoned_s} s");
            assert!(
                min_rate > 0.01,
                "seed {seed}: production stalled ({min_rate})"
            );
        }
        // The gas gradient: y falls from the centre to the corners.
        let mut z = Zgb::new(Size::new(20, 10));
        z.set_grade(GRADE);
        let (c, k) = (z.grade[5 * 20 + 10], z.grade[0]);
        assert!(c > -0.02 && (k + GRADE as f32).abs() < 0.02, "{c} {k}");
    }

    #[test]
    fn cat_a7_a_blink_is_one_pixel_and_fades_in_a_third_of_a_second() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(3, size);
        let mut g = CatalyticSurface::default();
        g.resize(size, &mut ctx);
        g.prepare();
        // 1:1 sites and pixels at 80×24: a blink covers exactly its own pixel.
        assert_eq!(g.zgb.sites.size(), g.pixels.pixel_size());
        let bright = |g: &CatalyticSurface| {
            let ps = g.pixels.pixel_size();
            (0..i32::from(ps.height()))
                .flat_map(|y| (0..i32::from(ps.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    g.pixels
                        .get(x, y)
                        .is_some_and(|i| i.role == Role::Ramp(FLASH_RAMP) && i != O_INK)
                })
                .count()
        };
        assert!(bright(&g) > 0, "the front sparks from the first frame");
        // With the events cleared, every blink is gone within ≈ 0.3 s.
        let keep = (-SIM_DT / SPARK_TAU).exp() as f32;
        for _ in 0..18 {
            g.zgb.clear_events();
            g.expose_events(keep);
        }
        g.rasterise();
        assert_eq!(bright(&g), 0);
    }
}
