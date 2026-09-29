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
const FALLOFF: f64 = 2.5;
/// Corona tone (knee, white) and its peak `Ramp(Starlight)` level.
const CORONA_TONE: (f32, f32) = (0.08, 1.5);
/// How much the Moon's approaching shadow darkens its side of the sky in partial phases.
const SHADOW: f64 = 0.6;
/// Corona levels below this tint the sky blue instead of drawing grey.
const CORONA_BLUE: f64 = 34.0;
/// Sky: twilight `Ramp(Cobalt)` level in totality and at full sunlight, and the glare
/// around the uncovered Sun.
const SKY_DARK: f64 = 12.0;
const SKY_LIT: f64 = 40.0;
const GLARE: f64 = 70.0;
/// Horizon glow: peak `Ramp(Ember)` level and its height (share of the picture).
const GLOW_PEAK: f64 = 140.0;
const GLOW_H: f64 = 0.1;
/// Prominences: count range, heights (solar radii) and `Ramp(HAlpha)` peak.
const PROMS: (usize, usize) = (3, 6);
const PROM_H: (f64, f64) = (0.07, 0.16);
const PROM_PEAK: f64 = 200.0;
/// Beads' glare kernels: every bead, and the diamond (the brightest, alone).
const BEAD: [(i32, i32, f32); 25] = halo_kernel(2);
const DIAMOND: [(i32, i32, f32); 121] = halo_kernel(5);
const BEAD_TONE: (f32, f32) = (0.05, 1.0);
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

    pub(crate) fn at(&self, r: f64, a: f64) -> f64 {
        if r <= 1.0 {
            return 0.0;
        }
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
            let w = width * (1.3 / r).sqrt();
            let off = wrap(a - centre) / w;
            streams +=
                strength * (-0.5 * off * off).exp() * (-(r - 1.0) / length).exp() * r.powf(0.9);
        }
        // Polar plumes: fine short rays within 30° of the poles.
        let pole = wrap(a - self.equator - PI / 2.0)
            .abs()
            .min(wrap(a - self.equator + PI / 2.0).abs());
        let plumes = 0.35
            * ease::smoothstep(1.0 - pole / 0.5)
            * (0.5 + 0.5 * (48.0 * a + self.phases[0]).cos_det())
            * (-(r - 1.0) / 0.5).exp();
        let striae = 1.0
            + 0.12 * (37.0 * a + self.phases[1]).sin_det()
            + 0.08 * (83.0 * a + self.phases[2]).sin_det();
        base * (0.35 + streams + plumes) * striae
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
    prom: Vec<u8>,
    /// Per pixel: horizon glow shape (0..1) and land cover (0..1).
    glow: Vec<f32>,
    land: Vec<u8>,
    /// Per pixel: the glare's falloff round the Sun, `exp(−(r − 1)/1.5)` as a byte.
    glare: Vec<u8>,
    /// A few bright planets and stars that come out in totality (px, level).
    lights: Vec<(i32, i32, u8)>,
    beads: Glow,
    bead_tone: Tone,
    picture: Pixels,
    t: f64,
    /// Uncovered fraction and Moon offset now.
    f: f64,
    d: f64,
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
                let px = self.to_px(p);
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
        let sky = SKY_DARK + (SKY_LIT - SKY_DARK) * f.sqrt();
        let approach = if self.d < 0.0 { 1.0 } else { -1.0 };
        let glow_k = GLOW_PEAK * (1.0 - f).powf(1.5);
        let (corona, prom, glow, land) = (&self.corona, &self.prom, &self.glow, &self.land);
        let glare = &self.glare;
        let beads = self.beads.values();
        let bt = &self.bead_tone;
        self.picture.shade(|x, y| {
            let i = y as usize * w + x as usize;
            let cover = f64::from(land.get(i).copied().unwrap_or(255)) / 255.0;
            if cover >= 0.999 {
                return Ink::BLANK;
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
            // The disc: limb-darkened `Blackbody` (the darkening's colour shift softened, so
            // the limb is warm, not an orange ring); partly covered rim pixels, at the Sun's
            // or the Moon's edge, fade as pale `Starlight` instead, so the rim is
            // anti-aliased by brightness rather than by a cooler, redder colour.
            if lit > 0.9 {
                let rr = (fx - sun.0).hypot(fy - sun.1) / r;
                let mu = (1.0 - rr * rr).max(0.0).sqrt();
                let i = stellar::limb_darkening(mu, LIMB_DARK).sqrt();
                return ink::blackbody(stellar::brightness_temperature(T_SUN, i));
            }
            if lit > 0.08 {
                return Ink::new(Role::Ramp(Ramp::Starlight), ink::level(60.0 + 190.0 * lit));
            }
            // Beads and the diamond ring's flare: white light over everything but the disc.
            let b = bt.level(beads.get(i).copied().unwrap_or(0.0));
            if b >= 200 {
                return Ink::new(Role::Blackbody, ink::blackbody(T_SUN).level);
            }
            // Corona and prominences outside the Moon (its limb anti-aliased).
            let clear = (1.0 - moon_cov) * open;
            // The corona's toned level, dimmed by visibility and the Moon's limb.
            let c = ink::level(f64::from(corona.get(i).copied().unwrap_or(0)) * vis * clear);
            let p = f64::from(prom.get(i).copied().unwrap_or(0)) / 255.0 * vis * clear * PROM_PEAK;
            if p >= 30.0 && p > f64::from(c) * 0.6 {
                return Ink::new(Role::Ramp(Ramp::HAlpha), ink::level(p));
            }
            // Sky: twilight blue with glare round the uncovered Sun, a sunset glow low down.
            // The Moon's shadow sweeps in from the side it comes from: that half of the sky
            // darkens first, the other half last, and back again after totality.
            let side = (fx / w as f64 * 2.0 - 1.0) * -approach;
            let shade = 1.0 - SHADOW * (1.0 - f).powf(0.6) * (0.5 + 0.5 * side) * f.sqrt().min(1.0);
            let blue = (sky * shade
                + GLARE * f * f64::from(glare.get(i).copied().unwrap_or(0)) / 255.0)
                * (1.0 - moon_cov * sun_cov.min(1.0))
                * open;
            let ember = glow_k * f64::from(glow.get(y as usize).copied().unwrap_or(0.0)) * open;
            // The brightest light wins. Ramps share their lightness at equal levels, so where
            // the tapering corona meets the sky of the same brightness the change is a hue
            // shift only: no edge, no speckle.
            let halo = f64::from(b.max(c));
            // The corona's faint outer reaches scatter into the twilight: below
            // `CORONA_BLUE` they brighten the blue sky instead of drawing grey, so the grey
            // corona ends well inside, on a smooth near-circular hue edge.
            let blue = if halo < CORONA_BLUE {
                blue.max(halo)
            } else {
                blue
            };
            if halo >= CORONA_BLUE && halo >= blue.max(ember) {
                return Ink::new(Role::Ramp(Ramp::Starlight), ink::level(halo));
            }
            if ember >= 12.0 && ember >= blue {
                return Ink::new(Role::Ramp(Ramp::Ember), ink::level(ember));
            }
            // The Moon's dark disc (drawn dark against the sky too, a silhouette closing in).
            if moon_cov >= 0.5 && halo < 12.0 {
                return Ink::BLANK;
            }
            if open >= 0.5 {
                Ink::new(Role::Ramp(Ramp::Cobalt), ink::level(blue.max(12.0)))
            } else {
                Ink::BLANK
            }
        });
        // Planets and bright stars come out as the sky darkens.
        // Venus first (brightest), then the others, as the light fails.
        let show = ease::smoothstep((0.12 - f) / 0.12);
        if show > 0.0 {
            for &(x, y, level) in &self.lights {
                let l = f64::from(level) * show.powf(1.0 + (255.0 - f64::from(level)) / 40.0);
                if l >= 40.0
                    && self
                        .picture
                        .get(x, y)
                        .is_some_and(|p| p.role == Role::Ramp(Ramp::Cobalt) || p == Ink::BLANK)
                {
                    self.picture
                        .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), ink::level(l)));
                }
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
        let model = &self.corona_model;
        let proms = &self.proms;
        let tone = Tone::asinh(CORONA_TONE.0, CORONA_TONE.1);
        self.corona.clear();
        self.prom.clear();
        self.glow.clear();
        self.land.clear();
        self.glare.clear();
        for y in 0..2 * h {
            for x in 0..w {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let (dx, dy) = ((fx - cx) / r, (fy - cy) / r);
                let rr = dx.hypot(dy);
                let a = dy.atan2(dx);
                // The outer corona tapers to nothing by 5 R☉, so it meets the dark sky
                // where both are near black, not in a speckled fringe.
                // Toned once here; its level tapers to nothing by 4.8 R☉, so the corona
                // meets the dark sky where both are near black instead of on an edge.
                let taper = ease::smoothstep((4.8 - rr) / 2.8);
                let level = f64::from(tone.level(model.at(rr, a) as f32)) * taper;
                self.corona.push(level.round() as u8);
                // Prominences: arches of glowing gas standing on the limb.
                let mut p: f64 = 0.0;
                for pr in proms {
                    let off = wrap(a - pr.angle) / pr.half;
                    if off.abs() < 1.0 {
                        let top = 1.0 + pr.height * (1.0 - off * off).sqrt();
                        let thick = 0.35 * pr.height + 0.5 / r;
                        let d = (rr - top).abs() / thick;
                        let fill = if rr > 1.0 && rr < top { 0.55 } else { 0.0 };
                        p = p.max((1.0 - d).max(0.0).max(fill * (1.0 - off.abs())));
                    }
                }
                self.prom.push((p.clamp(0.0, 1.0) * 255.0).round() as u8);
                let up = ((horizon - fy) / (GLOW_H * px_h)).max(0.0);
                if x == 0 {
                    self.glow.push((-up * up).exp() as f32);
                }
                self.land
                    .push((hills.cover(x as i32, y as i32, horizon) * 255.0).round() as u8);
                self.glare
                    .push(((-(rr - 1.0).max(0.0) / 1.5).exp() * 255.0).round() as u8);
            }
        }
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
        self.paint();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        if let Some(tl) = self.timeline() {
            self.d = moon_offset(tl, self.t);
        }
        self.f = uncovered(self.d, &self.limb);
        self.paint();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.picture.blit(canvas, |_, _, top, bottom| {
            let pick = raster::stronger(top, bottom);
            // The dim twilight sky and glow stay blank in ASCII: the Sun and corona read.
            (pick.role != Role::Ramp(Ramp::Cobalt) && pick.role != Role::Ramp(Ramp::Ember))
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
            if g.f >= CORONA_F {
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
