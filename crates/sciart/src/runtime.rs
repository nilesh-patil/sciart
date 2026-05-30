//! Terminal-free runtime state machine (SPEC-003 RT-R1..R7, RT-R12, RT-R13; ADR-0006).

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sciart_core::prelude::*;
use sciart_core::title::Title;
use sciart_generators::Entry;
use sciart_render::Theme;

/// Simulation steps allowed per frame before the backlog is dropped (ADR-0003).
const MAX_STEPS: u32 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    AllFailed,
}

/// User intents (RT-R6); produced by `keys::action_for`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Next,
    Prev,
    TogglePause,
    Reset,
    ToggleHud,
    CycleTheme,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
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
    /// `sim`'s walk through `entries`, as indices into it (RT-R13).
    order: Vec<usize>,
    /// Position in `order` of the entry activated last. It outlives a quarantine of that
    /// entry, so the walk resumes after it.
    at: Option<usize>,
    quarantined: Vec<bool>,
    current: Option<usize>,
    stage: Option<Stage>,
    /// Copy of the last complete frame; survives generator panics (RT-R3).
    frame: Canvas,
    /// See [`Runtime::epoch`].
    epoch: u64,
    clock: FixedStep,
    rng: ChaCha8Rng,
    size: Size,
    mode: GlyphMode,
    status: Status,
    notices: Vec<String>,
    /// Auto-cycle period in wall seconds (RT-R5); `None` = never.
    interval: Option<f64>,
    since_switch: f64,
    /// Indices shown, oldest first; the last one is normally `current` (RT-R5a).
    history: Vec<usize>,
    paused: bool,
    hud: bool,
    /// The `title` setting (CFG-R6, CLI-R7).
    title: bool,
    theme: Theme,
}

const HISTORY_LEN: usize = 64;

