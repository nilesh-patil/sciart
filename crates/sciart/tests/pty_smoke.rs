//! End-to-end: the real binary in an 80x24 pseudo-terminal, screen decoded by a vt100
//! emulator (SPEC-006 end-to-end layer; RND-R4, RND-R5, RND-R9, RT-R6, RT-R9, RT-R12, CLI-R7,
//! CFG-R2, CFG-R3).
//! Unix only (portable-pty's native backend).
#![cfg(unix)]
#![allow(clippy::disallowed_methods)] // real-time timeouts against a real process (ADR-0008 applies to generators)

use std::io::{Read, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const BIN: &str = env!("CARGO_BIN_EXE_sciart");
const ALT_ENTER: &[u8] = b"\x1b[?1049h";
const ALT_LEAVE: &[u8] = b"\x1b[?1049l";

struct Session {
    screen: Arc<Mutex<vt100::Parser>>,
    raw: Arc<Mutex<Vec<u8>>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Closing the master hangs up the pty, so it lives as long as the session.
    _master: Box<dyn portable_pty::MasterPty + Send>,
    _config_home: tempfile::TempDir,
}

impl Session {
    fn start(args: &[&str]) -> Session {
        Self::start_with_config(args, None)
    }

    /// Like `start`, with `config` written to `$XDG_CONFIG_HOME/sciart/config.toml`.
    fn start_with_config(args: &[&str], config: Option<&str>) -> Session {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        // Isolate from the developer's real config file.
        let config_home = tempfile::tempdir().unwrap();
        if let Some(text) = config {
            let dir = config_home.path().join("sciart");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.toml"), text).unwrap();
        }
        let mut cmd = CommandBuilder::new(BIN);
        cmd.args(args);
        cmd.env("XDG_CONFIG_HOME", config_home.path());
        let child = pair.slave.spawn_command(cmd).expect("spawn sciart");
        drop(pair.slave);

        let screen = Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0)));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (s, r) = (screen.clone(), raw.clone());
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.lock().unwrap().process(&buf[..n]);
                r.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        let writer = pair.master.take_writer().unwrap();
        Session {
            screen,
            raw,
            writer,
            child,
            _master: pair.master,
            _config_home: config_home,
        }
    }

    fn contents(&self) -> String {
        self.screen.lock().unwrap().screen().contents()
    }

    /// Poll the decoded screen until `pred` holds or the timeout elapses.
    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = self.contents();
            if pred(&text) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; screen:\n{text}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn press(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn pid(&self) -> u32 {
        self.child.process_id().expect("child pid")
    }

    fn quit(mut self) -> (portable_pty::ExitStatus, Vec<u8>) {
        self.press("q");
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                self.child.kill().ok();
                panic!("sciart did not exit after 'q'");
            }
            thread::sleep(Duration::from_millis(20));
        };
        thread::sleep(Duration::from_millis(100)); // let the reader drain
        let raw = self.raw.lock().unwrap().clone();
        (status, raw)
    }
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

#[test]
fn rnd_r5_run_draws_stars_and_restores_the_terminal() {
    let s = Session::start(&["run", "starfield", "--ascii", "--seed", "1"]);
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    let (status, raw) = s.quit();
    assert!(status.success(), "exit status {status:?}");
    assert_eq!(count(&raw, ALT_ENTER), 1, "entered alternate screen once");
    assert_eq!(count(&raw, ALT_LEAVE), 1, "left alternate screen once");
}

/// starfield's science title as the HUD and `--ascii` write it (`·` → `-`, CORE-R72).
const STARFIELD_TITLE: &str = "Parallax - near stars rush by";

fn last_row(screen: &str) -> &str {
    screen.lines().last().unwrap_or_default()
}

#[test]
fn rnd_r4_rt_r9_hud_reflects_keys() {
    // A fixed scene keeps theme and seed on screen at 80 columns. Flags never fall off the edge
    // (RND-R4 fitting, see the long-title test below), but key hints and the seed may be
    // dropped, so assertions don't rely on a separator after a flag.
    let mut s = Session::start(&["run", "starfield", "--hud", "--ascii", "--seed", "3"]);
    s.wait_for("HUD", |t| {
        t.contains(&format!("{STARFIELD_TITLE} | ")) && t.contains("| astro |")
    });
    s.press("t");
    s.wait_for("theme change", |t| t.contains("| rainbow |"));
    s.press(" ");
    s.wait_for("pause", |t| t.contains("| PAUSED"));
    s.press("h");
    s.wait_for("HUD hidden", |t| !t.contains("PAUSED"));
    let (status, _) = s.quit();
    assert!(status.success());
}

/// RND-R4 regression: at 80 columns the full HUD of a long name is ~100 characters, and plain
/// clipping used to cut `PAUSED` off the right edge. Priority fitting drops key hints and seed
/// instead, so the flag stays visible. The 32-column title is the longest allowed (GEN-R12). No
/// `--seed`: a random 20-digit seed is the realistic (worst) case.
#[test]
fn rnd_r4_r9_hud_keeps_paused_visible_with_a_long_title_at_80_columns() {
    let title = "Oldest light - early sound waves | ";
    let mut s = Session::start(&["run", "cosmic_microwave_background", "--hud", "--ascii"]);
    s.wait_for("HUD", |t| t.contains(title));
    s.press(" ");
    let screen = s.wait_for("PAUSED on an 80-column HUD", |t| t.contains("| PAUSED"));
    let hud_row = last_row(&screen);
    assert!(hud_row.starts_with(title), "{hud_row:?}");
    let (status, _) = s.quit();
    assert!(status.success());
}

