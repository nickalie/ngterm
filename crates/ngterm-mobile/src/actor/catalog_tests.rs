//! A headless machine's catalog on the actor: following it once a listing says the machine is headless, its rows
//! reported live, a fresh snapshot after a gap or a lost stream, and New Session, Rename Session and End Session sent
//! under an operation id. The actor's loop runs on the test's own `LocalSet` with tokio's paused clock.

use std::time::Duration;

use ngterm_core::catalog::frames::{self, CatalogChange, CatalogFrame, CatalogHello};
use ngterm_core::live::daemon_name;
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::origin::OriginShell;
use ngterm_core::remote::origin_replies::{READY_BUDGET, rename_session_arguments};
use ngterm_core::remote::ssh::{catalog_tail, control_tail, control_tail_with_operands, tree_tail};
use ngterm_proto::CatalogVersions;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::Command;
use super::pane_tests::SIZE;
use super::presentation_tests::{Actor, local, settle};
use crate::api::{MobileError, MobileEvent};
use crate::catalog::CatalogState;
use crate::catalog::operations::unknown_outcome;
use crate::catalog::tests::{EPOCH, ended, id, page, snapshot, terminal, versions, workspace};
use crate::sessions::RemoteSession;
use crate::transport::fake::{Opened, Script};
use crate::transport::{ChannelEnd, ChannelInput, ChannelPeer};

/// A reported row as name, workspace, page kind and ended.
type Row = (String, String, Option<String>, bool);

fn main_id() -> String {
    uuid_string(id(1))
}

/// A listing of the machine's main session, from a headless ngterm when `headless`.
fn listing(headless: bool) -> Script {
    let mut remote = json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "presentation": 1,
        "sessions": [{
            "id": main_id(), "name": "main", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": "ngterm-live-0"}],
            "status": "Working",
        }],
    });
    if headless {
        remote["headless"] = true.into();
    }
    Script::ok(&json!({"ok": true, "result": {"remote": remote}}).to_string())
}

fn version_line() -> String {
    control_tail(OriginShell::Posix, &["version"])
}

fn version_answer(versions: Option<CatalogVersions>) -> Script {
    let mut result = json!({
        "app": {"version": "0.2.0"},
        "headless": true,
        "conformance": {"parityLevel": "0.42.3", "capabilities": []},
    });
    if let Some(versions) = versions {
        result["versions"] = serde_json::to_value(versions).unwrap();
    }
    Script::ok(&json!({"ok": true, "result": result}).to_string())
}

/// The catalog the stream first brings: a workspace with the main session, a page row and an ended session.
fn first_snapshot() -> CatalogFrame {
    CatalogFrame::Snapshot(snapshot(
        4,
        vec![
            workspace(9, "work", vec![terminal(1, "main", None), page(4, "docs", "url", None)]),
            workspace(8, "play", vec![ended(3, "old")]),
        ],
    ))
}

fn hello() -> CatalogFrame {
    CatalogFrame::Hello(CatalogHello {
        version: 1,
        epoch: Some(EPOCH.to_owned()),
        versions: Some(versions()),
        page_rows: None,
    })
}

async fn send(peer: &ChannelPeer, frame: &CatalogFrame) {
    assert!(peer.data(frames::encode(frame).unwrap()).await);
    settle().await;
}

/// What the core wrote down `peer`: its frames, and whether it closed the channel.
fn written(peer: &mut ChannelPeer) -> (Vec<CatalogFrame>, bool) {
    let (mut frames, mut closed) = (Vec::new(), false);
    while let Some(input) = peer.try_input() {
        match input {
            ChannelInput::Data(line) => frames.push(frames::decode(line.strip_suffix(b"\n").unwrap()).unwrap()),
            ChannelInput::Close => closed = true,
            other => panic!("unexpected input {other:?}"),
        }
    }
    (frames, closed)
}

/// The channels opened since the last call.
async fn opened(actor: &Actor) -> Vec<Opened> {
    settle().await;
    actor.transport.take_opened()
}

/// The catalog stream, opened since the last call, after its hello.
async fn stream(actor: &Actor) -> ChannelPeer {
    let mut opened = opened(actor).await;
    assert_eq!(opened.len(), 1, "one stream opened");
    let mut stream = opened.remove(0);
    assert_eq!(stream.command, catalog_tail(OriginShell::Posix));
    let (frames, closed) = written(&mut stream.peer);
    assert_eq!(
        frames,
        [CatalogFrame::Hello(CatalogHello {
            version: 1,
            page_rows: Some(true),
            ..CatalogHello::default()
        })]
    );
    assert!(!closed);
    stream.peer
}

/// A connected, listed headless machine whose catalog stream brought [`first_snapshot`].
async fn following(actor: &Actor) -> ChannelPeer {
    actor.transport.script(&tree_tail(OriginShell::Posix), listing(true));
    actor
        .transport
        .script(&version_line(), version_answer(Some(versions())));
    actor.connect().await;
    actor.list().await;
    let peer = stream(actor).await;
    send(&peer, &hello()).await;
    send(&peer, &first_snapshot()).await;
    peer
}

