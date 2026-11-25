//! `sciart while` (SPEC-005 CLI-R14, ADR-0031): the command that runs beside the art, and
//! the spool that holds its output until the terminal is back.
//!
//! The art owns the screen, so the command gets no terminal: its stdin is null and its
//! stdout and stderr go to two temporary files. Each file is created under a fresh name
//! (`create_new`, so an existing file or link is never reused; mode 0600 on unix) and opened
//! a second time for reading, with its own offset. On unix the name is then removed at once:
//! the command writes through its open handle and sciart reads through its own, so the files
//! leave nothing behind on any exit, including `SIGKILL` and the RT-R20 signal exit, which
//! runs no destructors. Elsewhere the files are deleted when the spool is dropped.
//!
//! Only the standard library: the command is stopped with `Child::kill` (`SIGKILL` on unix),
//! so it cannot clean up, and a process it started itself is not stopped with it (ADR-0031).

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{self, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Exit status when the `--for` deadline stopped the command (CLI-R14, as `timeout` does).
pub const EXIT_DEADLINE: u8 = 124;
/// Exit status when the command could not be started (CLI-R14, as a shell does).
pub const EXIT_NOT_STARTED: u8 = 127;
/// Exit status when `Ctrl-C` stopped the command (CLI-R14: 128 + SIGINT).
pub const EXIT_INTERRUPTED: u8 = 130;

/// How often the copy after `q` looks for new output and for the command's exit.
const FOLLOW_POLL: Duration = Duration::from_millis(50);

/// Names tried per stream before giving up; only other files with sciart's names, left by
/// a process with the same pid, can take them.
const NAME_TRIES: u32 = 1000;

/// Why the command did not start.
#[derive(Debug)]
pub enum StartError {
    /// A spool file could not be created.
    Spool(io::Error),
    /// The command could not be started (not found, not executable, ...).
    Command { program: OsString, error: io::Error },
}

impl StartError {
    /// The exit status: 127 for a command that cannot start (CLI-R14), 1 for sciart's own
    /// failure to make a spool.
    pub fn code(&self) -> u8 {
        match self {
            StartError::Spool(_) => 1,
            StartError::Command { .. } => EXIT_NOT_STARTED,
        }
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Spool(error) => {
                write!(f, "cannot create a file for the command's output: {error}")
            }
            StartError::Command { program, error } => {
                write!(f, "cannot run '{}': {error}", program.to_string_lossy())
            }
        }
    }
}

/// How [`Child::follow`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// The command exited with this status.
    Exited(ExitStatus),
    /// The deadline passed first; the command is still running.
    Deadline,
}

/// The read side of one stream's spool file.
#[derive(Debug)]
struct Spool {
    reader: File,
    /// The file's name while it still has one (not on unix): deleted on drop.
    path: Option<PathBuf>,
}

