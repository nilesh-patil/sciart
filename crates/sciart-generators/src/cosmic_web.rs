//! The cosmic web by the Zel'dovich approximation (SPEC-002 `cosmic_web`).
//!
//! A particle that starts at lattice point `q` sits at `x = q + D(t)·s(q)`, with the
//! displacement `s = −∇ψ` of a potential made of `MODES` plane waves that are periodic
//! in the box. `s` is tabulated per particle once per loop (separable sin/cos tables, no
//! FFT), so a frame is one bilinear deposit per particle. Lengths are in lattice cells,
//! where the lattice is the display's pixel grid capped at `MAX_NODES` (ADR-0012).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::grid::lattice_for;
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
use sciart_core::timeline::ease;
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "cosmic_web",
    category: Category::DeepCosmos,
    description: "A smooth fog of dark matter collapsing into the sheets, filaments and knots of the cosmic web",
    title: "Cosmic web · gravity gathers",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(CosmicWeb::default())
}

/// Plane-wave modes of the potential.
pub(crate) const MODES: usize = 48;
/// Particle (and density lattice) cap (ADR-0012).
const MAX_NODES: usize = 16_384;
/// Mode wavenumbers in box periods along the height: log-uniform in this range. The box
/// is taken as `BOX_ASPECT` heights wide, so modes are round on screen.
const N_RANGE: (f64, f64) = (1.6, 22.0);
const BOX_ASPECT: f64 = 2.0;
/// Loop: collapse, hold and relax (s).
const RISE: f64 = 20.0;
const HOLD: f64 = 5.0;
const RELAX: f64 = 5.0;
pub(crate) const LOOP: f64 = RISE + HOLD + RELAX;
/// First shell crossing at this fraction of `D_max` (SPEC-002).
const CROSSING_AT: f64 = 0.4;
/// A particle freezes this far past its own shell-crossing growth.
const FREEZE: f32 = 1.0;
/// Pan across the box, lattice cells per second.
const PAN: f64 = 0.35;
/// Growth still creeping on during the hold, from this fraction of `D_max`.
const HOLD_FROM: f64 = 0.94;
/// Scene start in loop time: growth ≈ 0.3.
const START: f64 = 2.3;
/// Tone curve over density in particles per node (mean 1), and the halo threshold.
const KNEE: f32 = 1.4;
const WHITE: f32 = 7.0;
const HALO: f32 = 3.2;
/// Density subtracted before toning, so the mean fog sits at the ramp's near-black foot
/// and the voids end in black on a smooth ramp (no dither).
const FLOOR: f32 = 0.82;
const GAMMA: f32 = 1.2;
/// Fog below this level is drawn in Cobalt blue, the web above it in Plasma violet.
const FOG_BLUE: u8 = 46;
/// Halo nodes: gold-white, hotter (whiter) the denser.
const HALO_K: (f64, f64) = (4_200.0, 7_500.0);

/// One potential mode: wavevector in box periods (integers, so it tiles the box), its
/// amplitude, phase at cycle 0 and phase step per cycle.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Mode {
    nx: i32,
    ny: i32,
    amp: f64,
    phase: f64,
    step: f64,
}

/// Growth factor over one loop in units of `D_max`: a smooth rise, a slow creep, a
/// smooth relax to 0 (where the next cycle's modes take over invisibly).
pub(crate) fn growth(t: f64) -> f64 {
    let t = t.rem_euclid(LOOP);
    if t < RISE {
        // Fast at first, slowing as the web sharpens: no long uniform fog.
        let x = (t / RISE).clamp(0.0, 1.0);
        HOLD_FROM * (1.0 - (1.0 - x).powi(2))
    } else if t < RISE + HOLD {
        HOLD_FROM + (1.0 - HOLD_FROM) * ease::smoothstep((t - RISE) / HOLD)
    } else {
        1.0 - ease::smoothstep((t - RISE - HOLD) / RELAX)
    }
}

/// Growth (un-normalised) at which a node whose smallest deformation eigenvalue is
/// `lam` crosses shells, `−1/λ`; ∞ if it never does.
fn freeze_at(lam: f64) -> f32 {
    if lam < 0.0 {
        (-1.0 / lam) as f32
    } else {
        f32::INFINITY
    }
}

