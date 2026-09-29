//! Stability contract for EVERY registered generator (SPEC-002 GEN-R1..R10, GEN-R12, GEN-R13).
//!
//! Port of the Python `tests/test_generator_contract.py`, extended for animation.
//! New generators are covered automatically once they are in the registry.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use sciart_core::prelude::*;
use sciart_core::title::{self, Form, Title};
use sciart_generators::{Entry, find, registry};

const SEED: u64 = 99;

/// Titles are on by default (CORE-R75), so every check below also covers the title.
fn stage(entry: &Entry, size: Size, mode: GlyphMode) -> Stage {
    Stage::new((entry.build)(), SEED, size, mode)
}

fn check_frame(name: &str, canvas: &Canvas, size: Size) {
    assert_eq!(canvas.size(), size, "{name}: canvas size");
    let lines = canvas.to_lines();
    assert_eq!(
        lines.len(),
        usize::from(size.height()),
        "{name}: line count"
    );
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            line.chars().count(),
            usize::from(size.width()),
            "{name}: line {i} width"
        );
    }
}

/// Run `f` for every generator, collecting panics so one report names all offenders.
fn for_each_generator(test: &str, f: impl Fn(&Entry)) {
    let failures: Vec<_> = registry()
        .iter()
        .filter_map(|e| {
            catch_unwind(AssertUnwindSafe(|| f(e))).err().map(|p| {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(ToString::to_string))
                    .unwrap_or_default();
                format!("{}: {msg}", e.meta.name)
            })
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{test} failed for:\n{}",
        failures.join("\n")
    );
}

#[test]
fn gen_r1_registry_names_unique_sorted_snake_case() {
    let names: Vec<_> = registry().iter().map(|e| e.meta.name).collect();
    assert!(!names.is_empty());
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted, "registry must be sorted and unique");
    for n in &names {
        assert!(
            n.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{n} is not snake_case"
        );
    }
}

#[test]
fn gen_r1_built_generator_reports_its_registry_meta() {
    for e in registry() {
        assert_eq!((e.build)().meta().name, e.meta.name);
    }
}

#[test]
fn gen_r2_find_is_exact_match() {
    assert_eq!(find("starfield").map(|e| e.meta.name), Some("starfield"));
    assert!(find("star").is_none());
    assert!(find("").is_none());
}

#[test]
fn gen_r4_steady_long_run() {
    for_each_generator("steady_long_run", |e| {
        let size = Size::new(200, 50);
        let mut s = stage(e, size, GlyphMode::Unicode);
        for i in 0..600 {
            s.step(SIM_DT);
            if i % 2 == 0 {
                check_frame(e.meta.name, s.render(), size);
            }
        }
    });
}

#[test]
fn gen_r4_r5_resize_sequence() {
    for_each_generator("resize_sequence", |e| {
        let mut s = stage(e, Size::new(200, 50), GlyphMode::Unicode);
        for (w, h) in [
            (200, 50),
            (80, 24),
            (20, 6),
            (3, 2),
            (1, 1),
            (120, 40),
            (1, 60),
            (300, 1),
        ] {
            let size = Size::new(w, h);
            s.resize(size);
            for _ in 0..5 {
                s.step(SIM_DT);
                check_frame(e.meta.name, s.render(), size);
            }
        }
    });
}

#[test]
fn gen_r4_minimal_dims_from_fresh() {
    for_each_generator("minimal_dims", |e| {
        for (w, h) in [(1, 1), (1, 8), (8, 1), (2, 2)] {
            let size = Size::new(w, h);
            let mut s = stage(e, size, GlyphMode::Unicode);
            check_frame(e.meta.name, s.render(), size);
            s.step(0.25);
            check_frame(e.meta.name, s.render(), size);
        }
    });
}

