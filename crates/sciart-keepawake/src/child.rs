//! A helper process whose lifetime is tied to a guard (KA-R3, KA-R7).

use std::ffi::OsStr;
use std::io;
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::Backend;

#[derive(Debug)]
pub struct ChildGuard {
    label: &'static str,
    child: Child,
    exited: bool,
    /// The write end of the helper's stdin, for a piped helper (KA-R7). Nothing is ever
    /// written: the helper waits for end-of-file, which comes when this is dropped or when
    /// this process dies for any reason, `SIGKILL` included.
    stdin: Option<ChildStdin>,
}

impl ChildGuard {
    /// Spawn `program args…` with all stdio detached.
    pub fn spawn(label: &'static str, program: &str, args: &[String]) -> io::Result<Self> {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            label,
            child,
            exited: false,
            stdin: None,
        })
    }

    /// Spawn `program args…` reading a pipe whose write end the guard keeps (KA-R7): the
    /// helper sees end-of-file when the guard drops or this process dies. std opens the
    /// pipe close-on-exec, so no other child inherits the write end and keeps it open.
    #[cfg(unix)]
    pub fn spawn_piped(
        label: &'static str,
        program: impl AsRef<OsStr>,
        args: &[String],
    ) -> io::Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        Ok(Self {
            label,
            child,
            exited: false,
            stdin,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Backend for ChildGuard {
    fn status(&self) -> String {
        let mut out = String::new();
        // Writing to a `String` cannot fail.
        let _ = self.write_status(&mut out);
        out
    }

    fn write_status(&self, out: &mut dyn std::fmt::Write) -> std::fmt::Result {
        let state = if self.exited { "exited" } else { "on" };
        write!(out, "{}:{state}", self.label)
    }

    fn poll(&mut self, _elapsed: f64) {
        if !self.exited && matches!(self.child.try_wait(), Ok(Some(_))) {
            self.exited = true;
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Close the pipe first (KA-R7): a helper that outlives the kill (its own children)
        // still reads end-of-file and lets go.
        drop(self.stdin.take());
        // Kill fails harmlessly if it already exited; wait() reaps either way.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
pub(crate) fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(all(test, unix))]
mod tests {
    use std::io;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn ka_r3_child_is_killed_and_reaped_on_drop() {
        let guard = ChildGuard::spawn("sleeper", "sleep", &["30".into()]).unwrap();
        let pid = guard.pid();
        assert!(alive(pid));
        assert_eq!(guard.status(), "sleeper:on");
        drop(guard);
        assert!(!alive(pid), "helper must be gone (and reaped) after drop");
    }

    #[test]
    fn ka_r3_self_exit_is_noticed_by_poll() {
        let mut guard = ChildGuard::spawn("quick", "true", &[]).unwrap();
        // Poll rather than sleep a fixed time: on a loaded machine (or with an AV scanner
        // vetting every exec) `true` can take well over 300 ms to exit.
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(50));
            guard.poll(0.0);
            if guard.status() == "quick:exited" {
                break;
            }
        }
        assert_eq!(guard.status(), "quick:exited");
    }

    #[test]
    fn ka_r3_missing_program_is_an_io_error() {
        assert!(ChildGuard::spawn("x", "/definitely/not/a/program", &[]).is_err());
        assert!(ChildGuard::spawn_piped("x", "/definitely/not/a/program", &[]).is_err());
    }

    /// The stand-in for `systemd-inhibit … cat`: a parent process (the shell) that holds on
    /// for exactly as long as its `cat` child reads the pipe (KA-R9 runs without logind).
    fn stand_in() -> io::Result<ChildGuard> {
        ChildGuard::spawn_piped("inhibit", "sh", &["-c".into(), "cat; exit 0".into()])
    }

    /// Running and not a zombie: an orphaned helper that has exited may wait a moment for
    /// init (or launchd) to reap it, and `kill -0` still succeeds on a zombie.
    fn running(pid: u32) -> bool {
        Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| {
                let stat = String::from_utf8_lossy(&o.stdout);
                o.status.success() && !stat.trim().is_empty() && !stat.trim().starts_with('Z')
            })
    }

    fn gone_within(pid: u32, secs: u64) -> bool {
        for _ in 0..secs * 20 {
            if !running(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn ka_r7_piped_helper_exits_on_end_of_file_alone() {
        let mut guard = stand_in().unwrap();
        assert_eq!(guard.status(), "inhibit:on");
        thread::sleep(Duration::from_millis(100));
        guard.poll(0.0);
        assert_eq!(
            guard.status(),
            "inhibit:on",
            "the helper waits while the pipe is open"
        );
        // Close the write end without killing anything: the helper must let go by itself.
        drop(guard.stdin.take());
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(50));
            guard.poll(0.0);
            if guard.status() == "inhibit:exited" {
                break;
            }
        }
        assert_eq!(guard.status(), "inhibit:exited");
    }

    #[test]
    fn ka_r9_piped_helper_is_gone_after_drop() {
        let guard = stand_in().unwrap();
        let pid = guard.pid();
        assert!(running(pid));
        drop(guard);
        assert!(!alive(pid), "helper must be gone (and reaped) after drop");
    }

    const HOLDER_ENV: &str = "SCIART_KA_R9_HOLDER";
    const HOLDER_TEST: &str = "child::tests::ka_r9_holder_process";

    /// Not a check by itself: re-run by the SIGKILL test below as the process that holds the
    /// guard. Without the variable it returns at once.
    #[test]
    fn ka_r9_holder_process() {
        if std::env::var_os(HOLDER_ENV).is_none() {
            return;
        }
        let guard = stand_in().unwrap();
        println!("\nhelper-pid {}", guard.pid());
        let _ = io::Write::flush(&mut io::stdout());
        thread::sleep(Duration::from_secs(60));
        drop(guard);
    }

    #[test]
    fn ka_r9_helper_does_not_outlive_a_sigkilled_holder() {
        use std::io::BufRead;

        let mut holder = Command::new(std::env::current_exe().unwrap())
            .args([HOLDER_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(HOLDER_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = holder.stdout.take().unwrap();
        let pid = io::BufReader::new(out)
            .lines()
            .map_while(Result::ok)
            // libtest may have printed `test … ... ` on the same line first.
            .find_map(|line| line.split("helper-pid ").nth(1)?.trim().parse::<u32>().ok())
            .expect("the holder reports its helper's pid");
        assert!(running(pid), "the helper runs while its holder lives");
        // SIGKILL: no destructor runs, so only the pipe can tell the helper to go.
        holder.kill().unwrap();
        holder.wait().unwrap();
        assert!(
            gone_within(pid, 5),
            "helper {pid} outlived its SIGKILLed holder"
        );
    }
}
