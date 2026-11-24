use uuid::Uuid;

use super::super::fixture::{self, pane, session, session_with, snapshot, workspace};
use super::*;

/// Upstream's `applyingTheChangesInOrderLandsOnTheNextCatalog`.
#[test]
fn applying_the_changes_in_order_lands_on_the_next_catalog() {
    for (name, old, new) in fixture::transitions() {
        let current = snapshot(old);
        let next = snapshot(new);
        let mut applied = current.clone();
        for change in current.changes(&next) {
            applied.apply(&change);
        }
        assert_eq!(applied.workspaces, next.workspaces, "{name}");
    }
}

/// Upstream's `eachKindOfChangeIsNamedForWhatChanged`.
#[test]
fn each_kind_of_change_is_named_for_what_changed() {
    let w = Uuid::new_v4();
    let a = session("a");
    let (left, right) = (pane(), pane());
    let mut split = session_with(Uuid::new_v4(), "s", vec![left.clone(), right]);
    let current = snapshot(vec![workspace(
        w,
        "w",
        vec![a, session_with(split.id(), "s", vec![left])],
    )]);
    split.fields.name = "two".to_owned();
    let added = session("new");
    let next = snapshot(vec![workspace(w, "w2", vec![split, added])]);
    let kinds: Vec<&str> = current.changes(&next).iter().map(CatalogChange::kind).collect();
    assert_eq!(
        kinds,
        [
            "session.removed",
            "workspace.changed",
            "session.added",
            "pane.added",
            "session.changed"
        ]
    );
}

/// Upstream's `anUnchangedCatalogHasNoChanges`.
#[test]
fn an_unchanged_catalog_has_no_changes() {
    let current = snapshot(vec![workspace(Uuid::new_v4(), "w", vec![session("s")])]);
    assert!(current.changes(&current).is_empty());
}

#[test]
fn a_change_naming_something_absent_is_a_no_op() {
    let w = Uuid::new_v4();
    let a = session("a");
    let mut catalog = snapshot(vec![workspace(w, "w", vec![a.clone()])]);
    let before = catalog.clone();
    let nowhere = Uuid::new_v4();
    for change in [
        CatalogChange::WorkspaceChanged {
            id: nowhere,
            name: "x".to_owned(),
        },
        CatalogChange::SessionAdded {
            session: session("b"),
            workspace: nowhere,
            index: 0,
        },
        CatalogChange::SessionMoved {
            id: a.id(),
            workspace: nowhere,
            index: 0,
        },
        CatalogChange::SessionMoved {
            id: nowhere,
            workspace: w,
            index: 0,
        },
        CatalogChange::SessionOrder {
            ids: vec![],
            workspace: nowhere,
        },
        CatalogChange::PaneRemoved {
            identity: nowhere,
            session: nowhere,
        },
    ] {
        catalog.apply(&change);
    }
    assert_eq!(catalog, before);
}

#[test]
fn an_index_out_of_range_lands_at_the_nearest_end() {
    let w = Uuid::new_v4();
    let (a, b, c) = (session("a"), session("b"), session("c"));
    let mut catalog = snapshot(vec![workspace(w, "w", vec![a.clone()])]);
    catalog.apply(&CatalogChange::SessionAdded {
        session: b.clone(),
        workspace: w,
        index: 99,
    });
    catalog.apply(&CatalogChange::SessionAdded {
        session: c.clone(),
        workspace: w,
        index: -3,
    });
    let order: Vec<Uuid> = catalog.workspaces[0].sessions.iter().map(CatalogSession::id).collect();
    assert_eq!(order, [c.id(), a.id(), b.id()]);
    // an unlisted session keeps its place after the listed ones
    catalog.apply(&CatalogChange::SessionOrder {
        ids: vec![b.id()],
        workspace: w,
    });
    let order: Vec<Uuid> = catalog.workspaces[0].sessions.iter().map(CatalogSession::id).collect();
    assert_eq!(order, [b.id(), c.id(), a.id()]);
}
