//! Mitosis: one cell divides. Chromosomes condense out of the chromatin, line up on the
//! spindle, their sister chromatids are pulled to opposite poles, and the cell pinches in
//! two; the view then settles on one daughter, which becomes the next loop's cell
//! (SPEC-002 `mitosis`).
//!
//! A kinematic model on a looping `Timeline` (~30 s), all poses closed-form in the phase's
//! local time, so resizes never desync it and tests can read any instant directly:
//! - four homologous pairs (8 chromosomes, maternal `Hot`, paternal `Cold`), each a pair of
//!   sister chromatids joined at the centromere, drawn as an X (each chromatid a chevron
//!   whose vertex is its kinetochore);
//! - prophase: the chromatin haze condenses into chromosomes; the duplicated centrosomes
//!   part, one staying at the old pole, the other travelling over the nucleus;
//! - metaphase: the envelope dissolves and every chromosome rides a damped spring onto its
//!   slot on the metaphase plate (`x(t) = x₀ e^{−γt} cos ωt`);
//! - anaphase: sisters part at constant speed towards their poles (anaphase A) while the
//!   cell lengthens (anaphase B), arms trailing;
//! - telophase: chromatids decondense into new nuclei; a contractile ring starts;
//! - cytokinesis: the ring closes (the waist falls to zero), then the view zooms onto the
//!   left daughter, which grows to the full cell as the next loop starts.
//!
//! The membrane is the zero level of a signed distance field: two circles of radius `a`
//! about `(±e, 0)` joined by a polynomial smooth-min (blend `K`), plus a Gaussian ring
//! constriction `κ·exp(−x²/2w²)` at the equator. Its waist half-width is then exactly
//! `√((a + K/4 − κ)² − e²)` (zero once `κ ≥ a + K/4 − e`), and it is drawn by marching
//! squares (`Dots::contour`) on a capped node grid, recomputed only when the shape moves.
//!
//! ## Look (GEN-R26, molecules)
//! Braille only: chromatids as short chains of small stipple atoms with a solid core,
//! centrosomes small `Accent` atoms with short astral rays, spindle microtubules 1-px
//! `Dim` threads from the centrosomes to the kinetochores, the membrane a 1-px `Structure`
//! thread, the nuclear envelope a dotted `Dim` thread, chromatin a sparse fixed-threshold
//! stipple haze.

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01, value_noise};
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "mitosis",
    category: Category::Biophysics,
    description: "A cell divides: chromosomes condense, align on the spindle, split and the cell pinches in two",
    title: "Mitosis · chromosomes split",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 5] = [
    "prophase",
    "metaphase",
    "anaphase",
    "telophase",
    "cytokinesis",
];
const PRO: usize = 0;
const MET: usize = 1;
const ANA: usize = 2;
const TEL: usize = 3;
const CYT: usize = 4;
/// Phase lengths (s): about an hour of mitosis in half a minute.
const DURATIONS: [f64; 5] = [7.0, 7.0, 5.0, 5.0, 6.0];

pub fn build() -> Box<dyn Generator> {
    Box::new(Mitosis::default())
}

// ---- Model (SPEC-002 Model), in cell units: the interphase cell has radius 1 ----

/// Chromosomes: four homologous pairs, one maternal and one paternal copy each.
pub(crate) const CHROMOSOMES: usize = 8;
/// Atoms per chromatid arm, by pair (pairs differ in size, as real chromosomes do).
const ARMS: [u8; 4] = [3, 3, 2, 2];
/// Atoms in one chromatid at most: the centromere and two arms.
const MAX_ATOMS: usize = 1 + 2 * 3;
/// Alternate chromosomes sit this far (dots) either side of the plate.
const STAGGER: f64 = 1.5;
/// Smooth-min blend of the two daughter circles.
pub(crate) const K: f64 = 0.5;
/// Circle radius of the undivided cell, so that with the blend its radius is exactly 1.
const A_PRO: f64 = 1.0 - K / 4.0;
/// Width of the contractile ring's constriction.
const RING_W: f64 = 0.18;
/// The ring's share of its closing strength at the end of anaphase.
const RING_ANA: f64 = 0.15;
/// Centrosome distance from its daughter's centre, as a share of the daughter's radius.
const POLE: f64 = 0.66;
/// Nucleus radius, as a share of the cell's radius.
const NUC: f64 = 0.55;
/// Metaphase-plate slots span `±PLATE_SPAN`, and a kinetochore within `PLATE_BAND` of
/// the plate counts as aligned (MIT-A2).
const PLATE_SPAN: f64 = 0.68;
#[cfg(test)]
pub(crate) const PLATE_BAND: f64 = 0.1;
/// Half the distance between sister kinetochores, and the chromosome-centre spread in
/// prophase.
pub(crate) const SISTER: f64 = 0.014;
const SCATTER: f64 = 0.34;
/// Metaphase congression: a damped spring onto the plate.
const GAMMA: f64 = 1.0;
const OMEGA: f64 = 2.6;
/// Anaphase: the cell's half-length at its end, and the share of anaphase the chromatids
/// spend moving (then they wait at the poles).
const E_ANA: f64 = 0.4;
const ANA_MOVE: f64 = 0.85;
/// Chromatids sit this far poleward of their daughter's centre in telophase.
const NUC_SHIFT: f64 = 0.1;
/// Arm angle from the pole axis: flared in an X at metaphase, trailing in anaphase.
const ALPHA_MET: f64 = 0.9;
const ALPHA_ANA: f64 = 2.55;

