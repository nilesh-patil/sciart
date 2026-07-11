//! A young supernova remnant like Cassiopeia A or the Crab, as a shell of filaments
//! (SPEC-002 `supernova_remnant`).
//!
//! Three analytic surfaces, all scaled by the forward-shock radius `R_f(t)` (a
//! Truelove–McKee-style blend from ejecta-dominated to Sedov–Taylor expansion): the blast
//! wave, the contact discontinuity rippled by Rayleigh–Taylor fingers, and the reverse
//! shock inside it. Ejecta knots coast outward homologously and light up as the reverse
//! shock reaches them.
//!
//! M16 (GEN-R25, thread style): the shocked ejecta are radial braille filaments from the
//! reverse shock out to the R–T fingertips, coloured by temperature along the thread
//! (freshly shocked `Sun` inside, cooler `Hot` outward); freshly lit knots are sparks with
//! short radial trails; the blast wave a faint dotted `Cold` circle. Lengths on screen are
//! in dots (square), the model in units of `R_f`.

use std::cell::RefCell;
use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "supernova_remnant",
    category: Category::DeepCosmos,
    description: "A supernova blast wave sweeping up gas while ejecta fingers light up at the reverse shock",
    title: "Supernova remnant · blast wave",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SupernovaRemnant::default())
}

/// Loop: expansion, then the remnant fades before the next flash (s).
const EXPAND: f64 = 36.0;
const FADE: f64 = 4.0;
pub(crate) const LOOP: f64 = EXPAND + FADE;
/// Model time at the loop's start and the Sedov–Taylor transition time (model s).
const T0: f64 = 0.3;
const T_ST: f64 = 8.0;
/// Blast-wave radius at the end of the expansion, in half-heights.
const R_END: f64 = 0.9;
/// Log slopes of the two regimes, and the blend's sharpness.
const SLOPE_ED: f64 = 4.0 / 7.0;
const SLOPE_ST: f64 = 2.0 / 5.0;
const BLEND: f64 = 2.0;
/// Contact discontinuity (units of `R_f`) and the finger amplitudes about it.
const CONTACT: f64 = 0.8;
const FINGER_OUT: f64 = 0.18;
const FINGER_IN: f64 = 0.054;
/// Reverse shock (units of `R_f`), easing inward over the expansion.
const REVERSE: (f64, f64) = (0.62, 0.45);
/// Rayleigh–Taylor modes: count, wavenumber range, seed amplitude, growth rate and cap.
const RT_MODES: usize = 24;
const RT_K: (u32, u32) = (5, 22);
const RT_SEED: f64 = 0.004;
const RT_RATE: f64 = 0.35;
const RT_CAP: f64 = 0.12;
/// Angle table resolution.
const ANGLES: usize = 512;
/// Out-of-round shape (share of `R_f`) the whole remnant shares at low angular order
/// (SNR-A6).
const RIM_WOBBLE: f64 = 0.09;
/// Wobble modes `m` = 2, 3, 4: relative amplitudes and the slow drift of their phases
/// (rad/s), so the lopsided shell turns and reshapes as it ploughs into lumpy gas.
const WOBBLE_AMP: [f64; 3] = [1.0, 0.6, 0.35];
const WOBBLE_DRIFT: f64 = 0.05;
/// Filaments (M16, SNR-A8): the emitting shell starts at max(`R_r`, `FIL_INNER`·`R_c0`),
/// `R_c0 = CONTACT·R_f` the unperturbed contact, so fingers are long and the gaps between
/// them short stubs; one filament per `FIL_SPACING` dots of that inner edge's
/// circumference, at most `FIL_MAX`, along the golden-angle sequence.
const FIL_INNER: f64 = 0.72;
const FIL_SPACING: f64 = 3.5;
pub(crate) const FIL_MAX: usize = 400;
/// Golden-angle fraction: any prefix of `frac(i·φ)` is evenly spread round the circle.
const GOLDEN: f64 = 0.618_033_988_749_895;
/// Sideways wiggle of a filament (dots), at most, growing from 0 at its root.
const WIGGLE: f64 = 2.5;
/// The inner share of a filament that is freshly shocked, hot ejecta (SNR-A9).
const HOT_SHARE: f64 = 0.35;
/// Segment length along a filament (dots).
const SEG: f64 = 2.0;
/// Ejecta knots (SNR-A5) and their afterglow.
pub(crate) const KNOTS: usize = 600;
const KNOT_TAU: f64 = 1.5;
const KNOT_FLOOR: f64 = 0.25;
/// Sparks (SNR-A10): drawn while a knot's brightness is at least `SPARK_MIN`, white while
/// at least `SPARK_WHITE`, with a radial trail of the last `SPARK_TRAIL` seconds.
const SPARK_MIN: f64 = 0.45;
const SPARK_WHITE: f64 = 0.75;
pub(crate) const SPARK_TRAIL: f64 = 1.2;

