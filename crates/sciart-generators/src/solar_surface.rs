//! The granulated Sun turning slowly, with sunspots, limb loops and coronal rain
//! (SPEC-002 `solar_surface`).
//!
//! A composite view: the photosphere in white light (linear limb darkening, weighted
//! Worley granulation, bipolar spot groups with Wilson-depressed umbrae and faculae) and,
//! as in H-α, a chromospheric rim with spicules and a sheared arcade of cool dipole loops
//! that glow above the limb and absorb as filaments on the disc. Every surface feature rides the
//! Snodgrass–Ulrich differential rotation at 0.3 day per second; granules and coronal
//! rain run on display time. Heliographic state survives any resize; the view (disc
//! normals, rim annulus, granule spatial hash) is rebuilt, and the disc is rasterised in
//! `update`/`resize` so `render` only blits.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::v3::{axpy, cross, dot, norm};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{draw, geom, ink};

pub static META: Meta = Meta {
    name: "solar_surface",
    category: Category::SunPlanets,
    description: "The granulated Sun turning slowly, with sunspots, limb loops and coronal rain",
    title: "Solar surface · convection cells",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SolarSurface::new())
}

/// Solar days per simulated second.
const DAYS_PER_SEC: f64 = 0.3;
/// Linear limb-darkening coefficient in the visible continuum.
const LIMB_U: f64 = 0.6;
/// Sidereal differential rotation `A + B sin²φ + C sin⁴φ` (°/day), Snodgrass & Ulrich 1990.
const ROT: [f64; 3] = [14.713, -2.396, -1.787];
/// Granule spacing at disc centre: `0.085 R` clamped to [3.6, 5] px, so a cell spans
/// 2–3 terminal cells and lanes read as polygons rather than a blocky maze. At R = 46
/// that is 3.9 px (59 Mm), ≈ 45× the real 1.3 Mm, which would be sub-pixel.
const GRANULE_PER_R: f64 = 0.085;
const GRANULE_PX: (f64, f64) = (3.6, 5.0);
/// Worley candidates are gathered within this many spacings: F2 − F1 only shapes the
/// lanes below 0.3 s_g, and F1 rarely exceeds 1.2 s_g.
const WORLEY_REACH: f64 = 1.5;
const GRANULES_MIN: usize = 64;
const GRANULES_MAX: usize = 2400;
const GRANULE_LIFE: (f64, f64) = (6.0, 10.0);
const GRANULE_FADE: f64 = 1.5;
/// Below this radius (px) granulation and loops are unresolvable and skipped.
const R_MIN_DETAIL: f64 = 4.0;
/// Granulation contrast at disc centre: the real rms ≈ 0.13 in the visible, set just
/// under it so granule cores top out below the ramp's clip and the mottle stays a fine
/// texture under the spots and loops rather than a cracked-tile mosaic.
const GRANULE_CONTRAST: f64 = 0.12;

/// Wilson depression in units of the penumbral radius: ≈ 2.5× the real d/ρ ≈ 0.04
/// (0.4–0.8 Mm against ≈ 15 Mm). Spots are drawn 2–3× oversize, so in absolute depth
/// it is 5–7 Mm, ≈ 10× real. The shift reads without swallowing the penumbra.
const WILSON: f64 = 0.1;
/// Display floor on the penumbral radius (px): at 80×24 (R ≈ 22 px) a 2.5° following
/// spot is ≈ 1 px in penumbral radius and its umbra under half a pixel, so small discs
/// draw spots at least this large (an umbra ≥ 0.9 px), but never beyond
/// `sep/SPOT_SEP_CAP`, so a pair's blended penumbrae never touch (SUN-A6). At 200×50
/// (R = 46 px) the floor is 2.49°, under every spot's own ρ_p, so no spot is enlarged.
const SPOT_MIN_PX: f64 = 2.0;
/// `2 × 1.07` plus a margin: two spots floored to `sep/SPOT_SEP_CAP` keep their
/// blended penumbrae (1.07 ρ each) strictly apart even on the smallest discs.
const SPOT_SEP_CAP: f64 = 2.2;
/// Mean penumbral intensity (observed ≈ 0.75–0.85 in the visible): set a little darker so
/// that, with the granulation gated out, the ring still stands about one ramp step
/// below the intergranular lanes (0.88) at 80×24 instead of merging into them.
const PENUMBRA: f64 = 0.70;
/// Penumbral filament pitch (px of circumference) and the filament count range.
const PENUMBRA_FILAMENT_PX: f64 = 3.0;
const PENUMBRA_FILAMENTS: (f64, f64) = (6.0, 24.0);

const REGIONS: usize = 3;
/// Spot half-separation δ (deg) and penumbral radii ρ_p (deg) of the leading and
/// following spots.
const SPOT_HALF_SEP_DEG: (f64, f64) = (6.5, 7.5);
const LEAD_RHO_DEG: (f64, f64) = (4.0, 6.0);
const FOLLOW_RHO_DEG: (f64, f64) = (2.5, 4.0);
const LOOPS: usize = 3;
const LOOP_SAMPLES: usize = 56;
const KNOTS: usize = 2;
const KNOT_WIDTH: f64 = 0.05;
/// Coronal rain: a knot condenses within this distance of the apex (in σ) ...
const RAIN_BIRTH: f64 = 0.1;
/// ... and falls down one leg, its distance `a = |σ − ½|` from the apex growing as
/// `ȧ = (0.3 + 1.4a)/T`: slow near the top, accelerating down the leg.
const RAIN_V0: f64 = 0.3;
const RAIN_ACCEL: f64 = 1.4;
/// Knots fade in after condensing and out before reaching the footpoint, over this σ.
const KNOT_FADE: f64 = 0.05;
/// Tallest-loop apex heights (R☉) of new regions; the first region's is the maximum.
/// 0.20–0.32 R☉ is 139–223 Mm, ≈ 1–1.5× the tallest observed cool H-α and
/// coronal-rain loops (≲ 150 Mm): a stated display exaggeration, like the granules and
/// the Wilson shift, so the arcade reads as the focal point at 80×24 (≈ 7 px, 3.5 rows)
/// and still fits the empty columns beside the disc.
const LOOP_H: (f64, f64) = (0.20, 0.32);
/// Apex heights of the arcade's nested field lines relative to its tallest: the lowest
/// still clears `MIN_LOOP_PX` for the first region at 80×24, and neighbours stand
/// ≥ 1 px apart at the apex.
const NEST: [f64; LOOPS] = [1.0, 0.72, 0.48];
/// Shear (deg): the arcade's loops are turned this far from the spot axis (the
/// potential field's direction), a moderately sheared arcade. That is a display
/// compromise: the plane stays inclined to the line of sight at either limb, so the
/// loops show as nested arches rather than edge-on spikes. Real filament channels are
/// sheared ≳ 70°, their field within ≈ 20° of the polarity inversion line.
const SHEAR_DEG: (f64, f64) = (45.0, 65.0);
/// Twist: each field line is bowed out of the arcade's plane by up to this share of its
/// half-span, `ARCADE_TWIST·half·sin(π·a/half)` at `a` along it, one way on one leg and
/// the other on the other, so from above the arcade and its filament form an S (a
/// sigmoid, as twisted filament channels do) instead of a straight rod.
const ARCADE_TWIST: f64 = 0.1;
/// Filaments: the arcade's tallest loop darkens the disc only where it stands above this
/// share of its apex height. The low legs would trace a ring around the spots; the
/// elevated arch is one short dark thread, 1 px wide, along the loop plane, 25–45° off
/// the inversion line (a real filament lies along it; see `SHEAR_DEG`). The nested
/// arches below it would thicken the thread into a bar.
const FILAMENT_RISE: f64 = 0.35;
/// Loops with a lower apex (px) are not drawn.
const MIN_LOOP_PX: f64 = 2.5;
/// On discs smaller than this (px; 80×24 is 22) the middle arch is dropped: the three
/// would fuse into one orange blob, while the outer and inner arches keep a black gap.
const THIN_ARCADE_R: f64 = 30.0;
/// Relative longitude past which a fully hidden region respawns, and the respawn step.
const RESPAWN_LON: f64 = 150.0 * PI / 180.0;
const RESPAWN_STEP: f64 = 60.0 * PI / 180.0;

/// Chromosphere: a 0.8 px rim and spicules up to `spicule_px(R)` above it, all within
/// 2.6 px.
const RIM_PX: f64 = 0.8;
/// Spicule height: `SPICULE_PER_R·R`, at most `SPICULE_PX` (reached at R ≥ 46 px). Real
/// spicules stand ≈ 10 Mm, 0.014 R☉; fixed in pixels they would be 15 % of a small disc's
/// radius and break its limb into stray pixels and notches.
const SPICULE_PER_R: f64 = 0.035;
const SPICULE_PX: f64 = 1.6;
const ANNULUS_PX: f64 = 2.6;
/// Outward falloff exponent of the chromosphere's glow above the rim (SUN-A10).
const RIM_FALLOFF: f64 = 1.3;
/// ASCII rim: the cells holding a pixel within 1 px of the limb. Pixel-centre distances
/// step by < 1 px, so every radius meets one and the ring closes.
const ASCII_RIM_PX: f64 = 1.0;

/// Values of the H-α features (`h_alpha_ink`). Loops sit above the limb (0.43), spicules and rim
/// but below the penumbra (0.67): orange cool plasma, apart from the limb fuzz, rather
/// than a torn-off piece of the photosphere. Only the rain knots, adding up to
/// `V_KNOT` (≈ 0.88, pale yellow), are the moving highlight.
const V_RIM: f64 = 0.55;
const V_SPICULE: (f64, f64) = (0.50, 0.42);
const V_LOOP: f64 = 0.60;
/// The prominence sheet under the outer arch (SUN-A11): its value from foot to arch, the
/// shimmer's amplitude and its angular speed (rad/s) and wavenumber along the loop, so the
/// plasma visibly streams in a deeper crimson than the arches drawn over it.
const V_FILL: (f64, f64) = (0.40, 0.52);
const FILL_SHIMMER: f64 = 0.04;
const FILL_WAVE: (f64, f64) = (2.2, 9.0);
const V_KNOT: f64 = 0.28;
/// ASCII: an off-disc cell is a loop `.` from this value up (above every rim and
/// spicule value) ...
const V_LOOP_ASCII: f64 = 0.5 * (V_RIM + V_LOOP);
/// ... and a knot `o` from this value (the knot core) up.
const V_KNOT_CORE: f64 = V_LOOP + 0.9 * V_KNOT;
/// Filaments absorb to this fraction of the disc intensity behind them: a dark thread
/// against the granulation, still well above the umbrae (0.22).
const FILAMENT: f64 = 0.6;
/// Long-slit spectrograph inset (SUN-A8): 400–700 nm across, the slit's length down,
/// one cell row per slit position; its size in cells, and the clearance (disc radii)
/// every inset pixel keeps from the disc centre, beyond the tallest loops (1.32 R).
const SPECTRUM_NM: (f64, f64) = (400.0, 700.0);
const INSET_W: (usize, usize) = (24, 40);
const INSET_H: (usize, usize) = (4, 8);
const INSET_CLEAR: f64 = 1.35;
/// Hue turns of the violet end (the red end is turn 0 of the `Phase` loop).
const VIOLET_TURNS: f64 = 0.78;
/// Slit rows darker than this (an umbra) are drawn as a dark band.
const SLIT_DARK: f32 = 0.45;
/// Granulation under the slit brighter (darker) than its limb-darkened mean by this share
/// is rising (sinking) gas: its lines shift blueward (redward), a sub-cell wiggle. A
/// shifted row returns to rest only inside `DOPPLER_HOLD`, so a row near the threshold
/// does not flicker between glyphs from frame to frame.
const DOPPLER_SHARE: f32 = 0.02;
const DOPPLER_HOLD: f32 = 0.01;
/// Fraunhofer lines (nm) within the range: G (CH/Fe), H-γ, H-β (F), Mg b, Fe (E), Na D
/// (D1/D2 blend at this resolution), H-α (C). Each is drawn as one thin glyph, a column
/// about 7.5 nm wide at 40 cells: 20–70× its real width.
const FRAUNHOFER_NM: [f64; 7] = [430.8, 434.0, 486.1, 518.4, 527.0, 589.3, 656.3];
/// Absorption lines, the umbra's dark band and the slit's jaws: thin dark glyphs over the
/// colour behind them.
const DARK_INK: Ink = Ink::new(Role::Dim, 0);
/// The slit's jaws: a dark hairline on the slit's column in only the first `JAW_CELLS`
/// cells inside the limb, top and bottom, like the notches of a slit-jaw image. They tie
/// the inset to the column it samples without a seam through the granulation (the disc
/// fills the height at the default size, so they cannot sit outside it).
const JAW_CELLS: i32 = 2;
const SPECTRUM_LABEL: &str = "slit spectrum 400-700nm, false colour";
const SPECTRUM_LABEL_SHORT: &str = "slit spectrum 400-700nm";

/// IAU nominal solar effective temperature (K).
const TEFF_K: u32 = 5772;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

// --- Physics (pure) -----------------------------------------------------------------

/// The slit row's line shift (−1 blueward, 0, +1 redward) for granulation `excess` over
/// its mean, given the row's previous shift: set beyond `±DOPPLER_SHARE`, held until
/// the excess falls inside `±DOPPLER_HOLD` (SUN-A8).
pub(crate) fn doppler_shift(prev: i8, excess: f32) -> i8 {
    if excess > DOPPLER_SHARE || (prev == -1 && excess > DOPPLER_HOLD) {
        -1
    } else if excess < -DOPPLER_SHARE || (prev == 1 && excess < -DOPPLER_HOLD) {
        1
    } else {
        0
    }
}

/// Linear limb darkening `I(μ) = 1 − u(1 − μ)`, u = 0.6 (SUN-A1).
pub(crate) fn limb_intensity(mu: f64) -> f64 {
    1.0 - LIMB_U * (1.0 - mu)
}

/// Sidereal rotation rate (°/day) at latitude `lat_deg` (SUN-A2).
pub(crate) fn omega_deg_per_day(lat_deg: f64) -> f64 {
    let s2 = lat_deg.to_radians().sin_det().powi(2);
    ROT[0] + ROT[1] * s2 + ROT[2] * s2 * s2
}

/// Surface angular speed (rad per simulated second) at latitude `lat` (rad).
fn omega_rad_per_sec(lat: f64) -> f64 {
    omega_deg_per_day(lat.to_degrees()).to_radians() * DAYS_PER_SEC
}

/// The spot profile at normalised distance `q = β/ρ_p` and azimuth `psi` (SUN-A3), with
/// the umbra measured from its own, Wilson-shifted centre: `q_umbra` sets the
/// umbra–penumbra edge and `q` the penumbra–photosphere one. Returns
/// the intensity factor and the photosphere weight (0 in the spot, 1 outside), which
/// gates the faculae. Umbra 0.22 for q ≤ 0.375 (radius 0.45 ρ_p after the blend gives
/// U/(U+P) ≈ 0.2); penumbra `PENUMBRA` ± 0.08 in `filaments` radial filaments.
fn spot_profile(q_umbra: f64, q: f64, psi: f64, filaments: f64) -> (f64, f64) {
    let penumbra = PENUMBRA + 0.08 * (filaments * psi).cos_det();
    let inner = ease::lerp(0.22, penumbra, ease::smoothstep((q_umbra - 0.375) / 0.15));
    let outside = ease::smoothstep((q - 0.93) / 0.14);
    (ease::lerp(inner, 1.0, outside), outside)
}

