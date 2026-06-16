//! A star chart tracing and naming one constellation at a time (SPEC-002 `constellation_map`).
//!
//! Star positions are real J2000 RA/Dec, gnomonically projected about each figure's centre
//! and drawn as on a sky chart: north up, east to the LEFT (RA increases leftward). Stars
//! are 1–2 px points sized by magnitude and coloured by their B–V index, the brightest
//! scintillating; the figure is traced in faint braille strokes over a faint continuous
//! RA/Dec graticule, with a background field that thickens toward the galactic plane
//! (ADR-0017: no Milky Way wash, no blooms). Now and then a meteor streaks across the
//! chart. The graticule is rasterised when the chart or its fade changes, stars and
//! strokes every step; `render` blits them and draws text.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster;
use sciart_core::rng::hash01;
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "constellation_map",
    category: Category::EarthSky,
    description: "A star chart tracing and naming one constellation at a time",
    title: "Constellations · star patterns",
    captions: &[],
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
/// Fraction of the chart area the figure's bounding box may occupy.
const FIT: f64 = 0.8;
/// Rows below the chart area: a gap, the name, the epithet, a gap and the title row, so no
/// star or its bloom sits on the name block (CON-A7, GEN-R13).
const NAME_ROWS: u16 = 5;
/// Background star candidates per cell. Each chart keeps a share of them that rises from
/// `BG_KEEP.0` far from the Milky Way to `BG_KEEP.1` in its plane (galactic latitude
/// within ~`BG_PLANE_DEG`), so Cygnus, Crux and Scorpius sit in a denser field.
const BG_DENSITY: f64 = 0.06;
const BG_MAX: usize = 4000;
const BG_KEEP: (f64, f64) = (0.25, 1.0);
const BG_PLANE_DEG: f64 = 12.0;
/// North galactic pole, J2000 (deg), and the galactic longitude of the north celestial
/// pole (IAU 1958 system, J2000 values).
const NGP: (f64, f64) = (192.859_48, 27.128_25);
const L_NCP: f64 = 122.931_92;
/// Text levels: the name, its epithet and the α star's label stay dim, so the stars carry
/// the chart (ADR-0017).
const NAME_LEVEL: f64 = 120.0;
const EPITHET_LEVEL: f64 = 90.0;
const LABEL_LEVEL: f64 = 100.0;
/// Name typewriter speed, characters per second.
const TYPE_RATE: f64 = 18.0;

/// Strokes stop this far (rows, aspect-corrected) short of the stars they join (CON-A6).
const STROKE_GAP: f64 = 0.9;
const STROKE_LEVEL: f64 = 65.0;
/// Figure strokes: a faint warm grey, one braille dot wide, so the stars carry the chart
/// (ADR-0017).
const STROKE_ROLE: Role = Role::Ramp(Ramp::Starlight);
/// A figure star is a point whose radius (px) steps down with magnitude through
/// `STAR_TIERS` (`(brightest magnitude, radius)`; fainter than all is `STAR_R_RANGE.0`)
/// (CON-A4): a tiny plus (a pixel and its four neighbours, like a star's diffraction
/// cross) to 2.5th magnitude, one pixel below (ADR-0017: points).
const STAR_TIERS: [(f64, f64); 1] = [(2.5, 1.0)];
const STAR_R_RANGE: (f64, f64) = (0.5, 1.0);
/// Below this radius a star is one pixel, else a plus.
const STAR_ONE_PX: f64 = 0.75;
/// Text (labels, name, epithet, graticule labels) shows only while the chart's fade is at
/// least this: text colours keep a floor, so they would linger after the stars have gone.
const TEXT_FADE: f64 = 0.25;
/// Stars this bright (magnitude) scintillate: their colour temperature flickers by
/// ±`TWINKLE` at 1–3 Hz (CON-A11).
const BLOOM_MAG: f64 = 1.5;
const TWINKLE: f64 = 0.12;
/// A meteor every `METEOR_EVERY` s of a chart's show from `METEOR_FROM` s, crossing
/// `METEOR_LEN` rows in `METEOR_SECS` s along a hashed path, head bright, tail fading
/// (CON-A11). Meteors are real sky events; their times and paths are decorative.
const METEOR_FROM: f64 = 5.0;
const METEOR_EVERY: f64 = 3.5;
const METEOR_SECS: f64 = 0.7;
const METEOR_LEN: f64 = 9.0;
const METEOR_ROLE: Role = Role::Ramp(Ramp::Starlight);
/// The graticule is drawn only from `GRID_MIN_WIDTH` columns: on smaller charts it would
/// crowd the figure.
const GRID_MIN_WIDTH: i32 = 120;
/// Graticule: at most this many lines across the frame each way, from these steps
/// (declination in degrees; right ascension in minutes of time) (CON-A5).
const GRID_LINES: f64 = 4.0;
const DEC_STEPS: [f64; 6] = [2.0, 5.0, 10.0, 15.0, 20.0, 30.0];
const RA_STEPS_MIN: [f64; 6] = [10.0, 15.0, 20.0, 30.0, 60.0, 120.0];
const GRID_SAMPLES: usize = 96;
const GRID_LEVEL: f64 = 30.0;
const GRID_LABEL_LEVEL: f64 = 90.0;
/// A line's far side of the tangent plane (cos c below this) is not drawn.
const MIN_COS_C: f64 = 0.2;
/// Graticule labels per axis.
const MAX_LABELS: usize = 8;
/// Simulated seconds over which one chart is built ahead (CON-A10), well inside a 12 s
/// show: the next chart is always ready before it is due.
const BUILD_SECS: f64 = 4.0;

/// A catalogue star: J2000 right ascension and declination in degrees, visual magnitude
/// and B–V colour index (Hipparcos).
struct CatStar {
    name: &'static str,
    ra: f64,
    dec: f64,
    mag: f64,
    bv: f64,
}

