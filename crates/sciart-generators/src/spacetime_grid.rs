//! The rubber-sheet picture of general relativity: a perspective mesh dimpled by an
//! inspiralling binary, with quadrupole gravitational waves spreading outward at a finite
//! speed (SPEC-002 `spacetime_grid`).
//!
//! World units: the sheet spans `y ∈ [-1, 1]` (far edge at `+1`) and `x ∈ [-Lx, Lx]`, with
//! `Lx` widened to fill wide terminals; `z` points up, so wells are negative.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::{self, ASPECT};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "spacetime_grid",
    category: Category::DeepCosmos,
    description: "A rubber-sheet spacetime dimpled by an orbiting binary emitting gravitational waves",
    title: "Gravitational waves · inspiral",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(SpacetimeGrid::default())
}

/// Depth scale of the softened potential `z = -K Σ m / √(d² + ε²)`.
const WELL_K: f64 = 0.1;
/// Constant lift (per unit mass) that puts the sheet near `z = 0` around `r ≈ 1`, so the
/// camera fit sees the sheet rather than the far-field bowl.
const RIM_LIFT: f64 = WELL_K / 1.1;
/// Plummer softening: keeps the well bottom finite (and the mesh on screen).
const SOFTENING: f64 = 0.25;
/// Gravitational-wave speed `c` in sheet units per simulated second. Artistic: a real binary
/// radiates at wavelengths far larger than its orbit; here `λ = πc/Ω₀ ≈ 0.75` so the
/// two-armed spiral fits on the sheet while spanning several mesh spacings (no aliasing).
const WAVE_SPEED: f64 = 0.3;
/// Wave amplitude at the start of an inspiral cycle (sheet units, times the envelope).
const WAVE_AMP: f64 = 0.015;
/// Near-zone taper radius: the wave term fades inside it, where the wells dominate.
const TAPER: f64 = 0.5;

/// Camera pitch below the horizon and distance from the sheet centre.
const PITCH: f64 = 55.0 * PI / 180.0;
const CAM_DIST: f64 = 3.4;
/// Slow yaw sway: amplitude (radians) and period (s). Small enough that the `+y` edge
/// always stays the far edge, which the band-ordered painter relies on.
const YAW_AMP: f64 = 0.05;
const YAW_PERIOD: f64 = 47.0;

/// Mesh line spacing targets (cells) and samples per mesh edge along each line.
const COLS_PER_LINE: u16 = 8;
const ROWS_PER_LINE: f64 = 3.4;
const SUB_X: usize = 3;
const SUB_Y: usize = 2;
const MAX_NX: usize = 72;
const MAX_NY: usize = 40;

/// Classification thresholds for inks.
const COLD_DEPTH: f64 = -0.34;
const CREST: f64 = 0.6;
const CREST_ENV: f64 = 0.3;

const H: Glyph = Glyph::new('─', '-');
const V: Glyph = Glyph::new('│', '|');
const UP: Glyph = Glyph::new('╱', '/');
const DOWN: Glyph = Glyph::new('╲', '\\');
const CROSS: Glyph = Glyph::new('┼', '+');
const XCROSS: Glyph = Glyph::new('╳', 'X');
const SPHERE: [Glyph; 3] = [Glyph::ascii('@'), Glyph::ascii('O'), Glyph::ascii('o')];

/// A circular binary, optionally on a quadrupole-driven inspiral (Peters 1964, Newtonian
/// chirp): `a(τ) = a₀(1 − τ/τc)^¼`, hence by Kepler `Ω = Ω₀(1 − τ/τc)^-⅜` and
/// `Φ(τ) = Φ₀ + (8/5)Ω₀τc[1 − (1 − τ/τc)^⅝]`. Each cycle stops at `a = q_end·a₀` and
/// restarts; the phase stays continuous so the emitted wave train has no seam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Binary {
    pub m1: f64,
    pub m2: f64,
    pub a0: f64,
    pub omega0: f64,
    pub tau_c: f64,
    pub q_end: f64,
    pub phase0: f64,
    pub inspiral: bool,
}