/// Penumbral filaments of a spot whose penumbral radius spans `rho_px` pixels: one per
/// 3 px of its circumference 2πρ_p (so ≥ 2 px apart at mid-penumbra), 6–24 (SUN-A3).
/// Real penumbrae hold hundreds of sub-arcsecond fibrils; drawn at a fixed 24 they
/// would alias below a pixel and read as noise, so the count follows what the spot can
/// show.
pub(crate) fn penumbra_filaments(rho_px: f64) -> f64 {
    (TAU * rho_px / PENUMBRA_FILAMENT_PX)
        .round()
        .clamp(PENUMBRA_FILAMENTS.0, PENUMBRA_FILAMENTS.1)
}

/// Cosine of the cap about a spot of penumbral radius `rho` outside which it neither
/// darkens nor brightens: its faculae `exp(−(β/3ρ)²)` fall below 1 % by 6.5 ρ.
pub(crate) fn facular_reach(rho: f64) -> f64 {
    (6.5 * rho).cos_det()
}

/// Sky-plane shift of an umbra toward disc centre for a spot at unit vector `s`
/// (observer frame): the Wilson depression, of magnitude `WILSON·ρ_p sin θ` since
/// `|(s_x, s_y)| = sin θ` (SUN-A3). Lifted back onto the sphere it grows as
/// `WILSON·tan θ` in units of ρ_p, the Wilson effect: the centre-side penumbra narrows
/// toward the limb and is covered below μ ≈ 0.2 (≈ 0.08 at the real depth).
pub(crate) fn wilson_offset(rho_p: f64, s: [f64; 3]) -> [f64; 2] {
    [-WILSON * rho_p * s[0], -WILSON * rho_p * s[1]]
}

/// Where a loop sample shows (SUN-A5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sample {
    /// Beyond the limb: bright against the sky.
    Emission,
    /// In front of the disc: dark against it.
    Filament,
    Hidden,
}

/// Classify an observer-frame point (units of R☉) (SUN-A5).
pub(crate) fn classify(p: [f64; 3]) -> Sample {
    if p[0].hypot(p[1]) >= 1.0 {
        Sample::Emission
    } else if p[2] >= 0.0 {
        Sample::Filament
    } else {
        Sample::Hidden
    }
}

/// A point on the field line of a horizontal dipole at depth `d_b` (units of R☉) with
/// line constant `l`, at dipole colatitude `theta`, in the vertical plane through the
/// dipole axis. Local frame: `a` along the dipole axis, `b` across, `c` radially up,
/// with the Sun's centre at the origin (SUN-A4).
fn dipole_point(d_b: f64, l: f64, theta: f64) -> [f64; 3] {
    let (st, ct) = theta.sin_cos_det();
    let r = l * st * st;
    [r * ct, 0.0, 1.0 - d_b + r * st]
}

/// The dipole colatitudes of both footpoints (`|p| = 1`), by bisection on each side of
/// the apex (SUN-A4).
pub(crate) fn footpoint_theta(d_b: f64, l: f64) -> (f64, f64) {
    let radius = |th: f64| norm(dipole_point(d_b, l, th)) - 1.0;
    let bisect = |mut inside: f64, mut outside: f64| {
        for _ in 0..50 {
            let mid = 0.5 * (inside + outside);
            if radius(mid) < 0.0 {
                inside = mid;
            } else {
                outside = mid;
            }
        }
        0.5 * (inside + outside)
    };
    (bisect(0.0, PI / 2.0), bisect(PI, PI / 2.0))
}

/// The 56 samples of a field line between its footpoints, local frame (SUN-A4).
pub(crate) fn dipole_loop(d_b: f64, l: f64) -> [[f64; 3]; LOOP_SAMPLES] {
    let (t0, t1) = footpoint_theta(d_b, l);
    std::array::from_fn(|i| {
        let th = ease::lerp(t0, t1, i as f64 / (LOOP_SAMPLES - 1) as f64);
        dipole_point(d_b, l, th)
    })
}

/// Maximum spicule height (px) above the rim for a disc of radius `r_px`.
pub(crate) fn spicule_px(r_px: f64) -> f64 {
    (SPICULE_PER_R * r_px).min(SPICULE_PX)
}

/// Granule spacing (px) at disc centre for a disc of radius `r_px` (SUN-A7).
pub(crate) fn granule_spacing(r_px: f64) -> f64 {
    (GRANULE_PER_R * r_px).clamp(GRANULE_PX.0, GRANULE_PX.1)
}

/// Granule pool size for a disc of radius `r_px`: `4πR²/s_g²`, clamped (SUN-A7).
pub(crate) fn granule_count(r_px: f64) -> usize {
    let s_g = granule_spacing(r_px);
    let n = (4.0 * PI * r_px * r_px / (s_g * s_g)).round();
    (n.max(0.0) as usize).clamp(GRANULES_MIN, GRANULES_MAX)
}

/// Granulation brightness in [0, 1] from the weighted Worley distances: bright cores
/// (`F1` small, weighted by the nearest cell's life `w1`) and dark lanes where
/// `F2 − F1` vanishes (SUN-A7).
pub(crate) fn granule_pattern(f1: f64, f2: f64, w1: f64, s_g: f64) -> f64 {
    let lane = ease::smoothstep((f2 - f1) / (0.3 * s_g));
    let core = 1.0 - (f1 / (0.9 * s_g)).min(1.0);
    0.55 * lane + 0.45 * w1 * core * core
}

/// Granulation contrast at `μ`: `GRANULE_CONTRAST` at disc centre, falling as √μ and
/// tapering to zero over μ ∈ [0.1, 0.2] so no hard ring shows (SUN-A7).
pub(crate) fn granule_contrast(mu: f64) -> f64 {
    GRANULE_CONTRAST * mu.max(0.0).sqrt() * ease::smoothstep((mu - 0.1) / 0.1)
}

/// The `Ember` value of photospheric intensity `i`, `v = 0.08 + 0.74·i^1.25` (false
/// colour: oxblood umbrae, scarlet penumbrae, an orange-red limb, gold granules). The
/// power spreads the bright end, where the ramp's pale golds sit close together, so
/// intergranular lanes read some 20 % darker than the granules they part.
pub(crate) fn photosphere_value(i: f64) -> f64 {
    let i = i.max(0.0);
    0.08 + 0.74 * i * i.sqrt().sqrt()
}

/// The intensity behind an `Ember` value: the inverse of [`photosphere_value`].
pub(crate) fn photosphere_intensity(v: f64) -> f64 {
    ((v - 0.08) / 0.74).max(0.0).powf(0.8)
}

/// The photosphere's ink for intensity `i`.
pub(crate) fn photosphere_ink(i: f64) -> Ink {
    ink::ramp(Ramp::Ember, photosphere_value(i))
}

/// The ink of an H-α feature of value `v` (rim, spicules, loops, rain): the `HAlpha`
/// ramp, the colour of the 656.3 nm line they are seen in.
fn h_alpha_ink(v: f64) -> Ink {
    ink::ramp(Ramp::HAlpha, v)
}

/// Heliographic unit vector (x west, y north, z toward the observer at λ = 0).
fn sphere(lat: f64, lon: f64) -> [f64; 3] {
    let (sl, cl) = lat.sin_cos_det();
    let (so, co) = lon.sin_cos_det();
    [cl * so, sl, cl * co]
}

/// The observer's view of heliographic longitude `lon` with the disc centre at
/// latitude `b0`: yaw about the rotation axis, then pitch by B0.
fn view_of(lon: f64, b0: f64) -> Projector {
    Camera {
        yaw: lon,
        pitch: b0,
        distance: f64::INFINITY,
    }
    .projector()
}

// --- Heliographic state -------------------------------------------------------------

/// A granulation feature point: it rides the surface and lives 6–10 s.
#[derive(Debug, Clone, Copy)]
struct Granule {
    sin_lat: f64,
    cos_lat: f64,
    lon: f64,
    omega: f64,
    age: f64,
    life: f64,
}

impl Granule {
    /// A point uniform on the sphere; `age` in [0, life) so a fresh pool is steady.
    fn spawn(rng: &mut impl Rng, age_frac: f64) -> Self {
        let sin_lat: f64 = rng.random_range(-1.0..1.0);
        let life = rng.random_range(GRANULE_LIFE.0..GRANULE_LIFE.1);
        Self {
            sin_lat,
            cos_lat: (1.0 - sin_lat * sin_lat).sqrt(),
            lon: rng.random_range(-PI..PI),
            omega: omega_rad_per_sec(sin_lat.asin()),
            age: age_frac * life,
            life,
        }
    }

    /// Advance by `dt`: longitude by the closed form `ω(φ)·dt` (SUN-A2), then age.
    fn advance(&mut self, dt: f64) {
        self.lon = geom::wrap_angle(self.lon + self.omega * dt);
        self.age += dt;
    }

    /// Fade in and out over 1.5 s, so cells grow and shrink instead of popping.
    fn weight(&self) -> f64 {
        ease::smoothstep(self.age / GRANULE_FADE)
            * ease::smoothstep((self.life - self.age) / GRANULE_FADE)
    }
}

#[derive(Debug, Clone, Copy)]
struct Spot {
    /// Centre in the region frame (region at λ = 0).
    centre: [f64; 3],
    /// Penumbral radius (rad).
    rho: f64,
    psi0: f64,
    present: bool,
}

/// A knot of coronal rain: it condenses near the loop top and falls down one leg,
/// accelerating, then a new knot condenses (SUN-A4).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Knot {
    /// Which leg it falls down (±1) and its distance `a = |σ − ½|` from the apex.
    side: f64,
    a: f64,
    /// `a` at condensation, where the fade-in starts.
    born: f64,
}

impl Knot {
    /// A knot condensing now within `RAIN_BIRTH` of the apex.
    fn condense(rng: &mut impl Rng) -> Self {
        let a = rng.random_range(0.0..RAIN_BIRTH);
        Self {
            side: coin(rng),
            a,
            born: a,
        }
    }

    /// A knot already falling somewhere on its leg, fully faded in: the seeded state.
    fn falling(rng: &mut impl Rng) -> Self {
        let side = Self::condense(rng).side;
        let a = rng.random_range(0.0..0.5 - KNOT_FADE);
        Self {
            side,
            a,
            born: a - KNOT_FADE,
        }
    }

    /// Position along the loop, σ ∈ [0, 1].
    fn sigma(&self) -> f64 {
        0.5 + self.side * self.a
    }

    /// Brightness weight: fades in after condensing and out before the footpoint.
    fn weight(&self) -> f64 {
        ease::smoothstep((self.a - self.born) / KNOT_FADE)
            * ease::smoothstep((0.5 - self.a) / KNOT_FADE)
    }
}

/// Distance from the apex after falling for `dt` on a loop of period `period`: the
/// closed form of `ȧ = (RAIN_V0 + RAIN_ACCEL·a)/T`, exact for any step split.
pub(crate) fn rain_fall(a: f64, dt: f64, period: f64) -> f64 {
    let c = RAIN_V0 / RAIN_ACCEL;
    (a + c) * (RAIN_ACCEL * dt / period).exp() - c
}

#[derive(Debug, Clone, Copy)]
struct Loop {
    /// Field-line samples in the region frame (units of R☉).
    pts: [[f64; 3]; LOOP_SAMPLES],
    /// Apex height above the photosphere (R☉).
    apex: f64,
    knots: [Knot; KNOTS],
    /// Rain time scale T (s).
    period: f64,
}

/// A bipolar active region: two spots tilted by Joy's law and a sheared arcade of three
/// nested cool loops, rotating rigidly at the rate of its latitude.
#[derive(Debug, Clone)]
struct Region {
    lon: f64,
    /// The region has been short of +150° since it spawned, so it may respawn once it
    /// passes +150°: one spawned behind the Sun first comes round.
    armed: bool,
    omega: f64,
    spots: [Spot; 2],
    loops: [Loop; LOOPS],
}

impl Region {
    /// An inert stand-in until `SolarSurface::seed` runs (never shown: `resize` seeds
    /// before the first frame): no spot present, no loop tall enough to draw, and never
    /// armed to respawn.
    fn placeholder() -> Self {
        let spot = Spot {
            centre: [0.0, 0.0, 1.0],
            rho: 0.0,
            psi0: 0.0,
            present: false,
        };
        let knot = Knot {
            side: 1.0,
            a: 0.0,
            born: 0.0,
        };
        let lp = Loop {
            pts: [[0.0, 0.0, 1.0]; LOOP_SAMPLES],
            apex: 0.0,
            knots: [knot; KNOTS],
            period: 1.0,
        };
        Self {
            lon: PI,
            armed: false,
            omega: 0.0,
            spots: [spot; 2],
            loops: [lp; LOOPS],
        }
    }

    /// A new region at latitude `lat` and longitude `lon` whose tallest loop stands
    /// `h_max` R☉ high.
    fn spawn(rng: &mut impl Rng, lat: f64, lon: f64, h_max: f64) -> Self {
        let centre = sphere(lat, 0.0);
        let east_west = [1.0, 0.0, 0.0];
        let north = [0.0, lat.cos_det(), -lat.sin_det()];
        // Joy's law: tilt γ = 0.5|φ|, the leading (westward) spot equatorward.
        let tilt = (0.5 * lat.abs().to_degrees()).to_radians();
        let axis = axpy(
            -lat.signum() * tilt.sin_det(),
            north,
            east_west.map(|v| v * tilt.cos_det()),
        );
        let up = centre;
        let across = cross(axis, up);
        // Half-separation δ: the spots are drawn 2–3× oversize (below), so they sit
        // 2δ = 13–15° apart, beyond their widest blended penumbrae (1.07·(6° + 4°)) and
        // beyond those of a pair on the display floor at 80×24 (1.07·(6° + 5.2°)): a
        // shared penumbra would be a δ-spot, a few % of real groups.
        let delta: f64 = rng
            .random_range(SPOT_HALF_SEP_DEG.0..SPOT_HALF_SEP_DEG.1)
            .to_radians();
        let (sd, cd) = delta.sin_cos_det();
        let spot = |sign: f64, rho: f64, present: bool, psi0: f64| Spot {
            centre: axpy(sign * sd, axis, centre.map(|v| v * cd)),
            rho,
            psi0,
            present,
        };
        // Penumbral radii ≈ 2–3× the largest real ones (20–30 Mm), so spots resolve at
        // R = 46 px.
        let lead_rho = rng
            .random_range(LEAD_RHO_DEG.0..LEAD_RHO_DEG.1)
            .to_radians();
        let spots = [
            spot(1.0, lead_rho, true, rng.random_range(0.0..TAU)),
            spot(
                -1.0,
                rng.random_range(FOLLOW_RHO_DEG.0..FOLLOW_RHO_DEG.1)
                    .to_radians(),
                rng.random_range(0.0..1.0) >= 0.4,
                rng.random_range(0.0..TAU),
            ),
        ];
        // The dipole sits 2δ deep, so its radial field peaks under the spots (±δ). The
        // arcade is that dipole's field lines turned about the vertical by the shear,
        // either way: nested loops in one vertical plane over the inversion line, each
        // bowed into an S by the twist.
        let d_b = 2.0 * delta;
        let shear = rng.random_range(SHEAR_DEG.0..SHEAR_DEG.1).to_radians() * coin(rng);
        let (ss, cs) = shear.sin_cos_det();
        let arc_axis = axpy(ss, across, axis.map(|v| v * cs));
        let arc_across = axpy(-ss, axis, across.map(|v| v * cs));
        let loops = std::array::from_fn(|k| {
            let h = (h_max * NEST[k]).max(0.04);
            let local = dipole_loop(d_b, d_b + h);
            let apex = local.iter().map(|&p| norm(p) - 1.0).fold(0.0, f64::max);
            let half = local.iter().map(|p| p[0].abs()).fold(1e-9, f64::max);
            Loop {
                pts: local.map(|[a, b, c]| {
                    let b = b + ARCADE_TWIST * half * (PI * a / half).sin_det();
                    axpy(a, arc_axis, axpy(b, arc_across, up.map(|u| c * u)))
                }),
                apex,
                knots: std::array::from_fn(|_| Knot::falling(rng)),
                period: rng.random_range(5.0..9.0),
            }
        });
        Self {
            lon,
            armed: lon < RESPAWN_LON,
            omega: omega_rad_per_sec(lat),
            spots,
            loops,
        }
    }

