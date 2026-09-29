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
const START_COL: f64 = 0.08;
const STATION_COL: f64 = 0.76;
const TOP_ROW: f64 = 0.2;
const DIP_ROW: f64 = 0.75;
/// How long a burn's arrow and plume flash show (s).
const BURN_SHOW: f64 = 1.2;
/// The recent trail with a glow under it (s).
const TRAIL: f64 = 4.0;
/// Planned-path samples (the dim solid "still to fly" line and the flown trail).
const PLAN_N: usize = 1200;
/// Burn labels: the model's Δv in m/s for a 20 km starting gap and a 92.6-min orbit
/// (RDV-A9), as fixed strings so `render` never formats; `dv_ms` backs the numbers.
#[cfg(test)]
const GAP_M: f64 = 20_000.0;
#[cfg(test)]
const PERIOD_REAL: f64 = 5_556.0;
const DV_LABELS: [(&str, &str); 2] = [
    ("Δv −0.48 m/s", "dv -0.48 m/s"),
    ("Δv +0.48 m/s", "dv +0.48 m/s"),
];
/// Cells kept clear left of the station's centre: its sprite reaches `STATION_LEFT`.
const STATION_LEFT: i32 = -4;
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

/// A model Δv (gap units per model second) in m/s for the labels' assumed scale.
#[cfg(test)]
pub(crate) fn dv_ms(dv: f64) -> f64 {
    dv * GAP_M * PERIOD / PERIOD_REAL
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

/// Station sprite, 8 cells along V-bar (RDV-A8): offset from the centre, row, glyph,
/// part. The pressurised stack runs along track with the docking port facing the
/// chaser, truss joints carry solar-array wings above and below.
#[derive(Debug, Clone, Copy)]
enum Part {
    Port,
    Truss,
    Panel,
    Module,
}
const STATION: [(i32, i32, Glyph, Part); 12] = [
    (STATION_LEFT, 0, Glyph::new('═', '='), Part::Port),
    (-3, 0, Glyph::new('▐', '['), Part::Module),
    (-2, 0, Glyph::new('█', '#'), Part::Module),
    (-1, 0, Glyph::new('╋', '+'), Part::Truss),
    (0, 0, Glyph::new('█', 'H'), Part::Module),
    (1, 0, Glyph::new('╋', '+'), Part::Truss),
    (2, 0, Glyph::new('█', '#'), Part::Module),
    (3, 0, Glyph::new('▌', ']'), Part::Module),
    (-1, -1, Glyph::new('▓', '#'), Part::Panel),
    (1, -1, Glyph::new('▓', '#'), Part::Panel),
    (-1, 1, Glyph::new('▓', '#'), Part::Panel),
    (1, 1, Glyph::new('▓', '#'), Part::Panel),
];

fn burn(t: f64) -> Option<(f64, f64)> {
    let t = t.rem_euclid(LOOP);
    BURNS
        .into_iter()
        .find(|&(at, _)| t >= at && t < at + BURN_SHOW)
}

/// Chaser sprite, 6 cells (RDV-A8), left to right from the nose cell: offset, glyph,
/// part.
const CHASER: [(i32, Glyph, CraftPart); 6] = [
    (-5, Glyph::new('━', '-'), CraftPart::Array),
    (-4, Glyph::new('━', '='), CraftPart::Array),
    (-3, Glyph::new('▐', '['), CraftPart::Hull),
    (-2, Glyph::new('█', '#'), CraftPart::Hull),
    (-1, Glyph::new('█', '#'), CraftPart::Hull),
    (0, Glyph::new('▶', '>'), CraftPart::Nose),
];
/// The chaser's tail cell (its array's end), from the nose.
const CHASER_TAIL: i32 = -5;
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
    stars: Vec<(i32, i32, u8)>,
    salt: u64,
    /// Screen: cells, the station's cell position, and cells per unit along track
    /// (right) and radially (up).
    w: f64,
    h: f64,
    origin: (f64, f64),
    scale: (f64, f64),
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
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
        (cx.min(self.origin.0 + f64::from(STATION_LEFT) - 1.0), cy)
    }

    /// The cell where a burn at loop time `at` fired.
    fn burn_cell(&self, at: f64) -> (i32, i32) {
        let (bx, by) = self.chaser_cell(at);
        (bx.floor() as i32, by.floor() as i32)
    }

    /// Left end and row of burn `k`'s Δv label: two rows above the burn, centred on it,
    /// kept on screen and clear of the station's sprite (which ends one column short).
    fn dv_label_at(&self, k: usize, bx: i32, by: i32) -> (i32, i32) {
        let len = DV_LABELS.get(k).map_or(0, |l| l.0.chars().count()) as i32;
        let right = self.origin.0.floor() as i32 + STATION_LEFT - 2 - len;
        ((bx - len / 2).min(right).max(0), (by - 2).max(0))
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
    /// The inset's centre and orbit radius (dots), when it is drawn (RDV-A7).
    fn inset_geometry(&self) -> Option<(f64, f64, f64)> {
        if self.w < INSET_MIN.0 || self.h < INSET_MIN.1 {
            return None;
        }
        // Orbit radius in dots (square); a pixel is 2 × 2 dots.
        let rd = 0.4 * self.h;
        Some((2.0 * self.w - 1.2 * rd - 4.0, 1.15 * rd + 2.0, rd))
    }

    /// Whether background pixel `(x, y)` lies clear of the Δv labels and the inset, so
    /// no star shows through as a block behind text or inside the globe's frame.
    fn star_clear(&self, x: i32, y: i32) -> bool {
        for (k, (at, _)) in BURNS.into_iter().take(DV_LABELS.len()).enumerate() {
            let (bx, by) = self.burn_cell(at);
            let (lx, ly) = self.dv_label_at(k, bx, by);
            if y / 2 == ly && (lx - 1..=lx + 12).contains(&x) {
                return false;
            }
        }
        match self.inset_geometry() {
            Some((dcx, dcy, rd)) => {
                let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                (2.0 * px - dcx).hypot(2.0 * py - dcy) > rd + 3.0
            }
            None => true,
        }
    }

    fn inset(&mut self) {
        let Some((dcx, dcy, rd)) = self.inset_geometry() else {
            return;
        };
        // The Earth fills most of the orbit (radius 0.72 of it: a low orbit, its height
        // exaggerated).
        let re = 0.36 * rd;
        let (pcx, pcy) = (dcx / 2.0, dcy / 2.0);
        let salt = self.salt;
        let pixels = &mut self.pixels;
        disc_pixels((pcx, pcy), re, |x, y, cover| {
            // A lit globe: Lambert toward the Sun (upper left), a soft terminator, ocean
            // with a little cloud, and an antialiased edge.
            let (u, v) = (
                (f64::from(x) + 0.5 - pcx) / re,
                (f64::from(y) + 0.5 - pcy) / re,
            );
            let z = (1.0 - u * u - v * v).max(0.0).sqrt();
            let lit = (-0.6 * u - 0.45 * v + 0.66 * z).max(0.0);
            let cloud = value_noise(salt ^ 5, 3.0 * u, 3.0 * v, 1.0);
            let albedo = 0.45 + 0.4 * ((cloud - 0.55) / 0.2).clamp(0.0, 1.0);
            let shade = 0.1 + 0.9 * lit.powf(0.8);
            let val = cover * (albedo * shade).min(0.9);
            if val * 255.0 >= ink::MIN_VISIBLE_LEVEL {
                pixels.set(x, y, ink::ramp(Ramp::Ice, val));
            }
        });
        let orbit = Ink::new(Role::Ramp(Ramp::Starlight), 60);
        for i in 0..160 {
            if i % 2 == 0 {
                let a = TAU * f64::from(i) / 160.0;
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
        // Still to fly: a dim solid line from the chaser on (drawn after the trail, so it
        // never covers it; `Dots` keeps the brighter ink where they meet).
        let plan = Ink::new(Role::Ramp(Ramp::Aurora), 78);
        let mut prev = (hx, hy);
        for &(x, y) in self.plan.iter().skip(flown + 1) {
            self.dots.line(prev.0, prev.1, x, y, plan);
            prev = (x, y);
        }
        // A burn: the plume flashes (a hot glow at the burn marker that swells and
        // fades) and a spreading cone of exhaust dots streams opposite the Δv.
        if let Some((at, dir)) = burn(self.t) {
            let age = (now - at).clamp(0.0, BURN_SHOW);
            let k = 1.0 - age / BURN_SHOW;
            let (cx, cy) = self.chaser_cell(self.t);
            // The engine end: ahead of the nose for a retrograde burn, behind the array
            // for a prograde one.
            let nozzle = if dir < 0.0 {
                cx + 1.5
            } else {
                cx + f64::from(CHASER_TAIL) - 0.5
            };
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
        for i in 0..PLAN_N {
            let at = LOOP * i as f64 / PLAN_N as f64;
            let (cx, cy) = self.chaser_cell(at);
            self.plan.push(Dots::from_cells(cx, cy + 0.5));
        }
        let ps = self.pixels.pixel_size();
        self.stars.clear();
        for k in 0..(size.area() / 50) as i32 {
            let x = (hash01(cell_key(k, 51)) * f64::from(ps.width())) as i32;
            let y = (hash01(cell_key(k, 53)) * f64::from(ps.height()) * 0.8) as i32;
            let l = 18 + (hash01(cell_key(k, 57)).powi(4) * 120.0) as u8;
            if self.star_clear(x, y) {
                self.stars.push((x, y, l));
            }
        }
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_ramp_cell(top, bottom));
        self.dots.blit_over(canvas, &self.pixels);
        // The station and the chaser as sprites over the line work.
        let (sx, sy) = self.to_cells(0.0, 0.0);
        let (sx, sy) = (sx.floor() as i32, sy.floor() as i32);
        // The station: modules along V-bar, truss joints with solar-array wings.
        for (dx, dy, glyph, part) in STATION {
            let ink = match part {
                Part::Port => Ink::new(Role::Ramp(Ramp::Starlight), 170),
                Part::Truss => Ink::new(Role::Ramp(Ramp::Ferroin), 200),
                Part::Panel => Ink::new(Role::Ramp(Ramp::Ferroin), 150),
                Part::Module => {
                    Ink::new(Role::Ramp(Ramp::Starlight), if dx == 0 { 225 } else { 200 })
                }
            };
            canvas.put(sx + dx, sy + dy, glyph, ink);
        }
        let (cx, cy) = self.chaser_cell(self.t);
        let (cx, cy) = (cx.floor() as i32, cy.floor() as i32);
        // Where each burn fired so far this loop: a small star on the path; the two CW
        // burns also get their Δv above it (RDV-A9).
        let now = self.t.rem_euclid(LOOP);
        for (k, (at, _)) in BURNS.into_iter().enumerate() {
            if now < at {
                continue;
            }
            let (bx, by) = self.burn_cell(at);
            let on_craft = by == cy && (cx + CHASER_TAIL..=cx).contains(&bx);
            if !on_craft && bx < sx + STATION_LEFT {
                canvas.put(bx, by, Glyph::new('✶', '*'), Ink::new(Role::Accent, 160));
            }
            if let Some(&(uni, ascii)) = DV_LABELS.get(k) {
                let (lx, ly) = self.dv_label_at(k, bx, by);
                sciart_core::draw::text_mode(
                    canvas,
                    lx,
                    ly,
                    uni,
                    ascii,
                    Ink::new(Role::Accent, 150),
                );
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
                (Glyph::new('←', '<'), cx + CHASER_TAIL - 2)
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
        g.prepare();
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
            g.prepare();
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
    fn rdv_a8_zoomed_on_v_bar_with_readable_sprites_and_a_solid_plan() {
        let span = |offs: &mut dyn Iterator<Item = i32>| {
            let v: Vec<i32> = offs.collect();
            v.iter().max().unwrap() - v.iter().min().unwrap() + 1
        };
        let station = span(&mut STATION.iter().map(|p| p.0));
        let craft = span(&mut CHASER.iter().map(|p| p.0));
        assert!((6..=8).contains(&station), "station {station} cells");
        assert!((6..=8).contains(&craft), "chaser {craft} cells");
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(3, size);
            let mut g = OrbitalRendezvous::default();
            g.resize(size, &mut ctx);
            g.prepare();
            let share = g.origin.0 / f64::from(w);
            assert!(
                (0.74..=0.78).contains(&share),
                "{w}x{h}: station at {share}"
            );
            // The plan still to fly is a solid line: every sample's dot is lit.
            g.t = 0.2;
            g.raster();
            for &(x, y) in &g.plan[1..] {
                assert!(
                    g.dots.is_lit(x.round() as i32, y.round() as i32),
                    "{w}x{h}: gap at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn rdv_a9_burns_carry_their_dv_label_clear_of_the_station() {
        // The labels' numbers are the model's Δv at the stated scale.
        let want = format!("{:.2}", dv_ms(DV));
        assert_eq!(want, "0.48");
        for (uni, ascii) in DV_LABELS {
            assert!(
                uni.ends_with(&format!("{want} m/s")) && ascii.ends_with(&format!("{want} m/s"))
            );
        }
        assert!(DV_LABELS[0].1.contains('-') && DV_LABELS[1].1.contains('+'));
        for (w, h) in [(80, 24), (120, 40), (200, 50), (500, 150)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(3, size);
            let mut g = OrbitalRendezvous::default();
            g.resize(size, &mut ctx);
            g.prepare();
            g.t = LOOP - 0.5;
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            g.render(&mut canvas);
            let sx = g.origin.0.floor() as i32;
            for (k, (at, _)) in BURNS.into_iter().take(2).enumerate() {
                let (bx, by) = g.burn_cell(at);
                let (lx, ly) = g.dv_label_at(k, bx, by);
                let row: String = (lx..lx + 12)
                    .map(|x| canvas.get(x, ly).map_or(' ', |c| c.ch))
                    .collect();
                assert_eq!(row, DV_LABELS[k].1, "{w}x{h} burn {k}");
                assert!(
                    lx + 12 < sx + STATION_LEFT,
                    "{w}x{h}: label into the station"
                );
            }
        }
    }

    #[test]
    fn rdv_a7_the_inset_is_drawn_only_where_its_disc_is_round() {
        for (w, h, want) in [(80, 24, false), (99, 40, false), (120, 40, true)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(3, size);
            let mut g = OrbitalRendezvous::default();
            g.resize(size, &mut ctx);
            g.prepare();
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
