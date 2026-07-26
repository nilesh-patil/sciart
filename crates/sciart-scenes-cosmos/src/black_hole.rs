//! A black hole with a beamed, lensed accretion disk (SPEC-002 `black_hole`).
//!
//! Lengths inside the model are in units of the shadow radius `rs` (so the physics is
//! size-independent); the screen transform multiplies by `rs` in rows and stretches x by
//! `ASPECT`. The disk lies in a plane inclined 78° to the sky: a point at disk radius `r`
//! and azimuth `φ` (φ = 0 to the right, φ = π/2 straight behind the hole) projects to
//! `(r cos φ, −r sin φ cos i)` (screen y grows downward, so the far side is above) and
//! sits a depth `r sin φ sin i` behind the hole.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::{self, ASPECT};
use sciart_core::particles::{kepler_omega, plot};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "black_hole",
    category: Category::DeepCosmos,
    description: "A black hole with a beamed, lensed accretion disk and photon ring",
    title: "Black hole · infalling gas glows",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(BlackHole::default())
}

const SHADOW_FRAC: f64 = 0.14;
const DISK_FRAC: f64 = 0.95;
/// Inner disk edge (ISCO) and outer edge in shadow radii.
const ISCO: f64 = 2.2;
const R_OUT: f64 = DISK_FRAC / SHADOW_FRAC;
const ISCO_PERIOD: f64 = 2.5;
const INCLINATION: f64 = 78.0 * PI / 180.0;
/// Viscous inward drift `dr/dt = −DRIFT r` outside the ISCO, and the much faster plunge
/// inside it (1/s).
const DRIFT: f64 = 0.02;
const PLUNGE: f64 = 1.4;
/// Beaming strength in `(1 + BEAM β)³`.
const BEAM: f64 = 0.55;
/// Schwarzschild radius per shadow radius: the shadow of a Schwarzschild hole has radius
/// `(3√3/2) R_s`, so `R_s = 2/(3√3) rs`.
const SCHWARZSCHILD: f64 = 0.384_900_179_459_750_5;
/// Streak length in seconds of orbital motion (shows the differential rotation).
const STREAK: f64 = 0.12;
/// Images displaced more than this (in rs) by lensing count as the "lensed arc".
const ARC_SHIFT: f64 = 0.4;

/// Streaky glyphs: the disk is seen nearly edge-on, so orbital motion is mostly horizontal.
const DISK: [Glyph; 5] = [
    Glyph::new('·', '.'),
    Glyph::ascii('-'),
    Glyph::ascii('~'),
    Glyph::ascii('='),
    Glyph::new('≡', '#'),
];
const RING: Glyph = Glyph::new('•', 'o');
const STAR: Glyph = Glyph::new('·', '.');

type V2 = (f64, f64);

/// One parcel of disk gas.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mote {
    /// Radius in shadow radii.
    r: f64,
    phi: f64,
    /// Per-parcel brightness variation in `[0.55, 1]`.
    glow: f64,
}

/// `GM` in shadow-radius units chosen so the ISCO period is exactly `ISCO_PERIOD` (BH-A1).
pub(crate) fn gm() -> f64 {
    // ω² r³ = GM with ω = 2π / T at r = ISCO.
    let w = TAU / ISCO_PERIOD;
    w * w * ISCO * ISCO * ISCO
}

/// Keplerian angular velocity at radius `r` (rs units) (BH-A1).
pub(crate) fn omega(r: f64) -> f64 {
    kepler_omega(gm(), r)
}

/// Line-of-sight velocity as a fraction of the ISCO orbital speed; positive = approaching
/// (BH-A3). The gas orbits towards increasing φ, so the left side (φ = π) approaches.
pub(crate) fn beta_los(r: f64, phi: f64) -> f64 {
    // Velocity rω(−sin φ, cos φ) in (sky-x, depth); only the depth part, foreshortened
    // by sin i, lies along the line of sight, and moving to greater depth recedes.
    let v_isco = ISCO * omega(ISCO);
    (-r * omega(r) * phi.cos_det() * INCLINATION.sin_det() / v_isco).clamp(-1.0, 1.0)
}

/// Relativistic beaming factor `(1 + 0.55 β)³` used for shading (BH-A3).
pub(crate) fn beaming(beta: f64) -> f64 {
    (1.0 + BEAM * beta).powi(3)
}

/// Sky-plane offset (rs units, y down) of a disk point.
pub(crate) fn project(r: f64, phi: f64) -> V2 {
    (
        r * phi.cos_det(),
        -r * phi.sin_det() * INCLINATION.cos_det(),
    )
}