impl Binary {
    pub(crate) const DEFAULT: Binary = Binary {
        m1: 1.0,
        m2: 0.65,
        a0: 0.55,
        omega0: TAU / 5.0,
        tau_c: 30.0,
        q_end: 0.55,
        phase0: 0.0,
        inspiral: true,
    };

    fn cycle(&self) -> f64 {
        self.tau_c * (1.0 - self.q_end.powi(4))
    }

    /// `(separation, orbital phase)` at time `t` (any sign; the inspiral repeats).
    pub(crate) fn state(&self, t: f64) -> (f64, f64) {
        if !self.inspiral {
            return (self.a0, self.phase0 + self.omega0 * t);
        }
        let cycle = self.cycle();
        let k = (t / cycle).floor();
        let tau = (t - k * cycle).clamp(0.0, cycle);
        let u = 1.0 - tau / self.tau_c;
        let (s2, s4) = (u.sqrt(), u.sqrt().sqrt());
        let chirp = 1.6 * self.omega0 * self.tau_c;
        let per_cycle = chirp * (1.0 - self.q_end.powf(2.5));
        // u^⅝ = u^½ · u^⅛
        let within = chirp * (1.0 - s2 * s4.sqrt());
        (self.a0 * s4, self.phase0 + k * per_cycle + within)
    }

    #[cfg(test)]
    pub(crate) fn separation(&self, t: f64) -> f64 {
        self.state(t).0
    }

    #[cfg(test)]
    pub(crate) fn phase(&self, t: f64) -> f64 {
        self.state(t).1
    }

    /// Mass positions about the fixed centre of mass at the origin (SPT-A2).
    pub(crate) fn positions(&self, t: f64) -> [(f64, f64); 2] {
        let (a, ph) = self.state(t);
        let m = self.m1 + self.m2;
        let (r1, r2) = (a * self.m2 / m, a * self.m1 / m);
        let (s, c) = ph.sin_cos_det();
        [(-r1 * c, -r1 * s), (r2 * c, r2 * s)]
    }

    fn masses(&self, t: f64) -> [(f64, f64, f64); 2] {
        let [(x1, y1), (x2, y2)] = self.positions(t);
        [(x1, y1, self.m1), (x2, y2, self.m2)]
    }
}

/// Softened Newtonian potential depth under point masses `(x, y, m)` (SPT-A1).
pub(crate) fn well_depth(x: f64, y: f64, masses: &[(f64, f64, f64)]) -> f64 {
    -WELL_K
        * masses
            .iter()
            .map(|&(mx, my, m)| {
                m / ((x - mx).powi(2) + (y - my).powi(2) + SOFTENING * SOFTENING).sqrt()
            })
            .sum::<f64>()
}

/// Radial wave envelope: `1/√r` far away (2-D energy flux conservation on the sheet),
/// Gaussian-tapered to zero in the near zone.
pub(crate) fn envelope(r: f64) -> f64 {
    let r = r.max(0.0);
    if r < 1e-12 {
        return 0.0;
    }
    (1.0 - (-(r / TAPER).powi(2)).exp()) * (TAPER / r).sqrt()
}

/// Quadrupole wave height at polar `(r, φ)` about the centre of mass (SPT-A3):
/// `A · f(r) · a₀/a(t_r) · cos 2(φ − Φ(t_r))` with retarded time `t_r = t − r/c`. The
/// `a₀/a` factor is the `h ∝ a²Ω² ∝ 1/a` growth of the strain as the orbit shrinks.
/// Reference form; `refresh` evaluates it separably per sample (checked by
/// `spt_mesh_heights_match_the_field`).
#[cfg(test)]
pub(crate) fn wave(bin: &Binary, r: f64, phi: f64, t: f64) -> f64 {
    let (a, ph) = bin.state(t - r / WAVE_SPEED);
    WAVE_AMP * envelope(r) * bin.a0 / a * (2.0 * (phi - ph)).cos_det()
}

