//! Tumbling ball-and-stick models of real molecules, from water to buckminsterfullerene
//! (SPEC-002 `molecule_viewer`).
//!
//! Six molecules take turns on a dark bench: water, methane, ethanol, benzene, caffeine
//! and C60. Each grows out of a point, turns for 8.8 s in a lit perspective view, and
//! collapses back before the next one grows. Everything is a closed-form function of
//! scene time; `update` only advances the clock and `render` projects, depth-sorts and
//! paints at most 150 atoms and bonds from stack arrays.

use std::f64::consts::TAU;

use rand::Rng;
use sciart_core::draw;
use sciart_core::ease;
use sciart_core::geom::v3::{dot, norm, sub};
use sciart_core::geom::{self, ASPECT};
use sciart_core::ink::level;
use sciart_core::prelude::*;

use El::{C, H, N, O};
use Order::{Aromatic, Double, Single};

pub static META: Meta = Meta {
    name: "molecule_viewer",
    category: Category::Chemistry,
    description: "Tumbling ball-and-stick models of real molecules, from water to buckminsterfullerene",
    title: "Molecule · atoms share electrons",
    captions: &[],
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

    /// CPK colours as roles: H white, C grey, N blue, O red.
    fn role(self) -> Role {
        match self {
            El::H => Role::Text,
            El::C => Role::Structure,
            El::N => Role::Cold,
            El::O => Role::Hot,
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
    name: "water",
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
    name: "methane",
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
    name: "ethanol",
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
    name: "benzene",
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
    name: "caffeine",
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
    /// Label tiers, longest first: full, without the mass, the formula alone (MOL-A6).
    pub labels: [String; 3],
}

impl Model {
    fn new(
        name: &str,
        formula: &str,
        mass_label: &str,
        atoms: Vec<Atom>,
        bonds: Vec<Bond>,
    ) -> Self {
        // Build-time guard for `Frame`'s stack arrays; never reached on the frame path.
        assert!(
            atoms.len() <= MAX_ATOMS && atoms.len() + bonds.len() <= MAX_ITEMS,
            "{name}: raise MAX_ATOMS/MAX_ITEMS"
        );
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

    /// The longest label tier that fits in `room` columns, or `""` (MOL-A6).
    pub fn label(&self, room: usize) -> &str {
        self.labels
            .iter()
            .find(|l| l.chars().count() <= room)
            .map_or("", String::as_str)
    }
}

/// The gallery, in viewing order (SPEC-002).
pub(crate) fn gallery() -> Vec<Model> {
    let (atoms, bonds) = c60(R56, R66);
    let mut models: Vec<Model> = [&WATER, &METHANE, &ETHANOL, &BENZENE, &CAFFEINE]
        .into_iter()
        .map(Model::from_table)
        .collect();
    models.push(Model::new(
        "buckminsterfullerene",
        "C60",
        "720.66",
        atoms,
        bonds,
    ));
    models
}

const MOLECULES: usize = 6;
/// C60: 60 atoms + 90 bonds.
const MAX_ATOMS: usize = C60_ATOMS;
const MAX_ITEMS: usize = 150;

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
/// floored bodies and rounded cells (MOL-A3 checks the fit at eight sizes). The looser
/// on-axis bound `EYE/(EYE − 1)` = 1.25 left water and ethanol at ≈ 75 % of the frame.
/// The body classes ([`BODY_MIN`], [`COMPACT_MAX`]) are tuned at this scale.
const MAX_MAGNIFY: f64 = 1.05;
/// Fraction of the drawing area the framing circle fills.
const FILL: f64 = 0.86;

/// Screen framing for one size: rows `[1, h)` below the label row.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct View {
    pub cx: f64,
    pub cy: f64,
    /// Framing radius in rows at full size.
    pub fit: f64,
}

impl View {
    pub fn new(size: Size) -> Self {
        let (w, h) = (size.width(), size.height());
        Self {
            // Cell centres are integers: columns [0, w) centre on (w − 1)/2, rows [1, h)
            // on h/2.
            cx: (f64::from(w) - 1.0) / 2.0,
            cy: f64::from(h) / 2.0,
            fit: draw::fit_radius(Size::new(w, h.saturating_sub(1).max(1)), FILL),
        }
    }
}

/// One projected atom.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Proj {
    pub x: f64,
    pub y: f64,
    /// Camera-space depth in Å, larger = nearer.
    pub depth: f64,
    /// Projected ball radius, rows.
    pub r: f64,
    /// Depth-cued level `s²·(100 + 155·ν)` before shading ([`growth_level`]).
    pub level: f64,
    /// `ν ≥ 0.5`: the near half.
    pub near: bool,
    /// How the atom is drawn; fixed per molecule and size.
    pub body: Body,
    /// Radius `draw::disc` paints the ball with ([`footprint`]).
    pub foot: f64,
}

/// How an atom is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Body {
    /// A single dot glyph (`●`/`•`).
    #[default]
    Dot,
    /// A compact 3-cell shaded ball on its centre row.
    Compact,
    /// A shaded `draw::disc` ball.
    Round,
}