/// Inks (M16): temperature along the filaments, a faint blue blast wave, white sparks.
pub(crate) const SHOCK: Ink = Ink::new(Role::Cold, 110);
pub(crate) const HOT_EJECTA: Ink = Ink::new(Role::Sun, 220);
pub(crate) const FINGER: Ink = Ink::new(Role::Hot, 190);
pub(crate) const STUB: Ink = Ink::new(Role::Hot, 150);
const SPARK: Ink = Ink::new(Role::Star, 255);
/// The central compact object, and the explosion's white heart.
const COMPACT: Ink = Ink::new(Role::Star, 200);
pub(crate) const FLASH: Ink = Ink::new(Role::Star, 255);
const FLASH_RING: Ink = Ink::new(Role::Sun, 220);

/// Forward-shock radius in model units at model time `t` (SNR-A1): log slope `4/7`
/// early, `2/5` late.
pub(crate) fn forward_shock(t: f64) -> f64 {
    let x = (t / T_ST).powf(BLEND);
    t.powf(SLOPE_ED) / (1.0 + x).powf((SLOPE_ED - SLOPE_ST) / BLEND)
}

/// Reverse shock as a fraction of `R_f` at loop time `tau`.
pub(crate) fn reverse_frac(tau: f64) -> f64 {
    ease::lerp(REVERSE.0, REVERSE.1, ease::smoothstep(tau / EXPAND))
}

/// Contact radius as a fraction of `R_f` for a finger perturbation `p`: pushed out up to
/// `FINGER_OUT` where fingers reach, pulled in at most `FINGER_IN` between them, so
/// `R_r < R_c < R_f` always (SNR-A2).
pub(crate) fn contact_frac(p: f64) -> f64 {
    let out = (p.max(0.0) / 0.25).tanh();
    let inn = ((-p).max(0.0) / 0.25).tanh();
    CONTACT * (1.0 + FINGER_OUT * out * out - FINGER_IN * inn)
}

/// The blast wave's early growth from the flash (a share of `R_f`, 0 → 1 over 1.2 s).
fn grow(tau: f64) -> f64 {
    ease::out_quad(tau / 1.2)
}

/// A knot of ejecta: homologous speed (its radius in `R_f` at the end of the expansion),
/// direction, and when the reverse shock lit it (loop s).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    u: f64,
    /// `(sin, cos)` of its direction.
    dir: (f64, f64),
    lit: Option<f64>,
    /// Angle bin of `dir` (the remnant's shape factor there).
    bin: u16,
}

impl Knot {
    /// Radius in units of `R_f` at loop time `tau`: free coasting, `r ∝ t`, against a
    /// blast wave that slows.
    fn frac(&self, tau: f64, r_end: f64) -> f64 {
        let t = T0 + tau;
        let t_end = T0 + EXPAND;
        self.u * (t / t_end) * r_end / forward_shock(t)
    }

    /// Radius in units of the end radius `R_END·half` at loop time `tau` (≤ `EXPAND`),
    /// before the shape factor: `frac · R_f`, which is `u·t/t_end` (homologous), times
    /// the early growth the blast wave shares.
    fn reach(&self, tau: f64) -> f64 {
        let tau = tau.clamp(0.0, EXPAND);
        self.u * (T0 + tau) / (T0 + EXPAND) * grow(tau)
    }

    /// Brightness at loop time `tau`: 0 until shocked, then a flare fading to a floor
    /// (non-increasing, SNR-A3).
    fn brightness(&self, tau: f64) -> f64 {
        match self.lit {
            None => 0.0,
            Some(at) => KNOT_FLOOR + (1.0 - KNOT_FLOOR) * (-(tau - at).max(0.0) / KNOT_TAU).exp(),
        }
    }
}

