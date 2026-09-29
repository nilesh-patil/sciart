//! Fibrillation: a heart-shaped sheet of cardiac tissue whose regular beats break into a
//! re-entrant spiral, then into many (ventricular fibrillation), until a shock resets it
//! (SPEC-002 `heart_fibrillation`).
//!
//! ## Model
//!
//! Aliev–Panfilov tissue (`shared::excitable`) on a heart-shaped mask with no-flux walls,
//! explicit Euler at `dt = 0.05` time units, 600 sub-steps per simulated second (30 tu/s,
//! so a spiral turns about once a second). The node spacing is set so the heart is
//! always about [`HEART_SU`] space units tall, capped at [`H_MAX`] where the lattice gets
//! coarse (small terminals hold fewer spirals, never broken fronts).
//!
//! The story is a looping [`Timeline`] on the integer sub-step clock:
//!
//! - **sinus rhythm**: the sinoatrial pacemaker (upper left lobe) fires every 1.2 s;
//! - **re-entry**: the S1–S2 cross-field protocol. S1 excites the heart's left edge (a
//!   plane wave travelling right); S2 excites the recovered lower half once S1's
//!   refractory tail has crossed the middle row, so the S2 wave can only climb, and its
//!   free end curls round the recovering tail into a spiral;
//! - **fibrillation**: patchy refractoriness (smooth random patches where `ε` is lower or
//!   higher, as in ischaemic tissue) breaks the spiral's wavefront wherever it meets a
//!   patch still refractory, and every break curls into a new spiral;
//! - **defibrillation**: a shock excites every cell at once from a common state, the
//!   heart recovers as one, and the pacemaker takes over again.
//!
//! The display is the standard phase map `θ = atan2(v − v*, u − u*)` (cyclic hues), with
//! resting tissue dim and phase singularities (spiral tips) as white dots. A pseudo-ECG,
//! `Φ = Σ ∇u·∇(1/r)` at an electrode below the apex, sweeps along the bottom rows.

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::grid::{self, SampleMap};
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::ASCII_RAMP;
use sciart_core::{draw, title};

use crate::shared::excitable::{self, AlievPanfilov, Medium};

pub static META: Meta = Meta {
    name: "heart_fibrillation",
    category: Category::Biophysics,
    description: "Heart tissue as a phase map: sinus beats, a re-entrant spiral, breakup into fibrillation, a defibrillator shock",
    title: "Fibrillation · spirals break up",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 4] = ["sinus rhythm", "re-entry", "fibrillation", "defibrillation"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Heart::default())
}

// ---- Model (SPEC-002 Model) ----

/// Aliev–Panfilov with the standard `k = 8`, `μ₁ = 0.2`, `μ₂ = 0.3`, `ε₀ = 0.002`, a lower
/// threshold `a = 0.1` and `D = 0.5`; `h` is set per lattice in `resize`.
const MODEL: AlievPanfilov = AlievPanfilov {
    k: 8.0,
    a: 0.1,
    eps0: 0.002,
    mu1: 0.2,
    mu2: 0.3,
    d: 0.5,
    h: 1.0,
    dt: 0.05,
};
/// Sub-steps per simulated second (40 tu/s).
const SUB_PER_S: u64 = 800;
/// Height of the heart in space units.
const HEART_SU: f32 = 120.0;
/// Coarsest node spacing: beyond it fronts are too thin in nodes to propagate cleanly.
const H_MAX: f32 = 1.6;
const _: () = assert!(AlievPanfilov { h: 1.0, ..MODEL }.is_stable());

/// Phase-map centre `(u*, v*)` inside the excitation loop.
const CENTRE: (f32, f32) = (0.5, 0.5);
/// A node counts as recovered (excitable again) below this `v`.
const V_REC: f32 = 0.3;
/// Heterogeneous `ε` factors in the fibrillation act: patches of long (×0.2) and short
/// (×2) refractoriness.
const EPS_LO: f32 = 0.2;
const EPS_HI: f32 = 2.0;
/// Refractory patches per 1000 heart nodes, and their radius in space units.
const PATCHES_PER_KNODE: f32 = 2.5;
const PATCH_SU: f32 = 9.0;

// ---- Story (SPEC-002 Motion) ----

const SINUS_S: f64 = 12.0;
const REENTRY_S: f64 = 10.0;
const VF_S: f64 = 14.0;
const SHOCK_S: f64 = 6.0;
#[cfg(test)]
const REENTRY: usize = 1;
#[cfg(test)]
const VF: usize = 2;
#[cfg(test)]
const SHOCK: usize = 3;
/// Sinus beats every 1.2 s.
const BEAT_STEPS: u64 = 1200;
/// S1 fires this long into the re-entry act (the last sinus beat has recovered by then).
const S1_STEPS: u64 = 1600;
/// The pacemaker resumes this long after the shock.
const RESUME_STEPS: u64 = 1600;
const SA_RADIUS: f64 = 3.0;

