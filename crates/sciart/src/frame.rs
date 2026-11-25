//! `sciart frame` (CLI-R13): one frame of a scene on stdout as ANSI text, for a shell
//! greeting, a MOTD or a CI log. No terminal session, keep-awake or config file.
//!
//! The rows come from the one ANSI writer (`sciart_render::write_ansi`) that the library's
//! `Stage::to_ansi` and the asciicast recorder use, and time moves by the library's rule,
//! so in truecolor the output is byte for byte the library's for the same frame (LIB-R5).

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use clap::Args;
use sciart_core::prelude::{GlyphMode, SIM_DT, Size, Stage};
use sciart_generators::Entry;
use sciart_render::{AnsiColor, AnsiFraming, Theme, write_ansi};

use crate::cli::{Cli, ColorArg, UsageError, size_arg};
use crate::config::Env;

/// CLI-R13 defaults.
pub const FRAME_DEFAULT_T: f64 = 5.0;
pub const FRAME_DEFAULT_SEED: u64 = 42;
pub const FRAME_DEFAULT_SIZE: Size = Size::new(80, 24);
/// The latest `--t` accepted, in simulated seconds.
pub const FRAME_MAX_T: f64 = 3600.0;

/// CLI-R13: the `frame` subcommand's own flags; `--seed`, `--theme`, `--color`, `--ascii`
/// and `--no-title` are the global flags.
#[derive(Debug, Clone, PartialEq, Args)]
pub struct FrameArgs {
    /// Scene to draw (registered name or retired alias)
    pub scene: String,
    /// Simulated seconds before the frame is drawn, 0..=3600 [default: 5]
    #[arg(long = "t", value_name = "SECONDS", value_parser = frame_time)]
    pub t: Option<f64>,
    /// Cell size WxH [default: the terminal's size, else 80x24]
    #[arg(long, value_parser = size_arg)]
    pub size: Option<Size>,
}

/// CLI-R13: a finite number of simulated seconds in `0..=3600`.
fn frame_time(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && (0.0..=FRAME_MAX_T).contains(&v) => Ok(v),
        _ => Err(format!("'{s}' is not a number of seconds in 0..=3600")),
    }
}

/// A validated `frame` invocation.
#[derive(Debug)]
pub struct FrameSpec {
    pub entry: &'static Entry,
    pub steps: u64,
    pub size: Size,
    pub seed: u64,
    pub theme: Theme,
    pub color: AnsiColor,
    pub ascii: bool,
    pub title: bool,
    /// The `NO_COLOR` downgrade to mono, reported once on stderr (RND-R18).
    pub warn_no_color: bool,
}

impl FrameSpec {
    /// Apply the CLI-R13 defaults. `terminal` is stdout's size when stdout is a terminal.
    pub fn resolve(
        cli: &Cli,
        args: &FrameArgs,
        env: Env,
        terminal: Option<Size>,
    ) -> Result<Self, UsageError> {
        let entry = sciart_generators::find(&args.scene)
            .ok_or_else(|| UsageError::UnknownGenerator(args.scene.clone()))?;
        // RND-R18: `auto` is truecolor unless NO_COLOR asks for mono; mono paints in greys.
        let (color, mono, warn_no_color) = match cli.color.unwrap_or(ColorArg::Auto) {
            ColorArg::Auto if env.no_color => (AnsiColor::Truecolor, true, true),
            ColorArg::Auto | ColorArg::Truecolor => (AnsiColor::Truecolor, false, false),
            ColorArg::Indexed256 => (AnsiColor::Indexed, false, false),
            ColorArg::Mono => (AnsiColor::Truecolor, true, false),
        };
        let theme = if mono {
            Theme::Mono
        } else {
            cli.theme.map(Theme::from).unwrap_or(Theme::Astro)
        };
        let t = args.t.unwrap_or(FRAME_DEFAULT_T);
        Ok(Self {
            entry,
            steps: steps_for(t),
            size: args.size.or(terminal).unwrap_or(FRAME_DEFAULT_SIZE),
            seed: cli.seed.unwrap_or(FRAME_DEFAULT_SEED),
            theme,
            color,
            ascii: cli.ascii,
            title: !cli.no_title,
            warn_no_color,
        })
    }

    /// The frame as ANSI text, one reset-terminated line per row.
    pub fn render(&self) -> String {
        let mode = if self.ascii {
            GlyphMode::Ascii
        } else {
            GlyphMode::Unicode
        };
        let mut stage = Stage::new((self.entry.build)(), self.seed, self.size, mode);
        stage.set_title_visible(self.title);
        for _ in 0..self.steps {
            stage.step(SIM_DT);
        }
        let canvas = stage.render();
        let mut out = String::with_capacity(canvas.cells().len() * 4);
        write_ansi(canvas, self.theme, self.color, AnsiFraming::Lines, &mut out);
        out
    }
}

/// Whole 1/60 s steps for `t` simulated seconds: the library's `Stage::advance` rule
/// (LIB-R5), so `frame --t T` is `advance(T)`.
fn steps_for(t: f64) -> u64 {
    (t * 60.0 + 1e-6).floor() as u64
}

