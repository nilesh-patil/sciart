//! GEN-R25 (M16, ADR-0019): thread style for the listed scenes. The test's list must be
//! the one in SPEC-002's GEN-R25 text, and at 200×50 after 5 s and 20 s at most 3 % of a
//! listed scene's lit cells are half-block cells (its one small solid body).

#[path = "common/gen_r25.rs"]
mod gen_r25;

use gen_r25::GEN_R25;
use sciart_core::prelude::*;

const SPEC: &str = include_str!("../../../docs/specs/SPEC-002-generators.md");

/// Listed scenes whose M16 restyle has not landed yet: skipped by the half-block check.
/// An allowlist that may only shrink: a listed scene that already passes fails
/// `gen_r25_pending_only_shrinks` until its entry is deleted. Alphabetical.
const PENDING_GEN_R25: &[&str] = &[
    "comet_breakup",
    "cosmic_web",
    "hurricane",
    "jupiter_storms",
    "planetary_nebula",
    "relativistic_jet",
    "saturn_rings",
    "solar_eruption",
    "spiral_galaxy",
    "supernova_remnant",
    "total_solar_eclipse",
    "vortex_street",
];

const SIZE: (u16, u16) = (200, 50);
const TIMES: [u32; 2] = [5, 20];
const MAX_BLOCK_SHARE: f64 = 0.03;

/// The scenes named in SPEC-002's GEN-R25 text: the backticked names between "for " and
/// the first ": braille".
fn spec_list() -> Vec<String> {
    let line = SPEC
        .lines()
        .find(|l| l.starts_with("- **GEN-R25**"))
        .expect("GEN-R25 in SPEC-002");
    let start = line
        .find(", for ")
        .expect("GEN-R25 names its scenes after ', for '")
        + 6;
    let end = start
        + line[start..]
            .find(": braille")
            .expect("GEN-R25 list ends at ': braille'");
    line[start..end]
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The worst half-block share of lit cells over `TIMES` (bottom row, with the title and
/// readouts, left out): a cell is half-block when it shows a half/full block glyph or a
/// background colour.
fn block_share(scene: &str) -> (f64, u32) {
    let entry = sciart_generators::find(scene).expect("registered");
    let (w, h) = SIZE;
    let mut stage = Stage::new((entry.build)(), 42, Size::new(w, h), GlyphMode::Unicode);
    let mut steps = 0_u32;
    let mut worst = (0.0, 0);
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
        let share = block as f64 / lit.max(1) as f64;
        if share >= worst.0 {
            worst = (share, t);
        }
    }
    worst
}

#[test]
fn gen_r25_test_list_matches_the_spec() {
    let want: Vec<String> = GEN_R25.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        spec_list(),
        want,
        "GEN-R25 list in tests/common/gen_r25.rs vs SPEC-002"
    );
    for scene in GEN_R25 {
        assert!(
            sciart_generators::find(scene).is_some(),
            "{scene} not registered"
        );
    }
    for scene in PENDING_GEN_R25 {
        assert!(
            GEN_R25.contains(scene),
            "{scene} pending but not listed in GEN-R25"
        );
    }
    assert!(
        PENDING_GEN_R25.windows(2).all(|p| p[0] < p[1]),
        "PENDING not sorted"
    );
}

#[test]
fn gen_r25_thread_scenes_are_at_most_3_percent_half_blocks() {
    let checked: Vec<&str> = GEN_R25
        .iter()
        .copied()
        .filter(|s| !PENDING_GEN_R25.contains(s))
        .collect();
    let results: Vec<(&str, (f64, u32))> = std::thread::scope(|s| {
        let handles: Vec<_> = checked
            .iter()
            .map(|&scene| s.spawn(move || (scene, block_share(scene))))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker panicked"))
            .collect()
    });
    let failing: Vec<String> = results
        .iter()
        .filter(|(_, (share, _))| *share > MAX_BLOCK_SHARE)
        .map(|(scene, (share, t))| format!("  {scene}: {:.1} % at t = {t} s", 100.0 * share))
        .collect();
    assert!(
        failing.is_empty(),
        "GEN-R25: more than {:.0} % half-block cells:\n{}",
        100.0 * MAX_BLOCK_SHARE,
        failing.join("\n")
    );
}

#[test]
fn gen_r25_pending_only_shrinks() {
    let passing: Vec<&str> = std::thread::scope(|s| {
        let handles: Vec<_> = PENDING_GEN_R25
            .iter()
            .map(|&scene| s.spawn(move || (scene, block_share(scene).0)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker panicked"))
            .filter(|&(_, share)| share <= MAX_BLOCK_SHARE)
            .map(|(scene, _)| scene)
            .collect()
    });
    assert!(
        passing.is_empty(),
        "GEN-R25: pending scenes already within the half-block share; delete them from \
         PENDING_GEN_R25: {passing:?}"
    );
}
