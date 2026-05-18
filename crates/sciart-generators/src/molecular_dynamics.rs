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
//!    temperatures used here (T ≤ 1.0, the cycle's peak) pairs never get that close (U(R_CAP) ≈ 43 ε).
//! 4. the sim box is never smaller than [`MIN_BOX`] per side nor denser than
//!    1/[`MIN_AREA_PER_ATOM`]; on a tiny terminal the box is larger than the screen and
//!    simply clipped.
//!
//! Look (all rasterised in `update`/`resize`; `render` only blits): the box fills the
//! canvas above the title row. Atoms are round braille beads coloured by their smoothed
//! kinetic energy on a cool → hot Plasma ramp ([`heat_ink`], false colour), and hot atoms
//! leave dim Plasma-ramp streaks in a fading [`Density`]. On large terminals a top-right inset plots the
//! live speed histogram against the exact 2-D Maxwell–Boltzmann bins ([`mb_bins`]); atoms
//! pass behind it.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::particles::gaussian2;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, ink};

pub static META: Meta = Meta {
    name: "molecular_dynamics",
    category: Category::PhysicalChemistry,
    description: "A Lennard-Jones fluid melting and condensing as it is heated and cooled",
    title: "Condensation · cool atoms cling",
    captions: &CAPTIONS,
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
/// Spacing of the closest-packed crystal's rows, `R_MIN·√3/2` σ.
const ROW_H: f64 = R_MIN * 0.866_025_403_784_438_6;
/// Screen rows per σ (x is additionally stretched by ASPECT), chosen per terminal: as small
/// as legibility allows on small screens (for more atoms), growing on large screens once
/// N hits [`N_MAX`]. At `S_MIN` the crystal's rows are exactly one terminal row (two
/// pixels) apart: neighbouring atoms never share a cell, and single-pixel atoms (80×24)
/// land on evenly spaced pixel rows instead of beating into a blank row every ~7 rows.
const S_MIN: f64 = 1.0 / ROW_H;
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
/// 2-D LJ critical temperature for the potential truncated and shifted at r_c = 2.5
/// (Smit & Frenkel 1991, J. Chem. Phys. 94, 5663; the full potential's is ≈ 0.53). The
/// caption switches when the thermostat *target* crosses it (MD-A9), not on a measured
/// phase.
const T_C_2D: f64 = 0.46;
/// The two acts: condensing below T_c, a supercritical gas above it (GEN-R12).
const CAPTIONS: [&str; 2] = ["cool atoms cling", "hot atoms scatter"];

// The look (SPEC-002 molecular_dynamics, Draw). Atoms are round braille discs of this
// radius, σ: under half the LJ contact distance (≈ 0.56 σ), so touching atoms keep a thin
// dark seam and a crystal reads as a lattice of beads rather than a mosaic of tiles.
const ATOM_R: f64 = 0.42;
/// Kinetic energy → colour (MD-A6): `u = ln(1 + e/E_KNEE)/ln(1 + E_TOP/E_KNEE)`
/// ([`heat_u`]), drawn on the Plasma ramp from [`HEAT_FLOOR`]: the cold phase (ē ≈ 0.25,
/// u ≈ 0.23) is a deep violet, the hot gas (ē = 1, u ≈ 0.59) a bright lavender, the
/// fastest atoms near white. One hue family, so the hot/cold story is carried by
/// brightness in every theme; violet keeps the scene clear of the gallery's many ambers
/// and its many cold blues.
const E_KNEE: f64 = 0.4;
const E_TOP: f64 = 3.0;
/// Plasma-ramp position `clamp(HEAT_OFFSET + HEAT_GAIN·u, HEAT_FLOOR, 1)`: the cold phase
/// (u ≈ 0.23) at 0.40, the hot gas (u ≈ 0.59) at 0.87, and a crystal at rest never below
/// 0.35, so cold atoms stay clearly visible.
const HEAT_RAMP: Ramp = Ramp::Plasma;
const HEAT_OFFSET: f64 = 0.1;
const HEAT_GAIN: f64 = 1.3;
const HEAT_FLOOR: f64 = 0.35;
/// The atoms' colour is relative to the live temperature (MD-A9): the floor, and the
/// multiple of the mean kinetic energy that reaches the top of the ramp.
const REL_FLOOR: f64 = 0.35;
const REL_SPAN: f64 = 2.0;
/// An atom's colour follows its kinetic energy smoothed over this time (s): the
/// instantaneous energy of one atom is exponentially distributed, so unsmoothed a crystal
/// at one temperature reads as speckle.
const HEAT_TAU: f64 = 0.4;
/// Dots within this fraction of the radius are an atom's bright core; the rest of the
/// disc is its rim at [`RIM`] of the level, so a bead reads as a lit sphere.
const CORE: f64 = 0.62;
const RIM: f64 = 0.78;
/// Streaks: fade time (s), tone curve, and the top of the Plasma ramp they may reach:
/// level 31, well under the dimmest rim dot of any atom that streaks, so atoms stay the
/// brightest thing in every theme, Mono included (MD-A6).
const TRAIL_TAU: f64 = 0.12;
const TRAIL_KNEE: f32 = 0.02;
const TRAIL_WHITE: f32 = 0.25;
/// Smoothed heat above which an atom streaks: the cold phase sits near 0.25, the hot gas
/// near 1.
const E_STREAK: f64 = 0.5;
const TRAIL_MAX: f64 = 0.12;
/// Streak pixels whose tone level is below this are left empty (MD-A6): the faint fringe
/// of a streak was a scatter of near-black navy half-blocks beside the beads.
const TRAIL_CUT: u8 = 64;

// The Maxwell–Boltzmann inset (MD-A7): one speed bin per panel column.
const INSET_W: i32 = 26;
const INSET_H: i32 = 6;
/// Smallest canvas that gets the inset: below it the panel would cover over a tenth of the
/// box and its bars and curve merge into one braille blob.
const INSET_MIN: (u16, u16) = (100, 30);
const BINS: usize = INSET_W as usize;
/// Speed range of the bins; the last bin also holds every faster atom.
const V_MAX: f64 = 4.0;
const HIST_TAU: f64 = 0.3;
/// The MB peak sits at this fraction of the plot height.
const PEAK: f64 = 0.92;
const CURVE_INK: Ink = Ink::new(Role::Text, 150);
/// The legend: `speeds` in the bars' colour at T = 1, `Maxwell-Boltzmann` in the curve's.
const BARS_KEY: f64 = 1.0;

const NONE: usize = usize::MAX;

/// Heat position `u = min(1, ln(1 + e/E_KNEE)/ln(1 + E_TOP/E_KNEE))` of an atom with
/// kinetic energy `e` (LJ units; NaN and negatives are 0) (MD-A6), mapped to colour by
/// [`heat_ink`]. False colour: hotter-is-whiter reads at a glance.
pub(crate) fn heat_u(e: f64) -> f64 {
    let e = if e.is_nan() { 0.0 } else { e.max(0.0) };
    ((e / E_KNEE).ln_1p() / (E_TOP / E_KNEE).ln_1p()).min(1.0)
}

/// The colour of kinetic energy `e` (MD-A6): Plasma ramp at `clamp(0.1 + 1.3·u(e), 0.35,
/// 1)`, deep violet (cold) → lavender → near white (hot). False colour.
pub(crate) fn heat_ink(e: f64) -> Ink {
    ink::ramp(
        HEAT_RAMP,
        (HEAT_GAIN * heat_u(e) + HEAT_OFFSET).clamp(HEAT_FLOOR, 1.0),
    )
}

/// An atom's colour relative to the live temperature `t` (MD-A9): Plasma at
/// `REL_FLOOR + (1 − REL_FLOOR)·clamp(e/(REL_SPAN·t), 0, 1)`, with `e` its smoothed kinetic
/// energy (LJ units, k_B = 1; in 2-D the mean is `t`). An atom at the mean sits at 0.68,
/// one at twice the mean or more is near white, and a resting atom keeps the 0.35
/// floor. So in every phase the atoms hotter than their surroundings (the ones evaporating
/// off a cluster's edge) glow and the bulk stays calm, instead of the whole condensed
/// phase sinking to one dim indigo on an absolute scale. A non-positive or NaN `t` reads as
/// the floor. False colour; the absolute temperature is the readout's job.
pub(crate) fn relative_heat_ink(e: f64, t: f64) -> Ink {
    let u = if t > 0.0 && e.is_finite() {
        (e / (REL_SPAN * t)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ink::ramp(HEAT_RAMP, REL_FLOOR + (1.0 - REL_FLOOR) * u)
}

/// Exact 2-D Maxwell–Boltzmann probability of each speed bin at temperature `t`, the
/// last bin including the tail beyond [`V_MAX`] (MD-A7): `exp(−v₀²/2T) − exp(−v₁²/2T)`.
pub(crate) fn mb_bins(t: f64) -> [f64; BINS] {
    let t = if t > 0.0 && t.is_finite() { t } else { 1e-9 };
    let tail = |v: f64| (-v * v / (2.0 * t)).exp();
    std::array::from_fn(|b| {
        let (v0, v1) = (bin_edge(b), bin_edge(b + 1));
        if b + 1 == BINS {
            tail(v0)
        } else {
            tail(v0) - tail(v1)
        }
    })
}

fn bin_edge(b: usize) -> f64 {
    V_MAX * b as f64 / BINS as f64
}

/// Bin of speed `v` (NaN ⇒ 0, beyond the range ⇒ the last bin).
fn bin_of(v: f64) -> usize {
    ((v / V_MAX * BINS as f64).max(0.0) as usize).min(BINS - 1)
}

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
    let u = 0.5 * (1.0 - (TAU * t / CYCLE).cos_det());
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
    label_row: Option<i32>,
    /// Top-left cell of the Maxwell–Boltzmann panel, when there is room for it.
    inset: Option<(i32, i32)>,
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
    /// Each atom's kinetic energy smoothed over [`HEAT_TAU`]: what its colour shows.
    heat: Vec<f64>,
    /// Smoothed fraction of atoms per speed bin (MD-A7).
    hist: [f64; BINS],
    /// Long exposure of the atoms' kinetic energy: the streaks. One value per display
    /// *pixel* (a `Density` over half the cells each way, addressed in pixel units), since
    /// the streaks are drawn at pixel resolution.
    glow: Density,
    tone: Tone,
    /// Streaks over the sim box's rows, rebuilt in `update`/`resize`.
    pixels: Pixels,
    /// The atoms: round braille beads over the sim box's rows, drawn over the streaks.
    atoms: Dots,
    /// The inset's braille plot, over its rows only.
    plot: Dots,
}

/// What the bottom-right readout shows, sampled after each step: the energy sum is too
/// costly to redo in `render`, which only formats it.
#[derive(Debug, Default, Clone, Copy)]
struct Readout {
    temperature: f64,
    target: f64,
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
    /// centred in the box below `top` (σ, the inset's bottom edge) or else in the whole box;
    /// otherwise an even lattice spread over the whole box (guard 1).
    fn place_atoms(&mut self, n: usize, top: f64) {
        let (lx, ly) = (self.lx, self.ly);
        let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
        let rows = n.div_ceil(cols);
        let (bw, bh) = ((cols as f64 - 0.5) * R_MIN, (rows - 1) as f64 * ROW_H);
        self.x.clear();
        self.y.clear();
        if bw + 2.0 <= lx && bh + 2.0 <= ly {
            let top = if bh + 2.0 <= ly - top { top } else { 0.0 };
            let (x0, y0) = ((lx - bw) / 2.0, top + (ly - top - bh) / 2.0);
            // The first row on a pixel centre, so rows a whole number of pixels apart (at
            // S_MIN) all round the same way.
            let py = self.to_pixel(x0, y0).1;
            let y0 = y0 + (py.floor() + 0.5 - py) / (2.0 * self.view.scale);
            for k in 0..n {
                let (i, j) = (k % cols, k / cols);
                let shift = if j % 2 == 1 { 0.5 * R_MIN } else { 0.0 };
                self.x.push(x0 + i as f64 * R_MIN + shift);
                self.y.push(y0 + j as f64 * ROW_H);
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
        self.readout = Readout {
            temperature: self.temperature(),
            target: target_temperature(self.t),
            u_per_atom: self.potential_energy() / self.n().max(1) as f64,
        };
    }

    /// The readout, right-aligned on the label row and clear of the title (GEN-R13). The
    /// longest form that fits wins: the units note goes first, then the energy and atom
    /// count, then the target temperature; nothing when even `T 0.40` does not fit.
    fn draw_readout(&self, canvas: &mut Canvas, row: i32) {
        let room = title::readout_room(canvas.size(), &Title::of(self));
        let Readout {
            temperature: t,
            target,
            u_per_atom: u,
        } = self.readout;
        let n = self.n();
        // Typically 45 bytes at most; the slack keeps a runaway temperature from truncating
        // it. Writing into a StackText never fails.
        let mut text = StackText::<96>::new();
        for form in 0..4 {
            text.clear();
            let _ = match form {
                0 => write!(
                    text,
                    "T {t:.2} -> {target:.2}  U/N {u:+.2}  N {n}  LJ units"
                ),
                1 => write!(text, "T {t:.2} -> {target:.2}  U/N {u:+.2}  N {n}"),
                2 => write!(text, "T {t:.2} -> {target:.2}"),
                _ => write!(text, "T {t:.2}"),
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

    /// Continuous pixel coordinates of sim point `(x, y)`: pixel `(i, j)` spans
    /// `[i, i + 1) × [j, j + 1)`, and cell `(c, r)` holds pixels `c` and rows `2r, 2r + 1`.
    fn to_pixel(&self, x: f64, y: f64) -> (f64, f64) {
        let v = &self.view;
        (
            v.c0 + x * v.scale * ASPECT + 0.5,
            2.0 * (v.r0 + y * v.scale) + 1.0,
        )
    }

    /// Lay each hot atom's excess heat along its path: `(heat − E_STREAK)² · h`, so atoms
    /// rattling in a droplet or crystal leave nothing and only the gas streaks.
    fn deposit_streaks(&mut self, h: f64) {
        for i in 0..self.n() {
            let excess = (self.heat[i] - E_STREAK).max(0.0);
            if excess > 0.0 {
                let (px, py) = self.to_pixel(self.x[i], self.y[i]);
                // Pixel j spans [j, j + 1); `deposit` rounds to the nearest.
                let w = (excess * excess * h) as f32;
                self.glow.deposit(px - 0.5, py - 0.5, w);
            }
        }
    }

    /// Fold each atom's kinetic energy into its smoothed heat with weight `a`.
    fn sample_heat(&mut self, a: f64) {
        for ((h, vx), vy) in self.heat.iter_mut().zip(&self.vx).zip(&self.vy) {
            *h += a * (0.5 * (vx * vx + vy * vy) - *h);
        }
    }

    /// Fold the current speeds into the smoothed histogram with weight `a` (1 = replace).
    fn sample_speeds(&mut self, a: f64) {
        let mut count = [0usize; BINS];
        for (vx, vy) in self.vx.iter().zip(&self.vy) {
            count[bin_of(vx.hypot(*vy))] += 1;
        }
        let n = self.n().max(1) as f64;
        for (h, c) in self.hist.iter_mut().zip(count) {
            *h += a * (c as f64 / n - *h);
        }
    }

    /// Rebuild the picture: streaks, then the atoms' beads, then the inset (the M10 raster
    /// convention: `render` only blits).
    fn rasterise(&mut self) {
        let (glow, tone) = (&self.glow, &self.tone);
        self.pixels.shade(|x, y| {
            let level = tone.level(glow.get(x, y).unwrap_or(0.0));
            if level < TRAIL_CUT {
                Ink::BLANK
            } else {
                ink::ramp(Ramp::Plasma, TRAIL_MAX * f64::from(level) / 255.0)
            }
        });
        // The panel and its one-cell margin, in cells: atoms pass behind it.
        let panel = self
            .view
            .inset
            .map(|(x0, _)| (x0 - 1, x0 + INSET_W, INSET_H));
        // σ = `scale` rows = 4·scale dots; dots are square on screen.
        let r = ATOM_R * f64::from(Dots::CELL_H) * self.view.scale;
        self.atoms.clear();
        // Atoms are coloured relative to the live temperature, the readout's T (MD-A9).
        let t_now = self.temperature();
        for i in 0..self.n() {
            let (px, py) = self.to_pixel(self.x[i], self.y[i]);
            // Dot j spans pixel coordinates [j/2, (j + 1)/2).
            let (dx, dy) = (2.0 * px - 0.5, 2.0 * py - 0.5);
            paint_atom(
                &mut self.atoms,
                dx,
                dy,
                r,
                relative_heat_ink(self.heat[i], t_now),
                panel,
            );
        }
        self.plot.clear();
        if let Some((x0, _)) = self.view.inset {
            // A dark panel the streaks pass behind (one cell of margin).
            let rows = 2 * (INSET_H + 1);
            for y in 0..rows {
                for x in x0 - 1..x0 + INSET_W + 1 {
                    self.pixels.set(x, y, Ink::BLANK);
                }
            }
            self.plot_speeds(x0);
        }
    }

    /// Bars of the smoothed histogram in their bins' colours, and the exact MB bin
    /// probabilities at the measured T as a dotted grey curve, on rows 1..INSET_H (MD-A7).
    /// A braille cell has one ink, so the curve is drawn first and a bar reaching its
    /// cell recolours it: the grey shows where the live bars fall short of the law.
    fn plot_speeds(&mut self, x0: i32) {
        let mb = mb_bins(self.readout.temperature);
        let peak = mb.iter().copied().fold(0.0, f64::max).max(1e-9);
        let (bottom, height) = (
            INSET_H * Dots::CELL_H - 1,
            f64::from((INSET_H - 1) * Dots::CELL_H),
        );
        let dots = |p: f64| ((p / peak * PEAK * height).round() as i32).clamp(0, height as i32);
        for (b, (&frac, &p)) in self.hist.iter().zip(&mb).enumerate() {
            let x = (x0 + b as i32) * Dots::CELL_W;
            let v = 0.5 * (bin_edge(b) + bin_edge(b + 1));
            // The curve: one dot per bin, on the bin's right-hand column so it never hides
            // under a bar of the same height.
            let top = dots(p);
            if top > 0 {
                self.plot.set(x + 1, bottom + 1 - top, CURVE_INK);
            }
            let bar = heat_ink(0.5 * v * v);
            for k in 0..dots(frac) {
                self.plot.set(x, bottom - k, bar);
                self.plot.set(x + 1, bottom - k, bar);
            }
        }
    }
}

/// A round bead of radius `r` dots centred at dot `(cx, cy)`: dots within `CORE·r` at
/// `ink`, the rest at `RIM` of its level (a braille cell keeps its strongest ink, so a
/// cell holding core dots shows the core colour). Every atom lights at least its centre
/// dot. Dots in the inset panel's cells (`panel = (x0, x1, last row)`) are skipped.
fn paint_atom(dots: &mut Dots, cx: f64, cy: f64, r: f64, ink: Ink, panel: Option<(i32, i32, i32)>) {
    if !(cx.is_finite() && cy.is_finite() && r.is_finite()) {
        return;
    }
    let rim = Ink::new(ink.role, (f64::from(ink.level) * RIM).round() as u8);
    let hidden = |x: i32, y: i32| {
        panel.is_some_and(|(x0, x1, rows)| {
            (x0..=x1).contains(&x.div_euclid(Dots::CELL_W)) && y.div_euclid(Dots::CELL_H) <= rows
        })
    };
    let (x0, x1) = ((cx - r).ceil() as i32, (cx + r).floor() as i32);
    let (y0, y1) = ((cy - r).ceil() as i32, (cy + r).floor() as i32);
    let (r2, core2) = (r * r, CORE * CORE * r * r);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d2 = (f64::from(x) - cx).powi(2) + (f64::from(y) - cy).powi(2);
            if d2 <= r2 && !hidden(x, y) {
                dots.set(x, y, if d2 <= core2 { ink } else { rim });
            }
        }
    }
    let (x, y) = (cx.round() as i32, cy.round() as i32);
    if !hidden(x, y) {
        dots.set(x, y, ink);
    }
}

impl Generator for MolecularDynamics {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        let label_row = (h >= 8).then_some(h - 1);
        // The box fills the canvas above the title row; its walls are the screen edges.
        let (iw, ih) = (w, if label_row.is_some() { h - 1 } else { h });
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
        let (icx, icy) = (iw1 / 2.0, ih1 / 2.0);
        let inset = (size.width() >= INSET_MIN.0 && size.height() >= INSET_MIN.1)
            .then_some((w - INSET_W - 1, 0));
        // The start crystal sits below the inset (its bottom margin row, in σ).
        let r0 = icy - ly / 2.0 * scale;
        let top = inset.map_or(0.0, |_| (f64::from(INSET_H + 1) - r0) / scale);
        self.view = View {
            scale,
            c0: icx - lx / 2.0 * scale * ASPECT,
            r0,
            label_row,
            inset,
        };
        let box_cells = Size::new(size.width(), ih.max(1) as u16);
        // Pixel resolution: `w × 2·h` pixels are the dots of `⌈w/2⌉ × ⌈h/2⌉` cells.
        self.glow.resize(Size::new(
            box_cells.width().div_ceil(2),
            box_cells.height().div_ceil(2),
        ));
        self.pixels.resize(box_cells);
        self.atoms.resize(box_cells);
        self.plot.resize(Size::new(size.width(), INSET_H as u16));
        self.tone = Tone::asinh(TRAIL_KNEE, TRAIL_WHITE);

        self.t = 0.0;
        self.place_atoms(n, top);
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
        // Every atom starts at the mean kinetic energy per atom (2-D equipartition:
        // ⟨v²/2⟩ = T), not its own instantaneous one: that is exponentially distributed, and
        // the opening crystal would read as the speckle HEAT_TAU exists to smooth away.
        self.heat.clear();
        self.heat.resize(n, self.temperature());
        self.sample_speeds(1.0);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 || self.n() == 0 {
            return;
        }
        let span = dt * TIME_SCALE;
        let steps = (span / DT_MD_MAX).ceil().max(1.0);
        let h = span / steps;
        self.glow.decay((-dt / TRAIL_TAU).exp() as f32);
        for _ in 0..steps as usize {
            self.verlet(h);
            self.deposit_streaks(h);
        }
        self.t += dt;
        if !self.thermostat_off {
            let lambda =
                berendsen_lambda(self.temperature(), target_temperature(self.t), dt, TAU_T);
            self.scale_velocities(lambda);
        }
        self.refresh_readout();
        self.sample_heat(1.0 - (-dt / HEAT_TAU).exp());
        self.sample_speeds(1.0 - (-dt / HIST_TAU).exp());
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        // Streaks: the brighter half of each cell; the atoms' beads go on top, with the
        // streak glow kept behind them.
        self.pixels.blit(canvas, |_, _, top, bottom| {
            let ink = if top.level >= bottom.level {
                top
            } else {
                bottom
            };
            matches!(ink.role, Role::Ramp(_)).then_some(('.', ink))
        });
        self.atoms.blit_over(canvas, &self.pixels);
        if let Some((x0, y0)) = self.view.inset {
            self.plot.blit(canvas);
            let key = heat_ink(BARS_KEY);
            canvas.text(x0, y0, "speeds", key);
            let mb = "Maxwell-Boltzmann";
            canvas.text(x0 + INSET_W - mb.len() as i32, y0, mb, CURVE_INK);
        }
        if let Some(row) = self.view.label_row {
            self.draw_readout(canvas, row);
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(if target_temperature(self.t) < T_C_2D {
            CAPTIONS[0]
        } else {
            CAPTIONS[1]
        })
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
        // Heating: 1 s before the peak (target ≈ 1.0), system at T = 0.3.
        let (mut g, mut ctx) = fresh(100, 30, 4);
        g.t = 14.0;
        g.set_temperature(0.3);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        let (t1, target) = (g.temperature(), target_temperature(g.t));
        assert!(t1 > 0.6, "heated to {t1}");
        assert!((t1 - target).abs() < (0.3 - target).abs());

        // Cooling: near the end of a cycle (target ≈ 0.25), system at T = 1.6, a deliberate
        // overshoot above the 1.0 peak so the pull is large.
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
    fn md_a4_temperature_cycle_spans_the_spec_range() {
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
    fn md_a5_atom_count_follows_the_area() {
        // Small screens: σ sits at its legibility floor and N = ρ·area/σ² — 205 at 80x24.
        let (g, _) = fresh(80, 24, 1);
        let a1 = 79.0 * 22.0 / ASPECT;
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
    fn md_a5_glyphs_stay_legible_at_contact() {
        // Triangular-lattice rows (the closest vertical packing, r_min·√3/2 σ apart) must land
        // at least one terminal row apart, or crystal rows alias onto each other.
        for (w, h) in [(1, 1), (20, 10), (80, 24), (100, 30), (200, 50), (400, 100)] {
            let (g, _) = fresh(w, h, 3);
            let s = g.view.scale;
            assert!(ROW_H * s >= 1.0 - 1e-12, "{w}x{h}: {s} rows/σ");
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
    fn md_renders_balls_streaks_inset_and_readout_in_ascii() {
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Ascii);
        for _ in 0..60 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let lines = frame.to_lines();
        assert!(lines.iter().all(|l| l.is_ascii()));
        assert!(lines[29].contains("T 0."), "{}", lines[29]);
        // Beads fall back to dot counts; streaks are `.`.
        assert!(frame.cells().iter().any(|c| c.ch == '#' || c.ch == '+'));
        assert!(lines[0].contains("speeds") && lines[0].contains("Maxwell-Boltzmann"));
    }

    #[test]
    fn md_a6_atoms_glow_cool_to_hot_by_kinetic_energy() {
        let mut prev = 0.0;
        for k in 0..=500 {
            let u = heat_u(f64::from(k) * 0.01);
            assert!(u >= prev, "e = {}", f64::from(k) * 0.01);
            prev = u;
        }
        assert_eq!(heat_u(0.0), 0.0);
        assert_eq!(heat_u(3.0), 1.0);
        assert!(heat_u(2.9) < 1.0);
        assert_eq!(heat_u(50.0), 1.0);
        for e in [f64::NAN, f64::NEG_INFINITY, -1.0] {
            assert_eq!(heat_u(e), 0.0, "{e}");
        }
        assert_eq!(heat_u(f64::INFINITY), 1.0);
        // The colour is the Plasma ramp at clamp(0.1 + 1.3·u, 0.35, 1): cool → hot is
        // deep violet → lavender → near white, never an amber or an Ice blue, and
        // non-decreasing in e. The floor keeps cold atoms visible, and the cold phase and
        // the hot gas sit far apart, so the hot/cold story reads.
        let level = |e: f64| {
            let ink = heat_ink(e);
            assert_eq!(ink.role, Role::Ramp(Ramp::Plasma));
            f64::from(ink.level) / 255.0
        };
        assert!((level(0.0) - 0.35).abs() < 0.005);
        assert!((level(0.25) - 0.40).abs() < 0.02, "{}", level(0.25));
        assert!((level(1.0) - 0.86).abs() < 0.02, "{}", level(1.0));
        assert!(level(1.0) - level(0.25) >= 0.4);
        assert!(level(3.0) > 0.99);
        let mut prev = 0.0;
        for k in 0..=300 {
            let l = level(f64::from(k) * 0.01);
            assert!(l >= prev);
            prev = l;
        }
        // Every atom dot is Ice (a bead), every lit pixel the Plasma streak glow.
        let (mut g, mut ctx) = fresh(200, 50, 3);
        g.t = 12.0;
        g.set_temperature(1.0);
        for _ in 0..30 {
            g.update(SIM_DT, &mut ctx);
        }
        let px = g.pixels.pixel_size();
        let mut streaks = 0;
        // No faint fringe: a streak pixel is at least its cut level (MD-A6).
        let cut = ink::ramp(Ramp::Plasma, TRAIL_MAX * f64::from(TRAIL_CUT) / 255.0).level;
        for y in 0..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let p = g.pixels.get(x, y).unwrap();
                match p.role {
                    Role::Ramp(Ramp::Plasma) => {
                        assert!(p.level >= cut, "faint streak pixel {}", p.level);
                        streaks += 1;
                    }
                    Role::Background => {}
                    r => panic!("unexpected {r:?}"),
                }
            }
        }
        let dots = g.atoms.dot_size();
        let lit = (0..i32::from(dots.height()))
            .flat_map(|y| (0..i32::from(dots.width())).map(move |x| (x, y)))
            .filter(|&(x, y)| g.atoms.is_lit(x, y))
            .count();
        assert!(lit > 4 * g.n() && streaks > 0, "{lit} {streaks}");
    }

    #[test]
    fn md_a6_atoms_are_round_beads_with_dark_seams() {
        // A bead is a disc of radius ATOM_R σ in square dots: at 200x50 its lit dots are
        // within 25 % of πr², and two atoms at the LJ minimum distance never touch.
        let (g, _) = fresh(200, 50, 1);
        let r = ATOM_R * f64::from(Dots::CELL_H) * g.view.scale;
        let mut dots = Dots::new(Size::new(40, 20));
        paint_atom(&mut dots, 40.0, 40.0, r, heat_ink(0.25), None);
        let lit = (0..80)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .filter(|&(x, y)| dots.is_lit(x, y))
            .count() as f64;
        let area = std::f64::consts::PI * r * r;
        assert!((lit / area - 1.0).abs() < 0.25, "{lit} vs {area:.1}");
        const { assert!(2.0 * ATOM_R < R_MIN, "beads overlap at contact") };
    }

    #[test]
    fn md_a7_speed_histogram_matches_maxwell_boltzmann() {
        for t in [0.25, 0.5, 1.0] {
            let sum: f64 = mb_bins(t).iter().sum();
            assert!((sum - 1.0).abs() < 1e-12, "T {t}: {sum}");
        }
        let (mut g, mut ctx) = fresh(200, 50, 7);
        g.t = 12.0; // near the hot peak, where the target barely moves
        for _ in 0..(6.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let sum: f64 = g.hist.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "{sum}");
        let mb = mb_bins(g.temperature());
        let l1: f64 = g.hist.iter().zip(&mb).map(|(a, b)| (a - b).abs()).sum();
        assert!(l1 < 0.25, "L1 {l1:.3} at T {:.2}", g.temperature());
    }

    #[test]
    fn md_a8_box_keeps_off_the_title_row_and_the_inset_off_the_title() {
        for (w, h) in [
            (200, 50),
            (120, 40),
            (100, 30),
            (99, 40),
            (160, 29),
            (80, 24),
            (40, 12),
            (30, 7),
        ] {
            let size = Size::new(w, h);
            let (mut g, mut ctx) = fresh(w, h, 5);
            for _ in 0..30 {
                g.update(0.05, &mut ctx);
            }
            let mut c = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut c);
            let last = i32::from(h) - 1;
            if h >= 8 {
                // Only the readout (Text) may sit on the title row.
                for x in 0..i32::from(w) {
                    let cell = c.get(x, last).unwrap();
                    assert!(
                        matches!(cell.ink.role, Role::Text | Role::Background),
                        "{w}x{h}: {x} {cell:?}"
                    );
                }
            }
            let big = w >= INSET_MIN.0 && h >= INSET_MIN.1;
            assert_eq!(g.view.inset.is_some(), big, "{w}x{h}");
            if let Some((x0, y0)) = g.view.inset {
                let span = title::span(size, &Title::of(&g)).map_or(0, |(_, c)| c.end);
                assert!(x0 >= span && y0 + INSET_H < last, "{w}x{h}");
                assert!(x0 + INSET_W < i32::from(w), "{w}x{h}");
            }
        }
    }

    #[test]
    fn md_a8_atoms_start_clear_of_the_inset_and_pass_behind_it() {
        for (w, h) in [(120, 40), (100, 30), (200, 50)] {
            let (mut g, mut ctx) = fresh(w, h, 5);
            let (x0, _) = g.view.inset.expect("inset");
            // The panel and its one-cell margin, in cells.
            let inside = |c: i32, r: i32| (x0 - 1..x0 + INSET_W + 1).contains(&c) && r <= INSET_H;
            for i in 0..g.n() {
                let (px, py) = g.to_pixel(g.x[i], g.y[i]);
                let (c, r) = (px.floor() as i32, (py / 2.0).floor() as i32);
                assert!(!inside(c, r), "{w}x{h}: atom {i} starts at ({c}, {r})");
            }
            // Whatever the atoms do, no bead dot or streak pixel is drawn in the panel.
            let mut t = 0.0;
            for until in [0.0, 5.0, 60.0] {
                while t < until {
                    g.update(0.05, &mut ctx);
                    t += 0.05;
                }
                for y in 0..2 * (INSET_H + 1) {
                    for x in x0 - 1..x0 + INSET_W + 1 {
                        assert_eq!(g.pixels.get(x, y), Some(Ink::BLANK), "{w}x{h} t {t}");
                    }
                }
                for cy in 0..=INSET_H {
                    for cx in x0 - 1..=x0 + INSET_W {
                        assert_eq!(g.atoms.cell_mask(cx, cy), Some(0), "{w}x{h} t {t}");
                    }
                }
            }
        }
    }

    #[test]
    fn md_a9_atoms_are_coloured_relative_to_the_live_temperature() {
        let lv = |e: f64, t: f64| f64::from(relative_heat_ink(e, t).level) / 255.0;
        assert_eq!(relative_heat_ink(1.0, 1.0).role, Role::Ramp(Ramp::Plasma));
        assert!((lv(0.0, 0.3) - REL_FLOOR).abs() < 0.005);
        assert!((lv(0.3, 0.3) - (REL_FLOOR + (1.0 - REL_FLOOR) / REL_SPAN)).abs() < 0.005);
        assert!(lv(0.3 * REL_SPAN, 0.3) > 0.99 && lv(5.0, 0.3) > 0.99);
        // Scale-free: the same multiple of the temperature is the same colour.
        for (e, t) in [(0.1, 0.25), (0.5, 1.0), (1.3, 0.7)] {
            assert_eq!(relative_heat_ink(e, t), relative_heat_ink(2.0 * e, 2.0 * t));
        }
        for t in [0.0, -1.0, f64::NAN] {
            assert!((lv(1.0, t) - REL_FLOOR).abs() < 0.005, "{t}");
        }
        // In the condensed phase (t = 5 s, T ≈ 0.3) the bulk stays calm and the hottest
        // atoms glow: the brightest 3 % reach ≥ 0.8 while the median is ≤ 0.72.
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = MolecularDynamics::default();
        g.resize(size, &mut ctx);
        for _ in 0..(5.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let t = g.temperature();
        assert!(t < 0.5, "condensed: T {t}");
        let mut levels: Vec<f64> = g.heat.iter().map(|&e| lv(e, t)).collect();
        levels.sort_by(f64::total_cmp);
        let n = levels.len();
        let (median, top) = (levels[n / 2], levels[n - 1 - n * 3 / 100]);
        assert!(
            median <= 0.72 && top >= 0.8,
            "median {median:.2}, top 3 % {top:.2}"
        );
        assert!(levels[0] >= REL_FLOOR - 0.005);
    }

    #[test]
    fn md_a6_the_opening_crystal_is_one_colour() {
        // Heat starts at the mean kinetic energy, not each atom's exponentially
        // distributed instantaneous one, so t = 0 shows no speckle.
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let (g, _) = fresh(w, h, 42);
            let levels = g.heat.iter().map(|&e| 255.0 * heat_u(e));
            let (lo, hi) = levels.fold((f64::MAX, f64::MIN), |(a, b), l| (a.min(l), b.max(l)));
            assert!(hi - lo <= 10.0, "{w}x{h}: levels {lo:.0}..{hi:.0}");
        }
    }

    #[test]
    fn md_a5_start_crystal_rows_land_on_evenly_spaced_pixel_rows() {
        // At the legibility floor the crystal's rows are two pixels apart exactly, so the
        // single-pixel atoms of 80x24 show no blank pixel row through the crystal.
        let (g, _) = fresh(80, 24, 42);
        assert_eq!(g.view.scale, S_MIN);
        let mut rows: Vec<i32> = (0..g.n())
            .map(|i| g.to_pixel(g.x[i], g.y[i]).1.floor() as i32)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        assert!(rows.len() > 5, "{rows:?}");
        assert!(rows.windows(2).all(|p| p[1] - p[0] == 2), "{rows:?}");
    }

    #[test]
    fn md_a6_atoms_outshine_their_streaks() {
        // Luminance order holds in every theme (Mono maps both roles by level): an atom
        // hot enough to streak is brighter than the brightest streak, even at its rim.
        // Any atom is at least the relative colour's floor (MD-A9).
        let atom = f64::from(relative_heat_ink(0.0, 1.0).level);
        assert!(TRAIL_MAX * 255.0 < RIM * atom, "{atom}");
    }

    #[test]
    fn md_a9_caption_follows_the_target_across_t_c() {
        let (mut g, _) = fresh(80, 24, 1);
        for k in 0..600 {
            g.t = f64::from(k) * 0.05;
            let want = if target_temperature(g.t) < T_C_2D {
                "cool atoms cling"
            } else {
                "hot atoms scatter"
            };
            assert_eq!(g.caption(), Some(want), "t {}", g.t);
        }
        assert_eq!(META.captions, &CAPTIONS);
    }

    #[test]
    fn md_gen_r13_readout_is_right_aligned_and_shortened_clear_of_the_title() {
        let readout = |w: u16| {
            let size = Size::new(w, 24);
            let (g, _) = fresh(w, 24, 42);
            let mut c = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut c);
            let row = c.to_lines().pop().unwrap();
            let span = title::span(size, &Title::of(&g));
            // Four blanks after the title text, which ends at `cols.end - 2`.
            let free = span.map_or(0, |(_, cols)| cols.end as usize + 2);
            let lead = row.len() - row.trim_start().len();
            assert!(lead > free || row.trim().is_empty(), "{w}: {row:?}");
            assert!(row.ends_with(' '), "{w}: {row:?}");
            row.trim().to_owned()
        };
        let full = readout(200);
        assert!(
            full.starts_with("T 0.40 -> ") && full.ends_with("  LJ units"),
            "{full}"
        );
        // Beside the full title (32 cells) the room is w − 38, at most w/2.
        assert!(readout(100).ends_with("  LJ units"));
        let r = readout(80);
        assert!(
            r.contains("U/N ") && r.contains("  N ") && !r.contains("LJ"),
            "{r}"
        );
        let r = readout(60);
        assert!(r.starts_with("T 0.40 -> ") && !r.contains("U/N"), "{r}");
        // Beside "Condensation" alone (w < 57) the room is w − 18.
        assert_eq!(readout(30), "T 0.40");
    }
}
