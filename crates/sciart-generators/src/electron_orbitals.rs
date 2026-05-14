//! Hydrogen orbitals as turning Monte-Carlo clouds, sloshing between states
//! (SPEC-002 `electron_orbitals`).
//!
//! Each gallery state `ψ_nlm = R_nl(r)·Y(r̂)` is sampled exactly: every real harmonic here is
//! a product `Θ(cos θ)·Φ(φ)`, so `|ψ|² d³r` factors into three 1-D densities, each
//! tabulated once as an [`InverseCdf`]. Points live in atomic units (a₀) and never depend on
//! the terminal size.
//!
//! Between states the atom passes through the superposition
//! `Ψ = cos χ·ψ_a e^{−iE_a t} + sin χ·ψ_b e^{−iE_b t}`. Its density
//! `|Ψ|² = q + 2c_a c_b ψ_a ψ_b cos ωt` (with `q = c_a²ψ_a² + c_b²ψ_b²`) is drawn by thinning:
//! a per-point fixed uniform `v` proposes points of each slot with probability `p_a`/`p_b`
//! (intensity `M(χ)·q`), and a second fixed uniform `u` keeps a proposal iff
//! `u < |Ψ|²/(2q)`. The visible points then have density exactly `(M(χ)/2)·|Ψ(t)|²`, and
//! because `u`, `v` are fixed the cloud only changes where a threshold is crossed: no shimmer.
//! On screen a radial fade τ(r), shared by both slots, thins the far dust, so the drawn
//! density is exactly `τ(r)·(M(χ)/2)·|Ψ(t)|²`.
//!
//! The only liberty with time is stated on screen: `ω` is the true Bohr frequency `ΔE/h`
//! divided by a round per-line factor that the label prints (`Lyα 121.6 nm  slowed 4×10¹⁵`).
//!
//! Raster convention (M10): points are projected into [`Dots`] at the end of `update` and
//! `resize`; `render` draws the nucleus, blits, redraws the nucleus over a nearly empty
//! node cell and writes the label.
//!
//! Each state has its own camera pose (the orbitals are not all symmetric about z), blended
//! across a transition and rocked on the Timeline clock, so every hold sees its best view
//! at its midpoint; the whole atom turns and scales together, so the physics is
//! unaffected. While both states are on screen the larger one sets the scale, so neither is
//! ever drawn bigger than in its own hold.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, draw, ink};

pub static META: Meta = Meta {
    name: "electron_orbitals",
    category: Category::PhysicalChemistry,
    description: "Hydrogen orbitals as turning Monte-Carlo clouds, sloshing between states",
    title: "Hydrogen atom · electron clouds",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ElectronOrbitals::default())
}

/// Knots of each inverse-CDF table.
const KNOTS: usize = 1024;
/// Radial tables span `[0, R_SPAN·r₉₉.₉]`.
const R_SPAN: f64 = 1.5;
/// Timeline per orbital (s): pure hold, χ 0 → π/4, full sloshing, χ π/4 → π/2 (ORB-A6).
const HOLD: f64 = 6.0;
const RISE: f64 = 2.0;
const BEAT: f64 = 3.0;
const FALL: f64 = 2.0;
/// Points per orbital: `M_k = clamp(round(DENSITY·π(4ρ_k)²), M_MIN, M_MAX)` for an on-screen
/// r₉₀ radius of `ρ_k` rows (4 dots per row), ≈ 0.4 visible points per dot inside r₉₀.
const DENSITY: f64 = 0.8;
const M_MIN: usize = 256;
const M_MAX: usize = 16_384;
/// Slot preparation (sampling or partner-ψ rewrite) per 1/60 s during a hold.
const PREP_PER_TICK: f64 = 256.0;
/// Framing: the r₉₀ sphere of the widest state (4f) spans `FIT` of the [`stage`] height,
/// and state k is drawn at `g_k = (r₉₀,k / r₉₀,4f)^GLIDE` of that: smaller, but never a dot.
const FIT: f64 = 0.75;
const GLIDE: f64 = 0.3;
/// Camera (world unit = the blended r₉₀): every state has its own [`Pose`], blended with
/// weight c_b²; the yaw rocks about it and the pitch wobbles in quadrature, so the atom
/// always moves but no state is ever seen from its blind side.
const DISTANCE: f64 = 6.0;
/// Yaw rock amplitude (rad); peak 0.6·2π/13 s ≈ 16.6°/s.
const ROCK: f64 = 0.6;
/// The narrower rock of states whose nodal plane contains the vertical axis (2p_x,
/// 3d_xz): turned further from edge-on, the plane fills in (at 80×24 the 2p_x node
/// column holds up to 0.86 of the lobe column's dots at ±0.6, ≤ 0.26 at ±0.25; ORB-A7).
/// Phased a quarter cycle later, so these states are broadside at mid-hold.
const ROCK_NODE: f64 = 0.25;
/// The wider rock of 4f_xyz about its 45° yaw: at mid-hold it reaches the axis-aligned
/// view (yaw π/4 + 0.8 ≈ π/2), where the vertical nodal planes are edge-on and the lobes
/// separate into four `+#` blocks split by a cross; at ±0.6 the centre pairs stay merged
/// into two slabs.
const ROCK_XYZ: f64 = 0.8;
/// One period of the rock and the pitch wobble is one orbital's visit, on the Timeline
/// clock, so every hold sees the same part of it (and the 104 s loop is exactly periodic).
/// The rock peaks at mid-hold (`MID_HOLD` into the visit); on a free 16 s clock a hold's
/// phase was arbitrary and 4f_xyz could spend its hold merged into two slabs.
const ROCK_PERIOD: f64 = HOLD + RISE + BEAT + FALL;
const MID_HOLD: f64 = HOLD / 2.0;
/// Pitch of the states symmetric about their z axis: low, so m = 0 nodal planes stay open.
const PITCH_LOW: f64 = 0.15;
/// Pitch of 4f_xyz: level, so the wobble tilts it equally above and below its z = 0 nodal
/// plane and the two tiers of lobes stay split by an open gap through the nucleus. Raised
/// pitches fill that gap in: at 80×24 the nucleus row holds ≤ 0.46 of the fullest row's
/// cells over the camera cycle at 0, ≤ 0.61 at 0.05 and up to 1.0 (median 0.83) at 0.35.
const PITCH_XYZ: f64 = 0.0;
/// Pitch wobble amplitude (rad). In quadrature with the rock on the same period, so the
/// camera never stops: its angular speed stays ≥ 0.10·2π/13 s ≈ 2.8°/s.
const PITCH_WOBBLE: f64 = 0.10;
/// Display fade of the far dust, so the cloud ends inside the frame: a point is proposed
/// iff `v < p_k·τ(r)`, where the taper τ is 1 inside `r_in` and falls linearly to 0 at
/// `r_out`. The window is the r₈₈ and r₉₅ of the states on screen, blended geometrically
/// with weight c_b² and shared by both slots, so the displayed density is exactly
/// `τ(r)·(M(χ)/2)|Ψ|²` (ORB-A4). Given a proposal, `v/p_k` is a fixed uniform independent
/// of `u`, so the fade is shimmer-free. (r₉₀, r₉₇) left lone single-dot braille cells a
/// few columns out from the lobes, which read as grain rather than charge cloud.
const R_FADE: (f64, f64) = (0.88, 0.95);
/// Planck constant (eV·s, exact since the 2019 SI): the true Bohr frequency is `ΔE/h`.
const H_EV_S: f64 = 4.135_667_696e-15;
/// Hydrogen Rydberg energy with the reduced mass, `R∞hc·μ/mₑ` (eV), and `hc` (eV·nm; the
/// label strings are checked against it).
const RYDBERG_H: f64 = 13.598_287;
#[cfg(test)]
const HC: f64 = 1239.842;
/// Glow (ORB-A9): a ~`GLOW_TAU` s screen-space exposure of the visible points, each spread
/// over its pixel and the four next to it (`SPLAT`: no wider, so nodes stay dark; the
/// splat and the camera's turn smooth the Monte-Carlo mottling, time alone does not, as
/// the points are fixed). Tone-mapped by an asinh curve (knee and white in expected points
/// per pixel per frame, so a lobe's core reaches white and its edge sits near the knee) to
/// `g ∈ [0, 1]` and drawn in `GLOW_RAMP` at `HAZE_MAX·smoothstep(g)`: one soft ramp from
/// the fringe to the core, no cut and no plateau; blank below `ink::MIN_VISIBLE_LEVEL`.
/// The cap keeps the phase-coloured dots ≥ 2.5× (astro) and ≥ 2× (mono) brighter than any
/// glow behind them, and `Ice`'s deep blue shares no hue with the hold phases (±π/2).
/// The glow shows stationary states only: it eases in over the first `GLOW_IN` s of a hold
/// and out over its last `GLOW_OUT` s, and is off through the transition. A stationary
/// state's time-mean is its instantaneous `|ψ|²`, while in a beat an exposure would only
/// show the mean cloud lagging the slosh; and a transition's phases sweep through 0,
/// which mono greys near black (RND-R10), so dots there would read as holes in a glow.
const GLOW_TAU: f64 = 1.0;
const GLOW_KNEE: f32 = 0.5;
const GLOW_WHITE: f32 = 8.0;
const GLOW_FLOOR: f64 = 0.25;
const HAZE_MAX: u8 = 40;
const GLOW_RAMP: Ramp = Ramp::Ice;
const GLOW_IN: f64 = 1.5;
/// The glow's own radial window (charge quantiles): points expose it fully inside `r₅₀`
/// and not at all beyond `r₉₀`, so each lobe glows at its heart and the glow feathers
/// out across the outer half of the cloud, well inside the dots' own `R_FADE` edge.
const GLOW_FADE: (f64, f64) = (0.5, 0.9);
const GLOW_OUT: f64 = 1.0;
/// The glow's splat, `(dx, dy, weight)` in pixels: half in the point's pixel, an eighth in
/// each edge neighbour.
const SPLAT: [(i32, i32, f32); 5] = [
    (0, 0, 0.5),
    (-1, 0, 0.125),
    (1, 0, 0.125),
    (0, -1, 0.125),
    (0, 1, 0.125),
];
const NUCLEUS: Glyph = Glyph::new('•', 'o');
const NUCLEUS_INK: Ink = Ink::new(Role::Accent, 180);
/// The nucleus is drawn over its cell when at most this many of the 8 dots are lit.
const NUCLEUS_MAX_DOTS: usize = 2;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// Number of gallery states.
const K: usize = GALLERY.len();

/// The gallery's real hydrogen states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orb {
    S1,
    P2z,
    D3z2,
    F4z3,
    D3xz,
    F4xyz,
    D3xy,
    P2x,
}

/// A closed dipole-allowed loop (Δl = ±1), ORB-A6.
pub(crate) const GALLERY: [Orb; 8] = [
    Orb::S1,
    Orb::P2z,
    Orb::D3z2,
    Orb::F4z3,
    Orb::D3xz,
    Orb::F4xyz,
    Orb::D3xy,
    Orb::P2x,
];

impl Orb {
    pub(crate) const fn n(self) -> u32 {
        match self {
            Orb::S1 => 1,
            Orb::P2z | Orb::P2x => 2,
            Orb::D3z2 | Orb::D3xz | Orb::D3xy => 3,
            Orb::F4z3 | Orb::F4xyz => 4,
        }
    }

    /// Every gallery state is circular (l = n − 1), so `R_nl` has no radial node: all
    /// the structure on screen is angular.
    pub(crate) const fn l(self) -> u32 {
        self.n() - 1
    }

    /// Label name (Unicode, ASCII).
    const fn name(self) -> (&'static str, &'static str) {
        match self {
            Orb::S1 => ("1s", "1s"),
            Orb::P2z => ("2p_z", "2p_z"),
            Orb::D3z2 => ("3d_z²", "3d_z2"),
            Orb::F4z3 => ("4f_z³", "4f_z3"),
            Orb::D3xz => ("3d_xz", "3d_xz"),
            Orb::F4xyz => ("4f_xyz", "4f_xyz"),
            Orb::D3xy => ("3d_xy", "3d_xy"),
            Orb::P2x => ("2p_x", "2p_x"),
        }
    }

    /// Camera pose. States symmetric about z sit low; 2p_x and 3d_xz are broadside at
    /// yaw 0 (the eye looks along world z, so world x is screen horizontal) and rock
    /// less; 4f_xyz is turned 45° and kept nearly level, so its z = 0 nodal plane shows as
    /// a gap through the nucleus between two tiers of lobes; 3d_xy lies in the world
    /// horizontal plane, so it is seen from well above.
    pub(crate) const fn pose(self) -> Pose {
        let (yaw, pitch, rock, phase) = match self {
            Orb::S1 | Orb::P2z | Orb::D3z2 | Orb::F4z3 => (0.0, PITCH_LOW, ROCK, 0.0),
            Orb::P2x | Orb::D3xz => (0.0, PITCH_LOW, ROCK_NODE, FRAC_PI_2),
            Orb::F4xyz => (FRAC_PI_4, PITCH_XYZ, ROCK_XYZ, 0.0),
            Orb::D3xy => (0.0, 1.05, ROCK, 0.0),
        };
        Pose {
            yaw,
            pitch,
            rock,
            phase,
        }
    }
}

/// A state's camera pose (rad): yaw offset α_k, pitch p_k, yaw rock amplitude and the
/// rock's phase at mid-hold (0: at its extreme `α_k + rock`; π/2: passing through α_k).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    yaw: f64,
    pitch: f64,
    rock: f64,
    phase: f64,
}

impl Pose {
    /// Component-wise lerp, exactly `self` at `w = 0` and `other` at `w = 1`.
    fn lerp(self, other: Pose, w: f64) -> Pose {
        Pose {
            yaw: ease::lerp(self.yaw, other.yaw, w),
            pitch: ease::lerp(self.pitch, other.pitch, w),
            rock: ease::lerp(self.rock, other.rock, w),
            phase: ease::lerp(self.phase, other.phase, w),
        }
    }

    /// The camera at Timeline clock `t`: the yaw rocks about α with the phase set at
    /// mid-hold, and the pitch wobbles about p a quarter period behind, so one of them is
    /// always at full speed.
    fn camera(self, t: f64) -> Camera {
        let (s, c) = (TAU * (t - MID_HOLD) / ROCK_PERIOD + self.phase).sin_cos_det();
        Camera {
            yaw: self.yaw + self.rock * c,
            pitch: self.pitch - PITCH_WOBBLE * s,
            distance: DISTANCE,
        }
    }
}

