//! A ray-traced Schwarzschild black hole with a thin accretion disk and a lensed sky
//! (SPEC-002 `black_hole`, after Luminet 1979).
//!
//! Units are `G = c = M = 1`: the horizon is at `r = 2`, the photon sphere at 3, the ISCO
//! at 6, and the shadow's edge at impact parameter `b_c = 3√3`. The camera looks along
//! `−ô` (`ô = +z` points at the observer); screen x is right, y up. The disk plane is
//! tilted by `INCLINATION`: its normal is `n̂ = (0, sin i, cos i)`, and disk azimuth `φ`
//! is measured from `x̂` towards `d̂ = n̂ × x̂ = (0, cos i, −sin i)`, so `φ = π/2` lies
//! behind the hole and above it on screen. Gas orbits towards increasing `φ`: the left
//! side approaches.
//!
//! Null geodesics come from a size-independent table (`Geodesics`) built once; `resize`
//! traces every lattice node through it into hit lists and sky texels, and each step only
//! advects the clump texture, sums hits and looks up the sky (GEN-R8, GEN-R11).

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::grid::lattice_for;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;
use sciart_core::{geom, ink};

pub static META: Meta = Meta {
    name: "black_hole",
    category: Category::DeepCosmos,
    description: "A ray-traced black hole: lensed accretion disk, lensing ring and a lensed sky",
    title: "Black hole · infalling gas glows",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(BlackHole::default())
}

const INCLINATION: f64 = 80.0 * PI / 180.0;
/// Critical impact parameter `3√3`: the edge of the shadow.
const B_CRIT: f64 = 5.196_152_422_706_632;
/// Inner (ISCO) and outer disk edge.
const ISCO: f64 = 6.0;
const R_OUT: f64 = 24.0;
const ISCO_PERIOD: f64 = 2.5;
/// Viscous inward drift of the gas, `dr/dt = −DRIFT r` (1/s).
const DRIFT: f64 = 0.02;

/// Emitted temperature at the peak of the disk profile (stylised, SPEC-002).
const T_PEAK: f64 = 6000.0;
/// Radius of that peak: `d/dr [r⁻³ (1 − √(r_in/r))] = 0` at `r = (7/6)² r_in`.
const R_PEAK: f64 = 49.0 / 36.0 * ISCO;
/// Display temperature `T_PEAK · F^DISPLAY_EXP` of the summed flux `F` (in units of the
/// peak's, `F = (gT/T_PEAK)⁴`): `3/4` instead of the physical `1/4` stretches the
/// contrast to `T_disp = T_PEAK (gT/T_PEAK)³` (stylised, SPEC-002), so the disk runs from
/// white through gold to ember instead of one tan band.
const DISPLAY_EXP: f64 = 0.75;
/// Above `T_KNEE` the display temperature rolls off (Reinhard, in log T) towards
/// `T_WHITE`, so the Doppler-boosted side keeps a gradient instead of clipping to a flat
/// white plateau; only the hottest arc gets near white.
const T_KNEE: f64 = 4500.0;
const T_WHITE: f64 = 10_000.0;
/// Below `T_KNEE` the log temperature is lifted by this power, raising the dim outer disk
/// from brown towards ember.
const TOE: f64 = 0.8;
/// Pixels whose stretched temperature is below this show the sky instead; from here the
/// toe maps up from the coolest blackbody level, so the dim receding side still shows as
/// a continuous glow round its clumps.
const T_SHOW: f64 = 600.0;
/// Observer distance: the screen angle of a ray is `b / OBSERVER`.
const OBSERVER: f64 = 60.0;
/// The star field drifts round the disk axis (rad/s; stylised).
const SKY_SPIN: f64 = TAU / 150.0;

/// Cap on traced lattice nodes (ADR-0012): the 200×50 display's 20 000 pixels fit.
const MAX_NODES: usize = 24_000;
/// Supersamples per node axis (the lensing ring and stars are thinner than a pixel).
const SS: usize = 2;
/// The fit: the disk's outer edge spans this much of the width, and the image keeps a
/// cell clear above it and two below it (the title row and a gap; GEN-R13).
const FILL: f64 = 0.9;
const MARGIN_TOP: f64 = 2.0;
const MARGIN_BOTTOM: f64 = 4.0;

/// Geodesic table: impact parameters, asinh-spaced about `B_CRIT` with width `B_W`, and
/// sweep angles `0..=2π`.
const NB: usize = 320;
const NPHI: usize = 257;
const B_MIN: f64 = 0.02;
const B_MAX: f64 = 48.0;
const B_W: f64 = 0.25;
const SUBSTEPS: usize = 3;

/// Polar clump texture: bins in `ln r` and in azimuth.
const NR: usize = 48;
const NA: usize = 256;
// Clump texels are indexed by `u16` (`Hit::texel`).
const _: () = assert!(NR * NA <= u16::MAX as usize + 1);
/// Clump pattern: lifetime of one layer (s), log-normal contrast, and seeded blobs.
const LIFE: f64 = 6.0;
const CONTRAST: f64 = 0.45;
const BLOBS: usize = 180;
/// The clump contrast fades to zero from here to `R_OUT`, inside the feathered rim.
const CLUMP_EDGE: f64 = 0.8 * R_OUT;

/// Star texture (equirectangular, one texel per star).
const STAR_W: usize = 1024;
const STAR_H: usize = 512;
const STARS: usize = 2400;
/// Star levels run from `STAR_DIM` to `STAR_BRIGHT`, below the disk's hottest level in
/// every theme (BH-A8).
const STAR_DIM: f64 = 40.0;
const STAR_BRIGHT: f64 = 150.0;
/// The lensing ring is thinner than a pixel: nodes within `RING_BAND` outside the shadow
/// trace their order-1 image with `RING_SS`² samples, and a node it touches shows it at
/// full coverage, so it reads as a one-pixel line, not specks (stylised, SPEC-002).
const RING_SS: usize = 6;
const RING_BAND: f64 = 0.8;

/// `GM` in `M` units rescaled so the ISCO period is exactly `ISCO_PERIOD` (BH-A1).
pub(crate) fn gm() -> f64 {
    let w = TAU / ISCO_PERIOD;
    w * w * ISCO * ISCO * ISCO
}

/// Keplerian angular velocity of the texture gas at radius `r`, in rad/s (BH-A1).
pub(crate) fn omega(r: f64) -> f64 {
    sciart_core::particles::kepler_omega(gm(), r)
}

/// Emitted temperature at radius `r`, `T ∝ [r⁻³ (1 − √(6/r))]^¼`, scaled so the peak is
/// `T_PEAK`; 0 inside the ISCO (BH-A7).
pub(crate) fn temperature(r: f64) -> f64 {
    T_PEAK * (flux_profile(r) / flux_profile(R_PEAK)).sqrt().sqrt()
}

