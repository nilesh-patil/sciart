//! V838 Monocerotis: a stellar flash echoing off surrounding dust (SPEC-002 `light_echo`).
//!
//! Light scattered towards us at time `t` after the flash's direct light arrived left the
//! star at `t_e = t − Δ/c`, where `Δ = √(ρ² + z²) − z` is the extra path via the dust
//! point at sky radius `ρ` and depth `z` (towards us): the scattering surface is the
//! paraboloid `z = ρ²/(2ct) − ct/2`. The dust is four static, rippled sheets in front of
//! the star; each node's path lag `Δ` and geometric weight (density, Henyey–Greenstein
//! phase function, dilution) are fixed per resize. Lengths are in lattice half-heights.
//!
//! M16 (strange_attractors' thread idiom): each echo is drawn only on its crest, as a
//! braille thread one dot thick. The light curve's crests are three instants `t_p`, so
//! the crest of (sheet, flash, peak) is the level set `Δ(x, y) = c·(t + age − t_p)` of the
//! sheet's lag field: `prepare` traces it with marching squares over the lattice and draws
//! each piece as a dot line. Brightness (weight × `L(t_p)` × the flash's weight × the
//! exposure) decides whether a piece is drawn and its step; colour is the flash's age.

use rand::Rng;
use sciart_core::grid::lattice_for;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01, value_noise};

use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "light_echo",
    category: Category::DeepCosmos,
    description: "A stellar outburst lighting up surrounding dust in rings that sweep outward",
    title: "Light echo · flash lights dust",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(LightEcho::default())
}

/// Speed of light in half-heights per second.
pub(crate) const C: f64 = 0.034;
/// Loop: dark before the flash, then the echo (s). Loop time 0 is the flash. M16: 14 s
/// (was 21.5 s), so the previous flashes' rings are still on screen, not past its edge,
/// and no stretch of the loop is empty (ECHO-A10).
const PRE: f64 = 0.5;
const ECHO: f64 = 13.5;
pub(crate) const LOOP: f64 = PRE + ECHO;
/// The outburst's duration (s) and its three peaks: time, width, height.
pub(crate) const BURST: f64 = 6.0;
/// M14 (ADR-0017): sharp peaks, so each lit sheet is a thin shell.
const PEAKS: [(f64, f64, f64); 3] = [(0.7, 0.2, 1.0), (2.4, 0.25, 0.7), (4.2, 0.3, 0.8)];
pub(crate) const N_PEAKS: usize = PEAKS.len();
/// Outburst colour temperature: hot at first, cooling towards a red supergiant.
const T_HOT: f64 = 8_000.0;
const T_COOL: f64 = 3_000.0;
const COOL_TAU: f64 = 2.2;
/// Dust sheets: depths in front of the star, ripple amplitude, and the dust-free cavity.
const SHEETS: [f64; 4] = [0.12, 0.3, 0.55, 0.85];
pub(crate) const N_SHEETS: usize = SHEETS.len();
const RIPPLE: f64 = 0.14;
const CAVITY: f64 = 0.22;
/// Henyey–Greenstein asymmetry (forward scattering).
const G: f64 = 0.6;
/// Lattice cap (ADR-0012).
const MAX_NODES: usize = 24_000;
/// Flashes whose echoes are drawn: age offset (s) and weight. The newest is weight 1;
/// the ones before keep expanding, fading, while the next flash lights up.
const GENERATIONS: [(f64, f32); 3] = [(0.0, 1.0), (LOOP, 0.45), (2.0 * LOOP, 0.35)];
const N_TARGETS: usize = GENERATIONS.len() * N_PEAKS;
/// Light-curve table (the star's flash).
const LUT: usize = 512;
/// Exposure of the scattered light, like a camera's auto-exposure: `update` estimates the
/// peak scattering weight lit now, on every `EXPOSE_STRIDE`-th lattice node (a fixed
/// subsample), with the light curve at its envelope, and eases the gain toward
/// `EXPOSE·WHITE / peak` with time constant `EXPOSE_TAU` in simulated seconds, so it is
/// smooth in time and independent of the render cadence (GEN-R20, ECHO-A8); the gain is
/// clamped to `ECHO_GAIN`.
const WHITE: f32 = 2.2;
const EXPOSE: f32 = 6.0;
const ECHO_GAIN: (f32, f32) = (1.0, 6.0);
const EXPOSE_STRIDE: usize = 13;
const EXPOSE_TAU: f64 = 1.5;
/// A crest piece's exposed brightness `u` (1 ≈ the brightest node lit now): below
/// `U_MIN` it is not drawn; up to `U_FULL` a growing share of pieces is drawn (a fixed
/// per-node dither, so dim crests are sparser threads and the pattern holds still as the
/// ring passes); from `U_SUN` the newest flash's crest is `Sun`.
const U_MIN: f32 = 0.012;
const U_FULL: f32 = 0.06;
const U_SUN: f32 = 0.3;
/// Echo weight fades out beyond this sky radius (half-heights) over `VIGNETTE_SPAN`.
const VIGNETTE_AT: f64 = 1.3;
const VIGNETTE_SPAN: f64 = 0.6;
/// Inverse-square dilution, softened (stylised): weight `∝ 1 / r^DILUTION`.
const DILUTION: f64 = 1.0;
/// The star (ECHO-A9): one white dot; while the light curve exceeds `FLASH_FROM` a cross
/// of dots whose arms (dots) grow with `L`, and a `Sun` ring of radius `FLASH_RING`.
const STAR: Ink = Ink::new(Role::Star, 255);
const FLASH: Ink = Ink::new(Role::Sun, 230);
const FLASH_FROM: f32 = 0.3;
const FLASH_ARM: (f64, f64) = (4.0, 6.0);
const FLASH_RING: f64 = 3.0;

