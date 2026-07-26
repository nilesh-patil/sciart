//! Protocell assembly: soap-like molecules in water hide their oily tails from it.
//! Scattered amphiphiles gather into micelles, then a flat bilayer sheet, and the sheet
//! curls shut into a vesicle that traps a little water and solutes, a first cell-like
//! compartment; it dissolves and the cycle repeats (SPEC-002 `protocell_assembly`).
//!
//! Coarse-grained overdamped Brownian dynamics with implicit solvent. Each amphiphile is a
//! rigid head–tail dimer (centre `c`, angle `θ` from head to tail, length `ℓ = 1`); sites
//! interact by soft excluded volume, a short-range tail–tail attraction and a weak
//! head–head repulsion, found through a `SpatialHash`. Per step
//! `Δc = μ F Δt + √(2DΔt) ξ` and `Δθ = μ_r τ Δt + √(2D_rΔt) ξ` (`ctx.rng` kicks).
//!
//! A looping `Timeline` (~40 s): `dispersed` (attraction off, the last vesicle dissolves),
//! `micelles` (attraction on: small clusters, tails in, heads out), `bilayer` and
//! `vesicle`. The last two carry the scripted curvature bias that SPEC-002's stylisation
//! allows: a set of molecules is given slots on a two-leaflet sheet (tails meeting at the
//! mid-plane, heads out), recruited from the middle outwards by a capped spring, and the
//! sheet's curvature then ramps from 0 to `2π/L`, so its ends meet and it closes into a
//! vesicle of radius `L/2π` (leaflet slot counts chosen so both rings end evenly spaced).
//! Solutes diffuse freely, are kept out of the membrane by excluded volume, and some end
//! up inside.
//!
//! ## Look (GEN-R26, molecules)
//! Each amphiphile a small `Hot` head atom with a short 1-px `Cold` tail thread; solutes
//! single `Accent` dots. Positions are drawn through a short exponential average (0.35 s),
//! a camera exposure that keeps the thermal jitter from flickering every cell.

use rand::Rng;
use sciart_core::particles::gaussian2;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;

pub static META: Meta = Meta {
    name: "protocell_assembly",
    category: Category::LivingMatter,
    description: "Amphiphiles in water self-assemble into micelles, bilayers and a vesicle",
    title: "Protocell · lipids self-assemble",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 4] = ["dispersed", "micelles", "bilayer", "vesicle"];
const DISPERSED: usize = 0;
const MICELLES: usize = 1;
const BILAYER: usize = 2;
const VESICLE: usize = 3;
const DURATIONS: [f64; 4] = [8.0, 10.0, 10.0, 12.0];

pub fn build() -> Box<dyn Generator> {
    Box::new(Protocell::default())
}

// ---- Model (SPEC-002 Model), in box units: a molecule is 1 long ----

/// Molecules at most, and per box unit² (scaled with area).
pub(crate) const MAX_N: usize = 300;
const DENSITY: f64 = 0.07;
const MIN_N: usize = 48;
/// Solutes.
const SOLUTES: usize = 32;
/// Head-to-tail length.
pub(crate) const LEN: f64 = 1.0;
/// Mobilities and diffusion constants (units, s).
const MU: f64 = 1.0;
pub(crate) const D_T: f64 = 1.2;
const D_DISPERSE: f64 = 2.4;
/// Faster mixing while micelles gather, so they form within the act.
const D_MICELLE: f64 = 2.5;
const MU_R: f64 = 3.0;
const D_R: f64 = 1.0;
const D_SOLUTE: f64 = 0.5;
/// Solutes scattered in a sheet's future lumen when its slots are dealt.
const LUMEN_SOLUTES: usize = 5;
/// Soft excluded volume (any two sites), head–head repulsion range and strength.
const EV_R: f64 = 0.8;
const EV_K: f64 = 25.0;
const HH_R: f64 = 1.6;
const HH_K: f64 = 6.0;
/// Tail–tail attraction: full strength `ATTRACT` from `EV_R`, fading to 0 at `TT_R`.
const ATTRACT: f64 = 12.0;
/// The sheet's own tails need less: the slots hold it.
const ATTRACT_SHEET: f64 = 4.0;
const TT_R: f64 = 2.0;
const CUT: f64 = 2.6;
/// Bilayer slots: head spacing in the closed vesicle, the half-gap between the leaflets'
/// tail ends, the guide spring (capped) and the share of molecules recruited.
pub(crate) const SPACING: f64 = 0.85;
const GAP: f64 = 0.5 * EV_R;
const GUIDE_K: f64 = 30.0;
const GUIDE_TORQUE: f64 = 12.0;
const GUIDE_CAP: f64 = 8.0;
const RECRUIT: f64 = 0.55;
const GHOST_R: f64 = 1.5;
const MIN_RECRUITS: usize = 56;
const FREE: u8 = 0;
const GHOST: u8 = 1;
const PLACED: u8 = 2;
/// A recruit in its slot jiggles less: a membrane is more ordered than the solution.
const D_SLOT: f64 = 0.3;
#[cfg(test)]
/// Two heads closer than this are in contact (PROTO-A3): no site (excluded-volume
/// diameter `EV_R`) can pass between them.
pub(crate) const CONTACT: f64 = 2.0 * EV_R;
/// The longest step the dynamics takes.
const MAX_DT: f64 = 1.0 / 60.0;

/// A molecule's place in the sheet: its leaflet (+1 upper, which becomes the vesicle's
/// inner leaflet; −1 lower/outer; 0 free), its arc position and when its guide starts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Slot {
    pub(crate) leaflet: i8,
    /// Place along its leaflet, −½ … ½: both leaflets span the whole mid-plane, flat or
    /// closed.
    pub(crate) frac: f64,
    pub(crate) t_on: f64,
}