/// The rows of each `Tree` reported since the last take, as name, workspace, page kind and ended.
fn trees(events: &[MobileEvent]) -> Vec<Vec<Row>> {
    events
        .iter()
        .filter_map(|event| match event {
            MobileEvent::Tree { sessions, .. } => Some(
                sessions
                    .iter()
                    .map(|row| {
                        (
                            row.name.clone(),
                            row.workspace_name.clone(),
                            row.page.clone(),
                            row.ended,
                        )
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn states(events: &[MobileEvent]) -> Vec<CatalogState> {
    events
        .iter()
        .filter_map(|event| match event {
            MobileEvent::Catalog { state, .. } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

fn row(name: &str, workspace: &str, page: Option<&str>, ended: bool) -> Row {
    (name.to_owned(), workspace.to_owned(), page.map(str::to_owned), ended)
}

fn first_rows() -> Vec<Row> {
    vec![
        row("main", "work", None, false),
        row("docs", "work", Some("url"), false),
        row("old", "play", None, true),
    ]
}

/// The operation id in `command`: the first id after `--op-id`.
fn op_id_of(command: &str) -> String {
    let after = &command[command.find("--op-id").expect("an operation id")..];
    (0..after.len().saturating_sub(36))
        .find_map(|at| after.get(at..at + 36).filter(|word| Uuid::parse_str(word).is_ok()))
        .expect("an id")
        .to_owned()
}

async fn answer(peer: ChannelPeer, stdout: &Value) {
    assert!(peer.data(stdout.to_string().into_bytes()).await);
    peer.end(ChannelEnd::Exit {
        code: Some(0),
        signal: None,
    })
    .await;
    settle().await;
}

fn receipt(op_id: &str, outcome: &str, rev: Option<i64>) -> Value {
    json!({"opID": op_id, "epoch": EPOCH, "outcome": outcome, "rev": rev})
}

fn post_create(actor: &Actor, name: Option<&str>) -> oneshot::Receiver<Result<RemoteSession, MobileError>> {
    let (reply, answer) = oneshot::channel();
    actor.post(Command::CreateSession {
        machine_id: "m1".to_owned(),
        name: name.map(str::to_owned),
        reply,
    });
    answer
}

fn post_duplicate(actor: &Actor, session_id: &str) -> oneshot::Receiver<Result<RemoteSession, MobileError>> {
    let (reply, answer) = oneshot::channel();
    actor.post(Command::DuplicateSession {
        machine_id: "m1".to_owned(),
        session_id: session_id.to_owned(),
        reply,
    });
    answer
}

fn post_end(actor: &Actor, session_id: &str) -> oneshot::Receiver<Result<(), MobileError>> {
    let (reply, answer) = oneshot::channel();
    actor.post(Command::EndSession {
        machine_id: "m1".to_owned(),
        session_id: session_id.to_owned(),
        reply,
    });
    answer
}

fn post_rename(actor: &Actor, session_id: &str, name: &str) -> oneshot::Receiver<Result<(), MobileError>> {
    let (reply, answer) = oneshot::channel();
    actor.post(Command::RenameSession {
        machine_id: "m1".to_owned(),
        session_id: session_id.to_owned(),
        name: name.to_owned(),
        reply,
    });
    answer
}

async fn attach(actor: &Actor, session_id: &str) -> Result<(), MobileError> {
    let (reply, answer) = oneshot::channel();
    actor.post(Command::AttachSession {
        machine_id: "m1".to_owned(),
        session_id: session_id.to_owned(),
        reply,
    });
    answer.await.unwrap()
}

/// The one operation channel opened since the last call: its command's operation id, and its far end.
async fn operation(actor: &Actor) -> (String, String, ChannelPeer) {
    let mut opened = opened(actor).await;
    assert_eq!(opened.len(), 1, "one operation sent");
    let opened = opened.remove(0);
    (op_id_of(&opened.command), opened.command, opened.peer)
}

// MARK: - following

#[test]
fn a_headless_machine_lists_its_catalog_live_with_page_and_ended_rows() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let events = actor.sink.take();
        assert_eq!(states(&events), [CatalogState::Connecting, CatalogState::Connected]);
        assert_eq!(trees(&events).last(), Some(&first_rows()));

        let mut renamed = terminal(1, "build", None).fields;
        renamed.custom_name = Some("build".to_owned());
        let rename = CatalogFrame::Delta {
            epoch: EPOCH.to_owned(),
            rev: 5,
            change: CatalogChange::SessionChanged(renamed),
        };
        send(&peer, &rename).await;
        let events = actor.sink.take();
        assert_eq!(trees(&events)[0][0], row("build", "work", None, false));
        let MobileEvent::Tree { sessions, hint, .. } = &events[0] else {
            panic!("a tree: {events:?}")
        };
        assert_eq!(
            sessions[0].status.as_deref(),
            Some("Working"),
            "the listing's status stays"
        );
        assert_eq!(*hint, None);
    });
}

#[test]
fn a_ping_is_acked_and_a_listing_reports_the_catalogs_rows() {
    local(async {
        let actor = Actor::start();
        let mut peer = following(&actor).await;
        actor.sink.take();
        send(&peer, &CatalogFrame::Ping).await;
        assert_eq!(written(&mut peer), (vec![CatalogFrame::Ack], false));

        actor.list().await;
        assert_eq!(trees(&actor.sink.take()), [first_rows()]);
        assert!(opened(&actor).await.is_empty(), "the stream already runs");
    });
}

#[test]
fn a_machine_with_windows_keeps_its_listing_and_opens_no_stream() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(OriginShell::Posix), listing(false));
        actor.connect().await;
        actor.list().await;
        settle().await;
        let events = actor.sink.take();
        assert!(states(&events).is_empty());
        assert_eq!(trees(&events), [vec![row("main", "work", None, false)]]);
        assert!(actor.transport.take_opened().is_empty());
        let commands: Vec<String> = actor.transport.execs().into_iter().map(|exec| exec.command).collect();
        assert_eq!(commands, [tree_tail(OriginShell::Posix)]);
    });
}

#[test]
fn a_headless_machine_without_a_catalog_is_refused_for_good_and_keeps_its_listing() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(OriginShell::Posix), listing(true));
        actor.transport.script(&version_line(), version_answer(None));
        actor.connect().await;
        actor.list().await;
        settle().await;
        let events = actor.sink.take();
        assert_eq!(
            states(&events),
            [
                CatalogState::Connecting,
                CatalogState::Refused {
                    reason: "box runs an ngterm without a catalog; a live session list needs ngterm-headless"
                        .to_owned()
                }
            ]
        );
        assert!(
            trees(&events)
                .iter()
                .all(|rows| *rows == [row("main", "work", None, false)])
        );
        assert!(actor.transport.take_opened().is_empty());

        actor.list().await;
        settle().await;
        assert!(states(&actor.sink.take()).is_empty(), "nothing probes it again");
        let probes = actor
            .transport
            .execs()
            .into_iter()
            .filter(|exec| exec.command == version_line())
            .count();
        assert_eq!(probes, 1);
    });
}

