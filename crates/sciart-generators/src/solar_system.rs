//! The eight planets on Kepler orbits around the Sun, seen from 30° above the ecliptic
//! (SPEC-002 `solar_system`).
//!
//! An orrery, not a map: orbit spacing, planet sizes and periods are compressed (stated in
//! the spec). The orbits are ellipses of axis ratio sin 30°, so the system fills a wide
//! terminal, and each planet is a lit sphere: its phase follows where it sits, gibbous
//! beyond the Sun and crescent in front of it. Orbits, the belt, trails and Saturn's rings
//! are braille `Dots` (orbits and belt in a layer of their own, thinned in ASCII); the Sun
//! and the planet discs are `Pixels` drawn over them, far side first. All are rasterised
//! in `update`/`resize`, so `render` only blits.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "solar_system",
    category: Category::SunPlanets,
    description: "The eight planets orbiting the Sun at Kepler speeds, with an asteroid belt",
    title: "Planets · farther ones go slower",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SolarSystem::default())
}

/// Earth's year in simulated seconds.
const EARTH_PERIOD: f64 = 12.0;
const EARTH_FRAC: f64 = 0.32;
const BELT: usize = 360;
const BELT_FRAC: (f64, f64) = (0.47, 0.55);

/// The view: 30° above the ecliptic, so a circular orbit is an ellipse of axis ratio
/// `sin 30° = SQUASH`.
const SQUASH: f64 = 0.5;
/// `cos 30°`: how much an orbit's far/near offset moves toward or away from the viewer.
const DEPTH: f64 = 0.866_025_403_784_438_6;

/// IAU nominal solar effective temperature (K), the Sun's colour.
const SUN_K: f64 = 5772.0;
/// Linear limb darkening of the Sun's disc (visible continuum), as in `solar_surface`.
const SUN_LIMB_U: f64 = 0.6;
/// The disc is coloured by the brightness temperature of `I^gain`: the real limb is
/// ≈ 1000 K cooler than the centre, too little to see at this size, so the reddening is
/// drawn 3× as strong.
const LIMB_COLOUR_GAIN: f64 = 3.0;
/// Wien's approximation at 550 nm: `λ / c₂` in 1/K.
const WIEN_550: f64 = 550.0 / 1.438_776_9e7;
/// Sun radius as a share of the outer orbit radius, at least `SUN_MIN_PX` pixels.
const SUN_FRAC: f64 = 0.06;
const SUN_MIN_PX: f64 = 0.8;

/// Planet radius (px) is `s·(R/R_J)^SIZE_POWER`, `s = JUPITER_PX` at an outer orbit of
/// `REF_ORBIT_PX` (200×50), scaled with it and floored at `JUPITER_MIN_PX`.
const SIZE_POWER: f64 = 0.4;
const JUPITER_PX: f64 = 3.5;
const REF_ORBIT_PX: f64 = 87.0;
const JUPITER_MIN_PX: f64 = 0.6;
/// A disc is never smaller than one pixel.
const MIN_PLANET_PX: f64 = 0.5;
/// Discs smaller than this (px) centre on a pixel corner and are drawn at least 1 px in
/// radius, so they come out as a round 2 × 2 blot (then a 12-pixel one), never a plus.
const CORNER_BELOW: f64 = 1.5;
/// A pixel is drawn where the disc covers at least `MIN_COVER` of it, at full level from
/// `FULL_COVER`, and dimmed below that toward `EDGE_FLOOR`: an anti-aliased rim.
const MIN_COVER: f64 = 0.15;
const FULL_COVER: f64 = 0.75;
const EDGE_FLOOR: f64 = 0.35;
/// Night side at this share of the day-side level, reached over a soft terminator; for
/// discs under `CORNER_BELOW` at `NIGHT_SMALL`, so a 2 × 2 blot in crescent never sinks
/// into the sky (a dark planet like Mars would all but vanish).
const NIGHT: f64 = 0.3;
const NIGHT_SMALL: f64 = 0.5;
const TERMINATOR_WIDTH: f64 = 0.5;

/// Trail: this much orbit angle behind the planet, at most `TRAIL_MAX_DOTS` dots long.
const TRAIL_RAD: f64 = 0.45;
const TRAIL_MAX_DOTS: f64 = 30.0;
const TRAIL_LEVEL: (f64, f64) = (0.35, 0.8);
const ORBIT_INK: Ink = Ink::new(Role::Dim, 40);

