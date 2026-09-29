//! A mature hurricane seen from above (SPEC-002 `hurricane`).
//!
//! Cloud water rides a kinematic vortex on a capped lattice (`vortex_advect`,
//! semi-Lagrangian): a Holland-profile tangential wind, counter-clockwise as in the
//! northern hemisphere, plus a low-level inflow that stops at the eyewall and so draws
//! the bands in as logarithmic spirals. Convection feeds the eyewall and three spiral rain
//! bands (pulsing, noise-modulated cells), subsidence clears the eye, and evaporation
//! clears the ocean between the bands. A `Timeline` alternates the visible picture with an
//! enhanced-infrared one, in which the coldest (highest) cloud tops glow.
//!
//! Units: lengths in lattice heights (`H`), speeds in `H` per simulated second, so the
//! storm keeps its shape when the lattice is regridded (ADR-0012).

use std::f64::consts::TAU;

use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::ascii_ramp_cell;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{Grid, ink};

use crate::shared::vortex_advect::{self, Flow, XEdge};
use sciart_core::rng::value_noise;

pub static META: Meta = Meta {
    name: "hurricane",
    category: Category::EarthSky,
    description: "Spiral rain bands wind counter-clockwise into a clear eye; an infrared act",
    title: "Hurricane · Coriolis spirals",
    captions: &["visible", "infrared"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Hurricane::new())
}

/// Lattice cap (HUR-A5, ADR-0012) and aspect bucket.
pub(crate) const CAP: usize = 160 * 90;
const BUCKET: f64 = 1.25;

/// Storm radius (the outer edge of the bands), radius of maximum wind and the Holland
/// shape parameter, in `H`.
const R_STORM: f64 = 0.46;
pub(crate) const R_MAX_WIND: f64 = 0.14 * R_STORM;
const HOLLAND_B: f64 = 1.5;
/// Peak wind: one turn at the radius of maximum wind every 3 s (time-lapsed).
const V_MAX: f64 = TAU * R_MAX_WIND / 3.0;
/// Inflow as a fraction of the tangential wind outside the eyewall.
const INFLOW: f64 = 0.28;
/// Spiral bands: count and pitch angle.
const BANDS: usize = 3;
const PITCH: f64 = 0.28;
/// Rates per second: evaporation everywhere, subsidence in the eye, band and eyewall
/// convection.
const EVAPORATE: f32 = 0.4;
const SUBSIDE: f32 = 5.0;
const BAND_RATE: f32 = 1.4;
const WALL_RATE: f32 = 2.5;
/// Brightness below which the sea is clear, and above which cloud is opaque.
const CLEAR: f64 = 0.012;
const THICK: f64 = 0.07;
/// Convective texture tile: texels per side, the disc it spans (in `R_STORM`) and how
/// fast it turns with the storm, rad/s.
const TURRET_N: usize = 256;
const TURRET_SPAN: f64 = 1.15;
const TURRET_SPIN: f64 = 0.35;
/// Radial wind table resolution over `[0, 2·R_STORM]`.
const TABLE: usize = 512;
/// Steps run in the first resize, so the first frame is a mature storm.
const SPIN_UP: usize = 180;
/// Seconds of dithered dissolve between the visible and infrared pictures.
const DISSOLVE: f64 = 1.5;

const ACTS: [(&str, f64); 2] = [("visible", 25.0), ("infrared", 12.0)];
const IR: usize = 1;

/// Holland (1980) tangential wind at radius `r` (`H`), `H`/s: peaks at `R_MAX_WIND`
/// (HUR-A1), decays outward and vanishes at the centre.
pub(crate) fn holland(r: f64) -> f64 {
    if r.is_nan() || r <= 0.0 {
        return 0.0;
    }
    let x = (R_MAX_WIND / r).powf(HOLLAND_B);
    V_MAX * (x * (1.0 - x).exp()).sqrt()
}

