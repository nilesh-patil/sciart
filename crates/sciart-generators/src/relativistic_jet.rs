//! The M87 jet: knots beamed near the speed of light (SPEC-002 `relativistic_jet`).
//!
//! A jet at `θ = 17°` to the line of sight carries knots at bulk speed `β`. Seen from
//! Earth a knot's apparent speed is `β sin θ / (1 − β cos θ)`, several times `c`, and
//! its light is boosted by `δ³`; the receding counter-jet is dimmed by `(δ'/δ)³` to
//! nothing. Positions along the jet are in jet lengths `s ∈ [0, 1]`; the scene's speed of
//! light is `C_UNIT` jet lengths per second (a time-lapse of decades).
//!
//! M16 (GEN-R25, thread style): both jets are helical field-line threads (the counter-jet
//! a sparse, faint one) from a tiny accretion disc round a black core; knots are bright
//! dots with trails; the radio lobes a faint stipple. Screen lengths are in dots.

use std::cell::RefCell;
use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01, value_noise};
use sciart_core::timeline::ease;
use sciart_core::{draw, title};

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
/// Jet on screen: start of the jet (fraction of width), length (fraction of width),
/// position angle (radians, up from the right).
const CORE_X: f64 = 0.3;
const JET_LEN: f64 = 0.54;
const JET_PA: f64 = 0.12;
/// The helix (JET-A7): cone half-width `(BASE + OPENING·s)·len`, wavenumber (per jet
/// length), the pattern's outward drift (jet lengths per s), strands, and where the
/// strands end (jet lengths).
const BASE: f64 = 0.012;
const OPENING: f64 = 0.045;
pub(crate) const HELIX_K: f64 = 30.0;
pub(crate) const HELIX_DRIFT: f64 = 0.06;
const STRANDS: usize = 3;
const JET_END: f64 = 0.93;
/// Helix sample spacing (dots).
const STEP: f64 = 0.8;
/// The counter-jet is drawn one segment in this many.
const COUNTER_SPARSE: usize = 3;
/// Knots (JET-A8): drawn above this brightness, white-hot above `KNOT_BRIGHT`, with a
/// trail of the last `KNOT_TRAIL` seconds.
const KNOT_MIN: f64 = 0.15;
const KNOT_BRIGHT: f64 = 0.5;
pub(crate) const KNOT_TRAIL: f64 = 1.5;
/// Globular clusters per cell.
const GC_PER_CELL: f64 = 1.0 / 150.0;
/// Readout ink (GEN-R13).
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// Inks (M16): the helix by depth, knots `Accent`, the disc `Sun`, lobes a `Cold` stipple.
pub(crate) const FRONT: Ink = Ink::new(Role::Wave, 200);
pub(crate) const BACK: Ink = Ink::new(Role::Cold, 110);
pub(crate) const COUNTER: Ink = Ink::new(Role::Cold, 60);
const KNOT_HOT: Ink = Ink::new(Role::Accent, 255);
const KNOT_INK: Ink = Ink::new(Role::Accent, 200);
pub(crate) const TRAIL: Ink = Ink::new(Role::Accent, 150);
const DISC: Ink = Ink::new(Role::Sun, 210);
const HOTSPOT_INK: Ink = Ink::new(Role::Star, 255);
pub(crate) const LOBE: Ink = Ink::new(Role::Cold, 130);
const CLUSTER_INK: Ink = Ink::new(Role::Dim, 90);
/// The lobes' stipple (JET-A9): lit probability `STIPPLE·min(1, e/E_FULL)`.
pub(crate) const STIPPLE: f64 = 0.45;
const E_FULL: f64 = 0.25;
/// The stipple cache's refresh period (s): each row is re-evaluated once per period, the
/// rows staggered across it (GEN-R8: the plume noise is the scene's main cost).
const STIPPLE_Q: f64 = 0.4;
/// Radio plumes (jet lengths): the main lobe's half-width and how far its cocoon flows
/// back toward the core; the counter-lobe's hotspot distance, size and relative glow; the
/// backflow speed of the turbulence (jet lengths per second).
const LOBE_WIDTH: f64 = 0.17;
const LOBE_BACK: f64 = 0.42;
const COUNTER_AT: f64 = 0.44;
const COUNTER_SIZE: f64 = 0.7;
const COUNTER_GLOW: f64 = 0.45;
const BACKFLOW: f64 = 0.012;
/// The faint radio bridge from the core to the counter-lobe (unbeamed cocoon material).
const BRIDGE: f64 = 0.1;
const BRIDGE_WIDTH: f64 = 0.008;
/// The terminal hotspot (jet lengths).
const HOTSPOT: f64 = 0.95;

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

