//! Voyager 2's Grand Tour, 1977–1989, in 48 s (SPEC-002 `voyager_journey`).
//!
//! A top-down heliocentric map. Planets move on circular orbits with their real J2000
//! semi-major axes and mean longitudes, and with periods from Kepler's third law
//! (VOY-A2). Every position is a pure function of a *mission day* (days since
//! 2000-01-01), so the spacecraft path — a monotone cubic Hermite curve in (r, φ)
//! through the planets' model positions on the real encounter dates — meets each planet
//! exactly on its date (VOY-A1). The scene clock maps sim seconds to mission days with
//! slow-downs at every encounter (VOY-A3), and radii are drawn through a strictly
//! monotone log compression so Earth … Neptune fit on one screen (VOY-A4). A looping
//! `Timeline` runs the 48 s mission, then an 8 s "Pale Blue Dot" epilogue.

use std::f64::consts::TAU;
use std::fmt::{self, Write as _};

use rand::Rng;
use sciart_core::geom::{self, ASPECT};
use sciart_core::particles::plot;
use sciart_core::prelude::*;
use sciart_core::text::StackStr;
use sciart_core::{draw, ease};

pub static META: Meta = Meta {
    name: "voyager_journey",
    category: Category::Voyages,
    description: "Voyager 2 Grand Tour past Jupiter, Saturn, Uranus and Neptune",
    title: "Voyager 2 · planets fling it on",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(VoyagerJourney::new())
}

// ---------------------------------------------------------------------------------------
// Calendar (VOY-A3). Mission days are days since 2000-01-01 00:00 (civil, UTC-ish).
// ---------------------------------------------------------------------------------------

/// Days from 2000-01-01 to the proleptic-Gregorian date `y-m-d` (Hinnant's
/// `days_from_civil`, shifted from the 1970 epoch by 10 957 days).
pub(crate) const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 - 10_957
}

/// Inverse of [`days_from_civil`]: `(year, month, day)` of mission day `z`.
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 10_957 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Write mission day `day` (floored to its civil date) as `YYYY-MM-DD`.
fn write_date(out: &mut impl fmt::Write, day: f64) -> fmt::Result {
    // Saturating cast; the scene only ever passes days within the mission.
    let (y, m, d) = civil_from_days(day.floor() as i64);
    write!(out, "{y:04}-{m:02}-{d:02}")
}

const fn day(y: i64, m: i64, d: i64) -> f64 {
    days_from_civil(y, m, d) as f64
}

/// Voyager 2 launch, Cape Canaveral LC-41.
const LAUNCH: f64 = day(1977, 8, 20);
/// Closest approaches (NASA/JPL Voyager mission fact sheets).
const JUPITER_CA: f64 = day(1979, 7, 9);
const SATURN_CA: f64 = day(1981, 8, 26);
const URANUS_CA: f64 = day(1986, 1, 24);
const NEPTUNE_CA: f64 = day(1989, 8, 25);
/// Voyager 1's "Pale Blue Dot" family portrait — the epilogue's date.
const PALE_BLUE_DOT: f64 = day(1990, 2, 14);

// ---------------------------------------------------------------------------------------
// Planets (VOY-A2).
// ---------------------------------------------------------------------------------------

/// J2000.0 = 2000-01-01 12:00 TT, i.e. mission day 0.5.
const J2000: f64 = 0.5;
/// Sidereal year (days) — the Kepler-law anchor, `P = YEAR · (a / a⊕)^{3/2}`.
const YEAR_DAYS: f64 = 365.256_363;
const A_EARTH: f64 = 1.000_002_61;

struct Body {
    glyph: Glyph,
    /// Semi-major axis (AU), J2000 (Standish, JPL "Keplerian Elements for Approximate
    /// Positions of the Major Planets", Table 1, 1800–2050 AD).
    a: f64,
    /// Mean longitude at J2000 (degrees), same table.
    l0_deg: f64,
    /// Real sidereal period (days), NASA planetary fact sheets — only for VOY-A2.
    #[cfg_attr(not(test), allow(dead_code))]
    real_period: f64,
}

