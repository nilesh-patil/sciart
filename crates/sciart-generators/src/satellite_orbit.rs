//! Satellites on their real orbits around a turning Earth (SPEC-002 `satellite_orbit`).
//!
//! Every orbit is a Kepler ellipse with real elements (ISS, a Starlink train, a GPS plane,
//! three geostationary weather satellites and a Molniya), propagated in Earth-centred
//! inertial coordinates with the secular J2 drift of the node and perigee (SAT-A2..A5).
//! One sidereal day lasts `DAY` scene seconds, so the Earth turns once while a GEO
//! satellite goes round once and stays put over its longitude. Only the *drawing* bends
//! the truth: altitudes are squashed logarithmically (SAT-A7) so LEO and GEO share one
//! screen.
//!
//! The globe is a half-block [`Pixels`] image shaded per pixel from a Natural Earth land
//! map (SAT-A8), lit by a fixed Sun with a soft terminator, an ocean sun glint, city lights
//! on the night side and a thin limb glow (SAT-A9). Orbits, stars and satellites are
//! braille [`Dots`]; where an orbit passes in front of the globe it is dimmer braille over
//! it, and [`Pixels::backdrop`] puts the globe's colour behind those cells so the Earth
//! never shows holes (SAT-A11). Both layers are rasterised in `update`; `render` only
//! blits and labels.
//!
//! The globe shader lives in `shared::earth_globe`: `moon_landing` paints the Earth over Tranquility Base
//! with it.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::{ASPECT, v3, wrap_angle};
use sciart_core::ink::{self, ramp};
use sciart_core::prelude::*;
use sciart_core::{draw, ease};

use crate::shared::earth_globe::{Globe, to_view};

pub static META: Meta = Meta {
    name: "satellite_orbit",
    category: Category::Voyages,
    description: "ISS, Starlink, GPS, geostationary and Molniya orbits around a turning Earth",
    title: "Satellites · Earth turns beneath",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SatelliteOrbit::default())
}

// ---------------------------------------------------------------------------------------
// Orbital mechanics (km, s, rad). SAT-A2..A6.
// ---------------------------------------------------------------------------------------

/// Earth's equatorial radius (km, WGS-84).
const RE_KM: f64 = 6378.137;
/// Earth's gravitational parameter (km³/s²).
const MU: f64 = 398_600.441_8;
/// Earth's oblateness coefficient.
const J2: f64 = 1.082_63e-3;
/// One turn of the Earth relative to the stars (s).
const SIDEREAL_DAY: f64 = 86_164.090_5;
/// Scene seconds per sidereal day (SAT-A2): the ISS laps the globe in ~3.9 s.
const DAY: f64 = 60.0;
/// Real seconds per scene second.
const TIME_SCALE: f64 = SIDEREAL_DAY / DAY;

/// Classical orbital elements at scene time 0: semi-major axis (km), eccentricity,
/// inclination, right ascension of the ascending node, argument of perigee and mean
/// anomaly (rad).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Elements {
    pub a: f64,
    pub e: f64,
    pub i: f64,
    pub raan: f64,
    pub argp: f64,
    pub m0: f64,
}

/// Mean motion `n = √(μ/a³)` (rad per real second).
pub(crate) fn mean_motion(a: f64) -> f64 {
    (MU / (a * a * a)).sqrt()
}

/// Radius of an orbit whose period is `period` real seconds (Kepler's third law).
pub(crate) fn radius_for_period(period: f64) -> f64 {
    (MU * (period / TAU).powi(2)).cbrt()
}

/// Secular J2 drift `(dΩ/dt, dω/dt)` in rad per real second (SAT-A5):
/// `dΩ/dt = −3/2 n J2 (Re/p)² cos i`, `dω/dt = 3/4 n J2 (Re/p)² (5 cos² i − 1)`.
/// The perigee stands still at the critical inclination `cos² i = 1/5` (63.4°), which is
/// why Molniya orbits are flown there.
pub(crate) fn j2_rates(el: &Elements) -> (f64, f64) {
    let p = el.a * (1.0 - el.e * el.e);
    let k = 0.75 * mean_motion(el.a) * J2 * (RE_KM / p).powi(2);
    let c = el.i.cos_det();
    (-2.0 * k * c, k * (5.0 * c * c - 1.0))
}

/// Eccentric anomaly `E` solving Kepler's equation `E − e sin E = M` (e < 1) by Newton's
/// method from `E₀ = M` (or π for very eccentric orbits), to machine precision.
pub(crate) fn eccentric_anomaly(m: f64, e: f64) -> f64 {
    let m = wrap_angle(m);
    let mut ea = if e < 0.8 { m } else { PI.copysign(m) };
    for _ in 0..16 {
        let step = (ea - e * ea.sin_det() - m) / (1.0 - e * ea.cos_det());
        ea -= step;
        if step.abs() < 1e-15 {
            break;
        }
    }
    ea
}

/// Rotate a perifocal point `(x, y)` (x toward perigee) into ECI:
/// `R_z(Ω) · R_x(i) · R_z(ω)`.
fn perifocal_to_eci(x: f64, y: f64, raan: f64, i: f64, argp: f64) -> [f64; 3] {
    let (so, co) = raan.sin_cos_det();
    let (si, ci) = i.sin_cos_det();
    let (sw, cw) = argp.sin_cos_det();
    let (x1, y1) = (cw * x - sw * y, sw * x + cw * y);
    let (y2, z2) = (ci * y1, si * y1);
    [co * x1 - so * y2, so * x1 + co * y2, z2]
}

/// Point of the orbit `el` (with its node and perigee drifted to real time `tr`) at
/// eccentric anomaly `ea`, in ECI km: x toward the vernal equinox, z north.
///
/// An equatorial orbit has no node or perigee to drift: there J2 only moves the
/// synchronous radius by ~2 km, which GEO station-keeping absorbs, so no drift is applied.
fn point_at(el: &Elements, tr: f64, ea: f64) -> [f64; 3] {
    let (d_raan, d_argp) = if el.i > 0.0 { j2_rates(el) } else { (0.0, 0.0) };
    let (s, c) = ea.sin_cos_det();
    let b = el.a * (1.0 - el.e * el.e).sqrt();
    perifocal_to_eci(
        el.a * (c - el.e),
        b * s,
        el.raan + d_raan * tr,
        el.i,
        el.argp + d_argp * tr,
    )
}

