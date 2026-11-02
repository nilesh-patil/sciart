//! Self-replicating spots and worms from the Gray–Scott reaction–diffusion system
//! `U + 2V → 3V, V → P` (SPEC-002 `gray_scott`).
//!
//! Model (Pearson 1993, dimensionless, grid spacing 1 lattice node):
//!
//! ```text
//! ∂u/∂t = D_u ∇²u − u v² + F (1 − u)
//! ∂v/∂t = D_v ∇²v + u v² − (F + k) v          D_u = 2 D_v
//! ```
//!
//! on a torus of square nodes decoupled from the display (ADR-0012): below the node cap
//! one node covers 2×2 braille dots, above it the lattice stops growing and the pattern
//! zooms.
//!
//! ## Look (M17, GEN-R26: stipple cells + rim)
//!
//! Every braille dot samples `u` and `v` bilinearly from the torus (wrapping across the
//! screen edges, so the lattice never shows), which keeps the cells round at any zoom.
//! Inside a cell (`v ≥ v_m`, [`V_M`]) a dot is kept with probability rising smoothly with
//! `v` ([`cell_p`]), as electron_orbitals keeps a dot with probability |ψ|²; the core band
//! is `Sun`, the body `Hot`. The membrane is the `v = v_m` iso-line of the same sampled
//! surface at dot resolution, one `Accent` thread ([`draw_culture`]), so it closes around
//! every cell and pinches into a figure-8 when a cell divides. Outside, the substrate the
//! cells have eaten (`1 − u`) is a sparse `Cold` stipple, the moat. Thresholds are fixed
//! per dot (CORE-R107's hash, cached), and both densities are monotone in the field, so a
//! slowly changing culture gains and loses dots only where its density crosses them. Node
//! cells that hold neither a cell nor a moat are skipped whole, so the black majority
//! costs nothing, and the keep decisions are accumulated without a branch per dot.
//!
//! ## Discretisation
//!
//! ∇² is the isotropic 9-point stencil `(4·Σ_edge + Σ_corner − 20·c) / 6`
//! (Patra–Karttunen), the same as `grid::laplacian9`: its leading error term is
//! rotation-invariant, so spots stay round instead of drifting into diamonds as they do
//! with the 5-point stencil. Its most negative Fourier symbol is `−16/3` (checkerboard),
//! versus `−8` for 5-point. It is fused into the Euler update here, so a sub-step is one
//! branch-free pass over u and v with no Laplacian grids, and a test checks it against the
//! core function.
//!
//! Time stepping is explicit Euler with sub-step `τ` ([`TAU_SUB`]). Written as a weighted
//! sum, the centre weight of `u` is `1 − τ(10/3·D_u + v² + F)` and every neighbour weight
//! is positive, so if
//!
//! ```text
//! τ · (10/3 · D_u + 1 + F) ≤ 1        (v ≤ 1)        and   τ · (10/3 · D_v + F + k) ≤ 1
//! ```
//!
//! the step is a convex combination plus non-negative sources, so `u, v ≥ 0` and `u ≤ 1`
//! hold exactly; decaying diffusive modes stay decaying (`τ·D·16/3 ≪ 2`). Both bounds are
//! `const`-asserted for every preset and checked along the whole walk (CHEM-A5). The
//! kernel still clamps to `[0, 1]` as a guard; in normal runs `v` peaks at ≈ 0.4–0.63 (the
//! top is loops colliding) and the clamp never fires, which the whole-walk test checks by
//! applying the unclamped update to the live fields (CHEM-A1).
//!
//! ## Preset walk
//!
//! A looping [`Timeline`] of five holds and five 6–8 s smoothstep morphs through Pearson's
//! (F, k) plane (CHEM-A5), chosen for regimes that keep a dark majority and keep moving
//! (CHEM-A8). A morph does not follow the straight chord between two presets: a chord can
//! bulge across the saddle-node curve `F = 4(F + k)²` into the region where the uniform
//! `v₊` sheet is stable, and the culture would go featureless. Instead `F` is interpolated
//! and `k` keeps the endpoints' interpolated signed offset from the curve
//! `k_sn(F) = √F/2 − F`, so the culture reorganises rather than dying and being reseeded.
//!
//! Each preset has its own reaction rate (reaction time units per simulated second),
//! lerped by the same morph weight; τ stays fixed, so only the number of sub-steps per
//! frame changes. On the return to mitosis the culture is passaged: a seeded block mask
//! clears most of the moving spots, so the mitosis hold has room to divide again instead
//! of inheriting a jammed field.

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::grid;
use sciart_core::prelude::*;
use sciart_core::{draw, ease, raster};

pub static META: Meta = Meta {
    name: "gray_scott",
    category: Category::Chemistry,
    description: "Gray-Scott reaction-diffusion growing self-replicating spots and worms",
    title: "Spots and worms · react, spread",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(GrayScott::new(None))
}

/// Diffusion of the autocatalyst V, nodes² per reaction time unit. Pearson's value (1e-5 at
/// h = 0.01, i.e. 0.1 here) gives ~10-node spots, too coarse for a terminal; the pattern
/// wavelength scales as √D, so 0.03 gives ~5-node spots.
const D_V: f32 = 0.03;
/// D_u = 2 D_v (SPEC, Pearson 1993): the substrate must outrun the autocatalyst for spots to form.
const D_U: f32 = 2.0 * D_V;
/// Explicit Euler sub-step, reaction time units (bound derived in the module docs).
const TAU_SUB: f32 = 0.5;
/// The fastest preset's reaction time units per simulated second (see [`Preset::rate`]).
const MAX_RATE: f64 = 400.0;
/// Hard cap on sub-steps per `update`, matching the largest contract `dt` of 0.25 s at the
/// fastest rate.
const MAX_SUBSTEPS: usize = (0.25 * MAX_RATE / TAU_SUB as f64) as usize;
/// Sub-steps run in the first `resize`, so the opening frame already shows graded bodies
/// instead of the raw seed bumps.
const PREROLL_SUBSTEPS: usize = 200;
/// How often the field is checked for death / uniformity, simulated seconds.
const CHECK_EVERY: f64 = 1.0;
/// Lattice node cap and aspect bucket (ADR-0012), and the floor that keeps a culture
/// alive through tiny transient sizes and holds enough wavelengths to avoid locking into
/// a torus mode.
const MAX_NODES: usize = 5_120;
const BUCKET: f64 = 1.12;
const MIN_LATTICE: (u16, u16) = (32, 16);
/// Seed bumps: three plus one per this many nodes, with Gaussian radius `r0` in nodes.
const SEED_NODES_PER_BUMP: usize = 300;
const SEED_R0: (f32, f32) = (1.5, 3.0);
/// Passaging on the return to mitosis (CHEM-A8): during the spots → mitosis morph an extra
/// sink `−κ·m²·mask·v` (κ per reaction unit, m the morph's smoothstep) clears the nodes
/// under a seeded mask of `PASSAGE_BLOCK`-node blocks, each active with probability
/// `PASSAGE_SHARE`. About four in five moving spots fade over the morph's second half, so
/// the mitosis hold inherits room to divide instead of a jammed field, as a culture split
/// into fresh medium. Tuning measurements are in SPEC-002 `gray_scott`.
const PASSAGE_KAPPA: f32 = 0.02;
const PASSAGE_BLOCK: usize = 6;
const PASSAGE_SHARE: f64 = 0.8;
/// The membrane level `v_m`: a cell is where `v ≥ v_m`. Patterned v peaks at ≈ 0.3–0.45;
/// below 0.12 lie the thin tails between neighbouring spots and loops, which stay
/// substrate, so dense regimes keep a black majority (the M15 pattern edge, CHEM-A8).
const V_M: f32 = 0.12;
/// v where a cell's stipple reaches its core density, and where the `Sun` core band
/// starts. Spot cores reach ≈ 0.35–0.45; coral plateaus sit at ≈ 0.3–0.37.
const V_CORE: f32 = 0.30;
const V_SUN: f32 = 0.28;
/// Kept share of cell dots at the membrane and at the core (SPEC-002 Look (M17)).
const P_EDGE: f64 = 0.08;
const P_CORE: f64 = 0.45;
/// Substrate depletion `1 − u` where the moat stipple starts and where it reaches its
/// cap, and the cap: sparse, so the moat only hints at the eaten medium.
const MOAT_D0: f32 = 0.15;
const MOAT_D1: f32 = 0.45;
const P_MOAT: f64 = 0.06;
/// Stipple salt (CORE-R107). The cell and moat clouds never share a dot (a dot is inside
/// or outside), so one threshold per dot serves both.
const SALT: u64 = 0x6A09_E667_F3BC_C908;

/// Each dot's fixed stipple threshold: CORE-R107's `hash01(salt ^ cell_key(x, y))`,
/// quantised to 1/256 and cached in `resize`, since the raster tests up to every dot of
/// the screen each frame. A dot is kept when its threshold is below `256·p`.
pub(crate) fn threshold(x: i32, y: i32) -> u8 {
    (sciart_core::rng::hash01(SALT ^ sciart_core::rng::cell_key(x, y)) * 256.0) as u8
}
/// Cell body, core band, membrane and moat inks. A braille cell takes its most intense
/// dot's ink, so the levels rank them: the rim wins over the stipple it crosses, the core
/// over the body, and the moat never tints a cell dot.
const CELL_INK: Ink = Ink::new(Role::Hot, 215);
const SUN_INK: Ink = Ink::new(Role::Sun, 255);
const RIM_INK: Ink = Ink::new(Role::Accent, 255);
const MOAT_INK: Ink = Ink::new(Role::Cold, 185);
/// The label shows the preset name only from this width on: narrower, it would take most
/// of row 0 and cut a notch into the pattern.
const LABEL_NAME_MIN: u16 = 56;
/// The corner label, inside a blank gutter (level kept from M15).
const LABEL_INK: Ink = Ink::new(Role::Text, 195);

/// Feed rate F and kill rate k.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Coeffs {
    f: f32,
    k: f32,
}

/// One stop of the walk: label, (F, k), reaction rate, and the lengths of its hold and of
/// the morph that follows it, in simulated seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Preset {
    name: &'static str,
    c: Coeffs,
    /// Reaction time units per simulated second. Mitosis and coral run fast, so spots
    /// visibly bud and divide within seconds; the moving regimes evolve far faster in
    /// reaction time and run slower, so they jostle like organisms instead of boiling
    /// (CHEM-A8; measurements in SPEC-002).
    rate: f64,
    hold: f64,
    morph: f64,
}

const fn preset(name: &'static str, f: f32, k: f32, rate: f64, hold: f64, morph: f64) -> Preset {
    Preset {
        name,
        c: Coeffs { f, k },
        rate,
        hold,
        morph,
    }
}

/// The walk through Pearson's (F, k) plane, in order (CHEM-A5, CHEM-A8). Mitosis (Sims)
/// shows division until the spots fill the torus and jam, so its hold is short and later
/// returns are passaged ([`PASSAGE_KAPPA`]). Coral (Sims) is the one labyrinth beat: it
/// branches out of the mitosis spots during the morph and leaves before the labyrinth
/// fills the torus and freezes, so its hold and the morphs around it are short and it is
/// never a start slot. Loops, swarm and moving spots (Márquez-Neila's "spots and loops"
/// and "moving spots", with swarm between them) sit just right of the saddle-node curve
/// at low F; they keep a dark majority and never stop moving. Timings and the rejected
/// regimes are measured in SPEC-002 `gray_scott`.
pub(crate) const PRESETS: [Preset; 5] = [
    preset("mitosis", 0.0367, 0.0649, 400.0, 8.0, 6.0),
    preset("coral", 0.0545, 0.062, 400.0, 3.0, 6.0),
    preset("loops", 0.018, 0.051, 70.0, 24.0, 8.0),
    preset("swarm", 0.016, 0.052, 110.0, 24.0, 8.0),
    preset("spots", 0.014, 0.054, 150.0, 24.0, 8.0),
];

