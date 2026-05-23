//! Kelvin–Helmholtz billows: two layers of fluid sliding past each other curl up into
//! cat's-eye vortices, which pair and merge (SPEC-002 `kelvin_helmholtz`).
//!
//! ## Model
//!
//! The core D2Q9 lattice Boltzmann kernel (`sciart_core::lbm`, CORE-R98/R99, the one
//! vortex_street runs) on a capped lattice, periodic in both axes. Periodic in y means
//! the classic *double* shear layer: a band moving right at `+U` between two bands
//! moving left at `−U`,
//!
//! ```text
//! u(y) = U [tanh((y − H/4)/δ) − tanh((y − 3H/4)/δ) − 1]
//! ```
//!
//! so there are no walls (a no-slip wall would shed its own vorticity) and total mass and
//! momentum are conserved exactly by the kernel. Each interface is seeded with a small
//! transverse velocity at the most unstable wavelength of a tanh layer, `λ ≈ 7δ`
//! (Michalke 1964), and at its subharmonic `2λ`, which drives the pairing (Ho & Huerre
//! 1984); the phases come from `ctx.rng`, so every run rolls up differently. The two
//! interfaces carry vorticity of opposite sign, so the upper row of billows spins one way
//! and the lower row the other.
//!
//! Lattice units: `U = 0.1` (Ma ≈ 0.17), `τ = 0.508` (ν ≈ 0.0027), `δ = max(H/28, 2.5)`,
//! so the layer Reynolds number `Uδ/ν` is ≈ 95–200 with the aspect bucket (121 at
//! 200×50): laminar enough to read, high enough to roll up cleanly. Four lattice steps
//! per update (240 per simulated second) give billows within ≈ 3 s and pairing by
//! ≈ 9 s. Small displays get a smaller lattice (never more nodes than pixels), so they
//! show fewer, larger billows instead of minified ones.
//!
//! The act lasts [`ACT_S`]; then (or when the watchdog finds a non-finite or out-of-band
//! node) the layers are laid down afresh with new phases.
//!
//! ## Picture
//!
//! A passive dye marks the middle band (1) against the outer bands (0); it rides the
//! lattice velocity by a semi-Lagrangian step each update ([`advect_dye`]). Half-block
//! pixels pick their ramp by the dye's side of ½, dusk-rose cloud (`Ramp(Eosin)`) for
//! the middle band over indigo sky (`Ramp(Cobalt)`) for the outer bands, and their level
//! by the vorticity `ω` (calm fluid dim, the billows bright). Toward the interface,
//! where the dye is ½, both ramps fall smoothly to near-black, so each rolled-up sheet
//! reads as a soft dark spiral line through bright, spinning fluid (bilinear, no
//! stair-stepped cut).

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::lbm::{self, Lattice};
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{draw, grid, ink};

pub static META: Meta = Meta {
    name: "kelvin_helmholtz",
    category: Category::Physics,
    description: "Kelvin-Helmholtz instability: two sliding layers roll up into billows that pair",
    title: "Shear billows · layers roll up",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(KelvinHelmholtz::default())
}