/// A radial display window (a₀): τ(r) is 1 inside `inner` and falls linearly to 0 at
/// `outer` (see `R_FADE`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fade {
    inner: f64,
    outer: f64,
}

impl Fade {
    /// No fade (τ ≡ 1): the exact sampler.
    #[cfg(test)]
    pub(crate) const NONE: Fade = Fade {
        inner: f64::INFINITY,
        outer: f64::INFINITY,
    };

    /// The window of one orbital between the radii holding fractions `q.0` and `q.1` of
    /// its charge.
    fn window(orb: Orb, q: (f64, f64)) -> Self {
        let r = |q: f64| radius_quantile(orb.n(), orb.l(), q);
        Self {
            inner: r(q.0),
            outer: r(q.1),
        }
    }

    /// Geometric blend by `w = c_b²`, exactly `a` at `w = 0` and `b` at `w = 1` (so the
    /// display is continuous across every phase boundary, ORB-A5).
    fn blend(a: Fade, b: Fade, w: f64) -> Self {
        if w <= 0.0 {
            return a;
        }
        if w >= 1.0 {
            return b;
        }
        let geo = |x: f64, y: f64| ease::lerp(x.ln(), y.ln(), w).exp();
        Self {
            inner: geo(a.inner, b.inner),
            outer: geo(a.outer, b.outer),
        }
    }

    /// τ(r).
    fn taper(self, r: f64) -> f64 {
        if r <= self.inner {
            1.0
        } else {
            ((self.outer - r) / (self.outer - self.inner)).clamp(0.0, 1.0)
        }
    }
}

// ---------------------------------------------------------------------------------------
// Wavefunctions (ORB-A1..A3)
// ---------------------------------------------------------------------------------------

/// Generalised Laguerre `L_k^{(α)}(x)` by its three-term recurrence.
fn laguerre(k: u32, alpha: f64, x: f64) -> f64 {
    let (mut prev, mut cur) = (1.0, 1.0 + alpha - x);
    if k == 0 {
        return prev;
    }
    for j in 1..k {
        let j = f64::from(j);
        let next = ((2.0 * j + 1.0 + alpha - x) * cur - (j + alpha) * prev) / (j + 1.0);
        (prev, cur) = (cur, next);
    }
    cur
}

fn factorial(n: u32) -> f64 {
    (1..=n).map(f64::from).product()
}

/// `R_nl(r)/r^l`: finite at the origin, so `ψ` needs no direction there.
fn radial_reduced(n: u32, l: u32, r: f64) -> f64 {
    let nf = f64::from(n);
    let norm = ((2.0 / nf).powi(3) * factorial(n - l - 1) / (2.0 * nf * factorial(n + l))).sqrt();
    let rho = 2.0 * r / nf;
    norm * (2.0 / nf).powi(l as i32)
        * (-rho / 2.0).exp()
        * laguerre(n - l - 1, f64::from(2 * l + 1), rho)
}

/// Hydrogen radial function `R_nl(r)` (a₀ = 1).
pub(crate) fn radial(n: u32, l: u32, r: f64) -> f64 {
    radial_reduced(n, l, r) * r.powi(l as i32)
}

/// Solid harmonic `r^l·Y(r̂)`: a polynomial, so `ψ = (R_nl/r^l)·solid` is exact at r = 0,
/// and on a unit direction it is the real spherical harmonic `Y` itself.
fn solid(orb: Orb, [x, y, z]: [f64; 3]) -> f64 {
    let r2 = x * x + y * y + z * z;
    let k = |c: f64| (c / PI).sqrt();
    match orb {
        Orb::S1 => k(1.0 / 4.0),
        Orb::P2z => k(3.0 / 4.0) * z,
        Orb::P2x => k(3.0 / 4.0) * x,
        Orb::D3z2 => k(5.0 / 16.0) * (3.0 * z * z - r2),
        Orb::D3xz => k(15.0 / 4.0) * x * z,
        Orb::D3xy => k(15.0 / 4.0) * x * y,
        Orb::F4z3 => k(7.0 / 16.0) * z * (5.0 * z * z - 3.0 * r2),
        Orb::F4xyz => k(105.0 / 4.0) * x * y * z,
    }
}

/// `ψ(p)` in atomic units.
pub(crate) fn psi(orb: Orb, p: [f64; 3]) -> f64 {
    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    radial_reduced(orb.n(), orb.l(), r) * solid(orb, p)
}

/// Radius enclosing probability `q` (trapezoid on a 0.01 a₀ grid, linear inside a step).
pub(crate) fn radius_quantile(n: u32, l: u32, q: f64) -> f64 {
    const H: f64 = 0.01;
    let density = |r: f64| (r * radial(n, l, r)).powi(2);
    let (mut acc, mut r) = (0.0, 0.0);
    while r < 500.0 {
        let step = 0.5 * H * (density(r) + density(r + H));
        if acc + step >= q {
            return r + H * (q - acc) / step;
        }
        acc += step;
        r += H;
    }
    r
}

/// Energy gap `|E_a − E_b|` (eV).
fn gap(na: u32, nb: u32) -> f64 {
    let inv2 = |n: u32| 1.0 / f64::from(n * n);
    RYDBERG_H * (inv2(na) - inv2(nb)).abs()
}

/// Vacuum wavelength of the `n_a ↔ n_b` line (nm).
#[cfg(test)]
pub(crate) fn line_nm(na: u32, nb: u32) -> f64 {
    HC / gap(na, nb)
}

/// An α line the gallery crosses (n ↔ n + 1): its label, and the stated factor by which
/// the display slows time for its beat.
#[derive(Debug)]
pub(crate) struct Line {
    /// Label parts (Unicode, ASCII): the line, and the slow-down it is shown at.
    name: (&'static str, &'static str),
    slowed: (&'static str, &'static str),
    /// The cloud sloshes at the true Bohr frequency `ΔE/h` divided by this: one honest time
    /// dilation per transition, round so the label can say it exactly. Chosen for a calm
    /// beat (0.62 / 0.46 / 0.32 Hz) that stays monotone in ΔE, as the physics is.
    slowdown: f64,
}

/// Lyα, Hα, Pα (index: lower n − 1).
const LINES: [Line; 3] = [
    Line {
        name: ("Lyα 121.6 nm", "Ly-a 121.6 nm"),
        slowed: ("slowed 4×10¹⁵", "slowed 4e15"),
        slowdown: 4e15,
    },
    Line {
        name: ("Hα 656.5 nm", "H-a 656.5 nm"),
        slowed: ("slowed 10¹⁵", "slowed 1e15"),
        slowdown: 1e15,
    },
    Line {
        name: ("Pα 1876 nm", "P-a 1876 nm"),
        slowed: ("slowed 5×10¹⁴", "slowed 5e14"),
        slowdown: 5e14,
    },
];

/// The line of an `n_a ↔ n_b` transition, if it is one the gallery crosses (ORB-A3 checks
/// every gallery pair has one); any other pair shows no line and no beat rather than a
/// wrong one.
pub(crate) fn line(na: u32, nb: u32) -> Option<&'static Line> {
    match (na.min(nb), na.max(nb)) {
        (1, 2) => Some(&LINES[0]),
        (2, 3) => Some(&LINES[1]),
        (3, 4) => Some(&LINES[2]),
        _ => None,
    }
}

/// Display beat frequency (Hz): the true Bohr frequency `ΔE/h` slowed by the line's
/// stated factor.
pub(crate) fn beat_hz(na: u32, nb: u32) -> f64 {
    line(na, nb).map_or(0.0, |l| gap(na, nb) / H_EV_S / l.slowdown)
}

/// Signed display beat `ω = (E_b − E_a)/ħ` on the slowed clock (rad/s) for a transition
/// from `n_a` to `n_b` (ORB-A8): positive going up (`E_n = −R/n²` rises with n), negative
/// going down, so the phase winds the right way in both halves of the loop. `|Ψ|²` sees
/// only `cos ωt`, which is even, so the density does not depend on the sign.
pub(crate) fn beat_omega(na: u32, nb: u32) -> f64 {
    let sign = if nb > na { 1.0 } else { -1.0 };
    sign * TAU * beat_hz(na, nb)
}

// ---------------------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------------------

/// Exact separable sampler of `|ψ|²`: `r²R² dr · Θ² d cos θ · Φ² dφ`.
#[derive(Debug)]
struct Sampler {
    r: InverseCdf<KNOTS>,
    mu: InverseCdf<KNOTS>,
    phi: InverseCdf<KNOTS>,
}

impl Sampler {
    /// The angular factors are read off [`solid`] along a fixed meridian (`φ = π/4`) and a
    /// fixed parallel (`cos θ = 1/2`), where no gallery factor vanishes.
    fn new(orb: Orb) -> Self {
        let (n, l) = (orb.n(), orb.l());
        let r_max = R_SPAN * radius_quantile(n, l, 0.999);
        let meridian = |mu: f64| {
            let s = (1.0 - mu * mu).max(0.0).sqrt();
            solid(orb, [s * FRAC_PI_4.cos_det(), s * FRAC_PI_4.sin_det(), mu]).powi(2)
        };
        let parallel = |phi: f64| {
            let s = 0.75f64.sqrt();
            solid(orb, [s * phi.cos_det(), s * phi.sin_det(), 0.5]).powi(2)
        };
        Self {
            r: InverseCdf::build(0.0, r_max, |r| (r * radial(n, l, r)).powi(2)),
            mu: InverseCdf::build(-1.0, 1.0, meridian),
            phi: InverseCdf::build(0.0, TAU, parallel),
        }
    }

    fn sample(&self, rng: &mut SplitMix64) -> [f64; 3] {
        let r = self.r.sample(rng.next_f64());
        let mu = self.mu.sample(rng.next_f64());
        let phi = self.phi.sample(rng.next_f64());
        let s = r * (1.0 - mu * mu).max(0.0).sqrt();
        [s * phi.cos_det(), s * phi.sin_det(), r * mu]
    }
}

/// Samples of one orbital (SoA, f32 storage), with its fixed uniforms and the ψ of the
/// partner state it is currently paired with.
#[derive(Debug)]
struct Slot {
    orb: usize,
    partner: usize,
    x: Vec<f32>,
    y: Vec<f32>,
    z: Vec<f32>,
    psi: Vec<f32>,
    other: Vec<f32>,
    u: Vec<f32>,
    v: Vec<f32>,
    /// Points `< partner_done` hold `other = ψ_partner`.
    partner_done: usize,
}

impl Slot {
    fn with_capacity(orb: usize, partner: usize) -> Self {
        let buf = || Vec::with_capacity(M_MAX);
        Self {
            orb,
            partner,
            x: buf(),
            y: buf(),
            z: buf(),
            psi: buf(),
            other: buf(),
            u: buf(),
            v: buf(),
            partner_done: 0,
        }
    }

    fn len(&self) -> usize {
        self.x.len()
    }

    fn pos(&self, i: usize) -> [f64; 3] {
        [self.x[i], self.y[i], self.z[i]].map(f64::from)
    }

    /// Start over with a new orbital (keeps the capacity).
    fn reset(&mut self, orb: usize, partner: usize) {
        for b in [
            &mut self.x,
            &mut self.y,
            &mut self.z,
            &mut self.psi,
            &mut self.other,
            &mut self.u,
            &mut self.v,
        ] {
            b.clear();
        }
        (self.orb, self.partner, self.partner_done) = (orb, partner, 0);
    }

    fn repartner(&mut self, partner: usize) {
        (self.partner, self.partner_done) = (partner, 0);
    }

    /// Spend up to `budget` points of work sampling up to `target` points, then rewriting
    /// partner ψ; returns the unspent budget.
    fn prepare(
        &mut self,
        sampler: &Sampler,
        target: usize,
        rng: &mut SplitMix64,
        mut budget: usize,
    ) -> usize {
        let (orb, partner) = (GALLERY[self.orb], GALLERY[self.partner]);
        while budget > 0 && self.partner_done < self.len() {
            let i = self.partner_done;
            self.other[i] = psi(partner, self.pos(i)) as f32;
            self.partner_done += 1;
            budget -= 1;
        }
        while budget > 0 && self.len() < target.min(M_MAX) {
            let p = sampler.sample(rng);
            let p32 = p.map(|v| v as f32);
            self.x.push(p32[0]);
            self.y.push(p32[1]);
            self.z.push(p32[2]);
            // ψ from the stored (f32) position, so every later evaluation agrees.
            let p = p32.map(f64::from);
            self.psi.push(psi(orb, p) as f32);
            self.other.push(psi(partner, p) as f32);
            self.u.push(rng.next_f64() as f32);
            self.v.push(rng.next_f64() as f32);
            self.partner_done += 1;
            budget -= 1;
        }
        budget
    }
}

// ---------------------------------------------------------------------------------------
// Superposition (ORB-A4, ORB-A5)
// ---------------------------------------------------------------------------------------

/// One orbital's visit: pure hold, χ 0 → π/4, full sloshing, χ π/4 → π/2 (ORB-A6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Hold,
    Rise,
    Beat,
    Fall,
}

impl Phase {
    const ALL: [Phase; 4] = [Phase::Hold, Phase::Rise, Phase::Beat, Phase::Fall];

    /// The phase of Timeline beat `index` (four beats per orbital).
    fn from_index(index: usize) -> Self {
        Self::ALL[index % Self::ALL.len()]
    }

    fn next(self) -> Self {
        match self {
            Phase::Hold => Phase::Rise,
            Phase::Rise => Phase::Beat,
            Phase::Beat => Phase::Fall,
            Phase::Fall => Phase::Hold,
        }
    }

    /// Time from the rise start to this phase's start (s): the origin of ωt.
    fn offset(self) -> f64 {
        match self {
            Phase::Hold | Phase::Rise => 0.0,
            Phase::Beat => RISE,
            Phase::Fall => RISE + BEAT,
        }
    }
}

/// Mixing angle for a phase at `progress ∈ [0, 1]`; exactly π/2 at the end of the fall.
pub(crate) fn chi(phase: Phase, progress: f64) -> f64 {
    match phase {
        Phase::Hold => 0.0,
        Phase::Rise => FRAC_PI_4 * ease::smoothstep(progress),
        Phase::Beat => FRAC_PI_4,
        Phase::Fall => FRAC_PI_4 + FRAC_PI_4 * ease::smoothstep(progress),
    }
}

/// The display sampler's state for `Ψ = c_a ψ_a e^{−iE_a t} + c_b ψ_b e^{−iE_b t}`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mixture {
    pub(crate) c_a: f64,
    pub(crate) c_b: f64,
    pub(crate) cos_wt: f64,
    /// Proposal probabilities of the two slots: `p_k = M(χ)c_k²/M_k`, so `p_a + p_b = 1`
    /// with the harmonic mixture size `M(χ) = (c_a²/M_a + c_b²/M_b)⁻¹`.
    pub(crate) p_a: f64,
    pub(crate) p_b: f64,
}

