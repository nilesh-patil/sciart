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
//! The sweep of `y` runs on a looping [`Timeline`] in simulated seconds (45 s), so a
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
const Y_OXYGEN: f64 = 0.30;
const Y_REACTIVE: f64 = 0.46;
const Y_NUCLEATE: f64 = 0.54;
const Y_INVADE: f64 = 0.40;
const Y_CO: f64 = 0.60;
/// Coverage at which the caption calls the surface poisoned (CAT-A5).
pub(crate) const POISONED: f64 = 0.8;

/// Share of attempts that are desorption attempts instead of adsorptions, and the chance
/// an O picked by one leaves (CO always does): slow enough that the poisoned phases keep
/// ≥ 90 % coverage, fast enough that the surface recovers within a few seconds.
pub(crate) const DESORB: f64 = 0.004;
const O_DESORB: f64 = 0.5;

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
/// Coverage smoothing radius, sites.
const BLUR: usize = 2;

/// The 30 s sweep: (label, seconds, y at the start, y at the end), smoothstep between.
/// It opens just above `y₂`, where CO islands nucleate and grow through the O-rich
/// surface; the CO-poisoned surface is then dropped below `y₁`, so O islands eat into the
/// CO sea along bright reaction fronts; a short O-poisoned hold closes the loop.
const SWEEP: [(&str, f64, f64, f64); 8] = [
    ("CO islands", 10.0, Y_NUCLEATE, Y_NUCLEATE),
    ("to CO", 6.0, Y_NUCLEATE, Y_CO),
    ("CO", 1.0, Y_CO, Y_CO),
    ("to O", 4.0, Y_CO, Y_INVADE),
    ("O islands", 8.0, Y_INVADE, Y_INVADE),
    ("to O-poisoned", 3.0, Y_INVADE, Y_OXYGEN),
    ("O-poisoned", 1.5, Y_OXYGEN, Y_OXYGEN),
    ("to CO islands", 4.0, Y_OXYGEN, Y_NUCLEATE),
];

const O_RAMP: Ramp = Ramp::Aurora;
const CO_RAMP: Ramp = Ramp::Ember;
const FLASH_RAMP: Ramp = Ramp::Starlight;
/// The front between O and CO domains: a faint warm seam, lit by the sparks.
const FRONT_RAMP: Ramp = Ramp::Sulfur;
/// Bare platinum: near-black, so vacancy-rich metal reads as the surface's shadow.
const BARE: Ink = Ink::new(Role::Ramp(FLASH_RAMP), 8);
/// Share of CO₂ events drawn as sparks, their fade time (s), and the spark value below
/// which a site shows its domain again.
const SPARK_SHARE: f64 = 0.02;
const SPARK_TAU: f64 = 0.12;
const SPARK_MIN: f32 = 0.12;
/// A spark's brightness on its four neighbours, relative to its own.
const HALO: f32 = 0.45;
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
        ink::ramp(O_RAMP, 0.07 + 0.11 * o)
    } else {
        ink::ramp(CO_RAMP, 0.10 + 0.13 * c)
    }
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

/// Coverage levels per axis of the shading table.
const LUT_N: usize = 33;

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
}

impl Zgb {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            sites: Grid::new(size, EMPTY),
            // An attempt makes at most two molecules (an O₂ landing next to two CO).
            events: Vec::with_capacity(2 * MAX_ATTEMPTS as usize),
            produced: 0,
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

