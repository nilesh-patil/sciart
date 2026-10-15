//! A snow crystal grows from vapour on a hexagonal lattice (SPEC-002 `snowflake_growth`).
//!
//! Model: Reiter's local cellular automaton (C. A. Reiter, *A local cellular model for
//! snow crystal growth*, Chaos, Solitons & Fractals 23, 2005). Each site has water `s`;
//! a site is *receptive* if it is frozen (`s ≥ 1`) or touches a frozen site. Per step,
//! receptive sites keep their water and gain `γ` (vapour addition); the water of the
//! other sites, `u`, diffuses: `u ← u + α/2·(⟨u⟩₆ − u)` over all sites (receptive sites
//! hold `u = 0`, so they absorb what flows in); then `s = u + v` and sites with `s ≥ 1`
//! freeze. The surroundings are held at the humidity `β`. Each loop takes `(α, β, γ)`
//! from a table of forms: a compact plate, a sectored plate and a stellar dendrite.
//!
//! The lattice is a fixed disc of radius `R` (lattice units, independent of the
//! terminal), stored as one 1/12 wedge: every site of the disc maps to its canonical
//! image under the hexagon's twelve symmetries, and each wedge site keeps its six
//! neighbours' canonical indices. The crystal is therefore exactly six-fold symmetric
//! (SNOW-A1) and a step costs a twelfth of the full lattice.
//!
//! Look (GEN-R25): the growth rings as 1-dot isochrones. Freeze time is banded into
//! a few rings per form (9 for the plate, 4–5 for the busier forms) coloured in flat steps (`Cold` core → `Wave` → `Particle` newest rims,
//! levels alternating per ring); a frozen dot is lit where a neighbouring dot lies in an
//! older ring or outside the crystal, so each ring's inner edge and the outline are thin
//! lines at any scale and a plate reads as nested hexagons, a dendrite as outlined
//! fern-like arms, rather than a filled area. Every dot is sampled from the hexagonal
//! cell that holds it (cube rounding), so the lattice draws no moiré and stays round on
//! the square braille sub-cells. Vapour is very sparse `Dim` stipple, fixed thresholds,
//! density `∝ s`, fading out before the lattice's edge.

use sciart_core::prelude::*;
use sciart_core::raster::stipple_keep;

pub static META: Meta = Meta {
    name: "snowflake_growth",
    category: Category::PhysicalChemistry,
    description: "A snow crystal grows from vapour on a hexagonal lattice (Reiter model), showing growth rings",
    title: "Snowflake · tips grow fastest",
    captions: &["plate", "sectored plate", "stellar dendrite"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Snowflake::default())
}

/// Lattice disc radius (lattice spacings).
pub(crate) const RADIUS: f64 = 96.0;
/// Growth stops when a frozen site reaches this share of the radius.
const CAP: f64 = 0.75;

/// One crystal form: Reiter's `(α, β, γ)`, and the CA steps per simulated second that
/// make it reach the cap in about `GROW` seconds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Form {
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
    pub steps_per_s: f64,
    /// Growth rings drawn over the nominal growth (fewer for the busier forms, so the
    /// isochrones do not fill them).
    pub rings: u32,
}

/// The forms, picked from previews of Reiter's `(β, γ)` plane at `α = 1`: high humidity
/// and vapour addition give a compact plate; `β = 0.6, γ = 0.05` a plate whose tips
/// outrun its faces (concave sectors with ridges); low humidity and little addition,
/// `β = 0.35, γ = 0.0001`, a fern-like stellar dendrite with side branches all along its
/// arms. Tuned from previews on this lattice, not read off Reiter's published diagram.
pub(crate) const FORMS: [Form; 3] = [
    // plate
    Form {
        alpha: 1.0,
        beta: 0.95,
        gamma: 0.05,
        steps_per_s: 5.0,
        rings: 9,
    },
    // sectored plate
    Form {
        alpha: 1.0,
        beta: 0.6,
        gamma: 0.05,
        steps_per_s: 20.0,
        rings: 5,
    },
    // stellar dendrite
    Form {
        alpha: 1.0,
        beta: 0.35,
        gamma: 0.0001,
        steps_per_s: 160.0,
        rings: 4,
    },
];

/// Cube coordinates `(a, b, c)`, `a + b + c = 0`.
type Cube = (i32, i32, i32);

/// Canonical image of `p` under the hexagon's twelve symmetries (the coordinate
/// permutations and the inversion): sorted descending with a non-positive middle term.
fn canonical(p: Cube) -> Cube {
    let mut v = [p.0, p.1, p.2];
    v.sort_unstable_by(|a, b| b.cmp(a));
    if v[1] > 0 {
        v = [-v[2], -v[1], -v[0]];
    }
    (v[0], v[1], v[2])
}

/// The twelve images of `p` (with repeats on the mirror lines).
fn images(p: Cube) -> [Cube; 12] {
    let (a, b, c) = p;
    let perms = [
        (a, b, c),
        (b, c, a),
        (c, a, b),
        (a, c, b),
        (c, b, a),
        (b, a, c),
    ];
    let mut out = [(0, 0, 0); 12];
    for (i, &(x, y, z)) in perms.iter().enumerate() {
        out[2 * i] = (x, y, z);
        out[2 * i + 1] = (-x, -y, -z);
    }
    out
}

