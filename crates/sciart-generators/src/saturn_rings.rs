//! Saturn and its rings at a 20° tilt (SPEC-002 `saturn_rings`).
//!
//! The rings are an optical-depth profile `τ(r)` built once from published radii (C, B
//! and A rings, the Cassini Division, the Encke and Keeler gaps, the Huygens ringlet),
//! seen through `1 − e^{−τ/μ}`. Every ring pixel is inverted to ring-plane `(r, λ)`; its
//! clumpy texture is value noise in `(r, λ − Ω(r)·t)` with the Keplerian `Ω ∝ r^{−3/2}`,
//! so the rings shear honestly. Daphnis, in the Keeler gap, raises edge waves that decay
//! downstream; spokes corotate with the planet's magnetic field across the B ring; the
//! globe (oblate, banded, lit from the side) hides the far rings and throws its shadow
//! on them, and the rings shadow the globe in turn. The globe is shaded once per resize;
//! the rings are re-shaded every frame in `update`.
//!
//! Units: lengths in Saturn equatorial radii (60 268 km), time in simulated seconds with
//! the ring motion time-lapsed ×2000.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::wrap_angle;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
use sciart_core::rng::{cell_key, hash01};

use crate::shared::vortex_advect::value_noise_wrap_x;
use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "saturn_rings",
    category: Category::SunPlanets,
    description: "Saturn's tilted rings: Cassini Division, Encke and Keeler gaps, spokes, shadow",
    title: "Saturn rings · moons carve gaps",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SaturnRings::new())
}

/// Ring opening angle seen from the viewer, and the Sun's elevation and azimuth.
const TILT: f64 = 20.0 * PI / 180.0;
const SUN_ELEV: f64 = 14.0 * PI / 180.0;
const SUN_AZ: f64 = 38.0 * PI / 180.0;
/// Polar radius of the globe (flattening ≈ 0.1).
const POLAR: f64 = 0.9;
/// Saturn's rotation (10.66 h) time-lapsed ×2000, rad/s; the radius where the Keplerian
/// rate equals it (synchronous orbit, 1.86 R_S).
pub(crate) const OMEGA_SATURN: f64 = TAU / (10.66 * 3600.0) * 2000.0;
const R_SYNC: f64 = 1.86;

/// The profile LUT spans `[R_IN, R_OUT)`.
const R_IN: f64 = 1.20;
const R_OUT: f64 = 2.34;
const LUT: usize = 2048;

/// Gap radii (the gap widths are exaggerated so they show at terminal resolution).
pub(crate) const CASSINI: (f64, f64) = (1.951, 2.027);
pub(crate) const ENCKE: (f64, f64) = (2.203, 2.224);
pub(crate) const KEELER: (f64, f64) = (2.247, 2.259);
const A_OUTER: f64 = 2.2695;
/// Daphnis edge waves: amplitude (R_S, exaggerated), decay length and wavenumber in
/// radians downstream of the moon.
const WAVE_AMP: f64 = 0.010;
const WAVE_DECAY: f64 = 1.0;
const WAVE_K: f64 = 16.0;
/// Spokes alive at most, their radial band and life (s).
const SPOKES: usize = 8;
const SPOKE_BAND: (f64, f64) = (1.58, 1.92);
const SPOKE_LIFE: (f64, f64) = (6.0, 10.0);
/// Deepest spoke darkening (exaggerated so they read).
const SPOKE_DIM: f64 = 0.6;
/// The globe's ramp: butterscotch zones and tan belts.
const GLOBE: Ramp = Ramp::Ember;
/// Background stars per 1000 pixels.
const STAR_DENSITY: f64 = 2.5;

/// Keplerian angular rate at `r`, rad/s (time-lapsed): `Ω ∝ r^{−3/2}` (RING-A2).
pub(crate) fn kepler_rate(r: f64) -> f64 {
    // (R_SYNC / r)^{3/2} without `powf`: this runs for every ring pixel.
    let q = R_SYNC / r;
    OMEGA_SATURN * q * q.sqrt()
}

