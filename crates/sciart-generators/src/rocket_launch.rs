//! A two-stage rocket launch from countdown to orbit (SPEC-002 `rocket_launch`).
//!
//! The vehicle is a toy two-stage rocket whose constants are chosen for the screen, not
//! for a real launcher, but the dynamics are honest: during a burn `ṁ = F/ve` and
//! `a = F/m − g`, integrated so that the velocity tracks Tsiolkovsky's equation with
//! gravity loss, `v = ve ln(m0/m) − g t` (ROC-A1). Everything world-side (vehicle,
//! exhaust, clouds, camera) lives in metres; only `render` converts to rows with the
//! scale `k` (rows per metre) chosen in `resize`, so a resize mid-flight changes the
//! zoom, never the physics.

use std::f64::consts::{PI, TAU};

use rand::Rng;
use sciart_core::draw;
use sciart_core::geom::ASPECT;
use sciart_core::particles::{gaussian, gaussian2, plot};
use sciart_core::prelude::*;

pub static META: Meta = Meta {
    name: "rocket_launch",
    category: Category::Voyages,
    description: "A rocket launch from countdown through stage separation",
    title: "Rocket launch · stages drop away",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(RocketLaunch::new())
}

// ---------------------------------------------------------------------------------------
// Timeline (s, from the start of a cycle).
// ---------------------------------------------------------------------------------------

const CYCLE: f64 = 22.0;
/// End of the countdown / engine ignition (hold-down begins).
const T_IGN: f64 = 4.0;
/// Hold-down release.
const T_LIFT: f64 = 5.0;
/// First-stage burnout and separation.
const T_SEP: f64 = T_LIFT + BURN1;
/// Second-stage ignition after a short ballistic coast.
const T_IGN2: f64 = T_SEP + 0.6;
/// Second-stage burnout.
const T_BO2: f64 = T_IGN2 + BURN2;
/// Every instant at which the dynamics change form; substeps never straddle one.
const EVENTS: [f64; 6] = [T_IGN, T_LIFT, T_SEP, T_IGN2, T_BO2, CYCLE];
/// Flight time after which the camera starts following (the rocket is at 60 % height).
const FOLLOW_TAU: f64 = 5.0;
/// Rate (1/s) at which the camera stops once the rocket has left the frame.
const CAM_EASE: f64 = 2.5;

// ---------------------------------------------------------------------------------------
// Vehicle (tonnes, metres, seconds ⇒ forces in kN).
// ---------------------------------------------------------------------------------------

const G: f64 = 9.81;
/// Lift-off mass of the full stack.
const M0: f64 = 300.0;
const PROP1: f64 = 200.0;
const BURN1: f64 = 8.0;
/// Lift-off thrust-to-weight ratio.
const TW1: f64 = 1.4;
/// Second stage (upper stage + payload), fully fuelled.
const M02: f64 = 60.0;
const PROP2: f64 = 45.0;
const BURN2: f64 = 6.4;
const TW2: f64 = 1.3;

/// One engine burn: thrust `f` (kN), exhaust speed `ve` (m/s), mass flow `ṁ = F/ve`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Burn {
    pub f: f64,
    pub ve: f64,
    pub m0: f64,
    pub m_dry: f64,
}

impl Burn {
    /// A burn that uses `prop` of `m0` in `duration` at lift-off thrust-to-weight `tw`.
    fn sized(m0: f64, prop: f64, duration: f64, tw: f64) -> Self {
        let f = tw * m0 * G;
        let mdot = prop / duration;
        Self {
            f,
            ve: f / mdot,
            m0,
            m_dry: m0 - prop,
        }
    }

    /// `ṁ = F / ve`.
    pub fn mdot(&self) -> f64 {
        self.f / self.ve
    }
}

fn stage1() -> Burn {
    Burn::sized(M0, PROP1, BURN1, TW1)
}

fn stage2() -> Burn {
    Burn::sized(M02, PROP2, BURN2, TW2)
}

/// Tsiolkovsky with gravity loss: speed after burning from `m0` down to `m` for `tau`
/// seconds in uniform gravity `g`, starting from rest (ROC-A1 reference).
#[cfg(test)]
pub(crate) fn rocket_velocity(ve: f64, m0: f64, m: f64, g: f64, tau: f64) -> f64 {
    ve * (m0 / m).ln() - g * tau
}

/// Height gained in the same burn: `∫v dτ = ve [τ + (m/ṁ) ln(m/m0)] − g τ²/2`.
pub(crate) fn rocket_altitude(b: &Burn, g: f64, tau: f64) -> f64 {
    let mdot = b.mdot();
    let m = b.m0 - mdot * tau;
    b.ve * (tau + m / mdot * (m / b.m0).ln()) - 0.5 * g * tau * tau
}

/// Point-mass state of a vehicle: altitude (m), vertical velocity (m/s), mass (t).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Body {
    pub alt: f64,
    pub v: f64,
    pub m: f64,
}