/// Earth, then the four encounter targets, in encounter order.
const BODIES: [Body; 5] = [
    Body {
        glyph: Glyph::new('⊕', 'E'),
        a: A_EARTH,
        l0_deg: 100.464_571_66,
        real_period: 365.256,
    },
    Body {
        glyph: Glyph::new('♃', 'J'),
        a: 5.202_887,
        l0_deg: 34.396_440_51,
        real_period: 4_332.59,
    },
    Body {
        glyph: Glyph::new('♄', 'S'),
        a: 9.536_675_94,
        l0_deg: 49.954_244_23,
        real_period: 10_759.22,
    },
    Body {
        glyph: Glyph::new('⛢', 'U'),
        a: 19.189_164_64,
        l0_deg: 313.238_104_51,
        real_period: 30_688.5,
    },
    Body {
        glyph: Glyph::new('♆', 'N'),
        a: 30.069_922_76,
        l0_deg: -55.120_029_69,
        real_period: 60_182.0,
    },
];
const EARTH: usize = 0;

/// Kepler's third law with Earth's sidereal year as the unit (VOY-A2).
pub(crate) fn period_days(a: f64) -> f64 {
    YEAR_DAYS * (a / A_EARTH).powf(1.5)
}

/// Unwrapped heliocentric longitude (radians) of body `b` on mission day `day`:
/// `L = L₀ + 2π (day − J2000) / P` on a circular orbit.
pub(crate) fn longitude(b: usize, day: f64) -> f64 {
    let body = &BODIES[b];
    body.l0_deg.to_radians() + TAU / period_days(body.a) * (day - J2000)
}

// ---------------------------------------------------------------------------------------
// Radial compression (VOY-A4).
// ---------------------------------------------------------------------------------------

/// Softening radius (AU) of the log map: keeps Earth ~¼ of the way out.
const R0: f64 = 0.6;
/// Heliocentric distance (AU) drawn at the screen edge; Neptune lands at ~0.97.
const R_MAX: f64 = 34.0;

/// Fraction of the screen radius for heliocentric distance `r` (AU):
/// `ρ = ln(1 + r/r₀) / ln(1 + r_max/r₀)`, strictly increasing, `ρ(0) = 0`.
pub(crate) fn rho(r: f64) -> f64 {
    (r / R0).ln_1p() / (R_MAX / R0).ln_1p()
}

/// Maps heliocentric (r, φ) to fractional cell coordinates, north ecliptic pole up
/// (prograde = counter-clockwise on screen).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Proj {
    cx: f64,
    cy: f64,
    /// Rows at `ρ = 1`; chosen so `ρ = 1` exactly touches the nearer canvas edge.
    scale: f64,
}

impl Proj {
    pub(crate) fn new(size: Size) -> Self {
        let cx = (f64::from(size.width()) - 1.0).max(0.0) / 2.0;
        let cy = (f64::from(size.height()) - 1.0).max(0.0) / 2.0;
        Self {
            cx,
            cy,
            scale: (cx / ASPECT).min(cy),
        }
    }

    pub(crate) fn screen(&self, r: f64, phi: f64) -> (f64, f64) {
        let d = rho(r) * self.scale;
        (
            self.cx + d * phi.cos_det() * ASPECT,
            self.cy - d * phi.sin_det(),
        )
    }
}

// ---------------------------------------------------------------------------------------
// Spacecraft path (VOY-A1).
// ---------------------------------------------------------------------------------------

/// Cruise speed after Neptune (AU/yr): ~15.4 km/s.
const CRUISE_AU_PER_YEAR: f64 = 3.25;
/// End of the interpolated path; later days clamp here.
const PATH_END: f64 = day(1995, 1, 1);
/// Extra prograde sweep between Neptune and [`PATH_END`] (the real trajectory then
/// dives south of the ecliptic, so its projected longitude barely changes).
const POST_NEPTUNE_SWEEP: f64 = 0.09;
const KNOTS: usize = 6;

/// Monotone piecewise-cubic Hermite curve `(r(day), φ(day))` through the encounters.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Path {
    day: [f64; KNOTS],
    r: [f64; KNOTS],
    phi: [f64; KNOTS],
    mr: [f64; KNOTS],
    mphi: [f64; KNOTS],
}

