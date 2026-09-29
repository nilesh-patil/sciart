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
use sciart_core::rng::{cell_key, hash01, value_noise};
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
const CORE_X: f64 = 0.3;
const JET_LEN: f64 = 0.54;
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
/// Synchrotron colours: the jet violet to white at its brightest, the lobes teal.
const JET: Ramp = Ramp::Plasma;
const LOBE: Ramp = Ramp::Ice;
/// Radio plumes (jet lengths): the main lobe's half-width and how far its cocoon flows
/// back toward the core; the counter-lobe's hotspot distance, size and relative glow; the
/// backflow speed of the turbulence (jet lengths per second); and the lobes' level cap.
const LOBE_WIDTH: f64 = 0.17;
const LOBE_BACK: f64 = 0.42;
const COUNTER_AT: f64 = 0.44;
const COUNTER_SIZE: f64 = 0.7;
const COUNTER_GLOW: f64 = 0.45;
const BACKFLOW: f64 = 0.012;
const LOBE_MAX: u8 = 125;
/// Lobe pixels this far out (share of the local half-width) turn violet at the edge.
const LOBE_EDGE: f32 = 0.72;
const LOBE_EDGE_RAMP: Ramp = Ramp::Plasma;
/// The faint radio bridge from the core to the counter-lobe (unbeamed cocoon material).
const BRIDGE: f64 = 0.1;
const BRIDGE_WIDTH: f64 = 0.008;
/// The terminal hotspot (jet lengths), compact and bright.
const HOTSPOT: f64 = 0.95;
const HOTSPOT_CORE: [(i32, i32, f32); 9] = halo_kernel(1);