/// Which atoms get shaded bodies, chosen once per molecule and size (MOL-A6) so that no
/// atom flips between a ball and a dot as it turns, grows or shrinks: heavy atoms when
/// the largest unmagnified heavy ball is at least [`BODY_MIN`] rows at full size, and
/// hydrogens when their own ball is. Bodies of a class whose largest ball is under
/// [`COMPACT_MAX`] rows are compact 3-cell balls: at about one row `draw::disc` paints a
/// `+` of cells, which reads as a cross rather than a ball.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Bodies {
    pub heavy: Body,
    pub hydrogen: Body,
}

impl Bodies {
    pub fn new(view: &View, model: &Model) -> Self {
        let k = scale(view, model, 1.0);
        let class = |heavy: bool| {
            let biggest = model
                .atoms
                .iter()
                .filter(|a| (a.el != El::H) == heavy)
                .map(|a| ball_radius(a.el) * k)
                .fold(0.0, f64::max);
            match biggest {
                r if r < BODY_MIN => Body::Dot,
                r if r < COMPACT_MAX => Body::Compact,
                _ => Body::Round,
            }
        };
        Self {
            heavy: class(true),
            hydrogen: class(false),
        }
    }

    /// How `el` is drawn.
    fn of(self, el: El) -> Body {
        if el == El::H {
            self.hydrogen
        } else {
            self.heavy
        }
    }
}

/// Depth cue `ν = (depth + r_eff)/(2·r_eff)`, clamped to `[0, 1]`.
pub(crate) fn nu(depth: f64, r_eff: f64) -> f64 {
    ((depth + r_eff) / (2.0 * r_eff)).clamp(0.0, 1.0)
}

/// Unshaded level of an atom at depth cue `ν` (MOL-A3: strictly increasing in depth).
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
/// per atom), the view centre, the scale and the gallery scale.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lens {
    proj: Projector,
    centre: (f64, f64),
    /// Rows per Å ([`scale`]).
    k: f64,
    r_eff: f64,
    s: f64,
}

impl Lens {
    pub fn new(cam: &Camera, view: &View, model: &Model, s: f64) -> Self {
        Self {
            proj: cam.projector(),
            centre: (view.cx, view.cy),
            k: scale(view, model, s),
            r_eff: model.r_eff,
            s,
        }
    }

    /// Project one atom; `None` only for a degenerate view.
    pub fn project(&self, atom: &Atom, bodies: Bodies) -> Option<Proj> {
        let (x, y, depth) = self.proj.project(atom.pos, self.centre, self.k)?;
        let magnify = self.proj.magnification(depth);
        let n = nu(depth, self.r_eff);
        let r = ball_radius(atom.el) * self.k * magnify;
        let body = bodies.of(atom.el);
        Some(Proj {
            x,
            y,
            depth,
            r,
            level: growth_level(self.s, depth_level(n)),
            near: n >= 0.5,
            body,
            foot: footprint(body, r, self.s),
        })
    }
}

// ---------------------------------------------------------------------------------------
// Painting (MOL-A4, MOL-A6).
// ---------------------------------------------------------------------------------------

/// A molecule's heavy atoms (or its hydrogens) get shaded bodies when their largest
/// unmagnified ball is at least this many rows at full size ([`Bodies`]). At 80×24 this
/// lights ethanol (≈ 1.33 rows) and benzene (≈ 1.18) as well as water and methane, so
/// only caffeine and C60 (≈ 0.7–0.8 rows) are dot-and-stick, which their size justifies.
const BODY_MIN: f64 = 0.95;
/// Smallest body radius at full size, rows: far or small atoms (N and O beside C) keep a
/// few lit cells, so their colour still reads.
const BODY_FLOOR: f64 = 0.8 * BODY_MIN;
/// Bodies of a class whose largest unmagnified ball is under this many rows at full size
/// are compact ([`Bodies`]): at 80×24 benzene's (≈ 1.18), while water, methane
/// (≈ 2.3 rows) and ethanol (≈ 1.33) are round discs.
const COMPACT_MAX: f64 = 1.2;
/// `draw::disc` radius of a dot atom: under half a cell, so the centre cell only.
const DOT: f64 = 0.25;
/// Bonds at 0.8× their atom's level, so balls read in front of sticks.
const STICK: f64 = 0.8;
/// Double bonds split into two strokes only from this projected length (cells).
const DOUBLE_MIN_LEN: i32 = 6;
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
/// Unlit sphere cells keep this much light, so a ball's shadow side still reads.
const AMBIENT: f64 = 0.35;
/// Cells whose `n·H` exceeds this are the specular highlight.
const SPECULAR: f64 = 0.95;

/// Ball bodies by intensity, dim to bright. The dimmest ASCII tier is `:`, not `.`, so a
/// ball's shadow rim stays distinct from the dotted rear bonds ([`REAR_BOND`]) that
/// leave it.
const BODY: [Glyph; 5] = [
    Glyph::new('░', ':'),
    Glyph::new('░', '+'),
    Glyph::new('▒', '*'),
    Glyph::new('▓', '#'),
    Glyph::new('█', '@'),
];
const HIGHLIGHT: Glyph = Glyph::new('█', '@');
/// Every cell of a bond whose ends are both in the far half.
const REAR_BOND: Glyph = Glyph::new('·', '.');

/// Shading of a sphere cell from `draw::disc`'s `(nx, ny)` (screen y down): Lambert
/// intensity `I = 0.35 + 0.65·max(0, n·L)`, and whether the cell is specular
/// (Blinn–Phong, `n·H > 0.95`).
pub(crate) fn shade(nx: f64, ny: f64) -> (f64, bool) {
    let n = [nx, -ny, (1.0 - nx * nx - ny * ny).max(0.0).sqrt()];
    let ndl = dot(n, LIGHT);
    (
        AMBIENT + (1.0 - AMBIENT) * ndl.max(0.0),
        dot(n, HALF) > SPECULAR,
    )
}