/// The ink of a crest of age rank `rank` (0 newest) at exposed brightness `u` (M16:
/// colour by age, warm → cool as the generations recede).
pub(crate) fn generation_ink(rank: usize, u: f32) -> Ink {
    match rank {
        0 if u >= U_SUN => Ink::new(Role::Sun, 230),
        0 => Ink::new(Role::Hot, 190),
        1 => Ink::new(Role::Accent, 150),
        _ => Ink::new(Role::Cold, 130),
    }
}

/// `(L, T)` at `t_e` from the table `curve` (`LUT` entries over `[0, BURST]`), linearly
/// interpolated; 0 outside the burst.
fn lerp_curve(curve: &[(f32, f32)], te: f64) -> (f32, f32) {
    if !(0.0..=BURST).contains(&te) || curve.len() < LUT {
        return (0.0, 0.0);
    }
    let x = te / BURST * (LUT - 1) as f64;
    let i = (x as usize).min(LUT - 2);
    let f = (x - i as f64) as f32;
    let (a, b) = (curve[i], curve[i + 1]);
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

/// Light curve `L(t_e)` of the outburst, 0 outside `[0, BURST]`.
pub(crate) fn light_curve(te: f64) -> f64 {
    if !(0.0..=BURST).contains(&te) {
        return 0.0;
    }
    let taper = 1.0 - ease::smoothstep((te - (BURST - 1.2)) / 1.2);
    PEAKS
        .iter()
        .map(|&(at, w, h)| h * (-((te - at) / w).powi(2)).exp())
        .sum::<f64>()
        * taper
        * ease::smoothstep(te / 0.25)
}

/// The light curve's crests `(t_p, L(t_p))`: its local maxima, one per peak (the taper
/// and the neighbouring peaks shift them slightly off the peak centres).
pub(crate) fn crests() -> [(f64, f64); N_PEAKS] {
    const STEP: f64 = 1e-3;
    let mut out = PEAKS.map(|(at, _, _)| (at, light_curve(at)));
    for (c, &(at, w, _)) in out.iter_mut().zip(&PEAKS) {
        // Hill-climb from the centre within the peak's width.
        let mut te = at;
        loop {
            let (l, lo, hi) = (
                light_curve(te),
                light_curve(te - STEP),
                light_curve(te + STEP),
            );
            if hi > l && te + STEP <= at + w {
                te += STEP;
            } else if lo > l && te - STEP >= at - w {
                te -= STEP;
            } else {
                *c = (te, l);
                break;
            }
        }
    }
    out
}

/// Colour temperature of the outburst at `t_e` (K), non-increasing (ECHO-A4).
pub(crate) fn temperature(te: f64) -> f64 {
    T_COOL + (T_HOT - T_COOL) * (-te.max(0.0) / COOL_TAU).exp()
}

/// Extra light path via dust at sky radius `rho` and depth `z` (ECHO-A1).
pub(crate) fn lag(rho: f64, z: f64) -> f64 {
    (rho * rho + z * z).sqrt() - z
}

/// Sky radius of the echo from a flat sheet at depth `z0` at time `t` after the flash:
/// `ρ = √(ct (ct + 2 z0))` (ECHO-A2).
#[cfg(test)]
pub(crate) fn ring_radius(t: f64, z0: f64) -> f64 {
    let d = C * t.max(0.0);
    (d * (d + 2.0 * z0)).sqrt()
}

fn henyey_greenstein(cos: f64) -> f64 {
    (1.0 - G * G) / (1.0 + G * G - 2.0 * G * cos).powf(1.5)
}

/// Per node and sheet: path lag (half-heights) and scattering weight.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Sample {
    lag: f32,
    weight: f32,
}