/// Inflow speed (positive inward) at radius `r`: a share of the tangential wind outside
/// the eyewall, easing to nothing inside it, so air piles up at the wall and rises.
fn inflow(r: f64) -> f64 {
    let t = ((r - R_MAX_WIND) / (0.6 * R_MAX_WIND)).clamp(0.0, 1.0);
    INFLOW * holland(r) * t * t * (3.0 - 2.0 * t)
}

/// Storm-relative wind at offset `(dx, dy)` (`y` down), `H`/s: counter-clockwise on
/// screen, spiralling in.
#[cfg(test)]
pub(crate) fn wind(dx: f64, dy: f64) -> (f64, f64) {
    let r = dx.hypot(dy);
    wind_from(dx, dy, holland(r), inflow(r))
}

/// The wind at offset `(dx, dy)` from tangential speed `vt` and inflow `vr`.
#[inline]
fn wind_from(dx: f64, dy: f64, vt: f64, vr: f64) -> (f64, f64) {
    let r = dx.hypot(dy);
    if r < 1e-9 {
        return (0.0, 0.0);
    }
    // Counter-clockwise tangent with y down is (dy, −dx)/r; inward is −(dx, dy)/r.
    ((vt * dy - vr * dx) / r, (-vt * dx - vr * dy) / r)
}

#[derive(Debug)]
pub struct Hurricane {
    t: f64,
    timeline: Timeline,
    lattice: Size,
    cloud: Grid<f32>,
    /// Cloud-top brightness `1 − e^{−1.5·c}` and ocean swell, per node.
    bright: Grid<f32>,
    swell: Grid<f32>,
    /// Enhanced-IR ink by brightness level.
    ir_lut: [Ink; 256],
    /// Convective texture (cumulus towers, `[−½, ½]`) on a `TURRET_N²` tile over the
    /// storm disc, fixed per storm, turning slowly with it.
    turrets: Box<[f32]>,
    scratch: Grid<f32>,
    flow: Flow,
    /// `(tangential, inflow)` wind over `[0, 2·R_STORM]`, `H`/s.
    table: Vec<(f32, f32)>,
    map: SampleMap,
    pixels: Pixels,
    warmed: bool,
}

impl Hurricane {
    fn new() -> Self {
        Self {
            t: 0.0,
            timeline: Timeline::new(&ACTS),
            lattice: Size::new(1, 1),
            cloud: Grid::default(),
            bright: Grid::default(),
            swell: Grid::default(),
            turrets: (0..TURRET_N * TURRET_N)
                .map(|k| {
                    let (x, y) = ((k % TURRET_N) as f64, (k / TURRET_N) as f64);
                    // Two octaves of cauliflower-like bumps, a few pixels across.
                    let n = 0.65 * value_noise(0x7EE7, x, y, 12.0)
                        + 0.35 * value_noise(0x7EE8, x, y, 5.0);
                    (n * n * 1.6 - 0.5) as f32
                })
                .collect(),
            ir_lut: std::array::from_fn(|k| {
                let b = k as f64 / 255.0;
                ink::div((b / 0.72).min(1.0).powi(2))
            }),
            scratch: Grid::default(),
            flow: Flow::default(),
            table: (0..TABLE)
                .map(|k| {
                    let r = 2.0 * R_STORM * k as f64 / (TABLE - 1) as f64;
                    (holland(r) as f32, inflow(r) as f32)
                })
                .collect(),
            map: SampleMap::default(),
            pixels: Pixels::default(),
            warmed: false,
        }
    }

    /// Lattice width in `H`.
    fn width(&self) -> f64 {
        f64::from(self.lattice.width()) / f64::from(self.lattice.height())
    }

    /// Storm centre in `H`: a slow drift about the middle of the frame.
    fn centre(&self, t: f64) -> (f64, f64) {
        let (s, c) = (TAU * t / 90.0).sin_cos_det();
        (0.5 * self.width() + 0.05 * c, 0.5 + 0.03 * s)
    }