/// Radius `draw::disc` is called with for a ball: its projected radius, floored at
/// `BODY_FLOOR·s`, when it gets a body; else [`DOT`], which paints the centre cell only.
/// While a molecule grows or shrinks a body keeps its class; under half a cell
/// `draw::disc` paints just the centre cell, with the body's own ramp glyph.
fn footprint(body: Body, r: f64, s: f64) -> f64 {
    match body {
        Body::Dot => DOT,
        Body::Compact | Body::Round => r.max(BODY_FLOOR * s),
    }
}

/// Whether a compact body paints its 3-cell ball: from half a row up, like `draw::disc`
/// leaves the centre cell alone below that.
fn lozenge(p: &Proj) -> bool {
    p.body == Body::Compact && p.foot >= 0.5
}

/// Whether painting ball `p` writes cell `(x, y)`: every cell
/// `draw::disc(p.x, p.y, p.foot)` paints (or, for a compact body, the cells either side
/// of the centre on its row), plus the rounded centre cell, which [`paint_atom`] always
/// writes (MOL-A4, MOL-A6).
pub(crate) fn covers(p: &Proj, x: i32, y: i32) -> bool {
    let (cx, cy) = (p.x.round(), p.y.round());
    if f64::from(x) == cx && f64::from(y) == cy {
        return true;
    }
    if lozenge(p) {
        return f64::from(y) == cy && (f64::from(x) - cx).abs() <= 1.0;
    }
    // Below half a row `disc` paints only the centre cell, already covered above.
    p.foot >= 0.5 && draw::disc_covers(p.x, p.y, p.foot, x, y)
}

fn paint_atom(canvas: &mut Canvas, el: El, p: &Proj, s: f64) {
    let role = el.role();
    let (cx, cy) = (p.x.round(), p.y.round());
    let letter = el.symbol(p.near);
    if p.body == Body::Dot {
        let dot = if el == El::H { '•' } else { '●' };
        canvas.put(
            cx as i32,
            cy as i32,
            Glyph::new(dot, letter),
            Ink::new(role, level(p.level)),
        );
        return;
    }
    let cell = |nx: f64, ny: f64| -> (Glyph, Ink) {
        let (i, specular) = shade(nx, ny);
        if specular {
            (HIGHLIGHT, Ink::new(role, level(growth_level(s, 255.0))))
        } else {
            let glyph = draw::ramp((i - AMBIENT) / (1.0 - AMBIENT), &BODY);
            (glyph, Ink::new(role, level(p.level * i)))
        }
    };
    if lozenge(p) {
        // A 3-cell ball on the centre row, shaded as a one-row sphere: `█▓░` lit from
        // the left like the round balls.
        for dx in [-1.0, 1.0] {
            let (glyph, ink) = cell((cx + dx - p.x) / ASPECT, 0.0);
            canvas.put((cx + dx) as i32, cy as i32, glyph, ink);
        }
    } else {
        draw::disc(canvas, p.x, p.y, p.foot, |_, nx, ny| Some(cell(nx, ny)));
    }
    // The centre cell keeps its body glyph in Unicode and shows the element in ASCII.
    let (nx, ny) = if lozenge(p) {
        ((cx - p.x) / ASPECT, 0.0)
    } else if p.foot < 0.5 {
        (0.0, 0.0)
    } else {
        ((cx - p.x) / (p.foot * ASPECT), (cy - p.y) / p.foot)
    };
    let (glyph, ink) = cell(nx, ny);
    canvas.put(cx as i32, cy as i32, Glyph::new(glyph.unicode, letter), ink);
}

/// One stroke from `a` to `b` shifted by `(ox, oy)` cells: split at its midpoint into
/// the two atoms' inks, skipping every cell either ball paints (MOL-A4). A `rear` bond
/// (both ends in the far half) is dotted, `·` (ASCII `.`), so a cage's near face reads
/// as `─│╱╲` rings over a faint dotted back lattice instead of tangling with it (MOL-A6).
fn stroke(
    canvas: &mut Canvas,
    (a, ia): (&Proj, Ink),
    (b, ib): (&Proj, Ink),
    (ox, oy): (i32, i32),
    rear: bool,
) {
    let (x0, y0) = (a.x.round() as i32 + ox, a.y.round() as i32 + oy);
    let (x1, y1) = (b.x.round() as i32 + ox, b.y.round() as i32 + oy);
    let len = (x1 - x0).abs().max((y1 - y0).abs());
    geom::stroked_line(x0, y0, x1, y1, |k, x, y, (dx, dy)| {
        if covers(a, x, y) || covers(b, x, y) {
            return;
        }
        let ink = if 2 * k <= len { ia } else { ib };
        let glyph = if rear {
            REAR_BOND
        } else {
            draw::step_glyph(dx, dy)
        };
        canvas.put(x, y, glyph, ink);
    });
}

