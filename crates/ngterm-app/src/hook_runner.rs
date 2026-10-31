//! The app's [`ProcessLauncher`]: hooks and custom commands as detached children (upstream's `HookProcessRunner.swift`
//! and the spawn half of `CustomCommandRunner.swift`); the core hands it the program, the shell of
//! `ngterm_host::command_line` (`/bin/sh -c` on Unix, `cmd.exe /D /S /C` with the quoted line as the command line's raw
//! tail on Windows).
//!
//! stdout goes to the null device; stdin is the bytes the spec carries (written on a thread of its own, closed after,
//! a child leaving without reading them is no failure) or the null device; stderr is the null device or, when
//! captured, an owner-only temp file that is gone from the directory once the run ends, rather than a pipe, so a
//! background descendant the command started keeps running after the app quits instead of dying of a broken pipe. A
//! waiter thread reaps the child, stops a stdin write still pending (a descendant may hold the pipe without ever
//! reading it; upstream closes the channel with `.stop` on exit), and reports its [`ProcessExit`], so no input
//! outlives the run it belongs to. No run has a timeout and no output cap but the stderr tail's, as upstream's, and
//! the child's descendants are never ended with it: a command may start a background job on purpose.
//!
//! The OS forms ([`platform`]):
//!
//! - Unix: the child leads a process group of its own (a terminal's ^C aimed at the app does not reach it); the stdin
//!   write is non-blocking and waits in `poll` together with a stop pipe; the capture is created `0600` and unlinked
//!   as soon as it is open; a signal that ended the child is its status.
//! - Windows: the child starts in a process group of its own (`CREATE_NEW_PROCESS_GROUP`: no console Ctrl+C reaches
//!   it) and without a console window (`CREATE_NO_WINDOW`: a GUI app's child would otherwise flash one); the stdin
//!   write blocks on its thread and a stop cancels it (`CancelIoEx` on the pipe, `CancelSynchronousIo` on the
//!   thread); the capture is opened unshared (no other process can open it, the per-user temp folder's ACL aside)
//!   and delete-on-close, so the system removes it once the child, its descendants and the app have all closed it;
//!   the exit code is the status (there are no signals).

use std::fs::File;
use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;

use ngterm_host::{ExitHandler, LaunchSpec, LaunchedProcess, ProcessExit, ProcessLauncher, STDERR_TAIL_LIMIT};

use platform::{PendingStdin, StdinWriter};

/// Starts processes on the host.
#[derive(Debug, Default)]
pub struct SystemLauncher;

impl ProcessLauncher for SystemLauncher {
    fn launch(&self, spec: &LaunchSpec, on_exit: ExitHandler) -> io::Result<LaunchedProcess> {
        let capture = if spec.capture_stderr {
            StderrFile::create()
        } else {
            None
        };
        // made before the spawn, so nothing can fail between the spawn and the reaper taking the child
        let pending = spec.stdin.as_ref().map(|_| PendingStdin::new()).transpose()?;
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .stdout(Stdio::null())
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(match capture.as_ref().map(StderrFile::writer) {
                Some(Ok(file)) => Stdio::from(file),
                _ => Stdio::null(),
            });
        if let Some(tail) = &spec.raw_tail {
            platform::raw_arg(&mut command, tail);
        }
        platform::detach(&mut command);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn()?;
        let pid = child.id();
        let writer = child
            .stdin
            .take()
            .zip(spec.stdin.clone())
            .zip(pending)
            .map(|((stdin, bytes), pending)| pending.start(stdin, bytes));
        thread::Builder::new()
            .name(format!("ngterm-reap-{pid}"))
            .spawn(move || reap(child, writer, capture, on_exit))?;
        Ok(LaunchedProcess { pid })
    }
}

/// Waits for the child, then stops and joins its stdin write, then reads the stderr tail and reports.
fn reap(mut child: Child, writer: Option<StdinWriter>, capture: Option<StderrFile>, on_exit: ExitHandler) {
    let status = child.wait().map_or(-1, status_code);
    let stdin_failure = writer.and_then(StdinWriter::finish);
    let stderr_tail = capture.map(StderrFile::consume).unwrap_or_default();
    on_exit(ProcessExit {
        status,
        stderr_tail,
        stdin_failure,
    });
}

/// The exit code, or on Unix the signal that ended the process (Foundation's `terminationStatus` for an uncaught
/// signal).
fn status_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    status.code().or(signal).unwrap_or(-1)
}

