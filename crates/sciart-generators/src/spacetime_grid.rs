//! The rubber-sheet picture of a binary black-hole merger: a perspective braille mesh
//! dimpled by an inspiralling pair, quadrupole gravitational waves spreading outward at a
//! finite speed, a plunge, a single remnant ringing down, and a strain inset drawing the
//! chirp (SPEC-002 `spacetime_grid`).
//!
//! World units: the sheet spans `y ∈ [-1, 1]` (far edge at `+1`) and `x ∈ [-Lx, Lx]`, with
//! `Lx` widened to fill wide terminals; `z` points up, so wells are negative.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::{draw, ink, title};

pub static META: Meta = Meta {
    name: "spacetime_grid",
    category: Category::DeepCosmos,
    description: "A rubber-sheet spacetime rippled by a binary black hole that spirals in and merges",
    title: "Spacetime waves · orbits shrink",
    captions: &[MERGER, RINGDOWN],
};

const MERGER: &str = "merger";
const RINGDOWN: &str = "ringdown";

pub fn build() -> Box<dyn Generator> {
    Box::new(SpacetimeGrid::default())
}

/// Depth scale of the softened potential `z = -K Σ m / √(d² + ε²)`: deep enough that the
/// mesh visibly funnels into each mass (SPT-A9).
const WELL_K: f64 = 0.16;
/// Constant lift (per unit mass) that puts the sheet near `z = 0` around `r ≈ 1`, so the
/// camera fit sees the sheet rather than the far-field bowl.
const RIM_LIFT: f64 = WELL_K / 1.1;
/// Plummer softening: keeps the well bottom finite (and the mesh on screen).
const SOFTENING: f64 = 0.16;
/// The lowest the lifted sheet gets, the remnant's well bottom, for the camera fit:
/// `−(K/ε)·0.95 (m₁ + m₂) + RIM_LIFT (m₁ + m₂)` (the near zone carries no wave).
const DEEPEST: f64 = {
    let m = Binary::DEFAULT.m1 + Binary::DEFAULT.m2;
    -(WELL_K / SOFTENING) * (1.0 - RADIATED) * m + RIM_LIFT * m
};
/// Gravitational-wave speed `c` in sheet units per simulated second. Artistic: a real
/// binary radiates at wavelengths far longer than its orbit; here `λ = πc/Ω` stays above
/// the mesh spacing from the first orbit to the ringdown.
const WAVE_SPEED: f64 = 0.35;
/// Wave amplitude per unit strain (sheet units).
const WAVE_AMP: f64 = 0.08;
/// Near-zone taper radius: the wave term fades inside it, where the wells dominate.
const TAPER: f64 = 0.5;

/// The merger: the inspiral ends at `Q_MERGE · a₀`, the plunge takes `PLUNGE` s while the
/// orbital speed doubles, the strain peaks at `PEAK_GAIN` times its inspiral end value,
/// then the remnant rings down with e-folding time `TAU_RD` at `RD_SPEED` times the peak
/// orbital speed. `RADIATED` of the mass leaves as waves (GW150914: ~5 %).
const Q_MERGE: f64 = 0.5;
const PLUNGE: f64 = 0.5;
const PEAK_GAIN: f64 = 2.2;
const TAU_RD: f64 = 0.35;
const RD_SPEED: f64 = 1.2;
const RADIATED: f64 = 0.05;
/// Quiet time after the merger while the burst runs off the sheet, the new binary's
/// strain fade-in, the caption's ringdown window and the dip to dark at the cut (s).
const QUIET: f64 = 5.5;
const SWITCH_ON: f64 = 2.0;
const RINGDOWN_SHOWN: f64 = 2.5;
const DIP: f64 = 0.5;
/// The `merger` caption opens this long before the plunge and stays this long after
/// coalescence, so it can be read (SPT-A7).
const MERGER_LEAD: f64 = 0.5;
const MERGER_TAIL: f64 = 0.5;
/// The merger flash: e-folding time after coalescence (s) and the mesh brightening at
/// its peak (SPT-A8).
const FLASH_TAU: f64 = 0.3;
const FLASH_GAIN: f64 = 0.6;

/// Camera pitch below the horizon and distance from the sheet centre.
const PITCH: f64 = 55.0 * PI / 180.0;
const CAM_DIST: f64 = 3.4;
/// Slow yaw sway: amplitude (radians) and period (s). Small enough that the `+y` edge
/// always stays the far edge.
const YAW_AMP: f64 = 0.05;
const YAW_PERIOD: f64 = 47.0;

/// Mesh line spacing targets (cells) and samples per mesh edge along each line.
const COLS_PER_LINE: u16 = 7;
const ROWS_PER_LINE: f64 = 2.8;
const SUB_X: usize = 3;
const SUB_Y: usize = 3;
const MAX_NX: usize = 90;
const MAX_NY: usize = 48;

/// Mesh inks, `Seq` (inferno-like in astro): a dim purple resting level, raised where
/// the sheet dips into a well; wave crests climb towards magenta and orange by
/// `CREST_LIFT` (troughs dim by `TROUGH_DIM`), and the wave's energy density `∝ h²` adds
/// up to `BURST_LIFT`, so the chirp and the merger burst travel out as bands of light
/// (SPT-A10). `CREST_FULL` is the crest height `crest · envn` that counts as full.
/// Nearness only shades the level by up to `NEAR_SHADE`, and the foreground edge fades.
const MESH_ROLE: Role = Role::Seq;
const REST: f64 = 60.0;
const WELL_GLOW: f64 = 45.0;
const TROUGH_DIM: f64 = 0.3;
const CREST_LIFT: f64 = 110.0;
const CREST_FULL: f64 = 0.5;
const BURST_LIFT: f64 = 220.0;
const NEAR_SHADE: f64 = 0.3;
const EDGE_FADE: f64 = 0.55;
/// The mesh's brightest level, and its cap near a black hole: two-thirds of the rim's
/// 255 within `NEAR_HOLE.0` moat radii, easing up to `MESH_MAX` by `NEAR_HOLE.1`, so the
/// rims win in their funnels in every theme (SPT-A9).
const MESH_MAX: f64 = 210.0;
const NEAR_MAX: f64 = 170.0;
const NEAR_HOLE: (f64, f64) = (2.5, 3.5);

/// The masses: horizon-like radius per cube root of mass (sheet units; at least
/// `BODY_MIN` dots, so the dark core shows), the dark moat round the rim that the mesh
/// and trails keep out of (in disc radii), and orbit trail length (s) and samples.
const BODY_R: f64 = 0.12;
const BODY_MIN: f64 = 2.5;
const MOAT: f64 = 1.4;
const TRAIL: f64 = 1.0;
const TRAIL_N: usize = 16;

/// Strain inset: seconds shown, its size in cells, and the smallest canvas that has one.
const INSET_WINDOW: f64 = 4.0;
const INSET_ROWS: u16 = 3;
const INSET_MIN: (u16, u16) = (48, 16);
/// The strain trace's level runs from `TRACE_LEVELS.0` (quiet) to `.1` at the merger peak,
/// so the chirp brightens as it grows.
const TRACE_LEVELS: (f64, f64) = (110.0, 255.0);
const INSET_AXIS: Ink = Ink::new(Role::Ramp(Ramp::Aurora), 60);
const INSET_LABEL: &str = "strain";

/// Blackbody temperatures of the heavier and lighter mass's rim, and of the remnant's
/// (K): a hot rim round a dark core, graded by a cooler halo ring `HALO_GAP` dots out (in
/// braille cells of its own, which show one ink each) at `HALO_K` of the rim's
/// temperature, which warms and brightens with the merger flash. They only
/// mark the black holes, which emit no light (stylised, SPEC-002).
const HOT: [f64; 2] = [30_000.0, 25_000.0];
const REMNANT_K: f64 = 30_000.0;
const HALO_K: f64 = 0.15;
const HALO_GAP: f64 = 2.5;

