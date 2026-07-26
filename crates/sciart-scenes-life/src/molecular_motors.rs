//! Motor proteins: two molecular machines fuelled by ATP. Kinesin walks hand over hand
//! along a microtubule, one 8 nm step per ATP, jiggling in thermal noise between steps;
//! then ATP synthase's rotor turns in 120° steps, each split into 80° and 40° substeps
//! (SPEC-002 `molecular_motors`).
//!
//! Kinesin: two heads on the microtubule's 8 nm lattice of binding sites. Dwell times are
//! exponential with the Michaelis–Menten rate `k = k_cat[ATP]/(K_M + [ATP])`; each step
//! the trailing head unbinds, diffuses (overdamped Langevin) in a potential pulling it
//! forward (neck-linker docking) and binds 16 nm ahead, so the motor's centre moves
//! exactly 8 nm and the heads alternate. A bound head jiggles in a harmonic well (an
//! Ornstein–Uhlenbeck process with no drift). The cargo bead trails on a spring.
//! `[ATP]` steps down a `Timeline`; dwells are slowed [`SLOW`]× for the eye.
//!
//! ATP synthase, seen from the membrane's top as in Noji's single-molecule films: the
//! α₃β₃ ring round the γ rotor, with an actin filament stuck on γ to make its turns
//! visible. Each 120° turn is an 80° substep after an ATP-binding dwell and a 40°
//! substep after a catalytic dwell, both exponential; each turn releases one ATP.
//!
//! Look (M17, GEN-R26: cloud heads + step arches): all braille on black. Heads, tubulin
//! monomers, c-subunits and the α₃β₃ lobes are Gaussian stipple clouds whose thresholds
//! are keyed to the offset from their centre (CORE-R107), so a cloud moves rigidly with
//! its molecule and never shimmers; the bound head is `Cold`, the swinging head `Hot`.
//! Linkers, stalk, γ and the proton half-channels are 1-dot threads; the vesicle is a
//! hollow stipple shell. The swinging head's path is sampled at 80 Hz and drawn as an arch
//! thread (`Sun` while the neck linker pulls hard, `Hot` as it settles) that dissolves
//! over 2 s, so the walk writes a chain of arches. Subjects (heads; γ, c-ring, protons)
//! sit on a front layer blitted over the rest, so the cells they cross show them alone.

use std::f64::consts::PI;
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::particles::gaussian;
use sciart_core::prelude::*;
use sciart_core::{draw, ease, raster, title};

pub static META: Meta = Meta {
    name: "molecular_motors",
    category: Category::LivingMatter,
    description: "Kinesin walks hand over hand, one 8 nm step per ATP; ATP synthase turns in 120-degree steps",
    title: "Motor proteins · fuelled by ATP",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 2] = ["kinesin walks", "synthase turns"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Motors::default())
}

// ---- Kinesin (SPEC-002 Model) ----

/// Binding-site spacing along the microtubule (nm).
pub(crate) const SITE_NM: f64 = 8.0;
/// Kinesin's catalytic rate (steps per second at saturating ATP) and Michaelis constant.
pub(crate) const K_CAT: f64 = 100.0;
pub(crate) const K_M_UM: f64 = 60.0;
/// Display slow-down of the dwells.
pub(crate) const SLOW: f64 = 40.0;
/// Bound-head well stiffness (per second) and diffusion (nm²/s), display time.
const WELL_K: f64 = 30.0;
const WELL_D: f64 = 6.0;
/// Free-head pull towards its next site and diffusion (display time).
const SWING_K: f64 = 9.0;
const SWING_D: f64 = 20.0;
/// A free head binds within this distance of its site (nm).
const CAPTURE_NM: f64 = 1.2;
/// Model sub-step (s).
const H: f64 = 1.0 / 240.0;

/// The Michaelis–Menten stepping rate at `atp_um` (per second, real time).
pub(crate) fn rate(atp_um: f64) -> f64 {
    K_CAT * atp_um / (K_M_UM + atp_um)
}

/// An exponential dwell with rate `k`.
pub(crate) fn dwell(rng: &mut impl Rng, k: f64) -> f64 {
    let u: f64 = rng.random_range(f64::MIN_POSITIVE..1.0);
    -u.ln() / k
}

/// One step of an overdamped particle in a harmonic well about `centre`.
fn langevin(x: f64, centre: f64, k: f64, d: f64, h: f64, rng: &mut impl Rng) -> f64 {
    x - k * (x - centre) * h + gaussian(rng, 0.0, (2.0 * d * h).sqrt())
}

/// Two heads on a lattice of sites: `site[i]` is head `i`'s site, `free` the head in
/// flight (if any) with its position.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Kinesin {
    pub(crate) site: [i64; 2],
    /// Jiggle of each bound head about its site (nm), and its height offset.
    pub(crate) jig: [[f64; 2]; 2],
    pub(crate) free: Option<(usize, [f64; 2])>,
    /// Seconds (display) until the next step starts.
    pub(crate) wait: f64,
    pub(crate) steps: u64,
    /// Head that moved last.
    pub(crate) last_moved: Option<usize>,
}

impl Kinesin {
    pub(crate) fn new() -> Self {
        Self {
            site: [0, 1],
            jig: [[0.0; 2]; 2],
            free: None,
            wait: 0.5,
            steps: 0,
            last_moved: None,
        }
    }

    /// The motor's centre (nm): the midpoint of its two sites.
    pub(crate) fn centre(&self) -> f64 {
        0.5 * (self.site[0] + self.site[1]) as f64 * SITE_NM
    }

