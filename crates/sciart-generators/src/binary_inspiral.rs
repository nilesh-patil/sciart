//! Two black holes spiral together and merge (SPEC-002 `binary_inspiral`).
//!
//! Model: a Newtonian equal-mass binary shrinking under quadrupole (Peters) radiation
//! reaction, in model units (`G = M_total-scaled`, separation `a₀ = 1`, scene seconds):
//! `a⁴ = a₀⁴ − 4βt`, orbital phase `Φ` from Kepler `ω = √(M/a³)` in closed form, the
//! gravitational wave at twice the orbital phase with amplitude `∝ ω^{2/3}`. At contact a
//! short plunge and merger, then a single damped ringdown mode, then calm; ~20 s a loop.
//!
//! Look (GEN-R26, ADR-0020): the face-on plus strain `h₊ ∝ A(t − r/c)·cos(2θ − 2Φ(t − r/c))/r`
//! as fixed-threshold stipple (`Hot` where `h₊ > 0`, `Cold` where `h₊ < 0`), read at the
//! retarded time from a ring buffer of `(Φ, A)` samples, so the two arms carry the past
//! chirp outward; two small `Star` atoms with short `Accent` orbit threads; one `Sun`
//! puff at the merger; a 1-px `Wave` chirp trace right-aligned above the title row.
//!
//! Per frame the field costs two multiply-adds and one compare per *candidate* dot: the
//! dots whose fixed stipple threshold is below the peak density (45 % of the annulus) are
//! listed once per resize with their `cos 2θ`, `sin 2θ` and radial bin, and the retarded
//! source is evaluated once per radial bin.

use std::f64::consts::{PI, TAU};

use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};
use sciart_core::{ease, geom};

pub static META: Meta = Meta {
    name: "binary_inspiral",
    category: Category::DeepCosmos,
    description: "Two black holes spiral together and merge, radiating a two-armed gravitational-wave spiral",
    title: "Black hole merger · rising chirp",
    captions: &["inspiral", "merger", "ringdown"],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(BinaryInspiral::new())
}

// ---- model --------------------------------------------------------------------------

/// Initial separation (model length unit).
const A0: f64 = 1.0;
/// Separation at contact, where the inspiral ends: the GW frequency rises by
/// `(A0 / A_MIN)^{3/2} ≈ 4×` over the inspiral, as much as the stipple can resolve.
const A_MIN: f64 = 0.4;
/// Initial orbital angular speed: one orbit in 3.2 s. Fixes the mass, `M = ω₀² a₀³`.
const OMEGA0: f64 = TAU / 3.2;
const T_INSPIRAL: f64 = 14.0;
const T_MERGER: f64 = 0.8;
const T_RINGDOWN: f64 = 5.2;
/// One loop: inspiral, merger, then ringdown and calm while the last waves leave.
const LOOP_S: f64 = T_INSPIRAL + T_MERGER + T_RINGDOWN;
/// Seconds into the merger at which the plunging holes touch (separation 0).
const T_PLUNGE: f64 = 0.25;
/// Ringdown (quasi-normal mode) frequency over the frequency at contact.
const RD_RATIO: f64 = 1.5;
/// Ringdown e-folding time, s.
const RD_TAU: f64 = 0.9;

/// Wave speed in field radii per second: slowed so the arms fit the screen (≈ 3.3 s to
/// cross the field), giving a bit under one arm turn per field radius at the start and
/// about four at contact.
const C_WAVE: f64 = 0.5;
/// The field is drawn from this fraction of the field radius out (BBH spec: "beyond an
/// inner radius only"); it is also the `r₀` of the `1 / max(r, r₀)` falloff.
const R_IN: f64 = 0.2;
/// Field radius as a fraction of the drawable area's shorter side (dots).
const FIELD_FRAC: f64 = 0.45;
/// Over the outer `EDGE_TAPER` of the field radius the density eases to 0, so the field
/// fades out instead of ending at a drawn circle.
const EDGE_TAPER: f64 = 0.2;
/// Strain (in source-amplitude units at `r₀`) that maps to the peak stipple density.
const H_REF: f64 = 0.35;
/// Arms are sharpened by weighting the density with `|cos(2θ − 2Φ)|^ARM_POWER`, so the
/// wave crests read as thin spiral arms rather than broad lobes.
pub(crate) const ARM_POWER: i32 = 3;
/// Peak stipple density (GEN-R26 lesson: ≈ 0.5 or lower, or arms read solid).
pub(crate) const P_MAX: f64 = 0.45;
/// Screen radius of each hole at `a = a₀`, in field radii.
const HOLE_R: f64 = 0.55 * R_IN;

