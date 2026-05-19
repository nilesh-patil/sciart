//! The James Webb Space Telescope unfolding on its way to L2 (SPEC-002 `jwst_unfolding`).
//!
//! A diorama driven by a looping `Timeline` of the real deployment sequence, compressed:
//! the sunshield pallets drop, the tower lifts the telescope, the mid-booms stretch the
//! shield and its five layers tension one by one, the secondary mirror swings out and the
//! two mirror wings fold into place. The last act cuts to a halo orbit around L2 in the
//! Sun–Earth rotating frame. The observatory is modelled in metres in a body frame (x
//! along the shield's long axis, y up from the shield towards the telescope, z out of
//! the mirror) and drawn through `geom::Camera` as filled polygons (half-block pixels)
//! with line work in braille dots.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use rand::Rng;
use sciart_core::geom::{self, v3, v3::V3};
use sciart_core::prelude::*;
use sciart_core::raster::{ascii_ramp_cell, disc_pixels};
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;
use sciart_core::{Projector, ink};

pub static META: Meta = Meta {
    name: "jwst_unfolding",
    category: Category::Voyages,
    description: "The James Webb Space Telescope unfolding its sunshield and mirror on the way to L2",
    title: "JWST · unfolding in space",
    captions: &[SHIELD, LAYERS, MIRROR, ORBIT],
};

const SHIELD: &str = "sunshield opens";
const LAYERS: &str = "layers tension";
const MIRROR: &str = "mirror unfolds";
const ORBIT: &str = "halo orbit at L2";

pub fn build() -> Box<dyn Generator> {
    Box::new(JwstUnfolding::default())
}

/// Deployment beats (s), in the order they happened (JWST-A1).
const BEATS: [(&str, f64); 6] = [
    ("pallets", 5.0),
    ("tower", 4.0),
    ("layers", 10.0),
    ("secondary", 5.0),
    ("wings", 8.0),
    ("orbit", 18.0),
];
const B_PALLETS: usize = 0;
const B_TOWER: usize = 1;
const B_LAYERS: usize = 2;
const B_SECONDARY: usize = 3;
const B_WINGS: usize = 4;
const B_ORBIT: usize = 5;
/// Share of the layers beat spent extending the mid-booms, then each layer's share.
const BOOMS: f64 = 0.35;
const LAYER_SHARE: f64 = (1.0 - BOOMS) / 5.0;

/// Sunshield outline in the x–z plane (m): an elongated hexagon, 21 m × 14 m.
const SHIELD_OUTLINE: [(f64, f64); 6] = [
    (-10.5, 0.0),
    (-3.6, 7.1),
    (3.6, 7.1),
    (10.5, 0.0),
    (3.6, -7.1),
    (-3.6, -7.1),
];
/// Visual spacing between tensioned layers (m; exaggerated).
const LAYER_GAP: f64 = 0.28;
/// Mirror: segment circumradius (m) and the primary's centre height, stowed and lifted.
const SEG: f64 = 0.76;
const MIRROR_Y: f64 = 4.2;
const TOWER_LIFT: f64 = 1.2;
/// Secondary mirror distance in front of the primary (m).
const SM_DIST: f64 = 7.0;
/// Camera: base yaw, sway, pitch, eye distance (m).
const YAW: f64 = 0.38;
const SWAY: f64 = 0.25;
const PITCH: f64 = 0.42;
const EYE: f64 = 70.0;

/// How far each part is deployed at loop time `t`, 0 (stowed) to 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Deploy {
    pub pallets: f64,
    pub tower: f64,
    pub booms: f64,
    pub layers: [f64; 5],
    pub secondary: f64,
    pub wings: [f64; 2],
    pub orbit: f64,
}

/// The deployment state at loop time `t` (JWST-A1, A2, A5).
pub(crate) fn deploy(timeline: &Timeline, t: f64) -> Deploy {
    let beat = timeline.at(t);
    let done = |b: usize| {
        if beat.index > b {
            1.0
        } else if beat.index == b {
            beat.progress
        } else {
            0.0
        }
    };
    let e = ease::in_out_cubic;
    let lp = done(B_LAYERS);
    let mut layers = [0.0; 5];
    for (k, l) in layers.iter_mut().enumerate() {
        let from = BOOMS + LAYER_SHARE * k as f64;
        *l = e((lp - from) / LAYER_SHARE);
    }
    let wp = done(B_WINGS);
    Deploy {
        pallets: e(done(B_PALLETS)),
        tower: e(done(B_TOWER)),
        booms: e(lp / BOOMS),
        layers,
        secondary: e(done(B_SECONDARY)),
        wings: [e(wp / 0.5), e((wp - 0.5) / 0.5)],
        orbit: done(B_ORBIT),
    }
}

