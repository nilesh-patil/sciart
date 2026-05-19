//! The M87 jet: knots beamed near the speed of light (SPEC-002 `relativistic_jet`).
//!
//! A jet at `θ = 17°` to the line of sight carries knots at bulk speed `β`. Seen from
//! Earth a knot's apparent speed is `β sin θ / (1 − β cos θ)`, several times `c`, and
//! its light is boosted by `δ³`; the receding counter-jet is dimmed by `(δ'/δ)³` to
//! nothing. Positions along the jet are in jet lengths `s ∈ [0, 1]`; the scene's speed of
//! light is `C_UNIT` jet lengths per second (a time-lapse of decades).

use std::f64::consts::PI;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, disc_pixels, halo_kernel};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;
use sciart_core::{Tone, draw, ink, title};

pub static META: Meta = Meta {
    name: "relativistic_jet",
    category: Category::DeepCosmos,
    description: "The M87 jet: knots racing out at several times the apparent speed of light",
    title: "Black hole jet · beamed near c",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(RelativisticJet::default())
}

/// Viewing angle of the jet (17°) as its sine and cosine.
const SIN_T: f64 = 0.292_371_704_722_736_7;
const COS_T: f64 = 0.956_304_755_963_035_5;
/// Bulk speed of the jet body, and the knots' range.
const BETA_JET: f64 = 0.98;
const BETA_KNOTS: (f64, f64) = (0.95, 0.99);
/// The scene's speed of light, in jet lengths per second.
pub(crate) const C_UNIT: f64 = 1.0 / 90.0;
/// HST-1: where knots are born (jet lengths), its flare period and flare decay (s).
pub(crate) const S_HST: f64 = 0.07;
const FLARE_EVERY: f64 = 12.0;
const FLARE_TAU: f64 = 1.6;
/// Minor ejections between flares: mean interval (s).
const EJECT_EVERY: f64 = 3.2;
/// Knot pool (JET-A4).
pub(crate) const KNOTS: usize = 64;
/// Stationary brightenings along the jet (M87's knots D, E, F, I, A, B, C): position,
/// strength.
const FEATURES: [(f64, f64); 7] = [
    (0.22, 0.45),
    (0.33, 0.35),
    (0.43, 0.5),
    (0.52, 0.35),
    (0.63, 1.0),
    (0.72, 0.75),
    (0.84, 0.55),
];
/// Jet on screen: start of the jet (fraction of width), length (fraction of width),
/// position angle (radians, up from the right), half-opening and the base half-width
/// (pixels per jet length).
const CORE_X: f64 = 0.2;
const JET_LEN: f64 = 0.72;
const JET_PA: f64 = 0.12;
const OPENING: f64 = 0.036;
const BASE_WIDTH: f64 = 0.006;
/// Helical ripple: wavenumber (per jet length), drift (rad/s), amplitude (of the
/// half-width).
const HELIX_K: f64 = 38.0;
const HELIX_W: f64 = 0.9;
const HELIX_A: f64 = 0.55;
/// Host galaxy: effective radius (half-heights), axis ratio and peak.
const HOST_RE: f64 = 0.5;
const HOST_Q: f64 = 0.86;
const HOST_PEAK: f64 = 0.7;
/// Radius (in `r_e`) inside which the host is saturated.
const HOST_CORE: f64 = 0.04;
/// Globular clusters per cell.
const GC_PER_CELL: f64 = 1.0 / 55.0;
/// Readout ink (GEN-R13).
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
const KNOT_HALO: [(i32, i32, f32); 25] = halo_kernel(2);

/// Apparent transverse speed in units of `c` (JET-A1).
pub(crate) fn beta_app(beta: f64, sin_t: f64, cos_t: f64) -> f64 {
    beta * sin_t / (1.0 - beta * cos_t)
}

/// Doppler factor of an approaching (`sign = 1`) or receding (`sign = −1`) jet.
pub(crate) fn doppler(beta: f64, cos_t: f64, sign: f64) -> f64 {
    let gamma = 1.0 / (1.0 - beta * beta).sqrt();
    1.0 / (gamma * (1.0 - sign * beta * cos_t))
}

/// Counter-jet brightness relative to the jet's, `(δ'/δ)³` (JET-A3).
pub(crate) fn counter_ratio(beta: f64, cos_t: f64) -> f64 {
    (doppler(beta, cos_t, -1.0) / doppler(beta, cos_t, 1.0)).powi(3)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    beta: f64,
    age: f64,
    size: f64,
}

impl Knot {
    /// Position along the jet (jet lengths): straight out from HST-1 at `β_app c`.
    pub(crate) fn s(&self) -> f64 {
        S_HST + beta_app(self.beta, SIN_T, COS_T) * C_UNIT * self.age
    }