fn paint_bond(canvas: &mut Canvas, model: &Model, bond: &Bond, proj: &[Proj]) {
    let (a, b) = (&proj[usize::from(bond.a)], &proj[usize::from(bond.b)]);
    let ink = |i: u8, p: &Proj| {
        let role = model.atoms[usize::from(i)].el.role();
        Ink::new(role, level(STICK * p.level))
    };
    let (ea, eb) = ((a, ink(bond.a, a)), (b, ink(bond.b, b)));
    let rear = !a.near && !b.near;
    let (dx, dy) = ((b.x - a.x).round() as i32, (b.y - a.y).round() as i32);
    let split = bond.order == Order::Double
        && dx.abs().max(dy.abs()) >= DOUBLE_MIN_LEN
        && a.body != Body::Dot
        && b.body != Body::Dot;
    if !split {
        stroke(canvas, ea, eb, (0, 0), rear);
        return;
    }
    // One cell either side of the axis, across the stroke.
    let offset = if f64::from(dx.abs()) >= f64::from(dy.abs()) * ASPECT {
        (0, 1)
    } else {
        (1, 0)
    };
    stroke(canvas, ea, eb, offset, rear);
    stroke(canvas, ea, eb, (-offset.0, -offset.1), rear);
}

/// One frame's projected atoms and painter's order.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub proj: [Proj; MAX_ATOMS],
    /// Items far → near: atoms `0..n_atoms`, then bonds.
    pub order: [u16; MAX_ITEMS],
    pub items: usize,
    pub s: f64,
}

/// Sort item indices far → near by `(depth, index)`: a deterministic order with no heap
/// use (MOL-A4).
pub(crate) fn painter_order(depths: &[f32], order: &mut [u16]) {
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u16;
    }
    order.sort_unstable_by(|&i, &j| {
        depths[usize::from(i)]
            .total_cmp(&depths[usize::from(j)])
            .then(i.cmp(&j))
    });
}

/// Project `model` at `pose`; `bodies` is its per-size class ([`Bodies::new`]).
pub(crate) fn frame(model: &Model, view: &View, pose: &Pose, bodies: Bodies) -> Frame {
    let lens = Lens::new(&camera(pose, model.r_eff), view, model, pose.s);
    let mut f = Frame {
        proj: [Proj::default(); MAX_ATOMS],
        order: [0; MAX_ITEMS],
        items: 0,
        s: pose.s,
    };
    let mut depths = [0f32; MAX_ITEMS];
    for (i, atom) in model.atoms.iter().enumerate() {
        // Unreachable: every atom lies within `r_eff` of the centre and the eye is
        // `5·r_eff` away, so `project` never fails. The fallback only avoids a panic; its
        // bonds would still be stroked toward the corner.
        f.proj[i] = lens.project(atom, bodies).unwrap_or(Proj {
            x: -1.0,
            y: -1.0,
            ..Proj::default()
        });
        depths[i] = f.proj[i].depth as f32;
    }
    let n = model.atoms.len();
    for (j, bond) in model.bonds.iter().enumerate() {
        let (a, b) = (f.proj[usize::from(bond.a)], f.proj[usize::from(bond.b)]);
        depths[n + j] = (0.5 * (a.depth + b.depth)) as f32;
    }
    f.items = n + model.bonds.len();
    painter_order(&depths[..f.items], &mut f.order[..f.items]);
    f
}

/// Paint the given items (indices into atoms, then bonds) in order.
pub(crate) fn paint(canvas: &mut Canvas, model: &Model, frame: &Frame, items: &[u16]) {
    let n = model.atoms.len();
    for &i in items {
        let i = usize::from(i);
        if i < n {
            paint_atom(canvas, model.atoms[i].el, &frame.proj[i], frame.s);
        } else if frame.s >= STICKS_FROM {
            paint_bond(canvas, model, &model.bonds[i - n], &frame.proj);
        }
    }
}

// ---------------------------------------------------------------------------------------
// The scene.
// ---------------------------------------------------------------------------------------

const LABEL_INK: Ink = Ink::new(Role::Text, 140);

#[derive(Debug)]
pub(crate) struct MoleculeViewer {
    models: Vec<Model>,
    timeline: Timeline,
    /// Scene time; starts at the first `hold`, so the first frame is a full molecule.
    t: f64,
    /// Seeded first molecule, drawn on the first resize only.
    start: Option<usize>,
    view: View,
    /// Each molecule's ball-or-dot classes at this size (MOL-A6), rebuilt in `resize`.
    bodies: [Bodies; MOLECULES],
}

impl MoleculeViewer {
    pub fn new() -> Self {
        Self {
            models: gallery(),
            timeline: gallery_timeline(),
            t: GROW,
            start: None,
            view: View::default(),
            bodies: [Bodies::default(); MOLECULES],
        }
    }

    pub fn pose(&self) -> Pose {
        pose(&self.timeline, self.start.unwrap_or(0), self.t)
    }
}

impl Generator for MoleculeViewer {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn resize(&mut self, size: Size, ctx: &mut Ctx) {
        self.view = View::new(size);
        self.bodies = std::array::from_fn(|i| Bodies::new(&self.view, &self.models[i]));
        if self.start.is_none() {
            self.start = Some(ctx.rng.random_range(0..MOLECULES));
        }
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        self.t += dt;
    }

