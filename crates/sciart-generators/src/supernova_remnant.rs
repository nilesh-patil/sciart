//! A young supernova remnant like Cassiopeia A, as a Doppler-coloured shell of ejecta
//! knots (SPEC-002 `supernova_remnant`).
//!
//! Two analytic surfaces, both scaled by the forward-shock radius `R_f(t)` (a
//! Truelove–McKee-style blend from ejecta-dominated to Sedov–Taylor expansion): the blast
//! wave and the reverse shock inside it. Ejecta knots coast outward homologously
//! (`r ∝ t`) on a clumpy 3-D shell and light up as the reverse shock reaches them.
//!
//! M17 (GEN-R26, orbital-thread style; the "Doppler-shell Cas A" design): every knot is a
//! small stipple puff of fixed samples, coloured by its line-of-sight velocity as the two
//! signs of an orbital lobe (`Cold` approaching, `Hot` receding, `Sun` in the limb band
//! where the motion is across the sky), as Chandra's and Spitzer's Doppler maps of Cas A
//! show it. The shell turns slowly so the colours migrate across the limb; knots the
//! reverse shock has just lit flare and leave a short radial afterglow; Rayleigh–Taylor
//! fingers are fine threads trailing inward behind knots; the blast wave is a thin,
//! sparse `Cold` synchrotron rim; the neutron star is a tiny `Star` dot kicked off-centre
//! and drifting. Lengths on screen are in dots (square); the model is in units of `R_f`.

use std::cell::RefCell;
use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::raster::stipple_keep;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "supernova_remnant",
    category: Category::DeepCosmos,
    description: "Ejecta knots of a young remnant, blue coming toward us and red going away, inside a racing blast wave",
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
/// Rayleigh–Taylor growth of the fingers: seed amplitude, rate, cap (exponential, then
/// saturated) for the representative mode `k = RT_K`.
const RT_SEED: f64 = 0.004;
const RT_RATE: f64 = 0.35;
const RT_CAP: f64 = 0.12;
const RT_K: f64 = 12.0;
/// Longest finger thread, a share of `R_f` (before projection).
const FINGER_LEN: f64 = 0.17;
/// Share of knots that trail a finger.
const FINGER_SHARE: f64 = 0.35;
/// Angle table resolution of the remnant's shape.
const ANGLES: usize = 512;
/// Out-of-round shape (share of `R_f`) the whole remnant shares at low angular order
/// (SNR-A6).
const RIM_WOBBLE: f64 = 0.09;
/// Wobble modes `m` = 2, 3, 4: relative amplitudes and the slow drift of their phases
/// (rad/s), so the lopsided shell turns and reshapes as it ploughs into lumpy gas.
const WOBBLE_AMP: [f64; 3] = [1.0, 0.6, 0.35];
const WOBBLE_DRIFT: f64 = 0.05;
/// Ejecta knots (SNR-A5), the clumps they gather in, and their homologous speeds (their
/// radius in `R_f` at the end of the expansion: inside the forward shock, SNR-A12).
pub(crate) const KNOTS: usize = 260;
const CLUMPS: usize = 30;
const CLUMP_SPREAD: f64 = 0.3;
const SCATTERED: f64 = 0.35;
const KNOT_U: (f64, f64) = (0.55, 2.6);
/// Once shocked, a knot eases toward a radius in the shocked shell with time constant
/// `DECEL` (s): `SHELL_DEPTH` times the Rayleigh–Taylor-rippled contact radius
/// (`contact_frac` of a perturbation in `[−1, 1]`), so 0.61–0.75 `R_f`: just outside the
/// reverse shock, where the shell seen edge-on makes Cas A's bright ring.
const SHELL_DEPTH: f64 = 0.8;
const DECEL: f64 = 2.5;
/// Half-height (dots) at and above which every knot is drawn (200×50).
const FULL_HALF: f64 = 100.0;
/// A knot's flare when the reverse shock lights it, and the afterglow it fades to.
const KNOT_TAU: f64 = 1.5;
const KNOT_FLOOR: f64 = 0.4;
/// Puffs (GEN-R26): a Gaussian stipple (σ = `PUFF_SD` dots) over a box `PUFF_R` dots
/// either side of the knot, keep probability at most `PUFF_PEAK` at the core, with
/// thresholds fixed per dot relative to the knot.
const PUFF_R: i32 = 3;
const PUFF_W: i32 = 2 * PUFF_R + 1;
const PUFF_SD: f64 = 1.25;
const PUFF_PEAK: f64 = 0.5;
/// RT finger threads: length on screen (dots), clamped.
const FINGER_DOTS: (f64, f64) = (3.0, 8.0);
/// The shell turns about a tilted axis (rad/s), so the Doppler colours migrate.
const SPIN: f64 = 0.045;
const TILT: f64 = 0.35;
/// Line-of-sight speed (share of the knot's speed) inside which a knot is in the limb
/// band (SNR-A11).
pub(crate) const LIMB_BAND: f64 = 0.3;
/// Afterglow trail (SNR-A10): drawn while a knot's flare is at least `TRAIL_MIN`, along
/// the path it coasted in the last `TRAIL` seconds.
const TRAIL_MIN: f64 = 0.55;
pub(crate) const TRAIL: f64 = 1.2;
/// The forward shock (SNR-A4, SNR-A14): golden-angle samples, `RIM_DENSITY` per dot of
/// circumference, each on the rim or `RIM_INSET` dots inside (fixed per sample), two dots thick.
const RIM_SAMPLES: usize = 1400;
const RIM_DENSITY: f64 = 0.55;
const RIM_INSET: f64 = 0.7;
/// Golden-angle fraction: any prefix of `frac(i·φ)` is evenly spread round the circle.
const GOLDEN: f64 = 0.618_033_988_749_895;
/// Neutron-star kick: its offset at the end of the expansion, a share of `R_END·half`.
const KICK: f64 = 0.11;