/// Ring-buffer sample period (the runtime's step).
const SAMPLE_DT: f64 = 1.0 / 60.0;
/// Radial bins per dot of field radius: ¼-dot bins keep the retarded phase within
/// `2ω · ⅛ dot / c` ≈ 0.04 rad of exact at contact on 200×50.
const BINS_PER_DOT: f64 = 4.0;

/// Seconds of strain the chirp trace shows.
const TRACE_S: f64 = 2.5;
const TRACE_MAX_COLS: u16 = 36;
const TRACE_MIN_COLS: u16 = 12;
/// Half-height of the chirp trace in dots (3 rows tall: 12 dots).
const TRACE_AMP: f64 = 5.0;

/// Buffer span: the slowest reader, the field edge (`1 / c`) or the trace (`r₀/c + TRACE_S`).
const BUFFER_S: f64 = if 1.0 / C_WAVE > R_IN / C_WAVE + TRACE_S {
    1.0 / C_WAVE
} else {
    R_IN / C_WAVE + TRACE_S
};

/// The arms' density saturates once the envelope reaches this share of `H_REF`; below
/// it the arms keep their shape and fade in level instead ([`field_level`]), so the faint
/// outer arms and the dying ringdown read as rings, not sprinkle.
const SAT: f64 = 0.85;
const HOLE_INK: Ink = Ink::new(Role::Star, 255);
const FLASH_INK: Ink = Ink::new(Role::Sun, 255);
const TRACE_INK: Ink = Ink::new(Role::Wave, 200);
/// Orbit-thread levels at the newest and oldest ends (afterglow ≈ ½ orbit).
const THREAD_HEAD: f64 = 230.0;
const THREAD_TAIL: f64 = 70.0;
const THREAD_SEGMENTS: usize = 24;
/// Each hole's afterglow covers this many orbits (½ each would close the circle).
const THREAD_ORBITS: f64 = 0.25;
const HOLE_SIGMA: f64 = 1.3;
const REMNANT_SIGMA: f64 = 1.5;
/// Atom peak density: above 1 the core is solid (CORE-R108).
const HOLE_PEAK: f64 = 2.0;
const FLASH_SIGMA: f64 = 2.6;
/// The merger flash lasts from just before contact to this long after it.
const FLASH_FROM: f64 = T_PLUNGE - 0.1;
const FLASH_TO: f64 = T_PLUNGE + 0.3;

const FIELD_SALT: u64 = 0xB1AC_4013;
const HOLE_SALT: u64 = 0xB1AC_0001;

/// The scene opens this far into the inspiral, with the first arms already out, rather
/// than on the calm between loops.
const START_S: f64 = 3.0;

const TIMELINE: [(&str, f64); 3] = [
    ("inspiral", T_INSPIRAL),
    ("merger", T_MERGER),
    ("ringdown", T_RINGDOWN),
];

/// `M` in model units, from Kepler at `a₀`.
fn mass() -> f64 {
    OMEGA0 * OMEGA0 * A0.powi(3)
}

/// Peters' constant in `da/dt = −β/a³` (model units), chosen so contact comes at
/// [`T_INSPIRAL`].
pub(crate) fn beta() -> f64 {
    (A0.powi(4) - A_MIN.powi(4)) / (4.0 * T_INSPIRAL)
}

/// Separation `a(t) = (a₀⁴ − 4βt)^{1/4}` during the inspiral, `t` clamped to it (BBH-A1).
pub(crate) fn separation(t: f64) -> f64 {
    let t = t.clamp(0.0, T_INSPIRAL);
    (A0.powi(4) - 4.0 * beta() * t)
        .max(A_MIN.powi(4))
        .sqrt()
        .sqrt()
}

/// Kepler's orbital angular speed at separation `a`.
pub(crate) fn kepler_omega(a: f64) -> f64 {
    (mass() / (a * a * a)).sqrt()
}

/// Orbital phase `Φ(t) = ∫ω dt = √M · 2/(5β) · (a₀^{5/2} − a^{5/2})`, exact for the
/// Peters `a(t)` (d/dt gives `√M a^{−3/2}`).
pub(crate) fn inspiral_phase(t: f64) -> f64 {
    let a = separation(t);
    mass().sqrt() * 2.0 / (5.0 * beta()) * (A0.powf(2.5) - a.powf(2.5))
}

fn omega_contact() -> f64 {
    kepler_omega(A_MIN)
}

fn omega_ringdown() -> f64 {
    RD_RATIO * omega_contact()
}

/// GW amplitude `∝ ω^{2/3}`, normalised to 1 at the ringdown frequency (merger peak).
fn amplitude(omega: f64) -> f64 {
    (omega / omega_ringdown()).powf(2.0 / 3.0)
}

