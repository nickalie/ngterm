//! Page companions over the control socket (upstream's `ControlDispatcherCompanionsTests` page cases,
//! `ControlServerCompanionsTests` page cases and `ControlServerCompanionPageTests`): `session.companion.page`, the
//! refusal of the terminal commands, and `session.companion.page.attach` with the claims and channels it hands out.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ngterm_proto::command::{AskCmd, SessionCmd, SurfaceCmd};
use ngterm_proto::{Args, Command, Request, Response};
use serde_json::json;
use uuid::Uuid;

use super::overlay_page::fixture_with_host;
use super::page_serve::FakeChannel;
use super::{Fixture, answered_id, args, failure, wire};
use crate::app_actions::{ActionEffect, ActionOrigin, FrontendAction};
use crate::commands::companion_page::refusal;
use crate::keymap::BuiltinAction;
use crate::model::{NewSession, SessionContent};
use crate::remote::page_serve::{MAX_PENDING_LINES, PageChannel, PageServeFrame, PageServeStream};
use crate::remote::{CompanionPageServing, PAGE_CLAIM_WINDOW};

const FULL_SIZE: &str = "a page companion is full size: it takes no --pane, --size-percent, --width or --height";

/// A folder holding `index.html`, canonical so the page's grant compares as the channel resolves it.
struct Docs {
    _dir: tempfile::TempDir,
    root: String,
}

impl Docs {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap().join("docs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "<p>index</p>").unwrap();
        Self {
            _dir: dir,
            root: root.to_str().unwrap().to_owned(),
        }
    }

    fn index(&self) -> String {
        std::path::Path::new(&self.root)
            .join("index.html")
            .to_str()
            .unwrap()
            .to_owned()
    }
}

fn companion_page(fixture: &mut Fixture, target: Uuid, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.session(SessionCmd::CompanionPage, Some(&wire(target)), args(edit))
}

/// A page companion of `parent` showing `/tmp/p.html`: its row id.
fn file_page(fixture: &mut Fixture, parent: Uuid) -> Uuid {
    let response = companion_page(fixture, parent, |a| a.html = Some(crate::html::os_path("/tmp/p.html")));
    answered_id(&response).expect("a page companion")
}

fn result_field(response: &Response, key: &str) -> serde_json::Value {
    response
        .result
        .as_ref()
        .map_or(serde_json::Value::Null, |result| result[key].clone())
}

/// Upstream's `companionPageRoutesAFullSizePage` and `testAPageCompanionIsCreatedWithoutTouchingTheParentsOverlaySlot`.
#[test]
fn a_page_companion_opens_full_size_without_touching_the_parents_slot() {
    let (mut fixture, _) = fixture_with_host(1);
    let (parent, other) = (fixture.sessions[0], fixture.sessions[1]);
    let response = companion_page(&mut fixture, parent, |a| {
        a.html = Some(crate::html::os_path("/tmp/a/page.html"));
        a.color = Some("#112233".into());
        a.javascript = Some(true);
    });
    let id = answered_id(&response).expect("an id");
    assert_eq!(result_field(&response, "parent"), json!(wire(parent)));
    let page_id = result_field(&response, "pageID");
    let store = fixture.store();
    let row = store.session(id).unwrap();
    assert_eq!(row.content(), SessionContent::Page);
    assert_eq!(
        row.overlay.page().map(|page| json!(wire(page.id))),
        Some(page_id.clone())
    );
    assert!(row.overlay.page().unwrap().flags.javascript);
    assert_eq!(row.overlay.background_color.as_deref(), Some("#112233"));
    assert_eq!(row.overlay.size_percent, None);
    assert!(!store.session(parent).unwrap().overlay.active);
    assert_eq!(fixture.order(fixture.workspace), [parent, id, other]);
    assert_eq!(
        fixture.selected(),
        Some(other),
        "a page companion opens without switching"
    );
    let outcome = fixture.session(
        SessionCmd::OverlayResult,
        None,
        args(|a| a.page = page_id.as_str().map(Into::into)),
    );
    assert_eq!(result_field(&outcome, "pageOutcome")["outcome"], json!("pending"));
}

