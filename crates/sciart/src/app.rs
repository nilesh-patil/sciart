//! Terminal event loop: the only place that touches the wall clock and the terminal.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::layout::Rect;
use ratatui::style::Style;
use sciart_core::title::{Form, ascii_of};
use sciart_core::{GlyphMode, Size};
use sciart_generators::{Entry, registry};
use sciart_keepawake::{Guard, Mode};
use sciart_render::{FrameBackend, Hud, Painter, TerminalGuard, Theme};

use crate::fps::FpsMeter;
use crate::keys::action_for;
use crate::pacer::Pacer;
use crate::runtime::{Flow, Runtime, Status, install_panic_filter};

/// The project URL shown on the help page (RT-R18).
const PROJECT_URL: &str = "https://github.com/nilesh-patil/sciart";

/// The static key list of the help page (RT-R18). ASCII only; with the header, the `now`
/// block and the footer the page is 23 lines, inside 80x24.
const HELP_KEYS: &str = "\
  n / right       next scene
  p / left        previous scene
  space           pause / resume
  r               reset scene
  h               toggle HUD
  t               next theme
  ?               toggle this help
  f               toggle favourite
  v               favourites only
  [ / ]           previous / next category
  s               shuffle
  q / Esc / Ctrl-C  quit (Esc closes help)
";

#[derive(Debug, Clone)]
pub struct Options {
    pub entries: Vec<&'static Entry>,
    pub fps: u16,
    pub seed: u64,
    pub theme: Theme,
    pub glyphs: GlyphMode,
    pub hud: bool,
    /// Science title on (CFG-R6, CLI-R7).
    pub title: bool,
    /// Auto-cycle period; `None` for `run` (RT-R5).
    pub interval: Option<f64>,
    /// Simulated seconds per wall second (CFG-R9).
    pub speed: f64,
    pub keep_awake: Mode,
    pub low_power_fps: u16,
    pub idle_after: f64,
    /// The config file in use, or where it would be: shown on the help page (RT-R18).
    pub config_path: Option<PathBuf>,
    /// Config `favorites`, seeded into the session (CFG-R9, RT-R19).
    pub favorites: Vec<String>,
}

/// What the HUD and help page need besides the runtime and the keep-awake text.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HudExtras<'a> {
    pub fps: f64,
    pub seed: u64,
    pub low_power: bool,
    pub config_path: Option<&'a Path>,
}

