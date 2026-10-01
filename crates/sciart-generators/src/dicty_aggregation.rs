//! Slime amoebae: a lawn of starving *Dictyostelium* cells relays waves of cAMP; each
//! cell steps up the gradient as a wave front passes, and the lawn breaks into branching
//! streams that pour into mounds, as in the dark-field time-lapse films (SPEC-002
//! `dicty_aggregation`).
//!
//! The cAMP relay is a two-variable excitable medium (the shared Barkley kernel,
//! `shared::excitable`, a FitzHugh–Nagumo-type reduction like Martiel–Goldbeter's): `u`
//! is extracellular cAMP, `v` the cells' refractoriness. The medium is the cells
//! themselves, so its excitability follows the local cell density `ρ`: `b` falls from
//! [`B_EMPTY`] (no relay: `u_th > ½`, fronts die) to [`B_DENSE`] where cells are dense.
//! Pacemaker cells fire every [`PACE_S`] seconds. A cell moves up `∇u` only while the
//! front passes (cAMP rising and the cell not refractory), which solves the
//! back-of-the-wave problem; since relaying is stronger where cells are dense, cells
//! pull into lines along which later waves travel best: streams.

use rand::Rng;
use sciart_core::ease;
use sciart_core::grid;
use sciart_core::prelude::*;

use crate::shared::excitable::{self, Barkley, Front, FrontStroke};

pub static META: Meta = Meta {
    name: "dicty_aggregation",
    category: Category::Biochemistry,
    description: "Slime-mould amoebae relay spiral cAMP waves and stream up the gradient into mounds",
    title: "Slime amoebae · cAMP waves call",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Dicty::default())
}

// ---- Model (SPEC-002 Model) ----

pub(crate) const MODEL: Barkley = Barkley {
    a: 0.75,
    eps: 0.03,
    h: 0.5,
    tau: 0.025,
};
const _: () = assert!(MODEL.is_bounded());
/// Excitability where the lawn is dense, and where there are no cells.
pub(crate) const B_DENSE: f32 = 0.01;
pub(crate) const B_EMPTY: f32 = 0.42;
/// Sub-steps per simulated second (5 tu/s).
const SUB_PER_S: f64 = 200.0;
const MAX_NODES: usize = 12_800;
const BUCKET: f64 = 1.25;
/// Cells at most, and per lattice node on a fresh lawn.
pub(crate) const CAP: usize = 3000;
const CELLS_PER_NODE: f64 = 0.24;
/// Chemotactic speed while a front passes, in nodes per second.
const CHEMO_SPEED: f32 = 10.0;
/// Chemotaxis speed-up once the lawn aggregates (from `PACE_LATE_FROM`).
const CHEMO_LATE: f32 = 1.6;
/// Pacemaker period and radius; pacemakers only fire recovered nodes.
const PACE_S: f64 = 7.0;
/// The quicker relay of the aggregating lawn (s), from this time in the cycle (s).
const PACE_LATE_S: f64 = 3.5;
const PACE_LATE_FROM: f64 = 20.0;
const PACE_R: f64 = 5.0;
/// Pacemaker homing: search radius (nodes) and the least 5×5-node crowd it moves to
/// (≈ 2× a fresh lawn's 6).
const HOME_R: usize = 10;
const HOME_MIN: f32 = 12.0;
const PACE_V_MAX: f32 = 0.1;
/// Nodes more crowded than this take no more cells (a mound's packing).
const MAX_PER_NODE: f32 = 14.0;
/// Adhesion drift per unit excitability gradient (nodes per second).
const ADHESION: f32 = 10.0;
/// Spiral seeds, painted like BZ's broken fronts.
const STROKE: FrontStroke = FrontStroke {
    front_w: 2.0,
    tail_w: 6.0,
    tail_v: (0.69, 0.36),
};
/// One aggregation lasts this long, then the lawn is scattered afresh.
const CYCLE_S: f64 = 45.0;
const FADE_S: f64 = 2.0;

// ---- Display (M16, GEN-R25: strange_attractors' thread idiom) ----