#[test]
fn follow_selects_the_page_companion() {
    let (mut fixture, _) = fixture_with_host(1);
    let parent = fixture.sessions[0];
    let response = companion_page(&mut fixture, parent, |a| {
        a.url = Some("https://example.com/".into());
        a.follow = Some(true);
    });
    assert_eq!(fixture.selected(), answered_id(&response));
}

/// Upstream's `companionPageRefusesAProgramOrAnythingButFullSize`, with the page checks it shares with
/// `session.overlay.open` and a malformed color; nothing is created.
#[test]
fn a_page_companion_refuses_a_program_or_anything_but_full_size() {
    let (mut fixture, _) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    let html = || Some(crate::html::os_path("/tmp/p.html"));
    let cases: [(Args, &str); 7] = [
        (args(|a| (a.pane, a.html) = (Some("right".into()), html())), FULL_SIZE),
        (args(|a| (a.size_percent, a.html) = (Some(50), html())), FULL_SIZE),
        (args(|a| (a.width, a.html) = (Some(300), html())), FULL_SIZE),
        (args(|a| (a.height, a.html) = (Some(300), html())), FULL_SIZE),
        (
            args(|a| a.command = Some("top".into())),
            "a page companion needs --html or --url",
        ),
        (Args::default(), "session.overlay.open requires a command"),
        (
            args(|a| (a.color, a.html) = (Some("red".into()), html())),
            "invalid color: red (#rrggbb)",
        ),
    ];
    for (request, error) in cases {
        let response = fixture.session(SessionCmd::CompanionPage, Some(&wire(parent)), request.clone());
        assert_eq!(response, failure(error), "{request:?}");
    }
    assert_eq!(fixture.order(fixture.workspace), [parent]);
}

#[test]
fn a_page_companion_needs_web_views_and_a_local_parent() {
    let mut bare = Fixture::new(0);
    let parent = bare.sessions[0];
    assert_eq!(
        companion_page(&mut bare, parent, |a| a.html =
            Some(crate::html::os_path("/tmp/p.html"))),
        failure("session.companion.page: html overlays are not available in this build")
    );
    let (mut fixture, log) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    log.borrow_mut().store_failure = Some("the saved store is locked".to_owned());
    let persistent = companion_page(&mut fixture, parent, |a| {
        a.url = Some("https://example.com/".into());
        a.persistent = Some(true);
    });
    assert_eq!(persistent, failure("session.companion.page: the saved store is locked"));
    log.borrow_mut().unavailable = Some("no GPU".to_owned());
    assert_eq!(
        companion_page(&mut fixture, parent, |a| a.html =
            Some(crate::html::os_path("/tmp/p.html"))),
        failure("session.companion.page: html overlays are not available here: no GPU")
    );
    log.borrow_mut().unavailable = None;
    let workspace = fixture.workspace;
    let remote = fixture
        .store_mut()
        .and_then(|store| {
            let spec = NewSession {
                remote_host: Some("buildbox".into()),
                ..NewSession::at_cwd("/tmp")
            };
            store.add_session(workspace, spec)
        })
        .unwrap();
    let loopback = companion_page(&mut fixture, remote, |a| a.url = Some("http://localhost:3000/".into()));
    assert_eq!(
        loopback,
        failure("a loopback page cannot be a companion of a remote session")
    );
    let elsewhere = companion_page(&mut fixture, remote, |a| a.url = Some("https://example.com/".into()));
    assert_eq!(elsewhere, failure("the remote session is not attached"));
}

