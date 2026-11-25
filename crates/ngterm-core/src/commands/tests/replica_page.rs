//! Pages an origin handed to this machine, on the presenter (upstream's `AppStoreReplicaPageTests` and the core-level
//! cases of `ControlServerRemotePageTests`): the page placed under its own id, refused with its reason, closed by its
//! origin or here, hidden, resized, reloaded and navigated on the origin's command, its state mirrored back once per
//! commit, its channel's file reads and bridge calls, and every way it ends telling the origin once.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::viewer::{Viewer, viewer_built};
use super::{Fixture, args, failure, wire};
use crate::dispatch::Dispatched;
use crate::html::text_page::TextChanges;
use crate::html::{
    ChangesAnswer, HtmlHost, HtmlLoadState, HtmlNavigation, HtmlPageInfo, HtmlReloadTarget, HtmlSource, SearchAnswer,
    SearchCount, SearchStep,
};
use crate::model::OverlayPane;
use crate::remote::hub::{PAGE_DETACHED_KIND, SUPPORTED_KINDS};
use crate::remote::page_channel::{ChannelFailure, ChannelLink, Permits};
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
    /// The search and change steps the renderers were asked for, each answered when the test sends its count.
    search_steps: Vec<(Uuid, SearchStep, Sender<SearchCount>)>,
    change_steps: Vec<(Uuid, bool, Sender<TextChanges>)>,
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

    fn step_text_search(&self, page: Uuid, step: SearchStep) -> Option<SearchAnswer> {
        let (sender, answer) = std::sync::mpsc::channel();
        self.0.borrow_mut().search_steps.push((page, step, sender));
        Some(answer)
    }

    fn step_text_changes(&self, page: Uuid, forward: bool) -> Option<ChangesAnswer> {
        let (sender, answer) = std::sync::mpsc::channel();
        self.0.borrow_mut().change_steps.push((page, forward, sender));
        Some(answer)
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
        Self::attached(split, None)
    }

    /// [`Replica::new`] on an origin that speaks `page.detached` when `detaching`, one that predates it otherwise.
    fn origin(detaching: bool) -> Self {
        Self::attached(true, Some(detaching))
    }

    /// An attached row; `detaching` makes the origin's hello answer list the page kinds, and `page.detached` with them
    /// when true.
    fn attached(split: bool, detaching: Option<bool>) -> Self {
        let host = Host::default();
        let launcher = Rc::new(Launcher::default());
        let (installed, helpers) = (host.clone(), Rc::clone(&launcher));
        let mut viewer = viewer_built(split, move |fixture: &mut Fixture| {
            fixture.core.set_html_host(Box::new(installed));
            fixture.core.set_page_helper_launcher(helpers);
        });
        match detaching {
            None => viewer.connect(&json!({})),
            Some(detaching) => {
                let kinds: Vec<&str> = SUPPORTED_KINDS
                    .into_iter()
                    .filter(|kind| detaching || *kind != PAGE_DETACHED_KIND)
                    .collect();
                viewer.connect_listing(&kinds, &json!({}));
            }
        }
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
            path: "/docs/index.html".to_owned(),
            content: crate::html::ReplicaContent::Html,
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
            path: "/docs/index.html".to_owned(),
            content: crate::html::ReplicaContent::Html,
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

// MARK: - a page on the origin's loopback

impl Replica {
    /// The forward of the loopback page requested last: its helper's index, after a request of `address`.
    fn request_loopback(&mut self, address: &str) -> usize {
        self.request(Replica::page(Replica::url(address)));
        assert!(self.rejections().is_empty(), "{:?}", self.rejections());
        self.launcher.launches.borrow().len() - 1
    }

    /// Hands forward `index` a raw line from its ssh, as the reader does, holding a permit.
    fn tunnel_line(&mut self, index: usize, line: &str) {
        let (key, permits) = {
            let launch = self.launch(index);
            (launch.key, Arc::clone(&launch.permits))
        };
        assert!(permits.try_take());
        self.viewer.fixture.core.page_helper_line(key, line.as_bytes());
        self.commit();
        assert_eq!(
            permits.free(),
            crate::remote::page_channel::INBOUND_LIMIT,
            "the line gave its permit back"
        );
    }

    fn tunnel_closed(&mut self, index: usize, reason: &str) {
        let key = self.launch(index).key;
        self.viewer.fixture.core.page_helper_closed(key, reason);
        self.commit();
    }
}

#[test]
fn a_loopback_page_is_placed_once_its_forward_is_up_at_the_forwarded_address() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/board?run=3");
    assert!(
        replica.session().overlay.page().is_none(),
        "nothing loads before the forward answers"
    );
    let argv = replica.launch(index).argv.clone();
    let forward = argv[argv.iter().position(|word| word == "-L").unwrap() + 1].clone();
    let (local, target) = forward.split_once(':').unwrap();
    assert_eq!(local, "127.0.0.1");
    let (port, target) = target.split_once(':').unwrap();
    assert_eq!(target, "127.0.0.1:47231");
    assert!(argv.iter().any(|word| word == "origin"), "{argv:?}");

    replica.tunnel_line(index, "Welcome to the server\n");
    assert!(
        replica.session().overlay.page().is_none(),
        "only the ready line places the page"
    );
    replica.tunnel_line(index, "ngterm-tunnel-ready");

    let shown = replica.session().overlay.page().unwrap();
    assert_eq!(
        shown.source,
        HtmlSource::Url(format!("http://127.0.0.1:{port}/board?run=3").parse().unwrap())
    );
    assert_eq!(shown.replica_of.map(wire).as_deref(), Some(ORIGIN));
    assert!(replica.rejections().is_empty());
}

/// The local port of forward `index`'s `-L 127.0.0.1:PORT:HOST:PORT`.
fn forwarded_port(replica: &Replica, index: usize) -> u16 {
    let argv = replica.launch(index).argv.clone();
    let forward = &argv[argv.iter().position(|word| word == "-L").unwrap() + 1];
    forward.split(':').nth(1).unwrap().parse().unwrap()
}

#[test]
fn a_forward_listens_on_the_origins_port_unless_another_forward_holds_it() {
    let port = crate::remote::page_tunnel::reserved_test_port();
    let mut replica = Replica::new();
    let first = replica.request_loopback(&format!("http://127.0.0.1:{port}/"));
    assert_eq!(
        forwarded_port(&replica, first),
        port,
        "the page sends the Host its origin serves"
    );

    let mut other = Replica::page(Replica::url(&format!("http://localhost:{port}/x")));
    other["page"] = json!(wire(Uuid::new_v4()));
    replica.request(other);
    assert!(replica.rejections().is_empty(), "{:?}", replica.rejections());
    let second = replica.launcher.launches.borrow().len() - 1;
    assert_ne!(second, first);
    assert_ne!(
        forwarded_port(&replica, second),
        port,
        "the first forward's ssh has not bound yet, yet its port is held"
    );
}

#[test]
fn the_origin_reads_its_own_address_back_from_a_forwarded_page() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://localhost:5173/");
    replica.tunnel_line(index, "ngterm-tunnel-ready");
    let local = replica.local();
    let HtmlSource::Url(url) = replica.session().overlay.page().unwrap().source.clone() else {
        panic!("a url page");
    };
    let core = &mut replica.viewer.fixture.core;
    core.set_html_load_state(local, HtmlLoadState::Loaded, None);
    core.set_html_page(
        local,
        Some(HtmlPageInfo {
            page: url.join("/runs/2").unwrap().to_string(),
            title: Some("Runs".to_owned()),
            can_go_back: true,
            can_go_forward: false,
        }),
    );
    replica.commit();

    let states = replica.states();
    assert_eq!(states.last().unwrap()["address"], json!("http://localhost:5173/runs/2"));
}

