//! Travelling through a star field (SPEC-002 `starfield`): near stars stream past, far ones
//! creep, and the Milky Way, a fixed band of faint resolved stars, stays at infinity. No
//! twinkle: there is no air in space. ADR-0017: no glow wash, no galaxy, dim points.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::geom;
use sciart_core::ink;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "starfield",
    category: Category::DeepCosmos,
    description: "Travelling through the stars: near ones stream past the fixed Milky Way",
    title: "Parallax · near stars rush by",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Starfield::default())
}

const DENSITY: f64 = 0.08;
const MAX_STARS: usize = 4000;
/// Relative distance of parallax layers 1..=4 (farthest first): each layer is half as far
/// as the one behind it.
const DIST: [f64; 4] = [8.0, 4.0, 2.0, 1.0];
/// Drift of the nearest layer in cells per second; parallax scales as `1 / distance`, so
/// the farthest layer creeps at an eighth of it (STAR-A2).
const NEAR_SPEED: f64 = 12.0;
/// Seconds of travel a star's motion streak spans, and its ink (STAR-A2): 0.14 s, so the
/// nearer layers, which move more than a glyph's width in it, draw a short streak and the
/// farther ones none.
const STREAK_TIME: f64 = 0.14;
const STREAK: Ink = Ink::new(Role::Star, 42);
/// Brightness buckets gained per layer nearer: inverse-square dimming, stylised (STAR-A4).
const LAYER_GAIN: f64 = 0.5;

/// Glyphs of the bright buckets 2..=4; buckets 0 and 1 are single braille dots, so they
/// glide at sub-cell steps instead of jumping a whole cell.
const GLYPHS: [Glyph; 3] = [
    Glyph::ascii('+'),
    Glyph::new('✦', '*'),
    Glyph::new('★', '*'),
];
/// Levels of the two faint (dot) buckets, drawn colourless as the eye sees faint stars.
const FAINT_LEVELS: [u8; 2] = [70, 130];
const BUCKETS: usize = FAINT_LEVELS.len() + GLYPHS.len();
/// Luminosity function: `P(bucket ≥ k) = LF_RATIO^{−k}`.
const LF_RATIO: f64 = 3.0;
/// Spectral mix of the field, as `(cumulative fraction, lowest K, highest K)`: M, K, G,
/// F, A and B stars (STAR-A6).
const CLASSES: [(f64, f64, f64); 6] = [
    (0.10, 3200.0, 3700.0),
    (0.40, 3900.0, 5200.0),
    (0.55, 5300.0, 5900.0),
    (0.70, 6000.0, 7300.0),
    (0.90, 7500.0, 9500.0),
    (1.00, 10_000.0, 25_000.0),
];

/// The Milky Way: the band's Gaussian σ (fraction of the height), where its light ends
/// (in σ; it fades to zero over the last `BAND_FADE` of that, so it never stops on a
/// cliff), and its tilt (rows per column, as a fraction of h/w).
const BAND_SIGMA: f64 = 0.05;
const BAND_EDGE: f64 = 3.2;
const BAND_FADE: f64 = 0.4;
const BAND_TILT: f64 = 0.35;
/// Clumping of the band's light along it: relative depth, and the wavelengths (fraction of
/// the width) of the three sinusoids that make it.
const CLUMP: f64 = 0.6;
const CLUMP_WAVES: [f64; 3] = [0.37, 0.19, 0.11];
/// One bright star cloud per screen (like the Sagittarius cloud): extra light, how much
/// wider it makes the band, and its Gaussian half-length (fraction of the width).
const BULGE: f64 = 0.8;
const BULGE_WIDEN: f64 = 0.3;
const BULGE_LEN: f64 = 0.1;
/// The dust lane: half-width (fraction of the height), how far it wanders off the band's
/// centre line, and the fraction of light it lets through. Its edges are steep (a
/// super-Gaussian), so the rift cuts cleanly through the middle.
const LANE: f64 = 0.018;
const LANE_WANDER: f64 = 0.02;
const LANE_GLOW: f64 = 0.15;
/// Resolved band stars: dots per cell, their cap and their `Star` levels. They are
/// scattered in proportion to the band's light, so they crowd the bright clouds.
const BAND_DENSITY: f64 = 0.1;
const BAND_MAX: usize = 8000;
const BAND_LEVEL: (f64, f64) = (40.0, 110.0);
/// The brightest the band's light gets, relative to its peak.
const LIGHT_MAX: f64 = (1.0 + CLUMP) * (1.0 + BULGE);
#[derive(Debug, Clone)]
struct Star {
    /// Cell coordinates; `y` is continuous so faint stars sit on any dot row.
    x: f64,
    y: f64,
    /// 1 = farthest/slowest, 4 = nearest.
    layer: u8,
    /// Brightness bucket, `0..BUCKETS`.
    bucket: usize,
    ink: Ink,
}