/// A circular binary, optionally on a quadrupole-driven inspiral (Peters 1964, Newtonian
/// chirp): `a(τ) = a₀(1 − τ/τc)^¼`, hence by Kepler `Ω = Ω₀(1 − τ/τc)^-⅜` and
/// `Φ(τ) = Φ₀ + (8/5)Ω₀τc[1 − (1 − τ/τc)^⅝]`. Each cycle runs inspiral → plunge →
/// remnant ringdown → quiet, then restarts (SPEC-002).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Binary {
    pub m1: f64,
    pub m2: f64,
    pub a0: f64,
    pub omega0: f64,
    pub tau_c: f64,
    pub phase0: f64,
    pub inspiral: bool,
}

/// The source at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Source {
    /// Separation (0 once merged) and orbital phase.
    pub a: f64,
    pub phase: f64,
    /// Strain envelope (1 at the start of the inspiral) and pattern phase: the wave is
    /// `S cos 2(φ − Ψ)`.
    pub strain: f64,
    pub pattern: f64,
    /// `dΨ/dt`: the orbital speed, then the ringdown pattern speed.
    pub speed: f64,
    /// Mass still in the pair (drops by `RADIATED` through the plunge), and whether the
    /// pair has merged into one remnant.
    pub mass_scale: f64,
    pub merged: bool,
    /// Seconds into the cycle.
    pub tau: f64,
}

impl Binary {
    pub(crate) const DEFAULT: Binary = Binary {
        m1: 1.0,
        m2: 0.65,
        a0: 0.55,
        omega0: TAU / 4.0,
        tau_c: 15.0,
        phase0: 0.0,
        inspiral: true,
    };

    /// Inspiral duration (to `a = Q_MERGE a₀`).
    fn inspiral_time(&self) -> f64 {
        self.tau_c * (1.0 - Q_MERGE.powi(4))
    }

    /// Seconds from cycle start to coalescence.
    fn merge_time(&self) -> f64 {
        self.inspiral_time() + PLUNGE
    }

    fn cycle(&self) -> f64 {
        self.merge_time() + QUIET
    }

    /// Orbital speed at the end of the inspiral.
    fn omega_merge(&self) -> f64 {
        self.omega0 * Q_MERGE.powf(-1.5)
    }

    /// The source at time `t` (any sign; the cycle repeats).
    pub(crate) fn state(&self, t: f64) -> Source {
        if !self.inspiral {
            let phase = self.phase0 + self.omega0 * t;
            return Source {
                a: self.a0,
                phase,
                strain: 1.0,
                pattern: phase,
                speed: self.omega0,
                mass_scale: 1.0,
                merged: false,
                tau: t,
            };
        }
        let cycle = self.cycle();
        let k = (t / cycle).floor();
        let tau = (t - k * cycle).clamp(0.0, cycle);
        // Each cycle's binary starts at a new angle.
        let phase0 = self.phase0 + 1.3 * k;
        let chirp = 1.6 * self.omega0 * self.tau_c;
        let switch_on = ease::smoothstep(tau / SWITCH_ON);
        let (t_i, t_m) = (self.inspiral_time(), self.merge_time());
        let (w_m, phase_m) = (
            self.omega_merge(),
            phase0 + chirp * (1.0 - Q_MERGE.powf(2.5)),
        );
        if tau <= t_i {
            let u = 1.0 - tau / self.tau_c;
            let (s2, s4) = (u.sqrt(), u.sqrt().sqrt());
            // u^⅝ = u^½ · u^⅛
            let phase = phase0 + chirp * (1.0 - s2 * s4.sqrt());
            return Source {
                a: self.a0 * s4,
                phase,
                strain: switch_on / s4,
                pattern: phase,
                speed: self.omega0 / (s4 * s4.sqrt()),
                mass_scale: 1.0,
                merged: false,
                tau,
            };
        }
        let strain_m = switch_on / Q_MERGE;
        if tau < t_m {
            // Plunge: a = a_m (1 − s²) → 0, Ω = Ω_m (1 + s).
            let s = (tau - t_i) / PLUNGE;
            let phase = phase_m + w_m * PLUNGE * (s + 0.5 * s * s);
            return Source {
                a: self.a0 * Q_MERGE * (1.0 - s * s),
                phase,
                strain: strain_m * (1.0 + (PEAK_GAIN - 1.0) * s * s),
                pattern: phase,
                speed: w_m * (1.0 + s),
                mass_scale: 1.0 - RADIATED * s,
                merged: false,
                tau,
            };
        }
        let since = tau - t_m;
        let phase = phase_m + 1.5 * w_m * PLUNGE;
        Source {
            a: 0.0,
            phase,
            strain: strain_m * PEAK_GAIN * (-since / TAU_RD).exp(),
            pattern: phase + RD_SPEED * 2.0 * w_m * since,
            speed: RD_SPEED * 2.0 * w_m,
            mass_scale: 1.0 - RADIATED,
            merged: true,
            tau,
        }
    }

    /// Mass positions about the fixed centre of mass at the origin (SPT-A2).
    pub(crate) fn positions(&self, t: f64) -> [(f64, f64); 2] {
        let s = self.state(t);
        let m = self.m1 + self.m2;
        let (r1, r2) = (s.a * self.m2 / m, s.a * self.m1 / m);
        let (sn, c) = s.phase.sin_cos_det();
        [(-r1 * c, -r1 * sn), (r2 * c, r2 * sn)]
    }

    /// Point masses `(x, y, m)` at `t`: the pair, or the remnant and a massless partner.
    fn masses(&self, t: f64) -> [(f64, f64, f64); 2] {
        let s = self.state(t);
        if s.merged {
            return [
                (0.0, 0.0, (self.m1 + self.m2) * s.mass_scale),
                (0.0, 0.0, 0.0),
            ];
        }
        let [(x1, y1), (x2, y2)] = self.positions(t);
        [
            (x1, y1, self.m1 * s.mass_scale),
            (x2, y2, self.m2 * s.mass_scale),
        ]
    }

    /// The strain a distant face-on detector records at `t`: `S cos 2Ψ` (SPT-A6).
    pub(crate) fn strain(&self, t: f64) -> f64 {
        let s = self.state(t);
        s.strain * (2.0 * s.pattern).cos_det()
    }

    /// The strain envelope's largest value, the inset's full scale.
    fn peak_strain(&self) -> f64 {
        PEAK_GAIN / Q_MERGE
    }

    /// The inset's height for the strain at `t` in `[-1, 1]`: a signed square-root scale
    /// of `strain / peak`, so the early chirp reads as well as the merger (SPT-A6).
    fn inset_strain(&self, t: f64) -> f64 {
        let h = (self.strain(t) / self.peak_strain()).clamp(-1.0, 1.0);
        h.signum() * h.abs().sqrt()
    }

    /// Brightness of the scene through the cut between cycles: 0 at the cut, 1 away.
    fn dip(&self, t: f64) -> f64 {
        let tau = self.state(t).tau;
        (tau / DIP).min((self.cycle() - tau) / DIP).clamp(0.0, 1.0)
    }

    /// The merger flash in `[0, 1]`: rising as `s²` through the plunge, 1 at coalescence,
    /// then fading with `FLASH_TAU` (SPT-A8).
    fn flash(&self, t: f64) -> f64 {
        let tau = self.state(t).tau;
        let (t_i, t_m) = (self.inspiral_time(), self.merge_time());
        if tau < t_i {
            0.0
        } else if tau < t_m {
            ((tau - t_i) / PLUNGE).powi(2)
        } else {
            (-(tau - t_m) / FLASH_TAU).exp()
        }
    }