#[test]
fn a_forward_that_ends_before_it_is_up_refuses_the_page_with_why() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/");

    replica.tunnel_closed(index, "exit status 255");

    assert!(replica.session().overlay.page().is_none());
    assert_eq!(
        replica.rejections(),
        [json!({"page": ORIGIN, "reason": "the forward to origin ended: exit status 255"})]
    );
    assert!(replica.launch(index).helper.0.borrow().stopped);
}

#[test]
fn a_forward_that_ends_later_closes_its_page_and_tells_the_origin() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/");
    replica.tunnel_line(index, "ngterm-tunnel-ready");

    replica.tunnel_closed(index, "connection lost");

    assert!(replica.session().overlay.page().is_none());
    assert_eq!(replica.payloads("page.closed"), [json!({"page": ORIGIN})]);
    assert!(replica.rejections().is_empty());
}

#[test]
fn closing_a_forwarded_page_here_ends_its_forward() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/");
    replica.tunnel_line(index, "ngterm-tunnel-ready");
    let id = replica.id();

    replica.viewer.fixture.store_mut().unwrap().close_overlay(id);
    replica.commit();

    assert!(replica.launch(index).helper.0.borrow().stopped);
    assert_eq!(replica.payloads("page.closed"), [json!({"page": ORIGIN})]);
    replica.tunnel_closed(index, "stopped");
    assert_eq!(
        replica.closed_notices(),
        1,
        "the forward's end after the close says nothing more"
    );
}

#[test]
fn the_origin_closing_a_page_still_waiting_for_its_forward_drops_the_forward() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/");

    replica.origin_frame("page.close", json!({"page": ORIGIN}));
    assert!(replica.launch(index).helper.0.borrow().stopped);
    let (key, permits) = {
        let launch = replica.launch(index);
        (launch.key, Arc::clone(&launch.permits))
    };
    assert!(
        !permits.try_take(),
        "the reader gets no permit for a line after the stop"
    );
    // a ready line already on its way still lands
    replica
        .viewer
        .fixture
        .core
        .page_helper_line(key, b"ngterm-tunnel-ready");
    replica.commit();

    assert!(
        replica.session().overlay.page().is_none(),
        "a page its origin closed is never placed"
    );
}

#[test]
fn the_row_leaving_before_the_forward_is_up_ends_it_and_refuses_the_page() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/");
    let id = replica.id();

    replica.viewer.fixture.store_mut().unwrap().close_session(id);
    replica.commit();

    assert!(replica.launch(index).helper.0.borrow().stopped);
    assert_eq!(
        replica.rejections(),
        [json!({"page": ORIGIN, "reason": "the session's row on the presenting machine closed"})]
    );
}

#[test]
fn origin_commands_sent_before_the_forward_is_up_apply_once_the_page_is_placed() {
    let mut replica = Replica::new();
    let index = replica.request_loopback("http://127.0.0.1:47231/");

    replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 1, "action": "hide"}));
    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 2, "action": "resize", "sizePercent": 60}),
    );
    replica.tunnel_line(index, "ngterm-tunnel-ready");

    assert!(replica.session().overlay_hidden(None));
    assert_eq!(replica.session().overlay.size_percent, Some(60));
    let states = replica.states();
    assert_eq!(states.last().unwrap()["appliedSeq"], json!(2), "{states:?}");
    assert_eq!(states.last().unwrap()["hidden"], json!(true));
}

#[test]
fn a_forwarded_page_loads_in_a_store_of_its_own_whatever_the_origin_asked() {
    let mut replica = Replica::new();
    replica.host.0.borrow_mut().persistent_failure = Some("no saved store".to_owned());
    let mut page = Replica::page(Replica::url("http://127.0.0.1:47231/"));
    page["persistent"] = json!(true);
    replica.request(page);
    assert!(replica.rejections().is_empty(), "{:?}", replica.rejections());
    let index = replica.launcher.launches.borrow().len() - 1;

    replica.tunnel_line(index, "ngterm-tunnel-ready");

    assert!(!replica.session().overlay.page().unwrap().flags.persistent);
}

#[test]
fn a_forward_that_cannot_start_refuses_the_page() {
    let mut replica = Replica::new();
    *replica.launcher.refuse.borrow_mut() = true;

    replica.request(Replica::page(Replica::url("http://127.0.0.1:47231/")));

    assert_eq!(
        replica.rejections(),
        [json!({"page": ORIGIN, "reason": "ssh: not found"})]
    );
    assert!(replica.session().overlay.page().is_none());
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

fn announce_split(replica: &mut Replica, shown: bool) {
    let (left, right) = replica.viewer.origin;
    replica.viewer.frame(json!({"kind": "layout", "layout": {
        "panes": [wire(left), wire(right)], "primary": wire(left), "axis": "vertical", "shown": shown
    }}));
}

fn assert_remote_split_spawn(replica: &mut Replica, pane: Uuid) {
    let id = replica.id();
    replica
        .viewer
        .fixture
        .store_mut()
        .unwrap()
        .session_mut(id)
        .unwrap()
        .split_surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(wire(pane))));
    let seed = replica.viewer.fixture.core.request_pane_spawn(id, &wire(pane)).unwrap();
    let command = crate::remote::viewer_run::readable(&seed.command.unwrap());
    assert!(command.contains("ssh") && command.contains(&crate::live::daemon_name(replica.viewer.origin.1)));
    assert!(command.contains("NGTERM_LIVE_MANAGED_CLAIM=1") && command.contains("reconnecting"));
    assert!(seed.wait_after_command && !seed.live);
}

#[test]
fn a_page_request_attaches_the_origins_new_split_and_forwards_its_loopback() {
    let mut replica = Replica::built(false);
    let primary = replica.session().pane_identity;
    announce_split(&mut replica, true);
    assert!(!replica.session().split.exists, "a layout alone never creates a pane");
    let mut request = Replica::page(Replica::url("http://127.0.0.1:41163/"));
    request["pane"] = json!(wire(replica.viewer.origin.1));
    request["chromeless"] = json!(true);
    replica.request(request);
    replica.tunnel_line(0, "ngterm-tunnel-ready");

    assert!(replica.rejections().is_empty(), "{:?}", replica.rejections());
    let session = replica.session();
    assert_eq!(session.pane_identity, primary);
    assert!(session.split.shown);
    assert!(
        !session.split.focused,
        "the page must leave focus in the existing terminal"
    );
    assert_eq!(session.split.axis, crate::model::SplitAxis::LeftRight);
    let pane = session.split_pane_identity.unwrap();
    assert_eq!(
        session.remote_presentation().unwrap().binding.remote_pane(pane),
        Some(replica.viewer.origin.1)
    );
    let shown = replica.pane_page(OverlayPane::Right).unwrap();
    assert_eq!(shown.replica_of.map(wire).as_deref(), Some(ORIGIN));
    assert!(shown.flags.chromeless && shown.flags.javascript);
    let HtmlSource::Url(url) = &shown.source else {
        panic!("a forwarded URL")
    };
    let port = url.port().unwrap();
    assert!(
        replica
            .launch(0)
            .argv
            .iter()
            .any(|arg| arg == &format!("127.0.0.1:{port}:127.0.0.1:41163"))
    );
    assert_remote_split_spawn(&mut replica, pane);

    replica.origin_frame("page.close", json!({"page": ORIGIN}));
    assert!(replica.pane_page(OverlayPane::Right).is_none());
    assert!(replica.launch(0).helper.0.borrow().stopped);
    assert_eq!(replica.session().split_pane_identity, Some(pane));
}