#[test]
fn a_gap_subscribes_again_and_the_fresh_snapshot_replaces_the_rows() {
    local(async {
        let actor = Actor::start();
        let mut peer = following(&actor).await;
        actor.sink.take();
        let gap = CatalogFrame::Delta {
            epoch: EPOCH.to_owned(),
            rev: 9,
            change: CatalogChange::SessionRemoved(id(1)),
        };
        send(&peer, &gap).await;
        assert!(written(&mut peer).1, "the old stream is closed");
        let fresh = stream(&actor).await;
        send(&fresh, &hello()).await;
        let replaced = snapshot(9, vec![workspace(9, "work", vec![terminal(2, "next", None)])]);
        send(&fresh, &CatalogFrame::Snapshot(replaced)).await;
        let events = actor.sink.take();
        assert_eq!(states(&events), [CatalogState::Connecting, CatalogState::Connected]);
        assert_eq!(trees(&events), [vec![row("next", "work", None, false)]]);
        let probes = actor
            .transport
            .execs()
            .into_iter()
            .filter(|exec| exec.command == version_line())
            .count();
        assert_eq!(probes, 1, "a resubscription asks no version");
    });
}

#[test]
fn a_lost_stream_keeps_the_rows_and_retries_from_a_fresh_snapshot() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        actor.sink.take();
        assert!(peer.stderr(b"no catalog here\n".to_vec()).await);
        peer.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        let events = actor.sink.take();
        assert_eq!(
            states(&events),
            [CatalogState::Disconnected {
                reason: "the catalog stream from box ended: no catalog here".to_owned()
            }]
        );
        assert!(trees(&events).is_empty(), "the rows stay");

        tokio::time::sleep(Duration::from_millis(900)).await;
        assert!(opened(&actor).await.is_empty(), "the first retry waits a second");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let fresh = stream(&actor).await;
        send(&fresh, &hello()).await;
        send(&fresh, &first_snapshot()).await;
        let events = actor.sink.take();
        assert_eq!(states(&events), [CatalogState::Connecting, CatalogState::Connected]);
        assert_eq!(trees(&events), [first_rows()]);
    });
}

#[test]
fn a_silent_stream_is_dropped_as_lost() {
    local(async {
        let actor = Actor::start();
        let mut peer = following(&actor).await;
        actor.sink.take();
        tokio::time::sleep(Duration::from_millis(35_500)).await;
        settle().await;
        assert_eq!(
            states(&actor.sink.take()),
            [CatalogState::Disconnected {
                reason: "no word from box for 35 seconds".to_owned()
            }]
        );
        assert!(written(&mut peer).1);
    });
}

