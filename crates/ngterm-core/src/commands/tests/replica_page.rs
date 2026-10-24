//! Pages an origin handed to this machine, on the presenter (upstream's `AppStoreReplicaPageTests` and the core-level
//! cases of `ControlServerRemotePageTests`): the page placed under its own id, refused with its reason, closed by its
//! origin or here, hidden, resized, reloaded and navigated on the origin's command, its state mirrored back once per
//! commit, its channel's file reads and bridge calls, and every way it ends telling the origin once.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::viewer::{Viewer, viewer_built};
use super::{Fixture, args, failure, wire};
use crate::html::{HtmlHost, HtmlLoadState, HtmlNavigation, HtmlPageInfo, HtmlReloadTarget, HtmlSource};
use crate::model::OverlayPane;
use crate::remote::hub::SUPPORTED_KINDS;
use crate::remote::page_channel::{ChannelLink, Permits};
use crate::remote::page_serve::{self, PageServeFrame};
use crate::remote::replica_page::{HelperKey, PageHelperLauncher, PageResource};

const ORIGIN: &str = "BBBBBBBB-0000-0000-0000-000000000001";
const REMOTE_FILE: &str = "the page's file is on another Mac";

// MARK: - fakes

/// The web views, recording every navigation they are asked for.
#[derive(Default)]
struct HostState {
    navigations: Vec<(Uuid, HtmlNavigation)>,
    persistent_failure: Option<String>,
    unavailable: Option<String>,
}

#[derive(Clone, Default)]
struct Host(Rc<RefCell<HostState>>);

impl HtmlHost for Host {
    fn web_views_unavailable(&self) -> Option<String> {
        self.0.borrow().unavailable.clone()
    }

    fn persistent_store_failure(&self) -> Option<String> {
        self.0.borrow().persistent_failure.clone()
    }

    fn navigate(&self, page: Uuid, navigation: HtmlNavigation) -> Result<(), String> {
        self.0.borrow_mut().navigations.push((page, navigation));
        Ok(())
    }
}

/// One helper the launcher started: the lines the channel wrote to it, and whether it was stopped.
#[derive(Default)]
struct HelperState {
    sent: Vec<PageServeFrame>,
    stopped: bool,
}

#[derive(Clone, Default)]
struct Helper(Rc<RefCell<HelperState>>);

impl ChannelLink for Helper {
    fn send(&self, line: Vec<u8>) -> bool {
        self.0
            .borrow_mut()
            .sent
            .push(page_serve::decode(&line[..line.len() - 1]).unwrap());
        true
    }

    fn stop(&self) {
        self.0.borrow_mut().stopped = true;
    }
}

struct Launch {
    argv: Vec<String>,
    key: HelperKey,
    permits: Arc<Permits>,
    helper: Helper,
}

#[derive(Default)]
struct Launcher {
    launches: RefCell<Vec<Launch>>,
    refuse: RefCell<bool>,
}

impl PageHelperLauncher for Launcher {
    fn launch(&self, argv: &[String], key: HelperKey, permits: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
        if *self.refuse.borrow() {
            return Err("ssh: not found".to_owned());
        }
        let helper = Helper::default();
        self.launches.borrow_mut().push(Launch {
            argv: argv.to_vec(),
            key,
            permits,
            helper: helper.clone(),
        });
        Ok(Box::new(helper))
    }
}

/// An attached row of the origin's split session on a viewer that shows pages, connected and presenting.
struct Replica {
    viewer: Viewer,
    host: Host,
    launcher: Rc<Launcher>,
}

impl Replica {
    fn new() -> Self {
        Self::built(true)
    }

    fn built(split: bool) -> Self {
        let host = Host::default();
        let launcher = Rc::new(Launcher::default());
        let (installed, helpers) = (host.clone(), Rc::clone(&launcher));
        let mut viewer = viewer_built(split, move |fixture: &mut Fixture| {
            fixture.core.set_html_host(Box::new(installed));
            fixture.core.set_page_helper_launcher(helpers);
        });
        viewer.connect(&json!({}));
        Self { viewer, host, launcher }
    }

