//! Spinodal decomposition: a quenched binary mixture unmixes into two phases whose
//! domains coarsen (SPEC-002 `spinodal_decomposition`).
//!
//! Model: the shared [`cahn_hilliard`](crate::shared::cahn_hilliard) Oono–Puri cell-dynamics map
//! on a capped periodic lattice (ADR-0012). Each act is a fresh quench, `ψ = ψ̄ + noise`:
//!
//! - **spinodal** (`ψ̄ = 0`, the critical mix): the uniform state is unstable at every
//!   point, so a fine labyrinth of interpenetrating worm-like domains appears everywhere
//!   at once and coarsens, its length growing as `t^{1/3}`;
//! - **droplets ripen** (`ψ̄ = 0.35`, off-critical): the minority phase breaks into
//!   droplets, and the big ones grow at the expense of the small ones (Ostwald ripening).
//!
//! The coarsening law slows the picture down as it goes, so the map's step rate rises
//! through each act ([`steps_per_s`]), keeping the domains visibly moving without any
//! frame doing more than [`MAX_STEPS`] steps.
//!
//! Inks (ADR-0018, M11's idiom): lines only, on black, in a complementary pair. Both
//! phases are the terminal's black, and each interface is a thin flat line hugging the
//! zero level set (`|ψ| / |∇ψ|` below [`LINE_W`] pixels): `Hot` on its `ψ > 0` side and
//! `Cold` on its `ψ < 0` side, so the labyrinth and the droplets read as fine contours
//! that still tell the two phases apart. (A flat fill for one phase read as large
//! plateaus.)

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{grid, raster};

use crate::shared::cahn_hilliard::{self, OONO_PURI, Scratch};

pub static META: Meta = Meta {
    name: "spinodal_decomposition",
    category: Category::PhysicalChemistry,
    description: "Cahn-Hilliard phase separation: a quenched mixture unmixes and its domains coarsen",
    title: "Unmixing · domains coarsen",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 2] = ["spinodal", "droplets ripen"];
/// Seconds per act, and each act's mean composition ψ̄.
const ACT_S: f64 = 30.0;
const ACT_MEAN: [f32; 2] = [0.0, 0.35];
/// Half-width of the quench noise before it is smoothed ([`QUENCH_BLUR`]).
const NOISE: f32 = 0.1;
/// The bulk value ψ* of the phases at A = 1.3 (`1.3 tanh ψ* = ψ*`).
const PSI_STAR: f32 = 0.98;
/// Lattice node budget and aspect bucket (ADR-0012).
const MAX_NODES: usize = 16_000;
const BUCKET: f64 = 1.25;
/// Step rate at the start and end of an act (steps per simulated second), and the cap
/// per `update`.
const RATE: (f64, f64) = (120.0, 360.0);
const MAX_STEPS: usize = 6;
/// Seconds over which an act eases down from twice its opening step rate.
const EMERGE_S: f64 = 2.0;
/// Inks: the interface line's half-width in pixels, and its two steps (the young mush,
/// the formed pattern) for either phase's side.
const LINE_W: f32 = 1.3;
const PLUS_INKS: [Ink; 2] = [Ink::new(Role::Hot, 110), Ink::new(Role::Hot, 220)];
const MINUS_INKS: [Ink; 2] = [Ink::new(Role::Cold, 110), Ink::new(Role::Cold, 220)];
/// Formedness below which the interfaces are not drawn, and from which they are full.
const FORMED: (f32, f32) = (0.02, 0.6);
/// Box radius of the one smoothing pass over the quench noise, so the mixture unmixes
/// from a smooth, dark field rather than from white-noise speckle.
const QUENCH_BLUR: usize = 1;

pub fn build() -> Box<dyn Generator> {
    Box::new(Spinodal::default())
}

/// The simulation lattice for a display: the half-block pixel grid, capped (ADR-0012).
pub(crate) fn lattice_size(display: Size) -> Size {
    let px = Size::new(display.width(), display.height().saturating_mul(2));
    grid::lattice_for(px, MAX_NODES, BUCKET)
}

