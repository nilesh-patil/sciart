//! A star chart tracing and naming one constellation at a time (SPEC-002 `constellation_map`).
//!
//! Star positions are real J2000 RA/Dec, gnomonically projected about each figure's centre
//! and drawn as on a sky chart: north up, east to the LEFT (RA increases leftward).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::geom::{self, ASPECT};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "constellation_map",
    category: Category::Astronomy,
    description: "A star chart tracing and naming one constellation at a time",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ConstellationMap::default())
}

/// Seconds each constellation is on screen (CON-A3).
const SHOW: f64 = 12.0;
/// Seconds over which the stick figure traces in (CON-A2).
const TRACE: f64 = 4.0;
/// Final seconds during which the figure fades before the next one.
const FADE: f64 = 1.0;
/// Fraction of the screen the figure's bounding box may occupy.
const FIT: f64 = 0.8;
/// Background stars per cell.
const BG_DENSITY: f64 = 0.02;
const BG_MAX: usize = 2000;
/// Name typewriter speed, characters per second.
const TYPE_RATE: f64 = 18.0;

/// A catalogue star: J2000 right ascension and declination in degrees, visual magnitude.
struct CatStar {
    name: &'static str,
    ra: f64,
    dec: f64,
    mag: f64,
}

/// `ra` given as h, m, s for fidelity to the catalogue.
const fn star(name: &'static str, h: f64, m: f64, s: f64, dec: f64, mag: f64) -> CatStar {
    CatStar {
        name,
        ra: (h + m / 60.0 + s / 3600.0) * 15.0,
        dec,
        mag,
    }
}

/// A stick figure. The first star is the one labelled on the chart; edges are listed in
/// pen order, which is also the tracing order.
struct Figure {
    name: &'static str,
    epithet: &'static str,
    stars: &'static [CatStar],
    edges: &'static [(u8, u8)],
}

