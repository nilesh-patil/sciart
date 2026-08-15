//! Command-line interface (SPEC-005 CLI-R1..R8).

use std::fmt;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use sciart_core::Size;
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

    /// Hide the science title (the HUD then shows the scene id)
    #[arg(long, global = true)]
    pub no_title: bool,

    /// How to keep the machine awake [default: caffeinate on macOS, off elsewhere]
    #[arg(long, global = true, value_enum)]
    pub keep_awake: Option<KeepAwakeArg>,

    /// Config file [default: $XDG_CONFIG_HOME/sciart/config.toml or ~/.config/sciart/config.toml]
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum Command {
    /// Cycle through generators (default)
    Sim,
    /// Run a single generator
    Run { name: String },
    /// List generators
    List,
    /// Render a scene to GIF, PNG frames or an asciicast (CLI-R8; needs the `record` feature)
    Record(RecordArgs),
}

/// CLI-R8: the `record` subcommand's own flags. `--fps`, `--theme`, `--ascii` and `--seed`
/// are the global flags, so a recording uses the same spelling as the rest of the CLI.
#[derive(Debug, Clone, PartialEq, Args)]
pub struct RecordArgs {
    /// Scene to record (registered name or retired alias)
    pub scene: String,
    /// Output: a .gif file, a PNG-frame directory (.png), or an asciicast (.cast/.json)
    #[arg(long)]
    pub out: PathBuf,
    /// Simulated seconds to record [default: 5]
    #[arg(long, value_parser = positive_secs)]
    pub seconds: Option<f64>,
    /// Number of frames; overrides --seconds (CLI-R8 caps a recording at 36 000 frames)
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=36_000))]
    pub frames: Option<u32>,
    /// Cell size WxH [default: 80x24]
    #[arg(long, value_parser = size_arg)]
    pub size: Option<Size>,
    /// Output format; inferred from --out when omitted
    #[arg(long, value_enum)]
    pub format: Option<FormatArg>,
}

/// CLI-R8 output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    Gif,
    Png,
    Asciicast,
}

impl FormatArg {
    /// CLI-R8: `.gif`, `.png`, `.cast`/`.json`; anything else (including no extension) is gif.
    pub fn infer(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => Self::Png,
            Some("cast" | "json") => Self::Asciicast,
            _ => Self::Gif,
        }
    }
}

/// CLI-R8 defaults; recording is headless and does not read the config file.
pub const RECORD_DEFAULT_SECONDS: f64 = 5.0;
pub const RECORD_DEFAULT_FPS: u16 = 30;
pub const RECORD_DEFAULT_SIZE: Size = Size::new(80, 24);
pub const RECORD_DEFAULT_SEED: u64 = 42;
/// CLI-R8: a recording is capped at 36 000 frames (20 min at 30 fps), so a huge `--seconds`
/// is a usage error instead of an allocation/write that never ends.
pub const RECORD_MAX_FRAMES: usize = 36_000;

/// A validated `record` invocation: CLI-R8 defaults applied, scene resolved, timing checked.
#[cfg_attr(not(feature = "record"), allow(dead_code))] // read by the record implementation
#[derive(Debug)]
pub struct RecordSpec {
    pub entry: &'static Entry,
    pub out: PathBuf,
    pub format: FormatArg,
    pub frames: usize,
    pub fps: u16,
    pub size: Size,
    pub theme: Theme,
    pub ascii: bool,
    pub seed: u64,
}

