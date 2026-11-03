//! Session info validation, lifetime and runtime state (upstream SessionInfo/ControlServerSessionInfo tests).
use super::{Fixture, args, failure, wire};
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Command, Response};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

fn set(fixture: &mut Fixture, id: Uuid, text: &str, edit: impl FnOnce(&mut Args)) -> Response {
    fixture.session(
        SessionCmd::Info,
        Some(&wire(id)),
        args(|a| {
            a.mode = Some("set".to_owned());
            a.text = Some(text.to_owned());
            edit(a);
        }),
    )
}
fn clear(fixture: &mut Fixture, id: Uuid) -> Response {
    fixture.session(
        SessionCmd::Info,
        Some(&wire(id)),
        args(|a| a.mode = Some("clear".to_owned())),
    )
}
fn node(fixture: &mut Fixture, id: Uuid) -> Value {
    fixture.send(Command::Tree, None, Args::default()).result.unwrap()["tree"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|workspace| workspace["sessions"].as_array().unwrap())
        .find(|node| node["id"] == wire(id))
        .unwrap()
        .clone()
}

#[test]
fn info_sets_reads_back_replaces_and_clears_without_persisting() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    for field in ["info", "infoMarkdown", "infoHideAfter"] {
        assert!(node(&mut fixture, id).get(field).is_none());
    }
    assert!(
        set(&mut fixture, id, "**3**", |a| {
            a.markdown = Some(true);
            a.hide_after = Some(1.5);
        })
        .ok
    );
    let read = node(&mut fixture, id);
    assert_eq!(
        (
            read["info"].clone(),
            read["infoMarkdown"].clone(),
            read["infoHideAfter"].clone()
        ),
        (json!("**3**"), json!(true), json!(1.5))
    );
    assert_eq!(fixture.core.take_info_timers()[0].delay, Duration::from_millis(1500));
    assert!(set(&mut fixture, id, "literal **stars**", |_| {}).ok);
    assert_eq!(node(&mut fixture, id)["infoMarkdown"], false);
    assert_eq!(node(&mut fixture, id)["infoHideAfter"], 0.0);
    assert!(fixture.core.take_info_timers().is_empty());
    assert!(clear(&mut fixture, id).ok);
    for field in ["info", "infoMarkdown", "infoHideAfter"] {
        assert!(node(&mut fixture, id).get(field).is_none());
    }
}

#[test]
fn info_is_never_snapshotted_restored_or_duplicated() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    assert!(set(&mut fixture, id, "literal **stars**", |_| {}).ok);
    let snapshot = fixture.store().snapshot();
    assert!(!serde_json::to_string(&snapshot).unwrap().contains("literal **stars**"));
    let mut restored = crate::model::AppStore::new();
    restored.restore(&snapshot, false);
    assert!(restored.session(id).unwrap().effective_info().is_none());
    let duplicate = fixture.store_mut().unwrap().duplicate_session(id, "/home/u").unwrap();
    assert!(fixture.store().session(duplicate).unwrap().info.is_none());
}

#[test]
fn validation_precedes_target_resolution_and_keeps_value_and_timer() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    assert!(set(&mut fixture, id, "kept", |a| a.hide_after = Some(30.0)).ok);
    let timer = fixture.core.take_info_timers()[0];
    let bad = [
        (Args::default(), "invalid info mode:  (set|clear)"),
        (
            args(|a| a.mode = Some("other".into())),
            "invalid info mode: other (set|clear)",
        ),
        (args(|a| a.mode = Some("set".into())), "session.info set requires text"),
        (
            args(|a| {
                a.mode = Some("clear".into());
                a.text = Some(String::new());
            }),
            "session.info clear takes no text, markdown or hide-after",
        ),
        (
            args(|a| {
                a.mode = Some("clear".into());
                a.markdown = Some(false);
            }),
            "session.info clear takes no text, markdown or hide-after",
        ),
        (
            args(|a| {
                a.mode = Some("clear".into());
                a.hide_after = Some(0.0);
            }),
            "session.info clear takes no text, markdown or hide-after",
        ),
        (
            args(|a| {
                a.mode = Some("set".into());
                a.text = Some("x".into());
                a.hide_after = Some(-1.0);
            }),
            "session.info: --hide-after must be 0...86400 seconds",
        ),
        (
            args(|a| {
                a.mode = Some("set".into());
                a.text = Some("  ".into());
            }),
            "session.info: info text is empty",
        ),
    ];
    for (args, expected) in bad {
        assert_eq!(
            fixture.session(SessionCmd::Info, Some("zzz"), args.clone()),
            failure(expected)
        );
        assert_eq!(
            fixture.session(SessionCmd::Info, Some(&wire(id)), args),
            failure(expected)
        );
    }
    assert!(fixture.core.take_info_timers().is_empty());
    assert_eq!(
        fixture.store().session(id).unwrap().info.as_ref().unwrap().source,
        "kept"
    );
    assert!(fixture.core.fire_info_hide(timer));
}