/// Radio-plume emissivity at offset `(u, v)` from its hotspot (jet lengths; `u` along
/// the jet's travel, negative back toward the core), and how far out it is across the
/// plume (0 on the axis, 1 at the edge). A turbulent cocoon: narrow at the head, widest
/// part-way back, fading toward the core, its texture (domain-warped value noise) drifting
/// back toward the core with the backflow; a bow-shock arc just ahead of the hotspot.
pub(crate) fn plume(u: f64, v: f64, t: f64, salt: u64, size: f64) -> (f64, f64) {
    let (width, back) = (LOBE_WIDTH * size, LOBE_BACK * size);
    let x = -u / back;
    if !(-0.25..=1.0).contains(&x) {
        return (0.0, 1.0);
    }
    let w = width * (x + 0.25).max(0.0).sqrt() * (1.0 - 0.55 * x.max(0.0).powi(2));
    let q = v.abs() / w.max(1e-9);
    // The bow shock: a thin arc ahead of the hotspot.
    let rho = u.hypot(0.8 * v) / size;
    let arc =
        0.15 * ease::smoothstep(u / (0.03 * size) + 0.3) * (-((rho - 0.055) / 0.012).powi(2)).exp();
    if q >= 1.25 {
        return (arc, 1.0);
    }
    let (fu, fv) = ((u + BACKFLOW * t) / size, v / size);
    let warp = value_noise(salt ^ 0x5eed, 9.0 * fu, 9.0 * fv, 1.0) - 0.5;
    let n = value_noise(salt, 24.0 * fu + 3.0 * warp, 24.0 * fv - 3.0 * warp, 1.0);
    let turbulence = 0.3 + 1.4 * n * n;
    let tail = 1.0 - ease::smoothstep((x - 0.55) / 0.45);
    let body = 0.17 * (1.0 - (q / 1.25).powi(2)).powf(1.5) * tail * turbulence;
    let head = 0.3 * (-(u / (0.035 * size)).powi(2) - (v / (0.045 * size)).powi(2)).exp();
    (body + head + arc, q / 1.25)
}

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
        // Knots flicker as their shocks brighten and fade.
        let pulse = 0.75 + 0.25 * (2.3 * self.age + 7.0 * self.size).sin_det();
        boost * fade_in * fade_out * (1.0 - 0.45 * s) * pulse
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
    /// The radio lobes (unbeamed).
    lobe_glow: Glow,
    /// Per pixel: how far out across its plume a lobe pixel lies (for the violet edge).
    lobe_q: Vec<f32>,
    salt: u64,
    tone: Tone,
    pixels: Pixels,
    readout_room: usize,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl RelativisticJet {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.salt = ctx.rng.random();
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
        self.lobe_glow.clear();
        let reach = self.half_width(1.0) + 1.3 * LOBE_WIDTH * self.len + 2.0;
        let a = self.at(HOTSPOT + 0.1, 0.0);
        let b = self.at(-COUNTER_AT - 0.1, 0.0);
        self.lobe_q.fill(1.0);
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
                // Lobes are not beamed: the counter-lobe shows, fainter, where the
                // counter-jet cannot, joined to the core by a faint radio bridge. Only
                // the main lobe gets the violet edge (the counter side must stay free
                // of jet-violet ink, JET-A3).
                let (lobe, q) = if along >= 0.0 {
                    plume(along - HOTSPOT, d / self.len, self.t, self.salt, 1.0)
                } else {
                    let (v, _) = plume(
                        -along - COUNTER_AT,
                        -d / self.len,
                        self.t,
                        self.salt ^ 1,
                        COUNTER_SIZE,
                    );
                    let s = -along;
                    let bridge = BRIDGE
                        * ease::smoothstep(s / 0.08)
                        * (1.0 - ease::smoothstep((s - COUNTER_AT + 0.05) / 0.1))
                        * (-(d / self.len / BRIDGE_WIDTH).powi(2)).exp();
                    (COUNTER_GLOW * v + bridge, 0.0)
                };
                if lobe > 0.0 {
                    self.lobe_glow
                        .splat_bilinear(f64::from(x), f64::from(y), lobe as f32);
                    if let Some(slot) = self.lobe_q.get_mut(y as usize * pw as usize + x as usize) {
                        *slot = q as f32;
                    }
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
        // The terminal hotspot, where the jet slams into the intergalactic gas.
        let (tx, ty) = self.at(HOTSPOT, 0.0);
        let pulse = 0.85 + 0.15 * (0.9 * self.t).sin_det();
        self.glow
            .splat(2.0 * tx, 2.0 * ty, &KNOT_HALO, (0.45 * pulse) as f32);
        self.glow
            .splat(2.0 * tx, 2.0 * ty, &HOTSPOT_CORE, (2.2 * pulse) as f32);
        let (host, tone, values) = (&self.host, &self.tone, self.glow.values());
        let (lobes, lobe_q) = (self.lobe_glow.values(), &self.lobe_q);
        let width = usize::from(px.width());
        self.pixels.shade(|x, y| {
            let i = y as usize * width + x as usize;
            let jet = tone.level(values.get(i).copied().unwrap_or(0.0));
            let lobe = tone
                .level(lobes.get(i).copied().unwrap_or(0.0))
                .min(LOBE_MAX);
            let h = host.get(i).copied().unwrap_or(0);
            if u16::from(jet) * 2 >= u16::from(h)
                && jet >= lobe
                && f64::from(jet) >= ink::MIN_VISIBLE_LEVEL
            {
                Ink::new(Role::Ramp(JET), jet)
            } else if lobe >= h && f64::from(lobe) >= ink::MIN_VISIBLE_LEVEL {
                let edge = lobe_q.get(i).is_some_and(|&q| q > LOBE_EDGE);
                Ink::new(Role::Ramp(if edge { LOBE_EDGE_RAMP } else { LOBE }), lobe)
            } else if f64::from(h) >= ink::MIN_VISIBLE_LEVEL {
                Ink::new(Role::Ramp(Ramp::Starlight), h)
            } else {
                Ink::BLANK
            }
        });
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
        self.lobe_glow.resize(size);
        let px = self.pixels.pixel_size();
        self.lobe_q.clear();
        self.lobe_q
            .resize(usize::from(px.width()) * usize::from(px.height()), 1.0);
        let (w, h) = (f64::from(px.width()), f64::from(px.height()));
        self.half = h / 2.0;
        self.core = (CORE_X * w, 0.55 * h);
        let (s, c) = JET_PA.sin_cos_det();
        self.axis = (c, -s);
        self.len = JET_LEN * w;
        self.readout_room = title::stable_readout_room(size, &META);
        // Host: de Vaucouleurs profile, toned once.
        let re = HOST_RE * self.half.max(1.0);
        let host_tone = Tone::asinh(0.09, 1.0);
        self.host.clear();
        for y in 0..px.height() {
            for x in 0..px.width() {
                let dx = (f64::from(x) + 0.5 - self.core.0) * HOST_Q;
                let dy = f64::from(y) + 0.5 - self.core.1;
                let r = dx.hypot(dy) / re;
                let i = HOST_PEAK * (-7.67 * (r.max(0.02).powf(0.25) - HOST_CORE.powf(0.25))).exp();
                // Tapered so the halo sinks into the sky with no visible rim.
                // The taper works on the toned level, so the halo reaches the faintest
                // visible level gently and the cut there is one near-black step.
                let taper = 1.0 - ease::smoothstep((r - 0.35) / 0.9);
                let level = f64::from(host_tone.level(i.min(1.0) as f32 * 1.1)) * taper;
                self.host.push(level as u8);
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
        self.dirty = true;
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
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
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
        g.prepare();
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
            g.prepare();
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
                assert_ne!(ink.role, Role::Ramp(JET), "counter-jet at {x},{y}");
            }
        }
    }

    #[test]
    fn jet_a6_lobes_are_plumes_trailing_back_from_the_hotspot() {
        // Half-width (the first v where the plume ends) at a point along it.
        let half = |u: f64| {
            (0..400)
                .map(|i| f64::from(i) * 0.001)
                .find(|&v| plume(u, v, 0.0, 7, 1.0).1 >= 1.0)
                .unwrap_or(0.4)
        };
        assert!(
            half(-0.15) > 1.5 * half(0.02),
            "{} vs {}",
            half(-0.15),
            half(0.02)
        );
        // Nothing far ahead of the hotspot, and the plume closes toward the core.
        assert_eq!(plume(0.2, 0.0, 0.0, 7, 1.0).0, 0.0);
        assert_eq!(plume(-LOBE_BACK * 1.01, 0.0, 0.0, 7, 1.0).0, 0.0);
        // The hotspot end is the brightest part; the texture drifts with the backflow.
        let head = plume(0.0, 0.0, 0.0, 7, 1.0).0;
        let body = (1..40).map(|i| plume(-0.01 * f64::from(i) - 0.1, 0.0, 0.0, 7, 1.0).0);
        assert!(body.fold(0.0, f64::max) < head);
        let moved = (0..40).any(|i| {
            let u = -0.1 - 0.005 * f64::from(i);
            (plume(u, 0.02, 0.0, 7, 1.0).0 - plume(u, 0.02, 5.0, 7, 1.0).0).abs() > 1e-3
        });
        assert!(moved);
    }

    #[test]
    fn jet_a4_knots_are_pooled_and_start_at_hst1() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        g.knots.clear();
        for _ in 0..(120.0 / SIM_DT) as usize {
            let before = g.knots.len();
            g.update(SIM_DT, &mut ctx);
            g.prepare();
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