/// Lattice node budget and aspect bucket (ADR-0012): every display in one bucket runs
/// identical physics, and the cost does not grow with the terminal.
const MAX_NODES: usize = 16_384;
/// Floor of the node budget on tiny displays, so the flow still has two billows.
const MIN_NODES: usize = 1_200;
const BUCKET: f64 = 1.25;
/// Square-pixel aspects the lattice follows; beyond them it keeps the end value.
const ASPECT: (f64, f64) = (0.75, 4.0);
/// Shear speed and relaxation time (lattice units), TRT magic parameter.
const U: f64 = 0.1;
const TAU: f64 = 0.508;
const MAGIC: f64 = 0.25;
/// Layer half-thickness as a fraction of the lattice height, and its floor in nodes.
const DELTA_H: f64 = 1.0 / 28.0;
const DELTA_MIN: f64 = 2.5;
/// Most unstable wavelength of a tanh layer, in units of δ.
const LAMBDA_DELTA: f64 = 12.0;
/// Seed amplitudes (units of U) of the fundamental and its subharmonic, and the
/// Gaussian width of the seed about each interface (units of δ).
const SEED_FUND: f64 = 0.05;
const SEED_SUB: f64 = 0.006;
const SEED_WIDTH: f64 = 2.0;
/// Lattice steps per update; with 60 updates per second, 240 steps per second.
const STEPS_PER_UPDATE: usize = 4;
/// Updates run inside the first `resize` of a lattice (one simulated second).
const PREROLL_UPDATES: usize = 60;
/// Length of one act (simulated seconds) and the watchdog cadence and bands.
const ACT_S: f64 = 36.0;
const CHECK_EVERY: f64 = 1.0;
const RHO_OK: (f32, f32) = (0.8, 1.2);
/// Share of the laid-down shear the row-mean flow must keep for the act to go on.
const LAYERED_MIN: f64 = 0.25;
const U_MAX: f32 = 0.3;
/// Vorticity that saturates the colour, in units of the seed layer's peak U/δ, and the
/// fraction of it below which a pixel stays dark.
const VORT_SAT: f64 = 0.7;
const VORT_FLOOR: f64 = 0.01;
/// Brightness of calm, unspun fluid, as a share of full.
const CALM: f64 = 0.1;
/// Nodes per pixel above which the picture is box-filtered before sampling.
const MINIFY: f64 = 1.3;
/// Interface sharpening of the dye per update.
const SHARPEN: f32 = 0.015;
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// The middle band's ramp (dusk-rose cloud) and the outer bands' (indigo sky).
const CLOUD: Ramp = Ramp::Eosin;
const SKY: Ramp = Ramp::Cobalt;
/// ASCII fallback by strength.
const ASCII: [char; 6] = ['.', ':', '-', '=', '#', '@'];

/// Size-dependent flow parameters, pure in the lattice size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Flow {
    pub w: usize,
    pub h: usize,
    pub delta: f64,
    /// Seeded wavenumber: `n` wavelengths across the periodic width (even, so the
    /// subharmonic `n/2` fits too).
    pub n: usize,
}

/// The simulation lattice for a display: the display's square-pixel aspect `w/2h`,
/// clamped to [`ASPECT`] and snapped to its power of [`BUCKET`] (ADR-0012).
pub(crate) fn lattice_size(display: Size) -> Size {
    let (w, h) = (f64::from(display.width()), f64::from(display.height()));
    let aspect = (w / (2.0 * h)).clamp(ASPECT.0, ASPECT.1);
    let snapped = BUCKET.powi((aspect.ln() / BUCKET.ln()).round() as i32);
    // Never more nodes than half-block pixels: a minified lattice would alias its thin
    // sheets, so a small display simulates a smaller block (fewer, larger billows).
    let pixels = usize::from(display.width()) * 2 * usize::from(display.height());
    grid::lattice_for_aspect(snapped, MAX_NODES.min(pixels.max(MIN_NODES)))
}

pub(crate) fn flow_for(lattice: Size) -> Flow {
    let (w, h) = (usize::from(lattice.width()), usize::from(lattice.height()));
    let delta = (DELTA_H * h as f64).max(DELTA_MIN);
    let waves = w as f64 / (LAMBDA_DELTA * delta);
    let n = (2.0 * (waves / 2.0).round()).max(2.0) as usize;
    Flow { w, h, delta, n }
}

/// Base shear profile `u(y)` of the double layer (lattice rows, y down).
pub(crate) fn shear(f: &Flow, y: f64) -> f64 {
    let h = f.h as f64;
    let t = |c: f64| ((y - c) / f.delta).tanh();
    U * (t(0.25 * h) - t(0.75 * h) - 1.0)
}