/// One chromosome (both sisters): its pair, parent, size and this loop's scatter.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Chromosome {
    pub(crate) pair: u8,
    pub(crate) maternal: bool,
    /// Prophase centre and orientation of the sister axis.
    pub(crate) start: [f64; 2],
    pub(crate) beta: f64,
    /// Its slot on the metaphase plate (y).
    pub(crate) slot: f64,
}

/// A chromatid's pose: the kinetochore (cell units), the sister axis angle, the arm angle
/// from the pole-ward axis and how condensed it is (0 haze … 1 fully condensed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    pub(crate) kin: [f64; 2],
    pub(crate) beta: f64,
    pub(crate) alpha: f64,
    pub(crate) shown: f64,
}

/// The cell's outline parameters and the view's zoom progress `z` onto the left daughter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Shape {
    pub(crate) e: f64,
    pub(crate) a: f64,
    pub(crate) kappa: f64,
    pub(crate) z: f64,
}

/// The ring strength at which the waist closes for lobes `e` apart of radius `a`.
fn kappa_close(e: f64, a: f64) -> f64 {
    a + K / 4.0 - e
}

/// The cell outline in phase `act` at progress `p` (MIT-A4).
pub(crate) fn shape(act: usize, p: f64) -> Shape {
    let still = Shape {
        e: 0.0,
        a: A_PRO,
        kappa: 0.0,
        z: 0.0,
    };
    match act {
        ANA => {
            let q = ease::smoothstep(p);
            // The ring starts to gather late in anaphase.
            let (e, a) = (E_ANA * q, ease::lerp(A_PRO, 0.8, q));
            Shape {
                e,
                a,
                kappa: RING_ANA * kappa_close(e, a) * q * q,
                z: 0.0,
            }
        }
        TEL => {
            let q = ease::smoothstep(p);
            let (e, a) = (ease::lerp(E_ANA, 0.6, q), ease::lerp(0.8, 0.72, q));
            Shape {
                e,
                a,
                kappa: kappa_close(e, a) * ease::lerp(RING_ANA, 0.45, q),
                z: 0.0,
            }
        }
        CYT if p < 0.5 => {
            // The ring closes: the waist reaches zero just before halfway.
            let q = ease::smoothstep(2.0 * p);
            let k = kappa_close(0.6, 0.72);
            Shape {
                e: 0.6,
                a: 0.72,
                kappa: ease::lerp(0.45 * k, k + 0.03, q),
                z: 0.0,
            }
        }
        CYT => {
            // The daughters part and the view settles on the left one; the ring relaxes
            // only as fast as they part, so the waist stays shut.
            let z = ease::smoothstep(2.0 * p - 1.0);
            let e = 0.6 + 0.8 * z;
            Shape {
                e,
                a: 0.72,
                kappa: (kappa_close(e, 0.72) + 0.03).max(0.0),
                z,
            }
        }
        _ => still,
    }
}

/// The membrane's signed distance field: negative inside (MIT-A4).
pub(crate) fn membrane(s: &Shape, x: f64, y: f64) -> f64 {
    let d1 = (x + s.e).hypot(y) - s.a;
    let d2 = (x - s.e).hypot(y) - s.a;
    let h = (K - (d1 - d2).abs()).max(0.0) / K;
    let d = d1.min(d2) - h * h * K / 4.0;
    if s.kappa > 0.0 && x.abs() < 5.0 * RING_W {
        d + s.kappa * (-(x * x) / (2.0 * RING_W * RING_W)).exp()
    } else {
        d
    }
}

/// The waist half-width at the equator: `√((a + K/4 − κ)² − e²)`, 0 once pinched.
#[cfg(test)]
pub(crate) fn waist(s: &Shape) -> f64 {
    let r = s.a + K / 4.0 - s.kappa;
    if r <= s.e {
        0.0
    } else {
        (r * r - s.e * s.e).sqrt()
    }
}

/// The view: focus x (cell units) and zoom, moving onto the left daughter so that it ends
/// as the next loop's full cell.
pub(crate) fn view(s: &Shape) -> (f64, f64) {
    (-s.e * s.z, 1.0 + (1.0 / s.a - 1.0) * s.z)
}

/// Centrosome `side` (−1 left, +1 right) in phase `act` at progress `p`.
pub(crate) fn centrosome(act: usize, p: f64, side: f64) -> [f64; 2] {
    if act == PRO {
        // The left one holds the old pole; its duplicate travels over the nucleus.
        let th = if side < 0.0 {
            std::f64::consts::PI
        } else {
            std::f64::consts::PI * (1.0 + ease::smoothstep((p - 0.1) / 0.75))
        };
        let (s, c) = th.sin_cos_det();
        return [POLE * c, POLE * s];
    }
    let sh = shape(act, p);
    let r = sh.a + K / 4.0 * (1.0 - sh.z);
    [side * (sh.e + POLE * r), 0.0]
}

/// The daughter nucleus `side` (centre x, radius) once it reforms (telophase on).
fn daughter_nucleus(sh: &Shape, side: f64) -> ([f64; 2], f64) {
    let r = sh.a + K / 4.0 * (1.0 - sh.z);
    ([side * (sh.e + NUC_SHIFT * (1.0 - sh.z)), 0.0], NUC * r)
}

