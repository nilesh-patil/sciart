//! A hot Jupiter transiting a limb-darkened K dwarf, with a scrolling photometer strip
//! (SPEC-002 `exoplanet_transit`).
//!
//! Parameters follow HD 189733 b; the orbit is drawn much closer than the real one so it
//! fits the frame. The light curve integrates the quadratic limb-darkening law over the
//! occulted part of the disc (tabulated once per build), the star's lightness follows the
//! same limb-darkening law in false colour, and the planet shows its phase. Star, planet and orbit are rasterised in
//! `update`/`resize`; `render` blits and writes the labels.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use rand::Rng;
use sciart_core::geom::v3;
use sciart_core::prelude::*;
use sciart_core::title::{self, Title};
use sciart_core::{draw, ink, rng};

pub static META: Meta = Meta {
    name: "exoplanet_transit",
    category: Category::SunPlanets,
    description: "A hot Jupiter crossing its star while a photometer strip records the dip",
    title: "Planet transit · starlight dips",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(ExoplanetTransit::new())
}

/// Planet-to-star radius ratio and impact parameter of HD 189733 b (Bouchy et al. 2005;
/// Torres et al. 2008).
const P: f64 = 0.155;
const IMPACT: f64 = 0.66;
/// Quadratic limb darkening in the V band, approximate for a K1 dwarf (Claret 2000).
const U1: f64 = 0.62;
const U2: f64 = 0.12;
/// Drawn orbit radius in stellar radii (the real one is 8.8): a display compromise.
const A: f64 = 2.4;
/// Orbital period in simulated seconds.
const PERIOD: f64 = 14.0;

/// False colour: the disc is drawn on the `Ember` ramp at value
/// `STAR_V0 + (1 − STAR_V0)·I^STAR_GAMMA`, so its lightness falls with the intensity from
/// a pale-gold centre to a deep orange-red limb, as in a limb-darkened photograph.
const STAR_V0: f64 = 0.2;
const STAR_GAMMA: f64 = 1.3;

/// Flux table over `d ∈ [0, 1 + P]` and the radial quadrature steps per entry.
const FLUX_N: usize = 256;
const QUAD_N: usize = 200;

/// Photometer strip: rows `clamp(h/4, …)`, shown from this size up.
const STRIP_ROWS: (u16, u16) = (4, 12);
const STRIP_MIN: (u16, u16) = (24, 12);
/// Simulated seconds the strip spans: about half a period, so the dip fills a quarter of
/// its width.
const STRIP_SPAN: f64 = 0.55 * PERIOD;
/// Strip flux range, in transit depths above and below 1.
const STRIP_HI: f64 = 0.3;
const STRIP_LO: f64 = 1.35;
/// Photometric scatter of the observations, in transit depths.
const NOISE: f64 = 0.07;
const MODEL_INK: Ink = Ink::new(Role::Ramp(Ramp::Ember), 230);
const NOW_INK: Ink = Ink::new(Role::Ramp(Ramp::Ember), 255);
const OBS_INK: Ink = Ink::new(Role::Structure, 150);
const BASELINE_INK: Ink = Ink::new(Role::Dim, 30);
const ORBIT_INK: Ink = Ink::new(Role::Dim, 30);
const LABEL_INK: Ink = Ink::new(Role::Text, 110);
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
const STARS: usize = 80;
/// A star pixel this much covered by the planet is drawn blank.
const SILHOUETTE_COVER: f64 = 0.95;
/// Off the star, a planet pixel shows from this share of cover, at full brightness from
/// `PLANET_FULL_COVER`, so a planet a few pixels across is a round blot (TRAN-A9).
const PLANET_MIN_COVER: f64 = 0.3;
const PLANET_FULL_COVER: f64 = 0.75;

/// Quadratic limb darkening `I(μ) = 1 − u1(1 − μ) − u2(1 − μ)²` (TRAN-A4).
pub(crate) fn limb_intensity(mu: f64, u: (f64, f64)) -> f64 {
    let m = 1.0 - mu.clamp(0.0, 1.0);
    1.0 - u.0 * m - u.1 * m * m
}

/// Relative flux of a limb-darkened unit star with a planet of radius `p` centred `d`
/// away (TRAN-A4): `1 − ∫ I(r)·L(r) dr / π(1 − u1/3 − u2/6)`, where `L(r)` is the length
/// of the circle of radius `r` inside the planet, by the midpoint rule over the radii the
/// planet covers.
pub(crate) fn ld_flux(d: f64, p: f64, u: (f64, f64)) -> f64 {
    let d = d.abs();
    let (lo, hi) = ((d - p).max(0.0), (d + p).min(1.0));
    if p <= 0.0 || hi <= lo {
        return 1.0;
    }
    let arc = |r: f64| {
        if r <= p - d {
            TAU * r
        } else if d < 1e-12 {
            0.0
        } else {
            let c = (r * r + d * d - p * p) / (2.0 * r * d);
            2.0 * r * c.clamp(-1.0, 1.0).acos()
        }
    };
    let step = (hi - lo) / QUAD_N as f64;
    let blocked: f64 = (0..QUAD_N)
        .map(|k| {
            let r = lo + (k as f64 + 0.5) * step;
            limb_intensity((1.0 - r * r).max(0.0).sqrt(), u) * arc(r)
        })
        .sum();
    1.0 - blocked * step / (PI * (1.0 - u.0 / 3.0 - u.1 / 6.0))
}