/// How far behind the hole's sky plane a disk point lies (rs units; > 0 = far side).
pub(crate) fn depth(r: f64, phi: f64) -> f64 {
    r * phi.sin_det() * INCLINATION.sin_det()
}

/// Thin-lens images `[primary, secondary]` of a far-side point at sky offset `p` and
/// `depth` behind the hole, or `None` if the point is in front of it (BH-A2).
///
/// Weak-field point lens with the source `depth` behind the lens and a distant observer:
/// Einstein radius `θE² = 2 R_s · depth`, images `θ± = (β ± √(β² + 4θE²))/2` along the
/// lens→source direction. This is what bends the far side of the disk up over (and
/// faintly under) the shadow; at the disk's sides `β ≫ θE` and the image barely moves.
pub(crate) fn lens_images(p: V2, depth: f64) -> Option<[V2; 2]> {
    if depth <= 0.0 || !depth.is_finite() {
        return None;
    }
    let te2 = 2.0 * SCHWARZSCHILD * depth;
    let beta = p.0.hypot(p.1);
    // Exactly behind the hole the direction is degenerate; pick the ring's top.
    let (ux, uy) = if beta > 1e-12 {
        (p.0 / beta, p.1 / beta)
    } else {
        (0.0, -1.0)
    };
    let plus = 0.5 * (beta + (beta * beta + 4.0 * te2).sqrt());
    // θ+ θ− = −θE² avoids the cancellation of the "−" root for β ≫ θE.
    let minus = -te2 / plus;
    Some([(ux * plus, uy * plus), (ux * minus, uy * minus)])
}

/// Brightness in `[0, 1]`: radial emissivity × beaming, dimming through the plunge (BH-A3).
pub(crate) fn intensity(m: &Mote) -> f64 {
    let emissivity = if m.r >= ISCO {
        (ISCO / m.r).powf(1.2)
    } else {
        // Plunging gas fades as it falls towards the horizon.
        ((m.r - 1.0) / (ISCO - 1.0)).clamp(0.0, 1.0)
    };
    let beam = beaming(beta_los(m.r, m.phi)) / beaming(1.0);
    (m.glow * emissivity * beam).clamp(0.0, 1.0)
}

fn level(i: f64) -> u8 {
    (40.0 + 215.0 * i.clamp(0.0, 1.0).powf(0.55)) as u8
}

#[derive(Debug)]
pub struct BlackHole {
    cx: f64,
    cy: f64,
    /// Shadow radius in rows.
    rs: f64,
    motes: ParticlePool<Mote>,
    /// Background stars, normalized `[0, 1)²`.
    stars: Vec<V2>,
}

impl Default for BlackHole {
    fn default() -> Self {
        Self {
            cx: 0.0,
            cy: 0.0,
            rs: 0.0,
            motes: ParticlePool::with_capacity(0),
            stars: Vec::new(),
        }
    }
}

fn pool_size(size: Size) -> usize {
    (size.area() / 6).clamp(24, 1600)
}

fn spawn_uniform_log(rng: &mut impl Rng) -> Mote {
    // Steady state of `dr/dt = −DRIFT r` fed at R_OUT is uniform in ln r.
    let r = (ISCO.ln() + rng.random_range(0.0..1.0) * (R_OUT / ISCO).ln()).exp();
    Mote {
        r,
        phi: rng.random_range(0.0..TAU),
        glow: rng.random_range(0.55..1.0),
    }
}

impl BlackHole {
    fn to_cell(&self, p: V2) -> V2 {
        (self.cx + p.0 * self.rs * ASPECT, self.cy + p.1 * self.rs)
    }

    /// Draw one sample of a mote (its head or a streak point).
    fn draw_point(&self, canvas: &mut Canvas, r: f64, phi: f64, lvl: f64, far_pass: bool) {
        let d = depth(r, phi);
        let p = project(r, phi);
        if far_pass {
            let Some([primary, secondary]) = lens_images(p, d) else {
                return;
            };
            let beta = p.0.hypot(p.1);
            let rp = primary.0.hypot(primary.1);
            // Anything imaged inside the shadow was captured (BH-A2).
            if rp > 1.0 {
                let shift = rp - beta;
                let k = if shift > ARC_SHIFT { 0.8 } else { 1.0 };
                self.put(canvas, primary, lvl * k);
            }
            if secondary.0.hypot(secondary.1) > 1.05 {
                self.put(canvas, secondary, lvl * 0.35);
            }
        } else if d <= 0.0 {
            self.put(canvas, p, lvl);
        }
    }

