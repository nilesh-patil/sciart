//! Command-line interface (SPEC-005 CLI-R1..R17).

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use sciart_core::{Category, Size};
use sciart_generators::{Entry, find, registry};
use sciart_keepawake::Mode;
use sciart_render::{Background, Theme};
use serde::Deserialize;

#[derive(Debug, Parser)]
#[command(
    name = "sciart",
    version,
    about = "Real-time generative science & astronomy art",
    after_long_help = AFTER_LONG_HELP
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

    /// Colour capability: auto, truecolor, 256 or mono [default: auto]
    #[arg(long, global = true, value_enum)]
    pub color: Option<ColorArg>,

    /// Background behind every cell: opaque black or the terminal default; terminal output only
    /// [default: opaque]
    #[arg(long, global = true, value_enum)]
    pub background: Option<BackgroundArg>,

    /// Use pure-ASCII glyphs
    #[arg(long, global = true)]
    pub ascii: bool,

    /// Start with the status line (HUD) visible
    #[arg(long, global = true)]
    pub hud: bool,

    /// Hide the science title (the HUD then shows the scene id)
    #[arg(long, global = true)]
    pub no_title: bool,

    /// How to keep the machine awake [default: caffeinate on macOS; inhibit on Linux, off over SSH]
    #[arg(long, global = true, value_enum)]
    pub keep_awake: Option<KeepAwakeArg>,

    /// Simulated seconds per wall second (sim and run) [default: 1.0]
    #[arg(long, global = true, value_parser = speed)]
    pub speed: Option<f64>,

    /// Only scenes in this category id; repeatable (sim, list)
    #[arg(long, global = true, value_name = "ID")]
    pub category: Vec<String>,

    /// Only this scene; repeatable (sim)
    #[arg(long, global = true, value_name = "NAME")]
    pub only: Vec<String>,

    /// Skip this scene; repeatable (sim)
    #[arg(long, global = true, value_name = "NAME")]
    pub exclude: Vec<String>,

    /// Config file [default: $XDG_CONFIG_HOME/sciart/config.toml or ~/.config/sciart/config.toml]
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// End the run after this much wall time: 90s, 25m, 1.5h; a bare number is seconds, at
    /// most 24h (sim, run, while)
    #[arg(long = "for", global = true, value_name = "DURATION", value_parser = parse_duration)]
    pub run_for: Option<f64>,
}

/// Shown after `--help` and in the manual page (CLI-R12, CLI-R14, ADR-0031).
const AFTER_LONG_HELP: &str = "\
Scripting:
  sciart --for 25m                  cycle the scenes for 25 minutes, then exit 0
  sciart while -- make -j8          show the art while `make` runs, then print its
                                    output and exit with its status

`while` gives the command no terminal: its stdin is empty, and its stdout and stderr are
held in temporary files and printed, in that order, once the art has closed. Programs that
need a terminal (editors, pagers, password prompts) are not supported. `q` closes the art
early and keeps waiting for the command; Ctrl-C stops it and exits 130; a deadline set with
--for stops it and exits 124; a command that cannot be started exits 127.";

#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum Command {
    /// Cycle through generators (default)
    Sim,
    /// Run a single generator
    Run { name: String },
    /// List generators
    List {
        /// Print a versioned JSON object, {"schema": 1, "scenes": [...]}, sorted by name
        #[arg(long)]
        json: bool,
        /// Print `id<TAB>label<TAB>count` per non-empty category, sorted by label
        #[arg(long)]
        categories: bool,
        /// Tab-separated output even on a terminal
        #[arg(long)]
        plain: bool,
    },
    /// Render a scene to GIF, PNG frames or an asciicast (CLI-R8; needs the `record` feature)
    Record(RecordArgs),
    /// Show the art while a command runs, then print its output and exit with its status
    While(WhileArgs),
    /// Print a shell completion script
    Completions {
        /// Shell to complete for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the manual page (roff)
    Man,
}

/// CLI-R14: `while`'s command line, everything after `--`.
#[derive(Debug, Clone, PartialEq, Args)]
pub struct WhileArgs {
    /// Command to run, then its arguments (after `--`)
    #[arg(last = true, required = true, value_name = "COMMAND")]
    pub command: Vec<OsString>,
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
    Colorblind,
}

impl From<ThemeArg> for Theme {
    fn from(t: ThemeArg) -> Theme {
        match t {
            ThemeArg::Astro => Theme::Astro,
            ThemeArg::Rainbow => Theme::Rainbow,
            ThemeArg::Mono => Theme::Mono,
            ThemeArg::Aurora => Theme::Aurora,
            ThemeArg::Colorblind => Theme::Colorblind,
        }
    }
}

