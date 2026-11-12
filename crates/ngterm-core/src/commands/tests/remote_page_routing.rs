//! Where a session's pages and program overlays go when its viewers differ in what they show (this build's extension):
//! the presenter role stays first come, so asks and sounds keep going to whoever took it, while pages go to a viewer
//! that shows them when the presenter shows none, and program overlays too when the presenter refuses them (a phone).

use ngterm_proto::command::{AskCmd, SessionCmd};
use ngterm_proto::{Args, AskButton, Command, Request, Response};
use serde_json::json;

use super::remote::{Viewer, frame, subscribe_listing};
use super::remote_page::Origin;
use super::{args, failure, wire};
use crate::remote::client::viewer_kinds;
use crate::remote::hub::{NO_OVERLAYS_KIND, SUPPORTED_KINDS};

const PRESENTER: &str = "a viewer presents this session: an html overlay would open where nobody sees it";
const FILE: &str = "/tmp/docs/index.html";

fn phone_kinds() -> Vec<String> {
    let mut kinds = viewer_kinds(false);
    kinds.push(NO_OVERLAYS_KIND.to_owned());
    kinds
}

fn listed(kinds: &[String]) -> Vec<&str> {
    kinds.iter().map(String::as_str).collect()
}

fn attach(origin: &mut Origin, kinds: &[&str]) -> (u64, Viewer) {
    let session = origin.session();
    let core = &mut origin.live.fixture.core;
    let (serial, viewer) = subscribe_listing(core, session, "presenter", kinds);
    core.presentation_line(serial, &frame(&viewer, &json!({"kind": "presenter.acquire"})));
    core.commit();
    (serial, viewer)
}

fn mode(viewer: &Viewer) -> &'static str {
    match viewer.kinds().iter().rev().find(|kind| kind.starts_with("presenter.")) {
        Some(kind) if kind == "presenter.granted" => "presenter",
        Some(_) => "mirror",
        None => "none",
    }
}

fn open_page(origin: &mut Origin) -> Response {
    origin.open(FILE, |_| {})
}

fn open_ask(origin: &mut Origin) -> Response {
    let session = origin.session();
    let response = origin.live.fixture.core.dispatch(&Request {
        cmd: Command::Ask(AskCmd::Open),
        target: Some(wire(session)),
        args: Some(args(|a: &mut Args| {
            a.title = Some("t".to_owned());
            a.buttons = Some(vec![AskButton {
                id: "yes".to_owned(),
                label: "Yes".to_owned(),
                hotkey: None,
            }]);
        })),
    });
    origin.live.fixture.core.commit();
    response
}

fn open_program(origin: &mut Origin) -> Response {
    origin.overlay(SessionCmd::OverlayOpen, |a| {
        a.command = Some("sleep 9".to_owned());
        a.pane = Some("left".to_owned());
    })
}

fn page_id(response: &Response) -> String {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().unwrap()["pageID"].as_str().unwrap().to_owned()
}

/// The user's case: the phone attached first and holds the role, the desktop attached after it as a mirror. The page
/// opens on the desktop, its reports count there, and the phone still gets the session's asks.
#[test]
fn a_page_goes_to_a_mirror_showing_pages_when_the_phone_presents() {
    let phone = phone_kinds();
    let mut origin = Origin::new(false, &listed(&phone));
    let (desktop_serial, desktop) = attach(&mut origin, &SUPPORTED_KINDS);
    assert_eq!((mode(&origin.presenter), mode(&desktop)), ("presenter", "mirror"));

    let page = page_id(&open_page(&mut origin));
    assert_eq!(desktop.last("page.request").unwrap()["page"]["page"], json!(page));
    assert!(origin.presenter.last("page.request").is_none());

    let state = json!({"kind": "page.state", "page": {"page": page, "address": FILE, "load": "loaded",
        "canGoBack": false, "canGoForward": false, "hidden": false, "appliedSeq": 0, "title": "Docs"}});
    origin
        .live
        .fixture
        .core
        .presentation_line(desktop_serial, &frame(&desktop, &state));
    origin.live.fixture.core.commit();
    assert_eq!(origin.node()["htmlOverlays"][0]["title"], json!("Docs"));

    assert!(open_ask(&mut origin).ok);
    assert!(
        origin.presenter.last("ask.request").is_some(),
        "the phone keeps the asks"
    );
    assert!(desktop.last("ask.request").is_none());

    assert!(origin.overlay(SessionCmd::OverlayClose, |_| {}).ok);
    assert_eq!(desktop.last("page.close").unwrap()["page"], json!({"page": page}));
    assert!(origin.presenter.last("page.close").is_none());
}