/// The 18 primary segments in axial hex coordinates (flat-top): rings 1 and 2 of a hex
/// grid, the centre left open.
fn segments() -> [(i32, i32); 18] {
    let mut out = [(0, 0); 18];
    let mut n = 0;
    for q in -2i32..=2 {
        for r in -2i32..=2 {
            let s = -q - r;
            let ring = q.abs().max(r.abs()).max(s.abs());
            if (ring == 1 || ring == 2) && n < 18 {
                out[n] = (q, r);
                n += 1;
            }
        }
    }
    out
}

/// A convex polygon queued for painting: up to 6 pixel-space vertices, its depth and ink.
#[derive(Debug, Clone, Copy)]
struct Poly {
    pts: [(f64, f64); 6],
    n: usize,
    depth: f64,
    ink: Ink,
}

impl Default for Poly {
    fn default() -> Self {
        Self {
            pts: [(0.0, 0.0); 6],
            n: 0,
            depth: 0.0,
            ink: Ink::BLANK,
        }
    }
}

/// Fill a convex polygon (pixel coordinates) into `pixels`: every pixel whose centre is
/// inside all edges, either winding.
fn fill_convex(pixels: &mut Pixels, p: &Poly) {
    if p.n < 3 {
        return;
    }
    let pts = &p.pts[..p.n];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let size = pixels.pixel_size();
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    if !(x0.is_finite() && y1.is_finite()) || x1 < 0.0 || y1 < 0.0 || x0 > w || y0 > h {
        return;
    }
    let (ix0, ix1) = (x0.max(0.0).floor() as i32, x1.min(w - 1.0).ceil() as i32);
    let (iy0, iy1) = (y0.max(0.0).floor() as i32, y1.min(h - 1.0).ceil() as i32);
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            let (cx, cy) = (f64::from(ix) + 0.5, f64::from(iy) + 0.5);
            let (mut pos, mut neg) = (false, false);
            for i in 0..p.n {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[(i + 1) % p.n];
                let c = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
                pos |= c > 0.0;
                neg |= c < 0.0;
            }
            if !(pos && neg) {
                pixels.set(ix, iy, p.ink);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct JwstUnfolding {
    seeded: bool,
    t: f64,
    timeline: Option<Timeline>,
    pixels: Pixels,
    dots: Dots,
    polys: Vec<Poly>,
    stars: Vec<(i32, i32, u8)>,
    /// Screen centre (cells) and rows per metre.
    centre: (f64, f64),
    scale: f64,
}

impl JwstUnfolding {
    fn timeline(&self) -> Option<&Timeline> {
        self.timeline.as_ref()
    }

    fn state(&self) -> Deploy {
        self.timeline()
            .map_or_else(Deploy::default, |tl| deploy(tl, self.t))
    }

    fn beat_index(&self) -> usize {
        self.timeline().map_or(0, |tl| tl.at(self.t).index)
    }

    fn projector(&self) -> Projector {
        let sway = SWAY * (TAU * self.t / 50.0).sin_det();
        Camera {
            yaw: YAW + sway,
            pitch: PITCH,
            distance: EYE,
        }
        .projector()
    }

    /// World point → pixel position and depth.
    fn project(&self, pr: &Projector, p: V3) -> Option<(f64, f64, f64)> {
        let (x, y, z) = pr.project(p, self.centre, self.scale)?;
        Some((x, 2.0 * y, z))
    }

    fn push(&mut self, pr: &Projector, pts: &[V3], ink: Ink) {
        let mut poly = Poly {
            ink,
            ..Poly::default()
        };
        let mut depth = 0.0;
        for &p in pts.iter().take(6) {
            let Some((x, y, z)) = self.project(pr, p) else {
                return;
            };
            poly.pts[poly.n] = (x, y);
            poly.n += 1;
            depth += z;
        }
        poly.depth = depth / poly.n.max(1) as f64;
        if self.polys.len() < self.polys.capacity() {
            self.polys.push(poly);
        }
    }

    /// Sunshield half (`side = ±1` for fore/aft) of layer `k`.
    fn shield_half(&mut self, pr: &Projector, d: &Deploy, k: usize, side: f64) {
        let tension = d.layers[k];
        // Folded up against the telescope until the pallets drop.
        let fold = (1.0 - d.pallets) * FRAC_PI_2 * 0.96;
        let width = ease::lerp(0.18, 1.0, d.booms);
        let y = -LAYER_GAP * k as f64 * tension - 0.05 * k as f64;
        let mut pts = [[0.0; 3]; 4];
        let mut n = 0;
        for &(x, z) in &SHIELD_OUTLINE {
            if z * side < 0.0 {
                continue;
            }
            // Slack layers sag and crumple a little.
            let sag = (1.0 - tension) * d.booms * 0.35 * (x * 0.9 + k as f64).sin_det();
            let (x, z) = (x * width, z * ease::lerp(0.55, 1.0, d.pallets.min(1.0)));
            let (s, c) = fold.sin_cos_det();
            let (yy, zz) = (y + sag + z.abs() * s, z * c);
            if n < 4 {
                pts[n] = [x, yy, zz];
                n += 1;
            }
        }
        // Order the half-outline's points around its perimeter.
        let order = if side > 0.0 {
            [0, 1, 2, 3]
        } else {
            [0, 3, 2, 1]
        };
        let quad = [pts[order[0]], pts[order[1]], pts[order[2]], pts[order[3]]];
        let level = if tension > 0.0 {
            ease::lerp(70.0, 150.0 + 18.0 * k as f64, tension)
        } else {
            55.0 + 6.0 * k as f64
        };
        let ink = Ink::new(Role::Ramp(Ramp::Starlight), ink::level(level));
        self.push(pr, &quad, ink);
    }

    /// Primary-mirror segment `(q, r)`: its six corners (body frame) and whether it is
    /// on a wing (`Some(0)` left, `Some(1)` right).
    fn segment(&self, d: &Deploy, q: i32, r: i32) -> ([V3; 6], Option<usize>) {
        let (cx, cy) = (
            1.5 * SEG * f64::from(q),
            3f64.sqrt() * SEG * (f64::from(r) + f64::from(q) / 2.0),
        );
        let my = MIRROR_Y + TOWER_LIFT * d.tower;
        let wing = match q {
            -2 => Some(0),
            2 => Some(1),
            _ => None,
        };
        let mut out = [[0.0; 3]; 6];
        for (i, o) in out.iter_mut().enumerate() {
            let a = PI / 3.0 * i as f64;
            let (s, c) = a.sin_cos_det();
            let (x, y) = (cx + 1.04 * SEG * c, cy + 1.04 * SEG * s);
            // A slight parabolic dish, so the segments catch the light differently.
            let z = 0.02 * (x * x + y * y);
            *o = [x, my + y, z];
        }
        if let Some(side) = wing {
            let sign = if side == 0 { -1.0 } else { 1.0 };
            let hinge = sign * 1.5 * SEG * 1.5;
            let fold = (1.0 - d.wings[side]) * 0.53 * PI;
            let (s, c) = fold.sin_cos_det();
            for o in &mut out {
                let dx = o[0] - hinge;
                o[0] = hinge + dx * c;
                o[2] -= dx.abs() * s;
            }
        }
        (out, wing)
    }

    /// Where the secondary mirror is: swung from folded against the primary to its
    /// place `SM_DIST` in front.
    fn secondary(&self, d: &Deploy) -> (V3, V3) {
        let my = MIRROR_Y + TOWER_LIFT * d.tower;
        let hinge = [0.0, my - 2.6, 0.3];
        let angle = (1.0 - d.secondary) * 1.35;
        let (s, c) = angle.sin_cos_det();
        let arm = [0.0, 2.6 * s, SM_DIST * c + 0.4];
        (hinge, v3::add(hinge, arm))
    }

    fn draw_observatory(&mut self) {
        let d = self.state();
        let pr = self.projector();
        self.polys.clear();
        // Spacecraft bus under the shield (sun side).
        let bus = [
            [-1.4, -1.8, 1.4],
            [1.4, -1.8, 1.4],
            [1.4, -0.4, 1.4],
            [-1.4, -0.4, 1.4],
        ];
        self.push(&pr, &bus, Ink::new(Role::Ramp(Ramp::Starlight), 60));
        for k in 0..5 {
            for side in [1.0, -1.0] {
                self.shield_half(&pr, &d, k, side);
            }
        }
        // Mirror segments: gold, lit by a slow glint sweeping across.
        let glint_at = 3.0 * (TAU * self.t / 9.0).sin_det();
        for (q, r) in segments() {
            let (pts, wing) = self.segment(&d, q, r);
            let (cx, cy) = (
                1.5 * SEG * f64::from(q),
                3f64.sqrt() * SEG * (f64::from(r) + f64::from(q) / 2.0),
            );
            let glint = (-((0.8 * cx + 0.6 * cy - glint_at) / 1.2).powi(2)).exp();
            let fold = wing.map_or(0.0, |s| 1.0 - d.wings[s]);
            let jitter = hash01(cell_key(q, r)) * 20.0;
            let level = 130.0 + jitter + 90.0 * glint - 60.0 * fold;
            self.push(
                &pr,
                &pts,
                Ink::new(Role::Ramp(Ramp::Sulfur), ink::level(level)),
            );
        }
        // Painter's order: far first.
        self.polys
            .sort_unstable_by(|a, b| a.depth.total_cmp(&b.depth));
        self.pixels.clear();
        for &(x, y, l) in &self.stars {
            self.pixels
                .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
        }
        for p in &self.polys {
            fill_convex(&mut self.pixels, p);
        }
        // Line work: tower, secondary struts and mirror, shield edges.
        self.dots.clear();
        let strut = Ink::new(Role::Ramp(Ramp::Starlight), 150);
        let my = MIRROR_Y + TOWER_LIFT * d.tower;
        self.line(&pr, [0.0, -0.4, 0.0], [0.0, my - 2.8, -0.6], strut);
        let (_, sm) = self.secondary(&d);
        for anchor in [
            [0.0, my + 2.9, 0.2],
            [-2.6, my - 1.5, 0.2],
            [2.6, my - 1.5, 0.2],
        ] {
            self.line(&pr, anchor, sm, strut);
        }
        if let Some((x, y, _)) = self.project(&pr, sm) {
            let pixels = &mut self.pixels;
            disc_pixels((x, y), 0.9, |ix, iy, cover| {
                if cover > 0.2 {
                    pixels.set(ix, iy, Ink::new(Role::Ramp(Ramp::Sulfur), 235));
                }
            });
        }
    }

    /// A 3-D segment in braille dots.
    fn line(&mut self, pr: &Projector, a: V3, b: V3, ink: Ink) {
        let (Some(pa), Some(pb)) = (self.project(pr, a), self.project(pr, b)) else {
            return;
        };
        let (ax, ay) = Dots::from_cells(pa.0, pa.1 / 2.0);
        let (bx, by) = Dots::from_cells(pb.0, pb.1 / 2.0);
        self.dots.line(ax, ay, bx, by, ink);
    }

    /// The last act: the Sun–Earth rotating frame, JWST cruising out to a halo orbit
    /// around L2 (illustrative, not a computed CR3BP orbit).
    fn draw_orbit(&mut self, progress: f64) {
        self.pixels.clear();
        self.dots.clear();
        for &(x, y, l) in &self.stars {
            self.pixels
                .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
        }
        let ps = self.pixels.pixel_size();
        let (w, h) = (f64::from(ps.width()), f64::from(ps.height()));
        let earth = (0.24 * w, 0.5 * h);
        let er = (0.07 * h).max(1.5);
        // Earth lit from the Sun on the left: Cobalt, darker towards the night side.
        let pixels = &mut self.pixels;
        disc_pixels(earth, er, |x, y, cover| {
            let lit = ((earth.0 - (f64::from(x) + 0.5)) / er).clamp(-1.0, 1.0);
            let v = (0.55 + 0.45 * lit) * cover;
            if v > 0.1 {
                pixels.set(x, y, ink::ramp(Ramp::Cobalt, 0.25 + 0.7 * v));
            }
        });
        // The Moon on its orbit.
        let moon_a = TAU * self.t / 14.0;
        let (ms, mc) = moon_a.sin_cos_det();
        let mr = 0.16 * h;
        let moon = (earth.0 + mr * mc * 1.4, earth.1 + mr * ms * 0.6);
        disc_pixels(moon, (0.02 * h).max(0.7), |x, y, cover| {
            if cover > 0.3 {
                pixels.set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), 170));
            }
        });
        // L2 and the halo loop around it, seen obliquely.
        let l2 = (0.74 * w, 0.5 * h);
        let (ax, ay) = (0.08 * w, 0.3 * h);
        let halo = |a: f64| {
            let (s, c) = a.sin_cos_det();
            (
                l2.0 + ax * c,
                l2.1 + ay * s + 0.08 * h * (2.0 * a).sin_det(),
            )
        };
        let faint = Ink::new(Role::Ramp(Ramp::Starlight), 60);
        let steps = 96;
        for i in 0..steps {
            let (a0, a1) = (
                TAU * f64::from(i) / f64::from(steps),
                TAU * f64::from(i + 1) / f64::from(steps),
            );
            let (p0, p1) = (halo(a0), halo(a1));
            if i % 2 == 0 {
                let (x0, y0) = Dots::from_cells(p0.0, p0.1 / 2.0);
                let (x1, y1) = Dots::from_cells(p1.0, p1.1 / 2.0);
                self.dots.line(x0, y0, x1, y1, faint);
            }
        }
        let cross = Ink::new(Role::Ramp(Ramp::Starlight), 90);
        let (cx, cy) = Dots::from_cells(l2.0, l2.1 / 2.0);
        self.dots.line(cx - 2.0, cy, cx + 2.0, cy, cross);
        self.dots.line(cx, cy - 2.0, cx, cy + 2.0, cross);
        // JWST: cruise out (first quarter), then ride the halo loop.
        let cruise = ease::in_out_cubic(progress / 0.25);
        let loop_a = TAU * (progress - 0.25).max(0.0) / 0.75 * 1.5 + PI;
        let target = halo(loop_a);
        let start = (earth.0 + er * 1.6, earth.1);
        let pos = if progress < 0.25 {
            let bend = 0.12 * h * (PI * cruise).sin_det();
            (
                ease::lerp(start.0, halo(PI).0, cruise),
                ease::lerp(start.1, halo(PI).1, cruise) - bend,
            )
        } else {
            target
        };
        // Its wake.
        let trail = Ink::new(Role::Ramp(Ramp::Sulfur), 120);
        for i in 1..24 {
            let back = f64::from(i) * 0.012;
            let p = progress - back;
            if p < 0.0 {
                break;
            }
            let q = if p < 0.25 {
                let c = ease::in_out_cubic(p / 0.25);
                let bend = 0.12 * h * (PI * c).sin_det();
                (
                    ease::lerp(start.0, halo(PI).0, c),
                    ease::lerp(start.1, halo(PI).1, c) - bend,
                )
            } else {
                halo(TAU * (p - 0.25) / 0.75 * 1.5 + PI)
            };
            let (x, y) = Dots::from_cells(q.0, q.1 / 2.0);
            if i % 2 == 1 {
                self.dots.plot(x, y, trail);
            }
        }
        // The telescope: a silver kite (sunshield, facing the Sun) with a gold dot.
        let (px, py) = (pos.0.floor() as i32, pos.1.floor() as i32);
        let shield = Ink::new(Role::Ramp(Ramp::Starlight), 200);
        for (dx, dy) in [(-1, 0), (-1, -1), (-1, 1), (-2, 0)] {
            self.pixels.set(px + dx, py + dy, shield);
        }
        self.pixels
            .set(px, py, Ink::new(Role::Ramp(Ramp::Sulfur), 240));
        self.pixels
            .set(px + 1, py, Ink::new(Role::Ramp(Ramp::Sulfur), 180));
        // The Sun's glow on the left edge.
        for y in 0..ps.height() {
            let dy = (f64::from(y) + 0.5 - 0.5 * h) / (0.5 * h);
            let v = (1.0 - dy * dy).max(0.0);
            for x in 0..6u16 {
                let l = v * (1.0 - f64::from(x) / 6.0).powi(2) * 90.0;
                if l >= ink::MIN_VISIBLE_LEVEL {
                    self.pixels.set(
                        i32::from(x),
                        i32::from(y),
                        Ink::new(Role::Ramp(Ramp::Sulfur), ink::level(l)),
                    );
                }
            }
        }
    }

    fn raster(&mut self) {
        match self.timeline().map(|tl| tl.at(self.t)) {
            Some(b) if b.index == B_ORBIT => self.draw_orbit(b.progress),
            _ => self.draw_observatory(),
        }
    }
}