/// The preset whose morph back to mitosis is passaged (see [`PASSAGE_KAPPA`]).
const PASSAGE_FROM: usize = PRESETS.len() - 1;

/// Holds a visit may open in: every preset except coral.
pub(crate) const START_SLOTS: [usize; 4] = [0, 2, 3, 4];

// Positivity of the explicit sub-step for every preset (derivation in the module docs),
// checked at compile time: the centre weights of u (with v ≤ 1) and of v stay
// non-negative. CHEM-A5 checks the same bounds along every morph.
const _: () = {
    let mut i = 0;
    while i < PRESETS.len() {
        let c = PRESETS[i].c;
        assert!(TAU_SUB * (10.0 / 3.0 * D_U + 1.0 + c.f) <= 1.0);
        assert!(TAU_SUB * (10.0 / 3.0 * D_V + c.f + c.k) <= 1.0);
        assert!(PRESETS[i].rate > 0.0 && PRESETS[i].rate <= MAX_RATE);
        i += 1;
    }
};

/// One explicit Euler sub-step of length [`TAU_SUB`] from `(u, v)` into `(un, vn)`, periodic
/// in both axes. `REACT = false` switches off every non-diffusive term (CHEM-A3).
///
/// Row-wise sweep: the wrap is resolved once per row (`ym`, `yp`), the interior columns
/// read their west / centre / east neighbours as three offset slices of equal length (a
/// branch-free loop that vectorises), and only the two edge columns wrap.
pub(crate) fn gs_step<const REACT: bool>(
    w: usize,
    h: usize,
    u: &[f32],
    v: &[f32],
    un: &mut [f32],
    vn: &mut [f32],
    c: Coeffs,
) {
    let n = w * h;
    if n == 0 || u.len() < n || v.len() < n || un.len() < n || vn.len() < n {
        return;
    }
    let (cu, cv) = (TAU_SUB * D_U / 6.0, TAU_SUB * D_V / 6.0);
    let (tf, tfk) = (TAU_SUB * c.f, TAU_SUB * (c.f + c.k));
    // One node from its 3×3 neighbourhood, given as (west, centre, east) triples of the
    // rows above, at and below it.
    let node = |ua: [f32; 9], va: [f32; 9]| {
        let lap6 = |r: [f32; 9]| {
            4.0 * (r[3] + r[5] + r[1] + r[7]) + r[0] + r[2] + r[6] + r[8] - 20.0 * r[4]
        };
        let (a, b) = (ua[4], va[4]);
        let mut nu = a + cu * lap6(ua);
        let mut nv = b + cv * lap6(va);
        if REACT {
            let uvv = TAU_SUB * a * b * b;
            nu += tf * (1.0 - a) - uvv;
            nv += uvv - tfk * b;
        }
        // Guard only (see module docs): the bounds keep u, v ≥ 0 and u ≤ 1 already.
        (nu.clamp(0.0, 1.0), nv.clamp(0.0, 1.0))
    };
    for y in 0..h {
        let (ym, yp) = (
            if y == 0 { h - 1 } else { y - 1 },
            if y + 1 == h { 0 } else { y + 1 },
        );
        let (ur, vr) = (rows(u, w, [ym, y, yp]), rows(v, w, [ym, y, yp]));
        let (uo, vo) = (&mut un[y * w..][..w], &mut vn[y * w..][..w]);
        // Neighbourhood of column x with explicit west / east columns.
        let hood = |r: [&[f32]; 3], xm: usize, x: usize, xp: usize| {
            [
                r[0][xm], r[0][x], r[0][xp], r[1][xm], r[1][x], r[1][xp], r[2][xm], r[2][x],
                r[2][xp],
            ]
        };
        for x in [0, w - 1] {
            let (xm, xp) = ((x + w - 1) % w, (x + 1) % w);
            (uo[x], vo[x]) = node(hood(ur, xm, x, xp), hood(vr, xm, x, xp));
        }
        if w < 3 {
            continue;
        }
        let m = w - 2;
        let (us, vs) = (shifted(ur, m), shifted(vr, m));
        let (uo, vo) = (&mut uo[1..][..m], &mut vo[1..][..m]);
        for k in 0..m {
            let at = |s: [[&[f32]; 3]; 3]| {
                [
                    s[0][0][k], s[0][1][k], s[0][2][k], s[1][0][k], s[1][1][k], s[1][2][k],
                    s[2][0][k], s[2][1][k], s[2][2][k],
                ]
            };
            (uo[k], vo[k]) = node(at(us), at(vs));
        }
    }
}

/// Rows `ys` of a row-major field of width `w`.
fn rows(f: &[f32], w: usize, ys: [usize; 3]) -> [&[f32]; 3] {
    ys.map(|y| &f[y * w..][..w])
}

/// Each row as its (west, centre, east) windows of length `m` for columns `1..=m`.
fn shifted(r: [&[f32]; 3], m: usize) -> [[&[f32]; 3]; 3] {
    r.map(|row| [&row[..m], &row[1..][..m], &row[2..][..m]])
}

const WALK_BEATS: usize = 2 * PRESETS.len();

/// The preset walk: hold `i`, morph `i → i + 1`, … for every preset (119 s).
fn walk() -> Timeline {
    let beats: [(&'static str, f64); WALK_BEATS] = std::array::from_fn(|b| {
        let p = PRESETS[b / 2];
        (p.name, if b % 2 == 0 { p.hold } else { p.morph })
    });
    Timeline::new(&beats)
}

/// Walk time at which preset `slot` starts to hold.
fn slot_start(slot: usize) -> f64 {
    PRESETS[..slot].iter().map(|p| p.hold + p.morph).sum()
}

/// Pearson's saddle-node curve `F = 4(F + k)²` solved for k.
pub(crate) fn k_saddle_node(f: f64) -> f64 {
    0.5 * f.max(0.0).sqrt() - f
}

/// Where the walk is at one instant (CHEM-A5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WalkPoint {
    c: Coeffs,
    /// Reaction time units per simulated second.
    rate: f64,
    /// The presets being blended and the blend weight `m` (0 in a hold).
    from: usize,
    to: usize,
    m: f64,
}

/// `(F, k)` and the rate at walk time `t`, plus the presets being blended and the blend
/// weight `m` (0 in a hold, the smoothstep of the morph's progress otherwise) (CHEM-A5).
/// A morph interpolates F and the signed offset `k − k_sn(F)` from the saddle-node curve,
/// and the rate linearly, by the same `m`.
pub(crate) fn preset_at(walk: &Timeline, t: f64) -> WalkPoint {
    let beat = walk.at(t);
    let from = beat.index / 2;
    if beat.index % 2 == 0 {
        let p = PRESETS[from];
        return WalkPoint {
            c: p.c,
            rate: p.rate,
            from,
            to: from,
            m: 0.0,
        };
    }
    let to = (from + 1) % PRESETS.len();
    let m = ease::smoothstep(beat.progress);
    let (a, b) = (PRESETS[from].c, PRESETS[to].c);
    let (fa, fb) = (f64::from(a.f), f64::from(b.f));
    let offset = |c: Coeffs, f: f64| f64::from(c.k) - k_saddle_node(f);
    let f = ease::lerp(fa, fb, m);
    let k = k_saddle_node(f) + ease::lerp(offset(a, fa), offset(b, fb), m);
    let c = if m >= 1.0 {
        b
    } else if m <= 0.0 {
        a
    } else {
        Coeffs {
            f: f as f32,
            k: k as f32,
        }
    };
    WalkPoint {
        c,
        rate: ease::lerp(PRESETS[from].rate, PRESETS[to].rate, m),
        from,
        to,
        m,
    }
}

/// The passaging mask (CHEM-A8): 1 on the nodes of the active blocks, else 0. Blocks of
/// about [`PASSAGE_BLOCK`] nodes tile the torus exactly; a block is active when a
/// SplitMix64 draw keyed by `seed` and the block's index falls below [`PASSAGE_SHARE`],
/// so the mask is a pure function of the lattice and the seed and is rebuilt identically
/// on a regrid.
pub(crate) fn passage_mask(lattice: Size, seed: u64) -> Grid<f32> {
    let (w, h) = (usize::from(lattice.width()), usize::from(lattice.height()));
    let mut mask = Grid::new(lattice, 0.0);
    let blocks = |n: usize| (n as f64 / PASSAGE_BLOCK as f64).round().max(1.0) as usize;
    let (bw, bh) = (blocks(w), blocks(h));
    for (i, m) in mask.as_mut_slice().iter_mut().enumerate() {
        let (bx, by) = ((i % w) * bw / w, (i / w) * bh / h);
        let key = (by * bw + bx) as u64;
        let mut r = SplitMix64::new(seed ^ key.wrapping_mul(0xD6E8_FEB8_6659_FD93));
        if r.next_f64() < PASSAGE_SHARE {
            *m = 1.0;
        }
    }
    mask
}

/// The simulation lattice for a display (ADR-0012, CHEM-A6): one node per half-block
/// pixel (square, so still isotropic), at least [`MIN_LATTICE`], capped at [`MAX_NODES`].
pub(crate) fn lattice_size(display: Size) -> Size {
    let w = display.width().max(MIN_LATTICE.0);
    let h = display.height().saturating_mul(2).max(MIN_LATTICE.1);
    let l = grid::lattice_for(Size::new(w, h), MAX_NODES, BUCKET);
    // On extreme aspects the snap can undercut the floor; restore it inside the cap. With
    // h ≤ cap/32 and h ≥ 16, the width bound cap/h lies in [32, 320].
    let cap = |other: u16| u16::try_from(MAX_NODES / usize::from(other)).unwrap_or(u16::MAX);
    let h = l.height().clamp(MIN_LATTICE.1, cap(MIN_LATTICE.0));
    Size::new(l.width().max(MIN_LATTICE.0).min(cap(h)), h)
}

/// Kept share of the cell stipple at `v` (Look (M17), CHEM-A10): 0 outside a cell
/// (`v < v_m`, or NaN), [`P_EDGE`] at the membrane rising by a smoothstep to [`P_CORE`] at
/// [`V_CORE`]. Monotone in `v`, so a slowly changing cell only gains or loses dots where
/// it crosses their fixed thresholds.
pub(crate) fn cell_p(v: f32) -> f64 {
    if v.is_nan() || v < V_M {
        return 0.0;
    }
    let t = f64::from(((v - V_M) * (1.0 / (V_CORE - V_M))).min(1.0));
    P_EDGE + (P_CORE - P_EDGE) * t * t
}

/// Steps of the cell-density table over `v_m ..= V_CORE` (the raster's hot path reads
/// `256·cell_p` from it instead of evaluating [`cell_p`] per dot).
const P_STEPS: usize = 128;

/// `256·cell_p(v)` at `v = v_m + i·(V_CORE − v_m)/(P_STEPS − 1)`, rounded: the dot with
/// threshold `t` is kept where `t < table[i]`.
fn cell_table() -> [u8; P_STEPS] {
    std::array::from_fn(|i| {
        let v = V_M + (V_CORE - V_M) * i as f32 / (P_STEPS - 1) as f32;
        (256.0 * cell_p(v)).round().min(255.0) as u8
    })
}

