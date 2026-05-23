//! Shell patterns: a shell grows at its lip and paints itself as it goes; a reaction
//! along the lip lays down pigment, and as rows accumulate, oblique lines, tents and
//! stripes appear (SPEC-002 `seashell_pigmentation`).
//!
//! Meinhardt's activator–inhibitor model (*The Algorithmic Beauty of Sea Shells*, eq. 2.1)
//! on the one-dimensional growing edge, periodic, explicit Euler with unit steps:
//!
//! ```text
//! a' = s(a²/(b(1 + s_a a²)) + b_a) − r_a a + D_a a''
//! b' = s a² − r_b b + b_b
//! ```
//!
//! The inhibitor is long-lived and does not diffuse, so an activated cell leaves a
//! refractory wake: activation travels as waves that annihilate when they meet. Each
//! growth step appends the lip's activator as a new row of the shell: time becomes the
//! shell's length. The regimes differ in `b_b` and `b_a`:
//!
//! - **waves**: an excitable edge seeded with one-way pulses, which run round the lip for
//!   ever and draw parallel oblique lines;
//! - **tents**: a lower `b_b` lets the edge re-ignite by itself at random places; each
//!   ignition spreads both ways and meets its neighbours: triangles (as on *Conus* and
//!   *Oliva*);
//! - **stripes**: fast activator diffusion makes each ignition sweep the whole lip at
//!   once: bands parallel to the lip.

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::{ease, ink};

pub static META: Meta = Meta {
    name: "seashell_pigmentation",
    category: Category::Biochemistry,
    description: "A shell grows and paints itself: Meinhardt activator-inhibitor waves along the lip draw lines, tents and stripes",
    title: "Shell patterns · pigment waves",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 3] = ["waves", "tents", "stripes"];

pub fn build() -> Box<dyn Generator> {
    Box::new(Shell::default())
}

// ---- Model (SPEC-002 Model) ----

/// Model constants of one regime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Params {
    pub(crate) ra: f32,
    pub(crate) rb: f32,
    pub(crate) ba: f32,
    pub(crate) bb: f32,
    pub(crate) da: f32,
    pub(crate) sa: f32,
    /// Relative spread of the source density `s` along the lip.
    pub(crate) noise: f32,
    /// Spontaneous ignitions per row per 100 lip cells (a local fluctuation that
    /// crosses threshold).
    pub(crate) ignite: f32,
    /// Pigment memory kept per row (0 keeps none).
    pub(crate) memory: f32,
}

pub(crate) const WAVES: Params = Params {
    ra: 0.1,
    rb: 0.014,
    ba: 0.0,
    bb: 0.0015,
    da: 0.015,
    sa: 0.0,
    noise: 0.03,
    ignite: 0.0,
    memory: MEM_DECAY,
};
/// The excitable lip of `WAVES`, re-igniting at random.
pub(crate) const TENTS: Params = Params {
    ignite: 0.2,
    ..WAVES
};
/// A lower `b_b` and a basal activator production make the resting lip unstable: it
/// fires all at once whenever the inhibitor has decayed (a relaxation oscillation).
pub(crate) const STRIPES: Params = Params {
    ba: 0.01,
    bb: 0.0003,
    rb: 0.025,
    memory: 0.2,
    ..WAVES
};
const REGIMES: [Params; 3] = [WAVES, TENTS, STRIPES];

/// Lip cells at most.
pub(crate) const MAX_CELLS: usize = 400;
/// Model iterations per growth step (row).
pub(crate) const ITERS_PER_ROW: usize = 36;
/// Activator ceiling, a guard only (SHL-A1).
const A_MAX: f32 = 10.0;
/// Rows painted at once when a shell starts.
const PREFILL_MAX: usize = 40;
/// Seconds per regime (one shell each).
const REGIME_S: f64 = 20.0;
/// Rows the lip lays down per second at most, and the time a shell takes to fill.
const ROWS_PER_S_MAX: f64 = 14.0;
const FILL_S: f64 = 12.0;

