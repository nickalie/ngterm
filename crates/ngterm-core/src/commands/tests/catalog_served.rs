//! The catalog a headless origin serves (upstream's `ServerCatalogTests` and the catalog cases of
//! `ServerStreamsTests`): what each session projects to, the stream a `catalog.subscribe` connection becomes, and the
//! changes every commit publishes on it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ngterm_proto::command::{CatalogCmd, SessionCmd, WorkspaceCmd};
use ngterm_proto::{Args, Command, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::overlay_page::fixture_with_host;
use super::{Fixture, answered_id, args, wire};
use crate::LineChannel;
use crate::catalog::fixture::CATALOG;
use crate::catalog::frames::{self, CatalogChange, CatalogFrame, CatalogSession, CatalogSnapshot};

/// What the core sent a stream and whether it closed it.
#[derive(Default)]
pub(super) struct Wire {
    lines: RefCell<Vec<Vec<u8>>>,
    shut: Cell<bool>,
    /// The connection's queue is full: it takes no more lines.
    full: Cell<bool>,
}

impl Wire {
    pub(super) fn frames(&self) -> Vec<CatalogFrame> {
        self.lines
            .borrow()
            .iter()
            .map(|line| {
                assert_ne!(line.last(), Some(&b'\n'), "the channel adds the newline");
                frames::decode(line).unwrap()
            })
            .collect()
    }

    fn deltas(&self) -> Vec<(i64, CatalogChange)> {
        self.frames()
            .into_iter()
            .filter_map(|frame| match frame {
                CatalogFrame::Delta { rev, change, .. } => Some((rev, change)),
                _ => None,
            })
            .collect()
    }

    fn snapshot(&self) -> CatalogSnapshot {
        self.frames()
            .into_iter()
            .find_map(|frame| match frame {
                CatalogFrame::Snapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .expect("a snapshot")
    }
}

struct Channel(Rc<Wire>);

impl LineChannel for Channel {
    fn send(&self, line: Vec<u8>) -> bool {
        if self.0.full.get() {
            return false;
        }
        self.0.lines.borrow_mut().push(line);
        true
    }

    fn shutdown(&self) {
        self.0.shut.set(true);
    }
}

/// A fixture whose core serves the catalog, with `extra` sessions beside the seeded one.
pub(super) fn served(extra: usize) -> Fixture {
    let mut fixture = Fixture::new(extra);
    fixture.core.serve_catalog(CATALOG, None);
    fixture
}

/// A stream adopted after `catalog.subscribe`, before its hello.
pub(super) fn adopt(fixture: &mut Fixture) -> (u64, Rc<Wire>) {
    let stream = Rc::new(Wire::default());
    let serial = fixture.core.adopt_catalog_stream(Box::new(Channel(Rc::clone(&stream))));
    (serial, stream)
}

/// A stream whose hello (`pageRows` when `page_rows`) subscribed it.
pub(super) fn subscribed(fixture: &mut Fixture, page_rows: bool) -> Rc<Wire> {
    let (serial, stream) = adopt(fixture);
    let hello = if page_rows {
        r#"{"kind":"hello","hello":{"version":1,"pageRows":true}}"#
    } else {
        r#"{"kind":"hello","hello":{"version":1}}"#
    };
    fixture.core.catalog_line(serial, hello.as_bytes());
    stream
}

fn entry(snapshot: &CatalogSnapshot, id: Uuid) -> CatalogSession {
    snapshot.session(id).cloned().expect("the session's entry")
}

fn committed(fixture: &mut Fixture, cmd: Command, target: Option<&str>, args: Args) -> Response {
    let response = fixture.send(cmd, target, args);
    let _ = fixture.core.commit();
    response
}

/// Upstream's `catalogSubscribeIsServed` and `theSnapshotCarriesTheCatalogItsVersionsAndEveryPane`.
#[test]
fn a_subscriber_gets_hello_then_the_snapshot_of_every_session_and_pane() {
    let mut fixture = served(1);
    let subscribe = fixture.send(Command::Catalog(CatalogCmd::Subscribe), None, Args::default());
    assert_eq!(subscribe, Response::success());
    let session = fixture.sessions[1];
    committed(
        &mut fixture,
        Command::Session(SessionCmd::Split),
        Some(&wire(session)),
        args(|a| a.mode = Some("on".into())),
    );
    let stream = subscribed(&mut fixture, false);
    let frames = stream.frames();
    let CatalogFrame::Hello(hello) = &frames[0] else {
        panic!("no hello: {frames:?}")
    };
    assert_eq!(
        serde_json::to_value(hello).unwrap(),
        json!({"version": 1, "epoch": fixture.core.remote.respawns.epoch(),
               "versions": {"server": "0.1.0", "protocol": 1, "catalog": 1}})
    );
    let snapshot = stream.snapshot();
    // the split made before the hello is in the snapshot, at the revision it reached
    assert_eq!((snapshot.catalog_id, snapshot.rev), (CATALOG, 2));
    assert!(stream.deltas().is_empty());
    assert_eq!(snapshot.epoch, fixture.core.remote.respawns.epoch());
    assert_eq!(snapshot.workspaces.len(), 1);
    assert_eq!(snapshot.workspaces[0].id, fixture.workspace);
    let ids: Vec<Uuid> = snapshot.workspaces[0].sessions.iter().map(CatalogSession::id).collect();
    assert_eq!(ids, fixture.sessions);
    let model = fixture.store().session(session).unwrap();
    let split = entry(&snapshot, session);
    let panes: Vec<Uuid> = split.panes.iter().map(|pane| pane.identity).collect();
    assert_eq!(panes, [model.pane_identity, model.split_pane_identity.unwrap()]);
    assert_eq!(split.panes[0].daemon, crate::live::daemon_name(model.pane_identity));
    assert_eq!(
        (split.fields.primary, split.fields.axis.as_deref(), split.fields.shown),
        (model.pane_identity, Some("vertical"), true)
    );
    let plain = entry(&snapshot, fixture.sessions[0]);
    assert_eq!(
        (plain.panes.len(), plain.fields.axis.as_deref(), plain.fields.shown),
        (1, None, false)
    );
    assert_eq!(plain.fields.lifecycle, frames::Lifecycle::Live);
}

/// Upstream's `renamingASessionToItsOwnLabelPublishesTheManualName` and
/// `clearingTheNameOnTheServerReturnsTheMacRowToItsTerminalTitle` (#993): `name` is the resolved label, `customName`
/// the manual name, `""` without one.
#[test]
fn a_rename_publishes_the_manual_name_beside_the_label() {
    let mut fixture = served(0);
    let session = fixture.sessions[0];
    let stream = subscribed(&mut fixture, false);
    let unnamed = entry(&stream.snapshot(), session).fields;
    let label = fixture.store().session(session).unwrap().display_name();
    assert_eq!(
        (unnamed.name.as_str(), unnamed.custom_name.as_deref()),
        (label.as_str(), Some(""))
    );

    let rename = |name: &str| args(|a| a.name = Some(name.to_owned()));
    committed(
        &mut fixture,
        Command::Session(SessionCmd::Rename),
        Some(&wire(session)),
        rename(&label),
    );
    committed(
        &mut fixture,
        Command::Session(SessionCmd::Rename),
        Some(&wire(session)),
        rename(""),
    );
    let names: Vec<(String, Option<String>)> = stream
        .deltas()
        .into_iter()
        .map(|(_, change)| match change {
            CatalogChange::SessionChanged(fields) => (fields.name, fields.custom_name),
            other => panic!("not a session change: {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        [(label.clone(), Some(label.clone())), (label, Some(String::new()))]
    );
    assert_eq!(stream.deltas().iter().map(|(rev, _)| *rev).collect::<Vec<_>>(), [1, 2]);
}

/// Upstream's `aCompanionCarriesItsParent`.
#[test]
fn a_companion_carries_its_parent() {
    let mut fixture = served(0);
    let parent = fixture.sessions[0];
    let stream = subscribed(&mut fixture, false);
    let created = committed(
        &mut fixture,
        Command::Session(SessionCmd::CompanionNew),
        Some(&wire(parent)),
        Args::default(),
    );
    let companion = answered_id(&created).expect("a companion");
    let added = stream.deltas().into_iter().find_map(|(_, change)| match change {
        CatalogChange::SessionAdded { session, .. } => Some(session),
        _ => None,
    });
    let added = added.expect("an added session");
    assert_eq!((added.id(), added.fields.parent), (companion, Some(parent)));
}

/// Upstream's `aPageRowCarriesItsPageAndNoPane`: a page row's primary is its own id, and a viewer that does not
/// show page rows never sees it.
#[test]
fn a_page_row_carries_its_page_and_no_pane() {
    let (mut fixture, _) = fixture_with_host(0);
    fixture.core.serve_catalog(CATALOG, None);
    let older = subscribed(&mut fixture, false);
    let newer = subscribed(&mut fixture, true);
    let anchor = wire(fixture.sessions[0]);
    let page = committed(
        &mut fixture,
        Command::Session(SessionCmd::Page),
        Some(&anchor),
        args(|a| a.html = Some(crate::html::os_path("/tmp/p.html"))),
    );
    let row = answered_id(&page).expect("a page row");
    let added = newer.deltas().into_iter().find_map(|(_, change)| match change {
        CatalogChange::SessionAdded { session, .. } => Some(session),
        _ => None,
    });
    let added = added.expect("the page row");
    assert_eq!(added.id(), row);
    assert!(added.panes.is_empty());
    assert_eq!(added.fields.primary, row);
    let page = added.fields.page.expect("its page");
    assert_eq!((page.kind.as_str(), page.identity.as_str()), ("file", "p.html"));
    assert!(older.deltas().is_empty(), "{:?}", older.frames());
    assert!(
        older
            .frames()
            .iter()
            .any(|frame| matches!(frame, CatalogFrame::Unknown { kind, .. } if kind == frames::SKIP_KIND))
    );
}

/// Upstream's `eachServedStructuralCommandPublishesItsChange` and `aRefusedCommandPublishesNothing`.
#[test]
fn each_structural_command_publishes_its_change_and_a_refused_one_nothing() {
    let mut fixture = served(1);
    let stream = subscribed(&mut fixture, false);
    let (first, second) = (wire(fixture.sessions[0]), wire(fixture.sessions[1]));
    let refused = committed(
        &mut fixture,
        Command::Session(SessionCmd::Rename),
        Some(&wire(Uuid::new_v4())),
        args(|a| a.name = Some("x".into())),
    );
    assert!(!refused.ok);
    assert!(stream.deltas().is_empty());
    let workspace = committed(
        &mut fixture,
        Command::Workspace(WorkspaceCmd::New),
        None,
        args(|a| a.name = Some("dev".into())),
    );
    let workspace = workspace
        .result
        .as_ref()
        .and_then(|result| result["id"].as_str())
        .unwrap()
        .to_owned();
    let steps: [(Command, Option<String>, &str, Args); 4] = [
        (
            Command::Session(SessionCmd::New),
            None,
            "session.added",
            Args::default(),
        ),
        (
            Command::Session(SessionCmd::Move),
            Some(first),
            "session.moved",
            args(|a| a.workspace = Some(workspace.clone())),
        ),
        (
            Command::Session(SessionCmd::Split),
            Some(second),
            "pane.added",
            args(|a| a.mode = Some("on".into())),
        ),
        (
            Command::Workspace(WorkspaceCmd::Rename),
            Some(workspace.clone()),
            "workspace.changed",
            args(|a| a.name = Some("ops".into())),
        ),
    ];
    for (cmd, target, kind, step) in steps {
        let before = stream.deltas().len();
        let response = committed(&mut fixture, cmd, target.as_deref(), step);
        assert!(response.ok, "{cmd}: {response:?}");
        let kinds: Vec<&str> = stream.deltas()[before..]
            .iter()
            .map(|(_, change)| change.kind())
            .collect();
        assert!(kinds.contains(&kind), "{cmd}: {kinds:?}");
    }
    let revs: Vec<i64> = stream.deltas().iter().map(|(rev, _)| *rev).collect();
    assert_eq!(revs, (1..=i64::try_from(revs.len()).unwrap()).collect::<Vec<_>>());
}

/// Upstream's `eachServerRunHasItsOwnEpoch`.
#[test]
fn each_server_run_has_its_own_epoch() {
    let mut first = served(0);
    let mut second = served(0);
    let epochs = [
        subscribed(&mut first, false).snapshot().epoch,
        subscribed(&mut second, false).snapshot().epoch,
    ];
    assert_ne!(epochs[0], epochs[1]);
}

#[test]
fn a_stream_that_does_not_open_with_a_usable_hello_is_closed() {
    let mut fixture = served(0);
    for line in [
        &b"not json"[..],
        br#"{"kind":"ping"}"#,
        br#"{"kind":"hello","hello":{"version":0}}"#,
    ] {
        let (serial, stream) = adopt(&mut fixture);
        fixture.core.catalog_line(serial, line);
        assert!(stream.shut.get(), "{line:?}");
        assert!(stream.frames().is_empty());
    }
    let (serial, silent) = adopt(&mut fixture);
    fixture.core.catalog_hello_deadline(serial);
    assert!(silent.shut.get());
    let (serial, stream) = adopt(&mut fixture);
    fixture
        .core
        .catalog_line(serial, br#"{"kind":"hello","hello":{"version":1}}"#);
    fixture.core.catalog_hello_deadline(serial);
    assert!(!stream.shut.get(), "a subscribed stream outlives the deadline");
}

#[test]
fn a_subscribed_stream_is_pinged_answered_and_left_once_closed() {
    let mut fixture = served(0);
    let (serial, stream) = adopt(&mut fixture);
    fixture
        .core
        .catalog_line(serial, br#"{"kind":"hello","hello":{"version":1}}"#);
    assert!(fixture.core.catalog_heartbeat());
    assert_eq!(stream.frames().last(), Some(&CatalogFrame::Ping));
    fixture.core.catalog_line(serial, br#"{"kind":"ping"}"#);
    assert_eq!(stream.frames().last(), Some(&CatalogFrame::Ack));
    fixture.core.catalog_line(serial, b"garbage");
    assert!(stream.shut.get());
    fixture.core.catalog_closed(serial);
    let sent = stream.frames().len();
    committed(&mut fixture, Command::Session(SessionCmd::New), None, Args::default());
    assert_eq!(stream.frames().len(), sent, "a closed stream gets nothing more");
    assert!(!fixture.core.catalog_heartbeat());
    let live = subscribed(&mut fixture, false);
    fixture.core.shutdown_catalog_streams();
    assert!(live.shut.get());
}

#[test]
fn an_app_with_windows_publishes_no_catalog() {
    let mut fixture = Fixture::new(0);
    let (serial, stream) = adopt(&mut fixture);
    fixture
        .core
        .catalog_line(serial, br#"{"kind":"hello","hello":{"version":1}}"#);
    assert!(stream.shut.get());
    assert!(!fixture.core.catalog_heartbeat() || stream.frames().is_empty());
    let snapshot: Value = serde_json::to_value(fixture.core.catalog_source(CATALOG)).unwrap();
    assert_eq!(snapshot["catalogID"], json!(wire_id(CATALOG)));
}

fn wire_id(id: Uuid) -> String {
    crate::pane_env::uuid_string(id)
}

/// Upstream's `aStalledSubscriberIsDroppedAndTheOthersKeepTheirDeltas`, over the core: a stream whose queue is full is
/// closed, and the others keep their deltas.
#[test]
fn a_stalled_stream_is_closed_and_the_others_keep_their_deltas() {
    let mut fixture = served(0);
    let stalled = subscribed(&mut fixture, false);
    let healthy = subscribed(&mut fixture, false);
    stalled.full.set(true);
    committed(&mut fixture, Command::Session(SessionCmd::New), None, Args::default());
    assert!(stalled.shut.get());
    assert!(!healthy.shut.get());
    assert_eq!(healthy.deltas().len(), 1);
}