/// One filament of the shell: its direction and the shape of its sideways wiggle.
#[derive(Debug, Clone, Copy, Default)]
struct Filament {
    /// `(sin, cos)` of its direction and its angle bin.
    dir: (f64, f64),
    bin: u16,
    /// Wiggle: phases of two sine terms and the share of `WIGGLE` it reaches.
    phase: (f64, f64),
    amp: f64,
    /// Its fingertip short of the contact by this share (0–0.2), so tips are ragged.
    short: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Mode {
    k: u32,
    phase: f64,
    weight: f64,
}

/// The shell's radii (dots) at an angle bin.
#[derive(Debug, Clone, Copy)]
struct Radii {
    /// Blast wave, contact, emitting shell's inner edge.
    rf: f64,
    rc: f64,
    inner: f64,
}

#[derive(Debug, Default)]
pub struct SupernovaRemnant {
    seeded: bool,
    /// Loop time.
    tau: f64,
    modes: Vec<Mode>,
    /// `cos(kθ)` and `sin(kθ)` per mode per angle bin.
    cos_k: Vec<f32>,
    sin_k: Vec<f32>,
    /// Finger perturbation `p(θ)` this frame, per angle bin.
    finger: Vec<f32>,
    /// The remnant's low-order shape per angle bin this frame, in `[−1, 1]`, and its
    /// modes (phase, amplitude, drift).
    wobble: Vec<f32>,
    wob_modes: [(f64, f64, f64); 3],
    filaments: Vec<Filament>,
    knots: ParticlePool<Knot>,
    /// Screen centre and half-height, in dots.
    centre: (f64, f64),
    half: f64,
    stars: Vec<(i32, i32, Ink)>,
    /// The braille lattice; a cell so `prepare` can draw into it while reading the
    /// rest of `self` (no placeholder lattice allocated per frame, GEN-R11).
    dots: RefCell<Dots>,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl SupernovaRemnant {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.modes = (0..RT_MODES)
            .map(|_| Mode {
                k: ctx.rng.random_range(RT_K.0..=RT_K.1),
                phase: ctx.rng.random_range(0.0..TAU),
                weight: ctx.rng.random_range(0.5..1.0),
            })
            .collect();
        self.cos_k.clear();
        self.sin_k.clear();
        for m in &self.modes {
            for j in 0..ANGLES {
                let th = f64::from(m.k) * TAU * j as f64 / ANGLES as f64;
                let (s, c) = th.sin_cos_det();
                self.cos_k.push(c as f32);
                self.sin_k.push(s as f32);
            }
        }
        self.finger = vec![0.0; ANGLES];
        // Low-order modes of the remnant's shape (m = 2 dominant: a lopsided shell).
        self.wob_modes = std::array::from_fn(|k| {
            (
                ctx.rng.random_range(0.0..TAU),
                WOBBLE_AMP[k] * ctx.rng.random_range(0.8..1.0),
                WOBBLE_DRIFT * ctx.rng.random_range(-1.0..1.0),
            )
        });
        self.wobble = vec![0.0; ANGLES];
        self.update_wobble();
        self.filaments = vec![Filament::default(); FIL_MAX];
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.tau = ctx.rng.random_range(1.0..1.3);
        self.explode(ctx);
        self.light_knots();
    }

    /// A new supernova: fresh knots (all dark, inside the reverse shock), new finger
    /// phases and a new set of filaments.
    fn explode(&mut self, ctx: &mut Ctx) {
        self.knots.clear();
        for _ in 0..KNOTS {
            let u = ctx.rng.random_range(0.5..0.97);
            let a = ctx.rng.random_range(0.0..TAU);
            self.knots.spawn(Knot {
                u,
                dir: a.sin_cos_det(),
                lit: None,
                bin: bin_of(a),
            });
        }
        for m in &mut self.modes {
            m.phase = ctx.rng.random_range(0.0..TAU);
        }
        let offset = ctx.rng.random_range(0.0..1.0);
        for (i, f) in self.filaments.iter_mut().enumerate() {
            // Golden-angle order, so every prefix is evenly spread (SNR-A8); a little
            // jitter keeps the spacing from looking machined.
            let a = TAU
                * ((i as f64 * GOLDEN + offset).fract() + ctx.rng.random_range(-0.2..0.2) / 400.0);
            *f = Filament {
                dir: a.sin_cos_det(),
                bin: bin_of(a),
                phase: (
                    ctx.rng.random_range(0.0..TAU),
                    ctx.rng.random_range(0.0..TAU),
                ),
                amp: ctx.rng.random_range(0.3..1.0),
                short: ctx.rng.random_range(0.0..0.2),
            };
        }
    }

    fn r_end(&self) -> f64 {
        forward_shock(T0 + EXPAND)
    }

    /// Forward-shock radius in dots now (before the shape factor).
    fn rf_dots(&self) -> f64 {
        let tau = self.tau.min(EXPAND);
        R_END * self.half * forward_shock(T0 + tau) / self.r_end() * grow(self.tau)
    }

    /// Mark knots that the reverse shock has reached by `tau`.
    fn light_knots(&mut self) {
        let (tau, r_end) = (self.tau.min(EXPAND), self.r_end());
        let rr = reverse_frac(tau);
        for k in self.knots.iter_mut() {
            if k.lit.is_none() && k.frac(tau, r_end) > rr {
                k.lit = Some(self.tau);
            }
        }
    }