/// Inks (M17): Doppler roles for the knots, a faint blue blast wave, a white neutron star.
pub(crate) const SHOCK: Ink = Ink::new(Role::Cold, 110);
/// Knot levels: flaring, afterglow near the limb and face-on; trail and finger threads
/// (head side, root side).
const LEVEL_FLARE: u8 = 255;
const LEVEL_GLOW: u8 = 210;
const LEVEL_FACE: u8 = 160;
const LEVEL_TRAIL: u8 = 130;
const LEVEL_FINGER: (u8, u8) = (150, 90);
/// The neutron star, and the explosion's white heart.
const NEUTRON: Ink = Ink::new(Role::Star, 230);
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
/// `R_r < R_c < R_f` always (SNR-A2). The finger threads' lengths stay inside this band.
pub(crate) fn contact_frac(p: f64) -> f64 {
    let out = (p.max(0.0) / 0.25).tanh();
    let inn = ((-p).max(0.0) / 0.25).tanh();
    CONTACT * (1.0 + FINGER_OUT * out * out - FINGER_IN * inn)
}

/// Rayleigh–Taylor finger growth at loop time `tau`, in `[0, 1]`: exponential growth of
/// the representative mode, saturated at the cap.
fn rt_growth(tau: f64) -> f64 {
    (RT_SEED * (RT_RATE * (RT_K / 6.0).sqrt() * tau.max(0.0)).exp()).min(RT_CAP) / RT_CAP
}

/// The blast wave's early growth from the flash (a share of `R_f`, 0 → 1 over 1.2 s).
fn grow(tau: f64) -> f64 {
    ease::out_quad(tau / 1.2)
}