/// One dust seed's sheets over the lattice.
#[derive(Debug, Default)]
struct Dust {
    samples: Vec<[Sample; N_SHEETS]>,
}

impl Dust {
    fn build(&mut self, salt: u64, lattice: Size) {
        let (w, h) = (usize::from(lattice.width()), usize::from(lattice.height()));
        let half = h as f64 / 2.0;
        self.samples.clear();
        for j in 0..h {
            for i in 0..w {
                let x = (i as f64 + 0.5 - w as f64 / 2.0) / half;
                let y = (j as f64 + 0.5 - half) / half;
                let rho = x.hypot(y);
                let cavity = ease::smoothstep((rho - CAVITY * 0.5) / CAVITY);
                let vignette = 1.0 - ease::smoothstep((rho - VIGNETTE_AT) / VIGNETTE_SPAN);
                let mut cell = [Sample::default(); N_SHEETS];
                for (k, (s, &z0)) in cell.iter_mut().zip(&SHEETS).enumerate() {
                    let sk = salt ^ (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                    // Swirly density: domain-warped value noise, with lanes and voids.
                    let (u, v) = (x * 3.0, y * 3.0);
                    let wx = value_noise(sk ^ 1, u, v, 1.3) - 0.5;
                    let wy = value_noise(sk ^ 2, u, v, 1.3) - 0.5;
                    let n = 0.6 * value_noise(sk, u + 1.6 * wx, v + 1.6 * wy, 0.9)
                        + 0.4 * value_noise(sk ^ 3, u + 2.2 * wy, v - 2.2 * wx, 0.4);
                    let density = ((n - 0.38) / 0.62).clamp(0.0, 1.0).powf(1.6) * cavity;
                    let z = z0 + RIPPLE * (value_noise(sk ^ 4, x * 1.6, y * 1.6, 1.0) - 0.5);
                    let r2 = rho * rho + z * z;
                    s.lag = lag(rho, z) as f32;
                    s.weight = (vignette * density * henyey_greenstein(z / r2.sqrt())
                        / r2.powf(DILUTION / 2.0)) as f32;
                }
                self.samples.push(cell);
            }
        }
    }
}

/// One crest lit now: the lag it sits at, its brightness factor (`L(t_p)` × the flash's
/// weight) and its age rank.
#[derive(Debug, Clone, Copy, Default)]
struct Target {
    lag: f32,
    bright: f32,
    /// Its age rank among the flashes that have happened (0 newest), which picks its ink:
    /// before loop time 0 the previous flash is still the newest, so the rings change
    /// colour at the flash, not in the dark before it.
    rank: usize,
}

#[derive(Debug, Default)]
pub struct LightEcho {
    seeded: bool,
    /// Loop time: `−PRE` at the loop's start, 0 at the flash.
    t: f64,
    salt: u64,
    dust: Dust,
    /// Lattice nodes across and down.
    lattice: (usize, usize),
    /// `L` and `T` tabulated over `[0, BURST]`.
    curve: Vec<(f32, f32)>,
    crests: [(f64, f64); N_PEAKS],
    /// The smoothed exposure gain (see `EXPOSE`).
    gain: f32,
    dots: Dots,
    /// The raster is behind the simulation: `prepare` redraws it (GEN-R20).
    dirty: bool,
}

impl LightEcho {
    fn seed(&mut self, ctx: &mut Ctx) {
        self.seeded = true;
        self.salt = ctx.rng.random();
        self.curve = (0..LUT)
            .map(|i| {
                let te = BURST * i as f64 / (LUT - 1) as f64;
                (light_curve(te) as f32, temperature(te) as f32)
            })
            .collect();
        self.crests = crests();
        self.t = ctx.rng.random_range(4.0..8.0);
    }

