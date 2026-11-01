//! `session.overlay.hide|show|toggle` and opening into a hidden slot, ported from upstream's
//! `ControlDispatcherOverlayTests` (the visibility routing and failures), `ControlServerOverlayVisibilityTests` (as
//! commands through the core) and `AppActionsOverlayTests` (the toggle action, Close Session, the Edit items and the
//! close confirmation).

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, UNKNOWN, args, failure, wire};
use crate::app_actions::{ActionEffect, ActionOrigin, FrontendAction, close_confirmation_text};
use crate::html::{HtmlHost, HtmlNavigation};
use crate::keymap::BuiltinAction;
use crate::model::{OverlayPane, OverlayVisibilityFailure, RemoteOverlaySlot};
use crate::palette_catalog::{PaletteCommand, PaletteFacts};

const VISIBILITY: [(SessionCmd, &str); 3] = [
    (SessionCmd::OverlayHide, "hide"),
    (SessionCmd::OverlayShow, "show"),
    (SessionCmd::OverlayToggle, "toggle"),
];

fn overlay(fixture: &mut Fixture, cmd: SessionCmd, id: Uuid, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.session(cmd, Some(&wire(id)), args(edit))
}

fn open(fixture: &mut Fixture, id: Uuid, edit: impl FnOnce(&mut Args)) -> Response {
    overlay(fixture, SessionCmd::OverlayOpen, id, |a| {
        a.command = Some("true".to_owned());
        edit(a);
    })
}

fn on(pane: &'static str) -> impl FnOnce(&mut Args) {
    move |a: &mut Args| a.pane = Some(pane.to_owned())
}

/// The answer a visibility command gives when it leaves the scope `text`.
fn left(id: Uuid, text: &str) -> Response {
    Response::with_result(json!({"id": wire(id), "text": text}))
}

/// Shows the split of session `id`, both panes with a terminal.
fn split(fixture: &mut Fixture, id: Uuid) {
    let store = fixture.store_mut().unwrap();
    store.set_split_visibility(id, true, None);
    let session = store.session_mut(id).unwrap();
    session.surface.replace(Box::new(FakeSurface::new("L")));
    session.split_surface.replace(Box::new(FakeSurface::new("R")));
}

fn hidden_names(fixture: &Fixture, id: Uuid) -> Vec<&'static str> {
    fixture.store().session(id).unwrap().hidden_overlay_names()
}

/// Web views that load nothing, so a page can open.
struct NoopHost;

impl HtmlHost for NoopHost {
    fn web_views_unavailable(&self) -> Option<String> {
        None
    }

    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

// MARK: - the dispatcher

/// Upstream's `visibilityCommandsRouteModePaneTargetAndWindow`: the mode reaches the session, scoped to the pane.
#[test]
fn visibility_commands_route_their_mode_pane_target_and_window() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    split(&mut fixture, id);
    assert!(open(&mut fixture, id, |_| {}).ok);
    assert!(open(&mut fixture, id, on("split")).ok);
    let window = wire(fixture.window);
    let hide = overlay(&mut fixture, SessionCmd::OverlayHide, id, |a| {
        a.window = Some(window.clone());
    });
    assert_eq!(hide, left(id, "hidden"));
    assert_eq!(hidden_names(&fixture, id), ["session", "right"]);
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayShow, id, on("split")),
        left(id, "shown")
    );
    assert_eq!(hidden_names(&fixture, id), ["session"]);
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayToggle, id, on("right")),
        left(id, "hidden")
    );
    assert_eq!(hidden_names(&fixture, id), ["session", "right"]);
    // the session scope while one slot is shown: a toggle hides, its answer naming what it left
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayShow, id, |_| {}),
        left(id, "shown")
    );
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayToggle, id, |_| {}),
        left(id, "hidden")
    );
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayToggle, id, |_| {}),
        left(id, "shown")
    );
}

/// Upstream's `visibilityCommandsRejectAnInvalidPaneBeforeTheHost`: the pane is parsed before the target resolves.
#[test]
fn visibility_commands_reject_an_invalid_pane_before_the_target() {
    let mut fixture = Fixture::new(0);
    for (cmd, _) in VISIBILITY {
        let refused = fixture.session(cmd, Some(UNKNOWN), args(on("scratch")));
        assert_eq!(
            refused,
            failure("session.overlay: --pane must be left or right"),
            "{cmd:?}"
        );
        assert_eq!(
            fixture.session(cmd, Some(UNKNOWN), Args::default()),
            failure(&format!("no such session: {UNKNOWN}")),
            "{cmd:?}"
        );
    }
}