/// `r⁻³ (1 − √(r_in / r))`, the Newtonian (Shakura–Sunyaev) thin-disk flux shape with a
/// zero-torque inner edge.
fn flux_profile(r: f64) -> f64 {
    if r <= ISCO {
        0.0
    } else {
        (1.0 - (ISCO / r).sqrt()) / (r * r * r)
    }
}

/// Where the disk's outer rim starts to feather into space (stylised, SPEC-002).
const FEATHER: f64 = 0.55 * R_OUT;

/// Emission weight of the outer rim: 1 inside `FEATHER`, easing to 0 at `R_OUT`, so the
/// disk fades out instead of ending on a hard ellipse.
fn feather(r: f64) -> f64 {
    let s = ((r - FEATHER) / (R_OUT - FEATHER)).clamp(0.0, 1.0);
    (1.0 - s * s) * (1.0 - s * s)
}

/// Redshift factor `g = ν_obs / ν_emit` of disk gas at `r` seen along a ray with impact
/// parameter `b` at screen angle `ψ` (BH-A3, BH-A7): `√(1 − 3/r)` is the gravitational
/// redshift `√(1 − 2/r)` times the transverse Doppler (time-dilation) shift of the
/// orbiting gas; `1 / (1 − Ω λ)` is the line-of-sight Doppler shift, with the photon's
/// `λ = L_z / E = −b cos ψ sin i` and the true (geometric) Kepler `Ω = r^{-3/2}`.
pub(crate) fn redshift(r: f64, b: f64, psi: f64) -> f64 {
    let omega = r.powf(-1.5);
    (1.0 - 3.0 / r).max(0.0).sqrt() / (1.0 + omega * b * psi.cos() * INCLINATION.sin())
}

/// Sweep angle at which a ray from the observer at screen angle `psi` first reaches the
/// disk plane: `r̂(γ) = cos γ ô + sin γ ê` meets `n̂·r̂ = 0` (SPEC-002).
fn first_crossing(psi: f64) -> f64 {
    INCLINATION.cos().atan2(-psi.sin() * INCLINATION.sin())
}

/// Disk azimuth of the ray's position at sweep angle `gamma` (on the disk plane).
fn disk_azimuth(psi: f64, gamma: f64) -> f64 {
    let (sg, cg) = gamma.sin_cos();
    let (sp, cp) = psi.sin_cos();
    let along_d = sg * sp * INCLINATION.cos() - cg * INCLINATION.sin();
    geom::wrap(along_d.atan2(sg * cp), TAU)
}

/// Where a traced ray ends up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Fate {
    /// Hits the disk at radius `r`, azimuth `phi`, on crossing `order` (0 direct image,
    /// 1 secondary image: the lensing ring and the lower arc).
    Disk { r: f64, phi: f64, order: u8 },
    /// Escapes to infinity after sweeping `phi_end` (deflection `phi_end − π`).
    Sky { phi_end: f64 },
    /// Falls into the hole, or is still circling the photon sphere after 2π.
    Dark,
}

/// Schwarzschild null geodesics `u'' = −u + 3u²` (`u = 1/r`) for rays arriving from the
/// observer, tabulated over impact parameter and sweep angle (BH-A5).
#[derive(Debug)]
pub(crate) struct Geodesics {
    /// `u(b_k, φ_j)`, row-major by `k`.
    u: Box<[f32]>,
    /// Sweep angle at which ray `k` is captured (`u = ½`) or escapes (`u = 0`); `> 2π`
    /// when it is still circling at the end of the table.
    end: Box<[f32]>,
}

impl Geodesics {
    const DPHI: f64 = TAU / (NPHI - 1) as f64;

    fn t_of(b: f64) -> f64 {
        ((b - B_CRIT) / B_W).asinh()
    }

    /// The table's range in the asinh coordinate.
    fn t_range() -> (f64, f64) {
        (Self::t_of(B_MIN), Self::t_of(B_MAX))
    }

    fn b_at(k: usize) -> f64 {
        let (t0, t1) = Self::t_range();
        B_CRIT + B_W * (t0 + (t1 - t0) * k as f64 / (NB - 1) as f64).sinh()
    }

    pub(crate) fn new() -> Self {
        let mut u = vec![0.0f32; NB * NPHI].into_boxed_slice();
        let mut end = vec![0.0f32; NB].into_boxed_slice();
        let h = Self::DPHI / SUBSTEPS as f64;
        let binet = |_: f64, y: &[f64; 2]| [y[1], -y[0] + 3.0 * y[0] * y[0]];
        for (k, (row, end)) in u.chunks_exact_mut(NPHI).zip(end.iter_mut()).enumerate() {
            let mut y = [0.0, 1.0 / Self::b_at(k)];
            let mut phi = 0.0;
            *end = (TAU + 1.0) as f32;
            'sweep: for j in 1..NPHI {
                for _ in 0..SUBSTEPS {
                    let prev = y[0];
                    sciart_core::ode::rk4_step(&mut y, phi, h, binet);
                    phi += h;
                    // Capture at the horizon or escape back to u = 0: interpolate the angle,
                    // hold the final value for the rest of the row.
                    let target = if y[0] >= 0.5 {
                        0.5
                    } else if y[0] <= 0.0 {
                        0.0
                    } else {
                        continue;
                    };
                    let f = (target - prev) / (y[0] - prev);
                    *end = (phi - h + f * h) as f32;
                    row[j..].fill(target as f32);
                    break 'sweep;
                }
                row[j] = y[0] as f32;
            }
        }
        Self { u, end }
    }

    /// Fractional table row of `b`, or `None` beyond `B_MAX` (weak field).
    fn row(b: f64) -> Option<(usize, f64)> {
        if b > B_MAX {
            return None;
        }
        let (t0, t1) = Self::t_range();
        let x = ((Self::t_of(b) - t0) / (t1 - t0) * (NB - 1) as f64).clamp(0.0, (NB - 1) as f64);
        let k = (x as usize).min(NB - 2);
        Some((k, x - k as f64))
    }

    /// Sweep angle at which the ray with impact parameter `b` is captured or escapes.
    pub(crate) fn end(&self, b: f64) -> f64 {
        match Self::row(b) {
            Some((k, f)) => {
                let (a, c) = (f64::from(self.end[k]), f64::from(self.end[k + 1]));
                a + (c - a) * f
            }
            // Beyond the table the deflection is the weak-field series to second order.
            None => PI + 4.0 / b + 15.0 * PI / (4.0 * b * b),
        }
    }

    /// `u = 1/r` of the ray with impact parameter `b` after sweeping `phi ∈ [0, 2π]`.
    pub(crate) fn u(&self, b: f64, phi: f64) -> f64 {
        let Some((k, fb)) = Self::row(b) else {
            // u = sin φ / b + (1 − cos φ)² / b², first order in M/b.
            return phi.sin() / b + (1.0 - phi.cos()).powi(2) / (b * b);
        };
        let x = (phi / Self::DPHI).clamp(0.0, (NPHI - 1) as f64);
        let j = (x as usize).min(NPHI - 2);
        let fp = x - j as f64;
        let at = |k: usize, j: usize| f64::from(self.u[k * NPHI + j]);
        let lo = at(k, j) + (at(k, j + 1) - at(k, j)) * fp;
        let hi = at(k + 1, j) + (at(k + 1, j + 1) - at(k + 1, j)) * fp;
        lo + (hi - lo) * fb
    }

    /// Trace a camera ray with impact parameter `b` at screen angle `psi` (BH-A2, BH-A6).
    pub(crate) fn trace(&self, b: f64, psi: f64) -> Fate {
        let end = self.end(b);
        let gamma0 = first_crossing(psi);
        for order in 0..2u8 {
            let gamma = gamma0 + f64::from(order) * PI;
            if gamma >= end {
                break;
            }
            let u = self.u(b, gamma);
            if u > 0.0 {
                let r = 1.0 / u;
                if (ISCO..=R_OUT).contains(&r) {
                    let phi = disk_azimuth(psi, gamma);
                    return Fate::Disk { r, phi, order };
                }
            }
        }
        if b > B_CRIT && end <= TAU {
            Fate::Sky { phi_end: end }
        } else {
            Fate::Dark
        }
    }

    /// Half-width, top and bottom of the disk's image in `M` (the fit, GEN-R13).
    fn extent(&self) -> (f64, f64, f64) {
        let (mut half_w, mut top, mut bottom) = (0.0f64, 0.0f64, 0.0f64);
        for m in 0..48 {
            let psi = TAU * f64::from(m) / 48.0;
            // Outermost disk hit along this screen direction.
            let on_disk = |b: f64| matches!(self.trace(b, psi), Fate::Disk { .. });
            // A coarse scan inwards, then a fine one over the last coarse step.
            let Some(coarse) = (0..100)
                .map(|k| 40.0 - 0.4 * f64::from(k))
                .find(|&b| on_disk(b))
            else {
                continue;
            };
            let b = (0..5)
                .map(|k| coarse + 0.32 - 0.08 * f64::from(k))
                .find(|&b| on_disk(b))
                .unwrap_or(coarse);
            let (x, y) = (b * psi.cos(), b * psi.sin());
            half_w = half_w.max(x.abs());
            top = top.max(y);
            bottom = bottom.max(-y);
        }
        (half_w, top, bottom)
    }
}