    /// Advance one sub-step at `[ATP]`.
    pub(crate) fn step(&mut self, atp_um: f64, rng: &mut impl Rng) {
        for i in 0..2 {
            if self.free.is_none_or(|(f, _)| f != i) {
                for a in 0..2 {
                    self.jig[i][a] = langevin(self.jig[i][a], 0.0, WELL_K, WELL_D * 0.02, H, rng);
                }
            }
        }
        match self.free {
            None => {
                self.wait -= H;
                if self.wait <= 0.0 {
                    // ATP binds the leading head; the trailing head lets go.
                    let trail = if self.site[0] < self.site[1] { 0 } else { 1 };
                    let pos = [self.site[trail] as f64 * SITE_NM, 0.0];
                    self.free = Some((trail, pos));
                }
            }
            Some((i, mut p)) => {
                let lead = self.site[1 - i];
                let target = (lead + 1) as f64 * SITE_NM;
                // Pulled forward and held above the track by the docked neck linker.
                let arc = -4.0 * (1.0 - ((p[0] - target + SITE_NM) / SITE_NM).powi(2)).max(0.0);
                p[0] = langevin(p[0], target, SWING_K, SWING_D, H, rng);
                p[1] = langevin(p[1], arc, SWING_K * 2.0, SWING_D * 0.2, H, rng);
                if (p[0] - target).abs() < CAPTURE_NM && p[1].abs() < 2.0 {
                    self.site[i] = lead + 1;
                    self.jig[i] = [p[0] - target, 0.0];
                    self.free = None;
                    self.steps += 1;
                    self.last_moved = Some(i);
                    self.wait = dwell(rng, rate(atp_um)) * SLOW;
                } else {
                    self.free = Some((i, p));
                }
            }
        }
    }

    /// Position of head `i` (nm; y up is negative).
    pub(crate) fn head(&self, i: usize) -> [f64; 2] {
        match self.free {
            Some((f, p)) if f == i => p,
            _ => [
                self.site[i] as f64 * SITE_NM + self.jig[i][0],
                self.jig[i][1],
            ],
        }
    }
}

// ---- ATP synthase ----

/// Mean dwells (display seconds) before the 80° and the 40° substeps.
const BIND_DWELL: f64 = 0.45;
const CAT_DWELL: f64 = 0.25;
/// Seconds a substep's rotation takes on screen.
const TURN_S: f64 = 0.08;

/// The rotor: committed angle (degrees, a multiple of 40), and the substep in progress.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rotor {
    /// Angle reached by completed substeps (degrees).
    pub(crate) angle: f64,
    /// Next substep is the 80° one.
    pub(crate) next_big: bool,
    pub(crate) wait: f64,
    /// Seconds into the current substep's rotation, if turning.
    pub(crate) turning: Option<f64>,
    pub(crate) turns: u64,
}

impl Rotor {
    pub(crate) fn new() -> Self {
        Self {
            angle: 0.0,
            next_big: true,
            wait: BIND_DWELL,
            turning: None,
            turns: 0,
        }
    }

    fn substep_deg(&self) -> f64 {
        if self.next_big { 80.0 } else { 40.0 }
    }

    /// Advance by `h`; returns true when a 120° turn completes (one ATP made).
    pub(crate) fn step(&mut self, h: f64, rng: &mut impl Rng) -> bool {
        if let Some(t) = self.turning {
            let t = t + h;
            if t >= TURN_S {
                self.angle += self.substep_deg();
                let done = !self.next_big;
                self.next_big = !self.next_big;
                self.turning = None;
                let mean = if self.next_big { BIND_DWELL } else { CAT_DWELL };
                self.wait = dwell(rng, 1.0 / mean);
                if done {
                    self.turns += 1;
                }
                return done;
            }
            self.turning = Some(t);
        } else {
            self.wait -= h;
            if self.wait <= 0.0 {
                self.turning = Some(0.0);
            }
        }
        false
    }

    /// The angle shown (degrees): committed plus the eased part of the substep in flight.
    pub(crate) fn shown(&self) -> f64 {
        self.angle
            + self
                .turning
                .map_or(0.0, |t| self.substep_deg() * ease::in_out_cubic(t / TURN_S))
    }
}

// ---- Scene ----

const KINESIN_S: f64 = 24.0;
const SYNTHASE_S: f64 = 16.0;
const KINESIN: usize = 0;
/// `[ATP]` (µM) over the kinesin act: saturating, near `K_M`, scarce.
const ATP_STEPS: [(f64, f64); 3] = [(0.0, 1000.0), (8.0, 60.0), (16.0, 15.0)];
const ATP_DOTS: usize = 90;
/// Binding marks kept, and the seconds the site a head bound to glows (KIN-A7).
const TRAIL: usize = 8;
const TRAIL_S: f64 = 5.0;
/// Cargo vesicle radius and its tether rest height (nm).
const CARGO_R: f64 = 9.0;
const CARGO_Y: f64 = -18.0;
/// Low-pass time of the drawn vesicle position (s).
const CARGO_VIEW_S: f64 = 0.4;
/// ATP diffusion step per model sub-step (fraction of the half-screen): slowed for the eye,
/// so molecules drift rather than twinkle from place to place.
const ATP_STEP: f64 = 0.005;
/// The swinging head's path: sampled every [`ARCH_EVERY`] model sub-steps (80 Hz), the last
/// [`ARCH_N`] samples kept, each dissolving over [`ARCH_S`] (KIN-A11).
const ARCH_EVERY: u32 = 3;
const ARCH_N: usize = 256;
const ARCH_S: f64 = 2.0;
/// Pull speed (nm/s, display time) above which the arch is `Sun` rather than `Hot`: the
/// docking neck linker pulls hardest at the start of the swing.
const ARCH_FAST: f64 = SWING_K * SITE_NM;
/// Low-pass time of the drawn path (s): smooths thermal kicks, keeps the arch.
const ARCH_TAU: f64 = 0.03;
const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

// Inks (M11 roles, GEN-R23). Kinesin act: the heads are the two signs of an orbital lobe
// (bound `Cold`, swinging `Hot`); tubulin dimers alternate `Cold` (β, the binding sites)
// and `Structure` (α); a site a head just bound glows brighter; the linkers, stalk and the
// vesicle's shell are `Structure`; the arches run `Sun` → `Hot` as the swing slows; ATP is
// `Sun`. Synthase act: α `Hot`, β `Cold`, the c-ring `Structure`, γ `Accent`, protons
// `Cold`, the half-channels `Dim`.
const ATP_INK: Ink = Ink::new(Role::Sun, 255);
const TUB_ALPHA: Ink = Ink::new(Role::Structure, 120);
const TUB_BETA: Ink = Ink::new(Role::Cold, 150);
const TUB_MARK: Ink = Ink::new(Role::Cold, 235);
const SHELL: Ink = Ink::new(Role::Structure, 150);
const LINK: Ink = Ink::new(Role::Star, 200);
const BOUND: Ink = Ink::new(Role::Cold, 240);
const SWING: Ink = Ink::new(Role::Hot, 245);
const ALPHA: Ink = Ink::new(Role::Hot, 200);
const BETA: Ink = Ink::new(Role::Cold, 200);
const C_RING: Ink = Ink::new(Role::Structure, 210);
const GAMMA: Ink = Ink::new(Role::Accent, 255);
const GHOST: Ink = Ink::new(Role::Accent, 120);
const PROTON: Ink = Ink::new(Role::Cold, 255);
const CHANNEL: Ink = Ink::new(Role::Dim, 150);

