//! Command-line interface (SPEC-005 CLI-R1..R5).

use std::fmt;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use sciart_generators::{Entry, find, registry};
use sciart_keepawake::Mode;
use sciart_render::Theme;
use serde::Deserialize;

#[derive(Debug, Parser)]
#[command(
    name = "sciart",
    version,
    about = "Real-time generative science & astronomy art"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Frames per second [default: 30]
    #[arg(long, global = true, value_parser = clap::value_parser!(u16).range(1..=120))]
    pub fps: Option<u16>,

    /// Seconds between automatic scene changes in `sim` [default: 15]
    #[arg(long, global = true, value_parser = positive_secs)]
    pub interval: Option<f64>,

    /// RNG seed for a reproducible run (random when omitted)
    #[arg(long, global = true)]
    pub seed: Option<u64>,

    /// Color theme [default: astro]
    #[arg(long, global = true, value_enum)]
    pub theme: Option<ThemeArg>,

    /// Use pure-ASCII glyphs
    #[arg(long, global = true)]
    pub ascii: bool,

    /// Start with the status line (HUD) visible
    #[arg(long, global = true)]
    pub hud: bool,

    /// How to keep the machine awake [default: caffeinate on macOS, off elsewhere]
    #[arg(long, global = true, value_enum)]
    pub keep_awake: Option<KeepAwakeArg>,

    /// Config file [default: $XDG_CONFIG_HOME/sciart/config.toml or ~/.config/sciart/config.toml]
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// CLI-R6 / CFG-R5 spelling of `sciart_keepawake::Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeepAwakeArg {
    Caffeinate,
    Mouse,
    Off,
}

impl From<KeepAwakeArg> for Mode {
    fn from(k: KeepAwakeArg) -> Mode {
        match k {
            KeepAwakeArg::Caffeinate => Mode::Caffeinate,
            KeepAwakeArg::Mouse => Mode::Mouse,
            KeepAwakeArg::Off => Mode::Off,
        }
    }
}

/// CLI-R5: a finite number of seconds > 0.
fn positive_secs(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0.0 => Ok(v),
        _ => Err(format!("'{s}' is not a positive number of seconds")),
    }
}

impl Cli {
    /// No subcommand means `sim` (CLI-R1).
    pub fn command(&self) -> Command {
        self.command.clone().unwrap_or(Command::Sim)
    }
}

/// Errors that exit with code 2 (CLI-R2, CFG-R2..R4).
#[derive(Debug, PartialEq)]
pub enum UsageError {
    UnknownGenerator(String),
    AllDisabled,
    Config(String),
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsageError::UnknownGenerator(name) => {
                let names: Vec<_> = registry().iter().map(|e| e.meta.name).collect();
                write!(
                    f,
                    "unknown generator '{name}'. Available: {}",
                    names.join(", ")
                )
            }
            UsageError::AllDisabled => {
                f.write_str("every generator is disabled in the config file; nothing to show")
            }
            UsageError::Config(msg) => write!(f, "config: {msg}"),
        }
    }
}

/// Generators a command will use. `disabled` filters `sim` only (CLI-R2, CFG-R4).
pub fn select(command: &Command, disabled: &[String]) -> Result<Vec<&'static Entry>, UsageError> {
    match command {
        Command::Run { name } => find(name)
            .map(|e| vec![e])
            .ok_or_else(|| UsageError::UnknownGenerator(name.clone())),
        Command::Sim | Command::List => {
            let entries: Vec<_> = registry()
                .iter()
                .filter(|e| !disabled.iter().any(|d| d == e.meta.name))
                .collect();
            if entries.is_empty() {
                Err(UsageError::AllDisabled)
            } else {
                Ok(entries)
            }
        }
    }
}

/// `name<TAB>category id<TAB>description`, sorted by name (CLI-R4).
pub fn list_lines() -> Vec<String> {
    registry()
        .iter()
        .map(|e| {
            format!(
                "{}\t{}\t{}",
                e.meta.name,
                e.meta.category.id(),
                e.meta.description
            )
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("sciart").chain(args.iter().copied()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "--hud",
            "--theme",
            "mono",
            "--interval",
            "2.5",
        ])
        .unwrap();
        assert_eq!(
            cli.command(),
            Command::Run {
                name: "starfield".into()
            }
        );
        assert_eq!(
            (
                cli.fps,
                cli.seed,
                cli.ascii,
                cli.hud,
                cli.theme,
                cli.interval
            ),
            (
                Some(60),
                Some(7),
                true,
                true,
                Some(ThemeArg::Mono),
                Some(2.5)
            )
        );
    }

    #[test]
    fn cli_r2_unknown_generator_is_an_error_listing_valid_names() {
        let err = select(
            &Command::Run {
                name: "nope".into(),
            },
            &[],
        )
        .unwrap_err();
        assert_eq!(err, UsageError::UnknownGenerator("nope".into()));
        assert!(err.to_string().contains("starfield"));
        let run = Command::Run {
            name: "starfield".into(),
        };
        assert_eq!(select(&run, &[]).unwrap().len(), 1);
        assert_eq!(select(&Command::Sim, &[]).unwrap().len(), registry().len());
    }

    #[test]
    fn cli_r3_fps_out_of_range_exits_with_code_2() {
        for bad in ["0", "121", "-1", "fast"] {
            let err = parse(&["--fps", bad]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "--fps {bad}");
        }
        assert_eq!(parse(&["--fps", "120"]).unwrap().fps, Some(120));
        assert_eq!(parse(&[]).unwrap().fps, None, "default applied in Settings");
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
            "starfield\tdeep-cosmos\t{}",
            sciart_generators::starfield::META.description
        )));
        for (old, _) in sciart_generators::ALIASES {
            assert!(
                !lines.iter().any(|l| l.starts_with(&format!("{old}\t"))),
                "GEN-R14: retired name {old} listed"
            );
        }
    }

    #[test]
    fn cli_r2_gen_r14_run_accepts_a_retired_name() {
        let run = Command::Run {
            name: "chemical_reaction".into(),
        };
        let picked = select(&run, &[]).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].meta.name, "gray_scott");
    }

    #[test]
    fn cli_r5_interval_must_be_positive_and_finite() {
        for bad in ["0", "-1", "nan", "inf", "soon"] {
            let err = parse(&["--interval", bad]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "--interval {bad}");
        }
        assert_eq!(parse(&["--interval", "0.5"]).unwrap().interval, Some(0.5));
    }

    #[test]
    fn cli_r6_keep_awake_flag() {
        for (arg, mode) in [
            ("caffeinate", Mode::Caffeinate),
            ("mouse", Mode::Mouse),
            ("off", Mode::Off),
        ] {
            let cli = parse(&["--keep-awake", arg]).unwrap();
            assert_eq!(cli.keep_awake.map(Mode::from), Some(mode));
        }
        assert_eq!(parse(&[]).unwrap().keep_awake, None);
        assert_eq!(
            parse(&["--keep-awake", "jiggle"]).unwrap_err().exit_code(),
            2
        );
    }

    #[test]
    fn cfg_r4_disabled_filters_sim_but_not_run() {
        let all: Vec<String> = registry().iter().map(|e| e.meta.name.to_string()).collect();
        assert!(matches!(
            select(&Command::Sim, &all),
            Err(UsageError::AllDisabled)
        ));
        let run = Command::Run {
            name: "starfield".into(),
        };
        assert_eq!(select(&run, &all).unwrap().len(), 1);
        let unrelated = vec!["not_a_generator".to_string()];
        assert_eq!(
            select(&Command::Sim, &unrelated).unwrap().len(),
            registry().len()
        );
    }
}