/// Map steps per simulated second `local` seconds into an act: rising linearly, since
/// `L ∝ t^{1/3}` would otherwise freeze the picture late in the act.
pub(crate) fn steps_per_s(local: f64) -> f64 {
    let f = (local / ACT_S).clamp(0.0, 1.0);
    // The first seconds run faster (twice the act's opening rate, easing back), so the
    // quench's dim mush gives way to formed worms within the first second.
    let emerge = 2.0 - (local / EMERGE_S).clamp(0.0, 1.0);
    (RATE.0 + (RATE.1 - RATE.0) * f) * emerge
}

/// Periodic box blur of radius `r` along x then y (`tmp` is scratch); both outputs are
/// reshaped to `src`'s size, reusing their buffers.
pub(crate) fn blur_wrap(src: &Grid<f32>, dst: &mut Grid<f32>, tmp: &mut Grid<f32>, r: usize) {
    let (w, h) = (usize::from(src.width()), usize::from(src.height()));
    if dst.size() != src.size() {
        dst.resize(src.size(), 0.0);
    }
    if tmp.size() != src.size() {
        tmp.resize(src.size(), 0.0);
    }
    if w == 0 || h == 0 {
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    let ri = r as isize;
    // Whole shifted rows are summed, so the inner loops are plain slice adds with no
    // per-element wrap arithmetic.
    let (s, t) = (src.as_slice(), tmp.as_mut_slice());
    for (row, out) in s.chunks_exact(w).zip(t.chunks_exact_mut(w)) {
        out.fill(0.0);
        for k in -ri..=ri {
            let shift = k.rem_euclid(w as isize) as usize;
            let (head, tail) = out.split_at_mut(w - shift);
            head.iter_mut()
                .zip(&row[shift..])
                .for_each(|(o, v)| *o += v);
            tail.iter_mut()
                .zip(&row[..shift])
                .for_each(|(o, v)| *o += v);
        }
    }
    let (t, d) = (tmp.as_slice(), dst.as_mut_slice());
    for (y, out) in d.chunks_exact_mut(w).enumerate() {
        out.fill(0.0);
        for k in -ri..=ri {
            let sy = (y as isize + k).rem_euclid(h as isize) as usize;
            out.iter_mut()
                .zip(&t[sy * w..][..w])
                .for_each(|(o, v)| *o += v);
        }
        out.iter_mut().for_each(|o| *o *= norm * norm);
    }
}

/// `|∇ψ|` per node (central differences, periodic) into `out`, and the formedness of the
/// pattern: the mean `|ψ|/ψ*` mapped from 0.3 (a young mush) to 0.8 (formed domains) onto
/// `[0, 1]`.
pub(crate) fn slope_field(psi: &Grid<f32>, out: &mut Grid<f32>) -> f32 {
    let (w, h) = (usize::from(psi.width()), usize::from(psi.height()));
    if out.size() != psi.size() {
        out.resize(psi.size(), 0.0);
    }
    if w == 0 || h == 0 {
        return 0.0;
    }
    let (p, o) = (psi.as_slice(), out.as_mut_slice());
    for y in 0..h {
        let (yn, ys) = ((y + h - 1) % h, (y + 1) % h);
        for x in 0..w {
            let (xw, xe) = ((x + w - 1) % w, (x + 1) % w);
            let gx = 0.5 * (p[y * w + xe] - p[y * w + xw]);
            let gy = 0.5 * (p[ys * w + x] - p[yn * w + x]);
            o[y * w + x] = (gx * gx + gy * gy).sqrt();
        }
    }
    let amean = p.iter().map(|x| x.abs()).sum::<f32>() / (p.len() as f32 * PSI_STAR);
    ((amean - 0.3) / 0.5).clamp(0.0, 1.0)
}

/// The ink of a pixel with order parameter `psi`, local slope `g` (per node), `px` pixels
/// per node and formedness `f`: within [`LINE_W`] pixels of `ψ = 0`, the interface line
/// in its side's colour, a step dimmer while the pattern is still forming; else black.
pub(crate) fn pixel_ink(psi: f32, g: f32, px: f32, f: f32) -> Ink {
    if !(psi.is_finite() && g.is_finite()) || f < FORMED.0 {
        return Ink::BLANK;
    }
    if psi.abs() / g.max(1e-4) * px >= LINE_W {
        return Ink::BLANK;
    }
    let step = usize::from(f >= FORMED.1);
    if psi > 0.0 {
        PLUS_INKS[step]
    } else {
        MINUS_INKS[step]
    }
}

/// ASCII: the stronger pixel: the interface `#` (formed) or `+`, the phases blank.
pub(crate) fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let p = raster::stronger(top, bottom);
    if p.role == Role::Background {
        return None;
    }
    Some((
        if p.level >= PLUS_INKS[1].level {
            '#'
        } else {
            '+'
        },
        p,
    ))
}

