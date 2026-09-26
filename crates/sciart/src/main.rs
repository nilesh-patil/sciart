//! `sciart` - real-time generative science & astronomy art in the terminal.

mod app;
mod cli;
mod runtime;

use std::process::ExitCode;

use clap::Parser;
use sciart_core::GlyphMode;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = cli.command();

    if command == Command::List {
        for line in cli::list_lines() {
            println!("{line}");
        }
        return ExitCode::SUCCESS;
    }

    let entries = match cli::select(&command) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(2);
        }
    };

    #[allow(clippy::disallowed_methods)] // seeding from OS entropy is the binary's job
    let seed = cli.seed.unwrap_or_else(rand::random);
    let opts = app::Options {
        fps: cli.fps,
        seed,
        theme: cli.theme.into(),
        glyphs: if cli.ascii {
            GlyphMode::Ascii
        } else {
            GlyphMode::Unicode
        },
    };

    match app::run(entries, opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sciart: {e:#}");
            ExitCode::FAILURE
        }
    }
}