impl Drop for Spool {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Create one spool file in `dir`: the handle the command writes to, and sciart's reader.
fn spool(dir: &Path, stream: &str) -> io::Result<(File, Spool)> {
    for n in 0..NAME_TRIES {
        let path = dir.join(format!("sciart-while-{}-{n}.{stream}", process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let writer = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let reader = File::open(&path);
        // On unix the two open handles are all either side needs, so the name goes now.
        let unlinked = cfg!(unix) && fs::remove_file(&path).is_ok();
        let spool = |reader| Spool {
            reader,
            path: (!unlinked).then(|| path.clone()),
        };
        return match reader {
            Ok(reader) => Ok((writer, spool(reader))),
            Err(e) => {
                if !unlinked {
                    let _ = fs::remove_file(&path);
                }
                Err(e)
            }
        };
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free name for a spool file in {}", dir.display()),
    ))
}

/// The running command and its spooled output.
#[derive(Debug)]
pub struct Child {
    process: process::Child,
    out: Spool,
    err: Spool,
    /// Kept once the command has been reaped.
    status: Option<ExitStatus>,
}

impl Child {
    /// Start `argv` (program, then arguments) with stdin null and its output spooled to
    /// files in `dir` (CLI-R14).
    pub fn start(argv: &[OsString], dir: &Path) -> Result<Self, StartError> {
        let Some((program, args)) = argv.split_first() else {
            return Err(StartError::Command {
                program: OsString::new(),
                error: io::Error::new(io::ErrorKind::InvalidInput, "empty command line"),
            });
        };
        let (out_writer, out) = spool(dir, "out").map_err(StartError::Spool)?;
        let (err_writer, err) = spool(dir, "err").map_err(StartError::Spool)?;
        // The `Command` and with it sciart's copies of the write handles are dropped at the
        // end of this statement; only the command keeps them.
        let process = process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(out_writer)
            .stderr(err_writer)
            .spawn()
            .map_err(|error| StartError::Command {
                program: program.clone(),
                error,
            })?;
        Ok(Self {
            process,
            out,
            err,
            status: None,
        })
    }

    /// The command's exit status if it has exited, without waiting.
    pub fn try_status(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.process.try_wait()?;
        }
        Ok(self.status)
    }

    /// Kill the command if it still runs, and reap it: its status either way.
    pub fn stop(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.try_status()? {
            return Ok(status);
        }
        // It may exit between the check and the kill; `wait` reaps it either way.
        let _ = self.process.kill();
        let status = self.process.wait()?;
        self.status = Some(status);
        Ok(status)
    }

    /// Copy what the command has written since the last call: its stdout to `out`, then its
    /// stderr to `err` (CLI-R14).
    pub fn replay(&mut self, out: &mut impl Write, err: &mut impl Write) -> io::Result<()> {
        io::copy(&mut self.out.reader, out)?;
        out.flush()?;
        io::copy(&mut self.err.reader, err)?;
        err.flush()
    }

    /// After `q`: keep copying as the command writes, until it exits or `deadline` passes.
    /// `idle` is called with every pause, so keep-awake can be polled meanwhile.
    pub fn follow(
        &mut self,
        out: &mut impl Write,
        err: &mut impl Write,
        deadline: Option<Instant>,
        mut idle: impl FnMut(Duration),
    ) -> io::Result<Followed> {
        loop {
            self.replay(out, err)?;
            if let Some(status) = self.try_status()? {
                // What it wrote between that copy and its exit.
                self.replay(out, err)?;
                return Ok(Followed::Exited(status));
            }
            if deadline.is_some_and(|deadline| crate::app::wall_clock() >= deadline) {
                return Ok(Followed::Deadline);
            }
            thread::sleep(FOLLOW_POLL);
            idle(FOLLOW_POLL);
        }
    }
}

impl Drop for Child {
    /// An early return must not leave the command running.
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// The status sciart exits with for the command's `status` (CLI-R14): its exit code, or
/// `128 + signal` when a signal ended it.
pub fn exit_code(status: ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return u8::try_from(code).unwrap_or(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return u8::try_from(128 + signal).unwrap_or(u8::MAX);
        }
    }
    1
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Vec<OsString> {
        ["sh", "-c", script].map(OsString::from).to_vec()
    }

    fn start(script: &str, dir: &Path) -> Child {
        Child::start(&sh(script), dir).expect("start")
    }

    /// Files in `dir`, as names.
    fn files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn wait(child: &mut Child) -> ExitStatus {
        loop {
            if let Some(status) = child.try_status().unwrap() {
                return status;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cli_r14_exit_status_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        for (script, code) in [("exit 0", 0), ("exit 3", 3), ("exit 255", 255)] {
            let mut child = start(script, dir.path());
            assert_eq!(exit_code(wait(&mut child)), code, "{script}");
        }
    }

    #[test]
    fn cli_r14_a_signal_is_128_plus_its_number() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = start("kill -TERM $$", dir.path());
        assert_eq!(exit_code(wait(&mut child)), 143);
    }

    #[test]
    fn cli_r14_output_is_spooled_and_replayed_stdout_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = start(
            "echo err1 >&2; echo out1; echo err2 >&2; echo out2",
            dir.path(),
        );
        wait(&mut child);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        child.replay(&mut out, &mut err).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "out1\nout2\n");
        assert_eq!(String::from_utf8(err).unwrap(), "err1\nerr2\n");
        // A second replay copies only what is new: nothing.
        let (mut out, mut err) = (Vec::new(), Vec::new());
        child.replay(&mut out, &mut err).unwrap();
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn cli_r14_the_command_gets_no_terminal_input() {
        let dir = tempfile::tempdir().unwrap();
        // `read` fails at once on a null stdin instead of waiting for a key.
        let mut child = start("if read line; then echo got; else echo eof; fi", dir.path());
        assert_eq!(exit_code(wait(&mut child)), 0);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        child.replay(&mut out, &mut err).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "eof\n");
    }