/// ECI position (km) of a satellite with elements `el` at real time `tr` (s) (SAT-A3).
pub(crate) fn eci(el: &Elements, tr: f64) -> [f64; 3] {
    let ea = eccentric_anomaly(el.m0 + mean_motion(el.a) * tr, el.e);
    point_at(el, tr, ea)
}

/// Whether ECI point `p` (km) lies in Earth's cylindrical shadow for the unit Sun
/// direction `sun` (SAT-A6).
pub(crate) fn in_shadow(p: [f64; 3], sun: [f64; 3]) -> bool {
    let along = v3::dot(p, sun);
    along < 0.0 && v3::norm(v3::axpy(-along, sun, p)) < RE_KM
}

// ---------------------------------------------------------------------------------------
// Drawing geometry (Earth radii). SAT-A1, SAT-A7.
// ---------------------------------------------------------------------------------------

/// Viewer elevation above the equatorial plane: the north pole leans toward the viewer.
const VIEW_ELEV: f64 = 20.0 * PI / 180.0;
/// Log-squash strength of the drawn altitude (SAT-A7).
const SQUASH: f64 = 0.8;

/// Drawn radius (Re) of a true radius `r` (Re): `1 + k ln r` above the surface, the
/// identity inside it (SAT-A7). Strictly increasing, continuous at the surface, and keeps
/// LEO hugging the globe while GEO (6.6 Re) lands at ~2.5 Re.
pub(crate) fn squash(r: f64) -> f64 {
    if r > 1.0 { 1.0 + SQUASH * r.ln() } else { r }
}

/// Drawn view position (Re) of ECI point `p` (km): view axes, radius squashed.
pub(crate) fn drawn(p: [f64; 3]) -> [f64; 3] {
    let v = to_view(v3::scale(p, 1.0 / RE_KM), VIEW_ELEV);
    let r = v3::norm(v);
    if r > 0.0 {
        v3::scale(v, squash(r) / r)
    } else {
        v
    }
}

/// Whether the globe hides drawn point `v`: behind it and inside its disc (SAT-A1).
pub(crate) fn hidden(v: [f64; 3]) -> bool {
    v[2] < 0.0 && v[0].hypot(v[1]) < 1.0
}

/// Whether drawn point `v` sits in front of the globe's disc.
fn over_globe(v: [f64; 3]) -> bool {
    v[2] >= 0.0 && v[0].hypot(v[1]) < 1.0
}

// ---------------------------------------------------------------------------------------
// The fleet.
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Iss,
    Starlink,
    Gps,
    Geo,
    Molniya,
}