impl Slot {
    /// Arc position on the mid-plane of `sheet`.
    pub(crate) fn arc(&self, sheet: &Sheet) -> f64 {
        self.frac * sheet.len
    }
}

/// The sheet: mid-point, length and current curvature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sheet {
    pub(crate) mid: [f64; 2],
    pub(crate) len: f64,
    pub(crate) curv: f64,
}

impl Sheet {
    /// Point and inner normal (towards the centre of curvature, "up") at arc position `s`.
    pub(crate) fn at(&self, s: f64) -> ([f64; 2], [f64; 2]) {
        let k = self.curv;
        let (sn, cs) = (k * s).sin_cos_det();
        let (dx, dy) = if k.abs() < 1e-9 {
            (s, 0.0)
        } else {
            (sn / k, (1.0 - cs) / k)
        };
        ([self.mid[0] + dx, self.mid[1] - dy], [-sn, -cs])
    }

    /// The vesicle's radius once closed.
    pub(crate) fn radius(&self) -> f64 {
        self.len / std::f64::consts::TAU
    }
}

/// The forces the act switches on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drive {
    pub(crate) attract: f64,
    pub(crate) diffusion: f64,
    /// The sheet and the seconds since recruiting began, when guided.
    pub(crate) guide: Option<(Sheet, f64)>,
}

impl Drive {
    pub(crate) const OFF: Drive = Drive {
        attract: 0.0,
        diffusion: D_T,
        guide: None,
    };
}

#[derive(Debug, Clone)]
pub(crate) struct Soup {
    pub(crate) w: f64,
    pub(crate) h: f64,
    pub(crate) n: usize,
    pub(crate) c: Vec<[f64; 2]>,
    pub(crate) th: Vec<f64>,
    pub(crate) slot: Vec<Slot>,
    /// Heads (0..n) then tails (n..2n), and the force on each.
    pub(crate) sites: Vec<[f64; 2]>,
    force: Vec<[f64; 2]>,
    pub(crate) solutes: Vec<[f64; 2]>,
    hash: SpatialHash,
    order: Vec<u16>,
    /// Per molecule: free, a recruit still on its way (passes through others) or a
    /// recruit in its slot (see `step`).
    ghost: Vec<u8>,
    /// Forces on, so tests can switch them off (PROTO-A1).
    pub(crate) interact: bool,
}

impl Default for Soup {
    fn default() -> Self {
        Self {
            w: 1.0,
            h: 1.0,
            n: 0,
            c: Vec::with_capacity(MAX_N),
            th: Vec::with_capacity(MAX_N),
            slot: Vec::with_capacity(MAX_N),
            sites: Vec::with_capacity(2 * MAX_N),
            force: Vec::with_capacity(2 * MAX_N),
            solutes: Vec::with_capacity(SOLUTES),
            hash: SpatialHash::default(),
            order: Vec::with_capacity(MAX_N),
            ghost: Vec::with_capacity(MAX_N),
            interact: true,
        }
    }
}

fn unit(th: f64) -> [f64; 2] {
    let (s, c) = th.sin_cos_det();
    [c, s]
}

impl Soup {
    /// Resize the box to `w × h` with `n` molecules: positions are rescaled, missing
    /// molecules and solutes scattered at random.
    pub(crate) fn resize(&mut self, w: f64, h: f64, n: usize, rng: &mut impl Rng) {
        let (sx, sy) = (w / self.w, h / self.h);
        for p in self.c.iter_mut().chain(self.solutes.iter_mut()) {
            p[0] *= sx;
            p[1] *= sy;
        }
        self.w = w;
        self.h = h;
        let n = n.clamp(MIN_N, MAX_N);
        self.c.truncate(n);
        self.th.truncate(n);
        self.slot.truncate(n);
        while self.c.len() < n {
            self.c
                .push([rng.random::<f64>() * w, rng.random::<f64>() * h]);
            self.th.push(rng.random::<f64>() * std::f64::consts::TAU);
            self.slot.push(Slot::default());
        }
        while self.solutes.len() < SOLUTES {
            self.solutes
                .push([rng.random::<f64>() * w, rng.random::<f64>() * h]);
        }
        self.n = n;
        self.hash.resize(w, h, CUT);
        self.place_sites();
    }

    /// Scatter every molecule and solute at random.
    pub(crate) fn scatter(&mut self, rng: &mut impl Rng) {
        for k in 0..self.n {
            self.c[k] = [rng.random::<f64>() * self.w, rng.random::<f64>() * self.h];
            self.th[k] = rng.random::<f64>() * std::f64::consts::TAU;
            self.slot[k] = Slot::default();
        }
        for p in &mut self.solutes {
            *p = [rng.random::<f64>() * self.w, rng.random::<f64>() * self.h];
        }
        self.place_sites();
    }

    pub(crate) fn place_sites(&mut self) {
        self.sites.clear();
        for k in 0..self.n {
            let u = unit(self.th[k]);
            self.sites.push([
                self.c[k][0] - 0.5 * LEN * u[0],
                self.c[k][1] - 0.5 * LEN * u[1],
            ]);
        }
        for k in 0..self.n {
            let u = unit(self.th[k]);
            self.sites.push([
                self.c[k][0] + 0.5 * LEN * u[0],
                self.c[k][1] + 0.5 * LEN * u[1],
            ]);
        }
    }

    /// The sheet the scene builds for this box and molecule count: long enough for the
    /// recruited molecules, its vesicle centred in the box.
    pub(crate) fn sheet(&self) -> Sheet {
        let nv = self.recruits();
        let r = nv as f64 * SPACING / (2.0 * std::f64::consts::TAU);
        Sheet {
            mid: [0.5 * self.w, 0.5 * self.h + r],
            len: std::f64::consts::TAU * r,
            curv: 0.0,
        }
    }