    fn wind_at(table: &[(f32, f32)], r: f64) -> (f64, f64) {
        let pos = r / (2.0 * R_STORM) * (TABLE - 1) as f64;
        let k = (pos.max(0.0) as usize).min(TABLE - 2);
        let f = (pos - k as f64).clamp(0.0, 1.0) as f32;
        let (a, b) = (table[k], table[k + 1]);
        (
            f64::from(a.0 + f * (b.0 - a.0)),
            f64::from(a.1 + f * (b.1 - a.1)),
        )
    }

    fn fill_flow(&mut self) {
        let (w, h) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        let scale = h as f64;
        let (cx, cy) = self.centre(self.t);
        let (us, vs) = (self.flow.u.as_mut_slice(), self.flow.v.as_mut_slice());
        for k in 0..w * h {
            let (dx, dy) = ((k % w) as f64 / scale - cx, (k / w) as f64 / scale - cy);
            let r = dx.hypot(dy);
            let (vt, vr) = Self::wind_at(&self.table, r);
            let (u, v) = wind_from(dx, dy, vt, vr);
            us[k] = (u * scale) as f32;
            vs[k] = (v * scale) as f32;
        }
    }

    fn step(&mut self, dt: f64) {
        self.t += dt;
        self.fill_flow();
        vortex_advect::advect(
            &self.cloud,
            &mut self.scratch,
            &self.flow,
            dt as f32,
            XEdge::Clamp,
        );
        std::mem::swap(&mut self.cloud, &mut self.scratch);

        let (w, h) = (
            usize::from(self.lattice.width()),
            usize::from(self.lattice.height()),
        );
        let scale = h as f64;
        let (cx, cy) = self.centre(self.t);
        let dtf = dt as f32;
        let evaporate = (-EVAPORATE * dtf).exp();
        let subside = (-SUBSIDE * dtf).exp();
        let t = self.t;
        // The band pattern turns slowly with the storm; `(sp, cp)` rotate the pulse.
        let spin = 0.12 * t;
        let cot = 1.0 / PITCH.tan();
        let (sp, cp) = (0.4 * t).sin_cos_det();
        let (s13, c13) = 1.3f64.sin_cos_det();
        for (k, c) in self.cloud.as_mut_slice().iter_mut().enumerate() {
            let (dx, dy) = ((k % w) as f64 / scale - cx, (k / w) as f64 / scale - cy);
            let r = dx.hypot(dy);
            *c *= if r < 0.85 * R_MAX_WIND {
                subside
            } else {
                evaporate
            };
            if !(1e-9..=1.05 * R_STORM).contains(&r) {
                continue;
            }
            // Counter-clockwise screen angle (y down), as its cosine and sine.
            let (ct, st) = (dx / r, -dy / r);
            let mut feed = 0.0f32;
            if (R_MAX_WIND..1.6 * R_MAX_WIND).contains(&r) {
                let theta = st.atan2(ct);
                let cell = value_noise(0xE7E, 9.0 * theta + 3.0 * t, 40.0 * r, 1.0);
                feed += WALL_RATE * (0.6 + 0.6 * cell as f32);
            } else if (1.6 * R_MAX_WIND..3.4 * R_MAX_WIND).contains(&r) {
                // The central dense overcast: cirrus spreading out of the eyewall.
                let fall = 1.0 - (r / R_MAX_WIND - 1.6) / 1.8;
                feed += 0.45 * WALL_RATE * (fall * fall) as f32;
            }
            if r > 1.8 * R_MAX_WIND {
                // Logarithmic spirals: the band phase 3·(θ + cot(pitch)·ln r − spin) is
                // constant along curves that wind counter-clockwise inward. cos 3θ and
                // sin 3θ come from the triple-angle formulas, so a node costs one
                // `sin_cos` rather than an `atan2` and three.
                let (sx, cx3) = (BANDS as f64 * (cot * (r / R_MAX_WIND).ln() - spin)).sin_cos_det();
                let (c3, s3) = (4.0 * ct * ct * ct - 3.0 * ct, 3.0 * st - 4.0 * st * st * st);
                let (c1, s1) = (c3 * cx3 - s3 * sx, s3 * cx3 + c3 * sx);
                let c2 = (2.0 * c1 * c1 - 1.0) * c13 - 2.0 * s1 * c1 * s13;
                let band = (0.5 + 0.5 * c1).powi(8) + 0.35 * (0.5 + 0.5 * c2).powi(12);
                if band < 0.004 {
                    *c += (1.2 - *c).max(0.0) * feed * dtf;
                    continue;
                }
                let cells = value_noise(0xBA7D, 70.0 * dx + 1.5 * t, 70.0 * dy, 1.0).powi(2)
                    + 0.5 * value_noise(0xBA7E, 190.0 * dx, 190.0 * dy - 2.0 * t, 1.0);
                let edge = (1.0 - r / (1.05 * R_STORM)).clamp(0.0, 1.0).sqrt();
                let pulse = 0.7 + 0.3 * (sp * ct + cp * st);
                feed += BAND_RATE * (band * (0.2 + 1.6 * cells) * edge * pulse) as f32;
            }
            *c += (1.2 - *c).max(0.0) * feed * dtf;
        }
        // What the pictures need, per node: cloud-top brightness and the ocean swell.
        for (k, (b, sw)) in self
            .bright
            .as_mut_slice()
            .iter_mut()
            .zip(self.swell.as_mut_slice())
            .enumerate()
        {
            let c = self.cloud.as_slice()[k];
            *b = 1.0 - (-1.5 * c).exp();
            let (x, y) = ((k % w) as f64, (k / w) as f64);
            *sw = value_noise(0x0CEA, x * 0.1 + 0.2 * t, y * 0.1, 1.0) as f32;
        }
    }