// ---- Display (SPEC-002 Inks, Resize) ----

const MAX_NODES: usize = 12_800;
const BUCKET: f64 = 1.25;
/// Tips tracked for display.
const MAX_TIPS: usize = 48;
/// Sub-steps between singularity counts.
const COUNT_EVERY: u64 = 60;
/// Resting tissue: nearly black, so the waves carry the picture; its rim outlines the
/// heart.
const REST_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 20);
const RIM_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 80);
/// ECG trace ink (a monitor's green) at its write head.
const ECG_RAMP: Ramp = Ramp::Aurora;
/// The ECG write head: a white-hot dot.
const ECG_HEAD_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 255);
const TIP_INK: Ink = Ink::new(Role::Ramp(Ramp::Starlight), 255);
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// ECG sweep speed in dot columns per second.
const SWEEP_DOTS_PER_S: f64 = 40.0;
/// Blank dot columns ahead of the ECG write head.
const SWEEP_GAP: usize = 6;

#[derive(Debug)]
struct Heart {
    medium: Medium,
    model: AlievPanfilov,
    u: Grid<f32>,
    v: Grid<f32>,
    lap: Vec<f32>,
    /// Per-node `ε` factor for the fibrillation act.
    patchy: Vec<f32>,
    /// Whether `patchy` is in force.
    heterogeneous: bool,
    theta: Vec<f32>,
    taken: Vec<bool>,
    tips: [(i32, i32); MAX_TIPS],
    tip_count: usize,
    /// Pseudo-ECG lead field `∇(1/r)` per node.
    lead: Vec<[f32; 2]>,
    /// ECG samples per dot column, the sweep's write head and its running scale.
    ecg: Vec<f32>,
    head: f64,
    ecg_scale: f32,
    ecg_base: f32,
    /// Sinoatrial node, lattice coordinates.
    sa: (f64, f64),
    /// Left edge column and middle row of the heart, for S1–S2.
    left: i32,
    mid_row: i32,
    s2_fired: bool,
    /// Sub-step of the last shock (display flash).
    shocked_at: Option<u64>,
    step: u64,
    owed: f64,
    timeline: Timeline,
    pixels: Pixels,
    dots: Dots,
    map: SampleMap,
    /// Pixel rectangle the lattice maps onto: origin and size.
    view: (i32, i32, Size),
    /// Per view pixel: the heart's coverage and its rim band's coverage (0..=255), 4×4
    /// supersampled once per resize, so the outline is antialiased (HRT-A6).
    edge: Vec<[u8; 2]>,
    ecg_rows: u16,
    cells: Size,
    readout_room: usize,
}

impl Default for Heart {
    fn default() -> Self {
        Self {
            medium: Medium::default(),
            model: MODEL,
            u: Grid::default(),
            v: Grid::default(),
            lap: Vec::new(),
            patchy: Vec::new(),
            heterogeneous: false,
            theta: Vec::new(),
            taken: Vec::new(),
            tips: [(0, 0); MAX_TIPS],
            tip_count: 0,
            lead: Vec::new(),
            ecg: Vec::new(),
            head: 0.0,
            ecg_scale: 1e-3,
            ecg_base: 0.0,
            sa: (0.0, 0.0),
            left: 0,
            mid_row: 0,
            s2_fired: false,
            shocked_at: None,
            step: 0,
            owed: 0.0,
            timeline: Timeline::new(&[
                (CAPTIONS[0], SINUS_S),
                (CAPTIONS[1], REENTRY_S),
                (CAPTIONS[2], VF_S),
                (CAPTIONS[3], SHOCK_S),
            ]),
            pixels: Pixels::default(),
            dots: Dots::default(),
            map: SampleMap::default(),
            view: (0, 0, Size::new(1, 1)),
            edge: Vec::new(),
            ecg_rows: 0,
            cells: Size::new(1, 1),
            readout_room: 0,
        }
    }
}

/// Whether `(x, y)` (y up, heart units) lies inside the heart curve
/// `(x² + y² − 1)³ − x²y³ ≤ 0`, which spans `x ∈ [−1.14, 1.14]`, `y ∈ [−1, 1.23]`.
pub(crate) fn in_heart(x: f32, y: f32) -> bool {
    let r = x * x + y * y - 1.0;
    r * r * r - x * x * y * y * y <= 0.0
}

