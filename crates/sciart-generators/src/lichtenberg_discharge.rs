//! A Lichtenberg figure: the branching discharge tree that burns into a block of acrylic
//! charged by an electron beam when it is tapped, in four acts (SPEC-002
//! `lichtenberg_discharge`).
//!
//! Model: the dielectric breakdown model of the shared [`laplacian_growth`] module on a
//! capped lattice of square sites (ADR-0012). The tree is the discharge channel at φ = 0,
//! the charged acrylic's far edges are at φ = 1, and a perimeter site breaks down with
//! probability `∝ |∇φ|^η`. Across the acts η rises from 0.75 (dense coral) to 1.5 (fern) to 3
//! (lightning), and a fourth act shows the same Laplacian growth as viscous fingering: air
//! pushed from a central injection point into a Hele-Shaw cell of oil (η = 1).
//!
//! Pacing: breakdown is nanoseconds in acrylic, so time is stretched. The tree's reach
//! from the tap follows an eased schedule toward the nearest charged edge (sites are added
//! until the reach catches up, at most [`MAX_SITES`] a frame), so every η takes about the
//! same time to cross. When a branch reaches the edge the whole channel flashes, glows
//! and fades out, and the next act starts on a fresh block.
//!
//! Picture (M15, ADR-0018: M11's idiom): one braille dot layer. Each tree link is a 1-dot
//! line in the act's semantic colour (`Wave` for the discharges, `Cold` for the fingering
//! air), in three flat steps by the log of the current it carries (the size of its
//! subtree): twigs dim, limbs brighter, the trunk brightest; only the few links carrying
//! most of the current are doubled into a 2-dot stroke, so the trunk reads as the
//! channel. Newly broken-down tips, and the whole tree when a branch strikes, flash `Star`
//! white. In ASCII the same dots show by dot count. Rasterised once per drawn frame in
//! `prepare` (GEN-R20); `render` only blits.

use rand::Rng;
use sciart_core::grid;
use sciart_core::prelude::*;

use crate::shared::laplacian_growth::{Dbm, Edge, Edges};

const CAPTIONS: [&str; 4] = ["dense", "fern", "lightning", "fingering"];

pub static META: Meta = Meta {
    name: "lichtenberg_discharge",
    category: Category::PhysicalChemistry,
    description: "Dielectric breakdown burns a branching Lichtenberg tree into acrylic",
    title: "Lichtenberg figure · breakdown",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Lichtenberg::default())
}

/// Lattice node budget and aspect bucket (ADR-0012).
const MAX_NODES: usize = 160 * 80;
const BUCKET: f64 = 1.25;
/// Tree pool capacity: a dense η = 0.75 tree across the capped lattice has ≈ 2000 sites.
pub(crate) const CAPACITY: usize = 4096;
/// Seconds per act, the growth schedule inside it, and the fade at its end.
pub(crate) const ACT_S: f64 = 13.0;
const GROW_S: f64 = 8.5;
const FADE_S: f64 = 0.8;
/// Sites added per update at most, and SOR sweeps per update with their stop tolerance.
pub(crate) const MAX_SITES: usize = 24;
const SWEEPS: usize = 10;
const TOL: f32 = 2e-4;
/// Seconds a new tip stays hot, and the breakdown flash's decay.
const TIP_S: f32 = 0.5;
const FLASH_S: f64 = 0.9;
/// The scene opens this far into the first act, its tree pre-grown in `resize` to the
/// schedule's reach there (≈ 40 % of the block), so the first frame is a real figure.
pub(crate) const START_S: f64 = 2.0;
/// Pre-roll cap: growth rounds (each a few SOR sweeps and one batch of sites) per
/// `resize`, ≈ 10 ms at most.
const PREROLL_ROUNDS: usize = 80;
const PREROLL_SWEEPS: usize = 6;

/// One act: caption index, η, where the tap is (fractions of the lattice), which edges
/// hold the charge, the line's role, and whether its channels are wide.
#[derive(Debug, Clone, Copy)]
struct Act {
    eta: f64,
    seed: (f64, f64),
    edges: Edges,
    role: Role,
    wide: bool,
    /// The act completes when the reach from the tap is this fraction of the lattice
    /// height (the tree fills the block), or, when `None`, when a branch reaches a
    /// charged edge (the lightning strike).
    fill: Option<f64>,
}

const S: Edge = Edge::Source;
const M: Edge = Edge::Mirror;