    /// The caption at `t` (SPT-A7): `merger` from `MERGER_LEAD` before the plunge to
    /// `MERGER_TAIL` after coalescence, then `ringdown` until `RINGDOWN_SHOWN` after it.
    fn caption(&self, t: f64) -> Option<&'static str> {
        let since = self.state(t).tau - self.merge_time();
        if since < -(PLUNGE + MERGER_LEAD) {
            None
        } else if since < MERGER_TAIL {
            Some(MERGER)
        } else if since < RINGDOWN_SHOWN {
            Some(RINGDOWN)
        } else {
            None
        }
    }
}

/// Display low-pass of the sheet: waves whose pattern turns faster than `max_speed` (so
/// shorter than the mesh resolves) are faded as `(max_speed / speed)²` rather than
/// aliased into moiré. The strain inset shows them unfiltered (SPEC-002).
fn sheet_gain(speed: f64, max_speed: f64) -> f64 {
    let r = max_speed / speed.abs().max(1e-12);
    (r * r).min(1.0)
}

/// A black hole as drawn: centre and radius in dots, and its rim temperature (K).
#[derive(Debug, Clone, Copy)]
struct Body {
    dx: f64,
    dy: f64,
    r: f64,
    kelvin: f64,
}

impl Body {
    /// The disc the mesh and trails keep out of: the rim and its moat.
    fn hole(&self) -> (f64, f64, f64) {
        (self.dx, self.dy, MOAT * self.r + 1.0)
    }
}

/// A one-dot ring of radius `r` dots about `(dx, dy)`.
fn ring(dots: &mut Dots, (dx, dy): (f64, f64), r: f64, ink: Ink) {
    let steps = (TAU * r).ceil().clamp(8.0, 512.0) as usize;
    for k in 0..steps {
        let (s, c) = (TAU * k as f64 / steps as f64).sin_cos_det();
        dots.plot(dx + r * c, dy + r * s, ink);
    }
}

/// The segment `a–b` with the parts inside the discs `holes` (`(x, y, radius)`, dots)
/// left out, so the black holes' dark cores occlude what lies behind them (SPT-A9).
fn line_outside(
    dots: &mut Dots,
    a: (f64, f64),
    b: (f64, f64),
    holes: &[(f64, f64, f64)],
    ink: Ink,
) {
    let (ux, uy) = (b.0 - a.0, b.1 - a.1);
    let len2 = ux * ux + uy * uy;
    // Parameter intervals of the segment inside each disc, at most one per disc.
    let mut inside = [(1.0f64, 1.0f64); 2];
    for (slot, &(cx, cy, r)) in inside.iter_mut().zip(holes) {
        let (px, py) = (a.0 - cx, a.1 - cy);
        let half_b = px * ux + py * uy;
        let c = px * px + py * py - r * r;
        let disc = half_b * half_b - len2 * c;
        if len2 > 0.0 && disc > 0.0 {
            let root = disc.sqrt();
            let (t0, t1) = (
                ((-half_b - root) / len2).max(0.0),
                ((-half_b + root) / len2).min(1.0),
            );
            if t0 < t1 {
                *slot = (t0, t1);
            }
        } else if c < 0.0 {
            // A zero-length segment inside the disc.
            *slot = (0.0, 1.0);
        }
    }
    if inside[1].0 < inside[0].0 {
        inside.swap(0, 1);
    }
    let at = |t: f64| (a.0 + ux * t, a.1 + uy * t);
    let mut from = 0.0;
    for (t0, t1) in inside {
        if t0 > from {
            let (p, q) = (at(from), at(t0));
            dots.line(p.0, p.1, q.0, q.1, ink);
        }
        from = from.max(t1);
    }
    if from < 1.0 {
        let p = at(from);
        dots.line(p.0, p.1, b.0, b.1, ink);
    }
}

/// Softened Newtonian potential depth under point masses `(x, y, m)` (SPT-A1).
pub(crate) fn well_depth(x: f64, y: f64, masses: &[(f64, f64, f64)]) -> f64 {
    -WELL_K
        * masses
            .iter()
            .map(|&(mx, my, m)| {
                m / ((x - mx).powi(2) + (y - my).powi(2) + SOFTENING * SOFTENING).sqrt()
            })
            .sum::<f64>()
}

/// The radial wave envelope's largest value, `f(r)` near `r = 0.78`.
const ENVELOPE_MAX: f64 = 0.73;

/// Radial wave envelope: `1/√r` far away (2-D energy flux conservation on the sheet),
/// Gaussian-tapered to zero in the near zone.
pub(crate) fn envelope(r: f64) -> f64 {
    let r = r.max(0.0);
    if r < 1e-12 {
        return 0.0;
    }
    (1.0 - (-(r / TAPER).powi(2)).exp()) * (TAPER / r).sqrt()
}

/// Quadrupole wave height at polar `(r, φ)` about the centre of mass (SPT-A3):
/// `A · f(r) · S(t_r) · cos 2(φ − Ψ(t_r))` with retarded time `t_r = t − r/c`.
/// Reference form; `refresh` evaluates it separably per sample (checked by
/// `spt_mesh_heights_match_the_field`).
#[cfg(test)]
pub(crate) fn wave(bin: &Binary, r: f64, phi: f64, t: f64) -> f64 {
    let s = bin.state(t - r / WAVE_SPEED);
    WAVE_AMP * envelope(r) * s.strain * (2.0 * (phi - s.pattern)).cos_det()
}

/// Fixed world data of one mesh sample, stored as `f32` (a large mesh holds ~20 000):
/// `x, y, r`, then `cos 2φ, sin 2φ` (the quadrupole's azimuthal pattern) and the wave
/// envelope, all precomputed per resize.
#[derive(Debug, Clone, Copy, Default)]
struct Base([f32; 6]);

impl Base {
    fn new(x: f64, y: f64) -> Self {
        let r = x.hypot(y);
        let (s2, c2) = (2.0 * y.atan2(x)).sin_cos_det();
        Self([x, y, r, c2, s2, envelope(r)].map(|v| v as f32))
    }

    /// `[x, y, r, cos 2φ, sin 2φ, envelope]`.
    fn get(&self) -> [f64; 6] {
        self.0.map(f64::from)
    }
}

/// Per-step state of one mesh sample.
#[derive(Debug, Clone, Copy, Default)]
struct Pt {
    /// Dot-lattice position.
    dx: f64,
    dy: f64,
    depth: f64,
    well: f64,
    crest: f64,
    envn: f64,
    /// The unfiltered wave amplitude `f(r) S(t_r)` over its largest value, squared: the
    /// energy density the light shows even where the ripples are too fine for the mesh.
    loud: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Camera {
    cos_yaw: f64,
    sin_yaw: f64,
    scale: f64,
    ox: f64,
    oy: f64,
}

impl Camera {
    /// Normalised perspective coordinates `(u, v, depth)` of a world point.
    fn view(cos_yaw: f64, sin_yaw: f64, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let (cp, sp) = (PITCH.cos_det(), PITCH.sin_det());
        let (xr, yr) = (x * cos_yaw - y * sin_yaw, x * sin_yaw + y * cos_yaw);
        let d = (yr * cp - z * sp + CAM_DIST).max(0.1);
        let up = yr * sp + z * cp;
        (xr / d, up / d, d)
    }

    /// Cell coordinates (cell `i` centred at `i`) and depth of a world point.
    fn project(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let (u, v, d) = Self::view(self.cos_yaw, self.sin_yaw, x, y, z);
        (
            self.ox + self.scale * ASPECT * u,
            self.oy - self.scale * v,
            d,
        )
    }
}

/// The strain inset's rectangle in cells, clear of the title row (SPT-A6).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Inset {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Inset {
    /// Narrowest inset worth drawing (cells).
    const MIN_W: i32 = 12;

