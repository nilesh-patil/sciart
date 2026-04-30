//! A 2-D Lennard-Jones fluid in a box, slowly heated and cooled so it melts into a gas and
//! condenses back into droplets and crystallites (SPEC-002 `molecular_dynamics`).
//!
//! Reduced LJ units throughout (σ = ε = m = k_B = 1). Model:
//! - pair potential `U(r) = 4(r⁻¹² − r⁻⁶) − U(r_c)`, shifted to 0 at the cutoff `r_c = 2.5`;
//! - velocity-Verlet with sub-steps `h ≤ 0.004 τ`, one simulated second = [`TIME_SCALE`] τ;
//! - a cell list (cells ≥ `r_c`) so forces cost O(N), half-stencil + Newton's third law;
//! - specular (elastic) walls, applied by mirror-folding during the drift;
//! - a Berendsen thermostat (τ_T = 0.5 s) chasing a 30 s cycle between T = 0.25 and 1.0;
//! - a per-terminal layout ([`layout`]): the screen scale (rows per σ) and N are chosen from
//!   the terminal area so every size shows the same reduced density ρ = [`RHO`], which lies
//!   inside the 2-D LJ vapour–liquid dome — the cold phase visibly separates into droplets.
//!
//! Blow-up guards (the r⁻¹² core is the classic way an MD toy explodes):
//! 1. atoms start on a lattice at ≥ the LJ minimum spacing, never overlapping;
//! 2. every frame is sub-stepped, so a frame `dt` of 0.25 s is 375 ordinary MD steps;
//! 3. below [`R_CAP`] the potential continues *linearly* (constant repulsive force), so a
//!    pathological overlap produces a large but finite kick instead of ∞/NaN. Because U and
//!    F stay consistent (F = −dU/dr everywhere), energy conservation is unaffected; at the
//!    temperatures used here (T ≲ 1.6) pairs never get that close (U(R_CAP) ≈ 43 ε).
//! 4. the sim box is never smaller than [`MIN_BOX`] per side nor denser than
//!    1/[`MIN_AREA_PER_ATOM`]; on a tiny terminal the box is larger than the screen and
//!    simply clipped.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::particles::{gaussian2, plot};
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};

pub static META: Meta = Meta {
    name: "molecular_dynamics",
    category: Category::PhysicalChemistry,
    description: "A Lennard-Jones fluid melting and condensing as it is heated and cooled",
    title: "Gas to liquid · cool atoms cling",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(MolecularDynamics::default())
}

/// Potential cutoff, σ.
const RC: f64 = 2.5;
const RC2: f64 = RC * RC;
/// Linear-continuation radius for the repulsive core (guard 3 above), σ.
const R_CAP: f64 = 0.8;
/// LJ minimum `2^{1/6}`: the lattice spacing of the initial crystal.
const R_MIN: f64 = 1.122_462_048_309_373;
/// Target reduced number density N/A. The scene is 2-D, so the relevant phase diagram is the
/// 2-D LJ one, not the familiar 3-D one (T_c ≈ 1.3, T_t ≈ 0.69): for the potential truncated
/// and shifted at 2.5, T_c ≈ 0.46 and ρ_c ≈ 0.33–0.35 (Smit & Frenkel 1991; full potential
/// T_c ≈ 0.53), triple point T_t ≈ 0.4. ρ = 0.25 sits inside the vapour–liquid dome on the
/// low-density side of ρ_c: cold, it separates into dense droplets (ρ ≈ 0.75) in a thin
/// vapour (ρ ≈ 0.01) covering about a third of the box — droplets, not a percolating
/// sponge; hot, it is a uniform supercritical gas with ~2 σ between atoms.
const RHO: f64 = 0.25;
/// Screen rows per σ (x is additionally stretched by ASPECT), chosen per terminal: as small
/// as legibility allows on small screens (for more atoms), growing on large screens once
/// N hits [`N_MAX`]. At `S_MIN` the closest-packed crystal rows (`R_MIN·√3/2 σ`) are 1.07
/// rows apart, so neighbouring atoms never share a cell.
const S_MIN: f64 = 1.1;
const S_MAX: f64 = 2.5;
/// Reduced time units per simulated second. 6 τ/s gives nucleation and coarsening ~60 τ of
/// sub-critical time per cycle (droplets by ~10 s after the peak) at a thermal speed of
/// ~6 rows/s at T = 1.
const TIME_SCALE: f64 = 6.0;
/// Largest velocity-Verlet step, τ.
const DT_MD_MAX: f64 = 0.004;
/// Temperature cycle: period (s), extremes, and Berendsen coupling time (s).
/// T_LO is below the 2-D triple point (crystallites), T_HI ≈ 2 T_c (a clear gas). The
/// squared-cosine profile ([`target_temperature`]) keeps the target below T_c for ~52 % of
/// each cycle (vs ~35 % for a plain cosine), so the latent heat released on condensing can
/// be removed and droplets have time to coarsen.
const CYCLE: f64 = 30.0;
const T_LO: f64 = 0.25;
const T_HI: f64 = 1.0;
const TAU_T: f64 = 0.5;
/// Atom count bounds; the cap keeps a step + render well under 1 ms at 200x50 (GEN-R8).
const N_MIN: usize = 8;
const N_MAX: usize = 240;
/// Sim-box floor (guard 4).
const MIN_BOX: f64 = 3.0;
const MIN_AREA_PER_ATOM: f64 = 2.5;
/// Start slightly warmer than the cycle's floor so the crystal visibly shivers at once.
const T_START: f64 = 0.4;
/// Pairs closer than this get a faint bond dot (shows lattices and droplets).
const BOND_R2: f64 = 1.3 * 1.3;
/// Kinetic energy splitting Cold from Hot ink.
const KE_SPLIT: f64 = 0.45;

