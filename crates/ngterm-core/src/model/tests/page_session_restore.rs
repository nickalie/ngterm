//! Page sessions across a snapshot (upstream's `SnapshotRoundTripTests` page cases): a local page row, top-level or a
//! companion, is written with its page and no pane identity, and comes back as a page row with its companions in any
//! restore mode; a saved page that no longer checks out comes back as a page row that says so, never as a terminal.

use serde_json::json;
use uuid::Uuid;

use super::overlay_visibility::page;
use super::support::{add, ids};
use crate::html::{HtmlLoadState, HtmlOverlay, HtmlPageFlags, HtmlSource, os_path};
use crate::model::page_snapshot::UNREADABLE_PAGE;
use crate::model::snapshot::session_from_snapshot;
use crate::model::{AppStore, CompanionOptions, PageCompanionOptions, SessionContent};
use crate::persist::{SessionContentSnapshot, SessionSnapshot, Snapshot};
use crate::projection::session_node;

const HOME: &str = "/home/u";

/// `work`'s page session after `anchor`, showing `overlay` in the background.
fn page_session(store: &mut AppStore, anchor: Uuid, overlay: HtmlOverlay) -> Uuid {
    let options = PageCompanionOptions {
        select: false,
        background_color: Some("#112233".to_owned()),
        ..PageCompanionOptions::default()
    };
    store.add_page_session(anchor, overlay, options, HOME).unwrap()
}

/// `snapshot` through its JSON, as a launch reads it back.
fn round_trip(snapshot: &Snapshot) -> Snapshot {
    serde_json::from_value(serde_json::to_value(snapshot).unwrap()).unwrap()
}

fn restored(snapshot: &Snapshot, launch_restore: bool) -> AppStore {
    let mut store = AppStore::new();
    store.restore(&round_trip(snapshot), launch_restore);
    store
}

#[test]
fn a_page_session_writes_its_page_and_no_pane() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let anchor = add(&mut store, work, "/anchor");
    let flags = HtmlPageFlags {
        navigation: true,
        javascript: true,
        ..HtmlPageFlags::default()
    };
    let overlay = HtmlOverlay::new(
        HtmlSource::File {
            path: "/srv/site/index.html".to_owned(),
            grant_root: Some("/srv/site".to_owned()),
        },
        flags,
    );
    let id = page_session(&mut store, anchor, overlay);
    store.rename_session(id, "docs");

    let written = serde_json::to_value(store.snapshot()).unwrap();
    let saved = written["workspaces"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == json!(id.to_string().to_uppercase()))
        .unwrap()
        .clone();
    assert_eq!(saved["content"], json!("page"));
    assert_eq!(saved["customName"], json!("docs"));
    assert_eq!(saved["cwd"], json!("/anchor"));
    for pane_key in ["paneIdentity", "splitPaneIdentity", "isSplit", "initialCommand"] {
        assert!(saved.get(pane_key).is_none(), "{pane_key} in {saved}");
    }
    assert_eq!(
        saved["pages"],
        json!([{
            "slot": "session",
            "source": {"kind": "file", "path": "/srv/site/index.html", "grantRoot": "/srv/site"},
            "navigation": true,
            "javascript": true,
            "backgroundColor": "#112233",
        }])
    );
}

/// Upstream's page-group round trip in every restore mode: the launch restore (which arms commands) and a mid-run
/// rebuild alike.
#[test]
fn a_page_group_comes_back_whole_in_every_restore_mode() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let anchor = add(&mut store, work, "/anchor");
    let root = page_session(&mut store, anchor, page(&os_path("/tmp/root.html")));
    let terminal = store
        .add_companion(
            root,
            CompanionOptions {
                command: Some("make watch".to_owned()),
                ..CompanionOptions::default()
            },
            HOME,
        )
        .unwrap();
    let nested = store
        .add_page_companion(
            root,
            page(&os_path("/tmp/n.html")),
            PageCompanionOptions::default(),
            HOME,
        )
        .unwrap();
    store.set_session_expanded(root, false);
    store.session_mut(root).unwrap().flagged = true;
    let before = store.session(root).unwrap().overlay.page().unwrap().id;
    let snapshot = store.snapshot();

    for launch_restore in [true, false] {
        let back = restored(&snapshot, launch_restore);
        assert_eq!(ids(&back, work), [anchor, root, terminal, nested]);
        assert_restored_root(&back, root, before);
        let nested_row = back.session(nested).unwrap();
        assert_eq!(nested_row.parent_id(), Some(root));
        assert!(nested_row.overlay.page().is_some_and(|page| page.row));
        let terminal_row = back.session(terminal).unwrap();
        assert_eq!(terminal_row.content(), SessionContent::Terminal);
        assert_eq!(terminal_row.parent_id(), Some(root));
    }
}

/// The restored page root: a folded, flagged top-level page row owning no pane, its page open again under a fresh id.
fn assert_restored_root(back: &AppStore, root: Uuid, before: Uuid) {
    let row = back.session(root).unwrap();
    assert_eq!(row.content(), SessionContent::Page);
    assert_eq!(row.parent_id(), None);
    assert!(!row.companion.expanded);
    assert!(row.flagged);
    assert!(row.locally_managed_pane_identities().is_empty());
    let shown = row.overlay.page().expect("its page opens again");
    assert!(shown.row, "the page is its row again");
    assert_ne!(shown.id, before, "under a fresh id");
    assert!(matches!(&shown.source, HtmlSource::File { path, .. } if *path == os_path("/tmp/root.html")));
    assert_eq!(row.overlay.background_color.as_deref(), Some("#112233"));
}

