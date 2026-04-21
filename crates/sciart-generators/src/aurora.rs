//! Auroral curtains folding over a still northern lake at night (SPEC-002 `aurora`).
//!
//! Up to three thin east–west sheets of precipitating electrons stand at a northward
//! ground distance `D(s)` that folds along the arc coordinate `s`. Each sheet is sampled
//! at fixed stations, projected into sky angles (azimuth α, elevation e, with the
//! Earth's curvature drop) and splatted into per-column layers; every sky pixel then sums
//! the layers' altitude profiles: prompt green 557.7 nm just above the lower border, a
//! lagged diffuse red 630 nm fringe near 240 km, and at the substorm peak a pink N₂ hem
//! along the border. A 50 s `Timeline` compresses a substorm (growth → onset →
//! expansion → recovery), while folds and rays drift at real km/s speeds.
//!
//! All state lives in world or sky coordinates, so a resize only re-frames the same sky.
//! The sky is rasterised in `update`/`resize`; `render` blits and writes labels.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{draw, geom, ink};

pub static META: Meta = Meta {
    name: "aurora",
    category: Category::Astronomy,
    description: "Auroral curtains folding over a still northern lake at night",
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Aurora::new())
}

/// Stations per curtain: `s_j = 300 km·tan α_j`, α_j uniform in ±85°.
const M: usize = 480;
const CURTAINS: usize = 3;
const S_SCALE: f64 = 300.0;
const ALPHA_MAX: f64 = 85.0 * PI / 180.0;
const R_EARTH: f64 = 6371.0;
/// Fold modes (bend, surge, fold, curl): wavelengths in km; the bend has a fixed
/// amplitude, the others a slope amplitude σ scaled by F(t).
const FOLD_LAMBDA: [f64; 4] = [900.0, 240.0, 70.0, 18.0];
/// Fold wavenumbers `κ_k = 2π/λ_k` (rad/km).
const FOLD_KAPPA: [f64; 4] = [
    TAU / FOLD_LAMBDA[0],
    TAU / FOLD_LAMBDA[1],
    TAU / FOLD_LAMBDA[2],
    TAU / FOLD_LAMBDA[3],
];
const BEND_KM: f64 = 40.0;
const FOLD_SLOPE: [f64; 3] = [0.5, 1.3, 0.8];
/// Ray field wavelengths (km) and velocities relative to v_r(t).
const RAY_LAMBDA: [f64; 3] = [23.0, 37.0, 61.0];
const RAY_V: [f64; 3] = [1.0, -0.6, 0.8];
/// Ray wavenumbers `κ_m = 2π/λ_m` (rad/km).
const RAY_KAPPA: [f64; 3] = [
    TAU / RAY_LAMBDA[0],
    TAU / RAY_LAMBDA[1],
    TAU / RAY_LAMBDA[2],
];
/// Mean of `max(0, cos x)⁶`: rays redistribute flux without changing its mean.
const RHO_MEAN: f64 = 5.0 / 32.0;
/// Green scale height (km) for ρ = 0.
const H_GREEN: f64 = 22.0;
/// Red source relaxation time (s), the O(¹D) lag compressed only enough to stay visible.
const TAU_RED: f64 = 3.0;
const SLANT_MAX: f64 = 6.0;
/// O(¹D) atoms diffuse tens of km during their 110 s lifetime, so the red layer is thick
/// and its path-length gain saturates early: the red stays smooth and rayless.
const RED_SLANT_MAX: f64 = 2.0;

/// The substorm: growth 12 s → onset 3 s → expansion 15 s → recovery 20 s (AUR-A6);
/// expansion and recovery after Akasofu (1964), the growth phase after McPherron (1970).
const BEATS: [(&str, f64); 4] = [
    ("growth", 12.0),
    ("onset", 3.0),
    ("expansion", 15.0),
    ("recovery", 20.0),
];
/// Beat boundaries, derived from `BEATS`; every keyframe knot is written relative to
/// them, so the keyframes and the label never drift apart and the onset segments keep
/// their `out_quad` easing (`curve` matches `ONSET` exactly).
const ONSET: f64 = BEATS[0].1;
const EXPANSION: f64 = ONSET + BEATS[1].1;
const RECOVERY: f64 = EXPANSION + BEATS[2].1;
const LOOP: f64 = RECOVERY + BEATS[3].1;
/// The breakup: the equatorward arc brightens within 1.5 s of the onset.
const BREAKUP: f64 = ONSET + 1.5;
/// The substorm peak (AL most negative), 7 s into the expansion.
const PEAK: f64 = EXPANSION + 7.0;
/// Scene clock at build: breakup lands 4 s into a default 15 s visit.
const START_T: f64 = 8.0;

/// Emission for tone 1 − 1/e: a growth arc reads faint, an expansion curtain saturates
/// only in its rays.
const E_SAT: f64 = 0.9;
/// A pixel is lit when its tone exceeds `LIT_THRESHOLD × b`, with `b` its fixed dither
/// threshold in [0, 1) (a strand for curtain green, square blue noise for diffuse
/// light; see `Dither`): the theme floors lit roles at 30 % brightness, so tones below
/// 0.3 are shown by pixel density rather than by level.
const LIT_THRESHOLD: f32 = 0.3;
/// Tones below this stay dark: the faintest haze is dropped, not scattered.
const TONE_FLOOR: f32 = 0.04;
/// Red-dominant pixels need this tone (lit ≥ 30 % of the time): below it the red
/// fringe's sparse tail would scatter as isolated specks over a dome larger than the
/// green arc, so it is trimmed and the fringe keeps an envelope over the curtain tops.
const RED_TONE_FLOOR: f32 = 0.09;
/// Emission fades in under the top edge over this share of the pixel rows (at least
/// `TOP_FADE_MIN_PX`), as a smoothstep, applied to the tone before the density
/// threshold: a crown overhead thins into dark sky over ≈ 10 cell rows (keeping the
/// label in clear air), so its top edge follows the curtains' own red envelope
/// rather than meeting a lid.
const TOP_FADE: f64 = 0.2;
const TOP_FADE_MIN_PX: f64 = 3.0;
/// O(¹D) atoms diffuse tens of km during their 110 s lifetime, so the red emission is
/// smoothed along the arc by a Gaussian of this width (≈ 20 km at 300 km) before it is
/// dithered: where folds double back and the layer count steps, the red ramps instead
/// of standing as a wall.
const RED_DIFFUSE_DEG: f64 = 4.0;
/// Horizon skyglow: every sky pixel below this elevation glows, brightest at the
/// horizon and falling quadratically to black, so the ridge and spruce spires (at most
/// 4° + 4°) stand against it as a crisp black edge.
const GLOW_DEG: f64 = 8.0;
/// Skyglow `Seq` value at the horizon: the low, near-black end of the ramp, since lit
/// roles are floored at 30 % brightness and could not fade out.
const GLOW_MAX: f64 = 0.14;
/// The top share of the glow that thins out by pixel density: below it the band is
/// continuous, so the silhouette cuts a clean black edge into it.
const GLOW_FEATHER: f64 = 0.2;
/// The red share R/(G+R) over which lit pixels turn from green to red.
const RED_BAND: (f32, f32) = (0.3, 0.7);
const LAKE_GAIN: f32 = 0.45;
/// Display weight of red 630 nm against green 557.7 nm: the photopic luminous efficiency
/// ratio V(630)/V(557.7) ≈ 0.27. The emission profiles stay physical; this is how bright
/// the red *looks*, so the diffuse fringe stays a fringe.
const RED_WEIGHT: f32 = 0.27;
/// Minimum pink-hem tone for an `Accent` pixel: type-B hems mark only the brightest arc.
const HEM_TONE: f32 = 0.6;
/// Layers per column: 3 curtains, each doubled back once by a fold in the oblique sky.
/// A further layer merges its amplitudes into the one at the nearest distance.
const MAX_LAYERS: usize = 6;

const STARS: usize = 1200;
const STAR_FRACTION: f64 = 0.02;
const PATCHES: usize = 6;
/// Peak of `green_profile(·, 0, H_GREEN)`, reached at u ≈ 3 km.
const GREEN_PEAK: f64 = 0.80;
/// Pulsating-patch green: 0.35× the peak green of a unit-flux arc.
const PATCH_GAIN: f64 = 0.35 * GREEN_PEAK;

const LABEL_INK: Ink = Ink::new(Role::Text, 150);
const RIDGE_INK: Ink = Ink::new(Role::Dim, 70);
const STAR_DIM: Glyph = Glyph::new('·', '.');
const STAR_BRIGHT: Glyph = Glyph::new('✦', '*');
/// ASCII green: ':' from this level up, ',' below (stars keep '.' and '*').
const ASCII_COLON: u8 = 70;

// --- Physics (pure) -----------------------------------------------------------------

/// Lower-border altitude (km) for characteristic (monoenergetic-equivalent) energy
/// `e0_kev`: a fit to Rees (1963) peak energy-deposition heights of monoenergetic
/// beams, clamped to [85, 150] km (AUR-A1). Non-positive or NaN energies read as the
/// softest precipitation.
pub(crate) fn lower_border_km(e0_kev: f64) -> f64 {
    if e0_kev.is_nan() || e0_kev <= 0.0 {
        return 150.0;
    }
    (131.0 - 25.0 * e0_kev.log10()).clamp(85.0, 150.0)
}

/// Green 557.7 nm volume emission per unit flux at altitude `z` over a border at `zb`,
/// scale height `h` (AUR-A2): zero below `zb − 2 km`, peaking ≈ 3 km above `zb`.
pub(crate) fn green_profile(z: f64, zb: f64, h: f64) -> f64 {
    let u = z - zb;
    if u < -2.0 {
        0.0
    } else {
        (1.0 - (-(u + 2.0) / 2.0).exp()) * (-u / h).exp()
    }
}

/// Red 630.0 nm emission per unit source at altitude `z` (AUR-A2): a Chapman production
/// layer peaking at 240 km times collisional quenching of O(¹D) below ~200 km. The 0.4
/// normalisation gives a red column (49.5 km) ≈ 2.2× the green one (22.1 km) for equal
/// sources, 10–45× the photometric I630/I557.7 ≈ 0.05–0.2 of keV aurora; `RED_WEIGHT`
/// (photopic 0.27) brings the displayed ratio to ≈ 0.6, a vivid, photograph-like fringe. The red
/// source ignores E0 (real red weakens as the precipitation hardens).
pub(crate) fn red_profile(z: f64) -> f64 {
    let y = (z - 240.0) / 50.0;
    0.4 * (1.0 - y - (-y).exp()).exp() / (1.0 + (-(z - 190.0) / 12.0).exp())
}

/// Fraction of the green flux that lights the pink N₂ hem for characteristic energy
/// `e0`: fast electrons (type-B aurora) only.
fn pink_fraction(e0_kev: f64) -> f64 {
    ((e0_kev - 8.0) / 12.0).clamp(0.0, 1.0)
}

/// The pink N₂ 1PG band, `u = z − z_b` in km: fast electrons reach heights at which
/// O(¹S) is quenched, just below the green border (AUR-A2).
const HEM_BELOW: f64 = 4.0;
const HEM_ABOVE: f64 = 1.0;

#[inline]
fn in_hem_band(u: f32) -> bool {
    (-HEM_BELOW as f32..=HEM_ABOVE as f32).contains(&u)
}

/// The lagged red source relaxed exactly toward `target` over `dt` (AUR-A3); an
/// infinite `dt` lands on the target (the equilibrium start). `Curtain::sample` uses
/// the same formula with the decay factor hoisted out of its station loop.
#[cfg(test)]
pub(crate) fn red_relax(r: f64, target: f64, dt: f64) -> f64 {
    relax_by(r, target, red_decay(dt))
}

/// The red source's decay factor `e^{−dt/τ_R}` over one step.
fn red_decay(dt: f64) -> f64 {
    (-dt / TAU_RED).exp()
}

#[inline]
fn relax_by(r: f64, target: f64, decay: f64) -> f64 {
    target + (r - target) * decay
}

/// One ray family: `max(0, cos(κ s + φ))⁶`, narrow bright rays with mean 5/32.
#[inline]
fn ray_term(s: f64, kappa: f64, phase: f64) -> f64 {
    let c = (kappa * s + phase).cos().max(0.0);
    let c2 = c * c;
    c2 * c2 * c2
}

/// Height (km) below a flat tangent plane of a point `d` km away: the curvature drop.
#[inline]
fn drop_km(d: f64) -> f64 {
    d * d / (2.0 * R_EARTH)
}

/// Elevation (rad) of altitude `z` seen at ground distance `d`: flat earth plus the
/// curvature drop, so a straight east–west arc sags into both horizons (AUR-A4). Its
/// inverse, `z = d·tan e + d²/2R⊕`, is what each pixel row evaluates.
pub(crate) fn elevation(z: f64, d: f64) -> f64 {
    ((z - drop_km(d)) / d).atan()
}

/// Path-length gain through a thin sheet: `min(1/|n̂·l̂|, 6)` for sheet normal
/// `n ∝ (−ξ', 1, 0)` and line of sight `l = (s, D, h)` (east, north, up) (AUR-A5).
/// Folds seen edge-on become bright caustic rays.
pub(crate) fn slant_factor(dxi: f64, s: f64, dd: f64, h: f64) -> f64 {
    let dot = (dd - dxi * s).abs();
    let norm = (dxi * dxi + 1.0).sqrt() * (s * s + dd * dd + h * h).sqrt();
    if dot * SLANT_MAX <= norm {
        SLANT_MAX
    } else {
        norm / dot
    }
}

// --- Substorm keyframes -------------------------------------------------------------

/// Keyframed substorm state at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Params {
    /// Northward distance of each curtain (km).
    pub d0: [f64; CURTAINS],
    /// Precipitation amplitude A_c.
    pub amp: [f64; CURTAINS],
    /// Characteristic electron energy (keV).
    pub e0: f64,
    /// Fold amplitude factor F.
    pub fold: f64,
    /// Ray fraction r.
    pub rays: f64,
    /// Ray speed v_r (km/s).
    pub v_ray: f64,
    /// Westward surge speed v_s (km/s).
    pub v_surge: f64,
    /// Bulge amplitude b, centre α_b and width w_b (rad).
    pub bulge: f64,
    pub bulge_alpha: f64,
    pub bulge_width: f64,
    /// Pulsating-patch envelope (0 outside recovery).
    pub patches: f64,
    /// Activity a ∈ [0.02, 1], which sets AL.
    pub activity: f64,
}

/// Knots `(loop time, value)` of one keyframed parameter; first and last values match.
type Knots = &'static [(f64, f64)];