/// Cells: one braille dot per amoeba in `Hot`, at two flat levels: the thin lawn, and
/// the streams and mounds, where the 3×3-node neighbourhood holds at least
/// [`CROWD_N`] cells (≈ 4× a fresh lawn's); the ground is black.
const CELL_INKS: [Ink; 2] = [Ink::new(Role::Hot, 110), Ink::new(Role::Hot, 255)];
const CROWD_N: f32 = 9.0;
/// A moving amoeba's trail: a dim `Hot` dot line through its last [`TRAIL_N`]
/// positions, sampled every [`TRAIL_DT`] simulated seconds (so it is ≈ 0.5 s long and
/// fades by vanishing as the cell stops); resting cells draw none.
const TRAIL_INK: Ink = Ink::new(Role::Hot, 90);
const TRAIL_N: usize = 4;
const TRAIL_DT: f64 = 0.12;
/// A trail is drawn when the cell moved more than this many nodes over it.
const TRAIL_MIN: f32 = 0.35;
/// Wave fronts: the `u = ½` contour of the cAMP relay, one braille dot thick, traced by
/// marching squares (CORE-R105), on the rising edge only (`v < WAVE_V`), in `Cold`,
/// fresh edges (`v < ½ WAVE_V`) a step brighter: thin spiral threads.
const WAVE_INKS: [Ink; 2] = [Ink::new(Role::Cold, 190), Ink::new(Role::Cold, 255)];
const WAVE_V: f32 = 0.3;
#[cfg(test)]
const WAVE_MIN: f32 = 0.3;

/// One amoeba, in lattice units, with the cAMP it sensed last sub-step and its recent
/// positions (newest first) for the trail.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Cell {
    pub(crate) x: f32,
    pub(crate) y: f32,
    last_u: f32,
    /// Recent speed (1 just after a chemotactic step, decaying).
    pub(crate) moving: f32,
    trail: [(f32, f32); TRAIL_N],
}

impl Cell {
    fn at(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            trail: [(x, y); TRAIL_N],
            ..Self::default()
        }
    }
}

/// Up-gradient step of a cell at `(x, y)` over `h` seconds, or `None` when it should
/// not move: only while cAMP rises (`du > 0`) at a non-refractory cell (`v < 0.3`).
pub(crate) fn chemotaxis(
    u: &Grid<f32>,
    v: &Grid<f32>,
    (x, y): (f32, f32),
    du: f32,
    h: f32,
) -> Option<(f32, f32)> {
    let (ix, iy) = (x as i32, y as i32);
    let at = |dx: i32, dy: i32| u.get(ix + dx, iy + dy).copied();
    let (c, vv) = (at(0, 0)?, v.get(ix, iy).copied()?);
    if du <= 0.0 || vv >= 0.3 || c < 0.05 {
        return None;
    }
    let gx = at(1, 0).unwrap_or(c) - at(-1, 0).unwrap_or(c);
    let gy = at(0, 1).unwrap_or(c) - at(0, -1).unwrap_or(c);
    let g = gx.hypot(gy);
    if g < 1e-4 {
        return None;
    }
    let step = CHEMO_SPEED * h / g;
    Some((gx * step, gy * step))
}

#[derive(Debug)]
struct Dicty {
    u: Grid<f32>,
    v: Grid<f32>,
    lap: Grid<f32>,
    /// Excitability per node from the cell density.
    b: Vec<f32>,
    /// Cell count per node, and its blur (scratch).
    rho: Vec<f32>,
    blur: Vec<f32>,
    cells: Vec<Cell>,
    pacers: [(f64, f64); 3],
    n_pacers: usize,
    step: u64,
    owed: f64,
    time: f64,
    scratch: Grid<f32>,
    /// Seconds since the trails were last sampled.
    trail_clock: f64,
    /// Seconds since the pacemakers last fired.
    pace_clock: f64,
    /// Cells, trails and wave fronts (M16: all braille).
    dots: Dots,
    seeded: bool,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Dicty {
    fn default() -> Self {
        Self {
            dirty: true,
            u: Grid::default(),
            v: Grid::default(),
            lap: Grid::default(),
            b: Vec::new(),
            rho: Vec::new(),
            blur: Vec::new(),
            cells: Vec::with_capacity(CAP),
            pacers: [(0.0, 0.0); 3],
            n_pacers: 0,
            step: 0,
            owed: 0.0,
            time: 0.0,
            scratch: Grid::default(),
            trail_clock: 0.0,
            pace_clock: PACE_S - 1.0,
            dots: Dots::default(),
            seeded: false,
        }
    }
}

impl Dicty {
    fn lattice(&self) -> (f32, f32) {
        (f32::from(self.u.width()), f32::from(self.u.height()))
    }

