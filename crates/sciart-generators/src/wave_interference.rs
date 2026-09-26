//! Two coherent point sources and their interference pattern (SPEC-002 `wave_interference`).

use std::f64::consts::TAU;

use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "wave_interference",
    category: Category::PhysicsChemistry,
    description: "Two coherent sources whose ripples interfere into bright and dark bands",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(WaveInterference::default())
}

/// Angular frequency, rad/s.
const OMEGA: f64 = 3.0;
/// Source separation as a fraction of the height.
const SEPARATION: f64 = 0.35;
const RAMP: [Glyph; 6] = [
    Glyph::ascii('.'),
    Glyph::new('∘', '.'),
    Glyph::new('○', 'o'),
    Glyph::new('◎', 'O'),
    Glyph::new('●', '0'),
    Glyph::new('◉', '@'),
];
const SOURCE: Glyph = Glyph::new('✱', '*');
const TITLE: &str = " Two-Source Interference ";

/// Wavelength in rows for a screen `h` rows tall.
pub(crate) fn wavelength(h: u16) -> f64 {
    (f64::from(h) / 6.0).max(3.0)
}

/// Superposed displacement of two unit waves at distances `d1`, `d2` (WAVE-A1, WAVE-A2).
/// Reference form; `render` uses the equivalent precomputed `S cos ωt − C sin ωt`.
#[cfg(test)]
pub(crate) fn amplitude(d1: f64, d2: f64, k: f64, t: f64) -> f64 {
    (k * d1 - OMEGA * t).sin() + (k * d2 - OMEGA * t).sin()
}

#[derive(Debug, Default)]
pub struct WaveInterference {
    t: f64,
    width: usize,
    /// Sources in cell coordinates.
    sources: [(f64, f64); 2],
    /// Per cell: Σ sin(k dᵢ) and Σ cos(k dᵢ), so that
    /// `a(t) = S cos ωt − C sin ωt` needs no trig per cell.
    sin_sum: Vec<f64>,
    cos_sum: Vec<f64>,
    title: bool,
}

impl WaveInterference {
    /// Amplitude of cell `i` at the current time, from the precomputed tables.
    fn cell_amplitude(&self, i: usize, (s, c): (f64, f64)) -> f64 {
        self.sin_sum[i] * c - self.cos_sum[i] * s
    }
}

impl Generator for WaveInterference {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let (w, h) = (usize::from(size.width()), f64::from(size.height()));
        let k = TAU / wavelength(size.height());
        // Positions in rows (x divided by ASPECT) so distances are isotropic on screen.
        let sx = 0.25 * w as f64 / ASPECT;
        let ys = [
            h / 2.0 - SEPARATION * h / 2.0,
            h / 2.0 + SEPARATION * h / 2.0,
        ];
        self.sources = ys.map(|y| (sx * ASPECT, y));
        self.width = w;
        self.title = size.height() > 8;
        self.sin_sum.clear();
        self.cos_sum.clear();
        self.sin_sum.reserve(size.area());
        self.cos_sum.reserve(size.area());
        for y in 0..size.height() {
            let y = f64::from(y);
            for x in 0..w {
                let xr = x as f64 / ASPECT;
                let (mut s, mut c) = (0.0, 0.0);
                for &ys in &ys {
                    let (si, ci) = (k * (xr - sx).hypot(y - ys)).sin_cos();
                    s += si;
                    c += ci;
                }
                self.sin_sum.push(s);
                self.cos_sum.push(c);
            }
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let phase = (OMEGA * self.t).sin_cos();
        for i in 0..self.sin_sum.len() {
            let v = (self.cell_amplitude(i, phase) + 2.0) / 4.0;
            let level = (35.0 + 220.0 * v.clamp(0.0, 1.0)) as u8;
            let (x, y) = ((i % self.width) as i32, (i / self.width) as i32);
            canvas.put(x, y, draw::ramp(v, &RAMP), Ink::new(Role::Wave, level));
        }
        for (x, y) in self.sources {
            canvas.put(
                x.round() as i32,
                y.round() as i32,
                SOURCE,
                Ink::new(Role::Accent, 255),
            );
        }
        if self.title {
            draw::text_centered(canvas, 0, TITLE, Ink::new(Role::Text, 220));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: f64 = TAU / 5.0;

    #[test]
    fn wave_a1_bisector_amplitude_is_twice_a_single_wave() {
        for d in [0.0, 1.3, 7.7, 25.0] {
            for t in [0.0, 0.2, 1.1, 9.4] {
                let expected = 2.0 * (K * d - OMEGA * t).sin();
                assert!((amplitude(d, d, K, t) - expected).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn wave_a1_bisector_row_of_the_scene_has_equal_path_lengths() {
        let (w, h) = (200u16, 50u16);
        let size = Size::new(w, h);
        let mut g = WaveInterference::default();
        g.resize(size, &mut Ctx::new(1, size));
        let k = TAU / wavelength(h);
        let (sx, y_mid) = (g.sources[0].0 / ASPECT, f64::from(h) / 2.0);
        for t in [0.0, 0.37, 2.0] {
            g.t = t;
            let phase = (OMEGA * t).sin_cos();
            for x in [0usize, 13, 50, 199] {
                let d = (x as f64 / ASPECT - sx).hypot(g.sources[0].1 - y_mid);
                let i = usize::from(h / 2) * usize::from(w) + x;
                let expected = 2.0 * (k * d - OMEGA * t).sin();
                assert!(
                    (g.cell_amplitude(i, phase) - expected).abs() < 1e-9,
                    "x={x} t={t}"
                );
            }
        }
    }

    #[test]
    fn wave_a2_half_wavelength_path_difference_cancels_at_all_times() {
        let lambda = TAU / K;
        for d in [2.0, 11.5, 40.0] {
            for i in 0..200 {
                let t = f64::from(i) * 0.037;
                assert!(amplitude(d, d + lambda / 2.0, K, t).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn wave_tables_match_the_analytic_amplitude() {
        let size = Size::new(90, 30);
        let mut g = WaveInterference::default();
        g.resize(size, &mut Ctx::new(1, size));
        let k = TAU / wavelength(30);
        let t = 1.234;
        let phase = (OMEGA * t).sin_cos();
        for (x, y) in [(0, 0), (22, 10), (89, 29), (45, 15)] {
            let p = (f64::from(x) / ASPECT, f64::from(y));
            let d = |s: (f64, f64)| (p.0 - s.0 / ASPECT).hypot(p.1 - s.1);
            let expected = amplitude(d(g.sources[0]), d(g.sources[1]), k, t);
            let i = y as usize * 90 + x as usize;
            assert!((g.cell_amplitude(i, phase) - expected).abs() < 1e-9);
        }
    }
}
