//! A planet transiting a limb-darkened star (SPEC-002 `exoplanet_transit`).
//!
//! The light curve uses the exact circle–circle overlap area (a uniform-disc transit
//! model); limb darkening only shades the star, it does not enter the flux.

use std::f64::consts::PI;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::stellar;
use sciart_core::{draw, geom};

pub static META: Meta = Meta {
    name: "exoplanet_transit",
    category: Category::SunPlanets,
    description: "A planet transiting a limb-darkened star while its light curve dips",
    title: "Planet transit · starlight dips",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ExoplanetTransit::default())
}

/// Seconds for one pass across `u ∈ [−2.2 Rs, 2.2 Rs]`.
const PERIOD: f64 = 10.0;
const SWEEP: f64 = 2.2;
/// Impact parameter as a fraction of the stellar radius.
const IMPACT: f64 = 0.3;
/// Linear limb-darkening coefficient: `I(μ) = 1 − U(1 − μ)`.
const U: f64 = 0.6;
/// Fraction of the width given to the light-curve panel.
const PANEL_FRAC: f64 = 0.38;
const PANEL_MIN: (u16, u16) = (12, 6);
const STARS: usize = 90;

const STAR_RAMP: [Glyph; 5] = [
    Glyph::new('·', '.'),
    Glyph::new('░', ':'),
    Glyph::new('▒', '+'),
    Glyph::new('▓', '#'),
    Glyph::new('█', '@'),
];
const PLANET: Glyph = Glyph::new('●', 'o');
const CURVE: Glyph = Glyph::new('•', '*');
const CURVE_FILL: Glyph = Glyph::new('│', '|');
const MARKER: Glyph = Glyph::new('◆', '@');
const BG_STAR: Glyph = Glyph::new('·', '.');

/// Exact area of intersection of two circles of radii `r1`, `r2` whose centres are `d`
/// apart (TRAN-A3). Handles disjoint, contained and partial overlap; `acos` arguments are
/// clamped so rounding at tangency cannot produce NaN.
pub(crate) fn overlap_area(d: f64, r1: f64, r2: f64) -> f64 {
    let (r1, r2) = (r1.max(0.0), r2.max(0.0));
    let d = d.abs();
    let (small, big) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    if small == 0.0 || d >= big + small {
        return 0.0;
    }
    if d <= big - small {
        return PI * small * small;
    }
    // Two circular segments minus the kite between the centres and the chord ends.
    let c_small = ((d * d + small * small - big * big) / (2.0 * d * small)).clamp(-1.0, 1.0);
    let c_big = ((d * d + big * big - small * small) / (2.0 * d * big)).clamp(-1.0, 1.0);
    let kite = 0.5
        * ((-d + small + big) * (d + small - big) * (d - small + big) * (d + small + big))
            .max(0.0)
            .sqrt();
    (small * small * c_small.acos() + big * big * c_big.acos() - kite)
        .clamp(0.0, PI * small * small)
}

/// Relative stellar flux with the planet's centre `d` from the star's (TRAN-A1, TRAN-A2).
pub(crate) fn flux(d: f64, rs: f64, rp: f64) -> f64 {
    if rs <= 0.0 {
        return 1.0;
    }
    1.0 - overlap_area(d, rs, rp) / (PI * rs * rs)
}

/// Limb-darkened intensity for a point at normalized radius `r ∈ [0, 1]` on the disc:
/// the shared law (CORE-R93) with the linear coefficients `(U, 0)`.
pub(crate) fn limb_intensity(r: f64) -> f64 {
    stellar::limb_darkening((1.0 - r * r).max(0.0).sqrt(), (U, 0.0))
}

/// Planet offset from the star centre, in rows, at orbital phase `s ∈ [0, 1)`.
pub(crate) fn planet_offset(s: f64, rs: f64) -> (f64, f64) {
    (rs * SWEEP * (2.0 * s - 1.0), rs * IMPACT)
}

/// Light-curve panel layout, in cells.
#[derive(Debug, Clone, Copy)]
struct Panel {
    x0: i32,
    x1: i32,
    top: i32,
    bottom: i32,
}

#[derive(Debug, Default)]
pub struct ExoplanetTransit {
    t: f64,
    cx: f64,
    cy: f64,
    rs: f64,
    rp: f64,
    panel: Option<Panel>,
    /// Plot row of the light curve for each panel column.
    curve: Vec<i32>,
    w: f64,
    star_w: f64,
    h: f64,
    /// (u, v, level) with `u, v ∈ [0, 1)`.
    stars: Vec<(f64, f64, u8)>,
}

impl ExoplanetTransit {
    fn phase(&self) -> f64 {
        geom::wrap(self.t / PERIOD, 1.0)
    }

