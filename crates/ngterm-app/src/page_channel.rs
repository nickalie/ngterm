//! The helper carrying a handed-over page's channel as a child process (upstream's `RemotePageProcess` in
//! `RemotePageChannel.swift`): the ssh running `ngtermctl session overlay page-serve TOKEN` on the origin
//! ([`ngterm_core::remote::ssh::page_serve_command`]), its stdin and stdout piped. Stdout is read a line at a time on a
//! thread of its own, each line handed to the main loop only after it took one of the channel's permits, so a busy
//! loop backs the origin up into its own pipe; stdin is written from a thread of its own through a bounded queue, a
//! full queue counting as a helper that stopped taking requests; stderr goes to the log. The core runs the channel
//! itself ([`ngterm_core::remote::page_channel`]).
//!
//! Plain `std::process` with piped stdio on every OS, `ssh` found on `PATH` (upstream runs it through `/usr/bin/env` on
//! one end of a socket pair): nothing here is Unix-only. On Windows the helper starts without a console window
//! (`CREATE_NO_WINDOW`), which a GUI app's console child would otherwise open, and its stop is `TerminateProcess`
//! through the `Child`'s own handle.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use ngterm_core::remote::page_channel::{ChannelLink, MAX_PENDING_LINES, Permits};
use ngterm_core::remote::page_serve::MAX_FRAME_BYTES;
use ngterm_core::remote::replica_page::{HelperKey, PageHelperLauncher};
use ngterm_host::MainLoop;

/// The main-loop state a page helper reports to: a line it read (holding a permit) and its end with the reason.
pub trait PageHelperOwner: 'static {
    fn page_helper_line(&mut self, key: HelperKey, line: &[u8]);

    fn page_helper_closed(&mut self, key: HelperKey, reason: &str);
}

/// Where a helper's threads post: the main loop, held weakly so a thread outliving it posts nothing.
type Poster<S> = Weak<dyn MainLoop<S>>;

/// How often the reader checks a helper whose read ended (its stdout ended, or the reader stopped it) for its exit,
/// letting go of it between checks so a stop can still reach it.
const EXIT_POLL: Duration = Duration::from_millis(20);

/// Starts each helper as a child process reporting to the main loop.
pub struct PageHelperProcess<S> {
    poster: Poster<S>,
}

impl<S> PageHelperProcess<S> {
    #[must_use]
    pub fn new(poster: Poster<S>) -> Self {
        Self { poster }
    }
}

impl<S: PageHelperOwner> PageHelperLauncher for PageHelperProcess<S> {
    fn launch(&self, argv: &[String], key: HelperKey, permits: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
        let (program, arguments) = argv.split_first().ok_or_else(|| "an empty command".to_owned())?;
        let mut command = Command::new(program);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(
            &mut command,
            windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
        );
        let mut child = command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run {program}: {error}"))?;
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let child = Arc::new(Mutex::new(child));
        let (lines, queued) = mpsc::sync_channel::<Vec<u8>>(MAX_PENDING_LINES);
        if let Some(stdin) = stdin {
            spawn("ngterm.page-helper-write", move || write_lines(stdin, &queued));
        }
        if let Some(stderr) = stderr {
            spawn("ngterm.page-helper-stderr", move || log_lines(stderr));
        }
        let (poster, waited) = (self.poster.clone(), Arc::clone(&child));
        spawn("ngterm.page-helper-read", move || {
            let broken = stdout.and_then(|stdout| read_lines(stdout, key, &permits, &poster));
            // a read that ended before the helper's stdout did leaves it writing into a pipe nobody reads: it is
            // stopped, so the wait below ends and the close reaches the channel (upstream's `close("channel ended")`)
            if broken.is_some() {
                stop_child(&waited);
            }
            let exit = wait(&waited);
            let reason = broken.unwrap_or(exit);
            post(&poster, move |state: &mut S| state.page_helper_closed(key, &reason));
        });
        Ok(Box::new(HelperLink {
            lines: Mutex::new(Some(lines)),
            child,
        }))
    }
}

/// One running helper as the channel writes to it.
struct HelperLink {
    /// Taken at the stop, which closes the child's stdin once the writer drained it.
    lines: Mutex<Option<SyncSender<Vec<u8>>>>,
    child: Arc<Mutex<Child>>,
}

impl ChannelLink for HelperLink {
    fn send(&self, line: Vec<u8>) -> bool {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        match lines.as_ref().map(|lines| lines.try_send(line)) {
            Some(Ok(())) => true,
            Some(Err(TrySendError::Full(_) | TrySendError::Disconnected(_))) | None => false,
        }
    }

    fn stop(&self) {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).take();
        stop_child(&self.child);
    }
}

