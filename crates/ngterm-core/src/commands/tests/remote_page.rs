//! Pages handed to the viewer presenting a session, on the origin (upstream's `AppStoreRemotePageTests` origin cases,
//! `HtmlOverlayTests.openIsRefusedWhenAFollowingPaneBelongsToAPresenterWithoutPages` and
//! `ControlServerPageServeTests`' open, reload, navigation, result and read cases): the placement rule, the
//! reservation and its `page.request`, the mirrored state, the visibility requests, the commands sent on, and every
//! way a page ends.

use std::time::Instant;

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::live::{Live, live};
use super::remote::{OLDER_KINDS, Viewer, frame, node, set_lead, subscribe_listing};
use super::{Fixture, args, failure, wire};
use crate::html::{HtmlHost, HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::hud::{HudPanelSize, HudSpec};
use crate::model::{HudOpening, OverlayPane, RemoteOverlaySlot, RemotePage};
use crate::remote::frames::MAX_FRAME_BYTES;
use crate::remote::hub::SUPPORTED_KINDS;

const PRESENTER: &str = "a viewer presents this session: an html overlay would open where nobody sees it";
const LOOPBACK: &str = "a loopback url names this Mac, so the Mac presenting the session cannot load it";
const TOO_LARGE: &str = "page too large to show on another Mac";
const REMOTE_PAGE: &str = "page is shown on another Mac";
const VIEWER_GONE: &str = "the viewer showing this overlay is gone";
const FILE: &str = "/tmp/docs/index.html";

/// Web views that can run, so a page the placement rule keeps here opens.
struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

/// A presented live session (upstream's `origin`): its panes (both, with `split`) follow a remote lead, and a
/// presenter whose hello lists `kinds` holds it.
struct Origin {
    live: Live,
    serial: u64,
    presenter: Viewer,
}

impl Origin {
    fn new(split: bool, kinds: &[&str]) -> Self {
        Self::build(split, kinds, true)
    }

    /// [`Origin::new`], with web views here only with `host`.
    fn build(split: bool, kinds: &[&str], host: bool) -> Self {
        let mut live = live();
        if host {
            live.fixture.core.set_html_host(Box::new(Host));
        }
        let left = live.left;
        set_lead(&mut live.fixture.core, left, "follower");
        if split {
            let right = live.split();
            set_lead(&mut live.fixture.core, right, "follower");
        }
        let session = live.session;
        let (serial, presenter) = subscribe_listing(&mut live.fixture.core, session, "presenter", kinds);
        let acquire = frame(&presenter, &json!({"kind": "presenter.acquire"}));
        live.fixture.core.presentation_line(serial, &acquire);
        live.fixture.core.commit();
        assert_eq!(presenter.kinds().last().map(String::as_str), Some("presenter.granted"));
        Self {
            live,
            serial,
            presenter,
        }
    }

    fn pages() -> Self {
        Self::new(false, &SUPPORTED_KINDS)
    }

    fn session(&self) -> Uuid {
        self.live.session
    }

    fn fixture(&mut self) -> &mut Fixture {
        &mut self.live.fixture
    }

    fn overlay(&mut self, cmd: SessionCmd, edit: impl FnOnce(&mut Args)) -> Response {
        let session = wire(self.session());
        self.live.fixture.session(cmd, Some(&session), args(edit))
    }

    /// Opens a page with `edit` applied to a `--follow` open of `source`.
    fn open(&mut self, source: &str, edit: impl FnOnce(&mut Args)) -> Response {
        self.overlay(SessionCmd::OverlayOpen, |a| {
            if source.starts_with('/') {
                a.html = Some(source.to_owned());
                a.cwd = Some("/tmp/docs".to_owned());
                a.javascript = Some(true);
            } else {
                a.url = Some(source.to_owned());
            }
            a.follow = Some(true);
            edit(a);
        })
    }

    /// Opens `source` and expects it handed over; its page id.
    fn opened(&mut self, source: &str, edit: impl FnOnce(&mut Args)) -> String {
        let response = self.open(source, edit);
        assert!(response.ok, "{response:?}");
        let result = response.result.unwrap();
        assert_eq!(result["id"], json!(wire(self.session())));
        let page = result["pageID"].as_str().unwrap().to_owned();
        assert_eq!(
            self.presenter.last("page.request").unwrap()["page"]["page"],
            json!(page)
        );
        page
    }

    /// Sends what the presenter would, as the presenter; the `tree.changed` the commit after it fires.
    fn report(&mut self, body: &Value) -> usize {
        let line = frame(&self.presenter, body);
        self.live.fixture.core.presentation_line(self.serial, &line);
        self.live.fixture.core.commit().tree_changed.len()
    }

    /// The presenter's `page.state` for `page`; the `tree.changed` it fires.
    fn state(&mut self, page: &str, hidden: bool, seq: i64, title: Option<&str>) -> usize {
        let mut state = json!({"page": page, "address": "/tmp/docs/b.html", "load": "loaded",
            "canGoBack": true, "canGoForward": false, "hidden": hidden, "appliedSeq": seq});
        if let Some(title) = title {
            state["title"] = json!(title);
        }
        self.report(&json!({"kind": "page.state", "page": state}))
    }

    fn node(&mut self) -> Value {
        let session = self.session();
        node(&mut self.live.fixture, session)
    }

    fn outcome(&mut self, page: &str) -> Value {
        let response = self.live.fixture.session(
            SessionCmd::OverlayResult,
            None,
            args(|a| a.page = Some(page.to_owned())),
        );
        response.result.unwrap_or(Value::Null)["pageOutcome"].clone()
    }

    /// The `page.command` payloads the presenter got, in order.
    fn commands(&self) -> Vec<Value> {
        self.presenter
            .frames("page.command")
            .into_iter()
            .map(|frame| frame["page"].clone())
            .collect()
    }

    fn actions(&self) -> Vec<String> {
        self.commands()
            .iter()
            .map(|command| command["action"].as_str().unwrap().to_owned())
            .collect()
    }

    fn visibility(&mut self, cmd: SessionCmd) -> String {
        let response = self.overlay(cmd, |_| {});
        self.live.fixture.core.commit();
        assert!(response.ok, "{response:?}");
        response.result.unwrap()["text"].as_str().unwrap().to_owned()
    }

    fn slot_jobs(&self) -> Vec<String> {
        self.live
            .fixture
            .store()
            .session(self.live.session)
            .unwrap()
            .remote_overlays()
            .slots()
            .iter()
            .map(|slot| slot.job.clone())
            .collect()
    }

    fn tree_changes(&mut self) -> usize {
        self.live.fixture.core.commit().tree_changed.len()
    }
}

// MARK: - the placement rule

#[test]
fn a_page_opens_on_a_presenter_that_speaks_pages() {
    let mut origin = Origin::pages();
    origin.tree_changes();
    let address = "https://example.com/review/42?token=abc";
    let page = origin.opened(address, |a| a.size_percent = Some(70));
    assert_eq!(origin.tree_changes(), 1);
    assert_eq!(origin.outcome(&page)["outcome"], "pending");
    let request = origin.presenter.last("page.request").unwrap()["page"].clone();
    assert_eq!(request["source"], json!({"url": {"_0": address}}));
    assert_eq!(request["sizePercent"], 70);
    assert_eq!(request["follow"], true);
    assert_eq!(request["identity"], "https://example.com");
    assert!(Uuid::parse_str(request["token"].as_str().unwrap()).is_ok());
    let read = origin.node();
    assert_eq!(read["overlay"], false, "nothing covers the session here");
    assert_eq!(read["remoteOverlays"], json!([{"sizePercent": 70, "pageID": page}]));
    assert_eq!(
        read["htmlOverlays"],
        json!([{"id": page, "url": address, "state": "loading", "javascript": false, "chromeless": false,
            "persistent": false, "browse": false, "remote": true}]),
        "no zoom of this machine's"
    );
}

/// Upstream's `aPresenterWithoutPageKindsKeepsTheRefusal` and `openIsRefusedWhenAFollowingPaneBelongsToAPresenter
/// WithoutPages`.
#[test]
fn a_presenter_without_page_kinds_keeps_the_refusal() {
    for kinds in [&OLDER_KINDS[..], &[]] {
        let mut origin = Origin::new(false, kinds);
        assert_eq!(origin.open(FILE, |_| {}), failure(PRESENTER));
        assert!(origin.slot_jobs().is_empty());
        assert!(origin.presenter.last("page.request").is_none());
        assert_eq!(origin.node()["overlay"], false);
    }
}

/// Upstream's `aPaneThatDoesNotFollowKeepsThePageHere`.
#[test]
fn a_pane_that_does_not_follow_keeps_the_page_here() {
    for role in ["leader", "unowned"] {
        let mut origin = Origin::pages();
        let left = origin.live.left;
        set_lead(&mut origin.live.fixture.core, left, role);
        let opened = origin.open(FILE, |_| {});
        assert!(opened.ok, "{role}: {opened:?}");
        assert!(origin.presenter.last("page.request").is_none(), "{role}");
        let read = origin.node();
        assert_eq!(read["overlay"], true, "{role}");
        assert!(read["htmlOverlays"][0].get("remote").is_none(), "{role}");
    }
}

#[test]
fn a_session_wide_page_with_mixed_roles_stays_here() {
    let mut origin = Origin::new(true, &SUPPORTED_KINDS);
    let right = origin
        .live
        .fixture
        .store()
        .session(origin.session())
        .unwrap()
        .split_pane_identity
        .unwrap();
    set_lead(&mut origin.live.fixture.core, right, "leader");
    assert!(origin.open(FILE, |_| {}).ok);
    assert!(origin.presenter.last("page.request").is_none(), "kept here");
    origin.opened(FILE, |a| a.pane = Some("left".to_owned()));
}

#[test]
fn a_loopback_url_is_refused_only_when_the_page_would_go_to_the_presenter() {
    let mut origin = Origin::pages();
    assert_eq!(origin.open("http://127.0.0.1:8080/", |_| {}), failure(LOOPBACK));
    assert_eq!(origin.open("http://localhost:5173/", |_| {}), failure(LOOPBACK));
    let left = origin.live.left;
    set_lead(&mut origin.live.fixture.core, left, "leader");
    assert!(origin.open("http://127.0.0.1:8080/", |_| {}).ok);
}

#[test]
fn a_request_over_one_frame_is_refused_before_anything_changes() {
    let mut origin = Origin::new(true, &SUPPORTED_KINDS);
    let shown = origin.opened(FILE, |a| a.pane = Some("left".to_owned()));
    let session = origin.session();
    let dir = tempfile::tempdir().unwrap();
    let opening = HudOpening {
        command: "hud.sh".to_owned(),
        file: dir.path().join("hud"),
        size: HudPanelSize::new(30, 8),
        pane_identity: None,
        font_size: None,
    };
    assert!(
        origin
            .fixture()
            .store_mut()
            .unwrap()
            .open_hud(session, HudSpec::new("working"), opening)
    );
    origin.tree_changes();
    let sent = origin.presenter.sent();
    let query = "q".repeat(MAX_FRAME_BYTES);
    assert_eq!(
        origin.open(&format!("https://example.com/report?data={query}"), |_| {}),
        failure(TOO_LARGE)
    );
    assert_eq!(origin.tree_changes(), 0);
    let found = origin.live.fixture.store().session(session).unwrap();
    assert!(found.overlay.hud_active(), "the HUD keeps the slot");
    assert_eq!(origin.presenter.sent(), sent);
    assert_eq!(origin.slot_jobs(), [shown]);
}

#[test]
fn a_taken_slot_and_a_missing_pane_are_refused() {
    let mut origin = Origin::pages();
    origin.opened(FILE, |_| {});
    assert_eq!(origin.open(FILE, |_| {}), failure("overlay already open"));
    assert_eq!(
        origin.open(FILE, |a| a.pane = Some("right".to_owned())),
        failure("pane not visible")
    );
}

/// Upstream's `testARemoteOpenSkipsTheLocalPageFollowAndStore`: no web view, saved store or selection is this
/// machine's concern, so a persistent page goes even where no web view could run.
#[test]
fn a_remote_open_skips_the_local_page_follow_and_store() {
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    let workspace = origin.live.fixture.workspace;
    let other = origin.live.fixture.add_session(workspace);
    origin.fixture().store_mut().unwrap().select_session(Some(other));
    let page = origin.opened("https://example.com/", |a| a.persistent = Some(true));
    assert_eq!(
        origin.live.fixture.selected(),
        Some(other),
        "--follow is the presenter's"
    );
    let request = origin.presenter.last("page.request").unwrap()["page"].clone();
    assert_eq!(
        (request["persistent"].clone(), request["follow"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(origin.outcome(&page)["outcome"], "pending");
    // kept here, the same page finds no web views
    let left = origin.live.left;
    set_lead(&mut origin.live.fixture.core, left, "leader");
    assert_eq!(
        origin.open("https://example.com/", |a| a.pane = Some("left".to_owned())),
        failure("session.overlay.open: html overlays are not available in this build")
    );
}

// MARK: - the mirrored state

#[test]
fn the_mirrored_state_is_reported_and_only_a_hidden_change_touches_the_tree() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    origin.tree_changes();
    assert_eq!(origin.state(&page, false, 0, Some("Review")), 0);
    let read = origin.node()["htmlOverlays"][0].clone();
    assert_eq!(
        (read["title"].clone(), read["page"].clone(), read["state"].clone()),
        (json!("Review"), json!("/tmp/docs/b.html"), json!("loaded"))
    );
    assert_eq!(
        (read["canGoBack"].clone(), read["canGoForward"].clone()),
        (json!(true), json!(false))
    );
    assert!(read.get("hidden").is_none());
    assert_eq!(origin.state(&page, true, 0, None), 1);
    assert_eq!(origin.state(&page, true, 0, None), 0, "the same report changes nothing");
    let read = origin.node();
    assert_eq!(read["htmlOverlays"][0]["hidden"], true);
    assert_eq!(read["hiddenOverlays"], json!(["session"]));
}

/// A page report reaches the model only from the session's presenter (upstream's `aPageReportFromAMirrorIsIgnored`).
#[test]
fn a_page_report_from_a_mirror_is_ignored() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    let session = origin.session();
    let (serial, mirror) = subscribe_listing(&mut origin.live.fixture.core, session, "mirror", &SUPPORTED_KINDS);
    let closed = frame(&mirror, &json!({"kind": "page.closed", "page": {"page": page}}));
    origin.live.fixture.core.presentation_line(serial, &closed);
    origin.live.fixture.core.commit();
    assert_eq!(origin.slot_jobs(), [page]);
}

// MARK: - how a page ends

#[test]
fn closing_sends_the_close_and_dismisses_the_page() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    origin.tree_changes();
    assert!(origin.overlay(SessionCmd::OverlayClose, |_| {}).ok);
    assert_eq!(origin.tree_changes(), 1);
    assert_eq!(
        origin.presenter.last("page.close").unwrap()["page"],
        json!({"page": page})
    );
    assert!(origin.slot_jobs().is_empty());
    assert_eq!(origin.outcome(&page)["outcome"], "dismissed");
    assert_eq!(origin.overlay(SessionCmd::OverlayClose, |_| {}), failure("no overlay"));
}

#[test]
fn a_submit_records_the_answer_and_a_later_close_keeps_it() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    let submitted = origin.overlay(SessionCmd::OverlaySubmit, |a| a.value = Some("notes".to_owned()));
    assert_eq!(submitted.result, Some(json!({"id": wire(origin.session())})));
    assert_eq!(
        origin.presenter.last("page.close").unwrap()["page"],
        json!({"page": page})
    );
    origin.report(&json!({"kind": "page.closed", "page": {"page": page}}));
    assert_eq!(
        origin.outcome(&page),
        json!({"pageID": page, "outcome": "submitted", "value": "notes"})
    );
    assert!(origin.slot_jobs().is_empty());
}

