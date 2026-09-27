//! Terminal event loop: the only place that touches the wall clock and the terminal.

use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use sciart_core::{GlyphMode, Size};
use sciart_generators::Entry;
use sciart_keepawake::{Guard, Mode};
use sciart_render::{Hud, TerminalGuard, Theme, paint};

use crate::fps::FpsMeter;
use crate::keys::action_for;
use crate::pacer::Pacer;
use crate::runtime::{Flow, Runtime, Status, install_panic_filter};

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
    pub keep_awake: Mode,
    pub low_power_fps: u16,
    pub idle_after: f64,
}

/// What the HUD needs besides the runtime.
struct HudExtras<'a> {
    fps: f64,
    seed: u64,
    keep_awake: &'a str,
    low_power: bool,
}

pub fn run(opts: Options, mut keep_awake: Guard) -> anyhow::Result<()> {
    let mut guard = TerminalGuard::enter();
    install_panic_filter();
    let terminal = guard.terminal();

    let area = terminal.size()?;
    let size = Size::new(area.width, area.height);
    let mut runtime = Runtime::new(opts.entries, opts.seed, size, opts.glyphs, None)
        .with_hud(opts.hud)
        .with_title(opts.title)
        .with_theme(opts.theme);
    if let Some(interval) = opts.interval {
        runtime = runtime.with_interval(interval);
    }

    let mut pacer = Pacer::new(opts.fps, opts.low_power_fps, opts.idle_after);
    let mut fps = FpsMeter::default();
    let mut last = wall_clock();

    loop {
        let now = wall_clock();
        let elapsed = now.duration_since(last).as_secs_f64();
        last = now;
        fps.record(elapsed);
        pacer.record(elapsed);
        keep_awake.poll(elapsed);
        runtime.tick(elapsed);

        let status = HudExtras {
            fps: fps.fps(),
            seed: opts.seed,
            keep_awake: &keep_awake.status(),
            low_power: pacer.low_power(),
        };
        terminal.draw(|f| draw(f, &runtime, &status))?;

        // Handle input until the next frame is due (RT-R8: the rate can change here).
        let frame_time = Duration::from_secs_f64(1.0 / f64::from(pacer.fps()));
        let next = now + frame_time;
        while event::poll(next.saturating_duration_since(wall_clock()))? {
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
                Event::Resize(w, h) => runtime.resize(Size::new(w, h)),
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

fn draw(f: &mut Frame<'_>, runtime: &Runtime, status: &HudExtras<'_>) {
    let area = f.area();
    paint(runtime.frame(), runtime.theme(), area, f.buffer_mut());
    if runtime.status() == Status::AllFailed {
        // RT-R4: keep the last frame, explain on the bottom row.
        let msg = runtime.notices().last().map_or("", String::as_str);
        let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        f.render_widget(Line::from(msg).reversed(), row);
    } else if runtime.hud() {
        let hud = Hud {
            generator: runtime.current_name().unwrap_or("-"),
            title: runtime.title(),
            fps: status.fps,
            theme: runtime.theme(),
            seed: status.seed,
            keep_awake: status.keep_awake,
            low_power: status.low_power,
            paused: runtime.paused(),
            quarantined: runtime.quarantined(),
        };
        f.render_widget(&hud, area);
    }
}
