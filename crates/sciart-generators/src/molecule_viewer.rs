//! Tumbling ball-and-stick models of real molecules, from water to buckminsterfullerene
//! (SPEC-002 `molecule_viewer`).
//!
//! Six molecules take turns on a dark bench: water, methane, ethanol, benzene, caffeine
//! and C60. Each grows out of a point, turns for 8.8 s in a lit perspective view, and
//! collapses back before the next one grows. Everything is a closed-form function of
//! scene time. `update` advances the clock, projects at most 60 atoms and paints them and
//! their bonds as shaded spheres and cylinders into a z-buffered half-block raster;
//! `render` only blits it and writes the readout.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::geom::v3::{dot, norm, sub};
use sciart_core::ink::{self, level};
use sciart_core::prelude::*;
use sciart_core::title::{self, Form, Title};
use sciart_core::{draw, ease};

use El::{C, H, N, O};
use Order::{Aromatic, Double, Single};

pub static META: Meta = Meta {
    name: "molecule_viewer",
    category: Category::Chemistry,
    description: "Tumbling ball-and-stick models of real molecules, from water to buckminsterfullerene",
    title: "Molecule · atoms share electrons",
    captions: &CAPTIONS,
};

pub fn build() -> Box<dyn Generator> {
    Box::new(MoleculeViewer::new())
}

// ---------------------------------------------------------------------------------------
// Chemistry.
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum El {
    H,
    C,
    N,
    O,
}

impl El {
    /// Bondi (1964) van der Waals radius, Å.
    fn vdw(self) -> f64 {
        match self {
            El::H => 1.20,
            El::C => 1.70,
            El::N => 1.55,
            El::O => 1.52,
        }
    }