#[test]
fn a_rejection_ends_the_page_dismissed() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    origin.report(&json!({"kind": "page.rejected", "page": {"page": page, "reason": "slot taken"}}));
    assert!(origin.slot_jobs().is_empty());
    assert_eq!(origin.outcome(&page)["outcome"], "dismissed");
    assert!(origin.node().get("remoteOverlays").is_none());
}

#[test]
fn losing_the_presenter_ends_the_page_and_keeps_an_accepted_answer() {
    let mut origin = Origin::new(true, &SUPPORTED_KINDS);
    let pending = origin.opened(FILE, |_| {});
    let answered = origin.opened(FILE, |a| a.pane = Some("left".to_owned()));
    let submitted = origin.overlay(SessionCmd::OverlaySubmit, |a| {
        a.value = Some("kept".to_owned());
        a.pane = Some("left".to_owned());
    });
    assert!(submitted.ok);
    // the presenter goes before it closed the answered page
    let serial = origin.serial;
    origin.live.fixture.core.presentation_closed(serial);
    origin.live.fixture.core.commit();
    assert!(origin.slot_jobs().is_empty());
    assert_eq!(origin.outcome(&pending)["outcome"], "dismissed");
    assert_eq!(origin.outcome(&answered)["value"], "kept");
}

/// Upstream's `aPageThePresenterLossEndedLeavesNoOverlay`.
#[test]
fn a_page_the_presenter_loss_ended_leaves_no_overlay() {
    let mut origin = Origin::pages();
    origin.opened(FILE, |_| {});
    let serial = origin.serial;
    origin.live.fixture.core.presentation_closed(serial);
    origin.live.fixture.core.commit();
    assert_eq!(
        origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some("back".to_owned())),
        failure("no overlay")
    );
    assert_eq!(origin.overlay(SessionCmd::OverlayReload, |_| {}), failure("no overlay"));
    assert_eq!(origin.node()["overlay"], false);
}

