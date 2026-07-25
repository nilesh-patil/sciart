//! Ribosome translation: a ribosome reads a messenger RNA three letters at a time; transfer
//! RNAs bring amino acids, the protein grows as a chain of beads and coils into a helix as
//! it leaves (SPEC-002 `ribosome_translation`).
//!
//! The mRNAs are the starts of real human / reporter coding sequences (5′ → 3′, `T` read
//! as `U`), each cut after a few dozen codons and closed with a stop codon:
//! - haemoglobin β (human `HBB`, NM_000518): the first 31 residues,
//!   `MVHLTPEEKSAVTALWGKVNVDEVGGEALGR` (codon 7, `GAG`, is the sickle-cell site: Glu6 in
//!   the usual numbering, which skips the initiator Met), then `UGA`; first, because all
//!   three residue classes show within its first six residues;
//! - enhanced green fluorescent protein (EGFP, the common reporter CDS): the first 30
//!   residues, `MVSKGEELFTGVVPILVELDGDVNGHKFSV`, then `UAA`;
//! - preproinsulin (human `INS`, NM_000207): its 24-residue signal peptide,
//!   `MALWMRLLPLLALLALWGPDPAAA`, then `UAG`.
//!
//! Every chain is drawn folding into an α-helix whatever its real fold (EGFP's N-terminus,
//! for one, is a β-strand); release factors and elongation factors are not drawn.
//!
//! They are translated with the standard genetic code (all 64 codons, the `TCAG` table).
//! The elongation cycle runs on a looping `Timeline`: an aminoacyl-tRNA flies in to the A
//! site, peptidyl transfer moves the chain onto its amino acid (one bead more),
//! translocation moves the mRNA exactly 3 nucleotides and the tRNAs A → P → E, and the
//! empty tRNA leaves from the E site. The chain leaves the exit tunnel as a loose coil; at
//! the stop codon it is released, slides out of the tunnel and, in `folding`, winds into a
//! tight α-helix (3.6 residues per turn); the next loop translates the next mRNA. Both
//! coils are regular helices whose rise is set from their twist and radius so that
//! consecutive residues stay exactly one bond apart in 3-D.
//!
//! ## Look (GEN-R26, molecules: ball-and-stick, as `chemical_garden`)
//! The mRNA a 1-px `Wave` thread with codon ticks and its letters under the E, P and A
//! sites; the ribosome two subunits, each a 1-px `Structure` outline over a faint stipple
//! (peak 0.08); tRNAs small `Particle` atoms with a short stem down to their codon; the peptide
//! small stipple atoms joined by 1-px bonds, `Hot` charged, `Cold` hydrophobic, `Accent`
//! polar. Beads are placed along the exit path (tunnel, then a projected helix) at exactly
//! one bond length from each other, so the chain coils without stretching.

use sciart_core::draw;
use sciart_core::prelude::*;
use sciart_core::timeline::ease;
use sciart_core::title;

pub static META: Meta = Meta {
    name: "ribosome_translation",
    category: Category::Biochemistry,
    description: "A ribosome reads mRNA codon by codon; tRNAs bring amino acids and the protein chain grows",
    title: "Ribosome · codons become protein",
    captions: &CAPTIONS,
};

const CAPTIONS: [&str; 3] = ["elongation", "termination", "folding"];
const ELONG: usize = 0;
const TERM: usize = 1;
const FOLD: usize = 2;
const DURATIONS: [f64; 3] = [20.0, 3.0, 6.0];

pub fn build() -> Box<dyn Generator> {
    Box::new(Ribosome::default())
}

// ---- Model (SPEC-002 Model) ----

/// One mRNA and the protein it must give (RIB-A1).
pub(crate) struct Gene {
    pub(crate) mrna: &'static str,
    pub(crate) protein: &'static str,
}

/// See the module docs for the sources.
pub(crate) const GENES: [Gene; 3] = [
    Gene {
        mrna: "AUGGUGCAUCUGACUCCUGAGGAGAAGUCUGCCGUUACUGCCCUGUGGGGCAAGGUGAACGUGGAUGAAGUUGGUGGUGAGGCCCUGGGCAGGUGA",
        protein: "MVHLTPEEKSAVTALWGKVNVDEVGGEALGR",
    },
    Gene {
        mrna: "AUGGUGAGCAAGGGCGAGGAGCUGUUCACCGGGGUGGUGCCCAUCCUGGUCGAGCUGGACGGCGACGUAAACGGCCACAAGUUCAGCGUGUAA",
        protein: "MVSKGEELFTGVVPILVELDGDVNGHKFSV",
    },
    Gene {
        mrna: "AUGGCCCUGUGGAUGCGCCUCCUGCCCCUGCUGGCGCUGCUGGCCCUCUGGGGACCUGACCCAGCCGCAGCCUAG",
        protein: "MALWMRLLPLLALLALWGPDPAAA",
    },
];

/// The standard genetic code, codons ordered by bases `U C A G` at positions 1, 2, 3
/// (the classic `TCAG` table); `*` is a stop.
const CODE: &[u8; 64] = b"FFLLSSSSYY**CC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG";

fn base_index(b: u8) -> Option<usize> {
    match b {
        b'U' | b'T' => Some(0),
        b'C' => Some(1),
        b'A' => Some(2),
        b'G' => Some(3),
        _ => None,
    }
}

