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
//! ## Look (M18, GEN-R26: excitation cloud + tip-meander threads)
//!
//! Braille only, on a black organ. Each wave is a two-lobed probability cloud sampled
//! bilinearly from the lattice at every dot: the excited tissue a `Hot` stipple with
//! density `0.7·u²`, the recovering refractory tail a sparse `Cold` stipple rising with
//! `v` (the opposite lobe), each dot's threshold fixed (CORE-R107) so the clouds hold
//! still and change only where a wave passes. The fresh front (`u = ½`, `v < ⅓`) is a
//! 1-px `Sun` thread. Phase singularities, the points where `θ = atan2(v − v*, u − u*)`
//! winds by 2π, are spiral tips: small `Star` atoms (CORE-R108) trailing the last 1.2 s
//! of their own meander as a thread, so fibrillation paints curling rosettes. The heart
//! outline is a 1-px `Dim` thread. A pseudo-ECG, `Φ = Σ ∇u·∇(1/r)` at an electrode below
//! the apex, sweeps along the bottom rows.

use std::fmt::Write as _;

use rand::Rng;
use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::rng;
use sciart_core::{draw, title};

use sciart_shared::excitable::{self, AlievPanfilov, Medium};

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
/// Sub-steps between singularity counts: about one per drawn frame (30 FPS), so a
/// meandering tip moves well under a node between counts and its trail stays smooth.
const COUNT_EVERY: u64 = 27;
/// Inks in M11's roles and flat steps (ADR-0018), as braille (Look (M18)).
/// The heart's outline thread.
const RIM_INK: Ink = Ink::new(Role::Dim, 110);
/// The excited lobe: `Hot` stipple, kept with probability `EXCITED_P·u²` above `U_EXCITED`.
const EXCITED_INK: Ink = Ink::new(Role::Hot, 200);
const EXCITED_P: f32 = 0.4;
const U_EXCITED: f32 = 0.15;
/// The refractory lobe: sparse `Cold` stipple where `u ≤ U_EXCITED` and `v` exceeds
/// `REFRACTORY_V·V_MAX`, kept with probability `REFRACTORY_P·min(1, v/V_MAX)`.
const REFRACTORY_INK: Ink = Ink::new(Role::Cold, 150);
const REFRACTORY_P: f32 = 0.12;
const REFRACTORY_V: f32 = 0.4;
/// The model's typical plateau `v` (Aliev–Panfilov, `k = 8`, `a = 0.1`): the refractory
/// tail peaks at about 2.0 behind every wave, measured at 200×50.
const V_MAX: f32 = 2.0;
/// The excited lobe has thinned to nothing this long (seconds) after a node's upstroke.
const AGE_FRESH: f32 = 0.15;
/// A node's upstroke: `u` rising through `U_UP`; it can fire again once below `U_DOWN`.
const U_UP: f32 = 0.5;
const U_DOWN: f32 = 0.3;
/// The age of a node that has not fired since the lattice was built.
const AGE_NEVER: f32 = 1.0e3;
/// The fresh front: a 1-px `Sun` thread on `u = ½` while `v < FRONT_V`, the dots within
/// `FRONT_W` dots of the contour (distance `|u − ½| / |∇u|` × dots per node).
const FRONT_INK: Ink = Ink::new(Role::Sun, 255);
const FRONT_V: f32 = 1.0 / 3.0;
const FRONT_W: f32 = 0.75;
/// Fixed stipple salt (CORE-R107) of the per-dot threshold bytes.
const EXCITED_SALT: u64 = 0x4845_4152_545F_4854;
/// The defibrillation shock: the organ flashes as a dense `Star` stipple for its first
/// `FLASH_S`.
const FLASH_INK: Ink = Ink::new(Role::Star, 200);
const FLASH_P: f32 = 0.5;
const FLASH_S: f64 = 0.125;
/// The ECG trace in a monitor's green, newer half brighter, led by a white write head.
const ECG_INKS: [Ink; 2] = [Ink::new(Role::Particle, 90), Ink::new(Role::Particle, 160)];
const ECG_HEAD_INK: Ink = Ink::new(Role::Star, 255);
/// Spiral tips: bright `Star` atoms (σ in dots), trailing their meander for `TRAIL_S` as a
/// thin `Dim` thread, the newer half `TRAIL_INKS[1]`, the older `TRAIL_INKS[0]`; the
/// trail follows the tip smoothed with time constant `TRAIL_TAU` (seconds), since a
/// detected singularity jitters by a node from count to count.
const TIP_INK: Ink = Ink::new(Role::Star, 255);
const TIP_SIGMA: f64 = 0.8;
const TIP_PEAK: f64 = 0.95;
const TRAIL_INKS: [Ink; 2] = [Ink::new(Role::Dim, 60), Ink::new(Role::Dim, 110)];
const TRAIL_S: f64 = 0.9;
const TRAIL_TAU: f32 = 0.15;
/// Positions per trail: one per singularity count (every `COUNT_EVERY` sub-steps), with
/// room to spare over `TRAIL_S`.
const TRAIL_N: usize = 48;
/// Counts a trail may miss its tip for and still continue.
const TRAIL_GAP: u64 = 2;
/// A tip that moved further than this between counts (dots, or lattice nodes when the
/// node spacing is coarser) is a new tip: its trail restarts.
const TRAIL_JUMP_DOTS: f32 = 3.0;
const TRAIL_JUMP_NODES: f32 = 1.5;
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
    /// `|∇u|` per node, refreshed by `rasterise` for the front's line (M14).
    slope: Grid<f32>,
    /// Seconds since each node's last upstroke, and whether it is still up (Look (M18)).
    age: Grid<f32>,
    up: Vec<bool>,
    lap: Vec<f32>,
    /// Per-node `ε` factor for the fibrillation act.
    patchy: Vec<f32>,
    /// Whether `patchy` is in force.
    heterogeneous: bool,
    theta: Vec<f32>,
    taken: Vec<bool>,
    tips: [(i32, i32); MAX_TIPS],
    tip_count: usize,
    /// Each detected tip refined inside its plaquette (lattice index units).
    tip_pos: [(f32, f32); MAX_TIPS],
    /// Meander trails, one fixed ring per tip slot (HRT-A10).
    trails: [Trail; MAX_TIPS],
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
    dots: Dots,
    /// Lattice → dot view sampling (bilinear), and the dot view: origin and size in dots.
    map: SampleMap,
    dview: (i32, i32, Size),
    /// Dots per lattice node.
    dpn: f32,
    /// Pixel rectangle the lattice is sized for (half-block pixels): origin and size.
    view: (i32, i32, Size),
    /// Per view dot: [`OUTSIDE`], [`ORGAN`] or [`OUTLINE`], built once per resize.
    organ: Vec<u8>,
    /// Fresh-front dots in the last frame (HRT-A11).
    front_dots: usize,
    /// Per view dot its fixed stipple threshold byte (CORE-R107), and per dot column and
    /// row the lattice cell it samples (the sample map's bilinear taps), built per resize.
    thr: Vec<u8>,
    cols: Vec<u16>,
    rows: Vec<u16>,
    /// Per lattice cell what it can show this frame.
    bounds: Vec<Bound>,
    ecg_rows: u16,
    cells: Size,
    readout_room: usize,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Heart {
    fn default() -> Self {
        Self {
            dirty: true,
            medium: Medium::default(),
            model: MODEL,
            u: Grid::default(),
            v: Grid::default(),
            slope: Grid::default(),
            age: Grid::default(),
            up: Vec::new(),
            lap: Vec::new(),
            patchy: Vec::new(),
            heterogeneous: false,
            theta: Vec::new(),
            taken: Vec::new(),
            tips: [(0, 0); MAX_TIPS],
            tip_count: 0,
            tip_pos: [(0.0, 0.0); MAX_TIPS],
            trails: [Trail::EMPTY; MAX_TIPS],
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
            dots: Dots::default(),
            map: SampleMap::default(),
            dview: (0, 0, Size::new(1, 1)),
            dpn: 1.0,
            view: (0, 0, Size::new(1, 1)),
            organ: Vec::new(),
            front_dots: 0,
            thr: Vec::new(),
            cols: Vec::new(),
            rows: Vec::new(),
            bounds: Vec::new(),
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
        self.track_tips();
    }

    /// Refine each detected tip inside its plaquette and extend the trail of the tip it
    /// continues (the nearest tip of the last count within the jump limit), or start a
    /// trail in a free slot (HRT-A10).
    fn track_tips(&mut self) {
        let n = self.tip_count.min(MAX_TIPS);
        let w = i32::from(self.u.width());
        let (u, v) = (self.u.as_slice(), self.v.as_slice());
        for k in 0..n {
            let (x, y) = self.tips[k];
            let (mut sx, mut sy, mut sw) = (0.0_f32, 0.0_f32, 0.0_f32);
            for (cx, cy) in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)] {
                let i = (cy * w + cx) as usize;
                // Closest to the phase centre weighs most: the singularity is where the
                // u and v isolines through (u*, v*) cross.
                let d = (u[i] - CENTRE.0).abs() + (v[i] - CENTRE.1).abs() + 1e-3;
                let wt = 1.0 / (d * d);
                sx += wt * cx as f32;
                sy += wt * cy as f32;
                sw += wt;
            }
            self.tip_pos[k] = (sx / sw, sy / sw);
        }
        let now = self.step;
        let jump = (TRAIL_JUMP_DOTS / self.dpn).max(TRAIL_JUMP_NODES);
        let horizon = (TRAIL_S * SUB_PER_S as f64) as u64;
        let mut claimed = [false; MAX_TIPS];
        for k in 0..n {
            let p = self.tip_pos[k];
            // The nearest trail extended at one of the last `TRAIL_GAP` counts (a tip the
            // detector missed once keeps its trail), within the jump limit per count.
            let best = (0..MAX_TIPS)
                .filter(|&j| !claimed[j])
                .filter_map(|j| {
                    let (q, at) = self.trails[j].last_tip()?;
                    let counts = now.saturating_sub(at) / COUNT_EVERY;
                    let d = (q.0 - p.0).hypot(q.1 - p.1);
                    ((1..=TRAIL_GAP).contains(&counts) && d <= jump * counts as f32)
                        .then_some((j, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let slot = match best {
                Some((j, _)) => j,
                None => {
                    // A free slot: empty, or its trail older than the horizon.
                    let Some(j) = (0..MAX_TIPS).find(|&j| {
                        !claimed[j]
                            && self.trails[j]
                                .newest()
                                .is_none_or(|(_, at)| now.saturating_sub(at) > horizon)
                    }) else {
                        continue;
                    };
                    self.trails[j].clear();
                    j
                }
            };
            claimed[slot] = true;
            self.trails[slot].push(p, now);
        }
    }

    /// Age every node by `dt` and restart the clock of each node whose `u` has risen
    /// through `U_UP` since the last update (its upstroke).
    fn age_nodes(&mut self, dt: f32) {
        let u = self.u.as_slice();
        for ((a, up), &uu) in self.age.as_mut_slice().iter_mut().zip(&mut self.up).zip(u) {
            if uu > U_UP && !*up {
                *up = true;
                *a = 0.0;
            } else {
                *a = (*a + dt).min(AGE_NEVER);
            }
            if uu < U_DOWN {
                *up = false;
            }
        }
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
        self.age = Grid::new(lat, AGE_NEVER);
        self.up.clear();
        self.up.resize(n, false);
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
    }

    /// Draw the heart and the ECG sweep as braille (Look (M18)), once per drawn frame.
    fn rasterise(&mut self) {
        self.dots.clear();
        let flash = self
            .shocked_at
            .is_some_and(|at| (self.step.saturating_sub(at) as f64 / SUB_PER_S as f64) < FLASH_S);
        let g_max = slope_field(&self.u, &mut self.slope);
        let band = FRONT_W * g_max / self.dpn;
        cell_bounds(&self.u, &self.v, &self.age, band, &mut self.bounds);
        let (ox, oy, ds) = self.dview;
        let (dw, dh) = (i32::from(ds.width()), i32::from(ds.height()));
        let (map, u, v, slope, organ) = (&self.map, &self.u, &self.v, &self.slope, &self.organ);
        let age = &self.age;
        let (thr, cols, rows, bounds) = (&self.thr, &self.cols, &self.rows, &self.bounds);
        let lw = usize::from(u.width());
        let dpn = self.dpn;
        let mut front = 0;
        for y in 0..dh {
            let start = (y * dw) as usize;
            let row = &organ[start..][..dw as usize];
            if row.iter().all(|&o| o == OUTSIDE) {
                continue;
            }
            let thr_row = &thr[start..][..dw as usize];
            let cell_row = &bounds[usize::from(rows[y as usize]) * lw..][..lw];
            let (ur, vr, gr) = (map.row(u, y), map.row(v, y), map.row(slope, y));
            let ar = map.row(age, y);
            for (x, (&o, &t)) in (0..dw).zip(row.iter().zip(thr_row)) {
                let (px, py) = (ox + x, oy + y);
                match o {
                    OUTSIDE => continue,
                    OUTLINE => {
                        self.dots.set(px, py, RIM_INK);
                        continue;
                    }
                    _ => {}
                }
                if flash {
                    // The refractory byte, so the flash picks its own dots.
                    if thresholds(t).1 < FLASH_P {
                        self.dots.set(px, py, FLASH_INK);
                    }
                    continue;
                }
                // Most dots are settled by their cell's bounds alone: no front nearby and
                // a threshold above any probability the cell can reach.
                let b = cell_row[usize::from(cols[x as usize])];
                let (th, tc) = thresholds(t);
                if !b.front && th >= b.hot && tc >= b.cold {
                    continue;
                }
                let uu = ur.at(x);
                let vv = vr.at(x);
                // Only fresh dots within `band` of u = ½ can be on the front (a sampled
                // slope never exceeds the lattice's largest).
                let du = (uu - 0.5).abs();
                if b.front && vv < FRONT_V && du <= band && du / gr.at(x).max(1e-4) * dpn <= FRONT_W
                {
                    self.dots.set(px, py, FRONT_INK);
                    front += 1;
                    continue;
                }
                if let Some(ink) = lobe_ink(uu, vv, ar.at(x), t) {
                    self.dots.set(px, py, ink);
                }
            }
        }
        self.front_dots = front;
        self.draw_tips();
        self.draw_ecg();
    }

    /// Lattice index coordinates to dot coordinates (the inverse of the sample map's
    /// cell-centre alignment).
    fn lat_to_dot(&self, p: (f32, f32)) -> (f64, f64) {
        let (ox, oy, _) = self.dview;
        let d = f64::from(self.dpn);
        (
            f64::from(ox) + (f64::from(p.0) + 0.5) * d - 0.5,
            f64::from(oy) + (f64::from(p.1) + 0.5) * d - 0.5,
        )
    }

    /// Meander trails (a thin `Dim` thread, brighter in its newer half), then the tips as `Star`
    /// atoms over them (HRT-A10).
    fn draw_tips(&mut self) {
        let horizon = (TRAIL_S * SUB_PER_S as f64) as u64;
        let now = self.step;
        let (ox, oy, _) = self.dview;
        let d = f64::from(self.dpn);
        let to_dot = |p: (f32, f32)| {
            (
                f64::from(ox) + (f64::from(p.0) + 0.5) * d - 0.5,
                f64::from(oy) + (f64::from(p.1) + 0.5) * d - 0.5,
            )
        };
        // The older half first, so the newer half lies over it.
        for (pass, &ink) in TRAIL_INKS.iter().enumerate() {
            for trail in &self.trails {
                let mut prev: Option<(f64, f64)> = None;
                for (p, at) in trail.iter() {
                    let age = now.saturating_sub(at);
                    if age > horizon {
                        prev = None;
                        continue;
                    }
                    let q = to_dot(p);
                    let newer = usize::from(2 * age < horizon);
                    if let Some(a) = prev
                        && newer == pass
                    {
                        self.dots.line(a.0, a.1, q.0, q.1, ink);
                    }
                    prev = Some(q);
                }
            }
        }
        for k in 0..self.tip_count.min(MAX_TIPS) {
            let (x, y) = self.lat_to_dot(self.tip_pos[k]);
            let salt = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            self.dots
                .stipple_puff(x, y, TIP_SIGMA, TIP_PEAK, salt, TIP_INK);
        }
    }

    /// Classify every dot of the dot view once per resize: inside the heart (the same
    /// mask the lattice uses, at the dot's lattice position) or not, and the outline: an
    /// inside dot with a 4-neighbour outside, one dot thick and closed.
    fn build_organ(&mut self) {
        let (_, _, ds) = self.dview;
        let (dw, dh) = (i32::from(ds.width()), i32::from(ds.height()));
        let lat = self.u.size();
        let (per_node, ox, oy) = heart_frame(lat);
        let (sx, sy) = (
            f32::from(lat.width()) / dw.max(1) as f32,
            f32::from(lat.height()) / dh.max(1) as f32,
        );
        self.organ.clear();
        self.organ.extend((0..dh).flat_map(|y| {
            (0..dw).map(move |x| {
                // The dot centre in lattice units, node x spanning [x, x + 1).
                let (lx, ly) = ((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
                if in_heart((lx - ox) * per_node, (oy - ly) * per_node) {
                    ORGAN
                } else {
                    OUTSIDE
                }
            })
        }));
        let at = |o: &[u8], x: i32, y: i32| {
            (0..dw).contains(&x) && (0..dh).contains(&y) && o[(y * dw + x) as usize] != OUTSIDE
        };
        for y in 0..dh {
            for x in 0..dw {
                let i = (y * dw + x) as usize;
                if self.organ[i] == ORGAN
                    && [(-1, 0), (1, 0), (0, -1), (0, 1)]
                        .iter()
                        .any(|&(ddx, ddy)| !at(&self.organ, x + ddx, y + ddy))
                {
                    self.organ[i] = OUTLINE;
                }
            }
        }
        // The lattice cell of each dot column and row: the bilinear tap's lower index.
        let tap = |d: i32, dn: i32, sn: u16| {
            let s = grid::centre_of(f64::from(d), dn as usize, usize::from(sn))
                .clamp(0.0, f64::from(sn.saturating_sub(1)));
            s as u16
        };
        self.cols.clear();
        self.cols.extend((0..dw).map(|x| tap(x, dw, lat.width())));
        self.rows.clear();
        self.rows.extend((0..dh).map(|y| tap(y, dh, lat.height())));
        let (ox, oy, _) = self.dview;
        self.thr.clear();
        self.thr.extend((0..dh).flat_map(|y| {
            (0..dw).map(move |x| {
                let key = rng::cell_key(ox + x, oy + y);
                (rng::hash01(EXCITED_SALT ^ key) * 256.0) as u8
            })
        }));
    }

    fn draw_ecg(&mut self) {
        if self.ecg_rows == 0 || self.ecg.is_empty() {
            return;
        }
        let n = self.ecg.len();
        let top = f64::from(self.cells.height() - 1 - self.ecg_rows) * f64::from(Dots::CELL_H);
        let span = f64::from(self.ecg_rows) * f64::from(Dots::CELL_H) - 1.0;
        let mid = top + span * 0.55;
        let amp = span * 0.45;
        let head = self.head as usize % n;
        // Samples are stored already scaled (see `sweep`), so the drawn history never moves.
        let y_of = |s: f32| mid - amp * f64::from(s);
        for c in 1..n {
            // Columns just ahead of the head are the sweep's blank gap.
            let ahead = (c + n - head) % n;
            if (1..=SWEEP_GAP).contains(&ahead) || ahead == SWEEP_GAP + 1 {
                continue;
            }
            let age = (head + n - c) % n;
            // Dimmer than the white write head, so the head leads the trace.
            let ink = ECG_INKS[usize::from(2 * age < n)];
            let (y0, y1) = (y_of(self.ecg[c - 1]), y_of(self.ecg[c]));
            // A 1-dot trace on black.
            self.dots.line((c - 1) as f64, y0, c as f64, y1, ink);
        }
        // The monitor's write head: a small bright dot leading the trace (HRT-A7).
        let (hx, hy) = (head as f64, y_of(self.ecg[head]));
        self.dots.disc(hx, hy, 1.1, ECG_HEAD_INK);
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
        // Stored in units of the scale at write time, as a monitor writes its trace.
        let scaled = (sample / self.ecg_scale).clamp(-1.0, 1.0);
        let from = self.head as usize;
        self.head += dt * SWEEP_DOTS_PER_S;
        let to = self.head as usize;
        for c in from + 1..=to {
            self.ecg[c % n] = scaled;
        }
        self.head %= n as f64;
    }
}

/// A view dot's class in [`Heart::organ`].
const OUTSIDE: u8 = 0;
const ORGAN: u8 = 1;
const OUTLINE: u8 = 2;

/// The excited lobe's keep probability (Look (M18)): `0.45·u²` damped by the time since
/// the node's upstroke, `(1 − age/AGE_FRESH)²`, so freshly depolarised tissue is densest
/// and a wave is a crescent hugging its front instead of a filled band (`v` cannot tell
/// fresh tissue from old: it barely rises on the plateau). Rises with `u`, falls with
/// `age`.
fn excited_p(u: f32, age: f32) -> f32 {
    let fresh = (1.0 - age / AGE_FRESH).max(0.0);
    EXCITED_P * u * u * fresh * fresh
}

/// The refractory lobe's keep probability: `0.12·min(1, v/V_MAX)`. Rises with `v`.
fn refractory_p(v: f32) -> f32 {
    REFRACTORY_P * (v / V_MAX).min(1.0)
}

/// A dot's two fixed uniforms in `(0, 1)` from its stored threshold byte (CORE-R107):
/// the excited lobe's, and the refractory lobe's (the byte XOR a constant, another
/// uniform byte, so the two lobes pick different dots).
#[inline]
fn thresholds(t: u8) -> (f32, f32) {
    let uniform = |b: u8| (f32::from(b) + 0.5) / 256.0;
    (uniform(t), uniform(t ^ 0xA5))
}

/// The two lobes of the excitation cloud at a dot with threshold byte `t` (Look (M18),
/// HRT-A9): excited tissue (`u > U_EXCITED`) in `Hot`, recovering tissue
/// (`u ≤ U_EXCITED`, `v > ¼·V_MAX`) in `Cold`. `None` for an unlit dot.
fn lobe_ink(u: f32, v: f32, age: f32, t: u8) -> Option<Ink> {
    let (th, tc) = thresholds(t);
    if u > U_EXCITED {
        (th < excited_p(u, age)).then_some(EXCITED_INK)
    } else if v > REFRACTORY_V * V_MAX {
        (tc < refractory_p(v)).then_some(REFRACTORY_INK)
    } else {
        None
    }
}

/// What a lattice cell (the square between nodes `(x, y)` and `(x + 1, y + 1)`) can show
/// anywhere inside it: bilinear `u` and `v` stay within the corners' range, and the lobe
/// probabilities are monotone in them, so these bound every dot sampled in the cell.
#[derive(Debug, Clone, Copy, Default)]
struct Bound {
    hot: f32,
    cold: f32,
    /// The cell may hold a fresh front dot.
    front: bool,
}

/// Fill `out` (lattice-sized, indexed by the cell's top-left node) with each cell's
/// [`Bound`]; `band` is the widest `|u − ½|` a front dot can have.
fn cell_bounds(u: &Grid<f32>, v: &Grid<f32>, age: &Grid<f32>, band: f32, out: &mut Vec<Bound>) {
    let (w, h) = (usize::from(u.width()), usize::from(u.height()));
    out.clear();
    out.resize(w * h, Bound::default());
    let (u, v, age) = (u.as_slice(), v.as_slice(), age.as_slice());
    for y in 0..h {
        let y1 = (y + 1).min(h - 1);
        for x in 0..w {
            let x1 = (x + 1).min(w - 1);
            let idx = [y * w + x, y * w + x1, y1 * w + x, y1 * w + x1];
            let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            let mut a0 = f32::MAX;
            for i in idx {
                a0 = a0.min(age[i]);
                (u0, u1) = (u0.min(u[i]), u1.max(u[i]));
                (v0, v1) = (v0.min(v[i]), v1.max(v[i]));
            }
            out[y * w + x] = Bound {
                hot: if u1 > U_EXCITED {
                    excited_p(u1, a0)
                } else {
                    0.0
                },
                cold: if u0 <= U_EXCITED && v1 > REFRACTORY_V * V_MAX {
                    refractory_p(v1)
                } else {
                    0.0
                },
                front: v0 < FRONT_V && u0 <= 0.5 + band && u1 >= 0.5 - band,
            };
        }
    }
}

/// A tip's meander: a fixed ring of its last [`TRAIL_N`] smoothed positions (lattice
/// index units) and the sub-step each was taken at, plus the last raw detection (for
/// matching the next count's tips).
#[derive(Debug, Clone, Copy)]
struct Trail {
    pts: [(f32, f32); TRAIL_N],
    at: [u64; TRAIL_N],
    raw: (f32, f32),
    /// Next slot to write, and how many are filled.
    head: usize,
    len: usize,
}

impl Trail {
    const EMPTY: Self = Self {
        pts: [(0.0, 0.0); TRAIL_N],
        at: [0; TRAIL_N],
        raw: (0.0, 0.0),
        head: 0,
        len: 0,
    };

    fn clear(&mut self) {
        self.len = 0;
    }

    /// Record the tip detected at `p` at sub-step `at`: the trail moves toward it by an
    /// exponential moving average with time constant [`TRAIL_TAU`].
    fn push(&mut self, p: (f32, f32), at: u64) {
        let q = match self.newest() {
            Some((last, then)) => {
                let dt = at.saturating_sub(then) as f32 / SUB_PER_S as f32;
                let k = 1.0 - (-dt / TRAIL_TAU).exp();
                (last.0 + k * (p.0 - last.0), last.1 + k * (p.1 - last.1))
            }
            None => p,
        };
        self.raw = p;
        self.pts[self.head] = q;
        self.at[self.head] = at;
        self.head = (self.head + 1) % TRAIL_N;
        self.len = (self.len + 1).min(TRAIL_N);
    }

    /// The newest smoothed position and its sub-step.
    fn newest(&self) -> Option<((f32, f32), u64)> {
        (self.len > 0).then(|| {
            let i = (self.head + TRAIL_N - 1) % TRAIL_N;
            (self.pts[i], self.at[i])
        })
    }

    /// The last raw detection and its sub-step.
    fn last_tip(&self) -> Option<((f32, f32), u64)> {
        self.newest().map(|(_, at)| (self.raw, at))
    }

    /// Oldest first.
    fn iter(&self) -> impl Iterator<Item = ((f32, f32), u64)> + '_ {
        let start = (self.head + TRAIL_N - self.len) % TRAIL_N;
        (0..self.len).map(move |k| {
            let i = (start + k) % TRAIL_N;
            (self.pts[i], self.at[i])
        })
    }
}

/// `|∇u|` per node by central differences (one-sided at the edges) into `out`; returns
/// the largest.
fn slope_field(u: &Grid<f32>, out: &mut Grid<f32>) -> f32 {
    let (w, h) = (usize::from(u.width()), usize::from(u.height()));
    if out.size() != u.size() {
        out.resize(u.size(), 0.0);
    }
    if w == 0 || h == 0 {
        return 0.0;
    }
    let mut max = 0.0_f32;
    let (p, o) = (u.as_slice(), out.as_mut_slice());
    for y in 0..h {
        let (yn, ys) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for x in 0..w {
            let (xw, xe) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let gx = (p[y * w + xe] - p[y * w + xw]) / (xe - xw).max(1) as f32;
            let gy = (p[ys * w + x] - p[yn * w + x]) / (ys - yn).max(1) as f32;
            let g = (gx * gx + gy * gy).sqrt();
            o[y * w + x] = g;
            max = max.max(g);
        }
    }
    max
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
        }
        // The dot view over the same square: 2 dots per pixel each way.
        let dsize = Size::new(side.saturating_mul(2), px_h.saturating_mul(2));
        self.dview = (2 * vx, 0, dsize);
        self.map.rebuild(lat, dsize);
        self.dpn = f32::from(dsize.width()) / f32::from(lat.width().max(1));
        self.trails = [Trail::EMPTY; MAX_TIPS];
        self.build_organ();
        let cols = usize::from(size.width()) * Dots::CELL_W as usize;
        self.ecg.clear();
        self.ecg.resize(cols, 0.0);
        self.head = 0.0;
        self.dirty = true;
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
        self.age_nodes(dt as f32);
        let sample = self.ecg_sample();
        self.sweep(dt, sample);
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
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
        g.prepare();
        (g, ctx)
    }

    fn run_s(g: &mut Heart, ctx: &mut Ctx, secs: f64) {
        for _ in 0..(secs * 60.0).round() as usize {
            g.update(SIM_DT, ctx);
            g.prepare();
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
                g.prepare();
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

    /// Run to `t` seconds and render; the canvas and the heart.
    fn frame_at(size: Size, t: f64) -> (Heart, Canvas) {
        let (mut g, mut ctx) = heart_at(size);
        run_s(&mut g, &mut ctx, t);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        (g, canvas)
    }

    #[test]
    fn hrt_a8_frames_are_braille_only() {
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let (mut g, mut ctx) = heart_at(size);
            for t in [5.0, 30.0] {
                let wait = t - g.local_s();
                run_s(&mut g, &mut ctx, wait);
                let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                g.render(&mut canvas);
                let (mut lit, mut blocks) = (0_u32, 0_u32);
                for c in canvas.cells() {
                    if c.ch == ' ' && c.bg == Ink::BLANK {
                        continue;
                    }
                    lit += 1;
                    blocks += u32::from(c.bg != Ink::BLANK || matches!(c.ch, '▀' | '▄' | '█'));
                }
                assert!(lit > 50, "{size:?} t {t}: {lit} lit cells");
                assert!(
                    f64::from(blocks) <= 0.03 * f64::from(lit),
                    "{size:?} t {t}: {blocks} half-block cells of {lit}"
                );
            }
        }
    }

    #[test]
    fn hrt_a9_two_lobes_with_density_rising_in_u() {
        // Lit share over every threshold byte: the exact keep probability.
        let share = |u: f32, v: f32| {
            (0..=255u8)
                .filter(|&t| lobe_ink(u, v, 0.0, t).is_some())
                .count() as f64
                / 256.0
        };
        // The excited lobe's lit share rises with u: ≥ 3× from u ≈ 0.3 to u ≈ 0.9.
        let (lo, hi) = (share(0.3, 0.0), share(0.9, 0.0));
        assert!(
            hi >= 3.0 * lo && lo > 0.0,
            "lit share {lo:.3} at u 0.3, {hi:.3} at 0.9"
        );
        // Every lit field dot belongs to its lobe.
        for ui in 0..=20 {
            for vi in 0..=20 {
                let (u, v) = (ui as f32 / 20.0, vi as f32 / 20.0 * 2.5);
                for t in 0..=255u8 {
                    match lobe_ink(u, v, 0.05, t) {
                        Some(i) if i == EXCITED_INK => assert!(u > 0.15),
                        Some(i) if i == REFRACTORY_INK => {
                            assert!(u <= 0.15 && v > 0.25 * V_MAX, "cold at u {u} v {v}");
                        }
                        Some(i) => panic!("{i:?}"),
                        None => {}
                    }
                }
            }
        }
        // The refractory lobe is sparse: at most 0.22.
        assert!(share(0.0, 3.0) <= 0.23);
    }

    #[test]
    fn hrt_a10_tips_are_atoms_trailing_their_own_meander() {
        let (mut g, mut ctx) = heart_at(Size::new(200, 50));
        run_s(&mut g, &mut ctx, act_start(VF) + 4.0);
        let horizon = (TRAIL_S * SUB_PER_S as f64) as u64;
        let jump = (TRAIL_JUMP_DOTS / g.dpn).max(TRAIL_JUMP_NODES);
        let mut checked = 0;
        for _ in 0..12 {
            run_s(&mut g, &mut ctx, 0.25);
            assert!(g.tip_count > 0, "fibrillation has tips");
            for k in 0..g.tip_count.min(MAX_TIPS) {
                let (x, y) = g.lat_to_dot(g.tip_pos[k]);
                // The atom's centre dot is lit, and lies within 1 dot of the tip.
                assert!(g.dots.is_lit(x.round() as i32, y.round() as i32), "tip {k}");
                let (tx, ty) = g.tips[k];
                let p = g.tip_pos[k];
                assert!(
                    (tx as f32..=tx as f32 + 1.0).contains(&p.0)
                        && (ty as f32..=ty as f32 + 1.0).contains(&p.1),
                    "the tip stays in its plaquette"
                );
                checked += 1;
            }
            // Each trail: consecutive positions within the jump limit, drawn only while
            // no older than 1.2 s, and every live trail ends at a tip of this count.
            for trail in &g.trails {
                let pts: Vec<_> = trail
                    .iter()
                    .filter(|&(_, at)| g.step.saturating_sub(at) <= horizon)
                    .collect();
                for w in pts.windows(2) {
                    let (a, b) = (w[0].0, w[1].0);
                    let counts = ((w[1].1 - w[0].1) / COUNT_EVERY) as f32;
                    assert!(counts <= TRAIL_GAP as f32);
                    assert!((a.0 - b.0).hypot(a.1 - b.1) <= jump * counts + 1e-4);
                    assert!(w[1].1 > w[0].1, "oldest first");
                }
            }
        }
        assert!(checked > 20);
    }

    #[test]
    // HRT-A6 (M15 half-block rim and Hot front) is superseded by HRT-A8..A11; its thin-rim
    // check carries over on the dot lattice.
    fn hrt_a6_a11_reentry_frame_is_lobes_front_tips_outline_and_ecg() {
        let (g, canvas) = frame_at(Size::new(120, 40), act_start(REENTRY) + 3.0);
        assert!(g.front_dots > 20, "{} fresh-front dots", g.front_dots);
        let allowed = [
            EXCITED_INK,
            REFRACTORY_INK,
            FRONT_INK,
            TIP_INK,
            TRAIL_INKS[0],
            TRAIL_INKS[1],
            RIM_INK,
            FLASH_INK,
            ECG_INKS[0],
            ECG_INKS[1],
            ECG_HEAD_INK,
            LABEL_INK,
        ];
        for c in canvas.cells() {
            if c.ch != ' ' {
                assert!(allowed.contains(&c.ink), "{:?}", c.ink);
            }
        }
        // The outline is a thin thread: under a fifth of the organ's dots.
        let rim = g.organ.iter().filter(|&&o| o == OUTLINE).count();
        let organ = g.organ.iter().filter(|&&o| o != OUTSIDE).count();
        assert!(rim > 40 && organ > 5 * rim, "{rim} of {organ}");
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
        let s = g.ecg[head as usize];
        let hy = top + span * 0.55 - span * 0.45 * f64::from(s);
        let cell = canvas.get(head / 2, (hy / 4.0) as i32).copied();
        assert_eq!(cell.map(|c| c.ink), Some(ECG_HEAD_INK), "the write head");
        // The rest of the trace is dimmer.
        let trace = (0..i32::from(size.width()))
            .filter_map(|x| {
                (i32::from(size.height() - 1 - g.ecg_rows)..i32::from(size.height()) - 1)
                    .filter_map(|y| canvas.get(x, y))
                    .find(|c| ECG_INKS.contains(&c.ink))
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
