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
const FORE: [(f64, f64); 4] = [(-10.5, 0.0), (-1.6, 7.1), (1.6, 7.1), (10.5, 0.0)];
const AFT: [(f64, f64); 4] = [(-10.5, 0.0), (10.5, 0.0), (1.6, -7.1), (-1.6, -7.1)];
/// Visual spacing between tensioned layers (m; exaggerated).
const LAYER_GAP: f64 = 0.22;
/// Mirror: segment circumradius (m), the drawn share of it (the rest is the dark gap),
/// and the primary's centre height, stowed and lifted.
const SEG: f64 = 1.0;
#[cfg(test)]
const SEG_FILL: f64 = 0.84;
const MIRROR_Y: f64 = 4.6;
const TOWER_LIFT: f64 = 1.2;
/// Secondary mirror distance in front of the primary (m).
const SM_DIST: f64 = 8.0;
/// Camera: base yaw (deployed, and stowed: the folded stack in profile, JWST-A7), sway,
/// pitch, eye distance (m), and a roll so the shield reads as a tilted diamond.
const YAW: f64 = 0.14;
const YAW_STOWED: f64 = 1.1;
const ROLL: f64 = 0.08;
const SWAY: f64 = 0.12;
const PITCH: f64 = 0.4;
const EYE: f64 = 70.0;
/// Polygon cap per frame.
const MAX_POLYS: usize = 48;
/// Depth slack (m) for line work lying on a surface.
const DEPTH_EPS: f64 = 0.05;
/// The gold of the mirror (beryllium coated in gold): the top of Ember, pale gold
/// (level ~215 ≈ #E8C66A) to a near-white specular.
const GOLD: Role = Role::Sun;
/// M14 (ADR-0017): a dim gold, the glint sweeping over it the bright accent.
const GOLD_LEVEL: f64 = 200.0;
/// Seconds between glints crossing the frame.
const GLINT_PERIOD: f64 = 14.0;
/// The honeycomb's dark gaps (pixels wide) and the backplane showing through them and
/// the central hole (JWST-A7).
const GAP_PX: f64 = 1.0;
const BACKPLANE: u8 = 24;
const HOLE: u8 = 34;
/// M16 blueprint inks (GEN-R25): the frame (bus, array, tower, folded pallets) and the
/// tripod's struts in `Structure`, the fairing in `Wave`; each sunshield layer's outline
/// by its temperature, layer 5 (k = 0, facing the telescope, ≈ 40 K) `Cold` to layer 1
/// (k = 4, facing the Sun, ≈ 380 K) `Hot`; the Sun's limb in the orbit act.
const FRAME_INK: Ink = Ink::new(Role::Structure, 170);
const STRUT_INK: Ink = Ink::new(Role::Structure, 130);
const FAIRING_INK: Ink = Ink::new(Role::Wave, 150);
pub(crate) const LAYER_INKS: [Ink; 5] = [
    Ink::new(Role::Cold, 200),
    Ink::new(Role::Wave, 160),
    Ink::new(Role::Accent, 150),
    Ink::new(Role::Hot, 160),
    Ink::new(Role::Hot, 230),
];
const SUN_LIMB: Ink = Ink::new(Role::Sun, 200);

/// A level (0–255) of `role` in four flat steps (ADR-0018: M11's stepped fills).
pub(crate) fn step(role: Role, level: f64) -> Ink {
    let l = match level {
        l if l >= 185.0 => 230,
        l if l >= 120.0 => 160,
        l if l >= 60.0 => 90,
        _ => 30,
    };
    Ink::new(role, l)
}

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
#[cfg(test)]
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

/// The primary as a honeycomb panel (JWST-A7): the centre (`panel` 0, columns |q| ≤ 1)
/// or a wing (−1 / +1, q = ∓2), rasterised per pixel by mapping the pixel back into the
/// mirror plane, so every segment is a flat hex with a one-pixel dark gap.
#[derive(Debug, Clone, Copy, Default)]
struct Honey {
    /// Pixel position of the plane point `(x0, 0)`, the 2 × 2 map from plane metres to
    /// pixels and its inverse.
    origin: (f64, f64),
    x0: f64,
    fwd: [f64; 4],
    inv: [f64; 4],
    panel: i32,
    /// Level taken off while the wing is folded.
    dim: f64,
    /// Edge-on (a degenerate map): drawn as a flat strip of gold.
    flat: bool,
}

/// The segment holding plane point `(x, y)` (m): axial `(q, r)` of the flat-top grid.
fn hex_at(x: f64, y: f64) -> (i32, i32) {
    let q = 2.0 / 3.0 * x / SEG;
    let r = (-x / 3.0 + 3f64.sqrt() / 3.0 * y) / SEG;
    let s = -q - r;
    let (mut rq, mut rr, rs) = (q.round(), r.round(), s.round());
    let (dq, dr, ds) = ((rq - q).abs(), (rr - r).abs(), (rs - s).abs());
    if dq > dr && dq > ds {
        rq = -rr - rs;
    } else if dr > ds {
        rr = -rq - rs;
    }
    (rq as i32, rr as i32)
}

