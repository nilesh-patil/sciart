//! The Moon through a full lunation (SPEC-002 `moon_phases`).
//!
//! An orthographic view of a Lommel–Seeliger sphere carrying a selenographic texture:
//! maria at their true places, rayed craters, and seeded craters whose relief tilts the
//! surface normal, so they cast shadows along the terminator. The Sun direction `L`
//! swings round once per synodic month; the face nods with libration, and the night side
//! glows faintly with earthshine. The disc is rasterised in `update`/`resize`; `render`
//! blits it and writes the label.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::v3;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{draw, geom, ink, rng};

pub static META: Meta = Meta {
    name: "moon_phases",
    category: Category::EarthSky,
    description: "The Moon through a full lunation: maria, rayed craters, earthshine, libration",
    title: "Moon phases · half always sunlit",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(MoonPhases::default())
}

/// One synodic month (29.53 d) in simulated seconds.
const SYNODIC: f64 = 30.0;
/// Libration: amplitudes (deg) and periods (s) of the anomalistic (27.55 d) and
/// draconic (27.21 d) months on the 30 s ≙ 29.53 d clock (MOON-A6).
const LIB_LON: (f64, f64) = (7.9, 30.0 * 27.55 / 29.53);
const LIB_LAT: (f64, f64) = (6.7, 30.0 * 27.21 / 29.53);
const STARS: usize = 140;

/// Selenographic texture: 1.875° per texel.
const TEX_W: usize = 192;
const TEX_H: usize = 96;
/// Seeded craters and their radius range (deg): exaggerated, real ones this common would
/// be sub-pixel.
const CRATERS: usize = 70;
const CRATER_DEG: (f64, f64) = (1.5, 6.0);
/// Depth-to-radius ratio of a crater bowl (simple craters are ≈ 0.2 deep per diameter).
const CRATER_DEPTH: f64 = 0.22;
/// Largest relief tilt `|∇h|` a texel may carry: steeper bowls flip single pixels
/// between full sun and shadow, which reads as speckle rather than relief.
const MAX_TILT: f64 = 0.3;
/// Albedo noise per octave: (lattice spacing in texels, ± share). The spacings divide
/// `TEX_W`, so the noise wraps seamlessly at 180°.
const NOISE: [(usize, f64); 3] = [(24, 0.07), (8, 0.05), (3, 0.025)];

/// Exposure `v = ln(1 + K·2r)/ln(1 + K·WHITE)`, capped at 1: a log stretch of the
/// reflectance (full-Moon highland `2r = 1` reads 0.85, mare 0.58), so maria stay apart
/// from highlands even at the bright limb of a crescent, where `2r` nears 2. The ink level
/// is `LIT_FLOOR + (255 − LIT_FLOOR)·v²`, which spreads the bright end the theme would
/// otherwise compress; the floor keeps the dimmest sunlit pixel at twice the brightest
/// earthshine (MOON-A5).
const EXPOSURE_K: f64 = 4.0;
const WHITE: f64 = 1.4;
const LIT_FLOOR: f64 = 60.0;
/// Sunlit pixels dimmer than this read as the terminator's dark edge and are dropped.
const MIN_LIT: f64 = 0.03;
/// A crater's shadow keeps this share of the unrelieved exposure: a grey patch, as a
/// shadow several pixels across looks, never a black hole in the lit disc.
const SHADOW_FLOOR: f64 = 0.3;
/// Relief fades in over `μ0 ∈ [0, RELIEF_MU0]` of the smooth sphere, so relief alone
/// never moves the terminator.
const RELIEF_MU0: f64 = 0.15;
/// Relief fades out toward the limb over `μ ∈ [0, RELIEF_MU]`, where craters are
/// foreshortened to a pixel or two.
const RELIEF_MU: f64 = 0.6;
/// Earthshine: a `Dim` level `EARTH_DIM·A·E(p)`, held between the visible floor and half
/// of `LIT_FLOOR`, shown while `E(p) ≥ EARTH_MIN`, so it goes out before full Moon.
/// `Dim` is a slate grey, bluish like real earthshine (earthlight is bluer than
/// sunlight). Boosted: real earthshine is ≈ 10⁻⁴ of the sunlit side.
const EARTH_DIM: f64 = 28.0;
const EARTH_MIN: f64 = 0.5;
/// Quarter, new and full are instants: each of those names owns ± this share of the
/// lunation, and the crescents and gibbous phases own the rest (MOON-A2).
const PRINCIPAL: f64 = 0.02;

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
const STAR: Glyph = Glyph::new('·', '.');
/// ASCII sunlit ramp, darkest to brightest.
const ASCII_LIT: [char; 6] = ['.', ':', '+', '*', '#', '@'];

/// A mare: centre (lon E, lat N), half-extents (deg of arc) east–west and north–south,
/// and its albedo relative to the highlands. The basalt is at half cover on the outline.
struct Mare(f64, f64, f64, f64, f64);

