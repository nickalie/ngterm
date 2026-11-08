//! The outer terminal on Unix: termios' raw mode on standard input (zmx's `cfmakeraw`, restored with `TCSAFLUSH`),
//! the window size (`TIOCGWINSZ`, pixels included where the terminal sets them), a reader thread on standard input and
//! a `SIGWINCH` watcher.
//!
//! herdr: `src/client/terminal_setup.rs` and `src/platform/mod.rs` at `HERDR_REV` (`build/pins.env`), licensed under
//! the Apache License 2.0, modified as the parent module's docs say.

use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::sync::mpsc::Sender;
use std::thread;

use rustix::termios::{OptionalActions, Termios, tcgetattr, tcgetwinsize, tcsetattr};
use signal_hook::consts::SIGWINCH;
use signal_hook::iterator::Signals;

use super::{TtyEvent, TtySize};
use crate::protocol::GridSize;

/// The size of one read of the input.
const READ_SIZE: usize = 16 * 1024;

pub(super) fn is_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Standard input in raw mode; dropped, its mode is restored and its unread input discarded.
#[derive(Debug)]
pub(super) struct RawMode {
    saved: Termios,
}

impl RawMode {
    pub(super) fn enter() -> io::Result<Self> {
        let stdin = io::stdin();
        let saved = tcgetattr(&stdin)?;
        let mut raw = saved.clone();
        raw.make_raw();
        tcsetattr(&stdin, OptionalActions::Now, &raw)?;
        Ok(Self { saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = tcsetattr(io::stdin(), OptionalActions::Flush, &self.saved);
    }
}

pub(super) fn size() -> Option<TtySize> {
    let size = tcgetwinsize(io::stdout()).or_else(|_| tcgetwinsize(io::stdin())).ok()?;
    let pixels =
        (size.ws_xpixel > 0 && size.ws_ypixel > 0).then(|| (u32::from(size.ws_xpixel), u32::from(size.ws_ypixel)));
    Some(TtySize {
        grid: GridSize {
            cols: size.ws_col,
            rows: size.ws_row,
        },
        pixels,
    })
}

pub(super) fn start_threads<E: From<TtyEvent> + Send + 'static>(
    raw: Option<&RawMode>,
    events: &Sender<E>,
) -> io::Result<()> {
    let input = events.clone();
    thread::Builder::new()
        .name("attach-input".to_owned())
        .spawn(move || read_input(&input))?;
    if raw.is_some() {
        let mut signals = Signals::new([SIGWINCH])?;
        let resize = events.clone();
        thread::Builder::new().name("attach-resize".to_owned()).spawn(move || {
            for _ in signals.forever() {
                if resize.send(TtyEvent::Resize.into()).is_err() {
                    return;
                }
            }
        })?;
    }
    Ok(())
}

/// Standard input's bytes as they come, until its end.
fn read_input<E: From<TtyEvent>>(events: &Sender<E>) {
    let mut stdin = io::stdin().lock();
    let mut buf = vec![0_u8; READ_SIZE];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(read) => {
                if events.send(TtyEvent::Input(buf[..read].to_vec()).into()).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = events.send(TtyEvent::InputEnd.into());
}

pub(super) fn write_out(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use std::os::fd::{FromRawFd as _, OwnedFd};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::attach_tty::{DEFAULT_GRID, Tty};

    /// The test binary is the terminal client; only its standard descriptors and signal handlers are changed.
    fn child(case: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "attach_tty::unix::tests::terminal_child", "--nocapture"]);
        for (name, _) in std::env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("NGTERM_")) {
            command.env_remove(name);
        }
        command.env("SESSION_TTY_CASE", case);
        command
    }

    fn pty() -> (OwnedFd, OwnedFd) {
        let (mut master, mut slave) = (-1, -1);
        let result =
            // SAFETY: both out pointers are valid; null name, termios and winsize request the defaults.
            unsafe {
                libc::openpty(
                    &raw mut master,
                    &raw mut slave,
                    std::ptr::null_mut(),
                    // `*mut` for Apple's declaration; Linux's `*const` takes it too
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
        assert_eq!(result, 0);
        // SAFETY: openpty returned a fresh master descriptor owned by this test.
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        // SAFETY: openpty returned a fresh slave descriptor owned by this test.
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        (master, slave)
    }

    /// Runs `case` in a child on a fresh terminal and fails if it fails or outlives its deadline. The terminal's output
    /// is read as it comes: macOS's `TCSAFLUSH` waits for it to drain, so an unread terminal blocks raw mode's restore.
    fn run_on_terminal(case: &str) {
        let (master, slave) = pty();
        let mut process = child(case)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The reader ends when the child's end of the terminal closes; a terminal still open stays with it.
        thread::spawn(move || io::copy(&mut std::fs::File::from(master), &mut io::sink()));
        wait_within_deadline(&mut process, case);
        let output = process.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    /// Waits for `process` to exit, killing it and failing the test once the deadline passes.
    fn wait_within_deadline(process: &mut std::process::Child, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while process.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                process.kill().unwrap();
                panic!("{what}: the child outlived its deadline");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_terminal_client_restores_raw_mode_and_reports_pixels_and_resize() {
        run_on_terminal("terminal");
    }

    #[test]
    fn piped_input_is_delivered_whole_then_ends_and_output_is_flushed() {
        let mut process = child("pipe")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process.stdin.take().unwrap().write_all(b"typed bytes").unwrap();
        let output = process.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(
            output
                .stdout
                .windows(b"flushed bytes".len())
                .any(|bytes| bytes == b"flushed bytes")
        );
    }

    #[test]
    fn a_vanished_input_receiver_stops_the_reader() {
        let mut process = child("dropped")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Input stays open, so only the failed send can end the reader before the deadline.
        let mut stdin = process.stdin.take().unwrap();
        stdin.write_all(b"unused").unwrap();
        stdin.flush().unwrap();
        wait_within_deadline(&mut process, "the reader kept reading after its receiver vanished");
        let output = process.wait_with_output().unwrap();
        drop(stdin);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn terminal_child() {
        match std::env::var("SESSION_TTY_CASE").as_deref() {
            Ok("terminal") => terminal_case(),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Ok("refused") => refuse_terminal_writes(),
            Ok("pipe") => pipe_case(),
            Ok("dropped") => {
                let (send, recv) = mpsc::channel::<TtyEvent>();
                drop(recv);
                read_input(&send);
            }
            _ => {}
        }
    }

    fn terminal_case() {
        assert!(is_terminal());
        let before = tcgetattr(io::stdin()).unwrap();
        let tty = Tty::open();
        assert!(tty.is_terminal());
        let raw = tcgetattr(io::stdin()).unwrap();
        assert!(!raw.local_modes.contains(rustix::termios::LocalModes::ICANON));
        rustix::termios::tcsetwinsize(
            io::stdout(),
            rustix::termios::Winsize {
                ws_col: 91,
                ws_row: 32,
                ws_xpixel: 910,
                ws_ypixel: 640,
            },
        )
        .unwrap();
        assert_eq!(
            tty.size(),
            TtySize {
                grid: GridSize { cols: 91, rows: 32 },
                pixels: Some((910, 640))
            }
        );
        let (send, recv) = mpsc::channel::<TtyEvent>();
        tty.start_threads(&send).unwrap();
        resize_signal();
        assert_eq!(recv.recv_timeout(Duration::from_secs(5)).unwrap(), TtyEvent::Resize);
        drop(tty);
        let restored = tcgetattr(io::stdin()).unwrap();
        assert_eq!(restored.local_modes, before.local_modes);
        assert_eq!(restored.input_modes, before.input_modes);
        assert_eq!(restored.output_modes, before.output_modes);
        assert_eq!(restored.control_modes, before.control_modes);
        rustix::termios::tcsetwinsize(
            io::stdout(),
            rustix::termios::Winsize {
                ws_col: 0,
                ws_row: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        assert_eq!(
            Tty::open().size(),
            TtySize {
                grid: DEFAULT_GRID,
                pixels: None
            }
        );
        drop(recv);
        // Wake the watcher so it observes the vanished receiver and ends.
        resize_signal();
    }

    fn pipe_case() {
        assert!(!is_terminal());
        assert!(RawMode::enter().is_err());
        let tty = Tty::open();
        assert!(!tty.is_terminal());
        assert_eq!(
            tty.size(),
            TtySize {
                grid: DEFAULT_GRID,
                pixels: None
            }
        );
        let (send, recv) = mpsc::channel::<TtyEvent>();
        tty.start_threads(&send).unwrap();
        let mut bytes = Vec::new();
        loop {
            match recv.recv_timeout(Duration::from_secs(5)).unwrap() {
                TtyEvent::Input(data) => bytes.extend(data),
                TtyEvent::InputEnd => break,
                TtyEvent::Resize => panic!("a pipe has no resize watcher"),
            }
        }
        assert_eq!(bytes, b"typed bytes");
        crate::attach_tty::write_out(b"flushed bytes").unwrap();
    }

    fn resize_signal() {
        let pid = i32::try_from(std::process::id()).unwrap();
        // SAFETY: the signal targets this test child alone, after its watcher was registered.
        let result = unsafe { libc::kill(pid, SIGWINCH) };
        assert_eq!(result, 0);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn a_terminal_that_refuses_raw_mode_remains_usable_without_it() {
        run_on_terminal("refused");
    }

    /// Refuse terminal writes at the OS boundary, on this child test's thread alone; terminal detection still works.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn refuse_terminal_writes() {
        let instruction = |code: u32, k, jt, jf| libc::sock_filter {
            code: u16::try_from(code).unwrap(),
            jt,
            jf,
            k,
        };
        // On Linux x86_64, seccomp_data holds the syscall number at byte 0 and the second argument at byte 24.
        let mut filter = [
            instruction(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, 0, 0, 0),
            instruction(
                libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K,
                u32::try_from(libc::SYS_ioctl).unwrap(),
                0,
                4,
            ),
            instruction(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, 24, 0, 0),
            instruction(
                libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K,
                u32::try_from(libc::TCGETS).unwrap(),
                2,
                0,
            ),
            instruction(
                libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K,
                u32::try_from(libc::TCGETS2).unwrap(),
                1,
                0,
            ),
            instruction(
                libc::BPF_RET | libc::BPF_K,
                libc::SECCOMP_RET_ERRNO | u32::try_from(libc::EPERM).unwrap(),
                0,
                0,
            ),
            instruction(libc::BPF_RET | libc::BPF_K, libc::SECCOMP_RET_ALLOW, 0, 0),
        ];
        let program = libc::sock_fprog {
            len: u16::try_from(filter.len()).unwrap(),
            filter: filter.as_mut_ptr(),
        };
        // SAFETY: prctl's no-new-privileges operation takes integer arguments only and affects this child thread.
        let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        assert_eq!(result, 0);
        // SAFETY: the program and its instruction array are live for the call; the kernel copies them.
        let result = unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &raw const program) };
        assert_eq!(result, 0);
        assert!(is_terminal());
        let before = tcgetattr(io::stdin()).unwrap();
        assert!(!Tty::open().is_terminal());
        assert_eq!(RawMode::enter().unwrap_err().raw_os_error(), Some(libc::EPERM));
        assert_eq!(tcgetattr(io::stdin()).unwrap().local_modes, before.local_modes);
    }
}