/// Position of the particle from node `(qx, qy)` with displacement `s` at growth `k`
/// (un-normalised), frozen at its own shell-crossing growth `at` (WEB-A1):
/// `x = q + min(k, at)·s`.
pub(crate) fn displaced(qx: f32, qy: f32, s: (f32, f32), k: f32, at: f32) -> (f32, f32) {
    let k = k.min(at);
    (qx + k * s.0, qy + k * s.1)
}

/// Density bins of [`halo_threshold`]'s histogram and its range (particles per node).
const HIST_BINS: usize = 128;
const HIST_MAX: f32 = 32.0;

/// The density of the densest 1 % of nodes (the haloes), found from a fixed histogram so
/// it costs one pass and no allocation; never below `HALO`, so an uncollapsed fog has
/// no haloes.
fn halo_threshold(values: &[f32]) -> f32 {
    let mut hist = [0u32; HIST_BINS];
    for &v in values {
        let b = ((v / HIST_MAX) * HIST_BINS as f32) as usize;
        hist[b.min(HIST_BINS - 1)] += 1;
    }
    let mut above = 0usize;
    let target = values.len() / 100;
    for (b, &n) in hist.iter().enumerate().rev() {
        above += n as usize;
        if above > target {
            let v = (b + 1) as f32 * HIST_MAX / HIST_BINS as f32;
            return v.max(HALO);
        }
    }
    HALO
}

/// Separable tables for one cycle's modes over a `w × h` lattice: `cos`/`sin` of
/// `kx·x` per column and of `ky·y + φ` per row, per mode.
#[derive(Debug, Default)]
struct Tables {
    cx: Vec<f64>,
    sx: Vec<f64>,
    cy: Vec<f64>,
    sy: Vec<f64>,
}

/// Wavevector of `m` in radians per lattice cell on a `w × h` lattice.
fn wavevector(m: &Mode, w: usize, h: usize) -> (f64, f64) {
    (
        TAU * f64::from(m.nx) / w.max(1) as f64,
        TAU * f64::from(m.ny) / h.max(1) as f64,
    )
}

impl Tables {
    fn build(&mut self, modes: &[Mode], cycle: i64, w: usize, h: usize) {
        self.cx.clear();
        self.sx.clear();
        self.cy.clear();
        self.sy.clear();
        for m in modes {
            let (kx, ky) = wavevector(m, w, h);
            let phase = m.phase + m.step * cycle as f64;
            for i in 0..w {
                let (s, c) = (kx * i as f64).sin_cos_det();
                self.cx.push(c);
                self.sx.push(s);
            }
            for j in 0..h {
                let (s, c) = (ky * j as f64 + phase).sin_cos_det();
                self.cy.push(c);
                self.sy.push(s);
            }
        }
    }

    /// Displacement `s = −∇ψ` at node `(i, j)` and the smallest eigenvalue of its
    /// deformation tensor `∂s/∂q`.
    fn at(&self, modes: &[Mode], w: usize, h: usize, i: usize, j: usize) -> ((f64, f64), f64) {
        let (mut sx, mut sy) = (0.0, 0.0);
        let (mut dxx, mut dxy, mut dyy) = (0.0, 0.0, 0.0);
        for (k, m) in modes.iter().enumerate() {
            let (kx, ky) = wavevector(m, w, h);
            let (cx, snx) = (self.cx[k * w + i], self.sx[k * w + i]);
            let (cy, sny) = (self.cy[k * h + j], self.sy[k * h + j]);
            // θ = kx·x + ky·y + φ.
            let sin = snx * cy + cx * sny;
            let cos = cx * cy - snx * sny;
            // ψ = Σ A cos θ ⇒ s = −∇ψ = Σ A k sin θ, ∂s/∂q = Σ A k kᵀ cos θ.
            sx += m.amp * kx * sin;
            sy += m.amp * ky * sin;
            let a = m.amp * cos;
            dxx += a * kx * kx;
            dxy += a * kx * ky;
            dyy += a * ky * ky;
        }
        let mean = 0.5 * (dxx + dyy);
        let lam = mean - (0.25 * (dxx - dyy) * (dxx - dyy) + dxy * dxy).sqrt();
        ((sx, sy), lam)
    }
}

