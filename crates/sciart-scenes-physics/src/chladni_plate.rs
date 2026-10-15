//! Sand on a vibrating square plate gathers on the nodal lines: Chladni figures
//! (SPEC-002 `chladni_plate`).
//!
//! ## Model
//! Chladni's approximation for a free square plate: on `x, y ∈ [0, 1]` a mode `(m, n, ±)`
//! has the shape `u = ½ [cos(nπx) cos(mπy) ± cos(mπx) cos(nπy)]`, scaled into `[−1, 1]`.
//! Sand grains bounce off the moving plate: each step a grain takes a kick of length
//! `KICK · |u|` in a random direction, so grains leave the lobes and pile up where the
//! plate is still. A kick that would land a grain on livelier plate (larger `|u|`) is
//! rejected half the time, a net drift toward the nodes that settles the sand in a few
//! seconds. A kick shorter than [`STICK`] dots does not unstick a grain (static
//! friction), so settled sand holds still. Edges reflect. Kicks follow simulated time
//! (one per 1/60 s, fractions carried over). Each mode plays for [`MODE_S`]
//! on a looping [`Timeline`]; when the tone changes, the grains on the old nodes sit on
//! the new mode's moving lobes and scatter to the new figure.
//!
//! ## Look (GEN-R26)
//! The plate is a centred square of square braille dots. Sand grains are single bright
//! `Sun` dots; the mode shape is a barely-there fixed-threshold stipple of density
//! `≤ 0.10·|u|`, `Hot` where `u > 0` and `Cold` where `u < 0` (static per mode, so it
//! holds still); the plate edge is a 1-px `Dim` thread. The mode is read out on the
//! title row when it fits.
//!
//! Stylisation: Chladni's product approximation, not the true free-plate eigenmodes; the
//! sand is a random-kick caricature of bouncing grains.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{draw, title};

pub static META: Meta = Meta {
    name: "chladni_plate",
    category: Category::Physics,
    description: "Sand on a vibrating plate gathers on the nodal lines, drawing Chladni figures",
    title: "Chladni plate · sand finds nodes",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ChladniPlate::default())
}

/// One plate mode `(m, n, ±)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mode {
    pub m: u8,
    pub n: u8,
    pub plus: bool,
}

const fn mode(m: u8, n: u8, plus: bool) -> Mode {
    Mode { m, n, plus }
}

/// The modes, played in turn: classic figures alternating `+` and `−` forms, ordered so
/// each one's nodal lines lie on the next one's lobes (mean `|u| ≥ 0.35` there, CHL-A2).
pub(crate) const MODES: [Mode; 8] = [
    mode(1, 5, true),
    mode(4, 6, false),
    mode(2, 4, true),
    mode(2, 8, false),
    mode(3, 5, true),
    mode(4, 8, false),
    mode(2, 6, true),
    mode(3, 8, false),
];
/// Largest wavenumber in [`MODES`] (the cosine tables hold `0..=K_MAX`).
const K_MAX: usize = 8;
/// Seconds per mode.
pub(crate) const MODE_S: f64 = 15.0;

/// Kick length in dots at `|u| = 1`, and the shortest kick that moves a grain.
const KICK: f64 = 5.0;
const STICK: f64 = 0.4;
/// Plate area (dots²) per sand grain, and the grain cap.
const DOTS_PER_GRAIN: f64 = 12.0;
const MAX_GRAINS: usize = 3000;
/// Stipple density at `|u| = 1` (CHL-A4: ≤ 0.10) and the stipple's level.
pub(crate) const STIPPLE_PEAK: f64 = 0.08;
const STIPPLE_LEVEL: u8 = 150;
const STIPPLE_SALT: u64 = 0xC4_1AD1;
/// Plate side as a share of the canvas width at most (the height is the usual limit).
const FILL: f64 = 0.9;
const DIRS: usize = 64;

const SAND: Ink = Ink::new(Role::Sun, 255);
const EDGE: Ink = Ink::new(Role::Dim, 150);
const LABEL: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);