    fn update_fingers(&mut self) {
        let tau = self.tau.min(EXPAND);
        self.finger.fill(0.0);
        for (m, mode) in self.modes.iter().enumerate() {
            let a = (RT_SEED * (RT_RATE * (f64::from(mode.k) / 6.0).sqrt() * tau).exp())
                .min(RT_CAP)
                * mode.weight;
            let (cp, sp) = (mode.phase.cos_det(), mode.phase.sin_det());
            let (cs, ss) = (
                &self.cos_k[m * ANGLES..(m + 1) * ANGLES],
                &self.sin_k[m * ANGLES..(m + 1) * ANGLES],
            );
            for ((f, &c), &s) in self.finger.iter_mut().zip(cs).zip(ss) {
                // cos(kθ + φ)
                *f += (a * (f64::from(c) * cp - f64::from(s) * sp)) as f32;
            }
        }
    }

    /// The remnant's shape now: modes `m` = 2–4 with slowly drifting phases, normalised
    /// by the total amplitude so it stays in `[−1, 1]`.
    fn update_wobble(&mut self) {
        let tau = self.tau.min(EXPAND);
        let norm: f64 = self.wob_modes.iter().map(|m| m.1).sum::<f64>().max(1e-9);
        let modes = self.wob_modes;
        for (j, w) in self.wobble.iter_mut().enumerate() {
            let th = TAU * j as f64 / ANGLES as f64;
            let v: f64 = modes
                .iter()
                .enumerate()
                .map(|(k, &(ph, a, drift))| {
                    a * ((k as f64 + 2.0) * th + ph + drift * tau).sin_det()
                })
                .sum();
            *w = (v / norm) as f32;
        }
    }

    /// The shape factor at an angle bin: every radius (blast wave, contact, reverse shock,
    /// knots) scales with it, so the ordering SNR-A2 holds at every angle.
    fn shape(wobble: &[f32], bin: usize) -> f64 {
        1.0 + RIM_WOBBLE * f64::from(wobble.get(bin).copied().unwrap_or(0.0))
    }

    fn envelope(&self) -> f64 {
        1.0 - ease::smoothstep((self.tau - EXPAND) / FADE)
    }

    /// The shell's radii (dots) at angle bin `bin`, for the blast wave `rf` (dots).
    fn radii(&self, bin: usize, rf: f64) -> Radii {
        let sh = Self::shape(&self.wobble, bin);
        let rfa = rf * sh;
        let p = f64::from(self.finger.get(bin).copied().unwrap_or(0.0));
        let rr = reverse_frac(self.tau.min(EXPAND)) * rfa;
        Radii {
            rf: rfa,
            rc: contact_frac(p) * rfa,
            inner: rr.max(FIL_INNER * CONTACT * rfa),
        }
    }

    /// Filaments drawn now: one per `FIL_SPACING` dots of the shell's inner edge, scaled
    /// by the fade envelope (SNR-A8).
    fn filament_count(&self) -> usize {
        let rf = self.rf_dots();
        let inner = (reverse_frac(self.tau.min(EXPAND))).max(FIL_INNER * CONTACT) * rf;
        let n = TAU * inner / FIL_SPACING * self.envelope();
        (n.max(0.0) as usize).min(FIL_MAX).min(self.filaments.len())
    }

    /// Every filament segment `(from, to, ink)` in dots, in drawing order: each filament a
    /// wiggling radial thread from the shell's inner edge to its fingertip, `Sun` on its
    /// inner, freshly shocked share and `Hot` beyond (brighter in the fingers).
    fn filament_segments(&self, mut f: impl FnMut((f64, f64), (f64, f64), Ink)) {
        let rf = self.rf_dots();
        if rf <= 0.0 {
            return;
        }
        let c = self.centre;
        for fil in &self.filaments[..self.filament_count()] {
            let bin = usize::from(fil.bin);
            let r = self.radii(bin, rf);
            let tip = r.rc * (1.0 - fil.short);
            let len = tip - r.inner;
            if len <= 0.0 {
                continue;
            }
            let in_finger = self.finger.get(bin).is_some_and(|&p| p > 0.0);
            let outer_ink = if in_finger { FINGER } else { STUB };
            let (s, co) = fil.dir;
            // Radial unit (screen y down) and its perpendicular.
            let (ux, uy) = (co, -s);
            let (vx, vy) = (s, co);
            let wig = (WIGGLE * fil.amp).min(0.15 * len);
            let at = |q: f64| {
                let rad = r.inner + q * len;
                let side = wig
                    * q
                    * (0.65 * (TAU * 1.1 * q + fil.phase.0).sin_det()
                        + 0.35 * (TAU * 2.3 * q + fil.phase.1).sin_det());
                (c.0 + rad * ux + side * vx, c.1 + rad * uy + side * vy)
            };
            let n = (len / SEG).ceil().max(1.0) as usize;
            let mut prev = at(0.0);
            for k in 1..=n {
                let q = k as f64 / n as f64;
                let next = at(q);
                let mid = (k as f64 - 0.5) / n as f64;
                f(
                    prev,
                    next,
                    if mid < HOT_SHARE {
                        HOT_EJECTA
                    } else {
                        outer_ink
                    },
                );
                prev = next;
            }
        }
    }