/// Drawing state kept across frames, so a steady-state frame allocates nothing (RT-R14):
/// the painter's colour memory (RND-R14), reused text buffers for the HUD, and the help
/// page text (RT-R18).
#[derive(Debug, Default)]
pub(crate) struct Screen {
    painter: Painter,
    /// `Runtime::epoch` the painter's memory belongs to.
    epoch: u64,
    /// The keep-awake status, rewritten in place each frame (`Guard::write_status`).
    pub keep_awake: String,
    hud_text: String,
    /// The help page, rebuilt in place each frame it is open (RT-R18).
    help_text: String,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            painter: Painter::new(),
            ..Self::default()
        }
    }

    /// Draw one frame: the help page, or the canvas and the RT-R4 error line / HUD on the
    /// last row. Returns whether the picture changed (RT-R17): a frame whose only changes
    /// are on the HUD or error row (the fps readout follows the draw rate) counts as
    /// unchanged.
    pub fn draw<W: Write>(
        &mut self,
        terminal: &mut Terminal<FrameBackend<W>>,
        runtime: &Runtime,
        extras: HudExtras<'_>,
    ) -> io::Result<bool> {
        // A new or reset scene must not inherit the old one's emitted colours (RND-R14);
        // theme and size changes the painter notices by itself.
        if runtime.epoch() != self.epoch {
            self.epoch = runtime.epoch();
            self.painter.reset();
        }
        let help = runtime.help();
        let Self {
            painter,
            keep_awake,
            hud_text,
            help_text,
            ..
        } = self;
        if help {
            Self::build_help(help_text, runtime, extras.config_path);
        }
        let overlay = !help && (runtime.status() == Status::AllFailed || runtime.hud());
        let mut overlay_row = None;
        terminal.draw(|f| {
            let area = f.area();
            if help {
                // RT-R18: a static page instead of the scene and the HUD.
                draw_help(area, f.buffer_mut(), help_text);
                return;
            }
            if overlay {
                overlay_row = area.bottom().checked_sub(1);
            }
            let buf = f.buffer_mut();
            painter.paint(runtime.frame(), runtime.theme(), area, buf);
            if runtime.status() == Status::AllFailed {
                // RT-R4: keep the last frame, explain on the bottom row, reversed like the
                // HUD (what `Line::from(msg).reversed()` drew, without building a `Line`).
                let msg = runtime.notices().last().map_or("", String::as_str);
                let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1)
                    .intersection(buf.area);
                if !msg.is_empty() && !row.is_empty() {
                    buf.set_style(row, Style::new().reversed());
                    buf.set_stringn(row.x, row.y, msg, usize::from(row.width), Style::new());
                }
            } else if runtime.hud() {
                let hud = Hud {
                    generator: runtime.current_name().unwrap_or("-"),
                    title: runtime.title(),
                    fps: extras.fps,
                    theme: runtime.theme(),
                    seed: extras.seed,
                    keep_awake: keep_awake.as_str(),
                    low_power: extras.low_power,
                    paused: runtime.paused(),
                    quarantined: runtime.quarantined(),
                };
                hud.render_into(area, buf, hud_text);
            }
        })?;
        Ok(match terminal.backend().changed_rows() {
            None => false,
            Some((top, _)) => overlay_row.is_none_or(|row| top < row),
        })
    }

    /// Rebuild the help page in `out`, reusing its capacity (RT-R14, RT-R18). The page is
    /// 23 lines of at most 80 columns, so it fits an 80x24 terminal without scrolling.
    fn build_help(out: &mut String, runtime: &Runtime, config_path: Option<&Path>) {
        out.clear();
        out.push_str("sciart ");
        out.push_str(env!("CARGO_PKG_VERSION"));
        out.push_str(" - help\n");
        out.push_str(HELP_KEYS);
        out.push_str("\n now\n");
        let _ = writeln!(
            out,
            "  scene           {}",
            runtime.current_name().unwrap_or("-")
        );
        out.push_str("  title           ");
        match runtime.scene_title() {
            Some(title) => {
                for c in title.chars(Form::Full) {
                    out.push(ascii_of(c).unwrap_or('?'));
                }
            }
            None => out.push('-'),
        }
        out.push('\n');
        let _ = writeln!(
            out,
            "  category        {}",
            runtime
                .current_category()
                .map_or("-", sciart_core::Category::label)
        );
        let _ = writeln!(
            out,
            "  favourite       {}   filter  {}",
            if runtime.is_favourite() { "yes" } else { "no" },
            if runtime.favourites_only() {
                "favourites only"
            } else {
                "all scenes"
            }
        );
        let _ = writeln!(
            out,
            "  favourites      {}     scenes  {} registered",
            runtime.favourite_count(),
            registry().len()
        );
        const CONFIG_PREFIX: &str = "  config          ";
        out.push_str(CONFIG_PREFIX);
        match config_path {
            Some(path) => push_path(out, path, 80 - CONFIG_PREFIX.len() - 1),
            None => out.push('-'),
        }
        out.push('\n');
        let _ = writeln!(out, "  project         {PROJECT_URL}");
        out.push_str(" press ? or Esc to return to the art");
    }
}

/// Write `path` into `out`, eliding its tail with `~` if it would exceed `budget` columns.
fn push_path(out: &mut String, path: &Path, budget: usize) {
    struct Take<'a> {
        out: &'a mut String,
        left: usize,
        cut: bool,
    }
    impl std::fmt::Write for Take<'_> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for c in s.chars() {
                if self.left == 0 {
                    self.cut = true;
                    return Err(std::fmt::Error);
                }
                self.out.push(c);
                self.left -= 1;
            }
            Ok(())
        }
    }
    let mut take = Take {
        out,
        left: budget,
        cut: false,
    };
    let _ = write!(take, "{}", path.display());
    if take.cut {
        take.out.push('~');
    }
}

