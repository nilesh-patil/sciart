//! The eight planets on Kepler orbits around the Sun (SPEC-002 `solar_system`).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{draw, geom};

pub static META: Meta = Meta {
    name: "solar_system",
    category: Category::Astronomy,
    description: "The eight planets orbiting the Sun at Kepler speeds, with an asteroid belt",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SolarSystem::default())
}

/// Earth's year in simulated seconds.
const EARTH_PERIOD: f64 = 12.0;
const EARTH_FRAC: f64 = 0.30;
const BELT: usize = 60;

struct Planet {
    glyph: Glyph,
    /// Orbit radius as a fraction of the outermost orbit.
    frac: f64,
}

const PLANETS: [Planet; 8] = [
    Planet {
        glyph: Glyph::new('☿', 'm'),
        frac: 0.14,
    },
    Planet {
        glyph: Glyph::new('♀', 'v'),
        frac: 0.22,
    },
    Planet {
        glyph: Glyph::new('⊕', 'e'),
        frac: EARTH_FRAC,
    },
    Planet {
        glyph: Glyph::new('♂', 'M'),
        frac: 0.40,
    },
    Planet {
        glyph: Glyph::new('♃', 'J'),
        frac: 0.60,
    },
    Planet {
        glyph: Glyph::new('♄', 'S'),
        frac: 0.74,
    },
    Planet {
        glyph: Glyph::new('⛢', 'U'),
        frac: 0.87,
    },
    Planet {
        glyph: Glyph::new('♆', 'N'),
        frac: 1.00,
    },
];

const SUN: [Glyph; 3] = [Glyph::ascii('@'), Glyph::ascii('O'), Glyph::ascii('o')];
const ORBIT: Glyph = Glyph::new('·', '.');

/// Kepler's third law: angular speed for an orbit at `frac` of the outer radius (SOL-A1).
/// `ω(a) = ω⊕ · (a/a⊕)^-3/2`, i.e. `T² ∝ a³` with Earth's year pinned to [`EARTH_PERIOD`].
pub(crate) fn angular_speed(frac: f64) -> f64 {
    TAU / EARTH_PERIOD * (frac / EARTH_FRAC).powf(-1.5)
}

#[derive(Debug, Default)]
pub struct SolarSystem {
    cx: f64,
    cy: f64,
    outer: f64,
    phases: [f64; 8],
    /// (orbit fraction, angle)
    belt: Vec<(f64, f64)>,
}

impl Generator for SolarSystem {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cx = f64::from(size.width() / 2);
        self.cy = f64::from(size.height() / 2);
        self.outer = draw::fit_radius(size, 0.95);
        if self.belt.is_empty() {
            // Positions survive resizes; only the scale changes.
            for p in &mut self.phases {
                *p = ctx.rng.random_range(0.0..TAU);
            }
            self.belt = (0..BELT)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.48..0.54),
                        ctx.rng.random_range(0.0..TAU),
                    )
                })
                .collect();
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        for (phase, planet) in self.phases.iter_mut().zip(&PLANETS) {
            *phase += angular_speed(planet.frac) * dt;
        }
        for (frac, angle) in &mut self.belt {
            *angle += angular_speed(*frac) * dt;
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (cx, cy, outer) = (self.cx, self.cy, self.outer);
        for p in &PLANETS {
            draw::orbit(
                canvas,
                cx,
                cy,
                outer * p.frac,
                ORBIT,
                Ink::new(Role::Dim, 90),
            );
        }
        for &(frac, angle) in &self.belt {
            let (x, y) = geom::polar(cx, cy, outer * frac, angle);
            canvas.put(x, y, Glyph::ascii('.'), Ink::new(Role::Particle, 120));
        }
        for (p, &phase) in PLANETS.iter().zip(&self.phases) {
            for k in 1..=3u8 {
                let (x, y) = geom::polar(cx, cy, outer * p.frac, phase - f64::from(k) * 0.08);
                canvas.put(
                    x,
                    y,
                    Glyph::ascii('.'),
                    Ink::new(Role::Planet, 200 - 50 * k),
                );
            }
        }
        let sun_r = (outer * 0.07).max(0.3);
        draw::disc(canvas, cx, cy, sun_r, |d, _, _| {
            Some((
                draw::ramp(d, &SUN),
                Ink::new(Role::Sun, (255.0 * (1.0 - 0.5 * d)) as u8),
            ))
        });
        for (p, &phase) in PLANETS.iter().zip(&self.phases) {
            let (x, y) = geom::polar(cx, cy, outer * p.frac, phase);
            canvas.put(x, y, p.glyph, Ink::new(Role::Planet, 255));
        }
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
        let mut stage = Stage::new(build(), 1, size, GlyphMode::Ascii);
        for _ in 0..40 {
            for _ in 0..30 {
                stage.step(SIM_DT);
            }
            let frame = stage.render();
            let planets = frame
                .cells()
                .iter()
                .filter(|c| c.ink.role == Role::Planet && c.ink.level == 255);
            assert_eq!(planets.count(), 8, "all 8 planets visible");
        }
    }

    #[test]
    fn sol_a3_sun_core_on_center_cell() {
        let mut stage = Stage::new(build(), 1, Size::new(200, 50), GlyphMode::Ascii);
        let cell = *stage.render().get(100, 25).unwrap();
        assert_eq!((cell.ch, cell.ink.role), ('@', Role::Sun));
    }
}
