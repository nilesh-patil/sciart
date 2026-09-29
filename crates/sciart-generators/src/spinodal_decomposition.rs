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
//! Inks: `Role::Div`, the two phases at the two ends of the diverging map and the
//! interfaces through its dark midpoint, so each domain is outlined by its own boundary.

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{grid, ink, raster};

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
/// Seconds over which an act ramps up from half its step rate.
const EMERGE_S: f64 = 2.0;
/// Shading (see [`shade_field`]): the blur radius of the depth field (lattice cells, two
/// box passes), the interface ramp (|ψ|/ψ* from dark core to full colour), the Div value
/// of a domain's deep interior and how much brighter its rim glows, the sheen gain and
/// clamp for the fixed upper-left light, and the |v| below which a pixel stays empty.
const DEPTH_BLUR: usize = 2;
const EDGE: (f32, f32) = (0.2, 0.8);
/// Top of the interface ramp while the pattern is young (see [`shade_field`]).
const EDGE_YOUNG: f32 = 0.5;
const INTERIOR: f32 = 0.46;
const RIM: f32 = 0.34;
const SHEEN: (f32, f32, f32) = (0.9, -0.12, 0.18);
const EMPTY: f32 = 0.02;
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
    // The first seconds run slower, so the domains visibly emerge out of the dark quench
    // instead of appearing in a few frames.
    let emerge = 0.5 + 0.5 * (local / EMERGE_S).clamp(0.0, 1.0);
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

/// Smooth step on `[a, b]`.
#[inline]
fn smooth(x: f32, (a, b): (f32, f32)) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The signed display value (a Div value) of every lattice node, into `out`:
///
/// - the sign is the phase; the magnitude ramps up smoothly across each interface from a
///   dark core ([`EDGE`]), so boundaries stay crisp black lines;
/// - depth: `d = |blurred ψ/ψ*|` is ≈ 1 deep inside a domain and falls toward its edge,
///   so the interior sits at the deeper [`INTERIOR`] and the rim glows lighter
///   (`+ RIM·(1 − d)`), which reads as the meniscus of a liquid domain;
/// - sheen: `d` is treated as a height, lit from the upper left, so slopes facing the
///   light brighten and the far side darkens: blobs read as rounded.
///
/// A young quench (|ψ| ≪ ψ*) is dark everywhere and brightens as the domains form.
pub(crate) fn shade_field(
    psi: &Grid<f32>,
    out: &mut Grid<f32>,
    blur: &mut Grid<f32>,
    tmp: &mut Grid<f32>,
) {
    let (w, h) = (usize::from(psi.width()), usize::from(psi.height()));
    blur_wrap(psi, out, tmp, DEPTH_BLUR);
    blur_wrap(out, blur, tmp, DEPTH_BLUR);
    if out.size() != psi.size() {
        out.resize(psi.size(), 0.0);
    }
    if w == 0 || h == 0 {
        return;
    }
    let (p, b, o) = (psi.as_slice(), blur.as_slice(), out.as_mut_slice());
    let depth = |x: usize, y: usize| (b[y * w + x] / PSI_STAR).abs().min(1.0);
    // A young pattern (small amplitude everywhere) gets a soft ramp from zero, so it
    // emerges as smooth dim worms; as it forms, the ramp tightens to the crisp dark core.
    let amean = p.iter().map(|x| x.abs()).sum::<f32>() / (p.len() as f32 * PSI_STAR);
    let f = ((amean - 0.3) / 0.5).clamp(0.0, 1.0);
    let edge = (EDGE.0 * f, EDGE_YOUNG + (EDGE.1 - EDGE_YOUNG) * f);
    for y in 0..h {
        let (yn, ys) = ((y + h - 1) % h, (y + 1) % h);
        for x in 0..w {
            let (xw, xe) = ((x + w - 1) % w, (x + 1) % w);
            let s = (p[y * w + x] / PSI_STAR).clamp(-1.0, 1.0);
            let d = depth(x, y);
            // Height rising toward down-right faces the upper-left light.
            let slope = 0.5 * ((depth(xe, y) - depth(xw, y)) + (depth(x, ys) - depth(x, yn)));
            let sheen = (SHEEN.0 * slope).clamp(SHEEN.1, SHEEN.2);
            let m = smooth(s.abs(), edge) * (INTERIOR + RIM * (1.0 - d) + sheen);
            o[y * w + x] = s.signum() * m.clamp(0.0, 1.0);
        }
    }
}