// Coordinates: J2000 (Hipparcos / Yale BSC), declinations in decimal degrees.
const FIGURES: [Figure; 6] = [
    Figure {
        name: "Orion",
        epithet: "the Hunter",
        stars: &[
            star("Betelgeuse", 5.0, 55.0, 10.3, 7.407, 0.42), // α Ori
            star("Meissa", 5.0, 35.0, 8.3, 9.934, 3.39),      // λ Ori, the head
            star("Bellatrix", 5.0, 25.0, 7.9, 6.350, 1.64),   // γ Ori
            star("Mintaka", 5.0, 32.0, 0.4, -0.299, 2.23),    // δ Ori, belt (west)
            star("Alnilam", 5.0, 36.0, 12.8, -1.202, 1.69),   // ε Ori, belt
            star("Alnitak", 5.0, 40.0, 45.5, -1.943, 1.77),   // ζ Ori, belt (east)
            star("Saiph", 5.0, 47.0, 45.4, -9.670, 2.06),     // κ Ori
            star("Rigel", 5.0, 14.0, 32.3, -8.202, 0.13),     // β Ori
        ],
        edges: &[
            (0, 1),
            (1, 2),
            (0, 5),
            (2, 3),
            (3, 4),
            (4, 5),
            (5, 6),
            (3, 7),
        ],
    },
    Figure {
        name: "Ursa Major",
        epithet: "the Great Bear",
        stars: &[
            star("Dubhe", 11.0, 3.0, 43.7, 61.751, 1.79),   // α UMa
            star("Merak", 11.0, 1.0, 50.5, 56.382, 2.37),   // β UMa
            star("Phecda", 11.0, 53.0, 49.8, 53.695, 2.44), // γ UMa
            star("Megrez", 12.0, 15.0, 25.6, 57.033, 3.31), // δ UMa
            star("Alioth", 12.0, 54.0, 1.7, 55.960, 1.77),  // ε UMa
            star("Mizar", 13.0, 23.0, 55.5, 54.925, 2.23),  // ζ UMa
            star("Alkaid", 13.0, 47.0, 32.4, 49.313, 1.86), // η UMa
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (3, 0), (3, 4), (4, 5), (5, 6)],
    },
    Figure {
        name: "Cassiopeia",
        epithet: "the Queen",
        stars: &[
            star("Schedar", 0.0, 40.0, 30.4, 56.537, 2.24), // α Cas
            star("Caph", 0.0, 9.0, 10.7, 59.150, 2.27),     // β Cas
            star("Navi", 0.0, 56.0, 42.5, 60.717, 2.15),    // γ Cas
            star("Ruchbah", 1.0, 25.0, 49.0, 60.235, 2.68), // δ Cas
            star("Segin", 1.0, 54.0, 23.7, 63.670, 3.37),   // ε Cas
        ],
        edges: &[(1, 0), (0, 2), (2, 3), (3, 4)],
    },
    Figure {
        name: "Cygnus",
        epithet: "the Swan",
        stars: &[
            star("Deneb", 20.0, 41.0, 25.9, 45.280, 1.25), // α Cyg, the tail
            star("Sadr", 20.0, 22.0, 13.7, 40.257, 2.23),  // γ Cyg, the breast
            star("η Cyg", 19.0, 56.0, 18.4, 35.083, 3.89), // η Cyg, the neck
            star("Albireo", 19.0, 30.0, 43.3, 27.960, 3.05), // β Cyg, the beak
            star("Fawaris", 19.0, 44.0, 58.5, 45.131, 2.87), // δ Cyg, wing
            star("ι Cyg", 19.0, 29.0, 42.4, 51.730, 3.76), // ι Cyg, wingtip
            star("Aljanah", 20.0, 46.0, 12.7, 33.970, 2.48), // ε Cyg, wing
            star("ζ Cyg", 21.0, 12.0, 56.2, 30.227, 3.21), // ζ Cyg, wingtip
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (1, 4), (4, 5), (1, 6), (6, 7)],
    },
    Figure {
        name: "Crux",
        epithet: "the Southern Cross",
        stars: &[
            star("Acrux", 12.0, 26.0, 35.9, -63.099, 0.76), // α Cru, foot
            star("Gacrux", 12.0, 31.0, 10.0, -57.113, 1.64), // γ Cru, head
            star("Mimosa", 12.0, 47.0, 43.3, -59.689, 1.25), // β Cru, east arm
            star("Imai", 12.0, 15.0, 8.7, -58.749, 2.79),   // δ Cru, west arm
        ],
        edges: &[(0, 1), (2, 3)],
    },
    Figure {
        name: "Scorpius",
        epithet: "the Scorpion",
        stars: &[
            star("Antares", 16.0, 29.0, 24.5, -26.432, 1.06), // α Sco, the heart
            star("Acrab", 16.0, 5.0, 26.2, -19.806, 2.62),    // β¹ Sco, claw
            star("Dschubba", 16.0, 0.0, 20.0, -22.622, 2.29), // δ Sco, head
            star("Fang", 15.0, 58.0, 51.1, -26.114, 2.89),    // π Sco, claw
            star("Alniyat", 16.0, 21.0, 11.3, -25.593, 2.88), // σ Sco
            star("Paikauhale", 16.0, 35.0, 53.0, -28.216, 2.82), // τ Sco
            star("Larawag", 16.0, 50.0, 9.8, -34.293, 2.29),  // ε Sco
            star("μ¹ Sco", 16.0, 51.0, 52.2, -38.047, 3.00),  // μ¹ Sco
            star("ζ² Sco", 16.0, 54.0, 35.0, -42.362, 3.62),  // ζ² Sco
            star("η Sco", 17.0, 12.0, 9.2, -43.239, 3.33),    // η Sco
            star("Sargas", 17.0, 37.0, 19.1, -42.998, 1.86),  // θ Sco
            star("ι¹ Sco", 17.0, 47.0, 35.1, -40.127, 2.99),  // ι¹ Sco
            star("Girtab", 17.0, 42.0, 29.3, -39.030, 2.39),  // κ Sco
            star("Shaula", 17.0, 33.0, 36.5, -37.104, 1.62),  // λ Sco, the sting
            star("Lesath", 17.0, 30.0, 45.8, -37.296, 2.69),  // υ Sco, the sting
        ],
        edges: &[
            (1, 2),
            (2, 3),
            (2, 4),
            (4, 0),
            (0, 5),
            (5, 6),
            (6, 7),
            (7, 8),
            (8, 9),
            (9, 10),
            (10, 11),
            (11, 12),
            (12, 13),
            (13, 14),
        ],
    },
];

