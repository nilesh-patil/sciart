//! A fluid layer heated from below turns over in convection rolls (SPEC-002
//! `rayleigh_benard`).
//!
//! Model: 2-D Boussinesq convection between free-slip plates as a truncated Galerkin
//! (Saltzman-type) Fourier model, in thermal-diffusion units (layer depth 1, time d²/κ):
//!
//! ```text
//! ψ = Σ a_km sin(α_k x) sin(mπz),  θ = Σ b_km cos(α_k x) sin(mπz),  α_k = kπ/L,
//! ∂t ζ + u·∇ζ = Pr Ra ∂x θ + Pr ∇²ζ   (ζ = ∇²ψ),
//! ∂t θ + u·∇θ = w + ∇²θ,               (u, w) = (−∂z ψ, ∂x ψ).
//! ```
//!
//! `θ` is the anomaly from the conduction profile `T = 1 − z`; its `k = 0` modes are the
//! Lorenz/Saltzman mean-temperature terms `c_m sin(mπz)`. The advection terms are
//! evaluated pseudo-spectrally on a midpoint grid fine enough (`> 3K/2 × 3M/2`) that the
//! quadrature is exact for every product, so the projection is a true Galerkin one and
//! the truncated model keeps the advection's quadratic invariants (it cannot blow up).
//! Diffusion is integrated exactly (integrating-factor RK4); buoyancy and advection
//! explicitly, under a step bound. `Ra = 100 Ra_c`, `Pr = 10` (cool water) keep the rolls
//! time-dependent for every box aspect a terminal gives (1.2–3.5 was surveyed): plumes
//! sway, merge and split, and the rolls shift.
//!
//! Look (GEN-R26): the horizontal temperature anomaly `θ' = θ − ⟨θ⟩ₓ` (the `k ≥ 1`
//! modes: rising warm plumes and sinking cool sheets, not the mean profile) as
//! fixed-threshold stipple, `Hot` warm, `Cold` cool, density
//! `P_MAX·(|θ'| − θ₀)/(θ_ref − θ₀)` capped at `P_MAX` (only plume cores; the mixed
//! interior stays black); tracer threads advected through the analytic velocity,
//! coloured by speed, with a short afterglow; the hot floor and cold lid as 1-px threads,
//! the box inset from the frame and the title row. The flow opens on the growing
//! two-roll eigenmode (a central warm plume) and noise breaks it into wandering rolls.

use std::f64::consts::PI;

use rand::Rng;
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};

pub static META: Meta = Meta {
    name: "rayleigh_benard",
    category: Category::Physics,
    description: "A fluid layer heated from below turns over in convection rolls",
    title: "Convection · heated from below",
    captions: &[],
};

pub fn build() -> Box<dyn Generator> {
    Box::new(RayleighBenard::default())
}

/// Critical Rayleigh number of the free-slip layer, `27π⁴/4`.
pub(crate) const RA_C: f64 = 27.0 * PI * PI * PI * PI / 4.0;
/// Rayleigh number of the scene.
pub(crate) const RA: f64 = 100.0 * RA_C;
/// Prandtl number.
pub(crate) const PR: f64 = 10.0;
/// Vertical modes `m = 1..=M`.
pub(crate) const M_MODES: usize = 10;
/// Horizontal modes per unit box length (`K = round(K_PER_L·L)`, clamped).
const K_PER_L: f64 = 8.0;
const K_MIN: usize = 4;
const K_MAX: usize = 32;
/// Box aspect range of the model (the drawn box always spans the screen).
const L_MIN: f64 = 0.5;
const L_MAX: f64 = 4.0;
/// Largest model step (thermal-diffusion times) of one integrating-factor RK4 step.
const DT_MAX: f64 = 4.0e-4;
/// Advective Courant number of the step bound.
const CFL: f64 = 1.5;
/// Model time per simulated second.
pub(crate) const RATE: f64 = 0.006;
/// The opening flow: the `(k, m) = (2, 1)` linear eigenmode (a central warm plume
/// between two counter-rotating rolls) at this temperature amplitude, so readable rolls
/// show within the first second, plus random noise on every other mode, so the flow
/// leaves that pattern and wanders.
const SEED_AMP: f64 = 0.12;
const SEED_NOISE: f64 = 0.005;

/// Stipple: peak keep probability, the anomaly that reaches it, and the brighter step.
pub(crate) const P_MAX: f64 = 0.30;
pub(crate) const THETA_REF: f64 = 0.35;
/// Below this anomaly no dot is lit: the well-mixed interior stays black and the plumes
/// and boundary sheets carry the stipple.
pub(crate) const THETA_FLOOR: f64 = 0.18;
const BRIGHT: f64 = 0.7;
const FIELD_SALT: u64 = 0x5BE0_0000_0000_0001;
pub(crate) const HOT: [Ink; 2] = [Ink::new(Role::Hot, 170), Ink::new(Role::Hot, 235)];
pub(crate) const COLD: [Ink; 2] = [Ink::new(Role::Cold, 170), Ink::new(Role::Cold, 235)];
pub(crate) const FLOOR_INK: Ink = Ink::new(Role::Hot, 255);
pub(crate) const LID_INK: Ink = Ink::new(Role::Cold, 255);
/// Tracer inks by speed (slow, medium, fast), and the afterglow's dimmer tail.
pub(crate) const TRACER: [Ink; 3] = [
    Ink::new(Role::Wave, 220),
    Ink::new(Role::Particle, 190),
    Ink::new(Role::Particle, 255),
];
pub(crate) const TRACER_TAIL: [Ink; 3] = [
    Ink::new(Role::Wave, 120),
    Ink::new(Role::Particle, 110),
    Ink::new(Role::Particle, 140),
];
/// Speed tiers as fractions of the RMS speed.
const TIER: [f64; 2] = [0.8, 1.4];
/// Afterglow length in steps; its oldest quarter (`len / TAIL_DIV`) is drawn dim.
const TRAIL: usize = 20;
const TAIL_DIV: usize = 4;
/// Cell columns left blank at each side of the box.
const INSET_COLS: u16 = 2;
/// Tracers per terminal cell of the box, and their bounds.
const TRACER_DENSITY: f64 = 1.0 / 330.0;
const TRACERS_MIN: usize = 6;
const TRACERS_MAX: usize = 40;
/// Tracers keep this far (box units) from the walls.
const WALL_EPS: f64 = 1e-4;