const K_D0: Knots = &[
    (0.0, 520.0),
    (ONSET, 260.0),
    (EXPANSION, 260.0),
    (RECOVERY, 170.0),
    (LOOP, 520.0),
];
/// Arcs 2 and 3 sit poleward of arc 1 by these offsets (km).
const K_OFF2: Knots = &[
    (0.0, 150.0),
    (EXPANSION, 150.0),
    (EXPANSION + 9.0, 110.0),
    (RECOVERY, 110.0),
    (LOOP, 150.0),
];
const K_OFF3: Knots = &[(0.0, 330.0), (LOOP, 330.0)];
const K_A1: Knots = &[
    (0.0, 0.25),
    (ONSET, 0.4),
    (BREAKUP, 1.0),
    (PEAK, 1.0),
    (RECOVERY, 0.75),
    (LOOP, 0.25),
];
const K_A2: Knots = &[
    (0.0, 0.15),
    (ONSET, 0.25),
    (EXPANSION, 0.3),
    (EXPANSION + 6.0, 0.55),
    (RECOVERY, 0.4),
    (LOOP, 0.15),
];
const K_A3: Knots = &[
    (0.0, 0.0),
    (EXPANSION, 0.0),
    (EXPANSION + 6.0, 0.4),
    (RECOVERY, 0.3),
    (RECOVERY + 14.0, 0.0),
    (LOOP, 0.0),
];
const K_E0: Knots = &[
    (0.0, 2.0),
    (ONSET, 4.0),
    (BREAKUP, 20.0),
    (EXPANSION, 20.0),
    (PEAK, 12.0),
    (RECOVERY, 10.0),
    (LOOP, 2.0),
];
const K_FOLD: Knots = &[
    (0.0, 0.15),
    (ONSET, 0.15),
    (EXPANSION, 0.4),
    (EXPANSION + 9.0, 1.0),
    (RECOVERY, 1.0),
    (LOOP, 0.15),
];
const K_RAYS: Knots = &[
    (0.0, 0.1),
    (ONSET, 0.1),
    (BREAKUP, 0.8),
    (RECOVERY, 0.8),
    (RECOVERY + 12.0, 0.1),
    (LOOP, 0.1),
];
const K_VRAY: Knots = &[
    (0.0, 1.0),
    (ONSET, 1.0),
    (BREAKUP, 12.0),
    (RECOVERY, 8.0),
    (LOOP, 1.0),
];
const K_VSURGE: Knots = &[
    (0.0, 0.5),
    (ONSET, 0.5),
    (EXPANSION, 2.0),
    (PEAK, 6.0),
    (RECOVERY, 6.0),
    (LOOP, 0.5),
];
const K_BULGE: Knots = &[
    (0.0, 0.0),
    (ONSET, 0.0),
    (BREAKUP, 0.6),
    (PEAK, 1.2),
    (RECOVERY, 0.8),
    (RECOVERY + 15.0, 0.0),
    (LOOP, 0.0),
];
const K_BULGE_ALPHA: Knots = &[
    (0.0, 10.0),
    (EXPANSION, 10.0),
    (RECOVERY, -40.0),
    (LOOP, 10.0),
];
const K_BULGE_WIDTH: Knots = &[
    (0.0, 15.0),
    (ONSET, 15.0),
    (EXPANSION, 20.0),
    (RECOVERY, 60.0),
    (LOOP, 15.0),
];
const K_PATCHES: Knots = &[
    (0.0, 0.0),
    (RECOVERY, 0.0),
    (RECOVERY + 3.0, 1.0),
    (RECOVERY + 16.0, 1.0),
    (LOOP, 0.0),
];
const K_ACTIVITY: Knots = &[
    (0.0, 0.02),
    (ONSET, 0.15),
    (EXPANSION, 0.55),
    (PEAK, 1.0),
    (RECOVERY, 0.75),
    (LOOP, 0.02),
];

/// A keyframe curve at loop time `u`: smoothstep between knots, except that segments
/// starting at the onset use `out_quad`, the sudden breakup (AUR-A3). Every knot has
/// zero slope on its smoothstep side, so each parameter is continuous across the wrap.
fn curve(knots: Knots, u: f64) -> f64 {
    let i = knots
        .partition_point(|k| k.0 <= u)
        .clamp(1, knots.len() - 1);
    let ((u0, a), (u1, b)) = (knots[i - 1], knots[i]);
    let x = (u - u0) / (u1 - u0);
    let shape = if u0 == ONSET {
        ease::out_quad(x)
    } else {
        ease::smoothstep(x)
    };
    ease::lerp(a, b, shape)
}

/// Loop time in `[0, 50)` for scene time `t`.
fn loop_time(t: f64) -> f64 {
    geom::wrap(if t.is_finite() { t } else { 0.0 }, LOOP)
}

/// The keyframed substorm at scene time `t` (AUR-A6).
pub(crate) fn keyframes(t: f64) -> Params {
    let u = loop_time(t);
    let d0 = curve(K_D0, u);
    Params {
        d0: [d0, d0 + curve(K_OFF2, u), d0 + curve(K_OFF3, u)],
        amp: [curve(K_A1, u), curve(K_A2, u), curve(K_A3, u)],
        e0: curve(K_E0, u),
        fold: curve(K_FOLD, u),
        rays: curve(K_RAYS, u),
        v_ray: curve(K_VRAY, u),
        v_surge: curve(K_VSURGE, u),
        bulge: curve(K_BULGE, u),
        bulge_alpha: curve(K_BULGE_ALPHA, u).to_radians(),
        bulge_width: curve(K_BULGE_WIDTH, u).to_radians(),
        patches: curve(K_PATCHES, u),
        activity: curve(K_ACTIVITY, u),
    }
}

/// The AL index (nT) at scene time `t`: `−(30 + 800a)` (AUR-A6). The label reads the
/// same formula from the current step's activity.
#[cfg(test)]
pub(crate) fn al_nt(t: f64) -> f64 {
    al_from(keyframes(t).activity)
}

/// The AL index (nT) for activity `a`.
fn al_from(activity: f64) -> f64 {
    -(30.0 + 800.0 * activity)
}

/// Pulsating-patch brightness in [0, 1]: on 40 % of each `period` with 0.4 s smooth
/// edges, so the value exceeds ½ for exactly 0.4·period (AUR-A6).
pub(crate) fn patch_pulse(t: f64, period: f64, phase: f64) -> f64 {
    const EDGE: f64 = 0.4;
    let on = 0.4 * period;
    let tau = geom::wrap(t / period + phase, 1.0) * period;
    if tau < EDGE {
        ease::smoothstep(tau / EDGE)
    } else if tau < on {
        1.0
    } else {
        1.0 - ease::smoothstep((tau - on) / EDGE)
    }
}

// --- World --------------------------------------------------------------------------

/// The fixed stations along the arc and each mode's anti-alias weight there: a mode
/// fades out where it has fewer than 4 samples per wavelength, so flank folds and rays
/// never alias (unresolved rays contribute their mean).
#[derive(Debug)]
struct Stations {
    s: [f64; M],
    fold_w: [[f64; 4]; M],
    ray_w: [[f64; 3]; M],
}

impl Stations {
    fn new() -> Self {
        let s: [f64; M] = std::array::from_fn(|j| {
            let a = -ALPHA_MAX + 2.0 * ALPHA_MAX * j as f64 / (M - 1) as f64;
            S_SCALE * a.tan()
        });
        let spacing = |j: usize| {
            let (lo, hi) = (j.saturating_sub(1), (j + 1).min(M - 1));
            (s[hi] - s[lo]) / (hi - lo) as f64
        };
        let weight = |lambda: f64, ds: f64| (lambda / (2.0 * ds) - 1.0).clamp(0.0, 1.0);
        let fold_w = std::array::from_fn(|j| FOLD_LAMBDA.map(|l| weight(l, spacing(j))));
        let ray_w = std::array::from_fn(|j| RAY_LAMBDA.map(|l| weight(l, spacing(j))));
        Self { s, fold_w, ray_w }
    }
}

/// One station's projected geometry and emission amplitudes for the current step.
#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    /// Azimuth (rad, east positive) and ground distance (km).
    alpha: f32,
    d: f32,
    /// Green flux times slant factor, Φ·S.
    green: f32,
    /// Lagged red source times slant factor, R·S.
    red: f32,
    /// Green scale height step, `H = 22·(1 + 0.8·step/15)` km.
    h_step: u8,
}

/// One auroral sheet: integrated fold and ray phases, the lagged red source per station
/// and the current samples.
#[derive(Debug, Clone)]
struct Curtain {
    theta: [f64; 4],
    ray_phase: [f64; 3],
    red: [f64; M],
    samples: [Sample; M],
}

/// Fold amplitudes (km) for fold factor `F`: the bend is fixed, the others have slope
/// amplitude `F·σ_k`.
fn fold_amplitudes(fold: f64) -> [f64; 4] {
    let a = |k: usize| fold * FOLD_SLOPE[k - 1] * FOLD_LAMBDA[k] / TAU;
    [BEND_KM, a(1), a(2), a(3)]
}

const H_STEPS: usize = 16;

/// Quantise `H/22 ∈ [1, 1.8]` to one of 16 steps.
fn h_step(h_ratio: f64) -> u8 {
    (((h_ratio - 1.0) / 0.8 * (H_STEPS - 1) as f64).round()).clamp(0.0, (H_STEPS - 1) as f64) as u8
}

impl Curtain {
    /// A dark sheet with zero phases (seeded before the first frame).
    fn new() -> Self {
        Self {
            theta: [0.0; 4],
            ray_phase: [0.0; 3],
            red: [0.0; M],
            samples: [Sample::default(); M],
        }
    }

    /// Advance the fold and ray phases by `dt`: `θ_k −= κ_k c_k dt`, so mode k drifts at
    /// velocity c_k and speed changes never make the pattern jump.
    fn advance(&mut self, p: &Params, dt: f64) {
        let c = [0.3, -p.v_surge, 1.2, -0.8];
        for (k, theta) in self.theta.iter_mut().enumerate() {
            *theta = geom::wrap(*theta - FOLD_KAPPA[k] * c[k] * dt, TAU);
        }
        for (m, phase) in self.ray_phase.iter_mut().enumerate() {
            let v = p.v_ray * RAY_V[m];
            *phase = geom::wrap(*phase - RAY_KAPPA[m] * v * dt, TAU);
        }
    }

