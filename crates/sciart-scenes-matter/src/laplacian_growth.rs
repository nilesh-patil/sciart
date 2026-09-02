//! Laplacian growth: the dielectric breakdown model (Niemeyer, Pietronero & Wiesmann
//! 1984), shared `pub(crate)` model of `lichtenberg_discharge` (SPEC-002 Batch M13).
//!
//! A tree of lattice sites held at potential φ = 0 grows into a region whose outer
//! boundary ("source" edges) is held at φ = 1; the other edges are insulating mirrors
//! (∂φ/∂n = 0). Between growth steps φ solves Laplace's equation, relaxed by
//! successive over-relaxation (SOR) warm-started from the last solution, a few sweeps
//! per frame with the sweeps capped and stopped early once the largest update falls
//! below a tolerance. A perimeter site (a free 8-neighbour of the tree) joins with
//! probability `∝ |∇φ|^η`; next to the φ = 0 tree the field is `φ(site)` over one
//! lattice spacing, so the weight is `φ^η`. η = 1 is DLA-like coral (D ≈ 1.7), larger η
//! thins the tree toward lightning (D ≈ 1.3 at η = 3).
//!
//! The same equations describe viscous fingering in a Hele-Shaw cell (Paterson 1984),
//! with φ the pressure and the tree the invading low-viscosity fluid.
//!
//! Tree nodes live in a fixed-capacity pool in growth order with parent links, so a
//! parent always precedes its children; nothing allocates after [`Dbm::new`].

use rand::Rng;
use sciart_core::Size;

/// Boundary condition of one lattice edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// Held at φ = 1: the charge (or the far, high-pressure fluid).
    Source,
    /// Insulating: zero normal gradient.
    Mirror,
}

/// Edge order in [`Dbm::reset`]: top, right, bottom, left.
pub type Edges = [Edge; 4];

/// One tree site: lattice position, parent index (the root is its own parent) and the
/// growth time stamp the caller passed to [`Dbm::grow`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub x: u16,
    pub y: u16,
    pub parent: u32,
    pub born: f32,
}

const FREE: u8 = 0;
const TREE: u8 = 1;
const NO_NODE: u32 = u32::MAX;

/// SOR relaxation factor: near the optimum `2/(1 + π/N)` for the capped lattices (N ≈ 80),
/// kept a little lower so a warm start never overshoots visibly.
pub const OMEGA: f32 = 1.9;

/// The 8-neighbourhood, orthogonal steps first.
const HOOD: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
];

/// A dielectric-breakdown lattice and its tree.
#[derive(Debug, Clone, Default)]
pub struct Dbm {
    w: usize,
    h: usize,
    phi: Vec<f32>,
    kind: Vec<u8>,
    /// 1 on free sites, 0 on the tree: a branch-free mask for the SOR interior.
    free: Vec<f32>,
    node_at: Vec<u32>,
    nodes: Vec<Node>,
    cap: usize,
    edges: Option<Edges>,
    /// Perimeter candidates and their cumulative weights, rebuilt per [`Dbm::grow`].
    cand: Vec<u32>,
    cum: Vec<f64>,
    /// Per-site stamp so a candidate is listed once per rebuild.
    stamp: Vec<u32>,
    epoch: u32,
    seed: (usize, usize),
    reach: f64,
    touched: bool,
}

