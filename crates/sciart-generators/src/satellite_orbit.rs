//! A rotating Earth with satellites on inclined orbits (SPEC-002 `satellite_orbit`).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "satellite_orbit",
    category: Category::SpaceExploration,
    description: "A rotating Earth with satellites on inclined orbits",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SatelliteOrbit::default())
}

/// Sidereal day in simulated seconds.
const SPIN_PERIOD: f64 = 20.0;
/// Period of a (grazing) orbit at `a = Re`.
const ORBIT_PERIOD: f64 = 6.0;
const SATELLITES: usize = 5;
const A_RANGE: (f64, f64) = (1.25, 2.2);
const INC_RANGE: (f64, f64) = (0.2, 1.2);
/// Spin axis tipped toward the viewer so the north cap shows (≈ Earth's 23.4°).
const TILT: f64 = 0.41;
/// Unit sun direction in view space (x right, y up, z toward the viewer): upper left, so the
/// terminator crosses the right third of the disc.
const SUN: (f64, f64, f64) = (-0.768, 0.256, 0.588);
const MODES: usize = 6;
/// Continents where the normalized mode sum exceeds this (≈ 30 % land for 6 random modes).
const LAND: f64 = 0.15;
/// Latitude (rad) where the ice caps begin, before coastline noise.
const ICE_LAT: f64 = 1.18;
const STAR_DENSITY: f64 = 0.004;

/// Night ocean is `-`, not `·`, so the dark hemisphere never reads as an orbit path.
const OCEAN: [Glyph; 3] = [Glyph::ascii('-'), Glyph::ascii('~'), Glyph::new('≈', '=')];
const LAND_RAMP: [Glyph; 3] = [
    Glyph::new('░', ':'),
    Glyph::new('▒', '%'),
    Glyph::new('▓', '#'),
];
const ICE: [Glyph; 3] = [
    Glyph::new('░', '.'),
    Glyph::new('▒', '+'),
    Glyph::new('█', '#'),
];
const PATH: Glyph = Glyph::new('·', '.');
const SAT: Glyph = Glyph::new('◆', '*');

/// Kepler's third law in Earth radii: `ω = (2π/6 s)·(a/Re)^-3/2` (SAT-A2).
pub(crate) fn angular_speed(a_over_re: f64) -> f64 {
    TAU / ORBIT_PERIOD * a_over_re.powf(-1.5)
}

/// 3-D position of a circular orbit of radius `a` (SAT-A3).
///
/// The orbit starts in the screen plane, is tilted by `inc` about the screen x axis, then
/// turned by `node` about the line of sight. x is right, y is down (rows), z toward the viewer.
pub(crate) fn position(a: f64, inc: f64, node: f64, theta: f64) -> (f64, f64, f64) {
    let (st, ct) = theta.sin_cos();
    let (si, ci) = inc.sin_cos();
    let (x, y, z) = (a * ct, a * st * ci, a * st * si);
    let (sn, cn) = node.sin_cos();
    (x * cn - y * sn, x * sn + y * cn, z)
}

/// Whether Earth (radius `re`, at the origin) blocks the view of `pos` (SAT-A1).
pub(crate) fn hidden(pos: (f64, f64, f64), re: f64) -> bool {
    pos.2 < 0.0 && pos.0.hypot(pos.1) < re
}

/// One seeded sinusoid of the continent field; `m` is an integer so longitude wraps cleanly.
#[derive(Debug, Default, Clone, Copy)]
struct Mode {
    m: f64,
    n: f64,
    phase: f64,
    amp: f64,
}

#[derive(Debug, Default, Clone, Copy)]
struct Satellite {
    a_over_re: f64,
    inc: f64,
    node: f64,
    theta: f64,
}

#[derive(Debug, Default)]
pub struct SatelliteOrbit {
    t: f64,
    cx: f64,
    cy: f64,
    re: f64,
    modes: [Mode; MODES],
    amp_sum: f64,
    sats: Vec<Satellite>,
    /// Background stars as fractions of the screen, so they survive resizes.
    stars: Vec<(f64, f64)>,
    size: (f64, f64),
}