/// IAU centres; half-extents are half the IAU diameters (D/2 over 1737 km, in degrees),
/// so the maria cover their real share of the near side (MOON-A7). Procellarum is two
/// lobes and Frigoris a long narrow band; every outline is lobed and ragged, and
/// neighbours overlap and merge as they do on the Moon. Relative albedos follow full-Moon
/// photographs: Serenitatis and Frigoris pale, Tranquillitatis and Crisium darkest (real
/// maria ≈ 0.07 against highland ≈ 0.12–0.18).
const MARIA: [Mare; 14] = [
    Mare(-52.0, 30.0, 21.0, 20.0, 0.52), // Oceanus Procellarum, north lobe
    Mare(-50.0, 0.0, 17.0, 16.0, 0.5),   // Oceanus Procellarum, south lobe
    Mare(-15.6, 32.8, 18.9, 17.0, 0.55), // Imbrium (D 1146 km)
    Mare(17.5, 28.0, 11.1, 11.1, 0.62),  // Serenitatis (674 km)
    Mare(31.4, 8.5, 14.4, 12.0, 0.42),   // Tranquillitatis (873 km)
    Mare(59.1, 17.0, 12.2, 9.2, 0.45),   // Crisium (740 × 556 km)
    Mare(51.3, -7.8, 11.0, 15.0, 0.5),   // Fecunditatis (909 km)
    Mare(34.6, -15.2, 5.5, 5.5, 0.52),   // Nectaris (333 km)
    Mare(-16.6, -21.3, 11.8, 11.8, 0.56), // Nubium (715 km)
    Mare(-39.6, -24.4, 6.4, 6.4, 0.5),   // Humorum (389 km)
    Mare(1.4, 56.0, 40.0, 5.0, 0.64),    // Frigoris (1596 km long)
    Mare(4.1, 13.3, 4.0, 4.0, 0.55),     // Vaporum (245 km)
    Mare(-23.1, -10.0, 6.2, 6.2, 0.56),  // Cognitum (376 km)
    Mare(-30.9, 7.5, 8.5, 8.5, 0.55),    // Insularum (513 km)
];
/// Rays per rayed crater, and the spread of their lengths (share of the reach) and angular
/// widths (rad): irregular, tapering streaks rather than a regular wheel.
const RAYS: usize = 11;
const RAY_LEN: (f64, f64) = (0.35, 1.0);
const RAY_WIDTH: (f64, f64) = (0.05, 0.13);

/// A named crater: centre (lon E, lat N), radius (deg), floor albedo factor, ray reach
/// (deg, 0 = none) and ray brightness.
struct Named(f64, f64, f64, f64, f64, f64);

const NAMED: [Named; 6] = [
    Named(-11.4, -43.3, 2.5, 1.1, 40.0, 0.35), // Tycho
    Named(-20.1, 9.6, 2.8, 1.05, 16.0, 0.25),  // Copernicus
    Named(-38.0, 8.1, 1.6, 1.15, 9.0, 0.2),    // Kepler
    Named(-47.4, 23.7, 1.5, 1.45, 0.0, 0.0),   // Aristarchus, the brightest spot
    Named(-9.3, 51.6, 3.0, 0.62, 0.0, 0.0),    // Plato, dark floor
    Named(-68.3, -5.2, 4.0, 0.55, 0.0, 0.0),   // Grimaldi, dark floor
];

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
/// (MOON-A1). It places the terminator; relief only shades the day side near it.
pub(crate) fn illumination(nx: f64, ny: f64, light: [f64; 3]) -> f64 {
    let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
    v3::dot([nx, ny, nz], light)
}

/// Fraction of the visible disc that is lit, `(1 − cos 2πp)/2` (MOON-A3).
pub(crate) fn illuminated_fraction(phase: f64) -> f64 {
    (1.0 - (TAU * phase).cos_det()) / 2.0
}

/// Earth's lit fraction seen from the Moon, `(1 + cos 2πp)/2`: full Earth at new Moon
/// (MOON-A5).
pub(crate) fn earth_phase(phase: f64) -> f64 {
    (1.0 + (TAU * phase).cos_det()) / 2.0
}

/// Lommel–Seeliger reflectance `A·μ0/(μ0 + μ)` (MOON-A4): no limb darkening at full Moon.
pub(crate) fn reflectance(albedo: f64, mu0: f64, mu: f64) -> f64 {
    if mu0 <= 0.0 {
        0.0
    } else {
        albedo * mu0 / (mu0 + mu.max(1e-3))
    }
}

/// Libration (deg) at time `t`: longitude and latitude of the sub-observer point (MOON-A6).
pub(crate) fn libration(t: f64) -> (f64, f64) {
    (
        LIB_LON.0 * (TAU * t / LIB_LON.1).sin_det(),
        LIB_LAT.0 * (TAU * t / LIB_LAT.1).sin_det(),
    )
}

/// Traditional name of the phase (MOON-A2): new, first quarter, full and last quarter are
/// instants and own only ±`PRINCIPAL` of the lunation; the crescents and gibbous phases
/// fill the quarters between them.
pub(crate) fn phase_name(phase: f64) -> &'static str {
    let p = geom::wrap(phase, 1.0);
    let quarter = (p * 4.0).round();
    if (p - quarter / 4.0).abs() <= PRINCIPAL {
        NAMES[(2 * quarter as usize) % 8]
    } else {
        NAMES[2 * (p * 4.0).floor() as usize + 1]
    }
}

/// Unit vector of selenographic `(lon, lat)` (rad): x east, y north, z toward Earth.
fn body(lon: f64, lat: f64) -> [f64; 3] {
    let (so, co) = lon.sin_cos_det();
    let (sa, ca) = lat.sin_cos_det();
    [ca * so, sa, ca * co]
}

/// Smooth value noise in `[0, 1]` at texel `(x, y)` on a hashed lattice `cell` texels
/// apart, wrapping east–west (`cell` divides `TEX_W`).
fn value_noise(x: usize, y: usize, cell: usize, salt: u64) -> f64 {
    let wrap = (TEX_W / cell) as i32;
    let (fx, fy) = (x as f64 / cell as f64, y as f64 / cell as f64);
    let (i, j) = (fx.floor() as i32, fy.floor() as i32);
    let (u, v) = (ease::smoothstep(fx.fract()), ease::smoothstep(fy.fract()));
    let at = |i: i32, j: i32| rng::hash01(salt ^ rng::cell_key(i.rem_euclid(wrap), j));
    let top = ease::lerp(at(i, j), at(i + 1, j), u);
    let bottom = ease::lerp(at(i, j + 1), at(i + 1, j + 1), u);
    ease::lerp(top, bottom, v)
}

/// Rotation taking view-frame vectors to the body frame for libration `(λ, β)` (rad):
/// tilt by β about x, then turn by λ about y, so the view axis lands on `body(λ, β)`.
#[derive(Debug, Clone, Copy)]
struct Libration {
    sin_b: f64,
    cos_b: f64,
    sin_l: f64,
    cos_l: f64,
}