/// Draw the help page over the whole area, ASCII and clipped to the terminal (RT-R18).
fn draw_help(area: Rect, buf: &mut ratatui::buffer::Buffer, text: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    buf.reset();
    let width = usize::from(area.width);
    for (i, line) in text.split('\n').enumerate() {
        let Ok(row) = u16::try_from(i) else { break };
        let y = area.y.saturating_add(row);
        if y >= area.bottom() {
            break;
        }
        buf.set_stringn(area.x, y, line, width, Style::new());
    }
}

pub fn run(opts: Options, mut keep_awake: Guard) -> anyhow::Result<()> {
    let mut guard = TerminalGuard::enter()?;
    install_panic_filter();
    let terminal = guard.terminal();

    let area = terminal.size()?;
    let size = Size::new(area.width, area.height);
    let mut runtime = Runtime::new(opts.entries, opts.seed, size, opts.glyphs, None)
        .with_hud(opts.hud)
        .with_title(opts.title)
        .with_speed(opts.speed)
        .with_theme(opts.theme)
        .with_favourites(&opts.favorites);
    if let Some(interval) = opts.interval {
        runtime = runtime.with_interval(interval);
    }

    let mut pacer = Pacer::new(opts.fps, opts.low_power_fps, opts.idle_after);
    let mut fps = FpsMeter::default();
    let mut screen = Screen::new();
    let mut last = wall_clock();
    let mut epoch = runtime.epoch();
    let mut first = true;

    loop {
        let now = wall_clock();
        let elapsed = now.duration_since(last).as_secs_f64();
        last = now;
        // The first span is set-up time, microseconds: as the meter's seed (RT-R10) it read
        // as thousands of fps, which a stepped-down rate (RT-R17) took long to average out.
        if !std::mem::take(&mut first) {
            fps.record(elapsed);
        }
        pacer.record(elapsed);
        keep_awake.poll(elapsed);
        runtime.tick(elapsed);

        keep_awake.write_status(&mut screen.keep_awake);
        let extras = HudExtras {
            fps: fps.fps(),
            seed: opts.seed,
            low_power: pacer.low_power(),
            config_path: opts.config_path.as_deref(),
        };
        let changed = screen.draw(terminal, &runtime, extras)?;
        // A scene switch (auto-cycle) wakes the rate as a key would (RT-R17).
        pacer.drawn(changed || runtime.epoch() != epoch);
        epoch = runtime.epoch();

        // Handle input until the next frame is due. The rate can change here (RT-R8,
        // RT-R17), so the deadline is recomputed after every event: a key that wakes a
        // stepped-down loop is drawn at the full rate's deadline, not the slow one's.
        loop {
            let frame_time = Duration::from_secs_f64(1.0 / f64::from(pacer.fps()));
            let wait = (now + frame_time).saturating_duration_since(wall_clock());
            if !event::poll(wait)? {
                break;
            }
            match event::read()? {
                Event::Key(key) => {
                    pacer.input();
                    if let Some(action) = action_for(key)
                        && runtime.handle(action) == Flow::Quit
                    {
                        return Ok(());
                    }
                }
                Event::FocusLost => pacer.focus(false),
                Event::FocusGained => pacer.focus(true),
                Event::Resize(w, h) => {
                    pacer.wake();
                    runtime.resize(Size::new(w, h));
                }
                _ => {}
            }
        }
    }
}

/// The event loop's only clock. The allow is scoped to this call, so the rest of the file
/// stays under the GEN-R7 clock ban and the TEST-R8 leak bans.
#[allow(clippy::disallowed_methods)] // wall clock is the binary's job (ADR-0008)
fn wall_clock() -> Instant {
    Instant::now()
}

#[cfg(test)]
mod tests {
    use std::io;

    use sciart_render::FrameBackend;
    use sciart_testalloc::measure;

    use super::*;
    use crate::runtime::Action;

    const W: u16 = 80;
    const H: u16 = 24;

    fn terminal() -> Terminal<FrameBackend<io::Sink>> {
        Terminal::new(FrameBackend::with_size(io::sink(), W, H)).expect("terminal")
    }