const NONE: usize = usize::MAX;
const ATOM: Glyph = Glyph::new('●', 'O');
const BOND: Glyph = Glyph::new('·', '.');

/// Shifted LJ pair potential with the linear core continuation, 0 at and beyond `RC`
/// (MD-A1).
pub(crate) fn lj_potential(r: f64) -> f64 {
    if r.is_nan() || r >= RC {
        return 0.0;
    }
    if r < R_CAP {
        // Linear continuation: U(R_CAP) + F(R_CAP)(R_CAP − r), so F = −dU/dr stays exact.
        return lj_raw(R_CAP) - lj_raw(RC) + lj_force(R_CAP) * (R_CAP - r);
    }
    lj_raw(r) - lj_raw(RC)
}

/// Unshifted `4(r⁻¹² − r⁻⁶)`.
fn lj_raw(r: f64) -> f64 {
    let s6 = (r * r).powi(-3);
    4.0 * (s6 * s6 - s6)
}

/// Radial pair force `F(r) = −dU/dr` (positive = repulsive), 0 beyond `RC` (MD-A1).
pub(crate) fn lj_force(r: f64) -> f64 {
    if r.is_nan() || r >= RC {
        return 0.0;
    }
    let r = r.max(R_CAP);
    let s6 = (r * r).powi(-3);
    24.0 * (2.0 * s6 * s6 - s6) / r
}

/// `F(r)/r` from `r²` — what the force loop multiplies `(dx, dy)` by; no square root outside
/// the capped core.
#[inline]
pub(crate) fn force_over_r(r2: f64) -> f64 {
    if r2.is_nan() || r2 >= RC2 {
        return 0.0;
    }
    if r2 < R_CAP * R_CAP {
        // Constant-magnitude core force; guard r = 0 (coincident atoms get no direction).
        let r = r2.sqrt();
        return if r > 0.0 { lj_force(R_CAP) / r } else { 0.0 };
    }
    let inv2 = 1.0 / r2;
    let s6 = inv2 * inv2 * inv2;
    24.0 * inv2 * (2.0 * s6 * s6 - s6)
}

/// Berendsen velocity scale `λ = √(1 + (dt/τ)(T₀/T − 1))`, clamped to `[0.8, 1.25]` so a
/// long frame or a near-zero temperature can never jolt the system; 1 when `T ≈ 0` (MD-A4).
pub(crate) fn berendsen_lambda(t: f64, t0: f64, dt: f64, tau: f64) -> f64 {
    if !(t > 0.0 && t.is_finite() && t0.is_finite() && dt > 0.0 && tau > 0.0) {
        return 1.0;
    }
    (1.0 + dt / tau * (t0 / t - 1.0))
        .max(0.0)
        .sqrt()
        .clamp(0.8, 1.25)
}

/// Thermostat target at scene time `t`: `T_LO + (T_HI − T_LO)·u²` with
/// `u = (1 − cos 2πt/30)/2` — starts cold, lingers there, peaks at 15 s, cold again at 30 s.
pub(crate) fn target_temperature(t: f64) -> f64 {
    let u = 0.5 * (1.0 - (TAU * t / CYCLE).cos());
    T_LO + (T_HI - T_LO) * u * u
}

/// Screen scale (rows per σ) and atom count for an interior spanning `w` cols × `h` rows
/// between the outermost atom centres: `s = clamp(√(ρ·A₁/N_MAX), S_MIN, S_MAX)` and
/// `N = clamp(round(ρ·A₁/s²), N_MIN, N_MAX)`, with `A₁ = w·h/ASPECT` the area in σ² at one
/// row per σ. Small screens get the finest legible scale and as many atoms as the density
/// asks for; large screens get N_MAX atoms and a coarser scale that keeps the density.
pub(crate) fn layout(w: f64, h: f64) -> (f64, usize) {
    let a1 = w.max(0.0) * h.max(0.0) / ASPECT;
    let scale = (RHO * a1 / N_MAX as f64).sqrt().clamp(S_MIN, S_MAX);
    let n = (RHO * a1 / (scale * scale)).round() as usize;
    (scale, n.clamp(N_MIN, N_MAX))
}

/// Mirror-fold `x` into `[0, len]`; `true` when a velocity must flip (odd reflections).
fn fold(x: f64, len: f64) -> (f64, bool) {
    if !x.is_finite() || len.is_nan() || len <= 0.0 {
        return (0.0, false);
    }
    let u = x.rem_euclid(2.0 * len);
    if u > len {
        ((2.0 * len - u).clamp(0.0, len), true)
    } else {
        (u.clamp(0.0, len), false)
    }
}

