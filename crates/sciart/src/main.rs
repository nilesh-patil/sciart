//! `sciart` - real-time generative science & astronomy art in the terminal.

mod app;
mod child;
mod cli;
mod config;
mod fps;
mod hint;
mod keys;
mod pacer;
#[cfg(feature = "record")]
mod record;
mod runtime;
#[cfg(unix)]
mod signals;

/// The unit-test binary counts heap calls for the RT-R14 frame-loop test (ADR-0009). The
/// counters are per thread, so the other tests running in parallel do not disturb it.
#[cfg(test)]
#[global_allocator]
static GLOBAL: sciart_testalloc::CountingAlloc = sciart_testalloc::CountingAlloc;

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use sciart_core::GlyphMode;
use sciart_keepawake::Guard;

use crate::app::End;
use crate::child::{Child, Followed};
use crate::cli::{Cli, Command, Filters, UsageError};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = cli.command();

    match &command {
        Command::List {
            json,
            categories,
            plain,
        } => return run_list(&cli, &command, *json, *categories, *plain),
        // CLI-R8: recording is headless (no config file, terminal or keep-awake).
        Command::Record(_) => return run_record(&cli, &command),
        // CLI-R17: generated from the parser; no config, nothing started.
        Command::Completions { shell } => return print(&cli::completions(*shell)),
        Command::Man => {
            return match cli::man_page() {
                Ok(page) => print(&page),
                Err(e) => {
                    eprintln!("sciart: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        Command::Sim | Command::Run { .. } | Command::While(_) => {}
    }

    let (opts, hint) = match prepare(&cli, &command) {
        Ok(prepared) => prepared,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(2);
        }
    };

    let code = match &command {
        Command::While(args) => run_while(opts, &args.command),
        _ => run_art(opts),
    };
    // RND-R21: advice goes out once, after the terminal is restored, so it stays readable.
    if let Some(hint) = hint {
        eprintln!("{hint}");
    }
    code
}

/// CLI-R4/R10/R16: the catalogue. CFG-R2: `list` never loads the config; only CLI filters
/// apply (CLI-R9).
fn run_list(cli: &Cli, command: &Command, json: bool, categories: bool, plain: bool) -> ExitCode {
    let entries = match Filters::from_cli(command, cli).and_then(|f| cli::select(command, &[], &f))
    {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(2);
        }
    };
    let format = cli::list_format(json, categories, plain, io::stdout().is_terminal());
    print(cli::list_output(&entries, format).as_bytes())
}

/// Write `bytes` to stdout; a closed pipe is reported, not a panic.
fn print(bytes: &[u8]) -> ExitCode {
    let mut out = io::stdout().lock();
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sciart: {e}");
            ExitCode::FAILURE
        }
    }
}

/// KA-R2: a keep-awake failure is a warning; the art still runs.
fn start_keep_awake(opts: &app::Options) -> Guard {
    sciart_keepawake::start(opts.keep_awake, opts.seed).unwrap_or_else(|e| {
        eprintln!("sciart: warning: {e}");
        Guard::failed(opts.keep_awake)
    })
}