const ACTS: [Act; 4] = [
    // Tapped at the bottom edge; the charge is held at the top, and the sides are
    // insulating, so branches fan up through the block rather than creep along an edge.
    Act {
        eta: 0.75,
        seed: (0.5, 1.0),
        edges: [S, M, M, M],
        role: Role::Wave,
        wide: false,
        fill: Some(0.85),
    },
    Act {
        eta: 1.5,
        seed: (0.5, 1.0),
        edges: [S, M, M, M],
        role: Role::Wave,
        wide: false,
        fill: Some(0.85),
    },
    // From the top toward the grounded bottom edge, like a leader from a cloud.
    Act {
        eta: 3.0,
        seed: (0.5, 0.0),
        edges: [M, M, S, M],
        role: Role::Wave,
        wide: false,
        fill: None,
    },
    // Injection at the centre of a Hele-Shaw cell.
    Act {
        eta: 1.0,
        seed: (0.5, 0.5),
        edges: [S, S, S, S],
        role: Role::Cold,
        wide: true,
        fill: Some(0.45),
    },
];

/// The capped simulation lattice for a display, in square units (cells are ≈ 2:1 tall).
pub(crate) fn lattice_size(display: Size) -> Size {
    let square = Size::new(display.width(), display.height().saturating_mul(2));
    grid::lattice_for(square, MAX_NODES, BUCKET)
}

/// The act at scene time `t` and its identity `(cycle, index)`.
fn act_at(timeline: &Timeline, t: f64) -> (Beat, (i64, usize)) {
    let beat = timeline.at(t);
    (beat, (timeline.cycle(t), beat.index))
}

/// Reach the tree should have at `local` seconds into an act, as a fraction of the span.
pub(crate) fn reach_schedule(local: f64) -> f64 {
    let x = (local / GROW_S).clamp(0.0, 1.0);
    // Fast start, gentle arrival: the first branches spread at once.
    1.0 - (1.0 - x) * (1.0 - x)
}

/// Links whose current is at least this (of the whole tree's, on the log scale of
/// [`link_brightness`]) are doubled into a 2-dot stroke: the trunk and main limbs; in the
/// fingering act the air channels are wider, from `TRUNK_WIDE`.
const TRUNK: f32 = 0.8;
const TRUNK_WIDE: f32 = 0.6;

/// The three flat line levels (twig, limb, trunk) and the white of a hot tip or flash.
const LEVELS: [u8; 3] = [110, 180, 255];
const HOT_INK: Ink = Ink::new(Role::Star, 255);
/// Current from which a link is a limb rather than a twig.
const LIMB: f32 = 0.5;

/// A link's ink at base brightness `base` ([`link_brightness`] without the tip flash),
/// heat `hot ∈ [0, 1]` (a fresh tip, or the breakdown flash), fading out by `fade` at the
/// act's end, in `role`: white while hot, else the twig, limb or trunk step (LICH-A6);
/// in the fade's first half a step down, then gone.
pub(crate) fn link_ink(role: Role, base: f32, hot: f32, fade: f64, trunk: f32) -> Ink {
    if fade < 0.2 {
        return Ink::BLANK;
    }
    if hot >= 0.5 && fade >= 0.6 {
        return HOT_INK;
    }
    let step = if base >= trunk {
        2
    } else {
        usize::from(base >= LIMB)
    };
    let step = if fade < 0.6 {
        step.saturating_sub(1)
    } else {
        step
    };
    Ink::new(role, LEVELS[step])
}

/// Brightness in [0, 1] of a link carrying `sub` of the tree's `total` sites, born at
/// `born`, at time `now`: log current, plus a hot flash on new tips. (`rasterise` uses
/// the two parts separately, through [`line_level`].)
#[cfg(test)]
pub(crate) fn link_brightness(sub: u32, total: u32, born: f32, now: f32) -> f32 {
    link_current(sub, total).max(tip_heat(born, now))
}

/// The log-current part of [`link_brightness`], `0.2 + 0.8·ln(1 + sub)/ln(1 + total)`.
fn link_current(sub: u32, total: u32) -> f32 {
    let current = (1.0 + sub as f32).ln() / (1.0 + total.max(1) as f32).ln();
    0.2 + 0.8 * current.clamp(0.0, 1.0)
}

/// A new tip's heat, 1 when it breaks down, gone after [`TIP_S`].
fn tip_heat(born: f32, now: f32) -> f32 {
    let tip = (1.0 - (now - born).max(0.0) / TIP_S).max(0.0);
    tip * tip
}

