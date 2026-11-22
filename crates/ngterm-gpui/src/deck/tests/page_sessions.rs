//! Page sessions on a real deck (upstream's `HtmlOverlayViewTests` and `HtmlOverlayRegistryTests` page-row cases): a
//! failed page's own Reload, a saved page that could not be read shown in place of its page, and the page's own
//! `window.close()` taking the session close.

use ngterm_core::app_actions::FrontendAction;
use ngterm_core::html::HtmlLoadState;
use ngterm_core::model::{NewSession, SessionContent};
use ngterm_proto::Request;
use ngterm_proto::command::SessionCmd;

use super::overlay::overlay_harness;
use super::*;

/// The recording engine and the page host on `h`'s core.
fn page_host(h: &mut Harness<'_>) {
    let _web = crate::html_overlay::testing::install();
    let poster = h.state.poster().clone();
    crate::html_overlay::install(&mut h.state.core, h.state_dir.path().to_path_buf(), poster);
}

/// A selected page session after `anchor` showing a chromeless loopback page, which has no strip: its row and page.
fn chromeless_page_session(h: &mut Harness<'_>, anchor: Uuid) -> (Uuid, Uuid) {
    let args = Args {
        url: Some("http://127.0.0.1:2/".to_owned()),
        chromeless: Some(true),
        follow: Some(true),
        ..Args::default()
    };
    let response = h.state.core.dispatch(&Request {
        cmd: Command::Session(SessionCmd::Page),
        target: Some(uuid_string(anchor)),
        args: Some(args),
    });
    assert!(response.ok, "{response:?}");
    let result = response.result.unwrap();
    let id = Uuid::parse_str(result["id"].as_str().unwrap()).unwrap();
    let page = Uuid::parse_str(result["pageID"].as_str().unwrap()).unwrap();
    h.state.settle();
    h.turn();
    (id, page)
}

/// Upstream's failed-page Reload: a chromeless page has no strip, so the failure's own Reload is its retry.
#[gpui_kit::test]
fn a_failed_page_without_a_strip_reloads_from_its_failure(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (mut h, session, _) = overlay_harness(cx);
    page_host(&mut h);
    let (id, page) = chromeless_page_session(&mut h, session);
    h.state
        .core
        .set_html_load_state(page, HtmlLoadState::Failed, Some("Connection refused".to_owned()));
    h.state.settle();
    h.turn();
    assert!(h.cx.debug_bounds("htmlOverlay.error").is_some(), "the failure is drawn");
    assert!(
        h.cx.debug_bounds("htmlOverlay.reload").is_none(),
        "a chromeless page has no strip"
    );
    let reload =
        h.cx.debug_bounds("htmlOverlay.failureReload")
            .expect("the failure offers its Reload");

    h.cx.simulate_click(reload.center(), gpui_kit::Modifiers::default());
    h.turn();

    let shown = h.session(id).overlay.page().unwrap().clone();
    assert_eq!(shown.reload_revision, 1);
    assert_eq!(shown.load_state, HtmlLoadState::Loading);
    assert!(
        h.cx.debug_bounds("htmlOverlay.error").is_none(),
        "the reload replaces the failure"
    );
}

/// A page session whose saved page could not be read shows the failure and its reason where its page would be.
#[gpui_kit::test]
fn an_unreadable_page_session_shows_why_in_place_of_its_page(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (mut h, _, _) = overlay_harness(cx);
    let workspace = h.store().workspaces()[0].id();
    let spec = NewSession {
        content: SessionContent::Page,
        ..NewSession::at_cwd("/tmp")
    };
    let id = h.store().add_session(workspace, spec).unwrap();
    h.store().session_mut(id).unwrap().unreadable_page = Some("the saved page names nothing".to_owned());
    h.state.settle();
    h.turn();
    assert!(h.cx.debug_bounds("htmlOverlay.error").is_some(), "the failure is drawn");
}

/// The page's own `window.close()` closes its page session as Close Session does, confirmed first when asked.
#[gpui_kit::test]
fn a_page_sessions_own_close_takes_the_session_close(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (mut h, session, _) = overlay_harness(cx);
    page_host(&mut h);
    let (id, page) = chromeless_page_session(&mut h, session);
    h.state.page_closed_itself(page, true);
    h.turn();
    assert!(h.store().session(id).is_none(), "the row closed with its page");

    let (id, page) = chromeless_page_session(&mut h, session);
    h.state.core.update_settings(|s| s.confirm_close_session = Some(true));
    let _ = h.state.take_frontend_work();
    h.state.page_closed_itself(page, true);
    let window = h.window;
    let asked = h.state.take_frontend_work().actions;
    assert!(
        asked
            .iter()
            .any(|(action, _)| *action == FrontendAction::ConfirmCloseSession { window, session: id }),
        "{asked:?}"
    );
    assert!(h.store().session(id).is_some(), "nothing closes before the answer");
    h.state.page_closed_itself(page, false);
    assert!(
        h.state.take_frontend_work().actions.is_empty(),
        "an apply's repeat after a cancel does not ask again"
    );
    h.state.page_closed_itself(page, true);
    assert!(
        h.state
            .take_frontend_work()
            .actions
            .iter()
            .any(|(action, _)| *action == FrontendAction::ConfirmCloseSession { window, session: id }),
        "a new window.close() asks again"
    );
}
