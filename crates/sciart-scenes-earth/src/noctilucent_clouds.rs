//! Late in a northern summer night, around midnight (SPEC-002 `noctilucent_clouds`): the
//! land and the low clouds are dark, but high in the twilight arch electric silver-cyan
//! clouds glow, rippled with fine waves, still in sunlight at 83 km.
//!
//! The Sun sits `d` degrees below the northern horizon. A point of the 83 km ice layer is
//! lit when its ray toward the Sun clears the Earth plus a 20 km screening layer (ozone
//! and haze); tropospheric clouds (≤ 12 km) never clear it. Overhead the layer is in the
//! Earth's shadow, so the clouds glow only in a band above the northern horizon, which
//! sinks as the Sun sinks (NLC-A1, NLC-A2). The ice is a sum of drifting gravity-wave
//! packets (long "bands" and short "billows") in a patchy envelope, evaluated on a capped
//! lattice (ADR-0012) with per-node sin/cos tables, then sampled to pixels.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::grid;
use sciart_core::ink;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::timeline::ease;

use sciart_shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "noctilucent_clouds",
    category: Category::EarthSky,
    description: "Electric-blue ice clouds at 83 km, still sunlit long after sunset",
    title: "Noctilucent clouds · sunlit ice",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(Noctilucent::default())
}

/// Earth radius, ice layer height and the screening height a sunbeam must clear (km).
const EARTH_R: f64 = 6371.0;
const LAYER_KM: f64 = 83.0;
const SCREEN_KM: f64 = 20.0;
/// Soft edge of the lit region (km of clearance).
const EDGE_KM: f64 = 4.0;
/// Solar depression (deg), the NLC season's viewing window (the lit band's top falls from
/// ~28° to ~8°): it deepens toward midnight and recovers toward dawn, as the
/// midnight Sun skims below the northern horizon; seconds each way.
const DEPRESSION: (f64, f64) = (8.2, 9.8);
const HALF_NIGHT: f64 = 40.0;
/// The Sun's azimuth swings from north-west to north-east through the night (deg).
const SUN_AZ: f64 = 14.0;
/// Framing: horizon row (share of the height) and the sky shown above it (deg).
const HORIZON_ROW: f64 = 0.9;
const SKY_SPAN: f64 = 32.0;
/// Lattice cap for the ice field (ADR-0012).
const MAX_NODES: usize = 8500;
/// Envelope wander: amplitude (s of drift) and period (s).
const SWING: (f64, f64) = (12.0, 70.0);
/// Wave packets: count and time-lapse (phase speed km/s on screen).
const PACKETS: usize = 6;
const PHASE_SPEED: f64 = 3.0;
/// The ice is braille dots whose density follows it (full from `u ≥ CREST`), in two flat
/// electric-blue steps (M11's `Wave`, ADR-0018): dim where the sheet is thin, bright on
/// the dense crests (dot density from `ICE_CREST` of 255), so the sheet thins into black
/// and the crests read as bright streaks. Below `ICE_FLOOR` the sky stays black.
const ICE: [Ink; 2] = [Ink::new(Role::Wave, 90), Ink::new(Role::Wave, 220)];
const ICE_CREST: u8 = 110;
const ICE_FLOOR: f64 = 0.08;
const CREST: f64 = 0.45;
/// The field is stretched so its brightest node reaches this (at most ×`GAIN_MAX`): the
/// clouds keep an area in every act, however low the lit band sinks.
const PEAK: f64 = 0.6;
const GAIN_MAX: f64 = 2.5;
/// A billow's trough keeps this share of its crest's ice.
const BILLOW_FLOOR: f64 = 0.35;
/// The ice fades out over these elevations (deg) above the horizon, before the warm glow.
const ICE_LOW: (f64, f64) = (2.8, 3.6);
/// The glow's top edge: its slow wave (± deg, over this many pixels) and fine fray (deg).
const GLOW_WAVE: (f64, f64) = (0.9, 7.0);
/// The warm horizon glow: a few pixels of light over the northern horizon, e-folding over
/// `GLOW_PX` pixels, strongest under the Sun, cut to black below `GLOW_CUT` of its peak,
/// in three flat `Hot` steps ([`glow_ink`]).
const GLOW_PX: f64 = 1.6;
const GLOW_CUT: f64 = 0.12;

