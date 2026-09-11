//! End-to-end: the real binary in an 80x24 pseudo-terminal, screen decoded by a vt100
//! emulator (SPEC-006 end-to-end layer; RND-R4, RND-R5, RND-R9, RT-R6, RT-R9, RT-R12, CLI-R7,
//! CFG-R2, CFG-R3, RT-R17, RND-R18).
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
        Self::spawn(args, None, &[])
    }

    /// Like `start`, with `config` written to `$XDG_CONFIG_HOME/sciart/config.toml`.
    fn start_with_config(args: &[&str], config: Option<&str>) -> Session {
        Self::spawn(args, config, &[])
    }

    /// Like `start`, with extra environment variables (`NO_COLOR` for RND-R18).
    fn start_with_env(args: &[&str], env: &[(&str, &str)]) -> Session {
        Self::spawn(args, None, env)
    }

    fn spawn(args: &[&str], config: Option<&str>, env: &[(&str, &str)]) -> Session {
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
        // The colour environment is part of what RND-R18 tests assert, so the test's own
        // `env` list decides it; the developer's shell must not leak in (TEST-R12).
        cmd.env_remove("NO_COLOR");
        for (key, value) in env {
            cmd.env(key, value);
        }
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

    /// Poll the raw byte stream until `pred` holds or the timeout elapses.
    fn wait_raw(&self, what: &str, pred: impl Fn(&[u8]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if pred(&self.raw.lock().unwrap()) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait for a command that exits on its own (`list`), without sending input.
    fn wait_exit(mut self) -> (portable_pty::ExitStatus, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "sciart did not exit");
            thread::sleep(Duration::from_millis(20));
        };
        thread::sleep(Duration::from_millis(100)); // let the reader drain
        let raw = self.raw.lock().unwrap().clone();
        (status, raw)
    }

    fn quit(mut self) -> (portable_pty::ExitStatus, Vec<u8>) {
        self.press("q");
        self.wait_exit()
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
    let title = " Oldest light - early sound waves | ";
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
/// gutter, CORE-R74), in the same columns as in the HUD (RND-R9). Started under the HUD and
/// then paused, hiding the HUD must still bring the canvas title back: the frozen frame is
/// re-rendered, not stepped.
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
        last_row(t).starts_with(&format!(" {STARFIELD_TITLE} | "))
    });
    s.press(" ");
    s.wait_for("pause", |t| t.contains("| PAUSED"));
    s.press("h");
    s.wait_for("canvas title under the frozen frame, HUD gone", |t| {
        last_row(t).starts_with(&format!(" {STARFIELD_TITLE} ")) && !t.contains("PAUSED")
    });
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
    s.wait_for("id in the HUD", |t| {
        last_row(t).starts_with(" starfield | ")
    });
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

// ----- M25: termination signals (RT-R20) -----

/// Direct children of `pid` (the keep-awake helper, when one runs).
fn children(pid: u32) -> Vec<u32> {
    Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Alive and not a zombie (an orphan waits a moment for init or launchd to reap it).
fn running(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|o| {
            let stat = String::from_utf8_lossy(&o.stdout);
            let stat = stat.trim();
            o.status.success() && !stat.is_empty() && !stat.starts_with('Z')
        })
}