/// The pose of chromosome `c`'s chromatid on `side` at local time `t` of phase `act`
/// (progress `p`).
pub(crate) fn pose(c: &Chromosome, side: f64, act: usize, p: f64, t: f64) -> Pose {
    let at = |centre: [f64; 2], beta: f64, alpha: f64, shown: f64| {
        let (s, co) = beta.sin_cos_det();
        Pose {
            kin: [
                centre[0] + side * SISTER * co,
                centre[1] + side * SISTER * s,
            ],
            beta,
            alpha,
            shown,
        }
    };
    match act {
        PRO => {
            // Condensing in place, drifting a little in the nucleoplasm.
            let (s, co) = (0.4 * t + 1.7 * c.beta).sin_cos_det();
            let centre = [c.start[0] + 0.015 * co, c.start[1] + 0.015 * s];
            at(centre, c.beta, ALPHA_MET, ease::smoothstep((p - 0.1) / 0.6))
        }
        MET => {
            let decay = (-GAMMA * t).exp();
            let (x0, y0) = (c.start[0], c.start[1]);
            let centre = [
                x0 * decay * (OMEGA * t).cos_det(),
                c.slot + (y0 - c.slot) * decay,
            ];
            at(centre, c.beta * (-1.3 * GAMMA * t).exp(), ALPHA_MET, 1.0)
        }
        ANA => {
            // Anaphase A at constant speed, then waiting at the pole.
            let travel = (t / (ANA_MOVE * DURATIONS[ANA])).min(1.0);
            let x = SISTER + (E_ANA + NUC_SHIFT - SISTER) * travel;
            let y = c.slot * (1.0 - 0.25 * ease::smoothstep(p));
            let alpha = ease::lerp(ALPHA_MET, ALPHA_ANA, ease::smoothstep(travel / 0.35));
            Pose {
                kin: [side * x, y],
                beta: 0.0,
                alpha,
                shown: 1.0,
            }
        }
        TEL => {
            let sh = shape(TEL, p);
            Pose {
                kin: [side * (sh.e + NUC_SHIFT), c.slot * 0.75],
                beta: 0.0,
                alpha: ALPHA_ANA,
                shown: 1.0 - ease::smoothstep(p / 0.85),
            }
        }
        _ => {
            let sh = shape(CYT, p);
            Pose {
                kin: [side * (sh.e + NUC_SHIFT), c.slot * 0.75],
                beta: 0.0,
                alpha: ALPHA_ANA,
                shown: 0.0,
            }
        }
    }
}

/// Deal a loop's chromosomes: scattered in the nucleus with random orientations, each
/// given the plate slot that keeps their paths from crossing (sorted by height).
pub(crate) fn deal(rng: &mut impl Rng) -> [Chromosome; CHROMOSOMES] {
    let mut out = [Chromosome::default(); CHROMOSOMES];
    for k in 0..CHROMOSOMES {
        let mut best = [0.0, 0.0];
        for _ in 0..40 {
            let (r, th) = (
                SCATTER * rng.random_range(0.0f64..1.0).sqrt(),
                rng.random_range(0.0..std::f64::consts::TAU),
            );
            let (s, c) = th.sin_cos_det();
            best = [r * c, r * s];
            let clear = out[..k]
                .iter()
                .all(|o| (o.start[0] - best[0]).hypot(o.start[1] - best[1]) > 0.17);
            if clear {
                break;
            }
        }
        out[k] = Chromosome {
            pair: (k / 2) as u8,
            maternal: k % 2 == 0,
            start: best,
            beta: rng.random_range(-1.4..1.4),
            slot: 0.0,
        };
    }
    let mut order: [usize; CHROMOSOMES] = std::array::from_fn(|k| k);
    order.sort_by(|&i, &j| out[i].start[1].total_cmp(&out[j].start[1]));
    for (rank, &k) in order.iter().enumerate() {
        out[k].slot = -PLATE_SPAN + 2.0 * PLATE_SPAN * rank as f64 / (CHROMOSOMES - 1) as f64;
    }
    out
}

// ---- Look (SPEC-002 Look) ----

const CHROMATID_LEVEL: u8 = 255;
/// Atom σ (dots) and peak, with the centre dot always lit: a small solid core.
const ATOM_SIGMA: f64 = 0.6;
const ATOM_PEAK: f64 = 1.6;
const CENTROSOME_SIGMA: f64 = 0.7;
const CENTROSOME_PEAK: f64 = 1.6;
const MEMBRANE_INK: Ink = Ink::new(Role::Structure, 220);
const ENVELOPE_INK: Ink = Ink::new(Role::Dim, 150);
const SPINDLE_INK: Ink = Ink::new(Role::Dim, 120);
const ASTER_INK: Ink = Ink::new(Role::Accent, 120);
const CENTROSOME_INK: Ink = Ink::new(Role::Accent, 255);
const HAZE_LEVEL: u8 = 150;
/// Chromatin haze: peak stipple density, kept sparse (fixed per-dot thresholds).
const HAZE_PEAK: f64 = 0.2;
/// Membrane node grid: nodes per cell-unit at most, and the cap per axis.
const NODES_PER_UNIT: f64 = 40.0;
const GRID_W: usize = 200;
const GRID_H: usize = 120;
/// Largest cell radius in dots: a small subject with much black around it.
const MAX_SCALE: f64 = 110.0;
const HAZE_SALT: u64 = 0x6D17_0515_0000_0001;
const TINT_SALT: u64 = 0x6D17_0515_0000_0002;
const ATOM_SALT: u64 = 0x6D17_0515_0000_0003;
const ENVELOPE_SALT: u64 = 0x6D17_0515_0000_0004;
const NOISE_SALT: u64 = 0x6D17_0515_0000_0005;

fn parent_role(c: &Chromosome) -> Role {
    if c.maternal { Role::Hot } else { Role::Cold }
}

/// The fixed threshold an atom's condensation must pass for it to show: centromeres
/// first, arm tips last.
fn atom_threshold(k: usize, side: f64, j: u8, branch: f64) -> f64 {
    if j == 0 {
        return 0.02;
    }
    let key = (k as u64) << 24
        ^ u64::from(side > 0.0) << 16
        ^ u64::from(j) << 8
        ^ u64::from(branch > 0.0);
    0.1 + 0.6 * f64::from(j) / 4.0 + 0.2 * hash01(ATOM_SALT ^ key)
}