/// Heart units per lattice node and the lattice position of the heart's origin, for a
/// `size` lattice: the curve's bounding box `[−1.14, 1.14] × [−1, 1.23]` fits inside with a
/// node to spare on each side.
fn heart_frame(size: Size) -> (f32, f32, f32) {
    let (w, h) = (f32::from(size.width()), f32::from(size.height()));
    let per_node = (2.28 / (w - 2.0).max(1.0)).max(2.23 / (h - 2.0).max(1.0));
    // Centre the box: x centre at 0, y centre at (1.23 − 1)/2.
    (per_node, w / 2.0, h / 2.0 + 0.115 / per_node)
}

/// A new Aliev–Panfilov state on `m`: everything at rest.
fn reset(u: &mut Grid<f32>, v: &mut Grid<f32>) {
    u.fill(0.0);
    v.fill(0.0);
}

impl Heart {
    fn caption_index(&self) -> usize {
        self.timeline.at(self.local_s()).index
    }

    fn local_s(&self) -> f64 {
        let cycle = (self.timeline.total() * SUB_PER_S as f64) as u64;
        (self.step % cycle) as f64 / SUB_PER_S as f64
    }

    fn cycle_steps(&self) -> u64 {
        (self.timeline.total() * SUB_PER_S as f64) as u64
    }

    /// Fire the sinoatrial pacemaker.
    fn sinus_beat(&mut self) {
        excitable::excite_disc(&mut self.u, &self.v, self.sa, SA_RADIUS, V_REC);
    }

    /// S1: excite the recovered nodes within 3 columns of the heart's left edge.
    fn s1(&mut self) {
        let w = i32::from(self.u.width());
        for &i in self.medium.inside() {
            let i = i as usize;
            let x = i as i32 % w;
            if x < self.left + 3 && self.v.as_slice()[i] < V_REC {
                self.u.as_mut_slice()[i] = 1.0;
            }
        }
    }

    /// Whether S1's refractory tail has crossed the middle of the middle row: the first
    /// refractory node on the row, from the left, lies right of the heart's centre.
    fn s2_due(&self) -> bool {
        let w = i32::from(self.u.width());
        let row = self.mid_row;
        let first = (self.left..w).find(|&x| {
            let i = (row * w + x) as usize;
            self.medium.contains(i) && self.v.as_slice()[i] >= V_REC
        });
        first.is_some_and(|x| x >= w / 2)
    }

    /// S2: excite every recovered node below the middle row.
    fn s2(&mut self) {
        let w = i32::from(self.u.width());
        for &i in self.medium.inside() {
            let i = i as usize;
            if i as i32 / w > self.mid_row && self.v.as_slice()[i] < V_REC {
                self.u.as_mut_slice()[i] = 1.0;
            }
        }
    }

    /// Defibrillation: every cell excited from the same state.
    fn shock(&mut self) {
        for &i in self.medium.inside() {
            self.u.as_mut_slice()[i as usize] = 1.0;
            self.v.as_mut_slice()[i as usize] = 0.0;
        }
        self.shocked_at = Some(self.step);
    }

    /// Events scheduled at cycle-local sub-step `local`.
    fn events(&mut self, local: u64) {
        let s = |secs: f64| (secs * SUB_PER_S as f64) as u64;
        let (reentry, vf, shock) = (
            s(SINUS_S),
            s(SINUS_S + REENTRY_S),
            s(SINUS_S + REENTRY_S + VF_S),
        );
        if local < reentry {
            if local % BEAT_STEPS == 0 {
                self.sinus_beat();
            }
        } else if local < vf {
            if local == reentry {
                self.s2_fired = false;
            }
            if local == reentry + S1_STEPS {
                self.s1();
            }
            if local > reentry + S1_STEPS && !self.s2_fired && self.s2_due() {
                self.s2_fired = true;
                self.s2();
            }
        } else if local < shock {
            self.heterogeneous = true;
        } else {
            if local == shock {
                self.heterogeneous = false;
                self.shock();
            }
            let since = local - shock;
            if since >= RESUME_STEPS && (since - RESUME_STEPS) % BEAT_STEPS == 0 {
                self.sinus_beat();
            }
        }
    }

    fn substep(&mut self) {
        let local = self.step % self.cycle_steps();
        self.events(local);
        let patchy = &self.patchy;
        let het = self.heterogeneous;
        self.model.step(
            &self.medium,
            self.u.as_mut_slice(),
            self.v.as_mut_slice(),
            &mut self.lap,
            |i| if het { patchy[i] } else { 1.0 },
        );
        self.step += 1;
        if self.step % COUNT_EVERY == 0 {
            self.count_tips();
        }
    }

