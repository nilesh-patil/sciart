//! A star chart tracing and naming one constellation at a time (SPEC-002 `constellation_map`).
//!
//! Star positions are real J2000 RA/Dec, gnomonically projected about each figure's centre
//! and drawn as on a sky chart: north up, east to the LEFT (RA increases leftward). Stars
//! take their colour from their B–V index, and the brightest bloom; the figure is traced
//! in braille strokes over a faint RA/Dec graticule and, near the galactic plane, the
//! Milky Way as faint braille dust. Strokes, graticule, dust and blooms are rasterised in
//! `update` (the dust's dots are chosen in `resize`); `render` blits them and draws stars
//! and text.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
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
const BG_DENSITY: f64 = 0.12;
const BG_MAX: usize = 4000;
const BG_KEEP: (f64, f64) = (0.25, 1.0);
const BG_PLANE_DEG: f64 = 12.0;
/// Milky Way dust (CON-A8): each braille dot is lit with probability [`dust`]`(l, b)`, at
/// most `DUST_PEAK`, in `Dim` from `DUST_LEVEL.0` on the fringe to `DUST_LEVEL.1` in the
/// densest clumps. A density of dots rather than a filled background, so the band thins
/// out instead of ending at an edge, and fades by losing dots, not by dimming.
const DUST_PEAK: f64 = 0.3;
const DUST_LEVEL: (f64, f64) = (70.0, 170.0);
/// Star clouds: two octaves of value noise on `CLUMP_DEG` lattices in galactic (l, b),
/// weighted `CLUMP_WEIGHT`, scale the density between `CLUMP_FLOOR` and 1. The finer
/// octave gives structure to charts a few degrees across (Crux).
const CLUMP_DEG: [f64; 2] = [6.0, 2.0];
const CLUMP_WEIGHT: [f64; 2] = [0.6, 0.4];
const CLUMP_FLOOR: f64 = 0.3;
const CLUMP_SALT: u64 = 0x6D69_6C6B_7977_6179;
/// Per-chart salt for the dust cells' hashes.
const DUST_SALT: u64 = 0x9E37_79B9_7F4A_7C15;
/// The Great Rift (stylised): a dark lane at b ≈ `RIFT_B` from l = `RIFT_L.0` to
/// `RIFT_L.1` (Aquila to Cygnus), `RIFT_HALF` deg half-width, taking `LANE_DEPTH` of the
/// dust.
const RIFT_L: (f64, f64) = (25.0, 88.0);
const RIFT_B: f64 = 2.0;
const RIFT_HALF: f64 = 2.2;
/// The Coalsack beside Crux: centre (l, b) (from RA 12h50m, Dec −62.5°) and radius, deg.
const COALSACK: (f64, f64, f64) = (302.8, 0.4, 3.0);
const LANE_DEPTH: f64 = 0.85;
/// North galactic pole, J2000 (deg), and the galactic longitude of the north celestial
/// pole (IAU 1958 system, J2000 values).
const NGP: (f64, f64) = (192.859_48, 27.128_25);
const L_NCP: f64 = 122.931_92;
/// Name typewriter speed, characters per second.
const TYPE_RATE: f64 = 18.0;