#[test]
fn a_page_request_reveals_its_existing_hidden_split() {
    let mut replica = Replica::new();
    announce_split(&mut replica, true);
    let id = replica.id();
    let pane = replica.session().split_pane_identity;
    replica
        .viewer
        .fixture
        .store_mut()
        .unwrap()
        .set_split_visibility(id, false, None);
    let mut request = Replica::page(Replica::url("https://example.com/"));
    request["pane"] = json!(wire(replica.viewer.origin.1));
    replica.request(request);
    assert!(replica.rejections().is_empty());
    assert!(replica.pane_page(OverlayPane::Right).is_some());
    assert_eq!(replica.session().split_pane_identity, pane);
    assert!(replica.session().split.shown);
}

#[test]
fn a_page_request_never_replaces_a_viewer_split_or_attaches_an_unknown_pane() {
    for own_split in [false, true] {
        let mut replica = Replica::built(false);
        announce_split(&mut replica, true);
        let id = replica.id();
        if own_split {
            replica
                .viewer
                .fixture
                .store_mut()
                .unwrap()
                .set_split_visibility(id, true, None);
        }
        let pane = replica.session().split_pane_identity;
        let mut request = Replica::page(Replica::url("https://example.com/"));
        request["pane"] = json!(wire(if own_split {
            replica.viewer.origin.1
        } else {
            Uuid::new_v4()
        }));
        replica.request(request);
        assert_eq!(replica.rejections()[0]["reason"], "pane not visible");
        assert_eq!(replica.session().split_pane_identity, pane);
        assert!(replica.pane_page(OverlayPane::Right).is_none());
    }
}

#[test]
fn a_page_on_the_origins_maximized_pane_keeps_its_hidden_split() {
    for focused_right in [false, true] {
        let mut replica = Replica::new();
        let id = replica.id();
        replica
            .viewer
            .fixture
            .store_mut()
            .unwrap()
            .session_mut(id)
            .unwrap()
            .split
            .focused = focused_right;
        announce_split(&mut replica, false);
        assert!(!replica.session().split.shown);
        let (remote, role) = if focused_right {
            (replica.viewer.origin.1, OverlayPane::Right)
        } else {
            (replica.viewer.origin.0, OverlayPane::Left)
        };
        let mut request = Replica::page(Replica::url("https://example.com/"));
        request["pane"] = json!(wire(remote));
        replica.request(request);
        assert!(replica.rejections().is_empty(), "{:?}", replica.rejections());
        assert!(replica.pane_page(role).is_some());
        assert!(
            !replica.session().split.shown,
            "the origin's maximized layout stays intact"
        );
        assert_eq!(replica.session().split.focused, focused_right);
    }
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
/// one that can show them lists them all but a Markdown page's links.
#[test]
fn a_viewer_without_web_views_refuses_pages_and_lists_no_page_kind() {
    let mut replica = Replica::new();
    let hello = replica.viewer.sent("hello");
    let mut kinds = crate::remote::client::viewer_kinds(true);
    kinds.extend(crate::remote::hub::PICK_KINDS.map(str::to_owned));
    kinds.extend(crate::remote::hub::HELD_KINDS.map(str::to_owned));
    assert_eq!(hello[0]["hello"]["kinds"], json!(kinds));

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
    // upstream's address is a loopback one, which this build forwards before placing the page: any URL page will do
    replica.request(Replica::page(Replica::url("https://example.com/a")));
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

/// Opens `top` over the right pane of the replica's session, here.
fn open_local_pane_overlay(replica: &mut Replica) {
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
}

/// Upstream's `hidingTheReplicaLeavesItsNeighbourAlone` and `testAnOriginCommandIsAppliedAndReportedWithItsSequence`:
/// the pane overlay opens first, so it does not hold the page.
#[test]
fn hiding_the_replica_leaves_its_neighbour_alone_and_reports_the_sequence() {
    let mut replica = Replica::new();
    open_local_pane_overlay(&mut replica);
    replica.request(Replica::page(Replica::url("https://example.com/")));
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

/// Upstream's `aLocalPaneOpenerOverTheReplicaLeavesItsReportedStateAlone`: a pane overlay opened here over the replica
/// draws above it, while the origin keeps reading the page's own bit, so nothing is reported.
#[test]
fn a_local_pane_opener_over_the_replica_leaves_its_reported_state_alone() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    replica.commit();
    let local = replica.local();

    open_local_pane_overlay(&mut replica);
    replica.commit();

    assert!(!replica.session().html_overlay_visible());
    assert!(replica.states().is_empty(), "no visibility change reaches the origin");
    // the next report the page makes still carries its own bit
    let core = &mut replica.viewer.fixture.core;
    core.set_html_load_state(local, HtmlLoadState::Loaded, None);
    replica.commit();
    let states = replica.states();
    assert_eq!(states.len(), 1, "{states:?}");
    assert_eq!(states[0]["hidden"], json!(false));
}

/// Upstream's `anOriginHideOutlivesALocalPaneOpener`.
#[test]
fn an_origin_hide_outlives_a_local_pane_opener() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    open_local_pane_overlay(&mut replica);
    replica.commit();

    replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 1, "action": "hide"}));
    let id = replica.id();
    assert!(
        replica
            .viewer
            .fixture
            .store_mut()
            .unwrap()
            .close_pane_overlay(id, OverlayPane::Right)
    );
    replica.commit();

    assert!(!replica.session().html_overlay_visible());
    let states = replica.states();
    assert_eq!(
        states.last().map(|state| state["hidden"].clone()),
        Some(json!(true)),
        "{states:?}"
    );
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

/// Upstream's `aHandedOverPageTakesTheOriginsPointSizesAndItsResizes`.
#[test]
fn a_handed_over_page_installs_and_replaces_the_origins_point_axes() {
    use crate::model::OverlayPointSize;

    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["sizePercent"] = json!(70);
    page["width"] = json!(320);
    page["height"] = json!(240);
    replica.request(page);
    assert!(replica.rejections().is_empty());
    assert_eq!(replica.session().overlay.size_percent, Some(70));
    assert_eq!(
        replica.session().overlay.points,
        OverlayPointSize {
            width: Some(320),
            height: Some(240)
        }
    );

    // The origin already resolved merges: each command replaces the presenter's complete point state.
    let resizes = [
        (json!({"sizePercent": 60, "height": 120}), Some(60), None, Some(120)),
        (json!({"sizePercent": 50, "width": 200}), Some(50), Some(200), None),
        (json!({"sizePercent": 80}), Some(80), None, None),
        (json!({}), None, None, None),
    ];
    for (seq, (mut payload, percent, width, height)) in (1..).zip(resizes) {
        payload["page"] = json!(ORIGIN);
        payload["seq"] = json!(seq);
        payload["action"] = json!("resize");
        replica.origin_frame("page.command", payload);
        assert_eq!(replica.session().overlay.size_percent, percent, "seq {seq}");
        assert_eq!(
            replica.session().overlay.points,
            OverlayPointSize { width, height },
            "seq {seq}"
        );
        assert_eq!(replica.states().last().unwrap()["appliedSeq"], seq);
    }
}