    fn count_tips(&mut self) {
        self.tip_count = excitable::count_singularities(
            &self.medium,
            self.u.as_slice(),
            self.v.as_slice(),
            CENTRE,
            3,
            &mut self.theta,
            &mut self.taken,
            &mut self.tips,
        );
    }

    /// The pseudo-ECG `Φ = Σ ∇u·∇(1/r)` (central differences; walls give zero).
    fn ecg_sample(&self) -> f32 {
        let u = self.u.as_slice();
        self.medium
            .inside()
            .iter()
            .map(|&i| {
                let i = i as usize;
                let [w, e, n, s] = self.medium.neighbours(i).map(|j| u[j as usize]);
                let [lx, ly] = self.lead[i];
                0.5 * ((e - w) * lx + (s - n) * ly)
            })
            .sum()
    }

    /// Rebuild the heart lattice for the view, reset to rest (SPEC-002 Resize).
    fn rebuild(&mut self, view: Size, rng: &mut impl Rng) {
        let lat = grid::lattice_for(view, MAX_NODES, BUCKET);
        let (per_node, ox, oy) = heart_frame(lat);
        self.medium.rebuild(lat, |x, y| {
            in_heart(
                (x as f32 + 0.5 - ox) * per_node,
                (oy - y as f32 - 0.5) * per_node,
            )
        });
        let n = lat.area();
        self.u = Grid::new(lat, 0.0);
        self.v = Grid::new(lat, 0.0);
        for buf in [&mut self.lap, &mut self.theta] {
            buf.clear();
            buf.resize(n, 0.0);
        }
        self.taken.clear();
        self.taken.resize(n, false);
        // Node spacing: the heart (2.23 heart units tall) spans HEART_SU space units.
        let nodes_tall = 2.23 / per_node;
        self.model.h = (HEART_SU / nodes_tall).min(H_MAX);
        let (w, h) = (i32::from(lat.width()), i32::from(lat.height()));
        let to_lat =
            |hx: f32, hy: f32| (f64::from(ox + hx / per_node), f64::from(oy - hy / per_node));
        self.sa = to_lat(-0.62, 0.72);
        self.mid_row = to_lat(0.0, 0.1).1 as i32;
        self.left = (0..w)
            .find(|&x| self.medium.contains_xy(x, self.mid_row))
            .unwrap_or(0);
        // Lead field of a unipolar electrode below and right of the apex.
        let (ex, ey) = to_lat(0.9, -1.25);
        self.lead.clear();
        self.lead.extend(
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let (dx, dy) = ((f64::from(x) - ex) as f32, (f64::from(y) - ey) as f32);
                    let r2 = (dx * dx + dy * dy).max(1.0);
                    let r3 = r2 * r2.sqrt();
                    [-dx / r3, -dy / r3]
                }),
        );
        // Refractory patches for the fibrillation act: smooth bumps of ε factor, seeded.
        let inside = self.medium.inside().len() as f32;
        let n_patches = ((inside / 1000.0 * PATCHES_PER_KNODE).round() as usize).clamp(2, 64);
        let radius = PATCH_SU / self.model.h;
        self.patchy.clear();
        self.patchy.resize(n, 0.0);
        for _ in 0..n_patches {
            let px = rng.random_range(0.0..f32::from(lat.width()));
            let py = rng.random_range(0.0..f32::from(lat.height()));
            let sign = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
            let reach = (2.0 * radius).ceil() as i32;
            for y in (py as i32 - reach).max(0)..(py as i32 + reach).min(h) {
                for x in (px as i32 - reach).max(0)..(px as i32 + reach).min(w) {
                    let (dx, dy) = (x as f32 - px, y as f32 - py);
                    let q = (dx * dx + dy * dy) / (radius * radius);
                    self.patchy[(y * w + x) as usize] += sign * (-q).exp();
                }
            }
        }
        for p in &mut self.patchy {
            // The signed bump sum, geometrically between EPS_LO (−1) and EPS_HI (+1):
            // ×0.63 between patches, so the whole act recovers a little slower.
            let z = (p.clamp(-1.0, 1.0) + 1.0) / 2.0;
            *p = EPS_LO * (EPS_HI / EPS_LO).powf(z);
        }
        reset(&mut self.u, &mut self.v);
        self.tip_count = 0;
        self.heterogeneous = false;
        self.map.rebuild(lat, view);
    }

    /// Shade the heart into the pixels and draw the ECG sweep (update/resize only).
    fn rasterise(&mut self) {
        let (vx, vy, vs) = self.view;
        let flash = self.shocked_at.map_or(0.0, |at| {
            let age = self.step.saturating_sub(at) as f64 / SUB_PER_S as f64;
            (1.0 - age / 0.25).max(0.0)
        });
        let (map, u, v, edge) = (&self.map, &self.u, &self.v, &self.edge);
        let (vw, vh) = (i32::from(vs.width()), i32::from(vs.height()));
        self.pixels.shade(|x, y| {
            let (lx, ly) = (x - vx, y - vy);
            if !(0..vw).contains(&lx) || !(0..vh).contains(&ly) {
                return Ink::BLANK;
            }
            // The outline is the analytic heart curve, antialiased: every ink is dimmed by
            // the pixel's coverage (the ramps run to near-black, so a dimmer level reads as
            // a partly covered pixel), and a soft rim band outlines the resting organ.
            let [cover, rim] = edge.get((ly * vw + lx) as usize).copied().unwrap_or([0, 0]);
            if cover == 0 {
                return Ink::BLANK;
            }
            let (cover, rim) = (f64::from(cover) / 255.0, f64::from(rim) / 255.0);
            if flash > 0.0 {
                return Ink::new(
                    Role::Ramp(Ramp::Starlight),
                    ink::level((120.0 + 135.0 * flash) * cover),
                );
            }
            let (uu, vv) = (map.sample(u, lx, ly), map.sample(v, lx, ly));
            let ink = tissue_ink(uu, vv);
            if ink == REST_INK {
                let rest = f64::from(REST_INK.level) * cover;
                let rim = f64::from(RIM_INK.level) * rim;
                Ink::new(REST_INK.role, ink::level(rest.max(rim)))
            } else {
                Ink::new(ink.role, ink::level(f64::from(ink.level) * cover))
            }
        });
        // Tips as 2×2 white dots.
        let (sx, sy) = (
            f64::from(vs.width()) / f64::from(self.u.width()),
            f64::from(vs.height()) / f64::from(self.u.height()),
        );
        for &(tx, ty) in &self.tips[..self.tip_count.min(MAX_TIPS)] {
            let (px, py) = (
                vx + ((f64::from(tx) + 1.0) * sx) as i32,
                vy + ((f64::from(ty) + 1.0) * sy) as i32,
            );
            for (dx, dy) in [(0, 0), (-1, 0), (0, -1), (-1, -1)] {
                self.pixels.set(px + dx, py + dy, TIP_INK);
            }
        }
        self.draw_ecg();
    }

    /// Supersample the heart curve over the view (4×4 per pixel): its coverage, and the
    /// coverage of a rim band ~1.3 px wide inside the curve (the heart minus a copy
    /// shrunk about its centre by that much).
    fn build_edge(&mut self) {
        const SS: i32 = 4;
        let (_, _, vs) = self.view;
        let (vw, vh) = (i32::from(vs.width()), i32::from(vs.height()));
        let (hp, hx, hy) = heart_frame(vs);
        // The heart is ~1.1 units in radius, so this shrink moves its edge ~1.3 px in.
        let shrink = (1.0 - 1.3 * hp / 1.1).max(0.0);
        self.edge.clear();
        self.edge.reserve(vs.area());
        for py in 0..vh {
            for px in 0..vw {
                let (mut outer, mut inner) = (0u32, 0u32);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let fx = px as f32 + (sx as f32 + 0.5) / SS as f32;
                        let fy = py as f32 + (sy as f32 + 0.5) / SS as f32;
                        let (x, y) = ((fx - hx) * hp, (hy - fy) * hp);
                        if in_heart(x, y) {
                            outer += 1;
                            if shrink > 0.0 && in_heart(x / shrink, y / shrink) {
                                inner += 1;
                            }
                        }
                    }
                }
                let n = (SS * SS) as u32;
                let to_u8 = |k: u32| (k * 255 / n) as u8;
                self.edge.push([to_u8(outer), to_u8(outer - inner)]);
            }
        }
    }

    fn draw_ecg(&mut self) {
        self.dots.clear();
        if self.ecg_rows == 0 || self.ecg.is_empty() {
            return;
        }
        let n = self.ecg.len();
        let top = f64::from(self.cells.height() - 1 - self.ecg_rows) * f64::from(Dots::CELL_H);
        let span = f64::from(self.ecg_rows) * f64::from(Dots::CELL_H) - 1.0;
        let mid = top + span * 0.55;
        let amp = span * 0.45;
        let head = self.head as usize % n;
        let y_of = |s: f32| mid - amp * f64::from((s / self.ecg_scale).clamp(-1.0, 1.0));
        for c in 1..n {
            // Columns just ahead of the head are the sweep's blank gap.
            let ahead = (c + n - head) % n;
            if (1..=SWEEP_GAP).contains(&ahead) || ahead == SWEEP_GAP + 1 {
                continue;
            }
            let age = (head + n - c) % n;
            // Dimmer than the white write head, so the head leads the trace.
            let level = 205.0 - 115.0 * age as f64 / n as f64;
            let ink = Ink::new(Role::Ramp(ECG_RAMP), ink::level(level));
            let (y0, y1) = (y_of(self.ecg[c - 1]), y_of(self.ecg[c]));
            // Three dots thick over a dim glow in the pixels behind (the cell background
            // under the braille): a bold, luminous trace.
            for off in [-1.0, 0.0, 1.0] {
                self.dots
                    .line((c - 1) as f64, y0 + off, c as f64, y1 + off, ink);
            }
            let glow = Ink::new(Role::Ramp(ECG_RAMP), ink::level(level * 0.18));
            let (px, lo, hi) = (c as i32 / 2, y0.min(y1) / 2.0, y0.max(y1) / 2.0);
            for py in lo as i32..=hi as i32 {
                if self.pixels.get(px, py) == Some(Ink::BLANK) {
                    self.pixels.set(px, py, glow);
                }
            }
        }
        // The monitor's write head: a bright dot leading the trace, with a halo in the
        // pixels behind it (HRT-A7).
        let (hx, hy) = (head as f64, y_of(self.ecg[head]));
        self.dots.disc(hx, hy, 2.1, ECG_HEAD_INK);
        let (px, py) = (head as i32 / 2, (hy / 2.0) as i32);
        for (dx, dy) in [(0, 0), (-1, 0), (1, 0)] {
            let k = if dx == 0 { 0.6 } else { 0.25 };
            let halo = Ink::new(Role::Ramp(ECG_RAMP), ink::level(255.0 * k));
            if self.pixels.get(px + dx, py + dy) == Some(Ink::BLANK) {
                self.pixels.set(px + dx, py + dy, halo);
            }
        }
    }

    /// Advance the ECG sweep by `dt` seconds, writing `sample` into the columns passed.
    fn sweep(&mut self, dt: f64, sample: f32) {
        let n = self.ecg.len();
        if n == 0 {
            return;
        }
        // A lead's AC coupling: subtract a slow baseline so depolarisation reads as a
        // spike and repolarisation as a T wave, not as a plateau.
        self.ecg_base += (sample - self.ecg_base) * (dt / 0.15).min(1.0) as f32;
        let sample = sample - self.ecg_base;
        self.ecg_scale = (self.ecg_scale * 0.995).max(sample.abs() * 1.15).max(1e-3);
        let from = self.head as usize;
        self.head += dt * SWEEP_DOTS_PER_S;
        let to = self.head as usize;
        for c in from + 1..=to {
            self.ecg[c % n] = sample;
        }
        self.head %= n as f64;
    }
}