/// `s(q)` straight from the modes (no tables), for tests.
#[cfg(test)]
fn displacement_direct(
    modes: &[Mode],
    cycle: i64,
    w: usize,
    h: usize,
    q: (f64, f64),
) -> (f64, f64) {
    let (mut sx, mut sy) = (0.0, 0.0);
    for m in modes {
        let (kx, ky) = wavevector(m, w, h);
        let th = kx * q.0 + ky * q.1 + m.phase + m.step * cycle as f64;
        sx += m.amp * kx * th.sin_det();
        sy += m.amp * ky * th.sin_det();
    }
    (sx, sy)
}

#[derive(Debug, Default)]
pub struct CosmicWeb {
    seeded: bool,
    t: f64,
    modes: Vec<Mode>,
    /// Amplitude scale that puts the first shell crossing at `CROSSING_AT` of `D_max`,
    /// from cycle 0 (the statistics of every cycle match).
    norm: f64,
    tables: Tables,
    /// Displacements (lattice cells, un-normalised) of the current cycle, and of the
    /// next one, filled a chunk per step; `next_done` particles are ready.
    disp: Vec<(f32, f32, f32)>,
    next: Vec<(f32, f32, f32)>,
    next_done: usize,
    next_tables: Tables,
    cycle: i64,
    density: Grid<f32>,
    scratch: Grid<f32>,
    /// The density after one blur pass: the caustics' sharp version.
    sharp: Grid<f32>,
    map: SampleMap,
    tone: Tone,
    pixels: Pixels,
}