    fn session(&self) -> &crate::model::Session {
        self.viewer.session()
    }

    fn id(&self) -> Uuid {
        self.viewer.session
    }

    /// A page request of `source` from the origin.
    fn page(source: Value) -> Value {
        let mut page = json!({"page": ORIGIN, "token": "tok-1", "identity": "index.html", "chromeless": false,
            "javascript": true, "navigation": true, "browse": false, "persistent": false,
            "backgroundColor": "#102030", "follow": false});
        page["source"] = source;
        page
    }

    fn file(grant: Option<&str>) -> Value {
        let mut file = json!({"path": "/docs/index.html"});
        if let Some(grant) = grant {
            file["grantRoot"] = json!(grant);
        }
        json!({"file": file})
    }

    fn url(address: &str) -> Value {
        json!({"url": {"_0": address}})
    }

    fn request(&mut self, page: Value) {
        let mut frame = json!({"kind": "page.request"});
        frame["page"] = page;
        self.viewer.frame(frame);
    }

    fn origin_frame(&mut self, kind: &str, payload: Value) -> usize {
        let mut frame = json!({"kind": kind});
        frame["page"] = payload;
        self.viewer.frame_committed(frame)
    }

    fn rejections(&self) -> Vec<Value> {
        self.payloads("page.rejected")
    }

    fn closed_notices(&self) -> usize {
        self.payloads("page.closed").len()
    }

    fn states(&self) -> Vec<Value> {
        self.payloads("page.state")
    }

    fn payloads(&self, kind: &str) -> Vec<Value> {
        self.viewer
            .sent(kind)
            .into_iter()
            .map(|frame| frame["page"].clone())
            .collect()
    }

    /// The session-wide page's id.
    fn local(&self) -> Uuid {
        self.session().overlay.page().expect("a session-wide page").id
    }

    fn pane_page(&self, pane: OverlayPane) -> Option<&crate::html::HtmlOverlay> {
        self.session()
            .pane_overlay(pane)
            .and_then(|overlay| overlay.html.as_ref())
    }

    fn commit(&mut self) {
        self.viewer.fixture.core.commit();
    }

    fn launch(&self, index: usize) -> std::cell::Ref<'_, Launch> {
        std::cell::Ref::map(self.launcher.launches.borrow(), |launches| &launches[index])
    }

    /// Hands the helper `index` one line from the origin, as its reader does, holding a permit.
    fn helper_line(&mut self, index: usize, frame: &PageServeFrame) {
        let (key, permits) = {
            let launch = self.launch(index);
            (launch.key, Arc::clone(&launch.permits))
        };
        assert!(permits.try_take());
        let mut line = page_serve::encode(frame).unwrap();
        line.pop();
        self.viewer.fixture.core.page_helper_line(key, &line);
        self.commit();
    }

    fn helper_sent(&self, index: usize) -> Vec<PageServeFrame> {
        self.launch(index).helper.0.borrow().sent.clone()
    }
}

type Fetched = Rc<RefCell<Option<Result<PageResource, String>>>>;

/// Reads `path` of replica `page` through its channel; the answer lands in the cell.
fn fetch(replica: &mut Replica, page: Uuid, path: &str) -> Fetched {
    let cell: Fetched = Rc::default();
    let into = Rc::clone(&cell);
    replica
        .viewer
        .fixture
        .core
        .replica_fetch(page, path, Box::new(move |result| *into.borrow_mut() = Some(result)));
    cell
}

// MARK: - placing a page