/// Linked-cell neighbour structure over a `[0, lx] × [0, ly]` box. Buffers are sized in
/// `resize`, so rebuilding it every step allocates nothing.
#[derive(Debug, Default)]
struct CellList {
    ncx: usize,
    ncy: usize,
    cw: f64,
    ch: f64,
    head: Vec<usize>,
    next: Vec<usize>,
}

impl CellList {
    fn setup(&mut self, lx: f64, ly: f64, n: usize) {
        // Cells at least RC wide so the 3×3 neighbourhood holds every interacting pair.
        self.ncx = ((lx / RC).floor() as usize).max(1);
        self.ncy = ((ly / RC).floor() as usize).max(1);
        self.cw = lx / self.ncx as f64;
        self.ch = ly / self.ncy as f64;
        self.head.clear();
        self.head.resize(self.ncx * self.ncy, NONE);
        self.next.clear();
        self.next.resize(n, NONE);
    }

    fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        let cx = ((x / self.cw).max(0.0) as usize).min(self.ncx - 1);
        let cy = ((y / self.ch).max(0.0) as usize).min(self.ncy - 1);
        (cx, cy)
    }

    fn build(&mut self, x: &[f64], y: &[f64]) {
        self.head.fill(NONE);
        for i in 0..x.len() {
            let (cx, cy) = self.cell_of(x[i], y[i]);
            let c = cy * self.ncx + cx;
            self.next[i] = self.head[c];
            self.head[c] = i;
        }
    }

    /// Visit every unordered pair in the same or adjacent cells exactly once.
    fn for_each_pair(&self, mut f: impl FnMut(usize, usize)) {
        // Half stencil: self, E, SW, S, SE — each neighbouring cell pair is visited once.
        const STENCIL: [(isize, isize); 4] = [(1, 0), (-1, 1), (0, 1), (1, 1)];
        for cy in 0..self.ncy {
            for cx in 0..self.ncx {
                let mut i = self.head[cy * self.ncx + cx];
                while i != NONE {
                    let mut j = self.next[i];
                    while j != NONE {
                        f(i, j);
                        j = self.next[j];
                    }
                    for (dx, dy) in STENCIL {
                        let (nx, ny) = (cx as isize + dx, cy as isize + dy);
                        if nx < 0 || ny < 0 || nx as usize >= self.ncx || ny as usize >= self.ncy {
                            continue;
                        }
                        let mut j = self.head[ny as usize * self.ncx + nx as usize];
                        while j != NONE {
                            f(i, j);
                            j = self.next[j];
                        }
                    }
                    i = self.next[i];
                }
            }
        }
    }
}

/// Screen placement of the sim box.
#[derive(Debug, Default, Clone, Copy)]
struct View {
    /// Rows per σ (x is additionally stretched by ASPECT).
    scale: f64,
    /// Screen position (cols, rows) of sim coordinate (0, 0).
    c0: f64,
    r0: f64,
    /// Border rectangle (left, top, right, bottom) when there is room for one.
    border: Option<(i32, i32, i32, i32)>,
    label_row: Option<i32>,
}

#[derive(Debug, Default)]
pub struct MolecularDynamics {
    t: f64,
    thermostat_off: bool,
    lx: f64,
    ly: f64,
    // Structure-of-arrays: the force loop streams x/y and scatters into ax/ay.
    x: Vec<f64>,
    y: Vec<f64>,
    vx: Vec<f64>,
    vy: Vec<f64>,
    ax: Vec<f64>,
    ay: Vec<f64>,
    cells: CellList,
    view: View,
    readout: Readout,
}

/// What the bottom-right readout shows, sampled after each step: the energy sum is too
/// costly to redo in `render`, which only formats it.
#[derive(Debug, Default, Clone, Copy)]
struct Readout {
    temperature: f64,
    target: f64,
    mode: &'static str,
    u_per_atom: f64,
}

impl MolecularDynamics {
    fn n(&self) -> usize {
        self.x.len()
    }

    /// Instantaneous kinetic temperature `Σv²/(2N)` (2 degrees of freedom per atom).
    fn temperature(&self) -> f64 {
        let n = self.n();
        if n == 0 {
            return 0.0;
        }
        let s: f64 = self
            .vx
            .iter()
            .zip(&self.vy)
            .map(|(vx, vy)| vx * vx + vy * vy)
            .sum();
        s / (2.0 * n as f64)
    }

    /// Total pair potential energy over the (current) cell list. Falls as droplets form, so
    /// the readout shows condensation, not just temperature.
    fn potential_energy(&self) -> f64 {
        let mut u = 0.0;
        self.cells.for_each_pair(|i, j| {
            u += lj_potential((self.x[i] - self.x[j]).hypot(self.y[i] - self.y[j]));
        });
        u
    }

    #[cfg(test)]
    fn kinetic_energy(&self) -> f64 {
        self.temperature() * self.n() as f64
    }

    fn scale_velocities(&mut self, lambda: f64) {
        for v in self.vx.iter_mut().chain(self.vy.iter_mut()) {
            *v *= lambda;
        }
    }