    /// Every spark (a freshly lit knot) as `(tail, head, head ink)` in dots (SNR-A10).
    fn sparks(&self, mut f: impl FnMut((f64, f64), (f64, f64), Ink)) {
        let scale = R_END * self.half;
        let tau = self.tau.min(EXPAND);
        let env = self.envelope();
        for k in self.knots.iter() {
            let b = k.brightness(self.tau) * env;
            if b < SPARK_MIN {
                continue;
            }
            let sh = Self::shape(&self.wobble, usize::from(k.bin)) * scale;
            let (head, tail) = (k.reach(tau) * sh, k.reach(tau - SPARK_TRAIL) * sh);
            let (s, c) = k.dir;
            let p = |r: f64| (self.centre.0 + r * c, self.centre.1 - r * s);
            f(
                p(tail),
                p(head),
                if b >= SPARK_WHITE { SPARK } else { HOT_EJECTA },
            );
        }
    }

    fn raster(&mut self) {
        self.update_fingers();
        self.update_wobble();
        let mut dots = self.dots.borrow_mut();
        dots.clear();
        // Background stars first: everything else outshines them.
        for &(x, y, ink) in &self.stars {
            dots.set(x, y, ink);
        }
        self.shock(&mut dots);
        self.filament_segments(|a, b, ink| dots.line(a.0, a.1, b.0, b.1, ink));
        self.sparks(|tail, head, ink| {
            dots.line(tail.0, tail.1, head.0, head.1, HOT_EJECTA);
            dots.plot(head.0, head.1, ink);
        });
        self.centre_star(&mut dots);
        drop(dots);
    }

    /// The blast wave: a dotted circle one dot thick at `R_f(θ)` (SNR-A4), thinning out
    /// with the fade envelope.
    fn shock(&self, dots: &mut Dots) {
        let rf = self.rf_dots();
        if rf < 1.0 {
            return;
        }
        let env = self.envelope();
        let n = (TAU * rf * (1.0 + RIM_WOBBLE)).ceil() as usize;
        for k in (0..n).step_by(2) {
            if hash01(cell_key(k as i32, 17)) >= env {
                continue;
            }
            let a = TAU * k as f64 / n as f64;
            let r = self.radii(usize::from(bin_of(a)), rf).rf;
            let (s, c) = a.sin_cos_det();
            dots.plot(self.centre.0 + r * c, self.centre.1 - r * s, SHOCK);
        }
    }

