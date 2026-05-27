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
//! The lattice is shaded as domains, not sites: each species' occupancy is averaged over
//! a 5 × 5 box ([`box_blur_wrap`]) and sampled bilinearly, so O islands (Ferroin) and
//! CO islands (Aurora) read as smooth, saturated regions, brighter the purer they are,
//! and vacancy-rich metal as a dim neutral. Where the two meet (`4·o·c` high) a pale
//! Starlight rim marks the reaction front, and every CO₂ event splats a soft halo into a
//! per-site glow (blurred like the coverage) fading in ≈ 0.2 s that lifts the domain around it: the fronts shimmer, and no
//! single site ever shows as an isolated pixel.

use std::fmt::Write as _;

use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster;
use sciart_core::title;
use sciart_core::{Tone, draw, ease, ink, rng};

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

/// Glow e-folding time (simulated seconds): a flash is gone in ≈ 0.2 s. Events add
/// [`GLOW_PER_EVENT`] to their site; the glow is box-blurred like the coverage, so its
/// cost is per site, not per event.
const GLOW_TAU: f64 = 0.1;
const GLOW_PER_EVENT: f32 = 12.0;
const GLOW_KNEE: f32 = 1.0;
const GLOW_WHITE: f32 = 14.0;
/// Mix `4·o·c` from which a pixel reads as the front between O and CO domains.
const FRONT_MIX: f64 = 0.45;
/// Coverage smoothing radius, sites, and the wide radius of the O layer's field.
const BLUR: usize = 2;
const WIDE: usize = 4;

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

const O_RAMP: Ramp = Ramp::Aurora;
const CO_RAMP: Ramp = Ramp::Ember;
const FLASH_RAMP: Ramp = Ramp::Starlight;
/// The front between O and CO domains: a faint warm seam, lit by the sparks.
const FRONT_RAMP: Ramp = Ramp::Sulfur;
/// Bare platinum: near-black, so vacancy-rich metal reads as the surface's shadow.
const BARE: Ink = Ink::new(Role::Ramp(FLASH_RAMP), 8);
/// Smoothed O coverage of the reactive surface near `y₁` (≈ y 0.42) and of the O layer
/// (≈ y 0.36; the desorption channel keeps a few vacancies): the O shade rises between them.
const O_REACTIVE: f64 = 0.80;
const O_LAYER: f64 = 0.95;
/// Lift of the O layer over the reactive surface on the Aurora ramp.
const LAYER_LIFT: f64 = 0.07;
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
/// the spark value below which a site only lifts its domain.
const SPARK_SHARE: f64 = 0.012;
const SPARK_TAU: f64 = 0.1;
const SPARK_MIN: f32 = 0.3;
/// Spark peak after its two 3 × 3 box passes (1/9), rescaled to 1: each spark is a soft
/// round flash about five sites across, not a plus sign.
const SPARK_GAIN: f32 = 9.0;
/// Share of the last frame's smoothed coverage kept each update (time smoothing), so
/// domains and fronts glide rather than flicker site by site.
const COVER_KEEP: f32 = 0.8;
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// The CO fraction `y` of the gas at a beat of the sweep: holds at the three regimes,
pub(crate) fn y_at(beat: &Beat) -> f64 {
    let (_, _, a, b) = SWEEP[beat.index.min(SWEEP.len() - 1)];
    ease::lerp(a, b, ease::smoothstep(beat.progress))
}

/// Ink of a pixel from its smoothed `(O, CO)` coverage: the majority species' ramp,
/// brighter the purer the domain; bare, vacancy-rich metal a dim neutral.
pub(crate) fn domain_ink(o: f32, c: f32) -> Ink {
    let (o, c) = (f64::from(o), f64::from(c));
    if o.max(c) < 0.15 {
        return BARE;
    }
    // Quiet grounds: O a deep dark teal, CO a muted amber; purer domains a touch
    // brighter, so islands read by tone as well as hue. The sparks are the bright thing.
    if o >= c {
        // The O layer is lifted from the wide field ([`layer_lift`]).
        ink::ramp(O_RAMP, 0.06 + 0.06 * o)
    } else {
        ink::ramp(CO_RAMP, 0.10 + 0.13 * c)
    }
}

