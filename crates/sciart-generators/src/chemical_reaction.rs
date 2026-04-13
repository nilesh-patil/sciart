//! Living Turing patterns from the Gray–Scott reaction–diffusion system
//! `U + 2V → 3V, V → P` (SPEC-002 `chemical_reaction`).
//!
//! Model (Pearson 1993, dimensionless, grid spacing 1 sim cell):
//!
//! ```text
//! ∂u/∂t = D_u ∇²u − u v² + F (1 − u)
//! ∂v/∂t = D_v ∇²v + u v² − (F + k) v          D_u = 2 D_v
//! ```
//!
//! on a torus. One sim cell covers 2×1 terminal cells (cells are ~2:1 tall, `geom::ASPECT`),
//! so the sim grid is square in screen space and the patterns look isotropic; odd columns
//! are drawn by linear interpolation between their two sim neighbours.
//!
//! ## Discretisation
//!
//! ∇² is the isotropic 9-point stencil `(4·Σ_edge + Σ_corner − 20·c) / 6` (Patra–Karttunen):
//! exact for quadratics, and its leading error term is rotation-invariant, so spots stay round
//! instead of drifting into diamonds as they do with the 5-point stencil. Its Fourier symbol
//! is `(8cx + 8cy + 4 cx·cy − 20) / 6` with `cx = cos kx`; the most negative value is `−16/3`
//! (checkerboard), versus `−8` for 5-point, which buys a larger stable step.
//!
//! Time stepping is explicit Euler with sub-step `τ` ([`TAU_SUB`]). Writing the update as a
//! weighted sum, the centre weight of `u` is `1 − τ(10/3·D_u + v² + F)` and all neighbour
//! weights are positive, so if
//!
//! ```text
//! τ · (10/3 · D_u + 1 + F_max) ≤ 1        (v ≤ 1)        and   τ · (10/3 · D_v + F + k) ≤ 1
//! ```
//!
//! the step is a convex combination plus non-negative source terms: `u, v ≥ 0` and `u ≤ 1`
//! are preserved exactly, and every linearised mode (|λ| ≤ 16/3·D + reaction rate) sits inside
//! Euler's stability disc `|1 + τλ| ≤ 1` with margin. With `D_u = 0.06` and `F ≤ 0.078` this
//! gives `τ ≤ 0.78`; we use `τ = 0.5`, which also resolves the fastest reaction time scale
//! (`1/(v² + F) ≳ 2.5` units at `v ≤ 0.6`). Both bounds are `const`-asserted for every preset.
//! `v ≤ 1` is not a strict invariant of the discrete map (it is of the ODE via `u + v ≤ 1`),
//! so the kernel clamps to `[0, 1]` as a guard; in normal
//! runs `v` peaks below 0.6 and `u` stays above 0.15, so the clamp never fires.
//!
//! Pacing: [`RATE`] reaction time units per real second, i.e. `RATE / τ` sub-steps per second
//! (≈ 13 sub-steps per 60 Hz frame). At 200×50 that is 100×50 cells × 13 ≈ 65 k cell updates
//! per frame.

use std::io::Write as _;

use rand::Rng;
use sciart_core::draw;
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "chemical_reaction",
    category: Category::PhysicsChemistry,
    description: "Gray-Scott reaction-diffusion growing living Turing patterns",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ChemicalReaction::default())
}

