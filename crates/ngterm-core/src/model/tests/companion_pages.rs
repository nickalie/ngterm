//! Page sessions, ported from upstream's `AppStoreCompanionsTests` (the page and page-row cases) and the ask and dashboard
//! halves of a page row: a companion whose page is its session-wide overlay, full size, with no shell under it.

use uuid::Uuid;

use super::overlay_visibility::{page, program};
use super::support::{add, ids};
use crate::dashboard::ResolvedDashboardTarget;
use crate::html::{HtmlOverlay, HtmlSource, os_path};
use crate::model::{
    AppStore, AskAlignment, AskStyle, CompanionOptions, CompanionRefusal, ConvertRefusal, NewSession, OverlayPane,
    OverlayVisibilityMode, PageCompanionOptions, PendingAsk, Session, SessionContent,
};
use crate::pane_env::uuid_string;
use crate::persist::{PageSlotSnapshot, PageSourceSnapshot, SessionContentSnapshot, SessionSnapshot};

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

/// Upstream's `aPageCompanionIsSavedOnCreationAndArchivedWithItsParent`.
#[test]
fn a_page_companion_is_saved_and_archived_with_its_parent() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = add(&mut store, work, "/parent");
    let id = store
        .add_page_companion(parent, page("/tmp/p.html"), PageCompanionOptions::default(), HOME)
        .unwrap();

    let saved = saved_session(&store, id).expect("the page companion is saved");
    assert_eq!(saved.parent_id, Some(parent));
    assert_eq!(saved_page(&saved), file_entry("/tmp/p.html"));
    store.close_session(parent);
    let items = store.recent_closed_items();
    let companions = items[0].session.as_ref().unwrap().companions.clone().unwrap();
    assert_eq!(
        companions.iter().map(|companion| companion.id).collect::<Vec<_>>(),
        [id]
    );
    assert_eq!(saved_page(&companions[0]), file_entry("/tmp/p.html"));
}

#[test]
fn closing_the_page_closes_its_companion() {
    let mut g = Group::new();
    let id = g.page(g.parent, "/tmp/p.html");

    assert!(g.store.close_overlay(id));

    assert!(g.store.session(id).is_none());
    assert!(g.store.session(g.parent).is_some());
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

// MARK: - page sessions (upstream's `AppStoreCompanionsTests` page-row cases)

/// The session `id` as `store`'s snapshot writes it; `None` when it writes none.
fn saved_session(store: &AppStore, id: Uuid) -> Option<SessionSnapshot> {
    store
        .snapshot()
        .workspaces
        .into_iter()
        .flat_map(|workspace| workspace.sessions)
        .find(|session| session.id == id)
}

/// The page a page session's snapshot writes as its own.
fn saved_page(saved: &SessionSnapshot) -> (PageSlotSnapshot, PageSourceSnapshot) {
    assert_eq!(saved.content, Some(SessionContentSnapshot::Page));
    assert_eq!(saved.pane_identity, None, "a page owns no pane");
    let pages = saved.pages.as_deref().expect("its page");
    assert_eq!(pages.len(), 1);
    (pages[0].slot, pages[0].source.clone())
}

fn file_entry(path: &str) -> (PageSlotSnapshot, PageSourceSnapshot) {
    (
        PageSlotSnapshot::Session,
        PageSourceSnapshot::File {
            path: path.to_owned(),
            grant_root: None,
        },
    )
}

/// A top-level page session of `work` showing `/tmp/root.html`.
fn top_level_page(store: &mut AppStore, work: Uuid) -> Uuid {
    let spec = NewSession {
        content: SessionContent::Page,
        ..NewSession::at_cwd("/page")
    };
    let id = store.add_session(work, spec).unwrap();
    let overlay = HtmlOverlay {
        row: true,
        ..page(&os_path("/tmp/root.html"))
    };
    assert!(store.open_html_overlay(id, None, overlay, None, None).is_ok());
    id
}

#[test]
fn a_top_level_page_groups_terminal_and_page_companions() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let root = top_level_page(&mut store, work);
    let after = add(&mut store, work, "/after");

    let terminal = store.add_companion(root, CompanionOptions::default(), HOME).unwrap();
    let nested = store
        .add_page_companion(
            root,
            page(&os_path("/tmp/n.html")),
            PageCompanionOptions::default(),
            HOME,
        )
        .unwrap();

    assert_eq!(ids(&store, work), [root, terminal, nested, after]);
    assert_eq!(store.session(root).unwrap().parent_id(), None);
    assert_eq!(store.session(terminal).unwrap().content(), SessionContent::Terminal);
    assert_eq!(store.session(nested).unwrap().content(), SessionContent::Page);
    let companions: Vec<Uuid> = store.companions(root).into_iter().map(Session::id).collect();
    assert_eq!(companions, [terminal, nested]);
    // a companion of a page companion lands under the page root
    let sibling = store.add_companion(nested, CompanionOptions::default(), HOME).unwrap();
    assert_eq!(store.session(sibling).unwrap().parent_id(), Some(root));
    assert_eq!(ids(&store, work), [root, terminal, nested, sibling, after]);
}

