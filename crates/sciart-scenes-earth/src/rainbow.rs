//! The rainbow in two alternating acts (SPEC-002 `rainbow`, M16 thread style, GEN-R25).
//!
//! **Inside a raindrop**: one big drop and a ray of sunlight traced through it (Snell's
//! law at the surface, mirror reflection inside), split into six colours by water's
//! dispersion: one internal reflection sends the light back at 42° from the antisolar
//! direction (the primary bow), two at 51° (the secondary). **The sky**: a shower lit by
//! a low Sun behind the viewer; every colour's Descartes angle is a circle about the
//! antisolar point, so the primary (red outside) and the reversed secondary are drawn as
//! thin per-colour braille arcs, with Alexander's dark band black between them, faint
//! supernumerary arcs inside the primary, and slanted rain streaks in drifting curtains
//! that brighten and dim the bows along their arcs while the Sun passes in and out of
//! cloud.
//!
//! Colour (ADR-0018): each wavelength in the nearest spectral role of M11's palette
//! (orange-red `Hot`, yellow `Sun`, green `Particle`, cyan `Wave`, blue `Cold`, violet
//! `Accent`); everything is braille dots on black.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::draw;
use sciart_core::prelude::*;
use sciart_core::rng::{hash01, value_noise};
use sciart_core::timeline::ease;

use sciart_shared::skyline::{Skyline, Terrain};

pub static META: Meta = Meta {
    name: "rainbow",
    category: Category::EarthSky,
    description: "Light traced through a raindrop, then the primary and reversed secondary bows with Alexander's dark band",
    title: "Rainbow · raindrops split light",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 2] = ["inside a raindrop", "primary and secondary"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Rainbow::default())
}

/// The acts (s): the ray diagram, then the sky.
const ACTS: [(&str, f64); 2] = [("drop", 12.0), ("sky", 16.0)];
const ACT_DROP: usize = 0;

/// Drop radius (nm): 0.5 mm.
const DROP: f64 = 5.0e5;
/// Airy fringe scale: `z = AIRY · (a/λ)^{2/3} · Δθ` (Δθ in radians). Stylised: fringes
/// ~1.8° apart, about 2.5× real ones for 0.5 mm drops, so they resolve on a terminal.
const AIRY: f64 = 0.75;
/// Supernumeraries fade beyond the first few (a spread of drop sizes washes them out).
#[cfg(test)]
const FRINGE_FADE: f64 = 5.0;
/// Airy maxima `Ai(−z)²`: the main bow and the first supernumerary.
const FRINGES: [f64; 2] = [1.019, 3.248];
/// Colour dispersion drawn ×this about 550 nm in the sky (stylised: a real primary's
/// red-to-violet span of ~1.8° is 2–4 pixels on a terminal). The secondary, twice as
/// dispersed already, is widened less so Alexander's band stays wide.
const DISPERSION: (f64, f64) = (2.5, 1.15);
/// Neighbouring colour arcs are at least this many dots apart, so each keeps its own
/// braille cells (a cell shows one ink) and the order reads at any size.
const ARC_GAP: f64 = 4.0;
/// The six colours: wavelength (nm) and role, red → violet.
pub(crate) const COLOURS: [(f64, Role); 6] = [
    (650.0, Role::Hot),
    (590.0, Role::Sun),
    (540.0, Role::Particle),
    (490.0, Role::Wave),
    (455.0, Role::Cold),
    (415.0, Role::Accent),
];
/// Dispersion inside the drop of the ray diagram, ×this about 550 nm (stylised, so six
/// colours separate within one drop).
const DROP_DISPERSION: f64 = 4.0;
/// The secondary's strength relative to the primary's.
const SECONDARY: f64 = 0.62;
/// The bow's strength: its floor with the rain and Sun at their dimmest, and the share of
/// full sunlit rain at which it is at full strength.
const BOW_FLOOR: f64 = 0.35;
const BOW_FULL: f64 = 0.6;
/// Sun breaks: the Sun behind the viewer passes in and out of cloud (value noise in time,
/// cell in s), dimming the bow and the sunlit rain to this share.
const SUN_CELL: f64 = 6.0;
const SUN_LOW: f64 = 0.4;
/// Rain curtains: slant (px across per px down, as the rain falls), cell (share of the
/// width) and drift (px/s at 100 px wide).
const CURTAIN_SLANT: f64 = 0.15;
const CURTAIN_CELL: f64 = 0.22;
const CURTAIN_DRIFT: f64 = 2.2;
/// Rain streaks: one per `STREAK_AREA` cells (at most `MAX_STREAKS`), `STREAK_LEN` dots
/// long, falling at `STREAK_SPEED` (dots/s, ±40 %), drawn where the curtain passes
/// `STREAK_VEIL`.
const STREAK_AREA: usize = 30;
const MAX_STREAKS: usize = 400;
const STREAK_LEN: f64 = 5.0;
const STREAK_SPEED: f64 = 70.0;
const STREAK_VEIL: f64 = 0.35;
/// Sun elevation over one loop (deg): it sinks, so the bow rises.
const SUN_ELEV: (f64, f64) = (18.0, 8.0);
const LOOP: f64 = 50.0;
/// Framing: the horizon's row (share of the height) and the sky shown above it (deg).
const HORIZON_ROW: f64 = 0.9;
const SKY_SPAN: f64 = 46.0;
/// Points per traced arc (a full circle about the antisolar point).
const ARC_STEPS: usize = 360;
/// Hills the bow's feet stand on.
const HILLS: Terrain = Terrain {
    ridge: (0.6, 2.5),
    trees: Some((0.8, 2.2)),
    gap: (0.3, 1.2),
    clearing: 0.35,
};

// Inks (all braille dots).
const DROP_INK: Ink = Ink::new(Role::Structure, 150);
const SUNRAY: Ink = Ink::new(Role::Star, 255);
const CONTEXT_RAY: Ink = Ink::new(Role::Dim, 90);
const GUIDE: Ink = Ink::new(Role::Dim, 150);
const RIDGE: Ink = Ink::new(Role::Structure, 110);
/// Below the faintest bow step, so a streak never recolours a bow cell.
const STREAK: Ink = Ink::new(Role::Dim, 60);
const RAY_LEVEL: u8 = 230;
const SUPER_LEVEL: u8 = 70;
const LABEL_INK: Ink = Ink::new(Role::Text, 200);