/// The mode shape at plate point `(x, y) ∈ [0, 1]²`, in `[−1, 1]` (CHL-A1).
/// (The scene evaluates it from per-resize cosine tables in `u_at`; this is the reference.)
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn mode_u(md: Mode, x: f64, y: f64) -> f64 {
    let (m, n) = (f64::from(md.m) * PI, f64::from(md.n) * PI);
    let a = (n * x).cos_det() * (m * y).cos_det();
    let b = (m * x).cos_det() * (n * y).cos_det();
    0.5 * if md.plus { a + b } else { a - b }
}

/// The plate stipple at shape value `u`: keep density and ink, `None` at a node (CHL-A4).
pub(crate) fn plate_ink(u: f64) -> Option<(f64, Ink)> {
    let p = STIPPLE_PEAK * u.abs().min(1.0);
    if p.is_nan() || p <= 0.0 {
        return None;
    }
    let role = if u > 0.0 { Role::Hot } else { Role::Cold };
    Some((p, Ink::new(role, STIPPLE_LEVEL)))
}

/// Reflect `v` into `[0, 1]`.
fn reflect(v: f64) -> f64 {
    let v = v.rem_euclid(2.0);
    if v > 1.0 { 2.0 - v } else { v }
}

#[derive(Debug)]
pub struct ChladniPlate {
    size: Size,
    time: f64,
    timeline: Timeline,
    /// Index into [`MODES`] now playing.
    mode: usize,
    /// Plate origin (top-left dot) and side in dots.
    ox: f64,
    oy: f64,
    side: f64,
    /// `cos(kπ·i/(T−1))` for `k ∈ 0..=K_MAX`, `i ∈ 0..T`, row-major by `k`.
    cos: Vec<f32>,
    taps: usize,
    /// Grains in plate units.
    gx: Vec<f64>,
    gy: Vec<f64>,
    dirs: [(f64, f64); DIRS],
    rng: SplitMix64,
    /// The plate stipple for the current mode: dot `(x, y)` and ink.
    stipple: Vec<(i32, i32, Ink)>,
    stipple_stale: bool,
    /// Plate dots `(i, j)` whose fixed stipple threshold is below [`STIPPLE_PEAK`], with
    /// that threshold: the only dots any mode can stipple (built in `resize`).
    candidates: Vec<(u16, u16, f32)>,
    /// Kicks owed for fractions of a 1/60 s step.
    kick_debt: f64,
    /// Kicks so far (tests).
    kicks: u64,
    readout: StackText<20>,
    readout_ascii: StackText<20>,
    room: usize,
    dots: Dots,
    dirty: bool,
}

impl Default for ChladniPlate {
    fn default() -> Self {
        let phases: Vec<(&'static str, f64)> = MODES.iter().map(|_| ("", MODE_S)).collect();
        Self {
            size: Size::new(1, 1),
            time: 0.0,
            timeline: Timeline::new(&phases),
            mode: 0,
            ox: 0.0,
            oy: 0.0,
            side: 1.0,
            cos: Vec::new(),
            taps: 0,
            gx: Vec::new(),
            gy: Vec::new(),
            dirs: [(1.0, 0.0); DIRS],
            rng: SplitMix64::new(0),
            stipple: Vec::new(),
            stipple_stale: true,
            candidates: Vec::new(),
            kick_debt: 0.0,
            kicks: 0,
            readout: StackText::new(),
            readout_ascii: StackText::new(),
            room: 0,
            dots: Dots::default(),
            dirty: true,
        }
    }
}

impl ChladniPlate {
    /// The current mode's shape at plate point `(x, y)` from the cosine tables.
    #[inline]
    fn u_at(&self, x: f64, y: f64) -> f64 {
        let md = MODES[self.mode];
        let t = self.taps;
        let last = (t - 1) as f64;
        let i = ((x * last).round() as usize).min(t - 1);
        let j = ((y * last).round() as usize).min(t - 1);
        let c = |k: u8, i: usize| f64::from(self.cos[usize::from(k) * t + i]);
        let a = c(md.n, i) * c(md.m, j);
        let b = c(md.m, i) * c(md.n, j);
        0.5 * if md.plus { a + b } else { a - b }
    }

    /// Mean `|u|` of the current mode over the grains (CHL-A2).
    #[cfg(test)]
    pub(crate) fn mean_u_at_grains(&self) -> f64 {
        let n = self.gx.len().max(1);
        let s: f64 = self
            .gx
            .iter()
            .zip(&self.gy)
            .map(|(&x, &y)| mode_u(MODES[self.mode], x, y).abs())
            .sum();
        s / n as f64
    }