#[test]
fn closing_the_page_closes_its_row_and_dismisses_the_page() {
    let (mut fixture, _) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    let response = companion_page(&mut fixture, parent, |a| {
        a.html = Some(crate::html::os_path("/tmp/p.html"));
    });
    let id = answered_id(&response).unwrap();
    let page_id = result_field(&response, "pageID");
    // a page companion is renamed like any row
    assert!(
        fixture
            .session(
                SessionCmd::Rename,
                Some(&wire(id)),
                args(|a| a.name = Some("notes".into()))
            )
            .ok
    );
    assert_eq!(fixture.store().session(id).unwrap().display_name(), "notes");

    assert!(
        fixture
            .session(SessionCmd::OverlayClose, Some(&wire(id)), Args::default())
            .ok
    );
    fixture.core.commit();

    assert!(fixture.store().session(id).is_none());
    assert!(fixture.store().session(parent).is_some());
    let outcome = fixture.session(
        SessionCmd::OverlayResult,
        None,
        args(|a| a.page = page_id.as_str().map(Into::into)),
    );
    assert_eq!(result_field(&outcome, "pageOutcome")["outcome"], json!("dismissed"));
}

/// The row is gone into an undoable close and its page stays live through the grace.
fn assert_soft_closed(fixture: &Fixture, id: Uuid, page: Uuid) {
    assert!(fixture.store().session(id).is_none());
    assert!(fixture.store().pending_close_session(id).is_some());
    assert!(fixture.core.library().live_html_overlays().contains(&page));
}

/// A page companion of the first session: its row id and page id.
fn page_row(fixture: &mut Fixture) -> (Uuid, Uuid) {
    let parent = fixture.sessions[0];
    let response = companion_page(fixture, parent, |a| a.html = Some(crate::html::os_path("/tmp/p.html")));
    let page = result_field(&response, "pageID").as_str().unwrap().parse().unwrap();
    (answered_id(&response).unwrap(), page)
}

/// Upstream's strip `onClose` on a page row: `AppActions.closeSession`, the confirmation then the undoable close.
#[test]
fn the_strips_close_soft_closes_a_page_row_after_the_confirmation() {
    let (mut fixture, _) = fixture_with_host(0);
    let (id, page) = page_row(&mut fixture);
    assert_eq!(
        fixture.core.close_html_overlay_from_strip(page),
        Some(ActionEffect::Done)
    );
    fixture.core.commit();
    assert_soft_closed(&fixture, id, page);

    let (mut fixture, _) = fixture_with_host(0);
    let (id, page) = page_row(&mut fixture);
    fixture.core.update_settings(|s| s.confirm_close_session = Some(true));
    let window = fixture.window;
    assert_eq!(
        fixture.core.close_html_overlay_from_strip(page),
        Some(ActionEffect::Frontend(FrontendAction::ConfirmCloseSession {
            window,
            session: id
        }))
    );
    assert!(
        fixture.store().session(id).is_some(),
        "nothing closes before the answer"
    );
}

#[test]
fn close_session_on_a_selected_page_row_soft_closes_it_after_the_confirmation() {
    let origin = |fixture: &Fixture| ActionOrigin {
        window: fixture.window,
        quick: false,
    };
    let (mut fixture, _) = fixture_with_host(0);
    let (id, page) = page_row(&mut fixture);
    assert!(fixture.session(SessionCmd::Select, Some(&wire(id)), Args::default()).ok);
    let at = origin(&fixture);
    assert_eq!(
        fixture.core.perform_builtin(at, BuiltinAction::CloseSession),
        ActionEffect::Done
    );
    fixture.core.commit();
    assert_soft_closed(&fixture, id, page);

    let (mut fixture, _) = fixture_with_host(0);
    let (id, _) = page_row(&mut fixture);
    assert!(fixture.session(SessionCmd::Select, Some(&wire(id)), Args::default()).ok);
    fixture.core.update_settings(|s| s.confirm_close_session = Some(true));
    let at = origin(&fixture);
    assert_eq!(
        fixture.core.perform_builtin(at, BuiltinAction::CloseSession),
        ActionEffect::Frontend(FrontendAction::ConfirmCloseSession {
            window: at.window,
            session: id
        })
    );
}

// MARK: - the terminal commands

fn request(cmd: Command, target: Option<&str>, edit: impl FnOnce(&mut Args)) -> Request {
    Request {
        cmd,
        target: target.map(str::to_owned),
        args: Some(args(edit)),
    }
}