/// Advance `s` by `h` seconds under `a = F/m − g` (engine `Some`) or `a = −g`.
///
/// For a purely time-dependent acceleration RK4 reduces to Simpson's rule for `v` and
/// `x += h v + h²/6 (a₀ + 2 a½)` for `x`: fourth order, so a 1/120 s substep tracks the
/// rocket equation to ~1e-10 relative (ROC-A1). Mass falls linearly, which is exact.
pub(crate) fn integrate(s: &mut Body, burn: Option<&Burn>, g: f64, h: f64) {
    let (a0, am, a1, dm) = match burn {
        Some(b) => {
            let dm = b.mdot() * h;
            // Never burn into the dry mass, whatever the step.
            let m1 = (s.m - dm).max(b.m_dry);
            let dm = s.m - m1;
            let mm = s.m - 0.5 * dm;
            (b.f / s.m - g, b.f / mm - g, b.f / m1 - g, dm)
        }
        None => (-g, -g, -g, 0.0),
    };
    s.alt += h * s.v + h * h / 6.0 * (a0 + 2.0 * am);
    s.v += h / 6.0 * (a0 + 4.0 * am + a1);
    s.m -= dm;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Countdown,
    Ignition,
    Stage1,
    Coast,
    Stage2,
    Done,
}

/// Where in the cycle the scene is at time `t` (s) — the single source of truth for
/// which engine burns (ROC-A2).
pub(crate) fn phase_at(t: f64) -> Phase {
    match t {
        t if t < T_IGN => Phase::Countdown,
        t if t < T_LIFT => Phase::Ignition,
        t if t < T_SEP => Phase::Stage1,
        t if t < T_IGN2 => Phase::Coast,
        t if t < T_BO2 => Phase::Stage2,
        _ => Phase::Done,
    }
}

/// Whether any engine is firing (ignition hold-down counts).
pub(crate) fn engine_on(p: Phase) -> bool {
    matches!(p, Phase::Ignition | Phase::Stage1 | Phase::Stage2)
}

// ---------------------------------------------------------------------------------------
// Exhaust and vapour.
// ---------------------------------------------------------------------------------------

/// Exhaust pool budget: ≈ 0.08·area, at most 1500 (SPEC-002).
const EXHAUST_PER_CELL: f64 = 0.08;
const EXHAUST_MAX: usize = 1500;
const VAPOUR_MAX: usize = 90;
/// Mean exhaust lifetime; emission rate is sized so the pool runs just under budget.
const LIFE_MEAN: f64 = 2.6;
/// Mean lifetime of the second stage's high-altitude trail.
const LIFE_TRAIL: f64 = 4.2;
const LIFE_SPREAD: f64 = 0.9;
/// Linear drag rate (1/s): momentum-dominated jet → slow billowing smoke.
const DRAG: f64 = 2.4;
/// Cooling e-folding time (s) of the exhaust temperature `T = exp(−age/τ)`.
const COOL: f64 = 0.22;
/// Buoyant acceleration (m/s²) scale; hot gas rises harder than cold smoke.
const BUOY: f64 = 16.0;
/// Turbulent velocity diffusion (m/s per √s).
const TURB: f64 = 16.0;
/// Light breeze drifting the smoke to the right (m/s).
const WIND: f64 = 4.0;
/// Half-angle-ish spread (rad, 1σ) of the exhaust cone.
const SPREAD: f64 = 0.09;
/// Largest physics substep (s).
const H_MAX: f64 = 1.0 / 120.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Puff {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    age: f64,
    life: f64,
}

impl Puff {
    fn temperature(&self) -> f64 {
        (-self.age / COOL).exp()
    }
}

/// Exact update over `h` for linear drag towards `(wind, 0)` plus a constant
/// acceleration `(0, ay)`: unconditionally stable for any step.
fn drift(p: &mut Puff, ay: f64, drag: f64, h: f64) {
    let e = (-drag * h).exp();
    let decay = (1.0 - e) / drag;
    let (tx, ty) = (WIND, ay / drag);
    p.x += tx * h + (p.vx - tx) * decay;
    p.y += ty * h + (p.vy - ty) * decay;
    p.vx = tx + (p.vx - tx) * e;
    p.vy = ty + (p.vy - ty) * e;
    p.age += h;
}

/// Hot exhaust hitting the pad is turned sideways: vertical momentum becomes a fast
/// horizontal spread along the ground, with a slight bounce.
fn hit_ground(p: &mut Puff, side: f64) {
    if p.y < 0.0 {
        let impact = (-p.vy).max(0.0);
        p.y = 0.0;
        p.vy = 0.12 * impact;
        p.vx += side * 0.75 * impact;
    }
}

/// Buoyant acceleration of an exhaust parcel.
fn exhaust_buoyancy(p: &Puff) -> f64 {
    BUOY * (0.3 + p.temperature())
}

/// Cold vapour is denser than air and sinks.
const VAPOUR_SINK: f64 = -3.0;

// ---------------------------------------------------------------------------------------
// Sprites.
// ---------------------------------------------------------------------------------------

const NOSE: Glyph = Glyph::new('▲', '^');
const BODY: Glyph = Glyph::new('█', '#');
const WALL: Glyph = Glyph::new('│', '|');
const SLOPE_L: Glyph = Glyph::new('╱', '/');
const SLOPE_R: Glyph = Glyph::new('╲', '\\');
const RING: Glyph = Glyph::new('═', '=');
const RING_C: Glyph = Glyph::new('╪', '+');
const NONE: Glyph = Glyph::ascii(' ');

type Row = [Glyph; 3];
const R_NOSE: Row = [NONE, NOSE, NONE];
const R_FAIRING: Row = [SLOPE_L, BODY, SLOPE_R];
const R_BODY: Row = [WALL, BODY, WALL];
const R_RING: Row = [RING, RING_C, RING];
const R_FINS: Row = [SLOPE_L, BODY, SLOPE_R];