/// A page the desktop shows beside the phone is parked when the desktop goes, and handed to the next viewer that shows
/// pages; the phone is never handed it.
#[test]
fn a_page_on_the_mirror_parks_when_it_goes_and_moves_to_the_next() {
    let phone = phone_kinds();
    let mut origin = Origin::new(false, &listed(&phone));
    let (desktop_serial, _desktop) = attach(&mut origin, &SUPPORTED_KINDS);
    page_id(&open_page(&mut origin));

    origin.live.fixture.core.presentation_closed(desktop_serial);
    origin.live.fixture.core.commit();
    assert_eq!(origin.node()["htmlOverlays"][0]["parked"], json!(true));
    assert!(origin.presenter.last("page.request").is_none());

    let (_, next) = attach(&mut origin, &SUPPORTED_KINDS);
    assert!(next.last("page.request").is_some());
    assert!(origin.presenter.last("page.request").is_none());
}

/// The desktop attached first holds the role and the pages, as before; the phone after it is a mirror handed nothing.
#[test]
fn the_desktop_presenting_first_keeps_pages_and_asks() {
    let phone = phone_kinds();
    let mut origin = Origin::pages();
    let (_, phone_viewer) = attach(&mut origin, &listed(&phone));
    assert_eq!(mode(&phone_viewer), "mirror");

    page_id(&open_page(&mut origin));
    assert!(origin.presenter.last("page.request").is_some());
    assert!(open_ask(&mut origin).ok);
    assert!(origin.presenter.last("ask.request").is_some());
    assert!(phone_viewer.last("page.request").is_none());
    assert!(phone_viewer.last("ask.request").is_none());
}

/// With only the phone attached a page is refused as before.
#[test]
fn a_phone_alone_keeps_the_refusal() {
    let phone = phone_kinds();
    let mut origin = Origin::new(false, &listed(&phone));
    let (_, other_phone) = attach(&mut origin, &listed(&phone));
    assert_eq!(open_page(&mut origin), failure(PRESENTER));
    assert!(origin.presenter.last("page.request").is_none());
    assert!(other_phone.last("page.request").is_none());
}

/// A viewer that asked to stay a mirror is never handed pages, whatever it lists.
#[test]
fn a_viewer_that_asked_to_mirror_is_handed_no_page() {
    let phone = phone_kinds();
    let mut origin = Origin::new(false, &listed(&phone));
    let session = origin.session();
    let (_, mirror) = subscribe_listing(&mut origin.live.fixture.core, session, "mirror", &SUPPORTED_KINDS);
    assert_eq!(open_page(&mut origin), failure(PRESENTER));
    assert!(mirror.last("page.request").is_none());
}

/// The phone leaving takes the presenter role, and with it what the desktop beside it showed: the page is parked and
/// the program overlay goes, and the desktop, still attached, is told to drop both.
#[test]
fn the_phone_leaving_closes_the_page_and_the_overlay_on_the_desktop_that_stays() {
    let phone = phone_kinds();
    let mut origin = Origin::new(false, &listed(&phone));
    let (_, desktop) = attach(&mut origin, &SUPPORTED_KINDS);
    let page = page_id(&open_page(&mut origin));
    assert!(open_program(&mut origin).ok);
    let job = desktop.last("overlay.request").expect("the desktop shows the overlay")["overlay"]["job"].clone();
    assert!(origin.presenter.last("overlay.request").is_none());

    origin.live.fixture.core.presentation_closed(origin.serial);
    origin.live.fixture.core.commit();

    assert_eq!(origin.node()["htmlOverlays"][0]["parked"], json!(true));
    assert_eq!(desktop.last("page.close").unwrap()["page"], json!({"page": page}));
    assert_eq!(desktop.last("overlay.close").unwrap()["overlay"]["job"], job);
}

/// Upstream keeps a desktop presenter's program overlays on it whether or not it has web views; only the pages it
/// would refuse go to a desktop beside it that shows them.
#[test]
fn a_desktop_presenter_without_web_views_keeps_its_overlays_while_its_pages_go_beside() {
    let bare = viewer_kinds(false);
    let mut origin = Origin::new(false, &listed(&bare));
    let (_, desktop) = attach(&mut origin, &SUPPORTED_KINDS);
    assert_eq!((mode(&origin.presenter), mode(&desktop)), ("presenter", "mirror"));

    assert!(open_program(&mut origin).ok);
    assert!(origin.presenter.last("overlay.request").is_some());
    assert!(desktop.last("overlay.request").is_none());

    let page = page_id(&open_page(&mut origin));
    assert_eq!(desktop.last("page.request").unwrap()["page"]["page"], json!(page));
    assert!(origin.presenter.last("page.request").is_none());
}