/// Stipple salts (CORE-R107).
const SALT_SHELL: u64 = 0x1F83_D9AB_FB41_BD6B;
const SALT_HEAD: u64 = 0x5BE0_CD19_137E_2179;
const SALT_LOBE: u64 = 0x629A_292A_367C_D507;
const SALT_CRING: u64 = 0x9159_015A_3070_DD17;

/// Layers (tests draw them one at a time).
pub(crate) const TUBE: u8 = 1;
pub(crate) const HEADS: u8 = 2;
pub(crate) const ARCH: u8 = 4;
pub(crate) const CARGO: u8 = 8;
pub(crate) const LINKS: u8 = 16;
pub(crate) const ATP: u8 = 32;
pub(crate) const ALL_LAYERS: u8 = 63;

#[derive(Debug, Clone, Copy, Default)]
struct Dot {
    x: f64,
    y: f64,
    /// Seconds left flying out of the synthase (0: drifting).
    fly: f64,
    vx: f64,
    vy: f64,
}

/// One sample of the swinging head's path (nm, world), when it was taken, whether the pull
/// was fast, and the step it belongs to.
#[derive(Debug, Clone, Copy, Default)]
struct ArchPt {
    x: f64,
    y: f64,
    t: f64,
    fast: bool,
    step: u64,
}

#[derive(Debug)]
struct Motors {
    kinesin: Kinesin,
    rotor: Rotor,
    /// Cargo bead position (nm), lagging on its spring.
    cargo: [f64; 2],
    /// Where the vesicle is drawn: its position low-passed over [`CARGO_VIEW_S`], so its
    /// Brownian jitter shows as a slow wander, not a shell redrawn every frame.
    cargo_view: [f64; 2],
    /// Camera centre (nm), easing after the motor.
    cam: f64,
    cam_target: f64,
    dots: Vec<Dot>,
    /// Sites heads bound to (nm) and when, a ring buffer.
    trail: [(f64, f64); TRAIL],
    trail_len: usize,
    trail_next: usize,
    /// The swinging head's sampled path, a ring buffer; the low-passed head position and
    /// the step it follows; the sub-step counter that paces the samples.
    arch: [ArchPt; ARCH_N],
    arch_len: usize,
    arch_next: usize,
    smooth: Option<(u64, [f64; 2])>,
    substeps: u32,
    time: f64,
    owed: f64,
    act: usize,
    timeline: Timeline,
    /// The back layer (track, cargo, arches, lobes) and the front layer (heads, γ, c-ring,
    /// protons) blitted over it, so a cell the subject crosses shows the subject alone.
    layers: Option<[Dots; 2]>,
    /// The synthase's static α₃β₃ lobes, drawn in `resize` and blitted under the layers.
    lobes: Dots,
    /// Their size in dots (kept apart, as the layers are moved out while drawing).
    dot_size: Size,
    cells: Size,
    readout_room: usize,
    rng: SplitMix64,
    /// The dots are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Motors {
    fn default() -> Self {
        Self {
            dirty: true,
            kinesin: Kinesin::new(),
            rotor: Rotor::new(),
            cargo: [-14.0, CARGO_Y],
            cargo_view: [-14.0, CARGO_Y],
            cam: 0.0,
            cam_target: 0.0,
            dots: Vec::with_capacity(ATP_DOTS),
            trail: [(0.0, 0.0); TRAIL],
            trail_len: 0,
            trail_next: 0,
            arch: [ArchPt::default(); ARCH_N],
            arch_len: 0,
            arch_next: 0,
            smooth: None,
            substeps: 0,
            time: 0.0,
            owed: 0.0,
            act: KINESIN,
            timeline: Timeline::new(&[(CAPTIONS[0], KINESIN_S), (CAPTIONS[1], SYNTHASE_S)]),
            layers: Some([Dots::default(), Dots::default()]),
            lobes: Dots::default(),
            dot_size: Size::new(2, 4),
            cells: Size::new(1, 1),
            readout_room: 0,
            rng: SplitMix64::new(1),
        }
    }
}

/// `[ATP]` at kinesin-act time `t`.
fn atp_at(t: f64) -> f64 {
    ATP_STEPS
        .iter()
        .rev()
        .find(|(from, _)| t >= *from)
        .map_or(ATP_STEPS[0].1, |s| s.1)
}

/// The arch ink of a sample `age` seconds old: `Sun` where the pull was fast, `Hot` where
/// the swing settled, stepping down through three levels as it ages (flat steps, so a
/// fading arch changes its cells only twice).
fn arch_ink(fast: bool, age: f64) -> Ink {
    let step = ((1.0 - age / ARCH_S) * 3.0).ceil().clamp(1.0, 3.0) as u8;
    let level = 105 + 50 * step;
    Ink::new(if fast { Role::Sun } else { Role::Hot }, level)
}

/// A per-object salt: `salt` mixed with an index, so identical clouds differ.
fn salted(salt: u64, k: i64) -> u64 {
    salt ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// A Gaussian stipple cloud about `c` (dots): a dot is kept with probability
/// `peak·e^{−r²/2σ²}` out to 2.5σ. The thresholds are keyed to the dot's offset from the
/// rounded centre (CORE-R107), so a moving cloud moves rigidly and never shimmers.
fn cloud(dots: &mut Dots, c: (f64, f64), sigma: f64, peak: f64, salt: u64, ink: Ink) {
    if !(c.0.is_finite() && c.1.is_finite() && sigma > 0.0 && sigma < 1e3) {
        return;
    }
    let (cx, cy) = (c.0.round() as i32, c.1.round() as i32);
    let reach = (2.5 * sigma).ceil() as i32;
    let (k, cut) = (-0.5 / (sigma * sigma), (2.5 * sigma).powi(2));
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let r2 = f64::from(dx * dx + dy * dy);
            if r2 <= cut && raster::stipple_keep(salt, dx, dy, peak * (k * r2).exp()) {
                dots.set(cx.saturating_add(dx), cy.saturating_add(dy), ink);
            }
        }
    }
}