/// Index of mode `(k, m)` (`m ≥ 1`) in a coefficient array of `M_MODES` per `k`.
#[inline]
fn idx(k: usize, m: usize) -> usize {
    k * M_MODES + (m - 1)
}

/// The six grid fields of the explicit tendencies.
#[derive(Debug, Clone, Copy)]
enum Field {
    U,
    W,
    ZetaX,
    ZetaZ,
    ThetaX,
    ThetaZ,
}

impl Field {
    const ALL: [Field; 6] = [
        Field::U,
        Field::W,
        Field::ZetaX,
        Field::ZetaZ,
        Field::ThetaX,
        Field::ThetaZ,
    ];

    /// `(x basis is sine, z basis is sine)`.
    fn basis(self) -> (bool, bool) {
        match self {
            Field::U | Field::ZetaZ => (true, false),
            Field::W | Field::ZetaX => (false, true),
            Field::ThetaX => (true, true),
            Field::ThetaZ => (false, false),
        }
    }
}

/// The truncated Galerkin model and its pseudo-spectral workspace.
#[derive(Debug, Clone)]
pub(crate) struct Model {
    /// Box length (the depth is 1).
    pub l: f64,
    /// Horizontal modes `k = 0..=kn`.
    pub kn: usize,
    pub ra: f64,
    pub pr: f64,
    /// Streamfunction coefficients `a_km` (`k = 0` entries stay 0).
    pub a: Vec<f64>,
    /// Temperature coefficients `b_km`.
    pub b: Vec<f64>,
    /// Quadrature grid.
    nx: usize,
    nz: usize,
    /// `sin(α_k x_i)`, `cos(α_k x_i)` as `[k·nx + i]`; `sin(mπz_j)`, `cos(mπz_j)` as
    /// `[(m−1)·nz + j]`.
    sx: Vec<f64>,
    cx: Vec<f64>,
    sz: Vec<f64>,
    cz: Vec<f64>,
    /// Per mode: `α_k`, `q² = α_k² + (mπ)²`, and the six fields' synthesis factors.
    alpha: Vec<f64>,
    q2: Vec<f64>,
    fac: [Vec<f64>; 6],
    /// Half-step decay factors `e^{−λh/2}` of `a` and `b` for the step `h_cached`.
    h_cached: f64,
    ea: Vec<f64>,
    eb: Vec<f64>,
    /// Scratch: scaled coefficients, per-mode partial sums, grid fields, RK stages.
    coef: Vec<f64>,
    part: Vec<f64>,
    f: [Vec<f64>; 6],
    stage: [Vec<f64>; 10],
}

// Mode loops index several parallel per-mode tables at once.
#[allow(clippy::needless_range_loop)]
impl Model {
    pub(crate) fn new(l: f64, ra: f64, pr: f64) -> Self {
        let l = l.clamp(L_MIN, L_MAX);
        let kn = ((K_PER_L * l).round() as usize).clamp(K_MIN, K_MAX);
        let (nx, nz) = (3 * kn / 2 + 2, 3 * M_MODES / 2 + 2);
        let n = (kn + 1) * M_MODES;
        let mut alpha = vec![0.0; n];
        let mut q2 = vec![0.0; n];
        let mut fac: [Vec<f64>; 6] = std::array::from_fn(|_| vec![0.0; n]);
        for k in 0..=kn {
            let al = k as f64 * PI / l;
            for m in 1..=M_MODES {
                let (i, g) = (idx(k, m), m as f64 * PI);
                alpha[i] = al;
                q2[i] = al * al + g * g;
                // ψ-fields vanish for k = 0 (sin 0); θ-fields keep the mean modes.
                let psi = if k == 0 { 0.0 } else { 1.0 };
                fac[Field::U as usize][i] = -g * psi;
                fac[Field::W as usize][i] = al * psi;
                fac[Field::ZetaX as usize][i] = -q2[i] * al * psi;
                fac[Field::ZetaZ as usize][i] = -q2[i] * g * psi;
                fac[Field::ThetaX as usize][i] = -al;
                fac[Field::ThetaZ as usize][i] = g;
            }
        }
        let mut sx = vec![0.0; (kn + 1) * nx];
        let mut cx = vec![0.0; (kn + 1) * nx];
        for k in 0..=kn {
            for i in 0..nx {
                let x = (i as f64 + 0.5) * l / nx as f64;
                let (s, c) = (k as f64 * PI / l * x).sin_cos_det();
                sx[k * nx + i] = s;
                cx[k * nx + i] = c;
            }
        }
        let mut sz = vec![0.0; M_MODES * nz];
        let mut cz = vec![0.0; M_MODES * nz];
        for m in 1..=M_MODES {
            for j in 0..nz {
                let z = (j as f64 + 0.5) / nz as f64;
                let (s, c) = (m as f64 * PI * z).sin_cos_det();
                sz[(m - 1) * nz + j] = s;
                cz[(m - 1) * nz + j] = c;
            }
        }
        Self {
            l,
            kn,
            ra,
            pr,
            a: vec![0.0; n],
            b: vec![0.0; n],
            nx,
            nz,
            sx,
            cx,
            sz,
            cz,
            alpha,
            q2,
            fac,
            h_cached: f64::NAN,
            ea: vec![0.0; n],
            eb: vec![0.0; n],
            coef: vec![0.0; n],
            part: vec![0.0; (M_MODES * nx).max(nz * (kn + 1))],
            f: std::array::from_fn(|_| vec![0.0; nx * nz]),
            stage: std::array::from_fn(|_| vec![0.0; n]),
        }
    }

    /// Copy the overlapping modes of `other` (a resize keeps the flow).
    pub(crate) fn take_modes(&mut self, other: &Model) {
        for k in 0..=self.kn.min(other.kn) {
            for m in 1..=M_MODES {
                self.a[idx(k, m)] = other.a[idx(k, m)];
                self.b[idx(k, m)] = other.b[idx(k, m)];
            }
        }
    }