#[derive(Debug, Default)]
pub struct Starfield {
    stars: Vec<Star>,
    width: f64,
    /// The Milky Way's layout, per size.
    milky_way: MilkyWay,
    /// Resolved Milky Way stars in dot units: at infinity, so they never move.
    band: Vec<(f64, f64, Ink)>,
    /// Band stars plus the moving faint stars, rasterised every step.
    sky: Dots,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

fn star_count(size: Size) -> usize {
    ((size.area() as f64 * DENSITY) as usize).clamp(1, MAX_STARS)
}

/// Drift speed of `layer` in cells per second: parallax, `∝ 1 / distance` (STAR-A2).
fn speed(layer: u8) -> f64 {
    NEAR_SPEED / DIST[usize::from(layer.clamp(1, 4)) - 1]
}

/// A layer for a uniform `u`, weighted by distance: farther layers hold more stars
/// (STAR-A2). Stylised: a shell of space as thick as its distance holds ∝ d³ stars; ∝ d
/// keeps the near, fast layers from being empty.
fn layer_for(u: f64) -> u8 {
    let total: f64 = DIST.iter().sum();
    let mut cum = 0.0;
    for (i, d) in DIST.iter().enumerate() {
        cum += d / total;
        if u < cum {
            return i as u8 + 1;
        }
    }
    DIST.len() as u8
}

/// Brightness bucket for a uniform `u ∈ (0, 1]` in `layer`: `P(bucket ≥ k) = 3^{−k}` in
/// the farthest layer, each nearer layer `LAYER_GAIN` buckets brighter (STAR-A4).
fn bucket(u: f64, layer: u8) -> usize {
    let gain = LAYER_GAIN * f64::from(layer.saturating_sub(1));
    ((-u.ln() / LF_RATIO.ln() + gain) as usize).min(BUCKETS - 1)
}

/// A star's fixed ink: faint stars in plain starlight, bright ones in their blackbody
/// colour (only bright stars show colour to the eye).
fn star_ink(bucket: usize, k: f64) -> Ink {
    FAINT_LEVELS
        .get(bucket)
        .map_or_else(|| ink::blackbody(k), |&l| Ink::new(Role::Star, l))
}

/// The Milky Way's seeded layout: fixed at infinity, so it is laid out once per size.
#[derive(Debug, Clone, Copy, Default)]
struct MilkyWay {
    /// Phases of the clumping sinusoids and of the dust lane's wander.
    clumps: [f64; 3],
    lane: f64,
    /// Position of the bright star cloud along the band (fraction of the width).
    bulge: f64,
}

impl MilkyWay {
    fn new(rng: &mut impl Rng) -> Self {
        Self {
            clumps: std::array::from_fn(|_| rng.random_range(0.0..TAU)),
            lane: rng.random_range(0.0..TAU),
            bulge: rng.random_range(0.2..0.8),
        }
    }

    /// Row (cells) of the band's centre line at column `x`.
    fn centre(x: f64, w: f64, h: f64) -> f64 {
        h * (0.5 + BAND_TILT * (x / w - 0.5))
    }

    /// Offset (cells) of the dust lane's centre from the band's line at column `x`.
    fn lane_offset(&self, x: f64, w: f64, h: f64) -> f64 {
        LANE_WANDER * h * (1.3 * TAU * x / w + self.lane).sin_det()
    }

    /// The star cloud's weight at column `x`, 0 far from it and 1 on it.
    fn bulge_at(&self, x: f64, w: f64) -> f64 {
        (-0.5 * ((x / w - self.bulge) / BULGE_LEN).powi(2)).exp()
    }

    /// The band's σ (cells) at column `x`, wider through the star cloud.
    fn sigma(&self, x: f64, w: f64, h: f64) -> f64 {
        BAND_SIGMA * h * (1.0 + BULGE_WIDEN * self.bulge_at(x, w))
    }

    /// The band's light at column `x`, `off` cells from its centre line, in units of its
    /// peak (up to `LIGHT_MAX`): a Gaussian across the band that fades to exactly 0 at
    /// `BAND_EDGE` σ, clumped along it, brighter in the star cloud, cut by the dust lane.
    fn light(&self, x: f64, off: f64, w: f64, h: f64) -> f64 {
        let u = (off / self.sigma(x, w, h)).abs();
        self.light_at(x, off, u, self.bulge_at(x, w), w, h)
    }