/// Optical depth at ring radius `r` (RING-A1), with the fine ringlet structure; 0 off
/// the rings.
pub(crate) fn tau(r: f64) -> f64 {
    let ringlets = 0.75 + 0.5 * value_noise(0x51AC, r * 60.0, 0.5, 1.0);
    let base = match r {
        r if r < 1.239 => 0.0,
        // C ring: faint, with a few denser plateaus.
        r if r < 1.527 => {
            if (1.43..1.47).contains(&r) || (1.35..1.37).contains(&r) {
                0.35
            } else {
                0.09
            }
        }
        // B ring: inner B, the opaque core, the outer edge.
        r if r < 1.64 => 1.1,
        r if r < 1.90 => 2.8 + 1.4 * ((r - 1.64) * 40.0).sin_det().abs(),
        r if r < CASSINI.0 => 1.8,
        // Cassini Division: the Huygens gap, its ringlet, then sparse material.
        r if r < CASSINI.1 => {
            if (1.9565..1.9625).contains(&r) {
                0.6
            } else if r < 1.966 {
                0.0
            } else {
                0.05
            }
        }
        r if r < ENCKE.0 => 0.75 - 0.9 * (r - CASSINI.1),
        r if r < ENCKE.1 => 0.0,
        r if r < KEELER.0 => 0.55,
        r if r < KEELER.1 => 0.0,
        r if r < A_OUTER => 0.5,
        _ => 0.0,
    };
    base * ringlets
}

/// Relative albedo: the C ring and Cassini material are darker, dirtier ice.
fn albedo(r: f64) -> f64 {
    if r < 1.527 {
        0.42
    } else if r < CASSINI.0 {
        1.0
    } else if r < CASSINI.1 {
        0.5
    } else {
        0.82
    }
}

/// Edge-wave displacement (R_S) at angle `lag` downstream of Daphnis (RING-A5): a
/// decaying wave train; the envelope is `WAVE_AMP·e^{−lag/WAVE_DECAY}`.
pub(crate) fn edge_wave(lag: f64) -> f64 {
    let lag = lag.rem_euclid(TAU);
    WAVE_AMP * (-lag / WAVE_DECAY).exp() * (WAVE_K * lag).sin_det()
}

/// Unit vector towards the Sun: `z` north, `y` away from the viewer.
fn sun() -> [f64; 3] {
    let (se, ce) = SUN_ELEV.sin_cos_det();
    let (sa, ca) = SUN_AZ.sin_cos_det();
    [sa * ce, -ca * ce, se]
}

/// The ray `p + t·d` meets the globe (oblate spheroid) at some `t > 0`.
fn hits_globe(p: [f64; 3], d: [f64; 3]) -> bool {
    let q2 = POLAR * POLAR;
    let a = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] / q2;
    let b = 2.0 * (p[0] * d[0] + p[1] * d[1] + p[2] * d[2] / q2);
    let c = p[0] * p[0] + p[1] * p[1] + p[2] * p[2] / q2 - 1.0;
    let disc = b * b - 4.0 * a * c;
    disc > 0.0 && (-b + disc.sqrt()) > 0.0 && c > -1e-9
}

/// A ring-plane point lies in the globe's shadow (RING-A4).
pub(crate) fn in_planet_shadow(p: [f64; 3]) -> bool {
    // Cheap rejections first: sunward of the globe, or farther than its equatorial
    // radius from the shadow axis.
    let l = sun();
    let along = p[0] * l[0] + p[1] * l[1] + p[2] * l[2];
    if along >= 0.0 || p[0] * p[0] + p[1] * p[1] + p[2] * p[2] - along * along > 1.0 {
        return false;
    }
    hits_globe(p, l)
}

