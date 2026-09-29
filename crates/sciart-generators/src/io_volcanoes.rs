//! Io's volcanoes against black space, Jupiter looming (SPEC-002 `io_volcanoes`).
//!
//! Two vents throw umbrella plumes with the shared `plume` model: ballistic grains in
//! Io's gravity (1.8 m/s², no air), Pele at 1 km/s (≈ 280 km high) and Prometheus at
//! 0.5 km/s (≈ 70 km). Grains launched near 45° land at the ballistic range and pile up
//! there, so the fallout paints rings: Pele's red (short-chain sulfur), Prometheus's
//! white (SO₂ frost). The vents glow with a pulse in step with Io's tidal flexing.
//!
//! Io is an orthographic sphere seen from above its northern limb. Its surface texture
//! is shaded once per resize; each frame lays the deposit maps over it and the plume
//! grains on top (a `Glow`, toned into the Cobalt ramp).
//!
//! Units: plume physics in km and real seconds, run ×100 time-lapsed; the plumes are
//! drawn ×1.6 larger relative to Io than they are (heights keep their exact ratio).

use std::f64::consts::{SQRT_2, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Grid, Tone, ease, ink};

use crate::shared::plume::{self, Plume};
use crate::shared::vortex_advect::{XEdge, sample};
use sciart_core::rng::value_noise;

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
/// Dark calderas scattered over the globe.
const PATERAE: usize = 70;

/// A vent: launch speed (km/s), zenith-angle range, launch rate (grains per simulated
/// second), where it sits on Io (unit normal, screen axes: x right, y down, z to the
/// viewer) and its deposit colour.
#[derive(Debug, Clone, Copy)]
struct Vent {
    v0: f64,
    zenith: (f64, f64),
    rate: f64,
    normal: [f64; 3],
    deposit: Deposit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Deposit {
    /// Red short-chain sulfur (Pele).
    Red,
    /// White SO₂ frost (Prometheus).
    White,
}

const VENTS: [Vent; 2] = [
    // Pele: on the north-western limb, its canopy silhouetted against space.
    Vent {
        v0: 1.0,
        zenith: (0.15, 0.9),
        rate: 430.0,
        normal: [-0.28, -0.95, 0.16],
        deposit: Deposit::Red,
    },
    // Prometheus: on the disc, seen more from above.
    Vent {
        v0: 0.5,
        zenith: (0.15, 0.9),
        rate: 400.0,
        normal: [0.3, -0.62, 0.72],
        deposit: Deposit::White,
    },
];

/// Io's ground at brightness `v ∈ [0, 1]`: Ember's gold-yellow where bright, orange
/// and red-brown where shaded (Sulfur's low end is olive, so it is not used here).
fn surface_ink(v: f64) -> Ink {
    ink::ramp(Ramp::Ember, (0.45 + 0.5 * v).clamp(0.06, 0.97))
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
    /// Io's surface, Jupiter and the stars, shaded once per resize.
    base: Pixels,
    glow: Glow,
    tone: Tone,
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
            base: Pixels::default(),
            glow: Glow::default(),
            tone: Tone::asinh(0.8, 45.0),
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

    /// Deposit density at ground point `(a, b)` km of vent `k`, normalised so a
    /// well-fed ring is ≈ 1.
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
        // Steady state in a ring cell: rate · share of the cell / (1 − fade) per second.
        let norm = VENTS[k].rate * 0.003 / (1.0 - DEP_FADE);
        f64::from(sample(&self.deposits[k], x as f32, y as f32, XEdge::Clamp)) / norm
    }

    /// Io's unit normal under pixel `(x, y)`, if the pixel is on the disc.
    fn io_normal(&self, x: i32, y: i32) -> Option<[f64; 3]> {
        let v = self.view;
        let nx = (f64::from(x) + 0.5 - v.cx) / v.r;
        let ny = (f64::from(y) + 0.5 - v.cy) / v.r;
        let z2 = 1.0 - nx * nx - ny * ny;
        (z2 > 0.0).then(|| [nx, ny, z2.sqrt()])
    }