/// The lip: activator, inhibitor and source density per cell.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lip {
    pub(crate) a: Vec<f32>,
    pub(crate) b: Vec<f32>,
    s: Vec<f32>,
    na: Vec<f32>,
    /// Pigment memory per cell (what the newest row shows).
    pub(crate) mem: Vec<f32>,
    pub(crate) p: Params,
}

impl Default for Params {
    fn default() -> Self {
        WAVES
    }
}

impl Lip {
    /// A resting lip of `n` cells for regime `p`.
    #[cfg(test)]
    pub(crate) fn new(n: usize, p: Params, rng: &mut impl Rng) -> Self {
        let mut lip = Self::default();
        lip.reset(n, p, rng);
        lip
    }

    /// Back to rest with `n` cells and regime `p` (reusing buffers).
    pub(crate) fn reset(&mut self, n: usize, p: Params, rng: &mut impl Rng) {
        let n = n.clamp(1, MAX_CELLS);
        self.p = p;
        self.a.clear();
        self.a.resize(n, 0.0);
        self.b.clear();
        self.b.resize(n, p.bb / p.rb);
        self.na.clear();
        self.na.resize(n, 0.0);
        self.mem.clear();
        self.mem.resize(n, 0.0);
        self.s.clear();
        self.s
            .extend((0..n).map(|_| p.ra * (1.0 + p.noise * rng.random_range(-1.0f32..=1.0))));
    }

    /// Light a pulse at `i` that can only run towards higher indices: the cells behind it
    /// are made refractory.
    pub(crate) fn one_way_pulse(&mut self, i: usize) {
        let n = self.a.len();
        for k in 0..3 {
            self.a[(i + k) % n] = 2.0;
        }
        for k in 1..=5 {
            self.b[(i + n - k % n) % n] = 0.5;
        }
    }

    /// Light a two-way pulse at `i`.
    pub(crate) fn pulse(&mut self, i: usize) {
        let n = self.a.len();
        for k in 0..3 {
            self.a[(i + k) % n] = 2.0;
        }
    }

    /// One Euler step (inhibitor local: `D_b = 0`).
    pub(crate) fn step(&mut self) {
        let n = self.a.len();
        let p = self.p;
        for i in 0..n {
            let (l, r) = (self.a[(i + n - 1) % n], self.a[(i + 1) % n]);
            let (a, b, s) = (self.a[i], self.b[i], self.s[i]);
            let aa = a * a;
            let prod = s * (aa / (b * (1.0 + p.sa * aa)) + p.ba);
            self.na[i] = (a + prod - p.ra * a + p.da * (l + r - 2.0 * a)).clamp(0.0, A_MAX);
            self.b[i] = b + s * aa - p.rb * b + p.bb;
        }
        std::mem::swap(&mut self.a, &mut self.na);
    }

    /// Local activator maxima above `min` (periodic).
    #[cfg(test)]
    pub(crate) fn peaks(&self, min: f32) -> usize {
        let n = self.a.len();
        (0..n)
            .filter(|&i| {
                let a = self.a[i];
                a > min && a >= self.a[(i + n - 1) % n] && a > self.a[(i + 1) % n]
            })
            .count()
    }
}

/// Pigment memory kept per row (see `Shell::grow_row`).
const MEM_DECAY: f32 = 0.7;

/// Pigment laid at a lip cell: where the activator is (the wave), and fading along the
/// refractory wake the inhibitor leaves (so a line has a soft trailing edge, and a
/// triangle between two diverging waves fills in).
#[inline]
pub(crate) fn pigment(a: f32, b: f32) -> f32 {
    a.max(1.4 * (b - 0.3))
}

// ---- Shell ----