/// A missing file never drops the row: its page opens and fails as a load, with its reload.
#[test]
fn a_page_whose_file_is_gone_still_comes_back_as_its_page() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let anchor = add(&mut store, work, "/anchor");
    let missing = std::env::temp_dir().join(format!("ngterm-gone-{}.html", Uuid::new_v4()));
    let root = page_session(&mut store, anchor, page(&missing.to_string_lossy()));

    let back = restored(&store.snapshot(), true);

    let shown = back.session(root).unwrap().overlay.page().expect("the page opens");
    assert!(matches!(&shown.source, HtmlSource::File { path, .. } if *path == missing.to_string_lossy()));
}

/// Upstream's unreadable page: a saved page this build cannot open (another kind of page, a source it does not know,
/// none at all, a file grant outside the home) keeps the row a page session with its companions, owning no pane. The
/// row shows a failed page with the reason, as the tree reports it, and it saves back as a page session with no page.
#[test]
fn a_saved_page_that_does_not_check_out_restores_as_an_unreadable_page_session() {
    let parent = Uuid::new_v4();
    let companion = Uuid::new_v4();
    let entries = [
        json!([{"slot": "session", "source": {"kind": "image", "path": "/tmp/a.png"}}]),
        json!([{"slot": "session", "source": {"kind": "url", "url": "ftp://example.com/"}}]),
        json!([{"slot": "session", "source": {"kind": "hologram"}}]),
        json!([{"slot": "session", "source": {"kind": "file", "path": "/etc/p.html", "grantRoot": "/"}}]),
        json!([{"slot": "left", "source": {"kind": "url", "url": "https://example.com/"}}]),
        json!(null),
    ];
    for pages in entries {
        let saved = json!({
            "version": crate::persist::snapshot::SNAPSHOT_VERSION,
            "workspaces": [{"id": Uuid::new_v4().to_string(), "name": "work", "sessions": [
                {"id": parent.to_string(), "cwd": "/srv", "content": "page", "pages": pages},
                {"id": companion.to_string(), "cwd": "/srv", "parentID": parent.to_string()},
            ]}],
        });
        let snapshot: Snapshot = serde_json::from_value(saved).unwrap();
        let mut store = AppStore::new();
        store.restore(&snapshot, true);

        let row = store.session(parent).unwrap();
        assert_eq!(row.content(), SessionContent::Page, "{pages}");
        let shown = row.overlay.page().expect("the row shows its failed page");
        assert!(shown.row, "{pages}");
        assert_eq!(shown.load_state, HtmlLoadState::Failed, "{pages}");
        assert_eq!(shown.load_error.as_deref(), Some(UNREADABLE_PAGE), "{pages}");
        let node = serde_json::to_value(session_node(row, false)).unwrap();
        assert_eq!(node["overlay"], json!(true), "{pages}");
        assert_eq!(node["htmlOverlays"].as_array().map(Vec::len), Some(1), "{pages}");
        assert_eq!(node["htmlOverlays"][0]["state"], json!("failed"), "{pages}");
        assert_eq!(node["htmlOverlays"][0]["error"], json!(UNREADABLE_PAGE), "{pages}");
        assert!(row.locally_managed_pane_identities().is_empty());
        assert_eq!(store.session(companion).unwrap().parent_id(), Some(parent));
        let again = store.snapshot();
        let written = &again.workspaces[0].sessions[0];
        assert_eq!(written.content, Some(SessionContentSnapshot::Page), "{pages}");
        assert_eq!((written.pane_identity, written.pages.as_ref()), (None, None));
    }
}

/// A snapshot written before page sessions, and an unknown content, restore as a terminal.
#[test]
fn a_session_without_a_page_content_restores_as_a_terminal() {
    let legacy = SessionSnapshot::new(Uuid::new_v4(), "/srv");
    assert_eq!(
        session_from_snapshot(&legacy, true, HOME).content(),
        SessionContent::Terminal
    );
    let unknown: SessionSnapshot =
        serde_json::from_value(json!({"id": Uuid::new_v4().to_string(), "cwd": "/srv", "content": "hologram"}))
            .unwrap();
    assert_eq!(unknown.content, None);
    assert_eq!(
        session_from_snapshot(&unknown, true, HOME).content(),
        SessionContent::Terminal
    );
}

/// A page another machine serves is never written: a replica page row is not this machine's.
#[test]
fn a_replica_page_row_is_never_written() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let anchor = add(&mut store, work, "/anchor");
    let replica = HtmlOverlay::new(
        HtmlSource::Replica {
            path: "/srv/r.html".to_owned(),
            content: crate::html::ReplicaContent::Html,
        },
        HtmlPageFlags::default(),
    );
    let id = page_session(&mut store, anchor, replica);

    assert!(!store.session(id).unwrap().is_persistable());
    let snapshot = store.snapshot();
    assert!(snapshot.workspaces[0].sessions.iter().all(|session| session.id != id));
}