/// Plane position of cube coordinate `p` (neighbour spacing 1, x to the right, y down).
fn position(p: Cube) -> (f64, f64) {
    let (q, r) = (f64::from(p.0), f64::from(p.2));
    (q + 0.5 * r, r * 3f64.sqrt() / 2.0)
}

const DIRS: [Cube; 6] = [
    (1, -1, 0),
    (1, 0, -1),
    (0, 1, -1),
    (-1, 1, 0),
    (-1, 0, 1),
    (0, -1, 1),
];

/// The wedge lattice: sites, neighbours, orbit sizes.
#[derive(Debug, Clone)]
pub(crate) struct Lattice {
    /// Canonical cube coordinates per wedge site.
    pub cube: Vec<Cube>,
    /// Neighbour indices; `n` (one past the last site) is the outside, held at `β`.
    pub nb: Vec<[u32; 6]>,
    /// Number of disc sites each wedge site stands for.
    // Read by the frozen-count, perimeter and conservation checks (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub weight: Vec<u32>,
    /// Wedge index by `(i, j)` (`[i·(span + 1) + j]`, `u32::MAX` outside the disc).
    index: Vec<u32>,
    span: i32,
}

impl Lattice {
    pub(crate) fn new(radius: f64) -> Self {
        let r2 = radius * radius;
        let inside = |p: Cube| {
            let (x, y) = position(p);
            x * x + y * y <= r2
        };
        // Wedge sites (i + j, −j, −i), 0 ≤ j ≤ i, by hex distance; index by (i, j).
        let span = (radius * 2.0 / 3f64.sqrt()).ceil() as i32 + 2;
        let mut index = vec![u32::MAX; ((span + 1) * (span + 1)) as usize];
        let slot = |i: i32, j: i32| (i * (span + 1) + j) as usize;
        let mut cube = Vec::new();
        for i in 0..=span {
            for j in 0..=i {
                let p = (i + j, -j, -i);
                if inside(p) {
                    index[slot(i, j)] = cube.len() as u32;
                    cube.push(p);
                }
            }
        }
        let n = cube.len() as u32;
        let lookup = |p: Cube| {
            let (a, b, c) = canonical(p);
            debug_assert_eq!(a + b + c, 0);
            let (i, j) = (-c, -b);
            if i > span || !inside(p) {
                return n;
            }
            index[slot(i, j)]
        };
        let nb = cube
            .iter()
            .map(|&(a, b, c)| DIRS.map(|(da, db, dc)| lookup((a + da, b + db, c + dc))))
            .collect();
        let mut weight = Vec::with_capacity(cube.len());
        for &p in &cube {
            let mut seen: [Cube; 12] = [(i32::MIN, 0, 0); 12];
            let mut count = 0;
            for q in images(p) {
                if !seen[..count].contains(&q) {
                    seen[count] = q;
                    count += 1;
                }
            }
            weight.push(count as u32);
        }
        Self {
            cube,
            nb,
            weight,
            index,
            span,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.cube.len()
    }

    /// Wedge index of the disc site `p`, or `None` outside the disc.
    pub(crate) fn find(&self, p: Cube) -> Option<usize> {
        let (_, b, c) = canonical(p);
        let (i, j) = (-c, -b);
        if i > self.span {
            return None;
        }
        let k = self.index[(i * (self.span + 1) + j) as usize];
        (k != u32::MAX).then_some(k as usize)
    }

    /// Wedge index of the disc site whose hexagonal cell holds plane point `(x, y)`.
    pub(crate) fn site_at(&self, x: f64, y: f64) -> Option<usize> {
        // Fractional cube coordinates, rounded to the nearest site (cube rounding).
        let r = y * 2.0 / 3f64.sqrt();
        let q = x - 0.5 * r;
        let s = -q - r;
        let (mut rq, mut rr, rs) = (q.round(), r.round(), s.round());
        let (dq, dr, ds) = ((rq - q).abs(), (rr - r).abs(), (rs - s).abs());
        if dq > dr && dq > ds {
            rq = -rr - rs;
        } else if dr > ds {
            rr = -rq - rs;
        }
        let (a, c) = (rq as i32, rr as i32);
        if a.abs() > 4 * self.span || c.abs() > 4 * self.span {
            return None;
        }
        self.find((a, -a - c, c))
    }
}

/// Reiter's automaton on the wedge lattice.
#[derive(Debug, Clone)]
pub(crate) struct Crystal {
    pub form: Form,
    /// Water per site, plus the outside slot (always `β`).
    pub s: Vec<f64>,
    pub frozen: Vec<bool>,
    /// The step each site froze at (`u32::MAX`: not frozen).
    pub born: Vec<u32>,
    /// Steps taken.
    pub step: u32,
    /// Largest plane radius of a frozen site.
    pub reach: f64,
    rec: Vec<bool>,
    u: Vec<f64>,
    v: Vec<f64>,
    un: Vec<f64>,
}

impl Crystal {
    pub(crate) fn new(lat: &Lattice, form: Form) -> Self {
        let n = lat.len();
        let mut c = Self {
            form,
            s: vec![form.beta; n + 1],
            frozen: vec![false; n],
            born: vec![u32::MAX; n],
            step: 0,
            reach: 0.0,
            rec: vec![false; n],
            u: vec![0.0; n + 1],
            v: vec![0.0; n],
            un: vec![0.0; n],
        };
        c.reset(form);
        c
    }