/// CLI-R10/RND-R18: the requested colour capability, before `NO_COLOR` and the config
/// precedence are applied (`sciart::config::ColorMode` is the resolved mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorArg {
    /// Truecolor unless `NO_COLOR` selects mono (RND-R18).
    Auto,
    /// Always 24-bit RGB; the M22 stream, byte for byte.
    Truecolor,
    /// Nearest xterm-256 index (`38;5;n`/`48;5;n`).
    #[value(name = "256")]
    #[serde(rename = "256")]
    Indexed256,
    /// The mono theme (grey), whatever `--theme` says.
    Mono,
}

/// CLI-R11 / CFG-R11 spelling of `sciart_render::Background`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundArg {
    /// Opaque black behind every cell (RND-R20).
    Opaque,
    /// The terminal's own background for `Role::Background`.
    Terminal,
}

impl From<BackgroundArg> for Background {
    fn from(b: BackgroundArg) -> Background {
        match b {
            BackgroundArg::Opaque => Background::Opaque,
            BackgroundArg::Terminal => Background::Terminal,
        }
    }
}

/// CLI-R6 / CFG-R5 spelling of `sciart_keepawake::Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeepAwakeArg {
    Caffeinate,
    Inhibit,
    Mouse,
    Off,
}

impl From<KeepAwakeArg> for Mode {
    fn from(k: KeepAwakeArg) -> Mode {
        match k {
            KeepAwakeArg::Caffeinate => Mode::Caffeinate,
            KeepAwakeArg::Inhibit => Mode::Inhibit,
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

/// CLI-R12: the longest `--for`, 24 h in seconds.
pub const MAX_RUN_SECS: f64 = 24.0 * 3600.0;

/// CLI-R12: `--for` as wall seconds. A positive finite number with an optional unit, `s`,
/// `m` or `h` (`90s`, `25m`, `1.5h`); a bare number is seconds; at most 24 h.
pub fn parse_duration(s: &str) -> Result<f64, String> {
    let (number, unit) = match s.char_indices().last() {
        Some((i, 's')) => (&s[..i], 1.0),
        Some((i, 'm')) => (&s[..i], 60.0),
        Some((i, 'h')) => (&s[..i], 3600.0),
        _ => (s, 1.0),
    };
    let secs = number
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| v * unit);
    match secs {
        Some(secs) if secs <= MAX_RUN_SECS => Ok(secs),
        Some(_) => Err(format!("'{s}' is longer than 24h")),
        None => Err(format!(
            "'{s}' is not a positive duration such as 90s, 25m or 1.5h"
        )),
    }
}

/// CFG-R9 / CLI-R9: a finite speed in `0.1..=4`.
fn speed(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && (0.1..=4.0).contains(&v) => Ok(v),
        _ => Err(format!("'{s}' is not a speed in 0.1..=4")),
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
    UnknownCategory(String),
    AllDisabled,
    /// The active filters match no scene (CLI-R9).
    EmptySelection,
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
            UsageError::UnknownCategory(id) => {
                let ids: Vec<_> = Category::ALL.iter().map(|c| c.id()).collect();
                write!(f, "unknown category '{id}'. Available: {}", ids.join(", "))
            }
            UsageError::AllDisabled => {
                f.write_str("every generator is disabled in the config file; nothing to show")
            }
            UsageError::EmptySelection => f.write_str(
                "the filters select no scenes; check --category, --only, --exclude and the \
                 config's disabled list",
            ),
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

/// Scene filters from the CLI (CLI-R9). `sim` uses all of them, `list` only `--category`,
/// `run` ignores them, and `disabled` (CFG-R4) composes on top.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Filters {
    /// Category ids from `--category`, as a union; empty = all categories.
    pub categories: Vec<Category>,
    /// Canonical names from `--only` (retired aliases resolved, GEN-R14); empty = all.
    pub only: Vec<&'static str>,
    /// Canonical names from `--exclude`.
    pub exclude: Vec<&'static str>,
}

impl Filters {
    /// Parse the filter flags according to what `command` honours (CLI-R9).
    pub fn from_cli(command: &Command, cli: &Cli) -> Result<Self, UsageError> {
        if matches!(command, Command::Run { .. }) {
            return Ok(Self::default());
        }
        let mut filters = Self::default();
        for id in &cli.category {
            let category = Category::ALL
                .into_iter()
                .find(|c| c.id() == id)
                .ok_or_else(|| UsageError::UnknownCategory(id.clone()))?;
            if !filters.categories.contains(&category) {
                filters.categories.push(category);
            }
        }
        // `while` shows the `sim` cycle (CLI-R14).
        if matches!(command, Command::Sim | Command::While(_)) {
            filters.only = canonical_names(&cli.only)?;
            filters.exclude = canonical_names(&cli.exclude)?;
        }
        Ok(filters)
    }

    fn matches(&self, entry: &Entry) -> bool {
        (self.categories.is_empty() || self.categories.contains(&entry.meta.category))
            && (self.only.is_empty() || self.only.contains(&entry.meta.name))
            && !self.exclude.contains(&entry.meta.name)
    }
}

/// Resolve `--only`/`--exclude` names, accepting retired aliases like `run` and `disabled`
/// do (GEN-R14, CFG-R7), and rejecting unknown ones (CLI-R9).
fn canonical_names(names: &[String]) -> Result<Vec<&'static str>, UsageError> {
    names
        .iter()
        .map(|name| {
            find(name)
                .map(|e| e.meta.name)
                .ok_or_else(|| UsageError::UnknownGenerator(name.clone()))
        })
        .collect()
}

/// Generators a command will use. `disabled` filters `sim` only, retired names included
/// (CLI-R2, CFG-R4, CFG-R7); `filters` applies per CLI-R9.
pub fn select(
    command: &Command,
    disabled: &[String],
    filters: &Filters,
) -> Result<Vec<&'static Entry>, UsageError> {
    match command {
        Command::Run { name } => find(name)
            .map(|e| vec![e])
            .ok_or_else(|| UsageError::UnknownGenerator(name.clone())),
        Command::Record(args) => find(&args.scene)
            .map(|e| vec![e])
            .ok_or_else(|| UsageError::UnknownGenerator(args.scene.clone())),
        Command::Sim | Command::While(_) => {
            let disabled: Vec<&str> = disabled
                .iter()
                .filter_map(|d| find(d))
                .map(|e| e.meta.name)
                .collect();
            let healthy: Vec<_> = registry()
                .iter()
                .filter(|e| !disabled.contains(&e.meta.name))
                .collect();
            if healthy.is_empty() {
                return Err(UsageError::AllDisabled);
            }
            let entries: Vec<_> = healthy.into_iter().filter(|e| filters.matches(e)).collect();
            if entries.is_empty() {
                Err(UsageError::EmptySelection)
            } else {
                Ok(entries)
            }
        }
        // They show no scene (CLI-R17).
        Command::Completions { .. } | Command::Man => Ok(Vec::new()),
        Command::List { .. } => {
            // `disabled` only filters `sim` (CFG-R4); list honours `--category` only.
            let entries: Vec<_> = registry().iter().filter(|e| filters.matches(e)).collect();
            if entries.is_empty() {
                Err(UsageError::EmptySelection)
            } else {
                Ok(entries)
            }
        }
    }
}

