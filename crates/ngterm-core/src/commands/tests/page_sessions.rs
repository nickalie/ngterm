//! Page sessions over the control socket (upstream's `ControlDispatcherCompanionsTests` and
//! `ControlServerCompanionPageTests` `session.page` cases): a top-level page row after its target's group, its
//! refusals, the terminal commands it refuses, and its page coming back pending across a relaunch.

use ngterm_proto::PickItem;
use ngterm_proto::command::{AgentCmd, PickCmd, SessionCmd};
use ngterm_proto::{Args, Command, Request, Response};
use serde_json::json;
use uuid::Uuid;

use super::overlay_page::fixture_with_host;
use super::page_restore::{html_overlays, launch, outcome};
use super::{Fixture, answered_id, args, failure, send, short_tempdir, wire};
use crate::app_actions::{ActionEffect, FrontendAction};
use crate::commands::companion_page::refusal;
use crate::model::{CompanionOptions, NewSession, SessionContent};

const FULL_SIZE: &str = "a page session is full size: it takes no --pane, --size-percent, --width or --height";

fn page_session(fixture: &mut Fixture, target: Uuid, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.session(SessionCmd::Page, Some(&wire(target)), args(edit))
}

fn html() -> String {
    crate::html::os_path("/tmp/p.html")
}

/// Upstream's `pageSessionOpensATopLevelRowAfterTheTargetsGroup`: `{id, pageID}`, no parent, in the background.
#[test]
fn a_page_session_opens_a_top_level_row_after_its_targets_group() {
    let (mut fixture, _) = fixture_with_host(1);
    let (target, after) = (fixture.sessions[0], fixture.sessions[1]);
    let companion = fixture
        .store_mut()
        .unwrap()
        .add_companion(target, CompanionOptions::default(), "/home/u")
        .unwrap();
    fixture.store_mut().unwrap().select_session(Some(after));

    let response = page_session(&mut fixture, target, |a| {
        a.html = Some(html());
        a.color = Some("#112233".into());
    });

    let id = answered_id(&response).expect("an id");
    let result = response.result.clone().unwrap();
    assert!(result.get("parent").is_none(), "{result}");
    assert_eq!(fixture.order(fixture.workspace), [target, companion, id, after]);
    let row = fixture.store().session(id).unwrap();
    assert_eq!(row.content(), SessionContent::Page);
    assert_eq!(row.parent_id(), None);
    assert_eq!(
        row.overlay.page().map(|page| json!(wire(page.id))),
        Some(result["pageID"].clone())
    );
    assert_eq!(row.overlay.background_color.as_deref(), Some("#112233"));
    assert_eq!(
        fixture.selected(),
        Some(after),
        "a page session opens without switching"
    );
    let pending = fixture.session(
        SessionCmd::OverlayResult,
        None,
        args(|a| a.page = result["pageID"].as_str().map(Into::into)),
    );
    assert_eq!(pending.result.unwrap()["pageOutcome"]["outcome"], json!("pending"));
    // it holds companions like any session
    let held = fixture.session(SessionCmd::CompanionNew, Some(&wire(id)), Args::default());
    let held = answered_id(&held).expect("a companion");
    assert_eq!(fixture.store().session(held).unwrap().parent_id(), Some(id));
}

#[test]
fn follow_selects_the_page_session() {
    let (mut fixture, _) = fixture_with_host(1);
    let target = fixture.sessions[0];
    let response = page_session(&mut fixture, target, |a| {
        a.url = Some("https://example.com/".into());
        a.follow = Some(true);
    });
    assert_eq!(fixture.selected(), answered_id(&response));
}

/// Upstream's `pageSessionRefusals`: the page row checks under a page session's name; nothing is created.
#[test]
fn a_page_session_refuses_a_program_anything_but_full_size_and_an_unknown_target() {
    let (mut fixture, log) = fixture_with_host(0);
    let target = fixture.sessions[0];
    let cases: [(Args, String); 6] = [
        (
            args(|a| (a.pane, a.html) = (Some("right".into()), Some(html()))),
            FULL_SIZE.to_owned(),
        ),
        (
            args(|a| (a.size_percent, a.html) = (Some(50), Some(html()))),
            FULL_SIZE.to_owned(),
        ),
        (
            args(|a| (a.width, a.html) = (Some(300), Some(html()))),
            FULL_SIZE.to_owned(),
        ),
        (
            args(|a| (a.height, a.html) = (Some(300), Some(html()))),
            FULL_SIZE.to_owned(),
        ),
        (
            args(|a| a.command = Some("top".into())),
            "a page session needs --html or --url".to_owned(),
        ),
        (
            args(|a| (a.color, a.html) = (Some("red".into()), Some(html()))),
            "invalid color: red (#rrggbb)".to_owned(),
        ),
    ];
    for (input, error) in cases {
        let response = fixture.session(SessionCmd::Page, Some(&wire(target)), input);
        assert_eq!(response, failure(&error));
    }
    let unknown = Uuid::new_v4();
    assert_eq!(
        page_session(&mut fixture, unknown, |a| a.html = Some(html())),
        failure(&format!("no such session: {}", wire(unknown)))
    );
    log.borrow_mut().store_failure = Some("the saved store is locked".to_owned());
    assert_eq!(
        page_session(&mut fixture, target, |a| (a.url, a.persistent) =
            (Some("https://example.com/".into()), Some(true))),
        failure("session.page: the saved store is locked")
    );
    log.borrow_mut().unavailable = Some("no GPU".to_owned());
    assert_eq!(
        page_session(&mut fixture, target, |a| a.html = Some(html())),
        failure("session.page: html overlays are not available here: no GPU")
    );
    assert_eq!(fixture.order(fixture.workspace), [target]);
}