/// Counter-jet brightness relative to the jet's, `(δ'/δ)³` (JET-A3). M16: the scene draws
/// the counter-jet's field lines, not its light, so only the test reads it.
#[cfg(test)]
pub(crate) fn counter_ratio(beta: f64, cos_t: f64) -> f64 {
    (doppler(beta, cos_t, -1.0) / doppler(beta, cos_t, 1.0)).powi(3)
}

/// Phase of helix strand `j` at jet coordinate `s` and time `t` (JET-A7): a helix seen
/// from the side, its pattern drifting outward at `HELIX_DRIFT`.
pub(crate) fn helix_phase(s: f64, t: f64, j: usize) -> f64 {
    HELIX_K * (s - HELIX_DRIFT * t) + TAU * j as f64 / STRANDS as f64
}

/// The lobes' emissivity on the even dot lattice over their bounding box: origin (dots),
/// size, values and the time each row shows.
#[derive(Debug, Default)]
struct StippleCache {
    x0: i32,
    y0: i32,
    cols: usize,
    rows: usize,
    e: Vec<f32>,
    at: Vec<f64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    beta: f64,
    age: f64,
    size: f64,
}

impl Knot {
    /// Position along the jet (jet lengths) at age `age`: straight out from HST-1 at
    /// `β_app c`.
    fn s_at(&self, age: f64) -> f64 {
        S_HST + beta_app(self.beta, SIN_T, COS_T) * C_UNIT * age.max(0.0)
    }

    /// Position along the jet now.
    pub(crate) fn s(&self) -> f64 {
        self.s_at(self.age)
    }

