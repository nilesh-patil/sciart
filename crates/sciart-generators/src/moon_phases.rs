//! The Moon through a full lunation (SPEC-002 `moon_phases`).
//!
//! The disc is an orthographic view of a Lambertian sphere: each cell gets the surface
//! normal `n = (nx, ny, √(1−nx²−ny²))` (z towards the viewer) and is lit by `n·L`, with
//! the Sun direction `L` swinging round the Moon once per synodic month.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{draw, geom};

pub static META: Meta = Meta {
    name: "moon_phases",
    category: Category::EarthSky,
    description: "The Moon through a full lunation, with craters and the phase name",
    title: "Moon phases · sunlit half",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(MoonPhases::default())
}

/// One synodic month in simulated seconds.
const SYNODIC: f64 = 30.0;
const CRATERS: usize = 18;
const MARIA: usize = 5;
const STARS: usize = 140;
/// Normalized disc radius beyond which the unlit side still shows a faint limb.
const LIMB: f64 = 0.95;

const NAMES: [&str; 8] = [
    "New Moon",
    "Waxing Crescent",
    "First Quarter",
    "Waxing Gibbous",
    "Full Moon",
    "Waning Gibbous",
    "Last Quarter",
    "Waning Crescent",
];

const LIT: [Glyph; 6] = [
    Glyph::new('·', '.'),
    Glyph::ascii(':'),
    Glyph::new('░', '+'),
    Glyph::new('▒', '*'),
    Glyph::new('▓', '#'),
    Glyph::new('█', '@'),
];
const LIMB_GLYPH: Glyph = Glyph::new('·', '.');
const STAR: Glyph = Glyph::new('·', '.');

/// Phase `p ∈ [0, 1)` after `t` simulated seconds (0 = new, ½ = full).
pub(crate) fn phase_at(t: f64) -> f64 {
    geom::wrap(t / SYNODIC, 1.0)
}

/// Unit vector towards the Sun: `(sin 2πp, 0, −cos 2πp)` — behind the Moon at new moon,
/// behind the viewer at full moon, from screen-right at first quarter.
pub(crate) fn sun_direction(phase: f64) -> [f64; 3] {
    let a = TAU * phase;
    [a.sin_det(), 0.0, -a.cos_det()]
}

/// Lambert term `n·L` for the visible-hemisphere point `(nx, ny)`; negative ⇒ night side
/// (MOON-A1). Points outside the unit disc are projected onto the limb. The renderer calls
/// [`lambert`] directly with `L` hoisted out of the per-cell loop.
#[cfg(test)]
pub(crate) fn illumination(nx: f64, ny: f64, phase: f64) -> f64 {
    lambert(nx, ny, sun_direction(phase))
}

/// `n·L` for a precomputed light direction, so the per-cell shader needs no trig.
fn lambert(nx: f64, ny: f64, [lx, ly, lz]: [f64; 3]) -> f64 {
    let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
    nx * lx + ny * ly + nz * lz
}

/// Fraction of the visible disc that is lit, `(1 − cos 2πp)/2` (MOON-A3).
pub(crate) fn illuminated_fraction(phase: f64) -> f64 {
    (1.0 - (TAU * phase).cos_det()) / 2.0
}

/// Traditional name of the phase; each of the eight names owns a 1/8 slice centred on its
/// nominal phase (MOON-A2).
pub(crate) fn phase_name(phase: f64) -> &'static str {
    let p = geom::wrap(phase, 1.0);
    NAMES[((p + 1.0 / 16.0) * 8.0) as usize % 8]
}

/// A circular albedo feature on the disc, in normalized disc coordinates.
#[derive(Debug, Clone, Copy)]
struct Spot {
    x: f64,
    y: f64,
    r: f64,
    /// Albedo at the centre of the spot (1 = no darkening).
    depth: f64,
}

impl Spot {
    fn albedo(&self, nx: f64, ny: f64) -> f64 {
        let q = ((nx - self.x).powi(2) + (ny - self.y).powi(2)) / (self.r * self.r);
        if q >= 1.0 {
            1.0
        } else {
            // Bowl profile: darkest at the centre, blending into the surroundings at the rim.
            self.depth + (1.0 - self.depth) * q
        }
    }
}

#[derive(Debug, Default)]
pub struct MoonPhases {
    /// Scene time, seeded with a random start so each run opens on a different phase.
    t: f64,
    cx: f64,
    cy: f64,
    r: f64,
    label_y: Option<i32>,
    w: f64,
    h: f64,
    spots: Vec<Spot>,
    /// (u, v, twinkle phase, twinkle rate) with `u, v ∈ [0, 1)`.
    stars: Vec<(f64, f64, f64, f64)>,
}