    /// Scatter a fresh lawn and place pacemakers.
    fn scatter(&mut self, rng: &mut impl Rng) {
        let (w, h) = self.lattice();
        let n = ((f64::from(w) * f64::from(h) * CELLS_PER_NODE) as usize).clamp(1, CAP);
        self.cells.clear();
        for _ in 0..n {
            self.cells
                .push(Cell::at(rng.random_range(0.0..w), rng.random_range(0.0..h)));
        }
        self.n_pacers = if w * h > 4000.0 { 3 } else { 1 };
        // The first pulse 1 s into the cycle, as before.
        self.pace_clock = PACE_S - 1.0;
        for p in &mut self.pacers {
            *p = (rng.random_range(0.2..0.8), rng.random_range(0.2..0.8));
        }
        self.u.fill(0.0);
        self.v.fill(0.0);
        // Spiral centres: broken fronts whose free ends curl into rotating spirals, the
        // organising centres a real lawn aggregates to.
        let spirals = if w * h > 4000.0 { 2 } else { 1 };
        for _ in 0..spirals {
            let c = [
                f64::from(w) * rng.random_range(0.25..0.75),
                f64::from(h) * rng.random_range(0.25..0.75),
            ];
            let (sn, cs) = rng.random_range(0.0..std::f64::consts::TAU).sin_cos_det();
            let len = 0.3 * f64::from(w.min(h));
            let front = Front {
                a: c,
                b: [c[0] + cs * len, c[1] + sn * len],
                n: [-sn, cs],
                anchored: false,
            };
            excitable::paint_front(&mut self.u, &mut self.v, &front, &STROKE);
        }
    }