/// Planet position in stellar radii at orbital phase `phi` (0 = mid-transit): `(x, y)` on
/// the sky, `z` toward the observer (TRAN-A5).
pub(crate) fn orbit(phi: f64) -> [f64; 3] {
    let sin_i = (1.0 - (IMPACT / A).powi(2)).sqrt();
    let (s, c) = phi.sin_cos();
    [A * s, IMPACT * c, A * sin_i * c]
}

/// A stateless standard normal for photometric sample `k` (Box–Muller of two hashes), so
/// a point keeps its scatter as it scrolls (TRAN-A8).
fn scatter(k: i64) -> f64 {
    let key = (k as u64).wrapping_mul(2);
    let u1 = rng::hash01(key).max(1e-12);
    let u2 = rng::hash01(key ^ 0x9E37_79B9_7F4A_7C15);
    (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
}

/// `ramp` at value `v`, or `None` below the visible floor (CORE-R66), so fades end in
/// nothing rather than in near-black pixels.
fn visible(ramp: Ramp, v: f64) -> Option<Ink> {
    (255.0 * v >= ink::MIN_VISIBLE_LEVEL).then(|| ink::ramp(ramp, v))
}

/// Share of pixel `(x, y)` inside the circle of radius `r` about `c`, from a 4 × 4 grid
/// of samples, so a small disc comes out round rather than as a plus (TRAN-A9).
fn pixel_cover(x: i32, y: i32, c: (f64, f64), r: f64) -> f64 {
    let inside = (0..16)
        .filter(|k| {
            let sx = f64::from(x) + (f64::from(k % 4) + 0.5) / 4.0;
            let sy = f64::from(y) + (f64::from(k / 4) + 0.5) / 4.0;
            (sx - c.0).hypot(sy - c.1) <= r
        })
        .count();
    inside as f64 / 16.0
}

/// Photometer strip layout, in cells and dots.
#[derive(Debug, Clone, Copy)]
struct Strip {
    /// Top cell row; the strip ends on row `h − 2`.
    row: i32,
    /// Dot rows and columns.
    top: f64,
    height: f64,
    left: i32,
    width: i32,
}

#[derive(Debug)]
pub struct ExoplanetTransit {
    t: f64,
    /// Star centre and radius, pixels.
    cx: f64,
    cy: f64,
    rs: f64,
    strip: Option<Strip>,
    /// Limb-darkened flux over `d ∈ [0, 1 + P]`.
    table: [f32; FLUX_N],
    depth: f64,
    /// (u, v, level) with `u, v ∈ [0, 1)`.
    stars: Vec<(f64, f64, u8)>,
    /// The shaded star pixels, `(x, y, ink)`.
    star: Vec<(i32, i32, Ink)>,
    pixels: Pixels,
    dots: Dots,
    size: Size,
}

impl ExoplanetTransit {
    fn new() -> Self {
        let table = std::array::from_fn(|i| {
            ld_flux((1.0 + P) * i as f64 / (FLUX_N - 1) as f64, P, (U1, U2)) as f32
        });
        let mut s = Self {
            t: 0.0,
            cx: 0.0,
            cy: 0.0,
            rs: 1.0,
            strip: None,
            table,
            depth: 0.0,
            stars: Vec::new(),
            star: Vec::new(),
            pixels: Pixels::default(),
            dots: Dots::default(),
            size: Size::new(1, 1),
        };
        s.depth = 1.0 - s.flux_at_d(IMPACT);
        s
    }

    fn phi(&self) -> f64 {
        TAU * self.t / PERIOD
    }

    /// Flux with the planet in front at sky distance `d`, from the table.
    fn flux_at_d(&self, d: f64) -> f64 {
        let x = d.abs() / (1.0 + P) * (FLUX_N - 1) as f64;
        if x >= (FLUX_N - 1) as f64 {
            return 1.0;
        }
        let i = x as usize;
        let f = x - i as f64;
        f64::from(self.table[i]) * (1.0 - f) + f64::from(self.table[i + 1]) * f
    }

    /// Relative flux at time `t` (TRAN-A5): only a planet in front dims the star.
    fn flux_at(&self, t: f64) -> f64 {
        let [x, y, z] = orbit(TAU * t / PERIOD);
        if z > 0.0 {
            self.flux_at_d(x.hypot(y))
        } else {
            1.0
        }
    }

    /// The `Ember` value of the disc at unit-disc offset `(nx, ny)`, or `None` off it
    /// (TRAN-A7).
    fn star_value(nx: f64, ny: f64) -> Option<f64> {
        let r2 = nx * nx + ny * ny;
        (r2 <= 1.0).then(|| {
            let i = limb_intensity((1.0 - r2).sqrt(), (U1, U2));
            STAR_V0 + (1.0 - STAR_V0) * i.powf(STAR_GAMMA)
        })
    }

    /// Star pixel ink at `(nx, ny)` with a share `cover` of it hidden by the planet: its
    /// mean brightness, so the silhouette's edge is anti-aliased; `None` once too dim to
    /// see.
    fn star_ink(nx: f64, ny: f64, cover: f64) -> Option<Ink> {
        Self::star_value(nx, ny).and_then(|v| visible(Ramp::Ember, v * (1.0 - cover)))
    }

    /// The planet's lit-side ink for a normal `(nx, ny)` (clamped onto the disc, for rim
    /// pixels whose centre lies just outside it) and the unit star→planet vector `r`,
    /// dimmed by the share `cover` of the pixel the planet fills; `None` on the night side
    /// (TRAN-A6, TRAN-A9).
    fn planet_ink(nx: f64, ny: f64, r: [f64; 3], cover: f64) -> Option<Ink> {
        let k = nx.hypot(ny).max(1.0);
        let (nx, ny) = (nx / k, ny / k);
        let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
        let cos = -(nx * r[0] + ny * r[1] + nz * r[2]);
        let gain = (cover / PLANET_FULL_COVER).min(1.0);
        (cos > 0.0 && cover >= PLANET_MIN_COVER)
            .then(|| visible(Ramp::Ice, (0.25 + 0.4 * cos) * gain))
            .flatten()
    }

    fn strip_y(&self, s: Strip, f: f64) -> f64 {
        let (hi, lo) = (1.0 + STRIP_HI * self.depth, 1.0 - STRIP_LO * self.depth);
        s.top + (hi - f) / (hi - lo) * (s.height - 1.0)
    }

    fn rasterise(&mut self) {
        self.pixels.clear();
        self.dots.clear();
        let (cx, cy, rs) = (self.cx, self.cy, self.rs);
        let w = i32::from(self.size.width());
        let on_disc = |x: f64, y: f64| (x - cx).hypot(y - cy) <= rs + 0.5;

        // Pixel `(x, y)` (cells across, half rows down) as a dot position.
        let dot = |x: f64, y: f64| Dots::from_cells(x, y / 2.0);
        for &(u, v, level) in &self.stars {
            let (x, y) = (u * f64::from(w), v * 2.0 * cy);
            if !on_disc(x, y) {
                let (dx, dy) = dot(x, y);
                self.dots.plot(dx, dy, Ink::new(Role::Dim, level));
            }
        }
        // The orbit, faint and only off the disc and away from the planet.
        let phi = self.phi();
        let pos = orbit(phi);
        let (px, py) = (cx + rs * pos[0], cy + rs * pos[1]);
        // Every other dot along the major axis's circumference.
        let n = ((TAU * A * rs).ceil() as usize).max(8);
        for k in 0..n {
            let q = orbit(TAU * k as f64 / n as f64);
            let (x, y) = (cx + rs * q[0], cy + rs * q[1]);
            if !on_disc(x, y) && (x - px).hypot(y - py) > rs * P + 1.5 {
                let (dx, dy) = dot(x, y);
                self.dots.plot(dx, dy, ORBIT_INK);
            }
        }

        // The star, shaded once per size.
        for &(x, y, ink) in &self.star {
            self.pixels.set(x, y, ink);
        }
        // Planet: in front it is a silhouette on the disc and a crescent off it; behind,
        // the star hides it (TRAN-A5, TRAN-A6).
        let rp = rs * P;
        let r_hat = v3::unit(pos);
        let behind = pos[2] < 0.0;
        for y in (py - rp).floor() as i32 - 1..=(py + rp).ceil() as i32 {
            for x in (px - rp).floor() as i32 - 1..=(px + rp).ceil() as i32 {
                if behind && self.pixels.get(x, y).is_some_and(|i| i != Ink::BLANK) {
                    continue;
                }
                let cover = pixel_cover(x, y, (px, py), rp);
                if cover == 0.0 {
                    continue;
                }
                let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let (nx, ny) = ((fx - px) / rp, (fy - py) / rp);
                let (sx, sy) = ((fx - cx) / rs, (fy - cy) / rs);
                let ink = if sx * sx + sy * sy <= 1.0 {
                    Self::star_ink(sx, sy, cover).filter(|_| cover < SILHOUETTE_COVER)
                } else {
                    Self::planet_ink(nx, ny, r_hat, cover)
                };
                self.pixels.set(x, y, ink.unwrap_or(Ink::BLANK));
            }
        }
        self.draw_strip();
    }

    /// The star, pixel by pixel (TRAN-A7): static, so shaded in `resize`. Limb pixels
    /// take the limb's value times the share of the pixel the disc covers, so the edge is
    /// anti-aliased rather than stepped.
    fn shade_star(&mut self) {
        self.star.clear();
        let (cx, cy, rs) = (self.cx, self.cy, self.rs);
        let (w, rows) = (
            i32::from(self.size.width()),
            2 * i32::from(self.size.height()),
        );
        let (x0, x1) = ((cx - rs).floor() as i32, (cx + rs).ceil() as i32);
        let (y0, y1) = ((cy - rs).floor() as i32, (cy + rs).ceil() as i32);
        for y in y0.max(0)..=y1.min(rows - 1) {
            for x in x0.max(0)..=x1.min(w - 1) {
                let (nx, ny) = (
                    (f64::from(x) + 0.5 - cx) / rs,
                    (f64::from(y) + 0.5 - cy) / rs,
                );
                let k = nx.hypot(ny).max(1.0);
                let cover = pixel_cover(x, y, (cx, cy), rs);
                let ink =
                    Self::star_value(nx / k, ny / k).and_then(|v| visible(Ramp::Ember, v * cover));
                if let Some(ink) = ink {
                    self.star.push((x, y, ink));
                }
            }
        }
    }

    /// The scrolling light curve over the last `STRIP_SPAN`, newest sample at the right
    /// (TRAN-A8).
    fn draw_strip(&mut self) {
        let Some(s) = self.strip else { return };
        let dt = STRIP_SPAN / f64::from(s.width);
        let newest = (self.t / dt).floor() as i64;
        let base = self.strip_y(s, 1.0);
        for c in (0..s.width).step_by(4) {
            self.dots.set(s.left + c, base.round() as i32, BASELINE_INK);
        }
        let mut prev: Option<(f64, f64)> = None;
        for c in 0..s.width {
            let k = newest - i64::from(s.width - 1 - c);
            let f = self.flux_at(k as f64 * dt);
            let x = f64::from(s.left + c);
            if k % 2 == 0 {
                let obs = f + NOISE * self.depth * scatter(k);
                self.dots.plot(x, self.strip_y(s, obs), OBS_INK);
            }
            let y = self.strip_y(s, f);
            if let Some((px, py)) = prev {
                self.dots.line(px, py, x, y, MODEL_INK);
            }
            prev = Some((x, y));
        }
        // "Now": the newest sample, bright, on a short tick, so the eye can tie the
        // planet's place to the curve.
        if let Some((x, y)) = prev {
            for dy in -3..=3 {
                self.dots.plot(x, y + f64::from(dy), NOW_INK);
            }
            self.dots.plot(x - 1.0, y, NOW_INK);
        }
    }

    fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
        let ink = [top, bottom]
            .into_iter()
            .filter(|i| i.role != Role::Background)
            .max_by_key(|i| i.level)?;
        let ch = if ink.role == Role::Ramp(Ramp::Ember) {
            match ink.level {
                l if l >= 220 => '@',
                l if l >= 180 => '#',
                l if l >= 140 => '*',
                l if l >= 100 => '+',
                l if l >= 60 => ':',
                _ => '.',
            }
        } else {
            'o'
        };
        Some((ch, ink))
    }

    fn draw_labels(&self, canvas: &mut Canvas) {
        if let Some(s) = self.strip {
            draw::label(canvas, 0, s.row, "relative flux", LABEL_INK);
        }
        let size = canvas.size();
        let room = title::readout_room(size, &Title::new(META.title, None));
        let mut buf = StackText::<16>::new();
        let _ = write!(buf, "depth {:.1} %", 100.0 * self.depth);
        if size.height() >= 6 && buf.len() <= room {
            let row = i32::from(size.height()) - 1;
            draw::label_right(canvas, row, buf.as_str(), READOUT_INK);
        }
    }
}