    fn rebuild_lattice(&mut self, lattice: Size) {
        if lattice == self.lattice && !self.cloud.is_empty() {
            return;
        }
        if self.cloud.is_empty() {
            self.cloud.resize(lattice, 0.0);
        } else {
            grid::regrid_bilinear(&mut self.cloud, lattice, &mut self.scratch);
        }
        self.lattice = lattice;
        self.scratch.resize(lattice, 0.0);
        self.bright.resize(lattice, 0.0);
        self.swell.resize(lattice, 0.0);
        self.flow.resize(lattice);
    }

    /// Weight of the infrared picture now: 0 visible, 1 infrared, dissolving between.
    fn ir_weight(&self) -> f64 {
        let beat = self.timeline.at(self.t);
        let dur = ACTS[beat.index].1;
        let into = (beat.local / DISSOLVE).min(1.0);
        let left = ((dur - beat.local) / DISSOLVE).min(1.0);
        if beat.index == IR {
            // Dissolve in at the start and out at the end.
            into.min(left).max(0.0)
        } else {
            0.0
        }
    }

    /// Convective texture at pixel `(x, y)`: the tile, turned with the storm.
    fn turret(t: &[f32], centre: (f64, f64), x: i32, y: i32, rot: (f64, f64), ph: f64) -> f64 {
        let (cx, cy) = centre;
        let (dx, dy) = (
            (f64::from(x) + 0.5) / ph - cx,
            (f64::from(y) + 0.5) / ph - cy,
        );
        let (s, c) = rot;
        let (u, v) = (c * dx - s * dy, s * dx + c * dy);
        let k = TURRET_N as f64 / (2.0 * TURRET_SPAN * R_STORM);
        let (tu, tv) = (u * k + 0.5 * TURRET_N as f64, v * k + 0.5 * TURRET_N as f64);
        if !(0.0..(TURRET_N - 1) as f64).contains(&tu)
            || !(0.0..(TURRET_N - 1) as f64).contains(&tv)
        {
            return 0.0;
        }
        let (i, j) = (tu as usize, tv as usize);
        let (fu, fv) = ((tu - i as f64) as f32, (tv - j as f64) as f32);
        let at = |i: usize, j: usize| t[j * TURRET_N + i];
        let top = at(i, j) + fu * (at(i + 1, j) - at(i, j));
        let bottom = at(i, j + 1) + fu * (at(i + 1, j + 1) - at(i, j + 1));
        f64::from(top + fv * (bottom - top))
    }

