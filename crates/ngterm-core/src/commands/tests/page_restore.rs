//! Pages across a restart (an extension beyond upstream, whose overlays are ephemeral), through the whole chain on the
//! fakes: a page open at quit is written with its session, the next launch opens it again under a fresh id whose outcome
//! waits for its answer, and a page that opens, closes, hides or resizes asks its window for a save, so a crash neither
//! loses an open page nor revives a closed one or its old state.

use std::path::Path;

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{AppIdentity, Args, Command, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{args, send, short_tempdir, wire};
use crate::html::{HtmlHost, HtmlNavigation};
use crate::persist::{AppSettings, LibraryPersistence, PageSlotSnapshot, PageSnapshot};
use crate::{Core, CoreConfig, CoreState};

/// Web views that open every page.
struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _page: Uuid, _navigation: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

/// A launch over the state in `dir`, with web views; its first open window and that window's first session.
fn launch(dir: &Path) -> (Core, Uuid, Uuid) {
    let mut persistence = LibraryPersistence::new(dir);
    let library = persistence.bootstrap("/home/u");
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: AppSettings::default(),
            persistence: Some(persistence),
        },
    );
    core.set_html_host(Box::new(Host));
    let _ = core.commit();
    let window = core.library().open_ids()[0];
    let session = core.library().store(window).expect("the window is open").workspaces()[0].sessions[0].id();
    (core, window, session)
}

fn overlay(core: &mut Core, cmd: SessionCmd, session: Uuid, edit: impl FnOnce(&mut Args)) -> Response {
    send(core, Command::Session(cmd), Some(&wire(session)), args(edit))
}

/// Opens the file page `file` in the session-wide slot of `session`, granted its folder; its page id.
fn open_file_page(core: &mut Core, session: Uuid, file: &Path) -> Uuid {
    let response = overlay(core, SessionCmd::OverlayOpen, session, |a| {
        a.html = Some(file.to_string_lossy().into_owned());
        a.cwd = file.parent().map(|folder| folder.to_string_lossy().into_owned());
        a.size_percent = Some(40);
    });
    assert!(response.ok, "{response:?}");
    let id = response.result.expect("a result")["pageID"]
        .as_str()
        .expect("a page id")
        .to_owned();
    Uuid::parse_str(&id).expect("a page id")
}

fn outcome(core: &mut Core, page: Uuid) -> Response {
    send(
        core,
        Command::Session(SessionCmd::OverlayResult),
        None,
        args(|a| a.page = Some(wire(page))),
    )
}