/// RT-R12: with the HUD off the title sits at the bottom-left of the canvas (after the col-0
/// gutter, CORE-R74). Started under the HUD and then paused, hiding the HUD must still bring
/// the canvas title back: the frozen frame is re-rendered, not stepped.
#[test]
fn rt_r12_rnd_r9_title_moves_between_hud_and_canvas_even_while_paused() {
    let args = [
        "run",
        "starfield",
        "--hud",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    let mut s = Session::start(&args);
    s.wait_for("title in the HUD", |t| {
        last_row(t).starts_with(&format!("{STARFIELD_TITLE} | "))
    });
    s.press(" ");
    s.wait_for("pause", |t| t.contains("| PAUSED"));
    s.press("h");
    let screen = s.wait_for("canvas title under the frozen frame", |t| {
        last_row(t).starts_with(&format!(" {STARFIELD_TITLE} "))
    });
    assert!(!screen.contains("PAUSED"), "HUD gone:\n{screen}");
    let (status, _) = s.quit();
    assert!(status.success());
}

/// CLI-R7: `--no-title` leaves the canvas bare and the HUD falls back to the scene id.
#[test]
fn cli_r7_no_title_shows_the_id_in_the_hud_and_nothing_on_the_canvas() {
    let args = [
        "run",
        "starfield",
        "--no-title",
        "--hud",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    let mut s = Session::start(&args);
    s.wait_for("id in the HUD", |t| last_row(t).starts_with("starfield | "));
    s.press("h");
    let screen = s.wait_for("HUD hidden", |t| !t.contains("starfield |"));
    assert!(!screen.contains("Parallax"), "no title anywhere:\n{screen}");
    let (status, _) = s.quit();
    assert!(status.success());
}

#[test]
fn cfg_r2_missing_explicit_config_exits_2() {
    let out = Command::new(BIN)
        .args(["--config", "/definitely/not/here.toml", "list"])
        .output()
        .unwrap();
    assert!(out.status.success(), "list does not load config");

    let out = Command::new(BIN)
        .args(["--config", "/definitely/not/here.toml", "run", "starfield"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("/definitely/not/here.toml"), "{stderr}");
}

#[test]
fn cfg_r3_unknown_key_exits_2_with_location() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "fps = 30\nwarp = 9\n").unwrap();
    let out = Command::new(BIN)
        .args(["--config", path.to_str().unwrap(), "sim"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("line 2") && stderr.contains("warp"),
        "{stderr}"
    );
}

// ----- M3: keep-awake and low power -----

/// KA-R3 end-to-end: sciart's own `caffeinate` runs while it runs and is gone after `q`.
#[cfg(target_os = "macos")]
#[test]
fn ka_r3_caffeinate_lives_and_dies_with_sciart() {
    let s = Session::start(&["run", "starfield", "--hud", "--ascii", "--seed", "1"]);
    s.wait_for("keep-awake in HUD", |t| t.contains("| caffeinate:on"));
    let pattern = format!("caffeinate -dimsu -w {}", s.pid());
    let running = || {
        Command::new("pgrep")
            .args(["-f", &pattern])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    assert!(running(), "caffeinate for this sciart should be running");
    let (status, _) = s.quit();
    assert!(status.success());
    assert!(!running(), "caffeinate must not outlive sciart");
}

#[test]
fn ka_r2_unavailable_backend_is_a_warning_not_a_failure() {
    // Default test builds have no `mouse` feature, so this backend cannot start.
    let s = Session::start(&["run", "starfield", "--hud", "--keep-awake", "mouse"]);
    s.wait_for("fallback status", |t| t.contains("| mouse:unavailable"));
    let (status, raw) = s.quit();
    assert!(status.success());
    assert!(String::from_utf8_lossy(&raw).contains("warning: keep-awake 'mouse' unavailable"));
}

#[test]
fn rt_r8_focus_loss_enters_low_power_and_focus_gain_leaves_it() {
    let mut s = Session::start(&["run", "starfield", "--hud", "--keep-awake", "off"]);
    s.wait_for("HUD", |t| t.contains("| off"));
    s.press("\x1b[O"); // focus out
    s.wait_for("low power", |t| t.contains("| LOW-POWER"));
    s.press("\x1b[I"); // focus in
    s.wait_for("full power", |t| {
        !t.contains("LOW-POWER") && t.contains("| off")
    });
    let (status, raw) = s.quit();
    assert!(status.success());
    assert_eq!(
        count(&raw, b"\x1b[?1004h"),
        1,
        "focus reporting enabled once"
    );
    assert_eq!(
        count(&raw, b"\x1b[?1004l"),
        1,
        "focus reporting disabled on exit"
    );
}

#[test]
fn rt_r8_idle_time_enters_low_power_and_a_key_restores() {
    let config = "keep_awake = \"off\"\n[low_power]\nidle_after = 0.3\nfps = 4\n";
    let mut s = Session::start_with_config(&["run", "starfield", "--hud"], Some(config));
    s.wait_for("idle low power", |t| t.contains("| LOW-POWER"));
    s.press("x"); // unbound key still counts as activity
    s.wait_for("activity restores", |t| !t.contains("LOW-POWER"));
    let (status, _) = s.quit();
    assert!(status.success());
}
