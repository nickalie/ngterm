//! `events.read` through the dispatcher, fed by commits of real model mutations (upstream's
//! `ControlEventDispatcherTests` and the `AppStoreEventTests` that read the ring back).

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::command::{EventsCmd, SessionCmd};
use ngterm_proto::{Args, Command, EventKind, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, failure, wire};
use crate::events::{DEFAULT_CAPACITY, EventRing};
use crate::model::{OverlayPane, OverlayProgram, WindowLibrary};
use crate::persist::AppSettings;

const RUN: Uuid = Uuid::from_u128(0xCBB5_E3D0_7A9B_4C96_9EA2_18B1_4380_DDB1);
const RUN_TEXT: &str = "CBB5E3D0-7A9B-4C96-9EA2-18B14380DDB1";

/// A fixture whose seeding is committed before its ring is replaced by an empty one with a known run and clock.
fn fixture(extra: usize) -> Fixture {
    let mut fixture = Fixture::new(extra);
    let _ = fixture.core.commit();
    let placeholder = super::core_over(WindowLibrary::default(), AppSettings::default());
    let core = std::mem::replace(&mut fixture.core, placeholder);
    fixture.core = core.with_event_ring(EventRing::new(DEFAULT_CAPACITY, RUN, Box::new(|| 10.5)));
    fixture
}

fn read(fixture: &mut Fixture, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.send(Command::Events(EventsCmd::Read), None, args(edit))
}

fn from(after: u64) -> impl FnOnce(&mut Args) {
    move |args: &mut Args| {
        args.run = Some(RUN_TEXT.to_owned());
        args.after = Some(after.to_string());
    }
}

fn events_of(response: &Response) -> &Value {
    &response.result.as_ref().unwrap()["events"]
}

fn items(response: &Response) -> Vec<(String, Value, Value)> {
    assert!(response.ok, "{response:?}");
    events_of(response)["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                event["kind"].as_str().unwrap().to_owned(),
                event["session"].clone(),
                event["payload"].clone(),
            )
        })
        .collect()
}

#[test]
fn a_read_without_a_cursor_subscribes_from_now() {
    let mut fixture = fixture(0);
    let response = read(&mut fixture, |_| {});
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json!({"ok": true, "result": {"events": {"run": RUN_TEXT, "next": 0, "items": []}}})
    );
    fixture.add_session(fixture.workspace);
    let _ = fixture.core.commit();
    let later = read(&mut fixture, |_| {});
    assert_eq!(
        events_of(&later)["next"],
        2,
        "created and selected, no history returned"
    );
    assert_eq!(events_of(&later)["items"], json!([]));
}

#[test]
fn a_commit_sequences_lifecycle_events_and_debounces_tree_changed() {
    let mut fixture = fixture(0);
    let first = fixture.sessions[0];
    let response = fixture.session(SessionCmd::New, None, Args::default());
    let added = super::answered_id(&response).unwrap();
    let timers = fixture.core.commit();
    assert_eq!(timers.tree_changed.len(), 1, "one debounce for the window");

    let page = read(&mut fixture, from(0));
    let name = fixture.store().session(added).unwrap().display_name();
    assert_eq!(
        items(&page),
        [
            ("session.created".to_owned(), json!(wire(added)), json!({"name": name})),
            (
                "session.selected".to_owned(),
                json!(wire(added)),
                json!({"name": name, "previous": wire(first)})
            ),
        ]
    );
    let created = &events_of(&page)["items"][0];
    assert_eq!(created["window"], json!(wire(fixture.window)));
    assert_eq!(created["workspace"], json!(wire(fixture.workspace)));
    assert_eq!(created["ts"], json!(10.5));

    for timer in timers.tree_changed {
        fixture.core.fire_tree_changed(timer);
    }
    let tree = read(&mut fixture, from(2));
    assert_eq!(
        serde_json::to_value(&events_of(&tree)["items"]).unwrap(),
        json!([{"seq": 3, "ts": 10.5, "kind": "tree.changed", "window": wire(fixture.window), "payload": {}}])
    );
}