/// RT-R20: send `signal` to sciart in the state `keys` puts it in, then check it left the
/// event loop within 250 ms, restored the terminal as `q` does, exited `128 + n`, and left
/// no keep-awake helper running.
fn assert_signal_exit(keys: &str, state: impl Fn(&str) -> bool, signal: &str, code: u32) {
    let mut s = Session::start(&["run", "starfield", "--hud", "--ascii", "--seed", "1"]);
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    if !keys.is_empty() {
        s.press(keys);
        s.wait_for("the state under test", &state);
    }
    let helpers = children(s.pid());
    // On macOS the default keep-awake is a child `caffeinate`, so the helper check below is
    // not vacuous there; on Linux the inhibitor may have exited already (no logind/polkit).
    if cfg!(target_os = "macos") {
        assert!(!helpers.is_empty(), "caffeinate runs as sciart's child");
    }
    let sent = Instant::now();
    let kill = Command::new("kill")
        .args(["-s", signal, &s.pid().to_string()])
        .status()
        .unwrap();
    assert!(kill.success(), "kill -s {signal}");
    let status = loop {
        if let Some(status) = s.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            sent.elapsed() < Duration::from_secs(10),
            "sciart ignored SIG{signal}"
        );
        thread::sleep(Duration::from_millis(5));
    };
    let took = sent.elapsed();
    assert!(
        took < Duration::from_millis(250),
        "SIG{signal} took {took:?} to end the app"
    );
    assert_eq!(status.exit_code(), code, "exit status after SIG{signal}");
    thread::sleep(Duration::from_millis(100)); // let the reader drain
    let raw = s.raw.lock().unwrap().clone();
    let leave = raw
        .windows(ALT_LEAVE.len())
        .rposition(|w| w == ALT_LEAVE)
        .expect("left the alternate screen");
    let tail = &raw[leave.saturating_sub(64)..];
    for (seq, what) in [
        (&b"\x1b[?25h"[..], "cursor shown"),
        (&b"\x1b[?1004l"[..], "focus reporting off"),
    ] {
        assert!(count(tail, seq) >= 1, "{what} on SIG{signal}");
    }
    assert_eq!(count(&raw, ALT_LEAVE), 1, "left the alternate screen once");
    for pid in helpers {
        let deadline = Instant::now() + Duration::from_secs(5);
        while running(pid) {
            assert!(
                Instant::now() < deadline,
                "keep-awake helper {pid} outlived SIG{signal}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[test]
fn rt_r20_sigterm_while_running_restores_and_exits_143() {
    assert_signal_exit("", |_| true, "TERM", 143);
}

#[test]
fn rt_r20_sighup_while_paused_restores_and_exits_129() {
    assert_signal_exit(" ", |t| t.contains("| PAUSED"), "HUP", 129);
}

#[test]
fn rt_r20_sigterm_on_the_help_page_restores_and_exits_143() {
    assert_signal_exit("?", |t| t.contains("help"), "TERM", 143);
}

#[test]
fn rt_r20_sighup_on_the_paused_help_page_restores_and_exits_129() {
    assert_signal_exit(" ?", |t| t.contains("help"), "HUP", 129);
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

/// The measured fps on the HUD row (`… | 30 fps | …`).
fn hud_fps(screen: &str) -> Option<u32> {
    let row = last_row(screen);
    let end = row.find(" fps")?;
    row[..end].rsplit(' ').next()?.parse().ok()
}

/// RT-R17: a paused picture steps the draw rate down to `low_power.fps` (the HUD's measured
/// fps falls) without entering low power, and the next change brings the full rate back.
#[test]
fn rt_r17_a_paused_picture_steps_the_rate_down_and_a_change_restores_it() {
    let config = "keep_awake = \"off\"\n";
    let args = ["run", "starfield", "--hud", "--ascii", "--seed", "3"];
    let mut s = Session::start_with_config(&args, Some(config));
    s.wait_for("full rate", |t| {
        hud_fps(t).is_some_and(|f| (20..=40).contains(&f))
    });
    s.press(" ");
    let screen = s.wait_for("stepped down", |t| {
        t.contains("| PAUSED") && hud_fps(t).is_some_and(|f| f <= 8)
    });
    assert!(!screen.contains("LOW-POWER"), "quiet is not idle: {screen}");
    s.press(" ");
    s.wait_for("full rate again", |t| {
        !t.contains("PAUSED") && hud_fps(t).is_some_and(|f| (20..=40).contains(&f))
    });
    let (status, _) = s.quit();
    assert!(status.success());
}

/// RT-R17: with nothing changing on screen (paused, HUD off), no bytes reach the terminal.
#[test]
fn rt_r17_a_paused_picture_writes_nothing() {
    let config = "keep_awake = \"off\"\n";
    let mut s = Session::start_with_config(&["run", "starfield", "--ascii"], Some(config));
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    s.press(" ");
    thread::sleep(Duration::from_millis(500)); // the pause's own frame goes out
    let before = s.raw.lock().unwrap().len();
    thread::sleep(Duration::from_millis(1500));
    let after = s.raw.lock().unwrap().len();
    assert_eq!(after, before, "paused: {} bytes written", after - before);
    let (status, _) = s.quit();
    assert!(status.success());
}

/// CLI-R10: on a terminal `list` prints the aligned header; `--plain` keeps the TSV.
#[test]
fn cli_r10_list_on_a_terminal_is_aligned_and_plain_is_tsv() {
    let s = Session::start(&["list"]);
    s.wait_raw("aligned list", |b| {
        String::from_utf8_lossy(b).contains("starfield")
    });
    let (status, raw) = s.wait_exit();
    assert!(status.success());
    let out = String::from_utf8_lossy(&raw);
    let header = out
        .lines()
        .find(|l| l.contains("NAME") && l.contains("CATEGORY") && l.contains("DESCRIPTION"))
        .unwrap_or_else(|| panic!("no aligned header:\n{out}"));
    assert!(header.starts_with("NAME"), "{header:?}");
    assert!(
        out.lines()
            .any(|l| l.starts_with("starfield") && l.contains("deep-cosmos")),
        "{out}"
    );
    assert!(
        !out.contains("starfield\tdeep-cosmos\t"),
        "a terminal gets columns, not TSV:\n{out}"
    );

    let s = Session::start(&["list", "--plain"]);
    s.wait_raw("plain list", |b| {
        String::from_utf8_lossy(b).contains("starfield\tdeep-cosmos\t")
    });
    let (status, raw) = s.wait_exit();
    assert!(status.success());
    let out = String::from_utf8_lossy(&raw);
    assert!(
        out.lines().all(|l| l.split('\t').count() == 3),
        "--plain keeps the TSV:\n{out}"
    );
    assert!(!out.contains("NAME\tCATEGORY"), "{out}");
}

/// CLI-R4/CLI-R10: piped output is the TSV of CLI-R4, with no header.
#[test]
fn cli_r4_r10_list_when_piped_is_tsv() {
    let out = Command::new(BIN).arg("list").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), sciart_generators::registry().len());
    assert!(lines.iter().all(|l| l.split('\t').count() == 3));
    assert!(!text.contains("NAME\tCATEGORY"), "{text}");
}

/// CLI-R9: `list` honours `--category`; unknown/empty selections and a bad `--speed` exit 2.
#[test]
fn cli_r9_list_category_and_usage_errors_exit_2() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(BIN)
            .args(args)
            .env("XDG_CONFIG_HOME", home.path())
            .output()
            .unwrap()
    };

    let out = run(&["list", "--category", "deep-cosmos"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.is_empty());
    assert!(
        text.lines()
            .all(|l| l.split('\t').nth(1) == Some("deep-cosmos")),
        "{text}"
    );

    for args in [
        &["sim", "--category", "nope"][..],
        &["--only", "nope", "sim"][..],
        &["--only", "starfield", "--exclude", "starfield", "sim"][..],
        &["sim", "--speed", "9"][..],
    ] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}");
    }
}