/// The binary at one instant: orbital phase (rad), GW amplitude, separation (`a₀`
/// units, 0 once merged) and orbital angular speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Source {
    pub(crate) phase: f64,
    pub(crate) amp: f64,
    pub(crate) sep: f64,
    pub(crate) omega: f64,
}

/// The source at scene time `t` (any real; the story loops every [`LOOP_S`]).
pub(crate) fn source(t: f64) -> Source {
    let t = if t.is_finite() {
        t.rem_euclid(LOOP_S)
    } else {
        0.0
    };
    let (we, wr) = (omega_contact(), omega_ringdown());
    if t < T_INSPIRAL {
        let a = separation(t);
        let omega = kepler_omega(a);
        return Source {
            phase: inspiral_phase(t),
            amp: amplitude(omega),
            sep: a,
            omega,
        };
    }
    let phase_contact = inspiral_phase(T_INSPIRAL);
    let u = t - T_INSPIRAL;
    if u < T_MERGER {
        // Plunge and merger: the frequency sweeps linearly up to the ringdown mode.
        let omega = we + (wr - we) * u / T_MERGER;
        return Source {
            phase: phase_contact + we * u + (wr - we) * u * u / (2.0 * T_MERGER),
            amp: amplitude(omega),
            sep: A_MIN * (1.0 - u / T_PLUNGE).max(0.0),
            omega,
        };
    }
    let u = u - T_MERGER;
    let phase_merged = phase_contact + we * T_MERGER + (wr - we) * T_MERGER / 2.0;
    Source {
        phase: phase_merged + wr * u,
        amp: (-u / RD_TAU).exp(),
        sep: 0.0,
        omega: wr,
    }
}

/// Plus strain of `src` at radius `r` (field radii) and screen angle `θ`:
/// `A · cos(2θ − 2Φ) · r₀ / max(r, r₀)`. The reference form the tests hold the
/// rasteriser's per-bin, per-dot factorisation to (`strain`, `field`, `density`).
#[cfg(test)]
pub(crate) fn strain(src: Source, r: f64, theta: f64) -> f64 {
    src.amp * (2.0 * (theta - src.phase)).cos_det() * R_IN / r.max(R_IN)
}

/// The field at scene time `t`: the source seen at the retarded time `t − r/c` (BBH-A3).
#[cfg(test)]
pub(crate) fn field(t: f64, r: f64, theta: f64) -> f64 {
    strain(source(t - r / C_WAVE), r, theta)
}

/// Stipple density for strain `h` whose local envelope (`A·r₀/r`, the crest value) is
/// `envelope`: `P_MAX · min(envelope / H_REF, 1) · (|h| / envelope)^ARM_POWER`, so at most
/// `P_MAX` and largest on the crests (BBH-A5). With `ARM_POWER = 1` it is the spec's
/// `P_MAX · |h| / H_REF` capped at `P_MAX`.
#[cfg(test)]
pub(crate) fn density(envelope: f64, h: f64) -> f64 {
    if h.is_finite() && envelope.is_finite() && envelope > 0.0 {
        let c = (h.abs() / envelope).min(1.0);
        P_MAX * (envelope / (H_REF * SAT)).min(1.0) * c.powi(ARM_POWER)
    } else {
        0.0
    }
}

/// Ink level of the arms where the envelope is `a` (in `H_REF` units): four flat steps,
/// full where the field is strong, dimmer where it is weak.
pub(crate) fn field_level(a: f64) -> u8 {
    if a >= 1.0 {
        230
    } else if a >= 0.5 {
        180
    } else if a >= 0.25 {
        130
    } else {
        90
    }
}

/// Ease-out of the density over the field's outer rim, `r` in field radii.
fn taper(r: f64) -> f64 {
    ease::smoothstep((1.0 - r) / EDGE_TAPER)
}

// ---- retarded-time buffer -----------------------------------------------------------

/// `(Φ, A)` of the source sampled every [`SAMPLE_DT`] at fixed times `k · SAMPLE_DT`,
/// newest last, in model time (independent of the canvas).
#[derive(Debug, Clone)]
struct Retarded {
    phase: Vec<f64>,
    amp: Vec<f64>,
    /// Sample index of the newest entry.
    newest: i64,
}

impl Retarded {
    /// A buffer whose newest sample is `k_newest`, filled with the closed-form history.
    fn new(k_newest: i64) -> Self {
        let n = (BUFFER_S / SAMPLE_DT).ceil() as usize + 4;
        let mut buf = Self {
            phase: vec![0.0; n],
            amp: vec![0.0; n],
            newest: 0,
        };
        // The history up to the start: the source is closed-form, so the arms already
        // in flight at the first frame are exactly the ones it emitted.
        for k in (k_newest + 1 - n as i64)..=k_newest {
            buf.push(k);
        }
        buf
    }

