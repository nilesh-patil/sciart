//! Terminal event loop: the only place that touches the wall clock and the terminal.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::Backend as _;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use sciart_core::title::{Form, ascii_of};
use sciart_core::{GlyphMode, Size};
use sciart_generators::{Entry, registry};
use sciart_keepawake::{Guard, Mode};
use sciart_render::{
    Background, Compositor, Content, Draw, FrameBackend, Hud, Painter, TerminalGuard, Theme,
};

use crate::config::ColorMode;
use crate::fps::FpsMeter;
use crate::keys::action_for;
use crate::pacer::Pacer;
use crate::runtime::{Action, Flow, Runtime, Status, install_panic_filter};

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
    /// Effective theme (`mono` already substituted when the colour mode is mono, RND-R18).
    pub theme: Theme,
    /// Resolved colour capability (RND-R18): `mono`/`truecolor` are theme/RGB and `indexed`
    /// makes the painter emit xterm-256 indices.
    pub color: ColorMode,
    /// Opaque black or terminal-default cell backgrounds (RND-R20, CLI-R11/CFG-R11).
    pub background: Background,
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
    /// `--for`: end the run after this many wall seconds (CLI-R12, RT-R23).
    pub run_for: Option<f64>,
}

/// Why [`run`] returned. The terminal is restored by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// `q`, or `Esc` with help closed.
    Quit,
    /// `Ctrl-C` (CLI-R14 tells it from a quit).
    Interrupt,
    /// The `--for` deadline was reached (RT-R23).
    Deadline,
    /// The caller's `done` said so: the `while` command exited (CLI-R14).
    Done,
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
    /// What the terminal shows, so a frame sends only the cells that changed (RND-R22).
    compositor: Compositor,
    /// `Runtime::epoch` the painter's memory belongs to.
    epoch: u64,
    /// The keep-awake status, rewritten in place each frame (`Guard::write_status`).
    pub keep_awake: String,
    hud_text: String,
    /// The help page, rebuilt in place each frame it is open (RT-R18).
    help_text: String,
}

impl Screen {
    /// The default 24-bit painter (`--color auto` without `NO_COLOR`, or `truecolor`).
    pub fn new() -> Self {
        Self::with_painter(Painter::new())
    }

    /// A screen whose painter emits xterm-256 indices (RND-R18, `--color 256`).
    pub fn indexed() -> Self {
        Self::with_painter(Painter::indexed())
    }

    /// Use the selected background mode (RND-R20); the painter repaints every cell.
    pub fn with_background(mut self, background: Background) -> Self {
        self.painter = self.painter.with_background(background);
        self
    }

    fn with_painter(painter: Painter) -> Self {
        Self {
            painter,
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
            compositor,
            keep_awake,
            hud_text,
            help_text,
            ..
        } = self;
        if help {
            Self::build_help(help_text, runtime, extras.config_path);
        }
        let background = painter.background();
        let blank = painter.blank_color();
        let overlay = !help && (runtime.status() == Status::AllFailed || runtime.hud());
        // RND-R22: the compositor paints and sends only the cells that changed; it takes the
        // full-area path by itself on a resize, theme, painter reset or overlay toggle.
        let backend = terminal.backend_mut();
        let size = backend.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        let overlay_row = overlay.then(|| area.bottom().checked_sub(1)).flatten();
        if help {
            // RT-R18: a static page instead of the scene and the HUD.
            let mut page = |area: Rect, buf: &mut ratatui::buffer::Buffer| {
                draw_help(area, buf, help_text, background, blank);
            };
            compositor.compose(area, Content::Full(&mut page));
        } else if runtime.status() == Status::AllFailed {
            // RT-R4: keep the last frame, explain on the bottom row, reversed like the
            // HUD (what `Line::from(msg).reversed()` drew, without building a `Line`).
            let msg = runtime.notices().last().map_or("", String::as_str);
            let mut error_line = |area: Rect, buf: &mut ratatui::buffer::Buffer| {
                let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1)
                    .intersection(buf.area);
                if !msg.is_empty() && !row.is_empty() {
                    buf.set_style(row, Style::new().reversed());
                    buf.set_stringn(row.x, row.y, msg, usize::from(row.width), Style::new());
                }
            };
            compositor.compose(area, canvas(runtime, painter, Some(&mut error_line)));
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
                // Rounded up, so `00:00` is never shown while the run goes on (CLI-R12).
                ends: runtime.time_left().map(|left| left.ceil() as u64),
            };
            let mut line = |area: Rect, buf: &mut ratatui::buffer::Buffer| {
                hud.render_into(area, buf, hud_text);
            };
            compositor.compose(area, canvas(runtime, painter, Some(&mut line)));
        } else {
            compositor.compose(area, canvas(runtime, painter, None));
        }
        compositor.emit(backend)?;
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