    /// `L` and `T` at `t_e`, linearly interpolated in the table (0 outside the burst).
    fn curve_at(&self, te: f64) -> (f32, f32) {
        lerp_curve(&self.curve, te)
    }

    /// The exposure gain the echo at time `t` asks for: from the peak scattering weight
    /// lit now over a fixed subsample of nodes, at the light curve's envelope.
    fn target_gain(&self) -> f32 {
        let t = self.t;
        let mut peak = 0.0f32;
        for cell in self.dust.samples.iter().step_by(EXPOSE_STRIDE) {
            let mut sum = 0.0f32;
            for &(age, weight) in &GENERATIONS {
                for s in cell {
                    let te = t + age - f64::from(s.lag) / C;
                    if (0.0..=BURST).contains(&te) && s.weight > 0.0 {
                        sum += s.weight * weight;
                    }
                }
            }
            peak = peak.max(sum);
        }
        if peak > 0.0 {
            (EXPOSE * WHITE / peak).clamp(ECHO_GAIN.0, ECHO_GAIN.1)
        } else {
            ECHO_GAIN.1
        }
    }

    /// The crests lit now, every (flash, peak) pair whose crest has left the star.
    fn targets(&self) -> ([Target; N_TARGETS], usize) {
        let mut out = [Target::default(); N_TARGETS];
        let mut n = 0;
        let shift = usize::from(self.t < 0.0);
        for (gen_, &(age, weight)) in GENERATIONS.iter().enumerate() {
            for &(tp, lp) in &self.crests {
                let lag = C * (self.t + age - tp);
                if lag > 0.0 {
                    out[n] = Target {
                        lag: lag as f32,
                        bright: lp as f32 * weight,
                        rank: gen_.saturating_sub(shift),
                    };
                    n += 1;
                }
            }
        }
        (out, n)
    }

    /// Trace every lit crest over every sheet with marching squares and draw it as dot
    /// lines: a ring one dot thick (ECHO-A7).
    fn rings(&mut self) {
        let (lw, lh) = self.lattice;
        if lw < 2 || lh < 2 || self.dust.samples.len() < lw * lh {
            return;
        }
        let ds = self.dots.dot_size();
        let sx = f64::from(ds.width()) / lw as f64;
        let sy = f64::from(ds.height()) / lh as f64;
        let (targets, n) = self.targets();
        let targets = &targets[..n];
        if targets.is_empty() {
            return;
        }
        let max_bright = targets.iter().fold(0.0f32, |m, t| m.max(t.bright));
        // u = gain · brightness / (EXPOSE · WHITE): 1 at the exposure's white point.
        let expose = self.gain / (EXPOSE * WHITE);
        let samples = &self.dust.samples;
        let dots = &mut self.dots;
        let salt = self.salt;
        for j in 0..lh - 1 {
            for i in 0..lw - 1 {
                let i0 = j * lw + i;
                let quad = [
                    &samples[i0],
                    &samples[i0 + 1],
                    &samples[i0 + lw + 1],
                    &samples[i0 + lw],
                ];
                for k in 0..N_SHEETS {
                    let l = quad.map(|c| c[k].lag);
                    let w = 0.25 * quad.iter().map(|c| c[k].weight).sum::<f32>();
                    let base = w * expose;
                    if base * max_bright < U_MIN {
                        continue;
                    }
                    let (lo, hi) = (
                        l[0].min(l[1]).min(l[2].min(l[3])),
                        l[0].max(l[1]).max(l[2].max(l[3])),
                    );
                    // A fixed dither per node and sheet: dim crests are drawn on a
                    // share of the nodes that does not change as the ring moves on.
                    let dither = hash01(
                        salt ^ cell_key(i as i32, j as i32)
                            ^ (k as u64 + 1).wrapping_mul(0xd1b5_4a32_d192_ed03),
                    ) as f32;
                    for tg in targets {
                        if tg.lag < lo || tg.lag >= hi {
                            continue;
                        }
                        let u = base * tg.bright;
                        if u < U_MIN || (u < U_FULL && dither > (u - U_MIN) / (U_FULL - U_MIN)) {
                            continue;
                        }
                        let ink = generation_ink(tg.rank, u);
                        contour(dots, i, j, sx, sy, l, tg.lag, ink);
                    }
                }
            }
        }
    }

