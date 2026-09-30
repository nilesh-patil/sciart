//! A long exposure of the northern sky (SPEC-002 `star_trails`): concentric arcs grow
//! around the celestial pole above a treeline, each in its star's own colour, as Earth
//! turns under the camera.
//!
//! The camera is a fisheye pointed at the pole (azimuthal equidistant projection): a
//! star's distance from the pole on screen is `f · ρ` for its angular pole distance `ρ`,
//! so every trail is an exact circle about the pole and its length on screen is `∝ ρ`
//! (TRL-A1, TRL-A2). The exposure is a per-dot buffer of star temperatures, painted as
//! the stars move and faded out dot by dot at the end of each stacked exposure.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::{self, Glow, Tone};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

use crate::shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "star_trails",
    category: Category::EarthSky,
    description: "A long exposure: coloured star arcs grow around the pole above a treeline",
    title: "Star trails · Earth turns",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(StarTrails::default())
}

// --- The sky --------------------------------------------------------------------------

/// Earth's sidereal rotation rate (rad/s): 15.04°/h.
const SIDEREAL: f64 = TAU / 86_164.1;
/// Time-lapse factor: an exposure of 45 s spans 10 h of sky, arcs of ~150°.
const TIME_LAPSE: f64 = 800.0;
/// Exposure and fade (s) of one stacked exposure (TRL-A4).
const EXPOSE: f64 = 45.0;
const FADE: f64 = 5.0;
/// Seeded observer latitude range (deg): the pole stands this high.
const LAT: (f64, f64) = (35.0, 50.0);
/// Framing: at most this half field of view across (deg), the horizon at this share of
/// the height, and the pole at most this share of the height above it.
const HALF_FOV: f64 = 42.0;
const HORIZON_ROW: f64 = 0.84;
const POLE_DROP: f64 = 0.62;
/// A near forest: the camera stands at its edge, so the spruces rise well into the sky.
const NEAR_FOREST: Terrain = Terrain {
    ridge: (0.8, 2.5),
    trees: Some((5.0, 17.0)),
    gap: (1.4, 5.0),
    clearing: 0.25,
};
/// Visible stars per cell and the pool cap.
const DENSITY: f64 = 0.022;
const MAX_STARS: usize = 2500;
/// Faintest magnitude drawn; trails of stars brighter than `THICK` are two dots wide.
/// Every trail is a solid line: a stippled one reads as noise, not as an arc.
const MAG_LIMIT: f64 = 6.0;
const THICK: f64 = 1.5;
/// Stars fainter than this draw no braille line: their light goes into a soft exposure
/// (`Glow`) toned onto the sky in its ramp, so the many faint trails are a dim
/// haze of rings under the few bright coloured ones (a brightness hierarchy by level).
const FAINT: f64 = 4.4;
/// Faint exposure: light per second for a magnitude-`MAG_LIMIT` star, its tone (knee,
/// white) and the share of the tone's range it may use over the sky.
const FAINT_GAIN: f64 = 0.12;
const FAINT_TONE: (f32, f32) = (0.5, 8.0);
const FAINT_SHARE: f64 = 0.36;
/// Faint exposure per dot of arc, matching the old per-second rate at 40 px from the pole.
const ARC_GAIN: f64 = 0.35;
/// Seconds of exposure already taken when the scene opens, so the first frame shows arcs.
const PREROLL: f64 = 10.0;
/// The preroll's time step (s).
const PREROLL_STEP: f64 = 0.1;
/// Naked-eye spectral mix `(cumulative fraction, lowest K, highest K)`: the bright stars
/// are mostly hot B/A stars and cool K/M giants.
const CLASSES: [(f64, f64, f64); 5] = [
    (0.22, 3200.0, 4000.0),
    (0.50, 4100.0, 5200.0),
    (0.64, 5300.0, 6400.0),
    (0.84, 7000.0, 10_000.0),
    (1.00, 11_000.0, 25_000.0),
];
/// Colour saturation, as in a processed photograph: temperatures are pushed away from
/// the 6500 K white point by this power, and clamped onto the drawable range.
const SATURATE: f64 = 1.15;
const KELVIN_RANGE: (f64, f64) = (2700.0, 30_000.0);
/// Sky levels (ADR-0017): the night sky is black; `SKY_FLOOR` only marks a pixel as sky
/// (0 is land). A barely-lit skyglow sits on the horizon (`GLOW_LEVEL`, e-folding over
/// `GLOW_DEG`), just enough for the treeline to show as a silhouette.
const SKY_FLOOR: f64 = 1.0;
const GLOW_LEVEL: f64 = 22.0;
const GLOW_DEG: f64 = 2.5;
/// The level under which a sky pixel is drawn black.
const SKY_BLACK: f64 = 12.0;