#[test]
fn removing_the_session_ends_its_page() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    let session = wire(origin.session());
    assert!(
        origin
            .live
            .fixture
            .session(SessionCmd::Close, Some(&session), Args::default())
            .ok
    );
    origin.live.fixture.core.commit();
    assert_eq!(
        origin.presenter.last("page.close").unwrap()["page"],
        json!({"page": page})
    );
    assert_eq!(origin.outcome(&page)["outcome"], "dismissed");
}

// MARK: - visibility

#[test]
fn two_toggles_before_any_report_hide_then_show() {
    let mut origin = Origin::pages();
    origin.opened(FILE, |_| {});
    assert_eq!(origin.visibility(SessionCmd::OverlayToggle), "hidden");
    assert_eq!(origin.visibility(SessionCmd::OverlayToggle), "shown");
    assert_eq!(origin.actions(), ["hide", "show"]);
    let seqs: Vec<i64> = origin
        .commands()
        .iter()
        .map(|command| command["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, [1, 2]);
    assert!(origin.node().get("hiddenOverlays").is_none(), "nothing reported yet");
}

#[test]
fn a_stale_report_does_not_override_a_newer_request() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    origin.visibility(SessionCmd::OverlayHide);
    origin.visibility(SessionCmd::OverlayShow);
    origin.state(&page, true, 1, None);
    assert_eq!(origin.visibility(SessionCmd::OverlayToggle), "hidden");
    assert_eq!(origin.actions().last().map(String::as_str), Some("hide"));
}

#[test]
fn a_hide_made_on_the_presenter_turns_the_next_toggle_into_a_show() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    origin.visibility(SessionCmd::OverlayHide);
    origin.visibility(SessionCmd::OverlayShow);
    assert_eq!(origin.actions(), ["hide", "show"]);
    let show = origin.commands().last().unwrap()["seq"].as_i64().unwrap();
    origin.state(&page, false, show, None);
    origin.state(&page, true, show, None);
    assert_eq!(origin.visibility(SessionCmd::OverlayToggle), "shown");
    assert_eq!(origin.actions(), ["hide", "show", "show"]);
    // asking for what the page already shows sends nothing
    origin.visibility(SessionCmd::OverlayShow);
    assert_eq!(origin.actions().len(), 3);
}

