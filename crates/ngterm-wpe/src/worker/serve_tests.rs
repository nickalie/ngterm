use super::*;

fn pair() -> (OwnedFd, OwnedFd) {
    use rustix::net::{AddressFamily, SocketFlags, SocketType};
    rustix::net::socketpair(AddressFamily::UNIX, SocketType::SEQPACKET, SocketFlags::CLOEXEC, None).unwrap()
}

fn hello(socket: &OwnedFd) {
    let packet = wire::recv(socket.as_fd(), Limits::FROM_WORKER).unwrap().unwrap();
    let FromWorker::Hello { protocol, version, pid } = message::body(&packet).unwrap() else {
        panic!("hello")
    };
    assert_eq!(protocol, PROTOCOL);
    assert_eq!(version, ngterm_wpe_client::worker::build_identity());
    assert_eq!(pid, std::process::id());
}

#[test]
fn the_worker_requires_configuration_first_and_handles_a_peer_that_goes_away() {
    for body in [b"{}".to_vec(), b"not json".to_vec(), br#"{"Command":"Quit"}"#.to_vec()] {
        let (app, worker) = pair();
        let serving = std::thread::spawn(move || serve(worker, None));
        hello(&app);
        wire::send(
            app.as_fd(),
            &Packet {
                body,
                ..Packet::default()
            },
        )
        .unwrap();
        assert_eq!(serving.join().unwrap(), 2);
    }
    let (app, worker) = pair();
    let serving = std::thread::spawn(move || serve(worker, None));
    hello(&app);
    drop(app);
    assert_eq!(serving.join().unwrap(), 0);
    let (app, worker) = pair();
    drop(app);
    assert_eq!(serve(worker, None), 0);
}

#[test]
fn the_worker_refuses_a_broken_transport_before_starting() {
    let (app, worker) = pair();
    let serving = std::thread::spawn(move || serve(worker, None));
    hello(&app);
    rustix::net::send(&app, b"short", rustix::net::SendFlags::NOSIGNAL).unwrap();
    assert_eq!(serving.join().unwrap(), 2);
}

#[test]
fn commands_after_the_handshake_are_forwarded_and_malformed_messages_end_the_worker() {
    crate::tests::isolated(
        "worker::serve::tests::commands_after_the_handshake_are_forwarded_and_malformed_messages_end_the_worker",
        || {
            let (engine, _events) = Local::start(crate::HostConfig {
                frames: crate::FrameMode::Shm,
                schemes: Vec::new(),
            })
            .unwrap();
            let (app, socket) = pair();
            let (outgoing, queue) = async_channel::unbounded();
            let reader = std::thread::spawn(move || forward_commands(&app, &queue));
            assert_eq!(serve_commands(&socket, &engine, &outgoing, None), 2);
            reader.join().unwrap();
            let (app, socket) = pair();
            drop(app);
            assert_eq!(serve_until_end(&socket, &engine, &outgoing, None), 0);
            let (app, socket) = pair();
            rustix::net::send(&app, b"short", rustix::net::SendFlags::NOSIGNAL).unwrap();
            assert_eq!(serve_until_end(&socket, &engine, &outgoing, None), 2);
        },
    );
}

fn forward_commands(app: &OwnedFd, queue: &async_channel::Receiver<Outgoing>) {
    let (message, blob) = ToWorker::command(Command::Clipboard("desktop text".to_owned()));
    wire::send(app.as_fd(), &message.packet(blob).unwrap()).unwrap();
    wire::send(
        app.as_fd(),
        &ToWorker::Start(crate::HostConfig {
            frames: crate::FrameMode::Shm,
            schemes: Vec::new(),
        })
        .packet(None)
        .unwrap(),
    )
    .unwrap();
    wire::send(
        app.as_fd(),
        &ToWorker::SchemeBytes {
            token: crate::SchemeToken::from_raw(99),
            mime: "text/plain".to_owned(),
        }
        .packet(None)
        .unwrap(),
    )
    .unwrap();
    wire::send(
        app.as_fd(),
        &ToWorker::Evaluate {
            id: 7,
            view: crate::ViewId::from_raw(99),
            script: "1".to_owned(),
            world: None,
        }
        .packet(None)
        .unwrap(),
    )
    .unwrap();
    let Outgoing::Packet(packet) = queue.recv_blocking().unwrap() else {
        panic!("evaluation packet")
    };
    let FromWorker::Evaluation { id, result } = message::body(&packet).unwrap() else {
        panic!("evaluation")
    };
    assert_eq!(id, 7);
    assert_eq!(result, Err("no such page".to_owned()));
    wire::send(
        app.as_fd(),
        &Packet {
            body: b"broken".to_vec(),
            ..Packet::default()
        },
    )
    .unwrap();
}