impl CosmicWeb {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        let (lo, hi) = (N_RANGE.0.ln(), N_RANGE.1.ln());
        self.modes = vec![Mode::default(); MODES];
        for m in &mut self.modes {
            loop {
                let n = ctx.rng.random_range(lo..hi).exp();
                let a = ctx.rng.random_range(0.0..TAU);
                let (s, c) = a.sin_cos_det();
                m.nx = (n * c * BOX_ASPECT).round() as i32;
                m.ny = (n * s).round() as i32;
                if m.nx != 0 || m.ny != 0 {
                    break;
                }
            }
            let k = (f64::from(m.nx) / BOX_ASPECT).hypot(f64::from(m.ny));
            // δ ∝ k⁻¹ (P ∝ k⁻²), ψ = δ / k², with a Rayleigh-distributed amplitude.
            let rayleigh = (-2.0 * ctx.rng.random_range(1e-6f64..1.0).ln()).sqrt();
            m.amp = rayleigh * k.powf(-2.7);
            m.phase = ctx.rng.random_range(0.0..TAU);
            m.step = ctx.rng.random_range(0.8..TAU - 0.8);
        }
        self.tone = Tone::asinh(KNEE, WHITE);
        // Start where D ≈ 0.3 D_max: a faint web already on screen.
        self.t = START;
    }

    fn dims(&self) -> (usize, usize) {
        (
            usize::from(self.density.width()),
            usize::from(self.density.height()),
        )
    }

    /// Fill `out` with every node's displacement for `cycle`, returning `min λ`.
    fn fill_all(&mut self, cycle: i64) -> f64 {
        let (w, h) = self.dims();
        self.tables.build(&self.modes, cycle, w, h);
        self.disp.clear();
        let mut lam_min = 0.0f64;
        for j in 0..h {
            for i in 0..w {
                let ((sx, sy), lam) = self.tables.at(&self.modes, w, h, i, j);
                lam_min = lam_min.min(lam);
                self.disp.push((sx as f32, sy as f32, freeze_at(lam)));
            }
        }
        lam_min
    }

    /// Advance the next cycle's table by `count` particles.
    fn fill_next(&mut self, count: usize) {
        let (w, h) = self.dims();
        let n = w * h;
        if self.next_done == 0 {
            self.next_tables.build(&self.modes, self.cycle + 1, w, h);
            self.next.clear();
        }
        let end = (self.next_done + count).min(n);
        for idx in self.next_done..end {
            let ((sx, sy), lam) = self.next_tables.at(&self.modes, w, h, idx % w, idx / w);
            self.next.push((sx as f32, sy as f32, freeze_at(lam)));
        }
        self.next_done = end;
    }

    /// Particle positions deposited into the density lattice for growth `d` (in `D_max`).
    fn deposit(&mut self, d: f64) {
        let (w, h) = self.dims();
        self.density.fill(0.0);
        let dmax = self.norm / CROSSING_AT;
        let k = (d * dmax) as f32;
        let freeze = FREEZE;
        // A slow pan across the periodic box keeps the held web moving.
        let pan = (PAN * self.t).rem_euclid(w as f64) as f32;
        let (wf, hf) = (w as f32, h as f32);
        let cells = self.density.as_mut_slice();
        for (idx, &(sx, sy, at)) in self.disp.iter().enumerate() {
            // A particle stops at its own shell crossing (adhesion-like): it stays in
            // the caustic instead of streaming through and smearing it out.
            let (x, y) = displaced((idx % w) as f32, (idx / w) as f32, (sx, sy), k, at * freeze);
            let x = x + pan;
            let (x, y) = (x.rem_euclid(wf), y.rem_euclid(hf));
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            let (i0, j0) = ((x0 as usize) % w, (y0 as usize) % h);
            let (i1, j1) = ((i0 + 1) % w, (j0 + 1) % h);
            cells[j0 * w + i0] += (1.0 - fx) * (1.0 - fy);
            cells[j0 * w + i1] += fx * (1.0 - fy);
            cells[j1 * w + i0] += (1.0 - fx) * fy;
            cells[j1 * w + i1] += fx * fy;
        }
    }

    /// Two passes of a periodic `[1 2 1] / 4` blur along each axis: a stretched void has
    /// particles several cells apart, whose bare deposits alias into a moiré.
    fn smooth(&mut self) {
        let (w, h) = self.dims();
        for pass in 0..3 {
            if pass == 1 {
                self.sharp
                    .as_mut_slice()
                    .copy_from_slice(self.density.as_slice());
            }
            let (src, dst) = (self.density.as_slice(), self.scratch.as_mut_slice());
            for j in 0..h {
                let row = &src[j * w..(j + 1) * w];
                for i in 0..w {
                    let (l, r) = (row[(i + w - 1) % w], row[(i + 1) % w]);
                    dst[j * w + i] = 0.25 * (l + r) + 0.5 * row[i];
                }
            }
            let (src, dst) = (self.scratch.as_slice(), self.density.as_mut_slice());
            for j in 0..h {
                let (up, down) = ((j + h - 1) % h, (j + 1) % h);
                for i in 0..w {
                    dst[j * w + i] =
                        0.25 * (src[up * w + i] + src[down * w + i]) + 0.5 * src[j * w + i];
                }
            }
        }
        // Sharp caustics where matter has gathered, the smooth field in the voids.
        for (d, &s) in self
            .density
            .as_mut_slice()
            .iter_mut()
            .zip(self.sharp.as_slice())
        {
            let k = ((s - 1.2) / 1.5).clamp(0.0, 1.0);
            *d += (s - *d) * k;
        }
    }

    fn raster(&mut self) {
        self.deposit(growth(self.t));
        self.smooth();
        let halo = halo_threshold(self.density.as_slice());
        let (map, density, tone) = (&self.map, &self.density, &self.tone);
        let direct = density.size() == self.pixels.pixel_size();
        let w = usize::from(density.width());
        self.pixels.shade(|x, y| {
            let v = if direct {
                density.as_slice()[y as usize * w + x as usize]
            } else {
                map.sample(density, x, y)
            };
            if v >= halo {
                let k = f64::from((v - halo) / halo).clamp(0.0, 1.0);
                return ink::blackbody(ease::lerp(HALO_K.0, HALO_K.1, k));
            }
            // A gamma below the tone darkens the fog so the filaments carry the frame.
            let t = f32::from(tone.level(v - FLOOR)) / 255.0;
            let level = (255.0 * t.powf(GAMMA)) as u8;
            if f64::from(level) < ink::MIN_VISIBLE_LEVEL {
                Ink::BLANK
            } else {
                // The faintest fog in Cobalt's indigo-blue, the web in Plasma's violet, so
                // the frame sits bluer than lichtenberg_discharge's violet. The switch is
                // at a level so dark that the two ramps' hues barely differ there.
                let ramp = if level < FOG_BLUE {
                    Ramp::Cobalt
                } else {
                    Ramp::Plasma
                };
                Ink::new(Role::Ramp(ramp), level)
            }
        });
    }
}