#[derive(Debug)]
pub struct Lichtenberg {
    size: Size,
    dbm: Dbm,
    timeline: Timeline,
    t: f64,
    act_id: Option<(i64, usize)>,
    act: usize,
    /// When the tree reached a charged edge in this act (scene time).
    done_at: Option<f64>,
    /// How far the tree has grown across the block from the tap (rows), which the
    /// filling acts pace and complete on: a sideways branch does not fill the block.
    depth: f64,
    /// Subtree sizes (current per link), sized to the pool.
    sub: Vec<u32>,
    dots: Dots,
    /// The dots are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Lichtenberg {
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            dbm: Dbm::default(),
            timeline: Timeline::new(&[
                (CAPTIONS[0], ACT_S),
                (CAPTIONS[1], ACT_S),
                (CAPTIONS[2], ACT_S),
                (CAPTIONS[3], ACT_S),
            ]),
            t: START_S,
            act_id: None,
            act: 0,
            done_at: None,
            depth: 0.0,
            sub: Vec::new(),
            dots: Dots::default(),
            dirty: true,
        }
    }
}

impl Lichtenberg {
    /// Plant a fresh tree for act `index` (LICH-A4: the pool resets at the beat).
    fn start_act(&mut self, index: usize) {
        self.act = index % ACTS.len();
        let act = ACTS[self.act];
        let s = self.dbm.size();
        let (w, h) = (f64::from(s.width()), f64::from(s.height()));
        let seed = (
            (act.seed.0 * (w - 1.0)).round().max(0.0) as usize,
            (act.seed.1 * (h - 1.0)).round().max(0.0) as usize,
        );
        self.dbm.reset(seed, act.edges);
        self.done_at = None;
        self.depth = 0.0;
    }

    /// The depth (filling acts) or reach (strike) at which the act completes (see
    /// [`Act::fill`]), in lattice spacings.
    pub(crate) fn goal(&self) -> f64 {
        let h = f64::from(self.dbm.size().height());
        match ACTS[self.act].fill {
            Some(f) => f * (h - 1.0).max(1.0),
            None => 1.5 * self.dbm.span() + 3.0,
        }
    }

    /// What the schedule paces: depth for a filling act, reach for the strike.
    fn progress(&self, act: Act) -> f64 {
        if act.fill.is_some() {
            self.depth
        } else {
            self.dbm.reach()
        }
    }

    /// Grow toward the schedule's target for `local`, at most [`MAX_SITES`] sites, two per
    /// perimeter snapshot (a whole batch drawn from one snapshot lands side by side on the
    /// hottest tips and thickens them into blobs). Returns the sites added.
    fn grow_toward(&mut self, local: f64, born: f32, rng: &mut impl Rng) -> usize {
        let act = ACTS[self.act];
        // Aim a little past the goal, so the schedule's end is reached, not approached.
        let target = reach_schedule(local) * (1.05 * self.goal() + 2.0);
        let mut added = 0;
        while added < MAX_SITES && self.progress(act) < target {
            let n = self.dbm.grow(act.eta, 2, f64::INFINITY, born, rng);
            if n == 0 {
                break;
            }
            added += n;
            let nodes = self.dbm.nodes();
            let root = f64::from(nodes[0].y);
            for node in &nodes[nodes.len() - n..] {
                self.depth = self.depth.max((f64::from(node.y) - root).abs());
            }
        }
        added
    }

    fn step_tree(&mut self, local: f64, rng: &mut impl Rng) {
        let act = ACTS[self.act];
        self.dbm.relax(SWEEPS, TOL);
        if self.done_at.is_some() || self.dbm.is_full() {
            return;
        }
        self.grow_toward(local, self.t as f32, rng);
        let complete = match act.fill {
            Some(_) => self.depth >= self.goal(),
            None => self.dbm.touched_source(),
        };
        if complete || self.dbm.is_full() || local > GROW_S + 1.5 {
            self.done_at = Some(self.t);
        }
    }

    /// Catch a fresh tree up with the schedule at `local` in one go (in `resize`: the
    /// scene's opening and a lattice change), with its sites born long enough ago that
    /// they do not flash as new tips. Stops short of completing the act.
    fn preroll(&mut self, local: f64, rng: &mut impl Rng) {
        let born = (self.t - f64::from(TIP_S)) as f32;
        let local = local.min(GROW_S * 0.8);
        for _ in 0..PREROLL_ROUNDS {
            self.dbm.relax(PREROLL_SWEEPS, TOL);
            if self.dbm.is_full() || self.grow_toward(local, born, rng) == 0 {
                break;
            }
        }
        self.dbm.relax(SWEEPS, TOL);
    }

