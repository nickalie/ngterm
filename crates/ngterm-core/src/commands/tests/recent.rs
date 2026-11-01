//! `recent.*`, ported from upstream's `ControlDispatcherRecentTests.swift`, `ControlRecentTests.swift`,
//! `WindowLibraryRecentClosedTests.swift` and the app's `ControlServerRecentTests.swift`. The write failures run over a
//! state directory that refuses writes (read-only on Unix; on Windows, where a directory's read-only attribute stops
//! nothing, its files read-only, which a replace refuses).

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use ngterm_proto::command::RecentCmd;
use ngterm_proto::{AppIdentity, Args, Command, ControlRecentItem, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, UNKNOWN, args, failure, send, wire};
use crate::commands::recent::{project, resolve_recent_target};
use crate::error::ControlError;
use crate::model::{AppStore, NewSession, RecentClosedReopen, WindowLibrary};
use crate::persist::{
    AppSettings, LibraryPersistence, RecentClosedItem, RecentClosedKind, RecentClosedSession, RecentClosedWorkspace,
    SessionSnapshot, WorkspaceSnapshot,
};
use crate::{Core, CoreConfig, CoreState};

fn recent(core: &mut Core, cmd: RecentCmd, target: Option<&str>, args: Args) -> Response {
    send(core, Command::Recent(cmd), target, args)
}

/// `recent.list`'s entries.
fn listed(core: &mut Core) -> Vec<Value> {
    let response = recent(core, RecentCmd::List, None, Args::default());
    assert!(response.ok, "{response:?}");
    response.result.unwrap()["recent"].as_array().unwrap().clone()
}

/// What a successful `recent.open` reports.
fn opened(response: &Response) -> Value {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().unwrap()["recentOpen"].clone()
}

fn window_args(window: &str) -> Args {
    args(|a| a.window = Some(window.to_owned()))
}

/// Adds workspace `project-<name>` with one session named `name` to `store`, closes the session for good and returns
/// its id (upstream's `closeNewSession`).
fn close_new_session(store: &mut AppStore, name: &str) -> Uuid {
    let workspace = store.add_workspace(&format!("project-{name}"));
    let session = add_named(store, workspace, name);
    assert!(store.close_session(session).is_some());
    session
}

fn add_named(store: &mut AppStore, workspace: Uuid, name: &str) -> Uuid {
    let spec = NewSession {
        name: Some(name.to_owned()),
        ..NewSession::at_cwd("/tmp")
    };
    store.add_session(workspace, spec).expect("the workspace exists")
}

fn store_mut(core: &mut Core, window: Uuid) -> &mut AppStore {
    core.library_mut().store_mut(window).expect("an open window")
}

fn session_ids(entry: &Value) -> Vec<String> {
    serde_json::from_value(entry["sessionIDs"].clone()).unwrap()
}

// MARK: - Dispatcher (`ControlDispatcherRecentTests.swift`)

#[test]
fn list_and_clear_refuse_a_target_or_window_before_any_action() {
    let mut fixture = Fixture::new(0);
    close_new_session(store_mut(&mut fixture.core, fixture.window), "api");
    for (cmd, name) in [(RecentCmd::List, "recent.list"), (RecentCmd::Clear, "recent.clear")] {
        let refusal = failure(&format!("{name} takes no target or --window"));
        assert_eq!(recent(&mut fixture.core, cmd, Some("active"), Args::default()), refusal);
        assert_eq!(recent(&mut fixture.core, cmd, None, window_args("w1")), refusal);
        assert_eq!(recent(&mut fixture.core, cmd, None, window_args(UNKNOWN)), refusal);
    }
    assert_eq!(listed(&mut fixture.core).len(), 1, "a refused clear cleared nothing");
}

#[test]
fn clear_answers_a_bare_ok() {
    let mut fixture = Fixture::new(0);
    assert_eq!(
        recent(&mut fixture.core, RecentCmd::Clear, None, Args::default()),
        Response::success()
    );
}

// MARK: - Projection and resolution (`ControlRecentTests.swift`)

struct Entries {
    workspace: Uuid,
    first: SessionSnapshot,
    second: SessionSnapshot,
}