/// The excitation cycle in two saturated ramps (SPEC-002 Inks, as built): the excited
/// plateau hot (Ember, white-gold at the fresh front where `v` is still low, deepening
/// to orange as `v` rises), the refractory tail cold (Cobalt, brightest just behind the
/// plateau and fading to the dark resting tissue as `v` decays). Hue tells excited from
/// refractory at a glance, and lightness where in the cycle a cell is.
fn tissue_ink(u: f32, v: f32) -> Ink {
    if u >= 0.4 {
        let k = f64::from((v / 1.2).min(1.0));
        Ink::new(
            Role::Ramp(Ramp::Ember),
            ink::level(240.0 - 160.0 * k.powf(0.6)),
        )
    } else if v > 0.1 {
        let k = f64::from(((v - 0.1) / 1.3).min(1.0));
        Ink::new(
            Role::Ramp(Ramp::Cobalt),
            ink::level(28.0 + 200.0 * k.powf(0.8)),
        )
    } else {
        REST_INK
    }
}

/// Resting tissue (dimmed at the antialiased edge).
fn is_rest(ink: Ink) -> bool {
    ink.role == REST_INK.role && ink.level <= REST_INK.level
}

/// ASCII by what a pixel shows: tips `@`, the shock flash and excitation by phase angle,
/// rest `.`.
fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let pick = |i: Ink| i.role != Role::Background;
    let ink = match (pick(top), pick(bottom)) {
        (false, false) => return None,
        (true, false) => top,
        (false, true) => bottom,
        (true, true) => {
            if is_rest(top) {
                bottom
            } else {
                top
            }
        }
    };
    let ch = match ink.role {
        Role::Ramp(Ramp::Ember) => '#',
        Role::Ramp(Ramp::Cobalt) if ink.level > 110 => '+',
        Role::Ramp(Ramp::Cobalt) => ':',
        Role::Phase => {
            // Excited (angles near 0 and rising) dense, the recovering tail lighter.
            let a = ink::phase_value(ink);
            match a {
                a if (-0.6..1.2).contains(&a) => '#',
                a if (1.2..2.4).contains(&a) => '+',
                _ => ':',
            }
        }
        _ if ink == TIP_INK => '@',
        _ if is_rest(ink) => '.',
        _ => ASCII_RAMP[usize::from(ink.level) * (ASCII_RAMP.len() - 1) / 255],
    };
    Some((ch, ink))
}