    fn len(&self) -> i64 {
        self.phase.len() as i64
    }

    fn slot(&self, k: i64) -> usize {
        k.rem_euclid(self.len()) as usize
    }

    /// Record the source at sample time `k · SAMPLE_DT` as the newest entry.
    fn push(&mut self, k: i64) {
        let s = source(k as f64 * SAMPLE_DT);
        let i = self.slot(k);
        (self.phase[i], self.amp[i]) = (s.phase, s.amp);
        self.newest = k;
    }

    /// `(Φ, A)` at time `t`, linearly interpolated between samples (the phase along the
    /// short way round, so a loop's restart never sweeps the pattern), clamped to the
    /// buffered span.
    fn at(&self, t: f64) -> (f64, f64) {
        let oldest = self.newest - self.len() + 1;
        let f = if t.is_finite() { t / SAMPLE_DT } else { 0.0 };
        let f = f.clamp(oldest as f64, self.newest as f64);
        let k0 = (f.floor() as i64).clamp(oldest, self.newest);
        let k1 = (k0 + 1).min(self.newest);
        let w = f - k0 as f64;
        let (i0, i1) = (self.slot(k0), self.slot(k1));
        let dp = geom::wrap_angle(self.phase[i1] - self.phase[i0]);
        (
            self.phase[i0] + w * dp,
            self.amp[i0] + w * (self.amp[i1] - self.amp[i0]),
        )
    }
}

// ---- scene --------------------------------------------------------------------------

/// A field dot that can ever be lit: its fixed stipple threshold is below [`P_MAX`].
#[derive(Debug, Clone, Copy)]
struct FieldDot {
    x: u16,
    y: u16,
    bin: u32,
    /// `cos 2θ`, `sin 2θ` of the dot's screen angle about the centre.
    c2: f32,
    s2: f32,
    /// The dot's fixed stipple threshold, `hash01(salt ^ cell_key(x, y))` (CORE-R107).
    thr: f32,
}

/// Where the chirp trace sits, in dots.
#[derive(Debug, Clone, Copy)]
struct Trace {
    x0: i32,
    x1: i32,
    yc: f64,
}

#[derive(Debug)]
pub struct BinaryInspiral {
    size: Size,
    time: f64,
    /// Next ring-buffer sample index to record.
    next_k: i64,
    buf: Retarded,
    timeline: Timeline,
    /// Field centre and radius in dots.
    cx: f64,
    cy: f64,
    r_out: f64,
    field: Vec<FieldDot>,
    /// Per radial bin: `(A·r₀/r·taper/H_REF)·(cos 2Φ, sin 2Φ)` at the bin's retarded time.
    table: Vec<(f32, f32, f32, u8)>,
    /// Per cell: the sign of the field dots lit in it this frame (0 = none yet), so a
    /// cell never mixes `Hot` and `Cold` dots under one ink (BBH-A5).
    cell_sign: Vec<i8>,
    trace: Option<Trace>,
    dots: Dots,
    dirty: bool,
}

impl BinaryInspiral {
    fn new() -> Self {
        let start_k = (START_S / SAMPLE_DT).round() as i64;
        Self {
            size: Size::new(1, 1),
            time: START_S,
            next_k: start_k + 1,
            buf: Retarded::new(start_k),
            timeline: Timeline::new(&TIMELINE),
            cx: 0.0,
            cy: 0.0,
            r_out: 1.0,
            field: Vec::new(),
            table: Vec::new(),
            cell_sign: Vec::new(),
            trace: None,
            dots: Dots::default(),
            dirty: true,
        }
    }

    /// Scene time within the loop.
    fn loop_time(&self) -> f64 {
        self.time.rem_euclid(LOOP_S)
    }

    /// The retarded field value over `H_REF` at radius `rr` dots and angle with
    /// `(cos 2θ, sin 2θ) = (c2, s2)`, read from the buffer (before the taper).
    fn retarded(&self, rr: f64) -> (f64, f64) {
        let r = rr / self.r_out;
        let (phase, amp) = self.buf.at(self.time - r / C_WAVE);
        let k = amp * R_IN / r.max(R_IN) / H_REF;
        let (s, c) = (2.0 * phase).sin_cos_det();
        (k * c, k * s)
    }

