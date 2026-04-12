//! Terminal-free runtime state machine (SPEC-003 RT-R1..R4, RT-R7; ADR-0006).

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sciart_core::prelude::*;
use sciart_generators::Entry;

/// Simulation steps allowed per frame before the backlog is dropped (ADR-0003).
const MAX_STEPS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    AllFailed,
}

thread_local! {
    static CAPTURING: Cell<bool> = const { Cell::new(false) };
}

/// Wrap the current panic hook so panics caught by [`guarded`] stay silent and don't
/// trigger ratatui's terminal-restoring hook. Install *after* `ratatui::init`.
pub fn install_panic_filter() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !CAPTURING.with(Cell::get) {
            prev(info);
        }
    }));
}

/// Run `f`, converting a panic into its message (RT-R2).
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    CAPTURING.with(|c| c.set(true));
    let result = catch_unwind(AssertUnwindSafe(f));
    CAPTURING.with(|c| c.set(false));
    result.map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "unknown panic".to_string())
    })
}

#[derive(Debug)]
pub struct Runtime {
    entries: Vec<&'static Entry>,
    quarantined: Vec<bool>,
    current: Option<usize>,
    stage: Option<Stage>,
    /// Copy of the last complete frame; survives generator panics (RT-R3).
    frame: Canvas,
    clock: FixedStep,
    rng: ChaCha8Rng,
    size: Size,
    mode: GlyphMode,
    status: Status,
    notices: Vec<String>,
}

impl Runtime {
    /// `start`: index into `entries` to show first; `None` picks a random one.
    pub fn new(
        entries: Vec<&'static Entry>,
        seed: u64,
        size: Size,
        mode: GlyphMode,
        start: Option<usize>,
    ) -> Self {
        let mut rt = Self {
            quarantined: vec![false; entries.len()],
            entries,
            current: None,
            stage: None,
            frame: Canvas::new(size, mode),
            clock: FixedStep::new(SIM_DT, MAX_STEPS),
            rng: ChaCha8Rng::seed_from_u64(seed),
            size,
            mode,
            status: Status::Running,
            notices: Vec::new(),
        };
        match start {
            Some(i) if i < rt.entries.len() => rt.activate(i),
            _ => rt.switch_to_next(),
        }
        rt
    }

    /// Advance by `elapsed` wall seconds and render a new frame (RT-R2).
    pub fn tick(&mut self, elapsed: f64) {
        let Some(stage) = self.stage.as_mut() else {
            return;
        };
        let steps = self.clock.advance(elapsed);
        let frame = &mut self.frame;
        let outcome = guarded(|| {
            for _ in 0..steps {
                stage.step(SIM_DT);
            }
            frame.clone_from(stage.render());
        });
        if let Err(msg) = outcome {
            self.fail_current(&msg);
        }
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.frame.resize(size);
        let Some(stage) = self.stage.as_mut() else {
            return;
        };
        if let Err(msg) = guarded(|| stage.resize(size)) {
            self.fail_current(&msg);
        }
    }

    pub fn frame(&self) -> &Canvas {
        &self.frame
    }

