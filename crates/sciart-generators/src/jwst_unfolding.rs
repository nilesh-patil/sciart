//! The James Webb Space Telescope unfolding on its way to L2 (SPEC-002 `jwst_unfolding`).
//!
//! A diorama driven by a looping `Timeline` of the real deployment sequence, compressed:
//! the fairing opens on the folded observatory, the sunshield pallets drop, the tower
//! lifts the telescope, the mid-booms stretch the shield and its five layers tension one
//! by one (the Sun-facing layer first), the secondary mirror swings out and the two
//! mirror wings fold into place. The last act cuts to a halo orbit around L2 in the
//! Sun–Earth rotating frame. The observatory is modelled in metres in a body frame (x
//! along the shield's long axis, y up from the shield towards the telescope, z out of
//! the mirror) and drawn through `geom::Camera` as shaded convex polygons on half-block
//! pixels, with edges and struts in braille dots.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use rand::Rng;
use sciart_core::geom::{self, v3::V3};
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
    captions: &[FAIRING, SHIELD, LAYERS, MIRROR, ORBIT],
};

const FAIRING: &str = "fairing opens";
const SHIELD: &str = "sunshield opens";
const LAYERS: &str = "layers tension";
const MIRROR: &str = "mirror unfolds";
const ORBIT: &str = "halo orbit at L2";

pub fn build() -> Box<dyn Generator> {
    Box::new(JwstUnfolding::default())
}

/// Deployment beats (s), in the order they happened (JWST-A1).
const BEATS: [(&str, f64); 7] = [
    ("fairing", 3.5),
    ("pallets", 5.0),
    ("tower", 3.0),
    ("layers", 10.0),
    ("secondary", 4.0),
    ("wings", 7.0),
    ("orbit", 16.0),
];
const B_FAIRING: usize = 0;
const B_PALLETS: usize = 1;
const B_TOWER: usize = 2;
const B_LAYERS: usize = 3;
const B_SECONDARY: usize = 4;
const B_WINGS: usize = 5;
const B_ORBIT: usize = 6;
/// Share of the layers beat spent extending the mid-booms, then each layer's share.
const BOOMS: f64 = 0.3;
const LAYER_SHARE: f64 = (1.0 - BOOMS) / 5.0;

/// Sunshield outline in the x–z plane (m): an elongated hexagon, 21 m × 14 m, split
/// into its fore (z ≥ 0) and aft halves, each listed round its perimeter.
const FORE: [(f64, f64); 4] = [(-10.5, 0.0), (-3.6, 7.1), (3.6, 7.1), (10.5, 0.0)];
const AFT: [(f64, f64); 4] = [(-10.5, 0.0), (10.5, 0.0), (3.6, -7.1), (-3.6, -7.1)];
/// Visual spacing between tensioned layers (m; exaggerated).
const LAYER_GAP: f64 = 0.22;
/// Mirror: segment circumradius (m), the drawn share of it (the rest is the dark gap),
/// and the primary's centre height, stowed and lifted.
const SEG: f64 = 0.76;
const SEG_FILL: f64 = 0.84;
const MIRROR_Y: f64 = 4.2;
const TOWER_LIFT: f64 = 1.2;
/// Secondary mirror distance in front of the primary (m).
const SM_DIST: f64 = 7.0;
/// Camera: base yaw, sway, pitch, eye distance (m).
const YAW: f64 = 0.38;
const SWAY: f64 = 0.22;
const PITCH: f64 = 0.17;
const EYE: f64 = 70.0;
/// Polygon cap per frame.
const MAX_POLYS: usize = 48;
/// Depth slack (m) for line work lying on a surface.
const DEPTH_EPS: f64 = 0.05;
/// The gold of the mirror (beryllium coated in gold): the top of Ember, pale gold
/// (level ~215 ≈ #E8C66A) to a near-white specular.
const GOLD: Ramp = Ramp::Ember;
const GOLD_LEVEL: f64 = 214.0;
/// Seconds between glints crossing the frame.
const GLINT_PERIOD: f64 = 14.0;