/// The barely lit horizon (one flat `Dim` step, ADR-0018): just enough for the treeline to
/// show as a silhouette; the rest of the sky is black.
const SKYGLOW: Ink = Ink::new(Role::Dim, 0);
/// Faint trails are dim braille dots on the bright trails' lattice (ADR-0018: no
/// half-block haze): a dot lights where its fixed hash falls below its pixel's toned
/// faint light, none below `FAINT_FLOOR`.
const FAINT_DOT: Ink = Ink::new(Role::Dim, 150);
const FAINT_FLOOR: f64 = 12.0;

/// A bright trail's colour by its star's temperature (the exposure keeps the blackbody
/// level of the hottest trail through a dot, a model quantity): M11's stepped palette,
/// cool giants orange, Sun-like yellow, F white, A cyan-white, B blue (ADR-0018).
pub(crate) const TRAIL: [(f64, Ink); 5] = [
    (4300.0, Ink::new(Role::Hot, 210)),
    (5500.0, Ink::new(Role::Sun, 210)),
    (7200.0, Ink::new(Role::Star, 220)),
    (11_000.0, Ink::new(Role::Wave, 200)),
    (f64::INFINITY, Ink::new(Role::Cold, 230)),
];

/// The trail ink for exposure level `level` (a blackbody level).
fn trail_ink(level: u8) -> Ink {
    let k = ink::blackbody_kelvin(Ink::new(Role::Blackbody, level));
    TRAIL
        .iter()
        .find(|(below, _)| k < *below)
        .map_or(TRAIL[4].1, |t| t.1)
}

/// The Earth's rotation angle after `t` seconds of the time-lapse.
pub(crate) fn rotation(t: f64) -> f64 {
    SIDEREAL * TIME_LAPSE * t
}

#[cfg(test)]
/// Altitude (rad) of a star `rho` from the pole at hour angle `h` for latitude `lat`:
/// `sin a = sin φ cos ρ + cos φ sin ρ cos h`.
pub(crate) fn altitude(lat: f64, rho: f64, h: f64) -> f64 {
    (lat.sin_det() * rho.cos_det() + lat.cos_det() * rho.sin_det() * h.cos_det())
        .clamp(-1.0, 1.0)
        .asin()
}

/// The star at offset `(x, y)` from the pole (y down) after the sky turns by `theta`:
/// counter-clockwise on screen, as the northern sky turns for a camera facing north.
#[inline]
pub(crate) fn turn((x, y): (f64, f64), (s, c): (f64, f64)) -> (f64, f64) {
    (x * c + y * s, -x * s + y * c)
}

/// A temperature pushed away from white by [`SATURATE`].
fn saturate(kelvin: f64) -> f64 {
    (6500.0 * (kelvin / 6500.0).powf(SATURATE)).clamp(KELVIN_RANGE.0, KELVIN_RANGE.1)
}

#[derive(Debug, Clone, Copy)]
struct Star {
    /// Offset from the pole at rotation 0, in dots.
    off: (f64, f64),
    /// `Blackbody` level of its trail.
    level: u8,
    /// Whether the trail is two dots wide.
    thick: bool,
    /// Faint stars expose the soft layer at this rate (light per second); 0 for a
    /// braille trail.
    faint: f32,
    /// Position (dots) last step, for the faint exposure.
    prev: Option<(f64, f64)>,
    /// Dot last painted, if the star was up then.
    last: Option<(i32, i32)>,
}