/// Sprite rows (top → bottom) and how many of them belong to the upper stage.
fn sprite(height: u16) -> (&'static [Row], usize) {
    const TALL: [Row; 6] = [R_NOSE, R_FAIRING, R_BODY, R_RING, R_BODY, R_FINS];
    const MID: [Row; 5] = [R_NOSE, R_FAIRING, R_BODY, R_RING, R_FINS];
    const SMALL: [Row; 3] = [R_NOSE, R_BODY, R_FINS];
    const TINY: [Row; 1] = [[NONE, NOSE, NONE]];
    match height {
        24.. => (&TALL, 3),
        20.. => (&MID, 3),
        10.. => (&SMALL, 2),
        _ => (&TINY, 1),
    }
}

const FLAME: [(Glyph, Role, u8); 4] = [
    (Glyph::new('▓', '#'), Role::Accent, 255),
    (Glyph::new('▒', '*'), Role::Hot, 235),
    (Glyph::new('░', ':'), Role::Hot, 170),
    (Glyph::new('·', '.'), Role::Hot, 110),
];

const MINUS: Glyph = Glyph::new('−', '-');

// ---------------------------------------------------------------------------------------
// Scene.
// ---------------------------------------------------------------------------------------

/// The spent first stage after separation.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spent {
    body: Body,
    theta: f64,
    omega: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Cloud {
    /// Horizontal position as a fraction of the half-width.
    fx: f64,
    alt: f64,
    len: u16,
}

#[derive(Debug)]
pub struct RocketLaunch {
    /// Time within the cycle (s).
    t: f64,
    rocket: Body,
    spent: Option<Spent>,
    /// Altitude (m) at the bottom of the screen's pad reference, and its frozen speed
    /// once the second stage lights.
    cam: f64,
    cam_v: Option<f64>,
    exhaust: ParticlePool<Puff>,
    vapour: ParticlePool<Puff>,
    exhaust_acc: f64,
    vapour_acc: f64,
    /// Exhaust particles spawned by the last `update` and whether an engine burned in it.
    spawned: usize,
    burned: bool,
    clouds: [Option<Cloud>; 3],
    flicker: f64,
    // Screen mapping, set in `resize`.
    width: u16,
    height: u16,
    /// Rows per metre.
    k: f64,
    /// Screen row of the rocket's base when it stands on the pad.
    base_row: f64,
    /// Screen column of the pad centre.
    cx: f64,
    /// Altitude (m) above which the camera follows (60 % of the height).
    follow_alt: f64,
    ready: bool,
}

impl RocketLaunch {
    fn new() -> Self {
        Self {
            t: 0.0,
            rocket: Body::default(),
            spent: None,
            cam: 0.0,
            cam_v: None,
            exhaust: ParticlePool::with_capacity(0),
            vapour: ParticlePool::with_capacity(0),
            exhaust_acc: 0.0,
            vapour_acc: 0.0,
            spawned: 0,
            burned: false,
            clouds: [None; 3],
            flicker: 0.0,
            width: 0,
            height: 0,
            k: 0.0,
            base_row: 0.0,
            cx: 0.0,
            follow_alt: 1.0,
            ready: false,
        }
    }

    fn restart(&mut self, ctx: &mut Ctx) {
        self.t = 0.0;
        self.rocket = Body {
            alt: 0.0,
            v: 0.0,
            m: M0,
        };
        self.spent = None;
        self.cam = 0.0;
        self.cam_v = None;
        self.exhaust.clear();
        self.vapour.clear();
        self.exhaust_acc = 0.0;
        self.vapour_acc = 0.0;
        for (i, c) in self.clouds.iter_mut().enumerate() {
            *c = Some(Cloud {
                fx: ctx.rng.random_range(-0.9..0.9),
                alt: 140.0 + 170.0 * i as f64 + ctx.rng.random_range(-30.0..30.0),
                len: ctx.rng.random_range(5..12),
            });
        }
    }

    fn sprite(&self) -> (&'static [Row], usize) {
        sprite(self.height)
    }

    /// Metres per sprite row (the stage-2 nozzle sits this far above the stack base).
    fn row_m(&self) -> f64 {
        if self.k > 0.0 { 1.0 / self.k } else { 0.0 }
    }

    /// Screen row (fractional) of world altitude `alt`.
    fn row_of(&self, alt: f64) -> f64 {
        self.base_row - (alt - self.cam) * self.k
    }

    /// Screen column (fractional) of world x (m).
    fn col_of(&self, x: f64) -> f64 {
        self.cx + x * self.k * ASPECT
    }

    /// Altitude of the burning engine's nozzle.
    fn nozzle_alt(&self, p: Phase) -> f64 {
        let (rows, upper) = self.sprite();
        match p {
            Phase::Stage2 => self.rocket.alt + (rows.len() - upper) as f64 * self.row_m(),
            _ => self.rocket.alt,
        }
    }

    fn separate(&mut self, ctx: &mut Ctx) {
        let dry1 = M0 - PROP1 - M02;
        let sign = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
        self.spent = Some(Spent {
            body: Body {
                alt: self.rocket.alt,
                v: self.rocket.v - 3.0,
                m: dry1,
            },
            theta: 0.0,
            omega: sign * ctx.rng.random_range(1.3..2.2),
        });
        self.rocket.m = M02;
        // A puff of separation gas around the interstage.
        let (rows, upper) = self.sprite();
        let y = self.rocket.alt + (rows.len() - upper) as f64 * self.row_m();
        for _ in 0..14 {
            let (gx, gy) = gaussian2(&mut ctx.rng, 9.0);
            self.vapour.spawn(Puff {
                x: 0.0,
                y,
                vx: 2.0 * gx,
                vy: self.rocket.v + gy,
                age: 0.0,
                life: ctx.rng.random_range(0.6..1.4),
            });
        }
    }