    fn recruits(&self) -> usize {
        // At least `MIN_RECRUITS` (a vesicle with a lumen), else a share.
        ((RECRUIT * self.n as f64) as usize)
            .max(MIN_RECRUITS)
            .min(self.n)
            / 2
            * 2
    }

    /// Give the molecules nearest the sheet's middle their slots: per leaflet a count
    /// that spaces the closed vesicle's rings evenly, the upper ones (smallest y) the
    /// inner leaflet, each sorted along the sheet; guides switch on from the middle out
    /// over `spread` seconds.
    pub(crate) fn assign(&mut self, sheet: &Sheet, spread: f64, rng: &mut impl Rng) {
        for s in &mut self.slot {
            *s = Slot::default();
        }
        let nv = self.recruits();
        let r = sheet.radius();
        let (r_out, r_in) = (r + GAP + LEN, (r - GAP - LEN).max(0.1));
        let n_in = ((nv as f64 * r_in / (r_in + r_out)).round() as usize).clamp(1, nv - 1);
        let m = sheet.mid;
        let c = &self.c;
        self.order.clear();
        self.order.extend(0..self.n as u16);
        let d2 = |k: u16| {
            let p = c[usize::from(k)];
            (p[0] - m[0]).powi(2) + (p[1] - m[1]).powi(2)
        };
        self.order
            .sort_unstable_by(|&a, &b| d2(a).total_cmp(&d2(b)));
        let chosen = &mut self.order[..nv];
        chosen.sort_unstable_by(|&a, &b| c[usize::from(a)][1].total_cmp(&c[usize::from(b)][1]));
        let (inner, outer) = chosen.split_at_mut(n_in);
        for (group, leaflet) in [(inner, 1i8), (outer, -1i8)] {
            group.sort_unstable_by(|&a, &b| c[usize::from(a)][0].total_cmp(&c[usize::from(b)][0]));
            let count = group.len() as f64;
            for (j, &k) in group.iter().enumerate() {
                let frac = (j as f64 + 0.5) / count - 0.5;
                self.slot[usize::from(k)] = Slot {
                    leaflet,
                    frac,
                    t_on: spread * (2.0 * frac).abs(),
                };
            }
        }
        // A few solutes are already where the lumen will be (nothing drags them there).
        let centre = [m[0], m[1] - r];
        let reach = (r_in - 0.6).max(0.2);
        for p in self.solutes.iter_mut().take(LUMEN_SOLUTES) {
            let (rr, th) = (
                reach * rng.random::<f64>().sqrt(),
                std::f64::consts::TAU * rng.random::<f64>(),
            );
            let (sn, cs) = th.sin_cos_det();
            *p = [centre[0] + rr * cs, centre[1] + rr * sn];
        }
    }

    /// Where a slotted molecule's centre and angle should be on `sheet`.
    pub(crate) fn target(sheet: &Sheet, slot: &Slot) -> ([f64; 2], f64) {
        let (p, n) = sheet.at(slot.arc(sheet));
        let side = f64::from(slot.leaflet);
        let off = side * (GAP + 0.5 * LEN);
        // Head outwards from the mid-plane: head → tail points back towards it.
        (
            [p[0] + off * n[0], p[1] + off * n[1]],
            (-side * n[1]).atan2(-side * n[0]),
        )
    }

