//! An oblique rotator: a neutron star whose tilted magnetic axis sweeps two radio beams
//! through space, seen from the side, with the pulses we receive stacked in an inset
//! (SPEC-002 `pulsar`).
//!
//! World axes: x right, y up, z towards the viewer; lengths in units of the light-cylinder
//! radius `R_LC = cP/2π` (not to scale: CP 1919's is ~6400 star radii). The view is
//! orthographic. The spin axis leans `90° − ζ` towards us, the magnetic axis is `α` off it,
//! so each turn the beam sweeps a cone that brushes our line of sight once.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::geom::ASPECT;
use sciart_core::geom::v3::{V3, add, cross, dot, norm, scale, sub, unit};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, draw, ink, raster, rng};

pub static META: Meta = Meta {
    name: "pulsar",
    category: Category::DeepCosmos,
    description: "An oblique rotator sweeping radio beams, with the pulses it sends stacked in an inset",
    title: "Pulsar · lighthouse beams sweep",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Pulsar::default())
}

/// Spin period of CP 1919, the first pulsar found (s), played in real time.
pub(crate) const PERIOD: f64 = 1.3373;
/// Magnetic obliquity `α` and viewing angle `ζ` (from the spin axis); the impact
/// parameter `β = ζ − α` is 10°.
const ALPHA: f64 = 50.0 * PI / 180.0;
const ZETA: f64 = 60.0 * PI / 180.0;
/// Beam half-opening angle `ρ`.
pub(crate) const RHO: f64 = 16.0 * PI / 180.0;
/// Star radius, in `R_LC` (enlarged to a visible dot).
const R_STAR: f64 = 0.07;
/// Closed dipole loops (apex radius, `R_LC`) and the open lines' apex beyond the cylinder.
const CLOSED: [f64; 2] = [0.4, 0.8];
const OPEN: f64 = 2.5;
/// Open lines end this far from the star (`R_LC`), fading to nothing there (PUL-A7).
const OPEN_REACH: f64 = 0.8;
/// Magnetic meridians the closed loops and the open lines are drawn in: few loops, so the
/// co-rotating magnetosphere reads as nested shells rather than a tangle.
const CLOSED_MERIDIANS: usize = 4;
const OPEN_MERIDIANS: usize = 6;
/// Samples per field line.
const LINE_STEPS: usize = 48;
/// Half-height of the drawn light cylinder, `R_LC`.
const CYLINDER_H: f64 = 1.3;
/// Beam rays per cone, samples along each ray, and the exposure's e-folding time (s).
const RAYS: usize = 160;
const RAY_STEPS: usize = 150;
const BEAM_TAU: f64 = 0.06;
/// How fast the ray pattern turns about each beam axis (rad/s): fast enough that
/// consecutive frames interleave, which hides the discrete rays.
const RAY_TURN: f64 = 37.0;
/// Each ray's samples start at a hashed offset of up to one sample step, new every
/// exposure, so the samples of neighbouring rays never line up into radial spokes.
const RAY_JITTER_SALT: u64 = 0x5075_6C73_6172_0001;
/// Length of the flash's horizontal spikes at the pulse peak, in `R_LC`.
const FLASH_SPIKE: f64 = 1.6;
/// Beam tone: exposure at the knee and at white, and the level below which the beam's
/// faint outer cone dissolves into black space (the Plasma ramp's floor is indigo).
const BEAM_KNEE: f32 = 0.4;
const BEAM_WHITE: f32 = 60.0;
const BEAM_FLOOR: u8 = 24;
/// Exposure steps of the first frame after a resize, so it opens on a smooth beam rather
/// than one frame's rays.
const PRE_EXPOSURE: usize = 6;
/// Rows per `R_LC`, as a fraction of `draw::fit_radius(size, 1)`.
const ZOOM: f64 = 0.55;
/// Pulse stack: stored pulses, samples per pulse, and the phase window they show.
const PULSES: usize = 16;
const SAMPLES: usize = 80;
pub(crate) const WINDOW: f64 = 70.0 * PI / 180.0;
/// Drifting sub-pulses: count, spacing and width (rad), drift per pulse (in spacings).
const SUBPULSES: usize = 3;
const SUB_SPACING: f64 = 11.0 * PI / 180.0;
const SUB_WIDTH: f64 = 5.0 * PI / 180.0;
/// `1/SUB_DRIFT ≈ 4.2` pulses to march one spacing: CP 1919's `P₃` (Backer 1973).
const SUB_DRIFT: f64 = 0.24;
/// Receiver noise (fraction of a strong pulse).
const NOISE: f64 = 0.04;
/// The inset: largest size in cells, and the smallest canvas that shows it.
const INSET_MAX: (u16, u16) = (40, 12);
const INSET_MIN_CANVAS: (u16, u16) = (60, 16);
/// The beam's brightness under the inset, so the pulse traces read over it, and the
/// width (pixels) of the feathered band across the inset's border over which it dims.
const INSET_DIM: f64 = 0.2;
/// Pulse stack spacing: at most one trace per `STACK_ROW_DOTS` dot rows and per
/// `STACK_COL_DOTS` dot columns of the inset, and a pulse's height `STACK_AMP` gaps, so the
/// traces overlap like the CP 1919 plot without piling into one band.
const STACK_ROW_DOTS: i32 = 6;
const STACK_COL_DOTS: i32 = 8;
const STACK_AMP: f64 = 3.5;
const INSET_FEATHER: f64 = 2.0;
/// A beam whose axis points at us (`axis·ẑ` from `FACING.0` to `FACING.1`) is dimmed by
/// up to `FACING_DIM` and shortened by up to `FACING_SHORTEN`.
const FACING: (f64, f64) = (0.3, 0.9);
const FACING_DIM: f64 = 0.75;
const FACING_SHORTEN: f64 = 0.75;
/// Below this width only the inner closed loop is drawn per meridian: the outer one and
/// the cylinder crowd the star into a tangle of thin lines.
const OUTER_LOOP_MIN_WIDTH: u16 = 120;
/// Levels: closed field lines (by depth; each loop further out dimmer by
/// `CLOSED_FALLOFF`), open lines, axis and cylinder, pulse stack.
const CLOSED_LEVEL: (f64, f64) = (80.0, 150.0);
const CLOSED_FALLOFF: f64 = 0.65;
const OPEN_LEVEL: (f64, f64) = (150.0, 200.0);
const GUIDE_LEVEL: u8 = 70;
/// The star: a white-hot core pixel disc and a softer halo around it (radii in pixels).
const STAR_CORE: (f64, u8) = (0.8, 255);
const STAR_HALO: (f64, u8) = (1.9, 205);
const PULSE_LEVEL: (f64, f64) = (110.0, 200.0);
const PULSE_FRONT: u8 = 240;
/// The flash's spikes, from the star out: the share of the spike's length each part
/// reaches, and its ink, starting at the top of the blackbody scale (level 255,
/// `ink::BLACKBODY_MAX_K`; a real surface is ~10⁶ K).
const SPIKE_TAPER: [(f64, Ink); 3] = [
    (0.3, Ink::new(Role::Blackbody, 255)),
    (0.65, Ink::new(Role::Ramp(Ramp::Plasma), 210)),
    (1.0, Ink::new(Role::Ramp(Ramp::Plasma), 140)),
];
const READOUT: &str = "P 1.337 s";
const ASCII_RAMP: [char; 5] = ['.', ':', '+', '*', '@'];
const ASCII_MIN_LEVEL: u8 = 60;