/// Line glyph for a screen-space step `(dx, dy)` in cells (y down).
pub(crate) fn slope_glyph(dx: f64, dy: f64) -> Glyph {
    let (ax, ay) = (dx.abs(), dy.abs());
    if ay <= 0.5 * ax {
        H
    } else if ay >= 2.0 * ax {
        V
    } else if (dx > 0.0) != (dy > 0.0) {
        UP
    } else {
        DOWN
    }
}

/// Two perpendicular strokes meeting in one cell read as a crossing.
fn merge(existing: char, g: Glyph) -> Glyph {
    let is = |c: char, a: Glyph| c == a.unicode || c == a.ascii;
    if (g == H && is(existing, V))
        || (g == V && is(existing, H))
        || (is(existing, CROSS) && (g == H || g == V))
    {
        CROSS
    } else if (g == UP && is(existing, DOWN)) || (g == DOWN && is(existing, UP)) {
        XCROSS
    } else {
        g
    }
}

/// Fixed world data of one mesh sample.
#[derive(Debug, Clone, Copy, Default)]
struct Base {
    x: f64,
    y: f64,
    r: f64,
    /// cos 2φ, sin 2φ — the quadrupole's azimuthal pattern, precomputed per resize.
    c2: f64,
    s2: f64,
    env: f64,
}

/// Per-step state of one mesh sample.
#[derive(Debug, Clone, Copy, Default)]
struct Pt {
    sx: f64,
    sy: f64,
    depth: f64,
    well: f64,
    crest: f64,
    envn: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Camera {
    cos_yaw: f64,
    sin_yaw: f64,
    scale: f64,
    ox: f64,
    oy: f64,
}

impl Camera {
    /// Normalised perspective coordinates `(u, v, depth)` of a world point.
    fn view(cos_yaw: f64, sin_yaw: f64, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let (cp, sp) = (PITCH.cos_det(), PITCH.sin_det());
        let (xr, yr) = (x * cos_yaw - y * sin_yaw, x * sin_yaw + y * cos_yaw);
        let d = (yr * cp - z * sp + CAM_DIST).max(0.1);
        let up = yr * sp + z * cp;
        (xr / d, up / d, d)
    }

    fn project(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let (u, v, d) = Self::view(self.cos_yaw, self.sin_yaw, x, y, z);
        (
            self.ox + self.scale * ASPECT * u,
            self.oy - self.scale * v,
            d,
        )
    }
}

#[derive(Debug, Default)]
pub struct SpacetimeGrid {
    seeded: bool,
    bin: Option<Binary>,
    t: f64,
    lx: f64,
    /// Mesh lines along x (`ny` of them, `fx` samples each) and along y (`nx`, `fy`).
    nx: usize,
    ny: usize,
    fx: usize,
    fy: usize,
    base: Vec<Base>,
    pts: Vec<Pt>,
    cam: Camera,
    depth_near: f64,
    depth_far: f64,
    spheres: [(f64, f64, f64); 2],
}

impl SpacetimeGrid {
    fn binary(&self) -> Binary {
        self.bin.unwrap_or(Binary::DEFAULT)
    }

