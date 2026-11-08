//! A terminal click over a hidden page an earlier click opened (upstream's `AppStoreClickedPageTests`, the store half
//! of `ControlServerLinkOverlayTests`' clicked-page cases and `AppStoreTreeProjectionTests`' `clicked` read-back): the
//! five click paths mark their pages, and a click replaces the session-wide page only while it is marked and hidden by
//! hand, after every check that could still refuse the new one.

use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::live::live;
use super::overlay_page::fixture_with_host;
use super::remote::{frame, node, set_lead, subscribe_listing};
use super::{Fixture, args, failure, wire};
use crate::html::image_page::ImageData;
use crate::html::{HtmlHost, HtmlNavigation, HtmlOverlay, HtmlSource, errors};
use crate::remote::hub::SUPPORTED_KINDS;

/// Web views that take every page.
struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

fn link(path: &str) -> url::Url {
    url::Url::parse(&format!("http://127.0.0.1:1{path}")).unwrap()
}

/// The page in session `id`'s session-wide slot.
fn slot_page(f: &Fixture, id: Uuid) -> Option<HtmlOverlay> {
    f.store().session(id)?.overlay.page().cloned()
}

fn hidden(f: &Fixture, id: Uuid) -> bool {
    f.store().session(id).unwrap().overlay_hidden(None)
}

fn hide(f: &mut Fixture, id: Uuid) {
    let response = f.session(SessionCmd::OverlayHide, Some(&wire(id)), args(|_| {}));
    assert!(response.ok, "{response:?}");
    assert!(hidden(f, id));
}

fn outcome(f: &mut Fixture, page: Uuid) -> Value {
    let response = f.session(SessionCmd::OverlayResult, None, args(|a| a.page = Some(wire(page))));
    response.result.unwrap_or(Value::Null)["pageOutcome"]["outcome"].clone()
}

/// Upstream's `aHiddenClickedPageIsClosedAndItsSlotFreed`.
#[test]
fn a_hidden_clicked_page_is_closed_and_its_slot_freed() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    assert!(f.core.open_link_overlay(link("/pr/1"), id, None));
    let page = slot_page(&f, id).unwrap();
    assert!(page.clicked);
    hide(&mut f, id);

    assert!(f.core.release_hidden_clicked_page(id));

    let session = f.store().session(id).unwrap();
    assert!(!session.overlay.active && !session.overlay_hidden(None));
    assert!(session.overlay.html.is_none());
    assert_eq!(outcome(&mut f, page.id), json!("dismissed"));
}

/// Upstream's `anyOtherOccupantIsLeftInPlace`: a shown clicked page, a hidden page no click opened and a hidden program
/// all stay.
#[test]
fn any_other_occupant_is_left_in_place() {
    for occupant in ["shown clicked page", "hidden plain page", "hidden program"] {
        let (mut f, _) = fixture_with_host(0);
        let id = f.sessions[0];
        match occupant {
            "shown clicked page" => assert!(f.core.open_link_overlay(link("/pr/1"), id, None)),
            "hidden plain page" => {
                let opened = f.session(
                    SessionCmd::OverlayOpen,
                    Some(&wire(id)),
                    args(|a| a.url = Some(link("/dashboard").to_string())),
                );
                assert!(opened.ok, "{opened:?}");
                assert!(!slot_page(&f, id).unwrap().clicked);
                hide(&mut f, id);
            }
            _ => {
                let opened = f.session(
                    SessionCmd::OverlayOpen,
                    Some(&wire(id)),
                    args(|a| a.command = Some("exec cat".to_owned())),
                );
                assert!(opened.ok, "{opened:?}");
                hide(&mut f, id);
            }
        }
        let was_hidden = hidden(&f, id);

        assert!(!f.core.release_hidden_clicked_page(id), "{occupant}");

        assert!(f.store().session(id).unwrap().overlay.active, "{occupant}");
        assert_eq!(hidden(&f, id), was_hidden, "{occupant}");
    }
}