#[test]
fn gen_r4_large_dt_and_zero_dt() {
    for_each_generator("dt_extremes", |e| {
        let size = Size::new(80, 24);
        let mut s = stage(e, size, GlyphMode::Unicode);
        for dt in [0.0, 0.25, 0.0, 0.25, SIM_DT] {
            s.step(dt);
            check_frame(e.meta.name, s.render(), size);
        }
    });
}

#[test]
fn gen_r6_deterministic_for_same_seed() {
    for_each_generator("determinism", |e| {
        let run = || {
            let mut s = stage(e, Size::new(120, 40), GlyphMode::Unicode);
            for _ in 0..90 {
                s.step(SIM_DT);
            }
            s.resize(Size::new(60, 20));
            for _ in 0..30 {
                s.step(SIM_DT);
            }
            s.render().to_lines()
        };
        assert_eq!(run(), run(), "same seed must give identical frames");
    });
}

#[test]
fn gen_r20_frames_do_not_depend_on_render_cadence() {
    // `prepare` rasterises only the simulation state (GEN-R20): a stage drawn after every
    // step and one drawn every third step (the runtime draws every ~2) show identical
    // cells, inks included, whenever both draw; also right after a resize.
    for_each_generator("render cadence", |e| {
        let size = Size::new(90, 30);
        let mut every = stage(e, size, GlyphMode::Unicode);
        let mut sparse = stage(e, size, GlyphMode::Unicode);
        let same = |a: &mut Stage, b: &mut Stage, at: &str| {
            let (fa, fb) = (a.render().cells().to_vec(), b.render().cells().to_vec());
            assert!(fa == fb, "{at}: frame differs with render cadence");
        };
        for i in 1..=75 {
            every.step(SIM_DT);
            every.render();
            sparse.step(SIM_DT);
            if i % 3 == 0 {
                same(&mut every, &mut sparse, "step");
            }
            if i == 40 {
                every.resize(Size::new(61, 23));
                every.render();
                sparse.resize(Size::new(61, 23));
                same(&mut every, &mut sparse, "resize");
            }
        }
    });
}

#[test]
fn gen_r9_scene_moves_within_one_second() {
    for_each_generator("motion", |e| {
        let mut s = stage(e, Size::new(200, 50), GlyphMode::Unicode);
        let first = s.render().to_lines();
        for _ in 0..60 {
            s.step(SIM_DT);
        }
        assert_ne!(first, s.render().to_lines(), "frame unchanged after 1 s");
    });
}

#[test]
fn gen_r10_ascii_mode_is_pure_ascii() {
    for_each_generator("ascii_purity", |e| {
        let mut s = stage(e, Size::new(120, 40), GlyphMode::Ascii);
        for _ in 0..120 {
            s.step(SIM_DT);
            let bad: Vec<char> = s
                .render()
                .cells()
                .iter()
                .map(|c| c.ch)
                .filter(|c| !c.is_ascii())
                .collect();
            assert!(bad.is_empty(), "non-ASCII glyphs in ASCII mode: {bad:?}");
        }
    });
}

/// Chars that would turn a title into a formula or markup (GEN-R12).
const FORMULA: &[char] = &['=', '^', '_', '*', '/', '<', '>'];

fn check_title_chars(s: &str) {
    for c in s.chars() {
        assert!(
            title::ascii_of(c).is_some() && !FORMULA.contains(&c),
            "{c:?} is not allowed in a title: {s:?}"
        );
    }
}

fn check_part(what: &str, s: &str) {
    assert!(
        !s.is_empty() && s.trim() == s,
        "{what} must be non-empty and trimmed: {s:?}"
    );
}

