//! Parallax starfield with twinkling (SPEC-002 `starfield`).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::geom;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "starfield",
    category: Category::DeepCosmos,
    description: "Parallax starfield drifting past, stars twinkling in and out",
    title: "Parallax · near stars rush by",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Starfield::default())
}

const DENSITY: f64 = 0.08;
const MAX_STARS: usize = 4000;
/// Horizontal drift in cells per second, per parallax layer.
const SPEED: f64 = 1.5;
const LAYERS: u8 = 4;

/// Faintest to brightest.
const GLYPHS: [Glyph; 5] = [
    Glyph::ascii('.'),
    Glyph::new('·', '.'),
    Glyph::ascii('+'),
    Glyph::new('✦', '*'),
    Glyph::new('★', '*'),
];

#[derive(Debug, Clone)]
struct Star {
    x: f64,
    y: f64,
    /// 1 = farthest/slowest/dimmest, `LAYERS` = nearest.
    layer: u8,
    phase: f64,
    rate: f64,
}

#[derive(Debug, Default)]
pub struct Starfield {
    stars: Vec<Star>,
    width: f64,
    time: f64,
}

fn star_count(size: Size) -> usize {
    ((size.area() as f64 * DENSITY) as usize).clamp(1, MAX_STARS)
}

/// Brightness bucket `0..GLYPHS.len()` for a star at simulated time `t`,
/// or `None` when it has twinkled out.
fn brightness(star: &Star, t: f64) -> Option<usize> {
    let twinkle = (star.phase + star.rate * t).sin(); // -1..=1
    let b = f64::from(star.layer) - 1.0 + twinkle * 1.5;
    (b >= 0.0).then(|| (b as usize).min(GLYPHS.len() - 1))
}

impl Generator for Starfield {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        self.width = w;
        self.stars = (0..star_count(size))
            .map(|_| Star {
                x: ctx.rng.random_range(0.0..w),
                y: ctx.rng.random_range(0.0..h).floor(),
                layer: ctx.rng.random_range(1..=LAYERS),
                phase: ctx.rng.random_range(0.0..TAU),
                rate: ctx.rng.random_range(1.5..4.0),
            })
            .collect();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.time += dt;
        for s in &mut self.stars {
            s.x = geom::wrap(s.x + f64::from(s.layer) * SPEED * dt, self.width);
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        for s in &self.stars {
            if let Some(b) = brightness(s, self.time) {
                let level = 70 + (b as u8) * 46;
                canvas.put(
                    s.x as i32,
                    s.y as i32,
                    GLYPHS[b],
                    Ink::new(Role::Star, level),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (Starfield, Ctx) {
        let mut ctx = Ctx::new(3, size);
        let mut g = Starfield::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn star_a1_count_follows_density_formula() {
        for (w, h, want) in [(1, 1, 1), (100, 50, 400), (400, 200, MAX_STARS)] {
            let (g, _) = built(Size::new(w, h));
            assert_eq!(g.stars.len(), want, "{w}x{h}");
        }
    }

    #[test]
    fn star_a2_nearer_layers_move_farther() {
        let (mut g, mut ctx) = built(Size::new(1000, 10));
        for s in &mut g.stars {
            s.x = 0.0;
        }
        g.update(1.0, &mut ctx);
        for s in &g.stars {
            let expected = f64::from(s.layer) * SPEED;
            assert!(
                (s.x - expected).abs() < 1e-9,
                "layer {} moved {}",
                s.layer,
                s.x
            );
        }
    }

    #[test]
    fn brightness_is_bounded() {
        let star = Star {
            x: 0.0,
            y: 0.0,
            layer: LAYERS,
            phase: 0.0,
            rate: 2.0,
        };
        for i in 0..1000 {
            if let Some(b) = brightness(&star, f64::from(i) * 0.01) {
                assert!(b < GLYPHS.len());
            }
        }
    }
}