impl Drop for HelperLink {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Writes each queued line to the helper's stdin until the queue closes (the stop) or the helper stops reading.
fn write_lines(mut stdin: impl Write, queued: &mpsc::Receiver<Vec<u8>>) {
    for line in queued {
        if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
            return;
        }
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.to_owned()).spawn(work) {
        tracing::error!(%error, name, "a page helper thread did not start");
    }
}

/// Kills the helper; a child already reaped is not signalled: `Child` keeps its status, so its pid is never reused here.
fn stop_child(child: &Mutex<Child>) {
    if let Err(error) = child.lock().unwrap_or_else(PoisonError::into_inner).kill() {
        tracing::debug!(%error, "the page helper could not be stopped");
    }
}

/// Reads stdout one line at a time until its end, each handed to the main loop once it took a permit. `None` when
/// stdout ended; otherwise why the read stopped first: a read error, a line over the frame limit, a channel that closed
/// (no permit comes any more) or a main loop that is gone.
fn read_lines<S: PageHelperOwner>(
    stdout: impl Read,
    key: HelperKey,
    permits: &Permits,
    poster: &Poster<S>,
) -> Option<String> {
    let mut reader = BufReader::new(stdout);
    let limit = u64::try_from(MAX_FRAME_BYTES + 1).unwrap_or(u64::MAX);
    loop {
        let mut line = Vec::new();
        match (&mut reader).take(limit).read_until(b'\n', &mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(error) => return Some(format!("helper read failed: {error}")),
        }
        if line.last() != Some(&b'\n') {
            if line.len() <= MAX_FRAME_BYTES {
                // an unterminated last line: stdout ended
                return None;
            }
            tracing::warn!("a page helper sent an oversize line; ending the channel");
            return Some("helper sent an oversize line".to_owned());
        }
        line.pop();
        if !permits.take() {
            return Some("channel closed".to_owned());
        }
        if !post(poster, move |state: &mut S| state.page_helper_line(key, &line)) {
            return Some("main loop gone".to_owned());
        }
    }
}

/// Waits for the child's exit, letting go of it between checks; how it ended.
fn wait(child: &Mutex<Child>) -> String {
    loop {
        let status = child.lock().unwrap_or_else(PoisonError::into_inner).try_wait();
        match status {
            Ok(Some(status)) => {
                return status.code().map_or_else(
                    || "helper ended by a signal".to_owned(),
                    |code| format!("helper exit {code}"),
                );
            }
            Ok(None) => std::thread::sleep(EXIT_POLL),
            Err(error) => return format!("helper wait failed: {error}"),
        }
    }
}

fn log_lines(stderr: impl Read) {
    for line in BufReader::new(stderr).lines() {
        match line {
            Ok(line) => tracing::info!(line = %line, "page helper"),
            Err(_) => return,
        }
    }
}