#[derive(Debug)]
struct Shell {
    lip: Lip,
    /// Pigment rows, a ring buffer of `rows_cap` rows of `lip.a.len()` values; row
    /// `newest` was laid last.
    rows: Vec<f32>,
    rows_cap: usize,
    newest: usize,
    /// Rows laid on the current shell (≤ `rows_cap` shown).
    laid: usize,
    /// Rows laid in total (SHL-A4).
    total_rows: u64,
    time: f64,
    owed: f64,
    row_s: f64,
    regime: usize,
    timeline: Timeline,
    pixels: Pixels,
    /// Shell geometry in pixels: centre x, body top, body height, half-width.
    body: (f64, f64, f64, f64),
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            lip: Lip::default(),
            rows: Vec::new(),
            rows_cap: 1,
            newest: 0,
            laid: 0,
            total_rows: 0,
            time: 0.0,
            owed: 0.0,
            row_s: 0.1,
            regime: usize::MAX,
            timeline: Timeline::new(&[
                (CAPTIONS[0], REGIME_S),
                (CAPTIONS[1], REGIME_S),
                (CAPTIONS[2], REGIME_S),
            ]),
            pixels: Pixels::default(),
            body: (0.0, 0.0, 1.0, 1.0),
        }
    }
}

/// Half-width of the body at fraction `t` of its height from the shoulder (0) to the
/// growing lip (1): a cone shell, full at the shoulder, blunt at the lip.
fn body_half(t: f64) -> f64 {
    if !(0.0..=1.0).contains(&t) {
        return 0.0;
    }
    let shoulder = ease::smoothstep((t / 0.12).min(1.0));
    // A cone, full at the shoulder and blunt at the lip, so the newest rows (at the
    // lip) are always wide enough to see.
    (0.35 + 0.65 * shoulder) * (1.0 - 0.6 * t.powf(1.5))
}

impl Shell {
    /// Start a new shell for the regime at `index`.
    fn new_shell(&mut self, index: usize, rng: &mut impl Rng) {
        self.regime = index;
        let n = self.lip.a.len().max(1);
        self.lip.reset(n, REGIMES[index], rng);
        match index {
            0 => {
                // Evenly spaced one-way pulses: parallel oblique lines.
                let k = (n / 50).clamp(1, 8);
                for j in 0..k {
                    self.lip
                        .one_way_pulse(j * n / k + rng.random_range(0..(n / k / 4).max(1)));
                }
            }
            _ => {
                for _ in 0..(n / 80).max(1) {
                    self.lip.pulse(rng.random_range(0..n));
                }
            }
        }
        self.rows.fill(0.0);
        self.laid = 0;
        // The shell arrives half-painted, so a new regime shows its pattern at once;
        // bounded, so the one-off cost stays small at any size.
        for _ in 0..(self.rows_cap / 2).min(PREFILL_MAX) {
            self.grow_row(rng);
        }
    }

    fn grow_row(&mut self, rng: &mut impl Rng) {
        let n = self.lip.a.len();
        let ignite = f64::from(self.lip.p.ignite) * n as f64 / 100.0;
        if ignite > 0.0 && rng.random_bool(ignite.min(1.0)) {
            self.lip.pulse(rng.random_range(0..n));
        }
        for _ in 0..ITERS_PER_ROW {
            self.lip.step();
        }
        self.newest = (self.newest + 1) % self.rows_cap;
        let row = &mut self.rows[self.newest * n..][..n];
        // The mantle keeps depositing where it recently did: pigment memory decays over a
        // few rows, so a line gets a shaded trailing band and a tent a filled flank.
        let lip = &mut self.lip;
        for ((m, &a), &b) in lip.mem.iter_mut().zip(&lip.a).zip(&lip.b) {
            *m = (*m * lip.p.memory).max(pigment(a, b));
        }
        row.copy_from_slice(&lip.mem);
        self.laid += 1;
        self.total_rows += 1;
    }

