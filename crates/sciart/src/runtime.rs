//! Terminal-free runtime state machine (SPEC-003 RT-R1..R7, RT-R12, RT-R13; ADR-0006).

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

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
    /// Open or close the help overlay (RT-R18).
    ToggleHelp,
    /// Add or remove the current scene from the session's favourites (RT-R19).
    ToggleFavourite,
    /// Navigate only among healthy favourites (RT-R19).
    ToggleFavouritesOnly,
    PrevCategory,
    NextCategory,
    /// Rebuild the cycle order from the runtime RNG and start at its head (RT-R19).
    Shuffle,
    /// The `Esc` key: close help when open, otherwise quit (RT-R18).
    Escape,
    Quit,
    /// `Ctrl-C`: quits like `Quit`; `while` also stops its command (CLI-R14).
    Interrupt,
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
    /// Simulated seconds per wall second (CFG-R9); 1.0 = real time.
    speed: f64,
    /// Indices shown, oldest first; the last one is normally `current` (RT-R5a).
    history: Vec<usize>,
    paused: bool,
    hud: bool,
    /// The help overlay is open (RT-R18).
    help: bool,
    /// Per-entry favourite flags, seeded from config `favorites` (RT-R19).
    favourites: Vec<bool>,
    /// Navigate only among healthy favourites (RT-R19).
    favourites_only: bool,
    /// The `title` setting (CFG-R6, CLI-R7).
    title: bool,
    theme: Theme,
    /// Wall seconds passed to `tick`/`pass_wall` so far (RT-R23).
    wall: f64,
    /// End of the run in the same wall seconds (RT-R23, CLI-R12); `None` = no deadline.
    deadline: Option<f64>,
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
            favourites: vec![false; entries.len()],
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
            speed: 1.0,
            history: Vec::new(),
            paused: false,
            hud: false,
            help: false,
            favourites_only: false,
            title: true,
            theme: Theme::default(),
            wall: 0.0,
            deadline: None,
        };
        match start {
            Some(i) if i < rt.entries.len() => rt.activate(i, true),
            _ => rt.switch_to_next(),
        }
        rt
    }

    /// Advance by `elapsed` wall seconds, auto-cycling if due, and render a new frame
    /// (RT-R2, RT-R5). The scene does not move while paused, but the deadline clock always
    /// does: `Flow::Quit` once the deadline is reached, as the `Quit` action reports
    /// (RT-R23).
    pub fn tick(&mut self, elapsed: f64) -> Flow {
        self.pass_wall(elapsed);
        self.advance(elapsed);
        self.flow()
    }

    /// Count `elapsed` wall seconds towards the deadline without touching the scene
    /// (RT-R23): for a span the loop keeps out of the simulation clock, such as a wait
    /// that blocks while paused (RT-R21). `tick` already counts its own `elapsed`.
    pub fn pass_wall(&mut self, elapsed: f64) -> Flow {
        if elapsed.is_finite() && elapsed > 0.0 {
            self.wall += elapsed;
        }
        self.flow()
    }

    /// End the run once `secs` wall seconds have passed (RT-R23, CLI-R12).
    pub fn with_deadline(mut self, secs: f64) -> Self {
        self.deadline = Some(secs);
        self
    }

    /// `Quit` once the deadline is reached, otherwise `Continue` (RT-R23).
    pub fn flow(&self) -> Flow {
        match self.deadline {
            Some(deadline) if self.wall >= deadline => Flow::Quit,
            _ => Flow::Continue,
        }
    }

    /// Wall seconds left before the deadline, never negative; `None` without one (RT-R23).
    pub fn time_left(&self) -> Option<f64> {
        self.deadline
            .map(|deadline| (deadline - self.wall).max(0.0))
    }

    /// `wait` shortened to the time left, so a loop that waits for input wakes when the
    /// deadline arrives even when nothing else would wake it (RT-R23, RT-R21).
    pub fn cap_wait(&self, wait: Duration) -> Duration {
        // `try_`: a deadline too far away for a `Duration` caps nothing.
        match self.time_left().map(Duration::try_from_secs_f64) {
            Some(Ok(left)) => wait.min(left),
            _ => wait,
        }
    }

    /// The scene's share of `tick` (RT-R2, RT-R5).
    fn advance(&mut self, elapsed: f64) {
        // The help overlay is a static page: the scene keeps its last frame (RT-R18).
        if self.paused || self.help || self.stage.is_none() {
            return;
        }
        if elapsed.is_finite() && elapsed > 0.0 {
            self.since_switch += elapsed;
        }
        if let Some(interval) = self.interval
            && self.since_switch >= interval
            && self.eligible_count() > 1
        {
            self.switch_to_next();
        }
        let Some(stage) = self.stage.as_mut() else {
            return;
        };
        let steps = self.clock.advance(elapsed * self.speed);
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

    /// Simulated seconds per wall second (CFG-R9): scales only what reaches `FixedStep`,
    /// so the step stays 1/60 s and the auto-cycle interval stays wall time (RT-R19).
    pub fn with_speed(mut self, speed: f64) -> Self {
        self.speed = speed;
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

    /// Seed the session's favourites from config names (CFG-R9, RT-R19). Names match the
    /// active set directly; retired aliases resolve through `find`; names outside the
    /// active set are ignored.
    pub fn with_favourites<S: AsRef<str>>(mut self, names: &[S]) -> Self {
        for name in names {
            let name = name.as_ref();
            let direct = self.entries.iter().position(|e| e.meta.name == name);
            let index = direct.or_else(|| {
                let entry = sciart_generators::find(name)?;
                self.entries
                    .iter()
                    .position(|e| e.meta.name == entry.meta.name)
            });
            if let Some(i) = index {
                self.favourites[i] = true;
            }
        }
        self
    }

    /// Apply a user action (RT-R6, RT-R18, RT-R19).
    pub fn handle(&mut self, action: Action) -> Flow {
        // Esc is the only key that closes help instead of acting (RT-R18). `q` and Ctrl-C
        // keep their unconditional quit mapping.
        if action == Action::Escape {
            return if self.help {
                self.help = false;
                Flow::Continue
            } else {
                Flow::Quit
            };
        }
        // `Quit` still quits while help is open (RT-R18); so does `Ctrl-C`.
        if matches!(action, Action::Quit | Action::Interrupt) {
            return Flow::Quit;
        }
        if action == Action::ToggleHelp {
            self.help = !self.help;
            return Flow::Continue;
        }
        // Any other action closes the overlay before acting (RT-R18).
        self.help = false;

        let generator_action = matches!(
            action,
            Action::Next
                | Action::Prev
                | Action::Reset
                | Action::TogglePause
                | Action::ToggleFavourite
                | Action::ToggleFavouritesOnly
                | Action::PrevCategory
                | Action::NextCategory
                | Action::Shuffle
        );
        if generator_action && self.status == Status::AllFailed {
            return Flow::Continue;
        }
        match action {
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
            Action::ToggleFavourite => self.toggle_favourite(),
            Action::ToggleFavouritesOnly => self.toggle_favourites_only(),
            Action::PrevCategory => self.switch_to_category(-1),
            Action::NextCategory => self.switch_to_category(1),
            Action::Shuffle => self.shuffle(),
            Action::ToggleHelp | Action::Escape | Action::Quit | Action::Interrupt => {}
        }
        Flow::Continue
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn hud(&self) -> bool {
        self.hud
    }

    /// Whether the help overlay is open (RT-R18).
    pub fn help(&self) -> bool {
        self.help
    }

    /// Whether navigation is restricted to healthy favourites (RT-R19).
    pub fn favourites_only(&self) -> bool {
        self.favourites_only
    }

    /// Whether the current scene is a favourite; `false` with no stage (RT-R19).
    pub fn is_favourite(&self) -> bool {
        self.current.is_some_and(|i| self.favourites[i])
    }

    /// Number of favourite entries this session (healthy or not).
    pub fn favourite_count(&self) -> usize {
        self.favourites.iter().filter(|f| **f).count()
    }

    /// The current scene's category, for the help page (RT-R18).
    pub fn current_category(&self) -> Option<Category> {
        self.current.map(|i| self.entries[i].meta.category)
    }

    /// The live title regardless of the `title` setting, for the help page (RT-R18).
    pub fn scene_title(&self) -> Option<Title> {
        self.stage.as_ref().map(Stage::title)
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

    /// Entries navigation may select: never quarantined, and only favourites while the
    /// filter is on (RT-R19).
    fn eligible(&self, i: usize) -> bool {
        !self.quarantined[i] && (!self.favourites_only || self.favourites[i])
    }

    fn eligible_count(&self) -> usize {
        (0..self.entries.len())
            .filter(|&i| self.eligible(i))
            .count()
    }

    fn any_healthy_favourite(&self) -> bool {
        (0..self.entries.len()).any(|i| self.favourites[i] && !self.quarantined[i])
    }

    /// `f`: toggle the current scene's favourite flag; drop the filter when the last
    /// favourite goes (RT-R19).
    fn toggle_favourite(&mut self) {
        let Some(i) = self.current else {
            return;
        };
        self.favourites[i] = !self.favourites[i];
        if self.favourites_only && !self.favourites[i] && self.favourite_count() == 0 {
            self.favourites_only = false;
        }
    }

    /// `v`: flip the filter; turning it on moves to a favourite unless the current scene is
    /// one. With no healthy favourite there is nothing to show, so it is a no-op (RT-R19).
    fn toggle_favourites_only(&mut self) {
        if self.favourites_only {
            self.favourites_only = false;
            return;
        }
        if !self.is_favourite() && !self.any_healthy_favourite() {
            return;
        }
        self.favourites_only = true;
        if !self.is_favourite()
            && let Some(first) = self.order.iter().copied().find(|&i| self.eligible(i))
        {
            self.activate(first, true);
        }
    }

    /// Go back to the most recent earlier eligible generator (RT-R5a, RT-R19).
    fn prev(&mut self) {
        let mut history = self.history.clone();
        if history.last().copied() == self.current {
            history.pop();
        }
        while let Some(&j) = history.last() {
            if self.eligible(j) {
                self.history = history;
                self.activate(j, false);
                return;
            }
            history.pop();
        }
    }

    /// `[`/`]`: first eligible scene of the previous/next non-empty category in
    /// `Category::ALL` order, wrapping (RT-R19).
    fn switch_to_category(&mut self, direction: i32) {
        let Some(current) = self.current else {
            return;
        };
        let category = self.entries[current].meta.category;
        let start = Category::ALL
            .iter()
            .position(|&c| c == category)
            .unwrap_or(0);
        let count = Category::ALL.len();
        for step in 1..=count {
            let index = (start as i32 + direction * step as i32).rem_euclid(count as i32) as usize;
            let wanted = Category::ALL[index];
            if let Some(next) = (0..self.entries.len())
                .find(|&i| self.entries[i].meta.category == wanted && self.eligible(i))
            {
                self.activate(next, true);
                return;
            }
        }
    }

    /// `s`: rebuild the cycle order from the runtime RNG and activate its first eligible
    /// entry (RT-R19).
    fn shuffle(&mut self) {
        let categories: Vec<Category> = self.entries.iter().map(|e| e.meta.category).collect();
        self.order = cycle_order(&categories, &mut self.rng);
        self.at = None;
        self.switch_to_next();
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

    /// Activate the next eligible entry after the last one in the cycle order, wrapping
    /// (RT-R13, RT-R19). The current entry comes last in that walk, so it is rebuilt only
    /// when it is the sole eligible one.
    fn switch_to_next(&mut self) {
        let n = self.order.len();
        let from = self.at.map_or(0, |p| p + 1);
        let next = (0..n)
            .map(|k| self.order[(from + k) % n])
            .find(|&i| self.eligible(i));
        let Some(next) = next else {
            // A favourites-only filter with no healthy favourite must not strand the
            // runtime: drop it and fall back to every healthy scene (RT-R19).
            if self.favourites_only && !self.any_healthy_favourite() {
                self.favourites_only = false;
                return self.switch_to_next();
            }
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

    // ----- M25: run deadline (RT-R23) -----

    #[test]
    fn rt_r23_without_a_deadline_the_runtime_never_quits_by_itself() {
        let mut r = rt(&[&OK], 0);
        for _ in 0..100 {
            assert_eq!(r.tick(3600.0), Flow::Continue);
        }
        assert_eq!(r.time_left(), None);
        let wait = Duration::from_secs(5);
        assert_eq!(r.cap_wait(wait), wait);
    }

    #[test]
    fn rt_r23_the_deadline_quits_once_its_wall_time_has_passed() {
        let mut r = rt(&[&OK], 0).with_deadline(1.0);
        assert_eq!(r.flow(), Flow::Continue);
        for _ in 0..3 {
            assert_eq!(r.tick(0.25), Flow::Continue);
        }
        assert_eq!(r.tick(0.25), Flow::Quit, "reported like the Quit action");
        assert_eq!(r.flow(), Flow::Quit, "and it stays reached");
        assert_eq!(r.tick(0.25), Flow::Quit);
        assert_eq!(r.time_left(), Some(0.0));

        // A 30 fps loop ends on the frame that crosses the deadline, not a frame later.
        let mut r = rt(&[&OK], 0).with_deadline(2.0);
        let frames = (1..).find(|_| r.tick(1.0 / 30.0) == Flow::Quit).unwrap();
        assert!((60..=61).contains(&frames), "{frames} frames");
    }

    #[test]
    fn rt_r23_pause_help_low_rate_and_speed_do_not_stop_the_clock() {
        let paused = rt(&[&OK], 0).with_deadline(1.0);
        let help = rt(&[&OK], 0).with_deadline(1.0);
        let fast = rt(&[&OK], 0).with_deadline(1.0).with_speed(4.0);
        for (what, mut r, action) in [
            ("paused", paused, Some(Action::TogglePause)),
            ("help", help, Some(Action::ToggleHelp)),
            ("speed 4", fast, None),
        ] {
            if let Some(action) = action {
                r.handle(action);
            }
            let shown = first_line(&r);
            for _ in 0..3 {
                assert_eq!(r.tick(0.25), Flow::Continue, "{what}");
            }
            assert_eq!(r.tick(0.25), Flow::Quit, "{what}");
            if action.is_some() {
                assert_eq!(first_line(&r), shown, "{what}: the scene did not move");
            }
        }
        // A stepped-down rate (RT-R17) ticks less often, with longer spans.
        let mut r = rt(&[&OK], 0).with_deadline(1.0);
        assert_eq!(r.tick(0.5), Flow::Continue);
        assert_eq!(r.tick(0.5), Flow::Quit);
    }

    #[test]
    fn rt_r23_pass_wall_counts_a_span_the_scene_never_sees() {
        let mut r = rt(&[&OK], 0).with_deadline(2.0);
        r.tick(3.0 * SIM_DT);
        let shown = first_line(&r);
        assert_eq!(r.pass_wall(1.0), Flow::Continue);
        assert_eq!(first_line(&r), shown, "no simulation step");
        assert_eq!(r.pass_wall(1.0), Flow::Quit);
    }

    #[test]
    fn rt_r23_time_left_counts_down_and_caps_the_wait() {
        let mut r = rt(&[&OK], 0).with_deadline(10.0);
        assert_eq!(r.time_left(), Some(10.0));
        r.tick(2.5);
        assert_eq!(r.time_left(), Some(7.5));
        let second = Duration::from_secs(1);
        assert_eq!(r.cap_wait(second), second, "far from the deadline");
        r.pass_wall(7.0);
        assert_eq!(r.cap_wait(second), Duration::from_millis(500));
        r.pass_wall(5.0);
        assert_eq!(r.time_left(), Some(0.0), "never negative");
        assert_eq!(r.cap_wait(second), Duration::ZERO);
        // A deadline beyond what a `Duration` holds caps nothing.
        let r = rt(&[&OK], 0).with_deadline(f64::MAX);
        assert_eq!(r.cap_wait(second), second);
    }

    #[test]
    fn rt_r23_spans_that_are_not_time_are_ignored() {
        let mut r = rt(&[&OK], 0).with_deadline(1.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -5.0, 0.0] {
            assert_eq!(r.pass_wall(bad), Flow::Continue, "{bad}");
        }
        assert_eq!(r.time_left(), Some(1.0));
    }

    #[test]
    fn cli_r14_ctrl_c_quits_like_q_even_with_help_open() {
        let mut r = rt(&[&OK], 0);
        assert_eq!(r.handle(Action::Interrupt), Flow::Quit);
        r.handle(Action::ToggleHelp);
        assert_eq!(r.handle(Action::Interrupt), Flow::Quit);
        assert!(r.help(), "Ctrl-C quits without closing help first");
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
    fn cfg_r9_speed_scales_simulation_time_but_not_the_switch_timer() {
        // 0.1 wall seconds is 6 fixed steps at 1x and 12 at 2x (CFG-R9): speed multiplies
        // only what reaches `FixedStep`, so `dt` stays 1/60 and frames stay deterministic.
        let mut slow = rt(&[&OK], 0);
        slow.tick(0.1);
        assert_eq!(first_line(&slow), "6");
        let mut fast = rt(&[&OK], 0).with_speed(2.0);
        fast.tick(0.1);
        assert_eq!(first_line(&fast), "12");

        // The auto-cycle interval is wall time (RT-R5), so 2x speed does not halve it.
        let mut r = rt(&[&OK, &OK2], 0).with_interval(1.0).with_speed(2.0);
        r.tick(0.6);
        assert_eq!(r.current_name(), Some("ok"), "0.6 wall s < 1 s");
        r.tick(0.41);
        assert_eq!(r.current_name(), Some("ok2"), "1.01 wall s cycled");
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
            Action::ToggleFavourite,
            Action::ToggleFavouritesOnly,
            Action::PrevCategory,
            Action::NextCategory,
            Action::Shuffle,
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

    // ----- M23: help overlay (RT-R18) -----

    #[test]
    fn rt_r18_help_toggles_and_other_actions_close_it_before_acting() {
        let mut r = rt(&[&OK, &OK2], 0);
        assert!(!r.help());
        assert_eq!(r.handle(Action::ToggleHelp), Flow::Continue);
        assert!(r.help());
        r.handle(Action::ToggleHud);
        assert!(!r.help(), "another action closes help");
        assert!(r.hud(), "and still acts");
        r.handle(Action::ToggleHelp);
        assert!(r.help());
        r.handle(Action::Next);
        assert!(!r.help());
        assert_eq!(r.current_name(), Some("ok2"), "the action still ran");
    }

    #[test]
    fn rt_r18_esc_closes_help_and_quits_only_when_it_is_closed() {
        let mut r = rt(&[&OK], 0);
        assert_eq!(r.handle(Action::Escape), Flow::Quit);
        r.handle(Action::ToggleHelp);
        assert_eq!(r.handle(Action::Escape), Flow::Continue);
        assert!(!r.help());
        r.handle(Action::ToggleHelp);
        assert_eq!(r.handle(Action::ToggleHud), Flow::Continue);
        r.handle(Action::ToggleHelp);
        assert_eq!(r.handle(Action::Quit), Flow::Quit, "q quits with help open");
    }

    #[test]
    fn rt_r18_help_does_not_step_the_simulation() {
        let mut r = rt(&[&OK], 0);
        r.tick(SIM_DT);
        assert_eq!(first_line(&r), "1");
        r.handle(Action::ToggleHelp);
        r.tick(1.0);
        assert_eq!(first_line(&r), "1", "no step while help is open");
        r.handle(Action::ToggleHelp);
        r.tick(SIM_DT);
        assert_eq!(first_line(&r), "2");
    }

    // ----- M23: favourites and filters (RT-R19) -----

    #[test]
    fn rt_r19_toggle_favourite_marks_the_current_scene() {
        let mut r = rt(&[&OK, &OK2], 0);
        assert!(!r.is_favourite());
        r.handle(Action::ToggleFavourite);
        assert!(r.is_favourite());
        assert_eq!(r.favourite_count(), 1);
        r.handle(Action::Next);
        assert_eq!(r.current_name(), Some("ok2"));
        assert!(!r.is_favourite(), "a different scene is not a favourite");
        r.handle(Action::Prev);
        assert!(
            r.is_favourite(),
            "the flag follows the scene, not the session"
        );
        r.handle(Action::ToggleFavourite);
        assert!(!r.is_favourite());
        assert_eq!(r.favourite_count(), 0);
    }

    #[test]
    fn rt_r19_favourites_only_switches_to_a_favourite_and_is_a_noop_without_one() {
        let mut r = rt(&[&OK, &OK2], 0);
        r.handle(Action::ToggleFavouritesOnly);
        assert!(!r.favourites_only(), "no favourites: no-op");
        assert_eq!(r.current_name(), Some("ok"));

        let mut r = rt(&[&OK, &OK2], 0).with_favourites(&["ok2"]);
        r.handle(Action::ToggleFavouritesOnly);
        assert!(r.favourites_only());
        assert_eq!(r.current_name(), Some("ok2"));
        assert!(r.is_favourite());
        r.handle(Action::ToggleFavouritesOnly);
        assert!(!r.favourites_only());
        assert_eq!(
            r.current_name(),
            Some("ok2"),
            "turning it off keeps the scene"
        );
    }

    #[test]
    fn rt_r19_removing_the_last_favourite_turns_the_filter_off() {
        let mut r = rt(&[&OK, &OK2], 0).with_favourites(&["ok"]);
        r.handle(Action::ToggleFavouritesOnly);
        assert!(r.favourites_only());
        r.handle(Action::ToggleFavourite);
        assert!(!r.is_favourite());
        assert!(
            !r.favourites_only(),
            "no favourite remains: filter turns off"
        );
    }

    #[test]
    fn rt_r19_navigation_considers_only_healthy_favourites() {
        let mut r = rt(&[&OK, &OK2, &OK3], 0).with_favourites(&["ok2", "ok3"]);
        r.handle(Action::ToggleFavouritesOnly);
        let first = r.current_name().expect("a favourite");
        assert!(matches!(first, "ok2" | "ok3"), "{first}");
        assert!(r.is_favourite());
        r.handle(Action::Next);
        let second = r.current_name().expect("a favourite");
        assert!(matches!(second, "ok2" | "ok3"), "{second}");
        assert_ne!(second, first);
        r.handle(Action::Prev);
        assert_eq!(r.current_name(), Some(first));
        for _ in 0..6 {
            r.handle(Action::Next);
            assert!(
                matches!(r.current_name(), Some("ok2" | "ok3")),
                "{:?}",
                r.current_name()
            );
        }
    }

    #[test]
    fn rt_r19_quarantined_favourites_are_skipped() {
        let mut r = rt(&[&OK, &BOOM], 0).with_favourites(&["ok", "boom"]);
        r.handle(Action::ToggleFavouritesOnly); // current ok is a favourite: stays
        assert_eq!(r.current_name(), Some("ok"));
        r.handle(Action::Next); // the other favourite, which panics on its first step
        assert_eq!(r.current_name(), Some("boom"));
        r.tick(SIM_DT);
        assert_eq!(r.quarantined(), 1);
        assert_eq!(r.current_name(), Some("ok"));
        r.handle(Action::Next);
        assert_eq!(
            r.current_name(),
            Some("ok"),
            "quarantined entry is not selected"
        );
    }

    #[test]
    fn rt_r19_the_filter_turns_off_when_the_last_healthy_favourite_is_quarantined() {
        let mut r = rt(&[&BOOM, &OK], 0).with_favourites(&["boom"]);
        r.handle(Action::ToggleFavouritesOnly); // current boom is the favourite
        assert!(r.favourites_only());
        r.tick(SIM_DT); // boom panics
        assert_eq!(r.quarantined(), 1);
        assert!(!r.favourites_only(), "no healthy favourite remains");
        assert_eq!(r.current_name(), Some("ok"));
    }

    #[test]
    fn rt_r19_favourites_are_seeded_from_names_and_retired_aliases() {
        let star = sciart_generators::find("starfield").expect("scene");
        let gray = sciart_generators::find("gray_scott").expect("scene");
        let r = Runtime::new(
            vec![star, gray],
            1,
            Size::new(8, 2),
            GlyphMode::Ascii,
            Some(0),
        )
        .with_favourites(&["chemical_reaction", "starfield"]);
        assert_eq!(r.favourite_count(), 2);
        assert!(r.is_favourite(), "starfield is current and favourited");
    }

    #[test]
    fn rt_r19_category_keys_walk_category_all_order_and_wrap() {
        // Physics (ok), DeepCosmos (star), SunPlanets (moon), Biology (cell).
        let mut r = rt(&[&OK, &STAR, &MOON, &CELL], 0);
        r.handle(Action::NextCategory); // Physics -> Chemistry (empty) -> Biology
        assert_eq!(r.current_name(), Some("cell"));
        r.handle(Action::PrevCategory); // Biology -> Chemistry (empty) -> Physics
        assert_eq!(r.current_name(), Some("ok"));
        r.handle(Action::NextCategory);
        assert_eq!(r.current_name(), Some("cell"));
        r.handle(Action::NextCategory); // Biology -> ... -> wraps to DeepCosmos
        assert_eq!(r.current_name(), Some("star"));
        r.handle(Action::PrevCategory); // DeepCosmos -> ... -> wraps to Biology
        assert_eq!(r.current_name(), Some("cell"));
    }

    #[test]
    fn rt_r19_category_keys_respect_the_favourites_filter() {
        let mut r = rt(&[&OK, &STAR, &CELL], 0).with_favourites(&["star"]);
        r.handle(Action::ToggleFavouritesOnly); // ok is not a favourite -> star
        assert_eq!(r.current_name(), Some("star"));
        r.handle(Action::NextCategory);
        assert_eq!(r.current_name(), Some("star"), "only DeepCosmos has one");
        r.handle(Action::Next);
        assert_eq!(r.current_name(), Some("star"));
    }

    #[test]
    fn rt_r19_shuffle_rebuilds_the_order_and_activates_its_head() {
        let mut a = sim(&MIXED, 7);
        let before = order_names(&a);
        a.handle(Action::Shuffle);
        let after = order_names(&a);
        assert_eq!(
            after.iter().collect::<std::collections::HashSet<_>>(),
            before.iter().collect::<std::collections::HashSet<_>>(),
            "still a permutation"
        );
        assert_eq!(a.current_name(), Some(after[0]), "the new order's head");
        let mut b = sim(&MIXED, 7);
        b.handle(Action::Shuffle);
        assert_eq!(order_names(&b), after, "same seed, same shuffle");
    }

    #[test]
    fn rt_r19_shuffle_with_the_filter_lands_on_a_healthy_favourite() {
        let mut r = sim(&MIXED, 7).with_favourites(&["star", "cell"]);
        r.handle(Action::ToggleFavouritesOnly);
        r.handle(Action::Shuffle);
        assert!(
            matches!(r.current_name(), Some("star" | "cell")),
            "{:?}",
            r.current_name()
        );
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