/// Upstream's `aFilePageIsShownSessionWideUnderItsOwnId`.
#[test]
fn a_file_page_is_shown_session_wide_under_its_own_id() {
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::file(Some("/docs")));
    page["sizePercent"] = json!(70);

    replica.request(page);

    assert!(replica.rejections().is_empty(), "{:?}", replica.rejections());
    let session = replica.session();
    let shown = session.overlay.page().unwrap();
    assert!(session.html_overlay_active());
    assert_eq!(
        shown.source,
        HtmlSource::Replica {
            path: "/docs/index.html".to_owned()
        }
    );
    assert_eq!(shown.replica_of.map(wire).as_deref(), Some(ORIGIN));
    assert_ne!(wire(shown.id), ORIGIN);
    assert!(shown.flags.javascript && shown.flags.navigation);
    assert_eq!(session.overlay.size_percent, Some(70));
    assert_eq!(session.overlay.background_color.as_deref(), Some("#102030"));
    let local = shown.id;
    assert!(
        replica
            .viewer
            .fixture
            .session(SessionCmd::OverlayResult, None, args(|a| a.page = Some(wire(local))))
            == failure("no such page"),
        "no outcome is registered for a replica"
    );
}

/// Upstream's `aPageWithoutAGrantIsShownAsAReplicaOfItsFile`.
#[test]
fn a_page_without_a_grant_is_shown_as_a_replica_of_its_file() {
    let mut replica = Replica::new();

    replica.request(Replica::page(Replica::file(None)));

    assert_eq!(
        replica.session().overlay.page().map(|page| page.source.clone()),
        Some(HtmlSource::Replica {
            path: "/docs/index.html".to_owned()
        })
    );
}

/// Upstream's `aUrlPageLoadsItsFullAddressHere` and `ControlServerRemotePageTests.testAUrlReplicaNeedsNoHelper…`.
#[test]
fn a_url_page_loads_its_full_address_here_without_a_helper() {
    let mut replica = Replica::new();

    replica.request(Replica::page(Replica::url("https://example.com/a?b=1")));

    let shown = replica.session().overlay.page().unwrap();
    assert_eq!(
        shown.source,
        HtmlSource::Url("https://example.com/a?b=1".parse().unwrap())
    );
    assert_eq!(shown.replica_of.map(wire).as_deref(), Some(ORIGIN));
    assert!(replica.launcher.launches.borrow().is_empty());
    assert!(!replica.viewer.fixture.core.linked_replica(shown.id));
}

/// Upstream's `aPagePresentedForTheOriginsSplitTakesTheMappedPane`.
#[test]
fn a_page_presented_for_the_origins_split_takes_the_mapped_pane() {
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::file(Some("/docs")));
    page["pane"] = json!(wire(replica.viewer.origin.1));

    replica.request(page);

    assert_eq!(
        replica
            .pane_page(OverlayPane::Right)
            .and_then(|page| page.replica_of)
            .map(wire)
            .as_deref(),
        Some(ORIGIN)
    );
    assert!(!replica.session().html_overlay_active());
}

/// Upstream's `aPageThatCannotBePlacedIsRefused` and `aTakenSlotRefusesThePage`, with the presenter's own refusals.
#[test]
fn a_page_that_cannot_be_placed_is_refused_with_its_reason() {
    let unknown = wire(Uuid::new_v4());
    let cases: Vec<(Value, &str)> = vec![
        (Replica::page(Replica::url("not a url")), "invalid url"),
        (
            {
                let mut page = Replica::page(json!({"file": {"path": "/a.html"}}));
                page["pane"] = json!(unknown);
                page
            },
            "pane not visible",
        ),
        (
            {
                let mut page = Replica::page(Replica::file(None));
                page["page"] = json!("not-a-uuid");
                page
            },
            "invalid page id",
        ),
        (
            {
                let mut page = Replica::page(Replica::file(None));
                page["token"] = json!("t 1");
                page
            },
            "the origin cannot be reached for this page",
        ),
    ];
    for (page, reason) in cases {
        let mut replica = Replica::new();
        replica.request(page);
        let rejected = replica.rejections();
        assert_eq!(rejected.len(), 1, "{reason}: {rejected:?}");
        assert_eq!(rejected[0]["reason"], json!(reason));
        assert!(!replica.session().overlay.active, "{reason}");
    }
}

