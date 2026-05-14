//! An emission nebula with dust pillars where gas falls onto protostars that ignite and
//! launch bipolar jets (SPEC-002 `stellar_nursery`, NUR-A1..A3).
//!
//! Seen as a narrowband image (HOO palette: Hα red, [O III] blue, false colour) lit by an
//! off-frame cluster that carves the pillars and lights their rims from one side.
//!
//! Positions are in rows (x on screen = `x · ASPECT`). The nebula is a domain-warped sum of
//! sines whose warp is chosen *separable* (x is warped by a function of y and vice versa),
//! so every mode's phase splits into a column part plus a row part and a frame costs
//! O((w + h)·modes) trig calls instead of O(w·h·modes).

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::particles::kepler_speed;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{ink, raster, rng};

pub static META: Meta = Meta {
    name: "stellar_nursery",
    category: Category::DeepCosmos,
    description: "A glowing nebula where gas streams onto protostars that ignite",
    title: "Star birth · gas falls inward",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(StellarNursery::default())
}

/// Sine terms in the nebula field: two octaves of structure with a 1/f-like spectrum.
const MODES: usize = 12;
/// Largest internal integration step; `update` sub-steps larger `dt`s.
const MAX_STEP: f64 = 1.0 / 60.0;
/// Ignition mass (normalized units).
pub(crate) const M_IGN: f64 = 1.0;
/// Gas within this many rows of its protostar is absorbed.
pub(crate) const R_ABS: f64 = 0.8;
/// Gravitational softening length (rows).
const EPS: f64 = 1.0;
/// Brightness ramp after ignition (s).
pub(crate) const IGNITION_RAMP: f64 = 1.0;
/// Main-sequence display time before a star fades (s).
pub(crate) const SHINE: f64 = 25.0;
/// Fade-out before replacement (s).
pub(crate) const FADE: f64 = 2.5;
const GAS_PER_AREA: f64 = 0.03;
pub(crate) const GAS_MAX: usize = 600;
pub(crate) const JET_MAX: usize = 240;
/// Target free-fall time from the feeding radius (s); fixes `gm` per size.
const T_FF: f64 = 3.0;
/// Time to fill an empty gas pool (s).
const T_FILL: f64 = 2.0;
/// Typical infall time of a parcel (s), used to normalize the parcel mass.
const T_ARRIVE: f64 = 2.5;
/// Typical accretion time from zero mass to ignition (s).
const ACC_TIME: f64 = 16.0;
/// Closest spawn distance to a protostar (rows).
const R_SPAWN_MIN: f64 = 3.0;
/// Jet particles per second per freshly ignited star.
const JET_RATE: f64 = 90.0;
/// e-folding time of jet activity after ignition (s): outflows weaken as accretion stops.
const JET_DECAY: f64 = 9.0;
/// Period of episodic ejection, which is what makes Herbig–Haro knots (s).
const JET_PULSE: f64 = 0.8;

/// Nebula z-score below which nothing is drawn, the span mapped to full brightness, and
/// the exposure's contrast: above 1 it keeps most gas dim, so only the filaments near
/// the ionisation front reach the ramp's top and the sky between stays black.
const NEB_THRESHOLD: f64 = 0.0;
const NEB_SPAN: f64 = 2.6;
const NEB_GAMMA: f64 = 1.5;
/// The exposure's shoulder: brightness `peak·(1 − e^{−k·t})^γ` for `t` the z-score over
/// `NEB_SPAN`, approaching (never clipping at) each line's peak, [O III] below white so
/// its cyan filaments keep their texture (NUR-A6).
const NEB_SHOULDER: f64 = 1.6;
const OIII_PEAK: f64 = 0.85;
const HA_PEAK: f64 = 0.95;
/// Nebula levels below this dissolve into black space (the ramps' floors are tinted).
const NEB_FLOOR: u8 = 24;
/// The ionising cluster sits off-frame at this angle from straight up, on a seeded side.
const SOURCE_TILT: f64 = 35.0 * std::f64::consts::PI / 180.0;
/// How far the pillar tips lean towards the source, as a fraction of the width.
const LEAN: (f64, f64) = (0.03, 0.16);
/// Brightness gradient towards the source (z-score across the frame).
const SOURCE_BIAS: f64 = 0.9;
/// The cluster's light bleeding in from its corner: peak z-score and Gaussian radius (a
/// fraction of the frame's larger pixel side), centred just off-frame.
const SOURCE_GLOW: f64 = 1.2;
const SOURCE_GLOW_R: f64 = 0.3;
/// Ionisation proxy: `ION_BASE + ION_SLOPE·u` towards the source, plus `ION_BUBBLE` per
/// unit of HII-bubble or source glow, less `ION_DENSE` per unit of brightness (dense gas
/// recombines to Hα). [O III] lies where it exceeds ½: a zone round the source, its
/// edge stippled over `±ION_BAND` rather than cut.
const ION_BASE: f64 = 0.3;
const ION_SLOPE: f64 = 0.8;
const ION_BUBBLE: f64 = 0.5;
const ION_DENSE: f64 = 0.1;
const ION_BAND: f64 = 0.025;
/// Dust: `Ramp(HAlpha)` levels of the pillars' textured interior (dark wine, so they
/// stand as silhouettes), plus up to `LIMB` more a few pixels in from a lit face (limb
/// brightening: the column reads as lit in 3-D, not as a cut-out). Rims: levels from barely
/// lit to full light, and the light below which a rim pixel is plain dust.
const DUST_LEVEL: (f64, f64) = (0.0, 28.0);
const LIMB: f64 = 24.0;
const RIM_LEVEL: (f64, f64) = (60.0, 250.0);
const RIM_MIN: f64 = 0.08;
/// A rim is lit by `smoothstep` of `n̂·d̂` from `RIM_LIT.0` to `RIM_LIT.1` (full); `n̂` is
/// the outward normal of the pillar's 7 × 7-smoothed outline. The dark end lies past
/// grazing, so faces edge-on to the source keep a dim rim: the diffuse ionising field and
/// the photoevaporative flow light them too (NUR-A4).
const RIM_LIT: (f64, f64) = (-0.2, 0.8);
/// An ionisation front is brightest at the tip facing the source and fades down the
/// flanks to this share of it at the base.
const RIM_BASE: f64 = 0.55;
/// Pillar outline, as fractions of the half-width: lobes and necks (long periods, so the
/// rim's normal turns slowly and never zips, NUR-A4) and fine roughness.
const LOBE: f64 = 0.3;
/// Pillar widths scale with the terminal width, but never below this many columns' worth.
const MIN_SPAN: f64 = 110.0;
const ROUGH: f64 = 0.02;
/// Half-width of the outline smoothing (pixels), the light of a rim's second pixel, and
/// how deep (pixels) the limb brightening reaches.
const RIM_REACH: i32 = 3;
const RIM_INNER: f64 = 0.45;
const LIMB_REACH: usize = 5;
/// Dust texture: value-noise lattice spacings (pixels) of its two octaves.
const TEX_CELL: (f64, f64) = (6.0, 2.5);
/// Mask values: open sky, pillar interior; rims store `RIM + lit·(255 − RIM)`.
const OPEN: u8 = 0;
const DUST: u8 = 1;
const RIM: u8 = 2;
/// Temperature (K) of young stars once shining.
const SHINING_K: f64 = 22_000.0;
/// Star glows (pixels): an embedded protostar's Gaussian glow radius and peak `Ramp(Ember)`
/// value, growing from `.0` to `.1` with mass (an infrared source seen through its dust,
/// always dimmer than the lit rims); a shining star's glare radius and peak Ice level.
const EMBEDDED_HALO: (f64, f64) = (2.0, 3.4);
const EMBEDDED_PEAK: (f64, f64) = (0.5, 0.75);
const SHINING_HALO: f64 = 3.5;
const GLARE: f64 = OIII_PEAK;
const STAR: Glyph = Glyph::new('✦', '*');
/// Jet streak length (s of flight) and the age fraction after which a jet is a knot.
const JET_STREAK: f64 = 0.3;
const KNOT_AGE: f64 = 0.6;
/// Gas streak length: seconds of infall drawn behind each parcel.
const STREAK: f64 = 0.35;
const ASCII_RAMP: [char; 4] = ['.', ':', '+', '#'];

/// Radial acceleration toward a protostar, `gm / (d² + ε²)` (Plummer-softened).
#[inline]
pub(crate) fn gravity_accel(gm: f64, d: f64) -> f64 {
    gm / (d * d + EPS * EPS)
}