/// The amino acid (one letter) a codon codes for, `*` for a stop, `?` for a bad codon.
pub(crate) fn codon_aa(c: &[u8]) -> u8 {
    match (
        c.first().copied().and_then(base_index),
        c.get(1).copied().and_then(base_index),
        c.get(2).copied().and_then(base_index),
    ) {
        (Some(a), Some(b), Some(d)) => CODE[16 * a + 4 * b + d],
        _ => b'?',
    }
}

/// The codons of `mrna` from its start up to (not including) the first stop: the number
/// of sense codons a ribosome reads (RIB-A1).
pub(crate) fn sense_codons(mrna: &str) -> usize {
    mrna.as_bytes()
        .chunks(3)
        .take_while(|c| c.len() == 3 && codon_aa(c) != b'*')
        .count()
}

/// Residue `i` (0-based) of `gene`'s protein, read off its mRNA.
fn residue(gene: &Gene, i: usize) -> u8 {
    gene.mrna
        .as_bytes()
        .get(3 * i..3 * i + 3)
        .map_or(b'?', codon_aa)
}

/// Residue classes (RIB-A3): charged `Hot`, hydrophobic `Cold`, polar `Accent`.
pub(crate) fn residue_role(aa: u8) -> Role {
    match aa {
        b'D' | b'E' | b'K' | b'R' | b'H' => Role::Hot,
        b'A' | b'V' | b'L' | b'I' | b'M' | b'F' | b'W' | b'C' => Role::Cold,
        _ => Role::Accent,
    }
}

/// Where elongation stands at `t` seconds into it for a gene of `sense` codons: the
/// cycle `k` (decoding codon `k + 1` at the A site) and the progress `u` through it.
pub(crate) fn elongation(sense: usize, t: f64) -> (usize, f64) {
    let cycles = sense.saturating_sub(1).max(1);
    let period = DURATIONS[ELONG] / cycles as f64;
    let x = (t / period).max(0.0);
    let k = (x.floor() as usize).min(cycles - 1);
    (k, (x - k as f64).clamp(0.0, 1.0))
}

/// Cycle stages (shares of a cycle): arrival, peptidyl transfer, translocation, E exit.
const ARRIVE: f64 = 0.35;
const TRANSFER: f64 = 0.45;
const SHIFT: (f64, f64) = (0.5, 0.75);

fn shift_progress(u: f64) -> f64 {
    ease::smoothstep((u - SHIFT.0) / (SHIFT.1 - SHIFT.0))
}

/// mRNA offset (nucleotides moved through the ribosome) in cycle `k` at progress `u`:
/// exactly 3 per cycle (RIB-A2).
pub(crate) fn mrna_offset(k: usize, u: f64) -> f64 {
    3.0 * (k as f64 + shift_progress(u))
}

/// Beads in the chain in cycle `k` at progress `u`: the initiator Met plus one per
/// peptidyl transfer (RIB-A2).
pub(crate) fn bead_count(k: usize, u: f64) -> usize {
    1 + k + usize::from(u >= TRANSFER)
}

/// How far (in bonds) the newest bead sits along the exit path from the A site: at the
/// P site (1) between cycles, at the A site (0) right after transfer, pushed back to P
/// by translocation.
pub(crate) fn chain_shift(u: f64) -> f64 {
    if u < TRANSFER { 1.0 } else { shift_progress(u) }
}

// ---- Look (SPEC-002 Look) ----

/// Dots per nucleotide: one terminal cell, so each letter sits under its nucleotide.
const NT: f64 = 2.0;
/// Bond length (dots): one codon, so translocation moves the chain by exactly one bond.
pub(crate) const BOND: f64 = 3.0 * NT;
const ATOM_SIGMA: f64 = 0.7;
const ATOM_PEAK: f64 = 2.0;
const TRNA_SIGMA: f64 = 0.8;
const TRNA_PEAK: f64 = 2.2;
const BEAD_LEVEL: u8 = 230;
const BOND_LEVEL: u8 = 150;
/// Beads and bonds on the helix's far side.
const BACK_LEVEL: u8 = 120;
const BACK_BOND: u8 = 80;
const BOND_GAP: f64 = 2.0;
const MRNA_INK: Ink = Ink::new(Role::Wave, 200);
const TICK_INK: Ink = Ink::new(Role::Wave, 120);
const TRNA_INK: Ink = Ink::new(Role::Particle, 220);
const STEM_INK: Ink = Ink::new(Role::Particle, 120);
const LARGE_INK: Ink = Ink::new(Role::Structure, 110);
const SMALL_INK: Ink = Ink::new(Role::Dim, 140);
const OUTLINE_INK: Ink = Ink::new(Role::Structure, 170);
const LETTER_INK: Ink = Ink::new(Role::Text, 170);
const READOUT_INK: Ink = Ink::new(Role::Text, title::READOUT_LEVEL);
/// Ribosome subunit interior stipple peak density (an outline carries the shape).
pub(crate) const SUBUNIT_PEAK: f64 = 0.08;
/// The tRNA atom sits this far above the mRNA, its amino acid one bond higher.
const TRNA_UP: f64 = 6.5;
/// Large subunit: centre offset from the P site's tRNA and semi-axes (dots); the exit
/// tunnel opens above it.
const LARGE: (f64, f64, f64, f64) = (-2.0, -11.0, 20.0, 11.0);
/// The small subunit sits below the codon letters with one blank cell row between.
const SMALL: (f64, f64, f64, f64) = (-1.0, 13.0, 17.0, 4.0);
const TUNNEL_TOP: f64 = -24.0;
/// The tunnel's exit sits this far down the screen.
const EXIT_AT: f64 = 0.55;
/// The coils as (twist per residue, radius in bonds): the loose coil the chain leaves the
/// tunnel in, and the α-helix it folds into (Cα 3.8 Å apart on radius 2.3 Å, 100° per
/// residue: 3.6 residues per turn, rise 1.5 Å). The rise follows from the bond length.
const LOOSE: (f64, f64) = (140.0 * std::f64::consts::PI / 180.0, 0.45);
const ALPHA: (f64, f64) = (100.0 * std::f64::consts::PI / 180.0, 2.3 / 3.8);
const HELIX_TILT: f64 = 0.45;
const HELIX_AXIS: (f64, f64) = (-0.92, -0.39);
/// Beads the chain can hold (the longest gene) and path length to lay them out on.
const MAX_BEADS: usize = 40;
/// Exit path sampling step (dots).
const PATH_STEP: f64 = 0.25;
const SUBUNIT_SALT: u64 = 0x21B0_5011_0000_0001;
const ATOM_SALT: u64 = 0x21B0_5011_0000_0002;