    fn render(&self, canvas: &mut Canvas) {
        let pose = self.pose();
        let model = &self.models[pose.molecule];
        let frame = frame(model, &self.view, &pose, self.bodies[pose.molecule]);
        paint(canvas, model, &frame, &frame.order[..frame.items]);
        let room = usize::from(canvas.size().width()).saturating_sub(2);
        canvas.text(1, 0, model.label(room), LABEL_INK);
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
            let (w, h) = (i32::from(size.width()), i32::from(size.height()));
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
                        [x / ASPECT, -y, z]
                    })
                    .collect();
                for i in 0..m.atoms.len() {
                    for j in i + 1..m.atoms.len() {
                        let d0 = dist(m.atoms[i].pos, m.atoms[j].pos);
                        assert!((dist(rotated[i], rotated[j]) - d0).abs() < 1e-9);
                    }
                }
                // Every cell of every ball is on screen below the label row.
                let lens = Lens::new(&camera(&pose, m.r_eff), &view, m, 1.0);
                let bodies = Bodies::new(&view, m);
                for a in &m.atoms {
                    let p = lens.project(a, bodies).expect("atom projects");
                    let (x0, y0) = (p.x.round() as i32, p.y.round() as i32);
                    for y in y0 - 30..=y0 + 30 {
                        for x in x0 - 60..=x0 + 60 {
                            if covers(&p, x, y) {
                                assert!((0..w).contains(&x) && (1..h).contains(&y), "{size:?}");
                            }
                        }
                    }
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
                let bodies = Bodies::new(&view, m);
                let mut reach: f64 = 0.0;
                for k in 0..200 {
                    let t = f64::from(slot as u32) * SLOT + GROW + HOLD * f64::from(k) / 200.0;
                    let f = frame(m, &view, &pose(&tl, 0, t), bodies);
                    for p in &f.proj[..m.atoms.len()] {
                        let d = ((p.x - view.cx) / ASPECT).hypot(p.y - view.cy);
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
            let p = lens.project(&atom, Bodies::default()).unwrap();
            let cue = depth_level(nu(p.depth, m.r_eff));
            assert!(cue > last.0 && p.r > last.1, "z = {z}");
            last = (cue, p.r);
        }
    }

    fn caffeine_frame(size: Size) -> (Model, Frame) {
        let m = by_name("caffeine");
        let pose = Pose {
            molecule: 4,
            s: 1.0,
            yaw: 0.7,
            pitch: 0.4,
        };
        let view = View::new(size);
        let f = frame(&m, &view, &pose, Bodies::new(&view, &m));
        (m, f)
    }

    #[test]
    fn mol_a4_render_order_is_a_depth_sorted_permutation() {
        let (m, f) = caffeine_frame(Size::new(200, 50));
        let order = &f.order[..f.items];
        assert_eq!(f.items, m.atoms.len() + m.bonds.len());
        let mut seen = order.to_vec();
        seen.sort_unstable();
        assert!(seen.iter().enumerate().all(|(i, &o)| usize::from(o) == i));
        let n = m.atoms.len();
        let depth = |i: u16| -> f32 {
            let i = usize::from(i);
            if i < n {
                f.proj[i].depth as f32
            } else {
                let b = m.bonds[i - n];
                (0.5 * (f.proj[usize::from(b.a)].depth + f.proj[usize::from(b.b)].depth)) as f32
            }
        };
        for w in order.windows(2) {
            let (a, b) = (depth(w[0]), depth(w[1]));
            assert!(a < b || (a == b && w[0] < w[1]), "{w:?}");
        }
        // Ties go by index.
        let mut ties = [0u16; 4];
        painter_order(&[1.0, 0.0, 1.0, 0.0], &mut ties);
        assert_eq!(ties, [1, 3, 0, 2]);
    }

    #[test]
    fn mol_a4_nearer_atom_wins_shared_cells_and_bonds_skip_balls() {
        // An oxygen and a nearer nitrogen whose balls overlap on screen, bonded.
        let atoms = vec![at(El::O, -0.15, 0.0, -0.5), at(El::N, 0.15, 0.0, 0.5)];
        let m = Model::new("pair", "NO", "0", atoms, vec![bd(0, 1, Order::Single)]);
        let size = Size::new(120, 40);
        let pose = Pose {
            molecule: 0,
            s: 1.0,
            yaw: 0.0,
            pitch: 0.0,
        };
        let view = View::new(size);
        let f = frame(&m, &view, &pose, Bodies::new(&view, &m));
        let (far, near) = (f.proj[0], f.proj[1]);
        assert!(near.depth > far.depth);
        let mut canvas = Canvas::new(size, GlyphMode::Unicode);
        paint(&mut canvas, &m, &f, &f.order[..f.items]);
        let (far_disc, near_disc) = (disc_marks(size, &far), disc_marks(size, &near));
        let mut shared = 0;
        for y in 0..40 {
            for x in 0..120 {
                if marked(&far_disc, x, y) && marked(&near_disc, x, y) {
                    shared += 1;
                    assert_eq!(canvas.get(x, y).unwrap().ink.role, Role::Cold);
                }
            }
        }
        assert!(shared > 0, "the balls overlap");

        // Bonds never paint a cell either end ball paints, in any orientation; the
        // reference is `draw::disc` itself (or the 3-cell compact rule), not `covers`.
        let models = gallery();
        let size = Size::new(200, 50);
        for (k, m) in models.iter().enumerate() {
            for step in 0..24 {
                let pose = Pose {
                    molecule: k,
                    s: 1.0,
                    yaw: f64::from(step) * 0.27,
                    pitch: 0.3,
                };
                let view = View::new(size);
                let f = frame(m, &view, &pose, Bodies::new(&view, m));
                let n = m.atoms.len();
                for (j, bond) in m.bonds.iter().enumerate() {
                    let mut canvas = Canvas::new(size, GlyphMode::Ascii);
                    paint(&mut canvas, m, &f, &[(n + j) as u16]);
                    let (a, b) = (&f.proj[usize::from(bond.a)], &f.proj[usize::from(bond.b)]);
                    let (da, db) = (ball_marks(size, a), ball_marks(size, b));
                    for y in 0..50 {
                        for x in 0..200 {
                            if canvas.get(x, y).unwrap().ch != ' ' {
                                assert!(!marked(&da, x, y) && !marked(&db, x, y));
                            }
                        }
                    }
                }
            }
        }
    }

    const MARK: (Glyph, Ink) = (Glyph::ascii('#'), Ink::new(Role::Text, 255));

    /// The cells `draw::disc` paints for ball `p`, on a blank canvas.
    fn disc_marks(size: Size, p: &Proj) -> Canvas {
        let mut c = Canvas::new(size, GlyphMode::Ascii);
        draw::disc(&mut c, p.x, p.y, p.foot, |_, _, _| Some(MARK));
        c
    }

    /// The cells ball `p` paints: `draw::disc`'s, or for a compact body of at least
    /// half a row its centre cell and the cells either side.
    fn ball_marks(size: Size, p: &Proj) -> Canvas {
        if !(p.body == Body::Compact && p.foot >= 0.5) {
            return disc_marks(size, p);
        }
        let mut c = Canvas::new(size, GlyphMode::Ascii);
        let (x, y) = (p.x.round() as i32, p.y.round() as i32);
        for dx in -1..=1 {
            c.put(x + dx, y, MARK.0, MARK.1);
        }
        c
    }

    fn marked(c: &Canvas, x: i32, y: i32) -> bool {
        c.get(x, y).is_some_and(|c| c.ch == '#')
    }

    #[test]
    fn mol_a4_covers_matches_draw_disc() {
        let size = Size::new(60, 30);
        for body in [Body::Round, Body::Compact, Body::Dot] {
            for r in [0.3, 0.5, 0.96, 1.2, 2.7, 5.0] {
                for (i, j) in (0..16).map(|k| (k % 4, k / 4)) {
                    let p = Proj {
                        x: 30.0 + 0.25 * f64::from(i),
                        y: 15.0 + 0.25 * f64::from(j),
                        r,
                        body,
                        foot: footprint(body, r, 1.0),
                        ..Proj::default()
                    };
                    let disc = ball_marks(size, &p);
                    // What paint_atom writes: the disc plus its rounded centre cell.
                    let mut atom = Canvas::new(size, GlyphMode::Ascii);
                    paint_atom(&mut atom, El::C, &p, 1.0);
                    let centre = (p.x.round() as i32, p.y.round() as i32);
                    for y in 0..30 {
                        for x in 0..60 {
                            let want = marked(&disc, x, y) || (x, y) == centre;
                            assert_eq!(covers(&p, x, y), want, "{p:?} ({x}, {y})");
                            let painted = atom.get(x, y).unwrap().ch != ' ';
                            assert_eq!(painted, want, "{p:?} ({x}, {y})");
                        }
                    }
                }
            }
        }
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
    fn mol_a5_growth_fades_in_as_s_squared_and_sticks_wait_for_half_size() {
        // Mid grow/shrink the atoms converge on the centroid: dimmed by s², with no
        // sticks below half size, the clump reads as a spark rather than a knot.
        let size = Size::new(80, 24);
        let view = View::new(size);
        let models = gallery();
        for (k, m) in models.iter().enumerate() {
            let bodies = Bodies::new(&view, m);
            for s in [0.1, 0.3, 0.45, 0.55, 0.8, 1.0] {
                let pose = Pose {
                    molecule: k,
                    s,
                    yaw: 0.7,
                    pitch: 0.3,
                };
                let f = frame(m, &view, &pose, bodies);
                for (i, p) in f.proj[..m.atoms.len()].iter().enumerate() {
                    let want = s * s * depth_level(nu(p.depth, m.r_eff));
                    assert!(
                        (p.level - want).abs() < 1e-9,
                        "{} s = {s} atom {i}",
                        m.labels[0]
                    );
                }
                let mut c = Canvas::new(size, GlyphMode::Unicode);
                paint(&mut c, m, &f, &f.order[..f.items]);
                // Nothing, specular highlights included, outshines 255·s².
                let top = c
                    .cells()
                    .iter()
                    .map(|cell| cell.ink.level)
                    .max()
                    .unwrap_or(0);
                assert!(
                    f64::from(top) <= (255.0 * s * s).round(),
                    "{} s = {s}",
                    m.labels[0]
                );
                let n = m.atoms.len();
                let bonds: Vec<u16> = (n..f.items).map(|j| j as u16).collect();
                let mut sticks = Canvas::new(size, GlyphMode::Unicode);
                paint(&mut sticks, m, &f, &bonds);
                let drawn = sticks.cells().iter().any(|cell| cell.ch != ' ');
                assert_eq!(drawn, s >= 0.5, "{} s = {s}", m.labels[0]);
            }
        }
    }

    #[test]
    fn mol_a6_label_tier_is_the_longest_that_fits() {
        let m = by_name("caffeine");
        assert_eq!(m.label(80), "caffeine  C8H10N4O2  194.19 g/mol");
        assert_eq!(m.label(33), "caffeine  C8H10N4O2  194.19 g/mol");
        assert_eq!(m.label(32), "caffeine  C8H10N4O2");
        assert_eq!(m.label(18), "C8H10N4O2");
        assert_eq!(m.label(8), "");
        for room in 0..60 {
            let l = m.label(room);
            assert!(l.chars().count() <= room);
            // Every longer tier (they come longest first) would have been clipped.
            for tier in m.labels.iter().take_while(|x| x.as_str() != l) {
                assert!(tier.chars().count() > room);
            }
        }
    }

    /// Whether atom `i` of `f` paints as a single-cell dot (`●`/`•`) rather than a body.
    fn is_dot(m: &Model, f: &Frame, i: usize, size: Size) -> bool {
        let mut c = Canvas::new(size, GlyphMode::Unicode);
        paint(&mut c, m, f, &[i as u16]);
        let p = f.proj[i];
        let ch = c.get(p.x.round() as i32, p.y.round() as i32).map(|c| c.ch);
        matches!(ch, Some('●' | '•'))
    }

    #[test]
    fn mol_a6_ball_or_dot_is_fixed_per_molecule_and_size() {
        let tl = gallery_timeline();
        let models = gallery();
        for size in [Size::new(80, 24), Size::new(200, 50), Size::new(120, 36)] {
            let view = View::new(size);
            // Per slot: each atom's class, fixed at its first sample.
            let mut class: Vec<Vec<Option<bool>>> =
                models.iter().map(|m| vec![None; m.atoms.len()]).collect();
            for k in 0..500 {
                // Whole slots, grow and shrink included, over a 60 s cycle.
                let t = 60.0 * (f64::from(k) + 0.5) / 500.0;
                let pose = pose(&tl, 0, t);
                if pose.s < 0.02 {
                    continue;
                }
                let m = &models[pose.molecule];
                let f = frame(m, &view, &pose, Bodies::new(&view, m));
                let mut heavy = None;
                for (i, a) in m.atoms.iter().enumerate() {
                    let p = f.proj[i];
                    let on = (0.0..f64::from(size.width())).contains(&p.x.round())
                        && (0.0..f64::from(size.height())).contains(&p.y.round());
                    if !on {
                        continue;
                    }
                    let dot = is_dot(m, &f, i, size);
                    let first = *class[pose.molecule][i].get_or_insert(dot);
                    assert_eq!(dot, first, "{size:?} t = {t}: {} atom {i}", m.labels[0]);
                    // N and O keep their colour: all heavy atoms share one class.
                    if a.el != El::H {
                        assert_eq!(dot, *heavy.get_or_insert(dot), "{size:?} {}", m.labels[0]);
                    }
                }
            }
        }
    }

    #[test]
    fn mol_a6_small_molecules_are_lit_balls_at_80x24() {
        let view = View::new(Size::new(80, 24));
        let heavy: Vec<bool> = gallery()
            .iter()
            .map(|m| Bodies::new(&view, m).heavy != Body::Dot)
            .collect();
        // Water, methane, ethanol and benzene are lit; caffeine and C60 (≈ 0.7-row
        // balls) stay dot-and-stick.
        assert_eq!(heavy, [true, true, true, true, false, false]);
        // The scene caches the classes per size.
        let mut scene = MoleculeViewer::new();
        scene.resize(Size::new(80, 24), &mut Ctx::new(1, Size::new(80, 24)));
        for (m, b) in scene.models.iter().zip(scene.bodies) {
            assert_eq!(b, Bodies::new(&scene.view, m));
        }
        // Benzene keeps a visible C–C stick between its lit balls in every hold pose.
        let (m, tl) = (by_name("benzene"), gallery_timeline());
        let bodies = Bodies::new(&view, &m);
        for k in 0..40 {
            let t = GROW + HOLD * (f64::from(k) + 0.5) / 40.0;
            let pose = Pose {
                molecule: 3,
                ..pose(&tl, 3, t)
            };
            let f = frame(&m, &view, &pose, bodies);
            let n = m.atoms.len();
            let sticks = m
                .bonds
                .iter()
                .enumerate()
                .filter(|(_, b)| m.atoms[usize::from(b.a)].el == El::C)
                .filter(|(_, b)| m.atoms[usize::from(b.b)].el == El::C)
                .filter(|&(j, _)| {
                    let mut c = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
                    paint(&mut c, &m, &f, &[(n + j) as u16]);
                    c.to_lines().iter().any(|l| l.chars().any(|ch| ch != ' '))
                })
                .count();
            assert!(sticks >= 3, "t = {t}: {sticks} visible C–C sticks");
        }
    }

    #[test]
    fn mol_a6_small_bodies_are_compact_three_cell_balls() {
        let compact = |size: Size| -> Vec<bool> {
            let view = View::new(size);
            gallery()
                .iter()
                .map(|m| Bodies::new(&view, m).heavy == Body::Compact)
                .collect()
        };
        // At 80×24 benzene's ≈ 1.2-row balls; water, methane and ethanol are round discs,
        // caffeine and C60 dots. At 40×12 water's and methane's ≈ 1.1-row balls.
        assert_eq!(
            compact(Size::new(80, 24)),
            [false, false, false, true, false, false]
        );
        assert_eq!(
            compact(Size::new(40, 12)),
            [true, true, false, false, false, false]
        );
        let tl = gallery_timeline();
        for (size, name) in [
            (Size::new(80, 24), "benzene"),
            (Size::new(40, 12), "water"),
            (Size::new(40, 12), "methane"),
        ] {
            let view = View::new(size);
            let (w, h) = (i32::from(size.width()), i32::from(size.height()));
            let m = by_name(name);
            let bodies = Bodies::new(&view, &m);
            for k in 0..20 {
                let t = GROW + HOLD * (f64::from(k) + 0.5) / 20.0;
                let f = frame(&m, &view, &pose(&tl, 0, t), bodies);
                for (i, a) in m.atoms.iter().enumerate() {
                    if a.el == El::H {
                        continue;
                    }
                    let p = f.proj[i];
                    assert_eq!(p.body, Body::Compact, "{name} atom {i}");
                    let mut c = Canvas::new(size, GlyphMode::Unicode);
                    paint(&mut c, &m, &f, &[i as u16]);
                    let (cx, cy) = (p.x.round() as i32, p.y.round() as i32);
                    // A `█▓░` lozenge on the centre row, never a `+` of cells.
                    for y in 0..h {
                        for x in 0..w {
                            let want = y == cy && (x - cx).abs() <= 1;
                            let painted = c.get(x, y).unwrap().ch != ' ';
                            assert_eq!(painted, want, "{name} t = {t} atom {i} ({x}, {y})");
                            assert_eq!(covers(&p, x, y), want);
                        }
                    }
                    // Lit from the left, like the round balls.
                    let lvl = |x: i32| c.get(x, cy).unwrap().ink.level;
                    assert!(lvl(cx - 1) > lvl(cx + 1), "{name} t = {t} atom {i}");
                }
            }
        }
    }

    #[test]
    fn mol_a6_rear_bonds_are_dotted() {
        let size = Size::new(80, 24);
        // In ASCII a ball's dim rim never reads as a dotted rear stick running out of it.
        assert!(BODY.iter().all(|g| g.ascii != REAR_BOND.ascii));
        assert_eq!(REAR_BOND, Glyph::new('·', '.'));
        let (m, view) = (by_name("buckminsterfullerene"), View::new(size));
        let bodies = Bodies::new(&view, &m);
        let (mut rear, mut front) = (0, 0);
        for step in 0..12 {
            let pose = Pose {
                molecule: 5,
                s: 1.0,
                yaw: f64::from(step) * 0.5,
                pitch: 0.3,
            };
            let f = frame(&m, &view, &pose, bodies);
            let n = m.atoms.len();
            for (j, bond) in m.bonds.iter().enumerate() {
                let (a, b) = (f.proj[usize::from(bond.a)], f.proj[usize::from(bond.b)]);
                let far = !a.near && !b.near;
                for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
                    let mut c = Canvas::new(size, mode);
                    paint(&mut c, &m, &f, &[(n + j) as u16]);
                    for ch in c
                        .to_lines()
                        .iter()
                        .flat_map(|l| l.chars().collect::<Vec<_>>())
                    {
                        // The back face is a faint dotted lattice in both modes; the
                        // front face keeps its slope strokes.
                        match (mode, ch) {
                            (_, ' ') => {}
                            (GlyphMode::Ascii, '.') | (GlyphMode::Unicode, '·') => {
                                assert!(far, "front bond dotted");
                            }
                            (GlyphMode::Ascii, _) => {
                                assert!(!far && "-|/\\".contains(ch), "{ch:?}");
                            }
                            (GlyphMode::Unicode, _) => {
                                assert!(!far && "─│╱╲".contains(ch), "{ch:?}");
                            }
                        }
                    }
                }
                if far { rear += 1 } else { front += 1 }
            }
        }
        assert!(rear > 100 && front > 100, "{rear} {front}");
    }

    #[test]
    fn mol_a6_ascii_frames_show_element_symbols() {
        let size = Size::new(80, 24);
        let mut scene = MoleculeViewer::new();
        scene.resize(size, &mut Ctx::new(42, size));
        for step in 0..120 {
            scene.t = GROW + f64::from(step) * 0.5;
            let pose = scene.pose();
            if pose.s < 1.0 {
                continue;
            }
            let m = &scene.models[pose.molecule];
            let mut canvas = Canvas::new(size, GlyphMode::Ascii);
            scene.render(&mut canvas);
            assert!(canvas.to_lines().iter().all(|l| l.is_ascii()));
            let f = frame(m, &scene.view, &pose, scene.bodies[pose.molecule]);
            let order = &f.order[..f.items];
            for (k, &item) in order.iter().enumerate() {
                let i = usize::from(item);
                if i >= m.atoms.len() {
                    continue;
                }
                let p = f.proj[i];
                let (x, y) = (p.x.round() as i32, p.y.round() as i32);
                let mut later = Canvas::new(size, GlyphMode::Ascii);
                paint(&mut later, m, &f, &order[k + 1..]);
                // The label row is drawn last too.
                if y == 0 || later.get(x, y).is_some_and(|c| c.ch != ' ') {
                    continue;
                }
                let near = nu(p.depth, m.r_eff) >= 0.5;
                assert_eq!(canvas.get(x, y).unwrap().ch, m.atoms[i].el.symbol(near));
            }
        }
    }
}