/// Spin axis `Ω̂ = (0, sin ζ, cos ζ)`.
pub(crate) fn spin_axis() -> V3 {
    [0.0, ZETA.sin_det(), ZETA.cos_det()]
}

/// Rotation phase at time `t`.
pub(crate) fn phase(t: f64) -> f64 {
    TAU * t / PERIOD
}

/// Magnetic axis at rotation phase `φ`: `cos α Ω̂ + sin α (cos φ x̂ + sin φ ê₂)`,
/// `ê₂ = Ω̂ × x̂` (PUL-A1).
pub(crate) fn magnetic_axis(phi: f64) -> V3 {
    let e2 = cross(spin_axis(), [1.0, 0.0, 0.0]);
    let (s, c) = phi.sin_cos_det();
    add(
        scale(spin_axis(), ALPHA.cos_det()),
        scale(add([c, 0.0, 0.0], scale(e2, s)), ALPHA.sin_det()),
    )
}

/// Angle between a unit vector and our line of sight `ẑ`.
pub(crate) fn los_angle(v: V3) -> f64 {
    v[2].clamp(-1.0, 1.0).acos()
}

/// Whether we receive the pulse at time `t` (PUL-A2).
#[cfg(test)]
pub(crate) fn receiving(t: f64) -> bool {
    los_angle(magnetic_axis(phase(t))) < RHO
}

/// Rotation phase at the centre of the received pulse, where `m` passes closest to `ẑ`.
pub(crate) const PULSE_PHASE: f64 = -PI / 2.0;

/// Unit vectors `(u₁, u₂)` completing `m` to a right-handed frame.
fn frame(m: V3) -> (V3, V3) {
    let u1 = unit(cross(spin_axis(), m));
    (u1, cross(m, u1))
}

/// Point of the dipole field line `r = L sin²θ` at magnetic colatitude `θ`, in the
/// meridian at magnetic azimuth `ψ` about the axis `m` (PUL-A3).
pub(crate) fn field_point(l: f64, theta: f64, psi: f64, m: V3) -> V3 {
    let (u1, u2) = frame(m);
    let r = l * theta.sin_det().powi(2);
    let (sp, cp) = psi.sin_cos_det();
    let across = add(scale(u1, cp), scale(u2, sp));
    scale(
        add(scale(across, theta.sin_det()), scale(m, theta.cos_det())),
        r,
    )
}

/// The closed loops drawn on a canvas `width` cells wide (PUL-A3): the outer loop only
/// from `OUTER_LOOP_MIN_WIDTH`.
pub(crate) fn closed_loops(width: u16) -> &'static [f64] {
    if width < OUTER_LOOP_MIN_WIDTH {
        &CLOSED[..1]
    } else {
        &CLOSED
    }
}

/// Distance of `p` from the spin axis.
pub(crate) fn cylinder_radius(p: V3) -> f64 {
    norm(sub(p, scale(spin_axis(), dot(p, spin_axis()))))
}

/// Geometric pulse profile at phase `φ`: `exp(−γ²/2σ²)` with `γ` the angle between the
/// beam axis and our line of sight, `σ = ρ/2` (PUL-A4).
pub(crate) fn template(phi: f64) -> f64 {
    let g = los_angle(magnetic_axis(phi)) / (0.5 * RHO);
    (-0.5 * g * g).exp()
}

/// Phase of stack sample `i`, the window centred on the pulse.
pub(crate) fn sample_phase(i: usize) -> f64 {
    PULSE_PHASE + ((i as f64 + 0.5) / SAMPLES as f64 - 0.5) * WINDOW
}