    /// `light` given `u = |off| / σ(x)` and `bulge_at(x)`, which callers walking a column
    /// compute once for it. Exactly 0 from `u ≥ BAND_EDGE` (the fade is 0 there).
    fn light_at(&self, x: f64, off: f64, u: f64, bulge: f64, w: f64, h: f64) -> f64 {
        let fade = ease::smoothstep((BAND_EDGE - u) / (BAND_FADE * BAND_EDGE));
        let band = (-0.5 * u * u).exp() * fade;
        let lane = (off - self.lane_offset(x, w, h)) / (LANE * h);
        let lane = 1.0 - (1.0 - LANE_GLOW) * (-lane.powi(4)).exp();
        let clump = self
            .clumps
            .iter()
            .zip(CLUMP_WAVES)
            .map(|(&p, l)| (TAU * x / (l * w) + p + off / h * 4.0).sin_det())
            .sum::<f64>()
            / CLUMP_WAVES.len() as f64;
        band * lane * (1.0 + CLUMP * clump) * (1.0 + BULGE * bulge)
    }
}

impl Starfield {
    /// Scatter the resolved Milky Way stars in proportion to the band's light (by
    /// rejection), so they crowd the bright clouds and thin out in the dust lane.
    fn scatter_band(&mut self, size: Size, rng: &mut impl Rng) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let n = ((size.area() as f64 * BAND_DENSITY) as usize).min(BAND_MAX);
        self.band.clear();
        for _ in 0..8 * n {
            if self.band.len() == n {
                break;
            }
            let x = rng.random_range(0.0..w);
            let off = gaussian(rng, 0.0, self.milky_way.sigma(x, w, h));
            let light = self.milky_way.light(x, off, w, h) / LIGHT_MAX;
            if rng.random::<f64>() >= light {
                continue;
            }
            // Most faint, a few brighter (a steep spread), so the dots never read as an even grid.
            let level = BAND_LEVEL.0
                + (BAND_LEVEL.1 - BAND_LEVEL.0) * light.sqrt() * rng.random::<f64>().powi(3);
            let (dx, dy) = Dots::from_cells(x, MilkyWay::centre(x, w, h) + off);
            self.band
                .push((dx, dy, Ink::new(Role::Star, ink::level(level))));
        }
    }

    /// Redraw the dot layer: the fixed band stars, then the faint stars where they are now.
    fn rasterise(&mut self) {
        self.sky.clear();
        for &(x, y, ink) in &self.band {
            self.sky.plot(x, y, ink);
        }
        for s in &self.stars {
            let (x, y) = Dots::from_cells(s.x, s.y);
            let faint = s.bucket < FAINT_LEVELS.len();
            if faint {
                self.sky.plot(x, y, s.ink);
            }
            // A camera-exposure streak over the last `STREAK_TIME` of travel (stylised: an
            // eye sees none), so the parallax reads in any single frame; too short to
            // show for the far layers. It stops short of a bright star's glyph cell and is
            // a tick, never a dotted line: one dot behind a faint (one-dot) star, two behind a
            // glyph.
            let len = f64::from(Dots::CELL_W) * speed(s.layer) * STREAK_TIME;
            let gap = if faint { 1.0 } else { f64::from(Dots::CELL_W) };
            if len > gap {
                let tail = if faint { 0.0 } else { 1.0 };
                self.sky.line(x - gap - tail, y, x - gap, y, STREAK);
            }
        }
    }
}

