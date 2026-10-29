//! Bridges a streaming control command between a pair of descriptors and the app's socket (upstream's
//! `agtermctlKit/StreamBridge.swift`): the request goes out, its one ordinary reply comes back, and after an `ok`
//! everything is copied both ways untouched.
//!
//! `ngtermctl live present` runs it on the origin under the viewer's ssh, so the output is what the viewer parses:
//! only frames are ever written to it.
//!
//! On Unix the side reading the input waits in `poll` on it and on a cancel pipe, which the socket side ending
//! wakes. Windows has no `poll` for pipes or consoles: the input is read on a thread of its own, which the socket
//! side ending stops with `CancelSynchronousIo`. A console on either end is put in VT mode for the bridge's life
//! (raw input with `ENABLE_VIRTUAL_TERMINAL_INPUT`, output with `ENABLE_VIRTUAL_TERMINAL_PROCESSING`), so its bytes
//! pass untouched and nothing echoes into the frames, and is put back on every way out; the stream carries frames,
//! not a screen, so a console's resize means nothing to it.

use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::thread;

use ngterm_proto::{MAX_REQUEST_LINE_BYTES, Request, Response};

use crate::client::{self, Connection};

/// How much one read takes.
const CHUNK_BYTES: usize = 16 * 1024;

/// The socket to the app, and the files the other side's frames come in on and go out to.
#[derive(Debug)]
pub struct StreamBridge {
    socket: Connection,
    input: File,
    output: File,
}

impl StreamBridge {
    /// A bridge over `socket`, reading the other side from `input` and writing it to `output`. The pump works on
    /// duplicates of both.
    #[must_use]
    pub fn new(socket: Connection, input: File, output: File) -> Self {
        Self { socket, input, output }
    }

    /// A bridge over `socket` and duplicates of the process's stdin and stdout, which stay open.
    ///
    /// # Errors
    ///
    /// Either could not be duplicated.
    pub fn over_stdio(socket: Connection) -> Result<Self, String> {
        let duplicated = stdio_file(&io::stdin()).and_then(|input| Ok((input, stdio_file(&io::stdout())?)));
        let (input, output) =
            duplicated.map_err(|error| format!("no stdin or stdout: {}", client::os_message(&error)))?;
        Ok(Self::new(socket, input, output))
    }

    /// Sends `request` and reads its reply, having written nothing to the output either way.
    ///
    /// # Errors
    ///
    /// The server's refusal, or why no reply came.
    pub fn open(&self, request: &Request) -> Result<(), String> {
        let mut line = serde_json::to_vec(request).map_err(|error| format!("could not encode the request: {error}"))?;
        line.push(b'\n');
        client::send_all(&self.socket, &line).map_err(|_| "could not send the request".to_owned())?;
        // a byte at a time: a frame can sit right behind the reply in the same packet
        let reply = client::read_socket_line(&self.socket, MAX_REQUEST_LINE_BYTES)
            .ok_or_else(|| "the app closed the connection without answering".to_owned())?;
        let response: Response =
            serde_json::from_slice(&reply).map_err(|error| format!("could not decode the reply: {error}"))?;
        if response.ok {
            Ok(())
        } else {
            Err(response
                .error
                .unwrap_or_else(|| "the app refused the stream".to_owned()))
        }
    }

    /// Copies both ways until either side ends. The input ending shuts the socket down, which is how the app learns
    /// the viewer went away; the socket ending returns. Both directions have stopped by the time this returns.
    #[cfg(unix)]
    pub fn pump(&self) {
        use std::os::fd::AsRawFd as _;
        let (Ok(input), Ok(output)) = (self.input.try_clone(), self.output.try_clone()) else {
            return;
        };
        let Ok((cancel, wake)) = io::pipe() else {
            return;
        };
        let socket = &self.socket;
        // the reader stays here, open until both directions stopped: the input side may finish first, and a wake
        // written into a pipe with no reader left raises SIGPIPE, which main restored to its default
        let cancel_fd = cancel.as_raw_fd();
        thread::scope(|scope| {
            let upstream = thread::Builder::new()
                .name("ngtermctl.stream.upstream".to_owned())
                .spawn_scoped(scope, move || copy_until_cancelled(input, socket, cancel_fd));
            copy(socket, output);
            let mut wake = wake;
            let _ = wake.write_all(&[1]);
            if let Ok(upstream) = upstream {
                let _ = upstream.join();
            }
        });
    }
}