    /// Fit the sheet into the canvas and build the mesh sample lists.
    fn layout(&mut self, size: Size) {
        let (w, h) = (f64::from(size.width()), f64::from(size.height()));
        let extent = |lx: f64| {
            let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for yaw in [-YAW_AMP, 0.0, YAW_AMP] {
                let (s, c) = f64::sin_cos_det(yaw);
                for (x, y, z) in [
                    (-lx, -1.0, 0.0),
                    (lx, -1.0, 0.0),
                    (-lx, 1.0, 0.0),
                    (lx, 1.0, 0.0),
                    (0.0, -0.3, 0.15),
                    (0.0, 0.0, -0.8),
                ] {
                    let (u, v, _) = Camera::view(c, s, x, y, z);
                    (u0, u1, v0, v1) = (u0.min(u), u1.max(u), v0.min(v), v1.max(v));
                }
            }
            (u0, u1, v0, v1)
        };
        // Widen the sheet until its projected aspect matches the canvas.
        self.lx = 1.0;
        for _ in 0..3 {
            let (u0, u1, v0, v1) = extent(self.lx);
            let rho = ASPECT * (u1 - u0) / (v1 - v0);
            self.lx = (self.lx * w / h / rho).clamp(1.0, 2.6);
        }
        let (u0, u1, v0, v1) = extent(self.lx);
        let scale = ((w - 1.0) * 0.97 / (ASPECT * (u1 - u0))).min((h - 1.0) * 0.95 / (v1 - v0));
        self.cam.scale = scale.max(0.0);
        self.cam.ox = (w - 1.0) / 2.0 - self.cam.scale * ASPECT * (u0 + u1) / 2.0;
        self.cam.oy = (h - 1.0) / 2.0 + self.cam.scale * (v0 + v1) / 2.0;
        self.depth_near = Camera::view(1.0, 0.0, 0.0, -1.0, 0.0).2;
        self.depth_far = Camera::view(1.0, 0.0, 0.0, 1.0, 0.0).2;

        self.nx = (usize::from(size.width() / COLS_PER_LINE)).clamp(1, MAX_NX) + 1;
        self.ny = ((h / ROWS_PER_LINE).round() as usize).clamp(1, MAX_NY) + 1;
        self.fx = (self.nx - 1) * SUB_X + 1;
        self.fy = (self.ny - 1) * SUB_Y + 1;
        let lx = self.lx;
        let gx = |i: usize, n: usize| -lx + 2.0 * lx * i as f64 / (n - 1) as f64;
        let gy = |j: usize, n: usize| -1.0 + 2.0 * j as f64 / (n - 1) as f64;
        let sample = |x: f64, y: f64| {
            let r = x.hypot(y);
            let phi = y.atan2(x);
            let (s2, c2) = (2.0 * phi).sin_cos_det();
            Base {
                x,
                y,
                r,
                c2,
                s2,
                env: envelope(r),
            }
        };
        self.base.clear();
        for j in 0..self.ny {
            for i in 0..self.fx {
                self.base.push(sample(gx(i, self.fx), gy(j, self.ny)));
            }
        }
        for i in 0..self.nx {
            for j in 0..self.fy {
                self.base.push(sample(gx(i, self.nx), gy(j, self.fy)));
            }
        }
        self.pts.clear();
        self.pts.resize(self.base.len(), Pt::default());
    }

    /// Evaluate the sheet height and projection of every sample at the current time.
    fn refresh(&mut self) {
        let bin = self.binary();
        let t = self.t;
        let (sy, cy) = (YAW_AMP * (TAU * t / YAW_PERIOD).sin_det()).sin_cos_det();
        self.cam.cos_yaw = cy;
        self.cam.sin_yaw = sy;
        let masses = bin.masses(t);
        let lift = RIM_LIFT * (bin.m1 + bin.m2);
        for (p, b) in self.pts.iter_mut().zip(&self.base) {
            let well = well_depth(b.x, b.y, &masses);
            let (a, ph) = bin.state(t - b.r / WAVE_SPEED);
            let (s, c) = (2.0 * ph).sin_cos_det();
            // cos 2(φ − Φ) = cos 2φ cos 2Φ + sin 2φ sin 2Φ
            let crest = b.c2 * c + b.s2 * s;
            let envn = b.env * bin.a0 / a;
            let z = well + WAVE_AMP * envn * crest;
            let (sx, sy, depth) = self.cam.project(b.x, b.y, z + lift);
            *p = Pt {
                sx,
                sy,
                depth,
                well,
                crest,
                envn,
            };
        }
        for (k, &(x, y, m)) in masses.iter().enumerate() {
            let rad = 0.06 * m.cbrt();
            let z = well_depth(x, y, &masses) + lift + rad;
            let (sx, sy, d) = self.cam.project(x, y, z);
            self.spheres[k] = (sx, sy, (self.cam.scale * rad / d).max(0.4));
        }
    }