/// Fritsch–Butland tangents: a weighted harmonic mean of neighbouring secants, zero at
/// extrema, never more than 3× either secant, so each cubic piece is monotone.
/// `m0_frac` sets the start slope as a fraction of the first secant (must be in [0, 3]).
fn tangents(x: &[f64; KNOTS], y: &[f64; KNOTS], m0_frac: f64) -> [f64; KNOTS] {
    let h = |i: usize| x[i + 1] - x[i];
    let delta = |i: usize| (y[i + 1] - y[i]) / h(i);
    std::array::from_fn(|i| {
        if i == 0 {
            m0_frac * delta(0)
        } else if i == KNOTS - 1 {
            delta(KNOTS - 2)
        } else {
            let (d0, d1) = (delta(i - 1), delta(i));
            if d0 * d1 <= 0.0 {
                0.0
            } else {
                // Scipy's PCHIP weights: w₁ = 2h₁ + h₀ on δ₀, w₂ = h₁ + 2h₀ on δ₁.
                let (h0, h1) = (h(i - 1), h(i));
                3.0 * (h0 + h1) / ((2.0 * h1 + h0) / d0 + (h1 + 2.0 * h0) / d1)
            }
        }
    })
}

fn hermite(s: f64, h: f64, y0: f64, y1: f64, m0: f64, m1: f64) -> f64 {
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * y0
        + (s3 - 2.0 * s2 + s) * h * m0
        + (-2.0 * s3 + 3.0 * s2) * y1
        + (s3 - s2) * h * m1
}

impl Path {
    pub(crate) fn new() -> Self {
        let day = [
            LAUNCH, JUPITER_CA, SATURN_CA, URANUS_CA, NEPTUNE_CA, PATH_END,
        ];
        let mut r = [0.0; KNOTS];
        let mut phi = [0.0; KNOTS];
        for k in 0..5 {
            r[k] = BODIES[k].a;
            let lon = longitude(k, day[k]);
            // Unwrap so φ increases along the tour: each leg is its prograde arc.
            phi[k] = if k == 0 {
                lon.rem_euclid(TAU)
            } else {
                phi[k - 1] + (lon - phi[k - 1]).rem_euclid(TAU)
            };
        }
        r[5] = r[4] + CRUISE_AU_PER_YEAR * (PATH_END - NEPTUNE_CA) / YEAR_DAYS;
        phi[5] = phi[4] + POST_NEPTUNE_SWEEP;
        Self {
            // Leave Earth nearly tangentially (perihelion of the transfer orbit)…
            mr: tangents(&day, &r, 0.2),
            // …sweeping longitude as fast as monotonicity allows.
            mphi: tangents(&day, &phi, 3.0),
            day,
            r,
            phi,
        }
    }

    /// `(r AU, φ rad)` on mission day `day`, clamped to the path's date range.
    pub(crate) fn at(&self, day: f64) -> (f64, f64) {
        let x = if day.is_nan() {
            self.day[0]
        } else {
            day.clamp(self.day[0], self.day[KNOTS - 1])
        };
        let i = self.day[1..KNOTS - 1].partition_point(|&d| d <= x);
        let h = self.day[i + 1] - self.day[i];
        let s = (x - self.day[i]) / h;
        (
            hermite(s, h, self.r[i], self.r[i + 1], self.mr[i], self.mr[i + 1]),
            hermite(
                s,
                h,
                self.phi[i],
                self.phi[i + 1],
                self.mphi[i],
                self.mphi[i + 1],
            ),
        )
    }
}

// ---------------------------------------------------------------------------------------
// Scene clock (VOY-A3).
// ---------------------------------------------------------------------------------------

const MISSION: f64 = 48.0;
const EPILOGUE: f64 = 8.0;
/// Sim seconds at launch, each encounter, and the Pale Blue Dot.
const KEY_T: [f64; 6] = [0.0, 11.0, 20.0, 31.0, 42.0, MISSION];
const KEY_DAY: [f64; 6] = [
    LAUNCH,
    JUPITER_CA,
    SATURN_CA,
    URANUS_CA,
    NEPTUNE_CA,
    PALE_BLUE_DOT,
];
/// Share of each leg's calendar that runs at a constant rate; the rest is eased, so the
/// clock runs at 25 % of the leg average at the ends (encounters) and never stops.
const CLOCK_LINEAR: f64 = 0.25;