impl Entries {
    fn new() -> Self {
        Self {
            workspace: Uuid::new_v4(),
            first: SessionSnapshot::new(Uuid::new_v4(), "/a"),
            second: SessionSnapshot::new(Uuid::new_v4(), "/b"),
        }
    }

    fn session_item(&self, id: Uuid) -> RecentClosedItem {
        RecentClosedItem {
            id,
            kind: RecentClosedKind::Session,
            title: "a".to_owned(),
            subtitle: Some("project".to_owned()),
            closed_at: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            session: Some(RecentClosedSession {
                workspace_id: self.workspace,
                workspace_name: "project".to_owned(),
                workspace_index: 0,
                session_index: 1,
                snapshot: self.first.clone(),
            }),
            workspace: None,
        }
    }

    fn workspace_item(&self, id: Uuid) -> RecentClosedItem {
        RecentClosedItem {
            id,
            kind: RecentClosedKind::Workspace,
            title: "project".to_owned(),
            subtitle: Some("2 sessions".to_owned()),
            closed_at: UNIX_EPOCH + Duration::from_secs(1_700_000_500),
            session: None,
            workspace: Some(RecentClosedWorkspace {
                snapshot: WorkspaceSnapshot {
                    id: self.workspace,
                    name: "project".to_owned(),
                    sessions: vec![self.first.clone(), self.second.clone()],
                    collapsed: None,
                    tint: None,
                },
                selected_session_id: Some(self.second.id),
                focus_member: None,
            }),
        }
    }
}

fn parsed(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap()
}

#[test]
fn a_session_entry_projects_its_parent_workspace_and_one_session() {
    let entries = Entries::new();
    let item = entries.session_item(Uuid::new_v4());
    assert_eq!(
        project(&item),
        ControlRecentItem {
            id: wire(item.id),
            kind: "session".to_owned(),
            title: "a".to_owned(),
            subtitle: Some("project".to_owned()),
            closed_at: 1_700_000_000.0,
            workspace_id: wire(entries.workspace),
            session_ids: vec![wire(entries.first.id)],
        }
    );
}

#[test]
fn a_workspace_entry_projects_itself_and_every_snapshot_session() {
    let entries = Entries::new();
    let item = entries.workspace_item(Uuid::new_v4());
    assert_eq!(
        project(&item),
        ControlRecentItem {
            id: wire(item.id),
            kind: "workspace".to_owned(),
            title: "project".to_owned(),
            subtitle: Some("2 sessions".to_owned()),
            closed_at: 1_700_000_500.0,
            workspace_id: wire(entries.workspace),
            session_ids: vec![wire(entries.first.id), wire(entries.second.id)],
        }
    );
}

/// The file's reference-date seconds come out as Unix seconds, through the same `Date` the file decodes to.
#[test]
fn closed_at_converts_the_files_reference_date_seconds() {
    let text = r#"{"version":1,"items":[{"id":"AAAA0000-0000-0000-0000-000000000001","kind":"session",
        "title":"a","closedAt":721692800.25}]}"#;
    let state: crate::persist::RecentClosedState = serde_json::from_str(text).unwrap();
    let projected = project(&state.items[0]);
    assert_eq!(projected.closed_at.to_bits(), 1_700_000_000.25_f64.to_bits());
    assert_eq!((projected.workspace_id.as_str(), projected.session_ids.len()), ("", 0));
}

#[test]
fn no_target_resolves_the_newest_entry() {
    let entries = Entries::new();
    let (newest, older) = (
        entries.session_item(Uuid::new_v4()),
        entries.workspace_item(Uuid::new_v4()),
    );
    assert_eq!(resolve_recent_target(None, &[newest.clone(), older]), Ok(newest.id));
}

#[test]
fn a_target_resolves_by_exact_id_or_unique_prefix() {
    let entries = Entries::new();
    let a = entries.session_item(parsed("AAAA0000-0000-0000-0000-000000000001"));
    let b = entries.workspace_item(parsed("BBBB0000-0000-0000-0000-000000000002"));
    let items = [a.clone(), b.clone()];
    assert_eq!(resolve_recent_target(Some("bbbb"), &items), Ok(b.id));
    assert_eq!(
        resolve_recent_target(Some(&wire(a.id).to_lowercase()), &items),
        Ok(a.id)
    );
}