#[derive(Debug, Default)]
pub struct StarTrails {
    stars: Vec<Star>,
    /// Pole position in dots and the projection scale (dots per radian).
    pole: (f64, f64),
    scale: f64,
    /// The silhouette's top per dot column, in dot rows: stars below it are hidden.
    sil: Vec<f32>,
    /// The exposure: per dot, the `Blackbody` level of the hottest trail through it, 0
    /// for none. Sized once per resize and never grown (TRL-A4).
    exposure: Vec<u8>,
    dots_w: usize,
    dots_h: usize,
    /// Per dot, its place in the fade (a hashed byte: dots whose rank is below the fade
    /// level are gone), and the fade level now (0 while exposing).
    rank: Vec<u8>,
    fade: u16,
    /// The sky's fixed level per pixel (0 = land).
    base: Vec<u8>,
    /// The faint trails' soft exposure, and its toned dot density per pixel.
    haze: Glow,
    haze_tone: Tone,
    faint: Vec<u8>,
    sky: Pixels,
    /// Time since the first exposure began, and the exposure's index.
    t: f64,
    cycle: i64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl StarTrails {
    fn loop_len() -> f64 {
        EXPOSE + FADE
    }

    /// Where the pole and horizon go, per size: the pole about a third down, the north
    /// horizon `f · φ` below it, the sides no more than ~72° from the pole.
    fn frame(px_w: f64, px_h: f64, lat: f64) -> ((f64, f64), f64) {
        let f = (0.5 * px_w / HALF_FOV.to_radians().tan())
            .min(POLE_DROP * px_h / lat.tan())
            .max(1.0);
        let py = (HORIZON_ROW * px_h - f * lat.tan()).max(0.0);
        ((0.5 * px_w, py), f)
    }

    /// Paint the exposure from the stars' moves since the last step.
    fn expose(&mut self) {
        let theta = rotation(self.t).sin_cos_det();
        let (dw, dh) = (self.dots_w as i32, self.dots_h as i32);
        for s in &mut self.stars {
            let (x, y) = turn(s.off, theta);
            let (x, y) = (self.pole.0 + x, self.pole.1 + y);
            if s.faint > 0.0 {
                // Bilinear splats every half pixel of the move since the last step, so the
                // soft arc is smooth and its exposure is light × time spent (a slow star
                // near the pole burns a brighter trail).
                let (ix, iy) = (x.round() as i32, y.round() as i32);
                let up =
                    ix >= 0 && ix < dw && iy >= 0 && iy < dh && (iy as f32) < self.sil[ix as usize];
                if up {
                    let (px, py) = (x, y);
                    let from = s.prev.unwrap_or((px, py));
                    let len = (px - from.0).hypot(py - from.1);
                    let n = (len.ceil() as usize).clamp(1, 64);
                    // Light per dot of arc (not per second): every faint trail has an even
                    // surface brightness, so the slow stars round the pole do not pile up
                    // into a blot (stylised; a real exposure burns them brighter).
                    let w = s.faint * (ARC_GAIN * len) as f32 / n as f32;
                    for k in 0..n {
                        let f = (k as f64 + 0.5) / n as f64;
                        let (qx, qy) = (from.0 + (px - from.0) * f, from.1 + (py - from.1) * f);
                        self.haze
                            .splat_bilinear((qx - 0.5) / 2.0, (qy - 0.5) / 2.0, w);
                    }
                    s.prev = Some((px, py));
                } else {
                    s.prev = None;
                }
                continue;
            }
            let (ix, iy) = (x.round() as i32, y.round() as i32);
            let up =
                ix >= 0 && ix < dw && iy >= 0 && iy < dh && (iy as f32) < self.sil[ix as usize];
            if !up {
                s.last = None;
                continue;
            }
            let from = s.last.unwrap_or((ix, iy));
            // The line starts on the dot painted last step: skip it then.
            let skip = usize::from(s.last.is_some());
            for (px, py) in sciart_core::geom::line(from.0, from.1, ix, iy).skip(skip) {
                paint(
                    &mut self.exposure,
                    self.dots_w,
                    &self.sil,
                    (px, py),
                    s,
                    (x - self.pole.0, y - self.pole.1),
                );
            }
            s.last = Some((ix, iy));
        }
    }

    fn restart(&mut self) {
        self.exposure.fill(0);
        self.haze.clear();
        for s in &mut self.stars {
            s.last = None;
            s.prev = None;
        }
    }

    /// Tone the faint exposure into per-pixel dot densities (drawn as dim braille dots on
    /// the trails' lattice, dimmed by the fade), and shade the fixed sky: black but for
    /// the barely lit horizon.
    fn tone_sky(&mut self) {
        let keep = 1.0 - f64::from(self.fade) / 256.0;
        let (haze, tone) = (self.haze.values(), &self.haze_tone);
        for (d, &h) in self.faint.iter_mut().zip(haze) {
            let t = FAINT_SHARE * f64::from(tone.level(h)) * keep;
            *d = if t < FAINT_FLOOR {
                0
            } else {
                t.min(255.0) as u8
            };
        }
        let base = &self.base;
        let w = usize::from(self.sky.pixel_size().width());
        self.sky.shade(|x, y| {
            let l = f64::from(base.get(y as usize * w + x as usize).copied().unwrap_or(0));
            if l < SKY_BLACK { Ink::BLANK } else { SKYGLOW }
        });
    }
}

/// Paint one trail dot of `s` (and its outer neighbour for a thick trail).
fn paint(
    exposure: &mut [u8],
    w: usize,
    sil: &[f32],
    (x, y): (i32, i32),
    s: &Star,
    radial: (f64, f64),
) {
    let mut put = |x: i32, y: i32| {
        let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        if ux >= w || sil.get(ux).is_none_or(|&top| y as f32 >= top) {
            return;
        }
        if let Some(e) = exposure.get_mut(uy * w + ux)
            && s.level > *e
        {
            *e = s.level;
        }
    };
    put(x, y);
    if s.thick {
        let r = radial.0.hypot(radial.1).max(1e-9);
        put(
            x + (radial.0 / r).round() as i32,
            y + (radial.1 / r).round() as i32,
        );
    }
}

impl Generator for StarTrails {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let (px_w, px_h) = (w as f64, 2.0 * h as f64);
        let lat = ctx.rng.random_range(LAT.0..LAT.1).to_radians();
        let (pole_px, f) = Self::frame(px_w, px_h, lat);
        self.pole = (2.0 * pole_px.0, 2.0 * pole_px.1);
        self.scale = 2.0 * f;