    /// Recompute every station's geometry and flux for curtain amplitude `amp` at
    /// distance `d0` with fold amplitudes `folds`, relaxing the red source over `dt`.
    fn sample(&mut self, st: &Stations, p: &Params, amp: f64, d0: f64, folds: [f64; 4], dt: f64) {
        let zb = lower_border_km(p.e0);
        let decay = red_decay(dt);
        for j in 0..M {
            let s = st.s[j];
            let (mut xi, mut dxi) = (0.0, 0.0);
            for k in 0..4 {
                let wa = st.fold_w[j][k] * folds[k];
                if wa != 0.0 {
                    let kappa = FOLD_KAPPA[k];
                    let (sn, cs) = (kappa * s + self.theta[k]).sin_cos();
                    xi += wa * sn;
                    dxi += wa * kappa * cs;
                }
            }
            let dd = d0 + xi;
            let d = s.hypot(dd);
            let alpha = s.atan2(dd);
            // Rays relative to their mean; unresolved families contribute exactly 1.
            let rho_rel = (0..3)
                .map(|m| {
                    let w = st.ray_w[j][m];
                    if w == 0.0 {
                        1.0
                    } else {
                        let term = ray_term(s, RAY_KAPPA[m], self.ray_phase[m]) / RHO_MEAN;
                        w * term + 1.0 - w
                    }
                })
                .sum::<f64>()
                / 3.0;
            let x = (alpha - p.bulge_alpha) / p.bulge_width;
            let bulge = 1.0 + p.bulge * (-x * x).exp();
            let phi = amp * bulge * ((1.0 - p.rays) + p.rays * rho_rel);
            self.red[j] = relax_by(self.red[j], phi, decay);
            let slant = slant_factor(dxi, s, dd, zb - drop_km(d));
            self.samples[j] = Sample {
                alpha: alpha as f32,
                d: d as f32,
                green: (phi * slant) as f32,
                red: (self.red[j] * slant.min(RED_SLANT_MAX)) as f32,
                h_step: h_step(1.0 + 0.8 * p.rays * RHO_MEAN * rho_rel),
            };
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Star {
    alpha: f64,
    e: f64,
    mag: f64,
    phase: f64,
    rate: f64,
}

/// A spruce spire: azimuth, height and half-width (rad).
#[derive(Debug, Clone, Copy)]
struct Tree {
    alpha: f64,
    height: f64,
    half: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Patch {
    alpha: f64,
    e: f64,
    half_a: f64,
    half_e: f64,
    period: f64,
    phase: f64,
}

/// Value-noise octaves of the far-shore ridge: node spacing (deg) and weight.
const RIDGE_OCTAVES: [(f64, f64); 3] = [(20.0, 0.6), (7.0, 0.28), (2.5, 0.12)];

/// A seeded forest in angle: spire heights and gaps (deg) and half-width per height.
#[derive(Debug, Clone, Copy)]
struct Forest {
    height: (f64, f64),
    gap: (f64, f64),
    half: f64,
}

/// Spires far off across the lake: at up to `BIG_TREE_F` px/rad (80×24 is ≈ 35) they
/// are 1–2 px, which is all a spruce comb needs.
const FAR_FOREST: Forest = Forest {
    height: (1.2, 3.0),
    gap: (1.5, 4.0),
    half: 0.28,
};
/// Above it the far spires would be a comb of 1-px notches against the skyglow, so
/// fewer, taller, broader spires are shown, each with visible triangular flanks. With the
/// ridge (≤ 4°) they stay within the 8° glow.
const NEAR_FOREST: Forest = Forest {
    height: (2.5, 4.0),
    gap: (2.5, 6.0),
    half: 0.35,
};
const BIG_TREE_F: f64 = 60.0;

/// The seeded, size-independent setting: star catalogue, ridge, both forests and
/// patches.
#[derive(Debug, Default)]
struct Setting {
    stars: Vec<Star>,
    ridge: [Vec<f64>; 3],
    trees: Vec<Tree>,
    big_trees: Vec<Tree>,
    patches: [Patch; PATCHES],
}

/// Spires across the whole horizon for `forest`, sorted by azimuth.
fn plant(rng: &mut impl Rng, forest: Forest) -> Vec<Tree> {
    let mut trees = Vec::new();
    let mut a: f64 = -90.0 + rng.random_range(0.0..2.0);
    while a < 90.0 {
        let height: f64 = rng.random_range(forest.height.0..forest.height.1);
        trees.push(Tree {
            alpha: a.to_radians(),
            height: height.to_radians(),
            half: (forest.half * height).to_radians(),
        });
        a += rng.random_range(forest.gap.0..forest.gap.1);
    }
    trees
}

impl Setting {
    fn seed(rng: &mut impl Rng) -> Self {
        let mut stars: Vec<Star> = (0..STARS)
            .map(|_| Star {
                alpha: rng.random_range(-PI / 2.0..PI / 2.0),
                // Uniform on the hemisphere.
                e: rng.random_range(0.0f64..1.0).asin(),
                // N(<m) ∝ 10^{0.5 m}, faintest 6.5.
                mag: (6.5 + 2.0 * rng.random_range(1e-3f64..1.0).log10()).max(-1.0),
                phase: rng.random_range(0.0..TAU),
                rate: rng.random_range(1.5..4.0),
            })
            .collect();
        stars.sort_by(|a, b| a.mag.total_cmp(&b.mag));
        let ridge = RIDGE_OCTAVES.map(|(spacing, _)| {
            let n = (180.0 / spacing).ceil() as usize + 2;
            (0..n).map(|_| rng.random_range(0.0..1.0)).collect()
        });
        let trees = plant(rng, FAR_FOREST);
        let patches = std::array::from_fn(|_| Patch {
            alpha: rng.random_range(-50.0f64..50.0).to_radians(),
            e: rng.random_range(35.0f64..60.0).to_radians(),
            half_a: rng.random_range(3.0f64..6.0).to_radians(),
            half_e: rng.random_range(1.5f64..3.0).to_radians(),
            period: rng.random_range(4.0..12.0),
            phase: rng.random_range(0.0..1.0),
        });
        // Seeded last, so the rest of the setting does not depend on it.
        let big_trees = plant(rng, NEAR_FOREST);
        Self {
            stars,
            ridge,
            trees,
            big_trees,
            patches,
        }
    }

    /// The forest shown at scale `f` (px/rad).
    fn trees_for(&self, f: f64) -> &[Tree] {
        if f > BIG_TREE_F {
            &self.big_trees
        } else {
            &self.trees
        }
    }

    /// Ridge elevation (rad) at azimuth `alpha`: 0.8–4° of 3-octave value noise.
    fn ridge(&self, alpha: f64) -> f64 {
        let deg = alpha.to_degrees().clamp(-90.0, 90.0) + 90.0;
        let mut n = 0.0;
        for (nodes, (spacing, weight)) in self.ridge.iter().zip(RIDGE_OCTAVES) {
            let x = deg / spacing;
            // Every octave has ≥ 11 nodes, so i + 1 is always in range.
            let i = (x as usize).min(nodes.len() - 2);
            n += weight * ease::lerp(nodes[i], nodes[i + 1], ease::smoothstep(x - i as f64));
        }
        (0.8 + 3.2 * n).to_radians()
    }

    /// Highest spire top (rad) over the azimuth interval `[lo, hi]`, if any tree
    /// reaches into it: a triangle profile maximised over the interval.
    fn tree_top(&self, trees: &[Tree], lo: f64, hi: f64) -> Option<f64> {
        let first = trees.partition_point(|t| t.alpha + t.half < lo);
        trees[first..]
            .iter()
            .take_while(|t| t.alpha - t.half <= hi)
            .map(|t| {
                let gap = if t.alpha < lo {
                    lo - t.alpha
                } else if t.alpha > hi {
                    t.alpha - hi
                } else {
                    0.0
                };
                self.ridge(t.alpha) + t.height * (1.0 - gap / t.half).max(0.0)
            })
            .max_by(f64::total_cmp)
    }
}

// --- View ---------------------------------------------------------------------------

/// Pixel emission of one curtain crossing a column.
#[derive(Debug, Clone, Copy, Default)]
struct Layer {
    d: f32,
    drop: f32,
    zb: f32,
    h_step: u8,
    green: f32,
    red: f32,
    pink: f32,
    /// Rows `y_top..=y_low` can be lit: `z ≤ 600 km` down to the hem's lowest pixel.
    y_top: i32,
    y_low: i32,
}

impl Layer {
    /// Altitude (km) seen through a pixel row with elevation tangent `tan_e`: the
    /// inverse of `elevation`, `z = d·tan e + d²/2R⊕` (AUR-A4).
    #[inline]
    fn altitude(&self, tan_e: f32) -> f32 {
        self.d * tan_e + self.drop
    }
}

/// Precomputed altitude profiles and tone map (1-km and 0.5-km LUTs).
#[derive(Debug)]
struct Luts {
    green: Vec<f32>,
    red: Vec<f32>,
    tone: Vec<f32>,
}

const GREEN_U0: f64 = -2.0;
const GREEN_U1: f64 = 400.0;
const GREEN_DU: f64 = 0.5;
const GREEN_N: usize = ((GREEN_U1 - GREEN_U0) / GREEN_DU) as usize + 1;
const RED_Z1: f64 = 600.0;
const RED_N: usize = RED_Z1 as usize + 1;
const TONE_N: usize = 1024;
const TONE_MAX: f64 = 7.0;

impl Luts {
    fn new() -> Self {
        let mut green = Vec::with_capacity(H_STEPS * GREEN_N);
        for step in 0..H_STEPS {
            let h = H_GREEN * (1.0 + 0.8 * step as f64 / (H_STEPS - 1) as f64);
            green.extend(
                (0..GREEN_N).map(|i| green_profile(GREEN_U0 + GREEN_DU * i as f64, 0.0, h) as f32),
            );
        }
        let red = (0..RED_N).map(|z| red_profile(z as f64) as f32).collect();
        let tone = (0..TONE_N)
            .map(|i| (1.0 - (-TONE_MAX * i as f64 / (TONE_N - 1) as f64).exp()) as f32)
            .collect();
        Self { green, red, tone }
    }

    #[inline]
    fn green(&self, step: u8, u: f32) -> f32 {
        let x = (u - GREEN_U0 as f32) * (1.0 / GREEN_DU as f32) + 0.5;
        if x >= 0.0 && (x as usize) < GREEN_N {
            self.green[usize::from(step) * GREEN_N + x as usize]
        } else {
            0.0
        }
    }

    #[inline]
    fn red(&self, z: f32) -> f32 {
        let x = z + 0.5;
        if x >= 0.0 && (x as usize) < RED_N {
            self.red[x as usize]
        } else {
            0.0
        }
    }

    /// `1 − e^{−e}` for emission `e` in units of `E_SAT`.
    #[inline]
    fn tone(&self, e: f32) -> f32 {
        let i = (e * ((TONE_N - 1) as f64 / TONE_MAX) as f32).round();
        self.tone[(i.max(0.0) as usize).min(TONE_N - 1)]
    }
}

/// Rows of a column that share one dither threshold: partial coverage breaks into short
/// field-aligned vertical strands, like the rays themselves, rather than speckle.
const STRAND_PX: usize = 3;

/// A toroidal blue-noise threshold tile of `w` columns × `g` groups of `px` rows.
#[derive(Debug, Clone, Copy)]
struct TileSpec {
    w: usize,
    g: usize,
    px: usize,
    /// Void-and-cluster energy: a Gaussian of this width (px), measured with a group
    /// `px` rows tall, truncated at `reach` columns and groups (≈ 3.6σ).
    sigma: f64,
    reach: (i32, i32),
    seed: u64,
}

/// Curtain green: strands of `STRAND_PX` rows, 128 px × 96 px.
const STRANDS: TileSpec = TileSpec {
    w: 128,
    g: 32,
    px: STRAND_PX,
    sigma: 2.2,
    reach: (8, 3),
    seed: 0xA0B0,
};
/// Diffuse light (the red fringe and the pulsating patches): square pixels, 64 × 64 px,
/// so a soft glow stipples evenly instead of breaking into ray-like strands.
const SPECKLE: TileSpec = TileSpec {
    w: 64,
    g: 64,
    px: 1,
    sigma: 1.5,
    reach: (5, 5),
    seed: 0xA0B1,
};

/// Void-and-cluster ranks (Ulichney 1993) of the tile's cells, as thresholds
/// `(rank + ½)/n`: every prefix of the ranking is spread as evenly as possible, so any
/// share of lit cells has neither the clumps and holes of white noise nor the visible
/// period of an ordered dither. The strand metric counts a cell as 1 × 3 px, so
/// horizontal neighbours repel most and dense coverage stacks strands into vertical
/// threads. Energies are integer sums, so the ranking is exact on every platform.
fn void_and_cluster(spec: TileSpec) -> Vec<f32> {
    let TileSpec { w, g, px, .. } = spec;
    let mut rng = SplitMix64::new(spec.seed);
    let n = w * g;
    let (rx, rg) = spec.reach;
    let mut kernel = Vec::new();
    for dx in -rx..=rx {
        for dg in -rg..=rg {
            let d2 = f64::from(dx * dx) + f64::from(dg * dg) * (px * px) as f64;
            let k = (65536.0 * (-d2 / (2.0 * spec.sigma * spec.sigma)).exp()).round() as i64;
            kernel.push((dx, dg, k));
        }
    }
    let splat = |energy: &mut [i64], i: usize, sign: i64| {
        let (x, gi) = ((i / g) as i32, (i % g) as i32);
        for &(dx, dg, k) in &kernel {
            let xx = (x + dx).rem_euclid(w as i32) as usize;
            let gg = (gi + dg).rem_euclid(g as i32) as usize;
            energy[xx * g + gg] += sign * k;
        }
    };
    // The densest minority point, and the emptiest majority point (first on ties).
    let tightest = |on: &[bool], e: &[i64]| {
        (0..n)
            .filter(|&i| on[i])
            .fold(None, |best: Option<usize>, i| match best {
                Some(b) if e[b] >= e[i] => Some(b),
                _ => Some(i),
            })
            .unwrap_or(0)
    };
    let largest_void = |on: &[bool], e: &[i64]| {
        (0..n)
            .filter(|&i| !on[i])
            .fold(None, |best: Option<usize>, i| match best {
                Some(b) if e[b] <= e[i] => Some(b),
                _ => Some(i),
            })
            .unwrap_or(0)
    };
    // A random 10 % pattern, relaxed until moving its densest point fills its
    // emptiest void with that same point.
    let ones = n / 10;
    let (mut on, mut energy) = (vec![false; n], vec![0i64; n]);
    let mut placed = 0;
    while placed < ones {
        let i = rng.below(n as u32) as usize;
        if !on[i] {
            on[i] = true;
            splat(&mut energy, i, 1);
            placed += 1;
        }
    }
    // Each swap lowers the pattern's energy, so this settles in far fewer than `n`
    // swaps; the bound only guards a tile constant that would make it cycle.
    for _ in 0..n {
        let c = tightest(&on, &energy);
        on[c] = false;
        splat(&mut energy, c, -1);
        let v = largest_void(&on, &energy);
        on[v] = true;
        splat(&mut energy, v, 1);
        if v == c {
            break;
        }
    }
    let mut rank = vec![0usize; n];
    // Ranks below the pattern: remove the densest points first.
    let (mut on1, mut e1) = (on.clone(), energy.clone());
    for r in (0..ones).rev() {
        let c = tightest(&on1, &e1);
        on1[c] = false;
        splat(&mut e1, c, -1);
        rank[c] = r;
    }
    // Ranks above it: fill the emptiest voids.
    for r in ones..n {
        let v = largest_void(&on, &energy);
        on[v] = true;
        splat(&mut energy, v, 1);
        rank[v] = r;
    }
    rank.iter().map(|&r| (r as f32 + 0.5) / n as f32).collect()
}

/// The two blue-noise tiles, built once. The cache holds a pure function of constants
/// (the seed is part of the tile spec, not `ctx.rng`), so every scene and size shares
/// it and it does not break GEN-R7 determinism.
fn tiles() -> &'static [Vec<f32>; 2] {
    static TILES: std::sync::OnceLock<[Vec<f32>; 2]> = std::sync::OnceLock::new();
    TILES.get_or_init(|| [void_and_cluster(STRANDS), void_and_cluster(SPECKLE)])
}

/// A fixed dither threshold in [0, 1) for pixel `(x, y)` from tile `tile` built to
/// `spec`, shared by its group of rows. It never changes, so a still sky does not
/// shimmer.
#[inline]
fn tile_threshold(tile: &[f32], spec: TileSpec, x: usize, y: usize) -> f32 {
    tile[(x % spec.w) * spec.g + (y / spec.px) % spec.g]
}

/// The dither thresholds of a view: field-aligned strands for curtain green, square
/// blue noise for diffuse light.
#[derive(Debug, Clone, Copy, Default)]
struct Dither {
    strands: &'static [f32],
    speckle: &'static [f32],
}

impl Dither {
    fn new() -> Self {
        let [strands, speckle] = tiles();
        Self { strands, speckle }
    }

    #[inline]
    fn at(self, x: usize, y: usize, soft: bool) -> f32 {
        if soft {
            tile_threshold(self.speckle, SPECKLE, x, y)
        } else {
            tile_threshold(self.strands, STRANDS, x, y)
        }
    }
}

/// A sky pixel's emission before dithering.
#[derive(Debug, Clone, Copy, Default)]
struct Glow {
    /// Tone-mapped brightness in [0, 1).
    tone: f32,
    /// Red share of the emission, R/(G+R).
    red: f32,
    /// Inside a bright pink hem.
    hem: bool,
    /// Diffuse light (red-dominant, or mostly a pulsating patch): dithered with square
    /// blue noise rather than field-aligned strands.
    soft: bool,
}

/// A sky pixel's emission before tone mapping.
#[derive(Debug, Clone, Copy, Default)]
struct Emit {
    green: f32,
    /// Red after `RED_WEIGHT`, smoothed along the rows by `View::diffuse_red`.
    red: f32,
    pink: f32,
    /// The pulsating patches' share of `green`.
    patch: f32,
}

/// An empty row range for `View::emit_rows`.
const NO_ROWS: (usize, usize) = (usize::MAX, 0);

/// Chance that a lit pixel is red for red share `red` = R/(G+R): 0 below the band,
/// 1 above it, smoothstep across it. A pixel carries its whole tone in one role, so a
/// linear chance would scatter full-brightness red specks through green ray cores.
fn red_chance(red: f32) -> f32 {
    ease::smoothstep(f64::from((red - RED_BAND.0) / (RED_BAND.1 - RED_BAND.0))) as f32
}

impl Glow {
    /// The pixel's ink at `gain`× brightness under dither threshold `b` ∈ [0, 1), or
    /// `None` if unlit. Faint emission is expressed by pixel density: the theme floors
    /// lit roles at 30 % brightness, so a tone below 0.3 is lit for that share of the
    /// thresholds, a tone below `TONE_FLOOR` never, and a red-dominant one below
    /// `RED_TONE_FLOOR` never. Among lit pixels `b` is rescaled to be uniform on [0, 1)
    /// again, so red wins with probability `red_chance`: pure green cores, a pure red
    /// fringe and a stipple only where the two meet. The pink hem is crisp.
    fn dither(self, gain: f32, b: f32) -> Option<Ink> {
        let tone = self.tone * gain;
        let chance = red_chance(self.red);
        if tone < TONE_FLOOR || tone <= LIT_THRESHOLD * b || (chance > 0.5 && tone < RED_TONE_FLOOR)
        {
            return None;
        }
        let u = b * (LIT_THRESHOLD / tone).max(1.0);
        let role = if self.hem {
            Role::Accent
        } else if chance > u {
            Role::Hot
        } else {
            Role::Particle
        };
        Some(Ink::new(role, ink::level(f64::from(tone * 255.0))))
    }
}

/// Height (px) of the emission fade under the top edge for a view `rows` pixels tall.
fn top_fade_px(rows: usize) -> f64 {
    (TOP_FADE * rows as f64).max(TOP_FADE_MIN_PX)
}

/// Radius `k` of a box whose three-fold convolution has standard deviation closest to
/// `sigma` px: its variance is `k(k + 1)`.
fn box_radius(sigma: f64) -> usize {
    ((sigma * sigma + 0.25).sqrt() - 0.5).round().max(0.0) as usize
}

/// One running-sum box pass of radius `k`, averaging over the part of the window
/// inside the row, so the edges keep their level.
fn box_pass(src: &[f32], dst: &mut [f32], k: usize) {
    let n = src.len();
    let (mut lo, mut hi, mut sum) = (0, 0, 0.0f32);
    for (i, out) in dst.iter_mut().enumerate().take(n) {
        while hi < (i + k + 1).min(n) {
            sum += src[hi];
            hi += 1;
        }
        while lo < i.saturating_sub(k) {
            sum -= src[lo];
            lo += 1;
        }
        *out = (sum / (hi - lo) as f32).max(0.0);
    }
}

/// The skyglow `px` pixels above the horizon for a glow `height` px tall, or `None`
/// where it has faded out: its ink, solid low down, and the share of pixels it covers,
/// which thins to 0 over the top `GLOW_FEATHER` of the glow. Even the ramp's darkest
/// ink is not black, so a solid glow would end in a hard line.
fn skyglow(px: f64, height: f64) -> Option<(Ink, f32)> {
    let k = 1.0 - px.max(0.0) / height;
    let v = GLOW_MAX * k * k;
    (k > 0.0).then(|| (ink::seq(v), (k / GLOW_FEATHER).min(1.0) as f32))
}

/// A star that fits the current view, with its cell and draw attributes.
#[derive(Debug, Clone, Copy)]
struct ShownStar {
    cx: i32,
    cy: i32,
    idx: u16,
    bright: bool,
}

/// A patch active this step: its centre now and brightness.
#[derive(Debug, Clone, Copy, Default)]
struct LivePatch {
    alpha: f64,
    e: f64,
    half_a: f64,
    half_e: f64,
    gain: f64,
}

/// Which parts of the label fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Label {
    #[default]
    None,
    Short,
    Full,
}

/// Everything derived from the terminal size (rebuilt in `resize`) plus the raster.
#[derive(Debug, Default)]
struct View {
    w: usize,
    rows: usize,
    /// Horizon: the pixel boundary between sky row `y_h − 1` and lake row `y_h`.
    y_h: usize,
    lake: bool,
    /// Scale (px/rad): `x = W/2 + f·α`, `y = y_h − f·e`.
    f: f64,
    tan_e: Vec<f32>,
    ext: Vec<f32>,
    /// Emission factor per sky row: 0 at the top edge, 1 below the top fade.
    top: Vec<f32>,
    /// First silhouette row per column (`y_h` where the ridge is below a pixel).
    ridge_top: Vec<i32>,
    /// The same without the spruce spires: the bare ridge line.
    ridge_base: Vec<i32>,
    tree: Vec<bool>,
    dither: Dither,
    /// Box radius (px) of the red diffusion: three passes approximate the Gaussian.
    red_box: usize,
    /// Emission before tone mapping, `w` per sky row, and the rows that hold any
    /// (`emit_rows.0 > emit_rows.1` when none).
    emit: Vec<Emit>,
    emit_rows: (usize, usize),
    /// Two rows of scratch for the red diffusion.
    blur: [Vec<f32>; 2],
    /// Skyglow per sky row: its ink and the pixel share it covers (`None` above the
    /// glow).
    skyglow: Vec<Option<(Ink, f32)>>,
    ripple_cos: Vec<f32>,
    ripple_sin: Vec<f32>,
    stars: Vec<ShownStar>,
    layers: Vec<[Layer; MAX_LAYERS]>,
    nlayers: Vec<u8>,
    /// Undithered sky emission, `w` per sky row: ASCII sky cells re-dither it at cell
    /// resolution.
    glow: Vec<Glow>,
    pixels: Pixels,
    label: Label,
}

impl View {
    fn new(size: Size, setting: &Setting) -> Self {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let rows = 2 * h;
        let lake = h >= 14;
        let y_h = ((if lake { 0.80 } else { 0.92 }) * rows as f64).round() as usize;
        let f = (y_h as f64 / 62f64.to_radians()).max(w as f64 / PI);
        let elev = |y: usize| (y_h as f64 - y as f64 - 0.5) / f;
        let tan_e = (0..y_h).map(|y| elev(y).tan() as f32).collect();
        let ext = (0..y_h)
            .map(|y| {
                let sin_e = elev(y).max(2f64.to_radians()).sin();
                (-0.12 * (1.0 / sin_e - 1.0)).exp() as f32
            })
            .collect();
        let mut view = Self {
            w,
            rows,
            y_h,
            lake,
            f,
            skyglow: {
                // At least 3 px, so small views keep a readable silhouette.
                let height = (f * GLOW_DEG.to_radians()).max(3.0);
                (0..y_h)
                    .map(|y| skyglow((y_h - y) as f64 - 0.5, height))
                    .collect()
            },
            tan_e,
            ext,
            top: {
                let fade = top_fade_px(rows);
                (0..y_h)
                    .map(|y| ease::smoothstep(y as f64 / fade) as f32)
                    .collect()
            },
            dither: Dither::new(),
            red_box: box_radius(f * RED_DIFFUSE_DEG.to_radians()),
            emit: vec![Emit::default(); w * y_h],
            emit_rows: NO_ROWS,
            blur: [vec![0.0; w], vec![0.0; w]],
            ripple_cos: (0..w).map(|x| (0.13 * x as f64).cos() as f32).collect(),
            ripple_sin: (0..w).map(|x| (0.13 * x as f64).sin() as f32).collect(),
            layers: vec![[Layer::default(); MAX_LAYERS]; w],
            nlayers: vec![0; w],
            glow: vec![Glow::default(); w * y_h],
            pixels: Pixels::new(size),
            label: if w < 24 || h < 10 {
                Label::None
            } else if w < 60 {
                Label::Short
            } else {
                Label::Full
            },
            ..Self::default()
        };
        view.build_silhouette(setting);
        view.pick_stars(setting);
        view
    }

    #[cfg(test)]
    fn top_fade_px(&self) -> f64 {
        top_fade_px(self.rows)
    }

    fn alpha_at(&self, x: f64) -> f64 {
        (x - self.w as f64 / 2.0) / self.f
    }

    /// Pixel row whose centre sits at or below elevation `e`: `y_h − 0.5 − f·e`.
    fn row_of(&self, e: f64) -> f64 {
        self.y_h as f64 - 0.5 - self.f * e
    }

    /// The far-shore ridge and spruce spires per column; silhouette rows are those whose
    /// centre lies below the silhouette elevation.
    fn build_silhouette(&mut self, setting: &Setting) {
        let half = 0.5 / self.f;
        let top_row = |e: f64| (self.row_of(e).floor() as i32 + 1).clamp(0, self.y_h as i32);
        let (mut ridge_top, mut ridge_base, mut tree) = (Vec::new(), Vec::new(), Vec::new());
        let trees = setting.trees_for(self.f);
        for x in 0..self.w {
            let a = self.alpha_at(x as f64 + 0.5);
            let ridge = setting.ridge(a);
            let base = top_row(ridge);
            let (top, tip) = match setting.tree_top(trees, a - half, a + half) {
                Some(e) if e > ridge => {
                    let top = top_row(e);
                    // A tip only where a spire's centre is in this column and rises.
                    let centred = trees.iter().any(|t| (t.alpha - a).abs() <= half);
                    (top, centred && top < base)
                }
                _ => (base, false),
            };
            ridge_top.push(top);
            ridge_base.push(base);
            tree.push(tip);
        }
        self.ridge_top = ridge_top;
        self.ridge_base = ridge_base;
        self.tree = tree;
    }

    /// Brightest-first stars until 2 % of the sky cells hold one, so the limiting
    /// magnitude adapts to resolution; only cells wholly above the silhouette qualify.
    fn pick_stars(&mut self, setting: &Setting) {
        let h = self.rows / 2;
        let sky_cell = |cx: i32, cy: i32| {
            cx >= 0
                && (cx as usize) < self.w
                && cy >= 0
                && (cy as usize) < h
                && 2 * cy + 1 < self.ridge_top[cx as usize]
        };
        let sky_cells: usize = (0..self.w)
            .map(|x| (self.ridge_top[x].max(0) as usize) / 2)
            .sum();
        let cap = (STAR_FRACTION * sky_cells as f64) as usize;
        let mut taken = vec![false; self.w * h];
        let mut stars = Vec::with_capacity(cap);
        for (idx, s) in setting.stars.iter().enumerate() {
            if stars.len() >= cap {
                break;
            }
            let cx = (self.w as f64 / 2.0 + self.f * s.alpha).floor() as i32;
            let cy = ((self.row_of(s.e) + 0.5).floor() as i32).div_euclid(2);
            if sky_cell(cx, cy) && !taken[cy as usize * self.w + cx as usize] {
                taken[cy as usize * self.w + cx as usize] = true;
                stars.push(ShownStar {
                    cx,
                    cy,
                    idx: idx as u16,
                    bright: false,
                });
            }
        }
        let bright = stars.len().div_ceil(10);
        for s in &mut stars[..bright] {
            s.bright = true;
        }
        self.stars = stars;
    }

    fn clear_layers(&mut self) {
        self.nlayers.fill(0);
    }

    /// Zero the rows that held emission last frame.
    fn clear_emit(&mut self) {
        let (lo, hi) = self.emit_rows;
        if lo <= hi {
            self.emit[lo * self.w..(hi + 1) * self.w].fill(Emit::default());
        }
        self.emit_rows = NO_ROWS;
    }

    /// Add a layer to column `x`; a fifth merges into the one at the nearest distance.
    fn push_layer(&mut self, x: usize, layer: Layer) {
        let n = usize::from(self.nlayers[x]);
        let col = &mut self.layers[x];
        if n < MAX_LAYERS {
            col[n] = layer;
            self.nlayers[x] += 1;
        } else if let Some(near) = col
            .iter_mut()
            .min_by(|a, b| (a.d - layer.d).abs().total_cmp(&(b.d - layer.d).abs()))
        {
            near.green += layer.green;
            near.red += layer.red;
            near.pink += layer.pink;
        }
    }

    /// Splat a curtain into column layers: every column whose centre lies in the
    /// half-open x-span of two consecutive samples gets one layer interpolated between
    /// them, so a fold that doubles back on screen gives a column several layers.
    fn splat(&mut self, samples: &[Sample], zb: f64, pink: f64) {
        let cx = self.w as f64 / 2.0;
        let f = self.f;
        for pair in samples.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.green + a.red + b.green + b.red < 1e-4 {
                continue;
            }
            let (xa, xb) = (cx + f * f64::from(a.alpha), cx + f * f64::from(b.alpha));
            let (lo, hi) = (xa.min(xb), xa.max(xb));
            let first = (lo - 0.5).ceil().max(0.0);
            let end = (hi - 0.5).ceil().min(self.w as f64);
            let mut x = first;
            while x < end {
                let t = (x + 0.5 - xa) / (xb - xa);
                let lerp = |p: f32, q: f32| ease::lerp(f64::from(p), f64::from(q), t);
                let d = lerp(a.d, b.d);
                let green = lerp(a.green, b.green);
                let layer = self.layer(
                    d,
                    zb,
                    green,
                    lerp(a.red, b.red),
                    green * pink,
                    if t < 0.5 { a.h_step } else { b.h_step },
                );
                self.push_layer(x as usize, layer);
                x += 1.0;
            }
        }
    }

    fn layer(&self, d: f64, zb: f64, green: f64, red: f64, pink: f64, h_step: u8) -> Layer {
        let drop = drop_km(d);
        let e_low = elevation(zb - HEM_BELOW, d);
        let e_top = elevation(RED_Z1, d);
        Layer {
            d: d as f32,
            drop: drop as f32,
            zb: zb as f32,
            h_step,
            green: green as f32,
            red: red as f32,
            pink: pink as f32,
            y_top: self.row_of(e_top).ceil() as i32,
            y_low: self.row_of(e_low).floor() as i32,
        }
    }

    /// Sum one sky column's emission from its layers and the live patches into `emit`.
    fn emit_column(&mut self, x: usize, luts: &Luts, patches: &[LivePatch]) {
        let n = usize::from(self.nlayers[x]);
        let layers = self.layers[x];
        let layers = &layers[..n];
        let sil = self.ridge_top[x];
        let alpha = self.alpha_at(x as f64 + 0.5);
        let (mut y0, mut y1) = (sil, -1);
        for l in layers {
            y0 = y0.min(l.y_top);
            y1 = y1.max(l.y_low);
        }
        for p in patches {
            if (alpha - p.alpha).abs() < 2.0 * p.half_a {
                y0 = y0.min(self.row_of(p.e + 2.0 * p.half_e) as i32);
                y1 = y1.max(self.row_of(p.e - 2.0 * p.half_e) as i32 + 1);
            }
        }
        let (y0, y1) = (y0.max(0), y1.min(sil - 1));
        if y0 > y1 {
            return;
        }
        self.emit_rows = (
            self.emit_rows.0.min(y0 as usize),
            self.emit_rows.1.max(y1 as usize),
        );
        // Only the lowest pink border in the column (the brightest curtain's; see
        // `rasterise`) carries the hem: the pink edges of the folds seen above it would
        // scatter as confetti through the curtain body.
        let hem_layer = layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.pink > 0.0)
            .max_by_key(|(_, l)| l.y_low)
            .map(|(i, _)| i);
        for y in y0..=y1 {
            let yu = y as usize;
            let te = self.tan_e[yu];
            let (mut g, mut r, mut p, mut patch) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (i, l) in layers.iter().enumerate() {
                if y < l.y_top || y > l.y_low {
                    continue;
                }
                let z = l.altitude(te);
                let u = z - l.zb;
                g += l.green * luts.green(l.h_step, u);
                r += l.red * luts.red(z);
                // The hem always includes the layer's lowest lit pixel, so it never
                // vanishes when pixels are coarser than the 5 km band.
                if hem_layer == Some(i) && (in_hem_band(u) || y == l.y_low) {
                    p += l.pink;
                }
            }
            for lp in patches {
                let da = (alpha - lp.alpha) / lp.half_a;
                let de = ((self.y_h as f64 - f64::from(y) - 0.5) / self.f - lp.e) / lp.half_e;
                if da.abs() < 2.0 && de.abs() < 2.0 {
                    patch += (lp.gain * (-da * da - de * de).exp()) as f32;
                }
            }
            self.emit[yu * self.w + x] = Emit {
                green: g + patch,
                red: RED_WEIGHT * r,
                pink: p,
                patch,
            };
        }
    }