impl Mixture {
    /// Exact at the endpoints: χ = 0 gives `c = (1, 0)`, `p = (1, 0)`, and χ = π/2 the reverse,
    /// so the (prev, a) pair at π/2 and the (a, next) pair at 0 show the same points.
    pub(crate) fn new(chi: f64, m_a: usize, m_b: usize, cos_wt: f64) -> Self {
        let (m_a, m_b) = (m_a as f64, m_b as f64);
        let (c_a, c_b, p_a, p_b) = if chi <= 0.0 {
            (1.0, 0.0, 1.0, 0.0)
        } else if chi >= FRAC_PI_2 {
            (0.0, 1.0, 0.0, 1.0)
        } else {
            let (s, c) = chi.sin_cos_det();
            let m = 1.0 / (c * c / m_a + s * s / m_b);
            (c, s, m * c * c / m_a, m * s * s / m_b)
        };
        Self {
            c_a,
            c_b,
            cos_wt,
            p_a,
            p_b,
        }
    }
}

/// Whether a point shows: proposed iff `v < p`, kept iff `u < w/2` with
/// `w = |Ψ|²/q ∈ [0, 2]`. Symmetric in (own, other), so both slots use it.
pub(crate) fn visible(p: f64, c: (f64, f64), cos_wt: f64, psi: (f64, f64), u: f64, v: f64) -> bool {
    if v >= p {
        return false;
    }
    let (a, b) = (c.0 * psi.0, c.1 * psi.1);
    let q = a * a + b * b;
    if !(q > 0.0 && q.is_finite()) {
        return false;
    }
    u < 0.5 + a * b * cos_wt / q
}

/// Phase of the wavefunction at a point, `arg(i·e^{−ig(t)}Ψ)` (ORB-A8), for amplitudes
/// `c = (c_a, c_b)`, real `psi = (ψ_a, ψ_b)` and the terms' phases `theta = (θ_a, θ_b)`
/// from [`gauge`]. The constant `i` puts the two signs of a real state at ±π/2 rather than
/// 0 and π, a half turn apart and equally far from 0, so every theme (mono included:
/// RND-R10 greys phase by its distance from 0) draws the two lobes of a real state equally
/// bright, as their equal `|ψ|²` demands.
pub(crate) fn phase_of(c: (f64, f64), psi: (f64, f64), theta: (f64, f64)) -> f64 {
    let (a, b) = (c.0 * psi.0, c.1 * psi.1);
    let (sa, ca) = theta.0.sin_cos_det();
    let (sb, cb) = theta.1.sin_cos_det();
    FRAC_PI_2 + (a * sa + b * sb).atan2(a * ca + b * cb)
}

/// The terms' phases `(θ_a, θ_b)` at `since` s into a transition with beat `omega`
/// (ORB-A8). Only their difference `θ_a − θ_b = ωt` is physical (it is the phase in
/// `|Ψ|²`); a common phase is unobservable, so where one term alone is non-zero the hue
/// is a choice of gauge. This one is `θ_a = ωT·h(t/T)` over the transition's `T` s, with
/// `h(x) = x²(2 − x)`: `θ_a` starts at 0 at rest (`h′(0) = 0`), so the outgoing state
/// keeps its hold colours as the rise begins, and `θ_b = θ_a − ωt` ends at 0 at rest
/// (`h(1) = h′(1) = 1`), so the incoming state arrives in its hold colours without a
/// jump. `0 ≤ h′ ≤ 4/3`, so neither lone term's hue ever turns faster than `4|ω|/3`: the
/// beat stays the fastest colour motion on screen. (The mean-energy gauge `θ_a = c_b²ωt`,
/// `θ_b = −c_a²ωt` wound the lone terms 2.5|ω| rad away by the fall's start and back
/// over its 2 s, at up to ~3.4|ω|.)
pub(crate) fn gauge(omega: f64, since: f64) -> (f64, f64) {
    const SPAN: f64 = RISE + BEAT + FALL;
    let x = (since / SPAN).clamp(0.0, 1.0);
    let theta_a = omega * SPAN * x * x * (2.0 - x);
    (theta_a, theta_a - omega * since)
}

// ---------------------------------------------------------------------------------------
// Atom: slots + Timeline (display-independent apart from the point counts M_k)
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct Atom {
    samplers: Box<[Sampler]>,
    r90: [f64; K],
    /// Display fade window per orbital (`r₈₈`, `r₉₅`, a₀; see `R_FADE`).
    r_fade: [Fade; K],
    /// Glow fade window per orbital (`r₅₀`, `r₉₀`, a₀; see `GLOW_FADE`).
    r_glow: [Fade; K],
    /// `slots[cur]` holds `GALLERY[at]`; the other holds `GALLERY[at + 1]`.
    slots: [Slot; 2],
    cur: usize,
    timeline: Timeline,
    /// Timeline clock (s), starting at the seeded orbital's hold.
    t: f64,
    at: usize,
    phase: Phase,
    /// Points per orbital for the current display.
    m: [usize; K],
    mix: Mixture,
    /// The terms' phases `(θ_a, θ_b)` from [`gauge`] (0 in a hold), for the phase
    /// colouring; `θ_a − θ_b = ωt` since the rise.
    theta: (f64, f64),
    rng: SplitMix64,
}

impl Atom {
    /// Start at `GALLERY[first]`'s hold with both slots sampled for a `size` [`stage`].
    pub(crate) fn new(first: usize, size: Size, rng: SplitMix64) -> Self {
        let at = first % GALLERY.len();
        let r90 = GALLERY.map(|o| radius_quantile(o.n(), o.l(), 0.9));
        let m = counts(size, &r90);
        let next = (at + 1) % GALLERY.len();
        let phases: Vec<_> = GALLERY
            .iter()
            .flat_map(|&o| {
                [
                    (o.name().1, HOLD),
                    ("rise", RISE),
                    ("beat", BEAT),
                    ("fall", FALL),
                ]
            })
            .collect();
        let mut atom = Self {
            samplers: GALLERY.iter().map(|&o| Sampler::new(o)).collect(),
            r90,
            r_fade: GALLERY.map(|orb| Fade::window(orb, R_FADE)),
            r_glow: GALLERY.map(|orb| Fade::window(orb, GLOW_FADE)),
            slots: [Slot::with_capacity(at, next), Slot::with_capacity(next, at)],
            cur: 0,
            timeline: Timeline::new(&phases),
            t: (HOLD + RISE + BEAT + FALL) * at as f64,
            at,
            phase: Phase::Hold,
            m,
            mix: Mixture::new(0.0, m[at], m[next], 1.0),
            theta: (0.0, 0.0),
            rng,
        };
        atom.prepare(usize::MAX);
        atom
    }

    fn next(&self) -> usize {
        (self.at + 1) % GALLERY.len()
    }

    /// New point counts for a resized display. The live slot is topped up at once; so is
    /// the free slot outside a hold (it is on screen), while in a hold it is left to the
    /// per-tick budget and the synchronous completion at the rise.
    pub(crate) fn resize(&mut self, size: Size) {
        let m = counts(size, &self.r90);
        self.m = m;
        self.mix = Mixture::new(
            chi(self.phase, self.timeline.at(self.t).progress),
            m[self.at],
            m[self.next()],
            self.mix.cos_wt,
        );
        if self.phase == Phase::Hold {
            let live = &mut self.slots[self.cur];
            live.prepare(
                &self.samplers[live.orb],
                m[live.orb],
                &mut self.rng,
                usize::MAX,
            );
        } else {
            self.prepare(usize::MAX);
        }
    }

    /// Spend `budget` points of work: the free slot first (needed at the rise), then the
    /// live slot's top-up and partner rewrite.
    fn prepare(&mut self, budget: usize) {
        let [a, b] = &mut self.slots;
        let (live, free) = if self.cur == 0 { (a, b) } else { (b, a) };
        let rng = &mut self.rng;
        let budget = free.prepare(&self.samplers[free.orb], self.m[free.orb], rng, budget);
        live.prepare(&self.samplers[live.orb], self.m[live.orb], rng, budget);
    }

    /// Advance by `dt > 0` s (finite; the scene's `update` filters every other value).
    pub(crate) fn step(&mut self, dt: f64) {
        debug_assert!(dt.is_finite() && dt > 0.0, "dt = {dt}");
        self.t += dt;
        self.sync();
        let beat = self.timeline.at(self.t);
        let since_rise = beat.local + self.phase.offset();
        let (a, b) = (GALLERY[self.at], GALLERY[self.next()]);
        let omega = beat_omega(a.n(), b.n());
        self.theta = if self.phase == Phase::Hold {
            (0.0, 0.0)
        } else {
            gauge(omega, since_rise)
        };
        self.mix = Mixture::new(
            chi(self.phase, beat.progress),
            self.m[self.at],
            self.m[self.next()],
            (self.theta.0 - self.theta.1).cos_det(),
        );
        if self.phase == Phase::Hold {
            self.prepare((PREP_PER_TICK * dt / SIM_DT).ceil() as usize);
        }
    }

    /// Walk the state machine to the Timeline's phase (one transition per call at
    /// dt ≤ 0.25, but correct for any jump).
    fn sync(&mut self) {
        let beat = self.timeline.at(self.t);
        let target = (beat.index / 4, Phase::from_index(beat.index));
        for _ in 0..4 * GALLERY.len() {
            if (self.at, self.phase) == target {
                break;
            }
            match self.phase {
                Phase::Hold => {
                    // Both slots must be complete before the second state appears.
                    self.prepare(usize::MAX);
                    self.phase = Phase::Rise;
                }
                Phase::Fall => {
                    // χ = π/2 of (a, next) is χ = 0 of (next, next + 1): same visible set.
                    self.cur = 1 - self.cur;
                    self.at = self.next();
                    let next = self.next();
                    self.slots[self.cur].repartner(next);
                    self.slots[1 - self.cur].reset(next, self.at);
                    self.phase = Phase::Hold;
                }
                p => self.phase = p.next(),
            }
        }
    }

    /// The glow's radial window: the held state's (ORB-A9; the glow shows only in holds).
    pub(crate) fn glow_fade(&self) -> Fade {
        self.r_glow[self.at]
    }

    /// How much of the glow shows (ORB-A9): 1 through a hold's middle, eased in over its
    /// first `GLOW_IN` s and out over its last `GLOW_OUT` s, 0 in a transition.
    pub(crate) fn glow_weight(&self) -> f64 {
        if self.phase != Phase::Hold {
            return 0.0;
        }
        let local = self.timeline.at(self.t).local;
        ease::smoothstep((local / GLOW_IN).clamp(0.0, 1.0))
            * ease::smoothstep(((HOLD - local) / GLOW_OUT).clamp(0.0, 1.0))
    }

    /// Calls `f(slot, i, position (a₀), phase)` for every point visible through `fade`: one
    /// radial window for both slots, so the drawn density is exactly `τ(r)·(M(χ)/2)|Ψ|²`.
    pub(crate) fn for_each_visible(
        &self,
        fade: Fade,
        mut f: impl FnMut(usize, usize, [f64; 3], f64),
    ) {
        let mx = self.mix;
        let own = [
            (self.cur, mx.p_a, (mx.c_a, mx.c_b), false),
            (1 - self.cur, mx.p_b, (mx.c_b, mx.c_a), true),
        ];
        for (s, p, c, is_b) in own {
            if p <= 0.0 {
                continue;
            }
            let slot = &self.slots[s];
            for i in 0..self.m[slot.orb].min(slot.len()) {
                let pos = slot.pos(i);
                let taper =
                    fade.taper((pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt());
                let psi = (f64::from(slot.psi[i]), f64::from(slot.other[i]));
                let (u, v) = (f64::from(slot.u[i]), f64::from(slot.v[i]));
                if visible(p * taper, c, mx.cos_wt, psi, u, v) {
                    let ab = if is_b { (psi.1, psi.0) } else { psi };
                    f(s, i, pos, phase_of((mx.c_a, mx.c_b), ab, self.theta));
                }
            }
        }
    }

    /// The display window of the states on screen, blended by c_b².
    fn fade(&self) -> Fade {
        let w = self.mix.c_b * self.mix.c_b;
        Fade::blend(self.r_fade[self.at], self.r_fade[self.next()], w)
    }

    /// Blend weight of the incoming state's framing, as a function of its weight `w = c_b²`.
    /// The larger state of the pair sets the scale while it holds at least half the weight:
    /// into a larger state the framing eases over the rise, `1 − (1 − 2w)₊²`, out of one
    /// over the fall, `(2w − 1)₊²`. Mixing a smaller state's scale in would draw the larger
    /// one bigger than in its own hold (× `(r₉₀,b/r₉₀,a)^{0.7(1−W)}`, 1.47 for 1s → 2p_z
    /// at W = ½), pushing its dust off the frame. `w` is flat in time at every phase
    /// boundary, so the framing moves smoothly.
    fn glide(&self) -> f64 {
        let w = self.mix.c_b * self.mix.c_b;
        if self.r90[self.next()] > self.r90[self.at] {
            1.0 - (1.0 - 2.0 * w).max(0.0).powi(2)
        } else {
            (2.0 * w - 1.0).max(0.0).powi(2)
        }
    }

    /// Blended `ln r₉₀` and `ln g` of the two states on screen.
    fn framing(&self) -> (f64, f64) {
        let (a, b, w) = (self.at, self.next(), self.glide());
        let widest = widest(&self.r90);
        let g = |k: usize| GLIDE * (self.r90[k] / widest).ln();
        (
            ease::lerp(self.r90[a].ln(), self.r90[b].ln(), w),
            ease::lerp(g(a), g(b), w),
        )
    }

    /// Blended camera pose of the two states on screen, by weight c_b² (so a pure hold is
    /// exactly its state's pose).
    fn pose(&self) -> Pose {
        let (a, b) = (GALLERY[self.at].pose(), GALLERY[self.next()].pose());
        a.lerp(b, self.mix.c_b * self.mix.c_b)
    }

    /// The camera for the blended pose at the Timeline clock (the rock is phased to it).
    fn camera(&self) -> Camera {
        self.pose().camera(self.t)
    }

    /// Whether a state with l ≥ 1 is on screen (only the 1s hold has none).
    fn has_node(&self) -> bool {
        let next = self.phase != Phase::Hold && GALLERY[self.next()].l() >= 1;
        GALLERY[self.at].l() >= 1 || next
    }
}

// ---------------------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------------------

/// The largest r₉₀ (4f's): the framing's reference.
fn widest(r90: &[f64; K]) -> f64 {
    r90.iter().copied().fold(0.0, f64::max)
}

/// Points per orbital so each shows ≈ 0.4 visible points per dot inside its r₉₀.
fn counts(size: Size, r90: &[f64; K]) -> [usize; K] {
    let (big, widest) = (draw::fit_radius(size, FIT), widest(r90));
    std::array::from_fn(|k| {
        let rho = big * (r90[k] / widest).powf(GLIDE);
        ((DENSITY * PI * (4.0 * rho).powi(2)).round() as usize).clamp(M_MIN, M_MAX)
    })
}

/// A point's camera depth (larger is nearer; world units, the blended r₉₀) and phase level
/// packed so that the larger key is the nearer point: depth in 1/4096 steps over
/// [−8, 8) in the high bits, the level in the low 8. Every drawn point sits within
/// ~1.5 r₉₀ of the nucleus, well inside the range, and is > 0.
fn front_key(depth: f64, level: u8) -> u32 {
    let q = ((depth + 8.0) * 4096.0).clamp(1.0, f64::from(u16::MAX)) as u32;
    q << 8 | u32::from(level)
}

/// Whether the label is drawn (and row h−1 is kept free of cloud).
fn label_shown(size: Size) -> bool {
    size.width() >= 14 && size.height() >= 8
}

/// The area the atom is framed in and centred on: every row above the label, so the cloud
/// has as much black above it as below.
fn stage(size: Size) -> Size {
    Size::new(size.width(), size.height() - u16::from(label_shown(size)))
}

#[derive(Debug)]
struct ElectronOrbitals {
    /// Built at the first resize (the seeded start needs `ctx.rng`).
    atom: Option<Atom>,
    size: Size,
    dots: Dots,
    /// Long exposure of the visible points at pixel resolution (a dot-resolution `Density`
    /// would take 8 floats a cell and push the scene past the TEST-R7 ceiling at
    /// 500×150), and its tone-mapped glow (ORB-A9).
    exposure: Glow,
    glow: Pixels,
    tone: Tone,
    /// Per cell, the nearest visible point as a [`front_key`] (0: none yet), so each cell
    /// wears the phase of the cloud's front surface (ORB-A8): the dots of a cell share one
    /// ink, and one picked in sample order mixed front and back into confetti.
    front: Grid<u32>,
}

impl Default for ElectronOrbitals {
    fn default() -> Self {
        Self {
            atom: None,
            size: Size::new(1, 1),
            dots: Dots::default(),
            exposure: Glow::default(),
            glow: Pixels::default(),
            tone: Tone::asinh(GLOW_KNEE, GLOW_WHITE),
            front: Grid::default(),
        }
    }
}

impl ElectronOrbitals {
    /// The nucleus cell, mid-[`stage`]. The projected origin sits on the dot at its middle
    /// (see [`rasterise`](Self::rasterise)), so a nodal plane through the nucleus empties
    /// one whole dot row instead of half of two.
    fn nucleus(&self) -> (i32, i32) {
        let stage = stage(self.size);
        (i32::from(stage.width()) / 2, i32::from(stage.height()) / 2)
    }