#[test]
fn a_lost_connection_shows_disconnected_and_its_reconnect_follows_again() {
    local(async {
        let actor = Actor::start();
        let _peer = following(&actor).await;
        actor.sink.take();
        actor.transport.drop_connections("m1");
        settle().await;
        let events = actor.sink.take();
        assert_eq!(
            states(&events),
            [CatalogState::Disconnected {
                reason: "the connection to box was lost".to_owned()
            }]
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        let fresh = stream(&actor).await;
        send(&fresh, &hello()).await;
        send(&fresh, &first_snapshot()).await;
        assert_eq!(
            states(&actor.sink.take()),
            [CatalogState::Connecting, CatalogState::Connected]
        );
    });
}

// MARK: - operations

#[test]
fn new_session_carries_an_operation_id_and_answers_with_the_row_the_catalog_shows() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let answered = post_create(&actor, Some("build"));
        let (op_id, command, op) = operation(&actor).await;
        assert!(command.contains(r"'\''--name=build'\'' '\''--op-id'\''"), "{command}");
        assert!(
            command.contains(&format!(r"'\''--epoch'\'' '\''{EPOCH}'\''")),
            "{command}"
        );
        let created = uuid_string(id(7));
        answer(
            op,
            &json!({"ok": true, "result": {"id": created, "receipt": receipt(&op_id, "applied", Some(5))}}),
        )
        .await;

        let added = CatalogFrame::Delta {
            epoch: EPOCH.to_owned(),
            rev: 5,
            change: CatalogChange::SessionAdded {
                session: terminal(7, "build", None),
                workspace: id(9),
                index: 2,
            },
        };
        send(&peer, &added).await;
        let row = answered.await.unwrap().expect("created");
        assert_eq!((row.id.as_str(), row.name.as_str()), (created.as_str(), "build"));
        let commands: Vec<String> = actor.transport.execs().into_iter().map(|exec| exec.command).collect();
        assert!(
            !commands.iter().any(|command| command.contains("restore")),
            "no restore-mode check and no polling: {commands:?}"
        );
        assert_eq!(
            attach(&actor, &created).await,
            Ok(()),
            "the catalog's row attaches before a listing shows it"
        );
    });
}

/// New Session through `actor`'s catalog, whose machine then answers its listings with `listings` in turn: the
/// created row and the answer to a show of its pane.
async fn create_and_show(
    actor: &Actor,
    peer: &ChannelPeer,
    listings: Vec<Script>,
) -> (RemoteSession, Result<(), MobileError>) {
    let created = uuid_string(id(7));
    actor
        .transport
        .script_sequence(&tree_tail(OriginShell::Posix), listings);
    let answered = post_create(actor, Some("build"));
    let (op_id, _, op) = operation(actor).await;
    answer(
        op,
        &json!({"ok": true, "result": {"id": created, "receipt": receipt(&op_id, "applied", Some(5))}}),
    )
    .await;
    let added = CatalogFrame::Delta {
        epoch: EPOCH.to_owned(),
        rev: 5,
        change: CatalogChange::SessionAdded {
            session: terminal(7, "build", None),
            workspace: id(9),
            index: 2,
        },
    };
    send(peer, &added).await;
    let row = answered.await.unwrap().expect("created");
    let (reply, shown) = oneshot::channel();
    actor.post(Command::ShowPane {
        machine_id: "m1".to_owned(),
        session_id: created,
        pane_id: uuid_string(made_pane()),
        size: SIZE,
        reply,
    });
    (row, shown.await.unwrap().map(drop))
}

#[test]
fn a_new_session_the_catalog_made_shows_its_pane() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let (row, shown) = create_and_show(&actor, &peer, vec![listing_with(&uuid_string(id(7)), "build")]).await;
        assert_eq!(row.name, "build");
        assert_eq!(shown, Ok(()), "the new session's pane shows");
    });
}

/// The machine lists a session only once each of its panes' daemons runs, which can be after the catalog shows it.
#[test]
fn a_new_session_the_machine_lists_after_the_catalog_shows_it_shows_its_pane() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let listings = vec![listing(true), listing_with(&uuid_string(id(7)), "build")];
        let (row, shown) = create_and_show(&actor, &peer, listings).await;
        assert_eq!(row.name, "build");
        assert_eq!(shown, Ok(()), "the new session's pane shows");
    });
}

#[test]
fn a_new_session_that_ended_at_once_answers_with_no_listing() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let tail = tree_tail(OriginShell::Posix);
        actor.transport.script(&tail, Script::failing(1, "boom"));
        let listings = || {
            actor
                .transport
                .execs()
                .iter()
                .filter(|exec| exec.command == tail)
                .count()
        };
        let before = listings();
        let answered = post_create(&actor, Some("build"));
        let (op_id, _, op) = operation(&actor).await;
        let created = uuid_string(id(7));
        answer(
            op,
            &json!({"ok": true, "result": {"id": created, "receipt": receipt(&op_id, "applied", Some(5))}}),
        )
        .await;
        let added = CatalogFrame::Delta {
            epoch: EPOCH.to_owned(),
            rev: 5,
            change: CatalogChange::SessionAdded {
                session: ended(7, "build"),
                workspace: id(9),
                index: 2,
            },
        };
        let started = tokio::time::Instant::now();
        send(&peer, &added).await;
        let row = answered.await.unwrap().expect("created");
        assert!(row.ended, "{row:?}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "it does not wait for a listing"
        );
        assert_eq!(listings(), before, "no listing");
    });
}

