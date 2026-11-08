use std::io::{self, Read as _, Write as _};
use std::time::Duration;

use interprocess::local_socket::traits::Stream as _;

use super::*;
use crate::endpoint::Endpoint;
use crate::protocol::{DelegatedReply, ExitStatus, GridSize, ReplyMetadata, SessionSpec};
use crate::server::ServerConfig;

const SIZE: GridSize = GridSize { cols: 40, rows: 6 };
const WAIT: Duration = Duration::from_secs(5);

fn pair() -> (tempfile::TempDir, Arc<Shared>, Arc<Stream>, Stream) {
    let root = tempfile::Builder::new().prefix("ngat-").tempdir_in("/tmp").unwrap();
    let endpoint = Endpoint::new(root.path());
    let listener = endpoint.bind().unwrap();
    let client = endpoint.connect().unwrap();
    let server = Arc::new(listener.accept().unwrap());
    super::super::set_recv_timeout(&server, Some(WAIT));
    super::super::set_recv_timeout(&client, Some(WAIT));
    let shared = Shared::new(&endpoint, &ServerConfig::default());
    (root, shared, server, client)
}

fn request(create: bool) -> AttachRequest {
    AttachRequest {
        name: "test".to_owned(),
        create: create.then(|| SessionSpec {
            argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), "read line".to_owned()],
            env: Vec::new(),
            cwd: None,
            tracked_env: Vec::new(),
        }),
        size: SIZE,
        managed: None,
        metadata: ReplyMetadata::default(),
    }
}

/// Reap the test's private pane even when an assertion unwinds.
struct Running(Arc<Pane>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.kill(true).unwrap();
        assert!(self.0.wait_exit(WAIT).is_some());
    }
}

#[test]
fn a_connection_must_start_with_attach_and_cannot_attach_twice() {
    let (_root, shared, server, mut client) = pair();
    let connection = shared.register(&server, 0);
    write_message(&mut client, &AttachClientMessage::Detach).unwrap();
    serve(&server, &shared, &connection).unwrap();
    assert!(shared.panes.is_empty());
    let (pane, attached) = open(&shared, &request(true)).unwrap();
    let running = Running(pane);
    let error = take_message(
        &running.0,
        attached.subscription.id(),
        AttachClientMessage::Attach(request(false)),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "malformed payload: a second attach on an attached connection"
    );
    assert!(!take_message(&running.0, attached.subscription.id(), AttachClientMessage::Detach).unwrap());
}

#[test]
fn an_attach_admitted_before_handoff_creates_no_pane() {
    let (_root, shared, server, mut client) = pair();
    let connection = shared.register(&server, 0);
    // Advance the generation before the already decoded request can be operated on.
    shared.generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    write_message(&mut client, &AttachClientMessage::Attach(request(true))).unwrap();
    serve(&server, &shared, &connection).unwrap();
    assert!(shared.panes.is_empty());
}

#[test]
fn a_stopping_server_refuses_creation_without_counting_a_session() {
    let (_root, shared, _server, _client) = pair();
    shared.lock().hand_over();
    let Err(refusal) = open(&shared, &request(true)) else {
        panic!("a stopped server created a pane")
    };
    assert_eq!(
        refusal,
        Refusal::new(
            AttachRefusal::CreateFailed,
            "the ngterm-session server is stopping; connect again"
        )
    );
    assert_eq!(shared.lock().sessions(), 0);
    assert!(shared.panes.is_empty());
}

#[test]
fn a_pane_without_a_snapshot_cut_is_refused_as_unavailable() {
    let (_root, shared, _server, _client) = pair();
    let mut config = PaneConfig::new("test", &shared.namespace, request(true).create.unwrap(), SIZE);
    config.snapshot_wait = Duration::ZERO;
    let (pane, _attached) = shared.panes.spawn(config).unwrap();
    let running = Running(pane);
    running.0.print(b"text").unwrap();
    running
        .0
        .with_terminal(|terminal| terminal.set_continuation_limit(0).unwrap());
    let Err(refusal) = open(&shared, &request(false)) else {
        panic!("attached without a consistent cut")
    };
    assert_eq!(refusal.reason, AttachRefusal::Unavailable);
    assert_eq!(refusal.message, snapshot::SnapshotError::Unavailable.to_string());
}

#[test]
fn stale_replies_and_input_from_a_detached_client_leave_the_connection_usable() {
    let (_root, shared, _server, _client) = pair();
    let (pane, attached) = open(&shared, &request(true)).unwrap();
    let running = Running(pane);
    let id = attached.subscription.id();
    assert!(running.0.detach(id));
    for message in [
        AttachClientMessage::Input {
            data: b"ignored".to_vec(),
        },
        AttachClientMessage::DelegatedReply {
            id: 999,
            epoch: 999,
            reply: DelegatedReply::Denied,
        },
        AttachClientMessage::KittyComplete {
            id: 999,
            epoch: 999,
            data: b"ignored".to_vec(),
        },
        AttachClientMessage::Resize {
            size: GridSize { cols: 10, rows: 10 },
        },
        AttachClientMessage::Metadata(ReplyMetadata::default()),
    ] {
        assert!(take_message(&running.0, id, message).unwrap());
    }
    assert_eq!(running.0.size(), SIZE);
    assert_eq!(running.0.clients(), 0);
    assert_eq!(running.0.exit_status(), None);
}

#[test]
fn malformed_frames_on_an_attached_connection_are_reported() {
    let (_root, shared, server, mut client) = pair();
    let connection = shared.register(&server, 0);
    let (pane, attached) = open(&shared, &request(true)).unwrap();
    let running = Running(pane);
    client.write_all(b"not a frame header at all").unwrap();
    crate::endpoint::close(&client);
    assert!(matches!(
        read_messages(&server, &running.0, attached.subscription.id(), &connection),
        Err(FramingError::BadMagic([110, 111, 116, 32]))
    ));
}

#[test]
fn a_writer_of_a_closed_client_detaches_its_subscription() {
    let (_root, shared, server, client) = pair();
    let (pane, attached) = open(&shared, &request(true)).unwrap();
    let running = Running(pane);
    crate::endpoint::disconnect(&client);
    write_events(&server, &running.0, attached.subscription);
    assert_eq!(running.0.clients(), 0);
}

#[test]
fn a_snapshot_send_failure_is_reported_instead_of_sending_its_end() {
    let (_root, _shared, server, client) = pair();
    crate::endpoint::disconnect(&client);
    assert!(
        send_event(
            &server,
            ClientEvent::Snapshot {
                epoch: 4,
                offset: 7,
                size: SIZE,
                data: b"contents".to_vec(),
            }
        )
        .is_err()
    );
}

#[test]
fn exit_and_overflow_are_sent_only_by_the_writer() {
    let (_root, _shared, server, mut client) = pair();
    send_event(&server, ClientEvent::Overflowed { offset: 9 }).unwrap();
    send_event(&server, ClientEvent::Exit(ExitStatus::Unknown)).unwrap();
    client.set_nonblocking(true).unwrap();
    let mut byte = [0];
    assert_eq!(client.read(&mut byte).unwrap_err().kind(), io::ErrorKind::WouldBlock);
}