#[derive(Debug, Clone, Copy)]
struct Spoke {
    /// Angle in the corotating frame, its half-width (rad), radial extent, age and life.
    lambda: f64,
    half: f64,
    r0: f64,
    r1: f64,
    age: f64,
    life: f64,
}

impl Spoke {
    /// Angle now: spokes turn with the magnetic field, at Saturn's rate (RING-A3).
    fn angle(&self, t: f64) -> f64 {
        self.lambda + OMEGA_SATURN * t
    }

    fn strength(&self) -> f64 {
        (PI * (self.age / self.life).clamp(0.0, 1.0)).sin_det()
    }
}

/// A ring pixel: where it is on the ring plane and what does not change per frame.
#[derive(Debug, Clone, Copy)]
struct RingPx {
    /// Row-major pixel index.
    idx: u32,
    r: f32,
    lambda: f32,
    /// Brightness before texture and spokes.
    b0: f32,
    /// Shadow × lighting factor, kept (non-zero) only near the Keeler gap, whose
    /// brightness is recomputed each frame for Daphnis's waves.
    k: f32,
    /// Globe coverage under the pixel, negated on the far side of the rings (behind
    /// the globe's centre plane). 24 bytes a pixel: this table is the scene's largest
    /// buffer at 500×150 (TEST-R7).
    cover: f32,
}

/// Screen geometry, in pixels.
#[derive(Debug, Clone, Copy, Default)]
struct View {
    cx: f64,
    cy: f64,
    /// Pixels per Saturn radius.
    scale: f64,
}

#[derive(Debug)]
pub struct SaturnRings {
    t: f64,
    view: View,
    /// Ring brightness by radius (optical depth through the slant, albedo), `LUT` bins.
    profile: Box<[f32]>,
    /// Running sum of `profile` (`LUT + 1` entries) for footprint-averaged lookups.
    cumulative: Box<[f64]>,
    /// The globe and stars, shaded once per resize; the rings go over it each frame.
    globe: Pixels,
    /// Every ring pixel's static data, built per resize.
    ring_px: Vec<RingPx>,
    pixels: Pixels,
    spokes: Vec<Spoke>,
    size: Size,
}

impl SaturnRings {
    fn new() -> Self {
        let mu = TILT.sin_det();
        let profile = (0..LUT)
            .map(|k| {
                let r = R_IN + (R_OUT - R_IN) * (k as f64 + 0.5) / LUT as f64;
                (albedo(r) * (1.0 - (-tau(r) / mu).exp())) as f32
            })
            .collect::<Box<[f32]>>();
        let cumulative = std::iter::once(0.0)
            .chain(profile.iter().scan(0.0, |acc, &v| {
                *acc += f64::from(v);
                Some(*acc)
            }))
            .collect();
        Self {
            t: 0.0,
            view: View::default(),
            profile,
            cumulative,
            globe: Pixels::default(),
            ring_px: Vec::new(),
            pixels: Pixels::default(),
            spokes: Vec::with_capacity(SPOKES),
            size: Size::new(1, 1),
        }
    }

    /// Mean ring brightness over radii `r ± width/2`: the pixel's footprint, so gaps
    /// narrower than a pixel darken it instead of flickering in and out.
    fn brightness_over(&self, r: f64, width: f64) -> f64 {
        let bins = LUT as f64 / (R_OUT - R_IN);
        let half = (0.5 * width * bins).max(0.5);
        let at = |x: f64| {
            let x = x.clamp(0.0, LUT as f64);
            let k = (x.floor() as usize).min(LUT - 1);
            self.cumulative[k] + (x - k as f64) * f64::from(self.profile[k])
        };
        let c = (r - R_IN) * bins;
        (at(c + half) - at(c - half)) / (2.0 * half)
    }

    fn brightness(&self, r: f64) -> f64 {
        let k = ((r - R_IN) / (R_OUT - R_IN) * LUT as f64).floor();
        if (0.0..LUT as f64).contains(&k) {
            f64::from(self.profile[k as usize])
        } else {
            0.0
        }
    }