    /// Diffuse the red along each emitting row: three box passes of radius `red_box`,
    /// a Gaussian of width ≈ `RED_DIFFUSE_DEG` (see there).
    fn diffuse_red(&mut self) {
        let (k, w) = (self.red_box, self.w);
        if k == 0 || w == 0 {
            return;
        }
        let [a, b] = &mut self.blur;
        for y in self.emit_rows.0..=self.emit_rows.1 {
            let row = &mut self.emit[y * w..(y + 1) * w];
            if row.iter().all(|e| e.red == 0.0) {
                continue;
            }
            for (dst, e) in a.iter_mut().zip(row.iter()) {
                *dst = e.red;
            }
            box_pass(a, b, k);
            box_pass(b, a, k);
            box_pass(a, b, k);
            for (e, &r) in row.iter_mut().zip(b.iter()) {
                e.red = r;
            }
        }
    }

    /// Tone-map and dither every emitting sky pixel above the silhouette.
    fn shade(&mut self, luts: &Luts) {
        for y in self.emit_rows.0..=self.emit_rows.1 {
            let ext = self.ext[y] * (1.0 / E_SAT as f32);
            for x in 0..self.w {
                if y as i32 >= self.ridge_top[x] {
                    continue;
                }
                let e = self.emit[y * self.w + x];
                let sum = e.green + e.red + e.pink;
                if sum <= 0.0 {
                    continue;
                }
                let red = e.red / (e.green + e.red).max(1e-12);
                let glow = Glow {
                    tone: self.top[y] * luts.tone(ext * sum),
                    red,
                    hem: e.pink >= 0.25 * e.green && luts.tone(ext * e.pink) >= HEM_TONE,
                    soft: red_chance(red) > 0.5 || e.patch > 0.5 * sum,
                };
                self.glow[y * self.w + x] = glow;
                if let Some(ink) = glow.dither(1.0, self.dither.at(x, y, glow.soft)) {
                    self.pixels.set(x as i32, y as i32, ink);
                }
            }
        }
    }

