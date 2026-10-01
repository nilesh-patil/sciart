//! A von Kármán vortex street shed from a cylinder at laminar Re ≈ 90–150 (SPEC-002
//! `vortex_street`).
//!
//! ## Model
//!
//! D2Q9 lattice Boltzmann (Qian, d'Humières & Lallemand 1992) in lattice units, with the
//! two-relaxation-time collision (TRT; Ginzburg, Verhaeghe & d'Humières 2008):
//!
//! ```text
//! f_i* = f_i − ω⁺ (f_i⁺ − f_i^{eq+}) − ω⁻ (f_i⁻ − f_i^{eq−}),   f_i^± = (f_i ± f_ī) / 2
//! ω⁺ = 1/τ  (ν = (τ − ½)/3),   ω⁻ = 1/τ⁻  with  Λ = (τ − ½)(τ⁻ − ½) = ¼
//! ```
//!
//! TRT has BGK's equilibrium and ν(τ). BGK is TRT with `Λ = (τ − ½)² ≈ 9·10⁻⁴` at
//! τ = 0.53; the stable velocity range collapses as Λ → 0 and BGK blows up in the
//! under-resolved shear layers, whereas Λ = ¼ maximises it.
//!
//! The inlet speed breathes slowly, `U(t) = U₀ (1 + 0.08 sin(2π t / 47 s))` in the
//! scene's own simulated time, so the shedding frequency and eddy spacing drift over a
//! long visit instead of repeating one exact ≈ 1.6 s shedding cycle; Re stays laminar.
//!
//! The kernel is a fused pull-scheme stream + collide in `f32` over SoA planes, storing
//! `f_i − w_i` (Lehmann et al. 2022) so the O(Ma²) deviations keep their precision. It
//! pulls post-collision populations from the upwind node; if that node is solid it takes
//! the opposite population of the node itself (halfway bounce-back, Ladd 1994). Rows away
//! from the cylinder run a branch-free sliced loop; only the cylinder's box, the inlet,
//! outlet and sponge columns take special paths.
//!
//! ## Frames
//!
//! Lattice rows grow downward, like the screen, and all velocities are stored in that
//! frame. The analytic flows ([`potential_flow`], [`scully`]) and the vorticity
//! ([`vorticity`]) use the mathematical frame with y up, so "clockwise" means what it
//! looks like on screen; conversions happen at the call sites.

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::grid;
use sciart_core::prelude::*;
use sciart_core::{draw, ease, ink};

pub static META: Meta = Meta {
    name: "vortex_street",
    category: Category::Physics,
    description: "A von Karman vortex street shed from a cylinder at Re ~ 130",
    title: "Vortex street · eddies peel off",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(VortexStreet::default())
}

/// D2Q9 velocities in the lattice (rows-down) frame: rest, E, S, W, N, SE, SW, NW, NE.
const CX: [i32; 9] = [0, 1, 0, -1, 0, 1, -1, -1, 1];
const CY: [i32; 9] = [0, 0, 1, 0, -1, 1, 1, -1, -1];
/// Index of the opposite velocity, `c_ī = −c_i`.
const OPP: [usize; 9] = [0, 3, 4, 1, 2, 7, 8, 5, 6];
/// The four opposite pairs that TRT splits into even and odd parts.
const PAIRS: [(usize, usize); 4] = [(1, 3), (2, 4), (5, 7), (6, 8)];
const W: [f32; 9] = [
    4.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
];

/// Relaxation time: ν = (τ − ½)/3 = 0.01 lattice units.
const TAU: f64 = 0.53;
/// TRT magic parameter Λ = ¼: the most stable choice (Ginzburg et al. 2008); Λ = 3/16
/// would instead put bounce-back walls exactly halfway for Poiseuille flow.
const MAGIC: f64 = 0.25;
/// Inlet breathing: relative amplitude and period (simulated seconds).
const BREATH_AMP: f64 = 0.08;
const BREATH_PERIOD: f64 = 47.0;
/// Lattice steps run inside the first `resize` after the analytic start, so the first
/// frame is past the slip sheet on the staircase wall, the start-up pressure pulse and
/// the inlet's adjustment, which clear after ≈ 110–160 steps; 160 ≈ x_c/U, one flush of
/// the run-up. ≈ 1.6 M lattice updates: a one-off ≈ 12–20 ms on a release scene switch.
const PREROLL_STEPS: usize = 160;
/// Minimum distance of the cylinder centre from the inlet, in diameters: on square
/// displays 15 % of the width would put the front only ≈ 4 nodes past the inlet sponge,
/// and the calm inflow would fill with the inlet's disturbances.
const RUN_UP_D: f64 = 1.65;
/// Mean inflow speed U₀ on every lattice: inflow Ma = U₀√3 = 0.217 (0.234 at the breath's
/// peak). The flow is weakly compressible: the shoulders reach |u| ≈ 1.7 U (local
/// Ma ≈ 0.4) and ρ swings by ±6 %, yet the shedding frequency matches the literature
/// (VOR-A5). Re = U₀D/ν = 12.5 D ≤ 150, because the node cap keeps D ≤ 12 (D ≥ 13 would
/// need H ≥ 90 and W ≥ 125).
const U0: f64 = 0.125;
/// Lattice node budget and aspect bucket (ADR-0012): every display in one bucket runs
/// identical physics, and the cost does not grow with the terminal.
const MAX_NODES: usize = 10_240;
const BUCKET: f64 = 1.12;
/// Hidden sponge fraction on the right; the domain is widened by `1/(1 − SPONGE)`.
const SPONGE: f64 = 0.1;
/// Peak sponge relaxation rate toward the inflow equilibrium (Israeli & Orszag 1981).
const SPONGE_MAX: f32 = 0.25;
/// Columns of a short sponge behind the inlet. At τ = 0.53 the even non-equilibrium
/// part flips sign every step (1 − ω⁺ ≈ −0.89), and the forced inlet column feeds it:
/// without damping it grows into a checkerboard at the inlet within a few hundred
/// steps. Three columns of ramped extra relaxation remove it.
const INLET_SPONGE: usize = 3;
/// First visible lattice column: column 0 is the forced inlet.
const VIEW_X0: usize = 1;
/// Lattice steps per simulated second (4.5 per 60 Hz frame) and the cap per `update`.
const STEPS_PER_S: f64 = 270.0;
const MAX_STEPS: usize = 68;
/// Nodes collided together in the kernel's inner loop: one 128-bit SIMD register of f32
/// (NEON, SSE2). Measured 7 ns/node against 29 ns unbatched; 8 and 16 lanes spill.
const LANES: usize = 4;
/// Watchdog cadence and the density band it accepts (simulated seconds, lattice units).
const CHECK_EVERY: f64 = 1.0;
const RHO_OK: (f32, f32) = (0.7, 1.3);

/// Analytic start: circulation (units of U·D), core radius and street geometry (units
/// of D), von Kármán's stable row-gap ratio b/a, and where the rows begin.
const GAMMA_UD: f64 = 1.8;
const CORE_D: f64 = 0.35;
const SPACING_D: f64 = 4.3;
const GAP_RATIO: f64 = 0.281;
const STREET_START_D: f64 = 1.5;
/// Start-up pair (see [`init_street`]): circulation in units of U·D, and the eddy's
/// offset behind the centre and toward the leading row, in units of D.
const START_PAIR_UD: f64 = 1.5;
const START_EDDY_D: (f64, f64) = (0.9, 0.3);
/// The start-up pair is whole from this many diameters in front of the centre on and
/// fades toward the inlet ahead of it (see [`pair_window`]).
const PAIR_FADE_D: f64 = 1.0;

/// Vorticity display: `x = ω·D/(VORT_SCALE·U)` is soft-clipped to `s`.
const VORT_SCALE: f64 = 2.5;
/// M16 (GEN-R25): contour levels of |s|, outermost first, and their ink levels (brighter
/// inward); positive (counter-clockwise) `Hot`, negative (clockwise) `Cold`.
pub(crate) const LEVELS: [f32; 3] = [0.15, 0.4, 0.75];
pub(crate) const LEVEL_INKS: [u8; 3] = [170, 215, 255];
/// Sub-cell refinement: a crossed lattice cell is split into `k × k` bilinear sub-cells,
/// `k = ⌈max(sx, sy)/REFINE⌉` (dots per node), at most `MAX_REFINE`.
const REFINE: f64 = 2.2;
const MAX_REFINE: usize = 5;
/// The cylinder: a one-dot ring (M16 round 2), drawn last at the top level so its cells
/// stay grey, as `CYLINDER_PTS` chords.
const CYLINDER_INK: Ink = Ink::new(Role::Structure, 255);
const CYLINDER_PTS: usize = 48;
/// Upstream of the cylinder (the half facing the inlet), weak vorticity (|s| below
/// `UPSTREAM_MIN`) is staircase noise, not flow: within `UPSTREAM_ANNULUS` nodes of the
/// rim, where the steps' corners stick out of the round mask, and, when its sign is
/// opposite to the boundary layer's (CW above, CCW below), within `UPSTREAM_OPPOSITE_D`
/// diameters of it, where the steps' odd-even noise is washed along the wall. At Re ≲ 190
/// the only vorticity there is the attached boundary layer, which is far stronger;
/// secondary, opposite-signed vorticity only forms behind separation, on the lee side.
const UPSTREAM_ANNULUS: f64 = 1.0;
const UPSTREAM_OPPOSITE_D: f64 = 0.6;
/// M16: 0.3 (was 0.2 for the bilinear pixels): the contours are drawn from the nodes
/// themselves, where start-up transients on the shoulders reach |s| ≈ 0.24.
const UPSTREAM_MIN: f64 = 0.3;
const LABEL_INK: Ink = Ink::new(Role::Text, 150);

/// Strouhal probe: `PROBE_D` diameters behind the centre, crossings with ±`HYST`·U
/// hysteresis, St from the last ≤ `ST_INTERVALS` periods once `ST_MIN_CROSSINGS` exist.
const PROBE_D: f64 = 2.5;
const HYST: f64 = 0.02;
const ST_INTERVALS: usize = 4;
const ST_MIN_CROSSINGS: usize = 3;

/// Boundary handling of [`step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bc {
    /// Periodic in both axes (tests).
    Periodic,
    /// The scene's wind tunnel: equilibrium inlet at column 0, zero-gradient outlet at
    /// the last column, sponge before it, periodic in y.
    Channel,
}

/// Size-dependent flow parameters, pure in the lattice size (VOR-A4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlowParams {
    pub w: usize,
    pub h: usize,
    /// Columns before the hidden sponge (which starts at `w_vis`).
    pub w_vis: usize,
    pub d: f64,
    pub xc: f64,
    pub yc: f64,
    /// Mean inlet speed U₀ (the inlet breathes around it) and the Re it gives.
    pub u: f64,
    pub omega_p: f32,
    pub omega_m: f32,
    pub re: f64,
}

/// `τ⁻` from the magic parameter `Λ = (τ − ½)(τ⁻ − ½)` (VOR-A1).
pub(crate) fn tau_minus(tau: f64, magic: f64) -> f64 {
    0.5 + magic / (tau - 0.5)
}