    /// On-screen r₉₀ radius (rows) of the widest state.
    fn radius(&self) -> f64 {
        draw::fit_radius(stage(self.size), FIT)
    }

    fn centre(&self) -> (f64, f64) {
        let (x, y) = self.nucleus();
        (f64::from(x) + 0.5, f64::from(y) + 0.5)
    }

    /// Lit dots of the nucleus cell.
    fn nucleus_dots(&self) -> usize {
        let (x, y) = self.nucleus();
        self.dots.cell_mask(x, y).map_or(0, u8::count_ones) as usize
    }

    /// Project every visible point into `Dots`, orbital z up: (x, y, z) ↦ world (x, z, −y).
    /// The far dust fades out through the blended `R_FADE` window, and row h−1 is skipped
    /// while it holds the label (the dust would otherwise speckle the text). Each cell takes
    /// the phase of its nearest point (`front`; `Dots` keeps the last ink set for
    /// `Phase`, so every later, farther point is plotted in the front ink). The same points
    /// expose the glow: the exposure keeps `keep` of its old value (0 starts afresh) and each
    /// point adds `1 − keep` times its `GLOW_FADE` taper to its pixel, so it settles at the
    /// expected (tapered) points per pixel per frame; the hold envelope then scales what
    /// shows (ORB-A9).
    fn rasterise(&mut self, cam: Camera, keep: f32) {
        self.dots.clear();
        self.front.fill(0);
        self.exposure.decay(keep);
        let Some(atom) = &self.atom else {
            return;
        };
        let (ln_r90, ln_g) = atom.framing();
        let (inv_r90, scale) = ((-ln_r90).exp(), self.radius() * ln_g.exp());
        // Dot rows round to the nearest integer, so a dot lands on row h−1 from here.
        let floor = if label_shown(self.size) {
            f64::from(Dots::CELL_H) * f64::from(self.size.height() - 1) - 0.5
        } else {
            f64::INFINITY
        };
        // The origin is snapped half a dot right and down of the nucleus cell's exact
        // middle, onto the centre of its dot (2x + 1, 4y + 2) (CORE-R64 coordinates), so a
        // nodal plane through the nucleus runs along one dot row and empties it rather
        // than half-emptying the two rows either side.
        let (cx, cy) = self.centre();
        let centre = (
            cx + 0.5 / f64::from(Dots::CELL_W),
            cy + 0.5 / f64::from(Dots::CELL_H),
        );
        let proj = cam.projector();
        let shown = atom.glow_weight();
        let glow_fade = atom.glow_fade();
        let (dots, exposure) = (&mut self.dots, &mut self.exposure);
        let front = &mut self.front;
        let weight = 1.0 - keep;
        atom.for_each_visible(atom.fade(), |_, _, [x, y, z], phase| {
            let world = [x * inv_r90, z * inv_r90, -y * inv_r90];
            let Some((sx, sy, depth)) = proj.project(world, centre, scale) else {
                return;
            };
            let (dx, dy) = Dots::from_cells(sx, sy);
            if dy >= floor {
                return;
            }
            let mut ink = ink::phase(phase);
            if let Some(f) = front.get_mut(sx.floor() as i32, sy.floor() as i32) {
                *f = (*f).max(front_key(depth, ink.level));
                ink.level = *f as u8;
            }
            dots.plot(dx, dy, ink);
            // The pixel holding the dot, plus an eighth to each of its four neighbours:
            // enough to smooth the grain, too little to fill a node.
            let r = (x * x + y * y + z * z).sqrt();
            exposure.splat(dx, dy, &SPLAT, weight * glow_fade.taper(r) as f32);
        });
        self.exposure
            .to_pixels(&mut self.glow, &self.tone, |level| {
                let u = (f64::from(level) / 255.0 - GLOW_FLOOR) / (1.0 - GLOW_FLOOR);
                let (lo, hi) = (ink::MIN_VISIBLE_LEVEL, f64::from(HAZE_MAX));
                let haze = shown * (lo + (hi - lo) * ease::smoothstep(u.min(1.0)));
                if u <= 0.0 || haze < lo {
                    Ink::BLANK
                } else {
                    Ink::new(Role::Ramp(GLOW_RAMP), ink::level(haze))
                }
            });
    }

    /// Bottom-right, clear of the title (GEN-R13): the state in a hold, `a → b  line`
    /// during a transition. Parts are dropped, never clipped: the slow-down first, then
    /// the line, then the arrow part, then `a` itself.
    fn label(&self, canvas: &mut Canvas) {
        let Some(atom) = &self.atom else {
            return;
        };
        if !label_shown(canvas.size()) {
            return;
        }
        let room = title::readout_room(canvas.size(), &Title::of(self));
        let ascii = canvas.mode() == GlyphMode::Ascii;
        let pick = |&(u, a): &(&'static str, &'static str)| if ascii { a } else { u };
        let fits = |parts: &[(&'static str, &'static str)]| {
            parts.iter().map(|p| pick(p).chars().count()).sum::<usize>() <= room
        };
        let (a, b) = (GALLERY[atom.at], GALLERY[atom.next()]);
        let (name, slowed) =
            line(a.n(), b.n()).map_or((("", ""), ("", "")), |l| (l.name, l.slowed));
        let parts = [
            a.name(),
            (" → ", " -> "),
            b.name(),
            ("  ", "  "),
            name,
            ("  ", "  "),
            slowed,
        ];
        let longest: &[usize] = if atom.phase == Phase::Hold {
            &[1]
        } else {
            &[7, 5, 3, 1]
        };
        let Some(&n) = longest.iter().find(|&&n| fits(&parts[..n])) else {
            return;
        };
        // Every label is under 64 bytes (the longest, a Pα one, is 49); writing into a
        // StackText never fails.
        let mut text = StackText::<64>::new();
        for p in &parts[..n] {
            let _ = text.write_str(pick(p));
        }
        let y = i32::from(canvas.size().height()) - 1;
        draw::label_right(canvas, y, text.as_str(), LABEL_INK);
    }
}

impl Generator for ElectronOrbitals {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        self.exposure.resize(size);
        self.glow.resize(size);
        self.front.resize(size, 0);
        match &mut self.atom {
            Some(atom) => atom.resize(stage(size)),
            None => {
                let first = ctx.rng.random_range(0..GALLERY.len());
                let rng = SplitMix64::from_rng(&mut ctx.rng);
                self.atom = Some(Atom::new(first, stage(size), rng));
            }
        }
        if let Some(cam) = self.atom.as_ref().map(Atom::camera) {
            self.rasterise(cam, 0.0);
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let Some(atom) = &mut self.atom else {
            return;
        };
        // The one dt guard (`Atom::step` only asserts it).
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        atom.step(dt);
        let cam = atom.camera();
        self.rasterise(cam, (-dt / GLOW_TAU).exp() as f32);
    }