    /// Top-left, left of the far edge; `None` on small canvases.
    fn for_size(size: Size) -> Option<Self> {
        if size.width() < INSET_MIN.0 || size.height() < INSET_MIN.1 {
            return None;
        }
        Some(Self {
            x: 1,
            y: 1,
            w: i32::from(size.width() / 6).clamp(Self::MIN_W, 22),
            h: i32::from(INSET_ROWS),
        })
    }

    /// Narrowed to end two columns left of `mesh_left`, the mesh's leftmost column in
    /// the inset's rows; `None` if too little room is left (SPT-A6).
    fn clear_of(self, mesh_left: f64) -> Option<Self> {
        let w = self.w.min(mesh_left.floor() as i32 - 2 - self.x);
        (w >= Self::MIN_W).then_some(Self { w, ..self })
    }
}

#[derive(Debug, Default)]
pub struct SpacetimeGrid {
    bin: Option<Binary>,
    t: f64,
    lx: f64,
    /// Mesh lines along x (`ny` of them, `fx` samples each) and along y (`nx`, `fy`).
    nx: usize,
    ny: usize,
    fx: usize,
    fy: usize,
    base: Vec<Base>,
    pts: Vec<Pt>,
    cam: Camera,
    depth_near: f64,
    depth_far: f64,
    max_speed: f64,
    /// Dot layers: the sheet, the masses with their trails, and the strain trace.
    mesh: Dots,
    bodies: Dots,
    inset: Option<Inset>,
    trace: Dots,
}

impl SpacetimeGrid {
    fn binary(&self) -> Binary {
        self.bin.unwrap_or(Binary::DEFAULT)
    }

    /// Fit the sheet into the canvas and build the mesh sample lists.
    fn layout(&mut self, size: Size) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let extent = |lx: f64| {
            let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for yaw in [-YAW_AMP, 0.0, YAW_AMP] {
                let (s, c) = f64::sin_cos_det(yaw);
                for (x, y, z) in [
                    (-lx, -1.0, 0.0),
                    (lx, -1.0, 0.0),
                    (-lx, 1.0, 0.25),
                    (lx, 1.0, 0.25),
                    (0.0, -0.3, 0.15),
                    (0.0, 0.0, DEEPEST),
                ] {
                    let (u, v, _) = Camera::view(c, s, x, y, z);
                    (u0, u1, v0, v1) = (u0.min(u), u1.max(u), v0.min(v), v1.max(v));
                }
            }
            (u0, u1, v0, v1)
        };
        // Widen the sheet until its projected aspect matches the canvas.
        self.lx = 1.0;
        for _ in 0..3 {
            let (u0, u1, v0, v1) = extent(self.lx);
            let rho = ASPECT * (u1 - u0) / (v1 - v0);
            self.lx = (self.lx * w / h / rho).clamp(1.0, 2.6);
        }
        // Rows 1 ..= h − 3: a clear top row, and the title row with a gap above it free.
        // Where the strain inset would be squeezed, narrow the sheet (not its scale) until
        // the inset gets its full width (SPT-A6).
        let full = Inset::for_size(size);
        for _ in 0..8 {
            let (u0, u1, v0, v1) = extent(self.lx);
            let rows = (h - 4.0).max(0.0);
            let scale = ((w - 1.0) * 0.97 / (ASPECT * (u1 - u0))).min(rows / (v1 - v0));
            self.cam.scale = scale.max(0.0);
            self.cam.ox = (w - 1.0) / 2.0 - self.cam.scale * ASPECT * (u0 + u1) / 2.0;
            self.cam.oy = 1.0 + 0.5 * (rows - self.cam.scale * (v1 - v0)) + self.cam.scale * v1;
            self.inset = self.place_inset(size);
            let squeezed = full.is_some_and(|f| self.inset.is_none_or(|r| r.w < f.w));
            if !squeezed || self.lx <= 1.0 {
                break;
            }
            self.lx = (self.lx * 0.93).max(1.0);
        }
        self.depth_near = Camera::view(1.0, 0.0, 0.0, -1.0, 0.0).2;
        self.depth_far = Camera::view(1.0, 0.0, 0.0, 1.0, 0.0).2;