    /// The compact object, and in the first 1.5 s the flash: a `Sun` ring and `Star` arms
    /// that shrink as it fades round a white heart (SNR-A5).
    fn centre_star(&self, dots: &mut Dots) {
        let (cx, cy) = (self.centre.0.round() as i32, self.centre.1.round() as i32);
        let flash = 1.0 - ease::smoothstep(self.tau / 1.5);
        if flash > 0.0 {
            let ring = 1.5 + 0.06 * self.half * flash;
            let r = ring.ceil() as i32 + 1;
            for dy in -r..=r {
                for dx in -r..=r {
                    let d = f64::from(dx).hypot(f64::from(dy));
                    // Off the axes, so the ring's cells are not the arms'.
                    if dx != 0 && dy != 0 && (d - ring).abs() <= 0.5 {
                        dots.set(cx + dx, cy + dy, FLASH_RING);
                    }
                }
            }
            let arm = (ring + 3.0 * flash).round() as i32;
            for a in 1..=arm {
                for (dx, dy) in [(a, 0), (-a, 0), (0, a), (0, -a)] {
                    dots.set(cx + dx, cy + dy, FLASH);
                }
            }
        }
        dots.set(cx, cy, if flash > 0.3 { FLASH } else { COMPACT });
    }
}

/// Angle bin of the angle `a` (radians, counter-clockwise from +x).
fn bin_of(a: f64) -> u16 {
    ((a / TAU).rem_euclid(1.0) * ANGLES as f64) as u16 % ANGLES as u16
}

impl Generator for SupernovaRemnant {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.dots.get_mut().resize(size);
        let ds = self.dots.get_mut().dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        self.centre = (w / 2.0, h / 2.0);
        self.half = (h / 2.0).min(w / 2.0 / R_END * 0.95);
        self.stars.clear();
        let n = size.area() / 200;
        for k in 0..n as i32 {
            let x = (hash01(cell_key(k, 3)) * w) as i32;
            let y = (hash01(cell_key(k, 7)) * h) as i32;
            let l = if hash01(cell_key(k, 11)) > 0.8 {
                120
            } else {
                60
            };
            self.stars.push((x, y, Ink::new(Role::Star, l)));
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        self.tau += dt.max(0.0);
        if self.tau >= LOOP {
            self.tau = (self.tau - LOOP).rem_euclid(LOOP);
            self.explode(ctx);
        }
        self.light_knots();
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.borrow().blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (SupernovaRemnant, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = SupernovaRemnant::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    fn run(g: &mut SupernovaRemnant, ctx: &mut Ctx, secs: f64) {
        for _ in 0..(secs / SIM_DT).round() as usize {
            g.update(SIM_DT, ctx);
        }
        g.prepare();
    }

    fn log_slope(t: f64) -> f64 {
        let h = 1e-4 * t;
        (forward_shock(t + h).ln() - forward_shock(t - h).ln()) / ((t + h).ln() - (t - h).ln())
    }

    /// The filament dots alone, on a fresh lattice, with each dot's ink.
    fn filament_dots(g: &SupernovaRemnant) -> (Dots, Vec<(i32, i32, Ink)>) {
        let ds = g.dots.borrow().dot_size();
        let mut dots = Dots::new(Size::new(ds.width() / 2, ds.height() / 4));
        let mut list = Vec::new();
        g.filament_segments(|a, b, ink| {
            dots.line(a.0, a.1, b.0, b.1, ink);
            for (x, y) in sciart_core::geom::line(
                a.0.round() as i32,
                a.1.round() as i32,
                b.0.round() as i32,
                b.1.round() as i32,
            ) {
                list.push((x, y, ink));
            }
        });
        (dots, list)
    }

    #[test]
    fn snr_a1_blast_wave_decelerates_from_free_expansion_to_sedov() {
        let mut last = 0.0;
        for i in 0..=400 {
            let t = T0 + EXPAND * f64::from(i) / 400.0;
            let r = forward_shock(t);
            assert!(r > last);
            last = r;
        }
        assert!(log_slope(T0) >= 0.55, "early {}", log_slope(T0));
        assert!(
            log_slope(T0 + EXPAND) <= 0.45,
            "late {}",
            log_slope(T0 + EXPAND)
        );
    }

    #[test]
    fn snr_a2_reverse_shock_inside_contact_inside_blast_wave() {
        for i in 0..=200 {
            let tau = EXPAND * f64::from(i) / 200.0;
            let rr = reverse_frac(tau);
            for j in -200..=200 {
                let p = f64::from(j) / 20.0;
                let rc = contact_frac(p);
                assert!(rr < rc && rc < 1.0, "τ={tau} p={p}: {rr} {rc}");
            }
        }
        // And for the live finger field.
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for _ in 0..600 {
            g.update(SIM_DT * 3.0, &mut ctx);
            g.prepare();
            let rr = reverse_frac(g.tau.min(EXPAND));
            assert!(g.finger.iter().all(|&p| {
                let rc = contact_frac(f64::from(p));
                rr < rc && rc < 1.0
            }));
        }
    }

    #[test]
    fn snr_a3_knots_light_only_after_the_reverse_shock_and_then_only_fade() {
        let (mut g, mut ctx) = built(Size::new(80, 24));
        g.tau = 0.0;
        g.explode(&mut ctx);
        let r_end = g.r_end();
        let mut last: Vec<f64> = vec![0.0; g.knots.len()];
        for _ in 0..(EXPAND / SIM_DT) as usize - 2 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let tau = g.tau.min(EXPAND);
            let rr = reverse_frac(tau);
            for (k, prev) in g.knots.iter().zip(&mut last) {
                let b = k.brightness(g.tau);
                if k.frac(tau, r_end) <= rr {
                    assert_eq!(b, 0.0, "a knot inside the reverse shock is dark");
                }
                if *prev > 0.0 {
                    assert!(b <= *prev + 1e-12, "brightness rose after lighting");
                }
                *prev = b;
            }
        }
        assert!(g.knots.iter().filter(|k| k.lit.is_some()).count() > KNOTS / 2);
    }

    #[test]
    fn snr_a4_shock_dots_hug_the_blast_wave() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        for secs in [4.0, 10.0, 20.0] {
            run(&mut g, &mut ctx, secs);
            let rf = g.rf_dots();
            let mut dots = Dots::new(Size::new(200, 50));
            g.shock(&mut dots);
            let ds = dots.dot_size();
            let mut n = 0;
            for y in 0..i32::from(ds.height()) {
                for x in 0..i32::from(ds.width()) {
                    if !dots.is_lit(x, y) {
                        continue;
                    }
                    n += 1;
                    let (dx, dy) = (f64::from(x) - g.centre.0, g.centre.1 - f64::from(y));
                    let a = dy.atan2(dx);
                    let want = rf * SupernovaRemnant::shape(&g.wobble, usize::from(bin_of(a)));
                    let r = dx.hypot(dy);
                    assert!(
                        (r - want).abs() <= 1.5,
                        "shock dot at {r:.1} vs R_f {want:.1}"
                    );
                }
            }
            let share = f64::from(n) / (TAU * rf);
            assert!(
                share >= 0.4,
                "shock drawn on {share:.2} of its circumference"
            );
        }
    }