    /// A small random temperature perturbation (the rolls then grow from it).
    pub(crate) fn perturb(&mut self, rng: &mut impl Rng, amp: f64) {
        for k in 1..=self.kn {
            for m in 1..=M_MODES {
                self.b[idx(k, m)] = amp * (rng.random::<f64>() * 2.0 - 1.0) / (k * m) as f64;
            }
        }
    }

    /// The opening flow: the growing linear eigenmode `(2, 1)` with temperature amplitude
    /// `amp`, warm in the middle (`b < 0` puts `cos(2πx/L) = −1` at the centre), and
    /// `noise` on every other temperature mode.
    pub(crate) fn seed_plume(&mut self, rng: &mut impl Rng, amp: f64, noise: f64) {
        self.perturb(rng, noise);
        let i = idx(2, 1);
        let (al, q2) = (self.alpha[i], self.q2[i]);
        // Growth rate σ of the mode; its eigenvector has ḃ = α a − q² b = σ b.
        let (b, c) = (
            (self.pr + 1.0) * q2,
            self.pr * q2 * q2 - self.pr * self.ra * al * al / q2,
        );
        let sigma = 0.5 * (-b + (b * b - 4.0 * c).max(0.0).sqrt());
        self.b[i] = -amp;
        self.a[i] = (sigma + q2) * self.b[i] / al;
    }

    /// Evaluate field `field` of the coefficients in stage slot `src` on the grid.
    fn synth(&mut self, src: usize, field: Field) {
        let (nx, nz, kn) = (self.nx, self.nz, self.kn);
        let (xs, zs) = field.basis();
        for ((c, &v), &f) in self
            .coef
            .iter_mut()
            .zip(&self.stage[src])
            .zip(&self.fac[field as usize])
        {
            *c = v * f;
        }
        let part = &mut self.part[..M_MODES * nx];
        part.fill(0.0);
        let xt = if xs { &self.sx } else { &self.cx };
        for (m, row) in part.chunks_exact_mut(nx).enumerate() {
            for (k, xk) in xt.chunks_exact(nx).enumerate().take(kn + 1) {
                let c = self.coef[k * M_MODES + m];
                if c != 0.0 {
                    for (p, &x) in row.iter_mut().zip(xk) {
                        *p += c * x;
                    }
                }
            }
        }
        let zt = if zs { &self.sz } else { &self.cz };
        let out = &mut self.f[field as usize];
        out.fill(0.0);
        for (m, row) in part.chunks_exact(nx).enumerate() {
            let zm = &zt[m * nz..(m + 1) * nz];
            for (orow, &z) in out.chunks_exact_mut(nx).zip(zm) {
                for (v, &p) in orow.iter_mut().zip(row) {
                    *v += z * p;
                }
            }
        }
    }

    /// Galerkin projection of grid field `src` onto `X_k(x) sin(mπz)` (normalised so a
    /// field equal to one basis function projects to 1), into stage slot `out`.
    fn project(&mut self, src: Field, xs: bool, out: usize) {
        let (nx, nz, kn) = (self.nx, self.nz, self.kn);
        let xt = if xs { &self.sx } else { &self.cx };
        let f = &self.f[src as usize];
        // part[j·(kn+1) + k] = Σ_i f(i, j) X_k(x_i)
        let part = &mut self.part;
        for (j, row) in f.chunks_exact(nx).enumerate() {
            for (k, xk) in xt.chunks_exact(nx).enumerate().take(kn + 1) {
                part[j * (kn + 1) + k] = row.iter().zip(xk).map(|(a, b)| a * b).sum();
            }
        }
        let base = 4.0 / (nx * nz) as f64;
        let o = &mut self.stage[out];
        for k in 0..=kn {
            let norm = if k == 0 { 0.5 * base } else { base };
            for m in 1..=M_MODES {
                let zm = &self.sz[(m - 1) * nz..m * nz];
                let s: f64 = (0..nz).map(|j| part[j * (kn + 1) + k] * zm[j]).sum();
                o[idx(k, m)] = norm * s;
            }
        }
    }

    /// The explicit tendencies (advection, buoyancy and the `w` source of `θ`) of the
    /// state in stage slots `(a, b)`, written to slots `(da, db)`.
    fn explicit(&mut self, a: usize, b: usize, da: usize, db: usize) {
        for field in Field::ALL {
            let src = match field {
                Field::ThetaX | Field::ThetaZ => b,
                _ => a,
            };
            self.synth(src, field);
        }
        // ζ_x ← N_ζ = u ζ_x + w ζ_z; θ_x ← N_θ = u θ_x + w θ_z
        {
            let [u, w, zx, zz, tx, tz] = &mut self.f;
            for i in 0..u.len() {
                zx[i] = u[i] * zx[i] + w[i] * zz[i];
                tx[i] = u[i] * tx[i] + w[i] * tz[i];
            }
        }
        self.project(Field::ZetaX, true, da);
        self.project(Field::ThetaX, false, db);
        let coupling = self.pr * self.ra;
        for i in 0..self.a.len() {
            if i < M_MODES {
                // k = 0: no streamfunction mode; the mean temperature is only advected.
                self.stage[da][i] = 0.0;
                self.stage[db][i] = -self.stage[db][i];
                continue;
            }
            let (al, q2) = (self.alpha[i], self.q2[i]);
            let (sa, sb) = (self.stage[a][i], self.stage[b][i]);
            self.stage[da][i] = (self.stage[da][i] + coupling * al * sb) / q2;
            self.stage[db][i] = -self.stage[db][i] + al * sa;
        }
    }