/// `ra` given as h, m, s for fidelity to the catalogue.
const fn star(
    name: &'static str,
    (h, m, s): (f64, f64, f64),
    dec: f64,
    mag: f64,
    bv: f64,
) -> CatStar {
    CatStar {
        name,
        ra: (h + m / 60.0 + s / 3600.0) * 15.0,
        dec,
        mag,
        bv,
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
            star("Betelgeuse", (5.0, 55.0, 10.3), 7.407, 0.42, 1.85), // α Ori
            star("Meissa", (5.0, 35.0, 8.3), 9.934, 3.39, -0.18),     // λ Ori, the head
            star("Bellatrix", (5.0, 25.0, 7.9), 6.350, 1.64, -0.22),  // γ Ori
            star("Mintaka", (5.0, 32.0, 0.4), -0.299, 2.23, -0.22),   // δ Ori, belt (west)
            star("Alnilam", (5.0, 36.0, 12.8), -1.202, 1.69, -0.18),  // ε Ori, belt
            star("Alnitak", (5.0, 40.0, 45.5), -1.943, 1.77, -0.21),  // ζ Ori, belt (east)
            star("Saiph", (5.0, 47.0, 45.4), -9.670, 2.06, -0.17),    // κ Ori
            star("Rigel", (5.0, 14.0, 32.3), -8.202, 0.13, -0.03),    // β Ori
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
            star("Dubhe", (11.0, 3.0, 43.7), 61.751, 1.79, 1.07), // α UMa
            star("Merak", (11.0, 1.0, 50.5), 56.382, 2.37, -0.02), // β UMa
            star("Phecda", (11.0, 53.0, 49.8), 53.695, 2.44, 0.04), // γ UMa
            star("Megrez", (12.0, 15.0, 25.6), 57.033, 3.31, 0.08), // δ UMa
            star("Alioth", (12.0, 54.0, 1.7), 55.960, 1.77, -0.02), // ε UMa
            star("Mizar", (13.0, 23.0, 55.5), 54.925, 2.23, 0.02), // ζ UMa
            star("Alkaid", (13.0, 47.0, 32.4), 49.313, 1.86, -0.19), // η UMa
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (3, 0), (3, 4), (4, 5), (5, 6)],
    },
    Figure {
        name: "Cassiopeia",
        epithet: "the Queen",
        stars: &[
            star("Schedar", (0.0, 40.0, 30.4), 56.537, 2.24, 1.17), // α Cas
            star("Caph", (0.0, 9.0, 10.7), 59.150, 2.27, 0.34),     // β Cas
            star("Navi", (0.0, 56.0, 42.5), 60.717, 2.15, -0.15),   // γ Cas
            star("Ruchbah", (1.0, 25.0, 49.0), 60.235, 2.68, 0.13), // δ Cas
            star("Segin", (1.0, 54.0, 23.7), 63.670, 3.37, -0.15),  // ε Cas
        ],
        edges: &[(1, 0), (0, 2), (2, 3), (3, 4)],
    },
    Figure {
        name: "Cygnus",
        epithet: "the Swan",
        stars: &[
            star("Deneb", (20.0, 41.0, 25.9), 45.280, 1.25, 0.09), // α Cyg, the tail
            star("Sadr", (20.0, 22.0, 13.7), 40.257, 2.23, 0.67),  // γ Cyg, the breast
            star("η Cyg", (19.0, 56.0, 18.4), 35.083, 3.89, 1.02), // η Cyg, the neck
            star("Albireo", (19.0, 30.0, 43.3), 27.960, 3.05, 1.13), // β Cyg, the beak
            star("Fawaris", (19.0, 44.0, 58.5), 45.131, 2.87, -0.03), // δ Cyg, wing
            star("ι Cyg", (19.0, 29.0, 42.4), 51.730, 3.76, 0.11), // ι Cyg, wingtip
            star("Aljanah", (20.0, 46.0, 12.7), 33.970, 2.48, 1.03), // ε Cyg, wing
            star("ζ Cyg", (21.0, 12.0, 56.2), 30.227, 3.21, 0.99), // ζ Cyg, wingtip
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (1, 4), (4, 5), (1, 6), (6, 7)],
    },
    Figure {
        name: "Crux",
        epithet: "the Southern Cross",
        stars: &[
            star("Acrux", (12.0, 26.0, 35.9), -63.099, 0.76, -0.24), // α Cru, foot
            star("Gacrux", (12.0, 31.0, 10.0), -57.113, 1.64, 1.59), // γ Cru, head
            star("Mimosa", (12.0, 47.0, 43.3), -59.689, 1.25, -0.24), // β Cru, east arm
            star("Imai", (12.0, 15.0, 8.7), -58.749, 2.79, -0.19),   // δ Cru, west arm
        ],
        edges: &[(0, 1), (2, 3)],
    },
    Figure {
        name: "Scorpius",
        epithet: "the Scorpion",
        stars: &[
            star("Antares", (16.0, 29.0, 24.5), -26.432, 1.06, 1.83), // α Sco, the heart
            star("Acrab", (16.0, 5.0, 26.2), -19.806, 2.62, -0.07),   // β¹ Sco, claw
            star("Dschubba", (16.0, 0.0, 20.0), -22.622, 2.29, -0.12), // δ Sco, head
            star("Fang", (15.0, 58.0, 51.1), -26.114, 2.89, -0.19),   // π Sco, claw
            star("Alniyat", (16.0, 21.0, 11.3), -25.593, 2.88, 0.13), // σ Sco
            star("Paikauhale", (16.0, 35.0, 53.0), -28.216, 2.82, -0.25), // τ Sco
            star("Larawag", (16.0, 50.0, 9.8), -34.293, 2.29, 1.15),  // ε Sco
            star("μ¹ Sco", (16.0, 51.0, 52.2), -38.047, 3.00, -0.20), // μ¹ Sco
            star("ζ² Sco", (16.0, 54.0, 35.0), -42.362, 3.62, 1.37),  // ζ² Sco
            star("η Sco", (17.0, 12.0, 9.2), -43.239, 3.33, 0.41),    // η Sco
            star("Sargas", (17.0, 37.0, 19.1), -42.998, 1.86, 0.40),  // θ Sco
            star("ι¹ Sco", (17.0, 47.0, 35.1), -40.127, 2.99, 0.51),  // ι¹ Sco
            star("Girtab", (17.0, 42.0, 29.3), -39.030, 2.39, -0.17), // κ Sco
            star("Shaula", (17.0, 33.0, 36.5), -37.104, 1.62, -0.22), // λ Sco, the sting
            star("Lesath", (17.0, 30.0, 45.8), -37.296, 2.69, -0.22), // υ Sco, the sting
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

/// Brightest to faintest; picked by magnitude bands (≤1.5, ≤2.5, ≤3.3, fainter).
const STAR_GLYPHS: [Glyph; 4] = [
    Glyph::new('★', '@'),
    Glyph::new('✦', '*'),
    Glyph::new('✧', '+'),
    Glyph::new('·', '.'),
];
/// Background stars stay small so they never read as figure stars.
const BG_GLYPHS: [Glyph; 2] = [Glyph::ascii('.'), Glyph::new('·', '.')];
const PEN: Glyph = Glyph::new('•', 'o');

/// Rows of the chart area at `size`: all but the name block (CON-A7).
fn chart_rows(size: Size) -> f64 {
    f64::from(size.height().saturating_sub(NAME_ROWS).max(1))
}

/// Chart-building units to spend on a step of `dt` s: a whole chart per [`BUILD_SECS`],
/// at least one (CON-A10).
fn build_slice(units: usize, dt: f64) -> usize {
    ((units as f64 * (dt / BUILD_SECS).clamp(0.0, 1.0)).ceil() as usize).max(1)
}

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

/// Effective temperature (K) from the B–V colour index (Ballesteros 2012), clamped to
/// [2000, 30 000] K (CON-A4).
pub(crate) fn teff(bv: f64) -> f64 {
    let t = 4600.0 * (1.0 / (0.92 * bv + 1.7) + 1.0 / (0.92 * bv + 0.62));
    if t.is_finite() && t > 0.0 {
        t.clamp(2000.0, 30_000.0)
    } else {
        30_000.0
    }
}

/// Cosine of the angular distance of `(ra, dec)` from the tangent point `(ra0, dec0)`,
/// all in degrees: the gnomonic projection's denominator.
fn cos_c(ra: f64, dec: f64, ra0: f64, dec0: f64) -> f64 {
    let (d, d0) = (dec.to_radians(), dec0.to_radians());
    d0.sin_det() * d.sin_det() + d0.cos_det() * d.cos_det() * (ra - ra0).to_radians().cos_det()
}

/// Gnomonic (tangent-plane) projection of `(ra, dec)` about `(ra0, dec0)`, all in degrees.
/// Returns chart coordinates in radians with x growing to the WEST (east is left, as on a
/// sky chart seen from inside the celestial sphere) and y growing SOUTH (screen down).
pub(crate) fn project(ra: f64, dec: f64, ra0: f64, dec0: f64) -> (f64, f64) {
    let (a, d, d0) = (ra.to_radians(), dec.to_radians(), dec0.to_radians());
    let da = a - ra0.to_radians();
    let cos_c = cos_c(ra, dec, ra0, dec0);
    let xi = d.cos_det() * da.sin_det() / cos_c; // eastward
    let eta = (d0.cos_det() * d.sin_det() - d0.sin_det() * d.cos_det() * da.cos_det()) / cos_c; // northward
    (-xi, -eta)
}

/// The inverse of [`project`]: `(ra, dec)` in degrees of chart point `(x, y)`.
pub(crate) fn unproject((x, y): Point, ra0: f64, dec0: f64) -> (f64, f64) {
    let (xi, eta) = (-x, -y);
    let d0 = dec0.to_radians();
    let rho = xi.hypot(eta);
    if rho < 1e-15 {
        return (ra0, dec0);
    }
    let c = rho.atan();
    let (sc, cc) = c.sin_cos_det();
    let dec = (cc * d0.sin_det() + eta * sc * d0.cos_det() / rho).asin();
    let da = (xi * sc).atan2(rho * d0.cos_det() * cc - eta * d0.sin_det() * sc);
    ((ra0 + da.to_degrees()).rem_euclid(360.0), dec.to_degrees())
}

/// Galactic longitude and latitude `(l, b)` (deg, l in [0, 360)) of `(ra, dec)` (J2000,
/// deg).
pub(crate) fn galactic(ra: f64, dec: f64) -> (f64, f64) {
    let (d, dp) = (dec.to_radians(), NGP.1.to_radians());
    let (sin_da, cos_da) = (ra - NGP.0).to_radians().sin_cos_det();
    let sin_b = d.sin_det() * dp.sin_det() + d.cos_det() * dp.cos_det() * cos_da;
    let y = d.cos_det() * sin_da;
    let x = d.sin_det() * dp.cos_det() - d.cos_det() * dp.sin_det() * cos_da;
    let l = (L_NCP - y.atan2(x).to_degrees()).rem_euclid(360.0);
    (l, sin_b.clamp(-1.0, 1.0).asin().to_degrees())
}

/// Direction of the figure's centre: normalized mean of the stars' unit vectors,
/// which handles the RA wrap at 0h (Cassiopeia straddles it).
fn centre(stars: &[CatStar]) -> (f64, f64) {
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    for s in stars {
        let (a, d) = (s.ra.to_radians(), s.dec.to_radians());
        x += d.cos_det() * a.cos_det();
        y += d.cos_det() * a.sin_det();
        z += d.sin_det();
    }
    (y.atan2(x).to_degrees(), z.atan2(x.hypot(y)).to_degrees())
}

/// A figure's chart: its tangent point and the fit of chart radians onto the screen.
#[derive(Debug, Clone, Copy, Default)]
struct Chart {
    ra0: f64,
    dec0: f64,
    /// Rows per chart radian; x cells are `ASPECT` times narrower than rows are tall.
    scale: f64,
    /// Chart point drawn at the screen centre.
    mid: (f64, f64),
    /// Screen centre, cells.
    screen: (f64, f64),
}

impl Chart {
    /// The chart fitting `fig` into `FIT` of the chart area at `size`, preserving shape.
    fn fit(fig: &Figure, size: Size) -> Self {
        let (ra0, dec0) = centre(fig.stars);
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for s in fig.stars {
            let (x, y) = project(s.ra, s.dec, ra0, dec0);
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        let (w, h) = (f64::from(size.width()), chart_rows(size));
        let scale = (FIT * (w - 1.0) / (ASPECT * (x1 - x0)).max(1e-9))
            .min(FIT * (h - 1.0) / (y1 - y0).max(1e-9))
            .max(0.0);
        Self {
            ra0,
            dec0,
            scale,
            mid: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
            screen: ((w - 1.0) / 2.0, (h - 1.0) / 2.0),
        }
    }

    /// Screen position (cells, fractional) of `(ra, dec)`.
    fn screen_of(&self, ra: f64, dec: f64) -> (f64, f64) {
        let (x, y) = project(ra, dec, self.ra0, self.dec0);
        (
            self.screen.0 + (x - self.mid.0) * self.scale * ASPECT,
            self.screen.1 + (y - self.mid.1) * self.scale,
        )
    }

    /// Sky position `(ra, dec)` (deg) of the cell point `p`: the inverse of `screen_of`.
    fn sky_of(&self, p: Point) -> (f64, f64) {
        let s = self.scale.max(1e-9);
        let x = (p.0 - self.screen.0) / (s * ASPECT) + self.mid.0;
        let y = (p.1 - self.screen.1) / s + self.mid.1;
        unproject((x, y), self.ra0, self.dec0)
    }

    /// Graticule steps (deg) for this chart: the smallest giving at most `GRID_LINES`
    /// lines across the frame each way (CON-A5).
    fn steps(&self) -> (f64, f64) {
        let tall = (2.0 * self.screen.1 / self.scale.max(1e-9)).to_degrees();
        let wide = (2.0 * self.screen.0 / (ASPECT * self.scale.max(1e-9))).to_degrees()
            / self.dec0.to_radians().cos_det().max(0.1);
        let pick = |steps: &[f64], span: f64| {
            steps
                .iter()
                .copied()
                .find(|&s| span / s <= GRID_LINES)
                .unwrap_or(steps[steps.len() - 1])
        };
        let dec = pick(&DEC_STEPS, tall);
        let ra = pick(&RA_STEPS_MIN.map(|m| m / 4.0), wide);
        (dec, ra)
    }
}

/// The on-screen part of a graticule line through `n + 1` samples of `point(f)`,
/// `f ∈ [0, 1]`, as consecutive pairs of cell positions; samples near the tangent
/// plane's horizon are dropped.
fn polyline(
    chart: &Chart,
    point: impl Fn(f64) -> (f64, f64),
    mut segment: impl FnMut((f64, f64), (f64, f64)),
) {
    let mut prev: Option<(f64, f64)> = None;
    for k in 0..=GRID_SAMPLES {
        let (ra, dec) = point(k as f64 / GRID_SAMPLES as f64);
        let next =
            (cos_c(ra, dec, chart.ra0, chart.dec0) > MIN_COS_C).then(|| chart.screen_of(ra, dec));
        if let (Some(a), Some(b)) = (prev, next) {
            segment(a, b);
        }
        prev = next;
    }
}

/// The part of stroke `a → b` (cells) traced to `frac`, kept `STROKE_GAP` rows clear of
/// both stars, or `None` when nothing of it shows yet (CON-A6).
pub(crate) fn stroke(a: Point, b: Point, frac: f64) -> Option<(Point, Point)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx / ASPECT).hypot(dy);
    if len <= 2.0 * STROKE_GAP {
        return None;
    }
    let reach = (len * frac.clamp(0.0, 1.0)).min(len - STROKE_GAP);
    let (g0, g1) = (STROKE_GAP / len, reach / len);
    (g1 > g0).then_some((
        (a.0 + dx * g0, a.1 + dy * g0),
        (a.0 + dx * g1, a.1 + dy * g1),
    ))
}

/// A figure star's disc radius (px) at visual magnitude `mag` (CON-A4): brighter is
/// larger, within `STAR_R_RANGE`.
pub(crate) fn star_radius(mag: f64) -> f64 {
    STAR_TIERS
        .iter()
        .find(|&&(m, _)| mag <= m)
        .map_or(STAR_R_RANGE.0, |&(_, r)| r)
}

/// Where a star glyph cell `p` (cells) puts its point: the centre of the cell's top
/// pixel, whatever its size (`r` is kept for the call sites' symmetry).
fn star_centre(p: Point, _r: f64) -> Point {
    (p.0.round() + 0.5, 2.0 * p.1.round() + 0.5)
}

/// The pixels of a star of radius `r` at `c` (from [`star_centre`]): one pixel, or a plus
/// of five (CON-A4).
fn star_pixels(c: Point, r: f64, mut f: impl FnMut(i32, i32)) {
    let (x, y) = (c.0.floor() as i32, c.1.floor() as i32);
    if r < STAR_ONE_PX {
        f(x, y);
    } else {
        for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
            f(x + dx, y + dy);
        }
    }
}