/// One semi-implicit Euler step of radial infall: kick `vr` (inward speed, ≥ 0) with the
/// softened pull, then drift inward. Distance can only shrink (NUR-A1) because the
/// inward speed only grows and the drift subtracts it.
pub(crate) fn infall_step(r: f64, vr: f64, gm: f64, dt: f64) -> (f64, f64) {
    let vr = vr.max(0.0) + gravity_accel(gm, r) * dt.max(0.0);
    ((r - vr * dt.max(0.0)).max(0.0), vr)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Phase {
    /// Embedded protostar gathering gas.
    Accreting,
    /// Ignited; the value is seconds since ignition.
    Shining(f64),
    /// Dying away; seconds since the fade began.
    Fading(f64),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Protostar {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) mass: f64,
    pub(crate) phase: Phase,
    /// Jet axis angle (rad), in row units.
    jet: f64,
    /// Rotation sense of the infalling envelope (±1).
    spin: f64,
    /// Bumped on replacement so stale gas can recognise its target is gone.
    pub(crate) generation: u32,
    jet_acc: f64,
    /// Directions of the filaments that feed this hub (rad).
    feeds: [f64; 3],
    /// Relative gas supply of this hub.
    richness: f64,
}

impl Protostar {
    fn new(x: f64, y: f64, rng: &mut impl Rng, generation: u32, mass: f64) -> Self {
        Self {
            x,
            y,
            mass,
            phase: Phase::Accreting,
            jet: rng.random_range(0.0..TAU),
            spin: if rng.random_bool(0.5) { 1.0 } else { -1.0 },
            generation,
            jet_acc: 0.0,
            feeds: {
                let a: f64 = rng.random_range(0.0..TAU);
                [
                    a,
                    a + rng.random_range(1.6..=2.6),
                    a - rng.random_range(1.6..=2.6),
                ]
            },
            richness: rng.random_range(0.5..=1.5),
        }
    }

    /// Add absorbed gas mass; returns `true` iff this call ignites the star — i.e. exactly
    /// when the mass first reaches `M_IGN` (NUR-A2). Negative input is ignored so the mass
    /// never decreases.
    pub(crate) fn accrete(&mut self, dm: f64) -> bool {
        if dm > 0.0 {
            self.mass += dm;
        }
        if self.phase == Phase::Accreting && self.mass >= M_IGN {
            self.phase = Phase::Shining(0.0);
            true
        } else {
            false
        }
    }

    /// Visible brightness in `[0, 1]`: 0 while embedded, smoothstep over the 1 s
    /// ignition ramp, linear fade at the end.
    pub(crate) fn brightness(&self) -> f64 {
        match self.phase {
            Phase::Accreting => 0.0,
            Phase::Shining(a) => {
                let s = (a / IGNITION_RAMP).clamp(0.0, 1.0);
                s * s * (3.0 - 2.0 * s)
            }
            Phase::Fading(a) => (1.0 - a / FADE).clamp(0.0, 1.0),
        }
    }

    fn is_accreting(&self) -> bool {
        self.phase == Phase::Accreting
    }
}

/// Infalling gas, stored in polar coordinates about its target so its distance to the
/// target is exactly `r` (NUR-A1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gas {
    /// Stable identity, so tests can follow a parcel across `swap_remove` compaction.
    #[cfg_attr(not(test), expect(dead_code, reason = "only read by the NUR-A1 tests"))]
    pub(crate) id: u32,
    pub(crate) star: usize,
    pub(crate) generation: u32,
    pub(crate) r: f64,
    theta: f64,
    /// Inward radial speed (rows/s), ≥ 0.
    vr: f64,
    /// Specific angular momentum (rows²/s); gives the streams a gentle spiral.
    l: f64,
    age: f64,
}

impl Gas {
    pub(crate) fn pos(&self, s: &Protostar) -> (f64, f64) {
        let (sn, cs) = self.theta.sin_cos_det();
        (s.x + self.r * cs, s.y + self.r * sn)
    }
}

/// A ballistic jet particle (rows, rows/s).
#[derive(Debug, Clone, Copy)]
struct Jet {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
    life: f64,
}

/// One nebula plane wave; `k` in rad/row, `omega` in rad/s.
#[derive(Debug, Clone, Copy)]
struct Mode {
    kx: f64,
    ky: f64,
    phase: f64,
    omega: f64,
    weight: f64,
}

/// A dust pillar as fractions of the screen, so it survives resizes.
#[derive(Debug, Clone, Copy)]
struct Pillar {
    base: f64,
    lean: f64,
    height: f64,
    /// Half-width at the base (fraction of the width), the neck's share of it, and the
    /// head's half-width as a multiple of the neck's.
    base_half: f64,
    neck: f64,
    head: f64,
    /// Lobe period as a fraction of the pillar's length, and each edge's lobe phase: the
    /// edges swell and pinch out of step, so the column has necks and bulges.
    lobe_period: f64,
    lobe_phase: (f64, f64),
    /// Side knobs (evaporating gaseous globules): position down the body, side (±1) and
    /// radius as a share of the local half-width.
    knobs: [(f64, f64, f64); 2],
}

impl Pillar {
    /// Left and right half-widths (columns) at `s ∈ [0, 1]` from the head's widest row
    /// (0) to the base (1), for a base half-width of `base_half` columns: a bulbous head
    /// over a narrower neck, a flaring base, and lobes that grow in below the head.
    fn half_widths(&self, s: f64, base_half: f64) -> (f64, f64) {
        let neck = (base_half * self.neck).max(0.8);
        let core = neck + (base_half - neck) * s.powf(1.5);
        let bulb = neck * self.head * (-(s / 0.24).powi(2)).exp();
        let half = core.max(bulb);
        let grow = ease::smoothstep((s - 0.1) / 0.2);
        let edge = |phase: f64, k: f64| {
            half * (1.0
                + LOBE * grow * (TAU * s / self.lobe_period + phase).sin_det()
                + ROUGH * (k * TAU * s / self.lobe_period + 2.0 * phase).sin_det())
        };
        (edge(self.lobe_phase.0, 7.0), edge(self.lobe_phase.1, 9.0))
    }
}

/// Separable domain warp: x is displaced by `amp·sin(qy·y + …)`, y by `amp·sin(qx·x + …)`.
#[derive(Debug, Clone, Copy, Default)]
struct Warp {
    amp: f64,
    qx: f64,
    qy: f64,
    px: f64,
    py: f64,
    nu: f64,
}

#[derive(Debug)]
pub struct StellarNursery {
    t: f64,
    w: usize,
    h: usize,
    modes: Vec<Mode>,
    warp: Warp,
    /// 1/σ of the raw field (so the sum reads as a z-score).
    inv_sigma: f64,
    /// Per-column and per-pixel-row `(sin, cos)` tables of the nebula modes.
    col: Vec<[(f64, f64); MODES]>,
    row: Vec<[(f64, f64); MODES]>,
    /// Position towards the ionising source, split into a column and a pixel-row part:
    /// `u = col_u[x] + row_u[y]`, from about −0.6 (far) to +0.6 (near).
    col_u: Vec<f64>,
    row_u: Vec<f64>,
    /// The source's glow, separable: `glow = col_g[x]·row_g[py]`.
    col_g: Vec<f64>,
    row_g: Vec<f64>,
    /// Side of the ionising source: −1 upper left, +1 upper right.
    source: f64,
    pillars: Vec<Pillar>,
    /// Pillar mask at pixel resolution (`OPEN`, `DUST` or a lit rim).
    mask: Vec<u8>,
    /// Static `Ramp(HAlpha)` level of each pillar pixel: lit rims and textured dust.
    dust: Vec<u8>,
    /// Seed of the dust texture.
    tex_salt: u64,
    tips: Vec<(f64, f64)>,
    stars: Vec<Protostar>,
    gas: ParticlePool<Gas>,
    jets: ParticlePool<Jet>,
    gm: f64,
    r_feed: f64,
    r_hii: f64,
    gas_mass: f64,
    spawn_acc: f64,
    next_id: u32,
    /// The nebula, and the gas, jets and spikes over it.
    pixels: Pixels,
    dots: Dots,
    /// Cells of the shining stars' `✦` glyphs.
    shining: [(i32, i32); MAX_STARS],
    n_shining: usize,
}

/// Protostars at once (the scene keeps 3–5).
const MAX_STARS: usize = 5;

impl Default for StellarNursery {
    fn default() -> Self {
        Self {
            t: 0.0,
            w: 0,
            h: 0,
            modes: Vec::new(),
            warp: Warp::default(),
            inv_sigma: 0.0,
            col: Vec::new(),
            row: Vec::new(),
            col_u: Vec::new(),
            row_u: Vec::new(),
            col_g: Vec::new(),
            row_g: Vec::new(),
            source: 1.0,
            pillars: Vec::new(),
            mask: Vec::new(),
            dust: Vec::new(),
            tex_salt: 0,
            tips: Vec::new(),
            stars: Vec::new(),
            gas: ParticlePool::with_capacity(0),
            jets: ParticlePool::with_capacity(0),
            gm: 0.0,
            r_feed: 0.0,
            r_hii: 0.0,
            gas_mass: 0.0,
            spawn_acc: 0.0,
            next_id: 0,
            pixels: Pixels::default(),
            dots: Dots::default(),
            shining: [(0, 0); MAX_STARS],
            n_shining: 0,
        }
    }
}

/// Unit vector from a pixel towards the ionising source on side `s` (y down).
fn to_source(s: f64) -> (f64, f64) {
    (s * SOURCE_TILT.sin_det(), -SOURCE_TILT.cos_det())
}

/// How strongly a dust surface is lit by the source (NUR-A4), from the gradient `grad` of
/// the smoothed dust mask (it points into the pillar, so the outward normal is
/// `n̂ = −grad/|grad|`): `smoothstep` of `n̂·d̂` over `RIM_LIT`, so faces turned towards
/// the source glow, grazing ones fade and faces turned away stay dark; 0 on a flat mask.
/// Being taken from a smoothed outline, `n̂` turns gradually along a pixel staircase.
pub(crate) fn rim_light(grad: (f64, f64), d: (f64, f64)) -> f64 {
    let len = grad.0.hypot(grad.1);
    if len < 1e-9 {
        return 0.0;
    }
    let facing = -(grad.0 * d.0 + grad.1 * d.1) / len;
    ease::smoothstep((facing - RIM_LIT.0) / (RIM_LIT.1 - RIM_LIT.0))
}