/// Upstream's `visibilityFailuresMapToTheirMessages`.
#[test]
fn visibility_failures_map_to_their_messages() {
    for (failure, message) in [
        (OverlayVisibilityFailure::UnknownSession, "no such session"),
        (OverlayVisibilityFailure::NoOverlay, "no overlay"),
        (OverlayVisibilityFailure::Remote, "overlay is shown on another Mac"),
    ] {
        assert_eq!(failure.message(), message);
    }
}

// MARK: - the control server's cases

#[test]
fn hide_show_and_toggle_report_the_state_they_leave() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    assert!(open(&mut fixture, id, |_| {}).ok);
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayHide, id, |_| {}),
        left(id, "hidden")
    );
    let slot = &fixture.store().session(id).unwrap().overlay;
    assert!(slot.hidden && slot.active);
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayToggle, id, |_| {}),
        left(id, "shown")
    );
    assert!(!fixture.store().session(id).unwrap().overlay.hidden);
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayShow, id, |_| {}),
        left(id, "shown")
    );
}

#[test]
fn visibility_refuses_an_empty_session_and_an_unknown_target() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    for (cmd, _) in VISIBILITY {
        assert_eq!(overlay(&mut fixture, cmd, id, |_| {}), failure("no overlay"), "{cmd:?}");
        assert!(!fixture.session(cmd, Some("nope"), Args::default()).ok);
    }
}

#[test]
fn a_slot_reserved_for_a_viewer_is_shown_on_another_mac() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    let slot = RemoteOverlaySlot {
        job: "job".to_owned(),
        pane: None,
        owner: 1,
        size_percent: None,
        wait: false,
        ended: false,
        page: None,
    };
    assert!(fixture.store_mut().unwrap().reserve_remote_overlay(id, slot));
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayToggle, id, |_| {}),
        failure("overlay is shown on another Mac")
    );
    assert_eq!(
        overlay(&mut fixture, SessionCmd::OverlayHide, id, on("left")),
        failure("no overlay")
    );
}

#[test]
fn opening_into_a_hidden_session_slot_is_refused_by_name() {
    let mut fixture = Fixture::new(0);
    fixture.core.set_html_host(Box::new(NoopHost));
    let id = fixture.sessions[0];
    assert!(open(&mut fixture, id, |_| {}).ok);
    fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(id, None, true)
        .unwrap();
    assert_eq!(open(&mut fixture, id, |_| {}), failure("overlay hidden"));
    let page = overlay(&mut fixture, SessionCmd::OverlayOpen, id, |a| {
        a.html = Some(crate::html::os_path("/tmp/p.html"));
    });
    assert_eq!(page, failure("overlay hidden"));
    assert_eq!(
        fixture.store().session(id).unwrap().overlay.command.as_deref(),
        Some("true")
    );
}

#[test]
fn opening_into_a_hidden_pane_slot_is_refused_by_name_while_the_other_slots_stay_open() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    assert!(open(&mut fixture, id, on("left")).ok);
    let store = fixture.store_mut().unwrap();
    store.set_overlays_hidden(id, Some(OverlayPane::Left), true).unwrap();
    assert_eq!(open(&mut fixture, id, on("left")), failure("overlay hidden"));
    let session_wide = open(&mut fixture, id, |_| {});
    assert!(session_wide.ok, "{session_wide:?}");
}

#[test]
fn a_hud_cannot_open_over_a_hidden_program_but_a_hidden_pane_overlay_does_not_block_it() {
    let mut fixture = Fixture::new(0);
    let dir = tempfile::tempdir().unwrap();
    fixture.core.set_hud_body_dir(dir.path().join("bodies"));
    let id = fixture.sessions[0];
    let hud = |fixture: &mut Fixture| {
        overlay(fixture, SessionCmd::HudOpen, id, |a| {
            a.message = Some("building".into());
        })
    };
    assert!(open(&mut fixture, id, on("left")).ok);
    let store = fixture.store_mut().unwrap();
    store.set_overlays_hidden(id, Some(OverlayPane::Left), true).unwrap();
    let over_pane = hud(&mut fixture);
    assert!(over_pane.ok, "{over_pane:?}");
    assert!(fixture.store_mut().unwrap().close_overlay(id));

    assert!(open(&mut fixture, id, |_| {}).ok);
    fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(id, None, true)
        .unwrap();
    assert_eq!(hud(&mut fixture), failure("overlay hidden"));
    assert!(fixture.store().session(id).unwrap().overlay.hud.is_none());
    // an unknown session is named before the hidden slot
    assert_eq!(
        fixture.session(
            SessionCmd::HudOpen,
            Some(UNKNOWN),
            args(|a| a.message = Some("x".into()))
        ),
        failure(&format!("no such session: {UNKNOWN}"))
    );
}

