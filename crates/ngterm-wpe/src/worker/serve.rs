//! The worker's side (module docs of `worker`): the hello, the in-process engine started with the app's
//! configuration, the app's commands handed to it, its events written back by a thread of their own, and the end.

use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

use super::message::{self, FromWorker, PROTOCOL, StartFailure, ToWorker};
use super::wire::{self, Limits, Packet};
use crate::event::{Event, SchemeResponse};
use crate::frames::{BufferId, Frame, FrameContent};
use crate::host::{Command, Local};
use crate::spec::ViewId;

/// How long the worker waits for its last answer to leave before it ends anyway (an app that stopped reading).
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// A broken worker the conformance suite asks for (`worker::FAULT_ENV`), so the app's handling of one runs end to end
/// (`web-worker-faults` in `conformance/docs/rust-build.md`). Without it the worker keeps its protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fault {
    /// Ends before its hello (the process, at once).
    EndBeforeHello,
    /// Stops reading before its hello, so the app's configuration cannot reach it, then waits for the app to kill it.
    DeafAfterHello,
    /// Says no hello, and its read of the app's configuration fails (times out) while the app waits for that hello.
    TimeoutBeforeHello,
    /// Says hello only once its app hung up.
    HelloAfterHangUp,
    /// Answers an evaluation with a second hello.
    HelloOnEvaluate,
    /// Sends a frame of each page the app closes, after the close.
    FrameAfterClose,
    /// Holds the app's replies to its pages' requests until the app closes a page, then hands them on after the close.
    ReplyAfterClose,
}

impl Fault {
    /// The fault `name` names (`end-before-hello`, ...), or `None`.
    pub(super) fn named(name: &str) -> Option<Self> {
        Some(match name {
            "end-before-hello" => Self::EndBeforeHello,
            "deaf-after-hello" => Self::DeafAfterHello,
            "timeout-before-hello" => Self::TimeoutBeforeHello,
            "hello-after-hang-up" => Self::HelloAfterHangUp,
            "hello-on-evaluate" => Self::HelloOnEvaluate,
            "frame-after-close" => Self::FrameAfterClose,
            "reply-after-close" => Self::ReplyAfterClose,
            _ => return None,
        })
    }
}

/// What the writer thread sends.
enum Outgoing {
    Packet(Packet),
    /// Told once everything queued before it was sent.
    Flush(std::sync::mpsc::SyncSender<()>),
}