/// The share of dust in the `(2·RIM_REACH + 1)²` window around every pixel (clipped at
/// the edges): a smoothed outline whose gradient gives the rims their normals (a 7 × 7
/// box keeps them from flipping along pixel steps). Built on resize only.
fn smoothed_dust(mask: &[u8], w: usize) -> Vec<f32> {
    let h = mask.len() / w.max(1);
    let reach = RIM_REACH as usize;
    let dust = |i: usize| f32::from(u8::from(mask[i] != OPEN));
    // Separable box: rows, then columns.
    let mut rows = vec![0.0_f32; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(reach), (x + reach).min(w - 1));
            rows[y * w + x] = (a..=b).map(|k| dust(y * w + k)).sum::<f32>() / (b - a + 1) as f32;
        }
    }
    let mut out = vec![0.0_f32; mask.len()];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(reach), (y + reach).min(h - 1));
        for x in 0..w {
            out[y * w + x] = (a..=b).map(|k| rows[k * w + x]).sum::<f32>() / (b - a + 1) as f32;
        }
    }
    out
}

/// Smooth value noise in `[0, 1)` at pixel `(x, y)` on a lattice of spacing `cell` pixels
/// (hashed corners, smoothstep-blended): the dust's texture.
fn value_noise(salt: u64, x: f64, y: f64, cell: f64) -> f64 {
    let (u, v) = (x / cell, y / cell);
    let (i, j) = (u.floor(), v.floor());
    let corner = |di: i32, dj: i32| rng::hash01(salt ^ rng::cell_key(i as i32 + di, j as i32 + dj));
    let (fu, fv) = (ease::smoothstep(u - i), ease::smoothstep(v - j));
    ease::lerp(
        ease::lerp(corner(0, 0), corner(1, 0), fu),
        ease::lerp(corner(0, 1), corner(1, 1), fu),
        fv,
    )
}

impl StellarNursery {
    fn init_sky(&mut self, rng: &mut impl Rng) {
        self.modes = (0..MODES)
            .map(|_| {
                // Log-uniform wavelengths, longer ones stronger: filaments with texture.
                let wavelength: f64 = (rng.random_range(5f64.ln()..=42f64.ln())).exp();
                let dir: f64 = rng.random_range(0.0..TAU);
                let k = TAU / wavelength;
                Mode {
                    kx: k * dir.cos_det(),
                    ky: k * dir.sin_det(),
                    phase: rng.random_range(0.0..TAU),
                    omega: rng.random_range(0.04..=0.14)
                        * if rng.random_bool(0.5) { 1.0 } else { -1.0 },
                    weight: rng.random_range(0.5..=1.0) * (wavelength / 42.0).powf(0.8),
                }
            })
            .collect();
        // Independent random phases: variance of Σ aᵢ sin(·) is Σ aᵢ² / 2.
        let var: f64 = self.modes.iter().map(|m| m.weight * m.weight).sum::<f64>() / 2.0;
        self.inv_sigma = 1.0 / var.sqrt();
        self.warp = Warp {
            amp: rng.random_range(3.0..=5.0),
            qx: TAU / rng.random_range(18.0..=34.0),
            qy: TAU / rng.random_range(10.0..=20.0),
            px: rng.random_range(0.0..TAU),
            py: rng.random_range(0.0..TAU),
            nu: rng.random_range(0.08..=0.16),
        };
        self.source = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
        self.tex_salt = rng.random();
        let n = rng.random_range(2..=3);
        let slot = 1.0 / f64::from(n);
        self.pillars = (0..n)
            .map(|i| {
                let phase: f64 = rng.random_range(0.0..TAU);
                let mut knob = || {
                    let side = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                    (
                        rng.random_range(0.3..=0.85),
                        side,
                        rng.random_range(0.35..=0.6),
                    )
                };
                let knobs = [knob(), knob()];
                Pillar {
                    base: slot * (f64::from(i) + rng.random_range(0.3..=0.7)),
                    // Pillars point at the stars that sculpt them (NUR-A5).
                    lean: self.source * rng.random_range(LEAN.0..=LEAN.1),
                    height: rng.random_range(0.35..=0.75),
                    base_half: rng.random_range(0.04..=0.07),
                    neck: rng.random_range(0.45..=0.6),
                    head: rng.random_range(1.9..=2.3),
                    lobe_period: rng.random_range(0.3..=0.6),
                    lobe_phase: (phase, phase + rng.random_range(0.6..=1.6)),
                    // One or two knobs: a second, when absent, has zero radius.
                    knobs: [
                        knobs[0],
                        if rng.random_bool(0.5) {
                            knobs[1]
                        } else {
                            (0.0, 0.0, 0.0)
                        },
                    ],
                }
            })
            .collect();
    }

    /// Rasterise the pillars into the pixel `mask`, light their rims from the source, shade
    /// their dust into `dust` (static levels) and find their tips (rows).
    fn build_mask(&mut self) {
        let (w, ph) = (self.w, 2 * self.h);
        self.mask.clear();
        self.mask.resize(w * ph, OPEN);
        self.dust.clear();
        self.dust.resize(w * ph, 0);
        self.tips.clear();
        if w == 0 || ph == 0 {
            return;
        }
        // How far down its pillar each dust pixel lies (0 at the head, 1 at the base).
        let mut along = vec![1.0_f32; w * ph];
        let (wf, hf) = (w as f64, self.h as f64);
        for p in &self.pillars {
            let bottom = hf - 1.0;
            let tip_y = (bottom - p.height * hf).max(0.0);
            let length = (bottom - tip_y).max(1.0);
            // Narrow terminals keep the pillars as wide as at `MIN_SPAN` columns, so they
            // stay columns rather than sticks.
            let base_half = p.base_half * wf.max(MIN_SPAN);
            let s_at = |y: f64| ((y - tip_y) / length).clamp(0.0, 1.0);
            let centre = |y: f64| (p.base + p.lean * (1.0 - s_at(y))) * wf;
            let mut fill = |x: usize, py: usize, s: f64| {
                let i = py * w + x;
                self.mask[i] = DUST;
                along[i] = along[i].min(s as f32);
            };
            // The head's widest row, capped above by a round dome (a half-ellipse, round
            // on screen); the body below.
            let tip_half = p.half_widths(0.0, base_half).0;
            let dome = tip_half / ASPECT;
            for py in 0..ph {
                let yf = (py as f64 - 0.5) / 2.0;
                let (half_l, half_r) = if yf >= tip_y {
                    p.half_widths(s_at(yf), base_half)
                } else {
                    let d = (tip_y - yf) / dome.max(0.5);
                    if d > 1.0 {
                        continue;
                    }
                    let half = tip_half * (1.0 - d * d).sqrt();
                    (half, half)
                };
                let c = centre(yf);
                let x0 = (c - half_l).ceil().max(0.0);
                let x1 = (c + half_r).floor().min(wf - 1.0);
                if x0 <= x1 {
                    for x in x0 as usize..=x1 as usize {
                        fill(x, py, s_at(yf));
                    }
                }
            }
            // Side knobs: round globules half-emerged from the flank.
            for &(s, side, size) in &p.knobs {
                let yf = tip_y + s * length;
                let (half_l, half_r) = p.half_widths(s, base_half);
                let r = size * if side < 0.0 { half_l } else { half_r };
                if r < 1.0 {
                    continue;
                }
                let cx = centre(yf) + side * (if side < 0.0 { half_l } else { half_r } + 0.3 * r);
                let cy = 2.0 * yf + 0.5;
                let y_span =
                    (cy - r).floor().max(0.0) as usize..=((cy + r).ceil() as usize).min(ph - 1);
                for py in y_span {
                    for x in
                        (cx - r).floor().max(0.0) as usize..=((cx + r).ceil() as usize).min(w - 1)
                    {
                        if (x as f64 - cx).hypot(py as f64 - cy) <= r {
                            fill(x, py, s);
                        }
                    }
                }
            }
            self.tips.push((centre(tip_y) / ASPECT, tip_y + 0.5));
        }
        // Ionisation fronts: dust surfaces facing the source glow, brightest at the tip,
        // the rest stay dark. A rim is two pixels deep: full light on the surface,
        // `RIM_INNER` one pixel in. Deeper in, the dust is textured and limb-brightened.
        let d = to_source(self.source);
        let smooth = smoothed_dust(&self.mask, w);
        let at = |x: usize, y: usize| f64::from(smooth[y * w + x]);
        let open_within = |mask: &[u8], x: usize, y: usize, r: usize| {
            (y.saturating_sub(r)..=(y + r).min(ph - 1)).any(|ny| {
                (x.saturating_sub(r)..=(x + r).min(w - 1)).any(|nx| mask[ny * w + nx] == OPEN)
            })
        };
        for y in 0..ph {
            for x in 0..w {
                let i = y * w + x;
                if self.mask[i] == OPEN {
                    continue;
                }
                // Distance (pixels, Chebyshev) to open sky, up to the limb's reach.
                let depth = (1..=LIMB_REACH)
                    .find(|&r| open_within(&self.mask, x, y, r))
                    .unwrap_or(LIMB_REACH + 1);
                // A wide central difference of the smoothed outline: finely graded
                // directions, so the light turns smoothly along a pixel staircase.
                let grad = (
                    at((x + 2).min(w - 1), y) - at(x.saturating_sub(2), y),
                    at(x, (y + 2).min(ph - 1)) - at(x, y.saturating_sub(2)),
                );
                let facing = rim_light(grad, d);
                let tip = ease::lerp(1.0, RIM_BASE, f64::from(along[i]));
                let lit = facing
                    * match depth {
                        1 => 1.0,
                        2 => RIM_INNER,
                        _ => 0.0,
                    };
                if lit >= RIM_MIN {
                    self.mask[i] = RIM + (lit * f64::from(255 - RIM)).round() as u8;
                    self.dust[i] = ink::level(ease::lerp(RIM_LEVEL.0, RIM_LEVEL.1, lit * tip));
                    continue;
                }
                let limb = facing * (1.0 - (depth as f64 - 1.0) / LIMB_REACH as f64).max(0.0);
                let tex = 0.65 * value_noise(self.tex_salt, x as f64, y as f64, TEX_CELL.0)
                    + 0.35 * value_noise(!self.tex_salt, x as f64, y as f64, TEX_CELL.1);
                self.dust[i] =
                    ink::level(ease::lerp(DUST_LEVEL.0, DUST_LEVEL.1, tex) + LIMB * limb * tip);
            }
        }
    }