    /// Emit exhaust at the nozzle over a substep ending `left` seconds before the end of
    /// the frame; each particle is pre-advanced so emission is smooth for any `dt`.
    fn emit_exhaust(&mut self, p: Phase, h: f64, left: f64, ctx: &mut Ctx) {
        let (burn, v) = match p {
            Phase::Ignition => (stage1(), 0.0),
            Phase::Stage1 => (stage1(), self.rocket.v),
            Phase::Stage2 => (stage2(), self.rocket.v),
            _ => return,
        };
        let y0 = self.nozzle_alt(p);
        // Engines far above the top of the screen no longer feed visible smoke.
        if self.row_of(y0) < -4.0 {
            return;
        }
        // The high-altitude trail lingers; a lower rate keeps the pool in budget
        // (steady state ≈ rate · mean life).
        let life = if p == Phase::Stage2 {
            LIFE_TRAIL
        } else {
            LIFE_MEAN
        };
        let rate = 0.92 * self.exhaust.capacity() as f64 / life;
        self.exhaust_acc += rate * h;
        let half_cell = 0.5 * self.row_m() / ASPECT;
        while self.exhaust_acc >= 1.0 {
            self.exhaust_acc -= 1.0;
            let ang = gaussian(&mut ctx.rng, 0.0, SPREAD);
            let speed = burn.ve * ctx.rng.random_range(0.55..1.0);
            let mut q = Puff {
                x: ctx.rng.random_range(-half_cell..=half_cell),
                y: y0,
                vx: speed * ang.sin(),
                vy: v - speed * ang.cos(),
                age: 0.0,
                life: life + LIFE_SPREAD * ctx.rng.random_range(-1.0..1.0),
            };
            let ay = exhaust_buoyancy(&q);
            drift(&mut q, ay, DRAG, left);
            let side = if q.vx >= 0.0 { 1.0 } else { -1.0 };
            hit_ground(&mut q, side);
            if self.exhaust.spawn(q) {
                self.spawned += 1;
            }
        }
    }

    fn emit_vapour(&mut self, h: f64, ctx: &mut Ctx) {
        let rows = self.sprite().0;
        self.vapour_acc += 22.0 * h;
        while self.vapour_acc >= 1.0 {
            self.vapour_acc -= 1.0;
            let side = if ctx.rng.random_bool(0.5) { 1.0 } else { -1.0 };
            // Boil-off vents anywhere along the stack's tanks.
            let row = ctx.rng.random_range(0.0..rows.len() as f64);
            self.vapour.spawn(Puff {
                x: side * 0.6 * self.row_m(),
                y: row * self.row_m(),
                vx: side * ctx.rng.random_range(3.0..9.0),
                vy: ctx.rng.random_range(-1.0..1.5),
                age: 0.0,
                life: ctx.rng.random_range(0.9..1.8),
            });
        }
    }

    /// Advance vehicle, camera and emitters over `[t, t+h]`, which lies inside one phase.
    fn substep(&mut self, h: f64, left: f64, ctx: &mut Ctx) {
        let p = phase_at(self.t + 0.5 * h);
        match p {
            Phase::Countdown => self.emit_vapour(h, ctx),
            Phase::Ignition => {}
            Phase::Stage1 => integrate(&mut self.rocket, Some(&stage1()), G, h),
            Phase::Stage2 => integrate(&mut self.rocket, Some(&stage2()), G, h),
            Phase::Coast | Phase::Done => integrate(&mut self.rocket, None, G, h),
        }
        if engine_on(p) {
            self.burned = true;
            self.emit_exhaust(p, h, left, ctx);
        }
        if let Some(s) = &mut self.spent {
            integrate(&mut s.body, None, G, h);
            s.theta = (s.theta + s.omega * h).rem_euclid(TAU);
        }
        match self.cam_v {
            None => self.cam = self.cam.max(self.rocket.alt - self.follow_alt),
            Some(v) => {
                self.cam += v * h;
                // Once the upper stage has climbed out of the frame the camera eases to a
                // stop and lets the exhaust trail hang in the sky.
                let nose = self.rocket.alt + self.sprite().0.len() as f64 * self.row_m();
                if self.row_of(nose) < -1.0 {
                    self.cam_v = Some(v * (-CAM_EASE * h).exp());
                }
            }
        }
        self.t += h;
    }

    /// Discrete events at the phase boundaries (`t` has just been set to `e` exactly).
    fn on_event(&mut self, e: f64, ctx: &mut Ctx) {
        if e == T_SEP {
            self.separate(ctx);
        } else if e == T_IGN2 {
            // From here on the camera coasts at the upper stage's speed, so the
            // accelerating stage climbs out of the top of the frame.
            self.cam_v = Some(self.rocket.v);
        } else if e == CYCLE {
            self.restart(ctx);
        }
    }

    fn age_particles(&mut self, dt: f64, ctx: &mut Ctx) {
        let turb = TURB * dt.sqrt();
        let rng = &mut ctx.rng;
        self.exhaust.retain(|p| {
            let (kx, ky) = gaussian2(rng, turb);
            p.vx += kx;
            p.vy += ky;
            let side = if p.vx >= 0.0 { 1.0 } else { -1.0 };
            drift(p, exhaust_buoyancy(p), DRAG, dt);
            hit_ground(p, side);
            p.age < p.life
        });
        self.vapour.retain(|p| {
            let (kx, ky) = gaussian2(rng, 0.4 * turb);
            p.vx += kx;
            p.vy += ky;
            drift(p, VAPOUR_SINK, 1.5, dt);
            hit_ground(p, if p.vx >= 0.0 { 1.0 } else { -1.0 });
            p.age < p.life
        });
    }
}