    /// Rescale to exactly temperature `t` (no-op for a system at rest).
    fn set_temperature(&mut self, t: f64) {
        let now = self.temperature();
        if now > 0.0 {
            self.scale_velocities((t / now).sqrt());
        }
    }

    /// Rebuild the cell list and all accelerations (m = 1).
    fn compute_forces(&mut self) {
        self.cells.build(&self.x, &self.y);
        self.ax.fill(0.0);
        self.ay.fill(0.0);
        let (x, y, ax, ay) = (&self.x, &self.y, &mut self.ax, &mut self.ay);
        self.cells.for_each_pair(|i, j| {
            let (dx, dy) = (x[i] - x[j], y[i] - y[j]);
            let r2 = dx * dx + dy * dy;
            if r2 < RC2 {
                let fr = force_over_r(r2);
                ax[i] += fr * dx;
                ay[i] += fr * dy;
                ax[j] -= fr * dx;
                ay[j] -= fr * dy;
            }
        });
    }

    /// One velocity-Verlet step of `h` τ with specular walls.
    fn verlet(&mut self, h: f64) {
        let (lx, ly) = (self.lx, self.ly);
        for i in 0..self.n() {
            self.vx[i] += 0.5 * h * self.ax[i];
            self.vy[i] += 0.5 * h * self.ay[i];
            let (x, fx) = fold(self.x[i] + h * self.vx[i], lx);
            let (y, fy) = fold(self.y[i] + h * self.vy[i], ly);
            self.x[i] = x;
            self.y[i] = y;
            if fx {
                self.vx[i] = -self.vx[i];
            }
            if fy {
                self.vy[i] = -self.vy[i];
            }
        }
        self.compute_forces();
        for i in 0..self.n() {
            self.vx[i] += 0.5 * h * self.ax[i];
            self.vy[i] += 0.5 * h * self.ay[i];
        }
    }

    /// Place `n` atoms: a compact triangular crystal at the LJ minimum spacing when it fits,
    /// otherwise an even lattice spread over the whole box (guard 1).
    fn place_atoms(&mut self, n: usize) {
        let (lx, ly) = (self.lx, self.ly);
        let row_h = R_MIN * 3f64.sqrt() / 2.0;
        let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
        let rows = n.div_ceil(cols);
        let (bw, bh) = ((cols as f64 - 0.5) * R_MIN, (rows - 1) as f64 * row_h);
        self.x.clear();
        self.y.clear();
        if bw + 2.0 <= lx && bh + 2.0 <= ly {
            let (x0, y0) = ((lx - bw) / 2.0, (ly - bh) / 2.0);
            for k in 0..n {
                let (i, j) = (k % cols, k / cols);
                let shift = if j % 2 == 1 { 0.5 * R_MIN } else { 0.0 };
                self.x.push(x0 + i as f64 * R_MIN + shift);
                self.y.push(y0 + j as f64 * row_h);
            }
        } else {
            let cols = ((n as f64 * lx / ly).sqrt().round() as usize).clamp(1, n);
            let rows = n.div_ceil(cols);
            let (ax, ay) = (lx / cols as f64, ly / rows as f64);
            for k in 0..n {
                let (i, j) = (k % cols, k / cols);
                self.x.push((i as f64 + 0.5) * ax);
                self.y.push((j as f64 + 0.5) * ay);
            }
        }
    }

    fn refresh_readout(&mut self) {
        let heating = self.t.rem_euclid(CYCLE) < CYCLE / 2.0;
        self.readout = Readout {
            temperature: self.temperature(),
            target: target_temperature(self.t),
            mode: if self.thermostat_off {
                "NVE"
            } else if heating {
                "heating"
            } else {
                "cooling"
            },
            u_per_atom: self.potential_energy() / self.n().max(1) as f64,
        };
    }

    /// The readout, right-aligned on the label row and clear of the title (GEN-R13). The
    /// longest form that fits wins: the units note goes first, then the energy and atom
    /// count, then the target temperature; nothing when even `T = … heating` does not fit.
    fn draw_readout(&self, canvas: &mut Canvas, row: i32) {
        let room = title::readout_room(canvas.size(), &Title::of(self));
        let Readout {
            temperature: t,
            target,
            mode,
            u_per_atom: u,
        } = self.readout;
        let n = self.n();
        // Typically 70 bytes at most; the slack keeps a runaway temperature from truncating
        // it. Writing into a StackText never fails.
        let mut text = StackText::<128>::new();
        for form in 0..4 {
            text.clear();
            let _ = match form {
                0 => write!(
                    text,
                    "T = {t:.2} -> {target:.2} {mode}   U/N = {u:+.2}   N = {n}   LJ reduced units"
                ),
                1 => write!(
                    text,
                    "T = {t:.2} -> {target:.2} {mode}   U/N = {u:+.2}   N = {n}"
                ),
                2 => write!(text, "T = {t:.2} -> {target:.2} {mode}"),
                _ => write!(text, "T = {t:.2} {mode}"),
            };
            if text.len() <= room {
                draw::label_right(
                    canvas,
                    row,
                    text.as_str(),
                    Ink::new(Role::Text, title::READOUT_LEVEL),
                );
                return;
            }
        }
    }

    fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.view.c0 + x * self.view.scale * ASPECT,
            self.view.r0 + y * self.view.scale,
        )
    }
}

impl Generator for MolecularDynamics {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let label_row = (h >= 8).then_some(h - 1);
        let hb = if label_row.is_some() { h - 1 } else { h };
        // Interior (cols, rows) inside a 1-cell border when there is room for one.
        let (border, ic0, ir0, iw, ih) = if w >= 6 && hb >= 4 {
            (Some((0, 0, w - 1, hb - 1)), 1, 1, w - 2, hb - 2)
        } else {
            (None, 0, 0, w, hb)
        };
        let (iw1, ih1) = (f64::from(iw - 1).max(0.0), f64::from(ih - 1).max(0.0));
        let (scale, n) = layout(iw1, ih1);
        let screen_lx = iw1 / (scale * ASPECT);
        let screen_ly = ih1 / scale;

        let (mut lx, mut ly) = (screen_lx.max(MIN_BOX), screen_ly.max(MIN_BOX));
        let need = MIN_AREA_PER_ATOM * n as f64;
        if lx * ly < need {
            if lx < ly {
                lx = need / ly;
            } else {
                ly = need / lx;
            }
        }
        self.lx = lx;
        self.ly = ly;

        // Center the sim box on the interior (it is clipped when larger than the screen).
        let (icx, icy) = (
            f64::from(ic0) + f64::from(iw - 1).max(0.0) / 2.0,
            f64::from(ir0) + f64::from(ih - 1).max(0.0) / 2.0,
        );
        self.view = View {
            scale,
            c0: icx - lx / 2.0 * scale * ASPECT,
            r0: icy - ly / 2.0 * scale,
            border,
            label_row,
        };

        self.t = 0.0;
        self.place_atoms(n);
        self.vx.clear();
        self.vy.clear();
        for _ in 0..n {
            let (vx, vy) = gaussian2(&mut ctx.rng, T_START.sqrt());
            self.vx.push(vx);
            self.vy.push(vy);
        }
        // Zero net momentum, then hit the start temperature exactly.
        let (mx, my) = (
            self.vx.iter().sum::<f64>() / n as f64,
            self.vy.iter().sum::<f64>() / n as f64,
        );
        self.vx.iter_mut().for_each(|v| *v -= mx);
        self.vy.iter_mut().for_each(|v| *v -= my);
        self.set_temperature(T_START);

