//! A nerve impulse racing down an axon, with an oscilloscope trace of the spike
//! (SPEC-002 `action_potential`).
//!
//! Hodgkin & Huxley (1952) squid giant axon in the modern rest = −65 mV convention
//! (Dayan & Abbott 2001), on a sealed-end cable:
//!
//! ```text
//! C_m ∂V/∂t = D C_m ∂²V/∂x² − ḡ_Na m³h (V − E_Na) − ḡ_K n⁴ (V − E_K) − ḡ_L (V − E_L) + I_stim
//! ```
//!
//! with `D = a/(2 R_i C_m) = 0.336 cm²/ms` (a = 238 μm, R_i = 35.4 Ω·cm), 200
//! compartments over 10 cm (Δx = 0.5 mm, ≈ 1/14 of the resting length constant).
//!
//! ## Scheme
//!
//! Each 10 μs sub-step first advances m, h, n by Rush–Larsen (1978), which is exact for
//! frozen V and keeps every gate in `[0, 1]`, then takes a backward-Euler V step with
//! the ionic conductances frozen (Hines 1984): a tridiagonal M-matrix solved by the
//! Thomas algorithm. The diffusion term is stiff (explicit Euler would need ≤ 3.7 μs),
//! which is why `ode::rk4_step` is not used. The clock counts integer sub-steps, so how
//! frames chop time never changes the result (AP-A7).
//!
//! ## Stylisation
//!
//! The soma, dendrites and terminal arbor are drawn, not simulated (the real giant axon
//! is a syncytium grown from the stellate ganglion): the soma shows V of compartment 0
//! and the arbor fans out the last 0.8 cm of the cable.

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{draw, ease, geom, ink, ode, raster, rng};

pub static META: Meta = Meta {
    name: "action_potential",
    category: Category::Biophysics,
    description: "A nerve impulse racing down an axon, with an oscilloscope trace of the spike",
    title: "Nerve spike · ion channels fire",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ActionPotential::default())
}

// ---- Membrane (HH 1952, mS/cm² and mV) ----

const G_NA: f64 = 120.0;
const G_K: f64 = 36.0;
const G_L: f64 = 0.3;
const E_NA: f64 = 50.0;
const E_K: f64 = -77.0;
const E_L: f64 = -54.387;
const V_REST: f64 = -65.0;

// ---- Cable ----

const N: usize = 200;
const DX_CM: f64 = 0.05;
const AXON_RADIUS_CM: f64 = 0.0238;
/// Axoplasm resistivity, Ω·cm.
const R_I: f64 = 35.4;
/// Membrane capacitance, F/cm².
const C_M: f64 = 1e-6;
/// `a/(2 R_i C_m)` in cm²/ms.
const D: f64 = AXON_RADIUS_CM / (2.0 * R_I * C_M) * 1e-3;
/// Sub-step, ms. Against a converged fine-grid run (12.31 and 18.72 m/s) the velocity
/// error is 0.4 % at 6.3 °C and 0.9 % at 18.5 °C (SPEC-002).
const H_MS: f64 = 0.01;
/// Temperature factor `3^((T − 6.3)/10)` at the scene's 6.3 °C.
const PHI: f64 = 1.0;

// ---- Protocol (SPEC-002 Protocol) ----

/// Sub-steps per simulated second: 4 ms of axon time per second (250× slow motion).
const SUB_PER_S: u64 = 400;
const LOOP_STEPS: u64 = 18 * SUB_PER_S;
/// Stimulated compartments at each end: the first (or last) 0.25 cm.
const STIM_N: usize = 5;
/// 0.3 ms pulses.
const PULSE_STEPS: u64 = 30;
/// Threshold of a 0.3 ms pulse over 0.25 cm, μA/cm².
const THRESHOLD: f64 = 90.0;
const STRONG: f64 = 3.0 * THRESHOLD;
const WEAK: f64 = 0.5 * THRESHOLD;
/// A stimulus marker stays lit for 0.25 s.
const MARK_STEPS: u64 = SUB_PER_S / 4;

/// Where a pulse is injected. The protocol never stimulates the far end alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site {
    Soma,
    Both,
}

#[derive(Debug, Clone, Copy)]
struct Pulse {
    /// Loop-local sub-step of onset.
    at: u64,
    site: Site,
    /// μA/cm².
    amp: f64,
}

/// A pulse `tenths` tenths of a scene second into the loop (integer-exact).
const fn pulse(tenths: u64, site: Site, amp: f64) -> Pulse {
    Pulse {
        at: tenths * SUB_PER_S / 10,
        site,
        amp,
    }
}

/// The looping protocol: a single spike, a 100 Hz train, a collision, a sub-threshold
/// kick. Train and collision pulses start ≥ 9.5 ms after the previous spike passed their
/// site: past the absolute refractory period, inside the relative one.
const PULSES: [Pulse; 6] = [
    pulse(2, Site::Soma, STRONG),
    pulse(30, Site::Soma, STRONG),
    pulse(55, Site::Soma, STRONG),
    pulse(80, Site::Soma, STRONG),
    pulse(130, Site::Both, STRONG),
    pulse(160, Site::Soma, WEAK),
];

const BEATS: [(&str, f64); 4] = [
    ("single", 3.0),
    ("train", 8.0),
    ("collision", 4.5),
    ("threshold", 2.5),
];

// ---- Probes and readouts ----

/// Compartments at 3 cm and 7 cm (the velocity gates), the electrode at 5 cm, the end.
const AT_3CM: usize = 60;
const AT_7CM: usize = 140;
const ELECTRODE: usize = 100;
const END: usize = N - 1;
/// Trace: one sample per 0.05 ms, 400 samples = 20 ms = 5 s on screen.
const TRACE_EVERY: u64 = 5;
const TRACE_LEN: usize = 400;
const TRACE_TOP_MV: f64 = 45.0;
const TRACE_BOTTOM_MV: f64 = -85.0;

// ---- Drawing (SPEC-002 Look (M20): membrane threads + ion dust) ----

/// Compartment position (0..N−1) where the trunk hands over to the arbor: 9.2 cm.
const TRUNK_END: f64 = 184.0;
const VESICLES_PER_BOUTON: usize = 4;
const VESICLE_LIFE: f64 = 0.4;
const DENDRITES: usize = 40;
/// The soma outline, the hillock and the two axon membranes: one 1-px thread.
const MEMBRANE: Ink = Ink::new(Role::Structure, 105);
const ARBOR: Ink = Ink::new(Role::Structure, 85);
/// Dendrite thread level by branching depth: thinner branches read dimmer.
const DENDRITE_LEVEL: [u8; 4] = [110, 90, 75, 60];
/// Spines: 1–2-dot stubs, one per `SPINE_EVERY` dots of a dendrite beyond depth 1.
const SPINE: Ink = Ink::new(Role::Structure, 55);
const SPINE_EVERY: usize = 6;
/// Dendrites fan out within this angle (rad) of pointing straight left, away from the
/// axon.
const DENDRITE_CONE: f64 = 1.45;
/// Radial reach of a dendrite segment by depth, in soma radii (jittered ±20 %).
const DENDRITE_REACH: [f64; 4] = [0.9, 0.7, 0.55, 0.45];
/// Soma radii (cells) from which the dendrites fork a second and a third time: a small
/// neuron draws fewer, longer branches instead of a scribble of short ones.
const FORK_RADIUS: [f64; 2] = [4.0, 8.0];
/// Shortest dendrite segment drawn, in cells (primaries are stretched to it instead):
/// anything shorter reads as a stray speck, not a twig.
const MIN_TWIG_CELLS: f64 = 2.0;
/// The electrode's lead to the oscilloscope: a faint dotted wire.
const LEAD_INK: Ink = Ink::new(Role::Dim, 60);
/// The smallest oscilloscope panel: 4 rows (16 dots for 130 mV) by 12 columns.
const SCOPE_MIN_ROWS: i32 = 4;
const SCOPE_MIN_COLS: i32 = 12;
const TRACE_INK: Ink = Ink::new(Role::Accent, 210);
const TICK_INK: Ink = Ink::new(Role::Dim, 120);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
const ELECTRODE_INK: Ink = Ink::new(Role::Accent, 200);
/// Ion dust (CORE-R107 fixed-threshold stipple): density and ink at rest.
const DUST_REST_P: f64 = 0.03;
const DUST_REST: Ink = Ink::new(Role::Cold, 70);
/// Dust turns `Hot` this far above rest (mV), so the cable's sub-0.05 mV drift at rest
/// never tints the resting dust.
const HOT_FROM_MV: f64 = 0.5;
const SALT_DUST: u64 = 0xA1_D057;
const SALT_NUCLEUS: u64 = 0x5EED_0C1E;
const SALT_BOUTON: u64 = 0xB0_7700;
/// The nucleus: a `Dim` stipple atom of σ = `NUCLEUS_SIGMA` soma radii, with a
/// one-dot nucleolus.
const NUCLEUS: Ink = Ink::new(Role::Dim, 95);
const NUCLEUS_SIGMA: f64 = 0.28;
const NUCLEOLUS: Ink = Ink::new(Role::Structure, 170);
/// Boutons: `Structure` stipple atoms that flash `Hot` for `BOUTON_FLASH` s when a spike
/// arrives.
const BOUTON: Ink = Ink::new(Role::Structure, 120);
const BOUTON_HOT: Ink = Ink::new(Role::Hot, 240);
const BOUTON_SIGMA: f64 = 1.3;
const BOUTON_FLASH: f64 = 0.4;
/// Ion flux: Na⁺ where dV/dt > `NA_DVDT`, K⁺ where dV/dt < `K_DVDT` (mV/ms), each
/// living `ION_LIFE` s, at most `ION_CAP` at once.
const ION_CAP: usize = 160;
const ION_LIFE: f64 = 0.25;
const NA_DVDT: f64 = 20.0;
const K_DVDT: f64 = -10.0;
/// Chance per frame that a compartment past a flux threshold emits one ion.
const NA_RATE: f64 = 0.4;
const K_RATE: f64 = 0.15;
/// Where the soma outline leaves its circle for the hillock (rad above the axis), and
/// how far past the soma centre (in radii) the hillock meets the axon membranes.
const HILLOCK_ANGLE: f64 = 0.85;
const HILLOCK_REACH: f64 = 1.75;

/// Stimulus current per end, μA/cm².
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Stim {
    pub(crate) soma: f64,
    pub(crate) far: f64,
}

impl Stim {
    /// Current into compartment `i`.
    fn at(self, i: usize) -> f64 {
        let soma = if i < STIM_N { self.soma } else { 0.0 };
        let far = if i >= N - STIM_N { self.far } else { 0.0 };
        soma + far
    }
}

/// Cable state (SoA) plus the tridiagonal scratch of the implicit V step.
#[derive(Debug)]
pub(crate) struct Cable {
    pub(crate) v: [f64; N],
    pub(crate) m: [f64; N],
    pub(crate) h: [f64; N],
    pub(crate) n: [f64; N],
    off: [f64; N],
    diag: [f64; N],
    scratch: [f64; N],
}

impl Default for Cable {
    /// Every compartment at rest with its gates at steady state.
    fn default() -> Self {
        let [am, bm, ah, bh, an, bn] = rates(V_REST);
        Self {
            v: [V_REST; N],
            m: [am / (am + bm); N],
            h: [ah / (ah + bh); N],
            n: [an / (an + bn); N],
            off: [0.0; N],
            diag: [0.0; N],
            scratch: [0.0; N],
        }
    }
}

/// `x / (1 − e^{−x})`, with its removable singularity at 0 taken by the series
/// `1 + x/2 + x²/12` (the x³ term vanishes).
fn vtrap(x: f64) -> f64 {
    if x.abs() < 1e-6 {
        1.0 + x / 2.0 + x * x / 12.0
    } else {
        x / -(-x).exp_m1()
    }
}

