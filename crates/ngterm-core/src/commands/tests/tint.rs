//! `session.tint`, `workspace.tint` and the creation tint of `session.new`/`workspace.new` (upstream's
//! `ControlDispatcherSessionMetadataTests` tint cases, `ControlServerTintTests` and `ControlAPITintUITests`).

use ngterm_proto::command::{SessionCmd, WorkspaceCmd};
use ngterm_proto::{Args, Command, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, UNKNOWN, answered_id, args, failure, wire};
use crate::model::{SidebarTint, TintColor, TintParts};

/// One way to fill a request's arguments.
type ArgsEdit = fn(&mut Args);

fn set(color: &str, on: Option<&[&str]>) -> Args {
    args(|a| {
        a.mode = Some("set".into());
        a.tint = Some(color.into());
        a.tint_on = on.map(|parts| parts.iter().map(|part| (*part).to_owned()).collect());
    })
}

fn clear() -> Args {
    args(|a| a.mode = Some("clear".into()))
}

fn session_tint(fixture: &Fixture, id: Uuid) -> Option<SidebarTint> {
    fixture.store().session(id).and_then(|session| session.tint)
}

fn workspace_tint(fixture: &Fixture, id: Uuid) -> Option<SidebarTint> {
    fixture.store().workspace(id).and_then(|workspace| workspace.tint)
}

fn tree(fixture: &mut Fixture) -> Value {
    fixture
        .send(Command::Tree, None, Args::default())
        .result
        .expect("a tree")["tree"]
        .clone()
}

fn workspace_node(fixture: &mut Fixture, id: Uuid) -> Value {
    tree(fixture)["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| node["id"] == json!(wire(id)))
        .cloned()
        .expect("the workspace's node")
}

fn session_node(fixture: &mut Fixture, id: Uuid) -> Value {
    tree(fixture)["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|workspace| workspace["sessions"].as_array().cloned().unwrap_or_default())
        .find(|node| node["id"] == json!(wire(id)))
        .expect("the session's node")
}

/// `testSessionTintSetsKeepsPlacementOnRecolorAndClears`, read back from `tree` as in `ControlAPITintUITests`.
#[test]
fn a_session_tint_sets_keeps_its_placement_on_a_recolor_and_clears() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    let target = wire(id);

    let placed = fixture.session(SessionCmd::Tint, Some(&target), set("green", Some(&["name"])));
    assert_eq!(answered_id(&placed), Some(id));
    assert_eq!(
        session_tint(&fixture, id),
        Some(SidebarTint::new(TintColor::Green, TintParts::NAME))
    );
    fixture.session(SessionCmd::Tint, Some(&target), set("Red", None));
    assert_eq!(
        session_tint(&fixture, id),
        Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
    );
    fixture.session(SessionCmd::Tint, None, set("green", Some(&["icon", "name"])));
    let node = session_node(&mut fixture, id);
    assert_eq!(
        (&node["tint"], &node["tintOn"]),
        (&json!("green"), &json!(["icon", "name"]))
    );

    let cleared = fixture.session(SessionCmd::Tint, None, clear());
    assert_eq!(answered_id(&cleared), Some(id), "the active session by default");
    let node = session_node(&mut fixture, id);
    assert!(node.get("tint").is_none() && node.get("tintOn").is_none(), "{node}");
}

/// `testSessionTintReportsAnUnknownTarget`, and a refused call checked before the target resolves and changing nothing.
#[test]
fn a_refused_tint_changes_nothing_and_is_checked_before_the_target() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    assert!(fixture.session(SessionCmd::Tint, None, set("green", None)).ok);
    assert_eq!(
        fixture.session(SessionCmd::Tint, Some(UNKNOWN), set("red", None)),
        failure(&format!("no such session: {UNKNOWN}"))
    );
    let contradictory = args(|a| {
        a.mode = Some("clear".into());
        a.tint = Some("red".into());
    });
    assert_eq!(
        fixture.session(SessionCmd::Tint, Some(UNKNOWN), contradictory),
        failure("session.tint clear takes no color or placement")
    );
    assert_eq!(
        fixture.send(Command::Workspace(WorkspaceCmd::Tint), Some(UNKNOWN), set("teal", None)),
        failure("invalid tint: teal (red|orange|yellow|green|blue|purple|gray)")
    );
    assert_eq!(
        fixture.send(Command::Workspace(WorkspaceCmd::Tint), None, Args::default()),
        failure("invalid tint mode:  (set|clear)")
    );
    assert_eq!(session_tint(&fixture, id), Some(SidebarTint::on_icon(TintColor::Green)));
}