impl MoonPhases {
    fn albedo(&self, nx: f64, ny: f64) -> f64 {
        self.spots.iter().map(|s| s.albedo(nx, ny)).product()
    }
}

impl Generator for MoonPhases {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (size.width(), size.height());
        // Two rows are kept below the disc once there is room for a recognisable one: the
        // label's, at most h − 2, and the bottom row, which belongs to the title (GEN-R13).
        let disc_h = if h >= 8 { h - 2 } else { h };
        self.cx = f64::from(w / 2);
        self.cy = f64::from(disc_h / 2);
        self.r = draw::fit_radius(Size::new(w, disc_h), 0.9);
        self.label_y =
            (h >= 8).then(|| ((self.cy + self.r).ceil() as i32 + 1).min(i32::from(h) - 2));
        self.w = f64::from(w);
        self.h = f64::from(h);
        if self.spots.is_empty() {
            // Features live in normalized disc coordinates, so they survive resizes.
            self.t = ctx.rng.random_range(0.0..SYNODIC);
            for i in 0..MARIA + CRATERS {
                let mare = i < MARIA;
                let (r, depth) = if mare {
                    (
                        ctx.rng.random_range(0.16..0.32),
                        ctx.rng.random_range(0.62..0.78),
                    )
                } else {
                    (
                        ctx.rng.random_range(0.04..0.12),
                        ctx.rng.random_range(0.35..0.6),
                    )
                };
                // Uniform over the disc (sqrt for area), kept a little inside the limb.
                let rho = 0.85 * ctx.rng.random_range(0.0f64..1.0).sqrt();
                let ang = ctx.rng.random_range(0.0..TAU);
                self.spots.push(Spot {
                    x: rho * ang.cos_det(),
                    y: rho * ang.sin_det(),
                    r,
                    depth,
                });
            }
            self.stars = (0..STARS)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..TAU),
                        ctx.rng.random_range(1.0..3.0),
                    )
                })
                .collect();
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let phase = phase_at(self.t);
        let (cx, cy, r) = (self.cx, self.cy, self.r);

        for &(u, v, ph, rate) in &self.stars {
            let (x, y) = ((u * self.w).floor(), (v * self.h).floor());
            let (dx, dy) = ((x - cx) / (r * geom::ASPECT), (y - cy) / r);
            if dx * dx + dy * dy <= 1.1 {
                continue;
            }
            let tw = 0.5 + 0.5 * (ph + rate * self.t).sin_det();
            canvas.put(
                x as i32,
                y as i32,
                STAR,
                Ink::new(Role::Dim, (40.0 + 90.0 * tw) as u8),
            );
        }

        let light = sun_direction(phase);
        draw::disc(canvas, cx, cy, r, |d, nx, ny| {
            let lit = lambert(nx, ny, light);
            if lit > 0.0 {
                let v = lit * self.albedo(nx, ny);
                // Gamma < 1 lifts the thin terminator so crescents stay legible.
                let g = v.powf(0.75);
                Some((
                    draw::ramp(g, &LIT),
                    Ink::new(Role::Star, (70.0 + 185.0 * g) as u8),
                ))
            } else if d >= LIMB {
                Some((LIMB_GLYPH, Ink::new(Role::Dim, 70)))
            } else {
                None
            }
        });

        if let Some(y) = self.label_y {
            // "{name}  {pct}%" drawn in pieces so render never allocates (GEN-R11).
            let name = phase_name(phase);
            let pct = ((100.0 * illuminated_fraction(phase)).round() as u32).min(100);
            let digits = if pct >= 100 {
                3
            } else if pct >= 10 {
                2
            } else {
                1
            };
            let mut buf = [b'%'; 4];
            let mut n = pct;
            for slot in buf[..digits].iter_mut().rev() {
                *slot = b'0' + (n % 10) as u8;
                n /= 10;
            }
            let pct_text = std::str::from_utf8(&buf[..=digits]).unwrap_or("");
            let len = (name.len() + 2 + pct_text.len()) as i32;
            let x = (i32::from(canvas.size().width()) - len) / 2;
            let ink = Ink::new(Role::Text, 220);
            canvas.text(x, y, name, ink);
            canvas.text(x + name.len() as i32, y, "  ", ink);
            canvas.text(x + name.len() as i32 + 2, y, pct_text, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interior sample points of the visible hemisphere, strictly inside the limb.
    fn interior_grid() -> impl Iterator<Item = (f64, f64)> {
        (-20..=20)
            .flat_map(|i| (-20..=20).map(move |j| (f64::from(i) / 20.5, f64::from(j) / 20.5)))
            .filter(|&(x, y)| x * x + y * y < 0.999)
    }

    #[test]
    fn moon_a1_new_moon_dark_full_moon_lit_first_quarter_lights_right_half() {
        let mut n = 0;
        for (nx, ny) in interior_grid() {
            n += 1;
            assert!(
                illumination(nx, ny, 0.0) <= 1e-12,
                "new moon lit at ({nx}, {ny})"
            );
            assert!(
                illumination(nx, ny, 0.5) > 0.0,
                "full moon dark at ({nx}, {ny})"
            );
            let lit = illumination(nx, ny, 0.25) > 0.0;
            assert_eq!(lit, nx > 0.0, "first quarter at ({nx}, {ny})");
        }
        assert!(n > 1000, "grid too sparse: {n}");
    }

    #[test]
    fn moon_a1_illumination_is_lambertian() {
        // Full moon is a head-on light: n·L = nz exactly.
        let (nx, ny) = (0.3, -0.4);
        let nz = (1.0f64 - 0.09 - 0.16).sqrt();
        assert!((illumination(nx, ny, 0.5) - nz).abs() < 1e-12);
        // |n·L| ≤ 1 everywhere because both vectors are unit.
        for (nx, ny) in interior_grid() {
            for k in 0..16 {
                assert!(illumination(nx, ny, f64::from(k) / 16.0).abs() <= 1.0 + 1e-12);
            }
        }
    }

    #[test]
    fn moon_a2_phase_names() {
        assert_eq!(phase_name(0.0), "New Moon");
        assert_eq!(phase_name(0.25), "First Quarter");
        assert_eq!(phase_name(0.5), "Full Moon");
        assert_eq!(phase_name(0.75), "Last Quarter");
        assert_eq!(phase_name(0.97), "New Moon");
        assert_eq!(phase_name(0.125), "Waxing Crescent");
        assert_eq!(phase_name(0.375), "Waxing Gibbous");
        assert_eq!(phase_name(0.625), "Waning Gibbous");
        assert_eq!(phase_name(0.875), "Waning Crescent");
    }

    #[test]
    fn moon_a3_illuminated_fraction() {
        for (p, f) in [(0.0, 0.0), (0.25, 0.5), (0.5, 1.0), (0.75, 0.5)] {
            assert!((illuminated_fraction(p) - f).abs() < 1e-12, "p={p}");
        }
        // Cross-check against the lit area of the disc by quadrature (2000² midpoint grid):
        // the lit fraction of a Lambert sphere's projected disc is (1 − cos 2πp)/2.
        let n = 2000;
        for p in [0.1, 0.3, 0.6] {
            let (mut lit, mut all) = (0u32, 0u32);
            for i in 0..n {
                for j in 0..n {
                    let x = (f64::from(i) + 0.5) / f64::from(n) * 2.0 - 1.0;
                    let y = (f64::from(j) + 0.5) / f64::from(n) * 2.0 - 1.0;
                    if x * x + y * y < 1.0 {
                        all += 1;
                        lit += u32::from(illumination(x, y, p) > 0.0);
                    }
                }
            }
            let q = f64::from(lit) / f64::from(all);
            assert!((q - illuminated_fraction(p)).abs() < 2e-3, "p={p}: {q}");
        }
    }

    fn stage_at(t: f64) -> Canvas {
        let size = Size::new(120, 40);
        let mut moon = MoonPhases::default();
        let mut ctx = Ctx::new(3, size);
        moon.resize(size, &mut ctx);
        moon.t = t;
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        moon.render(&mut canvas);
        canvas
    }

    #[test]
    fn moon_render_new_moon_has_no_lit_cells_and_full_moon_has_many() {
        let lit = |c: &Canvas| {
            c.cells()
                .iter()
                .filter(|c| c.ink.role == Role::Star)
                .count()
        };
        assert_eq!(lit(&stage_at(0.0)), 0);
        assert!(lit(&stage_at(15.0)) > 500);
        let full = stage_at(15.0).to_lines().join("\n");
        assert!(full.contains("Full Moon  100%"), "{full}");
    }

    #[test]
    fn moon_gen_r13_label_sits_between_the_disc_and_the_title_row() {
        for (w, h) in [(40, 12), (80, 24), (30, 8), (200, 50), (60, 9)] {
            let size = Size::new(w, h);
            let mut moon = MoonPhases::default();
            moon.resize(size, &mut Ctx::new(1, size));
            let y = moon.label_y.expect("label shown");
            assert!(y <= i32::from(h) - 2, "{w}x{h}: label on row {y}");
            assert!(
                f64::from(y) > moon.cy + moon.r,
                "{w}x{h}: label on the disc"
            );
        }
    }
}