#[test]
fn a_new_session_whose_listing_fails_still_answers_with_its_row() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let listings = || {
            let tail = tree_tail(OriginShell::Posix);
            actor
                .transport
                .execs()
                .iter()
                .filter(|exec| exec.command == tail)
                .count()
        };
        let before = listings();
        let started = tokio::time::Instant::now();
        let (row, shown) = create_and_show(&actor, &peer, vec![Script::failing(1, "boom")]).await;
        assert!(started.elapsed() <= READY_BUDGET, "it answers within the budget");
        let commands: Vec<String> = actor.transport.execs().into_iter().map(|exec| exec.command).collect();
        assert!(listings() - before > 1, "it lists again: {commands:#?}");
        assert_eq!(row.name, "build", "the catalog made it");
        assert!(shown.is_err(), "no listing holds its pane");
        assert_eq!(
            trees(&actor.sink.take()).last().map(Vec::len),
            Some(4),
            "the catalog still shows it"
        );
    });
}

#[test]
fn a_new_session_the_catalog_never_shows_attachable_is_reported_created() {
    local(async {
        let actor = Actor::start();
        let _peer = following(&actor).await;
        let answered = post_create(&actor, None);
        let (op_id, _, op) = operation(&actor).await;
        let created = uuid_string(id(7));
        answer(
            op,
            &json!({"ok": true, "result": {"id": created, "receipt": receipt(&op_id, "applied", Some(5))}}),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(19_500)).await;
        settle().await;
        let mut answered = answered;
        assert!(answered.try_recv().is_err(), "it waits out the budget");
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Err(MobileError::Remote { reason }) = answered.await.unwrap() else {
            panic!("not attachable")
        };
        assert_eq!(
            reason,
            format!(
                "created session {created} on box but could not attach it: it did not become attachable; attach it \
                 from the session list once it shows there"
            )
        );
    });
}

#[test]
fn end_session_whose_reply_is_lost_asks_once_and_reports_an_unknown_outcome() {
    local(async {
        let actor = Actor::start();
        let _peer = following(&actor).await;
        let answered = post_end(&actor, &main_id());
        let (op_id, command, op) = operation(&actor).await;
        assert!(
            command.contains(&format!(r"'\''close'\'' '\''--target'\'' '\''{}'\''", main_id())),
            "{command}"
        );
        op.end(ChannelEnd::Lost("gone".to_owned())).await;

        let mut opened = opened(&actor).await;
        assert_eq!(opened.len(), 1, "one lookup");
        let lookup = opened.remove(0);
        assert_eq!(
            lookup.command,
            control_tail(
                OriginShell::Posix,
                &["catalog", "op", &op_id, "--epoch", EPOCH, "--rev", "4"]
            )
        );
        lookup
            .peer
            .end(ChannelEnd::Exit {
                code: Some(1),
                signal: None,
            })
            .await;
        assert_eq!(
            answered.await.unwrap(),
            Err(MobileError::Remote {
                reason: unknown_outcome("box", "End Session")
            })
        );
        assert_eq!(
            unknown_outcome("box", "End Session"),
            "box did not answer End Session; its outcome is unknown until box can be asked"
        );
    });
}

#[test]
fn a_lookup_that_finds_the_end_applied_answers_once_the_catalog_drops_the_row() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        let answered = post_end(&actor, &main_id());
        let (op_id, _, op) = operation(&actor).await;
        answer(op, &json!({"garbled": true})).await;
        let (_, _, lookup) = {
            let mut opened = opened(&actor).await;
            let lookup = opened.remove(0);
            (String::new(), lookup.command, lookup.peer)
        };
        answer(
            lookup,
            &json!({"ok": true, "result": {"receipt": receipt(&op_id, "applied", Some(5))}}),
        )
        .await;
        actor.sink.take();
        let removed = CatalogFrame::Delta {
            epoch: EPOCH.to_owned(),
            rev: 5,
            change: CatalogChange::SessionRemoved(id(1)),
        };
        send(&peer, &removed).await;
        assert_eq!(answered.await.unwrap(), Ok(()));
        let reported = trees(&actor.sink.take());
        assert_eq!(
            reported.last(),
            Some(&vec![
                row("docs", "work", Some("url"), false),
                row("old", "play", None, true)
            ])
        );
    });
}

#[test]
fn a_refused_rename_reports_the_machines_words_and_a_fenced_one_says_it_did_not_run() {
    local(async {
        let actor = Actor::start();
        let _peer = following(&actor).await;
        let answered = post_rename(&actor, &main_id(), "-weird");
        let (op_id, command, op) = operation(&actor).await;
        assert!(command.ends_with(r"'\''--'\'' '\''-weird'\'''"), "{command}");
        answer(
            op,
            &json!({"ok": false, "error": "the catalog is at epoch E2; take its snapshot and send the command again",
                    "result": {"receipt": receipt(&op_id, "refused", None)}}),
        )
        .await;
        assert_eq!(
            answered.await.unwrap(),
            Err(MobileError::Remote {
                reason: "the catalog is at epoch E2; take its snapshot and send the command again".to_owned()
            })
        );

        let answered = post_rename(&actor, &main_id(), "x");
        let (op_id, _, op) = operation(&actor).await;
        answer(
            op,
            &json!({"ok": true, "result": {"receipt": receipt(&op_id, "fenced", None)}}),
        )
        .await;
        assert_eq!(
            answered.await.unwrap(),
            Err(MobileError::Remote {
                reason: "box did not run Rename Session".to_owned()
            })
        );

        let answered = post_rename(&actor, &main_id(), "same");
        let (op_id, _, op) = operation(&actor).await;
        answer(
            op,
            &json!({"ok": true, "result": {"receipt": receipt(&op_id, "noop", None)}}),
        )
        .await;
        assert_eq!(answered.await.unwrap(), Ok(()));
    });
}