/// The Doppler role of a knot whose unit direction has line-of-sight component `vz`
/// (positive toward the viewer) (SNR-A11): `Cold` (blueshifted) when it approaches by
/// more than the limb band, `Hot` (redshifted) when it recedes, `Sun` in the band.
pub(crate) fn doppler_role(vz: f64) -> Role {
    if vz > LIMB_BAND {
        Role::Cold
    } else if vz < -LIMB_BAND {
        Role::Hot
    } else {
        Role::Sun
    }
}

/// A knot of ejecta: its free homologous speed (its radius in `R_f` at the end of the
/// expansion if it were never shocked), unit direction in the remnant's frame, when the
/// reverse shock lit it (loop s) and where (share of `R_f`), the radius in the shocked
/// shell it decelerates toward, its puff salt and size, and its finger.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Knot {
    u: f64,
    dir: [f64; 3],
    lit: Option<f64>,
    lit_frac: f64,
    shell: f64,
    salt: u16,
    /// Its puff's strength (0.75–1).
    mass: f64,
    /// Finger length as a share of `FINGER_LEN` (0 = no finger).
    finger: f64,
}

impl Knot {
    /// Radius in units of `R_f` at loop time `tau`: free coasting (`r ∝ t`, so it gains
    /// on the slowing blast wave) until the reverse shock lights it, then decelerated
    /// into the shocked shell, easing from where it was lit toward its shell radius, so
    /// shocked ejecta pile up between the reverse shock and the contact, inside the
    /// blast wave (SNR-A12).
    fn frac(&self, tau: f64, r_end: f64) -> f64 {
        match self.lit {
            Some(at) if tau >= at => {
                let target = self.shell.max(self.lit_frac);
                self.lit_frac + (target - self.lit_frac) * (1.0 - (-(tau - at) / DECEL).exp())
            }
            _ => {
                let t = T0 + tau;
                self.u * (t / (T0 + EXPAND)) * r_end / forward_shock(t)
            }
        }
    }

    /// Radius in units of the end radius `R_END·half` at loop time `tau` (≤ `EXPAND`),
    /// before the shape factor: `frac · R_f`, times the early growth the blast wave
    /// shares. Non-decreasing in `tau`, so a trail always lies behind its knot.
    fn reach(&self, tau: f64, r_end: f64) -> f64 {
        let tau = tau.clamp(0.0, EXPAND);
        self.frac(tau, r_end) * forward_shock(T0 + tau) / r_end * grow(tau)
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

/// A knot as seen now: screen position (dots), unit radial screen direction, projected
/// radius per unit 3-D radius (`sin` of its angle to the line of sight), line-of-sight
/// direction `vz`, brightness, and the knot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KnotView {
    pos: (f64, f64),
    radial: (f64, f64),
    /// 3-D radius in dots (with the shape factor).
    r3: f64,
    sin_los: f64,
    vz: f64,
    b: f64,
    knot: Knot,
}

#[derive(Debug, Default)]
pub struct SupernovaRemnant {
    seeded: bool,
    /// Loop time.
    tau: f64,
    /// The remnant's low-order shape per angle bin this frame, in `[−1, 1]`, and its
    /// modes (phase, amplitude, drift).
    wobble: Vec<f32>,
    wob_modes: [(f64, f64, f64); 3],
    knots: ParticlePool<Knot>,
    /// The puff's Gaussian kernel over its `PUFF_W²` box.
    puff_kernel: Vec<f64>,
    /// Rim samples: angle and radial offset (0 or −1 dot), golden-angle order.
    rim: Vec<(f64, f64)>,
    /// Neutron-star kick direction (screen unit vector).
    kick: (f64, f64),
    /// Screen centre and half-height, in dots.
    centre: (f64, f64),
    half: f64,
 /// Knots drawn at this size (a prefix of the pool).
 drawn: usize,
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
        // The puff kernel: a Gaussian of σ = `PUFF_SD` dots over the puff box.
        self.puff_kernel = vec![0.0; (PUFF_W * PUFF_W) as usize];
        for dy in -PUFF_R..=PUFF_R {
            for dx in -PUFF_R..=PUFF_R {
                let d2 = f64::from(dx * dx + dy * dy);
                self.puff_kernel[((dy + PUFF_R) * PUFF_W + dx + PUFF_R) as usize] =
                    (-d2 / (2.0 * PUFF_SD * PUFF_SD)).exp();
            }
        }
        self.rim = (0..RIM_SAMPLES)
            .map(|i| {
                let a = TAU * (i as f64 * GOLDEN).fract();
                let inset = if hash01(cell_key(i as i32, 29)) < 0.5 {
                    0.0
                } else {
                    -RIM_INSET
                };
                (a, inset)
            })
            .collect();
        self.knots = ParticlePool::with_capacity(KNOTS);
        self.tau = ctx.rng.random_range(5.2..5.6);
        self.explode(ctx);
        self.light_knots();
    }