    /// Current through each link: subtree sizes by one reverse pass (parents precede
    /// their children in the pool).
    fn currents(&mut self) {
        let nodes = self.dbm.nodes();
        self.sub.clear();
        self.sub.resize(nodes.len(), 1);
        for i in (1..nodes.len()).rev() {
            let p = nodes[i].parent as usize;
            if p < i {
                self.sub[p] += self.sub[i];
            }
        }
    }

    fn rasterise(&mut self, local: f64) {
        self.currents();
        self.dots.clear();
        let act = ACTS[self.act];
        let lat = self.dbm.size();
        let dot = self.dots.dot_size();
        let sx = f64::from(dot.width()) / f64::from(lat.width().max(1));
        let sy = f64::from(dot.height()) / f64::from(lat.height().max(1));
        let at = |x: u16, y: u16| {
            (
                (f64::from(x) + 0.5) * sx - 0.5,
                (f64::from(y) + 0.5) * sy - 0.5,
            )
        };
        let flash = self
            .done_at
            .map_or(0.0, |d| (-(self.t - d) / FLASH_S).exp()) as f32;
        let fade = ((ACT_S - local) / FADE_S).clamp(0.0, 1.0);
        let now = self.t as f32;
        let nodes = self.dbm.nodes();
        let total = self.sub.first().copied().unwrap_or(1);
        let trunk = if act.wide { TRUNK_WIDE } else { TRUNK };
        for (i, n) in nodes.iter().enumerate() {
            let base = link_current(self.sub[i], total);
            let hot = tip_heat(n.born, now).max(flash);
            let ink = link_ink(act.role, base, hot, fade, trunk);
            if ink == Ink::BLANK {
                continue;
            }
            let p = nodes[n.parent as usize];
            let (a, c) = (at(p.x, p.y), at(n.x, n.y));
            self.dots.line(a.0, a.1, c.0, c.1, ink);
            // The links carrying most of the current: a second line one dot across, so
            // the trunk is a solid 2-dot channel and the rest hairlines.
            if base >= trunk {
                let (ox, oy) = if (c.0 - a.0).abs() >= (c.1 - a.1).abs() {
                    (0.0, 1.0)
                } else {
                    (1.0, 0.0)
                };
                self.dots.line(a.0 + ox, a.1 + oy, c.0 + ox, c.1 + oy, ink);
            }
        }
    }
}