/// Queues `task` on the main loop; false when the loop is closed or gone.
fn post<S: 'static>(poster: &Poster<S>, task: impl FnOnce(&mut S) + Send + 'static) -> bool {
    poster
        .upgrade()
        .is_some_and(|main_loop| main_loop.post(Box::new(task)).is_ok())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use ngterm_core::remote::page_channel::ReplicaChannel;
    use ngterm_host::fakes::FakeMainLoop;
    use uuid::Uuid;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum Report {
        Line(String),
        Closed(String),
    }

    /// The loop's state: every report goes to the test, and a line gives its permit back, as the channel does.
    struct Owner {
        reports: mpsc::Sender<Report>,
        permits: Arc<Permits>,
    }

    impl PageHelperOwner for Owner {
        fn page_helper_line(&mut self, _: HelperKey, line: &[u8]) {
            self.permits.give();
            self.reports
                .send(Report::Line(String::from_utf8_lossy(line).into_owned()))
                .unwrap();
        }

        fn page_helper_closed(&mut self, _: HelperKey, reason: &str) {
            self.reports.send(Report::Closed(reason.to_owned())).unwrap();
        }
    }

    /// An owner that keeps every permit a line took.
    struct Keeper(mpsc::Sender<Report>);

    impl PageHelperOwner for Keeper {
        fn page_helper_line(&mut self, _: HelperKey, line: &[u8]) {
            self.0
                .send(Report::Line(String::from_utf8_lossy(line).into_owned()))
                .unwrap();
        }

        fn page_helper_closed(&mut self, _: HelperKey, reason: &str) {
            self.0.send(Report::Closed(reason.to_owned())).unwrap();
        }
    }

    const KEY: HelperKey = HelperKey {
        page: Uuid::nil(),
        launch: 1,
    };

    struct Running {
        reports: mpsc::Receiver<Report>,
        link: Box<dyn ChannelLink>,
        main_loop: Arc<FakeMainLoop<Owner>>,
    }

    /// A helper running `unix` under `/bin/sh` (`windows` under PowerShell on Windows).
    fn launch(unix: &str, windows: &str) -> Running {
        let (sender, reports) = mpsc::channel();
        let permits = ReplicaChannel::permits();
        let owner = Owner {
            reports: sender,
            permits: Arc::clone(&permits),
        };
        let main_loop = Arc::new(FakeMainLoop::spawn(owner).unwrap());
        let poster: Poster<Owner> = Arc::downgrade(&main_loop) as Poster<Owner>;
        let argv = crate::test_shell::script_argv(unix, windows);
        let link = PageHelperProcess::new(poster).launch(&argv, KEY, permits).unwrap();
        Running {
            reports,
            link,
            main_loop,
        }
    }

    impl Running {
        fn until_closed(&self) -> Vec<Report> {
            let mut reports = Vec::new();
            loop {
                let report = self.reports.recv_timeout(Duration::from_secs(10)).unwrap();
                let closed = matches!(report, Report::Closed(_));
                reports.push(report);
                if closed {
                    return reports;
                }
            }
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.link.stop();
            self.main_loop.close();
        }
    }

    #[test]
    fn the_helper_hands_each_line_to_the_loop_then_its_exit() {
        let running = launch(
            "printf 'one\\ntwo\\n'; exit 3",
            "[Console]::Out.Write(\"one`ntwo`n\"); exit 3",
        );
        assert_eq!(
            running.until_closed(),
            [
                Report::Line("one".to_owned()),
                Report::Line("two".to_owned()),
                Report::Closed("helper exit 3".to_owned()),
            ]
        );
    }

    #[test]
    fn a_line_sent_reaches_the_helpers_stdin() {
        let running = launch(
            "read line; printf '%s\\n' \"got $line\"",
            "$line = [Console]::In.ReadLine(); [Console]::Out.Write(\"got $line`n\")",
        );
        assert!(running.link.send(b"{\"id\":1}\n".to_vec()));
        assert_eq!(
            running.until_closed(),
            [
                Report::Line("got {\"id\":1}".to_owned()),
                Report::Closed("helper exit 0".to_owned()),
            ]
        );
    }

    #[test]
    fn a_stop_ends_the_helper_and_refuses_later_lines() {
        let running = launch("sleep 30", "Start-Sleep 30");
        running.link.stop();
        assert!(!running.link.send(b"late\n".to_vec()));
        let reports = running.until_closed();
        assert!(matches!(&reports[..], [Report::Closed(_)]), "{reports:?}");
    }

    #[test]
    fn an_oversize_line_stops_the_helper_and_closes_the_channel() {
        // more than a frame's worth without a newline, then a helper that would otherwise live on
        let running = launch(
            &format!("head -c {} /dev/zero | tr '\\0' 'x'; sleep 30", MAX_FRAME_BYTES + 2),
            &format!("[Console]::Out.Write('x' * {}); Start-Sleep 30", MAX_FRAME_BYTES + 2),
        );
        assert_eq!(
            running.until_closed(),
            [Report::Closed("helper sent an oversize line".to_owned())]
        );
    }

    #[test]
    fn a_helper_that_cannot_start_is_refused() {
        let poster: Poster<Owner> = Weak::<FakeMainLoop<Owner>>::new();
        let argv = ["/nonexistent/ssh".to_owned()];
        let refused = PageHelperProcess::new(poster).launch(&argv, KEY, ReplicaChannel::permits());
        assert!(refused.is_err_and(|reason| reason.starts_with("could not run /nonexistent/ssh")));
    }

    #[test]
    fn the_reader_stops_reading_while_no_permit_is_free() {
        let (sender, reports) = mpsc::channel();
        let permits = Arc::new(Permits::new(1));
        // the owner keeps every permit: one line goes through, the second waits
        let main_loop = Arc::new(FakeMainLoop::spawn(Keeper(sender)).unwrap());
        let poster: Poster<Keeper> = Arc::downgrade(&main_loop) as Poster<Keeper>;
        let argv = crate::test_shell::script_argv("printf 'a\\nb\\n'", "[Console]::Out.Write(\"a`nb`n\")");
        let link = PageHelperProcess::new(poster)
            .launch(&argv, KEY, Arc::clone(&permits))
            .unwrap();
        assert_eq!(
            reports.recv_timeout(Duration::from_secs(10)).unwrap(),
            Report::Line("a".to_owned())
        );
        assert!(
            reports.recv_timeout(Duration::from_millis(200)).is_err(),
            "the second line waits for a permit"
        );
        permits.give();
        assert_eq!(
            reports.recv_timeout(Duration::from_secs(10)).unwrap(),
            Report::Line("b".to_owned())
        );
        link.stop();
        main_loop.close();
    }
}