/// `testWorkspaceTintColorsOnlyTheWorkspace`.
#[test]
fn a_workspace_tint_colors_only_the_workspace_row() {
    let mut fixture = Fixture::new(0);
    let workspace = fixture.workspace;
    let response = fixture.send(
        Command::Workspace(WorkspaceCmd::Tint),
        Some(&wire(workspace)),
        set("blue", Some(&["icon", "name"])),
    );
    assert_eq!(answered_id(&response), Some(workspace));
    assert_eq!(
        workspace_tint(&fixture, workspace),
        Some(SidebarTint::new(TintColor::Blue, TintParts::BOTH))
    );
    assert_eq!(session_tint(&fixture, fixture.sessions[0]), None);
    let node = workspace_node(&mut fixture, workspace);
    assert_eq!(
        (&node["tint"], &node["tintOn"]),
        (&json!("blue"), &json!(["icon", "name"]))
    );
    assert!(node["sessions"][0].get("tint").is_none());

    fixture.send(Command::Workspace(WorkspaceCmd::Tint), None, clear());
    assert_eq!(
        workspace_tint(&fixture, workspace),
        None,
        "the active workspace by default"
    );
}

fn new_session(fixture: &mut Fixture, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.session(SessionCmd::New, None, args(edit))
}

/// `testCreateWorkspaceAndSessionCarryTheirCreationTint`, `testEverySessionPlacementPathCarriesTheCreationTint`: by
/// workspace id, by name, by anchor; a `--create-workspace` workspace does not take the session's tint.
#[test]
fn every_placement_path_carries_the_creation_tint() {
    let mut fixture = Fixture::new(0);
    let prod = fixture.send(
        Command::Workspace(WorkspaceCmd::New),
        None,
        args(|a| {
            a.name = Some("prod".into());
            a.tint = Some("red".into());
            a.tint_on = Some(vec!["name".into()]);
        }),
    );
    let prod = answered_id(&prod).unwrap();
    assert_eq!(
        workspace_tint(&fixture, prod),
        Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
    );

    let by_id = answered_id(&new_session(&mut fixture, |a| {
        a.workspace = Some(wire(prod));
        a.tint = Some("orange".into());
    }))
    .unwrap();
    let by_anchor = answered_id(&new_session(&mut fixture, |a| {
        a.after = Some(wire(by_id));
        a.tint = Some("gray".into());
        a.tint_on = Some(vec!["name".into()]);
    }))
    .unwrap();
    let by_name = answered_id(&new_session(&mut fixture, |a| {
        a.workspace_name = Some("staging".into());
        a.create_workspace = Some(true);
        a.tint = Some("blue".into());
    }))
    .unwrap();

    let node = session_node(&mut fixture, by_id);
    assert_eq!((&node["tint"], &node["tintOn"]), (&json!("orange"), &json!(["icon"])));
    assert_eq!(
        session_tint(&fixture, by_anchor),
        Some(SidebarTint::new(TintColor::Gray, TintParts::NAME))
    );
    assert_eq!(
        session_tint(&fixture, by_name),
        Some(SidebarTint::on_icon(TintColor::Blue))
    );
    let staging = fixture.store().workspace_for_session(by_name).unwrap().id();
    assert_eq!(
        workspace_tint(&fixture, staging),
        None,
        "the created workspace stays untinted"
    );

    // a duplicate starts untinted
    let copy = answered_id(&fixture.session(SessionCmd::Duplicate, Some(&wire(by_id)), Args::default())).unwrap();
    assert_eq!(session_tint(&fixture, copy), None);
}

/// `creationRefusesABadTintBeforeCreatingAnything`.
#[test]
fn a_bad_creation_tint_is_refused_before_anything_is_created() {
    let mut fixture = Fixture::new(0);
    let sessions = fixture.store().sessions().count();
    let workspaces = fixture.store().workspaces().len();
    let cases: [(ArgsEdit, &str); 3] = [
        (|a| a.tint_on = Some(vec!["icon".into()]), "--tint-on requires --tint"),
        (
            |a| a.tint = Some("teal".into()),
            "invalid tint: teal (red|orange|yellow|green|blue|purple|gray)",
        ),
        (
            |a| {
                a.tint = Some("red".into());
                a.tint_on = Some(vec!["both".into()]);
            },
            "invalid tint placement: both (icon,name)",
        ),
    ];
    for (edit, error) in cases {
        let session = new_session(&mut fixture, |a| {
            edit(a);
            a.workspace_name = Some("new".into());
            a.create_workspace = Some(true);
        });
        assert_eq!(session, failure(error));
        let workspace = fixture.send(Command::Workspace(WorkspaceCmd::New), None, args(edit));
        assert_eq!(workspace, failure(error));
    }
    assert_eq!(fixture.store().sessions().count(), sessions);
    assert_eq!(fixture.store().workspaces().len(), workspaces);
}

/// `ControlAPITintUITests`: an unchanged tint arms no `tree.changed`, a change does.
#[test]
fn only_a_changed_tint_invalidates_the_tree() {
    let mut fixture = Fixture::new(0);
    let _ = fixture.core.commit();
    fixture.session(SessionCmd::Tint, None, set("red", None));
    assert_eq!(fixture.core.commit().tree_changed.len(), 1);
    fixture.session(SessionCmd::Tint, None, set("red", Some(&["icon"])));
    assert!(fixture.core.commit().tree_changed.is_empty());
}