/// Run `sciart frame`: usage errors exit 2, a failed write 1, a closed pipe 0.
pub fn main(cli: &Cli, args: &FrameArgs) -> ExitCode {
    let stdout = std::io::stdout();
    let terminal = stdout
        .is_terminal()
        .then(crossterm::terminal::size)
        .and_then(Result::ok)
        .map(|(w, h)| Size::new(w.max(1), h.max(1)));
    let spec = match FrameSpec::resolve(cli, args, Env::from_process(), terminal) {
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("sciart: {e}");
            return ExitCode::from(2);
        }
    };
    if spec.warn_no_color {
        eprintln!("sciart: warning: NO_COLOR is set; using the mono theme");
    }
    let text = spec.render();
    let mut out = stdout.lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sciart: frame: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Command, parse};

    fn spec(args: &[&str], env: Env, terminal: Option<Size>) -> Result<FrameSpec, UsageError> {
        let cli = parse(args).expect("valid CLI");
        let Command::Frame(frame) = cli.command() else {
            panic!("not a frame command: {args:?}");
        };
        FrameSpec::resolve(&cli, &frame, env, terminal)
    }

    #[test]
    fn cli_r13_defaults_are_five_seconds_seed_42_and_80x24_off_a_terminal() {
        let s = spec(&["frame", "aurora"], Env::default(), None).unwrap();
        assert_eq!(s.entry.meta.name, "aurora");
        assert_eq!(s.steps, 300);
        assert_eq!(s.seed, 42);
        assert_eq!(s.size, Size::new(80, 24));
        assert_eq!((s.theme, s.color), (Theme::Astro, AnsiColor::Truecolor));
        assert!(s.title && !s.ascii && !s.warn_no_color);
    }

    #[test]
    fn cli_r13_size_follows_the_terminal_unless_given() {
        let tty = Some(Size::new(132, 43));
        assert_eq!(
            spec(&["frame", "aurora"], Env::default(), tty)
                .unwrap()
                .size,
            Size::new(132, 43)
        );
        let given = spec(&["frame", "aurora", "--size", "40x10"], Env::default(), tty).unwrap();
        assert_eq!(given.size, Size::new(40, 10));
    }

    #[test]
    fn cli_r13_global_flags_apply() {
        let s = spec(
            &[
                "frame",
                "chemical_reaction",
                "--t",
                "1.5",
                "--seed",
                "7",
                "--theme",
                "aurora",
                "--ascii",
                "--no-title",
            ],
            Env::default(),
            None,
        )
        .unwrap();
        assert_eq!(s.entry.meta.name, "gray_scott", "retired names resolve");
        assert_eq!((s.steps, s.seed, s.theme), (90, 7, Theme::Aurora));
        assert!(s.ascii && !s.title);
    }

    #[test]
    fn cli_r13_colour_modes_follow_rnd_r18() {
        let indexed = spec(&["frame", "aurora", "--color", "256"], Env::default(), None).unwrap();
        assert_eq!(indexed.color, AnsiColor::Indexed);
        let mono = spec(
            &["frame", "aurora", "--color", "mono", "--theme", "aurora"],
            Env::default(),
            None,
        )
        .unwrap();
        assert_eq!(
            (mono.theme, mono.color),
            (Theme::Mono, AnsiColor::Truecolor)
        );
        let no_color = Env { no_color: true };
        let auto = spec(&["frame", "aurora"], no_color, None).unwrap();
        assert_eq!(auto.theme, Theme::Mono);
        assert!(auto.warn_no_color);
        let explicit = spec(&["frame", "aurora", "--color", "truecolor"], no_color, None).unwrap();
        assert_eq!(explicit.theme, Theme::Astro);
        assert!(!explicit.warn_no_color);
    }

    #[test]
    fn cli_r13_bad_scene_and_time_are_usage_errors() {
        assert!(matches!(
            spec(&["frame", "no_such_scene"], Env::default(), None),
            Err(UsageError::UnknownGenerator(_))
        ));
        for t in ["-1", "3600.5", "inf", "NaN", "soon"] {
            let err = parse(&["frame", "aurora", "--t", t]).expect_err(t);
            assert_eq!(err.exit_code(), 2, "{t}");
        }
        assert_eq!(
            spec(&["frame", "aurora", "--t", "0"], Env::default(), None)
                .unwrap()
                .steps,
            0
        );
        assert_eq!(
            spec(&["frame", "aurora", "--t", "3600"], Env::default(), None)
                .unwrap()
                .steps,
            216_000
        );
    }

    #[test]
    fn cli_r13_lib_r5_steps_follow_the_library_rule() {
        for (t, steps) in [
            (0.0, 0),
            (1.0 / 60.0, 1),
            (0.999, 59),
            (1.0, 60),
            (2.5, 150),
        ] {
            assert_eq!(steps_for(t), steps, "{t}");
        }
    }
}