/// The twilight glow's strength `g` (0–1) in flat orange steps (ADR-0018).
fn glow_ink(g: f64) -> Ink {
    match g {
        g if g >= 0.55 => Ink::new(Role::Hot, 200),
        g if g >= 0.28 => Ink::new(Role::Hot, 90),
        g if g >= GLOW_CUT => Ink::new(Role::Hot, 15),
        _ => Ink::BLANK,
    }
}
/// Stars are drowned by sky or ice brighter than this level.
const STAR_DROWN: u8 = 55;
/// ASCII: ice levels from here up are spread over the glyph ramp.
const ASCII_ICE: f64 = 140.0;
/// How much of the glow a low-cloud wisp hides at its thickest.
const BANK_DARK: f64 = 0.55;
/// Low boreal hills with sparse spruce: the northern horizon's glow stays in view.
const LAND: Terrain = Terrain {
    ridge: (0.25, 0.9),
    trees: Some((0.5, 1.9)),
    gap: (0.4, 2.0),
    clearing: 0.35,
};

/// Clearance (km) of the sunbeam from a point `h` km up at elevation `el` and azimuth `az`
/// (rad) above `R + SCREEN_KM`, with the Sun `d` below the horizon at azimuth `sun_az`:
/// positive means lit (NLC-A1).
#[cfg(test)]
pub(crate) fn clearance(h: f64, el: f64, az: f64, d: f64, sun_az: f64) -> f64 {
    clearance_from(layer_point(h, el, az), sun_dir(d, sun_az))
}

/// The point `h` km up seen at elevation `el`, azimuth `az` (km, Earth's centre at the
/// origin, the observer at `(0, 0, R)`).
fn layer_point(h: f64, el: f64, az: f64) -> [f64; 3] {
    let (se, ce) = el.sin_cos_det();
    let (sa, ca) = az.sin_cos_det();
    let u = [sa * ce, ca * ce, se];
    let (r, rh) = (EARTH_R, EARTH_R + h);
    let len = -r * se + (r * r * se * se + rh * rh - r * r).max(0.0).sqrt();
    [len * u[0], len * u[1], r + len * u[2]]
}

/// Unit vector toward the Sun, `d` below the horizon at azimuth `sun_az`.
fn sun_dir(d: f64, sun_az: f64) -> [f64; 3] {
    let (ss, cs) = sun_az.sin_cos_det();
    let (sd, cd) = d.sin_cos_det();
    [ss * cd, cs * cd, -sd]
}

/// [`clearance`] from a precomputed layer point and Sun direction.
fn clearance_from(p: [f64; 3], s: [f64; 3]) -> f64 {
    let t = p[0] * s[0] + p[1] * s[1] + p[2] * s[2];
    let p2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
    let closest = if t >= 0.0 {
        p2.sqrt()
    } else {
        (p2 - t * t).max(0.0).sqrt()
    };
    closest - (EARTH_R + SCREEN_KM)
}

/// Where on the layer (km east, km north of the observer) the line of sight meets it.
fn layer_xy(el: f64, az: f64) -> (f64, f64) {
    let (se, ce) = el.sin_cos_det();
    let (sa, ca) = az.sin_cos_det();
    let (r, rh) = (EARTH_R, EARTH_R + LAYER_KM);
    let len = -r * se + (r * r * se * se + rh * rh - r * r).max(0.0).sqrt();
    (len * sa * ce, len * ca * ce)
}

/// Solar depression (rad) and azimuth at time `t`.
pub(crate) fn sun_at(t: f64) -> (f64, f64) {
    let p = (t / (2.0 * HALF_NIGHT)).rem_euclid(1.0);
    let deep = 1.0 - (1.0 - 2.0 * p).abs();
    let d = DEPRESSION.0 + (DEPRESSION.1 - DEPRESSION.0) * ease::smoothstep(deep);
    let az = SUN_AZ * (2.0 * p - 1.0);
    (d.to_radians(), az.to_radians())
}

#[derive(Debug, Clone, Copy, Default)]
struct Packet {
    /// Wave vector (rad/km), angular frequency, envelope centre, velocity and radius (km).
    k: (f64, f64),
    omega: f64,
    centre: (f64, f64),
    drift: (f64, f64),
    radius: f64,
    amp: f64,
    /// Billows are sharp-crested; bands are smooth.
    billow: bool,
}