    /// Element symbol: upper-case in the near half, lower-case in the far half (MOL-A6).
    fn symbol(self, near: bool) -> char {
        let c = match self {
            El::H => 'H',
            El::C => 'C',
            El::N => 'N',
            El::O => 'O',
        };
        if near { c } else { c.to_ascii_lowercase() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    Single,
    Double,
    /// Bond order 1.5 (benzene).
    Aromatic,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Atom {
    pub el: El,
    /// Position in Å; x right, y up, z toward the viewer (CORE-R45).
    pub pos: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bond {
    pub a: u8,
    pub b: u8,
    pub order: Order,
}

/// A molecule as typed in: coordinates in any origin, recentred by [`Model::new`].
struct Table {
    name: &'static str,
    formula: &'static str,
    /// Molar mass, g/mol, to 2 decimals (MOL-A1 checks it against the atoms).
    mass: &'static str,
    atoms: &'static [Atom],
    bonds: &'static [Bond],
}

const fn at(el: El, x: f64, y: f64, z: f64) -> Atom {
    Atom { el, pos: [x, y, z] }
}

const fn bd(a: u8, b: u8, order: Order) -> Bond {
    Bond { a, b, order }
}

/// r(OH) 0.9572 Å, ∠HOH 104.52° (Benedict, Gailar & Plyler 1956); plane = xy.
static WATER: Table = Table {
    name: CAPTIONS[0],
    formula: "H2O",
    mass: "18.02",
    atoms: &[
        at(O, 0.0, 0.0, 0.0),
        at(H, 0.756_950, -0.585_882, 0.0),
        at(H, -0.756_950, -0.585_882, 0.0),
    ],
    bonds: &[bd(0, 1, Single), bd(0, 2, Single)],
};

/// T_d, r(CH) 1.087 Å: hydrogens on alternate cube corners, `1.087/√3` per axis.
static METHANE: Table = Table {
    name: CAPTIONS[1],
    formula: "CH4",
    mass: "16.04",
    atoms: &[
        at(C, 0.0, 0.0, 0.0),
        at(H, 0.627_580, 0.627_580, 0.627_580),
        at(H, 0.627_580, -0.627_580, -0.627_580),
        at(H, -0.627_580, 0.627_580, -0.627_580),
        at(H, -0.627_580, -0.627_580, 0.627_580),
    ],
    bonds: &[
        bd(0, 1, Single),
        bd(0, 2, Single),
        bd(0, 3, Single),
        bd(0, 4, Single),
    ],
};

/// Trans (anti H–O–C–C) ethanol: r(CC) 1.512, r(CO) 1.431, r(OH) 0.971 Å, ∠CCO 107.8°,
/// ∠COH 105.4° (NIST CCCBDB experimental); C–H 1.09 Å at tetrahedral angles. The mirror
/// plane (C, C, O and the two anti hydrogens) is xy.
static ETHANOL: Table = Table {
    name: CAPTIONS[2],
    formula: "C2H6O",
    mass: "46.07",
    atoms: &[
        at(C, 1.512, 0.0, 0.0),
        at(C, 0.0, 0.0, 0.0),
        at(O, -0.437_450, 1.362_497, 0.0),
        at(H, -1.407_598, 1.321_836, 0.0),
        at(H, 1.875_333, -1.027_662, 0.0),
        at(H, 1.875_333, 0.513_831, 0.889_981),
        at(H, 1.875_333, 0.513_831, -0.889_981),
        at(H, -0.370_788, -0.508_478, 0.889_981),
        at(H, -0.370_788, -0.508_478, -0.889_981),
    ],
    bonds: &[
        bd(0, 1, Single),
        bd(1, 2, Single),
        bd(2, 3, Single),
        bd(0, 4, Single),
        bd(0, 5, Single),
        bd(0, 6, Single),
        bd(1, 7, Single),
        bd(1, 8, Single),
    ],
};

/// D₆h, r(CC) 1.397, r(CH) 1.084 Å (Herzberg 1966); ring in xy, `√3/2 = 0.866 025 4`.
static BENZENE: Table = Table {
    name: CAPTIONS[3],
    formula: "C6H6",
    mass: "78.11",
    atoms: &[
        at(C, 1.397, 0.0, 0.0),
        at(C, 0.698_5, 1.209_837, 0.0),
        at(C, -0.698_5, 1.209_837, 0.0),
        at(C, -1.397, 0.0, 0.0),
        at(C, -0.698_5, -1.209_837, 0.0),
        at(C, 0.698_5, -1.209_837, 0.0),
        at(H, 2.481, 0.0, 0.0),
        at(H, 1.240_5, 2.148_609, 0.0),
        at(H, -1.240_5, 2.148_609, 0.0),
        at(H, -2.481, 0.0, 0.0),
        at(H, -1.240_5, -2.148_609, 0.0),
        at(H, 1.240_5, -2.148_609, 0.0),
    ],
    bonds: &[
        bd(0, 1, Aromatic),
        bd(1, 2, Aromatic),
        bd(2, 3, Aromatic),
        bd(3, 4, Aromatic),
        bd(4, 5, Aromatic),
        bd(5, 0, Aromatic),
        bd(0, 6, Single),
        bd(1, 7, Single),
        bd(2, 8, Single),
        bd(3, 9, Single),
        bd(4, 10, Single),
        bd(5, 11, Single),
    ],
};

/// Caffeine, the PubChem CID 2519 3-D conformer, atoms and bonds in its SDF order (the
/// purine plane is already xy). Kekulé orders: C2=O, C6=O, C4=C5 and C8=N9.
static CAFFEINE: Table = Table {
    name: CAPTIONS[4],
    formula: "C8H10N4O2",
    mass: "194.19",
    atoms: &[
        at(O, 0.4700, 2.5688, 0.0006),
        at(O, -3.1271, -0.4436, -0.0003),
        at(N, -0.9686, -1.3125, 0.0000),
        at(N, 2.2182, 0.1412, -0.0003),
        at(N, -1.3477, 1.0797, -0.0001),
        at(N, 1.4119, -1.9372, 0.0002),
        at(C, 0.8579, 0.2592, -0.0008),
        at(C, 0.3897, -1.0264, -0.0004),
        at(C, 0.0307, 1.4220, -0.0006),
        at(C, -1.9061, -0.2495, -0.0004),
        at(C, 2.5032, -1.1998, 0.0003),
        at(C, -1.4276, -2.6960, 0.0008),
        at(C, 3.1926, 1.2061, 0.0003),
        at(C, -2.2969, 2.1881, 0.0007),
        at(H, 3.5163, -1.5787, 0.0008),
        at(H, -1.0451, -3.1973, -0.8937),
        at(H, -2.5186, -2.7596, 0.0011),
        at(H, -1.0447, -3.1963, 0.8957),
        at(H, 4.1992, 0.7801, 0.0002),
        at(H, 3.0468, 1.8092, -0.8992),
        at(H, 3.0466, 1.8083, 0.9004),
        at(H, -1.8087, 3.1651, -0.0003),
        at(H, -2.9322, 2.1027, 0.8881),
        at(H, -2.9346, 2.1021, -0.8849),
    ],
    bonds: &[
        bd(0, 8, Double),
        bd(1, 9, Double),
        bd(2, 7, Single),
        bd(2, 9, Single),
        bd(2, 11, Single),
        bd(3, 6, Single),
        bd(3, 10, Single),
        bd(3, 12, Single),
        bd(4, 8, Single),
        bd(4, 9, Single),
        bd(4, 13, Single),
        bd(5, 7, Single),
        bd(5, 10, Double),
        bd(6, 7, Double),
        bd(6, 8, Single),
        bd(10, 14, Single),
        bd(11, 15, Single),
        bd(11, 16, Single),
        bd(11, 17, Single),
        bd(12, 18, Single),
        bd(12, 19, Single),
        bd(12, 20, Single),
        bd(13, 21, Single),
        bd(13, 22, Single),
        bd(13, 23, Single),
    ],
};

/// C60 bond lengths, Å (Hedberg et al. 1991, gas-phase electron diffraction).
const R56: f64 = 1.458;
const R66: f64 = 1.401;
const C60_ATOMS: usize = 60;

/// Buckminsterfullerene as a truncated icosahedron (MOL-A2): each of the 30 edges of an
/// icosahedron with vertices at the cyclic permutations of `(0, ±1, ±φ)` (edge 2) carries
/// two atoms at fractions `t` and `1 − t`, scaled by `a/2` with `a = 2·r56 + r66` and
/// `t = r56/a`. The pair on one old edge is a 6:6 double bond; atoms at `r56` across a
/// shared vertex are 5:6 single bonds (one Kekulé structure: every C gets order 4).
pub(crate) fn c60(r56: f64, r66: f64) -> (Vec<Atom>, Vec<Bond>) {
    let phi = (1.0 + 5f64.sqrt()) / 2.0;
    let mut vertices = Vec::with_capacity(12);
    for s1 in [-1.0, 1.0] {
        for s2 in [-phi, phi] {
            vertices.extend([[0.0, s1, s2], [s1, s2, 0.0], [s2, 0.0, s1]]);
        }
    }
    let a = 2.0 * r56 + r66;
    let (t, scale) = (r56 / a, a / 2.0);
    let mut atoms = Vec::with_capacity(C60_ATOMS);
    let mut bonds = Vec::with_capacity(90);
    for (i, u) in vertices.iter().enumerate() {
        for v in &vertices[i + 1..] {
            if (dist(*u, *v) - 2.0).abs() > 1e-9 {
                continue; // not an icosahedron edge
            }
            let along = |f: f64| -> [f64; 3] {
                std::array::from_fn(|k| scale * (u[k] + f * (v[k] - u[k])))
            };
            let n = atoms.len() as u8;
            atoms.push(Atom {
                el: C,
                pos: along(t),
            });
            atoms.push(Atom {
                el: C,
                pos: along(1.0 - t),
            });
            bonds.push(bd(n, n + 1, Double));
        }
    }
    for i in 0..atoms.len() {
        for j in i + 1..atoms.len() {
            if (dist(atoms[i].pos, atoms[j].pos) - r56).abs() < 1e-6 {
                bonds.push(bd(i as u8, j as u8, Single));
            }
        }
    }
    (atoms, bonds)
}

pub(crate) fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm(sub(a, b))
}

/// Atomic mass, u (IUPAC abridged), used only to find the centre of mass.
fn mass(el: El) -> f64 {
    match el {
        El::H => 1.008,
        El::C => 12.011,
        El::N => 14.007,
        El::O => 15.999,
    }
}

/// Ball radius as a fraction of the Bondi (1964) van der Waals radius. It keeps ≥ 0.5 Å
/// of visible stick on C60's 1.40–1.46 Å bonds, so the cage reads as rings of balls
/// rather than a heap of blobs.
const BALL: f64 = 0.20;

fn ball_radius(el: El) -> f64 {
    BALL * el.vdw()
}

/// Smallest `r_eff` (Å), so the lightest molecules are framed but not ballooned: water
/// (raw r_eff ≈ 1.16 Å) reaches ≈ 0.9 of the framing radius at its widest, and its
/// small V still covers less of the screen than benzene and C60.
const R_EFF_MIN: f64 = 1.25;

/// A molecule ready to draw: recentred on its centre of mass, with its framing radius
/// and its label tiers.
#[derive(Debug)]
pub(crate) struct Model {
    pub atoms: Vec<Atom>,
    pub bonds: Vec<Bond>,
    /// `max(|rᵢ| + r_ball,ᵢ, R_EFF_MIN)`: the radius the camera frames.
    pub r_eff: f64,
    /// Label forms: full, without the mass, without the name, the formula alone (MOL-A6).
    pub labels: [String; 4],
}

/// The label forms (indices into `Model::labels`) tried longest first: with the name, or
/// without it while the title already names the molecule (SPEC-002 Label).
fn tiers(named: bool) -> &'static [usize] {
    if named { &[2, 3] } else { &[0, 1, 3] }
}

impl Model {
    fn new(
        name: &str,
        formula: &str,
        mass_label: &str,
        atoms: Vec<Atom>,
        bonds: Vec<Bond>,
    ) -> Self {
        // Build-time guard for the scene's per-atom arrays and the u8 pixel owners;
        // never reached on the frame path.
        assert!(atoms.len() <= MAX_ATOMS, "{name}: raise MAX_ATOMS");
        let total: f64 = atoms.iter().map(|a| mass(a.el)).sum();
        let com: [f64; 3] = std::array::from_fn(|k| {
            atoms.iter().map(|a| mass(a.el) * a.pos[k]).sum::<f64>() / total
        });
        let atoms: Vec<Atom> = atoms
            .into_iter()
            .map(|a| Atom {
                el: a.el,
                pos: std::array::from_fn(|k| a.pos[k] - com[k]),
            })
            .collect();
        let r_eff = atoms
            .iter()
            .map(|a| norm(a.pos) + ball_radius(a.el))
            .fold(R_EFF_MIN, f64::max);
        Self {
            atoms,
            bonds,
            r_eff,
            labels: [
                format!("{name}  {formula}  {mass_label} g/mol"),
                format!("{name}  {formula}"),
                format!("{formula}  {mass_label} g/mol"),
                formula.to_owned(),
            ],
        }
    }

    fn from_table(t: &Table) -> Self {
        Self::new(
            t.name,
            t.formula,
            t.mass,
            t.atoms.to_vec(),
            t.bonds.to_vec(),
        )
    }

    /// The longest label tier that fits in `room` columns, or `""` (MOL-A6); `named`
    /// leaves out the name, which the title's caption already shows.
    pub fn label(&self, room: usize, named: bool) -> &str {
        tiers(named)
            .iter()
            .map(|&i| self.labels[i].as_str())
            .find(|l| l.chars().count() <= room)
            .unwrap_or("")
    }
}

/// The gallery, in viewing order (SPEC-002).
pub(crate) fn gallery() -> Vec<Model> {
    let (atoms, bonds) = c60(R56, R66);
    let mut models: Vec<Model> = [&WATER, &METHANE, &ETHANOL, &BENZENE, &CAFFEINE]
        .into_iter()
        .map(Model::from_table)
        .collect();
    models.push(Model::new(CAPTIONS[5], "C60", "720.66", atoms, bonds));
    models
}

const MOLECULES: usize = 6;
/// Molecule names in gallery order; the shown one is the title's caption (GEN-R12),
/// `Molecule · caffeine`.
const CAPTIONS: [&str; MOLECULES] = [
    "water",
    "methane",
    "ethanol",
    "benzene",
    "caffeine",
    "buckminsterfullerene",
];
/// C60 is the largest molecule. Pixel owners are `u8`, so this stays below 255.
const MAX_ATOMS: usize = C60_ATOMS;

// ---------------------------------------------------------------------------------------
// Motion (MOL-A5).
// ---------------------------------------------------------------------------------------

const GROW: f64 = 0.6;
const HOLD: f64 = 8.8;
const SHRINK: f64 = 0.6;
const SLOT: f64 = GROW + HOLD + SHRINK;
/// One turn per slot.
const OMEGA: f64 = TAU / SLOT;
/// Yaw at the slot start, so each molecule reaches full size in a three-quarter view.
const YAW0: f64 = -0.78;
const PITCH_MEAN: f64 = 0.30;
const PITCH_SWING: f64 = 0.30;
const PITCH_PERIOD: f64 = 17.0;

fn gallery_timeline() -> Timeline {
    Timeline::new(&[("grow", GROW), ("hold", HOLD), ("shrink", SHRINK)])
}

/// Everything time decides: which molecule, how grown, and the view angles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    pub molecule: usize,
    /// Gallery scale `s ∈ [0, 1]`: 0 at slot boundaries, 1 throughout `hold`.
    pub s: f64,
    pub yaw: f64,
    pub pitch: f64,
}

pub(crate) fn pose(timeline: &Timeline, start: usize, t: f64) -> Pose {
    let beat = timeline.at(t);
    let s = match beat.index {
        0 => ease::smoothstep(beat.progress),
        1 => 1.0,
        _ => 1.0 - ease::smoothstep(beat.progress),
    };
    let cycle = timeline.cycle(t);
    let tau = t.rem_euclid(timeline.total());
    Pose {
        molecule: (start as i64 + cycle).rem_euclid(MOLECULES as i64) as usize,
        s,
        yaw: OMEGA * tau + YAW0,
        pitch: PITCH_MEAN + PITCH_SWING * (TAU * t / PITCH_PERIOD).sin(),
    }
}

// ---------------------------------------------------------------------------------------
// View and projection (MOL-A3).
// ---------------------------------------------------------------------------------------

/// Eye distance in units of `r_eff`: gentle perspective.
const EYE: f64 = 5.0;
/// Perspective allowance in the scale. A sphere of radius `r_eff` seen from `EYE·r_eff`
/// projects to at most `EYE/√(EYE² − 1)` ≈ 1.021 of its radius; the ≈ 3 % on top covers
/// rounding to pixels (MOL-A3 checks the fit at eight sizes).
const MAX_MAGNIFY: f64 = 1.05;
/// Fraction of the art area the framing circle fills.
const FILL: f64 = 0.95;

/// Rows of the art area: all but the title row, which the readout shares (GEN-R13).
fn art_rows(size: Size) -> u16 {
    if size.height() >= 6 {
        size.height() - 1
    } else {
        size.height()
    }
}

/// Screen framing for one size: the art rows `[0, art_rows)`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct View {
    pub cx: f64,
    pub cy: f64,
    /// Framing radius in rows at full size.
    pub fit: f64,
}