    fn set_mode(&mut self, i: usize) {
        self.mode = i;
        self.stipple_stale = true;
        let md = MODES[i];
        // The mode with its ± form; `−` falls back to `-` in ASCII (GEN-R10).
        let (sign, ascii) = if md.plus { ('+', '+') } else { ('−', '-') };
        self.readout.clear();
        let _ = write!(self.readout, "m {}  n {}  {sign}", md.m, md.n);
        self.readout_ascii.clear();
        let _ = write!(self.readout_ascii, "m {}  n {}  {ascii}", md.m, md.n);
    }

    /// One step of bouncing sand.
    fn kick(&mut self) {
        let k = KICK / self.side;
        let stick = STICK / KICK;
        for g in 0..self.gx.len() {
            let (x, y) = (self.gx[g], self.gy[g]);
            let u = self.u_at(x, y).abs();
            if u < stick {
                continue;
            }
            let r = self.rng.next_u64();
            let (dx, dy) = self.dirs[(r >> 58) as usize];
            let (nx, ny) = (reflect(x + dx * k * u), reflect(y + dy * k * u));
            // A bounce that lands on livelier plate is half as likely to stick there
            // as one that lands on stiller plate: the net drift toward the nodes.
            if r & 1 == 0 && self.u_at(nx, ny).abs() > u {
                continue;
            }
            self.gx[g] = nx;
            self.gy[g] = ny;
        }
    }

    /// The plate dots whose [`stipple_keep`](sciart_core::raster::stipple_keep) threshold lies below [`STIPPLE_PEAK`] (≈ 8 %
    /// of the plate): a mode change then evaluates only these.
    fn build_candidates(&mut self) {
        self.candidates.clear();
        let n = self.side as i32;
        let (ox, oy) = (self.ox as i32, self.oy as i32);
        for j in 0..n {
            for i in 0..n {
                let thr = hash01(STIPPLE_SALT ^ cell_key(ox + i, oy + j));
                if thr < STIPPLE_PEAK {
                    self.candidates.push((i as u16, j as u16, thr as f32));
                }
            }
        }
        // TEST-R7 (M22): built by `push` once per resize; drop the doubling slack.
        self.candidates.shrink_to_fit();
        self.stipple = Vec::with_capacity(self.candidates.len());
    }

    /// The current mode's stipple: exactly the dots [`stipple_keep`](sciart_core::raster::stipple_keep) keeps at
    /// [`plate_ink`]'s density (CHL-A4).
    fn rebuild_stipple(&mut self) {
        self.stipple.clear();
        let last = (self.taps - 1) as f64;
        let (ox, oy) = (self.ox as i32, self.oy as i32);
        for &(i, j, thr) in &self.candidates {
            let u = self.u_at(f64::from(i) / last, f64::from(j) / last);
            let Some((p, ink)) = plate_ink(u) else {
                continue;
            };
            if f64::from(thr) < p {
                self.stipple
                    .push((ox + i32::from(i), oy + i32::from(j), ink));
            }
        }
    }