impl Generator for RocketLaunch {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        let (w, h) = (size.width(), size.height());
        self.width = w;
        self.height = h;
        // Ground line two rows from the bottom when there is room; otherwise the pad is
        // just below the screen.
        let ground = if h >= 6 { h - 2 } else { h };
        self.base_row = f64::from(ground) - 1.0;
        self.cx = (f64::from(w) * 0.5).floor();
        // Scale so the rocket reaches 60 % of the height 5 s after lift-off; the follow
        // altitude in metres is therefore the same at every size.
        self.follow_alt = rocket_altitude(&stage1(), G, FOLLOW_TAU);
        self.k = (0.6 * self.base_row).max(0.6) / self.follow_alt;
        let cap = ((EXHAUST_PER_CELL * size.area() as f64) as usize).min(EXHAUST_MAX);
        self.exhaust.set_capacity(cap);
        self.vapour.set_capacity(VAPOUR_MAX.min(size.area() / 8));
        if !self.ready {
            self.ready = true;
            self.restart(ctx);
        }
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if !self.ready {
            self.resize(ctx.size(), ctx);
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.spawned = 0;
        self.burned = false;
        self.age_particles(dt, ctx);
        // Substeps of ≤ H_MAX that never straddle an event, landing exactly on each one so
        // the dynamics switch form (and separation happens) at the scheduled instant.
        let mut left = dt;
        while left > 1e-15 {
            let next = EVENTS
                .iter()
                .copied()
                .find(|&e| e > self.t + 1e-12)
                .unwrap_or(CYCLE);
            let to_event = next - self.t;
            let h = left.min(H_MAX);
            let hits = to_event <= h + 1e-12;
            let h = if hits { to_event } else { h };
            left = (left - h).max(0.0);
            self.substep(h, left, ctx);
            if hits {
                self.t = next;
                self.on_event(next, ctx);
            }
        }
        self.flicker = ctx.rng.random();
    }

    fn render(&self, canvas: &mut Canvas) {
        if !self.ready {
            return;
        }
        let (rows, upper) = self.sprite();
        let n = rows.len();
        self.draw_clouds(canvas);
        self.draw_ground(canvas, n);
        self.draw_particles(canvas);
        if let Some(s) = &self.spent {
            self.draw_spent(canvas, s, n - upper);
        }
        // The stack's base row; after separation only the upper stage remains, in place.
        let base = self.row_of(self.rocket.alt).round();
        let shown = if self.spent.is_some() { upper } else { n };
        let nozzle = base - (n - shown) as f64;
        let p = phase_at(self.t);
        if engine_on(p) {
            self.draw_flame(canvas, nozzle, n);
        }
        for (i, row) in rows.iter().take(shown).enumerate() {
            let y = base - (n - 1 - i) as f64;
            for (j, &g) in row.iter().enumerate() {
                if g != NONE {
                    let edge = j != 1;
                    let ink = Ink::new(Role::Text, if edge { 180 } else { 235 });
                    plot(canvas, self.cx + j as f64 - 1.0, y, g, ink);
                }
            }
        }
        let nose = base - (n - 1) as f64;
        let callout = (self.cx as i32 + 4, (nose + 1.0).clamp(-1.0, 1e4) as i32);
        self.draw_captions(canvas, p, callout);
    }
}

impl RocketLaunch {
    fn draw_clouds(&self, canvas: &mut Canvas) {
        let half = self.cx.max(1.0);
        for c in self.clouds.iter().flatten() {
            let y = self.row_of(c.alt).round();
            let x0 = (self.cx + c.fx * half).round();
            for i in 0..c.len {
                let edge = i == 0 || i + 1 == c.len;
                let g = if edge {
                    Glyph::ascii('.')
                } else {
                    Glyph::ascii('~')
                };
                plot(canvas, x0 + f64::from(i), y, g, Ink::new(Role::Cold, 70));
                if !edge && i % 3 == 1 {
                    plot(
                        canvas,
                        x0 + f64::from(i) + 1.0,
                        y - 1.0,
                        Glyph::ascii('-'),
                        Ink::new(Role::Cold, 50),
                    );
                }
            }
        }
    }