impl View {
    pub fn new(size: Size) -> Self {
        let (w, h) = (size.width(), art_rows(size).max(1));
        Self {
            // Cell centres are integers: columns [0, w) centre on (w − 1)/2, rows likewise.
            cx: (f64::from(w) - 1.0) / 2.0,
            cy: (f64::from(h) - 1.0) / 2.0,
            fit: draw::fit_radius(Size::new(w, h), FILL),
        }
    }
}

/// One projected atom.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Proj {
    /// Centre in cells (column, row).
    pub x: f64,
    pub y: f64,
    /// Camera-space depth in Å, larger = nearer.
    pub depth: f64,
    /// Projected ball radius, rows.
    pub r: f64,
    /// Depth cue `ν ∈ [0, 1]`, 1 = nearest ([`nu`]).
    pub nu: f64,
}

impl Proj {
    /// Centre in pixel coordinates: pixel `(i, j)` spans `[i, i + 1) × [j, j + 1)`, and
    /// cell `(c, r)` holds pixel column `c` and rows `2r, 2r + 1`.
    fn px(&self) -> (f64, f64) {
        (self.x + 0.5, 2.0 * self.y + 1.0)
    }
}

/// Depth cue `ν = (depth + r_eff)/(2·r_eff)`, clamped to `[0, 1]`.
pub(crate) fn nu(depth: f64, r_eff: f64) -> f64 {
    ((depth + r_eff) / (2.0 * r_eff)).clamp(0.0, 1.0)
}

/// Depth cue as a level before shading (MOL-A3: strictly increasing in depth): far atoms
/// recede to ≈ 40 % of the near ones.
pub(crate) fn depth_level(nu: f64) -> f64 {
    100.0 + 155.0 * nu
}

/// A full-size level `v` at gallery scale `s`: dimmed by `s²`, not `s`, so while a
/// molecule grows out of (or shrinks into) its centroid the converging atoms stay under
/// ≈ 25 % brightness until half size and read as a spark, not a solid knot (MOL-A5).
/// Hold frames (`s = 1`) are unchanged.
fn growth_level(s: f64, v: f64) -> f64 {
    s * s * v
}

/// Bonds are drawn only from this gallery scale up: below it the atoms are still
/// crowded about the centroid and sticks between them only thicken the clump (MOL-A5).
const STICKS_FROM: f64 = 0.5;

/// The camera for a pose and model.
pub(crate) fn camera(pose: &Pose, r_eff: f64) -> Camera {
    Camera {
        yaw: pose.yaw,
        pitch: pose.pitch,
        distance: EYE * r_eff,
    }
}

/// Rows per Å at gallery scale `s`.
pub(crate) fn scale(view: &View, model: &Model, s: f64) -> f64 {
    s * view.fit / (MAX_MAGNIFY * model.r_eff)
}

/// One frame's projection: the camera's projector (two `sin_cos` pairs per frame, not
/// per atom), the view centre, the scale and the model's framing radius.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lens {
    proj: Projector,
    centre: (f64, f64),
    /// Rows per Å ([`scale`]).
    k: f64,
    r_eff: f64,
}

impl Lens {
    pub fn new(cam: &Camera, view: &View, model: &Model, s: f64) -> Self {
        Self {
            proj: cam.projector(),
            centre: (view.cx, view.cy),
            k: scale(view, model, s),
            r_eff: model.r_eff,
        }
    }

    /// Project one atom; `None` only for a degenerate view.
    pub fn project(&self, atom: &Atom) -> Option<Proj> {
        let (x, y, depth) = self.proj.project(atom.pos, self.centre, self.k)?;
        let magnify = self.proj.magnification(depth);
        Some(Proj {
            x,
            y,
            depth,
            r: ball_radius(atom.el) * self.k * magnify,
            nu: nu(depth, self.r_eff),
        })
    }
}

/// Project every atom of `model` at `pose` into `out` (MOL-A3). Every atom lies within
/// `r_eff` of the centre and the eye is `5·r_eff` away, so projection never fails; the
/// fallback only avoids a panic.
pub(crate) fn project_all(model: &Model, view: &View, pose: &Pose, out: &mut [Proj]) {
    let lens = Lens::new(&camera(pose, model.r_eff), view, model, pose.s);
    for (p, atom) in out.iter_mut().zip(&model.atoms) {
        *p = lens.project(atom).unwrap_or_default();
    }
}

// ---------------------------------------------------------------------------------------
// Painting: a z-buffered half-block raster (MOL-A4, MOL-A7).
// ---------------------------------------------------------------------------------------

/// Stick radius, Å, and each strand of a double bond: two thinner sticks this far either
/// side of the axis, so the bond order reads (MOL-A7).
const STICK: f64 = 0.10;
const DOUBLE_R: f64 = 0.6 * STICK;
const DOUBLE_OFF: f64 = 1.25 * STICK;
/// Strands split only once a strand is this many pixels in radius; thinner, they merge
/// into one stick anyway.
const SPLIT_MIN_PX: f64 = 0.6;
/// Aromatic bonds: a solid stick plus a dashed thin strand on the ring's side, over the
/// middle of the bond, `DASHES` dashes long (the 1.5 order).
const DASH_FROM: f64 = 0.2;
const DASHES: f64 = 3.0;
/// [`Raster::owner`] of a stick pixel.
const STICK_OWNER: u8 = u8::MAX;
/// Sticks shade a little darker than balls, so balls read in front.
const STICK_LEVEL: f64 = 0.8;
/// The narrowest stick drawn, pixels: thinner ones would break into dots.
const MIN_STICK_PX: f64 = 0.55;
/// Light from the upper left, in front: `normalise(−0.45, 0.55, 0.70)`.
const LIGHT: [f64; 3] = [
    -0.451_129_236_405_377,
    0.551_380_177_828_794,
    0.701_756_589_963_920,
];
/// Blinn–Phong half-vector `normalise(L + V)` for a viewer on +z: the highlight sits
/// ≈ 0.39 of the radius up-left of centre, where a lit physical model shows it.
const HALF: [f64; 3] = [
    -0.244_532_875_621_478,
    0.298_873_514_648_473,
    0.922_430_645_079_596,
];
/// Unlit surface keeps this much light, so a ball's shadow side still reads.
const AMBIENT: f64 = 0.3;
/// Surface whose `n·H` exceeds this is the specular highlight.
const SPECULAR: f64 = 0.95;
/// Carbon is CPK grey: the white of hydrogen at this fraction.
const CARBON: f64 = 0.6;

