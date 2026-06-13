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
//! and the arbor fans out the last 0.8 cm of the cable. Membrane potential is false
//! colour in the Plasma ramp, like a voltage-dye movie: a dim indigo glow at rest,
//! shaded as a cylinder and a sphere, violet to lavender-white through the spike, and a
//! dark notch in the deep after-hyperpolarisation. Ions crossing the membrane are
//! symbolic: each short streak stands for a fixed share of the charge the live Na⁺
//! (sodium-flame yellow, falling in) or K⁺ (cool blue, drifting out) current carries
//! through that stretch of axon, and a sparse dim trickle stands for the resting leak.
//! The squid giant axon is unmyelinated, so no nodes of Ranvier are drawn.

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::title;
use sciart_core::{draw, ease, geom, ink, ode};

pub static META: Meta = Meta {
    name: "action_potential",
    category: Category::Biophysics,
    description: "A nerve impulse racing down an axon, with an oscilloscope trace of the spike",
    title: "Nerve spike · ion channels open",
    captions: &CAPTIONS,
};

/// Captions by beat: train, collision, threshold (`single` shows the title's own
/// mechanism).
const CAPTIONS: [&str; 3] = ["refractory slowing", "spikes annihilate", "all-or-none"];

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

// ---- Drawing ----

/// Compartment position (0..N−1) where the trunk hands over to the arbor: 9.2 cm.
const TRUNK_END: f64 = 184.0;
const VESICLES_PER_BOUTON: usize = 4;
const VESICLE_LIFE: f64 = 0.4;
const DENDRITES: usize = 40;
const MEMBRANE: Ink = Ink::new(Role::Structure, 100);
const RIM: Ink = Ink::new(Role::Structure, 110);
const ARBOR: Ink = Ink::new(Role::Structure, 85);
/// Dendrite outline level by branching depth: thinner branches read dimmer.
const DENDRITE_LEVEL: [u8; 4] = [110, 90, 75, 60];
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
/// Terminal arbor: trunk end → three primaries, the outer two forking into two each.
const ARBOR_SEGS: usize = 7;
/// Membrane potential's ramp (SPEC-002 Inks, AP-A11): Plasma, indigo → violet →
/// lavender-white, a discharge colour, off the gallery's amber. False colour, like a
/// voltage-dye movie.
const SPIKE: Ramp = Ramp::Plasma;
/// Ramp position at rest: the polarised membrane as a dim indigo glow, so the axoplasm
/// and soma read as living tissue between spikes. Above rest the tone climbs from here
/// to the ramp's top at the spike's peak. Below rest the long shallow tail of the
/// after-hyperpolarisation (tones above `−WAKE_FLOOR`, V ≳ −73 mV) keeps the glow, and
/// only the deep, recent wake sinks toward black, reaching it at tone `−WAKE_SPAN`
/// (≈ −76 mV, the undershoot's minimum): a dark notch just behind the spike.
/// M14 (ADR-0017 rules 1-2): the resting glow sits just above visibility (level 16, was
/// 0.16 ≈ 41), so the resting tissue is a near-black indigo whisper (L ≈ 0.21, chroma
/// ≈ 0.09) instead of a saturated violet fill, and the travelling spike is the one
/// accent. An exact level, so rest round-trips through `ink_tone` (AP-A11).
const U_REST: f64 = 16.0 / 255.0;
const WAKE_FLOOR: f64 = 0.3;
const WAKE_SPAN: f64 = 0.48;
/// Filled regions brighten the resting glow by up to `REST_SHADE` of itself toward the
/// axis or the lit side (`bulge` → 1), so they read as a cylinder and a sphere. It only
/// brightens, so a shaded resting pixel never reads as the wake, and it stays under
/// ASCII's `+` threshold.
const REST_SHADE: f64 = 0.35;
/// The oscilloscope: the spike's own violet phosphor, near the ramp's top.
const TRACE_INK: Ink = Ink::new(Role::Ramp(SPIKE), 225);
const TICK_INK: Ink = Ink::new(Role::Dim, 120);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
const ELECTRODE_INK: Ink = Ink::new(Role::Accent, 200);
/// ASCII tone thresholds (SPEC-002 ASCII): `@` from ≈ +20 mV, `#` from ≈ −20 mV, `+` from
/// ≈ −63 mV, `~` below ≈ −74 mV (the deep, recent after-hyperpolarisation only).
const X_AT: f64 = 113.5 / 127.0;
const X_HASH: f64 = 82.5 / 127.0;
const X_PLUS: f64 = 16.5 / 127.0;
const X_TILDE: f64 = -44.5 / 128.0;
/// Transmitter vesicles: pale glints, apart from the membrane's violet.
const VESICLE: Role = Role::Star;

// ---- Ions (SPEC-002 Ions) ----

/// Ion streaks alive at once, at most.
const ION_CAP: usize = 160;
/// Streaks of each species one spike sends across the whole trunk, whatever its width,
/// and the charge above `ION_FLOOR` (nC/cm²) one spike carries through a compartment
/// while that species' current dominates (measured at 6.3 °C), so each streak stands for
/// a fixed share of it. Na⁺ dominates only on the brief rising edge.
const IONS_PER_SPIKE: f64 = 12.0;
const SPIKE_CHARGE_NA: f64 = 70.0;
const SPIKE_CHARGE_K: f64 = 280.0;
/// Na⁺ inward, the current that depolarises; K⁺ outward, the one that repolarises. Each
/// has its own hue: sodium's flame-test yellow (the 589 nm D lines) and a cool blue.
const SODIUM: Role = Role::Ramp(Ramp::Sulfur);
const POTASSIUM: Role = Role::Cold;
/// The resting leak (AP-A13): at rest Na⁺ leaks in and K⁺ out through always-open
/// channels (balanced by the Na⁺/K⁺ pump, not simulated). Symbolic like the channel
/// streaks: `LEAK_PER_COL` streaks per trunk column per second, whatever V, drifting at
/// `LEAK_SLOW` of a channel streak's speed, at most `LEAK_LEVEL` of its brightness.
const LEAK_PER_COL: f64 = 0.3;
const LEAK_SLOW: f64 = 0.5;
const LEAK_LEVEL: f64 = 0.6;
const LEAK_CAP: usize = 96;
/// A streak's tail, in dots behind its head (against its motion): a 4-dot stroke.
const ION_TAIL: f64 = 3.0;
/// Channel currents below this (μA/cm²) spawn nothing: the resting leak is not shown,
/// only channels opening.
const ION_FLOOR: f64 = 25.0;
/// Na⁺ appear this many dots outside the membrane and fall onto it within `ION_LIFE`; K⁺
/// leave the membrane and drift the same distance out, fading.
const ION_REACH: f64 = 7.0;
const ION_LIFE: f64 = 0.35;

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

/// Membrane potential as a signed tone (SPEC-002 Inks): square-root transfer above rest,
/// so millivolt-scale responses show; below rest a 3/2-power one, so the shallow tail of
/// the after-hyperpolarisation stays near the background and only the deep, recent wake
/// (≈ −0.48 at the −75.9 mV minimum) reads.
fn tone(v: f64) -> f64 {
    let d = v - V_REST;
    if d >= 0.0 {
        (d / 105.0).sqrt()
    } else {
        -0.55 * (-d / 12.0).powf(1.5)
    }
}