/// One received pulse, number `n`: the geometric profile carried by drifting sub-pulses
/// with random amplitudes, plus receiver noise (PUL-A4).
pub(crate) fn pulse(n: i64, rng: &mut impl Rng, out: &mut [f32; SAMPLES]) {
    let mut amp = [0.0; SUBPULSES];
    for a in &mut amp {
        *a = rng.random_range(0.4..1.0);
    }
    // Sub-pulses march through the window by `SUB_DRIFT` spacings per pulse and wrap.
    let drift = (n as f64 * SUB_DRIFT).rem_euclid(1.0);
    for (i, o) in out.iter_mut().enumerate() {
        let phi = sample_phase(i);
        let mut sub = 0.0;
        for (k, a) in amp.iter().enumerate() {
            let c = PULSE_PHASE + (k as f64 + drift - 1.0) * SUB_SPACING;
            let d = (phi - c) / SUB_WIDTH;
            sub += a * (-d * d).exp();
        }
        let noise = NOISE * rng.random_range(-1.0..1.0);
        *o = (template(phi) * (0.35 + sub) + noise) as f32;
    }
}

/// Pulse-stack layout for an inset `dw × dh` dots: `(lines, amp, gap)` in dot rows,
/// `lines = clamp(min(PULSES, dh/6, dw/8), 2, PULSES)`, `amp = 3.5·gap`, and the stack
/// (the oldest trace's peak to the newest baseline) filling `dh − 2` (PUL-A5).
pub(crate) fn stack_layout(dw: i32, dh: i32) -> (usize, f64, f64) {
    let lines = (dh / STACK_ROW_DOTS)
        .min(dw / STACK_COL_DOTS)
        .clamp(2, PULSES as i32) as usize;
    let gap = f64::from((dh - 2).max(1)) / ((lines - 1) as f64 + STACK_AMP);
    (lines, STACK_AMP * gap, gap)
}

/// Where the pulse stack goes: the top-right corner, clear of the title (PUL-A5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Inset {
    /// Top-left cell and size in cells.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Inset {
    pub(crate) fn fit(size: Size) -> Option<Self> {
        let (w, h) = (size.width(), size.height());
        if w < INSET_MIN_CANVAS.0 || h < INSET_MIN_CANVAS.1 {
            return None;
        }
        // A fifth of the width and a quarter of the height: at 80×24 a 16×6 stack that
        // leaves the beam's sweep clear.
        let iw = (w / 5).min(INSET_MAX.0);
        let ih = (h / 4).min(INSET_MAX.1);
        Some(Self {
            x: i32::from(w - iw - 1),
            y: 1,
            w: i32::from(iw),
            h: i32::from(ih),
        })
    }
}

#[derive(Debug)]
pub struct Pulsar {
    t: f64,
    size: Size,
    /// Canvas centre (cells) and rows per `R_LC`.
    centre: (f64, f64),
    zoom: f64,
    /// Beam light: exposure, tone and the pixels it develops into.
    beams: Glow,
    tone: Tone,
    pixels: Pixels,
    /// Field lines, guides, the star and the pulse stack.
    dots: Dots,
    inset: Option<Inset>,
    /// Ring of received pulses; `newest` is the slot of the latest.
    pulses: [[f32; SAMPLES]; PULSES],
    newest: usize,
    /// Number of the latest pulse (the rotation it arrived in).
    pulse_no: i64,
    /// Whether the stack has been filled (by the first resize).
    primed: bool,
    /// Exposures so far: keys each exposure's ray jitter.
    exposures: u64,
    readout: bool,
}

impl Default for Pulsar {
    fn default() -> Self {
        Self {
            t: 0.0,
            size: Size::new(1, 1),
            centre: (0.0, 0.0),
            zoom: 1.0,
            beams: Glow::default(),
            tone: Tone::asinh(BEAM_KNEE, BEAM_WHITE),
            pixels: Pixels::default(),
            dots: Dots::default(),
            inset: None,
            pulses: [[0.0; SAMPLES]; PULSES],
            newest: 0,
            pulse_no: 0,
            primed: false,
            exposures: 0,
            readout: false,
        }
    }
}

/// ASCII stand-in for a beam pixel pair, by the brighter pixel's level; the faint fringe
/// is left out, or ASCII mode would fill the sky with dots.
fn ascii_pixel(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let ink = raster::stronger(top, bottom);
    (ink.role != Role::Background && ink.level >= ASCII_MIN_LEVEL).then(|| {
        let i = usize::from(ink.level) * ASCII_RAMP.len() / 256;
        (ASCII_RAMP[i], ink)
    })
}

fn plasma(level: f64) -> Ink {
    Ink::new(Role::Ramp(Ramp::Plasma), ink::level(level))
}

impl Pulsar {
    /// Continuous dot coordinates of the world point `p`.
    fn dot(&self, p: V3) -> (f64, f64) {
        let (cx, cy) = self.centre;
        Dots::from_cells(cx + p[0] * self.zoom * ASPECT, cy - p[1] * self.zoom)
    }

    fn line(&mut self, a: V3, b: V3, ink: Ink) {
        let ((x0, y0), (x1, y1)) = (self.dot(a), self.dot(b));
        self.dots.line(x0, y0, x1, y1, ink);
    }

    /// The pulse number of rotation phase `φ`: a new pulse starts half a turn before the
    /// pulse centre, so the whole window of one pulse belongs to one number.
    fn pulse_number(phi: f64) -> i64 {
        ((phi - PULSE_PHASE + PI) / TAU).floor() as i64
    }

