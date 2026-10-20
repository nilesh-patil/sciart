//! Run a scene without a terminal session and print frames (LIB-R7).
//!
//! ```sh
//! cargo run -p sciart --no-default-features --example headless -- aurora 5
//! ```
//!
//! Prints the scene's frame after the given simulated seconds as ANSI text, then the same
//! frame as plain text, then the catalogue entry it came from.

use std::process::ExitCode;

use sciart::{Options, Stage, Theme};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "aurora".to_owned());
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5.0);

    let Some(scene) = sciart::find(&name) else {
        eprintln!("unknown scene '{name}'; try one of:");
        for scene in sciart::scenes() {
            eprintln!("  {:<28} {}", scene.name(), scene.title());
        }
        return ExitCode::from(2);
    };

    let run = || -> Result<(), sciart::Error> {
        let mut stage = Stage::new(scene.name(), 80, 24, Options::default())?;
        stage.advance(seconds)?;
        print!("{}", stage.to_ansi(Theme::Astro)?);
        print!("{}", stage.to_text()?);
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    println!(
        "{} ({}) at t = {seconds} s: {}",
        scene.name(),
        scene.category().label(),
        scene.description()
    );
    ExitCode::SUCCESS
}