/// Extra level on the O ramp from the *wide* O coverage (a ≈ 17-site tent average, so
/// single vacancy clusters don't draw contours): the O layer sits brighter than the
/// reactive surface. Its edge is not outlined: a bright rim there read as faint cyan
/// contour rings over the layer, with no chemistry behind them.
pub(crate) fn layer_lift(o_wide: f32) -> u8 {
    let o = f64::from(o_wide);
    let t = ease::smoothstep(((o - O_REACTIVE) / (O_LAYER - O_REACTIVE)).clamp(0.0, 1.0));
    (LAYER_LIFT * t * 255.0).round() as u8
}

/// The reaction-front ink where O and CO meet: `Starlight` rising with the mix `4·o·c`
/// above [`FRONT_MIX`], blank inside pure domains.
pub(crate) fn domain_front(o: f32, c: f32) -> Ink {
    let mix = f64::from(4.0 * o * c);
    if mix <= FRONT_MIX {
        return Ink::BLANK;
    }
    let t = ((mix - FRONT_MIX) / (1.0 - FRONT_MIX)).min(1.0);
    ink::ramp(FRONT_RAMP, 0.3 + 0.2 * t)
}

/// Coverage levels per axis of the shading table (fine enough for the O layer's shading).
const LUT_N: usize = 65;

/// The domain ink of a pixel, with the reaction front where O and CO meet (`4·o·c` is 1
/// at an even mix and 0 inside a pure domain).
pub(crate) fn pixel_ink(o: f32, c: f32) -> Ink {
    raster::stronger(domain_ink(o, c), domain_front(o, c))
}

/// Table index of `(o, c)`, each rounded onto `LUT_N` levels (clamped; NaN ⇒ 0).
#[inline]
fn lut_index(o: f32, c: f32) -> usize {
    let q = |v: f32| ((v.clamp(0.0, 1.0) * (LUT_N - 1) as f32 + 0.5) as usize).min(LUT_N - 1);
    q(o) * LUT_N + q(c)
}