/// Unit sky coordinates `(lon / 2π, ½ − lat / π)` of the direction an escaped ray came
/// from: the source sits at angle `b/D − α` from the hole, on the ray's side (SPEC-002).
fn sky_coords(b: f64, psi: f64, phi_end: f64) -> (f64, f64) {
    let delta = b / OBSERVER - (phi_end - PI);
    let (sd, cd) = delta.sin_cos();
    let (sp, cp) = psi.sin_cos();
    // dir = −cos δ ô + sin δ ê, ê = (cos ψ, sin ψ, 0), ô = (0, 0, 1).
    let dir = [sd * cp, sd * sp, -cd];
    let (si, ci) = INCLINATION.sin_cos();
    let along_x = dir[0];
    let along_d = dir[1] * ci - dir[2] * si;
    let along_n = dir[1] * si + dir[2] * ci;
    let lon = geom::wrap(along_d.atan2(along_x), TAU);
    let lat = along_n.clamp(-1.0, 1.0).asin();
    (lon / TAU, 0.5 - lat / PI)
}

/// Star texel `y · STAR_W + x` holding unit sky coordinates `(u, v)`.
fn star_texel(u: f64, v: f64) -> u32 {
    let x = ((u * STAR_W as f64) as usize).min(STAR_W - 1);
    let y = ((v * STAR_H as f64) as usize).min(STAR_H - 1);
    (y * STAR_W + x) as u32
}

/// How far `(u, v)` lies from its star texel's centre, as the Chebyshev distance in
/// half-texels: 0 at the centre, 1 on the texel's edge.
fn off_centre(u: f64, v: f64) -> f64 {
    let f = |t: f64, n: usize| (2.0 * (t * n as f64).fract() - 1.0).abs();
    f(u, STAR_W).max(f(v, STAR_H))
}

/// The Milky Way's pole: its band is the great circle normal to it, tilted against the
/// disk plane.
fn band_pole() -> [f64; 3] {
    geom::v3::unit([0.35, 0.45, 0.82])
}

/// The stars: one texel each, `Role::Star` levels; a quarter crowd the band.
fn paint_stars(rng: &mut impl Rng) -> Box<[u8]> {
    let pole = band_pole();
    let e1 = geom::v3::unit(geom::v3::cross(pole, [0.0, 0.0, 1.0]));
    let e2 = geom::v3::cross(pole, e1);
    let mut stars = vec![0u8; STAR_W * STAR_H].into_boxed_slice();
    for k in 0..STARS {
        let (lon, lat) = if k % 4 == 0 {
            let a = rng.random_range(0.0..TAU);
            let off = 0.12 * (rng.random_range(-1.0f64..1.0)).powi(3);
            let p = geom::v3::add(
                geom::v3::scale(e1, a.cos() * off.cos()),
                geom::v3::axpy(a.sin() * off.cos(), e2, geom::v3::scale(pole, off.sin())),
            );
            (p[1].atan2(p[0]), p[2].clamp(-1.0, 1.0).asin())
        } else {
            (
                rng.random_range(0.0..TAU),
                (rng.random_range(-1.0f64..1.0)).asin(),
            )
        };
        let u = rng.random_range(0.0f64..1.0);
        let level = (STAR_DIM + (STAR_BRIGHT - STAR_DIM) * u.powi(5)) as u8;
        let i = star_texel(geom::wrap(lon, TAU) / TAU, 0.5 - lat / PI) as usize;
        stars[i] = stars[i].max(level);
    }
    stars
}

/// Continuous row of the clump texture at disk radius `r`: 0 at the ISCO, `NR` at the
/// outer edge, uniform in `ln r`.
fn radial_coord(r: f64) -> f64 {
    (r / ISCO).ln() / (R_OUT / ISCO).ln() * NR as f64
}

/// Clump-texture row holding radius `r ∈ [ISCO, R_OUT]`.
fn radial_bin(r: f64) -> usize {
    (radial_coord(r).max(0.0) as usize).min(NR - 1)
}