/// How far each part is deployed at loop time `t`, 0 (stowed) to 1. `layers[k]` counts
/// from the telescope side (k = 0, layer 5) to the Sun side (k = 4, layer 1).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Deploy {
    pub fairing: f64,
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
        // Layer 1 (Sun side, k = 4) first, as flown.
        let from = BOOMS + LAYER_SHARE * (4 - k) as f64;
        *l = e((lp - from) / LAYER_SHARE);
    }
    let wp = done(B_WINGS);
    Deploy {
        fairing: e(done(B_FAIRING)),
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

/// Centre of segment `(q, r)` in the mirror plane (m).
fn segment_centre(q: i32, r: i32) -> (f64, f64) {
    (
        1.5 * SEG * f64::from(q),
        3f64.sqrt() * SEG * (f64::from(r) + f64::from(q) / 2.0),
    )
}

/// How a polygon is shaded: a flat level, or the mirror's specular sweep added to it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shade {
    Flat,
    Specular,
    Sheen,
}

/// A convex polygon queued for painting: up to 6 pixel-space vertices, its depth, ramp,
/// base level and a linear level gradient in pixel x and y.
#[derive(Debug, Clone, Copy)]
struct Poly {
    pts: [(f64, f64); 6],
    /// Each corner's depth (larger = nearer).
    z: [f64; 6],
    n: usize,
    depth: f64,
    ramp: Ramp,
    level: f64,
    grad: (f64, f64),
    shade: Shade,
}

impl Default for Poly {
    fn default() -> Self {
        Self {
            pts: [(0.0, 0.0); 6],
            z: [0.0; 6],
            n: 0,
            depth: 0.0,
            ramp: Ramp::Starlight,
            level: 0.0,
            grad: (0.0, 0.0),
            shade: Shade::Flat,
        }
    }
}

impl Poly {
    fn centre(&self) -> (f64, f64) {
        let n = self.n.max(1) as f64;
        let (sx, sy) = self.pts[..self.n]
            .iter()
            .fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        (sx / n, sy / n)
    }
}