        self.nx = (usize::from(size.width() / COLS_PER_LINE)).clamp(1, MAX_NX) + 1;
        self.ny = ((h / ROWS_PER_LINE).round() as usize).clamp(1, MAX_NY) + 1;
        self.fx = (self.nx - 1) * SUB_X + 1;
        self.fy = (self.ny - 1) * SUB_Y + 1;
        let lx = self.lx;
        let gx = |i: usize, n: usize| -lx + 2.0 * lx * i as f64 / (n - 1) as f64;
        let gy = |j: usize, n: usize| -1.0 + 2.0 * j as f64 / (n - 1) as f64;
        let sample = Base::new;
        self.base.clear();
        for j in 0..self.ny {
            for i in 0..self.fx {
                self.base.push(sample(gx(i, self.fx), gy(j, self.ny)));
            }
        }
        for i in 0..self.nx {
            for j in 0..self.fy {
                self.base.push(sample(gx(i, self.nx), gy(j, self.fy)));
            }
        }
        self.pts.clear();
        self.pts.resize(self.base.len(), Pt::default());
        self.mesh.resize(size);
        self.bodies.resize(size);
        self.trace.resize(size);
        // The shortest wave the mesh resolves: 2.5 line spacings.
        let spacing = (2.0 / (self.ny - 1) as f64).max(2.0 * self.lx / (self.nx - 1) as f64);
        self.max_speed = PI * WAVE_SPEED / (2.5 * spacing);
    }

    /// The leftmost column the mesh can reach in the rows of `inset` (and the row below
    /// it), over the yaw sway and ±0.2 of sheet height; +∞ if it never gets there.
    /// And, second, the rightmost column the mesh reaches anywhere.
    fn mesh_span(&self, inset: Inset) -> (f64, f64) {
        let bottom = f64::from(inset.y + inset.h);
        let lx = self.lx;
        let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
        for yaw in [-YAW_AMP, 0.0, YAW_AMP] {
            let (s, c) = f64::sin_cos_det(yaw);
            for k in 0..=64 {
                let f = f64::from(k) / 64.0;
                // The far edge, the left edge and the right edge.
                for (x, y) in [
                    (-lx + 2.0 * lx * f, 1.0),
                    (-lx, 1.0 - 2.0 * f),
                    (lx, 1.0 - 2.0 * f),
                ] {
                    for z in [-0.2, 0.2] {
                        let (u, v, _) = Camera::view(c, s, x, y, z);
                        let (col, row) = (
                            self.cam.ox + self.cam.scale * ASPECT * u,
                            self.cam.oy - self.cam.scale * v,
                        );
                        right = right.max(col);
                        if row <= bottom + 0.5 {
                            left = left.min(col);
                        }
                    }
                }
            }
        }
        (left, right)
    }

    /// Place the strain inset, first sliding the mesh right into any free margin to
    /// make room for it (SPT-A6).
    fn place_inset(&mut self, size: Size) -> Option<Inset> {
        let r = Inset::for_size(size)?;
        let (left, right) = self.mesh_span(r);
        let need = f64::from(r.x + r.w + 2) - left;
        let slack = f64::from(size.width()) - 2.0 - right;
        self.cam.ox += need.min(slack).max(0.0);
        r.clear_of(self.mesh_span(r).0)
    }

    /// Evaluate the sheet height and projection of every sample at the current time.
    fn refresh(&mut self) {
        let bin = self.binary();
        let t = self.t;
        let (sy, cy) = (YAW_AMP * (TAU * t / YAW_PERIOD).sin_det()).sin_cos_det();
        self.cam.cos_yaw = cy;
        self.cam.sin_yaw = sy;
        let masses = bin.masses(t);
        let lift = RIM_LIFT * (bin.m1 + bin.m2);
        let inv_peak = 1.0 / (ENVELOPE_MAX * bin.peak_strain());
        for (p, b) in self.pts.iter_mut().zip(&self.base) {
            let [bx, by, r, c2, s2, env] = b.get();
            let well = well_depth(bx, by, &masses);
            let s = bin.state(t - r / WAVE_SPEED);
            let (sn, c) = (2.0 * s.pattern).sin_cos_det();
            // cos 2(φ − Ψ) = cos 2φ cos 2Ψ + sin 2φ sin 2Ψ
            let crest = c2 * c + s2 * sn;
            let envn = env * s.strain * sheet_gain(s.speed, self.max_speed);
            let amp = env * s.strain * inv_peak;
            let z = well + WAVE_AMP * envn * crest;
            let (x, y, depth) = self.cam.project(bx, by, z + lift);
            let (dx, dy) = Dots::from_cells(x + 0.5, y + 0.5);
            *p = Pt {
                dx,
                dy,
                depth,
                well,
                crest,
                envn,
                loud: amp * amp,
            };
        }
        self.rasterise();
    }

    /// Dot position of the sheet surface under world point `(x, y)`, raised by `up`.
    fn surface_dot(&self, x: f64, y: f64, up: f64, masses: &[(f64, f64, f64)]) -> (f64, f64, f64) {
        let lift = RIM_LIFT * (self.binary().m1 + self.binary().m2);
        let (sx, sy, d) = self.cam.project(x, y, well_depth(x, y, masses) + lift + up);
        let (dx, dy) = Dots::from_cells(sx + 0.5, sy + 0.5);
        (dx, dy, d)
    }

    /// The black holes now: at the sheet surface, their radius swelling with the merger
    /// flash; the remnant's partner (massless) is `None`.
    fn bodies_now(&self, bin: Binary) -> [Option<Body>; 2] {
        let t = self.t;
        let merged = bin.state(t).merged;
        let masses = bin.masses(t);
        let flash = bin.flash(t);
        std::array::from_fn(|k| {
            let (x, y, m) = masses[k];
            (m > 0.0).then(|| {
                let rad = BODY_R * m.cbrt() * (1.0 + flash);
                let (dx, dy, d) = self.surface_dot(x, y, 0.0, &masses);
                let r = self.cam.scale * rad / d * f64::from(Dots::CELL_H);
                let kelvin = if merged { REMNANT_K } else { HOT[k] };
                Body {
                    dx,
                    dy,
                    r: r.clamp(BODY_MIN, 64.0),
                    kelvin,
                }
            })
        })
    }

    /// Draw the black holes (a dark core inside a hot rim, in a dark moat), their
    /// fading orbit trails and the merger flash onto `bodies`.
    fn draw_bodies(
        &mut self,
        bin: Binary,
        dip: f64,
        now: &[Option<Body>; 2],
        holes: &[(f64, f64, f64)],
    ) {
        self.bodies.clear();
        if dip <= 0.0 {
            return;
        }
        let t = self.t;
        let state = bin.state(t);
        let masses = bin.masses(t);
        let flash = bin.flash(t);
        // Trails: where each mass was over the last `TRAIL` s of this cycle, cooling.
        if !state.merged {
            for (k, kelvin) in HOT.into_iter().enumerate() {
                let mut prev = None;
                for n in 0..=TRAIL_N {
                    let back = t - TRAIL * n as f64 / TRAIL_N as f64;
                    let then = bin.state(back);
                    if then.merged || then.tau > state.tau {
                        break;
                    }
                    let (x, y) = bin.positions(back)[k];
                    let (dx, dy, _) = self.surface_dot(x, y, 0.0, &masses);
                    let fade = 1.0 - n as f64 / (TRAIL_N + 1) as f64;
                    let ink = ink::blackbody(kelvin * (0.1 + 0.25 * fade) * dip);
                    if let Some(p) = prev {
                        line_outside(&mut self.bodies, p, (dx, dy), holes, ink);
                    }
                    prev = Some((dx, dy));
                }
            }
        }
        let glow = (0.5 + 0.5 * dip) * (1.0 + 0.5 * flash);
        for b in now.iter().flatten() {
            let centre = (b.dx, b.dy);
            let rim = ink::blackbody(b.kelvin * glow);
            let halo = ink::blackbody(HALO_K * b.kelvin * glow);
            ring(&mut self.bodies, centre, b.r + HALO_GAP, halo);
            ring(&mut self.bodies, centre, b.r, rim);
            ring(&mut self.bodies, centre, b.r - 0.7, rim);
        }
    }

    /// Draw the mesh, the masses and the strain trace into their dot layers.
    fn rasterise(&mut self) {
        let (fx, fy, nx, ny) = (self.fx, self.fy, self.nx, self.ny);
        let bin = self.binary();
        let dip = bin.dip(self.t);
        let gain = dip * (1.0 + FLASH_GAIN * bin.flash(self.t));
        let bodies = self.bodies_now(bin);
        let mut holes = [(0.0, 0.0, 0.0); 2];
        let mut n_holes = 0;
        for b in bodies.iter().flatten() {
            holes[n_holes] = b.hole();
            n_holes += 1;
        }
        let holes = &holes[..n_holes];
        let (pts, mesh) = (&self.pts, &mut self.mesh);
        let depths = (self.depth_near, self.depth_far);
        mesh.clear();
        if pts.len() == ny * fx + nx * fy && dip > 0.0 {
            let mut seg = |a: &Pt, b: &Pt| {
                let ink = mesh_ink(a, b, depths, gain, near_cap(a, b, holes));
                line_outside(mesh, (a.dx, a.dy), (b.dx, b.dy), holes, ink);
            };
            for j in 0..ny {
                for i in 0..fx - 1 {
                    seg(&pts[j * fx + i], &pts[j * fx + i + 1]);
                }
            }
            let yline = |i: usize, j: usize| &pts[ny * fx + i * fy + j];
            for i in 0..nx {
                for j in 0..fy - 1 {
                    seg(yline(i, j), yline(i, j + 1));
                }
            }
        }
        self.draw_bodies(bin, dip, &bodies, holes);
        self.trace_strain(bin, dip);
    }
}

/// The brightest level the mesh segment `a–b` may take near the moats `holes`:
/// `NEAR_MAX` within `NEAR_HOLE.0` moat radii, easing up to `MESH_MAX` by `NEAR_HOLE.1`.
fn near_cap(a: &Pt, b: &Pt, holes: &[(f64, f64, f64)]) -> f64 {
    let (mx, my) = (0.5 * (a.dx + b.dx), 0.5 * (a.dy + b.dy));
    let near = holes
        .iter()
        .map(|&(cx, cy, r)| (mx - cx).hypot(my - cy) / r)
        .fold(f64::INFINITY, f64::min);
    let s = ease::smoothstep((near - NEAR_HOLE.0) / (NEAR_HOLE.1 - NEAR_HOLE.0));
    NEAR_MAX + (MESH_MAX - NEAR_MAX) * s
}

/// `Seq` ink of the mesh segment `a–b` (`depths` = near and far edge depths,
/// `gain` = the cut's dip times the merger flash, `cap` = its brightest level): wells,
/// crests and the wave's energy carry the level, nearness only shades it and the
/// foreground edge fades.
fn mesh_ink(a: &Pt, b: &Pt, (depth_near, depth_far): (f64, f64), gain: f64, cap: f64) -> Ink {
    let well = 0.5 * (a.well + b.well);
    let crest = (0.5 * (a.crest * a.envn + b.crest * b.envn) / CREST_FULL).clamp(-1.0, 1.0);
    let loud = 0.5 * (a.loud + b.loud);
    let depth = 0.5 * (a.depth + b.depth);
    let span = (depth_far - depth_near).max(1e-9);
    let near = ((depth_far - depth) / span).clamp(0.0, 1.0);
    let pit = ((-well - 0.25) / 0.6).clamp(0.0, 1.0);
    let edge = 1.0 - EDGE_FADE * ease::smoothstep((near - 0.7) / 0.3);
    let shade = (1.0 - NEAR_SHADE + NEAR_SHADE * near) * edge;
    let level = ((REST + WELL_GLOW * pit) * (1.0 + TROUGH_DIM * crest.min(0.0))
        + CREST_LIFT * crest.max(0.0)
        + BURST_LIFT * loud)
        * shade
        * gain;
    Ink::new(MESH_ROLE, ink::level(level.min(cap)))
}