    fn rasterise(&mut self) {
        let (cx, top, height, half) = self.body;
        let n = self.lip.a.len();
        let (rows, cap, newest, laid) = (&self.rows, self.rows_cap, self.newest, self.laid);
        let spire_h = 0.16 * height;
        // The shell grows over its regime: the lip advances, carrying the newest rows
        // (the pattern stays attached to the shell, laid down at the lip).
        let grown = (self.time % REGIME_S) / REGIME_S;
        let length = height * (0.72 + 0.28 * grown);
        self.pixels.shade(|x, y| {
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            // The spire: a small stepped cone above the shoulder.
            if fy < top && fy >= top - spire_h {
                let t = (top - fy) / spire_h;
                let hw = half * 0.42 * (1.0 - t);
                let whorl = f64::from(u8::from((t * 4.0).fract() < 0.18));
                if (fx - cx).abs() <= hw {
                    let lvl = 150.0 - 70.0 * whorl - 40.0 * ((fx - cx).abs() / hw.max(1.0));
                    return Ink::new(Role::Ramp(Ramp::Starlight), ink::level(lvl));
                }
                return Ink::BLANK;
            }
            let t = (fy - top) / length;
            let hw = half * body_half(t);
            let u = (fx - cx) / hw.max(1e-6);
            if !(0.0..=1.0).contains(&t) || u.abs() > 1.0 {
                return Ink::BLANK;
            }
            // Round body: light falls off towards the silhouette.
            let shade = (1.0 - u * u).sqrt();
            // Edge coverage: the fraction of this pixel inside the silhouette, so the
            // outline fades over one pixel instead of stepping.
            let cover = ((1.0 - u.abs()) * hw).clamp(0.0, 1.0);
            let light = (0.45 + 0.55 * shade) * (0.35 + 0.65 * cover);
            // Rows from the lip (bottom) upward, newest first.
            let back = ((1.0 - t) * length) as usize;
            let pigment = if back < laid.min(cap) {
                let r = (newest + cap - back) % cap;
                // The lip wraps round the body: cell across the visible half.
                let cell = (((u + 1.0) * 0.5) * n as f64) as usize;
                rows[r * n + cell.min(n - 1)]
            } else {
                0.0
            };
            if pigment > 0.25 {
                // The wine-to-violet part of the ramp: the purple-brown of Oliva porphyria,
                // darkest (wine) in the fresh line, lighter violet in the fading band.
                let k = ease::smoothstep((f64::from(pigment) - 0.25) / 0.9);
                Ink::new(
                    Role::Ramp(Ramp::Ferroin),
                    ink::level((175.0 - 100.0 * k) * light),
                )
            } else {
                Ink::new(Role::Ramp(Ramp::Starlight), ink::level(200.0 * light))
            }
        });
    }
}

fn ascii_cell(top: Ink, bottom: Ink) -> Option<(char, Ink)> {
    let lit = |i: Ink| i.role != Role::Background;
    let pig = |i: Ink| i.role == Role::Ramp(Ramp::Ferroin);
    match (lit(top), lit(bottom)) {
        (false, false) => None,
        _ if pig(top) => Some(('#', top)),
        _ if pig(bottom) => Some(('#', bottom)),
        (true, _) => Some(('.', top)),
        (false, true) => Some(('.', bottom)),
    }
}