#[test]
fn tree_reads_back_the_hidden_slots_and_a_hidden_page() {
    let mut fixture = Fixture::new(0);
    fixture.core.set_html_host(Box::new(NoopHost));
    let id = fixture.sessions[0];
    let page = overlay(&mut fixture, SessionCmd::OverlayOpen, id, |a| {
        a.html = Some(crate::html::os_path("/tmp/p.html"));
    });
    let page_id = page.result.unwrap()["pageID"].clone();
    assert!(overlay(&mut fixture, SessionCmd::OverlayHide, id, |_| {}).ok);
    let tree = fixture.send(ngterm_proto::Command::Tree, None, Args::default());
    let node = tree.result.unwrap()["tree"]["workspaces"][0]["sessions"][0].clone();
    assert_eq!(node["hiddenOverlays"], json!(["session"]));
    assert_eq!(node["overlay"], json!(true));
    assert_eq!(node["htmlOverlays"][0]["hidden"], json!(true));
    assert_eq!(node["htmlOverlays"][0]["id"], page_id);
    assert!(overlay(&mut fixture, SessionCmd::OverlayShow, id, |_| {}).ok);
    let tree = fixture.send(ngterm_proto::Command::Tree, None, Args::default());
    let node = tree.result.unwrap()["tree"]["workspaces"][0]["sessions"][0].clone();
    assert!(node.get("hiddenOverlays").is_none());
    assert!(node["htmlOverlays"][0].get("hidden").is_none());
}

// MARK: - the app's actions

fn perform(fixture: &mut Fixture, action: BuiltinAction) -> ActionEffect {
    let origin = ActionOrigin {
        window: fixture.window,
        quick: false,
    };
    let effect = fixture.core.perform_builtin(origin, action);
    let _ = fixture.core.commit();
    effect
}

fn store_open(fixture: &mut Fixture, id: Uuid, pane: Option<OverlayPane>, command: &str) {
    let program = crate::model::OverlayProgram {
        command: command.to_owned(),
        ..crate::model::OverlayProgram::default()
    };
    let store = fixture.store_mut().unwrap();
    match pane {
        None => assert!(store.open_overlay(id, program)),
        Some(pane) => assert_eq!(store.open_pane_overlay(id, pane, program), Ok(())),
    }
}

#[test]
fn command_w_skips_a_hidden_session_overlay_and_closes_the_focused_pane_overlay() {
    let mut fixture = Fixture::new(0);
    let id = fixture.selected().unwrap();
    store_open(&mut fixture, id, None, "less");
    store_open(&mut fixture, id, Some(OverlayPane::Left), "htop");
    fixture.store_mut().unwrap().session_mut(id).unwrap().overlay.hidden = true;
    assert_eq!(perform(&mut fixture, BuiltinAction::CloseSession), ActionEffect::Done);
    let session = fixture.store().session(id).unwrap();
    assert!(
        session.pane_overlay(OverlayPane::Left).is_none(),
        "the visible pane overlay is the one in front"
    );
    assert!(session.overlay.active, "the hidden session overlay stays");
    assert!(session.overlay.hidden);
}

#[test]
fn command_w_with_only_hidden_overlays_closes_the_session() {
    let mut fixture = Fixture::new(1);
    let id = fixture.selected().unwrap();
    store_open(&mut fixture, id, None, "less");
    store_open(&mut fixture, id, Some(OverlayPane::Left), "htop");
    fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(id, None, true)
        .unwrap();
    perform(&mut fixture, BuiltinAction::CloseSession);
    assert!(fixture.store().session(id).is_none());
}

#[test]
fn an_edit_item_refused_by_a_hidden_overlay_rings_the_bell_and_leaves_it_hidden() {
    let mut host = super::keymap::Host::new(0);
    let window = host.fixture.window;
    let id = host.fixture.selected().unwrap();
    store_open(&mut host.fixture, id, Some(OverlayPane::Left), "htop");
    store_open(&mut host.fixture, id, None, "less");
    let store = host.fixture.store_mut().unwrap();
    store.set_overlays_hidden(id, None, true).unwrap();
    assert_eq!(
        host.fixture
            .core
            .perform_menu_command(window, PaletteCommand::EditKeymap),
        ActionEffect::Frontend(FrontendAction::AlertBell(window))
    );
    let session = host.fixture.store().session(id).unwrap();
    assert_eq!(session.overlay.command.as_deref(), Some("less"));
    assert_eq!(session.hidden_overlay_names(), ["session", "left"]);
    // a slot refused for another reason stays silent: the occupant is on screen
    host.fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(id, None, false)
        .unwrap();
    assert_eq!(
        host.fixture
            .core
            .perform_menu_command(window, PaletteCommand::EditHooks),
        ActionEffect::Done
    );
}