/// A teardrop stipple cloud of length ≈ `size` (dots) from its pointed `tip` outward along
/// the unit `dir`: across the axis a Gaussian whose width grows from the tip, along it a
/// Gaussian peaked 60 % of the way out, so the round end faces away from the tip. Keyed to
/// the offset from the rounded tip, like [`cloud`].
fn teardrop(dots: &mut Dots, tip: (f64, f64), dir: (f64, f64), size: f64, salt: u64, ink: Ink) {
    if !(tip.0.is_finite() && tip.1.is_finite() && size > 0.0 && size < 1e4) {
        return;
    }
    let len = 0.95 * size;
    let (tx, ty) = (tip.0.round() as i32, tip.1.round() as i32);
    // The bounding box of the rotated rectangle along × ±3 widths, so only candidate dots
    // are visited.
    let side = 3.0 * (0.17 * size + 0.5);
    let corners = [(0.0, -side), (0.0, side), (len, -side), (len, side)]
        .map(|(a, b)| (a * dir.0 + b * dir.1, a * dir.1 - b * dir.0));
    let span = |k: fn(&(f64, f64)) -> f64| {
        let lo = corners.iter().map(k).fold(f64::INFINITY, f64::min).floor() as i32;
        let hi = corners
            .iter()
            .map(k)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i32;
        lo..=hi
    };
    let (xs, ys) = (span(|p| p.0), span(|p| p.1));
    for dy in ys {
        for dx in xs.clone() {
            let (px, py) = (f64::from(dx), f64::from(dy));
            let along = px * dir.0 + py * dir.1;
            if !(0.0..len).contains(&along) {
                continue;
            }
            let across = px * dir.1 - py * dir.0;
            let f = along / len;
            // f^0.75: the width grows from the pointed tip.
            let width = 0.17 * size * (f * f.sqrt()).sqrt() + 0.5;
            let a = (f - 0.6) / 0.28;
            let p = 0.5 * (-0.5 * (a * a + (across / width).powi(2))).exp();
            if raster::stipple_keep(salt, dx, dy, p) {
                dots.set(tx.saturating_add(dx), ty.saturating_add(dy), ink);
            }
        }
    }
}

/// The synthase's centre and α₃β₃ radius (dots) in an art area of `(dw, dh)` dots.
fn synthase_frame((dw, dh): (f64, f64)) -> ((f64, f64), f64) {
    ((dw / 2.0, dh / 2.0), (dh * 0.24).min(dw * 0.22))
}

/// The α₃β₃ hexamer: six teardrops pointed at the centre, α `Hot` and β `Cold`
/// alternating. Static, so drawn once per resize into its own layer.
fn draw_lobes(dots: &mut Dots, art: (f64, f64)) {
    let (c, r) = synthase_frame(art);
    dots.clear();
    for j in 0..6i32 {
        let ink = if j % 2 == 0 { ALPHA } else { BETA };
        let deg = f64::from(j) * 60.0 + 30.0;
        let (x, y) = gamma_dir(deg);
        let tip = (c.0 + 0.42 * r * x, c.1 + 0.42 * r * y);
        teardrop(dots, tip, (x, y), r, salted(SALT_LOBE, i64::from(j)), ink);
    }
}

/// A hollow stipple shell about `c` (dots): dots with `r_in ≤ r ≤ r_out` kept with
/// probability `p`, keyed to the offset from the rounded centre like [`cloud`].
fn shell(dots: &mut Dots, c: (f64, f64), (r_in, r_out): (f64, f64), p: f64, ink: Ink) {
    if !(c.0.is_finite() && c.1.is_finite() && r_out > 0.0 && r_out < 1e4) {
        return;
    }
    let (cx, cy) = (c.0.round() as i32, c.1.round() as i32);
    let reach = r_out.ceil() as i32;
    let (lo, hi) = (r_in * r_in, r_out * r_out);
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let r2 = f64::from(dx * dx + dy * dy);
            if (lo..=hi).contains(&r2) && raster::stipple_keep(SALT_SHELL, dx, dy, p) {
                dots.set(cx.saturating_add(dx), cy.saturating_add(dy), ink);
            }
        }
    }
}

/// Screen direction of γ at rotor angle `deg`, viewed from the membrane (Fo) side with
/// y down: 0° points up and the angle grows clockwise on screen, the synthesis sense.
pub(crate) fn gamma_dir(deg: f64) -> (f64, f64) {
    let (s, c) = (deg.to_radians() - PI / 2.0).sin_cos_det();
    (c, s)
}