        // Gnomonic: the horizon, a great circle, is the straight row `f · tan φ` below the
        // pole (px per degree taken at the frame centre).
        let horizon = pole_px.1 + f * lat.tan();
        let deg = f * PI / 180.0;
        let land = Skyline::new(&mut ctx.rng, w, deg, NEAR_FOREST);

        // Sky: skyglow rising toward the horizon, the land a black silhouette over it.
        self.sky.resize(size);
        self.haze.resize(size);
        self.faint.clear();
        self.faint.resize(2 * w * h, 0);
        self.haze_tone = Tone::asinh(FAINT_TONE.0, FAINT_TONE.1);
        self.base.clear();
        for y in 0..2 * h as i32 {
            for x in 0..w as i32 {
                let elev = ((horizon - (f64::from(y) + 0.5)) / deg).max(0.0);
                let glow = SKY_FLOOR + GLOW_LEVEL * (-elev / GLOW_DEG).exp();
                let open = 1.0 - f64::from(land.cover(x, y, horizon));
                let l = if open < 0.5 {
                    0.0
                } else {
                    glow * open.max(SKY_FLOOR / glow)
                };
                self.base.push(ink::level(l));
            }
        }

        // Trails end where a star sinks behind the land.
        self.dots_w = 2 * w;
        self.sil = (0..self.dots_w)
            .map(|xd| {
                let x = (xd as f64 + 0.5) / 2.0;
                (2.0 * (horizon - land.height_at(x))) as f32
            })
            .collect();