#[test]
fn convert_nests_a_top_level_page_and_its_companions_under_a_fresh_terminal() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let before = add(&mut store, work, "/before");
    let root = top_level_page(&mut store, work);
    let companion = store
        .add_companion(
            root,
            CompanionOptions {
                select: false,
                ..CompanionOptions::default()
            },
            HOME,
        )
        .unwrap();
    let shown = store.session(root).unwrap().overlay.page().unwrap().id;

    let fresh = store.convert_to_companion(root).unwrap();

    assert_eq!(ids(&store, work), [before, fresh, root, companion]);
    let fresh_row = store.session(fresh).unwrap();
    assert_eq!(fresh_row.content(), SessionContent::Terminal);
    assert_eq!(fresh_row.parent_id(), None);
    assert_eq!(fresh_row.effective_cwd(), "/page");
    let page_row = store.session(root).unwrap();
    assert_eq!(page_row.parent_id(), Some(fresh));
    assert_eq!(page_row.overlay.page().map(|page| page.id), Some(shown));
    assert_eq!(store.session(companion).unwrap().parent_id(), Some(fresh));
    assert_eq!(store.selected_session_id(), Some(root));
}

/// Upstream's `aPageSessionLandsAfterTheAnchorsWholeGroupInItsCwd`, for each member of the group as the anchor.
#[test]
fn a_page_session_lands_after_the_anchors_whole_group_in_its_cwd() {
    for anchor in 0..3 {
        let mut g = Group::new();
        let overlay = page("/tmp/p.html");
        let shown = overlay.id;
        let target = [g.parent, g.first, g.second][anchor];
        let expected_cwd = g.store.session(target).unwrap().effective_cwd().to_owned();

        let id = g
            .store
            .add_page_session(target, overlay, PageCompanionOptions::default(), HOME)
            .unwrap();

        assert_eq!(ids(&g.store, g.work), [g.parent, g.first, g.second, id, g.after]);
        let row = g.store.session(id).unwrap();
        assert_eq!(row.parent_id(), None);
        assert_eq!(row.content(), SessionContent::Page);
        assert_eq!(row.overlay.page().map(|page| page.id), Some(shown));
        assert!(row.overlay.page().unwrap().row);
        assert_eq!(row.effective_cwd(), expected_cwd);
        assert_eq!(g.store.selected_session_id(), Some(id));
    }
}

/// Upstream's `aPageSessionIsSavedOnCreationAndCreatedUnselectedWhenAsked`.
#[test]
fn a_page_session_is_saved_and_created_unselected_when_asked() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let anchor = add(&mut store, work, "/anchor");
    let options = PageCompanionOptions {
        select: false,
        background_color: Some("#112233".to_owned()),
        ..PageCompanionOptions::default()
    };

    let id = store
        .add_page_session(anchor, page("/tmp/p.html"), options, HOME)
        .unwrap();

    assert_eq!(store.selected_session_id(), Some(anchor));
    assert_eq!(
        store.session(id).unwrap().overlay.background_color.as_deref(),
        Some("#112233")
    );
    let saved = saved_session(&store, id).expect("the page session is saved");
    assert_eq!(saved_page(&saved), file_entry("/tmp/p.html"));
    assert_eq!(saved.pages.unwrap()[0].background_color.as_deref(), Some("#112233"));
    assert_eq!(saved.parent_id, None);
}

/// Upstream's `aRemoteAnchorPlacesALocalPageSessionBesideIt`.
#[test]
fn a_remote_anchor_places_a_local_page_session_beside_it() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let spec = NewSession {
        remote_host: Some("buildbox".to_owned()),
        ..NewSession::at_cwd("/nowhere-on-this-machine")
    };
    let remote = store.add_session(work, spec).unwrap();
    let after = add(&mut store, work, "/after");

    let id = store
        .add_page_session(remote, page("/tmp/p.html"), PageCompanionOptions::default(), HOME)
        .unwrap();

    assert_eq!(ids(&store, work), [remote, id, after]);
    let row = store.session(id).unwrap();
    assert_eq!(row.remote_host(), None);
    assert_eq!(row.effective_cwd(), HOME);
    assert!(saved_session(&store, id).is_some(), "the page is this machine's own");
}