/// Lay down the double shear layer with the seeded ripples; ρ from the Bernoulli-free
/// uniform pressure (the layers are parallel, so it is uniform).
pub(crate) fn seed(lat: &mut Lattice, f: &Flow, phases: [f64; 4]) {
    let (w, h) = (f.w, f.h);
    let k = std::f64::consts::TAU / w as f64;
    let (kf, ks) = (k * f.n as f64, k * (f.n / 2) as f64);
    let interfaces = [0.25 * h as f64, 0.75 * h as f64];
    let (rho, ux, uy) = lat.moments_mut();
    rho.fill(1.0);
    for y in 0..h {
        let yf = y as f64;
        let u = shear(f, yf) as f32;
        for x in 0..w {
            let xf = x as f64;
            let mut v = 0.0;
            for (j, &c) in interfaces.iter().enumerate() {
                let d = (yf - c) / (SEED_WIDTH * f.delta);
                let env = (-d * d).exp();
                let fund = SEED_FUND * (kf * xf + phases[2 * j]).sin_det();
                let sub = SEED_SUB * (ks * xf + phases[2 * j + 1]).sin_det();
                v += U * env * (fund + sub);
            }
            let i = y * w + x;
            ux.as_mut_slice()[i] = u;
            uy.as_mut_slice()[i] = v as f32;
        }
    }
    lat.equilibrate();
}

/// `ω = ∂v/∂x − ∂u/∂y` by periodic central differences (rows down, so the sign is the
/// on-screen sense of rotation, anticlockwise positive).
pub(crate) fn vorticity(lat: &Lattice, out: &mut Grid<f32>) {
    let (w, h) = (lat.width(), lat.height());
    if out.size() != lat.size() {
        out.resize(lat.size(), 0.0);
    }
    let (ux, uy) = (lat.ux().as_slice(), lat.uy().as_slice());
    let o = out.as_mut_slice();
    for y in 0..h {
        let (yn, ys) = ((y + h - 1) % h, (y + 1) % h);
        for x in 0..w {
            let (xw, xe) = ((x + w - 1) % w, (x + 1) % w);
            let dvdx = uy[y * w + xe] - uy[y * w + xw];
            // Rows grow downward: ∂u/∂y (y up) = −(u[row+1] − u[row−1]) / 2.
            let dudy = -(ux[ys * w + x] - ux[yn * w + x]);
            // −dvdx − dudy: y down flips the sense of x-y rotation on screen.
            o[y * w + x] = -0.5 * (dvdx + dudy);
        }
    }
}

/// ASCII cell for two Div pixels: the stronger by `|level − 128|`, as a strength char.
pub(crate) fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let p = sciart_core::raster::stronger(top, bottom);
    if p.role != Role::Ramp(CLOUD) && p.role != Role::Ramp(SKY) {
        return None;
    }
    let m = ink::seq_value(p);
    let i = ((m * ASCII.len() as f64) as usize).min(ASCII.len() - 1);
    Some((ASCII[i], p))
}

/// The dye of the laid-down layers: the middle band's share of each row, a smooth step
/// of width δ about each interface (the same tanh as the shear).
pub(crate) fn seed_dye(dye: &mut Grid<f32>, f: &Flow) {
    let size = Size::new(f.w as u16, f.h as u16);
    if dye.size() != size {
        dye.resize(size, 0.0);
    }
    for (y, row) in dye.as_mut_slice().chunks_exact_mut(f.w.max(1)).enumerate() {
        let c = 0.5 * (1.0 + shear(f, y as f64) / U);
        row.fill(c as f32);
    }
}

