//! A session's pages in its snapshot (an extension beyond upstream, whose overlays are ephemeral): what is written per
//! source kind and slot, what a restore opens again under fresh ids, and the entries it drops because their file is
//! gone, their source fails its checks or their slot cannot take them.

use std::path::Path;

use url::Url;
use uuid::Uuid;

use super::support::add;
use crate::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::model::{AppStore, OverlayPane, OverlayPointSize, OverlaySize, Session};
use crate::persist::{PageSlotSnapshot, PageSnapshot, PageSourceSnapshot, SessionSnapshot, Snapshot};

/// A folder holding a page, an image and a Markdown document.
struct Files {
    dir: tempfile::TempDir,
}

impl Files {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        for name in ["page.html", "picture.png", "notes.md"] {
            std::fs::write(dir.path().join(name), "x").expect("a file");
        }
        Self { dir }
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().into_owned()
    }

    fn root(&self) -> String {
        self.dir.path().to_string_lossy().into_owned()
    }
}

/// A store with one session at `/a`, its split shown.
fn store_with_split() -> (AppStore, Uuid) {
    let mut store = AppStore::new();
    let workspace = store.add_workspace("work");
    let session = add(&mut store, workspace, "/a");
    store.set_split_visibility(session, true, None);
    (store, session)
}

fn open(
    store: &mut AppStore,
    session: Uuid,
    pane: Option<OverlayPane>,
    overlay: HtmlOverlay,
    size: OverlaySize,
) -> Uuid {
    let id = overlay.id;
    store
        .open_html_overlay_sized(session, pane, overlay, size, Some("#102030".to_owned()))
        .expect("the page opens");
    id
}

fn page(source: HtmlSource, flags: HtmlPageFlags) -> HtmlOverlay {
    HtmlOverlay::new(source, flags)
}

/// `store`'s snapshot through its JSON text and back, restored into a fresh store.
fn round_trip(store: &AppStore) -> AppStore {
    let text = serde_json::to_string(&store.snapshot()).expect("a snapshot");
    let snapshot: Snapshot = serde_json::from_str(&text).expect("a snapshot");
    let mut back = AppStore::new();
    back.restore(&snapshot, false);
    back
}

fn only_session(store: &AppStore) -> &Session {
    &store.workspaces()[0].sessions[0]
}

/// Opens `overlay` in `pane`'s slot.
fn open_in(store: &mut AppStore, session: Uuid, pane: OverlayPane, overlay: HtmlOverlay) -> Uuid {
    open(store, session, Some(pane), overlay, OverlaySize::default())
}

/// The pages `session` shows: each one's slot, source and flags.
fn shown(session: &Session) -> Vec<(Option<OverlayPane>, HtmlSource, HtmlPageFlags)> {
    session
        .html_overlays()
        .into_iter()
        .map(|(pane, page)| (pane, page.source.clone(), page.flags))
        .collect()
}

#[test]
fn every_page_kind_comes_back_in_its_slot_under_a_fresh_id() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let flags = HtmlPageFlags {
        navigation: true,
        javascript: true,
        ..HtmlPageFlags::default()
    };
    let file = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: Some(files.root()),
    };
    let size = OverlaySize {
        percent: Some(40),
        points: OverlayPointSize {
            width: Some(300),
            height: None,
        },
    };
    let wide = open(&mut store, session, None, page(file.clone(), flags), size);
    let url = HtmlSource::Url(Url::parse("https://example.com/a?b=1").unwrap());
    let browsing = HtmlPageFlags {
        persistent: true,
        browse: true,
        ..HtmlPageFlags::default()
    };
    let left = open_in(&mut store, session, OverlayPane::Left, page(url.clone(), browsing));
    let image = HtmlSource::Image {
        path: files.path("picture.png"),
    };
    let right = open_in(
        &mut store,
        session,
        OverlayPane::Right,
        page(image.clone(), HtmlPageFlags::default()),
    );
    store.session_mut(session).unwrap().overlay.hidden = true;

    let back = round_trip(&store);
    let restored = only_session(&back);
    assert_eq!(
        shown(restored),
        [
            (None, file, flags),
            (Some(OverlayPane::Left), url, browsing),
            (Some(OverlayPane::Right), image, HtmlPageFlags::default()),
        ]
    );
    for (_, restored_page) in &restored.html_overlays() {
        assert!(![wide, left, right].contains(&restored_page.id), "a fresh id");
        assert_eq!(restored_page.replica_of, None);
    }
    let slot = &restored.overlay;
    assert_eq!(
        (slot.size_percent, slot.points.width, slot.points.height),
        (Some(40), Some(300), None)
    );
    assert_eq!(slot.background_color.as_deref(), Some("#102030"));
    assert!(slot.hidden);
    let left_overlay = restored.pane_overlay(OverlayPane::Left).unwrap();
    let right_overlay = restored.pane_overlay(OverlayPane::Right).unwrap();
    assert!(!left_overlay.hidden);
    assert_eq!(right_overlay.background_color.as_deref(), Some("#102030"));
    assert_ne!(
        left_overlay.generation, right_overlay.generation,
        "each pane gets its own generation"
    );
    assert_eq!(
        left_overlay.holds_cover,
        Some(restored.overlay.generation),
        "the hold written beside it comes back over the restored cover"
    );
    assert!(
        !restored.session_slot_shown(),
        "the left page opened over it still draws above it"
    );
}