/// The table index of `v ≥ v_m`.
#[inline]
fn cell_step(v: f32) -> usize {
    (((v - V_M) * ((P_STEPS - 1) as f32 / (V_CORE - V_M))) as usize).min(P_STEPS - 1)
}

/// Kept share of the moat stipple at substrate `u` (outside cells only): 0 up to a
/// depletion `1 − u` of [`MOAT_D0`], rising linearly to the cap [`P_MOAT`] at [`MOAT_D1`].
pub(crate) fn moat_p(u: f32) -> f64 {
    let d = (1.0 - u - MOAT_D0) * (1.0 / (MOAT_D1 - MOAT_D0));
    P_MOAT * f64::from(d.clamp(0.0, 1.0))
}

/// The dots of one screen axis that fall between two neighbouring nodes: lattice cell
/// `cell` (−1 for the strip before node 0's centre, which wraps to the last node), its two
/// nodes on the torus with the nodes either side of them (`nm`, `n0`, `n1`, `np`, the
/// cubic's four), and the dot range `d0..d1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Span {
    cell: i32,
    nm: usize,
    n0: usize,
    n1: usize,
    np: usize,
    d0: i32,
    d1: i32,
}

/// One dot's position between its two nodes: the fraction `t` (for bilinear `u`) and the
/// uniform cubic B-spline weights of the four nodes `nm, n0, n1, np` at `t` (for `v`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tap {
    t: f32,
    w: [f32; 4],
}

impl Tap {
    fn new(t: f32) -> Self {
        let s = 1.0 - t;
        let (t2, t3) = (t * t, t * t * t);
        Self {
            t,
            w: [
                s * s * s / 6.0,
                (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0,
                (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0,
                t3 / 6.0,
            ],
        }
    }
}

/// One screen axis of `dots` dots over `nodes` torus nodes, node `n` centred at dot
/// `(n + ½)·s − ½` (`s = dots / nodes`): each dot's [`Tap`] into `taps`, and the runs of
/// dots between the same two nodes into `spans`. Built in `resize` (the vectors are
/// reused).
fn build_axis(nodes: usize, dots: usize, taps: &mut Vec<Tap>, spans: &mut Vec<Span>) {
    taps.clear();
    spans.clear();
    if nodes == 0 || dots == 0 {
        return;
    }
    let s = dots as f64 / nodes as f64;
    let n = nodes as i32;
    for d in 0..dots as i32 {
        let x = (f64::from(d) + 0.5) / s - 0.5;
        let cell = x.floor();
        taps.push(Tap::new((x - cell) as f32));
        let cell = cell as i32;
        match spans.last_mut() {
            Some(span) if span.cell == cell => span.d1 = d + 1,
            _ => spans.push(Span {
                cell,
                nm: (cell - 1).rem_euclid(n) as usize,
                n0: cell.rem_euclid(n) as usize,
                n1: (cell + 1).rem_euclid(n) as usize,
                np: (cell + 2).rem_euclid(n) as usize,
                d0: d,
                d1: d + 1,
            }),
        }
    }
}

/// Which layers [`draw_culture`] draws (tests draw them one at a time).
pub(crate) const CELLS: u8 = 1;
pub(crate) const RIM: u8 = 2;
pub(crate) const MOAT: u8 = 4;
pub(crate) const ALL_LAYERS: u8 = CELLS | RIM | MOAT;

/// The fields and the lattice → dot mapping, borrowed for one raster.
pub(crate) struct Culture<'a> {
    w: usize,
    u: &'a [f32],
    v: &'a [f32],
    fx: &'a [Tap],
    fy: &'a [Tap],
    xs: &'a [Span],
    ys: &'a [Span],
    /// Each dot's stipple threshold ([`threshold`]), row-major over the dot lattice.
    thr: &'a [u8],
    /// `256·cell_p` by [`cell_step`].
    table: [u8; P_STEPS],
}

/// The culture of fields `u`, `v` through the axes built by [`build_axis`] and the cached
/// thresholds. A free function so it borrows only these fields, leaving the scene's dots
/// free to draw on.
fn culture<'a>(
    u: &'a Grid<f32>,
    v: &'a Grid<f32>,
    [fx, fy]: [&'a [Tap]; 2],
    [xs, ys]: [&'a [Span]; 2],
    thr: &'a [u8],
) -> Culture<'a> {
    Culture {
        w: usize::from(v.width()),
        u: u.as_slice(),
        v: v.as_slice(),
        fx,
        fy,
        xs,
        ys,
        thr,
        table: cell_table(),
    }
}

/// What a stipple cell holds, as bits: a moat dot, a body dot, a core dot.
const HAS_MOAT: u8 = 1;
const HAS_BODY: u8 = 2;
const HAS_CORE: u8 = 4;

/// The cell and moat stipple as braille: per terminal cell a dot mask and the kinds of dot
/// it holds, accumulated without a branch per dot (the keep decisions are random by
/// design, so branching on them mispredicts half the time). Blitted as `Dots::blit` does,
/// each cell in the ink of its brightest kind (core, then body, then moat).
#[derive(Debug)]
pub(crate) struct Stipple {
    cells: Size,
    masks: Vec<u8>,
    kinds: Vec<u8>,
}

impl Default for Stipple {
    /// Empty until the first `resize`.
    fn default() -> Self {
        Self {
            cells: Size::new(0, 0),
            masks: Vec::new(),
            kinds: Vec::new(),
        }
    }
}

impl Stipple {
    fn resize(&mut self, cells: Size) {
        self.cells = cells;
        for v in [&mut self.masks, &mut self.kinds] {
            v.clear();
            v.resize(cells.area(), 0);
        }
    }

    fn clear(&mut self) {
        self.masks.fill(0);
        self.kinds.fill(0);
    }

    /// Light dot `(x, y)` (on the lattice) if `keep`, as `kind`.
    #[inline]
    fn put(&mut self, x: i32, y: i32, keep: bool, kind: u8) {
        let i = (y / 4) as usize * usize::from(self.cells.width()) + (x / 2) as usize;
        let k = u8::from(keep);
        self.masks[i] |= k * raster::dot_bit(x, y);
        self.kinds[i] |= k * kind;
    }

    /// Whether dot `(x, y)` is lit.
    #[cfg(test)]
    pub(crate) fn is_lit(&self, x: i32, y: i32) -> bool {
        let (w, h) = (
            i32::from(self.cells.width()),
            i32::from(self.cells.height()),
        );
        (0..2 * w).contains(&x)
            && (0..4 * h).contains(&y)
            && self.masks[(y / 4) as usize * w as usize + (x / 2) as usize] & raster::dot_bit(x, y)
                != 0
    }

