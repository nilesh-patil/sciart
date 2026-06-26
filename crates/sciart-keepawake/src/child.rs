//! A helper process whose lifetime is tied to a guard (KA-R3).

use std::io;
use std::process::{Child, Command, Stdio};

use crate::Backend;

#[derive(Debug)]
pub struct ChildGuard {
    label: &'static str,
    child: Child,
    exited: bool,
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
    }
}
