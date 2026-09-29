//! Totality seen from the ground (SPEC-002 `total_solar_eclipse`): the Moon slides over
//! the Sun, the sky darkens to deep twilight with a sunset glow all round the horizon,
//! the last sunlight breaks into Baily's beads and a diamond ring, then the pearly corona
//! blooms with long streamers and small pink prominences at the limb; then it all runs
//! backwards.
//!
//! Geometry is in solar radii about the Sun's centre. The Moon (1.03 R☉) moves along a
//! tilted chord through the Sun's centre; its offset `d(t)` is driven by a `Timeline`, so
//! the eclipse is symmetric in time (ECL-A1). The Moon's limb is a seeded valley profile:
//! once its mountain-top envelope covers the Sun, sunlight shows only through valleys whose
//! floors still dip inside the photosphere, which are the beads (ECL-A2).

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::{self, Glow, Tone, halo_kernel};
use sciart_core::stellar;
use sciart_core::timeline::ease;

use crate::shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "total_solar_eclipse",
    category: Category::EarthSky,
    description: "The Moon hides the Sun: Baily's beads, the diamond ring, the pearly corona",
    title: "Total eclipse · Moon hides Sun",
    captions: &["partial", "diamond ring", "totality"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Eclipse::default())
}

/// Moon radius (solar radii) and the depth of its deepest limb valley (stylised: real
/// lunar relief is ~0.002 R☉; deepened so beads last long enough to see).
const MOON_R: f64 = 1.03;
const VALLEY: f64 = 0.02;
/// Limb profile samples (one per degree) and valleys cut into it.
const LIMB_N: usize = 360;
const VALLEYS: usize = 14;
/// Where the Moon starts and ends, beyond first contact.
const START: f64 = MOON_R + 1.0 + 0.08;
/// The Moon's path, tilted from the horizontal (rad).
const PATH_TILT: f64 = -0.35;
/// Acts (s): partial, beads and diamond ring, totality, and back (ECL-A5).
const PHASES: [(&str, f64); 5] = [
    ("partial", 8.0),
    ("diamond ring", 4.0),
    ("totality", 14.0),
    ("diamond ring", 4.0),
    ("partial", 8.0),
];
/// Photosphere: centre temperature and limb darkening (quadratic, visible light).
const T_SUN: f64 = 5800.0;
const LIMB_DARK: (f64, f64) = (0.47, 0.23);
/// The corona shows once the uncovered fraction falls below this (ECL-A3).
const CORONA_F: f64 = 0.01;
/// Streamers: count range, and the corona's radial falloff exponent.
const STREAMERS: (usize, usize) = (6, 10);
const FALLOFF: f64 = 3.3;
/// Corona tone (knee, white) and its peak `Ramp(Starlight)` level.
const CORONA_TONE: (f32, f32) = (0.35, 2.2);
/// How much the Moon's approaching shadow darkens its side of the sky in partial phases.
const SHADOW: f64 = 0.6;
/// The one ramp for the air, its glare, the corona and the beads' halo: navy twilight at
/// the bottom, pale steel-blue in the middle, pearl white at the top.
const SKY: Ramp = Ramp::Ice;
/// Sky: twilight level near the horizon in totality and in sunlight, and the glare around
/// the uncovered Sun (e-folding `GLARE_R` solar radii beyond the limb).
const SKY_DARK: f64 = 0.0;
const SKY_LIT: f64 = 0.0;
const GLARE: f64 = 95.0;
const GLARE_R: f64 = 1.1;
/// GEN-R18: the air is black overhead. Its light ramps in (smoothstep) from `SKY_TOP` of
/// the picture's height down to the horizon, so only the lower sky holds the dim blue.
const SKY_TOP: f64 = 0.55;
/// Below these levels the light is drawn as true black (blank): the sky's `Ice` has a navy
/// floor, and the corona's pearl fades out into black rather than into a blue sky.
const SKY_BLANK: f64 = 2.0;
const SKY_DISSOLVE: u8 = 16;
const PEARL_BLANK: f64 = 10.0;
/// Corona rows filled per frame while the corona is hidden (GEN-R19).
const CORONA_ROWS: usize = 2;
/// Tone mapping of the summed light: linear to the knee, then an exponential roll-off to
/// the ceiling, so bright cores never clip flat.
const KNEE: f64 = 140.0;
const CEIL: f64 = 245.0;
/// Sky levels below this stay blank in ASCII (the daylit sky with its glare is ~100).
const ASCII_SKY: u8 = 110;
/// Horizon glow: peak `Ramp(Ember)` level and its height (share of the picture).
const GLOW_PEAK: f64 = 0.0;
/// The hills' top edge, the only lit land.
const SKYLINE: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 34);
const GLOW_H: f64 = 0.1;
/// Prominences: count range, heights (solar radii) and `Ramp(HAlpha)` peak.
const PROMS: (usize, usize) = (3, 6);
const PROM_H: (f64, f64) = (0.07, 0.16);
const PROM_PEAK: f64 = 200.0;
/// Beads' glare kernels: every bead, and the diamond (the brightest, alone).
const BEAD: [(i32, i32, f32); 25] = halo_kernel(2);
const DIAMOND: [(i32, i32, f32); 121] = halo_kernel(5);
const BEAD_TONE: (f32, f32) = (0.05, 1.0);
/// Beads are drawn this far outside the lunar limb (px).
const BEAD_LIFT: f64 = 0.8;
/// Hills on the horizon.
const HILLS: Terrain = Terrain {
    ridge: (0.8, 3.0),
    trees: Some((0.8, 2.0)),
    gap: (0.4, 1.5),
    clearing: 0.5,
};