    fn blit(&self, canvas: &mut Canvas) {
        let w = usize::from(self.cells.width());
        for (i, (&mask, &kind)) in self.masks.iter().zip(&self.kinds).enumerate() {
            if mask != 0 {
                let ink = if kind & HAS_CORE != 0 {
                    SUN_INK
                } else if kind & HAS_BODY != 0 {
                    CELL_INK
                } else {
                    MOAT_INK
                };
                canvas.put((i % w) as i32, (i / w) as i32, raster::braille(mask), ink);
            }
        }
    }
}

/// The display filter: `v` smoothed by the separable binomial `[1, 2, 1]/4` on the torus
/// into `out` (through `tmp`), a point-spread of about half a node. With the B-spline
/// sampling ([`for_each_dot`]) it rounds the corners a coarse lattice leaves on young,
/// growing spots, as a microscope's optics would; the simulation itself is untouched.
/// Mismatched sizes leave `out` as it is.
pub(crate) fn soften(v: &Grid<f32>, tmp: &mut Grid<f32>, out: &mut Grid<f32>) {
    let (w, h) = (usize::from(v.width()), usize::from(v.height()));
    if w == 0 || h == 0 || tmp.size() != v.size() || out.size() != v.size() {
        return;
    }
    let (src, t) = (v.as_slice(), tmp.as_mut_slice());
    for (row, out) in src.chunks_exact(w).zip(t.chunks_exact_mut(w)) {
        for x in 0..w {
            let (l, r) = (row[(x + w - 1) % w], row[(x + 1) % w]);
            out[x] = 0.25 * (l + r) + 0.5 * row[x];
        }
    }
    let o = out.as_mut_slice();
    for y in 0..h {
        let (up, dn) = (((y + h - 1) % h) * w, ((y + 1) % h) * w);
        for x in 0..w {
            o[y * w + x] = 0.25 * (t[up + x] + t[dn + x]) + 0.5 * t[y * w + x];
        }
    }
}

/// Words of an `inside` bitset row over `dots` dots (one bit per dot, see [`draw_culture`]).
pub(crate) fn row_words(dots: usize) -> usize {
    dots.div_ceil(64)
}

/// Visit the dots of every node cell that `keep(vmax, umin)` accepts — `vmax` the largest
/// `v` of the cell's 4×4 cubic support (which bounds every sampled `v` in it, the B-spline
/// weights being a partition of unity), `umin` the smallest `u` of its four corners (which
/// bounds the bilinear `u`) — calling `f(x, y, v, u)` with `v` sampled by the uniform
/// cubic B-spline and `u` bilinearly. The B-spline is C², so iso-lines of `v` are round
/// even where a lattice node spans several dots; bilinear sampling bent them along the
/// lattice and cells read as rounded squares.
pub(crate) fn for_each_dot(
    c: &Culture<'_>,
    keep: impl Fn(f32, f32) -> bool,
    mut f: impl FnMut(i32, i32, f32, f32),
) {
    let (u, v, w) = (c.u, c.v, c.w);
    if w == 0 || v.len() < w || u.len() < v.len() {
        return;
    }
    for sy in c.ys {
        let rows = [sy.nm, sy.n0, sy.n1, sy.np].map(|n| n * w);
        for sx in c.xs {
            let cols = [sx.nm, sx.n0, sx.n1, sx.np];
            let node = rows.map(|r| cols.map(|i| v[r + i]));
            let vmax = node
                .iter()
                .flatten()
                .fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            let uc = [
                u[rows[1] + cols[1]],
                u[rows[1] + cols[2]],
                u[rows[2] + cols[2]],
                u[rows[2] + cols[1]],
            ];
            let umin = uc.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            if !keep(vmax, umin) {
                continue;
            }
            for y in sy.d0..sy.d1 {
                let ty = c.fy[y as usize];
                // The four columns combined down the rows once per dot row.
                let col: [f32; 4] =
                    std::array::from_fn(|i| (0..4).map(|j| ty.w[j] * node[j][i]).sum::<f32>());
                let ul = uc[0] + (uc[3] - uc[0]) * ty.t;
                let ur = uc[1] + (uc[2] - uc[1]) * ty.t;
                for x in sx.d0..sx.d1 {
                    let tx = c.fx[x as usize];
                    let s =
                        tx.w[0] * col[0] + tx.w[1] * col[1] + tx.w[2] * col[2] + tx.w[3] * col[3];
                    f(x, y, s, ul + (ur - ul) * tx.t);
                }
            }
        }
    }
}

/// Draw the culture's `layers` (Look (M17)): the cell and moat stipple onto `stipple`, the
/// membrane onto `rim`. Node cells holding neither a cell nor a moat are skipped whole;
/// every other dot is sampled by [`for_each_dot`] and stippled.
///
/// The membrane is the `v = v_m` iso-line at dot resolution: the inside dots (`v ≥ v_m`,
/// one bit each in `inside`, a row-major bitset of [`row_words`] words per dot row) with a
/// 4-neighbour outside. That boundary is one dot thick, 8-connected and closed around
/// every cell by construction (screen edges count as inside, so a cell cut by the edge
/// gets no false rim there), and a dot joins or leaves it only when the iso-line crosses
/// the dot itself: the least churn a thread can have.
pub(crate) fn draw_culture(
    stipple: &mut Stipple,
    rim: &mut Dots,
    inside: &mut [u64],
    c: &Culture<'_>,
    layers: u8,
) {
    let (dw, dh) = (c.fx.len(), c.fy.len());
    let words = row_words(dw);
    if inside.len() < words * dh || c.thr.len() < dw * dh || stipple.masks.len() * 8 < dw * dh {
        return;
    }
    let inside = &mut inside[..words * dh];
    let trace = layers & RIM != 0;
    if trace {
        // Bits past the last dot of a row count as inside, like the screen edge.
        let pad = if dw % 64 == 0 { 0 } else { !0u64 << (dw % 64) };
        for row in inside.chunks_exact_mut(words) {
            row.fill(0);
            row[words - 1] |= pad;
        }
    }
    let (cells, moat) = (layers & CELLS != 0, layers & MOAT != 0);
    let keep = |vmax: f32, umin: f32| vmax >= V_M || (moat && 1.0 - umin > MOAT_D0);
    let table = &c.table;
    for_each_dot(c, keep, |x, y, s, u| {
        let t = c.thr[y as usize * dw + x as usize];
        if s >= V_M {
            if trace {
                inside[y as usize * words + x as usize / 64] |= 1 << (x % 64);
            }
            if cells {
                let kind = if s >= V_SUN { HAS_CORE } else { HAS_BODY };
                stipple.put(x, y, t < table[cell_step(s)], kind);
            }
        } else if moat {
            let p = 256.0 * moat_p(u) as f32;
            stipple.put(x, y, f32::from(t) < p, HAS_MOAT);
        }
    });
    if trace {
        trace_rim(rim, inside, words, dw);
    }
}

/// Light every inside dot with a 4-neighbour outside as the membrane, a word (64 dots) at
/// a time; rows above the first and below the last count as inside.
fn trace_rim(rim: &mut Dots, inside: &[u64], words: usize, dw: usize) {
    let rows = inside.len() / words.max(1);
    for y in 0..rows {
        let row = &inside[y * words..][..words];
        let up = (y > 0).then(|| &inside[(y - 1) * words..][..words]);
        let down = (y + 1 < rows).then(|| &inside[(y + 1) * words..][..words]);
        for (k, &c) in row.iter().enumerate() {
            if c == 0 {
                continue;
            }
            // Bit i is dot 64k + i: its west neighbour is bit i − 1 (the previous word's
            // top bit for i = 0), its east neighbour bit i + 1.
            let west = (c << 1) | if k > 0 { row[k - 1] >> 63 } else { 1 };
            let east = (c >> 1)
                | if k + 1 < words {
                    row[k + 1] << 63
                } else {
                    1 << 63
                };
            let north = up.map_or(!0, |r| r[k]);
            let south = down.map_or(!0, |r| r[k]);
            let mut edge = c & !(west & east & north & south);
            while edge != 0 {
                let x = 64 * k + edge.trailing_zeros() as usize;
                edge &= edge - 1;
                if x < dw {
                    rim.set(x as i32, y as i32, RIM_INK);
                }
            }
        }
    }
}

/// The two homogeneous non-trivial steady states of the Gray–Scott ODE, when they exist
/// (F > 4(F + k)², left of Pearson's saddle-node curve), after the trivial `(1, 0)`
/// (CHEM-A5): `v± = [F ± √(F² − 4F(F + k)²)] / (2(F + k))`, `u = (F + k)/v`.
#[cfg(test)]
pub(crate) fn uniform_states(f: f64, k: f64) -> ([(f64, f64); 3], usize) {
    let mut states = [(1.0, 0.0); 3];
    let fk = f + k;
    let disc = f * f - 4.0 * f * fk * fk;
    if disc <= 0.0 {
        return (states, 1);
    }
    for (slot, sign) in states[1..].iter_mut().zip([1.0, -1.0]) {
        let v = (f + sign * disc.sqrt()) / (2.0 * fk);
        *slot = (fk / v, v);
    }
    (states, 3)
}

#[derive(Debug)]
pub struct GrayScott {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    /// Simulated seconds since start, and the seeded walk offset (a whole slot).
    t: f64,
    t0: f64,
    /// Preset slot forced by tests (any slot); `None` draws one of [`START_SLOTS`] from the
    /// RNG on the first resize.
    slot: Option<usize>,
    walk: Timeline,
    /// Reaction time not yet integrated (less than one sub-step).
    pending: f64,
    since_check: f64,
    /// Times the culture was reseeded after the first resize (CHEM-A4, CHEM-A6).
    reseeds: u32,
    u: Grid<f32>,
    v: Grid<f32>,
    u_next: Grid<f32>,
    v_next: Grid<f32>,
    /// The passaging mask on the lattice and the seed it is keyed by (drawn once).
    mask: Grid<f32>,
    /// `v` through the display filter ([`soften`]), and its scratch pass, on the lattice.
    shown: Grid<f32>,
    scratch: Grid<f32>,
    mask_seed: u64,
    /// Lattice → dot mapping per screen axis ([`build_axis`]), the braille dots, and the
    /// terminal size for the label.
    fx: Vec<Tap>,
    fy: Vec<Tap>,
    xs: Vec<Span>,
    ys: Vec<Span>,
    /// One bit per dot: inside a cell, for tracing the membrane ([`draw_culture`]).
    inside: Vec<u64>,
    /// Each dot's fixed stipple threshold ([`threshold`]), built in `resize`.
    thr: Vec<u8>,
    /// The membrane thread, blitted over the stipple so a terminal cell it crosses shows
    /// the thread alone (a cell has one ink).
    rim: Dots,
    dots: Stipple,
    size: Size,
}

impl GrayScott {
    pub(crate) fn new(slot: Option<usize>) -> Self {
        Self {
            dirty: false,
            t: 0.0,
            t0: 0.0,
            slot,
            walk: walk(),
            pending: 0.0,
            since_check: 0.0,
            reseeds: 0,
            u: Grid::default(),
            v: Grid::default(),
            u_next: Grid::default(),
            v_next: Grid::default(),
            mask: Grid::default(),
            shown: Grid::default(),
            scratch: Grid::default(),
            mask_seed: 0,
            fx: Vec::new(),
            fy: Vec::new(),
            xs: Vec::new(),
            ys: Vec::new(),
            inside: Vec::new(),
            thr: Vec::new(),
            rim: Dots::default(),
            dots: Stipple::default(),
            size: Size::new(1, 1),
        }
    }

    fn dims(&self) -> (usize, usize) {
        (usize::from(self.u.width()), usize::from(self.u.height()))
    }

    /// Drop Gaussian perturbation bumps `v = ¼·exp(−(r/r₀)²)`, `u = 1 − 2v` (Pearson's
    /// u = ½, v = ¼ at the centre) with ±5 % noise per node into the field. Round, graded
    /// and jittered, so no square symmetry is imprinted; a bump only raises v and lowers u.
    fn seed_patches(&mut self, rng: &mut impl Rng) {
        let (w, h) = self.dims();
        if w == 0 || h == 0 {
            return;
        }
        let count = 3 + w * h / SEED_NODES_PER_BUMP;
        let (u, v) = (self.u.as_mut_slice(), self.v.as_mut_slice());
        for _ in 0..count {
            let (cx, cy) = (rng.random_range(0..w) as i32, rng.random_range(0..h) as i32);
            let r0: f32 = rng.random_range(SEED_R0.0..=SEED_R0.1);
            let reach = (2.5 * r0).ceil() as i32;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let r2 = (dx * dx + dy * dy) as f32 / (r0 * r0);
                    let bump = 0.25 * (-r2).exp() * rng.random_range(0.95..1.05f32);
                    // Wrapped onto the torus, so always in range.
                    let x = (cx + dx).rem_euclid(w as i32) as usize;
                    let y = (cy + dy).rem_euclid(h as i32) as usize;
                    let i = y * w + x;
                    u[i] = u[i].min(1.0 - 2.0 * bump);
                    v[i] = v[i].max(bump);
                }
            }
        }
    }

    /// Reseed a featureless culture after the first resize, counting it.
    fn reseed(&mut self, rng: &mut impl Rng) {
        self.reseeds += 1;
        self.seed_patches(rng);
    }

    /// True if the pattern has died out (max v < 0.02) or gone uniform (the spread of v
    /// is below 0.05). The spread, unlike σ(v), does not mistake a young, sparse culture
    /// for a dead one. A safety net: the walk's morphs follow the saddle-node curve, so no
    /// hold or morph passes through a homogeneous state (CHEM-A4, CHEM-A5).
    fn is_featureless(&self) -> bool {
        let v = self.v.as_slice();
        if v.is_empty() {
            return false;
        }
        let (min, max) = v.iter().fold((f32::INFINITY, 0.0f32), |(lo, hi), &x| {
            (lo.min(x), hi.max(x))
        });
        max < 0.02 || max - min < 0.05
    }

    fn substep(&mut self, c: Coeffs) {
        let (w, h) = self.dims();
        gs_step::<true>(
            w,
            h,
            self.u.as_slice(),
            self.v.as_slice(),
            self.u_next.as_mut_slice(),
            self.v_next.as_mut_slice(),
            c,
        );
        std::mem::swap(&mut self.u, &mut self.u_next);
        std::mem::swap(&mut self.v, &mut self.v_next);
    }

    /// The passaging sink, Lie-split after a sub-step: `v ← v·(1 − s·mask)` with
    /// `s = τκm² ≤ 0.01`, a shrinking factor, so positivity is untouched.
    fn passage(&mut self, s: f32) {
        for (v, &m) in self.v.as_mut_slice().iter_mut().zip(self.mask.as_slice()) {
            *v -= s * m * *v;
        }
    }

    /// Move the culture onto a new lattice, stretching the pattern to the new domain
    /// (ADR-0012); the edge-clamp seam heals by diffusion within ≈ 1 s.
    fn regrid(&mut self, lattice: Size) {
        let (mut u, mut v) = (Grid::new(lattice, 1.0), Grid::new(lattice, 0.0));
        grid::resample_bilinear(&self.u, &mut u);
        grid::resample_bilinear(&self.v, &mut v);
        (self.u, self.v) = (u, v);
        self.u_next = Grid::new(lattice, 1.0);
        self.v_next = Grid::new(lattice, 0.0);
    }

    /// The fields and the lattice → dot mapping, for one raster.
    #[cfg(test)]
    pub(crate) fn culture(&self) -> Culture<'_> {
        culture(
            &self.u,
            &self.shown,
            [&self.fx, &self.fy],
            [&self.xs, &self.ys],
            &self.thr,
        )
    }

    /// Stipple the culture onto the dots (Look (M17)): `v` through the display filter
    /// ([`soften`]), then every layer.
    fn rasterise(&mut self) {
        soften(&self.v, &mut self.scratch, &mut self.shown);
        let c = culture(
            &self.u,
            &self.shown,
            [&self.fx, &self.fy],
            [&self.xs, &self.ys],
            &self.thr,
        );
        self.dots.clear();
        self.rim.clear();
        draw_culture(
            &mut self.dots,
            &mut self.rim,
            &mut self.inside,
            &c,
            ALL_LAYERS,
        );
    }

    /// ` mitosis  F=.037 k=.065 ` on row 0 (CHEM-A7), the name only at width ≥ 56: the
    /// text starts at column 1 inside a one-cell blank gutter on each side, so it never
    /// touches the pattern. The name switches at mid-morph; F and k are written from
    /// integer thousandths into a stack buffer.
    fn label(&self, canvas: &mut Canvas) {
        let (w, h) = (self.size.width(), self.size.height());
        if w < 18 || h < 6 {
            return;
        }
        let WalkPoint { c, from, to, m, .. } = preset_at(&self.walk, self.t + self.t0);
        let thousandths = |x: f32| (f64::from(x) * 1000.0).round() as u32;
        let mut text = StackText::<40>::new();
        if w >= LABEL_NAME_MIN {
            let name = PRESETS[if m < 0.5 { from } else { to }].name;
            let _ = write!(text, "{name}  ");
        }
        let _ = write!(
            text,
            "F=.{:03} k=.{:03}",
            thousandths(c.f),
            thousandths(c.k)
        );
        draw::label(canvas, 0, 0, text.as_str(), LABEL_INK);
    }
}