#[test]
fn an_empty_list_is_refused_whatever_the_target() {
    for target in [None, Some("ab")] {
        assert_eq!(
            resolve_recent_target(target, &[]),
            Err(ControlError::rejected("no recently closed items"))
        );
    }
}

#[test]
fn unknown_ambiguous_and_active_targets_are_refused() {
    let entries = Entries::new();
    let a = entries.session_item(parsed("AAAA0000-0000-0000-0000-000000000001"));
    let b = entries.workspace_item(parsed("AAAA0000-0000-0000-0000-000000000002"));
    let items = [a, b];
    let message = |target| resolve_recent_target(Some(target), &items).unwrap_err().to_string();
    assert_eq!(message("cccc"), "no such recent item: cccc");
    assert_eq!(message("active"), "no such recent item: active");
    assert_eq!(
        message("aaaa"),
        "ambiguous recent item prefix 'aaaa' → AAAA0000, AAAA0000"
    );
}

// MARK: - The reopen outcome (`WindowLibraryRecentClosedTests.swift`)

#[test]
fn a_reopen_reports_its_window_and_consumes_the_entry() {
    let mut library = WindowLibrary::new("/home/u");
    let window = library.new_window(None);
    let store = library.store_mut(window).unwrap();
    let session = close_new_session(store, "api");
    let entry = library.recent_closed_items()[0].id;

    let outcome = library.reopen_recent_closed_outcome(entry, None);

    assert_eq!(outcome, Some(RecentClosedReopen { window, complete: true }));
    assert!(library.store(window).unwrap().session(session).is_some());
    assert!(library.recent_closed_items().is_empty());
}

#[test]
fn a_reopen_reports_the_window_holding_the_workspace_not_the_requested_one() {
    let mut library = WindowLibrary::new("/home/u");
    let source = library.new_window(None);
    let store = library.store_mut(source).unwrap();
    let workspace = store.add_workspace("project");
    let session = add_named(store, workspace, "api");
    let destination = library.new_window(None);
    assert!(library.store_mut(source).unwrap().close_session(session).is_some());
    let entry = library.recent_closed_items()[0].id;

    let outcome = library.reopen_recent_closed_outcome(entry, Some(destination));

    assert_eq!(
        outcome,
        Some(RecentClosedReopen {
            window: source,
            complete: true
        })
    );
    assert!(library.store(source).unwrap().session(session).is_some());
    assert!(library.store(destination).unwrap().session(session).is_none());
}

#[test]
fn an_unknown_entry_reopens_nothing() {
    let mut library = WindowLibrary::new("/home/u");
    library.new_window(None);
    assert_eq!(library.reopen_recent_closed_outcome(Uuid::new_v4(), None), None);
}

// MARK: - The app side (`ControlServerRecentTests.swift`)

