use ngterm_core::catalog::frames::{
    self, CatalogChange, CatalogFrame, CatalogHello, CatalogPage, CatalogPane, CatalogScratch, CatalogSession,
    CatalogSessionFields, CatalogSnapshot, CatalogWorkspace, Lifecycle,
};
use ngterm_proto::{CatalogVersions, ControlLiveEndpoint, ControlRemoteTree};
use serde_json::json;
use uuid::Uuid;

use super::operations::{Operation, Position, Settled, fenced, settle, untold_outcome};
use super::*;

pub(crate) const EPOCH: &str = "E1";

pub(crate) fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub(crate) fn endpoint() -> ControlLiveEndpoint {
    ControlLiveEndpoint {
        attach_bridge: None,
        executable: "/ngterm-session".to_owned(),
        socket_directory: "/tmp/ngterm-live-x".to_owned(),
        peer: None,
    }
}

pub(crate) fn versions() -> CatalogVersions {
    CatalogVersions {
        server: "0.1.0".to_owned(),
        protocol: 1,
        catalog: 1,
    }
}

/// A live terminal session `n` with one pane (`n + 100`), under `parent`.
pub(crate) fn terminal(n: u128, name: &str, parent: Option<u128>) -> CatalogSession {
    let pane = id(n + 100);
    CatalogSession::new(
        fields(n, name, parent, pane),
        vec![CatalogPane {
            identity: pane,
            daemon: ngterm_core::live::daemon_name(pane),
            endpoint: endpoint(),
        }],
    )
}

pub(crate) fn fields(n: u128, name: &str, parent: Option<u128>, primary: Uuid) -> CatalogSessionFields {
    CatalogSessionFields {
        id: id(n),
        name: name.to_owned(),
        parent: parent.map(id),
        cwd: "/repo".to_owned(),
        primary,
        axis: None,
        shown: false,
        lifecycle: Lifecycle::Live,
        scratch: None,
        page: None,
        custom_name: Some(String::new()),
    }
}

/// A page row `n` of `kind`, under `parent`.
pub(crate) fn page(n: u128, name: &str, kind: &str, parent: Option<u128>) -> CatalogSession {
    let mut fields = fields(n, name, parent, id(n));
    fields.page = Some(CatalogPage {
        kind: kind.to_owned(),
        identity: "/notes.html".to_owned(),
        endpoint: endpoint(),
    });
    CatalogSession::new(fields, Vec::new())
}

/// An ended session `n`: no panes.
pub(crate) fn ended(n: u128, name: &str) -> CatalogSession {
    let mut fields = fields(n, name, None, id(n + 100));
    fields.lifecycle = Lifecycle::Ended;
    CatalogSession::new(fields, Vec::new())
}

pub(crate) fn snapshot(rev: i64, workspaces: Vec<CatalogWorkspace>) -> CatalogSnapshot {
    CatalogSnapshot {
        epoch: EPOCH.to_owned(),
        rev,
        catalog_id: id(0xCA7),
        versions: versions(),
        workspaces,
    }
}

pub(crate) fn workspace(n: u128, name: &str, sessions: Vec<CatalogSession>) -> CatalogWorkspace {
    CatalogWorkspace {
        id: id(n),
        name: name.to_owned(),
        sessions,
    }
}

fn hello(versions: Option<CatalogVersions>) -> CatalogFrame {
    CatalogFrame::Hello(CatalogHello {
        version: 1,
        epoch: Some(EPOCH.to_owned()),
        versions,
        page_rows: None,
    })
}

fn delta(rev: i64, change: CatalogChange) -> CatalogFrame {
    CatalogFrame::Delta {
        epoch: EPOCH.to_owned(),
        rev,
        change,
    }
}

fn renamed(n: u128, name: &str, rev_session: &CatalogSession) -> CatalogChange {
    let mut fields = rev_session.fields.clone();
    fields.id = id(n);
    name.clone_into(&mut fields.name);
    CatalogChange::SessionChanged(fields)
}

fn names(mirror: &Mirror) -> Vec<String> {
    rows(mirror.catalog().expect("a catalog"), None)
        .into_iter()
        .map(|row| row.name)
        .collect()
}

