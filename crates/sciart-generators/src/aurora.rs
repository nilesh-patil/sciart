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
use sciart_core::{draw, geom, ink, rng};

pub static META: Meta = Meta {
    name: "aurora",
    category: Category::EarthSky,
    description: "Auroral curtains folding over a still northern lake at night",
    title: "Aurora · electrons light the air",
    captions: &CAPTIONS,
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
/// The title's caption during each beat, in `BEATS` order (AUR-A9).
const CAPTIONS: [&str; BEATS.len()] = [
    "growth phase",
    "substorm onset",
    "substorm expansion",
    "pulsating recovery",
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
/// Tones at which green and red light begin to show. The theme floors lit roles at 30 %
/// brightness, so fainter light is left dark rather than scattered as a dither: each
/// column lights one contiguous run where its emission profile crosses the threshold,
/// and the level ramps up from there (`Glow::ink`). Rays that reach higher end higher,
/// so the curtain's top edge is combed into vertical streaks, and the sky between the
/// curtains stays black.
const GREEN_LIT: f32 = 0.14;
const RED_LIT: f32 = 0.2;
/// The red fringe is drawn in the `HAlpha` ramp at levels up to this, its dark crimson
/// end, which is how the 630 nm crown reads over green in photographs. The pink hem, the
/// same ramp's rose, runs from `HEM_LEVEL`, well above it, to `HEM_LEVEL_MAX`.
const RED_LEVEL_MAX: f64 = 155.0;
/// The crown starts at this crimson rather than the core floor: the ramp's darkest
/// maroon, next to the green, read as brown mud rather than red light.
const RED_LEVEL_MIN: f64 = 56.0;
const RED: Role = Role::Ramp(Ramp::HAlpha);
/// Green 557.7 nm light: the `Aurora` ramp, deep teal where faint (false colour: the faint
/// green mixed with blue N₂⁺ 427.8 nm, as photographs show it) to lime-white in the rays.
const GREEN: Role = Role::Ramp(Ramp::Aurora);
/// Red-dominant light shows only where the green's own tone is at least this: the
/// crown caps each ray and fold with crimson and so follows the curtains' envelope.
/// Diffuse red far above the green would be cut flat by the top fade (a lid) and stand
/// as a slab over the sky; at the naked eye's photopic weight it is below notice there.
const CROWN_GREEN: f32 = 0.03;
const HEM_LEVEL: f64 = 196.0;
/// The hem tops out here, in the ramp's rose: above it `HAlpha` pales to near-white.
const HEM_LEVEL_MAX: f64 = 212.0;
/// Rose level of the green pixel just above a hem pixel (AUR-A10).
const HEM_BLEND: [u8; 1] = [120];
/// Crimson levels of the sky pixels just under the hem, fading toward the ground.
const HEM_BELOW_FADE: [u8; 1] = [45];
/// The curtains' ripple (AUR-A10): relative amplitude, wavenumber (rad per column) and
/// angular speed (rad/s): a slow brightness wave travelling along the arc, as the rays
/// of a real curtain ripple sideways.
const RIPPLE_AMP: f64 = 0.12;
const RIPPLE_K: f64 = 0.09;
const RIPPLE_W: f64 = 0.7;
/// Emission fades in under the top edge over this share of the pixel rows (at least
/// `TOP_FADE_MIN_PX`), as a smoothstep applied to the tone before the threshold: a crown
/// overhead thins into dark sky over ≈ 10 cell rows (keeping the label in clear air), so
/// its top edge follows the curtains' own envelope rather than meeting a lid.
const TOP_FADE: f64 = 0.2;
const TOP_FADE_MIN_PX: f64 = 3.0;
/// O(¹D) atoms diffuse tens of km during their 110 s lifetime, so the red emission is
/// smoothed along the arc by a Gaussian of this width (≈ 20 km at 300 km) before it is
/// thresholded: where folds double back and the layer count steps, the red ramps instead
/// of standing as a wall.
const RED_DIFFUSE_DEG: f64 = 4.0;
/// Horizon skyglow: sky pixels below this elevation (or below the tallest silhouette plus
/// `GLOW_CLEAR_PX`, if higher) glow, brightest at the horizon, so the ridge and spruce
/// spires stand against it as a crisp black edge.
const GLOW_DEG: f64 = 8.0;
/// Skyglow `Seq` value at the horizon and at the glow's top edge, falling quadratically
/// between: the ramp's black end is a dark navy, so the glow stops just above it rather
/// than being carried into it.
const GLOW_MAX: f64 = 0.2;
const GLOW_MIN: f64 = 0.02;
/// The skyglow's ramp (AUR-A10): `Starlight`, whose floor is near black, so the glow
/// behind the hills fades into the night with no band edge, and the black ridge and
/// spires stand against it as a silhouette.
const GLOW_RAMP: Ramp = Ramp::Starlight;
const SKYGLOW: Role = Role::Ramp(GLOW_RAMP);
/// The lake mirrors the sky at this share of its level, each pixel a 3 × 2 box sum of the
/// sky pixels round its source of the same light, over 6 (so a run's ends soften), and
/// fading with depth to `1 − LAKE_DEPTH_FADE` of that at the lake's far edge: soft
/// streaks of water rather than pixel noise.
const LAKE_GAIN: f32 = 0.25;
const LAKE_DEPTH_FADE: f32 = 0.6;
/// Ripple crests dim a lake row by up to this share, as bands rather than gaps.
const CREST_DIM: f32 = 0.45;
/// A sky run whose brightest pixel is below this level is not mirrored (see
/// `View::reflect`).
const LAKE_MIN_LEVEL: u8 = 40;
/// Sideways ripple displacement per pixel row below the shore.
const RIPPLE_SHIFT: f32 = 0.04;
/// Glints on the water (AUR-A10): each lake pixel's gain is scaled by `1 − SHIMMER` to 1
/// by a hash of its row and of `SHIMMER_RUN`-pixel runs drifting along it at
/// `SHIMMER_DRIFT` px/s, so the mirrored bands break into moving streaks of light.
const SHIMMER: f32 = 0.45;
const SHIMMER_RUN: f64 = 4.0;
const SHIMMER_DRIFT: f64 = 3.0;
/// Mirrored skyglow pixels whose glint factor is at or below this are left dark.
const SHIMMER_GAP: f32 = 1.0 - 0.6 * SHIMMER;
/// Display weight of red 630 nm against green 557.7 nm: the photopic luminous efficiency
/// ratio V(630)/V(557.7) ≈ 0.27. The emission profiles stay physical; this is how bright
/// the red *looks*, so the diffuse fringe stays a fringe.
const RED_WEIGHT: f32 = 0.27;
/// Minimum pink-hem tone for a hem pixel: type-B hems mark only the brightest arc.
const HEM_TONE: f32 = 0.5;
/// Layers per column: one crossing per curtain plus room for three more where folds
/// double back; a seventh merges into the nearest (see `View::push_layer`).
const MAX_LAYERS: usize = 6;

const STARS: usize = 1200;
const STAR_FRACTION: f64 = 0.02;
const PATCHES: usize = 6;
/// Peak of `green_profile(·, 0, H_GREEN)`, reached at u ≈ 3 km.
const GREEN_PEAK: f64 = 0.80;
/// Pulsating-patch green: 0.45× the peak green of a unit-flux arc, so a patch at full
/// pulse stays dimmer than the recovery arc, the focal point.
const PATCH_GAIN: f64 = 0.45 * GREEN_PEAK;
/// Display floor on the patch half-extents (px): a 6–12° patch is only 4–7 columns at
/// 80×24, a smudge rather than a patch. The floor is frame-relative, at most
/// `PATCH_FLOOR_SHARE` of the width and of the pixel rows, so on a small frame a patch
/// never swells into a cloud half the sky wide (`patch_floor_px`).
const PATCH_MIN_PX: (f64, f64) = (5.0, 3.0);
const PATCH_FLOOR_SHARE: (f64, f64) = (0.07, 1.0 / 12.0);
/// Gaussian support of a patch, in half-extents.
const PATCH_REACH: f64 = 1.6;
/// Patch azimuths are stratified over ±`PATCH_SPAN_DEG` with ±`PATCH_JITTER_DEG` of
/// jitter (strata 15° apart, ≈ 9 px at 80×24 where the floor makes a patch 10 px
/// wide): they all drift at the same rate, so neighbours keep apart instead of fusing
/// into one amorphous blob.
const PATCH_SPAN_DEG: f64 = 45.0;
const PATCH_JITTER_DEG: f64 = 3.0;

/// Patch half-extent floor (px) for a frame `w` columns by `rows` pixel rows.
fn patch_floor_px(w: usize, rows: usize) -> (f64, f64) {
    (
        PATCH_MIN_PX.0.min(PATCH_FLOOR_SHARE.0 * w as f64),
        PATCH_MIN_PX.1.min(PATCH_FLOOR_SHARE.1 * rows as f64),
    )
}

const LABEL_INK: Ink = Ink::new(Role::Text, 150);
const RIDGE_INK: Ink = Ink::new(Role::Dim, 70);
const STAR_DIM: Glyph = Glyph::new('·', '.');
const STAR_BRIGHT: Glyph = Glyph::new('✦', '*');
/// ASCII green by level: '|', '!', ':' from these levels up, ',' below (stars keep '.'
/// and '*').
const ASCII_BAR: u8 = 200;
const ASCII_BANG: u8 = 130;
const ASCII_COLON: u8 = 70;
/// ASCII red: '"' from this level up, '\'' below.
const ASCII_RED_BRIGHT: u8 = 60;
/// The roles of auroral light: green, and the red crown and pink hem (the skyglow is
/// `Seq`).
const EMISSION: [Role; 2] = [GREEN, RED];

/// A pink-hem pixel: `RED` above the red crown's levels.
fn is_hem(ink: Ink) -> bool {
    ink.role == RED && f64::from(ink.level) > RED_LEVEL_MAX
}

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
    let c = (kappa * s + phase).cos_det().max(0.0);
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
    (0.0, 0.35),
    (ONSET, 0.4),
    (BREAKUP, 1.0),
    (PEAK, 1.0),
    (RECOVERY, 0.75),
    (LOOP, 0.35),
];
const K_A2: Knots = &[
    (0.0, 0.3),
    (ONSET, 0.3),
    (EXPANSION, 0.3),
    (EXPANSION + 6.0, 0.55),
    (RECOVERY, 0.4),
    (LOOP, 0.3),
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
    (0.0, 0.04),
    (ONSET, 0.04),
    (BREAKUP, 0.8),
    (RECOVERY, 0.8),
    (RECOVERY + 12.0, 0.04),
    (LOOP, 0.04),
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
                    let (sn, cs) = (kappa * s + self.theta[k]).sin_cos_det();
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
/// Ridge elevation `RIDGE_DEG.0 + RIDGE_DEG.1·n` degrees for octave noise `n ∈ [0, 1]`:
/// low rolling hills across the lake.
const RIDGE_DEG: (f64, f64) = (0.8, 3.2);

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
/// Display floor (px) for the far spires: at 80×24 (f ≈ 35 px/rad) a 3° spruce is under
/// 2 px and the comb reads as a ruled line, so the far forest is stretched upward until
/// a 3° spire (its height cap) stands this high (widths unchanged, so the spires get
/// spikier, not wider).
const SPIRE_MIN_PX: f64 = 3.0;
/// The skyglow stands at least this far (px) above the tallest silhouette, so every
/// spire is cut out of the glow rather than out of the black sky.
const GLOW_CLEAR_PX: f64 = 1.5;

/// Height factor for the spires at scale `f` (px/rad): 1 for the near forest, and for
/// the far one the stretch that lifts a spire at its 3° height cap to `SPIRE_MIN_PX`.
fn spire_lift(f: f64) -> f64 {
    if f > BIG_TREE_F {
        1.0
    } else {
        (SPIRE_MIN_PX / (f * FAR_FOREST.height.1.to_radians())).max(1.0)
    }
}

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
        let patches = std::array::from_fn(|i| Patch {
            alpha: (-PATCH_SPAN_DEG
                + 2.0 * PATCH_SPAN_DEG * (i as f64 + 0.5) / PATCHES as f64
                + rng.random_range(-PATCH_JITTER_DEG..PATCH_JITTER_DEG))
            .to_radians(),
            e: rng.random_range(25.0f64..45.0).to_radians(),
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
        (RIDGE_DEG.0 + RIDGE_DEG.1 * n).to_radians()
    }

    /// Highest spire top (rad) over the azimuth interval `[lo, hi]`, if any tree
    /// reaches into it: a triangle profile, its height scaled by `lift`, maximised over
    /// the interval.
    fn tree_top(&self, trees: &[Tree], lift: f64, lo: f64, hi: f64) -> Option<f64> {
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
                self.ridge(t.alpha) + lift * t.height * (1.0 - gap / t.half).max(0.0)
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

/// A sky pixel's tone-mapped emission.
#[derive(Debug, Clone, Copy, Default)]
struct Glow {
    /// Tone-mapped brightness in [0, 1).
    tone: f32,
    /// Red share of the emission, R/(G+R).
    red: f32,
    /// Inside a bright pink hem.
    hem: bool,
    /// The green's own tone here, which gates the red crown (`CROWN_GREEN`).
    green: f32,
}

/// A sky pixel's emission before tone mapping.
#[derive(Debug, Clone, Copy, Default)]
struct Emit {
    /// Curtain green, which alone gates the red crown.
    green: f32,
    /// Red after `RED_WEIGHT`, smoothed along the rows by `View::diffuse_red`.
    red: f32,
    pink: f32,
    /// Pulsating-patch green: kept apart from the curtains' green so a patch inside the
    /// lingering red dome never switches the crown on around itself.
    patch: f32,
}

/// An empty row range for `View::emit_rows`.
const NO_ROWS: (usize, usize) = (usize::MAX, 0);

impl Glow {
    /// The pixel's ink, or `None` below its threshold (`GREEN_LIT`, or `RED_LIT` where
    /// red dominates, and there only within the green's tail, `CROWN_GREEN`). Above it
    /// the level ramps from the core floor `ink::MIN_VISIBLE_LEVEL` (CORE-R66; to 255 for
    /// green, to `RED_LEVEL_MAX` for red), so every lit run starts from the theme's
    /// dimmest shade of its role and no fade ends in a bright lid. The pink hem is crisp
    /// and bright: `HEM_LEVEL` to `HEM_LEVEL_MAX`.
    fn ink(self) -> Option<Ink> {
        let red = self.red > 0.5;
        let lit = if red { RED_LIT } else { GREEN_LIT };
        if self.tone < lit || red && self.green < CROWN_GREEN {
            return None;
        }
        let v = f64::from((self.tone - lit) / (1.0 - lit));
        let (role, lo, hi) = if self.hem {
            (RED, HEM_LEVEL, HEM_LEVEL_MAX)
        } else if red {
            (RED, RED_LEVEL_MIN, RED_LEVEL_MAX)
        } else {
            (GREEN, ink::MIN_VISIBLE_LEVEL, 255.0)
        };
        Some(Ink::new(role, ink::level(lo + (hi - lo) * v)))
    }
}

/// A lake row's gain at depth share `depth ∈ [0, 1)` below the shore and ripple phase
/// `sin` (crests at +1): fading with depth, and dimmed on the crests (AUR-A7).
fn lake_row_gain(depth: f32, sin: f32) -> f32 {
    (1.0 - LAKE_DEPTH_FADE * depth) * (1.0 - CREST_DIM * sin.max(0.0).powi(2))
}

/// The glint factor in `[1 − SHIMMER, 1]` of lake pixel `(x, y)`, `k` rows below the
/// shore, at time `t` (AUR-A10): alternate rows drift opposite ways, like wind ripples
/// crossing.
fn shimmer(x: usize, y: usize, k: usize, t: f64) -> f32 {
    let drift = if k % 2 == 0 {
        SHIMMER_DRIFT
    } else {
        -SHIMMER_DRIFT
    } * t;
    let run = ((x as f64 + drift) / SHIMMER_RUN).floor() as i32;
    1.0 - SHIMMER * rng::hash01(rng::cell_key(run, y as i32)) as f32
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
/// where it has faded out: solid, falling quadratically into the ramp's near-black end,
/// so it needs no dither to fade.
fn skyglow(px: f64, height: f64) -> Option<Ink> {
    let k = 1.0 - px.max(0.0) / height;
    (k > 0.0).then(|| ink::ramp(GLOW_RAMP, GLOW_MIN + (GLOW_MAX - GLOW_MIN) * k * k))
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
    /// Box radius (px) of the red diffusion: three passes approximate the Gaussian.
    red_box: usize,
    /// Emission before tone mapping, `w` per sky row, and the rows that hold any
    /// (`emit_rows.0 > emit_rows.1` when none).
    emit: Vec<Emit>,
    emit_rows: (usize, usize),
    /// Two rows of scratch for the red diffusion.
    blur: [Vec<f32>; 2],
    /// Per sky pixel (`w` per row), the brightest level of its column's lit run; valid
    /// where the pixel is emission (`View::run_peaks`).
    run_peak: Vec<u8>,
    /// Skyglow ink per sky row (`None` above the glow).
    skyglow: Vec<Option<Ink>>,
    ripple_cos: Vec<f32>,
    ripple_sin: Vec<f32>,
    /// This frame's curtain ripple gain per column (AUR-A10).
    ripple: Vec<f32>,
    /// Scratch: each column's pink-border row this frame (AUR-A10).
    fringe: Vec<i32>,
    stars: Vec<ShownStar>,
    layers: Vec<[Layer; MAX_LAYERS]>,
    nlayers: Vec<u8>,
    pixels: Pixels,
    /// Whether the AL readout fits.
    label: bool,
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
                let sin_e = elev(y).max(2f64.to_radians()).sin_det();
                (-0.12 * (1.0 / sin_e - 1.0)).exp() as f32
            })
            .collect();
        let mut view = Self {
            w,
            rows,
            y_h,
            lake,
            f,
            tan_e,
            ext,
            top: {
                let fade = top_fade_px(rows);
                (0..y_h)
                    .map(|y| ease::smoothstep(y as f64 / fade) as f32)
                    .collect()
            },
            red_box: box_radius(f * RED_DIFFUSE_DEG.to_radians()),
            emit: vec![Emit::default(); w * y_h],
            emit_rows: NO_ROWS,
            blur: [vec![0.0; w], vec![0.0; w]],
            run_peak: vec![0; w * y_h],
            ripple_cos: (0..w).map(|x| (0.13 * x as f64).cos_det() as f32).collect(),
            ripple_sin: (0..w).map(|x| (0.13 * x as f64).sin_det() as f32).collect(),
            layers: vec![[Layer::default(); MAX_LAYERS]; w],
            fringe: Vec::with_capacity(w),
            nlayers: vec![0; w],
            pixels: Pixels::new(size),
            label: w >= 24 && h >= 10,
            ..Self::default()
        };
        view.build_silhouette(setting);
        view.skyglow = {
            let height = view.glow_px();
            (0..y_h)
                .map(|y| skyglow((y_h - y) as f64 - 0.5, height))
                .collect()
        };
        view.pick_stars(setting);
        view
    }

    #[cfg(test)]
    fn top_fade_px(&self) -> f64 {
        top_fade_px(self.rows)
    }

    /// Skyglow height (px): `GLOW_DEG`, but at least `GLOW_CLEAR_PX` above the tallest
    /// silhouette (and 3 px), so small views keep a readable ridge.
    fn glow_px(&self) -> f64 {
        let tallest = self.ridge_top.iter().map(|&t| self.y_h as i32 - t).max();
        (self.f * GLOW_DEG.to_radians())
            .max(f64::from(tallest.unwrap_or(0)) + GLOW_CLEAR_PX)
            .max(3.0)
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
        let lift = spire_lift(self.f);
        for x in 0..self.w {
            let a = self.alpha_at(x as f64 + 0.5);
            let ridge = setting.ridge(a);
            let base = top_row(ridge);
            let (top, tip) = match setting.tree_top(trees, lift, a - half, a + half) {
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

    /// Add a layer to column `x`; a layer beyond `MAX_LAYERS` merges into the one at the
    /// nearest distance.
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
            if (alpha - p.alpha).abs() < PATCH_REACH * p.half_a {
                y0 = y0.min(self.row_of(p.e + PATCH_REACH * p.half_e) as i32);
                y1 = y1.max(self.row_of(p.e - PATCH_REACH * p.half_e) as i32 + 1);
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
            let (mut g, mut r, mut p, mut pa) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
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
                if da.abs() < PATCH_REACH && de.abs() < PATCH_REACH {
                    pa += (lp.gain * (-da * da - de * de).exp()) as f32;
                }
            }
            self.emit[yu * self.w + x] = Emit {
                green: g,
                red: RED_WEIGHT * r,
                pink: p,
                patch: pa,
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

    /// Tone-map every emitting sky pixel above the silhouette and light it if it clears
    /// its threshold (`Glow::ink`).
    fn shade(&mut self, luts: &Luts) {
        for y in self.emit_rows.0..=self.emit_rows.1 {
            let ext = self.ext[y] * (1.0 / E_SAT as f32);
            for x in 0..self.w {
                if y as i32 >= self.ridge_top[x] {
                    continue;
                }
                let e = self.emit[y * self.w + x];
                // The crown shows only within the curtains' green tail (`CROWN_GREEN`);
                // elsewhere the red is below notice and is dropped, so a patch inside the
                // lingering red dome shows green with no red hood.
                let green = luts.tone(ext * e.green);
                let red = if green >= CROWN_GREEN { e.red } else { 0.0 };
                let sum = e.green + e.patch + red + e.pink;
                if sum <= 0.0 {
                    continue;
                }
                let ripple = self.ripple.get(x).copied().unwrap_or(1.0);
                let glow = Glow {
                    tone: (self.top[y] * ripple * luts.tone(ext * sum)).min(0.999),
                    red: red / (e.green + e.patch + red).max(1e-12),
                    hem: e.pink >= 0.25 * e.green && luts.tone(ext * e.pink) >= HEM_TONE,
                    green,
                };
                if let Some(ink) = glow.ink() {
                    self.pixels.set(x as i32, y as i32, ink);
                }
            }
        }
    }

    /// Skyglow in the still-dark sky just above the silhouette of column `x`.
    fn skyglow_column(&mut self, x: usize) {
        for y in (0..self.ridge_top[x]).rev() {
            let Some(glow) = self.skyglow[y as usize] else {
                break;
            };
            if self
                .pixels
                .get(x as i32, y)
                .is_some_and(|i| i.role == Role::Background)
            {
                self.pixels.set(x as i32, y, glow);
            }
        }
    }

    /// Drop specks: lit sky pixels with no lit neighbour (of 8), and pairs whose two
    /// pixels neighbour only each other. Where a fading arc end or a distant arc on the
    /// horizon hovers at its threshold it would otherwise crumble into grit. In place is
    /// safe: a dropped speck touched nothing else, so no other pixel's test changes.
    fn despeckle(&mut self) {
        let (lo, hi) = self.emit_rows;
        let lit =
            |p: &Pixels, x: i32, y: i32| p.get(x, y).is_some_and(|i| EMISSION.contains(&i.role));
        // Lit neighbours of (x, y), and the last one found.
        let around = |p: &Pixels, x: i32, y: i32| {
            let (mut n, mut last) = (0, (x, y));
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if (dx, dy) != (0, 0) && lit(p, x + dx, y + dy) {
                        n += 1;
                        last = (x + dx, y + dy);
                    }
                }
            }
            (n, last)
        };
        for y in lo..=hi.min(self.y_h.saturating_sub(1)) {
            let y = y as i32;
            for x in 0..self.w as i32 {
                if !lit(&self.pixels, x, y) {
                    continue;
                }
                match around(&self.pixels, x, y) {
                    (0, _) => self.pixels.set(x, y, Ink::BLANK),
                    (1, (qx, qy)) if around(&self.pixels, qx, qy).0 == 1 => {
                        self.pixels.set(x, y, Ink::BLANK);
                        self.pixels.set(qx, qy, Ink::BLANK);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Demote hem pixels with no hem neighbour in the next column on either side (any
    /// of the three rows, so a sloping hem still counts as a run) to green. In place is
    /// safe: a demoted pixel had no hem neighbour, so no other pixel relied on it.
    fn trim_hem(&mut self) {
        let accent = |p: &Pixels, x: i32, y: i32| p.get(x, y).is_some_and(is_hem);
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
                    self.pixels.set(x, y, Ink::new(GREEN, level));
                }
            }
        }
    }

    /// The pink fringe (AUR-A10): in every column where the pink-carrying curtain's lower
    /// border is lit, its lowest lit pixel within the 5 km band is the hem (rose, brighter
    /// where the curtain is), the green above it steps through `HEM_BLEND` into the
    /// curtain, and the sky below takes `HEM_BELOW_FADE`, so the fringe follows the
    /// curtain continuously as a soft three-to-four-row gradient rather than dashes where
    /// the pink alone clears a threshold. A display softening: the real border is sharp.
    fn soften_hem(&mut self) {
        let span = (HEM_LEVEL_MAX - HEM_LEVEL) as f32;
        // Pass 1: the border row of each column (−1 where none), so a lone column, with no
        // neighbour on the border within a row, takes no fringe (no confetti).
        let mut border = std::mem::take(&mut self.fringe);
        border.clear();
        border.extend((0..self.w).map(|x| self.border_row(x).unwrap_or(-1)));
        for x in 0..self.w {
            let yb = border[x];
            if yb < 0 {
                continue;
            }
            let joined = [x.wrapping_sub(1), x + 1].iter().any(|&n| {
                border
                    .get(n)
                    .is_some_and(|&b| b >= 0 && (b - yb).abs() <= 1)
            });
            if !joined {
                continue;
            }
            let ridge = self.ridge_top[x].min(self.y_h as i32);
            let xi = x as i32;
            let base = self.pixels.get(xi, yb).map_or(0, |i| i.level);
            let level = HEM_LEVEL as f32 + span * f32::from(base) / 255.0;
            self.pixels.set(xi, yb, Ink::new(RED, level.round() as u8));
            for (k, &blend) in (1..).zip(&HEM_BLEND) {
                let y = yb - k;
                if y >= 0
                    && self.pixels.get(xi, y).is_some_and(|i| i.role == GREEN)
                    && self.curtain(x, y)
                {
                    self.pixels.set(xi, y, Ink::new(RED, blend));
                }
            }
            for (k, &fade) in (1..).zip(&HEM_BELOW_FADE) {
                let y = yb + k;
                let free = self
                    .pixels
                    .get(xi, y)
                    .is_some_and(|i| i == Ink::BLANK || i.role == SKYGLOW);
                if y >= ridge || !free {
                    break;
                }
                self.pixels.set(xi, y, Ink::new(RED, fade));
            }
        }
        self.fringe = border;
        // Any other hem pixel (a gated one off the border row) steps into the green too.
        for x in 0..self.w {
            let xi = x as i32;
            for y in 1..self.ridge_top[x].min(self.y_h as i32) {
                let hem = self.pixels.get(xi, y).is_some_and(is_hem);
                let green = self.pixels.get(xi, y - 1).is_some_and(|i| i.role == GREEN);
                if hem && green && self.curtain(x, y - 1) {
                    self.pixels.set(xi, y - 1, Ink::new(RED, HEM_BLEND[0]));
                }
            }
        }
    }

    /// Whether sky pixel `(x, y)`'s light is the curtains' alone (a pulsating patch adds
    /// at most a tenth), so the fringe never marks a patch.
    fn curtain(&self, x: usize, y: i32) -> bool {
        let e = self.emit[y as usize * self.w + x];
        e.patch <= 0.02 * e.green
    }

    /// The row of column `x`'s pink border (AUR-A10): the lowest lit curtain pixel within
    /// the 5 km band of the pink-carrying layer's lowest row, if any.
    fn border_row(&self, x: usize) -> Option<i32> {
        let layers = &self.layers[x][..usize::from(self.nlayers[x])];
        let lowest = layers
            .iter()
            .filter(|l| l.pink > 0.0)
            .map(|l| l.y_low)
            .max()?;
        let ridge = self.ridge_top[x].min(self.y_h as i32);
        let xi = x as i32;
        let lit = |y: i32| {
            self.pixels
                .get(xi, y)
                .is_some_and(|i| i.role == GREEN || is_hem(i))
        };
        let yb = (lowest - 4..=lowest.min(ridge - 1))
            .rev()
            .find(|&y| y >= 0 && lit(y))?;
        self.curtain(x, yb).then_some(yb)
    }

    /// The curtains' travelling ripple for time `t` (AUR-A10): a brightness wave running
    /// along the arc, `1 + RIPPLE_AMP·sin(RIPPLE_K·x − RIPPLE_W·t)` per column.
    fn set_ripple(&mut self, t: f64) {
        self.ripple.clear();
        self.ripple.extend(
            (0..self.w).map(|x| {
                (1.0 + RIPPLE_AMP * (RIPPLE_K * x as f64 - RIPPLE_W * t).sin_det()) as f32
            }),
        );
    }

    /// For every lit sky pixel, the brightest level of its column's contiguous run of
    /// emission pixels (`reflect` mirrors a run whole or not at all). Only the emitting
    /// rows can hold emission, so only they are scanned.
    fn run_peaks(&mut self) {
        let (lo, hi) = self.emit_rows;
        let hi = hi.min(self.y_h.saturating_sub(1));
        if lo > hi {
            return;
        }
        let (w, pixels, peaks) = (self.w, &self.pixels, &mut self.run_peak);
        let level = |x: usize, y: usize| {
            pixels
                .get(x as i32, y as i32)
                .filter(|i| EMISSION.contains(&i.role))
                .map(|i| i.level)
        };
        for x in 0..w {
            let mut y = lo;
            while y <= hi {
                let start = y;
                let mut peak = None;
                while y <= hi
                    && let Some(l) = level(x, y)
                {
                    peak = peak.max(Some(l));
                    y += 1;
                }
                if let Some(peak) = peak {
                    for yy in start..y {
                        peaks[yy * w + x] = peak;
                    }
                } else {
                    y += 1;
                }
            }
        }
    }

    /// Mirror the sky into the lake at `LAKE_GAIN`× level with a ripple whose wavefronts
    /// travel toward the viewer. The crests dim whole rows by up to `CREST_DIM`, so
    /// mirrored rays read as vertical streaks crossed by darker ripple bands; the
    /// x-dependent phase only shifts pixels sideways, by at most `RIPPLE_SHIFT·d` px, so
    /// the water stays still. Deeper rows fade (`LAKE_DEPTH_FADE`). The lake
    /// copies the skyglow and the sky's lit pixels with their roles, dimming only the
    /// level (never below the core floor `ink::MIN_VISIBLE_LEVEL`). Each column's lit
    /// run is reflected whole or not at all: only runs whose brightest pixel reaches
    /// `LAKE_MIN_LEVEL` (`run_peaks`), since a faint run's copy could not be dimmer than
    /// the theme's floor, while cutting the dim ends off bright runs would leave shards.
    /// The ridge and spires mirror as a black band cut out of the reflected glow.
    /// One past the lake's last pixel row: the title's cell row and the one above it
    /// stay dark, so no reflection crowds the title (GEN-R13).
    fn lake_end(&self) -> usize {
        self.rows.saturating_sub(4).max(self.y_h)
    }

    /// The lake's copy of sky pixel `(sx, src)` at `gain` (depth and ripple, ≤ 1), or
    /// `None` where the water shows nothing (AUR-A7): the skyglow, and every pixel of a
    /// sky run whose brightest pixel reaches `LAKE_MIN_LEVEL`, dimmed to `LAKE_GAIN·gain`
    /// of the 3 × 2 box sum over 6 of the pixels of its light in its row and the one above
    /// (soft ends where the box leaves the run).
    fn lake_ink(&self, sx: usize, src: usize, gain: f32) -> Option<Ink> {
        let sky = self.pixels.get(sx as i32, src as i32)?;
        // Only the green is mirrored: the dim red crown's copy would sit on the water as
        // isolated dark-red specks (AUR-A10).
        let emission = sky.role == GREEN && self.run_peak[src * self.w + sx] >= LAKE_MIN_LEVEL;
        if sky.role != SKYGLOW && !emission {
            return None;
        }
        let xs = sx.saturating_sub(1)..=(sx + 1).min(self.w - 1);
        let sum: f32 = (src.saturating_sub(1)..=src)
            .flat_map(|y| xs.clone().map(move |x| (x, y)))
            .filter_map(|(x, y)| self.pixels.get(x as i32, y as i32))
            .filter(|i| i.role == sky.role)
            .map(|i| f32::from(i.level))
            .sum();
        let level = (LAKE_GAIN * gain * sum / 6.0).round() as u8;
        let level = if emission {
            level.max(ink::MIN_VISIBLE_LEVEL as u8)
        } else {
            level
        };
        Some(Ink::new(sky.role, level))
    }

    fn reflect(&mut self, t: f64) {
        if !self.lake {
            return;
        }
        self.run_peaks();
        let depth = (self.lake_end() - self.y_h).max(1) as f32;
        for y in self.y_h..self.lake_end() {
            let k = y - self.y_h;
            let Some(src) = self.y_h.checked_sub(k + 1) else {
                break;
            };
            let d = (k + 1) as f64;
            let (sp, cp) = (0.9 * d - 1.7 * t).sin_cos_det();
            let (sp, cp) = (sp as f32, cp as f32);
            let gain = lake_row_gain(k as f32 / depth, sp);
            for x in 0..self.w {
                // sin(0.9d − 1.7t + 0.13x) by angle addition: no per-pixel trig.
                let s = sp * self.ripple_cos[x] + cp * self.ripple_sin[x];
                let sx = x as isize + (RIPPLE_SHIFT * d as f32 * s).round() as isize;
                let Ok(sx) = usize::try_from(sx) else {
                    continue;
                };
                let glint = shimmer(x, y, k, t);
                let gain = gain * glint;
                if sx < self.w
                    && let Some(ink) = self.lake_ink(sx, src, gain)
                    // The mirrored skyglow sits at the ramp's floor, where dimming cannot
                    // show: its darker glints are gaps instead, so the band ripples.
                    && (ink.role != SKYGLOW || glint > SHIMMER_GAP)
                {
                    self.pixels.set(x as i32, y as i32, ink);
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
        // Before the first resize there is no view to floor the size against.
        let f = if self.view.f > 0.0 {
            self.view.f
        } else {
            f64::INFINITY
        };
        let (floor_a, floor_e) = patch_floor_px(self.view.w, self.view.rows);
        for p in &self.setting.patches {
            let gain = PATCH_GAIN * self.params.patches * patch_pulse(self.t, p.period, p.phase);
            if gain > 1e-3 {
                self.patches[self.live_patches] = LivePatch {
                    alpha: p.alpha + drift,
                    e: p.e,
                    half_a: p.half_a.max(floor_a / f),
                    half_e: p.half_e.max(floor_e / f),
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
        view.clear_emit();
        let patches = &self.patches[..self.live_patches];
        for x in 0..view.w {
            view.emit_column(x, &self.luts, patches);
        }
        view.diffuse_red();
        view.set_ripple(self.t);
        view.shade(&self.luts);
        view.despeckle();
        for x in 0..view.w {
            view.skyglow_column(x);
        }
        if pink > 0.0 {
            view.trim_hem();
            view.soften_hem();
        }
        view.reflect(self.t);
    }

    /// The ASCII fallback for one cell: its brighter emission pixel (a hem pixel wins,
    /// so the hem stays a line). Skyglow is left blank for the ridge line.
    fn ascii_cell(&self, cy: i32, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let pair = [top, bottom];
        let emission = pair.into_iter().filter(|i| EMISSION.contains(&i.role));
        if 2 * cy as usize >= self.view.y_h {
            let ink = emission.max_by_key(|i| i.level)?;
            let ch = if ink.level >= 60 { '~' } else { '-' };
            return Some((ch, ink));
        }
        let ink = emission.max_by_key(|&i| (is_hem(i), i.level))?;
        let ch = match ink.role {
            _ if is_hem(ink) => '=',
            RED if ink.level >= ASCII_RED_BRIGHT => '"',
            RED => '\'',
            // The faintest green is ',' so a quiet arc never reads as the dim stars '.'.
            _ => match ink.level {
                ASCII_BAR.. => '|',
                ASCII_BANG.. => '!',
                ASCII_COLON.. => ':',
                _ => ',',
            },
        };
        Some((ch, ink))
    }

    /// Stars in cells whose two pixels are empty, twinkling more toward the horizon.
    /// A star also needs a still-blank cell (always true in Unicode mode when both
    /// pixels are empty).
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
            let depth = 0.15 + 0.45 * (1.0 - star.e.sin_det());
            let twinkle = 1.0 - depth * (0.5 + 0.5 * (star.rate * self.t + star.phase).sin_det());
            let level = (base * ext * twinkle).clamp(25.0, 200.0) as u8;
            let glyph = if s.bright { STAR_BRIGHT } else { STAR_DIM };
            canvas.put(s.cx, s.cy, glyph, Ink::new(Role::Star, level));
        }
    }

    /// ASCII only, in still-blank cells: the bare ridge line, one `_` per column in the
    /// cell holding its top pixel (spire flanks do not lift it), with a spire tip `^` in
    /// the cell just above the line wherever a spire's top pixel lies above it, so the
    /// forest is one comb however tall its spires. A `_` sits
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
            if let Some(b) = base
                && cell(v.ridge_top[x]).is_some_and(|tip| v.tree[x] && tip < b)
            {
                put(x, b - 1, '^');
            }
        }
    }

    /// ` AL -620 nT `: AL rounded to 10 nT, with a blank gutter cell on each side
    /// (CORE-R67) so the label never merges into the curtain. The beat is the title's
    /// caption, so it is not repeated here (AUR-A9).
    fn draw_label(&self, canvas: &mut Canvas) {
        if !self.view.label {
            return;
        }
        let al = (al_from(self.params.activity) / 10.0).round() as i32 * 10;
        let mut buf = StackText::<16>::new();
        let _ = write!(buf, "AL {al} nT");
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

    fn caption(&self) -> Option<&'static str> {
        CAPTIONS.get(self.timeline.at(self.t).index).copied()
    }

    fn render(&self, canvas: &mut Canvas) {
        self.view.pixels.blit(canvas, |_, cy, top, bottom| {
            self.ascii_cell(cy, top, bottom)
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
        scene_seeded(w, h, 7)
    }

    fn scene_seeded(w: u16, h: u16, seed: u64) -> (Aurora, Ctx) {
        let mut ctx = Ctx::new(seed, Size::new(w, h));
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
            if a.abs() < ALPHA_MAX && zb > drop_km(300.0 / a.cos_det()) {
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
        assert!((slant_factor(0.0, 0.0, dd, h) - 1.0 / eb.cos_det()).abs() < 1e-9);
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

    #[test]
    fn aur_a7_expansion_frame_respects_silhouette_lake_stars_and_hem() {
        let (a, canvas) = census(56);
        assert!((a.t - 22.0).abs() < 1e-9);
        let v = &a.view;
        let px = |x: usize, y: usize| v.pixels.get(x as i32, y as i32).unwrap_or(Ink::BLANK);
        let (mut sky, mut lit, mut sky_max, mut hems) = (0, 0, 0u8, 0);
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
                hems += usize::from(is_hem(ink));
            }
        }
        let frac = f64::from(lit) / f64::from(sky);
        assert!((0.15..=0.70).contains(&frac), "lit fraction {frac:.3}");
        assert!(hems > 0, "no pink hem at the expansion peak");
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
        let hem = (0..v.w)
            .any(|x| (0..v.rows).any(|y| v.pixels.get(x as i32, y as i32).is_some_and(is_hem)));
        assert!(!hem, "pink hem during growth");
    }

    fn glow(tone: f32, red: f32, hem: bool) -> Glow {
        Glow {
            tone,
            red,
            hem,
            green: 1.0,
        }
    }

    #[test]
    fn aur_a8_light_is_thresholded_into_ramped_levels() {
        // Below its threshold a pixel stays dark; at it the level starts from the core
        // floor `ink::MIN_VISIBLE_LEVEL` (CORE-R66); above it the level ramps
        // monotonically to the top.
        const FLOOR: u8 = ink::MIN_VISIBLE_LEVEL as u8;
        assert_eq!(glow(0.99 * GREEN_LIT, 0.2, false).ink(), None);
        assert_eq!(
            glow(GREEN_LIT, 0.2, false).ink(),
            Some(Ink::new(GREEN, FLOOR))
        );
        assert_eq!(glow(1.0, 0.2, false).ink(), Some(Ink::new(GREEN, 255)));
        // Red-dominant light has its own, higher threshold and stays a deep crimson
        // (`RED` ≤ RED_LEVEL_MAX); the hem is bright `RED` from HEM_LEVEL.
        const { assert!(RED_LIT > GREEN_LIT) };
        assert_eq!(glow(0.99 * RED_LIT, 0.8, false).ink(), None);
        assert!(glow(0.99 * RED_LIT, 0.4, false).ink().is_some());
        assert_eq!(
            glow(RED_LIT, 0.8, false).ink(),
            Some(Ink::new(RED, RED_LEVEL_MIN as u8))
        );
        // The crown needs the green's tail under it: no red slab over empty sky.
        let bare = Glow {
            green: 0.99 * CROWN_GREEN,
            ..glow(1.0, 0.8, false)
        };
        assert_eq!(bare.ink(), None);
        let green_only = Glow {
            green: 0.0,
            ..glow(1.0, 0.2, false)
        };
        assert!(green_only.ink().is_some());
        assert_eq!(
            glow(1.0, 0.8, false).ink(),
            Some(Ink::new(RED, RED_LEVEL_MAX as u8))
        );
        assert_eq!(
            glow(GREEN_LIT, 0.0, true).ink(),
            Some(Ink::new(RED, HEM_LEVEL as u8))
        );
        // Hem and crown must not blur.
        const { assert!(HEM_LEVEL > RED_LEVEL_MAX + 40.0) };
        for red in [0.1f32, 0.9] {
            let mut prev = 0;
            for i in 0..=100 {
                if let Some(ink) = glow(i as f32 / 100.0, red, false).ink() {
                    assert!(ink.level >= FLOOR, "level {} under the floor", ink.level);
                    assert!(ink.level >= prev, "level falls at tone {i}%");
                    prev = ink.level;
                }
            }
        }
    }

    /// The view's lit emission pixels in rows `0..y1`, row-major.
    fn lit_mask(v: &View, y1: usize) -> Vec<bool> {
        (0..y1)
            .flat_map(|y| (0..v.w).map(move |x| (x, y)))
            .map(|(x, y)| {
                v.pixels
                    .get(x as i32, y as i32)
                    .is_some_and(|i| EMISSION.contains(&i.role))
            })
            .collect()
    }

    /// Sizes of the 8-connected components of `lit` (`w` wide).
    fn components(lit: &[bool], w: usize) -> Vec<usize> {
        let mut seen = vec![false; lit.len()];
        let mut sizes = Vec::new();
        for start in 0..lit.len() {
            if !lit[start] || seen[start] {
                continue;
            }
            seen[start] = true;
            let (mut stack, mut size) = (vec![start], 0);
            while let Some(i) = stack.pop() {
                size += 1;
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx as usize >= w {
                            continue;
                        }
                        let j = ny as usize * w + nx as usize;
                        if j < lit.len() && lit[j] && !seen[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            sizes.push(size);
        }
        sizes
    }

    #[test]
    fn aur_a8_curtains_are_coherent_forms_not_confetti() {
        // Visit times across the whole loop: growth, breakup, expansion, peak, the
        // lingering crown, recovery with patches, the late quiet arc. Pixels in specks
        // (connected groups of ≤ 3) are at most 3 % of the lit sky: the light forms
        // arcs, curtains and patches, not scatter.
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let (mut a, mut ctx) = scene(w, h);
            for visit in [0.0, 5.0, 12.0, 20.0, 26.0, 34.0, 42.0] {
                while a.t < START_T + visit - 1e-9 {
                    a.update(1.0 / 60.0, &mut ctx);
                }
                let v = &a.view;
                let sizes = components(&lit_mask(v, v.y_h), v.w);
                let n: usize = sizes.iter().sum();
                let specks: usize = sizes.iter().filter(|&&s| s <= 3).sum();
                assert!(n > 0, "{w}x{h} t = {visit}: dark sky");
                assert!(
                    specks * 100 <= 3 * n,
                    "{w}x{h} t = {visit}: {specks} speck pixels of {n} lit"
                );
            }
        }
    }

    #[test]
    fn aur_a8_growth_arc_is_one_unbroken_band() {
        // A quiet arc is homogeneous: at 80×24 and 200×50 at scene start, every column
        // over the frame's middle third is lit, and the arch's lower border (the frame's
        // focal edge) steps by at most 2 px between neighbouring columns.
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let (a, _) = scene(w, h);
            let v = &a.view;
            let lit = lit_mask(v, v.y_h);
            let mut prev: Option<usize> = None;
            for x in v.w / 3..2 * v.w / 3 {
                let top = (0..v.y_h).find(|&y| lit[y * v.w + x]);
                let Some(top) = top else {
                    panic!("{w}x{h}: gap in the arc at column {x}");
                };
                // The lowest lit pixel of the first run is the arc's lower border.
                let low = (top..v.y_h)
                    .take_while(|&y| lit[y * v.w + x])
                    .last()
                    .unwrap_or(top);
                if let Some(p) = prev {
                    assert!(
                        p.abs_diff(low) <= 2,
                        "{w}x{h}: border jumps at {x}: {p} → {low}"
                    );
                }
                prev = Some(low);
            }
        }
    }

    #[test]
    fn aur_a8_pulsating_patches_read_as_patches() {
        // The display floor is frame-relative: 5 × 3 px at 80×24 and 200×50, but at most
        // 7 % of the width and 1/12 of the pixel rows, so a small frame's patches never
        // swell into clouds half the sky wide.
        assert_eq!(patch_floor_px(80, 48), (5.0, 3.0));
        assert_eq!(patch_floor_px(200, 100), (5.0, 3.0));
        let (fa, fe) = patch_floor_px(40, 24);
        assert!((fa - 2.8).abs() < 1e-12 && (fe - 2.0).abs() < 1e-12);
        // Azimuths are stratified over ±45°: each patch stays within 3° of its
        // stratum's centre (15° apart), so neighbours (all drifting at 0.5°/s) keep apart.
        for seed in 0..16 {
            let mut ctx = Ctx::new(seed, Size::new(80, 24));
            let setting = Setting::seed(&mut ctx.rng);
            for (i, p) in setting.patches.iter().enumerate() {
                let centre = -45.0 + 90.0 * (i as f64 + 0.5) / PATCHES as f64;
                let a = p.alpha.to_degrees();
                assert!(
                    (a - centre).abs() <= 3.0 + 1e-9,
                    "seed {seed}: patch {i} at {a:.1}°"
                );
            }
        }
        // Recovery at 80×24 and 40×12: every live patch's half-extents are its seeded
        // ones floored at `patch_floor_px`, and at full pulse it lights a solid core
        // about its centre (5 × 3 px at 80×24).
        for (w, h) in [(80u16, 24u16), (40, 12)] {
            let (mut a, mut ctx) = scene(w, h);
            let mut seen = 0;
            while a.t < START_T + 44.0 {
                a.update(0.25, &mut ctx);
                let v = &a.view;
                let (fa, fe) = patch_floor_px(v.w, v.rows);
                for p in &a.patches[..a.live_patches] {
                    let (ha, he) = (p.half_a * v.f, p.half_e * v.f);
                    assert!(
                        ha >= fa - 1e-9 && he >= fe - 1e-9,
                        "{w}x{h}: {ha:.2} × {he:.2}"
                    );
                    assert!(
                        ha <= (6f64.to_radians() * v.f).max(fa) + 1e-9
                            && he <= (3f64.to_radians() * v.f).max(fe) + 1e-9,
                        "{w}x{h}: patch swollen to {ha:.2} × {he:.2} px"
                    );
                    if p.gain < 0.95 * PATCH_GAIN {
                        continue;
                    }
                    let cx = (v.w as f64 / 2.0 + v.f * p.alpha).round() as i32;
                    let cy = v.row_of(p.e).round() as i32;
                    if !(0..v.w as i32).contains(&cx) || !(0..v.y_h as i32).contains(&cy) {
                        continue;
                    }
                    let lit = |x: i32, y: i32| v.pixels.get(x, y).is_some_and(|i| i.role == GREEN);
                    let (kx, ky) = ((fa / 2.5).floor() as i32, (fe / 3.0).floor() as i32);
                    let core = (-kx..=kx).all(|dx| (-ky..=ky).all(|dy| lit(cx + dx, cy + dy)));
                    assert!(
                        core,
                        "{w}x{h} t = {:.2}: patch at {cx},{cy} is not solid",
                        a.t
                    );
                    seen += 1;
                }
            }
            assert!(seen > 0, "{w}x{h}: no patch at full pulse during recovery");
        }
    }

    #[test]
    fn aur_a8_patches_stay_dimmer_than_the_recovery_arc() {
        // Early recovery (visit t = 32–36 s): a patch at full pulse peaks below the
        // brightest curtain light, so the arc stays the focal point.
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let (mut a, mut ctx) = scene(w, h);
            for visit in [32.0, 34.0, 36.0] {
                while a.t < START_T + visit - 1e-9 {
                    a.update(1.0 / 60.0, &mut ctx);
                }
                let v = &a.view;
                let live = &a.patches[..a.live_patches];
                // Pixels outside every patch's support are curtain light.
                let in_patch = |x: usize, y: usize| {
                    let al = v.alpha_at(x as f64 + 0.5);
                    let e = (v.y_h as f64 - y as f64 - 0.5) / v.f;
                    live.iter().any(|p| {
                        ((al - p.alpha) / p.half_a).abs() < PATCH_REACH
                            && ((e - p.e) / p.half_e).abs() < PATCH_REACH
                    })
                };
                let level = |x: usize, y: usize| {
                    v.pixels
                        .get(x as i32, y as i32)
                        .filter(|i| i.role == GREEN)
                        .map_or(0, |i| i.level)
                };
                let arc = (0..v.y_h)
                    .flat_map(|y| (0..v.w).map(move |x| (x, y)))
                    .filter(|&(x, y)| !in_patch(x, y))
                    .map(|(x, y)| level(x, y))
                    .max()
                    .unwrap_or(0);
                for p in live.iter().filter(|p| p.gain >= 0.95 * PATCH_GAIN) {
                    let cx = (v.w as f64 / 2.0 + v.f * p.alpha).floor();
                    let cy = v.row_of(p.e).round();
                    if cx < 0.0 || cx >= v.w as f64 || cy < 0.0 || cy >= v.y_h as f64 {
                        continue;
                    }
                    let core = level(cx as usize, cy as usize);
                    assert!(core < arc, "{w}x{h} t = {visit}: patch {core} vs arc {arc}");
                }
            }
        }
    }

    /// Curtain-only green tone at sky pixel (x, y), summed from the column's layers.
    fn curtain_green_tone(a: &Aurora, x: usize, y: usize) -> f32 {
        let v = &a.view;
        let g: f32 = v.layers[x][..usize::from(v.nlayers[x])]
            .iter()
            .filter(|l| (l.y_top..=l.y_low).contains(&(y as i32)))
            .map(|l| l.green * a.luts.green(l.h_step, l.altitude(v.tan_e[y]) - l.zb))
            .sum();
        a.luts.tone(v.ext[y] * (1.0 / E_SAT as f32) * g)
    }

    #[test]
    fn aur_a8_pulsating_patches_raise_no_red_hood() {
        // Pulsating aurora has no red cap: patch light never passes the crown's green
        // gate. At 200×50, visit t = 28 s (the lingering red dome over the first
        // patches), no crown pixel lies within 2 px of a live patch's ellipse unless
        // curtain green is lit within 12 px below it in the same column (also at 26 and
        // 27 s, and for the scene seed and the review seed 42).
        for seed in [7, 42] {
            let (mut a, mut ctx) = scene_seeded(200, 50, seed);
            for visit in [26.0, 27.0, 28.0] {
                while a.t < START_T + visit - 1e-9 {
                    a.update(1.0 / 60.0, &mut ctx);
                }
                assert_no_hood(&a, seed, visit);
            }
        }
    }

    fn assert_no_hood(a: &Aurora, seed: u64, visit: f64) {
        let v = &a.view;
        let live = &a.patches[..a.live_patches];
        assert!(
            visit < 28.0 || !live.is_empty(),
            "no live patch at visit t = 28 s"
        );
        let (mut near, mut hooded) = (0, 0);
        for x in 0..v.w {
            for y in 0..v.y_h {
                let crown = v
                    .pixels
                    .get(x as i32, y as i32)
                    .is_some_and(|i| i.role == RED && !is_hem(i))
                    && !is_fringe(v, x as i32, y as i32);
                if !crown {
                    continue;
                }
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let by_patch = live.iter().any(|p| {
                    let cx = v.w as f64 / 2.0 + v.f * p.alpha;
                    let cy = v.row_of(p.e) + 0.5;
                    let (ha, he) = (p.half_a * v.f + 2.0, p.half_e * v.f + 2.0);
                    ((px - cx) / ha).powi(2) + ((py - cy) / he).powi(2) <= 1.0
                });
                if !by_patch {
                    continue;
                }
                near += 1;
                // As for the crown (AUR-A8): curtain green lit within 12 px below.
                let capped =
                    (y + 1..(y + 12).min(v.y_h)).any(|g| curtain_green_tone(a, x, g) >= GREEN_LIT);
                hooded += usize::from(!capped);
            }
        }
        assert_eq!(
            hooded, 0,
            "seed {seed} t = {visit}: {hooded} of {near} crown pixels by a patch hood it"
        );
    }

    #[test]
    fn aur_a8_spruce_ridge_stands_black_against_the_glow() {
        for (w, h) in [(80u16, 24u16), (200, 50), (40, 12)] {
            let (a, _) = scene(w, h);
            let v = &a.view;
            // Spires rise at least SPIRE_MIN_PX over the bare ridge somewhere, even
            // where 3° is under 2 px.
            let rise = (0..v.w)
                .map(|x| v.ridge_base[x] - v.ridge_top[x])
                .max()
                .unwrap_or(0);
            assert!(
                f64::from(rise) >= SPIRE_MIN_PX - 1.0,
                "{w}x{h}: spires rise {rise} px"
            );
            // The glow stands clear of the tallest silhouette, so each spire is cut out
            // of it; the lake mirrors the silhouette as black.
            let tallest = (0..v.w).map(|x| v.y_h as i32 - v.ridge_top[x]).max();
            let tallest = f64::from(tallest.unwrap_or(0));
            assert!(v.glow_px() >= tallest + GLOW_CLEAR_PX - 1e-9, "{w}x{h}");
            for x in 0..v.w {
                let top = v.ridge_top[x];
                assert!(
                    top <= 0
                        || v.pixels
                            .get(x as i32, top - 1)
                            .is_some_and(|i| i.role != Role::Background),
                    "{w}x{h}: no glow above the silhouette at column {x}"
                );
                if v.lake {
                    // Silhouette row y mirrors to lake row 2·y_h − 1 − y.
                    for y in top.max(0)..v.y_h as i32 {
                        let m = 2 * v.y_h as i32 - 1 - y;
                        let ink = v.pixels.get(x as i32, m).unwrap_or(Ink::BLANK);
                        assert!(
                            ink.role != SKYGLOW,
                            "{w}x{h}: glow in the ridge's reflection at {x},{m}"
                        );
                    }
                }
            }
        }
    }

    /// The brightest level of the vertical run of emission pixels through sky pixel
    /// (x, y), or `None` if it is not emission.
    fn run_peak(v: &View, x: usize, y: usize) -> Option<u8> {
        let em = |y: usize| {
            v.pixels
                .get(x as i32, y as i32)
                .filter(|i| EMISSION.contains(&i.role))
                .map(|i| i.level)
        };
        em(y)?;
        let up = (0..=y).rev().map_while(em);
        let down = (y + 1..v.y_h).map_while(em);
        up.chain(down).max()
    }

    #[test]
    fn aur_a10_the_hem_blends_up_the_lake_mirrors_no_red_and_the_curtains_ripple() {
        let mut blended = 0;
        for steps in [20, 32, 48] {
            let (a, _) = census(steps);
            let v = &a.view;
            let px = |x: i32, y: i32| v.pixels.get(x, y).unwrap_or(Ink::BLANK);
            for y in 1..v.y_h as i32 {
                for x in 0..v.w as i32 {
                    if !is_hem(px(x, y)) {
                        continue;
                    }
                    // Over a hem pixel: never green straight away (the rose blend, another
                    // hem pixel, or dark sky); under it only the faint crimson fade.
                    let above = px(x, y - 1);
                    assert_ne!(above.role, GREEN, "hard hem edge at {x},{y}");
                    blended += usize::from(above == Ink::new(RED, HEM_BLEND[0]));
                    let below = px(x, y + 1);
                    assert!(
                        below.role != RED || is_hem(below) || below.level <= HEM_BELOW_FADE[0],
                        "bright red below {x},{y}"
                    );
                }
            }
            // The water mirrors no red.
            for y in v.y_h..v.lake_end() {
                for x in 0..v.w {
                    assert_ne!(
                        px(x as i32, y as i32).role,
                        RED,
                        "red in the lake at {x},{y}"
                    );
                }
            }
        }
        assert!(blended > 5, "{blended} hem pixels blend");
        assert!(HEM_BLEND.windows(2).all(|w| w[0] > w[1]));
        assert!(f64::from(HEM_BLEND[0]) <= RED_LEVEL_MAX);
        // Continuous: wherever the pink curtain's lower border is lit, its column carries
        // the fringe; no dashes along a lit border.
        for steps in [44, 52, 56] {
            let (a, _) = census(steps);
            let v = &a.view;
            let (mut lit_cols, mut hem_cols) = (0, 0);
            for x in 0..v.w {
                let layers = &v.layers[x][..usize::from(v.nlayers[x])];
                let Some(lowest) = layers
                    .iter()
                    .filter(|l| l.pink > 0.0)
                    .map(|l| l.y_low)
                    .max()
                else {
                    continue;
                };
                let band = (lowest - 4).max(0)..=lowest.min(v.y_h as i32 - 1);
                let at = |y: i32| v.pixels.get(x as i32, y).unwrap_or(Ink::BLANK);
                if band.clone().any(|y| at(y).role == GREEN || is_hem(at(y))) {
                    lit_cols += 1;
                    hem_cols += usize::from(band.clone().any(|y| is_hem(at(y))));
                }
            }
            assert!(lit_cols > 10, "t = {}: {lit_cols} lit border columns", a.t);
            assert!(
                hem_cols * 10 >= lit_cols * 9,
                "t = {}: fringe in {hem_cols} of {lit_cols} lit border columns",
                a.t
            );
        }
        // The ripple is a bounded wave that travels.
        let (mut a, _) = census(20);
        a.view.set_ripple(1.0);
        let r1 = a.view.ripple.clone();
        a.view.set_ripple(3.0);
        assert_ne!(r1, a.view.ripple);
        let amp = RIPPLE_AMP as f32 + 1e-6;
        assert!(r1.iter().all(|&g| (1.0 - amp..=1.0 + amp).contains(&g)));
    }

    #[test]
    fn aur_a7_lake_mirrors_the_lit_sky_as_broken_streaks() {
        // Visit t = 12, 14 (the AUR-A7 frame), 20 and 28 s.
        for steps in [48, 56, 80, 112] {
            let (a, _) = census(steps);
            let v = &a.view;
            let px = |x: usize, y: usize| v.pixels.get(x as i32, y as i32).unwrap_or(Ink::BLANK);
            let (mut lake_lit, mut sky_lit, mut rows) = (0usize, 0usize, 0);
            let (mut glow_lake, mut glow_sky) = (0usize, 0usize);
            // What the water mirrors: the skyglow, and every pixel of a sky run whose
            // brightest pixel reaches LAKE_MIN_LEVEL, so a run is reflected whole or
            // not at all and the lake shows streaks rather than shards.
            let mirrored = |x: usize, y: usize| {
                px(x, y).role == SKYGLOW
                    || (px(x, y).role == GREEN
                        && run_peak(v, x, y).is_some_and(|peak| peak >= LAKE_MIN_LEVEL))
            };
            // The two bottom cell rows stay dark for the title (GEN-R13).
            assert_eq!(v.lake_end(), v.rows - 4);
            for y in v.lake_end()..v.rows {
                assert!((0..v.w).all(|x| px(x, y) == Ink::BLANK), "row {y}");
            }
            let depth = (v.lake_end() - v.y_h) as f32;
            for y in v.y_h..v.lake_end() {
                let d = (y - v.y_h + 1) as f64;
                let src = v.y_h - (y - v.y_h) - 1;
                // No row drops out: a crest only dims it.
                let gain = lake_row_gain(
                    (y - v.y_h) as f32 / depth,
                    (0.9 * d - 1.7 * a.t).sin_det() as f32,
                );
                rows += 1;
                for x in 0..v.w {
                    // Emission and mirrored skyglow are counted apart: the skyglow's
                    // copy is broken by dark glints (AUR-A10).
                    let skyglow = px(x, src).role == SKYGLOW;
                    if skyglow {
                        glow_sky += 1;
                    } else {
                        sky_lit += usize::from(mirrored(x, src));
                    }
                    let ink = px(x, y);
                    if ink == Ink::BLANK {
                        continue;
                    }
                    if ink.role == SKYGLOW {
                        glow_lake += 1;
                    } else {
                        lake_lit += 1;
                    }
                    let gain = gain * shimmer(x, y, y - v.y_h, a.t);
                    // Each lit lake pixel is a dimmed, box-averaged copy (never under
                    // the core floor) of a mirrored sky pixel a ripple shift away, with
                    // its role (so red over green reflects as red over green), and never
                    // brighter than `LAKE_GAIN` of the sky it mirrors.
                    let copy = (x.saturating_sub(3)..(x + 4).min(v.w))
                        .any(|sx| mirrored(sx, src) && v.lake_ink(sx, src, gain) == Some(ink));
                    let peak = (x.saturating_sub(4)..(x + 5).min(v.w))
                        .flat_map(|sx| [px(sx, src).level, px(sx, src.saturating_sub(1)).level])
                        .max()
                        .unwrap_or(0);
                    assert!(
                        f32::from(ink.level)
                            <= (LAKE_GAIN * gain * f32::from(peak))
                                .max(ink::MIN_VISIBLE_LEVEL as f32)
                                + 1.0
                    );
                    assert!(
                        copy,
                        "t = {}: lake pixel {x},{y} {ink:?} mirrors nothing",
                        a.t
                    );
                }
            }
            assert!(rows > 0);
            // Lit-ness follows the sky, so the reflection is as dense as its source rows.
            let ratio = lake_lit as f64 / sky_lit as f64;
            assert!(
                (0.9..=1.1).contains(&ratio),
                "t = {}: lake/sky lit {ratio:.3}",
                a.t
            );
            // The mirrored skyglow ripples: about 40 % of it is dark glints (AUR-A10).
            if glow_sky > 200 {
                let glow = glow_lake as f64 / glow_sky as f64;
                assert!(
                    (0.4..=0.8).contains(&glow),
                    "t = {}: skyglow copy {glow:.3}",
                    a.t
                );
            }
        }
    }

    #[test]
    fn aur_a7_lake_pixel_is_the_dimmed_box_mean_of_its_source() {
        // Hand-set sky: a green run three pixels wide on a sky row and the row above it,
        // with empty sky either side. The copy of its middle pixel is
        // round(LAKE_GAIN · gain · (sum of the six) / 6); one end loses the three pixels
        // past the run (soft ends), and deeper rows and ripple crests dim it.
        let (mut a, _) = census(56);
        let v = &mut a.view;
        // A row that `run_peaks` scans (only the emitting rows can hold emission).
        let src = v.emit_rows.0 + 1;
        assert!(src < v.emit_rows.1.min(v.y_h - 1));
        let x0 = v.w / 2;
        let levels = [[80u8, 90, 100], [100, 120, 140]];
        for (row, y) in [src - 1, src].into_iter().enumerate() {
            v.pixels.set(x0 as i32 - 2, y as i32, Ink::BLANK);
            v.pixels.set(x0 as i32 + 2, y as i32, Ink::BLANK);
            for (i, &l) in levels[row].iter().enumerate() {
                v.pixels
                    .set((x0 + i) as i32 - 1, y as i32, Ink::new(GREEN, l));
            }
        }
        v.run_peaks();
        let sum: f32 = levels.iter().flatten().map(|&l| f32::from(l)).sum();
        let want = |gain: f32, sum: f32| {
            Some(Ink::new(
                GREEN,
                (LAKE_GAIN * gain * sum / 6.0).round() as u8,
            ))
        };
        assert_eq!(v.lake_ink(x0, src, 1.0), want(1.0, sum));
        assert_eq!(
            v.lake_ink(x0 + 1, src, 1.0),
            want(1.0, 90.0 + 100.0 + 120.0 + 140.0)
        );
        assert_eq!(v.lake_ink(x0, src, 0.5), want(0.5, sum));
        // The row gain: full at the shore between crests, fading with depth, dimmed on
        // a crest, and never zero.
        assert_eq!(lake_row_gain(0.0, -0.3), 1.0);
        assert!(lake_row_gain(0.9, 0.0) < lake_row_gain(0.2, 0.0));
        assert!((lake_row_gain(0.0, 1.0) - (1.0 - CREST_DIM)).abs() < 1e-6);
        assert!(lake_row_gain(1.0, 1.0) > 0.2);
    }

    #[test]
    fn aur_a7_hem_runs_only_along_the_brightest_curtains_lowest_border() {
        for steps in [44, 52, 56] {
            let (a, _) = census(steps);
            let v = &a.view;
            let accent = |x: i32, y: i32| v.pixels.get(x, y).is_some_and(is_hem);
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
                    // Crisp: rose pink, never the crown's deep crimson nor the ramp's
                    // near-white top.
                    let level = f64::from(v.pixels.get(x as i32, y).map_or(0, |i| i.level));
                    assert!(level >= HEM_LEVEL - 0.5, "hem level {level}");
                    assert!(level <= HEM_LEVEL_MAX + 0.5, "hem level {level}");
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
    fn aur_skyglow_is_solid_and_fades_to_near_black_above_the_silhouette() {
        assert_eq!(skyglow(9.0, 9.0), None);
        assert_eq!(skyglow(0.0, 9.0), Some(ink::ramp(GLOW_RAMP, GLOW_MAX)));
        let mut prev = 1.0;
        for i in 0..90 {
            let px = 0.1 * f64::from(i);
            let ink = skyglow(px, 9.0).expect("solid up to its height");
            let v = f64::from(ink.level) / 255.0;
            assert!(v <= prev, "skyglow brightens with elevation at {px}");
            assert!(
                v >= GLOW_MIN - 0.5 / 255.0,
                "glow carried into black at {px}"
            );
            prev = v;
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
                    // Every dark sky pixel in the glow glows: no holes, no dither.
                    let glow = v.skyglow[y as usize].unwrap_or(Ink::BLANK);
                    assert_eq!(ink, glow, "{w}x{h} sky pixel {x},{y}");
                    glowing += usize::from(ink.role == SKYGLOW);
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
        let glow_px = v.glow_px();
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
                .blit(&mut canvas, |_, cy, t, b| a.ascii_cell(cy, t, b));
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
    fn aur_a9_captions_follow_the_beat_and_the_label_is_al_only() {
        // Loop times 1, 13, 20 and 40 s fall in growth, onset, expansion and recovery.
        for (t, caption) in [1.0, 13.0, 20.0, 40.0].into_iter().zip(CAPTIONS) {
            let (mut a, mut ctx) = scene(80, 24);
            let steps = ((t + LOOP - START_T) % LOOP / 0.25).round() as usize;
            for _ in 0..steps {
                a.update(0.25, &mut ctx);
            }
            assert_eq!(a.caption(), Some(caption), "t = {t}");
            assert!(META.captions.contains(&caption));
            let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Ascii);
            a.render(&mut canvas);
            let top = &canvas.to_lines()[0];
            assert!(top.starts_with(" AL -"), "{top}");
            // A blank gutter cell right after the label, and no beat word.
            let end = top.find(" nT").map(|i| i + " nT".len());
            assert_eq!(
                end.and_then(|e| top.as_bytes().get(e)),
                Some(&b' '),
                "{top}"
            );
            for &(beat, _) in &BEATS {
                assert!(!top.contains(beat), "{top}");
            }
        }
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
                    .map(|i| i.role == RED && !is_hem(i) && !is_fringe(v, x as i32, y as i32))
            })
            .collect()
    }

    /// Whether red pixel `(x, y)` belongs to the hem's fringe (AUR-A10), not the crown: a
    /// hem pixel lies up to `HEM_BLEND.len()` rows below it or `HEM_BELOW_FADE.len()` above.
    fn is_fringe(v: &View, x: i32, y: i32) -> bool {
        let hem = |y: i32| v.pixels.get(x, y).is_some_and(is_hem);
        (1..=HEM_BLEND.len() as i32).any(|k| hem(y + k))
            || (1..=HEM_BELOW_FADE.len() as i32).any(|k| hem(y - k))
    }

    #[test]
    fn aur_a8_red_crown_caps_the_curtains_not_a_box() {
        // The substorm peak, visit t = 14–22 s (scene t = 22–30 s), at 200×50.
        let (mut a, mut ctx) = scene(200, 50);
        for visit in [14.0, 18.0, 22.0] {
            while a.t < START_T + visit - 1e-9 {
                a.update(1.0 / 60.0, &mut ctx);
            }
            let v = &a.view;
            let mask = red_mask(v);
            let (mut red, mut capped) = (0usize, 0usize);
            let mut tops = Vec::new();
            for x in 0..v.w {
                let mut top = None;
                for y in 0..v.y_h {
                    if mask[y * v.w + x] != Some(true) {
                        continue;
                    }
                    red += 1;
                    top.get_or_insert(y);
                    // Green light stands below it in the same column: a cap, not a slab.
                    capped += usize::from((y + 1..(y + 12).min(v.y_h)).any(|g| {
                        v.pixels
                            .get(x as i32, g as i32)
                            .is_some_and(|i| i.role == GREEN)
                    }));
                }
                tops.extend(top);
            }
            assert!(red >= 100, "t = {visit}: only {red} crown pixels");
            assert!(
                capped * 100 >= 95 * red,
                "t = {visit}: {capped} of {red} red pixels cap green"
            );
            // Its top edge follows the rays, not a lid.
            let (lo, hi) = (tops.iter().min().copied(), tops.iter().max().copied());
            assert!(
                hi.zip(lo).is_some_and(|(h, l)| h - l >= 6),
                "t = {visit}: flat crown {lo:?}..{hi:?}"
            );
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
                    GREEN => {
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