#[test]
fn list_reports_entries_newest_first_and_clear_empties_it() {
    let mut fixture = Fixture::new(0);
    let store = store_mut(&mut fixture.core, fixture.window);
    let older = close_new_session(store, "older");
    let newer = close_new_session(store, "newer");

    let entries = listed(&mut fixture.core);
    assert_eq!(
        entries.iter().map(session_ids).collect::<Vec<_>>(),
        [[wire(newer)], [wire(older)]]
    );
    let titles: Vec<&str> = entries.iter().map(|entry| entry["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["newer", "older"]);
    assert_eq!(entries[0]["kind"], json!("session"));
    assert_eq!(entries[0]["subtitle"], json!("project-newer"));

    let clear = || Args::default();
    assert_eq!(
        recent(&mut fixture.core, RecentCmd::Clear, None, clear()),
        Response::success()
    );
    assert!(listed(&mut fixture.core).is_empty());
    assert_eq!(
        recent(&mut fixture.core, RecentCmd::Clear, None, clear()),
        Response::success()
    );
}

#[test]
fn open_without_an_id_reopens_the_newest_entry_and_consumes_it() {
    let mut fixture = Fixture::new(0);
    let store = store_mut(&mut fixture.core, fixture.window);
    let older = close_new_session(store, "older");
    let newer = close_new_session(store, "newer");

    let open = opened(&recent(&mut fixture.core, RecentCmd::Open, None, Args::default()));

    assert_eq!(open["window"], json!(wire(fixture.window)));
    assert_eq!(open["complete"], json!(true));
    assert_eq!(session_ids(&open["item"]), [wire(newer)]);
    assert!(fixture.store().session(newer).is_some());
    assert!(fixture.store().session(older).is_none());
    let left: Vec<_> = listed(&mut fixture.core).iter().map(session_ids).collect();
    assert_eq!(left, [[wire(older)]]);
}

#[test]
fn open_by_prefix_reopens_that_entry() {
    let mut fixture = Fixture::new(0);
    let store = store_mut(&mut fixture.core, fixture.window);
    let older = close_new_session(store, "older");
    close_new_session(store, "newer");
    let entry = wire(fixture.core.library().recent_closed_items()[1].id);

    let response = recent(&mut fixture.core, RecentCmd::Open, Some(&entry[..12]), Args::default());

    assert_eq!(opened(&response)["item"]["id"], json!(entry));
    assert!(fixture.store().session(older).is_some());
}

#[test]
fn an_unknown_window_or_entry_restores_nothing() {
    let mut fixture = Fixture::new(0);
    let closed = close_new_session(store_mut(&mut fixture.core, fixture.window), "api");
    let before = fixture.core.library().recent_closed_items();

    let bad_window = recent(&mut fixture.core, RecentCmd::Open, None, window_args("ffffffff"));
    let bad_entry = recent(&mut fixture.core, RecentCmd::Open, Some("ffffffff"), Args::default());

    assert_eq!(bad_window, failure("no such window: ffffffff"));
    assert_eq!(bad_entry, failure("no such recent item: ffffffff"));
    assert!(fixture.store().session(closed).is_none());
    assert_eq!(fixture.core.library().recent_closed_items(), before);
}

#[test]
fn a_closed_window_restores_nothing() {
    let mut fixture = Fixture::new(0);
    let closed_session = close_new_session(store_mut(&mut fixture.core, fixture.window), "api");
    let closed_window = fixture.core.library_mut().new_window(None);
    fixture.core.library_mut().close_window(closed_window);
    let before = fixture.core.library().recent_closed_items();

    let response = recent(
        &mut fixture.core,
        RecentCmd::Open,
        None,
        window_args(&wire(closed_window)),
    );

    assert_eq!(response, failure("window not open — window.select it first"));
    assert!(fixture.store().session(closed_session).is_none());
    assert_eq!(fixture.core.library().recent_closed_items(), before);
}

#[test]
fn an_empty_list_is_refused() {
    let mut fixture = Fixture::new(0);
    assert_eq!(
        recent(&mut fixture.core, RecentCmd::Open, None, Args::default()),
        failure("no recently closed items")
    );
    assert_eq!(
        recent(&mut fixture.core, RecentCmd::Open, Some("ab"), Args::default()),
        failure("no recently closed items")
    );
}

#[test]
fn every_window_closed_is_refused_before_the_entry() {
    let mut fixture = Fixture::new(0);
    close_new_session(store_mut(&mut fixture.core, fixture.window), "api");
    fixture.core.library_mut().close_window(fixture.window);
    assert_eq!(
        recent(&mut fixture.core, RecentCmd::Open, Some(UNKNOWN), Args::default()),
        failure("no open window")
    );
    assert_eq!(listed(&mut fixture.core).len(), 1);
}

#[test]
fn open_routes_to_the_window_holding_the_workspace_without_moving_frontmost() {
    let mut fixture = Fixture::new(0);
    let source = fixture.window;
    let store = store_mut(&mut fixture.core, source);
    let workspace = store.add_workspace("project");
    let session = add_named(store, workspace, "api");
    let other = fixture.core.library_mut().new_window(None);
    let frontmost = fixture.core.library().frontmost_window_id;
    let takes = fixture.core.frontmost_takes();
    assert!(store_mut(&mut fixture.core, source).close_session(session).is_some());

    let open = opened(&recent(
        &mut fixture.core,
        RecentCmd::Open,
        None,
        window_args(&wire(other)),
    ));

    assert_eq!(open["window"], json!(wire(source)));
    assert!(fixture.store().session(session).is_some());
    assert!(fixture.core.library().store(other).unwrap().session(session).is_none());
    assert_eq!(fixture.core.library().frontmost_window_id, frontmost);
    assert_eq!(fixture.core.frontmost_takes(), takes);
}

#[test]
fn open_during_the_grace_window_restores_the_original_group() {
    let mut fixture = Fixture::new(0);
    let store = store_mut(&mut fixture.core, fixture.window);
    let workspace = store.add_workspace("project");
    let first = add_named(store, workspace, "a");
    let second = add_named(store, workspace, "b");
    assert!(store.soft_close_sessions(&[first, second]));
    assert!(fixture.store().session(first).is_none());

    let open = opened(&recent(&mut fixture.core, RecentCmd::Open, None, Args::default()));

    assert_eq!(open["complete"], json!(true));
    assert_eq!(fixture.order(workspace), [first, second]);
    assert!(fixture.store().pending_close_summary().is_none());
    assert!(listed(&mut fixture.core).is_empty());
}

// MARK: - Write failures

/// A core over a library bootstrapped from `dir`, committed once as the launch does.
fn persistent_core(dir: &Path) -> (Core, Uuid) {
    let mut persistence = LibraryPersistence::new(dir);
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().expect("the seeded window");
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: AppSettings::default(),
            persistence: Some(persistence),
        },
    );
    let _ = core.commit();
    (core, window)
}