/// Scintillation of star `key` at time `t`, in `[−1, 1]`: two incommensurate sines at a
/// hashed phase and 1–3 Hz rate, so no two stars pulse together (CON-A11).
pub(crate) fn twinkle(key: u64, t: f64) -> f64 {
    let h = hash01(key.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let rate = TAU * (1.0 + 2.0 * h);
    0.6 * (rate * t + TAU * h).sin_det() + 0.4 * (1.7 * rate * t + 5.0 * h).sin_det()
}

/// The meteor on chart `idx` at chart time `local` in a `w × h` frame: tail and head
/// (cells) and head level, or `None` between meteors (CON-A11). Meteor `k` starts at
/// `METEOR_FROM + k·METEOR_EVERY` s at a hashed point in the upper chart and runs
/// `METEOR_LEN` rows (aspect-corrected) down-left or down-right over `METEOR_SECS`,
/// flaring in and burning out; it never reaches the name block.
pub(crate) fn meteor(idx: usize, local: f64, w: i32, h: i32) -> Option<(Point, Point, f64)> {
    let since = local - METEOR_FROM;
    if since < 0.0 || w < 8 || h < 8 {
        return None;
    }
    let k = (since / METEOR_EVERY).floor();
    let age = (since - k * METEOR_EVERY) / METEOR_SECS;
    if age >= 1.0 {
        return None;
    }
    let key = (idx as u64) << 32 | k as u64;
    let r = |salt: u64| hash01(key.wrapping_mul(0xA24B_AED4_963E_E407) ^ salt);
    let rows = f64::from(h) - f64::from(NAME_ROWS) - 1.0;
    let start = (
        f64::from(w) * (0.15 + 0.7 * r(1)),
        rows * (0.05 + 0.3 * r(2)),
    );
    let side = if r(3) < 0.5 { -1.0 } else { 1.0 };
    let angle = 0.35 + 0.5 * r(4);
    let (s, c) = angle.sin_cos_det();
    let len = METEOR_LEN.min(rows * 0.5);
    let dir = (side * c * ASPECT, s);
    let at = |f: f64| (start.0 + dir.0 * len * f, start.1 + dir.1 * len * f);
    let head_f = ease::smoothstep(age);
    let tail_f = (head_f - 0.35).max(0.0);
    let level = 230.0 * (1.0 - age).min(age / 0.15).min(1.0);
    Some((at(tail_f), at(head_f), level))
}

fn mag_bucket(mag: f64) -> usize {
    match mag {
        m if m <= 1.5 => 0,
        m if m <= 2.5 => 1,
        m if m <= 3.3 => 2,
        _ => 3,
    }
}

/// A point in cells (fractional) or chart radians.
type Point = (f64, f64);

/// Dots position of the cell point `(x, y)` taken as a cell centre, as the star glyphs sit.
fn dots_of((x, y): Point) -> Point {
    Dots::from_cells(x + 0.5, y + 0.5)
}

#[derive(Debug, Clone)]
struct BgStar {
    x: i32,
    y: i32,
    phase: f64,
    rate: f64,
    /// Its `Dim` level where a chart keeps it, and the draw against that chart's keep
    /// share (the share is only known once the chart is built, CON-A10).
    level: u8,
    keep: f64,
    /// `Dim` level on each figure's chart, 0 where that chart leaves it out.
    levels: [u8; FIGURES.len()],
}

/// A graticule label: its cell, and a declination (deg) or right ascension (minutes).
#[derive(Debug, Clone, Copy, Default)]
enum Tick {
    #[default]
    None,
    Dec(i32, i32, i32),
    Ra(i32, i32, i32),
}

#[derive(Debug, Default)]
pub struct ConstellationMap {
    t: f64,
    width: i32,
    height: i32,
    charts: Vec<Chart>,
    /// Fitted star positions per figure.
    pos: Vec<Vec<(f64, f64)>>,
    /// Visual edge lengths per figure (rows), the tracing weights.
    lengths: Vec<Vec<f64>>,
    bg: Vec<BgStar>,
    /// Build progress per chart, in units: its background-star levels (CON-A10).
    built: [usize; FIGURES.len()],
    /// The chart and fade (bits) that `sky` and `ticks` were last drawn for.
    still: Option<(usize, u64)>,
    /// The figure's strokes and the meteor.
    dots: Dots,
    /// Figure-star points (Unicode only: ASCII draws magnitude glyphs instead).
    stars: Pixels,
    /// The graticule: Unicode only, since braille's ASCII fallback would turn faint dots
    /// into `:` as heavy as the figure.
    sky: Dots,
    ticks: [Tick; 2 * MAX_LABELS],
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl ConstellationMap {
    fn scaled(level: f64, fade: f64) -> u8 {
        (level * fade).clamp(0.0, 255.0) as u8
    }

    /// Fade in over the first half second, out over the last `FADE` seconds.
    fn fade(local: f64) -> f64 {
        (local / 0.5).min((SHOW - local) / FADE).clamp(0.0, 1.0)
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

    /// Graticule lines and their edge labels for chart `idx` (CON-A5).
    fn draw_graticule(&mut self, idx: usize, fade: f64) {
        self.ticks = [Tick::None; 2 * MAX_LABELS];
        if self.width < GRID_MIN_WIDTH {
            return;
        }
        let level = GRID_LEVEL * fade;
        if level < ink::MIN_VISIBLE_LEVEL {
            return;
        }
        let ink = Ink::new(Role::Dim, ink::level(level));
        let Some(&chart) = self.charts.get(idx) else {
            return;
        };
        // Continuous hairlines, not dotted: a dotted graticule reads as noise.
        let (dec_step, ra_step) = chart.steps();
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        // Half-spans (deg) generous enough to cross the frame.
        let half_dec = (h / chart.scale.max(1e-9)).to_degrees().min(89.0);
        let half_ra = ((w / ASPECT) / chart.scale.max(1e-9)).to_degrees()
            / chart.dec0.to_radians().cos_det().max(0.1);
        let half_ra = half_ra.min(180.0);
        let (mut n_dec, mut n_ra) = (0, 0);
        let dots = &mut self.sky;
        let mut line = |a: Point, b: Point| {
            let ((x0, y0), (x1, y1)) = (dots_of(a), dots_of(b));
            dots.line(x0, y0, x1, y1, ink);
        };
        let d0 = ((chart.dec0 - half_dec) / dec_step).ceil() as i32;
        let d1 = ((chart.dec0 + half_dec) / dec_step).floor() as i32;
        for k in d0..=d1 {
            let dec = f64::from(k) * dec_step;
            if dec.abs() >= 89.9 {
                continue;
            }
            let at = |f: f64| (chart.ra0 - half_ra + 2.0 * half_ra * f, dec);
            // Label where the parallel meets the left edge, clear of the title rows.
            let on_rows = 0.0..h - 3.0;
            let mut edge: Option<Point> = None;
            polyline(&chart, at, |a, b| {
                line(a, b);
                for p in [a, b] {
                    let leftmost = edge.is_none_or(|e| p.0 < e.0);
                    if on_rows.contains(&p.1) && p.0 >= 0.0 && leftmost {
                        edge = Some(p);
                    }
                }
            });
            if let Some((x, y)) = edge
                && x < 3.0
                && y >= 1.0
                && n_dec < MAX_LABELS
            {
                self.ticks[n_dec] = Tick::Dec(1, y.round() as i32, dec.round() as i32);
                n_dec += 1;
            }
        }
        let r0 = ((chart.ra0 - half_ra) / ra_step).ceil() as i32;
        let r1 = ((chart.ra0 + half_ra) / ra_step).floor() as i32;
        for k in r0..=r1 {
            let ra = f64::from(k) * ra_step;
            let at = |f: f64| {
                let dec = chart.dec0 - half_dec + 2.0 * half_dec * f;
                (ra, dec.clamp(-89.9, 89.9))
            };
            // Label where the meridian meets the top edge, clear of the right corner.
            let on_cols = 2.0..w - 8.0;
            let mut top: Option<Point> = None;
            polyline(&chart, at, |a, b| {
                line(a, b);
                for p in [a, b] {
                    let highest = top.is_none_or(|e| p.1 < e.1);
                    if on_cols.contains(&p.0) && p.1 >= 0.0 && highest {
                        top = Some(p);
                    }
                }
            });
            if let Some((x, y)) = top
                && y < 1.5
                && n_ra < MAX_LABELS
            {
                let minutes = (ra.rem_euclid(360.0) * 4.0).round() as i32;
                self.ticks[MAX_LABELS + n_ra] = Tick::Ra(x.round() as i32 + 1, 0, minutes);
                n_ra += 1;
            }
        }
    }

    /// The figure's strokes, traced to `progress` (CON-A2, CON-A6).
    fn draw_strokes(&mut self, idx: usize, progress: f64, fade: f64) {
        let (Some(pos), Some(lengths)) = (self.pos.get(idx), self.lengths.get(idx)) else {
            return;
        };
        let ink = Ink::new(STROKE_ROLE, Self::scaled(STROKE_LEVEL, fade));
        for (&(a, b), frac) in FIGURES[idx].edges.iter().zip(trace(progress, lengths)) {
            if frac <= 0.0 {
                continue;
            }
            let (pa, pb) = (pos[a as usize], pos[b as usize]);
            if let Some((s, e)) = stroke(pa, pb, frac) {
                let ((x0, y0), (x1, y1)) = (dots_of(s), dots_of(e));
                self.dots.line(x0, y0, x1, y1, ink);
            }
        }
    }

    /// Figure stars as 1–2 px points sized by magnitude in their blackbody colour (CON-A4,
    /// CON-A9), the brightest scintillating (CON-A11). While the chart fades each shrinks
    /// to one pixel.
    fn draw_stars(&mut self, idx: usize, fade: f64) {
        let Some(pos) = self.pos.get(idx) else {
            return;
        };
        if fade <= 0.05 {
            return;
        }
        for (i, (s, &p)) in FIGURES[idx].stars.iter().zip(pos).enumerate() {
            // The glyph cell's centre in pixels (one wide, two tall per cell).
            let r = if fade < 0.5 {
                STAR_R_RANGE.0
            } else {
                star_radius(s.mag)
            };
            let c = star_centre(p, r);
            // The brightest scintillate: their colour temperature flickers a little
            // (ADR-0017: points, no bloom).
            let k = if s.mag <= BLOOM_MAG && fade >= 0.5 {
                let key = (idx * 64 + i) as u64;
                teff(s.bv) * (1.0 + TWINKLE * twinkle(key, self.t))
            } else {
                teff(s.bv)
            };
            let ink = ink::blackbody(k);
            star_pixels(c, r, |x, y| self.stars.set(x, y, ink));
        }
    }

    /// The meteor showing at chart time `local`, if any, as a streak from its tail to its
    /// head (CON-A11), drawn into `dots`.
    fn draw_meteor(&mut self, idx: usize, local: f64, fade: f64) {
        let Some((tail, head, level)) = meteor(idx, local, self.width, self.height) else {
            return;
        };
        let level = level * fade;
        // Three pieces, brighter toward the head.
        for k in 0..3 {
            let (a, b) = (f64::from(k) / 3.0, f64::from(k + 1) / 3.0);
            let lerp = |f: f64| {
                (
                    tail.0 + (head.0 - tail.0) * f,
                    tail.1 + (head.1 - tail.1) * f,
                )
            };
            let l = level * (0.35 + 0.65 * b);
            if l >= ink::MIN_VISIBLE_LEVEL {
                let ((x0, y0), (x1, y1)) = (dots_of(lerp(a)), dots_of(lerp(b)));
                self.dots
                    .line(x0, y0, x1, y1, Ink::new(METEOR_ROLE, ink::level(l)));
            }
        }
    }

    /// Work units of one chart: a level per background star.
    fn chart_units(&self) -> usize {
        self.bg.len()
    }

    /// Build up to `budget` more units of chart `k` (CON-A10); returns the units spent. A
    /// background star is kept where its draw falls under the chart's galactic-latitude
    /// share.
    fn build_chart(&mut self, k: usize, budget: usize) -> usize {
        let chart = self.charts[k];
        let from = self.built[k];
        let to = self.bg.len().min(from.saturating_add(budget));
        for star in &mut self.bg[from.min(to)..to] {
            let (ra, dec) = chart.sky_of((f64::from(star.x), f64::from(star.y)));
            let b = galactic(ra, dec).1 / BG_PLANE_DEG;
            let share = BG_KEEP.0 + (BG_KEEP.1 - BG_KEEP.0) * (-b * b).exp();
            star.levels[k] = if star.keep < share { star.level } else { 0 };
        }
        self.built[k] = to.max(from);
        to.saturating_sub(from)
    }

    /// Spend one step's slice on the charts still to build, the showing one and then the
    /// next ones first, so each is complete long before it shows (CON-A10).
    fn build_ahead(&mut self, dt: f64) {
        let mut budget = build_slice(self.chart_units(), dt);
        let (idx, _) = current(self.t);
        for j in 0..FIGURES.len() {
            if budget == 0 {
                break;
            }
            budget -= self.build_chart((idx + j) % FIGURES.len(), budget);
        }
    }

    /// Finish chart `k` now: the chart about to be drawn must be whole.
    fn ensure_chart(&mut self, k: usize) {
        self.build_chart(k, usize::MAX);
    }

    /// Every chart built, as `resize` used to leave them (tests).
    #[cfg(test)]
    fn build_all(&mut self) {
        for k in 0..FIGURES.len() {
            self.ensure_chart(k);
        }
    }

    fn rasterise(&mut self) {
        let (idx, local) = current(self.t);
        let fade = Self::fade(local);
        // The background-star levels too, which `render` reads (CON-A10).
        self.ensure_chart(idx);
        // The graticule depends on the chart and its fade only, so it is redrawn only
        // while the chart fades in or out (CON-A10).
        if self.still != Some((idx, fade.to_bits())) {
            self.sky.clear();
            self.draw_graticule(idx, fade);
            self.still = Some((idx, fade.to_bits()));
        }
        self.dots.clear();
        self.stars.clear();
        self.draw_strokes(idx, local / TRACE, fade);
        self.draw_meteor(idx, local, fade);
        self.draw_stars(idx, fade);
    }

    /// Graticule labels, `+50°` and `11h30m` (Unicode only, like the graticule).
    fn draw_ticks(&self, canvas: &mut Canvas, fade: f64) {
        let ink = Ink::new(Role::Dim, Self::scaled(GRID_LABEL_LEVEL, fade));
        for tick in &self.ticks {
            let mut buf = StackText::<12>::new();
            let (x, y) = match *tick {
                Tick::None => continue,
                Tick::Dec(x, y, deg) => {
                    let _ = write!(buf, "{deg:+}°");
                    (x, y)
                }
                Tick::Ra(x, y, minutes) => {
                    let _ = write!(buf, "{}h", minutes / 60);
                    if minutes % 60 != 0 {
                        let _ = write!(buf, "{}m", minutes % 60);
                    }
                    (x, y)
                }
            };
            canvas.text(x, y, buf.as_str(), ink);
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
        self.charts = FIGURES.iter().map(|f| Chart::fit(f, size)).collect();
        self.pos = FIGURES
            .iter()
            .zip(&self.charts)
            .map(|(f, c)| f.stars.iter().map(|s| c.screen_of(s.ra, s.dec)).collect())
            .collect();
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
        // Each chart's star levels are built lazily (CON-A10): the showing one by
        // `rasterise` below, the others a slice per `update`.
        self.built = [0; FIGURES.len()];
        self.still = None;
        let (w, h) = (size.width(), size.height());
        let n = ((size.area() as f64 * BG_DENSITY) as usize).min(BG_MAX);
        self.bg = (0..n)
            .map(|_| {
                let (x, y) = (ctx.rng.random_range(0..w), ctx.rng.random_range(0..h));
                // Few bright, many faint: a steep magnitude distribution.
                let v: f64 = ctx.rng.random_range(0.0..1.0);
                let level = ink::level(45.0 + 150.0 * v * v * v);
                let keep: f64 = ctx.rng.random_range(0.0..1.0);
                BgStar {
                    x: i32::from(x),
                    y: i32::from(y),
                    phase: ctx.rng.random_range(0.0..TAU),
                    rate: ctx.rng.random_range(1.0..3.5),
                    level,
                    keep,
                    levels: [0; FIGURES.len()],
                }
            })
            .collect();
        self.dots.resize(size);
        self.sky.resize(size);
        self.stars.resize(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.build_ahead(dt);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (idx, local) = current(self.t);
        let fade = Self::fade(local);
        let unicode = canvas.mode() == GlyphMode::Unicode;
        // The graticule first, so the background stars sit on it.
        if unicode {
            self.sky.blit(canvas);
        }
        // Background sky: faint, twinkling, denser toward the galactic plane.
        for s in &self.bg {
            let level = f64::from(s.levels[idx]);
            let tw = 0.5 + 0.5 * (s.phase + s.rate * self.t).sin_det();
            if level > 0.0 && tw > 0.15 {
                let g = BG_GLYPHS[usize::from(level * tw > 60.0)];
                let ink = Ink::new(Role::Dim, ink::level(level * (0.55 + 0.45 * tw)));
                canvas.put(s.x, s.y, g, ink);
            }
        }
        self.dots.blit(canvas);
        if unicode {
            // Star cells only: the rest keep the graticule, strokes and field.
            let size = canvas.size();
            for y in 0..i32::from(size.height()) {
                for x in 0..i32::from(size.width()) {
                    let px = |py: i32| self.stars.get(x, py).unwrap_or(Ink::BLANK);
                    let (top, bottom) = (px(2 * y), px(2 * y + 1));
                    if top == Ink::BLANK && bottom == Ink::BLANK {
                        continue;
                    }
                    raster::half_block(canvas, x, y, top, bottom);
                }
            }
        }

        let Some(pos) = self.pos.get(idx) else {
            return;
        };
        let fig = &FIGURES[idx];
        if unicode && fade >= TEXT_FADE {
            self.draw_ticks(canvas, fade);
        }

        // The pen tip glows while drawing.
        if local < TRACE {
            let lengths = &self.lengths[idx];
            for (&(a, b), frac) in fig.edges.iter().zip(trace(local / TRACE, lengths)) {
                if frac > 0.0 && frac < 1.0 {
                    let (pa, pb) = (pos[a as usize], pos[b as usize]);
                    let (x, y) = (pa.0 + (pb.0 - pa.0) * frac, pa.1 + (pb.1 - pa.1) * frac);
                    let ink = Ink::new(Role::Accent, Self::scaled(255.0, fade));
                    canvas.put(x.round() as i32, y.round() as i32, PEN, ink);
                }
            }
        }

        // ASCII: stars as glyphs sized by magnitude and coloured by temperature (CON-A4).
        // Fading swaps the glyph for a dot rather than dimming the colour, which in
        // `Blackbody` would also redden it. (Unicode drew them as discs above.)
        if !unicode && fade > 0.05 {
            for (s, &(x, y)) in fig.stars.iter().zip(pos) {
                let bucket = if fade < 0.5 { 3 } else { mag_bucket(s.mag) };
                let star = ink::blackbody(teff(s.bv));
                canvas.put(
                    x.round() as i32,
                    y.round() as i32,
                    STAR_GLYPHS[bucket],
                    star,
                );
            }
        }

        if local < TRACE || fade < TEXT_FADE {
            return;
        }
        // After tracing: label the α star and type out the name beneath the figure.
        let since = local - TRACE;
        let label_ink = Ink::new(
            Role::Text,
            Self::scaled(LABEL_LEVEL * (since / 0.5).min(1.0), fade),
        );
        let bottom = pos.iter().map(|p| p.1).fold(0.0, f64::max).round() as i32;
        // Name and epithet stay a blank row above the title row: an epithet beside or
        // right above the title would read as one block of text with it (GEN-R13).
        let row = (bottom + 2).min(self.height - 4).max(0);
        let epithet_x = (self.width - fig.epithet.chars().count() as i32) / 2;

        // The α star's name beside it, on whichever side fits; its row is
        // in the chart area, above the name block (CON-A7).
        let lead = &fig.stars[0];
        let (lx, ly) = (pos[0].0.round() as i32, pos[0].1.round() as i32);
        let len = lead.name.chars().count() as i32;
        let spot = [(lx + 3, ly), (lx - 2 - len, ly)]
            .into_iter()
            .find(|&(x, y)| x >= 0 && y >= 0 && x + len <= self.width && y < row - 1);
        if let Some((x, y)) = spot {
            canvas.text(x, y, lead.name, label_ink);
        }

        let shown = (since * TYPE_RATE) as usize;
        let name_ink = Ink::new(Role::Text, Self::scaled(NAME_LEVEL, fade));
        Self::title(canvas, row, fig.name, shown, name_ink);
        let epithet_shown = shown.saturating_sub(fig.name.chars().count() + 2);
        let len = fig.epithet.chars().count();
        // Centred under the name, and dropped when it does not fit.
        let x = epithet_x;
        if epithet_shown > 0 && row + 1 < self.height && x + len as i32 <= self.width {
            let ink = Ink::new(Role::Text, Self::scaled(EPITHET_LEVEL, fade));
            for (i, ch) in fig.epithet.chars().take(len.min(epithet_shown)).enumerate() {
                canvas.put(x + i as i32, row + 1, Glyph::ascii(ch), ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use sciart_core::title::{self, Title};

    use super::*;

    fn fit(f: &Figure, size: Size) -> Vec<(f64, f64)> {
        let c = Chart::fit(f, size);
        f.stars.iter().map(|s| c.screen_of(s.ra, s.dec)).collect()
    }

    fn scene(size: Size) -> ConstellationMap {
        let mut g = ConstellationMap::default();
        g.resize(size, &mut Ctx::new(1, size));
        g.prepare();
        g.bg.clear();
        g
    }

    fn at(g: &mut ConstellationMap, t: f64, mode: GlyphMode) -> Canvas {
        g.t = t;
        g.rasterise();
        let size = Size::new(g.width as u16, g.height as u16);
        let mut c = Canvas::new(size, mode);
        g.render(&mut c);
        c
    }

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
                assert!((-0.4..2.0).contains(&s.bv), "{}", s.name);
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
            // Of the chart area: the frame less the name block's rows.
            let (w, h) = (f64::from(size.width()) - 1.0, chart_rows(size) - 1.0);
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
    fn con_a2_render_draws_no_stroke_at_start_and_all_after_trace() {
        let mut g = scene(Size::new(100, 30));
        let strokes = |c: &Canvas| {
            c.cells()
                .iter()
                .filter(|c| c.ink.role == STROKE_ROLE)
                .count()
        };
        assert_eq!(strokes(&at(&mut g, 0.0, GlyphMode::Unicode)), 0);
        let mid = strokes(&at(&mut g, 2.0, GlyphMode::Unicode));
        assert!(mid > 0);
        assert!(strokes(&at(&mut g, TRACE + 0.5, GlyphMode::Unicode)) >= mid);
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

    #[test]
    fn con_a4_stars_take_their_colour_from_b_minus_v() {
        let mut last = f64::INFINITY;
        for k in 0..=40 {
            let t = teff(-0.3 + 0.05 * f64::from(k));
            assert!(t < last || t == 2000.0, "not decreasing at {k}");
            last = t;
        }
        let t_of = |name: &str| {
            let s = FIGURES
                .iter()
                .flat_map(|f| f.stars)
                .find(|s| s.name == name)
                .expect(name);
            teff(s.bv)
        };
        for red in ["Betelgeuse", "Antares"] {
            assert!(t_of(red) < 4000.0, "{red}");
        }
        for blue in ["Rigel", "Acrux", "Shaula"] {
            assert!(t_of(blue) > 9000.0, "{blue}");
        }
        // Rendered: every figure star's cell shows its Blackbody temperature, as a glyph in
        // ASCII and as a half-block pixel in Unicode (the brightest within their
        // scintillation, ±`TWINKLE`).
        let mut g = scene(Size::new(120, 40));
        for (k, fig) in FIGURES.iter().enumerate() {
            let c = at(&mut g, k as f64 * SHOW + 4.5, GlyphMode::Unicode);
            let a = at(&mut g, k as f64 * SHOW + 4.5, GlyphMode::Ascii);
            for (s, &(x, y)) in fig.stars.iter().zip(&g.pos[k]) {
                let want = ink::blackbody(teff(s.bv));
                let tw = if s.mag <= BLOOM_MAG { TWINKLE } else { 0.0 };
                let lo = ink::blackbody(teff(s.bv) * (1.0 - tw)).level;
                let hi = ink::blackbody(teff(s.bv) * (1.0 + tw)).level;
                let ok = |i: Ink| i.role == Role::Blackbody && (lo..=hi).contains(&i.level);
                let (x, y) = (x.round() as i32, y.round() as i32);
                let cell = c.get(x, y).unwrap();
                assert!(ok(cell.ink) || ok(cell.bg), "{}: {cell:?}", s.name);
                assert_eq!(a.get(x, y).unwrap().ink, want, "{}", s.name);
            }
        }
        // Points grow with brightness, one pixel to a plus of five, and are symmetric.
        assert!(star_radius(0.0) >= star_radius(1.5) && star_radius(1.5) > star_radius(3.0));
        assert_eq!(star_radius(6.0), STAR_R_RANGE.0);
        for mag in [-0.5, 0.1, 0.5, 1.2, 1.8, 2.5, 3.3, 4.5] {
            let r = star_radius(mag);
            let mut px = Vec::new();
            star_pixels(star_centre((10.3, 7.6), r), r, |x, y| px.push((x, y)));
            let n = px.len();
            let (x0, x1) = (px.iter().map(|p| p.0).min(), px.iter().map(|p| p.0).max());
            let (y0, y1) = (px.iter().map(|p| p.1).min(), px.iter().map(|p| p.1).max());
            let (w, h) = (x1.unwrap() - x0.unwrap() + 1, y1.unwrap() - y0.unwrap() + 1);
            assert_eq!(w, h, "mag {mag}: {px:?}");
            assert!(
                (n == 1 && w == 1) || (n == 5 && w == 3),
                "mag {mag}: {n} of {w}×{h}"
            );
        }
    }

    #[test]
    fn con_a5_graticule_is_faint_and_meridians_are_straight() {
        let size = Size::new(200, 50);
        for (k, f) in FIGURES.iter().enumerate() {
            let chart = Chart::fit(f, size);
            let (_, ra_step) = chart.steps();
            let ra = (chart.ra0 / ra_step).round() * ra_step;
            let mut pts = Vec::new();
            polyline(
                &chart,
                |f| (ra, chart.dec0 - 20.0 + 40.0 * f),
                |a, _| pts.push(a),
            );
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            for p in &pts {
                let off = ((b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)).abs() / len;
                assert!(off < 0.05, "{}: meridian bends by {off}", f.name);
            }
            // Only faint Dim art besides the strokes, and labels off the bottom rows.
            let mut g = scene(size);
            let c = at(&mut g, k as f64 * SHOW + 2.0, GlyphMode::Unicode);
            for (i, cell) in c.cells().iter().enumerate() {
                if (0x2800..=0x28FF).contains(&(cell.ch as u32)) {
                    let level = f64::from(cell.ink.level);
                    let faint = cell.ink.role == Role::Dim && level <= GRID_LEVEL + 1.0;
                    let ok = faint || [STROKE_ROLE, METEOR_ROLE].contains(&cell.ink.role);
                    assert!(ok, "{}: {:?} at {i}", f.name, cell.ink);
                }
            }
            for tick in &g.ticks {
                if let Tick::Dec(_, y, _) | Tick::Ra(_, y, _) = *tick {
                    assert!((0..48).contains(&y), "{}: label on row {y}", f.name);
                }
            }
            assert!(
                g.ticks.iter().any(|t| !matches!(t, Tick::None)),
                "{}",
                f.name
            );
        }
    }

    #[test]
    fn con_a6_strokes_stop_short_of_their_stars() {
        let (a, b) = ((10.0, 5.0), (40.0, 20.0));
        let d = |p: (f64, f64), q: (f64, f64)| ((p.0 - q.0) / ASPECT).hypot(p.1 - q.1);
        let (s, e) = stroke(a, b, 1.0).unwrap();
        assert!((d(s, a) - STROKE_GAP).abs() < 1e-9 && (d(e, b) - STROKE_GAP).abs() < 1e-9);
        assert!(stroke(a, b, 0.0).is_none());
        let (_, e) = stroke(a, b, 0.5).unwrap();
        assert!((d(e, a) - 0.5 * d(a, b)).abs() < 1e-9);
        assert!(stroke(a, (11.0, 5.5), 1.0).is_none(), "too short to draw");
    }

    #[test]
    fn con_a7_the_alpha_label_never_overwrites_the_name_or_epithet() {
        // Crux at 80×24 once wrote "Acrux" over its own epithet ("the Southern Crossx").
        let sizes = [
            (40, 12),
            (60, 20),
            (80, 24),
            (100, 30),
            (120, 40),
            (160, 45),
            (200, 50),
        ];
        for (w, h) in sizes {
            let size = Size::new(w, h);
            let mut g = scene(size);
            for (k, fig) in FIGURES.iter().enumerate() {
                let c = at(
                    &mut g,
                    k as f64 * SHOW + SHOW - FADE - 0.1,
                    GlyphMode::Ascii,
                );
                let lines = c.to_lines();
                let row = lines.iter().position(|l| l.contains(fig.epithet));
                let Some(row) = row else {
                    // Dropped only when it cannot fit.
                    assert!(fig.epithet.len() > usize::from(w), "{w}x{h} {}", fig.name);
                    continue;
                };
                // The epithet stands intact, with no letter run into either end (the α
                // name may share the row only apart from it).
                let line: Vec<char> = lines[row].chars().collect();
                let at = lines[row].find(fig.epithet).unwrap();
                let start = lines[row][..at].chars().count();
                let end = start + fig.epithet.chars().count();
                let letter = |i: usize| line.get(i).is_some_and(|c| c.is_alphanumeric());
                let joined = (start > 0 && letter(start - 1)) || letter(end);
                assert!(!joined, "{w}x{h}: {:?}", lines[row]);
                // The letter-spaced name above it is intact, letter for letter.
                let name: Vec<char> = lines[row - 1].chars().collect();
                let n = fig.name.chars().count() as i32;
                let x0 = (i32::from(w) - (2 * n - 1)) / 2;
                for (i, ch) in fig.name.to_uppercase().chars().enumerate() {
                    let got = name[(x0 + 2 * i as i32) as usize];
                    assert_eq!(got, ch, "{w}x{h}: {:?}", lines[row - 1]);
                }
                // No star within a row of the name block: the block has rows of its own.
                if h >= 20 {
                    for &(_, y) in &g.pos[k] {
                        let y = y.round() as usize;
                        assert!(y + 2 < row, "{w}x{h} {}: star on row {y}", fig.name);
                    }
                }
            }
        }
    }

    #[test]
    fn con_a8_graticule_needs_room_and_the_milky_way_is_denser() {
        // Ursa Major at a width below `GRID_MIN_WIDTH`: no graticule.
        let mut g = scene(Size::new(100, 30));
        at(&mut g, SHOW + 2.0, GlyphMode::Unicode);
        assert!(g.ticks.iter().all(|t| matches!(t, Tick::None)));
        assert!((0..120).all(|y| (0..200).all(|x| !g.sky.is_lit(x, y))));
        // Sky round-trip through the chart.
        let chart = Chart::fit(&FIGURES[3], Size::new(200, 50));
        let (ra, dec) = chart.sky_of(chart.screen_of(310.0, 40.0));
        assert!(
            (ra - 310.0).abs() < 1e-9 && (dec - 40.0).abs() < 1e-9,
            "{ra} {dec}"
        );
        // Galactic coordinates: the pole is b = +90°, Deneb is (84.28°, +2.00°) and Sgr A*
        // the origin.
        assert!((galactic(NGP.0, NGP.1).1 - 90.0).abs() < 1e-9);
        let (l, b) = galactic(310.358, 45.280);
        assert!(
            (l - 84.28).abs() < 0.05 && (b - 2.00).abs() < 0.05,
            "{l} {b}"
        );
        let (l, b) = galactic(266.405, -28.936);
        assert!((l.min(360.0 - l)).abs() < 0.05 && b.abs() < 0.05, "{l} {b}");
        // Cygnus (in the Milky Way) keeps more background stars than Ursa Major (b ≈ 60°).
        let size = Size::new(200, 50);
        let mut g = ConstellationMap::default();
        g.resize(size, &mut Ctx::new(1, size));
        g.prepare();
        g.build_all();
        let shown = |k: usize| g.bg.iter().filter(|s| s.levels[k] > 0).count();
        assert!(
            shown(3) > 2 * shown(1),
            "Cygnus {} vs Ursa Major {}",
            shown(3),
            shown(1)
        );
    }

    #[test]
    fn con_a8_there_is_no_milky_way_wash() {
        // Cygnus lies in the Milky Way: at full show no pixel carries a glow, only points,
        // the graticule, strokes and text (ADR-0017).
        let mut g = scene(Size::new(200, 50));
        let c = at(&mut g, 3.0 * SHOW + 6.0, GlyphMode::Unicode);
        let lit_bg = c.cells().iter().filter(|c| c.bg != Ink::BLANK).count();
        let stars = FIGURES[3].stars.len();
        assert!(lit_bg <= 4 * stars, "{lit_bg} cells with a background");
    }

    #[test]
    fn con_a9_figure_stars_are_small_points() {
        // Every figure star is at most a plus of five: no bloom round even the brightest.
        let size = Size::new(200, 50);
        let mut g = scene(size);
        for (k, fig) in FIGURES.iter().enumerate() {
            at(&mut g, k as f64 * SHOW + 6.0, GlyphMode::Unicode);
            let lit: usize = (0..100)
                .flat_map(|y| (0..200).map(move |x| (x, y)))
                .filter(|&(x, y)| g.stars.get(x, y).is_some_and(|i| i != Ink::BLANK))
                .count();
            assert!(lit <= 5 * fig.stars.len(), "{}: {lit} px", fig.name);
            for (s, &p) in fig.stars.iter().zip(&g.pos[k]) {
                let r = star_radius(s.mag);
                assert!(r <= 1.0, "{}", s.name);
                let c = star_centre(p, r);
                for side in [-1.0, 1.0] {
                    let x = (c.0 + side * (r + 1.2)).floor() as i32;
                    let y = c.1.floor() as i32;
                    let ink = g.stars.get(x, y).unwrap_or(Ink::BLANK);
                    assert!(
                        ink == Ink::BLANK || ink.role == Role::Blackbody,
                        "{}: {ink:?}",
                        s.name
                    );
                }
            }
        }
        // Figure lines are the faint grey of ADR-0017; labels stay dim.
        assert_eq!(STROKE_ROLE, Role::Ramp(Ramp::Starlight));
        const { assert!(STROKE_LEVEL <= 70.0 && NAME_LEVEL <= 150.0 && EPITHET_LEVEL <= NAME_LEVEL) };
    }

    #[test]
    fn con_a7_text_leaves_with_the_stars() {
        // At a hand-over (fade ≈ 0) no text lingers: every chart ends and starts dark, so
        // no half-drawn frame shows a name without its stars (M14 critic).
        let size = Size::new(120, 40);
        let mut g = scene(size);
        let (y, _) = title::span(size, &Title::new(META.title, None)).unwrap();
        for k in 1..=FIGURES.len() {
            let c = at(&mut g, k as f64 * SHOW - 0.01, GlyphMode::Unicode);
            let lines = c.to_lines();
            for (row, line) in lines.iter().enumerate() {
                if row as i32 == y {
                    continue;
                }
                assert!(
                    !line.chars().any(char::is_alphanumeric),
                    "chart {k} row {row}: {line:?}"
                );
            }
        }
    }

    #[test]
    fn con_a11_stars_scintillate_and_meteors_cross_the_chart() {
        // Scintillation stays in [−1, 1], varies over a second and differs between stars.
        for key in 0..20 {
            let v: Vec<f64> = (0..60).map(|i| twinkle(key, f64::from(i) / 60.0)).collect();
            assert!(v.iter().all(|x| x.abs() <= 1.0 + 1e-12));
            let (lo, hi) = v
                .iter()
                .fold((1.0f64, -1.0f64), |(a, b), &x| (a.min(x), b.max(x)));
            assert!(hi - lo > 0.3, "{key}: {lo}..{hi}");
        }
        assert!((twinkle(1, 0.3) - twinkle(2, 0.3)).abs() > 1e-6);
        // A meteor every few seconds of each show after the trace, above the name block.
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            for idx in 0..FIGURES.len() {
                let mut seen = 0;
                for i in 0..(SHOW * 60.0) as i32 {
                    let local = f64::from(i) / 60.0;
                    if let Some((tail, head, level)) = meteor(idx, local, w, h) {
                        assert!(local >= METEOR_FROM);
                        assert!((0.0..=255.0).contains(&level));
                        for p in [tail, head] {
                            assert!(
                                p.1 >= 0.0 && p.1 < f64::from(h - i32::from(NAME_ROWS)),
                                "{p:?}"
                            );
                        }
                        seen += 1;
                    }
                }
                assert!(seen >= 40, "{w}x{h} {idx}: {seen} frames with a meteor");
            }
        }
        assert!(meteor(0, 1.0, 120, 40).is_none() && meteor(0, 5.0, 4, 4).is_none());
        // Visible motion after the trace: the bright stars' colour and the field twinkle.
        let mut g = scene(Size::new(120, 40));
        let a = at(&mut g, 7.0, GlyphMode::Unicode);
        let b = at(&mut g, 7.3, GlyphMode::Unicode);
        let changed = a
            .cells()
            .iter()
            .zip(b.cells())
            .filter(|(x, y)| x != y)
            .count();
        // Without the field (cleared here), only the brightest stars' points move.
        assert!(changed >= 3, "{changed} cells changed");
    }

    #[test]
    fn con_gen_r13_names_stay_off_the_title_row() {
        for size in [Size::new(80, 24), Size::new(40, 12), Size::new(30, 6)] {
            let mut g = scene(size);
            let (y, _) = title::span(size, &Title::new(META.title, None)).unwrap();
            for (k, fig) in FIGURES.iter().enumerate() {
                // Fully typed out, before the fade. Unicode, whose star glyphs are not letters.
                let c = at(
                    &mut g,
                    k as f64 * SHOW + SHOW - FADE - 0.1,
                    GlyphMode::Unicode,
                );
                let lines = c.to_lines();
                let bottom = &lines[y as usize];
                assert!(
                    !bottom.chars().any(char::is_alphanumeric),
                    "{size:?} {}: {bottom:?}",
                    fig.name
                );
                if size.width() == 80 {
                    // A blank row between the epithet and the title, so name, epithet
                    // and title do not stack into one block of text.
                    let row = lines.iter().position(|l| l.contains(fig.epithet));
                    assert!(
                        row.is_some_and(|r| r + 2 <= y as usize),
                        "{}: {row:?}",
                        fig.name
                    );
                }
            }
        }
    }

    #[test]
    fn con_a10_charts_built_ahead_match_charts_built_in_resize() {
        let size = Size::new(120, 40);
        // A whole cycle of the six charts and into the next, fine and coarse steps,
        // and a resize half a second before a hand-over.
        for dt in [SIM_DT, 0.25] {
            let (mut lazy, mut eager) = (ConstellationMap::default(), ConstellationMap::default());
            let (mut c1, mut c2) = (Ctx::new(5, size), Ctx::new(5, size));
            lazy.resize(size, &mut c1);
            lazy.prepare();
            eager.resize(size, &mut c2);
            eager.prepare();
            eager.build_all();
            assert!(
                lazy.built.iter().filter(|&&b| b > 0).count() == 1,
                "resize builds one"
            );
            let steps = (80.0 / dt) as usize;
            for i in 0..steps {
                if i == (35.5 / dt) as usize {
                    let other = Size::new(90, 30);
                    for (g, c) in [(&mut lazy, &mut c1), (&mut eager, &mut c2)] {
                        g.resize(other, c);
                        g.prepare();
                        g.resize(size, c);
                        g.prepare();
                    }
                    eager.build_all();
                }
                lazy.update(dt, &mut c1);
                lazy.prepare();
                eager.update(dt, &mut c2);
                eager.prepare();
                if i % 7 == 0 {
                    let (mut a, mut b) = (
                        Canvas::new(size, GlyphMode::Unicode),
                        Canvas::new(size, GlyphMode::Unicode),
                    );
                    lazy.render(&mut a);
                    eager.render(&mut b);
                    assert!(a.cells() == b.cells(), "dt {dt}, t = {:.2}", lazy.t);
                }
            }
            // Built ahead: every chart is complete long before the cycle ends.
            let units = lazy.chart_units();
            assert!(lazy.built.iter().all(|&b| b == units), "{:?}", lazy.built);
        }
    }

    #[test]
    fn con_a10_update_builds_at_most_one_slice() {
        let size = Size::new(200, 50);
        let mut g = ConstellationMap::default();
        let mut ctx = Ctx::new(1, size);
        g.resize(size, &mut ctx);
        g.prepare();
        let units = g.chart_units();
        let before: usize = g.built.iter().sum();
        g.update(SIM_DT, &mut ctx);
        g.prepare();
        let after: usize = g.built.iter().sum();
        assert_eq!(after - before, build_slice(units, SIM_DT));
        assert!(build_slice(units, SIM_DT) * 240 >= units, "a chart in 4 s");
    }
}