/// Area of the lens where discs of radius `r1`, `r2` with centres `d` apart overlap.
pub(crate) fn overlap(r1: f64, r2: f64, d: f64) -> f64 {
    let d = d.abs();
    if d >= r1 + r2 {
        return 0.0;
    }
    if d <= (r1 - r2).abs() {
        let r = r1.min(r2);
        return PI * r * r;
    }
    let a1 = ((d * d + r1 * r1 - r2 * r2) / (2.0 * d * r1))
        .clamp(-1.0, 1.0)
        .acos();
    let a2 = ((d * d + r2 * r2 - r1 * r1) / (2.0 * d * r2))
        .clamp(-1.0, 1.0)
        .acos();
    let k = ((-d + r1 + r2) * (d + r1 - r2) * (d - r1 + r2) * (d + r1 + r2)).max(0.0);
    r1 * r1 * a1 + r2 * r2 * a2 - 0.5 * k.sqrt()
}

/// The Moon's seeded limb: valley depth (solar radii) per degree.
#[derive(Debug, Clone)]
pub(crate) struct Limb {
    depth: [f64; LIMB_N],
}

impl Default for Limb {
    fn default() -> Self {
        Self {
            depth: [0.0; LIMB_N],
        }
    }
}

impl Limb {
    fn seeded(rng: &mut impl Rng) -> Self {
        let mut depth = [0.0; LIMB_N];
        for _ in 0..VALLEYS {
            let at = rng.random_range(0.0..LIMB_N as f64);
            let half = rng.random_range(1.5..5.0);
            let deep = VALLEY * rng.random_range(0.35..1.0);
            for (i, d) in depth.iter_mut().enumerate() {
                let off = (i as f64 - at + LIMB_N as f64 * 1.5).rem_euclid(LIMB_N as f64)
                    - LIMB_N as f64 / 2.0;
                let v = deep * (1.0 - (off / half).powi(2)).max(0.0);
                *d = f64::max(*d, v);
            }
        }
        Self { depth }
    }

    /// Light through each valley, as `(angle, floor point, penetration)`: the valley floor
    /// at angle `θ` lies `MOON_R − depth` from the Moon's centre `m`; it lets sunlight
    /// through when it is inside the Sun, by `1 − |floor|` solar radii.
    fn beads(&self, m: (f64, f64)) -> impl Iterator<Item = (f64, (f64, f64), f64)> + '_ {
        self.depth.iter().enumerate().filter_map(move |(i, &v)| {
            let a = TAU * i as f64 / LIMB_N as f64;
            let (s, c) = a.sin_cos_det();
            let p = (m.0 + (MOON_R - v) * c, m.1 + (MOON_R - v) * s);
            let gap = 1.0 - p.0.hypot(p.1);
            (gap > 0.0).then_some((a, p, gap))
        })
    }
}

/// The share of the Sun's disc left uncovered with the Moon `d` from its centre: the
/// smooth envelope's crescent plus, once the envelope covers the Sun, the light through
/// the valleys (each one degree of limb wide).
pub(crate) fn uncovered(d: f64, limb: &Limb) -> f64 {
    let smooth = 1.0 - overlap(1.0, MOON_R, d) / PI;
    if smooth > 1e-12 {
        return smooth;
    }
    let (s, c) = PATH_TILT.sin_cos_det();
    let arc = TAU / LIMB_N as f64;
    limb.beads((d * c, d * s))
        .map(|(_, _, gap)| gap * arc)
        .sum::<f64>()
        / PI
}

/// The Moon's offset along its path at loop time `t` (ECL-A1): approach, the beads while
/// the valleys close, totality, and out again, symmetric about mid-totality.
pub(crate) fn moon_offset(timeline: &Timeline, t: f64) -> f64 {
    let beat = timeline.at(t);
    let contact = MOON_R - 1.0; // the envelope just covers the Sun
    let closed = contact - VALLEY; // the deepest valley has closed
    let p = beat.progress;
    match beat.index {
        0 => -START + (START - contact) * p,
        1 => -contact + (contact - closed) * p,
        2 => -closed + 2.0 * closed * p,
        3 => closed + (contact - closed) * p,
        _ => contact + (START - contact) * p,
    }
}

/// Corona brightness at `r` solar radii (> 1) and angle `a`, before toning: a K-corona
/// falling as `r^{−FALLOFF}`, helmet streamers that taper and bend toward the equator,
/// polar plumes and fine radial striations.
#[derive(Debug, Clone, Default)]
pub(crate) struct Corona {
    /// Equator angle, and per streamer: base angle, width, strength, length.
    equator: f64,
    streamers: Vec<(f64, f64, f64, f64)>,
    phases: [f64; 3],
}