/// One lattice node's fixed data: layer position, view angles and per-packet phase.
#[derive(Debug, Clone, Copy, Default)]
struct Node {
    xy: (f32, f32),
    el: f32,
    /// Read by the NLC-A3 test (the lit test from the view angles).
    #[cfg_attr(not(test), allow(dead_code))]
    az: f32,
    /// Its point on the ice layer (km from Earth's centre), `(cos az, sin az)`, and the
    /// slant-path factor: fixed per node, so a step costs no trig here.
    p: [f64; 3],
    azc: (f32, f32),
    slant: f32,
    /// `(cos k·x, sin k·x)` per packet.
    phase: [(f32, f32); PACKETS],
    /// Per packet, how well the lattice resolves its waves here (0..1): far off, near the
    /// horizon, a short wave spans too few nodes and would alias into stripes, so it
    /// fades out.
    resolve: [u8; PACKETS],
}

#[derive(Debug, Default)]
pub struct Noctilucent {
    packets: [Packet; PACKETS],
    noise_seed: u64,
    nodes: Vec<Node>,
    field: Grid<f32>,
    map: SampleMap,
    /// Pixel geometry: horizon row, px per radian, centre column.
    horizon: f64,
    f: f64,
    cx: f64,
    /// Per pixel: land and low-cloud cover (0..255 for 0..1; a byte keeps the footprint small).
    cover: Vec<u8>,
    /// Per row and column this step: twilight arch factors.
    rows: Vec<(f32, f32)>,
    cols: Vec<f32>,
    /// Per picture pixel: the faint ice's dot density this frame (0 = none).
    faint: Vec<u8>,
    /// Per pixel column: the land's top (pixel row), where the glow sits.
    land_top: Vec<f32>,
    /// Per pixel column: the warm glow's uneven top (deg above or below `BANDS.0`).
    edge: Vec<f32>,
    picture: Pixels,
    stars: Dots,
    /// Star positions (dots) and levels; replotted each step where the sky is dark enough.
    star_list: Vec<(f64, f64, u8)>,
    t: f64,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

/// Smooth value noise in `[0, 1]` on a 90 km lattice.
fn value_noise(seed: u64, x: f64, y: f64) -> f64 {
    let (gx, gy) = (x / 90.0, y / 90.0);
    let (ix, iy) = (gx.floor(), gy.floor());
    let (fx, fy) = (ease::smoothstep(gx - ix), ease::smoothstep(gy - iy));
    let at = |dx: f64, dy: f64| hash01(seed ^ cell_key((ix + dx) as i32, (iy + dy) as i32));
    let top = ease::lerp(at(0.0, 0.0), at(1.0, 0.0), fx);
    let bottom = ease::lerp(at(0.0, 1.0), at(1.0, 1.0), fx);
    ease::lerp(top, bottom, fy)
}

impl Noctilucent {
    fn seed(&mut self, rng: &mut impl Rng) {
        self.noise_seed = rng.random();
        self.packets = std::array::from_fn(|i| {
            let billow = i >= 2;
            let lambda = if billow {
                rng.random_range(9.0..16.0)
            } else {
                rng.random_range(35.0..55.0)
            };
            // Bands run roughly across the line of sight; billows at any angle.
            let dir = if billow {
                rng.random_range(0.0..TAU)
            } else {
                rng.random_range(-0.5..0.5) + PI / 2.0
            };
            let (s, c) = dir.sin_cos_det();
            let kmag = TAU / lambda;
            let drift_dir = rng.random_range(0.0..TAU);
            let (ds, dc) = drift_dir.sin_cos_det();
            Packet {
                k: (kmag * c, kmag * s),
                omega: kmag * PHASE_SPEED,
                centre: (
                    rng.random_range(-450.0..450.0),
                    rng.random_range(250.0..900.0),
                ),
                drift: (14.0 * dc, 14.0 * ds),
                radius: if billow {
                    rng.random_range(140.0..260.0)
                } else {
                    rng.random_range(250.0..450.0)
                },
                amp: if billow { 1.1 } else { 0.8 },
                billow,
            }
        });
    }

