//! Terminal event loop: the only place that touches the wall clock and the terminal.
#![allow(clippy::disallowed_methods)] // wall clock is the binary's job (ADR-0008)

use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use sciart_core::{GlyphMode, Size};
use sciart_generators::Entry;
use sciart_render::{Hud, TerminalGuard, Theme, paint};

use crate::fps::FpsMeter;
use crate::keys::action_for;
use crate::runtime::{Flow, Runtime, Status, install_panic_filter};

#[derive(Debug, Clone)]
pub struct Options {
    pub entries: Vec<&'static Entry>,
    pub fps: u16,
    pub seed: u64,
    pub theme: Theme,
    pub glyphs: GlyphMode,
    pub hud: bool,
    /// Auto-cycle period; `None` for `run` (RT-R5).
    pub interval: Option<f64>,
}

pub fn run(opts: Options) -> anyhow::Result<()> {
    let mut guard = TerminalGuard::enter();
    install_panic_filter();
    let terminal = guard.terminal();

    let area = terminal.size()?;
    let size = Size::new(area.width, area.height);
    let mut runtime = Runtime::new(opts.entries, opts.seed, size, opts.glyphs, None)
        .with_hud(opts.hud)
        .with_theme(opts.theme);
    if let Some(interval) = opts.interval {
        runtime = runtime.with_interval(interval);
    }

    let frame_time = Duration::from_secs_f64(1.0 / f64::from(opts.fps));
    let mut fps = FpsMeter::default();
    let mut last = Instant::now();

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last).as_secs_f64();
        last = now;
        fps.record(elapsed);
        runtime.tick(elapsed);

        terminal.draw(|f| draw(f, &runtime, fps.fps(), opts.seed))?;

        // Handle input until the next frame is due.
        let next = now + frame_time;
        while event::poll(next.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                Event::Key(key) => {
                    if let Some(action) = action_for(key)
                        && runtime.handle(action) == Flow::Quit
                    {
                        return Ok(());
                    }
                }
                Event::Resize(w, h) => runtime.resize(Size::new(w, h)),
                _ => {}
            }
        }
    }
}

fn draw(f: &mut Frame<'_>, runtime: &Runtime, fps: f64, seed: u64) {
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
            fps,
            theme: runtime.theme(),
            seed,
            paused: runtime.paused(),
            quarantined: runtime.quarantined(),
        };
        f.render_widget(&hud, area);
    }
}