impl Generator for Starfield {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        self.width = w;
        self.stars = (0..star_count(size))
            .map(|_| {
                let layer = layer_for(ctx.rng.random());
                let b = bucket(1.0 - ctx.rng.random::<f64>(), layer);
                let k = ink::class_kelvin(&CLASSES, ctx.rng.random(), ctx.rng.random());
                Star {
                    x: ctx.rng.random_range(0.0..w),
                    y: ctx.rng.random_range(0.0..h),
                    layer,
                    bucket: b,
                    ink: star_ink(b, k),
                }
            })
            .collect();
        self.milky_way = MilkyWay::new(&mut ctx.rng);
        self.scatter_band(size, &mut ctx.rng);
        self.sky.resize(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        for s in &mut self.stars {
            s.x = geom::wrap(s.x + speed(s.layer) * dt, self.width);
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.sky.blit(canvas);
        for s in self.stars.iter().filter(|s| s.bucket >= FAINT_LEVELS.len()) {
            canvas.put(
                s.x as i32,
                s.y as i32,
                GLYPHS[s.bucket - FAINT_LEVELS.len()],
                s.ink,
            );
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
        g.prepare();
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
    fn star_a2_nearer_layers_move_farther_as_one_over_distance() {
        let (mut g, mut ctx) = built(Size::new(1000, 10));
        for s in &mut g.stars {
            s.x = 0.0;
        }
        g.update(1.0, &mut ctx);
        g.prepare();
        for s in &g.stars {
            let want = NEAR_SPEED / DIST[usize::from(s.layer) - 1];
            assert!((s.x - want).abs() < 1e-9, "layer {} moved {}", s.layer, s.x);
        }
        assert!((1..4).all(|l| speed(l + 1) > speed(l)));
        assert_eq!(speed(4) / speed(1), 8.0);
        // Farther layers hold more stars.
        let mut per_layer = [0usize; 4];
        for i in 0..10_000 {
            per_layer[usize::from(layer_for((f64::from(i) + 0.5) / 1e4)) - 1] += 1;
        }
        assert!(per_layer.windows(2).all(|w| w[0] > w[1]), "{per_layer:?}");
    }

    #[test]
    fn star_a2_near_stars_trail_a_motion_streak_and_far_ones_do_not() {
        // One star per layer on an empty row, well inside the frame.
        let size = Size::new(60, 12);
        let (mut g, _) = built(size);
        g.band.clear();
        g.stars = (1..=4)
            .map(|layer| Star {
                x: 40.5,
                y: f64::from(2 * layer) + 0.5,
                layer,
                bucket: 3,
                ink: ink::blackbody(6000.0),
            })
            .collect();
        g.rasterise();
        // Lit dots on the star's row behind it (to the left: stars drift right).
        let trail = |layer: u8| {
            let (x, y) = Dots::from_cells(40.5, f64::from(2 * layer) + 0.5);
            (1..20)
                .filter(|&d| g.sky.is_lit(x.round() as i32 - d, y.round() as i32))
                .count()
        };
        assert_eq!(trail(1), 0, "the farthest layer creeps without a streak");
        assert!(trail(4) >= 2, "the nearest layer streaks: {}", trail(4));
        assert!(
            trail(4) > trail(3) && trail(3) >= trail(2),
            "{:?}",
            (1..=4).map(trail).collect::<Vec<_>>()
        );
    }

    #[test]
    fn star_a3_stars_hold_their_glyph_and_ink() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        let looks = |g: &Starfield| {
            g.stars
                .iter()
                .map(|s| (s.bucket, s.ink))
                .collect::<Vec<_>>()
        };
        let before = looks(&g);
        for _ in 0..600 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(before, looks(&g), "a star changed its look");
        // What is drawn is that look: the last bright star drawn owns its cell, and a
        // faint star is a lit dot in its own ink.
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let s = g.stars.iter().rev().find(|s| s.bucket >= 2).unwrap();
        let cell = canvas.get(s.x as i32, s.y as i32).unwrap();
        let glyph = GLYPHS[s.bucket - FAINT_LEVELS.len()];
        assert_eq!((cell.ch, cell.ink), (glyph.unicode, s.ink));
        let f = g.stars.iter().find(|s| s.bucket < 2).unwrap();
        let (dx, dy) = Dots::from_cells(f.x, f.y);
        assert!(g.sky.is_lit(dx.round() as i32, dy.round() as i32));
    }

    #[test]
    fn star_a4_luminosity_function_is_steep_and_nearer_stars_are_brighter() {
        let n = 30_000;
        let counts = |layer: u8| {
            let mut c = [0usize; BUCKETS];
            for i in 0..n {
                c[bucket(1.0 - (f64::from(i) + 0.5) / f64::from(n), layer)] += 1;
            }
            c
        };
        let far = counts(1);
        assert!(far.windows(2).all(|w| w[1] < w[0]), "{far:?}");
        assert!((far[0] as f64 / f64::from(n) - 2.0 / 3.0).abs() < 0.01);
        assert_eq!(bucket(1.0, 1), 0);
        assert_eq!(bucket(1e-12, 1), BUCKETS - 1);
        let mean = |c: [usize; BUCKETS]| {
            c.iter().enumerate().map(|(k, &m)| k * m).sum::<usize>() as f64 / f64::from(n)
        };
        let means: Vec<f64> = (1..=4).map(|l| mean(counts(l))).collect();
        assert!(means.windows(2).all(|w| w[1] > w[0]), "{means:?}");
        assert!((1.2..2.1).contains(&(means[3] - means[0])), "{means:?}");
    }

    #[test]
    fn star_a5_milky_way_is_static_and_banded_with_a_dark_lane() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size);
        let band = g.band.clone();
        g.update(3.0, &mut ctx);
        g.prepare();
        assert_eq!(band, g.band, "the band stars moved");
        // Resolved stars crowd the band.
        let (w, h) = (200.0, 50.0);
        let off = |&(dx, dy, _): &(f64, f64, Ink)| {
            let (x, y) = ((dx + 0.5) / 2.0, (dy + 0.5) / 4.0);
            (y - MilkyWay::centre(x, w, h)).abs() / h
        };
        let inside = band.iter().filter(|p| off(p) < BAND_SIGMA).count();
        let outside = band.iter().filter(|p| off(p) > 3.0 * BAND_SIGMA).count();
        assert!(inside >= 20 * outside.max(1), "{inside} vs {outside}");
        // ... and most of all its bright clouds: stars follow the light.
        let mw = g.milky_way;
        let light = |&(dx, dy, _): &(f64, f64, Ink)| {
            let (x, y) = ((dx + 0.5) / 2.0, (dy + 0.5) / 4.0);
            mw.light(x, y - MilkyWay::centre(x, w, h), w, h)
        };
        let at_stars = band.iter().map(light).sum::<f64>() / band.len() as f64;
        let (mut sum, mut n) = (0.0, 0.0);
        for py in 0..100 {
            for x in 0..200 {
                let (x, y) = (f64::from(x) + 0.5, (f64::from(py) + 0.5) / 2.0);
                let l = mw.light(x, y - MilkyWay::centre(x, w, h), w, h);
                if l > 0.0 {
                    (sum, n) = (sum + l, n + 1.0);
                }
            }
        }
        assert!(
            at_stars > 1.5 * sum / n,
            "mean light at the band stars {at_stars:.2} vs over the band {:.2}",
            sum / n
        );
        // The dust lane thins the band's stars: fewer in the lane than in an equally
        // wide strip on its shoulder (M14: by star counts, no glow).
        let strip = |shift: f64| {
            band.iter()
                .filter(|&&(dx, dy, _)| {
                    let (x, y) = ((dx + 0.5) / 2.0, (dy + 0.5) / 4.0);
                    let lane = MilkyWay::centre(x, w, h) + mw.lane_offset(x, w, h) + shift;
                    (y - lane).abs() < 0.5 * LANE * h
                })
                .count()
        };
        let (lane, shoulder) = (strip(0.0), strip(2.5 * LANE * h));
        assert!(2 * lane < shoulder, "lane {lane} vs shoulder {shoulder}");
        // Band stars stay faint: the field's bright stars are the glyphs.
        assert!(
            band.iter()
                .all(|&(_, _, i)| f64::from(i.level) <= BAND_LEVEL.1 + 1.0)
        );
    }