/// HH rate constants at `v` mV, ms⁻¹, at φ = 1: `[α_m, β_m, α_h, β_h, α_n, β_n]`.
pub(crate) fn rates(v: f64) -> [f64; 6] {
    [
        vtrap((v + 40.0) / 10.0),
        4.0 * (-(v + 65.0) / 18.0).exp(),
        0.07 * (-(v + 65.0) / 20.0).exp(),
        1.0 / (1.0 + (-(v + 35.0) / 10.0).exp()),
        0.1 * vtrap((v + 55.0) / 10.0),
        0.125 * (-(v + 65.0) / 80.0).exp(),
    ]
}

/// Rush–Larsen: the exact update of `dx/dt = α(1 − x) − βx` over `dt` for frozen rates.
fn rush_larsen(x: f64, alpha: f64, beta: f64, dt: f64) -> f64 {
    let sum = alpha + beta;
    let inf = alpha / sum;
    inf + (x - inf) * (-dt * sum).exp()
}

/// One sub-step of `h_ms`: gates by Rush–Larsen, then backward-Euler V with frozen
/// conductances through `ode::tridiag_solve` (module docs). `phi` scales every rate.
pub(crate) fn hh_substep(c: &mut Cable, h_ms: f64, phi: f64, stim: Stim) {
    let hr = h_ms * D / (DX_CM * DX_CM);
    c.off.fill(-hr);
    for i in 0..N {
        let v = c.v[i];
        let [am, bm, ah, bh, an, bn] = rates(v).map(|r| r * phi);
        let m = rush_larsen(c.m[i], am, bm, h_ms);
        let h = rush_larsen(c.h[i], ah, bh, h_ms);
        let n = rush_larsen(c.n[i], an, bn, h_ms);
        (c.m[i], c.h[i], c.n[i]) = (m, h, n);
        let g_na = G_NA * m * m * m * h;
        let n2 = n * n;
        let g_k = G_K * n2 * n2;
        // Sealed ends: an end compartment couples to one neighbour only.
        let links = if i == 0 || i == N - 1 { 1.0 } else { 2.0 };
        c.diag[i] = 1.0 + h_ms * (g_na + g_k + G_L) + links * hr;
        c.v[i] = v + h_ms * (g_na * E_NA + g_k * E_K + G_L * E_L + stim.at(i));
    }
    // Diagonally dominant with unit-plus diagonal: the pivots never vanish. A failed
    // solve would leave the right-hand side in `v`, so say so loudly in debug builds.
    let solved = ode::tridiag_solve(&c.off, &c.diag, &c.off, &mut c.v, &mut c.scratch);
    debug_assert!(solved, "HH cable matrix is diagonally dominant");
}

/// The stimulus at global sub-step `step` (the protocol loops every 18 s).
pub(crate) fn pulses_at(step: u64) -> Stim {
    let local = step % LOOP_STEPS;
    let mut s = Stim::default();
    for p in PULSES
        .iter()
        .filter(|p| (p.at..p.at + PULSE_STEPS).contains(&local))
    {
        s.soma += p.amp;
        if p.site == Site::Both {
            s.far += p.amp;
        }
    }
    s
}

/// Stimulus marker levels `(soma, far)` at global sub-step `step`: lit for 0.25 s after
/// each pulse at a level proportional to its amplitude, unlit (`None`) otherwise.
fn marker_levels(step: u64) -> (Option<u8>, Option<u8>) {
    let local = step % LOOP_STEPS;
    let mut levels = (None, None);
    for p in PULSES
        .iter()
        .filter(|p| (p.at..p.at + MARK_STEPS).contains(&local))
    {
        let lit = Some(ink::level(255.0 * p.amp / STRONG));
        levels.0 = lit;
        if p.site == Site::Both {
            levels.1 = lit;
        }
    }
    levels
}

/// Conduction velocity from 0 mV up-crossings at 3 cm and 7 cm. A 7 cm crossing counts
/// only 1.5–6 ms after the latest 3 cm one, so antidromic and colliding spikes are
/// ignored and the readout holds.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VelocityMeter {
    t3: Option<f64>,
    /// Latest orthodromic velocity, m/s.
    pub(crate) speed: Option<f64>,
}

impl VelocityMeter {
    pub(crate) fn at_3cm(&mut self, t_ms: f64) {
        self.t3 = Some(t_ms);
    }

    pub(crate) fn at_7cm(&mut self, t_ms: f64) {
        if let Some(t3) = self.t3 {
            let dt = t_ms - t3;
            if (1.5..=6.0).contains(&dt) {
                // 4 cm in dt ms, and 1 cm/ms = 10 m/s.
                self.speed = Some(40.0 / dt);
            }
        }
    }
}

/// Fraction of a sub-step at which V rose through 0 mV, if it did.
fn up_crossing(before: f64, after: f64) -> Option<f64> {
    (before < 0.0 && after >= 0.0).then(|| -before / (after - before))
}

/// The membrane potential on SPEC-002's signed scale: `√((V − V_rest)/105 mV)` above
/// rest (≈ +0.99 at the peak; the square root keeps millivolt-scale responses visible),
/// `−0.55·((V_rest − V)/12 mV)^1.5` below it (≈ −0.48 at the −75.9 mV minimum, so only
/// the deep, recent after-hyperpolarisation reads).
fn potential_x(v: f64) -> f64 {
    let d = v - V_REST;
    if d >= 0.0 {
        (d / 105.0).sqrt()
    } else {
        -0.55 * (-d / 12.0).powf(1.5)
    }
}

/// The ion dust at `v` (SPEC-002 Look (M20)): stipple density and ink. 0.03 at rest;
/// `0.03 + 0.50·x` in `Hot` above it and `0.03 + 0.30·|x|` in `Cold` below. A resting
/// axon is sparse cool dust, a spike a dense bright puff, its wake a cool trail.
pub(crate) fn dust(v: f64) -> (f64, Ink) {
    let x = potential_x(v);
    if x >= 0.0 {
        let ink = if v - V_REST > HOT_FROM_MV {
            Ink::new(Role::Hot, ink::level(140.0 + 115.0 * x))
        } else {
            DUST_REST
        };
        (DUST_REST_P + 0.50 * x, ink)
    } else {
        let ink = Ink::new(Role::Cold, ink::level(70.0 - 260.0 * x));
        (DUST_REST_P - 0.30 * x, ink)
    }
}

/// A terminal-arbor dot at `v`: `Hot` while well depolarised, `Cold` in the deep wake,
/// otherwise the thread's outline.
fn arbor_ink(v: f64) -> Ink {
    let x = potential_x(v);
    if x > 0.25 {
        Ink::new(Role::Hot, ink::level(140.0 + 115.0 * x))
    } else if x < -0.25 {
        Ink::new(Role::Cold, ink::level(70.0 - 260.0 * x))
    } else {
        ARBOR
    }
}

/// Dot row of `mv` on the oscilloscope panel `[x0, y0, x1, y1]` (cells), clamped to it.
fn trace_y([_, y0, _, y1]: [i32; 4], mv: f64) -> f64 {
    let f = (mv - TRACE_BOTTOM_MV) / (TRACE_TOP_MV - TRACE_BOTTOM_MV);
    let (top, bottom) = (
        f64::from(Dots::CELL_H * y0),
        f64::from(Dots::CELL_H * y1 + Dots::CELL_H - 1),
    );
    bottom - f.clamp(0.0, 1.0) * (bottom - top)
}

/// V at fractional compartment position `c`, linearly interpolated.
fn v_at(v: &[f64; N], c: f64) -> f64 {
    let c = c.clamp(0.0, (N - 1) as f64);
    let i = (c as usize).min(N - 2);
    ease::lerp(v[i], v[i + 1], c - i as f64)
}

/// The quadratic Bézier from `a` to `b` pulled towards `c`, at `t`.
fn quad(a: [f64; 2], c: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (wa, wc, wb) = (u * u, 2.0 * u * t, t * t);
    [
        wa * a[0] + wc * c[0] + wb * b[0],
        wa * a[1] + wc * c[1] + wb * b[1],
    ]
}

/// The cubic Bézier with control points `p`, at `t`.
fn cubic(p: [[f64; 2]; 4], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    [
        w[0] * p[0][0] + w[1] * p[1][0] + w[2] * p[2][0] + w[3] * p[3][0],
        w[0] * p[0][1] + w[1] * p[1][1] + w[2] * p[2][1] + w[3] * p[3][1],
    ]
}

/// Length of the polyline through `pts`, an upper bound on a Bézier's length.
fn polyline_len(pts: &[[f64; 2]]) -> f64 {
    pts.windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// Walk the curve `at(t)`, `t ∈ [0, 1]`, sampled at `n + 1` points, as a connected
/// 1-px thread: `f(x, y, t)` once per dot, consecutive dots 8-adjacent.
fn walk(n: usize, at: impl Fn(f64) -> [f64; 2], mut f: impl FnMut(i32, i32, f64)) {
    let round = |p: [f64; 2]| (p[0].round() as i32, p[1].round() as i32);
    let mut prev = round(at(0.0));
    f(prev.0, prev.1, 0.0);
    let n = n.max(1);
    for k in 1..=n {
        let t = k as f64 / n as f64;
        let p = round(at(t));
        if p != prev {
            for (x, y) in geom::line(prev.0, prev.1, p.0, p.1).skip(1) {
                f(x, y, t);
            }
            prev = p;
        }
    }
}

/// Samples for a curve whose control polygon is `pts`: about two per dot of length.
fn samples(pts: &[[f64; 2]]) -> usize {
    (2.0 * polyline_len(pts)).ceil() as usize + 1
}

/// Every dot of the soma's teardrop outline and the two axon membranes, in dots: a
/// circle of radius `R` whose right side leaves at `HILLOCK_ANGLE` and tapers along a
/// cubic Bézier into the axon hillock, meeting each membrane row at `xj` without a gap,
/// then the membranes on to the trunk's end. Dots may repeat.
fn soma_outline(g: &Geo, mut f: impl FnMut(i32, i32)) {
    let [sx, sy, r] = g.soma;
    let (s, c) = HILLOCK_ANGLE.sin_cos_det();
    let a = 0.5 * f64::from(g.lb - g.ut);
    let arc = |th: f64| {
        let (s, c) = th.sin_cos_det();
        [sx + r * c, sy - r * s]
    };
    let n = (4.0 * PI * r).ceil() as usize + 4;
    walk(
        n,
        |t| arc(ease::lerp(HILLOCK_ANGLE, 2.0 * PI - HILLOCK_ANGLE, t)),
        |x, y, _| f(x, y),
    );
    for (side, row) in [(-1.0, g.ut), (1.0, g.lb)] {
        let p0 = [sx + r * c, sy + side * r * s];
        // The circle's tangent there, heading right and towards the axis.
        let t0 = [s, -side * c];
        let p3 = [g.xj, sy + side * a];
        let (k0, k1) = (0.4 * r, 0.5 * (g.xj - p0[0]).max(0.0));
        let p = [
            p0,
            [p0[0] + k0 * t0[0], p0[1] + k0 * t0[1]],
            [p3[0] - k1, p3[1]],
            p3,
        ];
        walk(samples(&p), |t| cubic(p, t), |x, y, _| f(x, y));
        for (x, y) in geom::line(g.xj.round() as i32, row, g.x1 as i32, row) {
            f(x, y);
        }
    }
}

/// A dendrite thread in dots: a quadratic Bézier from `a` to `b` bowed by `ctrl`.
// The drawing is baked into `art`; the segments stay for the AP-A9 and AP-A13 checks.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
struct Seg {
    a: [f64; 2],
    ctrl: [f64; 2],
    b: [f64; 2],
    depth: usize,
    ink: Ink,
}

/// One arbor dot and the compartment position whose V it shows.
#[derive(Debug, Clone, Copy)]
struct ArborDot {
    x: i32,
    y: i32,
    c: f64,
}

/// One dot of the axoplasm or soma interior, stippled by the V at compartment `c`.
#[derive(Debug, Clone, Copy)]
struct BodyDot {
    x: i32,
    y: i32,
    c: f32,
}

/// One dot of the static drawing: outline, membranes, nucleus, dendrites, lead.
#[derive(Debug, Clone, Copy)]
struct ArtDot {
    x: i32,
    y: i32,
    ink: Ink,
}

/// One decimated oscilloscope column, in dots (y grows downwards): the span it covers,
/// joined to the previous column, and the rows where the trace enters and leaves it.
#[derive(Debug, Clone, Copy, Default)]
struct TraceCol {
    x: i32,
    top: i32,
    bottom: i32,
    entry: i32,
    exit: i32,
}

#[derive(Debug, Clone, Copy)]
struct Vesicle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
}

