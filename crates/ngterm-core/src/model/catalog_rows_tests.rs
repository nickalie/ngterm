//! A remote window's rows against catalog entries (upstream's `RemoteCatalogMirrorTests` row cases), on a bare store.

use std::collections::HashSet;

use uuid::Uuid;

use super::{CatalogRowCommands, CatalogRowContext};
use crate::catalog::fixture::{page_row, pane, session, session_with, workspace};
use crate::catalog::frames::{CatalogPane, Lifecycle};
use crate::model::AppStore;

/// Names each command after what it attaches.
#[derive(Default)]
struct Commands {
    attached: Vec<Uuid>,
}

impl CatalogRowCommands for Commands {
    fn attach(&mut self, pane: &CatalogPane) -> String {
        self.attached.push(pane.identity);
        format!("attach {}", pane.daemon)
    }

    fn scratch(&mut self, pane: &CatalogPane) -> String {
        format!("scratch {}", pane.daemon)
    }

    fn ended(&mut self) -> String {
        "ended".to_owned()
    }
}

fn context(commands: &mut Commands) -> CatalogRowContext<'_> {
    CatalogRowContext {
        host: "box",
        protocol: 1,
        commands,
    }
}

#[test]
fn a_workspace_the_catalog_brings_opens_unless_the_view_collapsed_it() {
    let mut store = AppStore::new();
    let (open, folded) = (Uuid::new_v4(), Uuid::new_v4());
    let catalog = vec![
        workspace(open, "a\u{7}b", Vec::new()),
        workspace(folded, "f", Vec::new()),
    ];
    store.prepare_catalog_workspaces(&catalog, &HashSet::from([folded]));
    store.finish_catalog_workspaces(&catalog);
    let workspaces = store.workspaces();
    assert_eq!(workspaces[0].name, "ab", "a name is sanitized");
    assert!(workspaces[0].is_expanded);
    assert!(!workspaces[1].is_expanded);
}

#[test]
fn a_page_row_is_a_page_bound_to_its_server_with_no_pane() {
    let mut store = AppStore::new();
    let page = page_row();
    let w = Uuid::new_v4();
    let catalog = vec![workspace(w, "w", vec![page.clone()])];
    store.prepare_catalog_workspaces(&catalog, &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&page, w, 0, &mut context(&mut commands));
    let row = store.session(page.id()).unwrap();
    assert_eq!(row.content(), crate::model::SessionContent::Page);
    assert_eq!(
        row.custom_name.as_deref(),
        Some("page"),
        "a page row keeps its catalog label"
    );
    assert!(row.launch.command.is_none());
    let binding = &row.remote_presentation().unwrap().binding;
    assert!(binding.catalog_owned);
    assert_eq!(
        binding.origin.as_ref().unwrap().endpoint,
        page.fields.page.unwrap().endpoint
    );
    assert!(commands.attached.is_empty());
}

#[test]
fn a_split_row_ending_before_either_attach_started_runs_the_ended_line_in_both_panes() {
    let mut store = AppStore::new();
    let split = session_with(Uuid::new_v4(), "s", vec![pane(), pane()]);
    let w = Uuid::new_v4();
    store.prepare_catalog_workspaces(&[workspace(w, "w", vec![split.clone()])], &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&split, w, 0, &mut context(&mut commands));
    let mut ended = split.clone();
    ended.fields.lifecycle = Lifecycle::Ended;
    ended.panes.clear();
    store.update_catalog_session(split.id(), &ended, &mut context(&mut commands));
    let row = store.session(split.id()).unwrap();
    assert!(row.ended);
    assert_eq!(row.launch.command.as_deref(), Some("ended"));
    assert_eq!(row.split_launch.command.as_deref(), Some("ended"));
}

