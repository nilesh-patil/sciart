//! Config file loading and settings resolution (SPEC-005 CFG-R1..R11).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sciart_keepawake::Mode;
use sciart_render::{Background, Theme};
use serde::Deserialize;

use crate::cli::{BackgroundArg, Cli, ColorArg, KeepAwakeArg, ThemeArg, UsageError};

pub const DEFAULT_FPS: u16 = 30;
pub const DEFAULT_INTERVAL: f64 = 15.0;
pub const DEFAULT_LOW_POWER_FPS: u16 = 5;
/// Two minutes without a key press (CFG-R8; 600 s before M22).
pub const DEFAULT_IDLE_AFTER: f64 = 120.0;
/// Simulated seconds per wall second (CFG-R9).
pub const DEFAULT_SPEED: f64 = 1.0;

/// Resolved colour capability (RND-R18, CFG-R10).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit `Color::Rgb`: the default and `--color truecolor` (the M22 stream).
    #[default]
    Truecolor,
    /// xterm-256 indices (`38;5;n`/`48;5;n`): `--color 256`.
    Indexed,
    /// The mono theme: `--color mono`, or `auto` with `NO_COLOR` set.
    Mono,
}

impl ColorMode {
    /// Whether `paint` should emit indexed colours (RND-R18).
    pub const fn is_indexed(self) -> bool {
        matches!(self, Self::Indexed)
    }
}

/// Environment inputs that affect settings. The binary reads the process env and passes
/// this in, so tests never mutate `NO_COLOR` (RND-R18).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Env {
    /// `NO_COLOR` set to a non-empty value (<https://no-color.org>).
    pub no_color: bool,
}

impl Env {
    /// The real process environment.
    pub fn from_process() -> Self {
        Self {
            no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        }
    }
}

/// On-disk config; every key optional, unknown keys rejected (CFG-R3).
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub fps: Option<u16>,
    pub interval: Option<f64>,
    pub theme: Option<ThemeArg>,
    /// Colour capability (CFG-R10); `auto` follows `NO_COLOR` (RND-R18).
    pub color: Option<ColorArg>,
    /// Background behind every cell (CFG-R11); default `opaque`.
    pub background: Option<BackgroundArg>,
    pub ascii: Option<bool>,
    pub hud: Option<bool>,
    pub title: Option<bool>,
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Simulated seconds per wall second (CFG-R9).
    pub speed: Option<f64>,
    /// Session favourites, seeded at start (CFG-R9); never rewritten by the app.
    #[serde(default)]
    pub favorites: Vec<String>,
    pub keep_awake: Option<KeepAwakeArg>,
    #[serde(default)]
    pub low_power: LowPowerConfig,
}

/// `[low_power]` table (CFG-R5).
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LowPowerConfig {
    pub idle_after: Option<f64>,
    pub fps: Option<u16>,
}

/// Fully resolved settings the app runs with.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub fps: u16,
    pub interval: f64,
    /// Effective theme: `mono` when `color` resolves to [`ColorMode::Mono`] (RND-R18).
    pub theme: Theme,
    /// Resolved colour capability (RND-R18, CFG-R10).
    pub color: ColorMode,
    /// Resolved background (RND-R20, CFG-R11); default [`Background::Opaque`].
    pub background: Background,
    pub ascii: bool,
    pub hud: bool,
    /// Show the science title (CFG-R6, CLI-R7).
    pub title: bool,
    pub seed: Option<u64>,
    pub disabled: Vec<String>,
    /// Simulated seconds per wall second (CFG-R9).
    pub speed: f64,
    /// Config `favorites` names/aliases, seeded into the runtime (CFG-R9, RT-R19).
    pub favorites: Vec<String>,
    pub keep_awake: Mode,
    pub low_power_fps: u16,
    pub idle_after: f64,
}

/// `$XDG_CONFIG_HOME/sciart/config.toml`, else `$HOME/.config/sciart/config.toml` (CFG-R2).
pub fn default_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let non_empty = |v: Option<OsString>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
    let base = non_empty(xdg_config_home).or_else(|| non_empty(home).map(|h| h.join(".config")))?;
    Some(base.join("sciart").join("config.toml"))
}