/// RND-R18/CFG-R10: `NO_COLOR` is read by the binary (not by `resolve`), prints exactly one
/// warning for the automatic downgrade, and an explicit colour mode silences it.
#[test]
fn cfg_r10_no_color_warns_once_and_an_explicit_color_silences_it() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str], no_color: &str| {
        Command::new(BIN)
            .args(args)
            // The category error exits before the TUI, after settings are resolved.
            .args(["sim", "--category", "definitely-not-a-category"])
            .env("XDG_CONFIG_HOME", home.path())
            .env("NO_COLOR", no_color)
            .output()
            .unwrap()
    };
    let warning = "sciart: warning: NO_COLOR is set; using the mono theme";

    for args in [&[][..], &["--color", "auto"][..]] {
        let out = run(args, "1");
        assert_eq!(out.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(stderr.matches(warning).count(), 1, "{args:?}: {stderr}");
    }
    for args in [
        &["--color", "truecolor"][..],
        &["--color", "256"][..],
        &["--color", "mono"][..],
    ] {
        let out = run(args, "1");
        assert_eq!(out.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("NO_COLOR is set"), "{args:?}: {stderr}");
    }
    // An empty NO_COLOR is not set (the no-color.org convention).
    let out = run(&[], "");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("NO_COLOR is set"), "{stderr}");
}

// ----- M23: help overlay and favourites (RT-R18, RT-R19) -----