    /// Back to vapour at humidity `β` with one frozen seed site.
    pub(crate) fn reset(&mut self, form: Form) {
        self.form = form;
        self.s.fill(form.beta);
        self.frozen.fill(false);
        self.born.fill(u32::MAX);
        self.step = 0;
        self.reach = 0.0;
        // Site 0 is the origin (i = j = 0).
        self.s[0] = 1.0;
        self.frozen[0] = true;
        self.born[0] = 0;
    }

    /// The receptive set and the split `s = u + v` (Reiter's steps 1–2).
    fn split(&mut self, lat: &Lattice) {
        let n = lat.len();
        let Form { beta, gamma, .. } = self.form;
        for i in 0..n {
            let rec = self.frozen[i]
                || lat.nb[i]
                    .iter()
                    .any(|&j| (j as usize) < n && self.frozen[j as usize]);
            self.rec[i] = rec;
            if rec {
                self.u[i] = 0.0;
                self.v[i] = self.s[i] + gamma;
            } else {
                self.u[i] = self.s[i];
                self.v[i] = 0.0;
            }
        }
        self.u[n] = beta;
    }

    /// Diffuse `u` into `un` (Reiter's step 3).
    fn diffuse(&mut self, lat: &Lattice) {
        let half = 0.5 * self.form.alpha;
        for (i, nb) in lat.nb.iter().enumerate() {
            let sum: f64 = nb.iter().map(|&j| self.u[j as usize]).sum();
            self.un[i] = self.u[i] + half * (sum / 6.0 - self.u[i]);
        }
    }

    /// One automaton step; returns whether any site froze.
    pub(crate) fn advance(&mut self, lat: &Lattice) -> bool {
        self.split(lat);
        self.diffuse(lat);
        self.step += 1;
        let mut grew = false;
        for i in 0..lat.len() {
            self.s[i] = self.un[i] + self.v[i];
            if !self.frozen[i] && self.s[i] >= 1.0 {
                self.frozen[i] = true;
                self.born[i] = self.step;
                let (x, y) = position(lat.cube[i]);
                self.reach = self.reach.max(x.hypot(y));
                grew = true;
            }
        }
        grew
    }

    /// Frozen disc sites (orbit-weighted).
    #[cfg(test)]
    pub(crate) fn frozen_count(&self, lat: &Lattice) -> u32 {
        self.frozen
            .iter()
            .zip(&lat.weight)
            .filter(|(f, _)| **f)
            .map(|(_, w)| *w)
            .sum()
    }

    /// Frozen–vapour lattice edges of the disc (orbit-weighted), the crystal perimeter.
    #[cfg(test)]
    pub(crate) fn perimeter(&self, lat: &Lattice) -> u32 {
        let n = lat.len();
        let mut p = 0;
        for i in 0..n {
            if self.frozen[i] {
                let open = lat.nb[i]
                    .iter()
                    .filter(|&&j| (j as usize) >= n || !self.frozen[j as usize])
                    .count() as u32;
                p += open * lat.weight[i];
            }
        }
        p
    }
}

/// Phases of one loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Grow,
    Hold,
    Sublimate,
    Rest,
}

/// Growth, hold, sublimation and rest in simulated seconds (growth ends earlier when the
/// crystal reaches the cap).
const GROW_S: f64 = 15.0;
const GROW_MAX_S: f64 = 20.0;
const HOLD_S: f64 = 7.0;
const SUBLIMATE_S: f64 = 3.0;
const REST_S: f64 = 0.8;
/// Ring inks, core first: `Cold` → `Wave` → `Particle`, levels alternating per ring.
pub(crate) const RING_ROLES: [Role; 3] = [Role::Cold, Role::Wave, Role::Particle];
pub(crate) const RING_LEVELS: [u8; 2] = [255, 205];
/// The crystal outline while it grows: the newest ice, at the rims' role, brightest.
pub(crate) const FRONT_INK: Ink = Ink::new(Role::Particle, 255);
/// Vapour stipple: keep probability at the form's ambient humidity (density `∝ s`, scaled by
/// `1/β` and capped at `s = β·VAPOUR_CAP`, so the dry dendrite loop is as visible as the plate
/// and the screen never reads black between crystals; ≤ 0.08), and its ink.
pub(crate) const VAPOUR: f64 = 0.06;
const VAPOUR_CAP: f64 = 1.3;
pub(crate) const VAPOUR_INK: Ink = Ink::new(Role::Dim, 110);
const VAPOUR_SALT: u64 = 0x5A0F_0000_0000_0001;
/// The vapour fades out between these shares of the lattice radius, so the lattice's
/// edge draws no circle.
const VAPOUR_FADE: (f64, f64) = (0.55, 0.95);
/// The crystal's cap radius as a share of the half-height (or half-width) of the art.
const FILL: f64 = 0.92;