    /// Hole `i`'s centre in dots for source state `s`.
    fn hole(&self, s: Source, i: usize) -> (f64, f64) {
        let rho = s.sep / A0 * HOLE_R * self.r_out;
        let (sn, cs) = (s.phase + PI * i as f64).sin_cos_det();
        (self.cx + rho * cs, self.cy + rho * sn)
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        self.cell_sign.fill(0);
        let now = source(self.time);
        let lt = self.loop_time();

        // Orbit threads: each hole's true path over the last half orbit, fading.
        if now.sep > 0.0 {
            let span = THREAD_ORBITS * TAU / now.omega.max(1e-9);
            for i in 0..2 {
                let mut prev = self.hole(now, i);
                for j in 1..=THREAD_SEGMENTS {
                    let back = span * j as f64 / THREAD_SEGMENTS as f64;
                    if back > lt {
                        break; // not before this loop's start
                    }
                    let s = source(self.time - back);
                    let p = self.hole(s, i);
                    let age = (j as f64 - 0.5) / THREAD_SEGMENTS as f64;
                    let level = THREAD_HEAD + (THREAD_TAIL - THREAD_HEAD) * age;
                    let ink = Ink::new(Role::Accent, level.round() as u8);
                    self.dots.line(prev.0, prev.1, p.0, p.1, ink);
                    prev = p;
                }
            }
            for i in 0..2 {
                let (x, y) = self.hole(now, i);
                let salt = HOLE_SALT + i as u64;
                self.dots
                    .stipple_puff(x, y, HOLE_SIGMA, HOLE_PEAK, salt, HOLE_INK);
            }
        } else {
            self.dots.stipple_puff(
                self.cx,
                self.cy,
                REMNANT_SIGMA,
                HOLE_PEAK,
                HOLE_SALT,
                HOLE_INK,
            );
        }
        let u = lt - T_INSPIRAL;
        if (FLASH_FROM..FLASH_TO).contains(&u) {
            self.dots
                .stipple_puff(self.cx, self.cy, FLASH_SIGMA, 0.9, HOLE_SALT + 7, FLASH_INK);
        }

        // Chirp trace: the strain at the inner edge, θ = 0, over the last TRACE_S.
        if let Some(tr) = self.trace {
            let delay = R_IN / C_WAVE;
            let n = (tr.x1 - tr.x0).max(1);
            let mut prev: Option<(f64, f64)> = None;
            for x in tr.x0..=tr.x1 {
                let back = TRACE_S * f64::from(tr.x1 - x) / f64::from(n);
                let (phase, amp) = self.buf.at(self.time - delay - back);
                let h = amp * (-2.0 * phase).cos_det();
                let p = (f64::from(x), tr.yc - TRACE_AMP * h);
                match prev {
                    Some(q) => self.dots.line(q.0, q.1, p.0, p.1, TRACE_INK),
                    None => self.dots.plot(p.0, p.1, TRACE_INK),
                }
                prev = Some(p);
            }
        }

        // The retarded source per radial bin.
        for b in 0..self.table.len() {
            let rr = b as f64 / BINS_PER_DOT;
            let r = rr / self.r_out;
            self.table[b] = if r < R_IN * 0.98 {
                (0.0, 0.0, 0.0, 0)
            } else {
                let (x, y) = self.retarded(rr);
                let t = taper(r);
                let (x, y) = (x * t, y * t);
                let a = x.hypot(y);
                // p = P_MAX·min(a, 1)·(|v|/a)^ARM_POWER = k·|v|^ARM_POWER.
                let k = if a > 1e-12 {
                    P_MAX * (a / SAT).min(1.0) / a.powi(ARM_POWER)
                } else {
                    0.0
                };
                (x as f32, y as f32, k as f32, field_level(a))
            };
        }

        // The strain field as fixed-threshold stipple, drawn last so it never takes a
        // cell already lit by a hole, thread or the trace.
        let w = usize::from(self.size.width());
        let p_max = P_MAX as f32;
        for d in &self.field {
            let (tx, ty, k, level) = self.table[d.bin as usize];
            let v = d.c2 * tx + d.s2 * ty;
            let av = v.abs();
            if d.thr >= (k * av * av * av).min(p_max) {
                continue;
            }
            let (x, y) = (i32::from(d.x), i32::from(d.y));
            let cell = usize::from(d.y / 4) * w + usize::from(d.x / 2);
            let sign: i8 = if v > 0.0 { 1 } else { -1 };
            match self.cell_sign[cell] {
                0 => {
                    let taken = self
                        .dots
                        .cell_mask(i32::from(d.x / 2), i32::from(d.y / 4))
                        .is_some_and(|m| m != 0);
                    if taken {
                        self.cell_sign[cell] = 2; // someone else's cell
                        continue;
                    }
                    self.cell_sign[cell] = sign;
                }
                s if s != sign => continue,
                _ => {}
            }
            let role = if sign > 0 { Role::Hot } else { Role::Cold };
            self.dots.set(x, y, Ink::new(role, level));
        }
    }
}

