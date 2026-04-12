//! Config file loading and settings resolution (SPEC-005 CFG-R1..R4).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sciart_render::Theme;
use serde::Deserialize;

use crate::cli::{Cli, ThemeArg, UsageError};

pub const DEFAULT_FPS: u16 = 30;
pub const DEFAULT_INTERVAL: f64 = 15.0;

/// On-disk config; every key optional, unknown keys rejected (CFG-R3).
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub fps: Option<u16>,
    pub interval: Option<f64>,
    pub theme: Option<ThemeArg>,
    pub ascii: Option<bool>,
    pub hud: Option<bool>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Fully resolved settings the app runs with.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub fps: u16,
    pub interval: f64,
    pub theme: Theme,
    pub ascii: bool,
    pub hud: bool,
    pub seed: Option<u64>,
    pub disabled: Vec<String>,
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
    if let Some(fps) = config.fps
        && !(1..=120).contains(&fps)
    {
        return Err(invalid(format!("fps = {fps} is outside 1..=120")));
    }
    if let Some(interval) = config.interval
        && !(interval.is_finite() && interval > 0.0)
    {
        return Err(invalid(format!("interval = {interval} must be > 0")));
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

/// Merge CLI > file > defaults (CFG-R1); returns warnings for unknown `disabled` names (CFG-R4).
pub fn resolve(cli: &Cli, file: FileConfig) -> (Settings, Vec<String>) {
    let warnings = file
        .disabled
        .iter()
        .filter(|name| sciart_generators::find(name).is_none())
        .map(|name| format!("config disables unknown generator '{name}'"))
        .collect();
    let settings = Settings {
        fps: cli.fps.or(file.fps).unwrap_or(DEFAULT_FPS),
        interval: cli.interval.or(file.interval).unwrap_or(DEFAULT_INTERVAL),
        theme: cli
            .theme
            .or(file.theme)
            .map_or(Theme::default(), Theme::from),
        ascii: cli.ascii || file.ascii.unwrap_or(false),
        hud: cli.hud || file.hud.unwrap_or(false),
        seed: cli.seed,
        disabled: file.disabled,
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
                seed: None,
                disabled: vec![],
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
    fn cfg_r3_documented_example_config_is_valid() {
        let text = include_str!("../../../docs/config.example.toml");
        let f = parse(text, Path::new("docs/config.example.toml")).unwrap();
        assert_eq!(f.fps, Some(DEFAULT_FPS));
        assert_eq!(f.interval, Some(DEFAULT_INTERVAL));
    }

    #[test]
    fn cfg_r4_unknown_disabled_names_warn() {
        let f = file("disabled = [\"starfield\", \"warp_drive\"]");
        let (s, warnings) = resolve(&cli(&[]).unwrap(), f);
        assert_eq!(s.disabled, vec!["starfield", "warp_drive"]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("warp_drive"));
    }
}