/// Brightest to faintest; picked by magnitude bands (≤1, ≤2, ≤3, fainter).
const STAR_GLYPHS: [Glyph; 4] = [
    Glyph::new('★', '@'),
    Glyph::new('✦', '*'),
    Glyph::new('✧', '+'),
    Glyph::new('·', '.'),
];
/// Background stars stay small so they never read as figure stars.
const BG_GLYPHS: [Glyph; 2] = [Glyph::ascii('.'), Glyph::new('·', '.')];
const PEN: Glyph = Glyph::new('•', 'o');

/// Which constellation is showing at time `t`, and seconds into its slot (CON-A3).
pub(crate) fn current(t: f64) -> (usize, f64) {
    let t = t.max(0.0);
    let slot = (t / SHOW).floor();
    let idx = (slot as u64 % FIGURES.len() as u64) as usize;
    (idx, (t - slot * SHOW).clamp(0.0, SHOW))
}

/// Per-edge drawn fraction for tracing `progress ∈ [0, 1]` along the edges in order,
/// at constant pen speed over their total length (CON-A2).
pub(crate) fn trace(progress: f64, lengths: &[f64]) -> impl Iterator<Item = f64> + '_ {
    let p = if progress.is_nan() {
        0.0
    } else {
        progress.clamp(0.0, 1.0)
    };
    let total: f64 = lengths.iter().sum();
    let target = p * total;
    lengths.iter().scan(0.0, move |cum: &mut f64, &len| {
        let start = *cum;
        *cum += len;
        Some(if p >= 1.0 {
            1.0
        } else if p <= 0.0 {
            0.0
        } else if len <= 0.0 {
            if target >= start { 1.0 } else { 0.0 }
        } else {
            ((target - start) / len).clamp(0.0, 1.0)
        })
    })
}

/// Gnomonic (tangent-plane) projection of `(ra, dec)` about `(ra0, dec0)`, all in degrees.
/// Returns chart coordinates in radians with x growing to the WEST (east is left, as on a
/// sky chart seen from inside the celestial sphere) and y growing SOUTH (screen down).
pub(crate) fn project(ra: f64, dec: f64, ra0: f64, dec0: f64) -> (f64, f64) {
    let (a, d, d0) = (ra.to_radians(), dec.to_radians(), dec0.to_radians());
    let da = a - ra0.to_radians();
    let cos_c = d0.sin() * d.sin() + d0.cos() * d.cos() * da.cos();
    let xi = d.cos() * da.sin() / cos_c; // eastward
    let eta = (d0.cos() * d.sin() - d0.sin() * d.cos() * da.cos()) / cos_c; // northward
    (-xi, -eta)
}

/// Direction of the figure's centre: normalized mean of the stars' unit vectors,
/// which handles the RA wrap at 0h (Cassiopeia straddles it).
fn centre(stars: &[CatStar]) -> (f64, f64) {
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    for s in stars {
        let (a, d) = (s.ra.to_radians(), s.dec.to_radians());
        x += d.cos() * a.cos();
        y += d.cos() * a.sin();
        z += d.sin();
    }
    (y.atan2(x).to_degrees(), z.atan2(x.hypot(y)).to_degrees())
}

/// Screen positions (cells, fractional) of a figure fitted to `FIT` of `size`, preserving
/// shape: one chart radian spans the same visual distance horizontally and vertically.
fn fit(fig: &Figure, size: Size) -> Vec<(f64, f64)> {
    let (ra0, dec0) = centre(fig.stars);
    let pts: Vec<(f64, f64)> = fig
        .stars
        .iter()
        .map(|s| project(s.ra, s.dec, ra0, dec0))
        .collect();
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in &pts {
        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
    }
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    // Rows per chart radian; x cells are ASPECT times narrower than rows are tall.
    let scale = (FIT * (w - 1.0) / (ASPECT * (x1 - x0)).max(1e-9))
        .min(FIT * (h - 1.0) / (y1 - y0).max(1e-9))
        .max(0.0);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    pts.iter()
        .map(|&(x, y)| {
            (
                (w - 1.0) / 2.0 + (x - mx) * scale * ASPECT,
                (h - 1.0) / 2.0 + (y - my) * scale,
            )
        })
        .collect()
}