    /// Recompute the ice brightness on the lattice for the Sun and waves at `t`.
    fn step_field(&mut self) {
        let (d, sun_az) = sun_at(self.t);
        let t = self.t;
        let rot: [(f64, f64); PACKETS] =
            std::array::from_fn(|k| (self.packets[k].omega * t).sin_cos_det());
        let packets = &self.packets;
        // Envelopes wander to and fro about their centres rather than drifting away, so
        // the field fills the sky through the whole loop.
        let centres: [(f64, f64); PACKETS] = std::array::from_fn(|k| {
            let p = packets[k];
            let swing = (TAU * t / SWING.1 + p.omega * 7.0).sin_det() * SWING.0;
            (
                p.centre.0 + p.drift.0 * swing,
                p.centre.1 + p.drift.1 * swing,
            )
        });
        let sun = sun_dir(d, sun_az);
        let (sun_s, sun_c) = sun_az.sin_cos_det();
        let seed = self.noise_seed;
        for (cell, n) in self.field.as_mut_slice().iter_mut().zip(&self.nodes) {
            if n.el <= 0.0 {
                *cell = 0.0;
                continue;
            }
            let lit = ease::smoothstep((clearance_from(n.p, sun) + 0.5 * EDGE_KM) / EDGE_KM);
            if lit <= 0.0 {
                *cell = 0.0;
                continue;
            }
            let (x, y) = (f64::from(n.xy.0), f64::from(n.xy.1));
            let mut ice = 0.0;
            for (k, p) in packets.iter().enumerate() {
                let c = centres[k];
                let r2 = ((x - c.0).powi(2) + (y - c.1).powi(2)) / (p.radius * p.radius);
                let env = 1.0 / (1.0 + r2 * r2);
                // cos(k·x − ωt) from the node's table.
                let (ck, sk) = (f64::from(n.phase[k].0), f64::from(n.phase[k].1));
                let (sw, cw) = rot[k];
                let wave = ck * cw + sk * sw;
                // Billows: soft crests on a sheet of ice rather than thin bright lines on
                // dark gaps (which read as ripples on water).
                let crest = if p.billow {
                    BILLOW_FLOOR + (1.0 - BILLOW_FLOOR) * (1.0 - wave.abs()).powf(1.5)
                } else {
                    0.5 + 0.5 * wave
                };
                ice += p.amp * env * crest * f64::from(n.resolve[k]) / 255.0;
            }
            let patch = value_noise(seed, x - 3.0 * t, y + 1.5 * t);
            let patch = ease::smoothstep((patch - 0.3) / 0.45);
            // Forward scattering: brightest toward the Sun; long slant paths near the horizon.
            let cos_rel = f64::from(n.azc.0) * sun_c + f64::from(n.azc.1) * sun_s;
            let toward = 0.55 + 0.45 * cos_rel;
            let slant = f64::from(n.slant);
            *cell = (ice.min(1.6) / 1.6 * patch * lit * toward * (0.45 + 0.55 * slant)) as f32;
        }
    }

