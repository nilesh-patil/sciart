//! Io's volcanoes on its limb against black space (SPEC-002 `io_volcanoes`, ADR-0017).
//!
//! Two vents throw umbrella plumes with the shared `plume` model: ballistic grains in
//! Io's gravity (1.8 m/s², no air), Pele at 1 km/s (≈ 280 km high) and Prometheus at
//! 0.5 km/s (≈ 70 km). Grains launched near 45° land at the ballistic range and pile up
//! there, so the fallout paints rings: Pele's red-brown (short-chain sulfur),
//! Prometheus's white (SO₂ frost). The vents glow with a pulse in step with Io's tidal
//! flexing, and lava lakes in the black paterae flicker.
//!
//! Io is an orthographic sphere seen from above its northern limb, filling the lower
//! frame. Its muted surface is shaded once per resize; each frame lays the deposit rings
//! (one band of rows per step) and the lava over it. The plumes are the live grains
//! themselves, one braille dot each on its ballistic arc, drawn only against space; the
//! eruption breathes through the launch speed.
//!
//! Units: plume physics in km and real seconds, run ×100 time-lapsed; the plumes are
//! drawn ×1.6 larger relative to Io than they are (heights keep their exact ratio).

use std::f64::consts::{SQRT_2, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
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
/// ADR-0017: grain dots' Ice level from the vent to the canopy top, the lava lakes' and
/// vents' temperatures (K), cold to hot.
const GRAIN_LEVEL: (f64, f64) = (110.0, 215.0);
const LAVA_K: (f64, f64) = (3800.0, 6000.0);
const VENT_K: (f64, f64) = (4500.0, 7000.0);
/// The ground's base values (ADR-0017): pale cream-yellow plains at L ≈ 0.6, drawn as cream
/// `Starlight` with soft patches of yellow `Sulfur` at the same lightness (no ramp has a
/// low-chroma yellow), whiter SO₂ frost (`Starlight`) and tan highlands (`Sienna`).
const GROUND: Ground = Ground {
    sulfur: 0.64,
    cream: 0.6,
    frost: 0.74,
    rust: 0.56,
};

#[derive(Debug, Clone, Copy)]
struct Ground {
    sulfur: f64,
    cream: f64,
    frost: f64,
    rust: f64,
}
/// The fallout rings are repainted one row in `BANDS` per frame.
const BANDS: i32 = 6;

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

/// A lava lake: pixel centre and radius, its lake index.
#[derive(Debug, Clone, Copy, Default)]
struct Lake {
    x: f64,
    y: f64,
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
    /// Io's surface and the sparse stars, shaded once per resize.
    base: Pixels,
    /// `base` with the fallout rings painted in, a band of rows per frame.
    ground: Pixels,
    /// The next band of `ground` rows to repaint.
    band: i32,
    lakes: [Lake; LAVA],
    /// Live grains, one braille dot each (the plumes).
    dots: Dots,
    pixels: Pixels,
    warmed: bool,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
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
            lakes: [Lake::default(); LAVA],
            dots: Dots::default(),
            pixels: Pixels::default(),
            warmed: false,
            dirty: false,
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
                let breathe = plume_scale(self.t, vent.phase).sqrt();
                let v0 = vent.v0 * breathe * rng.random_range(0.96..1.0);
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
                ink::ramp(Ramp::Sienna, 0.2 + 0.05 * lit)
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
            // SO₂ frost: pale grey fields with a soft edge.
            let f = ease::smoothstep(((frost - 0.74) / 0.16).clamp(0.0, 1.0));
            ink::ramp(
                Ramp::Starlight,
                ((GROUND.frost + 0.08 * f + 0.06 * (mottle - 0.5)) * lit).max(0.45),
            )
        } else if rusty > 0.74 {
            // Rust-brown highlands and the darker, redder poles.
            let d = ease::smoothstep(((rusty - 0.74) / 0.16).clamp(0.0, 1.0));
            ink::ramp(Ramp::Sienna, ((GROUND.rust - 0.06 * d) * lit).max(0.4))
        } else {
            // Plains: cream, with soft patches of sulfur yellow at the same lightness,
            // paling towards the frost.
            let near_frost = ease::smoothstep(((frost - 0.62) / 0.12).clamp(0.0, 1.0));
            let yellow = fbm(0x19, 1.4 * u + 2.0, 1.4 * w - 1.0, 2);
            if yellow > 0.56 + 0.1 * near_frost {
                let v = GROUND.sulfur + 0.05 * (mottle - 0.5) + 0.04 * near_frost;
                ink::ramp(Ramp::Sulfur, (v * lit).clamp(0.42, 0.74))
            } else {
                let v = GROUND.cream + 0.05 * (mottle - 0.5) + 0.06 * near_frost;
                ink::ramp(Ramp::Starlight, (v * lit).max(0.42))
            }
        }
    }

    fn shade_base(&mut self) {
        let v = self.view;
        let size = self.base.pixel_size();
        let paterae = Self::paterae();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                // Paterae are laid over afterwards, each over its own box (GEN-R19).
                let ink = if let Some(n) = self.io_normal(x, y) {
                    Self::ground_ink(n, &[])
                } else if hash01(0x5E ^ cell_key(x, y)) < 0.002 {
                    ink::ramp(Ramp::Starlight, 0.3 + 0.4 * hash01(0x5F ^ cell_key(x, y)))
                } else {
                    Ink::BLANK
                };
                self.base.set(x, y, ink);
            }
        }
        // Paterae: rims first, then floors, so a floor wins where two overlap (as the
        // minimum over all of them does in `ground_ink`).
        for floors in [false, true] {
            for &(c, rad) in &paterae {
                if c[2] <= 0.0 {
                    continue;
                }
                let (px, py) = v.to_px(c);
                let reach = (rad * v.r).ceil() as i32 + 2;
                let (x0, y0) = (px.floor() as i32, py.floor() as i32);
                for y in y0 - reach..=y0 + reach {
                    for x in x0 - reach..=x0 + reach {
                        let Some(n) = self.io_normal(x, y) else {
                            continue;
                        };
                        let patera = (1.0 - dot(n, c)).max(0.0).sqrt() * SQRT_2 / rad;
                        if patera < 0.75 || (!floors && patera < 1.0) {
                            // Inside the patera `ground_ink` returns before any noise.
                            self.base.set(x, y, Self::ground_ink(n, &[(c, rad)]));
                        }
                    }
                }
            }
        }
        // Lava lakes: the first `LAVA` paterae on the visible hemisphere glow.
        let mut n = 0;
        for &(c, _) in &paterae {
            if n == LAVA {
                break;
            }
            if c[2] < 0.15 {
                continue;
            }
            let (x, y) = v.to_px(c);
            self.lakes[n] = Lake { x, y, on: true };
            n += 1;
        }
        for lake in &mut self.lakes[n..] {
            lake.on = false;
        }
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
                    let thr = 0.62 - 0.3 * inner;
                    let a = ease::smoothstep(((d - thr) / 0.4).clamp(0.0, 1.0));
                    if a <= 0.05 {
                        continue;
                    }
                    let lit = io_light(n);
                    let ink = match vent.deposit {
                        // Pele: short-chain sulfur, a dull red-brown ring (ADR-0017).
                        Deposit::Red => ink::ramp(Ramp::Sienna, (0.5 - 0.1 * a) * lit),
                        // Prometheus: pale SO₂ frost.
                        Deposit::White => {
                            ink::ramp(Ramp::Starlight, (0.7 + 0.1 * a) * (0.85 + 0.15 * lit))
                        }
                    };
                    self.ground.set(x, y, ink);
                }
            }
        }
    }

    /// Fallout: one band of rows per step, the display maps rebuilt once per cycle. Part of
    /// the step, so the bands keep their cadence however often a frame is drawn (GEN-R20).
    fn fallout_band(&mut self) {
        if self.band == 0 {
            self.smooth_deposits();
        }
        self.paint_deposits(Some(self.band));
        self.band = (self.band + 1) % BANDS;
    }

    fn compose(&mut self) {
        let v = self.view;
        let ground = &self.ground;
        self.pixels
            .shade(|x, y| ground.get(x, y).unwrap_or(Ink::BLANK));
        // Lava lakes: two flickering pixels of hot lava in each lit caldera, the accent
        // (IO-A8).
        for (i, lake) in self.lakes.iter().enumerate() {
            if lake.on {
                let k = LAVA_K.0 + (LAVA_K.1 - LAVA_K.0) * lava_flicker(i, self.t);
                let (x, y) = (lake.x.floor() as i32, lake.y.floor() as i32);
                self.pixels.set(x, y, ink::blackbody(k));
                self.pixels.set(x + 1, y, ink::blackbody(0.8 * k));
            }
        }
        // Vent glows: the vent's pixel, pulsing with the tide (the accent, IO-A5).
        let pulse = vent_glow(self.t);
        for f in &self.frames {
            let (x, y) = v.to_px(f.place(0.0, 0.0, 0.0));
            self.pixels.set(
                x.floor() as i32,
                y.floor() as i32,
                ink::blackbody(VENT_K.0 + (VENT_K.1 - VENT_K.0) * pulse),
            );
        }
        // The plumes: every live grain a braille dot on its ballistic arc, pale Ice,
        // brighter high in the canopy where the grains crowd (IO-A6).
        self.dots.clear();
        for g in self.plume.iter() {
            let k = usize::from(g.src).min(1);
            let f = self.frames[k];
            let h = f64::from(g.pos[2]);
            let p = f.place(f64::from(g.pos[0]), f64::from(g.pos[1]), h);
            if View::hidden(p) {
                continue;
            }
            // Only against space: the thin plume vanishes over the bright ground (as Pele
            // does in the Voyager and Galileo frames, seen on the limb).
            let (x, y) = v.to_px(p);
            if self.io_normal(x.floor() as i32, y.floor() as i32).is_some() {
                continue;
            }
            let apex = VENTS[k].v0 * VENTS[k].v0 / (2.0 * G_IO);
            let up = (h / apex).clamp(0.0, 1.0);
            let level = GRAIN_LEVEL.0 + (GRAIN_LEVEL.1 - GRAIN_LEVEL.0) * up;
            // The dot inside the grain's own pixel (two dots per pixel each way).
            self.dots.set(
                (2.0 * x).floor() as i32,
                (2.0 * y).floor() as i32,
                Ink::new(Role::Ramp(Ramp::Ice), ink::level(level)),
            );
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
        self.dots.resize(size);
        let px = self.base.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let r = (0.95 * h).min(0.65 * w).max(1.0);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.5 * h + r,
            r,
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
        self.fallout_band();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt, &mut ctx.rng);
        }
        self.fallout_band();
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.compose();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
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
        g.prepare();
        for _ in 0..(secs / SIM_DT).round() as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
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

    /// Braille plume cells (Ice) in the canvas rendered from `g`.
    fn plume_cells(g: &IoVolcanoes, size: Size) -> Vec<(i32, i32, Ink)> {
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let mut out = Vec::new();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let c = canvas.get(x, y).unwrap();
                if ('\u{2801}'..='\u{28FF}').contains(&c.ch) && c.ink.role == Role::Ramp(Ramp::Ice)
                {
                    out.push((x, y, c.ink));
                }
            }
        }
        out
    }

    #[test]
    fn io_a6_the_plumes_are_grains_on_their_arcs_against_space() {
        // ADR-0017: the plume is the live grains, one braille dot each, pale Ice, only
        // against space, brighter high in the canopy; it moves from frame to frame.
        for (w, h) in [(120u16, 40u16), (200, 50)] {
            let size = Size::new(w, h);
            let g = built(w, h, 1.0);
            let cells = plume_cells(&g, size);
            assert!(cells.len() > 40, "{w}x{h}: {} plume cells", cells.len());
            for &(x, y, ink) in &cells {
                assert!(
                    (2 * y..2 * y + 2).any(|py| g.io_normal(x, py).is_none()),
                    "{w}x{h}: plume over the ground at {x},{y}"
                );
                assert!(
                    (GRAIN_LEVEL.0 - 1.0..=GRAIN_LEVEL.1 + 1.0).contains(&f64::from(ink.level))
                );
            }
            // The canopy's top is brighter than its foot.
            let top = cells.iter().map(|c| c.1).min().unwrap();
            let bottom = cells.iter().map(|c| c.1).max().unwrap();
            let mean = |row: i32| {
                let v: Vec<f64> = cells
                    .iter()
                    .filter(|c| c.1 == row)
                    .map(|c| f64::from(c.2.level))
                    .collect();
                v.iter().sum::<f64>() / v.len().max(1) as f64
            };
            assert!(
                mean(top) > mean(bottom),
                "{w}x{h}: top {} foot {}",
                mean(top),
                mean(bottom)
            );
        }
        let (a, b) = (built(120, 40, 1.0), built(120, 40, 1.5));
        let (pa, pb) = (
            plume_cells(&a, Size::new(120, 40)),
            plume_cells(&b, Size::new(120, 40)),
        );
        assert_ne!(pa, pb, "the plume must move");
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
            Role::Ramp(Ramp::Starlight) if i.level >= 55 => Some(1),
            Role::Ramp(Ramp::Sienna) if i.level >= 50 => Some(2),
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
    fn io_a7_io_wears_muted_sulfur_frost_rust_and_black_lava() {
        let g = built(160, 50, 0.5);
        let size = g.pixels.pixel_size();
        let (mut disc, mut sulfur, mut frost, mut dark, mut lava, mut ring) = (0, 0, 0, 0, 0, 0);
        let mut sulfur_level = 0u64;
        let pele = g.frames[0].up;
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let Some(n) = g.io_normal(x, y) else { continue };
                disc += 1;
                let ink = g.pixels.get(x, y).unwrap();
                match ink.role {
                    Role::Ramp(Ramp::Sulfur) => sulfur += 1,
                    Role::Ramp(Ramp::Starlight) if ink.level >= 55 => frost += 1,
                    Role::Ramp(Ramp::Starlight) if ink.level < 40 => dark += 1,
                    Role::Blackbody => lava += 1,
                    Role::Ramp(Ramp::Sienna) if dot(n, pele) > 0.75 => ring += 1,
                    _ => {}
                }
                // Pale but not white (ADR-0017): nothing on the ground above the
                // sulfur's cap but the lava and the vents.
                if ink.role != Role::Blackbody {
                    assert!(ink.level <= 215, "{ink:?} at {x},{y}");
                }
                if ink.role == Role::Ramp(Ramp::Sulfur) {
                    sulfur_level += u64::from(ink.level);
                }
            }
        }
        // Pale cream-yellow plains: sulfur patches cover a good share of the disc, at
        // mid-high lightness.
        assert!(sulfur * 4 > disc, "sulfur {sulfur} of {disc}");
        let mean = sulfur_level as f64 / f64::from(sulfur.max(1));
        assert!((130.0..=190.0).contains(&mean), "sulfur mean level {mean}");
        assert!(frost * 50 > disc, "frost {frost} of {disc}");
        assert!(dark >= 10 && lava >= 8, "dark {dark}, lava {lava}");
        assert!(ring > 20, "Pele's ring {ring}");
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
        // The plume breathes through its launch speed: the canopy's top rises and falls.
        let tops: Vec<i32> = [1.0, 2.5, 4.0, 5.5]
            .iter()
            .map(|&t| {
                let g = built(120, 40, t);
                plume_cells(&g, Size::new(120, 40))
                    .iter()
                    .map(|c| c.1)
                    .min()
                    .unwrap_or(99)
            })
            .collect();
        assert!(tops.iter().all(|&t| t < 40), "{tops:?}");
    }
}