    fn render(&self, canvas: &mut Canvas) {
        // Under the cloud, and over it where a node leaves its cell nearly empty (a
        // lobe dot or two from the tilt would otherwise hide it); 1s stays covered.
        // The glow first (Unicode only: ASCII has no half blocks), then the dots over it
        // with the glow carried on as their cells' background.
        self.glow.blit(canvas, |_, _, _, _| None);
        let (x, y) = self.nucleus();
        canvas.put(x, y, NUCLEUS, NUCLEUS_INK);
        self.dots.blit_over(canvas, &self.glow);
        let node = self.atom.as_ref().is_some_and(Atom::has_node);
        if node && self.nucleus_dots() <= NUCLEUS_MAX_DOTS {
            canvas.put(x, y, NUCLEUS, NUCLEUS_INK);
        }
        self.label(canvas);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use sciart_core::geom::v3::norm;

    type Point = ([f64; 3], f64, f64, f64, f64);

    const NUCLEUS_CHAR: char = '•';

    /// Radial integral `∫ f(r) dr` on `[0, 90]` by the trapezoid rule, h = 0.01 (ORB-A1).
    fn radial_integral(f: impl Fn(f64) -> f64) -> f64 {
        const H: f64 = 0.01;
        let mut s = 0.5 * (f(0.0) + f(90.0));
        for i in 1..9000 {
            s += f(f64::from(i) * H);
        }
        s * H
    }

    /// 48-point Gauss–Legendre nodes and weights on [−1, 1] (Newton on P₄₈).
    fn gauss_legendre() -> Vec<(f64, f64)> {
        const N: usize = 48;
        (0..N)
            .map(|i| {
                let mut x = (PI * (i as f64 + 0.75) / (N as f64 + 0.5)).cos_det();
                let mut dp = 1.0;
                for _ in 0..100 {
                    let (mut p0, mut p1) = (1.0, x);
                    for k in 2..=N {
                        let k = k as f64;
                        (p0, p1) = (p1, ((2.0 * k - 1.0) * x * p1 - (k - 1.0) * p0) / k);
                    }
                    dp = N as f64 * (x * p1 - p0) / (x * x - 1.0);
                    let dx = p1 / dp;
                    x -= dx;
                    if dx.abs() < 1e-16 {
                        break;
                    }
                }
                (x, 2.0 / ((1.0 - x * x) * dp * dp))
            })
            .collect()
    }

    /// Angular integral `∫ f(r̂) dΩ`: Gauss–Legendre in cos θ × 96 equal φ steps (ORB-A1).
    fn angular_integral(f: impl Fn([f64; 3]) -> f64) -> f64 {
        let mut s = 0.0;
        for (mu, w) in gauss_legendre() {
            let st = (1.0 - mu * mu).sqrt();
            for j in 0..96 {
                let phi = TAU * f64::from(j) / 96.0;
                s += w * (TAU / 96.0) * f([st * phi.cos_det(), st * phi.sin_det(), mu]);
            }
        }
        s
    }

    /// `⟨a|r|b⟩` (a₀): radial × angular, both separable.
    fn dipole(a: Orb, b: Orb) -> [f64; 3] {
        let rad =
            radial_integral(|r| radial(a.n(), a.l(), r) * radial(b.n(), b.l(), r) * r * r * r);
        std::array::from_fn(|i| rad * angular_integral(|d| solid(a, d) * solid(b, d) * d[i]))
    }

    #[test]
    fn orb_a1_orbitals_are_normalised_and_orthogonal() {
        for o in GALLERY {
            let (n, l) = (o.n(), o.l());
            let rad = radial_integral(|r| (radial(n, l, r) * r).powi(2));
            assert!((rad - 1.0).abs() < 1e-6, "{o:?} radial norm {rad}");
            let ang = angular_integral(|d| solid(o, d).powi(2));
            assert!((ang - 1.0).abs() < 1e-9, "{o:?} angular norm {ang}");
        }
        for (i, &a) in GALLERY.iter().enumerate() {
            for &b in &GALLERY[i + 1..] {
                let rad =
                    radial_integral(|r| radial(a.n(), a.l(), r) * radial(b.n(), b.l(), r) * r * r);
                let ang = angular_integral(|d| solid(a, d) * solid(b, d));
                assert!((rad * ang).abs() < 1e-9, "<{a:?}|{b:?}> = {}", rad * ang);
            }
        }
    }

    #[test]
    fn orb_a1_laguerre_recurrence_matches_closed_forms() {
        // The gallery only runs k = 0 (circular states), so the recurrence is checked here.
        for x in [0.0, 0.5, 1.7, 4.0, 11.0] {
            let l2 = x * x / 2.0 - 3.0 * x + 3.0;
            assert!((laguerre(2, 1.0, x) - l2).abs() < 1e-12, "L_2^(1)({x})");
            let l3 = (-x * x * x + 9.0 * x * x - 18.0 * x + 6.0) / 6.0;
            assert!((laguerre(3, 0.0, x) - l3).abs() < 1e-12, "L_3^(0)({x})");
        }
        // Non-circular states (radial nodes) stay normalised.
        for (n, l) in [(3, 0), (4, 1), (4, 0)] {
            let rad = radial_integral(|r| (radial(n, l, r) * r).powi(2));
            assert!((rad - 1.0).abs() < 1e-6, "({n}, {l}) radial norm {rad}");
        }
    }

    #[test]
    fn orb_a2_radial_moments_match_closed_forms_and_the_sampler() {
        for o in GALLERY {
            let (n, l) = (f64::from(o.n()), f64::from(o.l()));
            let moment =
                |k: i32| radial_integral(|r| (radial(o.n(), o.l(), r) * r).powi(2) * r.powi(k));
            let (r1, r2) = (moment(1), moment(2));
            let e1 = (3.0 * n * n - l * (l + 1.0)) / 2.0;
            let e2 = n * n * (5.0 * n * n + 1.0 - 3.0 * l * (l + 1.0)) / 2.0;
            assert!((r1 / e1 - 1.0).abs() < 1e-6, "{o:?} <r> {r1} vs {e1}");
            assert!((r2 / e2 - 1.0).abs() < 1e-6, "{o:?} <r²> {r2} vs {e2}");

            let sampler = Sampler::new(o);
            let mut rng = SplitMix64::new(7);
            let draws = 20_000;
            let pts: Vec<_> = (0..draws).map(|_| sampler.sample(&mut rng)).collect();
            let mean = pts.iter().map(|&p| norm(p)).sum::<f64>() / f64::from(draws);
            let se = (e2 - e1 * e1).sqrt() / f64::from(draws).sqrt();
            assert!(
                (mean - e1).abs() < 4.0 * se,
                "{o:?} sampled <r> {mean} vs {e1} ± {se}"
            );

            // One angular moment per state with a non-constant factor, so every Θ and Φ
            // table is checked against its closed form: cos²θ for m = 0 (Θ only);
            // x̂² = sin²θ cos²φ for 2p_x; cos²φ for 3d_xz (Φ² ∝ cos²φ); sin²2φ for 3d_xy
            // and 4f_xyz (Φ² ∝ sin²2φ, so ⟨sin²2φ⟩ = ∫sin⁴/∫sin² = 3/4).
            fn rho2(p: [f64; 3]) -> f64 {
                p[0] * p[0] + p[1] * p[1]
            }
            let (stat, want): (fn([f64; 3]) -> f64, f64) = match o {
                Orb::P2z => (|p| (p[2] / norm(p)).powi(2), 3.0 / 5.0),
                Orb::D3z2 => (|p| (p[2] / norm(p)).powi(2), 11.0 / 21.0),
                Orb::P2x => (|p| (p[0] / norm(p)).powi(2), 3.0 / 5.0),
                Orb::D3xz => (|p| p[0] * p[0] / rho2(p), 3.0 / 4.0),
                Orb::D3xy | Orb::F4xyz => (|p| (2.0 * p[0] * p[1] / rho2(p)).powi(2), 3.0 / 4.0),
                Orb::S1 | Orb::F4z3 => continue,
            };
            let c: Vec<_> = pts.iter().map(|&p| stat(p)).collect();
            let m = c.iter().sum::<f64>() / f64::from(draws);
            let sd = (c.iter().map(|v| (v - m).powi(2)).sum::<f64>() / f64::from(draws)).sqrt();
            let se = sd / f64::from(draws).sqrt();
            assert!(
                (m - want).abs() < 4.0 * se,
                "{o:?} angular moment {m} vs {want} ± {se}"
            );
        }
    }

    #[test]
    fn orb_a2_ninety_percent_radii_match_the_spec_table() {
        for (o, want) in [
            (Orb::S1, 2.66),
            (Orb::P2z, 7.99),
            (Orb::D3z2, 15.80),
            (Orb::F4z3, 25.99),
        ] {
            let r = radius_quantile(o.n(), o.l(), 0.9);
            assert!((r - want).abs() < 0.01, "{o:?} r90 = {r}");
        }
    }

    #[test]
    fn orb_a3_dipoles_and_line_wavelengths() {
        let d = dipole(Orb::S1, Orb::P2z);
        let exact = 2f64.powf(7.5) / 3f64.powi(5);
        assert!((d[2] - exact).abs() < 1e-4, "<1s|z|2p_z> = {}", d[2]);
        // The documented |d_ab| around the loop, the wrap included (all > 0.5 a₀).
        let table = [0.7449, 2.452, 5.188, 2.995, 3.867, 3.867, 2.123, 0.7449];
        for (i, (&a, want)) in GALLERY.iter().zip(table).enumerate() {
            let b = GALLERY[(i + 1) % GALLERY.len()];
            let d = norm(dipole(a, b));
            assert!(
                (d - want).abs() < 1e-3 && d > 0.5,
                "|<{a:?}|r|{b:?}>| = {d}, want {want}"
            );
        }
        for ((na, nb), want, text) in [
            ((1, 2), 121.57, "121.6"),
            ((2, 3), 656.47, "656.5"),
            ((3, 4), 1875.6, "1876"),
        ] {
            let nm = line_nm(na, nb);
            assert!((nm / want - 1.0).abs() < 1e-3, "{na}->{nb}: {nm} nm");
            let rounded = if nm > 1000.0 {
                format!("{nm:.0}")
            } else {
                format!("{nm:.1}")
            };
            assert_eq!(rounded, text);
            let (u, a) = line(nb, na).unwrap().name;
            assert!(u.contains(text) && a.contains(text) && a.is_ascii());
        }
        assert!(line(1, 3).is_none() && beat_hz(1, 3) == 0.0);
        // Every transition of the loop, the wrap included, names the line `line_nm` gives
        // (the first number in the label, to its printed precision).
        for (i, &a) in GALLERY.iter().enumerate() {
            let b = GALLERY[(i + 1) % GALLERY.len()];
            let (u, ascii) = line(a.n(), b.n())
                .unwrap_or_else(|| panic!("{a:?} -> {b:?}: no line"))
                .name;
            let nm = line_nm(a.n(), b.n());
            for label in [u, ascii] {
                let printed: f64 = label
                    .split(' ')
                    .find_map(|w| w.parse().ok())
                    .unwrap_or_else(|| panic!("{a:?} -> {b:?}: no wavelength in {label:?}"));
                assert!(
                    (printed - nm).abs() <= 0.5,
                    "{a:?} -> {b:?}: {label:?} vs {nm}"
                );
            }
        }
    }

    #[test]
    fn orb_a3_beats_are_bohr_frequencies_slowed_by_the_stated_factor() {
        // The label states the slow-down exactly ("slowed 4×10¹⁵" / "slowed 4e15"), and the
        // beat is the true Bohr frequency ΔE/h divided by it: Lyα is 2.466 PHz.
        let lya = gap(1, 2) / H_EV_S;
        assert!((lya / 2.466e15 - 1.0).abs() < 1e-3, "Lyα {lya} Hz");
        let sup = |d: char| {
            "⁰¹²³⁴⁵⁶⁷⁸⁹"
                .chars()
                .nth(d.to_digit(10).unwrap() as usize)
                .unwrap()
        };
        let mut last = f64::INFINITY;
        for ((na, nb), want) in [((1, 2), 0.6165), ((2, 3), 0.4567), ((3, 4), 0.3197)] {
            let l = line(na, nb).unwrap();
            let (u, a) = l.slowed;
            let factor: f64 = a.strip_prefix("slowed ").unwrap().parse().unwrap();
            assert_eq!(factor, l.slowdown, "{a}");
            let (mantissa, exp) = a.strip_prefix("slowed ").unwrap().split_once('e').unwrap();
            let exp: String = exp.chars().map(sup).collect();
            let text = match mantissa {
                "1" => format!("slowed 10{exp}"),
                m => format!("slowed {m}×10{exp}"),
            };
            assert_eq!(u, text);
            let f = beat_hz(na, nb);
            assert!((f - gap(na, nb) / H_EV_S / factor).abs() < 1e-12);
            assert!((f - want).abs() < 1e-3, "{na}->{nb}: {f} Hz");
            // Calm (≤ 0.7 Hz), ordered as the physics is, and ≥ 0.9 cycles per 3 s beat.
            assert!(
                f <= 0.7 && f < last && f * BEAT >= 0.9,
                "{na}->{nb}: {f} Hz"
            );
            last = f;
        }
    }

    /// `m` iid points of `orb` with ψ of itself and `partner`, and fixed uniforms.
    fn synthetic(orb: Orb, partner: Orb, m: usize, seed: u64) -> Vec<Point> {
        let s = Sampler::new(orb);
        let mut rng = SplitMix64::new(seed);
        (0..m)
            .map(|_| {
                let p = s.sample(&mut rng);
                (
                    p,
                    psi(orb, p),
                    psi(partner, p),
                    rng.next_f64(),
                    rng.next_f64(),
                )
            })
            .collect()
    }

    /// Visible points of a two-slot superposition at mixing angle `chi`.
    fn shown(a: &[Point], b: &[Point], chi: f64, cos_wt: f64) -> Vec<[f64; 3]> {
        let mx = Mixture::new(chi, a.len(), b.len(), cos_wt);
        let pick = |set: &[Point], p: f64, c: (f64, f64)| {
            set.iter()
                .filter(|&&(_, own, other, u, v)| visible(p, c, cos_wt, (own, other), u, v))
                .map(|x| x.0)
                .collect::<Vec<_>>()
        };
        let mut out = pick(a, mx.p_a, (mx.c_a, mx.c_b));
        out.extend(pick(b, mx.p_b, (mx.c_b, mx.c_a)));
        out
    }

    #[test]
    fn orb_a4_superposition_density_is_exact() {
        let a = synthetic(Orb::S1, Orb::P2z, 40_000, 1);
        let b = synthetic(Orb::P2z, Orb::S1, 40_000, 2);
        for cos_wt in [1.0, 0.0, -1.0] {
            let pts = shown(&a, &b, FRAC_PI_4, cos_wt);
            let n = pts.len() as f64;
            assert!(
                (n - 20_000.0).abs() < 4.0 * 20_000f64.sqrt(),
                "count {n} at cos wt = {cos_wt}"
            );
            for (axis, want) in [(0, 0.0), (1, 0.0), (2, 0.7449 * cos_wt)] {
                let mean = pts.iter().map(|p| p[axis]).sum::<f64>() / n;
                let sd = (pts.iter().map(|p| (p[axis] - mean).powi(2)).sum::<f64>() / n).sqrt();
                assert!(
                    (mean - want).abs() < 4.0 * sd / n.sqrt(),
                    "axis {axis}: {mean} vs {want} at cos wt = {cos_wt}"
                );
            }
        }
        let n = shown(&a[..10_000], &b, FRAC_PI_4, 0.3).len() as f64;
        assert!(
            (n - 8_000.0).abs() < 4.0 * 8_000f64.sqrt(),
            "count {n}, want M(chi)/2 = 8000"
        );
    }

    #[test]
    fn orb_a4_the_display_fade_is_exact() {
        // Both slots share one window, blended by c_b² = ½ here, so the drawn density is
        // exactly τ(r)·(M/2)|Ψ|². Against quadrature: at χ = π/4 and cos ωt = 1 with
        // M_a = M_b = 40 000, |Ψ|² = ½(ψ_a + ψ_b)², so the expected count is
        // 10 000·(∫τR_a²r² + ∫τR_b²r²) (the cross term has no s–p overlap), and the
        // z centroid is (∫τR_aR_b r³ dr)(1/√3) over half that bracket.
        let (a, b) = (Orb::S1, Orb::P2z);
        let fade = Fade::blend(Fade::window(a, R_FADE), Fade::window(b, R_FADE), 0.5);
        assert!(
            fade.inner > Fade::window(a, R_FADE).inner
                && fade.outer < Fade::window(b, R_FADE).outer
        );
        let tapered = |f: &dyn Fn(f64) -> f64| radial_integral(|r| fade.taper(r) * f(r));
        let norm_a = tapered(&|r| (radial(1, 0, r) * r).powi(2));
        let norm_b = tapered(&|r| (radial(2, 1, r) * r).powi(2));
        let cross = tapered(&|r| radial(1, 0, r) * radial(2, 1, r) * r.powi(3));
        let want_n = 10_000.0 * (norm_a + norm_b);
        let want_z = cross / 3f64.sqrt() / (0.5 * (norm_a + norm_b));
        assert!(want_n < 19_000.0, "the fade must bite: {want_n}");

        let (pa, pb) = (synthetic(a, b, 40_000, 1), synthetic(b, a, 40_000, 2));
        let mx = Mixture::new(FRAC_PI_4, pa.len(), pb.len(), 1.0);
        let mut pts = Vec::new();
        for (set, p, c) in [
            (&pa, mx.p_a, (mx.c_a, mx.c_b)),
            (&pb, mx.p_b, (mx.c_b, mx.c_a)),
        ] {
            for &(x, own, other, u, v) in set {
                if visible(p * fade.taper(norm(x)), c, 1.0, (own, other), u, v) {
                    pts.push(x);
                }
            }
        }
        let n = pts.len() as f64;
        assert!(
            (n - want_n).abs() < 4.0 * want_n.sqrt(),
            "count {n} vs {want_n:.0}"
        );
        let mean = pts.iter().map(|p| p[2]).sum::<f64>() / n;
        let sd = (pts.iter().map(|p| (p[2] - mean).powi(2)).sum::<f64>() / n).sqrt();
        assert!(
            (mean - want_z).abs() < 4.0 * sd / n.sqrt(),
            "z centroid {mean} vs {want_z}"
        );
    }

    #[test]
    fn orb_a4_live_slots_carry_the_partner_after_a_slot_switch() {
        // 1s → 2p_z runs, then the slot switch re-partners the 2p_z slot with 3d_z² and
        // resamples the other: at the 2p_z → 3d_z² beat the visible centroid must be
        // sin 2χ·⟨2p_z|z|3d_z²⟩·cos ωt, which a stale partner ψ (1s) would not give.
        let mut atom = atom_at(0, 400, 200);
        while !(atom.at == 1 && atom.phase == Phase::Beat) {
            atom.step(SIM_DT);
        }
        let d = dipole(Orb::P2z, Orb::D3z2)[2];
        assert!((d.abs() - 2.452).abs() < 1e-3, "<2p_z|z|3d_z2> = {d}");
        for cos_wt in [1.0, -1.0] {
            atom.mix = Mixture::new(FRAC_PI_4, atom.m[1], atom.m[2], cos_wt);
            let mut pts = Vec::new();
            atom.for_each_visible(Fade::NONE, |_, _, p, _| pts.push(p));
            // Both slots hold M_MAX points here, so M(χ)/2 = 8192 are visible on average.
            let n = pts.len() as f64;
            assert_eq!((atom.m[1], atom.m[2]), (M_MAX, M_MAX));
            assert!(
                (n - 8192.0).abs() < 4.0 * 8192f64.sqrt(),
                "{n} visible points"
            );
            for (axis, want) in [(0, 0.0), (1, 0.0), (2, d * cos_wt)] {
                let mean = pts.iter().map(|p| p[axis]).sum::<f64>() / n;
                let sd = (pts.iter().map(|p| (p[axis] - mean).powi(2)).sum::<f64>() / n).sqrt();
                assert!(
                    (mean - want).abs() < 4.0 * sd / n.sqrt(),
                    "axis {axis}: {mean} vs {want} at cos wt = {cos_wt}"
                );
            }
        }
    }

    fn atom_at(first: usize, w: u16, h: u16) -> Atom {
        Atom::new(first, Size::new(w, h), SplitMix64::new(11))
    }

    /// The points on screen: visible through the atom's own display window.
    fn visible_set(atom: &Atom) -> HashSet<(usize, usize)> {
        let mut set = HashSet::new();
        atom.for_each_visible(atom.fade(), |s, i, _, _| {
            set.insert((s, i));
        });
        set
    }

    #[test]
    fn orb_a5_phase_boundaries_show_the_same_points() {
        let mut atom = atom_at(0, 200, 50);
        let hold = visible_set(&atom);
        assert!(!hold.is_empty());
        // Hold → rise: χ = 0 on both sides, with the next slot complete.
        let (ma, mb) = (atom.m[0], atom.m[1]);
        atom.prepare(usize::MAX);
        atom.mix = Mixture::new(chi(Phase::Rise, 0.0), ma, mb, 1.0);
        assert_eq!(visible_set(&atom), hold, "hold -> rise");
        // Fall → hold across the slot switch: χ = π/2 of (1s, 2p_z) vs the 2p_z hold.
        while atom.phase != Phase::Fall {
            atom.step(SIM_DT);
        }
        atom.mix = Mixture::new(chi(Phase::Fall, 1.0), ma, mb, -0.4);
        let end_of_fall = visible_set(&atom);
        while atom.phase != Phase::Hold {
            atom.step(SIM_DT);
        }
        assert_eq!(atom.at, 1);
        assert_eq!(visible_set(&atom), end_of_fall, "fall -> hold");
    }

    #[test]
    fn orb_a5_no_step_pops_more_than_six_percent() {
        // Over 1s → 2p_z, the worst pair (Lyα beats fastest, 0.62 Hz). The sampler toggles
        // ≤ 6 % per step (measured 4.7, against the 2.7 % ω bound plus ≤ 2 % of χ drift).
        // The display fade adds the drift of the shared window, which widens 3× over the
        // transition: ≤ 7 % (measured 5.1).
        let set = |atom: &Atom, fade: Fade| {
            let mut set = HashSet::new();
            atom.for_each_visible(fade, |s, i, _, _| {
                set.insert((s, i));
            });
            set
        };
        let mut atom = atom_at(0, 200, 50);
        let mut prev = [set(&atom, Fade::NONE), set(&atom, atom.fade())];
        let mut worst = [0.0f64; 2];
        while atom.at == 0 {
            atom.step(SIM_DT);
            let now = [set(&atom, Fade::NONE), set(&atom, atom.fade())];
            for k in 0..2 {
                let toggled = now[k].symmetric_difference(&prev[k]).count() as f64;
                worst[k] = worst[k].max(toggled / prev[k].len().max(1) as f64);
            }
            prev = now;
        }
        for (worst, bound, what) in [(worst[0], 0.06, "sampler"), (worst[1], 0.07, "display")] {
            assert!(
                worst <= bound,
                "{what}: a step toggled {:.1} % of the visible points",
                100.0 * worst
            );
        }
    }

    #[test]
    fn orb_a6_timeline_visits_the_loop_in_order() {
        let mut atom = atom_at(0, 40, 12);
        assert_eq!(atom.timeline.total(), 104.0);
        let mut seen = vec![(atom.at, atom.phase, 0.0)];
        for s in 1..=(104 * 60 + 1) {
            atom.step(SIM_DT);
            let last = seen[seen.len() - 1];
            if (atom.at, atom.phase) != (last.0, last.1) {
                seen.push((atom.at, atom.phase, f64::from(s) * SIM_DT));
            }
        }
        let expected: Vec<_> = (0..K)
            .flat_map(|k| Phase::ALL.map(|p| (k, p)))
            .chain([(0, Phase::Hold)])
            .collect();
        assert_eq!(
            seen.iter().map(|&(a, p, _)| (a, p)).collect::<Vec<_>>(),
            expected
        );
        let starts = [0.0, HOLD, HOLD + RISE, HOLD + RISE + BEAT];
        for (i, &(_, p, t)) in seen.iter().enumerate().skip(1) {
            let want = 13.0 * (i / 4) as f64 + starts[p as usize];
            assert!(
                (t - want).abs() <= SIM_DT + 1e-9,
                "phase {i} at {t}, want {want}"
            );
        }
    }

    fn scene(first: usize, w: u16, h: u16) -> ElectronOrbitals {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(42, size);
        let mut s = ElectronOrbitals::default();
        s.resize(size, &mut ctx);
        let rng = SplitMix64::from_rng(&mut ctx.rng);
        s.atom = Some(Atom::new(first, stage(size), rng));
        s.rasterise(
            Camera {
                yaw: 0.3,
                pitch: PITCH_LOW,
                distance: DISTANCE,
            },
            0.0,
        );
        s
    }

    fn bottom_row(s: &ElectronOrbitals, mode: GlyphMode) -> String {
        let mut c = Canvas::new(s.size, mode);
        s.render(&mut c);
        c.to_lines().pop().unwrap()
    }

    fn into_transition(s: &mut ElectronOrbitals) {
        let atom = s.atom.as_mut().unwrap();
        while atom.phase == Phase::Hold {
            atom.step(SIM_DT);
        }
    }

    #[test]
    fn orb_a6_gen_r13_label_is_complete_or_falls_back() {
        // Right-aligned and clear of the title (GEN-R13): beside the full "Hydrogen atom ·
        // electron clouds" (31 cells, from w = 55) the label has w − 37 chars; beside
        // "Hydrogen atom" alone, w − 19. Four blanks always separate the two texts.
        let label = |s: &ElectronOrbitals, mode| {
            let row = bottom_row(s, mode);
            let lead = row.chars().count() - row.trim_start().chars().count() - 1;
            let span = title::span(s.size, &Title::new(META.title, None));
            let free = span.map_or(0, |(_, cols)| cols.end + 2);
            assert!(i32::try_from(lead).unwrap() >= free, "{row:?}");
            assert!(row.ends_with(' '), "{row:?}");
            row.trim().to_owned()
        };
        let mut s = scene(1, 80, 24);
        assert_eq!(label(&s, GlyphMode::Unicode), "2p_z");
        into_transition(&mut s);
        assert_eq!(
            label(&s, GlyphMode::Ascii),
            "2p_z -> 3d_z2  H-a 656.5 nm  slowed 1e15"
        );
        assert_eq!(
            label(&s, GlyphMode::Unicode),
            "2p_z → 3d_z²  Hα 656.5 nm  slowed 10¹⁵"
        );

        // The slow-down goes first: the 40-char label needs w − 37 ≥ 40 and, as a readout
        // never takes over half the row (CORE-R76), w/2 ≥ 40.
        let mut s = scene(1, 79, 12);
        into_transition(&mut s);
        assert_eq!(label(&s, GlyphMode::Ascii), "2p_z -> 3d_z2  H-a 656.5 nm");
        let mut s = scene(1, 80, 12);
        into_transition(&mut s);
        assert!(label(&s, GlyphMode::Ascii).ends_with("slowed 1e15"));

        // "2p_z -> 3d_z2  H-a 656.5 nm" is 27 chars: beside the phenomenon alone (below
        // w = 55) the line part needs w − 19 ≥ 27 and w/2 ≥ 27.
        let mut s = scene(1, 53, 12);
        into_transition(&mut s);
        assert_eq!(label(&s, GlyphMode::Ascii), "2p_z -> 3d_z2");
        let mut s = scene(1, 54, 12);
        into_transition(&mut s);
        assert!(label(&s, GlyphMode::Ascii).ends_with("H-a 656.5 nm"));

        for (w, h) in [(13, 24), (80, 7)] {
            let s = scene(1, w, h);
            assert!(
                !bottom_row(&s, GlyphMode::Ascii).contains("2p_z"),
                "{w}x{h}"
            );
        }

        // "3d_xz -> 4f_xyz" (15 chars, 14 in Unicode) beside "Hydrogen atom" (from 25 wide,
        // room w − 19) needs w ≥ 34 (33): below that the arrow part falls back to the
        // current state's name instead of being clipped.
        for w in [14, 25, 32] {
            let mut s = scene(4, w, 12);
            into_transition(&mut s);
            for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
                assert_eq!(label(&s, mode), "3d_xz", "w = {w}, {mode:?}");
            }
        }
        let mut s = scene(4, 33, 12);
        into_transition(&mut s);
        assert_eq!(label(&s, GlyphMode::Unicode), "3d_xz → 4f_xyz");
        assert_eq!(label(&s, GlyphMode::Ascii), "3d_xz");
    }

