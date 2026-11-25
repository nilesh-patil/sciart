//! GEN-R22 (ADR-0018): the 41 scenes registered at M11 (`627cde6`) render as they did at
//! M11. The reference is `tests/fixtures/m11_frames.txt`, written from the M11 tree by
//! `examples/inkframes.rs` (the same file builds in both trees; this test includes it, so
//! fixture and live frames share one encoder):
//!
//! ```text
//! cargo run -q --release -p sciart-generators --example inkframes > tests/fixtures/m11_frames.txt
//! ```
//!
//! (run in the M11 tree). Seed 42, 80×24, Unicode and ASCII, t = 0, 1, 5 s; every frame
//! must match the reference in at least 99.5 % of its cells (glyph, fg and bg ink role and
//! level). The port to deterministic trigonometry (GEN-R6) may move a last bit, which is
//! what the 0.5 % allows. The M11 fixture was written by a release build; a debug build of
//! the M11 tree differs from it only in `molecular_dynamics` at t = 5 s (chaotic MD).
//!
//! One core rule changed after M11 and is applied to the reference before comparing:
//! title-row readouts are inked at `title::READOUT_LEVEL` (90 since M14, CORE-R76; 110 at
//! M11), so a reference cell on the title row, outside the title's own span, inked
//! `Text` 110 is expected at `READOUT_LEVEL` (`m11_readout_to_today`).

//!
//! The scenes GEN-R25 restyles to the thread idiom (M16, ADR-0019) and GEN-R26 to the
//! orbital-thread idiom (M17, ADR-0020) are no longer pinned: their frames are skipped
//! (`gen_r25::is_restyled`, the lists `thread_style.rs` and `orbital_style.rs` check
//! against SPEC-002).

#[path = "../examples/inkframes.rs"]
#[allow(dead_code)]
mod inkframes;

#[path = "common/gen_r25.rs"]
mod gen_r25;

#[path = "common/tier.rs"]
mod tier;

use gen_r25::is_restyled;
use inkframes::{CellKey, HEIGHT, MODES, WIDTH, decode, frame_keys, header, render_frames};
use sciart_core::prelude::*;
use sciart_core::title;

const FIXTURE: &str = include_str!("fixtures/m11_frames.txt");

/// Minimum share of equal cells per frame.
const MIN_EQUAL: f64 = 0.995;

/// M11's title-row readout level (`title::READOUT_LEVEL` at `627cde6`).
const M11_READOUT_LEVEL: u8 = 110;

/// The reference frame with M11's title-row readouts moved to today's CORE-R76 level:
/// cells on the last row outside `title_cols` inked `Text` at M11's readout level.
fn m11_readout_to_today(
    want: &[CellKey],
    title_cols: Option<std::ops::Range<i32>>,
) -> Vec<CellKey> {
    let w = usize::from(WIDTH);
    let last_row = want.len() - w;
    want.iter()
        .enumerate()
        .map(|(i, &(ch, fg, bg))| {
            let col = (i % w) as i32;
            let in_title = title_cols.as_ref().is_some_and(|r| r.contains(&col));
            let fg = if i >= last_row && !in_title && fg == ('X', M11_READOUT_LEVEL) {
                ('X', title::READOUT_LEVEL)
            } else {
                fg
            };
            (ch, fg, bg)
        })
        .collect()
}

/// Frames allowed to differ from the M11 reference: (scene, from simulated second, reason);
/// the scene's frames at t >= that second are exempt, earlier ones still count. Adding
/// one is a spec change, like a snapshot change.
const EXCEPTIONS: &[(&str, u32, &str)] = &[(
    "molecular_dynamics",
    5,
    "chaotic Lennard-Jones MD: the last-bit difference of the deterministic trigonometry \
     (GEN-R6) grows into different trajectories by t = 5 s; M11 itself renders this frame \
     differently in debug and release builds",
)];

fn exempt(scene: &str, t: u32) -> bool {
    EXCEPTIONS
        .iter()
        .any(|&(s, from, _)| s == scene && t >= from)
}