/// Saturn's obliquity to its orbit (deg). The ring pole leans this far from the orbit
/// normal toward +x and is fixed in space, so its ellipse is the same wherever Saturn is.
const SATURN_TILT_DEG: f64 = 26.7;
/// The B and A rings, in Saturn radii (B 1.53–1.95, Cassini Division, A 2.03–2.27), each
/// drawn as one thin ellipse at its middle.
const RINGS: [(f64, Ink); 2] = [
    (1.74, Ink::new(Role::Ramp(Ramp::Ember), 200)),
    (2.15, Ink::new(Role::Ramp(Ramp::Ember), 150)),
];

struct Planet {
    /// ASCII letter on the disc's centre cell.
    letter: char,
    /// Orbit radius as a fraction of the outermost orbit.
    frac: f64,
    /// Equatorial radius in Jupiter radii (71 492 km).
    radius: f64,
    /// Day-side ink; night side and trail scale its level.
    day: Ink,
}

const fn ramp(r: Ramp, level: u8) -> Ink {
    Ink::new(Role::Ramp(r), level)
}

/// One hue per planet, close to its own where a ramp allows: grey Mercury, pale cream
/// Venus, blue Earth, rust-orange Mars, tan Jupiter, gold Saturn, pale cyan Uranus, deep blue
/// Neptune. Levels stay ≤ 225, so in `mono` the Sun is the brightest thing on screen.
const PLANETS: [Planet; 8] = [
    Planet {
        letter: 'm',
        frac: 0.16,
        radius: 0.0341,
        day: Ink::new(Role::Structure, 170),
    },
    Planet {
        letter: 'v',
        frac: 0.24,
        radius: 0.0847,
        day: ramp(Ramp::Ember, 225),
    },
    Planet {
        letter: 'e',
        frac: EARTH_FRAC,
        radius: 0.0892,
        day: ramp(Ramp::Ice, 165),
    },
    Planet {
        letter: 'M',
        frac: 0.42,
        radius: 0.0475,
        day: ramp(Ramp::Ember, 120),
    },
    Planet {
        letter: 'J',
        frac: 0.60,
        radius: 1.0,
        day: ramp(Ramp::Ember, 190),
    },
    Planet {
        letter: 'S',
        frac: 0.74,
        radius: 0.843,
        day: ramp(Ramp::Ember, 222),
    },
    Planet {
        letter: 'U',
        frac: 0.87,
        radius: 0.358,
        day: ramp(Ramp::Ice, 220),
    },
    Planet {
        letter: 'N',
        frac: 1.00,
        radius: 0.346,
        day: ramp(Ramp::Ice, 105),
    },
];
const JUPITER: usize = 4;
const SATURN: usize = 5;
/// Jupiter's dark north and south equatorial belts span these latitudes (sin of 7°–20°).
const JUPITER_BELT: (f64, f64) = (0.122, 0.342);
const JUPITER_BELT_LEVEL: u8 = 140;

/// Kepler's third law: angular speed for an orbit at `frac` of the outer radius (SOL-A1).
/// `ω(a) = ω⊕ · (a/a⊕)^-3/2`, i.e. `T² ∝ a³` with Earth's year pinned to [`EARTH_PERIOD`].
pub(crate) fn angular_speed(frac: f64) -> f64 {
    TAU / EARTH_PERIOD * (frac / EARTH_FRAC).powf(-1.5)
}

/// Displayed radius (px) of a planet of `radius` Jupiter radii when the outer orbit is
/// `orbit_px` pixels (SOL-A4): compressed, never below one pixel.
pub(crate) fn planet_px(radius: f64, orbit_px: f64) -> f64 {
    let s = (JUPITER_PX * orbit_px / REF_ORBIT_PX).max(JUPITER_MIN_PX);
    (s * radius.powf(SIZE_POWER)).max(MIN_PLANET_PX)
}

/// Screen offset (px, y down) and depth toward the viewer of a point `(x, y, z)` in the
/// ecliptic frame (x right, y away from the viewer, z north).
fn project([x, y, z]: [f64; 3]) -> (f64, f64, f64) {
    (x, -(y * SQUASH + z * DEPTH), -y * DEPTH + z * SQUASH)
}

/// Screen offset and depth of the point at angle `t` on a ring of radius `a` (SOL-A6).
/// The ring plane's pole is fixed in space, so this does not depend on where Saturn is.
pub(crate) fn ring_offset(a: f64, t: f64) -> (f64, f64, f64) {
    let (st, ct) = SATURN_TILT_DEG.to_radians().sin_cos();
    let (s, c) = t.sin_cos();
    // Ring-plane axes: e1 = ŷ and e2 = pole × e1 = (−cos T, 0, sin T).
    project([-a * s * ct, a * c, a * s * st])
}