/// Upstream's `aHiddenClickedPageStaysWhenTheSlotWouldGoToThePresenter`: once every pane follows a presenter's lead,
/// the slot is the presenter's to fill, so the hidden page stays.
#[test]
fn a_hidden_clicked_page_stays_when_the_slot_would_go_to_the_presenter() {
    let mut live = live();
    live.fixture.core.set_html_host(Box::new(Host));
    let (session, left) = (live.session, live.left);
    assert!(live.fixture.core.open_link_overlay(link("/pr/1"), session, None));
    hide(&mut live.fixture, session);
    set_lead(&mut live.fixture.core, left, "follower");
    let (serial, presenter) = subscribe_listing(&mut live.fixture.core, session, "presenter", &SUPPORTED_KINDS);
    let acquire = frame(&presenter, &json!({"kind": "presenter.acquire"}));
    live.fixture.core.presentation_line(serial, &acquire);
    live.fixture.core.commit();
    assert!(live.fixture.core.page_belongs_remote(session, None));

    assert!(!live.fixture.core.release_hidden_clicked_page(session));
    assert!(
        !live.fixture.core.open_link_overlay(link("/pr/2"), session, None),
        "the click keeps the browser fallback"
    );

    assert!(live.fixture.store().session(session).unwrap().overlay.active);
    assert!(hidden(&live.fixture, session));
}

/// Upstream's `anUnknownSessionReleasesNothing`.
#[test]
fn an_unknown_session_releases_nothing() {
    let (mut f, _) = fixture_with_host(0);
    assert!(!f.core.release_hidden_clicked_page(Uuid::new_v4()));
}

/// Upstream's `testALinkReplacesAHiddenPageAnEarlierClickOpenedAndShowsIt`.
#[test]
fn a_link_replaces_a_hidden_page_an_earlier_click_opened_and_shows_it() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    assert!(f.core.open_link_overlay(link("/pr/1"), id, None));
    let old = slot_page(&f, id).unwrap();
    hide(&mut f, id);

    assert!(f.core.open_link_overlay(link("/pr/2"), id, Some(75)));

    let page = slot_page(&f, id).unwrap();
    assert_ne!(page.id, old.id);
    assert_eq!(page.source, HtmlSource::Url(link("/pr/2")));
    assert!(page.clicked);
    assert!(!hidden(&f, id));
    assert_eq!(f.store().session(id).unwrap().overlay.size_percent, Some(75));
    assert_eq!(outcome(&mut f, old.id), json!("dismissed"));
    assert_eq!(outcome(&mut f, page.id), json!("pending"));
}

/// Upstream's `testALinkLeavesAHiddenPageNoClickOpened`: a page the socket opened, even one shaped like a click's.
#[test]
fn a_link_leaves_a_hidden_page_no_click_opened() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    let opened = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| {
            a.url = Some(link("/dashboard").to_string());
            a.navigation = Some(true);
            a.javascript = Some(true);
            a.persistent = Some(true);
            a.browse = Some(true);
        }),
    );
    assert!(opened.ok, "{opened:?}");
    let kept = slot_page(&f, id).unwrap();
    assert!(!kept.clicked);
    hide(&mut f, id);

    assert!(!f.core.open_link_overlay(link("/pr/1"), id, None));

    assert_eq!(slot_page(&f, id).map(|page| page.id), Some(kept.id));
    assert!(hidden(&f, id));
}

/// Upstream's `testALinkLeavesAHiddenClickedPageWhenTheSavedStoreIsUnusable`, and the same for web views that cannot
/// run: the refusal comes before the old page goes.
#[test]
fn a_link_leaves_a_hidden_clicked_page_when_its_open_would_be_refused() {
    let (mut f, log) = fixture_with_host(0);
    let id = f.sessions[0];
    let markdown = HtmlSource::Markdown {
        path: crate::html::os_path("/tmp/agdoc/doc.md"),
    };
    assert!(f.core.open_document_link(markdown, id, None, None));
    let kept = slot_page(&f, id).unwrap();
    assert!(kept.clicked);
    hide(&mut f, id);

    log.borrow_mut().store_failure = Some(errors::PERSISTENT_UNAVAILABLE.to_owned());
    assert!(!f.core.open_link_overlay(link("/"), id, None));
    log.borrow_mut().store_failure = None;
    log.borrow_mut().unavailable = Some("no engine".to_owned());
    assert!(!f.core.open_link_overlay(link("/"), id, None));
    let text = HtmlSource::Text {
        path: crate::html::os_path("/tmp/agdoc/main.go"),
        line: None,
    };
    assert!(!f.core.open_document_link(text, id, None, None));

    assert_eq!(slot_page(&f, id).map(|page| page.id), Some(kept.id));
    assert!(hidden(&f, id));
    assert_eq!(outcome(&mut f, kept.id), json!("pending"));
}