/// The growth ring of a site frozen at step `born` when growth nominally takes `nominal`
/// steps.
pub(crate) fn ring_of(born: u32, nominal: u32, rings: u32) -> u32 {
    let n = u64::from(born) * u64::from(rings) / u64::from(nominal.max(1));
    n.min(u64::from(rings.max(1) - 1)) as u32
}

/// The ink of ring `n` of `rings`: `Cold` core → `Wave` → `Particle` rims, alternating levels.
pub(crate) fn ring_ink(n: u32, rings: u32) -> Ink {
    let rings = rings.max(1);
    let role = RING_ROLES[(n.min(rings - 1) * 3 / rings) as usize];
    Ink::new(role, RING_LEVELS[(n % 2) as usize])
}

#[derive(Debug)]
pub struct Snowflake {
    lat: Lattice,
    crystal: Crystal,
    /// Plane radius of each wedge site (lattice units).
    site_r: Vec<f32>,
    /// Index into `FORMS` of the current crystal.
    form: usize,
    phase: Phase,
    /// Seconds into the phase.
    clock: f64,
    /// Fractional automaton steps owed.
    owed: f64,
    /// Sites frozen after this step are hidden (sublimation).
    shown_until: u32,
    /// The disc's dot box: origin and size, and per dot the wedge site whose hexagonal
    /// cell holds it (`u32::MAX`: outside the disc).
    origin: (i32, i32),
    bw: usize,
    bh: usize,
    map: Vec<u32>,
    /// Per frame: per wedge site its ring + 1 (0: not shown frozen), and per box dot.
    site_code: Vec<u8>,
    code: Vec<u8>,
    dots: Dots,
    dirty: bool,
    /// The lattice disc's radius (`RADIUS`).
    radius: f64,
}

impl Default for Snowflake {
    fn default() -> Self {
        Self::with_radius(RADIUS)
    }
}

impl Snowflake {
    /// The scene on a lattice disc of `radius` (the scene uses `RADIUS`; tests use smaller
    /// discs, which only shorten the growth).
    fn with_radius(radius: f64) -> Self {
        let lat = Lattice::new(radius);
        let crystal = Crystal::new(&lat, FORMS[0]);
        let site_r = lat
            .cube
            .iter()
            .map(|&p| {
                let (x, y) = position(p);
                x.hypot(y) as f32
            })
            .collect();
        let n = lat.len();
        Self {
            lat,
            crystal,
            site_r,
            form: 0,
            phase: Phase::Grow,
            clock: 0.0,
            owed: 0.0,
            shown_until: u32::MAX,
            origin: (0, 0),
            bw: 0,
            bh: 0,
            map: Vec::new(),
            site_code: vec![0; n],
            code: Vec::new(),
            dots: Dots::default(),
            dirty: true,
            radius,
        }
    }

    fn nominal_steps(&self) -> u32 {
        (FORMS[self.form].steps_per_s * GROW_S).round() as u32
    }

    #[cfg(test)]
    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    /// Advance the loop by `dt` seconds.
    fn tick(&mut self, dt: f64) {
        self.clock += dt;
        match self.phase {
            Phase::Grow => {
                self.owed += FORMS[self.form].steps_per_s * dt;
                while self.owed >= 1.0 && self.crystal.reach < CAP * self.radius {
                    self.owed -= 1.0;
                    self.crystal.advance(&self.lat);
                }
                if self.crystal.reach >= CAP * self.radius || self.clock >= GROW_MAX_S {
                    self.enter(Phase::Hold);
                }
            }
            Phase::Hold => {
                if self.clock >= HOLD_S {
                    self.enter(Phase::Sublimate);
                }
            }
            Phase::Sublimate => {
                // Newest sites go first: the tips, then the rings inwards to the seed.
                let left = (1.0 - self.clock / SUBLIMATE_S).max(0.0);
                self.shown_until = (f64::from(self.crystal.step) * left).floor() as u32;
                if self.clock >= SUBLIMATE_S {
                    self.enter(Phase::Rest);
                }
            }
            Phase::Rest => {
                if self.clock >= REST_S {
                    self.form = (self.form + 1) % FORMS.len();
                    self.crystal.reset(FORMS[self.form]);
                    self.shown_until = u32::MAX;
                    self.owed = 0.0;
                    self.enter(Phase::Grow);
                }
            }
        }
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.clock = 0.0;
    }

    /// Per wedge site: its ring + 1 when shown frozen, else 0.
    fn site_codes(&mut self) {
        let nominal = self.nominal_steps();
        let rings = FORMS[self.form].rings;
        let shown = if self.phase == Phase::Rest {
            0
        } else {
            self.shown_until
        };
        let c = &self.crystal;
        for (code, (&f, &born)) in self.site_code.iter_mut().zip(c.frozen.iter().zip(&c.born)) {
            *code = if f && born <= shown {
                (ring_of(born, nominal, rings) + 1) as u8
            } else {
                0
            };
        }
    }