    #[test]
    fn snr_a5_pool_is_capped_and_the_loop_restarts_with_a_flash() {
        let size = Size::new(80, 24);
        let (mut g, mut ctx) = built(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        let mut flashed = false;
        let mut prev_tau = g.tau;
        for _ in 0..(2.5 * LOOP / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            assert!(g.knots.len() <= KNOTS);
            if g.tau < prev_tau {
                // Just restarted: the centre cell is the flash's heart, every knot dark.
                canvas.clear();
                g.render(&mut canvas);
                let (cx, cy) = (
                    (g.centre.0.round() as i32) / Dots::CELL_W,
                    (g.centre.1.round() as i32) / Dots::CELL_H,
                );
                assert_eq!(canvas.get(cx, cy).map(|c| c.ink), Some(FLASH));
                assert!(
                    g.knots
                        .iter()
                        .all(|k| k.lit.is_none() || k.lit == Some(g.tau))
                );
                flashed = true;
            }
            prev_tau = g.tau;
        }
        assert!(flashed);
    }

    #[test]
    fn snr_a6_blast_wave_is_out_of_round_at_low_order_and_reshapes_slowly() {
        for seed in [1_u64, 7, 42, 99, 1234] {
            let size = Size::new(120, 40);
            let mut ctx = Ctx::new(seed, size);
            let mut g = SupernovaRemnant::default();
            g.resize(size, &mut ctx);
            g.prepare();
            for _ in 0..600 {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
            }
            let radii = |g: &SupernovaRemnant| -> (f64, f64) {
                let s = (0..ANGLES).map(|j| SupernovaRemnant::shape(&g.wobble, j));
                s.clone()
                    .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
            };
            let (lo, hi) = radii(&g);
            assert!(hi / lo >= 1.08, "seed {seed}: out of round {}", hi / lo);
            assert!(hi / lo <= 1.2, "seed {seed}: too lopsided {}", hi / lo);
            // Low order: the shape changes sign at most 8 times round the rim (m ≤ 4).
            let crossings = (0..ANGLES)
                .filter(|&j| (g.wobble[j] >= 0.0) != (g.wobble[(j + 1) % ANGLES] >= 0.0))
                .count();
            assert!(crossings <= 8, "seed {seed}: {crossings} sign changes");
            // Slowly: one frame later the shape moved by far less than a pixel's share.
            let before = g.wobble.clone();
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            let moved = before
                .iter()
                .zip(&g.wobble)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(moved < 0.01, "seed {seed}: shape jumped {moved}");
        }
    }

    #[test]
    fn snr_a7_legible_start_and_only_m16_braille_inks() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        let allowed = |ink: Ink| {
            [SHOCK, HOT_EJECTA, FINGER, STUB, FLASH_RING].contains(&ink) || ink.role == Role::Star
        };
        let mut t = g.tau;
        for (i, until) in [1.0, 5.0, 20.0, 37.0, 39.5].into_iter().enumerate() {
            run(&mut g, &mut ctx, until - t);
            t = until;
            canvas.clear();
            g.render(&mut canvas);
            let mut lit = 0;
            for y in 0..50 {
                for x in 0..200 {
                    let c = canvas.get(x, y).unwrap();
                    if c.ch == ' ' {
                        continue;
                    }
                    lit += 1;
                    assert_eq!(c.bg, Ink::BLANK, "half block at {x},{y}");
                    assert!(
                        ('\u{2800}'..='\u{28ff}').contains(&c.ch),
                        "{:?} at {x},{y}",
                        c.ch
                    );
                    assert!(allowed(c.ink), "{:?} at {x},{y} t={until}", c.ink);
                }
            }
            if i == 0 {
                assert!(lit > 60, "{lit} cells lit at 1 s");
            }
        }
    }