/// A Na⁺ (`na`) or K⁺ ion crossing the membrane at compartment `c`, in dots.
#[derive(Debug, Clone, Copy)]
struct Ion {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
    /// The emitting compartment (AP-A12 checks its dV/dt).
    #[cfg_attr(not(test), allow(dead_code))]
    c: usize,
    na: bool,
}

/// Na⁺ ions are `Hot`, K⁺ ions `Cold`, both fading over their 0.25 s.
fn ion_ink(ion: &Ion) -> Ink {
    let fade = (1.0 - ion.age / ION_LIFE).clamp(0.0, 1.0);
    let level = ink::level(130.0 + 110.0 * fade);
    Ink::new(if ion.na { Role::Hot } else { Role::Cold }, level)
}

/// One dendrite segment in normalised tree space, drawn from `ctx.rng` once and fitted
/// to the space left of the soma on every resize. It ends at the node `(angle, rho)`:
/// polar about the soma centre, `rho` soma radii beyond the rim. A primary starts on
/// the rim at its own angle; every other segment starts at its parent's end. `wedge`
/// is the angular wedge its subtree keeps to, and `bow` how far (as a share of its
/// length) its thread bows towards the wedge's free side.
#[derive(Debug, Clone, Copy, Default)]
struct Branch {
    parent: Option<usize>,
    angle: f64,
    rho: f64,
    depth: usize,
    wedge: (f64, f64),
    bow: f64,
}

#[derive(Debug, Clone, Copy)]
struct Tree {
    branches: [Branch; DENDRITES],
    len: usize,
}

impl Tree {
    /// A radial tree: three primaries fan left, one up, one down and one to a random
    /// side, so the stimulus marker straight left of the soma stays clear. Each owns a
    /// wedge of the cone, and every fork splits its wedge at the parent's angle between
    /// its two children, so subtrees never cross: each lies in its own convex wedge
    /// about the soma centre. Every branch forks twice and about half the twigs once
    /// more (21 + ~6 segments). The bows are drawn after the topology, so they leave it
    /// as it was before the threads were curved.
    fn grow(rng: &mut impl Rng) -> Self {
        let mut t = Tree {
            branches: [Branch::default(); DENDRITES],
            len: 0,
        };
        let mut wedges = [(0.0, 0.0); DENDRITES];
        let third = if rng.random_bool(0.5) { 0.55 } else { -0.55 };
        let spread = [-1.15, third, 1.15].map(|s| s + rng.random_range(-0.08..0.08));
        let bounds = [
            -DENDRITE_CONE,
            0.5 * (spread[0] + spread[1]),
            0.5 * (spread[1] + spread[2]),
            DENDRITE_CONE,
        ];
        for (k, s) in spread.into_iter().enumerate() {
            wedges[t.len] = (PI + bounds[k], PI + bounds[k + 1]);
            t.push(Branch {
                parent: None,
                angle: PI + s,
                rho: DENDRITE_REACH[0] * rng.random_range(0.8..1.2),
                ..Branch::default()
            });
        }
        let mut i = 0;
        while i < t.len {
            let b = t.branches[i];
            let (lo, hi) = wedges[i];
            if b.depth < 2 || (b.depth == 2 && rng.random_bool(0.5)) {
                // The fork splits the wedge at the parent's own angle.
                for (side, (a, z)) in [(-1.0, (lo, b.angle)), (1.0, (b.angle, hi))] {
                    let reach = DENDRITE_REACH[b.depth + 1] * rng.random_range(0.8..1.2);
                    // Turn the child 22–38° off the parent's radial line, measured at
                    // its end, 1 + rho soma radii out: a natural fork, not a sideways
                    // spur. Clamped inside its half-wedge, which keeps the tree planar.
                    let out = 1.0 + b.rho + reach;
                    let turn = (rng.random_range(0.38..0.66f64).tan() * reach / out).atan();
                    let pad = 0.15 * (z - a);
                    if t.len < DENDRITES {
                        wedges[t.len] = (a, z);
                    }
                    t.push(Branch {
                        parent: Some(i),
                        angle: (b.angle + side * turn).clamp(a + pad, z - pad),
                        rho: b.rho + reach,
                        depth: b.depth + 1,
                        ..Branch::default()
                    });
                }
            }
            i += 1;
        }
        for (b, &w) in t.branches[..t.len].iter_mut().zip(&wedges) {
            b.wedge = w;
            b.bow = rng.random_range(0.08..0.15);
        }
        t
    }

    fn push(&mut self, b: Branch) {
        if self.len < DENDRITES {
            self.branches[self.len] = b;
            self.len += 1;
        }
    }

    /// Each segment's start as `(angle, rho)`: the rim for a primary, else the parent's
    /// end node.
    fn start(&self, b: &Branch) -> (f64, f64) {
        match b.parent {
            Some(p) => (self.branches[p].angle, self.branches[p].rho),
            None => (b.angle, 0.0),
        }
    }
}

/// Size-dependent geometry, rebuilt in `resize` (pixel units unless noted; a pixel is
/// one column wide and half a row tall, i.e. 2×2 braille dots).
#[derive(Debug, Clone, Copy, Default)]
struct Layout {
    /// Full neuron (w ≥ 24, h ≥ 8); otherwise a lone axon row.
    full: bool,
    /// Top pixel row of the axon: its upper membrane, or the only row.
    tube: i32,
    /// Axoplasm pixel rows between the two membrane rows: 4 on tall screens, else 2.
    fill_px: i32,
    /// First and last pixel column of the trunk.
    trunk: (i32, i32),
    /// Soma centre and radius.
    soma: [f64; 3],
    /// Bouton boxes: left column, top row and height.
    boutons: [[i32; 3]; 5],
    bouton_w: i32,
    /// Cells of the electrode and the two stimulus markers.
    electrode: [i32; 2],
    stim: [i32; 2],
    far: [i32; 2],
    /// Oscilloscope panel in cells: `[x0, y0, x1, y1]`, inclusive.
    panel: Option<[i32; 4]>,
    /// The electrode's lead to the scope, in cells: `[x, top, bottom, end]`, a column
    /// `x` from row `bottom` up to row `top`, then along row `top` to column `end`
    /// (none when `end ≤ x`).
    lead: Option<[i32; 4]>,
}

impl Layout {
    fn new(size: Size) -> Self {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let full = w >= 24.0 && h >= 8.0;
        if !full {
            // One cell row through the middle, spanning the width.
            return Self {
                tube: 2 * (i32::from(size.height()) / 2),
                trunk: (0, i32::from(size.width()) - 1),
                ..Self::default()
            };
        }
        let (wi, hi) = (i32::from(size.width()), i32::from(size.height()));
        // A 3-row tube on tall screens, so the pulse keeps its weight in the frame.
        let fill_px = if h >= 40.0 { 4 } else { 2 };
        let rows = fill_px / 2 + 1;
        let scope = w >= 56.0 && h >= 16.0;
        // With the scope the axon sits a little below centre (the scope takes the top);
        // without it, the neuron is centred.
        let row = if scope { (0.55 * h) as i32 } else { hi / 2 - 1 }.min(hi - 2 - rows);
        let tube = 2 * row;
        let half = f64::from(rows);
        let cy = f64::from(tube) + half;
        let r = (0.22 * h).min(0.075 * w).max(3.5);
        let cx = (0.2 * w).max(r + 3.0);
        // The trunk's compartments start where the tube's outer membrane meets the rim.
        let x0 = (cx + (r * r - half * half).max(0.0).sqrt()).floor() as i32;
        let x1 = (0.84 * w) as i32;
        let xb = (0.95 * w) as i32;
        // Boutons on whole cells, symmetric about the axon: the middle one centred on
        // it, the others `gap` cells above and below it.
        let gap = ((0.07 * h).round() as i32).max(1);
        let bouton_w = if w >= 120.0 { 2 } else { 1 };
        let last = row + rows - 1;
        let boutons = [
            [xb, 2 * (row - 2 * gap), 2],
            [xb, 2 * (row - gap), 2],
            [xb, tube, fill_px + 2],
            [xb, 2 * (last + gap), 2],
            [xb, 2 * (last + 2 * gap), 2],
        ];
        let at_s = |c: f64| f64::from(x0) + c / TRUNK_END * f64::from(x1 - x0);
        // The scope keeps clear of the terminal arbor. The trace's after-
        // hyperpolarisation dip sits on the panel's bottom row, so that row ends at
        // least two rows above the top bouton (one blank row between), shrinking the
        // panel to no fewer than `SCOPE_MIN_ROWS`. Failing that, the panel ends three
        // columns left of the arbor's first column; failing that too, there is none.
        // Either way it keeps the boutons' two-column margin from the right edge.
        let panel = scope
            .then(|| {
                let (x0, y1) = ((0.625 * w).round() as i32, 1 + (0.27 * h).round() as i32);
                let above = boutons[0][1] / 2 - 2;
                if above >= SCOPE_MIN_ROWS {
                    Some([x0, 1, wi - 3, y1.min(above)])
                } else {
                    let x1 = (wi - 3).min(x1 - 3);
                    (x1 - x0 + 1 >= SCOPE_MIN_COLS).then_some([x0, 1, x1, y1])
                }
            })
            .flatten();
        let mid = row + (rows - 1) / 2;
        let electrode = [at_s(ELECTRODE as f64).round() as i32, row - 1];
        // The lead climbs from the electrode to the trace's resting baseline and runs
        // along it to one cell short of the `-65` tick. Under the tick or the panel it
        // stops a row below them instead.
        let lead = panel.map(|p| {
            let [x, y] = electrode;
            let base = trace_y(p, V_REST).round() as i32 / 4;
            let label = p[0] - 5;
            if x < label {
                [x, base, y - 1, label - 1]
            } else {
                let under = if x < p[0] { base } else { p[3] };
                [x, under + 1, y - 1, x]
            }
        });
        Self {
            full,
            tube,
            fill_px,
            trunk: (x0, x1),
            soma: [cx, cy, r],
            boutons,
            bouton_w,
            electrode,
            stim: [(cx - r).floor() as i32 - 2, mid],
            // Just past the boutons, clamped on screen for narrow terminals.
            far: [(xb + bouton_w + 1).min(wi - 1), mid],
            panel,
            lead,
        }
    }
}

/// The neuron's geometry in dots (dot `j` centred at `j`; pixel `p` spans dots `2p` and
/// `2p + 1`), derived from the `Layout`.
#[derive(Debug, Clone, Copy, Default)]
struct Geo {
    /// Dot rows of the upper and lower axon membranes, and the axis midway between.
    ut: i32,
    lb: i32,
    axis: f64,
    /// First and last dot column of the trunk, and the compartment position at its end.
    x0: f64,
    x1: f64,
    end_c: f64,
    /// Soma centre and radius; the hillock's join with the membranes; the nucleus'
    /// centre and σ.
    soma: [f64; 3],
    xj: f64,
    nucleus: [f64; 3],
}

impl Geo {
    fn new(l: &Layout) -> Self {
        if !l.full {
            // One cell row: membranes on its top and bottom dot rows, two of axoplasm.
            let row = l.tube / 2;
            return Self {
                ut: 4 * row,
                lb: 4 * row + 3,
                axis: f64::from(4 * row) + 1.5,
                x0: 0.0,
                x1: f64::from(2 * l.trunk.1 + 1),
                end_c: (N - 1) as f64,
                ..Self::default()
            };
        }
        let ut = 2 * l.tube + 1;
        let lb = 2 * (l.tube + l.fill_px + 1);
        let [cx, cy, r] = l.soma;
        // `2·cy − ½` is the axis, midway between the membrane rows.
        let (sx, sy, rr) = (2.0 * cx - 0.5, 2.0 * cy - 0.5, 2.0 * r);
        let x1 = f64::from(2 * l.trunk.1 + 1);
        Self {
            ut,
            lb,
            axis: sy,
            x0: f64::from(2 * l.trunk.0),
            x1,
            end_c: TRUNK_END,
            soma: [sx, sy, rr],
            xj: (sx + HILLOCK_REACH * rr).min(x1 - 4.0),
            nucleus: [sx - 0.1 * rr, sy, NUCLEUS_SIGMA * rr],
        }
    }