    /// Fill the per-column and per-pixel-row `(sin, cos)` tables for the current time.
    fn refresh_tables(&mut self) {
        let (t, wp) = (self.t, self.warp);
        for (x, c) in self.col.iter_mut().enumerate() {
            let xr = x as f64 / ASPECT;
            let dy = wp.amp * (wp.qx * xr + wp.px + wp.nu * t).sin_det();
            for (o, m) in c.iter_mut().zip(&self.modes) {
                *o = (m.kx * xr + m.ky * dy + m.phase + m.omega * t).sin_cos_det();
            }
        }
        for (py, r) in self.row.iter_mut().enumerate() {
            let yf = (py as f64 - 0.5) / 2.0;
            let dx = wp.amp * (wp.qy * yf + wp.py - 0.7 * wp.nu * t).sin_det();
            for (o, m) in r.iter_mut().zip(&self.modes) {
                *o = (m.ky * yf + m.kx * dx).sin_cos_det();
            }
        }
    }

    /// Nebula z-score at pixel `(x, py)`, brighter towards the source.
    fn field(&self, x: usize, py: usize) -> f64 {
        nebula_z(&self.modes, self.inv_sigma, &self.col[x], &self.row[py])
            + SOURCE_BIAS * (self.col_u[x] + self.row_u[py])
    }

    fn in_bounds(&self, x: f64, y: f64) -> bool {
        let xmax = (self.w as f64 - 1.0) / ASPECT;
        (0.0..=xmax).contains(&x) && (0.0..=self.h as f64 - 1.0).contains(&y)
    }

    fn cell(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (cx, cy) = ((x * ASPECT).round(), y.round());
        ((0.0..self.w as f64).contains(&cx) && (0.0..self.h as f64).contains(&cy))
            .then_some((cx as usize, cy as usize))
    }