#[test]
fn replica_page_points_use_local_detail_bounds_and_local_point_resize_is_refused() {
    use crate::hud::{HudPaneFrame, HudPaneFrames};
    use crate::model::{OverlayPointSize, POINTS_REMOTE};

    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["sizePercent"] = json!(70);
    page["width"] = json!(5_000);
    page["height"] = json!(5_000);
    replica.request(page);
    let id = replica.id();
    replica.viewer.fixture.core.report_hud_frames(
        id,
        &HudPaneFrames {
            detail: Some(HudPaneFrame {
                x: 0.0,
                y: 0.0,
                width: 500.0,
                height: 300.0,
            }),
            ..HudPaneFrames::default()
        },
    );
    let node = crate::projection::session_node(replica.session(), true);
    assert_eq!(node.overlay_width, Some(400.0));
    let chrome = replica.session().overlay.page().unwrap().chrome();
    assert_eq!(node.overlay_height, Some(240.0 - chrome));
    assert_eq!(
        replica.session().overlay.points,
        OverlayPointSize {
            width: Some(5_000),
            height: Some(5_000)
        }
    );
    let before = replica.session().overlay.points;
    let refused = replica.viewer.fixture.session(
        SessionCmd::OverlayResize,
        Some(&wire(id)),
        args(|a| a.height = Some(120)),
    );
    assert_eq!(refused, failure(POINTS_REMOTE));
    assert_eq!(replica.session().overlay.points, before);
}