/// A leak streak's ink (AP-A13): its species' hue at up to `LEAK_LEVEL`, fading in and
/// out over its life, so the trickle stays under the channel streaks.
fn leak_ink(ion: &Ion) -> Ink {
    let f = (ion.age * LEAK_SLOW / ION_LIFE).clamp(0.0, 1.0);
    let v = 255.0 * LEAK_LEVEL * (0.5 + 0.5 * (1.0 - (2.0 * f - 1.0).abs()));
    let role = if ion.sodium { SODIUM } else { POTASSIUM };
    Ink::new(role, ink::level(v))
}

/// Ramp position of tone `x` with the resting glow at `rest` (AP-A11): from `rest` up to
/// 1 at `x = 1`; `rest` down to `x = −WAKE_FLOOR`, then down to 0 at `x = −WAKE_SPAN`.
fn tone_u(x: f64, rest: f64) -> f64 {
    if x >= 0.0 {
        rest + (1.0 - U_REST) * x
    } else {
        let deep = ((-x - WAKE_FLOOR) / (WAKE_SPAN - WAKE_FLOOR)).clamp(0.0, 1.0);
        rest * (1.0 - deep)
    }
}

/// The ink of tone `x` on the membrane's ramp: brighter with V, a dim glow at rest.
fn tone_ink(x: f64) -> Ink {
    ink::ramp(SPIKE, tone_u(x, U_REST))
}

/// The tone an ink of [`tone_ink`] stands for (to its level's rounding; the shallow
/// tail `[−WAKE_FLOOR, 0)` reads as 0), or `None` for any other ink.
fn ink_tone(i: Ink) -> Option<f64> {
    (i.role == Role::Ramp(SPIKE)).then(|| {
        let u = ink::seq_value(i);
        if u >= U_REST {
            (u - U_REST) / (1.0 - U_REST)
        } else {
            -WAKE_FLOOR - (1.0 - u / U_REST) * (WAKE_SPAN - WAKE_FLOOR)
        }
    })
}

/// [`tone_ink`] for a filled region (axoplasm, soma) at `bulge ∈ [0, 1]` across it (1 on
/// the axis or the lit side): the resting glow brightened as a cylinder or sphere
/// ([`REST_SHADE`]), so the tissue is lit even between spikes (AP-A13).
fn fill_ink(v: f64, bulge: f64) -> Ink {
    let rest = U_REST * (1.0 + REST_SHADE * bulge.clamp(0.0, 1.0));
    ink::ramp(SPIKE, tone_u(tone(v), rest))
}

/// [`tone_ink`], except that near rest a drawn outline shows instead.
fn outline_or_potential(v: f64, outline: Ink) -> Ink {
    let x = tone(v);
    if spike_char(x).is_some() {
        tone_ink(x)
    } else {
        outline
    }
}

/// ASCII for a tone away from rest (SPEC-002 ASCII thresholds), `None` near rest.
fn spike_char(x: f64) -> Option<char> {
    if x >= X_AT {
        Some('@')
    } else if x >= X_HASH {
        Some('#')
    } else if x >= X_PLUS {
        Some('+')
    } else if x < X_TILDE {
        // V ≲ −74 mV: the deep, recent after-hyperpolarisation only. The AHP stays
        // below −71.5 mV for 6 ms (1.5 s on screen, two thirds of the axon); below
        // −74 mV it covers about a third, so the resting `=` frames each pulse.
        Some('~')
    } else {
        None
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

/// The ASCII stroke for a segment between two dot positions (dots are square): `-`
/// below 0.55 rise per run (≳ 3.6 cells per row), `|` above 2.5, else `/` or a
/// backslash by the screen direction.
fn slope_glyph(a: [f64; 2], b: [f64; 2]) -> char {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if dy.abs() < 0.55 * dx.abs() {
        '-'
    } else if dy.abs() > 2.5 * dx.abs() {
        '|'
    } else if (dx > 0.0) == (dy > 0.0) {
        '\\'
    } else {
        '/'
    }
}

/// Cells on the segment between two dot positions (2×4 dots per cell).
fn cell_line(a: [f64; 2], b: [f64; 2]) -> impl Iterator<Item = (i32, i32)> {
    let cell = |p: [f64; 2]| {
        (
            (p[0] / f64::from(Dots::CELL_W)).floor() as i32,
            (p[1] / f64::from(Dots::CELL_H)).floor() as i32,
        )
    };
    let ((x0, y0), (x1, y1)) = (cell(a), cell(b));
    geom::line(x0, y0, x1, y1)
}

/// V at fractional compartment position `c`, linearly interpolated.
fn v_at(v: &[f64; N], c: f64) -> f64 {
    let c = c.clamp(0.0, (N - 1) as f64);
    let i = (c as usize).min(N - 2);
    ease::lerp(v[i], v[i + 1], c - i as f64)
}

/// A dendrite segment in dots.
#[derive(Debug, Clone, Copy)]
struct Seg {
    a: [f64; 2],
    b: [f64; 2],
    depth: usize,
    ink: Ink,
}

/// A terminal-arbor segment in dots, with the compartment positions at its two ends.
#[derive(Debug, Clone, Copy, Default)]
struct ArborSeg {
    a: [f64; 2],
    b: [f64; 2],
    c0: f64,
    c1: f64,
}

/// One arbor dot and the compartment position whose V it shows.
#[derive(Debug, Clone, Copy)]
struct ArborDot {
    x: i32,
    y: i32,
    c: f64,
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

/// An ion dot crossing the membrane, in dots: Na⁺ falls in, K⁺ drifts out.
#[derive(Debug, Clone, Copy)]
struct Ion {
    x: f64,
    y: f64,
    /// Dots per second, toward (Na⁺) or away from (K⁺) the membrane.
    vy: f64,
    age: f64,
    sodium: bool,
}

#[derive(Debug, Clone, Copy)]
struct Vesicle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
}

/// One dendrite segment in normalised tree space, drawn from `ctx.rng` once and fitted
/// to the space left of the soma on every resize. It ends at the node `(angle, rho)`:
/// polar about the soma centre, `rho` soma radii beyond the rim. A primary starts on
/// the rim at its own angle; every other segment starts at its parent's end.
#[derive(Debug, Clone, Copy, Default)]
struct Branch {
    parent: Option<usize>,
    angle: f64,
    rho: f64,
    depth: usize,
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
    /// more (21 + ~6 segments).
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
                depth: 0,
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
                    });
                }
            }
            i += 1;
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
    /// Axoplasm pixel rows between the two membrane rows: 2, 4 or 6 by the height.
    fill_px: i32,
    /// First and last pixel column of the trunk.
    trunk: (i32, i32),
    /// Soma centre and radius.
    soma: [f64; 3],
    /// Bouton pixel blocks: left column, top row and height.
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
            // One pixel row through the middle, spanning the width.
            return Self {
                tube: 2 * (i32::from(size.height()) / 2),
                trunk: (0, i32::from(size.width()) - 1),
                ..Self::default()
            };
        }
        let (wi, hi) = (i32::from(size.width()), i32::from(size.height()));
        // The tube thickens with the screen (2, 3, then 4 rows), so the pulse keeps its
        // weight in the frame.
        let fill_px = if h >= 48.0 {
            6
        } else if h >= 36.0 {
            4
        } else {
            2
        };
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
        // The trunk leaves the soma where the tube's outer membrane meets the rim.
        let x0 = (cx + (r * r - half * half).max(0.0).sqrt()).floor() as i32;
        let x1 = (0.84 * w) as i32;
        let xb = (0.95 * w) as i32;
        // Boutons on whole cells, symmetric about the axon: the middle one is as tall
        // as the tube, the others `gap` cells above and below it.
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

    /// Whether cell `(x, y)` is part of the axon (drawn `=` at rest in ASCII).
    fn on_axon(&self, x: i32, y: i32) -> bool {
        let rows = if self.full {
            self.tube / 2..=(self.tube + self.fill_px + 1) / 2
        } else {
            self.tube / 2..=self.tube / 2
        };
        rows.contains(&y) && (self.trunk.0..=self.trunk.1).contains(&x)
    }

    /// Whether cell `(x, y)` lies in the soma's bounding box (full neuron only).
    fn near_soma(&self, x: i32, y: i32) -> bool {
        let [cx, cy, r] = self.soma;
        let cells_x = (cx - r).floor() as i32..=(cx + r).ceil() as i32;
        let cells_y = ((cy - r) / 2.0).floor() as i32..=((cy + r) / 2.0).ceil() as i32;
        self.full && cells_x.contains(&x) && cells_y.contains(&y)
    }

    /// The soma at cell resolution for ASCII: `Some(true)` on its rim, `Some(false)`
    /// inside, `None` outside. The outline is the ellipse of radii `r` × `r/2` cells
    /// (cells are two pixels tall); rim cells are those inside with a 4-neighbour
    /// outside, which gives a closed outline without gaps or bumps.
    fn soma_cell(&self, x: i32, y: i32) -> Option<bool> {
        let [cx, cy, r] = self.soma;
        let inside = |x: i32, y: i32| {
            let dx = (f64::from(x) + 0.5 - cx) / r;
            let dy = (f64::from(y) + 0.5 - 0.5 * cy) / (0.5 * r);
            dx * dx + dy * dy <= 1.0
        };
        inside(x, y).then(|| {
            [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|&(dx, dy)| !inside(x + dx, y + dy))
        })
    }
}

