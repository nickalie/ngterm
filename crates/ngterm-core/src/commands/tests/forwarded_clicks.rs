//! Paths clicked in an attached pane, opened by its origin (upstream's `ControlServerForwardedClickTests`,
//! `ControlServerRemoteLinkClickTests`, `AppStoreLinkClickRequestTests`, `AppStoreForwardedClickTests` and the
//! forwarded cases of `AppStoreRemotePageTests`, #923): on the origin, each page kind reaches the presenter resolved
//! with the clicked pane's directory, and every click it cannot open is refused once; on the viewer, a click travels
//! only while this machine presents the session to an origin that speaks the kind, and every refusal shows a panel
//! that hides itself or plays the alert sound.

use serde_json::{Value, json};
use uuid::Uuid;

use super::content_pages::{Files, content_origin};
use super::remote::set_lead;
use super::remote_page::Origin;
use super::viewer::{Viewer, viewer_built};
use super::wire;
use crate::hud::HudSpec;
use crate::persist::settings::LinkOpenMode;
use crate::remote::frames::PresentationHud;
use crate::remote::hub::{LINK_CLICK_KIND, SUPPORTED_KINDS};
use crate::remote::link_click::{RemoteLinkClick, RemoteLinkRefusal};

/// Opens a HUD saying `message` over `session` as `session.hud.open` does.
fn open_hud(fixture: &mut super::Fixture, session: Uuid, message: &str) {
    let response = fixture.session(
        ngterm_proto::command::SessionCmd::HudOpen,
        Some(&wire(session)),
        super::args(|a| a.message = Some(message.to_owned())),
    );
    assert!(response.ok, "{response:?}");
}

// MARK: - origin

/// A presented split session whose presenter speaks every kind, its panes' directories set, and a folder of content
/// files with a `src/` beside it.
fn origin() -> (Origin, Files) {
    let files = Files::new();
    std::fs::create_dir_all(files.root.join("src")).unwrap();
    std::fs::write(files.root.join("src/main.py"), "print(1)\n").unwrap();
    std::fs::write(files.root.join("tool.bin"), [0_u8, 1, 2]).unwrap();
    let split = files.root.join("split");
    std::fs::create_dir_all(split.join("src")).unwrap();
    std::fs::write(split.join("src/main.py"), "print(2)\n").unwrap();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    origin.fixture().core.set_hud_body_dir(files.root.join("huds"));
    let session = origin.session();
    let (left, right) = (files.path(""), files.path("split"));
    let store = origin.fixture().store_mut().unwrap();
    let found = store.session_mut(session).unwrap();
    found.report_pwd(false, left.trim_end_matches('/'));
    found.report_pwd(true, &right);
    (origin, files)
}

fn identities(origin: &Origin) -> (Uuid, Uuid) {
    let session = origin.live.fixture.store().session(origin.session()).unwrap();
    (session.pane_identity, session.split_pane_identity.unwrap())
}

/// The presenter clicks `raw` in the pane `pane` names, in `mode`.
fn click(origin: &mut Origin, raw: &str, pane: Uuid, mode: &str) {
    origin.report(&json!({"kind": "link.click", "link": {"raw": raw, "pane": wire(pane), "mode": mode}}));
}

fn requests(origin: &Origin) -> Vec<Value> {
    origin.presenter.frames("page.request")
}

fn refusals(origin: &Origin) -> Vec<Value> {
    origin
        .presenter
        .frames("link.refused")
        .into_iter()
        .map(|frame| frame["link"]["reason"].clone())
        .collect()
}

fn session_overlay_active(origin: &Origin) -> bool {
    origin
        .live
        .fixture
        .store()
        .session(origin.session())
        .unwrap()
        .overlay
        .active
}