    /// Ring lines: a frozen dot is lit where a 4-neighbouring dot is in an
    /// older ring or outside the crystal (the inner edge of every ring band and the
    /// outline), so the rings read as 1-dot isochrones at any scale; the vapour as
    /// sparse stipple of the water `s`.
    fn raster(&mut self) {
        self.dots.clear();
        self.site_codes();
        let (bw, bh) = (self.bw, self.bh);
        for (code, &k) in self.code.iter_mut().zip(&self.map) {
            *code = if k == u32::MAX {
                0
            } else {
                self.site_code[k as usize]
            };
        }
        let growing = self.phase == Phase::Grow;
        let (f0, f1) = (VAPOUR_FADE.0 * self.radius, VAPOUR_FADE.1 * self.radius);
        for y in 0..bh {
            for x in 0..bw {
                let i = y * bw + x;
                let c = self.code[i];
                let (dx, dy) = (self.origin.0 + x as i32, self.origin.1 + y as i32);
                if c > 0 {
                    let left = if x > 0 { self.code[i - 1] } else { 0 };
                    let right = if x + 1 < bw { self.code[i + 1] } else { 0 };
                    let up = if y > 0 { self.code[i - bw] } else { 0 };
                    let down = if y + 1 < bh { self.code[i + bw] } else { 0 };
                    let low = left.min(right).min(up).min(down);
                    if low == 0 && growing {
                        // The advancing front: the newest ice, brightest.
                        self.dots.set(dx, dy, FRONT_INK);
                    } else if low < c {
                        self.dots
                            .set(dx, dy, ring_ink(u32::from(c - 1), FORMS[self.form].rings));
                    }
                    continue;
                }
                let k = self.map[i];
                if k == u32::MAX {
                    continue;
                }
                let r = f64::from(self.site_r[k as usize]);
                let t = ((r - f0) / (f1 - f0)).clamp(0.0, 1.0);
                let fade = 1.0 - t * t * (3.0 - 2.0 * t);
                let beta = FORMS[self.form].beta;
                let s = (self.crystal.s[k as usize] / beta).clamp(0.0, VAPOUR_CAP);
                let p = VAPOUR * s * fade;
                if stipple_keep(VAPOUR_SALT, dx, dy, p) {
                    self.dots.set(dx, dy, VAPOUR_INK);
                }
            }
        }
    }
}

impl Generator for Snowflake {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.dots.resize(size);
        let d = self.dots.dot_size();
        let (dw, dh) = (i32::from(d.width()), i32::from(d.height()));
        // The art area: all rows but the title row (when there are at least three).
        let rows = if size.height() >= 3 {
            size.height() - 1
        } else {
            size.height()
        };
        let ah = (f64::from(rows) * f64::from(Dots::CELL_H)).min(f64::from(dh));
        let (cx, cy) = (0.5 * f64::from(dw), 0.5 * ah);
        // Square dots: one scale for both axes; the cap circle fills FILL of the art.
        let scale = FILL * 0.5 * ah.min(f64::from(dw)) / (CAP * self.radius);
        let reach = self.radius * scale;
        let x0 = ((cx - reach).floor() as i32).max(0);
        let x1 = ((cx + reach).ceil() as i32).min(dw);
        let y0 = ((cy - reach).floor() as i32).max(0);
        let y1 = ((cy + reach).ceil() as i32).min(dh);
        self.origin = (x0, y0);
        self.bw = (x1 - x0).max(0) as usize;
        self.bh = (y1 - y0).max(0) as usize;
        self.map.clear();
        self.map.reserve(self.bw * self.bh);
        for y in y0..y1 {
            for x in x0..x1 {
                // The dot's centre in lattice units.
                let lx = (f64::from(x) + 0.5 - cx) / scale;
                let ly = (f64::from(y) + 0.5 - cy) / scale;
                let k = self.lat.site_at(lx, ly).map_or(u32::MAX, |k| k as u32);
                self.map.push(k);
            }
        }
        self.code.clear();
        self.code.resize(self.map.len(), 0);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let before = (self.crystal.step, self.shown_until, self.phase, self.form);
        self.tick(dt.max(0.0));
        // Only a step, a sublimation front, a phase or a form changes the picture (the
        // hold and rest re-raster nothing, GEN-R20).
        if before != (self.crystal.step, self.shown_until, self.phase, self.form) {
            self.dirty = true;
        }
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        Some(META.captions[self.form])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotation by 60° and a mirror of cube coordinates.
    fn rot60(p: Cube) -> Cube {
        (-p.2, -p.0, -p.1)
    }
    fn mirror(p: Cube) -> Cube {
        (p.0, p.2, p.1)
    }