/// Upstream's `beepIfHiddenOverlayRefused` reading `sessionSlotShown` (#831): a session-wide program out of sight under
/// a pane overlay opened over it refuses the editor with nothing on screen to say why, so the bell rings too.
#[test]
fn an_edit_item_refused_by_a_cover_under_a_pane_overlay_rings_the_bell() {
    let mut host = super::keymap::Host::new(0);
    let window = host.fixture.window;
    let id = host.fixture.selected().unwrap();
    store_open(&mut host.fixture, id, None, "less");
    store_open(&mut host.fixture, id, Some(OverlayPane::Left), "htop");
    assert_eq!(
        host.fixture.store().session(id).unwrap().hidden_overlay_names(),
        ["session"]
    );
    assert_eq!(
        host.fixture
            .core
            .perform_menu_command(window, PaletteCommand::EditKeymap),
        ActionEffect::Frontend(FrontendAction::AlertBell(window))
    );
    let session = host.fixture.store().session(id).unwrap();
    assert_eq!(session.overlay.command.as_deref(), Some("less"));
    assert!(!session.overlay.hidden, "the user's own bit is untouched");
}

#[test]
fn the_close_confirmation_names_hidden_overlays_only_when_there_are_any() {
    let mut fixture = Fixture::new(1);
    let (first, other) = (fixture.sessions[0], fixture.sessions[1]);
    store_open(&mut fixture, first, None, "less");
    let text = |fixture: &Fixture, ids: &[Uuid], grace: bool| {
        let sessions: Vec<_> = ids.iter().map(|id| fixture.store().session(*id).unwrap()).collect();
        close_confirmation_text(&sessions, grace)
    };
    assert_eq!(
        text(&fixture, &[first], true),
        "The session will close after a short undo window."
    );
    fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(first, None, true)
        .unwrap();
    assert_eq!(
        text(&fixture, &[first], true),
        "The session will close after a short undo window. Its hidden overlays close with it."
    );
    assert_eq!(
        text(&fixture, &[other, first], true),
        "The sessions will close after a short undo window. Their hidden overlays close with them."
    );
    assert_eq!(
        text(&fixture, &[other], false),
        "The session will close immediately and can be reopened from File > Open Recent."
    );
}

#[test]
fn toggle_overlays_flips_the_active_session_and_the_palette_follows_what_is_there() {
    let mut fixture = Fixture::new(0);
    let window = fixture.window;
    let id = fixture.selected().unwrap();
    let context = |fixture: &Fixture| fixture.core.palette_context(window);
    assert!(!context(&fixture).has(PaletteFacts::ACTIVE_SESSION_HAS_OVERLAY));
    assert!(!PaletteCommand::ToggleOverlay.is_enabled(&context(&fixture)));
    perform(&mut fixture, BuiltinAction::ToggleOverlay);
    assert!(hidden_names(&fixture, id).is_empty());

    store_open(&mut fixture, id, None, "less");
    assert!(PaletteCommand::ToggleOverlay.is_enabled(&context(&fixture)));
    perform(&mut fixture, BuiltinAction::ToggleOverlay);
    assert_eq!(hidden_names(&fixture, id), ["session"]);
    fixture.core.perform_menu_command(window, PaletteCommand::ToggleOverlay);
    assert!(hidden_names(&fixture, id).is_empty());
}

#[test]
fn a_hidden_full_overlay_lets_the_split_toggle_through() {
    let mut fixture = Fixture::new(0);
    let id = fixture.selected().unwrap();
    store_open(&mut fixture, id, None, "less");
    perform(&mut fixture, BuiltinAction::ToggleSplit);
    assert!(
        !fixture.store().session(id).unwrap().split.exists,
        "a shown full overlay keeps the split out"
    );
    fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(id, None, true)
        .unwrap();
    perform(&mut fixture, BuiltinAction::ToggleSplit);
    assert!(fixture.store().session(id).unwrap().split.exists);
}