    fn advance(&mut self, dt: f64) {
        self.lon = geom::wrap_angle(self.lon + self.omega * dt);
        self.armed |= self.lon < RESPAWN_LON;
    }

    /// Let the rain fall for `dt`; a knot that reaches its footpoint is replaced by a
    /// new one condensing near the apex.
    fn rain(&mut self, dt: f64, rng: &mut impl Rng) {
        for l in &mut self.loops {
            for k in &mut l.knots {
                k.a = rain_fall(k.a, dt, l.period);
                if k.a >= 0.5 {
                    *k = Knot::condense(rng);
                }
            }
        }
    }

    /// Nothing of the region can be seen: every spot faces away and every loop
    /// sample is hidden behind the disc.
    fn hidden(&self, b0: f64) -> bool {
        let view = view_of(self.lon, b0);
        self.spots.iter().all(|s| view.rotate(s.centre)[2] < 0.0)
            && self
                .loops
                .iter()
                .flat_map(|l| &l.pts)
                .all(|&p| classify(view.rotate(p)) == Sample::Hidden)
    }
}

/// Region latitude: the scene's Spörer mean ±5°, in either hemisphere.
fn region_lat(rng: &mut impl Rng, mean: f64) -> f64 {
    (mean + rng.random_range(-5.0f64..5.0).to_radians()) * coin(rng)
}

/// ±1 with equal odds.
fn coin(rng: &mut impl Rng) -> f64 {
    if rng.random_bool(0.5) { -1.0 } else { 1.0 }
}

// --- View ---------------------------------------------------------------------------

/// A disc pixel with its observer-frame normal and limb-darkened base intensity.
#[derive(Debug, Clone, Copy)]
struct DiscPx {
    x: u16,
    y: u16,
    n: [f32; 3],
    base: f32,
}

/// A pixel of the chromospheric annulus: distance past the limb, limb angle and the
/// next pixel inward along the radius.
#[derive(Debug, Clone, Copy)]
struct RimPx {
    x: u16,
    y: u16,
    above: f32,
    theta: f32,
    inward: (u16, u16),
}

/// A projected granule: screen position, squared distance weight and life weight.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    x: f32,
    y: f32,
    m2: f32,
    w: f32,
}

/// Everything derived from the terminal size, plus the raster.
#[derive(Debug, Default)]
struct View {
    w: usize,
    /// Disc centre and radius (px).
    cx: f64,
    cy: f64,
    r: f64,
    disc: Vec<DiscPx>,
    /// Granulation factor per disc pixel (1 where unresolved).
    grain: Vec<f32>,
    rim: Vec<RimPx>,
    /// Whole spicule lobe counts, so the pattern closes around the limb.
    lobes: (f64, f64),
    /// Granule spacing (px).
    s_g: f64,
    /// The visible granules projected this step, and their positions in a spatial
    /// hash with cell size `s_g` (CORE-R54).
    cells: Vec<Cell>,
    pts: Vec<[f64; 2]>,
    hash: SpatialHash,
    /// Photospheric intensity per pixel (NaN off the disc): what filaments absorb.
    disc_i: Vec<f32>,
    /// Disc pixels inside a spot (photosphere weight < ½), which filaments skip, so the
    /// thread bridges a group's spots without crossing their penumbrae.
    in_spot: Vec<bool>,
    pixels: Pixels,
    /// Whether the day readout is drawn at all: small terminals keep every cell for the Sun.
    label: bool,
    /// The spectrograph inset, when it fits clear of the Sun (SUN-A8).
    inset: Option<Inset>,
    /// The slit sampled at each inset row, in `rasterise`.
    slit: [SlitRow; INSET_H.1],
}

/// The spectrograph inset in cells: left column, top row, width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inset {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// One slit position: its photospheric intensity (NaN off the disc) and the Doppler
/// shift of its lines, −1 blueward, 0 or +1 redward.
#[derive(Debug, Clone, Copy)]
struct SlitRow {
    i: f32,
    shift: i8,
}

impl Default for SlitRow {
    fn default() -> Self {
        Self {
            i: f32::NAN,
            shift: 0,
        }
    }
}

impl Inset {
    /// The top-right inset for a `w × rows` pixel view with the disc at `(cx, cy)`,
    /// radius `r`, if every pixel of it keeps `INSET_CLEAR·r` from the centre.
    fn fit(w: usize, rows: usize, cx: f64, cy: f64, r: f64) -> Option<Self> {
        let (cells_w, cells_h) = (w, rows / 2);
        let iw = (cells_w / 5).clamp(INSET_W.0, INSET_W.1);
        let ih = (cells_h / 6).clamp(INSET_H.0, INSET_H.1);
        // One cell of margin at the right and top; the label row above it.
        let inset = Self {
            x: cells_w.checked_sub(iw + 1)?,
            y: 1,
            w: iw,
            h: ih,
        };
        // Its extent in pixels (rows are two tall).
        let (top, bottom) = (2 * inset.y, 2 * (inset.y + inset.h));
        let nx = (cx.clamp(inset.x as f64, (inset.x + inset.w) as f64) - cx).abs();
        let ny = (cy.clamp(top as f64, bottom as f64) - cy).abs();
        (bottom + 2 <= rows && nx.hypot(ny) >= INSET_CLEAR * r).then_some(inset)
    }

    /// Wavelength (nm) at inset column `u`.
    fn nm(&self, u: usize) -> f64 {
        let span = (self.w.max(2) - 1) as f64;
        SPECTRUM_NM.0 + (SPECTRUM_NM.1 - SPECTRUM_NM.0) * u as f64 / span
    }

    /// Whether column `u` holds a Fraunhofer line: the column nearest to the line.
    fn is_line(&self, u: usize) -> bool {
        let step = (SPECTRUM_NM.1 - SPECTRUM_NM.0) / (self.w.max(2) - 1) as f64;
        FRAUNHOFER_NM
            .iter()
            .any(|&l| (l - self.nm(u)).abs() <= 0.5 * step)
    }
}

impl View {
    fn new(size: Size) -> Self {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let rows = 2 * h;
        let (cx, cy) = (w as f64 / 2.0, rows as f64 / 2.0);
        let r = (0.46 * rows as f64).min(0.36 * w as f64);
        let mut disc = Vec::new();
        let mut rim = Vec::new();
        for y in 0..rows {
            for x in 0..w {
                let (dx, dy) = ((x as f64 + 0.5 - cx) / r, (y as f64 + 0.5 - cy) / r);
                let rho2 = dx * dx + dy * dy;
                let (x, y) = (x as u16, y as u16);
                if rho2 <= 1.0 {
                    let mu = (1.0 - rho2).sqrt();
                    disc.push(DiscPx {
                        x,
                        y,
                        n: [dx as f32, -dy as f32, mu as f32],
                        base: limb_intensity(mu) as f32,
                    });
                } else {
                    let rho = rho2.sqrt();
                    let above = (rho - 1.0) * r;
                    if above <= ANNULUS_PX {
                        // One pixel back toward the centre, clamped onto the buffer.
                        let step = |p: u16, d: f64, max: usize| {
                            (f64::from(p) + 0.5 - d / rho)
                                .floor()
                                .clamp(0.0, (max - 1) as f64) as u16
                        };
                        rim.push(RimPx {
                            x,
                            y,
                            above: above as f32,
                            theta: (-dy).atan2(dx) as f32,
                            inward: (step(x, dx, w), step(y, dy, rows)),
                        });
                    }
                }
            }
        }
        // Inner pixels first, so a spicule pixel can check the one below it.
        rim.sort_by(|a, b| a.above.total_cmp(&b.above));
        let s_g = granule_spacing(r);
        Self {
            s_g,
            w,
            cx,
            cy,
            r,
            grain: vec![1.0; disc.len()],
            disc,
            rim,
            lobes: ((TAU * r / 3.0).round(), (TAU * r / 2.0).round()),
            cells: Vec::with_capacity(GRANULES_MAX),
            pts: Vec::with_capacity(GRANULES_MAX),
            hash: {
                // Size its scratch for the whole pool now: rebuilding allocates whenever
                // the visible count beats its high-water mark (GEN-R11). Resize may
                // allocate; a `SpatialHash::reserve` in CORE-R54 would replace this.
                let mut hash = SpatialHash::new(w as f64, rows as f64, s_g);
                hash.rebuild(&vec![[0.0; 2]; GRANULES_MAX]);
                hash
            },
            disc_i: vec![f32::NAN; w * rows],
            in_spot: vec![false; w * rows],
            pixels: Pixels::new(size),
            label: w >= 30 && h >= 10,
            inset: Inset::fit(w, rows, cx, cy, r),
            slit: [SlitRow::default(); INSET_H.1],
        }
    }

    fn detailed(&self) -> bool {
        self.r >= R_MIN_DETAIL
    }

    fn on_disc(&self, x: i32, y: i32) -> bool {
        let (dx, dy) = (f64::from(x) + 0.5 - self.cx, f64::from(y) + 0.5 - self.cy);
        dx * dx + dy * dy <= self.r * self.r
    }

    /// Observer-frame point (units of R☉) to pixel coordinates.
    fn screen(&self, p: [f64; 3]) -> (f64, f64) {
        (self.cx + self.r * p[0], self.cy - self.r * p[1])
    }

    /// Project the visible granules (μ > 0) and hash them.
    fn project_granules(&mut self, granules: &[Granule], b0: f64) {
        let tilt = view_of(0.0, b0);
        self.cells.clear();
        self.pts.clear();
        for g in granules {
            let (so, co) = g.lon.sin_cos_det();
            let p = tilt.rotate([g.cos_lat * so, g.sin_lat, g.cos_lat * co]);
            if p[2] > 0.0 {
                let w = g.weight();
                let m = 1.0 + 1.5 * (1.0 - w);
                let (x, y) = self.screen(p);
                self.cells.push(Cell {
                    x: x as f32,
                    y: y as f32,
                    m2: (m * m) as f32,
                    w: w as f32,
                });
                self.pts.push([x, y]);
            }
        }
        self.hash.rebuild(&self.pts);
    }

    /// Weighted Worley distances `(F1, F2, w₁)` at `(x, y)` from the hashed granules
    /// within `radius`. Weights only lengthen distances (m ≥ 1), so every site within
    /// weighted distance `radius` is among the candidates, and a distance not found is
    /// at least `radius`: each F is returned as `min(F, radius)`, a lower bound that is
    /// exact whenever it is below `radius`.
    fn worley(hash: &SpatialHash, cells: &[Cell], x: f64, y: f64, radius: f64) -> (f64, f64, f64) {
        let (mut d1, mut d2, mut w1) = (f32::MAX, f32::MAX, 0.0f32);
        let (xf, yf) = (x as f32, y as f32);
        hash.for_each_within([x, y], radius, |i| {
            let c = cells[i];
            let (dx, dy) = (c.x - xf, c.y - yf);
            let d = (dx * dx + dy * dy) * c.m2;
            // Branch-free running two smallest.
            w1 = if d < d1 { c.w } else { w1 };
            d2 = d2.min(d.max(d1));
            d1 = d1.min(d);
        });
        (
            f64::from(d1.sqrt()).min(radius),
            f64::from(d2.sqrt()).min(radius),
            f64::from(w1),
        )
    }

    /// Refresh the granulation factor `1 + C(μ)(2g − 1)` of the disc pixels, or of
    /// every other pixel row (`parity`). The texture drifts ≈ 3.5 px/s, so refreshing
    /// each row at 30 Hz is invisible and halves the Worley cost of a 60 Hz step.
    fn granulate(&mut self, parity: Option<u16>) {
        let (s_g, reach) = (self.s_g, WORLEY_REACH * self.s_g);
        for (dp, grain) in self.disc.iter().zip(&mut self.grain) {
            if parity.is_some_and(|p| dp.y % 2 != p) {
                continue;
            }
            let mu = f64::from(dp.n[2]);
            // Contrast vanishes toward the limb, so Worley is skipped there.
            *grain = if mu >= 0.1 {
                let (px, py) = (f64::from(dp.x) + 0.5, f64::from(dp.y) + 0.5);
                // Exact below s_g; only the few pixels whose F2 reaches it widen the
                // query, which gives the same result as one query within `reach`.
                let near = Self::worley(&self.hash, &self.cells, px, py, s_g);
                let (f1, f2, w1) = if near.1 < s_g {
                    near
                } else {
                    Self::worley(&self.hash, &self.cells, px, py, reach)
                };
                let g = granule_pattern(f1, f2, w1, s_g);
                (1.0 + granule_contrast(mu) * (2.0 * g - 1.0)) as f32
            } else {
                1.0
            };
        }
    }

    /// Shade every disc pixel: limb darkening × spots × granulation (outside the spots
    /// only: convection is suppressed in the umbra and penumbra, so a spot is a clean
    /// dark ring on the mottle) × faculae.
    fn shade_disc(&mut self, spots: &[SpotView]) {
        for (dp, &grain) in self.disc.iter().zip(&self.grain) {
            let n = dp.n.map(f64::from);
            let mu = n[2];
            let mut i = f64::from(dp.base);
            let (px, py) = (f64::from(dp.x) + 0.5, f64::from(dp.y) + 0.5);
            // The strongest facular field, gated by the photosphere weight of every
            // spot covering the pixel, so no spot brightens another: order-independent.
            let (mut faculae, mut gate) = (0.0f64, 1.0f64);
            for s in spots {
                let c = dot(n, s.s);
                if c < s.reach {
                    continue;
                }
                let beta2 = 2.0 * (1.0 - c).max(0.0);
                faculae = faculae.max((-beta2 / (9.0 * s.rho * s.rho)).exp());
                let q = beta2.sqrt() / s.rho;
                if q < 1.07 {
                    let q_u = (2.0 * (1.0 - dot(n, s.umbra)).max(0.0)).sqrt() / s.rho;
                    // The only per-pixel trig: filament azimuth inside the penumbra.
                    let psi = (py - s.py).atan2(px - s.px) + s.psi0;
                    let (k, outside) = spot_profile(q_u, q, psi, s.filaments);
                    i *= k;
                    gate *= outside;
                }
            }
            i *= 1.0 + (f64::from(grain) - 1.0) * gate;
            // Faculae brighten near the limb only, as observed, and only outside spots.
            i *= 1.0 + 0.35 * (1.0 - mu) * faculae * gate;
            let at = usize::from(dp.y) * self.w + usize::from(dp.x);
            self.disc_i[at] = i as f32;
            self.in_spot[at] = gate < 0.5;
            self.pixels
                .set(i32::from(dp.x), i32::from(dp.y), photosphere_ink(i));
        }
    }