/// Which panel a segment belongs to: the wings are the outer columns (q = ∓2).
fn panel_of(q: i32) -> i32 {
    match q {
        -2 => -1,
        2 => 1,
        _ => 0,
    }
}

impl Honey {
    /// Ink for pixel centre `(cx, cy)`: `None` off the panel's segments.
    fn shade(&self, cx: f64, cy: f64, spec: &impl Fn(f64, f64) -> f64) -> Option<Ink> {
        let (dx, dy) = (cx - self.origin.0, cy - self.origin.1);
        let [a, b, c, d] = self.inv;
        let (mx, my) = (self.x0 + a * dx + b * dy, c * dx + d * dy);
        let (q, r) = hex_at(mx, my);
        let ring = q.abs().max(r.abs()).max((q + r).abs());
        if ring == 0 && self.panel == 0 {
            return Some(Ink::new(Role::Star, HOLE));
        }
        if !(1..=2).contains(&ring) || panel_of(q) != self.panel {
            return None;
        }
        let jitter = (hash01(cell_key(q, r)) - 0.5) * 18.0;
        let (hx, hy) = segment_centre(q, r);
        let [f0, f1, f2, f3] = self.fwd;
        let (px, py) = (
            self.origin.0 + f0 * (hx - self.x0) + f1 * hy,
            self.origin.1 + f2 * (hx - self.x0) + f3 * hy,
        );
        let level =
            (GOLD_LEVEL + jitter - self.dim + spec(px, py)).clamp(ink::MIN_VISIBLE_LEVEL, 255.0);
        if !self.flat {
            // Distance to the hex's nearest edge, in pixels: the edge function's
            // gradient in pixel space is `invᵀ n`.
            let (lx, ly) = (mx - hx, my - hy);
            let h = 3f64.sqrt() / 2.0;
            let mut worst = f64::MIN;
            let mut dist = f64::MAX;
            for (nx, ny) in [(0.0, 1.0), (h, 0.5), (h, -0.5)] {
                let e = (nx * lx + ny * ly).abs();
                let g = (a * nx + c * ny).hypot(b * nx + d * ny).max(1e-9);
                let px_dist = (SEG * h - e) / g;
                if e > worst {
                    worst = e;
                }
                dist = dist.min(px_dist);
            }
            if dist < 0.5 * GAP_PX {
                return Some(Ink::new(Role::Star, BACKPLANE));
            }
        }
        Some(step(GOLD, level))
    }
}

/// A convex polygon queued for the frame: up to 6 pixel-space vertices and their depths.
/// M16 (blueprint line art): surfaces are not filled; a polygon hides the line work
/// behind it and draws its outline in `edge` (unless blank), and a honeycomb panel
/// gilds its segments in dots.
#[derive(Debug, Clone, Copy)]
struct Poly {
    pts: [(f64, f64); 6],
    /// Each corner's depth (larger = nearer).
    z: [f64; 6],
    n: usize,
    depth: f64,
    edge: Ink,
    honey: Option<Honey>,
}

impl Default for Poly {
    fn default() -> Self {
        Self {
            pts: [(0.0, 0.0); 6],
            z: [0.0; 6],
            n: 0,
            depth: 0.0,
            edge: Ink::BLANK,
            honey: None,
        }
    }
}

/// The depth plane `z = a·x + b·y + c` (pixel coordinates) of a polygon, from three of
/// its corners; `None` for an edge-on one.
fn depth_plane(p: &Poly) -> Option<(f64, f64, f64)> {
    let (p0, p1, p2) = (p.pts[0], p.pts[1], p.pts[2]);
    let (z0, z1, z2) = (p.z[0], p.z[1], p.z[2]);
    let (ux, uy, uz) = (p1.0 - p0.0, p1.1 - p0.1, z1 - z0);
    let (vx, vy, vz) = (p2.0 - p0.0, p2.1 - p0.1, z2 - z0);
    let (nx, ny, nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    (nz.abs() > 1e-9).then(|| (-nx / nz, -ny / nz, z0 + (nx * p0.0 + ny * p0.1) / nz))
}

/// Whether pixel-space point `(cx, cy)` lies inside the convex polygon (either winding).
fn inside(p: &Poly, cx: f64, cy: f64) -> bool {
    let pts = &p.pts[..p.n];
    let (mut pos, mut neg) = (false, false);
    for i in 0..p.n {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % p.n];
        let cr = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        pos |= cr > 0.0;
        neg |= cr < 0.0;
    }
    !(pos && neg)
}