/// Upstream's `testEachPageKindAClickNamesOpensOnThePresenterAndNothingHere`: a Markdown, picture, text and HTML path each go to the presenter
/// as a clicked page, nothing covers the session here, and no refusal is sent.
#[test]
fn each_page_kind_a_forwarded_click_names_reaches_the_presenter() {
    for (name, kind) in [
        ("README.md", "markdown"),
        ("shot.png", "image"),
        ("main.go", "text"),
        ("pr.html", "document"),
    ] {
        let (mut origin, files) = origin();
        let (left, _) = identities(&origin);
        click(&mut origin, &files.path(name), left, "overlay");
        let sent = requests(&origin);
        assert_eq!(sent.len(), 1, "{name}");
        let page = &sent[0]["page"];
        assert_eq!(page["source"][kind]["path"], json!(files.path(name)), "{name}: {page}");
        let session = origin.live.fixture.store().session(origin.session()).unwrap();
        let reserved = session.remote_overlays().slot(None).unwrap().page.as_ref().unwrap();
        assert!(reserved.overlay.clicked, "{name}");
        assert!(refusals(&origin).is_empty(), "{name}");
        assert!(
            !session_overlay_active(&origin),
            "{name}: nothing renders on the origin"
        );
    }
}

/// Upstream's `testARelativePathOpensAtItsLineAgainstTheClickedPanesOwnDirectory` and
/// `aForwardedClickNamesThePrimaryOrTheSplitByItsOwnDirectory`: a relative `path:line` resolves against the
/// clicked pane's directory, the primary's and the split's own, and opens at its line.
#[test]
fn a_relative_path_resolves_against_the_clicked_panes_own_directory() {
    let (mut origin, files) = origin();
    let (left, right) = identities(&origin);
    click(&mut origin, "src/main.py:42", left, "overlay");
    click(&mut origin, "src/main.py:7", right, "overlay");
    let sent = requests(&origin);
    assert_eq!(
        sent[0]["page"]["source"]["text"],
        json!({"path": files.path("src/main.py"), "line": 42})
    );
    assert_eq!(
        sent.len(),
        1,
        "the split's page waits behind the primary's in the session-wide slot"
    );
    assert_eq!(refusals(&origin), [json!("notOpened")]);
    let (mut origin, files) = self::origin();
    let (_, right) = identities(&origin);
    click(&mut origin, "src/main.py:7", right, "overlay");
    assert_eq!(
        requests(&origin)[0]["page"]["source"]["text"],
        json!({"path": files.path("split/src/main.py"), "line": 7})
    );
}

/// Upstream's `testThePageTakesTheSizeTheClickersModeNames` and `testABrowserModeClickOpensAtNinetyFivePercent`: browser mode shows the page at 95%, an overlay mode at its size.
#[test]
fn the_page_size_follows_the_mode_the_click_carried() {
    for (mode, size) in [
        ("browser", json!(95)),
        ("overlay75", json!(75)),
        ("someday", Value::Null),
    ] {
        let (mut origin, files) = origin();
        let (left, _) = identities(&origin);
        click(&mut origin, &files.path("README.md"), left, mode);
        let page = &requests(&origin)[0]["page"];
        assert_eq!(page.get("sizePercent").cloned().unwrap_or(Value::Null), size, "{mode}");
    }
}

/// Upstream's `testAClickThatNamesNoPageIsRefusedOnceAndOpensNothing` and `anUnknownPaneOrSessionNamesNothing`: a missing file, a relative path no candidate
/// matched, an unknown pane, a file no page shows, a web link and a busy slot each answer one refusal and open nothing,
/// here or there.
#[test]
fn a_click_the_origin_cannot_open_is_refused_once() {
    let cases: [(&str, bool, &str); 6] = [
        ("/nowhere/gone.md", true, "notFound"),
        ("src/gone.py:3", true, "notFound"),
        ("README.md", false, "paneNotAttached"),
        ("tool.bin", true, "notAPage"),
        ("https://example.com/", true, "notAPage"),
        ("file:///nowhere/gone.png", true, "notFound"),
    ];
    for (raw, known, reason) in cases {
        let (mut origin, files) = origin();
        let (left, _) = identities(&origin);
        let raw = if raw == "tool.bin" {
            files.path(raw)
        } else {
            raw.to_owned()
        };
        let pane = if known { left } else { Uuid::new_v4() };
        click(&mut origin, &raw, pane, "overlay");
        assert_eq!(refusals(&origin), [json!(reason)], "{raw}");
        assert!(requests(&origin).is_empty(), "{raw}");
        assert!(!session_overlay_active(&origin), "{raw}");
    }
}