#[test]
fn a_page_row_and_an_ended_row_are_never_attached() {
    local(async {
        let actor = Actor::start();
        let _peer = following(&actor).await;
        assert_eq!(
            attach(&actor, &uuid_string(id(4))).await,
            Err(MobileError::Remote {
                reason: "docs is a page on box; this app shows terminal sessions only".to_owned()
            })
        );
        assert_eq!(
            attach(&actor, &uuid_string(id(3))).await,
            Err(MobileError::Remote {
                reason: "the session has ended on box".to_owned()
            })
        );
        assert!(opened(&actor).await.is_empty());
    });
}

#[test]
fn operations_on_a_machine_with_windows_run_plain_and_a_rename_reaches_its_listing() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(OriginShell::Posix), listing(false));
        let rename = control_tail_with_operands(OriginShell::Posix, &rename_session_arguments(&main_id()), &["build"]);
        actor.transport.script(&rename, Script::ok(r#"{"ok":true}"#));
        actor.connect().await;
        actor.list().await;
        actor.sink.take();
        let answered = post_rename(&actor, &main_id(), "build");
        settle().await;
        assert_eq!(answered.await.unwrap(), Ok(()));
        assert_eq!(trees(&actor.sink.take()), [vec![row("build", "work", None, false)]]);
        assert!(actor.transport.take_opened().is_empty(), "no operation id");

        let refused = r#"{"ok":false,"error":"no such session"}"#;
        actor.transport.script(&rename, Script::ok(refused));
        assert_eq!(
            post_rename(&actor, &main_id(), "build").await.unwrap(),
            Err(MobileError::Remote {
                reason: "no such session".to_owned()
            })
        );
        assert_eq!(
            post_rename(&actor, "not a token", "x").await.unwrap(),
            Err(MobileError::Remote {
                reason: ngterm_core::remote::tree::INVALID_REMOTE_SESSION.to_owned()
            })
        );
    });
}

#[test]
fn a_disconnect_forgets_the_catalog() {
    local(async {
        let actor = Actor::start();
        let _peer = following(&actor).await;
        actor.post(Command::Disconnect {
            machine_id: "m1".to_owned(),
        });
        settle().await;
        actor.sink.take();
        actor.connect().await;
        settle().await;
        assert!(opened(&actor).await.is_empty(), "only a new listing follows it again");
        assert!(states(&actor.sink.take()).is_empty());
    });
}

#[test]
fn an_unreadable_frame_subscribes_again_and_a_stream_hello_of_another_catalog_refuses() {
    local(async {
        let actor = Actor::start();
        let mut peer = following(&actor).await;
        actor.sink.take();
        assert!(peer.data(b"not json\n".to_vec()).await);
        settle().await;
        assert!(written(&mut peer).1, "the old stream is closed");
        let fresh = stream(&actor).await;
        let newer = CatalogFrame::Hello(CatalogHello {
            version: 1,
            epoch: Some(EPOCH.to_owned()),
            versions: Some(CatalogVersions {
                catalog: 9,
                ..versions()
            }),
            page_rows: None,
        });
        send(&fresh, &newer).await;
        let events = actor.sink.take();
        assert_eq!(
            states(&events),
            [
                CatalogState::Connecting,
                CatalogState::Refused {
                    reason: "box speaks catalog 9, this app speaks catalog 1".to_owned()
                }
            ]
        );
        assert_eq!(
            trees(&events),
            [vec![row("main", "work", None, false)]],
            "back to the listing"
        );
        assert!(opened(&actor).await.is_empty());
    });
}