        // Stars uniform on the cap of sky the frame can show.
        let corner = [(0.0, 0.0), (px_w, 0.0), (0.0, px_h), (px_w, px_h)]
            .iter()
            .map(|&(x, y)| (x - pole_px.0).hypot(y - pole_px.1))
            .fold(0.0, f64::max);
        let rho_max = (corner / f).atan();
        let cap = TAU * (1.0 - rho_max.cos_det());
        // The frame's solid angle: a pixel `r` from the centre spans `f / (f² + r²)^{3/2}`
        // steradians per unit area... times f; summed on a coarse grid.
        let mut frame = 0.0;
        for i in 0..16 {
            for j in 0..16 {
                let x = (f64::from(i) + 0.5) / 16.0 * px_w - pole_px.0;
                let y = (f64::from(j) + 0.5) / 16.0 * px_h - pole_px.1;
                frame += f / (f * f + x * x + y * y).powf(1.5) * (px_w * px_h / 256.0);
            }
        }
        let n = ((DENSITY * (w * h) as f64 * cap / frame.max(1e-9)) as usize).clamp(1, MAX_STARS);
        let mut stars = Vec::with_capacity(n + 1);
        // Polaris: 0.7° from the pole, a tiny bright circle (F7 supergiant).
        stars.push((
            0.7f64.to_radians(),
            ctx.rng.random_range(0.0..TAU),
            2.0,
            6000.0,
        ));
        for _ in 0..n {
            let rho = (1.0 - ctx.rng.random_range(0.0..1.0) * (1.0 - rho_max.cos_det()))
                .clamp(-1.0, 1.0)
                .acos();
            let h = ctx.rng.random_range(0.0..TAU);
            // N(< m) ∝ 10^{0.5 m}: faint stars dominate.
            let m = MAG_LIMIT + 2.0 * ctx.rng.random_range(1e-3f64..1.0).log10();
            let k = ink::class_kelvin(&CLASSES, ctx.rng.random(), ctx.rng.random());
            stars.push((rho, h, m.max(-1.0), k));
        }
        // One buffer at the cap: the star count varies with the seeded latitude, and a
        // resize must not grow the heap (TEST-R6).
        self.stars.clear();
        self.stars.reserve_exact(MAX_STARS + 1);
        self.stars.extend(stars.into_iter().map(|(rho, h, m, k)| {
            let (s, c) = h.sin_cos_det();
            let r = self.scale * rho.tan();
            Star {
                off: (-r * s, -r * c),
                level: ink::blackbody(saturate(k)).level,
                thick: m < THICK,
                faint: if m > FAINT {
                    (FAINT_GAIN * 10f64.powf(-0.4 * (m - MAG_LIMIT))) as f32
                } else {
                    0.0
                },
                prev: None,
                last: None,
            }
        }));