impl Generator for BinaryInspiral {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let (w, h) = (size.width(), size.height());
        // Leave the title row out of the field's box.
        let rows = h.saturating_sub(1).max(1);
        let (cx, cy) = Dots::from_cells(f64::from(w) / 2.0, f64::from(rows) / 2.0);
        let (wd, hd) = (
            f64::from(w) * f64::from(Dots::CELL_W),
            f64::from(rows) * f64::from(Dots::CELL_H),
        );
        (self.cx, self.cy) = (cx, cy);
        self.r_out = (FIELD_FRAC * wd.min(hd)).max(1.0);

        self.field.clear();
        let reach = self.r_out.ceil() as i32 + 1;
        let ds = self.dots.dot_size();
        let (dw, dh) = (i32::from(ds.width()), i32::from(ds.height()));
        let (xc, yc) = (cx.round() as i32, cy.round() as i32);
        for y in (yc - reach).max(0)..(yc + reach + 1).min(dh) {
            for x in (xc - reach).max(0)..(xc + reach + 1).min(dw) {
                let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
                let rr = dx.hypot(dy);
                let r = rr / self.r_out;
                if !(R_IN..=1.0).contains(&r) {
                    continue;
                }
                let thr = hash01(FIELD_SALT ^ cell_key(x, y));
                if thr >= P_MAX {
                    continue; // never lit at any density ≤ P_MAX
                }
                let r2 = rr * rr;
                self.field.push(FieldDot {
                    x: x as u16,
                    y: y as u16,
                    bin: (rr * BINS_PER_DOT).round() as u32,
                    c2: ((dx * dx - dy * dy) / r2) as f32,
                    s2: (2.0 * dx * dy / r2) as f32,
                    thr: thr as f32,
                });
            }
        }
        let bins = (self.r_out * BINS_PER_DOT).ceil() as usize + 2;
        self.table.clear();
        self.table.resize(bins, (0.0, 0.0, 0.0, 0));
        self.cell_sign.clear();
        self.cell_sign.resize(size.area(), 0);