    /// A protostar position away from the others (and inside the frame).
    fn place_star(&self, rng: &mut impl Rng, skip: Option<usize>) -> (f64, f64) {
        let (wr, hf) = (
            (self.w as f64 - 1.0).max(0.0) / ASPECT,
            (self.h as f64 - 1.0).max(0.0),
        );
        let min_sep = 0.22 * wr.min(hf * 2.0).max(1.0);
        let mut best = (wr / 2.0, hf / 2.0);
        let mut best_d = -1.0;
        for _ in 0..24 {
            let x = rng.random_range(0.12..=0.88) * wr;
            let y = rng.random_range(0.15..=0.8) * hf;
            let d = self
                .stars
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != skip)
                .map(|(_, s)| (s.x - x).hypot(s.y - y))
                .fold(f64::INFINITY, f64::min);
            if d > best_d {
                best = (x, y);
                best_d = d;
            }
            if d >= min_sep {
                break;
            }
        }
        best
    }

    fn nearest_accreting(&self, x: f64, y: f64) -> Option<usize> {
        self.stars
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_accreting())
            .map(|(i, s)| (i, (s.x - x).hypot(s.y - y)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Spawn one gas parcel in the nebula around an accreting protostar. Returns `false`
    /// when there is no room or nowhere to put it.
    fn spawn_gas(&mut self, rng: &mut impl Rng) -> bool {
        let accreting = self.stars.iter().filter(|s| s.is_accreting()).count();
        if accreting == 0 || self.gas.len() >= self.gas.capacity() {
            return false;
        }
        // Hubs differ in how rich their surroundings are, which desynchronizes ignitions.
        let total: f64 = self
            .stars
            .iter()
            .filter(|s| s.is_accreting())
            .map(|s| s.richness)
            .sum();
        let mut pick = rng.random_range(0.0..1.0) * total;
        let mut host = None;
        for s in self.stars.iter().filter(|s| s.is_accreting()) {
            host = Some(*s);
            pick -= s.richness;
            if pick < 0.0 {
                break;
            }
        }
        let Some(host) = host else {
            return false;
        };
        let (mut px, mut py) = (host.x, host.y);
        let mut found = false;
        for attempt in 0..6 {
            // Gas arrives along a hub's filaments rather than isotropically.
            let feed = host.feeds[rng.random_range(0..host.feeds.len())];
            let th = feed + rng.random_range(-0.22..=0.22);
            let u: f64 = rng.random();
            let r = R_SPAWN_MIN + (self.r_feed - R_SPAWN_MIN).max(0.0) * u.sqrt();
            let (x, y) = (host.x + r * th.cos_det(), host.y + r * th.sin_det());
            let Some((cx, cy)) = self.cell(x, y).filter(|_| self.in_bounds(x, y)) else {
                continue;
            };
            // Prefer the bright (dense) parts of the cloud; dust pillars count as dense too.
            if self.mask[2 * cy * self.w + cx] != OPEN
                || self.field(cx, 2 * cy) > NEB_THRESHOLD
                || attempt == 5
            {
                (px, py, found) = (x, y, true);
                break;
            }
        }
        if !found {
            return false;
        }
        let Some(target) = self.nearest_accreting(px, py) else {
            return false;
        };
        let s = self.stars[target];
        let r = (px - s.x).hypot(py - s.y);
        if r <= R_ABS + 0.5 {
            return false;
        }
        let v_circ = kepler_speed(self.gm, r);
        let gas = Gas {
            id: self.next_id,
            star: target,
            generation: s.generation,
            r,
            theta: (py - s.y).atan2(px - s.x),
            vr: rng.random_range(0.0..=0.15) * v_circ,
            l: s.spin * rng.random_range(0.1..=0.3) * r * v_circ,
            age: 0.0,
        };
        self.next_id = self.next_id.wrapping_add(1);
        self.gas.spawn(gas)
    }

    fn emit_jets(&mut self, i: usize, h: f64, rng: &mut impl Rng) {
        let s = self.stars[i];
        let Phase::Shining(age) = s.phase else { return };
        let pulse = (0.5 + 0.5 * (TAU * age / JET_PULSE).sin_det()).powi(2);
        let strength = s.brightness() * (-age / JET_DECAY).exp() * pulse;
        let mut acc = s.jet_acc + JET_RATE * strength * h;
        while acc >= 1.0 {
            acc -= 1.0;
            let sign = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
            let speed = rng.random_range(8.0..=12.0);
            let spread = rng.random_range(-0.08..=0.08);
            let (a_s, a_c) = (s.jet + spread).sin_cos_det();
            let jet = Jet {
                x: s.x + sign * 0.6 * a_c,
                y: s.y + sign * 0.6 * a_s,
                vx: sign * speed * a_c,
                vy: sign * speed * a_s,
                age: 0.0,
                life: rng.random_range(1.4..=2.2),
            };
            if !self.jets.spawn(jet) {
                acc = 0.0;
                break;
            }
        }
        self.stars[i].jet_acc = acc;
    }

    fn substep(&mut self, h: f64, rng: &mut impl Rng) {
        self.t += h;
        // Protostar life cycle.
        for i in 0..self.stars.len() {
            match self.stars[i].phase {
                Phase::Accreting => {}
                Phase::Shining(a) => {
                    self.stars[i].phase = if a + h >= SHINE {
                        Phase::Fading(0.0)
                    } else {
                        Phase::Shining(a + h)
                    };
                    self.emit_jets(i, h, rng);
                }
                Phase::Fading(a) if a + h < FADE => self.stars[i].phase = Phase::Fading(a + h),
                Phase::Fading(_) => {
                    let (x, y) = self.place_star(rng, Some(i));
                    let generation = self.stars[i].generation.wrapping_add(1);
                    let mass = rng.random_range(0.0..=0.4) * M_IGN;
                    self.stars[i] = Protostar::new(x, y, rng, generation, mass);
                }
            }
        }
        // Gas infall and accretion.
        let (gm, m_p) = (self.gm, self.gas_mass);
        let stars = &mut self.stars;
        self.gas.retain(|g| {
            let Some(s) = stars
                .get_mut(g.star)
                .filter(|s| s.generation == g.generation)
            else {
                return false;
            };
            let (r, vr) = infall_step(g.r, g.vr, gm, h);
            if r <= R_ABS {
                s.accrete(m_p);
                return false;
            }
            // ω = L / r², capped so a parcel never whips round in one step.
            g.theta += (g.l / (r * r) * h).clamp(-0.25, 0.25);
            (g.r, g.vr, g.age) = (r, vr, g.age + h);
            true
        });
        // Replenish the cloud.
        let accreting = self.stars.iter().filter(|s| s.is_accreting()).count();
        if accreting > 0 && self.gas.len() < self.gas.capacity() {
            // Each hub is fed at the same rate, so accretion time is independent of how
            // many hubs are active.
            let share = accreting as f64 / self.stars.len() as f64;
            self.spawn_acc += share * self.gas.capacity() as f64 / T_FILL * h;
            while self.spawn_acc >= 1.0 {
                self.spawn_acc -= 1.0;
                if !self.spawn_gas(rng) {
                    self.spawn_acc = 0.0;
                    break;
                }
            }
        } else {
            self.spawn_acc = 0.0;
        }
        // Jets coast outward, decelerating as they sweep up ambient gas.
        let drag = (1.0 - 0.5 * h).max(0.0);
        self.jets.retain(|j| {
            j.x += j.vx * h;
            j.y += j.vy * h;
            j.vx *= drag;
            j.vy *= drag;
            j.age += h;
            j.age < j.life
        });
    }

    /// Shade the nebula pixels (the M10 raster convention: `render` only blits).
    fn shade(&mut self) {
        // HII regions: each shining star ionizes a growing bubble (Strömgren sphere).
        let mut lamps = [(0.0, 0.0, 0.0, 0.0); MAX_STARS];
        let mut n = 0;
        for s in &self.stars {
            let b = s.brightness();
            let age = match s.phase {
                Phase::Shining(a) => a,
                Phase::Fading(_) => SHINE,
                Phase::Accreting => continue,
            };
            let r = self.r_hii * (1.0 - (-age / 3.0).exp());
            if b > 0.0 && r > 0.5 && n < lamps.len() {
                lamps[n] = (s.x * ASPECT, s.y, r, b);
                n += 1;
            }
        }
        let lamps = &lamps[..n];
        let Self {
            pixels,
            modes,
            inv_sigma,
            col,
            row,
            col_u,
            row_u,
            col_g,
            row_g,
            mask,
            dust,
            w,
            ..
        } = self;
        let w = *w;
        pixels.shade(|x, py| {
            let (xu, pyu) = (x as usize, py as usize);
            if mask[pyu * w + xu] != OPEN {
                // Dust is drawn after the fringe is dissolved.
                return Ink::BLANK;
            }
            let u = col_u[xu] + row_u[pyu];
            let z = nebula_z(modes, *inv_sigma, &col[xu], &row[pyu]) + SOURCE_BIAS * u;
            let yr = (f64::from(py) - 0.5) / 2.0;
            let mut glow = col_g[xu] * row_g[pyu];
            for &(lx, ly, r, b) in lamps {
                let (dx, dy) = ((f64::from(x) - lx) / ASPECT, yr - ly);
                let q = 1.0 - (dx * dx + dy * dy) / (r * r);
                if q > 0.0 {
                    glow += 2.2 * b * q * q;
                }
            }
            let z = z + glow;
            if z <= NEB_THRESHOLD {
                return Ink::BLANK;
            }
            let t = (z - NEB_THRESHOLD) / NEB_SPAN;
            let v = t.min(1.0);
            // [O III] in the highly ionised zone round the source and the stars' bubbles,
            // Hα beyond; the boundary is stippled over a band, not cut (NUR-A6).
            let ion = ION_BASE + ION_SLOPE * u + ION_BUBBLE * glow - ION_DENSE * v;
            let p = ease::smoothstep((ion - 0.5 + ION_BAND) / (2.0 * ION_BAND));
            let (line, peak) = if p > rng::hash01(rng::cell_key(x, py)) {
                (Ramp::Ice, OIII_PEAK)
            } else {
                (Ramp::HAlpha, HA_PEAK)
            };
            // A shoulder instead of a clip: the brightest gas approaches `peak` but never
            // flattens there, so filaments keep their texture (NUR-A6).
            let exposure = 1.0 - (-NEB_SHOULDER * t).exp();
            ink::ramp(line, peak * exposure.powf(NEB_GAMMA))
        });
        pixels.dissolve(NEB_FLOOR);
        // Pillars: textured dust and the ionisation fronts on the faces turned to the
        // source, fixed on resize (`build_mask`).
        for (i, (&m, &level)) in mask.iter().zip(dust.iter()).enumerate() {
            if m != OPEN {
                let (x, py) = ((i % w) as i32, (i / w) as i32);
                pixels.set(x, py, Ink::new(Role::Ramp(Ramp::HAlpha), level));
            }
        }
        self.star_glows();
    }

    /// Each protostar as a soft pixel glow that only ever brightens what is under it. An
    /// embedded star is an infrared source seen through its dust: a red-orange Gaussian
    /// glow in `Ramp(Ember)` that grows with mass, drawn only inside a pillar (in the open
    /// nebula it is its gas streams alone) and always dimmer than the lit rims. A shining
    /// star gets a blue glare in the Ice ramp, fainter than its `✦`, so the glyph still
    /// reads over it.
    fn star_glows(&mut self) {
        let w = self.w;
        for s in &self.stars {
            // Continuous pixel coordinates of the star.
            let (cx, cy) = Self::to_cells(s.x, s.y);
            let (px, py) = (cx, 2.0 * cy);
            // The glow's reach (pixels), ramp and peak, and its profile at `r / reach`.
            let (reach, ramp, peak, embedded) = match s.phase {
                Phase::Accreting => {
                    let (x, y) = (px.floor(), py.floor());
                    let in_dust = x >= 0.0
                        && y >= 0.0
                        && (x as usize) < w
                        && self
                            .mask
                            .get(y as usize * w + x as usize)
                            .is_some_and(|&m| m != OPEN);
                    if !in_dust {
                        continue;
                    }
                    let m = (s.mass / M_IGN).clamp(0.0, 1.0);
                    let radius = ease::lerp(EMBEDDED_HALO.0, EMBEDDED_HALO.1, m);
                    let peak = ease::lerp(EMBEDDED_PEAK.0, EMBEDDED_PEAK.1, m);
                    (radius, Ramp::Ember, peak, true)
                }
                _ => {
                    let b = s.brightness();
                    if b <= 0.3 {
                        continue;
                    }
                    (SHINING_HALO * b, Ramp::Ice, b * GLARE, false)
                }
            };
            // A Gaussian (σ = half the radius, cut at two σ) for a soft infrared source;
            // a quadratic fall to zero for the glare.
            let glow = |f: f64| {
                let profile = if embedded {
                    (-2.0 * f * f).exp()
                } else {
                    (1.0 - f).max(0.0).powi(2)
                };
                ink::ramp(ramp, peak * profile)
            };
            let n = reach.ceil() as i32;
            for dy in -n..=n {
                for dx in -n..=n {
                    let (x, y) = (px.floor() as i32 + dx, py.floor() as i32 + dy);
                    let r = (f64::from(x) + 0.5 - px).hypot(f64::from(y) + 0.5 - py);
                    let Some(under) = self.pixels.get(x, y).filter(|_| r <= reach) else {
                        continue;
                    };
                    let ink = glow(r / reach);
                    if f64::from(ink.level) >= ink::MIN_VISIBLE_LEVEL {
                        self.pixels.set(x, y, raster::stronger(ink, under));
                    }
                }
            }
        }
    }

    /// Continuous cell coordinates of a point `(x, y)` in rows (cell `i` spans
    /// `[i, i + 1)`, and a star at row `y` sits in cell `round(y)`).
    fn to_cells(x: f64, y: f64) -> (f64, f64) {
        (x * ASPECT + 0.5, y + 0.5)
    }

    /// Gas, jets and stars as braille dots over the nebula, and the shining stars' cells.
    /// Each gas parcel is a streak back along its last `STREAK` s of spiralling infall,
    /// so the flow reads as it speeds up towards its star.
    fn rasterise(&mut self) {
        self.dots.clear();
        for g in &self.gas {
            let Some(s) = self.stars.get(g.star) else {
                continue;
            };
            let close = (1.0 - g.r / self.r_feed).clamp(0.0, 1.0);
            let fade_in = (g.age / 0.4).min(1.0);
            let level = (130.0 + 125.0 * close) * fade_in;
            if level < ink::MIN_VISIBLE_LEVEL {
                continue;
            }
            // Where the parcel was `STREAK` s ago, at its current speeds.
            let back = Gas {
                r: g.r + g.vr * STREAK,
                theta: g.theta - (g.l / (g.r * g.r) * STREAK).clamp(-0.5, 0.5),
                ..*g
            };
            let ink = Ink::new(Role::Particle, ink::level(level));
            let ((x0, y0), (x1, y1)) = (back.pos(s), g.pos(s));
            let (b0, b1) = (Self::to_cells(x0, y0), Self::to_cells(x1, y1));
            let ((dx0, dy0), (dx1, dy1)) =
                (Dots::from_cells(b0.0, b0.1), Dots::from_cells(b1.0, b1.1));
            self.dots.line(dx0, dy0, dx1, dy1, ink);
        }
        for j in &self.jets {
            let f = 1.0 - j.age / j.life;
            let ink = Ink::new(Role::Accent, ink::level(130.0 + 125.0 * f));
            let (cx, cy) = Self::to_cells(j.x, j.y);
            let (x1, y1) = Dots::from_cells(cx, cy);
            if j.age < KNOT_AGE * j.life {
                // A thin streak along the flight direction.
                let (bx, by) = Self::to_cells(j.x - j.vx * JET_STREAK, j.y - j.vy * JET_STREAK);
                let (x0, y0) = Dots::from_cells(bx, by);
                self.dots.line(x0, y0, x1, y1, ink);
            } else {
                self.dots.plot(x1, y1, ink);
            }
        }
        self.n_shining = 0;
        for s in &self.stars {
            let (cx, cy) = Self::to_cells(s.x, s.y);
            let (dx, dy) = Dots::from_cells(cx, cy);
            match s.phase {
                // An embedded star is its pixel glow alone (`star_glows`).
                Phase::Accreting => {}
                _ => {
                    let b = s.brightness();
                    if b <= 0.3 {
                        if b > 0.02 {
                            self.dots.plot(dx, dy, ink::blackbody(SHINING_K));
                        }
                        continue;
                    }
                    // Brief flash at ignition: longer diffraction spikes for ~2 s.
                    let flash = match s.phase {
                        Phase::Shining(a) => (-(a - IGNITION_RAMP).max(0.0) / 1.2).exp() * b,
                        _ => 0.0,
                    };
                    let ink = ink::blackbody(SHINING_K);
                    let arm = f64::from(Dots::CELL_W) * (1.0 + 3.0 * flash) * b;
                    let varm = f64::from(Dots::CELL_H) * if flash > 0.3 { 2.0 } else { 1.0 } * b;
                    self.dots.line(dx - arm, dy, dx + arm, dy, ink);
                    self.dots.line(dx, dy - varm, dx, dy + varm, ink);
                    self.shining[self.n_shining] = (cx.floor() as i32, cy.floor() as i32);
                    self.n_shining += 1;
                }
            }
        }
    }
}

/// `Σ wᵢ sin(Aᵢ(x) + Bᵢ(y)) / σ` from a column's and a row's `(sin, cos)` tables.
#[inline]
fn nebula_z(
    modes: &[Mode],
    inv_sigma: f64,
    c: &[(f64, f64); MODES],
    r: &[(f64, f64); MODES],
) -> f64 {
    let mut f = 0.0;
    for (i, m) in modes.iter().enumerate() {
        f += m.weight * (c[i].0 * r[i].1 + c[i].1 * r[i].0);
    }
    f * inv_sigma
}

/// ASCII stand-in for a nebula pixel pair: a density char by the brighter pixel's level;
/// a cell of unlit pillar dust (`in_dust`, drawn in the Hα ramp) is left blank, so the
/// pillars stand as silhouettes inside their lit rims.
fn ascii_pixel(top: Ink, bottom: Ink, in_dust: bool) -> Option<(char, Ink)> {
    let ink = raster::stronger(top, bottom);
    let dust = in_dust && ink.role == Role::Ramp(Ramp::HAlpha);
    (ink.role != Role::Background && !dust).then(|| {
        (
            ASCII_RAMP[usize::from(ink.level) * ASCII_RAMP.len() / 256],
            ink,
        )
    })
}

impl Generator for StellarNursery {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.modes.is_empty() {
            self.init_sky(&mut ctx.rng);
        }
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        (self.w, self.h) = (w, h);
        self.col.clear();
        self.col.resize(w, [(0.0, 1.0); MODES]);
        self.row.clear();
        self.row.resize(2 * h, [(0.0, 1.0); MODES]);
        // u = (x/w − ½)·s·sin θ + (½ − y/h)·cos θ: the position towards the source.
        let (sx, sy) = to_source(self.source);
        let (wf, phf) = ((w as f64).max(1.0), (2 * h) as f64);
        self.col_u.clear();
        self.col_u
            .extend((0..w).map(|x| (x as f64 / wf - 0.5) * sx));
        self.row_u.clear();
        self.row_u
            .extend((0..2 * h).map(|py| (py as f64 / phf.max(1.0) - 0.5) * sy));
        // The cluster just off the top corner on its side; a Gaussian is separable.
        let (gx, gy) = (0.5 * wf * (1.0 + 1.1 * self.source), -0.1 * phf);
        let r = SOURCE_GLOW_R * wf.max(phf);
        let gauss = |d: f64| (-0.5 * (d / r).powi(2)).exp();
        self.col_g.clear();
        self.col_g
            .extend((0..w).map(|x| SOURCE_GLOW * gauss(x as f64 - gx)));
        self.row_g.clear();
        self.row_g
            .extend((0..2 * h).map(|py| gauss(py as f64 - gy)));
        self.pixels.resize(size);
        self.dots.resize(size);
        self.build_mask();
        self.refresh_tables();

        let (wr, hr) = (w as f64 / ASPECT, h as f64);
        let span = wr.min(hr).max(1.0);
        self.r_feed = (0.45 * span).clamp(R_SPAWN_MIN + 1.0, 18.0);
        self.r_hii = (0.3 * span).clamp(2.0, 9.0);
        // Free fall from rest: t_ff = (π/2)·√(r³ / 2gm) (ε ≪ r) ⇒ gm fixes the timescale.
        let half_pi = std::f64::consts::FRAC_PI_2;
        self.gm = half_pi * half_pi * self.r_feed.powi(3) / (2.0 * T_FF * T_FF);

        let cap = ((GAS_PER_AREA * size.area() as f64) as usize).clamp(8, GAS_MAX);
        self.gas.set_capacity(cap);
        self.gas.clear();
        let jet_cap = (size.area() / 60).clamp(4, JET_MAX);
        self.jets.set_capacity(jet_cap);
        self.jets.clear();
        self.spawn_acc = 0.0;

        self.stars.clear();
        let n = ctx.rng.random_range(3..=MAX_STARS);
        for k in 0..n {
            // Some protostars sit in the pillar heads (evaporating gaseous globules).
            let (x, y) = match self.tips.get(k) {
                Some(&tip) if k + 1 < n && self.in_bounds(tip.0, tip.1) => tip,
                _ => self.place_star(&mut ctx.rng, None),
            };
            let mass = ctx.rng.random_range(0.0..=0.8) * M_IGN;
            let mut s = Protostar::new(x, y, &mut ctx.rng, 0, mass);
            // One star is already shining, so the life cycles are staggered from the start.
            if k + 1 == n {
                s.mass = M_IGN;
                s.phase = Phase::Shining(ctx.rng.random_range(2.0..=10.0));
            }
            self.stars.push(s);
        }
        // Each hub keeps ≈ cap/n parcels in flight, each arriving after ≈ T_ARRIVE, so it
        // accretes ≈ (cap/n)·m/T_ARRIVE per second; m gives an ACC_TIME build-up at any size.
        self.gas_mass = M_IGN * T_ARRIVE * n as f64 / (ACC_TIME * cap as f64);
        self.shade();
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.w == 0 || self.h == 0 {
            return;
        }
        let n = (dt / MAX_STEP).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            self.substep(h, &mut ctx.rng);
        }
        self.refresh_tables();
        self.shade();
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        if self.w == 0 || self.h == 0 {
            return;
        }
        // A cell whose two pixels are both plain (unlit) dust.
        let dust = |x: i32, py: i32| self.mask[py as usize * self.w + x as usize] == DUST;
        self.pixels.blit(canvas, |x, y, top, bottom| {
            ascii_pixel(top, bottom, dust(x, 2 * y) && dust(x, 2 * y + 1))
        });
        self.dots.blit_over(canvas, &self.pixels);
        let unicode = canvas.mode() == GlyphMode::Unicode;
        let ink = ink::blackbody(SHINING_K);
        for &(x, y) in &self.shining[..self.n_shining] {
            let bg = if unicode {
                self.pixels.cell_bg(x, y)
            } else {
                Ink::BLANK
            };
            canvas.put_bg(x, y, STAR, ink, bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn scene(seed: u64, w: u16, h: u16) -> (StellarNursery, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = StellarNursery::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn nur_a1_infall_step_never_increases_distance() {
        for gm in [0.0, 1.0, 50.0, 500.0, 5000.0] {
            for r0 in [0.81, 1.0, 3.0, 10.0, 40.0] {
                for dt in [1e-4, 1.0 / 120.0, 1.0 / 60.0, 0.25] {
                    let (mut r, mut vr) = (r0, 0.0);
                    for _ in 0..200 {
                        let (r1, vr1) = infall_step(r, vr, gm, dt);
                        assert!(r1 <= r, "gm {gm} r {r} -> {r1}");
                        assert!(vr1 >= vr, "inward speed must not drop");
                        assert!(r1 >= 0.0 && r1.is_finite());
                        (r, vr) = (r1, vr1);
                    }
                }
            }
        }
    }

    #[test]
    fn nur_a1_infall_matches_softened_gravity() {
        // One kick of dt: Δv = gm/(r²+ε²)·dt exactly, then drift by the new speed.
        let (r1, vr1) = infall_step(5.0, 1.0, 26.0, 0.01);
        let a = 26.0 / (25.0 + EPS * EPS);
        assert!((vr1 - (1.0 + a * 0.01)).abs() < 1e-12);
        assert!((r1 - (5.0 - vr1 * 0.01)).abs() < 1e-12);
        assert!((gravity_accel(26.0, 5.0) - a).abs() < 1e-12);
    }

    #[test]
    fn nur_a1_gas_distance_to_target_is_non_increasing_in_the_scene() {
        for seed in [1, 7] {
            let (mut g, mut ctx) = scene(seed, 160, 48);
            let mut seen = 0;
            for _ in 0..900 {
                let before: HashMap<u32, (usize, u32, f64)> = g
                    .gas
                    .iter()
                    .map(|p| {
                        let (x, y) = p.pos(&g.stars[p.star]);
                        let s = &g.stars[p.star];
                        (p.id, (p.star, p.generation, (x - s.x).hypot(y - s.y)))
                    })
                    .collect();
                g.update(SIM_DT, &mut ctx);
                for p in &g.gas {
                    if let Some(&(star, generation, d0)) = before.get(&p.id) {
                        assert_eq!((star, generation), (p.star, p.generation), "target changed");
                        let s = &g.stars[p.star];
                        let (x, y) = p.pos(s);
                        let d1 = (x - s.x).hypot(y - s.y);
                        assert!(d1 <= d0 + 1e-9, "seed {seed}: {d0} -> {d1}");
                        assert!(d1 > R_ABS, "unabsorbed particle inside R_ABS");
                        seen += 1;
                    }
                }
            }
            assert!(seen > 10_000, "too few particles tracked: {seen}");
        }
    }

    #[test]
    fn nur_a2_accrete_ignites_exactly_when_mass_first_reaches_m_ign() {
        let mut rng = Ctx::new(3, Size::new(10, 10)).rng;
        let mut s = Protostar::new(0.0, 0.0, &mut rng, 0, 0.0);
        let mut ignitions = 0;
        let mut last = s.mass;
        for k in 1..=40 {
            let crossed_now = last < M_IGN && last + 0.07 >= M_IGN;
            let ignited = s.accrete(0.07);
            assert!(s.mass >= last, "mass decreased");
            assert_eq!(
                ignited, crossed_now,
                "step {k}: ignition must coincide with crossing"
            );
            if ignited {
                ignitions += 1;
                assert_eq!(s.phase, Phase::Shining(0.0));
            }
            last = s.mass;
        }
        assert_eq!(ignitions, 1);
        // Negative or NaN mass is ignored: never decreases, never ignites spuriously.
        let mut t = Protostar::new(0.0, 0.0, &mut rng, 0, 0.5);
        assert!(!t.accrete(-1.0));
        assert!(!t.accrete(f64::NAN));
        assert_eq!(t.mass, 0.5);
        assert!(t.accrete(0.5), "exactly M_IGN ignites");
    }

    #[test]
    fn nur_a2_scene_mass_is_monotone_and_ignition_is_at_threshold() {
        for seed in [2, 11] {
            let (mut g, mut ctx) = scene(seed, 160, 48);
            let mut ignitions = 0;
            for _ in 0..(60 * 60) {
                let before = g.stars.clone();
                g.update(SIM_DT, &mut ctx);
                for (a, b) in before.iter().zip(&g.stars) {
                    if a.generation != b.generation {
                        assert_eq!(b.phase, Phase::Accreting, "replacement starts embedded");
                        assert!(b.mass < M_IGN);
                        continue;
                    }
                    assert!(b.mass >= a.mass, "seed {seed}: mass decreased");
                    match (a.phase, b.phase) {
                        (Phase::Accreting, Phase::Accreting) => assert!(b.mass < M_IGN),
                        (Phase::Accreting, Phase::Shining(_)) => {
                            assert!(a.mass < M_IGN && b.mass >= M_IGN);
                            ignitions += 1;
                        }
                        (Phase::Accreting, Phase::Fading(_)) => panic!("skipped shining"),
                        _ => {}
                    }
                }
            }
            assert!(
                ignitions >= 3,
                "seed {seed}: only {ignitions} ignitions in 60 s"
            );
        }
    }

    #[test]
    fn nur_a3_pools_never_exceed_capacity() {
        let (mut g, mut ctx) = scene(5, 200, 50);
        let sizes = [
            (200, 50),
            (400, 100),
            (30, 8),
            (1, 1),
            (260, 80),
            (0, 0),
            (120, 40),
        ];
        for (k, &(w, h)) in sizes.iter().enumerate() {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            let gas_cap = ((GAS_PER_AREA * size.area() as f64) as usize).clamp(8, GAS_MAX);
            assert_eq!(g.gas.capacity(), gas_cap);
            assert!(g.jets.capacity() <= JET_MAX);
            for i in 0..600 {
                let dt = if (i + k) % 7 == 0 { 0.25 } else { SIM_DT };
                g.update(dt, &mut ctx);
                assert!(g.gas.len() <= g.gas.capacity() && g.gas.capacity() <= GAS_MAX);
                assert!(g.jets.len() <= g.jets.capacity());
            }
        }
    }

    #[test]
    fn nur_ignition_ramps_over_one_second_then_fades_and_is_replaced() {
        let (mut g, mut ctx) = scene(4, 120, 40);
        g.stars[0].mass = M_IGN - 1e-9;
        assert!(g.stars[0].accrete(1e-9));
        let (x0, gen0) = (g.stars[0].x, g.stars[0].generation);
        let mut b = Vec::new();
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            b.push(g.stars[0].brightness());
        }
        assert!(b.windows(2).all(|w| w[1] >= w[0]), "ramp must be monotone");
        assert!(b[29] > 0.2 && b[29] < 0.8);
        assert!((b[59] - 1.0).abs() < 1e-9);
        assert!(!g.jets.is_empty(), "an ignited star launches jets");
        for _ in 0..((SHINE + FADE) * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let s = g.stars[0];
        assert_eq!(s.generation, gen0 + 1, "replaced after shining and fading");
        assert_eq!(s.phase, Phase::Accreting);
        let _ = x0;
    }

    #[test]
    fn nur_pillars_occlude_the_nebula() {
        let (g, _) = scene(8, 160, 48);
        let dust = g.mask.iter().filter(|&&m| m == DUST).count();
        assert!(dust > 100, "pillars too small: {dust}");
        // Pixels (continuous coordinates) under a star's glow, which lies over the dust.
        let glowing = |x: f64, y: f64| {
            g.stars.iter().any(|s| {
                let (cx, cy) = StellarNursery::to_cells(s.x, s.y);
                (x - cx).hypot(y - 2.0 * cy) <= EMBEDDED_HALO.1.max(SHINING_HALO) + 1.0
            })
        };
        for (i, &m) in g.mask.iter().enumerate() {
            let (x, y) = ((i % 160) as i32, (i / 160) as i32);
            if m == DUST && !glowing(f64::from(x) + 0.5, f64::from(y) + 0.5) {
                let ink = g.pixels.get(x, y).unwrap();
                assert_eq!(ink.role, Role::Ramp(Ramp::HAlpha), "pixel {i}");
                assert!(
                    f64::from(ink.level) <= DUST_LEVEL.1 + LIMB + 0.5,
                    "lit dust at {i}"
                );
            }
        }
    }

    #[test]
    fn nur_a4_rims_are_lit_from_the_source_side_only() {
        let d = to_source(1.0);
        // The mask's gradient points into the dust. A face whose outward normal points at
        // the source is fully lit; one turned away, or a flat mask, is dark.
        assert!(rim_light((-d.0, -d.1), d) > 0.99);
        assert_eq!(rim_light(d, d), 0.0);
        assert_eq!(rim_light((0.0, 0.0), d), 0.0);
        // Lighting turns on smoothly with the facing angle, with no cut-off.
        let lit_at = |a: f64| {
            let (s, c) = (d.1.atan2(d.0) + a).sin_cos_det();
            rim_light((-c, -s), d)
        };
        let curve: Vec<f64> = (0..=90)
            .map(|deg| lit_at(f64::from(deg).to_radians()))
            .collect();
        assert!(
            curve.windows(2).all(|p| p[1] <= p[0] && p[0] - p[1] < 0.06),
            "{curve:?}"
        );
        for seed in [3, 8] {
            let (g, _) = scene(seed, 160, 48);
            let (w, h) = (160_i32, 96_i32);
            let d = to_source(g.source);
            let open = |x: i32, y: i32| {
                (0..w).contains(&x) && (0..h).contains(&y) && g.mask[(y * w + x) as usize] == OPEN
            };
            let (mut lit, mut facing) = (0, 0);
            for y in 0..h {
                for x in 0..w {
                    let m = g.mask[(y * w + x) as usize];
                    if m < RIM + 80 {
                        continue;
                    }
                    lit += 1;
                    let reach = -RIM_REACH..=RIM_REACH;
                    let sees_source = reach.clone().any(|dy| {
                        reach.clone().any(|dx| {
                            open(x + dx, y + dy) && f64::from(dx) * d.0 + f64::from(dy) * d.1 > 0.0
                        })
                    });
                    facing += usize::from(sees_source);
                }
            }
            assert!(lit > 20, "seed {seed}: only {lit} lit rim pixels");
            assert!(
                facing * 100 >= lit * 85,
                "seed {seed}: {facing}/{lit} face the source"
            );
            // Continuous rims: down a lit side, the surface pixel of one row is lit much
            // like the next row's, not a zipper of lit and dark pixels.
            let side = g.source as i32;
            // The outermost dust pixels of each row on the source's side of a pillar.
            let surface = |y: i32| -> Vec<i32> {
                (0..w)
                    .filter(|&x| g.mask[(y * w + x) as usize] != OPEN && open(x + side, y))
                    .collect()
            };
            // The light may turn with the outline's curves (the mean step stays small),
            // but hardly ever flips from lit to dark between two rows.
            let (mut pairs, mut jumps, mut total) = (0, 0, 0);
            for y in 0..h - 1 {
                let next = surface(y + 1);
                for x in surface(y) {
                    let Some(&x2) = next.iter().find(|x2| (*x2 - x).abs() <= 1) else {
                        continue;
                    };
                    let (a, b) = (
                        g.mask[(y * w + x) as usize],
                        g.mask[((y + 1) * w + x2) as usize],
                    );
                    if a.max(b) < RIM + 120 {
                        continue;
                    }
                    pairs += 1;
                    total += usize::from(a.abs_diff(b));
                    jumps += usize::from(a.abs_diff(b) > 90);
                }
            }
            assert!(pairs > 20, "seed {seed}: {pairs} lit surface pairs");
            assert!(
                total < 40 * pairs,
                "seed {seed}: mean rim step {}",
                total / pairs
            );
            assert!(jumps * 8 <= pairs, "seed {seed}: {jumps}/{pairs} rim jumps");
        }
    }

    #[test]
    fn nur_a4_rims_run_unbroken_down_the_source_side_of_each_pillar() {
        // Below the head, a pillar's source-facing surface is a rim nearly all the way
        // down: the diffuse ionising field lights even grazing faces. Only the underside
        // of a lobe or knob, turned away from the source, is shadowed, and such shadows
        // come as a few runs, never the many short gaps of a zipper.
        for seed in [1, 3, 5, 8, 13] {
            let (g, _) = scene(seed, 160, 48);
            let (w, hf) = (160_usize, 48.0);
            let side = g.source as i32;
            for (p, &(_, tip_y)) in g.pillars.iter().zip(&g.tips) {
                let bottom = hf - 1.0;
                let tip_y = tip_y - 0.5;
                let body =
                    (2.0 * (tip_y + 0.3 * (bottom - tip_y))) as usize..(2.0 * bottom) as usize - 2;
                let (mut dark, mut gaps, mut rows, mut was_lit) = (0, 0, 0, true);
                for py in body {
                    let yf = (py as f64 - 0.5) / 2.0;
                    let s = ((yf - tip_y) / (bottom - tip_y)).clamp(0.0, 1.0);
                    let mut x = ((p.base + p.lean * (1.0 - s)) * w as f64) as i32;
                    let at = |x: i32| g.mask[py * w + x as usize];
                    if !(0..w as i32).contains(&x) || at(x) == OPEN {
                        continue;
                    }
                    // Walk towards the source to the pillar's last dust pixel.
                    while (0..w as i32).contains(&(x + side)) && at(x + side) != OPEN {
                        x += side;
                    }
                    if !(0..w as i32).contains(&(x + side)) {
                        continue;
                    }
                    rows += 1;
                    let lit = f64::from(at(x).saturating_sub(RIM)) / f64::from(255 - RIM);
                    let is_lit = at(x) >= RIM && lit >= RIM_MIN;
                    dark += usize::from(!is_lit);
                    gaps += usize::from(was_lit && !is_lit);
                    was_lit = is_lit;
                }
                assert!(rows > 10, "seed {seed}: only {rows} body rows");
                assert!(
                    dark * 3 <= rows && gaps <= 3,
                    "seed {seed}: {dark} of {rows} rows dark, in {gaps} gaps"
                );
            }
        }
    }

    #[test]
    fn nur_a7_pillars_have_broad_heads_necks_and_lobes() {
        for seed in 1..=12 {
            let (g, _) = scene(seed, 160, 48);
            for p in &g.pillars {
                let width = |s: f64| {
                    let (l, r) = p.half_widths(s, 8.0);
                    l + r
                };
                // A head at least 1.3× the neck below it.
                let neck = (15..=50)
                    .map(|k| width(f64::from(k) / 100.0))
                    .fold(f64::INFINITY, f64::min);
                assert!(width(0.0) >= 1.3 * neck, "seed {seed}: head vs neck");
                // Down the body the column swells and pinches: its width is not monotone.
                let ws: Vec<f64> = (25..=100).map(|k| width(f64::from(k) / 100.0)).collect();
                let turns = ws
                    .windows(3)
                    .filter(|t| (t[1] - t[0]) * (t[2] - t[1]) < 0.0)
                    .count();
                assert!(turns >= 1, "seed {seed}: a straight taper");
                // One or two knobs.
                assert!(p.knobs[0].2 > 0.0, "seed {seed}");
            }
        }
    }

    #[test]
    fn nur_a7_dust_is_textured_and_limb_brightened() {
        for seed in [3, 8] {
            let (g, _) = scene(seed, 160, 48);
            let (w, ph) = (160_usize, 96_usize);
            // Distance (Chebyshev, capped) from a dust pixel to a lit rim.
            let near_rim = |x: usize, y: usize, r: usize| {
                (y.saturating_sub(r)..=(y + r).min(ph - 1)).any(|ny| {
                    (x.saturating_sub(r)..=(x + r).min(w - 1))
                        .any(|nx| g.mask[ny * w + nx] > RIM + 120)
                })
            };
            let (mut deep, mut limb) = (Vec::new(), Vec::new());
            for y in 0..ph {
                for x in 0..w {
                    if g.mask[y * w + x] != DUST {
                        continue;
                    }
                    let level = f64::from(g.dust[y * w + x]);
                    if near_rim(x, y, 2) {
                        limb.push(level);
                    } else if !near_rim(x, y, 7) {
                        deep.push(level);
                    }
                }
            }
            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
            let distinct = |v: &[f64]| {
                let mut v: Vec<u8> = v.iter().map(|&l| l as u8).collect();
                v.sort_unstable();
                v.dedup();
                v.len()
            };
            assert!(deep.len() > 100 && limb.len() > 20, "seed {seed}");
            assert!(distinct(&deep) >= 12, "seed {seed}: flat dust");
            assert!(
                deep.iter().all(|&l| l <= DUST_LEVEL.1 + 0.5),
                "seed {seed}: deep dust too bright"
            );
            assert!(
                mean(&limb) > mean(&deep) + 5.0,
                "seed {seed}: limb {} vs deep {}",
                mean(&limb),
                mean(&deep)
            );
        }
    }

    #[test]
    fn nur_a7_embedded_protostars_glow_softly_and_only_inside_the_dust() {
        let mut inside = 0;
        for seed in [1, 2, 4, 6, 9] {
            let (mut g, mut ctx) = scene(seed, 120, 40);
            for step in 0..(8 * 60) {
                g.update(SIM_DT, &mut ctx);
                if step % 120 != 0 {
                    continue;
                }
                for s in g.stars.iter().filter(|s| s.is_accreting()) {
                    let (cx, cy) = StellarNursery::to_cells(s.x, s.y);
                    let (px, py) = (cx.floor() as i32, (2.0 * cy).floor() as i32);
                    let reach = EMBEDDED_HALO.1.ceil() as i32;
                    let mut levels = Vec::new();
                    for y in py - reach..=py + reach {
                        for x in px - reach..=px + reach {
                            if let Some(ink) = g.pixels.get(x, y)
                                && ink.role == Role::Ramp(Ramp::Ember)
                            {
                                levels.push(ink.level);
                            }
                        }
                    }
                    let in_dust = g.mask[py as usize * 120 + px as usize] != OPEN;
                    if !in_dust {
                        assert!(levels.is_empty(), "seed {seed}: a glow in the open");
                        continue;
                    }
                    inside += 1;
                    // Soft: several levels, not one flat block; dimmer than a lit rim.
                    levels.sort_unstable();
                    levels.dedup();
                    assert!(levels.len() >= 3, "seed {seed}: {levels:?}");
                    let top = f64::from(*levels.last().unwrap());
                    assert!(top <= 255.0 * EMBEDDED_PEAK.1 + 1.0 && top < RIM_LEVEL.1);
                }
            }
        }
        assert!(inside > 5, "only {inside} embedded glows seen");
    }

    #[test]
    fn nur_a5_pillar_tips_lean_towards_the_source() {
        for seed in [1, 2, 3, 4, 5, 6] {
            let (g, _) = scene(seed, 160, 48);
            for (p, tip) in g.pillars.iter().zip(&g.tips) {
                let base = p.base * 160.0 / ASPECT;
                assert_eq!((tip.0 - base).signum(), g.source, "seed {seed}");
                assert!(p.lean.abs() >= LEAN.0);
            }
        }
    }

    #[test]
    fn nur_a6_halpha_and_oiii_with_more_oiii_near_the_source() {
        let (g, _) = scene(8, 160, 48);
        let (mut near, mut far) = ([0usize; 2], [0usize; 2]);
        for y in 0..96 {
            for x in 0..160 {
                if g.mask[(y * 160 + x) as usize] != OPEN {
                    continue;
                }
                let ink = g.pixels.get(x, y).unwrap();
                let oiii = match ink.role {
                    Role::Background => continue,
                    Role::Ramp(Ramp::Ice) => 1,
                    Role::Ramp(Ramp::HAlpha) => 0,
                    other => panic!("{other:?} in the nebula"),
                };
                let u = g.col_u[x as usize] + g.row_u[y as usize];
                if u > 0.0 {
                    near[oiii] += 1;
                } else {
                    far[oiii] += 1;
                }
            }
        }
        let frac = |c: [usize; 2]| c[1] as f64 / (c[0] + c[1]).max(1) as f64;
        assert!(
            frac(near) > frac(far) + 0.2,
            "[O III] near {} vs far {}",
            frac(near),
            frac(far)
        );
    }

    #[test]
    fn nur_a6_bright_gas_rolls_off_below_white_and_keeps_its_texture() {
        // A shoulder, not a clip: [O III] peaks at OIII_PEAK and bright filaments keep
        // distinct levels instead of posterising to one flat white-cyan.
        for seed in [8, 21] {
            let (mut g, mut ctx) = scene(seed, 120, 40);
            for _ in 0..(20 * 60) {
                g.update(SIM_DT, &mut ctx);
            }
            let mut levels = Vec::new();
            for y in 0..80 {
                for x in 0..120 {
                    let ink = g.pixels.get(x, y).unwrap();
                    if ink.role == Role::Ramp(Ramp::Ice) {
                        levels.push(ink.level);
                    }
                }
            }
            assert!(
                levels.len() > 200,
                "seed {seed}: {} [O III] pixels",
                levels.len()
            );
            let top = *levels.iter().max().unwrap();
            assert!(
                f64::from(top) <= 255.0 * OIII_PEAK + 1.0,
                "seed {seed}: [O III] at {top}"
            );
            let flat = levels.iter().filter(|&&l| l + 4 >= top).count();
            assert!(
                flat * 20 < levels.len(),
                "seed {seed}: {flat} of {} [O III] pixels within 4 levels of the top",
                levels.len()
            );
        }
    }
}