/// Strokes stop this far (rows, aspect-corrected) short of the stars they join (CON-A6).
const STROKE_GAP: f64 = 0.9;
const STROKE_LEVEL: f64 = 210.0;
/// Stars this bright (magnitude) bloom: a soft Gaussian of half-block pixels round the
/// glyph, `σ = BLOOM_SIGMA` px, peaking at `BLOOM_PEAK` (less by up to 30 % toward
/// `BLOOM_MAG`), in `Ice` for B−V below `BLOOM_BLUE_BV` and `Ember` above (false colour
/// close to the star's own), and cut below `BLOOM_MIN`.
const BLOOM_MAG: f64 = 1.5;
const BLOOM_BLUE_BV: f64 = 0.4;
const BLOOM_PEAK: f64 = 0.7;
const BLOOM_SIGMA: f64 = 1.3;
const BLOOM_MIN: f64 = 0.12;
/// The graticule is drawn one dot in `GRID_STRIDE`, and only from `GRID_MIN_WIDTH`
/// columns: on smaller charts it would crowd the figure.
const GRID_STRIDE: f64 = 3.0;
const GRID_MIN_WIDTH: i32 = 120;
/// Graticule: at most this many lines across the frame each way, from these steps
/// (declination in degrees; right ascension in minutes of time) (CON-A5).
const GRID_LINES: f64 = 6.0;
const DEC_STEPS: [f64; 6] = [2.0, 5.0, 10.0, 15.0, 20.0, 30.0];
const RA_STEPS_MIN: [f64; 6] = [10.0, 15.0, 20.0, 30.0, 60.0, 120.0];
const GRID_SAMPLES: usize = 96;
const GRID_LEVEL: f64 = 34.0;
const GRID_LABEL_LEVEL: f64 = 90.0;
/// A line's far side of the tangent plane (cos c below this) is not drawn.
const MIN_COS_C: f64 = 0.2;
/// Graticule labels per axis.
const MAX_LABELS: usize = 8;

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

/// Probability that a braille dot at galactic `(l, b)` is Milky Way dust (CON-A8): a
/// Gaussian band `exp(−(b/BG_PLANE_DEG)²)`, broken into star clouds and cut by the Great
/// Rift and the Coalsack. A stylised picture of the band's surface brightness, not a map.
pub(crate) fn dust(l: f64, b: f64) -> f64 {
    let band = (-(b / BG_PLANE_DEG).powi(2)).exp();
    let noise = CLUMP_DEG
        .iter()
        .zip(CLUMP_WEIGHT)
        .enumerate()
        .map(|(k, (&deg, w))| w * value_noise(l, b, deg, CLUMP_SALT + k as u64))
        .sum::<f64>();
    let clump = CLUMP_FLOOR + (1.0 - CLUMP_FLOOR) * noise;
    // Dark lanes: soft-edged, so the dust thins into them rather than stopping.
    let along = ease::smoothstep((l - RIFT_L.0) / 8.0) * ease::smoothstep((RIFT_L.1 - l) / 8.0);
    let rift = along * ease::smoothstep(1.0 - (b - RIFT_B).abs() / RIFT_HALF);
    let dl = (l - COALSACK.0 + 180.0).rem_euclid(360.0) - 180.0;
    let sack = ease::smoothstep(1.0 - dl.hypot(b - COALSACK.1) / COALSACK.2);
    DUST_PEAK * band * clump * (1.0 - LANE_DEPTH * rift.max(sack))
}