/// Box-drawing glyph approximating the direction of a segment in cell units.
fn line_glyph(dx: f64, dy: f64) -> Glyph {
    let slope = dy.abs() / dx.abs().max(1e-9);
    if slope < 0.4 {
        Glyph::new('─', '-')
    } else if slope > 2.5 {
        Glyph::new('│', '|')
    } else if (dx > 0.0) == (dy > 0.0) {
        Glyph::new('╲', '\\')
    } else {
        Glyph::new('╱', '/')
    }
}

fn mag_bucket(mag: f64) -> usize {
    match mag {
        m if m <= 1.0 => 0,
        m if m <= 2.0 => 1,
        m if m <= 3.0 => 2,
        _ => 3,
    }
}

/// Ink level for a star of magnitude `mag`: brighter star, brighter ink.
fn mag_level(mag: f64) -> f64 {
    (255.0 - (mag + 0.5) * 38.0).clamp(90.0, 255.0)
}

#[derive(Debug, Clone)]
struct BgStar {
    x: i32,
    y: i32,
    phase: f64,
    rate: f64,
}

#[derive(Debug, Default)]
pub struct ConstellationMap {
    t: f64,
    width: i32,
    height: i32,
    /// Fitted star positions per figure.
    pos: Vec<Vec<(f64, f64)>>,
    /// Visual edge lengths per figure (rows), the tracing weights.
    lengths: Vec<Vec<f64>>,
    bg: Vec<BgStar>,
}

impl ConstellationMap {
    fn scaled(level: f64, fade: f64) -> u8 {
        (level * fade).clamp(0.0, 255.0) as u8
    }

    /// Letter-spaced title, only the first `shown` characters (typewriter reveal).
    fn title(canvas: &mut Canvas, y: i32, s: &str, shown: usize, ink: Ink) {
        let n = s.chars().count() as i32;
        let x0 = (i32::from(canvas.size().width()) - (2 * n - 1)) / 2;
        for (i, ch) in s.chars().take(shown).enumerate() {
            let ch = ch.to_ascii_uppercase();
            canvas.put(x0 + 2 * i as i32, y, Glyph::ascii(ch), ink);
        }
    }
}

impl Generator for ConstellationMap {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.width = i32::from(size.width());
        self.height = i32::from(size.height());
        self.pos = FIGURES.iter().map(|f| fit(f, size)).collect();
        self.lengths = FIGURES
            .iter()
            .zip(&self.pos)
            .map(|(f, p)| {
                f.edges
                    .iter()
                    .map(|&(a, b)| {
                        let (pa, pb) = (p[a as usize], p[b as usize]);
                        ((pb.0 - pa.0) / ASPECT).hypot(pb.1 - pa.1)
                    })
                    .collect()
            })
            .collect();
        let n = ((size.area() as f64 * BG_DENSITY) as usize).min(BG_MAX);
        let (w, h) = (size.width(), size.height());
        self.bg = (0..n)
            .map(|_| BgStar {
                x: i32::from(ctx.rng.random_range(0..w)),
                y: i32::from(ctx.rng.random_range(0..h)),
                phase: ctx.rng.random_range(0.0..TAU),
                rate: ctx.rng.random_range(1.0..3.5),
            })
            .collect();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        // Background sky: faint, twinkling.
        for s in &self.bg {
            let tw = 0.5 + 0.5 * (s.phase + s.rate * self.t).sin();
            if tw > 0.15 {
                let g = BG_GLYPHS[usize::from(tw > 0.6)];
                canvas.put(s.x, s.y, g, Ink::new(Role::Dim, (40.0 + 90.0 * tw) as u8));
            }
        }

        let (idx, local) = current(self.t);
        let (Some(pos), Some(lengths)) = (self.pos.get(idx), self.lengths.get(idx)) else {
            return;
        };
        let fig = &FIGURES[idx];
        // Fade in over the first half second, out over the last FADE seconds.
        let fade = (local / 0.5).min((SHOW - local) / FADE).clamp(0.0, 1.0);
        let progress = local / TRACE;