    #[test]
    fn star_a7_no_wash_and_no_galaxy_only_points() {
        // ADR-0017: the frame is black ground with points and streaks: no cell carries a
        // background colour and no ink is a glow ramp, at any seed.
        for (seed, (w, h)) in [(3, (120, 40)), (8, (200, 50)), (11, (80, 24))] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(seed, size);
            let mut g = Starfield::default();
            g.resize(size, &mut ctx);
            g.update(1.0, &mut ctx);
            g.prepare();
            let mut c = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut c);
            for cell in c.cells() {
                assert_eq!(cell.bg, Ink::BLANK, "seed {seed}");
                assert!(
                    matches!(
                        cell.ink.role,
                        Role::Background | Role::Star | Role::Blackbody
                    ),
                    "seed {seed}: {:?}",
                    cell.ink
                );
            }
            // Faint stars and band stars stay dim (≤ 130).
            assert!(g.band.iter().all(|&(_, _, i)| i.level <= 130));
            assert!(FAINT_LEVELS.iter().all(|&l| l <= 130));
        }
    }

    #[test]
    fn star_a6_bright_stars_show_their_temperature() {
        let (g, _) = built(Size::new(200, 50));
        let mut coloured = 0;
        for s in &g.stars {
            if s.bucket < FAINT_LEVELS.len() {
                assert_eq!(s.ink.role, Role::Star);
            } else {
                coloured += 1;
                assert_eq!(s.ink.role, Role::Blackbody);
                let k = ink::blackbody_kelvin(s.ink);
                assert!((3100.0..26_000.0).contains(&k), "{k} K");
            }
        }
        assert!(coloured > 20, "{coloured} coloured stars");
    }
}