    /// One integrating-factor RK4 step of length `h` (diffusion exact).
    pub(crate) fn step(&mut self, h: f64) {
        if self.h_cached != h {
            self.h_cached = h;
            for i in 0..self.a.len() {
                self.ea[i] = (-self.pr * self.q2[i] * h * 0.5).exp();
                self.eb[i] = (-self.q2[i] * h * 0.5).exp();
            }
        }
        let n = self.a.len();
        // Slots: 0, 1 the stage state; 2..=9 the tendencies k1..k4 of (a, b).
        self.stage[0].copy_from_slice(&self.a);
        self.stage[1].copy_from_slice(&self.b);
        self.explicit(0, 1, 2, 3);
        for i in 0..n {
            let (ea, eb) = (self.ea[i], self.eb[i]);
            self.stage[0][i] = ea * (self.a[i] + 0.5 * h * self.stage[2][i]);
            self.stage[1][i] = eb * (self.b[i] + 0.5 * h * self.stage[3][i]);
        }
        self.explicit(0, 1, 4, 5);
        for i in 0..n {
            let (ea, eb) = (self.ea[i], self.eb[i]);
            self.stage[0][i] = ea * self.a[i] + 0.5 * h * self.stage[4][i];
            self.stage[1][i] = eb * self.b[i] + 0.5 * h * self.stage[5][i];
        }
        self.explicit(0, 1, 6, 7);
        for i in 0..n {
            let (ea, eb) = (self.ea[i], self.eb[i]);
            self.stage[0][i] = ea * ea * self.a[i] + h * ea * self.stage[6][i];
            self.stage[1][i] = eb * eb * self.b[i] + h * eb * self.stage[7][i];
        }
        self.explicit(0, 1, 8, 9);
        let s = &self.stage;
        for i in 0..n {
            let (ea, eb) = (self.ea[i], self.eb[i]);
            self.a[i] = ea * ea * self.a[i]
                + h / 6.0 * (ea * ea * s[2][i] + 2.0 * ea * (s[4][i] + s[6][i]) + s[8][i]);
            self.b[i] = eb * eb * self.b[i]
                + h / 6.0 * (eb * eb * s[3][i] + 2.0 * eb * (s[5][i] + s[7][i]) + s[9][i]);
        }
    }

    /// Advance by model time `t` in steps no longer than [`Self::max_step`].
    pub(crate) fn advance(&mut self, t: f64) {
        let mut left = t;
        while left > 0.0 {
            // Equal steps, so the decay factors stay cached.
            let n = (left / self.max_step()).ceil().max(1.0);
            let h = left / n;
            self.step(h);
            left -= h;
            if left < 1e-15 {
                break;
            }
        }
    }

    /// The step bound: `DT_MAX`, and an advective CFL bound `CFL / (|u|_max · k_max)`
    /// from the triangle-inequality bound on the speed.
    fn max_step(&self) -> f64 {
        let kmax = (self.kn as f64 * PI / self.l).max(M_MODES as f64 * PI);
        let mut speed = 0.0;
        for k in 1..=self.kn {
            let al = k as f64 * PI / self.l;
            for m in 1..=M_MODES {
                speed += self.a[idx(k, m)].abs() * al.max(m as f64 * PI);
            }
        }
        let cfl = if speed > 0.0 {
            CFL / (speed * kmax)
        } else {
            DT_MAX
        };
        // Never a step so short that a frame stalls (the bound only bites in a blow-up).
        cfl.clamp(DT_MAX / 64.0, DT_MAX)
    }

    /// `⟨w θ⟩` over the box: the convective heat flux (`Nu = 1 + ⟨wθ⟩`).
    #[cfg(test)]
    pub(crate) fn heat_flux(&self) -> f64 {
        let mut s = 0.0;
        for i in M_MODES..self.a.len() {
            s += self.alpha[i] * self.a[i] * self.b[i];
        }
        0.25 * s
    }

    /// Kinetic energy `½⟨u² + w²⟩`.
    pub(crate) fn kinetic(&self) -> f64 {
        let mut s = 0.0;
        for i in M_MODES..self.a.len() {
            s += self.q2[i] * self.a[i] * self.a[i];
        }
        0.125 * s
    }

    /// `sin(nθ)`, `cos(nθ)` for `n = 0..=count` by the angle-addition recurrence.
    fn harmonics(theta: f64, count: usize) -> ([f64; K_MAX + 1], [f64; K_MAX + 1]) {
        let (mut s, mut c) = ([0.0; K_MAX + 1], [0.0; K_MAX + 1]);
        let (s1, c1) = theta.sin_cos_det();
        c[0] = 1.0;
        for k in 1..=count {
            s[k] = s[k - 1] * c1 + c[k - 1] * s1;
            c[k] = c[k - 1] * c1 - s[k - 1] * s1;
        }
        (s, c)
    }

    /// Velocity `(u, w)` at `(x, z)` from the modes.
    pub(crate) fn velocity(&self, x: f64, z: f64) -> (f64, f64) {
        let (sx, cx) = Self::harmonics(PI * x / self.l, self.kn);
        let (sz, cz) = Self::harmonics(PI * z, M_MODES);
        let (mut u, mut w) = (0.0, 0.0);
        for k in 1..=self.kn {
            let (mut uk, mut wk) = (0.0, 0.0);
            for m in 1..=M_MODES {
                let a = self.a[idx(k, m)];
                uk += a * m as f64 * cz[m];
                wk += a * sz[m];
            }
            u -= PI * uk * sx[k];
            w += self.alpha[idx(k, 1)] * wk * cx[k];
        }
        (u, w)
    }

    /// `(∂u/∂x, ∂w/∂z)` at `(x, z)`, analytic.
    #[cfg(test)]
    pub(crate) fn divergence_parts(&self, x: f64, z: f64) -> (f64, f64) {
        let (_, cx) = Self::harmonics(PI * x / self.l, self.kn);
        let (_, cz) = Self::harmonics(PI * z, M_MODES);
        let (mut ux, mut wz) = (0.0, 0.0);
        for k in 1..=self.kn {
            let al = self.alpha[idx(k, 1)];
            for m in 1..=M_MODES {
                let (a, g) = (self.a[idx(k, m)], m as f64 * PI);
                ux -= a * g * al * cx[k] * cz[m];
                wz += a * al * g * cx[k] * cz[m];
            }
        }
        (ux, wz)
    }

    /// The horizontal temperature anomaly `θ'` (the `k ≥ 1` modes) at `(x, z)`.
    #[cfg(test)]
    pub(crate) fn anomaly(&self, x: f64, z: f64) -> f64 {
        let (_, cx) = Self::harmonics(PI * x / self.l, self.kn);
        let (sz, _) = Self::harmonics(PI * z, M_MODES);
        let mut t = 0.0;
        for k in 1..=self.kn {
            let mut tk = 0.0;
            for m in 1..=M_MODES {
                tk += self.b[idx(k, m)] * sz[m];
            }
            t += tk * cx[k];
        }
        t
    }
}