    fn runtime(names: &[&str]) -> Runtime {
        let entries = names
            .iter()
            .map(|n| sciart_generators::find(n).expect("scene"))
            .collect();
        Runtime::new(entries, 42, Size::new(W, H), GlyphMode::Unicode, Some(0))
    }

    /// One app frame, minus the event poll: tick, keep-awake text, draw.
    fn frame(
        rt: &mut Runtime,
        screen: &mut Screen,
        term: &mut Terminal<FrameBackend<io::Sink>>,
        ka: &Guard,
    ) {
        rt.tick(1.0 / 30.0);
        ka.write_status(&mut screen.keep_awake);
        let extras = HudExtras {
            fps: 29.7,
            seed: 16_376_649_230_169_585_180,
            low_power: false,
            config_path: None,
        };
        screen.draw(term, rt, extras).expect("draw");
    }

    #[test]
    fn rt_r14_steady_state_frames_do_not_allocate() {
        // A glyph scene and a half-block field scene, with and without the HUD.
        for (name, hud) in [
            ("starfield", true),
            ("wave_interference", true),
            ("wave_interference", false),
        ] {
            let mut rt = runtime(&[name]).with_hud(hud);
            let (mut screen, mut term, ka) = (Screen::new(), terminal(), Guard::off());
            for _ in 0..30 {
                frame(&mut rt, &mut screen, &mut term, &ka);
            }
            let ((), stats) = measure(|| {
                for _ in 0..60 {
                    frame(&mut rt, &mut screen, &mut term, &ka);
                }
            });
            assert_eq!(stats.total(), 0, "{name} hud={hud}: {stats:?}");
        }
    }

    /// Every cell of the screen changes glyph, fg, bg and modifiers from the last frame.
    fn worst(buf: &mut ratatui::buffer::Buffer, t: u8) {
        use ratatui::style::{Color, Modifier};
        for (i, cell) in buf.content.iter_mut().enumerate() {
            let v = 200 + ((i as u8).wrapping_add(t) % 50);
            let odd = (i + usize::from(t)) % 2 == 1;
            cell.set_char(if odd { '⣿' } else { '▀' })
                .set_fg(Color::Rgb(v, 255 - t, 100 + t))
                .set_bg(Color::Rgb(255 - t, v, 199 - t));
            cell.modifier = if odd {
                Modifier::REVERSED
            } else {
                Modifier::all()
            };
        }
    }

    #[test]
    fn rt_r14_a_frame_bigger_than_any_before_does_not_allocate() {
        // A quiet first frame, then the worst frame the backend can be asked to send: the
        // frame buffer was sized for it up front, so it must not grow mid-run.
        let mut term = terminal();
        term.draw(|f| {
            f.buffer_mut()[(0, 0)].set_char('x');
        })
        .expect("draw");
        let ((), stats) = measure(|| {
            for t in 0..4 {
                term.draw(|f| worst(f.buffer_mut(), t)).expect("draw");
            }
        });
        assert_eq!(stats.total(), 0, "{stats:?}");
    }

    #[test]
    fn rt_r14_a_paused_frame_and_a_hud_toggle_settle_to_zero() {
        let mut rt = runtime(&["starfield"]);
        let (mut screen, mut term, ka) = (Screen::new(), terminal(), Guard::off());
        for _ in 0..10 {
            frame(&mut rt, &mut screen, &mut term, &ka);
        }
        rt.handle(Action::TogglePause);
        rt.handle(Action::ToggleHud);
        for _ in 0..3 {
            frame(&mut rt, &mut screen, &mut term, &ka);
        }
        let ((), stats) = measure(|| {
            for _ in 0..20 {
                frame(&mut rt, &mut screen, &mut term, &ka);
            }
        });
        assert_eq!(stats.total(), 0, "{stats:?}");
    }

    fn draw(
        rt: &Runtime,
        screen: &mut Screen,
        term: &mut Terminal<FrameBackend<io::Sink>>,
        fps: f64,
    ) -> bool {
        let extras = HudExtras {
            fps,
            seed: 42,
            low_power: false,
            config_path: None,
        };
        screen.draw(term, rt, extras).expect("draw")
    }