/// One semi-Lagrangian step of the dye over `n` lattice steps: each node takes the dye
/// at its departure point `x − n·u` (clamped Catmull–Rom, periodic), then a weak sharpening
/// `c += SHARPEN·c(1 − c)(2c − 1)` undoes the interpolation's smear, so the rolled-up
/// sheets stay thin (a stylisation: real dye diffuses, slowly).
pub(crate) fn advect_dye(lat: &Lattice, dye: &Grid<f32>, out: &mut Grid<f32>, n: f32) {
    let (w, h) = (lat.width(), lat.height());
    if dye.size() != lat.size() || w * h == 0 {
        return;
    }
    if out.size() != dye.size() {
        out.resize(dye.size(), 0.0);
    }
    let (ux, uy, c) = (lat.ux().as_slice(), lat.uy().as_slice(), dye.as_slice());
    let (wf, hf) = (w as f32, h as f32);
    for (i, o) in out.as_mut_slice().iter_mut().enumerate() {
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        let sx = (x - n * ux[i]).rem_euclid(wf);
        let sy = (y - n * uy[i]).rem_euclid(hf);
        let (x0, y0) = (sx.floor(), sy.floor());
        let (fx, fy) = (sx - x0, sy - y0);
        let (x0, y0) = (x0 as usize % w, y0 as usize % h);
        let col = |k: usize| (x0 + w + k - 1) % w;
        let xs = [col(0), col(1), col(2), col(3)];
        let (wx, wy) = (catmull_rom(fx), catmull_rom(fy));
        let (mut v, mut lo, mut hi) = (0.0, f32::MAX, f32::MIN);
        for (j, wyj) in wy.into_iter().enumerate() {
            let row = &c[((y0 + h + j - 1) % h) * w..][..w];
            let r =
                wx[0] * row[xs[0]] + wx[1] * row[xs[1]] + wx[2] * row[xs[2]] + wx[3] * row[xs[3]];
            v += wyj * r;
            if j == 1 || j == 2 {
                lo = lo.min(row[xs[1]]).min(row[xs[2]]);
                hi = hi.max(row[xs[1]]).max(row[xs[2]]);
            }
        }
        // Clamped to the enclosing cell's corners: monotone, so no new extrema ring.
        // A non-finite neighbourhood (a blown-up step before the watchdog's restart)
        // reads as the interface, never as NaN.
        let v = if lo <= hi && v.is_finite() {
            v.clamp(lo, hi)
        } else {
            0.5
        };
        *o = (v + SHARPEN * v * (1.0 - v) * (2.0 * v - 1.0)).clamp(0.0, 1.0);
    }
}

/// Periodic 3 × 3 box filter of `src` into `dst` (reshaped to it).
pub(crate) fn box3(src: &Grid<f32>, dst: &mut Grid<f32>) {
    let (w, h) = (usize::from(src.width()), usize::from(src.height()));
    if dst.size() != src.size() {
        dst.resize(src.size(), 0.0);
    }
    let (a, o) = (src.as_slice(), dst.as_mut_slice());
    for y in 0..h {
        let rows = [(y + h - 1) % h, y, (y + 1) % h];
        for x in 0..w {
            let cols = [(x + w - 1) % w, x, (x + 1) % w];
            let mut sum = 0.0;
            for r in rows {
                for c in cols {
                    sum += a[r * w + c];
                }
            }
            o[y * w + x] = sum / 9.0;
        }
    }
}