#[derive(Debug)]
struct ActionPotential {
    cable: Cable,
    meter: VelocityMeter,
    /// V at the electrode, one sample per `TRACE_EVERY` sub-steps.
    trace: Trail<f32>,
    vesicles: ParticlePool<Vesicle>,
    ions: ParticlePool<Ion>,
    /// The resting leak's streaks (AP-A13), apart from the channel ions.
    leak: ParticlePool<Ion>,
    timeline: Timeline,
    /// Sub-steps run so far: the scene clock.
    step: u64,
    /// Fractional sub-steps owed to the clock.
    owed: f64,
    tree: Option<Tree>,
    layout: Layout,
    /// Compartment position of every trunk pixel column.
    trunk_c: Vec<f64>,
    /// Ion streaks per trunk column per spike: `IONS_PER_SPIKE` spread over the trunk's
    /// columns.
    ion_rate: f64,
    /// `title::stable_readout_room` at this size.
    readout_room: usize,
    arbor: Vec<ArborDot>,
    arbor_segs: [ArborSeg; ARBOR_SEGS],
    dendrites: Vec<Seg>,
    /// The decimated trace of the latest rasterisation (the first `n_trace_cols`).
    trace_cols: [TraceCol; TRACE_LEN],
    n_trace_cols: usize,
    /// Whether the trace window holds a spike (a sample above −20 mV).
    spike_in_window: bool,
    /// Rasterised in `update`/`resize`; `render` only blits. `art` holds the dendrites,
    /// the arbor and the trace, which ASCII draws as line art instead, and the ions,
    /// which ASCII leaves out: its dot-count glyphs would read as voltage glyphs.
    pixels: Pixels,
    dots: Dots,
    art: Dots,
    cells: Size,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for ActionPotential {
    fn default() -> Self {
        let mut trace = Trail::new(TRACE_LEN);
        for _ in 0..TRACE_LEN {
            trace.push(V_REST as f32);
        }
        Self {
            dirty: true,
            cable: Cable::default(),
            meter: VelocityMeter::default(),
            trace,
            vesicles: ParticlePool::with_capacity(5 * VESICLES_PER_BOUTON),
            ions: ParticlePool::with_capacity(ION_CAP),
            leak: ParticlePool::with_capacity(LEAK_CAP),
            timeline: Timeline::new(&BEATS),
            step: 0,
            owed: 0.0,
            tree: None,
            layout: Layout::default(),
            trunk_c: Vec::new(),
            ion_rate: 0.0,
            readout_room: 0,
            arbor: Vec::new(),
            arbor_segs: [ArborSeg::default(); ARBOR_SEGS],
            dendrites: Vec::new(),
            trace_cols: [TraceCol::default(); TRACE_LEN],
            n_trace_cols: 0,
            spike_in_window: false,
            pixels: Pixels::default(),
            dots: Dots::default(),
            art: Dots::default(),
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

    /// A puff of transmitter from every bouton, drifting across the cleft (dots).
    fn release(&mut self, rng: &mut impl Rng) {
        if !self.layout.full {
            return;
        }
        for b in self.layout.boutons {
            let x = f64::from(2 * (b[0] + self.layout.bouton_w));
            let y = f64::from(2 * b[1] + b[2]);
            for _ in 0..VESICLES_PER_BOUTON {
                let (sin, cos) = rng.random_range(-1.0..1.0f64).sin_cos_det();
                let speed = rng.random_range(5.0..10.0);
                self.vesicles.spawn(Vesicle {
                    x,
                    y,
                    vx: speed * cos,
                    vy: speed * sin,
                    age: 0.0,
                });
            }
        }
    }

    /// Ions for the `substeps` just run (SPEC-002 Ions): per trunk column, the charge
    /// its compartment's dominant current carried above `ION_FLOOR`, as a share of that
    /// species' `SPIKE_CHARGE_*`, times `ion_rate`, is the expected number of streaks; the whole part spawns and the rest
    /// by a hashed coin, so the RNG stream is untouched. Na⁺ starts `ION_REACH` dots
    /// outside a membrane (above or below the tube, and in either dot column of its cell,
    /// by coin) and falls onto it; K⁺ starts on it and drifts out.
    fn spawn_ions(&mut self, substeps: u32) {
        let l = self.layout;
        if !l.full || substeps == 0 {
            return;
        }
        let dt_ms = f64::from(substeps) * H_MS;
        let (top, bottom) = (2 * l.tube, 2 * (l.tube + l.fill_px + 1) + 1);
        let speed = ION_REACH / ION_LIFE;
        let c = &self.cable;
        let tick = SplitMix64::mix(self.step);
        for (k, (x, &pos)) in (l.trunk.0..).zip(&self.trunk_c).enumerate() {
            let i = (pos.round() as usize).min(N - 1);
            let (m, h, n) = (c.m[i], c.h[i], c.n[i]);
            let n2 = n * n;
            let i_na = (G_NA * m * m * m * h * (c.v[i] - E_NA)).abs();
            let i_k = (G_K * n2 * n2 * (c.v[i] - E_K)).abs();
            // Only the dominant current is shown: Na⁺ on the rising edge, K⁺ on the
            // repolarising edge behind it, so the two species form separate clusters.
            let (sodium, current, charge) = if i_na >= i_k {
                (true, i_na, SPIKE_CHARGE_NA)
            } else {
                (false, i_k, SPIKE_CHARGE_K)
            };
            let expected = self.ion_rate * (current - ION_FLOOR).max(0.0) / charge * dt_ms;
            // One key per (sub-step, column, species); `hash01` finalises it, so the
            // ions of consecutive keys draw independent coins.
            let key = cell_key(k as i32, i32::from(sodium)) ^ tick;
            let count = expected.floor() as u32 + u32::from(hash01(key) < expected.fract());
            for j in 0..count {
                let coin = hash01(key.wrapping_add(u64::from(j) + 1));
                let above = coin < 0.5;
                // The coin's second bit picks the cell's left or right dot column.
                let dx = f64::from(2 * x) + f64::from(u8::from((2.0 * coin).fract() >= 0.5));
                let (membrane, out) = if above {
                    (f64::from(top) - 1.0, -1.0)
                } else {
                    (f64::from(bottom) + 1.0, 1.0)
                };
                let (y, vy) = if sodium {
                    (membrane + out * ION_REACH, -out * speed)
                } else {
                    (membrane, out * speed)
                };
                self.ions.spawn(Ion {
                    x: dx,
                    y,
                    vy,
                    age: 0.0,
                    sodium,
                });
            }
        }
    }

    /// The resting leak for `dt` seconds (AP-A13): `LEAK_PER_COL·dt` expected streaks
    /// per trunk column whatever V, each Na⁺ in or K⁺ out by a hashed coin (the RNG
    /// stream is untouched), from the same start points as the channel streaks but
    /// slower. It shows that a resting membrane is not idle, not how big the leak is.
    fn spawn_leak(&mut self, dt: f64) {
        let l = self.layout;
        if !l.full {
            return;
        }
        let (top, bottom) = (2 * l.tube, 2 * (l.tube + l.fill_px + 1) + 1);
        let speed = LEAK_SLOW * ION_REACH / ION_LIFE;
        let expected = LEAK_PER_COL * dt;
        let tick = SplitMix64::mix(self.step ^ 0x1eaf);
        for k in 0..self.trunk_c.len() {
            let x = l.trunk.0 + k as i32;
            let key = cell_key(k as i32, 7) ^ tick;
            if hash01(key) >= expected {
                continue;
            }
            let coin = hash01(key.wrapping_add(1));
            let sodium = coin < 0.5;
            let above = (4.0 * coin).fract() < 0.5;
            let dx = f64::from(2 * x) + f64::from(u8::from((8.0 * coin).fract() >= 0.5));
            let (membrane, out) = if above {
                (f64::from(top) - 1.0, -1.0)
            } else {
                (f64::from(bottom) + 1.0, 1.0)
            };
            let (y, vy) = if sodium {
                (membrane + out * ION_REACH, -out * speed)
            } else {
                (membrane, out * speed)
            };
            self.leak.spawn(Ion {
                x: dx,
                y,
                vy,
                age: 0.0,
                sodium,
            });
        }
    }

    fn rebuild_geometry(&mut self) {
        let l = self.layout;
        self.trunk_c.clear();
        let span = f64::from((l.trunk.1 - l.trunk.0).max(1));
        let end = if l.full { TRUNK_END } else { (N - 1) as f64 };
        self.trunk_c
            .extend((l.trunk.0..=l.trunk.1).map(|x| f64::from(x - l.trunk.0) / span * end));
        self.ion_rate = IONS_PER_SPIKE / self.trunk_c.len().max(1) as f64;

        self.arbor.clear();
        self.dendrites.clear();
        if !l.full {
            return;
        }
        // Arbor: trunk end → three branches; the outer two fork into two boutons each.
        // Primaries cover 9.2–9.6 cm, the forks 9.6–10 cm; the middle branch all of it.
        let cy = l.soma[1];
        let dot = |x: f64, y: f64| [2.0 * x, 2.0 * y];
        let start = dot(f64::from(l.trunk.1) + 0.5, cy);
        let tip = |b: [i32; 3]| dot(f64::from(b[0]), f64::from(b[1]) + 0.5 * f64::from(b[2]));
        let fork =
            |a: [f64; 2], b: [f64; 2]| [a[0] + 0.45 * (b[0] - a[0]), a[1] + 0.6 * (b[1] - a[1])];
        let tips = l.boutons.map(tip);
        let (up, down) = (fork(start, tips[0]), fork(start, tips[4]));
        let mid_c = 0.5 * (TRUNK_END + (N - 1) as f64);
        let end_c = (N - 1) as f64;
        let seg = |a, b, c0, c1| ArborSeg { a, b, c0, c1 };
        self.arbor_segs = [
            seg(start, up, TRUNK_END, mid_c),
            seg(start, down, TRUNK_END, mid_c),
            seg(start, tips[2], TRUNK_END, end_c),
            seg(up, tips[0], mid_c, end_c),
            seg(up, tips[1], mid_c, end_c),
            seg(down, tips[3], mid_c, end_c),
            seg(down, tips[4], mid_c, end_c),
        ];
        for ArborSeg { a, b, c0, c1 } in self.arbor_segs {
            let pts = geom::line(a[0] as i32, a[1] as i32, b[0] as i32, b[1] as i32);
            let len = (b[0] - a[0]).abs().max((b[1] - a[1]).abs()).max(1.0);
            for (k, (x, y)) in pts.enumerate() {
                let c = c0 + (c1 - c0) * (k as f64 / len).min(1.0);
                self.arbor.push(ArborDot { x, y, c });
            }
        }

        // Dendrites: scale each primary's subtree as large as fits left of, above or
        // below the soma: a node `(angle, rho)` sits `r + k·rho` from the soma centre.
        // Subtrees keep to their own wedges, so scaling them apart keeps the tree
        // planar; the up and down ones may grow up to 1.6× the tightest, since terminal
        // rows are coarse and the room is mostly above and below.
        let Some(tree) = self.tree else { return };
        let [cx, cy, r] = l.soma;
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
            match b.parent {
                Some(p) if !drawn[p] || short || b.depth > max_depth => continue,
                None if short => {
                    let (dx, dy) = (b.angle.cos_det(), b.angle.sin_det());
                    let reach = MIN_TWIG_CELLS / cells([0.0, 0.0], [dx, dy]);
                    end = [start[0] + reach * dx, start[1] + reach * dy];
                }
                _ => {}
            }
            drawn[i] = true;
            self.dendrites.push(Seg {
                a: start,
                b: end,
                depth: b.depth,
                ink: Ink::new(Role::Structure, DENDRITE_LEVEL[b.depth.min(3)]),
            });
        }
    }

    /// Draw the current state into the pixel and dot rasters.
    fn rasterise(&mut self) {
        self.pixels.clear();
        self.dots.clear();
        self.art.clear();
        let l = self.layout;
        if let Some([x, top, bottom, end]) = l.lead {
            // Every other dot: a faint dotted wire. The run along the baseline shares
            // its dot row, so the wire reads as the scope's input line.
            for y in (4 * top..4 * bottom + 4).filter(|y| y % 2 == 1) {
                self.art.set(2 * x, y, LEAD_INK);
            }
            if let Some(panel) = l.panel {
                let base = trace_y(panel, V_REST).round() as i32;
                for dx in (2 * x + 1..2 * end + 2).step_by(2) {
                    self.art.set(dx, base, LEAD_INK);
                }
            }
        }
        if let Some(panel) = l.panel {
            self.draw_trace(panel);
        }
        if l.full {
            self.draw_soma();
        }
        let v = &self.cable.v;
        if !l.full {
            for (x, &c) in (l.trunk.0..).zip(&self.trunk_c) {
                self.pixels
                    .set(x, l.tube, outline_or_potential(v_at(v, c), MEMBRANE));
            }
            return;
        }
        for &Seg { a, b, ink, .. } in &self.dendrites {
            self.art.line(a[0], a[1], b[0], b[1], ink);
        }
        for &ArborDot { x, y, c } in &self.arbor {
            self.art.set(x, y, outline_or_potential(v_at(v, c), ARBOR));
        }
        for p in self.vesicles.iter() {
            let fade = 1.0 - p.age / VESICLE_LIFE;
            let level = ink::level(255.0 * (0.25 + 0.7 * fade));
            self.dots.plot(p.x, p.y, Ink::new(VESICLE, level));
        }
        for ion in self.ions.iter() {
            // A 4-dot stroke along its motion, head first. Na⁺ brightens as it lands; K⁺
            // fades as it leaves.
            let f = ion.age / ION_LIFE;
            let ink = if ion.sodium {
                Ink::new(SODIUM, ink::level(255.0 * (0.7 + 0.3 * f)))
            } else {
                Ink::new(POTASSIUM, ink::level(255.0 * (1.0 - 0.4 * f)))
            };
            let back = -ion.vy.signum() * ION_TAIL;
            self.art.line(ion.x, ion.y, ion.x, ion.y + back, ink);
        }
        for ion in self.leak.iter() {
            let back = -ion.vy.signum() * ION_TAIL;
            self.art
                .line(ion.x, ion.y, ion.x, ion.y + back, leak_ink(ion));
        }
        for (x, &c) in (l.trunk.0..).zip(&self.trunk_c) {
            let vc = v_at(v, c);
            self.pixels.set(x, l.tube, MEMBRANE);
            for y in l.tube + 1..=l.tube + l.fill_px {
                // Across the tube, −1 at one membrane to +1 at the other: a cylinder.
                let u = 2.0 * (f64::from(y - l.tube) - 0.5) / f64::from(l.fill_px) - 1.0;
                self.pixels.set(x, y, fill_ink(vc, 1.0 - u * u));
            }
            self.pixels.set(x, l.tube + l.fill_px + 1, MEMBRANE);
        }
        let bouton = outline_or_potential(v[END], RIM);
        for [x0, y0, height] in l.boutons {
            for y in y0..y0 + height {
                for x in x0..x0 + l.bouton_w {
                    self.pixels.set(x, y, bouton);
                }
            }
        }
    }

    fn draw_soma(&mut self) {
        let [cx, cy, r] = self.layout.soma;
        let v0 = self.cable.v[0];
        // Lit from the upper left, like a sphere: the glow peaks 0.3 r up-left of centre.
        let (lx, ly) = (cx - 0.3 * r, cy - 0.3 * r);
        for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let d = (px - cx).hypot(py - cy);
                if d <= r {
                    let lit = 1.0 - ((px - lx).hypot(py - ly) / (1.3 * r)).powi(2);
                    let ink = if d > r - 1.0 { RIM } else { fill_ink(v0, lit) };
                    self.pixels.set(x, y, ink);
                }
            }
        }
    }

