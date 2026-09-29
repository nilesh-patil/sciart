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
use sciart_core::rng::{cell_key, hash01, value_noise};
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
/// Screen layout, as shares of the size: the Earth's limb (a band ~15 % of the height),
/// the chaser's start and the station along the width, and the rows of the station's
/// altitude and of the hops' lowest dip, so the path spans ~70 % of the width and of the
/// height above the limb whatever the size (the radial axis is exaggerated to fit).
const LIMB_TOP: f64 = 0.85;
const START_COL: f64 = 0.13;
const STATION_COL: f64 = 0.83;
const TOP_ROW: f64 = 0.2;
const DIP_ROW: f64 = 0.75;
/// How long a burn's arrow and plume flash show (s).
const BURN_SHOW: f64 = 1.2;
/// The recent trail with a glow under it (s).
const TRAIL: f64 = 4.0;
/// Planned-path samples, and the dotted plan's spacing (dots of arc length).
const PLAN_N: usize = 1200;
const PLAN_GAP: f64 = 3.0;
/// Exhaust dots in a thruster puff.
const PUFF_DOTS: i32 = 18;
/// The inertial inset needs this many columns and rows (below it, a disc of a few
/// pixels reads as a notched square).
const INSET_MIN: (f64, f64) = (100.0, 30.0);

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
/// Every burn: its loop time and Δv direction along track.
const BURNS: [(f64, f64); 4] = [
    (T_PHASE, -1.0),
    (T_RAISE, 1.0),
    (T_APPROACH, 1.0),
    (T_APPROACH + APPROACH, -1.0),
];

/// Station sprite, left to right from the centre: offset, glyph, part.
#[derive(Debug, Clone, Copy)]
enum Part {
    Truss,
    Panel,
    Module,
}
const STATION: [(i32, Glyph, Part); 9] = [
    (-4, Glyph::new('▓', '#'), Part::Panel),
    (-3, Glyph::new('▓', '#'), Part::Panel),
    (-2, Glyph::new('═', '='), Part::Truss),
    (-1, Glyph::new('═', '='), Part::Truss),
    (0, Glyph::new('█', 'H'), Part::Module),
    (1, Glyph::new('═', '='), Part::Truss),
    (2, Glyph::new('═', '='), Part::Truss),
    (3, Glyph::new('▓', '#'), Part::Panel),
    (4, Glyph::new('▓', '#'), Part::Panel),
];

fn burn(t: f64) -> Option<(f64, f64)> {
    let t = t.rem_euclid(LOOP);
    BURNS
        .into_iter()
        .find(|&(at, _)| t >= at && t < at + BURN_SHOW)
}

/// Chaser sprite, left to right from the nose cell: offset, glyph, part.
const CHASER: [(i32, Glyph, CraftPart); 4] = [
    (-3, Glyph::new('━', '='), CraftPart::Array),
    (-2, Glyph::new('▐', '['), CraftPart::Hull),
    (-1, Glyph::new('█', '#'), CraftPart::Hull),
    (0, Glyph::new('▶', '>'), CraftPart::Nose),
];
#[derive(Debug, Clone, Copy)]
enum CraftPart {
    Array,
    Hull,
    Nose,
}

#[derive(Debug, Default)]
pub struct OrbitalRendezvous {
    seeded: bool,
    t: f64,
    timeline: Option<Timeline>,
    pixels: Pixels,
    dots: Dots,
    /// The whole loop's path in dot coordinates, `PLAN_N` samples evenly in time.
    plan: Vec<(f64, f64)>,
    /// The plan resampled every `PLAN_GAP` dots of arc length, with each dot's loop
    /// time: the dotted "still to fly" path.
    plan_dots: Vec<(f64, f64, f64)>,
    stars: Vec<(i32, i32, u8)>,
    salt: u64,
    /// Screen: cells, the station's cell position, and cells per unit along track
    /// (right) and radially (up).
    w: f64,
    h: f64,
    origin: (f64, f64),
    scale: (f64, f64),
}

/// Light pixel `(x, y)` with `ink` unless a brighter pixel of the same role is there.
fn glow_max(pixels: &mut Pixels, x: i32, y: i32, ink: Ink) {
    if let Some(old) = pixels.get(x, y)
        && (old.role != ink.role || old.level < ink.level)
    {
        pixels.set(x, y, ink);
    }
}