        self.dots_h = 4 * h;
        self.exposure.clear();
        self.exposure.resize(self.dots_w * self.dots_h, 0);
        self.fade = 0;
        let dw = self.dots_w;
        self.rank.clear();
        self.rank.extend((0..self.exposure.len()).map(|i| {
            let (x, y) = ((i % dw) as i32, (i / dw) as i32);
            (hash01(cell_key(x, y) ^ 0x7e11) * 256.0) as u8
        }));
        // A resize restarts the exposure (the old picture no longer fits the frame).
        self.t = 0.0;
        self.cycle = 0;
        self.restart();
        // The exposure is already under way when the scene opens.
        // In `PREROLL_STEP` strides (GEN-R19): a bright trail is the max of the dots it
        // crosses and a faint one gathers light per dot of arc, so the picture does not
        // depend on the stride; at 0.1 s a step turns the sky by a fraction of a degree.
        let steps = (PREROLL / PREROLL_STEP).round() as usize;
        for k in 0..=steps {
            self.t = k as f64 * PREROLL_STEP;
            self.expose();
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        let cycle = (self.t / Self::loop_len()).floor() as i64;
        if cycle != self.cycle {
            self.cycle = cycle;
            self.restart();
        }
        let phase = self.t - cycle as f64 * Self::loop_len();
        if phase < EXPOSE {
            self.fade = 0;
            self.expose();
        } else {
            self.fade = (256.0 * ease::smoothstep((phase - EXPOSE) / FADE)) as u16;
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.tone_sky();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        // The skyglow has no ASCII form: there the trails alone draw the sky.
        self.sky.blit(canvas, |_, _, _, _| None);
        // The exposure drawn straight as braille, each cell in its hottest trail's colour
        // over the sky's colour; while fading, a hashed share of the dots is left out (so
        // the fade costs no more than an ordinary frame).
        let unicode = canvas.mode() == GlyphMode::Unicode;
        let (dw, dh) = (self.dots_w, self.dots_h);
        for cy in 0..dh / 4 {
            for cx in 0..dw / 2 {
                let (mut mask, mut level, mut faint) = (0u8, 0u8, 0u8);
                for dy in 0..4 {
                    let row = (4 * cy + dy) * dw + 2 * cx;
                    for dx in 0..2 {
                        let v = self.exposure.get(row + dx).copied().unwrap_or(0);
                        // The trees hide the trails: no dot over a land pixel (the
                        // silhouette test when painting sees one height per dot column,
                        // which can slip between two spires).
                        let px = (2 * cy + dy / 2) * (dw / 2) + cx;
                        if self.base.get(px).is_none_or(|&b| b == 0) {
                            continue;
                        }
                        let rank = self.rank.get(row + dx).copied().unwrap_or(0);
                        let (x, y) = ((2 * cx + dx) as i32, (4 * cy + dy) as i32);
                        if v == 0 {
                            // A faint trail's dot: lit by its fixed hash against the
                            // pixel's toned faint light (already faded).
                            if rank < self.faint.get(px).copied().unwrap_or(0) {
                                faint |= raster::dot_bit(x, y);
                            }
                            continue;
                        }
                        if u16::from(rank) < self.fade {
                            continue;
                        }
                        mask |= raster::dot_bit(x, y);
                        level = level.max(v);
                    }
                }
                if mask | faint != 0 {
                    let (x, y) = (cx as i32, cy as i32);
                    let bg = if unicode {
                        self.sky.cell_bg(x, y)
                    } else {
                        Ink::BLANK
                    };
                    // A cell holds one colour: a bright trail's, else the faint dots'.
                    let ink = if mask != 0 {
                        trail_ink(level)
                    } else {
                        FAINT_DOT
                    };
                    canvas.put_bg(x, y, raster::braille(mask | faint), ink, bg);
                }
            }
        }
    }
}

/// Angular distance (rad) of dot `(x, y)` from the pole, for tests.
#[cfg(test)]
fn pole_distance(g: &StarTrails, (x, y): (f64, f64)) -> f64 {
    (x - g.pole.0).hypot(y - g.pole.1) / g.scale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (StarTrails, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = StarTrails::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn trl_a1_trail_points_stay_at_constant_pole_distance() {
        let (g, _) = built(Size::new(120, 40));
        for s in g.stars.iter().take(200) {
            let r0 = s.off.0.hypot(s.off.1);
            for i in 0..=450 {
                let t = f64::from(i) * EXPOSE / 450.0;
                let (x, y) = turn(s.off, rotation(t).sin_cos_det());
                let r = pole_distance(&g, (g.pole.0 + x, g.pole.1 + y)) * g.scale;
                // 0.5 px = 1 dot.
                assert!((r - r0).abs() < 1.0, "{r} vs {r0}");
            }
        }
    }

    #[test]
    fn trl_a2_arcs_share_one_angle_and_scale_with_pole_distance() {
        let (g, _) = built(Size::new(200, 50));
        let angle = rotation(EXPOSE);
        // 15.04°/h × 45 s × 800 = 150°.
        assert!(
            (angle.to_degrees() - 150.4).abs() < 0.5,
            "{}",
            angle.to_degrees()
        );
        let arc = |s: &Star| {
            let mut len = 0.0;
            let mut p = s.off;
            for i in 1..=900 {
                let q = turn(s.off, rotation(f64::from(i) * EXPOSE / 900.0).sin_cos_det());
                len += (q.0 - p.0).hypot(q.1 - p.1);
                p = q;
            }
            len
        };
        for s in g.stars.iter().skip(1).take(50) {
            let r = s.off.0.hypot(s.off.1);
            let swept = arc(s) / r;
            assert!((swept - angle).abs() < 1e-3, "{swept} vs {angle}");
        }
    }

    #[test]
    fn trl_a3_circumpolar_stars_never_set() {
        for lat_deg in [35.0f64, 45.0, 52.0] {
            let lat = lat_deg.to_radians();
            for rho_deg in [1.0, 10.0, lat_deg - 0.5] {
                let rho = f64::to_radians(rho_deg);
                for i in 0..360 {
                    let h = f64::from(i).to_radians();
                    assert!(altitude(lat, rho, h) > 0.0, "{lat_deg} {rho_deg} {i}");
                }
            }
            // A star farther than the latitude from the pole dips below at lower transit.
            assert!(altitude(lat, (lat_deg + 5.0).to_radians(), PI) < 0.0);
        }
    }

    #[test]
    fn trl_a4_exposure_is_allocated_once_and_resets_each_loop() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let (ptr, cap) = (g.exposure.as_ptr(), g.exposure.capacity());
        let lit = |g: &StarTrails| g.exposure.iter().filter(|&&l| l > 0).count();
        let mut peak = 0;
        for _ in 0..((EXPOSE - PREROLL) * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            peak = peak.max(lit(&g));
        }
        assert_eq!((g.exposure.as_ptr(), g.exposure.capacity()), (ptr, cap));
        assert!(peak > 2000, "{peak}");
        // Through the fade and into the next exposure: the buffer starts over.
        for _ in 0..(FADE * 60.0) as usize + 2 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!((g.exposure.as_ptr(), g.exposure.capacity()), (ptr, cap));
        assert!(lit(&g) < peak / 20, "{} vs {peak}", lit(&g));
    }

    #[test]
    fn trl_a5_trail_inks_follow_star_colour() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        for _ in 0..600 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
        g.render(&mut canvas);
        let (mut n, mut faint) = (0, 0);
        for c in canvas.cells() {
            if ('\u{2801}'..='\u{28ff}').contains(&c.ch) {
                if c.ink == FAINT_DOT {
                    faint += 1;
                    continue;
                }
                assert!(TRAIL.iter().any(|t| t.1 == c.ink), "{:?}", c.ink);
                n += 1;
            }
        }
        assert!(n > 50 && faint > 0, "{n} trail cells, {faint} faint");
    }

