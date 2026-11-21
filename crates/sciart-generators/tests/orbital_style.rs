//! GEN-R26 (M17, ADR-0020): the orbital-thread style, after `electron_orbitals` and
//! `strange_attractors`, for the listed scenes. The test's list must be the one in
//! SPEC-002's GEN-R26 text; at 200×50 after 5 s and 20 s at most 3 % of a listed scene's
//! lit cells are half-block cells; and the stipple holds still: on average at most
//! `MAX_CHANGED` of the cells change between consecutive 30 FPS frames over two seconds.

#[path = "common/gen_r25.rs"]
mod gen_r25;

use gen_r25::{GEN_R25, GEN_R26};
use sciart_core::prelude::*;

const SPEC: &str = include_str!("../../../docs/specs/SPEC-002-generators.md");

const SIZE: (u16, u16) = (200, 50);
const TIMES: [u32; 2] = [5, 20];
const MAX_BLOCK_SHARE: f64 = 0.03;
/// Mean share of cells that change per drawn frame (two 60 Hz steps), t = 10–12 s.
const MAX_CHANGED: f64 = 0.10;
/// Per-scene stillness limits that GEN-R26 states with a reason.
const CHANGED_EXCEPTIONS: &[(&str, f64)] = &[
    // The M16 rubber sheet is kept exactly (by choice); its travelling crests alone
    // change 10.1 % of cells per frame.
    ("spacetime_grid", 0.11),
];

fn max_changed(scene: &str) -> f64 {
    CHANGED_EXCEPTIONS
        .iter()
        .find(|(s, _)| *s == scene)
        .map_or(MAX_CHANGED, |&(_, m)| m)
}

/// The scenes named in SPEC-002's GEN-R26 text: the backticked names between ", for " and
/// the first ": everything".
fn spec_list() -> Vec<String> {
    let line = SPEC
        .lines()
        .find(|l| l.starts_with("- **GEN-R26**"))
        .expect("GEN-R26 in SPEC-002");
    let start = line
        .find(", for ")
        .expect("GEN-R26 names its scenes after ', for '")
        + 6;
    let end = start
        + line[start..]
            .find(": everything")
            .expect("GEN-R26 list ends at ': everything'");
    line[start..end]
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn stage(scene: &str) -> Stage {
    let entry = sciart_generators::find(scene).expect("registered");
    let (w, h) = SIZE;
    Stage::new((entry.build)(), 42, Size::new(w, h), GlyphMode::Unicode)
}

/// The worst half-block share of lit cells over `TIMES`, bottom row (title, readouts) left
/// out, as GEN-R25 measures it.
fn block_share(scene: &str) -> f64 {
    let (w, h) = SIZE;
    let mut stage = stage(scene);
    let mut steps = 0_u32;
    let mut worst = 0.0_f64;
    for t in TIMES {
        while steps < t * 60 {
            stage.step(SIM_DT);
            steps += 1;
        }
        let frame = stage.render();
        let body = &frame.cells()[..usize::from(w) * usize::from(h - 1)];
        let (mut lit, mut block) = (0_usize, 0_usize);
        for cell in body {
            let is_block = cell.bg != Ink::BLANK || matches!(cell.ch, '▀' | '▄' | '█');
            if is_block || cell.ch != ' ' {
                lit += 1;
                block += usize::from(is_block);
            }
        }
        worst = worst.max(block as f64 / lit.max(1) as f64);
    }
    worst
}

/// Mean share of cells (glyph or ink) that change between consecutive drawn frames over
/// t = 10–12 s.
fn changed_share(scene: &str) -> f64 {
    let mut stage = stage(scene);
    for _ in 0..600 {
        stage.step(SIM_DT);
    }
    let mut prev: Vec<Cell> = stage.render().cells().to_vec();
    let (mut changed, mut total) = (0_usize, 0_usize);
    for _ in 0..60 {
        stage.step(SIM_DT);
        stage.step(SIM_DT);
        let frame = stage.render();
        let cells = frame.cells();
        changed += cells.iter().zip(&prev).filter(|(a, b)| a != b).count();
        total += cells.len();
        prev.clear();
        prev.extend_from_slice(cells);
    }
    changed as f64 / total.max(1) as f64
}

fn par_map<T: Send>(scenes: &[&'static str], f: fn(&str) -> T) -> Vec<(&'static str, T)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = scenes
            .iter()
            .map(|&scene| s.spawn(move || (scene, f(scene))))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker panicked"))
            .collect()
    })
}

#[test]
fn gen_r26_test_list_matches_the_spec() {
    let want: Vec<String> = GEN_R26.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        spec_list(),
        want,
        "GEN-R26 list in tests/common/gen_r25.rs vs SPEC-002"
    );
    for scene in GEN_R26 {
        assert!(
            sciart_generators::find(scene).is_some(),
            "{scene} not registered"
        );
        assert!(
            !GEN_R25.contains(scene),
            "{scene} is listed by both GEN-R25 and GEN-R26"
        );
    }
}

#[test]
fn gen_r26_orbital_scenes_are_at_most_3_percent_half_blocks() {
    let failing: Vec<String> = par_map(GEN_R26, block_share)
        .into_iter()
        .filter(|&(_, share)| share > MAX_BLOCK_SHARE)
        .map(|(scene, share)| format!("  {scene}: {:.1} %", 100.0 * share))
        .collect();
    assert!(
        failing.is_empty(),
        "GEN-R26: more than {:.0} % half-block cells:\n{}",
        100.0 * MAX_BLOCK_SHARE,
        failing.join("\n")
    );
}

#[test]
fn gen_r26_stipple_holds_still() {
    let failing: Vec<String> = par_map(GEN_R26, changed_share)
        .into_iter()
        .filter(|&(scene, share)| share > max_changed(scene))
        .map(|(scene, share)| format!("  {scene}: {:.1} % of cells per frame", 100.0 * share))
        .collect();
    assert!(
        failing.is_empty(),
        "GEN-R26: stipple shimmers (more than {:.0} % of cells change per frame):\n{}",
        100.0 * MAX_CHANGED,
        failing.join("\n")
    );
}