impl OrbitalRendezvous {
    /// LVLH `(x, y)` → cell position.
    fn to_cells(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.origin.0 + y * self.scale.0,
            self.origin.1 - x * self.scale.1,
        )
    }

    /// The chaser's cell (its nose), held off the station's sprite so it docks at the
    /// port instead of overlapping the panels.
    fn chaser_cell(&self, t: f64) -> (f64, f64) {
        let s = chaser(t);
        let (cx, cy) = self.to_cells(s[0], s[1]);
        (cx.min(self.origin.0 - 5.0), cy)
    }

    fn earth(&mut self) {
        // Earth's limb along the bottom: a huge disc, a deep ocean under soft streaks of
        // cloud streaming with the orbit, a bright atmosphere at the edge and a faint
        // airglow above it, every pixel shaded by its signed distance to the limb so the
        // edge is antialiased below the half-block.
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()));
        let r = 3.0 * pw;
        let c = (0.5 * pw, LIMB_TOP * ph + r);
        let drift = 0.9 * self.t;
        let salt = self.salt;
        let deep = 0.14 * ph;
        let cloud_at = |fx: f64, depth: f64| {
            // Foreshortened toward the limb: √depth squeezes the streaks near the edge.
            let (u, v) = (fx + drift, 3.2 * depth.max(0.0).sqrt());
            // Streaks: stretched along the flow, warped, with a finer grain.
            let warp = value_noise(salt ^ 1, 0.3 * u, v, 6.0) - 0.5;
            let streak = value_noise(salt, 0.35 * u + 6.0 * warp, 3.5 * v, 6.0);
            let fine = value_noise(salt ^ 2, 0.8 * u, 5.0 * v, 4.0);
            let k = (0.75 * streak + 0.25 * fine - 0.55) / 0.2;
            let k = k.clamp(0.0, 1.0);
            k * k * (3.0 - 2.0 * k)
        };
        for y in ((LIMB_TOP * ph) as i32 - 6).max(0)..i32::from(ps.height()) {
            for x in 0..i32::from(ps.width()) {
                let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let depth = r - (fx - c.0).hypot(fy - c.1);
                if depth < -4.0 {
                    continue;
                }
                // Outside: airglow fading over a few pixels; inside: ocean and cloud with
                // the atmosphere's bright rim. Blended by the pixel's cover of the disc.
                let air = 0.5 * (depth.min(0.0) / 1.1).exp();
                let inside = {
                    let ocean = 0.3 + 0.2 * (1.0 - depth / deep).max(0.0);
                    let surface = ocean + (0.93 - ocean) * 0.9 * cloud_at(fx, depth);
                    let rim = (-depth.max(0.0) / 1.4).exp();
                    surface + (0.8 - surface).max(0.0) * rim
                };
                let cover = (depth + 0.5).clamp(0.0, 1.0);
                let v = cover * inside + (1.0 - cover) * air;
                if v * 255.0 < ink::MIN_VISIBLE_LEVEL {
                    continue;
                }
                self.pixels.set(x, y, ink::ramp(Ramp::Ice, v));
            }
        }
    }

    /// Inertial inset (top right): Earth, the station's circular orbit and both craft,
    /// the chaser's radius and lead exaggerated. Only where the disc has enough pixels
    /// to read as round.
    fn inset(&mut self) {
        if self.w < INSET_MIN.0 || self.h < INSET_MIN.1 {
            return;
        }
        // Orbit radius in dots (square); a pixel is 2 × 2 dots.
        let rd = 0.3 * self.h;
        let (dcx, dcy) = (2.0 * self.w - 1.3 * rd - 4.0, 1.2 * rd + 2.0);
        let pixels = &mut self.pixels;
        disc_pixels((dcx / 2.0, dcy / 2.0), 0.24 * rd, |x, y, cover| {
            // A lit hemisphere: brighter toward the Sun (upper left), antialiased edge.
            pixels.set(x, y, ink::ramp(Ramp::Ice, cover * 0.55));
        });
        let orbit = Ink::new(Role::Ramp(Ramp::Starlight), 60);
        for i in 0..96 {
            if i % 2 == 0 {
                let a = TAU * f64::from(i) / 96.0;
                let (s, co) = a.sin_cos_det();
                self.dots.plot(dcx + rd * co, dcy + rd * s, orbit);
            }
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
        let now = self.t.rem_euclid(LOOP);
        // Still to fly: a dim dotted line.
        let plan = Ink::new(Role::Ramp(Ramp::Aurora), 105);
        for &(x, y, at) in &self.plan_dots {
            if at > now {
                self.dots.plot(x, y, plan);
            }
        }
        // Flown: a solid line, brightest near the chaser and fading with age, over a
        // soft glow along the last few seconds.
        let step = LOOP / PLAN_N as f64;
        let flown = ((now / step) as usize).min(PLAN_N - 1);
        let (hx, hy) = {
            let (cx, cy) = self.chaser_cell(self.t);
            Dots::from_cells(cx, cy + 0.5)
        };
        let mut prev = None;
        for i in 0..=flown {
            let (x, y) = if i == flown { (hx, hy) } else { self.plan[i] };
            let age = now - i as f64 * step;
            if let Some((px, py)) = prev {
                let level = 180.0 + 70.0 * (-age / 8.0).exp();
                self.dots.line(
                    px,
                    py,
                    x,
                    y,
                    Ink::new(Role::Ramp(Ramp::Aurora), level as u8),
                );
            }
            if age < TRAIL && i % 2 == 0 {
                // Pixel = 2 × 2 dots; a faint glow under the line.
                let fade = 1.0 - age / TRAIL;
                let (gx, gy) = ((x / 2.0) as i32, (y / 2.0) as i32);
                let glow = ink::ramp(Ramp::Aurora, 0.04 + 0.09 * fade);
                glow_max(&mut self.pixels, gx, gy, glow);
            }
            prev = Some((x, y));
        }
        // A burn: the plume flashes (a hot glow at the burn marker that swells and
        // fades) and a spreading cone of exhaust dots streams opposite the Δv.
        if let Some((at, dir)) = burn(self.t) {
            let age = (now - at).clamp(0.0, BURN_SHOW);
            let k = 1.0 - age / BURN_SHOW;
            let (cx, cy) = self.chaser_cell(self.t);
            // The engine end: ahead of the nose for a retrograde burn, behind the array
            // for a prograde one.
            let nozzle = if dir < 0.0 { cx + 1.5 } else { cx - 3.5 };
            let (x0, y0) = Dots::from_cells(nozzle, cy + 0.5);
            let flash_r = 0.8 + 1.4 * (age / BURN_SHOW).sqrt();
            let pixels = &mut self.pixels;
            disc_pixels((x0 / 2.0, y0 / 2.0), flash_r, |x, y, cover| {
                let d = (f64::from(x) + 0.5 - x0 / 2.0).hypot(f64::from(y) + 0.5 - y0 / 2.0);
                let v = (0.7 + 0.3 * k) * cover * (1.0 - 0.2 * d / flash_r);
                if v >= 0.62 {
                    glow_max(pixels, x, y, ink::ramp(Ramp::Sulfur, v));
                }
            });
            let puff = ink::ramp(Ramp::Sulfur, 0.95 - 0.5 * (age / BURN_SHOW));
            for n in 0..PUFF_DOTS {
                let u = hash01(cell_key(n, 7));
                let v = hash01(cell_key(n, 9)) - 0.5;
                let reach = (2.0 + 9.0 * age) * (0.3 + 0.7 * u);
                self.dots
                    .plot(x0 - dir * (1.0 + reach), y0 + v * reach * 0.9, puff);
            }
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
        // Scale each axis to the trajectory: along track from the start gap to the
        // station, radially from the station's altitude to the hops' lowest dip.
        self.origin = (STATION_COL * self.w, TOP_ROW * self.h);
        let dip = 4.0 * DV / N;
        self.scale = (
            (STATION_COL - START_COL) * self.w / -START,
            (DIP_ROW - TOP_ROW) * self.h / dip,
        );
        self.plan.clear();
        self.plan_dots.clear();
        let mut carry = 0.0;
        for i in 0..PLAN_N {
            let at = LOOP * i as f64 / PLAN_N as f64;
            let s = chaser(at);
            let (cx, cy) = self.to_cells(s[0], s[1]);
            let p = Dots::from_cells(cx.min(self.origin.0 - 5.0), cy + 0.5);
            if let Some(&(qx, qy)) = self.plan.last() {
                carry += (p.0 - qx).hypot(p.1 - qy);
            }
            if i == 0 || carry >= PLAN_GAP {
                carry = 0.0;
                self.plan_dots.push((p.0, p.1, at));
            }
            self.plan.push(p);
        }
        let ps = self.pixels.pixel_size();
        self.stars.clear();
        for k in 0..(size.area() / 50) as i32 {
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
        // The station: a truss with two solar-panel wings a side, modules at the centre.
        let truss = Ink::new(Role::Ramp(Ramp::Ferroin), 200);
        let panel = Ink::new(Role::Ramp(Ramp::Ferroin), 140);
        let module = Ink::new(Role::Ramp(Ramp::Starlight), 220);
        for (dx, glyph, ink) in STATION {
            let ink = match ink {
                Part::Truss => truss,
                Part::Panel => panel,
                Part::Module => module,
            };
            canvas.put(sx + dx, sy, glyph, ink);
        }
        for dx in [-3, 3] {
            canvas.put(sx + dx, sy - 1, Glyph::new('▄', '_'), panel);
            canvas.put(sx + dx, sy + 1, Glyph::new('▀', '-'), panel);
        }
        let (cx, cy) = self.chaser_cell(self.t);
        let (cx, cy) = (cx.floor() as i32, cy.floor() as i32);
        // Where each burn fired so far this loop: a small star on the path.
        let now = self.t.rem_euclid(LOOP);
        for (at, _) in BURNS {
            if now >= at {
                let (bx, by) = self.chaser_cell(at);
                let (bx, by) = (bx.floor() as i32, by.floor() as i32);
                let on_craft = by == cy && (cx - 3..=cx).contains(&bx);
                if !on_craft && canvas.get(bx, by).is_some_and(|c| c.ch != '═') {
                    canvas.put(bx, by, Glyph::new('✶', '*'), Ink::new(Role::Accent, 160));
                }
            }
        }
        // The chaser: a solar array, a two-cell hull and a bright nose, pointing
        // prograde (toward the station).
        for (dx, glyph, part) in CHASER {
            let ink = match part {
                CraftPart::Array => Ink::new(Role::Ramp(Ramp::Cobalt), 170),
                CraftPart::Hull => Ink::new(
                    Role::Ramp(Ramp::Starlight),
                    if dx == -1 { 215 } else { 185 },
                ),
                CraftPart::Nose => Ink::new(Role::Ramp(Ramp::Aurora), 245),
            };
            canvas.put(cx + dx, cy, glyph, ink);
        }
        // A burn: Δv arrow beside the chaser (the plume points the other way).
        if let Some((_, dir)) = burn(self.t) {
            let accent = Ink::new(Role::Accent, 220);
            let (arrow, at) = if dir < 0.0 {
                (Glyph::new('←', '<'), cx - 5)
            } else {
                (Glyph::new('→', '>'), cx + 2)
            };
            canvas.put(at, cy, arrow, accent);
        }
        self.pixels.backdrop(canvas);
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

    #[test]
    fn rdv_a6_the_view_is_scaled_to_the_trajectory() {
        for (w, h) in [(80, 24), (120, 40), (200, 50), (500, 150)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(3, size);
            let mut g = OrbitalRendezvous::default();
            g.resize(size, &mut ctx);
            let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for i in 0..=400 {
                let (x, y) = g.chaser_cell(LOOP * f64::from(i) / 400.0);
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
            let (fw, fh) = (f64::from(w), f64::from(h));
            let above_limb = LIMB_TOP * fh;
            assert!(
                (x1 - x0) / fw > 0.6,
                "{w}x{h}: width share {}",
                (x1 - x0) / fw
            );
            let hs = (y1 - y0) / above_limb;
            assert!((0.6..0.8).contains(&hs), "{w}x{h}: height share {hs}");
            assert!(
                y1 + 1.0 < above_limb,
                "{w}x{h}: dip {y1} vs limb {above_limb}"
            );
            // The chaser's nose never enters the station sprite.
            assert!(x1 < g.origin.0 - 4.0);
        }
    }

    #[test]
    fn rdv_a7_the_inset_is_drawn_only_where_its_disc_is_round() {
        for (w, h, want) in [(80, 24, false), (99, 40, false), (120, 40, true)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(3, size);
            let mut g = OrbitalRendezvous::default();
            g.resize(size, &mut ctx);
            // The inset's orbit ring is the only Starlight line work in the dots.
            let orbit = Ink::new(Role::Ramp(Ramp::Starlight), 60);
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.dots.blit(&mut canvas);
            let found = (0..i32::from(h)).any(|y| {
                (0..i32::from(w)).any(|x| canvas.get(x, y).is_some_and(|c| c.ink == orbit))
            });
            assert_eq!(found, want, "{w}x{h}");
        }
    }
}
