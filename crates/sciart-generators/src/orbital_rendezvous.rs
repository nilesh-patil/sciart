//! Orbital rendezvous: slow down to catch up (SPEC-002 `orbital_rendezvous`).
//!
//! The chaser's motion relative to the station is the closed-form solution of the
//! Clohessy–Wiltshire equations in the station's LVLH frame (`x` radial, up; `y` along
//! track, the direction of motion), with impulsive burns at scheduled times: a
//! retrograde phasing burn drops it into a lower, faster orbit where it gains a looping
//! cycloid's hop per orbit, a prograde burn stops the drift, and a slow V-bar approach
//! brings it to the docking port. Distances are in units of the starting gap; the radial
//! axis is exaggerated on screen.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, disc_pixels};
use sciart_core::rng::value_noise;
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "orbital_rendezvous",
    category: Category::Voyages,
    description: "A chaser brakes to drop below a space station, overtakes it and docks",
    title: "Rendezvous · brake to catch up",
    captions: &[PHASING, OVERTAKING, RAISE, DOCKING],
};

const PHASING: &str = "phasing burn";
const OVERTAKING: &str = "overtaking";
const RAISE: &str = "raise orbit";
const DOCKING: &str = "docking";

pub fn build() -> Box<dyn Generator> {
    Box::new(OrbitalRendezvous::default())
}

/// Orbital period of the station (s) and its mean motion.
pub(crate) const PERIOD: f64 = 10.0;
pub(crate) const N: f64 = TAU / PERIOD;
/// Acts (s): the phasing burn fires 1 s in; two orbits later the raise burn.
const ACTS: [(&str, f64); 4] = [
    (PHASING, 2.0),
    (OVERTAKING, 19.0),
    (RAISE, 3.0),
    (DOCKING, 16.0),
];
pub(crate) const T_PHASE: f64 = 1.0;
const HOPS: f64 = 2.0;
pub(crate) const T_RAISE: f64 = T_PHASE + HOPS * PERIOD;
const T_APPROACH: f64 = 24.0;
const APPROACH: f64 = 13.0;
pub(crate) const LOOP: f64 = 40.0;
/// Start gap (along track, behind), hold point after the hops, docking point.
const START: f64 = -1.0;
const HOLD: f64 = -0.2;
pub(crate) const DOCK: f64 = -0.03;
/// Phasing Δv, set so the hops end exactly at the hold point.
pub(crate) const DV: f64 = (HOLD - START) * N / (6.0 * PI * HOPS);
/// Radial exaggeration on screen.
const X_EXAG: f64 = 5.0;
/// How long a burn's arrow shows (s).
const BURN_SHOW: f64 = 1.2;
/// Trail length (s) and samples.
const TRAIL: f64 = 4.0;
const TRAIL_N: usize = 48;

/// LVLH state `(x, y, ẋ, ẏ)`.
pub(crate) type State = [f64; 4];

/// Clohessy–Wiltshire propagation of `s` by `t` (closed form).
pub(crate) fn cw(s: State, t: f64) -> State {
    let [x0, y0, vx0, vy0] = s;
    let (sn, cs) = (N * t).sin_cos_det();
    let nt = N * t;
    [
        (4.0 - 3.0 * cs) * x0 + sn / N * vx0 + 2.0 / N * (1.0 - cs) * vy0,
        6.0 * (sn - nt) * x0 + y0 - 2.0 / N * (1.0 - cs) * vx0 + (4.0 * sn - 3.0 * nt) / N * vy0,
        3.0 * N * sn * x0 + cs * vx0 + 2.0 * sn * vy0,
        -6.0 * N * (1.0 - cs) * x0 - 2.0 * sn * vx0 + (4.0 * cs - 3.0) * vy0,
    ]
}

/// The chaser's LVLH state at loop time `t`.
pub(crate) fn chaser(t: f64) -> State {
    let t = t.rem_euclid(LOOP);
    let start = [0.0, START, 0.0, 0.0];
    if t < T_PHASE {
        return start;
    }
    let after_phase = [0.0, START, 0.0, -DV];
    if t < T_RAISE {
        return cw(after_phase, t - T_PHASE);
    }
    let at_raise = cw(after_phase, T_RAISE - T_PHASE);
    let held = [at_raise[0], at_raise[1], at_raise[2], at_raise[3] + DV];
    if t < T_APPROACH {
        return cw(held, t - T_RAISE);
    }
    // V-bar approach: thrusting straight in along track, eased (not free motion).
    let from = cw(held, T_APPROACH - T_RAISE)[1];
    let p = ease::smoothstep((t - T_APPROACH) / APPROACH);
    [0.0, ease::lerp(from, DOCK, p), 0.0, 0.0]
}