/// `?` opens the static help page, whose footer and project URL must fit an 80x24 screen;
/// `Esc` closes it instead of quitting (RT-R18).
#[test]
fn rt_r18_question_mark_opens_help_and_esc_closes_it() {
    let mut s = Session::start(&[
        "run",
        "starfield",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ]);
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    s.press("?");
    let screen = s.wait_for("help page", |t| {
        t.contains("toggle this help")
            && t.contains("github.com/nilesh-patil/sciart")
            && t.contains("press ? or Esc to return to the art")
    });
    assert!(screen.contains("Esc closes help"), "{screen}");
    s.press("\x1b"); // Esc closes the page, it does not quit
    s.wait_for("the art again", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    let (status, _) = s.quit();
    assert!(status.success());
}

/// `f` toggles the current scene's favourite, and `v` flips the filter; removing the last
/// favourite turns the filter back off (RT-R19).
#[test]
fn rt_r19_f_and_v_toggle_favourites_and_the_filter() {
    let config = "keep_awake = \"off\"\nfavorites = [\"starfield\"]\n";
    let args = ["run", "starfield", "--ascii", "--seed", "1"];
    let mut s = Session::start_with_config(&args, Some(config));
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    s.press("?");
    s.wait_for("seeded favourite", |t| {
        t.contains("favourite       yes") && t.contains("filter  all scenes")
    });
    s.press("?"); // close
    s.press("v"); // filter on; the current scene is already a favourite
    s.press("?");
    s.wait_for("filter on", |t| t.contains("filter  favourites only"));
    s.press("?"); // close
    s.press("f"); // remove the last favourite: the filter turns itself off
    s.press("?");
    s.wait_for("filter off", |t| {
        t.contains("favourite       no") && t.contains("filter  all scenes")
    });
    let (status, _) = s.quit();
    assert!(status.success());
}

// ----- M23: colour capability (RND-R18, CFG-R10) -----

/// RND-R18: `--color 256` quantizes every painted colour to an xterm-256 index, so the raw
/// stream carries `38;5;n`/`48;5;n` and no truecolor `38;2;`. Asserted on the bytes, not the
/// decoded screen: vt100 normalises both forms to RGB and would hide the difference.
#[test]
fn rnd_r18_colour_256_emits_indexed_sgr() {
    let args = [
        "run",
        "starfield",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
        "--color",
        "256",
    ];
    let s = Session::start(&args);
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    s.wait_raw("an indexed SGR sequence", |bytes| {
        String::from_utf8_lossy(bytes).contains("\x1b[38;5;")
    });
    let (status, raw) = s.quit();
    assert!(status.success(), "exit status {status:?}");
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.contains("\x1b[38;5;"),
        "no indexed foreground in the stream"
    );
    assert!(
        !text.contains("\x1b[38;2;"),
        "truecolor leaked into --color 256"
    );
}

/// RND-R18: an explicit `--color truecolor` wins over `NO_COLOR`, and (because the mode was
/// explicit) no `NO_COLOR` warning is printed.
#[test]
fn rnd_r18_colour_truecolor_overrides_no_color() {
    let args = [
        "run",
        "starfield",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
        "--color",
        "truecolor",
    ];
    let s = Session::start_with_env(&args, &[("NO_COLOR", "1")]);
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    s.wait_raw("a truecolor SGR sequence", |bytes| {
        String::from_utf8_lossy(bytes).contains("\x1b[38;2;")
    });
    let (status, raw) = s.quit();
    assert!(status.success(), "exit status {status:?}");
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.contains("\x1b[38;2;"),
        "explicit truecolor did not reach the terminal"
    );
    assert!(
        !text.contains("NO_COLOR") && !text.contains("sciart: warning:"),
        "an explicit --color must not warn:\n{text}"
    );
}