#[derive(Debug)]
pub struct Spinodal {
    size: Size,
    psi: Grid<f32>,
    scratch: Scratch,
    regrid_scratch: Grid<f32>,
    pixels: Pixels,
    map: SampleMap,
    /// Display value per lattice node ([`shade_field`]) and its working grids.
    shade: Grid<f32>,
    blur: Grid<f32>,
    blur_tmp: Grid<f32>,
    timeline: Timeline,
    t: f64,
    /// `(cycle, act)` of the running quench.
    act: (i64, usize),
    pending: f64,
    /// The pixels are stale: the state moved since the last `rasterise` (GEN-R20).
    dirty: bool,
}

impl Default for Spinodal {
    fn default() -> Self {
        Self {
            dirty: true,
            size: Size::new(1, 1),
            psi: Grid::default(),
            scratch: Scratch::default(),
            regrid_scratch: Grid::default(),
            pixels: Pixels::new(Size::new(1, 1)),
            map: SampleMap::default(),
            shade: Grid::default(),
            blur: Grid::default(),
            blur_tmp: Grid::default(),
            timeline: Timeline::new(&[(CAPTIONS[0], ACT_S), (CAPTIONS[1], ACT_S)]),
            t: 0.0,
            act: (0, 0),
            pending: 0.0,
        }
    }
}

impl Spinodal {
    fn quench(&mut self, act: usize, rng: &mut impl Rng) {
        let mean = ACT_MEAN[act.min(ACT_MEAN.len() - 1)];
        cahn_hilliard::quench(&mut self.psi, mean, NOISE, rng);
        // Smooth the noise (a finite correlation length), keeping the mean exactly.
        blur_wrap(&self.psi, &mut self.blur, &mut self.blur_tmp, QUENCH_BLUR);
        std::mem::swap(&mut self.psi, &mut self.blur);
        cahn_hilliard::shift_mean(&mut self.psi, f64::from(mean));
        self.pending = 0.0;
    }

    fn rasterise(&mut self) {
        // `shade` holds |∇ψ| per node (it is only scratch here).
        let f = slope_field(&self.psi, &mut self.shade);
        let (map, psi, slope) = (&self.map, &self.psi, &self.shade);
        let px = map.magnification().max(1e-3);
        // ψ bilinear up to 2× magnification, Catmull-Rom above (CORE-R102), so the line
        // does not crease on large terminals; the slope is smooth, so bilinear. Rows rebuilt per pixel row (CORE-R104).
        if px > 2.0 {
            let (mut ry, mut pr, mut gr) = (-1, map.row_cubic(psi, 0), map.row(slope, 0));
            self.pixels.shade(|x, y| {
                if y != ry {
                    (ry, pr, gr) = (y, map.row_cubic(psi, y), map.row(slope, y));
                }
                pixel_ink(pr.at(x), gr.at(x), px, f)
            });
        } else {
            let (mut ry, mut pr, mut gr) = (-1, map.row(psi, 0), map.row(slope, 0));
            self.pixels.shade(|x, y| {
                if y != ry {
                    (ry, pr, gr) = (y, map.row(psi, y), map.row(slope, y));
                }
                pixel_ink(pr.at(x), gr.at(x), px, f)
            });
        }
    }
}