    /// The chromosphere (SUN-A10): an unbroken rim, brightest on the limb and fading
    /// smoothly outward through the spicule layer, its brightness travelling round the
    /// limb in slow waves, a glowing ring rather than spikes. A pixel more than 1 px above
    /// the rim lights only if the next pixel inward is lit, so no speck floats free.
    fn shade_rim(&mut self, t: f64, phase: [f64; 2]) {
        let (n1, n2) = self.lobes;
        let reach = spicule_px(self.r).max(1e-6);
        for rp in &self.rim {
            let above = f64::from(rp.above);
            let th = f64::from(rp.theta);
            let wave = 0.5 * (n1 * th + 1.3 * t + phase[0]).sin_det()
                + 0.5 * (n2 * th - 0.9 * t + phase[1]).sin_det();
            let v = if above <= RIM_PX {
                V_RIM * (0.9 + 0.1 * wave)
            } else {
                let up = above - RIM_PX;
                let (ix, iy) = (i32::from(rp.inward.0), i32::from(rp.inward.1));
                let rooted = up <= 1.0
                    || self
                        .pixels
                        .get(ix, iy)
                        .is_some_and(|i| i.role != Role::Background);
                let fall = (1.0 - up / reach).max(0.0).powf(RIM_FALLOFF);
                let v = V_SPICULE.0 * fall * (0.8 + 0.2 * wave);
                if !rooted || 255.0 * v < ink::MIN_VISIBLE_LEVEL {
                    continue;
                }
                v
            };
            self.pixels
                .set(i32::from(rp.x), i32::from(rp.y), h_alpha_ink(v));
        }
    }

    /// Brighten an off-disc pixel to `ink` (max-combine).
    fn glow(&mut self, x: i32, y: i32, ink: Ink) {
        if self.on_disc(x, y) {
            return;
        }
        if self
            .pixels
            .get(x, y)
            .is_some_and(|old| old.level < ink.level)
        {
            self.pixels.set(x, y, ink);
        }
    }

    /// Absorb an on-disc pixel to `FILAMENT`× its photospheric intensity (idempotent),
    /// except inside a spot: the thread stops at the penumbrae rather than slashing
    /// through the group's focal point.
    fn absorb(&mut self, x: i32, y: i32) {
        let Some(old) = self.pixels.get(x, y) else {
            return;
        };
        let at = y as usize * self.w + x as usize;
        if self.in_spot[at] {
            return;
        }
        let i = self.disc_i[at];
        if !i.is_nan() {
            let dark = photosphere_ink(FILAMENT * f64::from(i));
            if dark.level < old.level {
                self.pixels.set(x, y, dark);
            }
        }
    }

    /// The pixel column the slit lies on: through the disc centre.
    fn slit_x(&self) -> usize {
        self.cx.floor() as usize
    }

    /// Sample the slit at each inset row, top to bottom across the disc (SUN-A8): the
    /// photospheric intensity, and the Doppler shift of rising bright granules (blue) and
    /// sinking dark lanes (red).
    fn sample_slit(&mut self) {
        let Some(inset) = self.inset else { return };
        let slit = self.slit_x();
        let rows = self.disc_i.len() / self.w.max(1);
        let dx = (slit as f64 + 0.5 - self.cx) / self.r;
        for (v, row) in self.slit.iter_mut().enumerate().take(inset.h) {
            let py = self.cy - self.r + (v as f64 + 0.5) * 2.0 * self.r / inset.h as f64;
            let i = if (0.0..rows as f64).contains(&py) {
                self.disc_i[py as usize * self.w + slit]
            } else {
                f32::NAN
            };
            let dy = (py - self.cy) / self.r;
            let mean = limb_intensity((1.0 - dx * dx - dy * dy).max(0.0).sqrt()) as f32;
            let shift = doppler_shift(row.shift, i / mean - 1.0);
            *row = SlitRow { i, shift };
        }
    }

    /// The spectrum's colour at inset column `u`: a hue by wavelength, red at 700 nm to
    /// violet at 400 nm (false colour: `Phase` is isoluminant).
    fn spectrum_ink(inset: Inset, u: usize) -> Ink {
        let turns = (SPECTRUM_NM.1 - inset.nm(u)) / (SPECTRUM_NM.1 - SPECTRUM_NM.0);
        ink::phase(TAU * VIOLET_TURNS * turns)
    }

    /// The long-slit spectrum (SUN-A8): one cell row per slit position, a continuous
    /// band of colour crossed by thin dark Fraunhofer lines, which wiggle with the
    /// granulation under the slit; an umbra on the slit is a dark stripe (about a quarter
    /// of the continuum) with heavier lines. Rows off the disc stay empty. Jaws above and
    /// below the disc mark the slit.
    fn draw_spectrum(&self, canvas: &mut Canvas) {
        let Some(inset) = self.inset else { return };
        let unicode = canvas.mode() == GlyphMode::Unicode;
        for (v, row) in self.slit.iter().enumerate().take(inset.h) {
            if row.i.is_nan() {
                continue;
            }
            let umbra = row.i < SLIT_DARK;
            let y = (inset.y + v) as i32;
            for u in 0..inset.w {
                let colour = Self::spectrum_ink(inset, u);
                let x = (inset.x + u) as i32;
                let line = inset.is_line(u);
                if !unicode {
                    let ch = if line {
                        '|'
                    } else if umbra {
                        '-'
                    } else {
                        '='
                    };
                    canvas.put(x, y, Glyph::ascii(ch), colour);
                    continue;
                }
                let ch = match (line, umbra, row.shift) {
                    (true, true, _) => '┃',
                    (true, false, -1) => '▏',
                    (true, false, 1) => '▕',
                    (true, false, _) => '│',
                    (false, true, _) => '▓',
                    (false, false, _) => ' ',
                };
                canvas.put_bg(x, y, Glyph::new(ch, '|'), DARK_INK, colour);
            }
        }
        if !unicode {
            return;
        }
        // The jaws: the first `JAW_CELLS` cells wholly on the disc from each end of the
        // slit's column.
        let x = self.slit_x() as i32;
        let cells = (self.disc_i.len() / self.w.max(1) / 2) as i32;
        let inside = |cy: &i32| self.on_disc(x, 2 * cy) && self.on_disc(x, 2 * cy + 1);
        let top = (0..cells).filter(inside).take(JAW_CELLS as usize);
        let bottom = (0..cells).rev().filter(inside).take(JAW_CELLS as usize);
        for cy in top.chain(bottom) {
            if let Some(colour) = self.pixels.get(x, 2 * cy) {
                canvas.put_bg(x, cy, Glyph::new('│', '|'), DARK_INK, colour);
            }
        }
    }

    /// Whether nested loop `k` (apex `apex` R☉) is drawn at this size: loops lower than
    /// `MIN_LOOP_PX` would fuse with the taller ones into a blob, and so would the middle
    /// arch on a disc smaller than `THIN_ARCADE_R`.
    fn loop_shown(&self, k: usize, apex: f64) -> bool {
        apex * self.r >= MIN_LOOP_PX && (k != 1 || self.r >= THIN_ARCADE_R)
    }

    /// Cool loops: bright above the limb with falling rain knots; on the disc their
    /// elevated arches absorb as one short dark filament (only the loops `loop_shown`).
    fn draw_loops(&mut self, regions: &[Region], b0: f64, t: f64) {
        for region in regions {
            let view = view_of(region.lon, b0);
            // The outer arch's sheet first, so the arches and their rain glow over it.
            if self.loop_shown(0, region.loops[0].apex) {
                let steps = fill_steps(region.loops[0].apex, self.r);
                for (u, f, q) in fill_points(&region.loops[0], &view, steps) {
                    if classify(q) == Sample::Emission {
                        let (sx, sy) = self.screen(q);
                        let shimmer = (FILL_WAVE.0 * t - FILL_WAVE.1 * u + 3.0 * f).sin_det();
                        let v = ease::lerp(V_FILL.0, V_FILL.1, f) + FILL_SHIMMER * shimmer;
                        self.glow(sx.floor() as i32, sy.floor() as i32, h_alpha_ink(v));
                    }
                }
            }
            for (k, lp) in region.loops.iter().enumerate() {
                if !self.loop_shown(k, lp.apex) {
                    continue;
                }
                let mut prev: Option<(i32, i32)> = None;
                for (i, &p) in lp.pts.iter().enumerate() {
                    let q = view.rotate(p);
                    let (sx, sy) = self.screen(q);
                    let (x, y) = (sx.floor() as i32, sy.floor() as i32);
                    match classify(q) {
                        Sample::Emission => {
                            let u = i as f64 / (LOOP_SAMPLES - 1) as f64;
                            let knot = lp
                                .knots
                                .iter()
                                .map(|k| {
                                    let z = (u - k.sigma()) / KNOT_WIDTH;
                                    k.weight() * (-0.5 * z * z).exp()
                                })
                                .fold(0.0, f64::max);
                            let ink = h_alpha_ink(V_LOOP + V_KNOT * knot);
                            match prev {
                                Some((x0, y0)) if (x - x0).abs().max((y - y0).abs()) > 1 => {
                                    for (lx, ly) in geom::line(x0, y0, x, y) {
                                        self.glow(lx, ly, ink);
                                    }
                                }
                                _ => self.glow(x, y, ink),
                            }
                            prev = Some((x, y));
                        }
                        Sample::Filament => {
                            if k == 0 && norm(p) - 1.0 > FILAMENT_RISE * lp.apex {
                                self.absorb(x, y);
                            }
                            prev = None;
                        }
                        Sample::Hidden => prev = None,
                    }
                }
            }
        }
    }
}

/// Steps up the outer arch's sheet from the surface to the arch, for an apex `apex` R☉
/// on a disc of `r_px` pixels: about two per pixel of height (SUN-A11).
pub(crate) fn fill_steps(apex: f64, r_px: f64) -> usize {
    ((2.0 * apex * r_px).ceil() as usize).clamp(2, 64)
}

/// The prominence sheet under loop `lp` (SUN-A11): for every loop sample, points from
/// its foot on the surface (the sample pulled in radially to R☉) up to the sample, in
/// `steps` steps, as `(u along the loop, f up the sheet, observer-frame point)`.
fn fill_points<'a>(
    lp: &'a Loop,
    view: &'a Projector,
    steps: usize,
) -> impl Iterator<Item = (f64, f64, [f64; 3])> + 'a {
    let last = (LOOP_SAMPLES - 1) as f64;
    lp.pts.iter().enumerate().flat_map(move |(i, &p)| {
        let n = norm(p).max(1e-9);
        let foot = [p[0] / n, p[1] / n, p[2] / n];
        (1..=steps).map(move |j| {
            let f = j as f64 / steps as f64;
            let q = [0, 1, 2].map(|a| foot[a] + (p[a] - foot[a]) * f);
            (i as f64 / last, f, view.rotate(q))
        })
    })
}

/// A spot as seen this frame (observer frame and pixels).
#[derive(Debug, Clone, Copy, Default)]
struct SpotView {
    s: [f64; 3],
    umbra: [f64; 3],
    rho: f64,
    psi0: f64,
    /// Penumbral filament count for the spot's size in pixels.
    filaments: f64,
    px: f64,
    py: f64,
    /// `facular_reach(rho)`: pixels with `n·s` below it are untouched by this spot.
    reach: f64,
}

// --- Scene --------------------------------------------------------------------------

#[derive(Debug)]
pub struct SolarSurface {
    t: f64,
    /// Heliographic latitude of the disc centre.
    b0: f64,
    seeded: bool,
    lat_mean: f64,
    /// Placeholders until `seed` (never shown: `resize` seeds before the first frame).
    regions: [Region; REGIONS],
    granules: Vec<Granule>,
    spicule_phase: [f64; 2],
    /// Update count, which picks the granulation rows to refresh.
    steps: u64,
    /// Test hooks (SUN-A1, SUN-A5, SUN-A7).
    show_regions: bool,
    show_loops: bool,
    view: View,
}

impl SolarSurface {
    fn new() -> Self {
        Self {
            t: 0.0,
            b0: 0.0,
            seeded: false,
            lat_mean: 0.0,
            regions: std::array::from_fn(|_| Region::placeholder()),
            granules: Vec::with_capacity(GRANULES_MAX),
            spicule_phase: [0.0; 2],
            steps: 0,
            show_regions: true,
            show_loops: true,
            view: View::default(),
        }
    }

    fn seed(&mut self, rng: &mut impl Rng) {
        self.b0 = rng.random_range(-7.25f64..7.25).to_radians();
        self.lat_mean = rng.random_range(10.0f64..25.0).to_radians();
        self.spicule_phase = [rng.random_range(0.0..TAU), rng.random_range(0.0..TAU)];
        // The first region sits just inside the east limb with the tallest loops, so
        // they show at t = 0; the others follow ~120° apart.
        let lat_mean = self.lat_mean;
        self.regions = std::array::from_fn(|k| {
            let lon = if k == 0 {
                -78f64.to_radians()
            } else {
                geom::wrap_angle(
                    (-78.0 + 120.0 * k as f64 + rng.random_range(-10.0..10.0)).to_radians(),
                )
            };
            let h = if k == 0 {
                LOOP_H.1
            } else {
                rng.random_range(LOOP_H.0..LOOP_H.1)
            };
            let lat = region_lat(rng, lat_mean);
            Region::spawn(rng, lat, lon, h)
        });
        self.seeded = true;
    }

    /// Match the granule pool to the disc: only when the target moves by > 10 %, so
    /// small resizes keep the texture; new points fade in.
    fn fit_granules(&mut self, rng: &mut impl Rng) {
        let target = granule_count(self.view.r);
        let n = self.granules.len();
        if n == 0 {
            for _ in 0..target {
                let age = rng.random_range(0.0..1.0);
                self.granules.push(Granule::spawn(rng, age));
            }
        } else if target.abs_diff(n) * 10 > n {
            self.granules.truncate(target);
            while self.granules.len() < target {
                self.granules.push(Granule::spawn(rng, 0.0));
            }
        }
    }

    /// The spots that face us (or nearly), with their Wilson-shifted umbrae; returns
    /// how many were written.
    fn spot_views(&self, out: &mut [SpotView; REGIONS * 2]) -> usize {
        let mut n = 0;
        for region in &self.regions {
            let view = view_of(region.lon, self.b0);
            let [lead, follow] = region.spots;
            // The display floor never lets the pair's penumbrae touch (SUN-A6).
            let apart = dot(lead.centre, follow.centre).clamp(-1.0, 1.0).acos() / SPOT_SEP_CAP;
            for spot in region.spots.iter().filter(|s| s.present) {
                let s = view.rotate(spot.centre);
                if s[2] <= -spot.rho {
                    continue;
                }
                let rho = spot.rho.max((SPOT_MIN_PX / self.view.r).min(apart));
                let [ox, oy] = wilson_offset(rho, s);
                let (ux, uy) = (s[0] + ox, s[1] + oy);
                let (px, py) = self.view.screen(s);
                out[n] = SpotView {
                    s,
                    // On the spot's own hemisphere, even just behind the limb.
                    umbra: [
                        ux,
                        uy,
                        (1.0 - ux * ux - uy * uy).max(0.0).sqrt().copysign(s[2]),
                    ],
                    rho,
                    psi0: spot.psi0,
                    filaments: penumbra_filaments(rho * self.view.r),
                    px,
                    py,
                    reach: facular_reach(rho),
                };
                n += 1;
            }
        }
        n
    }