impl Dbm {
    /// A `size` lattice with room for `cap` tree nodes; empty until [`Self::reset`].
    pub fn new(size: Size, cap: usize) -> Self {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let n = w * h;
        Self {
            w,
            h,
            phi: vec![1.0; n],
            kind: vec![FREE; n],
            free: vec![1.0; n],
            node_at: vec![NO_NODE; n],
            nodes: Vec::with_capacity(cap),
            cap,
            edges: None,
            cand: Vec::with_capacity(8 * cap.min(n)),
            cum: Vec::with_capacity(8 * cap.min(n)),
            stamp: vec![0; n],
            epoch: 0,
            seed: (0, 0),
            reach: 0.0,
            touched: false,
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.w as u16, self.h as u16)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn is_full(&self) -> bool {
        self.nodes.len() >= self.cap
    }

    /// Largest distance of a tree site from the seed, in lattice spacings.
    pub fn reach(&self) -> f64 {
        self.reach
    }

    /// Whether the tree has reached a source edge (breakdown complete).
    pub fn touched_source(&self) -> bool {
        self.touched
    }

    #[cfg(test)]
    pub fn phi(&self, x: usize, y: usize) -> f32 {
        self.phi[y * self.w + x]
    }

    /// Distance from the seed to the nearest source edge (the reach at breakdown).
    pub fn span(&self) -> f64 {
        let (sx, sy) = (self.seed.0 as f64, self.seed.1 as f64);
        let d = [sy, self.w as f64 - 1.0 - sx, self.h as f64 - 1.0 - sy, sx];
        let edges = self.edges.unwrap_or([Edge::Mirror; 4]);
        (0..4)
            .filter(|&k| edges[k] == Edge::Source)
            .map(|k| d[k])
            .fold(f64::INFINITY, f64::min)
            .max(1.0)
    }

    /// Clear the tree, plant the root at `seed` (clamped onto the lattice) and set the
    /// boundary, with a warm-start guess `φ = ln(1 + d)/ln(1 + span)` (d from the seed,
    /// the potential of a point charge), so a few sweeps per frame suffice.
    pub fn reset(&mut self, seed: (usize, usize), edges: Edges) {
        if self.w * self.h == 0 {
            return;
        }
        for &Node { x, y, .. } in &self.nodes {
            let i = usize::from(y) * self.w + usize::from(x);
            self.kind[i] = FREE;
            self.free[i] = 1.0;
            self.node_at[i] = NO_NODE;
        }
        self.nodes.clear();
        self.seed = (seed.0.min(self.w - 1), seed.1.min(self.h - 1));
        self.edges = Some(edges);
        self.reach = 0.0;
        self.touched = false;
        let norm = 1.0 / (1.0 + self.span()).ln();
        let (sx, sy) = (self.seed.0 as f64, self.seed.1 as f64);
        for y in 0..self.h {
            for x in 0..self.w {
                let d = (x as f64 - sx).hypot(y as f64 - sy);
                self.phi[y * self.w + x] = ((1.0 + d).ln() * norm).min(1.0) as f32;
            }
        }
        self.add(self.seed.0, self.seed.1, 0, 0.0);
    }

    fn add(&mut self, x: usize, y: usize, parent: u32, born: f32) -> bool {
        if self.nodes.len() >= self.cap {
            return false;
        }
        let i = y * self.w + x;
        let id = self.nodes.len() as u32;
        let parent = if self.nodes.is_empty() { id } else { parent };
        self.nodes.push(Node {
            x: x as u16,
            y: y as u16,
            parent,
            born,
        });
        self.kind[i] = TREE;
        self.free[i] = 0.0;
        self.node_at[i] = id;
        self.phi[i] = 0.0;
        let d = (x as f64 - self.seed.0 as f64).hypot(y as f64 - self.seed.1 as f64);
        self.reach = self.reach.max(d);
        if let Some(edges) = self.edges {
            let near = [y <= 1, x + 2 >= self.w, y + 2 >= self.h, x <= 1];
            self.touched |= (0..4).any(|k| edges[k] == Edge::Source && near[k]);
        }
        true
    }

    /// The Gauss–Seidel target of free site `(x, y)`: the mean of its in-lattice
    /// 4-neighbours, with a source edge's ghost at 1 (a mirror edge drops out).
    #[inline]
    fn target(&self, x: usize, y: usize, edges: &Edges) -> f32 {
        let (w, i) = (self.w, y * self.w + x);
        let (mut s, mut n) = (0.0f32, 0.0f32);
        let mut side = |inside: bool, j: usize, edge: Edge| {
            if inside {
                s += self.phi[j];
                n += 1.0;
            } else if edge == Edge::Source {
                s += 1.0;
                n += 1.0;
            }
        };
        side(y > 0, i.wrapping_sub(w), edges[0]);
        side(x + 1 < w, i + 1, edges[1]);
        side(y + 1 < self.h, i + w, edges[2]);
        side(x > 0, i.wrapping_sub(1), edges[3]);
        if n > 0.0 { s / n } else { self.phi[i] }
    }

    /// Up to `max_sweeps` red–black SOR sweeps over the free sites, stopping after the
    /// first sweep whose largest update `|Δφ|` is below `tol`; returns the sweeps run and
    /// that sweep's largest update (LICH-A1). Red–black ordering has no dependency between
    /// the sites of one colour, so the interior loop pipelines (lexicographic
    /// Gauss–Seidel chains every site on its left neighbour's fresh value).
    pub fn relax(&mut self, max_sweeps: usize, tol: f32) -> (usize, f32) {
        let Some(edges) = self.edges else {
            return (0, 0.0);
        };
        let (w, h) = (self.w, self.h);
        let mut largest = 0.0;
        for sweep in 1..=max_sweeps {
            largest = 0.0f32;
            for colour in 0..2 {
                for y in 0..h {
                    let first = (colour + y) % 2;
                    if y == 0 || y + 1 == h || w < 3 {
                        for x in (first..w).step_by(2) {
                            largest = largest.max(self.relax_rim(x, y, &edges));
                        }
                        continue;
                    }
                    // Rim columns through the boundary-aware target.
                    if first == 0 {
                        largest = largest.max(self.relax_rim(0, y, &edges));
                    }
                    if (w - 1) % 2 == first {
                        largest = largest.max(self.relax_rim(w - 1, y, &edges));
                    }
                    let (top, tail) = self.phi.split_at_mut(y * w);
                    let (mid, bottom) = tail.split_at_mut(w);
                    let (up, down) = (&top[(y - 1) * w..], &bottom[..w]);
                    let free = &self.free[y * w..(y + 1) * w];
                    let start = if first == 0 { 2 } else { 1 };
                    for x in (start..w - 1).step_by(2) {
                        let t = 0.25 * ((mid[x - 1] + mid[x + 1]) + (up[x] + down[x]));
                        let delta = (t - mid[x]) * free[x];
                        mid[x] = (mid[x] + OMEGA * delta).clamp(0.0, 1.0);
                        largest = largest.max(delta.abs());
                    }
                }
            }
            if largest < tol {
                return (sweep, largest);
            }
        }
        (max_sweeps, largest)
    }

    /// One SOR update of rim site `(x, y)`; returns `|Δφ|` (0 for a tree site).
    fn relax_rim(&mut self, x: usize, y: usize, edges: &Edges) -> f32 {
        let i = y * self.w + x;
        if self.kind[i] != FREE {
            return 0.0;
        }
        let delta = self.target(x, y, edges) - self.phi[i];
        self.phi[i] = (self.phi[i] + OMEGA * delta).clamp(0.0, 1.0);
        delta.abs()
    }

    /// Largest Laplace residual `|mean(neighbours) − φ|` over the free sites.
    #[cfg(test)]
    pub fn residual(&self) -> f32 {
        let Some(edges) = self.edges else {
            return 0.0;
        };
        let mut r = 0.0f32;
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                if self.kind[i] == FREE {
                    r = r.max((self.target(x, y, &edges) - self.phi[i]).abs());
                }
            }
        }
        r
    }

    /// Add up to `max_sites` perimeter sites (stopping early once the reach is
    /// `until_reach`; touching a source edge does not stop growth), each drawn with probability `∝ φ^η` from
    /// the perimeter as it stood at the call, and linked to its oldest tree neighbour
    /// (so branches hang off the trunk rather than their siblings). Stops when the pool
    /// is full; returns the sites added.
    pub fn grow(
        &mut self,
        eta: f64,
        max_sites: usize,
        until_reach: f64,
        born: f32,
        rng: &mut impl Rng,
    ) -> usize {
        if self.edges.is_none() || self.nodes.is_empty() {
            return 0;
        }
        self.rebuild_perimeter(eta);
        let Some(&total) = self.cum.last() else {
            return 0;
        };
        if total.is_nan() || total <= 0.0 {
            return 0;
        }
        let mut added = 0;
        let mut tries = 0;
        while added < max_sites
            && tries < 4 * max_sites
            && !self.is_full()
            && self.reach < until_reach
        {
            tries += 1;
            let u = rng.random::<f64>() * total;
            let k = self
                .cum
                .partition_point(|&c| c <= u)
                .min(self.cand.len() - 1);
            let i = self.cand[k] as usize;
            if self.kind[i] != FREE {
                continue;
            }
            let (x, y) = (i % self.w, i / self.w);
            let Some(parent) = self.oldest_neighbour(x, y) else {
                continue;
            };
            if self.add(x, y, parent, born) {
                added += 1;
            }
        }
        added
    }

    fn oldest_neighbour(&self, x: usize, y: usize) -> Option<u32> {
        HOOD.iter()
            .filter_map(|&(dx, dy)| {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx as usize >= self.w || ny as usize >= self.h {
                    return None;
                }
                let id = self.node_at[ny as usize * self.w + nx as usize];
                (id != NO_NODE).then_some(id)
            })
            .min()
    }

    fn rebuild_perimeter(&mut self, eta: f64) {
        self.cand.clear();
        self.cum.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.stamp.fill(0);
            self.epoch = 1;
        }
        let mut total = 0.0f64;
        for node in &self.nodes {
            for &(dx, dy) in &HOOD {
                let (nx, ny) = (i32::from(node.x) + dx, i32::from(node.y) + dy);
                if nx < 0 || ny < 0 || nx as usize >= self.w || ny as usize >= self.h {
                    continue;
                }
                let j = ny as usize * self.w + nx as usize;
                if self.kind[j] != FREE || self.stamp[j] == self.epoch {
                    continue;
                }
                self.stamp[j] = self.epoch;
                let phi = f64::from(self.phi[j].max(0.0));
                // Integer η (the scene's) avoids a libm `powf` per candidate.
                let wgt = match eta {
                    1.0 => phi,
                    0.75 => {
                        let r = phi.sqrt();
                        r * r.sqrt()
                    }
                    1.5 => phi * phi.sqrt(),
                    2.0 => phi * phi,
                    3.0 => phi * phi * phi,
                    _ => phi.powf(eta),
                };
                if wgt > 0.0 && self.cand.len() < self.cand.capacity() {
                    total += wgt;
                    self.cand.push(j as u32);
                    self.cum.push(total);
                }
            }
        }
    }
}

