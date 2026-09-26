//! End-to-end: the real binary in an 80x24 pseudo-terminal, screen decoded by a vt100
//! emulator (SPEC-006 end-to-end layer; RND-R4, RND-R5, RT-R6, RT-R9, CFG-R2, CFG-R3).
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

#[test]
fn rnd_r4_rt_r9_hud_reflects_keys() {
    let mut s = Session::start(&["sim", "--hud", "--ascii", "--seed", "3"]);
    s.wait_for("HUD", |t| {
        t.contains("starfield | ") && t.contains("| astro |")
    });
    s.press("t");
    s.wait_for("theme change", |t| t.contains("| rainbow |"));
    s.press(" ");
    s.wait_for("pause", |t| t.contains("| PAUSED |"));
    s.press("h");
    s.wait_for("HUD hidden", |t| !t.contains("seed 3"));
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