impl Libration {
    fn new(lon: f64, lat: f64) -> Self {
        let (sin_l, cos_l) = lon.sin_cos_det();
        let (sin_b, cos_b) = lat.sin_cos_det();
        Self {
            sin_b,
            cos_b,
            sin_l,
            cos_l,
        }
    }

    fn rotate(self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let (y, z) = (
            y * self.cos_b + z * self.sin_b,
            -y * self.sin_b + z * self.cos_b,
        );
        [
            x * self.cos_l + z * self.sin_l,
            y,
            -x * self.sin_l + z * self.cos_l,
        ]
    }
}

/// One texel of the selenographic texture.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Texel {
    /// Albedo relative to the mean highland, rays excluded.
    albedo: f32,
    /// Ray-system brightening, added to the albedo with a phase-dependent gain.
    ray: f32,
    /// Height gradient per radian east and north, its length capped at `MAX_TILT`.
    gx: f32,
    gy: f32,
}

impl Texel {
    fn lerp(self, other: Self, t: f32) -> Self {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Self {
            albedo: l(self.albedo, other.albedo),
            ray: l(self.ray, other.ray),
            gx: l(self.gx, other.gx),
            gy: l(self.gy, other.gy),
        }
    }
}

/// The selenographic texture.
#[derive(Debug, Default)]
struct Surface {
    /// Row-major `TEX_W × TEX_H`, row 0 at +90°, column 0 at −180°.
    texels: Vec<Texel>,
}

impl Surface {
    fn build(rng: &mut impl Rng) -> Self {
        let n = TEX_W * TEX_H;
        let mut albedo = vec![1.0f64; n];
        let mut ray = vec![0.0f64; n];
        let mut height = vec![0.0f64; n];
        let (_, mare) = Self::maria();
        let deg = PI / 180.0;
        let mut craters = Vec::with_capacity(CRATERS + NAMED.len());
        for &Named(lon, lat, r, floor, rays, bright) in &NAMED {
            craters.push((lon * deg, lat * deg, r * deg, floor, rays * deg, bright));
        }
        for _ in 0..CRATERS {
            let lat = rng.random_range(-1.0f64..1.0).asin();
            let lon = rng.random_range(-PI..PI);
            // More small craters than large ones.
            let u: f64 = rng.random_range(0.0..1.0);
            let r = ease::lerp(CRATER_DEG.0, CRATER_DEG.1, u * u) * deg;
            craters.push((lon, lat, r, 1.0, 0.0, 0.0));
        }
        for &(lon, lat, r, floor, rays, bright) in &craters {
            Self::around(lon, lat, 1.6 * r, &mut |i, d, _| {
                let q = d / r;
                height[i] += CRATER_DEPTH
                    * r
                    * if q < 1.0 {
                        q * q - 1.0
                    } else {
                        0.3 * (-((q - 1.0) / 0.25).powi(2)).exp()
                    };
                if q < 0.9 {
                    albedo[i] *= floor;
                }
                albedo[i] *= 1.0 + 0.08 * (-((q - 1.0) / 0.15).powi(2)).exp();
            });
            if rays > 0.0 {
                let east = [lon.cos_det(), 0.0, -lon.sin_det()];
                let north = [
                    -lat.sin_det() * lon.sin_det(),
                    lat.cos_det(),
                    -lat.sin_det() * lon.cos_det(),
                ];
                // Each ray: a direction, a length and a width, drawn once per crater.
                let streaks: [(f64, f64, f64); RAYS] = std::array::from_fn(|_| {
                    let angle = rng.random_range(-PI..PI);
                    let len = rays * rng.random_range(RAY_LEN.0..RAY_LEN.1);
                    (angle, len, rng.random_range(RAY_WIDTH.0..RAY_WIDTH.1))
                });
                Self::around(lon, lat, rays, &mut |i, d, v| {
                    let theta = v3::dot(v, north).atan2(v3::dot(v, east));
                    let near = (d / (1.2 * r)).min(1.0);
                    for &(angle, len, width) in &streaks {
                        if d < len {
                            let off = geom::wrap_angle(theta - angle) / width;
                            // Tapering: fades along its length and narrows toward the tip.
                            let fall = 1.0 - d / len;
                            let across = (-(off * off) / fall.max(0.2)).exp();
                            ray[i] += bright * across * fall * near;
                        }
                    }
                });
            }
        }
        let texels = (0..n)
            .map(|i| {
                let (x, y) = (i % TEX_W, i / TEX_W);
                // Soft albedo variation at three scales: regional highland tone, patchy
                // mare flows and fine mottle, so no region reads as a flat fill.
                let noise: f64 = NOISE
                    .iter()
                    .zip(1u64..)
                    .map(|(&(cell, share), salt)| {
                        share * (2.0 * value_noise(x, y, cell, salt << 40) - 1.0)
                    })
                    .sum();
                // Lava flows of different ages: the maria vary more than the highlands.
                let a = albedo[i] * (1.0 - mare[i]) * (1.0 + noise * (1.0 + 4.0 * mare[i]));
                let (gx, gy) = Self::gradient(&height, i);
                Texel {
                    albedo: a as f32,
                    ray: ray[i] as f32,
                    gx: gx as f32,
                    gy: gy as f32,
                }
            })
            .collect();
        Self { texels }
    }

    /// Apply `f(i, distance (rad), texel vector)` to texels within `reach` (rad) of
    /// `(lon, lat)`, visiting only its bounding box.
    fn around(lon: f64, lat: f64, reach: f64, f: &mut impl FnMut(usize, f64, [f64; 3])) {
        let c = body(lon, lat);
        let (j0, j1) = (Self::row(lat + reach), Self::row(lat - reach));
        for j in j0..=j1 {
            let tl = Self::lat_of(j);
            let span = if tl.cos_det() > reach.sin_det() {
                (reach.sin_det() / tl.cos_det()).clamp(-1.0, 1.0).asin() + 0.05
            } else {
                PI
            };
            let k = (span / (TAU / TEX_W as f64)).ceil() as i64;
            let ic = ((lon + PI) / TAU * TEX_W as f64) as i64;
            for i in ic - k..=ic + k {
                let i = i.rem_euclid(TEX_W as i64) as usize;
                let v = body(Self::lon_of(i), tl);
                let d = v3::dot(v, c).clamp(-1.0, 1.0).acos();
                if d <= reach {
                    f(j * TEX_W + i, d, v);
                }
            }
        }
    }