    /// Draw the spin axis, dashed (every other segment of 24), fading toward its ends. The
    /// light cylinder is not outlined: a dashed silhouette read as a ghost box round the
    /// star; the open field lines ending on it mark it instead (PUL-A7).
    fn guides(&mut self) {
        let axis = spin_axis();
        let steps = 24;
        for i in (0..steps).step_by(2) {
            let s = |i: usize| CYLINDER_H * (2.0 * i as f64 / steps as f64 - 1.0);
            let (a, b) = (scale(axis, s(i)), scale(axis, s(i + 1)));
            let fade = 1.0 - (s(i) + s(i + 1)).abs() / (2.0 * CYLINDER_H);
            let level = f64::from(GUIDE_LEVEL) * fade;
            if level >= ink::MIN_VISIBLE_LEVEL {
                self.line(a, b, Ink::new(Role::Dim, ink::level(level)));
            }
        }
    }

    /// Closed loops inside the light cylinder and open lines out through it, co-rotating
    /// with `m`; nearer segments are brighter, and outer loops dimmer.
    fn field(&mut self, m: V3) {
        let depth_level = |(lo, hi): (f64, f64), z: f64| lo + (hi - lo) * (0.5 + 0.5 * z);
        let loops = closed_loops(self.size.width());
        for j in 0..CLOSED_MERIDIANS {
            // Offset half a step, so no loop lies edge-on along the beam.
            let psi = TAU * (j as f64 + 0.5) / CLOSED_MERIDIANS as f64;
            let mut dim = 1.0;
            for &l in loops {
                // From the star's surface on one side to the other: r = L sin²θ ≥ R_STAR.
                let th0 = (R_STAR / l).sqrt().asin();
                let mut prev = field_point(l, th0, psi, m);
                for i in 1..=LINE_STEPS {
                    let th = th0 + (PI - 2.0 * th0) * i as f64 / LINE_STEPS as f64;
                    let p = field_point(l, th, psi, m);
                    let z = (prev[2] + p[2]) / (2.0 * l);
                    self.line(prev, p, plasma(dim * depth_level(CLOSED_LEVEL, z)));
                    prev = p;
                }
                dim *= CLOSED_FALLOFF;
            }
        }
        for j in 0..OPEN_MERIDIANS {
            let psi = TAU * j as f64 / OPEN_MERIDIANS as f64;
            // Open lines from both polar caps, stopped at the light cylinder.
            let th0 = (R_STAR / OPEN).sqrt().asin();
            for (start, dir) in [(th0, 1.0), (PI - th0, -1.0)] {
                let mut prev = field_point(OPEN, start, psi, m);
                for i in 1..=LINE_STEPS {
                    let th = start + dir * 0.5 * PI * i as f64 / LINE_STEPS as f64;
                    let p = field_point(OPEN, th, psi, m);
                    // Stopped at the light cylinder and within `OPEN_REACH` of the star,
                    // fading out along the way: a line seen end-on is a short fading
                    // stroke, never a spike across the frame (PUL-A7).
                    let r = norm(p);
                    if cylinder_radius(p) > 1.0 || r > OPEN_REACH {
                        break;
                    }
                    let fade = (1.0 - r / OPEN_REACH).sqrt();
                    let level = fade * depth_level(OPEN_LEVEL, (prev[2] + p[2]) / 2.0);
                    if level >= ink::MIN_VISIBLE_LEVEL {
                        self.line(prev, p, plasma(level));
                    }
                    prev = p;
                }
            }
        }
    }

    /// Expose both beams for `dt`: rays spread over each cone (a sunflower pattern, turned
    /// a little every frame so the exposure smooths it into a glow instead of spokes),
    /// marched out from the star; brightest on the axis and near the star.
    fn expose(&mut self, m: V3, dt: f64, t: f64) {
        self.beams.decay((-dt / BEAM_TAU).exp() as f32);
        self.exposures = self.exposures.wrapping_add(1);
        let salt = RAY_JITTER_SALT ^ self.exposures.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let reach = {
            let (w, h) = (f64::from(self.size.width()), f64::from(self.size.height()));
            (w / ASPECT).hypot(h) / self.zoom
        };
        let golden = PI * (3.0 - 5f64.sqrt());
        let turn = RAY_TURN * t;
        let edge = (-3.0_f64).exp();
        for axis in [m, scale(m, -1.0)] {
            // A beam aimed at us fans its whole length out round the star, where its few
            // outer rays read as spokes: dim and shorten it as it turns our way, so the
            // splash stays a glow round the flash rather than a flood of the frame.
            let facing = ease::smoothstep((axis[2] - FACING.0) / (FACING.1 - FACING.0));
            let w0 = (dt / BEAM_TAU * (1.0 - FACING_DIM * facing)) as f32;
            let length = reach * (1.0 - FACING_SHORTEN * facing);
            let (u1, u2) = frame(axis);
            for (j, key) in (0..RAYS).zip(0u64..) {
                let f = ((j as f64 + 0.5) / RAYS as f64).sqrt();
                let (s, c) = (golden * j as f64 + turn).sin_cos_det();
                let (sa, ca) = (f * RHO).sin_cos_det();
                let d = add(scale(axis, ca), scale(add(scale(u1, c), scale(u2, s)), sa));
                // Brightest on the axis, falling to exactly zero at the cone's edge, so the
                // fringe fades out before it dissolves.
                let across = w0 * (((-3.0 * f * f).exp() - edge) / (1.0 - edge)) as f32;
                let jitter = rng::hash01(salt ^ (key << 1 | u64::from(axis[2] < m[2])));
                for i in 0..RAY_STEPS {
                    let r = R_STAR + length * (i as f64 + jitter) / RAY_STEPS as f64;
                    let (x, y) = self.dot(scale(d, r));
                    // Bilinear (CORE-R92): a pixel is 2 × 2 dots, its centre at dot
                    // `2p + 0.5`, so no sample snaps to the nearest pixel.
                    self.beams.splat_bilinear(
                        (x - 0.5) / 2.0,
                        (y - 0.5) / 2.0,
                        across / (1.0 + 2.0 * r as f32),
                    );
                }
            }
        }
    }