/// Fill a convex polygon (pixel coordinates) into `pixels`: every pixel whose centre is
/// inside all edges, either winding, shaded by its gradient and `spec` for specular ones.
fn fill_convex(pixels: &mut Pixels, zbuf: &mut [f32], p: &Poly, spec: &impl Fn(f64, f64) -> f64) {
    if p.n < 3 {
        return;
    }
    // The polygon's depth plane `z = a·x + b·y + c` from three of its corners, for
    // depth-testing the line work drawn over it.
    let (p0, p1, p2) = (p.pts[0], p.pts[1], p.pts[2]);
    let (z0, z1, z2) = (p.z[0], p.z[1], p.z[2]);
    let (ux, uy, uz) = (p1.0 - p0.0, p1.1 - p0.1, z1 - z0);
    let (vx, vy, vz) = (p2.0 - p0.0, p2.1 - p0.1, z2 - z0);
    let (nx, ny, nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    let plane = (nz.abs() > 1e-9).then(|| (-nx / nz, -ny / nz, z0 + (nx * p0.0 + ny * p0.1) / nz));
    let zw = usize::from(pixels.pixel_size().width());
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
    let c = p.centre();
    let (ix0, ix1) = (x0.max(0.0).floor() as i32, x1.min(w - 1.0).ceil() as i32);
    let (iy0, iy1) = (y0.max(0.0).floor() as i32, y1.min(h - 1.0).ceil() as i32);
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            let (cx, cy) = (f64::from(ix) + 0.5, f64::from(iy) + 0.5);
            let (mut pos, mut neg) = (false, false);
            for i in 0..p.n {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[(i + 1) % p.n];
                let cr = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
                pos |= cr > 0.0;
                neg |= cr < 0.0;
            }
            if pos && neg {
                continue;
            }
            let mut level = p.level + p.grad.0 * (cx - c.0) + p.grad.1 * (cy - c.1);
            match p.shade {
                Shade::Flat => {}
                Shade::Specular => level += spec(cx, cy),
                Shade::Sheen => level += 0.5 * spec(cx, cy),
            }
            let level = level.clamp(ink::MIN_VISIBLE_LEVEL, 255.0);
            pixels.set(ix, iy, ink::ramp(p.ramp, level / 255.0));
            let z = plane.map_or(p.depth, |(a, b, c)| a * cx + b * cy + c);
            if let Some(slot) = zbuf.get_mut(iy as usize * zw + ix as usize) {
                *slot = z as f32;
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
    /// Per pixel: depth of the polygon painted there (−∞ for sky).
    zbuf: Vec<f32>,
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
        let sway = SWAY * (TAU * self.t / 48.0).sin_det();
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

    fn push(&mut self, pr: &Projector, pts: &[V3], template: Poly) {
        let mut poly = template;
        poly.n = 0;
        let mut depth = 0.0;
        for &p in pts.iter().take(6) {
            let Some((x, y, z)) = self.project(pr, p) else {
                return;
            };
            poly.pts[poly.n] = (x, y);
            poly.z[poly.n] = z;
            poly.n += 1;
            depth += z;
        }
        poly.depth = depth / poly.n.max(1) as f64;
        if self.polys.len() < MAX_POLYS {
            self.polys.push(poly);
        }
    }

    /// One half (`fore`/aft) of sunshield layer `k` in body coordinates.
    fn shield_half(d: &Deploy, k: usize, fore: bool) -> [V3; 4] {
        let tension = d.layers[k];
        // Folded up against the telescope until the pallets drop.
        let fold = (1.0 - d.pallets) * FRAC_PI_2 * 0.8;
        let (s, c) = fold.sin_cos_det();
        let width = ease::lerp(0.24, 1.0, d.booms);
        let depth = ease::lerp(0.55, 1.0, d.pallets);
        let y = -LAYER_GAP * k as f64 * ease::lerp(0.15, 1.0, tension);
        let outline = if fore { FORE } else { AFT };
        let mut out = [[0.0; 3]; 4];
        for (o, &(x, z)) in out.iter_mut().zip(&outline) {
            // Slack layers billow a little.
            let sag = (1.0 - tension) * d.booms * 0.3 * (x * 0.8 + k as f64).sin_det();
            let (x, z) = (x * width, z * depth);
            *o = [x, y + sag + z.abs() * s, z * c];
        }
        out
    }

    /// Primary-mirror segment `(q, r)`: its six corners (body frame) and whether it is
    /// on a wing (`Some(0)` left, `Some(1)` right).
    fn segment(d: &Deploy, q: i32, r: i32) -> ([V3; 6], Option<usize>) {
        let (cx, cy) = segment_centre(q, r);
        let my = MIRROR_Y + TOWER_LIFT * d.tower;
        let wing = match q {
            -2 => Some(0),
            2 => Some(1),
            _ => None,
        };
        let mut out = [[0.0; 3]; 6];
        for (i, o) in out.iter_mut().enumerate() {
            let (s, c) = (PI / 3.0 * i as f64).sin_cos_det();
            let (x, y) = (cx + SEG_FILL * SEG * c, cy + SEG_FILL * SEG * s);
            // A slight parabolic dish.
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

    /// The secondary mirror: swung from folded down across the primary to its place
    /// `SM_DIST` in front of it.
    fn secondary(d: &Deploy) -> V3 {
        let my = MIRROR_Y + TOWER_LIFT * d.tower;
        let angle = (1.0 - d.secondary) * 1.35;
        let (s, c) = angle.sin_cos_det();
        // The boom pivots at the primary's lower edge: folded, the secondary lies low
        // across the mirror's face; deployed, it stands `SM_DIST` out on the axis.
        [0.0, my - 2.6 * s, 0.3 + SM_DIST * c]
    }

    /// Queue the fairing halves: shut around the folded observatory, then split and
    /// drift away.
    fn fairing(&mut self, pr: &Projector, open: f64) {
        if open >= 1.0 {
            return;
        }
        let spread = 12.0 * open * open;
        let lean = 0.35 * open;
        for side in [-1.0, 1.0] {
            // Half an ogive in outline: straight sides and a nose, leaning outward
            // about its base as it drifts away.
            let pts = [
                [0.0, -3.0],
                [3.4, -3.0],
                [3.4, 8.5],
                [1.6, 11.5],
                [0.0, 12.2],
            ];
            let mut quad = [[0.0; 3]; 5];
            for (o, &[u, v]) in quad.iter_mut().zip(&pts) {
                o[0] = side * (u + spread + (v + 3.0) * lean);
                o[1] = v - spread * 0.2;
                o[2] = 3.2 + spread * 0.4;
            }
            let template = Poly {
                ramp: Ramp::Starlight,
                level: 150.0,
                grad: (-side * 6.0, -0.4),
                ..Poly::default()
            };
            self.push(pr, &quad, template);
        }
    }

    fn draw_observatory(&mut self) {
        let d = self.state();
        let pr = self.projector();
        self.polys.clear();
        // Spacecraft bus under the shield (Sun side).
        let bus = [
            [-1.4, -2.2, 1.4],
            [1.4, -2.2, 1.4],
            [1.4, -0.9, 1.4],
            [-1.4, -0.9, 1.4],
        ];
        let bus_poly = Poly {
            ramp: Ramp::Starlight,
            level: 70.0,
            ..Poly::default()
        };
        self.push(&pr, &bus, bus_poly);
        // Solar array: a thin panel hinged on the bus's side, swung out once the fairing
        // is off.
        let reach = 0.8 + 3.2 * d.fairing;
        let array = [
            [1.4, -1.75, 1.3],
            [1.4 + reach, -1.75, 1.3],
            [1.4 + reach, -1.4, 1.3],
            [1.4, -1.4, 1.3],
        ];
        let array_poly = Poly {
            ramp: Ramp::Cobalt,
            level: 120.0,
            grad: (1.2, 0.0),
            ..Poly::default()
        };
        self.push(&pr, &array, array_poly);
        // Sunshield. Stowed, the folded pallets are grey-silver slabs; once the booms
        // pull the membranes out, the top layer (layer 5, which the camera sees) is a
        // silver-pink Kapton sheet with a slow sheen, and the four layers under it show
        // only as violet slivers of shadowed underside along the near edge.
        let membrane = d.booms > 0.0;
        for k in 0..5 {
            let tension = d.layers[k];
            let (ramp, base) = if !membrane {
                (Ramp::Starlight, 132.0 - 8.0 * k as f64)
            } else if k == 0 {
                (Ramp::Eosin, ease::lerp(178.0, 214.0, tension))
            } else {
                (
                    Ramp::Plasma,
                    ease::lerp(70.0, 108.0, tension) - 6.0 * k as f64,
                )
            };
            for fore in [true, false] {
                let q = Self::shield_half(&d, k, fore);
                let template = Poly {
                    ramp,
                    level: base,
                    grad: (0.35, -0.6),
                    shade: if k == 0 && membrane {
                        Shade::Sheen
                    } else {
                        Shade::Flat
                    },
                    ..Poly::default()
                };
                let before = self.polys.len();
                self.push(&pr, &q, template);
                // Seen from above, a lower layer is always behind the one over it; the
                // mean-depth sort alone can put a near half of one before a far half of
                // the next and punch notches into the top sheet.
                if membrane && self.polys.len() > before {
                    self.polys[before].depth -= 0.8 * k as f64;
                }
            }
        }
        // Primary mirror segments: pale gold, each a touch brighter or dimmer (their
        // coatings and tilts differ), darker while a wing is still folded back.
        for (q, r) in segments() {
            let (pts, wing) = Self::segment(&d, q, r);
            let fold = wing.map_or(0.0, |s| 1.0 - d.wings[s]);
            let jitter = (hash01(cell_key(q, r)) - 0.5) * 18.0;
            let template = Poly {
                ramp: GOLD,
                level: GOLD_LEVEL + jitter - 55.0 * fold,
                grad: (-0.25, -0.4),
                shade: Shade::Specular,
                ..Poly::default()
            };
            self.push(&pr, &pts, template);
        }
        self.fairing(&pr, d.fairing);
        // Painter's order: far first.
        self.polys
            .sort_unstable_by(|a, b| a.depth.total_cmp(&b.depth));
        self.pixels.clear();
        self.zbuf.fill(f32::NEG_INFINITY);
        for &(x, y, l) in &self.stars {
            self.pixels
                .set(x, y, Ink::new(Role::Ramp(Ramp::Starlight), l));
        }
        // A glint sweeping slowly across the gold (and a sheen across the Kapton): a
        // narrow diagonal band crossing the frame every `GLINT_PERIOD` seconds.
        let ps = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()));
        let sweep = -0.3 + 1.8 * (self.t / GLINT_PERIOD).fract();
        let spec = move |x: f64, y: f64| {
            let u = 0.7 * x / pw + 0.5 * y / ph - sweep;
            60.0 * (-(u / 0.06).powi(2)).exp()
        };
        for p in &self.polys {
            fill_convex(&mut self.pixels, &mut self.zbuf, p, &spec);
        }
        // Line work: shield layer edges (hidden behind the gold), tower, struts.
        self.dots.clear();
        if d.fairing > 0.3 {
            for k in 0..5 {
                let edge = if d.booms > 0.0 {
                    ink::ramp(Ramp::Plasma, 0.62 + 0.3 * d.layers[k])
                } else {
                    ink::ramp(Ramp::Starlight, 0.8)
                };
                for fore in [true, false] {
                    let q = Self::shield_half(&d, k, fore);
                    for i in 0..4 {
                        // The fold line between the two halves is not an edge.
                        let hinge = if fore { 3 } else { 0 };
                        if i == hinge && d.pallets > 0.5 {
                            continue;
                        }
                        self.line(&pr, q[i], q[(i + 1) % 4], edge);
                    }
                }
            }
            let strut = Ink::new(Role::Ramp(Ramp::Starlight), 235);
            let my = MIRROR_Y + TOWER_LIFT * d.tower;
            self.line(&pr, [0.0, -0.9, 0.2], [0.0, my - 2.9, -0.5], strut);
            let sm = Self::secondary(&d);
            for anchor in [
                [0.0, my + 3.1, 0.2],
                [-2.7, my - 1.6, 0.2],
                [2.7, my - 1.6, 0.2],
            ] {
                self.line(&pr, anchor, sm, strut);
            }
            if let Some((x, y, _)) = self.project(&pr, sm) {
                let pixels = &mut self.pixels;
                disc_pixels((x, y), 1.1, |ix, iy, cover| {
                    if cover > 0.2 {
                        pixels.set(ix, iy, Ink::new(Role::Ramp(GOLD), 240));
                    }
                });
            }
        }
    }

    /// A 3-D segment in braille dots, depth-tested against the painted polygons so
    /// edges and struts behind a surface stay hidden.
    fn line(&mut self, pr: &Projector, a: V3, b: V3, ink: Ink) {
        let (Some(pa), Some(pb)) = (self.project(pr, a), self.project(pr, b)) else {
            return;
        };
        let (ax, ay) = Dots::from_cells(pa.0, pa.1 / 2.0);
        let (bx, by) = Dots::from_cells(pb.0, pb.1 / 2.0);
        let n = ((bx - ax).abs().max((by - ay).abs()).ceil() as usize).clamp(1, 2048);
        let width = usize::from(self.pixels.pixel_size().width());
        for i in 0..=n {
            let f = i as f64 / n as f64;
            let (x, y) = (ax + (bx - ax) * f, ay + (by - ay) * f);
            let z = pa.2 + (pb.2 - pa.2) * f;
            // A pixel is 2 × 2 dots.
            let (px, py) = ((x / 2.0).floor(), (y / 2.0).floor());
            if px < 0.0 || py < 0.0 {
                continue;
            }
            let covered = self
                .zbuf
                .get(py as usize * width + px as usize)
                .is_some_and(|&d| z < f64::from(d) - DEPTH_EPS);
            if !covered {
                self.dots.plot(x, y, ink);
            }
        }
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
        let (ms, mc) = (TAU * self.t / 14.0).sin_cos_det();
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
        for i in (0..steps).step_by(2) {
            let (a0, a1) = (
                TAU * f64::from(i) / f64::from(steps),
                TAU * f64::from(i + 1) / f64::from(steps),
            );
            let (p0, p1) = (halo(a0), halo(a1));
            let (x0, y0) = Dots::from_cells(p0.0, p0.1 / 2.0);
            let (x1, y1) = Dots::from_cells(p1.0, p1.1 / 2.0);
            self.dots.line(x0, y0, x1, y1, faint);
        }
        let cross = Ink::new(Role::Ramp(Ramp::Starlight), 90);
        let (cx, cy) = Dots::from_cells(l2.0, l2.1 / 2.0);
        self.dots.line(cx - 2.0, cy, cx + 2.0, cy, cross);
        self.dots.line(cx, cy - 2.0, cx, cy + 2.0, cross);
        // JWST: cruise out (first quarter), then ride the halo loop.
        let start = (earth.0 + er * 1.6, earth.1);
        let place = |p: f64| {
            if p < 0.25 {
                let c = ease::in_out_cubic(p / 0.25);
                let bend = 0.12 * h * (PI * c).sin_det();
                (
                    ease::lerp(start.0, halo(PI).0, c),
                    ease::lerp(start.1, halo(PI).1, c) - bend,
                )
            } else {
                halo(TAU * (p - 0.25) / 0.75 * 1.5 + PI)
            }
        };
        let pos = place(progress);
        let trail = Ink::new(Role::Ramp(GOLD), 120);
        for i in (1..24).step_by(2) {
            let p = progress - f64::from(i) * 0.012;
            if p < 0.0 {
                break;
            }
            let q = place(p);
            let (x, y) = Dots::from_cells(q.0, q.1 / 2.0);
            self.dots.plot(x, y, trail);
        }
        // The telescope: a lavender kite (sunshield, facing the Sun) with a gold dot.
        let (px, py) = (pos.0.floor() as i32, pos.1.floor() as i32);
        let shield = Ink::new(Role::Ramp(Ramp::Plasma), 200);
        for (dx, dy) in [(-1, 0), (-1, -1), (-1, 1), (-2, 0)] {
            self.pixels.set(px + dx, py + dy, shield);
        }
        self.pixels.set(px, py, Ink::new(Role::Ramp(GOLD), 240));
        self.pixels.set(px + 1, py, Ink::new(Role::Ramp(GOLD), 180));
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
            self.polys = Vec::with_capacity(MAX_POLYS);
            self.t = ctx.rng.random_range(0.0..0.3);
        }
        self.pixels.resize(size);
        let ps = self.pixels.pixel_size();
        self.zbuf.clear();
        self.zbuf.resize(
            usize::from(ps.width()) * usize::from(ps.height()),
            f32::NEG_INFINITY,
        );
        self.dots.resize(size);
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        self.centre = (0.5 * w, 0.64 * h);
        // Fit the 21 m shield to ~60–65 % of the width and ~12 m of height.
        self.scale = (0.68 * w / (22.0 * geom::ASPECT)).min(0.8 * h / 10.5);
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
            B_FAIRING => FAIRING,
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
        let fairing = first(|d| d.fairing);
        let pallets = first(|d| d.pallets);
        let tower = first(|d| d.tower);
        let booms = first(|d| d.booms);
        let layers = first(|d| d.layers[4]);
        let secondary = first(|d| d.secondary);
        let wings = first(|d| d.wings[0]);
        let order = [fairing, pallets, tower, booms, layers, secondary, wings];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    }

    #[test]
    fn jwst_a2_layers_tension_one_at_a_time_sun_side_first() {
        let tl = timeline();
        for i in 0..5000 {
            let d = deploy(&tl, f64::from(i) * 0.01);
            let moving = d.layers.iter().filter(|&&l| l > 0.0 && l < 1.0).count();
            assert!(moving <= 1, "t = {}: {:?}", f64::from(i) * 0.01, d.layers);
            // Layer 1 (k = 4) first: a layer only starts once the one Sunward is taut.
            for k in 0..4 {
                assert!(d.layers[k] == 0.0 || d.layers[k + 1] == 1.0);
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
        let wings_len = BEATS[B_WINGS].1;
        for t in [
            wings_end - wings_len + 0.1,
            wings_end - wings_len / 2.0,
            wings_end - 0.1,
        ] {
            g.t = t;
            g.update(0.0, &mut ctx);
            let d = g.state();
            let pr = g.projector();
            let ps = g.pixels.pixel_size();
            let mut shown = 0;
            for (q, r) in segments() {
                let (pts, _) = JwstUnfolding::segment(&d, q, r);
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
                if on && ink.role == Role::Ramp(GOLD) {
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
                "fairing" => FAIRING,
                "pallets" | "tower" => SHIELD,
                "layers" => LAYERS,
                "secondary" | "wings" => MIRROR,
                _ => ORBIT,
            };
            assert_eq!(g.caption(), Some(want), "t = {}", g.t);
        }
    }

    #[test]
    fn jwst_a6_pale_gold_mirror_over_a_shield_of_about_two_thirds_width() {
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let size = Size::new(w, h);
            let mut ctx = Ctx::new(42, size);
            let mut g = JwstUnfolding::default();
            g.resize(size, &mut ctx);
            // The end of the wings beat: fully deployed.
            g.t = BEATS[..=B_WINGS].iter().map(|b| b.1).sum::<f64>() - 0.05;
            g.update(0.0, &mut ctx);
            let ps = g.pixels.pixel_size();
            let (mut gold, mut pale, mut x0, mut x1) = (0, 0, i32::MAX, i32::MIN);
            for y in 0..i32::from(ps.height()) {
                for x in 0..i32::from(ps.width()) {
                    let ink = g.pixels.get(x, y).unwrap_or(Ink::BLANK);
                    if ink.role == Role::Ramp(GOLD) {
                        gold += 1;
                        pale += usize::from(ink.level >= 190);
                    }
                    if matches!(ink.role, Role::Ramp(Ramp::Eosin | Ramp::Plasma)) {
                        (x0, x1) = (x0.min(x), x1.max(x));
                    }
                }
            }
            assert!(
                gold > 0 && pale * 10 >= gold * 9,
                "{w}x{h}: {pale}/{gold} pale"
            );
            let share = f64::from(x1 - x0 + 1) / f64::from(ps.width());
            assert!((0.55..0.72).contains(&share), "{w}x{h}: shield {share}");
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