#[test]
fn loopback_page_forwarding_preserves_origin_points_until_the_page_is_installed() {
    use crate::model::OverlayPointSize;

    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("http://127.0.0.1:47231/board"));
    page["sizePercent"] = json!(65);
    page["width"] = json!(360);
    page["height"] = json!(240);
    replica.request(page);
    assert!(replica.session().overlay.page().is_none());
    assert!(replica.rejections().is_empty());
    replica.tunnel_line(0, "ngterm-tunnel-ready");
    assert!(replica.session().overlay.page().is_some());
    assert!(replica.rejections().is_empty());
    assert_eq!(replica.session().overlay.size_percent, Some(65));
    assert_eq!(
        replica.session().overlay.points,
        OverlayPointSize {
            width: Some(360),
            height: Some(240)
        }
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

// MARK: - a close of the page and a departure (an extension beyond upstream)

/// The payloads of the `page.closed` and the `page.detached` frames the origin heard after `show` put a page here and
/// `leave` took it away, on an origin that speaks `page.detached` when `detaching`; on the link that was up then.
fn notices(
    detaching: bool,
    show: impl FnOnce(&mut Replica),
    leave: impl FnOnce(&mut Replica),
) -> (Vec<Value>, Vec<Value>) {
    let mut replica = Replica::origin(detaching);
    show(&mut replica);
    let link = replica.viewer.link();
    leave(&mut replica);
    replica.commit();
    let id = replica.id();
    assert!(
        replica
            .viewer
            .fixture
            .store()
            .session(id)
            .is_none_or(|session| session.html_overlays().is_empty()),
        "the replica left its slot"
    );
    let heard = |kind: &str| -> Vec<Value> {
        link.0
            .borrow()
            .sent
            .iter()
            .filter(|frame| frame["kind"] == kind)
            .map(|frame| frame["page"].clone())
            .collect()
    };
    (heard("page.closed"), heard(PAGE_DETACHED_KIND))
}

fn show_file(replica: &mut Replica) {
    replica.request(Replica::page(Replica::file(None)));
    assert!(replica.session().html_overlay_active());
}

fn show_forwarded(replica: &mut Replica) {
    let index = replica.request_loopback("http://127.0.0.1:47231/");
    replica.tunnel_line(index, "ngterm-tunnel-ready");
    assert!(replica.session().html_overlay_active());
}

/// A close of the page itself sends `page.closed` to an origin that speaks `page.detached` too: a close here, and the
/// origin's own close echoed back.
#[test]
fn a_close_of_the_page_sends_page_closed_to_any_origin() {
    let closed = vec![json!({"page": ORIGIN})];
    for detaching in [true, false] {
        let here = notices(detaching, show_file, |replica| {
            let id = replica.id();
            replica.viewer.fixture.store_mut().unwrap().close_overlay(id);
        });
        assert_eq!(here, (closed.clone(), vec![]), "a close here, detaching: {detaching}");
        let there = notices(detaching, show_file, |replica| {
            replica.origin_frame("page.close", json!({"page": ORIGIN}));
        });
        assert_eq!(
            there,
            (closed.clone(), vec![]),
            "the origin's close, detaching: {detaching}"
        );
    }
}

/// The helper's end (the page channel's, one that could not start, or a loopback page's forward) sends `page.detached`
/// to an origin that speaks it, which parks the page while the stream lives on; an older origin gets `page.closed`.
#[test]
fn the_helpers_end_detaches_the_page_from_an_origin_that_speaks_it() {
    let page = vec![json!({"page": ORIGIN})];
    let helper_ends = |replica: &mut Replica| {
        let key = replica.launch(0).key;
        replica.viewer.fixture.core.page_helper_closed(key, "helper exited");
    };
    let refused = |replica: &mut Replica| {
        *replica.launcher.refuse.borrow_mut() = true;
        replica.request(Replica::page(Replica::file(None)));
    };
    let forward_ends = |replica: &mut Replica| replica.tunnel_closed(0, "connection lost");
    for (detaching, expected) in [(true, (vec![], page.clone())), (false, (page.clone(), vec![]))] {
        assert_eq!(
            notices(detaching, show_file, helper_ends),
            expected,
            "helper, {detaching}"
        );
        assert_eq!(notices(detaching, |_| {}, refused), expected, "no helper, {detaching}");
        assert_eq!(
            notices(detaching, show_forwarded, forward_ends),
            expected,
            "forward, {detaching}"
        );
    }
}

/// The row leaving (a close or a soft close) and the quit send nothing to an origin that speaks `page.detached`, which
/// parks the page when the stream ends; an older origin still hears `page.closed` when the row leaves, and nothing at
/// the quit, which ends the stream before the pages go.
#[test]
fn the_row_leaving_and_the_quit_tell_an_origin_that_speaks_page_detached_nothing() {
    let page = vec![json!({"page": ORIGIN})];
    let row_closes = |replica: &mut Replica| {
        let id = replica.id();
        replica.viewer.fixture.store_mut().unwrap().close_session(id);
    };
    let row_soft_closes = |replica: &mut Replica| {
        let id = replica.id();
        assert!(replica.viewer.fixture.store_mut().unwrap().soft_close_session(id));
    };
    let quits = |replica: &mut Replica| {
        replica.viewer.fixture.core.shutdown_remote_presentations();
        // the frontend tears its windows down after the quit save
        let id = replica.id();
        replica.viewer.fixture.store_mut().unwrap().close_overlay(id);
    };
    for (detaching, expected) in [(true, (vec![], vec![])), (false, (page.clone(), vec![]))] {
        assert_eq!(
            notices(detaching, show_file, row_closes),
            expected,
            "close, {detaching}"
        );
        assert_eq!(
            notices(detaching, show_file, row_soft_closes),
            expected,
            "soft close, {detaching}"
        );
        assert_eq!(
            notices(detaching, show_file, quits),
            (vec![], vec![]),
            "the quit ends the stream first, {detaching}"
        );
    }
}

/// A loopback page whose forward is not up yet leaves as a shown page does: the row leaving refuses it only to an
/// origin that predates `page.detached`, and its forward ending detaches it from one that speaks it.
#[test]
fn a_page_waiting_for_its_forward_is_let_go_as_a_shown_page_is() {
    for detaching in [true, false] {
        let mut replica = Replica::origin(detaching);
        let index = replica.request_loopback("http://127.0.0.1:47231/");
        let id = replica.id();
        replica.viewer.fixture.store_mut().unwrap().close_session(id);
        replica.commit();
        assert!(replica.launch(index).helper.0.borrow().stopped);
        let refused = usize::from(!detaching);
        assert_eq!(replica.rejections().len(), refused, "row left, detaching: {detaching}");

        let mut replica = Replica::origin(detaching);
        let index = replica.request_loopback("http://127.0.0.1:47231/");
        replica.tunnel_closed(index, "exit status 255");
        assert!(replica.session().overlay.page().is_none());
        assert_eq!(
            replica.rejections().len(),
            refused,
            "forward ended, detaching: {detaching}"
        );
        let detached = if detaching {
            vec![json!({"page": ORIGIN})]
        } else {
            vec![]
        };
        assert_eq!(replica.payloads(PAGE_DETACHED_KIND), detached, "detaching: {detaching}");
    }
}

/// The stream's loss takes the replica down and sends nothing: the link is gone, and the origin parks the page (an
/// older one ends it) as the stream ends there.
#[test]
fn losing_the_stream_sends_nothing_to_any_origin() {
    for detaching in [true, false] {
        let lost = notices(detaching, show_file, |replica| {
            let key = replica.viewer.link().0.borrow().key.unwrap();
            replica.viewer.fixture.core.viewer_closed(key, "exit 255");
        });
        assert_eq!(lost, (vec![], vec![]), "detaching: {detaching}");
    }
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

// MARK: - content pages (#891, #917)

/// Upstream's `aContentPageIsShownAsAReplicaOfItsKind`, a clicked document's too: each fetches over a channel of its
/// own, and none is linked to a bridge.
#[test]
fn a_content_page_is_shown_as_a_replica_of_its_kind() {
    use crate::html::ReplicaContent;
    for (source, content) in [
        (
            json!({"image": {"path": "/shots/a.png", "width": 800, "height": 600}}),
            ReplicaContent::Image {
                width: 800,
                height: 600,
            },
        ),
        (
            json!({"markdown": {"path": "/docs/README.md"}}),
            ReplicaContent::Markdown,
        ),
        (
            json!({"text": {"path": "/src/main.go", "line": 12}}),
            ReplicaContent::Text { line: Some(12) },
        ),
        (json!({"document": {"path": "/docs/pr.html"}}), ReplicaContent::Document),
    ] {
        let mut replica = Replica::new();
        replica.request(Replica::page(source.clone()));
        assert!(replica.rejections().is_empty(), "{source}: {:?}", replica.rejections());
        let shown = replica.session().overlay.page().unwrap();
        assert!(
            matches!(&shown.source, HtmlSource::Replica { content: kind, .. } if *kind == content),
            "{source}: {:?}",
            shown.source
        );
        assert_eq!(shown.replica_of.map(wire).as_deref(), Some(ORIGIN));
        assert!(!shown.bridged(), "{source}");
        let remote = replica.launch(0).argv.last().unwrap().clone();
        assert!(remote.contains("page-serve"), "{source}: {remote}");
    }
}

/// Upstream's `theTreeReportsAContentReplicaByItsKind`.
#[test]
fn the_tree_reports_a_content_replica_by_its_kind() {
    let mut replica = Replica::new();
    replica.request(Replica::page(json!({"text": {"path": "/src/main.go", "line": 12}})));
    let id = replica.id();
    let node = super::remote::node(&mut replica.viewer.fixture, id);
    let page = &node["htmlOverlays"][0];
    assert_eq!(page["text"]["file"], "/src/main.go");
    assert_eq!(page["text"]["language"], "go");
    assert_eq!(page["text"]["line"], 12);
    assert_eq!(page["replica"], true);
    assert!(page.get("file").is_none(), "{page}");
    for (source, key) in [
        (json!({"image": {"path": "/a.png", "width": 2, "height": 2}}), "image"),
        (json!({"markdown": {"path": "/a.md"}}), "markdown"),
        (json!({"document": {"path": "/a.html"}}), "document"),
    ] {
        let mut replica = Replica::new();
        replica.request(Replica::page(source));
        let id = replica.id();
        let node = super::remote::node(&mut replica.viewer.fixture, id);
        assert!(node["htmlOverlays"][0][key].is_string(), "{key}: {node}");
        assert!(node["htmlOverlays"][0].get("file").is_none());
    }
}

/// Upstream's `testAnUnsizedImageHandedToTheViewerIsSizedThere…` at the core: a session-wide picture the origin gave
/// no size is fitted here, a floating panel at the base percent sized from its pixels for this screen; a sized one, or
/// one on a pane, is placed as asked.
#[test]
fn an_unsized_picture_is_sized_for_this_screen() {
    use crate::html::image_page::BASE_PERCENT;
    use crate::model::OverlayPointSize;
    let mut replica = Replica::new();
    replica.request(Replica::page(
        json!({"image": {"path": "/shots/a.png", "width": 800, "height": 600}}),
    ));
    assert_eq!(replica.session().overlay.size_percent, Some(BASE_PERCENT));
    assert_eq!(
        replica.session().overlay.points,
        OverlayPointSize {
            width: Some(800),
            height: Some(600)
        }
    );
    let mut sized = Replica::new();
    let mut page = Replica::page(json!({"image": {"path": "/shots/a.png", "width": 800, "height": 600}}));
    page["sizePercent"] = json!(40);
    sized.request(page);
    assert_eq!(sized.session().overlay.size_percent, Some(40));
    assert!(sized.session().overlay.points.is_empty());
}

/// Upstream's `aMergingResizeKeepsTheAxisItDoesNotNameAndAPlainOneReplacesIt`.
#[test]
fn a_merging_resize_keeps_the_axis_it_does_not_name_and_a_plain_one_replaces_it() {
    use crate::model::OverlayPointSize;
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["sizePercent"] = json!(95);
    page["width"] = json!(400);
    page["height"] = json!(300);
    replica.request(page);
    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 1, "action": "resize", "sizePercent": 95, "width": 600, "merge": true}),
    );
    assert_eq!(
        replica.session().overlay.points,
        OverlayPointSize {
            width: Some(600),
            height: Some(300)
        }
    );
    replica.origin_frame(
        "page.command",
        json!({"page": ORIGIN, "seq": 2, "action": "resize", "sizePercent": 95, "width": 500}),
    );
    assert_eq!(
        replica.session().overlay.points,
        OverlayPointSize {
            width: Some(500),
            height: None
        }
    );
}

type Read = Rc<RefCell<Option<Result<Vec<u8>, String>>>>;

