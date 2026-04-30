//! A moving wave source bunching its fronts ahead and stretching them behind
//! (SPEC-002 `doppler_effect`).

use std::collections::VecDeque;
use std::f64::consts::TAU;

use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};

pub static META: Meta = Meta {
    name: "doppler_effect",
    category: Category::Physics,
    description: "A moving source compressing wavefronts ahead and stretching them behind",
    title: "Doppler effect · waves bunch up",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(DopplerEffect::default())
}

/// Wave speed, rows/s.
const C: f64 = 10.0;
/// Source speed, rows/s (β = 0.5).
const V: f64 = 0.5 * C;
/// Emission period, s.
const PERIOD: f64 = 0.35;
/// Where a fresh scene puts the source, as a fraction of the width.
const START_FRAC: f64 = 0.3;
/// tan(22.5°): octant boundary for picking a front glyph by its tangent direction.
const TAN_OCTANT: f64 = 0.414_213_562_373_095;
const MAX_SAMPLES: usize = 20_000;
const SOURCE: Glyph = Glyph::new('◉', '@');
/// The bottom-row legend, longest first (GEN-R13 picks the longest that clears the title).
/// The model is sound, so it names pitch and the speed of sound `c_s`, not light.
const LEGENDS: [&str; 3] = [
    "<- lower pitch  v = 0.5 c_s  higher pitch ->",
    "<- lower  v = 0.5 c_s  higher ->",
    "v = 0.5 c_s",
];

/// One circular wavefront: emitted at `(x0, y0)` (rows) at time `t0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Front {
    x0: f64,
    y0: f64,
    t0: f64,
}

impl Front {
    fn radius(&self, t: f64) -> f64 {
        C * (t - self.t0)
    }
}

/// Where the fronts cross the source's axis at time `t`: `(ahead, behind)`, each sorted by
/// distance from the source (DOP-A1). Crossings are `x0 ± c(t − t0)`.
#[cfg(test)]
pub(crate) fn axis_crossings<'a>(
    fronts: impl IntoIterator<Item = &'a Front>,
    t: f64,
    source_x: f64,
) -> (Vec<f64>, Vec<f64>) {
    let (mut ahead, mut behind) = (Vec::new(), Vec::new());
    for f in fronts {
        let r = f.radius(t);
        if f.x0 + r > source_x {
            ahead.push(f.x0 + r);
        }
        if f.x0 - r < source_x {
            behind.push(f.x0 - r);
        }
    }
    ahead.sort_by(f64::total_cmp);
    behind.sort_by(|a, b| b.total_cmp(a));
    (ahead, behind)
}

/// Upper bound on live fronts for a screen diagonal of `diag` rows (DOP-A2).
pub(crate) fn max_fronts(diag: f64) -> f64 {
    diag / (C * PERIOD) + 2.0
}

/// Glyph tracing the circle's tangent at direction `(c, s)` from its center (y down).
fn arc_glyph(c: f64, s: f64) -> Glyph {
    if s.abs() < TAN_OCTANT * c.abs() {
        Glyph::ascii(if c > 0.0 { ')' } else { '(' })
    } else if c.abs() < TAN_OCTANT * s.abs() {
        Glyph::ascii('-')
    } else if c * s > 0.0 {
        Glyph::ascii('/')
    } else {
        Glyph::ascii('\\')
    }
}

#[derive(Debug, Default)]
pub struct DopplerEffect {
    t: f64,
    /// Source position, rows (x already divided by ASPECT).
    sx: f64,
    sy: f64,
    /// Screen width in rows.
    width: f64,
    height: u16,
    margin: f64,
    diag: f64,
    /// Time since the last emission.
    acc: f64,
    fronts: VecDeque<Front>,
    ready: bool,
}

impl DopplerEffect {
    /// Fronts whose radius exceeds the diagonal can never reach a visible cell again.
    fn prune(&mut self) {
        while self
            .fronts
            .front()
            .is_some_and(|f| f.radius(self.t) > self.diag)
        {
            self.fronts.pop_front();
        }
    }

    /// Fill in the fronts a source that had been moving forever would have emitted, so the
    /// first frame already shows the steady-state pattern.
    fn prefill(&mut self) {
        self.fronts.clear();
        let n = (self.diag / (C * PERIOD)).floor() as u32;
        for k in (0..=n).rev() {
            let back = self.acc + f64::from(k) * PERIOD;
            self.fronts.push_back(Front {
                x0: self.sx - V * back,
                y0: self.sy,
                t0: self.t - back,
            });
        }
        self.prune();
    }
}

