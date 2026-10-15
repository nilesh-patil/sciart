//! The eight planets on Kepler orbits around the Sun (SPEC-002 `solar_system`).
//!
//! Look (M20, GEN-R25): everything is braille on one [`Dots`] lattice. The Sun and the
//! planets are stipple atoms sized by class, each planet drags a closed-form 2 s speed
//! trail (inner planets long arcs, outer ones short stubs: Kepler's law made visible),
//! orbits are faint dotted hints and the belt is a scatter of dust dots.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::draw;
use sciart_core::prelude::*;
use sciart_core::raster::Dots;

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
const EARTH_FRAC: f64 = 0.30;
/// Belt dust dots: 4× M11's 60 (SPEC-002 Look (M20)).
const BELT: usize = 240;
/// Simulated seconds of orbit each speed trail covers.
const TRAIL_S: f64 = 2.0;
/// Trail level at its oldest end, as a fraction of its newest end (≈ 25 % of the planet's).
const TRAIL_TAIL: f64 = 0.28;
/// Trail level at its newest end, a little under the planet's so the atom stays on top.
const TRAIL_HEAD: f64 = 0.9;
/// Dots per trail segment (the thread is a polyline of chords this long).
const TRAIL_STEP: f64 = 2.0;
const TRAIL_MAX_SEGMENTS: usize = 64;
/// The Sun atom's σ as a fraction of the outer orbit radius (dots).
const SUN_SIGMA: f64 = 0.045;
/// Orbit hints light one dot in this many along each orbit.
const ORBIT_EVERY: f64 = 4.0;
const ORBIT_LEVEL: u8 = 110;
const BELT_LEVEL: u8 = 120;
const PLANET_LEVEL: u8 = 255;
/// Saturn's ring: semi-major axis in σ, axis ratio 3:1, tilted 20°.
const RING_A: f64 = 3.4;
const RING_TILT: f64 = 20.0 * TAU / 360.0;
const RING_SEGMENTS: usize = 24;
const RING_LEVEL: u8 = 220;

struct Planet {
    /// Orbit radius as a fraction of the outermost orbit.
    frac: f64,
    /// Stipple atom σ in dots, by class (SOL-A6).
    sigma: f64,
    role: Role,
}

const PLANETS: [Planet; 8] = [
    Planet {
        frac: 0.14,
        sigma: 0.9,
        role: Role::Structure,
    },
    Planet {
        frac: 0.22,
        sigma: 1.0,
        role: Role::Sun,
    },
    Planet {
        frac: EARTH_FRAC,
        sigma: 1.0,
        role: Role::Planet,
    },
    Planet {
        frac: 0.40,
        sigma: 0.9,
        role: Role::Hot,
    },
    Planet {
        frac: 0.60,
        sigma: 1.85,
        role: Role::Accent,
    },
    Planet {
        frac: 0.74,
        sigma: 1.55,
        role: Role::Sun,
    },
    Planet {
        frac: 0.87,
        sigma: 1.35,
        role: Role::Wave,
    },
    Planet {
        frac: 1.00,
        sigma: 1.35,
        role: Role::Cold,
    },
];
const SATURN: usize = 5;

/// Kepler's third law: angular speed for an orbit at `frac` of the outer radius (SOL-A1).
/// `ω(a) = ω⊕ · (a/a⊕)^-3/2`, i.e. `T² ∝ a³` with Earth's year pinned to [`EARTH_PERIOD`].
pub(crate) fn angular_speed(frac: f64) -> f64 {
    TAU / EARTH_PERIOD * (frac / EARTH_FRAC).powf(-1.5)
}

#[derive(Debug)]
pub struct SolarSystem {
    size: Size,
    /// The Sun's centre in dots: the middle of the centre cell (SOL-A3).
    ox: f64,
    oy: f64,
    /// Outer orbit radius in dots.
    outer: f64,
    phases: [f64; 8],
    /// (orbit fraction, angle, angular speed)
    belt: Vec<(f64, f64, f64)>,
    dots: Dots,
    /// Rasterise in `prepare` only when the state changed (GEN-R20).
    dirty: bool,
}

impl Default for SolarSystem {
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            ox: 0.0,
            oy: 0.0,
            outer: 0.0,
            phases: [0.0; 8],
            belt: Vec::new(),
            dots: Dots::default(),
            dirty: true,
        }
    }
}

/// The orbit geometry at one instant, copied out so rasterising can borrow the lattice.
#[derive(Debug, Clone, Copy)]
struct View {
    ox: f64,
    oy: f64,
    outer: f64,
    phases: [f64; 8],
}

impl View {
    /// Planet `i`'s centre in dots at orbit angle `angle`.
    fn at(&self, i: usize, angle: f64) -> (f64, f64) {
        let r = self.outer * PLANETS[i].frac;
        let (s, c) = angle.sin_cos_det();
        (self.ox + r * c, self.oy + r * s)
    }