/// Diffusion of the autocatalyst V, sim cells² per reaction time unit. Pearson's value (1e-5 at
/// h = 0.01, i.e. 0.1 here) gives ~10-cell spots, too coarse for a terminal; the pattern
/// wavelength scales as √D, so 0.03 gives ~5-cell spots (10 columns × 5 rows).
const D_V: f32 = 0.03;
/// D_u = 2 D_v (SPEC): the substrate must outrun the activator for Turing instability.
const D_U: f32 = 2.0 * D_V;
/// Explicit Euler sub-step, reaction time units (bound derived in the module docs).
const TAU_SUB: f32 = 0.5;
/// Reaction time units per simulated second: fast enough that spots visibly bud and split
/// within a few seconds, slow enough to read as a calm, breathing culture.
const RATE: f64 = 400.0;
/// Hard cap on sub-steps per `update`, matching the largest contract `dt` of 0.25 s.
const MAX_SUBSTEPS: usize = (0.25 * RATE / TAU_SUB as f64) as usize;
/// Preset cycle: hold each preset, then morph (F, k) to the next with a smoothstep.
const HOLD: f64 = 30.0;
const MORPH: f64 = 10.0;
const PERIOD: f64 = HOLD + MORPH;
/// How often the field is checked for death / uniformity, simulated seconds.
const CHECK_EVERY: f64 = 1.0;
/// v that maps to full brightness (patterned v peaks ≈ 0.3–0.45).
const V_FULL: f32 = 0.34;
/// |∇v| (per sim cell) above which a cell is drawn as an advancing front.
const FRONT: f32 = 0.075;

const RAMP: [Glyph; 5] = [
    Glyph::new('·', '.'),
    Glyph::new('░', ':'),
    Glyph::new('▒', '+'),
    Glyph::new('▓', '#'),
    Glyph::new('█', '@'),
];
const HALO: Glyph = Glyph::new('·', '.');
const FRONT_GLYPH: [Glyph; 2] = [Glyph::new('▒', '*'), Glyph::new('▓', '%')];

/// Feed rate F and kill rate k.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Coeffs {
    f: f32,
    k: f32,
}

/// Pearson-class presets named in SPEC-002, in cycle order.
pub(crate) const PRESETS: [(&str, Coeffs); 4] = [
    (
        "mitosis",
        Coeffs {
            f: 0.0367,
            k: 0.0649,
        },
    ),
    (
        "coral",
        Coeffs {
            f: 0.0545,
            k: 0.062,
        },
    ),
    ("worms", Coeffs { f: 0.078, k: 0.061 }),
    ("spots", Coeffs { f: 0.035, k: 0.065 }),
];

// Positivity / stability of the explicit sub-step for every preset (derivation in the
// module docs), checked at compile time: the centre weight of u (with v <= 1) and of v
// stay non-negative. Interpolated (F, k) lie between presets, so they are covered too.
const _: () = {
    let mut i = 0;
    while i < PRESETS.len() {
        let c = PRESETS[i].1;
        assert!(TAU_SUB * (10.0 / 3.0 * D_U + 1.0 + c.f) <= 1.0);
        assert!(TAU_SUB * (10.0 / 3.0 * D_V + c.f + c.k) <= 1.0);
        i += 1;
    }
};

/// Isotropic 9-point Laplacian of `a` at `(x, y)` on a `w × h` torus (reference form of the
/// stencil inlined in [`gs_step`], which `chem_kernel_matches_reference_stencil` pins).
#[cfg(test)]
pub(crate) fn laplacian9(w: usize, h: usize, a: &[f32], x: usize, y: usize) -> f32 {
    let (xm, xp) = ((x + w - 1) % w, (x + 1) % w);
    let (ym, yp) = ((y + h - 1) % h, (y + 1) % h);
    let at = |x: usize, y: usize| a[y * w + x];
    let edge = at(xm, y) + at(xp, y) + at(x, ym) + at(x, yp);
    let corner = at(xm, ym) + at(xp, ym) + at(xm, yp) + at(xp, yp);
    (4.0 * edge + corner - 20.0 * at(x, y)) / 6.0
}