/// The runtime's frame through `painter`, with `overlay` on the last row (RND-R22).
fn canvas<'a>(
    runtime: &'a Runtime,
    painter: &'a mut Painter,
    overlay: Option<Draw<'a>>,
) -> Content<'a> {
    Content::Canvas {
        canvas: runtime.frame(),
        theme: runtime.theme(),
        painter,
        overlay,
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
fn draw_help(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    text: &str,
    background: Background,
    blank: Color,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    buf.reset();
    // RND-R20: the help page is a full-screen visual too; keep terminal wallpaper out and
    // use the same black the painter would emit (indexed black under `--color 256`).
    let style = match background {
        Background::Opaque => Style::new().bg(blank),
        Background::Terminal => Style::new(),
    };
    buf.set_style(area, style);
    let width = usize::from(area.width);
    for (i, line) in text.split('\n').enumerate() {
        let Ok(row) = u16::try_from(i) else { break };
        let y = area.y.saturating_add(row);
        if y >= area.bottom() {
            break;
        }
        buf.set_stringn(area.x, y, line, width, style);
    }
}

/// Show the art until a key, the deadline or `done` ends it. `done` is asked once per frame;
/// `keep_awake` is borrowed so it can outlive the art (CLI-R14).
pub fn run(
    opts: Options,
    keep_awake: &mut Guard,
    mut done: impl FnMut() -> bool,
) -> anyhow::Result<End> {
    // RT-R20: SIGTERM/SIGHUP restore the terminal from a watcher thread, whatever the loop
    // is doing. Started first, so no signal finds the terminal set up and unwatched; a
    // signal during the set-up waits for it, as `TerminalGuard::enter` holds the stdout
    // lock the restore takes (see `signals`).
    #[cfg(unix)]
    crate::signals::watch()?;
    // Before the guard, so it is dropped after the guard has restored the terminal.
    #[cfg(unix)]
    let _set_up = crate::signals::SetUp::mark();
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
    if let Some(secs) = opts.run_for {
        runtime = runtime.with_deadline(secs);
    }

    let mut pacer = Pacer::new(opts.fps, opts.low_power_fps, opts.idle_after);
    let mut fps = FpsMeter::default();
    let mut screen = if opts.color.is_indexed() {
        Screen::indexed()
    } else {
        Screen::new()
    }
    .with_background(opts.background);
    let mut last = wall_clock();
    let mut epoch = runtime.epoch();
    let mut first = true;
    let mut wakeups = Wakeups::from_env();
    // When the keep-awake backend was last polled: the RT-R21 block polls it too.
    let mut polled = last;
    // The status after a blocked poll, compared with the one the HUD shows (RT-R21).
    let mut status = String::new();

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
        keep_awake.poll(now.duration_since(polled).as_secs_f64());
        polled = now;
        if runtime.tick(elapsed) == Flow::Quit {
            return Ok(End::Deadline);
        }
        if done() {
            return Ok(End::Done);
        }
        // Wall time is counted towards the deadline up to here (RT-R23); a blocked wait
        // below counts its own span from this mark, so every span is counted once.
        let mut counted = now;

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
        let block = may_block(
            &runtime,
            &pacer,
            fps.fps(),
            terminal.backend().changed_rows().is_none(),
        );
        loop {
            let frame_time = Duration::from_secs_f64(1.0 / f64::from(pacer.fps()));
            // The deadline wakes the loop even if no frame is due (RT-R23).
            let wait = if block {
                block_timeout(runtime.remaining())
            } else {
                runtime.cap_wait((now + frame_time).saturating_duration_since(wall_clock()))
            };
            let ready = event::poll(wait)?;
            wakeups.count();
            if block {
                // The blocked span reaches neither the fps readout nor the simulation, but
                // it is wall time: it counts towards the deadline (RT-R23), and `while`'s
                // command may have exited meanwhile (CLI-R14).
                let t = wall_clock();
                let flow = runtime.pass_wall(t.duration_since(counted).as_secs_f64());
                counted = t;
                if flow == Flow::Quit {
                    return Ok(End::Deadline);
                }
                if done() {
                    return Ok(End::Done);
                }
            }
            let redraw = if !ready && block {
                // Only the keep-awake backend needs the time (KA-R3, KA-R4); a status the
                // HUD shows that the poll changed is drawn now, not at the next key.
                let t = wall_clock();
                keep_awake.poll(t.duration_since(polled).as_secs_f64());
                polled = t;
                keep_awake.write_status(&mut status);
                redraw_after_poll(&runtime, &screen.keep_awake, &status)
            } else {
                false
            };
            if !ready && block && !redraw {
                continue;
            }
            if !ready && !block {
                break;
            }
            if block {
                // The blocked span is not a frame interval: keep it out of the fps
                // readout and the simulation clock (paused or help: nothing to step).
                let t = wall_clock();
                pacer.record(t.duration_since(last).as_secs_f64());
                last = t;
                first = true;
            }
            if redraw {
                break;
            }
            match event::read()? {
                Event::Key(key) => {
                    pacer.input();
                    if let Some(action) = action_for(key)
                        && runtime.handle(action) == Flow::Quit
                    {
                        return Ok(if action == Action::Interrupt {
                            End::Interrupt
                        } else {
                            End::Quit
                        });
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
            if block {
                break; // draw what the event changed now (RT-R21)
            }
        }
    }
}

/// How long a blocked loop (RT-R21) waits for an event before it polls the keep-awake
/// backend; at least a second, which every backend's cadence allows (KA-R3, KA-R4).
const BLOCK_POLL: Duration = Duration::from_secs(1);

/// Whether the HUD is on screen, showing the keep-awake status and the fps readout.
fn hud_shown(runtime: &Runtime) -> bool {
    !runtime.help() && runtime.hud() && runtime.status() != Status::AllFailed
}

/// RT-R21: whether the loop may block on the next event after a drawn frame. Paused or
/// on the help page, once the rate is down (RT-R17) and the frame sent nothing
/// (`sent_nothing`), nothing can change before the next event. A visible HUD first lets
/// its fps readout (`fps`) settle on the stepped-down rate, or it would freeze on a
/// passing value.
fn may_block(runtime: &Runtime, pacer: &Pacer, fps: f64, sent_nothing: bool) -> bool {
    let settled = !hud_shown(runtime) || (fps - f64::from(pacer.fps())).abs() < 0.5;
    (runtime.paused() || runtime.help()) && pacer.at_floor() && settled && sent_nothing
}

/// RT-R21: how long a blocked loop waits for an event: [`BLOCK_POLL`], or less when a
/// deadline is `remaining` (the wait must not outlast it).
fn block_timeout(remaining: Option<Duration>) -> Duration {
    remaining.map_or(BLOCK_POLL, |r| r.min(BLOCK_POLL))
}

/// RT-R21: after a blocked loop polls the keep-awake backend, whether to draw again:
/// the HUD is on screen and the status it shows (`shown`) is no longer the backend's
/// (`status`), e.g. caffeinate exited during a pause.
fn redraw_after_poll(runtime: &Runtime, shown: &str, status: &str) -> bool {
    hud_shown(runtime) && shown != status
}

/// Test hook (RT-R21): with `SCIART_TRACE_WAKEUPS=<file>` the loop writes how often it
/// has woken (each return from the event wait) to that file, so the pty test can count
/// wake-ups from outside. Off, and free, without the variable.
struct Wakeups {
    path: Option<PathBuf>,
    count: u64,
}

impl Wakeups {
    fn from_env() -> Self {
        Self {
            path: std::env::var_os("SCIART_TRACE_WAKEUPS").map(PathBuf::from),
            count: 0,
        }
    }

    fn count(&mut self) {
        if let Some(path) = &self.path {
            self.count += 1;
            let _ = std::fs::write(path, self.count.to_string());
        }
    }
}

/// The binary's only clock: the event loop's, and the `while` deadline's once the art has
/// closed (CLI-R14). The allow is scoped to this call, so the rest of the binary stays
/// under the GEN-R7 clock ban and the TEST-R8 leak bans.
#[allow(clippy::disallowed_methods)] // wall clock is the binary's job (ADR-0008)
pub(crate) fn wall_clock() -> Instant {
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
    fn rt_r14_cli_r12_a_deadline_on_the_hud_does_not_allocate() {
        let mut rt = runtime(&["starfield"]).with_hud(true).with_deadline(7200.0);
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

    #[test]
    fn cli_r12_the_hud_shows_the_time_left_rounded_up() {
        let mut rt = runtime(&["starfield"]).with_hud(true).with_deadline(3600.0);
        let (mut screen, mut term) = (Screen::new(), terminal());
        draw(&rt, &mut screen, &mut term, 30.0);
        assert!(
            screen.hud_text.contains("| ends 1:00:00"),
            "{}",
            screen.hud_text
        );
        rt.tick(0.25);
        draw(&rt, &mut screen, &mut term, 30.0);
        assert!(
            screen.hud_text.contains("| ends 1:00:00"),
            "{}",
            screen.hud_text
        );
        rt.tick(0.75);
        draw(&rt, &mut screen, &mut term, 30.0);
        assert!(
            screen.hud_text.contains("| ends 59:59"),
            "{}",
            screen.hud_text
        );
        // Without a deadline there is no such field.
        let rt = runtime(&["starfield"]).with_hud(true);
        draw(&rt, &mut screen, &mut term, 30.0);
        assert!(!screen.hud_text.contains("ends"), "{}", screen.hud_text);
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

    // ----- M25: changed-cell pipeline (RND-R22) -----

    impl Screen {
        /// The full-area reference (RND-R22): what `Screen::draw` did before M25, a whole
        /// frame painted into ratatui's buffer and diffed in full by `Terminal::draw`.
        fn draw_reference<W: Write>(
            &mut self,
            terminal: &mut Terminal<FrameBackend<W>>,
            runtime: &Runtime,
            extras: HudExtras<'_>,
        ) -> io::Result<()> {
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
            let background = painter.background();
            let blank = painter.blank_color();
            terminal.draw(|f| {
                let area = f.area();
                if help {
                    draw_help(area, f.buffer_mut(), help_text, background, blank);
                    return;
                }
                let buf = f.buffer_mut();
                painter.paint(runtime.frame(), runtime.theme(), area, buf);
                if runtime.status() == Status::AllFailed {
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
                        ends: runtime.time_left().map(|left| left.ceil() as u64),
                    };
                    hud.render_into(area, buf, hud_text);
                }
            })?;
            Ok(())
        }
    }

    type Recorded = Terminal<FrameBackend<Vec<u8>>>;

    /// A screen on the changed-cell path and one on the full-area reference, with the same
    /// painter settings, each writing to its own byte stream.
    struct Pair {
        label: &'static str,
        new: (Screen, Recorded),
        reference: (Screen, Recorded),
        /// Stream lengths after each frame, to name the first frame that differs.
        marks: Vec<(usize, usize)>,
    }

    impl Pair {
        fn new(label: &'static str, screen: impl Fn() -> Screen, size: Size) -> Self {
            let term = || {
                Terminal::new(FrameBackend::with_size(
                    Vec::new(),
                    size.width(),
                    size.height(),
                ))
                .expect("terminal")
            };
            Self {
                label,
                new: (screen(), term()),
                reference: (screen(), term()),
                marks: Vec::new(),
            }
        }

        fn draw(&mut self, rt: &Runtime, fps: f64) {
            let extras = HudExtras {
                fps,
                seed: 42,
                low_power: false,
                config_path: None,
            };
            self.new.0.draw(&mut self.new.1, rt, extras).expect("draw");
            self.reference
                .0
                .draw_reference(&mut self.reference.1, rt, extras)
                .expect("draw");
            self.marks.push((
                self.new.1.backend().writer().len(),
                self.reference.1.backend().writer().len(),
            ));
        }

        fn resize(&mut self, size: Size) {
            for term in [&mut self.new.1, &mut self.reference.1] {
                term.backend_mut()
                    .set_fixed_size(size.width(), size.height());
            }
        }

        /// `None` when both streams are identical, else where they part.
        fn mismatch(&self) -> Option<String> {
            let (a, b) = (
                self.new.1.backend().writer(),
                self.reference.1.backend().writer(),
            );
            if a == b {
                return None;
            }
            let frame = self
                .marks
                .iter()
                .position(|&(x, y)| x != y || a[..x] != b[..y]);
            Some(format!(
                "{}: streams differ ({} vs {} bytes), first at frame {frame:?}",
                self.label,
                a.len(),
                b.len()
            ))
        }
    }

    const DRAWN_FRAMES: u32 = 72;

    /// Every registered scene through both paths, in each colour mode (RND-R18: truecolor,
    /// 256 colours, mono) and both backgrounds (RND-R20), with the HUD, theme, pause,
    /// resize, reset and help toggled along the way (RND-R22).
    fn every_scene_matches_the_full_area_stream(size: Size) {
        let shrunk = Size::new(size.width() - 7, size.height() - 3);
        let failures: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = registry()
                .iter()
                .map(|entry| {
                    s.spawn(move || {
                        let mut failures = Vec::new();
                        for mono in [false, true] {
                            let mut rt =
                                Runtime::new(vec![entry], 42, size, GlyphMode::Unicode, Some(0));
                            if mono {
                                rt = rt.with_theme(Theme::Mono);
                            }
                            let opaque = |s: Screen| s.with_background(Background::Opaque);
                            let terminal = |s: Screen| s.with_background(Background::Terminal);
                            let mut pairs = if mono {
                                vec![
                                    Pair::new("mono/opaque", || opaque(Screen::new()), size),
                                    Pair::new("mono/terminal", || terminal(Screen::new()), size),
                                ]
                            } else {
                                vec![
                                    Pair::new("truecolor/opaque", || opaque(Screen::new()), size),
                                    Pair::new(
                                        "truecolor/terminal",
                                        || terminal(Screen::new()),
                                        size,
                                    ),
                                    Pair::new("256/opaque", || opaque(Screen::indexed()), size),
                                    Pair::new("256/terminal", || terminal(Screen::indexed()), size),
                                ]
                            };
                            for frame in 1..=DRAWN_FRAMES {
                                let action = match frame {
                                    10 | 34 | 58 => Some(Action::ToggleHud),
                                    18 | 64 if !mono => Some(Action::CycleTheme),
                                    24 | 30 => Some(Action::TogglePause),
                                    48 => Some(Action::Reset),
                                    52 | 55 => Some(Action::ToggleHelp),
                                    _ => None,
                                };
                                if let Some(action) = action {
                                    rt.handle(action);
                                }
                                let resize = match frame {
                                    38 => Some(shrunk),
                                    44 => Some(size),
                                    _ => None,
                                };
                                if let Some(to) = resize {
                                    rt.resize(to);
                                    for pair in &mut pairs {
                                        pair.resize(to);
                                    }
                                }
                                rt.tick(1.0 / 30.0);
                                // The HUD's fps readout changes from frame to frame.
                                let fps = 28.0 + f64::from(frame % 3);
                                for pair in &mut pairs {
                                    pair.draw(&rt, fps);
                                }
                            }
                            failures.extend(
                                pairs
                                    .iter()
                                    .filter_map(Pair::mismatch)
                                    .map(|m| format!("{}: {m}", entry.meta.name)),
                            );
                        }
                        failures
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("scene thread"))
                .collect()
        });
        assert!(
            failures.is_empty(),
            "RND-R22 at {}x{}:\n{}",
            size.width(),
            size.height(),
            failures.join("\n")
        );
    }

    #[test]
    fn rnd_r22_changed_cells_send_the_full_area_bytes_for_every_scene_at_80x24() {
        every_scene_matches_the_full_area_stream(Size::new(80, 24));
    }

    #[test]
    fn rnd_r22_changed_cells_send_the_full_area_bytes_for_every_scene_at_200x50() {
        every_scene_matches_the_full_area_stream(Size::new(200, 50));
    }

    #[test]
    fn rnd_r22_a_frame_visits_only_changed_cells_plus_row_bookkeeping() {
        let size = Size::new(200, 50);
        for entry in registry() {
            let name = entry.meta.name;
            let mut rt = Runtime::new(vec![entry], 42, size, GlyphMode::Unicode, Some(0));
            let mut screen = Screen::new().with_background(Background::Opaque);
            let mut term = Terminal::new(FrameBackend::with_size(
                io::sink(),
                size.width(),
                size.height(),
            ))
            .expect("terminal");
            let mut last: Option<Vec<sciart_core::Cell>> = None;
            for frame in 0..40 {
                rt.tick(1.0 / 30.0);
                draw(&rt, &mut screen, &mut term, 30.0);
                let work = screen.compositor.work();
                let cells = rt.frame().cells();
                if let Some(prev) = &last {
                    let changed = cells.iter().zip(prev).filter(|(a, b)| a != b).count();
                    assert!(!work.full, "{name} frame {frame}: full-area path");
                    assert_eq!(work.changed, changed, "{name} frame {frame}");
                    // The counters sit where the painter maps a cell and where the encoder
                    // considers one (RND-R22): either stage scanning the screen fails here.
                    let bound = changed + usize::from(size.height());
                    assert!(
                        work.painted <= bound && work.encoded <= bound,
                        "{name} frame {frame}: {work:?}, {changed} changed"
                    );
                    assert!(work.sent <= work.encoded, "{name} frame {frame}: {work:?}");
                    assert_eq!(work.rows, usize::from(size.height()), "{name}: {work:?}");
                }
                last = Some(cells.to_vec());
            }
            // The same frame again: nothing is painted, considered or sent.
            draw(&rt, &mut screen, &mut term, 30.0);
            let work = screen.compositor.work();
            assert_eq!(
                (work.changed, work.painted, work.encoded, work.sent),
                (0, 0, 0, 0),
                "{name}"
            );
        }
    }

    #[test]
    fn rnd_r22_resize_theme_reset_and_overlay_take_the_full_area_path() {
        let mut rt = runtime(&["wave_interference"]);
        let (mut screen, mut term) = (Screen::new(), terminal());
        let full = |rt: &Runtime, screen: &mut Screen, term: &mut _| {
            draw(rt, screen, term, 30.0);
            screen.compositor.work().full
        };
        assert!(full(&rt, &mut screen, &mut term), "first frame");
        rt.tick(1.0 / 30.0);
        assert!(!full(&rt, &mut screen, &mut term), "steady frame");
        rt.handle(Action::CycleTheme);
        assert!(full(&rt, &mut screen, &mut term), "theme");
        rt.handle(Action::ToggleHud);
        assert!(full(&rt, &mut screen, &mut term), "HUD on");
        rt.tick(1.0 / 30.0);
        assert!(!full(&rt, &mut screen, &mut term), "steady with HUD");
        rt.handle(Action::Reset);
        assert!(
            full(&rt, &mut screen, &mut term),
            "reset clears the painter memory"
        );
        rt.handle(Action::ToggleHelp);
        assert!(full(&rt, &mut screen, &mut term), "help");
        rt.handle(Action::ToggleHelp);
        assert!(full(&rt, &mut screen, &mut term), "back from help");
        term.backend_mut().set_fixed_size(W - 4, H - 2);
        rt.resize(Size::new(W - 4, H - 2));
        assert!(full(&rt, &mut screen, &mut term), "resize");
    }

    /// A real scene that panics at its `boom_at`-th step, so the runtime quarantines it
    /// (RT-R3) and, once every scene has, shows the RT-R4 error line.
    struct LateBoom {
        inner: Box<dyn sciart_core::Generator>,
        steps: u32,
        boom_at: u32,
    }

    impl sciart_core::Generator for LateBoom {
        fn meta(&self) -> &'static sciart_core::Meta {
            self.inner.meta()
        }
        fn resize(&mut self, size: Size, ctx: &mut sciart_core::Ctx) {
            self.inner.resize(size, ctx);
        }
        fn update(&mut self, dt: f64, ctx: &mut sciart_core::Ctx) {
            self.steps += 1;
            assert!(self.steps < self.boom_at, "boom at step {}", self.steps);
            self.inner.update(dt, ctx);
        }
        fn prepare(&mut self) {
            self.inner.prepare();
        }
        fn render(&self, canvas: &mut sciart_core::Canvas) {
            self.inner.render(canvas);
        }
        fn caption(&self) -> Option<&'static str> {
            self.inner.caption()
        }
    }

    static WAVE_BOOM: Entry = Entry {
        meta: &sciart_generators::wave_interference::META,
        build: || {
            Box::new(LateBoom {
                inner: sciart_generators::wave_interference::build(),
                steps: 0,
                boom_at: 120,
            })
        },
    };
    static STAR_BOOM: Entry = Entry {
        meta: &sciart_generators::starfield::META,
        build: || {
            Box::new(LateBoom {
                inner: sciart_generators::starfield::build(),
                steps: 0,
                boom_at: 40,
            })
        },
    };

    #[test]
    fn rnd_r22_a_scene_switch_and_a_quarantined_scene_send_the_full_area_bytes() {
        // A key switches scenes, a panic quarantines one and switches again, the last
        // panic leaves the RT-R4 error line over the kept frame; both paths must agree.
        let size = Size::new(80, 24);
        let mut rt = Runtime::new(
            vec![&WAVE_BOOM, &STAR_BOOM],
            42,
            size,
            GlyphMode::Unicode,
            Some(0),
        );
        let opaque = |s: Screen| s.with_background(Background::Opaque);
        let terminal = |s: Screen| s.with_background(Background::Terminal);
        let mut pairs = vec![
            Pair::new("truecolor/opaque", || opaque(Screen::new()), size),
            Pair::new("truecolor/terminal", || terminal(Screen::new()), size),
            Pair::new("256/opaque", || opaque(Screen::indexed()), size),
            Pair::new("256/terminal", || terminal(Screen::indexed()), size),
        ];
        let mut seen: Vec<&str> = Vec::new();
        let mut error_frames = 0;
        for frame in 1..=110u32 {
            let action = match frame {
                8 | 50 => Some(Action::ToggleHud),
                15 => Some(Action::Next),
                104 | 106 => Some(Action::ToggleHelp),
                _ => None,
            };
            if let Some(action) = action {
                rt.handle(action);
            }
            rt.tick(1.0 / 30.0);
            if let Some(name) = rt.current_name()
                && seen.last() != Some(&name)
            {
                seen.push(name);
            }
            if rt.status() == Status::AllFailed {
                error_frames += 1;
            }
            let fps = 28.0 + f64::from(frame % 3);
            for pair in &mut pairs {
                pair.draw(&rt, fps);
            }
        }
        assert_eq!(
            seen,
            ["wave_interference", "starfield", "wave_interference"],
            "a key switch, then a quarantine switch"
        );
        assert_eq!(rt.status(), Status::AllFailed);
        assert!(
            error_frames >= 8,
            "the error line is drawn: {error_frames} frames"
        );
        let failures: Vec<String> = pairs.iter().filter_map(Pair::mismatch).collect();
        assert!(failures.is_empty(), "RND-R22:\n{}", failures.join("\n"));
    }

    // ----- M25: event-driven pause and help (RT-R21) -----

    /// A pacer whose rate has stepped down to its floor (RT-R17): 30 fps, floor 5.
    fn quiet_pacer() -> Pacer {
        let mut p = Pacer::new(30, 5, 600.0);
        p.record(2.0);
        assert!(p.at_floor() && p.fps() == 5);
        p
    }

    #[test]
    fn rt_r21_a_paused_scene_blocks_once_quiet_and_settled() {
        let pacer = quiet_pacer();
        let mut rt = runtime(&["starfield"]);
        assert!(!may_block(&rt, &pacer, 5.0, true), "running: never");
        rt.handle(Action::TogglePause);
        assert!(
            may_block(&rt, &pacer, 5.0, true),
            "paused, quiet, nothing sent"
        );
        assert!(!may_block(&rt, &pacer, 5.0, false), "the frame sent bytes");
        assert!(
            !may_block(&rt, &Pacer::new(30, 5, 600.0), 30.0, true),
            "the rate has not stepped down yet"
        );
        // The HUD shows the fps readout: wait until it reads the floor rate.
        let rt = rt.with_hud(true);
        assert!(!may_block(&rt, &pacer, 12.0, true), "readout still falling");
        assert!(may_block(&rt, &pacer, 5.2, true), "readout settled");
    }

    #[test]
    fn rt_r21_the_help_page_blocks_once_quiet_whatever_the_hidden_hud_reads() {
        let pacer = quiet_pacer();
        let mut rt = runtime(&["starfield"]).with_hud(true);
        rt.handle(Action::ToggleHelp);
        assert!(rt.help() && !rt.paused());
        assert!(
            may_block(&rt, &pacer, 12.0, true),
            "help hides the HUD and its readout"
        );
        assert!(!may_block(&rt, &pacer, 12.0, false), "the frame sent bytes");
        assert!(
            !may_block(&rt, &Pacer::new(30, 5, 600.0), 30.0, true),
            "the rate has not stepped down yet"
        );
        rt.handle(Action::ToggleHelp);
        assert!(
            !may_block(&rt, &pacer, 5.0, true),
            "back to a running scene"
        );
    }

    #[test]
    fn rt_r21_the_block_timeout_is_a_second_capped_by_a_deadline() {
        assert_eq!(block_timeout(None), Duration::from_secs(1));
        assert_eq!(
            block_timeout(Some(Duration::from_millis(300))),
            Duration::from_millis(300)
        );
        assert_eq!(
            block_timeout(Some(Duration::from_secs(5))),
            Duration::from_secs(1)
        );
        assert_eq!(block_timeout(Some(Duration::ZERO)), Duration::ZERO);
    }

    /// RT-R21 / RT-R23: a paused run's blocked wait ends at the `--for` deadline.
    #[test]
    fn rt_r21_rt_r23_a_paused_runs_blocked_wait_is_capped_by_its_deadline() {
        let mut rt = runtime(&["starfield"]).with_deadline(0.3);
        rt.handle(Action::TogglePause);
        assert_eq!(block_timeout(rt.remaining()), Duration::from_millis(300));
        assert_eq!(
            rt.pass_wall(0.3),
            Flow::Quit,
            "the wake-up then ends the run"
        );
        let rt = runtime(&["starfield"]);
        assert_eq!(block_timeout(rt.remaining()), BLOCK_POLL, "no deadline");
    }

    #[test]
    fn rt_r21_a_keep_awake_status_change_redraws_a_visible_hud() {
        let mut rt = runtime(&["starfield"]).with_hud(true);
        rt.handle(Action::TogglePause);
        assert!(redraw_after_poll(&rt, "caffeinate:on", "caffeinate:exited"));
        assert!(!redraw_after_poll(&rt, "caffeinate:on", "caffeinate:on"));
        rt.handle(Action::ToggleHelp);
        assert!(
            !redraw_after_poll(&rt, "caffeinate:on", "caffeinate:exited"),
            "the help page does not show the status"
        );
        rt.handle(Action::ToggleHelp);
        let rt = rt.with_hud(false);
        assert!(
            !redraw_after_poll(&rt, "caffeinate:on", "caffeinate:exited"),
            "no HUD, nothing shows the status"
        );
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
            env!("CARGO_PKG_VERSION"),
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