    /// One ZGB attempt at CO fraction `y` (CAT-A3): adsorption never overfills a site,
    /// and O₂ needs two empty neighbouring sites.
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
    frame: u32,
    tone: Tone,
    /// [`pixel_ink`] tabulated on a `LUT_N × LUT_N` grid of coverages.
    lut: [Ink; LUT_N * LUT_N],
    /// Site column of every pixel column and site row of every pixel row.
    cols: Vec<u16>,
    rows: Vec<u16>,
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
            frame: 0,
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
            lut: std::array::from_fn(|i| {
                let v = |q: usize| q as f32 / (LUT_N - 1) as f32;
                pixel_ink(v(i / LUT_N), v(i % LUT_N))
            }),
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
            if field.size() != size {
                // A fresh lattice starts from the present state, not from a fade-in.
                field.resize(size, 0.0);
                field.as_mut_slice().copy_from_slice(fresh.as_slice());
            }
            for (v, &f) in field.as_mut_slice().iter_mut().zip(fresh.as_slice()) {
                *v = COVER_KEEP * *v + (1.0 - COVER_KEEP) * f;
            }
        }
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
        // Sparks fade more slowly than the glow: gone in ≈ 0.3 s.
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
    }

    fn rasterise(&mut self) {
        self.smooth_coverage();
        // The fields are already box-blurred, so the nearest site is smooth enough; the
        // lattice is 1:1 with the pixels up to the cap, and only larger displays repeat
        // sites.
        let (o_cov, c_cov, heat) = (
            self.o_cov.as_slice(),
            self.c_cov.as_slice(),
            self.heat_blur.as_slice(),
        );
        let sharp = self.sparks.as_slice();
        let (tone, lut, cols, rows) = (&self.tone, &self.lut, &self.cols, &self.rows);
        let w = usize::from(self.zgb.sites.width());
        self.pixels.shade(|x, y| {
            let i = rows[y as usize] as usize * w + cols[x as usize] as usize;
            let (o, c) = (o_cov[i], c_cov[i]);
            let domain = lut[lut_index(o, c)];
            let g = tone.level(heat[i]);
            // A spark lights its site and, dimmer, its four neighbours.
            let (sx, sy) = (i % w, i / w);
            let hh = sharp.len() / w.max(1);
            let at = |x: usize, y: usize| sharp[y * w + x];
            let halo = at((sx + 1) % w, sy)
                .max(at((sx + w - 1) % w, sy))
                .max(at(sx, (sy + 1) % hh))
                .max(at(sx, (sy + hh - 1) % hh));
            let spark = sharp[i].max(HALO * halo);
            // CO₂ sparks are the hero: warm-white points of light fading in ≈ 0.3 s; a
            // faint glow only warms the domain around them.
            if spark >= SPARK_MIN {
                return ink::ramp(FLASH_RAMP, 0.3 + 0.7 * f64::from(spark));
            }
            let lift = u16::from(g) * u16::from(255 - domain.level) / 1600;
            Ink::new(domain.role, domain.level.saturating_add(lift as u8))
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
            let phases: [(&str, f64); 8] = std::array::from_fn(|i| (SWEEP[i].0, SWEEP[i].1));
            self.schedule = Some(Timeline::new(&phases));
            self.rng = SplitMix64::from_rng(&mut ctx.rng);
            self.zgb = Zgb::new(lattice);
            let n = self.zgb.sites.len() as u32;
            for _ in 0..PREROLL_SWEEPS {
                self.run(n, Y_REACTIVE);
                self.zgb.clear_events();
            }
        } else if self.zgb.sites.size() != lattice {
            grid::regrid_nearest(&mut self.zgb.sites, lattice, &mut self.scratch);
        }
        self.zgb.clear_events();
        self.cover = self.zgb.coverage();
        self.pixels.resize(size);
        self.heat.resize(lattice, 0.0);
        self.heat_blur.resize(lattice, 0.0);
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
    fn cat_a5_captions_follow_the_state_of_the_surface() {
        assert_eq!(regime((0.85, 0.0)), 0);
        assert_eq!(regime((0.3, 0.1)), 1);
        assert_eq!(regime((0.0, 0.95)), 2);
        let mut ctx = Ctx::new(1, Size::new(80, 24));
        let mut g = CatalyticSurface::default();
        g.resize(Size::new(80, 24), &mut ctx);
        let mut seen = [false; 3];
        for _ in 0..(46.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
            let r = regime(g.zgb.coverage());
            assert_eq!(g.caption(), Some(CAPTIONS[r]));
            seen[r] = true;
        }
        assert_eq!(seen, [true; 3], "the sweep reaches all three regimes");
    }
}