impl Generator for JwstUnfolding {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seeded = true;
            self.timeline = Some(Timeline::new(&BEATS));
            self.polys = Vec::with_capacity(32);
            self.t = ctx.rng.random_range(0.0..0.5);
        }
        self.pixels.resize(size);
        self.dots.resize(size);
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        self.centre = (0.5 * w, 0.6 * h);
        // Fit the 21 m shield across and ~13 m of height.
        self.scale = (0.86 * w / (21.0 * geom::ASPECT)).min(0.8 * h / 16.0);
        let ps = self.pixels.pixel_size();
        self.stars.clear();
        for k in 0..(size.area() / 45) as i32 {
            let x = (hash01(cell_key(k, 41)) * f64::from(ps.width())) as i32;
            let y = (hash01(cell_key(k, 43)) * f64::from(ps.height())) as i32;
            let l = 18 + (hash01(cell_key(k, 47)).powi(4) * 150.0) as u8;
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
    }

    fn caption(&self) -> Option<&'static str> {
        Some(match self.beat_index() {
            B_PALLETS | B_TOWER => SHIELD,
            B_LAYERS => LAYERS,
            B_SECONDARY | B_WINGS => MIRROR,
            _ => ORBIT,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> Timeline {
        Timeline::new(&BEATS)
    }

    /// First loop time at which `f` becomes positive.
    fn first(f: impl Fn(&Deploy) -> f64) -> f64 {
        let tl = timeline();
        (0..5000)
            .map(|i| f64::from(i) * 0.01)
            .find(|&t| f(&deploy(&tl, t)) > 0.0)
            .unwrap_or(f64::INFINITY)
    }

    #[test]
    fn jwst_a1_beats_run_in_the_real_order() {
        let pallets = first(|d| d.pallets);
        let tower = first(|d| d.tower);
        let layers = first(|d| d.layers[0]);
        let secondary = first(|d| d.secondary);
        let wings = first(|d| d.wings[0]);
        assert!(pallets < tower && tower < layers && layers < secondary && secondary < wings);
    }

    #[test]
    fn jwst_a2_layers_tension_one_at_a_time() {
        let tl = timeline();
        for i in 0..5000 {
            let d = deploy(&tl, f64::from(i) * 0.01);
            let moving = d.layers.iter().filter(|&&l| l > 0.0 && l < 1.0).count();
            assert!(moving <= 1, "t = {}: {:?}", f64::from(i) * 0.01, d.layers);
            // In order: a layer only starts once the one before is taut.
            for k in 1..5 {
                assert!(d.layers[k] == 0.0 || d.layers[k - 1] == 1.0);
            }
        }
    }

    #[test]
    fn jwst_a3_all_eighteen_segments_show_from_the_wings_on() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = JwstUnfolding::default();
        g.resize(size, &mut ctx);
        let wings_end: f64 = BEATS[..=B_WINGS].iter().map(|b| b.1).sum();
        for t in [wings_end - 8.0 + 0.1, wings_end - 4.0, wings_end - 0.1] {
            g.t = t;
            g.update(0.0, &mut ctx);
            let d = g.state();
            let pr = g.projector();
            let ps = g.pixels.pixel_size();
            let mut shown = 0;
            for (q, r) in segments() {
                let (pts, _) = g.segment(&d, q, r);
                let (mut sx, mut sy) = (0.0, 0.0);
                for p in pts {
                    let (x, y, _) = g.project(&pr, p).unwrap();
                    sx += x / 6.0;
                    sy += y / 6.0;
                }
                let on = sx >= 0.0
                    && sy >= 0.0
                    && sx < f64::from(ps.width())
                    && sy < f64::from(ps.height());
                let ink = g.pixels.get(sx as i32, sy as i32).unwrap_or(Ink::BLANK);
                if on && ink.role == Role::Ramp(Ramp::Sulfur) {
                    shown += 1;
                }
            }
            assert_eq!(shown, 18, "t = {t}");
        }
    }

    #[test]
    fn jwst_a4_captions_follow_the_timeline() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(1, size);
        let mut g = JwstUnfolding::default();
        g.resize(size, &mut ctx);
        let tl = timeline();
        for i in 0..500 {
            g.t = f64::from(i) * 0.1;
            let want = match tl.at(g.t).label {
                "pallets" | "tower" => SHIELD,
                "layers" => LAYERS,
                "secondary" | "wings" => MIRROR,
                _ => ORBIT,
            };
            assert_eq!(g.caption(), Some(want), "t = {}", g.t);
        }
    }

    #[test]
    fn jwst_a5_the_loop_resets_to_stowed() {
        let tl = timeline();
        let total = tl.total();
        assert_eq!(deploy(&tl, 0.0), Deploy::default());
        assert_eq!(deploy(&tl, total), Deploy::default());
        let end = deploy(&tl, total - 1e-6);
        assert_eq!(end.layers, [1.0; 5]);
        assert_eq!(end.wings, [1.0; 2]);
    }
}
