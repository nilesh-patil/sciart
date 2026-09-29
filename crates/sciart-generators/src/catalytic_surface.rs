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
//! One site per half-block pixel at 200 × 50. Adsorbates by named ramp, brighter inside
//! an island than on its edge; vacancies a dim neutral, so they read as bare metal. Every
//! CO₂ event splats a soft halo into a [`Glow`] that fades in ≈ 0.2 s; the glow lifts the
//! ink of the sites around it and turns white only where reactions crowd, so the reaction
//! fronts shimmer instead of speckling.

use std::fmt::Write as _;

use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::{self, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::title;
use sciart_core::{Tone, draw, ease, ink};

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
const PREROLL_SWEEPS: usize = 3;

/// Glow e-folding time (simulated seconds): a flash is gone in ≈ 0.2 s.
const GLOW_TAU: f64 = 0.07;
/// Soft 5 × 5 halo per CO₂ event, and the tone its sum goes through.
const HALO: [(i32, i32, f32); 25] = halo_kernel::<25>(2);
const GLOW_KNEE: f32 = 1.0;
const GLOW_WHITE: f32 = 14.0;
/// Glow level above which a pixel turns Starlight white instead of a lifted adsorbate.
const WHITE_FROM: u8 = 190;

/// Sweep phases: (label, seconds). Holds and ramps; see [`y_at`].
const PHASES: [(&str, f64); 7] = [
    ("reactive", 6.0),
    ("to CO", 6.0),
    ("CO", 7.0),
    ("to O", 6.0),
    ("O", 7.0),
    ("to reactive", 6.0),
    ("reactive late", 7.0),
];

const O_RAMP: Ramp = Ramp::Ferroin;
const CO_RAMP: Ramp = Ramp::Aurora;
const FLASH_RAMP: Ramp = Ramp::Starlight;
/// Bare platinum: a dim neutral, so vacancies read as metal, not holes.
const BARE: Ink = Ink::new(Role::Ramp(FLASH_RAMP), 26);
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// The CO fraction `y` of the gas at a beat of the sweep: holds at the three regimes,
/// smoothstep ramps between them.
pub(crate) fn y_at(beat: &Beat) -> f64 {
    let ramp = |a: f64, b: f64| ease::lerp(a, b, ease::smoothstep(beat.progress));
    match beat.index {
        0 | 6 => Y_REACTIVE,
        1 => ramp(Y_REACTIVE, Y_CO),
        2 => Y_CO,
        3 => ramp(Y_CO, Y_OXYGEN),
        4 => Y_OXYGEN,
        _ => ramp(Y_OXYGEN, Y_REACTIVE),
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
    glow: Glow,
    tone: Tone,
    map: SampleMap,
    /// Per-site ink of the last rasterisation, sampled into the pixels.
    inks: Vec<Ink>,
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
            glow: Glow::new(Size::new(1, 1)),
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
            map: SampleMap::default(),
            inks: Vec::new(),
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

    /// Ink of every site: species by ramp, brighter inside an island than on its edge
    /// (same-species neighbours), with a fixed per-site grain so atoms read as atoms.
    fn shade_sites(&mut self) {
        let z = &self.zgb;
        let (w, h) = (usize::from(z.sites.width()), usize::from(z.sites.height()));
        let s = z.sites.as_slice();
        self.inks.resize(s.len(), Ink::BLANK);
        for (i, ink) in self.inks.iter_mut().enumerate() {
            let state = s[i];
            if state == EMPTY {
                *ink = BARE;
                continue;
            }
            let (x, y) = (i % w, i / w);
            let same = [
                y * w + (x + 1) % w,
                ((y + 1) % h) * w + x,
                y * w + (x + w - 1) % w,
                ((y + h - 1) % h) * w + x,
            ]
            .iter()
            .filter(|&&j| s[j] == state)
            .count();
            let grain = hash01(cell_key(x as i32, y as i32)) - 0.5;
            let (base, step) = if state == OX {
                (0.26, 0.05)
            } else {
                (0.36, 0.06)
            };
            let v = base + step * same as f64 + 0.05 * grain;
            *ink = ink::ramp(if state == OX { O_RAMP } else { CO_RAMP }, v);
        }
    }

    /// Splat this update's CO₂ events into the glow at their sites' pixel centres.
    fn splat_events(&mut self) {
        let (lw, lh) = (
            f64::from(self.zgb.sites.width()),
            f64::from(self.zgb.sites.height()),
        );
        let px = self.pixels.pixel_size();
        let (sx, sy) = (f64::from(px.width()) / lw, f64::from(px.height()) / lh);
        let w = u32::from(self.zgb.sites.width());
        for &e in &self.zgb.events {
            let (x, y) = (f64::from(e % w), f64::from(e / w));
            // Pixel centre → dot coordinates (2 × 2 dots a pixel).
            let (cx, cy) = ((x + 0.5) * sx, (y + 0.5) * sy);
            self.glow.splat(2.0 * cx - 0.5, 2.0 * cy - 0.5, &HALO, 1.0);
        }
    }

    fn rasterise(&mut self) {
        self.shade_sites();
        let (map, inks, glow, tone) = (&self.map, &self.inks, &self.glow, &self.tone);
        let w = i32::from(self.zgb.sites.width());
        self.pixels.shade(|x, y| {
            let (sx, sy) = map.nearest(x, y);
            let site = inks
                .get((sy * w + sx) as usize)
                .copied()
                .unwrap_or(Ink::BLANK);
            let g = tone.level(glow.get(x, y).unwrap_or(0.0));
            if g >= WHITE_FROM {
                // Crowded reactions: the light itself.
                return raster::stronger(Ink::new(Role::Ramp(FLASH_RAMP), g), site);
            }
            // Otherwise lift the site's own ink toward its ramp's pale end.
            let lift = u16::from(g) * u16::from(255 - site.level) / 300;
            Ink::new(site.role, site.level.saturating_add(lift as u8))
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
            self.schedule = Some(Timeline::new(&PHASES));
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
        self.glow.resize(size);
        self.map.rebuild(lattice, self.pixels.pixel_size());
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
        self.glow.decay((-dt / GLOW_TAU).exp() as f32);
        self.splat_events();
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels.blit(canvas, |_, _, top, bottom| {
            let pick = raster::stronger(top, bottom);
            let ch = match pick.role {
                Role::Ramp(FLASH_RAMP) if pick.level >= WHITE_FROM => '*',
                Role::Ramp(O_RAMP) => 'o',
                Role::Ramp(CO_RAMP) => 'c',
                _ => return None,
            };
            Some((ch, pick))
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