/// Refractive index of water at `nm` (Cauchy fit through 404 nm and 700 nm).
pub(crate) fn water_n(nm: f64) -> f64 {
    1.324_96 + 2961.0 / (nm * nm)
}

/// The angle of incidence of Descartes' minimum-deviation ray after `k` internal
/// reflections: `cos² i = (n² − 1) / (k (k + 2))`.
fn descartes_incidence(n: f64, k: u32) -> f64 {
    let k = f64::from(k);
    ((n * n - 1.0) / (k * (k + 2.0))).sqrt().acos()
}

/// Descartes' rainbow angle (rad from the antisolar point) after `k` internal reflections.
pub(crate) fn bow_angle(n: f64, k: u32) -> f64 {
    let i = descartes_incidence(n, k);
    let r = (i.sin_det() / n).asin();
    let dev = 2.0 * (i - r) + f64::from(k) * (PI - 2.0 * r);
    // Primary: θ = π − D; secondary: θ = D − π (the light comes back from the other side).
    if k < 2 { PI - dev } else { dev - PI }
}

/// The bow angle as drawn: Descartes' angle with its spread about 550 nm widened by
/// [`DISPERSION`].
fn drawn_angle(nm: f64, k: u32) -> f64 {
    let mid = bow_angle(water_n(550.0), k);
    let spread = if k == 1 { DISPERSION.0 } else { DISPERSION.1 };
    mid + spread * (bow_angle(water_n(nm), k) - mid)
}

/// The arcs' angles (rad) at `dots_per_rad`: per colour, the primary's and the
/// secondary's, drawn angles spread so neighbours are at least [`ARC_GAP`] dots apart
/// about the 540 nm arc; and the two supernumeraries inside the primary's violet, spaced
/// as Airy's maxima (at least `ARC_GAP` apart).
pub(crate) fn arc_angles(dots_per_rad: f64) -> ([[f64; 6]; 2], [f64; 2]) {
    let gap = ARC_GAP / dots_per_rad.max(1e-9);
    let mut out = [[0.0; 6]; 2];
    for (k, bow) in out.iter_mut().enumerate() {
        let k = k as u32 + 1;
        let mid = drawn_angle(COLOURS[2].0, k);
        for (a, &(nm, _)) in bow.iter_mut().zip(&COLOURS) {
            *a = drawn_angle(nm, k);
        }
        // Widen about the green arc until every neighbour pair is `gap` apart.
        let min_step = bow
            .windows(2)
            .map(|p| (p[0] - p[1]).abs())
            .fold(f64::INFINITY, f64::min);
        if min_step < gap {
            let s = gap / min_step.max(1e-12);
            for a in bow.iter_mut() {
                *a = mid + s * (*a - mid);
            }
        }
    }
    let spacing = ((FRINGES[1] - FRINGES[0]) / (AIRY * (DROP / 550.0f64).powf(2.0 / 3.0))).max(gap);
    let violet = out[0][5];
    (out, [violet - spacing, violet - 2.0 * spacing])
}

/// Airy's function `Ai(x)`: the power series for `|x| ≤ 6`, the asymptotic forms beyond.
#[cfg(test)]
pub(crate) fn airy(x: f64) -> f64 {
    const C1: f64 = 0.355_028_053_887_817;
    const C2: f64 = 0.258_819_403_792_807;
    if x < -6.0 {
        let z = -x;
        let zeta = 2.0 / 3.0 * z.powf(1.5);
        return (zeta + PI / 4.0).sin_det() / (PI.sqrt() * z.powf(0.25));
    }
    if x > 6.0 {
        let zeta = 2.0 / 3.0 * x.powf(1.5);
        return (-zeta).exp() / (2.0 * PI.sqrt() * x.powf(0.25));
    }
    let x3 = x * x * x;
    let (mut f, mut g) = (1.0, x);
    let (mut tf, mut tg) = (1.0, x);
    for k in 0..60 {
        let k = f64::from(k);
        tf *= x3 / ((3.0 * k + 2.0) * (3.0 * k + 3.0));
        tg *= x3 / ((3.0 * k + 3.0) * (3.0 * k + 4.0));
        f += tf;
        g += tg;
        if tf.abs() < 1e-17 && tg.abs() < 1e-17 {
            break;
        }
    }
    C1 * f - C2 * g
}

/// One bow's intensity `z` Airy units onto its bright side: `Ai(−z)²`, with fringes past
/// the first damped toward their mean `1/(2π√z)`.
#[cfg(test)]
fn bow(z: f64) -> f64 {
    let ai = airy(-z);
    let wave = ai * ai;
    if z <= 2.0 {
        return wave;
    }
    let mean = 1.0 / (2.0 * PI * z.sqrt());
    mean + (wave - mean) * (-(z - 2.0) / FRINGE_FADE).exp()
}

#[cfg(test)]
/// The semantic role that carries a wavelength's colour, red → violet, in M11's saturated
/// palette (ADR-0018): orange-red, yellow, green, cyan, blue, magenta.
pub(crate) fn spectral_role(nm: f64) -> Role {
    match nm {
        n if n >= 610.0 => Role::Hot,
        n if n >= 570.0 => Role::Sun,
        n if n >= 505.0 => Role::Particle,
        n if n >= 475.0 => Role::Wave,
        n if n >= 435.0 => Role::Cold,
        _ => Role::Accent,
    }
}

/// A bow's light `a` (0–1 of its top) in three flat steps.
pub(crate) fn bow_level(a: f64) -> u8 {
    match a {
        a if a >= 0.8 => 230,
        a if a >= 0.55 => 150,
        _ => 70,
    }
}

// ---- The ray diagram (act 1) ----

/// Sideways spacing (dots) of the six colours' paths inside the drop (display only).
const RIBBON: f64 = 0.7;