#[test]
fn the_kinds_filter_and_the_page_limit_apply() {
    let mut fixture = fixture(0);
    let response = fixture.session(SessionCmd::New, None, Args::default());
    let added = super::answered_id(&response).unwrap();
    let _ = fixture.core.commit();
    fixture.session(SessionCmd::Close, Some(&wire(added)), Args::default());
    let _ = fixture.core.commit();

    let created_closed = |args: &mut Args| {
        from(0)(args);
        args.kinds = Some(vec![" session.created , session.closed".to_owned()]);
    };
    let whole = read(&mut fixture, created_closed);
    let kinds: Vec<String> = items(&whole).into_iter().map(|(kind, _, _)| kind).collect();
    assert_eq!(kinds, ["session.created", "session.closed"]);

    let page = read(&mut fixture, |args| {
        created_closed(args);
        args.limit = Some(1);
    });
    assert_eq!(events_of(&page)["next"], 1, "a full page stops at its last event");
    assert_eq!(items(&page).len(), 1);
    assert_eq!(EventKind::SessionCreated.wire_name(), items(&page)[0].0);
}

#[test]
fn a_cursor_failure_carries_the_anchor() {
    let mut fixture = fixture(0);
    fixture.add_session(fixture.workspace);
    let _ = fixture.core.commit();
    let ahead = read(&mut fixture, from(99));
    assert_eq!(
        serde_json::to_value(&ahead).unwrap(),
        json!({
            "ok": false,
            "result": {"events": {"run": RUN_TEXT, "next": 2, "items": []}},
            "error": "event cursor is ahead of the current sequence"
        })
    );
    let other_run = read(&mut fixture, |args| {
        args.run = Some("00000000-0000-4000-8000-000000000000".to_owned());
        args.after = Some("0".to_owned());
    });
    assert_eq!(other_run.error.as_deref(), Some("event run changed"));
    assert_eq!(events_of(&other_run)["run"], RUN_TEXT);
}

#[test]
fn a_lower_case_run_reads_and_the_answer_names_the_run_upper_case() {
    let mut fixture = fixture(0);
    let response = read(&mut fixture, |args| {
        args.run = Some(RUN_TEXT.to_lowercase());
        args.after = Some("0".to_owned());
    });
    assert!(response.ok);
    assert_eq!(events_of(&response)["run"], RUN_TEXT);
}

#[test]
fn refused_arguments_answer_upstreams_errors() {
    type Edit = fn(&mut Args);
    let mut fixture = fixture(0);
    let cases: [(Edit, &str); 4] = [
        (
            |args| args.run = Some(RUN_TEXT.to_owned()),
            "events.read requires --run and --after together",
        ),
        (
            |args| {
                args.run = Some(RUN_TEXT.to_owned());
                args.after = Some("-1".to_owned());
            },
            "invalid event cursor",
        ),
        (
            |args| args.limit = Some(1_001),
            "event limit must be between 1 and 1000",
        ),
        (
            |args| args.kinds = Some(vec!["session.created,".to_owned()]),
            "invalid event kind: ",
        ),
    ];
    for (edit, error) in cases {
        assert_eq!(read(&mut fixture, edit), failure(error));
    }
}

// MARK: - app.launched (upstream's `appLaunched*` cases of `AppStoreEventTests`)

/// The `app.launched` events a read from the ring's start returns.
fn launches(fixture: &mut Fixture) -> Vec<Value> {
    let page = read(fixture, |args| {
        from(0)(args);
        args.kinds = Some(vec!["app.launched".to_owned()]);
    });
    assert!(page.ok, "{page:?}");
    events_of(&page)["items"].as_array().unwrap().clone()
}

#[test]
fn app_launched_is_appended_once_per_run_with_no_window_or_session() {
    let mut fixture = fixture(0);
    fixture.core.emit_app_launched(true);
    fixture.core.emit_app_launched(true);
    assert_eq!(
        launches(&mut fixture),
        [json!({"seq": 1, "ts": 10.5, "kind": "app.launched", "payload": {}})]
    );
}

