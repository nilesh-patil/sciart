//! End-to-end: the real binary in an 80x24 pseudo-terminal, screen decoded by a vt100
//! emulator (SPEC-006 end-to-end layer; RND-R4, RND-R5, RND-R9, RT-R6, RT-R9, RT-R12, CLI-R7,
//! CFG-R2, CFG-R3, RT-R17, RND-R18, CLI-R12, CLI-R14, CLI-R16, CLI-R17, RT-R23).
//! Unix only (portable-pty's native backend).
#![cfg(unix)]
#![allow(clippy::disallowed_methods)] // real-time timeouts against a real process (ADR-0008 applies to generators)

use std::io::{Read, Write};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const BIN: &str = env!("CARGO_BIN_EXE_sciart");
const ALT_ENTER: &[u8] = b"\x1b[?1049h";
const ALT_LEAVE: &[u8] = b"\x1b[?1049l";
/// Every non-empty frame is one synchronized update (RT-R15, DEC mode 2026).
const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";

/// Sets its flag when dropped, so the reader thread reports its end however its loop ends.
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Session {
    screen: Arc<Mutex<vt100::Parser>>,
    raw: Arc<Mutex<Vec<u8>>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// While set, nothing is read from the pty: its buffer fills and sciart's writes stall.
    stalled: Arc<AtomicBool>,
    /// Set when the reader has seen the end of the stream: everything sciart wrote is in
    /// `raw` (the pty reports end-of-file once the process and its children have closed it).
    ended: Arc<AtomicBool>,
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
        // Likewise the terminal identity RND-R21's hint reads: a developer inside tmux or
        // Terminal.app must not change what the tests see.
        cmd.env("TERM", "xterm-256color");
        cmd.env_remove("TERM_PROGRAM");
        cmd.env_remove("COLORTERM");
        for (key, value) in env {
            cmd.env(key, value);
        }
        let child = pair.slave.spawn_command(cmd).expect("spawn sciart");
        drop(pair.slave);

        let screen = Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0)));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let stalled = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let (s, r, hold, end) = (screen.clone(), raw.clone(), stalled.clone(), ended.clone());
        thread::spawn(move || {
            let _end = SetOnDrop(end);
            let mut buf = [0u8; 8192];
            loop {
                while hold.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
                let Ok(n) = reader.read(&mut buf) else { break };
                if n == 0 {
                    break;
                }
                // Both under one critical section (raw first, as `settled_screen` takes
                // them), so the byte stream and the decoded screen always agree.
                let mut raw = r.lock().unwrap();
                s.lock().unwrap().process(&buf[..n]);
                raw.extend_from_slice(&buf[..n]);
            }
        });
        let writer = pair.master.take_writer().unwrap();
        Session {
            screen,
            raw,
            writer,
            child,
            stalled,
            ended,
            _master: pair.master,
            _config_home: config_home,
        }
    }

    fn contents(&self) -> String {
        self.screen.lock().unwrap().screen().contents()
    }

    /// The decoded screen once `pred` holds *between* frames: every synchronized update
    /// begun so far has ended, so no frame is half-applied. A pty read can split a frame,
    /// and a check that samples mid-frame sees cells the frame has not reached yet (the
    /// RND-R20 opaque-ground tests did, once in six runs under load).
    fn settled_screen(&self, what: &str, pred: impl Fn(&str) -> bool) -> vt100::Screen {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let raw = self.raw.lock().unwrap();
                let parser = self.screen.lock().unwrap();
                let ended = count(&raw, END_SYNC);
                if ended > 0
                    && ended == count(&raw, BEGIN_SYNC)
                    && pred(&parser.screen().contents())
                {
                    return parser.screen().clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what} between frames; screen:\n{}",
                self.contents()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Poll the decoded screen until `pred` holds or the timeout elapses.
    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        self.wait_for_within(Duration::from_secs(10), what, pred)
    }

    /// [`Session::wait_for`] with its own limit.
    fn wait_for_within(&self, limit: Duration, what: &str, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + limit;
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
        self.wait_raw_within(Duration::from_secs(10), what, pred);
    }

    /// [`Session::wait_raw`] with its own limit.
    fn wait_raw_within(&self, limit: Duration, what: &str, pred: impl Fn(&[u8]) -> bool) {
        let deadline = Instant::now() + limit;
        loop {
            if pred(&self.raw.lock().unwrap()) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait for a command that exits on its own (`list`), without sending input.
    fn wait_exit(self) -> (portable_pty::ExitStatus, Vec<u8>) {
        self.wait_exit_within(Duration::from_secs(10))
    }

    /// [`Session::wait_exit`] with its own limit, for runs that take time by design.
    fn wait_exit_within(mut self, limit: Duration) -> (portable_pty::ExitStatus, Vec<u8>) {
        let deadline = Instant::now() + limit;
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

/// RT-R20's allowance for the pty tests. The app's own budget is 250 ms on an idle machine
/// (the 150 ms restore deadline is pinned by the `signals` unit tests); a test measured
/// through a pty also measures the scheduler, which on a loaded machine (load average
/// 40–80) added more than 80 ms. A second still tells a prompt exit from a hang.
const SIGNAL_EXIT_ALLOWANCE: Duration = Duration::from_secs(1);

/// How long the signal tests wait before calling an exit a hang.
const HANG: Duration = Duration::from_secs(10);

/// RT-R20: send `signal` and check that sciart exits `code` within
/// [`SIGNAL_EXIT_ALLOWANCE`] of delivery. The terminal stays as it is (a stalled reader
/// stays stalled) until the exit has begun; every failure message carries the timeline of
/// process-state samples and the load average, so a failure says what the process was
/// doing rather than only that it was late.
fn assert_signal_ends_in_time(s: &mut Session, signal: &str, code: u32) {
    let pid = s.pid();
    let kill = Command::new("kill")
        .args(["-s", signal, &pid.to_string()])
        .status()
        .unwrap();
    assert!(kill.success(), "kill -s {signal}");
    // The clock starts once `kill` has returned, i.e. the signal is delivered: starting it
    // before would also time the fork and exec of `kill` itself.
    let sent = Instant::now();
    let load = load_average();
    // Each change of state with the time it was first seen, and the number of samples.
    let mut timeline = String::new();
    let (mut last, mut samples) = (None, 0);
    // The exit is dated by the start of the sample that first sees it, so a slow `ps` (a
    // busy machine) is not charged to the app.
    let took = loop {
        let at = sent.elapsed();
        let stat = process_state(pid);
        samples += 1;
        if last.as_ref() != Some(&stat) {
            timeline.push_str(&format!(" {at:.0?}:{stat:?}"));
            last = Some(stat.clone());
        }
        if exited_or_exiting(&stat) {
            timeline.push_str(&format!(" ({samples} samples)"));
            break at;
        }
        assert!(
            at < HANG,
            "sciart ignored SIG{signal} (load {load}); states:{timeline}"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert!(
        took < SIGNAL_EXIT_ALLOWANCE,
        "SIG{signal} took {took:?} to end the app (load at the signal {load}, now {}); \
         states:{timeline}",
        load_average()
    );
    // The kernel may hold an exiting process until its terminal output drains (macOS does,
    // even after SIGKILL), so the status is read once the terminal reads again.
    s.stalled.store(false, Ordering::Relaxed);
    let status = loop {
        if let Some(status) = s.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            sent.elapsed() < HANG,
            "sciart never finished exiting after SIG{signal}; states:{timeline}"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        status.exit_code(),
        code,
        "exit status after SIG{signal}; states:{timeline}"
    );
}

/// `ps`'s state letters for `pid` ("" once the process is gone and reaped).
fn process_state(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The process has called `exit` (macOS `E`), is a zombie, or is gone.
fn exited_or_exiting(stat: &str) -> bool {
    stat.is_empty() || stat.contains('E') || stat.contains('Z')
}

/// The 1, 5 and 15 minute load averages, for failure messages.
fn load_average() -> String {
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
        return text
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
    }
    Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// RT-R20: send `signal` to sciart in the state `keys` puts it in, then check it left the
/// event loop in time, restored the terminal as `q` does, exited `128 + n`, and left no
/// keep-awake helper running.
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
    assert_signal_ends_in_time(&mut s, signal, code);
    // Leaving the alternate screen is the restore's last write, so once it has been read
    // everything before it has too: no fixed drain time to outrun on a loaded machine.
    s.wait_raw("the restore", |raw| count(raw, ALT_LEAVE) >= 1);
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

/// RT-R20: with the terminal no longer reading (a stalled link), the frame write blocks
/// while holding stdout; the restore cannot get through, and the exit must not wait for it.
///
/// The stall is proven, not assumed: a stall fills the pty's buffer, after which sciart's
/// own write blocks, so the restore (whose last write leaves the alternate screen) cannot
/// reach the terminal. If the restore does arrive, the buffer had not filled when the
/// signal came; that attempt proves nothing either way, and the next one stalls longer.
/// The scene writes about 60 KB a second at 80×24, so the first stall fills a macOS pty
/// (1 KB) many times over and Linux's (tens of KB) too.
#[test]
fn rt_r20_sigterm_ends_the_app_even_when_its_writes_have_stalled() {
    let mut unproven = String::new();
    for stall in [1, 3, 9].map(Duration::from_secs) {
        match signal_a_stalled_writer(stall) {
            Ok(()) => return,
            Err(why) => {
                eprintln!("retrying with a longer stall: {why}");
                unproven.push_str(&format!("\n  {why}"));
            }
        }
    }
    panic!("the write never stalled, so RT-R20's stalled case was not exercised:{unproven}");
}

/// One attempt: stall for `stall`, signal, check the exit; `Err` when the restore got
/// through, i.e. the write had not stalled.
fn signal_a_stalled_writer(stall: Duration) -> Result<(), String> {
    let args = [
        "run",
        "cosmic_microwave_background",
        "--hud",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    let mut s = Session::start(&args);
    s.wait_for("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    s.stalled.store(true, Ordering::Relaxed);
    thread::sleep(stall);
    // Nothing more is read until the exit has begun, so this marks the stall's start.
    let read_before = s.raw.lock().unwrap().len();
    assert_signal_ends_in_time(&mut s, "TERM", 143);
    let deadline = Instant::now() + HANG;
    while !s.ended.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "the pty never reached end-of-file"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let raw = s.raw.lock().unwrap();
    let backlog = raw.len() - read_before;
    if count(&raw[read_before..], ALT_LEAVE) > 0 {
        return Err(format!(
            "stall of {stall:?}: the restore arrived ({backlog} bytes queued), so the write \
             had not stalled"
        ));
    }
    Ok(())
}

// ----- M25: terminal hint (RND-R21) -----

const HINT: &str = "sciart: hint: ";

/// RND-R21: on the Linux console the advice comes once, after the terminal is restored.
#[test]
fn rnd_r21_linux_console_hint_is_printed_once_after_the_restore() {
    let args = ["run", "starfield", "--seed", "1", "--keep-awake", "off"];
    let s = Session::start_with_env(&args, &[("TERM", "linux")]);
    s.wait_for("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    let (status, raw) = s.quit();
    assert!(status.success(), "exit status {status:?}");
    let leave = raw
        .windows(ALT_LEAVE.len())
        .position(|w| w == ALT_LEAVE)
        .expect("left the alternate screen");
    assert_eq!(count(&raw, HINT.as_bytes()), 1, "one hint");
    let after = String::from_utf8_lossy(&raw[leave..]);
    assert!(
        after.contains("Linux console"),
        "hint after the restore: {after:?}"
    );
    assert!(after.contains("--ascii"), "{after:?}");
}

/// Arguments, config text and extra environment for one [`Session::spawn`].
type Launch<'a> = (&'a [&'a str], Option<&'a str>, &'a [(&'a str, &'a str)]);

/// RND-R21: an explicit colour choice (flag, config or NO_COLOR) silences the hint, and a
/// capable terminal never gets one.
#[test]
fn rnd_r21_explicit_colour_or_a_capable_terminal_gets_no_hint() {
    let base = ["run", "starfield", "--seed", "1", "--keep-awake", "off"];
    let flag = [&base[..], &["--color", "truecolor"]].concat();
    let cases: [Launch; 4] = [
        (&flag, None, &[("TERM", "linux")]),
        (
            &base,
            Some("color = \"256\"\n"),
            &[("TERM", "tmux-256color")],
        ),
        (&base, None, &[("TERM", "linux"), ("NO_COLOR", "1")]),
        (
            &base,
            None,
            &[
                ("TERM_PROGRAM", "Apple_Terminal"),
                ("COLORTERM", "truecolor"),
            ],
        ),
    ];
    for (args, config, env) in cases {
        let s = Session::spawn(args, config, env);
        s.wait_for("a lit frame", |t| {
            t.chars().filter(|c| !c.is_whitespace()).count() > 50
        });
        let (status, raw) = s.quit();
        assert!(status.success(), "{args:?} {env:?}: {status:?}");
        assert_eq!(
            count(&raw, HINT.as_bytes()),
            0,
            "{args:?} {config:?} {env:?}"
        );
    }
}

/// RND-R21: `list` and a usage error never print the hint.
#[test]
fn rnd_r21_list_and_usage_errors_never_hint() {
    for args in [&["list"][..], &["run", "no_such_scene"][..]] {
        let out = Command::new(BIN)
            .args(args)
            .env("TERM", "linux")
            .env_remove("NO_COLOR")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains(HINT), "{args:?}: {stderr}");
    }
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

/// RT-R21: paused, once the rate has stepped down and a frame's diff is empty, the loop
/// blocks on the next event: no bytes and at most three wake-ups in 2 s; a key is drawn at
/// once. `SCIART_TRACE_WAKEUPS` makes the binary write its wake-up count to a file.
///
/// The waits are hang checks ([`STUCK`]), not timings: how soon the loop reaches the floor
/// rate and how soon a frame reaches the pty are the scheduler's. "At once" is the key
/// itself ending the blocked wait and being drawn; a loop that did not would wait on,
/// timeout after timeout, and never draw the resumed scene, which the last wait catches.
#[test]
fn rt_r21_a_paused_scene_blocks_writes_nothing_and_resumes_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("wakeups");
    let wakeups = || -> u64 {
        std::fs::read_to_string(&trace)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    let args = ["run", "starfield", "--ascii", "--keep-awake", "off"];
    let trace_env = trace.to_str().unwrap();
    let mut s = Session::start_with_env(&args, &[("SCIART_TRACE_WAKEUPS", trace_env)]);
    s.wait_for("stars", |t| {
        t.chars().filter(|c| "*+.".contains(*c)).count() > 50
    });
    s.press(" ");
    // RT-R17 steps the rate down after 1.5 s of quiet; then the loop blocks. Wait, however
    // long the machine takes, for a 2 s window that shows it: no bytes, few wake-ups.
    let blocking = Instant::now() + STUCK;
    let after = loop {
        let (bytes, woken) = (s.raw.lock().unwrap().len(), wakeups());
        thread::sleep(Duration::from_secs(2));
        let (after, woken_after) = (s.raw.lock().unwrap().len(), wakeups());
        if woken > 0 && after == bytes && woken_after - woken <= 3 {
            break after;
        }
        assert!(
            Instant::now() < blocking,
            "paused: never quiet ({} bytes, {} wake-ups in 2 s)",
            after - bytes,
            woken_after - woken
        );
    };
    s.press(" ");
    let resuming = Instant::now() + STUCK;
    while s.raw.lock().unwrap().len() == after {
        assert!(Instant::now() < resuming, "no frame after resuming");
        thread::sleep(Duration::from_millis(2));
    }
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

/// Every cell of the 80x24 screen (not just the lit ones) must have background `want`. Take
/// `screen` from [`Session::settled_screen`], never mid-frame.
fn assert_background(screen: &vt100::Screen, want: vt100::Color) {
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
    let screen = s.settled_screen("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    assert_background(&screen, vt100::Color::Rgb(0, 0, 0));
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
    let screen = s.settled_screen("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    assert_background(&screen, vt100::Color::Default);
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
    let screen = s.settled_screen("the HUD", |t| t.contains(" fps "));
    assert_background(&screen, vt100::Color::Rgb(0, 0, 0));
    s.press("?");
    let screen = s.settled_screen("the help page", |t| {
        t.contains("sciart") && t.contains("help")
    });
    assert_background(&screen, vt100::Color::Rgb(0, 0, 0));
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
    let screen = s.settled_screen("a lit frame", |t| {
        t.chars().filter(|c| !c.is_whitespace()).count() > 50
    });
    assert_background(&screen, vt100::Color::Idx(16));
    s.press("?");
    let screen = s.settled_screen("the help page", |t| {
        t.contains("sciart") && t.contains("help")
    });
    assert_background(&screen, vt100::Color::Idx(16));
    s.press("\x1b");
    let (status, _) = s.quit();
    assert!(status.success(), "exit status {status:?}");
}

// ----- M25: scriptable modes (CLI-R12, CLI-R14, CLI-R16, CLI-R17, RT-R23) -----

/// How long a wait in these tests may take before it counts as a hang. The limits tell
/// "ends" from "hangs" and nothing finer: on a loaded machine the scheduler, not the app,
/// decides how long a run takes, so a tight bound measures the machine. The timings that
/// matter are pinned without the wall clock (the `rt_r23_*` unit tests, synthetic time);
/// lower bounds ("not before the deadline") stay exact. Well under nextest's 180 s kill.
const STUCK: Duration = Duration::from_secs(60);

/// The art is on screen: enough lit cells.
fn lit(screen: &str) -> bool {
    screen.chars().filter(|c| !c.is_whitespace()).count() > 50
}

/// `sciart while … -- <command>` in the pty, a fixed scene and no keep-awake helper, with
/// the spool directory (`TMPDIR`) under the test's control.
fn start_while(flags: &[&str], command: &[&str]) -> (Session, tempfile::TempDir) {
    let spool_dir = tempfile::tempdir().unwrap();
    let mut args = vec![
        "while",
        "--only",
        "starfield",
        "--seed",
        "1",
        "--ascii",
        "--keep-awake",
        "off",
    ];
    args.extend_from_slice(flags);
    args.push("--");
    args.extend_from_slice(command);
    let tmpdir = spool_dir.path().to_str().unwrap();
    let s = Session::spawn(&args, None, &[("TMPDIR", tmpdir)]);
    (s, spool_dir)
}

/// CLI-R14: no spool file outlives the run.
fn assert_no_spool_left(dir: &std::path::Path) {
    let left: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert!(left.is_empty(), "spool files left behind: {left:?}");
}

fn find(haystack: &[u8], needle: &str) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle.as_bytes())
}

/// Where the terminal was restored: the (last) alternate-screen exit.
fn restored_at(raw: &[u8]) -> usize {
    raw.windows(ALT_LEAVE.len())
        .rposition(|w| w == ALT_LEAVE)
        .expect("left the alternate screen")
}

/// The long-running commands below start with `trap '' HUP`, which `exec` keeps: in the
/// pty sciart is the session leader, so its exit hangs up its process group and would end
/// a command it failed to kill. In a real shell sciart is not the leader and nothing
/// would; ignoring the hang-up makes the tests see what a user would. They sleep for ten
/// minutes, far past [`STUCK`], so a command that was not killed is still there when a
/// test looks for it.
/// The pid a `sh -c 'echo $$ > file; exec …'` command wrote, once it has.
fn read_pid(file: &std::path::Path) -> u32 {
    let deadline = Instant::now() + STUCK;
    loop {
        if let Some(pid) = std::fs::read_to_string(file)
            .ok()
            .and_then(|t| t.trim().parse().ok())
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "the command never wrote its pid");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn cli_r14_while_exits_with_the_commands_status_and_replays_its_output_after_the_restore() {
    for code in [0u32, 3] {
        let script = format!("echo to-stderr >&2; echo to-stdout; sleep 2; exit {code}");
        let (s, spool_dir) = start_while(&[], &["sh", "-c", &script]);
        s.wait_for_within(STUCK, "the art", lit);
        let (status, raw) = s.wait_exit_within(STUCK);
        assert_eq!(status.exit_code(), code, "exit {code}");
        assert_eq!(
            count(&raw, ALT_ENTER),
            1,
            "entered the alternate screen once"
        );
        assert_eq!(count(&raw, ALT_LEAVE), 1, "left the alternate screen once");
        let restored = restored_at(&raw);
        let out = find(&raw, "to-stdout").expect("stdout replayed");
        let err = find(&raw, "to-stderr").expect("stderr replayed");
        assert!(restored < out, "stdout only after the restore");
        assert!(
            out < err,
            "stdout, then stderr, whatever order they were written in"
        );
        assert_no_spool_left(spool_dir.path());
    }
}

#[test]
fn cli_r14_q_closes_the_art_and_keeps_waiting_and_copying() {
    let script = "echo early; sleep 3; echo late; exit 5";
    let (mut s, spool_dir) = start_while(&[], &["sh", "-c", script]);
    s.wait_for_within(STUCK, "the art", lit);
    s.press("q");
    s.wait_raw_within(STUCK, "the restore, then the output so far", |raw| {
        raw.windows(ALT_LEAVE.len()).any(|w| w == ALT_LEAVE)
            && find(raw, "early").is_some_and(|at| at > restored_at(raw))
    });
    assert!(
        s.child.try_wait().unwrap().is_none(),
        "sciart is still waiting for the command"
    );
    let (status, raw) = s.wait_exit_within(STUCK);
    assert_eq!(status.exit_code(), 5, "the command's status after q");
    let early = find(&raw, "early").unwrap();
    let late = find(&raw, "late").expect("output after q is copied too");
    assert!(early < late);
    assert_eq!(count(&raw, ALT_LEAVE), 1, "the terminal is restored once");
    assert_no_spool_left(spool_dir.path());
}

#[test]
fn cli_r14_ctrl_c_kills_the_command_and_exits_130() {
    let scratch = tempfile::tempdir().unwrap();
    let pid_file = scratch.path().join("pid");
    let script = format!(
        "trap '' HUP; echo $$ > '{}'; echo started; exec sleep 600",
        pid_file.display()
    );
    let (mut s, spool_dir) = start_while(&[], &["sh", "-c", &script]);
    s.wait_for_within(STUCK, "the art", lit);
    let pid = read_pid(&pid_file);
    assert!(running(pid), "the command runs beside the art");
    s.press("\x03");
    let (status, raw) = s.wait_exit_within(STUCK);
    assert_eq!(status.exit_code(), 130);
    assert!(!running(pid), "the command was killed");
    let restored = restored_at(&raw);
    assert!(
        find(&raw, "started").is_some_and(|at| at > restored),
        "its output so far is still replayed"
    );
    assert_no_spool_left(spool_dir.path());
}

#[test]
fn cli_r12_cli_r14_a_deadline_kills_the_command_and_exits_124() {
    let scratch = tempfile::tempdir().unwrap();
    let pid_file = scratch.path().join("pid");
    let script = format!(
        "trap '' HUP; echo $$ > '{}'; exec sleep 600",
        pid_file.display()
    );
    let (s, spool_dir) = start_while(&["--for", "2"], &["sh", "-c", &script]);
    let pid = read_pid(&pid_file);
    let (status, raw) = s.wait_exit_within(STUCK);
    assert_eq!(status.exit_code(), 124);
    assert!(!running(pid), "the command was killed at the deadline");
    assert_eq!(count(&raw, ALT_LEAVE), 1, "the terminal is restored");
    assert_no_spool_left(spool_dir.path());
}

#[test]
fn cli_r14_a_command_that_cannot_start_exits_127_before_the_alternate_screen() {
    let spool_dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args([
            "while",
            "--keep-awake",
            "off",
            "--",
            "/definitely/not/a/command",
        ])
        .env("TMPDIR", spool_dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(127));
    assert!(
        out.stdout.is_empty(),
        "no terminal set-up: {:?}",
        out.stdout
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("sciart: cannot run '/definitely/not/a/command': "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "one line: {stderr}");
    assert_no_spool_left(spool_dir.path());
    // A missing command line is a usage error.
    for args in [&["while"][..], &["while", "--"][..]] {
        let out = Command::new(BIN).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

/// The seconds left that the HUD shows (` | ends MM:SS`), if it shows them.
fn ends_secs(screen: &str) -> Option<u32> {
    let at = screen.find("| ends ")? + "| ends ".len();
    let (m, s) = screen.get(at..at + 5)?.split_once(':')?;
    Some(m.parse::<u32>().ok()? * 60 + s.parse::<u32>().ok()?)
}

/// RT-R23 / CLI-R12: the deadline is wall time, so a paused run still ends on time, and the
/// HUD counts it down.
#[test]
fn cli_r12_rt_r23_for_ends_even_a_paused_run_and_the_hud_shows_the_time_left() {
    let args = [
        "run",
        "starfield",
        "--for",
        "10",
        "--hud",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    let started = Instant::now();
    let mut s = Session::start(&args);
    s.wait_for_within(STUCK, "the time left in the HUD", |t| {
        t.contains("| ends 00:")
    });
    s.press(" ");
    let paused = s.wait_for_within(STUCK, "pause", |t| t.contains("| PAUSED"));
    // The countdown keeps moving while paused: some later frame shows less time left.
    let at_pause = ends_secs(&paused).expect("the time left with the pause");
    s.wait_for_within(STUCK, "the countdown while paused", |t| {
        ends_secs(t).is_some_and(|left| left < at_pause)
    });
    let (status, raw) = s.wait_exit_within(STUCK);
    let took = started.elapsed();
    assert!(status.success(), "a deadline is a normal end: {status:?}");
    assert!(took >= Duration::from_secs(10), "ended early: {took:?}");
    assert_eq!(
        count(&raw, ALT_LEAVE),
        1,
        "the terminal is restored as by q"
    );
    for bad in ["0", "25h", "soon"] {
        let out = Command::new(BIN)
            .args(["run", "starfield", "--for", bad])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "--for {bad}");
    }
}

/// RT-R21 / RT-R23: without a HUD nothing on screen changes while paused, so the loop
/// blocks on input until the end; the blocked span is wall time and the run still ends on
/// time.
#[test]
fn rt_r21_rt_r23_a_paused_run_without_a_hud_ends_at_its_deadline() {
    let args = [
        "run",
        "starfield",
        "--for",
        "4",
        "--ascii",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    let started = Instant::now();
    let mut s = Session::start(&args);
    s.wait_for_within(STUCK, "the art", lit);
    s.press(" ");
    let (status, raw) = s.wait_exit_within(STUCK);
    let took = started.elapsed();
    assert!(status.success(), "{status:?}");
    assert!(took >= Duration::from_secs(4), "ended early: {took:?}");
    assert_eq!(count(&raw, ALT_LEAVE), 1);
}

/// CLI-R16 through the real binary, piped: the versioned object, parsed as JSON.
#[test]
fn cli_r16_list_json_is_the_versioned_object() {
    let out = Command::new(BIN).args(["list", "--json"]).output().unwrap();
    assert!(out.status.success());
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(doc["schema"], 1);
    let scenes = doc["scenes"].as_array().unwrap();
    let starfield = scenes.iter().find(|s| s["name"] == "starfield").unwrap();
    assert_eq!(starfield["category"], "deep-cosmos");
    assert_eq!(starfield["category_label"], "Deep Cosmos");
    assert!(
        starfield["title"]
            .as_str()
            .is_some_and(|t| t.contains("Parallax"))
    );
    assert!(starfield["captions"].is_array() && starfield["aliases"].is_array());
}

/// CLI-R17: completions and the manual page come from the binary, read no config and exit 0;
/// an unknown shell is a usage error.
#[test]
fn cli_r17_completions_and_man_page_need_no_config() {
    let missing = ["--config", "/definitely/not/here.toml"];
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let out = Command::new(BIN)
            .args(missing)
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(out.status.success(), "{shell}: {out:?}");
        let script = String::from_utf8_lossy(&out.stdout);
        assert!(
            script.contains("sciart") && script.contains("while"),
            "{shell}"
        );
    }
    let out = Command::new(BIN).args(missing).arg("man").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains(".TH sciart"));
    let out = Command::new(BIN)
        .args(["completions", "tcsh"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// Wait until `pid` is no longer a live process (a killed orphan is reaped by init).
fn assert_gone(pid: u32, what: &str) {
    let deadline = Instant::now() + STUCK;
    while running(pid) {
        assert!(Instant::now() < deadline, "{what}: {pid} still runs");
        thread::sleep(Duration::from_millis(20));
    }
}

/// CLI-R14 / RT-R20: a termination signal to sciart, with the art on screen, kills the
/// command before sciart exits, within RT-R20's bound, and what it wrote is still replayed.
#[test]
fn cli_r14_rt_r20_sigterm_during_while_kills_the_command_and_exits_143() {
    let scratch = tempfile::tempdir().unwrap();
    let pid_file = scratch.path().join("pid");
    let script = format!(
        "trap '' HUP; echo $$ > '{}'; echo started; exec sleep 600",
        pid_file.display()
    );
    let (mut s, spool_dir) = start_while(&[], &["sh", "-c", &script]);
    s.wait_for_within(STUCK, "the art", lit);
    let pid = read_pid(&pid_file);
    assert!(running(pid), "the command runs beside the art");
    assert_signal_ends_in_time(&mut s, "TERM", 143);
    assert_gone(pid, "the command after SIGTERM");
    thread::sleep(Duration::from_millis(100)); // let the reader drain
    let raw = s.raw.lock().unwrap().clone();
    assert_eq!(count(&raw, ALT_LEAVE), 1, "the terminal is restored once");
    let restored = restored_at(&raw);
    assert!(
        find(&raw, "started").is_some_and(|at| at > restored),
        "its output is replayed after the restore"
    );
    assert_no_spool_left(spool_dir.path());
}

/// CLI-R14 / RT-R20: after `q` the art is gone and sciart only waits; a hang-up then still
/// kills the command, and the terminal, already given back, is not "restored" again.
#[test]
fn cli_r14_rt_r20_sighup_after_q_kills_the_command_and_leaves_the_terminal_alone() {
    let scratch = tempfile::tempdir().unwrap();
    let pid_file = scratch.path().join("pid");
    let script = format!(
        "trap '' HUP; echo $$ > '{}'; echo started; exec sleep 600",
        pid_file.display()
    );
    let (mut s, spool_dir) = start_while(&[], &["sh", "-c", &script]);
    s.wait_for_within(STUCK, "the art", lit);
    let pid = read_pid(&pid_file);
    s.press("q");
    s.wait_raw_within(STUCK, "the restore, then the output so far", |raw| {
        raw.windows(ALT_LEAVE.len()).any(|w| w == ALT_LEAVE)
            && find(raw, "started").is_some_and(|at| at > restored_at(raw))
    });
    assert!(running(pid), "q leaves the command running");
    assert_signal_ends_in_time(&mut s, "HUP", 129);
    assert_gone(pid, "the command after SIGHUP");
    thread::sleep(Duration::from_millis(100)); // let the reader drain
    let raw = s.raw.lock().unwrap().clone();
    assert_eq!(count(&raw, ALT_LEAVE), 1, "no second alternate-screen exit");
    assert_eq!(count(&raw, "started".as_bytes()), 1, "output copied once");
    assert_no_spool_left(spool_dir.path());
}

/// RT-R9: Ctrl-C in plain `sim` and `run` quits like `q` (it travels as its own action so
/// `while` can tell it apart, CLI-R14): status 0 and the terminal restored.
#[test]
fn rt_r9_cli_r14_ctrl_c_in_sim_and_run_exits_0_with_the_terminal_restored() {
    let run = ["run", "starfield", "--seed", "1", "--keep-awake", "off"];
    let sim = [
        "sim",
        "--only",
        "starfield",
        "--seed",
        "1",
        "--keep-awake",
        "off",
    ];
    for args in [&run[..], &sim[..]] {
        let mut s = Session::start(args);
        s.wait_for_within(STUCK, "the art", lit);
        s.press("\x03");
        let (status, raw) = s.wait_exit_within(STUCK);
        assert!(status.success(), "{args:?}: {status:?}");
        assert_eq!(count(&raw, ALT_ENTER), 1, "{args:?}");
        assert_eq!(count(&raw, ALT_LEAVE), 1, "{args:?}");
        let tail = &raw[restored_at(&raw).saturating_sub(64)..];
        assert!(count(tail, b"\x1b[?25h") >= 1, "{args:?}: cursor shown");
    }
}

/// CLI-R14 / RT-R21: a paused `while` whose loop blocks on the next event still notices the
/// command's exit (at the end of a blocked wait, `BLOCK_POLL`) and ends with its status.
/// The command waits for a file the test creates once the loop is seen to block.
#[test]
fn cli_r14_rt_r21_a_paused_while_notices_the_commands_exit_during_a_blocked_wait() {
    let scratch = tempfile::tempdir().unwrap();
    let go = scratch.path().join("go");
    let trace = scratch.path().join("wakeups");
    let wakeups = || -> u64 {
        std::fs::read_to_string(&trace)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    let script = format!(
        "trap '' HUP; until [ -e '{}' ]; do sleep 0.05; done; echo finished; exit 6",
        go.display()
    );
    let spool_dir = tempfile::tempdir().unwrap();
    let args = [
        "while",
        "--only",
        "starfield",
        "--seed",
        "1",
        "--ascii",
        "--keep-awake",
        "off",
        "--",
        "sh",
        "-c",
        &script,
    ];
    let env = [
        ("TMPDIR", spool_dir.path().to_str().unwrap()),
        ("SCIART_TRACE_WAKEUPS", trace.to_str().unwrap()),
    ];
    let mut s = Session::spawn(&args, None, &env);
    s.wait_for_within(STUCK, "the art", lit);
    s.press(" ");
    // RT-R17 steps the rate down after 1.5 s of quiet; then the loop blocks, waking about
    // once a second. Wait for a quiet window however long the machine takes to get there.
    let blocking = Instant::now() + STUCK;
    loop {
        let woken = wakeups();
        thread::sleep(Duration::from_millis(1500));
        if wakeups() - woken <= 3 {
            break;
        }
        assert!(Instant::now() < blocking, "the paused loop never blocked");
    }
    // Only a blocked wait can now notice the exit: without that check this never ends.
    std::fs::write(&go, "").unwrap();
    let (status, raw) = s.wait_exit_within(STUCK);
    assert_eq!(status.exit_code(), 6, "the command's status");
    assert!(
        find(&raw, "finished").is_some_and(|at| at > restored_at(&raw)),
        "its output is replayed after the restore"
    );
    assert_no_spool_left(spool_dir.path());
}