/// Parse and validate config text; `origin` is used in error messages (CFG-R3).
pub fn parse(text: &str, origin: &Path) -> Result<FileConfig, UsageError> {
    let invalid = |msg: String| UsageError::Config(format!("{}: {msg}", origin.display()));
    let config: FileConfig = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
    let fps_ok = |fps: Option<u16>| fps.is_none_or(|f| (1..=120).contains(&f));
    let secs_ok = |s: Option<f64>| s.is_none_or(|v| v.is_finite() && v > 0.0);
    if !fps_ok(config.fps) {
        return Err(invalid(format!(
            "fps = {:?} is outside 1..=120",
            config.fps
        )));
    }
    if !secs_ok(config.interval) {
        return Err(invalid(format!(
            "interval = {:?} must be > 0",
            config.interval
        )));
    }
    if !config
        .speed
        .is_none_or(|v| v.is_finite() && (0.1..=4.0).contains(&v))
    {
        return Err(invalid(format!(
            "speed = {:?} is outside 0.1..=4",
            config.speed
        )));
    }
    if !fps_ok(config.low_power.fps) {
        return Err(invalid(format!(
            "low_power.fps = {:?} is outside 1..=120",
            config.low_power.fps
        )));
    }
    if !secs_ok(config.low_power.idle_after) {
        return Err(invalid(format!(
            "low_power.idle_after = {:?} must be > 0",
            config.low_power.idle_after
        )));
    }
    Ok(config)
}

/// Load `explicit` (must exist) or else `default` (may be missing) (CFG-R2).
pub fn load(explicit: Option<&Path>, default: Option<&Path>) -> Result<FileConfig, UsageError> {
    let (path, required) = match (explicit, default) {
        (Some(p), _) => (p, true),
        (None, Some(p)) => (p, false),
        (None, None) => return Ok(FileConfig::default()),
    };
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {
            Ok(FileConfig::default())
        }
        Err(e) => Err(UsageError::Config(format!("{}: {e}", path.display()))),
    }
}