impl Runtime {
    /// `start`: index into `entries` to show first; `None` starts the cycle order (RT-R13).
    pub fn new(
        entries: Vec<&'static Entry>,
        seed: u64,
        size: Size,
        mode: GlyphMode,
        start: Option<usize>,
    ) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let categories: Vec<Category> = entries.iter().map(|e| e.meta.category).collect();
        let order = cycle_order(&categories, &mut rng);
        let mut rt = Self {
            quarantined: vec![false; entries.len()],
            entries,
            order,
            at: None,
            current: None,
            stage: None,
            frame: Canvas::new(size, mode),
            epoch: 0,
            clock: FixedStep::new(SIM_DT, MAX_STEPS),
            rng,
            size,
            mode,
            status: Status::Running,
            notices: Vec::new(),
            interval: None,
            since_switch: 0.0,
            history: Vec::new(),
            paused: false,
            hud: false,
            title: true,
            theme: Theme::default(),
        };
        match start {
            Some(i) if i < rt.entries.len() => rt.activate(i, true),
            _ => rt.switch_to_next(),
        }
        rt
    }

    /// Advance by `elapsed` wall seconds, auto-cycling if due, and render a new frame
    /// (RT-R2, RT-R5). Does nothing while paused.
    pub fn tick(&mut self, elapsed: f64) {
        if self.paused || self.stage.is_none() {
            return;
        }
        if elapsed.is_finite() && elapsed > 0.0 {
            self.since_switch += elapsed;
        }
        if let Some(interval) = self.interval
            && self.since_switch >= interval
            && self.healthy_count() > 1
        {
            self.switch_to_next();
        }
        let Some(stage) = self.stage.as_mut() else {
            return;
        };
        let steps = self.clock.advance(elapsed);
        // The stage keeps its last complete frame (CORE-R21), which `frame()` shows, so
        // no per-tick copy is needed (RT-R14).
        let outcome = guarded(|| {
            for _ in 0..steps {
                stage.step(SIM_DT);
            }
            stage.render();
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

    /// The frame to show: the stage's last complete frame, or with no stage (RT-R4) the
    /// last good frame kept when the stage was dropped.
    pub fn frame(&self) -> &Canvas {
        self.stage.as_ref().map_or(&self.frame, Stage::frame)
    }

    /// Increments whenever the shown generator is rebuilt or dropped (switch, `Reset`,
    /// quarantine): the painter's colour memory must not outlive it (RND-R14).
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Keep the stage's last complete frame before the stage goes away, unless the stage
    /// never got to the current size (a panicking `resize`): then the blank frame that
    /// `resize` left stays, as before.
    fn keep_frame(&mut self) {
        if let Some(stage) = &self.stage
            && stage.frame().size() == self.size
        {
            self.frame.clone_from(stage.frame());
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn current_name(&self) -> Option<&'static str> {
        self.current.map(|i| self.entries[i].meta.name)
    }

    pub fn quarantined(&self) -> usize {
        self.quarantined.iter().filter(|q| **q).count()
    }

    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    pub fn with_interval(mut self, secs: f64) -> Self {
        self.interval = Some(secs);
        self
    }

    pub fn with_hud(mut self, on: bool) -> Self {
        self.hud = on;
        self.show_title();
        self
    }

    /// The `title` setting (CFG-R6, CLI-R7).
    pub fn with_title(mut self, on: bool) -> Self {
        self.title = on;
        self.show_title();
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Apply a user action (RT-R6).
    pub fn handle(&mut self, action: Action) -> Flow {
        let generator_action = matches!(
            action,
            Action::Next | Action::Prev | Action::Reset | Action::TogglePause
        );
        if generator_action && self.status == Status::AllFailed {
            return Flow::Continue;
        }
        match action {
            Action::Quit => return Flow::Quit,
            Action::Next => self.switch_to_next(),
            Action::Prev => self.prev(),
            Action::Reset => {
                if let Some(i) = self.current {
                    self.activate(i, false);
                }
            }
            Action::TogglePause => self.paused = !self.paused,
            Action::ToggleHud => {
                self.hud = !self.hud;
                self.show_title();
            }
            Action::CycleTheme => self.theme = self.theme.next(),
        }
        Flow::Continue
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn hud(&self) -> bool {
        self.hud
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// The live science title for the HUD; `None` with no stage or the setting off (RT-R12).
    pub fn title(&self) -> Option<Title> {
        self.stage.as_ref().filter(|_| self.title).map(Stage::title)
    }

    /// The canvas title shows iff the setting is on and the HUD, which shows it instead,
    /// is off (RT-R12).
    fn title_visible(&self) -> bool {
        self.title && !self.hud
    }

    /// Apply [`Self::title_visible`] to the stage and re-render without stepping, so a
    /// paused frame neither keeps a title the HUD now shows nor lacks one (RT-R12).
    fn show_title(&mut self) {
        let visible = self.title_visible();
        let Some(stage) = self.stage.as_mut() else {
            return;
        };
        stage.set_title_visible(visible);
        if let Err(msg) = guarded(|| {
            stage.render();
        }) {
            self.fail_current(&msg);
        }
    }

    fn healthy_count(&self) -> usize {
        self.quarantined.iter().filter(|q| !**q).count()
    }

    /// Go back to the most recent earlier non-quarantined generator (RT-R5a).
    fn prev(&mut self) {
        let mut history = self.history.clone();
        if history.last().copied() == self.current {
            history.pop();
        }
        while history.last().is_some_and(|&j| self.quarantined[j]) {
            history.pop();
        }
        if let Some(&j) = history.last() {
            self.history = history;
            self.activate(j, false);
        }
    }

    /// Build a fresh instance of entry `i` and draw its first frame (RT-R7);
    /// a panicking build is quarantined. `push` records it in the history.
    fn activate(&mut self, i: usize, push: bool) {
        let entry = self.entries[i];
        let seed = self.rng.random();
        let (size, mode) = (self.size, self.mode);
        let title_visible = self.title_visible();
        self.at = self.order.iter().position(|&j| j == i);
        self.clock.reset();
        self.since_switch = 0.0;
        self.epoch += 1;
        // If the new stage fails before its first frame, the old one's stays up (RT-R3).
        self.keep_frame();
        let built = guarded(|| {
            let mut stage = Stage::new((entry.build)(), seed, size, mode);
            stage.set_title_visible(title_visible);
            stage.render();
            stage
        });
        match built {
            Ok(stage) => {
                self.current = Some(i);
                self.stage = Some(stage);
                if push {
                    self.history.push(i);
                    if self.history.len() > HISTORY_LEN {
                        self.history.remove(0);
                    }
                }
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
        self.keep_frame();
        self.stage = None;
        self.epoch += 1;
        self.switch_to_next();
    }

    /// Activate the next healthy entry after the last one in the cycle order, wrapping
    /// (RT-R13). The current entry comes last in that walk, so it is rebuilt only when it
    /// is the sole healthy one.
    fn switch_to_next(&mut self) {
        let n = self.order.len();
        let from = self.at.map_or(0, |p| p + 1);
        let next = (0..n)
            .map(|k| self.order[(from + k) % n])
            .find(|&i| !self.quarantined[i]);
        let Some(next) = next else {
            self.current = None;
            self.stage = None;
            self.status = Status::AllFailed;
            self.notices
                .push("all generators failed - press q to quit".to_string());
            return;
        };
        self.activate(next, true);
    }
}

/// `sim`'s cycle order over entries with the given `categories`, as indices (RT-R13). The
/// categories and each category's entries are shuffled, then dealt round-robin, one entry
/// per category per round, so neighbours differ in category until only one is left.
fn cycle_order(categories: &[Category], rng: &mut impl Rng) -> Vec<usize> {
    let mut groups: Vec<Vec<usize>> = Category::ALL
        .into_iter()
        .map(|c| {
            (0..categories.len())
                .filter(|&i| categories[i] == c)
                .collect()
        })
        .filter(|group: &Vec<usize>| !group.is_empty())
        .collect();
    groups.shuffle(rng);
    for group in &mut groups {
        group.shuffle(rng);
    }
    let rounds = groups.iter().map(Vec::len).max().unwrap_or(0);
    (0..rounds)
        .flat_map(|round| groups.iter().filter_map(move |g| g.get(round).copied()))
        .collect()
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
        meta_in(name, Category::Physics)
    }
    const fn meta_in(name: &'static str, category: Category) -> Meta {
        Meta {
            name,
            category,
            description: "test",
            title: "Test scene · fixture",
            captions: &[],
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
        r.tick(100.0); // clamped to MAX_STEPS = 15 (CORE-R10, RT-R11)
        assert_eq!(first_line(&r), "18");
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
    fn rt_r13_start_is_deterministic_for_a_seed() {
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

    // ----- M2: auto-cycle, history, actions -----

    static OK2_META: Meta = meta("ok2");
    static OK3_META: Meta = meta("ok3");
    static OK2: Entry = Entry {
        meta: &OK2_META,
        build: || {
            Box::new(Counter {
                meta: &OK2_META,
                steps: 0,
                boom_at: 0,
            })
        },
    };
    static OK3: Entry = Entry {
        meta: &OK3_META,
        build: || {
            Box::new(Counter {
                meta: &OK3_META,
                steps: 0,
                boom_at: 0,
            })
        },
    };

    #[test]
    fn rt_r11_low_fps_frames_keep_full_simulation_speed() {
        let mut r = rt(&[&OK], 0);
        r.tick(0.2); // one 5 FPS frame = 12 simulation steps
        assert_eq!(first_line(&r), "12");
    }

    #[test]
    fn rt_r7_switch_renders_immediately() {
        let r = rt(&[&OK], 0);
        assert_eq!(first_line(&r), "0", "first frame drawn before any tick");
    }

    #[test]
    fn rt_r5_auto_cycles_after_interval() {
        let mut r = rt(&[&OK, &OK2], 0).with_interval(1.0);
        r.tick(0.5);
        assert_eq!(r.current_name(), Some("ok"));
        r.tick(0.6);
        assert_eq!(r.current_name(), Some("ok2"));
    }

    #[test]
    fn rt_r5_pause_freezes_the_cycle_timer() {
        let mut r = rt(&[&OK, &OK2], 0).with_interval(1.0);
        r.handle(Action::TogglePause);
        r.tick(5.0);
        assert_eq!(r.current_name(), Some("ok"));
        r.handle(Action::TogglePause);
        r.tick(0.5);
        assert_eq!(r.current_name(), Some("ok"));
        r.tick(0.6);
        assert_eq!(r.current_name(), Some("ok2"));
    }

    #[test]
    fn rt_r5_single_healthy_generator_never_cycles() {
        let mut r = rt(&[&OK], 0).with_interval(0.1);
        r.tick(1.0);
        r.tick(1.0);
        assert_eq!(first_line(&r), "30", "not rebuilt by the cycle timer");
    }

    #[test]
    fn rt_r5_manual_switch_restarts_the_timer() {
        let mut r = rt(&[&OK, &OK2], 0).with_interval(1.0);
        r.tick(0.9);
        r.handle(Action::Next);
        assert_eq!(r.current_name(), Some("ok2"));
        r.tick(0.5);
        assert_eq!(r.current_name(), Some("ok2"));
    }

    #[test]
    fn rt_r5a_prev_walks_back_through_history() {
        let mut r = rt(&[&OK, &OK2, &OK3], 0);
        let mut shown = vec![r.current_name()];
        for _ in 0..2 {
            r.handle(Action::Next);
            shown.push(r.current_name());
        }
        assert_ne!(shown[1], shown[0]);
        assert_ne!(shown[2], shown[1]);
        r.handle(Action::Prev);
        assert_eq!(r.current_name(), shown[1]);
        r.handle(Action::Prev);
        assert_eq!(r.current_name(), shown[0]);
        r.handle(Action::Prev);
        assert_eq!(r.current_name(), shown[0], "no earlier history: no-op");
    }

    #[test]
    fn rt_r5a_prev_skips_quarantined_entries() {
        let mut r = rt(&[&OK, &BOOM], 0);
        r.handle(Action::Next);
        assert_eq!(r.current_name(), Some("boom"));
        r.tick(SIM_DT); // boom panics, runtime moves back to ok
        assert_eq!(r.current_name(), Some("ok"));
        r.handle(Action::Prev);
        assert_eq!(r.current_name(), Some("ok"));
        assert_eq!(r.quarantined(), 1);
    }

    #[test]
    fn rt_r6_pause_freezes_simulation_and_next_still_draws() {
        let mut r = rt(&[&OK, &OK2], 0);
        r.tick(SIM_DT);
        assert_eq!(r.handle(Action::TogglePause), Flow::Continue);
        assert!(r.paused());
        r.tick(1.0);
        assert_eq!(first_line(&r), "1");
        r.handle(Action::Next);
        assert_eq!(r.current_name(), Some("ok2"));
        assert_eq!(first_line(&r), "0", "new scene visible while paused");
        assert!(r.paused());
    }

    #[test]
    fn rt_r6_reset_rebuilds_the_current_generator() {
        let mut r = rt(&[&OK, &OK2], 0);
        r.tick(3.0 * SIM_DT);
        assert_eq!(first_line(&r), "3");
        r.handle(Action::Reset);
        assert_eq!(r.current_name(), Some("ok"));
        assert_eq!(first_line(&r), "0");
    }

    #[test]
    fn rt_r6_view_actions_and_quit() {
        let mut r = rt(&[&OK], 0).with_hud(false).with_theme(Theme::Astro);
        assert!(!r.hud());
        r.handle(Action::ToggleHud);
        assert!(r.hud());
        r.handle(Action::CycleTheme);
        assert_eq!(r.theme(), Theme::Astro.next());
        assert_eq!(r.handle(Action::Quit), Flow::Quit);
    }

    #[test]
    fn rt_r6_generator_actions_are_noops_when_all_failed() {
        let mut r = rt(&[&BOOM], 0);
        r.tick(SIM_DT);
        assert_eq!(r.status(), Status::AllFailed);
        for a in [
            Action::Next,
            Action::Prev,
            Action::Reset,
            Action::TogglePause,
        ] {
            assert_eq!(r.handle(a), Flow::Continue);
        }
        assert_eq!(r.status(), Status::AllFailed);
        assert_eq!(r.current_name(), None);
    }

    // ----- M11: science title (RT-R12) -----

    /// Tall enough for `Test scene · fixture` to fit in full (CORE-R73).
    fn titled(entries: &[&'static Entry]) -> Runtime {
        Runtime::new(
            entries.to_vec(),
            1,
            Size::new(40, 8),
            GlyphMode::Unicode,
            Some(0),
        )
    }

    fn bottom_line(rt: &Runtime) -> String {
        rt.frame().to_lines().last().cloned().unwrap_or_default()
    }

    fn shows_title(rt: &Runtime) -> bool {
        bottom_line(rt).starts_with(" Test scene · fixture ")
    }

    #[test]
    fn rt_r12_canvas_title_shows_iff_the_setting_is_on_and_the_hud_off() {
        let mut r = titled(&[&OK]);
        assert!(shows_title(&r), "default: title on, HUD off");
        r.handle(Action::ToggleHud);
        assert!(!shows_title(&r), "HUD on shows it instead");
        r.handle(Action::ToggleHud);
        assert!(shows_title(&r));

        let r = titled(&[&OK]).with_hud(true);
        assert!(!shows_title(&r));
        let r = titled(&[&OK]).with_title(false);
        assert!(!shows_title(&r));
        let mut r = r.with_hud(true);
        r.handle(Action::ToggleHud);
        assert!(!shows_title(&r), "setting off: never on the canvas");
    }

    #[test]
    fn rt_r12_scene_switches_keep_the_title_visibility() {
        let mut r = titled(&[&OK, &OK2]).with_hud(true);
        r.handle(Action::Next);
        assert!(!shows_title(&r));
        r.handle(Action::ToggleHud);
        r.handle(Action::Next);
        assert!(shows_title(&r));
        r.handle(Action::Reset);
        r.tick(SIM_DT);
        assert!(shows_title(&r));
    }

    #[test]
    fn rt_r12_toggling_the_hud_while_paused_re_renders_without_stepping() {
        let mut r = titled(&[&OK]);
        r.tick(3.0 * SIM_DT);
        r.handle(Action::TogglePause);
        r.handle(Action::ToggleHud);
        assert!(!shows_title(&r), "title gone under the frozen frame");
        assert_eq!(first_line(&r), "3", "no step taken");
        r.handle(Action::ToggleHud);
        assert!(shows_title(&r), "title back under the frozen frame");
        assert_eq!(first_line(&r), "3", "no step taken");
    }

    #[test]
    fn rt_r12_title_is_the_live_title_or_none() {
        let r = titled(&[&OK]);
        assert_eq!(r.title(), Some(Title::new(OK_META.title, None)));
        assert_eq!(
            r.with_hud(true).title().map(|t| t.phenomenon),
            Some("Test scene")
        );
        assert_eq!(titled(&[&OK]).with_title(false).title(), None);
    }

    #[test]
    fn rt_r4_r12_no_title_once_all_failed() {
        let mut r = titled(&[&BOOM]);
        r.tick(SIM_DT);
        assert_eq!(r.status(), Status::AllFailed);
        assert_eq!(r.title(), None);
        r.handle(Action::ToggleHud); // no stage: nothing to re-render, no panic
        assert_eq!(r.title(), None);
    }

    // ----- M11: cycle order (RT-R13) -----

    static STAR_META: Meta = meta_in("star", Category::DeepCosmos);
    static MOON_META: Meta = meta_in("moon", Category::SunPlanets);
    static CELL_META: Meta = meta_in("cell", Category::Biology);
    static STAR: Entry = Entry {
        meta: &STAR_META,
        build: || {
            Box::new(Counter {
                meta: &STAR_META,
                steps: 0,
                boom_at: 0,
            })
        },
    };
    static MOON: Entry = Entry {
        meta: &MOON_META,
        build: || {
            Box::new(Counter {
                meta: &MOON_META,
                steps: 0,
                boom_at: 0,
            })
        },
    };
    static CELL: Entry = Entry {
        meta: &CELL_META,
        build: || {
            Box::new(Counter {
                meta: &CELL_META,
                steps: 0,
                boom_at: 0,
            })
        },
    };

    /// Three Physics entries (`ok*`) and one each of three other categories.
    const MIXED: [&Entry; 6] = [&OK, &OK2, &OK3, &STAR, &MOON, &CELL];

    fn sim(entries: &[&'static Entry], seed: u64) -> Runtime {
        Runtime::new(
            entries.to_vec(),
            seed,
            Size::new(8, 2),
            GlyphMode::Ascii,
            None,
        )
    }

    fn order_names(r: &Runtime) -> Vec<&'static str> {
        r.order.iter().map(|&i| r.entries[i].meta.name).collect()
    }

    #[test]
    fn rt_r13_sim_starts_at_the_head_of_the_order_and_next_walks_it_wrapping() {
        let mut r = sim(&MIXED, 7);
        let order = order_names(&r);
        let mut shown = vec![r.current_name().unwrap()];
        for _ in 0..MIXED.len() {
            r.handle(Action::Next);
            shown.push(r.current_name().unwrap());
        }
        assert_eq!(shown[..MIXED.len()], order[..]);
        assert_eq!(shown[MIXED.len()], order[0], "wraps around");
    }

    #[test]
    fn rt_r5_r13_auto_cycle_follows_the_order() {
        let mut r = sim(&MIXED, 11).with_interval(1.0);
        let order = order_names(&r);
        for want in &order[1..] {
            r.tick(1.01);
            assert_eq!(r.current_name(), Some(*want));
        }
    }

    #[test]
    fn rt_r3_r13_after_a_panic_the_walk_resumes_after_the_failed_entry() {
        let entries = [&OK, &STAR, &BOOM, &MOON, &CELL];
        let boom = 2;
        for seed in 0..8 {
            let mut r = Runtime::new(
                entries.to_vec(),
                seed,
                Size::new(8, 2),
                GlyphMode::Ascii,
                Some(boom),
            );
            let at = r.order.iter().position(|&i| i == boom).unwrap();
            let want = r.entries[r.order[(at + 1) % entries.len()]].meta.name;
            r.tick(SIM_DT);
            assert_eq!(r.current_name(), Some(want), "seed {seed}");
            // The quarantined entry is skipped on the next lap.
            for _ in 0..entries.len() {
                r.handle(Action::Next);
                assert_ne!(r.current_name(), Some("boom"), "seed {seed}");
            }
        }
    }

    #[test]
    fn rt_r13_order_is_shuffled_by_the_seed() {
        let orders: std::collections::HashSet<_> = (0..32)
            .map(|seed| order_names(&sim(&MIXED, seed)))
            .collect();
        assert!(orders.len() > 8, "only {} distinct orders", orders.len());
        let heads: std::collections::HashSet<_> =
            orders.iter().map(|o| find_category(o[0])).collect();
        assert!(heads.len() > 1, "the category order is shuffled too");
    }

    fn find_category(name: &str) -> Category {
        MIXED
            .iter()
            .find(|e| e.meta.name == name)
            .map(|e| e.meta.category)
            .unwrap()
    }

    mod props {
        use proptest::prelude::*;

        use super::super::cycle_order;
        use super::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            #[test]
            fn rt_r13_cycle_order_is_a_deterministic_category_interleaved_permutation(
                picks in proptest::collection::vec(0..Category::ALL.len(), 0..60),
                seed: u64,
            ) {
                let cats: Vec<Category> = picks.iter().map(|&k| Category::ALL[k]).collect();
                let order = cycle_order(&cats, &mut ChaCha8Rng::seed_from_u64(seed));

                let mut sorted = order.clone();
                sorted.sort_unstable();
                prop_assert_eq!(sorted, (0..cats.len()).collect::<Vec<_>>(), "a permutation");

                let again = cycle_order(&cats, &mut ChaCha8Rng::seed_from_u64(seed));
                prop_assert_eq!(&again, &order, "same seed, same order");

                for i in 1..order.len() {
                    let cat = cats[order[i]];
                    if cats[order[i - 1]] == cat {
                        prop_assert!(
                            order[i..].iter().all(|&j| cats[j] == cat),
                            "{:?} repeats at {} while other categories remain", cat, i
                        );
                    }
                }
            }
        }
    }
}