    /// Bin the cells to nodes, blur (5×5 box, separable), and set the excitability.
    fn density(&mut self) {
        let (w, h) = (usize::from(self.u.width()), usize::from(self.u.height()));
        self.rho.fill(0.0);
        for c in &self.cells {
            let (x, y) = ((c.x as usize).min(w - 1), (c.y as usize).min(h - 1));
            self.rho[y * w + x] += 1.0;
        }
        // Horizontal then vertical running box of radius 2 (clipped at the walls).
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (x.saturating_sub(2), (x + 2).min(w - 1));
                self.blur[y * w + x] = self.rho[y * w + a..=y * w + b].iter().sum();
            }
        }
        let norm = 1.0 / (25.0 * CELLS_PER_NODE as f32);
        for x in 0..w {
            for y in 0..h {
                let (a, b) = (y.saturating_sub(2), (y + 2).min(h - 1));
                let s: f32 = (a..=b).map(|yy| self.blur[yy * w + x]).sum();
                let dense = ease::smoothstep(f64::from(s * norm) / 0.6) as f32;
                self.b[y * w + x] = B_EMPTY + (B_DENSE - B_EMPTY) * dense;
            }
        }
    }

    /// Cells in the 5×5 nodes about node `(x, y)`.
    fn crowd5(&self, x: usize, y: usize) -> f32 {
        let (w, h) = (usize::from(self.u.width()), usize::from(self.u.height()));
        let mut n = 0.0;
        for yy in y.saturating_sub(2)..=(y + 2).min(h - 1) {
            for xx in x.saturating_sub(2)..=(x + 2).min(w - 1) {
                n += self.rho[yy * w + xx];
            }
        }
        n
    }

    /// M16 (DICT-A9): a pacemaker moves to the densest crowd within `HOME_R` nodes once
    /// one has formed (a real aggregate's centre is its pacemaker), so waves keep
    /// starting where the cells are, run out along the streams (an empty lawn does not
    /// relay them) and pull the streams in to the mounds until the reset.
    fn home_pacers(&mut self) {
        let (w, h) = (usize::from(self.u.width()), usize::from(self.u.height()));
        if w < 2 || h < 2 {
            return;
        }
        for k in 0..self.n_pacers {
            let (fx, fy) = self.pacers[k];
            let (px, py) = (
                ((fx * (w - 1) as f64).round() as usize).min(w - 1),
                ((fy * (h - 1) as f64).round() as usize).min(h - 1),
            );
            let here = self.crowd5(px, py);
            let mut best = (here, px, py);
            for y in py.saturating_sub(HOME_R)..=(py + HOME_R).min(h - 1) {
                for x in px.saturating_sub(HOME_R)..=(px + HOME_R).min(w - 1) {
                    let c = self.crowd5(x, y);
                    if c > best.0 {
                        best = (c, x, y);
                    }
                }
            }
            if best.0 >= HOME_MIN && best.0 > 1.3 * here {
                self.pacers[k] = (
                    best.1 as f64 / (w - 1) as f64,
                    best.2 as f64 / (h - 1) as f64,
                );
            }
        }
    }

    fn substep(&mut self, rng: &mut impl Rng) {
        let h = (1.0 / SUB_PER_S) as f32;
        // The relay quickens as the cells aggregate (in real lawns from ~10 to ~3 min):
        // `PACE_S` at first, `PACE_LATE_S` once the streams have formed.
        let local = self.time % CYCLE_S;
        let late = local >= PACE_LATE_FROM;
        let period = if late { PACE_LATE_S } else { PACE_S };
        // Aggregating cells crawl faster (polarised, in streams): the streams shorten.
        let chemo = if late { CHEMO_LATE } else { 1.0 };
        self.pace_clock += f64::from(h);
        if self.pace_clock >= period {
            self.pace_clock = 0.0;
            self.home_pacers();
            let (w, hh) = (f64::from(self.u.width()), f64::from(self.u.height()));
            for &(px, py) in &self.pacers[..self.n_pacers] {
                excitable::excite_disc(
                    &mut self.u,
                    &self.v,
                    (px * (w - 1.0), py * (hh - 1.0)),
                    PACE_R,
                    PACE_V_MAX,
                );
            }
        }
        if self.step % 10 == 0 {
            self.density();
        }
        let b = &self.b;
        MODEL.step(&mut self.u, &mut self.v, &mut self.lap, |i| b[i]);
        let (w, hh) = self.lattice();
        let lw = usize::from(self.u.width());
        let lh = usize::from(self.u.height());
        for c in &mut self.cells {
            let now = self.u.get(c.x as i32, c.y as i32).copied().unwrap_or(0.0);
            let du = now - c.last_u;
            c.last_u = now;
            c.moving *= 0.97;
            if let Some((dx, dy)) = chemotaxis(&self.u, &self.v, (c.x, c.y), du, h * chemo) {
                let (nx, ny) = (
                    (c.x + dx).clamp(0.0, w - 0.01),
                    (c.y + dy).clamp(0.0, hh - 0.01),
                );
                let crowd = self.rho[(ny as usize) * lw + nx as usize];
                if crowd < MAX_PER_NODE {
                    (c.x, c.y) = (nx, ny);
                    c.moving = 1.0;
                }
            }
            // Cells stick together: a slow drift up the density gradient (read off the
            // excitability, which falls where cells are dense) keeps streams coherent.
            let (ix, iy) = (c.x as usize, c.y as usize);
            if c.moving < 0.5 && ix > 0 && iy > 0 && ix + 1 < lw && iy + 1 < lh {
                let at = |x: usize, y: usize| self.b[y * lw + x];
                let (gx, gy) = (
                    at(ix - 1, iy) - at(ix + 1, iy),
                    at(ix, iy - 1) - at(ix, iy + 1),
                );
                let crowd = self.rho[iy * lw + ix];
                if crowd < MAX_PER_NODE {
                    c.x = (c.x + ADHESION * h * gx).clamp(0.0, w - 0.01);
                    c.y = (c.y + ADHESION * h * gy).clamp(0.0, hh - 0.01);
                }
            }
            // A little random crawling.
            c.x = (c.x + rng.random_range(-0.02..0.02)).clamp(0.0, w - 0.01);
            c.y = (c.y + rng.random_range(-0.02..0.02)).clamp(0.0, hh - 0.01);
        }
        self.step += 1;
    }

    /// Sample every cell's position into its trail (newest first).
    fn sample_trails(&mut self) {
        for c in &mut self.cells {
            c.trail.copy_within(0..TRAIL_N - 1, 1);
            c.trail[0] = (c.x, c.y);
        }
    }

    /// Whether the 3×3-node neighbourhood of `(x, y)` holds a crowd (a stream or mound).
    fn crowded(&self, x: f32, y: f32) -> bool {
        let (w, h) = (usize::from(self.u.width()), usize::from(self.u.height()));
        let (ix, iy) = ((x as usize).min(w - 1), (y as usize).min(h - 1));
        let mut n = 0.0;
        for yy in iy.saturating_sub(1)..=(iy + 1).min(h - 1) {
            for xx in ix.saturating_sub(1)..=(ix + 1).min(w - 1) {
                n += self.rho[yy * w + xx];
            }
        }
        n >= CROWD_N
    }

    fn rasterise(&mut self) {
        let ds = self.dots.dot_size();
        let (lw, lh) = self.lattice();
        let (sx, sy) = (
            f64::from(ds.width()) / f64::from(lw),
            f64::from(ds.height()) / f64::from(lh),
        );
        // Lattice point → dot (a node spans `[i, i + 1)`, `Dots::from_cells`' convention).
        let to_dot = move |x: f64, y: f64| (x * sx - 0.5, y * sy - 0.5);
        let local = self.time % CYCLE_S;
        let fade = ease::smoothstep(local / FADE_S)
            .min(ease::smoothstep((CYCLE_S - local) / FADE_S)) as f32;
        self.dots.clear();
        // Trails first: a cell's dot outranks its own trail (one ink per braille cell).
        for c in &self.cells {
            let (ox, oy) = c.trail[TRAIL_N - 1];
            if (c.x - ox).hypot(c.y - oy) < TRAIL_MIN {
                continue;
            }
            let mut from = (f64::from(c.x), f64::from(c.y));
            for &(px, py) in &c.trail {
                let to = (f64::from(px), f64::from(py));
                let (a, b) = (to_dot(from.0, from.1), to_dot(to.0, to.1));
                self.dots.line(a.0, a.1, b.0, b.1, TRAIL_INK);
                from = to;
            }
        }
        // The cAMP fronts: node centres sit at `i + ½`.
        if fade > 0.05 {
            let (u, v) = (&self.u, &self.v);
            let w = usize::from(u.width());
            let vs = v.as_slice();
            let node = move |x: f64, y: f64| to_dot(x + 0.5, y + 0.5);
            self.dots.contour_by(
                u.as_slice(),
                w,
                usize::from(u.height()),
                0.5,
                node,
                |i, j| {
                    let k = j * w + i;
                    // All four nodes fresh: the rising edge only, never the back.
                    let vv = vs[k].max(vs[k + 1]).max(vs[k + w].max(vs[k + w + 1]));
                    (vv < WAVE_V).then(|| WAVE_INKS[usize::from(vv < 0.5 * WAVE_V)])
                },
            );
        }
        // The cells: one dot each, always drawn (the scattered lawn is the subject before
        // the first wave), a step brighter where they crowd; around a re-scatter the
        // crowds drop to the lawn step, so the old mounds fade before the new lawn.
        for c in &self.cells {
            let crowded = fade > 0.5 && self.crowded(c.x, c.y);
            let (x, y) = to_dot(f64::from(c.x), f64::from(c.y));
            self.dots.plot(x, y, CELL_INKS[usize::from(crowded)]);
        }
    }
}