/// The simulation lattice for a display (ADR-0012, VOR-A4): the display's square-pixel
/// aspect `w/2h`, clamped to [1, 4] and widened by the hidden sponge, is snapped to its
/// power of [`BUCKET`], and the capped lattice of exactly that aspect is used whatever
/// the display's size (CORE-R69), so the result depends only on the bucket.
pub(crate) fn lattice_size(display: Size) -> Size {
    grid::lattice_for_aspect(BUCKET.powi(bucket_index(display)), MAX_NODES)
}

/// The power of [`BUCKET`] nearest to the display's clamped, sponge-widened aspect.
fn bucket_index(display: Size) -> i32 {
    let (w, h) = (f64::from(display.width()), f64::from(display.height()));
    let aspect = (w / (2.0 * h)).clamp(1.0, 4.0) / (1.0 - SPONGE);
    (aspect.ln() / BUCKET.ln()).round() as i32
}

/// Geometry and dynamics for a lattice (VOR-A4): `D = clamp(round(min(0.14 H, W_vis/9)),
/// 6, 16)` (blockage ≤ 0.14) leaves room for 3–4 shed eddies in view even on square
/// displays, and `U₀ = 0.125` gives Re = 12.5 D ∈ [87.5, 150] (see [`U0`]). The centre sits
/// 15 % of the way across but never closer than `round(1.65 D)` to the inlet, so the
/// cylinder's front keeps a calm run-up of ≈ 1.15 D past the inlet sponge on every
/// display. The centre row `(H − 1)/2` is the domain's exact middle, so the staircase
/// mask and the drawn disc are both symmetric about it.
pub(crate) fn flow_params(lattice: Size) -> FlowParams {
    let (w, h) = (usize::from(lattice.width()), usize::from(lattice.height()));
    let sponge = (SPONGE * w as f64).ceil() as usize;
    let w_vis = w.saturating_sub(sponge).max(VIEW_X0 + 1);
    let d = (0.14 * h as f64)
        .min(w_vis as f64 / 9.0)
        .round()
        .clamp(6.0, 16.0);
    let u = U0;
    let nu = (TAU - 0.5) / 3.0;
    // `TAU − 0.5` is 0.030000000000000027 in f64, so U·D/ν lands a hair below exact
    // half-values (137.4999…); rounding to 1e-6 restores them, and the label's
    // round-to-5 then reads Re≈140 at t = 0 rather than flipping 135 → 140 as the
    // breath starts to rise.
    let re = (u * d / nu * 1e6).round() / 1e6;
    FlowParams {
        w,
        h,
        w_vis,
        d,
        xc: (0.15 * w_vis as f64).round().max((RUN_UP_D * d).round()),
        yc: (h as f64 - 1.0).max(0.0) / 2.0,
        u,
        omega_p: (1.0 / TAU) as f32,
        omega_m: (1.0 / tau_minus(TAU, MAGIC)) as f32,
        re,
    }
}

/// Shifted equilibrium `f_i^eq − w_i = w_i (δρ + ρ(3 c·u + 4.5 (c·u)² − 1.5 u²))` with
/// `ρ = 1 + δρ` (Lehmann et al. 2022): the O(Ma²) deviations keep full `f32` precision.
#[inline(always)]
pub(crate) fn heq(drho: f32, ux: f32, uy: f32) -> [f32; 9] {
    let rho = 1.0 + drho;
    let usq = 1.5 * (ux * ux + uy * uy);
    std::array::from_fn(|i| {
        let cu = CX[i] as f32 * ux + CY[i] as f32 * uy;
        W[i] * (drho + rho * (3.0 * cu + 4.5 * cu * cu - usq))
    })
}

/// TRT collision of one node's shifted populations in place; returns the node's
/// `(ρ, uₓ, u_y)`, which the collision conserves (VOR-A1).
#[inline(always)]
pub(crate) fn collide(h: &mut [f32; 9], omega_p: f32, omega_m: f32) -> (f32, f32, f32) {
    let drho = ((h[0] + h[1]) + (h[2] + h[3])) + ((h[4] + h[5]) + (h[6] + h[7])) + h[8];
    let rho = 1.0 + drho;
    let jx = (h[1] - h[3]) + (h[5] - h[7]) + (h[8] - h[6]);
    let jy = (h[2] - h[4]) + (h[5] - h[7]) + (h[6] - h[8]);
    let (ux, uy) = (jx / rho, jy / rho);
    let eq = heq(drho, ux, uy);
    h[0] -= omega_p * (h[0] - eq[0]);
    for (i, j) in PAIRS {
        let (ni, nj) = (h[i] - eq[i], h[j] - eq[j]);
        let (even, odd) = (0.5 * (ni + nj), 0.5 * (ni - nj));
        h[i] -= omega_p * even + omega_m * odd;
        h[j] -= omega_p * even - omega_m * odd;
    }
    (rho, ux, uy)
}

/// Irrotational flow past a cylinder of radius `r` in a uniform stream `u` (y up):
/// `u = U(1 − R²(x² − y²)/r⁴, −2R²xy/r⁴)`. Rational, so no libm and no jolt at the wall.
pub(crate) fn potential_flow(x: f64, y: f64, r: f64, u: f64) -> (f64, f64) {
    let r2 = x * x + y * y;
    if r2 <= r * r {
        return (0.0, 0.0);
    }
    let k = r * r / (r2 * r2);
    (u * (1.0 - k * (x * x - y * y)), -2.0 * u * k * x * y)
}

/// Kaufmann–Scully vortex of circulation `gamma` (> 0 counter-clockwise, y up) and core
/// radius `rc` at offset `(dx, dy)`: `v_θ = Γ r / (2π (r² + r_c²))`, rational in `r`.
pub(crate) fn scully(dx: f64, dy: f64, gamma: f64, rc: f64) -> (f64, f64) {
    let k = gamma / (2.0 * PI * (dx * dx + dy * dy + rc * rc));
    (-k * dy, k * dx)
}

/// Vorticity `ω = ∂ₓu_y − ∂_y uₓ` (y up) by central differences, periodic in y, for
/// lattice columns `x0 .. x0 + out.width()` from rows-down velocity grids. Columns whose
/// neighbours fall off the lattice are clamped to the edge.
pub(crate) fn vorticity(ux: &Grid<f32>, uy: &Grid<f32>, x0: usize, out: &mut Grid<f32>) {
    let (w, h) = (usize::from(ux.width()), usize::from(ux.height()));
    let ow = usize::from(out.width());
    if w == 0 || h == 0 || ow == 0 || uy.size() != ux.size() || usize::from(out.height()) != h {
        return;
    }
    let (ux, uy) = (ux.as_slice(), uy.as_slice());
    for (y, row) in out.as_mut_slice().chunks_exact_mut(ow).enumerate() {
        let (up, down) = ((y + h - 1) % h * w, (y + 1) % h * w);
        for (k, o) in row.iter_mut().enumerate() {
            let x = (x0 + k).min(w - 1);
            let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let span = (xr - xl) as f32;
            // Rows grow downward: u_y(up) = −u_y(row) and ∂_y(up) = −∂_row.
            let duy_dx = -(uy[y * w + xr] - uy[y * w + xl]) / span;
            let dux_dy = -(ux[down + x] - ux[up + x]) / 2.0;
            *o = duy_dx - dux_dy;
        }
    }
}

/// The Strouhal number `D/(U·P̄)` from the lattice steps of the last upward crossings
/// (oldest first), with P̄ their mean period and U = `u_at(step)` the inlet speed at the
/// middle of that window; `None` before [`ST_MIN_CROSSINGS`] crossings. The breathing
/// inlet drifts by up to ≈ 3 % over the ≈ 6 s window, so dividing by the speed at its
/// latest crossing would bias St by as much; the middle matches the averaged period.
///
/// St is based on the free-stream U, the same basis as Re. With y periodic at blockage
/// D/H ≈ 0.15 the fluid passes a row of cylinders and the gap speed is
/// U·H/(H − D) ≈ 1.17 U, so this St reads ≈ 0.20–0.21 at the reference lattices. The
/// label shows St on the gap speed, St·(1 − D/H): a heuristic confinement correction,
/// not an established blockage correction, which on these lattices matches Williamson's
/// unconfined St(Re) to ≈ 1 % at Re 75 and ≈ 3 % at Re 125–140.
pub(crate) fn strouhal(crossings: &Trail<u64>, d: f64, u_at: impl Fn(f64) -> f64) -> Option<f64> {
    if crossings.len() < ST_MIN_CROSSINGS {
        return None;
    }
    let (first, last) = (*crossings.iter().next()?, crossings.newest()?);
    let period = (last - first) as f64 / (crossings.len() - 1) as f64;
    let mid = 0.5 * (first + last) as f64;
    (period > 0.0).then(|| d / (u_at(mid) * period))
}

/// Populations, moments and the solid mask of one lattice.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lattice {
    w: usize,
    h: usize,
    /// Nine SoA planes of `w·h` shifted post-collision populations `f_i − w_i`.
    f: Vec<f32>,
    /// The other half of the pull scheme's ping-pong pair.
    next: Vec<f32>,
    rho: Grid<f32>,
    ux: Grid<f32>,
    uy: Grid<f32>,
    solid: Vec<bool>,
    /// Rows that touch a solid node (or neighbour one) and the column range around the
    /// solids: only there does the kernel check the mask.
    near_solid: Vec<bool>,
    solid_cols: (usize, usize),
    /// Per-row inlet populations for [`Bc::Channel`] (empty for other lattices), and the
    /// uniform stream the outlet sponge relaxes toward, both at the current inlet speed.
    inflow: Vec<[f32; 9]>,
    outflow: [f32; 9],
    /// Outlet sponge strength of each column from `w_vis` to `w − 2` (empty for other
    /// lattices), computed once so the hot loop only reads it.
    sponge: Vec<f32>,
}

impl Lattice {
    /// A `size` lattice at rest (`f = w`, ρ = 1, u = 0) with `solid(x, y)` nodes.
    pub(crate) fn new(size: Size, solid: impl Fn(usize, usize) -> bool) -> Self {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let n = w * h;
        let mask: Vec<bool> = (0..n).map(|i| solid(i % w, i / w)).collect();
        let mut near_solid = vec![false; h];
        let (mut x0, mut x1) = (w, 0);
        for (i, _) in mask.iter().enumerate().filter(|(_, s)| **s) {
            let (x, y) = (i % w, i / w);
            for dy in [h - 1, 0, 1] {
                near_solid[(y + dy) % h] = true;
            }
            x0 = x0.min(x.saturating_sub(1));
            x1 = x1.max((x + 2).min(w));
        }
        Self {
            w,
            h,
            f: vec![0.0; 9 * n],
            next: vec![0.0; 9 * n],
            rho: Grid::new(size, 1.0),
            ux: Grid::new(size, 0.0),
            uy: Grid::new(size, 0.0),
            solid: mask,
            near_solid,
            solid_cols: (x0, x1.max(x0)),
            inflow: Vec::new(),
            outflow: [0.0; 9],
            sponge: Vec::new(),
        }
    }