/// `ink` with its level scaled by `k`.
/// The 550 nm brightness temperature (K) of a solar surface element with intensity
/// `i ∈ (0, 1]` of the disc centre's, by Wien's law `1/T = 1/T☉ − (λ/c₂) ln i`: `SUN_K`
/// at the centre, cooler toward the limb. `i` is floored at 10⁻⁶ (≈ 2300 K).
fn brightness_temperature(i: f64) -> f64 {
    1.0 / (1.0 / SUN_K - WIEN_550 * i.clamp(1e-6, 1.0).ln())
}

fn scaled(ink: Ink, k: f64) -> Ink {
    Ink::new(ink.role, ink::level(f64::from(ink.level) * k))
}

/// The pixels of a disc of radius `r` near `(x, y)`, with each one's offset `(nx, ny)` in
/// units of `r` (y up) and its edge shade in `[EDGE_FLOOR, 1]` (SOL-A10). The centre snaps
/// to the nearest pixel centre, or, below `CORNER_BELOW`, to the nearest pixel corner with
/// `r ≥ 1`, so every disc is symmetric and a small one is a round blot, not a plus. Cover
/// is estimated from each pixel centre's distance, `r + 1/2 − d`.
fn disc_pixels(x: f64, y: f64, r: f64, mut f: impl FnMut(i32, i32, f64, f64, f64)) {
    let (ox, oy, r) = snap(x, y, r);
    let k = (r + 1.0).ceil() as i32;
    let (hx, hy) = (ox.floor() as i32, oy.floor() as i32);
    for py in hy - k..=hy + k {
        for px in hx - k..=hx + k {
            let (dx, dy) = (f64::from(px) + 0.5 - ox, f64::from(py) + 0.5 - oy);
            let cover = (r + 0.5 - dx.hypot(dy)).min(1.0);
            if cover >= MIN_COVER {
                let shade = EDGE_FLOOR + (1.0 - EDGE_FLOOR) * (cover / FULL_COVER).min(1.0);
                f(px, py, dx / r, -dy / r, shade);
            }
        }
    }
}

/// The centre and radius `disc_pixels` draws a disc of radius `r` near `(x, y)` with.
fn snap(x: f64, y: f64, r: f64) -> (f64, f64, f64) {
    if r < CORNER_BELOW {
        (x.round(), y.round(), r.max(1.0))
    } else {
        (x.floor() + 0.5, y.floor() + 0.5, r)
    }
}

#[derive(Debug, Default)]
pub struct SolarSystem {
    /// Sun centre and outer orbit radius, in pixels (square: x in cells, y in half rows).
    cx: f64,
    cy: f64,
    outer: f64,
    phases: [f64; 8],
    /// (orbit fraction, angle, level)
    belt: Vec<(f64, f64, u8)>,
    /// Pixel of each planet's centre, for the ASCII letters.
    centres: [(i32, i32); 8],
    /// Orbits and the belt: braille in Unicode, a sparse `.` in ASCII, where braille's
    /// fallback would turn them into rows of `:` as heavy as the planets.
    orbits: Dots,
    /// Trails and Saturn's rings.
    dots: Dots,
    pixels: Pixels,
}

impl SolarSystem {
    /// Screen position (px) of the point at angle `theta` on the orbit at `frac`.
    fn orbit_xy(&self, frac: f64, theta: f64) -> (f64, f64) {
        let (s, c) = theta.sin_cos();
        let r = self.outer * frac;
        let (dx, dy, _) = project([r * c, r * s, 0.0]);
        (self.cx + dx, self.cy + dy)
    }

    fn planet_xy(&self, i: usize) -> (f64, f64) {
        self.orbit_xy(PLANETS[i].frac, self.phases[i])
    }

    /// Dot coordinates of pixel-space point `(x, y)` (x in cells, y in half rows).
    fn dot(x: f64, y: f64) -> (f64, f64) {
        Dots::from_cells(x, y / 2.0)
    }