    /// Compartment position shown at dot column `x` (clamped to the trunk).
    fn col_c(&self, x: f64) -> f64 {
        ((x - self.x0) / (self.x1 - self.x0).max(1.0)).clamp(0.0, 1.0) * self.end_c
    }

    /// Dot column of compartment position `c`.
    fn col_x(&self, c: f64) -> f64 {
        self.x0 + c / self.end_c * (self.x1 - self.x0)
    }
}

#[derive(Debug)]
struct ActionPotential {
    /// The state changed since the last raster (GEN-R20: rasterised in `prepare`).
    dirty: bool,
    cable: Cable,
    meter: VelocityMeter,
    /// V at the electrode, one sample per `TRACE_EVERY` sub-steps.
    trace: Trail<f32>,
    vesicles: ParticlePool<Vesicle>,
    ions: ParticlePool<Ion>,
    /// dV/dt (mV/ms) of every compartment over the latest update.
    dvdt: [f64; N],
    /// Seconds left of the boutons' `Hot` flash.
    flash: f64,
    timeline: Timeline,
    /// Sub-steps run so far: the scene clock.
    step: u64,
    /// Fractional sub-steps owed to the clock.
    owed: f64,
    tree: Option<Tree>,
    layout: Layout,
    geo: Geo,
    /// The static drawing, the stippled interior, the arbor and the bouton centres.
    art: Vec<ArtDot>,
    body: Vec<BodyDot>,
    arbor: Vec<ArborDot>,
    boutons: [[f64; 2]; 5],
    dendrites: Vec<Seg>,
    /// The decimated trace of the latest rasterisation (the first `n_trace_cols`).
    trace_cols: [TraceCol; TRACE_LEN],
    n_trace_cols: usize,
    /// Whether the trace window holds a spike (a sample above −20 mV).
    spike_in_window: bool,
    /// Everything but the markers and text, rasterised in `prepare`; `render` blits it.
    dots: Dots,
    cells: Size,
}

impl Default for ActionPotential {
    fn default() -> Self {
        let mut trace = Trail::new(TRACE_LEN);
        for _ in 0..TRACE_LEN {
            trace.push(V_REST as f32);
        }
        Self {
            dirty: false,
            cable: Cable::default(),
            meter: VelocityMeter::default(),
            trace,
            vesicles: ParticlePool::with_capacity(5 * VESICLES_PER_BOUTON),
            ions: ParticlePool::with_capacity(ION_CAP),
            dvdt: [0.0; N],
            flash: 0.0,
            timeline: Timeline::new(&BEATS),
            step: 0,
            owed: 0.0,
            tree: None,
            layout: Layout::default(),
            geo: Geo::default(),
            art: Vec::new(),
            body: Vec::new(),
            arbor: Vec::new(),
            boutons: [[0.0; 2]; 5],
            dendrites: Vec::new(),
            trace_cols: [TraceCol::default(); TRACE_LEN],
            n_trace_cols: 0,
            spike_in_window: false,
            dots: Dots::default(),
            cells: Size::new(1, 1),
        }
    }
}

impl ActionPotential {
    /// One protocol sub-step, then the probes: velocity gates, terminal release, trace.
    fn substep(&mut self, rng: &mut impl Rng) {
        if self.step > 0 && self.step % LOOP_STEPS == 0 {
            // A new loop starts a new measurement: `--.-` until its first spike lands,
            // never the previous loop's slowest train spike beside `single`.
            self.meter = VelocityMeter::default();
        }
        let v = &self.cable.v;
        let before = [v[AT_3CM], v[AT_7CM], v[END]];
        hh_substep(&mut self.cable, H_MS, PHI, pulses_at(self.step));
        let v = &self.cable.v;
        let t0 = self.step as f64 * H_MS;
        if let Some(f) = up_crossing(before[0], v[AT_3CM]) {
            self.meter.at_3cm(t0 + f * H_MS);
        }
        if let Some(f) = up_crossing(before[1], v[AT_7CM]) {
            self.meter.at_7cm(t0 + f * H_MS);
        }
        if up_crossing(before[2], v[END]).is_some() {
            self.release(rng);
        }
        self.step += 1;
        if self.step % TRACE_EVERY == 0 {
            self.trace.push(self.cable.v[ELECTRODE] as f32);
        }
    }

    /// The spike reaches the terminals: the boutons flash and release a puff of
    /// transmitter that drifts across the cleft (dots).
    fn release(&mut self, rng: &mut impl Rng) {
        self.flash = BOUTON_FLASH;
        if !self.layout.full {
            return;
        }
        for [bx, by] in self.boutons {
            for _ in 0..VESICLES_PER_BOUTON {
                let (sin, cos) = rng.random_range(-1.0..1.0f64).sin_cos_det();
                let speed = rng.random_range(5.0..10.0);
                self.vesicles.spawn(Vesicle {
                    x: bx + 2.0,
                    y: by,
                    vx: speed * cos,
                    vy: speed * sin,
                    age: 0.0,
                });
            }
        }
    }

    /// Ion flux (SPEC-002 Look (M20)): where a trunk compartment depolarises fast a Na⁺
    /// ion enters from outside the membrane, drifting to the axis; where it repolarises
    /// fast a K⁺ ion leaves from inside, drifting out across the membrane.
    fn emit_ions(&mut self, rng: &mut impl Rng) {
        if !self.layout.full {
            return;
        }
        let g = self.geo;
        let a = 0.5 * f64::from(g.lb - g.ut);
        for i in 0..=TRUNK_END as usize {
            let d = self.dvdt[i];
            let na = d > NA_DVDT;
            if !na && d >= K_DVDT {
                continue;
            }
            if !rng.random_bool(if na { NA_RATE } else { K_RATE }) {
                continue;
            }
            let x = g.col_x(i as f64) + rng.random_range(-1.0..1.0);
            // Which membrane, as the direction away from the axis.
            let out = if rng.random_bool(0.5) { -1.0 } else { 1.0 };
            let membrane = g.axis + out * a;
            let (y, vy) = if na {
                (membrane + 2.0 * out, -out * (a + 1.0) / ION_LIFE)
            } else {
                (membrane - out, out * 4.0 / ION_LIFE)
            };
            self.ions.spawn(Ion {
                x,
                y,
                vx: rng.random_range(-2.0..2.0),
                vy,
                age: 0.0,
                c: i,
                na,
            });
        }
    }

    fn rebuild_geometry(&mut self) {
        let l = self.layout;
        let g = Geo::new(&l);
        self.geo = g;
        self.art.clear();
        self.body.clear();
        self.arbor.clear();
        self.dendrites.clear();
        if !l.full {
            for x in 0..=g.x1 as i32 {
                for y in [g.ut, g.lb] {
                    self.art.push(ArtDot {
                        x,
                        y,
                        ink: MEMBRANE,
                    });
                }
                let c = g.col_c(f64::from(x)) as f32;
                for y in g.ut + 1..g.lb {
                    self.body.push(BodyDot { x, y, c });
                }
            }
            return;
        }
        if let Some([x, top, bottom, end]) = l.lead {
            // Every other dot: a faint dotted wire. The run along the baseline shares
            // its dot row, so the wire reads as the scope's input line.
            for y in (4 * top..4 * bottom + 4).filter(|y| y % 2 == 1) {
                self.art.push(ArtDot {
                    x: 2 * x,
                    y,
                    ink: LEAD_INK,
                });
            }
            if let Some(panel) = l.panel {
                let y = trace_y(panel, V_REST).round() as i32;
                for dx in (2 * x + 1..2 * end + 2).step_by(2) {
                    self.art.push(ArtDot {
                        x: dx,
                        y,
                        ink: LEAD_INK,
                    });
                }
            }
        }
        self.build_soma();
        self.build_arbor();
        self.build_dendrites();
    }