fn answer(app: &OwnedFd) -> FromWorker {
    let packet = wire::recv(app.as_fd(), Limits::FROM_WORKER).unwrap().unwrap();
    message::body(&packet).unwrap()
}

#[test]
fn a_worker_reports_its_display_start_failure() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_reports_its_display_start_failure",
        || {
            let (app, worker) = pair();
            let serving = std::thread::spawn(move || serve(worker, None));
            hello(&app);
            let config = crate::HostConfig {
                frames: crate::FrameMode::DmaBuf(crate::DmaBufTarget {
                    render_node: "/dev/dri/renderD250".into(),
                    formats: vec![crate::BufferFormat {
                        fourcc: crate::fourcc(b"XR24"),
                        modifiers: vec![crate::MODIFIER_LINEAR],
                    }],
                }),
                schemes: Vec::new(),
            };
            wire::send(app.as_fd(), &ToWorker::Start(config).packet(None).unwrap()).unwrap();
            let FromWorker::Started(Err(StartFailure::Display(message))) = answer(&app) else {
                panic!("display failure")
            };
            assert!(message.contains("not found"), "{message}");
            assert_eq!(serving.join().unwrap(), 1);
        },
    );
}

#[test]
fn a_worker_forwards_engine_events_and_flushes_its_stop_before_returning() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_forwards_engine_events_and_flushes_its_stop_before_returning",
        || {
            use std::os::unix::ffi::OsStringExt as _;

            let (app, serving) = started_worker();
            let home = tempfile::tempdir().unwrap();
            let spec = crate::SessionSpec::Persistent {
                data_dir: home.path().join(std::ffi::OsString::from_vec(b"data-\xff".to_vec())),
                cache_dir: home.path().join(std::ffi::OsString::from_vec(b"cache-\xfe".to_vec())),
            };
            let session = crate::SessionId::from_raw(1);
            for command in [Command::CreateSession(session, spec), Command::ClearSession(session)] {
                let (message, blob) = ToWorker::command(command);
                wire::send(app.as_fd(), &message.packet(blob).unwrap()).unwrap();
            }
            assert!(
                matches!(answer(&app), FromWorker::Event(crate::Event::SessionCleared { session: id, result: Ok(()) }) if id == session)
            );
            let (quit, blob) = ToWorker::command(Command::Quit);
            wire::send(app.as_fd(), &quit.packet(blob).unwrap()).unwrap();
            assert!(matches!(answer(&app), FromWorker::Stopped));
            assert_eq!(serving.join().unwrap(), 0);
        },
    );
}

/// The app's end of a worker serving on a thread, past the hello and the engine's start.
fn started_worker() -> (OwnedFd, std::thread::JoinHandle<i32>) {
    faulty_worker(None)
}

/// [`started_worker`] playing `fault`.
fn faulty_worker(fault: Option<Fault>) -> (OwnedFd, std::thread::JoinHandle<i32>) {
    let (app, worker) = pair();
    let serving = std::thread::spawn(move || serve(worker, fault));
    hello(&app);
    let config = crate::HostConfig {
        frames: crate::FrameMode::Shm,
        schemes: Vec::new(),
    };
    wire::send(app.as_fd(), &ToWorker::Start(config).packet(None).unwrap()).unwrap();
    assert!(matches!(answer(&app), FromWorker::Started(Ok(()))));
    (app, serving)
}

#[test]
fn a_worker_whose_app_goes_away_before_hearing_of_the_start_ends_quietly() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_whose_app_goes_away_before_hearing_of_the_start_ends_quietly",
        || {
            let (app, worker) = pair();
            let serving = std::thread::spawn(move || serve(worker, None));
            hello(&app);
            // the startup reply must fail even if the worker handles Start before this thread runs again
            rustix::net::shutdown(&app, rustix::net::Shutdown::Read).unwrap();
            let config = crate::HostConfig {
                frames: crate::FrameMode::Shm,
                schemes: Vec::new(),
            };
            wire::send(app.as_fd(), &ToWorker::Start(config).packet(None).unwrap()).unwrap();
            assert_eq!(serving.join().unwrap(), 0);
        },
    );
}