    /// Skyglow in the still-dark sky just above the silhouette of column `x`.
    fn skyglow_column(&mut self, x: usize) {
        for y in (0..self.ridge_top[x]).rev() {
            let Some((glow, share)) = self.skyglow[y as usize] else {
                break;
            };
            if self.dither.at(x, y as usize, false) < share
                && self
                    .pixels
                    .get(x as i32, y)
                    .is_some_and(|i| i.role == Role::Background)
            {
                self.pixels.set(x as i32, y, glow);
            }
        }
    }

    /// Demote hem pixels with no hem neighbour in the next column on either side (any
    /// of the three rows, so a sloping hem still counts as a run) to green. In place is
    /// safe: a demoted pixel had no hem neighbour, so no other pixel relied on it.
    fn trim_hem(&mut self) {
        let accent =
            |p: &Pixels, x: i32, y: i32| p.get(x, y).is_some_and(|i| i.role == Role::Accent);
        for y in 0..self.y_h as i32 {
            for x in 0..self.w as i32 {
                if !accent(&self.pixels, x, y) {
                    continue;
                }
                let run = [x - 1, x + 1]
                    .iter()
                    .any(|&nx| (y - 1..=y + 1).any(|ny| accent(&self.pixels, nx, ny)));
                if !run {
                    let level = self.pixels.get(x, y).map_or(0, |i| i.level);
                    self.pixels.set(x, y, Ink::new(Role::Particle, level));
                    self.glow[y as usize * self.w + x as usize].hem = false;
                }
            }
        }
    }

    /// Mirror the sky into the lake at 0.45× level with a ripple whose wavefronts travel
    /// toward the viewer. Whole rows drop out on the crests, so mirrored rays read as
    /// vertical streaks cut by horizontal ripple gaps; the x-dependent phase only shifts
    /// pixels sideways. The lake copies the sky's lit pixels and their roles, dimming
    /// only the level: re-dithering the dimmer glow would light it sparsely at random
    /// and dissolve the ray structure into snow. Skyglow and silhouette stay dark.
    fn reflect(&mut self, t: f64) {
        if !self.lake {
            return;
        }
        for y in self.y_h..self.rows {
            let k = y - self.y_h;
            let Some(src) = self.y_h.checked_sub(k + 1) else {
                break;
            };
            let d = (k + 1) as f64;
            let (sp, cp) = (0.9 * d - 1.7 * t).sin_cos();
            if sp > 0.9 {
                continue;
            }
            let (sp, cp) = (sp as f32, cp as f32);
            for x in 0..self.w {
                // sin(0.9d − 1.7t + 0.13x) by angle addition: no per-pixel trig.
                let s = sp * self.ripple_cos[x] + cp * self.ripple_sin[x];
                let sx = x as isize + (0.08 * d as f32 * s).round() as isize;
                let Ok(sx) = usize::try_from(sx) else {
                    continue;
                };
                if let Some(sky) = self.pixels.get(sx as i32, src as i32)
                    && matches!(sky.role, Role::Particle | Role::Hot | Role::Accent)
                {
                    let level = (LAKE_GAIN * f32::from(sky.level)).round() as u8;
                    self.pixels
                        .set(x as i32, y as i32, Ink::new(sky.role, level));
                }
            }
        }
    }
}

// --- Scene --------------------------------------------------------------------------

#[derive(Debug)]
pub struct Aurora {
    t: f64,
    timeline: Timeline,
    params: Params,
    stations: Stations,
    curtains: [Curtain; CURTAINS],
    luts: Luts,
    setting: Setting,
    seeded: bool,
    patches: [LivePatch; PATCHES],
    live_patches: usize,
    view: View,
}

impl Aurora {
    fn new() -> Self {
        Self {
            t: START_T,
            timeline: Timeline::new(&BEATS),
            params: keyframes(START_T),
            stations: Stations::new(),
            curtains: std::array::from_fn(|_| Curtain::new()),
            luts: Luts::new(),
            setting: Setting::default(),
            seeded: false,
            patches: [LivePatch::default(); PATCHES],
            live_patches: 0,
            view: View::default(),
        }
    }

    /// Seed phases and the setting once; the red sources start in equilibrium.
    fn seed(&mut self, rng: &mut impl Rng) {
        for c in &mut self.curtains {
            c.theta = std::array::from_fn(|_| rng.random_range(0.0..TAU));
            c.ray_phase = std::array::from_fn(|_| rng.random_range(0.0..TAU));
        }
        self.setting = Setting::seed(rng);
        self.sample(f64::INFINITY);
        self.seeded = true;
    }

    /// Resample every curtain, relaxing the red sources over `dt`.
    fn sample(&mut self, dt: f64) {
        let p = self.params;
        let folds = fold_amplitudes(p.fold);
        for (ci, c) in self.curtains.iter_mut().enumerate() {
            c.sample(&self.stations, &p, p.amp[ci], p.d0[ci], folds, dt);
        }
    }

    /// The patches alive now: seeded positions drifting east 0.5°/s through recovery.
    fn update_patches(&mut self) {
        self.live_patches = 0;
        if self.params.patches <= 0.0 {
            return;
        }
        let drift = (0.5 * (loop_time(self.t) - RECOVERY)).to_radians();
        for p in &self.setting.patches {
            let gain = PATCH_GAIN * self.params.patches * patch_pulse(self.t, p.period, p.phase);
            if gain > 1e-3 {
                self.patches[self.live_patches] = LivePatch {
                    alpha: p.alpha + drift,
                    e: p.e,
                    half_a: p.half_a,
                    half_e: p.half_e,
                    gain,
                };
                self.live_patches += 1;
            }
        }
    }

    fn rasterise(&mut self) {
        let zb = lower_border_km(self.params.e0);
        let pink = pink_fraction(self.params.e0);
        let view = &mut self.view;
        view.clear_layers();
        // The hem marks the lower border of the brightest curtain only. Poleward arcs
        // are farther away and so sit lower on screen, so "lowest border" alone would
        // pick a dim poleward arc.
        let amp = self.params.amp;
        let brightest = (0..CURTAINS)
            .max_by(|&i, &j| amp[i].total_cmp(&amp[j]))
            .unwrap_or(0);
        for (ci, c) in self.curtains.iter().enumerate() {
            view.splat(&c.samples, zb, if ci == brightest { pink } else { 0.0 });
        }
        view.pixels.clear();
        view.glow.fill(Glow::default());
        view.clear_emit();
        let patches = &self.patches[..self.live_patches];
        for x in 0..view.w {
            view.emit_column(x, &self.luts, patches);
        }
        view.diffuse_red();
        view.shade(&self.luts);
        for x in 0..view.w {
            view.skyglow_column(x);
        }
        if pink > 0.0 {
            view.trim_hem();
        }
        view.reflect(self.t);
    }

    /// The ASCII fallback for one cell. Sky cells: the pair's mean undithered glow,
    /// dithered at cell resolution, so ASCII density follows brightness as the half
    /// blocks do. Lake cells: the brighter reflected pixel.
    fn ascii_cell(&self, cx: i32, cy: i32, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let v = &self.view;
        let (x, y0) = (cx as usize, 2 * cy as usize);
        let ink = if y0 < v.y_h {
            let a = v.glow[y0 * v.w + x];
            let b = v.glow.get((y0 + 1) * v.w + x).filter(|_| y0 + 1 < v.y_h);
            let cell = b.map_or(a, |b| {
                let bright = if a.tone >= b.tone { a } else { *b };
                Glow {
                    tone: 0.5 * (a.tone + b.tone),
                    hem: a.hem || b.hem,
                    ..bright
                }
            });
            cell.dither(1.0, v.dither.at(x, y0, cell.soft))?
        } else {
            let ink = [top, bottom]
                .into_iter()
                .filter(|i| i.role != Role::Background)
                .max_by_key(|i| i.level)?;
            let ch = if ink.level >= 60 { '~' } else { '-' };
            return Some((ch, ink));
        };
        let ch = match ink.role {
            Role::Hot if ink.level >= 110 => '"',
            Role::Hot => '\'',
            Role::Accent => '=',
            // The faintest green is ',' so a quiet arc never reads as the dim stars '.'.
            _ => match ink.level {
                200.. => '|',
                130.. => '!',
                ASCII_COLON.. => ':',
                _ => ',',
            },
        };
        Some((ch, ink))
    }

    /// Stars in cells whose two pixels are empty, twinkling more toward the horizon.
    /// ASCII sky cells are dithered per cell, so a star also needs a still-blank cell
    /// (always true in Unicode mode when both pixels are empty).
    fn draw_stars(&self, canvas: &mut Canvas) {
        let v = &self.view;
        for s in &v.stars {
            let empty = |y: i32| {
                v.pixels
                    .get(s.cx, y)
                    .is_none_or(|i| i.role == Role::Background)
            };
            if !(empty(2 * s.cy) && empty(2 * s.cy + 1))
                || canvas.get(s.cx, s.cy).is_some_and(|c| c.ch != ' ')
            {
                continue;
            }
            let star = self.setting.stars[usize::from(s.idx)];
            let base = 175.0 - 16.0 * (star.mag + 1.0);
            let ext = f64::from(v.ext[(2 * s.cy) as usize]);
            let depth = 0.15 + 0.45 * (1.0 - star.e.sin());
            let twinkle = 1.0 - depth * (0.5 + 0.5 * (star.rate * self.t + star.phase).sin());
            let level = (base * ext * twinkle).clamp(25.0, 200.0) as u8;
            let glyph = if s.bright { STAR_BRIGHT } else { STAR_DIM };
            canvas.put(s.cx, s.cy, glyph, Ink::new(Role::Star, level));
        }
    }

    /// ASCII only, in still-blank cells: the bare ridge line, one `_` per column in the
    /// cell holding its top pixel (spire flanks do not lift it), with spire tips `^` in
    /// the cell holding the spire's top pixel when that lies above the line. A `_` sits
    /// at the foot of its cell, so the ridge (only 1–3 px tall at 80×24) reads as one
    /// continuous shoreline rather than dashes offset by a cell.
    fn draw_ridge_ascii(&self, canvas: &mut Canvas) {
        let v = &self.view;
        let cell = |top: i32| (top >= 0 && top < v.y_h as i32).then_some(top / 2);
        let mut put = |x: usize, cy: i32, ch: char| {
            if canvas.get(x as i32, cy).is_some_and(|c| c.ch == ' ') {
                canvas.put(x as i32, cy, Glyph::ascii(ch), RIDGE_INK);
            }
        };
        for x in 0..v.w {
            let base = cell(v.ridge_base[x]);
            if let Some(cy) = base {
                put(x, cy, '_');
            }
            if let Some(tip) = cell(v.ridge_top[x]).filter(|_| v.tree[x])
                && base.is_none_or(|b| tip < b)
            {
                put(x, tip, '^');
            }
        }
    }

    /// ` AL -620 nT  expansion `: AL rounded to 10 nT, then the beat's name, with a
    /// blank gutter cell on each side (CORE-R67) so the label never merges into the
    /// curtain.
    fn draw_label(&self, canvas: &mut Canvas) {
        if self.view.label == Label::None {
            return;
        }
        let al = (al_from(self.params.activity) / 10.0).round() as i32 * 10;
        let mut buf = StackText::<32>::new();
        let _ = write!(buf, "AL {al} nT");
        if self.view.label == Label::Full {
            let _ = write!(buf, "  {}", self.timeline.at(self.t).label);
        }
        draw::label(canvas, 0, 0, buf.as_str(), LABEL_INK);
    }
}