    /// Ground, pad and launch tower, all fixed in the world so they scroll away.
    fn draw_ground(&self, canvas: &mut Canvas, n: usize) {
        let pad_row = self.row_of(0.0).round();
        let h = f64::from(self.height);
        let w = f64::from(self.width);
        if pad_row + 1.0 < h {
            let ground = Ink::new(Role::Structure, 170);
            let soil = Ink::new(Role::Structure, 55);
            let mut x = 0.0;
            while x < w {
                let on_pad = (x - self.cx).abs() <= 3.0;
                let g = if on_pad {
                    Glyph::new('▀', '#')
                } else {
                    Glyph::new('━', '=')
                };
                plot(canvas, x, pad_row + 1.0, g, ground);
                let mut y = pad_row + 2.0;
                while y < h {
                    let g = if (x as i64 + y as i64) % 3 == 0 {
                        Glyph::new('░', '.')
                    } else {
                        Glyph::ascii(' ')
                    };
                    if g.ascii != ' ' {
                        plot(canvas, x, y, g, soil);
                    }
                    y += 1.0;
                }
                x += 1.0;
            }
        }
        // Launch tower: a lattice column left of the pad with umbilical arms that swing
        // clear at ignition, and a blinking beacon on top.
        if n < 3 || self.width < 12 {
            return;
        }
        let tx = self.cx - 4.0;
        let top = pad_row - n as f64;
        let steel = Ink::new(Role::Structure, 190);
        let mut y = pad_row;
        while y >= top {
            plot(canvas, tx - 1.0, y, Glyph::new('║', 'H'), steel);
            plot(
                canvas,
                tx,
                y,
                Glyph::new('╳', 'X'),
                Ink::new(Role::Structure, 140),
            );
            y -= 1.0;
        }
        if self.t < T_IGN {
            for arm in [1.0, n as f64 - 2.0] {
                let ay = pad_row - (n as f64 - 1.0) + arm;
                let mut x = tx + 1.0;
                while x < self.cx - 1.0 {
                    plot(canvas, x, ay, Glyph::new('─', '-'), steel);
                    x += 1.0;
                }
            }
        }
        if (self.t * 1.25).fract() < 0.5 {
            plot(
                canvas,
                tx - 1.0,
                top - 1.0,
                Glyph::new('•', '*'),
                Ink::new(Role::Hot, 230),
            );
        }
    }

    fn draw_particles(&self, canvas: &mut Canvas) {
        // Smoke first, then the hot core on top of it.
        for p in &self.exhaust {
            if p.temperature() > HOT_T {
                continue;
            }
            let frac = (p.age / p.life).clamp(0.0, 1.0);
            let (g, n) = if frac < 0.4 {
                (Glyph::new('▒', '@'), 2)
            } else if frac < 0.75 {
                (Glyph::new('░', 'o'), 1)
            } else {
                (Glyph::new('·', '.'), 0)
            };
            let level = (70.0 + 160.0 * (1.0 - frac)) as u8;
            let (x, y) = (self.col_of(p.x), self.row_of(p.y));
            plot(canvas, x, y, g, Ink::new(Role::Dim, level));
            // Denser puffs are wider than a cell: billow to a side fixed per particle.
            if n >= 1 {
                let side = if (p.life * 1e3) as i64 % 2 == 0 {
                    1.0
                } else {
                    -1.0
                };
                let faint = Ink::new(Role::Dim, level / 2 + 30);
                plot(canvas, x + side, y, Glyph::new('░', 'o'), faint);
            }
        }
        for p in &self.vapour {
            let frac = (p.age / p.life).clamp(0.0, 1.0);
            let g = if frac < 0.5 {
                Glyph::new('░', '~')
            } else {
                Glyph::new('·', '.')
            };
            let level = (80.0 + 150.0 * (1.0 - frac)) as u8;
            plot(
                canvas,
                self.col_of(p.x),
                self.row_of(p.y),
                g,
                Ink::new(Role::Cold, level),
            );
        }
        for p in &self.exhaust {
            let t = p.temperature();
            if t <= HOT_T {
                continue;
            }
            let (g, ink) = if t > 0.55 {
                (Glyph::new('▓', '#'), Ink::new(Role::Accent, 255))
            } else {
                (
                    Glyph::new('▒', '*'),
                    Ink::new(Role::Hot, (130.0 + 200.0 * t) as u8),
                )
            };
            plot(canvas, self.col_of(p.x), self.row_of(p.y), g, ink);
        }
    }

    fn draw_flame(&self, canvas: &mut Canvas, nozzle: f64, n: usize) {
        let len = match n {
            6.. => 2 + (self.flicker * 2.99) as usize,
            3.. => 1 + (self.flicker * 1.99) as usize,
            _ => 1,
        };
        for (i, &(g, role, level)) in FLAME.iter().take(len).enumerate() {
            plot(
                canvas,
                self.cx,
                nozzle + 1.0 + i as f64,
                g,
                Ink::new(role, level),
            );
        }
        if n >= 3 {
            let side = Ink::new(Role::Hot, 200);
            plot(
                canvas,
                self.cx - 1.0,
                nozzle + 1.0,
                Glyph::new('▒', '*'),
                side,
            );
            plot(
                canvas,
                self.cx + 1.0,
                nozzle + 1.0,
                Glyph::new('▒', '*'),
                side,
            );
        }
    }

    /// The spent first stage: a short rod tumbling end over end as it falls.
    fn draw_spent(&self, canvas: &mut Canvas, s: &Spent, len: usize) {
        if len == 0 {
            return;
        }
        // Glyph approximating the rod's on-screen direction.
        let octant = PI / 8.0;
        let phi = s.theta.rem_euclid(PI);
        let g = if !(octant..7.0 * octant).contains(&phi) {
            Glyph::new('│', '|')
        } else if phi < 3.0 * octant {
            Glyph::new('╱', '/')
        } else if phi < 5.0 * octant {
            Glyph::new('─', '-')
        } else {
            Glyph::new('╲', '\\')
        };
        let (sin, cos) = s.theta.sin_cos();
        let mid = (len as f64 - 1.0) / 2.0;
        let (cx, cy) = (self.cx, self.row_of(s.body.alt) - mid);
        let ink = Ink::new(Role::Text, 170);
        // Off-screen (or non-finite) positions: nothing to draw, and no unsafe casts.
        let reach = mid * ASPECT + 1.0;
        let h = f64::from(self.height);
        if !(-reach..h + reach).contains(&cy) {
            return;
        }
        if len == 1 {
            plot(canvas, cx, cy, g, ink);
            return;
        }
        // Rasterise the rod end to end so a horizontal rod has no ASPECT gaps.
        let (dx, dy) = (mid * sin * ASPECT, mid * cos);
        let (x0, y0) = ((cx - dx).round() as i32, (cy + dy).round() as i32);
        let (x1, y1) = ((cx + dx).round() as i32, (cy - dy).round() as i32);
        for (x, y) in sciart_core::geom::line(x0, y0, x1, y1) {
            canvas.put(x, y, g, ink);
        }
    }