/// `a + (b − a)·t` on points.
fn mix(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

impl Motors {
    fn local(&self) -> f64 {
        self.time % self.timeline.total()
    }

    /// Dot extent of the art (the bottom row holds the title and readout).
    fn art(&self) -> (f64, f64) {
        let ds = self.dot_size;
        (
            f64::from(ds.width()),
            (f64::from(ds.height()) - 4.0).max(1.0),
        )
    }

    /// Dots per nm in the kinesin act: 64 nm across and 38 nm down fit, so the vesicle
    /// (up to 27 nm above the track) and the microtubule (10 nm below) stay on screen.
    fn scale(&self) -> f64 {
        let (dw, dh) = self.art();
        (dw / 64.0).min(dh / 38.0).max(0.8)
    }

    /// The kinesin act's world → dot map: x about the camera, y from the track surface,
    /// which sits 10.5 nm above the bottom of the art.
    fn kinesin_dot(&self, x: f64, y: f64) -> (f64, f64) {
        let (dw, dh) = self.art();
        let s = self.scale();
        (dw / 2.0 + (x - self.cam) * s, dh - 10.5 * s + y * s)
    }

    fn scatter_dots(&mut self, n: usize) {
        self.dots.clear();
        for _ in 0..n.min(ATP_DOTS) {
            let (x, y) = (
                self.rng.random_range(-1.0..1.0),
                self.rng.random_range(-1.0..1.0),
            );
            self.dots.push(Dot {
                x,
                y,
                ..Dot::default()
            });
        }
    }

    fn dots_for(atp: f64) -> usize {
        ((atp.log10() - 0.8) * 30.0).clamp(8.0, ATP_DOTS as f64) as usize
    }

    /// Follow the swinging head's path (KIN-A11): low-passed over [`ARCH_TAU`] so the arch
    /// shows the docking pull rather than every thermal kick, clamped to the stretch between
    /// its old and new sites and above the track surface (a head cannot enter the
    /// microtubule), and sampled every [`ARCH_EVERY`] sub-steps.
    fn track_arch(&mut self, h: f64) {
        let Some((i, p)) = self.kinesin.free else {
            self.smooth = None;
            return;
        };
        let step = self.kinesin.steps;
        let s = match self.smooth {
            Some((at, s)) if at == step => {
                let a = (h / ARCH_TAU).min(1.0);
                [s[0] + (p[0] - s[0]) * a, s[1] + (p[1] - s[1]) * a]
            }
            _ => p,
        };
        self.smooth = Some((step, s));
        self.substeps = self.substeps.wrapping_add(1);
        if self.substeps % ARCH_EVERY != 0 {
            return;
        }
        let lead = self.kinesin.site[1 - i];
        let (from, to) = ((lead - 1) as f64 * SITE_NM, (lead + 1) as f64 * SITE_NM);
        self.arch[self.arch_next] = ArchPt {
            x: s[0].clamp(from, to),
            y: s[1].min(0.0),
            t: self.time,
            fast: SWING_K * (to - s[0]).abs() >= ARCH_FAST,
            step,
        };
        self.arch_next = (self.arch_next + 1) % ARCH_N;
        self.arch_len = (self.arch_len + 1).min(ARCH_N);
    }

    fn step_model(&mut self, h: f64) {
        let local = self.local();
        let act = self.timeline.at(local).index;
        if act != self.act {
            self.act = act;
            if act == KINESIN {
                self.kinesin = Kinesin::new();
                self.cargo = [-14.0, CARGO_Y];
                self.cargo_view = self.cargo;
                self.cam = 0.0;
                self.cam_target = 0.0;
                self.trail_len = 0;
                self.arch_len = 0;
                self.scatter_dots(Self::dots_for(atp_at(0.0)));
            } else {
                self.rotor = Rotor::new();
                self.dots.clear();
            }
        }
        if act == KINESIN {
            let atp = atp_at(local);
            let want = Self::dots_for(atp);
            if self.dots.len() > want {
                self.dots.truncate(want);
            }
            let mut rng = self.rng.clone();
            let before = self.kinesin.steps;
            self.kinesin.step(atp, &mut rng);
            self.rng = rng;
            if self.kinesin.steps != before
                && let Some(i) = self.kinesin.last_moved
            {
                self.trail[self.trail_next] = (self.kinesin.site[i] as f64 * SITE_NM, self.time);
                self.trail_next = (self.trail_next + 1) % TRAIL;
                self.trail_len = (self.trail_len + 1).min(TRAIL);
            }
            self.track_arch(h);
            // The cargo lags behind on its tether.
            let c = self.kinesin.centre();
            self.cargo[0] += (c - 16.0 - self.cargo[0]) * (2.0 * h).min(1.0);
            self.cargo[1] += (CARGO_Y - self.cargo[1]) * (2.0 * h).min(1.0);
            self.cargo[0] += gaussian(&mut self.rng, 0.0, 0.15);
            let a = (h / CARGO_VIEW_S).min(1.0);
            for (v, c) in self.cargo_view.iter_mut().zip(self.cargo) {
                *v += (c - *v) * a;
            }
            // A lazy camera: the view holds still while the motor walks across it (so the
            // lattice does not slide under every step), and once the motor strays a third
            // of the view from the centre it re-centres in a short eased pan, leading the
            // walk so the motor enters from the left again.
            let zone = (self.art().0 / self.scale() / 3.0).min(24.0);
            if (c - self.cam_target).abs() > zone {
                self.cam_target = c + 0.25 * zone;
            }
            let gap = self.cam_target - self.cam;
            self.cam = if gap.abs() < 0.02 {
                self.cam_target
            } else {
                self.cam + gap * (6.0 * h).min(1.0)
            };
        } else {
            let mut rng = self.rng.clone();
            let made = self.rotor.step(h, &mut rng);
            self.rng = rng;
            if made && self.dots.len() < ATP_DOTS {
                // A fresh ATP leaves the catalytic site the rotor just passed.
                let (c, s) = gamma_dir(self.rotor.angle - 30.0);
                self.dots.push(Dot {
                    x: 0.55 * c,
                    y: 0.55 * s,
                    fly: 1.5,
                    vx: 0.5 * c,
                    vy: 0.5 * s,
                });
            }
        }
        for d in &mut self.dots {
            if d.fly > 0.0 {
                d.fly -= h;
                d.x += d.vx * h;
                d.y += d.vy * h;
            } else {
                d.x += gaussian(&mut self.rng, 0.0, ATP_STEP);
                d.y += gaussian(&mut self.rng, 0.0, ATP_STEP);
            }
            d.x = (d.x + 1.0).rem_euclid(2.0) - 1.0;
            d.y = (d.y + 1.0).rem_euclid(2.0) - 1.0;
        }
    }

    /// Seconds since a head bound to the site at `x` nm, if it is among the marks.
    fn mark_age(&self, x: f64) -> Option<f64> {
        self.trail[..self.trail_len]
            .iter()
            .filter(|m| m.0 == x)
            .map(|m| self.time - m.1)
            .reduce(f64::min)
    }

    /// The kinesin act (SPEC-002 Look (M17)).
    fn draw_kinesin(&self, back: &mut Dots, front: &mut Dots, layers: u8) {
        let (dw, dh) = self.art();
        let s = self.scale();
        let at = |x: f64, y: f64| self.kinesin_dot(x, y);
        let track_y = at(0.0, 0.0).1;
        // Clouds keep a few dots across even on small terminals.
        let sigma = |nm: f64| (nm * s).max(1.4);
        if layers & ATP != 0 {
            // ATP diffusing in solution, never inside the microtubule.
            for d in &self.dots {
                let (x, y) = (dw * (d.x + 1.0) / 2.0, dh * (d.y + 1.0) / 2.0);
                if !(track_y + 0.5 * s..track_y + 13.0 * s).contains(&y) {
                    back.plot(x, y, ATP_INK);
                }
            }
        }
        if layers & TUBE != 0 {
            // The microtubule side-on as a fine braille lattice: three thin protofilament
            // rows of tubulin monomers 4 nm apart, each a short 1-dot dash with a gap to the
            // next, each row shifted along the helix; β (`Cold`, under each 8 nm binding site
            // in the top row) and α (`Structure`) alternate, and a site a head has just bound
            // glows brighter and a dot longer.
            let first = ((self.cam - dw / 2.0 / s) / 4.0).floor() as i64 - 2;
            let last = ((self.cam + dw / 2.0 / s) / 4.0).ceil() as i64 + 2;
            let half = (0.5 * s).clamp(0.5, 2.5);
            for row in 0..3i64 {
                let y = 2.6 + 3.2 * row as f64;
                if at(0.0, y).1 > dh {
                    break;
                }
                for k in first..=last {
                    let x = 4.0 * k as f64 + 1.3 * row as f64;
                    let beta = k.rem_euclid(2) == 0;
                    let (ink, reach) = match (beta, row) {
                        (false, _) => (TUB_ALPHA, half),
                        (true, 0) => match self.mark_age(x) {
                            Some(age) if age < TRAIL_S => (TUB_MARK, half + 1.0),
                            _ => (TUB_BETA, half),
                        },
                        (true, _) => (TUB_BETA, half),
                    };
                    let p = at(x, y);
                    back.line(p.0 - reach, p.1, p.0 + reach, p.1, ink);
                }
            }
        }
        if layers & ARCH != 0 {
            // Each sample joins the previous one of the same step; the thread steps down in
            // brightness as it ages and is gone after ARCH_S.
            for j in 1..self.arch_len {
                let idx = |j: usize| (self.arch_next + ARCH_N - self.arch_len + j) % ARCH_N;
                let (a, b) = (self.arch[idx(j - 1)], self.arch[idx(j)]);
                let age = self.time - b.t;
                if a.step != b.step || !(0.0..ARCH_S).contains(&age) {
                    continue;
                }
                let ink = arch_ink(b.fast, age);
                let (p, q) = (at(a.x, a.y), at(b.x, b.y));
                back.line(p.0, p.1, q.0, q.1, ink);
            }
        }
        let cargo = at(self.cargo_view[0], self.cargo_view[1]);
        let r = CARGO_R * s;
        if layers & CARGO != 0 {
            // A vesicle: its membrane a sparse stipple shell, the lumen black.
            shell(back, cargo, (0.84 * r, r), 0.3, SHELL);
        }
        let k = self.kinesin;
        let heads = [k.head(0), k.head(1)];
        if layers & LINKS != 0 {
            // Neck linkers to the hip, and the coiled-coil stalk to the vesicle.
            let hip = at(
                0.5 * (heads[0][0] + heads[1][0]),
                0.5 * (heads[0][1] + heads[1][1]) - 6.5,
            );
            for h in heads {
                let p = at(h[0], h[1]);
                back.line(hip.0, hip.1, p.0, p.1, LINK);
            }
            let (ax, ay) = (cargo.0 + 0.55 * r, cargo.1 + 0.72 * r);
            back.line(hip.0, hip.1, ax, ay, LINK);
        }
        if layers & HEADS != 0 {
            for (i, h) in heads.iter().enumerate() {
                let bound = k.free.is_none_or(|(f, _)| f != i);
                let ink = if bound { BOUND } else { SWING };
                let salt = salted(SALT_HEAD, i as i64);
                cloud(front, at(h[0], h[1]), sigma(0.95), 0.75, salt, ink);
            }
        }
    }

    /// The synthase act, seen from the membrane (Fo) side as in Noji's films: six teardrop
    /// α₃β₃ lobes around a clear centre, the c-ring a stippled annulus whose ten subunits
    /// turn with γ, γ a short `Accent` thread at the centre, and protons crossing the
    /// a-subunit's half-channels in the gap below.
    fn draw_synthase(&self, back: &mut Dots, front: &mut Dots) {
        let (dw, dh) = self.art();
        let (c, r) = synthase_frame((dw, dh));
        let around = |deg: f64, radius: f64| {
            let (x, y) = gamma_dir(deg);
            (c.0 + radius * x, c.1 + radius * y)
        };
        // The α₃β₃ lobes never move: they are drawn once per resize ([`draw_lobes`]).
        let phi = self.rotor.shown();
        // The c-ring: an annulus whose density peaks on its ten c-subunits, turning with γ.
        let (r0, r1) = (0.2 * r, 0.32 * r);
        let reach = r1.ceil() as i32;
        let (cx, cy) = (c.0.round() as i32, c.1.round() as i32);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let d = f64::from(dx).hypot(f64::from(dy));
                if !(r0..=r1).contains(&d) {
                    continue;
                }
                // Angle on screen in the γ convention (0° up, clockwise), in degrees.
                let deg = f64::from(dx).atan2(-f64::from(dy)).to_degrees();
                let k = (5.0 * (deg - phi)).to_radians().cos_det();
                let p = 0.1 + 0.75 * k * k;
                if raster::stipple_keep(SALT_CRING, dx, dy, p) {
                    front.set(cx + dx, cy + dy, C_RING);
                }
            }
        }
        // γ inside the ring; while it turns, fading ghosts sweep behind it.
        if let Some(t) = self.rotor.turning {
            for lag in [10.0, 20.0, 30.0] {
                let g = around(phi - lag * (1.0 - t / TURN_S), 0.24 * r);
                front.line(c.0, c.1, g.0, g.1, GHOST);
            }
        }
        let tip = around(phi, 0.24 * r);
        front.line(c.0, c.1, tip.0, tip.1, GAMMA);
        // The a-subunit in the gap between the lowest lobes: protons ride in along one
        // half-channel to the ring and out along the other, one per 36° of rotation.
        let a = around(180.0, 0.34 * r);
        let (inlet, outlet) = (around(173.0, 1.25 * r), around(187.0, 1.25 * r));
        back.line(inlet.0, inlet.1, a.0, a.1, CHANNEL);
        back.line(a.0, a.1, outlet.0, outlet.1, CHANNEL);
        let frac = phi.rem_euclid(36.0) / 36.0;
        for p in [mix(inlet, a, frac), mix(a, outlet, frac)] {
            front.disc(p.0, p.1, 1.4, PROTON);
        }
        for d in &self.dots {
            front.plot(c.0 + d.x * dw * 0.5, c.1 + d.y * dh * 0.5, ATP_INK);
        }
    }

    fn rasterise(&mut self) {
        // Taking the layers out of their `Option` moves them without allocating, so the
        // drawing code can read the rest of the scene.
        let Some([mut back, mut front]) = self.layers.take() else {
            return;
        };
        back.clear();
        front.clear();
        if self.act == KINESIN {
            self.draw_kinesin(&mut back, &mut front, ALL_LAYERS);
        } else {
            self.draw_synthase(&mut back, &mut front);
        }
        self.layers = Some([back, front]);
    }
}