impl Generator for Heart {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.caption_index()])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        self.pixels.resize(size);
        self.dots.resize(size);
        let h = size.height();
        self.ecg_rows = match h {
            20.. => 4,
            12.. => 2,
            _ => 0,
        };
        let art_rows = h.saturating_sub(1 + self.ecg_rows).max(1);
        let px_h = 2 * art_rows;
        // A square view (square pixels) centred over the art rows.
        let side = px_h.min(size.width());
        let vx = (i32::from(size.width()) - i32::from(side)) / 2;
        self.view = (vx, 0, Size::new(side, px_h));
        let lat = grid::lattice_for(Size::new(side, px_h), MAX_NODES, BUCKET);
        if lat != self.u.size() || self.u.is_empty() {
            self.rebuild(Size::new(side, px_h), &mut ctx.rng);
            // The story restarts at sinus rhythm on a fresh heart (SPEC-002 Resize).
            self.step = 0;
            self.shocked_at = None;
        } else {
            self.map.rebuild(lat, Size::new(side, px_h));
        }
        self.build_edge();
        let cols = usize::from(size.width()) * Dots::CELL_W as usize;
        self.ecg.clear();
        self.ecg.resize(cols, 0.0);
        self.head = 0.0;
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.owed += dt * SUB_PER_S as f64;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        for _ in 0..n as u32 {
            self.substep();
        }
        let sample = self.ecg_sample();
        self.sweep(dt, sample);
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
        let h = self.cells.height();
        if h < 4 {
            return;
        }
        let tips = self.tip_count;
        let mut label = StackText::<24>::new();
        let _ = match self.readout_room {
            15.. => write!(label, "spiral tips {tips:>2}"),
            7.. => write!(label, "tips {tips:>2}"),
            _ => return,
        };
        draw::label_right(canvas, i32::from(h) - 1, label.as_str(), LABEL_INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heart_at(size: Size) -> (Heart, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Heart::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn run_s(g: &mut Heart, ctx: &mut Ctx, secs: f64) {
        for _ in 0..(secs * 60.0).round() as usize {
            g.update(SIM_DT, ctx);
        }
    }

    fn excited(g: &Heart) -> usize {
        g.u.as_slice().iter().filter(|&&u| u > 0.5).count()
    }

    #[test]
    fn hrt_a1_one_stimulus_one_wave_then_rest() {
        let (mut g, _) = heart_at(Size::new(120, 40));
        g.sinus_beat();
        let mut peak = 0;
        for _ in 0..SUB_PER_S * 6 {
            // Drive the model alone: no scheduled beats.
            g.model.step(
                &g.medium,
                g.u.as_mut_slice(),
                g.v.as_mut_slice(),
                &mut g.lap,
                |_| 1.0,
            );
            peak = peak.max(excited(&g));
        }
        let inside = g.medium.inside().len();
        assert!(
            peak > inside / 10,
            "the wave spread: peak {peak} of {inside}"
        );
        assert_eq!(excited(&g), 0, "the heart is at rest again");
        assert!(g.u.as_slice().iter().all(|&u| u < 0.05));
    }

    /// Seconds into the cycle at which each act starts.
    fn act_start(index: usize) -> f64 {
        [
            0.0,
            SINUS_S,
            SINUS_S + REENTRY_S,
            SINUS_S + REENTRY_S + VF_S,
        ][index]
    }

    #[test]
    fn hrt_a2_s1_s2_makes_a_lasting_spiral() {
        let (mut g, mut ctx) = heart_at(Size::new(200, 50));
        run_s(&mut g, &mut ctx, act_start(REENTRY) + 4.5);
        assert!(g.s2_fired, "S2 fired");
        let mut seen = 0;
        for _ in 0..8 {
            run_s(&mut g, &mut ctx, 0.5);
            if g.tip_count >= 1 {
                seen += 1;
            }
        }
        assert!(
            seen >= 8,
            "a singularity persisted for ≥ 4 s ({seen}/8 checks)"
        );
    }

    #[test]
    fn hrt_a3_a4_fibrillation_then_shock_clears_it() {
        let (mut g, mut ctx) = heart_at(Size::new(200, 50));
        run_s(&mut g, &mut ctx, act_start(VF) + 2.0);
        let (mut sum, mut n) = (0, 0);
        while g.caption_index() == VF {
            run_s(&mut g, &mut ctx, 0.5);
            if g.caption_index() == VF {
                sum += g.tip_count;
                n += 1;
            }
        }
        let mean = sum as f64 / f64::from(n);
        assert!(mean >= 4.0, "mean tips in VF {mean:.1} over {n} samples");
        assert_eq!(g.caption_index(), SHOCK);
        run_s(&mut g, &mut ctx, 1.0);
        assert_eq!(g.tip_count, 0, "no tips 1 s after the shock");
    }

    #[test]
    fn hrt_a5_fields_stay_finite_and_in_range() {
        for size in [Size::new(1, 1), Size::new(80, 24), Size::new(200, 50)] {
            let (mut g, mut ctx) = heart_at(size);
            for k in 0..(42 * 4) {
                g.update(if k % 2 == 0 { 0.25 } else { 0.0 }, &mut ctx);
                for (&u, &v) in g.u.as_slice().iter().zip(g.v.as_slice()) {
                    assert!((0.0..=1.0).contains(&u) && (0.0..=excitable::AP_V_MAX).contains(&v));
                }
            }
        }
    }

    #[test]
    fn hrt_captions_follow_the_acts() {
        let (mut g, mut ctx) = heart_at(Size::new(80, 24));
        for (act, caption) in CAPTIONS.iter().enumerate() {
            let target = act_start(act) + 1.0;
            let wait = target - g.local_s();
            run_s(&mut g, &mut ctx, wait);
            assert_eq!(g.caption(), Some(*caption));
        }
    }

    #[test]
    fn hrt_a6_the_outline_is_antialiased() {
        let (g, _) = heart_at(Size::new(120, 40));
        let partial = g.edge.iter().filter(|e| (1..255).contains(&e[0])).count();
        let rim = g.edge.iter().filter(|e| e[1] > 0).count();
        let full = g.edge.iter().filter(|e| e[0] == 255).count();
        assert!(partial > 40, "{partial} partly covered edge pixels");
        assert!(
            rim > partial && full > 5 * rim,
            "a thin rim band: {rim} of {full}"
        );
        // Partly covered pixels are dimmer than the tissue inside.
        let (vx, vy, vs) = g.view;
        let vw = i32::from(vs.width());
        for (i, e) in g.edge.iter().enumerate() {
            let (x, y) = (vx + i as i32 % vw, vy + i as i32 / vw);
            if e[0] == 0 {
                assert_eq!(g.pixels.get(x, y), Some(Ink::BLANK));
            } else if e[0] < 64 && e[1] < 64 {
                assert!(
                    g.pixels
                        .get(x, y)
                        .is_some_and(|p| p.level <= REST_INK.level)
                );
            }
        }
    }

    #[test]
    fn hrt_a7_a_bright_dot_leads_the_ecg() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = heart_at(size);
        run_s(&mut g, &mut ctx, 2.0);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let head = g.head as i32 % g.ecg.len() as i32;
        let top = f64::from(size.height() - 1 - g.ecg_rows) * f64::from(Dots::CELL_H);
        let span = f64::from(g.ecg_rows) * f64::from(Dots::CELL_H) - 1.0;
        let s = (g.ecg[head as usize] / g.ecg_scale).clamp(-1.0, 1.0);
        let hy = top + span * 0.55 - span * 0.45 * f64::from(s);
        let cell = canvas.get(head / 2, (hy / 4.0) as i32).copied();
        assert_eq!(cell.map(|c| c.ink), Some(ECG_HEAD_INK), "the write head");
        // The rest of the trace is dimmer.
        let trace = (0..i32::from(size.width()))
            .filter_map(|x| {
                (i32::from(size.height() - 1 - g.ecg_rows)..i32::from(size.height()) - 1)
                    .filter_map(|y| canvas.get(x, y))
                    .find(|c| c.ink.role == Role::Ramp(ECG_RAMP))
            })
            .count();
        assert!(trace > 60, "{trace} trace cells");
    }

    #[test]
    fn heart_mask_is_heart_shaped() {
        assert!(in_heart(0.0, 0.0));
        assert!(in_heart(-0.6, 0.7) && in_heart(0.6, 0.7), "two lobes");
        assert!(!in_heart(0.0, 1.1), "the cleft");
        assert!(in_heart(0.0, -0.95) && !in_heart(0.5, -0.95), "the apex");
    }
}