impl Generator for GrayScott {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let lattice = lattice_size(size);
        if self.u.is_empty() {
            // A whole start slot, so each visit opens at the start of a moving regime.
            let slot = self.slot.map_or_else(
                || START_SLOTS[ctx.rng.random_range(0..START_SLOTS.len())],
                |s| s % PRESETS.len(),
            );
            self.t0 = slot_start(slot);
            self.u = Grid::new(lattice, 1.0);
            self.v = Grid::new(lattice, 0.0);
            self.u_next = Grid::new(lattice, 1.0);
            self.v_next = Grid::new(lattice, 0.0);
            self.seed_patches(&mut ctx.rng);
            self.mask_seed = ctx.rng.random();
            let c = preset_at(&self.walk, self.t0).c;
            for _ in 0..PREROLL_SUBSTEPS {
                self.substep(c);
            }
        } else if self.u.size() != lattice {
            self.regrid(lattice);
            if self.is_featureless() {
                self.reseed(&mut ctx.rng);
            }
        }
        if self.mask.size() != lattice {
            self.mask = passage_mask(lattice, self.mask_seed);
        }
        if self.shown.size() != lattice {
            self.shown = Grid::new(lattice, 0.0);
            self.scratch = Grid::new(lattice, 0.0);
        }
        self.size = size;
        self.dots.resize(size);
        self.rim.resize(size);
        let dots = self.rim.dot_size();
        let (w, h) = self.dims();
        build_axis(w, usize::from(dots.width()), &mut self.fx, &mut self.xs);
        build_axis(h, usize::from(dots.height()), &mut self.fy, &mut self.ys);
        let (dw, dh) = (i32::from(dots.width()), i32::from(dots.height()));
        self.thr.clear();
        self.thr
            .extend((0..dh).flat_map(|y| (0..dw).map(move |x| threshold(x, y))));
        let bits = row_words(usize::from(dots.width())) * usize::from(dots.height());
        self.inside.clear();
        self.inside.resize(bits, 0);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.u.is_empty() {
            return;
        }
        self.t += dt;
        let at = preset_at(&self.walk, self.t + self.t0);
        // τ stays fixed, so the positivity bounds hold at every rate; the rate only sets
        // how many sub-steps a frame runs.
        self.pending += dt * at.rate;
        let sink = if at.from == PASSAGE_FROM && at.to != at.from {
            TAU_SUB * PASSAGE_KAPPA * (at.m * at.m) as f32
        } else {
            0.0
        };
        let tau = f64::from(TAU_SUB);
        let mut steps = 0;
        while self.pending >= tau && steps < MAX_SUBSTEPS {
            self.substep(at.c);
            if sink > 0.0 {
                self.passage(sink);
            }
            self.pending -= tau;
            steps += 1;
        }
        self.pending = self.pending.min(tau);
        self.since_check += dt;
        if self.since_check >= CHECK_EVERY {
            self.since_check = 0.0;
            if self.is_featureless() {
                self.reseed(&mut ctx.rng);
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
        self.rim.blit(canvas);
        self.label(canvas);
    }
}

#[cfg(test)]
mod tests {
    use sciart_core::grid::laplacian9;

    use super::*;

    fn variance(a: &[f32]) -> f64 {
        let n = a.len() as f64;
        let mean = a.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
        a.iter()
            .map(|&x| (f64::from(x) - mean).powi(2))
            .sum::<f64>()
            / n
    }

    fn mean(a: &[f32]) -> f64 {
        a.iter().map(|&x| f64::from(x)).sum::<f64>() / a.len() as f64
    }