#[test]
fn a_session_page_opened_over_a_pane_page_stays_on_top_after_the_round_trip() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let file = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: Some(files.root()),
    };
    let image = HtmlSource::Image {
        path: files.path("picture.png"),
    };
    // the left page opens first, so it holds no cover; the session-wide page opened after it draws above it
    open_in(
        &mut store,
        session,
        OverlayPane::Left,
        page(image, HtmlPageFlags::default()),
    );
    open(
        &mut store,
        session,
        None,
        page(file, HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    assert!(store.session(session).unwrap().session_slot_shown());

    let back = round_trip(&store);
    let restored = only_session(&back);
    assert_eq!(restored.pane_overlay(OverlayPane::Left).unwrap().holds_cover, None);
    assert!(restored.session_slot_shown(), "the session-wide page stays on top");
    assert!(restored.cover_overlay_visible());
}

#[test]
fn a_pane_page_held_over_a_cover_that_closed_does_not_hold_the_next_one_after_the_round_trip() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let file = |name: &str| HtmlSource::File {
        path: files.path(name),
        grant_root: Some(files.root()),
    };
    let image = HtmlSource::Image {
        path: files.path("picture.png"),
    };
    open(
        &mut store,
        session,
        None,
        page(file("page.html"), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    open_in(
        &mut store,
        session,
        OverlayPane::Left,
        page(image, HtmlPageFlags::default()),
    );
    // the cover the left page held closes, and a new one opens over the left page
    store.close_overlay(session);
    open(
        &mut store,
        session,
        None,
        page(file("page.html"), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    assert!(store.session(session).unwrap().session_slot_shown());

    let back = round_trip(&store);
    let restored = only_session(&back);
    assert_eq!(restored.pane_overlay(OverlayPane::Left).unwrap().holds_cover, None);
    assert!(
        restored.session_slot_shown(),
        "the newer session-wide page stays on top"
    );
}

#[test]
fn a_markdown_page_and_a_chromeless_loopback_page_round_trip() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let markdown = HtmlSource::Markdown {
        path: files.path("notes.md"),
    };
    open(
        &mut store,
        session,
        None,
        page(markdown.clone(), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    let local = HtmlSource::Url(Url::parse("http://localhost:5173/").unwrap());
    let chromeless = HtmlPageFlags {
        chromeless: true,
        javascript: true,
        ..HtmlPageFlags::default()
    };
    open_in(&mut store, session, OverlayPane::Right, page(local.clone(), chromeless));
    let back = round_trip(&store);
    assert_eq!(
        shown(only_session(&back)),
        [
            (None, markdown, HtmlPageFlags::default()),
            (Some(OverlayPane::Right), local, chromeless),
        ]
    );
    let slot = &only_session(&back).overlay;
    assert_eq!((slot.size_percent, slot.points), (None, OverlayPointSize::default()));
}

#[test]
fn a_replica_and_a_page_row_are_never_written_and_no_page_writes_no_key() {
    let (mut store, session) = store_with_split();
    let text = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(!text.contains("\"pages\""), "{text}");

    let mut replica = page(
        HtmlSource::Replica {
            path: "/remote/page.html".to_owned(),
        },
        HtmlPageFlags::default(),
    );
    replica.replica_of = Some(Uuid::new_v4());
    open(&mut store, session, None, replica, OverlaySize::default());
    let mut row = page(
        HtmlSource::Url(Url::parse("https://example.com/").unwrap()),
        HtmlPageFlags::default(),
    );
    row.row = true;
    open(
        &mut store,
        session,
        Some(OverlayPane::Left),
        row,
        OverlaySize::default(),
    );
    assert_eq!(store.snapshot().workspaces[0].sessions[0].pages, None);
}

/// A session snapshot with `pages`, its split existing when `split`.
fn persisted(pages: Vec<PageSnapshot>, split: bool) -> SessionSnapshot {
    SessionSnapshot {
        has_split: split.then_some(true),
        pages: Some(pages),
        ..SessionSnapshot::new(Uuid::new_v4(), "/a")
    }
}

fn entry(slot: PageSlotSnapshot, source: PageSourceSnapshot) -> PageSnapshot {
    PageSnapshot {
        slot,
        source,
        navigation: false,
        javascript: false,
        chromeless: false,
        persistent: false,
        browse: false,
        size_percent: None,
        width: None,
        height: None,
        background_color: None,
        hidden: false,
        holds_cover: false,
    }
}

fn restored_sources(snapshot: &SessionSnapshot, home: &str) -> Vec<(Option<OverlayPane>, HtmlSource)> {
    let session = crate::model::snapshot::session_from_snapshot(snapshot, false, home);
    session
        .html_overlays()
        .into_iter()
        .map(|(pane, page)| (pane, page.source.clone()))
        .collect()
}

fn file(path: String, grant_root: Option<String>) -> PageSourceSnapshot {
    PageSourceSnapshot::File { path, grant_root }
}

#[test]
fn an_entry_that_no_longer_checks_out_is_dropped_alone() {
    let files = Files::new();
    let gone = Path::new(&files.root()).join("gone.html");
    let url = |text: &str| PageSourceSnapshot::Url { url: text.to_owned() };
    let flagged = |source: PageSourceSnapshot, edit: fn(&mut PageSnapshot)| {
        let mut entry = entry(PageSlotSnapshot::Session, source);
        edit(&mut entry);
        entry
    };
    let home = files.root();
    let dropped = [
        entry(
            PageSlotSnapshot::Session,
            file(gone.to_string_lossy().into_owned(), None),
        ),
        entry(
            PageSlotSnapshot::Session,
            file(files.path("page.html"), Some(home.clone())),
        ),
        entry(PageSlotSnapshot::Session, file("page.html".to_owned(), None)),
        entry(PageSlotSnapshot::Session, url("ftp://example.com/")),
        entry(PageSlotSnapshot::Session, url("example.com")),
        entry(
            PageSlotSnapshot::Session,
            PageSourceSnapshot::Image {
                path: files.path("notes.md"),
            },
        ),
        entry(
            PageSlotSnapshot::Session,
            PageSourceSnapshot::Markdown {
                path: "notes.md".to_owned(),
            },
        ),
        flagged(url("https://example.com/"), |entry| entry.chromeless = true),
        flagged(url("http://localhost/"), |entry| {
            entry.chromeless = true;
            entry.browse = true;
        }),
        flagged(file(files.path("page.html"), None), |entry| entry.persistent = true),
        flagged(
            PageSourceSnapshot::Image {
                path: files.path("picture.png"),
            },
            |entry| entry.javascript = true,
        ),
        entry(PageSlotSnapshot::Right, url("https://example.com/")),
    ];
    for bad in dropped {
        let snapshot = persisted(vec![bad.clone()], false);
        assert_eq!(restored_sources(&snapshot, &home), [], "{bad:?}");
    }
    // a second page for a taken slot drops, the first stays
    let first = url("https://example.com/first");
    let snapshot = persisted(
        vec![
            entry(PageSlotSnapshot::Left, first),
            entry(PageSlotSnapshot::Left, url("https://example.com/second")),
            entry(PageSlotSnapshot::Right, file(files.path("page.html"), None)),
        ],
        true,
    );
    let expected_url = HtmlSource::Url(Url::parse("https://example.com/first").unwrap());
    let expected_file = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: None,
    };
    assert_eq!(
        restored_sources(&snapshot, &home),
        [
            (Some(OverlayPane::Left), expected_url),
            (Some(OverlayPane::Right), expected_file)
        ]
    );
}

#[test]
fn a_reopened_closed_session_brings_its_page_back_under_a_fresh_id() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let source = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: None,
    };
    let before = open(
        &mut store,
        session,
        Some(OverlayPane::Right),
        page(source.clone(), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    store.close_session(session);
    let item = store
        .recent_closed_items()
        .into_iter()
        .find(|item| {
            item.session
                .as_ref()
                .is_some_and(|recent| recent.snapshot.id == session)
        })
        .expect("the closed session's entry");
    assert!(store.restore_recent_closed(&item));
    let pages = store.session(session).unwrap().html_overlays();
    assert_eq!(pages.len(), 1);
    assert_eq!((pages[0].0, &pages[0].1.source), (Some(OverlayPane::Right), &source));
    assert_ne!(pages[0].1.id, before);
}

#[test]
fn a_hidden_split_keeps_its_page() {
    let files = Files::new();
    let source = HtmlSource::Markdown {
        path: files.path("notes.md"),
    };
    let (mut store, session) = store_with_split();
    open(
        &mut store,
        session,
        Some(OverlayPane::Right),
        page(source.clone(), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    store.set_split_visibility(session, false, None);
    let back = round_trip(&store);
    let restored = only_session(&back);
    assert!(restored.split.exists && !restored.split.shown);
    let pages = restored.html_overlays();
    assert_eq!(pages.len(), 1);
    assert_eq!((pages[0].0, &pages[0].1.source), (Some(OverlayPane::Right), &source));
}