    /// The scene's wind tunnel for `p`: the cylinder mask and, as the inlet, the
    /// equilibrium of the undisturbed [`base_flow`] at column 0. Imposing the potential
    /// flow there rather than a uniform stream matters because the inlet is only ≈ 2 D
    /// upstream of the cylinder: a uniform inlet would shear the deflected stream by a
    /// few % of U in one column and shed a visible vorticity sheet into the calm inflow.
    /// The inlet runs at `scale · U₀`.
    pub(crate) fn channel(p: &FlowParams, scale: f64) -> Self {
        let size = Size::new(p.w as u16, p.h as u16);
        let mut lat = Self::new(size, |x, y| in_cylinder(p, x, y));
        lat.inflow = vec![[0.0; 9]; p.h];
        lat.set_inlet(p, scale);
        let x0 = p.w_vis.min(p.w.saturating_sub(1));
        let span = p.w.saturating_sub(1 + x0).max(1) as f32;
        lat.sponge = (x0..p.w.saturating_sub(1))
            .map(|x| ramp((x - x0) as f32 / span))
            .collect();
        lat
    }

    /// Set the inlet and the outlet sponge's target to the stream at `scale · U₀`. The
    /// potential flow is linear in U, so the whole inlet profile scales. The inlet carries
    /// the flow's Bernoulli density `1 + 1.5 (U² − |u|²)`, as the start state does
    /// ([`init_street`]), so it forces no pressure mismatch at column 0.
    fn set_inlet(&mut self, p: &FlowParams, scale: f64) {
        let free = scale * p.u;
        for (y, row) in self.inflow.iter_mut().enumerate() {
            let (u, v) = base_flow(p, -p.xc, p.yc - y as f64);
            let (su, sv) = (scale * u, scale * v);
            let drho = 1.5 * (free * free - (su * su + sv * sv));
            *row = heq(drho as f32, su as f32, -sv as f32);
        }
        self.outflow = heq(0.0, (scale * p.u) as f32, 0.0);
    }

    #[cfg(test)]
    fn size(&self) -> Size {
        Size::new(self.w as u16, self.h as u16)
    }

    /// Rebuild every population as the equilibrium of the stored moments (solid nodes
    /// at rest). Used by the analytic start and after resampling on resize.
    fn equilibrate(&mut self) {
        let n = self.w * self.h;
        for i in 0..n {
            if self.solid[i] {
                self.rho.as_mut_slice()[i] = 1.0;
                self.ux.as_mut_slice()[i] = 0.0;
                self.uy.as_mut_slice()[i] = 0.0;
            }
            let (r, ux, uy) = (
                self.rho.as_slice()[i],
                self.ux.as_slice()[i],
                self.uy.as_slice()[i],
            );
            for (q, e) in heq(r - 1.0, ux, uy).into_iter().enumerate() {
                self.f[q * n + i] = e;
            }
        }
    }

    /// Whether every moment is finite and ρ is in the watchdog band.
    fn healthy(&self) -> bool {
        let rho_ok = self
            .rho
            .as_slice()
            .iter()
            .all(|r| (RHO_OK.0..=RHO_OK.1).contains(r));
        let finite = |g: &Grid<f32>| g.as_slice().iter().all(|v| v.is_finite());
        rho_ok && finite(&self.ux) && finite(&self.uy)
    }

    /// Σρ in f64, for conservation checks.
    #[cfg(test)]
    fn mass(&self) -> f64 {
        let n = self.w * self.h;
        (0..n)
            .map(|i| 1.0 + (0..9).map(|q| f64::from(self.f[q * n + i])).sum::<f64>())
            .sum()
    }
}

/// The nine planes of a population buffer as separate slices. Both buffers are sized
/// `9·n` by construction; a wrong size fails here rather than as a far-away index panic.
fn planes(buf: &[f32], n: usize) -> [&[f32]; 9] {
    debug_assert_eq!(buf.len(), 9 * n);
    let mut it = buf.chunks_exact(n);
    std::array::from_fn(|_| it.next().unwrap_or_default())
}

fn planes_mut(buf: &mut [f32], n: usize) -> [&mut [f32]; 9] {
    debug_assert_eq!(buf.len(), 9 * n);
    let mut it = buf.chunks_exact_mut(n);
    std::array::from_fn(|_| it.next().unwrap_or_default())
}

/// Moment outputs of one row segment.
struct Moments<'a> {
    rho: &'a mut [f32],
    ux: &'a mut [f32],
    uy: &'a mut [f32],
}

/// Stream + collide the fluid run `a..b` of row `y` with no solid and no wrap in reach:
/// every population is a plain offset slice, so the loop is branch-free.
fn run_fast(
    src: &[&[f32]; 9],
    dst: &mut [&mut [f32]; 9],
    out: Moments<'_>,
    (w, h): (usize, usize),
    (y, a, b): (usize, usize, usize),
    (wp, wm): (f32, f32),
) {
    let m = b - a;
    let s: [&[f32]; 9] = std::array::from_fn(|i| {
        let sy = (y as i32 - CY[i]).rem_euclid(h as i32) as usize;
        let start = (a as i32 - CX[i]) as usize;
        &src[i][sy * w + start..][..m]
    });
    let mut d = dst.each_mut().map(|p| &mut p[y * w + a..][..m]);
    let (rho, ux, uy) = (&mut out.rho[..m], &mut out.ux[..m], &mut out.uy[..m]);
    // Nodes go through `collide` in groups of LANES with every population as a small
    // array, a shape LLVM turns into SIMD; the remainder runs one node at a time.
    let full = m - m % LANES;
    for k in (0..full).step_by(LANES) {
        let mut f: [[f32; LANES]; 9] =
            std::array::from_fn(|q| std::array::from_fn(|l| s[q][k + l]));
        let mut moments = [[0.0f32; LANES]; 3];
        for l in 0..LANES {
            let mut node: [f32; 9] = std::array::from_fn(|q| f[q][l]);
            let (r, u, v) = collide(&mut node, wp, wm);
            (moments[0][l], moments[1][l], moments[2][l]) = (r, u, v);
            for (q, value) in node.into_iter().enumerate() {
                f[q][l] = value;
            }
        }
        for (plane, lanes) in d.iter_mut().zip(&f) {
            plane[k..k + LANES].copy_from_slice(lanes);
        }
        for (field, lanes) in [&mut *rho, &mut *ux, &mut *uy].into_iter().zip(&moments) {
            field[k..k + LANES].copy_from_slice(lanes);
        }
    }
    for k in full..m {
        let mut f: [f32; 9] = std::array::from_fn(|q| s[q][k]);
        (rho[k], ux[k], uy[k]) = collide(&mut f, wp, wm);
        for (plane, value) in d.iter_mut().zip(f) {
            plane[k] = value;
        }
    }
}

/// Stream + collide one node with wrap-around and halfway bounce-back: a pull from a
/// solid node returns the node's own opposite post-collision population.
fn node_slow(
    lat_solid: &[bool],
    src: &[&[f32]; 9],
    dst: &mut [&mut [f32]; 9],
    (w, h): (usize, usize),
    (x, y): (usize, usize),
    (wp, wm): (f32, f32),
) -> (f32, f32, f32) {
    let i = y * w + x;
    if lat_solid[i] {
        for plane in dst.iter_mut() {
            plane[i] = 0.0;
        }
        return (1.0, 0.0, 0.0);
    }
    let mut f = [0.0f32; 9];
    for q in 0..9 {
        let sx = (x as i32 - CX[q]).rem_euclid(w as i32) as usize;
        let sy = (y as i32 - CY[q]).rem_euclid(h as i32) as usize;
        let j = sy * w + sx;
        f[q] = if lat_solid[j] {
            src[OPP[q]][i]
        } else {
            src[q][j]
        };
    }
    let moments = collide(&mut f, wp, wm);
    for (plane, v) in dst.iter_mut().zip(f) {
        plane[i] = v;
    }
    moments
}

/// One lattice time step: fused pull stream + TRT collide into the spare buffer, then
/// the boundary conditions of `bc`, then swap. Writes ρ, u of the step's pre-collision
/// state into the moment grids.
pub(crate) fn step(lat: &mut Lattice, p: &FlowParams, bc: Bc) {
    let (w, h) = (lat.w, lat.h);
    let n = w * h;
    if n == 0 || (bc == Bc::Channel && (w < 3 || lat.inflow.len() != h)) {
        return;
    }
    let omegas = (p.omega_p, p.omega_m);
    {
        let src = planes(&lat.f, n);
        let mut dst = planes_mut(&mut lat.next, n);
        let (rho, ux, uy) = (
            lat.rho.as_mut_slice(),
            lat.ux.as_mut_slice(),
            lat.uy.as_mut_slice(),
        );
        let (sx0, sx1) = (lat.solid_cols.0.max(1), lat.solid_cols.1.min(w - 1));
        for y in 0..h {
            let mut fast = |a: usize, b: usize| {
                if a < b {
                    let r = y * w + a..y * w + b;
                    let out = Moments {
                        rho: &mut rho[r.clone()],
                        ux: &mut ux[r.clone()],
                        uy: &mut uy[r],
                    };
                    run_fast(&src, &mut dst, out, (w, h), (y, a, b), omegas);
                }
            };
            if lat.near_solid[y] && sx0 < sx1 {
                fast(1, sx0);
                fast(sx1, w - 1);
            } else {
                fast(1, w.saturating_sub(1).max(1));
            }
            let mut slow = |x: usize| {
                let i = y * w + x;
                (rho[i], ux[i], uy[i]) =
                    node_slow(&lat.solid, &src, &mut dst, (w, h), (x, y), omegas);
            };
            if lat.near_solid[y] {
                (sx0..sx1).for_each(&mut slow);
            }
            if bc == Bc::Periodic {
                slow(0);
                if w > 1 {
                    slow(w - 1);
                }
            }
        }
    }
    if bc == Bc::Channel {
        channel_boundaries(lat, p);
    }
    std::mem::swap(&mut lat.f, &mut lat.next);
}

/// Quadratic sponge ramp: 0 at `t = 0`, [`SPONGE_MAX`] at `t = 1`.
fn ramp(t: f32) -> f32 {
    SPONGE_MAX * t * t
}