#[test]
fn a_version_probe_without_an_answer_retries_and_a_stream_lost_by_the_link_names_why() {
    local(async {
        let actor = Actor::start();
        actor.transport.script(&tree_tail(OriginShell::Posix), listing(true));
        actor.transport.script_sequence(
            &version_line(),
            vec![Script::failing(255, ""), version_answer(Some(versions()))],
        );
        actor.connect().await;
        actor.list().await;
        settle().await;
        assert_eq!(
            states(&actor.sink.take()),
            [
                CatalogState::Connecting,
                CatalogState::Disconnected {
                    reason: "box did not answer".to_owned()
                }
            ]
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let peer = stream(&actor).await;
        peer.end(ChannelEnd::Lost("reset by peer".to_owned())).await;
        settle().await;
        assert_eq!(
            states(&actor.sink.take()),
            [
                CatalogState::Connecting,
                CatalogState::Disconnected {
                    reason: "the catalog stream from box ended: reset by peer".to_owned()
                }
            ]
        );
    });
}

#[test]
fn a_plain_end_while_the_stream_is_down_drops_the_row_it_shows() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        peer.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        actor.sink.take();
        let end = control_tail(OriginShell::Posix, &["session", "close", "--target", &main_id()]);
        actor.transport.script(&end, Script::ok(r#"{"ok":true}"#));
        assert_eq!(post_end(&actor, &main_id()).await.unwrap(), Ok(()));
        assert_eq!(
            trees(&actor.sink.take()).last(),
            Some(&vec![
                row("docs", "work", Some("url"), false),
                row("old", "play", None, true)
            ])
        );
    });
}

#[test]
fn a_machine_running_both_apps_keeps_agterms_sessions_beside_the_catalog() {
    local(async {
        let actor = Actor::start();
        let Script { stdout, .. } = listing(true);
        let agterm = json!({"ok": true, "result": {"remote": {
            "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/zmx"},
            "presentation": 1,
            "sessions": [{
                "id": uuid_string(id(0xA6)), "name": "theirs", "windowID": "w1", "windowName": "main",
                "workspaceID": "ws1", "workspaceName": "mac", "cwd": "/repo",
                "panes": [{"pane": "left", "daemon": "agterm-00000000000000000000000000000000"}],
            }],
        }}});
        let both = format!(
            "{}\n{}\n{agterm}\n",
            String::from_utf8(stdout).unwrap(),
            ngterm_core::remote::peer::AGTERM_ANSWER
        );
        actor
            .transport
            .script(&tree_tail(OriginShell::Posix), Script::ok(&both));
        actor
            .transport
            .script(&version_line(), version_answer(Some(versions())));
        actor.connect().await;
        actor.list().await;
        let peer = stream(&actor).await;
        send(&peer, &hello()).await;
        send(&peer, &first_snapshot()).await;
        let mut expected = first_rows();
        expected.push(row("theirs", "mac", None, false));
        assert_eq!(trees(&actor.sink.take()).last(), Some(&expected));

        let theirs = uuid_string(id(0xA6));
        let agterm_form = ngterm_core::remote::origin::OriginForm {
            peer: ngterm_proto::RemotePeer::Agterm,
            ..OriginShell::Posix.into()
        };
        let end = control_tail(agterm_form, &["session", "close", "--target", &theirs]);
        actor.transport.script(&end, Script::ok(r#"{"ok":true}"#));
        assert_eq!(post_end(&actor, &theirs).await.unwrap(), Ok(()));
        assert!(opened(&actor).await.is_empty(), "no operation id for agterm's session");
        assert_eq!(trees(&actor.sink.take()).last(), Some(&first_rows()));
    });
}

#[test]
fn a_plain_rename_while_the_stream_is_down_names_the_row_it_shows() {
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        peer.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        actor.sink.take();
        let rename = control_tail_with_operands(OriginShell::Posix, &rename_session_arguments(&main_id()), &["build"]);
        actor.transport.script(&rename, Script::ok(r#"{"ok":true}"#));
        assert_eq!(post_rename(&actor, &main_id(), "build").await.unwrap(), Ok(()));
        let mut expected = first_rows();
        expected[0] = row("build", "work", None, false);
        assert_eq!(trees(&actor.sink.take()).last(), Some(&expected));
    });
}

/// The pane of the session that [`listing_with`] adds.
fn made_pane() -> Uuid {
    Uuid::from_u128(0xb7)
}

/// A listing of the headless machine's main session and the session `made` named `name`.
fn listing_with(made: &str, name: &str) -> Script {
    let session = |id: &str, name: &str, daemon: &str| {
        json!({
            "id": id, "name": name, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
            "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon}],
        })
    };
    let remote = json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "presentation": 1,
        "headless": true,
        "sessions": [session(&main_id(), "main", "ngterm-live-0"), session(made, name, &daemon_name(made_pane()))],
    });
    Script::ok(&json!({"ok": true, "result": {"remote": remote}}).to_string())
}

