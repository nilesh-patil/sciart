//! `sciart` - real-time generative science & astronomy art in the terminal.

mod app;
mod cli;
mod config;
mod fps;
mod keys;
mod pacer;
#[cfg(feature = "record")]
mod record;
mod runtime;

/// The unit-test binary counts heap calls for the RT-R14 frame-loop test (ADR-0009). The
/// counters are per thread, so the other tests running in parallel do not disturb it.
#[cfg(test)]
#[global_allocator]
static GLOBAL: sciart_testalloc::CountingAlloc = sciart_testalloc::CountingAlloc;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::Parser;
use sciart_core::GlyphMode;

use crate::cli::{Cli, Command, Filters, UsageError};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = cli.command();

    if let Command::List {
        json,
        categories,
        plain,
    } = command
    {
        // CFG-R2: `list` never loads the config; only CLI filters apply (CLI-R9).
        let filters = match Filters::from_cli(&command, &cli) {
            Ok(filters) => filters,
            Err(e) => {
                eprintln!("sciart: {e}");
                return ExitCode::from(2);
            }
        };
        let entries = match cli::select(&command, &[], &filters) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("sciart: {e}");
                return ExitCode::from(2);
            }
        };
        let format = cli::list_format(json, categories, plain, std::io::stdout().is_terminal());
        print!("{}", cli::list_output(&entries, format));
        return ExitCode::SUCCESS;
    }

    // CLI-R8: recording is headless (no config file, terminal or keep-awake).
    if matches!(command, Command::Record(_)) {
        return run_record(&cli, &command);
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

/// CLI-R8: validate the recording request (exit 2) and write it (I/O errors exit 1).
#[cfg(feature = "record")]
fn run_record(cli: &Cli, command: &Command) -> ExitCode {
    let spec = match cli::RecordSpec::resolve(cli, command) {
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(2);
        }
    };
    match record::run(&spec) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sciart: {e}");
            ExitCode::from(e.code())
        }
    }
}

/// CLI-R8: a build without the feature keeps the subcommand parsing, validates the request
/// the same way (so a typo is reported identically), then says the feature is missing (exit 2).
#[cfg(not(feature = "record"))]
fn run_record(cli: &Cli, command: &Command) -> ExitCode {
    if let Err(e) = cli::RecordSpec::resolve(cli, command) {
        eprintln!("sciart: {e}");
        return ExitCode::from(2);
    }
    eprintln!(
        "sciart: the `record` command is not compiled into this build; \
         reinstall with `cargo install --path crates/sciart --features record`"
    );
    ExitCode::from(2)
}

/// Load config, resolve settings (CFG-R1..R4) and pick the generators to run.
fn prepare(cli: &Cli, command: &Command) -> Result<app::Options, UsageError> {
    let default = config::default_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    );
    let file = config::load(cli.config.as_deref(), default.as_deref())?;
    let (settings, warnings) = config::resolve(cli, file, config::Env::from_process());
    for warning in warnings {
        eprintln!("sciart: warning: {warning}");
    }
    let filters = Filters::from_cli(command, cli)?;
    let entries = cli::select(command, &settings.disabled, &filters)?;
    // The help page shows the file in use, or where it would be (RT-R18).
    let config_path = cli.config.clone().or(default);

    #[allow(clippy::disallowed_methods)] // seeding from OS entropy is the binary's job
    let seed = settings.seed.unwrap_or_else(rand::random);
    Ok(app::Options {
        entries,
        fps: settings.fps,
        seed,
        theme: settings.theme,
        color: settings.color,
        background: settings.background,
        glyphs: if settings.ascii {
            GlyphMode::Ascii
        } else {
            GlyphMode::Unicode
        },
        hud: settings.hud,
        title: settings.title,
        interval: (*command == Command::Sim).then_some(settings.interval),
        speed: settings.speed,
        keep_awake: settings.keep_awake,
        low_power_fps: settings.low_power_fps,
        idle_after: settings.idle_after,
        config_path,
        favorites: settings.favorites,
    })
}