/// The polygon's bounding box clipped to `w × h` (in the polygon's units), or `None`
/// when it is off the frame or degenerate.
fn bbox(p: &Poly, w: f64, h: f64) -> Option<(f64, f64, f64, f64)> {
    if p.n < 3 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in &p.pts[..p.n] {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if !(x0.is_finite() && y1.is_finite()) || x1 < 0.0 || y1 < 0.0 || x0 > w || y0 > h {
        return None;
    }
    Some((x0.max(0.0), y0.max(0.0), x1.min(w - 1.0), y1.min(h - 1.0)))
}

/// Write a convex polygon's depth into the per-pixel z-buffer (M16: surfaces are not
/// drawn, they only hide the line work behind them). A honeycomb panel occludes only
/// where it has a segment, gap or hole.
fn occlude(zbuf: &mut [f32], size: Size, p: &Poly, spec: &impl Fn(f64, f64) -> f64) {
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    let Some((x0, y0, x1, y1)) = bbox(p, w, h) else {
        return;
    };
    let plane = depth_plane(p);
    let zw = usize::from(size.width());
    for iy in y0.floor() as i32..=y1.ceil() as i32 {
        for ix in x0.floor() as i32..=x1.ceil() as i32 {
            let (cx, cy) = (f64::from(ix) + 0.5, f64::from(iy) + 0.5);
            if ix < 0 || iy < 0 || !inside(p, cx, cy) {
                continue;
            }
            if p.honey.is_some_and(|h| h.shade(cx, cy, spec).is_none()) {
                continue;
            }
            let z = plane.map_or(p.depth, |(a, b, c)| a * cx + b * cy + c);
            if let Some(slot) = zbuf.get_mut(iy as usize * zw + ix as usize) {
                *slot = slot.max(z as f32);
            }
        }
    }
}

/// The gold of a honeycomb panel at dot resolution (M16): every dot inside one of its
/// segments, off the one-pixel gaps and the hole, and not behind a nearer surface, lit
/// in the segment's flat gold step. Braille, so the mirror is a field of gold dots with
/// black seams, not half blocks (GEN-R25's 3 % budget).
fn gild(dots: &mut Dots, zbuf: &[f32], zsize: Size, p: &Poly, spec: &impl Fn(f64, f64) -> f64) {
    let Some(honey) = p.honey else {
        return;
    };
    let ds = dots.dot_size();
    let (dw, dh) = (f64::from(ds.width()), f64::from(ds.height()));
    // The box in dots (a pixel is 2 × 2 dots).
    let mut q = *p;
    for pt in &mut q.pts[..q.n] {
        *pt = (2.0 * pt.0, 2.0 * pt.1);
    }
    let Some((x0, y0, x1, y1)) = bbox(&q, dw, dh) else {
        return;
    };
    let plane = depth_plane(p);
    let zw = usize::from(zsize.width());
    for iy in y0.floor() as i32..=y1.ceil() as i32 {
        for ix in x0.floor() as i32..=x1.ceil() as i32 {
            let (cx, cy) = ((f64::from(ix) + 0.5) / 2.0, (f64::from(iy) + 0.5) / 2.0);
            if ix < 0 || iy < 0 || !inside(p, cx, cy) {
                continue;
            }
            let Some(ink) = honey.shade(cx, cy, spec) else {
                continue;
            };
            if ink.role != GOLD {
                continue;
            }
            let z = plane.map_or(p.depth, |(a, b, c)| a * cx + b * cy + c);
            let (px, py) = (cx.floor() as usize, cy.floor() as usize);
            let hidden = zbuf
                .get(py * zw + px)
                .is_some_and(|&d| z < f64::from(d) - DEPTH_EPS);
            if !hidden {
                dots.set(ix, iy, ink);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct JwstUnfolding {
    seeded: bool,
    t: f64,
    timeline: Option<Timeline>,
    /// Pixel lattice (`w × 2h`, the old half-block grid) the camera projects onto.
    psize: (u16, u16),
    /// Per pixel: depth of the nearest surface there (−∞ for sky).
    zbuf: Vec<f32>,
    dots: Dots,
    polys: Vec<Poly>,
    /// Stars: pixel position and level.
    stars: Vec<(i32, i32, u8)>,
    /// Screen centre (cells) and rows per metre.
    centre: (f64, f64),
    scale: f64,
    /// `sin`, `cos` of the camera roll.
    roll: (f64, f64),
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl JwstUnfolding {
    fn timeline(&self) -> Option<&Timeline> {
        self.timeline.as_ref()
    }

    fn pixel_size(&self) -> Size {
        Size::new(self.psize.0, self.psize.1)
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
        // Stowed, the stack is seen in profile; the view swings round as the pallets
        // drop.
        let yaw = ease::lerp(YAW_STOWED, YAW, self.state().pallets);
        Camera {
            yaw: yaw + sway,
            pitch: PITCH,
            distance: EYE,
        }
        .projector()
    }

    /// World point → pixel position and depth (pixels are square; the view is rolled
    /// about the screen centre).
    fn project(&self, pr: &Projector, p: V3) -> Option<(f64, f64, f64)> {
        let (x, y, z) = pr.project(p, self.centre, self.scale)?;
        let (cx, cy) = (self.centre.0, 2.0 * self.centre.1);
        let (dx, dy) = (x - cx, 2.0 * y - cy);
        let (s, c) = self.roll;
        Some((cx + c * dx + s * dy, cy - s * dx + c * dy, z))
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

    /// A point of the primary's plane (m, `x` across, `y` up from its centre) in the
    /// body frame, with the tower's lift and, on a wing (`side` 0 left, 1 right), the
    /// wing's fold about its hinge.
    fn mirror_point(d: &Deploy, side: Option<usize>, x: f64, y: f64) -> V3 {
        let my = MIRROR_Y + TOWER_LIFT * d.tower;
        let Some(side) = side else {
            return [x, my + y, 0.0];
        };
        let sign = if side == 0 { -1.0 } else { 1.0 };
        let hinge = sign * 1.5 * SEG * 1.5;
        let fold = (1.0 - d.wings[side]) * 0.53 * PI;
        let (s, c) = fold.sin_cos_det();
        let dx = x - hinge;
        [hinge + dx * c, my + y, -dx * sign * s]
    }

    /// Queue one honeycomb panel of the primary (JWST-A7): its bounding rectangle in the
    /// mirror plane, gilded per dot through the plane map.
    fn push_panel(&mut self, pr: &Projector, d: &Deploy, panel: i32, spec_dim: f64) {
        let side = match panel {
            -1 => Some(0),
            1 => Some(1),
            _ => None,
        };
        let sign = f64::from(panel);
        let x0 = sign * 1.5 * SEG * 1.5;
        let (xa, xb, half_h) = if panel == 0 {
            (-2.5 * SEG, 2.5 * SEG, 2.5 * 3f64.sqrt() * SEG)
        } else {
            let (a, b) = (2.0 * SEG * sign, 4.0 * SEG * sign);
            (a.min(b), a.max(b), 1.5 * 3f64.sqrt() * SEG)
        };
        let at = |x: f64, y: f64| Self::mirror_point(d, side, x, y);
        let (Some(o), Some(ux), Some(uy)) = (
            self.project(pr, at(x0, 0.0)),
            self.project(pr, at(x0 + 1.0, 0.0)),
            self.project(pr, at(x0, 1.0)),
        ) else {
            return;
        };
        let fwd = [ux.0 - o.0, uy.0 - o.0, ux.1 - o.1, uy.1 - o.1];
        let det = fwd[0] * fwd[3] - fwd[1] * fwd[2];
        let ppm = 2.0 * self.scale;
        let flat = det.abs() < 0.3 * ppm * ppm;
        let inv = if det.abs() > 1e-12 {
            [fwd[3] / det, -fwd[1] / det, -fwd[2] / det, fwd[0] / det]
        } else {
            [0.0; 4]
        };
        let rect = [
            at(xa, -half_h),
            at(xb, -half_h),
            at(xb, half_h),
            at(xa, half_h),
        ];
        let template = Poly {
            honey: Some(Honey {
                origin: (o.0, o.1),
                x0,
                fwd,
                inv,
                panel,
                dim: spec_dim,
                flat,
            }),
            ..Poly::default()
        };
        self.push(pr, &rect, template);
    }

    /// Primary-mirror segment `(q, r)`: its six corners (body frame) and whether it is
    /// on a wing (`Some(0)` left, `Some(1)` right).
    #[cfg(test)]
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
                edge: FAIRING_INK,
                ..Poly::default()
            };
            self.push(pr, &quad, template);
        }
    }

    fn draw_observatory(&mut self) {
        let d = self.state();
        let pr = self.projector();
        self.polys.clear();
        // Spacecraft bus under the shield (Sun side): a box right under the pallets'
        // hinge line, so the stowed stack reads as one body.
        let (b0, b1, bz) = (-2.3, -0.05, 1.4);
        for face in [
            [[-1.4, b0, bz], [1.4, b0, bz], [1.4, b1, bz], [-1.4, b1, bz]],
            [
                [-1.4, b0, -bz],
                [1.4, b0, -bz],
                [1.4, b1, -bz],
                [-1.4, b1, -bz],
            ],
            [[1.4, b0, -bz], [1.4, b0, bz], [1.4, b1, bz], [1.4, b1, -bz]],
            [
                [-1.4, b0, -bz],
                [-1.4, b0, bz],
                [-1.4, b1, bz],
                [-1.4, b1, -bz],
            ],
        ] {
            let bus_poly = Poly {
                edge: FRAME_INK,
                ..Poly::default()
            };
            self.push(&pr, &face, bus_poly);
        }
        // Solar array: a thin panel hinged on the bus's side, swung out once the fairing
        // is off.
        let reach = 0.8 + 3.2 * d.fairing;
        let array = [
            [1.4, -1.55, 1.2],
            [1.4 + reach, -1.55, 1.2],
            [1.4 + reach, -1.0, 1.2],
            [1.4, -1.0, 1.2],
        ];
        let array_poly = Poly {
            edge: FRAME_INK,
            ..Poly::default()
        };
        self.push(&pr, &array, array_poly);
        // Sunshield: occluding sheets; their outlines are drawn below, coloured by layer.
        for k in 0..5 {
            for fore in [true, false] {
                let q = Self::shield_half(&d, k, fore);
                self.push(&pr, &q, Poly::default());
            }
        }
        // Primary mirror: three honeycomb panels of gold hexes, each segment a touch
        // brighter or dimmer (their coatings and tilts differ), a wing darker while still
        // folded back.
        for panel in [-1, 0, 1] {
            let dim = match panel {
                -1 => 25.0 * (1.0 - d.wings[0]),
                1 => 25.0 * (1.0 - d.wings[1]),
                _ => 0.0,
            };
            self.push_panel(&pr, &d, panel, dim);
        }
        self.fairing(&pr, d.fairing);
        // A glint sweeping slowly across the gold: a narrow diagonal band crossing the
        // frame every `GLINT_PERIOD` seconds.
        let ps = self.pixel_size();
        let (pw, ph) = (f64::from(ps.width()), f64::from(ps.height()));
        let sweep = -0.3 + 1.8 * (self.t / GLINT_PERIOD).fract();
        let spec = move |x: f64, y: f64| {
            let u = 0.7 * x / pw + 0.5 * y / ph - sweep;
            60.0 * (-(u / 0.06).powi(2)).exp()
        };
        self.zbuf.fill(f32::NEG_INFINITY);
        for p in &self.polys {
            occlude(&mut self.zbuf, ps, p, &spec);
        }
        self.dots.clear();
        for &(x, y, l) in &self.stars {
            // Stars only on open sky.
            let open = self
                .zbuf
                .get(y as usize * usize::from(ps.width()) + x as usize)
                .is_none_or(|z| *z == f32::NEG_INFINITY);
            if open {
                self.dots.set(2 * x, 2 * y, Ink::new(Role::Star, l));
            }
        }
        for i in 0..self.polys.len() {
            let p = self.polys[i];
            if p.honey.is_some() {
                gild(&mut self.dots, &self.zbuf, ps, &p, &spec);
            } else if p.edge != Ink::BLANK {
                for k in 0..p.n {
                    let (a, b) = (k, (k + 1) % p.n);
                    self.line_px(
                        (p.pts[a].0, p.pts[a].1, p.z[a]),
                        (p.pts[b].0, p.pts[b].1, p.z[b]),
                        p.edge,
                    );
                }
            }
        }
        // Shield layer outlines, coloured by each layer's temperature (layer 1 faces
        // the Sun, layer 5 the telescope); folded, plain frame lines.
        if d.fairing > 0.3 {
            for (k, &layer_ink) in LAYER_INKS.iter().enumerate() {
                let edge = if d.booms > 0.0 { layer_ink } else { FRAME_INK };
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
            // The tower, and the secondary's tripod.
            let my = MIRROR_Y + TOWER_LIFT * d.tower;
            self.line(
                &pr,
                [0.0, -0.05, 0.2],
                [0.0, my - 4.3 * SEG, -0.5],
                FRAME_INK,
            );
            let sm = Self::secondary(&d);
            for anchor in [
                [0.0, my + 4.0 * SEG, 0.2],
                [-3.5 * SEG, my - 2.1 * SEG, 0.2],
                [3.5 * SEG, my - 2.1 * SEG, 0.2],
            ] {
                self.line(&pr, anchor, sm, STRUT_INK);
            }
            if let Some((x, y, _)) = self.project(&pr, sm) {
                // The secondary: a small gold disc of dots.
                self.dots
                    .disc(2.0 * x - 0.5, 2.0 * y - 0.5, 1.2, Ink::new(GOLD, 240));
            }
        }
    }

    /// A 3-D segment in braille dots, depth-tested against the surfaces so edges and
    /// struts behind one stay hidden.
    fn line(&mut self, pr: &Projector, a: V3, b: V3, ink: Ink) {
        let (Some(pa), Some(pb)) = (self.project(pr, a), self.project(pr, b)) else {
            return;
        };
        self.line_px(pa, pb, ink);
    }

    /// [`line`](Self::line) between projected points (pixel x, y and depth).
    fn line_px(&mut self, pa: (f64, f64, f64), pb: (f64, f64, f64), ink: Ink) {
        let (ax, ay) = Dots::from_cells(pa.0, pa.1 / 2.0);
        let (bx, by) = Dots::from_cells(pb.0, pb.1 / 2.0);
        let n = ((bx - ax).abs().max((by - ay).abs()).ceil() as usize).clamp(1, 2048);
        let width = usize::from(self.psize.0);
        for i in 0..=n {
            let f = i as f64 / n as f64;
            let (x, y) = (ax + (bx - ax) * f, ay + (by - ay) * f);
            let z = pa.2 + (pb.2 - pa.2) * f;
            // A pixel is 2 × 2 dots.
            let (px, py) = ((x / 2.0).floor(), (y / 2.0).floor());
            if px < 0.0 || py < 0.0 || px >= f64::from(self.psize.0) {
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
    /// around L2 (illustrative, not a computed CR3BP orbit). All braille: the Earth a
    /// small dotted disc, lit from the Sun on the left (its night side an outline).
    fn draw_orbit(&mut self, progress: f64) {
        self.dots.clear();
        for &(x, y, l) in &self.stars {
            self.dots.set(2 * x, 2 * y, Ink::new(Role::Star, l));
        }
        // Positions in pixels, as before M16; dots are 2 per pixel each way.
        let ps = self.pixel_size();
        let (w, h) = (f64::from(ps.width()), f64::from(ps.height()));
        let d = |p: (f64, f64)| (2.0 * p.0, 2.0 * p.1);
        let earth = (0.24 * w, 0.5 * h);
        let er = 2.0 * (0.07 * h).max(1.5);
        let ec = d(earth);
        // Day side filled with dots, night side a thin limb.
        let r0 = er.ceil() as i32;
        for dy in -r0..=r0 {
            for dx in -r0..=r0 {
                let (fx, fy) = (f64::from(dx), f64::from(dy));
                let rr = fx.hypot(fy);
                if rr > er {
                    continue;
                }
                let day = fx < 0.0;
                if day || rr > er - 1.0 {
                    let level = if day { 230 } else { 120 };
                    self.dots.set(
                        ec.0 as i32 + dx,
                        ec.1 as i32 + dy,
                        Ink::new(Role::Cold, level),
                    );
                }
            }
        }
        // The Moon on its orbit.
        let (ms, mc) = (TAU * self.t / 14.0).sin_cos_det();
        let mr = 0.16 * h;
        let moon = d((earth.0 + mr * mc * 1.4, earth.1 + mr * ms * 0.6));
        self.dots
            .disc(moon.0, moon.1, 1.0, Ink::new(Role::Star, 200));
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
        let faint = Ink::new(Role::Star, 60);
        let steps = 96;
        for i in (0..steps).step_by(2) {
            let (a0, a1) = (
                TAU * f64::from(i) / f64::from(steps),
                TAU * f64::from(i + 1) / f64::from(steps),
            );
            let (p0, p1) = (d(halo(a0)), d(halo(a1)));
            self.dots.line(p0.0, p0.1, p1.0, p1.1, faint);
        }
        let cross = Ink::new(Role::Star, 90);
        let (cx, cy) = d(l2);
        self.dots.line(cx - 2.0, cy, cx + 2.0, cy, cross);
        self.dots.line(cx, cy - 2.0, cx, cy + 2.0, cross);
        // JWST: cruise out (first quarter), then ride the halo loop.
        let start = (earth.0 + 0.5 * er * 1.6, earth.1);
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
        let trail = Ink::new(GOLD, 120);
        let mut prev = d(place(progress));
        for i in 1..24 {
            let p = progress - f64::from(i) * 0.012;
            if p < 0.0 {
                break;
            }
            let q = d(place(p));
            if i % 2 == 1 {
                self.dots.line(prev.0, prev.1, q.0, q.1, trail);
            }
            prev = q;
        }
        // The telescope: a sunshield stroke facing the Sun and a gold dot.
        let pos = d(place(progress));
        self.dots.line(
            pos.0 - 3.0,
            pos.1 - 3.0,
            pos.0 - 3.0,
            pos.1 + 3.0,
            LAYER_INKS[0],
        );
        self.dots.disc(pos.0, pos.1, 1.0, Ink::new(GOLD, 240));
        // The Sun off the left edge: a thin limb arc.
        let (sc, sr) = ((-0.6 * h, h), 0.6 * h + 3.0);
        let n = (sr * 2.0).ceil() as usize;
        let mut last: Option<(f64, f64)> = None;
        for i in 0..=n {
            let a = -FRAC_PI_2 + PI * i as f64 / n as f64;
            let (s, c) = a.sin_cos_det();
            let q = (sc.0 + sr * c, sc.1 + sr * s);
            if let Some(p) = last {
                self.dots.line(p.0, p.1, q.0, q.1, SUN_LIMB);
            }
            last = Some(q);
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
        self.psize = (size.width(), size.height().saturating_mul(2));
        let ps = self.pixel_size();
        self.zbuf.clear();
        self.zbuf.resize(
            usize::from(ps.width()) * usize::from(ps.height()),
            f32::NEG_INFINITY,
        );
        self.dots.resize(size);
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        self.centre = (0.5 * w, 0.66 * h);
        // Fit the 21 m shield to ~60–65 % of the width and ~12 m of height.
        self.scale = (0.68 * w / (22.0 * geom::ASPECT)).min(0.62 * h / 11.0);
        self.roll = ROLL.sin_cos_det();
        self.stars.clear();
        for k in 0..(size.area() / 150) as i32 {
            let x = (hash01(cell_key(k, 41)) * f64::from(ps.width())) as i32;
            let y = (hash01(cell_key(k, 43)) * f64::from(ps.height())) as i32;
            let l = 18 + (hash01(cell_key(k, 47)).powi(4) * 150.0) as u8;
            self.stars.push((x, y, l));
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
        self.dots.blit(canvas);
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

    /// The canvas of `g` now.
    fn canvas(g: &JwstUnfolding) -> Canvas {
        let size = g.dots.dot_size();
        let mut c = Canvas::new(
            Size::new(size.width() / 2, size.height() / 4),
            GlyphMode::Unicode,
        );
        g.render(&mut c);
        c
    }

    /// The ink of the cell holding pixel-space point `(x, y)`.
    fn cell_at(c: &Canvas, x: f64, y: f64) -> Ink {
        let w = usize::from(c.size().width());
        let (cx, cy) = (x.floor(), (y / 2.0).floor());
        if cx < 0.0 || cy < 0.0 {
            return Ink::BLANK;
        }
        c.cells()
            .get(cy as usize * w + cx as usize)
            .map_or(Ink::BLANK, |c| c.ink)
    }

    #[test]
    fn jwst_a3_all_eighteen_segments_show_once_the_wings_are_out() {
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = JwstUnfolding::default();
        g.resize(size, &mut ctx);
        g.prepare();
        let wings_end: f64 = BEATS[..=B_WINGS].iter().map(|b| b.1).sum();
        let wings_len = BEATS[B_WINGS].1;
        // Folded, a wing hides behind the centre panel (the camera faces the mirror);
        // each wing's hexes show once it has swung out, all 18 at the end of the beat.
        for (t, want) in [
            (wings_end - wings_len + 0.1, 12),
            (wings_end - wings_len / 2.0, 15),
            (wings_end - 0.1, 18),
        ] {
            g.t = t;
            g.update(0.0, &mut ctx);
            g.prepare();
            let c = canvas(&g);
            let d = g.state();
            let pr = g.projector();
            let mut shown = 0;
            for (q, r) in segments() {
                let (pts, _) = JwstUnfolding::segment(&d, q, r);
                let (mut sx, mut sy) = (0.0, 0.0);
                for p in pts {
                    let (x, y, _) = g.project(&pr, p).unwrap();
                    sx += x / 6.0;
                    sy += y / 6.0;
                }
                if cell_at(&c, sx, sy).role == GOLD {
                    shown += 1;
                }
            }
            assert!(shown >= want, "t = {t}: {shown} of {want}");
        }
    }

    #[test]
    fn jwst_a4_captions_follow_the_timeline() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(1, size);
        let mut g = JwstUnfolding::default();
        g.resize(size, &mut ctx);
        g.prepare();
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
    fn jwst_a6_gold_mirror_over_a_shield_of_about_two_thirds_width() {
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let g = deployed(Size::new(w, h));
            let c = canvas(&g);
            let (mut gold, mut pale, mut x0, mut x1) = (0, 0, usize::MAX, 0);
            for (i, cell) in c.cells().iter().enumerate() {
                if cell.ink.role == GOLD {
                    gold += 1;
                    // Most of the gold on its upper steps (160, 230).
                    pale += usize::from(cell.ink.level >= 110);
                }
                if LAYER_INKS.contains(&cell.ink) {
                    let x = i % usize::from(w);
                    (x0, x1) = (x0.min(x), x1.max(x));
                }
            }
            assert!(
                gold > 0 && pale * 10 >= gold * 9,
                "{w}x{h}: {pale}/{gold} pale"
            );
            let share = (x1 - x0 + 1) as f64 / f64::from(w);
            assert!((0.55..0.72).contains(&share), "{w}x{h}: shield {share}");
        }
    }

    /// The deployed observatory at `size`, stars cleared.
    fn deployed(size: Size) -> JwstUnfolding {
        let mut ctx = Ctx::new(42, size);
        let mut g = JwstUnfolding::default();
        g.resize(size, &mut ctx);
        g.prepare();
        g.stars.clear();
        g.t = BEATS[..=B_WINGS].iter().map(|b| b.1).sum::<f64>() - 0.05;
        g.update(0.0, &mut ctx);
        g.prepare();
        g
    }

    #[test]
    fn jwst_a7_the_primary_is_a_gapped_honeycomb_with_a_hole_and_a_tripod() {
        let g = deployed(Size::new(120, 40));
        let d = g.state();
        let pr = g.projector();
        // Walking from each segment's centre to the next segment's centre crosses a dark
        // seam (unlit dots) between two gilded hexes.
        let mut crossings = 0;
        for (q, r) in segments() {
            for (dq, dr) in [(1, 0), (0, 1), (1, -1)] {
                let (q2, r2) = (q + dq, r + dr);
                let ring = q2.abs().max(r2.abs()).max((q2 + r2).abs());
                if !(1..=2).contains(&ring) {
                    continue;
                }
                let (a, b) = (segment_centre(q, r), segment_centre(q2, r2));
                let side = |q: i32| match panel_of(q) {
                    -1 => Some(0),
                    1 => Some(1),
                    _ => None,
                };
                let (pa, pb) = (
                    g.project(&pr, JwstUnfolding::mirror_point(&d, side(q), a.0, a.1))
                        .unwrap(),
                    g.project(&pr, JwstUnfolding::mirror_point(&d, side(q2), b.0, b.1))
                        .unwrap(),
                );
                let (mut lit, mut gap, mut after_gap) = (false, false, false);
                for i in 0..=80 {
                    let f = f64::from(i) / 80.0;
                    let (x, y) = (pa.0 + (pb.0 - pa.0) * f, pa.1 + (pb.1 - pa.1) * f);
                    let on = g.dots.is_lit((2.0 * x) as i32, (2.0 * y) as i32);
                    if on {
                        after_gap |= gap;
                        lit = true;
                    } else if lit {
                        gap = true;
                    }
                }
                if after_gap {
                    crossings += 1;
                }
            }
        }
        assert!(crossings >= 30, "gapped neighbour pairs: {crossings}");
        // The central hole is dark, and the secondary sits in front of the mirror on its
        // tripod.
        let (hx, hy, _) = g
            .project(&pr, JwstUnfolding::mirror_point(&d, None, 0.0, 0.0))
            .unwrap();
        assert!(!g.dots.is_lit((2.0 * hx) as i32, (2.0 * hy) as i32), "hole");
        let (sx, sy, _) = g.project(&pr, JwstUnfolding::secondary(&d)).unwrap();
        assert_eq!(cell_at(&canvas(&g), sx, sy).role, GOLD);
    }

    #[test]
    fn jwst_a8_blueprint_lines_with_layers_coloured_by_temperature() {
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let g = deployed(Size::new(w, h));
            let c = canvas(&g);
            let mut seen = [false; 5];
            for cell in c.cells() {
                if cell.ch == ' ' {
                    continue;
                }
                // No fills: every lit cell is a braille cell on black.
                assert!(
                    cell.bg == Ink::BLANK && ('\u{2800}'..='\u{28ff}').contains(&cell.ch),
                    "{w}x{h}: {cell:?}"
                );
                for (k, ink) in LAYER_INKS.iter().enumerate() {
                    seen[k] |= cell.ink == *ink;
                }
            }
            // The cold top layer (5) and the hot Sun-facing layer (1) both show.
            assert!(seen[0] && seen[4], "{w}x{h}: layers {seen:?}");
        }
        // Layer 5 faces the telescope (coldest), layer 1 the Sun (hottest).
        assert_eq!(LAYER_INKS[0].role, Role::Cold);
        assert_eq!(LAYER_INKS[4].role, Role::Hot);
    }

    #[test]
    fn jwst_a7_stowed_the_stack_is_one_body_seen_in_profile() {
        // At 5 s (pallets beat) the camera looks at the stack side-on and the bus joins
        // the pallets' hinge line: no gap between the bus top and the stack above.
        let size = Size::new(120, 40);
        let mut ctx = Ctx::new(42, size);
        let mut g = JwstUnfolding::default();
        g.resize(size, &mut ctx);
        g.prepare();
        g.stars.clear();
        g.t = 5.0;
        g.update(0.0, &mut ctx);
        g.prepare();
        let d = g.state();
        assert!(d.pallets < 0.3 && d.fairing == 1.0, "{d:?}");
        let pr = g.projector();
        let (bx, by, _) = g.project(&pr, [0.0, -0.3, 0.0]).unwrap();
        let (tx, ty, _) = g.project(&pr, [0.0, MIRROR_Y, 0.0]).unwrap();
        // Every pixel on the line from the bus up to the mirror's centre is solid body.
        let w = usize::from(g.psize.0);
        for i in 0..=30 {
            let f = f64::from(i) / 30.0;
            let (x, y) = (bx + (tx - bx) * f, by + (ty - by) * f);
            let z = g.zbuf[y as usize * w + x as usize];
            assert!(z > f32::NEG_INFINITY, "gap at {x:.1},{y:.1}");
        }
        let yaw = ease::lerp(YAW_STOWED, YAW, d.pallets);
        assert!(yaw > 0.9, "profile view: yaw {yaw}");
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