    fn scene(slot: Option<usize>, seed: u64, w: u16, h: u16) -> (GrayScott, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = GrayScott::new(slot);
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn grid_of(w: usize, h: usize, cells: &[f32]) -> Grid<f32> {
        let mut g = Grid::new(Size::new(w as u16, h as u16), 0.0);
        g.as_mut_slice().copy_from_slice(cells);
        g
    }

    #[test]
    fn chem_a1_fields_stay_finite_and_in_unit_interval_for_any_dt() {
        for seed in 0..3 {
            let (mut g, mut ctx) = scene(None, seed, 60, 20);
            for i in 0..120 {
                let dt = [0.25, 0.0, 1.0 / 60.0, 0.173][i % 4];
                g.update(dt, &mut ctx);
                g.prepare();
                for &x in g.u.as_slice().iter().chain(g.v.as_slice()) {
                    assert!(x.is_finite() && (0.0..=1.0).contains(&x), "seed {seed}");
                }
            }
        }
    }

    #[test]
    fn chem_a2_uniform_unreacted_state_is_a_fixed_point() {
        let (w, h) = (17, 9);
        let (u, v) = (vec![1.0f32; w * h], vec![0.0f32; w * h]);
        let (mut un, mut vn) = (vec![0.5f32; w * h], vec![0.5f32; w * h]);
        for p in PRESETS {
            gs_step::<true>(w, h, &u, &v, &mut un, &mut vn, p.c);
            assert!(un.iter().all(|&x| x == 1.0) && vn.iter().all(|&x| x == 0.0));
        }
    }

    #[test]
    fn chem_a3_pure_diffusion_conserves_mass_on_the_torus() {
        let (w, h) = (31, 13);
        // Deterministic rough field in [0, 1].
        let mut u: Vec<f32> = (0..w * h)
            .map(|i| ((i * 7919 % 101) as f32) / 100.0)
            .collect();
        let mut v: Vec<f32> = (0..w * h)
            .map(|i| ((i * 104_729 % 37) as f32) / 72.0)
            .collect();
        let (mut un, mut vn) = (vec![0.0; w * h], vec![0.0; w * h]);
        let sum = |a: &[f32]| a.iter().map(|&x| f64::from(x)).sum::<f64>();
        let (su, sv, var0) = (sum(&u), sum(&v), variance(&v));
        let c = Coeffs { f: 0.05, k: 0.06 };
        for _ in 0..300 {
            gs_step::<false>(w, h, &u, &v, &mut un, &mut vn, c);
            std::mem::swap(&mut u, &mut un);
            std::mem::swap(&mut v, &mut vn);
        }
        // f32 round-off: ~1e-7 relative per add, 300 steps -> well under 1e-5.
        assert!((sum(&u) - su).abs() / su < 1e-5, "{} vs {su}", sum(&u));
        assert!((sum(&v) - sv).abs() / sv < 1e-5, "{} vs {sv}", sum(&v));
        assert!(
            variance(&v) < 0.1 * var0,
            "diffusion should smooth the field"
        );
    }

    /// Runs `f` on every item, each on its own thread: the runs are independent, long
    /// simulations.
    fn par<T: Send>(items: Vec<T>, f: impl Fn(T) + Sync) {
        let f = &f;
        std::thread::scope(|s| {
            for item in items {
                s.spawn(move || f(item));
            }
        });
    }

    fn active(g: &GrayScott) -> usize {
        g.v.as_slice().iter().filter(|&&x| x > 0.1).count()
    }

    /// Advance a scene to visit time `t` (simulated seconds) in 60 Hz frames.
    fn run_to(g: &mut GrayScott, ctx: &mut Ctx, t: f64) {
        while g.t < t - 0.5 * SIM_DT {
            g.update(SIM_DT, ctx);
            g.prepare();
        }
    }

    #[test]
    fn chem_a4_every_start_slot_develops_structure_without_reseeding() {
        // Seeds 1 and 3 (TEST time cut: a dead start is systematic, not a seed lottery).
        let runs: Vec<(usize, u64)> = START_SLOTS
            .iter()
            .flat_map(|&slot| [1, 3].map(|seed| (slot, seed)))
            .collect();
        par(runs, |(slot, seed)| {
            let size = Size::new(80, 24);
            let (mut g, mut ctx) = scene(Some(slot), seed, 80, 24);
            let a0 = active(&g);
            run_to(&mut g, &mut ctx, 10.0);
            let (var, a1) = (variance(g.v.as_slice()), active(&g));
            let (_, dark, _) = stats(&ascii_frame(&g, size), None);
            let id = format!("slot {slot} seed {seed}");
            assert!(var > 3e-3, "{id}: var(v) {var}");
            assert!(dark <= 0.9, "{id}: {dark} blank or '.'");
            if slot == 0 {
                // Mitosis grows from the seed bumps; the moving regimes are already
                // developed by the pre-roll.
                assert!(a1 >= 2 * a0, "{id}: {a0} → {a1} active");
            }
            run_to(&mut g, &mut ctx, 20.0);
            assert_eq!(g.reseeds, 0, "{id}: reseeded");
        });
    }

    #[test]
    fn chem_a8_start_holds_keep_a_dark_majority_and_keep_moving() {
        // Both seeds at 80×24; seed 1 only at 200×50 (TEST time cut: the size sweep
        // keeps one run per slot instead of two).
        let mut runs: Vec<((u16, u16), usize, u64)> = Vec::new();
        for size in [(80, 24), (200, 50)] {
            let seeds: &[u64] = if size.0 == 80 { &[1, 7] } else { &[1] };
            for slot in START_SLOTS {
                for &seed in seeds {
                    runs.push((size, slot, seed));
                }
            }
        }
        par(runs, |((w, h), slot, seed)| {
            let size = Size::new(w, h);
            let (mut g, mut ctx) = scene(Some(slot), seed, w, h);
            let id = format!("{w}x{h} slot {slot} seed {seed}");
            // The ASCII frame and the near-black share of the half-block pixels at `t`.
            let mut frame_at = |t: f64| {
                run_to(&mut g, &mut ctx, t);
                assert!(g.t < PRESETS[slot].hold, "{id}: past the hold");
                (ascii_frame(&g, size), near_black_share(&g))
            };
            // Loops and swarm: organisms jostling on a near-black field, not a boiling one.
            // Samples through the hold, each with its next frame (1/30 s) and the frame
            // 0.25 s on. Churn and darkness are averaged over the samples; a single frame
            // may dip a little below the mean's floor.
            let jostle = slot == 2 || slot == 3;
            // [dark, near-black, change per frame, change per 0.25 s] at hold time `t`.
            let sample = |frame_at: &mut dyn FnMut(f64) -> (Frame, f64), t: f64| {
                let (f, near_black) = frame_at(t);
                let (_, dark, _) = stats(&f, None);
                assert!(dark >= 0.4, "{id} t {t}: only {dark} blank or '.'");
                assert!(near_black >= 0.45, "{id} t {t}: {near_black} near-black");
                let per_frame = stats(&frame_at(t + 2.0 * SIM_DT).0, Some(&f)).2;
                let per_quarter = stats(&frame_at(t + 0.25).0, Some(&f)).2;
                [dark, near_black, per_frame, per_quarter]
            };
            let mut samples = Vec::new();
            if jostle {
                for t in [2.0, 5.0, 8.0] {
                    samples.push(sample(&mut frame_at, t));
                }
            }
            // 10 s into the hold, or 4 s before the end of a shorter one (mitosis, 8 s).
            let t1 = (PRESETS[slot].hold - 4.1).min(10.0);
            let f1 = frame_at(t1).0;
            let (dense, dark, _) = stats(&f1, None);
            assert!(dense <= 0.5, "{id}: {dense} of cells are # or @");
            assert!(dark >= 0.2, "{id}: only {dark} blank or '.'");
            let (.., change) = stats(&frame_at(t1 + 4.0).0, Some(&f1));
            assert!(
                change >= 0.05,
                "{id}: {change} changed in the 4 s from {t1}"
            );
            if jostle {
                for t in [15.0, 18.0] {
                    samples.push(sample(&mut frame_at, t));
                }
                let n = samples.len() as f64;
                let mean = |i: usize| samples.iter().map(|s| s[i]).sum::<f64>() / n;
                let [dark, near_black, per_frame, per_quarter] = [0, 1, 2, 3].map(mean);
                assert!(dark >= 0.45, "{id}: on average only {dark} blank or '.'");
                assert!(
                    near_black >= 0.5,
                    "{id}: on average {near_black} near-black"
                );
                assert!(per_frame <= 0.04, "{id}: {per_frame} change per frame");
                assert!(per_quarter <= 0.35, "{id}: {per_quarter} change per 0.25 s");
            }
            if slot != 0 {
                // The moving regimes never settle: still churning at the end of the hold.
                let f20 = frame_at(20.0).0;
                let (.., change) = stats(&frame_at(23.9).0, Some(&f20));
                assert!(change >= 0.3, "{id}: {change} changed in 20–24 s");
            }
        });
    }

    #[test]
    fn chem_a5_a8_whole_walk_never_reseeds_stays_dark_and_keeps_moving() {
        // At 80×24. From the mitosis slot, every seed: one full walk plus 2 s never
        // reseeds. Seeds 1 and 7 from the mitosis slot, over the walk and on through the
        // passaged return to mitosis and its hold, and from the spots slot through the
        // first return: each frame at 4i has at most 50 % `#`/`@` cells, 60 % in the
        // coral beat (a labyrinth covers ≈ 55 % of the cells at any exposure), and in
        // every 4 s window [4i, 4i + 4] at least 10 % of the cells change. At each
        // frame the unclamped Euler update of the live fields already stays in [0, 1]
        // with v < 0.7 (measured ≤ 0.63, loops colliding), so the kernel's clamp is only
        // a guard (CHEM-A1).
        let walk = walk();
        let total = walk.total();
        let spots = PRESETS.len() - 1;
        let back = slot_start(PRESETS.len()) - slot_start(spots);
        // Seeds 1 and 7 carry the frame-window checks; 5 and 11 only the no-reseed
        // claim, halved to seed 5 (TEST time cut: same assertion kinds, less coverage).
        let runs = [(0, 1u64), (0, 5), (0, 7), (spots, 1), (spots, 7)];
        par(runs.to_vec(), |(slot, seed)| {
            let size = Size::new(80, 24);
            let (mut g, mut ctx) = scene(Some(slot), seed, 80, 24);
            let checked = seed == 1 || seed == 7;
            // Whole 4 s windows, through the returning mitosis hold and into the morph
            // that follows it.
            let end = match (slot, checked) {
                (0, false) => total + 2.0,
                (0, true) => ((total + PRESETS[0].hold + 9.0) / 4.0).floor() * 4.0,
                _ => ((back + PRESETS[0].hold + 9.0) / 4.0).floor() * 4.0,
            };
            let mut last = ascii_frame(&g, size);
            let mut t = 0.0;
            while t < end {
                t = (t + 4.0).min(end);
                run_to(&mut g, &mut ctx, t);
                if !checked {
                    continue;
                }
                let frame = ascii_frame(&g, size);
                let (dense, _, change) = stats(&frame, Some(&last));
                last = frame;
                let at = preset_at(&walk, g.t + g.t0);
                let coral = at.from == 1 || (at.to == 1 && at.m >= 0.5);
                let cap = if coral { 0.6 } else { 0.5 };
                let id = format!("slot {slot} seed {seed} t {t}");
                assert!(dense <= cap, "{id}: {dense} of cells are # or @");
                assert!(change >= 0.1, "{id}: {change} changed in the last 4 s");
                let vmax = unclamped_step_in_unit_interval(&g, at.c, &id);
                assert!(vmax < 0.7, "{id}: v peaks at {vmax}");
            }
            assert_eq!(g.reseeds, 0, "slot {slot} seed {seed}: reseeded");
        });
    }

    /// Applies the unclamped reference Euler sub-step (core `laplacian9`) to the live
    /// fields, asserts every result lies in [0, 1], and returns the largest new v.
    fn unclamped_step_in_unit_interval(g: &GrayScott, c: Coeffs, id: &str) -> f32 {
        let (mut lu, mut lv) = (Grid::default(), Grid::default());
        laplacian9(&g.u, &mut lu, Edges::Wrap);
        laplacian9(&g.v, &mut lv, Edges::Wrap);
        let (u, v) = (g.u.as_slice(), g.v.as_slice());
        let mut vmax = 0.0f32;
        for i in 0..u.len() {
            let r = u[i] * v[i] * v[i];
            let eu = u[i] + TAU_SUB * (D_U * lu.as_slice()[i] - r + c.f * (1.0 - u[i]));
            let ev = v[i] + TAU_SUB * (D_V * lv.as_slice()[i] + r - (c.f + c.k) * v[i]);
            assert!((0.0..=1.0).contains(&eu), "{id}: unclamped u {eu}");
            assert!((0.0..=1.0).contains(&ev), "{id}: unclamped v {ev}");
            vmax = vmax.max(ev);
        }
        vmax
    }

    #[test]
    fn chem_a5_walk_holds_morphs_smoothly_along_the_saddle_node_curve() {
        // The walk is exactly the spec's preset list: name, F, k, rate, hold, morph.
        let spec: [(&str, f32, f32, f64, f64, f64); 5] = [
            ("mitosis", 0.0367, 0.0649, 400.0, 8.0, 6.0),
            ("coral", 0.0545, 0.062, 400.0, 3.0, 6.0),
            ("loops", 0.018, 0.051, 70.0, 24.0, 8.0),
            ("swarm", 0.016, 0.052, 110.0, 24.0, 8.0),
            ("spots", 0.014, 0.054, 150.0, 24.0, 8.0),
        ];
        let table = PRESETS.map(|p| (p.name, p.c.f, p.c.k, p.rate, p.hold, p.morph));
        assert_eq!(table, spec);

        let walk = walk();
        assert_eq!(walk.total(), 119.0);
        for (i, p) in PRESETS.iter().enumerate() {
            let start = slot_start(i);
            for t in [start, start + 0.5 * p.hold, start + p.hold - 0.01] {
                let hold = WalkPoint {
                    c: p.c,
                    rate: p.rate,
                    from: i,
                    to: i,
                    m: 0.0,
                };
                assert_eq!(preset_at(&walk, t), hold, "hold {i} at {t}");
            }
            let next = (i + 1) % PRESETS.len();
            let mid = preset_at(&walk, start + p.hold + 0.5 * p.morph);
            assert_eq!((mid.from, mid.to), (i, next));
            // The rate is lerped by the same smoothstep m (½ at mid-morph).
            let want = 0.5 * (p.rate + PRESETS[next].rate);
            assert!(
                (mid.rate - want).abs() < 1e-9,
                "morph {i}: rate {}",
                mid.rate
            );
        }
        assert_eq!(slot_start(PRESETS.len()), walk.total());
        assert_eq!(preset_at(&walk, 119.0 + 5.0), preset_at(&walk, 5.0));
        let mut opened = [false; PRESETS.len()];
        for seed in 0..40 {
            let (g, _) = scene(None, seed, 40, 12);
            let slot = (0..PRESETS.len()).find(|&i| g.t0 == slot_start(i));
            opened[slot.expect("a visit opens at the start of a hold")] = true;
        }
        let starts: Vec<usize> = (0..PRESETS.len()).filter(|&i| opened[i]).collect();
        assert_eq!(starts, START_SLOTS, "every start slot and never coral");

        // Continuous, inside the positivity bounds, and within a signed band around the
        // saddle-node curve everywhere along the walk, morphs included.
        let off_curve = |c: Coeffs| {
            let (f, k) = (f64::from(c.f), f64::from(c.k));
            f - 4.0 * (f + k) * (f + k)
        };
        let mut last = preset_at(&walk, 0.0);
        for i in 0..=11_900 {
            let at = preset_at(&walk, f64::from(i) * 0.01);
            let c = at.c;
            assert!(
                (c.f - last.c.f).abs() <= 1e-4 && (c.k - last.c.k).abs() <= 1e-4,
                "at {i}"
            );
            // The rate moves by at most 1 unit/s per 10 ms (250 over a 6 s smoothstep
            // peaks at 0.625) and stays within the presets' range.
            assert!((at.rate - last.rate).abs() <= 1.0, "rate jumps at {i}");
            assert!(
                (70.0..=MAX_RATE).contains(&at.rate),
                "rate {} at {i}",
                at.rate
            );
            last = at;
            assert!(TAU_SUB * (10.0 / 3.0 * D_U + 1.0 + c.f) <= 1.0);
            assert!(TAU_SUB * (10.0 / 3.0 * D_V + c.f + c.k) <= 1.0);
            assert!(c.f > 0.0 && c.k > 0.0);
            let off = off_curve(c);
            assert!((-0.005..=0.0015).contains(&off), "at {i}: {c:?} off {off}");
        }
        // Mid-morph, k keeps the interpolated offset from the curve rather than the chord.
        let (a, b) = (PRESETS[1].c, PRESETS[2].c);
        let c = preset_at(&walk, slot_start(2) - 0.5 * PRESETS[1].morph).c;
        let mid = |x: f32, y: f32| 0.5 * (f64::from(x) + f64::from(y));
        assert!((f64::from(c.f) - mid(a.f, b.f)).abs() < 1e-6);
        let offset = |c: Coeffs| f64::from(c.k) - k_saddle_node(f64::from(c.f));
        assert!((offset(c) - 0.5 * (offset(a) + offset(b))).abs() < 1e-6);
        for f in [0.01, 0.03, 0.06] {
            let k = k_saddle_node(f);
            assert!((f - 4.0 * (f + k) * (f + k)).abs() < 1e-12);
        }

        // That the culture carries over a whole loop without a reseed is checked with the
        // whole-walk frames in `chem_a5_a8_whole_walk_…`.

        for (f, k) in [(0.0367, 0.0649), (0.078, 0.061), (0.02, 0.05), (0.03, 0.06)] {
            let (states, n) = uniform_states(f, k);
            assert_eq!(states[0], (1.0, 0.0));
            assert_eq!(n == 3, f > 4.0 * (f + k) * (f + k), "({f}, {k})");
            for &(u, v) in &states[..n] {
                let du = -u * v * v + f * (1.0 - u);
                let dv = u * v * v - (f + k) * v;
                assert!(du.abs() < 1e-9 && dv.abs() < 1e-9, "({f}, {k}): ({u}, {v})");
            }
        }
    }

    #[test]
    fn chem_a6_lattice_is_capped_and_resize_resamples_without_reseeding() {
        for w in (1..=1000u16).step_by(9) {
            for h in (1..=300u16).step_by(7) {
                let l = lattice_size(Size::new(w, h));
                assert!(
                    l.area() <= 5120 && l.width() >= 32 && l.height() >= 16,
                    "{w}x{h}"
                );
                // One node per half-block pixel below the cap.
                let plain = Size::new(w.max(32), (2 * h).max(16));
                if plain.area() <= 4500 {
                    assert_eq!(l, plain, "{w}x{h}");
                }
            }
        }
        assert_eq!(lattice_size(Size::new(80, 24)), Size::new(80, 48));
        assert_eq!(lattice_size(Size::new(40, 12)), Size::new(40, 24));
        let big = lattice_size(Size::new(200, 50));
        assert_eq!(big, Size::new(100, 50));
        assert_eq!(lattice_size(Size::new(300, 75)), big);
        assert_eq!(lattice_size(Size::new(400, 100)), big);

        // Same lattice: the culture is untouched.
        let (mut g, mut ctx) = scene(Some(0), 3, 300, 75);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let (u, v) = (g.u.clone(), g.v.clone());
        g.resize(Size::new(400, 100), &mut ctx);
        g.resize(Size::new(200, 50), &mut ctx);
        assert!(g.u == u && g.v == v);

        // A new lattice resamples; the pattern survives and keeps its contrast (5 s in, so
        // the 2 s check ends inside the 8 s mitosis hold).
        for _ in 0..180 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let (mean0, std0) = (mean(g.v.as_slice()), variance(g.v.as_slice()).sqrt());
        assert_eq!(g.reseeds, 0);
        g.resize(Size::new(120, 50), &mut ctx);
        assert_eq!(g.u.size(), Size::new(80, 63));
        g.resize(Size::new(200, 50), &mut ctx);
        assert_eq!(g.reseeds, 0, "the round trip must not reseed");
        let mean1 = mean(g.v.as_slice());
        assert!(
            (mean1 / mean0 - 1.0).abs() < 0.05,
            "mean v {mean0} → {mean1}"
        );
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let std1 = variance(g.v.as_slice()).sqrt();
        assert!((std1 / std0 - 1.0).abs() < 0.3, "std v {std0} → {std1}");
        assert_eq!(g.reseeds, 0);
        assert!(
            g.t + g.t0 < PRESETS[0].hold,
            "the test must stay in the mitosis hold"
        );
        for (w, h) in [(1, 1), (3, 2), (200, 50), (5, 90), (1, 1)] {
            g.resize(Size::new(w, h), &mut ctx);
            g.update(0.25, &mut ctx);
            g.prepare();
            g.render(&mut Canvas::new(Size::new(w, h), GlyphMode::Unicode));
        }
    }

    #[test]
    fn chem_a7_a9_inks_ascii_and_label() {
        // M17: the M15 Seq heat map is superseded by the stipple's four inks (CHEM-A9); the
        // label is unchanged.
        let mut s = Stage::new(
            Box::new(GrayScott::new(Some(0))),
            42,
            Size::new(80, 24),
            GlyphMode::Unicode,
        );
        // The scene's own frame: the ASCII check below covers the field, not the title.
        s.set_title_visible(false);
        for _ in 0..180 {
            s.step(SIM_DT);
        }
        let frame = s.render();
        for c in frame.cells() {
            assert_eq!(c.bg, Ink::BLANK, "no half-block cells");
            assert!(!matches!(c.ch, '▀' | '▄' | '█'));
            assert!(
                [CELL_INK, SUN_INK, RIM_INK, MOAT_INK, LABEL_INK, Ink::BLANK].contains(&c.ink),
                "{:?}",
                c.ink
            );
        }
        let row: String = frame.to_lines()[0].chars().take(24).collect();
        assert_eq!(
            row, " mitosis  F=.037 k=.065 ",
            "label inside a blank gutter"
        );
        assert_eq!(
            frame.cells()[1].ink,
            Ink::new(Role::Text, 195),
            "bright label"
        );
        s.set_glyph_mode(GlyphMode::Ascii);
        let lines = s.render().to_lines();
        // The braille dot-count fallback (CORE-R41, GEN-R10).
        let ramp = |c: char| " .:+#".contains(c);
        assert!(lines[1..].iter().all(|l| l.chars().all(ramp)));
        assert!(lines[0].starts_with(" mitosis  F=.037 k=.065 "));
        assert!(lines[0].chars().skip(24).all(ramp));

        let label = |w: u16, h: u16| {
            let mut s = Stage::new(
                Box::new(GrayScott::new(Some(2))),
                1,
                Size::new(w, h),
                GlyphMode::Ascii,
            );
            s.render().to_lines()[0].clone()
        };
        assert!(label(56, 12).starts_with(" loops  F=.018 k=.051 "));
        assert!(label(55, 12).starts_with(" F=.018 k=.051 "));
        assert!(label(40, 12).starts_with(" F=.018 k=.051 "));
        assert!(label(18, 12).starts_with(" F=.018 k=.051 "));
        assert!(!label(17, 12).contains("F="));
        assert!(!label(80, 5).contains("F="));
        // The name switches at mid-morph: loops → swarm morphs over 24–32 s of the visit.
        let mut s = Stage::new(
            Box::new(GrayScott::new(Some(2))),
            1,
            Size::new(80, 24),
            GlyphMode::Ascii,
        );
        for (secs, name) in [(27.8, "loops  "), (28.2, "swarm  ")] {
            while s.ctx().time() < secs {
                s.step(0.1);
            }
            let row: String = s.render().to_lines()[0].chars().skip(1).collect();
            assert!(row.starts_with(name), "at {secs} s: {row}");
        }
    }

    #[test]
    fn chem_a2_a3_kernel_matches_core_laplacian9_reference() {
        // The fast row-sliced kernel must equal the reference Euler update
        // u + τ (D ∇²u − u v² + F(1 − u)), v + τ (D ∇²v + u v² − (F + k) v), with the core
        // isotropic Laplacian, incl. on 1-wide and 2-wide tori where the wrap aliases.
        let c = Coeffs { f: 0.05, k: 0.06 };
        for (w, h) in [(9, 7), (1, 5), (2, 2), (6, 1)] {
            let u: Vec<f32> = (0..w * h).map(|i| 0.3 + 0.07 * (i % 5) as f32).collect();
            let v: Vec<f32> = (0..w * h).map(|i| 0.05 * (i % 7) as f32).collect();
            let (mut un, mut vn) = (vec![0.0; w * h], vec![0.0; w * h]);
            gs_step::<true>(w, h, &u, &v, &mut un, &mut vn, c);
            let (mut lu, mut lv) = (Grid::default(), Grid::default());
            laplacian9(&grid_of(w, h, &u), &mut lu, Edges::Wrap);
            laplacian9(&grid_of(w, h, &v), &mut lv, Edges::Wrap);
            for i in 0..w * h {
                let r = u[i] * v[i] * v[i];
                let eu = u[i] + TAU_SUB * (D_U * lu.as_slice()[i] - r + c.f * (1.0 - u[i]));
                let ev = v[i] + TAU_SUB * (D_V * lv.as_slice()[i] + r - (c.f + c.k) * v[i]);
                assert!((un[i] - eu).abs() < 1e-6 && (vn[i] - ev).abs() < 1e-6);
            }
        }
    }

    /// An ASCII frame below the label row.
    type Frame = Vec<Vec<char>>;

    // The culture tests (CHEM-A4, A5, A8) measure the *field*: a dark majority, motion,
    // never jamming. Their thresholds were tuned on the M15 heat map, so they keep reading
    // the field through that quantiser (v / 0.40 on a Seq ramp from the 0.12 pattern edge,
    // a faint depletion halo, `.:+#@` by quartile), now a test instrument only; the M17
    // stipple look has its own acceptance (CHEM-A9..A12).

    /// The M15 half-block picture of the culture: one pixel per half cell, `v` and `u`
    /// sampled bilinearly (CORE-R52); Seq levels as at M15.
    fn m15_pixels(g: &GrayScott) -> Pixels {
        const V_FULL: f32 = 0.40;
        const Q_MIN: f32 = 0.3;
        let mut pixels = Pixels::new(g.size);
        let mut map = sciart_core::grid::SampleMap::default();
        map.rebuild(g.u.size(), pixels.pixel_size());
        pixels.shade(|x, y| {
            let q = (map.sample(&g.v, x, y) / V_FULL).clamp(0.0, 1.0);
            if q >= Q_MIN {
                let p = (f64::from(q) - f64::from(Q_MIN)) / (1.0 - f64::from(Q_MIN));
                return sciart_core::ink::seq(0.12 + 0.88 * p.powf(0.8));
            }
            let u = map.sample(&g.u, x, y);
            let fade = ((1.0 - u - 0.2) / 0.4).clamp(0.0, 1.0);
            sciart_core::ink::seq(f64::from(0.1 * fade))
        });
        pixels
    }

    /// The M15 ASCII frame below the label row: per cell the brighter half, `.` for the
    /// stronger half of the halo (level ≥ 13), then `:+#@` by quartile.
    fn ascii_frame(g: &GrayScott, size: Size) -> Frame {
        let pixels = m15_pixels(g);
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        pixels.blit(&mut canvas, |_, _, top, bottom| {
            let level = top.level.max(bottom.level);
            let ch = match level {
                0..=12 => return None,
                13..=31 => '.',
                _ => [':', '+', '#', '@'][(usize::from(level - 32) * 4 / 224).min(3)],
            };
            Some((ch, Ink::new(Role::Seq, level)))
        });
        canvas
            .to_lines()
            .iter()
            .skip(1)
            .map(|l| l.chars().collect())
            .collect()
    }

    /// Share of the M15 half-block pixels below the label row that are near-black: the
    /// terminal background or `Seq` below level 32 (the dim halo).
    fn near_black_share(g: &GrayScott) -> f64 {
        let pixels = m15_pixels(g);
        let px = pixels.pixel_size();
        let (mut n, mut dark) = (0usize, 0usize);
        for y in 2..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let ink = pixels.get(x, y).unwrap_or(Ink::BLANK);
                n += 1;
                dark += usize::from(ink.role == Role::Background || ink.level < 32);
            }
        }
        dark as f64 / n.max(1) as f64
    }

