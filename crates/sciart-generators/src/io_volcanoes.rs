//! Io's volcanoes against black space, Jupiter looming (SPEC-002 `io_volcanoes`).
//!
//! Two vents throw umbrella plumes with the shared `plume` model: ballistic grains in
//! Io's gravity (1.8 m/s², no air), Pele at 1 km/s (≈ 280 km high) and Prometheus at
//! 0.5 km/s (≈ 70 km). Grains launched near 45° land at the ballistic range and pile up
//! there, so the fallout paints rings: Pele's red-orange (short-chain sulfur),
//! Prometheus's white (SO₂ frost). The vents glow with a pulse in step with Io's tidal
//! flexing, and lava lakes in the dark paterae flicker.
//!
//! Io is an orthographic sphere seen from above its northern limb. Its surface texture
//! is shaded once per resize; each frame lays the deposit maps and lava over it and the
//! plume on top. The plume canopy is not drawn from the live grains (a few thousand
//! grains over a canopy of thousands of pixels is speckle): it is the exact steady-state
//! column density of the ballistic fountain, integrated along thousands of parabolas once
//! per resize into a per-vent image. Each frame scales that image about the vent (the
//! eruption's speed breathes, and a fountain launched `s` times faster is `s²` times
//! larger in every direction), sends brightness waves out along the arcs at the grains'
//! own speed, and adds the live grains as faint moving strands.
//!
//! Units: plume physics in km and real seconds, run ×100 time-lapsed; the plumes are
//! drawn ×1.6 larger relative to Io than they are (heights keep their exact ratio).

use std::f64::consts::{SQRT_2, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01, value_noise};
use sciart_core::{Grid, ease, ink};

use crate::shared::plume::{self, Plume};
use crate::shared::vortex_advect::{XEdge, sample};

pub static META: Meta = Meta {
    name: "io_volcanoes",
    category: Category::SunPlanets,
    description: "Pele and Prometheus throw umbrella plumes that paint red and white rings on Io",
    title: "Io volcanoes · tides melt rock",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(IoVolcanoes::new())
}

/// Io's radius (km) and surface gravity (km/s²).
const IO_KM: f64 = 1821.6;
pub(crate) const G_IO: f64 = 1.796e-3;
/// Time-lapse: real seconds per simulated second.
const LAPSE: f64 = 100.0;
/// Plume drawing scale relative to Io.
const EXAG: f64 = 1.6;
/// Grain pool (IO-A4).
pub(crate) const POOL: usize = 6000;
/// Period of the tidal pulse of the vent glow, simulated seconds (IO-A5).
pub(crate) const TIDE_PERIOD: f64 = 20.0;
/// Deposit map cells per side and the fading of old fallout per simulated second.
const DEP_N: usize = 32;
const DEP_FADE: f64 = 0.975;
/// Plume filaments: grains leave in this many azimuthal rays (Pele's canopy shows
/// radial streaks in Voyager and Galileo images).
const RAYS: u32 = 28;
/// Simulated seconds of eruption run in the first resize, so rings already show.
const WARM: usize = 1200;
/// Dark calderas scattered over the globe, and how many of them hold glowing lava.
const PATERAE: usize = 70;
const LAVA: usize = 14;
/// Brightness waves per arc in the canopy (IO-A8).
const WAVES: f64 = 2.0;
/// One live grain in this many is drawn as a spark on its arc (IO-A8).
const SPARK: usize = 40;
/// Plume brightness below which the canopy's interior is left as sky (IO-A6).
const VEIL: f64 = 0.2;
/// The fallout rings are repainted one row in `BANDS` per frame.
const BANDS: i32 = 6;
/// Canopy integration: zenith and azimuth samples per vent (IO-A6).
const CANOPY_ZEN: usize = 18;
const CANOPY_AZ: usize = 7 * RAYS as usize;

/// A vent: launch speed (km/s), zenith-angle range, launch rate (grains per simulated
/// second), where it sits on Io (unit normal, screen axes: x right, y down, z to the
/// viewer), its deposit colour and the phase of its breathing.
#[derive(Debug, Clone, Copy)]
struct Vent {
    v0: f64,
    zenith: (f64, f64),
    rate: f64,
    normal: [f64; 3],
    deposit: Deposit,
    phase: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Deposit {
    /// Red short-chain sulfur (Pele).
    Red,
    /// White SO₂ frost (Prometheus).
    White,
}

const VENTS: [Vent; 2] = [
    // Pele: close to the north-western limb, its canopy silhouetted against space.
    Vent {
        v0: 1.0,
        zenith: (0.15, 0.9),
        rate: 430.0,
        normal: [-0.16, -0.96, 0.22],
        deposit: Deposit::Red,
        phase: 0.0,
    },
    // Prometheus: on the disc, seen more from above.
    Vent {
        v0: 0.5,
        zenith: (0.15, 0.9),
        rate: 400.0,
        normal: [0.34, -0.6, 0.72],
        deposit: Deposit::White,
        phase: 2.1,
    },
];

/// The Sun's direction: upper right and in front, so the disc is mostly lit and its
/// lower-left limb turns away into a soft terminator.
const SUN: [f64; 3] = [0.45, -0.5, 0.74];

/// Io's shading at unit normal `n`, in `[0.52, 1]` (IO-A9): a soft terminator (the lit
/// fraction eases in over ≈ 60° of the dot product) times gentle limb darkening, compressed
/// so the shaded limb stays sulfur, not olive.
fn io_light(n: [f64; 3]) -> f64 {
    let lam = dot(n, normalize(SUN));
    let day = ease::smoothstep(((lam + 0.25) / 1.0).clamp(0.0, 1.0));
    let limb = n[2].max(0.0).powf(0.35);
    0.52 + 0.48 * day * (0.65 + 0.35 * limb)
}

/// Ballistic range for a 45° launch (IO-A3).
pub(crate) fn range_45(v0: f64) -> f64 {
    plume::range(v0, G_IO, std::f64::consts::FRAC_PI_4)
}

/// Vent glow in `[0, 1]` at simulated time `t`: a pulse with Io's (compressed) orbital
/// period, the tidal flexing that heats it (IO-A5).
pub(crate) fn vent_glow(t: f64) -> f64 {
    0.55 + 0.45 * (TAU * t / TIDE_PERIOD).cos_det()
}

/// The eruption's breathing: the plume's linear size relative to its mean, at simulated
/// time `t` (IO-A8). Launch speed varies by up to ±4 %, so the size (∝ v²) by ≈ ±8 %.
pub(crate) fn plume_scale(t: f64, phase: f64) -> f64 {
    let s = 1.0 + 0.03 * (TAU * t / 7.3 + phase).sin_det() + 0.012 * (TAU * t / 2.9).sin_det();
    s * s
}

/// Plume ink at brightness `l ∈ [0, 1]`, or `None` where the plume is too thin to show
/// (IO-A6). The interior is left as sky; faint filaments rise into Ice's dark navy just
/// above the sky's own lightness; the shell eases to pale Ice (≥ 0.75 from l ≈ 0.8) and the
/// brightest rim samples to Starlight's near-white (forward-scattering fine grains).
///
/// `lift ∈ [0, 1]` fades the veil (not the shell) out towards the vent: it is the
/// smoothstep of the height above the vent over the lowest 30 % of the canopy, so the
/// veil dissolves into the sky instead of ending on a hard cut.
fn plume_ink(l: f64, lift: f64) -> Option<Ink> {
    let l = l.clamp(0.0, 1.0);
    if l < VEIL {
        return None;
    }
    let v = 0.02
        + 0.04 * lift * ease::smoothstep(((l - VEIL) / (0.5 - VEIL)).min(1.0))
        + 0.88 * ease::smoothstep(((l - 0.5) / 0.32).clamp(0.0, 1.0));
    Some(if l > 0.93 {
        ink::ramp(Ramp::Starlight, v.min(0.97))
    } else {
        ink::ramp(Ramp::Ice, v)
    })
}

/// Lava-lake brightness in `[0, 1]` for lake `k` at time `t`: smooth flicker (IO-A8).
fn lava_flicker(k: usize, t: f64) -> f64 {
    let n = value_noise(0x1A7A ^ k as u64, 2.3 * t, 7.1 * k as f64, 1.0);
    ease::smoothstep(n)
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Fractal value noise in `[0, 1]`: `octaves` octaves, each rotated and offset off the
/// lattice so no grid line (a column of bright squares) survives the sum.
fn fbm(salt: u64, x: f64, y: f64, octaves: u64) -> f64 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let (mut u, mut v) = (x + 0.37, y + 0.61);
    for o in 0..octaves {
        sum += amp * value_noise(salt ^ (o << 12), u, v, 1.0);
        norm += amp;
        amp *= 0.5;
        // Rotate by ≈ 37° and double: the next octave's lattice lies across this one's.
        (u, v) = (1.6 * u - 1.2 * v + 5.3, 1.2 * u + 1.6 * v + 2.9);
    }
    sum / norm
}

/// A vent's local frame: up, and two ground axes.
#[derive(Debug, Clone, Copy)]
struct Frame {
    up: [f64; 3],
    e1: [f64; 3],
    e2: [f64; 3],
}

impl Frame {
    fn of(normal: [f64; 3]) -> Self {
        let up = normalize(normal);
        let e1 = normalize(cross([0.0, 0.0, 1.0], up));
        let e2 = cross(up, e1);
        Self { up, e1, e2 }
    }