    #[test]
    fn rt_r17_draw_reports_whether_the_picture_changed() {
        let mut rt = runtime(&["starfield"]);
        let (mut screen, mut term) = (Screen::new(), terminal());
        assert!(draw(&rt, &mut screen, &mut term, 30.0), "first frame");
        rt.tick(1.0 / 30.0);
        assert!(draw(&rt, &mut screen, &mut term, 30.0), "stars move");
        rt.handle(Action::TogglePause);
        draw(&rt, &mut screen, &mut term, 30.0);
        for _ in 0..5 {
            rt.tick(1.0 / 30.0);
            assert!(
                !draw(&rt, &mut screen, &mut term, 30.0),
                "paused: nothing new"
            );
        }
        // A key that changes the picture while paused.
        rt.handle(Action::CycleTheme);
        assert!(draw(&rt, &mut screen, &mut term, 30.0), "theme change");
    }

    #[test]
    fn rt_r17_a_change_on_the_hud_row_alone_is_quiet() {
        // The HUD's fps readout follows the stepped-down rate; if that counted as a
        // change, stepping down would undo itself.
        let mut rt = runtime(&["starfield"]).with_hud(true);
        let (mut screen, mut term) = (Screen::new(), terminal());
        rt.handle(Action::TogglePause);
        draw(&rt, &mut screen, &mut term, 30.0);
        for fps in [15.0, 7.0, 5.0] {
            assert!(!draw(&rt, &mut screen, &mut term, fps), "fps {fps}");
        }
        rt.handle(Action::TogglePause);
        rt.tick(1.0 / 30.0);
        assert!(
            draw(&rt, &mut screen, &mut term, 5.0),
            "the stars above the HUD move"
        );
    }

    #[test]
    fn rt_r17_rt_r11_a_stepped_down_rate_keeps_simulation_speed() {
        // A paused scene steps the draw rate down; the frames' wall time still drives the
        // fixed step, so after unpausing, a 5 FPS frame advances 12 steps (RT-R11) and
        // the pacer's frame times add up to wall time.
        let mut pacer = Pacer::new(30, 5, 120.0);
        let (mut wall, mut frames) = (0.0, 0u32);
        while wall < 3.0 {
            let dt = 1.0 / f64::from(pacer.fps());
            pacer.record(dt);
            pacer.drawn(false);
            wall += dt;
            frames += 1;
        }
        assert_eq!(pacer.fps(), 5);
        assert!(frames < 90 / 2, "{frames} frames in 3 s: stepped down");
        let mut rt = runtime(&["starfield"]);
        let mut fast = runtime(&["starfield"]);
        for _ in 0..30 {
            fast.tick(1.0 / 30.0);
        }
        for _ in 0..5 {
            rt.tick(1.0 / f64::from(pacer.fps()));
        }
        assert!(
            rt.frame().cells() == fast.frame().cells(),
            "1 s at 5 FPS = 1 s at 30 FPS"
        );
    }

    #[test]
    fn rnd_r14_scene_switch_and_reset_bump_the_epoch() {
        let mut rt = runtime(&["starfield", "voyager_journey"]);
        let e0 = rt.epoch();
        rt.handle(Action::Next);
        let e1 = rt.epoch();
        assert!(e1 > e0, "switch");
        rt.handle(Action::Reset);
        assert!(rt.epoch() > e1, "reset");
        let e2 = rt.epoch();
        rt.handle(Action::CycleTheme);
        rt.handle(Action::ToggleHud);
        rt.tick(0.1);
        assert_eq!(rt.epoch(), e2, "theme/HUD/tick keep the scene");
    }

    // ----- M23: help overlay (RT-R18) -----