    /// The received pulse as seen by us: while the beam faces us the star flares, drawn
    /// as a telescope's four diffraction spikes whose length follows the geometric
    /// profile `template(φ)` (PUL-A6).
    fn flash(&mut self) {
        let a = template(phase(self.t));
        let len = FLASH_SPIKE * a;
        if len * self.zoom < 0.5 {
            return;
        }
        for dir in [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
        ] {
            // Spikes are equal on screen: the vertical ones span half the rows. Each fades
            // outwards (white-hot, then the beam's pale and mid violet), a diffraction
            // spike rather than a crosshair.
            let k = if dir[1] == 0.0 {
                len
            } else {
                0.5 * len * ASPECT
            };
            let mut from = 0.0;
            for (to, ink) in SPIKE_TAPER {
                self.line(scale(dir, from * k), scale(dir, to * k), ink);
                from = to;
            }
        }
    }

    /// The star: a white-hot core at the top of the ramp in a softer halo, a point of
    /// light rather than a disc (it is ~10 km across).
    fn star(&mut self) {
        let (x, y) = (self.centre.0 - 0.5, 2.0 * self.centre.1 - 0.5);
        let n = STAR_HALO.0.ceil() as i32;
        for dy in -n..=n {
            for dx in -n..=n {
                let r2 = f64::from(dx * dx + dy * dy);
                let level = if r2 <= STAR_CORE.0 * STAR_CORE.0 {
                    STAR_CORE.1
                } else if r2 <= STAR_HALO.0 * STAR_HALO.0 {
                    STAR_HALO.1
                } else {
                    continue;
                };
                let (px, py) = (
                    (x + f64::from(dx)).round() as i32,
                    (y + f64::from(dy)).round() as i32,
                );
                if self.pixels.get(px, py).is_some_and(|i| i.level < level) {
                    self.pixels.set(px, py, plasma(f64::from(level)));
                }
            }
        }
    }

    /// Keep the ring of pulses current: one new pulse per turn, the newest revealed as
    /// the beam sweeps past.
    fn receive(&mut self, rng: &mut impl Rng) {
        let n = Self::pulse_number(phase(self.t));
        while self.pulse_no < n {
            self.pulse_no += 1;
            self.newest = (self.newest + 1) % PULSES;
            pulse(self.pulse_no, rng, &mut self.pulses[self.newest]);
        }
    }

    /// The pulse stack (a floating horizon: nearer lines hide the ones behind), newest at
    /// the front and revealed up to the current phase; the beam's glow shows through it.
    fn stack(&mut self) {
        let Some(inset) = self.inset else { return };
        let (dw, dh) = (inset.w * Dots::CELL_W, inset.h * Dots::CELL_H);
        let (x0, y0) = (inset.x * Dots::CELL_W, inset.y * Dots::CELL_H);
        let (lines, amp, gap) = stack_layout(dw, dh);
        let phi = phase(self.t) - PULSE_PHASE;
        // Samples of the newest pulse received so far.
        let into = (phi + PI).rem_euclid(TAU) - PI;
        let shown = (((into / WINDOW + 0.5) * SAMPLES as f64).floor()).clamp(0.0, SAMPLES as f64);
        let mut horizon = [f64::INFINITY; 2 * INSET_MAX.0 as usize];
        for age in 0..lines {
            let slot = (self.newest + PULSES - age) % PULSES;
            let base = f64::from(y0) + f64::from(dh - 2) - age as f64 * gap;
            let ink = if age == 0 {
                Ink::new(Role::Ramp(Ramp::Plasma), PULSE_FRONT)
            } else {
                let f = age as f64 / lines as f64;
                plasma(PULSE_LEVEL.1 + (PULSE_LEVEL.0 - PULSE_LEVEL.1) * f)
            };
            let mut prev: Option<(f64, f64)> = None;
            for i in 0..dw {
                let s = (i as usize * SAMPLES) / dw as usize;
                let hx = i as usize;
                let visible_part = age > 0 || (s as f64) < shown;
                let y = base - amp * f64::from(self.pulses[slot][s]).max(0.0);
                let x = f64::from(x0 + i);
                let seen = visible_part && y < horizon[hx];
                if seen {
                    match prev {
                        Some((px, py)) => self.dots.line(px, py, x, y, ink),
                        None => self.dots.plot(x, y, ink),
                    }
                    prev = Some((x, y));
                } else {
                    prev = None;
                }
                if visible_part {
                    horizon[hx] = horizon[hx].min(y);
                }
            }
        }
    }

    /// Everything for the current time (the M10 raster convention: `render` only blits).
    fn rasterise(&mut self, dt: f64) {
        let m = magnetic_axis(phase(self.t));
        self.expose(m, dt, self.t);
        self.develop();
        self.draw_lines(m);
    }

    /// The beam exposure as pixels, its faint outer cone dissolved into black space, and
    /// the star on top.
    fn develop(&mut self) {
        let tone = &self.tone;
        self.beams.to_pixels(&mut self.pixels, tone, |l| {
            Ink::new(Role::Ramp(Ramp::Plasma), l)
        });
        self.pixels.dissolve(BEAM_FLOOR);
        self.dim_inset();
        self.star();
    }