    fn put(&self, canvas: &mut Canvas, p: V2, i: f64) {
        let (x, y) = self.to_cell(p);
        plot(
            canvas,
            x,
            y,
            draw::ramp(i.sqrt(), &DISK),
            Ink::new(Role::Hot, level(i)),
        );
    }

    fn draw_disk(&self, canvas: &mut Canvas, far_pass: bool) {
        // Tail first so the bright head wins a shared cell. Lensing magnifies the far side
        // (spreading its gas over a larger arc), so it gets denser streaks to keep the
        // arc's surface brightness comparable to the direct disk's.
        const NEAR: [(f64, f64); 3] = [(1.0, 0.45), (0.5, 0.7), (0.0, 1.0)];
        const FAR: [(f64, f64); 5] = [
            (1.0, 0.45),
            (0.75, 0.55),
            (0.5, 0.7),
            (0.25, 0.85),
            (0.0, 1.0),
        ];
        let samples: &[(f64, f64)] = if far_pass { &FAR } else { &NEAR };
        for m in &self.motes {
            let i = intensity(m);
            let w = omega(m.r) * STREAK;
            for &(k, fade) in samples {
                self.draw_point(canvas, m.r, m.phi - w * k, i * fade, far_pass);
            }
        }
    }
}

impl Generator for BlackHole {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.cx = f64::from(size.width()) / 2.0;
        self.cy = f64::from(size.height()) / 2.0;
        self.rs = draw::fit_radius(size, SHADOW_FRAC);
        let n = pool_size(size);
        self.motes.set_capacity(n);
        while self.motes.len() < n {
            self.motes.spawn(spawn_uniform_log(&mut ctx.rng));
        }
        if self.stars.is_empty() {
            self.stars = (0..90)
                .map(|_| {
                    (
                        ctx.rng.random_range(0.0..1.0),
                        ctx.rng.random_range(0.0..1.0),
                    )
                })
                .collect();
        }
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        // Both motions are integrated exactly, so any dt is stable.
        let (outer, inner) = ((-DRIFT * dt).exp(), (-PLUNGE * dt).exp());
        for m in self.motes.iter_mut() {
            m.phi = geom::wrap(m.phi + omega(m.r) * dt, TAU);
            m.r *= if m.r >= ISCO { outer } else { inner };
            if m.r < 1.0 {
                // Swallowed: respawn at the outer edge (BH-A4, pool size unchanged).
                m.r = R_OUT * ctx.rng.random_range(0.97..1.0);
                m.phi = ctx.rng.random_range(0.0..TAU);
                m.glow = ctx.rng.random_range(0.55..1.0);
            }
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        let (w, h) = (
            f64::from(canvas.size().width()),
            f64::from(canvas.size().height()),
        );
        for &(u, v) in &self.stars {
            let (x, y) = (u * w, v * h);
            let (dx, dy) = ((x - self.cx) / (ASPECT * self.rs), (y - self.cy) / self.rs);
            if dx * dx + dy * dy > 1.6 * 1.6 {
                plot(canvas, x, y, STAR, Ink::new(Role::Star, 70));
            }
        }
        // Far side (lensed) → photon ring → near side, which passes in front of the shadow.
        self.draw_disk(canvas, true);
        draw::orbit(
            canvas,
            self.cx,
            self.cy,
            self.rs,
            RING,
            Ink::new(Role::Accent, 235),
        );
        self.draw_disk(canvas, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(size: Size) -> (BlackHole, Ctx) {
        let mut ctx = Ctx::new(5, size);
        let mut g = BlackHole::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn bh_a1_omega_decreases_with_radius_and_isco_period_is_2_5_s() {
        let mut last = f64::INFINITY;
        for k in 0..=200 {
            let r = 1.0 + f64::from(k) * (R_OUT - 1.0) / 200.0;
            let w = omega(r);
            assert!(w < last && w > 0.0, "r={r} ω={w}");
            last = w;
        }
        assert!((TAU / omega(ISCO) - ISCO_PERIOD).abs() < 1e-12);
        // Kepler's third law: ω² r³ = GM everywhere.
        for r in [1.0, ISCO, 4.0, R_OUT] {
            assert!((omega(r).powi(2) * r.powi(3) - gm()).abs() < 1e-9 * gm());
        }
    }

    #[test]
    fn bh_a2_far_side_is_never_drawn_inside_the_shadow() {
        // Direct check of the thin-lens images: both lie on the line through the hole,
        // the primary always outside the Einstein radius.
        for k in 0..64 {
            let phi = 0.05 + PI * 0.9 * f64::from(k) / 64.0;
            for r in [ISCO, 3.0, 5.0, R_OUT] {
                let (p, d) = (project(r, phi), depth(r, phi));
                assert!(d > 0.0);
                let [a, b] = lens_images(p, d).unwrap();
                let te2 = 2.0 * SCHWARZSCHILD * d;
                let beta = p.0.hypot(p.1);
                let (ra, rb) = (a.0.hypot(a.1), b.0.hypot(b.1));
                assert!(
                    ra >= beta && ra * ra >= te2 * (1.0 - 1e-12),
                    "r={r} φ={phi}"
                );
                assert!((ra * rb - te2).abs() < 1e-9 * te2.max(1e-9));
                assert!(a.0 * b.0 + a.1 * b.1 <= 0.0, "images on opposite sides");
            }
        }
        // Near side has no lensed images.
        assert!(lens_images(project(3.0, -1.0), depth(3.0, -1.0)).is_none());

        // Render only far-side gas: nothing may appear inside the shadow.
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = scene(size);
        for _ in 0..30 {
            g.update(SIM_DT, &mut ctx);
        }
        g.motes
            .retain(|m| depth(m.r, m.phi) > 0.0 && depth(m.r, m.phi - omega(m.r) * STREAK) > 0.0);
        assert!(g.motes.len() > 100);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let mut behind = 0;
        for m in &g.motes {
            let p = project(m.r, m.phi);
            if p.0.hypot(p.1) < 0.8 {
                behind += 1;
            }
        }
        assert!(behind > 0, "test needs gas directly behind the hole");
        for y in 0..50 {
            for x in 0..200 {
                let dx = (f64::from(x) - g.cx) / (ASPECT * g.rs);
                let dy = (f64::from(y) - g.cy) / g.rs;
                if dx.hypot(dy) < 1.0 - 0.75 / g.rs {
                    assert_eq!(c.get(x, y).unwrap().ink.role, Role::Background, "({x},{y})");
                }
            }
        }
    }

    #[test]
    fn bh_a3_approaching_half_is_brighter() {
        assert!(beaming(0.5) > beaming(0.0) && beaming(0.0) > beaming(-0.5));
        assert!((beaming(0.0) - 1.0).abs() < 1e-15);
        assert!(beta_los(3.0, PI) > 0.0 && beta_los(3.0, 0.0) < 0.0);
        assert!(beta_los(3.0, PI / 2.0).abs() < 1e-12);
        // Seeded ChaCha8 stream (the Ctx RNG).
        let mut r = Ctx::new(11, Size::new(1, 1)).rng;
        let (mut app, mut rec, mut na, mut nr) = (0.0, 0.0, 0, 0);
        for _ in 0..20_000 {
            let m = spawn_uniform_log(&mut r);
            let i = intensity(&m);
            assert!((0.0..=1.0).contains(&i));
            if beta_los(m.r, m.phi) > 0.0 {
                app += i;
                na += 1;
            } else {
                rec += i;
                nr += 1;
            }
        }
        let (app, rec) = (app / f64::from(na), rec / f64::from(nr));
        assert!(app > 2.0 * rec, "approaching {app} vs receding {rec}");

        // And on screen: the left (approaching) half carries more light.
        let size = Size::new(200, 50);
        let (g, _) = scene(size);
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        g.render(&mut c);
        let (mut left, mut right) = (0u64, 0u64);
        for y in 0..50 {
            for x in 0..200 {
                let cell = c.get(x, y).unwrap();
                if cell.ink.role == Role::Hot {
                    if f64::from(x) < g.cx {
                        left += u64::from(cell.ink.level);
                    } else {
                        right += u64::from(cell.ink.level);
                    }
                }
            }
        }
        assert!(left > right, "left {left} right {right}");
    }

    #[test]
    fn bh_a4_pool_size_is_constant_after_warm_up() {
        let (mut g, mut ctx) = scene(Size::new(200, 50));
        let n = g.motes.len();
        assert_eq!(n, g.motes.capacity());
        let mut swallowed = 0;
        for k in 0..3000 {
            let dt = if k % 7 == 0 { 0.25 } else { SIM_DT };
            let before: f64 = g.motes.iter().map(|m| m.r).sum();
            g.update(dt, &mut ctx);
            let after: f64 = g.motes.iter().map(|m| m.r).sum();
            if after > before {
                swallowed += 1;
            }
            assert_eq!(g.motes.len(), n);
            assert!(g.motes.iter().all(|m| (1.0..=R_OUT).contains(&m.r)));
        }
        assert!(
            swallowed > 0,
            "gas must actually be swallowed and respawned"
        );
    }
}