    /// Draw the frame; `all_rows` refreshes the whole granulation, otherwise every
    /// other row (alternating by step).
    fn rasterise(&mut self, all_rows: bool) {
        let mut spots = [SpotView::default(); REGIONS * 2];
        let n = if self.show_regions {
            self.spot_views(&mut spots)
        } else {
            0
        };
        let v = &mut self.view;
        v.pixels.clear();
        v.disc_i.fill(f32::NAN);
        let detailed = v.detailed();
        if detailed {
            v.project_granules(&self.granules, self.b0);
            v.granulate((!all_rows).then_some((self.steps % 2) as u16));
        }
        v.shade_disc(&spots[..n]);
        v.shade_rim(self.t, self.spicule_phase);
        if detailed && self.show_loops && self.show_regions {
            v.draw_loops(&self.regions, self.b0, self.t);
        }
        v.sample_slit();
    }

    /// The ASCII fallback. On the disc, a brightness ramp over the pair's mean lit `Seq`
    /// value. Off it, by the brighter pixel: a knot core `o`, then the one-cell rim `'`
    /// (a pixel within `ASCII_RIM_PX` of the limb), then loop plasma `.`; spicule-only
    /// cells stay blank, so the loops stand clear of the limb instead of merging into
    /// its fuzz.
    fn ascii_cell(&self, cx: i32, cy: i32, top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let lit = [top, bottom].map(|i| (i.role != Role::Background).then(|| ink::seq_value(i)));
        let view = &self.view;
        let past_limb = |py: f64| {
            let (dx, dy) = (f64::from(cx) + 0.5 - view.cx, py - view.cy);
            dx.hypot(dy) - view.r
        };
        // The cell centre is the boundary between its two pixels.
        if past_limb(f64::from(2 * cy + 1)) <= 0.0 {
            let (sum, count) = lit.iter().flatten().fold((0.0, 0.0), |(s, c), &v| {
                (s + photosphere_intensity(v), c + 1.0)
            });
            if count == 0.0 {
                return None;
            }
            let i = sum / count;
            let ch = match i {
                i if i >= 1.026 => '#',
                i if i >= 0.923 => '*',
                i if i >= 0.769 => '+',
                i if i >= 0.59 => '=',
                i if i >= 0.436 => ':',
                i if i >= 0.308 => '.',
                _ => ' ',
            };
            return Some((ch, photosphere_ink(i)));
        }
        let v = lit.iter().flatten().copied().reduce(f64::max)?;
        let rim = [2 * cy, 2 * cy + 1].into_iter().any(|py| {
            let d = past_limb(f64::from(py) + 0.5);
            d > 0.0 && d <= ASCII_RIM_PX
        });
        let ch = if v >= V_KNOT_CORE {
            'o'
        } else if rim {
            '\''
        } else if v >= V_LOOP_ASCII {
            '.'
        } else {
            return None;
        };
        Some((ch, h_alpha_ink(v)))
    }

    /// `day 12.4  Teff 5772 K`, bottom-right and clear of the title (GEN-R13); `Teff`
    /// is dropped first when the row is too narrow, then the whole readout.
    fn draw_label(&self, canvas: &mut Canvas) {
        if !self.view.label {
            return;
        }
        let room = title::readout_room(canvas.size(), &Title::of(self));
        let mut buf = StackText::<32>::new();
        let _ = write!(buf, "day {:.1}", DAYS_PER_SEC * self.t);
        let day = buf.len();
        let _ = write!(buf, "  Teff {TEFF_K} K");
        // ASCII only, so byte lengths are char counts and every cut is a char boundary.
        let n = if buf.len() <= room {
            buf.len()
        } else if day <= room {
            day
        } else {
            return;
        };
        let row = i32::from(canvas.size().height()) - 1;
        draw::label_right(canvas, row, &buf.as_str()[..n], LABEL_INK);
    }
}