    /// The teardrop outline and membranes, the nucleus, and the interior they enclose:
    /// a 4-connected flood from the soma centre, bounded by the outline and the
    /// membranes, up to the trunk's end.
    fn build_soma(&mut self) {
        let g = self.geo;
        let [sx, sy, r] = g.soma;
        let dots = Dots::new(self.cells).dot_size();
        let (bx0, bx1) = (((sx - r).floor() as i32 - 2).max(0), g.x1 as i32);
        let by0 = ((sy - r).floor() as i32 - 2).max(0);
        let by1 = ((sy + r).ceil() as i32 + 2).min(i32::from(dots.height()) - 1);
        let bw = (bx1 - bx0 + 1).max(0) as usize;
        let bh = (by1 - by0 + 1).max(0) as usize;
        let mut seen = vec![false; bw * bh];
        let idx = |x: i32, y: i32| {
            ((bx0..=bx1).contains(&x) && (by0..=by1).contains(&y))
                .then(|| (y - by0) as usize * bw + (x - bx0) as usize)
        };
        let art = &mut self.art;
        soma_outline(&g, |x, y| {
            art.push(ArtDot {
                x,
                y,
                ink: MEMBRANE,
            });
            if let Some(i) = idx(x, y) {
                seen[i] = true;
            }
        });
        let [nx, ny, sigma] = g.nucleus;
        let clear = (1.5 * sigma).powi(2);
        let start = (sx.round() as i32, sy.round() as i32);
        let mut todo = Vec::new();
        if let Some(i) = idx(start.0, start.1)
            && !seen[i]
        {
            seen[i] = true;
            todo.push(start);
        }
        while let Some((x, y)) = todo.pop() {
            let (dx, dy) = (f64::from(x) - nx, f64::from(y) - ny);
            if dx * dx + dy * dy > clear {
                let c = g.col_c(f64::from(x)) as f32;
                self.body.push(BodyDot { x, y, c });
            }
            for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (x, y) = (x + ox, y + oy);
                if let Some(i) = idx(x, y)
                    && !seen[i]
                {
                    seen[i] = true;
                    todo.push((x, y));
                }
            }
        }
        // The nucleus: a Gaussian stipple atom (as `Dots::stipple_puff`), centre always
        // lit, with a one-dot nucleolus.
        let reach = (3.0 * sigma).ceil() as i32;
        let (cx, cy) = (nx.round() as i32, ny.round() as i32);
        for y in cy - reach..=cy + reach {
            for x in cx - reach..=cx + reach {
                let (dx, dy) = (f64::from(x) - nx, f64::from(y) - ny);
                let p = 0.6 * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
                if (x, y) == (cx, cy) || raster::stipple_keep(SALT_NUCLEUS, x, y, p) {
                    self.art.push(ArtDot { x, y, ink: NUCLEUS });
                }
            }
        }
        self.art.push(ArtDot {
            x: cx + 1,
            y: cy - 1,
            ink: NUCLEOLUS,
        });
    }

    /// The terminal arbor: the trunk end forks into three curved threads, the outer two
    /// forking again, each ending at a bouton. Primaries cover 9.2–9.6 cm, the forks
    /// 9.6–10 cm; the middle branch all of it.
    fn build_arbor(&mut self) {
        let (l, g) = (self.layout, self.geo);
        self.boutons = l.boutons.map(|b| {
            [
                2.0 * f64::from(b[0]) + f64::from(l.bouton_w) - 0.5,
                2.0 * f64::from(b[1]) + f64::from(b[2]) - 0.5,
            ]
        });
        let start = [g.x1 + 1.0, g.axis];
        let tips = self.boutons.map(|b| [b[0] - 1.0, b[1]]);
        let fork =
            |a: [f64; 2], b: [f64; 2]| [a[0] + 0.45 * (b[0] - a[0]), a[1] + 0.6 * (b[1] - a[1])];
        let (up, down) = (fork(start, tips[0]), fork(start, tips[4]));
        // A primary leaves the trunk level and bends towards its fork; a twig leaves the
        // fork steeply and arrives level at its bouton.
        let primary = |a: [f64; 2], b: [f64; 2]| [a[0] + 0.6 * (b[0] - a[0]), a[1]];
        let twig = |a: [f64; 2], b: [f64; 2]| [a[0] + 0.4 * (b[0] - a[0]), b[1]];
        let middle = [0.5 * (start[0] + tips[2][0]), start[1]];
        let mid_c = 0.5 * (TRUNK_END + (N - 1) as f64);
        let end_c = (N - 1) as f64;
        let segs = [
            (start, primary(start, up), up, TRUNK_END, mid_c),
            (start, primary(start, down), down, TRUNK_END, mid_c),
            (start, middle, tips[2], TRUNK_END, end_c),
            (up, twig(up, tips[0]), tips[0], mid_c, end_c),
            (up, twig(up, tips[1]), tips[1], mid_c, end_c),
            (down, twig(down, tips[3]), tips[3], mid_c, end_c),
            (down, twig(down, tips[4]), tips[4], mid_c, end_c),
        ];
        for (a, ctrl, b, c0, c1) in segs {
            let arbor = &mut self.arbor;
            walk(
                samples(&[a, ctrl, b]),
                |t| quad(a, ctrl, b, t),
                |x, y, t| {
                    arbor.push(ArborDot {
                        x,
                        y,
                        c: ease::lerp(c0, c1, t),
                    });
                },
            );
        }
    }

    /// Dendrites: scale each primary's subtree as large as fits left of, above or
    /// below the soma: a node `(angle, rho)` sits `r + k·rho` from the soma centre.
    /// Subtrees keep to their own wedges, so scaling them apart keeps the tree planar;
    /// the up and down ones may grow up to 1.6× the tightest, since terminal rows are
    /// coarse and the room is mostly above and below. Each segment is a thread bowed
    /// towards its wedge's free side, with spines beyond depth 1.
    fn build_dendrites(&mut self) {
        let Some(tree) = self.tree else { return };
        let l = self.layout;
        let [cx, cy, r] = l.soma;
        let dot = |x: f64, y: f64| [2.0 * x - 0.5, 2.0 * y - 0.5];
        let max_depth = 1 + FORK_RADIUS.iter().filter(|&&f| r >= f).count();
        let (lo, top, bottom) = (1.0, 1.0, 2.0 * f64::from(self.cells.height()) - 4.0);
        let mut root = [0; DENDRITES];
        let mut ks = [1.3 * r; 3];
        for (i, b) in tree.branches[..tree.len].iter().enumerate() {
            root[i] = b.parent.map_or(i, |p| root[p]);
            if b.depth > max_depth {
                continue;
            }
            let (cos, sin) = (b.angle.cos_det(), b.angle.sin_det());
            let room = |free: f64, dir: f64| (free / dir - r) / b.rho;
            let k = &mut ks[root[i]];
            if cos < 0.0 {
                *k = k.min(room(cx - lo, -cos));
            }
            if sin < 0.0 {
                *k = k.min(room(cy - top, -sin));
            } else if sin > 0.0 {
                *k = k.min(room(bottom - cy, sin));
            }
        }
        let tightest = ks.iter().fold(f64::INFINITY, |m, &k| m.min(k)).max(0.0);
        let ks = ks.map(|k| k.clamp(0.0, 1.6 * tightest));
        let node = |(angle, rho): (f64, f64), k: f64| {
            let d = r + k * rho;
            dot(cx + d * angle.cos_det(), cy + d * angle.sin_det())
        };
        // Twigs too short to read are specks: drop them with their subtrees. The
        // primaries stay, stretched to the minimum, so a small neuron keeps its
        // dendrites.
        let mut drawn = [false; DENDRITES];
        for (i, b) in tree.branches[..tree.len].iter().enumerate() {
            let k = ks[root[i]];
            let (start, mut end) = (node(tree.start(b), k), node((b.angle, b.rho), k));
            let cells = |a: [f64; 2], e: [f64; 2]| {
                ((e[0] - a[0]) / f64::from(Dots::CELL_W))
                    .abs()
                    .max(((e[1] - a[1]) / f64::from(Dots::CELL_H)).abs())
            };
            let short = cells(start, end) < MIN_TWIG_CELLS;
            // A stretched primary stays straight: its children start on its radial line.
            let mut bow = b.bow;
            match b.parent {
                Some(p) if !drawn[p] || short || b.depth > max_depth => continue,
                None if short => {
                    let (dx, dy) = (b.angle.cos_det(), b.angle.sin_det());
                    let reach = MIN_TWIG_CELLS / cells([0.0, 0.0], [dx, dy]);
                    end = [start[0] + reach * dx, start[1] + reach * dy];
                    bow = 0.0;
                }
                _ => {}
            }
            drawn[i] = true;
            // Bow towards the wedge's middle, the side the subtree leaves free.
            let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
            let len = dx.hypot(dy).max(1e-9);
            let normal = [-dy / len, dx / len];
            let mid = [0.5 * (start[0] + end[0]), 0.5 * (start[1] + end[1])];
            // A primary bows towards its wedge's middle; a fork's child away from its
            // sibling, which lies across the parent's angle.
            let away = match b.parent {
                None => 0.5 * (b.wedge.0 + b.wedge.1),
                Some(p) => 2.0 * b.angle - tree.branches[p].angle,
            };
            let free = node((away, b.rho), k);
            let towards = (free[0] - mid[0]) * normal[0] + (free[1] - mid[1]) * normal[1];
            let mut push = 2.0 * bow * len * if towards < 0.0 { -1.0 } else { 1.0 };
            // The curve lies in the hull of its ends and control point: keeping the
            // control point inside the segment's own wedge keeps subtrees in disjoint
            // convex wedges, so threads still never cross.
            let centre = dot(cx, cy);
            let ctrl_at = |push: f64| [mid[0] + push * normal[0], mid[1] + push * normal[1]];
            let in_wedge = |p: [f64; 2]| {
                let a = (p[1] - centre[1])
                    .atan2(p[0] - centre[0])
                    .rem_euclid(2.0 * PI);
                (b.wedge.0 + 0.02..=b.wedge.1 - 0.02).contains(&a)
            };
            for _ in 0..6 {
                if in_wedge(ctrl_at(push)) {
                    break;
                }
                push *= 0.5;
            }
            if !in_wedge(ctrl_at(push)) {
                push = 0.0;
            }
            let ctrl = ctrl_at(push);
            let ink = Ink::new(Role::Structure, DENDRITE_LEVEL[b.depth.min(3)]);
            self.dendrites.push(Seg {
                a: start,
                ctrl,
                b: end,
                depth: b.depth,
                ink,
            });
            let art = &mut self.art;
            let mut k = 0usize;
            walk(
                samples(&[start, ctrl, end]),
                |t| quad(start, ctrl, end, t),
                |x, y, _| {
                    art.push(ArtDot { x, y, ink });
                    if b.depth > 1 && k % SPINE_EVERY == SPINE_EVERY / 2 {
                        let h = rng::hash01(((i as u64) << 32) ^ k as u64);
                        let side = if h < 0.5 { -1.0 } else { 1.0 };
                        let n = if (4.0 * h).fract() < 0.5 { 1 } else { 2 };
                        for j in 1..=n {
                            let s = side * f64::from(j);
                            art.push(ArtDot {
                                x: x + (s * normal[0]).round() as i32,
                                y: y + (s * normal[1]).round() as i32,
                                ink: SPINE,
                            });
                        }
                    }
                    k += 1;
                },
            );
        }
    }

    /// Draw the current state into the dot lattice.
    fn rasterise(&mut self) {
        self.dots.clear();
        for &ArtDot { x, y, ink } in &self.art {
            self.dots.set(x, y, ink);
        }
        if let Some(panel) = self.layout.panel {
            self.draw_trace(panel);
        }
        let v = &self.cable.v;
        for &BodyDot { x, y, c } in &self.body {
            let (p, ink) = dust(v_at(v, f64::from(c)));
            self.dots.stipple(x, y, p, SALT_DUST, ink);
        }
        if !self.layout.full {
            return;
        }
        for &ArborDot { x, y, c } in &self.arbor {
            self.dots.set(x, y, arbor_ink(v_at(v, c)));
        }
        let (ink, peak) = if self.flash > 0.0 {
            (BOUTON_HOT, 1.6)
        } else {
            (BOUTON, 1.0)
        };
        for (i, [x, y]) in self.boutons.into_iter().enumerate() {
            let salt = SALT_BOUTON + i as u64;
            self.dots.stipple_puff(x, y, BOUTON_SIGMA, peak, salt, ink);
        }
        for p in self.vesicles.iter() {
            let fade = 1.0 - p.age / VESICLE_LIFE;
            self.dots.plot(
                p.x,
                p.y,
                Ink::new(Role::Particle, (60.0 + 180.0 * fade) as u8),
            );
        }
        for ion in self.ions.iter() {
            self.dots.plot(ion.x, ion.y, ion_ink(ion));
        }
    }

    /// The oscilloscope: V at the electrode over the last 20 ms, newest at the right,
    /// over a dotted resting baseline.
    fn draw_trace(&mut self, panel: [i32; 4]) {
        let [x0, _, x1, _] = panel;
        let base = trace_y(panel, V_REST).round() as i32;
        for x in (2 * x0..=2 * x1 + 1).step_by(3) {
            self.dots.set(x, base, TICK_INK);
        }
        // One dot column per slice of samples, covering the span the slice reaches,
        // joined to the previous slice's last sample: one short line per column, and
        // the spike peak survives the decimation.
        let dot_cols = 2 * (x1 - x0 + 1);
        let len = self.trace.len();
        let cols = (dot_cols.max(1) as usize).min(len.max(1)).min(TRACE_LEN);
        let dot_y = |mv: f32| trace_y(panel, f64::from(mv)).round() as i32;
        let mut n = 0;
        let mut last = None;
        let mut peak = f32::NEG_INFINITY;
        for (k, &mv) in self.trace.iter().enumerate() {
            let col = k * cols / len;
            let y = dot_y(mv);
            peak = peak.max(mv);
            if col == n {
                let entry = last.map_or(y, dot_y);
                self.trace_cols[n] = TraceCol {
                    x: 2 * x0 + (col * dot_cols as usize / cols) as i32,
                    top: entry.min(y),
                    bottom: entry.max(y),
                    entry,
                    exit: y,
                };
                n += 1;
            } else {
                let c = &mut self.trace_cols[n - 1];
                (c.top, c.bottom, c.exit) = (c.top.min(y), c.bottom.max(y), y);
            }
            last = Some(mv);
        }
        self.n_trace_cols = n;
        self.spike_in_window = peak > -20.0;
        for c in &self.trace_cols[..n] {
            let (x, top, bottom) = (f64::from(c.x), f64::from(c.top), f64::from(c.bottom));
            self.dots.line(x, top, x, bottom, TRACE_INK);
        }
    }

    /// The trace as ASCII line art, one glyph per cell (SPEC-002 ASCII): `|` where it
    /// crosses a cell top to bottom, `/` or a backslash where it rises or falls within
    /// one, `_` or `-` for a flat run in a cell's lower or upper half. The resting
    /// baseline shows as `.` ticks only where the trace has left it.
    fn draw_trace_ascii(&self, canvas: &mut Canvas, panel: [i32; 4]) {
        let [x0, y0, x1, y1] = panel;
        // The braille trace's dot-count fallback would read as noise: clear it first.
        for y in y0..=y1 {
            for x in x0..=x1 {
                canvas.put(x, y, Glyph::ascii(' '), Ink::BLANK);
            }
        }
        let base = trace_y(panel, V_REST).round() as i32 / 4;
        for x in (x0..=x1).step_by(2) {
            canvas.put(x, base, Glyph::ascii('.'), TICK_INK);
        }
        let flat = |y: i32| if y.rem_euclid(4) >= 2 { '_' } else { '-' };
        let cols = &self.trace_cols[..self.n_trace_cols];
        for cell in cols.chunk_by(|a, b| a.x / 2 == b.x / 2) {
            let (first, last) = (cell[0], cell[cell.len() - 1]);
            let top = cell.iter().map(|c| c.top).min().unwrap_or(first.top);
            let bottom = cell.iter().map(|c| c.bottom).max().unwrap_or(first.bottom);
            let x = first.x / 2;
            let rise = last.exit - first.entry;
            if bottom - top <= 1 {
                // A flat run (≤ 2 dots of drift): one glyph where the trace leaves.
                canvas.put(x, last.exit / 4, Glyph::ascii(flat(last.exit)), TRACE_INK);
                continue;
            }
            for row in top.div_euclid(4)..=bottom.div_euclid(4) {
                let (above, below) = (top < 4 * row, bottom > 4 * row + 3);
                let glyph = match rise {
                    _ if above && below => '|',
                    ..0 => '/',
                    1.. => '\\',
                    // A spike inside one cell: up and back down again.
                    0 => '|',
                };
                canvas.put(x, row, Glyph::ascii(glyph), TRACE_INK);
            }
        }
    }

    /// The readout, right-aligned on the bottom row clear of the title (GEN-R13).
    fn draw_label(&self, canvas: &mut Canvas) {
        let h = self.cells.height();
        if h < 3 {
            return;
        }
        let room = title::readout_room(self.cells, &Title::of(self));
        let beat = self.timeline.at(self.step as f64 / SUB_PER_S as f64).label;
        let mut speed = StackText::<8>::new();
        // Writing into a StackText never fails.
        let _ = match self.meter.speed {
            Some(s) => write!(speed, "{s:.1}"),
            None => write!(speed, "--.-"),
        };
        let mut label = StackText::<80>::new();
        // Each threshold is its form's length with the longest beat and a 4-char speed,
        // so the form follows the room, never the beat. The temperature stays in every
        // form: 12.3 m/s is right for 6.3 °C, not the textbook 21 m/s at 18.5 °C.
        let _ = match room {
            62.. => write!(
                label,
                "Hodgkin-Huxley 1952 | squid axon 6.3C | v={speed} m/s | {beat}"
            ),
            38.. => write!(label, "HH squid 6.3C | v={speed} m/s | {beat}"),
            32.. => write!(label, "HH 6.3C | v={speed} m/s | {beat}"),
            7.. => write!(label, "HH 6.3C"),
            _ => return,
        };
        draw::label_right(canvas, i32::from(h) - 1, label.as_str(), LABEL_INK);
    }
}