    /// Planet `i`'s speed trail as chords `(x0, y0, x1, y1, age)`, newest first, where
    /// `age ∈ [0, 1]` is the chord's midpoint age as a fraction of [`TRAIL_S`]. Closed
    /// form from the phase: the arc swept over the last [`TRAIL_S`] seconds.
    fn trail(&self, i: usize, mut f: impl FnMut(f64, f64, f64, f64, f64)) {
        let omega = angular_speed(PLANETS[i].frac);
        let sweep = omega * TRAIL_S;
        let len = self.outer * PLANETS[i].frac * sweep;
        let n = ((len / TRAIL_STEP).ceil() as usize).clamp(1, TRAIL_MAX_SEGMENTS);
        let phase = self.phases[i];
        let mut a = self.at(i, phase);
        for k in 0..n {
            let u1 = (k + 1) as f64 / n as f64;
            let b = self.at(i, phase - sweep * u1);
            f(a.0, a.1, b.0, b.1, (k as f64 + 0.5) / n as f64);
            a = b;
        }
    }

    /// Length in dots of planet `i`'s drawn speed trail (SOL-A4).
    #[cfg(test)]
    fn trail_arc_dots(&self, i: usize) -> f64 {
        let mut len = 0.0;
        self.trail(i, |x0, y0, x1, y1, _| len += (x1 - x0).hypot(y1 - y0));
        len
    }
}

impl SolarSystem {
    fn view(&self) -> View {
        View {
            ox: self.ox,
            oy: self.oy,
            outer: self.outer,
            phases: self.phases,
        }
    }

    fn rasterise(&mut self) {
        let v = self.view();
        let dots = &mut self.dots;
        dots.clear();
        // Orbit hints: one dot in four, faint (drawn first, so everything else wins).
        let hint = Ink::new(Role::Dim, ORBIT_LEVEL);
        for p in &PLANETS {
            let r = self.outer * p.frac;
            let n = ((TAU * r / ORBIT_EVERY).round() as usize).max(4);
            let step = TAU / n as f64;
            for k in 0..n {
                let (s, c) = (k as f64 * step).sin_cos_det();
                dots.plot(self.ox + r * c, self.oy + r * s, hint);
            }
        }
        // Belt dust.
        let dust = Ink::new(Role::Particle, BELT_LEVEL);
        for &(frac, angle, _) in &self.belt {
            let r = self.outer * frac;
            let (s, c) = angle.sin_cos_det();
            dots.plot(self.ox + r * c, self.oy + r * s, dust);
        }
        // Speed trails: 1-px threads fading linearly with age.
        for (i, p) in PLANETS.iter().enumerate() {
            v.trail(i, |x0, y0, x1, y1, age| {
                let frac = TRAIL_HEAD * (1.0 - (1.0 - TRAIL_TAIL) * age);
                let level = (f64::from(PLANET_LEVEL) * frac).round() as u8;
                dots.line(x0, y0, x1, y1, Ink::new(p.role, level));
            });
        }
        // The Sun.
        let sun = Ink::new(Role::Sun, PLANET_LEVEL);
        dots.stipple_puff(self.ox, self.oy, SUN_SIGMA * self.outer, 1.0, 0x50, sun);
        // Planets, with Saturn's ring.
        for (i, p) in PLANETS.iter().enumerate() {
            let (x, y) = v.at(i, v.phases[i]);
            let ink = Ink::new(p.role, PLANET_LEVEL);
            dots.stipple_puff(x, y, p.sigma, 1.0, 0x51 + i as u64, ink);
            if i == SATURN {
                let (ra, rb) = (RING_A * p.sigma, RING_A * p.sigma / 3.0);
                let (ts, tc) = RING_TILT.sin_cos_det();
                let ring = Ink::new(p.role, RING_LEVEL);
                let point = |k: usize| {
                    let (s, c) = (k as f64 * TAU / RING_SEGMENTS as f64).sin_cos_det();
                    let (u, v) = (ra * c, rb * s);
                    (x + u * tc - v * ts, y + u * ts + v * tc)
                };
                let mut a = point(0);
                for k in 1..=RING_SEGMENTS {
                    let b = point(k);
                    dots.line(a.0, a.1, b.0, b.1, ring);
                    a = b;
                }
            }
        }
    }
}