/// Upstream's `testAPageCompanionRefusesTerminalCommandsAndKeepsOrdinaryOnes`.
#[test]
fn a_page_companion_refuses_terminal_commands_and_keeps_ordinary_ones() {
    let (mut fixture, _) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    let page = file_page(&mut fixture, parent);
    let id = wire(page);
    let surface = format!("surface:{id}:left");
    let session = |cmd| Command::Session(cmd);
    for refused in [
        request(session(SessionCmd::Type), Some(&id), |a| a.text = Some("ls".into())),
        request(session(SessionCmd::Split), Some(&id), |_| {}),
        request(session(SessionCmd::HudOpen), Some(&id), |_| {}),
        request(Command::Surface(SurfaceCmd::Zoom), Some(&surface), |_| {}),
        request(Command::Surface(SurfaceCmd::Cursor), Some(&id), |_| {}),
        request(session(SessionCmd::OverlayResize), Some(&id), |a| {
            a.size_percent = Some(50);
        }),
        request(Command::Ask(AskCmd::Open), Some(&id), |_| {}),
    ] {
        let error = refusal(&fixture.core, &refused).map(|error| Response::from(error).error.unwrap_or_default());
        assert_eq!(
            error,
            Some(format!("{}: not available on a page companion", refused.cmd))
        );
        // the socket meets the same refusal before the command runs
        assert_eq!(fixture.core.dispatch(&refused).error, error);
    }
    for allowed in [
        request(session(SessionCmd::Rename), Some(&id), |_| {}),
        request(session(SessionCmd::Close), Some(&id), |_| {}),
        request(session(SessionCmd::Type), Some(&wire(parent)), |_| {}),
        request(session(SessionCmd::OverlayResize), Some(&id), |a| a.full = Some(true)),
        request(Command::Ask(AskCmd::Open), Some(&id), |a| a.style = Some("gui".into())),
        request(Command::Surface(SurfaceCmd::Zoom), Some(&surface), |a| {
            a.mode = Some("hide".into());
        }),
    ] {
        assert!(refusal(&fixture.core, &allowed).is_none(), "{}", allowed.cmd);
    }
}

/// Upstream's `testWithAPageSelectedActiveZoomAndCursorStillReachTheZoomTarget`.
#[test]
fn with_a_page_selected_active_zoom_and_cursor_still_reach_the_zoom_target() {
    let (mut fixture, _) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    let page = file_page(&mut fixture, parent);
    fixture.store_mut().unwrap().select_session(Some(page));
    for mode in ["hide", "toggle", "show"] {
        let zoom = request(Command::Surface(SurfaceCmd::Zoom), None, |a| a.mode = Some(mode.into()));
        assert!(refusal(&fixture.core, &zoom).is_none(), "{mode}");
    }
    let cursor = request(Command::Surface(SurfaceCmd::Cursor), Some("active"), |_| {});
    assert!(refusal(&fixture.core, &cursor).is_none());
    let other = format!("surface:{}:left", wire(parent));
    let zoom = request(Command::Surface(SurfaceCmd::Zoom), Some(&other), |_| {});
    assert!(refusal(&fixture.core, &zoom).is_none());
}

// MARK: - attachments for another machine

/// Attaches page companion `page`: the reply's `companionPage`.
fn attach(fixture: &mut Fixture, page: Uuid) -> serde_json::Value {
    let response = fixture.session(SessionCmd::CompanionPageAttach, Some(&wire(page)), Args::default());
    assert!(response.ok, "{response:?}");
    result_field(&response, "companionPage")
}

fn token(attachment: &serde_json::Value) -> String {
    attachment["token"].as_str().unwrap().to_owned()
}

fn claim(fixture: &mut Fixture, token: &str) -> Response {
    let request = Request {
        target: Some(token.to_owned()),
        ..Request::new(Command::Session(SessionCmd::OverlayPageServe))
    };
    fixture.core.dispatch(&request)
}