    fn paint(&mut self) {
        let (d, sun_az) = sun_at(self.t);
        let dim = 1.0 - (d.to_degrees() - DEPRESSION.0) / (DEPRESSION.1 - DEPRESSION.0);
        // Per row: elevation (deg); per column: how far toward the Sun (1 = under it).
        let (f, horizon, cx) = (self.f, self.horizon, self.cx);
        for (y, row) in self.rows.iter_mut().enumerate() {
            let el = (horizon - (y as f64 + 0.5)) / f;
            *row = (el.max(0.0).to_degrees() as f32, 0.0);
        }
        for (x, col) in self.cols.iter_mut().enumerate() {
            let az = (x as f64 + 0.5 - cx) / f;
            *col = (0.5 + 0.5 * (az - sun_az).cos_det()).powi(2) as f32;
        }
        let w = usize::from(self.picture.pixel_size().width());
        let cols_edge = &self.edge;
        let peak = self.field.as_slice().iter().fold(0.0f32, |m, &v| m.max(v));
        let gain = (PEAK / f64::from(peak).max(1e-6)).clamp(1.0, GAIN_MAX);
        let (field, map, cover, rows, cols, faint, land_top) = (
            &self.field,
            &self.map,
            &self.cover,
            &self.rows,
            &self.cols,
            &mut self.faint,
            &self.land_top,
        );
        faint.fill(0);
        let mut row = (-1, map.row_cubic(field, 0));
        let px_per_deg = f * PI / 180.0;
        self.picture.shade(|x, y| {
            let i = y as usize * w + x as usize;
            let open = 1.0 - f64::from(cover.get(i).copied().unwrap_or(255)) / 255.0;
            if open <= 0.02 {
                return Ink::BLANK;
            }
            let e = f64::from(rows.get(y as usize).copied().unwrap_or((0.0, 0.0)).0);
            let side = f64::from(cols.get(x as usize).copied().unwrap_or(0.0));
            // Against a darker sky the ice stands out more: its contrast grows as the Sun
            // sinks (stylised: keeps the thin late band as bright as the early sheet).
            // Low down the ice's light crosses so much air that it is dimmed and reddened
            // into the glow: it fades out over the lowest degrees. Catmull-Rom from the
            // lattice (CORE-R102), and black below the floor, so the clouds are filaments.
            let ice =
                gain * f64::from({
                    // One row of Catmull-Rom taps per pixel row (CORE-R102).
                    if y != row.0 {
                        row = (y, map.row_cubic(field, y));
                    }
                    row.1.at(x)
                }) * open
                    * (1.0 + 0.8 * (1.0 - dim))
                    * ease::smoothstep((e - ICE_LOW.0) / ICE_LOW.1);
            if ice >= ICE_FLOOR {
                let u = ((ice - ICE_FLOOR) / (1.0 - ICE_FLOOR)).clamp(0.0, 1.0);
                if let Some(d) = faint.get_mut(i) {
                    *d = (255.0 * (u / CREST).min(1.0)).max(1.0) as u8;
                }
                return Ink::BLANK;
            }
            // The warm glow: a few pixels over the land's top (so it runs on unbroken
            // behind the hills), its top uneven along the horizon.
            let wave = f64::from(cols_edge.get(x as usize).copied().unwrap_or(0.0));
            let top = f64::from(land_top.get(x as usize).copied().unwrap_or(0.0));
            let above = top - (f64::from(y) + 0.5);
            // Past a few pixels the glow is below its cut: skip the `exp`.
            if !(0.0..=4.0 * GLOW_PX + GLOW_WAVE.0 * px_per_deg).contains(&above) {
                return Ink::BLANK;
            }
            let up = (above - wave * px_per_deg).max(0.0);
            let g = (-up / GLOW_PX).exp() * (0.35 + 0.65 * side) * (0.7 + 0.3 * dim);
            glow_ink(g)
        });
    }
}

impl Generator for Noctilucent {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (usize::from(size.width()), usize::from(size.height()));
        let (px_w, px_h) = (w as f64, 2.0 * h as f64);
        self.seed(&mut ctx.rng);
        self.horizon = HORIZON_ROW * px_h;
        let per_deg = (self.horizon / SKY_SPAN).max(0.05);
        self.f = per_deg * 180.0 / PI;
        self.cx = 0.5 * px_w;
        let (f, horizon, cx) = (self.f, self.horizon, self.cx);

        // The ice lattice over the picture, capped.
        let lattice = grid::lattice_for_aspect(px_w / px_h, MAX_NODES);
        let (lw, lh) = (usize::from(lattice.width()), usize::from(lattice.height()));
        self.field.resize(lattice, 0.0);
        self.map.rebuild(lattice, self.picture_size(size));
        let packets = self.packets;
        self.nodes.clear();
        for j in 0..lh {
            for i in 0..lw {
                let x = grid::centre_of(i as f64, lw, w) + 0.5;
                let y = grid::centre_of(j as f64, lh, 2 * h) + 0.5;
                let el = (horizon - y) / f;
                let az = (x - cx) / f;
                let xy = layer_xy(el.max(0.0), az);
                let phase = std::array::from_fn(|k| {
                    let p = packets[k];
                    let (s, c) = (p.k.0 * xy.0 + p.k.1 * xy.1).sin_cos_det();
                    (c as f32, s as f32)
                });
                // A wave's length in lattice nodes at this distance (small-angle), and the
                // resolution fade: gone below 3 nodes, whole from 6.
                let dist = xy.0.hypot(xy.1).hypot(LAYER_KM).max(1.0);
                let nodes_per_rad = f * lw as f64 / w as f64;
                let resolve = std::array::from_fn(|k| {
                    let p = packets[k];
                    let lambda = TAU / p.k.0.hypot(p.k.1).max(1e-9);
                    let lambda_nodes = lambda / dist * nodes_per_rad;
                    (255.0 * ease::smoothstep((lambda_nodes - 4.0) / 4.0)).round() as u8
                });
                self.nodes.push(Node {
                    xy: (xy.0 as f32, xy.1 as f32),
                    el: el as f32,
                    az: az as f32,
                    p: layer_point(LAYER_KM, el.max(0.0), az),
                    azc: {
                        let (s, c) = az.sin_cos_det();
                        (c as f32, s as f32)
                    },
                    slant: {
                        let se = el.max(0.0).sin_det();
                        (1.0 / (se * se + 0.015).sqrt() / 5.0).min(1.0) as f32
                    },
                    phase,
                    resolve,
                });
            }
        }