impl SpacetimeGrid {
    /// The strain inset: the source strain over the last `INSET_WINDOW` s, one sample per
    /// dot column, joined by segments (SPT-A6).
    fn trace_strain(&mut self, bin: Binary, dip: f64) {
        self.trace.clear();
        let Some(r) = self.inset else { return };
        let cols = r.w * Dots::CELL_W;
        let (top, rows) = (f64::from(r.y * Dots::CELL_H), f64::from(r.h * Dots::CELL_H));
        let mid = top + 0.5 * (rows - 1.0);
        let half = 0.5 * (rows - 1.0);
        let x0 = f64::from(r.x * Dots::CELL_W);
        let axis = Ink::new(
            INSET_AXIS.role,
            ink::level(f64::from(INSET_AXIS.level) * dip),
        );
        for c in 0..cols {
            self.trace.plot(x0 + f64::from(c), mid, axis);
        }
        let (quiet, peak) = TRACE_LEVELS;
        let mut prev: Option<(f64, f64)> = None;
        for c in 0..cols {
            let t = self.t - INSET_WINDOW * (1.0 - f64::from(c) / f64::from(cols - 1));
            let p = (x0 + f64::from(c), mid - half * bin.inset_strain(t));
            if let Some(q) = prev {
                // Brighter as the envelope grows: the chirp peak outshines the inspiral.
                let loud = (bin.state(t).strain / bin.peak_strain()).clamp(0.0, 1.0);
                let level = ink::level((quiet + (peak - quiet) * loud) * dip);
                self.trace.line(
                    q.0,
                    q.1,
                    p.0,
                    p.1,
                    Ink::new(Role::Ramp(Ramp::Aurora), level),
                );
            }
            prev = Some(p);
        }
    }
}