    /// Io-radius position of local point `(a, b, h)` km (drawn ×EXAG).
    fn place(&self, a: f64, b: f64, h: f64) -> [f64; 3] {
        let k = EXAG / IO_KM;
        std::array::from_fn(|i| self.up[i] * (1.0 + h * k) + (self.e1[i] * a + self.e2[i] * b) * k)
    }

    /// Local ground coordinates (km) of the surface point with normal `n`.
    fn ground(&self, n: [f64; 3]) -> (f64, f64) {
        let d = [n[0] - self.up[0], n[1] - self.up[1], n[2] - self.up[2]];
        let k = IO_KM / EXAG;
        (dot(d, self.e1) * k, dot(d, self.e2) * k)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct View {
    /// Io's centre and radius, pixels.
    cx: f64,
    cy: f64,
    r: f64,
    /// Jupiter's centre and radius, pixels.
    jx: f64,
    jy: f64,
    jr: f64,
}

impl View {
    fn to_px(self, p: [f64; 3]) -> (f64, f64) {
        (self.cx + self.r * p[0], self.cy + self.r * p[1])
    }

    /// Whether 3-D point `p` (Io radii) is hidden behind Io.
    fn hidden(p: [f64; 3]) -> bool {
        p[2] < 0.0 && p[0] * p[0] + p[1] * p[1] < 1.0
    }
}

/// One vent's steady-state plume image over a pixel box: column density and the
/// density-weighted flight-time fraction (0 at the vent, 1 at landing) (IO-A6).
#[derive(Debug, Clone, Default)]
struct Canopy {
    /// Box origin and size, pixels.
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    /// The vent, pixels.
    vx: f64,
    vy: f64,
    dens: Vec<f32>,
    tau: Vec<f32>,
    /// Per box row, the first and one-past-last column holding plume (`(0, 0)` if none):
    /// most of the box is empty, and the frame loop visits only these spans.
    spans: Vec<(i32, i32)>,
    /// The topmost row holding plume, pixels.
    top_y: f64,
}

impl Canopy {
    fn at(&self, x: f64, y: f64) -> (f32, f32) {
        // Bilinear, zero outside the box.
        let (fx, fy) = (x - f64::from(self.x0), y - f64::from(self.y0));
        let (i0, j0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - i0) as f32, (fy - j0) as f32);
        let (i0, j0) = (i0 as i32, j0 as i32);
        let mut d = 0.0;
        let mut t = 0.0;
        for (di, dj, w) in [
            (0, 0, (1.0 - tx) * (1.0 - ty)),
            (1, 0, tx * (1.0 - ty)),
            (0, 1, (1.0 - tx) * ty),
            (1, 1, tx * ty),
        ] {
            let (i, j) = (i0 + di, j0 + dj);
            if (0..self.w).contains(&i) && (0..self.h).contains(&j) {
                let idx = (j * self.w + i) as usize;
                d += w * self.dens[idx];
                t += w * self.tau[idx];
            }
        }
        (d, t)
    }
}

/// A lava lake: pixel centre and radius, its lake index.
#[derive(Debug, Clone, Copy, Default)]
struct Lake {
    x: f64,
    y: f64,
    r: f64,
    on: bool,
}

#[derive(Debug)]
pub struct IoVolcanoes {
    t: f64,
    view: View,
    frames: [Frame; 2],
    plume: Plume,
    /// Grains owed to each vent (fractional launches carried over).
    owed: [f64; 2],
    /// Fallout per vent on a `DEP_N²` map spanning ±1.5 × the 45° range.
    deposits: [Grid<f32>; 2],
    /// The deposit maps as drawn (see `smooth_deposits`).
    dep_smooth: [Grid<f32>; 2],
    /// Io's surface, Jupiter and the stars, shaded once per resize.
    base: Pixels,
    /// `base` with the fallout rings painted in, a band of rows per frame.
    ground: Pixels,
    /// The next band of `ground` rows to repaint.
    band: i32,
    canopies: [Canopy; 2],
    lakes: [Lake; LAVA],
    /// Live grains, splatted each frame (the strands inside the canopy).
    glow: Glow,
    pixels: Pixels,
    warmed: bool,
}

impl IoVolcanoes {
    fn new() -> Self {
        Self {
            t: 0.0,
            view: View::default(),
            frames: [Frame::of(VENTS[0].normal), Frame::of(VENTS[1].normal)],
            plume: Plume::with_capacity(POOL),
            owed: [0.0; 2],
            deposits: [
                Grid::new(Size::new(DEP_N as u16, DEP_N as u16), 0.0),
                Grid::new(Size::new(DEP_N as u16, DEP_N as u16), 0.0),
            ],
            dep_smooth: [
                Grid::new(Size::new(DEP_N as u16, DEP_N as u16), 0.0),
                Grid::new(Size::new(DEP_N as u16, DEP_N as u16), 0.0),
            ],
            base: Pixels::default(),
            ground: Pixels::default(),
            band: 0,
            canopies: [Canopy::default(), Canopy::default()],
            lakes: [Lake::default(); LAVA],
            glow: Glow::default(),
            pixels: Pixels::default(),
            warmed: false,
        }
    }