/// Mission day at mission sim-time `t ∈ [0, 48]` (clamped). Strictly increasing, and
/// exactly the key date at each key time.
pub(crate) fn mission_day(t: f64) -> f64 {
    let t = if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, MISSION)
    };
    let i = KEY_T[1..KEY_T.len() - 1].partition_point(|&k| k <= t);
    let x = (t - KEY_T[i]) / (KEY_T[i + 1] - KEY_T[i]);
    let f = CLOCK_LINEAR * x + (1.0 - CLOCK_LINEAR) * ease::smoothstep(x);
    ease::lerp(KEY_DAY[i], KEY_DAY[i + 1], f)
}

// ---------------------------------------------------------------------------------------
// Text (allocation-free).
// ---------------------------------------------------------------------------------------

/// First option that fits `width` with a one-cell margin each side, or none: captions
/// are shortened or skipped, never clipped mid-word.
fn pick<'a>(options: &[&'a str], width: u16) -> Option<&'a str> {
    let room = usize::from(width).saturating_sub(2);
    options
        .iter()
        .copied()
        .find(|s| !s.is_empty() && s.chars().count() <= room)
}

fn caption(canvas: &mut Canvas, y: i32, options: &[&str], level: u8) {
    if let Some(s) = pick(options, canvas.size().width()) {
        draw::text_centered(canvas, y, s, Ink::new(Role::Text, level));
    }
}

/// Encounter captions, longest first (distances: closest approach to the cloud tops,
/// NASA/JPL Voyager fact sheets). ASCII only, so they read the same in both modes.
const CAPTIONS: [[&str; 3]; 5] = [
    [
        "Launch - Cape Canaveral - 1977-08-20",
        "Launch 1977-08-20",
        "Launch",
    ],
    [
        "Jupiter flyby 1979-07-09 - 570,000 km",
        "Jupiter 1979-07-09",
        "Jupiter",
    ],
    [
        "Saturn flyby 1981-08-26 - 41,000 km",
        "Saturn 1981-08-26",
        "Saturn",
    ],
    [
        "Uranus flyby 1986-01-24 - 81,500 km",
        "Uranus 1986-01-24",
        "Uranus",
    ],
    [
        "Neptune flyby 1989-08-25 - 4,950 km",
        "Neptune 1989-08-25",
        "Neptune",
    ],
];
const EPILOGUE_LINES: [[&str; 3]; 3] = [
    [
        "Pale Blue Dot - Voyager 1 looks back, 1990-02-14",
        "Pale Blue Dot, 1990-02-14",
        "Pale Blue Dot",
    ],
    [
        "\"That's here. That's home. That's us.\" - Carl Sagan",
        "That's here. That's home. That's us.",
        "That's home.",
    ],
    [
        "Voyager 2 crossed the heliopause 2018-11-05 at 119 AU",
        "Heliopause 2018-11-05, 119 AU",
        "Heliopause 2018",
    ],
];

// ---------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------

/// Seconds (sim) a flyby flash ring expands for.
const FLASH: f64 = 2.0;
/// Caption window around each encounter, sim seconds.
const CAPTION_BEFORE: f64 = 0.8;
const CAPTION_AFTER: f64 = 4.5;
const TRAIL_SAMPLES: usize = 320;
/// One background star per this many cells.
const CELLS_PER_STAR: usize = 45;

const SUN: Glyph = Glyph::new('☉', 'O');
const ORBIT: Glyph = Glyph::new('·', '.');
const TRAIL: Glyph = Glyph::new('•', 'o');
const CRAFT: Glyph = Glyph::new('✦', '+');
const RING: Glyph = Glyph::new('∘', '*');
const STAR: Glyph = Glyph::ascii('.');

/// Orbit of `r` rows around the Sun as a dotted ellipse (~one dot per two cells of arc),
/// lighter than a solid outline so the trail stands out.
fn dotted_orbit(canvas: &mut Canvas, proj: Proj, r: f64, ink: Ink) {
    if !(r.is_finite() && r > 0.0) {
        return;
    }
    // Mean of the ellipse's semi-axes (r rows, r·ASPECT columns) approximates arc length.
    let arc = TAU * r * (1.0 + ASPECT) / 2.0;
    let n = ((arc / 2.0).ceil() as usize).clamp(8, 4096);
    for i in 0..n {
        let th = TAU * i as f64 / n as f64;
        plot(
            canvas,
            proj.cx + r * ASPECT * th.cos_det(),
            proj.cy - r * th.sin_det(),
            ORBIT,
            ink,
        );
    }
}

