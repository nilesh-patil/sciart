//! A scene inside someone else's ratatui application (LIB-R7).
//!
//! ```sh
//! cargo run -p sciart --no-default-features --features widget --example widget -- saturn_rings
//! ```
//!
//! The scene fills the left two thirds of the screen, a panel with its catalogue entry the
//! rest; `n` moves to the next scene, `q` or `Esc` quits.

use std::error::Error;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Paragraph, Wrap};
use sciart::{Options, SceneInfo, SceneWidget, Stage, Theme};

fn main() -> Result<(), Box<dyn Error>> {
    let first = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "saturn_rings".to_owned());
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &first);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, first: &str) -> Result<(), Box<dyn Error>> {
    let scenes: Vec<SceneInfo> = sciart::scenes().collect();
    let mut index = scenes
        .iter()
        .position(|s| s.name() == first)
        .unwrap_or_default();
    let mut stage = Stage::new(scenes[index].name(), 1, 1, Options::default())?;
    #[allow(clippy::disallowed_methods)] // a real application paces its frames by wall time
    let mut last = Instant::now();
    loop {
        let scene = stage.scene();
        terminal.draw(|frame| {
            let [art, panel] =
                Layout::horizontal([Constraint::Percentage(67), Constraint::Percentage(33)])
                    .areas(frame.area());
            frame.render_widget(SceneWidget::new(&mut stage).theme(Theme::Astro), art);
            let text = format!(
                "{}\n\n{}\n\n{}\n\nn: next scene · q: quit",
                scene.title(),
                scene.category().label(),
                scene.description()
            );
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: true })
                    .block(Block::bordered().title(scene.name())),
                panel,
            );
        })?;
        if event::poll(Duration::from_millis(33))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('n') => {
                    index = (index + 1) % scenes.len();
                    stage = Stage::new(scenes[index].name(), 1, 1, Options::default())?;
                }
                _ => {}
            }
        }
        #[allow(clippy::disallowed_methods)] // see above
        let now = Instant::now();
        stage.advance((now - last).as_secs_f64())?;
        last = now;
    }
}