    fn rasterise(&mut self) {
        if std::mem::take(&mut self.stipple_stale) {
            self.rebuild_stipple();
        }
        let dots = &mut self.dots;
        dots.clear();
        for &(x, y, ink) in &self.stipple {
            dots.set(x, y, ink);
        }
        // The plate edge, a 1-px thread half a dot outside the plate.
        let (x0, y0) = (self.ox - 1.0, self.oy - 1.0);
        let (x1, y1) = (self.ox + self.side, self.oy + self.side);
        dots.line(x0, y0, x1, y0, EDGE);
        dots.line(x1, y0, x1, y1, EDGE);
        dots.line(x1, y1, x0, y1, EDGE);
        dots.line(x0, y1, x0, y0, EDGE);
        let span = self.side - 1.0;
        for (&x, &y) in self.gx.iter().zip(&self.gy) {
            dots.plot(self.ox + x * span, self.oy + y * span, SAND);
        }
    }
}

impl Generator for ChladniPlate {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let ds = self.dots.dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        // Square in dots, its edge thread on rows 1 ..= h − 3: a blank row above it and
        // one between it and the title row.
        let cell_h = f64::from(Dots::CELL_H);
        let (band_top, band_bottom) = (cell_h, h - 2.0 * cell_h - 1.0);
        let band = band_bottom - band_top + 1.0;
        // `side` plate dots plus the two edge dots must fit the band.
        let room = (w * FILL).min(band - 2.0);
        self.side = room.floor().max(2.0);
        self.ox = ((w - self.side) / 2.0).floor();
        self.oy = (band_top + 1.0 + ((band - self.side - 2.0) / 2.0).floor()).max(1.0);
        self.taps = self.side as usize;
        let t = self.taps;
        self.cos.clear();
        for k in 0..=K_MAX {
            for i in 0..t {
                let x = i as f64 / (t - 1) as f64;
                self.cos.push((k as f64 * PI * x).cos_det() as f32);
            }
        }
        for (k, d) in self.dirs.iter_mut().enumerate() {
            let (s, c) = (k as f64 * TAU / DIRS as f64).sin_cos_det();
            *d = (c, s);
        }
        // Grains survive resizes in plate units; their count follows the plate's area.
        let want = ((self.side * self.side / DOTS_PER_GRAIN) as usize).clamp(16, MAX_GRAINS);
        if self.gx.is_empty() {
            self.rng = SplitMix64::from_rng(&mut ctx.rng);
        }
        self.gx.truncate(want);
        self.gy.truncate(want);
        while self.gx.len() < want {
            self.gx.push(ctx.rng.random_range(0.0..=1.0));
            self.gy.push(ctx.rng.random_range(0.0..=1.0));
        }
        self.build_candidates();
        let beat = self.timeline.at(self.time).index;
        self.set_mode(beat);
        self.room = title::readout_room(size, &title::Title::of(self));
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 || self.taps < 2 {
            return;
        }
        self.time += dt;
        let beat = self.timeline.at(self.time).index;
        if beat != self.mode {
            self.set_mode(beat);
        }
        // One kick per 1/60 s of simulated time, so the dance is step-size independent.
        // One kick per 1/60 s of simulated time, fractions carried over, so the dance
        // does not depend on the step size.
        self.kick_debt += dt / SIM_DT;
        while self.kick_debt >= 1.0 - 1e-9 {
            self.kick_debt -= 1.0;
            self.kick();
            self.kicks += 1;
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
        let text = match canvas.mode() {
            GlyphMode::Unicode => self.readout.as_str(),
            GlyphMode::Ascii => self.readout_ascii.as_str(),
        };
        if text.chars().count() <= self.room {
            draw::label_right(canvas, i32::from(self.size.height()) - 1, text, LABEL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::raster::stipple_keep;

    fn plate(seed: u64, size: Size) -> (ChladniPlate, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = ChladniPlate::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn chl_a1_mode_vanishes_on_nodal_set_with_its_symmetry() {
        for md in MODES {
            for k in 0..=50 {
                let x = f64::from(k) / 50.0;
                for l in 0..=50 {
                    let y = f64::from(l) / 50.0;
                    let (a, b) = (mode_u(md, x, y), mode_u(md, y, x));
                    // ± form: symmetric / antisymmetric under x ↔ y.
                    let want = if md.plus { a } else { -a };
                    assert!((b - want).abs() < 1e-12, "{md:?}");
                    assert!(a.abs() <= 1.0 + 1e-12);
                }
                if md.plus {
                    // With m + n odd the + form vanishes on the anti-diagonal.
                    if (md.m + md.n) % 2 == 1 {
                        assert!(mode_u(md, x, 1.0 - x).abs() < 1e-12, "{md:?}");
                    }
                } else {
                    assert!(mode_u(md, x, x).abs() < 1e-12, "{md:?} on the diagonal");
                }
            }
            // And it is a real vibration, not zero everywhere.
            let peak = (0..=50)
                .map(|k| mode_u(md, f64::from(k) / 50.0, 0.0).abs())
                .fold(0.0, f64::max);
            assert!(peak > 0.5, "{md:?}");
        }
    }

    #[test]
    fn chl_a2_sand_on_nodes_10s_after_each_change_and_tossed_at_it() {
        for seed in [1, 2, 3] {
            let (mut g, mut ctx) = plate(seed, Size::new(200, 50));
            for k in 0..3 {
                // Just after the change to mode k (from the sand of mode k − 1).
                let start = k as f64 * MODE_S;
                while g.time < start + 0.5 * SIM_DT {
                    g.update(SIM_DT, &mut ctx);
                }
                assert_eq!(g.mode, k);
                if k > 0 {
                    let before = g.mean_u_at_grains();
                    assert!(before > 0.3, "seed {seed} mode {k}: right after {before}");
                }
                while g.time < start + 10.0 {
                    g.update(SIM_DT, &mut ctx);
                }
                let after = g.mean_u_at_grains();
                assert!(after < 0.15, "seed {seed} mode {k}: 10 s on {after}");
            }
        }
    }

    #[test]
    fn chl_a2_every_mode_change_tosses_the_sand() {
        // Sand sitting exactly on one mode's nodal set is on the next mode's lobes.
        for (i, &md) in MODES.iter().enumerate() {
            let next = MODES[(i + 1) % MODES.len()];
            let (mut sum, mut n) = (0.0, 0);
            for k in 0..200 {
                for l in 0..200 {
                    let (x, y) = (f64::from(k) / 199.0, f64::from(l) / 199.0);
                    if mode_u(md, x, y).abs() < 0.05 {
                        sum += mode_u(next, x, y).abs();
                        n += 1;
                    }
                }
            }
            let mean = sum / f64::from(n);
            assert!(mean > 0.3, "{md:?} -> {next:?}: {mean}");
        }
    }

    #[test]
    fn chl_a3_no_grain_leaves_the_plate() {
        let (mut g, mut ctx) = plate(4, Size::new(80, 24));
        for _ in 0..600 {
            g.update(0.25, &mut ctx);
            for (&x, &y) in g.gx.iter().zip(&g.gy) {
                assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
            }
        }
        g.prepare();
        let span = g.side - 1.0;
        for (&x, &y) in g.gx.iter().zip(&g.gy) {
            let (px, py) = (g.ox + x * span, g.oy + y * span);
            assert!(px >= g.ox && px <= g.ox + span && py >= g.oy && py <= g.oy + span);
        }
    }

    #[test]
    fn chl_a4_stipple_inks_follow_the_sign_and_density_is_capped() {
        for u in [-1.0, -0.4, -1e-3, 0.0, 1e-3, 0.5, 1.0, 3.0] {
            match plate_ink(u) {
                None => assert_eq!(u, 0.0),
                Some((p, ink)) => {
                    assert!(p <= 0.10 + 1e-12);
                    assert!(p <= 0.10 * f64::abs(u) + 1e-12);
                    let want = if u > 0.0 { Role::Hot } else { Role::Cold };
                    assert_eq!(ink.role, want);
                }
            }
        }
        // In the drawn plate: every stipple dot's sign matches its ink, and the share of
        // stippled dots stays under 10 %.
        let (mut g, _) = plate(5, Size::new(200, 50));
        g.prepare();
        let span = g.side - 1.0;
        let mut n = 0usize;
        for &(x, y, ink) in &g.stipple {
            let u = g.u_at((f64::from(x) - g.ox) / span, (f64::from(y) - g.oy) / span);
            match ink.role {
                Role::Hot => assert!(u > 0.0),
                Role::Cold => assert!(u < 0.0),
                r => panic!("{r:?}"),
            }
            n += 1;
        }
        let share = n as f64 / (g.side * g.side);
        assert!(share > 0.005 && share <= 0.10, "{share}");
    }

    #[test]
    fn chl_a4_stipple_holds_still_within_a_mode() {
        let (mut g, mut ctx) = plate(6, Size::new(120, 40));
        g.prepare();
        let first = g.stipple.clone();
        for _ in 0..300 {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        assert_eq!(first, g.stipple);
    }

    #[test]
    fn modes_cycle_on_the_timeline_and_the_readout_names_them() {
        let (mut g, mut ctx) = plate(7, Size::new(200, 50));
        for (k, md) in MODES.iter().enumerate() {
            while g.time < k as f64 * MODE_S + 1.0 {
                g.update(0.25, &mut ctx);
            }
            assert_eq!(g.mode, k);
            let (sign, ascii) = if md.plus { ('+', '+') } else { ('−', '-') };
            assert_eq!(
                g.readout.as_str(),
                format!("m {}  n {}  {sign}", md.m, md.n)
            );
            assert_eq!(
                g.readout_ascii.as_str(),
                format!("m {}  n {}  {ascii}", md.m, md.n)
            );
        }
        // On screen, in both glyph modes, at the first (+) and second (−) mode.
        for (mode, secs, want) in [
            (GlyphMode::Unicode, 1.0, "m 1  n 5  +"),
            (GlyphMode::Unicode, MODE_S + 1.0, "m 4  n 6  −"),
            (GlyphMode::Ascii, MODE_S + 1.0, "m 4  n 6  -"),
        ] {
            let mut stage = Stage::new(build(), 1, Size::new(200, 50), mode);
            for _ in 0..(secs / SIM_DT) as usize {
                stage.step(SIM_DT);
            }
            let frame = stage.render();
            let row: String = (0..200).map(|x| frame.get(x, 49).unwrap().ch).collect();
            assert!(row.trim_end().ends_with(want), "{mode:?}: {row}");
        }
    }

    #[test]
    fn plate_keeps_a_blank_row_above_and_below() {
        for (w, h) in [(80, 24), (120, 40), (200, 50), (500, 150)] {
            let size = Size::new(w, h);
            let (mut g, _) = plate(1, size);
            g.gx.clear();
            g.gy.clear();
            g.prepare();
            let rows = i32::from(h);
            let ds = g.dots.dot_size();
            let mut any = false;
            for y in 0..i32::from(ds.height()) {
                for x in 0..i32::from(ds.width()) {
                    if g.dots.is_lit(x, y) {
                        any = true;
                        let row = y / Dots::CELL_H;
                        assert!(
                            (1..=rows - 3).contains(&row),
                            "{w}x{h}: plate dot on row {row}"
                        );
                    }
                }
            }
            assert!(any);
            // And in the frame the runtime actually draws, sand included, at t = 0 and
            // after the sand has gathered.
            for secs in [0.0, 12.0] {
                let mut stage = Stage::new(build(), 1, size, GlyphMode::Unicode);
                for _ in 0..(secs / SIM_DT) as usize {
                    stage.step(SIM_DT);
                }
                let frame = stage.render();
                for row in [0, rows - 2] {
                    for x in 0..i32::from(w) {
                        let ch = frame.get(x, row).unwrap().ch;
                        assert!(
                            ch == ' ' || ch == '\u{2800}',
                            "{w}x{h} t={secs}: {ch:?} at ({x}, {row})"
                        );
                    }
                }
            }
            // Still a large plate.
            assert!(
                g.side >= 0.8 * f64::from((rows - 3) * Dots::CELL_H),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn chl_a4_candidate_stipple_equals_the_full_plate_scan() {
        let (mut g, _) = plate(8, Size::new(120, 40));
        for k in 0..MODES.len() {
            g.set_mode(k);
            g.rebuild_stipple();
            let mut brute = Vec::new();
            let n = g.side as i32;
            let last = (g.taps - 1) as f64;
            for j in 0..n {
                for i in 0..n {
                    let u = g.u_at(f64::from(i) / last, f64::from(j) / last);
                    if let Some((p, ink)) = plate_ink(u) {
                        let (x, y) = (g.ox as i32 + i, g.oy as i32 + j);
                        if stipple_keep(STIPPLE_SALT, x, y, p) {
                            brute.push((x, y, ink));
                        }
                    }
                }
            }
            assert_eq!(g.stipple, brute, "mode {k}");
        }
    }

    #[test]
    fn kicks_follow_simulated_time_not_step_count() {
        let (mut g, mut ctx) = plate(9, Size::new(80, 24));
        for _ in 0..120 {
            g.update(SIM_DT / 2.0, &mut ctx);
        }
        assert_eq!(g.kicks, 60);
        for _ in 0..4 {
            g.update(0.25, &mut ctx);
        }
        assert_eq!(g.kicks, 120);
    }

    #[test]
    fn tiny_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (2, 1), (1, 3), (5, 2)] {
            let (mut g, mut ctx) = plate(1, Size::new(w, h));
            for _ in 0..30 {
                g.update(0.25, &mut ctx);
                g.prepare();
            }
        }
    }
}
