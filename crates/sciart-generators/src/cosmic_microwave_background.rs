//! A Planck-style oval sky map of CMB temperature anisotropies (SPEC-002
//! `cosmic_microwave_background`).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "cosmic_microwave_background",
    category: Category::Astronomy,
    description: "A shimmering Planck-style map of the CMB anisotropies",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CosmicMicrowaveBackground::default())
}

const MODES: usize = 6;
/// Fraction of the screen (per axis) covered by the sky oval.
const OVAL: f64 = 0.94;
const RAMP: [Glyph; 4] = [
    Glyph::new('░', '.'),
    Glyph::new('▒', ':'),
    Glyph::new('▓', '*'),
    Glyph::new('█', '#'),
];
const CAPTION: &str = "CMB  T=2.725K  dT~1e-5";

/// One plane-wave mode `a sin(k·(x, y) + φ + ωt)`; `k` in rad/row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mode {
    kx: f64,
    ky: f64,
    phase: f64,
    omega: f64,
    amp: f64,
}

impl Mode {
    fn random(rng: &mut impl Rng) -> Self {
        let wavelength = rng.random_range(3.0..=20.0);
        let dir: f64 = rng.random_range(0.0..TAU);
        let k = TAU / wavelength;
        Self {
            kx: k * dir.cos(),
            ky: k * dir.sin(),
            phase: rng.random_range(0.0..TAU),
            omega: rng.random_range(0.2..=0.8),
            amp: rng.random_range(0.5..=1.0),
        }
    }
}

/// Normalized temperature contrast at `(x_rows, y)` and time `t` (CMB-A1). Reference form;
/// `render` evaluates the same sum separably (checked by `cmb_separable_render_matches_field_value`).
#[cfg(test)]
pub(crate) fn field_value(modes: &[Mode], x_rows: f64, y: f64, t: f64) -> f64 {
    let norm: f64 = modes.iter().map(|m| m.amp).sum();
    if norm <= 0.0 {
        return 0.0;
    }
    modes
        .iter()
        .map(|m| m.amp * (m.kx * x_rows + m.ky * y + m.phase + m.omega * t).sin())
        .sum::<f64>()
        / norm
}

/// Largest possible change of the field between horizontally adjacent cells (CMB-A2):
/// `|∂f/∂x_rows| ≤ Σ aᵢ|kᵢ| / Σ aᵢ`, and adjacent cells are `1/ASPECT` rows apart.
#[cfg(test)]
pub(crate) fn smoothness_bound(modes: &[Mode]) -> f64 {
    let norm: f64 = modes.iter().map(|m| m.amp).sum();
    modes.iter().map(|m| m.amp * m.kx.hypot(m.ky)).sum::<f64>() / (ASPECT * norm)
}

#[derive(Debug, Default)]
pub struct CosmicMicrowaveBackground {
    t: f64,
    modes: Vec<Mode>,
    /// Per column, per mode: `sin(kx·x)` and `cos(kx·x)` (separable evaluation, so a frame
    /// costs O(h·modes) trig calls instead of O(w·h·modes)).
    col_sin: Vec<[f64; MODES]>,
    col_cos: Vec<[f64; MODES]>,
    /// Weights `aᵢ / Σa`.
    weights: [f64; MODES],
    /// |f| that maps to full brightness (≈ 2.5 σ of the field).
    full_scale: f64,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    height: u16,
    caption: bool,
}

impl CosmicMicrowaveBackground {
    /// `(sin, cos)` of `ky·y + φ + ωt` for every mode on row `y`.
    fn row_phases(&self, y: f64) -> [(f64, f64); MODES] {
        let mut out = [(0.0, 1.0); MODES];
        for (o, m) in out.iter_mut().zip(&self.modes) {
            *o = (m.ky * y + m.phase + m.omega * self.t).sin_cos();
        }
        out
    }

    /// Field at column `x` given that row's phases: sin(A + B) = sin A cos B + cos A sin B.
    fn cell_value(&self, x: usize, row: &[(f64, f64); MODES]) -> f64 {
        let (s, c) = (&self.col_sin[x], &self.col_cos[x]);
        (0..MODES)
            .map(|i| self.weights[i] * (s[i] * row[i].1 + c[i] * row[i].0))
            .sum()
    }
}

