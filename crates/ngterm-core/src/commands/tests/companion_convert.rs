//! `session.companion.convert` and the GUI's Convert to Companion on the fakes: upstream's
//! `ControlDispatcherCompanionsTests.companionConvertRoutesTargetAndWindow`, `ControlServerCompanionsTests`' two
//! convert cases, `AppActionsCompanionsTests`' two and `WorkspaceSidebarCompanionsTests`' row menu run.

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Command, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, UNKNOWN, args, failure, wire};
use crate::app_actions::{ActionEffect, ActionOrigin};
use crate::hud::HudSpec;
use crate::keymap::BuiltinAction;
use crate::model::{CompanionOptions, NewSession};
use crate::sidebar_actions::SidebarAction;

fn convert(fixture: &mut Fixture, target: &str, args: Args) -> Response {
    fixture.session(SessionCmd::CompanionConvert, Some(target), args)
}

fn parent_of(fixture: &Fixture, id: Uuid) -> Option<Uuid> {
    fixture.store().session(id).and_then(crate::model::Session::parent_id)
}

fn remote_row(fixture: &mut Fixture) -> Uuid {
    let workspace = fixture.workspace;
    let spec = NewSession {
        remote_host: Some("far".to_owned()),
        select: false,
        ..NewSession::at_cwd("/r")
    };
    fixture.store_mut().unwrap().add_session(workspace, spec).unwrap()
}

/// Upstream's `testConvertNestsTheTargetUnderAFreshParentSelectsItAndAnswersWithBoth`.
#[test]
fn convert_nests_the_target_under_a_fresh_parent_selects_it_and_answers_with_both() {
    let mut fixture = Fixture::new(1);
    let (task, other) = (fixture.sessions[0], fixture.sessions[1]);
    fixture.store_mut().unwrap().select_session(Some(other));
    fixture.core.clear_pane_focus_requests();
    let window = wire(fixture.window);

    let response = convert(&mut fixture, &wire(task), args(|a| a.window = Some(window)));

    assert!(response.ok, "{response:?}");
    let result = response.result.unwrap();
    assert_eq!(result["id"], json!(wire(task)));
    let fresh = result["parent"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .unwrap();
    assert_eq!(fixture.order(fixture.workspace), [fresh, task, other]);
    assert_eq!(parent_of(&fixture, task), Some(fresh));
    assert_eq!(parent_of(&fixture, fresh), None);
    assert_eq!(fixture.selected(), Some(task));
    assert!(
        fixture.core.pane_focus_requested(task),
        "its terminal takes the keyboard"
    );
    let tree = fixture.send(Command::Tree, None, Args::default());
    let sessions = tree.result.unwrap()["tree"]["workspaces"][0]["sessions"].clone();
    let node = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == json!(wire(task)))
        .cloned()
        .unwrap();
    assert_eq!(node["parent"], json!(wire(fresh)));
}

/// Upstream's `testConvertRefusesACompanionAndARemoteRowWithTheirReasons`: a remote row is refused, never forwarded.
#[test]
fn convert_refuses_a_companion_a_remote_row_and_an_unknown_target() {
    let mut fixture = Fixture::new(0);
    let parent = fixture.sessions[0];
    let child = fixture
        .store_mut()
        .unwrap()
        .add_companion(parent, CompanionOptions::default(), "/home/u")
        .unwrap();
    let remote = remote_row(&mut fixture);
    let before = fixture.order(fixture.workspace);

    assert_eq!(
        convert(&mut fixture, &wire(child), Args::default()),
        failure("already a companion")
    );
    assert_eq!(
        convert(&mut fixture, &wire(remote), Args::default()),
        failure("convert it on the Mac it runs on")
    );
    assert_eq!(
        convert(&mut fixture, UNKNOWN, Args::default()),
        failure(&format!("no such session: {UNKNOWN}"))
    );
    assert_eq!(fixture.order(fixture.workspace), before);
    assert_eq!(parent_of(&fixture, child), Some(parent));
    assert_eq!(parent_of(&fixture, remote), None);
}

/// Upstream's `testConvertToCompanionNestsTheSessionUnderAFreshParentAndKeepsItSelected`, through the keymap action.
#[test]
fn the_keymap_action_nests_the_active_session_and_keeps_it_selected() {
    let mut fixture = Fixture::new(0);
    let task = fixture.sessions[0];
    let origin = ActionOrigin {
        window: fixture.window,
        quick: false,
    };

    assert_eq!(
        fixture.core.perform_builtin(origin, BuiltinAction::ConvertCompanion),
        ActionEffect::Done
    );

    let fresh = parent_of(&fixture, task).expect("a fresh parent");
    assert_eq!(parent_of(&fixture, fresh), None);
    assert_eq!(fixture.selected(), Some(task));
    assert!(fixture.core.pane_focus_requested(task));
    assert_eq!(
        fixture.store().session(task).unwrap().overlay.effective_hud_spec(),
        None
    );
}

/// Upstream's `testARefusedConvertShowsTheReasonInATimedHudOverTheSession`, from the row menu.
#[test]
fn a_refused_convert_shows_the_reason_in_a_timed_hud_over_the_session() {
    let mut fixture = Fixture::new(0);
    let bodies = tempfile::tempdir().unwrap();
    // the panel is a HUD, which needs the painter
    fixture.core.set_hud_body_dir(bodies.path().to_path_buf());
    let parent = fixture.sessions[0];
    let child = fixture
        .store_mut()
        .unwrap()
        .add_companion(parent, CompanionOptions::default(), "/home/u")
        .unwrap();
    let window = fixture.window;

    assert!(
        fixture
            .core
            .sidebar_action(window, SidebarAction::ConvertToCompanion(child))
    );

    let hud = fixture
        .store()
        .session(child)
        .and_then(|session| session.overlay.effective_hud_spec());
    let expected = HudSpec {
        hide_after: Some(5.0),
        ..HudSpec::new("Convert to Companion: already a companion")
    };
    assert_eq!(hud, Some(expected));
    assert_eq!(parent_of(&fixture, child), Some(parent));
}

/// Upstream's `testRowMenuConvertToCompanionNestsTheRowUnderAFreshParent`: a background row converts and takes the
/// selection.
#[test]
fn the_row_menus_convert_nests_the_row_under_a_fresh_parent() {
    let mut fixture = Fixture::new(1);
    let (selected, row) = (fixture.sessions[0], fixture.sessions[1]);
    fixture.store_mut().unwrap().select_session(Some(selected));
    let window = fixture.window;

    assert!(
        fixture
            .core
            .sidebar_action(window, SidebarAction::ConvertToCompanion(row))
    );

    let fresh = parent_of(&fixture, row).expect("a fresh parent");
    assert_eq!(fixture.order(fixture.workspace), [selected, fresh, row]);
    assert_eq!(fixture.selected(), Some(row));
    assert!(fixture.core.pane_focus_requested(row));
}