impl Generator for Lichtenberg {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.act])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let lattice = lattice_size(size);
        if self.dbm.size() != lattice || self.dbm.nodes().is_empty() {
            self.dbm = Dbm::new(lattice, CAPACITY);
            self.sub = Vec::with_capacity(CAPACITY);
            let (beat, id) = act_at(&self.timeline, self.t);
            self.act_id = Some(id);
            self.start_act(beat.index);
            // The opening, or a lattice change mid-act: grow the fresh tree to where the
            // act's schedule is, so the figure never starts over as a stub.
            self.dbm.relax(4 * SWEEPS, TOL);
            self.preroll(beat.local, &mut ctx.rng);
        }
        self.size = size;
        self.dots.resize(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt >= 0.0) || self.dbm.nodes().is_empty() {
            return;
        }
        self.t += dt;
        let (beat, id) = act_at(&self.timeline, self.t);
        if self.act_id != Some(id) {
            self.act_id = Some(id);
            self.start_act(beat.index);
        }
        self.step_tree(beat.local, &mut ctx.rng);
        self.dirty = true;
    }

    /// Rasterise once per drawn frame, not once per step (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            let local = act_at(&self.timeline, self.t).0.local;
            self.rasterise(local);
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        // Braille in Unicode; by dot count (`. : + #`) in ASCII (GEN-R10).
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(w: u16, h: u16) -> (Lichtenberg, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(42, size);
        let mut g = Lichtenberg::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn lich_a2_scene_tree_stays_connected_through_an_act() {
        let (mut g, mut ctx) = scene(120, 40);
        for _ in 0..(8.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            let nodes = g.dbm.nodes();
            for (i, n) in nodes.iter().enumerate().skip(1) {
                assert!((n.parent as usize) < i, "orphan {i}");
            }
        }
        assert!(g.dbm.nodes().len() > 50);
    }

    #[test]
    fn lich_a4_act_change_resets_the_pool_within_capacity() {
        let (mut g, mut ctx) = scene(100, 30);
        let mut peak = 0;
        // Just past the first beat (the scene opens START_S into the act).
        for _ in 0..((ACT_S - START_S + 0.2) / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
            peak = peak.max(g.dbm.nodes().len());
            assert!(g.dbm.nodes().len() <= CAPACITY);
        }
        assert!(peak > 100, "peak {peak}");
        assert!(g.dbm.nodes().len() < peak / 4, "not reset at the beat");
        assert_eq!(g.dbm.capacity(), CAPACITY);
    }

    #[test]
    fn lich_a5_captions_follow_eta() {
        let (mut g, mut ctx) = scene(80, 24);
        let mut seen = Vec::new();
        for k in 0..4 {
            while g.t < f64::from(k) * ACT_S + 1.0 {
                g.update(0.25, &mut ctx);
            }
            seen.push((g.caption(), ACTS[g.act].eta));
        }
        assert_eq!(
            seen,
            vec![
                (Some("dense"), 0.75),
                (Some("fern"), 1.5),
                (Some("lightning"), 3.0),
                (Some("fingering"), 1.0)
            ]
        );
    }

    #[test]
    fn lich_a3_tree_fills_the_block_or_strikes_within_the_act() {
        for (act, spec) in ACTS.iter().enumerate() {
            let (mut g, mut ctx) = scene(120, 40);
            g.start_act(act);
            let mut local = 0.0;
            while local < GROW_S + 1.0 && g.done_at.is_none() {
                local += SIM_DT;
                g.t += SIM_DT;
                g.step_tree(local, &mut ctx.rng);
            }
            let ok = match spec.fill {
                Some(_) => g.depth >= g.goal(),
                None => g.dbm.touched_source(),
            };
            assert!(
                ok,
                "act {act} stalled at reach {} of {}",
                g.dbm.reach(),
                g.goal()
            );
        }
    }

    #[test]
    fn brightness_is_highest_on_the_trunk_and_on_new_tips() {
        let trunk = link_brightness(900, 1000, 0.0, 10.0);
        let twig = link_brightness(1, 1000, 0.0, 10.0);
        let tip = link_brightness(1, 1000, 10.0, 10.0);
        assert!(
            trunk > 0.9 && twig < 0.5 && tip > 0.99,
            "{trunk} {twig} {tip}"
        );
        assert!(reach_schedule(0.0) == 0.0 && reach_schedule(GROW_S) == 1.0);
    }

    #[test]
    fn lich_a6_lines_are_flat_steps_except_hot_tips_and_the_flash() {
        // M15 (ADR-0018): twigs, limbs and trunk in three flat steps of the act's role; a
        // fresh tip or the strike flash is white; the act's fade steps down, then out.
        let role = ACTS[0].role;
        assert_eq!(
            link_ink(role, 0.0, 0.0, 1.0, TRUNK),
            Ink::new(role, LEVELS[0])
        );
        assert_eq!(
            link_ink(role, LIMB, 0.0, 1.0, TRUNK),
            Ink::new(role, LEVELS[1])
        );
        assert_eq!(
            link_ink(role, 1.0, 0.0, 1.0, TRUNK),
            Ink::new(role, LEVELS[2])
        );
        assert_eq!(link_ink(role, 0.3, 1.0, 1.0, TRUNK), HOT_INK);
        assert_eq!(
            link_ink(role, 1.0, 0.0, 0.4, TRUNK),
            Ink::new(role, LEVELS[1])
        );
        assert_eq!(link_ink(role, 1.0, 0.0, 0.1, TRUNK), Ink::BLANK);
        // Seen in dots after the scene's first frames: every lit cell is the act's role or
        // a hot tip, and apart from the trunk the links are twigs and limbs.
        let (mut g, mut ctx) = scene(120, 40);
        for _ in 0..90 {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        let mut c = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut c);
        let lit: Vec<Ink> = c
            .cells()
            .iter()
            .filter(|c| c.ink.role != Role::Background)
            .map(|c| c.ink)
            .collect();
        assert!(lit.len() > 50, "{} lit cells", lit.len());
        assert!(lit.iter().all(|&i| i.role == role || i == HOT_INK));
        let thin = lit.iter().filter(|i| i.level < LEVELS[2]).count();
        assert!(thin * 10 >= lit.len() * 7, "{thin} of {} thin", lit.len());
        assert!(c.cells().iter().all(|c| c.bg == Ink::BLANK));
    }
}