    fn help_extras(config_path: Option<&Path>) -> HudExtras<'_> {
        HudExtras {
            fps: 30.0,
            seed: 42,
            low_power: false,
            config_path,
        }
    }

    #[test]
    fn rt_r18_help_draws_at_one_by_one_without_panicking() {
        let mut rt = runtime(&["starfield"]);
        rt.handle(Action::ToggleHelp);
        let mut term = Terminal::new(FrameBackend::with_size(io::sink(), 1, 1)).expect("terminal");
        let mut screen = Screen::new();
        screen
            .draw(
                &mut term,
                &rt,
                help_extras(Some(Path::new("/tmp/sciart/config.toml"))),
            )
            .expect("draw");
        assert!(screen.help_text.starts_with("sciart "), "page built");
    }

    #[test]
    fn rt_r18_help_page_lists_the_bindings_and_the_current_state() {
        let mut rt = runtime(&["starfield"]).with_favourites(&["starfield"]);
        rt.handle(Action::ToggleHud); // help replaces the HUD too
        rt.handle(Action::ToggleHelp);
        let (mut screen, mut term) = (Screen::new(), terminal());
        screen
            .draw(
                &mut term,
                &rt,
                help_extras(Some(Path::new("/tmp/sciart/config.toml"))),
            )
            .expect("draw");
        let page = &screen.help_text;
        for needle in [
            "next scene",
            "previous scene",
            "pause / resume",
            "reset scene",
            "toggle HUD",
            "next theme",
            "toggle this help",
            "toggle favourite",
            "favourites only",
            "previous / next category",
            "shuffle",
            "Esc closes help",
            "starfield",
            "Parallax - near stars rush by",
            "Deep Cosmos",
            "yes   filter",
            "all scenes",
            "0.1.0",
            "/tmp/sciart/config.toml",
            PROJECT_URL,
        ] {
            assert!(page.contains(needle), "missing {needle:?} in:\n{page}");
        }
        assert!(
            page.lines().count() <= 24,
            "the page fits 80x24 without scrolling:\n{page}"
        );
        for line in page.lines() {
            assert!(
                line.chars().count() <= 80,
                "line is wider than 80 columns: {line:?}"
            );
        }
        assert!(
            page.contains("press ? or Esc to return to the art"),
            "footer visible in:\n{page}"
        );
        assert!(
            page.contains(&format!("{} registered", registry().len())),
            "scene count in:\n{page}"
        );
    }

    #[test]
    fn rt_r18_help_elides_a_long_config_path_and_stays_80_columns() {
        let mut rt = runtime(&["starfield"]);
        rt.handle(Action::ToggleHelp);
        let (mut screen, mut term) = (Screen::new(), terminal());
        let long = "/very/long/path/that/keeps/going/and/going/for/a/while/sciart/config.toml";
        screen
            .draw(&mut term, &rt, help_extras(Some(Path::new(long))))
            .expect("draw");
        let page = &screen.help_text;
        for line in page.lines() {
            assert!(
                line.chars().count() <= 80,
                "line is wider than 80 columns: {line:?}"
            );
        }
        assert!(page.contains('~'), "the path is elided in:\n{page}");
    }

    #[test]
    fn rt_r18_opening_and_closing_help_repaints_the_screen() {
        let mut rt = runtime(&["starfield"]);
        let (mut screen, mut term) = (Screen::new(), terminal());
        assert!(draw(&rt, &mut screen, &mut term, 30.0), "scene");
        rt.handle(Action::ToggleHelp);
        assert!(draw(&rt, &mut screen, &mut term, 30.0), "help opens");
        assert!(
            !draw(&rt, &mut screen, &mut term, 30.0),
            "a static help page is unchanged"
        );
        rt.handle(Action::ToggleHelp);
        assert!(draw(&rt, &mut screen, &mut term, 30.0), "help closes");
    }

    #[test]
    fn rt_r14_help_frames_do_not_allocate() {
        let mut rt = runtime(&["starfield"]);
        rt.handle(Action::ToggleHelp);
        let (mut screen, mut term, ka) = (Screen::new(), terminal(), Guard::off());
        for _ in 0..30 {
            frame(&mut rt, &mut screen, &mut term, &ka);
        }
        let ((), stats) = measure(|| {
            for _ in 0..60 {
                frame(&mut rt, &mut screen, &mut term, &ka);
            }
        });
        assert_eq!(stats.total(), 0, "{stats:?}");
    }
}