#[test]
fn a_remote_program_still_refuses_to_hide() {
    let mut origin = Origin::pages();
    let opened = origin.overlay(SessionCmd::OverlayOpen, |a| a.command = Some("revdiff".to_owned()));
    assert!(opened.ok);
    assert_eq!(
        origin.overlay(SessionCmd::OverlayHide, |_| {}),
        failure("overlay is shown on another Mac")
    );
}

// MARK: - commands sent on

#[test]
fn resize_reaches_the_presenter_as_a_command() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    assert!(
        origin
            .overlay(SessionCmd::OverlayResize, |a| a.size_percent = Some(40))
            .ok
    );
    assert_eq!(
        origin.commands().last().unwrap(),
        &json!({"page": page, "seq": 1, "action": "resize", "sizePercent": 40})
    );
    assert_eq!(origin.node()["remoteOverlays"][0]["sizePercent"], 40);
}

#[test]
fn a_url_page_navigation_reaches_the_presenter() {
    for (to, action) in [("back", "back"), ("forward", "forward"), ("browser", "browser")] {
        let mut origin = Origin::pages();
        origin.opened("https://example.com/a", |_| {});
        let sent = origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some(to.to_owned()));
        assert_eq!(sent.result, Some(json!({"id": wire(origin.session())})), "{to}");
        assert_eq!(origin.actions().last().map(String::as_str), Some(action));
    }
}