    #[test]
    fn chem_a8_passage_mask_is_seeded_and_blocky() {
        for (w, h) in [(80u16, 48u16), (100, 50), (32, 16), (40, 24)] {
            let lattice = Size::new(w, h);
            let (mut active, mut differs) = (0.0, false);
            for seed in 0..8u64 {
                let m = passage_mask(lattice, seed);
                assert_eq!(
                    m,
                    passage_mask(lattice, seed),
                    "a pure function of the seed"
                );
                differs |= m != passage_mask(lattice, seed + 100);
                let cells = m.as_slice();
                assert!(cells.iter().all(|&x| x == 0.0 || x == 1.0));
                active += f64::from(cells.iter().sum::<f32>()) / cells.len() as f64 / 8.0;
                // Constant on blocks of about 6×6 nodes: at most one change of value
                // every 5 nodes along a row or column.
                let (w, h) = (usize::from(w), usize::from(h));
                let changes_x = (0..h)
                    .flat_map(|y| (1..w).map(move |x| (x, y)))
                    .filter(|&(x, y)| cells[y * w + x] != cells[y * w + x - 1])
                    .count();
                assert!(changes_x <= h * w.div_ceil(5), "{w}x{h}: {changes_x}");
            }
            assert!(differs);
            // PASSAGE_SHARE of the blocks, on average.
            assert!((0.55..=0.85).contains(&active), "{w}x{h}: {active} active");
        }
    }