/// Serves the engine on `socket` until the app quits it or goes away, playing `fault` if one is asked for; the worker's
/// exit code.
pub(super) fn serve(socket: OwnedFd, fault: Option<Fault>) -> i32 {
    // inherited across the exec alone: nothing WebKit starts may keep the app's peer alive
    if let Err(err) = rustix::io::fcntl_setfd(&socket, rustix::io::FdFlags::CLOEXEC) {
        eprintln!("ngterm: the WPE worker cannot make its socket close-on-exec: {err}");
        return 2;
    }
    let socket = Arc::new(socket);
    match fault {
        // the socket closes as the process ends, after its coverage profile is written: the app kills a worker whose
        // start failed at once
        Some(Fault::EndBeforeHello) => crate::exit_process(0),
        Some(Fault::DeafAfterHello) => {
            let _ = rustix::net::shutdown(&*socket, rustix::net::Shutdown::Read);
        }
        Some(Fault::TimeoutBeforeHello) => {
            let _ = rustix::net::sockopt::set_socket_timeout(
                &*socket,
                rustix::net::sockopt::Timeout::Recv,
                Some(Duration::from_millis(1)),
            );
            // held until the process ends, after its coverage profile is written: as with `EndBeforeHello`
            let _ = socket.try_clone().map(std::os::fd::IntoRawFd::into_raw_fd);
        }
        Some(Fault::HelloAfterHangUp) => {
            // the app sends nothing before the hello: what ends this wait is its hang-up
            let _ = wire::recv(socket.as_fd(), Limits::FROM_APP);
        }
        _ => {}
    }
    if fault != Some(Fault::TimeoutBeforeHello) && send(&socket, &hello()).is_err() {
        return 0;
    }
    if fault == Some(Fault::DeafAfterHello) {
        wait_to_be_killed();
    }
    let first = match wire::recv(socket.as_fd(), Limits::FROM_APP) {
        Ok(Some(packet)) => message::body::<ToWorker>(&packet),
        Ok(None) => return 0,
        Err(err) => {
            eprintln!("ngterm: the WPE worker's socket: {err}");
            return 2;
        }
    };
    let Ok(ToWorker::Start(config)) = first else {
        eprintln!("ngterm: the WPE worker was not given its configuration first");
        return 2;
    };
    let (engine, events) = match Local::start(config) {
        Ok(started) => started,
        Err(error) => {
            let _ = send(&socket, &FromWorker::Started(Err(StartFailure::from(&error))));
            return 1;
        }
    };
    if send(&socket, &FromWorker::Started(Ok(()))).is_err() {
        return 0;
    }
    let (outgoing, queue) = async_channel::unbounded::<Outgoing>();
    let writer_socket = Arc::clone(&socket);
    std::thread::spawn(move || {
        while let Ok(item) = queue.recv_blocking() {
            match item {
                Outgoing::Packet(packet) => {
                    if wire::send(writer_socket.as_fd(), &packet).is_err() {
                        // the app is gone: the reader sees the end of the socket and stops the engine before the
                        // worker ends
                        let _ = rustix::net::shutdown(&*writer_socket, rustix::net::Shutdown::Both);
                        return;
                    }
                }
                Outgoing::Flush(done) => {
                    let _ = done.send(());
                }
            }
        }
    });
    let forward = outgoing.clone();
    std::thread::spawn(move || {
        while let Ok(event) = events.recv_blocking() {
            match FromWorker::event_packet(event) {
                Ok(packet) => {
                    let _ = forward.try_send(Outgoing::Packet(packet));
                }
                Err(err) => tracing::error!(%err, "a WPE event could not be written for the app"),
            }
        }
    });
    serve_commands(&socket, &engine, &outgoing, fault)
}

fn hello() -> FromWorker {
    FromWorker::Hello {
        protocol: PROTOCOL,
        version: ngterm_wpe_client::worker::build_identity(),
        pid: std::process::id(),
    }
}

/// [`Fault::DeafAfterHello`]'s end: the app kills a worker whose start failed, and an exit of its own would race that
/// kill for the coverage profile it writes.
fn wait_to_be_killed() -> ! {
    loop {
        std::thread::park();
    }
}

/// Hands the app's commands to `engine` until a quit (after which the engine's stop is told and flushed) or the end of
/// the socket; the engine stops either way, so its web processes end with the worker (`engine::stop`) rather than
/// outlive it.
fn serve_commands(
    socket: &OwnedFd,
    engine: &Local,
    outgoing: &async_channel::Sender<Outgoing>,
    fault: Option<Fault>,
) -> i32 {
    let code = serve_until_end(socket, engine, outgoing, fault);
    engine.stop();
    code
}