/// Claims `token` and adopts a fake connection for the claim, as the frontend does after an `ok`.
fn adopt(fixture: &mut Fixture, token: &str) -> (Arc<FakeChannel>, Arc<PageServeStream>) {
    let claimed = claim(fixture, token);
    let claim = result_field(&claimed, "id");
    let fake = FakeChannel::new(MAX_PENDING_LINES);
    let shared: Arc<dyn PageChannel> = fake.clone();
    let stream = fixture
        .core
        .adopt_page_serve_stream(claim.as_str().unwrap(), shared)
        .expect("the attachment is adopted");
    (fake, stream)
}

/// A page companion of the fixture's first session showing `docs`' index, granted the folder.
fn served(docs: &Docs) -> (Fixture, Uuid, Uuid) {
    let (mut fixture, _) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    let response = companion_page(&mut fixture, parent, |a| {
        a.html = Some(docs.index());
        a.cwd = Some(docs.root.clone());
    });
    (fixture, parent, answered_id(&response).unwrap())
}

/// Upstream's `testOnlyAPageCompanionIsHandedOutAndALoopbackPageIsNot`, and what an attachment hands out.
#[test]
fn only_a_page_companion_is_handed_out_and_a_loopback_page_is_not() {
    let docs = Docs::new();
    let (mut fixture, parent, page) = served(&docs);
    let response = fixture.session(SessionCmd::CompanionPageAttach, Some(&wire(parent)), Args::default());
    assert_eq!(response, failure("not a page companion"));

    let reply = fixture.session(SessionCmd::CompanionPageAttach, Some(&wire(page)), Args::default());
    assert_eq!(result_field(&reply, "id"), json!(wire(page)));
    assert_eq!(result_field(&reply, "parent"), json!(wire(parent)));
    let attachment = result_field(&reply, "companionPage");
    let row = fixture.store().session(page).unwrap();
    assert_eq!(attachment["page"], json!(wire(row.overlay.page().unwrap().id)));
    assert_eq!(attachment["identity"], json!("index.html"));
    assert_eq!(attachment["source"]["file"]["grantRoot"], json!(docs.root));
    assert_eq!(attachment["follow"], json!(false));
    assert!(attachment.get("pane").is_none() && attachment.get("sizePercent").is_none());

    let loopback = companion_page(&mut fixture, parent, |a| a.url = Some("http://localhost:3000/".into()));
    let loopback = answered_id(&loopback).unwrap();
    let response = fixture.session(SessionCmd::CompanionPageAttach, Some(&wire(loopback)), Args::default());
    assert_eq!(
        response,
        failure("a page on this Mac's loopback cannot be shown on another Mac")
    );
}

/// Upstream's `testEachViewerClaimsItsOwnAttachmentAndOneLeavingEndsNeitherThePageNorTheOther`.
#[test]
fn each_viewer_claims_its_own_attachment_and_one_leaving_ends_neither_the_page_nor_the_other() {
    let docs = Docs::new();
    let (mut fixture, _, page) = served(&docs);
    let (first, second) = (token(&attach(&mut fixture, page)), token(&attach(&mut fixture, page)));

    let (_one, one_stream) = adopt(&mut fixture, &first);
    assert_eq!(
        claim(&mut fixture, &first),
        failure("page not claimable"),
        "a token is claimed once"
    );
    let (two, two_stream) = adopt(&mut fixture, &second);
    assert_eq!(fixture.core.companion_page_streams(), 2);

    fixture.core.page_serve_closed(&one_stream);
    fixture.core.commit();
    assert_eq!(fixture.core.companion_page_streams(), 1);
    assert!(
        fixture.store().session(page).is_some(),
        "a viewer leaving ends nothing here"
    );

    assert!(two_stream.take(&resource_line(1, &docs.index())).is_none());
    assert_eq!(
        two.frame(),
        Some(PageServeFrame::Head {
            id: 1,
            mime_type: "text/html".to_owned()
        })
    );
}

fn resource_line(id: i64, path: &str) -> Vec<u8> {
    let mut line = crate::remote::page_serve::encode(&PageServeFrame::Resource {
        id,
        path: path.to_owned(),
    })
    .unwrap();
    line.pop();
    line
}