    fn flux_row(&self, panel: Panel, f: f64) -> i32 {
        // The deepest point of the transit is at d = b; pad the range by 15 % on each side.
        let fmin = flux(self.rs * IMPACT, self.rs, self.rp);
        let depth = (1.0 - fmin).max(1e-9);
        let (hi, lo) = (1.0 + 0.15 * depth, fmin - 0.15 * depth);
        let frac = ((hi - f) / (hi - lo)).clamp(0.0, 1.0);
        panel.top + (frac * f64::from(panel.bottom - panel.top)).round() as i32
    }
}

impl Generator for ExoplanetTransit {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (size.width(), size.height());
        let pw = (f64::from(w) * PANEL_FRAC).round() as u16;
        let has_panel = pw >= PANEL_MIN.0 && h >= PANEL_MIN.1;
        let star_w = if has_panel { w - pw } else { w };
        self.cx = f64::from(star_w / 2);
        self.cy = f64::from(h / 2);
        self.rs = draw::fit_radius(Size::new(star_w, h), 0.8);
        self.rp = (0.12 * self.rs).max(0.5);
        self.w = f64::from(w);
        self.star_w = f64::from(star_w);
        self.h = f64::from(h);
        self.panel = has_panel.then(|| Panel {
            x0: i32::from(star_w) + 1,
            x1: i32::from(w) - 2,
            top: 2,
            bottom: i32::from(h) - 3,
        });
        self.curve.clear();
        if let Some(p) = self.panel {
            let n = (p.x1 - p.x0).max(1);
            for i in 0..=n {
                let (u, b) = planet_offset(f64::from(i) / f64::from(n), self.rs);
                let f = flux(u.hypot(b), self.rs, self.rp);
                self.curve.push(self.flux_row(p, f));
            }
        }
        if self.stars.is_empty() {
            self.stars = (0..STARS)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(30..110),
                    )
                })
                .collect();
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let (cx, cy, rs, rp) = (self.cx, self.cy, self.rs, self.rp);
        let (du, db) = planet_offset(self.phase(), rs);
        let (px, py) = (cx + du * geom::ASPECT, cy + db);

        for &(u, v, level) in &self.stars {
            let (x, y) = ((u * self.star_w).floor(), (v * self.h).floor());
            canvas.put(x as i32, y as i32, BG_STAR, Ink::new(Role::Dim, level));
        }

        // Star, with the planet's silhouette cut out as blank cells.
        draw::disc(canvas, cx, cy, rs, |d, nx, ny| {
            let (x, y) = (nx * rs - du, ny * rs - db);
            if x * x + y * y <= rp * rp {
                return Some((Glyph::ascii(' '), Ink::BLANK));
            }
            let i = limb_intensity(d);
            // Squaring stretches the subtle 1 → 0.4 brightness fall-off across the glyph ramp.
            let v = ((i - (1.0 - U)) / U).powi(2);
            Some((
                draw::ramp(v, &STAR_RAMP),
                Ink::new(Role::Sun, (255.0 * i) as u8),
            ))
        });
        // Off the disc the planet is seen in scattered starlight.
        draw::disc(canvas, px, py, rp, |_, nx, ny| {
            let (x, y) = (du + nx * rp, db + ny * rp);
            (x * x + y * y > rs * rs).then_some((PLANET, Ink::new(Role::Planet, 200)))
        });

        let Some(p) = self.panel else { return };
        let axis = Ink::new(Role::Structure, 150);
        for y in p.top - 1..=p.bottom + 1 {
            canvas.put(p.x0 - 1, y, Glyph::new('│', '|'), axis);
        }
        for x in p.x0..=p.x1 + 1 {
            canvas.put(x, p.bottom + 1, Glyph::new('─', '-'), axis);
        }
        canvas.put(p.x0 - 1, p.bottom + 1, Glyph::new('└', '+'), axis);
        canvas.text(p.x0, 0, "relative flux", Ink::new(Role::Text, 200));
        canvas.text(
            p.x1 - 3,
            p.bottom + 2,
            "time",
            Ink::new(Role::Structure, 150),
        );

        let curve = Ink::new(Role::Accent, 150);
        let mut prev: Option<i32> = None;
        for (i, &row) in self.curve.iter().enumerate() {
            let x = p.x0 + i as i32;
            if let Some(pr) = prev {
                // Fill steep ingress/egress so the curve reads as a connected line.
                for y in pr.min(row) + 1..pr.max(row) {
                    canvas.put(x, y, CURVE_FILL, curve);
                }
            }
            canvas.put(x, row, CURVE, curve);
            prev = Some(row);
        }
        let n = self.curve.len().saturating_sub(1);
        let mi = (self.phase() * n as f64).round() as usize;
        if mi < self.curve.len() {
            let f = flux(du.hypot(db), rs, rp);
            canvas.put(
                p.x0 + mi as i32,
                self.flux_row(p, f),
                MARKER,
                Ink::new(Role::Accent, 255),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tran_a1_flux_is_one_when_disks_do_not_touch() {
        for (rs, rp) in [(10.0, 1.2), (3.0, 0.5), (1.0, 1.0), (1.0, 4.0)] {
            for k in 0..50 {
                let d = rs + rp + f64::from(k) * 0.37;
                assert_eq!(flux(d, rs, rp), 1.0, "rs={rs} rp={rp} d={d}");
                assert_eq!(flux(-d, rs, rp), 1.0);
            }
        }
    }

    #[test]
    fn tran_a2_full_transit_depth_is_radius_ratio_squared() {
        for (rs, rp) in [(10.0, 1.2), (3.0, 0.5), (24.8, 2.976)] {
            let expected = 1.0 - (rp / rs) * (rp / rs);
            for k in 0..=20 {
                let d = (rs - rp) * f64::from(k) / 20.0;
                assert!(
                    (flux(d, rs, rp) - expected).abs() < 1e-12,
                    "rs={rs} rp={rp} d={d}"
                );
            }
        }
    }

    #[test]
    fn tran_a3_overlap_known_value_and_edge_continuity() {
        // Two unit circles one radius apart overlap in a lens of area 2π/3 − √3/2.
        let lens = 2.0 * PI / 3.0 - 3f64.sqrt() / 2.0;
        assert!((overlap_area(1.0, 1.0, 1.0) - lens).abs() < 1e-12);
        // Continuous at the contained and tangent limits.
        let (r1, r2) = (3.0, 1.0);
        assert!((overlap_area(2.0 + 1e-9, r1, r2) - PI).abs() < 1e-6);
        assert!(overlap_area(4.0 - 1e-9, r1, r2) < 1e-6);
        // Degenerate radii are harmless.
        assert_eq!(overlap_area(0.5, 0.0, 1.0), 0.0);
        assert!(overlap_area(0.0, 1.0, 1.0).is_finite());
    }

    #[test]
    fn tran_a3_overlap_symmetric_in_radii_and_non_increasing_in_d() {
        let radii: [f64; 6] = [0.3, 0.5, 1.0, 1.7, 3.0, 10.0];
        for &r1 in &radii {
            for &r2 in &radii {
                let scale = PI * r1.min(r2).powi(2);
                let mut last = f64::INFINITY;
                for k in 0..=2000 {
                    let d = (r1 + r2 + 0.5) * f64::from(k) / 2000.0;
                    let a = overlap_area(d, r1, r2);
                    let b = overlap_area(d, r2, r1);
                    assert!(
                        a.is_finite() && a >= 0.0 && a <= scale * (1.0 + 1e-15),
                        "range d={d}"
                    );
                    assert!(
                        (a - b).abs() <= 1e-12 * scale.max(1.0),
                        "asym r1={r1} r2={r2} d={d}"
                    );
                    assert!(
                        a <= last + 1e-12 * scale.max(1.0),
                        "increase r1={r1} r2={r2} d={d}"
                    );
                    last = a;
                }
            }
        }
    }

    #[test]
    fn tran_limb_darkening_law() {
        assert!((limb_intensity(0.0) - 1.0).abs() < 1e-12);
        assert!((limb_intensity(1.0) - (1.0 - U)).abs() < 1e-12);
        let steps: Vec<f64> = (0..=10)
            .map(|k| limb_intensity(f64::from(k) / 10.0))
            .collect();
        assert!(steps.windows(2).all(|w| w[0] > w[1]));
    }

    fn frame(size: Size, t: f64) -> Canvas {
        let mut g = ExoplanetTransit::default();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        g.t = t;
        let mut c = Canvas::new(size, GlyphMode::Ascii);
        g.render(&mut c);
        c
    }

    #[test]
    fn tran_silhouette_is_blank_on_disk_and_panel_needs_room() {
        let size = Size::new(200, 50);
        // Mid-transit: the planet sits at (cx, cy + b).
        let c = frame(size, PERIOD / 2.0);
        let mut g = ExoplanetTransit::default();
        g.resize(size, &mut Ctx::new(5, size));
        let (x, y) = (g.cx.round() as i32, (g.cy + g.rs * IMPACT).round() as i32);
        assert_eq!(c.get(x, y).unwrap().ch, ' ');
        assert_eq!(c.get(x, g.cy as i32).unwrap().ink.role, Role::Sun);
        let accents = |c: &Canvas| {
            c.cells()
                .iter()
                .filter(|c| c.ink.role == Role::Accent)
                .count()
        };
        assert!(accents(&c) > 20);
        assert_eq!(accents(&frame(Size::new(20, 5), 1.0)), 0);
        assert_eq!(accents(&frame(Size::new(30, 30), 1.0)), 0);
    }
}
