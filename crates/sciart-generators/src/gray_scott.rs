//! Living Turing patterns from the Gray–Scott reaction–diffusion system
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
//! one node is exactly one half-block pixel, above it the lattice stops growing and the
//! pattern zooms.
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
use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::{draw, ease, ink};

pub static META: Meta = Meta {
    name: "gray_scott",
    category: Category::Chemistry,
    description: "Gray-Scott reaction-diffusion growing living Turing patterns",
    title: "Chemical spots · feed and kill",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(GrayScott::new(None))
}

/// Diffusion of the autocatalyst V, nodes² per reaction time unit. Pearson's value (1e-5 at
/// h = 0.01, i.e. 0.1 here) gives ~10-node spots, too coarse for a terminal; the pattern
/// wavelength scales as √D, so 0.03 gives ~5-node spots.
const D_V: f32 = 0.03;
/// D_u = 2 D_v (SPEC): the substrate must outrun the activator for Turing instability.
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
/// v that maps to full brightness. Patterned v peaks at ≈ 0.3–0.45, and broad plateaus
/// (coral, the tops of large spots) sit at ≈ 0.3–0.37: with headroom above them they read
/// yellow-orange (level ≈ 200–225) and only spot cores reach the pale-yellow top (0.34
/// put 36 % of the coral beat's pixels there, with no internal grading).
const V_FULL: f32 = 0.40;
/// Below this share of `V_FULL` (v = 0.12) a pixel is substrate, not pattern: the thin
/// tails between neighbouring spots and loops fall to the halo or to black, so dense
/// regimes keep a near-black majority (CHEM-A8; tuning history in SPEC-002). Spots lose
/// only their outermost ring, which the halo still grades.
const Q_MIN: f32 = 0.3;
/// Substrate depletion `1 − u` where the dim halo starts to fade in, where it saturates,
/// and its peak Seq value.
const HALO_MIN: f32 = 0.2;
const HALO_FULL: f32 = 0.6;
const HALO_PEAK: f32 = 0.1;
/// The label shows the preset name only from this width on: narrower, it would take most
/// of row 0 and cut a notch into the pattern.
const LABEL_NAME_MIN: u16 = 56;
/// The corner label: level 195 because it sits on the brightest backgrounds of any scene,
/// inside a blank gutter.
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

/// Seq level of a pattern pixel for `q = v / V_FULL` at index `round(255 q)`: the ramp
/// starts at 0.12 at the pattern edge `q = Q_MIN`, just above the halo's 0.1, so edges
/// stay graded, and `p^0.8` (p the position between the edge and full scale) opens up
/// the mid tones of the spot bodies.
fn level_table() -> [u8; 256] {
    let edge = f64::from(Q_MIN);
    std::array::from_fn(|i| {
        let p = ((i as f64 / 255.0 - edge) / (1.0 - edge)).max(0.0);
        ink::seq(0.12 + 0.88 * p.powf(0.8)).level
    })
}

/// The ink of one pixel (CHEM-A7): pattern where `v` is at least [`Q_MIN`] of full scale
/// (v = 0.12); else a dim violet halo that fades in with substrate depletion; else
/// `None`, the terminal's own background. `u` is only read (and so only sampled) off the
/// pattern.
pub(crate) fn v_ink(levels: &[u8; 256], u: impl FnOnce() -> f32, v: f32) -> Option<Ink> {
    let q = (v / V_FULL).clamp(0.0, 1.0);
    if q >= Q_MIN {
        return Some(Ink::new(Role::Seq, levels[(q * 255.0).round() as usize]));
    }
    let fade = ((1.0 - u() - HALO_MIN) / (HALO_FULL - HALO_MIN)).clamp(0.0, 1.0);
    let halo = ink::seq(f64::from(HALO_PEAK * fade));
    (halo.level > 0).then_some(halo)
}

/// ASCII fallback for one cell (GEN-R10, CHEM-A7): the brighter half decides, so thin
/// features survive; `.` for the stronger half of the halo (level ≥ 13), then `:` `+`
/// `#` `@` by quartile, in that pixel's ink. The faint halo fringe, which Unicode shows
/// as a fade to black, and a cell with both halves empty are left untouched.
pub(crate) fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    const HALO_DOT: u8 = 13;
    let lit = |p: Ink| (p.role == Role::Seq && p.level >= HALO_DOT).then_some(p);
    // On a tie both halves are the same Seq ink, so which one wins does not matter.
    let ink = [top, bottom]
        .into_iter()
        .filter_map(lit)
        .max_by_key(|p| p.level)?;
    const RAMP: [char; 4] = [':', '+', '#', '@'];
    let ch = match ink.level.checked_sub(32) {
        None => '.',
        Some(above) => RAMP[(usize::from(above) * 4 / 224).min(3)],
    };
    Some((ch, ink))
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
    /// Simulated seconds since start, and the seeded walk offset (a whole slot).
    t: f64,
    t0: f64,
    /// Preset slot forced by tests (any slot); `None` draws one of [`START_SLOTS`] from the
    /// RNG on the first resize.
    slot: Option<usize>,
    walk: Timeline,
    levels: [u8; 256],
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
    mask_seed: u64,
    /// Lattice → pixel sampling, the pixels, and the terminal size for the label.
    map: SampleMap,
    pixels: Pixels,
    size: Size,
}