#[derive(Debug)]
struct Ribosome {
    time: f64,
    timeline: Timeline,
    dots: Dots,
    size: Size,
    /// The P codon's first nucleotide's dot column and the mRNA's dot row.
    x0: f64,
    ym: f64,
    /// The exit path from the A site's amino acid, sampled every `PATH_STEP` dots.
    path: Vec<(f64, f64)>,
    /// Bead centres this frame, newest first.
    beads: [(f64, f64); MAX_BEADS],
    /// The same beads in the model's 3-D frame (dots), for depth and RIB-A4.
    model: [[f64; 3]; MAX_BEADS],
    n_beads: usize,
    /// The frame last rasterised, for `render`'s labels.
    shown: Frame,
    /// The coil the beads are laid on this frame: twist, radius and rise (dots).
    coil: (f64, f64, f64),
    readout_room: usize,
    dirty: bool,
}

impl Default for Ribosome {
    fn default() -> Self {
        let phases: [(&'static str, f64); 3] = std::array::from_fn(|k| (CAPTIONS[k], DURATIONS[k]));
        Self {
            time: 0.0,
            timeline: Timeline::new(&phases),
            dots: Dots::default(),
            size: Size::new(1, 1),
            x0: 0.0,
            ym: 0.0,
            path: Vec::new(),
            beads: [(0.0, 0.0); MAX_BEADS],
            model: [[0.0; 3]; MAX_BEADS],
            n_beads: 0,
            shown: Frame {
                gene: 0,
                act: ELONG,
                k: 0,
                u: 0.0,
                offset: 0.0,
                beads: 1,
                shift: 1.0,
                q: 0.0,
                fold: 0.0,
            },
            coil: coil(0.0),
            readout_room: 0,
            dirty: true,
        }
    }
}

/// What the scene shows at one instant, all closed-form in the loop time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) gene: usize,
    pub(crate) act: usize,
    /// Elongation cycle and progress (the last cycle, complete, after elongation).
    pub(crate) k: usize,
    pub(crate) u: f64,
    pub(crate) offset: f64,
    pub(crate) beads: usize,
    /// Newest bead's distance along the path, in bonds.
    pub(crate) shift: f64,
    /// Progress through termination / folding.
    pub(crate) q: f64,
    /// How far the chain has folded: 0 the loose exit coil, 1 the α-helix.
    pub(crate) fold: f64,
}

/// The coil at fold `f` (0 loose … 1 α-helix): twist, radius and the rise that keeps
/// consecutive residues one bond apart (RIB-A4).
pub(crate) fn coil(f: f64) -> (f64, f64, f64) {
    let f = ease::smoothstep(f);
    let turn = ease::lerp(LOOSE.0, ALPHA.0, f);
    let r = BOND * ease::lerp(LOOSE.1, ALPHA.1, f);
    let chord = 2.0 * r * (0.5 * turn).sin_det();
    (turn, r, (BOND * BOND - chord * chord).max(0.0).sqrt())
}

impl Ribosome {
    fn frame(&self) -> Frame {
        let beat = self.timeline.at(self.time);
        let gene = self
            .timeline
            .cycle(self.time)
            .rem_euclid(GENES.len() as i64) as usize;
        let sense = sense_codons(GENES[gene].mrna);
        let cycles = sense.saturating_sub(1).max(1);
        match beat.index {
            ELONG => {
                let (k, u) = elongation(sense, beat.local);
                Frame {
                    gene,
                    act: ELONG,
                    k,
                    u,
                    offset: mrna_offset(k, u),
                    beads: bead_count(k, u),
                    shift: chain_shift(u),
                    q: 0.0,
                    fold: 0.0,
                }
            }
            act => {
                // Released at the stop codon: the chain slides out of the tunnel, then
                // folds.
                let q = beat.progress;
                let (slide, fold) = if act == TERM {
                    (ease::smoothstep((q - 0.2) / 0.8), 0.0)
                } else {
                    (1.0, (q / 0.55).min(1.0))
                };
                let tunnel = (self.path.len().max(1) - 1) as f64 * PATH_STEP / BOND;
                Frame {
                    gene,
                    act,
                    k: cycles - 1,
                    u: 1.0,
                    offset: 3.0 * cycles as f64,
                    beads: sense,
                    shift: 1.0 + (tunnel + 0.2) * slide,
                    q,
                    fold,
                }
            }
        }
    }