/// Upstream's `testAClickIntoABusySlotIsRefused` and `aForwardedClickLeavesAShownOrUnclickedPageInPlace`: a page the presenter
/// shows, and a hidden one no click opened, keep the slot.
#[test]
fn a_shown_or_unclicked_page_keeps_its_slot() {
    for hidden in [false, true] {
        let (mut origin, files) = origin();
        let (left, _) = identities(&origin);
        let page = origin.opened("https://example.com/", |_| {});
        origin.state(&page, hidden, 0, None);
        click(&mut origin, &files.path("README.md"), left, "overlay");
        assert_eq!(refusals(&origin), [json!("notOpened")], "hidden {hidden}");
        assert_eq!(requests(&origin).len(), 1, "hidden {hidden}");
        assert!(origin.presenter.frames("page.close").is_empty(), "hidden {hidden}");
    }
}

/// Upstream's `testAForwardedClickReplacesItsHiddenPageAndKeepsItWhenTheNewFileIsMissing`,
/// `aForwardedClickReplacesAHiddenClickedPage` and `aRefusedForwardedClickKeepsTheHiddenPage`: a clicked
/// page the presenter hides gives way to the next forwarded click, closed there before the new page is asked for; a
/// click that is refused leaves it in place.
#[test]
fn a_hidden_clicked_page_gives_way_to_the_next_forwarded_click() {
    let (mut origin, files) = origin();
    let (left, _) = identities(&origin);
    click(&mut origin, &files.path("README.md"), left, "overlay");
    let first = requests(&origin)[0]["page"]["page"].as_str().unwrap().to_owned();
    origin.state(&first, true, 0, None);
    click(&mut origin, "/nowhere/gone.md", left, "overlay");
    assert_eq!(refusals(&origin), [json!("notFound")]);
    assert!(
        origin.presenter.frames("page.close").is_empty(),
        "a refused click keeps it"
    );
    click(&mut origin, &files.path("main.go"), left, "overlay");
    let closes = origin.presenter.frames("page.close");
    assert_eq!(closes.len(), 1);
    assert_eq!(closes[0]["page"]["page"], json!(first));
    let sent = requests(&origin);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["page"]["source"]["text"]["path"], json!(files.path("main.go")));
    let kinds = origin.presenter.kinds();
    let close = kinds.iter().rposition(|kind| kind == "page.close").unwrap();
    let request = kinds.iter().rposition(|kind| kind == "page.request").unwrap();
    assert!(close < request, "{kinds:?}");
    assert_eq!(origin.outcome(&first)["outcome"], "dismissed");
}

/// Upstream's `aForwardedClickAPresenterCannotShowKeepsTheHiddenPage`: a presenter that builds no HTML document refuses
/// the click before the hidden clicked page is closed.
#[test]
fn a_forwarded_page_the_presenter_cannot_build_keeps_the_hidden_page() {
    let files = Files::new();
    let kinds: Vec<&str> = SUPPORTED_KINDS
        .into_iter()
        .filter(|kind| *kind != crate::remote::hub::PAGE_DOCUMENT_KIND)
        .collect();
    let mut origin = content_origin(&kinds);
    let session = origin.session();
    let left = files.path("");
    let store = origin.fixture().store_mut().unwrap();
    store
        .session_mut(session)
        .unwrap()
        .report_pwd(false, left.trim_end_matches('/'));
    let primary = origin.live.left;
    click(&mut origin, &files.path("README.md"), primary, "overlay");
    let first = requests(&origin)[0]["page"]["page"].as_str().unwrap().to_owned();
    origin.state(&first, true, 0, None);
    click(&mut origin, &files.path("pr.html"), primary, "overlay");
    assert_eq!(refusals(&origin), [json!("notOpened")]);
    assert!(origin.presenter.frames("page.close").is_empty());
    assert_eq!(requests(&origin).len(), 1);
}