    /// Screen offsets of pixel `(x, y)` from the centre in Saturn radii (`up` positive).
    fn screen(&self, x: i32, y: i32) -> (f64, f64) {
        let v = self.view;
        (
            (f64::from(x) + 0.5 - v.cx) / v.scale,
            (v.cy - f64::from(y) - 0.5) / v.scale,
        )
    }

    /// The globe's visible surface point under screen point `(sx, up)`, if any.
    fn globe_point(sx: f64, up: f64) -> Option<[f64; 3]> {
        let (st, ct) = TILT.sin_cos_det();
        // Ray p0 + t·d towards the viewer, p0 on the plane through the centre.
        let p0 = [sx, up * st, up * ct];
        let d = [0.0, -ct, st];
        let q2 = POLAR * POLAR;
        let a = d[1] * d[1] + d[2] * d[2] / q2;
        let b = 2.0 * (p0[1] * d[1] + p0[2] * d[2] / q2);
        let c = p0[0] * p0[0] + p0[1] * p0[1] + p0[2] * p0[2] / q2 - 1.0;
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let t = (-b + disc.sqrt()) / (2.0 * a);
        Some([p0[0], p0[1] + t * d[1], p0[2] + t * d[2]])
    }

    /// Share of four sub-pixel rays through pixel `(x, y)` that hit the globe.
    fn globe_cover(&self, x: i32, y: i32) -> f64 {
        let (sx, up) = self.screen(x, y);
        let q = 0.25 / self.view.scale;
        [(-q, -q), (q, -q), (-q, q), (q, q)]
            .iter()
            .filter(|&&(dx, dy)| Self::globe_point(sx + dx, up + dy).is_some())
            .count() as f64
            / 4.0
    }

