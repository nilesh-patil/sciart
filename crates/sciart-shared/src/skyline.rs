//! An anti-aliased horizon silhouette (a far ridge and an optional conifer forest) shared
//! by the Earth & Sky scenes. Heights are in degrees above the horizon, so the land keeps
//! its shape at every size; each pixel column is sub-sampled, so a spruce flank is a soft
//! slope of partly covered pixels rather than a staircase.

use rand::Rng;
use sciart_core::timeline::ease;

/// Height samples per pixel column: the silhouette is anti-aliased across as well as
/// down, so a spruce flank is a soft slope of partly covered pixels, not a staircase.
pub const SKY_SUB: usize = 4;

/// What stands on the horizon: a far ridge and, optionally, a conifer forest. Heights
/// are in degrees above the horizon, so the land keeps its shape at every size.
#[derive(Debug, Clone, Copy)]
pub struct Terrain {
    /// Ridge base and relief (deg): `base + relief · noise`.
    pub ridge: (f64, f64),
    /// Tree heights (deg), or `None` for bare hills.
    pub trees: Option<(f64, f64)>,
    /// Gap between neighbouring spires (deg).
    pub gap: (f64, f64),
    /// Chance that a stretch of forest is a clearing.
    pub clearing: f64,
}

/// A spruce forest on a low ridge (the tests' reference terrain; scenes set their own).
#[cfg(any(test, feature = "test-support"))]
pub const FOREST: Terrain = Terrain {
    ridge: (0.6, 2.2),
    trees: Some((1.4, 4.2)),
    gap: (0.35, 1.6),
    clearing: 0.2,
};

/// Spruce tiers: the crown's width runs down each tier like a sawtooth, so a tree reads
/// as a layered conifer rather than a triangle.
const TIERS: f64 = 5.0;
/// Crown half-width at the base, as a fraction of the tree's height.
const CROWN: f64 = 0.27;

/// An anti-aliased silhouette: silhouette height above the horizon in pixels, at
/// [`SKY_SUB`] samples per pixel column. Seeded and laid out in `resize`.
#[derive(Debug, Clone, Default)]
pub struct Skyline {
    sub: Vec<f32>,
}

/// One spire: centre (deg from the left edge), height and base half-width (deg).
#[derive(Debug, Clone, Copy)]
struct Spire {
    at: f64,
    height: f64,
    half: f64,
}

impl Spire {
    /// Height (deg) of the crown `off` degrees from the trunk, or 0 beyond it: the highest
    /// point of the tiered crown whose half-width at height `v` (0 = base, 1 = tip) is
    /// `half · (1 − v) · (0.72 + 0.28 · frac(v · TIERS))`.
    fn crown(&self, off: f64) -> f64 {
        let off = off.abs();
        if off >= self.half {
            return 0.0;
        }
        // Scan down from the tip: the first height whose width reaches `off` is the top.
        const STEPS: usize = 40;
        for i in 0..=STEPS {
            let v = 1.0 - i as f64 / STEPS as f64;
            let frac = (v * TIERS).fract();
            if self.half * (1.0 - v) * (0.72 + 0.28 * frac) >= off {
                return self.height * v;
            }
        }
        0.0
    }
}