/// Merge CLI > file > defaults (CFG-R1); returns warnings for unknown `disabled` and
/// `favorites` names (CFG-R4, CFG-R9) and for an automatic `NO_COLOR` downgrade (RND-R18).
///
/// `env` carries the ambient `NO_COLOR` decision instead of reading it here, so tests are
/// hermetic: `auto` — default, config or flag — follows it; an explicit
/// `truecolor`/`256`/`mono` wins.
pub fn resolve(cli: &Cli, file: FileConfig, env: Env) -> (Settings, Vec<String>) {
    let mut warnings: Vec<String> = file
        .disabled
        .iter()
        .filter(|name| sciart_generators::find(name).is_none())
        .map(|name| format!("config disables unknown generator '{name}'"))
        .collect();
    warnings.extend(
        file.favorites
            .iter()
            .filter(|name| sciart_generators::find(name).is_none())
            .map(|name| format!("config lists unknown generator '{name}' as a favourite")),
    );
    let (color, warn_no_color) = match cli.color.or(file.color).unwrap_or(ColorArg::Auto) {
        ColorArg::Auto if env.no_color => (ColorMode::Mono, true),
        ColorArg::Auto => (ColorMode::Truecolor, false),
        ColorArg::Truecolor => (ColorMode::Truecolor, false),
        ColorArg::Indexed256 => (ColorMode::Indexed, false),
        ColorArg::Mono => (ColorMode::Mono, false),
    };
    if warn_no_color {
        warnings.push("NO_COLOR is set; using the mono theme".to_owned());
    }
    let theme = if color == ColorMode::Mono {
        Theme::Mono
    } else {
        cli.theme
            .or(file.theme)
            .map_or(Theme::default(), Theme::from)
    };
    let settings = Settings {
        fps: cli.fps.or(file.fps).unwrap_or(DEFAULT_FPS),
        interval: cli.interval.or(file.interval).unwrap_or(DEFAULT_INTERVAL),
        theme,
        color,
        background: cli
            .background
            .or(file.background)
            .map_or(Background::Opaque, Background::from),
        ascii: cli.ascii || file.ascii.unwrap_or(false),
        hud: cli.hud || file.hud.unwrap_or(false),
        title: !cli.no_title && file.title.unwrap_or(true),
        seed: cli.seed,
        disabled: file.disabled,
        speed: cli.speed.or(file.speed).unwrap_or(DEFAULT_SPEED),
        favorites: file.favorites,
        keep_awake: cli
            .keep_awake
            .or(file.keep_awake)
            .map_or(Mode::platform_default(), Mode::from),
        low_power_fps: file.low_power.fps.unwrap_or(DEFAULT_LOW_POWER_FPS),
        idle_after: file.low_power.idle_after.unwrap_or(DEFAULT_IDLE_AFTER),
    };
    (settings, warnings)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use proptest::prelude::*;

    use super::*;
    use crate::cli::parse as cli;

    /// Resolve with a clean environment; the `cfg_r10_…` tests call `super::resolve` with
    /// an explicit `Env`.
    fn resolve(cli: &Cli, file: FileConfig) -> (Settings, Vec<String>) {
        super::resolve(cli, file, Env::default())
    }

    fn file(text: &str) -> FileConfig {
        parse(text, Path::new("test.toml")).unwrap()
    }

    #[test]
    fn cfg_r1_defaults_without_file_or_flags() {
        let (s, warnings) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert_eq!(
            s,
            Settings {
                fps: 30,
                interval: 15.0,
                theme: Theme::Astro,
                color: ColorMode::Truecolor,
                background: Background::Opaque,
                ascii: false,
                hud: false,
                title: true,
                seed: None,
                disabled: vec![],
                speed: DEFAULT_SPEED,
                favorites: vec![],
                keep_awake: Mode::platform_default(),
                low_power_fps: DEFAULT_LOW_POWER_FPS,
                idle_after: DEFAULT_IDLE_AFTER,
            }
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn cfg_r1_cli_beats_file_beats_default() {
        let f = file("fps = 45\ntheme = \"mono\"\nhud = true\ninterval = 5");
        let (s, _) = resolve(&cli(&["--fps", "60", "--interval", "2"]).unwrap(), f);
        assert_eq!(s.fps, 60, "flag wins");
        assert_eq!(s.interval, 2.0, "flag wins");
        assert_eq!(s.theme, Theme::Mono, "file wins over default");
        assert!(s.hud, "file wins over default");
        assert!(!s.ascii, "default");
    }

    #[test]
    fn rnd_r16_cfg_r1_aurora_from_flag_or_file() {
        let (s, _) = resolve(&cli(&["--theme", "aurora"]).unwrap(), FileConfig::default());
        assert_eq!(s.theme, Theme::Aurora, "flag");
        let (s, _) = resolve(&cli(&[]).unwrap(), file("theme = \"aurora\""));
        assert_eq!(s.theme, Theme::Aurora, "file");
    }

    #[test]
    fn cfg_r1_boolean_flags_can_only_turn_on() {
        let (s, _) = resolve(&cli(&["--ascii"]).unwrap(), file("ascii = false"));
        assert!(s.ascii);
        let (s, _) = resolve(&cli(&[]).unwrap(), file("ascii = true"));
        assert!(s.ascii);
    }

    #[test]
    fn cfg_r2_default_path_prefers_xdg_then_home() {
        assert_eq!(
            default_path(Some("/x".into()), Some("/h".into())),
            Some(PathBuf::from("/x/sciart/config.toml"))
        );
        assert_eq!(
            default_path(None, Some("/h".into())),
            Some(PathBuf::from("/h/.config/sciart/config.toml"))
        );
        assert_eq!(
            default_path(Some("".into()), Some("/h".into())),
            Some(PathBuf::from("/h/.config/sciart/config.toml")),
            "empty XDG_CONFIG_HOME is ignored"
        );
        assert_eq!(default_path(None, None), None);
    }

    #[test]
    fn cfg_r2_missing_default_is_fine_missing_explicit_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        assert_eq!(load(None, Some(&missing)), Ok(FileConfig::default()));
        assert_eq!(load(None, None), Ok(FileConfig::default()));
        let err = load(Some(&missing), None).unwrap_err();
        assert!(
            matches!(&err, UsageError::Config(m) if m.contains("nope.toml")),
            "{err}"
        );
    }

    #[test]
    fn cfg_r2_explicit_file_wins_over_default() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.toml"), dir.path().join("b.toml"));
        fs::write(&a, "fps = 10").unwrap();
        fs::write(&b, "fps = 20").unwrap();
        assert_eq!(load(Some(&a), Some(&b)).unwrap().fps, Some(10));
        assert_eq!(load(None, Some(&b)).unwrap().fps, Some(20));
    }

    #[test]
    fn cfg_r3_unknown_key_reports_path_and_location() {
        let err = parse("fps = 30\nfsp = 3\n", Path::new("/etc/sciart.toml")).unwrap_err();
        let UsageError::Config(msg) = err else {
            panic!("wrong error kind")
        };
        assert!(msg.contains("/etc/sciart.toml"), "{msg}");
        assert!(msg.contains("line 2"), "{msg}");
        assert!(msg.contains("fsp"), "{msg}");
    }

    #[test]
    fn cfg_r3_values_are_validated_like_the_cli() {
        for bad in [
            "fps = 0",
            "fps = 121",
            "interval = 0",
            "interval = -2",
            "theme = \"neon\"",
        ] {
            assert!(parse(bad, Path::new("c.toml")).is_err(), "{bad}");
        }
        assert_eq!(file("interval = 0.5").interval, Some(0.5));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// CFG-R3/TEST-R4: arbitrary text is rejected, never a panic. The config module is
        /// binary-only, so this proptest (not the cargo-fuzz crate) is its regression net
        /// (IMP-14).
        #[test]
        fn cfg_r3_parse_never_panics_on_arbitrary_text(text in any::<String>()) {
            let _ = parse(&text, Path::new("fuzz.toml"));
        }
    }

    #[test]
    fn cfg_r9_speed_defaults_to_1_and_cli_overrides_file() {
        let (s, _) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert_eq!(s.speed, 1.0);
        let (s, _) = resolve(&cli(&[]).unwrap(), file("speed = 2.0"));
        assert_eq!(s.speed, 2.0, "file wins over the default");
        let (s, _) = resolve(&cli(&["--speed", "0.5"]).unwrap(), file("speed = 2.0"));
        assert_eq!(s.speed, 0.5, "flag wins over the file");
    }

    #[test]
    fn cfg_r9_speed_values_are_validated_like_the_cli() {
        for bad in [
            "speed = 0",
            "speed = 0.05",
            "speed = 4.1",
            "speed = nan",
            "speed = inf",
            "speed = \"fast\"",
        ] {
            assert!(parse(bad, Path::new("c.toml")).is_err(), "{bad}");
        }
        assert_eq!(file("speed = 4").speed, Some(4.0));
    }

    #[test]
    fn cfg_r10_color_defaults_to_auto_then_truecolor() {
        let (s, warnings) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert_eq!(s.color, ColorMode::Truecolor);
        assert_eq!(s.theme, Theme::Astro);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(FileConfig::default().color, None, "auto is the default");
    }

    #[test]
    fn cfg_r10_no_color_selects_mono_unless_a_mode_is_explicit() {
        let env = Env { no_color: true };
        let warning = "NO_COLOR is set; using the mono theme";

        // The default (no key, no flag) follows NO_COLOR, as does an explicit `auto` in
        // either place.
        for args in [vec![], vec!["--color", "auto"]] {
            let (s, warnings) = super::resolve(&cli(&args).unwrap(), FileConfig::default(), env);
            assert_eq!(s.color, ColorMode::Mono, "{args:?}");
            assert_eq!(s.theme, Theme::Mono, "mono overrides the palette");
            assert_eq!(warnings, [warning], "{args:?}");
        }
        let (s, warnings) = super::resolve(&cli(&[]).unwrap(), file("color = \"auto\""), env);
        assert_eq!(s.color, ColorMode::Mono, "config auto");
        assert_eq!(warnings, [warning]);

        // An explicit truecolor/256/mono wins over the environment, with no warning.
        for (args, text, want) in [
            (vec!["--color", "truecolor"], "", ColorMode::Truecolor),
            (vec!["--color", "256"], "", ColorMode::Indexed),
            (vec!["--color", "mono"], "", ColorMode::Mono),
            (vec![], "color = \"truecolor\"", ColorMode::Truecolor),
            (vec![], "color = \"256\"", ColorMode::Indexed),
            (vec![], "color = \"mono\"", ColorMode::Mono),
        ] {
            let f = if text.is_empty() {
                FileConfig::default()
            } else {
                file(text)
            };
            let (s, warnings) = super::resolve(&cli(&args).unwrap(), f, env);
            assert_eq!(s.color, want, "{args:?} {text:?}");
            assert!(warnings.is_empty(), "explicit mode: {warnings:?}");
        }
    }

    #[test]
    fn cfg_r10_cli_color_beats_file_and_mono_forces_the_theme() {
        let (s, _) = super::resolve(
            &cli(&["--color", "256"]).unwrap(),
            file("color = \"mono\""),
            Env::default(),
        );
        assert_eq!(s.color, ColorMode::Indexed, "flag wins");
        let (s, _) = super::resolve(
            &cli(&["--color", "mono", "--theme", "aurora"]).unwrap(),
            FileConfig::default(),
            Env::default(),
        );
        assert_eq!(
            (s.color, s.theme),
            (ColorMode::Mono, Theme::Mono),
            "--color mono overrides --theme"
        );
        let (s, _) = super::resolve(
            &cli(&[]).unwrap(),
            file("color = \"256\"\ntheme = \"rainbow\""),
            Env::default(),
        );
        assert_eq!((s.color, s.theme), (ColorMode::Indexed, Theme::Rainbow));
    }

    #[test]
    fn cfg_r10_invalid_color_value_is_rejected() {
        for bad in ["color = \"neon\"", "color = true", "color = 256"] {
            assert!(parse(bad, Path::new("c.toml")).is_err(), "{bad}");
        }
        assert_eq!(
            file("color = \"256\"").color,
            Some(crate::cli::ColorArg::Indexed256)
        );
    }

    #[test]
    fn cfg_r5_keep_awake_and_low_power_defaults() {
        let (s, _) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert_eq!(s.keep_awake, Mode::platform_default());
        assert_eq!(s.low_power_fps, 5);
        assert_eq!(s.idle_after, 120.0); // CFG-R8
    }

    #[test]
    fn cfg_r5_file_values_and_cli_precedence() {
        let f = file("keep_awake = \"mouse\"\n[low_power]\nidle_after = 30\nfps = 2\n");
        let (s, _) = resolve(&cli(&[]).unwrap(), f);
        assert_eq!(
            (s.keep_awake, s.low_power_fps, s.idle_after),
            (Mode::Mouse, 2, 30.0)
        );
        let f = file("keep_awake = \"mouse\"");
        let (s, _) = resolve(&cli(&["--keep-awake", "off"]).unwrap(), f);
        assert_eq!(s.keep_awake, Mode::Off, "flag wins");
    }

    #[test]
    fn cfg_r5_ka_r7_config_accepts_inhibit() {
        let (s, _) = resolve(&cli(&[]).unwrap(), file("keep_awake = \"inhibit\""));
        assert_eq!(s.keep_awake, Mode::Inhibit);
        let (s, _) = resolve(&cli(&["--keep-awake", "inhibit"]).unwrap(), file(""));
        assert_eq!(s.keep_awake, Mode::Inhibit);
    }

    #[test]
    fn cfg_r5_low_power_values_and_keys_are_validated() {
        for bad in [
            "[low_power]\nidle_after = 0",
            "[low_power]\nidle_after = -3",
            "[low_power]\nfps = 0",
            "[low_power]\nfps = 500",
            "[low_power]\nenabled = true",
            "keep_awake = \"jiggle\"",
        ] {
            assert!(parse(bad, Path::new("c.toml")).is_err(), "{bad}");
        }
    }

    #[test]
    fn cfg_r3_documented_example_config_is_valid() {
        let text = include_str!("../../../docs/config.example.toml");
        let f = parse(text, Path::new("docs/config.example.toml")).unwrap();
        assert_eq!(f.fps, Some(DEFAULT_FPS));
        assert_eq!(f.interval, Some(DEFAULT_INTERVAL));
        assert_eq!(f.title, Some(true), "CFG-R6 key documented");
    }

    #[test]
    fn cfg_r8_low_power_starts_after_two_minutes_idle_and_fps_stays_30() {
        let (s, _) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert_eq!(s.idle_after, 120.0);
        assert_eq!(s.fps, 30);
        assert_eq!(s.low_power_fps, 5);
        let text = include_str!("../../../docs/config.example.toml");
        let f = parse(text, Path::new("docs/config.example.toml")).unwrap();
        assert_eq!(
            f.low_power.idle_after,
            Some(120.0),
            "the example shows the default"
        );
        assert_eq!(f.low_power.fps, Some(5));
    }

    #[test]
    fn cfg_r6_cli_r7_title_defaults_on_and_no_title_can_only_turn_it_off() {
        let title = |args: &[&str], text: &str| resolve(&cli(args).unwrap(), file(text)).0.title;
        assert!(title(&[], ""), "default on");
        assert!(!title(&[], "title = false"), "file wins over default");
        assert!(title(&[], "title = true"));
        assert!(!title(&["--no-title"], "title = true"), "flag beats file");
        assert!(!title(&["--no-title"], ""));
        assert!(!title(&["--no-title"], "title = false"));
        assert!(parse("title = \"yes\"", Path::new("c.toml")).is_err());
    }

    #[test]
    fn cfg_r7_retired_name_in_disabled_disables_the_renamed_scene_silently() {
        use crate::cli::{Command, Filters, select};
        let f = file("disabled = [\"chemical_reaction\", \"crystal_growth\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert!(warnings.is_empty(), "{warnings:?}");
        let names: Vec<_> = select(&Command::Sim, &s.disabled, &Filters::default())
            .unwrap()
            .iter()
            .map(|e| e.meta.name)
            .collect();
        assert_eq!(names.len(), sciart_generators::registry().len() - 2);
        assert!(!names.contains(&"gray_scott") && !names.contains(&"fractal_growth"));
    }

    #[test]
    fn cfg_r4_unknown_disabled_names_warn() {
        let f = file("disabled = [\"starfield\", \"warp_drive\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert_eq!(s.disabled, vec!["starfield", "warp_drive"]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("warp_drive"));
    }

    #[test]
    fn cfg_r9_favorites_default_empty_and_seed_the_session_set() {
        let (s, warnings) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert!(s.favorites.is_empty());
        assert!(warnings.is_empty());

        let f = file("favorites = [\"starfield\", \"chemical_reaction\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert_eq!(s.favorites, vec!["starfield", "chemical_reaction"]);
        assert!(
            warnings.is_empty(),
            "the retired alias resolves without a warning: {warnings:?}"
        );
    }

    #[test]
    fn cfg_r9_unknown_favourite_names_warn_like_disabled() {
        let f = file("favorites = [\"starfield\", \"warp_drive\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert_eq!(s.favorites, vec!["starfield", "warp_drive"]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("warp_drive"), "{warnings:?}");
        assert!(warnings[0].contains("favourite"), "{warnings:?}");
    }

    #[test]
    fn cfg_r9_favorites_are_file_only_and_do_not_affect_disabled() {
        let f = file("favorites = [\"starfield\"]\ndisabled = [\"aurora\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert_eq!(s.favorites, vec!["starfield"]);
        assert_eq!(s.disabled, vec!["aurora"]);
        assert!(warnings.is_empty());
    }
    #[test]
    fn cfg_r11_background_defaults_to_opaque() {
        let (s, warnings) = resolve(&cli(&[]).unwrap(), FileConfig::default());
        assert_eq!(s.background, Background::Opaque);
        assert!(warnings.is_empty());
    }

    #[test]
    fn cfg_r11_background_file_value_and_cli_precedence() {
        let (s, _) = resolve(&cli(&[]).unwrap(), file("background = \"terminal\""));
        assert_eq!(s.background, Background::Terminal, "file");
        let (s, _) = resolve(&cli(&[]).unwrap(), file("background = \"opaque\""));
        assert_eq!(s.background, Background::Opaque);
        let (s, _) = resolve(
            &cli(&["--background", "opaque"]).unwrap(),
            file("background = \"terminal\""),
        );
        assert_eq!(s.background, Background::Opaque, "flag beats file");
        let (s, _) = resolve(
            &cli(&["--background", "terminal"]).unwrap(),
            FileConfig::default(),
        );
        assert_eq!(s.background, Background::Terminal, "flag beats default");
    }

    #[test]
    fn cfg_r11_invalid_background_is_rejected_with_path_and_location() {
        let err = parse("background = \"windows\"\n", Path::new("/etc/sciart.toml")).unwrap_err();
        let UsageError::Config(msg) = err else {
            panic!("wrong error kind")
        };
        assert!(msg.contains("/etc/sciart.toml"), "{msg}");
        assert!(msg.contains("line 1"), "{msg}");
        assert!(msg.contains("windows"), "{msg}");
    }
}