#[derive(Debug)]
struct Mitosis {
    time: f64,
    cycle: i64,
    timeline: Timeline,
    chromo: [Chromosome; CHROMOSOMES],
    dots: Dots,
    size: Size,
    /// Dots per cell unit and the view centre (dots).
    scale: f64,
    centre: (f64, f64),
    /// Membrane SDF nodes, its size, origin and spacing (cell units), and the shape it holds.
    grid: Vec<f32>,
    gw: usize,
    gh: usize,
    origin: (f64, f64),
    step: f64,
    grid_of: Option<Shape>,
    started: bool,
    dirty: bool,
}

impl Default for Mitosis {
    fn default() -> Self {
        let phases: [(&'static str, f64); 5] = std::array::from_fn(|k| (CAPTIONS[k], DURATIONS[k]));
        Self {
            time: 0.0,
            cycle: 0,
            timeline: Timeline::new(&phases),
            chromo: [Chromosome::default(); CHROMOSOMES],
            dots: Dots::default(),
            size: Size::new(1, 1),
            scale: 1.0,
            centre: (0.0, 0.0),
            grid: Vec::with_capacity(GRID_W * GRID_H),
            gw: 0,
            gh: 0,
            origin: (0.0, 0.0),
            step: 1.0,
            grid_of: None,
            started: false,
            dirty: true,
        }
    }
}

impl Mitosis {
    /// Cell units to dots under the view.
    fn to_dot(&self, sh: &Shape, p: [f64; 2]) -> (f64, f64) {
        let (fx, zoom) = view(sh);
        let k = self.scale * zoom;
        (self.centre.0 + k * (p[0] - fx), self.centre.1 + k * p[1])
    }

    /// Level step for things on the right daughter while the view leaves it: full, half,
    /// then gone.
    fn right_fade(sh: &Shape, side: f64) -> Option<u8> {
        // The right daughter dissolves as soon as the view starts to leave it, before it
        // reaches the screen's edge.
        if side < 0.0 || sh.z < 0.08 {
            Some(255)
        } else if sh.z < 0.16 {
            Some(128)
        } else {
            None
        }
    }

    fn scaled(ink: Ink, fade: u8) -> Ink {
        Ink::new(
            ink.role,
            ((u16::from(ink.level) * u16::from(fade)) / 255) as u8,
        )
    }

    /// Refill the membrane's node grid when the shape changed.
    fn fill_grid(&mut self, sh: &Shape) {
        if self.grid_of == Some(*sh) {
            return;
        }
        self.grid_of = Some(*sh);
        let half_w = sh.e + sh.a + 0.3;
        let half_h = sh.a + 0.3;
        let step = (1.0 / NODES_PER_UNIT)
            .max(2.0 * half_w / (GRID_W - 1) as f64)
            .max(2.0 * half_h / (GRID_H - 1) as f64);
        self.gw = ((2.0 * half_w / step).ceil() as usize + 1).min(GRID_W);
        self.gh = ((2.0 * half_h / step).ceil() as usize + 1).min(GRID_H);
        self.origin = (-half_w, -half_h);
        self.step = step;
        self.grid.clear();
        for j in 0..self.gh {
            let y = -half_h + j as f64 * step;
            for i in 0..self.gw {
                let x = -half_w + i as f64 * step;
                self.grid.push(membrane(sh, x, y) as f32);
            }
        }
    }

    fn draw_membrane(&mut self, sh: &Shape) {
        self.fill_grid(sh);
        let (fx, zoom) = view(sh);
        let (k, c, o, st) = (self.scale * zoom, self.centre, self.origin, self.step);
        let to_dot = |i: f64, j: f64| (c.0 + k * (o.0 + i * st - fx), c.1 + k * (o.1 + j * st));
        let (gw, gh) = (self.gw, self.gh);
        let mid = (gw as f64 - 1.0) / 2.0;
        let shz = *sh;
        let ink = |i: usize, _j: usize| {
            let side = if i as f64 > mid { 1.0 } else { -1.0 };
            Self::right_fade(&shz, side).map(|f| Self::scaled(MEMBRANE_INK, f))
        };
        self.dots.contour_by(&self.grid, gw, gh, 0.0, to_dot, ink);
    }

    /// A dotted envelope circle: every third dot of the ring, each kept while `shown`
    /// passes its fixed threshold.
    fn draw_envelope(&mut self, sh: &Shape, centre: [f64; 2], r: f64, shown: f64, ink: Ink) {
        if shown <= 0.0 {
            return;
        }
        let (cx, cy) = self.to_dot(sh, centre);
        let rd = r * self.scale * view(sh).1;
        let n = ((std::f64::consts::TAU * rd / 3.0).floor() as usize).clamp(6, 600);
        for k in 0..n {
            if hash01(ENVELOPE_SALT ^ k as u64) >= shown {
                continue;
            }
            let (s, c) = (std::f64::consts::TAU * k as f64 / n as f64).sin_cos_det();
            self.dots.plot(cx + rd * c, cy + rd * s, ink);
        }
    }

    /// Chromatin: sparse two-tone stipple inside a nucleus, its density a slowly drifting
    /// noise; every dot keeps a fixed threshold, so the haze never shimmers.
    fn draw_haze(&mut self, sh: &Shape, centre: [f64; 2], r: f64, amount: f64, fade: u8) {
        if amount <= 0.0 {
            return;
        }
        let (cx, cy) = self.to_dot(sh, centre);
        let rd = r * self.scale * view(sh).1 * 0.92;
        let size = self.dots.dot_size();
        let (x0, x1) = (
            (cx - rd).ceil().max(0.0),
            (cx + rd).floor().min(f64::from(size.width()) - 1.0),
        );
        let (y0, y1) = (
            (cy - rd).ceil().max(0.0),
            (cy + rd).floor().min(f64::from(size.height()) - 1.0),
        );
        if x0 > x1 || y0 > y1 {
            return;
        }
        let peak = HAZE_PEAK * amount;
        let drift = 0.12 * self.time;
        let level = ((u16::from(HAZE_LEVEL) * u16::from(fade)) / 255) as u8;
        for y in y0 as i32..=y1 as i32 {
            let v = (f64::from(y) - cy) / rd;
            for x in x0 as i32..=x1 as i32 {
                let u = (f64::from(x) - cx) / rd;
                if u * u + v * v > 1.0 {
                    continue;
                }
                let key = cell_key(x, y);
                let th = hash01(HAZE_SALT ^ key);
                if th >= peak {
                    continue;
                }
                let n = value_noise(NOISE_SALT, 2.6 * u + drift, 2.6 * v - 0.7 * drift, 1.0);
                if th < peak * ease::smoothstep((n - 0.35) / 0.4) {
                    let role = if hash01(TINT_SALT ^ key) < 0.5 {
                        Role::Hot
                    } else {
                        Role::Cold
                    };
                    self.dots.set(x, y, Ink::new(role, level));
                }
            }
        }
    }

    /// Atoms per arm at this scale: as many as the pair has, but no more than keeps
    /// neighbouring chromosomes on the metaphase plate apart.
    fn arm_atoms(&self, pair: u8, spacing: f64, zoom: f64) -> u8 {
        let gap = 2.0 * PLATE_SPAN / (CHROMOSOMES - 1) as f64 * self.scale * zoom;
        let fit = ((gap - 2.5) / (2.0 * spacing * ALPHA_MET.sin_det()))
            .floor()
            .max(1.0);
        ARMS[usize::from(pair) % ARMS.len()].min(fit as u8)
    }

    /// Dot spacing of a chromatid's atoms, from the cell's size.
    fn atom_spacing(&self, zoom: f64) -> f64 {
        (0.03 * self.scale * zoom).clamp(1.8, 2.6)
    }

    /// The x offset (dots) that staggers alternate chromosomes on the metaphase plate, so
    /// neighbours' arms interleave instead of running together.
    fn stagger(&self, k: usize, act: usize, p: f64, t: f64) -> f64 {
        let rank = ((self.chromo[k].slot + PLATE_SPAN) / (2.0 * PLATE_SPAN)
            * (CHROMOSOMES - 1) as f64)
            .round() as i64;
        let sign = if rank % 2 == 0 { -1.0 } else { 1.0 };
        let w = match act {
            MET => 1.0 - (-GAMMA * t).exp(),
            ANA => 1.0 - ease::smoothstep(p / 0.3),
            _ => 0.0,
        };
        sign * STAGGER * w
    }

    /// The visible atoms of chromosome `k`'s chromatid on `side` (dots), centromere first,
    /// with their stipple salts; returns how many.
    fn chromatid_atoms_salted(
        &self,
        sh: &Shape,
        k: usize,
        side: f64,
        p: &Pose,
        out: &mut [(f64, f64); MAX_ATOMS],
        salts: &mut [u64; MAX_ATOMS],
    ) -> usize {
        let c = self.chromo[k];
        let beat = self.timeline.at(self.time);
        let mut kin = self.to_dot(sh, p.kin);
        kin.0 += self.stagger(k, beat.index, beat.progress, beat.local);
        let zoom = view(sh).1;
        let spacing = self.atom_spacing(zoom);
        let (sb, cb) = p.beta.sin_cos_det();
        let (axis, normal) = ((cb, sb), (-sb, cb));
        let (sa, ca) = p.alpha.sin_cos_det();
        let salt = ATOM_SALT ^ (k as u64) << 8 ^ u64::from(side > 0.0);
        let mut n = 0;
        if p.shown > atom_threshold(k, side, 0, 0.0) {
            out[n] = kin;
            salts[n] = salt;
            n += 1;
        }
        let arm = self.arm_atoms(c.pair, spacing, zoom);
        for branch in [-1.0, 1.0] {
            let dir = (
                side * ca * axis.0 + branch * sa * normal.0,
                side * ca * axis.1 + branch * sa * normal.1,
            );
            for j in 1..=arm {
                if p.shown <= atom_threshold(k, side, j, branch) || n >= MAX_ATOMS {
                    continue;
                }
                let d = spacing * f64::from(j);
                out[n] = (kin.0 + d * dir.0, kin.1 + d * dir.1);
                salts[n] = salt ^ u64::from(j) << 4 ^ u64::from(branch > 0.0) << 12;
                n += 1;
            }
        }
        n
    }

    #[cfg(test)]
    fn chromatid_atoms(
        &self,
        sh: &Shape,
        k: usize,
        side: f64,
        p: &Pose,
        out: &mut [(f64, f64); MAX_ATOMS],
    ) -> usize {
        self.chromatid_atoms_salted(sh, k, side, p, out, &mut [0; MAX_ATOMS])
    }

    fn draw_chromatid(&mut self, sh: &Shape, k: usize, side: f64, p: &Pose, fade: u8) {
        let ink = Ink::new(
            parent_role(&self.chromo[k]),
            ((u16::from(CHROMATID_LEVEL) * u16::from(fade)) / 255) as u8,
        );
        let mut at = [(0.0, 0.0); MAX_ATOMS];
        let mut salts = [0u64; MAX_ATOMS];
        let n = self.chromatid_atoms_salted(sh, k, side, p, &mut at, &mut salts);
        for (a, &salt) in at[..n].iter().zip(&salts[..n]) {
            self.dots
                .stipple_puff(a.0, a.1, ATOM_SIGMA, ATOM_PEAK, salt, ink);
        }
    }

    /// A thread from `a` to `b` (dots), stopped `gap` dots short of each end.
    fn thread(&mut self, a: (f64, f64), b: (f64, f64), gap: (f64, f64), ink: Ink) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        if len.is_nan() || len <= gap.0 + gap.1 + 0.5 {
            return;
        }
        let (fa, fb) = (gap.0 / len, gap.1 / len);
        self.dots.line(
            a.0 + fa * dx,
            a.1 + fa * dy,
            b.0 - fb * dx,
            b.1 - fb * dy,
            ink,
        );
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        let beat = self.timeline.at(self.time);
        let (act, p, t) = (beat.index, beat.progress, beat.local);
        let sh = shape(act, p);
        let zoom = view(&sh).1;
        self.draw_membrane(&sh);

        // Nuclei: the interphase/prophase nucleus, then the two daughters'.
        match act {
            PRO => {
                self.draw_envelope(&sh, [0.0, 0.0], NUC, 1.0, ENVELOPE_INK);
                let amount = 1.0 - ease::smoothstep((p - 0.05) / 0.75);
                self.draw_haze(&sh, [0.0, 0.0], NUC, amount, 255);
            }
            MET => {
                // The envelope breaks down early in (pro)metaphase.
                self.draw_envelope(&sh, [0.0, 0.0], NUC, 1.0 - p / 0.25, ENVELOPE_INK);
            }
            TEL | CYT => {
                let (shown, amount) = if act == TEL {
                    (
                        ease::smoothstep((p - 0.3) / 0.5),
                        ease::smoothstep((p - 0.2) / 0.7),
                    )
                } else {
                    (1.0, 1.0)
                };
                for side in [-1.0, 1.0] {
                    let Some(fade) = Self::right_fade(&sh, side) else {
                        continue;
                    };
                    let (c, r) = daughter_nucleus(&sh, side);
                    self.draw_envelope(&sh, c, r, shown, Self::scaled(ENVELOPE_INK, fade));
                    self.draw_haze(&sh, c, r, amount, fade);
                }
            }
            _ => {}
        }

        // Spindle: kinetochore microtubules (metaphase on, as they capture the
        // chromosomes), polar microtubules, astral rays.
        let poles = [centrosome(act, p, -1.0), centrosome(act, p, 1.0)];
        let pole_dots = [self.to_dot(&sh, poles[0]), self.to_dot(&sh, poles[1])];
        let spindle = matches!(act, MET | ANA) || (act == TEL && p < 0.5);
        if spindle {
            for k in 0..CHROMOSOMES {
                let c = self.chromo[k];
                // Capture: each chromosome is attached at its own moment early in metaphase.
                if act == MET && p < 0.08 + 0.2 * hash01(ATOM_SALT ^ 0xCA97 ^ k as u64) {
                    continue;
                }
                for (s, side) in [-1.0, 1.0].into_iter().enumerate() {
                    let ps = pose(&c, side, act, p, t);
                    let kin = self.to_dot(&sh, ps.kin);
                    self.thread(pole_dots[s], kin, (2.5, 1.5), SPINDLE_INK);
                }
            }
            // Polar microtubules overlap across the equator and push the poles apart.
            for (s, side) in [-1.0f64, 1.0].into_iter().enumerate() {
                for off in [-0.07, 0.07] {
                    let end = self.to_dot(&sh, [-side * 0.12, off + side * 0.02]);
                    self.thread(pole_dots[s], end, (2.5, 0.0), SPINDLE_INK);
                }
            }
        }
        for (s, side) in [-1.0f64, 1.0].into_iter().enumerate() {
            let Some(fade) = Self::right_fade(&sh, side) else {
                continue;
            };
            let at = pole_dots[s];
            // Astral rays point away from the cell's centre (or from the spindle).
            let out = if act == PRO {
                poles[s][1].atan2(poles[s][0])
            } else if side < 0.0 {
                std::f64::consts::PI
            } else {
                0.0
            };
            let len = 0.13 * self.scale * zoom;
            for r in 0..5 {
                let th = out + (f64::from(r) - 2.0) * 0.55;
                let (sn, cs) = th.sin_cos_det();
                self.thread(
                    at,
                    (at.0 + len * cs, at.1 + len * sn),
                    (2.0, 0.0),
                    Self::scaled(ASTER_INK, fade),
                );
            }
        }

        // Chromatids over the threads.
        if matches!(act, PRO | MET | ANA | TEL) {
            for k in 0..CHROMOSOMES {
                for side in [-1.0, 1.0] {
                    let ps = pose(&self.chromo[k], side, act, p, t);
                    if ps.shown > 0.0 {
                        self.draw_chromatid(&sh, k, side, &ps, 255);
                    }
                }
            }
        }
        for (s, side) in [-1.0f64, 1.0].into_iter().enumerate() {
            let Some(fade) = Self::right_fade(&sh, side) else {
                continue;
            };
            let at = pole_dots[s];
            self.dots.stipple_puff(
                at.0,
                at.1,
                CENTROSOME_SIGMA,
                CENTROSOME_PEAK,
                ATOM_SALT ^ 0xCE47 ^ s as u64,
                Self::scaled(CENTROSOME_INK, fade),
            );
        }
    }
}

impl Generator for Mitosis {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.time).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let dw = f64::from(size.width()) * f64::from(Dots::CELL_W);
        let dh = f64::from(size.height().saturating_sub(1).max(1)) * f64::from(Dots::CELL_H);
        self.scale = (dw / 2.9).min(dh / 2.6).clamp(0.5, MAX_SCALE);
        self.centre = (dw / 2.0 - 0.5, dh / 2.0 - 0.5);
        if !self.started {
            self.started = true;
            self.chromo = deal(&mut ctx.rng);
            self.cycle = self.timeline.cycle(self.time);
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.time += dt.min(0.25);
        let cycle = self.timeline.cycle(self.time);
        if cycle != self.cycle {
            self.cycle = cycle;
            self.chromo = deal(&mut ctx.rng);
        }
        self.dirty = true;
    }

