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
use sciart_core::grid::{self, SampleMap};
use sciart_core::prelude::*;
use sciart_core::raster::Tone;
use sciart_core::{ease, ink};

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
const CELLS_PER_NODE: f64 = 0.3;
/// Chemotactic speed while a front passes, in nodes per second.
const CHEMO_SPEED: f32 = 10.0;
/// Pacemaker period and radius; pacemakers only fire recovered nodes.
const PACE_S: f64 = 7.0;
const PACE_R: f64 = 5.0;
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
/// The glow a cell exposes around its pixel.
const CELL_HALO: [(i32, i32, f32); 9] = sciart_core::raster::halo_kernel(1);
/// One aggregation lasts this long, then the lawn is scattered afresh.
const CYCLE_S: f64 = 55.0;
const FADE_S: f64 = 2.0;

/// One amoeba, in lattice units, with the cAMP it sensed last sub-step.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Cell {
    pub(crate) x: f32,
    pub(crate) y: f32,
    last_u: f32,
    /// Recent speed, for the dark-field brightness.
    pub(crate) moving: f32,
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
    map: SampleMap,
    glow: Glow,
    tone: Tone,
    pixels: Pixels,
    seeded: bool,
}

impl Default for Dicty {
    fn default() -> Self {
        Self {
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
            map: SampleMap::default(),
            glow: Glow::default(),
            tone: Tone::asinh(0.3, 6.0),
            pixels: Pixels::default(),
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
            self.cells.push(Cell {
                x: rng.random_range(0.0..w),
                y: rng.random_range(0.0..h),
                ..Cell::default()
            });
        }
        self.n_pacers = if w * h > 4000.0 { 3 } else { 1 };
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

    fn substep(&mut self, rng: &mut impl Rng) {
        let h = (1.0 / SUB_PER_S) as f32;
        let pace = (PACE_S * SUB_PER_S) as u64;
        if self.step % pace == pace / 7 {
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
            if let Some((dx, dy)) = chemotaxis(&self.u, &self.v, (c.x, c.y), du, h) {
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

    fn rasterise(&mut self) {
        let px = self.pixels.pixel_size();
        let (lw, lh) = self.lattice();
        let (sx, sy) = (f32::from(px.width()) / lw, f32::from(px.height()) / lh);
        self.glow.clear();
        for c in &self.cells {
            // Dots: two per pixel each way. Moving cells scatter more light.
            let (x, y) = (f64::from(c.x * sx) * 2.0, f64::from(c.y * sy) * 2.0);
            self.glow.splat(x, y, &CELL_HALO, 0.35 + 0.45 * c.moving);
        }
        let local = self.time % CYCLE_S;
        let fade = ease::smoothstep(local / FADE_S)
            .min(ease::smoothstep((CYCLE_S - local) / FADE_S)) as f32;
        let (map, u, v, glow, tone) = (&self.map, &self.u, &self.v, &self.glow, &self.tone);
        self.pixels.shade(|x, y| {
            let g = tone.level(glow.get(x, y).unwrap_or(0.0) * fade);
            if g >= 28 {
                return Ink::new(Role::Ramp(Ramp::Starlight), g);
            }
            // A faint wash, brightest at the rising front (fresh cAMP, cells not yet
            // refractory), so waves read as thin rings rather than filled discs.
            let c = map.sample(u, x, y) * fade;
            let front = c * (1.0 - (map.sample(v, x, y) / 0.4).min(1.0));
            if c > 0.08 {
                Ink::new(
                    Role::Ramp(Ramp::Cobalt),
                    ink::level(f64::from(18.0 + 25.0 * c + 130.0 * front)),
                )
            } else {
                Ink::BLANK
            }
        });
    }
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let lit = |i: Ink| i.role != Role::Background;
    let cell = |i: Ink| i.role == Role::Ramp(Ramp::Starlight);
    let ink = match (lit(top), lit(bottom)) {
        (false, false) => return None,
        _ if cell(top) && (!cell(bottom) || top.level >= bottom.level) => top,
        _ if cell(bottom) => bottom,
        (true, _) => top,
        (false, true) => bottom,
    };
    let ch = if cell(ink) {
        match ink.level {
            200.. => '@',
            120.. => 'o',
            _ => '.',
        }
    } else {
        ':'
    };
    Some((ch, ink))
}

impl Generator for Dicty {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.pixels.resize(size);
        self.glow.resize(size);
        let px = self.pixels.pixel_size();
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
            for c in &mut self.cells {
                c.x = (c.x / old.0 * nw).clamp(0.0, nw - 0.01);
                c.y = (c.y / old.1 * nh).clamp(0.0, nh - 0.01);
            }
        }
        self.density();
        self.map.rebuild(lat, px);
        self.rasterise();
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
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lawn(size: Size) -> (Dicty, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Dicty::default();
        g.resize(size, &mut ctx);
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
    fn dict_a3_a4_the_lawn_streams_and_cells_are_conserved() {
        let (mut g, mut ctx) = lawn(Size::new(120, 40));
        let n = g.cells.len();
        assert!(n <= CAP);
        let v0 = variance(&g);
        for _ in 0..(50 * 60) {
            g.update(SIM_DT, &mut ctx);
            assert_eq!(g.cells.len(), n);
        }
        let v1 = variance(&g);
        assert!(v1 >= 3.0 * v0, "density variance {v0:.2} → {v1:.2}");
    }
}