        // The chirp trace: 3 rows above the title row, right-aligned, clear of the field.
        let field_right = (cx + self.r_out) / f64::from(Dots::CELL_W);
        let right = i32::from(w) - 2;
        let room = right - field_right.ceil() as i32 - 2;
        let cols = room.min(i32::from(TRACE_MAX_COLS).min(i32::from(w) / 5));
        self.trace = (h >= 12 && cols >= i32::from(TRACE_MIN_COLS)).then(|| {
            let (_, yc) = Dots::from_cells(0.0, f64::from(h) - 2.5);
            Trace {
                x0: (right - cols + 1) * Dots::CELL_W,
                x1: right * Dots::CELL_W + 1,
                yc,
            }
        });
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.time += dt;
        // Record every fixed sample time passed; a long gap only needs the last buffer's
        // worth (the tolerance absorbs the rounding of summed 1/60 s steps).
        let target = ((self.time + 1e-9) / SAMPLE_DT).floor() as i64;
        self.next_k = self.next_k.max(target - self.buf.len() + 1);
        while self.next_k <= target {
            self.buf.push(self.next_k);
            self.next_k += 1;
        }
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.rasterise();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
    }

    fn caption(&self) -> Option<&'static str> {
        Some(self.timeline.at(self.loop_time()).label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_at(size: Size, secs: f64) -> BinaryInspiral {
        let mut ctx = Ctx::new(42, size);
        let mut g = BinaryInspiral::new();
        g.resize(size, &mut ctx);
        // `secs` is the scene time to reach (the scene starts at START_S).
        for _ in 0..((secs - START_S).max(0.0) * 60.0).round() as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g.prepare();
        g
    }

    #[test]
    fn bbh_a1_inspiral_chirps_and_a4_falls_linearly() {
        let (a0_4, beta) = (A0.powi(4), beta());
        let mut prev = source(0.0);
        let mut t = 0.05;
        while t < T_INSPIRAL {
            let s = source(t);
            assert!(s.sep < prev.sep, "separation falls at t={t}");
            assert!(s.omega > prev.omega, "frequency rises at t={t}");
            assert!(s.amp > prev.amp, "amplitude rises at t={t}");
            // a⁴ linear in t within 1 %.
            let want = a0_4 - 4.0 * beta * t;
            assert!((s.sep.powi(4) - want).abs() <= 0.01 * want, "a⁴ at t={t}");
            // The closed-form phase integrates Kepler's ω (central difference, 1e-4).
            let e = 1e-4;
            let dphi = (inspiral_phase(t + e) - inspiral_phase(t - e)) / (2.0 * e);
            assert!((dphi / s.omega - 1.0).abs() < 1e-4, "dΦ/dt = ω at t={t}");
            prev = s;
            t += 0.05;
        }
        // Contact at the end of the inspiral; ~4× frequency sweep, a chirp.
        assert!((separation(T_INSPIRAL) - A_MIN).abs() < 1e-9);
        assert!(source(T_INSPIRAL - 1e-6).omega / source(0.0).omega > 3.5);
    }

    #[test]
    fn bbh_a2_field_has_quadrupole_symmetry() {
        for &t in &[0.7, 5.0, 13.9, 14.3, 16.0] {
            for &r in &[0.1, 0.25, 0.5, 0.9] {
                for k in 0..12 {
                    let th = f64::from(k) * 0.53;
                    let h = field(t, r, th);
                    assert!((field(t, r, th + PI) - h).abs() < 1e-12, "θ+π");
                    assert!((field(t, r, th + PI / 2.0) + h).abs() < 1e-12, "θ+π/2");
                }
            }
        }
        // The per-dot form (cos 2θ, sin 2θ) is the same pattern as the angle form.
        let g = scene_at(Size::new(200, 50), 9.0);
        for d in g.field.iter().step_by(97) {
            let (dx, dy) = (f64::from(d.x) - g.cx, f64::from(d.y) - g.cy);
            let th = dy.atan2(dx);
            assert!((f64::from(d.c2) - (2.0 * th).cos_det()).abs() < 1e-6);
            assert!((f64::from(d.s2) - (2.0 * th).sin_det()).abs() < 1e-6);
        }
    }

    #[test]
    fn bbh_a3_strain_is_the_source_delayed_by_r_over_c() {
        // Exact sample times and arbitrary radii: the buffer, interpolated, matches the
        // closed-form source at t − r/c. Linear interpolation of Φ over one 1/60 s step
        // errs by ≤ Φ''·h²/8 < 1e-4 rad and of A by ≤ 1e-4, so 1e-3 on h/H_REF is ample.
        for secs in [3.0, 9.5, 14.4, 15.5] {
            let g = scene_at(Size::new(200, 50), secs);
            for &frac in &[0.21, 0.37, 0.6, 0.83, 0.99] {
                let rr = frac * g.r_out;
                let (x, y) = g.retarded(rr);
                for k in 0..8 {
                    let th = f64::from(k) * 0.41;
                    let (s, c) = (2.0 * th).sin_cos_det();
                    let got = c * x + s * y;
                    let want = field(g.time, frac, th) / H_REF;
                    assert!(
                        (got - want).abs() < 1e-3,
                        "t={secs} r={frac}: {got} vs {want}"
                    );
                }
            }
        }
        // And it is a delay, not the present: far out the arm lags.
        let g = scene_at(Size::new(200, 50), 9.5);
        let s_now = source(g.time);
        let (x, y) = g.retarded(0.9 * g.r_out);
        let k_now = s_now.amp * R_IN / 0.9 / H_REF;
        let (sn, cn) = (2.0 * s_now.phase).sin_cos_det();
        assert!((x - k_now * cn).abs() + (y - k_now * sn).abs() > 0.05);
    }

    #[test]
    fn bbh_a4_captions_run_inspiral_merger_ringdown_and_loop() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(1, size);
        let mut g = BinaryInspiral::new();
        g.resize(size, &mut ctx);
        let mut seen: Vec<&str> = vec![g.caption().unwrap()];
        for _ in 0..(2.0 * LOOP_S * 60.0) as usize + 30 {
            g.update(SIM_DT, &mut ctx);
            let c = g.caption().unwrap();
            if seen.last() != Some(&c) {
                seen.push(c);
            }
        }
        let want = [
            "inspiral", "merger", "ringdown", "inspiral", "merger", "ringdown", "inspiral",
        ];
        assert_eq!(seen, want);
    }

    #[test]
    fn bbh_a5_hot_only_where_positive_cold_only_where_negative_density_capped() {
        for e in [0.0, 0.1, 0.35, 1.0, 1e9] {
            for k in -10..=10 {
                let p = density(e, e * f64::from(k) / 10.0);
                assert!((0.0..=P_MAX).contains(&p), "e={e} k={k}: {p}");
            }
        }
        assert!((density(1.0, -1.0) - P_MAX).abs() < 1e-12);
        assert!(density(1.0, f64::NAN) == 0.0 && density(0.5, 0.0) == 0.0);
        let size = Size::new(200, 50);
        for secs in [4.0, 12.0, 14.5, 15.2] {
            let g = scene_at(size, secs);
            // The field value at every candidate dot, from the bin table.
            let mut v_at = std::collections::HashMap::new();
            for d in &g.field {
                let (tx, ty, _, _) = g.table[d.bin as usize];
                v_at.insert((i32::from(d.x), i32::from(d.y)), d.c2 * tx + d.s2 * ty);
            }
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let (mut lit, mut checked) = (0usize, 0usize);
            for cy in 0..i32::from(size.height()) {
                for cx in 0..i32::from(size.width()) {
                    let mask = g.dots.cell_mask(cx, cy).unwrap();
                    if mask == 0 {
                        continue;
                    }
                    let role = canvas.get(cx, cy).unwrap().ink.role;
                    if !matches!(role, Role::Hot | Role::Cold) {
                        continue;
                    }
                    for x in 2 * cx..2 * cx + 2 {
                        for y in 4 * cy..4 * cy + 4 {
                            if !g.dots.is_lit(x, y) {
                                continue;
                            }
                            let v = *v_at.get(&(x, y)).expect("Hot/Cold dot is a field dot");
                            let ok = if role == Role::Hot { v > 0.0 } else { v < 0.0 };
                            assert!(ok, "t={secs} dot ({x},{y}) v={v} drawn {role:?}");
                            checked += 1;
                        }
                    }
                }
            }
            for d in &g.field {
                if !g.dots.is_lit(i32::from(d.x), i32::from(d.y)) {
                    continue;
                }
                lit += 1;
                // A lit field dot is one the reference density keeps (f32 slack).
                let (tx, ty, _, _) = g.table[d.bin as usize];
                let env = f64::from(tx.hypot(ty)) * H_REF;
                let h = f64::from(d.c2 * tx + d.s2 * ty) * H_REF;
                assert!(f64::from(d.thr) < density(env, h) + 1e-5, "t={secs}");
            }
            assert!(checked > 100, "t={secs}: field drawn ({checked} dots)");
            // Every annulus dot counts: candidates are 45 % of them, so lit ≤ 0.45.
            let annulus = PI * (1.0 - R_IN * R_IN) * g.r_out * g.r_out;
            let share = lit as f64 / annulus;
            assert!(share <= P_MAX, "t={secs}: lit share {share}");
        }
    }

    #[test]
    fn bbh_merger_flash_and_remnant() {
        let g = scene_at(Size::new(120, 40), T_INSPIRAL + T_PLUNGE + 0.1);
        assert_eq!(g.caption(), Some("merger"));
        let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
        g.render(&mut canvas);
        let suns = canvas
            .cells()
            .iter()
            .filter(|c| c.ink.role == Role::Sun)
            .count();
        assert!(suns > 0, "merger flash drawn");
        let g = scene_at(Size::new(120, 40), T_INSPIRAL + T_MERGER + 1.0);
        let mut canvas = Canvas::new(g.size, GlyphMode::Unicode);
        g.render(&mut canvas);
        assert!(canvas.cells().iter().all(|c| c.ink.role != Role::Sun));
        assert!(canvas.cells().iter().any(|c| c.ink.role == Role::Star));
    }

    #[test]
    fn bbh_faint_arms_keep_their_density_and_fade_in_level() {
        // A weak envelope (SAT of H_REF) still draws crests at full density...
        let e = SAT * H_REF;
        assert!((density(e, e) - P_MAX).abs() < 1e-12);
        // ...and the level steps down with the envelope instead.
        let levels: Vec<u8> = [0.1, 0.3, 0.6, 1.2]
            .iter()
            .map(|&a| field_level(a))
            .collect();
        assert!(levels.windows(2).all(|w| w[0] < w[1]), "{levels:?}");
    }

    #[test]
    fn bbh_holes_are_small_atoms_with_a_solid_core() {
        let g = scene_at(Size::new(200, 50), 6.0);
        let now = source(g.time);
        for i in 0..2 {
            let (x, y) = g.hole(now, i);
            let (x, y) = (x.round() as i32, y.round() as i32);
            for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                assert!(
                    g.dots.is_lit(x + dx, y + dy),
                    "hole {i} core at ({dx},{dy})"
                );
            }
        }
    }

    #[test]
    fn bbh_chirp_trace_only_when_room() {
        let g = scene_at(Size::new(200, 50), 1.0);
        assert!(g.trace.is_some());
        let g = scene_at(Size::new(40, 12), 1.0);
        assert!(g.trace.is_none());
    }

    #[test]
    fn bbh_tiny_and_odd_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (2, 1), (1, 9), (7, 3), (500, 150)] {
            let mut g = scene_at(Size::new(w, h), 0.5);
            let mut ctx = Ctx::new(1, Size::new(w, h));
            for dt in [0.0, 0.25, 0.1] {
                g.update(dt, &mut ctx);
                g.prepare();
            }
        }
    }
}