#[test]
fn gen_r22_m11_scenes_match_the_m11_ink_frames() {
    // Frames are pinned on tier 1 only (TEST-R13, ADR-0029).
    if tier::skip_off_tier_1("m11_fidelity") {
        return;
    }
    let (w, h) = (usize::from(WIDTH), usize::from(HEIGHT));
    let reference = decode(FIXTURE, w, h).expect("fixture decodes");
    assert_eq!(reference.len(), 41 * MODES.len() * 3, "fixture frame count");
    let mut scenes: Vec<&str> = reference
        .iter()
        .filter_map(|(head, _)| head.split(' ').nth(1))
        .collect();
    scenes.dedup();
    assert_eq!(scenes.len(), 41, "fixture scene count");
    let expected: std::collections::HashMap<&str, &Vec<_>> =
        reference.iter().map(|(hd, c)| (hd.as_str(), c)).collect();

    // (worst share equal, scene, frame header, cells differing)
    let mut results: Vec<(f64, String, String, usize)> = Vec::new();
    let mut missing = Vec::new();
    for scene in &scenes {
        // GEN-R25/R26 (ADR-0019, ADR-0020): restyled, no longer pinned to M11.
        if is_restyled(scene) {
            continue;
        }
        let Some(entry) = sciart_generators::find(scene) else {
            missing.push(*scene);
            continue;
        };
        let mut worst = (1.0_f64, String::new(), 0_usize);
        for (mode, m) in MODES {
            render_frames(entry.build, mode, |t, stage| {
                let head = header(scene, m, t);
                let cols = title::span(Size::new(WIDTH, HEIGHT), &stage.title()).map(|s| s.1);
                let want = m11_readout_to_today(expected[head.as_str()], cols);
                let got = frame_keys(stage.frame());
                let diff = want.iter().zip(&got).filter(|(a, b)| a != b).count();
                let share = 1.0 - diff as f64 / want.len() as f64;
                if exempt(scene, t) {
                    return;
                }
                if share < worst.0 || worst.1.is_empty() {
                    worst = (share, head, diff);
                }
            });
        }
        results.push((worst.0, (*scene).to_owned(), worst.1, worst.2));
    }
    assert!(missing.is_empty(), "M11 scenes not registered: {missing:?}");

    results.sort_by(|a, b| a.0.total_cmp(&b.0));
    let failing: Vec<_> = results.iter().filter(|r| r.0 < MIN_EQUAL).collect();
    let report = |rows: &[&(f64, String, String, usize)]| {
        rows.iter()
            .map(|(share, _, head, diff)| {
                format!(
                    "  {:6.2} % equal ({diff} cells differ) in {head}",
                    100.0 * share
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        failing.is_empty(),
        "GEN-R22: {} of 41 M11 scenes differ from the M11 reference (need >= {:.1} % equal \
         cells in every frame); worst frame per scene:\n{}",
        failing.len(),
        100.0 * MIN_EQUAL,
        report(&failing)
    );
}

#[test]
fn gen_r22_exceptions_name_m11_scenes_with_a_reason() {
    let (w, h) = (usize::from(WIDTH), usize::from(HEIGHT));
    let reference = decode(FIXTURE, w, h).expect("fixture decodes");
    for (scene, _, reason) in EXCEPTIONS {
        assert!(
            !reason.trim().is_empty(),
            "{scene}: exception without a reason"
        );
        assert!(
            reference
                .iter()
                .any(|(head, _)| head.split(' ').nth(1) == Some(*scene)),
            "{scene}: not an M11 scene"
        );
    }
}

#[test]
fn gen_r22_encoding_round_trips() {
    let (w, h) = (usize::from(WIDTH), usize::from(HEIGHT));
    let entry = &sciart_generators::registry()[0];
    render_frames(entry.build, MODES[0].0, |t, stage| {
        let keys = frame_keys(stage.frame());
        let mut text = String::new();
        inkframes::encode(&mut text, &header(entry.meta.name, 'u', t), w, &keys);
        let back = decode(&text, w, h).expect("decodes");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].1, keys);
    });
}