/// Sponges, inlet and zero-gradient outlet on the freshly written buffer.
///
/// The outlet sponge relaxes toward the uniform stream, so eddies fade before the
/// outlet instead of reflecting off it. The inlet sponge relaxes only the
/// non-equilibrium part (toward each node's own equilibrium): it raises the viscosity
/// of the first columns without imposing a velocity, which would itself shed vorticity.
fn channel_boundaries(lat: &mut Lattice, p: &FlowParams) {
    let (w, h, n) = (lat.w, lat.h, lat.w * lat.h);
    let uniform = lat.outflow;
    let mut dst = planes_mut(&mut lat.next, n);
    let x0 = p.w_vis.min(w - 1);
    // Row by row, so the inner loop runs over contiguous columns.
    for (plane, e) in dst.iter_mut().zip(uniform) {
        for row in plane.chunks_exact_mut(w) {
            for (v, s) in row[x0..w - 1].iter_mut().zip(&lat.sponge) {
                *v -= s * (*v - e);
            }
        }
    }
    for x in 1..=INLET_SPONGE.min(w - 2) {
        let sigma = ramp((INLET_SPONGE + 1 - x) as f32 / INLET_SPONGE as f32);
        for y in 0..h {
            let i = y * w + x;
            let local = heq(
                lat.rho.as_slice()[i] - 1.0,
                lat.ux.as_slice()[i],
                lat.uy.as_slice()[i],
            );
            for (plane, e) in dst.iter_mut().zip(local) {
                plane[i] -= sigma * (plane[i] - e);
            }
        }
    }
    for (q, plane) in dst.iter_mut().enumerate() {
        for (row, inlet) in plane.chunks_exact_mut(w).zip(&lat.inflow) {
            row[0] = inlet[q];
            row[w - 1] = row[w - 2];
        }
    }
    for g in [&mut lat.rho, &mut lat.ux, &mut lat.uy] {
        for row in g.as_mut_slice().chunks_exact_mut(w) {
            row[0] = row[1];
            row[w - 1] = row[w - 2];
        }
    }
}

/// The undisturbed flow (y up, relative to the cylinder centre): the potential flow past
/// the cylinder and its nearest periodic images at y ± H, so the field is continuous
/// across the periodic seam.
fn base_flow(p: &FlowParams, mx: f64, my: f64) -> (f64, f64) {
    let hf = p.h as f64;
    [-hf, 0.0, hf].iter().fold((p.u, 0.0), |(u, v), &image| {
        let (pu, pv) = potential_flow(mx, my - image, 0.5 * p.d, p.u);
        (u + pu - p.u, v + pv)
    })
}

/// Whether lattice node `(x, y)` is inside the cylinder (staircase mask).
fn in_cylinder(p: &FlowParams, x: usize, y: usize) -> bool {
    let (dx, dy) = (x as f64 - p.xc, y as f64 - p.yc);
    dx * dx + dy * dy <= 0.25 * p.d * p.d
}

/// Start from an analytic street (VOR-A5): the [`base_flow`] plus two staggered rows of
/// Scully vortices, CW above and CCW below, each with its periodic images at y ± H.
/// The street begins `1.5 D` behind the centre, offset by a seeded fraction of the
/// half-spacing, with a seeded leading row; together these cover every phase in
/// `[0, a)` while always placing an eddy in the near wake.
///
/// A start-up pair of zero net circulation (Kelvin) is added: a bound circulation on
/// the cylinder and the opposite eddy just behind it, on the side of the leading row.
/// An impulsive start is otherwise symmetric about the centre line, and the wake then
/// needs ≈ 10 s to grow its own asymmetry, showing twin shear layers meanwhile; with
/// the pair it sheds at full amplitude within ≈ 4 s.
///
/// ρ follows Bernoulli, `1 + 1.5 (U² − |u|²)`, which is exact in the irrotational flow,
/// plus each Scully core's cyclostrophic excess ([`scully_core_dp`]), which Bernoulli
/// misses inside the rotational cores; every population is at equilibrium.
pub(crate) fn init_street(lat: &mut Lattice, p: &FlowParams, rng: &mut impl Rng) {
    let a = SPACING_D * p.d;
    let half_gap = 0.5 * GAP_RATIO * a;
    let gamma = GAMMA_UD * p.u * p.d;
    let rc = CORE_D * p.d;
    let x_start = STREET_START_D * p.d + rng.random_range(0.0..0.5 * a);
    let lead_upper = rng.random_bool(0.5);
    // Upper (y up) and clockwise, or lower and counter-clockwise.
    let side = |upper: bool| if upper { 1.0 } else { -1.0 };
    let lead = side(lead_upper);
    let bound = -lead * START_PAIR_UD * p.u * p.d;
    let eddy = (START_EDDY_D.0 * p.d, lead * START_EDDY_D.1 * p.d);
    let (w, h) = (lat.w, lat.h);
    let hf = h as f64;
    let count = ((w as f64 - p.xc - x_start) / (0.5 * a)).ceil().max(0.0) as usize;
    for y in 0..h {
        for x in 0..w {
            // Math frame (y up) relative to the cylinder centre.
            let (mx, my) = (x as f64 - p.xc, p.yc - y as f64);
            // Street velocity and core pressure excess.
            let (mut su, mut sv, mut sp) = (0.0, 0.0, 0.0);
            for k in 0..count {
                let upper = (k % 2 == 0) == lead_upper;
                let (vx, vy) = (x_start + 0.5 * a * k as f64, side(upper) * half_gap);
                for image in [-hf, 0.0, hf] {
                    let (dx, dy) = (mx - vx, my - vy - image);
                    let (u, v) = scully(dx, dy, -side(upper) * gamma, rc);
                    (su, sv) = (su + u, sv + v);
                    sp += scully_core_dp(dx, dy, gamma, rc);
                }
            }
            // Start-up pair: the bound vortex is a point vortex (no core, no excess)
            // outside the wall.
            let (mut pu, mut pv) = if mx * mx + my * my > 0.25 * p.d * p.d {
                scully(mx, my, bound, 0.0)
            } else {
                (0.0, 0.0)
            };
            let (dx, dy) = (mx - eddy.0, my - eddy.1);
            let (u, v) = scully(dx, dy, -bound, rc);
            (pu, pv) = (pu + u, pv + v);
            let pp = scully_core_dp(dx, dy, bound, rc);
            let (xf, yf) = (x as f64, y as f64);
            let (ws, wp) = (street_taper(p, xf, yf), pair_window(p, xf, yf));
            let (u, v) = base_flow(p, mx, my);
            let (u, v) = (u + ws * su + wp * pu, v + ws * sv + wp * pv);
            let i = y * w + x;
            // Bernoulli: p − p∞ = ½(U² − |u|²) and δρ = δp / c_s², so the start carries
            // the pressure its velocity field implies instead of launching an acoustic
            // pulse from a uniform density. The core excess scales with Γ², so with the
            // square of each window.
            let dp = 0.5 * (p.u * p.u - (u * u + v * v)) + ws * ws * sp + wp * wp * pp;
            lat.rho.as_mut_slice()[i] = (1.0 + 3.0 * dp) as f32;
            lat.ux.as_mut_slice()[i] = u as f32;
            // Rows grow downward.
            lat.uy.as_mut_slice()[i] = -v as f32;
        }
    }
    lat.equilibrate();
}

/// The part of a Scully vortex's pressure that Bernoulli misses inside its rotational
/// core. The cyclostrophic balance `dp/dr = v_θ²/r` gives `p − p∞ = −Γ²/(8π²(r² + r_c²))`,
/// lowest at the centre, whereas `½(U² − |u|²)` gives `−Γ² r²/(8π²(r² + r_c²)²)`, which
/// climbs back to p∞ there. The difference is `−Γ² r_c²/(8π²(r² + r_c²)²)`: zero for a
/// point vortex, and negligible far from the core.
pub(crate) fn scully_core_dp(dx: f64, dy: f64, gamma: f64, rc: f64) -> f64 {
    let s = dx * dx + dy * dy + rc * rc;
    -gamma * gamma * rc * rc / (8.0 * PI * PI * s * s)
}

/// Window on the analytic street: 1 downstream, easing to 0 from half a diameter behind
/// the centre to the inlet (which imposes the undisturbed [`base_flow`]), and toward the
/// periodic seam (where the truncated image sum of a like-signed vortex column jumps by
/// ≈ Γ/(2π·1.5H)). Without it both mismatches shed visible vorticity sheets for the first
/// second; easing over many columns keeps them below the display threshold.
fn street_taper(p: &FlowParams, x: f64, y: f64) -> f64 {
    ease::smoothstep(x / (p.xc + 0.5 * p.d)) * seam_taper(p, y)
}

/// Window on the start-up pair. Its field is irrotational outside the eddy's core, and a
/// window that multiplies it creates vorticity `w′(x)·v` wherever `w` varies: the street's
/// taper, steepest just in front of the cylinder where the bound vortex is strong, left a
/// vorticity sheet there that took ≈ 120 steps to wash out. So the pair is kept whole
/// around the cylinder and fades only upstream of [`PAIR_FADE_D`] diameters in front of
/// the centre, toward the inlet, where its field is small; and toward the seam.
fn pair_window(p: &FlowParams, x: f64, y: f64) -> f64 {
    let full = (p.xc - PAIR_FADE_D * p.d).max(1.0);
    ease::smoothstep(x / full) * seam_taper(p, y)
}

/// 1 over the middle half of the rows, easing to 0 at the periodic seam.
fn seam_taper(p: &FlowParams, y: f64) -> f64 {
    let from_seam = 0.5 * p.h as f64 - (y - p.yc).abs();
    ease::smoothstep(from_seam / (0.25 * p.h as f64))
}

/// Inlet speed `U(t)/U₀` at simulated time `t` (VOR-A8): a slow breath that keeps Re
/// within ±8 % of its mean.
pub(crate) fn breath(t: f64) -> f64 {
    1.0 + BREATH_AMP * (2.0 * PI * t / BREATH_PERIOD).sin_det()
}

/// Zero-crossing probe for the Strouhal readout.
#[derive(Debug, Clone)]
struct Probe {
    index: usize,
    armed: bool,
    /// Lattice steps of the last ≤ `ST_INTERVALS + 1` upward crossings.
    crossings: Trail<u64>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            index: 0,
            armed: false,
            crossings: Trail::new(ST_INTERVALS + 1),
        }
    }
}

impl Probe {
    fn at(p: &FlowParams) -> Self {
        let x = (p.xc + PROBE_D * p.d).round() as usize;
        let y = p.yc.round() as usize;
        Self {
            index: y.min(p.h - 1) * p.w + x.min(p.w - 1),
            ..Self::default()
        }
    }

    /// Record an upward crossing of u_y (with hysteresis) at lattice step `step`.
    fn sample(&mut self, uy: f32, u: f64, step: u64) {
        let (uy, band) = (f64::from(uy), HYST * u);
        if uy < -band {
            self.armed = true;
        } else if uy > band && self.armed {
            self.armed = false;
            self.crossings.push(step);
        }
    }
}

#[derive(Debug, Default)]
pub struct VortexStreet {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    lat: Lattice,
    params: Option<FlowParams>,
    /// Fractional lattice steps carried between updates.
    acc: f64,
    steps: u64,
    since_check: f64,
    probe: Probe,
    reinits: u32,
    /// Soft-clipped vorticity over the visible columns (the upstream noise zeroed).
    vort: Grid<f32>,
    /// M16: the contours and the cylinder (GEN-R25).
    dots: Dots,
    size: Option<Size>,
}

impl VortexStreet {
    /// The inlet's current speed factor `U(t)/U₀`, from the scene's own step count.
    fn inlet_scale(&self) -> f64 {
        breath(self.steps as f64 / STEPS_PER_S)
    }