        // Land and a few dark banks of low cloud over the northern glow.
        let land = Skyline::new(&mut ctx.rng, w, per_deg, LAND);
        let bank_seed: u64 = ctx.rng.random();
        let banks: Vec<(f64, f64, f64, f64)> = (0..ctx.rng.random_range(1..4))
            .map(|_| {
                (
                    ctx.rng.random_range(0.0..px_w),
                    ctx.rng.random_range(1.2..3.5) * per_deg,
                    ctx.rng.random_range(0.15..0.4) * px_w,
                    ctx.rng.random_range(0.3..0.75) * per_deg,
                )
            })
            .collect();
        self.cover.clear();
        for y in 0..2 * h {
            for x in 0..w {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let mut c = f64::from(land.cover(x as i32, y as i32, horizon));
                for &(bx, up, half_w, half_h) in &banks {
                    let u = (fx - bx) / half_w;
                    let v = (fy - (horizon - up)) / half_h;
                    // A long, soft stratus wisp: its thickness wanders along it, its
                    // top is ragged, and it is dark but not opaque (the glow shows
                    // faintly through its thin parts).
                    let thick = 0.55 + 0.9 * value_noise(bank_seed ^ bx.to_bits(), fx * 3.0, 0.0);
                    let v = v / thick;
                    let r = u * u + v * v * (if v > 0.0 { 2.5 } else { 1.0 });
                    c = c.max(BANK_DARK * ease::smoothstep((1.0 - r) / 0.6));
                }
                self.cover.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        self.faint.clear();
        self.faint.resize(2 * h * w, 0);
        // The land's top per column (the glow sits on it): the first row it covers.
        self.land_top.clear();
        let cover = &self.cover;
        self.land_top.extend((0..w).map(|x| {
            (0..2 * h)
                .find(|&y| cover.get(y * w + x).is_some_and(|&c| c >= 200))
                .unwrap_or(2 * h) as f32
        }));
        self.rows.clear();
        self.rows.resize(2 * h, (0.0, 0.0));
        self.cols.clear();
        self.cols.resize(w, 0.0);
        let edge_seed: u64 = ctx.rng.random();
        self.edge.clear();
        self.edge.extend((0..w).map(|x| {
            // Value noise over ~7 px, as `value_noise`'s 90 km lattice.
            let n = value_noise(edge_seed, x as f64 * 90.0 / GLOW_WAVE.1, 0.0);
            (GLOW_WAVE.0 * (2.0 * n - 1.0)) as f32
        }));
        // A few faint stars high up, where the sky is darkest.
        self.stars.resize(size);
        let ds = self.stars.dot_size();
        let n = (0.012 * size.area() as f64) as usize;
        self.star_list.clear();
        for _ in 0..n {
            let x = ctx.rng.random_range(0.0..f64::from(ds.width()));
            let y = ctx.rng.random_range(0.0..0.45 * f64::from(ds.height()));
            let level = ctx.rng.random_range(70.0..150.0);
            self.star_list.push((x, y, level as u8));
        }
        self.picture.resize(size);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.step_field();
            self.paint();
            self.plot_stars();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.picture.blit(canvas, |_, _, top, bottom| {
            // ASCII: the ice by brightness; the dim twilight sky stays dark. The ice sits high
            // on its ramp, so its span (ASCII_ICE and up) is stretched over the glyph ramp,
            // keeping the waves' texture instead of a wall of `@`.
            let pick = sciart_core::raster::stronger(top, bottom);
            if pick.level < 60 {
                return None;
            }
            let (char, _) = sciart_core::raster::ascii_ramp_cell(
                Ink::new(
                    pick.role,
                    ink::level(
                        (f64::from(pick.level) - ASCII_ICE) * 255.0 / (255.0 - ASCII_ICE) + 40.0,
                    ),
                ),
                Ink::BLANK,
            )?;
            Some((char, pick))
        });
        self.stars.blit_over(canvas, &self.picture);
    }
}

impl Noctilucent {
    /// The stars, where the sky is dark enough to show them: the brighter ice drowns them
    /// as it drifts over (and they come back behind it).
    fn plot_stars(&mut self) {
        self.stars.clear();
        // The faint ice sheet as dots: each pixel's four dots lit by a fixed hash against
        // its density, so the texture drifts with the clouds instead of sparkling.
        let w = usize::from(self.picture.pixel_size().width()).max(1);
        for (i, &d) in self.faint.iter().enumerate() {
            if d == 0 {
                continue;
            }
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            for k in 0..4 {
                let (dx, dy) = (2 * x + (k & 1), 2 * y + (k >> 1));
                if hash01(0x1CE ^ cell_key(dx, dy)) * 255.0 < f64::from(d) {
                    self.stars.set(dx, dy, ICE[usize::from(d >= ICE_CREST)]);
                }
            }
        }
        for &(x, y, level) in &self.star_list {
            let (px, py) = (((x - 0.5) / 2.0) as i32, ((y - 0.5) / 2.0) as i32);
            let under = self.picture.get(px, py).map_or(0, |p| p.level);
            if under < STAR_DROWN {
                let level = if level >= 120 { 150 } else { 50 };
                self.stars.plot(x, y, Ink::new(Role::Star, level));
            }
        }
    }