#[test]
fn a_file_page_goes_back_and_forward_but_finder_and_browser_are_refused() {
    let mut origin = Origin::pages();
    origin.opened(FILE, |_| {});
    let navigate =
        |origin: &mut Origin, to: &str| origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some(to.to_owned()));
    assert!(navigate(&mut origin, "back").ok);
    assert!(navigate(&mut origin, "forward").ok);
    assert_eq!(navigate(&mut origin, "browser"), failure(REMOTE_PAGE));
    assert_eq!(navigate(&mut origin, "finder"), failure(REMOTE_PAGE));
    assert_eq!(origin.actions(), ["back", "forward"]);
}

#[test]
fn finder_on_a_url_page_gives_the_local_answer() {
    let mut origin = Origin::pages();
    origin.opened("https://example.com/a", |_| {});
    assert_eq!(
        origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some("finder".to_owned())),
        failure("show in Finder requires a file page")
    );
    assert!(origin.commands().is_empty());
}

#[test]
fn reload_reaches_the_presenter() {
    for (current, action) in [(false, "reloadOriginal"), (true, "reloadCurrent")] {
        let mut origin = Origin::pages();
        origin.opened(FILE, |_| {});
        let reloaded = origin.overlay(SessionCmd::OverlayReload, |a| a.current = Some(current));
        assert_eq!(reloaded.result, Some(json!({"id": wire(origin.session())})));
        assert_eq!(origin.actions(), [action]);
    }
}