        // Stick figure, traced at constant pen speed; the pen tip glows while drawing.
        for (&(a, b), frac) in fig.edges.iter().zip(trace(progress, lengths)) {
            if frac <= 0.0 {
                continue;
            }
            let (pa, pb) = (pos[a as usize], pos[b as usize]);
            let end = (pa.0 + (pb.0 - pa.0) * frac, pa.1 + (pb.1 - pa.1) * frac);
            let g = line_glyph(pb.0 - pa.0, pb.1 - pa.1);
            let ink = Ink::new(Role::Structure, Self::scaled(170.0, fade));
            let (x0, y0) = (pa.0.round() as i32, pa.1.round() as i32);
            let (x1, y1) = (end.0.round() as i32, end.1.round() as i32);
            for (x, y) in geom::line(x0, y0, x1, y1) {
                canvas.put(x, y, g, ink);
            }
            if frac < 1.0 {
                canvas.put(
                    x1,
                    y1,
                    PEN,
                    Ink::new(Role::Accent, Self::scaled(255.0, fade)),
                );
            }
        }

        // Stars over the lines, sized and lit by magnitude.
        for (s, &(x, y)) in fig.stars.iter().zip(pos) {
            let level = Self::scaled(mag_level(s.mag), fade);
            let g = STAR_GLYPHS[mag_bucket(s.mag)];
            canvas.put(
                x.round() as i32,
                y.round() as i32,
                g,
                Ink::new(Role::Star, level),
            );
        }

        if local < TRACE {
            return;
        }
        // After tracing: label the α star and type out the name beneath the figure.
        let since = local - TRACE;
        let label_ink = Ink::new(
            Role::Text,
            Self::scaled(150.0 * (since / 0.5).min(1.0), fade),
        );
        let lead = &fig.stars[0];
        let (lx, ly) = (pos[0].0.round() as i32, pos[0].1.round() as i32);
        let len = lead.name.chars().count() as i32;
        let x = if lx + 2 + len <= self.width {
            lx + 2
        } else {
            lx - 1 - len
        };
        canvas.text(x, ly, lead.name, label_ink);