    /// Dot column of nucleotide `i` when the mRNA is `offset` nucleotides in.
    fn nt_x(&self, i: f64, offset: f64) -> f64 {
        self.x0 + NT * (i - offset)
    }

    /// The P site's tRNA atom (dots); A is one codon right, E one left.
    fn p_site(&self) -> (f64, f64) {
        (self.x0 + 1.5 * NT - 0.5, self.ym - TRNA_UP)
    }

    /// Lay the exit path out from the A site's amino acid, sampled every `PATH_STEP`
    /// dots of arc: left to the P site, a quarter turn up (radius one bond), then the
    /// exit tunnel up through the large subunit. The helix beyond it is closed-form.
    fn build_path(&mut self) {
        self.path.clear();
        let (px, py) = self.p_site();
        let (ax, ay) = (px + BOND, py - BOND);
        let pa = (px, py - BOND);
        let n = (BOND / PATH_STEP).round() as usize;
        let arc = (std::f64::consts::FRAC_PI_2 * BOND / PATH_STEP).round() as usize;
        let rise = ((pa.1 - BOND - (self.ym + TUNNEL_TOP)).max(0.0) / PATH_STEP).round() as usize;
        // Exactly the samples below: line, quarter turn, tunnel (both ends).
        self.path.reserve(n + arc + rise.max(1) + 1);
        for i in 0..n {
            self.path.push((ax - BOND * i as f64 / n as f64, ay));
        }
        let r = BOND;
        let c = (pa.0, pa.1 - r);
        let steps = (std::f64::consts::FRAC_PI_2 * r / PATH_STEP).round() as usize;
        for i in 0..steps {
            let th = std::f64::consts::FRAC_PI_2 * (1.0 + i as f64 / steps as f64);
            let (s, co) = th.sin_cos_det();
            self.path.push((c.0 + r * co, c.1 + r * s));
        }
        let start = (c.0 - r, c.1);
        let rise = (start.1 - (self.ym + TUNNEL_TOP)).max(0.0);
        let steps = ((rise / PATH_STEP).round() as usize).max(1);
        for i in 0..=steps {
            self.path
                .push((start.0, start.1 - rise * i as f64 / steps as f64));
        }
    }

    /// A point `s` dots along the exit path, in the model's 3-D frame (screen plane x, y
    /// and depth z, dots): the tunnel, then the current coil about an up-left axis, whose
    /// rise keeps consecutive residues one bond apart in 3-D (as Cα atoms are 3.8 Å apart).
    pub(crate) fn at_arc(&self, s: f64) -> [f64; 3] {
        let Some(&exit) = self.path.last() else {
            return [0.0; 3];
        };
        let tunnel = (self.path.len() - 1) as f64 * PATH_STEP;
        if s <= tunnel {
            let x = (s.max(0.0) / PATH_STEP).min((self.path.len() - 1) as f64);
            let i = (x.floor() as usize).min(self.path.len() - 2);
            let f = x - i as f64;
            let (a, b) = (self.path[i], self.path[i + 1]);
            return [a.0 + f * (b.0 - a.0), a.1 + f * (b.1 - a.1), 0.0];
        }
        let m = (s - tunnel) / BOND;
        let (turn, r, rise) = self.coil;
        let (sn, cs) = (turn * m).sin_cos_det();
        let (along, across, depth) = (rise * m, r * sn, r * (cs - 1.0));
        let (dx, dy) = HELIX_AXIS;
        let (nx, ny) = (-dy, dx);
        [
            exit.0 + dx * along + nx * across,
            exit.1 + dy * along + ny * across,
            depth,
        ]
    }

    /// Place the chain: bead `k` (newest first) `k + shift` bonds along the exit path
    /// (RIB-A4), seen obliquely: depth leans along the axis, so the helix loops on screen.
    fn place_beads(&mut self, n: usize, shift: f64, drift: (f64, f64), fold: f64) {
        self.n_beads = n.min(MAX_BEADS);
        self.coil = coil(fold);
        let (dx, dy) = HELIX_AXIS;
        for k in 0..self.n_beads {
            let p = self.at_arc((k as f64 + shift) * BOND);
            self.model[k] = p;
            self.beads[k] = (
                p[0] - dx * HELIX_TILT * p[2] + drift.0,
                p[1] - dy * HELIX_TILT * p[2] + drift.1,
            );
        }
    }

    /// Whether bead `k` is on the far side of the helix (drawn a step dimmer, so the
    /// coil reads in depth).
    fn back(&self, k: usize) -> bool {
        self.model[k][2] < -self.coil.1
    }

    fn bond(&mut self, a: (f64, f64), b: (f64, f64), inks: (Ink, Ink)) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        if len.is_nan() || len <= 2.0 * BOND_GAP + 0.5 {
            return;
        }
        let (f0, f1) = (BOND_GAP / len, 1.0 - BOND_GAP / len);
        let m = (a.0 + 0.5 * dx, a.1 + 0.5 * dy);
        self.dots
            .line(a.0 + f0 * dx, a.1 + f0 * dy, m.0, m.1, inks.0);
        self.dots
            .line(m.0, m.1, a.0 + f1 * dx, a.1 + f1 * dy, inks.1);
    }

