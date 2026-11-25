//! Keep-awake backends (SPEC-004): `caffeinate` (macOS default), `inhibit` (Linux default,
//! ADR-0028), `mouse` (opt-in feature), `off`.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
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
    Inhibit,
    Mouse,
    Off,
}

/// The operating systems whose keep-awake defaults differ (KA-R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Other,
}

impl Os {
    /// The OS this binary was built for.
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

impl Mode {
    /// The default mode for `os`, given an environment lookup (KA-R8): `Caffeinate` on
    /// macOS; on Linux `Inhibit` for a local session and `Off` for a remote one (any SSH
    /// session, whatever `DISPLAY` says, since X forwarding sets it too: keeping the remote
    /// host awake is not what its viewer wants); `Off` elsewhere. An empty variable counts
    /// as unset.
    pub fn default_for(os: Os, env: impl Fn(&str) -> Option<OsString>) -> Mode {
        let set = |key: &str| env(key).is_some_and(|v| !v.is_empty());
        match os {
            Os::MacOs => Mode::Caffeinate,
            Os::Linux if set("SSH_CONNECTION") || set("SSH_TTY") => Mode::Off,
            Os::Linux => Mode::Inhibit,
            Os::Other => Mode::Off,
        }
    }

    /// [`Mode::default_for`] with this OS and the process environment (KA-R1, KA-R8).
    pub fn platform_default() -> Mode {
        Self::default_for(Os::current(), |key| std::env::var_os(key))
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Mode::Caffeinate => "caffeinate",
            Mode::Inhibit => "inhibit",
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
            "unknown keep-awake mode '{}' (expected caffeinate|inhibit|mouse|off)",
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
            "inhibit" => Ok(Mode::Inhibit),
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
    /// The same text written into `out`, for the per-frame HUD (RT-R14). The default
    /// allocates through [`Backend::status`]; the built-in backends write in place.
    fn write_status(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        out.write_str(&self.status())
    }
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

    fn write_status(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        out.write_str(&self.0)
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

    /// Replace `out` with the status text, reusing its buffer (RT-R14: no allocation per
    /// frame once `out` has grown to fit).
    pub fn write_status(&self, out: &mut String) {
        out.clear();
        // Writing to a `String` cannot fail.
        let _ = self.backend.write_status(out);
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

/// The Linux inhibitor helpers, in order of preference (KA-R7, ADR-0028).
pub const INHIBIT_HELPERS: [&str; 2] = ["systemd-inhibit", "elogind-inhibit"];

/// Arguments for an inhibitor helper: hold an `idle:sleep` block lock for as long as `cat`
/// runs. `cat` reads the pipe whose write end the guard keeps, so the lock goes when sciart
/// does, however it exits (KA-R7).
pub fn inhibit_args() -> Vec<String> {
    [
        "--what=idle:sleep",
        "--mode=block",
        "--who=sciart",
        "--why=sciart is running",
        "cat",
    ]
    .map(String::from)
    .to_vec()
}

/// The first of [`INHIBIT_HELPERS`] found on `path` (a `PATH` value), preferring
/// `systemd-inhibit` wherever it sits; `exists` says whether a candidate is a runnable
/// file. Empty `PATH` entries are skipped, not read as the working directory (KA-R7).
pub fn find_inhibitor(path: Option<&OsStr>, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let path = path?;
    INHIBIT_HELPERS.iter().find_map(|helper| {
        std::env::split_paths(path)
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| dir.join(helper))
            .find(|candidate| exists(candidate))
    })
}

/// Start the backend for `mode` (KA-R2). `seed` drives the mouse path.
pub fn start(mode: Mode, seed: u64) -> Result<Guard, KeepAwakeError> {
    match mode {
        Mode::Off => Ok(Guard::off()),
        Mode::Caffeinate => start_caffeinate(),
        Mode::Inhibit => start_inhibit(),
        Mode::Mouse => start_mouse(seed),
    }
}

#[cfg(target_os = "linux")]
fn start_inhibit() -> Result<Guard, KeepAwakeError> {
    use std::os::unix::fs::PermissionsExt;

    let runnable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let helper = find_inhibitor(std::env::var_os("PATH").as_deref(), runnable).ok_or(
        KeepAwakeError::Unsupported {
            mode: Mode::Inhibit,
            reason: "neither systemd-inhibit nor elogind-inhibit is on PATH",
        },
    )?;
    ChildGuard::spawn_piped("inhibit", &helper, &inhibit_args())
        .map(|child| Guard::new(Box::new(child)))
        .map_err(KeepAwakeError::Spawn)
}

#[cfg(not(target_os = "linux"))]
fn start_inhibit() -> Result<Guard, KeepAwakeError> {
    Err(KeepAwakeError::Unsupported {
        mode: Mode::Inhibit,
        reason: "inhibit is Linux-only (systemd-inhibit or elogind-inhibit)",
    })
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
        for m in [Mode::Caffeinate, Mode::Inhibit, Mode::Mouse, Mode::Off] {
            assert_eq!(m.to_string().parse::<Mode>(), Ok(m));
        }
        assert_eq!("OFF".parse::<Mode>(), Ok(Mode::Off));
        assert!("jiggle".parse::<Mode>().is_err());
    }

    #[test]
    fn ka_r7_inhibit_parses_and_displays() {
        assert_eq!("inhibit".parse::<Mode>(), Ok(Mode::Inhibit));
        assert_eq!(Mode::Inhibit.to_string(), "inhibit");
        assert_eq!(Guard::failed(Mode::Inhibit).status(), "inhibit:unavailable");
        assert!(ParseModeError("x".into()).to_string().contains("inhibit"));
    }

    #[test]
    fn ka_r7_helper_args_hold_an_idle_sleep_block_lock_around_cat() {
        assert_eq!(
            inhibit_args(),
            vec![
                "--what=idle:sleep",
                "--mode=block",
                "--who=sciart",
                "--why=sciart is running",
                "cat",
            ]
        );
    }

    #[test]
    fn ka_r7_helper_choice_prefers_systemd_then_elogind_in_path_order() {
        let path = std::env::join_paths(["/opt/a", "", "/usr/bin"]).unwrap();
        let only = |present: &'static [&'static str]| {
            move |p: &Path| present.iter().any(|q| Path::new(q) == p)
        };
        // systemd-inhibit wins over elogind-inhibit wherever each sits on PATH.
        assert_eq!(
            find_inhibitor(
                Some(&path),
                only(&["/opt/a/elogind-inhibit", "/usr/bin/systemd-inhibit"])
            ),
            Some(PathBuf::from("/usr/bin/systemd-inhibit"))
        );
        // The first PATH entry that has it.
        assert_eq!(
            find_inhibitor(
                Some(&path),
                only(&["/opt/a/systemd-inhibit", "/usr/bin/systemd-inhibit"])
            ),
            Some(PathBuf::from("/opt/a/systemd-inhibit"))
        );
        assert_eq!(
            find_inhibitor(Some(&path), only(&["/usr/bin/elogind-inhibit"])),
            Some(PathBuf::from("/usr/bin/elogind-inhibit"))
        );
        // Empty PATH entries are skipped rather than read as the working directory.
        assert_eq!(
            find_inhibitor(Some(&path), only(&["systemd-inhibit"])),
            None
        );
        assert_eq!(find_inhibitor(Some(&path), only(&[])), None);
        assert_eq!(find_inhibitor(None, |_| true), None);
    }

    fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn ka_r8_default_is_a_function_of_os_and_session() {
        let local = env_of(&[("DISPLAY", ":0")]);
        let bare = env_of(&[]);
        let ssh = env_of(&[("SSH_CONNECTION", "10.0.0.2 5000 10.0.0.1 22")]);
        let ssh_tty = env_of(&[("SSH_TTY", "/dev/pts/3")]);
        let ssh_x = env_of(&[("SSH_CONNECTION", "a"), ("DISPLAY", "localhost:10.0")]);
        let ssh_wayland = env_of(&[("SSH_TTY", "/dev/pts/3"), ("WAYLAND_DISPLAY", "wayland-0")]);
        let ssh_empty = env_of(&[("SSH_CONNECTION", ""), ("SSH_TTY", "")]);

        for env in [&local as &dyn Fn(&str) -> Option<OsString>, &ssh, &bare] {
            assert_eq!(Mode::default_for(Os::MacOs, env), Mode::Caffeinate);
            assert_eq!(Mode::default_for(Os::Other, env), Mode::Off);
        }
        assert_eq!(Mode::default_for(Os::Linux, &local), Mode::Inhibit);
        assert_eq!(
            Mode::default_for(Os::Linux, &bare),
            Mode::Inhibit,
            "a local console"
        );
        assert_eq!(
            Mode::default_for(Os::Linux, &ssh),
            Mode::Off,
            "remote: SSH_CONNECTION"
        );
        assert_eq!(
            Mode::default_for(Os::Linux, &ssh_tty),
            Mode::Off,
            "remote: SSH_TTY"
        );
        // X forwarding sets DISPLAY too: a session is remote whatever the display says.
        assert_eq!(
            Mode::default_for(Os::Linux, &ssh_x),
            Mode::Off,
            "forwarded display is still remote"
        );
        assert_eq!(Mode::default_for(Os::Linux, &ssh_wayland), Mode::Off);
        assert_eq!(
            Mode::default_for(Os::Linux, &ssh_empty),
            Mode::Inhibit,
            "empty = unset"
        );
    }

    #[test]
    fn ka_r1_r8_platform_default_uses_the_real_os_and_environment() {
        assert_eq!(
            Mode::platform_default(),
            Mode::default_for(Os::current(), |k| std::env::var_os(k))
        );
        let os = if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        };
        assert_eq!(Os::current(), os);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn ka_r7_inhibit_is_unsupported_off_linux() {
        let err = start(Mode::Inhibit, 0).unwrap_err();
        assert!(matches!(
            err,
            KeepAwakeError::Unsupported {
                mode: Mode::Inhibit,
                ..
            }
        ));
    }

    /// The real helper on a Linux host: it may be missing (no logind), refused by polkit
    /// (`inhibit:exited` soon after) or running; all three are acceptable outcomes (KA-R2).
    #[cfg(target_os = "linux")]
    #[test]
    fn ka_r7_inhibit_on_linux_starts_or_reports() {
        match start(Mode::Inhibit, 0) {
            Ok(mut guard) => {
                guard.poll(0.0);
                let status = guard.status();
                assert!(
                    status == "inhibit:on" || status == "inhibit:exited",
                    "{status}"
                );
            }
            Err(e) => assert!(
                matches!(
                    e,
                    KeepAwakeError::Unsupported { .. } | KeepAwakeError::Spawn(_)
                ),
                "{e}"
            ),
        }
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