/// Catmull–Rom weights of the four taps around a fraction `t ∈ [0, 1)`.
#[inline]
fn catmull_rom(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// The signed picture value of dye `c` and vorticity `s` (units of the saturation
/// value): the band's side of ½ as the sign, dim in calm fluid and bright where it spins,
/// and zero on the interface itself, so each rolled-up sheet reads as a dark spiral line.
/// Non-finite input gives 0.
pub(crate) fn shade_value(c: f64, s: f64) -> f64 {
    let side = 2.0 * c - 1.0;
    if !(side.is_finite() && s.is_finite()) {
        return 0.0;
    }
    let spin = ink::soft_clip(2.0 * s.abs());
    (CALM + (1.0 - CALM) * spin) * side.abs().sqrt().copysign(side)
}

/// The Div ink of a picture value, blank below the floor.
pub(crate) fn value_ink(v: f64) -> Ink {
    if v.is_nan() || v.abs() < VORT_FLOOR {
        return Ink::BLANK;
    }
    let ramp = if v > 0.0 { CLOUD } else { SKY };
    ink::ramp(ramp, v.abs())
}

/// [`value_ink`] of [`shade_value`].
#[cfg(test)]
pub(crate) fn pixel_ink(c: f64, s: f64) -> Ink {
    value_ink(shade_value(c, s))
}

/// Vortex count along a row: local maxima of `|ω|` (3-point smoothed, periodic) above
/// `floor`, each separated from the last by a dip below half its height.
#[cfg(test)]
pub(crate) fn count_vortices(row: &[f32], floor: f32) -> usize {
    let n = row.len();
    if n < 3 {
        return 0;
    }
    let s = |i: usize| (row[(i + n - 1) % n].abs() + row[i].abs() + row[(i + 1) % n].abs()) / 3.0;
    let mut count = 0;
    for i in 0..n {
        let (a, b, c) = (s((i + n - 1) % n), s(i), s((i + 1) % n));
        if b >= floor && b > a && b >= c {
            count += 1;
        }
    }
    count
}

#[derive(Debug)]
pub struct KelvinHelmholtz {
    size: Size,
    flow: Option<Flow>,
    lat: Lattice,
    vort: Grid<f32>,
    /// Passive dye: 1 in the middle band, 0 outside, and its advection scratch.
    dye: Grid<f32>,
    dye_next: Grid<f32>,
    /// Signed picture value per node (see [`shade_value`]).
    shade: Grid<f32>,
    /// Whether a pixel spans more than [`MINIFY`] nodes, so the picture is prefiltered.
    minified: bool,
    pixels: Pixels,
    map: SampleMap,
    omegas: (f32, f32),
    /// Vorticity scale: the seed layer's peak `U/δ` times [`VORT_SAT`].
    scale: f64,
    /// Time into the act and since the last watchdog check (simulated seconds).
    t: f64,
    since_check: f64,
    resets: u32,
}

impl Default for KelvinHelmholtz {
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            flow: None,
            lat: Lattice::default(),
            vort: Grid::new(Size::new(1, 1), 0.0),
            dye: Grid::new(Size::new(1, 1), 0.0),
            dye_next: Grid::new(Size::new(1, 1), 0.0),
            shade: Grid::new(Size::new(1, 1), 0.0),
            minified: false,
            pixels: Pixels::default(),
            map: SampleMap::default(),
            omegas: (1.0, 1.0),
            scale: 1.0,
            t: 0.0,
            since_check: 0.0,
            resets: 0,
        }
    }
}

impl KelvinHelmholtz {
    fn restart(&mut self, rng: &mut impl Rng) {
        let Some(f) = self.flow else { return };
        let tau = std::f64::consts::TAU;
        let phases = [
            rng.random::<f64>() * tau,
            rng.random::<f64>() * tau,
            rng.random::<f64>() * tau,
            rng.random::<f64>() * tau,
        ];
        seed(&mut self.lat, &f, phases);
        seed_dye(&mut self.dye, &f);
        // The seed state's moments, so the first picture is the laid-down layers.
        self.t = 0.0;
        self.since_check = 0.0;
    }

    /// One update's worth of flow: fixed lattice steps (ADR-0003 fixes dt at 1/60 s; a
    /// long dt is a catch-up the runtime already splits), then the dye rides them.
    fn advance(&mut self) {
        for _ in 0..STEPS_PER_UPDATE {
            self.lat.step_periodic(self.omegas.0, self.omegas.1);
        }
        advect_dye(
            &self.lat,
            &self.dye,
            &mut self.dye_next,
            STEPS_PER_UPDATE as f32,
        );
        std::mem::swap(&mut self.dye, &mut self.dye_next);
    }