/// Where the gas at texture position `(row, az)` (continuous, `az` in texels) was when
/// its clump layer was born `age` s ago (BH-A4): it drifted in as `r = r₀ e^{−DRIFT t}`
/// and orbited at the Kepler `ω ∝ r^{-3/2}`, so it came from `DRIFT · age` further out in
/// `ln r` and turned by `ω(r) (1 − e^{−1.5 DRIFT age}) / (1.5 DRIFT)` since.
fn birth(row: f64, az: f64, age: f64) -> (f64, f64) {
    let r = ISCO * (R_OUT / ISCO).powf(row / NR as f64);
    let rows = DRIFT * age / (R_OUT / ISCO).ln() * NR as f64;
    let k = 1.5 * DRIFT;
    let turned = omega(r) * (1.0 - (-k * age).exp()) / k;
    (row + rows, az - turned / TAU * NA as f64)
}

/// The two clump layers at time `t`: age, weight and generation of each. They are born
/// `LIFE / 2` apart and weighted `sin(π age / LIFE)`, so `w₀² + w₁² = 1` and the clump
/// contrast holds steady while each layer is sheared and replaced (BH-A4).
fn layers(t: f64) -> [(f64, f64, u64); 2] {
    std::array::from_fn(|j| {
        let shifted = t.max(0.0) + j as f64 * 0.5 * LIFE;
        let generation = (shifted / LIFE).floor();
        let age = shifted - generation * LIFE;
        (age, (PI * age / LIFE).sin(), generation as u64)
    })
}

/// Seeded clump noise over `(ln r, φ)`, normalised to mean 0 and unit variance: blobs
/// stretched along the orbit, a third of them large.
fn paint_clumps(rng: &mut impl Rng) -> Box<[f32]> {
    let mut n = vec![0.0f64; NR * NA];
    for k in 0..BLOBS {
        let (r0, a0) = (
            rng.random_range(0.0..NR as f64),
            rng.random_range(0.0..NA as f64),
        );
        let (sr, sa) = if k % 3 == 0 {
            (rng.random_range(1.5..3.5), rng.random_range(8.0..24.0))
        } else {
            (rng.random_range(0.7..1.5), rng.random_range(2.5..8.0))
        };
        let amp = rng.random_range(-1.0..1.0);
        let (reach_r, reach_a) = ((2.5 * sr) as i32, (2.5 * sa) as i32);
        for dr in -reach_r..=reach_r {
            let row = r0.floor() as i32 + dr;
            if !(0..NR as i32).contains(&row) {
                continue;
            }
            let er = ((f64::from(row) + 0.5 - r0) / sr).powi(2);
            for da in -reach_a..=reach_a {
                let col = a0.floor() as i32 + da;
                let ea = ((f64::from(col) + 0.5 - a0) / sa).powi(2);
                let i = row as usize * NA + col.rem_euclid(NA as i32) as usize;
                n[i] += amp * (-er - ea).exp();
            }
        }
    }
    let mean = n.iter().sum::<f64>() / n.len() as f64;
    let var = n.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n.len() as f64;
    let inv = 1.0 / var.sqrt().max(1e-12);
    n.iter().map(|v| ((v - mean) * inv) as f32).collect()
}

/// Bilinear sample of a `w`-wide, row-major texture (at least 2 rows) at continuous
/// texel coordinates `(x, y)`, texel centres at `i + ½`: x (azimuth) wraps, y clamps.
fn bilerp(tex: &[f32], w: usize, x: f64, y: f64) -> f64 {
    let h = tex.len() / w;
    let x = geom::wrap(x - 0.5, w as f64);
    let y = (y - 0.5).clamp(0.0, (h - 1) as f64);
    let (x0, y0) = ((x as usize).min(w - 1), (y as usize).min(h - 2));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let x1 = (x0 + 1) % w;
    let at = |x: usize, y: usize| tex.get(y * w + x).map_or(0.0, |&v| f64::from(v));
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let bottom = at(x0, y0 + 1) + (at(x1, y0 + 1) - at(x0, y0 + 1)) * fx;
    top + (bottom - top) * fy
}

/// One supersample of a node that lands on the disk.
#[derive(Debug, Clone, Copy)]
struct Hit {
    node: u32,
    /// Clump texel index `ir · NA + ia`.
    texel: u16,
    /// `(g T(r) / T_PEAK)⁴ · feather` over the node's sample count.
    weight: f32,
}

impl Hit {
    /// The hit of a ray `(b, psi)` on the disk at `(r, phi)`, one of `samples` per node.
    fn new(node: u32, r: f64, phi: f64, b: f64, psi: f64, samples: usize) -> Self {
        let t = redshift(r, b, psi) * temperature(r) / T_PEAK;
        let ia = ((phi / TAU * NA as f64) as usize).min(NA - 1);
        Self {
            node,
            texel: (radial_bin(r) * NA + ia) as u16,
            weight: (t * t * t * t * feather(r) / samples as f64) as f32,
        }
    }
}

/// Trace the lensing ring (the order-1 image) through `node` with `RING_SS`² samples;
/// `at(a, c)` gives `(b, ψ)` at fractional node offsets. The node shows the ring's mean
/// light as if it covered the node fully.
fn trace_ring(
    geo: &Geodesics,
    hits: &mut Vec<Hit>,
    node: u32,
    at: impl Fn(f64, f64) -> (f64, f64),
) {
    let start = hits.len();
    let n = RING_SS * RING_SS;
    for s in 0..n {
        let (a, c) = (s % RING_SS, s / RING_SS);
        let (b, psi) = at(
            (a as f64 + 0.5) / RING_SS as f64,
            (c as f64 + 0.5) / RING_SS as f64,
        );
        if let Fate::Disk { r, phi, order: 1 } = geo.trace(b, psi) {
            hits.push(Hit::new(node, r, phi, b, psi, 1));
        }
    }
    // The mean over the ring's own samples: full coverage, however thin it is.
    let inv = 1.0 / (hits.len() - start).max(1) as f32;
    for h in &mut hits[start..] {
        h.weight *= inv;
    }
}

/// A node's star texels, one per supersample: the star texel whose point image this
/// supersample carries, or `NO_STAR`. The sky's drift is a whole number of texels, so
/// they are fixed per resize (BH-A8).
type SkyRay = [u32; SS * SS];
const NO_STAR: u32 = u32::MAX;

#[derive(Debug)]
pub struct BlackHole {
    t: f64,
    geo: Option<Geodesics>,
    /// The disk image's half-width, top and bottom in `M`.
    extent: (f64, f64, f64),
    stars: Box<[u8]>,
    /// Clump noise and its per-generation azimuth salt.
    clump_base: Box<[f32]>,
    clump_salt: u64,
    /// The current clump multiplier over `(ln r, φ)`, log-normal with mean 1.
    tex: Box<[f32]>,
    hits: Vec<Hit>,
    sky: Vec<SkyRay>,
    flux: Grid<f32>,
    star_light: Grid<f32>,
    levels: FluxLevels,
    map: SampleMap,
    pixels: Pixels,
}