    #[test]
    fn orb_a6_cloud_leaves_the_label_row_free() {
        for (w, h) in [(80, 24), (40, 12), (14, 8)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(42, size);
            let mut s = ElectronOrbitals::default();
            s.resize(size, &mut ctx);
            for step in 0..4 * 104 {
                s.update(0.25, &mut ctx);
                let row = &cell_dots(&s)[usize::from(h) - 1];
                assert!(
                    row.iter().all(|&c| c == 0),
                    "{w}x{h} step {step}: dots on the label row {row:?}"
                );
            }
        }
    }

    /// Redraw `s` (in the hold of state `k`) with the scene's own camera `t` s into the
    /// state's visit on the Timeline clock (t = MID_HOLD is mid-hold). A hold is stationary,
    /// so this sweeps its camera over any part of the rock.
    fn at_clock(s: &mut ElectronOrbitals, k: usize, t: f64) {
        s.atom.as_mut().unwrap().t = ROCK_PERIOD * k as f64 + t;
        let cam = s.atom.as_ref().unwrap().camera();
        s.rasterise(cam, 0.0);
    }

    /// `scene` redrawn with the scene's own camera `t` s into the state's visit.
    fn posed(first: usize, w: u16, h: u16, t: f64) -> ElectronOrbitals {
        let mut s = scene(first, w, h);
        at_clock(&mut s, first, t);
        s
    }

    /// Clocks every 0.25 s over one whole rock cycle.
    fn cycle() -> impl Iterator<Item = f64> {
        (0..52).map(|i| f64::from(i) * 0.25)
    }

    /// First and last row holding ≥ 3 lit cells.
    fn row_span(cells: &[Vec<u32>]) -> usize {
        let rows: Vec<_> = (0..cells.len())
            .filter(|&y| cells[y].iter().filter(|&&c| c > 0).count() >= 3)
            .collect();
        rows.last().map_or(0, |&b| b - rows[0] + 1)
    }