#[test]
fn a_taken_slot_refuses_the_page() {
    let mut replica = Replica::new();
    let id = replica.id();
    assert!(replica.viewer.fixture.store_mut().unwrap().open_overlay(
        id,
        crate::model::OverlayProgram {
            command: "top".to_owned(),
            ..crate::model::OverlayProgram::default()
        }
    ));

    replica.request(Replica::page(Replica::file(None)));

    assert_eq!(
        replica.rejections(),
        [json!({"page": ORIGIN, "reason": "overlay already open"})]
    );
    assert!(replica.session().overlay.page().is_none());
}

/// A viewer without web views refuses every page (upstream's default `pageRequest` effect), and lists no page kind;
/// one that can show them lists them all.
#[test]
fn a_viewer_without_web_views_refuses_pages_and_lists_no_page_kind() {
    let mut replica = Replica::new();
    let hello = replica.viewer.sent("hello");
    assert_eq!(hello[0]["hello"]["kinds"], json!(SUPPORTED_KINDS));

    replica.host.0.borrow_mut().unavailable = Some("no GPU".to_owned());
    replica.request(Replica::page(Replica::url("https://example.com/")));

    assert_eq!(
        replica.rejections(),
        [json!({"page": ORIGIN, "reason": "pages are not shown on this Mac"})]
    );
    let plain = viewer_built(false, |_| {});
    let kinds = plain.sent("hello")[0]["hello"]["kinds"].clone();
    assert!(
        kinds
            .as_array()
            .unwrap()
            .iter()
            .all(|kind| !kind.as_str().unwrap().starts_with("page.")),
        "{kinds}"
    );
}

#[test]
fn a_persistent_url_page_needs_the_saved_store() {
    let mut replica = Replica::new();
    replica.host.0.borrow_mut().persistent_failure = Some("persistent browser storage is not available".to_owned());
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["persistent"] = json!(true);

    replica.request(page);

    assert_eq!(
        replica.rejections(),
        [json!({"page": ORIGIN, "reason": "persistent browser storage is not available"})]
    );
}

/// Upstream's `followSelectsTheSession`.
#[test]
fn follow_selects_the_session() {
    let mut replica = Replica::new();
    let workspace = replica.viewer.fixture.workspace;
    let other = replica.viewer.fixture.add_session(workspace);
    replica.viewer.fixture.store_mut().unwrap().select_session(Some(other));
    let mut page = Replica::page(Replica::file(None));
    page["follow"] = json!(true);

    replica.request(page);

    assert_eq!(replica.viewer.fixture.selected(), Some(replica.id()));
}

// MARK: - closing

/// Upstream's `closingByOriginIdFreesTheSlot`; the origin hears `page.closed` all the same.
#[test]
fn closing_by_origin_id_frees_the_slot() {
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::file(None));
    page["pane"] = json!(wire(replica.viewer.origin.1));
    replica.request(page);

    replica.origin_frame("page.close", json!({"page": wire(Uuid::new_v4())}));
    assert!(replica.pane_page(OverlayPane::Right).is_some());
    replica.origin_frame("page.close", json!({"page": ORIGIN}));

    assert!(replica.pane_page(OverlayPane::Right).is_none());
    assert_eq!(replica.closed_notices(), 1);
    assert!(replica.launch(0).helper.0.borrow().stopped, "its channel closed");
}

/// Upstream's `losingTheStreamClosesThePages` and `testEachTeardownTriggerEndsThePageOnce`'s last trigger.
#[test]
fn losing_the_stream_closes_the_pages_and_their_helpers() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::file(None)));

    let key = replica.viewer.link().0.borrow().key.unwrap();
    replica.viewer.fixture.core.viewer_closed(key, "exit 255");
    replica.commit();

    assert!(!replica.session().overlay.active);
    assert!(replica.session().overlay.page().is_none());
    assert!(replica.launch(0).helper.0.borrow().stopped);
}