/// Upstream's `testAnExpiredOrUnansweredClaimLeavesThePageAlone`: a late claim is refused, and a claim whose page
/// closed before its adoption gets no channel.
#[test]
fn an_expired_or_orphaned_claim_leaves_the_page_alone() {
    let docs = Docs::new();
    let (mut fixture, _, page) = served(&docs);
    let stale = token(&attach(&mut fixture, page));
    let late = Instant::now() + PAGE_CLAIM_WINDOW + Duration::from_secs(1);
    assert_eq!(fixture.core.claim_companion_page(&stale, late), Some(Err(())));
    assert!(fixture.store().session(page).is_some());

    let fresh = token(&attach(&mut fixture, page));
    let claimed = result_field(&claim(&mut fixture, &fresh), "id");
    fixture.store_mut().unwrap().close_session(page);
    fixture.core.commit();
    let fake = FakeChannel::new(MAX_PENDING_LINES);
    let shared: Arc<dyn PageChannel> = fake.clone();
    assert!(
        fixture
            .core
            .adopt_page_serve_stream(claimed.as_str().unwrap(), shared)
            .is_none()
    );
    assert!(fake.closed());
}

/// Upstream's `testClosingThePageHereEndsEveryAttachmentAndEveryUnclaimedToken`.
#[test]
fn closing_the_page_here_ends_every_attachment_and_every_unclaimed_token() {
    let docs = Docs::new();
    let (mut fixture, _, page) = served(&docs);
    let (first, second) = (token(&attach(&mut fixture, page)), token(&attach(&mut fixture, page)));
    let unclaimed = token(&attach(&mut fixture, page));
    let (one, _) = adopt(&mut fixture, &first);
    let (two, _) = adopt(&mut fixture, &second);

    fixture.store_mut().unwrap().close_session(page);
    fixture.core.commit();

    assert_eq!(one.frame(), None, "both channels end");
    assert_eq!(two.frame(), None);
    assert_eq!(fixture.core.companion_page_streams(), 0);
    assert!(!fixture.core.companion_page_token_pending(&unclaimed));
}

/// Upstream's `testASoftClosedPageStillAnswersForItsOwnWindow`.
#[test]
fn a_soft_closed_page_still_answers_for_its_own_window() {
    let docs = Docs::new();
    let (mut fixture, _, page) = served(&docs);
    let page_id = fixture.store().session(page).unwrap().overlay.page().unwrap().id;
    let serving = CompanionPageServing {
        claim: Uuid::new_v4(),
        session: page,
        page: page_id,
    };

    assert!(fixture.store_mut().unwrap().soft_close_session(page));
    fixture.core.commit();

    let place = fixture
        .core
        .companion_page_place(&serving)
        .expect("a soft-closed page still serves");
    assert_eq!(place.window, fixture.window);
    assert_eq!(place.session, page);
}

/// Upstream's `testAUrlAttachmentHasALifetimeChannelAndNoBridge`.
#[test]
fn a_url_attachment_has_a_channel_and_no_bridge() {
    let (mut fixture, _) = fixture_with_host(0);
    let parent = fixture.sessions[0];
    let response = companion_page(&mut fixture, parent, |a| a.url = Some("https://example.com/".into()));
    let page = answered_id(&response).unwrap();
    let attachment = attach(&mut fixture, page);
    assert_eq!(attachment["source"]["url"]["_0"], json!("https://example.com/"));
    let (fake, stream) = adopt(&mut fixture, &token(&attachment));
    assert_eq!(fixture.core.companion_page_streams(), 1);

    let mut line = crate::remote::page_serve::encode(&PageServeFrame::Bridge {
        id: 4,
        request: r#"{"cmd":"tree"}"#.to_owned(),
    })
    .unwrap();
    line.pop();
    let work = stream.take(&line).expect("a bridge call goes to the main loop");
    assert!(fixture.core.page_serve_work(work).is_none());
    assert_eq!(
        fake.frame(),
        Some(PageServeFrame::Failure {
            id: 4,
            error: "a URL page has no bridge".to_owned()
        })
    );
}