#[test]
fn a_build_without_web_views_refuses_a_page_session() {
    let mut fixture = Fixture::new(0);
    let target = fixture.sessions[0];
    assert_eq!(
        page_session(&mut fixture, target, |a| a.html = Some(html())),
        failure("session.page: html overlays are not available in this build")
    );
}

/// Upstream's `aRemoteAnchorPlacesALocalPageSessionBesideIt`, through the dispatcher: the page is this machine's own.
#[test]
fn a_remote_target_only_places_a_local_page_session() {
    let (mut fixture, _) = fixture_with_host(0);
    let workspace = fixture.workspace;
    let remote = fixture
        .store_mut()
        .unwrap()
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("buildbox".into()),
                ..NewSession::at_cwd("/srv")
            },
        )
        .unwrap();
    let response = page_session(&mut fixture, remote, |a| a.url = Some("https://example.com/".into()));
    let id = answered_id(&response).expect("a page session");
    assert_eq!(*fixture.order(workspace).last().unwrap(), id);
    assert_eq!(fixture.store().session(id).unwrap().remote_host(), None);
}

/// Upstream's `PageSessionRefusal` on a top-level page row: the same refusal a page companion meets, by its new name.
#[test]
fn a_page_session_refuses_terminal_commands() {
    let (mut fixture, _) = fixture_with_host(0);
    let target = fixture.sessions[0];
    let page = answered_id(&page_session(&mut fixture, target, |a| a.html = Some(html()))).unwrap();
    for (cmd, input) in [
        (
            Command::Session(SessionCmd::Type),
            args(|a| a.text = Some("ls\n".into())),
        ),
        (Command::Session(SessionCmd::Split), Args::default()),
        (
            Command::Session(SessionCmd::OverlayOpen),
            args(|a| a.command = Some("top".into())),
        ),
        (Command::Agent(AgentCmd::Report), Args::default()),
    ] {
        let request = Request {
            cmd,
            target: Some(wire(page)),
            args: Some(input),
        };
        let expected = format!("{cmd}: not available on a page session");
        assert_eq!(
            refusal(&fixture.core, &request).map(|error| error.to_string()),
            Some(expected.clone())
        );
        assert_eq!(fixture.core.dispatch(&request), failure(&expected));
    }
    assert!(
        fixture
            .session(
                SessionCmd::Rename,
                Some(&wire(page)),
                args(|a| a.name = Some("notes".into()))
            )
            .ok
    );
}