    fn brightness(&self) -> f64 {
        let boost = (doppler(self.beta, COS_T, 1.0) / doppler(BETA_JET, COS_T, 1.0)).powi(3);
        let s = self.s();
        let fade_in = ease::smoothstep(self.age / 0.8);
        let fade_out = 1.0 - ease::smoothstep((s - 0.85) / 0.2);
        boost * fade_in * fade_out * (1.0 - 0.45 * s)
    }
}

#[derive(Debug, Default)]
pub struct RelativisticJet {
    seeded: bool,
    t: f64,
    next_flare: f64,
    next_eject: f64,
    last_flare: f64,
    knots: ParticlePool<Knot>,
    /// β_app of the newest knot, for the readout.
    newest: f64,
    /// Screen: core (pixels), jet axis unit vector (screen, y down), length (pixels).
    core: (f64, f64),
    axis: (f64, f64),
    len: f64,
    half: f64,
    /// Static host light per pixel (tone level) and the globular clusters.
    host: Vec<u8>,
    clusters: Vec<(i32, i32, u8)>,
    glow: Glow,
    tone: Tone,
    pixels: Pixels,
    readout_room: usize,
}

impl RelativisticJet {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.tone = Tone::asinh(0.12, 2.4);
        // Start mid-stream: a few knots already on their way.
        self.t = 0.0;
        self.next_flare = ctx.rng.random_range(2.0..6.0);
        self.next_eject = 0.5;
        self.last_flare = -100.0;
        for _ in 0..6 {
            let beta = ctx.rng.random_range(BETA_KNOTS.0..BETA_KNOTS.1);
            let age = ctx.rng.random_range(1.0..16.0);
            let size = ctx.rng.random_range(0.7..1.2);
            self.knots.spawn(Knot { beta, age, size });
        }
        self.newest = beta_app(BETA_JET, SIN_T, COS_T);
    }

    fn launch(&mut self, ctx: &mut Ctx, big: bool) {
        let beta = ctx.rng.random_range(BETA_KNOTS.0..BETA_KNOTS.1);
        let size = if big {
            ctx.rng.random_range(1.1..1.5)
        } else {
            ctx.rng.random_range(0.6..1.0)
        };
        if self.knots.spawn(Knot {
            beta,
            age: 0.0,
            size,
        }) {
            self.newest = beta_app(beta, SIN_T, COS_T);
        }
    }

    /// Pixel position of jet coordinate `s` (jet lengths) at transverse offset `d`
    /// (pixels, left of the direction of motion).
    fn at(&self, s: f64, d: f64) -> (f64, f64) {
        let (ax, ay) = self.axis;
        (
            self.core.0 + ax * s * self.len + ay * d,
            self.core.1 + ay * s * self.len - ax * d,
        )
    }

    fn half_width(&self, s: f64) -> f64 {
        (BASE_WIDTH + OPENING * s) * self.len + 0.6
    }

    /// Jet-body emissivity at jet coordinate `s` and transverse offset `d` (pixels):
    /// a cone with a drifting double helix and the stationary knots, beamed by `δ³`.
    /// `sign = −1` is the counter-jet.
    pub(crate) fn body(&self, s: f64, d: f64, sign: f64) -> f64 {
        if !(0.0..=1.0).contains(&s) {
            return 0.0;
        }
        let w = self.half_width(s);
        let q = d / w;
        if q.abs() >= 1.0 {
            return 0.0;
        }
        let phase = HELIX_K * s - HELIX_W * self.t;
        let (h1, h2) = (HELIX_A * phase.sin_det(), -HELIX_A * phase.sin_det());
        let strand = |h: f64| (-((q - h) / 0.28).powi(2)).exp();
        let cone = 1.0 - q * q;
        let mut e = 0.18 + 0.08 * (1.0 - s);
        for &(at, k) in &FEATURES {
            e += 0.7 * k * (-((s - at) / 0.025).powi(2)).exp();
        }
        let start = ease::smoothstep(s / 0.05);
        let end = 1.0 - ease::smoothstep((s - 0.88) / 0.12);
        let beam = if sign > 0.0 {
            1.0
        } else {
            counter_ratio(BETA_JET, COS_T)
        };
        beam * start * end * e * (0.6 * cone + 0.4 * strand(h1) + 0.15 * strand(h2))
    }

    fn flare(&self) -> f64 {
        (-(self.t - self.last_flare).max(0.0) / FLARE_TAU).exp()
    }

    fn raster(&mut self) {
        self.glow.clear();
        let px = self.pixels.pixel_size();
        let (pw, ph) = (i32::from(px.width()), i32::from(px.height()));
        // Jet body over its bounding box, both sides of the core.
        let reach = self.half_width(1.0) + 2.0;
        let a = self.at(1.0, 0.0);
        let b = self.at(-1.0, 0.0);
        let (x0, x1) = (a.0.min(b.0) - reach, a.0.max(b.0) + reach);
        let (y0, y1) = (a.1.min(b.1) - reach, a.1.max(b.1) + reach);
        let (ax, ay) = self.axis;
        for y in (y0.floor() as i32).max(0)..=(y1.ceil() as i32).min(ph - 1) {
            for x in (x0.floor() as i32).max(0)..=(x1.ceil() as i32).min(pw - 1) {
                let (dx, dy) = (
                    f64::from(x) + 0.5 - self.core.0,
                    f64::from(y) + 0.5 - self.core.1,
                );
                let along = (dx * ax + dy * ay) / self.len;
                let d = dx * ay - dy * ax;
                let v = if along >= 0.0 {
                    self.body(along, d, 1.0)
                } else {
                    self.body(-along, -d, -1.0)
                };
                if v > 0.0 {
                    self.glow
                        .splat_bilinear(f64::from(x), f64::from(y), v as f32);
                }
            }
        }
        // Knots and the HST-1 flare.
        for k in self.knots.iter() {
            let (x, y) = self.at(k.s(), 0.0);
            let scale = (k.brightness() * k.size) as f32;
            self.glow.splat(2.0 * x, 2.0 * y, &KNOT_HALO, 0.9 * scale);
        }
        let (hx, hy) = self.at(S_HST, 0.0);
        self.glow.splat(
            2.0 * hx,
            2.0 * hy,
            &KNOT_HALO,
            (0.25 + 1.4 * self.flare()) as f32,
        );
        let (host, tone, values) = (&self.host, &self.tone, self.glow.values());
        let width = usize::from(px.width());
        self.pixels.shade(|x, y| {
            let i = y as usize * width + x as usize;
            let jet = tone.level(values.get(i).copied().unwrap_or(0.0));
            let h = host.get(i).copied().unwrap_or(0);
            if u16::from(jet) * 2 >= u16::from(h) && f64::from(jet) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Cobalt), jet)
            } else if f64::from(h) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Starlight), h)
            } else {
                Ink::BLANK
            }
        });
        self.pixels.dissolve(26);
        for &(x, y, l) in &self.clusters {
            let under = self.pixels.get(x, y).map_or(0, |i| i.level);
            if under < l / 2 {
                self.pixels
                    .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
            }
        }
        // The accretion-disc core: a hot point, flickering slightly.
        let kelvin = 10_500.0 + 1500.0 * (1.7 * self.t).sin_det();
        let r = 0.9 + 0.3 * self.flare();
        let pixels = &mut self.pixels;
        disc_pixels(self.core, r, |x, y, cover| {
            if cover > 0.35 {
                pixels.set(x, y, ink::blackbody(kelvin));
            }
        });
    }
}

