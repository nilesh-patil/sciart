//! An oblique rotator: a neutron star whose tilted magnetic axis sweeps two narrow radio
//! beams through space, seen from the side (SPEC-002 `pulsar`, ADR-0017).
//!
//! World axes: x right, y up, z towards the viewer; lengths in units of the light-cylinder
//! radius `R_LC = cP/2π` (not to scale: CP 1919's is ~6400 star radii). The view is
//! orthographic. The spin axis leans `90° − ζ` towards us, the magnetic axis is `α` off it,
//! so each turn the beam sweeps a cone that brushes our line of sight once, and the star
//! flashes.

use std::f64::consts::{PI, TAU};

use sciart_core::geom::ASPECT;
use sciart_core::geom::v3::{V3, add, cross, scale, unit};
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title::{self, Title};
use sciart_core::{Tone, draw, ink, raster, rng};

pub static META: Meta = Meta {
    name: "pulsar",
    category: Category::DeepCosmos,
    description: "An oblique rotator sweeping two narrow radio beams, flashing as one sweeps past us",
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
/// The drawn beam is the inner `NARROW` of the cone (ADR-0017: narrow beams).
const NARROW: f64 = 0.5;
/// Star radius, in `R_LC`.
const R_STAR: f64 = 0.07;
/// Beam rays per cone, samples along each ray, and the exposure's e-folding time (s):
/// short, so the sweep does not smear the beam into a fan (ADR-0017).
const RAYS: usize = 48;
const RAY_STEPS: usize = 80;
const BEAM_TAU: f64 = 0.02;
/// How fast the ray pattern turns about each beam axis (rad/s): fast enough that
/// consecutive frames interleave, which hides the discrete rays.
const RAY_TURN: f64 = 37.0;
/// Each ray's samples start at a hashed offset of up to one sample step, new every
/// exposure, so the samples of neighbouring rays never line up into radial spokes.
const RAY_JITTER_SALT: u64 = 0x5075_6C73_6172_0001;
/// The flare while we receive the pulse (ADR-0017: no crosshair): shown while the pulse
/// profile is at least `FLARE_ON` (its peak is ≈ 0.46 at β = 10°; ≈ 0.1 s a turn), a 3 × 3 white core and four spikes of
/// `FLARE_SPIKE` pixels fading from `FLARE_LEVEL.0` to `FLARE_LEVEL.1`.
const FLARE_ON: f64 = 0.3;
const FLARE_SPIKE: i32 = 5;
const FLARE_LEVEL: (f64, f64) = (220.0, 90.0);
/// Beam tone: exposure at the knee and at white, the beam's top level (its core by the
/// star reaches the pale top, rule 6; it dims fast outward: muted, ADR-0017),
/// and the level below which the beam's faint edge fades into black space.
const BEAM_KNEE: f32 = 0.4;
const BEAM_WHITE: f32 = 6.0;
const BEAM_TOP: f32 = 235.0;
const BEAM_FLOOR: u8 = 24;
/// Exposure steps of the first frame after a resize, so it opens on a smooth beam rather
/// than one frame's rays.
const PRE_EXPOSURE: usize = 6;
/// Rows per `R_LC`, as a fraction of `draw::fit_radius(size, 1)`.
const ZOOM: f64 = 0.55;
/// A beam whose axis points at us (`axis·ẑ` from `FACING.0` to `FACING.1`) is dimmed by
/// up to `FACING_DIM` and shortened by up to `FACING_SHORTEN`.
const FACING: (f64, f64) = (0.3, 0.9);
const FACING_DIM: f64 = 0.75;
const FACING_SHORTEN: f64 = 0.75;
/// The star: one white pixel (it is ~10 km across).
const STAR_K: f64 = 7000.0;
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
#[cfg(test)]
pub(crate) const PULSE_PHASE: f64 = -PI / 2.0;

/// Unit vectors `(u₁, u₂)` completing `m` to a right-handed frame.
fn frame(m: V3) -> (V3, V3) {
    let u1 = unit(cross(spin_axis(), m));
    (u1, cross(m, u1))
}

/// Geometric pulse profile at phase `φ`: `exp(−γ²/2σ²)` with `γ` the angle between the
/// beam axis and our line of sight, `σ = ρ/2` (PUL-A6).
pub(crate) fn template(phi: f64) -> f64 {
    let g = los_angle(magnetic_axis(phi)) / (0.5 * RHO);
    (-0.5 * g * g).exp()
}

/// A beam ray's weight at fractional cone radius `f ∈ [0, 1]` (of the drawn cone):
/// brightest on the axis, falling to exactly zero at the edge (PUL-A4).
pub(crate) fn across(f: f64) -> f64 {
    let edge = (-3.0_f64).exp();
    (((-3.0 * f * f).exp() - edge) / (1.0 - edge)).max(0.0)
}

/// The direction of ray `j` (of `RAYS`) at turn `turn` about beam `axis`, and its
/// fractional cone radius: a sunflower pattern over the drawn (narrow) cone.
pub(crate) fn ray(axis: V3, j: usize, turn: f64) -> (V3, f64) {
    let golden = PI * (3.0 - 5f64.sqrt());
    let (u1, u2) = frame(axis);
    let f = ((j as f64 + 0.5) / RAYS as f64).sqrt();
    let (s, c) = (golden * j as f64 + turn).sin_cos_det();
    let (sa, ca) = (f * NARROW * RHO).sin_cos_det();
    let d = add(scale(axis, ca), scale(add(scale(u1, c), scale(u2, s)), sa));
    (d, f)
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
    /// Exposures so far: keys each exposure's ray jitter.
    exposures: u64,
    readout: bool,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
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
            exposures: 0,
            readout: false,
            dirty: false,
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

impl Pulsar {
    /// Continuous dot coordinates of the world point `p`.
    fn dot(&self, p: V3) -> (f64, f64) {
        let (cx, cy) = self.centre;
        Dots::from_cells(cx + p[0] * self.zoom * ASPECT, cy - p[1] * self.zoom)
    }

    /// Expose both beams for `dt`: rays over the narrow cone (a sunflower pattern, turned
    /// a little every frame so the exposure smooths it into a glow instead of spokes),
    /// marched out from the star; brightest on the axis and near the star, zero at the
    /// cone's edge.
    fn expose(&mut self, m: V3, dt: f64, t: f64) {
        self.beams.decay((-dt / BEAM_TAU).exp() as f32);
        self.exposures = self.exposures.wrapping_add(1);
        let salt = RAY_JITTER_SALT ^ self.exposures.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let reach = {
            let (w, h) = (f64::from(self.size.width()), f64::from(self.size.height()));
            (w / ASPECT).hypot(h) / self.zoom
        };
        let turn = RAY_TURN * t;
        for axis in [m, scale(m, -1.0)] {
            // A beam aimed at us fans its whole length out round the star: dim and shorten
            // it as it turns our way, so the splash stays a glow round the flash.
            let facing = ease::smoothstep((axis[2] - FACING.0) / (FACING.1 - FACING.0));
            let w0 = (dt / BEAM_TAU * (1.0 - FACING_DIM * facing)) as f32;
            let length = reach * (1.0 - FACING_SHORTEN * facing);
            for (j, key) in (0..RAYS).zip(0u64..) {
                let (d, f) = ray(axis, j, turn);
                let weight = w0 * across(f) as f32;
                let jitter = rng::hash01(salt ^ (key << 1 | u64::from(axis[2] < m[2])));
                for i in 0..RAY_STEPS {
                    let r = R_STAR + length * (i as f64 + jitter) / RAY_STEPS as f64;
                    let (x, y) = self.dot(scale(d, r));
                    // Bilinear (CORE-R92): a pixel is 2 × 2 dots, its centre at dot
                    // `2p + 0.5`, so no sample snaps to the nearest pixel.
                    self.beams.splat_bilinear(
                        (x - 0.5) / 2.0,
                        (y - 0.5) / 2.0,
                        weight / (1.0 + 2.0 * r as f32),
                    );
                }
            }
        }
    }

    /// The received pulse as seen by us (PUL-A6): while the beam faces us the star flares
    /// for a moment, a 3 × 3 white core with four short spikes.
    fn flare(&mut self) {
        if template(phase(self.t)) < FLARE_ON {
            return;
        }
        let (x, y) = (
            (self.centre.0 - 0.5).round() as i32,
            (2.0 * self.centre.1 - 0.5).round() as i32,
        );
        let white = ink::blackbody(STAR_K);
        for dy in -1..=1 {
            for dx in -1..=1 {
                self.pixels.set(x + dx, y + dy, white);
            }
        }
        for k in 2..=FLARE_SPIKE {
            let f = f64::from(k - 2) / f64::from((FLARE_SPIKE - 2).max(1));
            let ink = Ink::new(
                Role::Ramp(Ramp::Plasma),
                ink::level(ease::lerp(FLARE_LEVEL.0, FLARE_LEVEL.1, f)),
            );
            for (dx, dy) in [(k, 0), (-k, 0), (0, k), (0, -k)] {
                self.pixels.set(x + dx, y + dy, ink);
            }
        }
    }

    /// The star: one white pixel.
    fn star(&mut self) {
        let (x, y) = (self.centre.0 - 0.5, 2.0 * self.centre.1 - 0.5);
        self.pixels
            .set(x.round() as i32, y.round() as i32, ink::blackbody(STAR_K));
    }

    /// Everything for the current time (the M10 raster convention: `render` only blits);
    /// the exposure accumulates per step, so `update` does that part (GEN-R20).
    fn rasterise(&mut self) {
        let tone = &self.tone;
        self.beams.to_pixels(&mut self.pixels, tone, |l| {
            let v = f32::from(l) * BEAM_TOP / 255.0;
            Ink::new(Role::Ramp(Ramp::Plasma), v as u8)
        });
        self.pixels.dissolve(BEAM_FLOOR);
        self.star();
        self.flare();
    }
}

impl Generator for Pulsar {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.centre = (
            f64::from(size.width()) / 2.0,
            f64::from(size.height()) / 2.0,
        );
        self.zoom = ZOOM * draw::fit_radius(size, 1.0);
        self.beams.resize(size);
        self.pixels.resize(size);
        self.readout = READOUT.len() <= title::readout_room(size, &Title::of(self));
        // Expose the last 3τ of the sweep in steps, so the first frame already shows a
        // smooth beam instead of one step's rays.
        let dt = 3.0 * BEAM_TAU / PRE_EXPOSURE as f64;
        for k in (0..PRE_EXPOSURE).rev() {
            let t = self.t - dt * k as f64;
            self.expose(magnetic_axis(phase(t)), dt, t);
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if !(dt.is_finite() && dt >= 0.0) {
            return;
        }
        self.t += dt;
        self.expose(magnetic_axis(phase(self.t)), dt, self.t);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_pixel(top, bottom));
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
    use sciart_core::geom::v3::{dot, norm, sub};

    const DEG: f64 = PI / 180.0;

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
    fn pul_a3_beam_rays_stay_inside_the_narrow_cone() {
        for phi in [0.0, 1.1, 2.9, 4.4] {
            let m = magnetic_axis(phi);
            for axis in [m, scale(m, -1.0)] {
                for j in 0..RAYS {
                    let (d, f) = ray(axis, j, 0.7 * j as f64);
                    assert!((norm(d) - 1.0).abs() < 1e-9);
                    let angle = dot(d, axis).clamp(-1.0, 1.0).acos();
                    assert!(angle <= NARROW * RHO + 1e-9, "ray {j} at {angle}");
                    assert!((0.0..=1.0).contains(&f));
                }
            }
        }
    }

    #[test]
    fn pul_a4_the_beam_falls_to_zero_at_the_cone_edge() {
        assert!((across(0.0) - 1.0).abs() < 1e-12);
        assert!(across(1.0).abs() < 1e-12);
        for k in 0..100 {
            let (a, b) = (f64::from(k) / 100.0, f64::from(k + 1) / 100.0);
            assert!(across(b) <= across(a), "{a}");
        }
    }

    #[test]
    fn pul_a5_no_inset_and_only_beams_star_flash_and_readout() {
        // ADR-0017: no pulse-stack inset, no field lines or guides; the pixels are beam
        // light (muted Plasma ≤ BEAM_TOP) and the star.
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(3, size);
        let mut g = Pulsar::default();
        g.resize(size, &mut ctx);
        g.prepare();
        for _ in 0..90 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let px = g.pixels.pixel_size();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    let ink = g.pixels.get(x, y).unwrap();
                    match ink.role {
                        Role::Background | Role::Blackbody => {}
                        Role::Ramp(Ramp::Plasma) => {
                            assert!(f32::from(ink.level) <= BEAM_TOP + 1.0, "{ink:?}");
                        }
                        other => panic!("{other:?}"),
                    }
                }
            }
        }
    }

    #[test]
    fn pul_a7_the_flare_is_brief_and_small() {
        // ADR-0017: no crosshair. Over two turns at 120×40, the flare (white pixels beyond
        // the star's own) lights only within FLARE_SPIKE of the star, for about 0.12 s a turn.
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(3, size);
        let mut g = Pulsar::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let (cx, cy) = (
            (g.centre.0 - 0.5).round() as i32,
            (2.0 * g.centre.1 - 0.5).round() as i32,
        );
        let mut flaring = 0;
        let steps = (2.0 * PERIOD * 60.0) as usize;
        for _ in 0..steps {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let mut white = 0;
            let px = g.pixels.pixel_size();
            for y in 0..i32::from(px.height()) {
                for x in 0..i32::from(px.width()) {
                    if g.pixels.get(x, y).unwrap().role == Role::Blackbody {
                        assert!(
                            (x - cx).abs().max((y - cy).abs()) <= FLARE_SPIKE,
                            "white at {x},{y}"
                        );
                        white += 1;
                    }
                }
            }
            flaring += usize::from(white > 1);
        }
        let per_turn = flaring as f64 * SIM_DT / 2.0;
        assert!(
            (0.05..=0.15).contains(&per_turn),
            "flare {per_turn} s a turn"
        );
    }

    /// Share of the sky left black: the beam's faint cone fades instead of tinting the
    /// whole frame.
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
            g.prepare();
            let mut darkest = 1.0_f64;
            for _ in 0..((PERIOD * 60.0) as usize + 1) {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
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
        g.prepare();
        // Lit flare pixels round the star (its 3 × 3 core and spikes, beyond the star's
        // own pixel), counted as white or bright violet.
        let spike = |g: &Pulsar| {
            let (x, y) = (
                (g.centre.0 - 0.5).round() as i32,
                (2.0 * g.centre.1 - 0.5).round() as i32,
            );
            (-FLARE_SPIKE..=FLARE_SPIKE)
                .flat_map(|dy| (-FLARE_SPIKE..=FLARE_SPIKE).map(move |dx| (dx, dy)))
                .filter(|&(dx, dy)| (dx, dy) != (0, 0))
                .filter(|&(dx, dy)| {
                    g.pixels
                        .get(x + dx, y + dy)
                        .is_some_and(|i| i.role == Role::Blackbody)
                })
                .count()
        };
        let (mut on, mut off, mut darkest) = (0, 0, 1.0_f64);
        for _ in 0..((PERIOD * 60.0) as usize + 1) {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if (phase(g.t) - PULSE_PHASE).rem_euclid(TAU) < 0.02 * TAU {
                on = on.max(spike(&g));
            } else if !receiving(g.t - 0.2) && !receiving(g.t + 0.2) && !receiving(g.t) {
                off = off.max(spike(&g));
                darkest = darkest.min(dark_share(&g));
            }
        }
        assert!(
            on >= 8 && off == 0,
            "flare {on} pixels at the pulse, {off} away"
        );
        assert!(
            darkest >= 0.5,
            "only {darkest:.2} of the sky is black off the pulse"
        );
    }
}