    fn picture_size(&self, size: Size) -> Size {
        Size::new(size.width(), size.height().saturating_mul(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rad(d: f64) -> f64 {
        d.to_radians()
    }

    fn built(size: Size) -> (Noctilucent, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = Noctilucent::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    #[test]
    fn nlc_a1_lit_below_the_shadow_line_and_never_low_clouds() {
        let d = rad(10.0);
        // Scan up from the northern horizon: lit, then (past the shadow line) unlit.
        let mut line = None;
        for i in 1..900 {
            let el = rad(f64::from(i) * 0.1);
            let lit = clearance(LAYER_KM, el, 0.0, d, 0.0) > 0.0;
            match (line, lit) {
                (None, false) => line = Some(el),
                (Some(_), true) => panic!("lit again above the shadow line at {el}"),
                _ => {}
            }
        }
        let line = line.expect("a shadow line");
        assert!(line > rad(2.0) && line < rad(40.0), "{}", line.to_degrees());
        assert!(clearance(LAYER_KM, 0.5 * line, 0.0, d, 0.0) > 0.0);
        assert!(clearance(LAYER_KM, 1.5 * line, 0.0, d, 0.0) < 0.0);
        for dep in [1.5, 3.0, 8.0, 16.0] {
            for e in [0.5, 2.0, 10.0, 45.0, 89.0] {
                for az in [-60.0, 0.0, 60.0] {
                    assert!(
                        clearance(10.0, rad(e), rad(az), rad(dep), 0.0) < 0.0,
                        "{dep} {e} {az}"
                    );
                }
            }
        }
    }

    #[test]
    fn nlc_a2_lit_area_shrinks_as_the_sun_sinks() {
        let lit_area = |dep: f64| {
            let mut n = 0;
            for i in 0..72 {
                for j in 0..90 {
                    let (az, el) = (rad(-180.0 + f64::from(i) * 5.0), rad(0.5 + f64::from(j)));
                    if clearance(LAYER_KM, el, az, rad(dep), 0.0) > 0.0 {
                        n += 1;
                    }
                }
            }
            n
        };
        let mut prev = usize::MAX;
        for dep in [6.0, 8.0, 10.0, 12.0, 14.0, 16.0] {
            let a = lit_area(dep);
            assert!(a < prev, "{dep}: {a}");
            prev = a;
        }
    }

    #[test]
    fn nlc_a3_ice_shows_only_where_lit() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for step in 0..(2.0 * HALF_NIGHT * 60.0) as usize {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
            if step % 97 != 0 {
                continue;
            }
            let (d, sun_az) = sun_at(g.t);
            for (v, n) in g.field.as_slice().iter().zip(&g.nodes) {
                if *v > 0.0 {
                    let c = clearance(LAYER_KM, f64::from(n.el), f64::from(n.az), d, sun_az);
                    assert!(c > -0.5 * EDGE_KM, "ice in shadow: {c} km");
                }
            }
        }
    }

    #[test]
    fn nlc_a4_the_wave_field_moves() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let before = g.field.as_slice().to_vec();
        for _ in 0..60 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let changed = before
            .iter()
            .zip(g.field.as_slice())
            .filter(|(a, b)| (*a - *b).abs() > 1e-3)
            .count();
        assert!(changed > before.len() / 20, "{changed} of {}", before.len());
    }

    #[test]
    fn nlc_a5_ice_is_blue_dots_on_black() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for _ in 0..300 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        // The picture holds only black and the glow; the ice is braille dots.
        let ps = g.picture.pixel_size();
        for y in 0..i32::from(ps.height()) {
            for x in 0..i32::from(ps.width()) {
                let p = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                assert!(matches!(p.role, Role::Background | Role::Hot), "{p:?}");
            }
        }
        let mut canvas = Canvas::new(Size::new(120, 40), GlyphMode::Unicode);
        g.render(&mut canvas);
        let ice: Vec<Ink> = canvas
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Wave)
            .map(|c| c.ink)
            .collect();
        assert!(ice.len() > 50, "{}", ice.len());
        // Two flat blue steps (ADR-0018), the bright crests among them.
        assert!(ice.iter().all(|i| ICE.contains(i)));
        assert!(ice.contains(&ICE[1]));
    }