/// Upstream's `aSlotWithoutARemotePageIsLeftToThisMac`: the commands find this machine's own slot empty.
#[test]
fn a_slot_without_a_remote_page_is_left_to_this_machine() {
    let mut origin = Origin::pages();
    assert_eq!(
        origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some("back".to_owned())),
        failure("no overlay")
    );
    assert_eq!(origin.overlay(SessionCmd::OverlayReload, |_| {}), failure("no overlay"));
}

/// Upstream's `aReservationWhoseStreamIsGoneRefusesWithoutReopening`: a page held for a stream that is no longer the
/// presenter's is refused, and nothing opens here instead.
#[test]
fn a_reservation_whose_stream_is_gone_refuses_without_reopening() {
    let mut origin = Origin::pages();
    let session = origin.session();
    let page = HtmlOverlay::new(
        HtmlSource::File {
            path: FILE.to_owned(),
            grant_root: None,
        },
        HtmlPageFlags::default(),
    );
    let slot = RemoteOverlaySlot {
        job: wire(page.id),
        pane: None,
        owner: 99,
        size_percent: None,
        wait: false,
        ended: false,
        page: Some(RemotePage::new(page, "t".to_owned(), Instant::now())),
    };
    assert!(
        origin
            .fixture()
            .store_mut()
            .unwrap()
            .reserve_remote_overlay(session, slot)
    );
    assert_eq!(origin.overlay(SessionCmd::OverlayReload, |_| {}), failure(VIEWER_GONE));
    assert_eq!(
        origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some("back".to_owned())),
        failure(VIEWER_GONE)
    );
    assert_eq!(
        origin.overlay(SessionCmd::OverlayResize, |a| a.size_percent = Some(40)),
        failure(VIEWER_GONE)
    );
    assert!(origin.commands().is_empty());
    assert_eq!(origin.node()["overlay"], false);
}