impl Kind {
    /// The name drawn beside one member of each class.
    fn label(self) -> &'static str {
        match self {
            Kind::Iss => "ISS",
            Kind::Starlink => "Starlink",
            Kind::Gps => "GPS",
            Kind::Geo => "GEO",
            Kind::Molniya => "Molniya",
        }
    }

    /// The ramp tinting this class's path and trail (SAT-A12): ice-blue low orbits, green
    /// navigation satellites, violet high orbits.
    fn ramp(self) -> Ramp {
        match self {
            Kind::Iss | Kind::Starlink => Ramp::Ice,
            Kind::Gps => Ramp::Aurora,
            Kind::Geo | Kind::Molniya => Ramp::Plasma,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sat {
    pub kind: Kind,
    pub el: Elements,
    /// Whether this member carries the class label.
    labelled: bool,
}

const STARLINK_N: usize = 16;
/// Train spacing in mean anomaly: a train some days after deployment (stylised: wider
/// than a fresh train so each satellite shows as its own dot).
const STARLINK_GAP: f64 = 4.0 * PI / 180.0;
const GPS_N: usize = 4;
/// Longitudes of the three GEO weather satellites: GOES-East, Meteosat, Himawari.
const GEO_LON_DEG: [f64; 3] = [-75.2, 0.0, 140.7];
pub(crate) const FLEET: usize = 1 + STARLINK_N + GPS_N + GEO_LON_DEG.len() + 1;

/// The fleet with seeded planes and phases; GEO slots are fixed by longitude, so they
/// need Earth's rotation angle at scene time 0 (`spin0`).
fn fleet(rng: &mut impl Rng, spin0: f64) -> [Sat; FLEET] {
    let half_day = radius_for_period(SIDEREAL_DAY / 2.0);
    let sat = |kind, el, labelled| Sat { kind, el, labelled };
    let mut out = [sat(Kind::Iss, Elements::default(), false); FLEET];
    let mut k = 0;
    let mut push = |s: Sat| {
        out[k] = s;
        k += 1;
    };
    push(sat(
        Kind::Iss,
        Elements {
            a: RE_KM + 420.0,
            e: 0.0005,
            i: 51.64_f64.to_radians(),
            raan: rng.random_range(0.0..TAU),
            argp: 0.0,
            m0: rng.random_range(0.0..TAU),
        },
        true,
    ));
    let (raan, m0) = (rng.random_range(0.0..TAU), rng.random_range(0.0..TAU));
    for j in 0..STARLINK_N {
        let el = Elements {
            a: RE_KM + 350.0,
            e: 0.0,
            i: 53.0_f64.to_radians(),
            raan,
            argp: 0.0,
            m0: m0 - j as f64 * STARLINK_GAP,
        };
        push(sat(Kind::Starlink, el, j == 0));
    }
    let (raan, m0) = (rng.random_range(0.0..TAU), rng.random_range(0.0..TAU));
    for j in 0..GPS_N {
        let el = Elements {
            a: half_day,
            e: 0.0,
            i: 55.0_f64.to_radians(),
            raan,
            argp: 0.0,
            m0: m0 + TAU * j as f64 / GPS_N as f64,
        };
        push(sat(Kind::Gps, el, j == 0));
    }
    let geo = radius_for_period(SIDEREAL_DAY);
    for (j, lon) in GEO_LON_DEG.into_iter().enumerate() {
        let el = Elements {
            a: geo,
            e: 0.0,
            i: 0.0,
            raan: 0.0,
            argp: 0.0,
            m0: lon.to_radians() + spin0,
        };
        push(sat(Kind::Geo, el, j == 1));
    }
    push(sat(
        Kind::Molniya,
        Elements {
            a: half_day,
            e: 0.74,
            i: 63.4_f64.to_radians(),
            raan: rng.random_range(0.0..TAU),
            argp: 270.0_f64.to_radians(),
            m0: rng.random_range(0.0..TAU),
        },
        true,
    ));
    out
}

// ---------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------

/// Sun: 12° north of the equator (northern spring), shining from the viewer's left.
const SUN_DEC: f64 = 12.0 * PI / 180.0;
const SUN_RA: f64 = -155.0 * PI / 180.0;
/// Samples per drawn orbit, uniform in eccentric anomaly.
const ORBIT_SAMPLES: usize = 240;
/// One background star per this many cells.
const CELLS_PER_STAR: usize = 55;

/// Orbit and satellite inks: paths in their class's ramp (front brighter), white sunlit
/// satellites.
const PATH_FRONT: f64 = 0.62;
const PATH_BACK: f64 = 0.36;
/// Paths crossing the globe are dimmer, so they read as in front of it without hiding
/// the ground.
const PATH_OVER: f64 = 0.58;
const SAT_LIT: Ink = Ink::new(Role::Star, 255);
/// Dot offsets of a satellite's head around its position: a 4×4 block less its corners.
const HEAD: [(i32, i32); 11] = [
    (1, 0),
    (0, 1),
    (1, 1),
    (-1, 0),
    (-1, 1),
    (2, 0),
    (2, 1),
    (0, -1),
    (1, -1),
    (0, 2),
    (1, 2),
];
/// A satellite in Earth's shadow: still there, but unlit.
const SAT_ECLIPSED: Ink = Ink::new(Role::Dim, 230);
/// Each satellite trails the last `TRAIL` scene seconds of its orbit, in `TRAIL_STEPS`
/// segments fading from `TRAIL_LEVEL.0` to `.1`: fast LEO satellites draw long streaks,
/// GEO ones stand still. The trail is the bright end of its path's ramp, so it outshines
/// the path under it (a braille cell shows its strongest dot's ink).
const TRAIL: f64 = 0.4;
const TRAIL_STEPS: usize = 8;
const TRAIL_LEVEL: (f64, f64) = (250.0, 170.0);

#[derive(Debug)]
pub struct SatelliteOrbit {
    /// Scene time (s); real time is `t · TIME_SCALE`.
    t: f64,
    spin0: f64,
    sun: [f64; 3],
    sats: [Sat; FLEET],
    ready: bool,
    size: Size,
    /// Globe centre (cells) and Earth radius (rows).
    cx: f64,
    cy: f64,
    re: f64,
    globe: Pixels,
    dots: Dots,
    /// Background stars as dot coordinates and levels, rebuilt on resize.
    stars: Vec<(i32, i32, u8)>,
    /// Cell of each labelled satellite this frame, when it is on show.
    labels: [Option<(Kind, i32, i32)>; 5],
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl Default for SatelliteOrbit {
    fn default() -> Self {
        Self {
            t: 0.0,
            spin0: 0.0,
            sun: [1.0, 0.0, 0.0],
            sats: [Sat {
                kind: Kind::Iss,
                el: Elements::default(),
                labelled: false,
            }; FLEET],
            ready: false,
            size: Size::new(1, 1),
            cx: 0.0,
            cy: 0.0,
            re: 1.0,
            globe: Pixels::default(),
            dots: Dots::default(),
            stars: Vec::new(),
            labels: [None; 5],
            dirty: false,
        }
    }
}

impl SatelliteOrbit {
    fn real_time(&self) -> f64 {
        self.t * TIME_SCALE
    }

    fn spin(&self) -> f64 {
        self.spin0 + TAU * self.t / DAY
    }

    /// Cell coordinates of drawn view point `v` (Re).
    fn cell(&self, v: [f64; 3]) -> (f64, f64) {
        (self.cx + v[0] * self.re * ASPECT, self.cy - v[1] * self.re)
    }

    /// Fit the drawn system: extents over every orbit, then the largest Earth that fits
    /// between row 1 and the row above the title (SAT-A7).
    fn fit(&mut self, size: Size) {
        let (mut up, mut down, mut side) = (1.1_f64, 1.1_f64, 1.1_f64);
        let tr = self.real_time();
        for s in &self.sats {
            for k in 0..ORBIT_SAMPLES {
                let v = drawn(point_at(&s.el, tr, TAU * k as f64 / ORBIT_SAMPLES as f64));
                up = up.max(v[1]);
                down = down.max(-v[1]);
                side = side.max(v[0].abs());
            }
        }
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let rows = (h - 3.0).max(1.0) / (up + down);
        let cols = (w / 2.0 - 1.0).max(0.5) / (ASPECT * side);
        self.re = rows.min(cols).max(0.25);
        self.cx = w / 2.0;
        self.cy = (1.0 + up * self.re).min(h / 2.0 + up * self.re);
    }

    /// Rasterise the globe, orbits, satellites and stars for the current time.
    fn rasterise(&mut self) {
        self.globe.clear();
        self.dots.clear();
        Globe {
            cx: self.cx,
            cy: 2.0 * self.cy,
            r: 2.0 * self.re,
            elev: VIEW_ELEV,
            spin: self.spin(),
            sun: to_view(self.sun, VIEW_ELEV),
            south_up: false,
        }
        .paint(&mut self.globe);
        for &(x, y, level) in &self.stars {
            // Stars only where the globe and its glow leave the sky black.
            let behind = self.globe.get(x / Dots::CELL_W, y / 2);
            if behind.is_some_and(|i| i.role == Role::Background) {
                self.dots.set(x, y, Ink::new(Role::Star, level));
            }
        }
        let tr = self.real_time();
        // One path per plane: the first member of each class stands for its plane.
        for k in 0..FLEET {
            let s = self.sats[k];
            let first = k == 0 || self.sats[k - 1].kind != s.kind;
            if first && s.kind != Kind::Starlink {
                self.draw_path(&s.el, s.kind.ramp(), tr);
            }
        }
        // Trails before heads, so no trail dims another satellite's head. The Starlink
        // train is its own trail.
        for k in 0..FLEET {
            let s = self.sats[k];
            if s.kind != Kind::Starlink {
                self.draw_trail(&s.el, s.kind.ramp(), tr);
            }
        }
        self.labels = [None; 5];
        let mut slot = 0;
        for k in 0..FLEET {
            let s = self.sats[k];
            let p = eci(&s.el, tr);
            let v = drawn(p);
            if hidden(v) {
                continue;
            }
            let ink = if in_shadow(p, self.sun) {
                SAT_ECLIPSED
            } else {
                SAT_LIT
            };
            let (x, y) = self.cell(v);
            let (dx, dy) = Dots::from_cells(x, y);
            let (dx, dy) = (dx.round() as i32, dy.round() as i32);
            self.dots.set(dx, dy, ink);
            if s.kind != Kind::Starlink {
                // A rounded 4×4 head: it outweighs a star or a path at a glance while
                // staying about one cell across.
                for (ox, oy) in HEAD {
                    self.dots.set(dx + ox, dy + oy, ink);
                }
            }
            if s.labelled && slot < self.labels.len() {
                self.labels[slot] = Some((s.kind, x.round() as i32, y.round() as i32));
                slot += 1;
            }
        }
    }

    /// Dot coordinates of drawn view point `v` (Re).
    fn dot(&self, v: [f64; 3]) -> (f64, f64) {
        let (x, y) = self.cell(v);
        Dots::from_cells(x, y)
    }

    /// One orbit as a braille polyline, hidden behind the globe's disc: bright in front
    /// of its plane, dim behind it, and dimmer still across the disc so the ground shows
    /// through (those cells take the globe's colour behind them in `render`).
    fn draw_path(&mut self, el: &Elements, tint: Ramp, tr: f64) {
        let mut prev: Option<([f64; 3], (f64, f64))> = None;
        for k in 0..=ORBIT_SAMPLES {
            let v = drawn(point_at(el, tr, TAU * k as f64 / ORBIT_SAMPLES as f64));
            let (x, y) = self.dot(v);
            if let Some((pv, (px, py))) = prev
                && !hidden(v)
                && !hidden(pv)
            {
                let level = if over_globe(v) || over_globe(pv) {
                    PATH_OVER
                } else if v[2] >= 0.0 && pv[2] >= 0.0 {
                    PATH_FRONT
                } else {
                    PATH_BACK
                };
                self.dots.line(px, py, x, y, ramp(tint, level));
            }
            prev = Some((v, (x, y)));
        }
    }

    /// The last `TRAIL` seconds of a satellite's orbit, fading back from its head; parts
    /// behind the globe are left out and parts in Earth's shadow are unlit.
    fn draw_trail(&mut self, el: &Elements, tint: Ramp, tr: f64) {
        let step = TRAIL * TIME_SCALE / TRAIL_STEPS as f64;
        let mut prev: Option<([f64; 3], (f64, f64))> = None;
        for k in 0..=TRAIL_STEPS {
            let p = eci(el, tr - step * k as f64);
            let v = drawn(p);
            let (x, y) = self.dot(v);
            if let Some((pv, (px, py))) = prev
                && !hidden(v)
                && !hidden(pv)
            {
                let f = k as f64 / TRAIL_STEPS as f64;
                let level = ease::lerp(TRAIL_LEVEL.0, TRAIL_LEVEL.1, f);
                let ink = if in_shadow(p, self.sun) {
                    Ink::new(Role::Dim, ink::level(level))
                } else {
                    ramp(tint, level / 255.0)
                };
                self.dots.line(px, py, x, y, ink);
            }
            prev = Some((v, (x, y)));
        }
    }
}

impl Generator for SatelliteOrbit {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.ready {
            // The fleet and the Earth's phase survive resizes; only the scale changes.
            self.ready = true;
            self.spin0 = ctx.rng.random_range(0.0..TAU);
            let (sd, cd) = SUN_DEC.sin_cos_det();
            self.sun = [cd * SUN_RA.cos_det(), cd * SUN_RA.sin_det(), sd];
            self.sats = fleet(&mut ctx.rng, self.spin0);
        }
        self.size = size;
        self.fit(size);
        self.globe.resize(size);
        self.dots.resize(size);
        let dot = self.dots.dot_size();
        let n = size.area() / CELLS_PER_STAR;
        self.stars.clear();
        self.stars.extend((0..n).map(|_| {
            (
                ctx.rng.random_range(0..i32::from(dot.width())),
                ctx.rng.random_range(0..i32::from(dot.height())),
                ctx.rng.random_range(40..150u8),
            )
        }));
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !self.ready {
            self.resize(ctx.size(), ctx);
        }
        if dt.is_finite() && dt > 0.0 {
            self.t += dt;
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.globe.blit(canvas, |_, _, top, bottom| {
            // ASCII: land, ice and lights read as marks, the sea as `~`.
            let ink = if top.role == Role::Background {
                bottom
            } else {
                top
            };
            match ink.role {
                Role::Background => None,
                Role::Ramp(Ramp::Ice) if ink.level < 150 => Some(('~', ink)),
                Role::Ramp(Ramp::Ice) => Some(('#', ink)),
                _ => Some(('%', ink)),
            }
        });
        self.dots.blit(canvas);
        // Orbits and satellites over the globe keep the Earth behind them.
        self.globe.backdrop(canvas);
        // Class labels right of the satellite's head, off the top and title rows: on a
        // dark chip with a blank cell either side (CORE-R67), so a city light or a sunlit
        // sea never swallows or touches a letter.
        let (w, h) = (i32::from(self.size.width()), i32::from(self.size.height()));
        if h >= 16 {
            // Placed chips as (row, first column, last column), chip cells included: a
            // label that would touch one moves a row up or down, or to the satellite's
            // left, and is dropped if none of those is clear (SAT-A13).
            // Blank cells kept between two chips on one row.
            const LABEL_GAP: i32 = 7;
            let mut placed = [(0, 0, -1); 5];
            let mut n = 0;
            for &(kind, x, y) in self.labels.iter().flatten() {
                let text = kind.label();
                let len = text.len() as i32;
                let clear = |gx: i32, gy: i32, placed: &[(i32, i32, i32)]| {
                    (1..h - 1).contains(&gy)
                        && gx >= 1
                        && gx + len < w - 1
                        && placed.iter().all(|&(py, a, b)| {
                            py != gy || gx + len + LABEL_GAP < a || gx - 1 > b + LABEL_GAP
                        })
                };
                let spots = [(x + 2, y), (x + 2, y - 1), (x + 2, y + 1), (x - 2 - len, y)];
                if let Some(&(gx, gy)) = spots.iter().find(|&&(gx, gy)| clear(gx, gy, &placed[..n]))
                {
                    draw::label(canvas, gx, gy, text, Ink::new(Role::Text, 210));
                    if n < placed.len() {
                        placed[n] = (gy, gx - 1, gx + len);
                        n += 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::earth_globe::*;

    const DAY_S: f64 = 86_400.0;

    fn scene(w: u16, h: u16, seed: u64) -> (SatelliteOrbit, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = SatelliteOrbit::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    fn of(g: &SatelliteOrbit, kind: Kind) -> Sat {
        *g.sats.iter().find(|s| s.kind == kind).unwrap()
    }

    #[test]
    fn sat_a1_hidden_iff_behind_and_inside_the_disc() {
        assert!(hidden([0.3, -0.4, -0.1]));
        assert!(!hidden([0.3, -0.4, 0.1]), "in front");
        assert!(!hidden([0.8, 0.7, -0.5]), "behind but outside the disc");
        assert!(!hidden([0.0, 1.0, -0.5]), "on the limb");
    }

    #[test]
    fn sat_a1_a_satellite_behind_the_globe_is_not_drawn() {
        let (mut g, mut ctx) = scene(160, 50, 5);
        let mut seen_hidden = false;
        for _ in 0..600 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let p = eci(&of(&g, Kind::Iss).el, g.real_time());
            let v = drawn(p);
            if hidden(v) {
                seen_hidden = true;
                let (x, y) = g.cell(v);
                let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
                g.render(&mut canvas);
                let cell = canvas.get(x as i32, y as i32).unwrap();
                assert_ne!(cell.ink, SAT_LIT, "hidden ISS drawn at {v:?}");
            }
        }
        assert!(seen_hidden, "the ISS passes behind the globe within 10 s");
    }

    #[test]
    fn sat_a2_periods_follow_kepler_on_the_scene_clock() {
        let period = |a: f64| TAU / mean_motion(a) / TIME_SCALE;
        let iss = period(RE_KM + 420.0);
        assert!((iss - 3.87).abs() < 0.04, "ISS {iss} s");
        assert!((period(radius_for_period(SIDEREAL_DAY)) - DAY).abs() < 1e-9);
        assert!((period(radius_for_period(SIDEREAL_DAY / 2.0)) - DAY / 2.0).abs() < 1e-9);
        // GEO radius is the textbook 42 164 km.
        assert!((radius_for_period(SIDEREAL_DAY) - 42_164.2).abs() < 1.0);
        // T² ∝ a³ across the fleet.
        let (g, _) = scene(200, 50, 1);
        for s in &g.sats {
            let t = TAU / mean_motion(s.el.a);
            assert!((t * t / s.el.a.powi(3) - TAU * TAU / MU).abs() < 1e-12);
        }
    }

    #[test]
    fn sat_a3_kepler_solver_and_orbit_equation() {
        for e in [0.0, 0.1, 0.5, 0.74, 0.9] {
            for k in 0..200 {
                let m = -PI + TAU * f64::from(k) / 200.0;
                let ea = eccentric_anomaly(m, e);
                let resid = wrap_angle(ea - e * ea.sin_det() - m);
                assert!(resid.abs() < 1e-12, "e={e} M={m}: residual {resid}");
            }
        }
        let el = Elements {
            a: 26_562.0,
            e: 0.74,
            i: 63.4_f64.to_radians(),
            raan: 1.0,
            argp: 270.0_f64.to_radians(),
            m0: 0.3,
        };
        let p = el.a * (1.0 - el.e * el.e);
        for k in 0..100 {
            let r = v3::norm(eci(&el, f64::from(k) * 431.0));
            // r lies between perigee and apogee: a(1 ∓ e).
            assert!(r >= el.a * (1.0 - el.e) - 1e-6 && r <= el.a * (1.0 + el.e) + 1e-6);
            assert!(r >= p / (1.0 + el.e) - 1e-6);
        }
        // Circular orbits keep |r| = a.
        let c = Elements { e: 0.0, ..el };
        assert!((v3::norm(eci(&c, 1234.0)) - c.a).abs() < 1e-9 * c.a);
    }

    #[test]
    fn sat_a4_geostationary_satellites_hang_over_their_longitude() {
        let (mut g, mut ctx) = scene(200, 50, 8);
        let sub = |g: &SatelliteOrbit, s: &Sat| {
            let p = eci(&s.el, g.real_time());
            wrap_angle(p[1].atan2(p[0]) - g.spin())
        };
        let geos: Vec<Sat> = g
            .sats
            .iter()
            .copied()
            .filter(|s| s.kind == Kind::Geo)
            .collect();
        let start: Vec<f64> = geos.iter().map(|s| sub(&g, s)).collect();
        for (s, want) in geos.iter().zip(GEO_LON_DEG) {
            assert!((sub(&g, s) - want.to_radians()).abs() < 1e-9);
        }
        for _ in 0..(DAY / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        for (s, l0) in geos.iter().zip(start) {
            assert!(wrap_angle(sub(&g, s) - l0).abs() < 1e-6, "GEO drifted");
            assert!(v3::norm(eci(&s.el, g.real_time())) > 42_000.0);
        }
        // Meanwhile the Earth turned once.
        assert!((g.spin() - g.spin0 - TAU).abs() < 1e-6);
    }

    #[test]
    fn sat_a5_j2_regresses_the_iss_node_and_freezes_molniya_perigee() {
        let per_day = |rad_s: f64| rad_s * DAY_S * 180.0 / PI;
        let iss = Elements {
            a: RE_KM + 420.0,
            e: 0.0,
            i: 51.64_f64.to_radians(),
            ..Elements::default()
        };
        let (node, _) = j2_rates(&iss);
        assert!(
            (per_day(node) + 5.0).abs() < 0.15,
            "ISS node {} °/day",
            per_day(node)
        );
        let molniya = Elements {
            a: radius_for_period(SIDEREAL_DAY / 2.0),
            e: 0.74,
            i: 63.4_f64.to_radians(),
            ..Elements::default()
        };
        let (_, perigee) = j2_rates(&molniya);
        assert!(per_day(perigee).abs() < 0.01, "{} °/day", per_day(perigee));
        // Either side of the critical inclination the perigee drifts, in opposite senses.
        let at = |i: f64| {
            per_day(
                j2_rates(&Elements {
                    i: i.to_radians(),
                    ..molniya
                })
                .1,
            )
        };
        assert!(at(55.0) > 0.05 && at(70.0) < -0.05);
    }

    #[test]
    fn sat_a6_satellites_pass_through_earths_shadow() {
        let sun = [1.0, 0.0, 0.0];
        assert!(in_shadow([-7000.0, 100.0, 0.0], sun));
        assert!(!in_shadow([7000.0, 100.0, 0.0], sun), "day side");
        assert!(
            !in_shadow([-7000.0, 6500.0, 0.0], sun),
            "outside the cylinder"
        );
        // An orbit in the Sun's plane (β = 0) is in shadow for asin(Re/a)/π of its period.
        let el = Elements {
            a: RE_KM + 420.0,
            ..Elements::default()
        };
        let n = 20_000;
        let period = TAU / mean_motion(el.a);
        let dark = (0..n)
            .filter(|&k| in_shadow(eci(&el, period * f64::from(k) / f64::from(n)), sun))
            .count();
        let want = (RE_KM / el.a).asin() / PI;
        let got = dark as f64 / f64::from(n);
        assert!((got - want).abs() < 0.002, "{got} vs {want}");
    }

    #[test]
    fn sat_a7_squash_is_monotone_and_every_orbit_fits() {
        assert_eq!(squash(1.0), 1.0);
        assert_eq!(squash(0.5), 0.5);
        let mut prev = 0.0;
        for k in 1..2000 {
            let r = f64::from(k) * 0.005;
            assert!(squash(r) > prev, "not increasing at {r}");
            prev = squash(r);
        }
        let geo = squash(radius_for_period(SIDEREAL_DAY) / RE_KM);
        assert!((2.3..2.7).contains(&geo), "GEO drawn at {geo} Re");
        for (w, h) in [(200, 50), (80, 24), (40, 12)] {
            let (g, _) = scene(w, h, 2);
            for s in &g.sats {
                for k in 0..360 {
                    let v = drawn(point_at(&s.el, 0.0, TAU * f64::from(k) / 360.0));
                    let (x, y) = g.cell(v);
                    assert!(
                        (0.0..f64::from(w)).contains(&x),
                        "{w}x{h} {:?} x {x}",
                        s.kind
                    );
                    assert!(
                        (0.0..f64::from(h) - 1.0).contains(&y),
                        "{w}x{h} {:?} y {y}",
                        s.kind
                    );
                }
            }
        }
    }

    #[test]
    fn sat_a8_the_land_map_is_earth() {
        let at = |lat: f64, lon: f64| ground(lon.to_radians(), lat.to_radians());
        assert_eq!(at(0.0, -150.0), Ground::Ocean, "Pacific");
        assert_eq!(at(30.0, -40.0), Ground::Ocean, "Atlantic");
        assert_eq!(at(-20.0, 75.0), Ground::Ocean, "Indian Ocean");
        assert_eq!(at(23.0, 10.0), Ground::Desert, "Sahara");
        assert_eq!(at(-25.0, 134.0), Ground::Desert, "Australian outback");
        assert_eq!(at(0.0, 20.0), Ground::Land, "Congo basin");
        assert_eq!(at(-10.0, -60.0), Ground::Land, "Amazon");
        assert_eq!(at(55.0, 90.0), Ground::Land, "Siberia");
        assert_eq!(at(72.0, -40.0), Ground::Ice, "Greenland");
        assert_eq!(at(-80.0, 0.0), Ground::Ice, "Antarctica");
        assert_eq!(at(35.7, 139.7), Ground::City, "Tokyo");
        // Longitude wraps.
        assert_eq!(
            ground(10.0_f64.to_radians() + TAU, 23.0_f64.to_radians()),
            Ground::Desert
        );
        assert!(MAP.iter().all(|row| row.len() == MAP_W));
    }

    #[test]
    fn sat_a9_the_sun_lights_the_day_side_and_cities_light_the_night() {
        let sun = v3::unit([-0.7, 0.2, 0.68]);
        let globe = Globe {
            cx: 0.0,
            cy: 0.0,
            r: 20.0,
            elev: VIEW_ELEV,
            spin: 0.3,
            sun,
            south_up: false,
        };
        // Sub-solar point: a bright day ink; the far side of the terminator: near-black.
        let day = globe.shade(sun[0] * 0.95, -sun[1] * 0.95, 0.5).unwrap();
        let night = globe.shade(-sun[0] * 0.95, sun[1] * 0.95, 0.5).unwrap();
        assert!(day.level > 90, "{day:?}");
        assert!(
            night.level < 40 || night.role == Role::Ramp(Ramp::Ember),
            "{night:?}"
        );
        // Night-side city cells glow in Ember; day-side ones are just land.
        let n = v3::unit([0.2, 0.1, 0.97]);
        let tokyo = (139.7_f64.to_radians(), 35.7_f64.to_radians());
        let lit = surface_ink(n, tokyo, n, 1.0, 0.5);
        let dark = surface_ink(n, tokyo, v3::scale(n, -1.0), 1.0, 0.5);
        assert_eq!(dark.role, Role::Ramp(Ramp::Ember));
        assert_eq!(lit.role, Role::Ramp(Ramp::Aurora));
        // Deserts are sand. A globe too small for detail
        // shows neither deserts nor city lights.
        let sahara = (10.0_f64.to_radians(), 23.0_f64.to_radians());
        let sand = surface_ink(n, sahara, n, 1.0, 0.5);
        assert_eq!(sand.role, Role::Blackbody);
        assert_eq!(
            surface_ink(n, sahara, n, 0.0, 0.5).role,
            Role::Ramp(Ramp::Aurora)
        );
        let unlit = surface_ink(n, tokyo, v3::scale(n, -1.0), 0.3, 0.5);
        assert_ne!(unlit.role, Role::Ramp(Ramp::Ember));
        // Lights on a mid-sized globe stay a mid Ember, not a bright block.
        let small = surface_ink(n, tokyo, v3::scale(n, -1.0), 0.6, 0.5);
        assert!(small.level <= ramp(Ramp::Ember, 0.6).level, "{small:?}");
        // Sand is a day-side colour: it never tints the terminator or the night.
        // A Sun 3° above the horizon.
        let dusk = v3::unit([0.989, 0.005, -0.1535]);
        for s in [v3::scale(n, -1.0), dusk] {
            assert_ne!(surface_ink(n, sahara, s, 1.0, 0.3).role, Role::Blackbody);
        }
        // The limb glows outside the disc on the sunward side only.
        let toward = [sun[0], -sun[1]];
        let len = toward[0].hypot(toward[1]);
        // Walking out along the sunward radius through the limb: the glow is brightest
        // at the edge and fades out, with no dark band between a bright ring and it.
        let ice = |d: f64| {
            globe
                .shade(d * toward[0] / len, d * toward[1] / len, 0.5)
                .filter(|i| i.role == Role::Ramp(Ramp::Ice))
                .map(|i| i.level)
        };
        let inner = ice(1.0 - 0.5 / globe.r).expect("ocean at the limb");
        let edge = ice(1.0 + 0.5 / globe.r).expect("glow just outside");
        assert!(
            edge >= inner,
            "edge glow {edge} under the ring inside it {inner}"
        );
        let mut last = edge;
        for k in 1..6 {
            if let Some(l) = ice(1.0 + (0.5 + f64::from(k)) / globe.r) {
                assert!(l <= last, "glow rises again {k} px out");
                last = l;
            }
        }
        let out = 1.0 + 0.5 / globe.r;
        assert!(
            globe
                .shade(out * toward[0] / len, out * toward[1] / len, 0.5)
                .is_some()
        );
        assert!(
            globe
                .shade(-out * toward[0] / len, -out * toward[1] / len, 0.5)
                .is_none()
        );
    }

    #[test]
    fn sat_a10_the_starlink_train_keeps_its_spacing() {
        let (mut g, mut ctx) = scene(200, 50, 4);
        let train: Vec<Sat> = g
            .sats
            .iter()
            .copied()
            .filter(|s| s.kind == Kind::Starlink)
            .collect();
        assert_eq!(train.len(), STARLINK_N);
        for _ in 0..240 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let tr = g.real_time();
        for pair in train.windows(2) {
            let (a, b) = (eci(&pair[0].el, tr), eci(&pair[1].el, tr));
            let ang = (v3::dot(a, b) / (v3::norm(a) * v3::norm(b)))
                .clamp(-1.0, 1.0)
                .acos();
            assert!((ang - STARLINK_GAP).abs() < 1e-6, "gap {ang}");
        }
    }

    #[test]
    fn sat_a8_desert_edges_blend_into_land() {
        let share = |lat: f64, lon: f64| desert_share(lon.to_radians(), lat.to_radians());
        assert!(share(23.0, 10.0) > 0.95, "Sahara interior");
        assert!(share(0.0, 20.0) < 0.05, "Congo basin");
        assert!(share(-10.0, -150.0) < 0.05, "Pacific");
        // Walking south out of the Sahara the share falls through a band, not a step.
        let band = (0..200)
            .map(|k| share(30.0 - 0.15 * f64::from(k), 10.0))
            .filter(|s| (0.15..0.85).contains(s))
            .count();
        assert!(band >= 10, "desert edge is {band} samples wide");
    }

    /// Every frame of `g` over the next `secs`, rendered.
    fn frames(g: &mut SatelliteOrbit, ctx: &mut Ctx, secs: f64) -> Vec<Canvas> {
        (0..(secs / SIM_DT) as usize)
            .map(|_| {
                g.update(SIM_DT, ctx);
                g.prepare();
                let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
                g.render(&mut canvas);
                canvas
            })
            .collect()
    }

    #[test]
    fn sat_a11_orbits_cross_the_globe_as_braille_on_the_globe() {
        let (mut g, mut ctx) = scene(200, 50, 3);
        let canvas = frames(&mut g, &mut ctx, 0.5).pop().unwrap();
        let globe = g.globe.pixel_size();
        for y in 0..i32::from(globe.height()) {
            for x in 0..i32::from(globe.width()) {
                let ink = g.globe.get(x, y).unwrap();
                assert_ne!(
                    ink.role,
                    Role::Ramp(Ramp::Plasma),
                    "orbit painted at {x},{y}"
                );
            }
        }
        let mut over = 0;
        for y in 0..50 {
            for x in 0..200 {
                let cell = canvas.get(x, y).unwrap();
                let on_globe = g.globe.get(x, 2 * y + 1).unwrap().role != Role::Background;
                if on_globe && ('\u{2801}'..='\u{28FF}').contains(&cell.ch) {
                    over += 1;
                    assert_ne!(cell.bg, Ink::BLANK, "black box on the globe at {x},{y}");
                }
            }
        }
        assert!(over > 10, "only {over} braille cells over the globe");
    }

    #[test]
    fn sat_a12_satellites_are_bright_heads_with_fading_trails() {
        let (mut g, mut ctx) = scene(200, 50, 5);
        let mut checked = 0;
        for _ in 0..(8.0 / SIM_DT) as usize {
            let canvas = frames(&mut g, &mut ctx, SIM_DT * 1.5).pop().unwrap();
            let tr = g.real_time();
            let iss = of(&g, Kind::Iss);
            let (p, back) = (eci(&iss.el, tr), eci(&iss.el, tr - 0.2 * TIME_SCALE));
            let (v, vb) = (drawn(p), drawn(back));
            if v[2] < 0.2 || vb[2] < 0.2 || in_shadow(p, g.sun) || in_shadow(back, g.sun) {
                continue;
            }
            let head = g.cell(v);
            let cell = canvas.get(head.0 as i32, head.1 as i32).unwrap();
            assert_eq!(cell.ink, SAT_LIT, "ISS head at {head:?}");
            let tail = g.cell(vb);
            let cell = canvas.get(tail.0 as i32, tail.1 as i32).unwrap();
            assert!(
                cell.ink.role == Role::Ramp(Kind::Iss.ramp()) && cell.ink.level < 255,
                "ISS trail at {tail:?}: {cell:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 30,
            "ISS seen sunlit in front only {checked} times"
        );
    }

    #[test]
    fn sat_a12_orbit_classes_are_tinted_by_ramp() {
        assert_eq!(Kind::Iss.ramp(), Kind::Starlink.ramp(), "one LEO tint");
        let tints = [Kind::Iss.ramp(), Kind::Gps.ramp(), Kind::Geo.ramp()];
        assert!(tints[0] != tints[1] && tints[1] != tints[2] && tints[0] != tints[2]);
        assert_eq!(
            Kind::Molniya.ramp(),
            Kind::Geo.ramp(),
            "high orbits share a tint"
        );
        let (mut g, mut ctx) = scene(200, 50, 3);
        let canvas = frames(&mut g, &mut ctx, 0.5).pop().unwrap();
        for ramp in tints {
            let n = canvas
                .cells()
                .iter()
                .filter(|c| {
                    ('\u{2801}'..='\u{28FF}').contains(&c.ch) && c.ink.role == Role::Ramp(ramp)
                })
                .count();
            assert!(n > 10, "{ramp:?}: {n} braille cells");
        }
    }

    #[test]
    fn sat_a12_starlink_pearls_are_separate_dots() {
        let (mut g, mut ctx) = scene(200, 50, 4);
        let mut pairs = 0;
        for _ in 0..240 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let tr = g.real_time();
            let heads: Vec<[f64; 3]> = g
                .sats
                .iter()
                .filter(|s| s.kind == Kind::Starlink)
                .map(|s| drawn(eci(&s.el, tr)))
                .collect();
            let dot = |v: [f64; 3]| {
                let (x, y) = g.cell(v);
                let (dx, dy) = Dots::from_cells(x, y);
                (dx.round(), dy.round())
            };
            for w in heads.windows(2) {
                // Face-on, where the limb does not foreshorten the train.
                if w.iter().any(|v| v[2] < 0.8) {
                    continue;
                }
                let (a, b) = (dot(w[0]), dot(w[1]));
                assert!(
                    (a.0 - b.0).abs().max((a.1 - b.1).abs()) >= 2.0,
                    "{a:?} {b:?}"
                );
                pairs += 1;
            }
        }
        assert!(pairs > 50, "{pairs} face-on pairs");
    }

    #[test]
    fn sat_a13_class_labels_never_touch_each_other() {
        let (mut g, mut ctx) = scene(120, 40, 42);
        for c in frames(&mut g, &mut ctx, 12.0) {
            for l in c.to_lines() {
                // Two labels on one row are at least one blank cell apart beyond their
                // chips: no "ISS GEO" run-together.
                let words: Vec<&str> = l.split_whitespace().collect();
                for pair in words.windows(2) {
                    let names = ["ISS", "GEO", "GPS", "Molniya", "Starlink"];
                    let both = names.contains(&pair[0]) && names.contains(&pair[1]);
                    if both {
                        let gap =
                            l.find(pair[1]).unwrap() - (l.find(pair[0]).unwrap() + pair[0].len());
                        assert!(gap >= 8, "labels too close: {l:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn sat_a12_the_iss_is_labelled_on_the_near_side() {
        let (mut g, mut ctx) = scene(200, 50, 5);
        let mut labelled = false;
        for c in frames(&mut g, &mut ctx, 4.0) {
            for (y, l) in c.to_lines().iter().enumerate() {
                let Some(i) = l
                    .chars()
                    .collect::<Vec<_>>()
                    .windows(3)
                    .position(|w| w == ['I', 'S', 'S'])
                else {
                    continue;
                };
                labelled = true;
                // On a chip: a blank cell either side (CORE-R67).
                for x in [i as i32 - 1, i as i32 + 3] {
                    let cell = c.get(x, y as i32).expect("on screen");
                    assert_eq!((cell.ch, cell.bg), (' ', Ink::BLANK), "chip at {x},{y}");
                }
            }
        }
        assert!(labelled);
    }

    #[test]
    fn sat_a9_a12_palette_and_layers() {
        let mut stage = Stage::new(build(), 42, Size::new(200, 50), GlyphMode::Unicode);
        for _ in 0..90 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let has = |role: Role| {
            frame
                .cells()
                .iter()
                .any(|c| c.ink.role == role || c.bg.role == role)
        };
        for role in [
            Role::Ramp(Ramp::Ice),
            Role::Ramp(Ramp::Aurora),
            Role::Ramp(Ramp::Plasma),
            Role::Star,
        ] {
            assert!(has(role), "{role:?} missing");
        }
        // Braille orbits and half-block globe.
        assert!(
            frame
                .cells()
                .iter()
                .any(|c| ('\u{2801}'..='\u{28FF}').contains(&c.ch))
        );
        assert!(frame.cells().iter().any(|c| c.ch == '▀' || c.ch == '▄'));
    }
}