#[test]
fn gen_r12_every_title_and_caption_is_valid() {
    for_each_generator("titles", |e| {
        let meta = e.meta;
        assert_eq!(
            meta.title.matches(title::SEPARATOR).count(),
            1,
            "exactly one ` · ` in {:?}",
            meta.title
        );
        check_title_chars(meta.title);
        let t = Title::new(meta.title, None);
        check_part("phenomenon", t.phenomenon);
        check_part("mechanism", t.mechanism);
        assert!(t.width(Form::Full) <= 32, "{:?} over 32 cells", meta.title);
        assert!(
            t.width(Form::Phenomenon) <= 20,
            "{:?} over 20 cells",
            t.phenomenon
        );
        for &caption in meta.captions {
            check_part("caption", caption);
            check_title_chars(caption);
            assert!(!caption.contains(title::SEPARATOR), "{caption:?}");
            let live = Title::new(meta.title, Some(caption));
            assert!(live.width(Form::Full) <= 32, "{live:?} over 32 cells");
        }
    });
}

#[test]
fn gen_r12_titles_are_unique() {
    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in registry() {
        owners.entry(e.meta.title).or_default().push(e.meta.name);
    }
    owners.retain(|_, names| names.len() > 1);
    assert!(owners.is_empty(), "shared titles: {owners:?}");
}

#[test]
fn gen_r12_live_captions_are_declared() {
    for_each_generator("captions", |e| {
        let size = Size::new(80, 24);
        let mut ctx = Ctx::new(42, size);
        let mut g = (e.build)();
        g.resize(size, &mut ctx);
        let mut steps = 0;
        for secs in [0, 1, 5, 20] {
            while steps < secs * 60 {
                g.update(SIM_DT, &mut ctx);
                ctx.advance(SIM_DT);
                steps += 1;
            }
            if let Some(caption) = g.caption() {
                assert!(
                    e.meta.captions.contains(&caption),
                    "caption {caption:?} at {secs} s is not in meta.captions"
                );
            }
        }
    });
}

#[test]
fn gen_r13_core_r76_title_row_readouts_use_the_readout_level() {
    // Title hidden, so every text char on row h − 1 is a scene readout; one shared level
    // keeps the numbers below the title's phenomenon however the scene's art is inked.
    for_each_generator("readout level", |e| {
        for size in [Size::new(80, 24), Size::new(40, 12), Size::new(200, 50)] {
            let mut stage = Stage::new((e.build)(), 42, size, GlyphMode::Unicode);
            stage.set_title_visible(false);
            for _ in 0..60 {
                stage.step(SIM_DT);
            }
            let y = i32::from(size.height()) - 1;
            let canvas = stage.render();
            for x in 0..i32::from(size.width()) {
                let cell = canvas.get(x, y).unwrap();
                if cell.ink.role == Role::Text && cell.ch.is_alphanumeric() {
                    assert_eq!(
                        cell.ink.level,
                        title::READOUT_LEVEL,
                        "{size:?} column {x} {:?}",
                        cell.ch
                    );
                }
            }
        }
    });
}

#[test]
fn gen_r13_no_scene_text_in_the_title_span_or_its_readout_gap() {
    // Title hidden, so any text left there is the scene's own; Unicode only, since ASCII
    // art fallbacks may use letters. The checked cells run from the span to where the
    // longest title-row readout may start, so the gap (CORE-R76) is covered too.
    for_each_generator("title span", |e| {
        for size in [Size::new(80, 24), Size::new(40, 12)] {
            let mut stage = Stage::new((e.build)(), 42, size, GlyphMode::Unicode);
            stage.set_title_visible(false);
            let mut steps = 0;
            for secs in [0, 1, 5] {
                while steps < secs * 60 {
                    stage.step(SIM_DT);
                    steps += 1;
                }
                let title = stage.title();
                let canvas = stage.render();
                let Some((y, _)) = title::span(size, &title) else {
                    continue;
                };
                let text_from = i32::from(size.width())
                    - 1
                    - i32::try_from(title::readout_room(size, &title)).unwrap();
                let row: String = (0..text_from)
                    .map(|x| canvas.get(x, y).map_or(' ', |c| c.ch))
                    .collect();
                assert!(
                    !row.chars().any(char::is_alphanumeric),
                    "{size:?} at {secs} s: {row:?}"
                );
            }
        }
    });
}