/// Upstream's `aSocketSubmitOnAReplicaIsRefusedAndLeavesItOpen`.
#[test]
fn a_socket_submit_on_a_replica_is_refused_and_leaves_it_open() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("http://127.0.0.1:9/a")));
    let target = wire(replica.id());

    let response = replica.viewer.fixture.session(
        SessionCmd::OverlaySubmit,
        Some(&target),
        args(|a| a.value = Some("x".to_owned())),
    );
    replica.commit();

    assert_eq!(response, failure("submit on the Mac that opened the page"));
    assert!(replica.session().html_overlay_active());
    assert_eq!(replica.closed_notices(), 0);
}

/// Upstream's `closingAUrlReplicaHereTellsItsOriginOnce`.
#[test]
fn closing_a_url_replica_here_tells_its_origin_once() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    let id = replica.id();

    for _ in 0..2 {
        replica.viewer.fixture.store_mut().unwrap().close_overlay(id);
        replica.commit();
    }

    assert_eq!(replica.payloads("page.closed"), [json!({"page": ORIGIN})]);
}

/// Upstream's `removingTheSessionTellsTheOriginOfAPanePage`.
#[test]
fn removing_the_session_tells_the_origin_of_a_pane_page() {
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["pane"] = json!(wire(replica.viewer.origin.1));
    replica.request(page);
    let id = replica.id();
    let link = replica.viewer.link();

    replica.viewer.fixture.store_mut().unwrap().close_session(id);
    replica.commit();

    let closed: Vec<Value> = link
        .0
        .borrow()
        .sent
        .iter()
        .filter(|frame| frame["kind"] == "page.closed")
        .cloned()
        .collect();
    assert_eq!(closed.len(), 1, "{closed:?}");
    assert_eq!(closed[0]["page"], json!({"page": ORIGIN}));
}

/// Upstream's `aLocalPageIsNoReplicaRelease` and `aLocalPageReportsNoReplicaChange`.
#[test]
fn a_local_page_is_no_replica() {
    let mut replica = Replica::new();
    let id = replica.id();
    let local = crate::html::HtmlOverlay::new(
        HtmlSource::File {
            path: "/a.html".to_owned(),
            grant_root: None,
        },
        crate::html::HtmlPageFlags::default(),
    );
    let local_id = local.id;
    replica
        .viewer
        .fixture
        .store_mut()
        .unwrap()
        .open_html_overlay(id, None, local, None, None)
        .unwrap();
    replica.commit();

    replica
        .viewer
        .fixture
        .core
        .set_html_load_state(local_id, HtmlLoadState::Loaded, None);
    replica.commit();
    replica.viewer.fixture.store_mut().unwrap().close_overlay(id);
    replica.commit();

    assert!(replica.states().is_empty());
    assert_eq!(replica.closed_notices(), 0);
}

// MARK: - the origin's commands and the state mirrored back

/// Upstream's `hidingTheReplicaLeavesItsNeighbourAlone` and `testAnOriginCommandIsAppliedAndReportedWithItsSequence`.
#[test]
fn hiding_the_replica_leaves_its_neighbour_alone_and_reports_the_sequence() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    let id = replica.id();
    replica
        .viewer
        .fixture
        .store_mut()
        .unwrap()
        .open_pane_overlay(
            id,
            OverlayPane::Right,
            crate::model::OverlayProgram {
                command: "top".to_owned(),
                ..crate::model::OverlayProgram::default()
            },
        )
        .unwrap();
    replica.commit();

    let changed = replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 5, "action": "hide"}));

    assert!(replica.session().overlay_hidden(None));
    assert!(!replica.session().overlay_hidden(Some(OverlayPane::Right)));
    assert!(changed > 0, "the hide invalidates the tree");
    let states = replica.states();
    assert_eq!(states.len(), 1, "{states:?}");
    assert_eq!(states[0]["hidden"], json!(true));
    assert_eq!(states[0]["appliedSeq"], json!(5));
    assert_eq!(states[0]["page"], json!(ORIGIN));
}