/// The desktop's answer to the `page.close` that parked its page can arrive after the phone came back and the page was
/// handed to the desktop again: that answer belongs to the old replica, so the page stays open until the desktop closes
/// the new one.
#[test]
fn a_late_answer_to_the_park_leaves_the_page_handed_back_to_the_same_desktop_open() {
    let phone = phone_kinds();
    let mut origin = Origin::new(false, &listed(&phone));
    let (desktop_serial, desktop) = attach(&mut origin, &SUPPORTED_KINDS);
    let page = page_id(&open_page(&mut origin));

    origin.live.fixture.core.presentation_closed(origin.serial);
    origin.live.fixture.core.commit();
    assert_eq!(desktop.last("page.close").unwrap()["page"], json!({"page": page}));
    attach(&mut origin, &listed(&phone));
    let requests = desktop.kinds().iter().filter(|kind| *kind == "page.request").count();
    assert_eq!(requests, 2, "the page is handed back to the desktop");

    let closed = json!({"kind": "page.closed", "page": {"page": page}});
    let core = &mut origin.live.fixture.core;
    core.presentation_line(desktop_serial, &frame(&desktop, &closed));
    core.commit();
    let node = origin.node();
    assert_eq!(node["htmlOverlays"][0]["id"], json!(page), "{node}");
    assert_ne!(node["htmlOverlays"][0]["parked"], json!(true));

    let core = &mut origin.live.fixture.core;
    core.presentation_line(desktop_serial, &frame(&desktop, &closed));
    core.commit();
    assert!(origin.node()["htmlOverlays"][0].is_null());
}

/// Each close of a page dropped twice before the desktop answered takes its own answer, in whichever order its two
/// answers come, so neither ends the page handed back a second time; the desktop's next close does.
#[test]
fn both_answers_to_a_page_dropped_twice_leave_the_page_handed_back_open() {
    let answer = |kind: &str, page: &str| json!({"kind": kind, "page": {"page": page}});
    for answers in [["page.closed", "page.rejected"], ["page.rejected", "page.closed"]] {
        let phone = phone_kinds();
        let mut origin = Origin::new(false, &listed(&phone));
        let (desktop_serial, desktop) = attach(&mut origin, &SUPPORTED_KINDS);
        let page = page_id(&open_page(&mut origin));
        let mut phone_serial = origin.serial;
        for _ in 0..2 {
            origin.live.fixture.core.presentation_closed(phone_serial);
            origin.live.fixture.core.commit();
            phone_serial = attach(&mut origin, &listed(&phone)).0;
        }
        let count = |kind: &str| desktop.kinds().iter().filter(|seen| *seen == kind).count();
        assert_eq!(
            (count("page.close"), count("page.request")),
            (2, 3),
            "dropped twice, handed back twice"
        );

        for kind in answers {
            let core = &mut origin.live.fixture.core;
            core.presentation_line(desktop_serial, &frame(&desktop, &answer(kind, &page)));
            core.commit();
            let node = origin.node();
            assert_eq!(node["htmlOverlays"][0]["id"], json!(page), "{node}");
            assert_ne!(node["htmlOverlays"][0]["parked"], json!(true), "{node}");
        }

        let core = &mut origin.live.fixture.core;
        core.presentation_line(desktop_serial, &frame(&desktop, &answer("page.closed", &page)));
        core.commit();
        assert!(origin.node()["htmlOverlays"][0].is_null());
    }
}

/// A program overlay already open on a desktop presenter without web views stays there when a desktop that shows
/// pages attaches beside it and takes them.
#[test]
fn an_open_overlay_stays_on_the_presenter_when_a_desktop_beside_it_takes_the_pages() {
    let bare = viewer_kinds(false);
    let mut origin = Origin::new(false, &listed(&bare));
    assert!(
        origin
            .overlay(SessionCmd::OverlayOpen, |a| a.command = Some("sleep 9".to_owned()))
            .ok
    );
    assert!(origin.presenter.last("overlay.request").is_some());

    let (_, desktop) = attach(&mut origin, &SUPPORTED_KINDS);
    assert!(origin.presenter.last("overlay.close").is_none());
    assert!(desktop.last("overlay.request").is_none());

    let response = origin.overlay(SessionCmd::OverlayResize, |a| a.size_percent = Some(50));
    assert!(response.ok, "{response:?}");
    assert!(origin.presenter.last("overlay.resize").is_some());
    assert!(desktop.last("overlay.resize").is_none());
}