/// Upstream's `testEveryPageKindIsRefusedWhenAPaneLedHereKeepsItHere`: with one pane led from the origin the session-wide page would
/// open here, so the click is refused and nothing opens on either machine.
#[test]
fn a_click_on_a_session_led_from_the_origin_is_refused_for_every_page_kind() {
    for name in ["README.md", "shot.png", "main.go", "pr.html"] {
        let (mut origin, files) = origin();
        let (left, right) = identities(&origin);
        set_lead(&mut origin.fixture().core, right, "leader");
        origin.fixture().core.commit();
        click(&mut origin, &files.path(name), left, "overlay");
        assert_eq!(refusals(&origin), [json!("notOpened")], "{name}");
        assert!(requests(&origin).is_empty(), "{name}");
        assert!(!session_overlay_active(&origin), "{name}");
    }
}

/// Upstream's `testAZoomedTerminalOnThisMacDoesNotBlockAForwardedClick`: a zoomed terminal on the origin is not what the presenter shows, while a
/// HUD on the session still refuses the page.
#[test]
fn the_origins_zoom_does_not_block_a_forwarded_click_and_its_hud_does() {
    let (mut origin, files) = origin();
    let (left, _) = identities(&origin);
    let (window, session) = (origin.live.fixture.window, origin.session());
    origin.fixture().core.set_zoom_target(
        window,
        Some(crate::zoom::SurfaceId {
            session,
            surface: crate::zoom::ZoomSurface::Primary,
        }),
    );
    click(&mut origin, &files.path("README.md"), left, "overlay");
    assert_eq!(requests(&origin).len(), 1);
    assert!(refusals(&origin).is_empty());

    let (mut origin, files) = self::origin();
    let (left, _) = identities(&origin);
    open_hud(&mut origin.live.fixture, origin.live.session, "busy");
    click(&mut origin, &files.path("README.md"), left, "overlay");
    assert_eq!(refusals(&origin), [json!("notOpened")]);
    assert!(requests(&origin).is_empty());
}

/// Upstream's `onlyThePresentersLinkClickReachesTheHost` through the origin: only the presenter's click is taken; a mirror's goes nowhere and
/// gets no answer.
#[test]
fn a_mirrors_click_is_dropped() {
    let (mut origin, files) = origin();
    let (left, _) = identities(&origin);
    let session = origin.session();
    let (serial, mirror) =
        super::remote::subscribe_listing(&mut origin.fixture().core, session, "mirror", &SUPPORTED_KINDS);
    let line = super::remote::frame(
        &mirror,
        &json!({"kind": "link.click", "link": {"raw": files.path("README.md"), "pane": wire(left), "mode": "overlay"}}),
    );
    origin.fixture().core.presentation_line(serial, &line);
    origin.fixture().core.commit();
    assert!(requests(&origin).is_empty());
    assert!(mirror.frames("link.refused").is_empty() && refusals(&origin).is_empty());
}

// MARK: - viewer

/// A presenting viewer of the origin's split session, the origin's hello listing `kinds`, HUDs writable.
fn presenting(kinds: &[&str]) -> Viewer {
    let dir = tempfile::tempdir().unwrap();
    let bodies = dir.keep().join("bodies");
    let mut viewer = viewer_built(true, |fixture| fixture.core.set_hud_body_dir(bodies));
    viewer.connect_listing(kinds, &json!({}));
    viewer.frame(json!({"kind": "presenter.granted"}));
    viewer
}

fn with_link_click() -> Vec<&'static str> {
    vec!["status", "hud", LINK_CLICK_KIND]
}

fn local_panes(viewer: &Viewer) -> (Uuid, Uuid) {
    let session = viewer.session();
    (session.pane_identity, session.split_pane_identity.unwrap())
}

fn forward(viewer: &mut Viewer, raw: &str, pane: Option<Uuid>, reveal: bool) {
    let click = RemoteLinkClick {
        raw: raw.to_owned(),
        session: viewer.session,
        pane,
        mode: LinkOpenMode::Overlay75,
    };
    viewer.fixture.core.forward_remote_link_click(&click, reveal);
    viewer.fixture.core.commit();
}