/// Periodic box mean of radius `r` over `field`, separable and in place, by running
/// sums (no per-tap modulo); `tmp` is the caller's buffer (grown only on a size change).
/// A radius wider than the lattice wraps as often as it needs.
pub(crate) fn box_blur_wrap(field: &mut Grid<f32>, tmp: &mut Vec<f32>, r: usize) {
    let (w, h) = (usize::from(field.width()), usize::from(field.height()));
    let n = w * h;
    if n == 0 || r == 0 {
        return;
    }
    tmp.resize(n, 0.0);
    let k = 1.0 / (2 * r + 1) as f32;
    let f = field.as_mut_slice();
    // Index `i + d` for d in [−r, r] wrapped onto [0, len).
    let wrap = |i: usize, d: isize, len: usize| (i as isize + d).rem_euclid(len as isize) as usize;
    for y in 0..h {
        let row = &f[y * w..(y + 1) * w];
        let out = &mut tmp[y * w..(y + 1) * w];
        let mut sum: f32 = (-(r as isize)..=r as isize)
            .map(|d| row[wrap(0, d, w)])
            .sum();
        let (mut add, mut sub) = (wrap(0, r as isize + 1, w), wrap(0, -(r as isize), w));
        for v in out.iter_mut() {
            *v = sum * k;
            sum += row[add] - row[sub];
            add = if add + 1 == w { 0 } else { add + 1 };
            sub = if sub + 1 == w { 0 } else { sub + 1 };
        }
    }
    for x in 0..w {
        let at = |y: usize| tmp[y * w + x];
        let mut sum: f32 = (-(r as isize)..=r as isize)
            .map(|d| at(wrap(0, d, h)))
            .sum();
        let (mut add, mut sub) = (wrap(0, r as isize + 1, h), wrap(0, -(r as isize), h));
        for y in 0..h {
            f[y * w + x] = sum * k;
            sum += at(add) - at(sub);
            add = if add + 1 == h { 0 } else { add + 1 };
            sub = if sub + 1 == h { 0 } else { sub + 1 };
        }
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
    /// Per-site CO₂ glow and its blurred copy.
    heat: Grid<f32>,
    heat_blur: Grid<f32>,
    /// Per-site spark brightness (a sparse sample of the events), and the update count
    /// that keys the sampling.
    sparks: Grid<f32>,
    /// The sparks blurred into soft round flashes.
    spark_blur: Grid<f32>,
    frame: u32,
    tone: Tone,
    /// [`pixel_ink`] tabulated on a `LUT_N × LUT_N` grid of coverages.
    lut: [Ink; LUT_N * LUT_N],
    /// Site column of every pixel column and site row of every pixel row.
    cols: Vec<u16>,
    rows: Vec<u16>,
    /// [`layer_lift`] tabulated on `LUT_N` levels of the wide O coverage.
    lift: [u8; LUT_N],
    /// The O coverage averaged wide (for the O layer's lift).
    o_wide: Grid<f32>,
    /// Smoothed local O and CO coverage per site, and the blur's buffer.
    o_cov: Grid<f32>,
    c_cov: Grid<f32>,
    /// This update's box-blurred coverage, before the time smoothing.
    o_fresh: Grid<f32>,
    c_fresh: Grid<f32>,
    tmp: Vec<f32>,
    readout_room: usize,
}

impl Default for CatalyticSurface {
    fn default() -> Self {
        Self {
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
            heat: Grid::new(Size::new(1, 1), 0.0),
            heat_blur: Grid::new(Size::new(1, 1), 0.0),
            sparks: Grid::new(Size::new(1, 1), 0.0),
            spark_blur: Grid::new(Size::new(1, 1), 0.0),
            frame: 0,
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
            lut: std::array::from_fn(|i| {
                let v = |q: usize| q as f32 / (LUT_N - 1) as f32;
                pixel_ink(v(i / LUT_N), v(i % LUT_N))
            }),
            lift: std::array::from_fn(|q| layer_lift(q as f32 / (LUT_N - 1) as f32)),
            o_wide: Grid::new(Size::new(1, 1), 0.0),
            cols: Vec::new(),
            rows: Vec::new(),
            o_cov: Grid::new(Size::new(1, 1), 0.0),
            c_cov: Grid::new(Size::new(1, 1), 0.0),
            o_fresh: Grid::new(Size::new(1, 1), 0.0),
            c_fresh: Grid::new(Size::new(1, 1), 0.0),
            tmp: Vec::new(),
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

    /// Local O and CO coverage: each species' occupancy averaged over the
    /// `(2·BLUR + 1)²` box around every site (periodic), so islands shade as smooth
    /// domains instead of single-site speckle.
    fn smooth_coverage(&mut self) {
        let s = self.zgb.sites.as_slice();
        let size = self.zgb.sites.size();
        for (field, fresh, species) in [
            (&mut self.o_cov, &mut self.o_fresh, OX),
            (&mut self.c_cov, &mut self.c_fresh, CO),
        ] {
            if fresh.size() != size {
                fresh.resize(size, 0.0);
            }
            for (v, &site) in fresh.as_mut_slice().iter_mut().zip(s) {
                *v = f32::from(u8::from(site == species));
            }
            box_blur_wrap(fresh, &mut self.tmp, BLUR);
            // A second, small pass turns the box into a tent: no square blotches.
            box_blur_wrap(fresh, &mut self.tmp, 1);
            if field.size() != size {
                // A fresh lattice starts from the present state, not from a fade-in.
                field.resize(size, 0.0);
                field.as_mut_slice().copy_from_slice(fresh.as_slice());
            }
            for (v, &f) in field.as_mut_slice().iter_mut().zip(fresh.as_slice()) {
                *v = COVER_KEEP * *v + (1.0 - COVER_KEEP) * f;
            }
        }
        if self.o_wide.size() != size {
            self.o_wide.resize(size, 0.0);
        }
        self.o_wide
            .as_mut_slice()
            .copy_from_slice(self.o_cov.as_slice());
        box_blur_wrap(&mut self.o_wide, &mut self.tmp, WIDE);
        box_blur_wrap(&mut self.o_wide, &mut self.tmp, WIDE);
    }

    /// Fade the per-site glow by `keep`, add this update's CO₂ events, and blur a copy.
    fn expose_events(&mut self, keep: f32) {
        let size = self.zgb.sites.size();
        if self.heat.size() != size {
            self.heat.resize(size, 0.0);
        }
        if self.sparks.size() != size {
            self.sparks.resize(size, 0.0);
        }
        let h = self.heat.as_mut_slice();
        h.iter_mut().for_each(|v| *v *= keep);
        // Sparks fade as the glow does: gone in ≈ 0.3 s.
        let spark_keep = keep.powf((GLOW_TAU / SPARK_TAU) as f32);
        let sp = self.sparks.as_mut_slice();
        sp.iter_mut().for_each(|v| *v *= spark_keep);
        self.frame = self.frame.wrapping_add(1);
        for &e in &self.zgb.events {
            if let Some(v) = h.get_mut(e as usize) {
                *v += GLOW_PER_EVENT;
            }
            // A sparse, deterministic sample of the events is drawn as sparks, so they
            // read as single bright points on a quiet surface, not a blizzard.
            let key = rng::cell_key(e as i32, self.frame as i32);
            if rng::hash01(key) < SPARK_SHARE {
                if let Some(v) = sp.get_mut(e as usize) {
                    *v = 1.0;
                }
            }
        }
        if self.heat_blur.size() != size {
            self.heat_blur.resize(size, 0.0);
        }
        self.heat_blur
            .as_mut_slice()
            .copy_from_slice(self.heat.as_slice());
        box_blur_wrap(&mut self.heat_blur, &mut self.tmp, BLUR);
        // Two 3 × 3 passes: a tent kernel five sites across, near round.
        if self.spark_blur.size() != size {
            self.spark_blur.resize(size, 0.0);
        }
        self.spark_blur
            .as_mut_slice()
            .copy_from_slice(self.sparks.as_slice());
        box_blur_wrap(&mut self.spark_blur, &mut self.tmp, 1);
        box_blur_wrap(&mut self.spark_blur, &mut self.tmp, 1);
    }

    fn rasterise(&mut self) {
        self.smooth_coverage();
        // The fields are already box-blurred, so the nearest site is smooth enough; the
        // lattice is 1:1 with the pixels up to the cap, and only larger displays repeat
        // sites.
        let (o_cov, c_cov, heat, o_wide, lifts) = (
            self.o_cov.as_slice(),
            self.c_cov.as_slice(),
            self.heat_blur.as_slice(),
            self.o_wide.as_slice(),
            &self.lift,
        );
        let sparks = self.spark_blur.as_slice();
        let (tone, lut, cols, rows) = (&self.tone, &self.lut, &self.cols, &self.rows);
        let w = usize::from(self.zgb.sites.width());
        self.pixels.shade(|x, y| {
            let i = rows[y as usize] as usize * w + cols[x as usize] as usize;
            let (o, c) = (o_cov[i], c_cov[i]);
            let mut domain = lut[lut_index(o, c)];
            if domain.role == Role::Ramp(O_RAMP) {
                let q = (o_wide[i].clamp(0.0, 1.0) * (LUT_N - 1) as f32 + 0.5) as usize;
                let lift = lifts[q.min(LUT_N - 1)];
                domain = Ink::new(domain.role, domain.level.saturating_add(lift));
            }
            let g = tone.level(heat[i]);
            // CO₂ sparks are the hero: soft round warm-white flashes fading in ≈ 0.3 s;
            // their skirts and the faint event glow only warm the domain around them.
            let spark = (SPARK_GAIN * sparks[i]).min(1.0);
            if spark >= SPARK_MIN {
                let v = (spark - SPARK_MIN) / (1.0 - SPARK_MIN);
                let base = f64::from(domain.level) / 255.0 + f64::from(SPARK_MIN) * 160.0 / 255.0;
                return ink::ramp(FLASH_RAMP, (base + 0.65 * f64::from(v)).min(1.0));
            }
            let lift = u16::from(g) * u16::from(255 - domain.level) / 1600 + (spark * 160.0) as u16;
            Ink::new(
                domain.role,
                domain.level.saturating_add(lift.min(255) as u8),
            )
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
                self.expose_events((-SIM_DT / GLOW_TAU).exp() as f32);
            }
        } else if self.zgb.sites.size() != lattice {
            grid::regrid_nearest(&mut self.zgb.sites, lattice, &mut self.scratch);
            self.zgb.set_grade(GRADE);
        }
        self.zgb.clear_events();
        self.cover = self.zgb.coverage();
        self.pixels.resize(size);
        self.heat.resize(lattice, 0.0);
        self.heat_blur.resize(lattice, 0.0);
        self.sparks.resize(lattice, 0.0);
        self.spark_blur.resize(lattice, 0.0);
        self.o_wide.resize(lattice, 0.0);
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
        self.rasterise();
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
        self.expose_events((-dt / GLOW_TAU).exp() as f32);
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        // ASCII: a density ramp by brightness (sparks brightest), not per-site letters.
        self.pixels.blit(canvas, |_, _, top, bottom| {
            raster::ascii_ramp_cell(top, bottom).filter(|&(_, ink)| ink.level > BARE.level)
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
            for dt in [0.25, SIM_DT, 0.25, 0.1] {
                g.update(dt, &mut ctx);
                assert!(
                    g.last_attempts <= MAX_ATTEMPTS,
                    "{size:?}: {}",
                    g.last_attempts
                );
            }
        }
    }

    #[test]
    fn box_blur_keeps_the_mean_and_spreads_a_lone_site() {
        let mut g = Grid::new(Size::new(9, 7), 0.0f32);
        g.set(0, 0, 1.0);
        let mut tmp = Vec::new();
        box_blur_wrap(&mut g, &mut tmp, 2);
        let sum: f32 = g.as_slice().iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "{sum}");
        // Periodic: the corner's box wraps to the far edges.
        assert!((g.get(8, 6).copied().unwrap_or(0.0) - 1.0 / 25.0).abs() < 1e-6);
        assert!(g.as_slice().iter().all(|&v| v <= 1.0 / 25.0 + 1e-6));
        assert_eq!(domain_ink(0.05, 0.05), BARE);
        assert_eq!(domain_ink(0.9, 0.0).role, Role::Ramp(O_RAMP));
        assert_eq!(domain_ink(0.1, 0.8).role, Role::Ramp(CO_RAMP));
        assert_eq!(domain_front(1.0, 0.0), Ink::BLANK);
        assert_eq!(domain_front(0.5, 0.5).role, Role::Ramp(FRONT_RAMP));
    }

    #[test]
    fn cat_a8_the_loop_opens_mid_window_on_balanced_domains_with_sparks() {
        for seed in [1, 42] {
            let size = Size::new(80, 24);
            let mut ctx = Ctx::new(seed, size);
            let mut g = CatalyticSurface::default();
            g.resize(size, &mut ctx);
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
        let mut seen = [false; 3];
        for _ in 0..(40.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
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
            let loop_s: f64 = SWEEP.iter().map(|p| p.1).sum();
            let (mut poisoned_s, mut min_rate) = (0.0, f64::INFINITY);
            let mut last = g.zgb.produced;
            for _ in 0..(loop_s / 0.5) as usize {
                for _ in 0..30 {
                    g.update(SIM_DT, &mut ctx);
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
    fn cat_a7_the_o_layer_is_brighter_without_an_outline_and_sparks_are_soft_round_flashes() {
        // The layer's lift rises monotonically from the reactive surface to the layer: no
        // rim peaks at its edge (it drew faint contour rings).
        let lifts: Vec<u8> = (0..=20)
            .map(|k| layer_lift((O_REACTIVE + (O_LAYER - O_REACTIVE) * f64::from(k) / 20.0) as f32))
            .collect();
        assert!(lifts.windows(2).all(|w| w[0] <= w[1]), "{lifts:?}");
        assert!(layer_lift(O_LAYER as f32) > layer_lift(O_REACTIVE as f32));
        // One spark blurred twice by 3 × 3 boxes: a round tent, its peak rescaled to 1,
        // with the diagonal neighbours lit as the axial ones are (not a plus sign).
        let mut g = Grid::new(Size::new(9, 9), 0.0f32);
        g.set(4, 4, 1.0);
        let mut tmp = Vec::new();
        box_blur_wrap(&mut g, &mut tmp, 1);
        box_blur_wrap(&mut g, &mut tmp, 1);
        let at = |x: i32, y: i32| SPARK_GAIN * g.get(x, y).copied().unwrap_or(0.0);
        assert!((at(4, 4) - 1.0).abs() < 1e-5);
        assert!(
            at(5, 4) >= SPARK_MIN && at(5, 5) >= SPARK_MIN,
            "{} {}",
            at(5, 4),
            at(5, 5)
        );
        // A round disc of radius ≈ 2: the far diagonal stays dark.
        assert!(at(6, 5) < SPARK_MIN && at(6, 6) < SPARK_MIN);
    }
}