/// A duplicate of a standard stream, owned, so closing it leaves the stream open.
#[cfg(unix)]
fn stdio_file(stream: &impl std::os::fd::AsFd) -> io::Result<File> {
    stream.as_fd().try_clone_to_owned().map(File::from)
}

/// A duplicate of a standard stream, owned, so closing it leaves the stream open.
#[cfg(windows)]
fn stdio_file(stream: &impl std::os::windows::io::AsHandle) -> io::Result<File> {
    stream.as_handle().try_clone_to_owned().map(File::from)
}

/// Copies `source` to the socket until `source` ends, which shuts the socket down, or `cancel` becomes readable, which
/// just stops.
#[cfg(unix)]
fn copy_until_cancelled(mut source: File, socket: &Connection, cancel: std::os::fd::RawFd) {
    use std::os::fd::AsRawFd as _;
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    loop {
        match wait_readable(source.as_raw_fd(), cancel) {
            Readiness::Interrupted => continue,
            Readiness::Cancelled | Readiness::Failed => return,
            Readiness::Source => {}
        }
        let count = match source.read(&mut chunk) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Ok(count) => count,
            Err(_) => 0,
        };
        if count == 0 || client::send_all(socket, &chunk[..count]).is_err() {
            let _ = socket.shutdown(Shutdown::Both);
            return;
        }
    }
}

/// Copies the socket to `destination` until either ends.
fn copy(mut socket: &Connection, mut destination: File) {
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    loop {
        let count = match socket.read(&mut chunk) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Ok(count) => count,
            Err(_) => 0,
        };
        if count == 0 || destination.write_all(&chunk[..count]).is_err() {
            return;
        }
    }
}

#[cfg(unix)]
enum Readiness {
    Source,
    Cancelled,
    Interrupted,
    Failed,
}