#[test]
fn identical_sets_refresh_timers_and_superseded_timers_are_inert() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    assert!(set(&mut fixture, id, "brief", |a| a.hide_after = Some(0.05)).ok);
    let first = fixture.core.take_info_timers()[0];
    assert!(set(&mut fixture, id, "brief", |a| a.hide_after = Some(0.05)).ok);
    let second = fixture.core.take_info_timers()[0];
    assert!(!fixture.core.fire_info_hide(first));
    assert!(fixture.core.fire_info_hide(second));
    assert!(!fixture.core.fire_info_hide(second));
    assert!(fixture.store().session(id).unwrap().info.is_none());
    for mode in ["clear", "omitted", "zero"] {
        assert!(set(&mut fixture, id, "timed", |a| a.hide_after = Some(30.0)).ok);
        let timer = fixture.core.take_info_timers()[0];
        if mode == "clear" {
            assert!(clear(&mut fixture, id).ok);
        } else {
            assert!(
                set(&mut fixture, id, "stays", |a| a.hide_after =
                    (mode == "zero").then_some(0.0))
                .ok
            );
        }
        assert!(!fixture.core.fire_info_hide(timer));
        assert!(fixture.core.take_info_timers().is_empty());
    }
}

#[test]
fn teardown_cancels_but_overlay_close_preserves_info() {
    for scope in ["session", "workspace", "window", "overlay", "hud"] {
        let mut fixture = Fixture::new(0);
        let id = fixture.sessions[0];
        assert!(set(&mut fixture, id, "kept", |a| a.hide_after = Some(30.0)).ok);
        let timer = fixture.core.take_info_timers()[0];
        match scope {
            "session" => {
                fixture.store_mut().unwrap().close_session(id);
            }
            "workspace" => {
                let workspace = fixture.workspace;
                fixture.add_workspace("staying", 0);
                fixture.store_mut().unwrap().remove_workspace(workspace);
            }
            "window" => {
                let window = fixture.window;
                fixture.core.library_mut().close_window(window);
            }
            "overlay" => {
                fixture.store_mut().unwrap().close_overlay(id);
            }
            _ => {
                fixture.store_mut().unwrap().close_hud(id);
            }
        }
        assert_eq!(
            fixture.core.fire_info_hide(timer),
            matches!(scope, "overlay" | "hud"),
            "{scope}"
        );
    }
}

#[test]
fn soft_close_drops_timed_info_and_undo_keeps_untimed_info() {
    for path in ["session", "batch", "workspace"] {
        let mut fixture = Fixture::new(1);
        let timed = fixture.sessions[0];
        let kept = fixture.sessions[1];
        assert!(set(&mut fixture, timed, "brief", |a| a.hide_after = Some(30.0)).ok);
        let expiry = fixture.core.take_info_timers()[0];
        assert!(set(&mut fixture, kept, "kept", |_| {}).ok);
        let workspace = fixture.workspace;
        match path {
            "session" => {
                assert!(fixture.store_mut().unwrap().soft_close_session(timed));
                assert!(fixture.store_mut().unwrap().soft_close_session(kept));
            }
            "batch" => {
                assert!(fixture.store_mut().unwrap().soft_close_sessions(&[timed, kept]));
            }
            _ => {
                fixture.add_workspace("staying", 0);
                assert!(fixture.store_mut().unwrap().soft_remove_workspace(workspace));
            }
        }
        assert!(!fixture.core.fire_info_hide(expiry));
        while fixture.store_mut().unwrap().undo_pending_close(None) {}
        assert!(fixture.store().session(timed).unwrap().info.is_none(), "{path}");
        assert_eq!(
            fixture.store().session(kept).unwrap().info.as_ref().unwrap().source,
            "kept",
            "{path}"
        );
        assert!(!fixture.core.fire_info_hide(expiry));
    }
}

#[test]
fn info_schedules_tree_changed_only_for_effective_changes_and_never_saves() {
    use crate::model::SaveRequest;
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    for timer in fixture.core.commit().tree_changed {
        fixture.core.fire_tree_changed(timer);
    }
    assert!(set(&mut fixture, id, "3 agents", |_| {}).ok);
    assert_eq!(fixture.store().outbox().save, SaveRequest::None);
    let timers = fixture.core.commit().tree_changed;
    assert_eq!(timers.len(), 1);
    for timer in timers {
        fixture.core.fire_tree_changed(timer);
    }
    assert!(set(&mut fixture, id, "3 agents", |_| {}).ok);
    assert!(fixture.core.commit().tree_changed.is_empty());
    assert!(clear(&mut fixture, id).ok);
    assert_eq!(fixture.core.commit().tree_changed.len(), 1);
}