    fn shade_base(&mut self) {
        let v = self.view;
        let size = self.base.pixel_size();
        // Sunlight from the upper left and a little in front.
        let sun = normalize([-0.5, -0.45, 0.75]);
        // Caldera centres and angular radii, fixed by hash (the same Io every run).
        let paterae: [([f64; 3], f64); PATERAE] = std::array::from_fn(|k| {
            let h = |s: u64| 2.0 * hash01(0x9A7E ^ ((k as u64) << 8) ^ s) - 1.0;
            (
                normalize([h(1), h(2), h(3)]),
                0.012 + 0.02 * (0.5 + 0.5 * h(4)),
            )
        });
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let ink = if let Some(n) = self.io_normal(x, y) {
                    let lon = n[0].atan2(n[2]);
                    let lat = (-n[1]).asin();
                    let (u, w) = (lon * 9.0, lat * 9.0);
                    let mottle = 0.6 * value_noise(0x10, u, w, 1.0)
                        + 0.4 * value_noise(0x11, 3.0 * u, 3.0 * w, 1.0);
                    let lit = dot(n, sun).max(0.0).powf(0.6) * (0.7 + 0.3 * n[2]);
                    // Dark paterae: scattered round calderas with a darker floor.
                    let patera = paterae
                        .iter()
                        .map(|&(c, rad)| (1.0 - dot(n, c)).max(0.0).sqrt() * SQRT_2 / rad)
                        .fold(f64::INFINITY, f64::min);
                    // Pale SO₂ frost fields.
                    let frost = 0.6 * value_noise(0x13, 1.3 * u, 1.3 * w, 1.0)
                        + 0.4 * value_noise(0x14, 2.9 * u + 3.0, 2.9 * w, 1.0);
                    if patera < 1.0 {
                        // Dark calderas: red-brown floors (Ember's dark end).
                        ink::ramp(Ramp::Ember, (0.16 + 0.14 * patera) * lit.max(0.4))
                    } else {
                        // Sulfur plains: pale lemon where bright and frosted, warming to
                        // orange and red-brown where shaded or mottled darker (Ember),
                        // never Sulfur's olive low end.
                        // SO₂ frost fields brighten the ground towards pale cream.
                        let frosty = ease::smoothstep(((frost - 0.6) / 0.25).clamp(0.0, 1.0));
                        let v =
                            ((0.8 + 0.3 * (mottle - 0.5) + 0.25 * frosty) * lit).clamp(0.0, 1.0);
                        surface_ink(v)
                    }
                } else {
                    let (dx, dy) = (f64::from(x) + 0.5 - v.jx, f64::from(y) + 0.5 - v.jy);
                    let d2 = (dx * dx + dy * dy) / (v.jr * v.jr);
                    if d2 < 1.0 {
                        // Jupiter: dim, banded, lit from the same Sun.
                        let n = [dx / v.jr, dy / v.jr, (1.0 - d2).sqrt()];
                        let lit = dot(n, sun).max(0.0);
                        let lat = -n[1];
                        let bands = (lat * 14.0 + 0.4).sin_det() + 0.4 * (lat * 31.0).sin_det();
                        let lv = (0.3 + 0.05 * bands) * lit.powf(0.8);
                        if lv * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                            ink::ramp(Ramp::Starlight, lv)
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
    }

    fn compose(&mut self) {
        let size = self.pixels.pixel_size();
        // Surface, then fallout over each vent's neighbourhood only.
        let base = &self.base;
        self.pixels
            .shade(|x, y| base.get(x, y).unwrap_or(Ink::BLANK));
        let v = self.view;
        for (k, vent) in VENTS.iter().enumerate() {
            let f = self.frames[k];
            let c = f.place(0.0, 0.0, 0.0);
            let reach = (Self::dep_span(k) * EXAG / IO_KM * v.r).ceil() + 1.0;
            let (x0, x1) = (
                (v.cx + v.r * c[0] - reach) as i32,
                (v.cx + v.r * c[0] + reach) as i32,
            );
            let (y0, y1) = (
                (v.cy + v.r * c[1] - reach) as i32,
                (v.cy + v.r * c[1] + reach) as i32,
            );
            for y in y0.max(0)..=y1.min(i32::from(size.height()) - 1) {
                for x in x0.max(0)..=x1.min(i32::from(size.width()) - 1) {
                    let Some(n) = self.io_normal(x, y) else {
                        continue;
                    };
                    if dot(n, f.up) < 0.9 {
                        continue;
                    }
                    let (a, b) = f.ground(n);
                    let d = self.deposit_at(k, a, b);
                    // Fallout blends smoothly into the ground: red sulfur darkens it
                    // towards brick (Ember), SO₂ frost whitens it towards Starlight.
                    let a = ease::smoothstep(((d - 0.12) / 0.6).clamp(0.0, 1.0));
                    if a <= 0.0 {
                        continue;
                    }
                    let lit = dot(n, normalize([-0.5, -0.45, 0.75])).max(0.0).powf(0.6);
                    let lit = 0.55 + 0.45 * lit;
                    let under = self.pixels.get(x, y).unwrap_or(Ink::BLANK);
                    let ground = if under.role == Role::Ramp(Ramp::Ember) {
                        f64::from(under.level) / 255.0
                    } else {
                        0.45 + 0.5 * 0.8 * lit
                    };
                    let ink = match vent.deposit {
                        Deposit::Red => {
                            let v = ground + a * (0.3 * lit - ground);
                            ink::ramp(Ramp::Ember, v)
                        }
                        Deposit::White if a < 0.5 => {
                            ink::ramp(Ramp::Ember, (ground + a * 0.3).min(0.95))
                        }
                        Deposit::White => ink::ramp(Ramp::Starlight, 0.62 + 0.33 * lit * a),
                    };
                    self.pixels.set(x, y, ink);
                }
            }
        }
        // Plume grains, hidden behind Io's limb.
        self.glow.clear();
        let v = self.view;
        for g in self.plume.iter() {
            let f = self.frames[usize::from(g.src).min(1)];
            let p = f.place(
                f64::from(g.pos[0]),
                f64::from(g.pos[1]),
                f64::from(g.pos[2]),
            );
            if p[2] < 0.0 && p[0] * p[0] + p[1] * p[1] < 1.0 {
                continue;
            }
            // Sharp grains: a bilinear splat, so filaments and the canopy edge hold.
            self.glow
                .splat_bilinear(v.cx + v.r * p[0] - 0.5, v.cy + v.r * p[1] - 0.5, 0.7);
        }
        // Vent glows: 1300–1700 K, pulsing with the tide.
        let pulse = vent_glow(self.t);
        const HALO: [(i32, i32, f32); 9] = halo_kernel::<9>(1);
        for f in &self.frames {
            let p = f.place(0.0, 0.0, 0.0);
            let (x, y) = (v.cx + v.r * p[0], v.cy + v.r * p[1]);
            for &(dx, dy, w) in &HALO {
                if w <= 0.0 {
                    continue;
                }
                let kelvin = 1300.0 + 400.0 * pulse * f64::from(w);
                let (px, py) = (x.floor() as i32 + dx, y.floor() as i32 + dy);
                if dx == 0 && dy == 0 || f64::from(w) * pulse > 0.35 {
                    self.pixels.set(px, py, ink::blackbody(kelvin));
                }
            }
        }
        let (tone, glow) = (&self.tone, &self.glow);
        let gw = i32::from(glow.pixel_size().width());
        for (i, &val) in glow.values().iter().enumerate() {
            let level = tone.level(val);
            if level < 12 {
                continue;
            }
            let (x, y) = (i as i32 % gw.max(1), i as i32 / gw.max(1));
            let under = self.pixels.get(x, y).unwrap_or(Ink::BLANK);
            // Forward-scattered plume blue over space; over the bright ground only the
            // dense canopy shows.
            if under.role == Role::Background || level > under.level.saturating_add(40) {
                self.pixels.set(
                    x,
                    y,
                    ink::ramp(Ramp::Cobalt, 0.2 + 0.8 * f64::from(level) / 255.0),
                );
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
        self.pixels.resize(size);
        self.glow.resize(size);
        let px = self.base.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let r = (1.1 * h).min(0.75 * w).max(1.0);
        self.view = View {
            cx: 0.5 * w,
            cy: 0.4 * h + r,
            r,
            jx: 0.86 * w,
            jy: 0.12 * h,
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
}