/// A passive tracer and its afterglow (box units, newest last; the speed tier per point).
#[derive(Debug, Clone)]
struct Tracer {
    x: f64,
    z: f64,
    trail: Trail<(f32, f32, u8)>,
}

#[derive(Debug)]
pub struct RayleighBenard {
    model: Model,
    /// Whether the first resize has seeded the flow.
    seeded: bool,
    size: Size,
    /// The box on the dot lattice: origin `(x0, y0)` and `bw × bh` dots; the lid is its
    /// top dot row, the floor its bottom one.
    x0: usize,
    y0: usize,
    bw: usize,
    bh: usize,
    /// Per box dot: the `|θ'|` above which it is lit (∞: never).
    thr: Vec<f32>,
    /// `cos(α_k x)` per box column, `[(k−1)·bw + x]`, and `sin(mπz)` per box row,
    /// `[y·M + m − 1]`.
    col_cos: Vec<f32>,
    row_sin: Vec<f32>,
    /// Per frame: `Σ_k b_km cos(α_k x)` as `[(m−1)·bw + x]`, and one row of `θ'`.
    colsum: Vec<f32>,
    row: Vec<f32>,
    /// `b` as f32 for the frame sums.
    bf: Vec<f32>,
    tracers: Vec<Tracer>,
    /// RMS speed (box units per model time), for the speed tiers.
    v_rms: f64,
    dots: Dots,
    dirty: bool,
}

impl Default for RayleighBenard {
    fn default() -> Self {
        Self {
            model: Model::new(2.0, RA, PR),
            seeded: false,
            size: Size::new(1, 1),
            x0: 0,
            y0: 0,
            bw: 0,
            bh: 0,
            thr: Vec::new(),
            col_cos: Vec::new(),
            row_sin: Vec::new(),
            colsum: Vec::new(),
            row: Vec::new(),
            bf: Vec::new(),
            tracers: Vec::new(),
            v_rms: 0.0,
            dots: Dots::default(),
            dirty: true,
        }
    }
}

/// The box in cells for a `w × h` terminal: `(x0, y0, width, height)`. It leaves the
/// title row, and when there is room `INSET_COLS` columns at each side and one row above
/// and below, so the hot floor never underlines the title and the walls sit off the
/// frame.
pub(crate) fn box_cells(w: u16, h: u16) -> (u16, u16, u16, u16) {
    let rows = if h >= 3 { h - 1 } else { h };
    let (y0, bh) = if rows >= 6 { (1, rows - 2) } else { (0, rows) };
    let (x0, bw) = if w >= 4 * INSET_COLS + 4 {
        (INSET_COLS, w - 2 * INSET_COLS)
    } else {
        (0, w)
    };
    (x0, y0, bw, bh)
}

/// The ink of a field dot with anomaly `v` and threshold `t`, or `None` when it is not
/// lit: lit with probability `P_MAX·clamp((|v| − θ₀)/(θ_ref − θ₀), 0, 1)` through its
/// fixed threshold, `Hot` where `v > 0`, `Cold` where `v < 0`, brighter where
/// `|v| ≥ BRIGHT·θ_ref`.
#[inline]
fn dot_ink(v: f32, t: f32) -> Option<Ink> {
    let a = v.abs();
    if a <= t {
        return None;
    }
    let inks = if v > 0.0 { &HOT } else { &COLD };
    Some(inks[usize::from(a >= (BRIGHT * THETA_REF) as f32)])
}

/// The speed tier of speed `v` against the RMS speed.
fn tier(v: f64, v_rms: f64) -> u8 {
    if v < TIER[0] * v_rms {
        0
    } else if v < TIER[1] * v_rms {
        1
    } else {
        2
    }
}

impl RayleighBenard {
    /// Box coordinates of box dot `(x, y)`.
    fn to_box(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x + 0.5) / self.bw as f64 * self.model.l,
            1.0 - (y + 0.5) / self.bh as f64,
        )
    }

    /// Lattice dot coordinates of box point `(x, z)`.
    fn to_dot(&self, x: f64, z: f64) -> (f64, f64) {
        (
            self.x0 as f64 + x / self.model.l * self.bw as f64 - 0.5,
            self.y0 as f64 + (1.0 - z) * self.bh as f64 - 0.5,
        )
    }

    /// Advect every tracer through the current flow by model time `h` (midpoint rule),
    /// keeping it inside the box (RB-A4).
    fn advect(&mut self, h: f64) {
        let (model, v_rms) = (&self.model, self.v_rms);
        let l = model.l;
        for t in &mut self.tracers {
            let (u0, w0) = model.velocity(t.x, t.z);
            let (xm, zm) = (
                (t.x + 0.5 * h * u0).clamp(WALL_EPS, l - WALL_EPS),
                (t.z + 0.5 * h * w0).clamp(WALL_EPS, 1.0 - WALL_EPS),
            );
            let (u, w) = model.velocity(xm, zm);
            t.x = (t.x + h * u).clamp(WALL_EPS, l - WALL_EPS);
            t.z = (t.z + h * w).clamp(WALL_EPS, 1.0 - WALL_EPS);
            let v = u.hypot(w);
            t.trail.push((t.x as f32, t.z as f32, tier(v, v_rms)));
        }
    }

    /// Per-frame column sums `Σ_k b_km cos(α_k x)` of the current modes.
    fn column_sums(&mut self) {
        let (bw, kn) = (self.bw, self.model.kn);
        for (f, &b) in self.bf.iter_mut().zip(&self.model.b) {
            *f = b as f32;
        }
        self.colsum.fill(0.0);
        for m in 0..M_MODES {
            let cs = &mut self.colsum[m * bw..(m + 1) * bw];
            for k in 1..=kn {
                let b = self.bf[k * M_MODES + m];
                let ck = &self.col_cos[(k - 1) * bw..k * bw];
                for (c, &x) in cs.iter_mut().zip(ck) {
                    *c += b * x;
                }
            }
        }
    }

    /// `θ'` along box dot row `y` into `self.row` (after [`Self::column_sums`]).
    fn field_row(&mut self, y: usize) {
        let bw = self.bw;
        self.row.fill(0.0);
        let s = &self.row_sin[y * M_MODES..(y + 1) * M_MODES];
        for (m, &sm) in s.iter().enumerate() {
            for (r, &c) in self.row.iter_mut().zip(&self.colsum[m * bw..(m + 1) * bw]) {
                *r += sm * c;
            }
        }
    }

    /// Light the stipple of `θ'` into `dots` (RB-A5).
    fn raster_field(&mut self) {
        self.column_sums();
        let (bw, x0, y0) = (self.bw, self.x0 as i32, self.y0 as i32);
        for y in 0..self.bh {
            self.field_row(y);
            let thr = &self.thr[y * bw..(y + 1) * bw];
            for (x, (&v, &t)) in self.row.iter().zip(thr).enumerate() {
                if let Some(ink) = dot_ink(v, t) {
                    self.dots.set(x0 + x as i32, y0 + y as i32, ink);
                }
            }
        }
    }

    /// The floor and lid threads and the tracer afterglows.
    fn raster_threads(&mut self) {
        let (x0, y0) = (self.x0 as f64, self.y0 as f64);
        let (x1, y1) = (x0 + self.bw as f64 - 1.0, y0 + self.bh as f64 - 1.0);
        self.dots.line(x0, y0, x1, y0, LID_INK);
        self.dots.line(x0, y1, x1, y1, FLOOR_INK);
        let tracers = std::mem::take(&mut self.tracers);
        for t in &tracers {
            let tail = t.trail.len() / TAIL_DIV;
            let mut prev: Option<(f64, f64)> = None;
            for (i, &(x, z, tr)) in t.trail.iter().enumerate() {
                let p = self.to_dot(f64::from(x), f64::from(z));
                let ink = if i < tail {
                    TRACER_TAIL[usize::from(tr)]
                } else {
                    TRACER[usize::from(tr)]
                };
                match prev {
                    Some(q) => self.dots.line(q.0, q.1, p.0, p.1, ink),
                    None => self.dots.plot(p.0, p.1, ink),
                }
                prev = Some(p);
            }
        }
        self.tracers = tracers;
    }

    fn raster(&mut self) {
        self.dots.clear();
        self.raster_field();
        self.raster_threads();
    }
}