/// The file the child's stderr goes to, under a fresh name in the temp directory, where only this user can open it
/// and nothing is left behind ([`platform::open_capture`]); the app reads the tail through its own handle.
struct StderrFile {
    file: File,
    /// Where it was created, for the test that checks nothing else can open it.
    #[cfg(all(test, windows))]
    path: std::path::PathBuf,
}

impl StderrFile {
    /// `None` when the file cannot be created; stderr then goes to the null device.
    fn create() -> Option<Self> {
        let path = std::env::temp_dir().join(format!("ngterm-command-{}.err", uuid::Uuid::new_v4()));
        match platform::open_capture(&path) {
            Ok(file) => Some(Self {
                file,
                #[cfg(all(test, windows))]
                path,
            }),
            Err(err) => {
                tracing::warn!(%err, "stderr capture unavailable");
                None
            }
        }
    }

    /// The child's handle: the same open file, appending.
    fn writer(&self) -> io::Result<File> {
        self.file.try_clone()
    }

    /// The last [`STDERR_TAIL_LIMIT`] bytes written so far. The cap is on the read: a descendant still holding the
    /// file keeps growing it until it exits.
    fn consume(self) -> Vec<u8> {
        read_tail(&self.file).unwrap_or_default()
    }
}

fn read_tail(file: &File) -> io::Result<Vec<u8>> {
    let size = file.metadata()?.len();
    let start = size.saturating_sub(STDERR_TAIL_LIMIT as u64);
    let mut tail = Vec::new();
    let mut chunk = [0; 4096];
    let mut offset = start;
    while offset < size {
        match platform::read_at(file, &mut chunk, offset) {
            Ok(0) => break,
            Ok(read) => {
                let wanted = usize::try_from(size - offset).map_or(read, |left| read.min(left));
                tail.extend_from_slice(&chunk[..wanted]);
                offset += wanted as u64;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(tail)
}

#[cfg(unix)]
mod platform {
    use std::fs::{self, File};
    use std::io::{self, PipeReader, PipeWriter, Write};
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
    use std::os::unix::fs::{FileExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{ChildStdin, Command};
    use std::thread::{self, JoinHandle};

    /// Its own process group: a terminal's ^C aimed at the app does not reach it.
    pub(super) fn detach(command: &mut Command) {
        command.process_group(0);
    }

    /// One more argument: Unix has no command line to append to.
    pub(super) fn raw_arg(command: &mut Command, tail: &str) {
        command.arg(tail);
    }

    /// Created owner-only (0600) under its fresh name and unlinked at once, so no other user can open it and nothing
    /// is left behind.
    pub(super) fn open_capture(path: &Path) -> io::Result<File> {
        let file = fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        let _ = fs::remove_file(path);
        Ok(file)
    }

    pub(super) fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        file.read_at(buf, offset)
    }

    /// The pipe that will stop a stdin write, made before the spawn.
    pub(super) struct PendingStdin {
        stop_reader: PipeReader,
        stop: PipeWriter,
    }

    impl PendingStdin {
        pub(super) fn new() -> io::Result<Self> {
            let (stop_reader, stop) = io::pipe()?;
            Ok(Self { stop_reader, stop })
        }

        /// Writes `bytes` to `stdin` on a thread of its own, then closes it.
        pub(super) fn start(self, stdin: ChildStdin, bytes: Vec<u8>) -> StdinWriter {
            let stop_reader = self.stop_reader;
            let thread = thread::spawn(move || write_until_stopped(stdin, &bytes, &stop_reader));
            StdinWriter {
                thread,
                stop: self.stop,
            }
        }
    }

    /// The thread writing a child's stdin, and the pipe that stops it.
    pub(super) struct StdinWriter {
        thread: JoinHandle<Option<String>>,
        /// Dropped to stop the write: the thread's end of the pipe then polls as hung up.
        stop: PipeWriter,
    }

    impl StdinWriter {
        /// The child exited: a write still pending is stopped (only a descendant could still read it, and upstream
        /// drops it too), then why the delivery failed, a broken pipe or the stop aside.
        pub(super) fn finish(self) -> Option<String> {
            drop(self.stop);
            self.thread
                .join()
                .unwrap_or_else(|_| Some("the stdin writer panicked".to_owned()))
        }
    }

    /// Writes `bytes` to `stdin` without blocking past a stop: the pipe is non-blocking, and a full one is waited on
    /// together with `stop`. Why it failed, a broken pipe (the reader left) or a stop aside.
    fn write_until_stopped(mut stdin: ChildStdin, bytes: &[u8], stop: &PipeReader) -> Option<String> {
        if let Err(err) = set_nonblocking(stdin.as_fd()) {
            return Some(err.to_string());
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            match stdin.write(rest) {
                Ok(0) => return Some(io::Error::from(io::ErrorKind::WriteZero).to_string()),
                Ok(written) => rest = &rest[written..],
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    match wait_writable(stdin.as_fd(), stop.as_fd()) {
                        Ok(true) => {}
                        Ok(false) => return None,
                        Err(err) => return Some(err.to_string()),
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return None,
                Err(err) => return Some(err.to_string()),
            }
        }
        None
    }

    /// Puts the app's end of the stdin pipe in non-blocking mode (the child's end is a separate open file, untouched).
    fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
        // SAFETY: `fd` is an open descriptor for the borrow's lifetime; F_GETFL only reads its status flags.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above; F_SETFL only changes the status flags of this open file.
        let set = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if set < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Waits until `pipe` takes more bytes, or its reader left (`true`: the next write tells which), or `stop` is
    /// written to or closed (`false`).
    fn wait_writable(pipe: BorrowedFd<'_>, stop: BorrowedFd<'_>) -> io::Result<bool> {
        let mut fds = [
            libc::pollfd {
                fd: pipe.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            },
            libc::pollfd {
                fd: stop.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        loop {
            // SAFETY: `fds` is a live array of two initialized pollfd entries whose descriptors stay open for the
            // call.
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if ready >= 0 {
                return Ok(fds[1].revents == 0);
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::os::windows::fs::{FileExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{ChildStdin, Command};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, PoisonError, RwLock};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_DELETE_ON_CLOSE};
    use windows_sys::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

    /// How long a stop waits before cancelling again a write its first cancel may have come before.
    const CANCEL_RETRY: Duration = Duration::from_millis(5);

    /// A process group of its own, so no console Ctrl+C reaches it, and no console window.
    pub(super) fn detach(command: &mut Command) {
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    /// Appended to the command line as written (`cmd`'s quoted command text), past the C runtime's quoting.
    pub(super) fn raw_arg(command: &mut Command, tail: &str) {
        command.raw_arg(tail);
    }

    /// Opened unshared, so no other process can open it while the app holds it, and delete-on-close: the system
    /// removes it once its last handle closes (the app's, the child's and any descendant's), so nothing is left
    /// behind. It lives in the per-user temp folder, whose ACL already keeps other users out.
    pub(super) fn open_capture(path: &Path) -> io::Result<File> {
        fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .attributes(FILE_ATTRIBUTE_TEMPORARY)
            .open(path)
    }

    pub(super) fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        file.seek_read(buf, offset)
    }

    /// Nothing to make before the spawn: the stop is a flag and a cancel.
    pub(super) struct PendingStdin;

    impl PendingStdin {
        #[expect(
            clippy::unnecessary_wraps,
            reason = "the Unix form's pipe can fail; both forms share the launcher's call"
        )]
        pub(super) fn new() -> io::Result<Self> {
            Ok(Self)
        }

        /// Writes `bytes` to `stdin` on a thread of its own, then closes it.
        #[expect(
            clippy::unused_self,
            reason = "the Unix form consumes its stop pipe here; both forms share the launcher's call"
        )]
        pub(super) fn start(self, stdin: ChildStdin, bytes: Vec<u8>) -> StdinWriter {
            let stdin = Arc::new(RwLock::new(Some(stdin)));
            let stopped = Arc::new(AtomicBool::new(false));
            let (pipe, stop) = (Arc::clone(&stdin), Arc::clone(&stopped));
            let thread = thread::spawn(move || {
                let failure = pipe
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    .and_then(|stdin| write_until_stopped(stdin, &bytes, &stop));
                // closed once written, so the child reads its end; a stop's cancel still holding the lock is waited for
                pipe.write().unwrap_or_else(PoisonError::into_inner).take();
                failure
            });
            StdinWriter { thread, stdin, stopped }
        }
    }

    /// The stdin pipe, shared between its writer and a stop's cancel, and closed (taken) by the writer once it is
    /// done; a cancel holds it open for the call, so it never names a closed handle.
    type SharedStdin = Arc<RwLock<Option<ChildStdin>>>;

    /// The thread writing a child's stdin, the pipe it writes and the flag that stops it.
    pub(super) struct StdinWriter {
        thread: JoinHandle<Option<String>>,
        stdin: SharedStdin,
        stopped: Arc<AtomicBool>,
    }

    impl StdinWriter {
        /// The child exited: a write still pending is cancelled (only a descendant could still read it, and upstream
        /// drops it too), then why the delivery failed, a broken pipe or the stop aside. A write the thread was about
        /// to start when the first cancel came sees the flag, or the next cancel.
        pub(super) fn finish(self) -> Option<String> {
            self.stopped.store(true, Ordering::SeqCst);
            while !self.thread.is_finished() {
                if let Some(stdin) = self.stdin.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
                    // SAFETY: the stdin pipe, which the read guard keeps open for the call; a null `OVERLAPPED`
                    // cancels every request of this process on it, and "nothing to cancel" is a harmless failure.
                    unsafe { CancelIoEx(stdin.as_raw_handle(), std::ptr::null()) };
                }
                // SAFETY: the writer thread, whose handle the JoinHandle keeps open; it does no other I/O than this
                // write, and "nothing to cancel" is a harmless failure.
                unsafe { CancelSynchronousIo(self.thread.as_raw_handle()) };
                thread::sleep(CANCEL_RETRY);
            }
            self.thread
                .join()
                .unwrap_or_else(|_| Some("the stdin writer panicked".to_owned()))
        }
    }

    /// Writes `bytes` to `stdin` until done or stopped. Why it failed, a broken pipe (the reader left) or a stop
    /// aside.
    fn write_until_stopped(stdin: &ChildStdin, bytes: &[u8], stopped: &AtomicBool) -> Option<String> {
        let mut pipe = stdin;
        let mut rest = bytes;
        while !rest.is_empty() {
            if stopped.load(Ordering::SeqCst) {
                return None;
            }
            match pipe.write(rest) {
                Ok(0) => return Some(io::Error::from(io::ErrorKind::WriteZero).to_string()),
                Ok(written) => rest = &rest[written..],
                // the stop's cancel
                Err(_) if stopped.load(Ordering::SeqCst) => return None,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return None,
                Err(err) => return Some(err.to_string()),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;

    use super::*;
    use crate::test_shell::{CHILD_DEADLINE, script_argv};

    fn run(spec: &LaunchSpec) -> ProcessExit {
        let (tx, rx) = mpsc::channel();
        SystemLauncher
            .launch(spec, Box::new(move |exit| tx.send(exit).unwrap()))
            .unwrap();
        rx.recv_timeout(CHILD_DEADLINE).unwrap()
    }

    /// `unix` under `/bin/sh -c` on Unix, `windows` under PowerShell on Windows.
    fn script(unix: &str, windows: &str) -> LaunchSpec {
        let mut argv = script_argv(unix, windows);
        LaunchSpec {
            program: argv.remove(0).into(),
            args: argv,
            ..LaunchSpec::default()
        }
    }

    #[test]
    fn reports_the_exit_status_and_the_stderr_tail() {
        let exit = run(&LaunchSpec {
            capture_stderr: true,
            ..script("echo oops >&2; exit 3", "[Console]::Error.Write(\"oops`n\"); exit 3")
        });
        assert_eq!(exit.status, 3);
        assert_eq!(exit.stderr_tail, b"oops\n");
        assert_eq!(exit.stdin_failure, None);
    }

    #[test]
    fn stdin_env_and_cwd_reach_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let exit = run(&LaunchSpec {
            env: vec![("AGT_X".to_owned(), "x".to_owned())],
            cwd: Some(dir.path().to_owned()),
            stdin: Some(b"payload\n".to_vec()),
            ..script(
                "read line; printf '%s|%s|%s' \"$line\" \"$AGT_X\" \"$(basename \"$PWD\")\" > out",
                "$line = [Console]::In.ReadLine()\n\
                 $name = Split-Path -Leaf (Get-Location).Path\n\
                 [IO.File]::WriteAllText((Join-Path (Get-Location).Path 'out'), \"$line|$env:AGT_X|$name\")",
            )
        });
        assert_eq!(exit.status, 0);
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(fs::read_to_string(out).unwrap(), format!("payload|x|{name}"));
    }

    #[test]
    fn a_child_reading_stdin_to_its_end_sees_the_end() {
        // the pipe closes once written, so a child reading all of it is not left waiting
        let exit = run(&LaunchSpec {
            stdin: Some(b"a\nb\n".to_vec()),
            capture_stderr: true,
            ..script("wc -l >&2", "[Console]::Error.Write([Console]::In.ReadToEnd().Length)")
        });
        assert_eq!(exit.status, 0);
        let counted = String::from_utf8(exit.stderr_tail).unwrap();
        assert_eq!(counted.trim(), if cfg!(windows) { "4" } else { "2" });
    }

    #[test]
    fn a_child_leaving_without_reading_is_no_delivery_failure() {
        let exit = run(&LaunchSpec {
            stdin: Some(vec![b'x'; 1 << 20]),
            ..script("exit 0", "exit 0")
        });
        assert_eq!((exit.status, exit.stdin_failure), (0, None));
    }

    #[cfg(unix)]
    #[test]
    fn a_descendant_holding_stdin_unread_does_not_hold_back_the_exit() {
        // upstream's testAGrandchildHoldingStdinDoesNotDelayExitOrCleanup: more than a pipe buffer of input, and a
        // background descendant that keeps the pipe open without reading it, living past the deadline
        let (tx, rx) = mpsc::channel();
        let launched = SystemLauncher
            .launch(
                &LaunchSpec {
                    stdin: Some(vec![b'x'; 300_000]),
                    ..script("exec 3<&0; (sleep 300 <&3 3<&-) & exec 3<&-; exit 0", "")
                },
                Box::new(move |exit| tx.send(exit).unwrap()),
            )
            .unwrap();
        let exit = rx.recv_timeout(CHILD_DEADLINE);
        // the descendant is in the child's process group: the kill finding it shows it still ran when the exit came
        let killed = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", launched.pid)])
            .status()
            .is_ok_and(|status| status.success());
        let exit = exit.expect("the exit is reported while the descendant still holds stdin");
        assert!(killed, "the exit came only once the descendant had ended");
        assert_eq!((exit.status, exit.stdin_failure), (0, None));
    }

    /// The Windows twin: a descendant inherits the stdin pipe and keeps it open unread, living past the deadline; the
    /// cancel of the pending write lets the exit through. The descendant's pid comes back in a file, for the cleanup,
    /// whose kill finding it shows it still ran when the exit came.
    #[cfg(windows)]
    #[test]
    fn a_descendant_holding_stdin_unread_does_not_hold_back_the_exit() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let (tx, rx) = mpsc::channel();
        SystemLauncher
            .launch(
                &LaunchSpec {
                    stdin: Some(vec![b'x'; 300_000]),
                    env: vec![("PID_FILE".to_owned(), pid_file.to_string_lossy().into_owned())],
                    ..script(
                        "",
                        "$start = New-Object Diagnostics.ProcessStartInfo 'ping.exe', '-n 300 127.0.0.1'\n\
                         $start.UseShellExecute = $false\n\
                         $descendant = [Diagnostics.Process]::Start($start)\n\
                         [IO.File]::WriteAllText($env:PID_FILE, $descendant.Id)\n\
                         exit 0",
                    )
                },
                Box::new(move |exit| tx.send(exit).unwrap()),
            )
            .unwrap();
        let exit = rx.recv_timeout(CHILD_DEADLINE);
        let killed = fs::read_to_string(&pid_file).is_ok_and(|pid| {
            Command::new("taskkill")
                .args(["/F", "/PID", pid.trim()])
                .output()
                .is_ok_and(|output| output.status.success())
        });
        let exit = exit.expect("the exit is reported while the descendant still holds stdin");
        assert!(killed, "the exit came only once the descendant had ended");
        assert_eq!((exit.status, exit.stdin_failure), (0, None));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_stderr_capture_is_owner_only_and_already_unlinked() {
        let exit = run(&LaunchSpec {
            capture_stderr: true,
            ..script("stat -L -c %a /proc/$$/fd/2 >&2; readlink /proc/$$/fd/2 >&2", "")
        });
        let text = String::from_utf8(exit.stderr_tail).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("600"));
        assert!(lines.next().is_some_and(|link| link.ends_with(" (deleted)")), "{text}");
    }

    /// The Windows twin: while open the capture admits no other opener, and once its handles close it is gone.
    #[cfg(windows)]
    #[test]
    fn the_stderr_capture_is_unshared_and_gone_once_closed() {
        let capture = StderrFile::create().unwrap();
        let path = capture.path.clone();
        assert!(path.exists());
        let other = fs::OpenOptions::new().read(true).open(&path);
        assert!(other.is_err(), "another opener is refused");
        let writer = capture.writer().unwrap();
        std::io::Write::write_all(&mut &writer, b"tail").unwrap();
        drop(writer);
        assert_eq!(capture.consume(), b"tail");
        assert!(!path.exists(), "deleted on close");
    }

    #[test]
    fn only_the_stderr_tail_comes_back() {
        let exit = run(&LaunchSpec {
            capture_stderr: true,
            ..script(
                &format!("head -c {} /dev/zero >&2; printf end >&2", STDERR_TAIL_LIMIT * 2),
                &format!("[Console]::Error.Write(('x' * {}) + 'end')", STDERR_TAIL_LIMIT * 2),
            )
        });
        assert_eq!(exit.stderr_tail.len(), STDERR_TAIL_LIMIT);
        assert!(exit.stderr_tail.ends_with(b"end"));
    }

    /// `unix` on Unix, `windows` on Windows, as the user's command line the core hands over
    /// ([`LaunchSpec::command_line`]: `/bin/sh -c`, or `cmd.exe /D /S /C` with the quoted line as written).
    fn command_line(unix: &str, windows: &str) -> LaunchSpec {
        LaunchSpec::command_line(if cfg!(windows) { windows } else { unix })
    }

    #[test]
    fn a_hook_line_reads_its_event_on_stdin_and_writes_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display();
        let exit = run(&LaunchSpec {
            stdin: Some(b"{\"kind\":\"status\"}\n".to_vec()),
            env: vec![("AGT_EVENT_KIND".to_owned(), "status".to_owned())],
            // `set /p` without input exits 1: the line ends on `more`, whose status is the line's
            ..command_line(
                &format!("printf '%s' \"$AGT_EVENT_KIND\" > '{root}/kind'; cat > '{root}/event.json'"),
                &format!(r#">"{root}\kind" <nul set /p "=%AGT_EVENT_KIND%" & more > "{root}\event.json""#),
            )
        });
        assert_eq!((exit.status, exit.stdin_failure), (0, None));
        let event = fs::read_to_string(dir.path().join("event.json")).unwrap();
        assert_eq!(event.trim_end(), "{\"kind\":\"status\"}");
        assert_eq!(fs::read_to_string(dir.path().join("kind")).unwrap(), "status");
    }

    #[test]
    fn a_command_line_keeps_its_quotes_and_spaces() {
        // a quoted program path with a space, starting the line, and a quoted argument with one
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("with space");
        fs::create_dir(&folder).unwrap();
        let tool = folder.join(if cfg!(windows) { "copy tool.cmd" } else { "copy tool.sh" });
        let script = if cfg!(windows) {
            "@>\"%~dp0out file.txt\" <nul set /p \"=%~1|%~2\"\r\n"
        } else {
            "#!/bin/sh\nprintf '%s|%s' \"$1\" \"$2\" > \"$(dirname \"$0\")/out file.txt\"\n"
        };
        fs::write(&tool, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let line = format!("\"{}\" \"a b\" c", tool.display());
        let exit = run(&LaunchSpec::command_line(&line));
        // `set /p` without input exits 1, the Windows script's last status
        assert_eq!(exit.status, i32::from(cfg!(windows)));
        assert_eq!(fs::read_to_string(folder.join("out file.txt")).unwrap(), "a b|c");
    }

    #[test]
    fn a_failing_command_line_reports_its_status_and_stderr() {
        let exit = run(&LaunchSpec {
            capture_stderr: true,
            ..command_line("echo oops >&2; exit 7", ">&2 echo oops & exit 7")
        });
        assert_eq!(exit.status, 7);
        assert_eq!(String::from_utf8(exit.stderr_tail).unwrap().trim_end(), "oops");
    }

    #[test]
    fn a_missing_program_fails_to_launch() {
        let spec = LaunchSpec {
            program: "/nonexistent/ngterm-test".into(),
            capture_stderr: true,
            ..LaunchSpec::default()
        };
        assert!(SystemLauncher.launch(&spec, Box::new(|_| ())).is_err());
    }
}