impl Generator for RelativisticJet {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.pixels.resize(size);
        self.glow.resize(size);
        let px = self.pixels.pixel_size();
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        self.half = h / 2.0;
        self.core = (CORE_X * w, 0.55 * h);
        let (s, c) = JET_PA.sin_cos_det();
        self.axis = (c, -s);
        self.len = JET_LEN * w;
        self.readout_room = title::stable_readout_room(size, &META);
        // Host: de Vaucouleurs profile, toned once.
        let re = HOST_RE * self.half.max(1.0);
        let host_tone = Tone::asinh(0.03, 1.0);
        self.host.clear();
        for y in 0..px.height() {
            for x in 0..px.width() {
                let dx = (f64::from(x) + 0.5 - self.core.0) * HOST_Q;
                let dy = f64::from(y) + 0.5 - self.core.1;
                let r = dx.hypot(dy) / re;
                let i = HOST_PEAK * (-7.67 * (r.max(0.02).powf(0.25) - HOST_CORE.powf(0.25))).exp();
                self.host.push(host_tone.level(i.min(1.0) as f32 * 1.1));
            }
        }
        // Globular clusters: a swarm thinning outward from the core.
        self.clusters.clear();
        let n = (size.area() as f64 * GC_PER_CELL) as i32;
        for k in 0..n {
            let r = 0.15 + 2.2 * hash01(cell_key(k, 1)).powf(1.6);
            let a = 2.0 * PI * hash01(cell_key(k, 2));
            let (sa, ca) = a.sin_cos_det();
            let x = self.core.0 + ca * r * re * 1.2 / HOST_Q;
            let y = self.core.1 + sa * r * re;
            let l = 28 + (hash01(cell_key(k, 3)).powi(3) * 80.0) as u8;
            self.clusters.push((x.floor() as i32, y.floor() as i32, l));
        }
        self.raster();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        for k in self.knots.iter_mut() {
            k.age += dt;
        }
        self.knots.retain(|k| k.s() < 1.1);
        while self.t >= self.next_flare {
            self.last_flare = self.next_flare;
            self.next_flare += FLARE_EVERY * ctx.rng.random_range(0.85..1.15);
            self.launch(ctx, true);
        }
        while self.t >= self.next_eject {
            self.next_eject += EJECT_EVERY * ctx.rng.random_range(0.5..1.5);
            self.launch(ctx, false);
        }
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
        self.pixels.backdrop(canvas);
        // Readout: the newest knot's apparent speed, right-aligned on the title row.
        let mut label = StackText::<16>::new();
        let tenths = (self.newest * 10.0).round() as u32;
        let _ = std::fmt::Write::write_fmt(
            &mut label,
            format_args!("v_app {}.{}c", tenths / 10, tenths % 10),
        );
        let h = i32::from(canvas.size().height());
        if label.as_str().chars().count() <= self.readout_room {
            draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (RelativisticJet, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = RelativisticJet::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn jet_a1_apparent_speed_is_superluminal() {
        let (st, ct) = (17f64.to_radians().sin_det(), 17f64.to_radians().cos_det());
        assert!((st - SIN_T).abs() < 1e-15 && (ct - COS_T).abs() < 1e-15);
        let b = beta_app(0.98, SIN_T, COS_T);
        let direct = 0.98 * st / (1.0 - 0.98 * ct);
        assert!((b - direct).abs() < 1e-12);
        assert!(b > 1.0 && (4.0..5.0).contains(&b), "β_app = {b}");
    }

    #[test]
    fn jet_a2_knots_move_on_screen_at_beta_app() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let knot = Knot {
            beta: 0.97,
            age: 0.0,
            size: 1.0,
        };
        g.knots.clear();
        g.knots.spawn(knot);
        let p0 = g.at(g.knots.iter().next().unwrap().s(), 0.0);
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
        }
        let k = g
            .knots
            .iter()
            .find(|k| (k.beta - 0.97).abs() < 1e-12)
            .unwrap();
        let p1 = g.at(k.s(), 0.0);
        let px_per_s = (p1.0 - p0.0).hypot(p1.1 - p0.1) / (120.0 * SIM_DT);
        let in_c = px_per_s / (C_UNIT * g.len);
        let want = beta_app(0.97, SIN_T, COS_T);
        assert!((in_c / want - 1.0).abs() < 0.02, "{in_c} vs {want}");
    }