impl Default for BlackHole {
    fn default() -> Self {
        Self {
            t: 0.0,
            geo: None,
            extent: (R_OUT, R_OUT, R_OUT),
            stars: Box::default(),
            clump_base: Box::default(),
            clump_salt: 0,
            tex: Box::default(),
            hits: Vec::new(),
            sky: Vec::new(),
            flux: Grid::default(),
            star_light: Grid::default(),
            levels: FluxLevels::new(),
            map: SampleMap::default(),
            pixels: Pixels::default(),
        }
    }
}

impl BlackHole {
    fn seed(&mut self, ctx: &mut Ctx) {
        let geo = Geodesics::new();
        self.extent = geo.extent();
        self.geo = Some(geo);
        self.stars = paint_stars(&mut ctx.rng);
        self.clump_base = paint_clumps(&mut ctx.rng);
        self.clump_salt = ctx.rng.random();
        self.tex = vec![1.0; NR * NA].into_boxed_slice();
    }

    /// The camera fit: pixels per `M` and the hole's pixel position. The disk's width or
    /// the image's height binds, whichever is tighter (GEN-R13).
    fn fit(&self) -> (f64, f64, f64) {
        let px = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(px.width()), f64::from(px.height()));
        let (half_w, top, bottom) = self.extent;
        let room = (ph - MARGIN_TOP - MARGIN_BOTTOM).max(1.0);
        let scale = (FILL * pw / (2.0 * half_w)).min(room / (top + bottom));
        let cy = MARGIN_TOP + 0.5 * (room - (top + bottom) * scale) + top * scale;
        (scale, pw / 2.0, cy)
    }

    /// Trace every lattice node into hit lists and sky texels.
    fn trace(&mut self) {
        let Some(geo) = &self.geo else { return };
        let px = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(px.width()), f64::from(px.height()));
        let lattice = lattice_for(px, MAX_NODES, 1.0);
        let (lw, lh) = (usize::from(lattice.width()), usize::from(lattice.height()));
        let (scale, cx, cy) = self.fit();
        let (sx, sy) = (pw / lw as f64, ph / lh as f64);
        let polar = |x: f64, y: f64| {
            let (bx, by) = ((x - cx) / scale, (cy - y) / scale);
            (bx.hypot(by), by.atan2(bx))
        };

        self.hits.clear();
        self.sky.clear();
        // A node's width in `M`, for the lensing-ring band.
        let node_m = sx.max(sy) / scale;
        // Per escaping supersample: its image key (star texel · 2, plus 1 above the hole),
        // its distance from the texel's centre and its slot in `sky`.
        let mut escapes: Vec<(u32, f32, u32)> = Vec::with_capacity(lw * lh * SS * SS);
        for j in 0..lh {
            for i in 0..lw {
                let node = (j * lw + i) as u32;
                let centre = polar((i as f64 + 0.5) * sx, (j as f64 + 0.5) * sy);
                let ring = (-node_m..RING_BAND + node_m).contains(&(centre.0 - B_CRIT));
                for s in 0..SS * SS {
                    let (a, c) = (s % SS, s / SS);
                    let x = (i as f64 + (a as f64 + 0.5) / SS as f64) * sx;
                    let y = (j as f64 + (c as f64 + 0.5) / SS as f64) * sy;
                    let (b, psi) = polar(x, y);
                    match geo.trace(b, psi) {
                        // Near the shadow the order-1 image is traced more finely below.
                        Fate::Disk { order: 1, .. } if ring => {}
                        Fate::Disk { r, phi, .. } => {
                            self.hits.push(Hit::new(node, r, phi, b, psi, SS * SS));
                        }
                        Fate::Sky { phi_end } => {
                            let (u, v) = sky_coords(b, psi, phi_end);
                            let slot = node * (SS * SS) as u32 + s as u32;
                            let key = 2 * star_texel(u, v) + u32::from(psi >= 0.0);
                            escapes.push((key, off_centre(u, v) as f32, slot));
                        }
                        Fate::Dark => {}
                    }
                }
                if ring {
                    trace_ring(geo, &mut self.hits, node, |a, c| {
                        polar((i as f64 + a) * sx, (j as f64 + c) * sy)
                    });
                }
            }
        }
        // Stars are points (BH-A8): of the supersamples that see a texel on one side of the
        // hole, only the one nearest its centre carries its star. A star's direct and
        // lensed images lie on opposite sides, so each image is exactly one pixel, however
        // strongly lensing stretches its texel, and the images do not trade places as the
        // sky turns.
        self.sky.resize(lw * lh, [NO_STAR; SS * SS]);
        escapes.sort_unstable_by(|a, b| {
            (a.0.cmp(&b.0))
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        for run in escapes.chunk_by(|a, b| a.0 == b.0) {
            let (key, _, slot) = run[0];
            let slot = slot as usize;
            self.sky[slot / (SS * SS)][slot % (SS * SS)] = key / 2;
        }
        // The hit count is only known after tracing: drop the doubling slack (TEST-R7).
        self.hits.shrink_to_fit();
        self.flux.resize(lattice, 0.0);
        self.star_light.resize(lattice, 0.0);
        self.map.rebuild(lattice, px);
    }

    /// Advect the clump layers to the current time (BH-A4).
    fn advect(&mut self) {
        let layers = layers(self.t);
        let salt = self.clump_salt;
        let offset = |j: usize, generation: u64| {
            hash01(salt ^ cell_key(j as i32, generation as i32)) * NA as f64
        };
        let offsets = [offset(0, layers[0].2), offset(1, layers[1].2)];
        let base = &self.clump_base;
        for (ir, row) in self.tex.chunks_exact_mut(NA).enumerate() {
            let centre = ir as f64 + 0.5;
            // Clumps fade out towards the rim, so none shows past the smooth feather.
            let r = ISCO * (R_OUT / ISCO).powf(centre / NR as f64);
            let contrast =
                CONTRAST * (1.0 - ease::smoothstep((r - CLUMP_EDGE) / (R_OUT - CLUMP_EDGE)));
            let norm = -0.5 * contrast * contrast;
            // Per layer: birth row and azimuth shift, the same along the whole row.
            let from = [0, 1].map(|j| {
                let (r0, a0) = birth(centre, 0.0, layers[j].0);
                (r0, a0 - offsets[j])
            });
            for (ia, v) in row.iter_mut().enumerate() {
                let az = ia as f64 + 0.5;
                let n = layers[0].1 * bilerp(base, NA, az + from[0].1, from[0].0)
                    + layers[1].1 * bilerp(base, NA, az + from[1].1, from[1].0);
                *v = (contrast * n + norm).exp() as f32;
            }
        }
    }

    /// Sum the disk hits and look up the sky on the lattice, then fill the pixels.
    fn raster(&mut self) {
        self.flux.fill(0.0);
        {
            let flux = self.flux.as_mut_slice();
            for h in &self.hits {
                if let Some(f) = flux.get_mut(h.node as usize) {
                    *f += h.weight * self.tex[usize::from(h.texel)];
                }
            }
        }
        let spin = geom::wrap(SKY_SPIN * self.t, TAU) / TAU;
        let star_shift = STAR_W - ((spin * STAR_W as f64) as usize).min(STAR_W - 1);
        let stars = &self.stars;
        for (s, ray) in self.star_light.as_mut_slice().iter_mut().zip(&self.sky) {
            // The brightest star the node's supersamples carry (BH-A8).
            *s = ray
                .iter()
                .filter(|&&t| t != NO_STAR)
                .map(|&t| {
                    let (x, y) = (t as usize % STAR_W, t as usize / STAR_W);
                    f32::from(stars[y * STAR_W + (x + star_shift) % STAR_W])
                })
                .fold(0.0, f32::max);
        }
        let (map, levels) = (&self.map, &self.levels);
        let (flux, star) = (&self.flux, &self.star_light);
        // At the display sizes the lattice is not capped, the map is the identity.
        let direct = flux.size() == self.pixels.pixel_size();
        let width = usize::from(flux.width());
        self.pixels.shade(|x, y| {
            let at = |g: &Grid<f32>| {
                if direct {
                    g.as_slice()[y as usize * width + x as usize]
                } else {
                    map.sample(g, x, y)
                }
            };
            let level = levels.level(at(flux));
            if level > 0 {
                return Ink::new(Role::Blackbody, level);
            }
            let s = f64::from(at(star));
            if s >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Star, ink::level(s))
            } else {
                Ink::BLANK
            }
        });
    }
}