/// Makes writes into a directory fail for the guard's life (as `tests/persistence.rs`' guard): on Unix the directory
/// read-only; on Windows each file in it read-only, which a write or a replace refuses.
struct ReadOnly(PathBuf);

impl ReadOnly {
    fn new(dir: &Path) -> Self {
        Self::set(dir, true);
        Self(dir.to_owned())
    }

    #[cfg(unix)]
    fn set(dir: &Path, read_only: bool) {
        use std::os::unix::fs::PermissionsExt as _;

        let mode = if read_only { 0o500 } else { 0o700 };
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(windows)]
    fn set(dir: &Path, read_only: bool) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let mut permissions = std::fs::metadata(&path).unwrap().permissions();
                permissions.set_readonly(read_only);
                std::fs::set_permissions(&path, permissions).unwrap();
            }
        }
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        Self::set(&self.0, false);
    }
}

#[test]
fn an_entry_whose_removal_cannot_be_written_reports_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, window) = persistent_core(dir.path());
    let session = close_new_session(store_mut(&mut core, window), "api");
    let _ = core.commit();
    let entry = core.library().recent_closed_items()[0].id;
    let _guard = ReadOnly::new(dir.path());

    let open = opened(&recent(&mut core, RecentCmd::Open, None, Args::default()));

    assert_eq!(open["complete"], json!(false));
    assert_eq!(open["window"], json!(wire(window)));
    assert!(core.library().store(window).unwrap().session(session).is_some());
    let left: Vec<Uuid> = core
        .library()
        .recent_closed_items()
        .iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(left, [entry]);
}

#[test]
fn a_written_removal_reports_complete_and_leaves_the_file_without_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, window) = persistent_core(dir.path());
    close_new_session(store_mut(&mut core, window), "older");
    close_new_session(store_mut(&mut core, window), "newer");
    let _ = core.commit();

    let open = opened(&recent(&mut core, RecentCmd::Open, None, Args::default()));

    assert_eq!(open["complete"], json!(true));
    let on_disk: Vec<String> = crate::persist::RecentClosedStore::new(dir.path())
        .load()
        .iter()
        .map(|item| item.title.clone())
        .collect();
    assert_eq!(on_disk, ["older"]);
}

#[test]
fn a_clear_that_cannot_be_written_is_an_error_and_keeps_the_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, window) = persistent_core(dir.path());
    close_new_session(store_mut(&mut core, window), "api");
    let _ = core.commit();
    let guard = ReadOnly::new(dir.path());

    assert_eq!(
        recent(&mut core, RecentCmd::Clear, None, Args::default()),
        failure("could not clear recent items")
    );
    assert_eq!(listed(&mut core).len(), 1);

    drop(guard);
    assert_eq!(
        recent(&mut core, RecentCmd::Clear, None, Args::default()),
        Response::success()
    );
    assert!(crate::persist::RecentClosedStore::new(dir.path()).load().is_empty());
}