/// The `htmlOverlays` the tree reads back for `session`.
fn html_overlays(core: &mut Core, session: Uuid) -> Value {
    let tree = send(core, Command::Tree, None, Args::default()).result.expect("a tree")["tree"].clone();
    tree["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|workspace| workspace["sessions"].as_array().cloned().unwrap_or_default())
        .find(|node| node["id"] == json!(wire(session)))
        .expect("the session's node")["htmlOverlays"]
        .clone()
}

/// Lands the debounced saves a commit armed, as the app's timers do once they ran out; how many there were.
fn land_debounced_saves(core: &mut Core) -> usize {
    let saves = core.commit().debounced;
    let count = saves.len();
    for save in saves {
        core.fire_debounced(save);
    }
    count
}

/// How many pages window `window`'s file holds for its first session.
fn written_pages(dir: &Path, window: Uuid) -> usize {
    let snapshot = LibraryPersistence::new(dir)
        .read_window_checked(window)
        .expect("the window's file");
    snapshot.workspaces[0].sessions[0].pages.as_ref().map_or(0, Vec::len)
}

/// The first page window `window`'s file holds for its first session.
fn written_page(dir: &Path, window: Uuid) -> PageSnapshot {
    let snapshot = LibraryPersistence::new(dir)
        .read_window_checked(window)
        .expect("the window's file");
    snapshot.workspaces[0].sessions[0].pages.as_ref().expect("the pages")[0].clone()
}

#[test]
fn a_page_open_at_quit_comes_back_pending_under_a_fresh_id_and_takes_a_submit() {
    let state = short_tempdir();
    let site = tempfile::tempdir().expect("a temp dir");
    let file = site.path().join("page.html");
    std::fs::write(&file, "<p>hi</p>").expect("the page");
    let (mut core, _, session) = launch(state.path());
    let before = open_file_page(&mut core, session, &file);
    assert!(core.save_all());
    drop(core);

    let (mut core, _, restored_session) = launch(state.path());
    assert_eq!(restored_session, session);
    let pages = html_overlays(&mut core, session);
    assert_eq!(pages.as_array().map(Vec::len), Some(1), "{pages}");
    assert_eq!(pages[0]["file"], json!(file.to_string_lossy()));
    let after = Uuid::parse_str(pages[0]["id"].as_str().expect("an id")).expect("an id");
    assert_ne!(after, before, "a restore mints a fresh page id");
    assert_eq!(
        outcome(&mut core, after).result,
        Some(json!({"pageOutcome": {"pageID": wire(after), "outcome": "pending"}}))
    );
    assert_eq!(outcome(&mut core, before), Response::failure("no such page"));
    let submitted = overlay(&mut core, SessionCmd::OverlaySubmit, session, |a| {
        a.value = Some("yes".to_owned());
    });
    assert!(submitted.ok, "{submitted:?}");
    let _ = core.commit();
    assert_eq!(
        outcome(&mut core, after).result,
        Some(json!({"pageOutcome": {"pageID": wire(after), "outcome": "submitted", "value": "yes"}}))
    );
}

#[test]
fn a_page_that_opens_or_closes_asks_its_window_for_a_save() {
    let state = short_tempdir();
    let site = tempfile::tempdir().expect("a temp dir");
    let file = site.path().join("page.html");
    std::fs::write(&file, "<p>hi</p>").expect("the page");
    let (mut core, window, session) = launch(state.path());
    let _ = land_debounced_saves(&mut core);
    assert_eq!(written_pages(state.path(), window), 0);

    open_file_page(&mut core, session, &file);
    assert!(land_debounced_saves(&mut core) > 0, "an open asks for a save");
    assert_eq!(written_pages(state.path(), window), 1);
    let page = written_page(state.path(), window);
    assert_eq!((page.slot, page.size_percent), (PageSlotSnapshot::Session, Some(40)));
    // nothing about the page moved, so the next commit asks for nothing
    assert_eq!(land_debounced_saves(&mut core), 0);

    assert!(overlay(&mut core, SessionCmd::OverlayClose, session, |_| {}).ok);
    assert!(land_debounced_saves(&mut core) > 0, "a close asks for a save");
    assert_eq!(written_pages(state.path(), window), 0);
}

#[test]
fn a_page_that_hides_or_resizes_asks_its_window_for_a_save() {
    let state = short_tempdir();
    let site = tempfile::tempdir().expect("a temp dir");
    let file = site.path().join("page.html");
    std::fs::write(&file, "<p>hi</p>").expect("the page");
    let (mut core, window, session) = launch(state.path());
    open_file_page(&mut core, session, &file);
    let _ = land_debounced_saves(&mut core);
    assert!(!written_page(state.path(), window).hidden);

    assert!(overlay(&mut core, SessionCmd::OverlayHide, session, |_| {}).ok);
    assert!(land_debounced_saves(&mut core) > 0, "a hide asks for a save");
    assert!(written_page(state.path(), window).hidden);
    let resized = overlay(&mut core, SessionCmd::OverlayResize, session, |a| {
        a.size_percent = Some(70);
    });
    assert!(resized.ok, "{resized:?}");
    assert!(land_debounced_saves(&mut core) > 0, "a resize asks for a save");
    assert_eq!(written_page(state.path(), window).size_percent, Some(70));
    assert_eq!(land_debounced_saves(&mut core), 0, "nothing changed since");
}

/// A launch over the state in `dir` as a presenter-only origin (headless): no web views; its first open window and
/// that window's first session.
fn launch_presenter_only(dir: &Path) -> (Core, Uuid, Uuid) {
    let mut persistence = LibraryPersistence::new(dir);
    let library = persistence.bootstrap("/home/u");
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: AppSettings::default(),
            persistence: Some(persistence),
        },
    );
    core.set_page_presenter_only();
    let _ = core.commit();
    let window = core.library().open_ids()[0];
    let session = core.library().store(window).expect("the window is open").workspaces()[0].sessions[0].id();
    (core, window, session)
}

/// A presenter-only origin, which renders no page, parks what a restore opened in its own slots: the page waits for a
/// presenter under a fresh id, pending, written as the viewer's, and comes back parked from that entry too, on any
/// origin. Closing it ends it and writes it no more.
#[test]
fn a_presenter_only_origin_restores_a_page_parked_for_its_viewer() {
    let state = short_tempdir();
    let site = tempfile::tempdir().expect("a temp dir");
    let file = site.path().join("page.html");
    std::fs::write(&file, "<p>hi</p>").expect("the page");
    let (mut core, _, session) = launch(state.path());
    let before = open_file_page(&mut core, session, &file);
    assert!(core.save_all());
    drop(core);

    let (mut core, window, _) = launch_presenter_only(state.path());
    let pages = html_overlays(&mut core, session);
    assert_eq!(pages.as_array().map(Vec::len), Some(1), "{pages}");
    assert_eq!((&pages[0]["remote"], &pages[0]["parked"]), (&json!(true), &json!(true)));
    let parked = Uuid::parse_str(pages[0]["id"].as_str().expect("an id")).expect("an id");
    assert_ne!(parked, before);
    assert_eq!(
        outcome(&mut core, parked).result,
        Some(json!({"pageOutcome": {"pageID": wire(parked), "outcome": "pending"}}))
    );
    assert!(core.save_all());
    let written = written_page(state.path(), window);
    assert!(written.viewer && written.size_percent == Some(40), "{written:?}");
    drop(core);

    // the viewer's entry parks again, on an origin with web views too
    let (mut core, window, _) = launch(state.path());
    let pages = html_overlays(&mut core, session);
    assert_eq!((&pages[0]["remote"], &pages[0]["parked"]), (&json!(true), &json!(true)));
    let again = Uuid::parse_str(pages[0]["id"].as_str().expect("an id")).expect("an id");
    assert_ne!(again, parked);
    let pending = outcome(&mut core, again).result.expect("a result");
    assert_eq!(pending["pageOutcome"]["outcome"], "pending");
    let _ = land_debounced_saves(&mut core);
    assert!(overlay(&mut core, SessionCmd::OverlayClose, session, |_| {}).ok);
    assert!(
        land_debounced_saves(&mut core) > 0,
        "a parked page's close asks for a save"
    );
    assert_eq!(written_pages(state.path(), window), 0);
    let closed = outcome(&mut core, again).result.expect("a result");
    assert_eq!(closed["pageOutcome"]["outcome"], "dismissed");
}