impl Generator for CosmicWeb {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let lattice = lattice_for(px, MAX_NODES, 1.0);
        self.density.resize(lattice, 0.0);
        self.map.rebuild(lattice, px);
        self.cycle = (self.t / LOOP).floor() as i64;
        let lam0 = self.fill_all(0);
        self.norm = if lam0 < 0.0 { -1.0 / lam0 } else { 1.0 };
        if self.cycle != 0 {
            self.fill_all(self.cycle);
        }
        // Size every per-cycle buffer now, so steady-state updates never allocate
        // (GEN-R11): the next cycle's table and tables, and the blur scratch.
        let (w, h) = self.dims();
        self.next.clear();
        self.next.reserve(w * h);
        self.next_tables.build(&self.modes, self.cycle + 1, w, h);
        self.scratch.resize(lattice, 0.0);
        self.sharp.resize(lattice, 0.0);
        self.next_done = 0;
        self.raster();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        let (w, h) = self.dims();
        let n = w * h;
        let cycle = (self.t / LOOP).floor() as i64;
        if cycle != self.cycle {
            // The relax phase ended at D = 0, so the swap is invisible.
            if cycle == self.cycle + 1 {
                self.fill_next(n);
                std::mem::swap(&mut self.disp, &mut self.next);
            } else {
                self.fill_all(cycle);
            }
            self.cycle = cycle;
            self.next_done = 0;
        } else {
            // Spread the next cycle's tables over the first two thirds of the loop.
            let chunk = n.div_ceil((LOOP * 40.0) as usize).max(1);
            self.fill_next(chunk);
        }
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (CosmicWeb, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = CosmicWeb::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// std / mean of the density lattice.
    fn contrast(g: &CosmicWeb) -> f64 {
        let v = g.density.as_slice();
        let n = v.len() as f64;
        let mean = v.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
        let var = v
            .iter()
            .map(|&x| (f64::from(x) - mean).powi(2))
            .sum::<f64>()
            / n;
        var.sqrt() / mean
    }

    #[test]
    fn web_a1_positions_are_grid_plus_d_times_s() {
        let (mut g, _) = built(Size::new(80, 24));
        let (w, h) = g.dims();
        g.fill_all(0);
        g.deposit(0.0);
        // D = 0: every node holds exactly one particle's worth.
        assert!(g.density.as_slice().iter().all(|&v| (v - 1.0).abs() < 1e-5));
        for &(i, j) in &[(0usize, 0usize), (5, 7), (w - 1, h - 1), (w / 2, h / 3)] {
            let (sx, sy, _) = g.disp[j * w + i];
            let direct = displacement_direct(&g.modes, 0, w, h, (i as f64, j as f64));
            assert!((f64::from(sx) - direct.0).abs() <= 1e-6 * direct.0.abs().max(1.0));
            assert!((f64::from(sy) - direct.1).abs() <= 1e-6 * direct.1.abs().max(1.0));
            let (tab, _) = g.tables.at(&g.modes, w, h, i, j);
            assert!((tab.0 - direct.0).abs() < 1e-9 && (tab.1 - direct.1).abs() < 1e-9);
        }
        // Before its own crossing a particle sits at q + D·s exactly; after, it stays put.
        let s = (0.3f32, -0.7f32);
        assert_eq!(displaced(4.0, 9.0, s, 0.0, 2.0), (4.0, 9.0));
        assert_eq!(
            displaced(4.0, 9.0, s, 1.5, 2.0),
            (4.0 + 1.5 * 0.3, 9.0 - 1.5 * 0.7)
        );
        assert_eq!(
            displaced(4.0, 9.0, s, 3.0, 2.0),
            displaced(4.0, 9.0, s, 2.0, 2.0)
        );
        assert_eq!(
            displaced(4.0, 9.0, s, 3.0, f32::INFINITY),
            (4.0 + 0.9, 9.0 - 2.1)
        );
    }

    #[test]
    fn web_a6_the_web_is_sharpest_in_the_hold_not_smeared() {
        let (mut g, _) = built(Size::new(200, 50));
        g.fill_all(0);
        g.deposit(growth(15.0));
        let mid = contrast(&g);
        g.deposit(growth(RISE + HOLD / 2.0));
        assert!(contrast(&g) >= mid, "{} < {mid}", contrast(&g));
    }

    #[test]
    fn web_a2_density_contrast_rises_with_growth() {
        let (mut g, _) = built(Size::new(200, 50));
        g.fill_all(0);
        let mut last = 0.0;
        for step in 1..=20 {
            let d = f64::from(step) / 20.0;
            g.deposit(d);
            let c = contrast(&g);
            assert!(c > last, "contrast fell at D = {d}: {c} ≤ {last}");
            last = c;
        }
    }

    #[test]
    fn web_a3_at_d_max_a_third_of_the_mass_sits_in_the_densest_tenth() {
        let (mut g, _) = built(Size::new(200, 50));
        g.fill_all(0);
        g.deposit(1.0);
        let mut v: Vec<f32> = g.density.as_slice().to_vec();
        v.sort_by(|a, b| b.total_cmp(a));
        let total: f32 = v.iter().sum();
        let top: f32 = v[..v.len() / 10].iter().sum();
        assert!(top / total >= 0.35, "densest 10 % hold {}", top / total);
    }

    #[test]
    fn web_a4_displacement_is_curl_free() {
        let (g, _) = built(Size::new(120, 40));
        let (w, h) = g.dims();
        // Analytic Jacobian: ∂s_y/∂x and ∂s_x/∂y are the same sum.
        for &(x, y) in &[(3.3, 4.1), (50.0, 17.5), (0.2, 0.9)] {
            let (mut a, mut b, mut mag) = (0.0, 0.0, 0.0);
            for m in &g.modes {
                let (kx, ky) = wavevector(m, w, h);
                let c = (kx * x + ky * y + m.phase).cos_det();
                a += m.amp * ky * kx * c;
                b += m.amp * kx * ky * c;
                mag += (m.amp * kx * kx * c).abs();
            }
            assert!((a - b).abs() <= 1e-9 * mag.max(1e-300));
        }
        // Discrete circulation round a small plaquette vanishes to O(h²).
        let hh = 1e-3;
        let s = |x: f64, y: f64| displacement_direct(&g.modes, 0, w, h, (x, y));
        let (x, y) = (10.0, 12.0);
        let circ = (s(x, y - hh).0 - s(x, y + hh).0) + (s(x + hh, y).1 - s(x - hh, y).1);
        let mag = s(x, y).0.hypot(s(x, y).1) * 2.0 * hh;
        assert!(circ.abs() <= 1e-4 * mag.max(1e-12), "circulation {circ}");
    }

    #[test]
    fn web_a5_the_loop_repeats_with_shifted_phases() {
        for t in [0.0, 3.0, 12.5, 21.0, 27.0] {
            assert_eq!(growth(t), growth(t + LOOP));
        }
        assert_eq!(growth(0.0), 0.0);
        assert!(growth(LOOP - 1e-9) < 1e-6);
        // The next cycle's precomputed displacements are the cycle-0 field with every
        // mode's phase advanced by its step.
        let size = Size::new(80, 24);
        let (mut g, mut ctx) = built(size);
        g.t = 0.0;
        g.cycle = 0;
        g.fill_all(0);
        g.next_done = 0;
        while g.t + SIM_DT < LOOP {
            g.update(SIM_DT, &mut ctx);
        }
        let (w, h) = g.dims();
        g.update(SIM_DT * 2.0, &mut ctx);
        assert_eq!(g.cycle, 1);
        for &(i, j) in &[(1usize, 2usize), (w - 2, h - 1)] {
            let (sx, sy, _) = g.disp[j * w + i];
            let direct = displacement_direct(&g.modes, 1, w, h, (i as f64, j as f64));
            assert!((f64::from(sx) - direct.0).abs() <= 1e-5 * direct.0.abs().max(1.0));
            assert!((f64::from(sy) - direct.1).abs() <= 1e-5 * direct.1.abs().max(1.0));
        }
    }
}