    /// Dim the beam under the pulse stack to `INSET_DIM` of its level, so the traces stay
    /// legible when the beam sweeps through the corner, while the glow still shows. The
    /// dimming is feathered across the border, so it reads as a veil, not a pasted panel.
    fn dim_inset(&mut self) {
        let Some(inset) = self.inset else { return };
        let (x0, x1) = (f64::from(inset.x), f64::from(inset.x + inset.w));
        let (y0, y1) = (f64::from(2 * inset.y), f64::from(2 * (inset.y + inset.h)));
        let reach = INSET_FEATHER.ceil() as i32;
        for py in 2 * inset.y - reach..2 * (inset.y + inset.h) + reach {
            for px in inset.x - reach..inset.x + inset.w + reach {
                let Some(ink) = self
                    .pixels
                    .get(px, py)
                    .filter(|i| i.role != Role::Background)
                else {
                    continue;
                };
                // Signed depth (pixels) of the pixel's centre inside the inset.
                let (cx, cy) = (f64::from(px) + 0.5, f64::from(py) + 0.5);
                let depth = (cx - x0).min(x1 - cx).min(cy - y0).min(y1 - cy);
                let veil = ease::smoothstep((depth + INSET_FEATHER) / (2.0 * INSET_FEATHER));
                let level = f64::from(ink.level) * ease::lerp(1.0, INSET_DIM, veil);
                self.pixels.set(px, py, Ink::new(ink.role, level as u8));
            }
        }
    }

    /// Guides, field lines, the flash and the pulse stack, as dots.
    fn draw_lines(&mut self, m: V3) {
        self.dots.clear();
        self.guides();
        self.field(m);
        self.flash();
        self.stack();
    }
}