impl Generator for SpacetimeGrid {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.bin.is_none() {
            let mut bin = Binary::DEFAULT;
            bin.phase0 = ctx.rng.random_range(0.0..TAU);
            // Start part-way into the chirp so the first frames already have a wave train.
            self.t = ctx.rng.random_range(0.0..0.5 * bin.inspiral_time());
            self.bin = Some(bin);
        }
        self.layout(size);
        self.refresh();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.refresh();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.mesh.blit(canvas);
        self.bodies.blit(canvas);
        let Some(r) = self.inset else { return };
        let blank = Glyph::ascii(' ');
        for y in r.y - 1..r.y + r.h {
            for x in r.x - 1..=r.x + r.w {
                canvas.put(x, y, blank, Ink::BLANK);
            }
        }
        self.trace.blit(canvas);
        let label = Ink::new(Role::Text, title::READOUT_LEVEL);
        draw::label(canvas, r.x, r.y - 1, INSET_LABEL, label);
    }

    fn caption(&self) -> Option<&'static str> {
        self.binary().caption(self.t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circular() -> Binary {
        Binary {
            inspiral: false,
            phase0: 0.7,
            ..Binary::DEFAULT
        }
    }

    #[test]
    fn spt_a1_single_mass_well_is_radially_symmetric_and_deepens_toward_mass() {
        let (mx, my) = (0.3, -0.2);
        let masses = [(mx, my, 1.0)];
        let mut prev = 0.0f64;
        // Walk inward from r = 2 to r = 0: |z| must strictly increase.
        for k in (0..=200).rev() {
            let r = 0.01 * f64::from(k);
            let ref_z = well_depth(mx + r, my, &masses);
            assert!(ref_z < 0.0, "well points down");
            for j in 0..16 {
                let th = TAU * f64::from(j) / 16.0;
                let z = well_depth(mx + r * th.cos_det(), my + r * th.sin_det(), &masses);
                assert!((z - ref_z).abs() < 1e-12, "not symmetric at r={r} th={th}");
            }
            if k < 200 {
                assert!(ref_z.abs() > prev, "not strictly deeper at r={r}");
            }
            prev = ref_z.abs();
        }
    }

    #[test]
    fn spt_a2_circular_binary_fixed_com_and_constant_separation() {
        let b = circular();
        for k in 0..600 {
            let t = 0.05 * f64::from(k);
            let [(x1, y1), (x2, y2)] = b.positions(t);
            let (cx, cy) = (b.m1 * x1 + b.m2 * x2, b.m1 * y1 + b.m2 * y2);
            assert!(cx.abs() < 1e-12 && cy.abs() < 1e-12, "COM moved at t={t}");
            let sep = (x2 - x1).hypot(y2 - y1);
            assert!((sep - b.a0).abs() < 1e-12, "separation {sep} at t={t}");
        }
        // It does orbit: a quarter period later the pair has turned by 90 degrees.
        let [(x1, y1), _] = b.positions(0.0);
        let [(x1q, y1q), _] = b.positions(PI / 2.0 / b.omega0);
        assert!((x1 * x1q + y1 * y1q).abs() < 1e-12);
        // Heavier mass sits closer to the centre of mass.
        let [(x1, y1), (x2, y2)] = b.positions(1.3);
        assert!(x1.hypot(y1) < x2.hypot(y2));
    }

    #[test]
    fn spt_a3_wave_has_azimuthal_period_pi_and_retarded_phase() {
        for b in [
            circular(),
            Binary {
                phase0: 0.3,
                ..Binary::DEFAULT
            },
        ] {
            for &(r, phi, t) in &[
                (0.3, 0.2, 3.0),
                (0.8, 2.5, 7.3),
                (1.7, -1.0, 12.0),
                (2.4, 4.0, 13.5),
            ] {
                let w = wave(&b, r, phi, t);
                // Period pi (quadrupole, m = 2) and not period pi/2.
                assert!((wave(&b, r, phi + PI, t) - w).abs() < 1e-12);
                // Phase equals the source's orbital angle at the retarded time t - r/c,
                // read off the actual mass positions.
                let tr = t - r / WAVE_SPEED;
                let [(x1, y1), (x2, y2)] = b.positions(tr);
                let psi = (y2 - y1).atan2(x2 - x1);
                let amp = WAVE_AMP * envelope(r) * b.state(tr).strain;
                let expect = amp * (2.0 * (phi - psi)).cos_det();
                assert!(
                    (w - expect).abs() < 1e-9,
                    "r={r} phi={phi} t={t}: {w} vs {expect}"
                );
            }
        }
        // Retardation matters: half a wave period of delay flips the sign.
        let b = circular();
        let r = WAVE_SPEED * (PI / b.omega0) / 2.0;
        let phi = b.state(3.0).phase;
        assert!(wave(&b, r, phi, 3.0) < 0.0, "delay must invert the crest");
    }

    #[test]
    fn spt_envelope_tapers_near_source_and_decays_like_inverse_sqrt() {
        assert!(envelope(0.0).abs() < 1e-12);
        assert!(envelope(0.05) < 0.1 * envelope(0.8));
        let ratio = envelope(8.0) / envelope(2.0);
        assert!((ratio - 0.5).abs() < 0.01, "1/sqrt(r): {ratio}");
    }

    #[test]
    fn spt_inspiral_follows_kepler_and_the_strain_grows_as_one_over_a() {
        let b = Binary::DEFAULT;
        for &t in &[3.0, 8.0, 13.0] {
            let h = 1e-5;
            let omega = (b.state(t + h).phase - b.state(t - h).phase) / (2.0 * h);
            let s = b.state(t);
            let kepler = b.omega0 * (b.a0 / s.a).powf(1.5);
            assert!((omega - kepler).abs() < 1e-6 * kepler, "t={t}");
            assert!((s.strain - b.a0 / s.a).abs() < 1e-12, "t={t}");
            assert!(b.state(t + 0.1).a < s.a, "shrinks");
        }
    }

    #[test]
    fn spt_a4_the_pair_plunges_continuously_into_one_remnant() {
        let b = Binary::DEFAULT;
        let (t_i, t_m) = (b.inspiral_time(), b.merge_time());
        let mut last = f64::INFINITY;
        let n = 400;
        for k in 0..=n {
            let t = t_i - 1.0 + (t_m - t_i + 1.0) * f64::from(k) / f64::from(n);
            let s = b.state(t);
            assert!(s.a <= last + 1e-12, "separation grew at t={t}");
            last = s.a;
            let [(x1, y1), (x2, y2)] = b.positions(t);
            assert!((b.m1 * x1 + b.m2 * x2).abs() < 1e-12 && (b.m1 * y1 + b.m2 * y2).abs() < 1e-12);
        }
        // Continuous at both joins.
        for t in [t_i, t_m] {
            let (lo, hi) = (b.state(t - 1e-9), b.state(t + 1e-9));
            assert!(
                (lo.a - hi.a).abs() < 1e-6 && (lo.phase - hi.phase).abs() < 1e-6,
                "t={t}"
            );
            let depth = |s: f64| well_depth(0.3, 0.1, &b.masses(s));
            assert!(
                (depth(t - 1e-9) - depth(t + 1e-9)).abs() < 1e-6,
                "well jumps at t={t}"
            );
        }
        let after = b.masses(t_m + 1.0);
        assert_eq!(after[0], (0.0, 0.0, (1.0 - RADIATED) * (b.m1 + b.m2)));
        assert_eq!(after[1].2, 0.0);
    }

    #[test]
    fn spt_a5_strain_peaks_at_merger_then_rings_down_faster() {
        let b = Binary::DEFAULT;
        let t_m = b.merge_time();
        let peak = b.state(t_m).strain;
        assert!((peak - b.peak_strain()).abs() < 1e-12);
        for k in 1..200 {
            let t = 0.1 * f64::from(k);
            assert!(b.state(t).strain <= peak + 1e-12, "t={t}");
        }
        for dt in [0.1, 0.35, 1.0, 2.0] {
            let expect = peak * (-dt / TAU_RD).exp();
            assert!((b.state(t_m + dt).strain - expect).abs() < 1e-12);
        }
        let h = 1e-5;
        let speed = |t: f64| (b.state(t + h).pattern - b.state(t - h).pattern) / (2.0 * h);
        assert!(speed(t_m + 0.5) > speed(b.inspiral_time() - 0.01));
    }

    #[test]
    fn spt_a6_the_inset_traces_the_strain_clear_of_the_title() {
        for (w, h) in [(80, 24), (200, 50), (48, 16), (47, 40), (120, 15)] {
            let size = Size::new(w, h);
            let Some(r) = Inset::for_size(size) else {
                assert!(w < INSET_MIN.0 || h < INSET_MIN.1);
                continue;
            };
            // Inside the canvas with its cleared margin, and far above the title row.
            assert!(r.x >= 1 && r.y >= 1 && r.x + r.w < i32::from(w));
            assert!(r.y + r.h < i32::from(h) - 1);
        }
        // The mesh never runs into the inset's cleared box.
        for (w, h) in [(80, 24), (120, 40), (200, 50), (60, 30), (240, 30)] {
            let size = Size::new(w, h);
            let mut g = SpacetimeGrid::default();
            let mut ctx = Ctx::new(6, size);
            g.resize(size, &mut ctx);
            let Some(r) = g.inset else { continue };
            for _ in 0..240 {
                g.update(4.0 * SIM_DT, &mut ctx);
                for y in r.y - 1..r.y + r.h {
                    for x in r.x - 1..=r.x + r.w {
                        assert_eq!(g.mesh.cell_mask(x, y), Some(0), "{w}×{h}: mesh at {x},{y}");
                    }
                }
            }
        }
        // From 120 columns up the inset gets its full width (the sheet narrows instead).
        for (w, h) in [(120, 40), (200, 50), (160, 40), (240, 30)] {
            let size = Size::new(w, h);
            let mut g = SpacetimeGrid::default();
            g.resize(size, &mut Ctx::new(3, size));
            let full = Inset::for_size(size).expect("inset").w;
            assert!(full >= 20, "{w}×{h}");
            assert_eq!(g.inset.map(|r| r.w), Some(full), "{w}×{h}");
        }
        let size = Size::new(120, 40);
        let mut g = SpacetimeGrid::default();
        let mut ctx = Ctx::new(2, size);
        g.resize(size, &mut ctx);
        for _ in 0..200 {
            g.update(SIM_DT, &mut ctx);
        }
        let r = g.inset.expect("inset at 120×40");
        let bin = g.binary();
        let cols = r.w * Dots::CELL_W;
        let rows = f64::from(r.h * Dots::CELL_H);
        let mid = f64::from(r.y * Dots::CELL_H) + 0.5 * (rows - 1.0);
        for c in [0, cols / 3, cols - 1] {
            let t = g.t - INSET_WINDOW * (1.0 - f64::from(c) / f64::from(cols - 1));
            let y = mid - 0.5 * (rows - 1.0) * bin.inset_strain(t);
            assert!(
                g.trace.is_lit(r.x * Dots::CELL_W + c, y.round() as i32),
                "column {c}"
            );
        }
    }

    #[test]
    fn spt_a7_captions_follow_the_merger() {
        let mut g = SpacetimeGrid::default();
        let mut ctx = Ctx::new(1, Size::new(80, 24));
        g.resize(Size::new(80, 24), &mut ctx);
        let bin = g.binary();
        let cycle = bin.cycle();
        let (mut merger, mut last) = (0.0, None);
        for k in 0..2000 {
            g.t = cycle * f64::from(k) / 1000.0;
            let tau = bin.state(g.t).tau;
            let expect = if tau < bin.inspiral_time() - 0.5 {
                None
            } else if tau < bin.merge_time() + 0.5 {
                Some(MERGER)
            } else if tau < bin.merge_time() + RINGDOWN_SHOWN {
                Some(RINGDOWN)
            } else {
                None
            };
            assert_eq!(g.caption(), expect, "tau={tau}");
            if expect == Some(MERGER) && last == Some(MERGER) {
                merger += cycle / 1000.0;
            }
            last = expect;
        }
        // Long enough to read: 1.5 s per cycle, over two cycles.
        assert!(merger > 2.0 * 1.4, "merger shown {merger} s");
    }

    #[test]
    fn spt_gen_r13_the_mesh_clears_the_top_row_and_the_title_row() {
        for (w, h) in [(80, 24), (120, 40), (200, 50), (60, 30), (240, 30)] {
            let size = Size::new(w, h);
            let mut g = SpacetimeGrid::default();
            let mut ctx = Ctx::new(8, size);
            g.resize(size, &mut ctx);
            let cycle = g.binary().cycle();
            for _ in 0..(cycle / (4.0 * SIM_DT)) as usize {
                g.update(4.0 * SIM_DT, &mut ctx);
                for y in [0, i32::from(h) - 1] {
                    for x in 0..i32::from(w) {
                        assert_eq!(g.mesh.cell_mask(x, y), Some(0), "{w}×{h}: mesh at {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn spt_a9_the_black_holes_sit_in_deep_funnels_and_outshine_the_mesh() {
        let size = Size::new(80, 24);
        let mut g = SpacetimeGrid::default();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        let bin = g.binary();
        let lift = RIM_LIFT * (bin.m1 + bin.m2);
        let mut frames = 0;
        for k in 0..400 {
            g.t = bin.cycle() * f64::from(k) / 400.0;
            g.refresh();
            if bin.dip(g.t) < 1.0 {
                continue;
            }
            frames += 1;
            let masses = bin.masses(g.t);
            // The funnel: the sheet under the heavier mass lies at least two rows below
            // the sheet's height half a unit away from it (the pair's own dip included).
            let (x, y, _) = masses[0];
            let rim = well_depth(x + 0.5, y, &masses).max(well_depth(x - 0.5, y, &masses));
            let rows = g.cam.project(x, y, lift + well_depth(x, y, &masses)).1
                - g.cam.project(x, y, lift + rim).1;
            assert!(rows >= 2.0, "t={}: funnel {rows} rows deep", g.t);
            // The dark cores occlude the mesh.
            for b in g.bodies_now(bin).iter().flatten() {
                let (cx, cy, r) = b.hole();
                let reach = r.ceil() as i32;
                for oy in -reach..=reach {
                    for ox in -reach..=reach {
                        let (px, py) = (cx.round() as i32 + ox, cy.round() as i32 + oy);
                        let d = (f64::from(px) - cx).hypot(f64::from(py) - cy);
                        if d < r - 1.5 {
                            assert!(!g.mesh.is_lit(px, py), "t={}: mesh in a core", g.t);
                        }
                    }
                }
            }
            // In its funnel (within two moat radii, about three disc radii) the rim
            // outshines the mesh by half again (mono greys follow the level).
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let rim = canvas
                .cells()
                .iter()
                .filter(|c| c.ink.role == Role::Blackbody)
                .map(|c| c.ink.level)
                .max()
                .unwrap_or(0);
            let holes: Vec<_> = g.bodies_now(bin).iter().flatten().map(Body::hole).collect();
            for y in 0..i32::from(size.height()) {
                for x in 0..i32::from(size.width()) {
                    let (dx, dy) = Dots::from_cells(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let near = holes
                        .iter()
                        .any(|&(cx, cy, r)| (dx - cx).hypot(dy - cy) < 2.0 * r);
                    let cell = canvas.get(x, y).expect("on canvas");
                    if near && cell.ink.role == MESH_ROLE {
                        assert!(
                            2 * u32::from(rim) >= 3 * u32::from(cell.ink.level),
                            "t={}: rim {rim}, mesh {} at {x},{y}",
                            g.t,
                            cell.ink.level
                        );
                    }
                }
            }
        }
        assert!(frames > 300, "{frames}");
        // The camera fit's lowest point is the remnant's lifted well bottom.
        let bin = Binary::DEFAULT;
        let remnant = bin.masses(bin.merge_time() + 1.0);
        let bottom = well_depth(0.0, 0.0, &remnant) + RIM_LIFT * (bin.m1 + bin.m2);
        assert!((DEEPEST - bottom).abs() < 1e-12, "{DEEPEST} vs {bottom}");
    }

    #[test]
    fn spt_a10_the_chirp_and_the_burst_light_up_the_sheet() {
        let size = Size::new(120, 40);
        let mut g = SpacetimeGrid::default();
        let mut ctx = Ctx::new(5, size);
        g.resize(size, &mut ctx);
        let bin = g.binary();
        // Mean and brightest on-screen mesh level over 40 frames of cycle time `from..to`.
        let mut over = |from: f64, to: f64| {
            let (mut mean, mut top) = (0.0, 0u8);
            for k in 0..40 {
                g.t = 3.0 * bin.cycle() + from + (to - from) * f64::from(k) / 40.0;
                g.refresh();
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                g.render(&mut canvas);
                let mesh = canvas
                    .cells()
                    .iter()
                    .filter(|c| c.ink.role == MESH_ROLE)
                    .map(|c| c.ink.level);
                let (sum, n) = mesh.fold((0.0, 0), |(s, n), l| {
                    top = top.max(l);
                    (s + f64::from(l), n + 1)
                });
                mean += sum / f64::from(n.max(1)) / 40.0;
            }
            (mean, top)
        };
        let (t_i, t_m) = (bin.inspiral_time(), bin.merge_time());
        let (quiet, quiet_top) = over(SWITCH_ON, 0.5 * t_i);
        let (late, _) = over(t_i - 3.0, t_i);
        let (burst, burst_top) = over(t_m, t_m + 3.0);
        // The chirp brightens the sheet, and the burst after coalescence outshines the
        // inspiral by 40 % on average; its crests climb the ramp far past the rest level.
        assert!(late > quiet, "inspiral {quiet} → {late}");
        assert!(burst >= 1.4 * quiet, "inspiral {quiet}, burst {burst}");
        assert!(
            burst_top >= 180 && quiet_top >= 120,
            "{quiet_top} / {burst_top}"
        );
    }

    #[test]
    fn spt_a8_the_merger_is_the_brightest_moment() {
        let b = Binary::DEFAULT;
        let t_m = b.merge_time();
        assert!((b.flash(t_m) - 1.0).abs() < 1e-9);
        for k in 0..2000 {
            let t = b.cycle() * f64::from(k) / 2000.0;
            let f = b.flash(t);
            assert!((0.0..=1.0).contains(&f), "t={t}");
            if t < b.inspiral_time() || t > t_m + 3.0 {
                assert!(f < 1e-4, "flash {f} at t={t}");
            }
        }
        // The mesh is brightest at coalescence: same segment, flash on versus off.
        let p = Pt {
            well: -0.3,
            crest: 0.5,
            envn: 1.0,
            ..Pt::default()
        };
        let calm = mesh_ink(&p, &p, (1.0, 2.0), 1.0, MESH_MAX);
        let lit = mesh_ink(
            &p,
            &p,
            (1.0, 2.0),
            1.0 + FLASH_GAIN * b.flash(t_m),
            MESH_MAX,
        );
        assert!(lit.level > calm.level);
    }

    #[test]
    fn spt_mesh_heights_match_the_field() {
        let mut g = SpacetimeGrid::default();
        let mut ctx = Ctx::new(3, Size::new(120, 40));
        g.resize(Size::new(120, 40), &mut ctx);
        for _ in 0..30 {
            g.update(SIM_DT, &mut ctx);
        }
        let bin = g.binary();
        let masses = bin.masses(g.t);
        for idx in (0..g.pts.len()).step_by(37) {
            let b = g.base[idx];
            let [x, y, r, ..] = b.get();
            let gain = sheet_gain(bin.state(g.t - r / WAVE_SPEED).speed, g.max_speed);
            let expect = well_depth(x, y, &masses) + gain * wave(&bin, r, y.atan2(x), g.t);
            let p = g.pts[idx];
            let z = p.well + WAVE_AMP * p.envn * p.crest;
            assert!((z - expect).abs() < 1e-6, "sample {idx}");
        }
    }

    #[test]
    fn spt_mesh_resolution_adapts_and_stays_a_wireframe() {
        let lines = |w, h| {
            let mut g = SpacetimeGrid::default();
            let mut ctx = Ctx::new(1, Size::new(w, h));
            g.resize(Size::new(w, h), &mut ctx);
            (g.nx, g.ny)
        };
        let (small, big) = (lines(80, 24), lines(200, 50));
        assert!(big.0 > small.0 && big.1 > small.1, "{small:?} vs {big:?}");
        let mut stage = Stage::new(build(), 42, Size::new(120, 40), GlyphMode::Unicode);
        for _ in 0..60 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let has = |r: Role| frame.cells().iter().any(|c| c.ink.role == r);
        assert!(has(MESH_ROLE) && has(Role::Blackbody));
        assert!(has(Role::Ramp(Ramp::Aurora)), "strain inset");
    }
}