/// Lambert intensity and specularity of a unit-sphere point seen at screen offset
/// `(nx, ny)` of its radius (screen y down): `I = 0.3 + 0.7·max(0, n·L)` and whether
/// `n·H > 0.95` (Blinn–Phong).
pub(crate) fn shade(nx: f64, ny: f64) -> (f64, bool) {
    let n = [nx, -ny, (1.0 - nx * nx - ny * ny).max(0.0).sqrt()];
    let ndl = dot(n, LIGHT);
    (
        AMBIENT + (1.0 - AMBIENT) * ndl.max(0.0),
        dot(n, HALF) > SPECULAR,
    )
}

/// The ink of element `el` at brightness `v ∈ [0, 1]` (CPK, false colour where it must
/// be, ADR-0015): H white and C grey (`Text`, which every theme keeps neutral), N in the
/// Ice ramp, O in the H-alpha ramp.
pub(crate) fn element_ink(el: El, v: f64) -> Ink {
    match el {
        El::H => Ink::new(Role::Text, level(255.0 * v)),
        El::C => Ink::new(Role::Text, level(255.0 * CARBON * v)),
        El::N => ink::ramp(Ramp::Ice, v),
        El::O => ink::ramp(Ramp::HAlpha, v),
    }
}

/// A half-block picture with a depth per pixel: nearer surfaces win, whatever order
/// they are drawn in (MOL-A4).
#[derive(Debug, Default)]
pub(crate) struct Raster {
    pixels: Pixels,
    /// Surface depth (Å, larger = nearer) per pixel; `−∞` where nothing is drawn.
    z: Vec<f32>,
    /// What each pixel shows: atom `i` as `i + 1`, a bond as [`STICK_OWNER`], 0 nothing.
    owner: Vec<u8>,
    w: i32,
    h: i32,
}

impl Raster {
    fn resize(&mut self, cells: Size) {
        self.pixels.resize(cells);
        let px = self.pixels.pixel_size();
        (self.w, self.h) = (i32::from(px.width()), i32::from(px.height()));
        self.z.clear();
        self.z.resize(px.area(), f32::NEG_INFINITY);
        self.owner.clear();
        self.owner.resize(px.area(), 0);
    }

    /// What pixel `(x, y)` shows ([`Raster::owner`]); 0 off the raster.
    fn owner(&self, x: i32, y: i32) -> u8 {
        if (0..self.w).contains(&x) && (0..self.h).contains(&y) {
            self.owner[(y * self.w + x) as usize]
        } else {
            0
        }
    }

    fn clear(&mut self) {
        self.pixels.clear();
        self.z.fill(f32::NEG_INFINITY);
        self.owner.fill(0);
    }

    /// Paint pixel `(x, y)` at depth `z` unless something nearer is already there.
    #[inline]
    fn plot(&mut self, x: i32, y: i32, z: f64, ink: Ink, owner: u8) {
        if (0..self.w).contains(&x) && (0..self.h).contains(&y) {
            let i = (y * self.w + x) as usize;
            if z as f32 > self.z[i] {
                self.z[i] = z as f32;
                self.owner[i] = owner;
                self.pixels.set(x, y, ink);
            }
        }
    }

    /// A shaded ball: Lambert body, Blinn–Phong highlight at `255·s²`, depth-cued.
    fn ball(&mut self, el: El, owner: u8, p: &Proj, r_a: f64, s: f64) {
        let (cx, cy) = p.px();
        let r = (2.0 * p.r).max(0.5);
        let cue = growth_level(s, depth_level(p.nu)) / 255.0;
        for y in (cy - r).floor() as i32..(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..(cx + r).ceil() as i32 {
                let (nx, ny) = ((f64::from(x) + 0.5 - cx) / r, (f64::from(y) + 0.5 - cy) / r);
                let d2 = nx * nx + ny * ny;
                // Balls under a pixel wide still light their centre pixel.
                if d2 > 1.0 && !(x == cx.floor() as i32 && y == cy.floor() as i32) {
                    continue;
                }
                let (i, specular) = shade(nx.clamp(-1.0, 1.0), ny.clamp(-1.0, 1.0));
                let v = if specular { s * s } else { cue * i };
                let z = p.depth + r_a * (1.0 - d2).max(0.0).sqrt();
                self.plot(x, y, z, element_ink(el, v), owner);
            }
        }
    }

    /// A shaded stick `strand` between `a` and `b` at `k` rows per Å and gallery scale
    /// `s`, each half in its atom's colour.
    fn stick(
        &mut self,
        (a, ea): (&Proj, El),
        (b, eb): (&Proj, El),
        strand: Strand,
        k: f64,
        s: f64,
    ) {
        let Strand { rad, off, dashes } = strand;
        let ((ax, ay), (bx, by)) = (a.px(), b.px());
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-9 {
            return;
        }
        let len = len2.sqrt();
        // Screen normal of the axis, and the offset strand's axis.
        let (ux, uy) = (-dy / len, dx / len);
        let px_per_a = 2.0 * k;
        let (ax, ay) = (ax + ux * off * px_per_a, ay + uy * off * px_per_a);
        let r = (rad * px_per_a).max(MIN_STICK_PX);
        let (x0, x1) = (ax.min(ax + dx) - r, ax.max(ax + dx) + r);
        let (y0, y1) = (ay.min(ay + dy) - r, ay.max(ay + dy) + r);
        for y in y0.floor() as i32..y1.ceil() as i32 {
            for x in x0.floor() as i32..x1.ceil() as i32 {
                let (qx, qy) = (f64::from(x) + 0.5 - ax, f64::from(y) + 0.5 - ay);
                let t = ((qx * dx + qy * dy) / len2).clamp(0.0, 1.0);
                if dashes > 0.0 {
                    let u = (t - DASH_FROM) / (1.0 - 2.0 * DASH_FROM);
                    if !(0.0..1.0).contains(&u) || (u * (2.0 * dashes - 1.0)).fract() > 0.5 {
                        continue;
                    }
                }
                // Signed offset across the axis, and distance from the capsule's segment
                // (the axis, with round ends), both in radii.
                let across = (qx * ux + qy * uy) / r;
                let dist = (qx - t * dx).hypot(qy - t * dy) / r;
                if dist > 1.0 {
                    continue;
                }
                let nz = (1.0 - across * across).max(0.0).sqrt();
                let (i, _) = shade(across * ux, across * uy);
                let el = if t < 0.5 { ea } else { eb };
                let depth = a.depth + t * (b.depth - a.depth);
                let cue = growth_level(s, depth_level(nu_mix(a, b, t))) / 255.0;
                let ink = element_ink(el, STICK_LEVEL * cue * i);
                self.plot(x, y, depth + rad * nz, ink, STICK_OWNER);
            }
        }
    }
}

/// Depth (Å) of the front of ball `p` at `k` rows per Å: its centre plus its radius.
fn ball_top(p: &Proj, k: f64) -> f64 {
    p.depth + p.r / k.max(1e-12)
}

/// One strand of a bond: radius `rad` Å, shifted `off` Å across the axis on screen;
/// `dashes > 0` draws only the middle `1 − 2·DASH_FROM` of it, in that many dashes.
#[derive(Debug, Clone, Copy)]
struct Strand {
    rad: f64,
    off: f64,
    dashes: f64,
}

impl Strand {
    const SOLID: Strand = Strand {
        rad: STICK,
        off: 0.0,
        dashes: 0.0,
    };

    /// A thin strand `off` Å across the axis.
    const fn thin(off: f64, dashes: f64) -> Strand {
        Strand {
            rad: DOUBLE_R,
            off,
            dashes,
        }
    }
}