/// The point `s` dots along polyline `pts` and its unit direction there; `None` past
/// its end or on a degenerate one.
fn point_at(pts: &[(f64, f64)], mut s: f64) -> Option<((f64, f64), (f64, f64))> {
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l = (b.0 - a.0).hypot(b.1 - a.1);
        if l < 1e-9 {
            continue;
        }
        let d = ((b.0 - a.0) / l, (b.1 - a.1) / l);
        if s <= l {
            return Some(((a.0 + d.0 * s, a.1 + d.1 * s), d));
        }
        s -= l;
    }
    None
}

type V2 = (f64, f64);

fn dot2(a: V2, b: V2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

/// Refract unit direction `d` at a surface with unit normal `nrm` facing against `d`,
/// from index `n1` into `n2` (Snell's law, vector form); `None` on total reflection.
pub(crate) fn refract(d: V2, nrm: V2, n1: f64, n2: f64) -> Option<V2> {
    let eta = n1 / n2;
    let cos_i = -dot2(nrm, d);
    let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
    (k >= 0.0).then(|| {
        let f = eta * cos_i - k.sqrt();
        (eta * d.0 + f * nrm.0, eta * d.1 + f * nrm.1)
    })
}

/// A ray through a unit drop at the origin (screen axes, y down): sunlight travelling
/// `+x` enters at height `−b` (b > 0 above the centre), refracts, reflects `k` times
/// inside and leaves. Returns the surface points (entry, `k` reflections, exit) and the
/// exit direction.
pub(crate) fn trace_drop(b: f64, n: f64, k: usize) -> ([V2; 4], V2) {
    let mut pts = [(0.0, 0.0); 4];
    let p0 = (-(1.0 - b * b).max(0.0).sqrt(), -b);
    pts[0] = p0;
    let mut d = refract((1.0, 0.0), p0, 1.0, n).unwrap_or((1.0, 0.0));
    let mut p = p0;
    for i in 1..=k + 1 {
        // The far crossing of the unit circle from `p` along `d` (|p| = |d| = 1).
        let s = -2.0 * dot2(p, d);
        p = (p.0 + s * d.0, p.1 + s * d.1);
        pts[i.min(3)] = p;
        if i <= k {
            let c = 2.0 * dot2(d, p);
            d = (d.0 - c * p.0, d.1 - c * p.1);
        }
    }
    let out = refract(d, (-p.0, -p.1), n, 1.0).unwrap_or(d);
    (pts, out)
}

/// The angle (rad) between the light leaving a drop along `out` and the antisolar
/// direction an observer looks along to see it (`+x`, the sunlight's): the bow angle.
pub(crate) fn exit_angle(out: V2) -> f64 {
    (-out.0).clamp(-1.0, 1.0).acos()
}

/// The diagram's refractive index for a wavelength: water's, with its dispersion about
/// 550 nm widened by [`DROP_DISPERSION`].
fn diagram_n(nm: f64) -> f64 {
    let mid = water_n(550.0);
    mid + DROP_DISPERSION * (water_n(nm) - mid)
}

/// The impact parameter (b) of the bow ray with `k` reflections at 550 nm: the primary
/// enters above the centre, the secondary below.
fn bow_b(k: u32) -> f64 {
    let b = descartes_incidence(water_n(550.0), k).sin_det();
    if k == 1 { b } else { -b }
}

#[derive(Debug, Default)]
pub struct Rainbow {
    salt: u64,
    t: f64,
    acts: Option<Timeline>,
    /// Art size in dots (above the title row) and the canvas cells.
    art: (f64, f64),
    cells: (u16, u16),
    /// Horizon (dot row) and dots per degree of sky.
    horizon: f64,
    per_deg: f64,
    /// The land's top (dot row) per dot column.
    ridge: Vec<f32>,
    /// Rain-curtain density along `x − slant·y` (px), refreshed per drawn frame.
    curtain: Vec<f32>,
    dots: Dots,
    /// The labels of act 1: cell and text, `None` when hidden.
    labels: [Option<(i32, i32, u8)>; 2],
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl Rainbow {
    fn size(&self) -> Size {
        Size::new(self.cells.0, self.cells.1)
    }

    fn beat(&self) -> Beat {
        match &self.acts {
            Some(tl) => tl.at(self.t),
            None => Beat {
                index: ACT_DROP,
                label: ACTS[0].0,
                local: 0.0,
                progress: 0.0,
            },
        }
    }

    /// Sun elevation (rad) and the rain curtain's strength at loop time `t`.
    pub(crate) fn sky_state(t: f64) -> (f64, f64) {
        // The scene opens with the shower already under way, past the seam.
        let p = (t / LOOP + 0.12).rem_euclid(1.0);
        let elev = SUN_ELEV.0 + (SUN_ELEV.1 - SUN_ELEV.0) * p;
        // The shower thins at the loop's seam, so the Sun's jump back is never seen, and
        // swells and ebbs in between.
        let seam = ease::smoothstep(p / 0.1) * ease::smoothstep((1.0 - p) / 0.1);
        let swell = 0.8 + 0.2 * (TAU * 2.0 * p).cos_det();
        (elev.to_radians(), seam * swell)
    }

    /// Sunlight on the rain at time `t` (`SUN_LOW`..1): the Sun behind the viewer passes
    /// in and out of broken cloud, brightening and dimming the whole bow over seconds.
    pub(crate) fn sun_at(salt: u64, t: f64) -> f64 {
        let n = value_noise(salt ^ 0x5a11, t, 0.0, SUN_CELL);
        SUN_LOW + (1.0 - SUN_LOW) * ease::smoothstep((n - 0.2) / 0.6)
    }

    /// Rain-curtain density (0..1) at curtain coordinate `x − slant·y` (px) and time `t`:
    /// two octaves of drifting value noise, sharpened so veils have gaps between them.
    fn curtain_at(salt: u64, x: f64, width: f64, t: f64) -> f64 {
        let cell = (CURTAIN_CELL * width).max(4.0);
        let drift = CURTAIN_DRIFT * t * width / 100.0;
        let a = value_noise(salt, x - drift, 0.0, cell);
        let b = value_noise(salt ^ 0x77, x - 1.7 * drift, 5.0, 0.37 * cell);
        ease::smoothstep((0.7 * a + 0.3 * b - 0.28) / 0.44)
    }

    /// The curtain density at dot `(x, y)`.
    fn veil(&self, x: f64, y: f64) -> f64 {
        // Curtain coordinates are in pixels (half a dot's resolution in x, a quarter in y).
        let rows = self.art.1 / 2.0;
        let c = x / 2.0 - CURTAIN_SLANT * (y / 2.0) + CURTAIN_SLANT * rows;
        let i = c.max(0.0) as usize;
        f64::from(self.curtain.get(i).copied().unwrap_or(0.0))
    }

    /// Refresh the per-frame table: curtain density.
    fn refresh(&mut self) {
        let w = self.art.0 / 2.0;
        let rows = self.art.1 / 2.0;
        let x0 = -CURTAIN_SLANT * rows;
        for (i, c) in self.curtain.iter_mut().enumerate() {
            *c = Self::curtain_at(self.salt, x0 + i as f64, w, self.t) as f32;
        }
    }

    fn raster(&mut self) {
        self.dots.clear();
        self.labels = [None, None];
        let beat = self.beat();
        if beat.index == ACT_DROP {
            self.diagram(beat.local);
        } else {
            self.refresh();
            self.sky();
        }
    }

    // ---- Act 1 ----

    /// The drop's centre and radius (dots).
    fn drop_frame(&self) -> (V2, f64) {
        let (w, h) = self.art;
        let r = (0.3 * h).min(0.2 * w).max(2.0);
        ((0.62 * w, 0.4 * h), r)
    }

    /// Draw `pts` as a polyline, its first `len` dots only.
    fn polyline(&mut self, pts: &[V2], mut len: f64, ink: Ink) {
        for p in pts.windows(2) {
            if len <= 0.0 {
                return;
            }
            let (a, b) = (p[0], p[1]);
            let l = (b.0 - a.0).hypot(b.1 - a.1);
            let f = (len / l.max(1e-9)).min(1.0);
            self.dots
                .line(a.0, a.1, a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, ink);
            len -= l;
        }
    }

    /// One bow ray (`k` reflections) traced `progress` (0..1) of the way.
    fn bow_ray(&mut self, k: usize, progress: f64) {
        let (c, r) = self.drop_frame();
        let b = bow_b(k as u32);
        let to_screen = |p: V2| (c.0 + r * p.0, c.1 + r * p.1);
        let entry = to_screen((-(1.0 - b * b).sqrt(), -b));
        let start = (0.0, entry.1);
        // The white sunlight to the drop: the first quarter of the progress.
        let first = (progress / 0.25).min(1.0) * (entry.0 - start.0);
        self.polyline(&[start, entry], first, SUNRAY);
        if progress <= 0.25 {
            return;
        }
        let reach = 1.6 * self.art.0;
        let along = (progress - 0.25) / 0.75;
        // Per colour: the path (entry, reflections, exit, far end), the length inside the
        // drop and the length drawn so far.
        let n = k + 2;
        let mut paths = [[(0.0, 0.0); 5]; 6];
        let mut inside = [0.0; 6];
        let mut drawn = [0.0; 6];
        let mut out = [(0.0, 0.0); 6];
        for (ci, &(nm, _)) in COLOURS.iter().enumerate() {
            let (pts, dir) = trace_drop(b, diagram_n(nm), k);
            for (q, p) in paths[ci].iter_mut().zip(&pts[..n]) {
                *q = to_screen(*p);
            }
            let exit = paths[ci][n - 1];
            paths[ci][n] = (exit.0 + reach * dir.0, exit.1 + reach * dir.1);
            out[ci] = dir;
            inside[ci] = paths[ci][..n]
                .windows(2)
                .map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1))
                .sum();
            // Inside the drop the ray travels on at the same pace; outside it races to
            // the frame's edge over the last third.
            drawn[ci] = if along < 0.66 {
                inside[ci] * along / 0.66
            } else {
                inside[ci] + reach * (along - 0.66) / 0.34
            };
        }
        // Inside the drop the six paths nearly coincide and a braille cell shows one ink:
        // drawn a dot at a time, each path offset 0.7 dot sideways by its colour, with
        // the colour drawn last (the one a shared cell shows) rotating along the ray, so
        // the interior reads as a multicoloured ribbon (BOW-A12).
        let steps = drawn
            .iter()
            .zip(&inside)
            .map(|(d, i)| d.min(*i))
            .fold(0.0, f64::max)
            .ceil() as usize;
        for j in 0..=steps {
            for o in 0..6 {
                let ci = (j + 1 + o) % 6;
                let s = j as f64;
                if s > drawn[ci].min(inside[ci]) {
                    continue;
                }
                let Some((p, d)) = point_at(&paths[ci][..n], s) else {
                    continue;
                };
                let off = (ci as f64 - 2.5) * RIBBON;
                let ink = Ink::new(COLOURS[ci].1, RAY_LEVEL);
                self.dots.plot(p.0 - d.1 * off, p.1 + d.0 * off, ink);
            }
        }
        // Out of the drop the colours fan apart: plain lines, red last on shared cells.
        for ci in (0..6).rev() {
            let past = drawn[ci] - inside[ci];
            if past > 0.0 {
                let e = paths[ci][n - 1];
                let f = (e.0 + past * out[ci].0, e.1 + past * out[ci].1);
                self.dots
                    .line(e.0, e.1, f.0, f.1, Ink::new(COLOURS[ci].1, RAY_LEVEL));
            }
        }
    }

    /// The bow angle marker of the ray with `k` reflections: a dashed guide along the
    /// antisolar direction from its exit point, a dotted arc to the green ray, and the
    /// angle's label.
    fn angle_marker(&mut self, k: usize, slot: usize) {
        let (c, r) = self.drop_frame();
        // At 550 nm, where the diagram's index is water's: the true bow angle.
        let (pts, out) = trace_drop(bow_b(k as u32), diagram_n(550.0), k);
        let e = pts[k + 1];
        let exit = (c.0 + r * e.0, c.1 + r * e.1);
        // The guide: the line an observer at the ray's end looks along to the antisolar
        // point (parallel to the sunlight), drawn back from the exit.
        let guide = 1.1 * r;
        let mut s = 0.0;
        while s <= guide {
            self.dots.plot(exit.0 - s, exit.1, GUIDE);
            s += 3.0;
        }
        let theta = exit_angle(out);
        let rho = 0.75 * r;
        // From the guide (pointing −x) to the ray, which leaves below it.
        let steps = (rho * theta).ceil().max(2.0) as usize;
        for i in 0..=steps {
            if i % 2 == 1 {
                continue;
            }
            let a = PI - theta * i as f64 / steps as f64;
            let (sn, cs) = a.sin_cos_det();
            self.dots.plot(exit.0 + rho * cs, exit.1 + rho * sn, GUIDE);
        }
        // The label beside the arc's middle, outside it.
        let (sn, cs) = (PI - 0.5 * theta).sin_cos_det();
        let (lx, ly) = (exit.0 + 1.35 * rho * cs, exit.1 + 1.35 * rho * sn);
        let deg = theta.to_degrees().round().clamp(0.0, 180.0) as u8;
        let (cx, cy) = (
            (lx / f64::from(Dots::CELL_W)) as i32 - 2,
            (ly / f64::from(Dots::CELL_H)) as i32,
        );
        self.labels[slot] = Some((cx, cy, deg));
    }

    fn diagram(&mut self, local: f64) {
        let (c, r) = self.drop_frame();
        // The drop: a thin circle.
        let n = (TAU * r).ceil().clamp(16.0, 2048.0) as usize;
        let mut prev = (c.0 + r, c.1);
        for i in 1..=n {
            let (sn, cs) = (TAU * i as f64 / n as f64).sin_cos_det();
            let q = (c.0 + r * cs, c.1 + r * sn);
            self.dots.line(prev.0, prev.1, q.0, q.1, DROP_INK);
            prev = q;
        }
        // Parallel sunlight, faint, stopping at the drop.
        for b in [-0.75, -0.3, 0.15, 0.5] {
            let x = c.0 - r * (1.0f64 - b * b).sqrt();
            let mut s = 0.0;
            while s < x {
                self.dots.plot(s, c.1 - r * b, CONTEXT_RAY);
                s += 2.0;
            }
        }
        // The primary from 0.5 s, its 42° at 4.5 s; the secondary from 5.5 s, its 51° at
        // 9.5 s; then both hold.
        let p1 = ((local - 0.5) / 3.5).clamp(0.0, 1.0);
        let p2 = ((local - 5.5) / 3.5).clamp(0.0, 1.0);
        if p1 > 0.0 {
            self.bow_ray(1, p1);
        }
        if p2 > 0.0 {
            self.bow_ray(2, p2);
        }
        if local >= 4.5 {
            self.angle_marker(1, 0);
        }
        if local >= 9.5 {
            self.angle_marker(2, 1);
        }
    }

    // ---- Act 2 ----

    /// The dot position of the sky direction `θ` from the antisolar point at angle `ψ`
    /// about it (ψ = 0 straight up), with the Sun at elevation `e` (sin, cos).
    fn sky_point(&self, theta: (f64, f64), psi: (f64, f64), e: (f64, f64)) -> Option<V2> {
        let ((st, ct), (sp, cp), (se, ce)) = (theta, psi, e);
        // Forward x, right y, up z; antisolar a = (cos e, 0, −sin e), up-ish u = (sin e,
        // 0, cos e), right w = (0, 1, 0).
        let v = (ct * ce + st * cp * se, st * sp, -ct * se + st * cp * ce);
        if v.2 <= 0.0 || v.0 <= 0.0 {
            return None;
        }
        let az = v.1.atan2(v.0);
        let el = v.2.clamp(-1.0, 1.0).asin();
        let per_rad = self.per_deg * 180.0 / PI;
        Some((0.5 * self.art.0 + az * per_rad, self.horizon - el * per_rad))
    }

    /// Whether dot `(x, y)` is above the land.
    fn open(&self, (x, y): V2) -> bool {
        let i = (x.max(0.0) as usize).min(self.ridge.len().saturating_sub(1));
        self.ridge.get(i).is_some_and(|&top| y < f64::from(top))
    }

    /// Trace the circle `θ` about the antisolar point; `ink(x, y)` colours (or skips)
    /// each segment by its midpoint, and `dotted` skips every other one.
    fn arc(
        &mut self,
        theta: f64,
        e: (f64, f64),
        dotted: bool,
        ink: impl Fn(&Self, V2) -> Option<Ink>,
    ) {
        let st = theta.sin_cos_det();
        let mut prev: Option<V2> = None;
        for i in 0..=ARC_STEPS {
            let psi = PI * (2.0 * i as f64 / ARC_STEPS as f64 - 1.0);
            let q = self
                .sky_point(st, psi.sin_cos_det(), e)
                .filter(|&q| self.open(q));
            if let (Some(a), Some(b)) = (prev, q)
                && (!dotted || i % 2 == 0)
                && (b.0 - a.0).abs() < 0.25 * self.art.0
            {
                let mid = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
                if let Some(k) = ink(self, mid) {
                    self.dots.line(a.0, a.1, b.0, b.1, k);
                }
            }
            prev = q;
        }
    }

    fn sky(&mut self) {
        let (elev, rain) = Self::sky_state(self.t);
        let e = elev.sin_cos_det();
        let sun = Self::sun_at(self.salt, self.t);
        // The land's top: a thin ridge line.
        let mut prev: Option<V2> = None;
        for (x, &top) in self.ridge.iter().enumerate() {
            let q = (x as f64, f64::from(top));
            if let Some(p) = prev {
                self.dots.line(p.0, p.1, q.0, q.1, RIDGE);
            }
            prev = Some(q);
        }
        let per_rad = self.per_deg * 180.0 / PI;
        let (bows, supers) = arc_angles(per_rad);
        // The bow's light at a point: brighter where the curtain is dense and the Sun out.
        let light = move |g: &Self, p: V2, share: f64| {
            let lit = rain * sun * (0.55 + 0.45 * g.veil(p.0, p.1));
            share * (BOW_FLOOR + (1.0 - BOW_FLOOR) * (lit / BOW_FULL).min(1.0))
        };
        for (&theta, &(_, role)) in bows[1].iter().zip(&COLOURS) {
            self.arc(theta, e, false, |g, p| {
                Some(Ink::new(role, bow_level(light(g, p, SECONDARY))))
            });
        }
        for (&theta, &(_, role)) in bows[0].iter().zip(&COLOURS) {
            self.arc(theta, e, false, |g, p| {
                Some(Ink::new(role, bow_level(light(g, p, 1.0))))
            });
        }
        // Supernumeraries: two faint dotted arcs inside the violet, green then violet.
        for (&theta, role) in supers.iter().zip([Role::Particle, Role::Accent]) {
            self.arc(theta, e, true, |g, p| {
                (light(g, p, 1.0) > 0.5).then_some(Ink::new(role, SUPER_LEVEL))
            });
        }
        // The rain streaks last, only in cells nothing else lit: a streak dot in a bow's
        // cell would take the bow's colour (one ink per cell).
        let (w, h) = self.art;
        let n = ((self.size().area() / STREAK_AREA).min(MAX_STREAKS)) as u64;
        let (dx, dy) = {
            let l = CURTAIN_SLANT.hypot(1.0);
            (CURTAIN_SLANT / l, 1.0 / l)
        };
        for k in 0..n {
            let speed = STREAK_SPEED * (0.6 + 0.8 * hash01(self.salt ^ (k * 3 + 1)));
            let y = (hash01(self.salt ^ (k * 3 + 2)) * h + speed * self.t).rem_euclid(h);
            let x = (hash01(self.salt ^ (k * 3)) * (w + 0.2 * h) + CURTAIN_SLANT * y)
                .rem_euclid(w + 0.2 * h)
                - 0.1 * h;
            let (x1, y1) = (x - STREAK_LEN * dx, y - STREAK_LEN * dy);
            let empty = |g: &Self, x: f64, y: f64| {
                let (cx, cy) = (
                    (x / f64::from(Dots::CELL_W)).floor() as i32,
                    (y / f64::from(Dots::CELL_H)).floor() as i32,
                );
                g.dots.cell_mask(cx, cy) == Some(0)
            };
            if self.veil(x, y) * rain < STREAK_VEIL
                || !self.open((x, y))
                || !empty(self, x, y)
                || !empty(self, x1, y1)
            {
                continue;
            }
            self.dots.line(x, y, x1, y1, STREAK);
        }
    }
}