    fn ink(&self, a: &Pt, b: &Pt) -> Ink {
        let well = 0.5 * (a.well + b.well);
        let crest = 0.5 * (a.crest + b.crest);
        let envn = 0.5 * (a.envn + b.envn);
        let depth = 0.5 * (a.depth + b.depth);
        let span = (self.depth_far - self.depth_near).max(1e-9);
        let near = ((self.depth_far - depth) / span).clamp(0.0, 1.0);
        let lvl = |v: f64| v.clamp(0.0, 255.0) as u8;
        if well < COLD_DEPTH {
            let f = ((COLD_DEPTH - well) / 0.3).clamp(0.0, 1.0);
            Ink::new(Role::Cold, lvl(110.0 + 145.0 * f))
        } else if crest > CREST && envn > CREST_ENV {
            let f = (crest * envn / 1.2).clamp(0.0, 1.0);
            Ink::new(Role::Wave, lvl(110.0 + 110.0 * f + 35.0 * near))
        } else {
            let shade = 0.8 + 0.2 * (crest * envn.min(1.0));
            Ink::new(Role::Structure, lvl((50.0 + 170.0 * near) * shade))
        }
    }

    fn segment(&self, canvas: &mut Canvas, a: &Pt, b: &Pt) {
        let ink = self.ink(a, b);
        let g = slope_glyph(b.sx - a.sx, b.sy - a.sy);
        let clamp = |v: f64| v.round().clamp(-10_000.0, 10_000.0) as i32;
        let (x0, y0, x1, y1) = (clamp(a.sx), clamp(a.sy), clamp(b.sx), clamp(b.sy));
        let mut pts = geom::line(x0, y0, x1, y1).peekable();
        while let Some((x, y)) = pts.next() {
            let Some(cell) = canvas.get(x, y) else {
                continue;
            };
            // A diagonal Bresenham step gets a diagonal stroke, so gentle curves read as
            // connected instead of as a staircase of offset dashes.
            let here = match pts.peek() {
                Some(&(nx, ny)) if nx != x && ny != y => {
                    slope_glyph(f64::from(nx - x), f64::from(ny - y))
                }
                _ => g,
            };
            let g = merge(cell.ch, here);
            canvas.put(x, y, g, ink);
        }
    }
}

impl Generator for SpacetimeGrid {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seeded = true;
            let mut bin = Binary::DEFAULT;
            bin.phase0 = ctx.rng.random_range(0.0..TAU);
            // Start part-way into the chirp so the first frames already have a wave train.
            self.t = ctx.rng.random_range(0.0..0.6 * bin.cycle());
            self.bin = Some(bin);
        }
        self.layout(size);
        self.refresh();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt.max(0.0);
        self.refresh();
    }

    fn render(&self, canvas: &mut Canvas) {
        let (fx, fy, nx, ny) = (self.fx, self.fy, self.nx, self.ny);
        if self.pts.len() != ny * fx + nx * fy {
            return;
        }
        let xline = |j: usize, i: usize| &self.pts[j * fx + i];
        let yline = |i: usize, j: usize| &self.pts[ny * fx + i * fy + j];
        // Painter's order by depth bands: far (+y) to near (−y), near strokes win.
        for j in (0..ny).rev() {
            if j + 1 < ny {
                for i in 0..nx {
                    for k in j * SUB_Y..(j + 1) * SUB_Y {
                        self.segment(canvas, yline(i, k), yline(i, k + 1));
                    }
                }
            }
            for i in 0..fx - 1 {
                self.segment(canvas, xline(j, i), xline(j, i + 1));
            }
        }
        // The masses, drawn last so they sit visibly in their wells.
        for (k, &(sx, sy, rad)) in self.spheres.iter().enumerate() {
            let role = if k == 0 { Role::Sun } else { Role::Star };
            draw::disc(canvas, sx, sy, rad, |d, _, _| {
                Some((
                    draw::ramp(d, &SPHERE),
                    Ink::new(role, (255.0 * (1.0 - 0.4 * d)) as u8),
                ))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circular() -> Binary {
        Binary {
            inspiral: false,
            phase0: 0.7,
            ..Binary::DEFAULT
        }
    }

    #[test]
    fn spt_a1_single_mass_well_is_radially_symmetric_and_deepens_toward_mass() {
        let (mx, my) = (0.3, -0.2);
        let masses = [(mx, my, 1.0)];
        let mut prev = 0.0f64;
        // Walk inward from r = 2 to r = 0: |z| must strictly increase.
        for k in (0..=200).rev() {
            let r = 0.01 * f64::from(k);
            let ref_z = well_depth(mx + r, my, &masses);
            assert!(ref_z < 0.0, "well points down");
            for j in 0..16 {
                let th = TAU * f64::from(j) / 16.0;
                let z = well_depth(mx + r * th.cos_det(), my + r * th.sin_det(), &masses);
                assert!((z - ref_z).abs() < 1e-12, "not symmetric at r={r} th={th}");
            }
            if k < 200 {
                assert!(ref_z.abs() > prev, "not strictly deeper at r={r}");
            }
            prev = ref_z.abs();
        }
    }

    #[test]
    fn spt_a2_circular_binary_fixed_com_and_constant_separation() {
        let b = circular();
        for k in 0..600 {
            let t = 0.05 * f64::from(k);
            let [(x1, y1), (x2, y2)] = b.positions(t);
            let (cx, cy) = (b.m1 * x1 + b.m2 * x2, b.m1 * y1 + b.m2 * y2);
            assert!(cx.abs() < 1e-12 && cy.abs() < 1e-12, "COM moved at t={t}");
            let sep = (x2 - x1).hypot(y2 - y1);
            assert!((sep - b.a0).abs() < 1e-12, "separation {sep} at t={t}");
            assert!((b.separation(t) - b.a0).abs() < 1e-15);
        }
        // It does orbit: a quarter period later the pair has turned by 90 degrees.
        let [(x1, y1), _] = b.positions(0.0);
        let [(x1q, y1q), _] = b.positions(PI / 2.0 / b.omega0);
        assert!((x1 * x1q + y1 * y1q).abs() < 1e-12);
        // Heavier mass sits closer to the centre of mass.
        let [(x1, y1), (x2, y2)] = b.positions(1.3);
        assert!(x1.hypot(y1) < x2.hypot(y2));
    }

    #[test]
    fn spt_a3_wave_has_azimuthal_period_pi_and_retarded_phase() {
        for b in [
            circular(),
            Binary {
                phase0: 0.3,
                ..Binary::DEFAULT
            },
        ] {
            for &(r, phi, t) in &[
                (0.3, 0.2, 1.0),
                (0.8, 2.5, 7.3),
                (1.7, -1.0, 22.0),
                (2.4, 4.0, 29.5),
                (1.2, 0.9, 61.0),
            ] {
                let w = wave(&b, r, phi, t);
                // Period pi (quadrupole, m = 2) and not period pi/2.
                assert!((wave(&b, r, phi + PI, t) - w).abs() < 1e-12);
                // Phase equals the source's orbital angle at the retarded time t - r/c,
                // read off the actual mass positions.
                let tr = t - r / WAVE_SPEED;
                let [(x1, y1), (x2, y2)] = b.positions(tr);
                let psi = (y2 - y1).atan2(x2 - x1);
                let amp = WAVE_AMP * envelope(r) * b.a0 / b.separation(tr);
                let expect = amp * (2.0 * (phi - psi)).cos_det();
                assert!(
                    (w - expect).abs() < 1e-9,
                    "r={r} phi={phi} t={t}: {w} vs {expect}"
                );
            }
        }
        // Retardation matters: a quarter wave period of delay flips the sign.
        let b = circular();
        let gw_period = PI / b.omega0;
        let r = WAVE_SPEED * gw_period / 2.0;
        let phi = b.phase(3.0);
        assert!(
            wave(&b, r, phi, 3.0) < 0.0,
            "half-wave delay must invert the crest"
        );
    }

    #[test]
    fn spt_envelope_tapers_near_source_and_decays_like_inverse_sqrt() {
        assert!(envelope(0.0).abs() < 1e-12);
        assert!(envelope(0.05) < 0.1 * envelope(0.8));
        let ratio = envelope(8.0) / envelope(2.0);
        assert!((ratio - 0.5).abs() < 0.01, "1/sqrt(r): {ratio}");
    }

    #[test]
    fn spt_inspiral_follows_kepler_and_resets_with_continuous_phase() {
        let b = Binary::DEFAULT;
        // dPhi/dt = Kepler angular speed for the current separation.
        for &t in &[0.5, 10.0, 25.0] {
            let h = 1e-5;
            let omega = (b.phase(t + h) - b.phase(t - h)) / (2.0 * h);
            let kepler = b.omega0 * (b.a0 / b.separation(t)).powf(1.5);
            assert!((omega - kepler).abs() < 1e-6 * kepler, "t={t}");
            assert!(b.separation(t + 0.1) < b.separation(t), "shrinks");
        }
        // Separation reaches q_end then resets; phase stays continuous across the reset.
        let cycle = b.tau_c * (1.0 - b.q_end.powi(4));
        assert!((b.separation(cycle - 1e-9) - b.q_end * b.a0).abs() < 1e-6);
        assert!((b.separation(cycle + 1e-9) - b.a0).abs() < 1e-6);
        assert!((b.phase(cycle + 1e-9) - b.phase(cycle - 1e-9)).abs() < 1e-6);
        // COM stays fixed while inspiralling too.
        let [(x1, y1), (x2, y2)] = b.positions(17.0);
        assert!((b.m1 * x1 + b.m2 * x2).abs() < 1e-12 && (b.m1 * y1 + b.m2 * y2).abs() < 1e-12);
    }

    #[test]
    fn spt_mesh_heights_match_the_field() {
        let mut g = SpacetimeGrid::default();
        let mut ctx = Ctx::new(3, Size::new(120, 40));
        g.resize(Size::new(120, 40), &mut ctx);
        for _ in 0..30 {
            g.update(SIM_DT, &mut ctx);
        }
        let bin = g.binary();
        let masses = bin.masses(g.t);
        for idx in (0..g.pts.len()).step_by(37) {
            let b = g.base[idx];
            let expect = well_depth(b.x, b.y, &masses) + wave(&bin, b.r, b.y.atan2(b.x), g.t);
            let p = g.pts[idx];
            let z = p.well + WAVE_AMP * p.envn * p.crest;
            assert!((z - expect).abs() < 1e-9, "sample {idx}");
        }
    }

    #[test]
    fn spt_mesh_resolution_adapts_and_stays_a_wireframe() {
        let lines = |w, h| {
            let mut g = SpacetimeGrid::default();
            let mut ctx = Ctx::new(1, Size::new(w, h));
            g.resize(Size::new(w, h), &mut ctx);
            (g.nx, g.ny)
        };
        let (small, big) = (lines(80, 24), lines(200, 50));
        assert!(big.0 > small.0 && big.1 > small.1, "{small:?} vs {big:?}");
        // Not a solid smear: a clear majority of cells stay blank.
        let mut stage = Stage::new(build(), 42, Size::new(100, 30), GlyphMode::Unicode);
        for _ in 0..60 {
            stage.step(SIM_DT);
        }
        let frame = stage.render();
        let lit = frame.cells().iter().filter(|c| c.ch != ' ').count();
        assert!(lit * 100 < frame.cells().len() * 60, "{lit} lit cells");
        let has = |r: Role| frame.cells().iter().any(|c| c.ink.role == r);
        assert!(has(Role::Sun) && has(Role::Star) && has(Role::Cold) && has(Role::Wave));
        assert!(has(Role::Structure));
    }

    #[test]
    fn spt_slope_glyphs() {
        assert_eq!(slope_glyph(4.0, 0.5), H);
        assert_eq!(slope_glyph(0.2, 3.0), V);
        assert_eq!(slope_glyph(2.0, -2.0), UP);
        assert_eq!(slope_glyph(-2.0, 2.0), UP);
        assert_eq!(slope_glyph(2.0, 2.0), DOWN);
        assert_eq!(merge('─', V), CROSS);
        assert_eq!(merge('/', DOWN), XCROSS);
        assert_eq!(merge(' ', H), H);
    }
}