/// A mirror with `snapshot` taken after the hello.
fn mirrored(snapshot: CatalogSnapshot) -> Mirror {
    let mut mirror = Mirror::new("box");
    mirror.subscribing();
    assert_eq!(mirror.receive(hello(Some(versions()))), Received::default());
    assert_eq!(mirror.receive(CatalogFrame::Snapshot(snapshot)).moved, Some(true));
    mirror
}

// MARK: - the mirror

#[test]
fn a_snapshot_connects_and_each_next_revision_moves_the_rows() {
    let main = terminal(1, "main", None);
    let mut mirror = Mirror::new("box");
    assert_eq!(*mirror.state(), CatalogState::Connecting);
    mirror.receive(hello(Some(versions())));
    let moved = mirror.receive(CatalogFrame::Snapshot(snapshot(
        4,
        vec![workspace(9, "work", vec![main.clone()])],
    )));
    assert_eq!((moved.moved, mirror.state()), (Some(true), &CatalogState::Connected));
    assert_eq!(names(&mirror), ["main"]);

    let moved = mirror.receive(delta(5, renamed(1, "build", &main)));
    assert_eq!(moved.moved, Some(false));
    assert_eq!(names(&mirror), ["build"]);
    assert!(mirror.reaches(EPOCH, 5));
    assert!(!mirror.reaches(EPOCH, 6));
    assert!(!mirror.reaches("E2", 5));
}

#[test]
fn a_revision_already_seen_is_dropped_and_a_gap_or_another_epoch_asks_for_a_snapshot() {
    let main = terminal(1, "main", None);
    let mut mirror = mirrored(snapshot(4, vec![workspace(9, "work", vec![main.clone()])]));
    assert_eq!(mirror.receive(delta(4, renamed(1, "old", &main))).moved, None);
    assert!(!mirror.needs_snapshot());
    assert_eq!(names(&mirror), ["main"]);

    assert_eq!(mirror.receive(delta(6, renamed(1, "skipped", &main))).moved, None);
    assert!(mirror.needs_snapshot());
    assert_eq!(names(&mirror), ["main"], "the rows stay until the fresh snapshot");
    assert_eq!(
        mirror.receive(delta(5, renamed(1, "late", &main))).moved,
        None,
        "nothing applies past a gap"
    );

    let mut mirror = mirrored(snapshot(4, vec![workspace(9, "work", vec![main.clone()])]));
    let other = CatalogFrame::Delta {
        epoch: "E2".to_owned(),
        rev: 5,
        change: renamed(1, "restarted", &main),
    };
    assert_eq!(mirror.receive(other).moved, None);
    assert!(mirror.needs_snapshot());

    mirror.subscribing();
    assert!(!mirror.needs_snapshot());
    assert_eq!(*mirror.state(), CatalogState::Connecting);
    assert_eq!(
        names(&mirror),
        ["main"],
        "a new subscription keeps the rows until its snapshot"
    );
}

#[test]
fn a_snapshot_of_another_epoch_than_its_hello_asks_again() {
    let mut mirror = Mirror::new("box");
    mirror.receive(hello(Some(versions())));
    let mut stale = snapshot(1, Vec::new());
    stale.epoch = "E0".to_owned();
    assert_eq!(mirror.receive(CatalogFrame::Snapshot(stale)).moved, None);
    assert!(mirror.needs_snapshot());
    assert_eq!(mirror.catalog(), None);
}

#[test]
fn an_unknown_change_moves_the_cursor_past_it_and_a_ping_is_acked() {
    let mut mirror = mirrored(snapshot(4, vec![workspace(9, "work", vec![terminal(1, "main", None)])]));
    let unknown = CatalogFrame::Unknown {
        kind: frames::SKIP_KIND.to_owned(),
        epoch: Some(EPOCH.to_owned()),
        rev: Some(5),
    };
    assert_eq!(mirror.receive(unknown).moved, Some(false));
    assert!(mirror.reaches(EPOCH, 5));
    assert_eq!(mirror.receive(CatalogFrame::Ping).reply, Some(CatalogFrame::Ack));
    assert_eq!(mirror.receive(CatalogFrame::Ack), Received::default());
}