    fn subunit(&mut self, (ox, oy, rx, ry): (f64, f64, f64, f64), ink: Ink, salt: u64) {
        let (px, _) = self.p_site();
        let (cx, cy) = (px + ox, self.ym + oy);
        let size = self.dots.dot_size();
        let (x0, x1) = (
            (cx - rx).ceil().max(0.0),
            (cx + rx).floor().min(f64::from(size.width()) - 1.0),
        );
        let (y0, y1) = (
            (cy - ry).ceil().max(0.0),
            (cy + ry).floor().min(f64::from(size.height()) - 1.0),
        );
        for y in y0 as i32..=y1 as i32 {
            let v = (f64::from(y) - cy) / ry;
            for x in x0 as i32..=x1 as i32 {
                let u = (f64::from(x) - cx) / rx;
                let r2 = u * u + v * v;
                if r2 < 1.0 {
                    // Soft edge: the density falls off over the outer third.
                    let p = SUBUNIT_PEAK * ease::smoothstep((1.0 - r2) / 0.45);
                    self.dots.stipple(x, y, p, salt, ink);
                }
            }
        }
        // The outline, a 1-px thread round the ellipse.
        let n = 48;
        let mut last = (cx + rx, cy);
        for i in 1..=n {
            let (sn, cs) = (std::f64::consts::TAU * f64::from(i) / f64::from(n)).sin_cos_det();
            let next = (cx + rx * cs, cy + ry * sn);
            self.dots.line(last.0, last.1, next.0, next.1, OUTLINE_INK);
            last = next;
        }
    }

    /// A tRNA: a `Particle` atom with its stem down to the codon, and its amino acid (if
    /// charged) one bond above, joined by a bond.
    fn trna(&mut self, at: (f64, f64), aa: Option<u8>, level: u8, salt: u64) {
        let scale = |ink: Ink| {
            Ink::new(
                ink.role,
                ((u16::from(ink.level) * u16::from(level)) / 255) as u8,
            )
        };
        self.dots.line(
            at.0,
            at.1 + 2.0,
            at.0,
            (at.1 + TRNA_UP - 2.0).min(self.ym - 2.0),
            scale(STEM_INK),
        );
        self.dots
            .stipple_puff(at.0, at.1, TRNA_SIGMA, TRNA_PEAK, salt, scale(TRNA_INK));
        if let Some(aa) = aa {
            let b = (at.0, at.1 - BOND);
            let ink = Ink::new(residue_role(aa), BEAD_LEVEL);
            self.bond(at, b, (scale(STEM_INK), Ink::new(ink.role, BOND_LEVEL)));
            self.dots
                .stipple_puff(b.0, b.1, ATOM_SIGMA, ATOM_PEAK, salt ^ 0xAA, scale(ink));
        }
    }

    fn rasterise(&mut self) {
        self.dots.clear();
        let f = self.frame();
        self.shown = f;
        let gene = &GENES[f.gene];
        let sense = sense_codons(gene.mrna);
        self.subunit(LARGE, LARGE_INK, SUBUNIT_SALT);
        self.subunit(SMALL, SMALL_INK, SUBUNIT_SALT ^ 1);

        // The mRNA, with 5′ and 3′ tails, and a tick at each codon's start.
        let len = gene.mrna.len() as f64;
        let (a, b) = (self.nt_x(-18.0, f.offset), self.nt_x(len + 18.0, f.offset));
        self.dots.line(a, self.ym, b, self.ym, MRNA_INK);
        let mut i = 0.0;
        while i <= len {
            let x = self.nt_x(i, f.offset).round();
            self.dots.plot(x, self.ym - 1.0, TICK_INK);
            i += 3.0;
        }

        let (px, py) = self.p_site();
        let slide = f.offset - 3.0 * f.k as f64;
        let dx = -NT * slide;
        let salt = |n: usize| ATOM_SALT ^ (n as u64) << 8;
        match f.act {
            ELONG => {
                let (k, u) = (f.k, f.u);
                // P-site tRNA (codon k), moving to E in translocation, then leaving.
                let p_at = (px + dx, py);
                if u < SHIFT.1 {
                    self.trna(p_at, None, 255, salt(k));
                } else {
                    let w = ease::smoothstep((u - SHIFT.1) / (1.0 - SHIFT.1));
                    let level = if w < 0.6 { 255 } else { 120 };
                    self.trna((p_at.0 - 15.0 * w, p_at.1 - 17.0 * w), None, level, salt(k));
                }
                // A-site tRNA (codon k + 1): flies in, then holds the chain.
                let target = (px + BOND + dx, py);
                let at = if u < ARRIVE {
                    let w = ease::out_quad(u / ARRIVE);
                    (
                        ease::lerp(target.0 + 25.0, target.0, w),
                        ease::lerp(target.1 - 20.0, target.1, w),
                    )
                } else {
                    target
                };
                let aa = residue(gene, k + 1);
                let charged = (u < TRANSFER).then_some(aa);
                self.trna(at, charged, 255, salt(k + 1));
                // Before transfer the P-site tRNA holds the chain: a bond to its newest
                // bead; after it the A-site one does.
                let holder = if u < TRANSFER { p_at } else { at };
                self.place_beads(f.beads, f.shift, (0.0, 0.0), f.fold);
                if self.n_beads > 0 {
                    let first = self.beads[0];
                    let role = residue_role(residue(gene, f.beads - 1));
                    self.bond(holder, first, (STEM_INK, Ink::new(role, BOND_LEVEL)));
                }
            }
            TERM => {
                // The stop codon sits in the A site; the last tRNA lets go and leaves.
                let w = ease::smoothstep(f.q / 0.6);
                let level = if w < 0.6 { 255 } else { 120 };
                if f.q < 0.6 {
                    self.trna((px - 12.0 * w, py - 14.0 * w), None, level, salt(f.k + 1));
                }
                self.place_beads(f.beads, f.shift, (0.0, 0.0), f.fold);
            }
            _ => {
                // The released protein winds into its helix, then fades in steps.
                self.place_beads(f.beads, f.shift, (0.0, 0.0), f.fold);
            }
        }

        // The chain: bonds first, then the atoms.
        let level = match f.act {
            FOLD if f.q > 0.94 => return,
            FOLD if f.q > 0.85 => 110,
            _ => BEAD_LEVEL,
        };
        let bond_level = level.min(BOND_LEVEL);
        let n = self.n_beads.min(sense);
        // Bead k (newest first) is residue n − 1 − k.
        for k in 1..n {
            let bond_level = if self.back(k) || self.back(k - 1) {
                bond_level.min(BACK_BOND)
            } else {
                bond_level
            };
            let (ra, rb) = (
                residue_role(residue(gene, n - k)),
                residue_role(residue(gene, n - 1 - k)),
            );
            self.bond(
                self.beads[k - 1],
                self.beads[k],
                (Ink::new(ra, bond_level), Ink::new(rb, bond_level)),
            );
        }
        for k in 0..n {
            let role = residue_role(residue(gene, n - 1 - k));
            let (x, y) = self.beads[k];
            let level = if self.back(k) {
                level.min(BACK_LEVEL)
            } else {
                level
            };
            self.dots.stipple_puff(
                x,
                y,
                ATOM_SIGMA,
                ATOM_PEAK,
                ATOM_SALT ^ 0xBEAD ^ (n - 1 - k) as u64,
                Ink::new(role, level),
            );
        }
    }
}