/// How `list` writes its output (CLI-R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    /// Header row and aligned columns, on a terminal.
    Aligned,
    /// The TSV of CLI-R4, when piped or with `--plain`.
    Tsv,
    /// The versioned JSON object of CLI-R16, with `--json`.
    Json,
    /// One line per category, with `--categories`.
    Categories,
}

/// Pick the format: `--json` wins, then `--categories`, then TSV when piped or `--plain`
/// (CLI-R4, CLI-R10).
pub fn list_format(json: bool, categories: bool, plain: bool, tty: bool) -> ListFormat {
    if json {
        ListFormat::Json
    } else if categories {
        ListFormat::Categories
    } else if plain || !tty {
        ListFormat::Tsv
    } else {
        ListFormat::Aligned
    }
}

/// Render `entries` in `format`; every format lists the same, name-sorted scene set.
pub fn list_output(entries: &[&'static Entry], format: ListFormat) -> String {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|e| e.meta.name);
    match format {
        ListFormat::Aligned => list_aligned(&entries),
        ListFormat::Tsv => list_tsv(&entries),
        ListFormat::Json => list_json(&entries),
        ListFormat::Categories => list_categories(&entries),
    }
}

/// `name<TAB>category id<TAB>description`, sorted by name (CLI-R4).
fn list_tsv(entries: &[&Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(e.meta.name);
        out.push('\t');
        out.push_str(e.meta.category.id());
        out.push('\t');
        out.push_str(e.meta.description);
        out.push('\n');
    }
    out
}

/// Header row and aligned `NAME CATEGORY DESCRIPTION` columns (CLI-R10).
fn list_aligned(entries: &[&Entry]) -> String {
    let width = |s: &str| s.chars().count();
    let name_w = entries
        .iter()
        .map(|e| width(e.meta.name))
        .chain([width("NAME")])
        .max()
        .unwrap_or(4);
    let cat_w = entries
        .iter()
        .map(|e| width(e.meta.category.id()))
        .chain([width("CATEGORY")])
        .max()
        .unwrap_or(8);
    let mut out = format!(
        "{:<name_w$}  {:<cat_w$}  {}\n",
        "NAME", "CATEGORY", "DESCRIPTION"
    );
    for e in entries {
        out.push_str(&format!(
            "{:<name_w$}  {:<cat_w$}  {}\n",
            e.meta.name,
            e.meta.category.id(),
            e.meta.description
        ));
    }
    out
}

/// CLI-R16: the `schema` of `list --json`. Fields are only ever added; removing one or
/// changing its meaning raises it.
pub const LIST_SCHEMA: u32 = 1;

/// The versioned catalogue of CLI-R16, `{"schema":1,"scenes":[...]}` on one line,
/// hand-rolled so the binary needs no JSON dependency.
fn list_json(entries: &[&Entry]) -> String {
    let mut out = format!("{{\"schema\":{LIST_SCHEMA},\"scenes\":[");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let meta = e.meta;
        for (key, value) in [
            ("{\"name\":", meta.name),
            (",\"category\":", meta.category.id()),
            (",\"category_label\":", meta.category.label()),
            (",\"title\":", meta.title),
            (",\"description\":", meta.description),
        ] {
            out.push_str(key);
            out.push_str(&json_string(value));
        }
        out.push_str(",\"captions\":");
        json_array(&mut out, meta.captions.iter().copied());
        // Retired names that still resolve to this scene (GEN-R14).
        out.push_str(",\"aliases\":");
        let aliases = sciart_generators::ALIASES
            .iter()
            .filter(|(_, name)| *name == meta.name)
            .map(|(old, _)| *old);
        json_array(&mut out, aliases);
        out.push('}');
    }
    out.push_str("]}\n");
    out
}