impl Generator for Rainbow {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.beat().index.min(CAPTIONS.len() - 1)])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if self.acts.is_none() {
            self.acts = Some(Timeline::new(&ACTS));
            self.salt = ctx.rng.random();
        }
        self.cells = (size.width(), size.height());
        self.dots.resize(size);
        let ds = self.dots.dot_size();
        let (w, h) = (f64::from(ds.width()), f64::from(ds.height()));
        self.art = (w, (h - f64::from(Dots::CELL_H)).max(1.0));
        // The horizon and the sky's scale as before M16 (pixel rows), in dots.
        let px_h = 2.0 * f64::from(size.height());
        let horizon_px = HORIZON_ROW * px_h;
        let per_deg_px = (horizon_px / SKY_SPAN).max(0.05);
        self.horizon = 2.0 * horizon_px;
        self.per_deg = 2.0 * per_deg_px;
        let hills = Skyline::new(&mut ctx.rng, usize::from(size.width()), per_deg_px, HILLS);
        self.ridge.clear();
        for x in 0..usize::from(ds.width()) {
            let px = (x as f64 + 0.5) / 2.0;
            self.ridge
                .push((2.0 * (horizon_px - hills.height_at(px))) as f32);
        }
        let span = usize::from(size.width()) + (CURTAIN_SLANT * px_h).ceil() as usize + 2;
        self.curtain.clear();
        self.curtain.resize(span, 0.0);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        let rows = i32::from(self.cells.1) - 1;
        for &(x, y, deg) in self.labels.iter().flatten() {
            if (0..rows).contains(&y) {
                let mut u = StackText::<8>::new();
                let mut a = StackText::<8>::new();
                let _ = write!(u, "{deg}°");
                let _ = write!(a, "{deg}d");
                draw::text_mode(canvas, x, y, u.as_str(), a.as_str(), LABEL_INK);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(a: f64) -> f64 {
        a.to_degrees()
    }

    fn scene(size: Size, seed: u64) -> (Rainbow, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = Rainbow::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// The canvas at scene time `t` (jumping there).
    fn frame(g: &mut Rainbow, t: f64) -> Canvas {
        g.t = t;
        g.dirty = true;
        g.prepare();
        let mut canvas = Canvas::new(g.size(), GlyphMode::Unicode);
        g.render(&mut canvas);
        canvas
    }

    const SKY_T: f64 = 12.0 + 4.0;

    #[test]
    fn bow_a1_descartes_angles() {
        let na = water_n(589.0);
        let p = deg(bow_angle(na, 1));
        let s = deg(bow_angle(na, 2));
        assert!((p - 42.0).abs() <= 0.5, "primary {p}");
        assert!((50.5..=51.5).contains(&s), "secondary {s}");
    }

    #[test]
    fn bow_a2_red_outside_in_the_primary_inside_in_the_secondary() {
        let (red, violet) = (water_n(700.0), water_n(400.0));
        assert!(bow_angle(red, 1) > bow_angle(violet, 1));
        assert!(bow_angle(red, 2) < bow_angle(violet, 2));
        // The drawn arcs keep the order: angle falls red → violet in the primary and
        // rises in the secondary; the primary's rim is `Hot`, its inner arc `Accent`.
        for per_rad in [40.0, 120.0, 400.0] {
            let (bows, _) = arc_angles(per_rad);
            assert!(bows[0].windows(2).all(|p| p[0] > p[1]), "{bows:?}");
            assert!(bows[1].windows(2).all(|p| p[0] < p[1]), "{bows:?}");
        }
        assert_eq!(COLOURS[0].1, Role::Hot);
        assert_eq!(COLOURS[5].1, Role::Accent);
        // And through a drop: the primary's red leaves at a larger angle than its violet.
        let (_, red) = trace_drop(bow_b(1), diagram_n(650.0), 1);
        let (_, violet) = trace_drop(bow_b(1), diagram_n(415.0), 1);
        assert!(exit_angle(red) > exit_angle(violet));
        let (_, red) = trace_drop(bow_b(2), diagram_n(650.0), 2);
        let (_, violet) = trace_drop(bow_b(2), diagram_n(415.0), 2);
        assert!(exit_angle(red) < exit_angle(violet));
    }

    #[test]
    fn bow_a3_the_secondary_is_fainter() {
        for lit in [0.0, 0.3, 1.0] {
            let s = BOW_FLOOR + (1.0 - BOW_FLOOR) * f64::min(lit / BOW_FULL, 1.0);
            assert!(bow_level(SECONDARY * s) <= bow_level(s));
        }
        assert!(bow_level(SECONDARY) < bow_level(1.0));
    }

    #[test]
    fn bow_a4_supernumeraries_lie_inside_the_primary() {
        let n = water_n(550.0);
        let peak = deg(bow_angle(n, 1));
        let scale = AIRY * (DROP / 550.0f64).powf(2.0 / 3.0);
        let airy_at = |d: f64| bow(scale * (bow_angle(n, 1) - d.to_radians()));
        let mut wave = Vec::new();
        let mut d = 30.0;
        while d < 48.0 {
            if airy_at(d) > airy_at(d - 0.01) && airy_at(d) > airy_at(d + 0.01) {
                wave.push(d);
            }
            d += 0.01;
        }
        assert!(wave.len() >= 3, "{wave:?}");
        assert!(wave.iter().all(|m| *m < peak), "{wave:?} vs {peak}");
        // The drawn supernumerary arcs lie inside the violet, at Airy's spacing or more.
        for per_rad in [40.0, 400.0] {
            let (bows, supers) = arc_angles(per_rad);
            let gap = ARC_GAP / per_rad;
            assert!(supers[0] < bows[0][5] - gap + 1e-9 && supers[1] < supers[0]);
        }
    }

    #[test]
    fn bow_a5_threads_on_black() {
        // M16: black ground, braille only; the bows in the six roles at the three steps.
        let (mut g, _) = scene(Size::new(120, 40), 42);
        for t in [4.5, 10.0, SKY_T, SKY_T + 8.0] {
            let canvas = frame(&mut g, t);
            let mut bow = 0;
            for c in canvas.cells() {
                if c.ch == ' ' || c.ink.role == Role::Text {
                    continue;
                }
                assert!(
                    c.bg == Ink::BLANK && ('\u{2800}'..='\u{28ff}').contains(&c.ch),
                    "t = {t}: {c:?}"
                );
                if COLOURS.iter().any(|&(_, r)| r == c.ink.role) {
                    assert!(
                        [70, 150, 230].contains(&c.ink.level),
                        "t = {t}: {:?}",
                        c.ink
                    );
                    bow += 1;
                }
            }
            assert!(bow > 40, "t = {t}: {bow} bow cells");
        }
    }

    #[test]
    fn bow_a6_six_colours_in_order_each_on_its_own_cells() {
        // Down some column through both bows: the secondary's violet … red, then the
        // primary's red … violet (then perhaps the supernumeraries), each arc at least
        // ARC_GAP dots from its neighbours, so each colour keeps its own cells.
        let (mut g, _) = scene(Size::new(120, 40), 1);
        let canvas = frame(&mut g, SKY_T);
        use Role::*;
        let primary = [Hot, Sun, Particle, Wave, Cold, Accent];
        let mut secondary = primary;
        secondary.reverse();
        let mut found = false;
        for x in (8..60usize).rev() {
            let mut seen = Vec::new();
            for y in 0..39usize {
                let c = canvas.cells()[y * 120 + x];
                if COLOURS.iter().any(|&(_, r)| r == c.ink.role) && seen.last() != Some(&c.ink.role)
                {
                    seen.push(c.ink.role);
                }
            }
            // The two red arcs face each other across the dark band: one `Hot` run.
            if seen.len() >= 11 && seen[..6] == secondary && seen[5..11] == primary {
                found = true;
                break;
            }
        }
        assert!(found, "no column shows both bows in order");
        for (&(nm, role), _) in COLOURS.iter().zip(0..) {
            assert_eq!(spectral_role(nm), role);
        }
    }

    #[test]
    fn bow_a7_curtains_modulate_the_bow_along_the_arc_and_in_time() {
        let (mut g, _) = scene(Size::new(120, 40), 7);
        let bright = |c: &Canvas| {
            let mut sums = [(0.0, 0.0); 3];
            for (i, cell) in c.cells().iter().enumerate() {
                if COLOURS.iter().any(|&(_, r)| r == cell.ink.role) {
                    let k = 3 * (i % 120) / 120;
                    sums[k].0 += f64::from(cell.ink.level);
                    sums[k].1 += 1.0;
                }
            }
            sums.map(|(s, n)| if n > 0.0 { s / n } else { 0.0 })
        };
        let first = bright(&frame(&mut g, 13.0));
        assert!(first.iter().all(|&m| m > 0.0), "{first:?}");
        let later = bright(&frame(&mut g, 21.0));
        let change: f64 = first.iter().zip(&later).map(|(a, b)| (a - b).abs()).sum();
        assert!(change > 3.0, "{first:?} → {later:?}");
    }

    #[test]
    fn bow_a8_alexanders_band_is_black() {
        // Between the primary's red and the secondary's red no bow dot is drawn.
        let size = Size::new(120, 40);
        let (mut g, _) = scene(size, 3);
        g.t = SKY_T;
        g.dirty = true;
        g.prepare();
        let (elev, _) = Rainbow::sky_state(g.t);
        let e = elev.sin_cos_det();
        let (bows, _) = arc_angles(g.per_deg * 180.0 / PI);
        let (lo, hi) = (bows[0][0], bows[1][0]);
        // A cell is 2 × 4 dots: keep the sampled points a cell clear of the red arcs.
        let margin = 4.5 / (g.per_deg * 180.0 / PI);
        let canvas = frame(&mut g, SKY_T);
        let mut checked = 0;
        for k in 1..8 {
            let theta = lo + margin + (hi - lo - 2.0 * margin) * f64::from(k) / 8.0;
            for psi in [-0.6, -0.3, 0.0, 0.3, 0.6] {
                let Some((x, y)) = g.sky_point(theta.sin_cos_det(), f64::sin_cos_det(psi), e)
                else {
                    continue;
                };
                let (cx, cy) = ((x / 2.0) as usize, (y / 4.0) as usize);
                if cx < 120 && cy < 39 {
                    let ink = canvas.cells()[cy * 120 + cx].ink;
                    assert!(
                        !COLOURS.iter().any(|&(_, r)| r == ink.role),
                        "bow ink in Alexander's band: {ink:?} at ({cx}, {cy})"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 10);
    }

    #[test]
    fn bow_a9_sun_breaks_brighten_and_dim_the_whole_bow() {
        let mut lo: f64 = 1.0;
        let mut hi: f64 = 0.0;
        let mut widest: f64 = 0.0;
        for i in 0..600 {
            let t = f64::from(i) * 0.1;
            let s = Rainbow::sun_at(9, t);
            assert!((SUN_LOW - 1e-9..=1.0 + 1e-9).contains(&s), "{s} at {t}");
            lo = lo.min(s);
            hi = hi.max(s);
            let later = Rainbow::sun_at(9, t + 10.0);
            widest = widest.max((later - s).abs());
        }
        assert!(hi - lo > 0.4 && widest > 0.3, "{lo}..{hi}, {widest}");
        // A dim spell dims the whole bow: summed bow levels in the sky act.
        let (mut g, _) = scene(Size::new(120, 40), 5);
        let sky_times: Vec<f64> = (0..300)
            .map(|i| 12.5 + f64::from(i) * 0.05 + 28.0 * f64::from(i % 3))
            .collect();
        let sun = |t: f64| Rainbow::sun_at(g.salt, t);
        let pick = |want_hi: bool| {
            sky_times
                .iter()
                .copied()
                .max_by(|a, b| {
                    let o = sun(*a).total_cmp(&sun(*b));
                    if want_hi { o } else { o.reverse() }
                })
                .unwrap_or(SKY_T)
        };
        let (t_hi, t_lo) = (pick(true), pick(false));
        assert!(sun(t_hi) - sun(t_lo) > 0.2, "{t_hi} {t_lo}");
        let sum = |c: &Canvas| -> u32 {
            c.cells()
                .iter()
                .filter(|c| COLOURS.iter().any(|&(_, r)| r == c.ink.role))
                .map(|c| u32::from(c.ink.level))
                .sum()
        };
        let (bright, dim) = (sum(&frame(&mut g, t_hi)), sum(&frame(&mut g, t_lo)));
        assert!(bright > 0 && dim * 10 < bright * 9, "{bright} vs {dim}");
    }

    #[test]
    fn bow_a10_the_ray_diagram_shows_42_and_51_degrees() {
        // A ray through the drop at Descartes' impact parameter leaves at the bow angle.
        for (k, want) in [(1usize, 42.0), (2, 51.0)] {
            let n = water_n(550.0);
            let (pts, out) = trace_drop(bow_b(k as u32), n, k);
            let got = deg(exit_angle(out));
            assert!((got - want).abs() < 0.8, "k = {k}: {got}°");
            assert!((got - deg(bow_angle(n, k as u32))).abs() < 1e-6);
            // Its surface events lie on the drop, and it leaves downward (to the viewer).
            for p in &pts[..k + 2] {
                assert!((p.0.hypot(p.1) - 1.0).abs() < 1e-9, "{p:?}");
            }
            assert!(out.1 > 0.0 && out.0 < 0.0, "k = {k}: {out:?}");
        }
        // Drawn: by 10 s the six colours, the white sunray and both labels.
        let (mut g, _) = scene(Size::new(120, 40), 2);
        let canvas = frame(&mut g, 10.0);
        for &(_, role) in &COLOURS {
            assert!(
                canvas.cells().iter().any(|c| c.ink.role == role),
                "{role:?}"
            );
        }
        assert!(canvas.cells().iter().any(|c| c.ink == SUNRAY));
        let text: String = canvas.cells().iter().map(|c| c.ch).collect();
        assert!(text.contains("42°") && text.contains("51°"), "labels");
        // Early in the act only the white ray: the colours come as it enters the drop.
        let canvas = frame(&mut g, 0.9);
        assert!(!canvas.cells().iter().any(|c| c.ink.role == Role::Hot));
    }

    #[test]
    fn bow_a11_the_acts_alternate_with_their_captions() {
        let (mut g, _) = scene(Size::new(120, 40), 4);
        for (t, act) in [
            (1.0, 0),
            (11.5, 0),
            (12.5, 1),
            (27.5, 1),
            (28.5, 0),
            (41.0, 1),
        ] {
            g.t = t;
            assert_eq!(g.caption(), Some(CAPTIONS[act]), "t = {t}");
            let canvas = frame(&mut g, t);
            let drop = canvas.cells().iter().any(|c| c.ink == DROP_INK);
            assert_eq!(drop, act == 0, "t = {t}: the drop shows only in act 1");
        }
    }

    #[test]
    fn bow_a12_the_rays_inside_the_drop_are_a_multicoloured_ribbon() {
        for size in [Size::new(120, 40), Size::new(160, 45)] {
            let (mut g, _) = scene(size, 2);
            let canvas = frame(&mut g, 10.0);
            let (c, r) = g.drop_frame();
            let w = usize::from(size.width());
            let mut count = [0usize; 6];
            for (i, cell) in canvas.cells().iter().enumerate() {
                let (x, y) = (((i % w) as f64 + 0.5) * 2.0, ((i / w) as f64 + 0.5) * 4.0);
                if (x - c.0).hypot(y - c.1) > r - 3.0 {
                    continue;
                }
                if let Some(k) = COLOURS.iter().position(|&(_, role)| role == cell.ink.role) {
                    count[k] += 1;
                }
            }
            let total: usize = count.iter().sum();
            assert!(total > 30, "{count:?}");
            let top = *count.iter().max().unwrap_or(&0);
            assert!(
                top * 10 <= total * 4,
                "{size:?}: interior colours {count:?}"
            );
        }
    }
}