    /// Per texel, the basalt cover in `[0, 1]` and the deepest mare darkening
    /// (`cover · (1 − relative albedo)`) over it (MOON-A7).
    fn maria() -> (Vec<f64>, Vec<f64>) {
        let n = TEX_W * TEX_H;
        let (mut cover, mut dark) = (vec![0.0f64; n], vec![0.0f64; n]);
        let deg = PI / 180.0;
        for (k, &Mare(lon, lat, a, b, relative)) in MARIA.iter().enumerate() {
            let (lon, lat) = (lon * deg, lat * deg);
            let reach = 1.8 * a.max(b) * deg;
            // Each mare's own lobed outline, roughened by noise, so the basins are not
            // plain ellipses and neighbours run into each other.
            let seed = k as f64 * 2.4;
            Self::around(lon, lat, reach, &mut |i, _, v| {
                // Local east/north offsets of the texel from the centre, in half-extents.
                let x = (v[0] * lon.cos_det() - v[2] * lon.sin_det()) / (a * deg);
                let y = (v[1] * lat.cos_det()
                    - (v[0] * lon.sin_det() + v[2] * lon.cos_det()) * lat.sin_det())
                    / (b * deg);
                let theta = y.atan2(x);
                let lobes = 1.0
                    + 0.15 * (2.0 * theta + seed).sin_det()
                    + 0.1 * (3.0 * theta - seed).sin_det();
                let ragged = 0.3 * (value_noise(i % TEX_W, i / TEX_W, 6, 0x6d61 << 40) - 0.5);
                let q = x.hypot(y) / lobes + ragged;
                // A soft shore, at half cover on the outline `q = 1`.
                let c = 1.0 - ease::smoothstep((q - 0.75) / 0.5);
                cover[i] = cover[i].max(c);
                dark[i] = dark[i].max(c * (1.0 - relative));
            });
        }
        (cover, dark)
    }

    fn lon_of(i: usize) -> f64 {
        (i as f64 + 0.5) / TEX_W as f64 * TAU - PI
    }

    fn lat_of(j: usize) -> f64 {
        PI / 2.0 - (j as f64 + 0.5) / TEX_H as f64 * PI
    }

    fn row(lat: f64) -> usize {
        (((PI / 2.0 - lat) / PI * TEX_H as f64).floor().max(0.0) as usize).min(TEX_H - 1)
    }

    /// Central-difference height gradient per radian east and north at texel `i`, its
    /// length capped at `MAX_TILT`. The `cos lat` floor keeps the east–west slope finite
    /// near the poles, which libration brings into view.
    fn gradient(h: &[f64], i: usize) -> (f64, f64) {
        let (x, y) = (i % TEX_W, i / TEX_W);
        let step = TAU / TEX_W as f64;
        let (e, w) = ((x + 1) % TEX_W, (x + TEX_W - 1) % TEX_W);
        let (n, s) = (y.saturating_sub(1), (y + 1).min(TEX_H - 1));
        let cos = Self::lat_of(y).cos_det().max(0.3);
        let gx = (h[y * TEX_W + e] - h[y * TEX_W + w]) / (2.0 * step * cos);
        let gy = (h[n * TEX_W + x] - h[s * TEX_W + x]) / (((s - n).max(1)) as f64 * step);
        let k = (MAX_TILT / gx.hypot(gy)).min(1.0);
        (gx * k, gy * k)
    }

    /// Bilinear sample at selenographic `(lon, lat)` (rad).
    fn sample(&self, lon: f64, lat: f64) -> Texel {
        if self.texels.is_empty() {
            return Texel {
                albedo: 1.0,
                ..Texel::default()
            };
        }
        let u = (lon + PI) / TAU * TEX_W as f64 - 0.5;
        let v = ((PI / 2.0 - lat) / PI * TEX_H as f64 - 0.5).clamp(0.0, (TEX_H - 1) as f64);
        let (i0, j0) = (u.floor(), v.floor());
        let (fu, fv) = ((u - i0) as f32, (v - j0) as f32);
        let i0 = (i0 as i64).rem_euclid(TEX_W as i64) as usize;
        let i1 = (i0 + 1) % TEX_W;
        let j0 = j0 as usize;
        let j1 = (j0 + 1).min(TEX_H - 1);
        let t = |i: usize, j: usize| self.texels[j * TEX_W + i];
        let top = t(i0, j0).lerp(t(i1, j0), fu);
        let bottom = t(i0, j1).lerp(t(i1, j1), fu);
        top.lerp(bottom, fv)
    }
}

#[derive(Debug, Default)]
pub struct MoonPhases {
    /// Scene time, seeded with a random start so each run opens on a different phase.
    t: f64,
    /// Disc centre and radius in pixels (square: x in cells, y in half rows).
    cx: f64,
    cy: f64,
    r: f64,
    label_y: Option<i32>,
    w: f64,
    h: f64,
    surface: Surface,
    pixels: Pixels,
    /// (u, v, twinkle phase, twinkle rate) with `u, v ∈ [0, 1)`.
    stars: Vec<(f64, f64, f64, f64)>,
}

/// What a disc pixel shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shade {
    /// Sunlit, with its exposure `v ∈ (0, 1]`.
    Lit(f64),
    /// Night side lit by the Earth, with its `Dim` level.
    Earthshine(u8),
    Dark,
}