/// Reads `path` of content replica `page` under `limit` bytes; the answer lands in the cell.
fn read(replica: &mut Replica, page: Uuid, path: &str, limit: usize) -> Read {
    let cell: Read = Rc::default();
    let into = Rc::clone(&cell);
    let failures = crate::remote::replica_page::ContentFailures {
        too_large: format!("text file too large (over 4 MB): {path}"),
        unreadable: format!("unreadable text file: {path}"),
    };
    replica.viewer.fixture.core.replica_read(
        page,
        path,
        limit,
        failures,
        Box::new(move |result| *into.borrow_mut() = Some(result)),
    );
    cell
}

/// Upstream's `fetchReplica`: a read stops once its bytes pass its cap, failing as too large and withdrawn on the origin;
/// the origin's refusal keeps its reason, with the path; a page with no channel reads nothing.
#[test]
fn a_content_replicas_read_stops_past_its_cap_and_keeps_the_origins_refusal() {
    let mut replica = Replica::new();
    replica.request(Replica::page(json!({"text": {"path": "/src/main.go"}})));
    let local = replica.local();
    let whole = read(&mut replica, local, "/src/main.go", 8);
    replica.helper_line(
        0,
        &PageServeFrame::Head {
            id: 1,
            mime_type: "text/plain; charset=utf-8".to_owned(),
        },
    );
    replica.helper_line(
        0,
        &PageServeFrame::Chunk {
            id: 1,
            data: b"package".to_vec(),
        },
    );
    replica.helper_line(0, &PageServeFrame::End { id: 1 });
    assert_eq!(whole.borrow_mut().take(), Some(Ok(b"package".to_vec())));

    let grown = read(&mut replica, local, "/src/main.go", 8);
    replica.helper_line(
        0,
        &PageServeFrame::Chunk {
            id: 2,
            data: b"package main".to_vec(),
        },
    );
    assert_eq!(
        grown.borrow_mut().take(),
        Some(Err("text file too large (over 4 MB): /src/main.go".to_owned()))
    );
    assert_eq!(
        replica.helper_sent(0).last(),
        Some(&PageServeFrame::Cancel { id: 2 }),
        "the origin stops sending"
    );

    let refused = read(&mut replica, local, "/src/other.go", 8);
    replica.helper_line(
        0,
        &PageServeFrame::Failure {
            id: 3,
            error: "file is outside the page's folder".to_owned(),
        },
    );
    assert_eq!(
        refused.borrow_mut().take(),
        Some(Err("file is outside the page's folder: /src/other.go".to_owned()))
    );

    let unlinked = read(&mut replica, Uuid::new_v4(), "/src/main.go", 8);
    assert_eq!(
        unlinked.borrow_mut().take(),
        Some(Err("unreadable text file: /src/main.go".to_owned()))
    );
}

// MARK: - text pages

/// A text page request of `path` from the origin, a log page's when `log` names its tail.
fn text_page(path: &str, log: Option<usize>) -> Value {
    let mut page = Replica::page(json!({"text": {"path": path}}));
    if let Some(tail) = log {
        page["textLog"] = json!({"tail": tail, "follow": true});
    }
    page
}

/// An origin text action on the page it handed over.
fn text_command(replica: &mut Replica, seq: i64, fields: &Value) {
    let mut command = json!({"page": ORIGIN, "seq": seq});
    command
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    replica.origin_frame("page.command", command);
}

fn results(replica: &Replica) -> Vec<Value> {
    replica.payloads("page.result")
}

/// Runs the steps the origin's text actions left for the main loop until none is left, as the frontend's loop does.
fn run_text_steps(replica: &mut Replica) {
    loop {
        let steps = replica.viewer.fixture.core.take_replica_text_steps();
        if steps.is_empty() {
            return;
        }
        for (_, resume) in steps {
            let step = Dispatched::Pending {
                after: Duration::ZERO,
                resume,
            };
            drop(step.finish_now(&mut replica.viewer.fixture.core));
        }
        replica.commit();
    }
}

/// Answers the renderer's change steps so far with `changes`.
fn answer_change_steps(replica: &Replica, changes: TextChanges) {
    for (_, _, sender) in std::mem::take(&mut replica.host.0.borrow_mut().change_steps) {
        sender.send(changes).unwrap();
    }
}

/// The state a text page reports, as the wire carries it.
fn text_state(state: crate::remote::frames::PresentationTextState) -> Value {
    serde_json::to_value(state).unwrap()
}

/// Upstream's `aLogReplicaOpensWithItsRequestAndTheSeedBeforeItsFirstReport`: a log page handed on again opens with its
/// log request and the following, query and wrap it had, and its first report carries them.
#[test]
fn a_log_replica_opens_with_its_request_and_the_seed_before_its_first_report() {
    use crate::html::text_page::{TextLog, TextLogReport};
    let mut replica = Replica::new();
    let mut page = text_page("/var/log/app.log", Some(200));
    page["textSeed"] = json!({"following": false, "query": "err", "revision": 4, "wrap": true});
    replica.request(page);

    let shown = replica.session().overlay.page().unwrap();
    assert_eq!(
        shown.text_log,
        Some(TextLog {
            tail: 200,
            follow: true
        })
    );
    assert_eq!(shown.log_state.map(|state| state.following), Some(false));
    assert_eq!(
        shown.text_search.as_ref().map(|search| search.query.as_str()),
        Some("err")
    );
    assert_eq!(shown.search_revision, 4);
    assert!(shown.text_wrap);
    replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 1, "action": "hide"}));
    let expected = crate::remote::frames::PresentationTextState {
        following: Some(false),
        log: Some(TextLogReport::default()),
        query: Some("err".to_owned()),
        revision: 4,
        matches: Some(0),
        current: Some(0),
        wrap: true,
        changes: None,
    };
    assert_eq!(replica.states().last().unwrap()["text"], text_state(expected));
}

/// A text page's seed reaches no other kind of page, and a page with no log request takes none.
#[test]
fn a_seed_or_a_log_request_reaches_only_the_page_it_is_for() {
    let mut replica = Replica::new();
    let mut page = Replica::page(Replica::url("https://example.com/"));
    page["textLog"] = json!({"tail": 5, "follow": true});
    page["textSeed"] = json!({"query": "err", "revision": 4, "wrap": true});
    replica.request(page);
    let shown = replica.session().overlay.page().unwrap();
    assert!(shown.text_log.is_none() && shown.log_state.is_none());
    assert!(shown.text_search.is_none() && !shown.text_wrap);
    assert_eq!(shown.search_revision, 0);
}

/// Upstream's `aTextReplicaReportsItsChangesAndOtherReplicasCarryNoTextState`: a text replica's wrap, query and changes
/// reach its origin in the state report, it is a diff page for `changes`, and a replica of another kind reports no text
/// state.
#[test]
fn a_text_replica_reports_its_changes_and_other_replicas_carry_no_text_state() {
    let mut replica = Replica::new();
    replica.request(text_page("/src/a.diff", None));
    let (session, local) = (replica.id(), replica.local());
    let response = replica.viewer.fixture.session(
        SessionCmd::OverlayWrap,
        Some(&wire(session)),
        args(|a| a.mode = Some("on".to_owned())),
    );
    assert_eq!(response.result.unwrap()["text"], "wrapped");
    let core = &mut replica.viewer.fixture.core;
    let revision = core.set_text_search_from_page(local, "x").unwrap();
    core.set_text_changes_report(local, Some(TextChanges { count: 3, current: 1 }));
    replica.commit();
    let text = replica.states().last().unwrap()["text"].clone();
    assert_eq!((text["wrap"].clone(), text["query"].clone()), (json!(true), json!("x")));
    assert_eq!(text["revision"], json!(revision));
    assert_eq!(text["changes"], json!({"count": 3, "current": 1}));
    assert!(text.get("following").is_none(), "{text}");

    let mut other = Replica::new();
    other.request(Replica::page(Replica::url("https://example.com/")));
    other.origin_frame("page.command", json!({"page": ORIGIN, "seq": 1, "action": "hide"}));
    assert!(other.states().last().unwrap().get("text").is_none());
}