    #[test]
    fn cli_r14_spool_files_leave_no_name_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = start("echo hello; sleep 0.2", dir.path());
        assert_eq!(files(dir.path()), Vec::<String>::new(), "while running");
        wait(&mut child);
        drop(child);
        assert_eq!(files(dir.path()), Vec::<String>::new(), "after the end");
    }

    #[test]
    fn cli_r14_an_existing_file_is_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let taken = dir
            .path()
            .join(format!("sciart-while-{}-0.out", process::id()));
        fs::write(&taken, "someone else's").unwrap();
        let mut child = start("echo mine", dir.path());
        wait(&mut child);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        child.replay(&mut out, &mut err).unwrap();
        assert_eq!(out, b"mine\n");
        assert_eq!(fs::read_to_string(&taken).unwrap(), "someone else's");
        drop(child);
        assert_eq!(
            files(dir.path()),
            [taken.file_name().unwrap().to_string_lossy()]
        );
    }

    #[test]
    fn cli_r14_a_command_that_cannot_start_is_127_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let argv = [OsString::from("/definitely/not/a/command")];
        let err = Child::start(&argv, dir.path()).unwrap_err();
        assert_eq!(err.code(), EXIT_NOT_STARTED);
        let text = err.to_string();
        assert!(
            text.starts_with("cannot run '/definitely/not/a/command': "),
            "{text}"
        );
        assert_eq!(text.lines().count(), 1, "{text}");
        assert_eq!(files(dir.path()), Vec::<String>::new());
        let err = Child::start(&[], dir.path()).unwrap_err();
        assert_eq!(err.code(), EXIT_NOT_STARTED);
    }

    #[test]
    fn cli_r14_a_spool_that_cannot_be_made_is_exit_1() {
        let missing = Path::new("/definitely/not/a/directory");
        let err = Child::start(&sh("true"), missing).unwrap_err();
        assert!(matches!(err, StartError::Spool(_)), "{err:?}");
        assert_eq!(err.code(), 1);
    }

    /// Whether `pid` is still a live process (not a zombie).
    fn alive(pid: u32) -> bool {
        process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                let stat = String::from_utf8_lossy(&o.stdout);
                o.status.success() && !stat.trim().is_empty() && !stat.trim().starts_with('Z')
            })
    }

    #[test]
    fn cli_r14_stop_kills_and_reaps_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = start("exec sleep 30", dir.path());
        let pid = child.process.id();
        assert!(alive(pid));
        let status = child.stop().unwrap();
        assert_eq!(exit_code(status), 128 + 9, "SIGKILL");
        assert!(!alive(pid), "reaped, not left running");
        assert_eq!(child.stop().unwrap(), status, "stopping again is a no-op");
    }

    #[test]
    fn cli_r14_dropping_the_child_stops_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let child = start("exec sleep 30", dir.path());
        let pid = child.process.id();
        drop(child);
        assert!(!alive(pid));
    }

    #[test]
    fn cli_r14_follow_copies_until_the_command_exits() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = start("echo a; sleep 0.3; echo b >&2; echo c; exit 4", dir.path());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut pauses = 0;
        let followed = child
            .follow(&mut out, &mut err, None, |_| pauses += 1)
            .unwrap();
        let Followed::Exited(status) = followed else {
            panic!("{followed:?}");
        };
        assert_eq!(exit_code(status), 4);
        assert_eq!(String::from_utf8(out).unwrap(), "a\nc\n");
        assert_eq!(String::from_utf8(err).unwrap(), "b\n");
        assert!(pauses > 0, "keep-awake was polled while waiting");
    }

    #[test]
    fn cli_r14_follow_gives_up_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = start("exec sleep 30", dir.path());
        let soon = crate::app::wall_clock() + Duration::from_millis(100);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let followed = child
            .follow(&mut out, &mut err, Some(soon), |_| {})
            .unwrap();
        assert_eq!(followed, Followed::Deadline);
        assert!(child.try_status().unwrap().is_none(), "still running");
        child.stop().unwrap();
    }
}