/// The depth cue along a bond, interpolated between its atoms'.
fn nu_mix(a: &Proj, b: &Proj, t: f64) -> f64 {
    a.nu + t * (b.nu - a.nu)
}

/// Paint `model` projected as `proj` at gallery scale `s` into `raster`: bonds (only from
/// half size up, MOL-A5), then balls; the z-buffer sorts them (MOL-A4). Double bonds are
/// two strands and aromatic bonds a stick plus a dashed strand on the ring side, whenever
/// a strand is wide enough to see (MOL-A7).
pub(crate) fn paint(raster: &mut Raster, model: &Model, proj: &[Proj], view: &View, s: f64) {
    raster.clear();
    let k = scale(view, model, s);
    if s >= STICKS_FROM {
        for bond in &model.bonds {
            let (ia, ib) = (usize::from(bond.a), usize::from(bond.b));
            let a = (&proj[ia], model.atoms[ia].el);
            let b = (&proj[ib], model.atoms[ib].el);
            let split = DOUBLE_R * 2.0 * k >= SPLIT_MIN_PX;
            match bond.order {
                Order::Double if split => {
                    for side in [-1.0, 1.0] {
                        raster.stick(a, b, Strand::thin(side * DOUBLE_OFF, 0.0), k, s);
                    }
                }
                Order::Aromatic if split => {
                    raster.stick(a, b, Strand::SOLID, k, s);
                    // The dashed strand goes on the side of the view centre, inside the
                    // ring for benzene in any orientation.
                    let ((ax, ay), (bx, by)) = (a.0.px(), b.0.px());
                    let (mx, my) = (
                        0.5 * (ax + bx) - (view.cx + 0.5),
                        0.5 * (ay + by) - (2.0 * view.cy + 1.0),
                    );
                    let side = if (ay - by) * mx + (bx - ax) * my > 0.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    let dashed = Strand::thin(side * 2.0 * DOUBLE_OFF, DASHES);
                    raster.stick(a, b, dashed, k, s);
                }
                _ => raster.stick(a, b, Strand::SOLID, k, s),
            }
        }
    }
    for (i, (p, atom)) in proj.iter().zip(&model.atoms).enumerate() {
        // At most MAX_ATOMS (60) atoms, so `i + 1` fits a u8 below STICK_OWNER.
        raster.ball(atom.el, i as u8 + 1, p, ball_top(p, k) - p.depth, s);
    }
}

// ---------------------------------------------------------------------------------------
// The scene.
// ---------------------------------------------------------------------------------------

const LABEL_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// ASCII body tiers by brightness (Unicode shows the half-block pixels themselves).
const ASCII_BODY: [char; 5] = [':', '+', '*', '#', '@'];
/// Pixels dimmer than this are left blank in ASCII, so a bond's shadow side and far
/// atoms do not smear.
const ASCII_MIN_LEVEL: u8 = 24;

#[derive(Debug)]
pub(crate) struct MoleculeViewer {
    models: Vec<Model>,
    timeline: Timeline,
    /// Scene time; starts at the first `hold`, so the first frame is a full molecule.
    t: f64,
    /// Seeded first molecule, drawn on the first resize only.
    start: Option<usize>,
    view: View,
    /// Whether the title shows each molecule's name at this size (`Form::Full`), so its
    /// readout can leave the name out; set in `resize`.
    named: [bool; MOLECULES],
    /// This frame's projection and picture, rebuilt in `update`/`resize`.
    proj: [Proj; MAX_ATOMS],
    raster: Raster,
    /// ASCII element symbols: the cell and char of every atom whose centre pixel it owns.
    symbols: [(i32, i32, char); MAX_ATOMS],
    n_symbols: usize,
    label_row: Option<i32>,
}

impl MoleculeViewer {
    pub fn new() -> Self {
        Self {
            models: gallery(),
            timeline: gallery_timeline(),
            t: GROW,
            start: None,
            view: View::default(),
            named: [false; MOLECULES],
            proj: [Proj::default(); MAX_ATOMS],
            raster: Raster::default(),
            symbols: [(0, 0, ' '); MAX_ATOMS],
            n_symbols: 0,
            label_row: None,
        }
    }

    pub fn pose(&self) -> Pose {
        pose(&self.timeline, self.start.unwrap_or(0), self.t)
    }

    /// Project, paint, and note which atoms show their symbol in ASCII (MOL-A6).
    fn rasterise(&mut self) {
        let pose = self.pose();
        let model = &self.models[pose.molecule];
        let n = model.atoms.len();
        project_all(model, &self.view, &pose, &mut self.proj[..n]);
        paint(&mut self.raster, model, &self.proj[..n], &self.view, pose.s);
        self.n_symbols = 0;
        for (i, (p, atom)) in self.proj[..n].iter().zip(&model.atoms).enumerate() {
            let (px, py) = p.px();
            let (x, y) = (px.floor() as i32, py.floor() as i32);
            if self.raster.owner(x, y) == i as u8 + 1 {
                self.symbols[self.n_symbols] = (x, y / 2, atom.el.symbol(p.nu >= 0.5));
                self.n_symbols += 1;
            }
        }
    }
}