#[test]
fn a_page_session_needs_an_anchor() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");

    let missing = store.add_page_session(
        Uuid::new_v4(),
        page("/tmp/p.html"),
        PageCompanionOptions::default(),
        HOME,
    );

    assert_eq!(missing, Err(CompanionRefusal::NoSuchSession));
    assert!(ids(&store, work).is_empty());
}

/// Upstream's `aRemotePageRowIsRefusedAsAParent`.
#[test]
fn a_remote_page_row_is_refused_as_a_parent() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let spec = NewSession {
        remote_host: Some("buildbox".to_owned()),
        content: SessionContent::Page,
        ..NewSession::at_cwd("/r")
    };
    let remote = store.add_session(work, spec).unwrap();

    let terminal = store.add_companion(remote, CompanionOptions::default(), HOME);
    let nested = store.add_page_companion(remote, page("/tmp/p.html"), PageCompanionOptions::default(), HOME);

    assert_eq!(terminal, Err(CompanionRefusal::RemoteParent));
    assert_eq!(nested, Err(CompanionRefusal::RemoteParent));
    assert_eq!(store.convert_to_companion(remote), Err(ConvertRefusal::RemoteSession));
    assert_eq!(ids(&store, work), [remote]);
    assert!(
        saved_session(&store, remote).is_none(),
        "a row attached from another machine is never saved"
    );
}

#[test]
fn closing_a_page_root_closes_its_companions_into_one_recent_entry() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let root = top_level_page(&mut store, work);
    let after = add(&mut store, work, "/after");
    let terminal = store.add_companion(root, CompanionOptions::default(), HOME).unwrap();
    let nested = store
        .add_page_companion(
            root,
            page(&os_path("/tmp/n.html")),
            PageCompanionOptions::default(),
            HOME,
        )
        .unwrap();

    store.close_session(root);

    assert_eq!(ids(&store, work), [after]);
    assert!([root, terminal, nested].iter().all(|id| store.session(*id).is_none()));
    let item = store.recent_closed_items()[0].clone();
    let recent = item.session.as_ref().unwrap();
    assert_eq!(saved_page(&recent.snapshot), file_entry(&os_path("/tmp/root.html")));
    let companions: Vec<Uuid> = recent
        .companions
        .iter()
        .flatten()
        .map(|companion| companion.id)
        .collect();
    assert_eq!(companions, [terminal, nested]);

    assert!(store.restore_recent_closed(&item));
    assert_eq!(ids(&store, work), [root, terminal, nested, after]);
    for (id, path) in [(root, os_path("/tmp/root.html")), (nested, os_path("/tmp/n.html"))] {
        let row = store.session(id).unwrap();
        assert_eq!(row.content(), SessionContent::Page);
        let shown = row.overlay.page().expect("its page opens again");
        assert!(shown.row);
        assert!(matches!(&shown.source, HtmlSource::File { path: shown, .. } if *shown == path));
    }
    assert_eq!(store.session(nested).unwrap().parent_id(), Some(root));
    assert_eq!(store.session(terminal).unwrap().parent_id(), Some(root));
}

/// Upstream's `softClosingAPageRootIsOneUndoForTheWholeGroup`.
#[test]
fn soft_closing_a_page_root_is_one_undo_for_the_whole_group() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let root = top_level_page(&mut store, work);
    let after = add(&mut store, work, "/after");
    let terminal = store.add_companion(root, CompanionOptions::default(), HOME).unwrap();
    let nested = store
        .add_page_companion(
            root,
            page(&os_path("/tmp/n.html")),
            PageCompanionOptions::default(),
            HOME,
        )
        .unwrap();
    let shown = store.session(root).unwrap().overlay.page().unwrap().id;

    assert!(store.soft_close_session(root));
    assert_eq!(ids(&store, work), [after]);
    assert!(store.undo_pending_close(None));

    assert_eq!(ids(&store, work), [root, terminal, nested, after]);
    assert_eq!(store.session(terminal).unwrap().parent_id(), Some(root));
    assert_eq!(store.session(nested).unwrap().parent_id(), Some(root));
    assert_eq!(
        store.session(root).unwrap().overlay.page().map(|page| page.id),
        Some(shown)
    );
}
