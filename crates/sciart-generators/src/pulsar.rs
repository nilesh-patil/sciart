//! A spinning neutron star with sweeping radiation cones (SPEC-002 `pulsar`).

use std::f64::consts::{PI, TAU};

use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;
use sciart_core::{draw, geom};

pub static META: Meta = Meta {
    name: "pulsar",
    category: Category::DeepCosmos,
    description: "A lighthouse neutron star: two sweeping beams, dipole field loops, flashes",
    title: "Pulsar · spinning neutron star",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Pulsar::default())
}

const SPIN_PERIOD: f64 = 2.5;
/// Cone half-angle in radians.
const CONE: f64 = 0.12;
/// Direction of the observer (screen up).
const VIEWER: f64 = -PI / 2.0;
const FLASH_WINDOW: f64 = 0.2;
const BEAM: [Glyph; 4] = [
    Glyph::ascii('.'),
    Glyph::ascii(':'),
    Glyph::ascii('='),
    Glyph::ascii('#'),
];
const CORE: [Glyph; 3] = [Glyph::ascii('@'), Glyph::ascii('#'), Glyph::ascii('o')];
const FIELD: Glyph = Glyph::new('·', '.');

/// Angles of the two beams at time `t`, in `[0, 2π)` (PUL-A1).
pub(crate) fn beam_angles(t: f64) -> [f64; 2] {
    let a = geom::wrap(TAU * t / SPIN_PERIOD, TAU);
    [a, geom::wrap(a + PI, TAU)]
}

/// Signed smallest difference `a − b` in `[−π, π)`.
fn angle_diff(a: f64, b: f64) -> f64 {
    geom::wrap_angle(a - b)
}

/// Whether a beam currently faces the viewer (PUL-A2).
pub(crate) fn flashing(t: f64) -> bool {
    beam_angles(t)
        .iter()
        .any(|&b| angle_diff(b, VIEWER).abs() < FLASH_WINDOW)
}

/// Point on the dipole field line `r = L sin²φ`, rotated so φ is measured from `axis` (PUL-A3).
///
/// `φ` is the magnetic colatitude: the loop leaves the star near one pole (`φ → 0`), peaks at
/// `r = L` on the magnetic equator and returns near the other (`φ → ±π`). Units are rows,
/// before the x stretch.
pub(crate) fn field_point(l: f64, phi: f64, axis: f64) -> (f64, f64) {
    let r = l * phi.sin().powi(2);
    let (s, c) = (phi + axis).sin_cos();
    (r * c, r * s)
}

#[derive(Debug, Default)]
pub struct Pulsar {
    t: f64,
    cx: f64,
    cy: f64,
    reach: f64,
    fit: f64,
}

impl Generator for Pulsar {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.cx = f64::from(size.width() / 2);
        self.cy = f64::from(size.height() / 2);
        self.reach = (f64::from(size.width()) / 2.0 / ASPECT).hypot(f64::from(size.height()) / 2.0);
        self.fit = draw::fit_radius(size, 1.0);
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let [axis, _] = beam_angles(self.t);
        let (cx, cy, reach) = (self.cx, self.cy, self.reach);

        // The field co-rotates with the magnetic axis, which is also the beam axis.
        for l in [0.35, 0.6, 0.85].map(|f| f * self.fit) {
            let mut phi = -PI + 0.05;
            while phi < PI - 0.05 {
                let (u, v) = field_point(l, phi, axis);
                if u.hypot(v) > 1.5 {
                    let (x, y) = ((cx + u * ASPECT).round(), (cy + v).round());
                    canvas.put(x as i32, y as i32, FIELD, Ink::new(Role::Dim, 80));
                }
                phi += 0.03;
            }
        }

        // Both cones lie on one axis, so one pass over the cells shades them: project each cell
        // onto the axis (|along| = distance out the nearer cone) and across it. One write per
        // cell means dim samples never overwrite bright ones and the x stretch leaves no gaps;
        // the cost is a few multiplies per cell and no trig.
        let (s, c) = axis.sin_cos();
        let tan_cone = CONE.tan();
        draw::field(canvas, |x, y| {
            let u = (f64::from(x) - cx) / ASPECT;
            let v = f64::from(y) - cy;
            let dist = (u * c + v * s).abs();
            if dist < 1.0 {
                return None;
            }
            let perp = (v * c - u * s).abs();
            // +0.5 row keeps the cone at least one cell wide near its apex.
            let across = 1.0 - perp / (dist * tan_cone + 0.5);
            let intensity = (1.0 - dist / reach) * across;
            (across > 0.0 && intensity > 0.02).then(|| {
                let level = (60.0 + 195.0 * intensity) as u8;
                (draw::ramp(intensity, &BEAM), Ink::new(Role::Wave, level))
            })
        });

        draw::disc(canvas, cx, cy, 1.2, |d, _, _| {
            Some((draw::ramp(d, &CORE), Ink::new(Role::Hot, 255)))
        });
        if flashing(self.t) {
            draw::orbit(
                canvas,
                cx,
                cy,
                2.4,
                Glyph::ascii('*'),
                Ink::new(Role::Accent, 255),
            );
            draw::orbit(canvas, cx, cy, 3.2, FIELD, Ink::new(Role::Accent, 150));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pul_a1_beams_opposite_and_rotating() {
        for t in [0.0, 0.4, 1.3, 7.9] {
            let [a, b] = beam_angles(t);
            assert!((angle_diff(b, a).abs() - PI).abs() < 1e-9);
            assert!((0.0..TAU).contains(&a) && (0.0..TAU).contains(&b));
        }
        let [a0, _] = beam_angles(0.0);
        let [a1, _] = beam_angles(0.1);
        assert!((angle_diff(a1, a0) - TAU / SPIN_PERIOD * 0.1).abs() < 1e-9);
    }

    #[test]
    fn pul_a2_two_flashes_per_rotation() {
        let steps = 10_000;
        let mut rising = 0;
        let mut prev = flashing(0.0);
        for i in 1..=steps {
            let now = flashing(SPIN_PERIOD * f64::from(i) / f64::from(steps));
            if now && !prev {
                rising += 1;
            }
            prev = now;
        }
        assert_eq!(rising, 2);
    }

    #[test]
    fn pul_a3_field_lines_are_dipoles() {
        let (l, axis) = (10.0, 0.7);
        for phi in [0.3, 1.0, 1.57, 2.5, -0.8, -2.0] {
            let (x, y) = field_point(l, phi, axis);
            let r = x.hypot(y);
            assert!((r - l * phi.sin().powi(2)).abs() < 1e-9, "phi={phi}");
            let local = angle_diff(y.atan2(x), axis);
            assert!((local - phi).abs() < 1e-9, "phi={phi} local={local}");
        }
    }

    #[test]
    fn pul_beam_cells_are_brightest_on_axis() {
        // The beam shader must put its peak on the axis, not wherever the last sample landed.
        let mut stage = Stage::new(build(), 1, Size::new(200, 50), GlyphMode::Ascii);
        let frame = stage.render();
        let peak = frame
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Wave)
            .map(|c| c.ink.level)
            .max();
        assert!(peak.is_some_and(|l| l > 200), "{peak:?}");
    }
}