    /// The scene's lattice and each form grown to the cap, once for all tests.
    fn grown() -> &'static (Lattice, [Crystal; 3]) {
        static GROWN: std::sync::OnceLock<(Lattice, [Crystal; 3])> = std::sync::OnceLock::new();
        GROWN.get_or_init(|| {
            let lat = Lattice::new(RADIUS);
            let crystals = std::array::from_fn(|form| {
                let mut c = Crystal::new(&lat, FORMS[form]);
                while c.reach < CAP * RADIUS && c.step < 20_000 {
                    c.advance(&lat);
                }
                c
            });
            (lat, crystals)
        })
    }

    /// Form `form` as it was when it first had at least `count` frozen sites (its frozen
    /// set at an earlier step is the sites born by then).
    fn at_count(form: usize, count: u32) -> Crystal {
        let (lat, crystals) = grown();
        let full = &crystals[form];
        let mut c = full.clone();
        for step in 0..=full.step {
            for (f, &born) in c.frozen.iter_mut().zip(&full.born) {
                *f = born <= step;
            }
            if c.frozen_count(lat) >= count {
                c.step = step;
                break;
            }
        }
        c
    }

    /// Every disc site as cube coordinates with its wedge index.
    fn disc_sites(lat: &Lattice) -> Vec<(Cube, usize)> {
        let mut out = Vec::new();
        for (k, &p) in lat.cube.iter().enumerate() {
            let start = out.len();
            for q in images(p) {
                if !out[start..].contains(&(q, k)) {
                    out.push((q, k));
                }
            }
        }
        out
    }

    #[test]
    fn snow_a1_frozen_set_is_six_fold_symmetric_at_every_step() {
        // The symmetry does not depend on the radius; a small disc keeps the test fast.
        let lat = Lattice::new(32.0);
        let sites = disc_sites(&lat);
        let disc = lat.weight.iter().sum::<u32>() as usize;
        assert_eq!(sites.len(), disc);
        for form in FORMS {
            let mut c = Crystal::new(&lat, form);
            for step in 0..120 {
                for &(p, k) in &sites {
                    for q in [rot60(p), mirror(p)] {
                        let kq = lat.find(q).expect("the disc is symmetric");
                        assert_eq!(c.frozen[kq], c.frozen[k], "step {step}: {p:?} vs {q:?}");
                    }
                }
                c.advance(&lat);
            }
        }
    }

    /// Reiter's automaton on the full disc, by brute force, as the wedge's reference.
    struct Full {
        sites: Vec<Cube>,
        nb: Vec<[usize; 6]>,
        s: Vec<f64>,
        frozen: Vec<bool>,
    }

    impl Full {
        fn new(radius: f64, form: Form) -> Self {
            let r = radius.ceil() as i32 + 1;
            let mut sites = Vec::new();
            for a in -2 * r..=2 * r {
                for c in -2 * r..=2 * r {
                    let p = (a, -a - c, c);
                    let (x, y) = position(p);
                    if x.hypot(y) <= radius {
                        sites.push(p);
                    }
                }
            }
            let n = sites.len();
            let nb = sites
                .iter()
                .map(|&(a, b, c)| {
                    DIRS.map(|(da, db, dc)| {
                        let q = (a + da, b + db, c + dc);
                        sites.iter().position(|&s| s == q).unwrap_or(n)
                    })
                })
                .collect();
            let mut s = vec![form.beta; n + 1];
            let mut frozen = vec![false; n];
            let o = sites.iter().position(|&p| p == (0, 0, 0)).unwrap();
            s[o] = 1.0;
            frozen[o] = true;
            Self {
                sites,
                nb,
                s,
                frozen,
            }
        }

        fn step(&mut self, form: Form) {
            let n = self.sites.len();
            let mut u = vec![0.0; n + 1];
            let mut v = vec![0.0; n];
            for i in 0..n {
                let rec = self.frozen[i] || self.nb[i].iter().any(|&j| j < n && self.frozen[j]);
                if rec {
                    v[i] = self.s[i] + form.gamma;
                } else {
                    u[i] = self.s[i];
                }
            }
            u[n] = form.beta;
            for i in 0..n {
                let mean = self.nb[i].iter().map(|&j| u[j]).sum::<f64>() / 6.0;
                self.s[i] = u[i] + 0.5 * form.alpha * (mean - u[i]) + v[i];
                if self.s[i] >= 1.0 {
                    self.frozen[i] = true;
                }
            }
        }
    }

    #[test]
    fn snow_a1_wedge_automaton_equals_the_full_lattice() {
        let radius = 18.0;
        let lat = Lattice::new(radius);
        for form in FORMS {
            let mut wedge = Crystal::new(&lat, form);
            let mut full = Full::new(radius, form);
            assert_eq!(full.sites.len(), lat.weight.iter().sum::<u32>() as usize);
            for step in 0..60 {
                wedge.advance(&lat);
                full.step(form);
                for (i, &p) in full.sites.iter().enumerate() {
                    let k = lat.find(p).unwrap();
                    assert!(
                        (wedge.s[k] - full.s[i]).abs() < 1e-9,
                        "step {step} {p:?}: {} vs {}",
                        wedge.s[k],
                        full.s[i]
                    );
                    if (full.s[i] - 1.0).abs() > 1e-9 {
                        assert_eq!(wedge.frozen[k], full.frozen[i], "step {step} {p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn snow_a2_diffusion_conserves_water_over_non_receptive_sites() {
        let lat = Lattice::new(48.0);
        let mut c = Crystal::new(&lat, FORMS[2]);
        for _ in 0..200 {
            c.advance(&lat);
        }
        // Water only well inside the disc, so none reaches the held boundary in a step;
        // the outside slot carries none.
        c.split(&lat);
        let n = lat.len();
        c.u[n] = 0.0;
        for (k, &p) in lat.cube.iter().enumerate() {
            let (x, y) = position(p);
            if x.hypot(y) > 0.6 * 48.0 {
                c.u[k] = 0.0;
            }
        }
        let before: f64 = (0..n).map(|k| f64::from(lat.weight[k]) * c.u[k]).sum();
        c.diffuse(&lat);
        let after: f64 = (0..n).map(|k| f64::from(lat.weight[k]) * c.un[k]).sum();
        assert!(before > 100.0);
        assert!(
            (after - before).abs() <= 1e-12 * before,
            "{before} → {after}"
        );
    }

    #[test]
    fn snow_a3_plate_is_compact_and_dendrite_branched_at_equal_size() {
        let count = 2500;
        let lat = &grown().0;
        let shape = |form: usize| {
            let c = at_count(form, count);
            assert!(c.frozen_count(lat) >= count, "form {form} stopped early");
            let (a, p) = (f64::from(c.frozen_count(lat)), f64::from(c.perimeter(lat)));
            p * p / a
        };
        let (plate, sectored, dendrite) = (shape(0), shape(1), shape(2));

        // A regular hexagon of side n has A ≈ 3n² sites and P ≈ 12n open edges (two per
        // edge site), so P²/A ≈ 48; branching raises it by an order of magnitude.
        assert!(plate < 60.0, "plate P²/A {plate}");
        assert!(dendrite > 400.0, "dendrite P²/A {dendrite}");
        assert!(
            plate < sectored && sectored < dendrite,
            "{plate} {sectored} {dendrite}"
        );
    }

    /// Separate runs of frozen sites along the wedge line `j` (parallel to the tip axis
    /// at distance `j·√3/2`): each side branch leaving the main arm crosses it as one run.
    fn runs_at(lat: &Lattice, c: &Crystal, j: i32) -> u32 {
        let (mut runs, mut inside) = (0, false);
        for i in j.. {
            let Some(k) = lat.find((i + j, -j, -i)) else {
                break;
            };
            let f = c.frozen[k];
            if f && !inside {
                runs += 1;
            }
            inside = f;
        }
        runs
    }

    #[test]
    fn snow_a3_dendrite_grows_side_branches_off_its_main_arms() {
        let (lat, crystals) = grown();
        let side = |form: usize| {
            let c = &crystals[form];
            assert!(c.reach >= CAP * RADIUS, "form {form} did not reach the cap");
            // Lines just off the main arm (2.6 and 3.5 sites from its axis): a fern's
            // side branches leave it one by one with black between them; a leaf-shaped
            // or needle arm is solid there (one run, serrated only at its far edge).
            (3..=4).map(|j| runs_at(lat, c, j)).min().unwrap_or(0)
        };
        let (plate, dendrite) = (side(0), side(2));
        // The plate is solid across every line; the dendrite's arm sprouts a row of side
        // branches (a fern, not a bare needle or a leaf).
        assert_eq!(plate, 1, "plate runs {plate}");
        assert!(
            dendrite >= 12,
            "dendrite side branches per arm side {dendrite}"
        );
    }

    #[test]
    fn snow_forms_reach_the_cap_in_about_the_growth_time() {
        for (n, form) in FORMS.iter().enumerate() {
            let c = &grown().1[n];
            let secs = f64::from(c.step) / form.steps_per_s;
            assert!(
                (11.0..=17.0).contains(&secs),
                "form {n}: {} steps take {secs:.1} s",
                c.step
            );
        }
    }

    #[test]
    fn snow_a4_freeze_time_never_decreases_outward_along_rays() {
        /// Steps a face site may trail the next one out (3 % of the sectored growth).
        const FACE_LAG: u32 = 6;
        // The six axes and the six 30° rays of the lattice, from the seed.
        let rays: [Cube; 12] = [
            (1, -1, 0),
            (1, 0, -1),
            (0, 1, -1),
            (-1, 1, 0),
            (-1, 0, 1),
            (0, -1, 1),
            (2, -1, -1),
            (1, 1, -2),
            (-1, 2, -1),
            (-2, 1, 1),
            (-1, -1, 2),
            (1, -2, 1),
        ];
        let (lat, crystals) = grown();
        for (form, c) in crystals.iter().enumerate() {
            for (i, d) in rays.into_iter().enumerate() {
                let mut last = 0;
                for n in 0.. {
                    let Some(k) = lat.find((n * d.0, n * d.1, n * d.2)) else {
                        break;
                    };
                    if !c.frozen[k] {
                        break;
                    }
                    // Along the tip axes strictly; across the faces (30° rays) a face
                    // site may fill a few steps after the next one out, bridged from
                    // the faster tips on both sides (the sectored plate's concave faces).
                    let lag = if i < 6 { 0 } else { FACE_LAG };
                    assert!(
                        c.born[k] + lag >= last,
                        "form {form} ray {d:?} at {n}: {} after {last}",
                        c.born[k]
                    );
                    last = c.born[k];
                }
            }
        }
    }

    #[test]
    fn snow_a5_caption_names_the_form_and_loops() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        // A small disc: the loop is the same, only each growth is shorter.
        let mut g = Snowflake::with_radius(24.0);
        g.resize(size, &mut ctx);
        let mut seen = vec![g.caption().unwrap()];
        for _ in 0..(60.0 / 0.25) as usize {
            g.update(0.25, &mut ctx);
            let cap = g.caption().unwrap();
            if cap != *seen.last().unwrap() {
                assert_eq!(g.phase(), Phase::Grow);
                assert_eq!(g.crystal.step, 0);
                seen.push(cap);
            }
        }
        assert_eq!(
            seen[..4],
            ["plate", "sectored plate", "stellar dendrite", "plate"]
        );
    }

    #[test]
    fn snow_hold_and_rest_do_not_reraster() {
        // GEN-R20: only a new step, sublimation front, phase or form dirties the frame.
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(5, size);
        let mut g = Snowflake::with_radius(24.0);
        g.resize(size, &mut ctx);
        while g.phase() != Phase::Hold {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            assert_eq!(g.phase(), Phase::Hold);
            assert!(!g.dirty, "a hold step dirtied the frame");
        }
        while g.phase() != Phase::Rest {
            g.update(SIM_DT, &mut ctx);
        }
        g.update(SIM_DT, &mut ctx);
        g.prepare();
        g.update(SIM_DT, &mut ctx);
        assert!(!g.dirty, "a rest step dirtied the frame");
    }

    #[test]
    fn snow_rings_run_from_cold_core_to_particle_rims() {
        for rings in FORMS.map(|f| f.rings) {
            let ink = |born| ring_ink(ring_of(born, 100, rings), rings);
            assert_eq!(ink(0).role, Role::Cold);
            assert_eq!(ink(50).role, Role::Wave);
            assert_eq!(ink(99).role, Role::Particle);
            assert_eq!(ring_of(500, 100, rings), rings - 1);
            // Neighbouring rings differ in level, so each band reads.
            for n in 0..rings - 1 {
                assert_ne!(ring_ink(n, rings), ring_ink(n + 1, rings));
            }
        }
        // The busier forms draw fewer rings (wider spacing between isochrones).
        assert!(FORMS[1].rings * 3 <= FORMS[0].rings * 2);
        assert!(FORMS[2].rings * 3 <= FORMS[0].rings * 2);
    }

    #[test]
    fn snow_screen_is_never_fully_black_over_two_loops() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        // A smaller disc shortens each growth; the vapour still fills the same screen area.
        let mut g = Snowflake::with_radius(40.0);
        g.resize(size, &mut ctx);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        // Every drawn frame (two steps) of two full loops shows something: at least 8 % of
        // the cells lit, crystal or vapour, also while it sublimates and rests.
        let mut wraps = 0;
        let mut last_form = g.form;
        for i in 0..(200.0 / SIM_DT) as usize {
            if g.form != last_form {
                wraps += usize::from(g.form == 0);
                last_form = g.form;
            }
            if wraps == 2 {
                break;
            }
            g.update(SIM_DT, &mut ctx);
            if i % 2 == 1 {
                g.prepare();
                c.clear();
                g.render(&mut c);
                let lit = c.cells().iter().filter(|cell| cell.ch != ' ').count();
                assert!(
                    lit * 12 >= c.cells().len(),
                    "near-black frame at t = {:.2} s ({:?}, form {}): {lit} cells",
                    (i + 1) as f64 * SIM_DT,
                    g.phase,
                    g.form
                );
            }
        }
    }

    #[test]
    fn snow_frames_are_braille_and_rings_are_lines() {
        let size = Size::new(200, 50);
        let mut ctx = Ctx::new(42, size);
        let mut g = Snowflake::default();
        g.resize(size, &mut ctx);
        // The plate at full size.
        while g.phase() == Phase::Grow {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let (mut lit, mut dots) = (0usize, 0u32);
        for cell in c.cells() {
            assert_eq!(cell.bg, Ink::BLANK);
            if cell.ch != ' ' {
                assert!(('\u{2801}'..='\u{28FF}').contains(&cell.ch));
                lit += 1;
                dots += (cell.ch as u32 - 0x2800).count_ones();
            }
        }
        // Lines, not fills: the lit cells average well under half their dots.
        assert!(lit > 300, "{lit} lit cells");
        assert!(
            f64::from(dots) / lit as f64 <= 4.0,
            "{dots} dots in {lit} cells"
        );
    }
}