impl Generator for Ribosome {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn caption(&self) -> Option<&'static str> {
        Some(CAPTIONS[self.timeline.at(self.time).index])
    }

    fn resize(&mut self, size: Size, _ctx: &mut Ctx) {
        self.size = size;
        self.dots.resize(size);
        let rows = f64::from(size.height().saturating_sub(1).max(1));
        // The mRNA on the bottom dot row of a cell row, placed so the exit tunnel opens
        // ~55 % down the screen; letters in the row below it. The ribosome sits right of
        // centre: the chain leaves to the left, the tRNAs arrive from the right.
        let want = (EXIT_AT * 4.0 * rows - TUNNEL_TOP - 3.0) / 4.0;
        let row = want.round().min(rows - 2.0).max(0.0);
        self.ym = 4.0 * row + 3.0;
        self.x0 = 2.0 * (f64::from(size.width()) * 0.68).floor();
        self.path.clear();
        self.build_path();
        self.readout_room = title::stable_readout_room(size, &META);
        self.dirty = true;
    }

    fn update(&mut self, dt: f64, _ctx: &mut Ctx) {
        if dt > 0.0 {
            self.time += dt.min(0.25);
            self.dirty = true;
        }
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
        // The frame `prepare` drew, so the labels match the art.
        let f = self.shown;
        let gene = GENES[f.gene].mrna.as_bytes();
        let h = i32::from(self.size.height());
        // Codon letters under the E, P and A sites, snapped to whole nucleotides.
        let row = (self.ym as i32) / Dots::CELL_H + 1;
        if row < h - 1 {
            let m = f.offset.round() as i64;
            for i in (m - 3)..(m + 6) {
                let Some(&b) = usize::try_from(i).ok().and_then(|i| gene.get(i)) else {
                    continue;
                };
                let x = self.x0 + NT * (i as f64 - f.offset.round());
                let col = (x / f64::from(Dots::CELL_W)).floor() as i32;
                canvas.put(col, row, Glyph::ascii(char::from(b)), LETTER_INK);
            }
        }
        // Readout: the tail of the protein made so far.
        // The last dozen residues: the readout stays a glance, not a line of text.
        let room = self.readout_room.min(12);
        if room >= 4 {
            let made = f.beads.min(GENES[f.gene].protein.len());
            let protein = &GENES[f.gene].protein[..made];
            let tail = &protein[protein.len().saturating_sub(room)..];
            draw::label_right(canvas, h - 1, tail, READOUT_INK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(size: Size) -> (Ribosome, Ctx) {
        let mut ctx = Ctx::new(1, size);
        let mut g = Ribosome::default();
        g.resize(size, &mut ctx);
        (g, ctx)
    }

    #[test]
    fn rib_a1_the_mrnas_give_their_proteins_by_the_standard_code() {
        // The table: 64 codons, 3 stops (UAA UAG UGA), AUG = M, 20 amino acids with the
        // textbook degeneracies.
        let stops: Vec<String> = (0..64)
            .filter(|&i| CODE[i] == b'*')
            .map(|i| {
                let b = |k: usize| ['U', 'C', 'A', 'G'][k];
                [b(i / 16), b(i / 4 % 4), b(i % 4)].iter().collect()
            })
            .collect();
        assert_eq!(stops, ["UAA", "UAG", "UGA"]);
        assert_eq!(codon_aa(b"AUG"), b'M');
        assert_eq!(codon_aa(b"UGG"), b'W');
        let count = |aa: u8| CODE.iter().filter(|&&c| c == aa).count();
        for (aa, n) in [
            (b'L', 6),
            (b'S', 6),
            (b'R', 6),
            (b'A', 4),
            (b'G', 4),
            (b'P', 4),
            (b'T', 4),
            (b'V', 4),
            (b'I', 3),
            (b'M', 1),
            (b'W', 1),
            (b'F', 2),
            (b'Y', 2),
            (b'H', 2),
            (b'Q', 2),
            (b'N', 2),
            (b'K', 2),
            (b'D', 2),
            (b'E', 2),
            (b'C', 2),
        ] {
            assert_eq!(count(aa), n, "{}", char::from(aa));
        }
        for g in &GENES {
            assert_eq!(g.mrna.len() % 3, 0);
            assert!(g.mrna.starts_with("AUG"));
            let n = sense_codons(g.mrna);
            assert_eq!(n * 3 + 3, g.mrna.len(), "the stop is the last codon");
            let protein: String = (0..n).map(|i| char::from(residue(g, i))).collect();
            assert_eq!(protein, g.protein);
            assert!(n < MAX_BEADS);
        }
        let ends: Vec<&str> = GENES.iter().map(|g| &g.mrna[g.mrna.len() - 3..]).collect();
        assert_eq!(ends, ["UGA", "UAA", "UAG"], "every stop codon is used");
    }

    #[test]
    fn rib_a2_each_cycle_moves_three_nucleotides_and_adds_one_bead() {
        for g in &GENES {
            let sense = sense_codons(g.mrna);
            let n = 4000;
            let (mut last_off, mut last_beads) = (0.0, 1);
            let mut last_k = 0;
            for i in 0..=n {
                let t = DURATIONS[ELONG] * f64::from(i) / f64::from(n) * 0.99999;
                let (k, u) = elongation(sense, t);
                let (off, beads) = (mrna_offset(k, u), bead_count(k, u));
                assert!(off >= last_off - 1e-12 && beads >= last_beads);
                if k != last_k {
                    assert_eq!(k, last_k + 1);
                    // A cycle ended: exactly 3 nt and one bead more than it began with.
                    assert!((mrna_offset(k, 0.0) - 3.0 * k as f64).abs() < 1e-12);
                    assert_eq!(bead_count(k, 0.0), bead_count(last_k, 0.0) + 1);
                }
                assert_eq!(mrna_offset(k, 1.0), 3.0 * (k + 1) as f64);
                last_off = off;
                last_beads = beads;
                last_k = k;
            }
            // The whole sequence is read: the stop ends in the A site.
            assert_eq!(last_k, sense - 2);
            assert_eq!(last_beads, sense);
        }
    }

    #[test]
    fn rib_a3_bead_inks_follow_the_residue_classes() {
        for aa in *b"DEKRH" {
            assert_eq!(residue_role(aa), Role::Hot);
        }
        for aa in *b"AVLIMFWC" {
            assert_eq!(residue_role(aa), Role::Cold);
        }
        for aa in *b"STNQGYP" {
            assert_eq!(residue_role(aa), Role::Accent);
        }
        // Rendered: every bead centre's cell carries its residue's role or a brighter
        // tRNA/bead neighbour; check the chain's own colours on a big canvas mid-way.
        let size = Size::new(200, 60);
        let (mut g, mut ctx) = started(size);
        while g.time < 15.0 {
            g.update(1.0 / 60.0, &mut ctx);
        }
        g.prepare();
        let f = g.frame();
        let gene = &GENES[f.gene];
        let mut ok = 0;
        for k in 0..g.n_beads {
            let (x, y) = g.beads[k];
            let (cx, cy) = ((x.round() as usize) / 2, (y.round() as usize) / 4);
            let want = residue_role(residue(gene, f.beads - 1 - k));
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.dots.blit(&mut canvas);
            if let Some(c) = canvas.cells().get(cy * 200 + cx) {
                ok += usize::from(c.ink.role == want);
            }
        }
        assert!(
            ok * 10 >= g.n_beads * 8,
            "{ok} of {} beads in their class colour",
            g.n_beads
        );
    }

    #[test]
    fn rib_a4_consecutive_beads_stay_one_bond_apart() {
        let (mut g, mut ctx) = started(Size::new(200, 50));
        let total: f64 = DURATIONS.iter().sum();
        let mut checked = 0;
        while g.time < 3.0 * total {
            g.update(0.05, &mut ctx);
            g.prepare();
            for k in 1..g.n_beads {
                // In the model's 3-D frame the bonds keep their length …
                let (a, b) = (g.model[k - 1], g.model[k]);
                let d =
                    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                assert!(
                    (d - BOND).abs() <= 0.1 * BOND,
                    "t {:.2}: bond {k} is {d:.2}",
                    g.time
                );
                // … and the oblique view never stretches one on screen.
                let (a, b) = (g.beads[k - 1], g.beads[k]);
                let e = (a.0 - b.0).hypot(a.1 - b.1);
                assert!(e <= 1.1 * BOND, "t {:.2}: drawn bond {k} is {e:.2}", g.time);
                checked += 1;
            }
        }
        assert!(checked > 1000);
    }

    #[test]
    fn rib_a5_captions_follow_the_acts_and_loop() {
        let (mut g, mut ctx) = started(Size::new(80, 24));
        let mut seen: Vec<&str> = Vec::new();
        let total: f64 = DURATIONS.iter().sum();
        while g.time < 2.0 * total + 1.0 {
            let c = g.caption().expect("caption");
            if seen.last() != Some(&c) {
                seen.push(c);
            }
            g.update(0.1, &mut ctx);
        }
        let mut want = CAPTIONS.to_vec();
        want.extend(CAPTIONS);
        want.push(CAPTIONS[0]);
        assert_eq!(seen, want);
    }

    #[test]
    fn rib_subunits_are_outlined_and_the_palette_is_m11() {
        // Review round 1: a faint interior and a 1-px outline per subunit.
        let peak = SUBUNIT_PEAK;
        assert!(peak <= 0.08, "subunit interior {peak}");
        let size = Size::new(200, 50);
        {
            let (mut g, _) = started(size);
            g.prepare();
            let (px, _) = g.p_site();
            for (ox, oy, rx, _) in [LARGE, SMALL] {
                let (x, y) = (px + ox - rx, g.ym + oy);
                let lit = (-1..=1).any(|d| g.dots.is_lit(x.round() as i32 + d, y.round() as i32));
                assert!(lit, "subunit outline at ({x}, {y})");
            }
        }
        let (mut g, mut ctx) = started(size);
        for t in [1.0, 8.0, 21.0, 25.0, 40.0] {
            while g.time < t {
                g.update(1.0 / 60.0, &mut ctx);
            }
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            for c in canvas.cells() {
                if c.ch != ' ' {
                    assert!(c.bg == Ink::BLANK && !matches!(c.ch, '▀' | '▄' | '█'));
                    assert!(matches!(
                        c.ink.role,
                        Role::Hot
                            | Role::Cold
                            | Role::Accent
                            | Role::Particle
                            | Role::Wave
                            | Role::Dim
                            | Role::Structure
                            | Role::Text
                    ));
                }
            }
        }
    }

    #[test]
    fn rib_trnas_are_particle_not_a_residue_colour() {
        assert_eq!(TRNA_INK.role, Role::Particle);
        assert_eq!(STEM_INK.role, Role::Particle);
    }

    #[test]
    fn rib_the_first_gene_shows_every_residue_class_early() {
        let roles: Vec<Role> = (0..6)
            .map(|i| residue_role(residue(&GENES[0], i)))
            .collect();
        for want in [Role::Hot, Role::Cold, Role::Accent] {
            assert!(roles.contains(&want), "{want:?} missing from {roles:?}");
        }
    }

    #[test]
    fn rib_the_chain_stays_on_screen() {
        let total: f64 = DURATIONS.iter().sum();
        for size in [Size::new(80, 24), Size::new(120, 40), Size::new(200, 50)] {
            let (mut g, mut ctx) = started(size);
            let dots = g.dots.dot_size();
            let (w, h) = (f64::from(dots.width()), f64::from(dots.height() - 4));
            while g.time < 3.0 * total {
                g.update(0.25, &mut ctx);
                g.prepare();
                if g.frame().act == FOLD && g.frame().q > 0.85 {
                    continue; // fading out
                }
                for k in 0..g.n_beads {
                    let (x, y) = g.beads[k];
                    assert!(
                        x >= 1.0 && x <= w - 2.0 && y >= 1.0 && y <= h - 2.0,
                        "{size:?} t {:.2}: bead {k} at ({x:.1}, {y:.1})",
                        g.time
                    );
                }
            }
        }
    }

    #[test]
    fn rib_folding_coils_the_chain_tighter() {
        let (mut g, mut ctx) = started(Size::new(200, 50));
        let start = DURATIONS[ELONG] + DURATIONS[TERM];
        let span = |g: &Ribosome| {
            let (a, b) = (g.model[0], g.model[g.n_beads - 1]);
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        while g.time < start + 0.1 {
            g.update(1.0 / 60.0, &mut ctx);
        }
        g.prepare();
        assert_eq!(g.frame().act, FOLD);
        let loose = span(&g);
        while g.time < start + 0.7 * DURATIONS[FOLD] {
            g.update(1.0 / 60.0, &mut ctx);
        }
        g.prepare();
        let tight = span(&g);
        assert!(tight < 0.8 * loose, "end to end {loose:.1} → {tight:.1}");
        // The folded helix: 3.6 residues per turn.
        assert!((360.0 / ALPHA.0.to_degrees() - 3.6).abs() < 1e-9);
    }

    #[test]
    fn rib_letters_have_a_blank_row_below_and_a_short_readout() {
        // Review round 2: one blank cell row between the codon letters and the small
        // subunit; the readout shows at most the last 12 residues.
        for size in [Size::new(80, 24), Size::new(120, 40), Size::new(200, 50)] {
            let (mut g, mut ctx) = started(size);
            while g.time < 15.0 {
                g.update(0.25, &mut ctx);
            }
            g.prepare();
            let mut canvas = Canvas::new(size, GlyphMode::Unicode);
            g.render(&mut canvas);
            let row = (g.ym as i32) / Dots::CELL_H + 2;
            let (px, _) = g.p_site();
            let (x0, x1) = (
                ((px + SMALL.0 - SMALL.2) / 2.0).floor() as i32,
                ((px + SMALL.0 + SMALL.2) / 2.0).ceil() as i32,
            );
            for x in x0..=x1 {
                if let Some(c) = canvas.get(x, row) {
                    assert_eq!(
                        c.ch, ' ',
                        "{size:?}: cell ({x}, {row}) under the letters is lit"
                    );
                }
            }
            let h = i32::from(size.height()) - 1;
            let readout = (0..i32::from(size.width()))
                .filter(|&x| {
                    canvas
                        .get(x, h)
                        .is_some_and(|c| c.ink == READOUT_INK && c.ch != ' ')
                })
                .count();
            assert!(readout <= 12, "{size:?}: readout of {readout} letters");
        }
    }
}