    fn draw_captions(&self, canvas: &mut Canvas, p: Phase, callout: (i32, i32)) {
        if self.height < 8 {
            return;
        }
        let y = i32::from(self.height / 5);
        let ink = Ink::new(Role::Text, 235);
        match p {
            Phase::Countdown => {
                // "T−3", "T−2", "T−1" across the four-second countdown.
                let n = 3 - (self.t / (T_IGN / 3.0)).floor().clamp(0.0, 2.0) as u8;
                let x = i32::from(self.width) / 2 - 1;
                canvas.put(x, y, Glyph::ascii('T'), ink);
                canvas.put(x + 1, y, MINUS, ink);
                canvas.put(x + 2, y, Glyph::ascii(char::from(b'0' + n)), ink);
            }
            Phase::Ignition => draw::text_centered(canvas, y, "IGNITION", ink),
            Phase::Stage1 if self.t < T_LIFT + 1.5 => {
                draw::text_centered(canvas, y, "LIFTOFF", ink);
            }
            // In flight the rocket sits high on screen: call out events beside it.
            Phase::Coast => canvas.text(callout.0, callout.1, "STAGE SEPARATION", ink),
            Phase::Stage2 if self.t < T_IGN2 + 1.2 => {
                canvas.text(callout.0, callout.1, "STAGE 2 IGNITION", ink);
            }
            _ => {}
        }
        if self.height >= 12 && self.width >= 44 {
            self.draw_telemetry(canvas);
        }
    }

    /// `T+07.3 s  ALT 1234 m  VEL 102 m/s` in the top-left corner, formatted without
    /// allocating.
    fn draw_telemetry(&self, canvas: &mut Canvas) {
        use std::fmt::Write;
        let mut buf = LineBuf::default();
        let met = self.t - T_LIFT;
        let sign = if met < 0.0 { '-' } else { '+' };
        let _ = write!(
            buf,
            "T{sign}{:04.1} s  ALT {:>5.0} m  VEL {:>4.0} m/s",
            met.abs(),
            self.rocket.alt,
            self.rocket.v
        );
        canvas.text(1, 0, buf.as_str(), Ink::new(Role::Text, 140));
    }
}

/// Temperature above which exhaust is drawn as flame rather than smoke.
const HOT_T: f64 = 0.28;

/// A fixed-capacity text buffer, so `render` can format numbers without allocating.
struct LineBuf {
    bytes: [u8; 64],
    len: usize,
}

impl Default for LineBuf {
    fn default() -> Self {
        Self {
            bytes: [0; 64],
            len: 0,
        }
    }
}

