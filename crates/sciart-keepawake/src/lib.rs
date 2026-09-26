//! Keep-awake backends (SPEC-004). M1 ships the mode model (KA-R1);
//! the `caffeinate` and `mouse` backends land in M3.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Caffeinate,
    Mouse,
    Off,
}

impl Mode {
    /// `Caffeinate` on macOS, `Off` elsewhere (KA-R1).
    pub const fn platform_default() -> Mode {
        if cfg!(target_os = "macos") {
            Mode::Caffeinate
        } else {
            Mode::Off
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Mode::Caffeinate => "caffeinate",
            Mode::Mouse => "mouse",
            Mode::Off => "off",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseModeError(String);

impl fmt::Display for ParseModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown keep-awake mode '{}' (expected caffeinate|mouse|off)",
            self.0
        )
    }
}

impl std::error::Error for ParseModeError {}

impl FromStr for Mode {
    type Err = ParseModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "caffeinate" => Ok(Mode::Caffeinate),
            "mouse" => Ok(Mode::Mouse),
            "off" => Ok(Mode::Off),
            _ => Err(ParseModeError(s.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ka_r1_parse_round_trips() {
        for m in [Mode::Caffeinate, Mode::Mouse, Mode::Off] {
            assert_eq!(m.to_string().parse::<Mode>(), Ok(m));
        }
        assert_eq!("OFF".parse::<Mode>(), Ok(Mode::Off));
        assert!("jiggle".parse::<Mode>().is_err());
    }

    #[test]
    fn ka_r1_platform_default() {
        let expected = if cfg!(target_os = "macos") {
            Mode::Caffeinate
        } else {
            Mode::Off
        };
        assert_eq!(Mode::platform_default(), expected);
    }
}