fn hud_message(viewer: &mut Viewer) -> Option<String> {
    viewer.node()["hud"]["message"].as_str().map(str::to_owned)
}

/// Upstream's `testAClickTravelsToTheOriginWithItsPaneIdentityAndMode` and
/// `aClickInAnAttachedPaneTravelsWithTheOriginsPaneIdentity`: each pane's click goes to the origin as typed,
/// with the origin's identity of that pane and the mode, and nothing shows here.
#[test]
fn a_presenters_click_travels_with_the_origins_pane_identity() {
    let mut viewer = presenting(&with_link_click());
    let (left, right) = local_panes(&viewer);
    forward(&mut viewer, "src/main.py:42", Some(left), false);
    forward(&mut viewer, "/repo/README.md", Some(right), false);
    let sent = viewer.sent("link.click");
    assert_eq!(
        sent.iter().map(|frame| frame["link"].clone()).collect::<Vec<_>>(),
        [
            json!({"raw": "src/main.py:42", "pane": wire(viewer.origin.0), "mode": "overlay75"}),
            json!({"raw": "/repo/README.md", "pane": wire(viewer.origin.1), "mode": "overlay75"}),
        ]
    );
    assert_eq!(hud_message(&mut viewer), None);
    assert!(viewer.fixture.core.take_status_sounds().is_empty());
}

/// Upstream's `testAClickThatCannotTravelSendsNothing` and `eachReasonAClickCannotTravelIsNamed`: a mirror, an origin without the kind, the reveal chord
/// and a pane with no origin twin each show their reason in a panel that hides itself, and nothing is sent.
#[test]
fn a_click_that_cannot_travel_shows_why_and_sends_nothing() {
    let cases: [(&[&str], bool, bool, bool, RemoteLinkRefusal); 4] = [
        (
            &["status", "hud", LINK_CLICK_KIND],
            false,
            true,
            false,
            RemoteLinkRefusal::NotPresenter,
        ),
        (&["status", "hud"], true, true, false, RemoteLinkRefusal::OriginTooOld),
        (
            &["status", "hud", LINK_CLICK_KIND],
            true,
            true,
            true,
            RemoteLinkRefusal::RevealIntent,
        ),
        (
            &["status", "hud", LINK_CLICK_KIND],
            true,
            false,
            false,
            RemoteLinkRefusal::PaneNotAttached,
        ),
    ];
    for (kinds, presents, mapped, reveal, reason) in cases {
        let mut viewer = presenting(kinds);
        if !presents {
            viewer.frame(json!({"kind": "presenter.refused"}));
        }
        let (left, _) = local_panes(&viewer);
        let pane = if mapped { Some(left) } else { Some(Uuid::new_v4()) };
        forward(&mut viewer, "/repo/README.md", pane, reveal);
        assert!(viewer.sent("link.click").is_empty(), "{reason:?}");
        assert_eq!(hud_message(&mut viewer), Some(reason.message("origin")), "{reason:?}");
        assert_eq!(viewer.node()["hud"]["hideAfter"], json!(5.0), "{reason:?}");
        assert!(viewer.fixture.core.take_status_sounds().is_empty(), "{reason:?}");
    }
}

/// Upstream's `testARefusalBeepsAndLeavesAHudAlreadyUp` and `testARefusalBeepsUnderAZoomedTerminalOrACover`: with another HUD up or a zoomed terminal
/// the page would be hidden, so the click is refused with the alert sound and the HUD stays as it was.
#[test]
fn a_click_under_a_hud_or_a_zoom_plays_the_alert_and_keeps_the_hud() {
    let mut viewer = presenting(&with_link_click());
    let (left, _) = local_panes(&viewer);
    open_hud(&mut viewer.fixture, viewer.session, "agent running");
    forward(&mut viewer, "/repo/README.md", Some(left), false);
    assert!(viewer.sent("link.click").is_empty());
    assert_eq!(viewer.fixture.core.take_status_sounds(), ["default"]);
    assert_eq!(hud_message(&mut viewer).as_deref(), Some("agent running"));

    let mut viewer = presenting(&with_link_click());
    let (left, _) = local_panes(&viewer);
    let (window, session) = (viewer.fixture.window, viewer.session);
    viewer.fixture.core.set_zoom_target(
        window,
        Some(crate::zoom::SurfaceId {
            session,
            surface: crate::zoom::ZoomSurface::Primary,
        }),
    );
    forward(&mut viewer, "/repo/README.md", Some(left), false);
    assert!(viewer.sent("link.click").is_empty());
    assert_eq!(viewer.fixture.core.take_status_sounds(), ["default"]);
    assert_eq!(hud_message(&mut viewer), None);
}

