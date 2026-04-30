//! `sciart` - real-time generative science & astronomy art in the terminal.

mod app;
mod cli;
mod config;
mod fps;
mod keys;
mod pacer;
mod runtime;

use std::process::ExitCode;

use clap::Parser;
use sciart_core::GlyphMode;

use crate::cli::{Cli, Command, UsageError};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = cli.command();

    if command == Command::List {
        for line in cli::list_lines() {
            println!("{line}");
        }
        return ExitCode::SUCCESS;
    }

    let opts = match prepare(&cli, &command) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(2);
        }
    };

    // KA-R2: a keep-awake failure is a warning; the art still runs.
    let keep_awake = sciart_keepawake::start(opts.keep_awake, opts.seed).unwrap_or_else(|e| {
        eprintln!("sciart: warning: {e}");
        sciart_keepawake::Guard::failed(opts.keep_awake)
    });

    match app::run(opts, keep_awake) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sciart: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Load config, resolve settings (CFG-R1..R4) and pick the generators to run.
fn prepare(cli: &Cli, command: &Command) -> Result<app::Options, UsageError> {
    let default = config::default_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    );
    let file = config::load(cli.config.as_deref(), default.as_deref())?;
    let (settings, warnings) = config::resolve(cli, file);
    for warning in warnings {
        eprintln!("sciart: warning: {warning}");
    }
    let entries = cli::select(command, &settings.disabled)?;

    #[allow(clippy::disallowed_methods)] // seeding from OS entropy is the binary's job
    let seed = settings.seed.unwrap_or_else(rand::random);
    Ok(app::Options {
        entries,
        fps: settings.fps,
        seed,
        theme: settings.theme,
        glyphs: if settings.ascii {
            GlyphMode::Ascii
        } else {
            GlyphMode::Unicode
        },
        hud: settings.hud,
        title: settings.title,
        interval: (*command == Command::Sim).then_some(settings.interval),
        keep_awake: settings.keep_awake,
        low_power_fps: settings.low_power_fps,
        idle_after: settings.idle_after,
    })
}