    fn shade_globe(&mut self) {
        let l = sun();
        let size = self.globe.pixel_size();
        self.globe.clear();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let (sx, up) = self.screen(x, y);
                let cover = self.globe_cover(x, y);
                let Some(p) = Self::globe_point(sx, up).or_else(|| {
                    // A limb pixel whose centre misses: shade it at the nearest hit.
                    (cover > 0.0).then(|| {
                        let q = 0.25 / self.view.scale;
                        [(-q, -q), (q, -q), (-q, q), (q, q)]
                            .iter()
                            .find_map(|&(dx, dy)| Self::globe_point(sx + dx, up + dy))
                    })?
                }) else {
                    let h = hash01(0x5A7 ^ cell_key(x, y));
                    if h < STAR_DENSITY / 1000.0 {
                        let lv = 0.25 + 0.5 * hash01(0x5A8 ^ cell_key(x, y));
                        self.globe.set(x, y, ink::ramp(Ramp::Starlight, lv));
                    }
                    continue;
                };
                let n = [p[0], p[1], p[2] / (POLAR * POLAR)];
                let nn = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let lit = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) / nn;
                if lit <= 0.0 {
                    continue; // night side: black against black space
                }
                // Zonal bands by planetographic latitude: pale butterscotch zones, tan
                // belts, a darker polar hood; fine streaks within them.
                let lat = (p[2] / POLAR).clamp(-1.0, 1.0).asin();
                let lon = p[0].atan2(-p[1]);
                let bands = 0.55 * (lat * 9.0 + 0.4).sin_det() + 0.3 * (lat * 21.0).sin_det();
                let streak = value_noise(0x5A70, 3.0 * lon, 90.0 * lat, 1.0) - 0.5;
                let polar = (lat.abs() / 1.15).min(1.0);
                let tone =
                    0.96 + 0.09 * bands * (1.0 - polar) + 0.04 * streak - 0.22 * polar * polar;
                // The rings' shadow on the southern globe (the Sun is north of them).
                let mut shadow = 1.0;
                if p[2] < 0.0 {
                    let s = -p[2] / l[2];
                    let r = (p[0] + s * l[0]).hypot(p[1] + s * l[1]);
                    shadow = 1.0 - 0.8 * self.brightness(r);
                }
                // Anti-aliased limb by sub-pixel coverage.
                let v = tone * (0.45 + 0.55 * lit.powf(0.6)) * shadow * (0.45 + 0.55 * cover);
                if v * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                    self.globe.set(x, y, ink::ramp(GLOBE, v));
                }
            }
        }
    }

    /// Precompute every ring pixel's ring-plane position and static brightness (the
    /// profile over its footprint, the globe's shadow, the lighting), and how much of it
    /// the globe covers. Far-side pixels wholly behind the globe are left out (RING-A4).
    fn build_rings(&mut self) {
        let l = sun();
        let size = self.pixels.pixel_size();
        let sin_t = TILT.sin_det();
        let light = 0.85 + 0.15 * l[2] / SUN_ELEV.sin_det();
        self.ring_px.clear();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let (sx, up) = self.screen(x, y);
                let (rx, ry) = (sx, up / sin_t);
                let r = rx.hypot(ry);
                if !(R_IN..R_OUT).contains(&r) {
                    continue;
                }
                let cover = self.globe_cover(x, y);
                if cover >= 1.0 && ry > 0.0 {
                    continue;
                }
                let foot = (rx.abs() + ry.abs() / sin_t) / (r * self.view.scale);
                let mut k = light;
                if in_planet_shadow([rx, ry, 0.0]) {
                    k *= 0.06;
                }
                let b0 = self.brightness_over(r, foot) * k;
                let keeler = (KEELER.0 - 0.03..KEELER.1 + 0.03).contains(&r);
                if b0 <= 0.0 && !keeler {
                    continue;
                }
                self.ring_px.push(RingPx {
                    idx: (y as u32) * u32::from(size.width()) + x as u32,
                    r: r as f32,
                    lambda: ry.atan2(rx) as f32,
                    b0: b0 as f32,
                    k: if keeler { k as f32 } else { 0.0 },
                    cover: if ry > 0.0 {
                        -cover as f32
                    } else {
                        cover as f32
                    },
                });
            }
        }
    }

    fn spawn_spoke(&mut self, rng: &mut impl Rng) {
        if self.spokes.len() >= SPOKES {
            return;
        }
        // Spokes form on the morning ansa (left, where ring material leaves the shadow).
        let lambda = PI + rng.random_range(-0.6..0.6) - OMEGA_SATURN * self.t;
        let r0 = rng.random_range(SPOKE_BAND.0..SPOKE_BAND.0 + 0.1);
        self.spokes.push(Spoke {
            lambda,
            half: rng.random_range(0.04..0.09),
            r0,
            r1: rng.random_range(r0 + 0.14..SPOKE_BAND.1),
            age: 0.0,
            life: rng.random_range(SPOKE_LIFE.0..SPOKE_LIFE.1),
        });
    }

    /// Spoke darkening at `(r, λ)`: 0 none, up to `SPOKE_DIM`.
    fn spoke_dim(&self, r: f64, lambda: f64) -> f64 {
        let mut dim: f64 = 0.0;
        for s in &self.spokes {
            if r < s.r0 || r > s.r1 {
                continue;
            }
            // Wedge-shaped: wider outward.
            let half = s.half * (0.6 + 0.8 * (r - s.r0) / (s.r1 - s.r0));
            let d = wrap_angle(lambda - s.angle(self.t)).abs();
            if d < half {
                let edge = 1.0 - d / half;
                dim = dim.max(SPOKE_DIM * s.strength() * edge.sqrt());
            }
        }
        dim
    }

    fn shade_rings(&mut self) {
        let globe = &self.globe;
        self.pixels
            .shade(|x, y| globe.get(x, y).unwrap_or(Ink::BLANK));
        let t = self.t;
        let daphnis = kepler_rate(0.5 * (KEELER.0 + KEELER.1)) * t + 1.3;
        let w = usize::from(self.pixels.pixel_size().width()).max(1);
        for k in 0..self.ring_px.len() {
            let p = self.ring_px[k];
            let (r, lambda) = (f64::from(p.r), f64::from(p.lambda));
            let mut b = f64::from(p.b0);
            if p.k > 0.0 {
                // Daphnis's waves displace the Keeler gap's edges: leading on the
                // faster inner edge, trailing on the slower outer one (RING-A5).
                let lag = if r < 0.5 * (KEELER.0 + KEELER.1) {
                    lambda - daphnis
                } else {
                    daphnis - lambda
                };
                let (s, c) = lambda.sin_cos_det();
                let foot = (c.abs() + s.abs() / TILT.sin_det()) / self.view.scale;
                b = self.brightness_over(r - edge_wave(lag), foot) * f64::from(p.k);
            }
            if b <= 0.0 {
                continue;
            }
            // Clumpy texture sheared at the Keplerian rate (RING-A2).
            let phase = (lambda - kepler_rate(r) * t).rem_euclid(TAU) / TAU;
            let tex = value_noise_wrap_x(0x7E47, phase * 64.0, r * 32.0, 64);
            b *= 0.72 + 0.5 * tex;
            b *= 1.0 - self.spoke_dim(r, lambda);
            let ring_lum = (0.06 + 0.8 * b).min(1.0);
            let (x, y) = ((p.idx as usize % w) as i32, (p.idx as usize / w) as i32);
            let under = self.globe.get(x, y).unwrap_or(Ink::BLANK);
            let (cover, far) = (f64::from(p.cover.abs()), p.cover < 0.0);
            // How much of the pixel the ring owns: on the far side, what the globe
            // leaves uncovered; on the near side, its opacity over the globe (the thin
            // C ring lets the globe show through), and all of it off the globe.
            let alpha = if far {
                1.0 - cover
            } else {
                let opaque = ((b - 0.08) / 0.3).clamp(0.0, 1.0);
                opaque + (1.0 - opaque) * (1.0 - cover)
            };
            let under_lum = if under.role == Role::Ramp(GLOBE) {
                f64::from(under.level) / 255.0
            } else {
                0.0
            };
            let lum = alpha * ring_lum + (1.0 - alpha) * under_lum;
            // One ink per pixel: the ramp of whichever owns more of it, at the blended
            // brightness, so the globe's edge steps smoothly.
            let ink = if alpha >= 0.5 || under.role != Role::Ramp(GLOBE) {
                ink::ramp(Ramp::Starlight, lum)
            } else {
                ink::ramp(GLOBE, lum)
            };
            if lum * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                self.pixels.set(x, y, ink);
            }
        }
        // Daphnis itself: a lone bright dot in the Keeler gap.
        let sin_t = TILT.sin_det();
        let rd = 0.5 * (KEELER.0 + KEELER.1);
        let (s, c) = daphnis.sin_cos_det();
        let (px, py) = (
            self.view.cx + self.view.scale * rd * c,
            self.view.cy - self.view.scale * rd * s * sin_t,
        );
        let hidden = s > 0.0 && Self::globe_point(rd * c, rd * s * sin_t).is_some();
        if !hidden {
            self.pixels.set(
                px.floor() as i32,
                py.floor() as i32,
                ink::ramp(Ramp::Starlight, 1.0),
            );
        }
    }
}