/// Upstream's `aLocalHideOfTheReplicaIsReported`.
#[test]
fn a_local_hide_of_the_replica_is_reported() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    let id = replica.id();

    replica
        .viewer
        .fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(id, None, true)
        .unwrap();
    replica.commit();

    let states = replica.states();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0]["hidden"], json!(true));
    assert_eq!(states[0]["appliedSeq"], json!(0));
}

/// Upstream's `theStateMirrorsWhatThePageReported` and `testStateChangesInOneTurnAreReportedOnce`.
#[test]
fn the_state_mirrors_what_the_page_reported_once_per_commit() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::file(Some("/docs"))));
    let local = replica.local();
    assert!(replica.states().is_empty(), "the page starts as the origin assumed");

    let core = &mut replica.viewer.fixture.core;
    core.set_html_load_state(local, HtmlLoadState::Failed, Some("one".to_owned()));
    core.set_html_load_state(local, HtmlLoadState::Loaded, Some("two".to_owned()));
    core.set_html_page(
        local,
        Some(HtmlPageInfo {
            page: "/docs/next.html".to_owned(),
            title: Some("Next".to_owned()),
            can_go_back: true,
            can_go_forward: false,
        }),
    );
    replica.commit();
    replica.commit();

    assert_eq!(
        replica.states(),
        [
            json!({"page": ORIGIN, "title": "Next", "address": "/docs/next.html", "load": "loaded", "error": "two",
            "canGoBack": true, "canGoForward": false, "hidden": false, "appliedSeq": 0})
        ]
    );
}

/// Upstream's `resizeReachesOnlyTheSessionWideSlot`.
#[test]
fn resize_reaches_only_the_session_wide_slot() {
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["sizePercent"] = json!(50);
    replica.request(page);

    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 1, "action": "resize", "sizePercent": 80}),
    );

    assert_eq!(replica.session().overlay.size_percent, Some(80));

    let mut covered = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["pane"] = json!(wire(covered.viewer.origin.1));
    covered.request(page);
    covered.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 1, "action": "resize", "sizePercent": 80}),
    );
    assert_eq!(
        covered.session().overlay.size_percent,
        None,
        "a pane page is always full-pane"
    );
}

#[test]
fn reload_and_history_commands_reach_the_page() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    let local = replica.local();

    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 1, "action": "reloadCurrent"}),
    );
    let page = replica.session().overlay.page().unwrap();
    assert_eq!(
        (page.reload_revision, page.reload_target),
        (1, HtmlReloadTarget::Current)
    );
    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 2, "action": "reloadOriginal"}),
    );
    for (seq, action) in [(3, "back"), (4, "forward"), (5, "browser")] {
        replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": seq, "action": action}));
    }

    let page = replica.session().overlay.page().unwrap();
    assert_eq!(
        (page.reload_revision, page.reload_target),
        (2, HtmlReloadTarget::Original)
    );
    assert_eq!(
        replica.host.0.borrow().navigations,
        [
            (local, HtmlNavigation::Back),
            (local, HtmlNavigation::Forward),
            (local, HtmlNavigation::Browser),
        ]
    );
    assert_eq!(replica.states().last().unwrap()["appliedSeq"], json!(5));
}