impl Skyline {
    /// A silhouette `width` pixels wide at `px_per_deg`, seeded from `rng`.
    pub fn new(rng: &mut impl Rng, width: usize, px_per_deg: f64, terrain: Terrain) -> Self {
        let px_per_deg = if px_per_deg.is_finite() && px_per_deg > 0.0 {
            px_per_deg
        } else {
            1.0
        };
        let span = width as f64 / px_per_deg;
        // Two octaves of value noise for the ridge, nodes every 9° and 2.5°.
        let coarse: Vec<f64> = (0..(span / 9.0) as usize + 3)
            .map(|_| rng.random_range(0.0..1.0))
            .collect();
        let fine: Vec<f64> = (0..(span / 2.5) as usize + 3)
            .map(|_| rng.random_range(0.0..1.0))
            .collect();
        let noise = |nodes: &[f64], x: f64| {
            let i = (x as usize).min(nodes.len() - 2);
            ease::lerp(nodes[i], nodes[i + 1], ease::smoothstep(x - i as f64))
        };
        let mut spires = Vec::new();
        if let Some((lo, hi)) = terrain.trees {
            let mut at = rng.random_range(-1.0..0.5);
            let mut clearing = 0.0;
            while at < span + 2.0 {
                if clearing > 0.0 {
                    clearing -= 1.0;
                } else if rng.random_bool(terrain.clearing.clamp(0.0, 1.0) / 6.0) {
                    clearing = f64::from(rng.random_range(2u8..7));
                } else {
                    // Heights skew low: many young trees, a few tall ones.
                    let u: f64 = rng.random_range(0.0..1.0);
                    let height = lo + (hi - lo) * u * u;
                    spires.push(Spire {
                        at,
                        height,
                        half: CROWN * height * rng.random_range(0.8..1.25),
                    });
                }
                at += rng.random_range(terrain.gap.0..terrain.gap.1);
            }
        }
        let sub = (0..width * SKY_SUB)
            .map(|i| {
                let deg = (i as f64 + 0.5) / (SKY_SUB as f64 * px_per_deg);
                let ridge = terrain.ridge.0
                    + terrain.ridge.1
                        * (0.7 * noise(&coarse, deg / 9.0) + 0.3 * noise(&fine, deg / 2.5));
                let trees = spires
                    .iter()
                    .filter(|s| (s.at - deg).abs() < s.half)
                    .map(|s| s.crown(deg - s.at))
                    .fold(0.0, f64::max);
                // Trees stand on the ridge's lower slopes: part of each trunk is hidden.
                (px_per_deg * ridge.max(0.6 * terrain.ridge.0 + trees + 0.4 * ridge)) as f32
            })
            .collect();
        Self { sub }
    }

    /// Silhouette height (px) above the horizon at continuous pixel column `x`.
    pub fn height_at(&self, x: f64) -> f64 {
        if self.sub.is_empty() || !x.is_finite() {
            return 0.0;
        }
        let i = ((x * SKY_SUB as f64).max(0.0) as usize).min(self.sub.len() - 1);
        f64::from(self.sub[i])
    }

    /// Share of pixel `(x, y)` (spanning rows `[y, y + 1)`) hidden by the land when the
    /// horizon lies at pixel row `base`: averaged over the column's sub-samples, each
    /// covering `clamp(y + 1 − top, 0, 1)` of the pixel.
    pub fn cover(&self, x: i32, y: i32, base: f64) -> f32 {
        let Ok(x) = usize::try_from(x) else {
            return 0.0;
        };
        let Some(col) = self.sub.get(x * SKY_SUB..(x + 1) * SKY_SUB) else {
            return 0.0;
        };
        let bottom = y as f32 + 1.0;
        let sum: f32 = col
            .iter()
            .map(|&h| (bottom - (base as f32 - h)).clamp(0.0, 1.0))
            .sum();
        sum / SKY_SUB as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    #[test]
    fn skyline_cover_is_soft_and_complete_below_the_horizon() {
        let mut rng = SplitMix64::new(9);
        let land = Skyline::new(&mut rng, 100, 2.0, FOREST);
        for x in 0..100 {
            assert_eq!(land.cover(x, 60, 50.0), 1.0);
            assert_eq!(land.cover(x, 20, 50.0), 0.0);
        }
        // Some pixels are partly covered: the edge is anti-aliased.
        let partial = (0..100)
            .flat_map(|x| (30..50).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let c = land.cover(x, y, 50.0);
                c > 0.05 && c < 0.95
            })
            .count();
        assert!(partial > 30, "{partial}");
    }
}
