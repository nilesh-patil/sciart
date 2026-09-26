//! Command-line interface (SPEC-005 CLI-R1..R4).

use std::fmt;

use clap::{Parser, Subcommand, ValueEnum};
use sciart_generators::{Entry, find, registry};
use sciart_render::Theme;

#[derive(Debug, Parser)]
#[command(
    name = "sciart",
    version,
    about = "Real-time generative science & astronomy art"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Frames per second
    #[arg(long, global = true, default_value_t = 30, value_parser = clap::value_parser!(u16).range(1..=120))]
    pub fps: u16,

    /// RNG seed for a reproducible run (random when omitted)
    #[arg(long, global = true)]
    pub seed: Option<u64>,

    /// Color theme
    #[arg(long, global = true, value_enum, default_value_t = ThemeArg::Astro)]
    pub theme: ThemeArg,

    /// Use pure-ASCII glyphs
    #[arg(long, global = true)]
    pub ascii: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Cycle through generators (default)
    Sim,
    /// Run a single generator
    Run { name: String },
    /// List generators
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThemeArg {
    Astro,
    Rainbow,
    Mono,
}

impl From<ThemeArg> for Theme {
    fn from(t: ThemeArg) -> Theme {
        match t {
            ThemeArg::Astro => Theme::Astro,
            ThemeArg::Rainbow => Theme::Rainbow,
            ThemeArg::Mono => Theme::Mono,
        }
    }
}

impl Cli {
    /// No subcommand means `sim` (CLI-R1).
    pub fn command(&self) -> Command {
        self.command.clone().unwrap_or(Command::Sim)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnknownGenerator(pub String);

impl fmt::Display for UnknownGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = registry().iter().map(|e| e.meta.name).collect();
        write!(
            f,
            "unknown generator '{}'. Available: {}",
            self.0,
            names.join(", ")
        )
    }
}

/// Generators a `sim`/`run` command will use (CLI-R2).
pub fn select(command: &Command) -> Result<Vec<&'static Entry>, UnknownGenerator> {
    match command {
        Command::Run { name } => find(name)
            .map(|e| vec![e])
            .ok_or_else(|| UnknownGenerator(name.clone())),
        Command::Sim | Command::List => Ok(registry().iter().collect()),
    }
}

/// `name<TAB>category<TAB>description`, sorted by name (CLI-R4).
pub fn list_lines() -> Vec<String> {
    registry()
        .iter()
        .map(|e| {
            format!(
                "{}\t{}\t{}",
                e.meta.name,
                e.meta.category.label(),
                e.meta.description
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("sciart").chain(args.iter().copied()))
    }

    #[test]
    fn cli_r1_no_subcommand_is_sim() {
        assert_eq!(parse(&[]).unwrap().command(), Command::Sim);
        assert_eq!(parse(&["list"]).unwrap().command(), Command::List);
    }

    #[test]
    fn cli_r1_global_flags_work_after_subcommand() {
        let cli = parse(&[
            "run",
            "starfield",
            "--fps",
            "60",
            "--seed",
            "7",
            "--ascii",
            "--theme",
            "mono",
        ])
        .unwrap();
        assert_eq!(
            cli.command(),
            Command::Run {
                name: "starfield".into()
            }
        );
        assert_eq!(
            (cli.fps, cli.seed, cli.ascii, cli.theme),
            (60, Some(7), true, ThemeArg::Mono)
        );
    }

    #[test]
    fn cli_r2_unknown_generator_is_an_error_listing_valid_names() {
        let err = select(&Command::Run {
            name: "nope".into(),
        })
        .unwrap_err();
        assert_eq!(err, UnknownGenerator("nope".into()));
        assert!(err.to_string().contains("starfield"));
        assert_eq!(
            select(&Command::Run {
                name: "starfield".into()
            })
            .unwrap()
            .len(),
            1
        );
        assert_eq!(select(&Command::Sim).unwrap().len(), registry().len());
    }

    #[test]
    fn cli_r3_fps_out_of_range_exits_with_code_2() {
        for bad in ["0", "121", "-1", "fast"] {
            let err = parse(&["--fps", bad]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "--fps {bad}");
        }
        assert_eq!(parse(&["--fps", "120"]).unwrap().fps, 120);
        assert_eq!(parse(&[]).unwrap().fps, 30);
    }

    #[test]
    fn cli_r4_list_is_tab_separated_and_sorted() {
        let lines = list_lines();
        assert_eq!(lines.len(), registry().len());
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted);
        assert!(lines.iter().all(|l| l.split('\t').count() == 3));
        assert!(lines.contains(&format!(
            "starfield\tbasic\t{}",
            sciart_generators::starfield::META.description
        )));
    }
}