/// Upstream's `testAnOriginRefusalShowsAPanelThatHidesItself`, `testAClickRightAfterARefusalStillTravels`
/// and `testAnOriginHudReplacesTheRefusalPanelButNotALaterLocalHud`: the origin's refusal shows its reason, worded here from
/// the session's host, an unknown reason read as `notOpened`; a click after it still travels, and a HUD the origin
/// mirrors replaces the panel.
#[test]
fn the_origins_refusal_shows_a_panel_that_gives_way() {
    let mut viewer = presenting(&with_link_click());
    let (left, _) = local_panes(&viewer);
    viewer.frame(json!({"kind": "link.refused", "link": {"reason": "notFound"}}));
    assert_eq!(hud_message(&mut viewer), Some("no such file on origin".to_owned()));
    viewer.frame(json!({"kind": "link.refused", "link": {"reason": "someday"}}));
    assert_eq!(
        hud_message(&mut viewer),
        Some("origin could not open the file here".to_owned())
    );
    assert!(viewer.fixture.core.take_status_sounds().is_empty());
    forward(&mut viewer, "/repo/README.md", Some(left), false);
    assert_eq!(viewer.sent("link.click").len(), 1, "a refusal panel hides no page");
    let hud = PresentationHud {
        spec: HudSpec::new("from the origin"),
        pane: None,
        generation: 1,
        remaining: None,
    };
    viewer.frame(json!({"kind": "hud", "hud": serde_json::to_value(hud).unwrap()}));
    assert_eq!(hud_message(&mut viewer), Some("from the origin".to_owned()));
}

/// Upstream's `testAHiddenLocalPageRefusesTheClickAndStays`: this machine's own hidden page would sit over the page,
/// so the click is refused with the alert sound and sends nothing, and so is the origin's refusal.
#[test]
fn a_click_over_an_own_hidden_page_plays_the_alert() {
    let mut viewer = presenting(&with_link_click());
    let session = viewer.session;
    let store = viewer.fixture.store_mut().unwrap();
    let source = crate::html::HtmlSource::Url(url::Url::parse("https://example.com/").unwrap());
    let page = crate::html::HtmlOverlay::new(source, crate::html::HtmlPageFlags::default());
    assert!(store.open_html_overlay(session, None, page, None, None).is_ok());
    assert!(
        store
            .apply_overlay_visibility(session, None, crate::model::OverlayVisibilityMode::Hide)
            .is_ok()
    );
    viewer.fixture.core.commit();
    let (left, _) = local_panes(&viewer);
    forward(&mut viewer, "/repo/README.md", Some(left), false);
    assert!(viewer.sent("link.click").is_empty());
    viewer.frame(json!({"kind": "link.refused", "link": {"reason": "notFound"}}));
    assert_eq!(viewer.fixture.core.take_status_sounds(), ["default", "default"]);
    assert_eq!(hud_message(&mut viewer), None);
}

/// Upstream's `aLocalSessionHasNothingToForward`: a session with no origin sends nothing and, with no host to name,
/// plays the alert.
#[test]
fn a_local_session_has_nothing_to_forward() {
    let mut fixture = super::Fixture::new(1);
    let session = fixture.sessions[0];
    let click = RemoteLinkClick {
        raw: "/tmp/a.md".to_owned(),
        session,
        pane: None,
        mode: LinkOpenMode::Overlay,
    };
    assert_eq!(
        fixture.core.link_click_request(&click, true, false),
        Err(RemoteLinkRefusal::NotPresenter)
    );
    fixture.core.forward_remote_link_click(&click, false);
    assert_eq!(fixture.core.take_status_sounds(), ["default"]);
}