impl GrayScott {
    pub(crate) fn new(slot: Option<usize>) -> Self {
        Self {
            t: 0.0,
            t0: 0.0,
            slot,
            walk: walk(),
            levels: level_table(),
            pending: 0.0,
            since_check: 0.0,
            reseeds: 0,
            u: Grid::default(),
            v: Grid::default(),
            u_next: Grid::default(),
            v_next: Grid::default(),
            mask: Grid::default(),
            mask_seed: 0,
            map: SampleMap::default(),
            pixels: Pixels::default(),
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

    /// Rasterise u, v into the pixels (bilinear, CORE-R52); u is sampled only off the
    /// pattern, where the halo needs it.
    fn rasterise(&mut self) {
        let (map, u, v, levels) = (&self.map, &self.u, &self.v, &self.levels);
        self.pixels.shade(|x, y| {
            let s = map.sample(v, x, y);
            v_ink(levels, || map.sample(u, x, y), s).unwrap_or(Ink::BLANK)
        });
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
        self.size = size;
        self.pixels.resize(size);
        self.map.rebuild(lattice, self.pixels.pixel_size());
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
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
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
        }
    }

    #[test]
    fn chem_a4_every_start_slot_develops_structure_without_reseeding() {
        let runs: Vec<(usize, u64)> = START_SLOTS
            .iter()
            .flat_map(|&slot| [1, 2, 3, 7].map(|seed| (slot, seed)))
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
        let runs: Vec<((u16, u16), usize, u64)> = [(80, 24), (200, 50)]
            .into_iter()
            .flat_map(|size| {
                START_SLOTS
                    .iter()
                    .flat_map(move |&slot| [1, 7].map(|seed| (size, slot, seed)))
            })
            .collect();
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
        let runs = [(0, 1u64), (0, 5), (0, 7), (0, 11), (spots, 1), (spots, 7)];
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
        }
        let (u, v) = (g.u.clone(), g.v.clone());
        g.resize(Size::new(400, 100), &mut ctx);
        g.resize(Size::new(200, 50), &mut ctx);
        assert!(g.u == u && g.v == v);

        // A new lattice resamples; the pattern survives and keeps its contrast (5 s in, so
        // the 2 s check ends inside the 8 s mitosis hold).
        for _ in 0..180 {
            g.update(SIM_DT, &mut ctx);
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
            g.render(&mut Canvas::new(Size::new(w, h), GlyphMode::Unicode));
        }
    }

    #[test]
    fn chem_a7_inks_ascii_and_label() {
        let levels = level_table();
        for u in [0.0f32, 0.5, 0.79, 0.81, 1.0] {
            let mut last = 0;
            for i in 0..=100 {
                let v = i as f32 * 0.005;
                let ink = v_ink(&levels, || u, v);
                // The halo level 0.1·clamp((1 − u − 0.2)/0.4) rounds to 0 below 1/255.
                let halo = 0.1 * ((1.0 - u - 0.2) / 0.4).clamp(0.0, 1.0);
                let quiet = v / V_FULL < Q_MIN && halo * 255.0 < 0.5;
                assert_eq!(ink.is_none(), quiet, "u {u} v {v}");
                if let Some(ink) = ink {
                    assert_eq!(ink.role, Role::Seq);
                    assert!(ink.level >= last, "level falls at u {u} v {v}");
                    last = ink.level;
                }
            }
        }
        // The halo fades in from 0 at the threshold (no hard black notch) to 0.1 at full
        // depletion, monotonically; the pattern never reads u.
        let mut last = 0;
        for i in 0..=100 {
            let u = 1.0 - i as f32 * 0.01;
            let level = v_ink(&levels, || u, 0.0).map_or(0, |ink| ink.level);
            assert!(level >= last && level <= last + 1, "halo jumps at u {u}");
            last = level;
        }
        assert_eq!(last, ink::seq(0.1).level);
        assert!(v_ink(&levels, || unreachable!(), 0.2).is_some());
        // ASCII: the faint halo fringe stays blank, the rest of the halo is '.', and the
        // brighter half decides.
        let seq = |level| Ink::new(Role::Seq, level);
        assert_eq!(ascii_cell(seq(12), Ink::BLANK), None);
        assert_eq!(ascii_cell(Ink::BLANK, seq(13)), Some(('.', seq(13))));
        assert_eq!(ascii_cell(seq(31), seq(20)), Some(('.', seq(31))));
        assert_eq!(ascii_cell(seq(32), seq(255)), Some(('@', seq(255))));
        assert_eq!(ascii_cell(Ink::BLANK, Ink::BLANK), None);

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
            for ink in [c.ink, c.bg] {
                assert!(matches!(
                    ink.role,
                    Role::Seq | Role::Text | Role::Background
                ));
            }
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
        assert!(
            lines[1..]
                .iter()
                .all(|l| l.chars().all(|c| " .:+#@".contains(c)))
        );
        let ramp = |c: char| " .:+#@".contains(c);
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

    fn ascii_frame(g: &GrayScott, size: Size) -> Frame {
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        g.render(&mut canvas);
        canvas
            .to_lines()
            .iter()
            .skip(1)
            .map(|l| l.chars().collect())
            .collect()
    }

    /// Share of the half-block pixels below the label row that are near-black: the
    /// terminal background or `Seq` below level 32 (the dim violet halo).
    fn near_black_share(g: &GrayScott) -> f64 {
        let px = g.pixels.pixel_size();
        let (mut n, mut dark) = (0usize, 0usize);
        for y in 2..i32::from(px.height()) {
            for x in 0..i32::from(px.width()) {
                let ink = g.pixels.get(x, y).unwrap_or(Ink::BLANK);
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

    #[test]
    fn chem_a4_dead_field_is_reseeded() {
        let (mut g, mut ctx) = scene(None, 3, 80, 24);
        g.u.fill(1.0);
        g.v.fill(0.0);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.v.as_slice().iter().any(|&x| x > 0.05));
        assert!(g.reseeds >= 1);
    }
}