fn picture() -> ImageData {
    ImageData {
        bytes: vec![0x89, b'P', b'N', b'G'],
        mime_type: "image/png".to_owned(),
        width: 40,
        height: 30,
        orientation: 1,
    }
}

/// Upstream's `testEachClickedFileKindReplacesAHiddenClickedPage`, with the clicked HTML document beside them.
#[test]
fn each_clicked_file_kind_replaces_a_hidden_clicked_page() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    let markdown = HtmlSource::Markdown {
        path: crate::html::os_path("/tmp/agdoc/doc.md"),
    };
    assert!(f.core.open_document_link(markdown.clone(), id, None, None));
    let kinds = [
        (
            HtmlSource::Text {
                path: crate::html::os_path("/tmp/agdoc/main.go"),
                line: Some(3),
            },
            None,
        ),
        (
            HtmlSource::Image {
                path: crate::html::os_path("/tmp/agdoc/shot.png"),
            },
            Some(picture()),
        ),
        (
            HtmlSource::Document {
                path: crate::html::os_path("/tmp/agdoc/pr.html"),
            },
            None,
        ),
        (markdown, None),
    ];
    for (source, image) in kinds {
        hide(&mut f, id);
        assert!(f.core.open_document_link(source.clone(), id, None, image), "{source:?}");
        let page = slot_page(&f, id).unwrap();
        assert_eq!(page.source, source);
        assert!(page.clicked, "{source:?}");
        assert!(!hidden(&f, id), "{source:?}");
    }
}

/// A picture no probe could read refuses before the hidden clicked page goes.
#[test]
fn a_clicked_picture_that_cannot_be_read_leaves_the_hidden_clicked_page() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    assert!(f.core.open_link_overlay(link("/pr/1"), id, None));
    let kept = slot_page(&f, id).unwrap();
    hide(&mut f, id);
    let image = HtmlSource::Image {
        path: crate::html::os_path("/tmp/agdoc/missing.png"),
    };
    assert!(!f.core.open_document_link(image, id, None, None));
    assert_eq!(slot_page(&f, id).map(|page| page.id), Some(kept.id));
    assert!(hidden(&f, id));
}

/// A hidden program keeps the slot: the click is refused `overlay hidden` and nothing opens.
#[test]
fn a_hidden_program_keeps_the_click_out() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    let opened = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| a.command = Some("exec cat".to_owned())),
    );
    assert!(opened.ok, "{opened:?}");
    hide(&mut f, id);
    assert!(!f.core.open_link_overlay(link("/pr/1"), id, None));
    let markdown = HtmlSource::Markdown {
        path: crate::html::os_path("/tmp/agdoc/doc.md"),
    };
    assert!(!f.core.open_document_link(markdown, id, None, None));
    assert!(slot_page(&f, id).is_none());
    assert!(hidden(&f, id));
}

/// The socket never marks a page, and a socket open into a hidden clicked page's slot is still refused.
#[test]
fn the_socket_never_replaces_a_hidden_clicked_page() {
    let (mut f, _) = fixture_with_host(0);
    let id = f.sessions[0];
    assert!(f.core.open_link_overlay(link("/pr/1"), id, None));
    hide(&mut f, id);
    let opened = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| a.url = Some(link("/dashboard").to_string())),
    );
    assert_eq!(opened, failure("overlay hidden"));
    assert!(slot_page(&f, id).unwrap().clicked);
}

/// Upstream's `AppStoreTreeProjectionTests` `clicked` read-back: `true` for the clicked page only, never `false`.
#[test]
fn only_a_clicked_page_reads_back_clicked() {
    let (mut f, _) = fixture_with_host(1);
    let (clicked, plain) = (f.sessions[0], f.sessions[1]);
    assert!(f.core.open_link_overlay(link("/pr/1"), clicked, None));
    let opened = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(plain)),
        args(|a| a.url = Some(link("/dashboard").to_string())),
    );
    assert!(opened.ok, "{opened:?}");
    assert_eq!(node(&mut f, clicked)["htmlOverlays"][0]["clicked"], json!(true));
    let read = node(&mut f, plain)["htmlOverlays"][0].clone();
    assert!(read.get("clicked").is_none(), "{read}");
    let tree = serde_json::to_string(&f.core.tree(None).unwrap()).unwrap();
    assert_eq!(tree.matches(r#""clicked":true"#).count(), 1);
    assert!(!tree.contains(r#""clicked":false"#));
}