        self.ax.clear();
        self.ax.resize(n, 0.0);
        self.ay.clear();
        self.ay.resize(n, 0.0);
        self.cells.setup(lx, ly, n);
        self.compute_forces();
        self.refresh_readout();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 || self.n() == 0 {
            return;
        }
        let span = dt * TIME_SCALE;
        let steps = (span / DT_MD_MAX).ceil().max(1.0);
        let h = span / steps;
        for _ in 0..steps as usize {
            self.verlet(h);
        }
        self.t += dt;
        if !self.thermostat_off {
            let lambda =
                berendsen_lambda(self.temperature(), target_temperature(self.t), dt, TAU_T);
            self.scale_velocities(lambda);
        }
        self.refresh_readout();
    }

    fn render(&self, canvas: &mut Canvas) {
        if let Some((l, t, r, b)) = self.view.border {
            let ink = Ink::new(Role::Structure, 110);
            for x in l + 1..r {
                canvas.put(x, t, Glyph::new('─', '-'), ink);
                canvas.put(x, b, Glyph::new('─', '-'), ink);
            }
            for y in t + 1..b {
                canvas.put(l, y, Glyph::new('│', '|'), ink);
                canvas.put(r, y, Glyph::new('│', '|'), ink);
            }
            canvas.put(l, t, Glyph::new('┌', '+'), ink);
            canvas.put(r, t, Glyph::new('┐', '+'), ink);
            canvas.put(l, b, Glyph::new('└', '+'), ink);
            canvas.put(r, b, Glyph::new('┘', '+'), ink);
        }

        // Bonds first so atoms draw over them. The cell list is current: it was rebuilt
        // after the last drift.
        self.cells.for_each_pair(|i, j| {
            let (dx, dy) = (self.x[i] - self.x[j], self.y[i] - self.y[j]);
            if dx * dx + dy * dy < BOND_R2 {
                let (c, r) =
                    self.to_screen((self.x[i] + self.x[j]) / 2.0, (self.y[i] + self.y[j]) / 2.0);
                plot(canvas, c, r, BOND, Ink::new(Role::Structure, 70));
            }
        });

        for i in 0..self.n() {
            let e = 0.5 * (self.vx[i] * self.vx[i] + self.vy[i] * self.vy[i]);
            let ink = if e < KE_SPLIT {
                Ink::new(Role::Cold, (140.0 + 115.0 * (1.0 - e / KE_SPLIT)) as u8)
            } else {
                let u = ((e - KE_SPLIT) / 2.0).min(1.0);
                Ink::new(Role::Hot, (140.0 + 115.0 * u) as u8)
            };
            let (c, r) = self.to_screen(self.x[i], self.y[i]);
            plot(canvas, c, r, ATOM, ink);
        }

        if let Some(row) = self.view.label_row {
            self.draw_readout(canvas, row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(w: u16, h: u16, seed: u64) -> (MolecularDynamics, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = MolecularDynamics::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// O(N²) reference energy with the same pair potential.
    fn potential_energy(g: &MolecularDynamics) -> f64 {
        let mut u = 0.0;
        for i in 0..g.n() {
            for j in i + 1..g.n() {
                u += lj_potential((g.x[i] - g.x[j]).hypot(g.y[i] - g.y[j]));
            }
        }
        u
    }

    fn total_energy(g: &MolecularDynamics) -> f64 {
        g.kinetic_energy() + potential_energy(g)
    }

    fn min_pair_distance(g: &MolecularDynamics) -> f64 {
        let mut d = f64::INFINITY;
        for i in 0..g.n() {
            for j in i + 1..g.n() {
                d = d.min((g.x[i] - g.x[j]).hypot(g.y[i] - g.y[j]));
            }
        }
        d
    }

    #[test]
    fn md_a1_force_is_minus_du_dr_and_vanishes_beyond_cutoff() {
        let eps = 1e-6;
        let mut r = 0.55;
        while r < 3.2 {
            // Skip the two C¹ kinks (cap join, cutoff) where the central difference straddles.
            if (r - R_CAP).abs() > 1e-3 && (r - RC).abs() > 1e-3 {
                let fd = (lj_potential(r - eps) - lj_potential(r + eps)) / (2.0 * eps);
                let f = lj_force(r);
                assert!(
                    (fd - f).abs() <= 1e-6 * f.abs().max(1.0),
                    "r={r}: F={f} vs -dU/dr={fd}"
                );
                // The hot-loop form agrees with the scalar one.
                assert!((force_over_r(r * r) * r - f).abs() <= 1e-12 * f.abs().max(1.0));
            }
            r += 0.01;
        }
        for r in [RC, RC + 1e-9, 3.0, 10.0, f64::INFINITY] {
            assert_eq!(lj_force(r), 0.0, "r={r}");
            assert_eq!(lj_potential(r), 0.0, "r={r}");
            assert_eq!(force_over_r(r * r), 0.0, "r={r}");
        }
        // Shifted potential is continuous at the cutoff, and the minimum is at 2^{1/6}.
        assert!(lj_potential(RC - 1e-9).abs() < 1e-8);
        assert!(lj_force(R_MIN).abs() < 1e-12);
        assert!(lj_force(1.0) > 0.0 && lj_force(1.5) < 0.0);
        // The capped core stays finite and repulsive all the way to r = 0.
        assert!(lj_force(0.0).is_finite() && lj_force(0.0) > 0.0);
        assert!(lj_potential(0.0).is_finite());
        assert!(force_over_r(0.0).is_finite());
    }

    #[test]
    fn md_a1_cell_list_forces_match_brute_force() {
        let (mut g, mut ctx) = fresh(200, 50, 3);
        // Let the crystal melt a little so pairs straddle cell boundaries in every direction.
        for _ in 0..45 {
            g.update(SIM_DT, &mut ctx);
        }
        g.compute_forces();
        let (u_cells, u_all) = (g.potential_energy(), potential_energy(&g));
        assert!((u_cells - u_all).abs() < 1e-9 * u_all.abs().max(1.0));
        for i in 0..g.n() {
            let (mut fx, mut fy) = (0.0, 0.0);
            for j in 0..g.n() {
                if i != j {
                    let (dx, dy) = (g.x[i] - g.x[j], g.y[i] - g.y[j]);
                    let r = dx.hypot(dy);
                    if r < RC {
                        fx += lj_force(r) * dx / r;
                        fy += lj_force(r) * dy / r;
                    }
                }
            }
            assert!((g.ax[i] - fx).abs() < 1e-9 * fx.abs().max(1.0), "atom {i}");
            assert!((g.ay[i] - fy).abs() < 1e-9 * fy.abs().max(1.0), "atom {i}");
        }
    }

    /// Total-energy drift over 5 simulated seconds (20 τ) of NVE dynamics.
    fn nve_drift(frame_dt: f64) -> (f64, f64, f64) {
        let (mut g, mut ctx) = fresh(80, 24, 7);
        g.thermostat_off = true;
        // A lively liquid: the crystal at T = 1 melts, exercising walls and cell moves.
        g.set_temperature(1.0);
        let e0 = total_energy(&g);
        for _ in 0..(5.0 / frame_dt).round() as usize {
            g.update(frame_dt, &mut ctx);
        }
        let e1 = total_energy(&g);
        (e0, e1, (e1 - e0).abs() / e0.abs())
    }

    #[test]
    fn md_a2_energy_is_conserved_with_the_thermostat_off() {
        let (e0, e1, drift) = nve_drift(SIM_DT);
        assert!(drift < 0.01, "E {e0:.4} -> {e1:.4} ({drift:.2e})");
    }

    #[test]
    fn md_a2_energy_is_conserved_for_long_frames_too() {
        // dt = 0.25 s frames are sub-stepped to the same h ≤ 0.004 τ.
        let (e0, e1, drift) = nve_drift(0.25);
        assert!(drift < 0.01, "E {e0:.4} -> {e1:.4} ({drift:.2e})");
    }

    #[test]
    fn md_a3_atoms_stay_inside_the_box_for_any_size_and_step() {
        let mut ctx = Ctx::new(9, Size::new(1, 1));
        let mut g = MolecularDynamics::default();
        let dts = [SIM_DT, 0.25, 0.0, 0.07, SIM_DT, SIM_DT];
        for (w, h) in [(200, 50), (1, 1), (3, 40), (80, 24), (120, 2), (60, 70)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            assert!(
                min_pair_distance(&g) >= 0.99 * R_MIN.min(1.2),
                "{w}x{h}: overlap"
            );
            for k in 0..36 {
                g.update(dts[k % dts.len()], &mut ctx);
                for i in 0..g.n() {
                    assert!(
                        (0.0..=g.lx).contains(&g.x[i]) && (0.0..=g.ly).contains(&g.y[i]),
                        "{w}x{h}: atom {i} at ({}, {})",
                        g.x[i],
                        g.y[i]
                    );
                }
                assert!(g.temperature() < 10.0, "{w}x{h}: T = {}", g.temperature());
            }
        }
    }

    #[test]
    fn md_a4_berendsen_scale_points_toward_the_target() {
        assert!(berendsen_lambda(0.5, 1.0, SIM_DT, TAU_T) > 1.0);
        assert!(berendsen_lambda(1.5, 1.0, SIM_DT, TAU_T) < 1.0);
        assert_eq!(berendsen_lambda(1.0, 1.0, SIM_DT, TAU_T), 1.0);
        // First-order Berendsen: T' ≈ T + (dt/τ)(T₀ − T).
        let (t, t0, dt) = (0.5, 1.0, 0.01);
        let l = berendsen_lambda(t, t0, dt, TAU_T);
        assert!((l * l * t - (t + dt / TAU_T * (t0 - t))).abs() < 1e-12);
        // Guards: clamped, finite for T = 0 and huge dt.
        assert_eq!(berendsen_lambda(0.0, 1.0, SIM_DT, TAU_T), 1.0);
        assert_eq!(berendsen_lambda(1e-9, 1.0, 0.25, TAU_T), 1.25);
        assert_eq!(berendsen_lambda(100.0, 0.25, 1.0, TAU_T), 0.8);
    }

    #[test]
    fn md_a4_thermostat_heats_and_cools_the_fluid() {
        // Heating: 1 s before the peak (target ≈ 1.6), system at T = 0.3.
        let (mut g, mut ctx) = fresh(100, 30, 4);
        g.t = 14.0;
        g.set_temperature(0.3);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        let (t1, target) = (g.temperature(), target_temperature(g.t));
        assert!(t1 > 0.6, "heated to {t1}");
        assert!((t1 - target).abs() < (0.3 - target).abs());

        // Cooling: near the end of a cycle (target ≈ 0.25), system at T = 1.6.
        let (mut g, mut ctx) = fresh(100, 30, 5);
        g.t = 29.0;
        g.set_temperature(1.6);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        let (t1, target) = (g.temperature(), target_temperature(g.t));
        assert!(t1 < 1.2, "cooled to {t1}");
        assert!((t1 - target).abs() < (1.6 - target).abs());
    }

    #[test]
    fn md_temperature_cycle_spans_the_spec_range() {
        assert!((target_temperature(0.0) - T_LO).abs() < 1e-12);
        assert!((target_temperature(15.0) - T_HI).abs() < 1e-12);
        assert!((target_temperature(30.0) - T_LO).abs() < 1e-12);
        // The path crosses the 2-D LJ critical temperature (≈ 0.46, rc = 2.5 truncated and
        // shifted) both ways and stays below it for about half the cycle, above 2 T_c at the
        // peak.
        let (tc, samples) = (0.46, 3000u32);
        let below = (0..samples)
            .filter(|&k| target_temperature(CYCLE * f64::from(k) / f64::from(samples)) < tc)
            .count() as f64
            / f64::from(samples);
        assert!(
            (0.45..0.6).contains(&below),
            "below T_c {below:.2} of the cycle"
        );
        assert!(T_LO < 0.4 && T_HI > 2.0 * tc);
    }

    /// Mean number of neighbours within 1.5 σ: ≈ πr²ρ ≈ 1.8 for a uniform gas at ρ = 0.25,
    /// 4–6 inside a 2-D droplet or crystallite.
    fn coordination(g: &MolecularDynamics) -> f64 {
        let mut k = 0usize;
        for i in 0..g.n() {
            for j in i + 1..g.n() {
                if (g.x[i] - g.x[j]).hypot(g.y[i] - g.y[j]) < 1.5 {
                    k += 2;
                }
            }
        }
        k as f64 / g.n().max(1) as f64
    }

    #[test]
    fn md_atom_count_follows_the_area() {
        // Small screens: σ sits at its legibility floor and N = ρ·area/σ² — 159 at 80x24,
        // 3.8x the old fixed-scale count (42).
        let (g, _) = fresh(80, 24, 1);
        let a1 = 77.0 * 20.0 / ASPECT;
        assert_eq!(g.view.scale, S_MIN);
        assert_eq!(g.n(), (RHO * a1 / (S_MIN * S_MIN)).round() as usize);
        assert!(g.n() >= 120, "N = {}", g.n());
        // Large screens: N is capped (frame budget) and σ grows to keep the density.
        let (g, _) = fresh(200, 50, 1);
        assert_eq!(g.n(), N_MAX);
        assert!(g.view.scale > S_MIN && g.view.scale <= S_MAX);
        assert_eq!(fresh(1, 1, 1).0.n(), N_MIN);
    }

    #[test]
    fn md_a5_density_is_inside_the_2d_coexistence_dome() {
        // Wherever σ is not clamped the box holds the target density ρ = 0.25: between the
        // cold 2-D LJ vapour (≈ 0.01) and liquid/solid (≈ 0.8) branches and below ρ_c ≈ 0.35,
        // so the cold phase separates into droplets in a vapour.
        for (w, h) in [(80, 24), (100, 30), (120, 40), (160, 45), (200, 50)] {
            let (g, _) = fresh(w, h, 2);
            let rho = g.n() as f64 / (g.lx * g.ly);
            assert!((rho / RHO - 1.0).abs() < 0.05, "{w}x{h}: rho = {rho:.3}");
        }
    }

    #[test]
    fn md_glyphs_stay_legible_at_contact() {
        // Triangular-lattice rows (the closest vertical packing, r_min·√3/2 σ apart) must land
        // at least one terminal row apart, or crystal rows alias onto each other.
        for (w, h) in [(1, 1), (20, 10), (80, 24), (100, 30), (200, 50), (400, 100)] {
            let (g, _) = fresh(w, h, 3);
            let s = g.view.scale;
            assert!(R_MIN * 3f64.sqrt() / 2.0 * s >= 1.0, "{w}x{h}: {s} rows/σ");
            assert!((S_MIN..=S_MAX).contains(&s), "{w}x{h}: {s} rows/σ");
        }
    }

    #[test]
    fn md_a5_small_terminal_condenses_in_the_cold_phase() {
        // At 80x24, from the start crystal through the hot peak (15 s) into the next cold
        // phase: around the peak the fluid is a uniform supercritical gas (coordination ≈ πr²ρ
        // ≈ 1.8); by 28–33 s (T → T_LO, below the 2-D triple point) it has condensed again
        // into droplets and crystallites (≈ 4–5). The old fixed 1.6 rows/σ layout (N = 42,
        // ρ = 0.14) reached only 1.3 here.
        let (mut g, mut ctx) = fresh(80, 24, 11);
        let (mut hot, mut cold, mut nh, mut nc) = (0.0, 0.0, 0u32, 0u32);
        while g.t < 33.0 {
            g.update(0.05, &mut ctx);
            if (14.0..19.0).contains(&g.t) {
                hot += coordination(&g);
                nh += 1;
            } else if g.t >= 28.0 {
                cold += coordination(&g);
                nc += 1;
            }
        }
        let (hot, cold) = (hot / f64::from(nh), cold / f64::from(nc));
        assert!(
            cold > 2.0 * hot && cold > 3.5,
            "cold {cold:.2} vs hot {hot:.2}"
        );
    }

    #[test]
    fn md_renders_box_atoms_and_readout_in_ascii() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        for _ in 0..60 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let lines = frame.to_lines();
        assert!(lines.iter().all(|l| l.is_ascii()));
        assert!(lines[29].contains("T = "), "{}", lines[29]);
        assert!(frame.cells().iter().any(|c| c.ch == 'O'));
        assert!(lines[0].starts_with('+'));
    }

    #[test]
    fn md_gen_r13_readout_is_right_aligned_and_shortened_clear_of_the_title() {
        let readout = |w: u16| {
            let size = Size::new(w, 24);
            let (g, _) = fresh(w, 24, 42);
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut c);
            let row = c.to_lines().pop().unwrap();
            let span = title::span(size, &Title::new(META.title, None));
            // Four blanks after the title text, which ends at `cols.end - 2`.
            let free = span.map_or(0, |(_, cols)| cols.end as usize + 2);
            let lead = row.len() - row.trim_start().len();
            assert!(lead > free || row.trim().is_empty(), "{w}: {row:?}");
            assert!(row.ends_with(' '), "{w}: {row:?}");
            row.trim().to_owned()
        };
        let full = readout(200);
        assert!(full.starts_with("T = 0.40 -> ") && full.ends_with("   LJ reduced units"));
        // Beside the full "Gas to liquid · cool atoms cling" (32 cells, from w = 57) the
        // room is w − 38.
        let r = readout(100);
        assert!(
            r.contains("U/N = ") && r.contains("N = ") && !r.contains("LJ"),
            "{r}"
        );
        let r = readout(80);
        assert!(
            r.starts_with("T = 0.40 -> ") && r.ends_with(" heating"),
            "{r}"
        );
        assert_eq!(readout(60), "T = 0.40 heating");
        // Beside "Gas to liquid" alone the room is w − 19.
        assert_eq!(readout(40), "T = 0.40 heating");
        assert_eq!(readout(30), "");
    }
}
