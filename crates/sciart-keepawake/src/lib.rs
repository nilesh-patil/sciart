//! Keep-awake backends (SPEC-004): `caffeinate` (macOS default), `mouse` (opt-in feature), `off`.

use std::fmt;
use std::str::FromStr;

mod child;
mod jiggler;
#[cfg(feature = "mouse")]
mod mouse;

pub use child::ChildGuard;
pub use jiggler::{INTERVAL as MOUSE_INTERVAL, Jiggler, Mover};

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

/// A running keep-awake mechanism (KA-R6). Dropping it releases the assertion.
pub trait Backend {
    /// Short HUD string (KA-R5).
    fn status(&self) -> String;
    /// Called once per frame with the elapsed wall seconds.
    fn poll(&mut self, elapsed: f64);
}

/// Owns the active backend for the lifetime of the app (KA-R2).
pub struct Guard {
    backend: Box<dyn Backend>,
}

impl fmt::Debug for Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guard")
            .field("status", &self.status())
            .finish()
    }
}

/// A backend that does nothing but report a fixed status.
struct Inert(String);

impl Backend for Inert {
    fn status(&self) -> String {
        self.0.clone()
    }

    fn poll(&mut self, _elapsed: f64) {}
}

impl Guard {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend }
    }

    pub fn off() -> Self {
        Self::new(Box::new(Inert("off".into())))
    }

    /// Stand-in after `start` failed: does nothing, says so in the HUD.
    pub fn failed(mode: Mode) -> Self {
        Self::new(Box::new(Inert(format!("{mode}:unavailable"))))
    }

    pub fn status(&self) -> String {
        self.backend.status()
    }

    pub fn poll(&mut self, elapsed: f64) {
        self.backend.poll(elapsed);
    }
}

#[derive(Debug)]
pub enum KeepAwakeError {
    Unsupported { mode: Mode, reason: &'static str },
    Spawn(std::io::Error),
    Backend(String),
}

impl fmt::Display for KeepAwakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeepAwakeError::Unsupported { mode, reason } => {
                write!(f, "keep-awake '{mode}' unavailable: {reason}")
            }
            KeepAwakeError::Spawn(e) => write!(f, "keep-awake helper failed to start: {e}"),
            KeepAwakeError::Backend(e) => write!(f, "keep-awake backend failed: {e}"),
        }
    }
}

impl std::error::Error for KeepAwakeError {}

/// Arguments for `caffeinate`: prevent display/idle/disk/system sleep, declare user
/// activity, and exit by itself when process `pid` exits (KA-R3).
pub fn caffeinate_args(pid: u32) -> Vec<String> {
    vec!["-dimsu".into(), "-w".into(), pid.to_string()]
}

/// Start the backend for `mode` (KA-R2). `seed` drives the mouse path.
pub fn start(mode: Mode, seed: u64) -> Result<Guard, KeepAwakeError> {
    match mode {
        Mode::Off => Ok(Guard::off()),
        Mode::Caffeinate => start_caffeinate(),
        Mode::Mouse => start_mouse(seed),
    }
}

#[cfg(target_os = "macos")]
fn start_caffeinate() -> Result<Guard, KeepAwakeError> {
    let args = caffeinate_args(std::process::id());
    ChildGuard::spawn("caffeinate", "/usr/bin/caffeinate", &args)
        .map(|child| Guard::new(Box::new(child)))
        .map_err(KeepAwakeError::Spawn)
}

#[cfg(not(target_os = "macos"))]
fn start_caffeinate() -> Result<Guard, KeepAwakeError> {
    Err(KeepAwakeError::Unsupported {
        mode: Mode::Caffeinate,
        reason: "caffeinate is macOS-only",
    })
}

#[cfg(feature = "mouse")]
fn start_mouse(seed: u64) -> Result<Guard, KeepAwakeError> {
    let mover = mouse::EnigoMover::new().map_err(KeepAwakeError::Backend)?;
    Ok(Guard::new(Box::new(Jiggler::new(
        mover,
        seed,
        jiggler::INTERVAL,
    ))))
}

#[cfg(not(feature = "mouse"))]
fn start_mouse(_seed: u64) -> Result<Guard, KeepAwakeError> {
    Err(KeepAwakeError::Unsupported {
        mode: Mode::Mouse,
        reason: "this build lacks the `mouse` feature (pixi run rs-install-mouse)",
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

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

    struct Fake {
        polled: Rc<Cell<f64>>,
        dropped: Rc<Cell<bool>>,
    }

    impl Backend for Fake {
        fn status(&self) -> String {
            "fake:on".into()
        }
        fn poll(&mut self, elapsed: f64) {
            self.polled.set(self.polled.get() + elapsed);
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }

    #[test]
    fn ka_r6_guard_delegates_and_releases_on_drop() {
        let (polled, dropped) = (Rc::new(Cell::new(0.0)), Rc::new(Cell::new(false)));
        let mut guard = Guard::new(Box::new(Fake {
            polled: polled.clone(),
            dropped: dropped.clone(),
        }));
        guard.poll(0.5);
        guard.poll(0.25);
        assert_eq!(guard.status(), "fake:on");
        assert!((polled.get() - 0.75).abs() < 1e-12);
        assert!(!dropped.get());
        drop(guard);
        assert!(
            dropped.get(),
            "KA-R2: dropping the guard releases the backend"
        );
    }

    #[test]
    fn ka_r2_r5_off_and_failed_guards() {
        assert_eq!(Guard::off().status(), "off");
        assert_eq!(Guard::failed(Mode::Mouse).status(), "mouse:unavailable");
        assert_eq!(start(Mode::Off, 0).unwrap().status(), "off");
    }

    #[test]
    fn ka_r3_caffeinate_args_wait_on_our_pid() {
        assert_eq!(caffeinate_args(4242), vec!["-dimsu", "-w", "4242"]);
    }

    #[cfg(not(feature = "mouse"))]
    #[test]
    fn ka_r4_mouse_without_feature_is_unsupported() {
        let err = start(Mode::Mouse, 0).unwrap_err();
        assert!(matches!(
            err,
            KeepAwakeError::Unsupported {
                mode: Mode::Mouse,
                ..
            }
        ));
        assert!(err.to_string().contains("mouse"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ka_r3_caffeinate_is_unsupported_off_macos() {
        assert!(matches!(
            start(Mode::Caffeinate, 0),
            Err(KeepAwakeError::Unsupported { .. })
        ));
    }

    /// Real `caffeinate`: running while the guard lives, gone after drop.
    #[cfg(target_os = "macos")]
    #[test]
    fn ka_r3_caffeinate_runs_for_the_guard_lifetime() {
        let pattern = format!(
            "caffeinate {}",
            caffeinate_args(std::process::id()).join(" ")
        );
        let running = || {
            std::process::Command::new("pgrep")
                .args(["-f", &pattern])
                .output()
                .is_ok_and(|o| o.status.success())
        };
        let guard = start(Mode::Caffeinate, 0).unwrap();
        assert_eq!(guard.status(), "caffeinate:on");
        assert!(running(), "caffeinate should be running");
        drop(guard);
        assert!(!running(), "caffeinate should be gone after drop");
    }
}