impl Corona {
    fn seeded(rng: &mut impl Rng) -> Self {
        let equator = rng.random_range(-0.3..0.3);
        let n = rng.random_range(STREAMERS.0..=STREAMERS.1);
        let mut streamers = Vec::with_capacity(STREAMERS.1);
        streamers.extend((0..n).map(|i| {
            // Solar minimum: streamers crowd the equator on both sides.
            let side = if i % 2 == 0 { 0.0 } else { PI };
            let lat = rng.random_range(-0.55f64..0.55);
            let near = 1.0 - (lat.abs() / 0.55);
            (
                equator + side + lat,
                rng.random_range(0.08..0.16),
                rng.random_range(0.6..1.4) * (0.5 + 0.5 * near),
                0.8 + 3.0 * near * rng.random_range(0.5..1.0),
            )
        }));
        Self {
            equator,
            streamers,
            phases: std::array::from_fn(|_| rng.random_range(0.0..TAU)),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(&self, r: f64, a: f64) -> f64 {
        self.shape(r, a, true)
    }

    /// The corona's broad shape, with (`fine`) or without its fine rays (the plumes' and
    /// striations' angular ripple): the pearl-or-blue choice follows the broad shape, so
    /// its contour is smooth while the rays still show in the lightness.
    #[cfg(test)]
    fn shape(&self, r: f64, a: f64, fine: bool) -> f64 {
        let (f, b) = self.shapes(r, a);
        if fine { f } else { b }
    }

    /// `shape` with and without the fine rays at once (GEN-R19: the streamer sum is shared).
    fn shapes(&self, r: f64, a: f64) -> (f64, f64) {
        if r <= 1.0 {
            return (0.0, 0.0);
        }
        let r09 = r.powf(0.9);
        let wr = (1.3 / r).sqrt();
        let base = r.powf(-FALLOFF);
        let mut streams = 0.0;
        for &(at, width, strength, length) in &self.streamers {
            // Helmet streamers bend toward the equator and narrow to a point.
            let eq = if (at - self.equator).cos_det() > 0.0 {
                self.equator
            } else {
                self.equator + PI
            };
            let centre = eq + wrap(at - eq) * (0.55 + 0.45 / r);
            let w = width * wr;
            let off = wrap(a - centre) / w;
            streams += strength * (-0.5 * off * off).exp() * (-(r - 1.0) / length).exp() * r09;
        }
        // Polar plumes: fine short rays within 30° of the poles.
        let pole = wrap(a - self.equator - PI / 2.0)
            .abs()
            .min(wrap(a - self.equator + PI / 2.0).abs());
        let plume = |ripple: f64| {
            0.35 * ease::smoothstep(1.0 - pole / 0.5) * ripple * (-(r - 1.0) / 0.5).exp()
        };
        let ripple = 0.5 + 0.5 * (48.0 * a + self.phases[0]).cos_det();
        let striae = 1.0
            + 0.12 * (37.0 * a + self.phases[1]).sin_det()
            + 0.08 * (83.0 * a + self.phases[2]).sin_det();
        (
            base * (0.35 + streams + plume(ripple)) * striae,
            base * (0.35 + streams + plume(0.5)),
        )
    }
}

/// Summed light (level units) → level: linear below [`KNEE`], rolling off smoothly toward
/// [`CEIL`] above it (continuous in value and slope).
pub(crate) fn soft_clip(v: f64) -> f64 {
    if v <= KNEE {
        v.max(0.0)
    } else {
        let room = CEIL - KNEE;
        KNEE + room * (1.0 - (-(v - KNEE) / room).exp())
    }
}

/// `a` wrapped onto `(−π, π]`.
fn wrap(a: f64) -> f64 {
    sciart_core::geom::wrap_angle(a)
}

#[derive(Debug, Clone, Copy)]
struct Prominence {
    angle: f64,
    half: f64,
    height: f64,
}

#[derive(Debug, Default)]
pub struct Eclipse {
    timeline: Option<Timeline>,
    limb: Limb,
    corona_model: Corona,
    proms: Vec<Prominence>,
    /// Picture geometry: Sun centre (px), solar radius (px).
    centre: (f64, f64),
    radius: f64,
    /// Per pixel: raw corona brightness, and prominence light (0..1).
    corona: Vec<u8>,
    /// Per pixel: the corona's broad shape without its fine rays, toned (the pearl test).
    broad: Vec<u8>,
    prom: Vec<u8>,
    /// Per pixel: horizon glow shape (0..1) and land cover (0..1).
    glow: Vec<f32>,
    land: Vec<u8>,
    /// Per pixel: the glare's falloff round the Sun, `exp(−(r − 1)/GLARE_R)` as a byte.
    glare: Vec<u8>,
    /// Corona rows filled so far (`fill_corona`).
    corona_rows: usize,
    /// Per pixel row: the air's share of its light by height (0 overhead, GEN-R18).
    rows: Vec<f32>,
    /// A few bright planets and stars that come out in totality (px, level).
    lights: Vec<(i32, i32, u8)>,
    beads: Glow,
    bead_tone: Tone,
    picture: Pixels,
    t: f64,
    /// Uncovered fraction and Moon offset now.
    f: f64,
    d: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl Eclipse {
    fn timeline(&self) -> Option<&Timeline> {
        self.timeline.as_ref()
    }

    /// Solar radii → pixel position.
    fn to_px(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.centre.0 + p.0 * self.radius,
            self.centre.1 + p.1 * self.radius,
        )
    }

    fn moon(&self) -> (f64, f64) {
        let (s, c) = PATH_TILT.sin_cos_det();
        (self.d * c, self.d * s)
    }

    /// Corona visibility: 0 while the photosphere's glare drowns it, 1 in totality.
    fn corona_vis(&self) -> f64 {
        ease::smoothstep((CORONA_F - self.f) / CORONA_F)
    }

    fn paint(&mut self) {
        let ps = self.picture.pixel_size();
        let w = usize::from(ps.width());
        let (f, vis) = (self.f, self.corona_vis());
        let moon = self.to_px(self.moon());
        let (sun, r) = (self.centre, self.radius);
        let mr = MOON_R * r;
        // Beads and the diamond ring: glare from each valley letting light through.
        self.beads.clear();
        let mut total = 0.0;
        let mut brightest = (0.0, (0.0, 0.0));
        if 1.0 - overlap(1.0, MOON_R, self.d) / PI <= 1e-12 {
            let m = self.moon();
            let pts: [(f64, (f64, f64)); LIMB_N] = {
                let mut a = [(0.0, (0.0, 0.0)); LIMB_N];
                for (slot, (_, p, gap)) in a.iter_mut().zip(self.limb.beads(m)) {
                    *slot = (gap, p);
                }
                a
            };
            for &(gap, p) in pts.iter().filter(|(g, _)| *g > 0.0) {
                // The bead sits just outside the lunar limb (its glare is masked by the
                // Moon's disc), so its core is a pixel beyond the limb, never on the disc.
                let (ox, oy) = (p.0 - m.0, p.1 - m.1);
                let len = ox.hypot(oy).max(1e-9);
                let lift = (MOON_R - len + BEAD_LIFT / self.radius.max(1e-9)) / len;
                let px = self.to_px((p.0 + ox * lift, p.1 + oy * lift));
                let wgt = (gap / VALLEY) as f32;
                self.beads
                    .splat(2.0 * px.0 + 0.5, 2.0 * px.1 + 0.5, &BEAD, wgt);
                total += gap;
                if gap > brightest.0 {
                    brightest = (gap, px);
                }
            }
            if brightest.0 > 0.0 {
                // The diamond: one bead's flare, strongest when it is almost alone.
                let lone = (brightest.0 / total.max(1e-12)).clamp(0.0, 1.0) as f32;
                let px = brightest.1;
                self.beads.splat(
                    2.0 * px.0 + 0.5,
                    2.0 * px.1 + 0.5,
                    &DIAMOND,
                    0.35 + 0.65 * lone * (brightest.0 / VALLEY) as f32,
                );
            }
        }
        // The sky dims with the light left (a log-ish eye response), toward deep twilight.
        let sky = SKY_DARK + (SKY_LIT - SKY_DARK) * f.powf(0.8);
        let approach = if self.d < 0.0 { 1.0 } else { -1.0 };
        let glow_k = GLOW_PEAK * (1.0 - f).powf(1.5);
        let (corona, prom, glow, land) = (&self.corona, &self.prom, &self.glow, &self.land);
        let broad = &self.broad;
        let glare = &self.glare;
        let rows = &self.rows;
        let beads = self.beads.values();
        let bt = &self.bead_tone;
        self.picture.shade(|x, y| {
            let i = y as usize * w + x as usize;
            let cover = f64::from(land.get(i).copied().unwrap_or(255)) / 255.0;
            // The hills: black, their top a 1-px dim skyline (ADR-0017).
            if cover >= 0.5 {
                let above = i
                    .checked_sub(w)
                    .and_then(|j| land.get(j))
                    .copied()
                    .unwrap_or(0);
                return if above < 128 { SKYLINE } else { Ink::BLANK };
            }
            let open = 1.0 - cover;
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            // Discs are walked only inside their bounding squares.
            let near = |c: (f64, f64), rad: f64| {
                (fx - c.0).abs() < rad + 1.0 && (fy - c.1).abs() < rad + 1.0
            };
            let moon_cov = if near(moon, mr) {
                raster::disc_cover(x, y, moon, mr)
            } else {
                0.0
            };
            let sun_cov = if near(sun, r) {
                raster::disc_cover(x, y, sun, r)
            } else {
                0.0
            };
            // The photosphere, where the Moon leaves it.
            let lit = sun_cov * (1.0 - moon_cov);
            if lit > 0.9 {
                let rr = (fx - sun.0).hypot(fy - sun.1) / r;
                let mu = (1.0 - rr * rr).max(0.0).sqrt();
                let i = stellar::limb_darkening(mu, LIMB_DARK).sqrt();
                return ink::blackbody(stellar::brightness_temperature(T_SUN, i));
            }
            // Light in front of the Moon's limb (it is anti-aliased by `clear`): the beads'
            // glare, the corona and the prominences stop at the dark disc, which stays whole.
            let clear = (1.0 - moon_cov) * open;
            let b = f64::from(bt.level(beads.get(i).copied().unwrap_or(0.0))) * clear;
            if b >= 200.0 {
                return Ink::new(Role::Blackbody, ink::blackbody(T_SUN).level);
            }
            let c = f64::from(corona.get(i).copied().unwrap_or(0)) * vis * clear;
            let p = f64::from(prom.get(i).copied().unwrap_or(0)) / 255.0 * vis * clear * PROM_PEAK;
            if p >= 30.0 && p > c * 0.6 {
                return Ink::new(Role::Ramp(Ramp::HAlpha), ink::level(p));
            }
            // Sky: twilight with glare round the uncovered Sun, a sunset glow low down. The
            // Moon's shadow sweeps in from the side it comes from: that half of the sky
            // darkens first, the other half last, and back again after totality.
            let side = (fx / w as f64 * 2.0 - 1.0) * -approach;
            let shade = 1.0 - SHADOW * (1.0 - f).powf(0.6) * (0.5 + 0.5 * side) * f.sqrt().min(1.0);
            let high = f64::from(rows.get(y as usize).copied().unwrap_or(1.0));
            // The glare round the uncovered Sun is pale (pearl), the air twilight blue.
            let halo = GLARE * f * f64::from(glare.get(i).copied().unwrap_or(0)) / 255.0;
            // The new Moon: a black silhouette closing in, its edge anti-aliased against the
            // air (and the corona, dimmed above by `clear`).
            let (air, halo) = (
                sky * shade * high * (1.0 - moon_cov),
                halo * (1.0 - moon_cov),
            );
            // The partly covered rim pixels of the crescent add their light too.
            let rim = if lit > 0.02 { 60.0 + 190.0 * lit } else { 0.0 };
            // One ramp for the air, the glare, the corona and the beads' halo, added in
            // light and tone-mapped: the corona's power law fades into the sky with no
            // threshold and no hue edge, and the brightest core rolls off instead of
            // clipping.
            let light = soft_clip((air + halo + c + b + rim) * open);
            let ember = glow_k * f64::from(glow.get(y as usize).copied().unwrap_or(0.0)) * open;
            if ember >= 12.0 && ember >= light {
                return Ink::new(Role::Ramp(Ramp::Ember), ink::level(ember));
            }
            if open < 0.5 || (moon_cov >= 0.5 && rim == 0.0) {
                return Ink::BLANK;
            }
            // Pearl where the corona (or a bead's halo) outshines the air, twilight blue
            // elsewhere. The light is summed first and the ramps share their lightness at
            // equal levels, so this changes hue only, never lightness. The crescent's partly
            // covered rim pixels are always pale (no blue rim). Faint light is black (blank):
            // the corona's outer rays thin out into black, the sky is black overhead.
            let broad = f64::from(broad.get(i).copied().unwrap_or(0)) * vis * clear;
            let (ramp, floor) = if rim > 0.0 || broad + b + halo >= air {
                (Ramp::Starlight, PEARL_BLANK)
            } else {
                (SKY, SKY_BLANK)
            };
            if light < floor {
                return Ink::BLANK;
            }
            Ink::new(Role::Ramp(ramp), ink::level(light))
        });
        // The twilight's navy floor would end in a hard contour: it dissolves into black.
        self.picture.dissolve(SKY_DISSOLVE);
        self.stars(f);
    }

    /// Fill the next `rows` pixel rows of the corona, its broad shape and the prominences
    /// (toned once: the power law alone, with no taper or cut-off, so the outer rays thin
    /// out as far as they reach).
    fn fill_corona(&mut self, rows: usize) {
        let w = usize::from(self.picture.pixel_size().width());
        let total = if w == 0 { 0 } else { self.corona.len() / w };
        let end = self.corona_rows.saturating_add(rows).min(total);
        let (cx, cy, r) = (self.centre.0, self.centre.1, self.radius);
        let tone = Tone::asinh(CORONA_TONE.0, CORONA_TONE.1);
        for y in self.corona_rows..end {
            for x in 0..w {
                let i = y * w + x;
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let (dx, dy) = ((fx - cx) / r, (fy - cy) / r);
                let rr = dx.hypot(dy);
                let a = dy.atan2(dx);
                let (fine, broad) = self.corona_model.shapes(rr, a);
                let level = f64::from(tone.level(fine as f32));
                self.corona[i] = level.round() as u8;
                self.broad[i] = tone.level(broad as f32);
                // Prominences: arches of glowing gas standing on the limb.
                let mut p: f64 = 0.0;
                for pr in &self.proms {
                    let off = wrap(a - pr.angle) / pr.half;
                    if off.abs() < 1.0 {
                        let top = 1.0 + pr.height * (1.0 - off * off).sqrt();
                        let thick = 0.35 * pr.height + 0.5 / r;
                        let d = (rr - top).abs() / thick;
                        let fill = if rr > 1.0 && rr < top { 0.55 } else { 0.0 };
                        p = p.max((1.0 - d).max(0.0).max(fill * (1.0 - off.abs())));
                    }
                }
                self.prom[i] = (p.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        self.corona_rows = end;
    }

    /// Planets and bright stars come out as the sky darkens: Venus first (brightest), then
    /// the others, as the light fails.
    fn stars(&mut self, f: f64) {
        let show = ease::smoothstep((0.12 - f) / 0.12);
        if show <= 0.0 {
            return;
        }
        for &(x, y, level) in &self.lights {
            let l = f64::from(level) * show.powf(1.0 + (255.0 - f64::from(level)) / 40.0);
            if l >= 40.0
                && self.picture.get(x, y).is_some_and(|p| {
                    p == Ink::BLANK || (p.role == Role::Ramp(SKY) && f64::from(p.level) < 0.5 * l)
                })
            {
                self.picture
                    .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), ink::level(l)));
            }
        }
    }
}

impl Generator for Eclipse {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let (px_w, px_h) = (w as f64, 2.0 * h as f64);
        if self.timeline.is_none() {
            self.timeline = Some(Timeline::new(&PHASES));
        }
        self.limb = Limb::seeded(&mut ctx.rng);
        self.corona_model = Corona::seeded(&mut ctx.rng);
        let n = ctx.rng.random_range(PROMS.0..=PROMS.1);
        self.proms.clear();
        self.proms.reserve_exact(PROMS.1);
        self.proms.extend((0..n).map(|_| Prominence {
            angle: ctx.rng.random_range(0.0..TAU),
            half: ctx.rng.random_range(0.04..0.12),
            height: ctx.rng.random_range(PROM_H.0..PROM_H.1),
        }));
        self.radius = (0.1 * px_w).min(0.14 * px_h).max(0.5);
        self.centre = (0.5 * px_w, 0.44 * px_h);
        let (cx, cy, r) = (self.centre.0, self.centre.1, self.radius);
        let deg = (0.5 * px_h / 30.0).max(0.1);
        let hills = Skyline::new(&mut ctx.rng, w, deg, HILLS);
        let horizon = 0.97 * px_h;
        // The corona and prominences are only drawn once the Sun is nearly covered, so
        // they are filled row by row over the partial phase's frames (`fill_corona`).
        let n = w * 2 * h;
        for buf in [&mut self.corona, &mut self.broad, &mut self.prom] {
            buf.clear();
            buf.resize(n, 0);
        }
        self.corona_rows = 0;
        self.glow.clear();
        self.land.clear();
        self.glare.clear();
        for y in 0..2 * h {
            for x in 0..w {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let rr = ((fx - cx) / r).hypot((fy - cy) / r);
                let up = ((horizon - fy) / (GLOW_H * px_h)).max(0.0);
                if x == 0 {
                    self.glow.push((-up * up).exp() as f32);
                }
                self.land
                    .push((hills.cover(x as i32, y as i32, horizon) * 255.0).round() as u8);
                self.glare
                    .push(((-(rr - 1.0).max(0.0) / GLARE_R).exp() * 255.0).round() as u8);
            }
        }
        self.rows.clear();
        self.rows.extend((0..2 * h).map(|y| {
            let u = ((y as f64 + 0.5) / px_h - SKY_TOP) / (0.97 - SKY_TOP);
            ease::smoothstep(u) as f32
        }));
        self.bead_tone = Tone::asinh(BEAD_TONE.0, BEAD_TONE.1);
        // Venus low beside the Sun, Jupiter and a couple of bright stars.
        self.lights.clear();
        for (fx, fy, level) in [
            (0.25, 0.62, 250u8),
            (0.78, 0.3, 200),
            (0.12, 0.2, 130),
            (0.9, 0.7, 120),
        ] {
            self.lights
                .push(((fx * px_w) as i32, (fy * px_h) as i32, level));
        }
        self.beads.resize(size);
        self.picture.resize(size);
        let tl = self
            .timeline
            .take()
            .unwrap_or_else(|| Timeline::new(&PHASES));
        self.d = moon_offset(&tl, self.t);
        self.f = uncovered(self.d, &self.limb);
        self.timeline = Some(tl);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        if let Some(tl) = self.timeline() {
            self.d = moon_offset(tl, self.t);
        }
        self.f = uncovered(self.d, &self.limb);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            // GEN-R19: a few corona rows per frame while it is hidden, the rest at once
            // if it is needed before then; the picture is the same either way.
            let rows = if self.corona_vis() > 0.0 {
                usize::MAX
            } else {
                CORONA_ROWS
            };
            self.fill_corona(rows);
            self.paint();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.picture.blit(canvas, |_, _, top, bottom| {
            let pick = raster::stronger(top, bottom);
            // The dim twilight sky and glow stay blank in ASCII: the Sun and corona read.
            let dim = (pick.role == Role::Ramp(SKY) && pick.level < ASCII_SKY)
                || pick.role == Role::Ramp(Ramp::Ember);
            (!dim)
                .then(|| raster::ascii_ramp_cell(top, bottom))
                .flatten()
        });
    }

    fn caption(&self) -> Option<&'static str> {
        self.timeline().map(|tl| tl.at(self.t).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tl() -> Timeline {
        Timeline::new(&PHASES)
    }

    fn built(size: Size) -> (Eclipse, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Eclipse::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn ecl_a1_uncovered_fraction_runs_one_to_zero_symmetrically() {
        let limb = Limb::seeded(&mut SplitMix64::new(3));
        let tl = tl();
        let total = tl.total();
        assert_eq!(uncovered(moon_offset(&tl, 0.0), &limb), 1.0);
        assert_eq!(uncovered(moon_offset(&tl, total / 2.0), &limb), 0.0);
        for i in 0..=200 {
            let t = total * f64::from(i) / 200.0 * 0.999;
            let (da, db) = (moon_offset(&tl, t), moon_offset(&tl, total - t));
            let smooth = |d: f64| 1.0 - overlap(1.0, MOON_R, d) / PI;
            assert!((smooth(da) - smooth(db)).abs() < 1e-9, "t = {t}");
            // The beads differ at second and third contact (different valleys), by less
            // than their whole light.
            let (a, b) = (uncovered(da, &limb), uncovered(db, &limb));
            assert!((a - b).abs() < 0.005, "t = {t}: {a} vs {b}");
        }
    }

    #[test]
    fn ecl_a2_beads_only_as_the_last_light_and_on_the_moons_limb() {
        let limb = Limb::seeded(&mut SplitMix64::new(3));
        let tl = tl();
        let mut seen = 0;
        for i in 0..4600 {
            let t = f64::from(i) * 0.01;
            let d = moon_offset(&tl, t);
            let smooth = 1.0 - overlap(1.0, MOON_R, d) / PI;
            if smooth > 1e-12 {
                continue; // beads are drawn only once the envelope covers the Sun
            }
            let (s, c) = PATH_TILT.sin_cos_det();
            let m = (d * c, d * s);
            for (a, p, _) in limb.beads(m) {
                let f = uncovered(d, &limb);
                assert!(f > 0.0 && f < 0.005, "{f}");
                let from_moon = (p.0 - m.0).hypot(p.1 - m.1);
                assert!((from_moon - MOON_R).abs() <= VALLEY + 1e-12);
                assert!((0.0..TAU).contains(&a));
                seen += 1;
            }
        }
        assert!(seen > 20, "{seen}");
    }

    #[test]
    fn ecl_a3_corona_only_once_the_sun_is_nearly_covered() {
        let (mut g, mut ctx) = built(Size::new(80, 30));
        for _ in 0..(46.0 * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if g.f >= CORONA_F {
                // The corona's light is off; the pale glare round the uncovered Sun is the
                // only pearl light off the disc (GEN-R18 grading), so it is left out below.
                assert_eq!(g.corona_vis(), 0.0, "f = {}", g.f);
                let corona = g.picture.pixel_size().area();
                let mut n = 0;
                for y in 0..60 {
                    for x in 0..80 {
                        if let Some(p) = g.picture.get(x, y)
                            && p.role == Role::Ramp(Ramp::Starlight)
                            && g.beads.get(x, y).unwrap_or(0.0) == 0.0
                            // The disc's own anti-aliased rim is not corona.
                            && (f64::from(x) + 0.5 - g.centre.0)
                                .hypot(f64::from(y) + 0.5 - g.centre.1)
                                > g.radius + 1.0
                            // Nor are the planets coming out.
                            && !g.lights.iter().any(|&(lx, ly, _)| (lx, ly) == (x, y))
                            // Nor the glare, wherever it reaches.
                            && g.glare[y as usize * 80 + x as usize] == 0
                        {
                            n += 1;
                        }
                    }
                }
                assert_eq!(n, 0, "f = {} of {corona}", g.f);
            }
        }
    }

    #[test]
    fn ecl_a4_corona_fades_with_radius() {
        let model = Corona::seeded(&mut SplitMix64::new(5));
        let median = |r: f64| {
            let mut v: Vec<f64> = (0..360)
                .map(|i| model.at(r, f64::from(i).to_radians()))
                .collect();
            v.sort_by(f64::total_cmp);
            v[180]
        };
        let mut prev = f64::INFINITY;
        for r in [1.05, 1.2, 1.5, 2.0, 3.0, 4.0] {
            let m = median(r);
            assert!(m < prev, "r = {r}");
            prev = m;
        }
    }

    #[test]
    fn ecl_a5_captions_follow_the_timeline() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        let tl = tl();
        for i in 0..(46 * 60) {
            let want = tl.at(g.t).label;
            assert_eq!(g.caption(), Some(want));
            assert!(META.captions.contains(&want), "{i}");
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
    }

    /// Advance `g` to loop time `t`.
    fn run_to(g: &mut Eclipse, ctx: &mut Ctx, t: f64) {
        while g.t < t {
            g.update(SIM_DT, ctx);
            g.prepare();
        }
    }

    #[test]
    fn ecl_a6_corona_is_tone_mapped_into_the_sky_without_an_edge() {
        // The tone curve is continuous and monotone, and never reaches its ceiling.
        let mut prev = soft_clip(0.0);
        for i in 1..2000 {
            let v = soft_clip(f64::from(i) * 0.5);
            assert!(v >= prev && v - prev <= 0.5 + 1e-9 && v < CEIL, "{i}");
            prev = v;
        }
        // In totality, walking out from the limb, the light falls smoothly into the sky:
        // no step between neighbouring pixels anywhere beyond the Moon.
        let (mut g, mut ctx) = built(Size::new(120, 40));
        run_to(&mut g, &mut ctx, 19.0);
        let (cx, cy, r) = (g.centre.0, g.centre.1, g.radius);
        for k in 0..24 {
            let a = TAU * f64::from(k) / 24.0;
            let (s, c) = a.sin_cos_det();
            let mut last: Option<u8> = None;
            let mut d = MOON_R * r + 2.0;
            while d < 5.0 * r {
                let (x, y) = ((cx + d * c) as i32, (cy + d * s) as i32);
                if let Some(p) = g.picture.get(x, y)
                    && matches!(p.role, Role::Ramp(Ramp::Starlight | SKY))
                {
                    if let Some(l) = last {
                        assert!(
                            l.abs_diff(p.level) <= 48,
                            "ray {k} at {d}: {l} → {}",
                            p.level
                        );
                    }
                    last = Some(p.level);
                }
                d += 1.0;
            }
        }
    }

    #[test]
    fn ecl_a7_sky_dims_toward_twilight_and_the_moon_stays_whole() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let corner = |g: &Eclipse| g.picture.get(4, 4).map_or(0, |p| p.level);
        let mut prev = corner(&g);
        let mut t = 0.0;
        while t < 8.0 {
            t += 0.5;
            run_to(&mut g, &mut ctx, t);
            let l = corner(&g);
            assert!(l <= prev, "t = {t}: {prev} → {l}");
            prev = l;
        }
        assert!(prev <= 16, "twilight level {prev}");
        // Through the beads and diamond ring the Moon's disc stays dark: glare never
        // spills onto it.
        while g.t < 12.0 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let m = g.to_px(g.moon());
            let rr = MOON_R * g.radius - 1.5;
            for y in (m.1 - rr) as i32..=(m.1 + rr) as i32 {
                for x in (m.0 - rr) as i32..=(m.0 + rr) as i32 {
                    let (dx, dy) = (f64::from(x) + 0.5 - m.0, f64::from(y) + 0.5 - m.1);
                    if dx.hypot(dy) < rr {
                        let p = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                        assert!(p.level <= 16, "t = {} ({x},{y}) {p:?}", g.t);
                    }
                }
            }
        }
    }

    #[test]
    fn ecl_a8_stylised_timing_matches_the_spec() {
        // Partial phases 8 s against 14 s of totality, and the Moon moving ~180× faster in
        // the partial phases than through totality (the real motion is uniform).
        let contact = MOON_R - 1.0;
        let partial = (START - contact) / PHASES[0].1;
        let total = 2.0 * (contact - VALLEY) / PHASES[2].1;
        let ratio = partial / total;
        assert!((150.0..=210.0).contains(&ratio), "{ratio}");
        assert_eq!((PHASES[0].1, PHASES[2].1), (8.0, 14.0));
    }

    #[test]
    fn ecl_a9_power_law_corona_without_cutoff_or_fray_and_no_blue_rim() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        // Partial phase: the crescent's anti-aliased rim is pale, never the sky's blue.
        run_to(&mut g, &mut ctx, 6.0);
        let (cx, cy, r) = (g.centre.0, g.centre.1, g.radius);
        let m = g.to_px(g.moon());
        let mut rim = 0;
        for y in 0..i32::from(g.picture.pixel_size().height()) {
            for x in 0..i32::from(g.picture.pixel_size().width()) {
                let sun = raster::disc_cover(x, y, (cx, cy), r);
                let moon = raster::disc_cover(x, y, m, MOON_R * r);
                let lit = sun * (1.0 - moon);
                if lit > 0.02 && lit <= 0.9 {
                    let p = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                    assert_ne!(p.role, Role::Ramp(SKY), "blue rim at ({x},{y}): {p:?}");
                    rim += 1;
                }
            }
        }
        assert!(rim > 4, "{rim}");
        // The corona's toned light falls along every ray with no cut-off: still lit at 4 R.
        for k in 0..24 {
            let a = TAU * f64::from(k) / 24.0;
            let (s, c) = a.sin_cos_det();
            let at = |d: f64| {
                let (x, y) = ((cx + d * r * c) as usize, (cy + d * r * s) as usize);
                g.corona[y.min(79) * 120 + x.min(119)]
            };
            if (cy + 4.0 * r * s) >= 0.0 && (cy + 4.0 * r * s) < 80.0 {
                assert!(at(4.0) > 0, "ray {k} cut off by 4 R");
            }
        }
        // In totality the pearl-to-blue change is clean along each ray: once, or where a
        // ray grazes a bending streamer, in and out again, never a fray.
        run_to(&mut g, &mut ctx, 19.0);
        for k in 0..48 {
            let a = TAU * f64::from(k) / 48.0;
            let (s, c) = a.sin_cos_det();
            let mut changes = 0;
            let mut last = None;
            let mut d = MOON_R * r + 2.0;
            while d < 5.0 * r {
                let (x, y) = ((cx + d * c) as i32, (cy + d * s) as i32);
                if let Some(p) = g.picture.get(x, y)
                    && matches!(p.role, Role::Ramp(Ramp::Starlight | SKY))
                    && !g.lights.iter().any(|&(lx, ly, _)| (lx, ly) == (x, y))
                {
                    if last.is_some_and(|l| l != p.role) {
                        changes += 1;
                    }
                    last = Some(p.role);
                }
                d += 0.5;
            }
            assert!(changes <= 3, "ray {k}: {changes} changes");
        }
    }