    /// Lit-dot counts per cell of the scene's raster.
    fn cell_dots(s: &ElectronOrbitals) -> Vec<Vec<u32>> {
        let (w, h) = (i32::from(s.size.width()), i32::from(s.size.height()));
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| {
                        (0..8)
                            .filter(|d| s.dots.is_lit(2 * x + d % 2, 4 * y + d / 2))
                            .count() as u32
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn orb_a7_composition_at_80x24() {
        let cells = cell_dots(&scene(0, 80, 24));
        let lit: Vec<_> = cells.iter().flatten().filter(|&&c| c > 0).collect();
        let full = lit.iter().filter(|&&&c| c == 8).count() as f64;
        assert!(
            full <= 0.05 * lit.len() as f64,
            "1s: {full} of {} lit cells are full",
            lit.len()
        );

        let cells = cell_dots(&scene(3, 80, 24));
        let rows = cells
            .iter()
            .filter(|r| r.iter().filter(|&&c| c > 0).count() >= 3)
            .count();
        assert!(rows >= 16, "4f_z3 fills {rows} rows");

        let s = scene(1, 80, 24);
        let cells = cell_dots(&s);
        let per_row: Vec<_> = cells
            .iter()
            .map(|r| r.iter().filter(|&&c| c > 0).count())
            .collect();
        let fullest = *per_row.iter().max().unwrap() as f64;
        // The origin sits mid-row on the nucleus row, so that row holds the nodal plane.
        let gap = per_row[s.nucleus().1 as usize] as f64;
        assert!(
            gap <= 0.6 * fullest,
            "2p_z nodal rows {gap} vs fullest {fullest}: {per_row:?}"
        );
    }

    /// Sign runs of the Phase cells (±π/2 in a hold) met walking a ring of radius 0.5ρ_k about the nucleus
    /// (ρ_k: state k's on-screen r₉₀); a clover gives 4.
    fn ring_sign_runs(s: &ElectronOrbitals, k: usize) -> usize {
        let r90 = &s.atom.as_ref().unwrap().r90;
        let rho = s.radius() * (r90[k] / widest(r90)).powf(GLIDE);
        let mut c = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut c);
        let (cx, cy) = s.centre();
        let mut signs: Vec<bool> = (0..96)
            .filter_map(|i| {
                let a = TAU * f64::from(i) / 96.0;
                let x = cx + 0.5 * rho * sciart_core::geom::ASPECT * a.cos_det();
                let y = cy - 0.5 * rho * a.sin_det();
                let cell = c.get(x.floor() as i32, y.floor() as i32)?;
                (cell.ink.role == Role::Phase).then_some(cell.ink.level == POS)
            })
            .collect();
        signs.dedup();
        if signs.len() > 1 && signs[0] == signs[signs.len() - 1] {
            signs.pop();
        }
        signs.len()
    }

    #[test]
    fn orb_a7_3d_xy_and_3d_xz_read_as_clovers() {
        let (d3xz, d3xy, f4z3) = (4, 6, 3);
        assert_eq!(
            [GALLERY[d3xz], GALLERY[d3xy], GALLERY[f4z3]],
            [Orb::D3xz, Orb::D3xy, Orb::F4z3]
        );
        // A hold is stationary, so sweep camera clocks across the whole 13 s rock.
        for t in (0..13).map(f64::from) {
            let s = posed(d3xy, 80, 24, t);
            let span = row_span(&cell_dots(&s));
            let reference = row_span(&cell_dots(&posed(f4z3, 80, 24, t)));
            assert!(
                span as f64 >= 0.6 * reference as f64,
                "t = {t}: 3d_xy spans {span} rows, 4f_z3 {reference}"
            );
            for k in [d3xy, d3xz] {
                let runs = ring_sign_runs(&posed(k, 80, 24, t), k);
                assert!(
                    runs >= 4,
                    "t = {t}: {:?} ring has {runs} sign runs",
                    GALLERY[k]
                );
            }
        }
    }

    #[test]
    fn orb_a7_2p_x_node_stays_open() {
        // The hold is stationary, so sweep the whole camera cycle, not only the hold's part
        // of it (measured ≤ 0.26). The gap is ~3 columns wide at 80x24, so the nucleus
        // column is compared with the fullest column within ±3 (the lobes), not with its
        // equally empty neighbours. At mid-hold the rock passes through broadside.
        let p2x = 7;
        assert_eq!(GALLERY[p2x], Orb::P2x);
        let mut s = scene(p2x, 80, 24);
        let cx = s.nucleus().0;
        at_clock(&mut s, p2x, MID_HOLD);
        assert!(
            s.atom.as_ref().unwrap().camera().yaw.abs() < 1e-12,
            "2p_x is broadside at mid-hold"
        );
        for t in cycle() {
            at_clock(&mut s, p2x, t);
            let cells = cell_dots(&s);
            let column = |x: i32| -> u32 { cells.iter().map(|r| r[x as usize]).sum() };
            let node = column(cx);
            let lobe = (cx - 3..=cx + 3).map(column).max().unwrap();
            assert!(
                f64::from(node) <= 0.4 * f64::from(lobe),
                "t = {t}: node column {node} vs lobe column {lobe}"
            );
        }
    }

    #[test]
    fn orb_a7_nucleus_glints_through_the_2p_z_node() {
        // Over the whole 13 s camera cycle (measured 0.83 of frames).
        let mut s = scene(1, 80, 24);
        let (x, y) = s.nucleus();
        let frames = 13 * 60;
        let mut shown = 0;
        for i in 0..frames {
            at_clock(&mut s, 1, f64::from(i) * SIM_DT);
            let mut c = Canvas::new(s.size, GlyphMode::Unicode);
            s.render(&mut c);
            if c.get(x, y).is_some_and(|cell| cell.ch == NUCLEUS_CHAR) {
                shown += 1;
            }
        }
        assert!(
            f64::from(shown) >= 0.8 * f64::from(frames),
            "nucleus shown in {shown} of {frames} frames"
        );
    }

    #[test]
    fn orb_a7_4f_xyz_keeps_its_nodal_plane_open() {
        // The nucleus row (origin at the cell centre) holds ≤ 50 % of the lit cells of the
        // fullest row at the hold's own camera, over the whole cycle at 0.25 s steps
        // (measured 0.24–0.45 over the 13 s cycle).
        let f4xyz = 5;
        assert_eq!(GALLERY[f4xyz], Orb::F4xyz);
        let mut s = scene(f4xyz, 80, 24);
        let ny = s.nucleus().1 as usize;
        for t in cycle() {
            at_clock(&mut s, f4xyz, t);
            let per_row: Vec<_> = cell_dots(&s)
                .iter()
                .map(|r| r.iter().filter(|&&c| c > 0).count())
                .collect();
            let fullest = *per_row.iter().max().unwrap() as f64;
            assert!(
                per_row[ny] as f64 <= 0.5 * fullest,
                "t = {t}: nucleus row {} vs fullest {fullest}: {per_row:?}",
                per_row[ny]
            );
        }
    }

    /// Lit dots of the nucleus column and of the fullest column within ±12 (the lobes).
    fn node_and_lobe_columns(s: &ElectronOrbitals) -> (u32, u32) {
        let cx = s.nucleus().0;
        let cells = cell_dots(s);
        let column = |x: i32| -> u32 { cells.iter().map(|r| r[x as usize]).sum() };
        (column(cx), (cx - 12..=cx + 12).map(column).max().unwrap())
    }

    #[test]
    fn orb_a7_4f_xyz_shows_four_blocks_at_mid_hold() {
        // The rock runs on the Timeline clock and peaks at mid-hold, where 4f_xyz is turned
        // to yaw π/4 + 0.8 ≈ π/2: its vertical nodal planes are edge-on and the lobes stand
        // as four `+#` blocks split by a cross, so the nucleus column holds ≤ 60 % of the
        // fullest lobe column's dots (at its 45° pose, two slabs, it is the fullest).
        let f4xyz = 5;
        let mut s = scene(f4xyz, 80, 24);
        at_clock(&mut s, f4xyz, MID_HOLD);
        let yaw = s.atom.as_ref().unwrap().camera().yaw;
        assert!((yaw - FRAC_PI_2).abs() < 0.02, "mid-hold yaw {yaw}");
        let (node, lobe) = node_and_lobe_columns(&s);
        assert!(
            f64::from(node) <= 0.6 * f64::from(lobe),
            "mid-hold: node column {node} vs lobe column {lobe}"
        );
        // Every visit sees the same part of the rock: the loop is a whole number of cycles.
        let total = s.atom.as_ref().unwrap().timeline.total();
        assert_eq!(
            total % ROCK_PERIOD,
            0.0,
            "{total} s loop vs {ROCK_PERIOD} s rock"
        );
    }

    #[test]
    fn orb_a7_4f_xyz_lobes_separate_for_part_of_each_rock() {
        // Near the axis-aligned ends of its rock the vertical nodal planes are edge-on, and
        // the centre band (nucleus column ± 1) empties to ≤ 15 % of the fullest lobe column:
        // four `+#` blocks split by a cross instead of two slabs. Measured over 0.41 of the
        // cycle at rock 0.8 (never at 0.6).
        let f4xyz = 5;
        let mut s = scene(f4xyz, 80, 24);
        let cx = s.nucleus().0;
        let mut split = 0;
        for t in cycle() {
            at_clock(&mut s, f4xyz, t);
            let cells = cell_dots(&s);
            let column = |x: i32| -> u32 { cells.iter().map(|r| r[x as usize]).sum() };
            let lobe = (cx - 12..=cx + 12).map(column).max().unwrap();
            let band = (cx - 1..=cx + 1).map(column).min().unwrap();
            split += usize::from(f64::from(band) <= 0.15 * f64::from(lobe));
        }
        assert!(split >= 16, "centre band open in {split} of 52 clocks");
    }

    #[test]
    fn orb_a7_the_camera_never_stalls() {
        // The rock and the pitch wobble share one period in quadrature, so every pose
        // keeps turning at ≥ PITCH_WOBBLE·2π/13 s ≈ 2.8°/s (a separate 17 s wobble let both
        // turn back together, slowing to ≈ 0.2°/s near t = 4 s).
        let (dt, floor) = (0.01, 2f64.to_radians());
        for orb in GALLERY {
            let pose = orb.pose();
            for i in 0..1300 {
                let t = f64::from(i) * dt;
                let (a, b) = (pose.camera(t), pose.camera(t + dt));
                let speed = (b.yaw - a.yaw).hypot(b.pitch - a.pitch) / dt;
                assert!(speed >= floor, "{orb:?} t = {t}: {speed} rad/s");
            }
        }
    }

    /// Lit cells farther than `1.25 ρ_k` (aspect-corrected) from the nucleus.
    fn far_cells(s: &ElectronOrbitals, k: usize) -> usize {
        let r90 = &s.atom.as_ref().unwrap().r90;
        let rho = s.radius() * (r90[k] / widest(r90)).powf(GLIDE);
        let (cx, cy) = s.centre();
        let mut far = 0;
        for (y, row) in cell_dots(s).iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                let dx = (x as f64 + 0.5 - cx) / sciart_core::geom::ASPECT;
                let dy = y as f64 + 0.5 - cy;
                far += usize::from(c > 0 && dx.hypot(dy) > 1.25 * rho);
            }
        }
        far
    }

    #[test]
    fn orb_a7_far_dust_fades_out() {
        // The fade ends at r₉₅, so no hold scatters specks beyond 1.25 ρ_k at 80×24
        // (measured 0 over 16 camera clocks for every state; a taper out to r₉₉ leaves up
        // to 4 per frame).
        for (k, orb) in GALLERY.iter().enumerate() {
            let far: usize = (0..16)
                .map(|t| far_cells(&posed(k, 80, 24, f64::from(t)), k))
                .sum();
            let mean = far as f64 / 16.0;
            assert!(mean <= 1.0, "{orb:?}: {mean} far cells per frame");
        }
    }

    /// Phase levels of the two signs of a real state: +π/2 and −π/2.
    const POS: u8 = 64;
    const NEG: u8 = 192;

    #[test]
    fn orb_a8_phase_is_fixed_in_holds_and_winds_at_the_beat_in_superpositions() {
        use std::f64::consts::FRAC_1_SQRT_2 as H;
        let turn = |p: f64| p.rem_euclid(TAU);
        // A hold (θ = 0) has one phase per sign: +π/2 where ψ > 0, −π/2 where ψ < 0; so
        // does the incoming state at the end of a transition, where θ_b is 0 again.
        let omega = 2.3;
        let end = gauge(omega, RISE + BEAT + FALL);
        for (c, psi, theta) in [
            ((1.0, 0.0), (0.7, -3.0), (0.0, 0.0)),
            ((0.0, 1.0), (5.0, 0.4), (0.0, 0.0)),
            ((0.0, 1.0), (5.0, 0.4), end),
            ((1.0, 0.0), (0.7, -3.0), gauge(omega, 0.0)),
        ] {
            let p = turn(phase_of(c, psi, theta));
            assert!((p - FRAC_PI_2).abs() < 1e-9, "{c:?} {psi:?} {theta:?}: {p}");
            let n = turn(phase_of(c, (-psi.0, -psi.1), theta));
            assert!((n - 3.0 * FRAC_PI_2).abs() < 1e-9, "{c:?} {theta:?}: {n}");
        }
        assert_eq!(ink::phase(FRAC_PI_2).level, POS);
        assert_eq!(ink::phase(-FRAC_PI_2).level, NEG);
        // Only the relative phase is physical: at every time of a transition the terms
        // differ by exactly ωt (the phase in |Ψ|² = q + 2c_a c_b ψ_a ψ_b cos ωt), and where
        // ψ_a = ψ_b at χ = π/4 the phase is the mean of the two (a half turn on while
        // the terms point more than a quarter turn apart).
        for t in [0.0, 0.5, 2.0, 3.7, 6.2, RISE + BEAT + FALL] {
            let theta = gauge(omega, t);
            assert!((theta.0 - theta.1 - omega * t).abs() < 1e-12, "t {t}");
            let a = phase_of((H, H), (1.0, 0.0), theta);
            let b = phase_of((H, H), (0.0, 1.0), theta);
            let both = phase_of((H, H), (1.0, 1.0), theta);
            let flip = if (0.5 * omega * t).cos_det() < 0.0 {
                PI
            } else {
                0.0
            };
            let mean = (FRAC_PI_2 + 0.5 * (theta.0 + theta.1) + flip - both).rem_euclid(TAU);
            assert!(mean.min(TAU - mean) < 1e-9, "t {t}");
            let rel = (a - b - omega * t).rem_euclid(TAU);
            assert!(rel.min(TAU - rel) < 1e-9, "t {t}");
        }
    }

    #[test]
    fn orb_a8_the_phase_winds_with_the_sign_of_the_energy_step() {
        // ω = (E_b − E_a)/ħ: up the ladder positive, down it negative, same size.
        for (lo, hi) in [(1, 2), (2, 3), (3, 4)] {
            let up = beat_omega(lo, hi);
            assert!(up > 0.0 && (up - TAU * beat_hz(lo, hi)).abs() < 1e-12);
            assert_eq!(beat_omega(hi, lo), -up);
        }
        // Stepping the atom into a downward beat (2p_x → 1s, the loop's last transition):
        // the term of the lower, final state (ψ_b) runs ahead of the upper one's, by |ω|t
        // = −(E_b − E_a)t, and in an upward beat (1s → 2p_z) behind it.
        for (first, down) in [(K - 1, true), (0, false)] {
            let mut atom = atom_at(first, 80, 24);
            while atom.phase == Phase::Hold {
                atom.step(SIM_DT);
            }
            for _ in 0..30 {
                atom.step(SIM_DT);
            }
            let (a, b) = (GALLERY[atom.at], GALLERY[atom.next()]);
            assert_eq!(b.n() < a.n(), down, "{a:?} → {b:?}");
            let wt = atom.theta.0 - atom.theta.1;
            let lead = atom.theta.1 - atom.theta.0;
            let want = -beat_omega(a.n(), b.n()) / beat_omega(a.n(), b.n()).abs();
            assert!(wt.abs() > 0.1, "{wt}");
            assert!(
                (lead - want * wt.abs()).abs() < 1e-9,
                "{a:?} → {b:?}: lead {lead}, ωt {wt}"
            );
        }
    }

    #[test]
    fn orb_a8_single_term_hues_turn_no_faster_than_1_5_omega_and_meet_the_holds() {
        // Where only one term is non-zero its hue is pure gauge (θ_a or θ_b). Over every
        // transition of the loop those hues start where the hold left them, end where the
        // next hold starts, and never turn faster than 1.5|ω| (the fastest colour motion
        // on screen is the beat, not bookkeeping).
        let turned = |p: f64, q: f64| {
            let d = (p - q).rem_euclid(TAU);
            d.min(TAU - d)
        };
        let hues = |atom: &Atom| {
            let c = (atom.mix.c_a, atom.mix.c_b);
            (
                phase_of(c, (1.0, 0.0), atom.theta),
                phase_of(c, (0.0, 1.0), atom.theta),
            )
        };
        let dt = SIM_DT;
        for first in 0..K {
            let mut atom = atom_at(first, 80, 24);
            while atom.phase == Phase::Hold {
                atom.step(dt);
            }
            let (a, b) = (GALLERY[atom.at], GALLERY[atom.next()]);
            let omega = beat_omega(a.n(), b.n()).abs();
            let mut prev = (FRAC_PI_2, FRAC_PI_2);
            loop {
                let now = hues(&atom);
                if atom.phase == Phase::Hold {
                    // The next hold: its hue is π/2 again, reached without a jump.
                    assert!(
                        turned(now.1, prev.1) <= 1.5 * omega * dt + 1e-9,
                        "{a:?}→{b:?}"
                    );
                    assert!(turned(now.0, FRAC_PI_2) < 1e-9);
                    break;
                }
                // A term with no amplitude has no hue (the fall ends at c_a = 0 exactly).
                let live = [atom.mix.c_a > 1e-9, atom.mix.c_b > 1e-9];
                for (k, (p, q)) in [(now.0, prev.0), (now.1, prev.1)].into_iter().enumerate() {
                    if !live[k] {
                        continue;
                    }
                    let rate = turned(p, q) / dt;
                    assert!(
                        rate <= 1.5 * omega + 1e-6,
                        "{a:?}→{b:?} term {k} at t {:.2}: {rate:.2} rad/s vs ω {omega:.2}",
                        atom.t
                    );
                }
                prev = now;
                atom.step(dt);
            }
        }
    }