#[test]
fn an_oversized_snapshot_applies_at_its_commit_and_bad_chunks_ask_again() {
    let big = snapshot(
        2,
        vec![workspace(
            9,
            "work",
            (1..=40).map(|n| terminal(n, &"x".repeat(8000), None)).collect(),
        )],
    );
    let chunked = frames::snapshot_frames(&big).unwrap();
    assert!(chunked.len() > 2, "chunks and a commit");
    let mut mirror = Mirror::new("box");
    mirror.receive(hello(Some(versions())));
    let mut moved = Vec::new();
    for frame in chunked.clone() {
        moved.push(mirror.receive(frame).moved);
    }
    assert_eq!(moved.last(), Some(&Some(true)));
    assert!(moved[..moved.len() - 1].iter().all(Option::is_none));
    assert_eq!(mirror.catalog(), Some(&big));

    let mut mirror = Mirror::new("box");
    mirror.receive(hello(Some(versions())));
    mirror.receive(chunked[1].clone());
    assert!(mirror.needs_snapshot(), "a chunk out of order");
    let mut mirror = Mirror::new("box");
    mirror.receive(hello(Some(versions())));
    mirror.receive(chunked.last().unwrap().clone());
    assert!(mirror.needs_snapshot(), "a commit with too few chunks");
}

#[test]
fn a_lost_stream_keeps_the_rows_and_a_refusal_drops_them_for_good() {
    let mut mirror = mirrored(snapshot(1, vec![workspace(9, "work", vec![terminal(1, "main", None)])]));
    mirror.connection_lost("gone".to_owned());
    assert_eq!(
        *mirror.state(),
        CatalogState::Disconnected {
            reason: "gone".to_owned()
        }
    );
    assert_eq!(names(&mirror), ["main"]);

    let mut mirror = Mirror::new("box");
    mirror.receive(hello(Some(CatalogVersions {
        catalog: 9,
        ..versions()
    })));
    assert!(mirror.is_refused());
    assert_eq!(
        *mirror.state(),
        CatalogState::Refused {
            reason: "box speaks catalog 9, this app speaks catalog 1".to_owned()
        }
    );
    mirror.subscribing();
    mirror.connection_lost("gone".to_owned());
    assert!(mirror.is_refused(), "nothing undoes a refusal");
    assert_eq!(
        mirror.receive(CatalogFrame::Snapshot(snapshot(1, Vec::new()))),
        Received::default()
    );
    assert_eq!(mirror.catalog(), None);
}

#[test]
fn the_versions_a_machine_advertises_decide_whether_it_serves_a_catalog() {
    assert_eq!(version_refusal("box", Some(&versions())), None);
    assert_eq!(
        version_refusal("box", None).as_deref(),
        Some("box runs an ngterm without a catalog; a live session list needs ngterm-headless")
    );
    let old = CatalogVersions {
        catalog: 0,
        ..versions()
    };
    assert_eq!(
        version_refusal("box", Some(&old)).as_deref(),
        Some("box speaks catalog 0, this app speaks catalog 1")
    );
    let presentation = CatalogVersions {
        protocol: 0,
        ..versions()
    };
    assert_eq!(
        version_refusal("box", Some(&presentation)).as_deref(),
        Some("box speaks presentation 0, this app speaks presentation 1")
    );
}

#[test]
fn the_hello_asks_for_page_rows() {
    let line = hello_line();
    assert_eq!(line.last(), Some(&b'\n'));
    let value: serde_json::Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(
        value,
        json!({"kind": "hello", "hello": {"version": 1, "pageRows": true}})
    );
}

#[test]
fn forgetting_a_session_drops_it_and_its_companions() {
    let mut mirror = mirrored(snapshot(
        1,
        vec![workspace(
            9,
            "work",
            vec![
                terminal(1, "main", None),
                terminal(2, "helper", Some(1)),
                terminal(3, "other", None),
            ],
        )],
    ));
    mirror.forget(&uuid_string(id(1)).to_lowercase());
    assert_eq!(names(&mirror), ["other"]);
}