impl Generator for Spinodal {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.timeline.at(self.t).label)
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let lattice = lattice_size(size);
        if self.psi.is_empty() {
            self.psi = Grid::new(lattice, 0.0);
            self.quench(self.act.1, &mut ctx.rng);
        } else {
            cahn_hilliard::regrid(&mut self.psi, lattice, &mut self.regrid_scratch);
        }
        self.size = size;
        self.pixels.resize(size);
        self.map.rebuild(lattice, self.pixels.pixel_size());
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !(dt.is_finite() && dt > 0.0) || self.psi.is_empty() {
            return;
        }
        self.t += dt;
        let beat = self.timeline.at(self.t);
        let act = (self.timeline.cycle(self.t), beat.index);
        if act != self.act {
            self.act = act;
            self.quench(act.1, &mut ctx.rng);
        }
        self.pending += dt * steps_per_s(beat.local);
        let mut n = 0;
        while self.pending >= 1.0 && n < MAX_STEPS {
            cahn_hilliard::step(&mut self.psi, &mut self.scratch, &OONO_PURI);
            self.pending -= 1.0;
            n += 1;
        }
        self.pending = self.pending.min(1.0);
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
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::cahn_hilliard::{correlation_length, count_below, mean};

    fn scene(w: u16, h: u16, seed: u64) -> (Spinodal, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = Spinodal::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    fn quenched(w: u16, h: u16, m: f32, seed: u64) -> Grid<f32> {
        let mut ctx = Ctx::new(seed, Size::new(1, 1));
        let mut psi = Grid::new(Size::new(w, h), 0.0);
        cahn_hilliard::quench(&mut psi, m, NOISE, &mut ctx.rng);
        psi
    }

    #[test]
    fn spin_a1_mean_is_conserved_every_step() {
        for m in ACT_MEAN {
            let mut psi = quenched(96, 64, m, 11);
            let mut s = Scratch::default();
            let mut prev = mean(&psi);
            assert!((prev - f64::from(m)).abs() < 1e-7);
            for _ in 0..300 {
                cahn_hilliard::step(&mut psi, &mut s, &OONO_PURI);
                let now = mean(&psi);
                // f32 rounding: 1e-8 a step, and no systematic drift.
                assert!((now - prev).abs() < 1e-8, "{prev} → {now}");
                assert!((now - f64::from(m)).abs() < 1e-6, "{now}");
                prev = now;
            }
        }
    }

    #[test]
    fn spin_a2_domain_length_grows_as_a_power_near_one_third() {
        let mut psi = quenched(160, 160, 0.0, 7);
        let mut s = Scratch::default();
        let mut pts = Vec::new();
        let mut t = 0usize;
        for target in [200usize, 400, 800, 1600, 3200] {
            while t < target {
                cahn_hilliard::step(&mut psi, &mut s, &OONO_PURI);
                t += 1;
            }
            pts.push(((t as f64).ln(), correlation_length(&psi).ln()));
        }
        let n = pts.len() as f64;
        let (mx, my) = (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let slope = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>()
            / pts.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>();
        assert!((0.2..=0.45).contains(&slope), "slope {slope}, {pts:?}");
    }

    #[test]
    fn spin_a3_off_critical_mix_forms_droplets_that_ripen() {
        let mut psi = quenched(160, 100, ACT_MEAN[1], 3);
        let mut s = Scratch::default();
        let mut counts = Vec::new();
        for _ in 0..6 {
            for _ in 0..600 {
                cahn_hilliard::step(&mut psi, &mut s, &OONO_PURI);
            }
            counts.push(count_below(&psi, 0.0, 4));
        }
        let peak = *counts.iter().max().unwrap_or(&0);
        assert!(peak >= 10, "{counts:?}");
        assert!(*counts.last().unwrap_or(&0) < peak, "{counts:?}");
    }

    #[test]
    fn spin_a4_psi_stays_bounded_through_both_acts_at_any_dt() {
        let (mut g, mut ctx) = scene(120, 40, 42);
        for i in 0..(2.2 * ACT_S * 60.0) as usize {
            let dt = [SIM_DT, 0.25, 0.0, 0.1][i % 4];
            g.update(dt, &mut ctx);
            g.prepare();
            if i % 50 == 0 {
                let bad = g.psi.as_slice().iter().find(|x| !(-1.2..=1.2).contains(*x));
                assert!(bad.is_none(), "ψ = {bad:?} at frame {i}");
            }
        }
    }

    #[test]
    fn spin_a5_regridding_keeps_the_mean() {
        let (mut g, mut ctx) = scene(200, 50, 1);
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let m0 = mean(&g.psi);
        for size in [(80, 24), (13, 40), (1, 1), (200, 50)] {
            g.resize(Size::new(size.0, size.1), &mut ctx);
            g.prepare();
            assert!((mean(&g.psi) - m0).abs() < 1e-6, "{size:?}");
        }
    }

    #[test]
    fn captions_follow_the_acts_and_each_act_starts_from_a_fresh_quench() {
        let (mut g, mut ctx) = scene(80, 24, 42);
        assert_eq!(g.caption(), Some("spinodal"));
        for _ in 0..(ACT_S * 60.0) as usize + 2 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(g.caption(), Some("droplets ripen"));
        assert!((mean(&g.psi) - f64::from(ACT_MEAN[1])).abs() < 1e-5);
    }

    #[test]
    fn lattice_is_capped_and_the_step_rate_rises_through_an_act() {
        let l = lattice_size(Size::new(500, 150));
        assert!(usize::from(l.width()) * usize::from(l.height()) <= MAX_NODES);
        assert_eq!(lattice_size(Size::new(40, 12)), Size::new(40, 24));
        assert!(steps_per_s(0.0) < steps_per_s(ACT_S));
        assert!(steps_per_s(ACT_S) / 60.0 <= MAX_STEPS as f64);
    }

    #[test]
    fn spin_a6_interfaces_are_thin_two_colour_lines_and_the_mush_forms_fast() {
        // ADR-0018: both phases black and the interface a thin flat line, `Hot` on the
        // ψ > 0 side and `Cold` on the other, at any angle.
        let (w, h) = (48u16, 48u16);
        let mut psi = Grid::new(Size::new(w, h), 0.0f32);
        let (nx, ny) = (0.5f32, 0.866f32);
        for y in 0..h {
            for x in 0..w {
                let d = (f32::from(x) - 24.0) * nx + (f32::from(y) - 24.0) * ny;
                psi.set(i32::from(x), i32::from(y), PSI_STAR * (d / 1.2).tanh());
            }
        }
        let mut slope = Grid::default();
        let f = slope_field(&psi, &mut slope);
        let at = |x: u16, y: u16| {
            let i = usize::from(y) * usize::from(w) + usize::from(x);
            pixel_ink(psi.as_slice()[i], slope.as_slice()[i], 1.0, f)
        };
        let (mut plus, mut minus) = (0, 0);
        for y in 16..32u16 {
            for x in 16..32u16 {
                let d = (f32::from(x) - 24.0) * nx + (f32::from(y) - 24.0) * ny;
                let ink = at(x, y);
                if d.abs() < 0.3 {
                    assert_ne!(ink, Ink::BLANK, "a gap on the line at ({x}, {y})");
                }
                if d.abs() > 1.5 {
                    assert_eq!(ink, Ink::BLANK, "a thick line at ({x}, {y})");
                }
                match ink {
                    i if i == PLUS_INKS[1] => plus += usize::from(d > 0.0),
                    i if i == MINUS_INKS[1] => minus += usize::from(d < 0.0),
                    i => assert_eq!(i, Ink::BLANK),
                }
            }
        }
        assert!(plus > 5 && minus > 5, "both sides drawn: {plus} {minus}");
        // Deep in either phase: black.
        assert_eq!(pixel_ink(PSI_STAR, 0.01, 1.0, 1.0), Ink::BLANK);
        assert_eq!(pixel_ink(-PSI_STAR, 0.01, 1.0, 1.0), Ink::BLANK);
        // The quench starts fast, so the t = 1 s frame is formed.
        let (mut g, mut ctx) = scene(120, 40, 42);
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert!(g.psi.as_slice().iter().any(|x| x.abs() > 0.5 * PSI_STAR));
    }
}