/// RND-R18/CFG-R10: with `NO_COLOR` set and no `--color`, the automatic choice is mono and
/// exactly one `sciart: warning:` line names `NO_COLOR`.
#[test]
fn rnd_r18_no_color_selects_mono_with_a_warning() {
    let args = [
        "run",
        "starfield",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    let s = Session::start_with_env(&args, &[("NO_COLOR", "1")]);
    s.wait_raw("the NO_COLOR warning", |bytes| {
        String::from_utf8_lossy(bytes).contains("NO_COLOR")
    });
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    let (status, raw) = s.quit();
    assert!(status.success(), "exit status {status:?}");
    let text = String::from_utf8_lossy(&raw);
    let warnings: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("sciart: warning:") && line.contains("NO_COLOR"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "exactly one NO_COLOR warning, got {warnings:?} in:\n{text}"
    );
}

/// RND-R18/CFG-R10: an invalid `--color` value is a usage error: exit 2, with a message that
/// names the flag, before any terminal setup.
#[test]
fn rnd_r18_bad_color_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args(["--color", "42", "run", "starfield"])
        .env("XDG_CONFIG_HOME", home.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(stderr.contains("color"), "{stderr}");
}

// ----- M23: colour-blind theme (RND-R19) -----

/// RND-R19: `colorblind` is a selectable fifth theme, `t` cycles back to astro after it, and
/// it paints in truecolor (the theme is a palette, not a colour mode).
#[test]
fn rnd_r19_colorblind_theme_is_selectable_and_cycles() {
    let args = [
        "run",
        "starfield",
        "--hud",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
        "--theme",
        "colorblind",
    ];
    let mut s = Session::start(&args);
    s.wait_for("the colorblind theme in the HUD", |t| {
        t.contains("| colorblind |")
    });
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    // The documented cycle is astro -> rainbow -> mono -> aurora -> colorblind -> astro.
    s.press("t");
    s.wait_for("astro after colorblind", |t| t.contains("| astro |"));
    let (status, raw) = s.quit();
    assert!(status.success(), "exit status {status:?}");
    assert!(
        String::from_utf8_lossy(&raw).contains("\x1b[38;2;"),
        "the colorblind theme still emits truecolor"
    );
}

// ----- M24: opaque live background (RND-R20, CLI-R11) -----

/// Every cell of the 80x24 screen (not just the lit ones) must have background `want`.
fn assert_background(session: &Session, want: vt100::Color) {
    let parser = session.screen.lock().unwrap();
    let screen = parser.screen();
    for y in 0..24 {
        for x in 0..80 {
            let cell = screen.cell(y, x).expect("80x24 cell");
            assert_eq!(cell.bgcolor(), want, "background at ({x}, {y})");
        }
    }
}

/// RND-R20/CLI-R11: the live painter defaults to an explicit opaque black on every cell, so
/// terminal transparency or a wallpaper cannot show through the art as horizontal bands.
#[test]
fn rnd_r20_live_background_is_opaque_black_by_default() {
    let args = ["run", "starfield", "--seed", "1", "--keep-awake", "off"];
    let s = Session::start(&args);
    s.wait_for("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    assert_background(&s, vt100::Color::Rgb(0, 0, 0));
    let (status, _) = s.quit();
    assert!(status.success(), "exit status {status:?}");
}

/// RND-R20/CLI-R11: `--background terminal` restores the terminal's own background for
/// `Role::Background` cells (`Color::Reset`), for shells whose default background is not
/// black.
#[test]
fn rnd_r20_background_terminal_keeps_the_terminal_default() {
    let args = [
        "run",
        "starfield",
        "--seed",
        "1",
        "--keep-awake",
        "off",
        "--background",
        "terminal",
    ];
    let s = Session::start(&args);
    s.wait_for("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    assert_background(&s, vt100::Color::Default);
    let (status, _) = s.quit();
    assert!(status.success(), "exit status {status:?}");
}

/// RND-R20: the HUD and the help page are full-screen visuals too; both keep the opaque
/// ground (a `Reset` HUD row would be the same horizontal band the fix removes).
#[test]
fn rnd_r20_hud_and_help_use_the_opaque_ground() {
    let args = [
        "run",
        "starfield",
        "--seed",
        "1",
        "--keep-awake",
        "off",
        "--hud",
    ];
    let mut s = Session::start(&args);
    s.wait_for("the HUD", |t| t.contains(" fps "));
    assert_background(&s, vt100::Color::Rgb(0, 0, 0));
    s.press("?");
    s.wait_for("the help page", |t| {
        t.contains("sciart") && t.contains("help")
    });
    assert_background(&s, vt100::Color::Rgb(0, 0, 0));
    s.press("\x1b");
    let (status, _) = s.quit();
    assert!(status.success(), "exit status {status:?}");
}

/// RND-R20/RND-R18: under `--color 256` the opaque ground is the xterm black index, not a
/// truecolor escape (the help page included).
#[test]
fn rnd_r20_indexed_opaque_ground_is_xterm_black() {
    let args = [
        "run",
        "starfield",
        "--seed",
        "1",
        "--keep-awake",
        "off",
        "--color",
        "256",
    ];
    let mut s = Session::start(&args);
    s.wait_for("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    assert_background(&s, vt100::Color::Idx(16));
    s.press("?");
    s.wait_for("the help page", |t| {
        t.contains("sciart") && t.contains("help")
    });
    assert_background(&s, vt100::Color::Idx(16));
    s.press("\x1b");
    let (status, _) = s.quit();
    assert!(status.success(), "exit status {status:?}");
}