/// How strongly a node draws as a wave edge: its cAMP `u`, only while it is not yet
/// refractory (`v < WAVE_V`), so a wave shows as a thin rising ring [DICT-A6].
#[cfg(test)]
pub(crate) fn wave_edge(u: f32, v: f32) -> f32 {
    u * (1.0 - (v / WAVE_V).min(1.0))
}

impl Generator for Dicty {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.dots.resize(size);
        // One node per half-block pixel at most (ADR-0012), as before M16.
        let px = Size::new(size.width(), size.height().saturating_mul(2));
        let lat = grid::lattice_for(px, MAX_NODES, BUCKET);
        let old = self.lattice();
        if !self.seeded {
            self.seeded = true;
            self.u = Grid::new(lat, 0.0);
            self.v = Grid::new(lat, 0.0);
        } else if lat != self.u.size() {
            // ADR-0012: carry the waves and the cells over.
            grid::regrid_bilinear(&mut self.u, lat, &mut self.scratch);
            grid::regrid_bilinear(&mut self.v, lat, &mut self.scratch);
        }
        let n = lat.area();
        for buf in [&mut self.b, &mut self.rho, &mut self.blur] {
            buf.clear();
            buf.resize(n, B_EMPTY);
        }
        if self.cells.is_empty() {
            self.scatter(&mut ctx.rng);
        } else {
            let (nw, nh) = self.lattice();
            let scale = |(x, y): (f32, f32)| {
                (
                    (x / old.0 * nw).clamp(0.0, nw - 0.01),
                    (y / old.1 * nh).clamp(0.0, nh - 0.01),
                )
            };
            for c in &mut self.cells {
                (c.x, c.y) = scale((c.x, c.y));
                for p in &mut c.trail {
                    *p = scale(*p);
                }
            }
        }
        self.density();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        let before = (self.time / CYCLE_S).floor();
        self.time += dt;
        if (self.time / CYCLE_S).floor() > before {
            self.scatter(&mut ctx.rng);
        }
        self.owed += dt * SUB_PER_S;
        let n = (self.owed + 1e-9).floor();
        self.owed -= n;
        for _ in 0..n as u32 {
            self.substep(&mut ctx.rng);
        }
        self.trail_clock += dt;
        if self.trail_clock >= TRAIL_DT {
            self.trail_clock %= TRAIL_DT;
            self.sample_trails();
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
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lawn(size: Size) -> (Dicty, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Dicty::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    fn variance(g: &Dicty) -> f64 {
        let mut g2 = Vec::new();
        // Coarse bins of 4×4 nodes.
        let (w, h) = (
            usize::from(g.u.width()) / 4 + 1,
            usize::from(g.u.height()) / 4 + 1,
        );
        g2.resize(w * h, 0.0f64);
        for c in &g.cells {
            g2[(c.y as usize / 4) * w + c.x as usize / 4] += 1.0;
        }
        let mean = g2.iter().sum::<f64>() / g2.len() as f64;
        g2.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / g2.len() as f64
    }

    #[test]
    fn dict_a1_a5_one_wave_then_refractory() {
        let s = Size::new(40, 40);
        let (mut u, mut v, mut lap) = (Grid::new(s, 0.0f32), Grid::new(s, 0.0f32), Grid::default());
        excitable::excite_disc(&mut u, &v, (20.0, 20.0), PACE_R, 1.0);
        let (mut peak_seen, mut restimulated_at) = (false, None);
        let centre = |u: &Grid<f32>| u.get(20, 20).copied().unwrap_or(1.0);
        for k in 0..2000 {
            MODEL.step(&mut u, &mut v, &mut lap, |_| B_DENSE);
            for (&a, &b) in u.as_slice().iter().zip(v.as_slice()) {
                assert!(a.is_finite() && b.is_finite() && a >= 0.0 && b >= 0.0);
            }
            peak_seen |= u
                .iter()
                .any(|((x, y), &val)| val > 0.5 && (x - 20).abs().max((y - 20).abs()) >= 8);
            if restimulated_at.is_none() && centre(&u) < 0.5 {
                // The centre has just fallen back: kick it hard. Refractory, it does not
                // take, and no second wave starts.
                excitable::excite_disc(&mut u, &v, (20.0, 20.0), PACE_R, 1.0);
                restimulated_at = Some(k);
            }
            if restimulated_at.is_some_and(|at| k == at + 40) {
                assert!(
                    centre(&u) < 0.5,
                    "refractory centre re-excited: {}",
                    centre(&u)
                );
            }
        }
        assert!(restimulated_at.is_some());
        assert!(peak_seen, "a wave travelled out");
    }

    #[test]
    fn dict_a2_cells_move_only_up_gradient() {
        let s = Size::new(20, 20);
        let mut u = Grid::new(s, 0.0f32);
        let v = Grid::new(s, 0.0f32);
        for y in 0..20 {
            for x in 0..20 {
                u.set(x, y, 0.04 * x as f32);
            }
        }
        let (dx, dy) =
            chemotaxis(&u, &v, (10.5, 10.5), 0.1, 0.01).expect("moves on a rising front");
        assert!(dx > 0.0 && dy.abs() < 1e-6, "up the gradient: {dx} {dy}");
        assert!(
            chemotaxis(&u, &v, (10.5, 10.5), -0.1, 0.01).is_none(),
            "not on the back of the wave"
        );
        let refractory = Grid::new(s, 0.5f32);
        assert!(chemotaxis(&u, &refractory, (10.5, 10.5), 0.1, 0.01).is_none());
    }

    #[test]
    fn dict_a6_only_the_rising_edge_of_a_wave_is_drawn() {
        assert!((wave_edge(1.0, 0.0) - 1.0).abs() < 1e-6);
        // Behind the edge (refractory cells) the wave draws nothing, however high u is.
        assert_eq!(wave_edge(1.0, WAVE_V), 0.0);
        assert_eq!(wave_edge(0.9, 0.6), 0.0);
        assert!(wave_edge(0.8, 0.5 * WAVE_V) > WAVE_MIN);
    }

    #[test]
    fn dict_a3_a4_the_lawn_streams_and_cells_are_conserved() {
        let (mut g, mut ctx) = lawn(Size::new(120, 40));
        let n = g.cells.len();
        assert!(n <= CAP);
        let v0 = variance(&g);
        for _ in 0..(40 * 60) {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            assert_eq!(g.cells.len(), n);
        }
        let v1 = variance(&g);
        assert!(v1 >= 3.0 * v0, "density variance {v0:.2} → {v1:.2}");
    }

    #[test]
    fn dict_a7_cells_are_hot_dots_and_fronts_cold_threads_in_flat_steps() {
        let (mut g, mut ctx) = lawn(Size::new(120, 40));
        for _ in 0..(20 * 60) {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let mut canvas = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut canvas);
        let (mut cells, mut waves) = (0, 0);
        for c in canvas.cells() {
            match c.ink.role {
                Role::Hot => {
                    assert!(CELL_INKS.contains(&c.ink) || c.ink == TRAIL_INK, "{c:?}");
                    cells += 1;
                }
                Role::Cold => {
                    assert!(WAVE_INKS.contains(&c.ink), "{c:?}");
                    waves += 1;
                }
                Role::Background => {}
                r => panic!("unexpected role {r:?}"),
            }
        }
        assert!(cells > 50 && waves > 0, "{cells} cells, {waves} wave cells");
        // Streams and mounds exist: some cells are crowded.
        assert!(canvas.cells().iter().any(|c| c.ink == CELL_INKS[1]));
    }

    #[test]
    fn dict_a7_the_scattered_lawn_shows_from_the_first_frame() {
        let size = Size::new(120, 40);
        let (g, _) = lawn(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let cells = canvas
            .cells()
            .iter()
            .filter(|c| c.ink == CELL_INKS[0])
            .count();
        assert!(cells > 100, "{cells} lawn cells at t = 0");
    }

    #[test]
    fn dict_a9_the_aggregate_keeps_moving_and_signalling_to_the_reset() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = lawn(size);
        while g.time < 10.0 {
            g.update(SIM_DT, &mut ctx);
        }
        let mut start = 10.0;
        while start + 5.0 <= CYCLE_S - 1.0 {
            let before: Vec<(f32, f32)> = g.cells.iter().map(|c| (c.x, c.y)).collect();
            let mut crest = false;
            let mut next = start + 0.5;
            while g.time < start + 5.0 {
                g.update(SIM_DT, &mut ctx);
                if g.time >= next {
                    next += 0.5;
                    g.prepare();
                    let mut canvas = Canvas::new(size, GlyphMode::Unicode);
                    g.render(&mut canvas);
                    crest |= canvas.cells().iter().any(|c| c.ink.role == Role::Cold);
                }
            }
            let moved = g
                .cells
                .iter()
                .zip(&before)
                .filter(|(c, b)| (c.x - b.0).hypot(c.y - b.1) >= 1.0)
                .count();
            let share = moved as f64 / g.cells.len() as f64;
            assert!(
                share >= 0.2,
                "{start}–{} s: {share:.2} of cells moved",
                start + 5.0
            );
            assert!(crest, "{start}–{} s: no cAMP crest", start + 5.0);
            start += 5.0;
        }
    }

    #[test]
    fn dict_a8_threads_only_amoebae_leave_trails_and_fronts_are_thin() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = lawn(size);
        let (mut trails, mut front_cells, mut front_dots) = (0, 0, 0);
        for t in [12, 20, 30] {
            while g.time < f64::from(t) {
                g.update(SIM_DT, &mut ctx);
            }
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            for c in canvas.cells() {
                if c.ch == ' ' {
                    continue;
                }
                // Every lit cell is a braille dot cell on black: no half blocks.
                assert!(
                    c.bg == Ink::BLANK && ('\u{2800}'..='\u{28ff}').contains(&c.ch),
                    "{c:?}"
                );
                trails += usize::from(c.ink == TRAIL_INK);
                if c.ink.role == Role::Cold {
                    front_cells += 1;
                    front_dots += (c.ch as u32 - 0x2800).count_ones();
                }
            }
        }
        assert!(trails > 20, "{trails} trail cells");
        assert!(front_cells > 0, "no fronts");
        let mean = f64::from(front_dots) / f64::from(front_cells);
        assert!(
            mean < 4.0,
            "fronts {mean:.2} dots per cell: not a thin thread"
        );
    }
}