    fn shade(&mut self) {
        let ir = self.ir_weight();
        let ph = f64::from(self.pixels.pixel_size().height());
        let rot = (TURRET_SPIN * self.t).sin_cos_det();
        let centre = self.centre(self.t);
        let (bright, swell, map, ir_lut, turrets) = (
            &self.bright,
            &self.swell,
            &self.map,
            &self.ir_lut,
            &self.turrets,
        );
        self.pixels.shade(|x, y| {
            let b = map.sample(bright, x, y);
            let use_ir = ir > 0.0 && (ir >= 1.0 || hash01(0x1A ^ cell_key(x, y)) < ir);
            if use_ir {
                // Enhanced IR: the warm sea and low cloud dark, the coldest
                // (thickest, highest) tops orange to yellow.
                return if f64::from(b) > CLEAR {
                    ir_lut[((b * 255.0) as usize).min(255)]
                } else {
                    Ink::BLANK
                };
            }
            // Relief: tops facing the low sun in the west brighter, lee sides darker; the
            // convective towers add bumps (and their own sunlit and shadowed sides) where
            // the cloud is thick.
            let b = f64::from(b);
            if b > THICK {
                let lee = f64::from(map.sample(bright, x + 1, y + 1));
                let bump = Self::turret(turrets, centre, x, y, rot, ph);
                let bump_lee = Self::turret(turrets, centre, x + 1, y + 1, rot, ph);
                let conv = (b / 0.5).min(1.0);
                let v = 0.14
                    + 1.1 * b * (1.0 + 0.22 * conv * bump)
                    + 2.0 * (b - lee)
                    + 0.3 * conv * (bump - bump_lee);
                ink::ramp(Ramp::Starlight, v.clamp(0.1, 1.0))
            } else if b > CLEAR {
                // Thin cloud over the sea: the dark ocean shows through as a blue haze
                // (Cobalt, never above 60), with a little swell texture.
                let sw = f64::from(map.sample(swell, x, y));
                let v = (b - CLEAR) / (THICK - CLEAR) * (0.07 + 0.05 * sw);
                ink::ramp(Ramp::Cobalt, v.min(60.0 / 255.0))
            } else {
                // Open ocean: as dark as the sea is from orbit (~5 % reflectance).
                Ink::BLANK
            }
        });
    }
}

impl Generator for Hurricane {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let display = Size::new(size.width(), size.height().saturating_mul(2).max(1));
        self.rebuild_lattice(grid::lattice_for(display, CAP, BUCKET));
        if !self.warmed {
            self.warmed = true;
            for _ in 0..SPIN_UP {
                self.step(SIM_DT);
            }
            self.t = 0.0;
        }
        self.pixels.resize(size);
        self.map.rebuild(self.lattice, self.pixels.pixel_size());
        self.shade();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt > 0.0 {
            self.step(dt);
        }
        self.shade();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.timeline.at(self.t).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn hur_a1_wind_peaks_at_the_radius_of_maximum_wind_and_turns_ccw() {
        let peak = (1..400)
            .map(|k| f64::from(k) * 0.002)
            .max_by(|a, b| holland(*a).total_cmp(&holland(*b)))
            .unwrap_or(0.0);
        assert!((peak - R_MAX_WIND).abs() < 0.003, "peak at {peak}");
        // East of the centre the wind blows up the screen (y down): counter-clockwise.
        let (_, v) = wind(R_MAX_WIND, 0.0);
        assert!(v < 0.0);
        let (u, _) = wind(0.0, -R_MAX_WIND);
        assert!(u < 0.0, "north of the centre the wind blows west");
    }