impl Generator for DopplerEffect {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        let width = f64::from(size.width()) / ASPECT;
        // Keep the source at the same fraction of the screen across resizes.
        let frac = if self.ready && self.width > 0.0 {
            self.sx / self.width
        } else {
            START_FRAC
        };
        self.width = width;
        self.height = size.height();
        self.margin = 2.0 + 0.15 * width;
        self.diag = width.hypot(f64::from(size.height()));
        self.sx = frac * width;
        self.sy = f64::from(size.height() / 2);
        self.ready = true;
        self.prefill();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.sx += V * dt;
        self.acc += dt;
        // Back-date each emission by the overshoot so fronts are exactly T apart in time and
        // vT apart in space regardless of the step size (DOP-A1).
        while self.acc >= PERIOD {
            self.acc -= PERIOD;
            self.fronts.push_back(Front {
                x0: self.sx - V * self.acc,
                y0: self.sy,
                t0: self.t - self.acc,
            });
        }
        // Wrap after emitting; fronts keep their emission points and simply age out.
        let span = self.width + 2.0 * self.margin;
        if self.sx > self.width + self.margin {
            self.sx -= span;
        }
        self.prune();
        debug_assert!(self.fronts.len() as f64 <= max_fronts(self.diag));
    }

    fn render(&self, canvas: &mut Canvas) {
        let diag = self.diag.max(1e-9);
        for f in &self.fronts {
            let r = f.radius(self.t);
            if r < 0.5 {
                continue;
            }
            let fade = (1.0 - r / diag).clamp(0.0, 1.0);
            // ≤ 0.5 cells between samples keeps the front 8-connected.
            let n = ((TAU * r * ASPECT * 2.0).ceil() as usize).clamp(8, MAX_SAMPLES);
            let (ds, dc) = (TAU / n as f64).sin_cos();
            let (mut c, mut s) = (1.0f64, 0.0f64);
            let mut last = None;
            for _ in 0..n {
                let x = ((f.x0 + r * c) * ASPECT).round() as i32;
                let y = (f.y0 + r * s).round() as i32;
                if last != Some((x, y)) {
                    last = Some((x, y));
                    // An observer in direction θ from the emission point hears
                    // f' = f / (1 − β cos θ): higher pitch iff cos θ > 0.
                    let role = if c > 0.0 { Role::Cold } else { Role::Hot };
                    let level = 55.0 + 200.0 * fade * (0.45 + 0.55 * c.abs());
                    canvas.put(x, y, arc_glyph(c, s), Ink::new(role, level as u8));
                }
                // Rotate by one sample step (cheaper than sin/cos per point).
                (c, s) = (c * dc - s * ds, s * dc + c * ds);
            }
        }

        let (sx, sy) = ((self.sx * ASPECT).round() as i32, self.sy.round() as i32);
        canvas.put(sx, sy, SOURCE, Ink::new(Role::Accent, 255));
        canvas.put(
            sx - 1,
            sy,
            Glyph::new('━', '='),
            Ink::new(Role::Accent, 150),
        );
        canvas.put(sx - 2, sy, Glyph::new('─', '-'), Ink::new(Role::Accent, 90));

        // Bottom-right, clear of the title (GEN-R13).
        let room = title::readout_room(canvas.size(), &Title::new(META.title, self.caption()));
        if self.height > 8
            && let Some(legend) = LEGENDS.iter().find(|s| s.len() <= room)
        {
            let y = i32::from(self.height) - 1;
            draw::label_right(canvas, y, legend, Ink::new(Role::Text, 170));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(w: u16, h: u16) -> (DopplerEffect, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(1, size);
        let mut g = DopplerEffect::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// Consecutive gaps of a sorted crossing list.
    fn gaps(v: &[f64]) -> Vec<f64> {
        v.windows(2).map(|p| (p[1] - p[0]).abs()).collect()
    }

    #[test]
    fn dop_a1_axis_spacing_is_c_minus_v_ahead_and_c_plus_v_behind() {
        // A wide screen so the source never wraps during the run.
        let (mut g, mut ctx) = fresh(400, 50);
        g.sx = 0.0;
        g.prefill();
        // Step with a dt that does not divide T, so emission back-dating matters.
        for _ in 0..(4.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        let stepped: Vec<_> = g.fronts.iter().filter(|f| f.t0 > 0.0).copied().collect();
        assert!(stepped.len() >= 10, "{} fronts", stepped.len());
        let (ahead, behind) = axis_crossings(&stepped, g.t, g.sx);
        assert_eq!(ahead.len(), stepped.len());
        assert_eq!(behind.len(), stepped.len());
        for gap in gaps(&ahead) {
            assert!((gap - (C - V) * PERIOD).abs() < 1e-9, "ahead gap {gap}");
        }
        for gap in gaps(&behind) {
            assert!((gap - (C + V) * PERIOD).abs() < 1e-9, "behind gap {gap}");
        }
    }

    #[test]
    fn dop_a1_holds_for_the_prefilled_steady_state_and_coarse_steps() {
        let (mut g, mut ctx) = fresh(400, 50);
        g.sx = 5.0;
        g.prefill();
        for _ in 0..12 {
            g.update(0.23, &mut ctx);
        }
        let (ahead, behind) = axis_crossings(&g.fronts, g.t, g.sx);
        for gap in gaps(&ahead) {
            assert!((gap - (C - V) * PERIOD).abs() < 1e-9, "ahead gap {gap}");
        }
        for gap in gaps(&behind) {
            assert!((gap - (C + V) * PERIOD).abs() < 1e-9, "behind gap {gap}");
        }
    }

    #[test]
    fn dop_a2_live_fronts_are_bounded_by_the_diagonal() {
        let (mut g, mut ctx) = fresh(200, 50);
        for (w, h) in [(200, 50), (80, 24), (3, 2), (1, 1), (0, 0), (300, 80)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            for i in 0..600 {
                g.update(if i % 7 == 0 { 0.25 } else { SIM_DT }, &mut ctx);
                assert!(
                    g.fronts.len() as f64 <= max_fronts(g.diag),
                    "{w}x{h}: {} fronts > {}",
                    g.fronts.len(),
                    max_fronts(g.diag)
                );
            }
        }
    }

    #[test]
    fn dop_source_wraps_and_fronts_keep_their_emission_points() {
        let (mut g, mut ctx) = fresh(80, 24);
        g.sx = g.width + g.margin - 0.01;
        let before: Vec<_> = g.fronts.iter().copied().collect();
        g.update(SIM_DT, &mut ctx);
        assert!(g.sx < 0.0, "source wrapped to the left margin: {}", g.sx);
        for f in &before {
            if let Some(same) = g.fronts.iter().find(|n| n.t0 == f.t0) {
                assert_eq!(same, f);
            }
        }
    }

    #[test]
    fn dop_fronts_ahead_are_cold_and_behind_hot() {
        let mut stage = Stage::new(build(), 3, Size::new(200, 50), GlyphMode::Ascii);
        for _ in 0..30 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let src = frame
            .cells()
            .iter()
            .position(|c| c.ch == '@')
            .expect("source visible");
        let (sx, sy) = ((src % 200) as i32, (src / 200) as i32);
        let right = (sx + 1..200).find_map(|x| frame.get(x, sy).filter(|c| c.ch == ')'));
        let left = (0..sx - 2)
            .rev()
            .find_map(|x| frame.get(x, sy).filter(|c| c.ch == '('));
        assert_eq!(right.map(|c| c.ink.role), Some(Role::Cold));
        assert_eq!(left.map(|c| c.ink.role), Some(Role::Hot));
    }

    #[test]
    fn dop_gen_r13_legend_names_pitch_and_sits_right_of_the_title() {
        // "Doppler effect · waves bunch up" takes cols 0..33 at 80 wide (45 chars left);
        // at 40 wide only "Doppler effect" shows, in cols 0..16 (22 chars left).
        for (w, h, legend) in [
            (80, 24, LEGENDS[0]),
            (54, 12, LEGENDS[1]),
            (40, 12, LEGENDS[2]),
        ] {
            let mut stage = Stage::new(build(), 3, Size::new(w, h), GlyphMode::Ascii);
            let row = stage.render().to_lines().pop().unwrap();
            assert!(row.ends_with(&format!(" {legend} ")), "{w}x{h}: {row:?}");
        }
        assert!(LEGENDS.iter().all(|s| !s.contains("shift")));
    }
}