/// Upstream's `testAHiddenReplicaPageLetsTheClickTravel`: a hidden page the origin handed here is the one a forwarded
/// page replaces, so the click still travels.
#[test]
fn a_hidden_replica_page_lets_the_click_travel() {
    let mut viewer = presenting(&with_link_click());
    let session = viewer.session;
    let store = viewer.fixture.store_mut().unwrap();
    let source = crate::html::HtmlSource::Markdown {
        path: "/tmp/remote.md".to_owned(),
    };
    let mut replica = crate::html::HtmlOverlay::new(source, crate::html::HtmlPageFlags::default());
    replica.replica_of = Some(Uuid::new_v4());
    assert!(store.open_html_overlay(session, None, replica, None, None).is_ok());
    assert!(
        store
            .apply_overlay_visibility(session, None, crate::model::OverlayVisibilityMode::Hide)
            .is_ok()
    );
    viewer.fixture.core.commit();
    let (left, _) = local_panes(&viewer);
    forward(&mut viewer, "/repo/README.md", Some(left), false);
    assert_eq!(viewer.sent("link.click").len(), 1);
    assert!(viewer.fixture.core.take_status_sounds().is_empty());
}

/// The second half of upstream's `testAnOriginHudReplacesTheRefusalPanelButNotALaterLocalHud`: a HUD this machine
/// opens after the refusal panel is no refusal panel, so the origin's next HUD waits behind it.
#[test]
fn an_origin_hud_leaves_a_local_hud_opened_after_the_refusal() {
    let mut viewer = presenting(&with_link_click());
    viewer.frame(json!({"kind": "link.refused", "link": {"reason": "notFound"}}));
    assert_eq!(hud_message(&mut viewer), Some("no such file on origin".to_owned()));
    open_hud(&mut viewer.fixture, viewer.session, "agent");
    let hud = PresentationHud {
        spec: HudSpec::new("again"),
        pane: None,
        generation: 2,
        remaining: None,
    };
    viewer.frame(json!({"kind": "hud", "hud": serde_json::to_value(hud).unwrap()}));
    assert_eq!(hud_message(&mut viewer), Some("agent".to_owned()));
}

/// The cover half of upstream's `testARefusalBeepsUnderAZoomedTerminalOrACover`: a shown program overlay would sit
/// over the page, so the origin's refusal plays the alert sound and opens no panel; a shown page the origin handed
/// here would too, so a click under it sends nothing and plays the alert sound.
#[test]
fn a_refusal_under_a_shown_cover_plays_the_alert() {
    let mut viewer = presenting(&with_link_click());
    let session = viewer.session;
    let program = crate::model::OverlayProgram {
        command: "/bin/cat".to_owned(),
        ..crate::model::OverlayProgram::default()
    };
    assert!(viewer.fixture.store_mut().unwrap().open_overlay(session, program));
    viewer.fixture.core.commit();
    viewer.frame(json!({"kind": "link.refused", "link": {"reason": "notOpened"}}));
    assert_eq!(viewer.fixture.core.take_status_sounds(), ["default"]);
    assert_eq!(hud_message(&mut viewer), None);

    let mut viewer = presenting(&with_link_click());
    let session = viewer.session;
    let source = crate::html::HtmlSource::Markdown {
        path: "/tmp/remote.md".to_owned(),
    };
    let mut replica = crate::html::HtmlOverlay::new(source, crate::html::HtmlPageFlags::default());
    replica.replica_of = Some(Uuid::new_v4());
    let store = viewer.fixture.store_mut().unwrap();
    assert!(store.open_html_overlay(session, None, replica, None, None).is_ok());
    viewer.fixture.core.commit();
    let (left, _) = local_panes(&viewer);
    forward(&mut viewer, "/repo/README.md", Some(left), false);
    assert!(viewer.sent("link.click").is_empty());
    assert_eq!(viewer.fixture.core.take_status_sounds(), ["default"]);
    assert_eq!(hud_message(&mut viewer), None);
}