    /// A new supernova: fresh knots (all dark, inside the reverse shock) gathered in
    /// clumps on the shell, and a new neutron-star kick.
    fn explode(&mut self, ctx: &mut Ctx) {
        self.knots.clear();
        let clumps: [[f64; 3]; CLUMPS] = std::array::from_fn(|_| unit(&mut ctx.rng));
        for _ in 0..KNOTS {
            let dir = if ctx.rng.random::<f64>() < SCATTERED {
                unit(&mut ctx.rng)
            } else {
                let c = clumps[ctx.rng.random_range(0..CLUMPS)];
                let j = unit(&mut ctx.rng);
                let s = CLUMP_SPREAD * ctx.rng.random::<f64>();
                let v = [c[0] + s * j[0], c[1] + s * j[1], c[2] + s * j[2]];
                let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
                [v[0] / n, v[1] / n, v[2] / n]
            };
            let finger = if ctx.rng.random::<f64>() < FINGER_SHARE {
                ctx.rng.random_range(0.35..1.0)
            } else {
                0.0
            };
            self.knots.spawn(Knot {
                u: ctx.rng.random_range(KNOT_U.0..KNOT_U.1),
                dir,
                lit: None,
                lit_frac: 0.0,
                shell: SHELL_DEPTH * contact_frac(ctx.rng.random_range(-1.0..1.0)),
                salt: ctx.rng.random(),
                mass: ctx.rng.random_range(0.75..1.0),
                finger,
            });
        }
        let a: f64 = ctx.rng.random_range(0.0..TAU);
        let (s, c) = a.sin_cos_det();
        self.kick = (c, s);
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
            if k.lit.is_none() {
                let f = k.frac(tau, r_end);
                if f > rr {
                    k.lit = Some(self.tau);
                    k.lit_frac = f.min(SHELL_DEPTH * CONTACT);
                }
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

    /// The shape factor at an angle bin: every radius (blast wave, reverse shock, knots)
    /// scales with it, so the ordering SNR-A2 / SNR-A12 holds at every angle.
    fn shape(wobble: &[f32], bin: usize) -> f64 {
        1.0 + RIM_WOBBLE * f64::from(wobble.get(bin).copied().unwrap_or(0.0))
    }

    fn envelope(&self) -> f64 {
        1.0 - ease::smoothstep((self.tau - EXPAND) / FADE)
    }

    /// The shell's orientation now: a turn about the vertical, then a fixed tilt toward
    /// the viewer, applied to a knot's unit direction. Returns `(x right, y up, z toward
    /// the viewer)`.
    fn orient(&self, d: [f64; 3]) -> [f64; 3] {
        let (s, c) = (SPIN * self.tau).sin_cos_det();
        let (x, z) = (d[0] * c + d[2] * s, -d[0] * s + d[2] * c);
        let (st, ct) = TILT.sin_cos_det();
        let (y, z) = (d[1] * ct - z * st, d[1] * st + z * ct);
        [x, y, z]
    }

    /// Every lit knot as seen now (SNR-A11, SNR-A12).
    fn knot_views(&self, mut f: impl FnMut(&KnotView)) {
        let scale = R_END * self.half;
        let (tau, r_end) = (self.tau.min(EXPAND), self.r_end());
 for k in self.knots.iter().take(self.drawn) {
            let b = k.brightness(self.tau);
            if b <= 0.0 {
                continue;
            }
            let o = self.orient(k.dir);
            let sin_los = o[0].hypot(o[1]);
            let radial = if sin_los > 1e-9 {
                (o[0] / sin_los, -o[1] / sin_los)
            } else {
                (1.0, 0.0)
            };
            let bin = usize::from(bin_of((-radial.1).atan2(radial.0)));
            let r3 = k.reach(tau, r_end) * scale * Self::shape(&self.wobble, bin);
            let rp = r3 * sin_los;
            f(&KnotView {
                pos: (self.centre.0 + rp * radial.0, self.centre.1 + rp * radial.1),
                radial,
                r3,
                sin_los,
                vz: o[2],
                b,
                knot: *k,
            });
        }
    }

    /// The radial afterglow of a freshly lit knot as `(tail, head)` in dots (SNR-A10):
    /// the path it coasted in the last `TRAIL` seconds, projected.
    fn trail(&self, v: &KnotView) -> Option<((f64, f64), (f64, f64))> {
        if v.b < TRAIL_MIN {
            return None;
        }
        let (tau, r_end) = (self.tau.min(EXPAND), self.r_end());
        let back = v.knot.reach(tau - TRAIL, r_end) / v.knot.reach(tau, r_end).max(1e-9);
        let rt = v.r3 * back * v.sin_los;
        Some((
            (
                self.centre.0 + rt * v.radial.0,
                self.centre.1 + rt * v.radial.1,
            ),
            v.pos,
        ))
    }

    fn raster(&mut self) {
        self.update_wobble();
        let mut dots = self.dots.borrow_mut();
        dots.clear();
        // Background stars first: everything else outshines them.
        for &(x, y, ink) in &self.stars {
            dots.set(x, y, ink);
        }
        self.shock(&mut dots);
        let env = self.envelope();
        let growth = rt_growth(self.tau.min(EXPAND));
        let rf = self.rf_dots();
        self.knot_views(|v| {
            let role = doppler_role(v.vz);
            // Rayleigh–Taylor finger: a fine thread trailing inward from the knot, its
            // length growing with the instability and foreshortened by the projection.
            // One dot wide and `FINGER_DOTS` long, drawn behind the brighter knots seen
            // well off the line of sight.
            let len = v.knot.finger * FINGER_LEN * rf * growth * v.sin_los * env;
            if len >= 1.5 && v.sin_los > 0.45 {
                let len = len.clamp(FINGER_DOTS.0, FINGER_DOTS.1);
                let (rx, ry) = v.radial;
                let start = (v.pos.0 - 2.0 * rx, v.pos.1 - 2.0 * ry);
                let mid = (start.0 - 0.5 * len * rx, start.1 - 0.5 * len * ry);
                let root = (start.0 - len * rx, start.1 - len * ry);
                dots.line(start.0, start.1, mid.0, mid.1, Ink::new(role, LEVEL_FINGER.0));
                dots.line(mid.0, mid.1, root.0, root.1, Ink::new(role, LEVEL_FINGER.1));
            }
            if let Some((tail, head)) = self.trail(v) {
                dots.line(tail.0, tail.1, head.0, head.1, Ink::new(role, LEVEL_TRAIL));
            }
            // The puff: a hydrogen-style Gaussian stipple, densest at the centre (keep
            // ≤ `PUFF_PEAK`), stronger while it flares and near the limb, where the line of
            // sight runs along the shell (limb brightening). Thresholds are fixed per dot
            // *relative to the knot*, so the puff moves as one cloud without shimmer.
            let weight = 0.3 + 0.2 * v.b + 0.5 * v.sin_los * v.sin_los * v.sin_los;
            let peak = PUFF_PEAK * v.knot.mass * env * weight;
            let level = if v.b >= 0.7 {
                LEVEL_FLARE
            } else if v.sin_los > 0.6 {
                LEVEL_GLOW
            } else {
                LEVEL_FACE
            };
            let ink = Ink::new(role, level);
            let (x, y) = (v.pos.0.round() as i32, v.pos.1.round() as i32);
            if env > 0.05 {
                dots.set(x, y, ink);
            }
            let salt = 0x5_0FF0_0000 ^ u64::from(v.knot.salt).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            for dy in -PUFF_R..=PUFF_R {
                for dx in -PUFF_R..=PUFF_R {
                    let p = peak * self.puff_kernel[((dy + PUFF_R) * PUFF_W + dx + PUFF_R) as usize];
                    if stipple_keep(salt, dx, dy, p) {
                        dots.set(x + dx, y + dy, ink);
                    }
                }
            }
        });
        self.centre_star(&mut dots);
        drop(dots);
    }

    /// The blast wave: a thin, sparse rim (SNR-A4, SNR-A14): a golden-angle prefix of
    /// fixed samples, `RIM_DENSITY` per dot of circumference, each on `R_f(θ)` or one dot
    /// inside, so it holds still as it grows and thins out with the fade envelope.
    fn shock(&self, dots: &mut Dots) {
        let rf = self.rf_dots();
        if rf < 1.0 {
            return;
        }
        let n = (TAU * rf * RIM_DENSITY * self.envelope()).ceil() as usize;
        for &(a, inset) in &self.rim[..n.min(self.rim.len())] {
            let r = rf * Self::shape(&self.wobble, usize::from(bin_of(a))) + inset;
            let (s, c) = a.sin_cos_det();
            dots.plot(self.centre.0 + r * c, self.centre.1 - r * s, SHOCK);
        }
    }

    /// The neutron star's position (dots): kicked off-centre at birth, drifting outward
    /// at a constant speed (SNR-A13).
    fn neutron_star(&self) -> (f64, f64) {
        let tau = self.tau.min(EXPAND);
        let d = KICK * R_END * self.half * (T0 + tau) / (T0 + EXPAND);
        (
            self.centre.0 + d * self.kick.0,
            self.centre.1 - d * self.kick.1,
        )
    }

    /// The neutron star, and in the first 1.5 s the flash: a `Sun` ring and `Star` arms
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
        if flash > 0.3 {
            dots.set(cx, cy, FLASH);
        } else {
            let (x, y) = self.neutron_star();
            dots.plot(x, y, NEUTRON);
        }
    }
}

/// A unit vector uniform on the sphere.
fn unit(rng: &mut impl Rng) -> [f64; 3] {
    let z: f64 = rng.random_range(-1.0..1.0);
    let a: f64 = rng.random_range(0.0..TAU);
    let s = (1.0 - z * z).max(0.0).sqrt();
    let (sa, ca) = a.sin_cos_det();
    [s * ca, s * sa, z]
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
 // Small screens show a prefix of the (randomly ordered) knots, so the shell keeps
 // its black gaps instead of crowding into a blot.
 self.drawn = (KNOTS as f64 * (self.half / FULL_HALF).clamp(0.4, 1.0)).round() as usize;
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
        // The fingers reach no further than the contact band allows.
        assert!(FINGER_LEN < contact_frac(10.0) - REVERSE.1);
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

    /// The rim's lit dots alone, on a fresh lattice.
    fn rim_dots(g: &SupernovaRemnant) -> Vec<(f64, f64)> {
        let mut dots = Dots::new(Size::new(200, 50));
        g.shock(&mut dots);
        let ds = dots.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if dots.is_lit(x, y) {
                    let (dx, dy) = (f64::from(x) - g.centre.0, g.centre.1 - f64::from(y));
                    out.push((dy.atan2(dx), dx.hypot(dy)));
                }
            }
        }
        out
    }

    #[test]
    fn snr_a4_shock_dots_hug_the_blast_wave() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        for secs in [2.0, 6.0, 10.0] {
            run(&mut g, &mut ctx, secs);
            let rf = g.rf_dots();
            let lit = rim_dots(&g);
            for &(a, r) in &lit {
                let want = rf * SupernovaRemnant::shape(&g.wobble, usize::from(bin_of(a)));
                assert!(
                    (r - want).abs() <= 1.5,
                    "shock dot at {r:.1} vs R_f {want:.1}"
                );
            }
            let share = lit.len() as f64 / (TAU * rf);
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
    fn snr_a7_legible_start_and_only_m17_braille_inks() {
        // M17: braille only, in the Doppler roles (Cold / Hot / Sun), the flash's ring and
        // the white stars and neutron star.
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = built(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        let allowed = |ink: Ink| {
            ink == FLASH_RING || matches!(ink.role, Role::Cold | Role::Hot | Role::Sun | Role::Star)
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
    fn snr_a10_knot_afterglow_trails_are_radial_and_behind_the_head() {
        // M17: the sparks of M16 are now the flaring knots' afterglow trails.
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let mut seen = 0;
        let mut t = 0.0;
        for until in [3.0, 10.0, 20.0, 30.0] {
            run(&mut g, &mut ctx, until - t);
            t = until;
            let c = g.centre;
            g.knot_views(|v| {
                let Some((tail, head)) = g.trail(v) else {
                    return;
                };
                seen += 1;
                let (rt, rh) = (
                    (tail.0 - c.0).hypot(tail.1 - c.1),
                    (head.0 - c.0).hypot(head.1 - c.1),
                );
                assert!(rh >= rt, "head {rh} inside tail {rt}");
                // Radial: the tail, the head and the centre are collinear.
                let cross = (tail.0 - c.0) * (head.1 - c.1) - (tail.1 - c.1) * (head.0 - c.0);
                assert!(
                    cross.abs() <= 1e-6 * rh * rh.max(1.0),
                    "not radial: {cross}"
                );
            });
        }
        assert!(seen > 20, "{seen} trails");
        // The trail is the path covered in TRAIL: for a free knot the distance coasted,
        // `u·TRAIL/t_end`; a shocked knot rides the shell, still outward.
        let mut k = Knot {
            u: 0.8,
            dir: [1.0, 0.0, 0.0],
            lit: None,
            lit_frac: 0.0,
            shell: 0.8,
            salt: 0,
            mass: 1.0,
            finger: 0.0,
        };
        let r_end = forward_shock(T0 + EXPAND);
        let len = k.reach(20.0, r_end) - k.reach(20.0 - TRAIL, r_end);
        let v = 0.8 / (T0 + EXPAND);
        assert!((len - v * TRAIL).abs() < 1e-9, "{len}");
        k.lit_frac = k.frac(18.8, r_end);
        k.lit = Some(18.8);
        let shocked = k.reach(20.0, r_end) - k.reach(20.0 - TRAIL, r_end);
        assert!(shocked >= 0.0, "{shocked}");
    }

    #[test]
    fn snr_a11_knots_are_coloured_by_line_of_sight_velocity() {
        assert_eq!(doppler_role(0.9), Role::Cold);
        assert_eq!(doppler_role(LIMB_BAND + 1e-6), Role::Cold);
        assert_eq!(doppler_role(LIMB_BAND), Role::Sun);
        assert_eq!(doppler_role(0.0), Role::Sun);
        assert_eq!(doppler_role(-LIMB_BAND), Role::Sun);
        assert_eq!(doppler_role(-0.9), Role::Hot);
        // In the scene: each knot's core dot carries its Doppler role, and all three
        // roles are present (a shell has a near side, a far side and a limb).
        let (mut g, mut ctx) = built(Size::new(200, 50));
        run(&mut g, &mut ctx, 14.0);
        let dots = g.dots.borrow();
        let mut roles = [0usize; 3];
        let mut views = Vec::new();
        g.knot_views(|v| views.push(*v));
        assert!(views.len() > KNOTS / 2, "{} lit knots", views.len());
        for v in &views {
            let want = doppler_role(v.vz);
            let i = match want {
                Role::Cold => 0,
                Role::Hot => 1,
                _ => 2,
            };
            roles[i] += 1;
            // The knot's limb-band status follows its projected radius: a knot moving
            // across the sky is seen near the shell's rim.
            assert!((v.sin_los * v.sin_los + v.vz * v.vz - 1.0).abs() < 1e-9);
            let (x, y) = (v.pos.0.round() as i32, v.pos.1.round() as i32);
            assert!(dots.is_lit(x, y), "knot core unlit at {x},{y}");
        }
        assert!(roles.iter().all(|&n| n > 10), "roles {roles:?}");
    }

    #[test]
    fn snr_a12_every_knot_lies_inside_the_forward_shock() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let mut t = g.tau;
        for until in [6.0, 12.0, 20.0, 30.0, 35.9] {
            run(&mut g, &mut ctx, until - t);
            t = until;
            let rf = g.rf_dots();
            let c = g.centre;
            g.knot_views(|v| {
                let (dx, dy) = (v.pos.0 - c.0, c.1 - v.pos.1);
                let bin = usize::from(bin_of(dy.atan2(dx)));
                let shock = rf * SupernovaRemnant::shape(&g.wobble, bin);
                assert!(
                    dx.hypot(dy) < shock,
                    "knot at {:.1} outside the shock {shock:.1}",
                    dx.hypot(dy)
                );
                assert!(v.r3 < shock + 1e-9);
            });
        }
    }

    #[test]
    fn snr_a13_the_neutron_star_is_kicked_off_centre_and_drifts() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        run(&mut g, &mut ctx, 1.0);
        let a = g.neutron_star();
        run(&mut g, &mut ctx, 10.0);
        let b = g.neutron_star();
        let off = |p: (f64, f64)| (p.0 - g.centre.0).hypot(p.1 - g.centre.1);
        assert!(off(a) > 1.0, "off-centre by {:.2}", off(a));
        assert!(off(b) > off(a) + 1.0, "drifts {:.2} → {:.2}", off(a), off(b));
        // It is drawn there, a single white dot.
        let dots = g.dots.borrow();
        assert!(dots.is_lit(b.0.round() as i32, b.1.round() as i32));
    }

    #[test]
    fn snr_a14_the_forward_shock_rim_is_thin_and_sparse() {
        let (mut g, mut ctx) = built(Size::new(200, 50));
        let mut t = g.tau;
        for until in [3.0, 10.0, 25.0] {
            run(&mut g, &mut ctx, until - t);
            t = until;
            let rf = g.rf_dots();
            let lit = rim_dots(&g);
            // Thin: in each 2° sector the rim's dots span at most two dots radially
            // (plus the rounding of plotting).
            let mut span = [(f64::MAX, f64::MIN); 180];
            for &(a, r) in &lit {
                let s = &mut span[((a / TAU).rem_euclid(1.0) * 180.0) as usize % 180];
                *s = (s.0.min(r), s.1.max(r));
            }
            for (i, &(lo, hi)) in span.iter().enumerate() {
                if hi >= lo {
                    assert!(hi - lo <= 2.0 + 0.8, "sector {i}: {lo:.1}..{hi:.1}");
                }
            }
            // Sparse: at most half the dots of the two-dot band are lit.
            let band = 2.0 * TAU * rf;
            assert!(
                (lit.len() as f64) <= 0.5 * band,
                "{} of {band:.0} rim dots lit",
                lit.len()
            );
        }
    }
}