    #[test]
    fn snr_a8_filaments_are_thin_radial_threads_that_follow_the_shell() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        g.tau = 3.0;
        g.prepare();
        let early = g.filament_count();
        let (mut thin, mut total) = (0usize, 0usize);
        let mut t = 3.0;
        for until in [8.0, 15.0, 25.0, 30.0] {
            run(&mut g, &mut ctx, until - t);
            t = until;
            g.update_fingers();
            g.update_wobble();
            let rf = g.rf_dots();
            let (dots, list) = filament_dots(&g);
            for &(x, y, _) in &list {
                let (dx, dy) = (f64::from(x) - g.centre.0, f64::from(y) - g.centre.1);
                let r = dx.hypot(dy).max(1e-9);
                // Between the reverse shock and the blast wave (plus a dot of rounding).
                let a = (-dy).atan2(dx);
                let rad = g.radii(usize::from(bin_of(a)), rf);
                let rr = reverse_frac(g.tau.min(EXPAND)) * rad.rf;
                assert!(
                    r >= rr - 2.5 - WIGGLE && r <= rad.rf + 1.0,
                    "r {r:.1} rr {rr:.1} rf {:.1}",
                    rad.rf
                );
                // Across the radial direction.
                let (vx, vy) = (-dy / r, dx / r);
                let across = (-2..=2)
                    .filter(|&k| {
                        let k = f64::from(k);
                        dots.is_lit(
                            (f64::from(x) + k * vx).round() as i32,
                            (f64::from(y) + k * vy).round() as i32,
                        )
                    })
                    .count();
                thin += usize::from(across <= 3);
                total += 1;
            }
        }
        let share = thin as f64 / total as f64;
        assert!(share >= 0.85, "thin share {share:.3}");
        assert!(
            g.filament_count() > early,
            "{early} → {}",
            g.filament_count()
        );
    }

    #[test]
    fn snr_a9_hot_freshly_shocked_ejecta_sit_inside_the_cooler_fingers() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        run(&mut g, &mut ctx, 15.0);
        let rf = g.rf_dots();
        let (_, list) = filament_dots(&g);
        let (mut sun, mut hot) = ((0.0, 0), (0.0, 0));
        for (x, y, ink) in list {
            let r = (f64::from(x) - g.centre.0).hypot(f64::from(y) - g.centre.1) / rf;
            if ink == HOT_EJECTA {
                sun = (sun.0 + r, sun.1 + 1);
            } else {
                hot = (hot.0 + r, hot.1 + 1);
            }
        }
        assert!(sun.1 > 50 && hot.1 > 50, "{sun:?} {hot:?}");
        let (s, h) = (sun.0 / f64::from(sun.1), hot.0 / f64::from(hot.1));
        assert!(s < h, "Sun at {s:.3} R_f, Hot at {h:.3}");
    }

    #[test]
    fn snr_a10_spark_trails_are_radial_and_behind_the_head() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let mut seen = 0;
        let mut t = 0.0;
        for until in [3.0, 10.0, 20.0, 30.0] {
            run(&mut g, &mut ctx, until - t);
            t = until;
            let c = g.centre;
            g.sparks(|tail, head, _| {
                seen += 1;
                let (rt, rh) = (
                    (tail.0 - c.0).hypot(tail.1 - c.1),
                    (head.0 - c.0).hypot(head.1 - c.1),
                );
                assert!(rh > rt, "head {rh} inside tail {rt}");
                // Radial: the tail, the head and the centre are collinear.
                let cross = (tail.0 - c.0) * (head.1 - c.1) - (tail.1 - c.1) * (head.0 - c.0);
                assert!(
                    cross.abs() <= 1e-6 * rh * rh.max(1.0),
                    "not radial: {cross}"
                );
            });
        }
        assert!(seen > 20, "{seen} sparks");
        // The trail is the distance coasted in SPARK_TRAIL.
        let k = Knot {
            u: 0.8,
            dir: (0.0, 1.0),
            lit: Some(0.0),
            bin: 0,
        };
        let len = k.reach(20.0) - k.reach(20.0 - SPARK_TRAIL);
        let v = 0.8 / (T0 + EXPAND);
        assert!((len - v * SPARK_TRAIL).abs() < 1e-9, "{len}");
    }
}