    fn stats(a: &[Vec<char>], b: Option<&[Vec<char>]>) -> (f64, f64, f64) {
        let n = a.iter().map(Vec::len).sum::<usize>() as f64;
        let count =
            |p: &dyn Fn(char) -> bool| a.iter().flatten().filter(|&&c| p(c)).count() as f64 / n;
        let dense = count(&|c| c == '#' || c == '@');
        let dark = count(&|c| c == ' ' || c == '.');
        let change = b.map_or(0.0, |b| {
            a.iter()
                .flatten()
                .zip(b.iter().flatten())
                .filter(|(x, y)| x != y)
                .count() as f64
                / n
        });
        (dense, dark, change)
    }

    /// `v` at every dot, row-major over the dot lattice, sampled as the raster samples it.
    fn v_dots(g: &GrayScott) -> (usize, Vec<f32>) {
        let c = g.culture();
        let dw = c.fx.len();
        let mut out = vec![f32::NAN; dw * c.fy.len()];
        for_each_dot(
            &c,
            |_, _| true,
            |x, y, s, _| out[y as usize * dw + x as usize] = s,
        );
        assert!(
            out.iter().all(|v| v.is_finite()),
            "the spans tile the lattice"
        );
        (dw, out)
    }

    /// The dots of `layers` alone at the scene's current state.
    fn layer(g: &GrayScott, layers: u8) -> Dots {
        let (mut s, mut d) = (Stipple::default(), Dots::new(g.size));
        s.resize(g.size);
        let mut inside = vec![0; g.inside.len()];
        draw_culture(&mut s, &mut d, &mut inside, &g.culture(), layers);
        for (x, y) in stipple_lit(&s, d.dot_size()) {
            d.set(x, y, CELL_INK);
        }
        d
    }

    fn stipple_lit(s: &Stipple, size: Size) -> Vec<(i32, i32)> {
        let (w, h) = (i32::from(size.width()), i32::from(size.height()));
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| s.is_lit(x, y))
            .collect()
    }

    /// The scene's raster: stipple and rim.
    fn raster(g: &GrayScott) -> std::collections::HashSet<(i32, i32)> {
        stipple_lit(&g.dots, g.rim.dot_size())
            .into_iter()
            .chain(lit(&g.rim))
            .collect()
    }

    fn lit(d: &Dots) -> Vec<(i32, i32)> {
        let s = d.dot_size();
        let (w, h) = (i32::from(s.width()), i32::from(s.height()));
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| d.is_lit(x, y))
            .collect()
    }

    #[test]
    fn chem_a9_every_lit_dot_is_a_cell_rim_or_moat_dot() {
        let runs: Vec<(usize, (u16, u16))> = [0, 2, 3, 4]
            .into_iter()
            .flat_map(|slot| [(120, 40), (80, 24)].map(|s| (slot, s)))
            .collect();
        par(runs, |(slot, (w, h))| {
            let (mut g, mut ctx) = scene(Some(slot), 5, w, h);
            run_to(&mut g, &mut ctx, 6.0);
            let id = format!("slot {slot} {w}x{h}");
            let (dw, v) = v_dots(&g);
            let vat = |x: i32, y: i32| v[y as usize * dw + x as usize];
            let cells = lit(&layer(&g, CELLS));
            let moat = lit(&layer(&g, MOAT));
            let rim = lit(&layer(&g, RIM));
            assert!(!cells.is_empty() && !rim.is_empty(), "{id}: a culture");
            assert!(cells.iter().all(|&(x, y)| vat(x, y) >= V_M), "{id}");
            assert!(moat.iter().all(|&(x, y)| vat(x, y) < V_M), "{id}");
            let dh = (v.len() / dw) as i32;
            for &(x, y) in &rim {
                // Within one dot of the iso-line: the 3×3 block straddles v_m. On the
                // outermost dots the neighbour across the iso-line may be off screen.
                if x == 0 || y == 0 || x + 1 == dw as i32 || y + 1 == dh {
                    continue;
                }
                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                for (nx, ny) in (-1..=1).flat_map(|a| (-1..=1).map(move |b| (x + a, y + b))) {
                    if (0..dw as i32).contains(&nx) && (0..dh).contains(&ny) {
                        lo = lo.min(vat(nx, ny));
                        hi = hi.max(vat(nx, ny));
                    }
                }
                assert!(
                    lo <= V_M && V_M <= hi,
                    "{id}: rim dot ({x}, {y}) off the membrane"
                );
            }
            // The full raster is exactly the three layers, on braille only.
            let all = raster(&g);
            let parts: std::collections::HashSet<_> =
                cells.iter().chain(&moat).chain(&rim).copied().collect();
            assert_eq!(all, parts, "{id}");
            let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
            g.render(&mut canvas);
            assert!(canvas.cells().iter().all(|c| c.bg == Ink::BLANK), "{id}");
        });
    }

    #[test]
    fn chem_a10_stipple_density_follows_v() {
        for slot in [0, 2, 4] {
            let (mut g, mut ctx) = scene(Some(slot), 9, 200, 50);
            run_to(&mut g, &mut ctx, 6.0);
            let (dw, v) = v_dots(&g);
            let cells = layer(&g, CELLS);
            let mut inside: Vec<(f32, bool)> = v
                .iter()
                .enumerate()
                .filter(|&(_, &s)| s >= V_M)
                .map(|(i, &s)| (s, cells.is_lit((i % dw) as i32, (i / dw) as i32)))
                .collect();
            inside.sort_by(|a, b| a.0.total_cmp(&b.0));
            let q = inside.len() / 4;
            assert!(q > 200, "slot {slot}: {} cell dots", inside.len());
            let share =
                |s: &[(f32, bool)]| s.iter().filter(|p| p.1).count() as f64 / s.len() as f64;
            let (bottom, top) = (share(&inside[..q]), share(&inside[inside.len() - q..]));
            assert!(
                top >= 2.0 * bottom,
                "slot {slot}: top {top} vs bottom {bottom}"
            );
            // And the kept share tracks the density it was drawn from.
            let want = inside[inside.len() - q..]
                .iter()
                .map(|p| cell_p(p.0))
                .sum::<f64>()
                / q as f64;
            assert!(
                (top - want).abs() < 0.05,
                "slot {slot}: {top} kept, {want} wanted"
            );
        }
        // The density itself: 0 outside, P_EDGE at the membrane, P_CORE at the core,
        // monotone between; the moat is capped and only where u is depleted.
        assert_eq!(cell_p(V_M - 1e-4), 0.0);
        assert_eq!(cell_p(f32::NAN), 0.0);
        assert!((cell_p(V_M) - P_EDGE).abs() < 1e-9);
        assert!((cell_p(V_CORE) - P_CORE).abs() < 1e-9 && cell_p(0.9) == cell_p(V_CORE));
        let mut last = 0.0;
        for i in 0..=100 {
            let p = cell_p(V_M + i as f32 * 0.003);
            assert!(p >= last);
            last = p;
        }
        assert_eq!(moat_p(1.0), 0.0);
        assert!(moat_p(1.0 - MOAT_D0) < 1e-6 && moat_p(0.95) == 0.0);
        assert!((moat_p(0.0) - P_MOAT).abs() < 1e-12 && P_MOAT <= 0.06);
    }

    #[test]
    fn chem_a11_stipple_holds_still() {
        let runs: Vec<usize> = START_SLOTS.to_vec();
        par(runs, |slot| {
            let (mut g, mut ctx) = scene(Some(slot), 42, 200, 50);
            run_to(&mut g, &mut ctx, 20.0);
            let render = |g: &GrayScott| {
                let mut c = Canvas::new(g.size, GlyphMode::Unicode);
                g.render(&mut c);
                c
            };
            // An unchanged state re-rasterises to the identical frame.
            let before = render(&g);
            g.dirty = true;
            g.prepare();
            g.rasterise();
            assert!(render(&g).cells() == before.cells(), "slot {slot}");
            // One 1/60 s step changes under 5 % of the stipple's lit dots (the rim is a
            // thread and moves with the moving spots), and under 10 % of all lit dots.
            let set = |d: &Dots| lit(d).into_iter().collect::<std::collections::HashSet<_>>();
            let (a, a_all) = (set(&layer(&g, CELLS | MOAT)), raster(&g));
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let (b, b_all) = (set(&layer(&g, CELLS | MOAT)), raster(&g));
            let share = |a: &std::collections::HashSet<(i32, i32)>, b| {
                a.symmetric_difference(b).count() as f64 / a.len().max(1) as f64
            };
            let (stipple, all) = (share(&a, &b), share(&a_all, &b_all));
            assert!(
                stipple < 0.05,
                "slot {slot}: {stipple} of stipple dots changed"
            );
            assert!(all < 0.10, "slot {slot}: {all} of lit dots changed");
        });
    }

    #[test]
    fn chem_a12_every_mitosis_spot_has_a_closed_rim() {
        for (seed, (w, h)) in [(1, (120, 40)), (7, (200, 50)), (3, (80, 24))] {
            let (mut g, mut ctx) = scene(Some(0), seed, w, h);
            run_to(&mut g, &mut ctx, 5.0);
            assert_eq!(
                preset_at(&g.walk, g.t + g.t0).from,
                0,
                "in the mitosis hold"
            );
            let (dw, v) = v_dots(&g);
            let dh = v.len() / dw;
            let rim = layer(&g, RIM);
            // A 4-connected flood from the medium over non-rim dots: an 8-connected rim
            // blocks it, so if every rim is closed it never reaches a cell's interior.
            let mut seen = vec![false; v.len()];
            let mut stack: Vec<usize> = (0..v.len())
                .filter(|&i| v[i] < V_M - 0.02 && !rim.is_lit((i % dw) as i32, (i / dw) as i32))
                .collect();
            for &i in &stack {
                seen[i] = true;
            }
            while let Some(i) = stack.pop() {
                let (x, y) = (i % dw, i / dw);
                let mut push = |j: usize| {
                    if !seen[j] && !rim.is_lit((j % dw) as i32, (j / dw) as i32) {
                        seen[j] = true;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    push(i - 1);
                }
                if x + 1 < dw {
                    push(i + 1);
                }
                if y > 0 {
                    push(i - dw);
                }
                if y + 1 < dh {
                    push(i + dw);
                }
            }
            let interior = (0..v.len()).filter(|&i| v[i] > V_M + 0.03).count();
            let leaked = (0..v.len())
                .filter(|&i| seen[i] && v[i] > V_M + 0.03)
                .count();
            assert!(interior > 100, "seed {seed}: spots exist");
            assert_eq!(leaked, 0, "seed {seed} {w}x{h}: a rim is open");
        }
    }

    #[test]
    fn chem_a4_dead_field_is_reseeded() {
        let (mut g, mut ctx) = scene(None, 3, 80, 24);
        g.u.fill(1.0);
        g.v.fill(0.0);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert!(g.v.as_slice().iter().any(|&x| x > 0.05));
        assert!(g.reseeds >= 1);
    }
}