    #[test]
    fn jet_a3_counter_jet_is_beamed_away() {
        assert!(counter_ratio(BETA_JET, COS_T) < 1e-3);
        let (g, _) = built(Size::new(200, 50));
        let mut jet: f64 = 0.0;
        let mut counter: f64 = 0.0;
        for i in 1..200 {
            let s = f64::from(i) / 200.0;
            jet = jet.max(g.body(s, 0.0, 1.0));
            counter = counter.max(g.body(s, 0.0, -1.0));
        }
        assert!(counter < 0.01 * jet, "{counter} vs {jet}");
        // And on screen: no jet ink beyond a few pixels behind the core.
        let px = g.pixels.pixel_size();
        for y in 0..i32::from(px.height()) {
            for x in 0..(g.core.0 - 6.0) as i32 {
                let ink = g.pixels.get(x, y).unwrap();
                assert_ne!(ink.role, Role::Ramp(Ramp::Cobalt), "counter-jet at {x},{y}");
            }
        }
    }

    #[test]
    fn jet_a4_knots_are_pooled_and_start_at_hst1() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        g.knots.clear();
        for _ in 0..(120.0 / SIM_DT) as usize {
            let before = g.knots.len();
            g.update(SIM_DT, &mut ctx);
            assert!(g.knots.len() <= KNOTS);
            if g.knots.len() > before {
                let newest = g.knots.iter().last().unwrap();
                assert!((newest.s() - S_HST).abs() < 0.01);
            }
        }
    }

    #[test]
    fn jet_a5_readout_stays_right_of_the_title() {
        for size in [Size::new(80, 24), Size::new(40, 12), Size::new(200, 50)] {
            let mut stage = Stage::new(build(), 42, size, GlyphMode::Unicode);
            stage.set_title_visible(true);
            for _ in 0..240 {
                stage.step(SIM_DT);
            }
            let title = stage.title();
            let canvas = stage.render();
            if let Some((y, span)) = title::span(size, &title) {
                let row: String = (span.end..i32::from(size.width()))
                    .map(|x| canvas.get(x, y).map_or(' ', |c| c.ch))
                    .collect();
                let gap: String = (span.end..span.end + 2)
                    .map(|x| canvas.get(x, y).map_or(' ', |c| c.ch))
                    .collect();
                assert!(!gap.chars().any(char::is_alphanumeric), "{row:?}");
            }
        }
    }
}