        let bottom = pos.iter().map(|p| p.1).fold(0.0, f64::max).round() as i32;
        let row = (bottom + 2).min(self.height - 2).max(0);
        let shown = (since * TYPE_RATE) as usize;
        let name_ink = Ink::new(Role::Text, Self::scaled(255.0, fade));
        Self::title(canvas, row, fig.name, shown, name_ink);
        let epithet_shown = shown.saturating_sub(fig.name.chars().count() + 2);
        if epithet_shown > 0 && row + 1 < self.height {
            let n = fig.epithet.chars().count().min(epithet_shown);
            let x = (self.width - fig.epithet.chars().count() as i32) / 2;
            let ink = Ink::new(Role::Text, Self::scaled(140.0, fade));
            for (i, ch) in fig.epithet.chars().take(n).enumerate() {
                canvas.put(x + i as i32, row + 1, Glyph::ascii(ch), ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn con_a1_every_edge_valid_and_every_star_on_an_edge() {
        assert_eq!(FIGURES.len(), 6);
        for f in &FIGURES {
            assert!(!f.edges.is_empty(), "{}", f.name);
            let mut used = vec![false; f.stars.len()];
            for &(a, b) in f.edges {
                let (a, b) = (usize::from(a), usize::from(b));
                assert!(
                    a < f.stars.len() && b < f.stars.len(),
                    "{}: edge {a}-{b}",
                    f.name
                );
                assert_ne!(a, b, "{}: self-loop", f.name);
                used[a] = true;
                used[b] = true;
            }
            assert!(used.iter().all(|&u| u), "{}: orphan star", f.name);
            for s in f.stars {
                assert!((0.0..360.0).contains(&s.ra) && (-90.0..=90.0).contains(&s.dec));
                assert!((-1.5..6.0).contains(&s.mag), "{}", s.name);
            }
        }
    }

    #[test]
    fn con_a1_charts_put_east_left_and_north_up() {
        let orion = &FIGURES[0];
        let p = fit(orion, Size::new(120, 40));
        // Betelgeuse (higher RA, east) sits left of Bellatrix; Rigel is below Betelgeuse.
        assert!(p[0].0 < p[2].0, "Betelgeuse should be left of Bellatrix");
        assert!(p[7].1 > p[0].1, "Rigel should be below Betelgeuse");
        // Crux: Gacrux (head, north) above Acrux (foot).
        let crux = fit(&FIGURES[4], Size::new(120, 40));
        assert!(crux[1].1 < crux[0].1);
    }

    #[test]
    fn con_a1_gnomonic_projection_matches_analytic_cases() {
        // Centre maps to the origin; a star due north by θ lands at η = tan θ.
        let (x, y) = project(83.0, -1.0, 83.0, -1.0);
        assert!(x.abs() < 1e-12 && y.abs() < 1e-12);
        let (x, y) = project(83.0, 9.0, 83.0, -1.0);
        assert!(x.abs() < 1e-12 && (y + 10f64.to_radians().tan()).abs() < 1e-12);
        // On the equator, a star 10° east lies at ξ = tan 10°, drawn to the left.
        let (x, _) = project(93.0, 0.0, 83.0, 0.0);
        assert!((x + 10f64.to_radians().tan()).abs() < 1e-12);
    }

    #[test]
    fn con_a1_figures_fill_80_percent_preserving_shape() {
        for size in [Size::new(200, 50), Size::new(80, 60)] {
            let (w, h) = (
                f64::from(size.width()) - 1.0,
                f64::from(size.height()) - 1.0,
            );
            for f in &FIGURES {
                let p = fit(f, size);
                let xs = p.iter().map(|q| q.0);
                let ys = p.iter().map(|q| q.1);
                let span_x = xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min);
                let span_y = ys.clone().fold(f64::MIN, f64::max) - ys.fold(f64::MAX, f64::min);
                assert!(
                    span_x <= FIT * w + 1e-9 && span_y <= FIT * h + 1e-9,
                    "{}",
                    f.name
                );
                // Tight in at least one dimension.
                let tight = (span_x - FIT * w).abs() < 1e-6 || (span_y - FIT * h).abs() < 1e-6;
                assert!(tight, "{} {span_x} {span_y}", f.name);
            }
        }
        // Shape: Dubhe–Merak vs Merak–Phecda ratio independent of screen size.
        let ratio = |size| {
            let p = fit(&FIGURES[1], size);
            let d = |a: usize, b: usize| ((p[a].0 - p[b].0) / ASPECT).hypot(p[a].1 - p[b].1);
            d(0, 1) / d(1, 2)
        };
        assert!((ratio(Size::new(200, 50)) - ratio(Size::new(90, 70))).abs() < 1e-9);
    }

    #[test]
    fn con_a2_trace_empty_full_and_monotone() {
        let lengths = [3.0, 0.0, 5.5, 1.25, 7.0];
        assert!(trace(0.0, &lengths).all(|f| f == 0.0));
        assert!(trace(1.0, &lengths).all(|f| f == 1.0));
        let drawn = |p| {
            trace(p, &lengths)
                .zip(&lengths)
                .map(|(f, l)| f * l)
                .sum::<f64>()
        };
        let mut prev = 0.0;
        for i in 0..=1000 {
            let d = drawn(f64::from(i) / 1000.0);
            assert!(d + 1e-12 >= prev, "not monotone at {i}");
            prev = d;
        }
        // Constant pen speed: halfway through time is halfway along the path.
        assert!((drawn(0.5) - 0.5 * lengths.iter().sum::<f64>()).abs() < 1e-9);
    }

    #[test]
    fn con_a2_render_draws_nothing_at_start_and_everything_after_trace() {
        let size = Size::new(100, 30);
        let lines_at = |t: f64| {
            let mut g = ConstellationMap::default();
            let mut ctx = Ctx::new(1, size);
            g.resize(size, &mut ctx);
            g.bg.clear();
            g.t = t;
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut c);
            c.to_lines()
                .concat()
                .chars()
                .filter(|ch| "-|/\\".contains(*ch))
                .count()
        };
        assert_eq!(lines_at(0.0), 0);
        assert!(lines_at(2.0) > 0);
        assert!(lines_at(TRACE + 0.5) >= lines_at(2.0));
    }

    #[test]
    fn con_a3_index_advances_every_12s_and_wraps() {
        assert_eq!(current(0.0), (0, 0.0));
        assert_eq!(current(11.9).0, 0);
        assert_eq!(current(12.0).0, 1);
        for k in 0..20 {
            let (i, local) = current(SHOW * f64::from(k) + 3.0);
            assert_eq!(i, k as usize % FIGURES.len());
            assert!((local - 3.0).abs() < 1e-9);
        }
        assert_eq!(current(SHOW * 6.0).0, 0);
    }
}