    pub fn status(&self) -> Status {
        self.status
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "shown by the HUD in M2 (F4)"))]
    pub fn current_name(&self) -> Option<&'static str> {
        self.current.map(|i| self.entries[i].meta.name)
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "shown by the HUD in M2 (F4)"))]
    pub fn quarantined(&self) -> usize {
        self.quarantined.iter().filter(|q| **q).count()
    }

    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// Build a fresh instance of entry `i` (RT-R7); a panicking build is quarantined.
    fn activate(&mut self, i: usize) {
        let entry = self.entries[i];
        let seed = self.rng.random();
        let (size, mode) = (self.size, self.mode);
        self.clock.reset();
        match guarded(|| Stage::new((entry.build)(), seed, size, mode)) {
            Ok(stage) => {
                self.current = Some(i);
                self.stage = Some(stage);
            }
            Err(msg) => {
                self.current = Some(i);
                self.fail_current(&msg);
            }
        }
    }

    /// Quarantine the current generator and move on (RT-R3, RT-R4).
    fn fail_current(&mut self, msg: &str) {
        if let Some(i) = self.current.take() {
            self.quarantined[i] = true;
            let name = self.entries[i].meta.name;
            self.notices
                .push(format!("{name} panicked: {msg} (quarantined)"));
        }
        self.stage = None;
        self.switch_to_next();
    }

    /// Activate a random healthy generator other than the current one when possible.
    fn switch_to_next(&mut self) {
        let healthy: Vec<usize> = (0..self.entries.len())
            .filter(|&i| !self.quarantined[i])
            .collect();
        let candidates: Vec<usize> = match self.current {
            Some(cur) if healthy.len() > 1 => healthy.into_iter().filter(|&i| i != cur).collect(),
            _ => healthy,
        };
        if candidates.is_empty() {
            self.current = None;
            self.stage = None;
            self.status = Status::AllFailed;
            self.notices
                .push("all generators failed - press q to quit".to_string());
            return;
        }
        let pick = candidates[self.rng.random_range(0..candidates.len())];
        self.activate(pick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- test generators -----

    /// Renders its step count; panics in `update` once it reaches `boom_at` (0 = never).
    #[derive(Debug)]
    struct Counter {
        meta: &'static Meta,
        steps: u32,
        boom_at: u32,
    }

    impl Generator for Counter {
        fn meta(&self) -> &'static Meta {
            self.meta
        }
        fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {}
        fn update(&mut self, _dt: f64, _ctx: &mut Ctx) {
            self.steps += 1;
            assert!(
                self.boom_at == 0 || self.steps < self.boom_at,
                "boom at {}",
                self.steps
            );
        }
        fn render(&self, canvas: &mut Canvas) {
            canvas.text(0, 0, &self.steps.to_string(), Ink::new(Role::Text, 255));
        }
    }

    #[derive(Debug)]
    struct BadResize;

    impl Generator for BadResize {
        fn meta(&self) -> &'static Meta {
            &BAD_RESIZE_META
        }
        fn resize(&mut self, _size: Size, _ctx: &mut Ctx) {
            panic!("cannot resize");
        }
        fn update(&mut self, _dt: f64, _ctx: &mut Ctx) {}
        fn render(&self, _canvas: &mut Canvas) {}
    }

    const fn meta(name: &'static str) -> Meta {
        Meta {
            name,
            category: Category::Basic,
            description: "test",
        }
    }
    static OK_META: Meta = meta("ok");
    static BOOM_META: Meta = meta("boom");
    static LATE_META: Meta = meta("late_boom");
    static BAD_RESIZE_META: Meta = meta("bad_resize");

    static OK: Entry = Entry {
        meta: &OK_META,
        build: || {
            Box::new(Counter {
                meta: &OK_META,
                steps: 0,
                boom_at: 0,
            })
        },
    };
    static BOOM: Entry = Entry {
        meta: &BOOM_META,
        build: || {
            Box::new(Counter {
                meta: &BOOM_META,
                steps: 0,
                boom_at: 1,
            })
        },
    };
    static LATE: Entry = Entry {
        meta: &LATE_META,
        build: || {
            Box::new(Counter {
                meta: &LATE_META,
                steps: 0,
                boom_at: 10,
            })
        },
    };
    static BAD_RESIZE: Entry = Entry {
        meta: &BAD_RESIZE_META,
        build: || Box::new(BadResize),
    };

    fn rt(entries: &[&'static Entry], start: usize) -> Runtime {
        Runtime::new(
            entries.to_vec(),
            1,
            Size::new(8, 2),
            GlyphMode::Ascii,
            Some(start),
        )
    }

    fn first_line(rt: &Runtime) -> String {
        rt.frame().to_lines()[0].trim_end().to_string()
    }

    // ----- tests -----

    #[test]
    fn rt_r2_tick_runs_fixed_steps_then_renders() {
        let mut r = rt(&[&OK], 0);
        r.tick(3.0 * SIM_DT);
        assert_eq!(first_line(&r), "3");
        r.tick(100.0); // clamped to max_steps = 5 (CORE-R10)
        assert_eq!(first_line(&r), "8");
        assert_eq!(r.status(), Status::Running);
    }

    #[test]
    fn rt_r3_panic_quarantines_and_switches_to_next_healthy() {
        let mut r = rt(&[&BOOM, &OK], 0);
        assert_eq!(r.current_name(), Some("boom"));
        r.tick(SIM_DT);
        assert_eq!(r.current_name(), Some("ok"));
        assert_eq!(r.quarantined(), 1);
        assert!(r.notices().iter().any(|n| n.contains("boom")));
        r.tick(SIM_DT);
        assert_eq!(first_line(&r), "1");
    }

    #[test]
    fn rt_r3_last_good_frame_survives_a_panic() {
        let mut r = rt(&[&LATE], 0);
        for _ in 0..9 {
            r.tick(SIM_DT);
        }
        assert_eq!(first_line(&r), "9");
        r.tick(SIM_DT); // step 10 panics
        assert_eq!(r.status(), Status::AllFailed);
        assert_eq!(first_line(&r), "9", "last good frame kept");
    }

    #[test]
    fn rt_r4_all_failed_makes_tick_a_noop() {
        let mut r = rt(&[&BOOM, &BAD_RESIZE], 0);
        r.tick(SIM_DT);
        assert_eq!(r.status(), Status::AllFailed);
        assert_eq!(r.quarantined(), 2);
        assert_eq!(r.current_name(), None);
        r.tick(SIM_DT);
        r.resize(Size::new(3, 3));
    }

    #[test]
    fn rt_r7_panicking_build_is_quarantined_and_skipped() {
        let r = rt(&[&BAD_RESIZE, &OK], 0);
        assert_eq!(r.current_name(), Some("ok"));
        assert_eq!(r.quarantined(), 1);
    }

    #[test]
    fn rt_r1_resize_reaches_frame_and_generator() {
        let mut r = rt(&[&OK], 0);
        r.resize(Size::new(10, 3));
        r.tick(SIM_DT);
        assert_eq!(r.frame().size(), Size::new(10, 3));
    }

    #[test]
    fn random_start_is_deterministic_for_a_seed() {
        let pick = |seed| {
            Runtime::new(
                vec![&OK, &LATE],
                seed,
                Size::new(4, 1),
                GlyphMode::Ascii,
                None,
            )
            .current_name()
        };
        assert_eq!(pick(5), pick(5));
    }
}