    #[test]
    fn hur_a2_a_tracer_spirals_inward_counter_clockwise() {
        let (mut x, mut y) = (0.8 * R_STORM, 0.0);
        let mut angle = 0.0f64; // unwrapped, counter-clockwise on screen
        let mut last_r = x.hypot(y);
        let dt = 0.01;
        for _ in 0..3000 {
            let (u, v) = wind(x, y);
            let (xm, ym) = (x + 0.5 * dt * u, y + 0.5 * dt * v);
            let (u, v) = wind(xm, ym);
            let before = (-y).atan2(x);
            x += dt * u;
            y += dt * v;
            let mut d = (-y).atan2(x) - before;
            d -= TAU * (d / TAU).round();
            assert!(d >= 0.0, "turned clockwise by {d}");
            angle += d;
            let r = x.hypot(y);
            assert!(r <= last_r + 1e-12, "moved outward");
            last_r = r;
        }
        assert!(last_r < 0.5 * R_STORM, "still at {last_r}");
        assert!(angle > PI);
    }

    fn storm() -> Hurricane {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(42, Size::new(120, 40));
        g.resize(Size::new(120, 40), &mut ctx);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        g
    }

    #[test]
    fn hur_a3_the_eye_is_clear() {
        let g = storm();
        let (w, h) = (
            usize::from(g.lattice.width()),
            usize::from(g.lattice.height()),
        );
        let (cx, cy) = g.centre(g.t);
        let (mut eye, mut ne, mut wall, mut nw) = (0.0, 0, 0.0, 0);
        for (k, &c) in g.cloud.as_slice().iter().enumerate() {
            let (dx, dy) = (
                (k % w) as f64 / h as f64 - cx,
                (k / w) as f64 / h as f64 - cy,
            );
            let r = dx.hypot(dy);
            if r < 0.1 * R_STORM {
                eye += f64::from(c);
                ne += 1;
            } else if (R_MAX_WIND..1.6 * R_MAX_WIND).contains(&r) {
                wall += f64::from(c);
                nw += 1;
            }
        }
        let (eye, wall) = (eye / f64::from(ne.max(1)), wall / f64::from(nw.max(1)));
        assert!(ne > 0 && nw > 0);
        assert!(eye < 0.1 * wall, "eye {eye} vs eyewall {wall}");
    }

    #[test]
    fn hur_a4_the_infrared_act_uses_div_inks_only() {
        let mut g = storm();
        let mut ctx = Ctx::new(1, Size::new(60, 20));
        g.resize(Size::new(60, 20), &mut ctx);
        let mid_ir = ACTS[0].1 + 0.5 * ACTS[1].1;
        while g.t < mid_ir {
            g.update(0.25, &mut ctx);
        }
        assert_eq!(g.caption(), Some("infrared"));
        let mut canvas = Canvas::new(Size::new(60, 20), GlyphMode::Unicode);
        g.render(&mut canvas);
        for cell in canvas.cells() {
            for ink in [cell.ink, cell.bg] {
                assert!(matches!(ink.role, Role::Div | Role::Background), "{ink:?}");
            }
        }
    }

    #[test]
    fn hur_a5_lattice_stays_within_its_cap() {
        let mut g = Hurricane::new();
        let mut ctx = Ctx::new(1, Size::new(500, 150));
        g.resize(Size::new(500, 150), &mut ctx);
        assert!(g.lattice.area() <= CAP, "{:?}", g.lattice);
        assert_eq!(g.cloud.size(), g.lattice);
    }
}