    fn brightness(&self) -> f64 {
        let boost = (doppler(self.beta, COS_T, 1.0) / doppler(BETA_JET, COS_T, 1.0)).powi(3);
        let s = self.s();
        let fade_in = ease::smoothstep(self.age / 0.8);
        let fade_out = 1.0 - ease::smoothstep((s - 0.85) / 0.2);
        // Knots flicker as their shocks brighten and fade.
        let pulse = 0.75 + 0.25 * (2.3 * self.age + 7.0 * self.size).sin_det();
        boost * fade_in * fade_out * (1.0 - 0.45 * s) * pulse * self.size
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
    /// Screen: core (dots), jet axis unit vector (screen, y down), length (dots).
    core: (f64, f64),
    axis: (f64, f64),
    len: f64,
    /// Accretion disc semi-axes (dots).
    disc: (f64, f64),
    clusters: Vec<(i32, i32)>,
    salt: u64,
    /// The braille lattice; a cell so `prepare` can draw into it while reading the
    /// rest of `self` (no placeholder lattice allocated per frame, GEN-R11).
    dots: RefCell<Dots>,
    /// The lattice's size in dots, and the lobes' stipple cache.
    dot_wh: (i32, i32),
    cache: StippleCache,
    readout_room: usize,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl RelativisticJet {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.salt = ctx.rng.random();
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

    /// Dot position of jet coordinate `s` (jet lengths) at transverse offset `d` (dots,
    /// left of the direction of motion).
    fn at(&self, s: f64, d: f64) -> (f64, f64) {
        let (ax, ay) = self.axis;
        (
            self.core.0 + ax * s * self.len + ay * d,
            self.core.1 + ay * s * self.len - ax * d,
        )
    }

    /// The cone's half-width (dots) at jet coordinate `s`.
    fn half_width(&self, s: f64) -> f64 {
        (BASE + OPENING * s) * self.len
    }

    fn flare(&self) -> f64 {
        (-(self.t - self.last_flare).max(0.0) / FLARE_TAU).exp()
    }

    /// Every helix segment `(from, to, ink, counter)` in dots: three strands per jet, by
    /// depth (JET-A7); the counter-jet one segment in `COUNTER_SPARSE`, faint (JET-A3).
    fn strands(&self, mut f: impl FnMut((f64, f64), (f64, f64), Ink, bool)) {
        if self.len <= 0.0 {
            return;
        }
        // Start clear of the disc.
        let s0 = (self.disc.0 + 1.0) / self.len;
        for (sign, end) in [(1.0, JET_END), (-1.0, COUNTER_AT)] {
            if end <= s0 {
                continue;
            }
            let n = ((end - s0) * self.len / STEP).ceil().max(2.0) as usize;
            for j in 0..STRANDS {
                let point = |s: f64| {
                    let ph = helix_phase(s, self.t, j);
                    let d = self.half_width(s) * ph.sin_det();
                    (self.at(sign * s, sign * d), ph.cos_det())
                };
                let mut prev = point(s0);
                for k in 1..=n {
                    let s = s0 + (end - s0) * k as f64 / n as f64;
                    let next = point(s);
                    if sign > 0.0 {
                        let depth = prev.1 + next.1;
                        f(
                            prev.0,
                            next.0,
                            if depth > 0.0 { FRONT } else { BACK },
                            false,
                        );
                    } else if (k + j) % COUNTER_SPARSE == 0 {
                        f(prev.0, next.0, COUNTER, true);
                    }
                    prev = next;
                }
            }
        }
    }

    /// Every visible knot as `(tail, head, head ink)` in dots (JET-A8).
    fn knot_marks(&self, mut f: impl FnMut((f64, f64), (f64, f64), Ink)) {
        for k in self.knots.iter() {
            let b = k.brightness();
            if b < KNOT_MIN || k.s() > HOTSPOT {
                continue;
            }
            let head = self.at(k.s(), 0.0);
            let tail = self.at(k.s_at(k.age - KNOT_TRAIL), 0.0);
            f(
                tail,
                head,
                if b >= KNOT_BRIGHT { KNOT_HOT } else { KNOT_INK },
            );
        }
    }

    /// The lobes' stipple (JET-A9): on the even dot lattice inside the plumes' bounding
    /// box, each dot lit with probability `STIPPLE·min(1, e/E_FULL)` by a fixed hash, `e`
    /// from the row-refreshed cache ([`Self::refresh_stipple`]).
    fn stipple(&self, mut f: impl FnMut(i32, i32)) {
        let c = &self.cache;
        for (j, row) in c.e.chunks_exact(c.cols.max(1)).enumerate() {
            let y = c.y0 + 2 * j as i32;
            for (k, &e) in row.iter().enumerate() {
                if e <= 0.0 {
                    continue;
                }
                let x = c.x0 + 2 * k as i32;
                let p = STIPPLE * (f64::from(e) / E_FULL).min(1.0);
                if hash01(self.salt ^ cell_key(x, y)) < p {
                    f(x, y);
                }
            }
        }
    }

    /// Lay out the stipple cache over the plumes' bounding box (even dots) and fill it.
    fn build_stipple(&mut self) {
        let (w, h) = self.dot_wh;
        let reach = 1.3 * LOBE_WIDTH * self.len + 2.0;
        let a = self.at(HOTSPOT + 0.1, 0.0);
        let b = self.at(-COUNTER_AT - 0.1, 0.0);
        let x0 = ((a.0.min(b.0) - reach).floor() as i32).max(0) & !1;
        let x1 = ((a.0.max(b.0) + reach).ceil() as i32).min(w - 1);
        let y0 = ((a.1.min(b.1) - reach).floor() as i32).max(0) & !1;
        let y1 = ((a.1.max(b.1) + reach).ceil() as i32).min(h - 1);
        let c = &mut self.cache;
        (c.x0, c.y0) = (x0, y0);
        c.cols = usize::try_from((x1 - x0) / 2 + 1).unwrap_or(0);
        c.rows = usize::try_from((y1 - y0) / 2 + 1).unwrap_or(0);
        c.e.clear();
        c.e.resize(c.cols * c.rows, 0.0);
        c.at.clear();
        c.at.resize(c.rows, f64::NAN);
        self.refresh_stipple();
    }

    /// The time cache row `j` of `rows` shows at time `t`: the latest multiple of
    /// `STIPPLE_Q` shifted by `j/rows` of it, so rows refresh in turn and a frame depends
    /// only on its time, not on how often it is drawn (GEN-R20).
    pub(crate) fn row_time(t: f64, j: usize, rows: usize) -> f64 {
        let ph = j as f64 / rows.max(1) as f64;
        ((t / STIPPLE_Q + ph).floor() - ph) * STIPPLE_Q
    }

    /// Re-evaluate the cache rows whose time has moved on (about `Δt/STIPPLE_Q` of them
    /// per frame): the plumes' texture drifts slowly, so this amortises the noise.
    fn refresh_stipple(&mut self) {
        let mut e = std::mem::take(&mut self.cache.e);
        let mut at = std::mem::take(&mut self.cache.at);
        let (x0, y0, cols, rows) = (
            self.cache.x0,
            self.cache.y0,
            self.cache.cols,
            self.cache.rows,
        );
        let (ax, ay) = self.axis;
        for (j, row) in e.chunks_exact_mut(cols.max(1)).enumerate() {
            let tq = Self::row_time(self.t, j, rows);
            if at.get(j).is_some_and(|&a| a == tq) {
                continue;
            }
            let y = f64::from(y0 + 2 * j as i32);
            for (k, slot) in row.iter_mut().enumerate() {
                let x = f64::from(x0 + 2 * k as i32);
                let (dx, dy) = (x - self.core.0, y - self.core.1);
                let along = (dx * ax + dy * ay) / self.len;
                let d = (dx * ay - dy * ax) / self.len;
                // Cheap cull: well off the jet line, or past either hotspot.
                *slot = if d.abs() > 1.3 * LOBE_WIDTH
                    || !(-COUNTER_AT - 0.1..=HOTSPOT + 0.1).contains(&along)
                {
                    0.0
                } else {
                    self.lobe_emission(along, d, tq) as f32
                };
            }
            if let Some(a) = at.get_mut(j) {
                *a = tq;
            }
        }
        self.cache.e = e;
        self.cache.at = at;
    }

    /// The lobes' emissivity at jet coordinate `along` and transverse offset `d` (jet
    /// lengths) at time `t`: not beamed, so the counter-lobe shows (fainter) where the counter-jet
    /// cannot, joined to the core by a faint radio bridge.
    pub(crate) fn lobe_emission(&self, along: f64, d: f64, t: f64) -> f64 {
        if along >= 0.0 {
            plume(along - HOTSPOT, d, t, self.salt, 1.0).0
        } else {
            let (v, _) = plume(-along - COUNTER_AT, -d, t, self.salt ^ 1, COUNTER_SIZE);
            let s = -along;
            let bridge = BRIDGE
                * ease::smoothstep(s / 0.08)
                * (1.0 - ease::smoothstep((s - COUNTER_AT + 0.05) / 0.1))
                * (-(d / BRIDGE_WIDTH).powi(2)).exp();
            COUNTER_GLOW * v + bridge
        }
    }

    fn raster(&mut self) {
        let mut dots = self.dots.borrow_mut();
        dots.clear();
        for &(x, y) in &self.clusters {
            dots.set(x, y, CLUSTER_INK);
        }
        self.stipple(|x, y| dots.set(x, y, LOBE));
        self.strands(|a, b, ink, _| dots.line(a.0, a.1, b.0, b.1, ink));
        self.knot_marks(|tail, head, ink| {
            dots.line(tail.0, tail.1, head.0, head.1, TRAIL);
            dots.disc(head.0, head.1, 1.0, ink);
        });
        // HST-1, flaring.
        let (hx, hy) = self.at(S_HST, 0.0);
        dots.disc(hx, hy, 0.5 + 1.5 * self.flare(), KNOT_HOT);
        // The terminal hotspot, where the jet slams into the intergalactic gas.
        let (tx, ty) = self.at(HOTSPOT, 0.0);
        dots.disc(tx, ty, 1.0, HOTSPOT_INK);
        self.accretion_disc(&mut dots);
        drop(dots);
    }

    /// The accretion disc: a flat ellipse of dots across the jet axis round an unlit core.
    fn accretion_disc(&self, dots: &mut Dots) {
        let (a, b) = self.disc;
        let (ax, ay) = self.axis;
        let r = a.ceil() as i32 + 1;
        let (cx, cy) = (self.core.0.round() as i32, self.core.1.round() as i32);
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (f64::from(dx), f64::from(dy));
                // Along the jet (the disc's thin direction) and across it.
                let (u, v) = (x * ax + y * ay, x * ay - y * ax);
                let q = (v / a).powi(2) + (u / b).powi(2);
                let core = (v / (0.4 * a)).powi(2) + (u / (0.6 * b)).powi(2);
                if q <= 1.0 && core > 1.0 {
                    dots.set(cx + dx, cy + dy, DISC);
                }
            }
        }
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
        self.dots.get_mut().resize(size);
        let ds = self.dots.get_mut().dot_size();
        self.dot_wh = (i32::from(ds.width()), i32::from(ds.height()));
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        self.core = (CORE_X * w, 0.55 * h);
        let (s, c) = JET_PA.sin_cos_det();
        self.axis = (c, -s);
        self.len = JET_LEN * w;
        let a = (0.03 * h).max(3.0);
        self.disc = (a, 0.35 * a);
        self.readout_room = title::stable_readout_room(size, &META);
        // Globular clusters: a swarm thinning outward from the core.
        self.clusters.clear();
        let re = 0.28 * h / 2.0;
        let n = (size.area() as f64 * GC_PER_CELL) as i32;
        for k in 0..n {
            let r = 0.4 + 2.2 * hash01(cell_key(k, 1)).powf(1.6);
            let a = 2.0 * PI * hash01(cell_key(k, 2));
            let (sa, ca) = a.sin_cos_det();
            let x = self.core.0 + ca * r * re * 1.4;
            let y = self.core.1 + sa * r * re;
            self.clusters.push((x.round() as i32, y.round() as i32));
        }
        self.build_stipple();
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
            self.refresh_stipple();
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.borrow().blit(canvas);
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

    /// Jet coordinates `(s, d)` (jet lengths, dots) of a dot position.
    fn jet_coords(g: &RelativisticJet, p: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (p.0 - g.core.0, p.1 - g.core.1);
        let (ax, ay) = g.axis;
        ((dx * ax + dy * ay) / g.len, dx * ay - dy * ax)
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
        let dots_per_s = (p1.0 - p0.0).hypot(p1.1 - p0.1) / (120.0 * SIM_DT);
        let in_c = dots_per_s / (C_UNIT * g.len);
        let want = beta_app(0.97, SIN_T, COS_T);
        assert!((in_c / want - 1.0).abs() < 0.02, "{in_c} vs {want}");
    }

    #[test]
    fn jet_a3_counter_jet_is_beamed_away() {
        assert!(counter_ratio(BETA_JET, COS_T) < 1e-3);
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        for _ in 0..4 {
            for _ in 0..300 {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            canvas.clear();
            g.render(&mut canvas);
            // No knot ink behind the core.
            let cx = (g.core.0 / f64::from(Dots::CELL_W)) as i32 - 2;
            for y in 0..49 {
                for x in 0..cx {
                    let c = canvas.get(x, y).unwrap();
                    assert!(
                        c.ch == ' ' || c.ink.role != Role::Accent,
                        "knot ink at {x},{y}"
                    );
                }
            }
            let (mut jet, mut counter) = (0usize, 0usize);
            g.strands(|a, b, ink, back| {
                let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).round() as usize + 1;
                if back {
                    assert_eq!(ink, COUNTER);
                    counter += n;
                } else {
                    jet += n;
                }
            });
            assert!(3 * counter < jet, "counter {counter} vs jet {jet}");
        }
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
    fn jet_a7_three_helical_threads_inside_the_cone_moving_out() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        for _ in 0..200 {
            g.update(SIM_DT, &mut ctx);
        }
        let (mut front, mut back) = (0, 0);
        g.strands(|a, b, ink, counter| {
            if counter {
                return;
            }
            for p in [a, b] {
                let (s, d) = jet_coords(&g, p);
                assert!(
                    d.abs() <= g.half_width(s) + 1.0,
                    "s {s}: |d| {d} > {}",
                    g.half_width(s)
                );
            }
            match ink {
                FRONT => front += 1,
                BACK => back += 1,
                other => panic!("{other:?}"),
            }
        });
        assert!(front > 50 && back > 50, "{front} {back}");
        // The ink follows the depth: the front half of the turn is `Wave`.
        for j in 0..STRANDS {
            for i in 0..100 {
                let s = 0.05 + 0.009 * f64::from(i);
                for (t, dt) in [(0.0, 0.5), (3.0, 2.0)] {
                    let (a, b) = (
                        helix_phase(s, t, j),
                        helix_phase(s + HELIX_DRIFT * dt, t + dt, j),
                    );
                    assert!((a - b).abs() < 1e-9, "s {s} t {t}");
                }
            }
        }
    }

    #[test]
    fn jet_a8_knot_trails_lie_behind_their_heads() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let mut seen = 0;
        for _ in 0..6 {
            for _ in 0..200 {
                g.update(SIM_DT, &mut ctx);
            }
            g.knot_marks(|tail, head, _| {
                seen += 1;
                let (st, dt) = jet_coords(&g, tail);
                let (sh, dh) = jet_coords(&g, head);
                assert!(dt.abs() < 1e-6 && dh.abs() < 1e-6, "off the axis");
                assert!(st < sh, "tail ahead of the head");
            });
        }
        assert!(seen > 5, "{seen} knots");
        let k = Knot {
            beta: 0.97,
            age: 5.0,
            size: 1.0,
        };
        let len = k.s() - k.s_at(k.age - KNOT_TRAIL);
        let want = beta_app(0.97, SIN_T, COS_T) * C_UNIT * KNOT_TRAIL;
        assert!((len - want).abs() < 1e-12);
    }

    #[test]
    fn jet_a9_lobes_are_a_stipple() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        for _ in 0..300 {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let (mut lit, mut inside) = (0usize, 0usize);
        g.stipple(|x, y| {
            assert!(x % 2 == 0 && y % 2 == 0, "off the even lattice at {x},{y}");
            lit += 1;
        });
        g.prepare();
        // Inside the plume: the cached emissivity is positive.
        inside += g.cache.e.iter().filter(|&&e| e > 0.0).count();
        // Each row shows the plume at its own staggered refresh time, at most one period
        // old (GEN-R20: a function of the time, not of the draw cadence).
        for j in 0..g.cache.rows {
            let tq = RelativisticJet::row_time(g.t, j, g.cache.rows);
            assert!(tq <= g.t && g.t - tq < STIPPLE_Q + 1e-9);
            assert_eq!(g.cache.at[j], tq);
        }
        assert!(lit > 30, "{lit} stipple dots");
        assert!(lit as f64 <= STIPPLE * inside as f64, "{lit} of {inside}");
    }
}