/// `sim` and `run`: a key or the `--for` deadline ends them, all with status 0 (CLI-R12).
fn run_art(opts: app::Options) -> ExitCode {
    let mut keep_awake = start_keep_awake(&opts);
    match app::run(opts, &mut keep_awake, || false) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sciart: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// CLI-R14: the art, with keep-awake, for as long as `argv` runs; then its output and its
/// exit status.
fn run_while(opts: app::Options, argv: &[OsString]) -> ExitCode {
    // RT-R20 / CLI-R14: the signal watcher runs before the command starts, and the command
    // starts registered with it, so a signal at any point kills it before the exit.
    #[cfg(unix)]
    if let Err(e) = signals::watch() {
        eprintln!("sciart: {e}");
        return ExitCode::FAILURE;
    }
    // Started before the terminal is touched, so a command that cannot start is a plain
    // one-line error (exit 127) and the alternate screen is never entered.
    let start = || Child::start(argv, &std::env::temp_dir());
    #[cfg(unix)]
    let started = signals::start_command(start);
    #[cfg(not(unix))]
    let started = start();
    let mut child = match started {
        Ok(child) => child,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(e.code());
        }
    };
    // After `q` the deadline still holds, on the same wall clock as the art's.
    let deadline = opts
        .run_for
        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
        .map(|secs| app::wall_clock() + secs);
    // Keep-awake lasts until the command ends, also after `q` closed the art.
    let mut keep_awake = start_keep_awake(&opts);
    // A panic outside the scene quarantine still replays what was spooled (CLI-R14).
    let ended = child.beside(&mut io::stdout(), &mut io::stderr(), |child| {
        app::run(opts, &mut keep_awake, || {
            // An error asking counts as an exit; `stop` below reports the real status.
            !matches!(child.try_status(), Ok(None))
        })
    });

    // The terminal is restored: the spooled output can follow, stdout first.
    let (mut out, mut err) = (io::stdout(), io::stderr());
    let mut replay = |child: &mut Child| {
        if let Err(e) = child.replay(&mut out, &mut err) {
            eprintln!("sciart: cannot copy the command's output: {e}");
        }
    };
    let code = match ended {
        Ok(End::Quit) => {
            let followed = child.follow(&mut io::stdout(), &mut io::stderr(), deadline, |d| {
                keep_awake.poll(d.as_secs_f64());
            });
            match followed {
                Ok(Followed::Exited(status)) => Ok(child::exit_code(status)),
                Ok(Followed::Deadline) => child.stop().map(|_| child::EXIT_DEADLINE),
                Err(e) => Err(e),
            }
        }
        Ok(End::Done) => child.stop().map(child::exit_code),
        Ok(End::Interrupt) => child.stop().map(|_| child::EXIT_INTERRUPTED),
        Ok(End::Deadline) => child.stop().map(|_| child::EXIT_DEADLINE),
        Err(e) => {
            eprintln!("sciart: {e:#}");
            child.stop().map(|_| 1)
        }
    };
    replay(&mut child);
    match code {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("sciart: {e}");
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

/// Load config, resolve settings (CFG-R1..R4) and pick the generators to run; also the
/// terminal hint to print on exit, unless the colour mode was chosen explicitly (RND-R21).
fn prepare(
    cli: &Cli,
    command: &Command,
) -> Result<(app::Options, Option<&'static str>), UsageError> {
    let default = config::default_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    );
    let file = config::load(cli.config.as_deref(), default.as_deref())?;
    let env = config::Env::from_process();
    let hint = if config::color_explicit(cli, &file, env) {
        None
    } else {
        hint::terminal_hint(&hint::TermEnv::from_process())
    };
    let (settings, warnings) = config::resolve(cli, file, env);
    for warning in warnings {
        eprintln!("sciart: warning: {warning}");
    }
    let filters = Filters::from_cli(command, cli)?;
    let entries = cli::select(command, &settings.disabled, &filters)?;
    // The help page shows the file in use, or where it would be (RT-R18).
    let config_path = cli.config.clone().or(default);

    #[allow(clippy::disallowed_methods)] // seeding from OS entropy is the binary's job
    let seed = settings.seed.unwrap_or_else(rand::random);
    let opts = app::Options {
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
        // `while` shows the `sim` cycle (CLI-R14).
        interval: matches!(command, Command::Sim | Command::While(_)).then_some(settings.interval),
        speed: settings.speed,
        keep_awake: settings.keep_awake,
        low_power_fps: settings.low_power_fps,
        idle_after: settings.idle_after,
        config_path,
        favorites: settings.favorites,
        run_for: cli.run_for,
    };
    Ok((opts, hint))
}