impl Generator for SolarSurface {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(&mut ctx.rng);
        }
        self.view = View::new(size);
        self.fit_granules(&mut ctx.rng);
        self.rasterise(true);
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.t += dt;
        self.steps += 1;
        for g in &mut self.granules {
            g.advance(dt);
            if g.age >= g.life {
                *g = Granule::spawn(&mut ctx.rng, 0.0);
            }
        }
        for r in &mut self.regions {
            r.advance(dt);
            r.rain(dt, &mut ctx.rng);
            // Respawn only out of sight, 0–60° further on, so nothing pops on the disc.
            if r.armed && r.lon >= RESPAWN_LON && r.hidden(self.b0) {
                let lon = geom::wrap_angle(r.lon + ctx.rng.random_range(0.0..RESPAWN_STEP));
                let lat = region_lat(&mut ctx.rng, self.lat_mean);
                let h = ctx.rng.random_range(LOOP_H.0..LOOP_H.1);
                *r = Region::spawn(&mut ctx.rng, lat, lon, h);
            }
        }
        self.rasterise(false);
    }

    fn render(&self, canvas: &mut Canvas) {
        self.view.pixels.blit(canvas, |cx, cy, top, bottom| {
            self.ascii_cell(cx, cy, top, bottom)
        });
        self.view.draw_spectrum(canvas);
        if let Some(inset) = self.view.inset {
            // The false-colour note when the row has room for it.
            let label = if SPECTRUM_LABEL.len() + 2 <= inset.w + 1 {
                SPECTRUM_LABEL
            } else {
                SPECTRUM_LABEL_SHORT
            };
            draw::label(
                canvas,
                inset.x as i32 - 1,
                inset.y as i32 - 1,
                label,
                LABEL_INK,
            );
        }
        self.draw_label(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::geom::v3::unit;

    /// The spot intensity at normalised distance `q` and azimuth `psi` (24 filaments).
    fn spot_intensity(q: f64, psi: f64) -> f64 {
        spot_profile(q, q, psi, 24.0).0
    }

    #[test]
    fn sun_a3_penumbral_filaments_resolve_at_every_size() {
        // One filament per 3 px of penumbral circumference, 6–24.
        assert_eq!(penumbra_filaments(0.0), 6.0);
        assert_eq!(penumbra_filaments(1.9), 6.0);
        assert_eq!(penumbra_filaments(46.0 * 5f64.to_radians()), 8.0);
        assert_eq!(penumbra_filaments(12.0), 24.0);
        for n in [6.0, 10.0, 17.0, 24.0] {
            for q in [0.6, 0.8] {
                let m = mean((0..720).map(|k| spot_profile(q, q, TAU * f64::from(k) / 720.0, n).0));
                assert!((m - 0.70).abs() < 0.005, "{n} filaments: mean {m}");
            }
        }
        // The rendered spots use it: unless held at the minimum, a 200×50 spot's
        // filaments stand ≥ 2 px apart at mid-penumbra, where 24 would alias below a
        // pixel.
        let (s, _) = scene(200, 50);
        let mut views = [SpotView::default(); REGIONS * 2];
        let n = s.spot_views(&mut views);
        assert!(n > 0);
        for v in &views[..n] {
            let rho_px = v.rho * s.view.r;
            assert_eq!(v.filaments, penumbra_filaments(rho_px));
            let pitch = TAU * 0.73 * rho_px / v.filaments;
            assert!(
                v.filaments == 6.0 || pitch >= 2.0,
                "{} at {pitch:.2} px",
                v.filaments
            );
        }
    }

    #[test]
    fn sun_a6_bipolar_spots_never_share_a_penumbra() {
        // A shared penumbra is a δ-spot, a few % of real groups: the pair's blended
        // penumbrae (out to 1.07 ρ_p each) must never touch.
        let mut rng = SplitMix64::new(4);
        for i in 0..2000 {
            let lat = (5.0 + 25.0 * f64::from(i % 50) / 50.0).to_radians();
            let r = Region::spawn(&mut rng, lat, 0.0, LOOP_H.0);
            let [lead, follow] = r.spots;
            let sep = dot(lead.centre, follow.centre).clamp(-1.0, 1.0).acos();
            // 2δ = 13–15°, wide enough that the display floor never makes them touch
            // (SUN-A3).
            assert!(
                (13.0..=15.0).contains(&sep.to_degrees()),
                "region {i}: {:.2}° apart",
                sep.to_degrees()
            );
            assert!(
                sep > 1.07 * (lead.rho + follow.rho),
                "region {i}: {:.2}° apart, radii {:.2}° + {:.2}°",
                sep.to_degrees(),
                lead.rho.to_degrees(),
                follow.rho.to_degrees()
            );
        }
    }

    #[test]
    fn sun_a4_small_discs_thin_the_arcade_so_arches_stay_apart() {
        // At 80×24 (R = 22 px) the middle arch is dropped; the outer and inner ones keep
        // a black gap everywhere clear of the limb fuzz.
        let (s, _) = scene(80, 24);
        let v = &s.view;
        assert!(v.r < THIN_ARCADE_R);
        let first = &s.regions[0];
        let shown: Vec<usize> = (0..LOOPS)
            .filter(|&k| v.loop_shown(k, first.loops[k].apex))
            .collect();
        assert_eq!(shown, [0, 2]);
        let pixels = |k: usize| loop_pixels(&s, first, k);
        let (outer, inner) = (pixels(0), pixels(2));
        assert!(!outer.is_empty() && !inner.is_empty());
        for &(x, y) in &outer {
            for &(ix, iy) in &inner {
                assert!(
                    (x - ix).abs().max((y - iy).abs()) >= 2,
                    "arches touch at {x},{y} / {ix},{iy}"
                );
            }
        }
        // 200×50 keeps all three.
        let (big, _) = scene(200, 50);
        assert!(big.view.r >= THIN_ARCADE_R);
        assert!((0..LOOPS).all(|k| big.view.loop_shown(k, big.regions[0].loops[k].apex)));
    }

    /// Emission pixels of loop `k` of `region` (joins included) clear of the annulus.
    fn loop_pixels(s: &SolarSurface, region: &Region, k: usize) -> Vec<(i32, i32)> {
        let v = &s.view;
        let view = view_of(region.lon, s.b0);
        let mut out = Vec::new();
        let mut prev: Option<(i32, i32)> = None;
        for &p in &region.loops[k].pts {
            let q = view.rotate(p);
            if classify(q) != Sample::Emission {
                prev = None;
                continue;
            }
            let (sx, sy) = v.screen(q);
            let here = (sx.floor() as i32, sy.floor() as i32);
            let (x0, y0) = prev.unwrap_or(here);
            for (x, y) in geom::line(x0, y0, here.0, here.1) {
                let (dx, dy) = (f64::from(x) + 0.5 - v.cx, f64::from(y) + 0.5 - v.cy);
                if dx.hypot(dy) - v.r > ANNULUS_PX {
                    out.push((x, y));
                }
            }
            prev = Some(here);
        }
        out
    }

    /// A region's midpoint (region frame), halfway between its spots on the sphere.
    fn midpoint(r: &Region) -> [f64; 3] {
        let m = axpy(1.0, r.spots[0].centre, r.spots[1].centre);
        unit(m)
    }

    /// A region's latitude (its midpoint lies at λ = 0 in the region frame).
    fn lat_of(r: &Region) -> f64 {
        midpoint(r)[1].asin()
    }

    fn scene(w: u16, h: u16) -> (SolarSurface, Ctx) {
        let mut ctx = Ctx::new(11, Size::new(w, h));
        let mut s = SolarSurface::new();
        s.resize(Size::new(w, h), &mut ctx);
        (s, ctx)
    }

    /// The scene with regions (and so loops) off, re-rasterised.
    fn quiet(w: u16, h: u16) -> SolarSurface {
        let (mut s, _) = scene(w, h);
        s.show_regions = false;
        s.rasterise(true);
        s
    }

    /// `(μ, Seq value)` of every disc pixel.
    fn disc_values(s: &SolarSurface) -> Vec<(f64, f64)> {
        let v = &s.view;
        v.disc
            .iter()
            .map(|d| {
                let ink = v
                    .pixels
                    .get(i32::from(d.x), i32::from(d.y))
                    .unwrap_or(Ink::BLANK);
                (f64::from(d.n[2]), ink::seq_value(ink))
            })
            .collect()
    }

    fn mean(xs: impl Iterator<Item = f64>) -> f64 {
        let (s, n) = xs.fold((0.0, 0.0), |(s, n), x| (s + x, n + 1.0));
        s / n
    }

    #[test]
    fn sun_a1_limb_darkening_law_and_rendered_contrast() {
        assert!((limb_intensity(1.0) - 1.0).abs() < 1e-12);
        assert!((limb_intensity(0.0) - 0.4).abs() < 1e-12);
        assert!((limb_intensity(0.5) - 0.7).abs() < 1e-12);
        for i in 0..100 {
            let mu = f64::from(i) / 100.0;
            assert!(limb_intensity(mu + 0.01) > limb_intensity(mu));
        }
        let px = disc_values(&quiet(200, 50));
        let centre = mean(px.iter().filter(|p| p.0 > 0.8).map(|p| 255.0 * p.1));
        let limb = mean(px.iter().filter(|p| p.0 < 0.3).map(|p| 255.0 * p.1));
        assert!(
            centre - limb >= 60.0,
            "centre {centre:.1} vs limb {limb:.1}"
        );
    }

    #[test]
    fn sun_a2_differential_rotation_is_exact_and_split_invariant() {
        assert!((omega_deg_per_day(0.0) - 14.713).abs() < 1e-12);
        for lat in [30.0, -30.0] {
            assert!((omega_deg_per_day(lat) - 14.002).abs() < 1e-3);
        }
        for lat in [60.0, -60.0] {
            assert!((omega_deg_per_day(lat) - 11.911).abs() < 1e-3);
        }
        for i in 0..90 {
            let a = f64::from(i);
            assert_eq!(omega_deg_per_day(a), omega_deg_per_day(-a));
            assert!(omega_deg_per_day(a + 1.0) < omega_deg_per_day(a));
        }
        let period = 360.0 / omega_deg_per_day(0.0) / DAYS_PER_SEC;
        assert!((period - 81.56).abs() < 0.01, "{period}");

        let schedules: [Vec<f64>; 3] = [
            vec![0.25; 40],
            vec![1.0 / 60.0; 600],
            (0..400).map(|i| [0.01, 0.04][i % 2]).collect(),
        ];
        let (s0, _) = scene(80, 24);
        let region = s0.regions[1].clone();
        let mut ctx = Ctx::new(3, Size::new(1, 1));
        let granule = Granule {
            life: 1e9,
            ..Granule::spawn(&mut ctx.rng, 0.0)
        };
        let expect = |lat: f64| omega_deg_per_day(lat) * 3.0;
        let moved = |from: f64, to: f64| geom::wrap_angle(to - from).to_degrees();
        for dts in &schedules {
            assert!((dts.iter().sum::<f64>() - 10.0).abs() < 1e-12);
            let (mut r, mut g) = (region.clone(), granule);
            for &dt in dts {
                r.advance(dt);
                g.advance(dt);
            }
            let dr = moved(region.lon, r.lon) - expect(lat_of(&region).to_degrees());
            let dg = moved(granule.lon, g.lon) - expect(granule.sin_lat.asin().to_degrees());
            assert!(dr.abs() < 1e-9 && dg.abs() < 1e-9, "{dr:e} {dg:e}");
        }
    }

    #[test]
    fn sun_a3_spot_profile_umbra_penumbra_and_wilson_shift() {
        let azimuthal =
            |q: f64| mean((0..720).map(|k| spot_intensity(q, TAU * f64::from(k) / 720.0)));
        for q in [0.0, 0.2, 0.375] {
            assert!((spot_intensity(q, 1.0) - 0.22).abs() < 1e-12);
        }
        for i in 0..=40 {
            let q = 0.525 + (0.93 - 0.525) * f64::from(i) / 40.0;
            assert!((azimuthal(q) - 0.70).abs() < 0.005);
            for k in 0..100 {
                let m = spot_intensity(q, f64::from(k) * 0.07) - 0.70;
                assert!(m.abs() <= 0.08 + 1e-12);
            }
        }
        for q in [1.07, 1.5, 9.0] {
            assert!((spot_intensity(q, 0.3) - 1.0).abs() < 1e-12);
        }
        let mut prev = 0.0;
        let (mut half, mut edge) = (None, None);
        for i in 0..=1500 {
            let q = 1.5 * f64::from(i) / 1500.0;
            let m = azimuthal(q);
            assert!(m >= prev - 1e-12, "mean decreases at q = {q}");
            // The umbra–penumbra midpoint (0.22 + 0.70)/2 and the penumbra–photosphere
            // one (0.70 + 1)/2 mark the umbral and penumbral radii.
            if half.is_none() && m >= 0.46 {
                half = Some(q);
            }
            if edge.is_none() && m >= 0.85 {
                edge = Some(q);
            }
            prev = m;
        }
        assert!((half.unwrap_or(0.0) - 0.45).abs() < 0.005, "{half:?}");
        assert!((edge.unwrap_or(0.0) - 1.0).abs() < 0.005, "{edge:?}");
        let rho = 0.08;
        for (lat, lon) in [(0.3, -1.0), (-0.2, 0.7), (0.0, 0.0)] {
            let s = sphere(lat, lon);
            let [ox, oy] = wilson_offset(rho, s);
            let sin_theta = s[0].hypot(s[1]);
            assert!((ox.hypot(oy) - 0.1 * rho * sin_theta).abs() < 1e-9);
            // Toward the centre: antiparallel to the spot's sky-plane position.
            assert!(ox * s[0] + oy * s[1] <= 0.0);
        }
        // Lifted back onto the sphere the shift grows as 0.1·tan θ (in ρ_p). The
        // centre-side penumbra (q from 0.45 to 0.93) survives to μ ≈ 0.2, as observed,
        // and vanishes nearer the limb.
        let umbra_shift = |mu: f64| {
            let s = sphere(0.0, -mu.acos());
            let [ox, oy] = wilson_offset(rho, s);
            let (ux, uy) = (s[0] + ox, s[1] + oy);
            let u = [ux, uy, (1.0 - ux * ux - uy * uy).max(0.0).sqrt()];
            (2.0 * (1.0 - dot(u, s))).max(0.0).sqrt() / rho
        };
        for mu in [0.25, 0.5, 0.9] {
            assert!(umbra_shift(mu) < 0.93 - 0.45, "penumbra gone at μ = {mu}");
        }
        assert!(
            umbra_shift(0.12) > 0.93 - 0.45,
            "no Wilson effect at the limb"
        );
    }

    /// A spot at heliographic (lat, lon) in degrees, unshifted, always in reach.
    fn spot_at(v: &View, lat: f64, lon: f64, rho_deg: f64) -> SpotView {
        let s = sphere(lat.to_radians(), lon.to_radians());
        let (px, py) = v.screen(s);
        SpotView {
            s,
            umbra: s,
            rho: rho_deg.to_radians(),
            psi0: 0.0,
            filaments: 24.0,
            px,
            py,
            reach: -1.0,
        }
    }

    /// The shown spots of `s` (Wilson-shifted views).
    fn shown_spots(s: &SolarSurface) -> Vec<SpotView> {
        let mut out = [SpotView::default(); REGIONS * 2];
        let n = s.spot_views(&mut out);
        out[..n].to_vec()
    }

    #[test]
    fn sun_a3_spots_are_free_of_granulation() {
        // Convection is suppressed in the spot: a pixel well inside a penumbra (q ≤ 0.9)
        // shades the same whatever the granulation there.
        let (mut s, _) = scene(200, 50);
        let spots = shown_spots(&s);
        assert!(!spots.is_empty());
        let inside: Vec<usize> = s
            .view
            .disc
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                let n = d.n.map(f64::from);
                spots.iter().any(|sp| {
                    let beta = (2.0 * (1.0 - dot(n, sp.s)).max(0.0)).sqrt();
                    beta / sp.rho <= 0.9
                })
            })
            .map(|(i, _)| i)
            .collect();
        assert!(inside.len() >= 10, "only {} spot pixels", inside.len());
        let shade = |s: &mut SolarSurface, g: f32| {
            s.view.grain.fill(g);
            s.view.shade_disc(&spots);
            inside
                .iter()
                .map(|&i| {
                    let d = s.view.disc[i];
                    s.view.disc_i[usize::from(d.y) * s.view.w + usize::from(d.x)]
                })
                .collect::<Vec<f32>>()
        };
        let (plain, grainy) = (shade(&mut s, 1.0), shade(&mut s, 1.12));
        for (a, b) in plain.iter().zip(&grainy) {
            assert!((a - b).abs() < 1e-6, "granulation in a spot: {a} vs {b}");
        }
    }

    /// The seeded scene's regions with only the view built (no granules, no raster).
    fn spots_only(w: u16, h: u16, seed: u64) -> SolarSurface {
        let mut ctx = Ctx::new(seed, Size::new(w, h));
        let mut s = SolarSurface::new();
        s.seed(&mut ctx.rng);
        s.view = View::new(Size::new(w, h));
        s
    }

    #[test]
    fn sun_a3_small_discs_floor_the_spots_in_pixels() {
        // Each shown spot is drawn at ρ_eff = max(ρ_p, min(2.0 px/R, sep/2.2)), in the
        // order `spot_views` visits them: at 200×50 (R = 46 px) the floor is 2.49°,
        // under every spot's own ρ_p, so every spot keeps its size; at 80×24 the umbra
        // (0.45 ρ_eff) is ≥ 0.85 px in radius; and at every size the floored pair's
        // blended penumbrae stay strictly apart (SUN-A6). Over 200 seeds.
        for seed in 0..200 {
            for (w, h) in [(80u16, 24u16), (200, 50), (40, 12)] {
                let s = spots_only(w, h, seed);
                let r = s.view.r;
                let mut want = Vec::new();
                for g in &s.regions {
                    let [a, b] = g.spots;
                    let sep = dot(a.centre, b.centre).clamp(-1.0, 1.0).acos();
                    let eff = |rho: f64| rho.max((2.0 / r).min(sep / 2.2));
                    if a.present && b.present {
                        assert!(
                            sep > 1.07 * (eff(a.rho) + eff(b.rho)),
                            "seed {seed} {w}x{h}: {:.2}° apart, drawn {:.2}° + {:.2}°",
                            sep.to_degrees(),
                            eff(a.rho).to_degrees(),
                            eff(b.rho).to_degrees()
                        );
                    }
                    let view = view_of(g.lon, s.b0);
                    for spot in g.spots.iter().filter(|x| x.present) {
                        if view.rotate(spot.centre)[2] > -spot.rho {
                            want.push((spot.rho, eff(spot.rho)));
                        }
                    }
                }
                let shown = shown_spots(&s);
                assert_eq!(shown.len(), want.len(), "seed {seed} {w}x{h}");
                for (sp, &(rho, eff)) in shown.iter().zip(&want) {
                    assert!(
                        (sp.rho - eff).abs() < 1e-12,
                        "seed {seed} {w}x{h}: spot drawn at {:.2} px, want {:.2}",
                        sp.rho * r,
                        eff * r
                    );
                    if w == 200 {
                        assert_eq!(sp.rho, rho, "seed {seed}: floored at 200x50");
                    } else if w == 80 {
                        assert!(0.45 * sp.rho * r >= 0.85, "{:.2} px", sp.rho * r);
                    }
                }
            }
        }
    }

    #[test]
    fn sun_a3_faculae_never_brighten_a_spot() {
        let mut v = View::new(Size::new(200, 50));
        // Near the east limb, where the facular gain is large; B's faculae reach A.
        let a = spot_at(&v, 5.0, -55.0, 5.0);
        let b = spot_at(&v, 5.0, -45.0, 4.0);
        let mut shade = |spots: &[SpotView]| {
            v.shade_disc(spots);
            v.disc_i.clone()
        };
        let (alone, ab, ba, plain) = (shade(&[a]), shade(&[a, b]), shade(&[b, a]), shade(&[]));
        let mut checked = 0;
        for dp in &v.disc {
            let n = dp.n.map(f64::from);
            let q = (2.0 * (1.0 - dot(n, a.s))).max(0.0).sqrt() / a.rho;
            if q < 0.5 {
                let i = usize::from(dp.y) * v.w + usize::from(dp.x);
                assert_eq!(ab[i], alone[i], "spot A brightened at {},{}", dp.x, dp.y);
                assert_eq!(ba[i], ab[i], "spot order matters at {},{}", dp.x, dp.y);
                checked += 1;
            }
        }
        assert!(checked > 5, "only {checked} pixels in spot A");
        // Outside both spots the faculae still brighten.
        let far = v.disc.iter().find(|dp| {
            let n = dp.n.map(f64::from);
            let beta = |s: &SpotView| (2.0 * (1.0 - dot(n, s.s))).max(0.0).sqrt() / s.rho;
            beta(&a) > 1.2 && beta(&b) > 1.2 && beta(&b) < 1.6
        });
        let dp = far.expect("a pixel just outside spot B");
        let i = usize::from(dp.y) * v.w + usize::from(dp.x);
        assert!(ab[i] > plain[i], "no facular brightening outside the spots");
    }

    #[test]
    fn sun_a3_each_spot_acts_only_within_its_own_facular_cap() {
        // A spot's faculae fall below 1 % by 6.5 ρ_p, so each spot view carries its own
        // cap `cos(6.5 ρ_p)` about its centre, not one sized for the leading spot.
        assert_eq!(facular_reach(0.05), (6.5f64 * 0.05).cos_det());
        // At the cap the facular field is < 1 %.
        assert!((-(6.5f64 * 6.5) / 9.0).exp() < 0.01);
        let (s, _) = scene(200, 50);
        let mut views = [SpotView::default(); REGIONS * 2];
        let n = s.spot_views(&mut views);
        assert!(n > 0);
        for v in &views[..n] {
            assert_eq!(v.reach, facular_reach(v.rho));
        }
        let mut v = View::new(Size::new(200, 50));
        let spot = SpotView {
            reach: facular_reach(3f64.to_radians()),
            ..spot_at(&v, 10.0, -40.0, 3.0)
        };
        v.shade_disc(&[]);
        let plain = v.disc_i.clone();
        v.shade_disc(&[spot]);
        let (mut inside, mut changed) = (0, 0);
        for dp in &v.disc {
            let i = usize::from(dp.y) * v.w + usize::from(dp.x);
            let c = dot(dp.n.map(f64::from), spot.s);
            if c < spot.reach {
                assert_eq!(v.disc_i[i], plain[i], "spot acts outside its cap");
            } else {
                inside += 1;
                changed += usize::from(v.disc_i[i] != plain[i]);
            }
        }
        assert!(
            inside > 0 && changed * 2 > inside,
            "{changed} of {inside} in the cap"
        );
    }

    #[test]
    fn sun_a4_rain_condenses_at_the_apex_and_falls_accelerating() {
        // The closed form composes exactly over any step split.
        let (a0, t) = (0.03, 5.0);
        let whole = rain_fall(a0, 0.3, t);
        let split = rain_fall(rain_fall(a0, 0.1, t), 0.2, t);
        assert!((whole - split).abs() < 1e-12);
        // Slow at the top, faster down the leg.
        let dt = 1.0 / 60.0;
        let near_top = rain_fall(0.0, dt, t) - 0.0;
        let low = rain_fall(0.4, dt, t) - 0.4;
        assert!(low > 2.5 * near_top, "{near_top} vs {low}");
        let mut rng = SplitMix64::new(9);
        for _ in 0..200 {
            let k = Knot::condense(&mut rng);
            assert!((0.5 - RAIN_BIRTH..=0.5 + RAIN_BIRTH).contains(&k.sigma()));
            assert_eq!(k.weight(), 0.0, "a condensing knot pops in");
            let f = Knot::falling(&mut rng);
            assert!(
                (f.weight() - smooth_out(f.a)).abs() < 1e-12,
                "seeded knot not faded in"
            );
        }
        // Along a whole fall the knot moves one way and fades out at the footpoint.
        let mut k = Knot {
            side: 1.0,
            a: 0.05,
            born: 0.05,
        };
        let mut prev = k.sigma();
        while k.a < 0.5 {
            k.a = rain_fall(k.a, dt, t);
            assert!(k.sigma() > prev);
            prev = k.sigma();
        }
        k.a = 0.5;
        assert_eq!(k.weight(), 0.0);
    }

    #[test]
    fn sun_a4_arcade_is_nested_in_one_sheared_vertical_plane() {
        let (s, _) = scene(80, 24);
        for r in &s.regions {
            let up = midpoint(r);
            let spot_axis = axpy(-1.0, r.spots[1].centre, r.spots[0].centre);
            // The plane's horizontal direction, from the tallest loop's footpoints.
            let big = &r.loops[0].pts;
            let chord = axpy(-1.0, big[0], big[LOOP_SAMPLES - 1]);
            let dir = axpy(-dot(chord, up), up, chord);
            let normal = cross(dir, up);
            let horizontal = |v: [f64; 3]| unit(axpy(-dot(v, up), up, v));
            let shear = dot(horizontal(spot_axis), unit(dir))
                .abs()
                .acos()
                .to_degrees();
            assert!((44.9..=65.1).contains(&shear), "shear {shear}°");
            let mut prev: Option<(f64, f64)> = None;
            for lp in &r.loops {
                // Every sample within the twist of the vertical plane through the region
                // centre, bowed one way on one leg and the other way on the other: an S.
                let off = |p: [f64; 3]| dot(axpy(-1.0, up, p), unit(normal));
                let along = |p: [f64; 3]| dot(axpy(-1.0, up, p), unit(dir)).abs();
                let half = lp.pts.iter().map(|&p| along(p)).fold(0.0, f64::max);
                let mut bow = 0.0f64;
                for (i, &p) in lp.pts.iter().enumerate() {
                    assert!(off(p).abs() <= ARCADE_TWIST * half * 1.05 + 1e-9);
                    let mirror = off(lp.pts[LOOP_SAMPLES - 1 - i]);
                    assert!((off(p) + mirror).abs() < 1e-6 * half.max(1e-3), "not an S");
                    bow = bow.max(off(p).abs());
                }
                assert!(bow > 0.5 * ARCADE_TWIST * half, "no twist: {bow}");
                let apex = lp.pts.iter().map(|&p| norm(p) - 1.0).fold(0.0, f64::max);
                let span = norm(axpy(-1.0, lp.pts[0], lp.pts[LOOP_SAMPLES - 1]));
                if let Some((a0, s0)) = prev {
                    assert!(apex < a0 && span < s0, "loops not nested");
                }
                prev = Some((apex, span));
            }
        }
    }

    fn smooth_out(a: f64) -> f64 {
        ease::smoothstep((0.5 - a) / KNOT_FADE)
    }

    #[test]
    fn sun_spicules_grow_out_of_the_rim() {
        let (mut s, mut ctx) = scene(200, 50);
        s.show_loops = false;
        for _ in 0..40 {
            s.update(0.25, &mut ctx);
            s.rasterise(true);
            let v = &s.view;
            let lit = |x: i32, y: i32| {
                v.pixels
                    .get(x, y)
                    .is_some_and(|i| i.role != Role::Background)
            };
            for rp in &v.rim {
                let (x, y) = (i32::from(rp.x), i32::from(rp.y));
                if f64::from(rp.above) > RIM_PX + 1.0 && lit(x, y) {
                    let (ix, iy) = rp.inward;
                    assert!(
                        lit(i32::from(ix), i32::from(iy)),
                        "floating spicule pixel {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn sun_spicules_scale_with_the_disc() {
        // Spicules stand 0.035 R high, capped at 1.6 px: full fuzz at 200×50, a thin
        // fringe on small discs whose limb would otherwise look moth-eaten.
        assert!((spicule_px(46.0) - 1.6).abs() < 1e-12);
        assert!((spicule_px(22.0) - 0.77).abs() < 1e-12);
        assert!((spicule_px(11.0) - 0.385).abs() < 1e-12);
        for (w, h) in [(40u16, 12u16), (80, 24), (200, 50)] {
            let (mut s, mut ctx) = scene(w, h);
            s.show_loops = false;
            let reach = RIM_PX + spicule_px(s.view.r);
            for _ in 0..40 {
                s.update(0.25, &mut ctx);
                let v = &s.view;
                for rp in &v.rim {
                    let lit = v
                        .pixels
                        .get(i32::from(rp.x), i32::from(rp.y))
                        .is_some_and(|i| i.role != Role::Background);
                    assert!(
                        !lit || f64::from(rp.above) <= reach + 1e-6,
                        "{w}x{h}: spicule pixel {} px above the limb (R = {})",
                        rp.above,
                        v.r
                    );
                }
            }
        }
    }

    #[test]
    fn sun_a4_dipole_loops_follow_field_lines_between_footpoints() {
        for (d_b, h) in [(0.14, 0.2), (0.1, 0.04), (0.17, 0.12)] {
            let l = d_b + h;
            let pts = dipole_loop(d_b, l);
            let c_d = [0.0, 0.0, 1.0 - d_b];
            let (t0, t1) = footpoint_theta(d_b, l);
            for (i, p) in pts.iter().enumerate() {
                let th = ease::lerp(t0, t1, i as f64 / (LOOP_SAMPLES - 1) as f64);
                let rel = axpy(-1.0, c_d, *p);
                let expect = l * th.sin_det().powi(2);
                assert!((norm(rel) - expect).abs() <= 1e-9 * expect.max(1e-12));
                // In the vertical plane through the dipole axis.
                assert_eq!(rel[1], 0.0);
                if i > 0 && i < LOOP_SAMPLES - 1 {
                    assert!(norm(*p) > 1.0, "sample {i} below the photosphere");
                }
            }
            let (a, b) = (pts[0], pts[LOOP_SAMPLES - 1]);
            assert!((norm(a) - 1.0).abs() < 1e-6 && (norm(b) - 1.0).abs() < 1e-6);
            assert!(
                a[0] * b[0] < 0.0,
                "footpoints on opposite sides along the axis"
            );
        }
        let apex = dipole_point(0.14, 0.34, PI / 2.0);
        assert!((norm(apex) - 1.0 - 0.2).abs() < 1e-9);
    }

    /// Pixel buffers of the same 200×50 frame with loops on and off.
    fn loops_on_off() -> (SolarSurface, SolarSurface) {
        let (on, _) = scene(200, 50);
        let (mut off, _) = scene(200, 50);
        off.show_loops = false;
        off.rasterise(true);
        (on, off)
    }

    #[test]
    fn sun_a5_loops_emit_above_the_limb_and_absorb_on_the_disc() {
        assert_eq!(classify([1.2, 0.0, -0.5]), Sample::Emission);
        assert_eq!(classify([0.0, 1.0, 0.0]), Sample::Emission);
        assert_eq!(classify([0.3, 0.2, 0.1]), Sample::Filament);
        assert_eq!(classify([0.3, 0.2, 0.0]), Sample::Filament);
        assert_eq!(classify([0.3, 0.2, -0.1]), Sample::Hidden);

        let (on, off) = loops_on_off();
        let v = &on.view;
        let rows = usize::from(v.pixels.pixel_size().height());
        // Pixels near any projected Emission sample or the join between two of them.
        let mut near = vec![false; v.w * rows];
        for region in &on.regions {
            let view = view_of(region.lon, on.b0);
            for lp in &region.loops {
                let mut prev: Option<(i32, i32)> = None;
                for &p in &lp.pts {
                    let q = view.rotate(p);
                    if classify(q) != Sample::Emission {
                        prev = None;
                        continue;
                    }
                    let (sx, sy) = v.screen(q);
                    let here = (sx.floor() as i32, sy.floor() as i32);
                    let (x0, y0) = prev.unwrap_or(here);
                    for (x, y) in geom::line(x0, y0, here.0, here.1) {
                        for (dx, dy) in [
                            (-1, -1),
                            (0, -1),
                            (1, -1),
                            (-1, 0),
                            (0, 0),
                            (1, 0),
                            (-1, 1),
                            (0, 1),
                            (1, 1),
                        ] {
                            let (nx, ny) = (x + dx, y + dy);
                            if (0..v.w as i32).contains(&nx) && (0..rows as i32).contains(&ny) {
                                near[ny as usize * v.w + nx as usize] = true;
                            }
                        }
                    }
                    prev = Some(here);
                }
            }
        }
        // ... or on the outer arch's prominence sheet (SUN-A11).
        for region in &on.regions {
            if !v.loop_shown(0, region.loops[0].apex) {
                continue;
            }
            let view = view_of(region.lon, on.b0);
            let steps = fill_steps(region.loops[0].apex, v.r);
            for (_, _, q) in fill_points(&region.loops[0], &view, steps) {
                if classify(q) == Sample::Emission {
                    let (sx, sy) = v.screen(q);
                    let (x, y) = (sx.floor() as i32, sy.floor() as i32);
                    if (0..v.w as i32).contains(&x) && (0..rows as i32).contains(&y) {
                        near[y as usize * v.w + x as usize] = true;
                    }
                }
            }
        }
        let (mut emitted, mut absorbed) = (0, 0);
        for y in 0..rows as i32 {
            for x in 0..v.w as i32 {
                let (a, b) = (on.view.pixels.get(x, y), off.view.pixels.get(x, y));
                if v.on_disc(x, y) {
                    let (a, b) = (a.unwrap_or(Ink::BLANK), b.unwrap_or(Ink::BLANK));
                    assert!(a.level <= b.level, "disc pixel {x},{y} brighter with loops");
                    absorbed += usize::from(a.level < b.level);
                } else if a != b {
                    assert!(
                        near[y as usize * v.w + x as usize],
                        "stray loop pixel {x},{y}"
                    );
                    emitted += 1;
                }
            }
        }
        assert!(
            emitted > 0 && absorbed > 0,
            "{emitted} emitted, {absorbed} absorbed"
        );
    }

    /// Lit off-disc pixels beyond the chromospheric annulus: loop emission only.
    fn clear_of_the_limb(s: &SolarSurface) -> Vec<Ink> {
        let v = &s.view;
        let rows = usize::from(v.pixels.pixel_size().height());
        let mut out = Vec::new();
        for y in 0..rows as i32 {
            for x in 0..v.w as i32 {
                let (dx, dy) = (f64::from(x) + 0.5 - v.cx, f64::from(y) + 0.5 - v.cy);
                if dx.hypot(dy) - v.r <= ANNULUS_PX {
                    continue;
                }
                if let Some(ink) = v.pixels.get(x, y).filter(|i| i.role != Role::Background) {
                    out.push(ink);
                }
            }
        }
        out
    }

    #[test]
    fn sun_a4_loops_are_the_focal_point_at_the_default_size() {
        let (s, _) = scene(80, 24);
        let r = s.view.r;
        // The first region's arcade stands 0.32 R tall and all three arches clear
        // MIN_LOOP_PX (the middle one is still dropped at this size, see
        // `sun_a4_small_discs_thin_the_arcade_so_arches_stay_apart`).
        let first = &s.regions[0];
        assert!(
            first.loops[0].apex * r >= 7.0,
            "apex {} px",
            first.loops[0].apex * r
        );
        for lp in &first.loops {
            assert!(lp.apex * r >= MIN_LOOP_PX, "nested loop {} px", lp.apex * r);
        }
        // Every region's arches clear MIN_LOOP_PX at 200×50.
        let (big, _) = scene(200, 50);
        for region in &big.regions {
            for lp in &region.loops {
                assert!(lp.apex * big.view.r >= MIN_LOOP_PX);
            }
        }
        // Above the limb the two arches read as a system, brighter than the rim and
        // spicules (orange plasma, not limb fuzz) and darker than the penumbra.
        let lit = clear_of_the_limb(&s);
        assert!(
            lit.len() >= 12,
            "only {} loop pixels clear of the limb",
            lit.len()
        );
        let values: Vec<f64> = lit.into_iter().map(ink::seq_value).collect();
        for &v in &values {
            // (The prominence sheet under the outer arch, SUN-A11, is dimmer crimson.)
            assert!(
                (V_FILL.0 - FILL_SHIMMER - 0.01..=V_LOOP + V_KNOT + 0.01).contains(&v),
                "loop value {v}"
            );
        }
        // The loop body sits under the quiet disc's dark lanes (intensity 0.8); only knots
        // pass it.
        let body = values.iter().copied().fold(1.0, f64::min);
        assert!(body < photosphere_value(0.8), "loop body {body}");
    }

    #[test]
    fn sun_ascii_rim_is_one_cell_and_loops_stand_clear_of_it() {
        let (s, _) = scene(80, 24);
        let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Ascii);
        s.render(&mut canvas);
        let lines = canvas.to_lines();
        let v = &s.view;
        // Cells holding a pixel within 1 px of the limb.
        let mut rim_cell = vec![false; 80 * 24];
        for rp in v
            .rim
            .iter()
            .filter(|rp| f64::from(rp.above) <= ASCII_RIM_PX)
        {
            rim_cell[usize::from(rp.y / 2) * 80 + usize::from(rp.x)] = true;
        }
        let mut loops = 0;
        for (cy, line) in lines.iter().enumerate().take(23) {
            for (cx, ch) in line.chars().enumerate() {
                let (dx, dy) = (cx as f64 + 0.5 - v.cx, (2 * cy + 1) as f64 - v.cy);
                if dx.hypot(dy) <= v.r {
                    continue;
                }
                match ch {
                    '\'' => assert!(rim_cell[cy * 80 + cx], "' off the rim at {cx},{cy}"),
                    '.' | 'o' => {
                        loops += usize::from(!rim_cell[cy * 80 + cx]);
                    }
                    ' ' => {}
                    _ => panic!("unexpected off-disc '{ch}' at {cx},{cy}"),
                }
            }
        }
        assert!(loops >= 8, "only {loops} loop cells clear of the rim");
    }

    /// Whether disc pixel (x, y) lies inside a shown spot: the product of the spots'
    /// photosphere weights `smoothstep((q − 0.93)/0.14)` is below ½.
    fn in_a_spot(s: &SolarSurface, spots: &[SpotView], x: usize, y: usize) -> bool {
        let v = &s.view;
        let (dx, dy) = ((x as f64 + 0.5 - v.cx) / v.r, (y as f64 + 0.5 - v.cy) / v.r);
        let n = [dx, -dy, (1.0 - dx * dx - dy * dy).max(0.0).sqrt()];
        let gate: f64 = spots
            .iter()
            .map(|sp| {
                let q = (2.0 * (1.0 - dot(n, sp.s)).max(0.0)).sqrt() / sp.rho;
                ease::smoothstep((q - 0.93) / 0.14)
            })
            .product();
        gate < 0.5
    }

    #[test]
    fn sun_a5_filaments_are_elevated_threads_of_the_arcade() {
        // Seed 11 (the scene seed) and the review seed 42, whose filament runs between
        // its spots.
        for seed in [11, 42] {
            filaments_over_80_s(seed);
        }
    }

    fn filaments_over_80_s(seed: u64) {
        let mut ctx = Ctx::new(seed, Size::new(200, 50));
        let mut s = SolarSurface::new();
        s.resize(Size::new(200, 50), &mut ctx);
        let rows = usize::from(s.view.pixels.pixel_size().height());
        let w = s.view.w;
        let mut absorbed_total = 0;
        for _ in 0..10 {
            for _ in 0..32 {
                s.update(0.25, &mut ctx);
            }
            s.show_loops = false;
            s.rasterise(true);
            let off: Vec<Ink> = (0..rows * w)
                .map(|i| {
                    s.view
                        .pixels
                        .get((i % w) as i32, (i / w) as i32)
                        .unwrap_or(Ink::BLANK)
                })
                .collect();
            s.show_loops = true;
            s.rasterise(true);
            let spots = shown_spots(&s);
            // Pixels within 1 px of a Filament sample standing higher than 0.35 of its
            // loop's apex (apex measured from the samples themselves).
            let mut allowed = vec![false; rows * w];
            for region in &s.regions {
                let view = view_of(region.lon, s.b0);
                for lp in &region.loops {
                    let apex = lp.pts.iter().map(|&p| norm(p) - 1.0).fold(0.0, f64::max);
                    for &p in &lp.pts {
                        let q = view.rotate(p);
                        if classify(q) != Sample::Filament || norm(p) - 1.0 <= 0.35 * apex {
                            continue;
                        }
                        let (sx, sy) = s.view.screen(q);
                        let (x, y) = (sx.floor() as i32, sy.floor() as i32);
                        for (nx, ny) in
                            (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (x + dx, y + dy)))
                        {
                            if (0..w as i32).contains(&nx) && (0..rows as i32).contains(&ny) {
                                allowed[ny as usize * w + nx as usize] = true;
                            }
                        }
                    }
                }
            }
            for y in 0..rows {
                for x in 0..w {
                    if !s.view.on_disc(x as i32, y as i32) {
                        continue;
                    }
                    let on = s.view.pixels.get(x as i32, y as i32).unwrap_or(Ink::BLANK);
                    if on.level < off[y * w + x].level {
                        assert!(allowed[y * w + x], "t = {}: low filament at {x},{y}", s.t);
                        // The thread bridges the spots but never crosses a penumbra.
                        assert!(
                            !in_a_spot(&s, &spots, x, y),
                            "seed {seed} t = {}: filament across a spot at {x},{y}",
                            s.t
                        );
                        absorbed_total += 1;
                    }
                }
            }
        }
        assert!(
            absorbed_total > 0,
            "seed {seed}: no filament crossed the disc"
        );
    }

    #[test]
    fn sun_a7_worley_distances_are_lower_bounds_within_the_query() {
        let s_g = 4.0;
        let radius = 1.5 * s_g;
        // A pixel 0.8 s_g into its bucket: a site 1.4 s_g east lies two buckets over,
        // beyond a 1 s_g query but inside the 1.5 s_g one.
        let (x0, y0) = (10.8 * s_g, 10.5 * s_g);
        let cell = |x: f64, y: f64| Cell {
            x: x as f32,
            y: y as f32,
            m2: 1.0,
            w: 1.0,
        };
        let mut hash = SpatialHash::new(40.0 * s_g, 40.0 * s_g, s_g);
        let cells = [cell(x0, y0), cell(x0 + 1.4 * s_g, y0)];
        hash.rebuild(&cells.map(|c| [f64::from(c.x), f64::from(c.y)]));
        let (f1, f2, _) = View::worley(&hash, &cells, x0, y0, radius);
        assert!(
            f1.abs() < 1e-4 && (f2 - 1.4 * s_g).abs() < 1e-4,
            "{f1} {f2}"
        );
        // A lone site: the second distance is only known to exceed the query radius.
        let cells = [cell(x0, y0)];
        hash.rebuild(&[[x0, y0]]);
        let (f1, f2, _) = View::worley(&hash, &cells, x0 + 0.5, y0, radius);
        assert!(
            (f1 - 0.5).abs() < 1e-4 && (f2 - radius).abs() < 1e-9,
            "{f1} {f2}"
        );
        // Spacing grows with the disc so cells span 2–3 terminal cells.
        assert_eq!(granule_spacing(10.0), 3.6);
        assert!((granule_spacing(46.0) - 3.91).abs() < 1e-9);
        assert_eq!(granule_spacing(200.0), 5.0);
    }

    #[test]
    fn sun_a6_regions_ride_spoerer_latitudes_and_respawn_unseen() {
        let (mut s, mut ctx) = scene(80, 24);
        let first = &s.regions[0];
        assert!((first.lon.to_degrees() + 78.0).abs() < 1e-9);
        let view = view_of(first.lon, s.b0);
        let east_emission = first.loops.iter().flat_map(|l| &l.pts).any(|&p| {
            let q = view.rotate(p);
            classify(q) == Sample::Emission && q[0] < 0.0
        });
        assert!(east_emission, "no loop above the east limb at t = 0");
        let mut respawns = 0;
        for _ in 0..1200 {
            let before = s.regions.clone();
            s.update(0.25, &mut ctx);
            for (old, new) in before.iter().zip(&s.regions) {
                let lat = lat_of(new).to_degrees().abs();
                assert!((5.0..=30.0).contains(&lat), "latitude {lat}");
                if new.omega != old.omega || new.spots[0].rho != old.spots[0].rho {
                    let mut old = old.clone();
                    old.advance(0.25);
                    assert!(
                        old.lon >= RESPAWN_LON,
                        "respawn at {}",
                        old.lon.to_degrees()
                    );
                    let view = view_of(old.lon, s.b0);
                    for sp in &old.spots {
                        assert!(view.rotate(sp.centre)[2] < 0.0);
                    }
                    for &p in old.loops.iter().flat_map(|l| &l.pts) {
                        assert_eq!(classify(view.rotate(p)), Sample::Hidden);
                    }
                    respawns += 1;
                }
            }
        }
        assert!(respawns >= 3, "only {respawns} respawns in 300 s");
    }

    #[test]
    fn sun_a7_granule_pool_and_contrast() {
        let (mut s, mut ctx) = scene(200, 50);
        assert!((s.view.r - 46.0).abs() < 1e-9);
        // 4π·46²/3.91² with s_g = 0.085·R.
        assert_eq!(granule_count(46.0), 1739);
        for _ in 0..120 {
            s.update(0.25, &mut ctx);
            assert_eq!(s.granules.len(), 1739);
            assert!(s.granules.capacity() <= GRANULES_MAX);
            for g in &s.granules {
                assert!((GRANULE_LIFE.0..=GRANULE_LIFE.1).contains(&g.life));
            }
        }
        s.show_regions = false;
        s.rasterise(true);
        let rms = |lo: f64, hi: f64| {
            let ratio: Vec<f64> = disc_values(&s)
                .into_iter()
                .filter(|p| p.0 > lo && p.0 < hi)
                .map(|(mu, v)| photosphere_intensity(v) / limb_intensity(mu))
                .collect();
            let m = mean(ratio.iter().copied());
            mean(ratio.iter().map(|r| (r - m) * (r - m))).sqrt()
        };
        let (centre, limb) = (rms(0.8, 1.01), rms(0.15, 0.3));
        assert!((0.06..=0.18).contains(&centre), "centre rms {centre:.3}");
        assert!(limb < 0.6 * centre, "limb rms {limb:.3} vs {centre:.3}");
        for (f1, dd) in [(0.45, 0.0), (0.45, 0.1), (0.9, 0.05), (2.0, 0.1)] {
            let s_g = granule_spacing(46.0);
            assert!(granule_pattern(f1 * s_g, (f1 + dd) * s_g, 1.0, s_g) < 0.26);
        }
        for (f1, dd, w) in [(0.0, 0.3, 0.0), (1.0, 0.5, 1.0), (0.2, 2.0, 0.5)] {
            let s_g = granule_spacing(46.0);
            assert!(granule_pattern(f1 * s_g, (f1 + dd) * s_g, w, s_g) >= 0.55);
        }
    }

    #[test]
    fn sun_gen_r13_label_and_tiny_sizes() {
        // Right-aligned, clear of the title: beside the full "Solar surface · convection
        // cells" (32 cells, from 57 wide) the room is w − 38, 19 chars at 57, so Teff is
        // dropped there.
        for (w, want) in [(80, " day 3.0  Teff 5772 K "), (57, " day 3.0 ")] {
            let (mut s, mut ctx) = scene(w, 24);
            for _ in 0..40 {
                s.update(0.25, &mut ctx);
            }
            let mut canvas = Canvas::new(Size::new(w, 24), GlyphMode::Ascii);
            s.render(&mut canvas);
            assert!(canvas.to_lines()[23].ends_with(want), "{w}");
        }
        let (mut s, mut ctx) = scene(80, 24);
        for (w, h) in [(1, 1), (3, 2), (50, 12), (300, 1)] {
            s.resize(Size::new(w, h), &mut ctx);
            s.update(0.25, &mut ctx);
            let mut canvas = Canvas::new(Size::new(w, h), GlyphMode::Unicode);
            s.render(&mut canvas);
        }
    }

    #[test]
    fn sun_a8_slit_spectrum_shows_lines_and_spot_bands_clear_of_the_sun() {
        assert!(quiet(80, 24).view.inset.is_none());
        let mut s = quiet(200, 50);
        let inset = s.view.inset.expect("inset at 200x50");
        let v = &s.view;
        // Clear of the disc and loops (in pixels: rows are two tall), above the title.
        for y in 2 * inset.y..2 * (inset.y + inset.h) {
            for x in inset.x..inset.x + inset.w {
                let d = (x as f64 + 0.5 - v.cx).hypot(y as f64 + 0.5 - v.cy);
                assert!(d >= INSET_CLEAR * v.r, "({x}, {y}) at {d}");
            }
        }
        assert!(inset.y + inset.h <= 50 - 2);
        // Every line in range falls on its nearest column (G and H-γ, 3 nm apart, share
        // one at this resolution).
        let step = (SPECTRUM_NM.1 - SPECTRUM_NM.0) / (inset.w - 1) as f64;
        for l in FRAUNHOFER_NM {
            let u = ((l - SPECTRUM_NM.0) / step).round() as usize;
            assert!(inset.is_line(u), "{l} nm");
        }
        let lines = (0..inset.w).filter(|&u| inset.is_line(u)).count();
        assert!(lines >= FRAUNHOFER_NM.len() - 1 && lines <= FRAUNHOFER_NM.len());
        let render = |s: &SolarSurface| {
            let mut c = Canvas::new(Size::new(200, 50), GlyphMode::Unicode);
            s.render(&mut c);
            c
        };
        let cell = |c: &Canvas, u: usize, row: usize| {
            *c.get((inset.x + u) as i32, (inset.y + row) as i32).unwrap()
        };
        // A continuous band: every cell of a disc row is coloured, and only the line
        // columns carry a thin dark line.
        let mid = inset.h / 2;
        let c = render(&s);
        for u in 0..inset.w {
            let k = cell(&c, u, mid);
            assert_eq!(k.bg.role, Role::Phase, "column {u}");
            let thin = ['│', '▏', '▕'].contains(&k.ch);
            assert_eq!(thin, inset.is_line(u), "column {u}: {:?}", k.ch);
            if thin {
                assert_eq!(k.ink, DARK_INK);
            }
        }
        // Red at the right, violet at the left.
        let hue = |u: usize| ink::phase_value(cell(&c, u, mid).bg);
        assert!(hue(inset.w - 1) < 0.1 * TAU && hue(0) > 0.7 * TAU);
        // An umbra on the slit is a dark stripe with heavy lines; the next row is not.
        let row = 3;
        s.view.slit[row] = SlitRow { i: 0.2, shift: 0 };
        s.view.slit[row + 1] = SlitRow { i: 1.0, shift: -1 };
        let c = render(&s);
        for u in 0..inset.w {
            let (k, next) = (cell(&c, u, row), cell(&c, u, row + 1));
            assert_eq!(k.ch, if inset.is_line(u) { '┃' } else { '▓' }, "column {u}");
            assert_eq!(k.bg.role, Role::Phase);
            // Rising gas shifts the lines blueward.
            assert_eq!(
                next.ch,
                if inset.is_line(u) { '▏' } else { ' ' },
                "column {u}"
            );
        }
        // Off the disc the slit sees nothing.
        s.view.slit[0].i = f32::NAN;
        let c = render(&s);
        assert!((0..inset.w).all(|u| cell(&c, u, 0).bg.role == Role::Background));
        // The slit is marked by its jaws just inside the limb, top and bottom: a dark
        // hairline over the photosphere, and none across the middle of the disc.
        let x = s.view.slit_x() as i32;
        let jaws = (0..50)
            .filter(|&y| {
                c.get(x, y)
                    .is_some_and(|k| k.ch == '│' && k.ink == DARK_INK)
            })
            .collect::<Vec<_>>();
        assert_eq!(jaws.len(), 2 * JAW_CELLS as usize, "{jaws:?}");
        for &y in &jaws {
            assert_eq!(
                c.get(x, y).unwrap().bg.role,
                Role::Ramp(Ramp::Ember),
                "row {y}"
            );
        }
        let (top, bottom) = (jaws[0], jaws[jaws.len() - 1]);
        assert!(
            jaws[1] - top < 2 && bottom - jaws[jaws.len() - 2] < 2,
            "{jaws:?}"
        );
        assert!(!s.view.on_disc(x, 2 * top - 1) && !s.view.on_disc(x, 2 * bottom + 2));
        // Sampled from the photosphere: the Doppler sign follows the granulation.
        s.view.sample_slit();
        assert!(s.view.slit[..inset.h].iter().any(|r| !r.i.is_nan()));
    }

    #[test]
    fn sun_a8_doppler_shift_holds_until_the_excess_falls_back() {
        assert_eq!(doppler_shift(0, 0.03), -1);
        assert_eq!(doppler_shift(0, -0.03), 1);
        assert_eq!(doppler_shift(0, 0.015), 0);
        // Hysteresis: a shifted row near the threshold stays shifted...
        assert_eq!(doppler_shift(-1, 0.015), -1);
        assert_eq!(doppler_shift(1, -0.015), 1);
        // ...until the excess falls inside the hold band, or flips sign past the threshold.
        assert_eq!(doppler_shift(-1, 0.005), 0);
        assert_eq!(doppler_shift(1, -0.005), 0);
        assert_eq!(doppler_shift(-1, -0.03), 1);
        // A wobble about the threshold changes the glyph once, not every frame.
        let mut shift = 0;
        let mut changes = 0;
        for k in 0..40 {
            let excess = 0.02 + 0.004 * if k % 2 == 0 { 1.0 } else { -1.0 };
            let next = doppler_shift(shift, excess);
            changes += i32::from(next != shift);
            shift = next;
        }
        assert_eq!(changes, 1);
    }

    #[test]
    fn sun_a11_prominences_are_filled_sheets_that_stream() {
        // Under the outer arch, the sheet lights the pixels between the arch and the
        // limb: many more loop pixels than the arches alone, and they change over time.
        let (on, _) = scene(200, 50);
        let count = |s: &SolarSurface| clear_of_the_limb(s).len();
        assert!(count(&on) >= 12);
        // Sheet values stay under the arches (so they read over it), above the floor.
        assert!(V_FILL.1 + FILL_SHIMMER < V_LOOP);
        assert!(255.0 * (V_FILL.0 - FILL_SHIMMER) > 3.0 * ink::MIN_VISIBLE_LEVEL);
        assert!(fill_steps(0.3, 46.0) >= 2 && fill_steps(10.0, 1e6) == 64);
        // Streaming: the same frame a quarter second on differs in its sheet pixels.
        let (mut s, mut ctx) = scene(200, 50);
        let before: Vec<Ink> = clear_of_the_limb(&s);
        s.update(0.25, &mut ctx);
        let after: Vec<Ink> = clear_of_the_limb(&s);
        assert_ne!(before, after);
    }

    #[test]
    fn sun_a10_the_chromosphere_is_a_smooth_ring_fading_outward() {
        // At 200×50 as at 80×24: every rim pixel is lit (no breaks, no spikes), and above
        // the rim the glow never brightens outward along a radius.
        for (w, h) in [(200, 50), (80, 24)] {
            let mut ctx = Ctx::new(11, Size::new(w, h));
            let mut s = SolarSurface::new();
            s.resize(Size::new(w, h), &mut ctx);
            s.show_loops = false;
            s.rasterise(true);
            let v = &s.view;
            let level = |x: u16, y: u16| {
                v.pixels
                    .get(i32::from(x), i32::from(y))
                    .filter(|i| i.role == Role::Ramp(Ramp::HAlpha))
                    .map(|i| i.level)
            };
            for rp in v.rim.iter().filter(|rp| f64::from(rp.above) <= RIM_PX) {
                assert!(
                    level(rp.x, rp.y).is_some(),
                    "{w}x{h}: rim gap at {},{}",
                    rp.x,
                    rp.y
                );
            }
            for rp in v.rim.iter().filter(|rp| f64::from(rp.above) > RIM_PX + 1.0) {
                if let (Some(out), Some(inn)) = (level(rp.x, rp.y), level(rp.inward.0, rp.inward.1))
                {
                    assert!(
                        out <= inn,
                        "{w}x{h}: {out} above {inn} at {},{}",
                        rp.x,
                        rp.y
                    );
                }
            }
        }
    }

    #[test]
    fn sun_a9_granules_stand_out_and_umbrae_punch_through() {
        // The photosphere's ramp values: granules gold, lanes ≥ 15 % darker, an orange-red
        // limb and umbrae near the bottom of the ramp.
        let (granule, lane) = (photosphere_value(1.1), photosphere_value(0.9));
        assert!(granule - lane >= 0.15 * granule, "{granule} {lane}");
        assert!((0.25..0.45).contains(&photosphere_value(0.45)));
        assert!(photosphere_value(0.22) < 0.2);
        for k in 0..=100 {
            let i = 1.3 * f64::from(k) / 100.0;
            assert!(
                (photosphere_intensity(photosphere_value(i)) - i).abs() < 1e-9,
                "{i}"
            );
        }
    }
}