    /// Half-width (km) of vent `k`'s deposit map.
    fn dep_span(k: usize) -> f64 {
        1.5 * range_45(VENTS[k].v0)
    }

    fn step(&mut self, dt: f64, rng: &mut impl Rng) {
        self.t += dt;
        for (k, vent) in VENTS.iter().enumerate() {
            self.owed[k] += vent.rate * dt;
            while self.owed[k] >= 1.0 {
                self.owed[k] -= 1.0;
                let v0 = vent.v0 * rng.random_range(0.96..1.0);
                let zen = (vent.zenith.0 as f32, vent.zenith.1 as f32);
                if !self.plume.launch_rays(rng, k as u8, v0 as f32, zen, RAYS) {
                    self.owed[k] = 0.0;
                    break;
                }
            }
        }
        let deposits = &mut self.deposits;
        self.plume.step((dt * LAPSE) as f32, G_IO as f32, |p| {
            let k = usize::from(p.src).min(1);
            let span = Self::dep_span(k);
            // Bilinear splat onto the map (cell centres at integer + ½), so the rings
            // are not Poisson speckle.
            let to_cell = |v: f32| (f64::from(v) / span + 1.0) * 0.5 * DEP_N as f64 - 0.5;
            let (x, y) = (to_cell(p.pos[0]), to_cell(p.pos[1]));
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
            for (dx, dy, w) in [
                (0, 0, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                if let Some(c) = deposits[k].get_mut(x0 as i32 + dx, y0 as i32 + dy) {
                    *c += w;
                }
            }
        });
        let fade = DEP_FADE.powf(dt) as f32;
        for d in &mut self.deposits {
            for c in d.as_mut_slice() {
                *c *= fade;
            }
        }
    }

    /// Rebuild the display maps: each cell is the raw fallout averaged over a fan of
    /// azimuths one ray wide (so the rays' fallout blends into a smooth ring, no saw
    /// teeth), faded towards the map's edge (no rim of the map ever shows) and divided by
    /// the densest cell. 2 × 32² × 5 samples per frame, instead of 5 per ground pixel.
    fn smooth_deposits(&mut self) {
        let fan: [(f64, f64); 5] = std::array::from_fn(|i| {
            let ang = (i as f64 - 2.0) * 0.25 * TAU / f64::from(RAYS);
            (ang.cos_det(), ang.sin_det())
        });
        let n = DEP_N as f64;
        let mid = 0.5 * (n - 1.0);
        for k in 0..2 {
            let mut top = 0.0f32;
            for j in 0..DEP_N {
                for i in 0..DEP_N {
                    let (u, w) = (i as f64 - mid, j as f64 - mid);
                    let mut sum = 0.0;
                    for &(c, s) in &fan {
                        let (x, y) = (mid + u * c - w * s, mid + u * s + w * c);
                        sum +=
                            f64::from(sample(&self.deposits[k], x as f32, y as f32, XEdge::Clamp));
                    }
                    let edge = ((i.min(j).min(DEP_N - 1 - i).min(DEP_N - 1 - j)) as f64 / 3.0)
                        .clamp(0.0, 1.0);
                    let v = (edge * sum / fan.len() as f64) as f32;
                    top = top.max(v);
                    if let Some(c) = self.dep_smooth[k].get_mut(i as i32, j as i32) {
                        *c = v;
                    }
                }
            }
            let inv = 1.0 / top.max(1e-6);
            for c in self.dep_smooth[k].as_mut_slice() {
                *c *= inv;
            }
        }
    }

    /// Smoothed deposit density at ground point `(a, b)` km of vent `k`, relative to the
    /// map's densest cell.
    fn deposit_at(&self, k: usize, a: f64, b: f64) -> f64 {
        let span = Self::dep_span(k);
        let n = DEP_N as f64;
        let (x, y) = (
            (a / span + 1.0) * 0.5 * n - 0.5,
            (b / span + 1.0) * 0.5 * n - 0.5,
        );
        if !((-0.5..n - 0.5).contains(&x) && (-0.5..n - 0.5).contains(&y)) {
            return 0.0;
        }
        f64::from(sample(
            &self.dep_smooth[k],
            x as f32,
            y as f32,
            XEdge::Clamp,
        ))
    }

    /// Io's unit normal under pixel `(x, y)`, if the pixel is on the disc.
    fn io_normal(&self, x: i32, y: i32) -> Option<[f64; 3]> {
        let v = self.view;
        let nx = (f64::from(x) + 0.5 - v.cx) / v.r;
        let ny = (f64::from(y) + 0.5 - v.cy) / v.r;
        let z2 = 1.0 - nx * nx - ny * ny;
        (z2 > 0.0).then(|| [nx, ny, z2.sqrt()])
    }

    /// Caldera centres and angular radii, fixed by hash (the same Io every run).
    fn paterae() -> [([f64; 3], f64); PATERAE] {
        std::array::from_fn(|k| {
            let h = |s: u64| 2.0 * hash01(0x9A7E ^ ((k as u64) << 8) ^ s) - 1.0;
            (
                normalize([h(1), h(2), h(3)]),
                0.014 + 0.022 * (0.5 + 0.5 * h(4)),
            )
        })
    }

    /// Io's ground under unit normal `n`, lit (IO-A7): sulfur-yellow plains, pale SO₂
    /// frost fields, rust-brown highlands and polar caps, black paterae.
    fn ground_ink(n: [f64; 3], paterae: &[([f64; 3], f64)]) -> Ink {
        let lon = n[0].atan2(n[2]);
        let lat = (-n[1]).asin();
        let (u, w) = (lon * 4.0, lat * 4.0);
        let lit = io_light(n);
        let patera = paterae
            .iter()
            .map(|&(c, rad)| (1.0 - dot(n, c)).max(0.0).sqrt() * SQRT_2 / rad)
            .fold(f64::INFINITY, f64::min);
        if patera < 1.0 {
            // Black caldera floors (fresh silicate lava) with a dark brown rim.
            return if patera < 0.75 {
                ink::ramp(Ramp::Starlight, 0.1 + 0.05 * patera)
            } else {
                ink::ramp(Ramp::Sienna, 0.3 + 0.1 * lit)
            };
        }
        let mottle = fbm(0x10, 2.0 * u, 2.0 * w, 3);
        let frost = fbm(0x13, 1.1 * u + 7.0, 1.1 * w, 2);
        let rust = fbm(0x17, 0.8 * u - 3.0, 1.0 * w + 1.0, 2);
        // Io's poles are darker and redder than its sulfur-yellow equator.
        let polar = ease::smoothstep(((lat.abs() - 0.75) / 0.5).clamp(0.0, 1.0));
        // Boundaries between the units are soft: the sulfur pales to lemon-cream for a few
        // pixels before a frost patch, which starts at a cream level; the highlands start
        // pale (tan, close to the plains' lightness) and deepen inward, mottled, only as
        // far as a capped rust, so every step across a unit edge is small (IO-A9).
        let rusty = rust + 0.25 * polar + 0.18 * (mottle - 0.5);
        if frost > 0.74 {
            // SO₂ frost: white patches with a pale cream edge.
            let f = ease::smoothstep(((frost - 0.74) / 0.06).clamp(0.0, 1.0));
            ink::ramp(Ramp::Starlight, ((0.86 + 0.08 * f) * lit).max(0.6))
        } else if rusty > 0.74 {
            // Rust-brown highlands and the darker, redder poles.
            let d = ease::smoothstep(((rusty - 0.74) / 0.16).clamp(0.0, 1.0));
            ink::ramp(Ramp::Sienna, ((0.8 - 0.16 * d) * lit).max(0.55))
        } else {
            // Sulfur plains: chrome yellow, paling towards lemon-cream near the frost.
            let near_frost = ease::smoothstep(((frost - 0.62) / 0.12).clamp(0.0, 1.0));
            let v = 0.84 + 0.14 * (mottle - 0.5) + 0.12 * near_frost;
            ink::ramp(Ramp::Sulfur, (v * lit).clamp(0.55, 0.95))
        }
    }

    fn shade_base(&mut self) {
        let v = self.view;
        let size = self.base.pixel_size();
        let sun = normalize(SUN);
        let paterae = Self::paterae();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let ink = if let Some(n) = self.io_normal(x, y) {
                    Self::ground_ink(n, &paterae)
                } else {
                    let (dx, dy) = (f64::from(x) + 0.5 - v.jx, f64::from(y) + 0.5 - v.jy);
                    let d2 = (dx * dx + dy * dy) / (v.jr * v.jr);
                    if d2 < 1.0 {
                        // Jupiter: dim, banded in its own sienna and cream, lit by the
                        // same Sun, with a soft limb.
                        let n = [dx / v.jr, dy / v.jr, (1.0 - d2).sqrt()];
                        let lit = dot(n, sun).max(0.0).powf(0.7);
                        let lat = -n[1];
                        let bands = (lat * 11.0 + 0.4).sin_det()
                            + 0.35 * (lat * 27.0 + 1.3 * n[0]).sin_det();
                        let lv = (0.66 + 0.12 * bands) * (0.35 + 0.65 * lit) * n[2].powf(0.3);
                        if lv * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                            ink::ramp(Ramp::Sienna, lv)
                        } else {
                            Ink::BLANK
                        }
                    } else if hash01(0x5E ^ cell_key(x, y)) < 0.002 {
                        ink::ramp(Ramp::Starlight, 0.3 + 0.4 * hash01(0x5F ^ cell_key(x, y)))
                    } else {
                        Ink::BLANK
                    }
                };
                self.base.set(x, y, ink);
            }
        }
        // Lava lakes: the first `LAVA` paterae on the visible hemisphere glow.
        let mut n = 0;
        for &(c, rad) in &paterae {
            if n == LAVA {
                break;
            }
            if c[2] < 0.15 {
                continue;
            }
            let (x, y) = v.to_px(c);
            self.lakes[n] = Lake {
                x,
                y,
                r: (0.6 * rad * v.r * c[2].sqrt()).max(0.5),
                on: true,
            };
            n += 1;
        }
        for lake in &mut self.lakes[n..] {
            lake.on = false;
        }
    }