/// A recent burn: its time and Δv direction along track (−1 retrograde, +1 prograde).
fn burn(t: f64) -> Option<(f64, f64)> {
    let t = t.rem_euclid(LOOP);
    [
        (T_PHASE, -1.0),
        (T_RAISE, 1.0),
        (T_APPROACH, 1.0),
        (T_APPROACH + APPROACH, -1.0),
    ]
    .into_iter()
    .find(|&(at, _)| t >= at && t < at + BURN_SHOW)
}

#[derive(Debug, Default)]
pub struct OrbitalRendezvous {
    seeded: bool,
    t: f64,
    timeline: Option<Timeline>,
    pixels: Pixels,
    dots: Dots,
    /// The whole loop's planned path in dot coordinates.
    plan: Vec<(f64, f64)>,
    stars: Vec<(i32, i32, u8)>,
    salt: u64,
    /// Screen: cells, and the LVLH panel's origin (station, cells) and scale (cells per
    /// unit along track).
    w: f64,
    h: f64,
    origin: (f64, f64),
    scale: f64,
}

impl OrbitalRendezvous {
    /// LVLH `(x, y)` → cell position.
    fn to_cells(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.origin.0 + y * self.scale,
            self.origin.1 - x * X_EXAG * self.scale / sciart_core::geom::ASPECT,
        )
    }

    fn earth(&mut self) {
        // Earth's limb along the bottom: a huge disc, ocean and drifting clouds, a thin
        // bright atmosphere at the edge.
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()));
        let r = 2.2 * pw;
        let c = (0.5 * pw, 0.83 * ph + r);
        let drift = 0.6 * self.t;
        let salt = self.salt;
        for y in ((0.8 * ph) as i32).max(0)..i32::from(ps.height()) {
            for x in 0..i32::from(ps.width()) {
                let d = (f64::from(x) + 0.5 - c.0).hypot(f64::from(y) + 0.5 - c.1);
                let depth = r - d;
                if depth < -1.5 {
                    continue;
                }
                let ink = if depth < 0.0 {
                    ink::ramp(Ramp::Cobalt, 0.35 * (1.5 + depth) / 1.5)
                } else if depth < 1.2 {
                    ink::ramp(Ramp::Cobalt, 0.85)
                } else {
                    let cloud = value_noise(salt, f64::from(x) + drift, f64::from(y) * 2.0, 5.0);
                    if cloud > 0.62 {
                        Ink::new(
                            Role::Ramp(Ramp::Starlight),
                            ink::level(175.0 + 180.0 * (cloud - 0.62)),
                        )
                    } else {
                        ink::ramp(
                            Ramp::Cobalt,
                            0.32 + 0.25 * (1.0 - depth / (0.2 * ph)).max(0.0),
                        )
                    }
                };
                self.pixels.set(x, y, ink);
            }
        }
    }

    /// Inertial inset (top right): Earth, the station's circular orbit and both craft,
    /// the chaser's radius and lead exaggerated.
    fn inset(&mut self) {
        if self.w < 60.0 || self.h < 18.0 {
            return;
        }
        let ps = self.pixels.pixel_size();
        let r = (0.085 * f64::from(ps.height())).max(3.0);
        let c = (f64::from(ps.width()) - 2.2 * r, 1.6 * r);
        let pixels = &mut self.pixels;
        disc_pixels(c, 0.45 * r, |x, y, cover| {
            if cover > 0.3 {
                pixels.set(x, y, ink::ramp(Ramp::Cobalt, 0.6));
            }
        });
        let orbit = Ink::new(Role::Ramp(Ramp::Ferroin), 60);
        let (dcx, dcy) = Dots::from_cells(c.0, c.1 / 2.0);
        let rd = 2.0 * r;
        for i in 0..72 {
            let a = TAU * f64::from(i) / 72.0;
            let (s, co) = a.sin_cos_det();
            self.dots.plot(dcx + rd * co, dcy + rd * s, orbit);
        }
        let theta = -N * self.t;
        let s = chaser(self.t);
        let place = |dr: f64, lead: f64| {
            let (sn, cs) = (theta - lead).sin_cos_det();
            (dcx + (rd + dr) * cs, dcy + (rd + dr) * sn)
        };
        let (tx, ty) = place(0.0, 0.0);
        self.dots
            .disc(tx, ty, 1.0, Ink::new(Role::Ramp(Ramp::Ferroin), 200));
        let (cx, cy) = place(60.0 * s[0], 0.5 * s[1]);
        self.dots
            .disc(cx, cy, 0.8, Ink::new(Role::Ramp(Ramp::Aurora), 230));
    }

    fn raster(&mut self) {
        self.pixels.clear();
        self.dots.clear();
        for &(x, y, l) in &self.stars {
            self.pixels
                .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
        }
        self.earth();
        // Planned path, faint; the recent trail, bright.
        let plan = Ink::new(Role::Ramp(Ramp::Aurora), 45);
        for (i, &(x, y)) in self.plan.iter().enumerate() {
            if i % 2 == 0 {
                self.dots.plot(x, y, plan);
            }
        }
        for i in 1..=TRAIL_N {
            let back = TRAIL * i as f64 / TRAIL_N as f64;
            let tt = self.t - back;
            if tt.rem_euclid(LOOP) > self.t.rem_euclid(LOOP) {
                break;
            }
            let s = chaser(tt);
            let (cx, cy) = self.to_cells(s[0], s[1]);
            let (x, y) = Dots::from_cells(cx, cy);
            let fade = 1.0 - back / TRAIL;
            self.dots
                .plot(x, y, ink::ramp(Ramp::Aurora, 0.35 + 0.55 * fade));
        }
        self.inset();
    }
}