impl Generator for SolarSystem {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        let (ox, oy) = Dots::from_cells(
            f64::from(size.width() / 2) + 0.5,
            f64::from(size.height() / 2) + 0.5,
        );
        (self.ox, self.oy) = (ox, oy);
        // Rows → dots: 4 dots per row, and 2 dots per column × ASPECT columns per row.
        self.outer = draw::fit_radius(size, 0.95) * f64::from(Dots::CELL_H);
        if self.belt.is_empty() {
            // Positions survive resizes; only the scale changes.
            for p in &mut self.phases {
                *p = ctx.rng.random_range(0.0..TAU);
            }
            self.belt = (0..BELT)
                .map(|_| {
                    let frac = ctx.rng.random_range(0.48..0.54);
                    (frac, ctx.rng.random_range(0.0..TAU), angular_speed(frac))
                })
                .collect();
        }
        self.dots.resize(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        for (phase, planet) in self.phases.iter_mut().zip(&PLANETS) {
            *phase = (*phase + angular_speed(planet.frac) * dt) % TAU;
        }
        for (_, angle, omega) in &mut self.belt {
            *angle = (*angle + *omega * dt) % TAU;
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sol_a1_kepler_speeds_decrease_outward_and_earth_year_is_12s() {
        let speeds: Vec<f64> = PLANETS.iter().map(|p| angular_speed(p.frac)).collect();
        assert!(speeds.windows(2).all(|w| w[0] > w[1]), "{speeds:?}");
        assert!((TAU / angular_speed(EARTH_FRAC) - EARTH_PERIOD).abs() < 1e-9);
        // T² ∝ a³
        let t = |f: f64| TAU / angular_speed(f);
        assert!((t(1.0).powi(2) / t(0.3).powi(2) - (1.0f64 / 0.3).powi(3)).abs() < 1e-6);
    }

    #[test]
    fn sol_a2_planets_stay_on_canvas() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(1, size);
        let mut g = SolarSystem::default();
        g.resize(size, &mut ctx);
        let ds = g.dots.dot_size();
        for _ in 0..40 {
            for _ in 0..30 {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            for i in 0..8 {
                let (x, y) = g.view().at(i, g.phases[i]);
                let (x, y) = (x.round() as i32, y.round() as i32);
                assert!((0..i32::from(ds.width())).contains(&x), "planet {i} x={x}");
                assert!((0..i32::from(ds.height())).contains(&y), "planet {i} y={y}");
                assert!(g.dots.is_lit(x, y), "planet {i} centre dot lit");
            }
        }
    }

    #[test]
    fn sol_a3_sun_core_on_center_cell() {
        for mode in [GlyphMode::Unicode, GlyphMode::Ascii] {
            let mut stage = Stage::new(build(), 1, Size::new(200, 50), mode);
            let cell = *stage.render().get(100, 25).unwrap();
            assert_eq!(cell.ink, Ink::new(Role::Sun, PLANET_LEVEL), "{mode:?}");
            assert_ne!(cell.ch, ' ', "{mode:?}");
        }
    }

    fn stage_at(size: Size, secs: f64) -> Stage {
        let mut stage = Stage::new(build(), 1, size, GlyphMode::Unicode);
        for _ in 0..(secs * 60.0).round() as usize {
            stage.step(SIM_DT);
        }
        stage
    }

    #[test]
    fn sol_a4_trail_arc_length_decreases_outward() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(1, size);
        let mut g = SolarSystem::default();
        g.resize(size, &mut ctx);
        let arcs: Vec<f64> = (0..8).map(|i| g.view().trail_arc_dots(i)).collect();
        assert!(arcs.windows(2).all(|w| w[0] > w[1]), "{arcs:?}");
        assert!(arcs[7] > 4.0, "Neptune still draws a stub: {arcs:?}");
    }

    #[test]
    fn sol_a5_everything_is_braille_at_2s_and_9s() {
        let size = Size::new(200, 50);
        for t in [2.0, 9.0] {
            let mut stage = stage_at(size, t);
            let frame = stage.render();
            let (mut lit, mut braille, mut blocks) = (0u32, 0u32, 0u32);
            for y in 0..50 {
                for x in 0..200 {
                    let ch = frame.get(x, y).unwrap().ch;
                    blocks += u32::from(matches!(ch, '▀' | '▄' | '█'));
                    if y == 49 || ch == ' ' || ch == '\u{2800}' {
                        continue;
                    }
                    lit += 1;
                    braille += u32::from(('\u{2801}'..='\u{28FF}').contains(&ch));
                }
            }
            assert_eq!(blocks, 0, "t={t}");
            let share = f64::from(braille) / f64::from(lit.max(1));
            assert!(share >= 0.9, "t={t}: braille share {share:.3} of {lit}");
        }
    }

    #[test]
    fn sol_a6_atom_sizes_follow_class_and_roles_are_colourful() {
        let s = |i: usize| PLANETS[i].sigma;
        let (jup, sat, ura, nep) = (s(4), s(5), s(6), s(7));
        assert!(jup > sat && sat > ura && (ura - nep).abs() < 1e-12);
        for i in 0..4 {
            assert!(nep > s(i) && (0.9..=1.0).contains(&s(i)), "planet {i}");
        }
        let mut roles: Vec<Role> = Vec::new();
        for p in &PLANETS {
            if !roles.contains(&p.role) {
                roles.push(p.role);
            }
        }
        assert!(roles.len() >= 6, "{roles:?}");
    }
}