    /// One Brownian step of `dt` (≤ `MAX_DT`).
    pub(crate) fn step(&mut self, dt: f64, drive: &Drive, rng: &mut impl Rng) {
        let n = self.n;
        self.force.clear();
        self.force.resize(2 * n, [0.0; 2]);
        // A recruit more than `GHOST_R` from its slot is still on its way and passes
        // through the membrane and the micelles (a stand-in for lipid exchange, which
        // the 2-D model cannot otherwise do), so the sheet fills without jams.
        self.ghost.clear();
        for k in 0..n {
            let ghost = match drive.guide {
                Some((sheet, _)) if self.slot[k].leaflet != 0 => {
                    let (c, _) = Self::target(&sheet, &self.slot[k]);
                    if (c[0] - self.c[k][0]).hypot(c[1] - self.c[k][1]) > GHOST_R {
                        GHOST
                    } else {
                        PLACED
                    }
                }
                _ => FREE,
            };
            self.ghost.push(ghost);
        }
        if self.interact {
            self.hash.rebuild(&self.sites);
            for i in 0..2 * n {
                let pi = self.sites[i];
                let sites = &self.sites;
                let force = &mut self.force;
                let ghost = &self.ghost;
                if ghost[i % n] == GHOST {
                    continue;
                }
                self.hash.for_each_within(pi, CUT, |j| {
                    if j <= i || j == i + n || ghost[j % n] == GHOST {
                        return;
                    }
                    // The slots already space a sheet's heads.
                    let both_placed = ghost[i % n] == PLACED && ghost[j % n] == PLACED;
                    let pj = sites[j];
                    let (dx, dy) = (pi[0] - pj[0], pi[1] - pj[1]);
                    let r = dx.hypot(dy);
                    if !(r > 1e-9 && r < CUT) {
                        return;
                    }
                    // Positive pushes i away from j.
                    let mut f = 0.0;
                    if r < EV_R {
                        f += EV_K * (EV_R - r);
                    }
                    let (hi, hj) = (i < n, j < n);
                    if hi && hj && r < HH_R && !both_placed {
                        f += HH_K * (HH_R - r);
                    } else if !hi && !hj && r > EV_R && r < TT_R {
                        // Free micelles keep their full pull; the sheet's slots hold it.
                        let a = if both_placed {
                            drive.attract.min(ATTRACT_SHEET)
                        } else {
                            drive.attract
                        };
                        f -= a * (TT_R - r) / (TT_R - EV_R);
                    }
                    if f != 0.0 {
                        let (fx, fy) = (f * dx / r, f * dy / r);
                        force[i][0] += fx;
                        force[i][1] += fy;
                        force[j][0] -= fx;
                        force[j][1] -= fy;
                    }
                });
            }
        }
        let sd_free = (2.0 * drive.diffusion * dt).sqrt();
        let sd_slot = (2.0 * D_SLOT * dt).sqrt();
        let sd_r = (2.0 * D_R * dt).sqrt();
        for k in 0..n {
            let (fh, ft) = (self.force[k], self.force[k + n]);
            let u = unit(self.th[k]);
            let mut f = [fh[0] + ft[0], fh[1] + ft[1]];
            let mut tau = 0.5 * LEN * (u[0] * (ft[1] - fh[1]) - u[1] * (ft[0] - fh[0]));
            if let Some((sheet, since)) = drive.guide {
                let slot = self.slot[k];
                if slot.leaflet != 0 {
                    let on = ease::smoothstep((since - slot.t_on) / 1.5);
                    if on > 0.0 {
                        let (c, th) = Self::target(&sheet, &slot);
                        let (mut gx, mut gy) = (
                            GUIDE_K * on * (c[0] - self.c[k][0]),
                            GUIDE_K * on * (c[1] - self.c[k][1]),
                        );
                        // A stiff spring near the slot, a steady glide (`GUIDE_CAP`) from afar.
                        let (g, d) = (gx.hypot(gy), (GUIDE_K * on).recip() * gx.hypot(gy));
                        let cap = GUIDE_CAP + GUIDE_K * (1.5 - d).max(0.0);
                        if g > cap {
                            gx *= cap / g;
                            gy *= cap / g;
                        }
                        f[0] += gx;
                        f[1] += gy;
                        tau += GUIDE_TORQUE * on * (th - self.th[k]).sin_det();
                    }
                }
            }
            let placed = self.ghost[k] == PLACED;
            let (nx, ny) = gaussian2(rng, if placed { sd_slot } else { sd_free });
            let c = &mut self.c[k];
            c[0] = (c[0] + MU * f[0] * dt + nx).clamp(0.5, (self.w - 0.5).max(0.5));
            c[1] = (c[1] + MU * f[1] * dt + ny).clamp(0.5, (self.h - 0.5).max(0.5));
            let (nr, _) = gaussian2(
                rng,
                if placed {
                    sd_r * (D_SLOT / D_R).sqrt()
                } else {
                    sd_r
                },
            );
            self.th[k] = (self.th[k] + MU_R * tau * dt + nr).rem_euclid(std::f64::consts::TAU);
        }
        // Solutes: free diffusion, kept out of the membrane (no other force).
        let sd_s = (2.0 * D_SOLUTE * dt).sqrt();
        for q in 0..self.solutes.len() {
            let p = self.solutes[q];
            let mut f = [0.0, 0.0];
            if self.interact {
                let sites = &self.sites;
                self.hash.for_each_within(p, EV_R, |j| {
                    let (dx, dy) = (p[0] - sites[j][0], p[1] - sites[j][1]);
                    let r = dx.hypot(dy);
                    if r > 1e-9 && r < EV_R {
                        f[0] += EV_K * (EV_R - r) * dx / r;
                        f[1] += EV_K * (EV_R - r) * dy / r;
                    }
                });
            }
            let (nx, ny) = gaussian2(rng, sd_s);
            let s = &mut self.solutes[q];
            s[0] = (s[0] + MU * f[0] * dt + nx).clamp(0.3, (self.w - 0.3).max(0.3));
            s[1] = (s[1] + MU * f[1] * dt + ny).clamp(0.3, (self.h - 0.3).max(0.3));
        }
        self.place_sites();
    }
}

/// The act's drive at `local` seconds into act `act` (`sheet` from the soup).
pub(crate) fn drive(act: usize, local: f64, sheet: Sheet) -> Drive {
    match act {
        DISPERSED => Drive {
            diffusion: D_DISPERSE,
            ..Drive::OFF
        },
        MICELLES => Drive {
            attract: ATTRACT * ease::smoothstep(local / 2.0),
            diffusion: D_MICELLE,
            guide: None,
        },
        BILAYER => Drive {
            attract: ATTRACT,
            diffusion: D_T,
            guide: Some((sheet, local)),
        },
        _ => {
            let curv = std::f64::consts::TAU / sheet.len * ease::smoothstep(local / 8.0);
            Drive {
                attract: ATTRACT,
                diffusion: D_T,
                guide: Some((Sheet { curv, ..sheet }, local + DURATIONS[BILAYER])),
            }
        }
    }
}

// ---- Look (SPEC-002 Look) ----

/// Ink precedence in a shared braille cell (the stronger level wins): a head's centre
/// dot over a tail, a tail over a head's outer dots, so tails read `Cold` and heads `Hot`.
const HEAD_INK: Ink = Ink::new(Role::Hot, 235);
const HEAD_CORE_INK: Ink = Ink::new(Role::Hot, 255);
const TAIL_INK: Ink = Ink::new(Role::Cold, 245);
const SOLUTE_INK: Ink = Ink::new(Role::Accent, 230);
const HEAD_SIGMA: f64 = 0.5;
const HEAD_PEAK: f64 = 1.8;
const SOLUTE_SIGMA: f64 = 0.5;
const SOLUTE_PEAK: f64 = 1.6;
const SOLUTE_SALT: u64 = 0x9207_0CE1_0000_0002;
/// A tail is drawn to its bead's far edge (the bead's centre is the tail site), so the
/// two leaflets' tails meet at the mid-plane.
/// The tail thread starts this far (dots) from the head's centre.
const TAIL_GAP: f64 = 1.6;
/// Display exposure (s): long enough that thermal jitter does not flicker cells.
const EXPOSURE: f64 = 0.35;
/// Dots per box unit at least, and the box area (units²) the molecule cap fills.
const MIN_UNIT: f64 = 4.5;
const FULL_AREA: f64 = MAX_N as f64 / DENSITY;
const HEAD_SALT: u64 = 0x9207_0CE1_0000_0001;