impl Generator for MoleculeViewer {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.view = View::new(size);
        self.named =
            CAPTIONS.map(|c| Title::new(META.title, Some(c)).fit(size) == Some(Form::Full));
        self.label_row = (size.height() >= 6).then(|| i32::from(size.height()) - 1);
        self.raster.resize(Size::new(size.width(), art_rows(size)));
        if self.start.is_none() {
            self.start = Some(ctx.rng.random_range(0..MOLECULES));
        }
        self.rasterise();
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
        self.rasterise();
    }

    fn render(&self, canvas: &mut Canvas) {
        self.raster.pixels.blit(canvas, |_, _, top, bottom| {
            let ink = if top.level >= bottom.level {
                top
            } else {
                bottom
            };
            (ink.role != Role::Background && ink.level >= ASCII_MIN_LEVEL).then(|| {
                let tier = usize::from(ink.level) * ASCII_BODY.len() / 256;
                (ASCII_BODY[tier], ink)
            })
        });
        if canvas.mode() == GlyphMode::Ascii {
            for &(x, y, ch) in &self.symbols[..self.n_symbols] {
                if let Some(&cell) = canvas.get(x, y) {
                    canvas.put(x, y, Glyph::ascii(ch), cell.ink);
                }
            }
        }
        if let Some(row) = self.label_row {
            let molecule = self.pose().molecule;
            let room = title::readout_room(canvas.size(), &Title::of(self));
            let label = self.models[molecule].label(room, self.named[molecule]);
            if !label.is_empty() {
                draw::label_right(canvas, row, label, LABEL_INK);
            }
        }
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.pose().molecule])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciart_core::geom::v3::{cross, unit};

    /// Cordero et al. (2008) covalent radii, Å.
    fn covalent(el: El) -> f64 {
        match el {
            El::H => 0.31,
            El::C => 0.76,
            El::N => 0.71,
            El::O => 0.66,
        }
    }

    fn valence(el: El) -> f64 {
        match el {
            El::H => 1.0,
            El::C => 4.0,
            El::N => 3.0,
            El::O => 2.0,
        }
    }

    fn bond_order(o: Order) -> f64 {
        match o {
            Order::Single => 1.0,
            Order::Double => 2.0,
            Order::Aromatic => 1.5,
        }
    }

    /// Hill formula: C first, then H, then the rest alphabetically; without carbon,
    /// everything alphabetically.
    fn hill_formula(atoms: &[Atom]) -> String {
        let count = |e: El| atoms.iter().filter(|a| a.el == e).count();
        let has_c = count(El::C) > 0;
        let order: &[El] = if has_c {
            &[El::C, El::H, El::N, El::O]
        } else {
            &[El::H, El::N, El::O]
        };
        let mut s = String::new();
        for &e in order {
            match count(e) {
                0 => {}
                1 => s.push(e.symbol(true)),
                n => s.push_str(&format!("{}{n}", e.symbol(true))),
            }
        }
        s
    }

    fn molar_mass(atoms: &[Atom]) -> f64 {
        atoms.iter().map(|a| mass(a.el)).sum()
    }

    fn bonded(m: &Model, i: usize, j: usize) -> bool {
        m.bonds.iter().any(|b| {
            let (a, b) = (usize::from(b.a), usize::from(b.b));
            (a, b) == (i, j) || (a, b) == (j, i)
        })
    }

    fn angle(a: [f64; 3], centre: [f64; 3], b: [f64; 3]) -> f64 {
        let (u, v) = (sub(a, centre), sub(b, centre));
        (dot(u, v) / (norm(u) * norm(v))).acos().to_degrees()
    }

    fn dihedral(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> f64 {
        let (b0, b1, b2) = (sub(a, b), sub(c, b), sub(d, c));
        let b1n = unit(b1);
        let v = sub(b0, b1n.map(|x| x * dot(b0, b1n)));
        let w = sub(b2, b1n.map(|x| x * dot(b2, b1n)));
        dot(cross(b1n, v), w).atan2(dot(v, w)).to_degrees()
    }

    fn by_name(name: &str) -> Model {
        gallery()
            .into_iter()
            .find(|m| m.labels[0].starts_with(name))
            .unwrap()
    }

    #[test]
    fn mol_a1_tables_are_chemically_consistent() {
        let expect = [
            ("H2O", 18.015),
            ("CH4", 16.043),
            ("C2H6O", 46.069),
            ("C6H6", 78.114),
            ("C8H10N4O2", 194.194),
            ("C60", 720.66),
        ];
        for (m, (formula, mass)) in gallery().iter().zip(expect) {
            let label = &m.labels[0];
            // Connectivity: bonded pairs in the covalent window, non-bonded well outside.
            for i in 0..m.atoms.len() {
                for j in i + 1..m.atoms.len() {
                    let (a, b) = (m.atoms[i], m.atoms[j]);
                    let ratio = dist(a.pos, b.pos) / (covalent(a.el) + covalent(b.el));
                    if bonded(m, i, j) {
                        assert!(
                            (0.80..=1.12).contains(&ratio),
                            "{label}: bond {i}-{j} {ratio}"
                        );
                    } else {
                        assert!(ratio >= 1.35, "{label}: {i}..{j} too close ({ratio})");
                    }
                }
            }
            for (i, a) in m.atoms.iter().enumerate() {
                let sum: f64 = m
                    .bonds
                    .iter()
                    .filter(|b| usize::from(b.a) == i || usize::from(b.b) == i)
                    .map(|b| bond_order(b.order))
                    .sum();
                assert_eq!(sum, valence(a.el), "{label}: valence of atom {i}");
            }
            assert_eq!(hill_formula(&m.atoms), formula);
            let parts: Vec<&str> = label.split("  ").collect();
            assert_eq!(parts[1], formula, "{label}");
            let derived = molar_mass(&m.atoms);
            assert!((derived - mass).abs() < 0.001, "{label}: {derived}");
            let shown: f64 = parts[2].trim_end_matches(" g/mol").parse().unwrap();
            assert!((shown - derived).abs() <= 0.005 + 1e-9, "{label}");
            for k in 0..3 {
                let com: f64 = m.atoms.iter().map(|a| super::mass(a.el) * a.pos[k]).sum();
                assert!((com / derived).abs() < 1e-6, "{label}: centre of mass");
            }
        }
    }

    #[test]
    fn mol_a2_reference_geometry() {
        let w = by_name("water");
        let p = |m: &Model, i: usize| m.atoms[i].pos;
        assert!((dist(p(&w, 0), p(&w, 1)) - 0.9572).abs() < 0.0005);
        assert!((dist(p(&w, 0), p(&w, 2)) - 0.9572).abs() < 0.0005);
        assert!((angle(p(&w, 1), p(&w, 0), p(&w, 2)) - 104.52).abs() < 0.05);

        let m = by_name("methane");
        for i in 1..5 {
            assert!((dist(p(&m, 0), p(&m, i)) - 1.087).abs() < 0.001);
            for j in i + 1..5 {
                assert!((angle(p(&m, i), p(&m, 0), p(&m, j)) - 109.47).abs() < 0.05);
            }
        }

        let e = by_name("ethanol");
        assert_eq!((e.atoms.len(), e.bonds.len()), (9, 8));
        assert!((dist(p(&e, 0), p(&e, 1)) - 1.512).abs() < 0.001);
        assert!((dist(p(&e, 1), p(&e, 2)) - 1.431).abs() < 0.001);
        assert!((dist(p(&e, 2), p(&e, 3)) - 0.971).abs() < 0.001);
        let hocc = dihedral(p(&e, 3), p(&e, 2), p(&e, 1), p(&e, 0));
        assert!((hocc.abs() - 180.0).abs() < 1.0, "H-O-C-C {hocc}");

        let b = by_name("benzene");
        assert_eq!(b.atoms.len(), 12);
        assert!(b.atoms.iter().all(|a| a.pos[2].abs() < 0.001));
        for bond in &b.bonds {
            let (x, y) = (b.atoms[usize::from(bond.a)], b.atoms[usize::from(bond.b)]);
            let want = if x.el == y.el { 1.397 } else { 1.084 };
            assert!((dist(x.pos, y.pos) - want).abs() < 0.001);
        }

        // Caffeine's heavy atoms lie in one plane: the best-fit plane through three
        // ring atoms is z = const in its own frame, so check distances to it.
        let c = by_name("caffeine");
        let heavy: Vec<[f64; 3]> = c
            .atoms
            .iter()
            .filter(|a| a.el != El::H)
            .map(|a| a.pos)
            .collect();
        assert_eq!(heavy.len(), 14);
        let normal = cross(sub(heavy[4], heavy[2]), sub(heavy[5], heavy[2]));
        let unit = unit(normal);
        for q in &heavy {
            assert!(dot(sub(*q, heavy[2]), unit).abs() < 0.05);
        }

        let f = by_name("buckminsterfullerene");
        assert_eq!((f.atoms.len(), f.bonds.len()), (60, 90));
        let lengths: Vec<f64> = f
            .bonds
            .iter()
            .map(|b| dist(f.atoms[usize::from(b.a)].pos, f.atoms[usize::from(b.b)].pos))
            .collect();
        assert_eq!(
            lengths
                .iter()
                .filter(|l| (*l - 1.401).abs() < 0.001)
                .count(),
            30
        );
        assert_eq!(
            lengths
                .iter()
                .filter(|l| (*l - 1.458).abs() < 0.001)
                .count(),
            60
        );
        // The double bonds are the short 6:6 ones, the single bonds the long 5:6 ones.
        for (b, len) in f.bonds.iter().zip(&lengths) {
            let want = match b.order {
                Order::Double => 1.401,
                Order::Single => 1.458,
                Order::Aromatic => panic!("C60 is drawn in its Kekulé structure"),
            };
            assert!((len - want).abs() < 0.001, "{b:?}: {len}");
        }
        for (i, a) in f.atoms.iter().enumerate() {
            assert!((norm(a.pos) - 3.562).abs() < 0.002);
            let mine: Vec<&Bond> = f
                .bonds
                .iter()
                .filter(|b| usize::from(b.a) == i || usize::from(b.b) == i)
                .collect();
            assert_eq!(mine.len(), 3);
            assert_eq!(mine.iter().filter(|b| b.order == Order::Double).count(), 1);
        }
        // The 5:6 bonds form exactly 12 disjoint pentagons: every atom has two single
        // bonds, so their graph is a union of cycles; walk each one.
        let singles: Vec<(usize, usize)> = f
            .bonds
            .iter()
            .filter(|b| b.order == Order::Single)
            .map(|b| (usize::from(b.a), usize::from(b.b)))
            .collect();
        let next = |i: usize, prev: usize| {
            singles
                .iter()
                .find_map(|&(a, b)| {
                    (a == i && b != prev)
                        .then_some(b)
                        .or((b == i && a != prev).then_some(a))
                })
                .unwrap()
        };
        let mut seen = [false; 60];
        let mut cycles = 0;
        for s in 0..60 {
            if seen[s] {
                continue;
            }
            let (mut prev, mut cur, mut len) = (usize::MAX, s, 0);
            loop {
                seen[cur] = true;
                len += 1;
                let n = next(cur, prev);
                (prev, cur) = (cur, n);
                if cur == s {
                    break;
                }
            }
            assert_eq!(len, 5);
            cycles += 1;
        }
        assert_eq!(cycles, 12);
    }

    fn projected(m: &Model, view: &View, pose: &Pose) -> Vec<Proj> {
        let mut p = vec![Proj::default(); m.atoms.len()];
        project_all(m, view, pose, &mut p);
        p
    }

    #[test]
    fn mol_a3_motion_is_rigid_and_fits_the_frame() {
        let tl = gallery_timeline();
        let models = gallery();
        for size in [
            Size::new(80, 24),
            Size::new(200, 50),
            Size::new(40, 12),
            Size::new(120, 36),
            Size::new(60, 20),
            Size::new(100, 30),
            Size::new(30, 10),
            Size::new(160, 40),
        ] {
            let view = View::new(size);
            let (w, h) = (f64::from(size.width()), f64::from(art_rows(size)));
            for k in 0..500 {
                // Hold times only, spread over a 60 s cycle.
                let slot = k % 6;
                let t = f64::from(slot) * SLOT + GROW + HOLD * (f64::from(k) + 0.5) / 500.0;
                let pose = pose(&tl, 0, t);
                assert_eq!(pose.s, 1.0);
                let m = &models[pose.molecule];
                // Rigid: an orthographic camera at scale 1 preserves every distance.
                let ortho = Camera {
                    distance: f64::INFINITY,
                    ..camera(&pose, m.r_eff)
                };
                let rotated: Vec<[f64; 3]> = m
                    .atoms
                    .iter()
                    .map(|a| {
                        let (x, y, z) = ortho.project(a.pos, (0.0, 0.0), 1.0).unwrap();
                        [x / sciart_core::geom::ASPECT, -y, z]
                    })
                    .collect();
                for i in 0..m.atoms.len() {
                    for j in i + 1..m.atoms.len() {
                        let d0 = dist(m.atoms[i].pos, m.atoms[j].pos);
                        assert!((dist(rotated[i], rotated[j]) - d0).abs() < 1e-9);
                    }
                }
                // Every ball's pixel disc lies inside the art area (above the title row).
                for p in projected(m, &view, &pose) {
                    let ((x, y), r) = (p.px(), 2.0 * p.r);
                    assert!(x - r >= 0.0 && x + r <= w, "{size:?} x {x} r {r}");
                    assert!(y - r >= 0.0 && y + r <= 2.0 * h, "{size:?} y {y} r {r}");
                }
            }
        }
    }

    #[test]
    fn mol_a3_every_molecule_reaches_the_framing_circle() {
        // The scale allows only the true perspective bound plus slack, so even water
        // and ethanol, framed by their bounding sphere, reach ≥ 85 % of the framing
        // radius at some point of their hold turn.
        let (tl, models) = (gallery_timeline(), gallery());
        for size in [Size::new(80, 24), Size::new(200, 50)] {
            let view = View::new(size);
            for (slot, m) in models.iter().enumerate() {
                let mut reach: f64 = 0.0;
                for k in 0..200 {
                    let t = f64::from(slot as u32) * SLOT + GROW + HOLD * f64::from(k) / 200.0;
                    for p in projected(m, &view, &pose(&tl, 0, t)) {
                        let d = ((p.x - view.cx) / sciart_core::geom::ASPECT).hypot(p.y - view.cy);
                        reach = reach.max(d + p.r);
                    }
                }
                let fill = reach / view.fit;
                assert!(fill >= 0.85, "{size:?} {}: {fill:.2}", m.labels[2]);
            }
        }
    }

    #[test]
    fn mol_a3_depth_cue_and_ball_radius_increase_with_depth() {
        let (view, m) = (View::new(Size::new(200, 50)), by_name("water"));
        let cam = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: EYE * m.r_eff,
        };
        let lens = Lens::new(&cam, &view, &m, 1.0);
        let mut last = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for k in 0..=40 {
            let z = -m.r_eff + 2.0 * m.r_eff * f64::from(k) / 40.0;
            let atom = at(El::O, 0.0, 0.0, z);
            let p = lens.project(&atom).unwrap();
            let cue = depth_level(p.nu);
            assert!(cue > last.0 && p.r > last.1, "z = {z}");
            last = (cue, p.r);
        }
    }

    /// A face-on pose (yaw 0, pitch 0) at full size.
    const FACE_ON: Pose = Pose {
        molecule: 0,
        s: 1.0,
        yaw: 0.0,
        pitch: 0.0,
    };

    fn painted(m: &Model, size: Size, pose: &Pose) -> (Raster, Vec<Proj>) {
        let view = View::new(size);
        let proj = projected(m, &view, pose);
        let mut r = Raster::default();
        r.resize(Size::new(size.width(), art_rows(size)));
        paint(&mut r, m, &proj, &view, pose.s);
        (r, proj)
    }

    #[test]
    fn mol_a4_the_nearer_surface_wins_every_pixel() {
        // An oxygen and a nearer nitrogen whose balls overlap on screen, bonded.
        let atoms = vec![at(El::O, -0.15, 0.0, -0.5), at(El::N, 0.15, 0.0, 0.5)];
        let m = Model::new("pair", "NO", "0", atoms, vec![bd(0, 1, Order::Single)]);
        let size = Size::new(120, 40);
        let (r, proj) = painted(&m, size, &FACE_ON);
        let (far, near) = (proj[0], proj[1]);
        assert!(near.depth > far.depth);
        let inside = |p: &Proj, x: i32, y: i32| {
            let (cx, cy) = p.px();
            (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) < 2.0 * p.r - 0.5
        };
        let mut shared = 0;
        for y in 0..r.h {
            for x in 0..r.w {
                if inside(&far, x, y) && inside(&near, x, y) {
                    shared += 1;
                    let ink = r.pixels.get(x, y).unwrap();
                    assert_eq!(ink.role, Role::Ramp(Ramp::Ice), "({x}, {y})");
                }
            }
        }
        assert!(shared > 20, "the balls overlap ({shared})");

        // In every gallery molecule and orientation, each visible ball centre shows its
        // own element: sticks never cover a ball.
        let size = Size::new(200, 50);
        let (mut shown, mut total) = (0, 0);
        for (k, m) in gallery().iter().enumerate() {
            for step in 0..12 {
                let pose = Pose {
                    molecule: k,
                    s: 1.0,
                    yaw: f64::from(step) * 0.53,
                    pitch: 0.3,
                };
                let (r, proj) = painted(m, size, &pose);
                for (i, (p, a)) in proj.iter().zip(&m.atoms).enumerate() {
                    let (x, y) = p.px();
                    let (x, y) = (x.floor() as i32, y.floor() as i32);
                    // A centre shows its own element, or something in front of it: a
                    // nearer ball, or a stick running toward the viewer.
                    match r.owner(x, y) {
                        o if o == i as u8 + 1 => {
                            let ink = r.pixels.get(x, y).unwrap();
                            assert_eq!(ink.role, element_ink(a.el, 0.5).role);
                            shown += 1;
                        }
                        STICK_OWNER => {}
                        o => {
                            let other = &proj[usize::from(o) - 1];
                            assert!(other.depth > p.depth, "{}: atom {i}", m.labels[0]);
                        }
                    }
                    total += 1;
                }
            }
        }
        // Most centres are in view: balls in front of their sticks.
        assert!(shown * 10 >= total * 8, "{shown} of {total}");
    }

    #[test]
    fn mol_a7_double_and_aromatic_bonds_show_their_order() {
        // Two carbons 1.4 Å apart along x, face-on: scan the column through the bond's
        // middle for lit runs. A single bond is one run, a double bond two, and an
        // aromatic bond a solid run plus a dashed strand that is lit only in places.
        let size = Size::new(120, 40);
        let runs = |order: Order| {
            let atoms = vec![at(El::C, -0.7, 0.0, 0.0), at(El::C, 0.7, 0.0, 0.0)];
            let m = Model::new("pair", "C2", "0", atoms, vec![bd(0, 1, order)]);
            let (r, proj) = painted(&m, size, &FACE_ON);
            let x = ((proj[0].px().0 + proj[1].px().0) / 2.0).floor() as i32;
            let lit: Vec<bool> = (0..r.h)
                .map(|y| r.pixels.get(x, y).unwrap() != Ink::BLANK)
                .collect();
            let n = lit.windows(2).filter(|w| w[1] && !w[0]).count() + usize::from(lit[0]);
            (n, lit.iter().filter(|&&l| l).count())
        };
        assert_eq!(runs(Order::Single).0, 1);
        assert_eq!(runs(Order::Double).0, 2);
        let (aromatic, _) = runs(Order::Aromatic);
        assert!((1..=2).contains(&aromatic), "{aromatic}");
        // Palette (CPK): H and C neutral text greys, N Ice, O H-alpha.
        assert_eq!(element_ink(El::H, 1.0), Ink::new(Role::Text, 255));
        assert_eq!(element_ink(El::C, 1.0).role, Role::Text);
        assert!(element_ink(El::C, 1.0).level < 200);
        assert_eq!(element_ink(El::N, 0.5).role, Role::Ramp(Ramp::Ice));
        assert_eq!(element_ink(El::O, 0.5).role, Role::Ramp(Ramp::HAlpha));
    }

    #[test]
    fn shading_vectors_are_unit_and_the_highlight_is_blinn_phong() {
        let l = [-0.45f64, 0.55, 0.70];
        let len = norm(l);
        for k in 0..3 {
            assert!((LIGHT[k] - l[k] / len).abs() < 1e-14);
        }
        let h = [LIGHT[0], LIGHT[1], LIGHT[2] + 1.0];
        for k in 0..3 {
            assert!((HALF[k] - h[k] / norm(h)).abs() < 1e-14);
        }
        // Brightest diffuse cell at L's screen position; the highlight at H's.
        let (at_h, specular) = shade(HALF[0], -HALF[1]);
        assert!(specular && at_h > 0.9);
        assert!(
            !shade(LIGHT[0], -LIGHT[1]).1,
            "n·L's peak is not the highlight"
        );
        assert!(!shade(0.5, 0.5).1);
    }

    #[test]
    fn mol_a5_gallery_timeline() {
        let tl = gallery_timeline();
        let start = 3;
        let mut prev = pose(&tl, start, GROW);
        let mut t = GROW;
        let mut visits = vec![prev.molecule];
        for _ in 0..(60.0 / SIM_DT) as usize {
            t += SIM_DT;
            let p = pose(&tl, start, t);
            assert!((0.0..=1.0).contains(&p.s));
            assert!((p.s - prev.s).abs() <= 0.05, "t = {t}");
            let beat = tl.at(t);
            if beat.index == 1 {
                assert_eq!(p.s, 1.0);
            }
            if p.molecule != prev.molecule {
                // Only across a slot boundary, where s passes through 0.
                assert_eq!(tl.cycle(t), tl.cycle(t - SIM_DT) + 1);
                assert!(p.s < 0.05 && prev.s < 0.05);
                visits.push(p.molecule);
            }
            prev = p;
        }
        assert_eq!(visits, [3, 4, 5, 0, 1, 2, 3]);
        for k in 0..8 {
            assert_eq!(pose(&tl, start, SLOT * f64::from(k)).s, 0.0);
        }

        // The seeded start reaches every molecule. Six uniform draws cover all six only
        // 6!/6⁶ ≈ 1.5 % of the time, so this checks seeds 1..=32 (MOL-A5 wording
        // (noted: "seeds 1..=32 yield all six starting molecules").
        let size = Size::new(80, 24);
        let starts: std::collections::BTreeSet<usize> = (1..=32)
            .map(|seed| {
                let mut g = MoleculeViewer::new();
                g.resize(size, &mut Ctx::new(seed, size));
                // A later resize keeps the start.
                g.resize(Size::new(40, 12), &mut Ctx::new(seed + 100, size));
                g.pose().molecule
            })
            .collect();
        assert_eq!(starts.len(), 6, "{starts:?}");
    }

    #[test]
    fn gen_r12_caption_names_the_shown_molecule() {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(7, size);
        let mut g = MoleculeViewer::new();
        g.resize(size, &mut ctx);
        for _ in 0..MOLECULES * 2 {
            let m = g.pose().molecule;
            let caption = g.caption().unwrap();
            assert_eq!(caption, CAPTIONS[m]);
            assert!(g.models[m].labels[0].starts_with(caption), "{caption}");
            g.update(SLOT / 2.0, &mut ctx);
        }
    }

    #[test]
    fn mol_a5_growth_fades_in_as_s_squared_and_sticks_wait_for_half_size() {
        // Mid grow/shrink the atoms converge on the centroid: dimmed by s², with no
        // sticks below half size, the clump reads as a spark rather than a knot.
        let size = Size::new(80, 24);
        for (k, m) in gallery().iter().enumerate() {
            for s in [0.1, 0.3, 0.45, 0.55, 0.8, 1.0] {
                let pose = Pose {
                    molecule: k,
                    s,
                    yaw: 0.7,
                    pitch: 0.3,
                };
                let (r, _) = painted(m, size, &pose);
                // Nothing, specular highlights included, outshines 255·s².
                let top = (0..r.h)
                    .flat_map(|y| (0..r.w).map(move |x| (x, y)))
                    .map(|(x, y)| r.pixels.get(x, y).unwrap().level)
                    .max()
                    .unwrap_or(0);
                assert!(
                    f64::from(top) <= (255.0 * s * s).round(),
                    "{} s = {s}",
                    m.labels[0]
                );
                // Sticks: the bonded molecule paints more than its bare balls from half size.
                let bare = Model::new("bare", "", "0", m.atoms.clone(), Vec::new());
                let (b, _) = painted(&bare, size, &pose);
                let lit = |r: &Raster| r.z.iter().filter(|z| z.is_finite()).count();
                assert_eq!(lit(&r) > lit(&b), s >= 0.5, "{} s = {s}", m.labels[0]);
            }
        }
    }

    #[test]
    fn mol_a6_label_tier_is_the_longest_that_fits() {
        let m = by_name("caffeine");
        assert_eq!(m.label(80, false), "caffeine  C8H10N4O2  194.19 g/mol");
        assert_eq!(m.label(33, false), "caffeine  C8H10N4O2  194.19 g/mol");
        assert_eq!(m.label(32, false), "caffeine  C8H10N4O2");
        assert_eq!(m.label(18, false), "C8H10N4O2");
        assert_eq!(m.label(8, false), "");
        // The title's caption names the molecule: the label keeps formula and mass.
        assert_eq!(m.label(80, true), "C8H10N4O2  194.19 g/mol");
        assert_eq!(m.label(23, true), "C8H10N4O2  194.19 g/mol");
        assert_eq!(m.label(22, true), "C8H10N4O2");
        for named in [false, true] {
            for room in 0..60 {
                let l = m.label(room, named);
                assert!(l.chars().count() <= room);
                // Every longer tier (they come longest first) would have been clipped.
                for &i in tiers(named).iter().take_while(|&&i| m.labels[i] != l) {
                    assert!(m.labels[i].chars().count() > room);
                }
            }
        }
    }

    #[test]
    fn mol_a6_the_readout_is_right_aligned_and_leaves_out_the_name_the_title_shows() {
        let size = Size::new(80, 24);
        let (mut g, mut ctx) = (MoleculeViewer::new(), Ctx::new(3, size));
        g.resize(size, &mut ctx);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        for _ in 0..120 {
            g.update(0.25, &mut ctx);
            canvas.clear();
            g.render(&mut canvas);
            let caption = g.caption().unwrap_or_default();
            let lines = canvas.to_lines();
            let last = &lines[23];
            assert!(
                !caption.is_empty() && !last.contains(caption),
                "{caption}: {last}"
            );
            assert!(last.ends_with(" g/mol "), "{last}");
            // The rest of the canvas is art: no text at the top any more.
            assert!(!lines[0].contains("g/mol"));
        }
    }

    #[test]
    fn mol_a6_ascii_frames_show_element_symbols() {
        let size = Size::new(80, 24);
        let (mut g, mut ctx) = (MoleculeViewer::new(), Ctx::new(1, size));
        g.resize(size, &mut ctx);
        let mut canvas = Canvas::new(size, GlyphMode::Ascii);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..(60.0 / 0.5) as usize {
            g.update(0.5, &mut ctx);
            canvas.clear();
            g.render(&mut canvas);
            let lines = canvas.to_lines();
            assert!(lines.iter().all(|l| l.is_ascii()));
            let pose = g.pose();
            if pose.s < 1.0 {
                continue;
            }
            // Every atom that owns its centre pixel shows its symbol, upper-case when near.
            for &(x, y, ch) in &g.symbols[..g.n_symbols] {
                assert_eq!(canvas.get(x, y).unwrap().ch, ch);
                seen.insert(ch);
            }
            assert!(g.n_symbols > 0);
        }
        for ch in ['H', 'C', 'N', 'O', 'h', 'c'] {
            assert!(seen.contains(&ch), "{ch} never shown: {seen:?}");
        }
    }
}