    fn rasterise(&mut self) {
        self.orbits.clear();
        self.dots.clear();
        self.pixels.clear();
        let outer_dots = 2.0 * self.outer;

        // Orbits: every other dot, so they read as fine dotted ellipses.
        for p in &PLANETS {
            let n = (TAU * self.outer * p.frac).ceil().max(8.0) as usize;
            for k in 0..n {
                let (x, y) = self.orbit_xy(p.frac, TAU * k as f64 / n as f64);
                let (x, y) = Self::dot(x, y);
                self.orbits.plot(x, y, ORBIT_INK);
            }
        }
        for &(frac, angle, level) in &self.belt {
            let (x, y) = self.orbit_xy(frac, angle);
            let (x, y) = Self::dot(x, y);
            self.orbits.plot(x, y, Ink::new(Role::Dim, level));
        }
        // Trails behind each planet, fading toward their tail (SOL-A7).
        for (i, p) in PLANETS.iter().enumerate() {
            let r_dots = outer_dots * p.frac;
            let span = TRAIL_RAD.min(TRAIL_MAX_DOTS / r_dots.max(1.0));
            let n = (span * r_dots).ceil().max(1.0) as usize;
            for k in 1..=n {
                let f = k as f64 / n as f64;
                let (x, y) = self.orbit_xy(p.frac, self.phases[i] - span * f);
                let (x, y) = Self::dot(x, y);
                let k = TRAIL_LEVEL.1 + (TRAIL_LEVEL.0 - TRAIL_LEVEL.1) * f;
                self.dots.plot(x, y, scaled(p.day, k));
            }
        }
        self.draw_rings();

        // The far side of the system, the Sun, and the near side. The Sun has no halo:
        // its reddened limb is its glow, and any darker ring round it reads as an outline.
        let r_sun = (SUN_FRAC * self.outer).max(SUN_MIN_PX);
        for i in 0..PLANETS.len() {
            if self.phases[i].sin() > 0.0 {
                self.draw_planet(i);
            }
        }
        disc_pixels(self.cx, self.cy, r_sun, |x, y, nx, ny, _| {
            let mu = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let i = 1.0 - SUN_LIMB_U * (1.0 - mu);
            let t = brightness_temperature(i.powf(LIMB_COLOUR_GAIN));
            self.pixels.set(x, y, ink::blackbody(t));
        });
        for i in 0..PLANETS.len() {
            if self.phases[i].sin() <= 0.0 {
                self.draw_planet(i);
            }
        }
    }

    /// Saturn's rings as thin braille ellipses (SOL-A6); the parts in front of the ball
    /// are drawn over it by `draw_planet`.
    fn draw_rings(&mut self) {
        let (x, y) = self.planet_xy(SATURN);
        let r_s = planet_px(PLANETS[SATURN].radius, self.outer);
        for &(k, ink) in &RINGS {
            let a = k * r_s;
            let n = (TAU * 2.0 * a).ceil().max(12.0) as usize;
            for j in 0..n {
                let (u, v, _) = ring_offset(a, TAU * j as f64 / n as f64);
                let (dx, dy) = Self::dot(x + u, y + v);
                self.dots.plot(dx, dy, ink);
            }
        }
    }

    /// A planet's disc, lit from the Sun with a soft terminator (SOL-A5).
    fn draw_planet(&mut self, i: usize) {
        let p = &PLANETS[i];
        let (x, y) = self.planet_xy(i);
        let r = planet_px(p.radius, self.outer);
        // The Sun's direction from the planet, in view axes (right, up, toward viewer).
        let (s, c) = self.phases[i].sin_cos();
        let (_, up, toward) = project([-c, -s, 0.0]);
        let sun = [-c, -up, toward];
        let night = if r < CORNER_BELOW { NIGHT_SMALL } else { NIGHT };
        disc_pixels(x, y, r, |px, py, nx, ny, shade| {
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let lambert = nx * sun[0] + ny * sun[1] + nz * sun[2];
            let day = ease::smoothstep(lambert / TERMINATOR_WIDTH + 0.5);
            // Latitude on the planet (pole along the ecliptic north): the belts are bands.
            let sin_lat = (ny * DEPTH + nz * SQUASH).abs();
            let base = if i == JUPITER && (JUPITER_BELT.0..JUPITER_BELT.1).contains(&sin_lat) {
                Ink::new(p.day.role, JUPITER_BELT_LEVEL)
            } else {
                p.day
            };
            let light = night + (1.0 - night) * day;
            self.pixels.set(px, py, scaled(base, light * shade));
        });
        if i == SATURN {
            // The near halves of the rings pass in front of the ball, as drawn.
            let (ox, oy, rb) = snap(x, y, r);
            for &(k, ink) in &RINGS {
                let a = k * r;
                let n = (TAU * 2.0 * a).ceil().max(12.0) as usize;
                for j in 0..n {
                    let (u, v, depth) = ring_offset(a, TAU * j as f64 / n as f64);
                    let (qx, qy) = ((x + u).floor(), (y + v).floor());
                    let (dx, dy) = (qx + 0.5 - ox, qy + 0.5 - oy);
                    if depth > 0.0 && dx.hypot(dy) <= rb + 0.5 - MIN_COVER {
                        self.pixels.set(qx as i32, qy as i32, ink);
                    }
                }
            }
        }
        self.centres[i] = (x.floor() as i32, y.floor() as i32);
    }