/// Blocks until `source` or `cancel` can be read; the cancel wins a tie.
#[cfg(unix)]
fn wait_readable(source: std::os::fd::RawFd, cancel: std::os::fd::RawFd) -> Readiness {
    let mut pollers = [
        libc::pollfd {
            fd: source,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: cancel,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    // SAFETY: `pollers` is a live array of two initialized `pollfd`s, and the count passed is its length.
    let ready = unsafe { libc::poll(pollers.as_mut_ptr(), 2, -1) };
    if ready < 0 {
        return if io::Error::last_os_error().kind() == ErrorKind::Interrupted {
            Readiness::Interrupted
        } else {
            Readiness::Failed
        };
    }
    if pollers[1].revents != 0 {
        Readiness::Cancelled
    } else {
        Readiness::Source
    }
}

#[cfg(windows)]
impl StreamBridge {
    /// Copies both ways until either side ends. The input ending shuts the socket down, which is how the app learns
    /// the viewer went away; the socket ending returns. Both directions have stopped by the time this returns.
    pub fn pump(&self) {
        let (Ok(input), Ok(output), Ok(socket)) =
            (self.input.try_clone(), self.output.try_clone(), self.socket.try_clone())
        else {
            return;
        };
        let _console = windows::ConsoleModes::vt(&self.input, &self.output);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let upstream = {
            let stop = std::sync::Arc::clone(&stop);
            thread::Builder::new()
                .name("ngtermctl.stream.upstream".to_owned())
                .spawn(move || copy_until_stopped(input, &socket, &stop))
        };
        copy(&self.socket, output);
        if let Ok(upstream) = upstream
            && windows::stop_reader(&upstream, &stop)
        {
            let _ = upstream.join();
        }
    }
}

/// Copies `source` to the socket until `source` ends, which shuts the socket down, or `stop` is set, which just
/// stops; a read the stop cancelled ends it too.
#[cfg(windows)]
fn copy_until_stopped(mut source: File, socket: &Connection, stop: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    while !stop.load(Ordering::SeqCst) {
        let count = match source.read(&mut chunk) {
            Err(error) if error.kind() == ErrorKind::Interrupted && !stop.load(Ordering::SeqCst) => continue,
            Ok(count) => count,
            Err(_) => 0,
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if count == 0 || client::send_all(socket, &chunk[..count]).is_err() {
            let _ = socket.shutdown(Shutdown::Both);
            return;
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
        SetConsoleMode,
    };
    use windows_sys::Win32::System::IO::CancelSynchronousIo;

    /// How long [`stop_reader`] keeps cancelling a read that will not end before it leaves the thread to the
    /// process's exit.
    const STOP_DRAIN: Duration = Duration::from_secs(2);

    /// Sets `stop`, then cancels the reader's blocking read until the thread is done, and says whether it is: a
    /// cancel that lands before the read starts is lost, and the flag alone cannot end a read already waiting. A
    /// read no cancel ends (a device that does not take one) is left past [`STOP_DRAIN`], its thread unjoined.
    pub(super) fn stop_reader(reader: &JoinHandle<()>, stop: &AtomicBool) -> bool {
        stop.store(true, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + STOP_DRAIN;
        while !reader.is_finished() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            // SAFETY: the thread's handle stays open while its `JoinHandle` lives; a thread with no read pending
            // fails the call, which changes nothing.
            unsafe { CancelSynchronousIo(reader.as_raw_handle()) };
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// The consoles' modes before the bridge changed them, put back on drop.
    pub(super) struct ConsoleModes {
        saved: Vec<(File, CONSOLE_MODE)>,
    }

    impl ConsoleModes {
        /// Raw VT input on `input` and VT output on `output`, for whichever of them is a console.
        pub(super) fn vt(input: &File, output: &File) -> Self {
            let mut saved = Vec::new();
            let raw_input = |mode: CONSOLE_MODE| {
                (mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT
            };
            let vt_output = |mode: CONSOLE_MODE| {
                mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN
            };
            for (file, change) in [
                (input, &raw_input as &dyn Fn(CONSOLE_MODE) -> CONSOLE_MODE),
                (output, &vt_output),
            ] {
                let Ok(file) = file.try_clone() else {
                    continue;
                };
                let mut mode: CONSOLE_MODE = 0;
                // SAFETY: the handle is open; a handle that is not a console fails the call.
                if unsafe { GetConsoleMode(file.as_raw_handle(), &raw mut mode) } == 0 {
                    continue;
                }
                // SAFETY: as above, a console handle and a mode built from its own.
                if unsafe { SetConsoleMode(file.as_raw_handle(), change(mode)) } != 0 {
                    saved.push((file, mode));
                }
            }
            Self { saved }
        }
    }

    impl Drop for ConsoleModes {
        fn drop(&mut self) {
            for (file, mode) in &self.saved {
                // SAFETY: the console handle the mode was read from, still open.
                unsafe { SetConsoleMode(file.as_raw_handle(), *mode) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufRead;
    use std::time::Duration;

    use ngterm_proto::Command;
    use ngterm_proto::command::LiveCmd;

    use super::*;

    fn present() -> Request {
        Request {
            cmd: Command::Live(LiveCmd::Present),
            target: Some("S".to_owned()),
            args: None,
        }
    }

    /// A bridge between a socket pair and two pipes: the app's end, the viewer's writer and the viewer's reader.
    struct Rig {
        app: Connection,
        viewer_in: io::PipeWriter,
        viewer_out: io::BufReader<io::PipeReader>,
        bridge: StreamBridge,
        _ends: (io::PipeReader, io::PipeWriter),
    }

    /// A pipe end as the file the bridge takes.
    #[cfg(unix)]
    fn file(end: impl Into<std::os::fd::OwnedFd>) -> File {
        File::from(end.into())
    }

    #[cfg(windows)]
    fn file(end: impl Into<std::os::windows::io::OwnedHandle>) -> File {
        File::from(end.into())
    }

    fn rig() -> Rig {
        let (bridge_side, app) = ngterm_server::endpoint::pair().unwrap();
        let (input_reader, viewer_in) = io::pipe().unwrap();
        let (viewer_out, output_writer) = io::pipe().unwrap();
        let bridge = StreamBridge::new(
            bridge_side,
            file(input_reader.try_clone().unwrap()),
            file(output_writer.try_clone().unwrap()),
        );
        Rig {
            app,
            viewer_in,
            viewer_out: io::BufReader::new(viewer_out),
            bridge,
            _ends: (input_reader, output_writer),
        }
    }

    fn app_line(app: &Connection) -> String {
        let mut line = String::new();
        io::BufReader::new(app).read_line(&mut line).unwrap();
        line
    }

    #[test]
    fn the_request_goes_first_and_frames_then_pass_both_ways_unmodified() {
        let Rig {
            app,
            mut viewer_in,
            mut viewer_out,
            bridge,
            _ends,
        } = rig();
        let app_side = thread::spawn(move || {
            let request = app_line(&app);
            // the first frame rides right behind the reply in one write
            client::send_all(&app, b"{\"ok\":true,\"result\":{\"id\":\"S\"}}\n{\"kind\":\"hello\"}\n").unwrap();
            let from_viewer = app_line(&app);
            (request, from_viewer)
        });
        bridge.open(&present()).unwrap();
        let pumping = thread::spawn(move || bridge.pump());
        let mut frame = String::new();
        viewer_out.read_line(&mut frame).unwrap();
        assert_eq!(frame, "{\"kind\":\"hello\"}\n");
        viewer_in.write_all(b"{\"kind\":\"ack\"}\n").unwrap();
        let (request, from_viewer) = app_side.join().unwrap();
        let sent: Request = serde_json::from_str(request.trim_end()).unwrap();
        assert_eq!(sent, present());
        assert_eq!(from_viewer, "{\"kind\":\"ack\"}\n");
        // the app hanging up ends the bridge even though stdin stays open
        pumping.join().unwrap();
        drop(viewer_in);
    }

    #[test]
    fn stdin_closing_ends_the_bridge_and_the_app_sees_end_of_stream() {
        let Rig {
            app,
            viewer_in,
            viewer_out: _viewer_out,
            bridge,
            _ends,
        } = rig();
        // before anything can shut the bridge's side down: macOS refuses a socket option once the peer shut down
        app.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        client::send_all(&app, b"{\"ok\":true}\n").unwrap();
        bridge.open(&present()).unwrap();
        let pumping = thread::spawn(move || bridge.pump());
        drop(viewer_in);
        let mut rest = Vec::new();
        (&app).read_to_end(&mut rest).unwrap();
        assert!(
            String::from_utf8_lossy(&rest).contains("live.present"),
            "the request, then end of stream"
        );
        drop(app);
        pumping.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn the_viewer_leaving_first_ends_the_bridge() {
        // which side of the bridge notices the end first is up to the scheduler
        for _ in 0..50 {
            let Rig {
                app,
                viewer_in,
                viewer_out: _viewer_out,
                bridge,
                _ends,
            } = rig();
            client::send_all(&app, b"{\"ok\":true}\n").unwrap();
            bridge.open(&present()).unwrap();
            drop(viewer_in);
            bridge.pump();
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_viewer_leaving_first_ends_the_bridge_without_sigpipe() {
        // with SIGPIPE at its default, as the real binary runs: the socket side ending after the input did wakes the
        // finished input side's cancel pipe, which must still have its reader. Repeated, as which side of the bridge
        // notices the end first is up to the scheduler.
        crate::local::default_sigpipe::run(
            "stream_bridge::tests::the_viewer_leaving_first_ends_the_bridge_without_sigpipe",
            || {
                for _ in 0..200 {
                    let Rig {
                        app,
                        viewer_in,
                        viewer_out: _viewer_out,
                        bridge,
                        _ends,
                    } = rig();
                    client::send_all(&app, b"{\"ok\":true}\n").unwrap();
                    bridge.open(&present()).unwrap();
                    drop(viewer_in);
                    bridge.pump();
                }
            },
        );
    }

    #[test]
    fn a_refusal_or_a_silent_close_is_an_error_and_writes_nothing() {
        let Rig {
            app,
            viewer_out,
            bridge,
            _ends: ends,
            ..
        } = rig();
        client::send_all(&app, b"{\"ok\":false,\"error\":\"no such session: S\"}\n").unwrap();
        assert_eq!(bridge.open(&present()), Err("no such session: S".to_owned()));
        drop(bridge);
        drop(ends);
        let mut written = Vec::new();
        viewer_out.into_inner().read_to_end(&mut written).unwrap();
        assert!(written.is_empty());
        let Rig { app, bridge, .. } = rig();
        // takes the request, then hangs up without answering
        let hanging_up = thread::spawn(move || drop(app_line(&app)));
        assert_eq!(
            bridge.open(&present()),
            Err("the app closed the connection without answering".to_owned())
        );
        hanging_up.join().unwrap();
    }

    #[test]
    fn the_input_ending_while_the_app_is_silent_ends_the_bridge() {
        let Rig {
            app,
            viewer_in,
            viewer_out: _viewer_out,
            bridge,
            _ends,
        } = rig();
        client::send_all(&app, b"{\"ok\":true}\n").unwrap();
        bridge.open(&present()).unwrap();
        let pumping = thread::spawn(move || bridge.pump());
        thread::sleep(Duration::from_millis(100));
        drop(viewer_in);
        pumping.join().unwrap();
        drop(app);
    }
}