    #[test]
    fn ecl_a10_gen_r18_black_overhead_and_the_corona_fades_to_black() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let (cx, cy, r) = (g.centre.0, g.centre.1, g.radius);
        for t in [2.0, 6.0, 10.0, 19.0, 30.0, 36.0] {
            run_to(&mut g, &mut ctx, t);
            // The top quarter's corners, well clear of the Sun's glare, are black.
            for y in 0..20 {
                for x in (0..24).chain(96..120) {
                    let p = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                    let lights = g.lights.iter().any(|&(lx, ly, _)| (lx, ly) == (x, y));
                    assert!(
                        lights || p == Ink::BLANK,
                        "t = {t}: ({x},{y}) {p:?} is not black"
                    );
                }
            }
            // The land is black but for its 1-px skyline: at most one lit pixel per column
            // below the sky's lowest open row.
            for x in 0..120 {
                let lit = (70..80)
                    .filter(|&y| g.picture.get(x, y).is_some_and(|p| p == SKYLINE))
                    .count();
                assert!(lit <= 2, "t = {t}: column {x} has {lit} skyline pixels");
            }
            if t == 19.0 {
                corona_fades_in_pearl(&g, (cx, cy, r));
            }
        }
    }

    /// In totality the corona thins into black in pearl above the Sun (below it, it meets
    /// the twilight): no blue within 3 R there.
    fn corona_fades_in_pearl(g: &Eclipse, (cx, cy, r): (f64, f64, f64)) {
        for y in 0..80 {
            for x in 0..120 {
                let d = (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) / r;
                let p = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                if d < 3.0 && f64::from(y) + 0.5 < cy {
                    assert_ne!(p.role, Role::Ramp(SKY), "blue corona at ({x},{y}) {d:.2} R");
                }
            }
        }
    }

    #[test]
    fn ecl_overlap_is_exact_at_the_limits() {
        assert_eq!(overlap(1.0, 1.03, 3.0), 0.0);
        assert!((overlap(1.0, 1.03, 0.0) - PI).abs() < 1e-12);
        // Half-way: equal discs one radius apart overlap by 2π/3 − √3/2.
        let lens = overlap(1.0, 1.0, 1.0);
        assert!((lens - (2.0 * PI / 3.0 - 3f64.sqrt() / 2.0)).abs() < 1e-12);
    }
}