impl Generator for OrbitalRendezvous {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seeded = true;
            self.timeline = Some(Timeline::new(&ACTS));
            self.salt = ctx.rng.random();
            self.t = ctx.rng.random_range(0.0..0.5);
        }
        self.pixels.resize(size);
        self.dots.resize(size);
        self.w = f64::from(size.width());
        self.h = f64::from(size.height());
        self.scale = 0.8 * self.w;
        self.origin = (0.86 * self.w, 0.46 * self.h);
        // Keep the lowest dip above the Earth's limb.
        let dip = 4.0 * DV / N * X_EXAG * self.scale / sciart_core::geom::ASPECT;
        if dip > 0.33 * self.h {
            self.scale *= 0.33 * self.h / dip;
        }
        self.plan.clear();
        let n = 600;
        for i in 0..n {
            let s = chaser(LOOP * f64::from(i) / f64::from(n));
            let (cx, cy) = self.to_cells(s[0], s[1]);
            self.plan.push(Dots::from_cells(cx, cy));
        }
        let ps = self.pixels.pixel_size();
        self.stars.clear();
        for k in 0..(size.area() / 50) as i32 {
            use sciart_core::rng::{cell_key, hash01};
            let x = (hash01(cell_key(k, 51)) * f64::from(ps.width())) as i32;
            let y = (hash01(cell_key(k, 53)) * f64::from(ps.height()) * 0.8) as i32;
            let l = 18 + (hash01(cell_key(k, 57)).powi(4) * 120.0) as u8;
            self.stars.push((x, y, l));
        }
        self.raster();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.raster();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
        // The station and the chaser as sprites over the line work.
        let (sx, sy) = self.to_cells(0.0, 0.0);
        let (sx, sy) = (sx.floor() as i32, sy.floor() as i32);
        let station = Ink::new(Role::Ramp(Ramp::Ferroin), 200);
        canvas.put(sx - 1, sy, Glyph::new('═', '-'), station);
        canvas.put(sx, sy, Glyph::new('╬', '#'), station);
        canvas.put(sx + 1, sy, Glyph::new('═', '-'), station);
        let s = chaser(self.t);
        let (cx, cy) = self.to_cells(s[0], s[1]);
        let (cx, cy) = (cx.floor() as i32, cy.floor() as i32);
        if (cx, cy) != (sx, sy) {
            canvas.put(
                cx,
                cy,
                Glyph::new('▶', '>'),
                Ink::new(Role::Ramp(Ramp::Aurora), 235),
            );
        }
        // A burn: Δv arrow beside the chaser (the plume points the other way).
        if let Some((_, dir)) = burn(self.t) {
            let accent = Ink::new(Role::Accent, 220);
            let (arrow, plume) = if dir < 0.0 {
                (Glyph::new('←', '<'), Glyph::new('≈', '~'))
            } else {
                (Glyph::new('→', '>'), Glyph::new('≈', '~'))
            };
            canvas.put(cx + dir as i32 * 2, cy, arrow, accent);
            canvas.put(
                cx - dir as i32,
                cy,
                plume,
                Ink::new(Role::Ramp(Ramp::Sulfur), 220),
            );
        }
    }

    fn caption(&self) -> Option<&'static str> {
        let tl = self.timeline.as_ref()?;
        Some(tl.at(self.t).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdv_a1_closed_form_satisfies_the_cw_equations() {
        let s0 = [0.02, -0.7, 0.01, -0.03];
        let h = 1e-4;
        for i in 0..50 {
            let t = f64::from(i) * 0.37;
            let (a, b, c) = (cw(s0, t - h), cw(s0, t), cw(s0, t + h));
            let xdd = (c[0] - 2.0 * b[0] + a[0]) / (h * h);
            let ydd = (c[1] - 2.0 * b[1] + a[1]) / (h * h);
            let rx = xdd - (3.0 * N * N * b[0] + 2.0 * N * b[3]);
            let ry = ydd + 2.0 * N * b[2];
            assert!(rx.abs() < 1e-6 && ry.abs() < 1e-6, "t={t}: {rx} {ry}");
            // Velocities are the derivatives of the positions.
            assert!(((c[0] - a[0]) / (2.0 * h) - b[2]).abs() < 1e-6);
            assert!(((c[1] - a[1]) / (2.0 * h) - b[3]).abs() < 1e-6);
        }
    }

    #[test]
    fn rdv_a2_braking_drops_the_chaser_and_it_gains_on_the_station() {
        let mid = chaser(T_PHASE + PERIOD / 2.0);
        assert!(
            mid[0] < 0.0,
            "altitude after the retrograde burn: {}",
            mid[0]
        );
        let before = chaser(T_PHASE)[1];
        let one_orbit = chaser(T_PHASE + PERIOD)[1];
        assert!(one_orbit > before, "{one_orbit} vs {before}");
        assert!(one_orbit.abs() < before.abs());
    }

    #[test]
    fn rdv_a3_without_burns_a_co_orbiting_chaser_stays_put() {
        let s = [0.0, -0.6, 0.0, 0.0];
        for i in 0..100 {
            let p = cw(s, f64::from(i) * 0.9);
            assert!((p[0]).abs() < 1e-12 && (p[1] + 0.6).abs() < 1e-12);
        }
        assert_eq!(chaser(0.5), [0.0, START, 0.0, 0.0]);
    }

    #[test]
    fn rdv_a4_the_approach_ends_at_the_docking_point() {
        let end = chaser(LOOP - 0.5);
        assert!((end[1] - DOCK).abs() <= 0.01 * DOCK.abs(), "{end:?}");
        // The hops end at the hold point.
        let held = chaser(T_RAISE + 0.5);
        assert!(
            (held[1] - HOLD).abs() < 1e-6 && held[0].abs() < 1e-6,
            "{held:?}"
        );
    }

    #[test]
    fn rdv_a5_captions_follow_the_burns() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(3, size);
        let mut g = OrbitalRendezvous::default();
        g.resize(size, &mut ctx);
        for (t, want) in [
            (T_PHASE + 0.2, PHASING),
            (T_PHASE + 5.0, OVERTAKING),
            (T_RAISE + 0.5, RAISE),
            (T_APPROACH + 3.0, DOCKING),
        ] {
            g.t = t;
            assert_eq!(g.caption(), Some(want), "t = {t}");
        }
        // The burns sit inside their captioned acts.
        let tl = Timeline::new(&ACTS);
        assert_eq!(tl.at(T_PHASE).label, PHASING);
        assert_eq!(tl.at(T_RAISE + 1e-9).label, RAISE);
        assert_eq!(tl.at(T_APPROACH).label, DOCKING);
    }
}