/// Upstream's `testFinderAndBrowserAreRefusedOnAFileReplicaButBrowserOpensAUrlReplica`, on the socket and on the
/// origin's command.
#[test]
fn finder_and_browser_are_refused_on_a_file_replica() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::file(None)));
    let target = wire(replica.id());

    for navigation in ["finder", "browser"] {
        let response = replica.viewer.fixture.session(
            SessionCmd::OverlayNavigate,
            Some(&target),
            args(|a| a.to = Some(navigation.to_owned())),
        );
        assert_eq!(response, failure(REMOTE_FILE), "{navigation}");
    }
    replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 1, "action": "browser"}));
    assert!(replica.host.0.borrow().navigations.is_empty());
    assert_eq!(
        replica.states().last().unwrap()["appliedSeq"],
        json!(1),
        "a refused request still counts as applied"
    );
}

// MARK: - the page's channel

/// Upstream's `testAFileReplicaLoadsItsPageAndAnAssetThroughItsChannel`, at the core: the helper runs the page-serve
/// command, and a read goes over the channel and back.
#[test]
fn a_file_replica_reads_its_files_through_its_helper() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::file(Some("/docs"))));
    let local = replica.local();

    {
        let launch = replica.launch(0);
        assert_eq!(launch.argv[..2], ["ssh", "-T"]);
        let remote = launch.argv.last().unwrap();
        assert!(remote.contains("page-serve") && remote.contains("tok-1"), "{remote}");
        assert_eq!(launch.key.page, local);
    }
    assert!(replica.viewer.fixture.core.linked_replica(local));

    let answer = fetch(&mut replica, local, "/docs/style.css");
    assert_eq!(
        replica.helper_sent(0),
        [PageServeFrame::Resource {
            id: 1,
            path: "/docs/style.css".to_owned()
        }]
    );
    replica.helper_line(
        0,
        &PageServeFrame::Head {
            id: 1,
            mime_type: "text/css".to_owned(),
        },
    );
    for part in [&b"#p { color: "[..], b"red }"] {
        replica.helper_line(
            0,
            &PageServeFrame::Chunk {
                id: 1,
                data: part.to_vec(),
            },
        );
    }
    assert!(answer.borrow().is_none());
    replica.helper_line(0, &PageServeFrame::End { id: 1 });

    assert_eq!(
        answer.borrow_mut().take(),
        Some(Ok(PageResource {
            mime_type: "text/css".to_owned(),
            body: b"#p { color: red }".to_vec(),
        }))
    );
    assert_eq!(
        replica.launch(0).permits.free(),
        crate::remote::page_channel::INBOUND_LIMIT
    );
    let missing = fetch(&mut replica, local, "/docs/missing.css");
    replica.helper_line(
        0,
        &PageServeFrame::Failure {
            id: 2,
            error: "file cannot be read".to_owned(),
        },
    );
    assert_eq!(missing.borrow_mut().take(), Some(Err("file cannot be read".to_owned())));
    // the read-back on this machine marks the replica and names the origin's file
    let node = replica.viewer.node();
    let read = &node["htmlOverlays"][0];
    assert_eq!(read["replica"], json!(true));
    assert_eq!(read["file"], json!("/docs/index.html"));
    assert!(read.get("cwd").is_none());
}

/// Upstream's `testThePageScriptsBridgeRelaysToTheOriginAndKeepsUnknownFields`, at the core.
#[test]
fn a_replicas_bridge_relays_the_request_unchanged_and_shapes_the_reply() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::file(None)));
    let local = replica.local();
    let answers: Rc<RefCell<Vec<Result<Value, String>>>> = Rc::default();
    let body = r#"{"cmd":"tree","args":{"futureArgument":1}}"#;
    for _ in 0..2 {
        let into = Rc::clone(&answers);
        replica
            .viewer
            .fixture
            .core
            .replica_bridge(local, body, Box::new(move |result| into.borrow_mut().push(result)));
    }

    assert_eq!(
        replica.helper_sent(0),
        [
            PageServeFrame::Bridge {
                id: 1,
                request: body.to_owned()
            },
            PageServeFrame::Bridge {
                id: 2,
                request: body.to_owned()
            },
        ]
    );
    replica.helper_line(
        0,
        &PageServeFrame::Reply {
            id: 1,
            response: r#"{"ok":true,"result":{"extra":{"nested":"kept"}}}"#.to_owned(),
        },
    );
    replica.helper_line(
        0,
        &PageServeFrame::Reply {
            id: 2,
            response: r#"{"ok":false,"error":"no such session"}"#.to_owned(),
        },
    );

    assert_eq!(
        *answers.borrow(),
        [
            Ok(json!({"extra": {"nested": "kept"}})),
            Err("no such session".to_owned())
        ]
    );
}