#[test]
fn a_launch_that_was_not_ready_never_emits_app_launched_later() {
    let mut fixture = fixture(0);
    fixture.core.emit_app_launched(false);
    fixture.core.emit_app_launched(true);
    assert_eq!(launches(&mut fixture), Vec::<Value>::new());
}

// MARK: - slot changes outside the store's methods

/// Whether the next commit arms a `tree.changed` debounce, after one that drained what came before.
fn tree_changes(fixture: &mut Fixture, action: impl FnOnce(&mut Fixture)) -> usize {
    let _ = fixture.core.commit();
    action(fixture);
    fixture.core.commit().tree_changed.len()
}

fn program(command: &str) -> OverlayProgram {
    OverlayProgram {
        command: command.to_owned(),
        ..OverlayProgram::default()
    }
}

#[test]
fn a_terminal_exit_that_closes_a_slot_invalidates_the_tree() {
    let mut fixture = fixture(0);
    let id = fixture.sessions[0];
    let store = fixture.store_mut().unwrap();
    assert!(store.open_overlay(id, program("top")));
    store.open_pane_overlay(id, OverlayPane::Left, program("top")).unwrap();
    let generation = fixture.store().session(id).unwrap().overlay.generation;
    let pane_generation = fixture
        .store()
        .session(id)
        .unwrap()
        .pane_overlay(OverlayPane::Left)
        .unwrap()
        .generation;
    assert_eq!(
        tree_changes(&mut fixture, |f| assert!(!f.core.overlay_exited(id, generation + 1))),
        0,
        "a replaced generation"
    );
    assert_eq!(
        tree_changes(&mut fixture, |f| assert!(f.core.overlay_exited(id, generation))),
        1
    );
    assert_eq!(
        tree_changes(&mut fixture, |f| assert!(
            f.core.pane_overlay_exited(id, pane_generation)
        )),
        1
    );
    assert_eq!(
        tree_changes(&mut fixture, |f| assert!(
            !f.core.pane_overlay_exited(id, pane_generation)
        )),
        0
    );
}

#[test]
fn a_terminal_exit_in_a_pending_close_leaves_every_tree_alone() {
    let mut fixture = fixture(1);
    let id = fixture.sessions[0];
    let store = fixture.store_mut().unwrap();
    assert!(store.open_overlay(id, program("top")));
    store.open_pane_overlay(id, OverlayPane::Left, program("top")).unwrap();
    let session = fixture.store().session(id).unwrap();
    let (generation, pane_generation) = (
        session.overlay.generation,
        session.pane_overlay(OverlayPane::Left).unwrap().generation,
    );
    assert!(fixture.store_mut().unwrap().soft_close_session(id));
    assert_eq!(
        tree_changes(&mut fixture, |f| {
            assert!(f.core.overlay_exited(id, generation));
            assert!(f.core.pane_overlay_exited(id, pane_generation));
        }),
        0
    );
}

#[test]
fn a_stranded_pane_overlay_dropped_at_the_commit_invalidates_the_tree() {
    let mut fixture = fixture(0);
    let id = fixture.sessions[0];
    let store = fixture.store_mut().unwrap();
    store.toggle_split(id, None);
    store
        .session_mut(id)
        .unwrap()
        .split_surface
        .replace(Box::new(FakeSurface::new("R")));
    store.open_pane_overlay(id, OverlayPane::Left, program("top")).unwrap();
    let _ = fixture.core.commit();
    // hiding the split behind its focused right pane un-renders the left one before its overlay realized
    fixture.store_mut().unwrap().set_split_visibility(id, false, None);
    let _ = fixture.store_mut().unwrap().take_outbox();
    assert_eq!(fixture.core.commit().tree_changed.len(), 1);
    assert!(
        fixture
            .store()
            .session(id)
            .unwrap()
            .pane_overlay(OverlayPane::Left)
            .is_none()
    );
}