/// A page session and its companions across a relaunch: the page opens again under a fresh id, its outcome pending.
#[test]
fn a_page_session_comes_back_with_its_companions_after_a_relaunch() {
    let state = short_tempdir();
    let site = tempfile::tempdir().unwrap();
    let file = site.path().join("page.html");
    std::fs::write(&file, "<p>hi</p>").unwrap();
    let (mut core, _, anchor) = launch(state.path());
    let created = send(
        &mut core,
        Command::Session(SessionCmd::Page),
        Some(&wire(anchor)),
        args(|a| a.html = Some(file.to_string_lossy().into_owned())),
    );
    let page = answered_id(&created).expect("a page session");
    let companion = answered_id(&send(
        &mut core,
        Command::Session(SessionCmd::CompanionNew),
        Some(&wire(page)),
        Args::default(),
    ))
    .expect("a companion");
    let before = created.result.unwrap()["pageID"].clone();
    assert!(core.save_all());
    drop(core);

    let (mut core, window, _) = launch(state.path());
    let store = core.library().store(window).unwrap();
    let order: Vec<Uuid> = store.workspaces()[0]
        .sessions
        .iter()
        .map(crate::model::Session::id)
        .collect();
    assert_eq!(order, [anchor, page, companion]);
    assert_eq!(store.session(page).unwrap().content(), SessionContent::Page);
    assert_eq!(store.session(companion).unwrap().parent_id(), Some(page));
    let pages = html_overlays(&mut core, page);
    assert_eq!(pages[0]["file"], json!(file.to_string_lossy()));
    assert_ne!(pages[0]["id"], before, "a restore mints a fresh page id");
    let after = Uuid::parse_str(pages[0]["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        outcome(&mut core, after).result.unwrap()["pageOutcome"]["outcome"],
        json!("pending")
    );
}

// MARK: - the page's own close (upstream's `HtmlOverlayRegistryTests` and `ControlServerCompanionPageTests` close cases)

/// A page session of the first session: its row id and page id.
fn page_row(fixture: &mut Fixture) -> (Uuid, Uuid) {
    let target = fixture.sessions[0];
    let response = page_session(fixture, target, |a| a.html = Some(html()));
    let page = response.result.as_ref().unwrap()["pageID"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    (answered_id(&response).unwrap(), page)
}

/// A page session's `window.close()` takes the undoable session close once: the request an undo's remount repeats does
/// nothing, and the row stays.
#[test]
fn a_page_sessions_own_close_soft_closes_it_once() {
    let (mut fixture, _) = fixture_with_host(0);
    let (id, page) = page_row(&mut fixture);
    let companion = fixture
        .store_mut()
        .unwrap()
        .add_companion(id, CompanionOptions::default(), "/home/u")
        .unwrap();

    assert_eq!(
        fixture.core.close_html_overlay_from_page(page),
        Some(ActionEffect::Done)
    );
    let _ = fixture.core.commit();
    assert!(fixture.store().session(id).is_none());
    assert!(
        fixture.store().pending_close_session(id).is_some(),
        "the close is undoable"
    );
    assert!(
        fixture.store().pending_close_session(companion).is_some(),
        "with its companions"
    );

    assert!(fixture.store_mut().unwrap().undo_pending_close(None));
    let _ = fixture.core.commit();
    assert_eq!(fixture.core.close_html_overlay_from_page(page), None);
    let _ = fixture.core.commit();
    assert!(fixture.store().session(id).is_some(), "an undo does not close it again");
}

/// With the confirmation on, the page's own close asks once: a cancelled answer leaves the row and asks no more.
#[test]
fn a_page_sessions_own_close_asks_for_the_confirmation_once() {
    let (mut fixture, _) = fixture_with_host(0);
    let (id, page) = page_row(&mut fixture);
    fixture.core.update_settings(|s| s.confirm_close_session = Some(true));
    let window = fixture.window;

    assert_eq!(
        fixture.core.close_html_overlay_from_page(page),
        Some(ActionEffect::Frontend(FrontendAction::ConfirmCloseSession {
            window,
            session: id
        }))
    );
    assert_eq!(
        fixture.core.close_html_overlay_from_page(page),
        None,
        "a cancel does not ask again"
    );
    assert!(fixture.store().session(id).is_some());
}

/// The page's own close waits while a picker is pending in its window and goes through once the picker ends.
#[test]
fn a_page_sessions_own_close_waits_for_a_pending_pick() {
    let (mut fixture, _) = fixture_with_host(0);
    let (id, page) = page_row(&mut fixture);
    let item = PickItem {
        id: "a".to_owned(),
        label: "Alpha".to_owned(),
        ..PickItem::default()
    };
    let opened = fixture.send(Command::Pick(PickCmd::Open), None, args(|a| a.items = Some(vec![item])));
    let pick = opened.result.as_ref().unwrap()["id"].as_str().unwrap().to_owned();

    assert_eq!(fixture.core.close_html_overlay_from_page(page), None);
    assert!(fixture.store().session(id).is_some(), "it waits for the picker");

    assert!(
        fixture
            .send(Command::Pick(PickCmd::Cancel), Some(&pick), Args::default())
            .ok
    );
    assert_eq!(
        fixture.core.close_html_overlay_from_page(page),
        Some(ActionEffect::Done)
    );
    let _ = fixture.core.commit();
    assert!(fixture.store().pending_close_session(id).is_some());
}

/// A page over a terminal closes only itself, as before; the control close of a page session never asks.
#[test]
fn a_page_over_a_terminal_closes_only_itself_and_the_control_close_never_asks() {
    let (mut fixture, _) = fixture_with_host(0);
    let terminal = fixture.sessions[0];
    let opened = fixture.session(
        SessionCmd::OverlayOpen,
        Some(&wire(terminal)),
        args(|a| a.html = Some(html())),
    );
    let page: Uuid = opened.result.as_ref().unwrap()["pageID"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        fixture.core.close_html_overlay_from_page(page),
        Some(ActionEffect::Done)
    );
    assert!(
        fixture
            .store()
            .session(terminal)
            .is_some_and(|session| session.overlay.page().is_none())
    );

    let (id, _) = page_row(&mut fixture);
    fixture.core.update_settings(|s| s.confirm_close_session = Some(true));
    assert!(
        fixture
            .session(SessionCmd::OverlayClose, Some(&wire(id)), Args::default())
            .ok
    );
    let _ = fixture.core.commit();
    assert!(fixture.store().session(id).is_none());
}