/// The shown temperature of a stretched display temperature `t ≥ T_SHOW` (stylised,
/// SPEC-002): from `T_SHOW` to `T_KNEE` mapped onto the coolest blackbody level
/// (`T_MIN`) … `T_KNEE`, lifted in log T as `x^TOE`, so the dim disk glows ember instead
/// of brown while its feathered edge still fades from the coolest level; above `T_KNEE`
/// rolled off in log T as `x / (1 + x)`, so it approaches `T_WHITE` without clipping.
fn display_temperature(t: f64) -> f64 {
    const T_MIN: f64 = ink::BLACKBODY_MIN_K;
    if t <= T_KNEE {
        let x = ((t / T_SHOW).ln() / (T_KNEE / T_SHOW).ln()).max(0.0);
        return T_MIN * ((T_KNEE / T_MIN).ln() * x.powf(TOE)).exp();
    }
    let span = (T_WHITE / T_KNEE).ln();
    let x = (t / T_KNEE).ln() / span;
    T_KNEE * (span * x / (1.0 + x)).exp()
}

/// Blackbody levels of the summed disk flux `f` (in units of the peak's): the colour of
/// the display temperature `T_PEAK · f^DISPLAY_EXP`, tabulated in `√f` so a lookup costs
/// one `sqrt`; level 0 means "too cold to show" (below `T_SHOW`).
#[derive(Debug)]
struct FluxLevels {
    lut: [u8; FluxLevels::LUT],
}

impl FluxLevels {
    const LUT: usize = 4096;
    /// The brightest flux tabulated: `g ≈ 1.6` at the peak radius, with a dense clump;
    /// the roll-off keeps the colour changing all the way up.
    const MAX: f32 = 16.0;

    fn new() -> Self {
        let last = (Self::LUT - 1) as f64;
        Self {
            lut: std::array::from_fn(|i| {
                let f = f64::from(Self::MAX) * (i as f64 / last).powi(2);
                let t = T_PEAK * f.powf(DISPLAY_EXP);
                if t < T_SHOW {
                    0
                } else {
                    ink::blackbody(display_temperature(t)).level.max(1)
                }
            }),
        }
    }

    #[inline]
    fn level(&self, f: f32) -> u8 {
        // NaN or negative ⇒ entry 0 through the saturating cast.
        let pos = (f / Self::MAX).sqrt() * (Self::LUT - 1) as f32;
        self.lut[(pos.round() as usize).min(Self::LUT - 1)]
    }
}

/// ASCII fallback shared by the half-block sky scenes: the brighter half's level on the
/// ramp `.:-=+*#%@`, in that half's ink (GEN-R10).
pub(crate) fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    const RAMP: [char; 9] = ['.', ':', '-', '=', '+', '*', '#', '%', '@'];
    let ink = [top, bottom]
        .into_iter()
        .filter(|p| p.role != Role::Background)
        .max_by_key(|p| p.level)?;
    Some((RAMP[usize::from(ink.level) * RAMP.len() / 256], ink))
}