impl Generator for CosmicMicrowaveBackground {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.modes.is_empty() {
            // The sky pattern survives resizes; only the viewport changes.
            self.modes = (0..MODES).map(|_| Mode::random(&mut ctx.rng)).collect();
            let norm: f64 = self.modes.iter().map(|m| m.amp).sum();
            for (w, m) in self.weights.iter_mut().zip(&self.modes) {
                *w = m.amp / norm;
            }
            // Random phases make f a sum of independent sinusoids: σ² = Σ wᵢ² / 2.
            let sigma = (self.weights.iter().map(|w| w * w).sum::<f64>() / 2.0).sqrt();
            self.full_scale = (2.5 * sigma).min(1.0);
        }
        let w = usize::from(size.width());
        self.col_sin.clear();
        self.col_cos.clear();
        for x in 0..w {
            let xr = x as f64 / ASPECT;
            let (mut s, mut c) = ([0.0; MODES], [0.0; MODES]);
            for (i, m) in self.modes.iter().enumerate() {
                (s[i], c[i]) = (m.kx * xr).sin_cos();
            }
            self.col_sin.push(s);
            self.col_cos.push(c);
        }
        self.height = size.height();
        self.caption = size.height() > 6;
        let sky_rows = f64::from(size.height()) - if self.caption { 1.0 } else { 0.0 };
        self.cx = (w as f64 - 1.0) / 2.0;
        self.cy = (sky_rows - 1.0) / 2.0;
        self.rx = (OVAL * w as f64 / 2.0).max(0.5);
        self.ry = (OVAL * sky_rows / 2.0).max(0.5);
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let w = self.col_sin.len();
        if w == 0 {
            return;
        }
        for y in 0..self.height {
            let dy = (f64::from(y) - self.cy) / self.ry;
            if dy.abs() > 1.0 {
                continue;
            }
            // Only the cells inside the oval; the corners stay black like a Mollweide map.
            let half = self.rx * (1.0 - dy * dy).sqrt();
            let x0 = (self.cx - half).ceil().max(0.0) as usize;
            let x1 = ((self.cx + half).floor().max(0.0) as usize).min(w - 1);
            let row = self.row_phases(f64::from(y));
            for x in x0..=x1 {
                let f = self.cell_value(x, &row);
                let v = (f.abs() / self.full_scale).min(1.0);
                let role = if f > 0.0 { Role::Hot } else { Role::Cold };
                let ink = Ink::new(role, (45.0 + 210.0 * v) as u8);
                canvas.put(x as i32, i32::from(y), draw::ramp(v, &RAMP), ink);
            }
        }
        if self.caption {
            let y = i32::from(self.height) - 1;
            draw::text_centered(canvas, y, CAPTION, Ink::new(Role::Text, 200));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(seed: u64, w: u16, h: u16) -> CosmicMicrowaveBackground {
        let size = Size::new(w, h);
        let mut g = CosmicMicrowaveBackground::default();
        g.resize(size, &mut Ctx::new(seed, size));
        g
    }

    #[test]
    fn cmb_a1_field_is_bounded_by_one() {
        for seed in 0..8 {
            let g = scene(seed, 40, 12);
            for i in 0..2000 {
                let (x, y, t) = (
                    f64::from(i % 97) * 0.7,
                    f64::from(i % 53),
                    f64::from(i) * 0.05,
                );
                assert!(field_value(&g.modes, x, y, t).abs() <= 1.0 + 1e-12);
            }
        }
    }

    #[test]
    fn cmb_a2_adjacent_cells_differ_by_at_most_the_gradient_bound() {
        for seed in 0..8 {
            let g = scene(seed, 200, 50);
            let bound = smoothness_bound(&g.modes);
            for t in [0.0, 3.3, 17.0] {
                for y in 0..50 {
                    for x in 0..199 {
                        let at =
                            |x: i32| field_value(&g.modes, f64::from(x) / ASPECT, f64::from(y), t);
                        let diff = (at(x + 1) - at(x)).abs();
                        assert!(diff <= bound + 1e-12, "seed {seed}: {diff} > {bound}");
                    }
                }
            }
        }
    }

    #[test]
    fn cmb_a3_full_frame_has_hot_and_cold_cells() {
        for seed in 0..8 {
            let mut stage = Stage::new(build(), seed, Size::new(200, 50), GlyphMode::Unicode);
            let frame = stage.render();
            let count = |r| frame.cells().iter().filter(|c| c.ink.role == r).count();
            assert!(
                count(Role::Hot) > 100 && count(Role::Cold) > 100,
                "seed {seed}"
            );
        }
    }

    #[test]
    fn cmb_separable_render_matches_field_value() {
        let mut g = scene(5, 120, 40);
        g.t = 4.2;
        for y in [0, 7, 39] {
            let row = g.row_phases(f64::from(y));
            for x in [0usize, 31, 119] {
                let direct = field_value(&g.modes, x as f64 / ASPECT, f64::from(y), g.t);
                assert!((g.cell_value(x, &row) - direct).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn cmb_modes_follow_the_spec_ranges_and_survive_resize() {
        let mut g = scene(9, 80, 24);
        let before = g.modes.clone();
        g.resize(Size::new(30, 10), &mut Ctx::new(123, Size::new(30, 10)));
        assert_eq!(g.modes, before);
        for m in &g.modes {
            let wavelength = TAU / m.kx.hypot(m.ky);
            assert!((3.0 - 1e-9..=20.0 + 1e-9).contains(&wavelength));
            assert!((0.2..=0.8).contains(&m.omega));
        }
    }

    #[test]
    fn cmb_oval_leaves_corners_empty_and_caption_on_last_row() {
        let mut stage = Stage::new(build(), 1, Size::new(100, 30), GlyphMode::Ascii);
        let frame = stage.render();
        assert_eq!(frame.get(0, 0).unwrap().ch, ' ');
        assert_eq!(frame.get(99, 28).unwrap().ch, ' ');
        assert_ne!(frame.get(50, 14).unwrap().ch, ' ');
        assert!(frame.to_lines()[29].contains(CAPTION));
    }
}