#[test]
fn a_plain_new_session_while_the_stream_is_down_stays_listed_until_a_snapshot() {
    use ngterm_core::remote::origin_replies::{NEW_SESSION_ARGUMENTS, RESTORE_MODE_ARGUMENTS};
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        peer.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        actor.sink.take();
        let made = uuid_string(id(7));
        let live = json!({"ok": true, "result": {"restore": {"configured": "live", "requestedAtLaunch": "live",
            "active": "live", "restartRequired": false}}});
        actor.transport.script(
            &control_tail(OriginShell::Posix, &RESTORE_MODE_ARGUMENTS),
            Script::ok(&live.to_string()),
        );
        actor.transport.script(
            &control_tail(OriginShell::Posix, &NEW_SESSION_ARGUMENTS),
            Script::ok(&json!({"ok": true, "result": {"id": made}}).to_string()),
        );
        for tail in [
            tree_tail(OriginShell::Posix),
            ngterm_core::remote::ssh::tree_tail_for(OriginShell::Posix, Some(ngterm_proto::RemotePeer::Ngterm)),
        ] {
            actor.transport.script(&tail, listing_with(&made, "fresh"));
        }

        assert_eq!(post_create(&actor, None).await.unwrap().unwrap().id, made);
        let mut expected = first_rows();
        expected.push(row("fresh", "work", None, false));
        assert_eq!(trees(&actor.sink.take()).last(), Some(&expected));
        actor.list().await;
        assert_eq!(trees(&actor.sink.take()).last(), Some(&expected), "a refresh keeps it");

        tokio::time::sleep(Duration::from_millis(1100)).await;
        let fresh = stream(&actor).await;
        send(&fresh, &hello()).await;
        let mut held = first_snapshot();
        if let CatalogFrame::Snapshot(snapshot) = &mut held {
            snapshot.workspaces[0].sessions.push(terminal(7, "fresh", None));
        }
        send(&fresh, &held).await;
        let mut once = first_rows();
        once.insert(2, row("fresh", "work", None, false));
        assert_eq!(
            trees(&actor.sink.take()).last(),
            Some(&once),
            "shown once the catalog holds it"
        );
        send(&fresh, &CatalogFrame::Snapshot(snapshot(5, Vec::new()))).await;
        assert_eq!(
            trees(&actor.sink.take()).last(),
            Some(&Vec::new()),
            "the catalog's word after that"
        );
    });
}

#[test]
fn a_duplicate_while_the_stream_is_down_stays_listed() {
    use ngterm_core::remote::origin_replies::{RESTORE_MODE_ARGUMENTS, duplicate_session_arguments};
    local(async {
        let actor = Actor::start();
        let peer = following(&actor).await;
        peer.end(ChannelEnd::Exit {
            code: Some(1),
            signal: None,
        })
        .await;
        settle().await;
        actor.sink.take();
        let made = uuid_string(id(7));
        let live = json!({"ok": true, "result": {"restore": {"configured": "live", "requestedAtLaunch": "live",
            "active": "live", "restartRequired": false}}});
        actor.transport.script(
            &control_tail(OriginShell::Posix, &RESTORE_MODE_ARGUMENTS),
            Script::ok(&live.to_string()),
        );
        actor.transport.script(
            &control_tail(OriginShell::Posix, &duplicate_session_arguments(&main_id())),
            Script::ok(&json!({"ok": true, "result": {"id": made}}).to_string()),
        );
        for tail in [
            tree_tail(OriginShell::Posix),
            ngterm_core::remote::ssh::tree_tail_for(OriginShell::Posix, Some(ngterm_proto::RemotePeer::Ngterm)),
        ] {
            actor.transport.script(&tail, listing_with(&made, "main copy"));
        }

        assert_eq!(post_duplicate(&actor, &main_id()).await.unwrap().unwrap().id, made);
        let mut expected = first_rows();
        expected.push(row("main copy", "work", None, false));
        assert_eq!(trees(&actor.sink.take()).last(), Some(&expected));
        actor.list().await;
        assert_eq!(trees(&actor.sink.take()).last(), Some(&expected), "a refresh keeps it");
    });
}

#[test]
fn a_machine_running_both_apps_keeps_the_listings_hint_beside_the_catalog() {
    local(async {
        let actor = Actor::start();
        let Script { stdout, .. } = listing(true);
        let agterm = json!({"ok": true, "result": {"remote": {
            "endpoint": {"executable": "/Applications/agterm.app/Contents/MacOS/zmx", "socketDirectory": "/tmp/zmx"},
            "presentation": 1,
            "sessions": [],
        }}});
        let both = format!(
            "{}\n{}\n{agterm}\n",
            String::from_utf8(stdout).unwrap(),
            ngterm_core::remote::peer::AGTERM_ANSWER
        );
        actor
            .transport
            .script(&tree_tail(OriginShell::Posix), Script::ok(&both));
        let mode = json!({"ok": true, "result": {"restore": {
            "configured": "none", "requestedAtLaunch": "none", "active": "none", "restartRequired": false,
        }}});
        actor.transport.script(
            &ngterm_core::remote::not_live::restore_tail(),
            Script::ok(&mode.to_string()),
        );
        actor
            .transport
            .script(&version_line(), version_answer(Some(versions())));
        actor.connect().await;
        actor.list().await;
        let peer = stream(&actor).await;
        send(&peer, &hello()).await;
        actor.sink.take();
        send(&peer, &first_snapshot()).await;
        let events = actor.sink.take();
        let Some(MobileEvent::Tree { hint, .. }) = events
            .iter()
            .rev()
            .find(|event| matches!(event, MobileEvent::Tree { .. }))
        else {
            panic!("no tree in {events:?}");
        };
        assert_eq!(trees(&events).last(), Some(&first_rows()));
        assert_eq!(
            hint.as_deref(),
            Some(
                "agterm on box is not in Live sessions mode: set Settings > General > Restore sessions to Live \
                 sessions and restart agterm; only sessions started after that can be attached"
            )
        );
    });
}