/// Upstream's `followOnAReplicaAnswersAsOnALocalPageOfItsKind`: a log replica follows as a local log page does, a
/// static text replica is no log page, and any other replica keeps its refusal.
#[test]
fn follow_on_a_replica_answers_as_on_a_local_page_of_its_kind() {
    let follow = |replica: &mut Replica, mode: &str| {
        let target = wire(replica.id());
        replica.viewer.fixture.session(
            SessionCmd::OverlayFollow,
            Some(&target),
            args(|a| a.mode = Some(mode.to_owned())),
        )
    };
    let mut log = Replica::new();
    log.request(text_page("/var/log/app.log", Some(50)));
    assert_eq!(follow(&mut log, "toggle").result.unwrap()["text"], "paused");
    assert_eq!(
        log.session()
            .overlay
            .page()
            .unwrap()
            .log_state
            .map(|state| state.following),
        Some(false)
    );

    let mut text = Replica::new();
    text.request(text_page("/src/main.go", None));
    assert_eq!(follow(&mut text, "on"), failure("no log page"));

    let mut url = Replica::new();
    url.request(Replica::page(Replica::url("https://example.com/")));
    assert_eq!(
        follow(&mut url, "on"),
        failure("submit on the Mac that opened the page")
    );
}

/// Upstream's `testTextCommandsOnAReplicaAnswerAsLocallyAndReachTheOrigin`: search, wrap, follow and a change step
/// on a text replica answer as on a local page, and the origin hears the state they left.
#[test]
fn text_commands_on_a_replica_answer_as_locally_and_reach_the_origin() {
    let mut replica = Replica::new();
    replica.request(text_page("/src/a.diff", None));
    let target = wire(replica.id());
    let command = |replica: &mut Replica, cmd, change: fn(&mut ngterm_proto::Args)| {
        let request = ngterm_proto::Request {
            cmd: ngterm_proto::command::Command::Session(cmd),
            target: Some(target.clone()),
            args: Some(args(change)),
        };
        replica.viewer.fixture.core.begin(&request)
    };
    let search = command(&mut replica, SessionCmd::OverlaySearch, |a| {
        a.text = Some("b".to_owned());
    });
    let wrap = command(&mut replica, SessionCmd::OverlayWrap, |a| {
        a.mode = Some("on".to_owned());
    });
    let follow = command(&mut replica, SessionCmd::OverlayFollow, |a| {
        a.mode = Some("on".to_owned());
    });
    let step = command(&mut replica, SessionCmd::OverlayChanges, |a| {
        a.to = Some("next".to_owned());
    });
    answer_change_steps(&replica, TextChanges { count: 1, current: 1 });
    let core = &mut replica.viewer.fixture.core;
    let [search, wrap, follow, step] = [search, wrap, follow, step].map(|dispatched| dispatched.finish_now(core));
    assert!(search.ok, "{:?}", search.error);
    assert_eq!(wrap.result.unwrap()["text"], "wrapped");
    assert_eq!(follow.error.as_deref(), Some("no log page"));
    assert_eq!(step.result.unwrap()["text"], "1 of 1");
    replica.commit();
    let text = replica.states().last().unwrap()["text"].clone();
    assert_eq!(text["query"], "b");
    assert_eq!(text["wrap"], true);
    assert_eq!(text["changes"], json!({"count": 1, "current": 1}));
    assert!(text.get("following").is_none(), "{text}");
}

/// Upstream's `testAnOriginTextActionRunsAsTheLocalCommandAndAnswersWithThePageState`: the origin's actions run here in
/// order, one waiting on the renderer holding back those after it, and each answers with the page's text state after
/// it, a refusal included.
#[test]
fn an_origin_text_action_runs_as_the_local_command_and_answers_with_the_page_state() {
    let mut replica = Replica::new();
    replica.request(text_page("/src/a.diff", None));
    text_command(&mut replica, 1, &json!({"action": "wrap", "toggle": "on"}));
    text_command(&mut replica, 2, &json!({"action": "changes", "step": "next"}));
    text_command(&mut replica, 3, &json!({"action": "follow", "toggle": "on"}));
    assert_eq!(results(&replica).len(), 1, "the follow waits for the change step");
    assert!(!replica.states().iter().any(|state| state["appliedSeq"] != json!(0)));

    answer_change_steps(&replica, TextChanges { count: 1, current: 1 });
    run_text_steps(&mut replica);
    let answered = results(&replica);
    assert_eq!(
        answered.iter().map(|result| result["seq"].clone()).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        (answered[0]["text"].clone(), answered[0]["state"]["wrap"].clone()),
        (json!("wrapped"), json!(true))
    );
    assert_eq!(answered[1]["text"], "1 of 1");
    assert_eq!(answered[1]["state"]["changes"], json!({"count": 1, "current": 1}));
    assert_eq!(
        answered[2],
        json!({"page": ORIGIN, "seq": 3, "ok": false, "error": "no log page", "state": answered[1]["state"]})
    );
}

/// Upstream's `testATextActionOnAPageThatIsNotTextAnswersTheLocalRefusalAndAdvancesNoSequence`: the local refusal, an
/// incomplete action as invalid and one for a page not shown here as such, with no state report and `appliedSeq` left.
#[test]
fn a_text_action_on_a_page_that_is_not_text_answers_the_local_refusal_and_advances_no_sequence() {
    let mut replica = Replica::new();
    replica.request(Replica::page(Replica::url("https://example.com/")));
    replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 5, "action": "hide"}));
    let before = replica.states().len();
    text_command(&mut replica, 6, &json!({"action": "search", "query": "x"}));
    text_command(&mut replica, 7, &json!({"action": "search"}));
    replica.origin_frame(
        "page.command",
        json!({"page": "elsewhere", "seq": 2, "action": "wrap", "toggle": "on"}),
    );
    assert_eq!(
        results(&replica),
        [
            json!({"page": ORIGIN, "seq": 6, "ok": false, "error": "no text page"}),
            json!({"page": ORIGIN, "seq": 7, "ok": false, "error": "invalid text page action"}),
            json!({"page": "elsewhere", "seq": 2, "ok": false, "error": "page not shown"}),
        ]
    );
    assert_eq!(replica.states().len(), before);
    replica.origin_frame("page.command", json!({"page": ORIGIN, "seq": 9, "action": "show"}));
    assert_eq!(replica.states().last().unwrap()["appliedSeq"], json!(9));
}