/// Everything about the lighting that is the same for every pixel of a frame.
#[derive(Debug, Clone, Copy)]
struct Frame {
    lib: Libration,
    /// Sun direction in the view frame and in the body frame.
    light: [f64; 3],
    light_body: [f64; 3],
    /// View direction (+z) in the body frame.
    view_body: [f64; 3],
    /// Earth's phase while earthshine shows, else 0.
    earth: f64,
    /// Gain on the ray systems: they stand out toward full Moon, when shadows vanish.
    ray_gain: f64,
}

impl Frame {
    fn new(phase: f64, (lon, lat): (f64, f64)) -> Self {
        let lib = Libration::new(lon.to_radians(), lat.to_radians());
        let light = sun_direction(phase);
        let e = earth_phase(phase);
        let full = illuminated_fraction(phase);
        Self {
            lib,
            light,
            light_body: lib.rotate(light),
            view_body: lib.rotate([0.0, 0.0, 1.0]),
            earth: if e >= EARTH_MIN { e } else { 0.0 },
            ray_gain: 0.5 + 0.9 * full * full,
        }
    }
}

/// The log exposure of reflectance `r` (see `EXPOSURE_K`).
fn exposure(r: f64) -> f64 {
    ((1.0 + EXPOSURE_K * 2.0 * r).ln() / (1.0 + EXPOSURE_K * WHITE).ln()).min(1.0)
}

impl MoonPhases {
    /// The shade of the visible-hemisphere point `(nx, ny)` (view frame, y up)
    /// (MOON-A1, A4, A5, A8, A9).
    fn shade(&self, nx: f64, ny: f64, frame: &Frame) -> Shade {
        let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
        let n = [nx, ny, nz];
        let b = frame.lib.rotate(n);
        let lon = b[0].atan2(b[2]);
        let lat = b[1].clamp(-1.0, 1.0).asin();
        let tex = self.surface.sample(lon, lat);
        let albedo = f64::from(tex.albedo);
        // Whatever the Sun does not light, the Earth does; shown or not by the Earth's
        // phase alone, so dark maria never punch holes in it.
        let earthshine = if frame.earth > 0.0 {
            let level = EARTH_DIM * albedo * frame.earth;
            Shade::Earthshine(ink::level(
                level.clamp(ink::MIN_VISIBLE_LEVEL, LIT_FLOOR / 2.0),
            ))
        } else {
            Shade::Dark
        };
        // The smooth sphere places the terminator (MOON-A1).
        let mu0_smooth = illumination(nx, ny, frame.light);
        if mu0_smooth <= 0.0 {
            return earthshine;
        }
        // Tilt the normal down the height gradient, n' ∝ n − gx·east − gy·north. Relief
        // fades out toward the limb, where craters are foreshortened to a pixel, and at the
        // terminator, so it shades the day side without moving the day–night line.
        let relief = ease::smoothstep(nz / RELIEF_MU) * ease::smoothstep(mu0_smooth / RELIEF_MU0);
        let (gx, gy) = (f64::from(tex.gx) * relief, f64::from(tex.gy) * relief);
        let (so, co) = lon.sin_cos_det();
        let (sa, ca) = lat.sin_cos_det();
        let east = [co, 0.0, -so];
        let north = [-sa * so, ca, -sa * co];
        let tilted = v3::unit(v3::axpy(-gx, east, v3::axpy(-gy, north, b)));
        let a = albedo + frame.ray_gain * f64::from(tex.ray);
        let mu0 = v3::dot(tilted, frame.light_body);
        let v = exposure(reflectance(a, mu0, v3::dot(tilted, frame.view_body)));
        // A shadowed wall stays grey: a share of what the smooth sphere would show.
        let v = v.max(SHADOW_FLOOR * exposure(reflectance(a, mu0_smooth, nz)));
        if v >= MIN_LIT {
            Shade::Lit(v)
        } else {
            earthshine
        }
    }