/// The ink of a display value `v` ([`shade_field`]): Div; NaN reads as 0.
pub(crate) fn shade_ink(v: f32) -> Ink {
    ink::div(f64::from(if v.is_nan() { 0.0 } else { v }))
}

/// Whether the domains have formed: some |ψ| is past half of ψ*. Before that, pixels
/// near zero stay empty so the fresh quench is the terminal's own black; after it, the
/// interface cores are the Div map's dark zero, a continuous line rather than a dither
/// of empty and dim pixels.
pub(crate) fn formed(psi: &Grid<f32>) -> bool {
    psi.as_slice().iter().any(|x| x.abs() > 0.5 * PSI_STAR)
}

/// Entries of the display-value → ink table over `[−1, 1]`.
const LUT_N: usize = 512;

/// [`shade_ink`] tabulated.
fn ink_table() -> [Ink; LUT_N] {
    std::array::from_fn(|i| shade_ink(2.0 * i as f32 / (LUT_N - 1) as f32 - 1.0))
}

/// The nearest table entry for `v` (NaN reads as 0).
#[inline]
fn lut_index(v: f32) -> usize {
    let half = 0.5 * (LUT_N - 1) as f32;
    let u = (v + 1.0) * half;
    let u = if u.is_nan() { half } else { u };
    (u.round().max(0.0) as usize).min(LUT_N - 1)
}

/// ASCII: the stronger pixel, `#`/`+` for the positive phase, `:`/`.` for the negative,
/// blank on the interfaces.
pub(crate) fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let p = raster::stronger(top, bottom);
    if p.role != Role::Div {
        return None;
    }
    let v = ink::div_value(p);
    let ch = match v {
        v if v >= 0.55 => '#',
        v if v >= 0.3 => '+',
        v if v <= -0.55 => ':',
        v if v <= -0.3 => '.',
        _ => return None,
    };
    Some((ch, p))
}

#[derive(Debug)]
pub struct Spinodal {
    size: Size,
    psi: Grid<f32>,
    scratch: Scratch,
    regrid_scratch: Grid<f32>,
    pixels: Pixels,
    map: SampleMap,
    inks: [Ink; LUT_N],
    /// Display value per lattice node ([`shade_field`]) and its working grids.
    shade: Grid<f32>,
    blur: Grid<f32>,
    blur_tmp: Grid<f32>,
    timeline: Timeline,
    t: f64,
    /// `(cycle, act)` of the running quench.
    act: (i64, usize),
    pending: f64,
}

impl Default for Spinodal {
    fn default() -> Self {
        Self {
            size: Size::new(1, 1),
            psi: Grid::default(),
            scratch: Scratch::default(),
            regrid_scratch: Grid::default(),
            pixels: Pixels::new(Size::new(1, 1)),
            map: SampleMap::default(),
            inks: ink_table(),
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
        shade_field(
            &self.psi,
            &mut self.shade,
            &mut self.blur,
            &mut self.blur_tmp,
        );
        let (map, shade, inks) = (&self.map, &self.shade, &self.inks);
        let formed = formed(&self.psi);
        self.pixels.shade(|x, y| {
            let v = map.sample(shade, x, y);
            if !formed && (v.is_nan() || v.abs() < EMPTY) {
                return Ink::BLANK;
            }
            inks[lut_index(v)]
        });
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
        self.rasterise();
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
    use crate::shared::cahn_hilliard::{correlation_length, count_below, mean};

    fn scene(w: u16, h: u16, seed: u64) -> (Spinodal, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = Spinodal::default();
        g.resize(size, &mut ctx);
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
        }
        let m0 = mean(&g.psi);
        for size in [(80, 24), (13, 40), (1, 1), (200, 50)] {
            g.resize(Size::new(size.0, size.1), &mut ctx);
            assert!((mean(&g.psi) - m0).abs() < 1e-6, "{size:?}");
        }
    }

    #[test]
    fn captions_follow_the_acts_and_each_act_starts_from_a_fresh_quench() {
        let (mut g, mut ctx) = scene(80, 24, 42);
        assert_eq!(g.caption(), Some("spinodal"));
        for _ in 0..(ACT_S * 60.0) as usize + 2 {
            g.update(SIM_DT, &mut ctx);
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
}