/// Upstream's `testAQueuedTextActionNeverReachesAPageThatReplacedItsOwn`: an action waiting behind another when its
/// page goes answers that the page is not shown, and never acts on the page offered after it.
#[test]
fn a_queued_text_action_never_reaches_a_page_that_replaced_its_own() {
    let mut replica = Replica::new();
    replica.request(text_page("/src/a.diff", None));
    let first = replica.local();
    text_command(&mut replica, 1, &json!({"action": "changes", "step": "next"}));
    text_command(&mut replica, 2, &json!({"action": "wrap", "toggle": "on"}));
    replica.origin_frame("page.close", json!({"page": ORIGIN}));
    assert_eq!(
        results(&replica),
        [json!({"page": ORIGIN, "seq": 2, "ok": false, "error": "page not shown"})]
    );
    replica.request(text_page("/src/a.diff", None));
    answer_change_steps(&replica, TextChanges { count: 1, current: 1 });
    run_text_steps(&mut replica);
    assert_ne!(replica.local(), first);
    assert!(!replica.session().overlay.page().unwrap().text_wrap);
    assert_eq!(results(&replica).len(), 2, "the running step still answers");
}

/// A text action whose waiting step panics answers the origin `ok:false` as a panicking handler does, and the actions
/// queued behind it still run.
#[test]
fn a_panicking_text_step_answers_the_origin_and_runs_the_queue_behind_it() {
    let mut replica = Replica::new();
    replica.request(text_page("/src/a.diff", None));
    text_command(&mut replica, 1, &json!({"action": "changes", "step": "next"}));
    text_command(&mut replica, 2, &json!({"action": "wrap", "toggle": "on"}));
    let core = &mut replica.viewer.fixture.core;
    assert_eq!(core.take_replica_text_steps().len(), 1);
    let command =
        serde_json::from_value(json!({"page": ORIGIN, "seq": 1, "action": "changes", "step": "next"})).unwrap();
    let panics: crate::dispatch::Resume = Box::new(|_| std::panic::resume_unwind(Box::new(())));
    let (_, step) =
        crate::remote::replica_page::text_step(replica.local(), replica.id(), command, Duration::ZERO, panics);
    let step = Dispatched::Pending {
        after: Duration::ZERO,
        resume: step,
    };
    let response = step.finish_now(&mut replica.viewer.fixture.core);
    replica.commit();
    let error = "internal error handling session.overlay.changes";
    assert_eq!(response.error.as_deref(), Some(error));
    let answered = results(&replica);
    assert_eq!(
        answered
            .iter()
            .map(|result| (result["seq"].clone(), result["ok"].clone()))
            .collect::<Vec<_>>(),
        [(json!(1), json!(false)), (json!(2), json!(true))]
    );
    assert_eq!(answered[0]["error"], error);
    assert!(replica.session().overlay.page().unwrap().text_wrap);
}

type Tailed = Rc<RefCell<Option<Result<Option<crate::bounded_file::TailRead>, ChannelFailure>>>>;

fn log_tail(replica: &mut Replica, stamp: Option<crate::bounded_file::FileStamp>) -> Tailed {
    let cell: Tailed = Rc::default();
    let into = Rc::clone(&cell);
    let local = replica.local();
    replica.viewer.fixture.core.replica_log_tail(
        local,
        stamp,
        Box::new(move |result| *into.borrow_mut() = Some(result)),
    );
    cell
}

/// Upstream's `logTail` through a log replica's channel: the origin's end of the file with its stamp and offset, nothing
/// while the stamp holds, a cap on what an answer may carry, and a page with no channel reads nothing.
#[test]
fn a_log_replica_reads_its_files_end_over_its_channel() {
    use crate::bounded_file::{FileStamp, TailRead};
    let mut replica = Replica::new();
    replica.request(text_page("/var/log/app.log", Some(20)));
    let stamp = FileStamp {
        identity: (1, 2),
        size: 9,
        modified: (3, 4),
    };
    let fresh = log_tail(&mut replica, None);
    assert_eq!(
        replica.helper_sent(0).last(),
        Some(&PageServeFrame::LogTail { id: 1, stamp: None })
    );
    replica.helper_line(
        0,
        &PageServeFrame::LogHead {
            id: 1,
            stamp,
            offset: 2,
        },
    );
    replica.helper_line(
        0,
        &PageServeFrame::Chunk {
            id: 1,
            data: b"a\nb\n".to_vec(),
        },
    );
    replica.helper_line(0, &PageServeFrame::End { id: 1 });
    assert_eq!(
        fresh.borrow_mut().take(),
        Some(Ok(Some(TailRead {
            data: b"a\nb\n".to_vec(),
            offset: 2,
            stamp,
        })))
    );

    let unchanged = log_tail(&mut replica, Some(stamp));
    replica.helper_line(0, &PageServeFrame::LogUnchanged { id: 2 });
    assert_eq!(unchanged.borrow_mut().take(), Some(Ok(None)));

    let grown = log_tail(&mut replica, None);
    replica.helper_line(
        0,
        &PageServeFrame::LogHead {
            id: 3,
            stamp,
            offset: 0,
        },
    );
    let chunk = crate::remote::page_serve::MAX_CHUNK_BYTES;
    for _ in 0..=crate::html::text_page::MAX_TAIL_BYTES / chunk {
        assert!(grown.borrow().is_none(), "an answer within the cap still reads");
        replica.helper_line(
            0,
            &PageServeFrame::Chunk {
                id: 3,
                data: vec![b'x'; chunk],
            },
        );
    }
    assert_eq!(grown.borrow_mut().take(), Some(Err(ChannelFailure::TooLarge)));
    assert_eq!(replica.helper_sent(0).last(), Some(&PageServeFrame::Cancel { id: 3 }));

    let cell: Tailed = Rc::default();
    let into = Rc::clone(&cell);
    replica.viewer.fixture.core.replica_log_tail(
        Uuid::new_v4(),
        None,
        Box::new(move |result| *into.borrow_mut() = Some(result)),
    );
    assert_eq!(cell.borrow_mut().take(), Some(Err(ChannelFailure::Closed)));
}

/// A log tail still waiting when its page closes fails as the channel's close, once.
#[test]
fn a_log_tail_in_flight_fails_when_its_page_closes() {
    let mut replica = Replica::new();
    replica.request(text_page("/var/log/app.log", Some(20)));
    let waiting = log_tail(&mut replica, None);
    replica.origin_frame("page.close", json!({"page": ORIGIN}));
    assert_eq!(waiting.borrow_mut().take(), Some(Err(ChannelFailure::Closed)));
}

/// Upstream's `TextPageBridge.step`: the model takes a search step's count before the step answers, and a step that
/// found a match pauses a following log as the page's own move does, so the origin's result already carries both.
#[test]
fn a_search_step_records_its_count_and_pauses_a_following_log_before_it_answers() {
    let mut replica = Replica::new();
    replica.request(text_page("/var/log/app.log", Some(20)));
    text_command(&mut replica, 1, &json!({"action": "search", "query": "err"}));
    text_command(&mut replica, 2, &json!({"action": "search", "step": "next"}));
    let steps = std::mem::take(&mut replica.host.0.borrow_mut().search_steps);
    let [(_, step, sender)] = <[_; 1]>::try_from(steps).ok().unwrap();
    assert_eq!((step.query.as_str(), step.forward), ("err", true));
    sender.send(SearchCount { matches: 2, current: 1 }).unwrap();
    run_text_steps(&mut replica);
    let answered = results(&replica);
    assert_eq!(answered[1]["text"], "1 of 2");
    let state = &answered[1]["state"];
    assert_eq!(
        (state["matches"].clone(), state["current"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(state["following"], false);
}