    fn rasterise(&mut self, phase: f64, lib: (f64, f64)) {
        self.pixels.clear();
        let frame = Frame::new(phase, lib);
        let (cx, cy, r) = (self.cx, self.cy, self.r);
        let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
        let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let nx = (f64::from(x) + 0.5 - cx) / r;
                let ny = -(f64::from(y) + 0.5 - cy) / r;
                if nx * nx + ny * ny > 1.0 {
                    continue;
                }
                let ink = match self.shade(nx, ny, &frame) {
                    Shade::Lit(v) => {
                        Ink::new(Role::Star, ink::level(ease::lerp(LIT_FLOOR, 255.0, v * v)))
                    }
                    Shade::Earthshine(level) => Ink::new(Role::Dim, level),
                    Shade::Dark => continue,
                };
                self.pixels.set(x, y, ink);
            }
        }
    }

    fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let ink = [top, bottom]
            .into_iter()
            .filter(|i| i.role != Role::Background)
            .max_by_key(|i| (i.role == Role::Star, i.level))?;
        let ch = if ink.role == Role::Star {
            // Sunlit levels run from `LIT_FLOOR` to 255.
            let share = (f64::from(ink.level) - LIT_FLOOR) / (256.0 - LIT_FLOOR);
            ASCII_LIT[((share.max(0.0) * ASCII_LIT.len() as f64) as usize).min(ASCII_LIT.len() - 1)]
        } else {
            '.'
        };
        Some((ch, ink))
    }

    fn draw_label(&self, canvas: &mut Canvas, phase: f64) {
        let Some(y) = self.label_y else { return };
        // "{name}  {pct}%" drawn in pieces so render never allocates (GEN-R11).
        let name = phase_name(phase);
        let pct = ((100.0 * illuminated_fraction(phase)).round() as u32).min(100);
        let mut buf = StackText::<24>::new();
        let _ = std::fmt::Write::write_fmt(&mut buf, format_args!("{name}  {pct}%"));
        draw::text_centered(canvas, y, buf.as_str(), Ink::new(Role::Text, 220));
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
        let rows = draw::fit_radius(Size::new(w, disc_h), 0.9);
        self.cx = f64::from(w) / 2.0;
        self.cy = f64::from(disc_h);
        self.r = 2.0 * rows;
        self.label_y = (h >= 8)
            .then(|| ((f64::from(disc_h) / 2.0 + rows).ceil() as i32 + 1).min(i32::from(h) - 2));
        self.w = f64::from(w);
        self.h = f64::from(h);
        if self.surface.texels.is_empty() {
            // The surface and sky survive resizes.
            self.t = ctx.rng.random_range(0.0..SYNODIC);
            self.surface = Surface::build(&mut ctx.rng);
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
        self.pixels.resize(size);
        self.rasterise(phase_at(self.t), libration(self.t));
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.rasterise(phase_at(self.t), libration(self.t));
    }

    fn render(&self, canvas: &mut Canvas) {
        let (cx, cy, r) = (self.cx, self.cy / 2.0, self.r / 2.0);
        for &(u, v, ph, rate) in &self.stars {
            let (x, y) = ((u * self.w).floor(), (v * self.h).floor());
            let (dx, dy) = ((x + 0.5 - cx) / (r * geom::ASPECT), (y + 0.5 - cy) / r);
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
        self.pixels
            .blit(canvas, |_, _, top, bottom| Self::ascii_cell(top, bottom));
        self.draw_label(canvas, phase_at(self.t));
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

    fn moon(w: u16, h: u16) -> MoonPhases {
        let size = Size::new(w, h);
        let mut m = MoonPhases::default();
        m.resize(size, &mut Ctx::new(3, size));
        m
    }

    const NO_LIB: (f64, f64) = (0.0, 0.0);

    #[test]
    fn moon_a1_new_moon_dark_full_moon_lit_first_quarter_lights_right_half() {
        let mut n = 0;
        for (nx, ny) in interior_grid() {
            n += 1;
            assert!(
                illumination(nx, ny, sun_direction(0.0)) <= 1e-12,
                "new moon lit at ({nx}, {ny})"
            );
            assert!(
                illumination(nx, ny, sun_direction(0.5)) > 0.0,
                "full moon dark at ({nx}, {ny})"
            );
            let lit = illumination(nx, ny, sun_direction(0.25)) > 0.0;
            assert_eq!(lit, nx > 0.0, "first quarter at ({nx}, {ny})");
        }
        assert!(n > 1000, "grid too sparse: {n}");
        // Rendered: no sunlit pixel at new moon, and sunlit ones only right of centre at
        // first quarter.
        let mut m = moon(120, 40);
        m.rasterise(0.0, NO_LIB);
        let lit = |m: &MoonPhases, right: Option<bool>| {
            (0..80)
                .flat_map(|y| (0..120).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let star = m.pixels.get(x, y).is_some_and(|i| i.role == Role::Star);
                    let side = f64::from(x) + 0.5 > m.cx;
                    star && right.is_none_or(|r| r != side)
                })
                .count()
        };
        assert_eq!(lit(&m, None), 0);
        m.rasterise(0.25, NO_LIB);
        assert!(lit(&m, None) > 500);
        assert_eq!(lit(&m, Some(true)), 0, "lit pixels left of centre");
    }

    #[test]
    fn moon_a2_phase_names() {
        assert_eq!(phase_name(0.0), "New Moon");
        assert_eq!(phase_name(0.25), "First Quarter");
        assert_eq!(phase_name(0.5), "Full Moon");
        assert_eq!(phase_name(0.75), "Last Quarter");
        assert_eq!(phase_name(0.99), "New Moon");
        assert_eq!(phase_name(0.125), "Waxing Crescent");
        assert_eq!(phase_name(0.375), "Waxing Gibbous");
        assert_eq!(phase_name(0.625), "Waning Gibbous");
        assert_eq!(phase_name(0.875), "Waning Crescent");
        // Quarters, new and full are instants: a 3 % or a 61 % Moon is not one of them.
        assert_eq!(phase_name(0.97), "Waning Crescent");
        assert_eq!(phase_name(0.03), "Waxing Crescent");
        assert_eq!(phase_name(0.715), "Waning Gibbous");
        assert_eq!(phase_name(0.225), "Waxing Crescent");
        assert_eq!(phase_name(0.275), "Waxing Gibbous");
        assert_eq!(phase_name(0.519), "Full Moon");
    }

    #[test]
    fn moon_a3_illuminated_fraction() {
        for (p, f) in [(0.0, 0.0), (0.25, 0.5), (0.5, 1.0), (0.75, 0.5)] {
            assert!((illuminated_fraction(p) - f).abs() < 1e-12, "p={p}");
        }
        // Cross-check against the lit area of the disc by quadrature (2000² midpoint grid):
        // the lit fraction of a sphere's projected disc is (1 − cos 2πp)/2.
        let n = 2000;
        for p in [0.1, 0.3, 0.6] {
            let (mut lit, mut all) = (0u32, 0u32);
            for i in 0..n {
                for j in 0..n {
                    let x = (f64::from(i) + 0.5) / f64::from(n) * 2.0 - 1.0;
                    let y = (f64::from(j) + 0.5) / f64::from(n) * 2.0 - 1.0;
                    if x * x + y * y < 1.0 {
                        all += 1;
                        lit += u32::from(illumination(x, y, sun_direction(p)) > 0.0);
                    }
                }
            }
            let q = f64::from(lit) / f64::from(all);
            assert!((q - illuminated_fraction(p)).abs() < 2e-3, "p={p}: {q}");
        }
    }

    #[test]
    fn moon_a4_lommel_seeliger_full_moon_is_flat_to_the_limb() {
        // At zero phase angle μ0 = μ: the same reflectance at the centre and the limb.
        assert!((reflectance(1.0, 1.0, 1.0) - reflectance(1.0, 0.1, 0.1)).abs() < 1e-12);
        assert_eq!(reflectance(1.0, -0.2, 0.5), 0.0);
        // Near quarter, the bright limb outshines the terminator side.
        assert!(reflectance(1.0, 0.95, 0.1) > reflectance(1.0, 0.1, 0.95));
    }

    #[test]
    fn moon_a5_earthshine_is_ashen_and_strongest_at_new_moon() {
        assert!((earth_phase(0.0) - 1.0).abs() < 1e-12 && earth_phase(0.5).abs() < 1e-12);
        // (count, summed level, brightest level) of the earthshine pixels at phase `p`.
        let earthshine = |p: f64| {
            let mut m = moon(120, 40);
            m.rasterise(p, NO_LIB);
            let (mut n, mut sum, mut max) = (0u32, 0u32, 0u8);
            for y in 0..80 {
                for x in 0..120 {
                    if let Some(ink) = m.pixels.get(x, y)
                        && ink.role == Role::Dim
                    {
                        n += 1;
                        sum += u32::from(ink.level);
                        max = max.max(ink.level);
                    }
                }
            }
            (n, sum, max)
        };
        let (new, crescent, gibbous) = (earthshine(0.02), earthshine(0.15), earthshine(0.4));
        assert!(
            new.0 > crescent.0 && crescent.0 > gibbous.0,
            "{new:?} {crescent:?}"
        );
        // Brighter per pixel too: a fuller Earth lights the night side more.
        assert!(new.1 * crescent.0 > crescent.1 * new.0);
        assert_eq!(earthshine(0.5).0, 0);
        // A ghost disc, not a coloured fill: the brightest earthshine stays a dim grey,
        // well under the darkest mare of the full Moon.
        assert!(new.2 <= 80, "earthshine level {}", new.2);
    }

    #[test]
    fn moon_a5_the_crescent_outshines_the_earthshine_at_the_terminator() {
        // Every sunlit pixel within 3 px of an earthshine pixel is at least twice the
        // brightest earthshine level, so the terminator never dissolves into the ghost
        // disc in a theme (mono) that maps `Dim` and `Star` levels through one grey ramp.
        for (w, h) in [(200, 50), (120, 40), (80, 24)] {
            let mut m = moon(w, h);
            for p in [0.04, 0.08, 0.12, 0.16, 0.84, 0.88, 0.92, 0.96] {
                assert!(illuminated_fraction(p) < 0.35);
                m.rasterise(p, NO_LIB);
                let (pw, ph) = (i32::from(w), 2 * i32::from(h));
                let ink =
                    |x: i32, y: i32| m.pixels.get(x, y).filter(|i| i.role != Role::Background);
                let earth_max = (0..ph)
                    .flat_map(|y| (0..pw).map(move |x| (x, y)))
                    .filter_map(|(x, y)| ink(x, y).filter(|i| i.role == Role::Dim))
                    .map(|i| i.level)
                    .max();
                let Some(earth_max) = earth_max else {
                    panic!("{w}x{h} p={p}: no earthshine");
                };
                for y in 0..ph {
                    for x in 0..pw {
                        let Some(lit) = ink(x, y).filter(|i| i.role == Role::Star) else {
                            continue;
                        };
                        let near = (-3..=3).any(|dy| {
                            (-3..=3)
                                .any(|dx| ink(x + dx, y + dy).is_some_and(|i| i.role == Role::Dim))
                        });
                        assert!(
                            !near || u16::from(lit.level) >= 2 * u16::from(earth_max),
                            "{w}x{h} p={p}: lit {} beside earthshine {earth_max} at ({x}, {y})",
                            lit.level
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn moon_a6_libration_is_bounded_and_periodic() {
        let (mut max_l, mut max_b) = (0.0f64, 0.0f64);
        for k in 0..6000 {
            let (l, b) = libration(f64::from(k) * 0.01);
            assert!(l.abs() <= LIB_LON.0 + 1e-12 && b.abs() <= LIB_LAT.0 + 1e-12);
            max_l = max_l.max(l.abs());
            max_b = max_b.max(b.abs());
        }
        assert!(
            max_l >= 0.95 * 7.9 && max_b >= 0.95 * 6.7,
            "{max_l} {max_b}"
        );
        let (l0, b0) = libration(3.7);
        assert!((libration(3.7 + LIB_LON.1).0 - l0).abs() < 1e-9);
        assert!((libration(3.7 + LIB_LAT.1).1 - b0).abs() < 1e-9);
        assert!((LIB_LON.1 - 27.99).abs() < 0.01 && (LIB_LAT.1 - 27.64).abs() < 0.01);
    }

    #[test]
    fn moon_a7_maria_sit_in_their_true_places() {
        let m = moon(120, 40);
        let deg = PI / 180.0;
        let albedo = |lon: f64, lat: f64| f64::from(m.surface.sample(lon * deg, lat * deg).albedo);
        let highland = albedo(10.0, -50.0);
        assert!(albedo(59.0, 17.0) < 0.7 * highland, "Crisium");
        assert!(albedo(-16.0, 33.0) < 0.7 * highland, "Imbrium");
        // The maria are not one grey: Serenitatis is paler than Tranquillitatis.
        assert!(
            albedo(17.5, 28.0) > albedo(31.0, 8.5) + 0.08,
            "Serenitatis vs Tranquillitatis"
        );
        // Real sizes: the maria cover 25–35 % of the near side (31 % on the Moon), and
        // the eastern chain Serenitatis–Tranquillitatis–Fecunditatis is joined.
        let (cover, _) = Surface::maria();
        let (mut mare, mut all) = (0.0, 0.0);
        for (i, &c) in cover.iter().enumerate() {
            let (lon, lat) = (Surface::lon_of(i % TEX_W), Surface::lat_of(i / TEX_W));
            if lon.abs() < PI / 2.0 {
                all += lat.cos_det();
                mare += if c > 0.5 { lat.cos_det() } else { 0.0 };
            }
        }
        let share = mare / all;
        assert!((0.25..=0.35).contains(&share), "mare share {share}");
        let at = |lon: f64, lat: f64| {
            let (j, i) = (
                Surface::row(lat * deg),
                ((lon + 180.0) / 360.0 * TEX_W as f64) as usize,
            );
            cover[j * TEX_W + i]
        };
        for (lon, lat) in [(24.0, 18.0), (42.0, 0.0), (-35.0, 30.0)] {
            assert!(at(lon, lat) > 0.5, "gap in the maria at ({lon}, {lat})");
        }
        // At zero libration the view frame is the body frame: Crisium (east) on the right,
        // Imbrium upper left.
        let lib = Libration::new(0.0, 0.0);
        let c = body(59.0 * deg, 17.0 * deg);
        assert_eq!(lib.rotate(c), c);
        assert!(c[0] > 0.0);
        let i = body(-16.0 * deg, 33.0 * deg);
        assert!(i[0] < 0.0 && i[1] > 0.0);
        // A libration of (λ, β) brings body(λ, β) to the disc centre.
        let lib = Libration::new(0.1, -0.08);
        let centre = lib.rotate([0.0, 0.0, 1.0]);
        let expected = body(0.1, -0.08);
        assert!((0..3).all(|k| (centre[k] - expected[k]).abs() < 1e-12));
    }

    #[test]
    fn moon_a8_crater_walls_facing_the_sun_are_brighter_near_the_terminator() {
        // Last quarter: the Sun is to the left (west), the terminator at lon 0; Copernicus
        // (20° W) is 20° inside the day side. Its east inner wall faces the Sun.
        let m = moon(120, 40);
        let deg = PI / 180.0;
        let frame = Frame::new(0.75, NO_LIB);
        let (lon, lat, r) = (-20.1 * deg, 9.6 * deg, 2.8 * deg);
        let at = |dlon: f64| {
            let v = body(lon + dlon, lat);
            match m.shade(v[0], v[1], &frame) {
                Shade::Lit(v) => v,
                _ => 0.0,
            }
        };
        let (sunward, far) = (at(-0.55 * r / lat.cos_det()), at(0.55 * r / lat.cos_det()));
        assert!(
            far > sunward + 0.05,
            "far wall {far} vs sunward wall {sunward}"
        );
    }

    #[test]
    fn moon_a9_relief_never_punches_holes_in_the_sunlit_disc() {
        // Wherever the smooth sphere is lit by more than a sliver, a pixel is sunlit `Star`
        // at no less than the shadow floor of its smooth-sphere exposure: no black, no
        // earthshine and no empty pixel inside the day side, at any phase, libration or
        // size (the relief speckle near the poles came from exactly these).
        for (w, h) in [(200, 50), (80, 24), (40, 12)] {
            let mut m = moon(w, h);
            for p in [0.1, 0.2, 0.25, 0.3, 0.5, 0.7, 0.75, 0.8, 0.9] {
                for lib in [NO_LIB, (7.9, 6.7), (-7.9, -6.7)] {
                    m.rasterise(p, lib);
                    let frame = Frame::new(p, lib);
                    let (x0, x1) = ((m.cx - m.r) as i32, (m.cx + m.r) as i32 + 1);
                    let (y0, y1) = ((m.cy - m.r) as i32, (m.cy + m.r) as i32 + 1);
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let nx = (f64::from(x) + 0.5 - m.cx) / m.r;
                            let ny = -(f64::from(y) + 0.5 - m.cy) / m.r;
                            if nx * nx + ny * ny > 1.0 || illumination(nx, ny, frame.light) < 0.1 {
                                continue;
                            }
                            let ink = m.pixels.get(x, y);
                            assert!(
                                ink.is_some_and(|i| i.role == Role::Star),
                                "{w}x{h} p={p} lib={lib:?}: ({x}, {y}) is {ink:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn moon_a9_crater_rims_near_the_limb_are_no_brighter_than_the_sphere() {
        // Near the limb (μ < 0.3) relief may darken a pixel but not lift it more than
        // 10 % over the smooth sphere, so foreshortened rims never flash as white specks.
        let m = moon(200, 50);
        for p in [0.3, 0.4, 0.5, 0.6, 0.7] {
            for lib in [NO_LIB, (7.9, 6.7), (-7.9, -6.7)] {
                let frame = Frame::new(p, lib);
                for i in 0..720 {
                    let a = f64::from(i) * TAU / 720.0;
                    for rho in [0.9, 0.95, 0.98, 0.99, 0.995] {
                        let (nx, ny) = (rho * a.cos_det(), rho * a.sin_det());
                        let nz = (1.0 - rho * rho).sqrt();
                        let mu0 = illumination(nx, ny, frame.light);
                        let Shade::Lit(v) = m.shade(nx, ny, &frame) else {
                            continue;
                        };
                        let b = frame.lib.rotate([nx, ny, nz]);
                        let tex = m
                            .surface
                            .sample(b[0].atan2(b[2]), b[1].clamp(-1.0, 1.0).asin());
                        let albedo = f64::from(tex.albedo) + frame.ray_gain * f64::from(tex.ray);
                        let smooth = exposure(reflectance(albedo, mu0, nz));
                        assert!(
                            nz > 0.3 || v <= 1.1 * smooth + 1e-9,
                            "p={p} lib={lib:?} at {a}: {v} over the sphere's {smooth}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn moon_render_full_moon_is_bright_and_labelled() {
        let size = Size::new(120, 40);
        let mut m = moon(120, 40);
        m.t = 15.0;
        m.rasterise(0.5, NO_LIB);
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        m.render(&mut canvas);
        let lit = canvas
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Star)
            .count();
        assert!(lit > 500, "{lit}");
        let text = canvas.to_lines().join("\n");
        assert!(text.contains("Full Moon  100%"), "{text}");
    }

    #[test]
    fn moon_gen_r13_label_sits_between_the_disc_and_the_title_row() {
        for (w, h) in [(40, 12), (80, 24), (30, 8), (200, 50), (60, 9)] {
            let m = moon(w, h);
            let y = m.label_y.expect("label shown");
            assert!(y <= i32::from(h) - 2, "{w}x{h}: label on row {y}");
            // The disc's lowest pixel row lies in a cell row above the label.
            let bottom_cell = ((m.cy + m.r).ceil() as i32 - 1) / 2;
            assert!(bottom_cell < y, "{w}x{h}: label on the disc");
        }
    }
}