    #[test]
    fn nlc_a6_the_cloud_field_stays_in_the_sky_all_loop() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let ps = g.picture.pixel_size();
        let area = ps.area();
        for _ in 0..16 {
            for _ in 0..(5.0 / SIM_DT) as usize {
                g.update(SIM_DT, &mut ctx);
                g.prepare();
            }
            let mut ice = 0;
            for y in 0..i32::from(ps.height()) {
                for x in 0..i32::from(ps.width()) {
                    if g.picture
                        .get(x, y)
                        .is_some_and(|p| matches!(p.role, Role::Wave | Role::Star))
                    {
                        ice += 1;
                    }
                }
            }
            // Crest pixels and the faint sheet's dotted pixels together.
            let ice = ice + g.faint.iter().filter(|&&d| d > 0).count();
            assert!(ice > area / 25, "t = {}: {ice} of {area}", g.t);
        }
    }

    #[test]
    fn nlc_a7_the_sky_is_black_over_a_thin_warm_horizon_glow() {
        // No twilight wash (ADR-0018): an orange `Hot` glow a few pixels deep over the
        // horizon, fading upward into black.
        let (mut g, mut ctx) = built(Size::new(120, 40));
        for _ in 0..120 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        let horizon = g.horizon as i32;
        let mut glow_cols = 0;
        for x in 0..120 {
            let mut seen = false;
            for y in (0..horizon).rev() {
                let p = g.picture.get(x, y).unwrap_or(Ink::BLANK);
                if p.role != Role::Hot {
                    continue;
                }
                seen = true;
                assert!(horizon - y <= 8, "glow {} px up at {x},{y}", horizon - y);
            }
            glow_cols += usize::from(seen);
        }
        assert!(glow_cols > 20, "{glow_cols} glowing columns");
        // The warm glow's top is uneven along the horizon, within its wave.
        let (g, _) = built(Size::new(120, 40));
        let (lo, hi) = g
            .edge
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &e| {
                (lo.min(e), hi.max(e))
            });
        assert!(hi - lo > 0.3 && hi <= GLOW_WAVE.0 as f32 && lo >= -GLOW_WAVE.0 as f32);
    }

    #[test]
    fn nlc_lattice_stays_capped() {
        let (g, _) = built(Size::new(500, 150));
        assert!(g.field.len() <= MAX_NODES && g.nodes.len() == g.field.len());
    }
}