/// One explicit Euler sub-step of length [`TAU_SUB`] from `(u, v)` into `(un, vn)`, periodic
/// in both axes. `REACT = false` switches off every non-diffusive term (CHEM-A3).
///
/// Row-wise sweep with the three source rows as slices, so the inner loop is bounds-check
/// friendly; the wrap is resolved once per row (`ym`, `yp`) and per column (`xm`, `xp`).
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
    for y in 0..h {
        let (ym, yp) = (
            if y == 0 { h - 1 } else { y - 1 },
            if y + 1 == h { 0 } else { y + 1 },
        );
        let (u0, um, up) = (&u[y * w..][..w], &u[ym * w..][..w], &u[yp * w..][..w]);
        let (v0, vm, vp) = (&v[y * w..][..w], &v[ym * w..][..w], &v[yp * w..][..w]);
        let (uo, vo) = (&mut un[y * w..][..w], &mut vn[y * w..][..w]);
        for x in 0..w {
            let xm = if x == 0 { w - 1 } else { x - 1 };
            let xp = if x + 1 == w { 0 } else { x + 1 };
            let lap6 = |r0: &[f32], rm: &[f32], rp: &[f32]| {
                4.0 * (r0[xm] + r0[xp] + rm[x] + rp[x]) + rm[xm] + rm[xp] + rp[xm] + rp[xp]
                    - 20.0 * r0[x]
            };
            let (a, b) = (u0[x], v0[x]);
            let mut nu = a + cu * lap6(u0, um, up);
            let mut nv = b + cv * lap6(v0, vm, vp);
            if REACT {
                let uvv = TAU_SUB * a * b * b;
                nu += tf * (1.0 - a) - uvv;
                nv += uvv - tfk * b;
            }
            // Guard only (see module docs): the bounds keep u, v ≥ 0 and u ≤ 1 already.
            uo[x] = nu.clamp(0.0, 1.0);
            vo[x] = nv.clamp(0.0, 1.0);
        }
    }
}

/// `(F, k)` at simulated time `t`, plus the presets being blended and the blend weight.
/// Each preset holds for [`HOLD`] s, then smoothsteps into the next over [`MORPH`] s.
pub(crate) fn preset_at(t: f64) -> (Coeffs, usize, usize, f64) {
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
    let cycles = (t / PERIOD).floor();
    let from = (cycles as u64 % PRESETS.len() as u64) as usize;
    let to = (from + 1) % PRESETS.len();
    let phase = t - cycles * PERIOD;
    let s = ((phase - HOLD) / MORPH).clamp(0.0, 1.0);
    let m = s * s * (3.0 - 2.0 * s);
    let (a, b) = (PRESETS[from].1, PRESETS[to].1);
    let lerp = |p: f32, q: f32| p + (q - p) * m as f32;
    (
        Coeffs {
            f: lerp(a.f, b.f),
            k: lerp(a.k, b.k),
        },
        from,
        to,
        m,
    )
}

#[derive(Debug, Default)]
pub struct ChemicalReaction {
    /// Simulated seconds since start (drives the preset cycle).
    t: f64,
    /// Reaction time not yet integrated (less than one sub-step).
    pending: f64,
    since_check: f64,
    u: Grid<f32>,
    v: Grid<f32>,
    u_next: Grid<f32>,
    v_next: Grid<f32>,
    /// Terminal size.
    width: u16,
    height: u16,
}

impl ChemicalReaction {
    fn dims(&self) -> (usize, usize) {
        (usize::from(self.u.width()), usize::from(self.u.height()))
    }