#[derive(Debug)]
pub struct VoyagerJourney {
    /// Sim time within the current cycle, `[0, MISSION + EPILOGUE)`.
    t: f64,
    timeline: Timeline,
    path: Path,
    proj: Proj,
    /// `(x, y, level)` of background stars, rebuilt on resize.
    stars: Vec<(i32, i32, u8)>,
}

impl VoyagerJourney {
    fn new() -> Self {
        Self {
            t: 0.0,
            timeline: Timeline::new(&[("mission", MISSION), ("epilogue", EPILOGUE)]),
            path: Path::new(),
            proj: Proj::default(),
            stars: Vec::new(),
        }
    }

    fn draw_trail(&self, canvas: &mut Canvas, today: f64, fade: f64) {
        let mut prev: Option<(i32, i32)> = None;
        for i in 0..=TRAIL_SAMPLES {
            let u = i as f64 / TRAIL_SAMPLES as f64;
            let (r, phi) = self.path.at(ease::lerp(LAUNCH, today, u));
            let (x, y) = self.proj.screen(r, phi);
            let p = (x.round() as i32, y.round() as i32);
            let ink = Ink::new(Role::Structure, ((60.0 + 150.0 * u) * fade) as u8);
            match prev {
                Some(q) if q != p => {
                    for (lx, ly) in geom::line(q.0, q.1, p.0, p.1).skip(1) {
                        canvas.put(lx, ly, TRAIL, ink);
                    }
                }
                None => canvas.put(p.0, p.1, TRAIL, ink),
                _ => {}
            }
            prev = Some(p);
        }
    }
}