impl Generator for Aurora {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(&mut ctx.rng);
        }
        self.view = View::new(size, &self.setting);
        self.update_patches();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.t += dt;
        self.params = keyframes(self.t);
        let p = self.params;
        for c in &mut self.curtains {
            c.advance(&p, dt);
        }
        self.sample(dt);
        self.update_patches();
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.view.pixels.blit(canvas, |cx, cy, top, bottom| {
            self.ascii_cell(cx, cy, top, bottom)
        });
        self.draw_stars(canvas);
        if canvas.mode() == GlyphMode::Ascii {
            self.draw_ridge_ascii(canvas);
        }
        self.draw_label(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elevation_deg(z: f64, d: f64) -> f64 {
        elevation(z, d).to_degrees()
    }

    fn altitude_km(e_deg: f64, d: f64) -> f64 {
        d * e_deg.to_radians().tan() + drop_km(d)
    }

    /// Pink emission per unit flux: the hem band times the type-B fraction.
    fn pink_profile(z: f64, zb: f64, e0: f64) -> f64 {
        if in_hem_band((z - zb) as f32) {
            pink_fraction(e0)
        } else {
            0.0
        }
    }

    /// The ray field `ρ = ⅓ Σ_m max(0, cos(κ_m s + φ_m))⁶`.
    fn ray_field(s: f64, phases: &[f64; 3]) -> f64 {
        (0..3)
            .map(|m| ray_term(s, TAU / RAY_LAMBDA[m], phases[m]))
            .sum::<f64>()
            / 3.0
    }

    fn scene(w: u16, h: u16) -> (Aurora, Ctx) {
        let mut ctx = Ctx::new(7, Size::new(w, h));
        let mut a = Aurora::new();
        a.resize(Size::new(w, h), &mut ctx);
        (a, ctx)
    }

    #[test]
    fn aur_a1_lower_border_follows_rees_fit_and_is_clamped() {
        for (e0, zb) in [(5.0, 113.5), (10.0, 106.0), (20.0, 98.5), (50.0, 88.5)] {
            assert!((lower_border_km(e0) - zb).abs() < 0.1, "{e0} keV");
        }
        let mut prev = f64::INFINITY;
        for i in 0..=590 {
            let z = lower_border_km(1.0 + 0.1 * f64::from(i));
            assert!(z < prev, "not strictly decreasing at {i}");
            prev = z;
        }
        for e0 in [f64::NAN, -1.0, 0.0, 1e-9, 1e9, f64::INFINITY] {
            assert!((85.0..=150.0).contains(&lower_border_km(e0)), "{e0}");
        }
    }

    /// Argmax of `f` over z ∈ [lo, hi] km in 0.01 km steps.
    fn peak(f: impl Fn(f64) -> f64, lo: f64, hi: f64) -> (f64, f64) {
        (0..=((hi - lo) * 100.0) as usize)
            .map(|i| lo + 0.01 * i as f64)
            .map(|z| (z, f(z)))
            .fold((lo, f64::MIN), |a, b| if b.1 > a.1 { b } else { a })
    }

    #[test]
    fn aur_a2_emission_profiles_stratify_green_red_and_pink() {
        for e0 in [3.0, 10.0, 20.0] {
            let zb = lower_border_km(e0);
            let green = |z| green_profile(z, zb, H_GREEN);
            let (zp, gp) = peak(green, zb - 10.0, zb + 200.0);
            assert!((zb..=zb + 6.0).contains(&zp), "green peak {zp} for zb {zb}");
            assert!((gp - GREEN_PEAK).abs() <= 0.01, "green peak value {gp}");
            for k in 1..=5000 {
                let z = zb - 2.0 - 0.01 * f64::from(k);
                assert_eq!(green(z), 0.0, "green below the border at {z}");
            }
            for i in 0..400 {
                assert!(green(zb + 80.01 + f64::from(i)) < 0.05 * gp);
            }
            for i in 0..=250 {
                let z = 200.0 + f64::from(i);
                assert!(red_profile(z) > green(z), "red > green at {z}");
            }
            let mut z = zb;
            while z <= 150.0 {
                assert!(red_profile(z) < 0.05 * green(z), "red ≪ green at {z}");
                z += 0.25;
            }
            let pink = |z| pink_profile(z, zb, e0);
            assert_eq!(pink(zb - 4.01), 0.0);
            assert_eq!(pink(zb + 1.01), 0.0);
            for i in 0..=80 {
                let soft = 0.1 * f64::from(i);
                assert_eq!(pink_profile(zb, zb, soft), 0.0, "pink at {soft} keV");
            }
        }
        assert!(pink_profile(100.0, 100.0, 20.0) > 0.0);
        let (zp, rp) = peak(red_profile, 100.0, 600.0);
        assert!((220.0..=260.0).contains(&zp), "red peak {zp}");
        for i in 0..340 {
            let z = 0.5 * f64::from(i);
            assert!(red_profile(z) < 0.05 * rp, "red at {z} km");
        }
    }

    #[test]
    fn aur_a3_red_lags_and_rays_keep_the_mean_flux() {
        let one_tau = red_relax(0.0, 1.0, TAU_RED);
        assert!((one_tau - (1.0 - (-1.0f64).exp())).abs() < 1e-12);
        let split = red_relax(red_relax(0.3, 1.7, 0.1), 1.7, 0.2);
        assert!((split - red_relax(0.3, 1.7, 0.3)).abs() < 1e-12);
        let mut r = 0.0;
        for _ in 0..1000 {
            r = red_relax(r, 1.0, 0.25);
            assert!(r <= 1.0);
        }
        // The breakup: A at least doubles within 1.5 s; the red lags well behind.
        let a0 = keyframes(ONSET).amp[0];
        let a1 = keyframes(ONSET + 1.5).amp[0];
        assert!(a1 >= 2.0 * a0, "{a0} -> {a1}");
        let mut red = a0;
        for i in 1..=90 {
            red = red_relax(
                red,
                keyframes(ONSET + f64::from(i) / 60.0).amp[0],
                1.0 / 60.0,
            );
        }
        assert!(
            (red - a0) / (a1 - a0) < 0.45,
            "red covered {}",
            (red - a0) / (a1 - a0)
        );
        let phases = [0.3, 2.0, 4.1];
        let n = 200_000;
        let mean = (0..n)
            .map(|i| ray_field(1e4 * f64::from(i) / f64::from(n), &phases))
            .sum::<f64>()
            / f64::from(n);
        assert!((mean - 5.0 / 32.0).abs() < 0.005, "ray mean {mean}");

        // The production combination in `Curtain::sample` (mean-normalised families,
        // anti-alias weights `w·term + 1 − w`, averaged over 3): at every station, the
        // flux averaged over the ray phase is the flux with rays off.
        const PHASES: usize = 64;
        let st = Stations::new();
        let mut p = keyframes(START_T);
        p.bulge = 0.0;
        p.rays = 0.0;
        let mut flat = Curtain::new();
        flat.sample(&st, &p, 1.0, 300.0, [0.0; 4], f64::INFINITY);
        p.rays = 1.0;
        let mut sum = [0.0f64; M];
        for k in 0..PHASES {
            let off = TAU * k as f64 / PHASES as f64;
            let mut c = Curtain {
                ray_phase: [off, 0.37 + off, 1.9 + off],
                ..Curtain::new()
            };
            c.sample(&st, &p, 1.0, 300.0, [0.0; 4], f64::INFINITY);
            for (acc, (s, f)) in sum.iter_mut().zip(c.samples.iter().zip(&flat.samples)) {
                *acc += f64::from(s.green) / f64::from(f.green);
            }
        }
        let resolved = st
            .ray_w
            .iter()
            .filter(|w| w.iter().all(|&w| w == 1.0))
            .count();
        assert!(resolved > M / 2, "only {resolved} fully resolved stations");
        for (j, acc) in sum.iter().enumerate() {
            let mean = acc / PHASES as f64;
            assert!((mean - 1.0).abs() < 1e-3, "station {j}: mean flux {mean}");
        }
    }

    /// One curtain sampled at `d0` with explicit fold amplitudes and zero phases.
    fn lone_curtain(folds: [f64; 4]) -> (Curtain, Params) {
        lone_curtain_at(folds, 0.0)
    }

    /// The same with the fold phase `θ₂` (the 70 km fold) set to `theta2`.
    fn lone_curtain_at(folds: [f64; 4], theta2: f64) -> (Curtain, Params) {
        let mut p = keyframes(START_T);
        p.bulge = 0.0;
        p.rays = 0.0;
        let mut c = Curtain {
            theta: [0.0, 0.0, theta2, 0.0],
            ..Curtain::new()
        };
        c.sample(&Stations::new(), &p, 1.0, 300.0, folds, f64::INFINITY);
        (c, p)
    }

    #[test]
    fn aur_a4_geometry_round_trips_and_a_straight_arc_is_an_arch() {
        assert!((elevation_deg(105.0, 600.0) - 7.29).abs() < 0.05);
        assert!((elevation_deg(105.0, 180.0) - 29.65).abs() < 0.05);
        for (z, d) in [(105.0, 600.0), (240.0, 90.0), (90.0, 1500.0)] {
            assert!((altitude_km(elevation_deg(z, d), d) - z).abs() < 1e-9);
        }
        let (c, p) = lone_curtain([0.0; 4]);
        let zb = lower_border_km(p.e0);
        let eb: Vec<(f64, f64)> = c
            .samples
            .iter()
            .map(|s| (f64::from(s.alpha), elevation_deg(zb, f64::from(s.d))))
            .collect();
        let mid = M / 2;
        // Stations are symmetric about α = 0: the crest is the middle pair.
        // Stations run from α = −85° (west) to +85° (east).
        for w in eb[..mid].windows(2) {
            assert!(w[1].1 > w[0].1, "west flank rises toward α = 0");
        }
        for w in eb[mid..].windows(2) {
            assert!(w[1].1 < w[0].1, "east flank falls away from α = 0");
        }
        assert!((eb[mid - 1].1 - eb[mid].1).abs() < 1e-9);
    }

    /// Layer counts per column of the 200×50 view for one curtain.
    fn layer_counts(c: &Curtain, p: &Params) -> (View, Vec<u8>) {
        let (a, _) = scene(200, 50);
        let mut v = a.view;
        v.clear_layers();
        v.splat(&c.samples, lower_border_km(p.e0), 0.0);
        let n = v.nlayers.clone();
        (v, n)
    }

    #[test]
    fn aur_a5_folds_double_back_only_in_the_oblique_sky() {
        let (c, p) = lone_curtain([0.0; 4]);
        let (v, n) = layer_counts(&c, &p);
        let zb = lower_border_km(p.e0);
        let mut above = 0;
        for (x, &count) in n.iter().enumerate() {
            let a = v.alpha_at(x as f64 + 0.5);
            // Straight arc at 300 km: the lower border is above the horizon iff z_b
            // exceeds the curvature drop at d = 300 / cos α.
            if a.abs() < ALPHA_MAX && zb > drop_km(300.0 / a.cos()) {
                assert_eq!(count, 1, "column {x} (α {:.1}°)", a.to_degrees());
                above += 1;
            }
        }
        assert!(above > 150, "{above} columns under the arch");

        // The no-fold bound |α| < 37.6° holds for every fold phase.
        let fold = 1.3 * FOLD_LAMBDA[2] / TAU;
        let mut doubled_oblique = false;
        for theta2 in [0.0, 1.1, 2.7, 4.4] {
            let (c, p) = lone_curtain_at([0.0, 0.0, fold, 0.0], theta2);
            let (v, n) = layer_counts(&c, &p);
            for (x, &count) in n.iter().enumerate() {
                let a = v.alpha_at(x as f64 + 0.5).abs().to_degrees();
                if a < 37.0 {
                    assert!(
                        count <= 1,
                        "θ₂ {theta2}: column {x} at {a:.1}° has {count} layers"
                    );
                }
                doubled_oblique |= a > 38.0 && count >= 2;
            }
        }
        assert!(doubled_oblique, "no fold doubles back beyond 38°");

        // With unit flux (no rays, no bulge) a sample's green amplitude is S itself.
        for folds in [[0.0; 4], [BEND_KM, 19.0, fold, 3.0]] {
            let (c, _) = lone_curtain(folds);
            for s in &c.samples {
                let k = f64::from(s.green);
                assert!((1.0 - 1e-6..=SLANT_MAX + 1e-6).contains(&k), "S = {k}");
            }
        }
        for (dxi, s, dd, h) in [
            (0.0, 0.0, 300.0, 100.0),
            (3.0, 500.0, 200.0, 80.0),
            (1e9, 1.0, 1.0, 1.0),
        ] {
            let k = slant_factor(dxi, s, dd, h);
            assert!((1.0..=SLANT_MAX).contains(&k), "{k}");
        }
        let (dd, h) = (300.0f64, 100.0);
        let eb = (h / dd).atan();
        assert!((slant_factor(0.0, 0.0, dd, h) - 1.0 / eb.cos()).abs() < 1e-9);
    }

    #[test]
    fn aur_a6_substorm_timeline_and_keyframes_loop_seamlessly() {
        let tl = Timeline::new(&BEATS);
        assert_eq!(tl.total(), LOOP);
        let names: Vec<_> = [0.0, 12.0, 15.0, 30.0].map(|t| tl.at(t).label).to_vec();
        assert_eq!(names, ["growth", "onset", "expansion", "recovery"]);
        assert_eq!(Aurora::new().t, 8.0);
        let (a, b) = (keyframes(LOOP - 1e-12), keyframes(LOOP));
        let fields = |p: Params| {
            [
                p.d0[0],
                p.d0[1],
                p.d0[2],
                p.amp[0],
                p.amp[1],
                p.amp[2],
                p.e0,
                p.fold,
                p.rays,
                p.v_ray,
                p.v_surge,
                p.bulge,
                p.bulge_alpha,
                p.bulge_width,
                p.patches,
                p.activity,
            ]
        };
        for (x, y) in fields(a).iter().zip(fields(b)) {
            assert!((x - y).abs() < 1e-9, "{x} vs {y} across the wrap");
        }
        // Every knot table spans the loop in increasing time.
        for knots in [
            K_D0,
            K_OFF2,
            K_OFF3,
            K_A1,
            K_A2,
            K_A3,
            K_E0,
            K_FOLD,
            K_RAYS,
            K_VRAY,
            K_VSURGE,
            K_BULGE,
            K_BULGE_ALPHA,
            K_BULGE_WIDTH,
            K_PATCHES,
            K_ACTIVITY,
        ] {
            assert_eq!(knots[0].0, 0.0);
            assert_eq!(knots[knots.len() - 1].0, LOOP);
            assert!(knots.windows(2).all(|w| w[0].0 < w[1].0));
            assert!(knots.iter().all(|k| k.0 <= ONSET || k.0 >= BREAKUP));
        }
        for i in 0..=5000 {
            let u = LOOP * f64::from(i) / 5000.0;
            let al = al_nt(u);
            if u < ONSET {
                assert!((-150.0..=-30.0).contains(&al), "growth AL {al} at {u}");
            } else if (15.0..RECOVERY).contains(&u) {
                assert!(al <= -450.0, "expansion AL {al} at {u}");
            }
            if u < RECOVERY {
                assert_eq!(keyframes(u).patches, 0.0, "patch outside recovery at {u}");
            }
        }
        for period in [4.0, 7.3, 12.0] {
            let n = 100_000;
            let on = (0..n)
                .filter(|&i| patch_pulse(period * f64::from(i) / f64::from(n), period, 0.37) > 0.5)
                .count();
            let frac = on as f64 / f64::from(n);
            assert!((frac - 0.4).abs() < 0.02, "on-fraction {frac}");
            for t in [0.0, 1.3, 5.9] {
                let d = patch_pulse(t, period, 0.2) - patch_pulse(t + period, period, 0.2);
                assert!(d.abs() < 1e-12);
            }
        }
    }

    /// Pixel census of the 200×50 frame after `steps` updates of 0.25 s.
    fn census(steps: usize) -> (Aurora, Canvas) {
        let (mut a, mut ctx) = scene(200, 50);
        for _ in 0..steps {
            a.update(0.25, &mut ctx);
        }
        let mut canvas = Canvas::new(Size::new(200, 50), GlyphMode::Unicode);
        a.render(&mut canvas);
        (a, canvas)
    }

    const EMISSION: [Role; 3] = [Role::Particle, Role::Hot, Role::Accent];

    #[test]
    fn aur_a7_expansion_frame_respects_silhouette_lake_stars_and_hem() {
        let (a, canvas) = census(56);
        assert!((a.t - 22.0).abs() < 1e-9);
        let v = &a.view;
        let px = |x: usize, y: usize| v.pixels.get(x as i32, y as i32).unwrap_or(Ink::BLANK);
        let (mut sky, mut lit, mut sky_max, mut accent) = (0, 0, 0u8, 0);
        for x in 0..v.w {
            for y in 0..v.y_h {
                let ink = px(x, y);
                if y as i32 >= v.ridge_top[x] {
                    assert_eq!(ink.role, Role::Background, "silhouette lit at {x},{y}");
                    continue;
                }
                sky += 1;
                if EMISSION.contains(&ink.role) {
                    lit += 1;
                    sky_max = sky_max.max(ink.level);
                }
                accent += usize::from(ink.role == Role::Accent);
            }
        }
        let frac = f64::from(lit) / f64::from(sky);
        assert!((0.15..=0.70).contains(&frac), "lit fraction {frac:.3}");
        assert!(accent > 0, "no pink hem at the expansion peak");
        for x in 0..v.w {
            for y in v.y_h..v.rows {
                let ink = px(x, y);
                if ink.role != Role::Background {
                    assert!(
                        f64::from(ink.level) <= 0.5 * f64::from(sky_max),
                        "lake {x},{y}"
                    );
                }
            }
        }
        let mut stars = 0;
        for (i, c) in canvas.cells().iter().enumerate() {
            if c.ink.role == Role::Star {
                let (x, y) = (i % v.w, i / v.w);
                assert_eq!(px(x, 2 * y).role, Role::Background);
                assert_eq!(px(x, 2 * y + 1).role, Role::Background);
                stars += 1;
            }
        }
        assert!(stars > 0, "no stars shown");

        let (a, _) = census(4);
        let v = &a.view;
        let hem = (0..v.w).any(|x| {
            (0..v.rows).any(|y| {
                v.pixels
                    .get(x as i32, y as i32)
                    .is_some_and(|i| i.role == Role::Accent)
            })
        });
        assert!(!hem, "pink hem during growth");
    }

    /// Every sky threshold of the view's `soft` or strand dither, row by row.
    fn thresholds(v: &View, soft: bool) -> Vec<f32> {
        (0..v.rows)
            .flat_map(|y| (0..v.w).map(move |x| v.dither.at(x, y, soft)))
            .collect()
    }

    /// Variance of the lit count over `cols` × `groups` windows of `spec`'s cells at lit
    /// share `p`, relative to the binomial variance of independent cells.
    fn window_variance(v: &View, spec: TileSpec, (cols, groups): (usize, usize), p: f32) -> f64 {
        let soft = spec.px == 1;
        let mut counts = Vec::new();
        for g0 in (0..v.rows / spec.px - groups).step_by(groups) {
            for x0 in (0..v.w - cols).step_by(cols) {
                let lit = (0..groups)
                    .flat_map(|g| (0..cols).map(move |x| (x0 + x, (g0 + g) * spec.px)))
                    .filter(|&(x, y)| v.dither.at(x, y, soft) < p)
                    .count();
                counts.push(lit as f64);
            }
        }
        let m = mean_of(&counts);
        let var = counts.iter().map(|c| (c - m) * (c - m)).sum::<f64>() / counts.len() as f64;
        let n = (cols * groups) as f64;
        var / (n * f64::from(p) * (1.0 - f64::from(p)))
    }

    fn mean_of(xs: &[f64]) -> f64 {
        xs.iter().sum::<f64>() / xs.len() as f64
    }

    fn glow(tone: f32, red: f32) -> Glow {
        Glow {
            tone,
            red,
            hem: false,
            soft: false,
        }
    }

    #[test]
    fn aur_dither_breaks_into_even_strands_and_red_stipples_only_the_transition_band() {
        let (a, _) = scene(200, 50);
        let v = &a.view;
        for (spec, window, tile) in [
            (STRANDS, (8, 2), &tiles()[0]),
            (SPECKLE, (4, 4), &tiles()[1]),
        ] {
            let soft = spec.px == 1;
            let t = |x: usize, y: usize| v.dither.at(x, y, soft);
            assert!(thresholds(v, soft).iter().all(|b| (0.0..1.0).contains(b)));
            // Field-aligned strands: a threshold is shared by STRAND_PX rows of a column,
            // so partial coverage reads as short vertical strands, never as speckle.
            for y in 0..v.rows {
                for x in 0..v.w {
                    if y % spec.px != 0 {
                        assert_eq!(t(x, y), t(x, y - 1), "strand broken at {x},{y}");
                    }
                }
            }
            // The tile ranks every cell once: thresholds are uniform on [0, 1).
            let n = spec.w * spec.g;
            assert_eq!(tile.len(), n);
            let mut ranks: Vec<usize> = tile
                .iter()
                .map(|&b| (b * n as f32 - 0.5).round() as usize)
                .collect();
            ranks.sort_unstable();
            assert!(ranks.iter().enumerate().all(|(i, &r)| i == r));
            // Even coverage: blue noise clumps and leaves holes far less than independent
            // (white-noise) cells, at every faint share.
            for p in [0.1, 0.2, 0.35, 0.5] {
                let rel = window_variance(v, spec, window, p);
                assert!(
                    rel < 0.4,
                    "{soft}: variance {rel:.2}× binomial at share {p}"
                );
            }
            // No ordered lattice: neither a 4-px Bayer period nor any short offset repeats
            // the pattern (an ordered dither agrees ≈ 100 % under its period).
            for (dx, dg) in [
                (4, 0),
                (1, 1),
                (2, 1),
                (-1, 1),
                (8, 3),
                (4, 2),
                (0, 2),
                (0, 4),
            ] {
                let (mut same, mut n) = (0usize, 0usize);
                for g in 0..v.rows / spec.px - 4 {
                    for x in 8..v.w - 8 {
                        let x2 = (x as i32 + dx) as usize;
                        let (y, y2) = (g * spec.px, (g + dg) * spec.px);
                        same += usize::from((t(x, y) < 0.25) == (t(x2, y2) < 0.25));
                        n += 1;
                    }
                }
                let agree = same as f64 / n as f64;
                assert!(
                    agree < 0.8,
                    "{soft}: offset ({dx}, {dg}) agreement {agree:.3}"
                );
            }
        }
        let thresh = thresholds(v, false);
        // A uniform field: lit share ≈ tone/L for faint tones. Red stipples only the
        // transition band R/(G+R) ∈ (0.3, 0.7): green cores stay pure green (no orange
        // specks in the brightest rays) and the upper fringe pure red.
        for (tone, red, expect_red) in [
            (0.06f32, 0.2f32, Some(0.0)),
            (0.9, 0.29, Some(0.0)),
            (0.15, 0.5, None),
            (0.24, 0.8, Some(1.0)),
            (0.9, 0.71, Some(1.0)),
        ] {
            let g = glow(tone, red);
            let (mut lit, mut hot) = (0usize, 0usize);
            for &b in &thresh {
                if let Some(ink) = g.dither(1.0, b) {
                    lit += 1;
                    hot += usize::from(ink.role == Role::Hot);
                }
            }
            if tone < LIT_THRESHOLD {
                let share = lit as f64 / thresh.len() as f64;
                let expect = f64::from(tone / LIT_THRESHOLD);
                assert!(
                    (share - expect).abs() < 0.02,
                    "lit {share:.3} vs {expect:.3}"
                );
            }
            let red_share = hot as f64 / lit as f64;
            match expect_red {
                Some(e) => assert_eq!(red_share, e, "red share at R/(G+R) = {red}"),
                // Mid-band: an even stipple.
                None => assert!((red_share - 0.5).abs() < 0.04, "red {red_share:.3}"),
            }
        }
        let haze = glow(0.9 * TONE_FLOOR, 1.0);
        assert!(thresh.iter().all(|&b| haze.dither(1.0, b).is_none()));
        // The red fringe's sparse tail drops out (lit < 30 % of the time), so it keeps
        // an envelope over the curtain tops instead of scattering as confetti; faint
        // green at the same tone still stipples.
        let red_haze = glow(0.95 * RED_TONE_FLOOR, 0.8);
        assert!(thresh.iter().all(|&b| red_haze.dither(1.0, b).is_none()));
        let green_haze = glow(0.95 * RED_TONE_FLOOR, 0.2);
        assert!(thresh.iter().any(|&b| green_haze.dither(1.0, b).is_some()));
        // Just above the floor, red lights ≈ 30 % of pixels.
        let edge = glow(1.02 * RED_TONE_FLOOR, 0.8);
        let lit = thresh
            .iter()
            .filter(|&&b| edge.dither(1.0, b).is_some())
            .count();
        let share = lit as f64 / thresh.len() as f64;
        assert!(
            (share - 0.3).abs() < 0.03,
            "red lit share {share:.3} at the floor"
        );
    }

    #[test]
    fn aur_a7_lake_mirrors_the_lit_sky_as_broken_streaks() {
        let (a, _) = census(56);
        let v = &a.view;
        let px = |x: usize, y: usize| v.pixels.get(x as i32, y as i32).unwrap_or(Ink::BLANK);
        let (mut lake_lit, mut sky_lit, mut rows) = (0usize, 0usize, 0);
        for y in v.y_h..v.rows {
            let d = (y - v.y_h + 1) as f64;
            let src = v.y_h - (y - v.y_h) - 1;
            if (0.9 * d - 1.7 * a.t).sin() > 0.9 {
                // A ripple crest: the whole row drops out.
                assert!((0..v.w).all(|x| px(x, y) == Ink::BLANK), "row {y}");
                continue;
            }
            rows += 1;
            for x in 0..v.w {
                sky_lit += usize::from(EMISSION.contains(&px(x, src).role));
                let ink = px(x, y);
                if ink == Ink::BLANK {
                    continue;
                }
                lake_lit += 1;
                // Each lit lake pixel is a dimmed copy of a lit sky pixel a ripple
                // shift away: same role (so red over green reflects as red over green).
                let copy = (x.saturating_sub(3)..(x + 4).min(v.w)).any(|sx| {
                    let s = px(sx, src);
                    EMISSION.contains(&s.role)
                        && s.role == ink.role
                        && ink.level == (LAKE_GAIN * f32::from(s.level)).round() as u8
                });
                assert!(copy, "lake pixel {x},{y} {ink:?} mirrors nothing");
            }
        }
        assert!(rows > 0);
        // Lit-ness follows the sky, so the reflection is as dense as its source rows.
        let ratio = lake_lit as f64 / sky_lit as f64;
        assert!((0.9..=1.1).contains(&ratio), "lake/sky lit {ratio:.3}");
    }

    #[test]
    fn aur_a7_hem_runs_only_along_the_brightest_curtains_lowest_border() {
        for steps in [44, 52, 56] {
            let (a, _) = census(steps);
            let v = &a.view;
            let accent =
                |x: i32, y: i32| v.pixels.get(x, y).is_some_and(|i| i.role == Role::Accent);
            let mut hems = 0;
            for x in 0..v.w {
                // Pink is splatted for the brightest curtain only.
                let layers = &v.layers[x][..usize::from(v.nlayers[x])];
                let lowest = layers
                    .iter()
                    .filter(|l| l.pink > 0.0)
                    .map(|l| l.y_low)
                    .max()
                    .unwrap_or(-1);
                for y in 0..v.y_h as i32 {
                    if !accent(x as i32, y) {
                        continue;
                    }
                    hems += 1;
                    // Within the lowest layer's 5 km band, never inside a higher layer.
                    assert!(
                        y >= lowest - 4 && y <= lowest,
                        "t = {}: hem at {x},{y}, lowest border {lowest}",
                        a.t
                    );
                    // No lone confetti pixels: a hem runs at least 2 px sideways.
                    let run = [-1, 1]
                        .iter()
                        .any(|dx| (-1..=1).any(|dy| accent(x as i32 + dx, y + dy)));
                    assert!(run, "t = {}: isolated hem pixel at {x},{y}", a.t);
                    // ASCII reads the same flag.
                    assert!(v.glow[y as usize * v.w + x].hem);
                }
            }
            assert!(hems > 0, "no hem at t = {}", a.t);
        }
    }

    #[test]
    fn aur_a4_pixel_rows_sample_the_altitude_of_their_centre() {
        for (w, h) in [(200u16, 50u16), (80, 24), (61, 17)] {
            let (a, _) = scene(w, h);
            let v = &a.view;
            for d in [150.0, 420.0, 1300.0] {
                let l = v.layer(d, 105.0, 1.0, 1.0, 0.0, 0);
                for y in 0..v.y_h {
                    // Pixel-centre elevation, computed independently of the tables.
                    let e = (v.y_h as f64 - y as f64 - 0.5) / v.f;
                    let z = f64::from(l.altitude(v.tan_e[y]));
                    let want = altitude_km(e.to_degrees(), d);
                    assert!(
                        (z - want).abs() < 0.5,
                        "{w}x{h} d {d} row {y}: {z} vs {want}"
                    );
                }
                // The layer's lowest row is the last one whose centre is at or above
                // the bottom of the hem band.
                let low = l.y_low;
                let z_hem = 105.0 - HEM_BELOW;
                if (0..v.y_h as i32 - 1).contains(&low) {
                    let at = |y: i32| f64::from(l.altitude(v.tan_e[y as usize]));
                    assert!(at(low) >= z_hem - 0.01, "{w}x{h} d {d}");
                    assert!(at(low + 1) < z_hem + 0.01, "{w}x{h} d {d}");
                }
            }
        }
    }

    #[test]
    fn aur_skyglow_is_solid_and_fades_to_black_above_the_silhouette() {
        assert_eq!(skyglow(9.0, 9.0), None);
        assert_eq!(skyglow(0.0, 9.0), Some((ink::seq(GLOW_MAX), 1.0)));
        let horizon = skyglow(0.0, 9.0).map_or(0.0, |(ink, _)| ink::seq_value(ink));
        assert!(horizon >= 0.135, "a band, not a ragged stripe: {horizon}");
        let (mut prev, mut prev_share) = (1.0, 1.0);
        for i in 0..90 {
            let px = 0.1 * f64::from(i);
            let (ink, share) = skyglow(px, 9.0).unwrap_or((Ink::BLANK, 0.0));
            let v = ink::seq_value(ink);
            assert!(
                v <= prev && share <= prev_share,
                "skyglow brightens with elevation at {px}"
            );
            // Solid over the lower 80 %, where the ridge and spires stand; only the
            // top edge thins by density.
            assert!(px > 9.0 * 0.8 - 1e-9 || share == 1.0, "thinned at {px}");
            (prev, prev_share) = (v, share);
        }
        for (w, h) in [(200u16, 50u16), (80, 24), (40, 12)] {
            let (a, _) = scene(w, h);
            let v = &a.view;
            let mut glowing = 0;
            for x in 0..v.w {
                for y in 0..v.ridge_top[x] {
                    let ink = v.pixels.get(x as i32, y).unwrap_or(Ink::BLANK);
                    if EMISSION.contains(&ink.role) {
                        continue;
                    }
                    // Every dark sky pixel in the solid glow glows: no dither holes.
                    let (glow, share) = v.skyglow[y as usize].unwrap_or((Ink::BLANK, 0.0));
                    if share == 1.0 {
                        assert_eq!(ink, glow, "{w}x{h} sky pixel {x},{y}");
                    } else {
                        assert!(
                            ink == glow || ink == Ink::BLANK,
                            "{w}x{h} sky pixel {x},{y}"
                        );
                    }
                    glowing += usize::from(ink.role == Role::Seq);
                }
            }
            assert!(glowing >= v.w / 2, "{w}x{h}: only {glowing} glow pixels");
        }
    }

    #[test]
    fn aur_spires_grow_into_trees_at_large_sizes() {
        // 80×24 keeps the far forest; 200×50 shows the near one.
        let (small, _) = scene(80, 24);
        assert!(small.view.f <= BIG_TREE_F);
        let (big, _) = scene(200, 50);
        let v = &big.view;
        assert!(v.f > BIG_TREE_F);
        for (trees, forest) in [
            (&big.setting.trees, FAR_FOREST),
            (&big.setting.big_trees, NEAR_FOREST),
        ] {
            for t in trees.iter() {
                let h = t.height.to_degrees();
                assert!((forest.height.0..forest.height.1).contains(&h), "{h}°");
                assert!((t.half.to_degrees() - forest.half * h).abs() < 1e-9);
            }
            for w in trees.windows(2) {
                let gap = (w[1].alpha - w[0].alpha).to_degrees();
                assert!((forest.gap.0 - 1e-9..forest.gap.1).contains(&gap), "{gap}°");
            }
        }
        // Every spire stays inside the skyglow, so it reads as a black edge.
        let glow_px = v.f * GLOW_DEG.to_radians();
        // Each spire rises ≥ 3 px on average and has flanks: both neighbouring columns
        // stand above the bare ridge, no higher than the tip.
        let tips: Vec<usize> = (1..v.w - 1).filter(|&x| v.tree[x]).collect();
        assert!(tips.len() >= 20, "only {} spires", tips.len());
        let mut rise = 0;
        for &x in &tips {
            rise += v.ridge_base[x] - v.ridge_top[x];
            assert!(f64::from(v.y_h as i32 - v.ridge_top[x]) <= glow_px + 1.0);
            for n in [x - 1, x + 1] {
                assert!(
                    v.ridge_top[n] < v.ridge_base[n] && v.ridge_top[n] >= v.ridge_top[x],
                    "spire at {x} has no flank at {n}"
                );
            }
        }
        let mean = f64::from(rise) / tips.len() as f64;
        assert!(mean >= 3.0, "spires rise only {mean:.1} px");
    }

    #[test]
    fn aur_crown_fades_below_the_top_edge_and_stars_keep_to_blank_cells() {
        let (mut a, mut ctx) = scene(200, 50);
        for _ in 0..56 {
            a.update(0.25, &mut ctx);
        }
        let v = &a.view;
        let lit = |y: i32| {
            (0..v.w)
                .filter(|&x| {
                    v.pixels
                        .get(x as i32, y)
                        .is_some_and(|i| EMISSION.contains(&i.role))
                })
                .count()
        };
        for y in 0..4 {
            assert_eq!(lit(y), 0, "emission in top pixel row {y}");
        }
        assert!(
            lit(8) < lit(16),
            "no fade over the top rows: {} vs {}",
            lit(8),
            lit(16)
        );
        // The fade scales with the view (20 px of 100 here, ≈ 10 cell rows; never under
        // 3 px) and eases in as a smoothstep, so the crown thins into the dark over
        // rows instead of meeting a lid.
        assert_eq!(v.top.len(), v.y_h);
        assert!((v.top_fade_px() - 20.0).abs() < 1e-9);
        let (small, _) = scene(40, 12);
        assert!((small.view.top_fade_px() - 4.8).abs() < 1e-9);
        let (tiny, _) = scene(20, 6);
        assert!((tiny.view.top_fade_px() - 3.0).abs() < 1e-9);
        assert_eq!(v.top[0], 0.0);
        assert!(v.top.windows(2).all(|w| w[1] >= w[0]));
        assert!(
            (v.top[10] - 0.5).abs() < 0.05,
            "half-way down the fade {}",
            v.top[10]
        );
        assert!(v.top[20..].iter().all(|&k| k == 1.0));
        for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
            let mut canvas = Canvas::new(Size::new(200, 50), mode);
            a.view
                .pixels
                .blit(&mut canvas, |cx, cy, t, b| a.ascii_cell(cx, cy, t, b));
            let before = canvas.to_lines();
            a.draw_stars(&mut canvas);
            for (y, (old, new)) in before.iter().zip(canvas.to_lines()).enumerate() {
                for (x, (o, n)) in old.chars().zip(new.chars()).enumerate() {
                    assert!(o == n || o == ' ', "{mode:?} star over '{o}' at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn aur_label_is_rounded_and_fits_the_width() {
        let (a, _) = scene(80, 24);
        let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Ascii);
        a.render(&mut canvas);
        let top = &canvas.to_lines()[0];
        assert!(top.starts_with(" AL -"), "{top}");
        assert!(top.contains(" nT  growth "), "{top}");
        // The expansion peak: a blank gutter cell right after the label.
        let (mut a, mut ctx) = scene(80, 24);
        for _ in 0..56 {
            a.update(0.25, &mut ctx);
        }
        let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Ascii);
        a.render(&mut canvas);
        let top = &canvas.to_lines()[0];
        let end = top.find("expansion").map(|i| i + "expansion".len());
        assert_eq!(
            end.and_then(|e| top.as_bytes().get(e)),
            Some(&b' '),
            "{top}"
        );
        let (a, _) = scene(40, 24);
        let mut canvas = Canvas::new(Size::new(40, 24), GlyphMode::Ascii);
        a.render(&mut canvas);
        assert!(!canvas.to_lines()[0].contains("growth"));
        let (a, _) = scene(20, 24);
        let mut canvas = Canvas::new(Size::new(20, 24), GlyphMode::Ascii);
        a.render(&mut canvas);
        assert!(!canvas.to_lines()[0].contains("AL"));
    }

    /// The view's sky and lake pixels, row by row: `Some(true)` red, `Some(false)`
    /// another emission, `None` dark.
    fn red_mask(v: &View) -> Vec<Option<bool>> {
        (0..v.rows)
            .flat_map(|y| (0..v.w).map(move |x| (x, y)))
            .map(|(x, y)| {
                v.pixels
                    .get(x as i32, y as i32)
                    .filter(|i| EMISSION.contains(&i.role))
                    .map(|i| i.role == Role::Hot)
            })
            .collect()
    }

    /// Share of pixels matching `kind` in columns `xs` × rows `ys`.
    fn cover(
        v: &View,
        mask: &[Option<bool>],
        kind: Option<bool>,
        xs: std::ops::Range<usize>,
        ys: std::ops::Range<usize>,
    ) -> f64 {
        let n = xs.len() * ys.len();
        let hits = ys
            .flat_map(|y| xs.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| mask[y * v.w + x] == kind)
            .count();
        hits as f64 / n as f64
    }

    #[test]
    fn aur_red_crown_is_a_soft_dome_not_a_box() {
        // The substorm peak, visit t = 14–22 s (scene t = 22–30 s), at 200×50.
        let (mut a, mut ctx) = scene(200, 50);
        for visit in [14.0, 18.0, 22.0] {
            while a.t < START_T + visit - 1e-9 {
                a.update(1.0 / 60.0, &mut ctx);
            }
            let v = &a.view;
            let mask = red_mask(v);
            let red = |xs, ys| cover(v, &mask, Some(true), xs, ys);
            if visit == 18.0 {
                // Top edge per column: the first row whose 5 × 4 px neighbourhood is
                // ≥ 30 % red. It follows the curtains' red envelope, not a lid.
                let tops: Vec<usize> = (2..v.w - 3)
                    .filter_map(|x| (0..v.y_h / 2).find(|&y| red(x - 2..x + 3, y..y + 4) >= 0.3))
                    .collect();
                assert!(
                    tops.len() > v.w / 3,
                    "crown only {} columns wide",
                    tops.len()
                );
                let (lo, hi) = (tops.iter().min().copied(), tops.iter().max().copied());
                assert!(
                    hi.zip(lo).is_some_and(|(h, l)| h - l >= 3),
                    "flat top {lo:?}..{hi:?}"
                );
            }
            // No walls against the dark: across any column boundary, the red coverage of
            // the 4 px on either side never differs by more than half over more than 5
            // consecutive rows where the fainter side is dark sky (a green curtain's
            // edge next to the red fringe is a hue change, not a wall; a sloping flank
            // moves on within a few rows).
            for x in 4..v.w - 4 {
                let mut run = 0;
                for y in 0..v.y_h {
                    let (l, r) = (red(x - 4..x, y..y + 1), red(x..x + 4, y..y + 1));
                    let dim = if l < r { x - 4..x } else { x..x + 4 };
                    let dark = cover(v, &mask, None, dim, y..y + 1) >= 0.75;
                    run = if (l - r).abs() > 0.5 && dark {
                        run + 1
                    } else {
                        0
                    };
                    assert!(run <= 5, "t = {visit}: red wall at x = {x}, rows ..={y}");
                }
            }
        }
    }

    #[test]
    fn aur_ascii_arc_is_not_starlight_and_the_shore_is_one_line() {
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let (a, _) = scene(w, h);
            let mut canvas = Canvas::new(Size::new(w, h), GlyphMode::Ascii);
            a.render(&mut canvas);
            let lines = canvas.to_lines();
            // The quiet growth arc: faint green is ',' (':' brighter), never the dim
            // stars' '.'; stars keep '.' and '*'.
            let mut commas = 0;
            for (i, c) in canvas.cells().iter().enumerate() {
                let (cx, cy) = (i % usize::from(w), i / usize::from(w));
                if 2 * cy + 1 >= a.view.y_h {
                    continue;
                }
                let ch = lines[cy].chars().nth(cx);
                match c.ink.role {
                    Role::Particle => {
                        assert!(matches!(ch, Some(',' | ':' | '!' | '|')), "{ch:?}");
                        commas += usize::from(ch == Some(','));
                    }
                    Role::Star => assert!(matches!(ch, Some('.' | '*')), "{ch:?}"),
                    _ => {}
                }
            }
            assert!(commas > 10, "{w}x{h}: only {commas} ',' in the arc");
            // One '_' per column at most, nearly every column has one, and the line
            // seldom changes row: a shoreline, not offset dashes.
            let rows: Vec<Option<usize>> = (0..usize::from(w))
                .map(|x| {
                    let hits: Vec<usize> = (0..usize::from(h))
                        .filter(|&y| lines[y].chars().nth(x) == Some('_'))
                        .collect();
                    assert!(hits.len() <= 1, "{w}x{h}: column {x} has {hits:?}");
                    hits.first().copied()
                })
                .collect();
            let drawn: Vec<usize> = rows.iter().flatten().copied().collect();
            assert!(
                drawn.len() * 10 >= 9 * usize::from(w),
                "{w}x{h}: gaps in the shore"
            );
            let steps = drawn.windows(2).filter(|p| p[0] != p[1]).count();
            assert!(
                steps * 20 <= usize::from(w),
                "{w}x{h}: shore steps {steps} times"
            );
        }
    }

    #[test]
    fn aur_diffuse_light_is_dithered_without_strands() {
        // Recovery (visit t = 32 s, scene t = 40 s): pulsating patches and the red fringe.
        let (mut a, mut ctx) = scene(200, 50);
        while a.t < START_T + 32.0 - 1e-9 {
            a.update(1.0 / 60.0, &mut ctx);
        }
        assert!(a.live_patches > 0, "no patch alive");
        let v = &a.view;
        let (mut soft, mut strand) = (0, 0);
        for y in 0..v.y_h {
            for x in 0..v.w {
                let g = v.glow[y * v.w + x];
                if g.tone == 0.0 || y as i32 >= v.ridge_top[x] {
                    continue;
                }
                let e = v.emit[y * v.w + x];
                let patchy = e.patch > 0.5 * (e.green + e.red + e.pink);
                let reddish = red_chance(g.red) > 0.5;
                // Red-dominant or patch-dominant light takes the square blue noise,
                // curtain green the field-aligned strands.
                assert_eq!(g.soft, patchy || reddish, "{x},{y}");
                let lit = v
                    .pixels
                    .get(x as i32, y as i32)
                    .is_some_and(|i| EMISSION.contains(&i.role));
                let want = g.dither(1.0, v.dither.at(x, y, g.soft)).is_some();
                assert_eq!(lit, want, "{x},{y}");
                soft += usize::from(g.soft && patchy);
                strand += usize::from(!g.soft);
            }
        }
        assert!(
            soft > 50 && strand > 50,
            "{soft} patch / {strand} strand pixels"
        );
    }

    #[test]
    fn aur_resize_keeps_the_world_and_reframes_the_view() {
        let (mut a, mut ctx) = scene(200, 50);
        for _ in 0..30 {
            a.update(0.1, &mut ctx);
        }
        let theta = a.curtains[0].theta;
        let red = a.curtains[1].red;
        a.resize(Size::new(61, 17), &mut ctx);
        assert_eq!(a.curtains[0].theta, theta);
        assert_eq!(a.curtains[1].red, red);
        assert_eq!(a.view.pixels.pixel_size(), Size::new(61, 34));
        a.resize(Size::new(1, 1), &mut ctx);
        a.update(0.25, &mut ctx);
        let mut canvas = Canvas::new(Size::new(1, 1), GlyphMode::Unicode);
        a.render(&mut canvas);
    }
}