#[test]
fn each_state_has_a_name_for_the_log_and_forgetting_needs_a_catalog() {
    let names: Vec<&str> = [
        CatalogState::Connecting,
        CatalogState::Connected,
        CatalogState::Disconnected { reason: "x".to_owned() },
        CatalogState::Refused { reason: "x".to_owned() },
    ]
    .iter()
    .map(CatalogState::class)
    .collect();
    assert_eq!(names, ["connecting", "connected", "disconnected", "refused"]);
    let mut mirror = Mirror::new("box");
    mirror.forget("S1");
    assert_eq!(mirror.catalog(), None);
}

#[test]
fn retries_back_off_to_thirty_seconds() {
    let delays: Vec<u64> = (0..7).map(|attempt| retry_delay(attempt).as_secs()).collect();
    assert_eq!(delays, [1, 2, 5, 10, 30, 30, 30]);
}

// MARK: - the rows

#[test]
fn rows_follow_the_workspaces_with_companions_under_their_parents_and_page_rows_marked() {
    let catalog = snapshot(
        1,
        vec![
            workspace(
                9,
                "work",
                vec![
                    terminal(2, "helper", Some(1)),
                    terminal(1, "main", None),
                    page(4, "docs", "url", None),
                    page(5, "preview", "file", Some(1)),
                    terminal(6, "stray", Some(77)),
                ],
            ),
            workspace(8, "play", vec![ended(3, "old")]),
        ],
    );
    let rows = rows(&catalog, None);
    let shown: Vec<(&str, &str, Option<&str>, bool)> = rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.workspace_name.as_str(),
                row.page.as_deref(),
                row.ended,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("main", "work", None, false),
            ("helper", "work", None, false),
            ("docs", "work", Some("url"), false),
            ("stray", "work", None, false),
            ("old", "play", None, true),
        ]
    );
    assert_eq!(rows[0].id, uuid_string(id(1)));
    assert_eq!(rows[1].parent.as_deref(), Some(uuid_string(id(1)).as_str()));
    assert_eq!((rows[0].pane_count, rows[2].pane_count, rows[4].pane_count), (1, 0, 0));
    assert!(rows.iter().all(|row| row.peer == Some(crate::peer::PeerKind::Ngterm)));
}

#[test]
fn a_row_takes_its_status_context_and_window_from_the_listing() {
    let catalog = snapshot(
        1,
        vec![workspace(
            9,
            "work",
            vec![terminal(1, "main", None), terminal(3, "new", None)],
        )],
    );
    let tree: ControlRemoteTree = serde_json::from_value(json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [{
            "id": uuid_string(id(1)).to_lowercase(), "name": "stale name", "windowID": "w1", "windowName": "main window",
            "workspaceID": "ws1", "workspaceName": "stale", "cwd": "/old", "panes": [],
            "status": "Working", "statusColor": "blue", "statusShape": "checkmark", "context": "the build",
        }],
    }))
    .unwrap();
    let rows = rows(&catalog, Some(&tree));
    assert_eq!(rows[0].name, "main", "the catalog's name wins");
    assert_eq!(rows[0].workspace_name, "work");
    assert_eq!(rows[0].cwd, "/repo");
    assert_eq!(rows[0].window_name, "main window");
    assert_eq!(rows[0].status.as_deref(), Some("Working"));
    assert_eq!(rows[0].status_color.as_deref(), Some("blue"));
    assert_eq!(rows[0].status_shape.as_deref(), Some("checkmark"));
    assert_eq!(rows[0].context.as_deref(), Some("the build"));
    assert_eq!((rows[1].status.as_deref(), rows[1].window_name.as_str()), (None, ""));
}

