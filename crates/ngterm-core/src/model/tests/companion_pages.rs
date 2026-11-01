//! Page companions, ported from upstream's `AppStoreCompanionsTests` (the six page cases) and the ask and dashboard
//! halves of a page row: a companion whose page is its session-wide overlay, full size, with no shell under it.

use uuid::Uuid;

use super::overlay_visibility::{page, program};
use super::support::{add, ids};
use crate::dashboard::ResolvedDashboardTarget;
use crate::model::{
    AppStore, AskAlignment, AskStyle, CompanionOptions, CompanionRefusal, OverlayPane, OverlayVisibilityMode,
    PageCompanionOptions, PendingAsk, SessionContent,
};
use crate::pane_env::uuid_string;

const HOME: &str = "/home/u";

/// Upstream's `CompanionGroup`: `work` holds `parent` with its companions `first` and `second`, then `after`.
struct Group {
    store: AppStore,
    work: Uuid,
    parent: Uuid,
    first: Uuid,
    second: Uuid,
    after: Uuid,
}

impl Group {
    fn new() -> Self {
        let mut store = AppStore::new();
        let work = store.add_workspace("work");
        let parent = add(&mut store, work, "/parent");
        let after = add(&mut store, work, "/after");
        let companion = |store: &mut AppStore| {
            store
                .add_companion(parent, CompanionOptions::default(), HOME)
                .expect("the companion is created")
        };
        let first = companion(&mut store);
        let second = companion(&mut store);
        Self {
            store,
            work,
            parent,
            first,
            second,
            after,
        }
    }

    /// A page companion of `target` showing `path`, in the background.
    fn page(&mut self, target: Uuid, path: &str) -> Uuid {
        let options = PageCompanionOptions {
            select: false,
            ..PageCompanionOptions::default()
        };
        self.store
            .add_page_companion(target, page(path), options, HOME)
            .expect("the page companion is created")
    }
}

#[test]
fn a_page_companion_shows_its_page_full_size_under_the_parent() {
    let mut g = Group::new();
    let overlay = page("/tmp/a/page.html");
    let page_id = overlay.id;
    let options = PageCompanionOptions {
        name: Some("docs".to_owned()),
        ..PageCompanionOptions::default()
    };

    // a companion target means its parent
    let id = g.store.add_page_companion(g.first, overlay, options, HOME).unwrap();

    let row = g.store.session(id).unwrap();
    assert_eq!(row.content(), SessionContent::Page);
    assert_eq!(row.parent_id(), Some(g.parent));
    assert_eq!(ids(&g.store, g.work), [g.parent, g.first, g.second, id, g.after]);
    assert_eq!(row.overlay.page().map(|page| page.id), Some(page_id));
    assert!(row.overlay.page().unwrap().row, "the page is its row");
    assert_eq!(row.overlay.size_percent, None);
    assert!(row.locally_managed_pane_identities().is_empty());
    assert!(row.hideable_overlay_slots(None).is_empty());
    assert!(!row.renders_overlay_pane(OverlayPane::Left) && !row.renders_overlay_pane(OverlayPane::Right));
    assert_eq!(g.store.selected_session_id(), Some(id));
}

#[test]
fn a_page_companion_is_named_for_its_page_and_reports_its_parents_cwd() {
    let mut g = Group::new();
    let id = g.page(g.parent, "/tmp/a/report.html");

    assert_eq!(g.store.session(id).unwrap().display_name(), "report.html");
    g.store.rename_session(id, "notes");
    assert_eq!(g.store.session(id).unwrap().display_name(), "notes");
    let tree = crate::projection::control_tree(&g.store, &super::companions::tree_context());
    let node = tree
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.sessions)
        .find(|node| node.id == uuid_string(id))
        .unwrap();
    assert_eq!(node.cwd, g.store.session(g.parent).unwrap().effective_cwd());
    assert_eq!(node.content.as_deref(), Some("page"));
}

#[test]
fn a_page_companion_refuses_everything_that_would_cover_float_or_hide_its_page() {
    let mut g = Group::new();
    let id = g.page(g.parent, "/tmp/p.html");

    assert!(!g.store.open_overlay(id, program("top")));
    assert!(
        g.store
            .open_pane_overlay(id, OverlayPane::Left, program("top"))
            .is_err()
    );
    assert!(!g.store.resize_overlay(id, Some(50)));
    assert!(
        g.store
            .apply_overlay_visibility(id, None, OverlayVisibilityMode::Hide)
            .is_err()
    );
    assert!(
        g.store
            .apply_overlay_visibility(id, None, OverlayVisibilityMode::Show)
            .is_ok()
    );
    let row = g.store.session(id).unwrap();
    assert!(!row.overlay_hidden(None));
    assert!(row.overlay.page().is_some());
    // a full-size resize leaves it as it is
    assert!(g.store.resize_overlay(id, None));
}

#[test]
fn a_page_companion_is_never_persisted_or_archived() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let id = store
        .add_page_companion(parent, page("/tmp/p.html"), PageCompanionOptions::default(), HOME)
        .unwrap();

    let snapshot = store.snapshot();
    assert!(
        !snapshot
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.sessions)
            .any(|session| session.id == id)
    );
    store.close_session(parent);
    let items = store.recent_closed_items();
    let companions = items[0].session.as_ref().unwrap().companions.clone();
    assert!(companions.is_none_or(|companions| companions.is_empty()));
}

#[test]
fn closing_the_page_closes_its_companion() {
    let mut g = Group::new();
    let id = g.page(g.parent, "/tmp/p.html");

    assert!(g.store.close_overlay(id));

    assert!(g.store.session(id).is_none());
    assert!(g.store.session(g.parent).is_some());
    // a page closed alone leaves no Open Recent entry
    assert!(g.store.recent_closed_items().is_empty());
}

#[test]
fn a_page_companion_has_no_dashboard_cell() {
    let mut g = Group::new();
    let id = g.page(g.parent, "/tmp/p.html");

    let cells = g.store.dashboard_pane_cells(&[
        ResolvedDashboardTarget {
            session: id,
            pane: None,
        },
        ResolvedDashboardTarget {
            session: g.parent,
            pane: None,
        },
    ]);

    assert_eq!(cells.iter().map(|cell| cell.session).collect::<Vec<_>>(), [g.parent]);
}

#[test]
fn a_page_companion_takes_no_terminal_ask() {
    let mut g = Group::new();
    let id = g.page(g.parent, "/tmp/p.html");
    let ask = PendingAsk {
        id: "a".to_owned(),
        title: "Pick".to_owned(),
        message: None,
        buttons: Vec::new(),
        default_id: None,
        destructive_id: None,
        style: AskStyle::Terminal,
        align: AskAlignment::Right,
        width: None,
        anchor: None,
        form: None,
        cancel_id: None,
    };

    assert!(!g.store.open_session_ask(id, ask.clone(), None));
    assert!(g.store.open_session_ask(g.parent, ask, None));
}

#[test]
fn a_page_the_slot_refuses_leaves_no_row() {
    let mut g = Group::new();
    let overlay = page("/tmp/p.html");
    let missing = g
        .store
        .add_page_companion(Uuid::new_v4(), overlay, PageCompanionOptions::default(), HOME);
    assert_eq!(missing, Err(CompanionRefusal::NoSuchSession));
    assert_eq!(ids(&g.store, g.work), [g.parent, g.first, g.second, g.after]);
}