    fn healthy(&self) -> bool {
        let fast = |g: &Grid<f32>| g.as_slice().iter().all(|v| v.abs() < U_MAX);
        self.lat.healthy(RHO_OK) && fast(self.lat.ux()) && fast(self.lat.uy())
    }

    /// Whether the flow still remembers its layers: the row-mean `u` projected on the
    /// laid-down shear profile. On small periodic blocks the merged vortices can
    /// reorganise into a vertical jet; the act then ends (the turbulence-and-reset beat).
    fn layered(&self) -> bool {
        let Some(f) = self.flow else { return true };
        let ux = self.lat.ux().as_slice();
        let (mut dot, mut norm) = (0.0, 0.0);
        for (y, row) in ux.chunks_exact(f.w.max(1)).enumerate() {
            let mean = row.iter().map(|&u| f64::from(u)).sum::<f64>() / f.w as f64;
            let base = shear(&f, y as f64);
            dot += mean * base;
            norm += base * base;
        }
        norm == 0.0 || dot / norm >= LAYERED_MIN
    }

    fn rasterise(&mut self) {
        vorticity(&self.lat, &mut self.vort);
        // The signed picture value per node (one pass over the capped lattice), then one
        // bilinear sample per pixel: a sign flip interpolates through the dark zero, so
        // the interface stays a dark line at any zoom.
        let k = 1.0 / self.scale;
        for ((v, &c), &s) in self
            .shade
            .as_mut_slice()
            .iter_mut()
            .zip(self.dye.as_slice())
            .zip(self.vort.as_slice())
        {
            *v = shade_value(f64::from(c), f64::from(s) * k) as f32;
        }
        // On a small display a pixel spans more than a node: point samples of the thin
        // sheets would alias into speckle, so the picture is box-filtered first (into
        // the vorticity grid, which is done with).
        let src = if self.minified {
            box3(&self.shade, &mut self.vort);
            &self.vort
        } else {
            &self.shade
        };
        let map = &self.map;
        self.pixels
            .shade(|x, y| value_ink(f64::from(map.sample(src, x, y))));
    }

    /// `Re 121` on the title row, right-aligned, when there is room (GEN-R13).
    fn readout(&self, canvas: &mut Canvas) {
        let Some(f) = self.flow else { return };
        let nu = (TAU - 0.5) / 3.0;
        let re = (U * f.delta / nu).round() as u32;
        let mut text = StackText::<16>::new();
        let _ = match title::readout_room(self.size, &Title::of(self)) {
            6.. => write!(text, "Re {re}"),
            _ => return,
        };
        let y = i32::from(self.size.height()) - 1;
        draw::label_right(canvas, y, text.as_str(), READOUT_INK);
    }
}