impl Generator for BlackHole {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.geo.is_none() {
            self.seed(ctx);
        }
        self.pixels.resize(size);
        self.trace();
        self.advect();
        self.raster();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.advect();
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn geo() -> &'static Geodesics {
        static GEO: OnceLock<Geodesics> = OnceLock::new();
        GEO.get_or_init(Geodesics::new)
    }

    fn scene(size: Size) -> (BlackHole, Ctx) {
        let mut ctx = Ctx::new(5, size);
        let mut g = BlackHole::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn bh_a1_omega_decreases_with_radius_and_isco_period_is_2_5_s() {
        let horizon = 2.0;
        let mut last = f64::INFINITY;
        for k in 0..=200 {
            let r = horizon + f64::from(k) * (R_OUT - horizon) / 200.0;
            let w = omega(r);
            assert!(w < last && w > 0.0, "r={r} ω={w}");
            last = w;
        }
        assert!((TAU / omega(ISCO) - ISCO_PERIOD).abs() < 1e-12);
        for r in [horizon, ISCO, 10.0, R_OUT] {
            assert!((omega(r).powi(2) * r.powi(3) - gm()).abs() < 1e-9 * gm());
        }
    }

    #[test]
    fn bh_a5_geodesics_capture_below_the_critical_impact_parameter_and_deflect_weakly_far_out() {
        let g = geo();
        for k in 0..400 {
            let b = 0.05 + f64::from(k) * 0.03;
            let end = g.end(b);
            // Within the table's resolution around b_c, where rays circle for > 2π.
            if (b - B_CRIT).abs() < 0.02 {
                continue;
            }
            let captured = g.u(b, end.min(TAU) - 1e-9) > 0.25;
            assert_eq!(captured, b < B_CRIT, "b={b} end={end}");
        }
        for b in [20.0, 30.0, 40.0, 47.0, 60.0] {
            let alpha = g.end(b) - PI;
            // The weak-field series to third order in M/b.
            let expect = 4.0 / b + 15.0 * PI / (4.0 * b * b) + 128.0 / (3.0 * b * b * b);
            assert!(
                (alpha - expect).abs() < 0.01 * expect,
                "b={b}: {alpha} vs {expect}"
            );
        }
        // Early on, every ray is still the straight line u = sin φ / b to first order.
        for b in [8.0, 15.0, 40.0] {
            let phi = 0.3;
            let u = g.u(b, phi);
            assert!((u - phi.sin() / b).abs() < 2.0 / (b * b), "b={b}");
        }
    }

    #[test]
    fn bh_a2_the_shadow_is_black_and_the_far_side_stays_outside_it() {
        let g = geo();
        for k in 0..60 {
            for m in 0..72 {
                let b = B_CRIT * (0.02 + 0.97 * f64::from(k) / 60.0);
                let psi = TAU * f64::from(m) / 72.0;
                match g.trace(b, psi) {
                    Fate::Sky { .. } => panic!("sky inside the shadow at b={b} ψ={psi}"),
                    Fate::Disk { phi, order, .. } => {
                        assert_eq!(order, 0, "b={b} ψ={psi}");
                        // Only the near half of the disk passes in front of the shadow.
                        assert!(
                            phi > PI,
                            "far side inside the shadow: b={b} ψ={psi} φ={phi}"
                        );
                    }
                    Fate::Dark => {}
                }
            }
        }
        // And the far half's direct image is seen, above the shadow.
        let top = g.trace(B_CRIT + 3.0, PI / 2.0);
        assert!(
            matches!(top, Fate::Disk { phi, order: 0, .. } if phi < PI),
            "{top:?}"
        );
    }

    #[test]
    fn bh_a6_the_order_one_image_hugs_the_shadow_as_a_thin_ring_above_it() {
        let g = geo();
        let (mut above, mut below) = (0, 0);
        for k in 0..2000 {
            let b = 0.5 + f64::from(k) * 0.02;
            for m in 0..36 {
                let psi = TAU * f64::from(m) / 36.0;
                if let Fate::Disk { order: 1, .. } = g.trace(b, psi) {
                    assert!(b > B_CRIT, "order-1 image inside the shadow: b={b} ψ={psi}");
                    // Above the hole it is the thin lensing ring; below, the lower arc
                    // (the far side's underside, bent under the hole).
                    if psi.sin() >= 0.0 {
                        above += 1;
                        assert!(b < B_CRIT + 0.8, "b={b} ψ={psi}");
                    } else {
                        below += 1;
                    }
                }
            }
        }
        assert!(
            above > 18 && below > above,
            "ring {above}, lower arc {below}"
        );
        // On screen the ring reads as a line: every pixel it passes through above the
        // shadow is lit, at the sizes where it is thinner than a pixel.
        for size in [Size::new(120, 40), Size::new(200, 50)] {
            let (bh, _) = scene(size);
            let (scale, cx, cy) = bh.fit();
            for m in 1..40 {
                let psi = PI * (0.2 + 0.6 * f64::from(m) / 40.0);
                // The middle of the ring's (thin) band along this direction.
                let on_ring = |k: &i32| {
                    let b = B_CRIT + 0.001 * f64::from(*k);
                    matches!(g.trace(b, psi), Fate::Disk { order: 1, .. })
                };
                let Some(lo) = (0..800).find(on_ring) else {
                    continue;
                };
                let hi = (lo..800).take_while(on_ring).last().unwrap_or(lo);
                let b = B_CRIT + 0.0005 * f64::from(lo + hi);
                // Only where the gas the ring images is hot enough to show at all.
                let Fate::Disk { r, .. } = g.trace(b, psi) else {
                    continue;
                };
                let f = (redshift(r, b, psi) * temperature(r) / T_PEAK).powi(4) * feather(r);
                if T_PEAK * f.powf(DISPLAY_EXP) < 2.0 * T_SHOW {
                    continue;
                }
                let (x, y) = (cx + b * scale * psi.cos(), cy - b * scale * psi.sin());
                // Either pixel when the ring runs along a pixel edge.
                let lit = [y - 0.3, y + 0.3]
                    .into_iter()
                    .any(|y| bh.pixels.get(x as i32, y as i32).unwrap().role == Role::Blackbody);
                assert!(lit, "{size:?}: gap at ψ={psi}");
            }
        }
    }

    #[test]
    fn bh_a3_approaching_half_is_bluer_and_brighter() {
        for r in [ISCO + 0.1, R_PEAK, 12.0, R_OUT] {
            for b in [0.5 * r, r] {
                assert!(redshift(r, b, PI) > redshift(r, b, 0.0), "r={r} b={b}");
            }
        }
        let size = Size::new(200, 50);
        let (g, _) = scene(size);
        let px = g.pixels.pixel_size();
        let (mut left, mut right) = ((0u64, 0u64), (0u64, 0u64));
        for y in 0..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let p = g.pixels.get(x, y).unwrap();
                if p.role == Role::Blackbody {
                    let side = if x < i32::from(px.width()) / 2 {
                        &mut left
                    } else {
                        &mut right
                    };
                    side.0 += u64::from(p.level);
                    side.1 += 1;
                }
            }
        }
        let mean = |(s, n): (u64, u64)| s as f64 / n.max(1) as f64;
        assert!(
            mean(left) > mean(right) + 10.0,
            "left {left:?} right {right:?}"
        );
    }

    #[test]
    fn bh_a7_pole_line_has_no_line_of_sight_doppler_and_temperature_peaks_at_49_36_isco() {
        // Along cos ψ = 0 only gravitational redshift × transverse Doppler remains.
        for r in [ISCO, 9.0, R_OUT] {
            for b in [3.0, 10.0] {
                let g = redshift(r, b, PI / 2.0);
                assert!((g - (1.0 - 3.0 / r).sqrt()).abs() < 1e-12);
            }
        }
        assert!((temperature(R_PEAK) - T_PEAK).abs() < 1e-9);
        for r in [ISCO + 0.01, 7.0, 8.0, 8.3, 9.0, 15.0, R_OUT] {
            assert!(temperature(r) < T_PEAK, "r={r}");
        }
        assert_eq!(temperature(ISCO), 0.0);
    }

    #[test]
    fn bh_a4_clumps_ride_the_kepler_flow_with_steady_contrast() {
        // A parcel drifting in and orbiting (integrated numerically) traces back to the
        // birth position `birth` gives, for any age.
        for (row0, az0) in [(10.0, 30.0), (40.0, 200.0), (25.0, 3.0)] {
            let mut ln_r = (ISCO * (R_OUT / ISCO).powf(row0 / NR as f64)).ln();
            let mut az = az0;
            let h = 1e-3;
            for step in 1..=5000 {
                // Midpoint rule for dφ/dt = ω(r(t)), r(t) = r₀ e^{−DRIFT t}.
                az += omega((ln_r - 0.5 * DRIFT * h).exp()) * h / TAU * NA as f64;
                ln_r -= DRIFT * h;
                if step % 1000 == 0 {
                    let age = f64::from(step) * h;
                    let row = radial_coord(ln_r.exp());
                    let (r_back, a_back) = birth(row, az, age);
                    assert!((r_back - row0).abs() < 1e-6, "age {age}: row {r_back}");
                    assert!((a_back - az0).abs() < 1e-4, "age {age}: az {a_back}");
                }
            }
        }
        // The two layers' weights keep the log-normal contrast steady.
        for k in 0..500 {
            let [a, b] = layers(0.037 * f64::from(k));
            assert!((a.1 * a.1 + b.1 * b.1 - 1.0).abs() < 1e-12);
            assert!((0.0..LIFE).contains(&a.0) && (0.0..LIFE).contains(&b.0));
        }
        // The clump multiplier averages 1 and varies visibly.
        let (mut g, mut ctx) = scene(Size::new(80, 24));
        for _ in 0..100 {
            g.update(SIM_DT, &mut ctx);
        }
        let n = g.tex.len() as f64;
        let mean = g.tex.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
        let lo = g.tex.iter().filter(|&&v| v < 0.6).count() as f64 / n;
        let hi = g.tex.iter().filter(|&&v| v > 1.6).count() as f64 / n;
        assert!((mean - 1.0).abs() < 0.15, "mean {mean}");
        assert!(lo > 0.05 && hi > 0.05, "dark {lo}, bright {hi}");
    }

    #[test]
    fn bh_a8_the_sky_is_a_quiet_field_of_steady_point_stars() {
        let size = Size::new(120, 40);
        let (mut g, _) = scene(size);
        let px = g.pixels.pixel_size();
        let area = px.area();
        let turn = TAU / SKY_SPIN;
        for k in 0..24 {
            g.t = turn * f64::from(k) / 24.0;
            g.advect();
            g.raster();
            let lit = (0..i32::from(px.height()))
                .flat_map(|y| (0..i32::from(px.width())).map(move |x| (x, y)))
                .filter(|&(x, y)| g.pixels.get(x, y).unwrap().role == Role::Star)
                .count();
            assert!(
                20 * lit <= area,
                "t={}: stars cover {lit} of {area} px",
                g.t
            );
        }
        for size in [Size::new(80, 24), Size::new(120, 40), Size::new(200, 50)] {
            // Points: however strongly lensed, a star shows in at most two nodes, one per
            // image (direct, and lensed on the far side of the hole).
            let (mut g, mut ctx) = scene(size);
            let mut carried: Vec<u32> = g
                .sky
                .iter()
                .flatten()
                .copied()
                .filter(|&t| t != NO_STAR)
                .collect();
            carried.sort_unstable();
            let worst = carried.chunk_by(|a, b| a == b).map(<[u32]>::len).max();
            assert!(
                worst.unwrap_or(0) <= 2,
                "{size:?}: a star shows in {worst:?} nodes"
            );
            // Steady: as the sky turns, a star image lasts until it moves on (lensing
            // stretches a one-texel step up to two pixels) or behind the disk, rather than
            // blinking out between texels. Mean life of an image that vanishes with no star
            // within two pixels: ≥ 0.55 s (0.41–0.47 s with the round-2 footprint rule).
            let px = g.pixels.pixel_size();
            let star = |g: &BlackHole, x: i32, y: i32| {
                g.pixels.get(x, y).is_some_and(|p| p.role == Role::Star)
            };
            let (mut shown, mut vanished) = (0u32, 0u32);
            let mut before: Vec<(i32, i32)> = Vec::new();
            for _ in 0..(20.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                for &(x, y) in &before {
                    let near = (-2..=2).any(|dy| (-2..=2).any(|dx| star(&g, x + dx, y + dy)));
                    vanished += u32::from(!near);
                }
                before = (0..i32::from(px.height()))
                    .flat_map(|y| (0..i32::from(px.width())).map(move |x| (x, y)))
                    .filter(|&(x, y)| star(&g, x, y))
                    .collect();
                shown += before.len() as u32;
            }
            let life = f64::from(shown) * SIM_DT / f64::from(vanished.max(1));
            assert!(life >= 0.55, "{size:?}: star images last {life:.2} s");
        }
    }

    #[test]
    fn bh_a9_the_disk_is_the_hero_and_never_clips_to_a_flat_white() {
        // The mapping starts at the coolest level, rises strictly and never reaches
        // T_WHITE; it is continuous at the knee and lifts the dim disk (2 000 K shows as
        // ~2 700 K, towards ember rather than brown).
        assert!((display_temperature(T_SHOW) - ink::BLACKBODY_MIN_K).abs() < 1e-9);
        let mut last = 0.0;
        for k in 0..4000 {
            let t = T_SHOW + 10.0 * f64::from(k);
            let d = display_temperature(t);
            assert!(d > last && d < T_WHITE, "t={t}: {d}");
            last = d;
        }
        let (lo, hi) = (
            display_temperature(T_KNEE - 1e-6),
            display_temperature(T_KNEE + 1e-6),
        );
        assert!((hi - lo).abs() < 0.1 && (lo - T_KNEE).abs() < 0.1);
        assert!(display_temperature(2000.0) > 2600.0);
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let (mut g, mut ctx) = scene(Size::new(w, h));
            for _ in 0..120 {
                g.update(SIM_DT, &mut ctx);
            }
            let px = g.pixels.pixel_size();
            let inks: Vec<Ink> = (0..i32::from(px.height()))
                .flat_map(|y| (0..i32::from(px.width())).map(move |x| (x, y)))
                .map(|(x, y)| g.pixels.get(x, y).unwrap())
                .collect();
            let disk: Vec<u8> = inks
                .iter()
                .filter(|p| p.role == Role::Blackbody)
                .map(|p| p.level)
                .collect();
            let hottest = disk.iter().copied().max().unwrap_or(0);
            // Near-white: within 3 % of T_WHITE.
            let white = ink::blackbody(0.97 * T_WHITE).level;
            let flat = disk.iter().filter(|&&l| l >= white).count();
            assert!(
                20 * flat < disk.len(),
                "{w}×{h}: {flat} of {} disk px near white",
                disk.len()
            );
            // Brightest in every theme: mono greys follow the level.
            let star = inks
                .iter()
                .filter(|p| p.role == Role::Star)
                .map(|p| p.level)
                .max();
            assert!(
                star.unwrap_or(0) < hottest,
                "{w}×{h}: star {star:?} vs disk {hottest}"
            );
        }
    }

    #[test]
    fn bh_gen_r13_the_disk_clears_the_top_row_and_the_title_row() {
        for (w, h) in [(80, 24), (120, 40), (200, 50), (60, 40), (240, 30)] {
            let (g, _) = scene(Size::new(w, h));
            let px = g.pixels.pixel_size();
            let rows = [0, 1, i32::from(px.height()) - 2, i32::from(px.height()) - 1];
            for y in rows {
                for x in 0..i32::from(px.width()) {
                    let p = g.pixels.get(x, y).unwrap();
                    assert_ne!(p.role, Role::Blackbody, "{w}×{h}: disk at pixel {x},{y}");
                }
            }
        }
    }
}