    fn raster(&mut self) {
        self.dots.clear();
        self.rings();
        self.star();
    }

    /// The star (ECHO-A9): a white dot, and while the flash is bright a starburst.
    fn star(&mut self) {
        let (l, _) = self.curve_at(self.t);
        let ds = self.dots.dot_size();
        let (cx, cy) = (i32::from(ds.width()) / 2, i32::from(ds.height()) / 2);
        if l > FLASH_FROM {
            let k = f64::from(((l - FLASH_FROM) / (1.0 - FLASH_FROM)).clamp(0.0, 1.0));
            let arm = (FLASH_ARM.0 + (FLASH_ARM.1 - FLASH_ARM.0) * k).round() as i32;
            // Inner ring off the axes, so its cells are not the spikes' (one ink per cell).
            let r = FLASH_RING.ceil() as i32 + 1;
            for dy in -r..=r {
                for dx in -r..=r {
                    let d = f64::from(dx).hypot(f64::from(dy));
                    if dx != 0 && dy != 0 && (d - FLASH_RING).abs() <= 0.5 {
                        self.dots.set(cx + dx, cy + dy, FLASH);
                    }
                }
            }
            for a in 1..=arm {
                for (dx, dy) in [(a, 0), (-a, 0), (0, a), (0, -a)] {
                    self.dots.set(cx + dx, cy + dy, STAR);
                }
            }
        }
        self.dots.set(cx, cy, STAR);
    }
}

/// Draw the level set `lag = level` inside lattice quad `(i, j)` (corner lags `l` in the
/// order (i, j), (i+1, j), (i+1, j+1), (i, j+1)) as dot lines; `(sx, sy)` dots per node.
#[allow(clippy::too_many_arguments)]
fn contour(
    dots: &mut Dots,
    i: usize,
    j: usize,
    sx: f64,
    sy: f64,
    l: [f32; 4],
    level: f32,
    ink: Ink,
) {
    const CORNERS: [(f64, f64); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let mut pts = [(0.0f64, 0.0f64); 4];
    let mut n = 0;
    for e in 0..4 {
        let (a, b) = (e, (e + 1) % 4);
        if (l[a] < level) != (l[b] < level) {
            let f = f64::from((level - l[a]) / (l[b] - l[a]));
            let (pa, pb) = (CORNERS[a], CORNERS[b]);
            let (x, y) = (pa.0 + (pb.0 - pa.0) * f, pa.1 + (pb.1 - pa.1) * f);
            // Node centres in dot units (`Dots::from_cells`' convention, node = cell).
            pts[n] = (
                (i as f64 + x + 0.5) * sx - 0.5,
                (j as f64 + y + 0.5) * sy - 0.5,
            );
            n += 1;
        }
    }
    // Two crossings: one piece; four (a saddle): pair them in edge order.
    for p in pts[..n].chunks_exact(2) {
        dots.line(p[0].0, p[0].1, p[1].0, p[1].1, ink);
    }
}

impl Generator for LightEcho {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        if !self.seeded {
            self.seed(ctx);
        }
        self.dots.resize(size);
        // Square nodes: one per half-block pixel (2 dots), capped (ADR-0012).
        let ds = self.dots.dot_size();
        let px = Size::new(
            ds.width().div_ceil(2).max(1),
            ds.height().div_ceil(2).max(1),
        );
        let lattice = lattice_for(px, MAX_NODES, 1.0);
        self.lattice = (usize::from(lattice.width()), usize::from(lattice.height()));
        self.dust.build(self.salt, lattice);
        // The exposure starts where the light is, with no ramp after a resize.
        self.gain = self.target_gain();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = dt.max(0.0);
        self.t += dt;
        while self.t >= ECHO {
            self.t -= LOOP;
        }
        let k = 1.0 - (-dt / EXPOSE_TAU).exp();
        self.gain += (self.target_gain() - self.gain) * k as f32;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: Size) -> (LightEcho, Ctx) {
        let mut ctx = Ctx::new(42, size);
        let mut g = LightEcho::default();
        g.resize(size, &mut ctx);
        g.prepare();
        (g, ctx)
    }

    /// Jump to loop time `t` with the exposure settled there (`update` eases it).
    fn jump(g: &mut LightEcho, t: f64) {
        g.t = t;
        g.gain = g.target_gain();
    }

    /// Loop times every 0.5 s over one loop.
    fn loop_times() -> impl Iterator<Item = f64> {
        (0..(LOOP / 0.5) as usize).map(|i| -PRE + 0.5 * i as f64)
    }

    /// Lit echo dots (the star not drawn).
    fn echo_dots(g: &mut LightEcho, t: f64) -> Vec<(i32, i32)> {
        jump(g, t);
        g.dots.clear();
        g.rings();
        let ds = g.dots.dot_size();
        let mut out = Vec::new();
        for y in 0..i32::from(ds.height()) {
            for x in 0..i32::from(ds.width()) {
                if g.dots.is_lit(x, y) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn echo_a1_scattering_points_lie_on_the_paraboloid() {
        for &t in &[0.5, 3.0, 12.0, 30.0] {
            let ct = C * t;
            for i in 0..50 {
                let rho = f64::from(i) * 0.05;
                let z = rho * rho / (2.0 * ct) - ct / 2.0;
                assert!(
                    (lag(rho, z) - ct).abs() < 1e-9 * ct.max(1.0),
                    "t={t} ρ={rho}"
                );
            }
        }
    }

    #[test]
    fn echo_a2_rings_grow_apparently_faster_than_light_at_first() {
        let z0 = 0.3;
        let mut last = 0.0;
        for i in 1..=400 {
            let t = f64::from(i) * 0.1;
            let r = ring_radius(t, z0);
            assert!(r > last);
            last = r;
            // The ring is where the lag equals ct.
            assert!((lag(r, z0) - C * t).abs() < 1e-9);
        }
        let speed = (ring_radius(0.2, z0) - ring_radius(0.1, z0)) / 0.1;
        assert!(speed > C, "early apparent speed {speed} vs c {C}");
    }

    #[test]
    fn echo_a3_the_dust_never_moves() {
        let (mut g, mut ctx) = built(Size::new(120, 40));
        let samples = g.dust.samples.clone();
        for _ in 0..600 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(g.dust.samples, samples);
    }

    #[test]
    fn echo_a4_colour_is_the_outburst_temperature_at_emission() {
        let mut last = f64::INFINITY;
        for i in 0..=300 {
            let te = BURST * f64::from(i) / 300.0;
            let t = temperature(te);
            assert!(t <= last);
            last = t;
        }
        // The crests sit at the light curve's local maxima, one per peak, so a ring's
        // leading crest is the hottest light.
        let c = crests();
        for (k, &(tp, lp)) in c.iter().enumerate() {
            assert!((tp - PEAKS[k].0).abs() < PEAKS[k].1, "crest {k} at {tp}");
            assert!(light_curve(tp - 0.01) <= lp && light_curve(tp + 0.01) <= lp);
            if k > 0 {
                assert!(temperature(tp) < temperature(c[k - 1].0));
            }
        }
    }

    #[test]
    fn echo_a5_nothing_is_lit_by_a_flash_before_it_happens() {
        // Before loop time 0 the newest flash has not happened: every node's emission
        // time for it is negative, so it adds nothing; only older echoes can show.
        let (mut g, mut ctx) = built(Size::new(80, 24));
        while g.t > -PRE + 0.1 {
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        while g.t < 0.0 {
            for cell in &g.dust.samples {
                for s in cell {
                    assert_eq!(g.curve_at(g.t - f64::from(s.lag) / C).0, 0.0);
                }
            }
            // Only the two older flashes' crests are lit.
            assert!(g.targets().1 <= 2 * N_PEAKS);
            g.update(SIM_DT, &mut ctx);
            g.prepare();
        }
        assert_eq!(lerp_curve(&g.curve, -1e-9), (0.0, 0.0));
    }

    #[test]
    fn echo_a6_inks_by_age_newest_dominating() {
        let size = Size::new(120, 40);
        let (mut g, _) = built(size);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        let (cx, cy) = (60, 20);
        let mut by_gen = [0usize; 3];
        for t in loop_times() {
            jump(&mut g, t);
            g.raster();
            canvas.clear();
            g.render(&mut canvas);
            for y in 0..40 {
                for x in 0..120 {
                    let cell = canvas.get(x, y).unwrap();
                    if cell.ink == Ink::BLANK || cell.ch == ' ' {
                        continue;
                    }
                    let near = (x - cx).abs() <= 4 && (y - cy).abs() <= 4;
                    let (r, l) = (cell.ink.role, cell.ink.level);
                    match (r, l) {
                        (Role::Star, 255) => {}
                        (Role::Sun, 230) | (Role::Hot, 190) => by_gen[0] += 1,
                        (Role::Accent, 150) => by_gen[1] += 1,
                        (Role::Cold, 130) => by_gen[2] += 1,
                        _ => panic!("{:?} at {x},{y} t={t} (near star: {near})", cell.ink),
                    }
                }
            }
        }
        assert!(by_gen.iter().all(|&n| n > 20), "{by_gen:?}");
        assert!(by_gen[0] > by_gen[1] && by_gen[0] > by_gen[2], "{by_gen:?}");
    }

    #[test]
    fn echo_a7_rings_are_thin_threads() {
        let (mut g, _) = built(Size::new(120, 40));
        let ds = g.dots.dot_size();
        let (cx, cy) = (f64::from(ds.width()) / 2.0, f64::from(ds.height()) / 2.0);
        let (mut lit, mut thin, mut specks) = (0usize, 0usize, 0usize);
        for t in loop_times() {
            for (x, y) in echo_dots(&mut g, t) {
                lit += 1;
                let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
                let r = dx.hypot(dy).max(1e-9);
                let (ux, uy) = (dx / r, dy / r);
                let across = (-2..=2)
                    .filter(|&k| {
                        let k = f64::from(k);
                        g.dots.is_lit(
                            (f64::from(x) + k * ux).round() as i32,
                            (f64::from(y) + k * uy).round() as i32,
                        )
                    })
                    .count();
                thin += usize::from(across <= 3);
                let alone = (-1..=1)
                    .flat_map(|a| (-1..=1).map(move |b| (a, b)))
                    .filter(|&(a, b)| (a, b) != (0, 0))
                    .all(|(a, b)| !g.dots.is_lit(x + a, y + b));
                specks += usize::from(alone);
            }
        }
        let (thin, specks) = (thin as f64 / lit as f64, specks as f64 / lit as f64);
        assert!(thin >= 0.85, "thin share {thin:.3}");
        assert!(specks <= 0.025, "specks {specks:.3}");
    }

    #[test]
    fn echo_a8_exposure_is_smooth_in_time_and_cadence_free() {
        // M14: the gain eases with time constant `EXPOSE_TAU` in simulated time: one step
        // moves it by at most `1 − e^{−dt/τ}` of the way, and stepping 1/60 s or 1/30 s
        // lands on the same gain within a few per cent.
        let size = Size::new(120, 40);
        let (mut a, mut ca) = built(size);
        let (mut b, mut cb) = built(size);
        let k = 1.0 - (-SIM_DT / EXPOSE_TAU).exp();
        for _ in 0..600 {
            let (g0, target) = (a.gain, a.target_gain());
            a.update(SIM_DT, &mut ca);
            let bound = (f64::from((a.target_gain() - g0).abs())
                .max(f64::from((target - g0).abs()))
                * k) as f32;
            assert!((a.gain - g0).abs() <= bound + 1e-4, "{g0} -> {}", a.gain);
        }
        for _ in 0..300 {
            b.update(2.0 * SIM_DT, &mut cb);
        }
        let rel = (a.gain - b.gain).abs() / a.gain.max(1e-6);
        assert!(rel < 0.05, "{} vs {}", a.gain, b.gain);
    }

    #[test]
    fn echo_a9_the_flash_is_a_visible_starburst() {
        let (mut g, _) = built(Size::new(120, 40));
        let ds = g.dots.dot_size();
        let (cx, cy) = (i32::from(ds.width()) / 2, i32::from(ds.height()) / 2);
        let burst = |g: &LightEcho| {
            let (mut n, mut x0, mut x1, mut y0, mut y1) = (0, cx, cx, cy, cy);
            for y in cy - 20..=cy + 20 {
                for x in cx - 20..=cx + 20 {
                    if g.dots.is_lit(x, y) {
                        n += 1;
                        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                    }
                }
            }
            (n, x1 - x0 + 1, y1 - y0 + 1)
        };
        for (tp, _) in crests() {
            for dt in [-0.1, 0.0, 0.1] {
                g.t = tp + dt;
                g.dots.clear();
                g.star();
                let (n, w, h) = burst(&g);
                assert!(n >= 9, "t={}: {n} dots", g.t);
                // ≥ 2 cells each way: 4 dots across, 8 down.
                assert!(w >= 4 && h >= 8, "t={}: {w}×{h} dots", g.t);
            }
        }
        for t in [-0.4, 1.4, 3.3, 8.0, 12.0] {
            g.t = t;
            assert!(g.curve_at(t).0 < FLASH_FROM);
            g.dots.clear();
            g.star();
            assert_eq!(burst(&g).0, 1, "t={t}");
        }
    }

    #[test]
    fn echo_a10_no_dead_stretch() {
        let (mut g, _) = built(Size::new(120, 40));
        let counts: Vec<usize> = loop_times().map(|t| echo_dots(&mut g, t).len()).collect();
        let mean = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        for (t, &n) in loop_times().zip(&counts) {
            assert!(n as f64 >= 0.35 * mean, "t={t}: {n} dots vs mean {mean:.0}");
        }
    }

    #[test]
    fn echo_a11_no_horizontal_stripes() {
        let (mut g, _) = built(Size::new(120, 40));
        let ds = g.dots.dot_size();
        let (w, h) = (i32::from(ds.width()), i32::from(ds.height()));
        let (mut lit, mut hrun, mut vrun) = (0usize, 0usize, 0usize);
        // Dots in runs of ≥ 6 along one axis.
        let runs = |g: &LightEcho, outer: i32, inner: i32, at: &dyn Fn(i32, i32) -> (i32, i32)| {
            let mut total = 0;
            for o in 0..outer {
                let mut run = 0;
                for i in 0..=inner {
                    let (x, y) = at(o, i);
                    if i < inner && g.dots.is_lit(x, y) {
                        run += 1;
                    } else {
                        if run >= 6 {
                            total += run;
                        }
                        run = 0;
                    }
                }
            }
            total
        };
        for t in loop_times() {
            lit += echo_dots(&mut g, t).len();
            hrun += runs(&g, h, w, &|o, i| (i, o));
            vrun += runs(&g, w, h, &|o, i| (o, i));
        }
        let (hs, vs) = (hrun as f64 / lit as f64, vrun as f64 / lit as f64);
        assert!(
            hs <= 1.5 * vs,
            "horizontal runs {hs:.3} vs vertical {vs:.3}"
        );
    }
}