#[derive(Debug)]
struct Protocell {
    time: f64,
    act: usize,
    timeline: Timeline,
    size: Size,
    soup: Soup,
    /// Displayed (exposed) head and tail positions, box units.
    shown: Vec<[f64; 2]>,
    shown_solutes: Vec<[f64; 2]>,
    dots: Dots,
    /// Dots per box unit.
    unit: f64,
    started: bool,
    dirty: bool,
}

impl Default for Protocell {
    fn default() -> Self {
        let phases: [(&'static str, f64); 4] = std::array::from_fn(|k| (CAPTIONS[k], DURATIONS[k]));
        Self {
            time: 0.0,
            act: DISPERSED,
            size: Size::new(1, 1),
            timeline: Timeline::new(&phases),
            soup: Soup::default(),
            shown: Vec::with_capacity(2 * MAX_N),
            shown_solutes: Vec::with_capacity(SOLUTES),
            dots: Dots::default(),
            unit: MIN_UNIT,
            started: false,
            dirty: true,
        }
    }
}

impl Protocell {
    fn snap_shown(&mut self) {
        self.shown.clear();
        self.shown.extend_from_slice(&self.soup.sites);
        self.shown_solutes.clear();
        self.shown_solutes.extend_from_slice(&self.soup.solutes);
    }

    fn expose(&mut self, dt: f64) {
        if self.shown.len() != self.soup.sites.len()
            || self.shown_solutes.len() != self.soup.solutes.len()
        {
            self.snap_shown();
            return;
        }
        let a = (dt / EXPOSURE).min(1.0);
        for (s, p) in self
            .shown
            .iter_mut()
            .zip(&self.soup.sites)
            .chain(self.shown_solutes.iter_mut().zip(&self.soup.solutes))
        {
            s[0] += a * (p[0] - s[0]);
            s[1] += a * (p[1] - s[1]);
        }
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        let n = self.soup.n.min(self.shown.len() / 2);
        let u = self.unit;
        let to_dot = |p: [f64; 2]| (p[0] * u - 0.5, p[1] * u - 0.5);
        for k in 0..n {
            let (h, t) = (to_dot(self.shown[k]), to_dot(self.shown[k + n]));
            let (dx, dy) = (t.0 - h.0, t.1 - h.1);
            let len = dx.hypot(dy);
            if len > TAIL_GAP + 0.5 {
                let (f, e) = (TAIL_GAP / len, 1.0 + 0.5 * EV_R * u / len);
                self.dots.line(
                    h.0 + f * dx,
                    h.1 + f * dy,
                    h.0 + e * dx,
                    h.1 + e * dy,
                    TAIL_INK,
                );
            }
        }
        for k in 0..n {
            let h = to_dot(self.shown[k]);
            self.dots.stipple_puff(
                h.0,
                h.1,
                HEAD_SIGMA,
                HEAD_PEAK,
                HEAD_SALT ^ (k as u64) << 8,
                HEAD_INK,
            );
            self.dots.plot(h.0, h.1, HEAD_CORE_INK);
        }
        for (q, &p) in self.shown_solutes.iter().enumerate() {
            let (x, y) = to_dot(p);
            self.dots.stipple_puff(
                x,
                y,
                SOLUTE_SIGMA,
                SOLUTE_PEAK,
                SOLUTE_SALT ^ (q as u64) << 8,
                SOLUTE_INK,
            );
        }
    }

    fn enter(&mut self, act: usize, rng: &mut impl Rng) {
        if act == BILAYER || (act == VESICLE && self.act != BILAYER) {
            let sheet = self.soup.sheet();
            self.soup.assign(&sheet, 0.5 * DURATIONS[BILAYER], rng);
            self.snap_shown();
        }
        if act == DISPERSED {
            for s in &mut self.soup.slot {
                *s = Slot::default();
            }
        }
        self.act = act;
    }
}

impl Generator for Protocell {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.time).index])
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let dw = f64::from(size.width()) * f64::from(Dots::CELL_W);
        let dh = f64::from(size.height().saturating_sub(1).max(1)) * f64::from(Dots::CELL_H);
        self.unit = (dw * dh / FULL_AREA).sqrt().max(MIN_UNIT);
        let (w, h) = ((dw / self.unit).max(2.0), (dh / self.unit).max(2.0));
        let n = ((w * h * DENSITY) as usize).clamp(MIN_N, MAX_N);
        self.soup.resize(w, h, n, &mut ctx.rng);
        if !self.started {
            self.started = true;
            self.soup.scatter(&mut ctx.rng);
        }
        // A new box needs new sheet slots mid-assembly.
        let act = self.timeline.at(self.time).index;
        if act == BILAYER || act == VESICLE {
            let sheet = self.soup.sheet();
            self.soup.assign(&sheet, 0.0, &mut ctx.rng);
        }
        self.snap_shown();
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, ctx: &mut Ctx) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        let steps = (dt / MAX_DT).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as u32 {
            self.time += h;
            let beat = self.timeline.at(self.time);
            if beat.index != self.act {
                self.enter(beat.index, &mut ctx.rng);
            }
            let d = drive(beat.index, beat.local, self.soup.sheet());
            self.soup.step(h, &d, &mut ctx.rng);
            self.expose(h);
        }
        self.dirty = true;
    }

    /// Rasterise once per drawn frame (GEN-R20).
    fn prepare(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.rasterise();
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
    use std::collections::VecDeque;

    fn started(seed: u64, size: Size) -> (Protocell, Ctx) {
        let mut ctx = Ctx::new(seed, size);
        let mut g = Protocell::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    fn run_to(g: &mut Protocell, ctx: &mut Ctx, t: f64) {
        while g.time < t - 1e-9 {
            g.update(MAX_DT, ctx);
        }
    }

    #[test]
    fn proto_a1_free_diffusion_has_variance_2ddt() {
        let mut rng = SplitMix64::new(1);
        let mut soup = Soup::default();
        soup.resize(1000.0, 1000.0, MAX_N, &mut rng);
        soup.scatter(&mut rng);
        soup.interact = false;
        let dt = MAX_DT;
        let (mut sum, mut count) = (0.0, 0);
        for _ in 0..200 {
            let before = soup.c.clone();
            soup.step(dt, &Drive::OFF, &mut rng);
            for (a, b) in before.iter().zip(&soup.c) {
                // Skip the rare molecule held at a wall.
                if b[0] > 1.0 && b[0] < 999.0 && b[1] > 1.0 && b[1] < 999.0 {
                    sum += (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
                    count += 2;
                }
            }
        }
        let var = sum / f64::from(count);
        let want = 2.0 * D_T * dt;
        assert!(
            (var / want - 1.0).abs() < 0.05,
            "variance {var:.5} vs 2DΔt {want:.5}"
        );
    }

    /// Tails whose nearest site is a tail, and heads outward of their cluster's centre.
    #[allow(clippy::needless_range_loop)]
    fn micelle_stats(s: &Soup) -> (f64, f64) {
        let n = s.n;
        let mut tail_nn = 0;
        for i in n..2 * n {
            let (mut best, mut best_tail) = (f64::INFINITY, false);
            for j in 0..2 * n {
                if j == i || j + n == i {
                    continue;
                }
                let d = (s.sites[i][0] - s.sites[j][0]).hypot(s.sites[i][1] - s.sites[j][1]);
                if d < best {
                    best = d;
                    best_tail = j >= n;
                }
            }
            tail_nn += usize::from(best_tail);
        }
        // Clusters: tails within 1.6 of each other.
        let mut label = vec![usize::MAX; n];
        let mut outward = (0usize, 0usize);
        for start in 0..n {
            if label[start] != usize::MAX {
                continue;
            }
            let mut members = vec![start];
            label[start] = start;
            let mut q = VecDeque::from([start]);
            while let Some(a) = q.pop_front() {
                for b in 0..n {
                    let (ta, tb) = (s.sites[a + n], s.sites[b + n]);
                    if label[b] == usize::MAX && (ta[0] - tb[0]).hypot(ta[1] - tb[1]) < 1.6 {
                        label[b] = start;
                        members.push(b);
                        q.push_back(b);
                    }
                }
            }
            if members.len() >= 3 {
                let m = members.len() as f64;
                let cx = members.iter().map(|&k| s.c[k][0]).sum::<f64>() / m;
                let cy = members.iter().map(|&k| s.c[k][1]).sum::<f64>() / m;
                for &k in &members {
                    let (h, t) = (s.sites[k], s.sites[k + n]);
                    let dh = (h[0] - cx).hypot(h[1] - cy);
                    let dt = (t[0] - cx).hypot(t[1] - cy);
                    outward.0 += usize::from(dh > dt);
                    outward.1 += 1;
                }
            }
        }
        (
            tail_nn as f64 / n as f64,
            outward.0 as f64 / outward.1.max(1) as f64,
        )
    }

    #[test]
    fn proto_a2_micelles_hide_their_tails() {
        // One run per seed: chance level at t = 5 s (no attraction yet), micelles late in
        // their act. "Most" tails, so two seeds keep the debug run short.
        for seed in [1, 2] {
            let (mut g, mut ctx) = started(seed, Size::new(200, 50));
            run_to(&mut g, &mut ctx, 5.0);
            let (before, _) = micelle_stats(&g.soup);
            run_to(
                &mut g,
                &mut ctx,
                DURATIONS[DISPERSED] + DURATIONS[MICELLES] - 0.5,
            );
            assert_eq!(g.caption(), Some("micelles"));
            let (tails, out) = micelle_stats(&g.soup);
            assert!(
                tails > 0.5,
                "seed {seed}: {tails:.2} of tails next to a tail"
            );
            assert!(out > 0.7, "seed {seed}: {out:.2} of heads outward");
            assert!(
                tails > before + 0.15,
                "seed {seed}: {before:.2} → {tails:.2}"
            );
        }
    }

    /// The head-contact graph's edges rasterised on a fine grid; flooding the water from
    /// the box edge, the cells it cannot reach around `centre` are enclosed (units²): the
    /// lumen of a closed ring of heads about it.
    pub(crate) fn enclosed_area(s: &Soup, centre: [f64; 2]) -> f64 {
        let res = 0.2;
        let (gw, gh) = (
            (s.w / res).ceil() as usize + 2,
            (s.h / res).ceil() as usize + 2,
        );
        let mut wall = vec![false; gw * gh];
        let n = s.n;
        for a in 0..n {
            for b in a + 1..n {
                let (p, q) = (s.sites[a], s.sites[b]);
                let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                if d < CONTACT {
                    let steps = (d / (0.5 * res)).ceil() as usize + 1;
                    for i in 0..=steps {
                        let f = i as f64 / steps as f64;
                        let (x, y) = (p[0] + f * (q[0] - p[0]), p[1] + f * (q[1] - p[1]));
                        let (gx, gy) = ((x / res) as usize + 1, (y / res) as usize + 1);
                        // Thicken by one cell so diagonal leaks are closed.
                        for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            if gx + ox < gw && gy + oy < gh {
                                wall[(gy + oy) * gw + gx + ox] = true;
                            }
                        }
                    }
                }
            }
        }
        let mut seen = vec![false; gw * gh];
        let mut q = VecDeque::from([0usize]);
        seen[0] = true;
        while let Some(i) = q.pop_front() {
            let (x, y) = (i % gw, i / gw);
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < gw && ny < gh {
                    let j = ny * gw + nx;
                    if !seen[j] && !wall[j] {
                        seen[j] = true;
                        q.push_back(j);
                    }
                }
            }
        }
        // The unreached component holding the centre.
        let c = ((centre[1] / res) as usize + 1) * gw + (centre[0] / res) as usize + 1;
        if c >= gw * gh || seen[c] || wall[c] {
            return 0.0;
        }
        let mut lumen = vec![false; gw * gh];
        lumen[c] = true;
        let mut q = VecDeque::from([c]);
        let mut count = 0u32;
        while let Some(i) = q.pop_front() {
            count += 1;
            let (x, y) = (i % gw, i / gw);
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < gw && ny < gh {
                    let j = ny * gw + nx;
                    if !lumen[j] && !wall[j] {
                        lumen[j] = true;
                        q.push_back(j);
                    }
                }
            }
        }
        f64::from(count) * res * res
    }

    #[test]
    fn proto_a3_a_vesicle_closes_within_its_act() {
        let start = DURATIONS[DISPERSED] + DURATIONS[MICELLES] + DURATIONS[BILAYER];
        for seed in [1, 2, 3] {
            for size in [Size::new(80, 24)] {
                let (mut g, mut ctx) = started(seed, size);
                run_to(&mut g, &mut ctx, start + 0.5);
                assert_eq!(g.caption(), Some("vesicle"));
                let sheet = g.soup.sheet();
                let centre = [sheet.mid[0], sheet.mid[1] - sheet.radius()];
                let open = enclosed_area(&g.soup, centre);
                let r = g.soup.sheet().radius();
                let inner = std::f64::consts::PI * (r - GAP - LEN).max(0.5).powi(2);
                // Sampled every 0.5 s over the act's last 5 s: the ring has closed.
                let (mut area, mut closed) = (0.0f64, 0);
                for k in 0..10 {
                    run_to(
                        &mut g,
                        &mut ctx,
                        start + DURATIONS[VESICLE] - 5.0 + 0.5 * f64::from(k),
                    );
                    let a = enclosed_area(&g.soup, centre);
                    closed += usize::from(a > 0.5 * inner);
                    area = area.max(a);
                }
                assert_eq!(g.caption(), Some("vesicle"));
                assert!(
                    closed >= 5,
                    "seed {seed} {size:?}: enclosed {area:.1} (sheet open: {open:.1}), lumen ≈ {inner:.1}"
                );
            }
        }
    }

    #[test]
    fn proto_a4_captions_follow_the_acts_and_loop() {
        let (mut g, mut ctx) = started(4, Size::new(80, 24));
        let total: f64 = DURATIONS.iter().sum();
        let mut seen: Vec<&str> = Vec::new();
        while g.time < 2.0 * total + 1.0 {
            let c = g.caption().expect("caption");
            if seen.last() != Some(&c) {
                seen.push(c);
            }
            g.update(0.25, &mut ctx);
        }
        let mut want = CAPTIONS.to_vec();
        want.extend(CAPTIONS);
        want.push(CAPTIONS[0]);
        assert_eq!(seen, want);
    }

    #[test]
    fn proto_a5_heads_hot_tails_cold_solutes_accent() {
        let size = Size::new(120, 40);
        let (mut g, mut ctx) = started(5, size);
        for t in [3.0, 15.0, 25.0, 38.0] {
            run_to(&mut g, &mut ctx, t);
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let mut roles = [0usize; 3];
            for c in canvas.cells() {
                if c.ch == ' ' {
                    continue;
                }
                assert!(c.bg == Ink::BLANK && !matches!(c.ch, '▀' | '▄' | '█'));
                let i = match c.ink {
                    HEAD_INK | HEAD_CORE_INK => 0,
                    TAIL_INK => 1,
                    SOLUTE_INK => 2,
                    other => panic!("t {t}: {other:?}"),
                };
                roles[i] += 1;
            }
            assert!(roles[0] > 20 && roles[1] > 0, "t {t}: {roles:?}");
        }
    }

    fn sheet_soup(seed: u64) -> (Soup, SplitMix64) {
        let mut rng = SplitMix64::new(seed);
        let mut soup = Soup::default();
        soup.resize(90.0, 44.0, 270, &mut rng);
        soup.scatter(&mut rng);
        (soup, rng)
    }

    #[test]
    fn proto_solutes_are_not_dragged_into_the_vesicle() {
        // Review round 1: solutes only diffuse; none is pulled towards the lumen.
        let (mut soup, mut rng) = sheet_soup(7);
        soup.interact = false;
        let sheet = soup.sheet();
        let closed = Sheet {
            curv: std::f64::consts::TAU / sheet.len,
            ..sheet
        };
        let centre = [closed.mid[0], closed.mid[1] - closed.radius()];
        let drive = Drive {
            attract: 0.0,
            diffusion: D_T,
            guide: Some((closed, 20.0)),
        };
        let (mut inward, mut count) = (0.0, 0);
        for rep in 0..5 {
            let r0 = 0.8 * closed.radius();
            let m = soup.solutes.len();
            for (q, p) in soup.solutes.iter_mut().enumerate() {
                let th = std::f64::consts::TAU * (q as f64 + 0.3 * f64::from(rep)) / m as f64;
                let (sn, cs) = th.sin_cos_det();
                *p = [centre[0] + r0 * cs, centre[1] + r0 * sn];
            }
            for _ in 0..120 {
                soup.step(MAX_DT, &drive, &mut rng);
            }
            for p in &soup.solutes {
                inward += r0 - (p[0] - centre[0]).hypot(p[1] - centre[1]);
                count += 1;
            }
        }
        let mean = inward / f64::from(count);
        assert!(mean.abs() < 0.6, "mean inward drift {mean:.2} units in 2 s");
    }

    #[test]
    fn proto_some_solutes_start_in_the_lumen() {
        let (mut soup, mut rng) = sheet_soup(8);
        let sheet = soup.sheet();
        soup.assign(&sheet, 5.0, &mut rng);
        let r = sheet.radius() - GAP - LEN - 0.5;
        let centre = [sheet.mid[0], sheet.mid[1] - sheet.radius()];
        let inside = soup
            .solutes
            .iter()
            .filter(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]) < r)
            .count();
        assert!(inside >= 3, "{inside} solutes in the future lumen");
    }

    #[test]
    fn proto_flat_sheet_leaflets_are_equally_long() {
        let (mut soup, mut rng) = sheet_soup(9);
        let sheet = soup.sheet();
        soup.assign(&sheet, 5.0, &mut rng);
        // Review round 2: the two leaflets of the flat sheet span the same length.
        let extent = |leaf: i8| {
            let xs = (0..soup.n)
                .filter(|&k| soup.slot[k].leaflet == leaf)
                .map(|k| Soup::target(&sheet, &soup.slot[k]).0[0]);
            let (lo, hi) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
            (lo, hi)
        };
        let (a, b) = (extent(1), extent(-1));
        let tol = 2.0 * SPACING;
        assert!(
            (a.0 - b.0).abs() < tol && (a.1 - b.1).abs() < tol,
            "flat leaflets span {a:.1?} vs {b:.1?}"
        );
    }

    #[test]
    fn proto_tails_meet_mid_sheet() {
        let (mut soup, mut rng) = sheet_soup(10);
        let sheet = soup.sheet();
        soup.assign(&sheet, 5.0, &mut rng);
        for k in 0..soup.n {
            let slot = soup.slot[k];
            if slot.leaflet == 0 {
                continue;
            }
            let (c, th) = Soup::target(&sheet, &slot);
            let u = unit(th);
            let tail = [c[0] + 0.5 * LEN * u[0], c[1] + 0.5 * LEN * u[1]];
            // Flat sheet: the mid-plane is the line y = mid.y.
            let gap = (tail[1] - sheet.mid[1]).abs();
            assert!(
                (gap - 0.5 * EV_R).abs() < 1e-9,
                "tail bead {gap:.3} from the mid-plane, want touching ({})",
                0.5 * EV_R
            );
        }
    }

    #[test]
    fn proto_tail_dots_are_never_hot() {
        // Review round 2: a cell holding tail dots but no head centre is Cold.
        let size = Size::new(200, 50);
        let (mut g, mut ctx) = started(3, size);
        for t in [17.0, 26.0, 38.0] {
            run_to(&mut g, &mut ctx, t);
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let n = g.soup.n;
            let u = g.unit;
            let cell_of = |p: (f64, f64)| {
                (
                    (p.0.round() as i32).div_euclid(Dots::CELL_W),
                    (p.1.round() as i32).div_euclid(Dots::CELL_H),
                )
            };
            let heads: std::collections::HashSet<(i32, i32)> = (0..n)
                .map(|k| cell_of((g.shown[k][0] * u - 0.5, g.shown[k][1] * u - 0.5)))
                .collect();
            let mut checked = 0;
            let mut tail = Dots::new(size);
            for k in 0..n {
                let h = (g.shown[k][0] * u - 0.5, g.shown[k][1] * u - 0.5);
                let tl = (g.shown[k + n][0] * u - 0.5, g.shown[k + n][1] * u - 0.5);
                let (dx, dy) = (tl.0 - h.0, tl.1 - h.1);
                let len = dx.hypot(dy);
                if len <= TAIL_GAP + 0.5 {
                    continue;
                }
                // The tail's own dots, drawn exactly as `rasterise` draws them.
                tail.clear();
                let (f, e) = (TAIL_GAP / len, 1.0 + 0.5 * EV_R * u / len);
                tail.line(
                    h.0 + f * dx,
                    h.1 + f * dy,
                    h.0 + e * dx,
                    h.1 + e * dy,
                    TAIL_INK,
                );
                let (x0, x1) = (h.0.min(tl.0) as i32 - 4, h.0.max(tl.0) as i32 + 4);
                let (y0, y1) = (h.1.min(tl.1) as i32 - 4, h.1.max(tl.1) as i32 + 4);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let away = (f64::from(x) - h.0).hypot(f64::from(y) - h.1) > 2.5;
                        if !away || !tail.is_lit(x, y) {
                            continue;
                        }
                        let c = (x.div_euclid(Dots::CELL_W), y.div_euclid(Dots::CELL_H));
                        if !heads.contains(&c)
                            && let Some(cell) = canvas.get(c.0, c.1)
                        {
                            assert_ne!(
                                cell.ink.role,
                                Role::Hot,
                                "t {t}: tail of {k} in cell {c:?}"
                            );
                            checked += 1;
                        }
                    }
                }
            }
            assert!(checked > 100, "t {t}: {checked} tail cells checked");
        }
    }
}