impl SatelliteOrbit {
    /// Normalized continent field in `[-1, 1]` at body longitude/latitude.
    fn terrain(&self, lon: f64, lat: f64) -> f64 {
        let sum: f64 = self
            .modes
            .iter()
            .map(|k| k.amp * (k.m * lon + k.n * lat + k.phase).sin())
            .sum();
        if self.amp_sum > 0.0 {
            sum / self.amp_sum
        } else {
            0.0
        }
    }

    /// Orthographic shading of the disc point with normalized screen offsets `(nx, ny)`.
    fn surface(&self, d: f64, nx: f64, ny: f64) -> (Glyph, Ink) {
        // View-space unit normal (y up).
        let (x, y, z) = (nx, -ny, (1.0 - d * d).max(0.0).sqrt());
        let (st, ct) = TILT.sin_cos();
        // Body frame: pole = (0, cos T, sin T); the prime meridian faces the viewer at t = 0.
        let lat = (y * ct + z * st).clamp(-1.0, 1.0).asin();
        // Prograde spin: surface features drift left → right across the near side.
        let lon = x.atan2(z * ct - y * st) - TAU * self.t / SPIN_PERIOD;
        let f = self.terrain(lon, lat);

        let light = x * SUN.0 + y * SUN.1 + z * SUN.2;
        // A soft terminator: a little twilight past the geometric shadow line.
        let b = ((light + 0.12) / 1.12).clamp(0.0, 1.0);
        let level = (40.0 + 215.0 * b) as u8;
        let (ramp, role) = if lat.abs() > ICE_LAT - 0.2 * f {
            (&ICE, Role::Text)
        } else if f > LAND {
            (&LAND_RAMP, Role::Particle)
        } else {
            (&OCEAN, Role::Cold)
        };
        (draw::ramp(b, ramp), Ink::new(role, level))
    }

    fn screen(&self, (x, y, _): (f64, f64, f64)) -> (i32, i32) {
        (
            (self.cx + x * ASPECT).round() as i32,
            (self.cy + y).round() as i32,
        )
    }
}