/// Box-counting dimension of a node set: the log-log slope of occupied boxes against
/// 1/box size over sizes 1, 2, 4, 8 (least squares) (LICH-A3).
#[cfg(test)]
pub fn box_dimension(nodes: &[Node]) -> f64 {
    use std::collections::HashSet;
    let pts: Vec<(f64, f64)> = [1u16, 2, 4, 8]
        .iter()
        .map(|&s| {
            let boxes: HashSet<(u16, u16)> = nodes.iter().map(|n| (n.x / s, n.y / s)).collect();
            ((1.0 / f64::from(s)).ln(), (boxes.len() as f64).ln())
        })
        .collect();
    let n = pts.len() as f64;
    let (mx, my) = (
        pts.iter().map(|p| p.0).sum::<f64>() / n,
        pts.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    sxy / sxx
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::Ctx;

    fn rng(seed: u64) -> impl Rng {
        Ctx::new(seed, Size::new(1, 1)).rng
    }

    const FAN: Edges = [Edge::Source, Edge::Source, Edge::Mirror, Edge::Source];

    fn grown(eta: f64, seed: u64) -> Dbm {
        let mut d = Dbm::new(Size::new(120, 60), 6000);
        d.reset((60, 59), FAN);
        d.relax(400, 1e-5);
        let mut rng = rng(seed);
        let mut t = 0.0;
        while !d.touched_source() && !d.is_full() {
            d.grow(eta, 3, f64::INFINITY, t, &mut rng);
            d.relax(20, 1e-4);
            t += 1.0;
        }
        d
    }

    #[test]
    fn lich_a1_sor_converges_from_a_warm_start_within_the_cap() {
        let mut d = grown(2.0, 3);
        d.relax(2000, 1e-6);
        // A few new sites disturb the field only locally: the warm start converges fast.
        let mut rng = rng(9);
        d.reset((60, 59), FAN);
        d.relax(3000, 1e-6);
        assert!(d.residual() < 1e-5, "cold solve {}", d.residual());
        for k in 0..10 {
            d.grow(2.0, 3, f64::INFINITY, k as f32, &mut rng);
            let (sweeps, largest) = d.relax(60, 1e-3);
            assert!(
                largest < 1e-3 && sweeps <= 60,
                "step {k}: {sweeps} sweeps, {largest}"
            );
            assert!(d.residual() < 2e-3);
        }
        // The field is a potential: 0 on the tree, in [0, 1] everywhere.
        for y in 0..60 {
            for x in 0..120 {
                assert!((0.0..=1.0).contains(&d.phi(x, y)));
            }
        }
    }

    #[test]
    fn lich_a2_every_node_links_to_an_earlier_adjacent_node() {
        let d = grown(1.0, 5);
        for (i, n) in d.nodes().iter().enumerate() {
            let p = d.nodes()[n.parent as usize];
            if i == 0 {
                assert_eq!(n.parent, 0);
                continue;
            }
            assert!((n.parent as usize) < i);
            let (dx, dy) = (
                i32::from(n.x) - i32::from(p.x),
                i32::from(n.y) - i32::from(p.y),
            );
            assert!(dx.abs() <= 1 && dy.abs() <= 1 && (dx, dy) != (0, 0));
        }
    }

    #[test]
    fn lich_a3_higher_eta_grows_a_sparser_tree() {
        let (mut dense, mut sparse) = (0.0, 0.0);
        for seed in 0..3 {
            dense += box_dimension(grown(1.0, seed).nodes());
            sparse += box_dimension(grown(3.0, seed).nodes());
        }
        assert!(dense > sparse + 0.15, "D(η=1) {dense} vs D(η=3) {sparse}");
    }

    #[test]
    fn lich_a4_pool_never_exceeds_capacity_and_reset_empties_it() {
        let mut d = Dbm::new(Size::new(40, 20), 50);
        d.reset((20, 19), FAN);
        let mut rng = rng(1);
        for k in 0..100 {
            d.grow(1.0, 7, f64::INFINITY, k as f32, &mut rng);
            assert!(d.nodes().len() <= 50);
        }
        assert!(d.is_full());
        assert_eq!(d.grow(1.0, 7, f64::INFINITY, 0.0, &mut rng), 0);
        d.reset((0, 0), [Edge::Source; 4]);
        assert_eq!(d.nodes().len(), 1);
        assert_eq!(d.capacity(), 50);
    }

    #[test]
    fn degenerate_lattices_do_not_panic() {
        let mut rng = rng(1);
        for (w, h) in [(1, 1), (1, 3), (3, 1)] {
            let mut d = Dbm::new(Size::new(w, h), 8);
            d.reset((5, 5), [Edge::Source; 4]);
            d.relax(3, 1e-4);
            d.grow(2.0, 4, f64::INFINITY, 0.0, &mut rng);
            assert!(d.nodes().len() <= 8);
        }
        let mut empty = Dbm::default();
        empty.reset((0, 0), [Edge::Source; 4]);
        assert_eq!(empty.relax(3, 0.0), (0, 0.0));
        assert_eq!(empty.grow(1.0, 3, f64::INFINITY, 0.0, &mut rng), 0);
    }
}