/// Upstream's `testAResultOrReadOfARemotePageGivesThePageAnswers`.
#[test]
fn a_result_or_read_of_a_remote_page_gives_the_page_answers() {
    let mut origin = Origin::pages();
    let page = origin.opened(FILE, |_| {});
    assert_eq!(
        origin.overlay(SessionCmd::OverlayResult, |_| {}),
        failure("no overlay result: the slot holds an html page")
    );
    for cmd in [SessionCmd::OverlayText, SessionCmd::OverlayCopy] {
        assert_eq!(
            origin.overlay(cmd, |_| {}),
            failure("no overlay to read: the slot holds an html page"),
            "{cmd:?}"
        );
    }
    assert_eq!(origin.outcome(&page)["outcome"], "pending");
}

// MARK: - the read-back

/// Upstream's `HtmlOverlayTests.aLocalPageNodeOmitsTheRemoteFields` and `aReplicaIsNamedByItsFileAndReportsThe
/// OriginPathAsReplica`.
#[test]
fn a_local_page_omits_the_remote_fields_and_a_replica_reports_its_origin_path() {
    let mut fixture = Fixture::new(2);
    let (first, second) = (fixture.sessions[0], fixture.sessions[1]);
    let local = HtmlOverlay::new(
        HtmlSource::File {
            path: FILE.to_owned(),
            grant_root: None,
        },
        HtmlPageFlags::default(),
    );
    let mut replica = HtmlOverlay::new(
        HtmlSource::Replica {
            path: "/origin/docs/index.html".to_owned(),
        },
        HtmlPageFlags::default(),
    );
    replica.replica_of = Some(Uuid::new_v4());
    let store = fixture.store_mut().unwrap();
    store.open_html_overlay(first, None, local, None, None).unwrap();
    store
        .open_html_overlay(second, Some(OverlayPane::Left), replica, None, None)
        .unwrap();
    let read = node(&mut fixture, first)["htmlOverlays"][0].clone();
    assert!(read.get("remote").is_none() && read.get("replica").is_none(), "{read}");
    assert_eq!(read["zoom"], 1.0, "a local page has this machine's zoom");
    let read = node(&mut fixture, second)["htmlOverlays"][0].clone();
    assert_eq!(read["file"], "/origin/docs/index.html");
    assert!(read.get("cwd").is_none() && read.get("remote").is_none());
    assert_eq!(read["replica"], true);
    let program = ngterm_proto::tree::ControlRemoteOverlayNode {
        pane: None,
        size_percent: Some(60),
        page_id: None,
    };
    assert_eq!(serde_json::to_value(&program).unwrap(), json!({"sizePercent": 60}));
    let page = ngterm_proto::tree::ControlRemoteOverlayNode {
        pane: Some("left".to_owned()),
        size_percent: None,
        page_id: Some("p1".to_owned()),
    };
    let encoded = serde_json::to_value(&page).unwrap();
    assert_eq!(encoded, json!({"pane": "left", "pageID": "p1"}));
    assert_eq!(
        serde_json::from_value::<ngterm_proto::tree::ControlRemoteOverlayNode>(encoded).unwrap(),
        page
    );
}

#[test]
fn the_page_kinds_close_upstreams_list() {
    assert_eq!(&SUPPORTED_KINDS[8..], crate::remote::hub::PAGE_KINDS);
}