impl RecordSpec {
    /// Resolve CLI-R8: every value is either explicit or its documented default; anything
    /// unknown (scene, fps that does not divide 60) is a `UsageError` (exit 2).
    pub fn resolve(cli: &Cli, command: &Command) -> Result<Self, UsageError> {
        let Command::Record(args) = command else {
            return Err(UsageError::Record("not a record command".to_owned()));
        };
        let fps = cli.fps.unwrap_or(RECORD_DEFAULT_FPS);
        if 60 % fps != 0 {
            return Err(UsageError::RecordFps(fps));
        }
        let entry =
            find(&args.scene).ok_or_else(|| UsageError::UnknownGenerator(args.scene.clone()))?;
        // CLI-R8: `--frames` overrides `--seconds`; without either, 5 s at the chosen fps.
        let frames = match args.frames {
            Some(frames) => frames as usize,
            None => {
                let seconds = args.seconds.unwrap_or(RECORD_DEFAULT_SECONDS);
                (seconds * f64::from(fps)).round().max(1.0) as usize
            }
        };
        if frames > RECORD_MAX_FRAMES {
            return Err(UsageError::RecordFrames(frames));
        }
        Ok(Self {
            entry,
            out: args.out.clone(),
            format: args.format.unwrap_or_else(|| FormatArg::infer(&args.out)),
            frames,
            fps,
            size: args.size.unwrap_or(RECORD_DEFAULT_SIZE),
            theme: cli.theme.map(Theme::from).unwrap_or(Theme::Astro),
            ascii: cli.ascii,
            seed: cli.seed.unwrap_or(RECORD_DEFAULT_SEED),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeArg {
    Astro,
    Rainbow,
    Mono,
    Aurora,
}

impl From<ThemeArg> for Theme {
    fn from(t: ThemeArg) -> Theme {
        match t {
            ThemeArg::Astro => Theme::Astro,
            ThemeArg::Rainbow => Theme::Rainbow,
            ThemeArg::Mono => Theme::Mono,
            ThemeArg::Aurora => Theme::Aurora,
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

/// CLI-R8: `WxH` in cells, both at least 1.
fn size_arg(s: &str) -> Result<Size, String> {
    let dim = |v: &str| v.parse::<u16>().ok().filter(|&n| n > 0);
    s.split_once('x')
        .and_then(|(w, h)| dim(w).zip(dim(h)))
        .map(|(w, h)| Size::new(w, h))
        .ok_or_else(|| format!("'{s}' is not WxH with positive sides (e.g. 80x24)"))
}

impl Cli {
    /// No subcommand means `sim` (CLI-R1).
    pub fn command(&self) -> Command {
        self.command.clone().unwrap_or(Command::Sim)
    }
}

/// Errors that exit with code 2 (CLI-R2, CFG-R2..R4, CLI-R8).
#[derive(Debug, PartialEq)]
pub enum UsageError {
    UnknownGenerator(String),
    AllDisabled,
    Config(String),
    /// CLI-R8: frame k must land on whole 1/60 s steps.
    RecordFps(u16),
    /// CLI-R8: the recording cap (36 000 frames) was exceeded.
    RecordFrames(usize),
    Record(String),
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
            UsageError::RecordFps(fps) => write!(
                f,
                "--fps {fps} does not divide 60: recording needs a whole number of 1/60 s \
                 steps per frame (1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30, 60)"
            ),
            UsageError::RecordFrames(frames) => write!(
                f,
                "a recording is limited to {RECORD_MAX_FRAMES} frames (20 min at 30 fps); \
                 this request is {frames} frames"
            ),
            UsageError::Record(msg) => write!(f, "record: {msg}"),
        }
    }
}

/// Generators a command will use. `disabled` filters `sim` only, retired names included
/// (CLI-R2, CFG-R4, CFG-R7).
pub fn select(command: &Command, disabled: &[String]) -> Result<Vec<&'static Entry>, UsageError> {
    match command {
        Command::Run { name } => find(name)
            .map(|e| vec![e])
            .ok_or_else(|| UsageError::UnknownGenerator(name.clone())),
        Command::Record(args) => find(&args.scene)
            .map(|e| vec![e])
            .ok_or_else(|| UsageError::UnknownGenerator(args.scene.clone())),
        Command::Sim | Command::List => {
            let disabled: Vec<&str> = disabled
                .iter()
                .filter_map(|d| find(d))
                .map(|e| e.meta.name)
                .collect();
            let entries: Vec<_> = registry()
                .iter()
                .filter(|e| !disabled.contains(&e.meta.name))
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
    fn cli_r7_no_title_flag() {
        assert!(!parse(&[]).unwrap().no_title);
        assert!(parse(&["--no-title"]).unwrap().no_title);
        assert!(parse(&["run", "starfield", "--no-title"]).unwrap().no_title);
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

    /// Parse and resolve in one step, as `main` does.
    fn record(argv: &[&str]) -> (Cli, RecordSpec) {
        let cli = parse(argv).unwrap();
        let spec = RecordSpec::resolve(&cli, &cli.command()).unwrap();
        (cli, spec)
    }

    #[test]
    fn cli_r8_record_parses_scene_out_and_headless_defaults() {
        let (_, spec) = record(&["record", "aurora", "--out", "/tmp/a.gif"]);
        assert_eq!(spec.entry.meta.name, "aurora");
        assert_eq!(spec.out, PathBuf::from("/tmp/a.gif"));
        assert_eq!(
            (spec.fps, spec.frames, spec.size, spec.theme, spec.seed),
            (30, 150, Size::new(80, 24), Theme::Astro, 42)
        );
        assert_eq!(spec.format, FormatArg::Gif);
        assert!(!spec.ascii);
    }

    #[test]
    fn cli_r8_record_requires_a_scene_and_an_out_path() {
        assert_eq!(parse(&["record"]).unwrap_err().exit_code(), 2);
        assert_eq!(parse(&["record", "aurora"]).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn cli_r8_record_frames_override_seconds_and_round_up_to_whole_frames() {
        let (_, spec) = record(&[
            "record",
            "starfield",
            "--out",
            "x.gif",
            "--seconds",
            "2",
            "--frames",
            "7",
        ]);
        assert_eq!(spec.frames, 7);
        let (_, spec) = record(&[
            "record",
            "starfield",
            "--out",
            "x.gif",
            "--seconds",
            "2.5",
            "--fps",
            "10",
        ]);
        assert_eq!(spec.frames, 25);
    }

    #[test]
    fn cli_r8_record_fps_must_divide_60() {
        let argv = ["record", "starfield", "--out", "x.gif", "--fps", "7"];
        let cli = parse(&argv).unwrap();
        assert_eq!(
            RecordSpec::resolve(&cli, &cli.command()).unwrap_err(),
            UsageError::RecordFps(7)
        );
        for good in [1u16, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30, 60] {
            let fps = good.to_string();
            let argv = ["record", "starfield", "--out", "x.gif", "--fps", &fps];
            let cli = parse(&argv).unwrap();
            assert_eq!(RecordSpec::resolve(&cli, &cli.command()).unwrap().fps, good);
        }
    }

    #[test]
    fn cli_r8_record_caps_a_recording_at_36000_frames() {
        // `--frames` itself is range-checked by clap (1..=36 000).
        assert_eq!(
            parse(&["record", "starfield", "--out", "x.gif", "--frames", "36001"])
                .unwrap_err()
                .exit_code(),
            2
        );
        let (_, spec) = record(&["record", "starfield", "--out", "x.gif", "--frames", "36000"]);
        assert_eq!(spec.frames, RECORD_MAX_FRAMES);
        let (_, spec) = record(&["record", "starfield", "--out", "x.gif", "--seconds", "1200"]);
        assert_eq!(spec.frames, RECORD_MAX_FRAMES);
        // A `--seconds` request above the cap is a usage error, never a write.
        let argv = ["record", "starfield", "--out", "x.gif", "--seconds", "1201"];
        let cli = parse(&argv).unwrap();
        let err = RecordSpec::resolve(&cli, &cli.command()).unwrap_err();
        assert_eq!(err, UsageError::RecordFrames(36_030));
        assert!(err.to_string().contains("36000"), "{err}");
        // The absurd case is the same usage error (no allocation, no file).
        let argv = ["record", "starfield", "--out", "x.gif", "--seconds", "1e9"];
        let cli = parse(&argv).unwrap();
        assert!(matches!(
            RecordSpec::resolve(&cli, &cli.command()),
            Err(UsageError::RecordFrames(_))
        ));
    }

    #[test]
    fn cli_r8_record_format_is_inferred_from_the_path() {
        use std::path::Path;
        for (path, format) in [
            ("a.gif", FormatArg::Gif),
            ("a.png", FormatArg::Png),
            ("a.cast", FormatArg::Asciicast),
            ("a.json", FormatArg::Asciicast),
            ("frames", FormatArg::Gif),
            ("a.mov", FormatArg::Gif),
        ] {
            assert_eq!(FormatArg::infer(Path::new(path)), format, "{path}");
        }
        let (_, spec) = record(&[
            "record",
            "starfield",
            "--out",
            "a.gif",
            "--format",
            "asciicast",
        ]);
        assert_eq!(spec.format, FormatArg::Asciicast);
    }

    #[test]
    fn cli_r8_record_rejects_unknown_scenes_and_bad_values_with_code_2() {
        let argv = ["record", "nope", "--out", "x.gif"];
        let cli = parse(&argv).unwrap();
        assert!(matches!(
            RecordSpec::resolve(&cli, &cli.command()),
            Err(UsageError::UnknownGenerator(name)) if name == "nope"
        ));
        for argv in [
            ["record", "starfield", "--out", "x.gif", "--size", "0x24"],
            ["record", "starfield", "--out", "x.gif", "--size", "80"],
            ["record", "starfield", "--out", "x.gif", "--seconds", "0"],
            ["record", "starfield", "--out", "x.gif", "--frames", "0"],
            ["record", "starfield", "--out", "x.gif", "--format", "mp4"],
        ] {
            assert_eq!(parse(&argv).unwrap_err().exit_code(), 2, "{argv:?}");
        }
    }

    #[test]
    fn cli_r8_record_honours_global_flags() {
        let (_, spec) = record(&[
            "record",
            "starfield",
            "--out",
            "x.gif",
            "--theme",
            "mono",
            "--ascii",
            "--seed",
            "7",
            "--size",
            "20x8",
        ]);
        assert_eq!(
            (spec.theme, spec.ascii, spec.seed, spec.size),
            (Theme::Mono, true, 7, Size::new(20, 8))
        );
    }
}