#[test]
fn a_live_row_listing_no_pane_keeps_its_layout() {
    let mut store = AppStore::new();
    let split = session_with(Uuid::new_v4(), "s", vec![pane(), pane()]);
    let w = Uuid::new_v4();
    store.prepare_catalog_workspaces(&[workspace(w, "w", vec![split.clone()])], &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&split, w, 0, &mut context(&mut commands));
    let mut between = split.clone();
    between.panes.clear();
    store.update_catalog_session(split.id(), &between, &mut context(&mut commands));
    let row = store.session(split.id()).unwrap();
    assert!(row.split.exists);
    assert_eq!(row.pane_identity, split.panes[0].identity);
}

#[test]
fn a_new_primary_pane_attaches_in_place_of_the_old_one() {
    let mut store = AppStore::new();
    let a = session("a");
    let w = Uuid::new_v4();
    store.prepare_catalog_workspaces(&[workspace(w, "w", vec![a.clone()])], &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&a, w, 0, &mut context(&mut commands));
    let fresh = pane();
    let mut replaced = a.clone();
    replaced.panes = vec![fresh.clone()];
    replaced.fields.primary = fresh.identity;
    store.update_catalog_session(a.id(), &replaced, &mut context(&mut commands));
    let row = store.session(a.id()).unwrap();
    assert_eq!(row.pane_identity, fresh.identity);
    assert_eq!(row.launch.command, Some(format!("attach {}", fresh.daemon)));
    assert_eq!(commands.attached.last(), Some(&fresh.identity));
}

#[test]
fn a_removed_row_records_nothing_in_open_recent() {
    let mut store = AppStore::new();
    let a = session("a");
    let w = Uuid::new_v4();
    store.prepare_catalog_workspaces(&[workspace(w, "w", vec![a.clone()])], &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&a, w, 0, &mut context(&mut commands));
    let removed = store.prepare_catalog_workspaces(&[workspace(w, "w", Vec::new())], &HashSet::new());
    assert_eq!(removed, vec![a.id()]);
    assert!(store.session(a.id()).is_none());
    assert!(store.recent_closed.items().is_empty());
}

#[test]
fn an_ended_row_with_a_running_scratch_points_its_presentation_at_the_scratch() {
    let mut store = AppStore::new();
    let a = session("a");
    let w = Uuid::new_v4();
    store.prepare_catalog_workspaces(&[workspace(w, "w", vec![a.clone()])], &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&a, w, 0, &mut context(&mut commands));
    let mut scratch = pane();
    scratch.endpoint.socket_directory = "/run/scratch".to_owned();
    let mut ended = a.clone();
    ended.fields.lifecycle = Lifecycle::Ended;
    ended.panes.clear();
    ended.fields.scratch = Some(crate::catalog::frames::CatalogScratch {
        pane: scratch.clone(),
        shown: false,
    });
    store.update_catalog_session(a.id(), &ended, &mut context(&mut commands));
    let row = store.session(a.id()).unwrap();
    assert!(row.ended && row.presents_catalog());
    let origin = row.remote_presentation().unwrap().binding.origin.clone().unwrap();
    assert_eq!(origin.endpoint, scratch.endpoint);
    assert_eq!(origin.session_name, crate::pane_env::uuid_string(a.id()));
}

#[test]
fn a_shown_split_takes_the_servers_axis() {
    let mut store = AppStore::new();
    let split = session_with(Uuid::new_v4(), "s", vec![pane(), pane()]);
    let w = Uuid::new_v4();
    store.prepare_catalog_workspaces(&[workspace(w, "w", vec![split.clone()])], &HashSet::new());
    let mut commands = Commands::default();
    store.insert_catalog_session(&split, w, 0, &mut context(&mut commands));
    let before = store.session(split.id()).unwrap().split.axis;
    let mut turned = split.clone();
    turned.fields.axis = Some(if before == crate::model::SplitAxis::from_wire("vertical").unwrap() {
        "horizontal".to_owned()
    } else {
        "vertical".to_owned()
    });
    store.update_catalog_session(split.id(), &turned, &mut context(&mut commands));
    let row = store.session(split.id()).unwrap();
    assert_ne!(row.split.axis, before);
    assert!(row.split.shown);
}