impl Generator for RayleighBenard {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let (cx, cy, cw, ch) = box_cells(size.width(), size.height());
        let (cw_dots, ch_dots) = (Dots::CELL_W as usize, Dots::CELL_H as usize);
        (self.x0, self.y0) = (usize::from(cx) * cw_dots, usize::from(cy) * ch_dots);
        (self.bw, self.bh) = (usize::from(cw) * cw_dots, usize::from(ch) * ch_dots);
        let l = (self.bw as f64 / self.bh.max(1) as f64).clamp(L_MIN, L_MAX);
        let old_l = self.model.l;
        let mut model = Model::new(l, RA, PR);
        if self.seeded {
            model.take_modes(&self.model);
        } else {
            model.seed_plume(&mut ctx.rng, SEED_AMP, SEED_NOISE);
        }
        self.model = model;
        let (bw, bh, kn) = (self.bw, self.bh, self.model.kn);
        // Per-dot thresholds: lit ⇔ P_MAX·(|θ'| − θ₀)/(θ_ref − θ₀) > u. The lid and floor
        // rows are threads, never stipple. Keys are lattice dots, so a move of the box
        // keeps each dot's threshold.
        self.thr.clear();
        self.thr.resize(bw * bh, f32::INFINITY);
        for y in 1..bh.saturating_sub(1) {
            for x in 0..bw {
                let key = cell_key((self.x0 + x) as i32, (self.y0 + y) as i32);
                let u = hash01(FIELD_SALT ^ key);
                if u < P_MAX {
                    self.thr[y * bw + x] =
                        (THETA_FLOOR + u / P_MAX * (THETA_REF - THETA_FLOOR)) as f32;
                }
            }
        }
        let mut col_cos = vec![0.0f32; kn * bw];
        for x in 0..bw {
            let (bx, _) = self.to_box(x as f64, 0.0);
            let (_, c) = Model::harmonics(PI * bx / l, kn);
            for k in 1..=kn {
                col_cos[(k - 1) * bw + x] = c[k] as f32;
            }
        }
        let mut row_sin = vec![0.0f32; bh * M_MODES];
        for y in 0..bh {
            let (_, z) = self.to_box(0.0, y as f64);
            let (s, _) = Model::harmonics(PI * z, M_MODES);
            for m in 1..=M_MODES {
                row_sin[y * M_MODES + m - 1] = s[m] as f32;
            }
        }
        self.col_cos = col_cos;
        self.row_sin = row_sin;
        self.colsum = vec![0.0; M_MODES * bw];
        self.row = vec![0.0; bw];
        self.bf = vec![0.0; (kn + 1) * M_MODES];
        // Tracers, 6–40 scaled with the area: kept (rescaled to the new length) or
        // seeded uniformly.
        let cells = size.area() as f64;
        let n = ((cells * TRACER_DENSITY).round() as usize).clamp(TRACERS_MIN, TRACERS_MAX);
        let scale = if self.seeded { l / old_l } else { 1.0 };
        for t in &mut self.tracers {
            t.x = (t.x * scale).clamp(WALL_EPS, l - WALL_EPS);
            t.trail.clear();
        }
        self.tracers.truncate(n);
        while self.tracers.len() < n {
            let x = ctx.rng.random::<f64>() * l;
            let z = ctx.rng.random::<f64>();
            self.tracers.push(Tracer {
                x: x.clamp(WALL_EPS, l - WALL_EPS),
                z: z.clamp(WALL_EPS, 1.0 - WALL_EPS),
                trail: Trail::new(TRAIL),
            });
        }
        self.seeded = true;
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        let h = RATE * dt.max(0.0);
        if h > 0.0 {
            self.model.advance(h);
        }
        self.v_rms = (2.0 * self.model.kinetic()).sqrt();
        self.advect(h);
        self.dirty = true;
    }

    fn prepare(&mut self) {
        if std::mem::take(&mut self.dirty) && self.seeded {
            self.raster();
        }
    }

    fn render(&self, canvas: &mut Canvas) {
        if canvas.size() != self.size {
            return;
        }
        self.dots.blit(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(seed: u64) -> Ctx {
        Ctx::new(seed, Size::new(80, 24))
    }

    /// Growth rate of the most unstable linear mode `(k, 1)`: the larger root of
    /// `σ² + (Pr + 1) q² σ + Pr q⁴ − Pr Ra α²/q² = 0`.
    fn growth_rate(alpha: f64, ra: f64, pr: f64) -> f64 {
        let q2 = alpha * alpha + PI * PI;
        let (b, c) = ((pr + 1.0) * q2, pr * q2 * q2 - pr * ra * alpha * alpha / q2);
        0.5 * (-b + (b * b - 4.0 * c).sqrt())
    }

    #[test]
    fn rb_a1_small_perturbation_decays_below_ra_c_and_grows_above() {
        // A box of one critical wavelength: mode k = 2 has α = π/√2, the critical one.
        let l = 2.0 * 2f64.sqrt();
        for (ratio, grows) in [(0.9, false), (1.2, true), (3.0, true)] {
            let ra = ratio * RA_C;
            let mut m = Model::new(l, ra, PR);
            m.b[idx(2, 1)] = 1e-6;
            // Let the fast transient die, then measure the rate of the (2, 1) mode.
            m.advance(0.05);
            let b0 = m.b[idx(2, 1)];
            m.advance(0.1);
            let b1 = m.b[idx(2, 1)];
            assert_eq!(b1.abs() > b0.abs(), grows, "Ra = {ratio} Ra_c");
            let sigma = (b1 / b0).ln() / 0.1;
            let want = growth_rate(PI / 2f64.sqrt(), ra, PR);
            assert!(
                (sigma - want).abs() < 0.01 * want.abs().max(1.0),
                "Ra = {ratio} Ra_c: σ {sigma} vs linear theory {want}"
            );
        }
    }

    #[test]
    fn rb_a1_critical_rayleigh_number_is_27_pi4_over_4() {
        // Marginal stability of the critical mode at Ra_c.
        assert!(growth_rate(PI / 2f64.sqrt(), RA_C, PR).abs() < 1e-9);
    }

    fn spun_up(l: f64, model_time: f64) -> Model {
        let mut m = Model::new(l, RA, PR);
        m.perturb(&mut ctx(7).rng, SEED_AMP);
        m.advance(model_time);
        m
    }

    #[test]
    fn rb_a2_velocity_is_divergence_free() {
        let m = spun_up(2.0, 0.05);
        let h = 1e-5;
        for i in 0..7 {
            for j in 0..5 {
                let (x, z) = (0.13 + 0.27 * f64::from(i), 0.07 + 0.2 * f64::from(j));
                let (ux, wz) = m.divergence_parts(x, z);
                assert!(
                    (ux + wz).abs() <= 1e-9 * ux.abs().max(1.0),
                    "analytic ∇·u {ux} + {wz}"
                );
                // The velocity actually used to move tracers, by central differences.
                let dudx = (m.velocity(x + h, z).0 - m.velocity(x - h, z).0) / (2.0 * h);
                let dwdz = (m.velocity(x, z + h).1 - m.velocity(x, z - h).1) / (2.0 * h);
                assert!((dudx - ux).abs() <= 1e-5 * ux.abs().max(1.0));
                assert!(
                    (dudx + dwdz).abs() <= 1e-5 * dudx.abs().max(1.0),
                    "finite-difference ∇·u {dudx} + {dwdz}"
                );
            }
        }
    }

    #[test]
    fn rb_galerkin_advection_conserves_energy_and_heat_variance() {
        // With no buoyancy, the projected advection must neither create nor destroy
        // kinetic energy Σ q²a² or temperature variance Σ b² (exact quadrature).
        let mut m = spun_up(1.8, 0.05);
        m.ra = 0.0;
        m.stage[0].copy_from_slice(&m.a.clone());
        m.stage[1].copy_from_slice(&m.b.clone());
        m.explicit(0, 1, 2, 3);
        let (mut de, mut dv, mut scale) = (0.0, 0.0, 0.0);
        for i in 0..m.a.len() {
            de += m.q2[i] * m.a[i] * m.stage[2][i];
            // Remove the linear w source from the θ tendency.
            let adv = m.stage[3][i] - m.alpha[i] * m.a[i];
            let w = if i < M_MODES { 2.0 } else { 1.0 };
            dv += w * m.b[i] * adv;
            scale += (m.q2[i] * m.a[i] * m.stage[2][i]).abs() + (m.b[i] * adv).abs();
        }
        assert!(de.abs() < 1e-9 * scale, "dE/dt {de} (scale {scale})");
        assert!(dv.abs() < 1e-9 * scale, "dθ²/dt {dv} (scale {scale})");
    }

    #[test]
    fn rb_a3_hot_fluid_rises_once_rolls_form() {
        for l in [1.5, 2.7] {
            let mut m = spun_up(l, 0.05);
            let mut mean = 0.0;
            for _ in 0..10 {
                m.advance(0.002);
                let flux = m.heat_flux();
                assert!(flux > 0.0, "L {l}: ⟨wθ⟩ = {flux}");
                mean += flux / 10.0;
            }
            // Nusselt number well above conduction.
            assert!(1.0 + mean > 3.0, "L {l}: Nu {}", 1.0 + mean);
        }
        // The modal flux is the box mean of w·θ' sampled on a grid (one aspect).
        let l = 2.0;
        let m = spun_up(l, 0.05);
        let (nx, nz) = (48u32, 32u32);
        let mut grid = 0.0;
        for i in 0..nx {
            for j in 0..nz {
                let x = (f64::from(i) + 0.5) * l / f64::from(nx);
                let z = (f64::from(j) + 0.5) / f64::from(nz);
                grid += m.velocity(x, z).1 * m.anomaly(x, z);
            }
        }
        grid /= f64::from(nx * nz);
        let flux = m.heat_flux();
        assert!((grid - flux).abs() < 1e-6 * flux, "{grid} vs {flux}");
    }

    fn scene(w: u16, h: u16, seed: u64) -> (RayleighBenard, Ctx) {
        let size = Size::new(w, h);
        let mut ctx = Ctx::new(seed, size);
        let mut g = RayleighBenard::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn rb_a4_tracers_never_leave_the_box() {
        for (w, h, dt) in [(80u16, 24u16, 0.25), (13, 5, SIM_DT)] {
            let (mut g, mut ctx) = scene(w, h, 3);
            for i in 0..300 {
                g.update(dt, &mut ctx);
                if i == 150 {
                    g.resize(Size::new(w + 17, h + 3), &mut ctx);
                }
                if i % 10 != 0 {
                    continue;
                }
                let l = g.model.l;
                for t in &g.tracers {
                    assert!((0.0..=l).contains(&t.x) && (0.0..=1.0).contains(&t.z));
                    for &(x, z, _) in t.trail.iter() {
                        assert!((0.0..=l as f32).contains(&x) && (0.0..=1.0).contains(&z));
                    }
                }
            }
        }
    }

    #[test]
    fn rb_box_is_inset_off_the_frame_and_the_title_row() {
        assert_eq!(box_cells(200, 50), (2, 1, 196, 47));
        assert_eq!(box_cells(80, 24), (2, 1, 76, 21));
        // Too low for the row insets, too narrow for the column insets.
        assert_eq!(box_cells(13, 5), (2, 0, 9, 4));
        assert_eq!(box_cells(11, 5), (0, 0, 11, 4));
        assert_eq!(box_cells(1, 1), (0, 0, 1, 1));
        // The floor thread sits a blank row above the title row.
        let (mut g, mut ctx) = scene(80, 24, 1);
        g.update(SIM_DT, &mut ctx);
        g.prepare();
        let mut c = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
        g.render(&mut c);
        let row = |y: i32| {
            (0..80)
                .filter(|&x| c.get(x, y).is_some_and(|c| c.ch != ' '))
                .count()
        };
        assert_eq!(row(22), 0, "the row above the title is blank");
        assert_eq!(row(0), 0, "the top row is blank");
        assert!(row(21) >= 70, "the floor thread");
        assert!(row(1) >= 70, "the lid thread");
    }

    #[test]
    fn rb_opens_with_a_central_warm_plume_between_two_rolls_within_a_second() {
        for (w, h) in [(200u16, 50u16), (80, 24)] {
            let (mut g, mut ctx) = scene(w, h, 42);
            for _ in 0..60 {
                g.update(SIM_DT, &mut ctx);
            }
            let l = g.model.l;
            let mid = g.model.anomaly(0.5 * l, 0.5);
            assert!(mid > THETA_FLOOR, "{w}x{h}: centre θ' {mid}");
            assert!(g.model.anomaly(0.05 * l, 0.5) < 0.0, "{w}x{h}: walls cool");
            // Rising in the middle: two counter-rotating rolls.
            assert!(g.model.velocity(0.5 * l, 0.5).1 > 0.0);
            assert!(g.model.velocity(0.25 * l, 0.5).1.abs() < g.model.velocity(0.5 * l, 0.5).1);
            g.prepare();
            let lit = (0..i32::from(g.dots.dot_size().height()))
                .flat_map(|y| (0..i32::from(g.dots.dot_size().width())).map(move |x| (x, y)))
                .filter(|&(x, y)| g.dots.is_lit(x, y))
                .count();
            assert!(lit > 200, "{w}x{h}: {lit} lit dots after 1 s");
        }
    }

    #[test]
    fn rb_opening_pattern_does_not_freeze() {
        // From the seeded opening the flow leaves the symmetric two-roll state and keeps
        // changing (the noise breaks the symmetry; Ra = 100 Ra_c, Pr = 10 is unsteady).
        let size = Size::new(80, 24);
        let (mut g, mut ctx) = scene(size.width(), size.height(), 42);
        let l = g.model.l;
        g.model.advance(0.15);
        let sample = |m: &Model| -> Vec<f64> {
            (0..12)
                .flat_map(|i| (0..6).map(move |j| (i, j)))
                .map(|(i, j)| {
                    m.anomaly((f64::from(i) + 0.5) * l / 12.0, (f64::from(j) + 0.5) / 6.0)
                })
                .collect()
        };
        let before = sample(&g.model);
        g.model.advance(0.05);
        let after = sample(&g.model);
        let d: f64 = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (a - b).powi(2))
            .sum();
        let s: f64 = after.iter().map(|a| a * a).sum();
        assert!((d / s).sqrt() > 0.2, "relative change {}", (d / s).sqrt());
        let _ = &mut ctx;
    }

    #[test]
    fn rb_a5_stipple_sign_sets_the_ink_and_density_is_capped() {
        let (mut g, mut ctx) = scene(120, 40, 42);
        for _ in 0..(8.0 / SIM_DT) as usize {
            g.update(SIM_DT, &mut ctx);
        }
        g.dots.clear();
        g.raster_field();
        let (mut lit, mut strong, mut any) = (0usize, 0usize, 0usize);
        for y in 0..g.bh {
            g.field_row(y);
            for x in 0..g.bw {
                let (bx, bz) = g.to_box(x as f64, y as f64);
                let v = g.model.anomaly(bx, bz);
                // The f32 frame tables are the modal sum.
                let table = g.row[x];
                assert!(
                    (f64::from(table) - v).abs() < 1e-4,
                    "({x},{y}) {table} vs {v}"
                );
                let ink = dot_ink(table, g.thr[y * g.bw + x]);
                if let Some(ink) = ink {
                    any += 1;
                    assert!(g.dots.is_lit((g.x0 + x) as i32, (g.y0 + y) as i32));
                    if v.abs() > 1e-3 {
                        let want = if v > 0.0 { Role::Hot } else { Role::Cold };
                        assert_eq!(ink.role, want, "({x},{y}) θ' {v}");
                    }
                }
                if v.abs() < THETA_FLOOR - 1e-3 {
                    assert!(ink.is_none(), "({x},{y}) lit below the floor, θ' {v}");
                }
                if v.abs() >= THETA_REF {
                    strong += 1;
                    lit += usize::from(ink.is_some());
                }
                if y == 0 || y + 1 == g.bh {
                    assert!(ink.is_none(), "stipple on the floor or lid row");
                }
            }
        }
        assert!(any > 300, "{any} field dots");
        // Where the density saturates, the lit share is P_MAX (sampling noise aside).
        assert!(strong > 150, "{strong} saturated dots");
        let share = lit as f64 / strong as f64;
        assert!(share <= P_MAX + 0.04, "share {share}");
        assert!(share >= P_MAX - 0.06, "share {share}");
    }
}