    #[test]
    fn orb_a8_both_signs_of_a_hold_are_equally_bright_and_over_the_glow() {
        use sciart_render::{Rgb, Theme};
        let luma =
            |c: Rgb| 0.2126 * f64::from(c.0) + 0.7152 * f64::from(c.1) + 0.0722 * f64::from(c.2);
        let haze = Ink::new(Role::Ramp(GLOW_RAMP), HAZE_MAX);
        // Rainbow is full-saturation HSV, whose blue is dark in any use; it separates the
        // lobes by hue alone.
        for theme in [Theme::Astro, Theme::Mono] {
            let [pos, neg, glow] = [Ink::new(Role::Phase, POS), Ink::new(Role::Phase, NEG), haze]
                .map(|i| luma(theme.color(i, 0, 0)));
            // Mono greys phase by its distance from 0 (RND-R10): ±π/2 are the same grey.
            if theme == Theme::Mono {
                assert!((pos - neg).abs() < 1.0, "{theme:?}: {pos} vs {neg}");
            }
            // Dots always stand out over the brightest glow behind them: ≥ 2.5× its luma in
            // astro, ≥ 2× in mono (whose ±π/2 grey is mid-grey, RND-R10).
            let ratio = if theme == Theme::Mono { 2.0 } else { 2.5 };
            assert!(
                pos.min(neg) >= ratio * glow,
                "{theme:?}: dots {pos}/{neg}, glow {glow}"
            );
        }
    }

    #[test]
    fn orb_a8_dots_stand_out_over_the_glow_in_every_frame() {
        // Over a whole loop at 80×24, every dotted cell over a glow: its dots have ≥ 2.5×
        // the glow's luma in astro and ≥ 2× in mono, for whatever phase they wear (in a
        // beat, phases near 0 are near-black in mono, RND-R10).
        use sciart_render::{Rgb, Theme};
        let luma =
            |c: Rgb| 0.2126 * f64::from(c.0) + 0.7152 * f64::from(c.1) + 0.0722 * f64::from(c.2);
        let mut s = scene(0, 80, 24);
        let mut ctx = Ctx::new(1, s.size);
        let mut c = Canvas::new(s.size, GlyphMode::Unicode);
        let mut checked = 0;
        for _ in 0..(ROCK_PERIOD * K as f64 / 0.1) as usize {
            s.update(0.1, &mut ctx);
            c.clear();
            s.render(&mut c);
            for cell in c.cells().iter().filter(|c| c.ink.role == Role::Phase) {
                if cell.bg.role != Role::Ramp(GLOW_RAMP) || cell.bg.level == 0 {
                    continue;
                }
                for (theme, ratio) in [(Theme::Astro, 2.5), (Theme::Mono, 2.0)] {
                    let (dot, bg) = (theme.color(cell.ink, 0, 0), theme.color(cell.bg, 0, 0));
                    assert!(
                        luma(dot) >= ratio * luma(bg),
                        "{theme:?}: phase level {} over glow {}",
                        cell.ink.level,
                        cell.bg.level
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked} cells");
    }

    #[test]
    fn orb_a8_cloud_cells_wear_the_phase_and_both_signs_show() {
        // 2p_z hold: every cloud cell is a Phase cell at exactly +π/2 or −π/2.
        let s = scene(1, 80, 24);
        let mut c = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut c);
        let (mut pos, mut neg) = (0, 0);
        for cell in c
            .cells()
            .iter()
            .filter(|c| c.ch as u32 >= 0x2801 && c.ch as u32 <= 0x28FF)
        {
            assert_eq!(cell.ink.role, Role::Phase);
            match cell.ink.level {
                POS => pos += 1,
                NEG => neg += 1,
                l => panic!("level {l} in a real hold"),
            }
        }
        assert!(pos > 20 && neg > 20, "{pos} positive, {neg} negative cells");
        // Mid-beat of 1s → 2p_z the phase winds: many cells leave the two real levels.
        let mut s = scene(0, 80, 24);
        let mut ctx = Ctx::new(1, s.size);
        while s.atom.as_ref().unwrap().phase != Phase::Beat {
            s.update(SIM_DT, &mut ctx);
        }
        for _ in 0..20 {
            s.update(SIM_DT, &mut ctx);
        }
        let mut c = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut c);
        let cloud: Vec<u8> = c
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Phase)
            .map(|c| c.ink.level)
            .collect();
        let complex = cloud.iter().filter(|&&l| l != POS && l != NEG).count();
        assert!(
            complex * 4 >= cloud.len(),
            "{complex} of {} cells off the real axis",
            cloud.len()
        );
    }

    #[test]
    fn orb_a8_each_cell_wears_the_phase_of_its_nearest_point() {
        // Mid-beat of 3d_z2 → 4f_z3, where front and back of the cloud differ in phase.
        let mut s = scene(2, 80, 24);
        let mut ctx = Ctx::new(1, s.size);
        while s.atom.as_ref().unwrap().phase != Phase::Beat {
            s.update(SIM_DT, &mut ctx);
        }
        s.update(SIM_DT, &mut ctx);
        let atom = s.atom.as_ref().unwrap();
        let (ln_r90, ln_g) = atom.framing();
        let (inv_r90, scale) = ((-ln_r90).exp(), s.radius() * ln_g.exp());
        let (cx, cy) = s.centre();
        let centre = (cx + 0.25, cy + 0.125);
        let proj = atom.camera().projector();
        let mut best: HashMap<(i32, i32), (f64, u8)> = HashMap::new();
        atom.for_each_visible(atom.fade(), |_, _, [x, y, z], phase| {
            let world = [x * inv_r90, z * inv_r90, -y * inv_r90];
            let (sx, sy, d) = proj.project(world, centre, scale).unwrap();
            if Dots::from_cells(sx, sy).1 >= 4.0 * 23.0 - 0.5 {
                return;
            }
            let key = (sx.floor() as i32, sy.floor() as i32);
            let e = best.entry(key).or_insert((f64::NEG_INFINITY, 0));
            if d > e.0 {
                *e = (d, ink::phase(phase).level);
            }
        });
        let mut c = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut c);
        let mut checked = 0;
        for (&(x, y), &(_, level)) in &best {
            let cell = c.get(x, y).unwrap();
            if cell.ink.role == Role::Phase {
                assert_eq!(cell.ink.level, level, "cell ({x},{y})");
                checked += 1;
            }
        }
        assert!(checked > 50, "{checked} cells");
    }

    #[test]
    fn orb_a9_a_dim_glow_sits_under_the_cloud_and_nowhere_else() {
        // 4f_z3 hold at 80×24, once the glow has eased in.
        let mut s = scene(3, 80, 24);
        let mut ctx = Ctx::new(1, s.size);
        for _ in 0..(GLOW_IN / SIM_DT) as usize + 30 {
            s.update(SIM_DT, &mut ctx);
        }
        let mut c = Canvas::new(s.size, GlyphMode::Unicode);
        s.render(&mut c);
        let glow = |ink: Ink| ink.role == Role::Ramp(GLOW_RAMP) && ink.level > 0;
        let (w, h) = (80, 24);
        let cloud: Vec<&Cell> = c
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Phase)
            .collect();
        let lit_bg = cloud.iter().filter(|c| glow(c.bg)).count();
        assert!(
            lit_bg * 3 >= cloud.len() * 2,
            "{lit_bg} of {} dotted cells glow",
            cloud.len()
        );
        for cell in c.cells() {
            for ink in [cell.ink, cell.bg] {
                if ink.role == Role::Ramp(GLOW_RAMP) {
                    assert!(ink.level <= HAZE_MAX, "haze level {}", ink.level);
                }
            }
        }
        // Corners stay black and the label row takes no glow.
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 2), (w - 1, h - 2)] {
            assert_eq!(*c.get(x, y).unwrap(), Cell::BLANK, "corner ({x},{y})");
        }
        for x in 0..w {
            let cell = c.get(x, h - 1).unwrap();
            assert!(!glow(cell.ink) && !glow(cell.bg), "label row cell {x}");
        }
        // ASCII mode draws no glow at all.
        let mut a = Canvas::new(s.size, GlyphMode::Ascii);
        s.render(&mut a);
        assert!(a.cells().iter().all(|c| !glow(c.ink) && !glow(c.bg)));
        // The glow is the points' own density, unblurred: in the 2p_z hold the nodal plane
        // (the nucleus row) glows far less than the lobes above and below it, and the
        // halo's faint end is feathered (it holds more than one level below the cap).
        let mut s = scene(1, 80, 24);
        let mut ctx = Ctx::new(1, s.size);
        for _ in 0..(GLOW_IN / SIM_DT) as usize + 60 {
            s.update(SIM_DT, &mut ctx);
        }
        let (_, ny) = s.nucleus();
        let row_glow =
            |y: i32| -> u32 { (0..80).map(|x| u32::from(s.glow.cell_bg(x, y).level)).sum() };
        let (node, lobe) = (row_glow(ny), row_glow(ny - 3).max(row_glow(ny + 3)));
        assert!(2 * node < lobe, "node row glow {node}, lobe rows {lobe}");
        let mut levels: Vec<u8> = (0..80)
            .flat_map(|x| (0..46).map(move |y| (x, y)))
            .filter_map(|(x, y)| s.glow.get(x, y))
            .filter(|i| i.role == Role::Ramp(GLOW_RAMP))
            .map(|i| i.level)
            .collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() > 20, "{} glow levels", levels.len());
    }

    #[test]
    fn orb_a9_the_glow_ramps_softly_from_edge_to_core() {
        // 2p_z hold at 200×50, glow fully in: down the column through the nucleus, from
        // the first glowing pixel of the upper lobe to its brightest, the glow climbs over
        // at least 3 pixels and never jumps by more than half the cap in one pixel.
        let mut s = scene(1, 200, 50);
        let mut ctx = Ctx::new(1, s.size);
        for _ in 0..(3.0 / SIM_DT) as usize {
            s.update(SIM_DT, &mut ctx);
        }
        let (nx, ny) = s.nucleus();
        let mut ramps = 0;
        for x in nx - 4..=nx + 4 {
            let col: Vec<u8> = (0..2 * ny)
                .map(|y| s.glow.get(x, y).map_or(0, |i| i.level))
                .collect();
            let Some(start) = col.iter().position(|&l| l > 0) else {
                continue;
            };
            let peak = (start..col.len()).max_by_key(|&y| col[y]).unwrap();
            for y in start.max(1)..=peak {
                let jump = i32::from(col[y]) - i32::from(col[y - 1]);
                assert!(
                    jump <= i32::from(HAZE_MAX / 2),
                    "column {x} pixel {y}: {col:?}"
                );
            }
            ramps += usize::from(peak >= start + 3);
        }
        assert!(ramps >= 7, "{ramps} of 9 columns ramp over ≥ 3 pixels");
    }

    #[test]
    fn orb_a9_the_glow_shows_stationary_states_only() {
        // One loop at 80×24: no glow anywhere during a transition, full glow mid-hold.
        let mut s = scene(0, 80, 24);
        let mut ctx = Ctx::new(1, s.size);
        let lit = |s: &ElectronOrbitals| {
            (0..80)
                .flat_map(|x| (0..46).map(move |y| (x, y)))
                .filter(|&(x, y)| s.glow.get(x, y).is_some_and(|i| i.level > 0))
                .count()
        };
        let (mut mid_hold, mut moving) = (0, 0);
        for _ in 0..(ROCK_PERIOD * K as f64 / 0.05) as usize {
            s.update(0.05, &mut ctx);
            let atom = s.atom.as_ref().unwrap();
            let local = atom.timeline.at(atom.t).local;
            if atom.phase != Phase::Hold {
                assert_eq!(lit(&s), 0, "glow in {:?} at t {:.2}", atom.phase, atom.t);
                moving += 1;
            } else if (GLOW_IN + 0.5..HOLD - GLOW_OUT).contains(&local) {
                assert!(
                    lit(&s) > 20,
                    "mid-hold at t {:.2}: {} pixels",
                    atom.t,
                    lit(&s)
                );
                mid_hold += 1;
            }
        }
        assert!(mid_hold > 100 && moving > 500, "{mid_hold} {moving}");
    }

    #[test]
    fn orb_a7_the_cloud_is_framed_in_black() {
        // One loop of the real scene at dt = 0.1, centred on the rows above the label. In
        // holds and beats (the larger state of the pair sets the scale) no dot reaches the
        // outermost rows (blank rows: seed 42 ≥ 2, seeds 0–7 ≥ 1 at 80×24 and ≥ 2 at
        // 200×50); over the loop the χ ramps leave ≤ 0.04 cells per frame on them (bound
        // 0.1; mixing the smaller state's scale in during a beat gave 0.58).
        let seed = 42;
        for (w, h) in [(80u16, 24u16), (200, 50)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(seed, size);
            let mut s = ElectronOrbitals::default();
            s.resize(size, &mut ctx);
            let last = usize::from(h) - 2;
            let (mut edge, mut frames, mut margin) = (0, 0, usize::MAX);
            for _ in 0..1040 {
                s.update(0.1, &mut ctx);
                let cells = cell_dots(&s);
                let lit = |y: usize| cells[y].iter().filter(|&&c| c > 0).count();
                let rows: Vec<usize> = (0..=last).filter(|&y| lit(y) > 0).collect();
                let (top, bottom) = (rows[0], rows[rows.len() - 1]);
                let phase = s.atom.as_ref().unwrap().phase;
                if matches!(phase, Phase::Hold | Phase::Beat) {
                    margin = margin.min(top).min(last - bottom);
                }
                edge += lit(0) + lit(last);
                frames += 1;
            }
            let mean = edge as f64 / f64::from(frames);
            assert!(
                margin >= 1,
                "{w}x{h}: {margin} blank rows in a hold or beat"
            );
            assert!(mean <= 0.1, "{w}x{h}: {mean:.2} edge-row cells per frame");
        }
    }
}