    /// Drop Pearson-style perturbation discs (u = ½, v = ¼, ±5 % noise) into the field.
    fn seed_patches(&mut self, rng: &mut impl Rng) {
        let (w, h) = self.dims();
        if w == 0 || h == 0 {
            return;
        }
        let count = 3 + w * h / 500;
        for _ in 0..count {
            let (cx, cy) = (rng.random_range(0..w) as i32, rng.random_range(0..h) as i32);
            let r: i32 = rng.random_range(2..=4);
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let x = (cx + dx).rem_euclid(w as i32);
                    let y = (cy + dy).rem_euclid(h as i32);
                    let (nu, nv) = (
                        0.5 * rng.random_range(0.95..1.05f32),
                        0.25 * rng.random_range(0.95..1.05f32),
                    );
                    self.u.set(x, y, nu);
                    self.v.set(x, y, nv);
                }
            }
        }
    }

    /// True if the pattern has died out or gone uniform, so nothing would ever move again.
    fn is_featureless(&self) -> bool {
        let v = self.v.as_slice();
        if v.is_empty() {
            return false;
        }
        let n = v.len() as f64;
        let (mut s, mut s2, mut max) = (0.0f64, 0.0f64, 0.0f32);
        for &x in v {
            s += f64::from(x);
            s2 += f64::from(x) * f64::from(x);
            max = max.max(x);
        }
        let var = s2 / n - (s / n).powi(2);
        max < 0.02 || var < 1e-6
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

    /// v at terminal column `x` of sim row `row`, interpolating odd columns.
    #[inline]
    fn v_at(&self, row: &[f32], x: usize) -> f32 {
        let i = x / 2;
        if x % 2 == 0 {
            row[i]
        } else {
            0.5 * (row[i] + row[(i + 1) % row.len()])
        }
    }

    fn caption(&self, canvas: &mut Canvas) {
        let (c, from, to, m) = preset_at(self.t);
        let name = if m < 0.5 {
            PRESETS[from].0
        } else {
            PRESETS[to].0
        };
        let arrow = match canvas.mode() {
            GlyphMode::Unicode => "→",
            GlyphMode::Ascii => "->",
        };
        let mut buf = [0u8; 96];
        let len = {
            let mut cur = &mut buf[..];
            let _ = write!(
                cur,
                "  Gray-Scott  {name}  F={:.4} k={:.4}  U + 2V {arrow} 3V  ",
                c.f, c.k
            );
            96 - cur.len()
        };
        if let Ok(s) = std::str::from_utf8(&buf[..len]) {
            let y = i32::from(self.height) - 1;
            draw::text_centered(canvas, y, s, Ink::new(Role::Text, 210));
        }
    }
}