/// [`serve_commands`]'s loop: the worker's exit code once the app quit the engine or went away.
fn serve_until_end(
    socket: &OwnedFd,
    engine: &Local,
    outgoing: &async_channel::Sender<Outgoing>,
    fault: Option<Fault>,
) -> i32 {
    let mut held = Vec::new();
    loop {
        let packet = match wire::recv(socket.as_fd(), Limits::FROM_APP) {
            Ok(Some(packet)) => packet,
            Ok(None) => return 0,
            Err(err) => {
                eprintln!("ngterm: the WPE worker's socket broke: {err}");
                return 2;
            }
        };
        let blob = packet.blob.clone();
        match message::body::<ToWorker>(&packet) {
            Ok(ToWorker::Command(Command::Quit)) => {
                engine.stop();
                if let Ok(stopped) = FromWorker::Stopped.packet() {
                    let _ = outgoing.try_send(Outgoing::Packet(stopped));
                }
                let (done, flushed) = std::sync::mpsc::sync_channel(1);
                let _ = outgoing.try_send(Outgoing::Flush(done));
                let _ = flushed.recv_timeout(FLUSH_TIMEOUT);
                return 0;
            }
            Ok(ToWorker::Command(command)) => {
                post(&mut |command| engine.post(command), outgoing, command, fault, &mut held);
            }
            Ok(ToWorker::PersistentSession {
                session,
                data_dir,
                cache_dir,
            }) => engine.post(Command::CreateSession(
                session,
                crate::SessionSpec::Persistent { data_dir, cache_dir },
            )),
            Ok(ToWorker::SchemeFile { token, path }) => engine.post(Command::Scheme(token, SchemeResponse::File(path))),
            Ok(ToWorker::SchemeBytes { token, mime }) => engine.post(Command::Scheme(
                token,
                SchemeResponse::Bytes {
                    data: blob.unwrap_or_default(),
                    mime,
                },
            )),
            Ok(ToWorker::Evaluate { .. }) if fault == Some(Fault::HelloOnEvaluate) => {
                if let Ok(packet) = hello().packet() {
                    let _ = outgoing.try_send(Outgoing::Packet(packet));
                }
            }
            Ok(ToWorker::Evaluate {
                id,
                view,
                script,
                world,
            }) => {
                let result = engine.evaluate(view, script, world);
                let answer = outgoing.clone();
                std::thread::spawn(move || {
                    let result = result.recv_blocking().unwrap_or_else(|_| Err("page closed".to_owned()));
                    if let Ok(packet) = (FromWorker::Evaluation { id, result }).packet() {
                        let _ = answer.try_send(Outgoing::Packet(packet));
                    }
                });
            }
            Ok(ToWorker::Start(_)) => tracing::warn!("the WPE worker was started twice: ignored"),
            Err(err) => {
                eprintln!("ngterm: the WPE worker got a broken message: {err}");
                return 2;
            }
        }
    }
}

/// Hands `command` to the engine through `engine`; under [`Fault::FrameAfterClose`] a page's close is followed by a frame
/// of it, and under [`Fault::ReplyAfterClose`] replies wait in `held` for the next close and follow it.
fn post(
    engine: &mut impl FnMut(Command),
    outgoing: &async_channel::Sender<Outgoing>,
    command: Command,
    fault: Option<Fault>,
    held: &mut Vec<Command>,
) {
    if fault == Some(Fault::ReplyAfterClose) && matches!(command, Command::Reply(..)) {
        held.push(command);
        return;
    }
    let closed = match &command {
        Command::CloseView(view) => Some(*view),
        _ => None,
    };
    engine(command);
    let Some(view) = closed else { return };
    if fault == Some(Fault::FrameAfterClose) {
        send_late_frame(outgoing, view);
    }
    for reply in held.drain(..) {
        engine(reply);
    }
}

/// [`Fault::FrameAfterClose`]: an empty frame of page `view`, which the app closed before it sent the close.
fn send_late_frame(outgoing: &async_channel::Sender<Outgoing>, view: ViewId) {
    let frame = Frame {
        view,
        buffer: BufferId::from_raw(0),
        width: 1,
        height: 1,
        damage: Vec::new(),
        content: FrameContent::Shm(Vec::new()),
    };
    if let Ok(packet) = FromWorker::event_packet(Event::Frame(frame)) {
        let _ = outgoing.try_send(Outgoing::Packet(packet));
    }
}

fn send(socket: &OwnedFd, message: &FromWorker) -> Result<(), wire::WireError> {
    wire::send(socket.as_fd(), &message.packet()?)
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