impl Generator for ExoplanetTransit {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (size.width(), size.height());
        self.size = size;
        let has_strip = w >= STRIP_MIN.0 && h >= STRIP_MIN.1;
        let strip_rows = (h / 4).clamp(STRIP_ROWS.0, STRIP_ROWS.1);
        // The star's rows: above the strip and a blank row, or the whole frame.
        let region = if has_strip { h - strip_rows - 2 } else { h };
        self.strip = has_strip.then(|| {
            let row = i32::from(h - 1 - strip_rows);
            Strip {
                row,
                top: f64::from(4 * row + 4),
                height: f64::from(4 * i32::from(strip_rows) - 4),
                left: 2,
                width: 2 * i32::from(w) - 4,
            }
        });
        self.cx = f64::from(w) / 2.0;
        self.cy = f64::from(region);
        let rows_px = 2.0 * f64::from(region);
        self.rs = (0.4 * rows_px)
            .min(0.94 * f64::from(w) / (2.0 * (A + P)))
            .max(0.5);
        if self.stars.is_empty() {
            self.stars = (0..STARS)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(30..90),
                    )
                })
                .collect();
        }
        self.pixels.resize(size);
        self.shade_star();
        self.dots.resize(size);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.dots.blit(canvas);
        self.pixels
            .blit(canvas, |_, _, top, bottom| Self::ascii_cell(top, bottom));
        self.draw_labels(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact area of intersection of two circles of radii `r1`, `r2` whose centres are
    /// `d` apart: the uniform-disc reference `ld_flux` is checked against (TRAN-A3).
    /// `acos` arguments are clamped so rounding at tangency cannot produce NaN.
    fn overlap_area(d: f64, r1: f64, r2: f64) -> f64 {
        let (r1, r2) = (r1.max(0.0), r2.max(0.0));
        let d = d.abs();
        let (small, big) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
        if small == 0.0 || d >= big + small {
            return 0.0;
        }
        if d <= big - small {
            return PI * small * small;
        }
        // Two circular segments minus the kite between the centres and the chord ends.
        let c_small = ((d * d + small * small - big * big) / (2.0 * d * small)).clamp(-1.0, 1.0);
        let c_big = ((d * d + big * big - small * small) / (2.0 * d * big)).clamp(-1.0, 1.0);
        let sides = (-d + small + big) * (d + small - big) * (d - small + big) * (d + small + big);
        let kite = 0.5 * sides.max(0.0).sqrt();
        (small * small * c_small.acos() + big * big * c_big.acos() - kite)
            .clamp(0.0, PI * small * small)
    }

    /// Relative flux of a uniform unit star with the planet's centre `d` from its centre.
    fn uniform_flux(d: f64, p: f64) -> f64 {
        1.0 - overlap_area(d, 1.0, p) / PI
    }

    #[test]
    fn tran_a1_flux_is_one_when_disks_do_not_touch() {
        for u in [(U1, U2), (0.0, 0.0)] {
            for k in 0..50 {
                let d = 1.0 + P + f64::from(k) * 0.037;
                assert_eq!(ld_flux(d, P, u), 1.0, "d={d}");
                assert_eq!(ld_flux(-d, P, u), 1.0);
            }
        }
        // And the scene: out of transit (|x| ≥ 1 + p on the sky) the star is at full flux.
        let s = ExoplanetTransit::new();
        for k in 0..1000 {
            let t = PERIOD * f64::from(k) / 1000.0;
            let [x, y, _] = orbit(TAU * t / PERIOD);
            if x.hypot(y) >= 1.0 + P {
                assert_eq!(s.flux_at(t), 1.0, "t={t}");
            }
        }
    }

    #[test]
    fn tran_a2_full_transit_depth_is_radius_ratio_squared() {
        // Without limb darkening a planet wholly on the disc blocks exactly p².
        for k in 0..=20 {
            let d = (1.0 - P) * f64::from(k) / 20.0;
            let depth = 1.0 - ld_flux(d, P, (0.0, 0.0));
            assert!((depth / (P * P) - 1.0).abs() < 2e-3, "d={d}: {depth}");
        }
    }

    #[test]
    fn tran_a3_overlap_known_value_and_edge_continuity() {
        // Two unit circles one radius apart overlap in a lens of area 2π/3 − √3/2.
        let lens = 2.0 * PI / 3.0 - 3f64.sqrt() / 2.0;
        assert!((overlap_area(1.0, 1.0, 1.0) - lens).abs() < 1e-12);
        // Continuous at the contained and tangent limits.
        let (r1, r2) = (3.0, 1.0);
        assert!((overlap_area(2.0 + 1e-9, r1, r2) - PI).abs() < 1e-6);
        assert!(overlap_area(4.0 - 1e-9, r1, r2) < 1e-6);
        // Degenerate radii are harmless.
        assert_eq!(overlap_area(0.5, 0.0, 1.0), 0.0);
        assert!(overlap_area(0.0, 1.0, 1.0).is_finite());
    }

    #[test]
    fn tran_a3_overlap_symmetric_in_radii_and_non_increasing_in_d() {
        let radii: [f64; 6] = [0.3, 0.5, 1.0, 1.7, 3.0, 10.0];
        for &r1 in &radii {
            for &r2 in &radii {
                let scale = PI * r1.min(r2).powi(2);
                let mut last = f64::INFINITY;
                for k in 0..=2000 {
                    let d = (r1 + r2 + 0.5) * f64::from(k) / 2000.0;
                    let a = overlap_area(d, r1, r2);
                    let b = overlap_area(d, r2, r1);
                    assert!(
                        a.is_finite() && a >= 0.0 && a <= scale * (1.0 + 1e-15),
                        "range d={d}"
                    );
                    assert!(
                        (a - b).abs() <= 1e-12 * scale.max(1.0),
                        "asym r1={r1} r2={r2} d={d}"
                    );
                    assert!(
                        a <= last + 1e-12 * scale.max(1.0),
                        "increase r1={r1} r2={r2} d={d}"
                    );
                    last = a;
                }
            }
        }
    }

    #[test]
    fn tran_a4_limb_darkened_flux() {
        // The law itself: 1 at the centre, 1 − u1 − u2 at the limb, monotone.
        assert!((limb_intensity(1.0, (U1, U2)) - 1.0).abs() < 1e-12);
        assert!((limb_intensity(0.0, (U1, U2)) - (1.0 - U1 - U2)).abs() < 1e-12);
        // Without limb darkening the quadrature reproduces the exact overlap model.
        for k in 0..=400 {
            let d = 1.3 * f64::from(k) / 400.0;
            let err = (ld_flux(d, P, (0.0, 0.0)) - uniform_flux(d, P)).abs();
            assert!(err < 3e-4, "d={d}: {err}");
        }
        // Central transit: deeper than p², close to p²/(1 − u1/3 − u2/6).
        let depth = 1.0 - ld_flux(0.0, P, (U1, U2));
        let expected = P * P / (1.0 - U1 / 3.0 - U2 / 6.0);
        assert!(
            depth > P * P && (depth / expected - 1.0).abs() < 0.03,
            "{depth}"
        );
        // The scene's transit is deepest at d = b.
        let s = ExoplanetTransit::new();
        let deepest = (0..=2000)
            .map(|k| s.flux_at(PERIOD * (f64::from(k) / 2000.0 - 0.5) * 0.3))
            .fold(f64::INFINITY, f64::min);
        assert!((deepest - s.flux_at_d(IMPACT)).abs() < 1e-4);
        assert!((1.0 - deepest - s.depth).abs() < 1e-4);
    }

    #[test]
    fn tran_a5_only_a_planet_in_front_dims_and_behind_it_is_hidden() {
        let s = ExoplanetTransit::new();
        for k in 0..1000 {
            let t = PERIOD * f64::from(k) / 1000.0;
            let [_, _, z] = orbit(TAU * t / PERIOD);
            if z <= 0.0 {
                assert_eq!(s.flux_at(t), 1.0, "t={t}");
            }
        }
        // Occultation: while the planet is behind, no planet pixel lands on the star, so at
        // ingress and egress it slips under the limb, not over it.
        for (w, h) in [(200, 50), (120, 40), (80, 24)] {
            let size = Size::new(w, h);
            let mut g = ExoplanetTransit::new();
            g.resize(size, &mut Ctx::new(5, size));
            assert!(g.star.len() > 20);
            for k in 0..=200 {
                let phi = PI / 2.0 + PI * f64::from(k) / 200.0;
                if orbit(phi)[2] >= 0.0 {
                    continue;
                }
                g.t = phi / TAU * PERIOD;
                g.rasterise();
                for &(x, y, star) in &g.star {
                    assert_eq!(g.pixels.get(x, y), Some(star), "{w}x{h} φ={phi} ({x}, {y})");
                }
            }
        }
    }

    fn lit_share(phi: f64) -> f64 {
        let size = Size::new(200, 50);
        let mut g = ExoplanetTransit::new();
        g.resize(size, &mut Ctx::new(5, size));
        g.t = phi / TAU * PERIOD;
        g.rasterise();
        let pos = orbit(phi);
        let (px, py, rp) = (g.cx + g.rs * pos[0], g.cy + g.rs * pos[1], g.rs * P);
        let (mut lit, mut all) = (0u32, 0u32);
        for y in (py - rp) as i32 - 1..=(py + rp) as i32 + 1 {
            for x in (px - rp) as i32 - 1..=(px + rp) as i32 + 1 {
                let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                if (fx - px).hypot(fy - py) <= rp {
                    all += 1;
                    lit += u32::from(
                        g.pixels
                            .get(x, y)
                            .is_some_and(|i| i.role == Role::Ramp(Ramp::Ice)),
                    );
                }
            }
        }
        f64::from(lit) / f64::from(all)
    }

    #[test]
    fn tran_a6_planet_shows_its_phase() {
        for phi in [PI / 4.0, PI / 2.0, 0.75 * PI, 1.25 * PI, 1.5 * PI] {
            let pos = orbit(phi);
            let expected = (1.0 - pos[2] / A) / 2.0;
            let got = lit_share(phi);
            assert!(
                (got - expected).abs() < 0.05,
                "φ={phi}: {got} vs {expected}"
            );
        }
    }

    #[test]
    fn tran_a7_star_lightness_follows_the_limb_darkening_law() {
        // Full value at the centre, falling with I(μ) all the way to a dim limb.
        let value = |r: f64| ExoplanetTransit::star_value(r, 0.0).unwrap();
        assert!((value(0.0) - 1.0).abs() < 1e-12);
        for k in 0..=100 {
            let r = f64::from(k) / 100.0;
            let i = limb_intensity((1.0 - r * r).max(0.0).sqrt(), (U1, U2));
            assert!((value(r) - (STAR_V0 + (1.0 - STAR_V0) * i.powf(STAR_GAMMA))).abs() < 1e-12);
        }
        // Visibly darker by mid-radius (not flat until the limb), and a dim limb.
        assert!(value(0.7) < 0.85, "{}", value(0.7));
        assert!(value(1.0) < 0.4, "{}", value(1.0));
        // Drawn: along a radius the level never rises, and the disc ends within a pixel of
        // its radius (anti-aliased, no glow), every star pixel above the visible floor.
        let size = Size::new(200, 50);
        let mut g = ExoplanetTransit::new();
        g.resize(size, &mut Ctx::new(5, size));
        let row = g.cy as i32;
        let mut last = u8::MAX;
        let mut lit = 0;
        for x in g.cx as i32..200 {
            let ink = g.pixels.get(x, row).unwrap();
            if ink == Ink::BLANK {
                break;
            }
            assert_eq!(ink.role, Role::Ramp(Ramp::Ember), "x={x}");
            assert!(ink.level <= last, "x={x}: {} after {last}", ink.level);
            last = ink.level;
            lit += 1;
        }
        assert!((g.rs - 1.0..=g.rs + 1.0).contains(&f64::from(lit)), "{lit}");
        for &(_, _, ink) in &g.star {
            assert!(f64::from(ink.level) >= ink::MIN_VISIBLE_LEVEL, "{ink:?}");
        }
    }

    #[test]
    fn tran_a8_strip_scrolls_and_draws_the_dip() {
        let size = Size::new(200, 50);
        let mut g = ExoplanetTransit::new();
        g.resize(size, &mut Ctx::new(5, size));
        let s = g.strip.expect("strip at 200x50");
        assert_eq!(
            ((s.top + s.height - 1.0) / 4.0) as i32,
            48,
            "strip ends on row h − 2"
        );
        // Every sample keeps its value and moves one dot left per P/W seconds: the
        // strip three samples later is the same strip shifted three dots.
        let dt = STRIP_SPAN / f64::from(s.width);
        let rows = s.top as i32..(s.top + s.height) as i32;
        let base = g.strip_y(s, 1.0).round() as i32;
        let lit_at = |g: &mut ExoplanetTransit, t: f64, shift: i32| {
            g.t = t;
            g.rasterise();
            rows.clone()
                .filter(|&y| y != base)
                .flat_map(|y| (13..s.width - 10).map(move |c| (c - shift, y)))
                .map(|(c, y)| g.dots.is_lit(s.left + c, y))
                .collect::<Vec<bool>>()
        };
        let t0 = (PERIOD * 0.9 / dt).floor() * dt + 0.5 * dt;
        assert_eq!(lit_at(&mut g, t0, 0), lit_at(&mut g, t0 + 3.0 * dt, 3));
        // Mid-transit, the strip's lowest model dot sits at F(b).
        g.t = PERIOD * 1.1;
        g.rasterise();
        let low = g.strip_y(s, g.flux_at_d(IMPACT)).round() as i32;
        let deepest = rows
            .filter(|&y| (s.left..s.left + s.width).any(|x| g.dots.is_lit(x, y)))
            .max()
            .unwrap_or(0);
        // Observations scatter below the model by at most ~4σ.
        let sigma_dots = NOISE * g.depth / (g.depth * (STRIP_HI + STRIP_LO)) * s.height;
        assert!(deepest >= low && f64::from(deepest - low) <= 4.0 * sigma_dots + 1.0);
    }

    #[test]
    fn tran_a9_silhouette_is_blank_with_a_soft_rim_and_strip_needs_room() {
        let size = Size::new(200, 50);
        let mut g = ExoplanetTransit::new();
        g.resize(size, &mut Ctx::new(5, size));
        // Mid-transit: the planet's centre pixel is empty, the disc centre is star.
        g.t = 0.0;
        g.rasterise();
        let pos = orbit(0.0);
        let (x, y) = (g.cx + g.rs * pos[0], g.cy + g.rs * pos[1]);
        assert_eq!(
            g.pixels.get(x as i32, y as i32).unwrap().role,
            Role::Background
        );
        assert_eq!(
            g.pixels.get(g.cx as i32, g.cy as i32).unwrap().role,
            Role::Ramp(Ramp::Ember)
        );
        // The silhouette's rim is anti-aliased: star pixels part-covered by the planet
        // are darker than the unobscured disc there, but not blank.
        let rp = g.rs * P;
        let mut rim = 0;
        for yy in (y - rp) as i32 - 1..=(y + rp) as i32 + 1 {
            for xx in (x - rp) as i32 - 1..=(x + rp) as i32 + 1 {
                let (fx, fy) = (f64::from(xx) + 0.5, f64::from(yy) + 0.5);
                let d = (fx - x).hypot(fy - y);
                if (rp - 0.3..rp + 0.3).contains(&d) {
                    let (sx, sy) = ((fx - g.cx) / g.rs, (fy - g.cy) / g.rs);
                    let bare = ExoplanetTransit::star_ink(sx, sy, 0.0).unwrap();
                    let got = g.pixels.get(xx, yy).unwrap();
                    assert_eq!(got.role, Role::Ramp(Ramp::Ember), "({xx}, {yy})");
                    assert!(got.level < bare.level, "({xx}, {yy}): {got:?} vs {bare:?}");
                    rim += 1;
                }
            }
        }
        assert!(rim >= 8, "{rim} rim pixels");
        // A small silhouette is a round blot, not a plus: at 80×24 (rp ≈ 2 px), across a
        // pixel of drift, every pixel well inside the planet is blank.
        let size = Size::new(80, 24);
        let mut g = ExoplanetTransit::new();
        g.resize(size, &mut Ctx::new(5, size));
        let rp = g.rs * P;
        assert!((1.5..2.5).contains(&rp), "{rp}");
        for k in 0..8 {
            g.t = PERIOD * (f64::from(k) - 4.0) / 400.0;
            g.rasterise();
            let pos = orbit(g.phi());
            let (x, y) = (g.cx + g.rs * pos[0], g.cy + g.rs * pos[1]);
            let mut blank = 0;
            for yy in (y - rp) as i32 - 1..=(y + rp) as i32 + 1 {
                for xx in (x - rp) as i32 - 1..=(x + rp) as i32 + 1 {
                    let (fx, fy) = (f64::from(xx) + 0.5, f64::from(yy) + 0.5);
                    if (fx - x).hypot(fy - y) <= rp - 0.75 {
                        assert_eq!(g.pixels.get(xx, yy), Some(Ink::BLANK), "k={k}");
                        blank += 1;
                    }
                }
            }
            assert!(blank >= 4, "k={k}: {blank}");
        }
        // Off the star the day side is round too: across the orbit, every pixel at least
        // half covered by the planet whose (rim-clamped) normal faces the star is drawn,
        // and none under `PLANET_MIN_COVER`.
        for k in 0..64 {
            let phi = TAU * f64::from(k) / 64.0;
            g.t = phi / TAU * PERIOD;
            g.rasterise();
            let pos = orbit(phi);
            let r_hat = v3::unit(pos);
            let (x, y) = (g.cx + g.rs * pos[0], g.cy + g.rs * pos[1]);
            for yy in (y - rp) as i32 - 1..=(y + rp) as i32 + 1 {
                for xx in (x - rp) as i32 - 1..=(x + rp) as i32 + 1 {
                    let (fx, fy) = (f64::from(xx) + 0.5, f64::from(yy) + 0.5);
                    if (fx - g.cx).hypot(fy - g.cy) <= g.rs + 1.0 {
                        continue;
                    }
                    let cover = pixel_cover(xx, yy, (x, y), rp);
                    let got = g
                        .pixels
                        .get(xx, yy)
                        .filter(|i| i.role == Role::Ramp(Ramp::Ice));
                    let (nx, ny) = ((fx - x) / rp, (fy - y) / rp);
                    let day = ExoplanetTransit::planet_ink(nx, ny, r_hat, 1.0).is_some();
                    if cover >= 0.5 && day {
                        assert!(got.is_some(), "φ={phi} ({xx}, {yy}) cover {cover}");
                    }
                    if cover < PLANET_MIN_COVER {
                        assert!(got.is_none(), "φ={phi} ({xx}, {yy}) cover {cover}");
                    }
                }
            }
        }
        for (w, h) in [(20, 5), (30, 11), (23, 30)] {
            let size = Size::new(w, h);
            let mut g = ExoplanetTransit::new();
            g.resize(size, &mut Ctx::new(5, size));
            assert!(g.strip.is_none(), "{w}x{h}");
        }
    }

    #[test]
    fn tran_a9_star_outshines_the_planet_in_mono() {
        // The disc centre is brighter in grey than the planet's day side at every phase.
        use sciart_render::Theme;
        let grey = |ink: Ink| Theme::Mono.color(ink, 0, 0).0;
        let size = Size::new(200, 50);
        let mut g = ExoplanetTransit::new();
        g.resize(size, &mut Ctx::new(5, size));
        let centre = g.pixels.get(g.cx as i32, g.cy as i32).unwrap();
        for k in 0..16 {
            g.t = PERIOD * f64::from(k) / 16.0;
            g.rasterise();
            for y in 0..100 {
                for x in 0..200 {
                    let ink = g.pixels.get(x, y).unwrap();
                    if ink.role == Role::Ramp(Ramp::Ice) {
                        assert!(grey(ink) < grey(centre), "{ink:?}");
                    }
                }
            }
        }
    }
}