impl Generator for SaturnRings {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.globe.resize(size);
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        let scale = (0.485 * w / R_OUT).min(0.44 * h / POLAR.max(R_OUT * TILT.sin_det()));
        self.view = View {
            cx: 0.5 * w,
            cy: 0.5 * h,
            scale: scale.max(0.5),
        };
        self.shade_globe();
        self.build_rings();
        self.shade_rings();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        self.t += dt;
        if self.t <= dt && self.spokes.is_empty() {
            // Open with a pair of spokes already on the ansa.
            self.spawn_spoke(&mut ctx.rng);
            self.spawn_spoke(&mut ctx.rng);
            for s in &mut self.spokes {
                s.age = 0.3 * s.life;
            }
        }
        for s in &mut self.spokes {
            s.age += dt;
        }
        self.spokes.retain(|s| s.age < s.life);
        // About one new spoke every 1.5 s.
        if dt > 0.0 && ctx.rng.random_bool((dt / 1.5).min(1.0)) {
            self.spawn_spoke(&mut ctx.rng);
        }
        self.shade_rings();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_a1_gaps_are_nearly_empty() {
        for r in [
            0.5 * (CASSINI.0 + CASSINI.1),
            0.5 * (ENCKE.0 + ENCKE.1),
            0.5 * (KEELER.0 + KEELER.1),
        ] {
            assert!(tau(r) < 0.1, "τ({r}) = {}", tau(r));
        }
        // …between rings that are not.
        assert!(tau(1.75) > 1.0 && tau(2.1) > 0.4);
    }