    #[test]
    fn trl_a7_the_sky_stays_black_between_the_arcs() {
        // ADR-0018: faint trails are dim braille dots on the trails' lattice, not a
        // half-block haze: at 20 s the sky's pixel layer is black above the horizon glow.
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        for _ in 0..(20.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let ps = g.sky.pixel_size();
        let lit = (0..i32::from(ps.height()) / 2)
            .flat_map(|y| (0..i32::from(ps.width())).map(move |x| (x, y)))
            .filter(|&(x, y)| g.sky.get(x, y).is_some_and(|p| p != Ink::BLANK))
            .count();
        assert_eq!(lit, 0, "{lit} lit sky pixels in the upper half");
    }

    #[test]
    fn trl_a6_black_sky_with_a_barely_lit_horizon_and_no_trail_over_trees() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = built(size);
        for _ in 0..1800 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        // The sky: black overhead, a barely-lit glow just above the land (ADR-0017).
        let w = usize::from(g.sky.pixel_size().width());
        let col = w / 2;
        let levels: Vec<u8> = g.base.iter().skip(col).step_by(w).copied().collect();
        let top = levels[0];
        let low = levels
            .iter()
            .copied()
            .take_while(|&b| b > 0)
            .last()
            .unwrap_or(0);
        assert!(top <= 2 && low > top && low <= 24, "{top} → {low}");
        let top_ink = g.sky.get(col as i32, 0).unwrap_or(Ink::BLANK);
        // Black, or a faint trail's grey haze.
        assert!(
            top_ink == Ink::BLANK || matches!(top_ink.role, Role::Dim | Role::Structure),
            "{top_ink:?}"
        );
        // No braille trail cell where both of its pixels are land.
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        for y in 0..40 {
            for x in 0..120 {
                let land = |py: usize| g.base.get(py * w + x).is_none_or(|&b| b == 0);
                if land(2 * y) && land(2 * y + 1) {
                    let c = canvas.get(x as i32, y as i32).map_or(' ', |c| c.ch);
                    assert!(!('\u{2801}'..='\u{28ff}').contains(&c), "({x},{y})");
                }
            }
        }
    }

    #[test]
    fn trl_trees_hide_the_trails_below_them() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for _ in 0..1200 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        for (i, &l) in g.exposure.iter().enumerate() {
            if l > 0 {
                let (x, y) = (i % g.dots_w, i / g.dots_w);
                assert!((y as f32) < g.sil[x]);
            }
        }
    }
}