impl Generator for ActionPotential {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        // The cable does not depend on the size: only the drawing is rebuilt.
        self.cells = size;
        if self.tree.is_none() {
            self.tree = Some(Tree::grow(&mut ctx.rng));
        }
        self.layout = Layout::new(size);
        self.dots.resize(size);
        self.vesicles.clear();
        self.ions.clear();
        self.rebuild_geometry();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.flash = (self.flash - dt).max(0.0);
        // 1e-9 of slack so that exact totals (400 sub-steps per second) never lose one.
        self.owed += dt * SUB_PER_S as f64;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        let before = self.cable.v;
        for _ in 0..n as u32 {
            self.substep(&mut ctx.rng);
        }
        self.vesicles.retain(|p| {
            p.age += dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.age < VESICLE_LIFE
        });
        self.ions.retain(|p| {
            p.age += dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.age < ION_LIFE
        });
        let ms = n * H_MS;
        for ((d, &v), &v0) in self.dvdt.iter_mut().zip(&self.cable.v).zip(&before) {
            *d = if ms > 0.0 { (v - v0) / ms } else { 0.0 };
        }
        self.emit_ions(&mut ctx.rng);
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
        let l = self.layout;
        self.dots.blit(canvas);
        // ASCII has no braille: the trace and the lead are drawn as line art instead of
        // their dot-count fallbacks.
        if canvas.mode() == GlyphMode::Ascii {
            if let Some(panel) = l.panel {
                self.draw_trace_ascii(canvas, panel);
            }
            if let Some([x, top, bottom, end]) = l.lead {
                for y in top..=bottom {
                    canvas.put(x, y, Glyph::ascii(':'), LEAD_INK);
                }
                for cx in x + 1..=end {
                    let ch = if (cx - x) % 2 == 0 { '.' } else { ' ' };
                    canvas.put(cx, top, Glyph::ascii(ch), LEAD_INK);
                }
            }
        }
        if l.full {
            let (soma, far) = marker_levels(self.step);
            let accent = |level| Ink::new(Role::Accent, level);
            if let Some(level) = soma {
                canvas.put(l.stim[0], l.stim[1], Glyph::new('▶', '>'), accent(level));
            }
            if let Some(level) = far {
                canvas.put(l.far[0], l.far[1], Glyph::new('◀', '<'), accent(level));
            }
            let [x, y] = l.electrode;
            canvas.put(x, y, Glyph::new('▾', 'v'), ELECTRODE_INK);
        }
        if let Some(panel) = l.panel {
            // '+40' marks the scale of a spike, so it shows only while one is in view;
            // alone above a flat baseline it would float.
            let ticks = [(40.0, "+40", self.spike_in_window), (V_REST, "-65", true)];
            for (mv, tick, _) in ticks.into_iter().filter(|t| t.2) {
                let row = trace_y(panel, mv).round() as i32 / 4;
                canvas.text(panel[0] - 4, row, tick, TICK_INK);
            }
        }
        self.draw_label(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `steps` sub-steps with stimulus `stim(k)`, calling `obs(k, cable)` after each.
    fn run(
        c: &mut Cable,
        phi: f64,
        steps: u64,
        stim: impl Fn(u64) -> Stim,
        mut obs: impl FnMut(u64, &Cable),
    ) {
        for k in 0..steps {
            hh_substep(c, H_MS, phi, stim(k));
            obs(k + 1, c);
        }
    }

    /// A 0.3 ms pulse of `amp` at the soma end starting at sub-step `at`.
    fn soma_pulse(at: u64, amp: f64) -> impl Fn(u64) -> Stim {
        move |k| Stim {
            soma: if (at..at + PULSE_STEPS).contains(&k) {
                amp
            } else {
                0.0
            },
            far: 0.0,
        }
    }

    /// Times (ms) of 0 mV up-crossings at each compartment of `at`, from one run that
    /// also hands every sub-step's cable to `obs`.
    fn crossings_obs(
        phi: f64,
        steps: u64,
        stim: impl Fn(u64) -> Stim,
        at: &[usize],
        mut obs: impl FnMut(u64, &Cable),
    ) -> Vec<Vec<f64>> {
        let mut c = Cable::default();
        let mut prev: Vec<f64> = at.iter().map(|&i| c.v[i]).collect();
        let mut out = vec![Vec::new(); at.len()];
        run(&mut c, phi, steps, stim, |k, c| {
            for ((&i, p), times) in at.iter().zip(&mut prev).zip(&mut out) {
                if let Some(f) = up_crossing(*p, c.v[i]) {
                    times.push((k as f64 - 1.0 + f) * H_MS);
                }
                *p = c.v[i];
            }
            obs(k, c);
        });
        out
    }

    /// Times (ms) of 0 mV up-crossings at each compartment of `at`, from one run.
    fn crossings(phi: f64, steps: u64, stim: impl Fn(u64) -> Stim, at: &[usize]) -> Vec<Vec<f64>> {
        crossings_obs(phi, steps, stim, at, |_, _| {})
    }

    fn velocity(phi: f64) -> f64 {
        let t = crossings(phi, 1500, soma_pulse(0, STRONG), &[AT_3CM, AT_7CM]);
        40.0 / (t[1][0] - t[0][0])
    }

    fn i_ion_steady(v: f64) -> f64 {
        let [am, bm, ah, bh, an, bn] = rates(v);
        let (m, h, n) = (am / (am + bm), ah / (ah + bh), an / (an + bn));
        G_NA * m.powi(3) * h * (v - E_NA) + G_K * n.powi(4) * (v - E_K) + G_L * (v - E_L)
    }

    #[test]
    fn ap_a1_rest_is_a_stable_root_and_rates_are_continuous() {
        let (mut lo, mut hi) = (-80.0, -50.0);
        assert!(i_ion_steady(lo) < 0.0 && i_ion_steady(hi) > 0.0);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if i_ion_steady(mid) < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let rest = 0.5 * (lo + hi);
        assert!((rest - V_REST).abs() <= 0.1, "rest at {rest} mV");

        let mut c = Cable::default();
        run(
            &mut c,
            PHI,
            5000,
            |_| Stim::default(),
            |_, c| {
                assert!(c.v.iter().all(|v| (v - rest).abs() <= 0.05));
            },
        );

        // Stable, not merely an equilibrium: a 1 mV depolarisation of the whole cable
        // (gates left at rest) relaxes back within 50 ms.
        let mut c = Cable {
            v: [V_REST + 1.0; N],
            ..Cable::default()
        };
        run(&mut c, PHI, 5000, |_| Stim::default(), |_, _| {});
        let worst = c.v.iter().fold(0.0f64, |m, v| m.max((v - rest).abs()));
        assert!(worst <= 0.05, "1 mV kick left {worst:.4} mV after 50 ms");

        // α_m = vtrap(x) and α_n = 0.1·vtrap(x), x = (V − V₀)/10, near their removable
        // singularities, against the tangent lines of vtrap = 1 + x/2 + x²/12: values
        // 1 and 0.1, slopes 0.05 and 0.005 per mV. The series branch holds for
        // |x| < 1e-6, i.e. |δ| < 1e-5 mV, so 0.99e-5 and 1.01e-5 mV sit either side of
        // the hand-off; the curvature term x²/12 is ≤ 8.4e-10 at δ = 1e-3 mV.
        for (v0, k, value, slope) in [(-40.0, 0, 1.0, 0.05), (-55.0, 4, 0.1, 0.005)] {
            let at = rates(v0)[k];
            assert!((at - value).abs() < 1e-12, "α{k}({v0}) = {at}");
            for d in [1e-7, 0.99e-5, 1.01e-5, 1e-3] {
                for delta in [-d, d] {
                    let a = rates(v0 + delta)[k];
                    let tangent = at + slope * delta;
                    assert!(
                        a.is_finite() && (a - tangent).abs() < 2e-9,
                        "α{k} at {v0}{delta:+e} mV: {a} vs tangent {tangent}"
                    );
                }
            }
        }
    }

    #[test]
    fn ap_a2_spike_shape_and_protocol_bounds() {
        let mut c = Cable::default();
        let mut trace = Vec::new();
        run(&mut c, PHI, 5000, soma_pulse(0, STRONG), |_, c| {
            trace.push(c.v[ELECTRODE]);
        });
        let (k_peak, &peak) = trace
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        let trough = trace.iter().copied().fold(f64::INFINITY, f64::min);
        assert!((34.0..=42.0).contains(&peak), "peak {peak:.2} mV");
        assert!((-77.0..=-74.5).contains(&trough), "AHP {trough:.2} mV");
        let later = trace[k_peak + 2500];
        assert!(
            (later - V_REST).abs() < 0.5,
            "25 ms after the peak: {later:.2} mV"
        );

        let mut c = Cable::default();
        run(&mut c, PHI, LOOP_STEPS, pulses_at, |k, c| {
            let gates = c.m.iter().chain(&c.h).chain(&c.n);
            assert!(
                gates.clone().all(|g| (0.0..=1.0).contains(g)),
                "gate out at {k}"
            );
            assert!(
                c.v.iter().all(|v| (E_K - 0.5..=E_NA).contains(v)),
                "V out at {k}"
            );
        });
    }

    #[test]
    fn ap_a3_conduction_velocity_at_6_3c() {
        let v = velocity(PHI);
        assert!((v / 12.3 - 1.0).abs() <= 0.03, "v = {v:.2} m/s");
    }

    #[test]
    fn ap_a4_conduction_velocity_at_18_5c_matches_hh() {
        let v = velocity(3f64.powf((18.5 - 6.3) / 10.0));
        assert!((v / 18.8 - 1.0).abs() <= 0.03, "v = {v:.2} m/s");
    }

    #[test]
    fn ap_a5_all_or_none() {
        let mut c = Cable::default();
        run(&mut c, PHI, 2000, soma_pulse(0, WEAK), |_, c| {
            assert!(
                c.v[AT_3CM] < -60.0,
                "weak pulse reached {} mV at 3 cm",
                c.v[AT_3CM]
            );
        });
        let peaks: Vec<f64> = [1.5, 3.0, 6.0]
            .iter()
            .map(|k| {
                let mut c = Cable::default();
                let mut peak = f64::NEG_INFINITY;
                run(&mut c, PHI, 1500, soma_pulse(0, k * THRESHOLD), |_, c| {
                    peak = peak.max(c.v[ELECTRODE]);
                });
                peak
            })
            .collect();
        let spread = peaks
            .iter()
            .fold(0.0f64, |m, p| m.max((p - peaks[0]).abs()));
        assert!(spread <= 0.3, "peaks {peaks:?}");

        // THRESHOLD itself (bisection on this scheme: 87.2 μA/cm²): ±10 % decides
        // whether a spike reaches 7 cm within 15 ms.
        for (k, fires) in [(0.9, false), (1.1, true)] {
            let n = crossings(PHI, 1500, soma_pulse(0, k * THRESHOLD), &[AT_7CM])[0].len();
            assert_eq!(n > 0, fires, "{k}× threshold: {n} crossings at 7 cm");
        }
    }

    #[test]
    fn ap_a6_refractoriness_and_collision() {
        let pair = |gap: u64| {
            move |k: u64| {
                let s = soma_pulse(0, STRONG)(k).soma + soma_pulse(gap, STRONG)(k).soma;
                Stim { soma: s, far: 0.0 }
            }
        };
        assert_eq!(
            crossings(PHI, 2500, pair(500), &[AT_7CM])[0].len(),
            1,
            "5 ms apart"
        );
        let t = crossings(PHI, 2500, pair(1000), &[AT_3CM, AT_7CM]);
        let (t3, t7) = (&t[0], &t[1]);
        assert_eq!((t3.len(), t7.len()), (2, 2), "10 ms apart");
        assert!(
            t7[1] - t3[1] > t7[0] - t3[0],
            "the second spike must be slower"
        );

        let both = |k: u64| {
            let a = if k < PULSE_STEPS { STRONG } else { 0.0 };
            Stim { soma: a, far: a }
        };
        // One run: the crossings, and the cable's max V after every sub-step.
        let mut max_v = Vec::new();
        let t = crossings_obs(PHI, 1500, both, &[0, ELECTRODE, END], |_, c| {
            max_v.push(c.v.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        });
        for (i, times) in [0, ELECTRODE, END].iter().zip(&t) {
            assert_eq!(times.len(), 1, "compartment {i}");
        }
        let meet = t[1][0];
        // `max_v[k - 1]` is the state after `k` sub-steps.
        let at = ((meet + 10.0) / H_MS).round() as usize;
        let max = max_v[at - 1];
        assert!(max < -60.0, "10 ms after the collision max V = {max:.2}");
    }

    fn scene() -> (ActionPotential, Ctx) {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        (ap, ctx)
    }

    #[test]
    fn ap_a7_clock_is_frame_independent_and_protocol_loops() {
        let (mut a, mut ctx_a) = scene();
        let (mut b, mut ctx_b) = scene();
        for _ in 0..60 {
            a.update(1.0 / 60.0, &mut ctx_a);
            a.prepare();
        }
        for _ in 0..4 {
            b.update(0.25, &mut ctx_b);
            b.prepare();
        }
        assert_eq!((a.step, b.step), (400, 400));
        let bits = |c: &Cable| [c.v, c.m, c.h, c.n].map(|x| x.map(f64::to_bits));
        assert!(
            bits(&a.cable) == bits(&b.cable),
            "frame chopping changed the cable"
        );

        let mut onsets = Vec::new();
        let mut last = Stim::default();
        for step in 0..2 * LOOP_STEPS {
            let s = pulses_at(step);
            if s != last && s != Stim::default() {
                onsets.push((step as f64 / SUB_PER_S as f64, s.soma, s.far));
            }
            last = s;
        }
        let one = [
            (0.2, STRONG, 0.0),
            (3.0, STRONG, 0.0),
            (5.5, STRONG, 0.0),
            (8.0, STRONG, 0.0),
            (13.0, STRONG, STRONG),
            (16.0, WEAK, 0.0),
        ];
        let two: Vec<_> = one
            .iter()
            .chain(&one)
            .enumerate()
            .map(|(i, &(t, s, f))| (t + if i < 6 { 0.0 } else { 18.0 }, s, f))
            .collect();
        assert_eq!(onsets, two);
    }

    #[test]
    fn ap_a8_live_velocity_readout_through_the_protocol() {
        let (mut ap, mut ctx) = scene();
        let mut t = 0.0;
        let mut read = |until: f64, ap: &mut ActionPotential| {
            while t < until - 1e-9 {
                ap.update(0.25, &mut ctx);
                ap.prepare();
                t += 0.25;
            }
            ap.meter.speed
        };
        assert_eq!(read(0.25, &mut ap), None, "--.- before the first spike");
        assert_eq!(
            format!("{:.1}", read(3.0, &mut ap).unwrap()),
            "12.3",
            "from rest"
        );
        for (until, expect) in [(5.5, 10.9), (8.0, 10.1), (11.0, 9.8)] {
            let v = read(until, &mut ap).expect("a reading");
            assert!(
                (v - expect).abs() <= 0.3,
                "at {until} s read {v:.2}, want {expect}"
            );
        }
        let train_end = ap.meter.speed;
        assert_eq!(
            read(15.5, &mut ap),
            train_end,
            "collision must not change it"
        );
        assert_eq!(
            read(18.0, &mut ap),
            train_end,
            "threshold must not change it"
        );
        // A new loop starts a new measurement: no stale train velocity beside `single`.
        assert_eq!(
            read(18.25, &mut ap),
            None,
            "--.- again after the loop wraps"
        );
        // Later loops start 20 ms after the collision, in its supernormal phase (h up,
        // n down): their single spike runs 0.8 % faster, and reads the same each loop.
        let shown = |v: Option<f64>| format!("{:.1}", v.expect("a reading"));
        let second = shown(read(21.0, &mut ap));
        let third = shown(read(39.0, &mut ap));
        assert_eq!((second.as_str(), third.as_str()), ("12.4", "12.4"));
    }

    /// Whether segments `p` and `q` properly cross (touching endpoints do not count).
    fn cross(p: &Seg, q: &Seg) -> bool {
        let side = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        let eps = 1e-9;
        side(p.a, p.b, q.a) * side(p.a, p.b, q.b) < -eps
            && side(q.a, q.b, p.a) * side(q.a, q.b, p.b) < -eps
    }

    #[test]
    fn ap_a9_dendrites_never_cross_and_every_twig_reads() {
        for seed in 1..=24u64 {
            for (w, h) in [
                (24u16, 8u16),
                (40, 12),
                (56, 16),
                (80, 24),
                (120, 40),
                (200, 50),
            ] {
                let size = Size::new(w, h);
                let mut ap = ActionPotential::default();
                ap.resize(size, &mut Ctx::new(seed, size));
                let d = &ap.dendrites;
                assert!(d.iter().filter(|s| s.depth == 0).count() == 3);
                for (i, p) in d.iter().enumerate() {
                    let cells = ((p.b[0] - p.a[0]) / 2.0)
                        .abs()
                        .max(((p.b[1] - p.a[1]) / 4.0).abs());
                    assert!(
                        cells >= MIN_TWIG_CELLS - 1e-9,
                        "seed {seed} {w}x{h}: segment {i} spans {cells:.2} cells"
                    );
                    for (j, q) in d[..i].iter().enumerate() {
                        assert!(
                            !cross(p, q),
                            "seed {seed} {w}x{h}: segments {j} and {i} cross: {q:?} {p:?}"
                        );
                    }
                }
            }
        }
    }

    /// The scene at `size` after `t` simulated seconds, rendered in ASCII.
    fn ascii_frame(size: Size, t: f64) -> (ActionPotential, Vec<String>) {
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        let mut done = 0.0;
        while done < t - 1e-9 {
            ap.update(SIM_DT, &mut ctx);
            ap.prepare();
            done += SIM_DT;
        }
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        ap.render(&mut canvas);
        (ap, canvas.to_lines())
    }

    #[test]
    fn ap_electrode_lead_runs_to_the_scope() {
        for (w, h) in [(56u16, 16u16), (80, 24), (120, 40), (200, 50)] {
            let (ap, lines) = ascii_frame(Size::new(w, h), 0.0);
            let l = ap.layout;
            let [x, top, bottom, end] = l.lead.expect("a lead with the scope");
            let [ex, ey] = l.electrode;
            assert_eq!(
                (x, bottom),
                (ex, ey - 1),
                "{w}x{h}: it leaves the electrode"
            );
            let panel = l.panel.unwrap();
            let base = trace_y(panel, V_REST).round() as i32 / 4;
            let at = |x: i32, y: i32| lines[y as usize].chars().nth(x as usize).unwrap();
            for y in top..=bottom {
                assert_eq!(at(x, y), ':', "{w}x{h}: lead at row {y}");
            }
            if x < panel[0] - 5 {
                // Up to the baseline, then along it to one blank cell before the
                // `-65` tick.
                assert_eq!((top, end), (base, panel[0] - 6), "{w}x{h}");
                for x in (x + 2..=end).step_by(2) {
                    assert_eq!(at(x, top), '.', "{w}x{h}: lead at column {x}");
                }
                assert_eq!(at(end + 1, top), ' ', "{w}x{h}");
                assert_eq!(at(end + 2, top), '-', "{w}x{h}: meets the tick");
            } else {
                // Under the tick or the panel: it stops a row below them.
                assert!(top == base + 1 || top == panel[3] + 1, "{w}x{h}");
            }
        }
        let no_scope = Layout::new(Size::new(40, 12));
        assert!(no_scope.lead.is_none());
    }

    #[test]
    fn ap_layout_fits_every_size() {
        for w in 24u16..=200 {
            for h in [8u16, 12, 16, 24, 50] {
                let size = Size::new(w, h);
                let l = Layout::new(size);
                let far = l.far;
                assert!(
                    (0..i32::from(w)).contains(&far[0]) && (0..i32::from(h)).contains(&far[1]),
                    "{w}x{h}: far marker at {far:?}"
                );
                if let Some([x0, y0, x1, y1]) = l.panel {
                    // The scope never touches the terminal arbor: its bottom row (the
                    // trace's dip) ends two rows above the top bouton, one blank row
                    // between, or it ends three columns left of the arbor.
                    let top = l.boutons[0][1] / 2;
                    let arbor = l.trunk.1;
                    assert!(
                        y1 <= top - 2 || x1 <= arbor - 3,
                        "{w}x{h}: panel {:?} vs arbor from x {arbor}, row {top}",
                        [x0, y0, x1, y1]
                    );
                    assert!(y1 - y0 + 1 >= SCOPE_MIN_ROWS && x1 - x0 + 1 >= SCOPE_MIN_COLS);
                    // The trace's lowest point, the −76 mV dip, lies within the panel.
                    let dip = trace_y([x0, y0, x1, y1], -76.0).round() as i32 / 4;
                    assert!(dip <= y1, "{w}x{h}: dip row {dip}");
                }
            }
        }
        // Small tier: no scope, so the axon is centred.
        let l = Layout::new(Size::new(40, 12));
        assert!(l.panel.is_none());
        assert_eq!(l.tube / 2, 5, "axon rows centred at 40x12");
        assert!(l.soma[2] >= 3.5, "soma radius {}", l.soma[2]);
        // Large tier: a 3-row tube.
        let l = Layout::new(Size::new(200, 50));
        assert_eq!(l.fill_px, 4);
        assert_eq!(Layout::new(Size::new(80, 24)).fill_px, 2);
    }

    #[test]
    fn ap_small_tier_keeps_its_primary_dendrites() {
        let size = Size::new(40, 12);
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        let tree = ap.tree.expect("grown on the first resize");
        let grown = tree.branches[..tree.len].iter().filter(|b| b.depth == 0);
        assert_eq!(grown.count(), 3);
        // Primaries are the only segments drawn at the brightest dendrite level.
        let drawn = ap
            .dendrites
            .iter()
            .filter(|d| d.ink.level == DENDRITE_LEVEL[0]);
        assert_eq!(drawn.count(), 3, "every primary dendrite is drawn");
    }

    /// The bottom row of a blank `w`×12 canvas holding only the readout, at scene time `t`.
    fn readout(w: u16, t: f64, speed: Option<f64>) -> String {
        let size = Size::new(w, 12);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut Ctx::new(1, size));
        ap.step = (t * SUB_PER_S as f64) as u64;
        ap.meter.speed = speed;
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        ap.draw_label(&mut canvas);
        canvas.to_lines().swap_remove(11)
    }

    #[test]
    fn ap_gen_r13_label_forms_follow_the_room_right_of_the_title() {
        let long = "Hodgkin-Huxley 1952 | squid axon 6.3C | v=--.- m/s | single";
        let mid = "HH squid 6.3C | v=--.- m/s | single";
        let short = "HH 6.3C | v=--.- m/s | single";
        // The room is w − 2 minus the title's gutter and text and the 4-cell gap, and at
        // most w/2 (CORE-R76): min(w − 37, w/2) from w = 55, where the full title fits,
        // min(w − 17, w/2) for the phenomenon below that, and w/2 below w = 22, where the
        // title hides.
        for (w, want) in [
            (124u16, long),
            (123, mid),
            (76, mid),
            (75, short),
            (69, short),
            (68, "HH 6.3C"),
            (55, "HH 6.3C"),
            (54, "HH 6.3C"),
            (24, "HH 6.3C"),
            (23, ""),
            (21, "HH 6.3C"),
            (14, "HH 6.3C"),
            (13, ""),
        ] {
            let row = readout(w, 0.0, None);
            // Whole, never clipped, and ending one blank cell from the right edge.
            assert_eq!(row.trim(), want, "w={w}");
            assert!(row.ends_with(&format!("{want} ")), "w={w}: {row:?}");
        }
        // The longest beats fit the least room of their form.
        for (w, t, want) in [
            (76, 14.0, "HH squid 6.3C | v=12.3 m/s | collision"),
            (69, 16.0, "HH 6.3C | v=12.3 m/s | threshold"),
        ] {
            assert_eq!(readout(w, t, Some(12.3)).trim(), want, "w={w}");
        }
    }

    #[test]
    fn ap_scope_leaves_a_margin_and_shows_its_scale_with_a_spike() {
        for w in [56u16, 80, 120, 200, 257] {
            for h in [16u16, 24, 50] {
                if let Some([_, _, x1, _]) = Layout::new(Size::new(w, h)).panel {
                    assert!(x1 <= i32::from(w) - 3, "{w}x{h}: panel ends at {x1}");
                }
            }
        }
        // '+40' labels the spike's peak, so it shows once the window holds one: the
        // first spike passes the electrode ≈ 1.1 s after the 0.2 s stimulus.
        let (_, rest) = ascii_frame(Size::new(80, 24), 0.0);
        assert!(!rest.join("\n").contains("+40"), "{}", rest.join("\n"));
        let (_, spike) = ascii_frame(Size::new(80, 24), 2.0);
        assert!(spike.join("\n").contains("+40"), "{}", spike.join("\n"));
        assert!(rest.join("\n").contains("-65"));
    }

    /// The cells of `lines` inside `[x0, y0, x1, y1]`, row by row.
    fn region(lines: &[String], [x0, y0, x1, y1]: [i32; 4]) -> Vec<Vec<char>> {
        (y0..=y1)
            .map(|y| {
                let row: Vec<char> = lines[y as usize].chars().collect();
                (x0..=x1).map(|x| row[x as usize]).collect()
            })
            .collect()
    }

    #[test]
    fn ap_ascii_trace_is_line_art() {
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            for t in [0.0, 2.0, 5.0] {
                let (ap, lines) = ascii_frame(Size::new(w, h), t);
                let panel = ap.layout.panel.expect("a scope at this size");
                let cells = region(&lines, panel);
                let all: String = cells.iter().flatten().collect();
                let frame = lines.join("\n");
                assert!(
                    all.chars().all(|c| " _-/\\|.".contains(c)),
                    "{w}x{h} t={t}: the trace uses density glyphs:\n{frame}"
                );
                // At rest the trace is one flat run along the baseline.
                let flat = all.chars().filter(|&c| c == '_' || c == '-').count();
                assert!(flat >= 10, "{w}x{h} t={t}: flat run of {flat}:\n{frame}");
                if t > 0.0 {
                    // Each spike in the window: a vertical upstroke, a falling edge.
                    assert!(
                        all.contains('|') && all.contains('\\'),
                        "{w}x{h} t={t}:\n{frame}"
                    );
                }
            }
        }
    }