impl Generator for KelvinHelmholtz {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        let lattice = lattice_size(size);
        if self.flow.is_none() || self.lat.size() != lattice {
            // ADR-0012: a new bucket gets its own lattice and a fresh act (the seeded
            // wavelengths depend on the lattice, so resampling the old flow would seed
            // the wrong modes).
            let f = flow_for(lattice);
            self.flow = Some(f);
            self.lat = Lattice::new(lattice, |_, _| false);
            self.shade.resize(lattice, 0.0);
            self.omegas = (
                (1.0 / TAU) as f32,
                (1.0 / lbm::tau_minus(TAU, MAGIC)) as f32,
            );
            self.scale = VORT_SAT * U / f.delta;
            self.restart(&mut ctx.rng);
            // A one-off pre-roll past the seed's relaxation, so a visit opens on growing
            // ripples rather than on still, flat layers (≈ 25 ms at the node cap).
            for _ in 0..PREROLL_UPDATES {
                self.advance();
            }
            self.t = PREROLL_UPDATES as f64 * SIM_DT;
        }
        self.pixels.resize(size);
        self.map.rebuild(lattice, self.pixels.pixel_size());
        let px = self.pixels.pixel_size();
        self.minified = f64::from(lattice.width()) > MINIFY * f64::from(px.width())
            || f64::from(lattice.height()) > MINIFY * f64::from(px.height());
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.flow.is_none() {
            return;
        }
        self.t += dt;
        self.since_check += dt;
        self.advance();
        if self.since_check >= CHECK_EVERY {
            self.since_check = 0.0;
            if !self.healthy() {
                self.resets += 1;
                self.restart(&mut ctx.rng);
            } else if !self.layered() {
                self.restart(&mut ctx.rng);
            }
        }
        if self.t >= ACT_S {
            self.restart(&mut ctx.rng);
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        self.readout(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(seed: u64, size: Size) -> (KelvinHelmholtz, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = KelvinHelmholtz::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// RMS transverse velocity: the amplitude of the interface ripples.
    fn ripple(g: &KelvinHelmholtz) -> f64 {
        let v = g.lat.uy().as_slice();
        (v.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / v.len() as f64).sqrt()
    }

    /// Vortices along the upper interface's row.
    fn vortices(g: &KelvinHelmholtz) -> usize {
        let f = g.flow.unwrap();
        let mut vort = Grid::new(g.lat.size(), 0.0);
        vorticity(&g.lat, &mut vort);
        // The billow cores sit on the interface row; average a band of ±δ about it.
        let y0 = (0.25 * f.h as f64).round() as usize;
        let band = f.delta.round() as usize;
        let mut row = vec![0.0f32; f.w];
        for y in y0 - band..=y0 + band {
            for (x, r) in row.iter_mut().enumerate() {
                *r += vort.as_slice()[y * f.w + x];
            }
        }
        count_vortices(&row, (0.3 * g.scale * (2 * band + 1) as f64) as f32)
    }

    #[test]
    fn kh_a1_scene_runs_on_the_core_lbm_kernel() {
        // The kernel itself is vortex_street's, pinned bit for bit by
        // vortex_street::tests::kh_a1_lifted_lbm_keeps_the_vortex_street_bit_identical;
        // here the scene's own step is exactly `step_periodic` on the core lattice.
        let (mut g, mut ctx) = scene(3, Size::new(80, 24));
        let mut twin = g.lat.clone();
        g.update(SIM_DT, &mut ctx);
        for _ in 0..STEPS_PER_UPDATE {
            twin.step_periodic(g.omegas.0, g.omegas.1);
        }
        assert_eq!(twin.populations(), g.lat.populations());
    }

    #[test]
    fn kh_a2_ripples_grow_exponentially_early() {
        let (mut g, mut ctx) = scene(11, Size::new(200, 50));
        let (mut ts, mut ls) = (Vec::new(), Vec::new());
        // Skip the first 0.5 s (the seed relaxes onto the eigenmode), fit 0.5–2.5 s.
        for frame in 0..150u32 {
            g.update(SIM_DT, &mut ctx);
            if frame >= 30 && frame % 5 == 0 {
                ts.push(f64::from(frame) * SIM_DT);
                ls.push(ripple(&g).ln());
            }
        }
        let n = ts.len() as f64;
        let (mt, ml) = (ts.iter().sum::<f64>() / n, ls.iter().sum::<f64>() / n);
        let sxy: f64 = ts.iter().zip(&ls).map(|(t, l)| (t - mt) * (l - ml)).sum();
        let sxx: f64 = ts.iter().map(|t| (t - mt).powi(2)).sum();
        let syy: f64 = ls.iter().map(|l| (l - ml).powi(2)).sum();
        let slope = sxy / sxx;
        let r2 = sxy * sxy / (sxx * syy);
        assert!(slope > 0.3, "growth rate {slope}/s");
        assert!(r2 > 0.9, "log-linear R² {r2}");
    }

    #[test]
    fn kh_a3_billows_pair_and_the_vortex_count_falls() {
        let (mut g, mut ctx) = scene(5, Size::new(200, 50));
        let n = g.flow.unwrap().n;
        let at = |g: &mut KelvinHelmholtz, ctx: &mut Ctx, s: f64| {
            while g.t < s {
                g.update(SIM_DT, ctx);
            }
            vortices(g)
        };
        let early = at(&mut g, &mut ctx, 4.0);
        let late = at(&mut g, &mut ctx, 15.0);
        assert!(early >= n - 1, "rolled up into {early} of {n} billows");
        assert!(late < early, "pairing: {early} → {late}");
        assert_eq!(g.resets, 0);
    }

    #[test]
    fn kh_a4_total_mass_is_conserved() {
        let (mut g, mut ctx) = scene(9, Size::new(120, 40));
        let m0 = g.lat.mass();
        for _ in 0..600 {
            g.update(SIM_DT, &mut ctx);
        }
        let rel = (g.lat.mass() - m0).abs() / m0;
        assert!(rel < 1e-6, "relative mass drift {rel}");
    }

    #[test]
    fn kh_a5_step_cost_is_capped_whatever_the_display() {
        // Worst frame (< 2 ms at 200×50) is measured by examples/worst_frame; the cost
        // it bounds is lattice nodes × steps, which is capped for every display.
        for (w, h) in [
            (1, 1),
            (80, 24),
            (200, 50),
            (500, 150),
            (1000, 20),
            (20, 200),
        ] {
            let l = lattice_size(Size::new(w, h));
            let nodes = usize::from(l.width()) * usize::from(l.height());
            assert!(nodes <= MAX_NODES && nodes * STEPS_PER_UPDATE <= 4 * 16_384);
            let f = flow_for(l);
            assert!(f.n >= 2 && f.n % 2 == 0 && f.delta >= DELTA_MIN);
        }
    }

    #[test]
    fn kh_resize_in_one_bucket_keeps_the_flow_and_reset_after_the_act() {
        let (mut g, mut ctx) = scene(2, Size::new(200, 50));
        for _ in 0..30 {
            g.update(SIM_DT, &mut ctx);
        }
        let before = g.lat.populations().to_vec();
        g.resize(Size::new(210, 52), &mut ctx);
        assert_eq!(g.lat.populations(), &before[..]);
        g.t = ACT_S - 0.5 * SIM_DT;
        g.update(SIM_DT, &mut ctx);
        assert!(g.t < SIM_DT && ripple(&g) < 0.05 * U);
    }

    #[test]
    fn kh_vorticity_signs_differ_between_the_two_rows() {
        let (g, _) = scene(4, Size::new(200, 50));
        let f = g.flow.unwrap();
        let at = |y: f64| g.vort.as_slice()[(y.round() as usize) * f.w];
        let (upper, lower) = (at(0.25 * f.h as f64), at(0.75 * f.h as f64));
        assert!(upper * lower < 0.0, "{upper} {lower}");
        // Calm fluid is dim, spinning fluid bright, the interface (c = ½) dark.
        let calm = pixel_ink(1.0, 0.0);
        assert_eq!(calm.role, Role::Ramp(CLOUD));
        assert!(ink::seq_value(calm) > 0.0 && ink::seq_value(calm) < 0.2);
        let (cloud, sky) = (pixel_ink(1.0, 1.0), pixel_ink(0.0, 1.0));
        assert!(cloud.role == Role::Ramp(CLOUD) && ink::seq_value(cloud) > 0.9);
        assert!(sky.role == Role::Ramp(SKY) && ink::seq_value(sky) > 0.9);
        assert_eq!(pixel_ink(0.5, 1.0), Ink::BLANK);
        assert_eq!(pixel_ink(f64::NAN, 0.0), Ink::BLANK);
    }
}