impl Default for VoyagerJourney {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator for VoyagerJourney {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.proj = Proj::new(size);
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        self.stars.clear();
        if w > 0 && h > 0 {
            let n = size.area() / CELLS_PER_STAR;
            self.stars.extend((0..n).map(|_| {
                (
                    ctx.rng.random_range(0..w),
                    ctx.rng.random_range(0..h),
                    ctx.rng.random_range(35..110u8),
                )
            }));
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_finite() && dt > 0.0 {
            self.t = geom::wrap(self.t + dt, self.timeline.total());
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (w, h) = (canvas.size().width(), canvas.size().height());
        if w == 0 || h == 0 {
            return;
        }
        let beat = self.timeline.at(self.t);
        let epilogue = beat.index == 1;
        // Epilogue: everything but Earth fades to a whisper over 2 s.
        let fade = if epilogue {
            1.0 - 0.65 * ease::smoothstep(beat.local / 2.0)
        } else {
            1.0
        };
        let today = mission_day(if epilogue { MISSION } else { self.t });
        let proj = self.proj;

        for &(x, y, level) in &self.stars {
            canvas.put(x, y, STAR, Ink::new(Role::Star, level));
        }
        for b in &BODIES {
            dotted_orbit(
                canvas,
                proj,
                rho(b.a) * proj.scale,
                Ink::new(Role::Dim, (90.0 * fade) as u8),
            );
        }
        canvas.put(
            proj.cx.round() as i32,
            proj.cy.round() as i32,
            SUN,
            Ink::new(Role::Sun, 255),
        );

        self.draw_trail(canvas, today, fade);

        // Flyby flashes: expanding rings around the planet (sim-time driven).
        if !epilogue {
            for k in 1..5 {
                let age = self.t - KEY_T[k];
                if (0.0..FLASH).contains(&age) {
                    let (px, py) = proj.screen(BODIES[k].a, longitude(k, today));
                    let u = age / FLASH;
                    let level = (255.0 * (1.0 - u).powf(0.7)) as u8;
                    // Rings scale with the map: ~5 % → 25 % of the screen radius.
                    let outer = proj.scale * (0.05 + 0.2 * u);
                    for (r, dim) in [(outer.max(0.5), 1.0), ((0.5 * outer).max(0.3), 0.6)] {
                        draw::orbit(
                            canvas,
                            px,
                            py,
                            r,
                            RING,
                            Ink::new(Role::Hot, (f64::from(level) * dim) as u8),
                        );
                    }
                }
            }
        }

        // Spacecraft, then planets on top so each planet stays visible at its flyby.
        let (r, phi) = self.path.at(today);
        let (sx, sy) = proj.screen(r, phi);
        if epilogue && beat.local > 1.0 {
            // Looking back: a faint dotted sight-line from the spacecraft to Earth.
            let (ex, ey) = proj.screen(BODIES[EARTH].a, longitude(EARTH, today));
            let ink = Ink::new(
                Role::Cold,
                (70.0 * ease::smoothstep(beat.local - 1.0)) as u8,
            );
            let (x0, y0) = (sx.round() as i32, sy.round() as i32);
            for (i, (x, y)) in geom::line(x0, y0, ex.round() as i32, ey.round() as i32).enumerate()
            {
                if i % 3 == 1 {
                    canvas.put(x, y, ORBIT, ink);
                }
            }
        }
        let blink = !epilogue || (beat.local * 2.0).fract() < 0.6;
        if blink {
            plot(canvas, sx, sy, CRAFT, Ink::new(Role::Accent, 255));
        }
        for (k, b) in BODIES.iter().enumerate() {
            let (px, py) = proj.screen(b.a, longitude(k, today));
            let ink = if epilogue && k == EARTH {
                Ink::new(Role::Cold, 255)
            } else if epilogue {
                Ink::new(Role::Planet, (200.0 * fade) as u8)
            } else if k == EARTH || today >= KEY_DAY[k] {
                Ink::new(Role::Planet, 255)
            } else {
                Ink::new(Role::Planet, 190)
            };
            // Only where Earth's orbit is roomy enough that the ring stays off the Sun.
            if epilogue && k == EARTH && beat.local > 1.0 && proj.scale >= 8.0 {
                // The pale blue dot: a slow breathing ring around Earth.
                let pulse = 0.5 + 0.5 * (TAU * (beat.local - 1.0) / 2.5).sin_det();
                draw::orbit(
                    canvas,
                    px,
                    py,
                    1.0 + 0.6 * pulse,
                    RING,
                    Ink::new(Role::Cold, (90.0 + 120.0 * pulse) as u8),
                );
            }
            plot(canvas, px, py, b.glyph, ink);
        }

        // Text last, stacked from the top: in the real 1977–89 geometry Uranus, Neptune
        // and the outbound spacecraft all sit near φ ≈ 250–290° (bottom of the map), so
        // the top rows only ever cross faint orbit dots. Captions need a row of their own
        // below the readout; on very short canvases they are skipped.
        readout(canvas, today, r);
        if epilogue {
            let l = beat.local;
            // (start s, minimum canvas height): extra lines only where the map has room.
            let lines = [(0.5, 3), (2.5, 14), (5.0, 18)];
            for (row, (&(start, min_h), text)) in lines.iter().zip(&EPILOGUE_LINES).enumerate() {
                if l > start && h >= min_h {
                    caption(canvas, row as i32 + 1, text, [255, 220, 170][row]);
                }
            }
        } else {
            let active = (0..5).find(|&k| {
                let age = self.t - KEY_T[k];
                let before = if k == 0 { 0.0 } else { CAPTION_BEFORE };
                (-before..CAPTION_AFTER).contains(&age)
            });
            if let Some(k) = active
                && h >= 3
            {
                caption(canvas, 1, &CAPTIONS[k], 255);
            }
        }
    }
}

/// Top-left date / distance readout, shortened to fit.
fn readout(canvas: &mut Canvas, today: f64, r: f64) {
    let (mut long, mut mid, mut short) = (
        StackStr::<64>::default(),
        StackStr::<64>::default(),
        StackStr::<64>::default(),
    );
    let mut date = StackStr::<64>::default();
    if write_date(&mut date, today).is_err() {
        return;
    }
    let d = date.as_str();
    let ok = write!(long, "VOYAGER 2  {d}  {r:.1} AU").is_ok()
        && write!(mid, "{d}  {r:.1} AU").is_ok()
        && write!(short, "{d}").is_ok();
    if !ok {
        return;
    }
    if let Some(s) = pick(
        &[long.as_str(), mid.as_str(), short.as_str()],
        canvas.size().width(),
    ) {
        canvas.text(1, 0, s, Ink::new(Role::Text, 200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCOUNTER_DATES: [&str; 5] = [
        "1977-08-20",
        "1979-07-09",
        "1981-08-26",
        "1986-01-24",
        "1989-08-25",
    ];
    const SIZES: [(u16, u16); 6] = [(200, 50), (100, 30), (80, 24), (41, 13), (20, 8), (3, 2)];

    fn date_str(day: f64) -> String {
        let mut s = String::new();
        write_date(&mut s, day).unwrap();
        s
    }

    #[test]
    fn voy_a1_spacecraft_meets_each_planet_at_its_encounter_date() {
        let path = Path::new();
        for k in 0..5 {
            let day = KEY_DAY[k];
            let (r, phi) = path.at(day);
            let (a, lon) = (BODIES[k].a, longitude(k, day));
            assert!((r - a).abs() < 1e-9, "encounter {k}: r {r} vs a {a}");
            let dphi = (phi - lon).rem_euclid(TAU);
            assert!(
                dphi.min(TAU - dphi) < 1e-9,
                "encounter {k}: φ {phi} vs {lon}"
            );
            // The scene clock lands exactly on the encounter day at the encounter time.
            assert_eq!(mission_day(KEY_T[k]), day, "clock at encounter {k}");
            for (w, h) in SIZES {
                let p = Proj::new(Size::new(w, h));
                let (sx, sy) = p.screen(r, phi);
                let (px, py) = p.screen(a, lon);
                let d = ((sx - px).powi(2) + (sy - py).powi(2)).sqrt();
                assert!(d < 0.5, "encounter {k} at {w}x{h}: {d} cells apart");
            }
        }
    }

    #[test]
    fn voy_a1_path_is_smooth_with_monotone_r_and_prograde_phi() {
        let path = Path::new();
        let (d0, d1) = (KEY_DAY[0], KEY_DAY[5]);
        let n = 4000;
        let mut prev = path.at(d0);
        for i in 1..=n {
            let day = d0 + (d1 - d0) * f64::from(i) / f64::from(n);
            let cur = path.at(day);
            assert!(cur.0 >= prev.0, "r not monotone at day {day}");
            assert!(cur.1 >= prev.1, "φ not prograde at day {day}");
            // No jumps: a step of ~1.1 days moves < 0.05 AU and < 1°.
            assert!(cur.0 - prev.0 < 0.05 && cur.1 - prev.1 < 1f64.to_radians());
            prev = cur;
        }
        // Each leg sweeps a sensible prograde arc (< one revolution).
        for k in 0..4 {
            let sweep = path.at(KEY_DAY[k + 1]).1 - path.at(KEY_DAY[k]).1;
            assert!(sweep > 0.0 && sweep < TAU, "leg {k} sweep {sweep}");
        }
    }

    #[test]
    fn voy_a2_planet_periods_follow_kepler_and_match_real_within_1_percent() {
        for (i, b) in BODIES.iter().enumerate() {
            let p = period_days(b.a);
            let err = (p - b.real_period).abs() / b.real_period;
            assert!(
                err < 0.01,
                "body {i}: model {p} d vs real {} d",
                b.real_period
            );
            // The longitude function actually advances at 2π / P.
            let rate = longitude(i, 1000.0) - longitude(i, 0.0);
            assert!((rate / 1000.0 - TAU / p).abs() < 1e-12, "body {i} rate");
            for b2 in &BODIES {
                let ratio = period_days(b.a) / period_days(b2.a);
                assert!((ratio - (b.a / b2.a).powf(1.5)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn voy_a3_calendar_conversion_round_trips_and_is_exact_at_encounters() {
        assert_eq!(days_from_civil(2000, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 1), -10_957);
        assert_eq!(days_from_civil(1977, 8, 20), -8_169);
        assert_eq!(civil_from_days(-8_169), (1977, 8, 20));
        assert_eq!(civil_from_days(days_from_civil(1980, 2, 29)), (1980, 2, 29));
        for z in -9000..-2000 {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        for (k, want) in ENCOUNTER_DATES.iter().enumerate() {
            assert_eq!(date_str(mission_day(KEY_T[k])), *want, "encounter {k}");
        }
        assert_eq!(date_str(mission_day(KEY_T[5])), "1990-02-14");
    }

    #[test]
    fn voy_a3_mission_clock_is_strictly_monotone_and_slows_at_encounters() {
        let mut prev = mission_day(0.0);
        let n = 48 * 240;
        for i in 1..=n {
            let t = 48.0 * f64::from(i) / f64::from(n);
            let d = mission_day(t);
            assert!(d > prev, "clock not strictly increasing at t={t}");
            prev = d;
        }
        // Either side of each planet encounter the calendar runs at well under the
        // average rate of that leg.
        let h = 0.05;
        for k in 1..5 {
            let before = (mission_day(KEY_T[k]) - mission_day(KEY_T[k] - h)) / h;
            let after = (mission_day(KEY_T[k] + h) - mission_day(KEY_T[k])) / h;
            let avg_before = (KEY_DAY[k] - KEY_DAY[k - 1]) / (KEY_T[k] - KEY_T[k - 1]);
            let avg_after = (KEY_DAY[k + 1] - KEY_DAY[k]) / (KEY_T[k + 1] - KEY_T[k]);
            assert!(before < 0.5 * avg_before, "encounter {k}: {before} d/s");
            assert!(after < 0.5 * avg_after, "encounter {k}: {after} d/s");
        }
    }

    #[test]
    fn voy_a4_radial_map_is_strictly_monotone_and_fits_neptune() {
        assert_eq!(rho(0.0), 0.0);
        assert!((rho(R_MAX) - 1.0).abs() < 1e-12);
        let mut prev = rho(0.0);
        for i in 1..=10_000 {
            let r = 0.01 * f64::from(i);
            let v = rho(r);
            assert!(v > prev, "rho not strictly increasing at {r} AU");
            prev = v;
        }
        let rn = rho(BODIES[4].a);
        assert!(rn < 1.0 && rn > 0.9, "Neptune at {rn} of the radius");
        // Earth is not squashed into the Sun: well clear of the centre.
        assert!(rho(1.0) > 0.2);
        for (w, h) in SIZES {
            let p = Proj::new(Size::new(w, h));
            for i in 0..360 {
                let (x, y) = p.screen(BODIES[4].a, f64::from(i).to_radians());
                let (x, y) = (x.round(), y.round());
                assert!((0.0..f64::from(w)).contains(&x), "{w}x{h}: x {x}");
                assert!((0.0..f64::from(h)).contains(&y), "{w}x{h}: y {y}");
            }
        }
    }

    #[test]
    fn voy_a4_neptune_is_drawn_throughout_the_cycle() {
        for (w, h) in [(200, 50), (60, 16)] {
            let mut stage = Stage::new(build(), 3, Size::new(w, h), GlyphMode::Ascii);
            // The scene's own frame: the title may cover Neptune low on the bottom row.
            stage.set_title_visible(false);
            for s in 0..56 {
                for _ in 0..60 {
                    stage.step(SIM_DT);
                }
                let frame = stage.render();
                let n = frame
                    .cells()
                    .iter()
                    .filter(|c| c.ch == 'N' && c.ink.role == Role::Planet)
                    .count();
                assert_eq!(n, 1, "{w}x{h} at t={s}s");
            }
        }
    }

    #[test]
    fn voy_captions_never_clip_and_scene_animates() {
        for (w, h) in [(200, 50), (100, 30), (30, 10), (12, 4)] {
            let mut stage = Stage::new(build(), 5, Size::new(w, h), GlyphMode::Unicode);
            let first = stage.render().to_lines();
            for _ in 0..60 {
                stage.step(SIM_DT);
            }
            assert_ne!(
                first,
                stage.render().to_lines(),
                "{w}x{h}: static after 1 s"
            );
            for _ in 0..(56 * 12) {
                for _ in 0..5 {
                    stage.step(SIM_DT);
                }
                let frame = stage.render();
                // Captions keep a one-cell margin, so text never reaches the right edge.
                for y in 0..i32::from(h) {
                    let edge = frame.get(i32::from(w) - 1, y).unwrap();
                    assert!(
                        edge.ink.role != Role::Text,
                        "{w}x{h}: text at the right edge on row {y}"
                    );
                }
            }
        }
    }
}