impl Generator for SatelliteOrbit {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cx = f64::from(size.width() / 2);
        self.cy = f64::from(size.height() / 2);
        self.re = draw::fit_radius(size, 0.45);
        self.size = (f64::from(size.width()), f64::from(size.height()));
        if self.sats.is_empty() {
            // The world and its satellites survive resizes; only the scale changes.
            for k in &mut self.modes {
                *k = Mode {
                    m: f64::from(ctx.rng.random_range(1..=4u8)),
                    n: ctx.rng.random_range(-3.0..3.0),
                    phase: ctx.rng.random_range(0.0..TAU),
                    amp: ctx.rng.random_range(0.4..1.0),
                };
            }
            self.amp_sum = self.modes.iter().map(|k| k.amp).sum();
            // Stratified draws keep the shells apart and guarantee a spread of tilts, so
            // some orbits always dip behind the planet.
            let strata = SATELLITES as f64;
            self.sats = (0..SATELLITES)
                .map(|i| {
                    let s = i as f64;
                    let lerp = |(lo, hi): (f64, f64), u: f64| lo + (hi - lo) * (s + u) / strata;
                    Satellite {
                        a_over_re: lerp(A_RANGE, ctx.rng.random_range(0.2..0.8)),
                        inc: lerp(INC_RANGE, ctx.rng.random_range(0.0..1.0)),
                        node: ctx.rng.random_range(0.0..TAU),
                        theta: ctx.rng.random_range(0.0..TAU),
                    }
                })
                .collect();
            // Inclination ⟂ radius: shuffle so the widest orbit isn't always the most tilted.
            for i in (1..SATELLITES).rev() {
                let j = ctx.rng.random_range(0..=i);
                let inc = self.sats[i].inc;
                self.sats[i].inc = self.sats[j].inc;
                self.sats[j].inc = inc;
            }
        }
        let want = (STAR_DENSITY * self.size.0 * self.size.1) as usize;
        while self.stars.len() < want {
            self.stars.push((
                ctx.rng.random_range(0.0..1.0),
                ctx.rng.random_range(0.0..1.0),
            ));
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        for s in &mut self.sats {
            s.theta += angular_speed(s.a_over_re) * dt;
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (w, h) = self.size;
        let want = (STAR_DENSITY * w * h) as usize;
        for &(u, v) in self.stars.iter().take(want) {
            canvas.put(
                (u * w) as i32,
                (v * h) as i32,
                Glyph::ascii('.'),
                Ink::new(Role::Dim, 60),
            );
        }

        let (cx, cy, re) = (self.cx, self.cy, self.re);
        draw::disc(canvas, cx, cy, re, |d, nx, ny| {
            Some(self.surface(d, nx, ny))
        });

        // Atmosphere: a thin limb glow on the day side only.
        let halo = re + 0.7;
        let samples = ((TAU * halo * ASPECT * 2.0).ceil() as usize).clamp(8, 4000);
        for i in 0..samples {
            let phi = TAU * i as f64 / samples as f64;
            let (s, c) = phi.sin_cos();
            let light = c * SUN.0 - s * SUN.1;
            if light > -0.1 {
                let level = (70.0 + 110.0 * light.max(0.0)) as u8;
                let (x, y) = self.screen((halo * c, halo * s, 0.0));
                canvas.put(x, y, PATH, Ink::new(Role::Cold, level));
            }
        }

        // Dotted orbit paths: roughly one dot every four cells; far arcs drawn fainter.
        for s in &self.sats {
            let a = s.a_over_re * re;
            let dots = ((TAU * a * ASPECT / 4.0) as usize).clamp(12, 2000);
            for i in 0..dots {
                let p = position(a, s.inc, s.node, TAU * i as f64 / dots as f64);
                if !hidden(p, re) {
                    let (x, y) = self.screen(p);
                    let level = if p.2 >= 0.0 { 120 } else { 55 };
                    canvas.put(x, y, PATH, Ink::new(Role::Dim, level));
                }
            }
        }

        // Far-side satellites first so a near one wins a shared cell.
        let far = |s: &&Satellite| position(s.a_over_re, s.inc, s.node, s.theta).2 < 0.0;
        let near = |s: &&Satellite| !far(s);
        for s in self
            .sats
            .iter()
            .filter(far)
            .chain(self.sats.iter().filter(near))
        {
            let a = s.a_over_re * re;
            // A short wake, about one cell per dot along the orbit.
            let step = 1.0 / (a * ASPECT).max(1.0);
            for k in (1..=2u8).rev() {
                let p = position(a, s.inc, s.node, s.theta - f64::from(k) * step);
                if !hidden(p, re) {
                    let (x, y) = self.screen(p);
                    canvas.put(x, y, PATH, Ink::new(Role::Accent, 200 - 60 * k));
                }
            }
            let p = position(a, s.inc, s.node, s.theta);
            if !hidden(p, re) {
                let (x, y) = self.screen(p);
                let level = if p.2 >= 0.0 { 255 } else { 170 };
                canvas.put(x, y, SAT, Ink::new(Role::Accent, level));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn norm((x, y, z): (f64, f64, f64)) -> f64 {
        (x * x + y * y + z * z).sqrt()
    }

    #[test]
    fn sat_a1_hidden_iff_behind_and_inside_disc() {
        let re = 10.0;
        assert!(hidden((3.0, -4.0, -1.0), re), "behind, inside");
        assert!(!hidden((3.0, -4.0, 1.0), re), "in front, inside");
        assert!(!hidden((8.0, 7.0, -5.0), re), "behind, outside the disc");
        assert!(!hidden((0.0, 0.0, 0.0), re), "z = 0 is not behind");
        // Sweep a steep orbit: hidden exactly where both conditions hold.
        for i in 0..720 {
            let p = position(1.3 * re, 1.1, 0.4, TAU * f64::from(i) / 720.0);
            assert_eq!(hidden(p, re), p.2 < 0.0 && p.0.hypot(p.1) < re, "{p:?}");
        }
    }

    #[test]
    fn sat_a1_hidden_satellite_is_not_drawn_but_a_front_one_is() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(3, size);
        let mut g = SatelliteOrbit::default();
        g.resize(size, &mut ctx);
        let re = g.re;
        // θ = -π/2 with inc = π/2 - ε puts the satellite almost straight behind Earth's center.
        let behind = Satellite {
            a_over_re: 1.5,
            inc: FRAC_PI_2 - 0.05,
            node: 0.0,
            theta: -FRAC_PI_2,
        };
        let front = Satellite {
            theta: FRAC_PI_2,
            ..behind
        };
        for (sat, want) in [(behind, false), (front, true)] {
            g.sats = vec![sat];
            let p = position(sat.a_over_re * re, sat.inc, sat.node, sat.theta);
            assert_eq!(hidden(p, re), !want);
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut canvas);
            let (x, y) = g.screen(p);
            let cell = canvas.get(x, y).unwrap();
            let drawn = cell.ch == '*' && cell.ink.role == Role::Accent;
            assert_eq!(drawn, want, "{sat:?} at {p:?}");
        }
    }

    #[test]
    fn sat_a2_angular_speed_decreases_with_radius() {
        let a: Vec<f64> = (0..=20).map(|i| 1.0 + 0.1 * f64::from(i)).collect();
        let w: Vec<f64> = a.iter().map(|&a| angular_speed(a)).collect();
        assert!(w.windows(2).all(|p| p[0] > p[1]), "{w:?}");
        assert!((TAU / angular_speed(1.0) - ORBIT_PERIOD).abs() < 1e-12);
        // T² ∝ a³
        let t = |a: f64| TAU / angular_speed(a);
        assert!((t(2.2).powi(2) / t(1.25).powi(2) - (2.2f64 / 1.25).powi(3)).abs() < 1e-9);
    }

    #[test]
    fn sat_a3_position_lies_on_sphere_of_radius_a() {
        for (a, inc, node) in [(5.0, 0.2, 0.0), (12.5, 1.2, 2.0), (30.0, 0.7, -4.0)] {
            for i in 0..64 {
                let p = position(a, inc, node, TAU * f64::from(i) / 64.0 + 0.3);
                assert!((norm(p) - a).abs() < 1e-9 * a, "{p:?}");
            }
        }
        // i = 0 stays in the screen plane.
        assert_eq!(position(3.0, 0.0, 1.0, 2.0).2, 0.0);
    }

    #[test]
    fn sat_orbits_respect_spec_ranges_and_spin_moves_continents() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(9, size);
        let mut g = SatelliteOrbit::default();
        g.resize(size, &mut ctx);
        assert_eq!(g.sats.len(), SATELLITES);
        for s in &g.sats {
            assert!((A_RANGE.0..=A_RANGE.1).contains(&s.a_over_re), "{s:?}");
            assert!((INC_RANGE.0..=INC_RANGE.1).contains(&s.inc), "{s:?}");
        }
        // The terrain field wraps in longitude (integer wavenumbers).
        for lat in [-1.0, 0.0, 0.7] {
            assert!((g.terrain(0.4, lat) - g.terrain(0.4 + TAU, lat)).abs() < 1e-9);
        }
        // A quarter day later the globe shows a different face.
        let globe = |g: &SatelliteOrbit| -> Vec<(Glyph, Ink)> {
            (0..40)
                .map(|i| {
                    let (nx, ny) = (f64::from(i) / 40.0 - 0.5, 0.1);
                    g.surface(nx.hypot(ny), nx, ny)
                })
                .collect()
        };
        let before = globe(&g);
        g.t = SPIN_PERIOD / 4.0;
        assert_ne!(before, globe(&g));
    }
}