impl Generator for ChemicalReaction {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.width = size.width();
        self.height = size.height();
        let sim = Size::new(size.width().div_ceil(2), size.height());
        let fresh = self.u.is_empty();
        // Keep the overlapping culture; new territory is fresh substrate (u, v) = (1, 0),
        // which the pattern then invades.
        self.u.resize(sim, 1.0);
        self.v.resize(sim, 0.0);
        self.u_next.resize(sim, 1.0);
        self.v_next.resize(sim, 0.0);
        if fresh || self.is_featureless() {
            self.seed_patches(&mut ctx.rng);
        }
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.u.is_empty() {
            return;
        }
        self.t += dt;
        let c = preset_at(self.t).0;
        self.pending += dt * RATE;
        let tau = f64::from(TAU_SUB);
        let mut steps = 0;
        while self.pending >= tau && steps < MAX_SUBSTEPS {
            self.substep(c);
            self.pending -= tau;
            steps += 1;
        }
        self.pending = self.pending.min(tau);
        self.since_check += dt;
        if self.since_check >= CHECK_EVERY {
            self.since_check = 0.0;
            if self.is_featureless() {
                self.seed_patches(&mut ctx.rng);
            }
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (sw, sh) = self.dims();
        if sw == 0 || sh == 0 {
            return;
        }
        let (u, v) = (self.u.as_slice(), self.v.as_slice());
        let w = usize::from(self.width).min(2 * sw);
        for y in 0..sh.min(usize::from(self.height)) {
            let (ym, yp) = ((y + sh - 1) % sh, (y + 1) % sh);
            let (v0, vm, vp) = (&v[y * sw..][..sw], &v[ym * sw..][..sw], &v[yp * sw..][..sw]);
            let u0 = &u[y * sw..][..sw];
            for x in 0..w {
                let i = x / 2;
                let s = (self.v_at(v0, x) / V_FULL).min(1.0);
                let (glyph, ink) = if s < 0.08 {
                    // Substrate: a faint halo where u is depleted around the colonies.
                    let d = 1.0 - u0[i];
                    if d < 0.2 {
                        continue;
                    }
                    (
                        HALO,
                        Ink::new(Role::Cold, (40.0 + 140.0 * d.min(1.0)) as u8),
                    )
                } else {
                    let gx = 0.5 * (v0[(i + 1) % sw] - v0[(i + sw - 1) % sw]);
                    let gy = 0.5 * (vp[i] - vm[i]);
                    let grad = gx.hypot(gy);
                    if grad > FRONT && s < 0.8 {
                        let g = FRONT_GLYPH[usize::from(s > 0.45)];
                        let level = (150.0 + 100.0 * (grad / (2.0 * FRONT)).min(1.0)) as u8;
                        (g, Ink::new(Role::Accent, level))
                    } else if s > 0.55 {
                        (
                            draw::ramp(f64::from(s), &RAMP),
                            Ink::new(Role::Hot, (110.0 + 145.0 * s) as u8),
                        )
                    } else {
                        (
                            draw::ramp(f64::from(s), &RAMP),
                            Ink::new(Role::Cold, (90.0 + 200.0 * s) as u8),
                        )
                    }
                };
                canvas.put(x as i32, y as i32, glyph, ink);
            }
        }
        if self.height > 8 {
            self.caption(canvas);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variance(a: &[f32]) -> f64 {
        let n = a.len() as f64;
        let mean = a.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
        a.iter()
            .map(|&x| (f64::from(x) - mean).powi(2))
            .sum::<f64>()
            / n
    }

    fn scene(seed: u64, w: u16, h: u16) -> (ChemicalReaction, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = ChemicalReaction::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn chem_a1_fields_stay_finite_and_in_unit_interval_for_any_dt() {
        for seed in 0..3 {
            let (mut g, mut ctx) = scene(seed, 60, 20);
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
        for p in 0..4 {
            let c = preset_at(40.0 * f64::from(p)).0;
            gs_step::<true>(w, h, &u, &v, &mut un, &mut vn, c);
            assert!(un.iter().all(|&x| x == 1.0) && vn.iter().all(|&x| x == 0.0));
        }
    }

    #[test]
    fn chem_a3_pure_diffusion_conserves_mass_on_the_torus() {
        let (w, h) = (31, 13);
        let (mut g, _) = scene(4, 1, 1);
        let _ = &mut g;
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

    #[test]
    fn chem_a4_seeded_start_develops_structure_within_ten_seconds() {
        for seed in [1, 99] {
            let (mut g, mut ctx) = scene(seed, 200, 50);
            let active = |g: &ChemicalReaction| g.v.as_slice().iter().filter(|&&x| x > 0.1).count();
            let a0 = active(&g);
            for _ in 0..600 {
                g.update(SIM_DT, &mut ctx);
            }
            let var = variance(g.v.as_slice());
            // A patterned field has v ~ 0 between features and ~0.3 on them: σ ≳ 0.1.
            assert!(var > 5e-3, "seed {seed}: variance {var}");
            assert!(active(&g) > 2 * a0, "seed {seed}: patches did not grow");
        }
    }

    #[test]
    fn chem_laplacian_matches_exact_discrete_eigenvalue_of_a_fourier_mode() {
        let (w, h) = (24, 10);
        let kx = std::f32::consts::TAU * 3.0 / w as f32;
        let ky = std::f32::consts::TAU * 2.0 / h as f32;
        let a: Vec<f32> = (0..w * h)
            .map(|i| ((i % w) as f32 * kx).cos() * ((i / w) as f32 * ky).cos())
            .collect();
        // Isotropic 9-point stencil symbol: (2/3)(cx + cy + cx cy) + ... = see module docs.
        let (cx, cy) = (kx.cos(), ky.cos());
        let lambda = (4.0 * (2.0 * cx + 2.0 * cy) + 4.0 * cx * cy - 20.0) / 6.0;
        for y in 0..h {
            for x in 0..w {
                let got = laplacian9(w, h, &a, x, y);
                assert!((got - lambda * a[y * w + x]).abs() < 1e-5);
            }
        }
        // And x² has Laplacian 2 away from the seam.
        let q: Vec<f32> = (0..w * h).map(|i| ((i % w) as f32).powi(2)).collect();
        assert!((laplacian9(w, h, &q, 5, 4) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn chem_kernel_matches_reference_stencil() {
        // The fast row-sliced kernel must equal the reference Euler update
        // u + τ (D ∇²u − u v² + F(1 − u)), v + τ (D ∇²v + u v² − (F + k) v), incl. on
        // 1-wide and 2-wide tori where the wrap aliases neighbours.
        let c = Coeffs { f: 0.05, k: 0.06 };
        for (w, h) in [(9, 7), (1, 5), (2, 2), (6, 1)] {
            let u: Vec<f32> = (0..w * h).map(|i| 0.3 + 0.07 * (i % 5) as f32).collect();
            let v: Vec<f32> = (0..w * h).map(|i| 0.05 * (i % 7) as f32).collect();
            let (mut un, mut vn) = (vec![0.0; w * h], vec![0.0; w * h]);
            gs_step::<true>(w, h, &u, &v, &mut un, &mut vn, c);
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let r = u[i] * v[i] * v[i];
                    let eu = u[i]
                        + TAU_SUB * (D_U * laplacian9(w, h, &u, x, y) - r + c.f * (1.0 - u[i]));
                    let ev = v[i]
                        + TAU_SUB * (D_V * laplacian9(w, h, &v, x, y) + r - (c.f + c.k) * v[i]);
                    assert!((un[i] - eu).abs() < 1e-6 && (vn[i] - ev).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn chem_presets_hold_then_interpolate_smoothly_and_cycle() {
        let (c0, i0, _, m0) = preset_at(0.0);
        assert_eq!((c0, i0, m0), (PRESETS[0].1, 0, 0.0));
        assert_eq!(preset_at(29.0).0, PRESETS[0].1);
        assert_eq!(preset_at(40.0).0, PRESETS[1].1);
        assert_eq!(preset_at(160.0).0, PRESETS[0].1);
        // Continuity: small time steps never jump the parameters.
        let mut last = preset_at(0.0).0;
        for i in 1..20_000 {
            let c = preset_at(f64::from(i) * 0.01).0;
            assert!((c.f - last.f).abs() < 1e-3 && (c.k - last.k).abs() < 1e-3);
            last = c;
        }
    }

    #[test]
    fn chem_dead_field_is_reseeded() {
        let (mut g, mut ctx) = scene(3, 80, 24);
        g.u.fill(1.0);
        g.v.fill(0.0);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        assert!(g.v.as_slice().iter().any(|&x| x > 0.05));
    }

    #[test]
    fn chem_caption_only_when_tall_enough_and_resize_is_safe() {
        let mut s = Stage::new(build(), 7, Size::new(100, 30), GlyphMode::Unicode);
        let lines = s.render().to_lines();
        assert!(lines[29].contains("U + 2V → 3V") && lines[29].contains(PRESETS[0].0));
        let mut s = Stage::new(build(), 7, Size::new(100, 8), GlyphMode::Ascii);
        assert!(!s.render().to_lines().concat().contains("2V"));
        for (w, h) in [(1, 1), (3, 2), (200, 50), (5, 90), (1, 1)] {
            s.resize(Size::new(w, h));
            s.step(0.25);
            let _ = s.render();
        }
    }

    #[test]
    fn chem_renders_hot_cold_and_accent_inks() {
        let mut s = Stage::new(build(), 42, Size::new(200, 50), GlyphMode::Unicode);
        for _ in 0..600 {
            s.step(SIM_DT);
        }
        let frame = s.render();
        let count = |r| frame.cells().iter().filter(|c| c.ink.role == r).count();
        assert!(count(Role::Hot) > 50 && count(Role::Cold) > 50 && count(Role::Accent) > 20);
    }
}