    /// Re-grid the flow onto a new lattice after a bucket change (ADR-0012): ρ and u are
    /// resampled, then rebuilt at equilibrium. Every lattice runs at the same U₀ ([`U0`]),
    /// so the moments need no rescaling.
    fn regrid(&mut self, p: &FlowParams) {
        let mut lat = Lattice::channel(p, self.inlet_scale());
        grid::resample_bilinear(&self.lat.rho, &mut lat.rho);
        grid::resample_bilinear(&self.lat.ux, &mut lat.ux);
        grid::resample_bilinear(&self.lat.uy, &mut lat.uy);
        lat.equilibrate();
        self.lat = lat;
    }

    /// Start from the analytic street and pre-roll [`PREROLL_STEPS`]: the potential flow
    /// slips at the no-slip wall and starts impulsively, and the few steps it takes the
    /// slip sheet to diffuse and the start-up pressure pulse to leave are not shown.
    fn restart(&mut self, p: &FlowParams, rng: &mut impl Rng) {
        self.lat = Lattice::channel(p, self.inlet_scale());
        init_street(&mut self.lat, p, rng);
        for _ in 0..PREROLL_STEPS {
            step(&mut self.lat, p, Bc::Channel);
        }
        self.probe = Probe::at(p);
    }

    /// The soft-clipped vorticity on the visible nodes, with the staircase noise upstream
    /// of the round mask set to 0 (see [`UPSTREAM_MIN`]).
    fn field(&mut self, p: &FlowParams) {
        vorticity(&self.lat.ux, &self.lat.uy, VIEW_X0, &mut self.vort);
        let scale = p.d / (VORT_SCALE * p.u);
        let vw = usize::from(self.vort.width());
        let radius = 0.5 * p.d;
        let opposite = radius + UPSTREAM_OPPOSITE_D * p.d;
        for (i, v) in self.vort.as_mut_slice().iter_mut().enumerate() {
            let s = ink::soft_clip(f64::from(*v) * scale);
            let (dx, dy) = ((VIEW_X0 + i % vw) as f64 - p.xc, (i / vw) as f64 - p.yc);
            let r2 = dx * dx + dy * dy;
            // Rows grow downward, so `s·dy < 0` is the sign opposite to the boundary
            // layer's.
            let noise = dx < 0.0
                && s.abs() < UPSTREAM_MIN
                && r2 < opposite * opposite
                && (r2.sqrt() < radius + UPSTREAM_ANNULUS || s * dy < 0.0);
            *v = if noise { 0.0 } else { s as f32 };
        }
    }

    /// Recompute the vorticity and draw it as contours (VOR-A9), then the cylinder on top.
    fn rasterise(&mut self) {
        let Some(p) = self.params else { return };
        self.field(&p);
        self.dots.clear();
        let (vw, vh) = (
            usize::from(self.vort.width()),
            usize::from(self.vort.height()),
        );
        let ds = self.dots.dot_size();
        if vw < 2 || vh < 2 {
            return;
        }
        // Node (x, y) → dot coordinates, the cell-centre convention (CORE-R68).
        let (sx, sy) = (
            f64::from(ds.width()) / vw as f64,
            f64::from(ds.height()) / vh as f64,
        );
        let to_dot = move |x: f64, y: f64| ((x + 0.5) * sx - 0.5, (y + 0.5) * sy - 0.5);
        let k = ((sx.max(sy) / REFINE).ceil() as usize).clamp(1, MAX_REFINE);
        let values = self.vort.as_slice();
        let dots = &mut self.dots;
        for j in 0..vh - 1 {
            let (top, bottom) = (
                &values[j * vw..(j + 1) * vw],
                &values[(j + 1) * vw..(j + 2) * vw],
            );
            for i in 0..vw - 1 {
                let c = [top[i], top[i + 1], bottom[i + 1], bottom[i]];
                let (lo, hi) = (
                    c[0].min(c[1]).min(c[2].min(c[3])),
                    c[0].max(c[1]).max(c[2].max(c[3])),
                );
                if hi < LEVELS[0] && -lo < LEVELS[0] {
                    continue;
                }
                for (sign, role) in [(1.0f32, Role::Hot), (-1.0, Role::Cold)] {
                    let (slo, shi) = if sign > 0.0 { (lo, hi) } else { (-hi, -lo) };
                    for (&level, &ink_level) in LEVELS.iter().zip(&LEVEL_INKS) {
                        if !(slo < level && level <= shi) {
                            continue;
                        }
                        let ink = Ink::new(role, ink_level);
                        dots.contour_cell_refined(
                            c.map(|v| sign * v),
                            level,
                            (i, j),
                            k,
                            to_dot,
                            ink,
                        );
                    }
                }
            }
        }
        // The cylinder: a one-dot ring of radius D/2 in square dots.
        let (cx, cy) = to_dot(p.xc - VIEW_X0 as f64, p.yc);
        let r = 0.5 * p.d * 0.5 * (sx + sy);
        let point = |k: usize| {
            let (s, c) = (std::f64::consts::TAU * k as f64 / CYLINDER_PTS as f64).sin_cos_det();
            (cx + r * c, cy + r * s)
        };
        for k in 0..CYLINDER_PTS {
            let (a, b) = (point(k), point(k + 1));
            self.dots.line(a.0, a.1, b.0, b.1, CYLINDER_INK);
        }
    }

    /// `Re≈140` and, once measured and at width ≥ 40, `  St≈0.18` (row 0, column 1); both
    /// follow the breathing inlet. St is on the gap speed, St·(1 − D/H) (see [`strouhal`]).
    fn label(&self, canvas: &mut Canvas) {
        let (Some(p), Some(size)) = (self.params, self.size) else {
            return;
        };
        if size.width() < 16 || size.height() < 6 {
            return;
        }
        let re = (p.re * self.inlet_scale() / 5.0).round() as u32 * 5;
        // On the gap speed, so the number compares with the unconfined literature value.
        let confinement = 1.0 - p.d / p.h as f64;
        let u_at = |step: f64| p.u * breath(step / STEPS_PER_S);
        let st = strouhal(&self.probe.crossings, p.d, u_at)
            .filter(|_| size.width() >= 40)
            .map(|st| st * confinement);
        let (mut uni, mut asc) = (StackText::<32>::new(), StackText::<32>::new());
        let _ = write!(uni, "Re≈{re}");
        let _ = write!(asc, "Re~{re}");
        if let Some(st) = st {
            let _ = write!(uni, "  St≈{st:.2}");
            let _ = write!(asc, "  St~{st:.2}");
        }
        draw::text_mode(canvas, 1, 0, uni.as_str(), asc.as_str(), LABEL_INK);
    }
}

impl Generator for VortexStreet {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let lat_size = lattice_size(size);
        if self.params.is_none_or(|p| {
            (p.w, p.h)
                != (
                    usize::from(lat_size.width()),
                    usize::from(lat_size.height()),
                )
        }) {
            let p = flow_params(lat_size);
            match self.params {
                None => self.restart(&p, &mut ctx.rng),
                Some(_) => {
                    self.regrid(&p);
                    self.probe = Probe::at(&p);
                }
            }
            self.params = Some(p);
            self.vort = Grid::new(Size::new((p.w_vis - VIEW_X0) as u16, p.h as u16), 0.0);
        }
        self.dots.resize(size);
        self.size = Some(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let Some(p) = self.params else { return };
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        self.acc += dt * STEPS_PER_S;
        let n = (self.acc as usize).min(MAX_STEPS);
        self.acc = (self.acc - n as f64).min(1.0);
        // Quasi-static: the inlet changes by < 0.1 % U per update.
        self.lat.set_inlet(&p, self.inlet_scale());
        for _ in 0..n {
            step(&mut self.lat, &p, Bc::Channel);
            self.steps += 1;
            let uy = self.lat.uy.as_slice()[self.probe.index];
            // Rows grow downward; the crossing direction does not change the period.
            self.probe.sample(uy, p.u, self.steps);
        }
        self.since_check += dt;
        if self.since_check >= CHECK_EVERY {
            self.since_check = 0.0;
            if !self.lat.healthy() {
                self.reinits += 1;
                self.restart(&p, &mut ctx.rng);
            }
        }
        self.dirty = true;
    }