/// Append `items` as a JSON array of strings.
fn json_array<'a>(out: &mut String, items: impl Iterator<Item = &'a str>) {
    out.push('[');
    for (i, item) in items.enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&json_string(item));
    }
    out.push(']');
}

/// A JSON string literal with the minimal escaping a JSON parser requires.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `id<TAB>label<TAB>count` per non-empty category, sorted by label (CLI-R10).
fn list_categories(entries: &[&Entry]) -> String {
    let mut rows: Vec<(Category, usize)> = Category::ALL
        .iter()
        .map(|c| {
            let n = entries.iter().filter(|e| e.meta.category == *c).count();
            (*c, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    rows.sort_by_key(|(c, _)| c.label());
    let mut out = String::new();
    for (category, n) in rows {
        out.push_str(&format!("{}\t{}\t{}\n", category.id(), category.label(), n));
    }
    out
}

/// CLI-R17: the completion script for `shell`, generated from the parser's own definition.
/// Built in memory, so a closed pipe is an error to report, not a panic in the generator.
pub fn completions(shell: clap_complete::Shell) -> Vec<u8> {
    let mut command = <Cli as clap::CommandFactory>::command();
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut command, "sciart", &mut out);
    out
}

/// CLI-R17: the roff manual page, generated from the parser's own definition.
pub fn man_page() -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    clap_mangen::Man::new(<Cli as clap::CommandFactory>::command()).render(&mut out)?;
    Ok(out)
}

#[cfg(test)]
pub(crate) fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("sciart").chain(args.iter().copied()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_command() -> Command {
        Command::List {
            json: false,
            categories: false,
            plain: false,
        }
    }

    /// Select the scenes for `args`, returning their canonical names in registry order.
    fn selected(args: &[&str], disabled: &[String]) -> Result<Vec<&'static str>, UsageError> {
        let cli = parse(args).expect("valid CLI");
        let command = cli.command();
        let filters = Filters::from_cli(&command, &cli)?;
        select(&command, disabled, &filters)
            .map(|entries| entries.iter().map(|e| e.meta.name).collect())
    }

    #[test]
    fn cli_r1_no_subcommand_is_sim() {
        assert_eq!(parse(&[]).unwrap().command(), Command::Sim);
        assert!(matches!(
            parse(&["list"]).unwrap().command(),
            Command::List { .. }
        ));
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
            &Filters::default(),
        )
        .unwrap_err();
        assert_eq!(err, UsageError::UnknownGenerator("nope".into()));
        assert!(err.to_string().contains("starfield"));
        let run = Command::Run {
            name: "starfield".into(),
        };
        assert_eq!(select(&run, &[], &Filters::default()).unwrap().len(), 1);
        assert_eq!(
            select(&Command::Sim, &[], &Filters::default())
                .unwrap()
                .len(),
            registry().len()
        );
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
        let entries = select(&list_command(), &[], &Filters::default()).unwrap();
        let text = list_output(&entries, ListFormat::Tsv);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), registry().len());
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted);
        assert!(lines.iter().all(|l| l.split('\t').count() == 3));
        assert!(
            lines.contains(
                &format!(
                    "starfield\tdeep-cosmos\t{}",
                    sciart_generators::starfield::META.description
                )
                .as_str()
            )
        );
        for (old, _) in sciart_generators::ALIASES {
            assert!(
                !lines.iter().any(|l| l.starts_with(&format!("{old}\t"))),
                "GEN-R14: retired name {old} listed"
            );
        }
    }

    #[test]
    fn cli_r9_category_only_and_exclude_filter_sim() {
        let names = selected(&["--category", "deep-cosmos"], &[]).unwrap();
        assert!(names.contains(&"starfield"));
        assert!(
            names
                .iter()
                .all(|n| find(n).unwrap().meta.category == Category::DeepCosmos)
        );

        // Repeatable categories are a union.
        let names = selected(&["--category", "deep-cosmos", "--category", "voyages"], &[]).unwrap();
        assert!(names.contains(&"starfield") && names.contains(&"moon_landing"));
        assert!(
            names
                .iter()
                .any(|n| { find(n).unwrap().meta.category == Category::Voyages })
        );

        // `--only` restricts, `--exclude` removes; both keep registry order.
        assert_eq!(
            selected(&["--only", "starfield", "--only", "moon_landing"], &[]).unwrap(),
            ["moon_landing", "starfield"]
        );
        let names = selected(&["--exclude", "starfield"], &[]).unwrap();
        assert_eq!(names.len(), registry().len() - 1);
        assert!(!names.contains(&"starfield"));

        // Retired aliases resolve as they do for `run` and `disabled` (GEN-R14, CFG-R7).
        assert_eq!(
            selected(&["--only", "chemical_reaction"], &[]).unwrap(),
            ["gray_scott"]
        );
    }

    #[test]
    fn cli_r9_filters_compose_with_disabled() {
        let disabled = vec!["starfield".to_string()];
        let names = selected(&["--category", "deep-cosmos"], &disabled).unwrap();
        assert!(!names.is_empty());
        assert!(!names.contains(&"starfield"));
        assert!(
            names
                .iter()
                .all(|n| find(n).unwrap().meta.category == Category::DeepCosmos)
        );
    }

    #[test]
    fn cli_r9_unknown_category_name_or_empty_selection_exits_2() {
        let err = selected(&["--category", "nope"], &[]).unwrap_err();
        assert_eq!(err, UsageError::UnknownCategory("nope".into()));
        assert!(err.to_string().contains("deep-cosmos"), "{err}");
        for flag in ["--only", "--exclude"] {
            assert_eq!(
                selected(&[flag, "nope"], &[]).unwrap_err(),
                UsageError::UnknownGenerator("nope".into()),
                "{flag}"
            );
        }
        assert_eq!(
            selected(&["--only", "starfield", "--exclude", "starfield"], &[]).unwrap_err(),
            UsageError::EmptySelection
        );
    }

    #[test]
    fn cli_r9_run_ignores_filters() {
        let cli = parse(&[
            "run",
            "starfield",
            "--category",
            "nope",
            "--only",
            "nope",
            "--exclude",
            "starfield",
        ])
        .unwrap();
        let filters = Filters::from_cli(&cli.command(), &cli).unwrap();
        assert_eq!(filters, Filters::default(), "run ignores every filter");
        assert_eq!(
            select(&cli.command(), &[], &filters).unwrap()[0].meta.name,
            "starfield"
        );
    }

    #[test]
    fn cli_r10_format_choice_is_json_then_categories_then_tty() {
        assert_eq!(list_format(false, false, false, true), ListFormat::Aligned);
        assert_eq!(list_format(false, false, false, false), ListFormat::Tsv);
        assert_eq!(
            list_format(false, false, true, true),
            ListFormat::Tsv,
            "--plain overrides a terminal"
        );
        assert_eq!(list_format(true, false, false, true), ListFormat::Json);
        assert_eq!(
            list_format(true, true, true, true),
            ListFormat::Json,
            "--json wins"
        );
        assert_eq!(
            list_format(false, true, false, true),
            ListFormat::Categories
        );
    }

    #[test]
    fn cli_r9_list_honours_category_filter() {
        let cli = parse(&["list", "--category", "deep-cosmos"]).unwrap();
        let filters = Filters::from_cli(&cli.command(), &cli).unwrap();
        let entries = select(&cli.command(), &[], &filters).unwrap();
        assert!(!entries.is_empty());
        assert!(
            entries
                .iter()
                .all(|e| e.meta.category == Category::DeepCosmos)
        );
        let text = list_output(&entries, ListFormat::Tsv);
        assert_eq!(text.lines().count(), entries.len());
        for line in text.lines() {
            assert_eq!(line.split('\t').nth(1), Some("deep-cosmos"));
        }
    }

    #[test]
    fn cli_r10_aligned_output_has_a_header_and_columns() {
        let entries = select(&list_command(), &[], &Filters::default()).unwrap();
        let text = list_output(&entries, ListFormat::Aligned);
        let mut lines = text.lines();
        let header = lines.next().unwrap();
        assert!(header.starts_with("NAME"));
        let cat_col = header.find("CATEGORY").unwrap();
        let desc_col = header.find("DESCRIPTION").unwrap();
        assert!(cat_col > 4 && desc_col > cat_col);
        for line in lines {
            let rest = &line[cat_col..];
            assert!(
                Category::ALL.iter().any(|c| rest.starts_with(c.id())),
                "{line:?}"
            );
            assert!(
                line.len() > desc_col && !line[desc_col..].trim().is_empty(),
                "{line:?}"
            );
        }
        assert_eq!(text.lines().count(), entries.len() + 1);
    }

    #[test]
    fn cli_r16_json_is_a_versioned_object_sorted_by_name() {
        let entries = select(&list_command(), &[], &Filters::default()).unwrap();
        let text = list_output(&entries, ListFormat::Json);
        assert!(
            text.starts_with("{\"schema\":1,\"scenes\":[{\"name\":\""),
            "{text:?}"
        );
        assert!(text.ends_with("}]}\n"));
        assert_eq!(text.lines().count(), 1, "one line");
        assert_eq!(text.matches("\"name\":").count(), entries.len());
        let mut sorted = entries.clone();
        sorted.sort_by_key(|e| e.meta.name);
        let mut pos = 0;
        for e in &sorted {
            let needle = format!(
                "{{\"name\":\"{}\",\"category\":\"{}\",\"category_label\":\"{}\",\"title\":",
                e.meta.name,
                e.meta.category.id(),
                e.meta.category.label()
            );
            let at = text[pos..]
                .find(&needle)
                .unwrap_or_else(|| panic!("missing {needle} after byte {pos}"));
            pos += at + needle.len();
        }
        assert_eq!(
            json_string("a\"b\\c\nd\te\u{1}"),
            "\"a\\\"b\\\\c\\nd\\te\\u0001\""
        );
    }

    /// CLI-R16 through a real JSON parser: every field, with the scene's own values.
    #[test]
    fn cli_r16_json_fields_parse_and_match_the_registry() {
        let entries = select(&list_command(), &[], &Filters::default()).unwrap();
        let text = list_output(&entries, ListFormat::Json);
        let doc: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(doc["schema"], 1);
        let scenes = doc["scenes"].as_array().expect("scenes array");
        assert_eq!(scenes.len(), registry().len());
        let strings = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .expect("an array")
                .iter()
                .map(|s| s.as_str().expect("a string").to_owned())
                .collect()
        };
        for scene in scenes {
            let name = scene["name"].as_str().expect("name");
            let meta = find(name).expect("a registered name").meta;
            assert_eq!(meta.name, name, "listed under its own name");
            assert_eq!(scene["category"], meta.category.id(), "{name}");
            assert_eq!(scene["category_label"], meta.category.label(), "{name}");
            assert_eq!(scene["title"], meta.title, "{name}");
            assert_eq!(scene["description"], meta.description, "{name}");
            assert_eq!(strings(&scene["captions"]), meta.captions, "{name}");
            let aliases: Vec<&str> = sciart_generators::ALIASES
                .iter()
                .filter(|(_, to)| *to == name)
                .map(|(old, _)| *old)
                .collect();
            assert_eq!(strings(&scene["aliases"]), aliases, "{name}");
        }
        let gray_scott = scenes.iter().find(|s| s["name"] == "gray_scott").unwrap();
        assert_eq!(strings(&gray_scott["aliases"]), ["chemical_reaction"]);
        assert!(
            scenes.iter().any(|s| !strings(&s["captions"]).is_empty()),
            "some scene has acts"
        );
        // Filters narrow the list, not the shape.
        let cli = parse(&["list", "--json", "--category", "voyages"]).unwrap();
        let filters = Filters::from_cli(&cli.command(), &cli).unwrap();
        let entries = select(&cli.command(), &[], &filters).unwrap();
        let text = list_output(&entries, ListFormat::Json);
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["scenes"].as_array().unwrap().len(), entries.len());
    }

    // ----- M25: scriptable modes (CLI-R12, CLI-R14, CLI-R17) -----

    #[test]
    fn cli_r12_durations_take_s_m_h_or_bare_seconds() {
        for (text, secs) in [
            ("90s", 90.0),
            ("90", 90.0),
            ("25m", 1500.0),
            ("1.5h", 5400.0),
            ("0.5", 0.5),
            ("24h", MAX_RUN_SECS),
            ("1440m", MAX_RUN_SECS),
            ("86400", MAX_RUN_SECS),
        ] {
            assert_eq!(parse_duration(text), Ok(secs), "{text}");
        }
    }

    #[test]
    fn cli_r12_bad_or_too_long_durations_are_rejected() {
        for bad in [
            "", "s", "m", "h", "0", "0s", "-5", "-1m", "nan", "inf", "infh", "1e400", "5ms", "5 m",
            " 5", "5d", "1:30", "five", "24.001h", "86401", "1e308h",
        ] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
        assert!(parse_duration("25h").unwrap_err().contains("24h"));
    }

    #[test]
    fn cli_r12_for_is_a_global_flag_and_a_bad_value_exits_2() {
        assert_eq!(parse(&[]).unwrap().run_for, None);
        assert_eq!(parse(&["--for", "25m"]).unwrap().run_for, Some(1500.0));
        assert_eq!(
            parse(&["run", "starfield", "--for", "90s"])
                .unwrap()
                .run_for,
            Some(90.0)
        );
        assert_eq!(
            parse(&["--for", "2", "while", "--", "true"])
                .unwrap()
                .run_for,
            Some(2.0)
        );
        for bad in ["0", "25h", "soon"] {
            assert_eq!(parse(&["--for", bad]).unwrap_err().exit_code(), 2, "{bad}");
        }
    }

    #[test]
    fn cli_r14_while_takes_the_command_after_a_double_dash() {
        let cli = parse(&[
            "while",
            "--only",
            "aurora",
            "--",
            "make",
            "-j8",
            "--keep-going",
        ])
        .unwrap();
        let Command::While(args) = cli.command() else {
            panic!("not a while command");
        };
        assert_eq!(args.command, ["make", "-j8", "--keep-going"]);
        // The command's own flags are never read as sciart's.
        let cli = parse(&["while", "--", "sh", "-c", "exit 3", "--fps", "7"]).unwrap();
        assert_eq!(cli.fps, None);
        // A missing command line is a usage error, with or without `--`.
        for argv in [&["while"][..], &["while", "--"][..], &["while", "make"][..]] {
            assert_eq!(parse(argv).unwrap_err().exit_code(), 2, "{argv:?}");
        }
    }

    #[test]
    fn cli_r14_while_selects_like_sim() {
        let names = selected(&["while", "--only", "starfield", "--", "true"], &[]).unwrap();
        assert_eq!(names, ["starfield"]);
        let all = selected(&["while", "--", "true"], &[]).unwrap();
        assert_eq!(all.len(), registry().len());
        let disabled = vec!["starfield".to_string()];
        let names = selected(&["while", "--", "true"], &disabled).unwrap();
        assert!(!names.contains(&"starfield"), "config `disabled` applies");
    }

    #[test]
    fn cli_r17_completions_cover_every_shell_and_subcommand() {
        for shell in clap_complete::Shell::value_variants() {
            let script = String::from_utf8(completions(*shell)).expect("UTF-8");
            for word in [
                "sciart",
                "while",
                "record",
                "completions",
                "man",
                "keep-awake",
            ] {
                assert!(script.contains(word), "{shell}: {word}");
            }
        }
        for name in ["bash", "zsh", "fish", "elvish", "powershell"] {
            assert!(parse(&["completions", name]).is_ok(), "completions {name}");
        }
        assert_eq!(parse(&["completions", "tcsh"]).unwrap_err().exit_code(), 2);
        assert_eq!(parse(&["completions"]).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn cli_r17_man_page_is_roff_from_the_same_definition() {
        let page = String::from_utf8(man_page().expect("render")).expect("UTF-8");
        assert!(page.contains(".TH sciart"), "{page}");
        for word in [
            "while",
            "completions",
            "record",
            "list",
            "DURATION",
            "no terminal",
        ] {
            assert!(page.contains(word), "man page mentions {word}");
        }
        assert!(matches!(parse(&["man"]).unwrap().command(), Command::Man));
    }

    #[test]
    fn cli_r10_categories_are_sorted_by_label_and_cover_every_scene() {
        let entries = select(&list_command(), &[], &Filters::default()).unwrap();
        let text = list_output(&entries, ListFormat::Categories);
        let rows: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        assert!(rows.iter().all(|r| r.len() == 3), "{rows:?}");
        let labels: Vec<&str> = rows.iter().map(|r| r[1]).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        assert_eq!(labels, sorted);
        let total: usize = rows
            .iter()
            .map(|r| r[2].parse::<usize>().expect("count"))
            .sum();
        assert_eq!(total, registry().len());
        for r in &rows {
            let c = Category::ALL.iter().find(|c| c.id() == r[0]).unwrap();
            assert_eq!(c.label(), r[1]);
        }
        let non_empty = Category::ALL
            .iter()
            .filter(|c| registry().iter().any(|e| e.meta.category == **c))
            .count();
        assert_eq!(rows.len(), non_empty);
    }

    #[test]
    fn cli_r10_every_format_covers_the_same_scene_set() {
        let entries = select(&list_command(), &[], &Filters::default()).unwrap();
        let mut expected: Vec<&str> = entries.iter().map(|e| e.meta.name).collect();
        expected.sort_unstable();
        let tsv = list_output(&entries, ListFormat::Tsv);
        let tsv_names: Vec<&str> = tsv.lines().map(|l| l.split('\t').next().unwrap()).collect();
        assert_eq!(tsv_names, expected);
        let aligned = list_output(&entries, ListFormat::Aligned);
        let aligned_names: Vec<&str> = aligned
            .lines()
            .skip(1)
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(aligned_names, expected);
        let json = list_output(&entries, ListFormat::Json);
        assert_eq!(json.matches("\"name\":").count(), expected.len());
    }

    #[test]
    fn cli_r2_gen_r14_run_accepts_a_retired_name() {
        let run = Command::Run {
            name: "chemical_reaction".into(),
        };
        let picked = select(&run, &[], &Filters::default()).unwrap();
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
    fn cli_r6_ka_r7_keep_awake_flag() {
        for (arg, mode) in [
            ("caffeinate", Mode::Caffeinate),
            ("inhibit", Mode::Inhibit),
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
    fn cli_r9_speed_must_be_finite_and_in_range() {
        for good in ["0.1", "1", "2.5", "4"] {
            assert_eq!(
                parse(&["--speed", good]).unwrap().speed,
                Some(good.parse().unwrap()),
                "--speed {good}"
            );
        }
        for bad in ["0", "0.09", "4.1", "-1", "nan", "inf", "fast"] {
            let err = parse(&["--speed", bad]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "--speed {bad}");
        }
        assert_eq!(
            parse(&[]).unwrap().speed,
            None,
            "default applied in Settings"
        );
    }

    #[test]
    fn cfg_r10_color_flag_values_and_bad_value_exits_2() {
        for (arg, want) in [
            ("auto", ColorArg::Auto),
            ("truecolor", ColorArg::Truecolor),
            ("256", ColorArg::Indexed256),
            ("mono", ColorArg::Mono),
        ] {
            assert_eq!(
                parse(&["--color", arg]).unwrap().color,
                Some(want),
                "--color {arg}"
            );
        }
        for bad in ["neon", "16", "true-color", "indexed"] {
            let err = parse(&["--color", bad]).unwrap_err();
            assert_eq!(err.exit_code(), 2, "--color {bad}");
        }
        assert_eq!(
            parse(&[]).unwrap().color,
            None,
            "default (auto) applied in Settings"
        );
        // The global flag also works after a subcommand.
        assert_eq!(
            parse(&["run", "starfield", "--color", "256"])
                .unwrap()
                .color,
            Some(ColorArg::Indexed256)
        );
    }

    #[test]
    fn cfg_r4_disabled_filters_sim_but_not_run() {
        let all: Vec<String> = registry().iter().map(|e| e.meta.name.to_string()).collect();
        assert!(matches!(
            select(&Command::Sim, &all, &Filters::default()),
            Err(UsageError::AllDisabled)
        ));
        let run = Command::Run {
            name: "starfield".into(),
        };
        assert_eq!(select(&run, &all, &Filters::default()).unwrap().len(), 1);
        let unrelated = vec!["not_a_generator".to_string()];
        assert_eq!(
            select(&Command::Sim, &unrelated, &Filters::default())
                .unwrap()
                .len(),
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
    fn cli_r11_background_flag_is_global_and_defaults_to_none() {
        assert_eq!(
            parse(&[]).unwrap().background,
            None,
            "the opaque default is applied in Settings"
        );
        for (arg, want) in [
            ("opaque", BackgroundArg::Opaque),
            ("terminal", BackgroundArg::Terminal),
        ] {
            assert_eq!(
                parse(&["--background", arg]).unwrap().background,
                Some(want),
                "{arg}"
            );
            // Global: it is accepted after a subcommand too.
            assert_eq!(
                parse(&["run", "starfield", "--background", arg])
                    .unwrap()
                    .background,
                Some(want),
                "run {arg}"
            );
            // `record` accepts it as a global flag (the recording path ignores it).
            assert_eq!(
                parse(&["record", "starfield", "--out", "x.gif", "--background", arg])
                    .unwrap()
                    .background,
                Some(want),
                "record {arg}"
            );
        }
    }

    #[test]
    fn cli_r11_invalid_background_exits_2() {
        for bad in ["neon", "windows", "opaqueish", ""] {
            assert_eq!(
                parse(&["--background", bad]).unwrap_err().exit_code(),
                2,
                "--background {bad:?}"
            );
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