impl Generator for Motors {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.local()).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cells = size;
        self.readout_room = title::stable_readout_room(size, &META);
        let layers = self.layers.get_or_insert_with(Default::default);
        for layer in layers.iter_mut() {
            layer.resize(size);
        }
        self.dot_size = layers[0].dot_size();
        self.lobes.resize(size);
        let art = self.art();
        draw_lobes(&mut self.lobes, art);
        if self.dots.is_empty() && self.time == 0.0 {
            self.rng = SplitMix64::from_rng(&mut ctx.rng);
            self.scatter_dots(Self::dots_for(atp_at(0.0)));
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.owed += dt / H;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        for _ in 0..n as u32 {
            self.time += H;
            self.step_model(H);
        }
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
        // ASCII falls back by dot count (`Dots::blit`).
        if self.act != KINESIN {
            self.lobes.blit(canvas);
        }
        for layer in self.layers.iter().flatten() {
            layer.blit(canvas);
        }
        let h = i32::from(self.cells.height());
        if h < 4 {
            return;
        }
        let mut label = StackText::<32>::new();
        if self.act == KINESIN {
            let atp = atp_at(self.local());
            let k = rate(atp);
            let _ = match self.readout_room {
                24.. => write!(label, "ATP {atp:>4.0} uM  {k:>3.0} steps/s"),
                9.. => write!(label, "ATP {atp:>4.0}"),
                _ => return,
            };
        } else {
            let turns = self.rotor.turns;
            let _ = match self.readout_room {
                16.. => write!(label, "ATP made {turns:>4}"),
                5.. => write!(label, "{turns:>4}"),
                _ => return,
            };
        }
        draw::label_right(canvas, h - 1, label.as_str(), LABEL_INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kin_a1_mean_dwell_matches_michaelis_menten() {
        let mut rng = SplitMix64::new(1);
        for atp in [1000.0, 20.0] {
            let k = rate(atp);
            let mean = (0..2000).map(|_| dwell(&mut rng, k)).sum::<f64>() / 2000.0;
            assert!(
                (mean * k - 1.0).abs() < 0.1,
                "[ATP] {atp}: mean {mean}, 1/k {}",
                1.0 / k
            );
        }
        assert!(rate(1000.0) > 2.0 * rate(20.0));
    }

    #[test]
    fn kin_a2_steps_are_8_nm_and_hand_over_hand() {
        let mut rng = SplitMix64::new(2);
        let mut k = Kinesin::new();
        let (mut last_centre, mut last_steps, mut movers) = (k.centre(), 0, Vec::new());
        for _ in 0..(240 * 60) {
            k.step(1000.0, &mut rng);
            if k.steps != last_steps {
                assert_eq!(k.centre() - last_centre, SITE_NM, "one 8 nm step");
                assert_eq!(
                    (k.site[0] - k.site[1]).abs(),
                    1,
                    "heads on neighbouring sites"
                );
                movers.push(k.last_moved.unwrap_or(9));
                last_centre = k.centre();
                last_steps = k.steps;
            }
        }
        assert!(movers.len() >= 20, "{} steps", movers.len());
        assert!(
            movers.windows(2).all(|w| w[0] != w[1]),
            "heads alternate: {movers:?}"
        );
    }

    #[test]
    fn kin_a3_rotor_turns_forward_in_80_40_substeps() {
        let mut rng = SplitMix64::new(3);
        let mut r = Rotor::new();
        let (mut shown, mut committed) = (r.shown(), vec![r.angle]);
        for _ in 0..(240 * 30) {
            r.step(H, &mut rng);
            assert!(r.shown() >= shown, "never turns back");
            shown = r.shown();
            if *committed.last().unwrap_or(&0.0) != r.angle {
                committed.push(r.angle);
            }
        }
        let steps: Vec<f64> = committed.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.len() >= 6);
        for (i, s) in steps.iter().enumerate() {
            assert_eq!(*s, if i % 2 == 0 { 80.0 } else { 40.0 });
        }
        assert_eq!(r.turns, (committed.len() as u64 - 1) / 2);
    }

    #[test]
    fn kin_a4_bound_jiggle_has_no_drift() {
        let mut rng = SplitMix64::new(4);
        let (mut x, mut sum) = (0.0, 0.0);
        let n = 200_000;
        for _ in 0..n {
            x = langevin(x, 0.0, WELL_K, WELL_D * 0.02, H, &mut rng);
            sum += x;
        }
        assert!(
            (sum / f64::from(n)).abs() < 0.02,
            "mean {}",
            sum / f64::from(n)
        );
    }

    #[test]
    fn kin_a6_gamma_turns_clockwise_seen_from_the_membrane() {
        // Screen y points down, so a positive cross product of successive directions is
        // a clockwise turn on screen (viewed from the Fo side: the synthesis sense).
        for k in 0..9 {
            let (a, b) = (
                gamma_dir(f64::from(k) * 40.0),
                gamma_dir(f64::from(k + 1) * 40.0),
            );
            assert!(a.0 * b.1 - a.1 * b.0 > 0.5, "clockwise at {}°", k * 40);
        }
        assert!(gamma_dir(0.0).1 < -0.99, "0° points up");
    }

    #[test]
    fn kin_a7_each_step_leaves_a_footprint_at_its_site() {
        let mut ctx = Ctx::new(7, Size::new(120, 40));
        let mut g = Motors::default();
        g.resize(Size::new(120, 40), &mut ctx);
        g.prepare();
        let mut seen = 0;
        while g.time < 8.0 {
            let before = g.kinesin.steps;
            g.update(1.0 / 60.0, &mut ctx);
            g.prepare();
            let new = (g.kinesin.steps - before) as usize;
            seen += new;
            if new > 0 {
                let last = g.trail[(g.trail_next + TRAIL - 1) % TRAIL];
                let i = g.kinesin.last_moved.unwrap_or(0);
                assert_eq!(last.0, g.kinesin.site[i] as f64 * SITE_NM);
            }
        }
        assert!(seen >= 3, "{seen} steps");
        assert_eq!(g.trail_len, seen.min(TRAIL));
    }

    #[test]
    fn kin_a5_captions_follow_the_acts() {
        let mut ctx = Ctx::new(5, Size::new(80, 24));
        let mut g = Motors::default();
        g.resize(Size::new(80, 24), &mut ctx);
        g.prepare();
        assert_eq!(g.caption(), Some("kinesin walks"));
        while g.time < KINESIN_S + 1.0 {
            g.update(0.25, &mut ctx);
            g.prepare();
        }
        assert_eq!(g.caption(), Some("synthase turns"));
        while g.time < KINESIN_S + SYNTHASE_S + 1.0 {
            g.update(0.25, &mut ctx);
            g.prepare();
        }
        assert_eq!(g.caption(), Some("kinesin walks"));
    }

    fn scene(size: Size, seed: u64) -> (Motors, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = Motors::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    /// The kinesin act's `layers` alone, back and front merged, as lit dot coordinates.
    fn kinesin_layer(g: &Motors, layers: u8) -> Vec<(i32, i32)> {
        let (mut a, mut b) = (Dots::new(g.cells), Dots::new(g.cells));
        g.draw_kinesin(&mut a, &mut b, layers);
        let ds = a.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if a.is_lit(x, y) || b.is_lit(x, y) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn kin_a9_no_half_block_cells_in_either_act() {
        for size in [Size::new(120, 40), Size::new(80, 24), Size::new(200, 50)] {
            let mut s = Stage::new(Box::new(Motors::default()), 9, size, GlyphMode::Unicode);
            let mut steps = 0;
            for t in [3.0, 12.0, 20.0, KINESIN_S + 3.0, KINESIN_S + 10.0] {
                while f64::from(steps) * SIM_DT < t {
                    s.step(SIM_DT);
                    steps += 1;
                }
                let frame = s.render();
                let lit = frame.cells().iter().filter(|c| c.ch != ' ').count();
                assert!(lit > 50, "{size:?} t {t}: {lit} lit cells");
                for c in frame.cells() {
                    assert_eq!(c.bg, Ink::BLANK, "{size:?} t {t}");
                    assert!(!matches!(c.ch, '▀' | '▄' | '█'), "{size:?} t {t}");
                }
            }
        }
    }

    #[test]
    fn kin_a10_bound_head_cold_swinging_head_hot_throughout_a_step() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = scene(size, 10);
        let mut checked = 0;
        while g.time < 10.0 {
            g.update(SIM_DT, &mut ctx);
            let Some((free, _)) = g.kinesin.free else {
                continue;
            };
            // The swinging head passes over the bound one and is drawn on top: a cell
            // holding any of its dots is `Hot`, a cell holding only the bound head's `Cold`.
            let heads = |g: &Motors| {
                let mut front = Dots::new(size);
                g.draw_kinesin(&mut Dots::new(size), &mut front, HEADS);
                front
            };
            let both = heads(&g);
            let bound = 1 - free;
            let saved = g.kinesin.jig[bound][0];
            g.kinesin.jig[bound][0] = 1e6;
            let swing_only = heads(&g);
            g.kinesin.jig[bound][0] = saved;
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            both.blit(&mut canvas);
            for cy in 0..40 {
                for cx in 0..120 {
                    let Some(mask) = both.cell_mask(cx, cy).filter(|&m| m != 0) else {
                        continue;
                    };
                    let ink = canvas.get(cx, cy).unwrap().ink;
                    let swinging = swing_only.cell_mask(cx, cy).unwrap_or(0) != 0;
                    let want = if swinging { SWING } else { BOUND };
                    assert_eq!(ink, want, "t {} cell ({cx}, {cy}) mask {mask:#x}", g.time);
                    checked += 1;
                }
            }
        }
        assert!(checked > 200, "{checked} head cells checked");
        assert_eq!((BOUND.role, SWING.role), (Role::Cold, Role::Hot));
    }

    #[test]
    fn kin_a11_steps_write_arches_above_the_track_that_fade() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = scene(size, 11);
        let mut checked = 0;
        while g.time < 8.0 {
            g.update(SIM_DT, &mut ctx);
            let Some((free, _)) = g.kinesin.free else {
                continue;
            };
            // During a step every arch dot of this step lies above the track and between
            // the head's old and new sites (within a dot of rounding).
            let lead = g.kinesin.site[1 - free];
            let (x0, track) = g.kinesin_dot((lead - 1) as f64 * SITE_NM, 0.0);
            let (x1, _) = g.kinesin_dot((lead + 1) as f64 * SITE_NM, 0.0);
            let step = g.kinesin.steps;
            let mut only = g.arch;
            for p in &mut only {
                if p.step != step {
                    p.t = f64::NEG_INFINITY;
                }
            }
            let saved = std::mem::replace(&mut g.arch, only);
            for (x, y) in kinesin_layer(&g, ARCH) {
                let (x, y) = (f64::from(x), f64::from(y));
                assert!(y <= track + 1.0, "arch below the track at t {}", g.time);
                assert!((x0 - 1.0..=x1 + 1.0).contains(&x), "arch off the step");
                checked += 1;
            }
            g.arch = saved;
        }
        assert!(checked > 50, "{checked} arch dots");
        // Stepping stops: 3 s later the trace is gone.
        while g.kinesin.free.is_some() {
            g.update(SIM_DT, &mut ctx);
        }
        g.kinesin.wait = 1e9;
        assert!(!kinesin_layer(&g, ARCH).is_empty(), "an arch to fade");
        let stop = g.time;
        while g.time < stop + 3.0 {
            g.kinesin.wait = 1e9;
            g.update(SIM_DT, &mut ctx);
        }
        assert!(
            kinesin_layer(&g, ARCH).is_empty(),
            "arches still lit after 3 s"
        );
    }

    #[test]
    fn kin_a12_the_cargo_is_hollow() {
        for size in [Size::new(120, 40), Size::new(200, 50), Size::new(80, 24)] {
            let (mut g, mut ctx) = scene(size, 12);
            for t in [2.0, 9.0] {
                while g.time < t {
                    g.update(SIM_DT, &mut ctx);
                }
                let c = g.kinesin_dot(g.cargo_view[0], g.cargo_view[1]);
                let r = CARGO_R * g.scale();
                let shell = kinesin_layer(&g, CARGO);
                assert!(shell.len() > 20, "{size:?}: {} shell dots", shell.len());
                for (x, y) in shell {
                    let d = (f64::from(x) - c.0).hypot(f64::from(y) - c.1);
                    assert!(d >= 0.6 * r, "{size:?}: dot {d} from the centre, r {r}");
                }
            }
        }
    }
}
