//! A session's pages in its snapshot (an extension beyond upstream, whose overlays are ephemeral): what is written per
//! source kind and slot, what a restore opens again under fresh ids, and the entries it drops because their file is
//! gone, their source fails its checks or their slot cannot take them.

use std::path::Path;
use std::time::Instant;

use url::Url;
use uuid::Uuid;

use super::support::add;
use crate::html::text_page::{TextLog, TextLogReport, TextLogState};
use crate::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::model::{AppStore, OverlayPane, OverlayPointSize, OverlaySize, RemoteOverlaySlot, RemotePage, Session};
use crate::persist::{PageSlotSnapshot, PageSnapshot, PageSourceSnapshot, SessionSnapshot, Snapshot};

/// A folder holding a page, an image and a Markdown document.
struct Files {
    dir: tempfile::TempDir,
}

impl Files {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        for name in ["page.html", "picture.png", "notes.md", "main.go"] {
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

/// A text page is written with the line it was opened at and opens again there; one whose file is gone or no longer a
/// text page's is dropped.
#[test]
fn a_text_page_round_trips_with_its_line() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let text = HtmlSource::Text {
        path: files.path("main.go"),
        line: Some(42),
    };
    open(
        &mut store,
        session,
        None,
        page(text.clone(), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    let written = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(written.contains("\"kind\":\"text\""), "{written}");
    assert!(written.contains("\"line\":42"), "{written}");
    let back = round_trip(&store);
    assert_eq!(shown(only_session(&back)), [(None, text, HtmlPageFlags::default())]);

    let (mut store, session) = store_with_split();
    let gone = HtmlSource::Text {
        path: files.path("gone.go"),
        line: None,
    };
    open(
        &mut store,
        session,
        None,
        page(gone, HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    assert!(shown(only_session(&round_trip(&store))).is_empty());
}

/// A clicked HTML document is written by its file and opens again as one; one whose file is gone, or that names no
/// HTML file, is dropped.
#[test]
fn a_clicked_document_round_trips_by_its_file() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let document = HtmlSource::Document {
        path: files.path("page.html"),
    };
    open(
        &mut store,
        session,
        None,
        page(document.clone(), HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    let written = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(written.contains("\"kind\":\"document\""), "{written}");
    let back = round_trip(&store);
    assert_eq!(shown(only_session(&back)), [(None, document, HtmlPageFlags::default())]);

    for name in ["gone.html", "main.go"] {
        let (mut store, session) = store_with_split();
        let refused = HtmlSource::Document { path: files.path(name) };
        open(
            &mut store,
            session,
            None,
            page(refused, HtmlPageFlags::default()),
            OverlaySize::default(),
        );
        assert!(shown(only_session(&round_trip(&store))).is_empty(), "{name}");
    }
}

#[test]
fn a_replica_and_a_page_row_are_never_written_and_no_page_writes_no_key() {
    let (mut store, session) = store_with_split();
    let text = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(!text.contains("\"pages\""), "{text}");

    let mut replica = page(
        HtmlSource::Replica {
            path: "/remote/page.html".to_owned(),
            content: crate::html::ReplicaContent::Html,
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
        viewer: false,
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

/// Reserves `pane`'s slot of `session` for `handed`, as a hand-over to the session's presenter does.
fn hand_over(store: &mut AppStore, session: Uuid, pane: Option<OverlayPane>, handed: RemotePage, size: OverlaySize) {
    let slot = RemoteOverlaySlot {
        job: crate::pane_env::uuid_string(handed.overlay.id),
        pane,
        owner: 1,
        size_percent: size.percent,
        points: size.points,
        wait: false,
        ended: false,
        page: Some(handed),
        presenter_sized: false,
    };
    assert!(store.reserve_remote_overlay(session, slot));
}

/// A store whose split session's viewer holds a file page shown in the session-wide slot (60% and 300 points, a
/// background, JavaScript on) and a hidden URL page parked in the right slot, beside a local URL page on the left:
/// the store, the two viewer pages' ids, and the file and URL sources.
fn viewer_pages(files: &Files) -> (AppStore, [Uuid; 2], HtmlSource, HtmlSource) {
    let (mut store, session) = store_with_split();
    let file = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: Some(files.root()),
    };
    let flags = HtmlPageFlags {
        javascript: true,
        ..HtmlPageFlags::default()
    };
    let mut wide = RemotePage::new(page(file.clone(), flags), "t".to_owned(), Instant::now());
    wide.background_color = Some("#102030".to_owned());
    let size = OverlaySize {
        percent: Some(60),
        points: OverlayPointSize {
            width: Some(300),
            height: None,
        },
    };
    let wide_id = wide.overlay.id;
    hand_over(&mut store, session, None, wide, size);
    let url = HtmlSource::Url(Url::parse("https://example.com/").unwrap());
    let mut parked = RemotePage::new(
        page(url.clone(), HtmlPageFlags::default()),
        "u".to_owned(),
        Instant::now(),
    );
    parked.hidden = true;
    parked.parked = true;
    let ids = [wide_id, parked.overlay.id];
    hand_over(
        &mut store,
        session,
        Some(OverlayPane::Right),
        parked,
        OverlaySize::default(),
    );
    open_in(
        &mut store,
        session,
        OverlayPane::Left,
        page(url.clone(), HtmlPageFlags::default()),
    );
    (store, ids, file, url)
}

/// A page the session's viewer holds, shown on a presenter or parked, is written marked `viewer` beside the local
/// ones, with the size its slot keeps, its background and the visibility it was last asked for.
#[test]
fn a_viewer_page_is_written_marked() {
    let files = Files::new();
    let (store, _, _, _) = viewer_pages(&files);
    let written = store.snapshot().workspaces[0].sessions[0].pages.clone().unwrap();
    let marks: Vec<_> = written
        .iter()
        .map(|entry| (entry.slot, entry.viewer, entry.hidden))
        .collect();
    assert_eq!(
        marks,
        [
            (PageSlotSnapshot::Left, false, false),
            (PageSlotSnapshot::Session, true, false),
            (PageSlotSnapshot::Right, true, true),
        ]
    );
    let entry = &written[1];
    assert_eq!(
        (
            entry.size_percent,
            entry.width,
            entry.background_color.as_deref(),
            entry.javascript
        ),
        (Some(60), Some(300), Some("#102030"), true)
    );
}

/// A viewer entry is restored parked in its slot under a fresh id, never opened here, with its size, background and
/// visibility.
#[test]
fn a_viewer_page_is_restored_parked() {
    let files = Files::new();
    let (store, ids, file, url) = viewer_pages(&files);
    let back = round_trip(&store);
    let restored = only_session(&back);
    assert_eq!(
        shown(restored),
        [(Some(OverlayPane::Left), url.clone(), HtmlPageFlags::default())],
        "only the local page opens here"
    );
    assert!(!restored.overlay.active, "a parked page covers nothing");
    let overlays = restored.remote_overlays();
    assert_eq!(overlays.slots().len(), 2);
    for slot in overlays.slots() {
        let parked = slot.page.as_ref().unwrap();
        assert!(parked.parked && !ids.contains(&parked.overlay.id), "{slot:?}");
        assert_eq!(slot.job, crate::pane_env::uuid_string(parked.overlay.id));
    }
    let wide = overlays.slot(None).unwrap();
    let wide_page = wide.page.as_ref().unwrap();
    assert_eq!(
        (
            wide.size_percent,
            wide.points.width,
            wide_page.background_color.as_deref()
        ),
        (Some(60), Some(300), Some("#102030"))
    );
    assert_eq!(wide_page.overlay.source, file);
    assert!(wide_page.overlay.flags.javascript);
    let right = overlays.slot(Some(OverlayPane::Right)).unwrap().page.as_ref().unwrap();
    assert!(right.hidden && right.effective_hidden());
    assert_eq!(right.overlay.source, url);
}

/// A viewer entry checks out as a local one does, and drops when its slot is taken here or its split is gone.
#[test]
fn a_viewer_entry_needs_a_free_slot() {
    let url = |text: &str| PageSourceSnapshot::Url { url: text.to_owned() };
    let viewer = |slot, source| PageSnapshot {
        viewer: true,
        ..entry(slot, source)
    };
    let parked_slots = |snapshot: &SessionSnapshot| {
        let session = crate::model::snapshot::session_from_snapshot(snapshot, false, "/home/u");
        let panes: Vec<Option<OverlayPane>> = session.remote_overlays().slots().iter().map(|slot| slot.pane).collect();
        (panes, session.html_overlays().len())
    };
    let no_split = persisted(
        vec![viewer(PageSlotSnapshot::Right, url("https://example.com/"))],
        false,
    );
    assert_eq!(parked_slots(&no_split), (vec![], 0));
    let bad = persisted(vec![viewer(PageSlotSnapshot::Session, url("example.com"))], false);
    assert_eq!(parked_slots(&bad), (vec![], 0));
    let taken = persisted(
        vec![
            entry(PageSlotSnapshot::Session, url("https://example.com/here")),
            viewer(PageSlotSnapshot::Session, url("https://example.com/there")),
            viewer(PageSlotSnapshot::Left, url("https://example.com/left")),
            entry(PageSlotSnapshot::Left, url("https://example.com/also-left")),
        ],
        false,
    );
    assert_eq!(parked_slots(&taken), (vec![Some(OverlayPane::Left)], 1));
}

/// A presenter-only origin parks the pages a restore opened in its own slots, with their size, background and
/// visibility, a picture included now that a presenter builds one (#891); a page no presenter can show (a log page) is
/// closed, as nothing there would ever show it.
#[test]
fn local_pages_park_for_a_presenter_only_origin() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let file = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: None,
    };
    let size = OverlaySize {
        percent: Some(40),
        points: OverlayPointSize::default(),
    };
    let wide = open(
        &mut store,
        session,
        None,
        page(file.clone(), HtmlPageFlags::default()),
        size,
    );
    store.session_mut(session).unwrap().overlay.hidden = true;
    let image = HtmlSource::Image {
        path: files.path("picture.png"),
    };
    let picture = open_in(
        &mut store,
        session,
        OverlayPane::Left,
        page(image.clone(), HtmlPageFlags::default()),
    );
    let log = HtmlSource::Text {
        path: files.path("app.log"),
        line: None,
    };
    let log = page(log, HtmlPageFlags::default()).with_text_log(Some(TextLog { tail: 10, follow: true }));
    open_in(&mut store, session, OverlayPane::Right, log);

    let found = store.session_mut(session).unwrap();
    let swept = crate::model::page_snapshot::park_local_pages(found);
    assert_eq!(
        (swept.parked, swept.dropped),
        (vec![wide, picture], true),
        "in drawing order"
    );
    let again = crate::model::page_snapshot::park_local_pages(found);
    assert!(again.parked.is_empty() && !again.dropped, "once");
    let found = store.session(session).unwrap();
    assert!(!found.overlay.active);
    assert!(shown(found).is_empty(), "the log page is closed and the rest parked");
    assert!(
        !found.local_overlay_holds(Some(OverlayPane::Right)),
        "the log page's slot is free"
    );
    let left = found.remote_overlays().slot(Some(OverlayPane::Left)).unwrap();
    let parked_picture = left.page.as_ref().unwrap();
    assert_eq!(parked_picture.overlay.source, image);
    assert_eq!(parked_picture.image_pixels, None, "probed when a presenter takes it");
    let slot = found.remote_overlays().slot(None).unwrap();
    let parked = slot.page.as_ref().unwrap();
    assert_eq!((parked.overlay.id, &parked.overlay.source), (wide, &file));
    assert!(parked.parked && parked.hidden);
    assert_eq!(
        (slot.size_percent, parked.background_color.as_deref()),
        (Some(40), Some("#102030"))
    );
}

/// A presenter-only origin parks its pages in the order they were drawn, so the next presenter, handed them in that
/// order, stacks them the same way: a pane page the session-wide page was opened over before it, a pane page opened
/// over the session-wide page after it.
#[test]
fn local_pages_park_in_drawing_order() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let file = HtmlSource::File {
        path: files.path("page.html"),
        grant_root: Some(files.root()),
    };
    let url = |path: &str| HtmlSource::Url(Url::parse(&format!("https://example.com/{path}")).unwrap());
    let under = open_in(
        &mut store,
        session,
        OverlayPane::Left,
        page(url("under"), HtmlPageFlags::default()),
    );
    let wide = open(
        &mut store,
        session,
        None,
        page(file, HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    let over = open_in(
        &mut store,
        session,
        OverlayPane::Right,
        page(url("over"), HtmlPageFlags::default()),
    );
    let found = store.session_mut(session).unwrap();
    assert_eq!(
        crate::model::page_snapshot::park_local_pages(found).parked,
        [under, wide, over]
    );
    let order: Vec<Option<OverlayPane>> = found.remote_overlays().slots().iter().map(|slot| slot.pane).collect();
    assert_eq!(order, [Some(OverlayPane::Left), None, Some(OverlayPane::Right)]);
}

/// Viewer pages are written and restored in the order their slots were reserved, not slot by slot: a pane page handed
/// over before the session-wide page stays under it once the next presenter is handed both again.
#[test]
fn viewer_pages_keep_their_opening_order_across_a_restart() {
    let (mut store, session) = store_with_split();
    let handed = |path: &str| {
        let url = HtmlSource::Url(Url::parse(&format!("https://example.com/{path}")).unwrap());
        RemotePage::new(page(url, HtmlPageFlags::default()), "t".to_owned(), Instant::now())
    };
    hand_over(
        &mut store,
        session,
        Some(OverlayPane::Left),
        handed("first"),
        OverlaySize::default(),
    );
    hand_over(&mut store, session, None, handed("second"), OverlaySize::default());
    let written: Vec<PageSlotSnapshot> = store.snapshot().workspaces[0].sessions[0]
        .pages
        .clone()
        .unwrap()
        .iter()
        .map(|entry| entry.slot)
        .collect();
    assert_eq!(written, [PageSlotSnapshot::Left, PageSlotSnapshot::Session]);

    let back = round_trip(&store);
    let order: Vec<Option<OverlayPane>> = only_session(&back)
        .remote_overlays()
        .slots()
        .iter()
        .map(|slot| slot.pane)
        .collect();
    assert_eq!(order, [Some(OverlayPane::Left), None]);
}

/// A log page is written with its request, `--tail` and `--follow-file`, never with what it last read or whether it
/// was paused since; a restore opens it following as requested, nothing read yet.
#[test]
fn a_log_page_round_trips_its_request_not_its_state() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let text = HtmlSource::Text {
        path: files.path("main.go"),
        line: None,
    };
    let log = TextLog { tail: 50, follow: true };
    let id = open(
        &mut store,
        session,
        None,
        page(text.clone(), HtmlPageFlags::default()).with_text_log(Some(log)),
        OverlaySize::default(),
    );
    store.session_mut(session).unwrap().update_html_overlay(id, |page| {
        page.log_state = Some(TextLogState {
            following: false,
            report: TextLogReport {
                lines: 12,
                clipped: true,
                waiting: false,
                unreadable: true,
            },
        });
    });
    let written = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(written.contains("\"tail\":50,\"follow\":true"), "{written}");
    for key in ["following", "lines", "clipped", "unreadable"] {
        assert!(!written.contains(key), "{key}: {written}");
    }
    let back = round_trip(&store);
    let (_, restored) = only_session(&back).html_overlays()[0];
    assert_eq!(restored.source, text);
    assert_eq!(restored.text_log, Some(log));
    assert_eq!(restored.log_state, Some(TextLogState::new(log)));

    // a plain text page writes neither key and comes back plain
    let (mut store, session) = store_with_split();
    open(
        &mut store,
        session,
        None,
        page(text, HtmlPageFlags::default()),
        OverlaySize::default(),
    );
    let written = serde_json::to_string(&store.snapshot()).unwrap();
    assert!(!written.contains("tail") && !written.contains("follow"), "{written}");
    let back = round_trip(&store);
    assert_eq!(only_session(&back).html_overlays()[0].1.text_log, None);
}

/// A log entry is checked again as `session.overlay.open` checks it: a tail in 1...100000 and no line beside it; a
/// follow alone takes the default tail. A viewer entry never holds a log page, so one claiming to is dropped.
#[test]
fn a_log_entry_rechecks_its_request_on_restore() {
    let files = Files::new();
    let log = |line: Option<i64>, tail: Option<i64>, follow: bool| PageSourceSnapshot::Text {
        path: files.path("main.go"),
        line,
        tail,
        follow,
    };
    let restored_log = |source: PageSourceSnapshot, viewer: bool| {
        let snapshot = persisted(
            vec![PageSnapshot {
                viewer,
                ..entry(PageSlotSnapshot::Session, source)
            }],
            false,
        );
        let session = crate::model::snapshot::session_from_snapshot(&snapshot, false, "/home/u");
        let local = session.html_overlays().first().map(|(_, page)| page.text_log);
        let parked = session.remote_overlays().slot(None).is_some();
        (local, parked)
    };
    let opened = |tail, follow| (Some(Some(TextLog { tail, follow })), false);
    assert_eq!(restored_log(log(None, Some(1), false), false), opened(1, false));
    assert_eq!(
        restored_log(log(None, Some(100_000), true), false),
        opened(100_000, true)
    );
    assert_eq!(restored_log(log(None, None, true), false), opened(5_000, true));
    assert_eq!(restored_log(log(Some(3), None, false), false), (Some(None), false));
    for dropped in [
        log(None, Some(0), false),
        log(None, Some(100_001), true),
        log(None, Some(-4), false),
        log(Some(3), Some(10), false),
        log(Some(3), None, true),
    ] {
        assert_eq!(restored_log(dropped.clone(), false), (None, false), "{dropped:?}");
    }
    assert_eq!(
        restored_log(log(None, Some(10), true), true),
        (None, false),
        "no viewer log page"
    );
}

/// A presenter-only origin closes a log page a restore opened, as nothing there shows it and a presenter would get it
/// without its log state.
#[test]
fn a_presenter_only_origin_closes_a_log_page() {
    let files = Files::new();
    let (mut store, session) = store_with_split();
    let text = HtmlSource::Text {
        path: files.path("main.go"),
        line: None,
    };
    let log = Some(TextLog { tail: 10, follow: true });
    open(
        &mut store,
        session,
        None,
        page(text, HtmlPageFlags::default()).with_text_log(log),
        OverlaySize::default(),
    );
    let swept = crate::model::page_snapshot::park_local_pages(store.session_mut(session).unwrap());
    assert!(swept.parked.is_empty() && swept.dropped);
    let found = store.session(session).unwrap();
    assert!(!found.overlay.active && found.remote_overlays().slot(None).is_none());
}