impl Generator for Pulsar {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.centre = (
            f64::from(size.width()) / 2.0,
            f64::from(size.height()) / 2.0,
        );
        self.zoom = ZOOM * draw::fit_radius(size, 1.0);
        self.beams.resize(size);
        self.pixels.resize(size);
        self.dots.resize(size);
        self.inset = Inset::fit(size);
        self.readout = READOUT.len() <= title::readout_room(size, &Title::of(self));
        if !self.primed {
            self.primed = true;
            // A full stack from the start: the pulses of the turns before t = 0.
            self.pulse_no = Self::pulse_number(phase(self.t)) - PULSES as i64;
            self.receive(&mut ctx.rng);
        }
        // Expose the last 3τ of the sweep in steps, so the first frame already shows a
        // smooth beam instead of one step's rays.
        let dt = 3.0 * BEAM_TAU / PRE_EXPOSURE as f64;
        for k in (0..PRE_EXPOSURE).rev() {
            let t = self.t - dt * k as f64;
            self.expose(magnetic_axis(phase(t)), dt, t);
        }
        self.develop();
        self.draw_lines(magnetic_axis(phase(self.t)));
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt >= 0.0) {
            return;
        }
        self.t += dt;
        self.receive(&mut ctx.rng);
        self.rasterise(dt);
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_pixel(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
        if self.readout {
            let y = i32::from(self.size.height()) - 1;
            let ink = Ink::new(Role::Text, title::READOUT_LEVEL);
            draw::label_right(canvas, y, READOUT, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEG: f64 = PI / 180.0;

    #[test]
    fn pul_a5_stack_lines_are_capped_and_spaced_by_their_amplitude() {
        for size in [Size::new(80, 24), Size::new(120, 40), Size::new(200, 50)] {
            let inset = Inset::fit(size).unwrap();
            let (dw, dh) = (inset.w * Dots::CELL_W, inset.h * Dots::CELL_H);
            let (lines, amp, gap) = stack_layout(dw, dh);
            let cap = (dh / 6).min(dw / 8).clamp(2, PULSES as i32) as usize;
            assert_eq!(lines, cap, "{size:?}");
            assert!((amp / gap - 3.5).abs() < 1e-9);
            // Oldest peak to newest baseline fills the inset exactly.
            let height = (lines - 1) as f64 * gap + amp;
            assert!((height - f64::from(dh - 2)).abs() < 1e-9, "{size:?}");
            assert!(gap >= 3.0, "{size:?}: traces {gap} dots apart");
        }
        // Degenerate insets never panic and keep two lines.
        assert_eq!(stack_layout(0, 0).0, 2);
    }

    #[test]
    fn pul_a7_no_cylinder_box_and_no_line_spikes_across_the_frame() {
        let size = Size::new(120, 40);
        let mut g = Pulsar::default();
        g.resize(size, &mut Ctx::new(3, size));
        let keep_out = g.inset;
        for k in 0..40 {
            g.t = f64::from(k) * 0.07;
            g.draw_lines(magnetic_axis(phase(g.t)));
            let d = g.dots.dot_size();
            for y in 0..i32::from(d.height()) {
                for x in 0..i32::from(d.width()) {
                    if !g.dots.is_lit(x, y) {
                        continue;
                    }
                    let (cx, cy) = ((f64::from(x) + 0.5) / 2.0, (f64::from(y) + 0.5) / 4.0);
                    let in_inset = keep_out.is_some_and(|i| {
                        (i.x..i.x + i.w).contains(&(cx as i32))
                            && (i.y..i.y + i.h).contains(&(cy as i32))
                    });
                    if in_inset {
                        continue;
                    }
                    let wx = (cx - g.centre.0) / (g.zoom * ASPECT);
                    let wy = (g.centre.1 - cy) / g.zoom;
                    // Nothing reaches past the flash's spikes: open lines stop at
                    // `OPEN_REACH`, the axis at the cylinder's half-height.
                    assert!(
                        wx.hypot(wy) <= FLASH_SPIKE + 0.1,
                        "t={}: dot at ({wx}, {wy})",
                        g.t
                    );
                }
            }
            // The only dim guide is the spin axis, near x = 0: no cylinder walls.
            let mut c = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut c);
            for (i, cell) in c.cells().iter().enumerate() {
                let braille = (0x2801..=0x28FF).contains(&(cell.ch as u32));
                if braille && cell.ink.role == Role::Dim {
                    let cx = (i % 120) as f64 + 0.5;
                    let wx = (cx - g.centre.0) / (g.zoom * ASPECT);
                    assert!(wx.abs() < 0.4, "t={}: dim guide at x {wx}", g.t);
                }
            }
        }
        // Open lines fade out before `OPEN_REACH`, and the axis toward its ends.
        const { assert!(OPEN_REACH < CYLINDER_H && OPEN_REACH < FLASH_SPIKE) };
    }

    #[test]
    fn pul_a1_magnetic_axis_turns_about_the_spin_axis_once_per_period() {
        for t in [0.0, 0.3, 0.9, 5.1] {
            let m = magnetic_axis(phase(t));
            assert!((norm(m) - 1.0).abs() < 1e-12);
            assert!((dot(m, spin_axis()).acos() - ALPHA).abs() < 1e-9, "t={t}");
            let later = magnetic_axis(phase(t + PERIOD));
            assert!(norm(sub(m, later)) < 1e-9);
        }
        assert!((PERIOD - 1.3373).abs() < 1e-12, "CP 1919's period");
        assert!((phase(PERIOD) - TAU).abs() < 1e-12);
    }

    #[test]
    fn pul_a2_one_pulse_per_turn_and_no_interpulse() {
        let steps = 20_000;
        let turns = 10.0;
        let mut rising = 0;
        let mut prev = receiving(0.0);
        let mut far: f64 = PI;
        for i in 1..=steps {
            let t = turns * PERIOD * f64::from(i) / f64::from(steps);
            let now = receiving(t);
            rising += usize::from(now && !prev);
            prev = now;
            far = far.min(los_angle(scale(magnetic_axis(phase(t)), -1.0)));
        }
        assert_eq!(rising, 10);
        assert!(
            far >= 70.0 * DEG - 1e-6,
            "the far pole comes within {}°",
            far / DEG
        );
        // The pulse is centred where m is closest to our line of sight: β = ζ − α.
        let beta = los_angle(magnetic_axis(PULSE_PHASE));
        assert!((beta - (ZETA - ALPHA)).abs() < 1e-9);
    }

    #[test]
    fn pul_a3_field_lines_are_dipoles_and_closed_ones_stay_inside_the_cylinder() {
        let m = magnetic_axis(0.7);
        for (l, psi) in [(0.6, 0.0), (0.85, 2.1), (2.5, 4.0)] {
            for th in [0.2, 0.9, 1.57, 2.6] {
                let p = field_point(l, th, psi, m);
                assert!((norm(p) - l * th.sin_det().powi(2)).abs() < 1e-12);
                let cos = dot(p, m) / norm(p);
                assert!((cos - th.cos_det()).abs() < 1e-9, "colatitude");
            }
        }
        for &l in &CLOSED {
            for i in 0..=100 {
                let th = PI * f64::from(i) / 100.0;
                assert!(cylinder_radius(field_point(l, th, 1.0, m)) <= 1.0);
            }
        }
        // The open lines really are open: their apex lies outside the cylinder.
        assert!(cylinder_radius(field_point(OPEN, PI / 2.0, 0.0, m)) > 1.0);
        // Small canvases keep only the inner loop, so the star is not a tangle.
        assert_eq!(closed_loops(80), &CLOSED[..1]);
        assert_eq!(closed_loops(119), &CLOSED[..1]);
        assert_eq!(closed_loops(120), &CLOSED);
    }

    #[test]
    fn pul_a5_beam_is_dimmed_under_the_inset() {
        // Aim the beam at the top-right corner: without the dimming the stack sits on
        // bright violet; with it no pixel under the inset exceeds INSET_DIM of white.
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(3, size);
        let mut g = Pulsar::default();
        g.resize(size, &mut ctx);
        let inset = g.inset.expect("inset at 80x24");
        let (mut brightest_seen, mut steps) = (0u8, 0);
        while steps < (PERIOD * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            steps += 1;
            // The interior: the veil is feathered across the border.
            let f = INSET_FEATHER as i32;
            for py in 2 * inset.y + f..2 * (inset.y + inset.h) - f {
                for px in inset.x + f..inset.x + inset.w - f {
                    let ink = g.pixels.get(px, py).unwrap();
                    brightest_seen = brightest_seen.max(ink.level);
                }
            }
        }
        assert!(brightest_seen > 0, "the beam never reached the inset");
        assert!(
            f64::from(brightest_seen) <= 255.0 * INSET_DIM + 1.0,
            "beam at {brightest_seen} under the inset"
        );
    }

    #[test]
    fn pul_a5_the_inset_veil_is_feathered_not_a_pasted_panel() {
        // Under an even beam, the dimming across the inset's left border comes in steps,
        // not one hard edge, and the pixels well outside it are untouched.
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(3, size);
        let mut g = Pulsar::default();
        g.resize(size, &mut ctx);
        let inset = g.inset.expect("inset at 80x24");
        g.pixels.fill(plasma(200.0));
        g.dim_inset();
        let py = 2 * inset.y + inset.h;
        let row: Vec<u8> = (inset.x - 4..inset.x + 4)
            .map(|x| g.pixels.get(x, py).unwrap().level)
            .collect();
        assert_eq!(row[0], 200, "{row:?}");
        assert!(row.windows(2).all(|w| w[1] <= w[0]), "{row:?}");
        let steepest = row.windows(2).map(|w| w[0] - w[1]).max().unwrap();
        assert!(steepest <= 60, "a {steepest}-level edge: {row:?}");
        assert!(f64::from(row[7]) <= 200.0 * INSET_DIM + 1.0, "{row:?}");
    }

    #[test]
    fn pul_a4_pulses_vary_but_average_to_the_beam_profile() {
        let mut rng = Ctx::new(5, Size::new(1, 1)).rng;
        let mut stack = [[0.0_f32; SAMPLES]; PULSES];
        for (n, p) in stack.iter_mut().enumerate() {
            pulse(n as i64, &mut rng, p);
        }
        let centre = SAMPLES / 2;
        for p in &stack {
            let peak = (0..SAMPLES).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap();
            let off = (sample_phase(peak) - sample_phase(centre)).abs();
            assert!(off < 2.0 * RHO, "peak {off} rad off the pulse centre");
        }
        let mean: Vec<f64> = (0..SAMPLES)
            .map(|i| stack.iter().map(|p| f64::from(p[i])).sum::<f64>() / PULSES as f64)
            .collect();
        let geo: Vec<f64> = (0..SAMPLES).map(|i| template(sample_phase(i))).collect();
        let corr = {
            let m = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let (ma, mb) = (m(&mean), m(&geo));
            let cov: f64 = mean
                .iter()
                .zip(&geo)
                .map(|(a, b)| (a - ma) * (b - mb))
                .sum();
            let va: f64 = mean.iter().map(|a| (a - ma).powi(2)).sum();
            let vb: f64 = geo.iter().map(|b| (b - mb).powi(2)).sum();
            cov / (va * vb).sqrt()
        };
        assert!(corr >= 0.9, "mean pulse correlates {corr} with the beam");
        let diff = stack[0]
            .iter()
            .zip(&stack[1])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(diff >= 0.05, "consecutive pulses are identical");
    }

    #[test]
    fn pul_a5_inset_sits_top_right_clear_of_the_title() {
        for (w, h) in [(80, 24), (200, 50), (60, 16)] {
            let size = Size::new(w, h);
            let inset = Inset::fit(size).expect("room for the inset");
            assert!(inset.x >= 0 && inset.x + inset.w < i32::from(w));
            assert!(inset.y >= 1 && inset.y + inset.h < i32::from(h) - 1);
            assert!(inset.x >= i32::from(w) / 2, "right half");
            let mut stage = Stage::new(build(), 42, size, GlyphMode::Unicode);
            stage.step(0.5);
            let frame = stage.render();
            let lit = (inset.y..inset.y + inset.h)
                .flat_map(|y| (inset.x..inset.x + inset.w).map(move |x| (x, y)))
                .filter(|&(x, y)| frame.get(x, y).is_some_and(|c| c.ch != ' '))
                .count();
            assert!(lit > 10, "the stack is drawn at {w}x{h}");
        }
        assert_eq!(Inset::fit(Size::new(59, 30)), None);
        assert_eq!(Inset::fit(Size::new(100, 15)), None);
    }

    /// Share of the sky left black: the beam's faint cone dissolves instead of tinting
    /// the whole frame.
    fn dark_share(g: &Pulsar) -> f64 {
        let px = g.pixels.pixel_size();
        let (w, h) = (i32::from(px.width()), i32::from(px.height()));
        let empty = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| g.pixels.get(x, y) == Some(Ink::BLANK))
            .count();
        empty as f64 / f64::from(w * h)
    }

    #[test]
    fn pul_a6_a_beam_aimed_at_us_lights_under_a_third_of_the_frame() {
        for (w, h) in [(80, 24), (120, 40)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(1, size);
            let mut g = Pulsar::default();
            g.resize(size, &mut ctx);
            let mut darkest = 1.0_f64;
            for _ in 0..((PERIOD * 60.0) as usize + 1) {
                g.update(SIM_DT, &mut ctx);
                if receiving(g.t) {
                    darkest = darkest.min(dark_share(&g));
                }
            }
            assert!(
                darkest >= 2.0 / 3.0,
                "{w}x{h}: only {darkest:.2} of the sky stays black at the pulse"
            );
        }
    }

    #[test]
    fn pul_a6_beams_are_plasma_and_flash_when_aimed_at_us() {
        // PUL-A6: the star flares while the beam points at us.
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(1, size);
        let mut g = Pulsar::default();
        g.resize(size, &mut ctx);
        let spike = |g: &Pulsar| {
            let (x, y) = g.dot([0.0; 3]);
            let (x, y) = (x.round() as i32, y.round() as i32);
            // Lit dots on the spike's row, well clear of the star and the field lines'
            // footpoints.
            (8..40).filter(|&d| g.dots.is_lit(x + d, y)).count()
        };
        let (mut on, mut off, mut darkest) = (0, 0, 1.0_f64);
        for _ in 0..((PERIOD * 60.0) as usize + 1) {
            g.update(SIM_DT, &mut ctx);
            if (phase(g.t) - PULSE_PHASE).rem_euclid(TAU) < 0.02 * TAU {
                on = on.max(spike(&g));
            } else if !receiving(g.t - 0.2) && !receiving(g.t + 0.2) && !receiving(g.t) {
                off = off.max(spike(&g));
                darkest = darkest.min(dark_share(&g));
            }
        }
        assert!(
            on >= 25 && on > 2 * off,
            "spike {on} dots at the pulse, {off} away"
        );
        assert!(
            darkest >= 0.5,
            "only {darkest:.2} of the sky is black off the pulse"
        );
        let size = g.pixels.pixel_size();
        for y in 0..i32::from(size.height()) {
            for x in 0..i32::from(size.width()) {
                let role = g.pixels.get(x, y).unwrap().role;
                assert!(
                    matches!(
                        role,
                        Role::Background | Role::Ramp(Ramp::Plasma) | Role::Blackbody
                    ),
                    "{role:?}"
                );
            }
        }
    }
}