/// Upstream's `testEachTeardownTriggerEndsThePageOnce`: the origin's close, the helper's end and a close here each end
/// the page once and stop its helper.
#[test]
fn each_teardown_trigger_ends_the_page_once() {
    let mut replica = Replica::new();
    let id = replica.id();

    replica.request(Replica::page(Replica::file(None)));
    let local = replica.local();
    let pending = fetch(&mut replica, local, "/docs/a.css");
    replica.origin_frame("page.close", json!({"page": ORIGIN}));
    assert!(!replica.session().overlay.active);
    assert_eq!(replica.closed_notices(), 1);
    assert!(replica.launch(0).helper.0.borrow().stopped);
    assert_eq!(pending.borrow_mut().take(), Some(Err("page channel closed".to_owned())));

    replica.request(Replica::page(Replica::file(None)));
    let key = replica.launch(1).key;
    replica.viewer.fixture.core.page_helper_closed(key, "helper exited");
    replica.commit();
    assert!(!replica.session().overlay.active);
    assert_eq!(replica.closed_notices(), 2);

    replica.request(Replica::page(Replica::file(None)));
    replica.viewer.fixture.store_mut().unwrap().close_overlay(id);
    replica.commit();
    assert_eq!(replica.closed_notices(), 3);
    assert!(replica.launch(2).helper.0.borrow().stopped);

    // a late report from an ended helper reaches nothing
    replica.viewer.fixture.core.page_helper_closed(key, "helper exited");
    replica.commit();
    assert_eq!(replica.closed_notices(), 3);
}

#[test]
fn a_helper_that_cannot_start_closes_the_page() {
    let mut replica = Replica::new();
    *replica.launcher.refuse.borrow_mut() = true;

    replica.request(Replica::page(Replica::file(None)));

    assert!(replica.rejections().is_empty(), "the page was shown, then closed");
    assert!(!replica.session().overlay.active);
    assert_eq!(replica.closed_notices(), 1);
}

/// A read made for a load the page left is dropped: a reload withdraws it, and the scheme request fails.
#[test]
fn a_read_for_a_load_the_page_left_is_withdrawn() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::file(None)));
    let local = replica.local();
    let stale = fetch(&mut replica, local, "/docs/a.css");

    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 1, "action": "reloadOriginal"}),
    );

    assert_eq!(stale.borrow_mut().take(), Some(Err("cancelled".to_owned())));
    assert!(replica.helper_sent(0).contains(&PageServeFrame::Cancel { id: 1 }));
    let fresh = fetch(&mut replica, local, "/docs/a.css");
    replica.helper_line(
        0,
        &PageServeFrame::Head {
            id: 2,
            mime_type: "text/css".to_owned(),
        },
    );
    replica.helper_line(0, &PageServeFrame::End { id: 2 });
    assert_eq!(
        fresh.borrow_mut().take(),
        Some(Ok(PageResource {
            mime_type: "text/css".to_owned(),
            body: Vec::new(),
        }))
    );
}

#[test]
fn a_url_replica_has_no_channel_to_read_through() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    let local = replica.local();

    let answer = fetch(&mut replica, local, "/a.css");

    assert_eq!(answer.borrow_mut().take(), Some(Err(REMOTE_FILE.to_owned())));
}