impl Generator for Shell {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.time).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.pixels.resize(size);
        let px = self.pixels.pixel_size();
        let (pw, ph) = (f64::from(px.width()), f64::from(px.height()));
        // The shell stands in the art rows above the title row.
        let usable = (ph - 3.0).max(2.0);
        let height = usable * 0.8;
        let top = usable * 0.17;
        let half = (height * 0.36).min(pw * 0.45).max(1.0);
        self.body = (pw / 2.0, top, height, half);
        // The lip spans the body's widest front (edge regridded to the width).
        let n = ((6.0 * half).round() as usize).clamp(8, MAX_CELLS);
        self.rows_cap = (height.round() as usize).max(1);
        self.rows.clear();
        self.rows.resize(self.rows_cap * n, 0.0);
        self.newest = 0;
        self.row_s = (FILL_S / self.rows_cap as f64).max(1.0 / ROWS_PER_S_MAX);
        let index = self.timeline.at(self.time).index;
        self.lip.reset(n, REGIMES[index], &mut ctx.rng);
        self.new_shell(index, &mut ctx.rng);
        self.rasterise();
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        self.time += dt;
        let index = self.timeline.at(self.time).index;
        if index != self.regime {
            self.new_shell(index, &mut ctx.rng);
        }
        self.owed += dt / self.row_s;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            self.grow_row(&mut ctx.rng);
        }
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.pixels
            .blit(canvas, |_, _, top, bottom| ascii_cell(top, bottom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak_pos(lip: &Lip) -> usize {
        (0..lip.a.len())
            .max_by(|&i, &j| lip.a[i].total_cmp(&lip.a[j]))
            .unwrap_or(0)
    }

    fn row(lip: &mut Lip) {
        for _ in 0..ITERS_PER_ROW {
            lip.step();
        }
    }

    #[test]
    fn shl_a1_concentrations_stay_non_negative_and_finite() {
        let mut rng = SplitMix64::new(1);
        for p in REGIMES {
            let mut lip = Lip::new(200, p, &mut rng);
            lip.pulse(10);
            lip.pulse(120);
            for _ in 0..400 {
                row(&mut lip);
                for (&a, &b) in lip.a.iter().zip(&lip.b) {
                    assert!(a.is_finite() && b.is_finite() && a >= 0.0 && b > 0.0);
                }
            }
        }
    }

    #[test]
    fn shl_a2_a_wave_moves_a_constant_offset_per_row() {
        let mut rng = SplitMix64::new(2);
        let mut lip = Lip::new(300, WAVES, &mut rng);
        lip.one_way_pulse(20);
        for _ in 0..10 {
            row(&mut lip);
        }
        let mut last = peak_pos(&lip);
        let mut steps = Vec::new();
        for _ in 0..30 {
            row(&mut lip);
            let p = peak_pos(&lip);
            steps.push((p + 300 - last) % 300);
            last = p;
        }
        let mean = steps.iter().sum::<usize>() as f64 / steps.len() as f64;
        assert!(mean >= 0.5, "the wave moves: {steps:?}");
        for s in &steps {
            assert!(
                (*s as f64 - mean).abs() <= 1.0,
                "constant offset: {steps:?}"
            );
        }
    }

    #[test]
    fn shl_a3_colliding_waves_annihilate() {
        let mut rng = SplitMix64::new(3);
        let mut lip = Lip::new(200, TENTS, &mut rng);
        lip.pulse(60);
        // Let it split into two waves, then watch them meet on the far side.
        for _ in 0..8 {
            row(&mut lip);
        }
        assert_eq!(lip.peaks(0.5), 2, "two waves");
        let mut met = false;
        for _ in 0..80 {
            row(&mut lip);
            if lip.peaks(0.5) == 0 {
                met = true;
                break;
            }
        }
        assert!(met, "the waves annihilated");
    }

    #[test]
    fn shl_a4_one_row_per_growth_step() {
        let mut ctx = Ctx::new(4, Size::new(80, 24));
        let mut g = Shell::default();
        g.resize(Size::new(80, 24), &mut ctx);
        for _ in 0..40 {
            let (before, owed) = (g.total_rows, g.owed);
            let dt = 0.1;
            g.update(dt, &mut ctx);
            let expected = (owed + dt / g.row_s).floor() as u64;
            assert_eq!(g.total_rows - before, expected);
            if expected > 0 {
                // The newest row is the lip as it stands after that growth step.
                let n = g.lip.a.len();
                assert_eq!(&g.rows[g.newest * n..][..n], &g.lip.mem[..]);
            }
        }
        assert!(g.total_rows > 0);
    }

    #[test]
    fn shl_a5_captions_follow_the_regime() {
        let mut ctx = Ctx::new(5, Size::new(80, 24));
        let mut g = Shell::default();
        g.resize(Size::new(80, 24), &mut ctx);
        for (k, c) in CAPTIONS.iter().enumerate() {
            while g.time < k as f64 * REGIME_S + 1.0 {
                g.update(0.25, &mut ctx);
            }
            assert_eq!(g.caption(), Some(*c));
            assert_eq!(g.lip.p, REGIMES[k]);
        }
    }
}