    // ---- Look (M20): membrane threads + ion dust ----

    /// The scene at `size` (seed 42) after `t` s of `SIM_DT` frames, rendered in Unicode.
    fn unicode_frame(size: Size, t: f64) -> (ActionPotential, Canvas) {
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        let mut done = 0.0;
        while done < t - 1e-9 {
            ap.update(SIM_DT, &mut ctx);
            ap.prepare();
            done += SIM_DT;
        }
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        ap.render(&mut canvas);
        (ap, canvas)
    }

    #[test]
    fn ap_a10_no_half_blocks_and_mostly_braille() {
        let size = Size::new(200, 50);
        for t in [0.5, 1.5, 9.0] {
            let (_, canvas) = unicode_frame(size, t);
            let (mut lit, mut braille) = (0u32, 0u32);
            for y in 0..49 {
                for x in 0..200 {
                    let c = canvas.get(x, y).unwrap();
                    assert!(
                        c.bg == Ink::BLANK && !matches!(c.ch, '▀' | '▄' | '█'),
                        "t={t}: half-block cell at ({x}, {y})"
                    );
                    if c.ch == ' ' || c.ch == '\u{2800}' {
                        continue;
                    }
                    lit += 1;
                    braille += u32::from(('\u{2801}'..='\u{28FF}').contains(&c.ch));
                }
            }
            let share = f64::from(braille) / f64::from(lit.max(1));
            assert!(share >= 0.85, "t={t}: braille share {share:.3} of {lit}");
        }
    }