/// Value noise in `[0, 1]` at galactic `(l, b)` on a `deg` lattice (360/`deg` a whole
/// number, so it wraps in l): smoothstep-blended hashes of the lattice cell's corners.
fn value_noise(l: f64, b: f64, deg: f64, salt: u64) -> f64 {
    let (u, v) = (l.rem_euclid(360.0) / deg, (b + 90.0) / deg);
    let (i, j) = (u.floor(), v.floor());
    let n = (360.0 / deg).round() as i32;
    let corner = |di: i32, dj: i32| {
        let key = cell_key((i as i32 + di).rem_euclid(n), j as i32 + dj);
        hash01(salt ^ key)
    };
    let (fu, fv) = (ease::smoothstep(u - i), ease::smoothstep(v - j));
    ease::lerp(
        ease::lerp(corner(0, 0), corner(1, 0), fu),
        ease::lerp(corner(0, 1), corner(1, 1), fu),
        fv,
    )
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
    /// Milky Way dust density per chart and cell, chart-major: [`dust`] over `DUST_PEAK`,
    /// as 0–255 (one byte a cell keeps six charts small at any size, TEST-R7).
    dust: Vec<u8>,
    /// The figure's strokes.
    dots: Dots,
    /// Blooms round the brightest stars (Unicode only: a half-block glow has no ASCII
    /// form that would not read as a figure star).
    bloom: Pixels,
    /// The graticule and the Milky Way dust: Unicode only, since braille's ASCII fallback
    /// would turn faint dots into `:` as heavy as the figure. (In ASCII the denser star
    /// field alone marks the Milky Way.)
    sky: Dots,
    ticks: [Tick; 2 * MAX_LABELS],
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
        let (dec_step, ra_step) = chart.steps();
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        // Half-spans (deg) generous enough to cross the frame.
        let half_dec = (h / chart.scale.max(1e-9)).to_degrees().min(89.0);
        let half_ra = ((w / ASPECT) / chart.scale.max(1e-9)).to_degrees()
            / chart.dec0.to_radians().cos_det().max(0.1);
        let half_ra = half_ra.min(180.0);
        let (mut n_dec, mut n_ra) = (0, 0);
        let dots = &mut self.sky;
        // One dot in `GRID_STRIDE` along the line, the spacing carried across segments.
        let mut carry = 0.0;
        let mut line = |a: Point, b: Point| {
            let ((x0, y0), (x1, y1)) = (dots_of(a), dots_of(b));
            let len = (x1 - x0).hypot(y1 - y0);
            let mut s = carry;
            while s < len {
                let f = s / len;
                dots.plot(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, ink);
                s += GRID_STRIDE;
            }
            carry = s - len;
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
        let ink = Ink::new(Role::Structure, Self::scaled(STROKE_LEVEL, fade));
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

    /// Blooms round the brightest stars (CON-A9), so they outweigh the field at a glance.
    /// Hidden while fading, like the magnitude glyphs.
    fn draw_blooms(&mut self, idx: usize, fade: f64) {
        let Some(pos) = self.pos.get(idx) else {
            return;
        };
        if fade < 0.5 {
            return;
        }
        for (s, &p) in FIGURES[idx].stars.iter().zip(pos) {
            if s.mag > BLOOM_MAG {
                continue;
            }
            let ramp = if s.bv < BLOOM_BLUE_BV {
                Ramp::Ice
            } else {
                Ramp::Ember
            };
            let peak = BLOOM_PEAK * (1.0 - 0.3 * s.mag.max(0.0) / BLOOM_MAG);
            // The glyph cell's centre in pixels (one wide, two tall per cell).
            let (ox, oy) = (p.0.round() + 0.5, 2.0 * p.1.round() + 1.0);
            for y in oy as i32 - 3..=oy as i32 + 2 {
                for x in ox as i32 - 3..=ox as i32 + 2 {
                    let (dx, dy) = (f64::from(x) + 0.5 - ox, f64::from(y) + 0.5 - oy);
                    let v = peak * (-(dx * dx + dy * dy) / (2.0 * BLOOM_SIGMA.powi(2))).exp();
                    if v >= BLOOM_MIN {
                        self.bloom.set(x, y, ink::ramp(ramp, v));
                    }
                }
            }
        }
    }

    /// Chart `idx`'s Milky Way dust (CON-A8). One hash per cell gives each of its eight
    /// dots a byte `u`; a dot is dust while `u < 256·p·fade`, `p` the cell's [`dust`]
    /// probability, so the dust is fixed from frame to frame and fading only drops dots.
    fn draw_dust(&mut self, idx: usize, fade: f64) {
        let w = self.width as usize;
        let cells = w * self.height as usize;
        let salt = (idx as u64 + 1).wrapping_mul(DUST_SALT);
        let Some(chart) = self.dust.get(idx * cells..(idx + 1) * cells) else {
            return;
        };
        for (i, &q) in chart.iter().enumerate() {
            if q == 0 {
                continue;
            }
            let share = f64::from(q) / 255.0;
            let bar = (256.0 * DUST_PEAK * share * fade) as u64;
            let ink = Ink::new(
                Role::Dim,
                ink::level(ease::lerp(DUST_LEVEL.0, DUST_LEVEL.1, share)),
            );
            let (cx, cy) = ((i % w) as i32, (i / w) as i32);
            let bytes = SplitMix64::mix(salt ^ cell_key(cx, cy));
            for j in 0..8 {
                if (bytes >> (8 * j)) & 0xFF < bar {
                    let (dx, dy) = (j % Dots::CELL_W, j / Dots::CELL_W);
                    self.sky
                        .set(cx * Dots::CELL_W + dx, cy * Dots::CELL_H + dy, ink);
                }
            }
        }
    }

    /// Chart `k`'s dust density per cell, appended to `self.dust`; none on the title row.
    fn build_dust(&mut self, k: usize) {
        let chart = self.charts[k];
        for cy in 0..self.height {
            for cx in 0..self.width {
                let (ra, dec) = chart.sky_of((f64::from(cx) + 0.5, f64::from(cy) + 0.5));
                let (l, b) = galactic(ra, dec);
                let q = if cy < self.height - 1 {
                    ink::level(255.0 * dust(l, b) / DUST_PEAK)
                } else {
                    0
                };
                self.dust.push(q);
            }
        }
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        self.sky.clear();
        self.bloom.clear();
        let (idx, local) = current(self.t);
        let fade = Self::fade(local);
        self.draw_dust(idx, fade);
        self.draw_graticule(idx, fade);
        self.draw_strokes(idx, local / TRACE, fade);
        self.draw_blooms(idx, fade);
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
        self.dust.clear();
        for k in 0..FIGURES.len() {
            self.build_dust(k);
        }
        let (w, h) = (size.width(), size.height());
        let n = ((size.area() as f64 * BG_DENSITY) as usize).min(BG_MAX);
        let charts = &self.charts;
        self.bg = (0..n)
            .map(|_| {
                let (x, y) = (ctx.rng.random_range(0..w), ctx.rng.random_range(0..h));
                // Few bright, many faint: a steep magnitude distribution.
                let v: f64 = ctx.rng.random_range(0.0..1.0);
                let level = ink::level(30.0 + 110.0 * v * v * v);
                let keep: f64 = ctx.rng.random_range(0.0..1.0);
                let levels = std::array::from_fn(|k| {
                    let (ra, dec) = charts[k].sky_of((f64::from(x), f64::from(y)));
                    let b = galactic(ra, dec).1 / BG_PLANE_DEG;
                    let share = BG_KEEP.0 + (BG_KEEP.1 - BG_KEEP.0) * (-b * b).exp();
                    if keep < share { level } else { 0 }
                });
                BgStar {
                    x: i32::from(x),
                    y: i32::from(y),
                    phase: ctx.rng.random_range(0.0..TAU),
                    rate: ctx.rng.random_range(1.0..3.5),
                    levels,
                }
            })
            .collect();
        self.dots.resize(size);
        self.sky.resize(size);
        self.bloom.resize(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        let (idx, local) = current(self.t);
        let fade = Self::fade(local);
        let unicode = canvas.mode() == GlyphMode::Unicode;
        // Milky Way dust and graticule first, so the background stars sit on them.
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
            self.bloom.blit(canvas, |_, _, _, _| None);
        }

        let Some(pos) = self.pos.get(idx) else {
            return;
        };
        let fig = &FIGURES[idx];
        if unicode {
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

        // Stars over the strokes, sized by magnitude and coloured by temperature (CON-A4),
        // on their bloom's glow. Fading swaps the glyph for a dot rather than dimming the
        // colour, which in `Blackbody` would also redden it.
        for (s, &(x, y)) in fig.stars.iter().zip(pos) {
            let bucket = if fade < 0.5 { 3 } else { mag_bucket(s.mag) };
            let (x, y) = (x.round() as i32, y.round() as i32);
            let glow = match self.bloom.get(x, 2 * y) {
                Some(ink) if unicode => ink,
                _ => Ink::BLANK,
            };
            if fade > 0.05 {
                let star = ink::blackbody(teff(s.bv));
                canvas.put_bg(x, y, STAR_GLYPHS[bucket], star, glow);
            }
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
        let bottom = pos.iter().map(|p| p.1).fold(0.0, f64::max).round() as i32;
        // Name and epithet stay a blank row above the title row: an epithet beside or
        // right above the title would read as one block of text with it (GEN-R13).
        let row = (bottom + 2).min(self.height - 4).max(0);
        let epithet_x = (self.width - fig.epithet.chars().count() as i32) / 2;

        // The α star's name beside it, on whichever side fits, past its bloom; its row is
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
        let name_ink = Ink::new(Role::Text, Self::scaled(255.0, fade));
        Self::title(canvas, row, fig.name, shown, name_ink);
        let epithet_shown = shown.saturating_sub(fig.name.chars().count() + 2);
        let len = fig.epithet.chars().count();
        // Centred under the name, and dropped when it does not fit.
        let x = epithet_x;
        if epithet_shown > 0 && row + 1 < self.height && x + len as i32 <= self.width {
            let ink = Ink::new(Role::Text, Self::scaled(140.0, fade));
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
                .filter(|c| c.ink.role == Role::Structure)
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
        // Rendered: every figure star's cell is Blackbody at its temperature.
        let mut g = scene(Size::new(120, 40));
        for (k, fig) in FIGURES.iter().enumerate() {
            let c = at(&mut g, k as f64 * SHOW + 6.0, GlyphMode::Unicode);
            for (s, &(x, y)) in fig.stars.iter().zip(&g.pos[k]) {
                let cell = c.get(x.round() as i32, y.round() as i32).unwrap();
                assert_eq!(cell.ink, ink::blackbody(teff(s.bv)), "{}", s.name);
            }
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
                    let faint = cell.ink.role == Role::Dim && level <= DUST_LEVEL.1 + 1.0;
                    let ok = faint || cell.ink.role == Role::Structure;
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
        // Ursa Major, far from the Milky Way, at a width below `GRID_MIN_WIDTH`: no
        // graticule, no dust.
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
        let shown = |k: usize| g.bg.iter().filter(|s| s.levels[k] > 0).count();
        assert!(
            shown(3) > 2 * shown(1),
            "Cygnus {} vs Ursa Major {}",
            shown(3),
            shown(1)
        );
    }

    /// Chart `k`'s dust dots drawn at `fade`, as a lit-dot mask.
    fn dust_at(g: &mut ConstellationMap, k: usize, fade: f64) -> Vec<bool> {
        g.sky.clear();
        g.draw_dust(k, fade);
        let d = g.sky.dot_size();
        let (w, h) = (i32::from(d.width()), i32::from(d.height()));
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| g.sky.is_lit(x, y))
            .collect()
    }

    #[test]
    fn con_a8_the_milky_way_is_dust_thinning_away_from_the_plane() {
        let size = Size::new(200, 50);
        let mut g = scene(size);
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        // Share of dots lit by |b| band, over every chart: it falls off band by band, so
        // the Milky Way thins out instead of ending at an edge.
        let (mut lit, mut all) = ([0usize; 4], [0usize; 4]);
        for k in 0..FIGURES.len() {
            let mask = dust_at(&mut g, k, 1.0);
            for cy in 0..h - 1 {
                for cx in 0..w {
                    let p = (cx as f64 + 0.5, cy as f64 + 0.5);
                    let (ra, dec) = g.charts[k].sky_of(p);
                    let bin = (galactic(ra, dec).1.abs() / 5.0) as usize;
                    if bin < 4 {
                        all[bin] += 8;
                        lit[bin] += (0..4)
                            .flat_map(|j| (0..2).map(move |i| (cy * 4 + j) * 2 * w + cx * 2 + i))
                            .filter(|&i| mask[i])
                            .count();
                    }
                }
            }
        }
        let share: Vec<f64> = (0..4)
            .map(|i| lit[i] as f64 / all[i].max(1) as f64)
            .collect();
        assert!(all.iter().all(|&n| n > 1000), "{all:?}");
        assert!(share.windows(2).all(|s| s[0] > s[1]), "{share:?}");
        assert!(share[2] > 0.0 && share[0] <= DUST_PEAK, "{share:?}");
        // None far from the plane (Ursa Major), none on the title row, and faint.
        assert!(!dust_at(&mut g, 1, 1.0).iter().any(|&l| l));
        for k in 0..FIGURES.len() {
            let mask = dust_at(&mut g, k, 1.0);
            assert!(
                !mask[8 * w * (h - 1)..].iter().any(|&l| l),
                "dust on the title row"
            );
        }
        // Fading thins the dust: the lit dots only ever grow with fade, and some show
        // until the chart has all but gone.
        for k in [2, 3, 4] {
            let mut prev = dust_at(&mut g, k, 0.1);
            assert!(prev.iter().any(|&l| l), "{}", FIGURES[k].name);
            for f in 2..=10 {
                let next = dust_at(&mut g, k, f64::from(f) / 10.0);
                assert!(prev.iter().zip(&next).all(|(&a, &b)| !a || b), "{f}");
                prev = next;
            }
        }
        // Structure, not a flat wash: on the charts inside the band (Cassiopeia, Crux),
        // the dust density varies from one 10 × 5-cell block to the next.
        for k in [2, 4] {
            let mask = dust_at(&mut g, k, 1.0);
            let blocks: Vec<f64> = (0..(h - 1) / 5)
                .flat_map(|by| (0..w / 10).map(move |bx| (bx, by)))
                .map(|(bx, by)| {
                    let n = (by * 20..by * 20 + 20)
                        .flat_map(|y| (bx * 20..bx * 20 + 20).map(move |x| y * 2 * w + x))
                        .filter(|&i| mask[i])
                        .count();
                    n as f64
                })
                .collect();
            let mean = blocks.iter().sum::<f64>() / blocks.len() as f64;
            let var = blocks.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / blocks.len() as f64;
            let cv = var.sqrt() / mean;
            assert!(cv > 0.2, "{}: variation {cv}", FIGURES[k].name);
        }
        // The Coalsack and the Great Rift take most of the dust where they lie.
        let lane_max = |b: f64| DUST_PEAK * (-(b / BG_PLANE_DEG).powi(2)).exp();
        let sack = dust(COALSACK.0, COALSACK.1);
        assert!(sack <= (1.0 - LANE_DEPTH) * lane_max(COALSACK.1) + 1e-12);
        let rift = dust(60.0, RIFT_B);
        assert!(rift <= (1.0 - LANE_DEPTH) * lane_max(RIFT_B) + 1e-12);
        // Rendered: the dust covers much of Cygnus's chart and none of Ursa Major's.
        let dusty = |c: &Canvas| {
            let braille = |c: &&Cell| (0x2801..=0x28FF).contains(&(c.ch as u32));
            let dust = |c: &&Cell| c.ink.role == Role::Dim && f64::from(c.ink.level) > GRID_LEVEL;
            c.cells().iter().filter(braille).filter(dust).count()
        };
        let c = at(&mut g, 3.0 * SHOW + 6.0, GlyphMode::Unicode);
        assert!(dusty(&c) > c.cells().len() / 4, "Cygnus dust {}", dusty(&c));
        assert_eq!(dusty(&at(&mut g, SHOW + 6.0, GlyphMode::Unicode)), 0);
    }

    #[test]
    fn con_a9_the_brightest_stars_bloom_in_their_own_hue() {
        let size = Size::new(200, 50);
        let mut g = scene(size);
        for (k, fig) in FIGURES.iter().enumerate() {
            let c = at(&mut g, k as f64 * SHOW + 6.0, GlyphMode::Unicode);
            for (s, &(x, y)) in fig.stars.iter().zip(&g.pos[k]) {
                let (x, y) = (x.round() as i32, y.round() as i32);
                // The cells either side of the glyph glow, blue or warm by B−V.
                let side = [c.get(x - 1, y), c.get(x + 1, y)];
                let lit = side
                    .iter()
                    .filter(|c| c.is_some_and(|c| c.ch == '█' || c.ch == '▀' || c.ch == '▄'))
                    .count();
                if s.mag <= BLOOM_MAG {
                    let want = if s.bv < BLOOM_BLUE_BV {
                        Ramp::Ice
                    } else {
                        Ramp::Ember
                    };
                    assert!(lit > 0, "{}: no bloom", s.name);
                    for cell in side.into_iter().flatten() {
                        if matches!(cell.ch, '▀' | '▄') {
                            assert_eq!(cell.ink.role, Role::Ramp(want), "{}", s.name);
                            assert!(f64::from(cell.ink.level) <= 255.0 * BLOOM_PEAK + 1.0);
                        }
                    }
                }
            }
        }
        // ASCII has no blooms.
        let c = at(&mut g, 6.0, GlyphMode::Ascii);
        assert!(
            c.cells()
                .iter()
                .all(|c| !matches!(c.ink.role, Role::Ramp(_)))
        );
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
}