    /// Rasterise once per drawn frame (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(seed: u64, size: Size) -> (Mitosis, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = Mitosis::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn run_to(g: &mut Mitosis, ctx: &mut Ctx, t: f64) {
        while g.time < t {
            g.update(1.0 / 60.0, ctx);
        }
    }

    #[test]
    fn mit_a1_captions_follow_the_phases_and_loop() {
        let (mut g, mut ctx) = started(1, Size::new(80, 24));
        let mut seen: Vec<&str> = Vec::new();
        let total: f64 = DURATIONS.iter().sum();
        while g.time < 2.0 * total + 1.0 {
            let c = g.caption().expect("a caption");
            if seen.last() != Some(&c) {
                seen.push(c);
            }
            g.update(0.1, &mut ctx);
        }
        let mut want: Vec<&str> = CAPTIONS.to_vec();
        want.extend(CAPTIONS);
        want.push(CAPTIONS[0]);
        assert_eq!(seen, want);
    }

    #[test]
    fn mit_a2_kinetochores_lie_on_the_plate_at_the_end_of_metaphase() {
        for seed in 0..20 {
            let chromo = deal(&mut SplitMix64::new(seed));
            let t = DURATIONS[MET] * 0.999;
            for c in &chromo {
                for side in [-1.0, 1.0] {
                    let p = pose(c, side, MET, 0.999, t);
                    assert!(
                        p.kin[0].abs() <= PLATE_BAND,
                        "seed {seed}: kinetochore at x {:.3}",
                        p.kin[0]
                    );
                }
            }
            // And they started well off it.
            let off = chromo
                .iter()
                .map(|c| pose(c, -1.0, MET, 0.0, 0.0).kin[0].abs())
                .fold(0.0, f64::max);
            assert!(off > PLATE_BAND, "seed {seed}: already aligned");
        }
    }

    #[test]
    fn mit_a3_sisters_part_monotonically_and_each_daughter_gets_one_of_each() {
        let chromo = deal(&mut SplitMix64::new(3));
        let n = 200;
        for c in &chromo {
            let mut last = 0.0;
            for k in 0..n {
                let p = f64::from(k) / f64::from(n);
                let t = p * DURATIONS[ANA];
                let (l, r) = (pose(c, -1.0, ANA, p, t), pose(c, 1.0, ANA, p, t));
                let d = (r.kin[0] - l.kin[0]).hypot(r.kin[1] - l.kin[1]);
                assert!(d >= last - 1e-12, "distance fell {last} → {d}");
                last = d;
            }
            assert!(last > 0.8, "sisters ended {last:.2} apart");
        }
        // Daughters by the side of the cell each chromatid ends anaphase on.
        for daughter in [-1.0f64, 1.0] {
            for pair in 0..4u8 {
                let mut hot = 0;
                let mut cold = 0;
                for c in chromo.iter().filter(|c| c.pair == pair) {
                    for side in [-1.0, 1.0] {
                        let x = pose(c, side, ANA, 0.999, DURATIONS[ANA] * 0.999).kin[0];
                        if x.signum() == daughter {
                            if c.maternal {
                                hot += 1;
                            } else {
                                cold += 1;
                            }
                        }
                    }
                }
                assert_eq!((hot, cold), (1, 1), "daughter {daughter}, pair {pair}");
            }
        }
    }

    #[test]
    fn mit_a4_the_membrane_is_closed_and_its_waist_pinches_to_zero() {
        // The waist only narrows through anaphase … cytokinesis and is zero at the end.
        let mut last = f64::INFINITY;
        for act in [ANA, TEL, CYT] {
            for k in 0..=100 {
                let p = f64::from(k) / 100.0 * 0.999;
                let sh = shape(act, p);
                let w = waist(&sh);
                assert!(
                    w <= last + 1e-12,
                    "{} p {p:.2}: waist {last} → {w}",
                    CAPTIONS[act]
                );
                last = w;
                // The formula matches the field: the equator is outside beyond the waist
                // and inside within it.
                assert!(membrane(&sh, 0.0, w + 1e-3) > 0.0);
                if w > 1e-3 {
                    assert!(membrane(&sh, 0.0, w - 1e-3) < 0.0);
                } else {
                    for j in 0..50 {
                        assert!(
                            membrane(&sh, 0.0, f64::from(j) * 0.02) > 0.0,
                            "pinched shut"
                        );
                    }
                }
            }
        }
        assert_eq!(waist(&shape(CYT, 0.999)), 0.0);
        assert!(waist(&shape(PRO, 0.5)) > 0.9);
        // Closed: the drawn node grid is outside the cell all round its border, at every
        // phase, so every contour piece is part of a closed loop.
        let (mut g, mut ctx) = started(4, Size::new(120, 40));
        let total: f64 = DURATIONS.iter().sum();
        while g.time < total {
            g.update(0.2, &mut ctx);
            g.prepare();
            let (w, h) = (g.gw, g.gh);
            for i in 0..w {
                assert!(g.grid[i] > 0.0 && g.grid[(h - 1) * w + i] > 0.0);
            }
            for j in 0..h {
                assert!(g.grid[j * w] > 0.0 && g.grid[j * w + w - 1] > 0.0);
            }
            assert!(g.grid.iter().any(|&v| v < 0.0), "a cell is drawn");
        }
        // The loop closes: the zoomed daughter is the next loop's cell (radius 1).
        let end = shape(CYT, 0.9999);
        let (fx, zoom) = view(&end);
        let left_edge = (-end.e - end.a - fx) * zoom;
        assert!(
            (left_edge + 1.0).abs() < 1e-3,
            "daughter radius {left_edge}"
        );
        let pole = centrosome(CYT, 0.9999, -1.0);
        assert!(((pole[0] - fx) * zoom + POLE).abs() < 1e-3);
        assert!((centrosome(PRO, 0.0, -1.0)[0] + POLE).abs() < 1e-9);
    }

    #[test]
    fn mit_a5_chromatids_are_small_atoms_and_sparse() {
        // At metaphase, measure the lit share of each chromatid's footprint (the dots
        // within its atoms' reach) on an otherwise empty lattice.
        let (mut g, mut ctx) = started(5, Size::new(200, 50));
        run_to(&mut g, &mut ctx, DURATIONS[PRO] + 5.0);
        let beat = g.timeline.at(g.time);
        assert_eq!(beat.index, MET);
        let sh = shape(MET, beat.progress);
        for k in 0..CHROMOSOMES {
            g.dots.clear();
            let ps = pose(&g.chromo[k], -1.0, MET, beat.progress, beat.local);
            g.draw_chromatid(&sh, k, -1.0, &ps, 255);
            let size = g.dots.dot_size();
            let (mut lit, mut foot) = (0, 0);
            let kin = g.to_dot(&sh, ps.kin);
            let reach = 3.0 * 3.0 + 3.0 * ATOM_SIGMA;
            for y in 0..i32::from(size.height()) {
                for x in 0..i32::from(size.width()) {
                    if (f64::from(x) - kin.0).hypot(f64::from(y) - kin.1) <= reach {
                        foot += 1;
                        lit += i32::from(g.dots.is_lit(x, y));
                    }
                }
            }
            let share = f64::from(lit) / f64::from(foot);
            assert!(
                lit >= 5 && share <= 0.5,
                "chromosome {k}: {lit} lit, share {share:.2}"
            );
        }
        // Peak stipple density of an atom's halo (beyond its solid centre dot).
        assert!(ATOM_PEAK * (-0.5 / (ATOM_SIGMA * ATOM_SIGMA)).exp() <= 0.5);
        const { assert!(HAZE_PEAK <= 0.5) };
    }

    #[test]
    fn mit_braille_only_and_inks() {
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = started(6, size);
        for t in [1.0, 5.0, 10.0, 16.0, 20.0, 24.0, 27.0, 29.5] {
            run_to(&mut g, &mut ctx, t);
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let mut lit = 0;
            for c in canvas.cells() {
                if c.ch == ' ' {
                    continue;
                }
                lit += 1;
                assert!(c.bg == Ink::BLANK && !matches!(c.ch, '▀' | '▄' | '█'));
                assert!(
                    matches!(
                        c.ink.role,
                        Role::Hot | Role::Cold | Role::Accent | Role::Dim | Role::Structure
                    ),
                    "t {t}: {:?}",
                    c.ink
                );
            }
            assert!(lit > 50, "t {t}: {lit} lit cells");
        }
    }

    #[test]
    fn mit_metaphase_chromosomes_stay_apart() {
        // Review round 1: arms lie near the plate and no two chromosomes run together.
        assert!((0.85..=1.0).contains(&ALPHA_MET), "arm angle {ALPHA_MET}");
        for size in [Size::new(80, 24), Size::new(120, 40), Size::new(200, 50)] {
            for seed in 0..4 {
                let (mut g, mut ctx) = started(seed, size);
                run_to(&mut g, &mut ctx, DURATIONS[PRO] + DURATIONS[MET] - 0.05);
                let beat = g.timeline.at(g.time);
                assert_eq!(beat.index, MET);
                let sh = shape(MET, beat.progress);
                let mut atoms: Vec<(usize, (f64, f64))> = Vec::new();
                let mut buf = [(0.0, 0.0); MAX_ATOMS];
                for k in 0..CHROMOSOMES {
                    for side in [-1.0, 1.0] {
                        let ps = pose(&g.chromo[k], side, MET, beat.progress, beat.local);
                        let n = g.chromatid_atoms(&sh, k, side, &ps, &mut buf);
                        atoms.extend(buf[..n].iter().map(|&a| (k, a)));
                    }
                }
                for (i, &(ka, a)) in atoms.iter().enumerate() {
                    for &(kb, b) in &atoms[i + 1..] {
                        if ka != kb {
                            let d = (a.0 - b.0).hypot(a.1 - b.1);
                            assert!(
                                d >= 2.0,
                                "{size:?} seed {seed}: chromosomes {ka} and {kb} {d:.2} dots apart"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mit_daughters_keep_a_margin() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = started(9, size);
        let start: f64 = DURATIONS[..CYT].iter().sum();
        run_to(&mut g, &mut ctx, start);
        let dw = i32::from(g.dots.dot_size().width());
        let dh = i32::from(g.dots.dot_size().height());
        while g.time < start + DURATIONS[CYT] - 0.05 {
            g.update(0.1, &mut ctx);
            g.prepare();
            for y in 0..dh {
                for x in (0..8).chain(dw - 8..dw) {
                    assert!(
                        !g.dots.is_lit(x, y),
                        "t {:.2}: lit at column {}",
                        g.time,
                        x / 2
                    );
                }
            }
        }
    }
}