    #[test]
    fn ap_a11_dust_density_is_a_pure_function_of_v() {
        let (p, ink) = dust(V_REST);
        assert!((p - 0.03).abs() < 1e-12, "rest density {p}");
        assert_eq!(ink.role, Role::Cold, "resting dust is cool");
        let mut last = p;
        for k in 1..=1150 {
            let v = V_REST + f64::from(k) * 0.1;
            let (p, ink) = dust(v);
            assert!(p >= last, "density falls at {v:.1} mV");
            last = p;
            if k >= 10 {
                assert_eq!(ink.role, Role::Hot, "{v:.1} mV");
            }
        }
        last = 0.03;
        for k in 1..=150 {
            let v = V_REST - f64::from(k) * 0.1;
            let (p, ink) = dust(v);
            assert!(p >= last, "density falls at {v:.1} mV");
            last = p;
            assert_eq!(ink.role, Role::Cold, "{v:.1} mV");
        }
        // At 1.0 s the spike is a dense puff a third of the way along the axon.
        let (ap, _) = unicode_frame(Size::new(200, 50), 1.0);
        let v = &ap.cable.v;
        let peak = (0..=TRUNK_END as usize)
            .max_by(|&a, &b| v[a].total_cmp(&v[b]))
            .unwrap() as f64;
        let density = |sel: &dyn Fn(f64) -> bool| {
            let (mut n, mut lit) = (0u32, 0u32);
            for d in ap.body.iter().filter(|d| sel(f64::from(d.c))) {
                n += 1;
                lit += u32::from(ap.dots.is_lit(d.x, d.y));
            }
            assert!(n > 0);
            f64::from(lit) / f64::from(n)
        };
        let near = density(&|c| c > 0.0 && (c - peak).abs() <= 10.0);
        let far = density(&|c| c >= 100.0);
        assert!(
            far > 0.0 && near >= 8.0 * far,
            "peak at {peak}: near {near:.3} vs far {far:.3}"
        );
    }

    #[test]
    fn ap_a12_ions_flow_with_the_currents() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        let axis = ap.geo.axis;
        let (mut na, mut k) = (0u32, 0u32);
        for f in 1..=(10 * 60) {
            ap.update(SIM_DT, &mut ctx);
            ap.prepare();
            assert!(ap.ions.len() <= ION_CAP, "pool over capacity");
            if f64::from(f) * SIM_DT < 0.2 - 1e-9 {
                assert!(ap.ions.is_empty(), "ions at rest, frame {f}");
            }
            for ion in ap.ions.iter().filter(|i| i.age == 0.0) {
                let d = ap.dvdt[ion.c];
                let out = ion.vy * (ion.y - axis);
                if ion.na {
                    assert!(d > 20.0 && out < 0.0, "Na at dV/dt {d:.1}, {out}");
                    assert_eq!(ion_ink(ion).role, Role::Hot);
                    na += 1;
                } else {
                    assert!(d < -10.0 && out > 0.0, "K at dV/dt {d:.1}, {out}");
                    assert_eq!(ion_ink(ion).role, Role::Cold);
                    k += 1;
                }
            }
        }
        assert!(na >= 20 && k >= 20, "{na} Na, {k} K ions");
    }

    #[test]
    fn ap_a13_soma_outline_closed_nucleus_inside_dendrites_never_cross() {
        use std::collections::HashSet;
        for (w, h) in [
            (24u16, 8u16),
            (40, 12),
            (56, 16),
            (80, 24),
            (120, 40),
            (200, 50),
        ] {
            for seed in 1..=24u64 {
                let size = Size::new(w, h);
                let mut ap = ActionPotential::default();
                ap.resize(size, &mut Ctx::new(seed, size));
                if seed == 1 {
                    let g = ap.geo;
                    let mut wall = HashSet::new();
                    soma_outline(&g, |x, y| {
                        wall.insert((x, y));
                    });
                    let [sx, sy, r] = g.soma;
                    let start = (sx.round() as i32, sy.round() as i32);
                    let mut seen = HashSet::from([start]);
                    let mut todo = vec![start];
                    let stop = g.xj.ceil() as i32 + 3;
                    while let Some((x, y)) = todo.pop() {
                        assert!(
                            f64::from(x) >= sx - r - 2.0 && (f64::from(y) - sy).abs() <= r + 2.0,
                            "{w}x{h}: the soma leaks at ({x}, {y})"
                        );
                        if f64::from(x) > g.xj + 0.5 {
                            assert!(
                                g.ut < y && y < g.lb,
                                "{w}x{h}: gap at the hillock, ({x}, {y})"
                            );
                        }
                        if x >= stop {
                            continue;
                        }
                        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            let n = (x + dx, y + dy);
                            if !wall.contains(&n) && seen.insert(n) {
                                todo.push(n);
                            }
                        }
                    }
                    let [nx, ny, sigma] = g.nucleus;
                    let reach = (2.0 * sigma).ceil() as i32;
                    for dy in -reach..=reach {
                        for dx in -reach..=reach {
                            if f64::from(dx * dx + dy * dy) <= 4.0 * sigma * sigma {
                                let p = (nx.round() as i32 + dx, ny.round() as i32 + dy);
                                assert!(seen.contains(&p), "{w}x{h}: nucleus dot {p:?} outside");
                            }
                        }
                    }
                }
                // Each dendrite thread as 12 chords: no two threads cross.
                let chords: Vec<(usize, Seg)> = ap
                    .dendrites
                    .iter()
                    .enumerate()
                    .flat_map(|(i, s)| {
                        (0..12).map(move |k| {
                            let at = |t: f64| quad(s.a, s.ctrl, s.b, t);
                            let (t0, t1) = (f64::from(k) / 12.0, f64::from(k + 1) / 12.0);
                            (
                                i,
                                Seg {
                                    a: at(t0),
                                    b: at(t1),
                                    ..*s
                                },
                            )
                        })
                    })
                    .collect();
                for (n, (i, p)) in chords.iter().enumerate() {
                    for (j, q) in &chords[..n] {
                        assert!(
                            i == j || !cross(p, q),
                            "seed {seed} {w}x{h}: threads {j} and {i} cross: {:?} {:?}",
                            ap.dendrites[*j],
                            ap.dendrites[*i]
                        );
                    }
                }
            }
        }
    }
}