    /// Integrate vent `k`'s steady ballistic fountain into its canopy image (IO-A6).
    ///
    /// Every grain spends equal time on equal time slices of its parabola, so sampling
    /// each parabola uniformly in time (one sample per ≤ 0.7 px of flight) and summing
    /// over launch directions weighted by solid angle gives the column density. The rise
    /// is weighted down and the fall up: Pele's canopy is a shock where gas-borne grains
    /// pile up, so the bright rim and curtain of Voyager images outshine the vent column
    /// (a stylisation of the pure ballistic density). Azimuths bunch into `RAYS` rays of
    /// hashed strength, the canopy's radial filaments.
    fn build_canopy(&mut self, k: usize) {
        let v = self.view;
        let f = self.frames[k];
        let vent = VENTS[k];
        let px_per_km = v.r * EXAG / IO_KM;
        // Box: every point the plume reaches at its largest (a +10 % margin for the
        // breathing), projected.
        let (reach, top) = (
            range_45(vent.v0) * 1.12,
            vent.v0 * vent.v0 / (2.0 * G_IO) * 1.12,
        );
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(a, b, h) in &[
            (reach, reach, 0.0),
            (reach, -reach, 0.0),
            (-reach, reach, 0.0),
            (-reach, -reach, 0.0),
            (reach, reach, top),
            (reach, -reach, top),
            (-reach, reach, top),
            (-reach, -reach, top),
        ] {
            let (x, y) = v.to_px(f.place(a, b, h));
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let size = self.pixels.pixel_size();
        let (sw, sh) = (f64::from(size.width()), f64::from(size.height()));
        let (bx0, by0) = (x0.floor().max(-2.0), y0.floor().max(-2.0));
        let (bx1, by1) = (x1.ceil().min(sw + 2.0), y1.ceil().min(sh + 2.0));
        let c = &mut self.canopies[k];
        let (vx, vy) = v.to_px(f.place(0.0, 0.0, 0.0));
        c.vx = vx;
        c.vy = vy;
        c.x0 = bx0 as i32;
        c.y0 = by0 as i32;
        c.w = (bx1 - bx0).max(0.0) as i32 + 1;
        c.h = (by1 - by0).max(0.0) as i32 + 1;
        let len = (c.w * c.h) as usize;
        c.dens.clear();
        c.dens.resize(len, 0.0);
        c.tau.clear();
        c.tau.resize(len, 0.0);
        let (cz0, cz1) = (vent.zenith.0.cos_det(), vent.zenith.1.cos_det());
        let v0 = vent.v0;
        let apex = v0 * v0 / (2.0 * G_IO);
        let ray_w: [f64; RAYS as usize] =
            std::array::from_fn(|r| 0.9 + 0.2 * hash01(0x7A15 ^ (k as u64) << 16 ^ r as u64));
        for iz in 0..CANOPY_ZEN {
            // Uniform in cos θ: equal solid angle per zenith sample.
            let cz = cz1 + (cz0 - cz1) * (iz as f64 + 0.5) / CANOPY_ZEN as f64;
            let sz = (1.0 - cz * cz).max(0.0).sqrt();
            let flight = 2.0 * v0 * cz / G_IO;
            let n_t = ((flight * v0 * px_per_km / 0.7).ceil() as usize).clamp(8, 4000);
            let dt = flight / n_t as f64;
            for ia in 0..CANOPY_AZ {
                // Rays: azimuths bunched towards RAYS evenly spaced directions.
                let base = (ia as f64 + 0.5) / CANOPY_AZ as f64;
                let ray = ((base * f64::from(RAYS)).round() as usize) % RAYS as usize;
                let centre = ray as f64 / f64::from(RAYS);
                let off = base - centre;
                let off = off - off.round();
                let az = TAU * (centre + 0.8 * off);
                let (sa, ca) = az.sin_cos_det();
                let w_dir = ray_w[ray] * (1.0 - 0.3 * (off * f64::from(RAYS)).abs());
                for it in 0..n_t {
                    let t = (it as f64 + 0.5) * dt;
                    let frac = t / flight;
                    let r = v0 * sz * t;
                    let h = v0 * cz * t - 0.5 * G_IO * t * t;
                    let p = f.place(r * ca, r * sa, h);
                    if View::hidden(p) {
                        continue;
                    }
                    // The shock canopy: grains near the envelope of all the parabolas
                    // (the paraboloid `h = H − r²/4H` that every path touches once) are
                    // bright, the interior faint.
                    let below = (apex - r * r / (4.0 * apex) - h).max(0.0) / apex;
                    let w = (w_dir * (0.01 + (-below / 0.02).exp())) as f32;
                    let (x, y) = v.to_px(p);
                    let (fx, fy) = (x - 0.5 - bx0, y - 0.5 - by0);
                    let (i0, j0) = (fx.floor(), fy.floor());
                    let (tx, ty) = ((fx - i0) as f32, (fy - j0) as f32);
                    let (i0, j0) = (i0 as i32, j0 as i32);
                    for (di, dj, bw) in [
                        (0, 0, (1.0 - tx) * (1.0 - ty)),
                        (1, 0, tx * (1.0 - ty)),
                        (0, 1, (1.0 - tx) * ty),
                        (1, 1, tx * ty),
                    ] {
                        let (i, j) = (i0 + di, j0 + dj);
                        if (0..c.w).contains(&i) && (0..c.h).contains(&j) {
                            let idx = (j * c.w + i) as usize;
                            c.dens[idx] += w * bw;
                            c.tau[idx] += w * bw * frac as f32;
                        }
                    }
                }
            }
        }
        // A light [1 2 1] blur along rows, then columns, so the thin rim is a continuous
        // line rather than beads where the rays cross it (filaments survive: they are
        // several pixels long). One pass per resize; the scratch is the τ buffer's twin.
        for data in [&mut c.dens, &mut c.tau] {
            let (w, h) = (c.w as usize, c.h as usize);
            let mut prev;
            for j in 0..h {
                let row = &mut data[j * w..(j + 1) * w];
                prev = row.first().copied().unwrap_or(0.0);
                for i in 0..w {
                    let cur = row[i];
                    let next = if i + 1 < w { row[i + 1] } else { cur };
                    row[i] = 0.25 * prev + 0.5 * cur + 0.25 * next;
                    prev = cur;
                }
            }
            for i in 0..w {
                prev = if h > 0 { data[i] } else { 0.0 };
                for j in 0..h {
                    let cur = data[j * w + i];
                    let next = if j + 1 < h {
                        data[(j + 1) * w + i]
                    } else {
                        cur
                    };
                    data[j * w + i] = 0.25 * prev + 0.5 * cur + 0.25 * next;
                    prev = cur;
                }
            }
        }
        // Normalise: τ to a mean, density so the brightest 0.5 % of the box is ≈ 1.
        let mut peak = 0.0f32;
        for (d, t) in c.dens.iter().zip(c.tau.iter_mut()) {
            if *d > 0.0 {
                *t /= *d;
            }
            peak = peak.max(*d);
        }
        // A robust top: the 97th percentile of the lit pixels (a 64-bin histogram), so the
        // whole rim, not a lone caustic pixel, sets white.
        let mut hist = [0u32; 64];
        let mut lit = 0u32;
        for &d in &c.dens {
            if d > 0.0 {
                hist[((d / peak.max(1e-12)) * 63.0) as usize] += 1;
                lit += 1;
            }
        }
        let (mut acc, mut bin) = (0u32, 63usize);
        while bin > 0 && acc + hist[bin] < lit / 33 {
            acc += hist[bin];
            bin -= 1;
        }
        let top = (peak * (bin as f32 + 0.5) / 63.0).max(1e-12);
        for d in &mut c.dens {
            *d /= top;
        }
        c.spans.clear();
        c.top_y = f64::from(c.y0)
            + c.dens
                .chunks(c.w.max(1) as usize)
                .position(|row| row.iter().any(|&d| d > 0.0))
                .unwrap_or(0) as f64;
        for row in c.dens.chunks(c.w.max(1) as usize) {
            let first = row.iter().position(|&d| d > 0.0);
            let last = row.iter().rposition(|&d| d > 0.0);
            c.spans.push(match (first, last) {
                (Some(a), Some(b)) => (a as i32, b as i32 + 1),
                _ => (0, 0),
            });
        }
    }

    /// Plume brightness in `[0, ~1.3]` at pixel `(x, y)` from vent `k`, with the plume at
    /// linear scale `s` and the brightness waves at phase `ph` (turns).
    fn canopy_value(&self, k: usize, x: i32, y: i32, s: f64, ph: f64) -> f64 {
        let c = &self.canopies[k];
        let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let (sx, sy) = (c.vx + (px - c.vx) / s, c.vy + (py - c.vy) / s);
        let (d, tau) = c.at(sx - 0.5, sy - 0.5);
        if d <= 0.0 {
            return 0.0;
        }
        let wave = (TAU * (WAVES * f64::from(tau) - ph)).sin_det();
        f64::from(d) * (1.0 + 0.1 * wave)
    }

    /// Repaint the fallout rings into `ground` on the rows `y ≡ band (mod BANDS)` (all
    /// rows for `band = None`). The rings change over seconds, so one band per frame keeps
    /// the per-frame cost at 1/BANDS of a full repaint (GEN-R8) with no visible lag.
    fn paint_deposits(&mut self, band: Option<i32>) {
        let size = self.ground.pixel_size();
        let v = self.view;
        let on_band = |y: i32| band.is_none_or(|b| y.rem_euclid(BANDS) == b);
        // Restore the bare ground on the band's rows first.
        for y in 0..i32::from(size.height()) {
            if !on_band(y) {
                continue;
            }
            for x in 0..i32::from(size.width()) {
                let ink = self.base.get(x, y).unwrap_or(Ink::BLANK);
                self.ground.set(x, y, ink);
            }
        }
        for (k, vent) in VENTS.iter().enumerate() {
            let f = self.frames[k];
            let c = f.place(0.0, 0.0, 0.0);
            let reach = (Self::dep_span(k) * EXAG / IO_KM * v.r).ceil() + 1.0;
            let cap = (Self::dep_span(k) * EXAG / IO_KM * 1.2).min(1.2).cos_det();
            let (x0, x1) = (
                (v.cx + v.r * c[0] - reach) as i32,
                (v.cx + v.r * c[0] + reach) as i32,
            );
            let (y0, y1) = (
                (v.cy + v.r * c[1] - reach) as i32,
                (v.cy + v.r * c[1] + reach) as i32,
            );
            for y in y0.max(0)..=y1.min(i32::from(size.height()) - 1) {
                if !on_band(y) {
                    continue;
                }
                for x in x0.max(0)..=x1.min(i32::from(size.width()) - 1) {
                    let Some(n) = self.io_normal(x, y) else {
                        continue;
                    };
                    if dot(n, f.up) < cap {
                        continue;
                    }
                    let under = self.ground.get(x, y).unwrap_or(Ink::BLANK);
                    // Paterae stay black under the fallout.
                    if under.role == Role::Ramp(Ramp::Starlight) && under.level < 60 {
                        continue;
                    }
                    let (a, b) = f.ground(n);
                    let d = self.deposit_at(k, a, b);
                    // Inside the ring the threshold drops, so the fallout fades in over a
                    // wider, softer inner edge (Galileo's Pele ring is sharper outside).
                    let rho = a.hypot(b) / range_45(vent.v0);
                    let inner = ease::smoothstep(((1.0 - rho) / 0.4).clamp(0.0, 1.0));
                    let thr = 0.5 - 0.3 * inner;
                    let a = ease::smoothstep(((d - thr) / 0.4).clamp(0.0, 1.0));
                    if a <= 0.05 {
                        continue;
                    }
                    let lit = io_light(n);
                    let ink = match vent.deposit {
                        // Pele: short-chain sulfur, gold at the fringe deepening to red-
                        // orange on the ring (Ember's orange, its one iconic use here).
                        Deposit::Red => ink::ramp(Ramp::Ember, (0.8 - 0.22 * a) * lit),
                        // Prometheus: white SO₂ frost.
                        Deposit::White => {
                            ink::ramp(Ramp::Starlight, (0.78 + 0.17 * a) * (0.85 + 0.15 * lit))
                        }
                    };
                    self.ground.set(x, y, ink);
                }
            }
        }
    }

    fn compose(&mut self) {
        let size = self.pixels.pixel_size();
        let v = self.view;
        // Fallout: one band of rows per frame, the display maps rebuilt once per cycle.
        if self.band == 0 {
            self.smooth_deposits();
        }
        self.paint_deposits(Some(self.band));
        self.band = (self.band + 1) % BANDS;
        let ground = &self.ground;
        self.pixels
            .shade(|x, y| ground.get(x, y).unwrap_or(Ink::BLANK));
        // Lava lakes: glowing crimson-red cores in the black calderas, flickering (IO-A8).
        // HAlpha, not Blackbody: at Io's ≈ 1500 K a blackbody is a dim brown on screen.
        for (i, lake) in self.lakes.iter().enumerate() {
            if !lake.on {
                continue;
            }
            let fl = lava_flicker(i, self.t);
            let rr = lake.r.ceil() as i32;
            for dy in -rr..=rr {
                for dx in -rr..=rr {
                    let (x, y) = (lake.x.floor() as i32 + dx, lake.y.floor() as i32 + dy);
                    let (ex, ey) = (f64::from(x) + 0.5 - lake.x, f64::from(y) + 0.5 - lake.y);
                    let q = (ex * ex + ey * ey).sqrt() / lake.r.max(0.5);
                    if q > 1.0 && !(dx == 0 && dy == 0) {
                        continue;
                    }
                    let glow = (0.42 + 0.3 * fl) * (1.0 - 0.45 * q.min(1.0));
                    self.pixels.set(x, y, ink::ramp(Ramp::HAlpha, glow));
                }
            }
        }
        // Vent glows: up to 3200 K at the vent, pulsing with the tide.
        let pulse = vent_glow(self.t);
        const HALO: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
        for f in &self.frames {
            let p = f.place(0.0, 0.0, 0.0);
            let (x, y) = v.to_px(p);
            for &(dx, dy, w) in &HALO {
                if w <= 0.0 {
                    continue;
                }
                let centre = if dx == 0 && dy == 0 { 1.0 } else { 0.0 };
                let kelvin = 1600.0 + 1000.0 * pulse * f64::from(w) + 1500.0 * pulse * centre;
                let (px, py) = (x.floor() as i32 + dx, y.floor() as i32 + dy);
                if dx == 0 && dy == 0 || f64::from(w) * pulse > 0.35 {
                    self.pixels.set(px, py, ink::blackbody(kelvin));
                }
            }
        }
        // Live grains: one in `SPARK` drawn as a soft moving spark along its arc (the
        // rest are in the canopy), at the plume's current scale.
        self.glow.clear();
        for g in self.plume.iter().step_by(SPARK) {
            let k = usize::from(g.src).min(1);
            let f = self.frames[k];
            let p = f.place(
                f64::from(g.pos[0]),
                f64::from(g.pos[1]),
                f64::from(g.pos[2]),
            );
            if View::hidden(p) {
                continue;
            }
            let c = &self.canopies[k];
            let s = plume_scale(self.t, VENTS[k].phase);
            let (x, y) = v.to_px(p);
            self.glow.splat_bilinear(
                c.vx + (x - c.vx) * s - 0.5,
                c.vy + (y - c.vy) * s - 0.5,
                1.0,
            );
        }
        // Canopy: pale blue-white (Ice) over space, only its dense rim over the ground.
        for (k, vent) in VENTS.iter().enumerate() {
            let c = &self.canopies[k];
            let scale = plume_scale(self.t, vent.phase);
            // Brightness waves travel out along the arcs at the grains' speed: one arc per
            // flight time.
            let ph = WAVES * self.t / (2.0 * vent.v0 / G_IO / LAPSE);
            let s = scale * 1.02;
            let (bx0, by0) = (
                c.vx + (f64::from(c.x0) - c.vx) * s,
                c.vy + (f64::from(c.y0) - c.vy) * s,
            );
            let (bx1, by1) = (
                c.vx + (f64::from(c.x0 + c.w) - c.vx) * s,
                c.vy + (f64::from(c.y0 + c.h) - c.vy) * s,
            );
            let (w, h) = (i32::from(size.width()), i32::from(size.height()));
            let _ = (bx0, bx1);
            for y in (by0.floor() as i32).max(0)..(by1.ceil() as i32).min(h) {
                // The source rows this output row samples, and their plume spans, mapped
                // back to output columns (with a pixel of slack for the bilinear reach).
                let sy = c.vy + (f64::from(y) + 0.5 - c.vy) / scale - 0.5 - f64::from(c.y0);
                let j0 = sy.floor() as i32;
                let (mut lo, mut hi) = (i32::MAX, i32::MIN);
                for j in [j0, j0 + 1] {
                    if j < 0 {
                        continue;
                    }
                    if let Some(&(a, b)) = c.spans.get(j as usize)
                        && b > a
                    {
                        lo = lo.min(a - 1);
                        hi = hi.max(b + 1);
                    }
                }
                if lo >= hi {
                    continue;
                }
                let to_out = |i: i32| c.vx + (f64::from(c.x0 + i) + 0.5 - c.vx) * scale - 0.5;
                let (xa, xb) = (to_out(lo).floor() as i32, to_out(hi).ceil() as i32 + 1);
                for x in xa.max(0)..xb.min(w) {
                    let grains = f64::from(self.glow.get(x, y).unwrap_or(0.0));
                    let val = self.canopy_value(k, x, y, scale, ph);
                    if val <= 0.0 {
                        continue;
                    }
                    let under = self.pixels.get(x, y).unwrap_or(Ink::BLANK);
                    if self.io_normal(x, y).is_none() {
                        // Over space (and Jupiter): canopy plus sparks, the brighter wins.
                        let l = (val + 0.15 * grains.min(1.0)).min(1.25) / 1.25;
                        // Stars hide behind any trace of plume, even where the veil itself
                        // is left as sky; Jupiter shows through the veil.
                        let jupiter = under.role == Role::Ramp(Ramp::Sienna);
                        // Height above the vent as a fraction of the (scaled) canopy.
                        let tall = ((c.vy - c.top_y) * scale).max(1.0);
                        let above = (c.vy - (f64::from(y) + 0.5)) / tall;
                        let lift = ease::smoothstep((above / 0.3).clamp(0.0, 1.0));
                        match plume_ink(l, lift) {
                            Some(ink) if !jupiter || l >= ink::seq_value(under) => {
                                self.pixels.set(x, y, ink);
                            }
                            None if !jupiter && val > 0.02 => self.pixels.set(x, y, Ink::BLANK),
                            _ => {}
                        }
                    } else if val > 0.88 && under.role != Role::Blackbody {
                        // Over the bright ground only the canopy's densest rim shows, as a
                        // thin near-white line (Starlight, not a saturated cyan).
                        self.pixels.set(
                            x,
                            y,
                            ink::ramp(Ramp::Starlight, 0.9 + 0.07 * (val - 0.88).min(1.0)),
                        );
                    }
                }
            }
        }
    }
}

impl Generator for IoVolcanoes {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.base.resize(size);
        self.ground.resize(size);
        self.pixels.resize(size);
        self.glow.resize(size);
        let px = self.base.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let r = (1.1 * h).min(0.75 * w).max(1.0);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.4 * h + r,
            r,
            jx: 0.9 * w,
            jy: 0.1 * h,
            jr: 0.42 * h.min(0.6 * w),
        };
        if !self.warmed {
            self.warmed = true;
            for _ in 0..WARM {
                self.step(SIM_DT, &mut ctx.rng);
            }
            self.t = 0.0;
        }
        self.shade_base();
        self.smooth_deposits();
        self.paint_deposits(None);
        self.band = 0;
        self.build_canopy(0);
        self.build_canopy(1);
        self.compose();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt, &mut ctx.rng);
        }
        self.compose();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::SplitMix64;

    fn built(w: u16, h: u16, secs: f64) -> IoVolcanoes {
        let mut g = IoVolcanoes::new();
        let mut ctx = Ctx::new(7, Size::new(w, h));
        g.resize(Size::new(w, h), &mut ctx);
        for _ in 0..(secs / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g
    }

    #[test]
    fn io_a1_apex_matches_the_closed_form() {
        // Vertical launch: v0²/(2g).
        for vent in VENTS {
            let want = vent.v0 * vent.v0 / (2.0 * G_IO);
            assert!((plume::apex(vent.v0, G_IO, 0.0) - want).abs() < 1e-9 * want);
        }
        // Pele ≈ 280 km, Prometheus ≈ 70 km.
        assert!((plume::apex(1.0, G_IO, 0.0) - 278.0).abs() < 5.0);
    }

    #[test]
    fn io_a2_pele_is_at_least_three_times_taller() {
        let tallest = |k: usize| {
            let mut rng = SplitMix64::new(9);
            let mut p = Plume::with_capacity(400);
            for _ in 0..400 {
                p.launch(
                    &mut rng,
                    0,
                    VENTS[k].v0 as f32,
                    (0.0, VENTS[k].zenith.1 as f32),
                );
            }
            let mut top = 0.0f32;
            for _ in 0..2000 {
                p.step(1.0, G_IO as f32, |_| {});
                top = p.iter().map(|g| g.pos[2]).fold(top, f32::max);
            }
            top
        };
        let (pele, prometheus) = (tallest(0), tallest(1));
        assert!(pele >= 3.0 * prometheus, "{pele} vs {prometheus}");
    }

    #[test]
    fn io_a3_deposits_ring_at_the_45_degree_range() {
        let mut g = IoVolcanoes::new();
        let mut ctx = Ctx::new(3, Size::new(80, 24));
        for _ in 0..900 {
            g.step(SIM_DT, &mut ctx.rng);
        }
        for (k, vent) in VENTS.iter().enumerate() {
            // Radial profile of the deposit: the densest annulus is the ring.
            let span = IoVolcanoes::dep_span(k);
            let mut bins = [0.0f64; 32];
            let mut counts = [0usize; 32];
            for j in 0..DEP_N {
                for i in 0..DEP_N {
                    let a = ((i as f64 + 0.5) / DEP_N as f64 * 2.0 - 1.0) * span;
                    let b = ((j as f64 + 0.5) / DEP_N as f64 * 2.0 - 1.0) * span;
                    let r = a.hypot(b) / span;
                    let bin = (r * 32.0) as usize;
                    if bin < 32 {
                        bins[bin] += f64::from(*g.deposits[k].get(i as i32, j as i32).unwrap());
                        counts[bin] += 1;
                    }
                }
            }
            let (best, _) = (0..32)
                .map(|b| (b, bins[b] / counts[b].max(1) as f64))
                .fold((0, 0.0), |acc, x| if x.1 > acc.1 { x } else { acc });
            let ring = (best as f64 + 0.5) / 32.0 * span;
            let want = range_45(vent.v0);
            assert!(
                (ring / want - 1.0).abs() < 0.15,
                "vent {k}: ring {ring} vs {want}"
            );
        }
    }

    #[test]
    fn io_a4_the_pool_never_exceeds_capacity() {
        let mut g = IoVolcanoes::new();
        let mut ctx = Ctx::new(4, Size::new(80, 24));
        for _ in 0..1200 {
            g.step(0.25, &mut ctx.rng);
            assert!(g.plume.len() <= POOL);
        }
        assert_eq!(g.plume.capacity(), POOL);
    }

    #[test]
    fn io_a5_vent_glow_pulses_with_the_tidal_period() {
        for k in 0..40 {
            let t = f64::from(k) * 0.37;
            assert!((vent_glow(t) - vent_glow(t + TIDE_PERIOD)).abs() < 1e-9);
        }
        assert!(vent_glow(0.0) > 0.9 && vent_glow(0.5 * TIDE_PERIOD) < 0.2);
    }

    /// Pele's canopy over space above pixel column `x`: `(level, is_plume)` from the limb
    /// up to the topmost plume pixel (sky pixels inside the canopy count as level 0).
    fn canopy_column(g: &IoVolcanoes, x: i32) -> Vec<(u8, bool)> {
        let size = g.pixels.pixel_size();
        let mut y = i32::from(size.height()) - 1;
        while y >= 0 && g.io_normal(x, y).is_some() {
            y -= 1;
        }
        let mut col = Vec::new();
        while y >= 0 {
            let ink = g.pixels.get(x, y).unwrap();
            let plume = matches!(
                ink.role,
                Role::Ramp(Ramp::Ice) | Role::Ramp(Ramp::Starlight)
            ) && ink != g.base.get(x, y).unwrap();
            // (A background star is `base` shining through the sky, not plume.)
            col.push(if plume { (ink.level, true) } else { (0, false) });
            y -= 1;
        }
        while col.last().is_some_and(|&(_, p)| !p) {
            col.pop();
        }
        col
    }

    #[test]
    fn io_a6_the_canopy_is_a_thin_bright_umbrella_over_a_sky_interior() {
        for (w, h) in [(120u16, 40u16), (200, 50)] {
            let g = built(w, h, 1.0);
            let c = &g.canopies[0];
            let (lo, hi) = (c.vx - 0.25 * f64::from(c.w), c.vx + 0.25 * f64::from(c.w));
            let (mut checked, mut pale) = (0, 0);
            for x in (lo as i32)..(hi as i32) {
                let col = canopy_column(&g, x);
                let n = col.len();
                if n < 6 {
                    continue;
                }
                checked += 1;
                let lv = |i: usize| u32::from(col[i].0);
                // The rim: the column's brightest pixel lies in its top third.
                let top = (0..n).max_by_key(|&i| lv(i)).unwrap();
                assert!(
                    3 * top >= 2 * n,
                    "{w}x{h} x {x}: peak at {top} of {n} {col:?}"
                );
                // Pale: Ice ≥ 0.75 (level 191) or Starlight on the rim.
                if lv(top) >= 191 {
                    pale += 1;
                }
                // Faint interior: the lower half stays below Ice 0.12 (level 31) or sky.
                let inside = (0..n / 2).map(lv).max().unwrap_or(0);
                assert!(inside <= 31, "{w}x{h} x {x}: interior {inside} {col:?}");
                // No speckle in the bright shell: in the top quarter no pixel between two
                // shell pixels (level ≥ 60) dips under half of them.
                for i in (3 * n / 4).max(1)..n - 1 {
                    let (m, side) = (lv(i), lv(i - 1).min(lv(i + 1)));
                    let hole = side >= 60 && 2 * m < side;
                    assert!(!hole, "{w}x{h} x {x}: hole at {i} in {col:?}");
                }
            }
            assert!(checked >= 5, "{w}x{h}: only {checked} canopy columns");
            assert!(
                2 * pale >= checked,
                "{w}x{h}: {pale} of {checked} rims pale"
            );
            // Stars never show through the plume.
            let size = g.pixels.pixel_size();
            for y in 0..i32::from(size.height()) {
                for x in 0..i32::from(size.width()) {
                    let star = g.io_normal(x, y).is_none()
                        && g.base.get(x, y).unwrap().role == Role::Ramp(Ramp::Starlight);
                    let s = plume_scale(g.t, VENTS[0].phase);
                    if star && g.canopy_value(0, x, y, s, 0.0) > 0.03 {
                        let ink = g.pixels.get(x, y).unwrap();
                        assert_ne!(ink, g.base.get(x, y).unwrap(), "star at {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn io_a9_io_is_shaded_as_a_sphere_with_soft_unit_edges() {
        // Soft terminator and limb darkening: the light falls towards the lower-left limb.
        let n = |x: f64, y: f64| [x, y, (1.0 - x * x - y * y).sqrt()];
        assert!(io_light(n(-0.7, 0.3)) + 0.2 < io_light(n(0.2, -0.3)));
        assert!(io_light(n(0.0, 0.0)) > io_light(n(-0.9, 0.0)));
        // No hard terminator: the light changes by < 2 % per 1° step across the disc.
        let mut prev = io_light(n(-0.999, 0.0));
        for k in 1..180 {
            let a = (f64::from(k) - 90.0).to_radians();
            let l = io_light([a.sin_det(), 0.0, a.cos_det()]);
            assert!((l - prev).abs() < 0.02, "{k}: {prev} -> {l}");
            prev = l;
        }
        // Unit edges: across neighbouring ground pixels of different units (sulfur, frost,
        // highland), the ramp level jumps by more than 0.2 on < 8 % of the edges.
        let g = built(160, 50, 0.2);
        let size = g.base.pixel_size();
        let unit = |i: Ink| match i.role {
            Role::Ramp(Ramp::Sulfur) => Some(0),
            Role::Ramp(Ramp::Starlight) if i.level >= 150 => Some(1),
            Role::Ramp(Ramp::Sienna) if i.level >= 110 => Some(2),
            _ => None,
        };
        let (mut edges, mut hard) = (0, 0);
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) - 1 {
                let (a, b) = (g.base.get(x, y).unwrap(), g.base.get(x + 1, y).unwrap());
                if let (Some(ua), Some(ub)) = (unit(a), unit(b))
                    && ua != ub
                {
                    edges += 1;
                    if (ink::seq_value(a) - ink::seq_value(b)).abs() > 0.2 {
                        hard += 1;
                    }
                }
            }
        }
        assert!(edges > 20, "{edges} unit edges");
        assert!(hard * 100 < 8 * edges, "{hard} of {edges} edges hard");
    }

    #[test]
    fn io_a7_io_wears_sulfur_frost_rust_and_black_lava() {
        let g = built(160, 50, 0.5);
        let size = g.pixels.pixel_size();
        let (mut disc, mut sulfur, mut frost, mut dark, mut lava, mut ember) = (0, 0, 0, 0, 0, 0);
        let pele = g.frames[0].up;
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let Some(n) = g.io_normal(x, y) else { continue };
                disc += 1;
                let ink = g.pixels.get(x, y).unwrap();
                match ink.role {
                    Role::Ramp(Ramp::Sulfur) => sulfur += 1,
                    Role::Ramp(Ramp::Starlight) if ink.level >= 180 => frost += 1,
                    Role::Ramp(Ramp::Starlight) if ink.level < 60 => dark += 1,
                    Role::Ramp(Ramp::HAlpha) => lava += 1,
                    Role::Ramp(Ramp::Ember) => {
                        ember += 1;
                        // Ember only in Pele's fallout ring.
                        assert!(dot(n, pele) > 0.75, "ember away from Pele at {x},{y}");
                    }
                    _ => {}
                }
            }
        }
        assert!(sulfur * 2 > disc, "sulfur {sulfur} of {disc}");
        assert!(frost * 50 > disc, "frost {frost} of {disc}");
        assert!(dark >= 10 && lava >= 6, "dark {dark}, lava {lava}");
        assert!(ember > 20, "ember ring {ember}");
    }

    #[test]
    fn io_a8_the_plume_breathes_and_the_lava_flickers() {
        let sizes: Vec<f64> = (0..100)
            .map(|i| plume_scale(f64::from(i) * 0.1, 0.0))
            .collect();
        let (lo, hi) = sizes
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), &s| (a.min(s), b.max(s)));
        assert!(hi / lo > 1.08 && hi / lo < 1.25, "breathing {lo}..{hi}");
        let changed = (0..LAVA)
            .filter(|&k| (lava_flicker(k, 1.0) - lava_flicker(k, 1.6)).abs() > 0.1)
            .count();
        assert!(changed >= LAVA / 3, "{changed} lakes flicker");
        // The canopy differs between t = 1 and t = 5 even though the rings hold.
        let (a, b) = (built(120, 40, 1.0), built(120, 40, 5.0));
        let c = &a.canopies[0];
        let mut diff = 0;
        for y in c.y0.max(0)..(c.y0 + c.h).min(80) {
            for x in c.x0.max(0)..(c.x0 + c.w).min(120) {
                if a.pixels.get(x, y) != b.pixels.get(x, y) {
                    diff += 1;
                }
            }
        }
        assert!(diff > 100, "only {diff} canopy pixels change");
    }
}
