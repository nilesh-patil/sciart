//! Config file loading and settings resolution (SPEC-005 CFG-R1..R7).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sciart_keepawake::Mode;
use sciart_render::Theme;
use serde::Deserialize;

use crate::cli::{Cli, KeepAwakeArg, ThemeArg, UsageError};

pub const DEFAULT_FPS: u16 = 30;
pub const DEFAULT_INTERVAL: f64 = 15.0;
pub const DEFAULT_LOW_POWER_FPS: u16 = 5;
/// Two minutes without a key press (CFG-R8; 600 s before M22).
pub const DEFAULT_IDLE_AFTER: f64 = 120.0;

/// On-disk config; every key optional, unknown keys rejected (CFG-R3).
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub fps: Option<u16>,
    pub interval: Option<f64>,
    pub theme: Option<ThemeArg>,
    pub ascii: Option<bool>,
    pub hud: Option<bool>,
    pub title: Option<bool>,
    #[serde(default)]
    pub disabled: Vec<String>,
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
    pub theme: Theme,
    pub ascii: bool,
    pub hud: bool,
    /// Show the science title (CFG-R6, CLI-R7).
    pub title: bool,
    pub seed: Option<u64>,
    pub disabled: Vec<String>,
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
/// `favorites` names (CFG-R4, CFG-R9).
pub fn resolve(cli: &Cli, file: FileConfig) -> (Settings, Vec<String>) {
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
    let settings = Settings {
        fps: cli.fps.or(file.fps).unwrap_or(DEFAULT_FPS),
        interval: cli.interval.or(file.interval).unwrap_or(DEFAULT_INTERVAL),
        theme: cli
            .theme
            .or(file.theme)
            .map_or(Theme::default(), Theme::from),
        ascii: cli.ascii || file.ascii.unwrap_or(false),
        hud: cli.hud || file.hud.unwrap_or(false),
        title: !cli.no_title && file.title.unwrap_or(true),
        seed: cli.seed,
        disabled: file.disabled,
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

    use super::*;
    use crate::cli::parse as cli;

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
                ascii: false,
                hud: false,
                title: true,
                seed: None,
                disabled: vec![],
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
        use crate::cli::{Command, select};
        let f = file("disabled = [\"chemical_reaction\", \"crystal_growth\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert!(warnings.is_empty(), "{warnings:?}");
        let names: Vec<_> = select(&Command::Sim, &s.disabled)
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
}