    /// GEN-R20: rasterise once per drawn frame, only after the state changed.
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        self.label(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seeded RNG from the scene context (the crate's only RNG, GEN-R7).
    fn rng(seed: u64) -> impl Rng {
        Ctx::new(seed, Size::new(1, 1)).rng
    }

    fn params_for(w: u16, h: u16, tau: f64) -> FlowParams {
        FlowParams {
            omega_p: (1.0 / tau) as f32,
            omega_m: (1.0 / tau_minus(tau, MAGIC)) as f32,
            ..flow_params(Size::new(w, h))
        }
    }

    /// Full populations `f = w + h` of node `i`, in f64.
    fn node(lat: &Lattice, i: usize) -> [f64; 9] {
        let n = lat.w * lat.h;
        std::array::from_fn(|q| f64::from(W[q]) + f64::from(lat.f[q * n + i]))
    }

    fn moments(f: &[f64; 9]) -> (f64, f64, f64) {
        let rho: f64 = f.iter().sum();
        let jx: f64 = (0..9).map(|q| f[q] * f64::from(CX[q])).sum();
        let jy: f64 = (0..9).map(|q| f[q] * f64::from(CY[q])).sum();
        (rho, jx, jy)
    }

    #[test]
    fn vor_a1_equilibrium_moments_trt_collision_and_magic_tau() {
        let mut rng = rng(1);
        assert!((tau_minus(0.53, 0.25) - 8.8333).abs() < 1e-4);
        assert_eq!(heq(0.0, 0.0, 0.0), [0.0; 9], "feq(1, 0) = w");
        let mut missed = 0;
        for _ in 0..100 {
            let rho: f64 = rng.random_range(0.9..1.1);
            let (ux, uy): (f64, f64) =
                (rng.random_range(-0.14..0.14), rng.random_range(-0.14..0.14));
            let e = heq((rho - 1.0) as f32, ux as f32, uy as f32);
            let f: [f64; 9] = std::array::from_fn(|q| f64::from(W[q]) + f64::from(e[q]));
            let (r, jx, jy) = moments(&f);
            assert!(
                (r - rho).abs() < 1e-6
                    && (jx - rho * ux).abs() < 1e-6
                    && (jy - rho * uy).abs() < 1e-6
            );
            let u = [ux, uy];
            for a in 0..2 {
                for b in 0..2 {
                    let c = |q: usize, k: usize| f64::from(if k == 0 { CX[q] } else { CY[q] });
                    let pi: f64 = (0..9).map(|q| f[q] * c(q, a) * c(q, b)).sum();
                    let want = rho * (if a == b { 1.0 / 3.0 } else { 0.0 } + u[a] * u[b]);
                    assert!((pi - want).abs() < 1e-6, "Π_{a}{b}: {pi} vs {want}");
                }
            }

            // A perturbed state: collision conserves ρ and ρu, and relaxes the even and
            // odd non-equilibrium parts by (1 − ω⁺) and (1 − ω⁻), each to 1e-4 of the
            // part itself (f32 round-off is ≈ 1e-8 absolute), so a slightly wrong ω⁻
            // cannot hide under an absolute tolerance.
            let mut h: [f32; 9] =
                std::array::from_fn(|q| e[q] + rng.random_range(-2e-2..2e-2) * W[q]);
            let before = h;
            let (wp, wm) = (1.0 / 0.53f32, 1.0 / 8.8333f32);
            let (r0, ux0, uy0) = collide(&mut h.clone(), 1.0, 1.0);
            let full = |h: &[f32; 9]| std::array::from_fn(|q| f64::from(W[q]) + f64::from(h[q]));
            let m0 = moments(&full(&before));
            let (r1, _, _) = collide(&mut h, wp, wm);
            let m1 = moments(&full(&h));
            assert!(
                (m0.0 - m1.0).abs() < 1e-6
                    && (m0.1 - m1.1).abs() < 1e-6
                    && (m0.2 - m1.2).abs() < 1e-6
            );
            assert_eq!(r0, r1);
            let eq = heq(r0 - 1.0, ux0, uy0);
            let neq = |h: &[f32; 9], q: usize| f64::from(h[q]) - f64::from(eq[q]);
            let relaxed = |after: f64, before: f64, omega: f32| {
                let want = (1.0 - f64::from(omega)) * before;
                (after - want).abs() <= 1e-4 * before.abs() + 5e-8
            };
            let (n0, n1) = (neq(&before, 0), neq(&h, 0));
            assert!(relaxed(n1, n0, wp), "rest: {n0} → {n1}");
            // A slightly wrong ω⁻ (|Δ(1 − ω⁻)| = 1e-3) must fail the odd check.
            let mut wrong = before;
            collide(&mut wrong, wp, wm + 1e-3);
            let mut caught = false;
            for (i, j) in PAIRS {
                let even = |h: &[f32; 9]| 0.5 * (neq(h, i) + neq(h, j));
                let odd = |h: &[f32; 9]| 0.5 * (neq(h, i) - neq(h, j));
                assert!(relaxed(even(&h), even(&before), wp), "even ({i}, {j})");
                assert!(relaxed(odd(&h), odd(&before), wm), "odd ({i}, {j})");
                caught |= !relaxed(odd(&wrong), odd(&before), wm);
            }
            missed += usize::from(!caught);

            // ω⁻ = ω⁺ is BGK.
            let mut bgk = before;
            collide(&mut bgk, wp, wp);
            for q in 0..9 {
                let want = before[q] - wp * (before[q] - eq[q]);
                assert!((bgk[q] - want).abs() < 1e-6);
            }
        }
        assert!(missed <= 5, "a wrong ω⁻ passed {missed} of 100 states");
    }

    #[test]
    fn vor_a2_uniform_flow_stays_uniform_and_bounce_back_conserves_mass() {
        let size = Size::new(32, 16);
        let p = params_for(32, 16, TAU);
        let mut lat = Lattice::new(size, |_, _| false);
        lat.ux.fill(0.1);
        lat.equilibrate();
        for _ in 0..1000 {
            step(&mut lat, &p, Bc::Periodic);
        }
        let first = node(&lat, 0);
        for i in 0..lat.w * lat.h {
            assert_eq!(node(&lat, i), first, "node {i} differs");
        }
        let (rho, jx, jy) = moments(&first);
        assert!((rho - 1.0).abs() < 1e-6 && (jx / rho - 0.1).abs() < 1e-6 && jy.abs() < 1e-6);

        let disc = |x: usize, y: usize| {
            let (dx, dy) = (x as f64 - 10.0, y as f64 - 8.0);
            dx * dx + dy * dy <= 9.0
        };
        let mut lat = Lattice::new(size, disc);
        lat.ux.fill(0.05);
        lat.equilibrate();
        let m0 = lat.mass();
        for _ in 0..300 {
            step(&mut lat, &p, Bc::Periodic);
        }
        assert!(
            ((lat.mass() - m0) / m0).abs() < 1e-5,
            "{} vs {m0}",
            lat.mass()
        );
    }

    /// ν fitted from the decay of a periodic shear wave between steps `t0` and `t1`.
    fn shear_viscosity(tau: f64, t0: usize, t1: usize) -> f64 {
        let (w, h) = (4usize, 64usize);
        let p = params_for(w as u16, h as u16, tau);
        let mut lat = Lattice::new(Size::new(w as u16, h as u16), |_, _| false);
        let k = 2.0 * PI / h as f64;
        for (i, u) in lat.ux.as_mut_slice().iter_mut().enumerate() {
            *u = (0.01 * (k * (i / w) as f64).sin_det()) as f32;
        }
        lat.equilibrate();
        let amplitude = |lat: &Lattice| {
            let n = w * h;
            (0..n)
                .map(|i| {
                    let (rho, jx, _) = moments(&node(lat, i));
                    jx / rho * (k * (i / w) as f64).sin_det()
                })
                .sum::<f64>()
                * 2.0
                / n as f64
        };
        let mut a0 = 0.0;
        for t in 0..=t1 {
            if t == t0 {
                a0 = amplitude(&lat);
            }
            if t < t1 {
                step(&mut lat, &p, Bc::Periodic);
            }
        }
        (a0 / amplitude(&lat)).ln() / (k * k * (t1 - t0) as f64)
    }

    #[test]
    fn vor_a3_shear_wave_decays_at_the_trt_viscosity() {
        for (tau, t1) in [(0.53, 2100), (0.8, 1100)] {
            let nu = shear_viscosity(tau, 100, t1);
            let want = (tau - 0.5) / 3.0;
            assert!(
                ((nu - want) / want).abs() < 0.02,
                "τ = {tau}: ν = {nu}, want {want}"
            );
        }
    }

    #[test]
    fn vor_a4_lattice_is_capped_bucketed_and_keeps_the_display_aspect() {
        let bucket_of = |w: u16, h: u16| {
            let aspect = (f64::from(w) / (2.0 * f64::from(h))).clamp(1.0, 4.0) / 0.9;
            (aspect.log(1.12)).round() as i32
        };
        let mut by_bucket = std::collections::HashMap::new();
        for w in (1..=480u16).step_by(7) {
            for h in (1..=160u16).step_by(3) {
                let l = lattice_size(Size::new(w, h));
                let nodes = l.area();
                assert!((9500..=10_240).contains(&nodes), "{w}x{h}: {nodes}");
                let prev = by_bucket.insert(bucket_of(w, h), l);
                assert!(
                    prev.is_none_or(|p| p == l),
                    "{w}x{h}: bucket maps to two lattices"
                );
                let p = flow_params(l);
                let aspect = f64::from(w) / (2.0 * f64::from(h));
                if (1.0..=4.0).contains(&aspect) {
                    let visible = (p.w_vis - VIEW_X0) as f64 / p.h as f64;
                    assert!(
                        (visible / aspect - 1.0).abs() <= 0.07,
                        "{w}x{h}: {visible} vs {aspect}"
                    );
                }
                assert_eq!(p.omega_p, (1.0 / TAU) as f32);
                assert!(p.u == U0 && p.u * 3f64.sqrt() <= 0.217 && p.d <= 12.0);
                assert!((6.0..=16.0).contains(&p.d) && p.d / p.h as f64 <= 0.15);
                assert!(
                    (87.0..=150.0 + 1e-9).contains(&p.re),
                    "{w}x{h}: Re {}",
                    p.re
                );
                let want_d = (0.14 * p.h as f64)
                    .min(p.w_vis as f64 / 9.0)
                    .round()
                    .clamp(6.0, 16.0);
                assert_eq!(p.d, want_d, "{w}x{h}");
                let run_up = (1.65 * p.d).round();
                assert_eq!(p.xc, (0.15 * p.w_vis as f64).round().max(run_up));
                // The cylinder's front keeps a calm run-up of ≥ 1.1 D from the inlet.
                assert!(p.xc - 0.5 * p.d >= 1.1 * p.d, "{w}x{h}");
                assert_eq!(p.yc, (p.h as f64 - 1.0) / 2.0);
            }
        }
        for ((w, h), (lw, lh), d, xc, re) in [
            ((200, 50), (150, 68), 10.0, 20.0, 125.0),
            ((80, 24), (134, 76), 11.0, 18.0, 137.5),
            // Square displays (a vertical split of 200×50): the run-up and W/9 bind.
            ((100, 50), (107, 95), 11.0, 18.0, 137.5),
        ] {
            let l = lattice_size(Size::new(w, h));
            assert_eq!(l, Size::new(lw, lh));
            let p = flow_params(l);
            assert_eq!((p.d, p.xc), (d, xc), "{w}x{h}");
            assert!((p.re - re).abs() <= 1e-9, "{w}x{h}: Re {}", p.re);
        }
    }

    #[test]
    fn vor_a5_street_sheds_at_the_expected_strouhal_number() {
        let size = Size::new(96, 36);
        let p = flow_params(size);
        assert_eq!((p.d, p.u), (6.0, 0.125));
        assert!((p.re - 75.0).abs() < 1e-9);
        let mut lat = Lattice::channel(&p, 1.0);
        init_street(&mut lat, &p, &mut rng(42));
        let mut probe = Probe::at(&p);
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        let mut vort = Grid::new(Size::new(p.w as u16, p.h as u16), 0.0);
        let (mut upper, mut lower) = (0.0, 0.0);
        for t in 1..=2400u64 {
            step(&mut lat, &p, Bc::Channel);
            assert!(lat.healthy(), "watchdog would fire at step {t}");
            // Throughout, from the first step.
            for (i, (&r, (&u, &v))) in lat
                .rho
                .as_slice()
                .iter()
                .zip(lat.ux.as_slice().iter().zip(lat.uy.as_slice()))
                .enumerate()
            {
                assert!((0.9..=1.1).contains(&r), "step {t}: ρ = {r} at {i}");
                assert!(f64::from(u.hypot(v)) <= 2.5 * p.u, "step {t}: |u| at {i}");
            }
            let uy = lat.uy.as_slice()[probe.index];
            if t > 1000 {
                probe.sample(uy, p.u, t);
                (lo, hi) = (lo.min(uy), hi.max(uy));
                if t % 20 == 0 {
                    vorticity(&lat.ux, &lat.uy, 0, &mut vort);
                    for y in 0..p.h {
                        for x in p.xc as usize..(p.xc + 2.0 * p.d) as usize {
                            let dy = y as f64 - p.yc;
                            let om = f64::from(vort.as_slice()[y * p.w + x]);
                            if (0.5 * p.d..1.5 * p.d).contains(&-dy) {
                                upper += om;
                            } else if (0.5 * p.d..1.5 * p.d).contains(&dy) {
                                lower += om;
                            }
                        }
                    }
                }
            }
        }
        let st = strouhal(&probe.crossings, p.d, |_| p.u).expect("shedding");
        // St is based on the free-stream U. With y periodic the fluid passes a row of
        // cylinders H apart, so the gap speed is U·H/(H − D): Williamson's unconfined
        // St(75) ≈ 0.149 predicts 0.149/(1 − D/H) = 0.149/(1 − 1/6) ≈ 0.179 here. ±8 %
        // around that checks that shedding happens at the literature frequency. ν itself
        // is pinned by VOR-A3; a ±20 % ν error moves St by only ±5 %.
        assert!((0.165..=0.195).contains(&st), "St = {st}");
        assert!(
            f64::from(hi - lo) / 2.0 > 0.1 * p.u,
            "u_y amplitude {lo}..{hi}"
        );
        assert!(
            upper < 0.0 && lower > 0.0,
            "near-wake vorticity {upper} / {lower}"
        );
    }

    #[test]
    fn vor_a5_start_pressure_is_cyclostrophic_in_the_cores() {
        // For an isolated Scully vortex at rest, Bernoulli plus the core excess is the
        // exact cyclostrophic pressure −Γ²/(8π²(r² + r_c²)), lowest at the centre.
        let (gamma, rc) = (2.25, 3.5);
        let mut last = f64::NEG_INFINITY;
        for i in 0..=40 {
            let r = f64::from(i) * 0.5;
            let (u, v) = scully(r, 0.0, gamma, rc);
            let dp = -0.5 * (u * u + v * v) + scully_core_dp(r, 0.0, gamma, rc);
            let want = -gamma * gamma / (8.0 * PI * PI * (r * r + rc * rc));
            assert!((dp - want).abs() < 1e-12, "r = {r}: {dp} vs {want}");
            assert!(dp > last, "pressure rises outward");
            last = dp;
        }
        // A point vortex has no core and no excess.
        assert_eq!(scully_core_dp(3.0, 4.0, gamma, 0.0), 0.0);
    }

    /// Contoured nodes (|s| ≥ the outermost level, after the upstream rule) upstream of
    /// `x_c − 0.6 D` outside the band `|y − y_c| ≤ 0.6 D`: the calm inflow must hold none.
    fn upstream_nodes(g: &VortexStreet) -> usize {
        let p = g.params.unwrap();
        let vw = usize::from(g.vort.width());
        g.vort
            .as_slice()
            .iter()
            .enumerate()
            .filter(|&(i, s)| {
                let (x, y) = ((VIEW_X0 + i % vw) as f64, (i / vw) as f64);
                s.abs() >= LEVELS[0] && x < p.xc - 0.6 * p.d && (y - p.yc).abs() > 0.6 * p.d
            })
            .count()
    }

    /// Contoured fluid nodes on the upstream shoulders (dx < 0, |dy| > D/4, within 1.1 D
    /// of the centre) whose vorticity has the sign opposite to the boundary layer's.
    fn upstream_opposite_nodes(g: &VortexStreet) -> usize {
        let p = g.params.unwrap();
        let vw = usize::from(g.vort.width());
        g.vort
            .as_slice()
            .iter()
            .enumerate()
            .filter(|&(i, &s)| {
                let (dx, dy) = ((VIEW_X0 + i % vw) as f64 - p.xc, (i / vw) as f64 - p.yc);
                let r = (dx * dx + dy * dy).sqrt();
                // Rows grow downward: the layer is CW (s < 0) above, CCW below.
                let opposite = f64::from(s) * dy < 0.0;
                s.abs() >= LEVELS[0]
                    && dx < 0.0
                    && dy.abs() > 0.25 * p.d
                    && r > 0.5 * p.d
                    && r < 1.1 * p.d
                    && opposite
            })
            .count()
    }

    #[test]
    fn vor_a7_calm_inflow_and_clean_shoulders_from_the_first_frame_on() {
        // Square displays (a vertical split of 200×50) and the references, from the
        // first frame (after the pre-roll) through the start-up and in the developed
        // street; seed 3 once showed a persistent haze at 100×50.
        let runs = [
            ((100, 50), 3),
            ((100, 50), 42),
            ((100, 40), 42),
            ((80, 24), 42),
            ((200, 50), 42),
        ];
        std::thread::scope(|scope| {
            for (size, seed) in runs {
                scope.spawn(move || {
                    let size = Size::new(size.0, size.1);
                    let mut ctx = Ctx::new(seed, size);
                    let mut g = VortexStreet::default();
                    g.resize(size, &mut ctx);
                    let mut t = 0.0;
                    for check in [0.0, 0.1, 0.2, 0.3, 1.0, 20.0, 30.0, 45.0] {
                        while t < check - 0.5 * SIM_DT {
                            g.update(SIM_DT, &mut ctx);
                            g.prepare();
                            t += SIM_DT;
                        }
                        let id = format!("{size:?} seed {seed} t = {check}");
                        assert_eq!(upstream_nodes(&g), 0, "{id}: upstream contours");
                        assert_eq!(upstream_opposite_nodes(&g), 0, "{id}: shoulder specks");
                    }
                    // The watchdog never fires in normal operation.
                    assert_eq!(g.reinits, 0, "{size:?} seed {seed}");
                });
            }
            // And at the highest Re: a D = 12 lattice (119×85, Re₀ = 150, 162 at the
            // breath's peak).
            scope.spawn(|| {
                let size = Size::new(13, 5);
                let mut ctx = Ctx::new(42, size);
                let mut g = VortexStreet::default();
                g.resize(size, &mut ctx);
                let p = g.params.unwrap();
                assert_eq!((p.w, p.h, p.d, p.re), (119, 85, 12.0, 150.0));
                for _ in 0..(45.0 / SIM_DT) as usize {
                    g.update(SIM_DT, &mut ctx);
                    g.prepare();
                }
                assert_eq!(g.reinits, 0, "D = 12");
                assert!(g.lat.healthy());
            });
        });
    }

    fn scene(size: Size) -> (VortexStreet, Ctx) {
        let mut ctx = Ctx::new(7, size);
        let mut g = VortexStreet::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn fluid_mean_ux(g: &VortexStreet) -> f64 {
        let (mut s, mut n) = (0.0, 0.0);
        for (i, &u) in g.lat.ux.as_slice().iter().enumerate() {
            if !g.lat.solid[i] {
                s += f64::from(u);
                n += 1.0;
            }
        }
        s / n
    }

    #[test]
    fn vor_a6_resize_keeps_or_resamples_the_flow() {
        let (mut g, mut ctx) = scene(Size::new(200, 50));
        for _ in 0..30 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let before = g.lat.f.clone();
        g.resize(Size::new(190, 48), &mut ctx);
        g.resize(Size::new(200, 50), &mut ctx);
        assert_eq!(g.lat.size(), Size::new(150, 68));
        assert!(
            g.lat.f == before,
            "same-bucket resize must not touch the populations"
        );

        let mean = |v: &[f32]| v.iter().map(|&x| f64::from(x)).sum::<f64>() / v.len() as f64;
        let (rho0, u0) = (
            mean(g.lat.rho.as_slice()),
            fluid_mean_ux(&g) / g.params.unwrap().u,
        );
        g.resize(Size::new(100, 50), &mut ctx);
        let p = g.params.unwrap();
        assert_eq!((g.lat.size(), p.d), (Size::new(107, 95), 11.0));
        let (rho1, u1) = (mean(g.lat.rho.as_slice()), fluid_mean_ux(&g) / p.u);
        assert!((rho1 - rho0).abs() < 2e-3, "mean ρ {rho0} → {rho1}");
        assert!((u1 / u0 - 1.0).abs() < 0.03, "mean uₓ/U {u0} → {u1}");
        assert!(g.lat.f.iter().all(|v| v.is_finite()));
        for _ in 0..200 {
            step(&mut g.lat, &p, Bc::Channel);
        }
        assert!(g.lat.rho.as_slice().iter().all(|r| (0.9..=1.1).contains(r)));
        g.update(1.0, &mut ctx);
        g.prepare();
        assert_eq!(g.reinits, 0);
    }

    fn is_braille(c: char) -> bool {
        ('\u{2800}'..='\u{28ff}').contains(&c)
    }

    #[test]
    fn vor_a7_frame_shows_disc_and_two_signed_rows() {
        let (w, h) = (80usize, 24usize);
        let mut s = Stage::new(build(), 42, Size::new(80, 24), GlyphMode::Unicode);
        // The scene's own frame: the title would sit on the calm inflow's last row.
        s.set_title_visible(false);
        for _ in 0..30 {
            s.step(SIM_DT);
        }
        let frame = s.render();
        let cells = frame.cells();
        let at = |x: usize, y: usize| &cells[y * w + x];
        for y in 0..h {
            for x in 0..w {
                let c = at(x, y);
                if c.ch == ' ' || (y == 0 && c.ink.role == Role::Text) {
                    continue;
                }
                assert!(is_braille(c.ch), "({x}, {y}) {:?}", c.ch);
                assert!(
                    matches!(c.ink.role, Role::Hot | Role::Cold | Role::Structure),
                    "({x}, {y}) {:?}",
                    c.ink
                );
            }
        }
        let disc: Vec<(usize, usize)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| at(x, y).ink.role == Role::Structure && at(x, y).ch != ' ')
            .collect();
        assert!(!disc.is_empty());
        // One 8-connected blob (a ring of cells): flood fill reaches them all.
        let mut seen = vec![disc[0]];
        let mut i = 0;
        while i < seen.len() {
            let (x, y) = seen[i];
            for n in [
                (x + 1, y),
                (x.wrapping_sub(1), y),
                (x, y + 1),
                (x, y.wrapping_sub(1)),
                (x + 1, y + 1),
                (x + 1, y.wrapping_sub(1)),
                (x.wrapping_sub(1), y + 1),
                (x.wrapping_sub(1), y.wrapping_sub(1)),
            ] {
                if disc.contains(&n) && !seen.contains(&n) {
                    seen.push(n);
                }
            }
            i += 1;
        }
        assert_eq!(seen.len(), disc.len(), "the cylinder is one blob");
        let n = disc.len() as f64;
        let cx = disc.iter().map(|p| p.0 as f64).sum::<f64>() / n;
        let cy = disc.iter().map(|p| p.1 as f64).sum::<f64>() / n;
        assert!(
            (cx - 11.3).abs() <= 1.5 && (cy - 11.5).abs() <= 0.5,
            "centroid {cx}, {cy}"
        );
        let per_row = |y: usize| disc.iter().filter(|p| p.1 == y).count();
        for y in 0..12 {
            assert_eq!(per_row(y), per_row(23 - y), "rows {y} and {}", 23 - y);
        }
        assert!((0..24).filter(|&y| per_row(y) > 0).count() >= 4);
        for y in 1..h {
            assert!((0..6).all(|x| at(x, y).ch == ' '), "calm inflow, row {y}");
        }
        let right = |role: Role, rows: std::ops::Range<usize>| {
            rows.flat_map(|y| (cx.ceil() as usize + 5..w).map(move |x| (x, y)))
                .filter(|&(x, y)| at(x, y).ch != ' ' && at(x, y).ink.role == role)
                .count()
        };
        assert!(right(Role::Cold, 0..24) > 0 && right(Role::Hot, 0..24) > 0);
        assert!(
            right(Role::Cold, 0..12) > right(Role::Hot, 0..12),
            "upper row is clockwise"
        );
        assert!(
            right(Role::Hot, 12..24) > right(Role::Cold, 12..24),
            "lower row is counter-clockwise"
        );
        s.set_glyph_mode(GlyphMode::Ascii);
        assert!(s.render().to_lines().iter().all(|l| l.is_ascii()));
    }

    /// Same-sign eddies (`Hot` or `Cold` cells, 8-connected, at least 4 cells) whose
    /// centroid lies right of column `x0`: `(clockwise, counter-clockwise)`.
    fn eddies_right_of(frame: &Canvas, x0: f64) -> (usize, usize) {
        let (w, h) = (
            usize::from(frame.size().width()),
            usize::from(frame.size().height()),
        );
        let cells = frame.cells();
        let sign = |x: usize, y: usize| {
            let c = &cells[y * w + x];
            match c.ink.role {
                _ if c.ch == ' ' => None,
                Role::Cold => Some(false),
                Role::Hot => Some(true),
                _ => None,
            }
        };
        let mut seen = vec![vec![false; w]; h];
        let (mut cw, mut ccw) = (0, 0);
        for y0 in 1..h {
            for x in 0..w {
                let Some(s) = sign(x, y0).filter(|_| !seen[y0][x]) else {
                    continue;
                };
                let (mut stack, mut xs) = (vec![(x, y0)], Vec::new());
                seen[y0][x] = true;
                while let Some((cx, cy)) = stack.pop() {
                    xs.push(cx);
                    for dy in [-1i32, 0, 1] {
                        for dx in [-1i32, 0, 1] {
                            let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                            if !((0..w as i32).contains(&nx) && (1..h as i32).contains(&ny)) {
                                continue;
                            }
                            let (nx, ny) = (nx as usize, ny as usize);
                            if !seen[ny][nx] && sign(nx, ny) == Some(s) {
                                seen[ny][nx] = true;
                                stack.push((nx, ny));
                            }
                        }
                    }
                }
                let mean = xs.iter().sum::<usize>() as f64 / xs.len() as f64;
                if xs.len() >= 4 && mean > x0 {
                    *(if s { &mut ccw } else { &mut cw }) += 1;
                }
            }
        }
        (cw, ccw)
    }

    #[test]
    fn vor_a7_developed_street_shows_three_or_more_separate_eddies() {
        let p = flow_params(lattice_size(Size::new(80, 24)));
        // Lattice column → display column at 80×24 (the view spans columns 1 … W_vis − 1).
        let col = |x: f64| (x - VIEW_X0 as f64 + 0.5) * 80.0 / (p.w_vis - VIEW_X0) as f64 - 0.5;
        let x0 = col(p.xc + 3.0 * p.d);
        // Two independent runs, one thread each. Right of x_c + 3D the view holds ≈ 1.5
        // wavelengths, and the newest eddy stays joined to the shear layer for part of
        // each period, so the count depends on the phase: five frames span one shedding
        // period (≈ 1.6 s) from t = 20 s, each with separate eddies of both signs, and
        // ≥ 3 in at least one.
        std::thread::scope(|scope| {
            for seed in [42, 7] {
                scope.spawn(move || {
                    let mut s = Stage::new(build(), seed, Size::new(80, 24), GlyphMode::Unicode);
                    s.set_title_visible(false);
                    let mut three = 0;
                    for frame in 0..5 {
                        let steps = if frame == 0 { 1200 } else { 24 };
                        for _ in 0..steps {
                            s.step(SIM_DT);
                        }
                        let (cw, ccw) = eddies_right_of(s.render(), x0);
                        assert!(cw >= 1 && ccw >= 1, "seed {seed} frame {frame}: {cw}/{ccw}");
                        three += usize::from(cw + ccw >= 3);
                    }
                    assert!(three >= 1, "seed {seed}: never ≥ 3 separate eddies");
                });
            }
        });
    }

    #[test]
    fn vor_a9_vorticity_is_threads_not_fills() {
        let mut s = Stage::new(build(), 42, Size::new(200, 50), GlyphMode::Unicode);
        s.set_title_visible(false);
        for _ in 0..1200 {
            s.step(SIM_DT);
        }
        let (mut cells, mut dots, mut full) = (0usize, 0u32, 0usize);
        let mut levels = std::collections::BTreeSet::new();
        for c in s.render().cells() {
            if is_braille(c.ch) && matches!(c.ink.role, Role::Hot | Role::Cold) {
                let mask = c.ch as u32 - 0x2800;
                cells += 1;
                dots += mask.count_ones();
                full += usize::from(mask == 0xff);
                levels.insert(c.ink.level);
            }
        }
        assert!(cells > 100, "{cells} vorticity cells");
        let mean = f64::from(dots) / cells as f64;
        assert!(mean <= 4.5, "{mean:.2} dots per cell");
        assert!(full as f64 <= 0.15 * cells as f64, "{full} of {cells} full");
        assert_eq!(levels.into_iter().collect::<Vec<_>>(), LEVEL_INKS.to_vec());
    }

    #[test]
    fn vor_a8_inlet_breathes_slowly_within_the_laminar_band() {
        // breath(t) = 1 + 0.08 sin(2π t / 47 s).
        assert_eq!(breath(0.0), 1.0);
        assert!((breath(47.0 / 4.0) - 1.08).abs() < 1e-12);
        assert!((breath(3.0 * 47.0 / 4.0) - 0.92).abs() < 1e-12);
        for t in [3.0, 17.5, 40.0] {
            assert!((breath(t + 47.0) - breath(t)).abs() < 1e-12);
        }
        // Re stays in the 2-D laminar shedding band (onset ≈ 47, mode A ≈ 190) and the
        // peak inlet Mach number stays small, for every lattice.
        for w in (1..=480u16).step_by(7) {
            for h in (1..=160u16).step_by(3) {
                let p = flow_params(lattice_size(Size::new(w, h)));
                assert!(p.re * 0.92 > 47.0 && p.re * 1.08 < 190.0, "{w}x{h}");
                assert!(p.u * 1.08 * 3f64.sqrt() <= 0.234, "{w}x{h}");
            }
        }

        // The scene's inlet follows its own simulated time: after an update at t, the
        // centre-row inlet velocity is breath(t) times the unscaled one.
        let (mut g, mut ctx) = scene(Size::new(200, 50));
        let p = g.params.unwrap();
        let row = p.yc.round() as usize;
        let ux_in = |g: &VortexStreet| {
            let f: [f64; 9] =
                std::array::from_fn(|q| f64::from(W[q]) + f64::from(g.lat.inflow[row][q]));
            let (rho, jx, _) = moments(&f);
            jx / rho
        };
        let u0 = ux_in(&g);
        // The inlet carries the Bernoulli density of its potential flow, like the start
        // state (`init_street`), so it forces no pressure mismatch.
        for (y, inlet) in g.lat.inflow.iter().enumerate() {
            let f: [f64; 9] = std::array::from_fn(|q| f64::from(W[q]) + f64::from(inlet[q]));
            let (rho, jx, jy) = moments(&f);
            let (u, v) = base_flow(&p, -p.xc, p.yc - y as f64);
            let want = 1.0 + 1.5 * (p.u * p.u - (u * u + v * v));
            assert!((rho - want).abs() < 1e-6, "row {y}: ρ {rho} vs {want}");
            assert!((jx / rho - u).abs() < 1e-6 && (jy / rho + v).abs() < 1e-6);
        }
        for _ in 0..90 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let t = g.steps as f64 / STEPS_PER_S;
        assert!((t - 1.5).abs() < 0.01, "t = {t}");
        // Set once per update at its start, so it lags by one frame (≈ 2e-4).
        assert!((ux_in(&g) / u0 - breath(t)).abs() < 1e-3);
        let out = g.lat.outflow;
        let f: [f64; 9] = std::array::from_fn(|q| f64::from(W[q]) + f64::from(out[q]));
        assert!((moments(&f).1 / (p.u * breath(t)) - 1.0).abs() < 1e-3);
        assert_eq!(g.reinits, 0);
    }

    #[test]
    fn vor_a5_a8_strouhal_uses_the_inlet_speed_of_its_own_window() {
        // Five crossings 400 steps apart, oldest evicted: the last four periods count.
        let mut crossings = Trail::new(ST_INTERVALS + 1);
        assert_eq!(strouhal(&crossings, 10.0, |_| 0.1), None);
        for step in [200, 1000, 1400, 1800, 2200, 2600] {
            crossings.push(step);
        }
        // The inlet speed drifts through the window. St divides by its value at the
        // window's middle (step 1800), not at the latest crossing, so a period averaged
        // over ≈ 6 s is not biased by the breath's drift.
        let u_at = |step: f64| 0.1 * (1.0 + step / 10_000.0);
        let st = strouhal(&crossings, 10.0, u_at).expect("four periods");
        assert!(
            (st - 10.0 / (u_at(1800.0) * 400.0)).abs() < 1e-12,
            "St = {st}"
        );
        // Below three crossings there is no reading.
        let mut two = Trail::new(ST_INTERVALS + 1);
        two.push(100);
        two.push(500);
        assert_eq!(strouhal(&two, 10.0, u_at), None);
    }

    #[test]
    fn vor_a8_label_reads_re_and_then_st() {
        let re_of = |row: &str| -> u32 {
            let digits: String = row
                .trim_start()
                .trim_start_matches("Re≈")
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().unwrap_or(0)
        };
        let mut s = Stage::new(build(), 42, Size::new(80, 24), GlyphMode::Unicode);
        // Re₀ = 137.5 exactly (not 137.4999… from ν's f64 round-off), so the first
        // number the viewer reads is 140 and it does not flip as the breath rises.
        assert_eq!(flow_params(lattice_size(Size::new(80, 24))).re, 137.5);
        let row = s.render().to_lines()[0].clone();
        assert!(row.starts_with(" Re≈1"), "{row}");
        assert_eq!(re_of(&row), 140, "{row}");
        for _ in 0..30 {
            s.step(SIM_DT);
        }
        let row = s.render().to_lines()[0].clone();
        assert_eq!(re_of(&row), 140, "t = 0.5 s: {row}");
        for _ in 0..570 {
            s.step(SIM_DT);
        }
        let row = s.render().to_lines()[0].clone();
        assert!(row.contains("  St≈0."), "{row}");
        // The label shows St on the gap speed, St·(1 − D/H), comparable with
        // Williamson's unconfined St ≈ 0.18 at Re 140–150 (free-stream basis: ≈ 0.21).
        let st: f64 = row
            .split("St≈")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        assert!((0.16..=0.19).contains(&st), "St {st}: {row}");
        // At t = 10 s the inlet runs at breath(10) ≈ 1.078 U₀: Re 137.5 → ≈ 148.
        assert_eq!(re_of(&row), 150, "{row}");
        s.resize(Size::new(15, 24));
        assert!(!s.render().to_lines()[0].contains("Re"));
        s.resize(Size::new(39, 24));
        s.set_glyph_mode(GlyphMode::Ascii);
        let row = s.render().to_lines()[0].clone();
        assert!(row.starts_with(" Re~") && !row.contains("St"), "{row}");
    }
}