impl LineBuf {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl std::fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len + s.len();
        if end > self.bytes.len() {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(w: u16, h: u16) -> (RocketLaunch, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(7, size);
        let mut g = RocketLaunch::new();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    /// Step until the cycle clock reaches `t` (s).
    fn run_to(g: &mut RocketLaunch, ctx: &mut Ctx, t: f64, dt: f64) {
        while g.t + 1e-9 < t {
            g.update(dt.min(t - g.t), ctx);
        }
    }

    fn assert_rocket_equation(g: &RocketLaunch) {
        let b = stage1();
        let tau = g.t - T_LIFT;
        let va = rocket_velocity(b.ve, b.m0, g.rocket.m, G, tau);
        let err = (g.rocket.v - va).abs();
        assert!(
            err <= 0.01 * va.abs() + 1e-9,
            "t={:.3}: v={} vs rocket eq {va}",
            g.t,
            g.rocket.v
        );
        // Mass bookkeeping: m = m0 − ṁ τ.
        assert!((g.rocket.m - (b.m0 - b.mdot() * tau)).abs() < 1e-6 * b.m0);
    }

    #[test]
    fn roc_a1_first_stage_velocity_follows_the_rocket_equation() {
        let (mut g, mut ctx) = fresh(100, 30);
        run_to(&mut g, &mut ctx, T_LIFT + 0.2, SIM_DT);
        while g.t < T_SEP - 0.05 {
            assert_eq!(phase_at(g.t), Phase::Stage1);
            assert_rocket_equation(&g);
            let alt = rocket_altitude(&stage1(), G, g.t - T_LIFT);
            assert!((g.rocket.alt - alt).abs() <= 0.01 * alt, "altitude {alt}");
            g.update(SIM_DT, &mut ctx);
        }
        // Burnout: the full Δv of the stage, less gravity loss.
        run_to(&mut g, &mut ctx, T_SEP - 1e-9, SIM_DT);
        let b = stage1();
        let v_bo = b.ve * (b.m0 / b.m_dry).ln() - G * BURN1;
        assert!(
            (g.rocket.v - v_bo).abs() < 0.01 * v_bo,
            "{} vs {v_bo}",
            g.rocket.v
        );
    }

    #[test]
    fn roc_a1_holds_for_coarse_and_uneven_steps() {
        let (mut g, mut ctx) = fresh(80, 24);
        let steps = [0.25, 0.013, 0.2, 1.0 / 60.0, 0.07];
        let mut i = 0;
        while g.t < T_SEP - 0.3 {
            g.update(steps[i % steps.len()], &mut ctx);
            i += 1;
            if phase_at(g.t) == Phase::Stage1 && g.t > T_LIFT + 0.1 {
                assert_rocket_equation(&g);
            }
        }
    }

    #[test]
    fn roc_a1_rocket_equation_reference_values() {
        // No burn ⇒ no speed; no gravity ⇒ pure Tsiolkovsky ve·ln(R).
        assert_eq!(rocket_velocity(3000.0, 10.0, 10.0, 9.81, 0.0), 0.0);
        let v = rocket_velocity(3000.0, 10.0, 10.0 / std::f64::consts::E, 0.0, 5.0);
        assert!((v - 3000.0).abs() < 1e-9);
        assert!(
            (rocket_velocity(100.0, 4.0, 2.0, 10.0, 1.0) - (100.0 * 2f64.ln() - 10.0)).abs()
                < 1e-12
        );
        // Liftoff needs F > m0 g, and the stages are sized so both lift off.
        for b in [stage1(), stage2()] {
            assert!(b.f > b.m0 * G);
            assert!((b.mdot() - b.f / b.ve).abs() < 1e-12);
        }
        assert!(rocket_altitude(&stage1(), G, 0.0).abs() < 1e-12);
    }

    #[test]
    fn roc_a2_exhaust_only_while_an_engine_burns_and_pool_is_bounded() {
        let (mut g, mut ctx) = fresh(120, 40);
        let mut total = 0;
        let mut t = 0.0;
        while t < 2.0 * CYCLE {
            g.update(SIM_DT, &mut ctx);
            t += SIM_DT;
            if g.spawned > 0 {
                assert!(g.burned, "exhaust spawned with no engine at t={}", g.t);
            }
            if !engine_on(phase_at(g.t - 1e-9)) && !g.burned {
                assert_eq!(g.spawned, 0);
            }
            assert!(g.exhaust.len() <= g.exhaust.capacity());
            assert!(g.vapour.len() <= g.vapour.capacity());
            total += g.spawned;
        }
        assert!(total > 100, "engines produced exhaust: {total}");
        // Countdown is pure vapour.
        let (mut g, mut ctx) = fresh(120, 40);
        run_to(&mut g, &mut ctx, T_IGN - 0.1, SIM_DT);
        assert!(g.exhaust.is_empty());
        assert!(!g.vapour.is_empty());
    }

    #[test]
    fn roc_a2_pool_stays_bounded_across_resizes_and_huge_steps() {
        let (mut g, mut ctx) = fresh(200, 50);
        for (w, h) in [(200, 50), (30, 10), (1, 1), (250, 80), (80, 24)] {
            let size = Size::new(w, h);
            ctx.set_size(size);
            g.resize(size, &mut ctx);
            for i in 0..300 {
                g.update(if i % 5 == 0 { 0.25 } else { SIM_DT }, &mut ctx);
                assert!(g.exhaust.len() <= g.exhaust.capacity());
                assert!(g.exhaust.capacity() <= EXHAUST_MAX);
            }
        }
    }

    #[test]
    fn roc_a3_altitude_non_decreasing_until_separation_and_cycle_restarts() {
        let (mut g, mut ctx) = fresh(100, 30);
        let mut last = 0.0;
        while g.t < T_SEP - 1e-9 {
            g.update(SIM_DT, &mut ctx);
            if g.t < T_LIFT {
                assert_eq!(g.rocket.alt, 0.0, "held down at t={}", g.t);
            }
            assert!(g.rocket.alt >= last, "altitude fell at t={}", g.t);
            last = g.rocket.alt;
        }
        assert!(last > g.follow_alt, "camera-follow altitude reached");
        run_to(&mut g, &mut ctx, CYCLE - 0.1, SIM_DT);
        assert!(g.spent.is_some());
        assert!(g.rocket.alt > 0.0);
        g.update(0.2, &mut ctx);
        assert!(g.t < 0.2, "cycle wrapped: t={}", g.t);
        assert_eq!(g.rocket.alt, 0.0);
        assert!(g.spent.is_none());
        assert_eq!(g.cam, 0.0);
    }

    #[test]
    fn roc_phases_follow_the_timeline() {
        assert_eq!(phase_at(0.0), Phase::Countdown);
        assert_eq!(phase_at(4.5), Phase::Ignition);
        assert_eq!(phase_at(6.0), Phase::Stage1);
        assert_eq!(phase_at(T_SEP + 0.1), Phase::Coast);
        assert_eq!(phase_at(T_IGN2 + 0.1), Phase::Stage2);
        assert_eq!(phase_at(CYCLE - 0.1), Phase::Done);
    }

    #[test]
    fn roc_sprite_is_tall_on_big_screens_and_one_glyph_when_tiny() {
        assert!(sprite(20).0.len() >= 5);
        assert!(sprite(50).0.len() >= 5);
        assert_eq!(sprite(5).0.len(), 1);
        let mut stage = Stage::new(build(), 1, Size::new(80, 30), GlyphMode::Ascii);
        let f = stage.render();
        assert!(
            f.to_lines().iter().any(|l| l.contains("/#\\")),
            "rocket drawn"
        );
    }
}