    /// ASCII fallback for one cell's two pixels.
    fn ascii_cell(&self, x: i32, y: i32, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let ink = [top, bottom]
            .into_iter()
            .filter(|i| i.role != Role::Background)
            .max_by_key(|i| i.level)?;
        if let Some(i) = self
            .centres
            .iter()
            .position(|&(px, py)| px == x && py / 2 == y)
        {
            return Some((PLANETS[i].letter, ink));
        }
        let ch = if ink.role == Role::Blackbody {
            '@'
        } else {
            'o'
        };
        Some((ch, ink))
    }
}

impl Generator for SolarSystem {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (size.width(), size.height());
        // The last row belongs to the title and the one above it stays empty, so no
        // orbit dot trails the title like an ellipsis (GEN-R13, SOL-A8).
        let rows = if h >= 6 { h - 2 } else { h };
        // The centre of the middle cell, so the Sun's core sits on it (SOL-A3).
        self.cx = f64::from(w / 2) + 0.5;
        self.cy = 2.0 * f64::from(rows / 2) + 1.0;
        // Half-extents in pixels (rows are two pixels tall), less a margin for the
        // outermost discs.
        let half_w = 0.96 * f64::from(w) / 2.0 - 1.0;
        let half_h = 0.96 * self.cy.min(2.0 * f64::from(rows) - self.cy) - 1.5;
        self.outer = half_w.min(half_h / SQUASH).max(1.0);
        if self.belt.is_empty() {
            // Positions survive resizes; only the scale changes.
            for p in &mut self.phases {
                *p = ctx.rng.random_range(0.0..TAU);
            }
            self.belt = (0..BELT)
                .map(|_| {
                    (
                        ctx.rng.random_range(BELT_FRAC.0..BELT_FRAC.1),
                        ctx.rng.random_range(0.0..TAU),
                        ctx.rng.random_range(60..100),
                    )
                })
                .collect();
        }
        self.orbits.resize(size);
        self.dots.resize(size);
        self.pixels.resize(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        for (phase, planet) in self.phases.iter_mut().zip(&PLANETS) {
            *phase = (*phase + angular_speed(planet.frac) * dt) % TAU;
        }
        for (frac, angle, _) in &mut self.belt {
            *angle = (*angle + angular_speed(*frac) * dt) % TAU;
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.mode() == GlyphMode::Unicode {
            self.orbits.blit(canvas);
        } else {
            // Every other lit cell, as a dot: a light dotted ellipse.
            let size = canvas.size();
            for y in 0..i32::from(size.height()) {
                for x in (y & 1..i32::from(size.width())).step_by(2) {
                    if self.orbits.cell_mask(x, y).is_some_and(|m| m != 0) {
                        canvas.put(x, y, Glyph::ascii('.'), ORBIT_INK);
                    }
                }
            }
        }
        self.dots.blit(canvas);
        self.pixels.blit(canvas, |x, y, top, bottom| {
            self.ascii_cell(x, y, top, bottom)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(w: u16, h: u16) -> SolarSystem {
        let size = Size::new(w, h);
        let mut s = SolarSystem::default();
        s.resize(size, &mut Ctx::new(1, size));
        s
    }

    #[test]
    fn sol_a1_kepler_speeds_decrease_outward_and_earth_year_is_12s() {
        let speeds: Vec<f64> = PLANETS.iter().map(|p| angular_speed(p.frac)).collect();
        assert!(speeds.windows(2).all(|w| w[0] > w[1]), "{speeds:?}");
        assert!((TAU / angular_speed(EARTH_FRAC) - EARTH_PERIOD).abs() < 1e-9);
        // T² ∝ a³
        let t = |f: f64| TAU / angular_speed(f);
        let ratio = t(1.0).powi(2) / t(EARTH_FRAC).powi(2);
        assert!((ratio - (1.0 / EARTH_FRAC).powi(3)).abs() < 1e-6);
    }

    #[test]
    fn sol_a2_every_planet_centre_is_on_canvas_in_its_ink() {
        let mut s = scene(200, 50);
        let mut ctx = Ctx::new(1, Size::new(200, 50));
        for _ in 0..40 {
            for _ in 0..60 {
                s.update(SIM_DT, &mut ctx);
            }
            for (i, p) in PLANETS.iter().enumerate() {
                let (x, y) = s.planet_xy(i);
                let ink = s.pixels.get(x.floor() as i32, y.floor() as i32);
                // Between its night and day levels (a planet in front of the Sun shows a
                // crescent, so its centre may be on the night side).
                let ok = ink.is_some_and(|k| {
                    k.role == p.day.role
                        && k.level >= scaled(p.day, NIGHT).level
                        && k.level <= p.day.level
                });
                assert!(ok, "planet {i} at ({x}, {y}): {ink:?}");
            }
        }
    }

    #[test]
    fn sol_a3_sun_core_on_center_cell() {
        let (w, h) = (200, 50);
        let (cx, cy) = (i32::from(w / 2), i32::from((h - 2) / 2));
        let mut stage = Stage::new(build(), 1, Size::new(w, h), GlyphMode::Unicode);
        let cell = *stage.render().get(cx, cy).unwrap();
        assert_eq!(cell.ink.role, Role::Blackbody);
        let k = ink::blackbody_kelvin(cell.ink);
        assert!((k / SUN_K - 1.0).abs() < 0.02, "{k} K");
        let mut ascii = Stage::new(build(), 1, Size::new(w, h), GlyphMode::Ascii);
        assert_eq!(ascii.render().get(cx, cy).unwrap().ch, '@');
    }

    #[test]
    fn sol_a3_the_limb_is_cooler_by_wien_and_stays_finite() {
        assert!((brightness_temperature(1.0) - SUN_K).abs() < 1e-9);
        // 1/T − 1/T☉ = −(λ/c₂) ln i: a limb at 0.4 of the centre reads ≈ 970 K cooler.
        let limb = brightness_temperature(0.4);
        assert!((SUN_K - limb - 970.0).abs() < 20.0, "{limb}");
        // Outside (0, 1] it is clamped: never hotter than the centre, never NaN or ≤ 0.
        for i in [0.0, -1.0, 1e-9, 1.5, 100.0, f64::INFINITY] {
            let t = brightness_temperature(i);
            assert!(t.is_finite() && t > 0.0 && t <= SUN_K + 1e-9, "{i}: {t}");
        }
    }

    #[test]
    fn sol_a4_sizes_keep_the_real_order_but_are_compressed() {
        for orbit in [12.0, 47.5, 95.0] {
            let r: Vec<f64> = PLANETS.iter().map(|p| planet_px(p.radius, orbit)).collect();
            // J > S > U > N > E > V > Ma > Me
            let order = [4, 5, 6, 7, 2, 1, 3, 0];
            for w in order.windows(2) {
                let (a, b) = (r[w[0]], r[w[1]]);
                assert!(a > b || b == MIN_PLANET_PX, "{orbit}: {w:?} {a} {b}");
            }
            assert!(r.iter().all(|&x| x >= MIN_PLANET_PX));
            assert!(
                r[JUPITER] / r[2] < 4.0,
                "not to scale: {}",
                r[JUPITER] / r[2]
            );
        }
    }

    #[test]
    fn sol_a5_day_side_faces_the_sun() {
        let mut s = scene(200, 50);
        for k in 0..8 {
            s.phases[JUPITER] = TAU * f64::from(k) / 8.0;
            s.rasterise();
            let (x, y) = s.planet_xy(JUPITER);
            let r = planet_px(1.0, s.outer) - 0.6;
            let (dx, dy) = (s.cx - x, s.cy - y);
            let d = dx.hypot(dy);
            let at = |sign: f64| {
                let (px, py) = (x + sign * r * dx / d, y + sign * r * dy / d);
                s.pixels.get(px.floor() as i32, py.floor() as i32).unwrap()
            };
            let (near, far) = (at(1.0).level, at(-1.0).level);
            // Beyond the Sun the disc is nearly full, so both may be fully lit.
            assert!(near >= far, "phase {k}: {near} < {far}");
            if s.phases[JUPITER].sin() <= 0.0 {
                assert!(near > far, "phase {k}: {near} vs {far}");
            }
        }
        // A 2 × 2 blot keeps half its light on the night side: at 80×24 Mars in crescent,
        // in front of the Sun, is never darker than half its day level.
        let mut s = scene(80, 24);
        const MARS: usize = 3;
        use std::f64::consts::PI;
        assert!(planet_px(PLANETS[MARS].radius, s.outer) < CORNER_BELOW);
        for k in 0..16 {
            s.phases[MARS] = PI + PI * f64::from(k) / 16.0;
            s.rasterise();
            let (x, y) = s.planet_xy(MARS);
            let floor = scaled(PLANETS[MARS].day, NIGHT_SMALL).level;
            for (px, py) in [(0, 0), (-1, 0), (0, -1), (-1, -1)] {
                let ink = s
                    .pixels
                    .get(x.round() as i32 + px, y.round() as i32 + py)
                    .unwrap();
                assert!(ink.role == PLANETS[MARS].day.role, "phase {k}: {ink:?}");
                assert!(ink.level >= floor, "phase {k}: {} < {floor}", ink.level);
            }
        }
    }

    #[test]
    fn sol_a6_saturn_rings_hold_one_ellipse_wherever_saturn_is() {
        // The ring pole is tilted 26.7° and seen from 30° above the ecliptic: an ellipse
        // of axis ratio cos 26.7° · sin 30°.
        let minor = SATURN_TILT_DEG.to_radians().cos() * SQUASH;
        assert!((minor - 0.447).abs() < 0.001);
        let lengths: Vec<f64> = (0..3600)
            .map(|j| {
                let (u, v, _) = ring_offset(1.0, TAU * f64::from(j) / 3600.0);
                u.hypot(v)
            })
            .collect();
        let max = lengths.iter().copied().fold(0.0, f64::max);
        let min = lengths.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            (max - 1.0).abs() < 1e-4 && (min - minor).abs() < 1e-4,
            "{max} {min}"
        );
        // Half the ring is in front of the planet, half behind.
        let front = (0..360)
            .filter(|&j| ring_offset(1.0, TAU * f64::from(j) / 360.0).2 > 0.0)
            .count();
        assert_eq!(front, 180);
        // Drawn: at two phases a quarter orbit apart, every ring dot lies on one of the
        // same two ellipses about the planet.
        let mut s = scene(400, 100);
        for phase in [0.3, 0.3 + TAU / 4.0] {
            s.phases[SATURN] = phase;
            s.dots.clear();
            s.draw_rings();
            let (px, py) = s.planet_xy(SATURN);
            let (x0, y0) = SolarSystem::dot(px, py);
            let r_s = 2.0 * planet_px(PLANETS[SATURN].radius, s.outer);
            let size = s.dots.dot_size();
            let mut n = 0;
            for y in 0..i32::from(size.height()) {
                for x in 0..i32::from(size.width()) {
                    if !s.dots.is_lit(x, y) {
                        continue;
                    }
                    let (dx, dy) = (f64::from(x) - x0, f64::from(y) - y0);
                    let near = RINGS.iter().any(|&(k, _)| {
                        (0..720).any(|j| {
                            let (u, v, _) = ring_offset(k * r_s, TAU * f64::from(j) / 720.0);
                            (u - dx).hypot(v - dy) < 1.0
                        })
                    });
                    assert!(near, "phase {phase}: stray ring dot at ({dx}, {dy})");
                    n += 1;
                }
            }
            assert!(n > 60, "{n} ring dots");
        }
        // The rings cross the ball only where the ball is drawn: at 80×24 (Saturn under
        // 1.5 px, its disc snapped to a pixel corner) every ring-inked pixel near Saturn is
        // one of the disc's own pixels.
        let mut s = scene(80, 24);
        let r = planet_px(PLANETS[SATURN].radius, s.outer);
        assert!(r < CORNER_BELOW);
        for k in 0..32 {
            s.phases[SATURN] = TAU * f64::from(k) / 32.0;
            s.rasterise();
            let (x, y) = s.planet_xy(SATURN);
            let mut ball = Vec::new();
            disc_pixels(x, y, r, |px, py, _, _, _| ball.push((px, py)));
            let (cx, cy) = (x.round() as i32, y.round() as i32);
            for py in cy - 4..=cy + 4 {
                for px in cx - 4..=cx + 4 {
                    let ringed = s
                        .pixels
                        .get(px, py)
                        .is_some_and(|i| RINGS.iter().any(|&(_, ink)| ink == i));
                    assert!(
                        !ringed || ball.contains(&(px, py)),
                        "phase {k}: ring over empty sky at ({px}, {py})"
                    );
                }
            }
        }
    }

    #[test]
    fn sol_a7_distinct_inks_dim_orbits_and_trails_behind() {
        for (i, a) in PLANETS.iter().enumerate() {
            for b in &PLANETS[i + 1..] {
                assert_ne!(a.day, b.day);
            }
        }
        for w in PLANETS.windows(2) {
            let (a, b) = (w[0].day, w[1].day);
            assert!(
                a.role != b.role || a.level.abs_diff(b.level) >= 30,
                "{a:?} {b:?}"
            );
        }
        assert!(ORBIT_INK.role == Role::Dim && ORBIT_INK.level <= 40);
        // Trails: every planet at orbit angle 1 rad, so its trail lies at smaller angles.
        let mut s = scene(200, 50);
        s.belt.clear();
        s.phases = [1.0; 8];
        s.rasterise();
        let size = s.dots.dot_size();
        let (cx, cy) = SolarSystem::dot(s.cx, s.cy);
        let mut seen = 0;
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                if !s.dots.is_lit(x, y) {
                    continue;
                }
                // Back from the screen ellipse to the orbit plane: angle and radius.
                let (ex, ey) = (f64::from(x) - cx, -(f64::from(y) - cy) / SQUASH);
                let off = ey.atan2(ex) - 1.0;
                let r = ex.hypot(ey) / (2.0 * s.outer);
                // Anything clearly ahead of the planets must be an orbit dot or a ring.
                let ring = (r - PLANETS[SATURN].frac).abs() < 0.13;
                if off > 0.08 && off < 0.4 && !ring {
                    let on_orbit = PLANETS.iter().any(|p| (r - p.frac).abs() < 0.02);
                    assert!(on_orbit, "stray dot ahead at r = {r}");
                }
                if (-0.4..-0.05).contains(&off) {
                    seen += 1;
                }
            }
        }
        assert!(seen > 20, "trails behind: {seen}");
    }

    #[test]
    fn sol_a8_the_two_bottom_rows_stay_clear_for_the_title() {
        // Nothing on the title row, nor on the row above it, so no orbit dot trails the
        // title like an ellipsis (GEN-R13), at any time or size.
        for (w, h) in [(80, 24), (200, 50), (120, 40), (40, 12)] {
            let mut stage = Stage::new(build(), 5, Size::new(w, h), GlyphMode::Ascii);
            stage.set_title_visible(false);
            for _ in 0..30 {
                let canvas = stage.render();
                for y in [i32::from(h) - 2, i32::from(h) - 1] {
                    for x in 0..i32::from(w) {
                        let c = canvas.get(x, y).unwrap();
                        assert!(c.ch == ' ', "{w}x{h}: '{}' at ({x}, {y})", c.ch);
                    }
                }
                for _ in 0..4 {
                    stage.step(0.25);
                }
            }
        }
    }

    #[test]
    fn sol_a10_discs_are_symmetric_round_blots_never_a_plus() {
        for r in [0.5, 0.9, 1.2, 1.49, 1.5, 2.0, 2.4, 3.5] {
            for (x, y) in [(10.1, 7.2), (10.49, 7.51), (10.9, 7.9)] {
                let mut px = Vec::new();
                disc_pixels(x, y, r, |x, y, _, _, shade| px.push((x, y, shade)));
                // Four-fold symmetric about the snapped centre: its bounding box is square
                // and every pixel's quarter-turn about the box centre is drawn too.
                let (x0, x1) = (px.iter().map(|p| p.0).min(), px.iter().map(|p| p.0).max());
                let (y0, y1) = (px.iter().map(|p| p.1).min(), px.iter().map(|p| p.1).max());
                let (x0, x1, y0, y1) = (x0.unwrap(), x1.unwrap(), y0.unwrap(), y1.unwrap());
                assert_eq!(x1 - x0, y1 - y0, "r={r}");
                for &(a, b, _) in &px {
                    let (u, v) = (x0 + (y1 - b), y0 + (a - x0));
                    assert!(px.iter().any(|p| (p.0, p.1) == (u, v)), "r={r}: {px:?}");
                }
                // Not a plus: the corners of the inner 2 × 2 (small) or 3 × 3 (large)
                // square are drawn, at most lightly shaded.
                let side = if r < CORNER_BELOW { 2 } else { 3 };
                let (cx, cy) = ((x0 + x1 - side + 1) / 2, (y0 + y1 - side + 1) / 2);
                for (a, b) in [(0, 0), (side - 1, 0), (0, side - 1), (side - 1, side - 1)] {
                    let full = px
                        .iter()
                        .any(|p| (p.0, p.1) == (cx + a, cy + b) && p.2 >= 0.75);
                    assert!(full, "r={r} ({x}, {y}): corner ({a}, {b}) of {px:?}");
                }
            }
        }
    }

    #[test]
    fn sol_a9_the_sun_is_the_brightest_object_in_mono() {
        // In `mono` a blackbody keeps its colour's lightness (RND-R11), so the near-white
        // Sun's core outshines every planet and ring pixel.
        use sciart_render::Theme;
        let grey = |ink: Ink| Theme::Mono.color(ink, 0, 0).0;
        let mut s = scene(200, 50);
        let mut ctx = Ctx::new(1, Size::new(200, 50));
        let sun = s.pixels.get(s.cx as i32, s.cy as i32).unwrap();
        assert_eq!(sun.role, Role::Blackbody);
        for _ in 0..20 {
            for _ in 0..30 {
                s.update(SIM_DT, &mut ctx);
            }
            for y in 0..100 {
                for x in 0..200 {
                    let ink = s.pixels.get(x, y).unwrap();
                    if ink.role != Role::Blackbody && ink.role != Role::Background {
                        assert!(grey(ink) < grey(sun), "{ink:?} at ({x}, {y})");
                    }
                }
            }
        }
    }
}