#[test]
fn a_page_row_and_an_ended_row_without_a_scratch_have_nothing_to_attach() {
    assert!(is_page(&page(4, "docs", "url", None)));
    assert!(!is_page(&terminal(1, "main", None)));
    assert!(has_ended(&ended(3, "old")));
    let mut scratched = ended(3, "old");
    scratched.fields.scratch = Some(CatalogScratch {
        pane: terminal(9, "s", None).panes[0].clone(),
        shown: false,
    });
    assert!(!has_ended(&scratched), "its scratch still runs");
    let catalog = snapshot(1, vec![workspace(9, "work", vec![terminal(1, "main", None)])]);
    assert!(session(&catalog, &uuid_string(id(1)).to_lowercase()).is_some());
    assert!(session(&catalog, &uuid_string(id(2))).is_none());
}

// MARK: - operations

fn receipt(outcome: &str, rev: Option<i64>, error: Option<&str>) -> ngterm_proto::CatalogReceipt {
    ngterm_proto::CatalogReceipt {
        op_id: "OP".to_owned(),
        epoch: EPOCH.to_owned(),
        outcome: outcome.to_owned(),
        rev,
        error: error.map(str::to_owned),
    }
}

#[test]
fn a_receipt_settles_the_operation_by_its_outcome() {
    let reply: ngterm_proto::Response<ngterm_proto::ControlResult> =
        serde_json::from_value(json!({"ok": false, "error": "no such session"})).unwrap();
    assert_eq!(
        settle("box", "End Session", &receipt("applied", Some(7), None), None),
        Settled::Applied {
            epoch: EPOCH.to_owned(),
            rev: Some(7),
            reply: None
        }
    );
    assert_eq!(
        settle("box", "End Session", &receipt("noop", None, None), None),
        Settled::NoChange
    );
    assert_eq!(
        settle(
            "box",
            "End Session",
            &receipt("refused", None, Some("stale")),
            Some(reply)
        ),
        Settled::Failed("no such session".to_owned()),
        "the reply's own error first"
    );
    assert_eq!(
        settle("box", "End Session", &receipt("refused", None, Some("stale")), None),
        Settled::Failed("stale".to_owned())
    );
    assert_eq!(
        settle("box", "End Session", &receipt("refused", None, None), None),
        Settled::Failed("the remote command failed on box".to_owned())
    );
    assert_eq!(
        settle("box", "End Session", &receipt("fenced", None, None), None),
        Settled::Failed(fenced("box", "End Session"))
    );
    assert_eq!(
        settle("box", "Rename Session", &receipt("unknown", None, None), None),
        Settled::Failed(untold_outcome("box", "Rename Session"))
    );
    assert_eq!(fenced("box", "End Session"), "box did not run End Session");
    assert_eq!(
        untold_outcome("box", "Rename Session"),
        "box cannot tell what became of Rename Session"
    );
}

#[test]
fn each_operation_names_its_words_and_keeps_user_text_an_operand() {
    let new = Operation::NewSession {
        name: Some("-build".to_owned()),
    };
    assert_eq!(new.arguments(), ["session", "new", "--no-select", "--name=-build"]);
    assert!(new.operands().is_empty());
    let rename = Operation::RenameSession {
        id: "S1".to_owned(),
        name: "-x".to_owned(),
    };
    assert_eq!(rename.arguments(), ["session", "rename", "--target", "S1"]);
    assert_eq!(rename.operands(), ["-x"]);
    let end = Operation::EndSession { id: "S1".to_owned() };
    assert_eq!(end.arguments(), ["session", "close", "--target", "S1"]);
    assert_eq!(
        [new.label(), rename.label(), end.label()],
        ["New Session", "Rename Session", "End Session"]
    );
    let position = Position {
        epoch: EPOCH.to_owned(),
        rev: 3,
    };
    let form = ngterm_core::remote::origin::OriginShell::Posix.into();
    let line = operations::operation_line(form, &rename, "OP", &position);
    assert!(line.contains("--op-id"), "{line}");
    assert!(line.ends_with(r"'\''--'\'' '\''-x'\'''"), "{line}");
    let lookup = operations::lookup_line(form, "OP", &position);
    assert!(
        lookup.contains(r"'\''catalog'\'' '\''op'\'' '\''OP'\'' '\''--epoch'\'' '\''E1'\'' '\''--rev'\'' '\''3'\''"),
        "{lookup}"
    );
}