#[test]
fn a_worker_whose_app_stops_reading_its_events_ends() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_whose_app_stops_reading_its_events_ends",
        || {
            let (app, serving) = started_worker();
            // the worker's next write fails (EPIPE) while the app's socket stays open: only that failure ends it
            rustix::net::shutdown(&app, rustix::net::Shutdown::Read).unwrap();
            let session = crate::SessionId::from_raw(1);
            for command in [
                Command::CreateSession(session, crate::SessionSpec::Ephemeral),
                Command::ClearSession(session),
            ] {
                let (message, blob) = ToWorker::command(command);
                wire::send(app.as_fd(), &message.packet(blob).unwrap()).unwrap();
            }
            assert_eq!(serving.join().unwrap(), 0);
        },
    );
}

#[test]
fn a_fault_is_named_as_the_suite_names_it() {
    for (name, fault) in [
        ("end-before-hello", Fault::EndBeforeHello),
        ("deaf-after-hello", Fault::DeafAfterHello),
        ("timeout-before-hello", Fault::TimeoutBeforeHello),
        ("hello-after-hang-up", Fault::HelloAfterHangUp),
        ("hello-on-evaluate", Fault::HelloOnEvaluate),
        ("frame-after-close", Fault::FrameAfterClose),
        ("reply-after-close", Fault::ReplyAfterClose),
    ] {
        assert_eq!(Fault::named(name), Some(fault));
    }
    assert_eq!(Fault::named("crash"), None);
}

#[test]
fn a_worker_that_says_hello_after_its_app_hung_up_ends_as_the_hello_fails() {
    let (app, worker) = pair();
    let serving = std::thread::spawn(move || serve(worker, Some(Fault::HelloAfterHangUp)));
    let mut fds = [rustix::event::PollFd::new(&app, rustix::event::PollFlags::IN)];
    let wait = rustix::event::Timespec {
        tv_sec: 0,
        tv_nsec: 200_000_000,
    };
    assert_eq!(
        rustix::event::poll(&mut fds, Some(&wait)).unwrap(),
        0,
        "a hello before the hang-up"
    );
    drop(app);
    assert_eq!(serving.join().unwrap(), 0);
}

#[test]
fn a_worker_that_answers_an_evaluation_with_a_hello_ends_with_its_app() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_that_answers_an_evaluation_with_a_hello_ends_with_its_app",
        || {
            let (app, serving) = faulty_worker(Some(Fault::HelloOnEvaluate));
            let evaluate = ToWorker::Evaluate {
                id: 1,
                view: crate::ViewId::from_raw(1),
                script: "1".to_owned(),
                world: None,
            };
            wire::send(app.as_fd(), &evaluate.packet(None).unwrap()).unwrap();
            assert!(matches!(answer(&app), FromWorker::Hello { .. }));
            drop(app);
            assert_eq!(serving.join().unwrap(), 0);
        },
    );
}

#[test]
fn a_worker_sends_a_frame_of_a_page_after_its_close() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_sends_a_frame_of_a_page_after_its_close",
        || {
            let (app, serving) = faulty_worker(Some(Fault::FrameAfterClose));
            let view = crate::ViewId::from_raw(3);
            let (close, blob) = ToWorker::command(Command::CloseView(view));
            wire::send(app.as_fd(), &close.packet(blob).unwrap()).unwrap();
            let FromWorker::ShmFrame { frame, regions } = answer(&app) else {
                panic!("a frame")
            };
            assert_eq!(frame.view(), view);
            assert!(regions.is_empty());
            let (quit, blob) = ToWorker::command(Command::Quit);
            wire::send(app.as_fd(), &quit.packet(blob).unwrap()).unwrap();
            assert!(matches!(answer(&app), FromWorker::Stopped));
            assert_eq!(serving.join().unwrap(), 0);
        },
    );
}

#[test]
fn a_worker_holding_replies_until_a_close_serves_on() {
    crate::tests::isolated(
        "worker::serve::tests::a_worker_holding_replies_until_a_close_serves_on",
        || {
            let (app, serving) = faulty_worker(Some(Fault::ReplyAfterClose));
            for command in [
                Command::Reply(crate::ReplyToken::from_raw(7), Ok("null".to_owned())),
                Command::CloseView(crate::ViewId::from_raw(3)),
                Command::Quit,
            ] {
                let (message, blob) = ToWorker::command(command);
                wire::send(app.as_fd(), &message.packet(blob).unwrap()).unwrap();
            }
            assert!(matches!(answer(&app), FromWorker::Stopped));
            assert_eq!(serving.join().unwrap(), 0);
        },
    );
}