    /// The oscilloscope: V at the electrode over the last 20 ms, newest at the right,
    /// over a dotted resting baseline.
    fn draw_trace(&mut self, panel: [i32; 4]) {
        let [x0, _, x1, _] = panel;
        let base = trace_y(panel, V_REST).round() as i32;
        for x in (2 * x0..=2 * x1 + 1).step_by(3) {
            self.art.set(x, base, TICK_INK);
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
            self.art.line(x, top, x, bottom, TRACE_INK);
        }
    }

    /// The trace as ASCII line art, one glyph per cell (SPEC-002 ASCII): `|` where it
    /// crosses a cell top to bottom, `/` or a backslash where it rises or falls within
    /// one, `_` or `-` for a flat run in a cell's lower or upper half. The resting
    /// baseline shows as `.` ticks only where the trace has left it.
    fn draw_trace_ascii(&self, canvas: &mut Canvas, panel: [i32; 4]) {
        let [x0, _, x1, _] = panel;
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

    /// The dendrites and the terminal arbor as ASCII line art (SPEC-002 ASCII): one
    /// slope glyph per cell in the outline ink. Dendrites go shallowest first and never
    /// overwrite a drawn cell, so a parent's stroke wins every junction and a fork
    /// reads `\|/`, not a clump. An arbor cell above rest shows its voltage glyph
    /// instead, except at the junction with the trunk; below rest the stroke stays and
    /// only its blue carries the after-hyperpolarisation.
    fn draw_line_art(&self, canvas: &mut Canvas) {
        for &Seg { a, b, depth, ink } in &self.dendrites {
            let glyph = slope_glyph(a, b);
            // A child's first cell is its parent's last.
            for (x, y) in cell_line(a, b).skip(usize::from(depth > 0)) {
                if canvas.get(x, y).is_some_and(|c| c.ch == ' ') {
                    canvas.put(x, y, Glyph::ascii(glyph), ink);
                }
            }
        }
        let v = &self.cable.v;
        for (i, &ArborSeg { a, b, c0, c1 }) in self.arbor_segs.iter().enumerate() {
            let glyph = slope_glyph(a, b);
            let n = cell_line(a, b).count().max(2) - 1;
            for (k, (x, y)) in cell_line(a, b).enumerate() {
                // The first two cells of a primary sit on the trunk and at the fork.
                let junction = i < 3 && k < 2;
                let u = tone(v_at(v, ease::lerp(c0, c1, k as f64 / n as f64)));
                let ink = tone_ink(u);
                match spike_char(u) {
                    Some(ch) if !junction && u > 0.0 => {
                        canvas.put(x, y, Glyph::ascii(ch), ink);
                    }
                    Some(_) => canvas.put(x, y, Glyph::ascii(glyph), ink),
                    None => canvas.put(x, y, Glyph::ascii(glyph), ARBOR),
                }
            }
        }
    }

    fn ascii_cell(&self, x: i32, y: i32, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let l = &self.layout;
        // The pixel furthest from rest decides.
        let potential = [top, bottom]
            .into_iter()
            .filter_map(|i| ink_tone(i).map(|x| (x, i)))
            .max_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        let spike = potential.and_then(|(x, i)| spike_char(x).map(|ch| (ch, i)));
        let on_axon = l.on_axon(x, y);
        if l.near_soma(x, y) && !on_axon {
            // The soma at cell resolution: a rim derived from pixel pairs comes out
            // with gaps and bumps, so the rim is its own ellipse of cells, drawn
            // whatever the voltage, and only the cells inside it show the fill.
            return match l.soma_cell(x, y) {
                Some(true) => Some(('o', RIM)),
                Some(false) => spike,
                None => None,
            };
        }
        match spike {
            // A hyperpolarised bouton keeps its `o`; the blue colour carries the
            // after-hyperpolarisation, as on the arbor strokes.
            Some(('~', ink)) if !on_axon => return Some(('o', ink)),
            Some(_) => return spike,
            None => {}
        }
        let outline = [top, bottom]
            .into_iter()
            .find(|i| i.role == Role::Structure);
        if on_axon {
            Some(('=', outline.unwrap_or(MEMBRANE)))
        } else {
            outline.map(|o| ('o', o))
        }
    }

    /// The readout, right-aligned on the bottom row clear of the title (GEN-R13).
    fn draw_label(&self, canvas: &mut Canvas) {
        let h = self.cells.height();
        if h < 3 {
            return;
        }
        let mut speed = StackText::<8>::new();
        // Writing into a StackText never fails.
        let _ = match self.meter.speed {
            Some(s) => write!(speed, "{s:.1}"),
            None => write!(speed, "--.-"),
        };
        let mut label = StackText::<80>::new();
        // Each threshold is its form's length with a 4-char speed. The beat is the
        // title's caption, not repeated here. The temperature stays in every form:
        // 12.3 m/s is right for 6.3 °C, not the textbook 21 m/s at 18.5 °C.
        // Sized for every caption, so the form never flickers with the beat.
        let _ = match self.readout_room {
            48.. => write!(label, "Hodgkin-Huxley 1952  squid axon 6.3C  v={speed} m/s"),
            25.. => write!(label, "HH squid 6.3C  v={speed} m/s"),
            19.. => write!(label, "HH 6.3C  v={speed} m/s"),
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

    fn caption(&self) -> Option<&'static str> {
        let beat = self.timeline.at(self.step as f64 / SUB_PER_S as f64).index;
        beat.checked_sub(1).map(|k| CAPTIONS[k])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        // The cable does not depend on the size: only the drawing is rebuilt.
        self.cells = size;
        if self.tree.is_none() {
            self.tree = Some(Tree::grow(&mut ctx.rng));
        }
        self.layout = Layout::new(size);
        self.pixels.resize(size);
        self.dots.resize(size);
        self.art.resize(size);
        self.vesicles.clear();
        self.ions.clear();
        self.readout_room = title::stable_readout_room(size, &META);
        self.rebuild_geometry();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        // 1e-9 of slack so that exact totals (400 sub-steps per second) never lose one.
        self.owed += dt * SUB_PER_S as f64;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
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
            p.y += p.vy * dt;
            p.age < ION_LIFE
        });
        self.leak.retain(|p| {
            p.age += dt;
            p.y += p.vy * dt;
            p.age < ION_LIFE / LEAK_SLOW
        });
        self.spawn_ions(n as u32);
        self.spawn_leak(dt);
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let l = self.layout;
        // ASCII has no braille: the dot-count fallback would reuse the voltage glyphs
        // for line art, so dendrites and arbor are drawn with slope glyphs instead. The
        // pixels go last so the axon, soma and boutons sit on top of both.
        if canvas.mode() == GlyphMode::Ascii {
            if let Some([x, top, bottom, end]) = l.lead {
                for y in top..=bottom {
                    canvas.put(x, y, Glyph::ascii(':'), LEAD_INK);
                }
                for x in (x + 2..=end).step_by(2) {
                    canvas.put(x, top, Glyph::ascii('.'), LEAD_INK);
                }
            }
            self.dots.blit(canvas);
            if l.full {
                self.draw_line_art(canvas);
            }
            if let Some(panel) = l.panel {
                self.draw_trace_ascii(canvas, panel);
            }
        } else {
            self.art.blit(canvas);
            self.dots.blit(canvas);
        }
        self.pixels.blit(canvas, |x, y, top, bottom| {
            self.ascii_cell(x, y, top, bottom)
        });
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
        ap.prepare();
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
                ap.prepare();
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
        ap.prepare();
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
    fn ap_ascii_voltage_glyphs_only_mark_membrane_off_rest() {
        for (w, h) in [(80u16, 24u16), (40, 12), (56, 16), (120, 30), (200, 50)] {
            let (_, lines) = ascii_frame(Size::new(w, h), 0.0);
            let text = lines.join("\n").replace("+40", "");
            for ch in ['@', '#', '+', '~'] {
                assert!(
                    !text.contains(ch),
                    "{w}x{h} at rest shows {ch:?}:\n{}",
                    lines.join("\n")
                );
            }
            let art = text.chars().filter(|c| "/\\|-".contains(*c)).count();
            assert!(art >= 6, "{w}x{h}: dendrites and arbor as line art");
        }
    }

    #[test]
    fn ap_ascii_arbor_keeps_its_strokes_below_rest() {
        // 6 s: the first train spike has passed the terminals and left them
        // hyperpolarised; 14.2 s: the collision's wake.
        for t in [6.0, 14.2] {
            let (ap, lines) = ascii_frame(Size::new(80, 24), t);
            let x0 = ap.layout.trunk.1 + 1;
            let arbor: String = lines
                .iter()
                .map(|row| row.chars().skip(x0 as usize).collect::<String>())
                .collect();
            assert!(!arbor.contains('~'), "t={t}:\n{}", lines.join("\n"));
            let strokes = arbor.chars().filter(|c| "/\\-|".contains(*c)).count();
            assert!(
                strokes >= 12,
                "t={t}: {strokes} strokes:\n{}",
                lines.join("\n")
            );
            assert!(arbor.matches('o').count() >= 5, "t={t}: boutons");
        }
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
        // Taller tiers: a 3-row tube, then a 4-row one.
        assert_eq!(Layout::new(Size::new(120, 40)).fill_px, 4);
        let l = Layout::new(Size::new(200, 50));
        assert_eq!(l.fill_px, 6);
        assert_eq!(Layout::new(Size::new(80, 24)).fill_px, 2);
    }

    #[test]
    fn ap_small_tier_keeps_its_primary_dendrites() {
        let size = Size::new(40, 12);
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        ap.prepare();
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
        ap.prepare();
        ap.step = (t * SUB_PER_S as f64) as u64;
        ap.meter.speed = speed;
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        ap.draw_label(&mut canvas);
        canvas.to_lines().swap_remove(11)
    }

    #[test]
    fn ap_gen_r13_label_forms_follow_the_room_right_of_the_title() {
        let long = "Hodgkin-Huxley 1952  squid axon 6.3C  v=--.- m/s";
        let mid = "HH squid 6.3C  v=--.- m/s";
        let short = "HH 6.3C  v=--.- m/s";
        // The room is `title::stable_readout_room` (CORE-R83): the least over the title
        // with each caption, at this width and every wider one, so the form never
        // flickers with the beat and only grows with the width.
        for (w, want) in [
            (124u16, long),
            (96, long),
            (95, mid),
            (63, mid),
            (62, short),
            (56, short),
            (55, "HH 6.3C"),
            (24, "HH 6.3C"),
            (23, ""),
            (10, ""),
        ] {
            // The form is the same in every beat.
            for t in [0.0, 5.0, 14.0, 16.0] {
                let row = readout(w, t, None);
                // Whole, never clipped, and ending one blank cell from the right edge.
                assert_eq!(row.trim(), want, "w={w} t={t}");
                assert!(row.ends_with(&format!("{want} ")), "w={w}: {row:?}");
            }
        }
        assert_eq!(
            readout(64, 14.0, Some(12.3)).trim(),
            "HH squid 6.3C  v=12.3 m/s"
        );
    }

    #[test]
    fn ap_a10_captions_follow_the_protocol() {
        let (mut ap, mut ctx) = scene();
        let mut seen = Vec::new();
        for _ in 0..(18 * 4) {
            ap.update(0.25, &mut ctx);
            ap.prepare();
            let beat = ap.timeline.at(ap.step as f64 / SUB_PER_S as f64).label;
            let want = match beat {
                "single" => None,
                "train" => Some("refractory slowing"),
                "collision" => Some("spikes annihilate"),
                _ => Some("all-or-none"),
            };
            assert_eq!(ap.caption(), want, "{beat}");
            if let Some(c) = want {
                assert!(META.captions.contains(&c));
                if !seen.contains(&c) {
                    seen.push(c);
                }
            }
        }
        assert_eq!(seen.len(), 3);
    }

    /// Run the seeded 200×50 scene to `t` in frames of 1/60 s.
    fn run_to(t: f64) -> ActionPotential {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        ap.prepare();
        while (ap.step as f64) < t * SUB_PER_S as f64 - 1e-6 {
            ap.update(SIM_DT, &mut ctx);
            ap.prepare();
        }
        ap
    }

    #[test]
    fn ap_a11_voltage_is_one_violet_ramp_brightening_with_v() {
        // One named ramp, off amber: brighter with V, a resting glow at rest, dimmer in
        // the after-hyperpolarisation, and the tone recoverable from the ink (ASCII).
        assert_eq!(SPIKE, Ramp::Plasma);
        let mut last = 0;
        for k in 0..=74 {
            let x = -0.48 + f64::from(k) * 0.02;
            let i = tone_ink(x);
            assert_eq!(i.role, Role::Ramp(SPIKE), "{x}");
            assert!(i.level >= last, "{x}: {} < {last}", i.level);
            last = i.level;
            let back = ink_tone(i).expect("a potential ink");
            let want = if (-WAKE_FLOOR..0.0).contains(&x) {
                0.0
            } else {
                x
            };
            assert!((back - want).abs() < 0.02, "{x} -> {back}");
        }
        assert_eq!(tone_ink(0.0), ink::ramp(SPIKE, U_REST));
        assert_eq!(tone_ink(1.0).level, 255);
        assert!(f64::from(tone_ink(0.0).level) >= ink::MIN_VISIBLE_LEVEL);
        // Mid-axon spike at 1.2 s (the pulse left the soma at 0.2 s): above rest ahead of
        // the wake, below rest in it, the resting glow elsewhere; never an empty pixel.
        let ap = run_to(1.2);
        let l = ap.layout;
        let (mut above, mut below, mut resting) = (0, 0, 0);
        for x in l.trunk.0..=l.trunk.1 {
            for y in l.tube + 1..=l.tube + l.fill_px {
                let px = ap.pixels.get(x, y).expect("an axon pixel");
                let t = ink_tone(px).unwrap_or_else(|| panic!("axoplasm in {px:?}"));
                match t {
                    t if t > X_PLUS => above += 1,
                    t if t < -0.1 => below += 1,
                    _ => resting += 1,
                }
            }
        }
        assert!(
            above > 20 && below > 4 && resting > 20,
            "{above} {below} {resting}"
        );
        // The oscilloscope is the spike's own violet phosphor.
        assert_eq!(TRACE_INK.role, Role::Ramp(SPIKE));
        const { assert!(TRACE_INK.level >= 200) };
    }

    #[test]
    fn ap_a11_wake_is_quiet_beside_the_spike() {
        // The ≈ 10 mV undershoot must not weigh like the ≈ 105 mV spike: over the filled
        // trunk and soma at 5 s (120×40), the wake's total departure from the resting
        // glow is under 0.35 of the spike's, and no wake pixel departs by more than the
        // glow itself.
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut ap = ActionPotential::default();
        ap.resize(size, &mut ctx);
        ap.prepare();
        while (ap.step as f64) < 5.0 * SUB_PER_S as f64 - 1e-6 {
            ap.update(SIM_DT, &mut ctx);
            ap.prepare();
        }
        let rest = ink::seq_value(tone_ink(0.0));
        let (mut spike, mut wake, mut peak) = (0.0, 0.0, 0.0_f64);
        for y in 0..2 * i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let px = ap.pixels.get(x, y).expect("a pixel");
                if px.role != Role::Ramp(SPIKE) {
                    continue;
                }
                // Shading only brightens the glow, so below it is the wake; above it,
                // only a real response (past ASCII's `+`) counts as spike.
                let d = ink::seq_value(px) - rest;
                if ink_tone(px).is_some_and(|x| x > X_PLUS) {
                    spike += d;
                } else if d < 0.0 {
                    wake -= d;
                    peak = peak.max(-d);
                }
            }
        }
        assert!(spike > 0.0, "no spike at 5 s");
        assert!(wake < 0.35 * spike, "wake {wake} vs spike {spike}");
        assert!(peak <= rest + 1e-9, "wake peak {peak}");
    }

    #[test]
    fn ap_a13_soma_and_axon_glow_at_rest_and_leak_ions_trickle() {
        // Between spikes (11.5 s: the 8 s spike has left, the 13 s pair not begun) the
        // soma is a filled, shaded disc and the axoplasm a lit tube, not outlines.
        let ap = run_to(11.5);
        let l = ap.layout;
        let [cx, cy, r] = l.soma;
        let (mut lo, mut hi, mut inside) = (u8::MAX, 0, 0);
        for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                let d = (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy);
                if d > r - 1.5 {
                    continue;
                }
                let px = ap.pixels.get(x, y).expect("a soma pixel");
                assert_eq!(px.role, Role::Ramp(SPIKE), "({x}, {y})");
                assert!(f64::from(px.level) >= ink::MIN_VISIBLE_LEVEL, "({x}, {y})");
                (lo, hi) = (lo.min(px.level), hi.max(px.level));
                inside += 1;
            }
        }
        assert!(
            inside > 50 && hi > lo + 4,
            "{inside} pixels, levels {lo}..{hi}"
        );
        for x in l.trunk.0..=l.trunk.1 {
            for y in l.tube + 1..=l.tube + l.fill_px {
                let px = ap.pixels.get(x, y).expect("an axon pixel");
                assert_eq!(px.role, Role::Ramp(SPIKE), "({x}, {y})");
            }
        }
        // The resting leak: a sparse, dim trickle of both species, inward Na⁺ and
        // outward K⁺, drawn in the ions' own hues but well under a spike's streaks.
        let (mut na, mut k) = (0, 0);
        for t in [0.15, 11.5, 12.0] {
            let ap = run_to(t);
            assert!(!ap.leak.is_empty(), "no leak at {t}");
            assert!(ap.leak.len() <= LEAK_CAP);
            for ion in ap.leak.iter() {
                let i = leak_ink(ion);
                assert!(i.level <= 160, "{i:?}");
                if ion.sodium {
                    na += 1;
                    assert_eq!(i.role, SODIUM);
                } else {
                    k += 1;
                    assert_eq!(i.role, POTASSIUM);
                }
            }
        }
        assert!(na > 0 && k > 0, "Na {na} K {k}");
        // Sodium streaks are sodium-flame yellow, not the gallery's amber.
        assert_eq!(SODIUM, Role::Ramp(Ramp::Sulfur));
    }

    #[test]
    fn ap_a12_ions_cross_where_channels_open() {
        // At rest no ion is drawn: the leak is not shown.
        let rest = run_to(0.15);
        assert!(rest.ions.is_empty(), "{} ions at rest", rest.ions.len());
        // While the first spike runs down the axon, Na⁺ falls onto the membrane and K⁺
        // leaves it, only outside the tube and over the trunk.
        let (mut sodium, mut potassium) = (0, 0);
        let mut columns = [0; 2];
        for t in [0.8, 1.2, 1.6] {
            let ap = run_to(t);
            let l = ap.layout;
            // A few streaks per spike, not a dust cloud: under twice `IONS_PER_SPIKE` of
            // each species at once, each in its own hue, both dot columns used.
            for na in [true, false] {
                let n = ap.ions.iter().filter(|i| i.sodium == na).count();
                assert!(n as f64 <= 2.0 * IONS_PER_SPIKE, "t = {t}: {n} (Na⁺ {na})");
            }
            let mut canvas = Canvas::new(ap.cells, GlyphMode::Unicode);
            ap.art.blit(&mut canvas);
            for ion in ap.ions.iter() {
                columns[(ion.x as usize) % 2] += 1;
                // Its head's cell is drawn in an ion hue (the two species may share one).
                let (cx, cy) = ((ion.x / 2.0) as i32, (ion.y.round() / 4.0).floor() as i32);
                let role = canvas.get(cx, cy).map(|c| c.ink.role);
                assert!(
                    role == Some(SODIUM) || role == Some(POTASSIUM),
                    "{ion:?}: {role:?}"
                );
            }
            let (top, bottom) = (
                f64::from(2 * l.tube),
                f64::from(2 * (l.tube + l.fill_px + 2)),
            );
            let centre = 0.5 * (top + bottom);
            for ion in ap.ions.iter() {
                assert!(ion.y < top || ion.y >= bottom, "inside the tube: {ion:?}");
                let x = ion.x / 2.0;
                assert!(
                    (f64::from(l.trunk.0)..f64::from(l.trunk.1 + 1)).contains(&x),
                    "off the trunk: {ion:?}"
                );
                // Toward the axis for Na⁺, away from it for K⁺.
                let inward = (centre - ion.y).signum() == ion.vy.signum();
                assert_eq!(inward, ion.sodium, "{ion:?}");
                if ion.sodium {
                    sodium += 1;
                } else {
                    potassium += 1;
                }
            }
            assert!(ap.ions.len() < ION_CAP, "the pool saturated at t = {t}");
            // Two clusters: the first spike runs toward the terminal (+x), Na⁺ on its
            // rising front ahead of K⁺ on the repolarising edge.
            let mean_x = |na: bool| {
                let xs: Vec<f64> = ap
                    .ions
                    .iter()
                    .filter(|i| i.sodium == na)
                    .map(|i| i.x)
                    .collect();
                xs.iter().sum::<f64>() / xs.len().max(1) as f64
            };
            assert!(mean_x(true) > mean_x(false), "t = {t}: Na⁺ not ahead of K⁺");
        }
        assert!(sodium >= 10 && potassium >= 10, "Na {sodium} K {potassium}");
        assert!(columns[0] > 0 && columns[1] > 0, "dot columns {columns:?}");
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

    /// The soma's cell box at `size`, one cell of margin each side.
    fn soma_box(ap: &ActionPotential) -> [i32; 4] {
        let [cx, cy, r] = ap.layout.soma;
        [
            (cx - r).floor() as i32 - 1,
            ((cy - r) / 2.0).floor() as i32 - 1,
            (cx + r).ceil() as i32 + 1,
            ((cy + r) / 2.0).ceil() as i32 + 1,
        ]
    }

    #[test]
    fn ap_ascii_soma_rim_is_a_clean_closed_outline() {
        for (w, h) in [(40u16, 12u16), (56, 16), (80, 24), (120, 30), (200, 50)] {
            let (ap, rest) = ascii_frame(Size::new(w, h), 0.0);
            let bx = soma_box(&ap);
            let cells = region(&rest, bx);
            let frame = rest.join("\n");
            let outline = |c: char| c == 'o' || c == '=';
            // Convex: going down the rows, the outline's left edge moves out and then
            // back in, and so does its right edge (no bites, no bumps).
            let edges: Vec<(usize, usize)> = cells
                .iter()
                .filter_map(|row| {
                    let first = row.iter().position(|&c| outline(c))?;
                    let last = row.iter().rposition(|&c| outline(c))?;
                    Some((first, last))
                })
                .collect();
            assert!(edges.len() >= 3, "{w}x{h}: soma rows {edges:?}\n{frame}");
            let unimodal = |xs: Vec<i64>| {
                let turn = xs.iter().enumerate().max_by_key(|(_, x)| **x).unwrap().0;
                xs[..=turn].windows(2).all(|p| p[0] <= p[1])
                    && xs[turn..].windows(2).all(|p| p[0] >= p[1])
            };
            assert!(
                unimodal(edges.iter().map(|e| -(e.0 as i64)).collect())
                    && unimodal(edges.iter().map(|e| e.1 as i64).collect()),
                "{w}x{h}: rim edges {edges:?}\n{frame}"
            );
            // Closed: a row's outline cells come in at most two runs (the two sides), and
            // the top and bottom rows are one run each.
            let runs = |row: &Vec<char>| {
                let mut n = 0;
                let mut prev = false;
                for &c in row {
                    let on = outline(c);
                    n += usize::from(on && !prev);
                    prev = on;
                }
                n
            };
            let rows: Vec<&Vec<char>> = cells.iter().filter(|r| runs(r) > 0).collect();
            assert!(rows.iter().all(|r| runs(r) <= 2), "{w}x{h}:\n{frame}");
            assert_eq!(runs(rows[0]), 1, "{w}x{h} top:\n{frame}");
            assert_eq!(runs(rows[rows.len() - 1]), 1, "{w}x{h} bottom:\n{frame}");
            // A firing or hyperpolarised soma fills its interior; the rim stays drawn.
            let (_, busy) = ascii_frame(Size::new(w, h), 2.0);
            let rims = |lines: &[String]| -> Vec<(usize, usize)> {
                region(lines, bx)
                    .iter()
                    .enumerate()
                    .flat_map(|(y, row)| {
                        row.iter()
                            .enumerate()
                            .filter(|(_, c)| **c == 'o')
                            .map(move |(x, _)| (x, y))
                    })
                    .collect()
            };
            assert_eq!(rims(&rest), rims(&busy), "{w}x{h}:\n{}", busy.join("\n"));
        }
    }
}