    #[test]
    fn ring_a2_texture_shears_at_the_keplerian_rate() {
        let (inner_b, outer_a) = (1.53, 2.26);
        let ratio = kepler_rate(inner_b) / kepler_rate(outer_a);
        let want = (outer_a / inner_b).powf(1.5);
        assert!((ratio / want - 1.0).abs() < 0.05, "{ratio} vs {want}");
        assert!(ratio > 1.0, "inner ring must lead");
    }

    #[test]
    fn ring_a3_spokes_corotate_with_the_planet() {
        let s = Spoke {
            lambda: 0.3,
            half: 0.05,
            r0: 1.6,
            r1: 1.85,
            age: 0.0,
            life: 8.0,
        };
        let rate = (s.angle(2.0) - s.angle(1.0)) / 1.0;
        assert!((rate - OMEGA_SATURN).abs() < 1e-12);
        assert!((rate - kepler_rate(s.r0)).abs() > 0.1 * OMEGA_SATURN);
    }

    #[test]
    fn ring_a4_the_globe_hides_and_shadows_the_far_rings() {
        let mut g = SaturnRings::new();
        let mut ctx = Ctx::new(42, Size::new(100, 40));
        g.resize(Size::new(100, 40), &mut ctx);
        // The far ring straight behind the globe's centre shows the globe, not ring.
        let (st, v) = (TILT.sin_det(), g.view);
        let far = |r: f64| (v.cx as i32, (v.cy - v.scale * r * st).floor() as i32);
        let (x, y) = far(1.8);
        let px = g.pixels.get(x, y).expect("on screen");
        assert_eq!(px.role, Role::Ramp(GLOBE), "{px:?}");
        // Anti-sunward far-side ring is in the shadow, its mirror image is lit.
        let l = sun();
        let anti = [-l[0], -l[1]];
        let n = anti[0].hypot(anti[1]);
        let p = [1.8 * anti[0] / n, 1.8 * anti[1] / n, 0.0];
        assert!(p[1] > 0.0, "the shadow falls on the far side");
        assert!(in_planet_shadow(p));
        assert!(!in_planet_shadow([-p[0], -p[1], 0.0]));
        assert!(!in_planet_shadow([2.2, -0.3, 0.0]));
    }

    #[test]
    fn ring_a5_edge_waves_decay_downstream() {
        let envelope = |lag: f64| {
            (0..64)
                .map(|k| edge_wave(lag + f64::from(k) * TAU / WAVE_K / 64.0).abs())
                .fold(0.0, f64::max)
        };
        let (near, mid, far) = (envelope(0.05), envelope(1.0), envelope(3.0));
        assert!(near > mid && mid > far, "{near} {mid} {far}");
        assert!(near > 0.5 * WAVE_AMP);
    }
}
