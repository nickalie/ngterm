//! Saved rows are forgotten only on a definitive refusal, with a banner explaining the loss.

use std::sync::Arc;

use ngterm_proto::AppIdentity;
use serde_json::json;

use super::*;
use crate::model::WindowLibrary;
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};
use crate::{CoreConfig, CoreState, NotificationEffect};

/// An ssh that never reaches any machine, so no listing leaves this one.
struct Offline;

impl RemoteCommandRunner for Offline {
    fn run(&self, _argv: &[String], _deadline: std::time::Duration) -> RemoteCommandResult {
        RemoteCommandResult {
            status: 255,
            stdout: String::new(),
            stderr: "ssh: offline in tests".to_owned(),
        }
    }
}

fn core() -> (Core, Uuid, Uuid) {
    let mut library = WindowLibrary::new("/tmp");
    let window = library.new_window(None);
    let workspace = library.store(window).unwrap().workspaces()[0].id();
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: crate::persist::AppSettings::default(),
            persistence: None,
        },
    );
    core.set_remote_runner(Arc::new(Offline));
    (core, window, workspace)
}

fn saved(host: &str, name: &str, workspace: Uuid) -> RemoteSessionSnapshot {
    RemoteSessionSnapshot {
        host: host.to_owned(),
        session_id: uuid::Uuid::new_v4().to_string(),
        name: name.to_owned(),
        local_name: None,
        workspace_id: workspace,
        after_session_id: None,
        order: 0,
        parent_session_id: None,
        waiting_since: None,
        peer: None,
    }
}

/// A listing of `host`'s `peer` with nothing in flight.
fn read(host: &str, peer: RemotePeer) -> RelaunchRead {
    RelaunchRead {
        host: host.to_owned(),
        peer,
        answer: None,
        failures: 0,
        failing_since: None,
        retry_at: Instant::now(),
    }
}

/// An ssh that never reaches any machine and records every remote command it was given.
#[derive(Default)]
struct Recording(std::sync::Mutex<Vec<Vec<String>>>);

impl RemoteCommandRunner for Recording {
    fn run(&self, argv: &[String], deadline: std::time::Duration) -> RemoteCommandResult {
        self.0.lock().unwrap().push(argv.to_vec());
        Offline.run(argv, deadline)
    }
}

fn banners(core: &mut Core) -> Vec<(String, String, String)> {
    core.take_notifications()
        .into_iter()
        .map(|effect| match effect {
            NotificationEffect::App { identity, title, body } => (identity, title, body),
            other => panic!("unexpected notification: {other:?}"),
        })
        .collect()
}

#[test]
fn invalid_saved_hosts_are_forgotten_in_every_window_and_named_once() {
    let (mut core, first, workspace) = core();
    let second = core.library_mut().new_window(None);
    let other_workspace = core.library().store(second).unwrap().workspaces()[0].id();
    let third = core.library_mut().new_window(None);
    for (window, rows) in [
        (first, vec![saved("-invalid", "one", workspace)]),
        (second, vec![saved("-invalid", "two", other_workspace)]),
    ] {
        core.library_mut()
            .store_mut(window)
            .unwrap()
            .restore_saved_remote_rows(Some(&rows));
    }

    core.reattach_saved_remotes();

    assert_eq!(core.relaunch_pending(), None);
    for window in [first, second, third] {
        assert!(core.library().store(window).unwrap().saved_remote_rows().is_empty());
        assert_eq!(core.library().store(window).unwrap().snapshot().remote_sessions, None);
    }
    assert_eq!(
        banners(&mut core),
        [(
            "remote-relaunch-invalid--invalid".to_owned(),
            "Attach Remote".to_owned(),
            "2 sessions from -invalid not attached again (an invalid host): one, two".to_owned()
        )]
    );
    core.reattach_saved_remotes();
    assert!(banners(&mut core).is_empty());
}

#[test]
fn retry_waits_for_its_deadline_and_stops_when_the_saved_row_is_forgotten() {
    let (mut core, window, workspace) = core();
    let row = saved("studio", "one", workspace);
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .restore_saved_remote_rows(Some(std::slice::from_ref(&row)));
    let now = Instant::now();
    core.remote.viewer.relaunch_reads.push(RelaunchRead {
        host: "studio".to_owned(),
        peer: RemotePeer::Ngterm,
        answer: None,
        failures: 1,
        failing_since: Some(now),
        retry_at: now + std::time::Duration::from_secs(30),
    });
    core.tick_relaunch(now);
    assert_eq!(core.relaunch_pending(), Some(false));
    assert_eq!(
        core.library().store(window).unwrap().saved_remote_rows(),
        std::slice::from_ref(&row)
    );
    core.forget_saved_remote_session("studio", &row.session_id);
    core.tick_relaunch(now);
    assert_eq!(core.relaunch_pending(), None);
    assert_eq!(core.library().store(window).unwrap().snapshot().remote_sessions, None);
    assert!(banners(&mut core).is_empty());
}

#[test]
fn a_listing_still_in_flight_keeps_the_saved_row_without_a_banner() {
    let (mut core, window, workspace) = core();
    let row = saved("studio", "one", workspace);
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .restore_saved_remote_rows(Some(std::slice::from_ref(&row)));
    let now = Instant::now();
    core.remote.viewer.relaunch_reads.push(RelaunchRead {
        host: "studio".to_owned(),
        peer: RemotePeer::Ngterm,
        answer: Some(Answer::default()),
        failures: 0,
        failing_since: None,
        retry_at: now,
    });
    core.reattach_saved_remotes();
    core.tick_relaunch(now);
    assert_eq!(
        core.remote.viewer.relaunch_reads.len(),
        1,
        "no second listing of studio"
    );
    assert_eq!(core.relaunch_pending(), Some(true));
    assert_eq!(core.library().store(window).unwrap().saved_remote_rows(), &[row]);
    assert!(banners(&mut core).is_empty());
}

#[test]
fn an_unattachable_saved_session_is_forgotten_with_its_exact_refusal() {
    let (mut core, window, workspace) = core();
    let row = saved("studio", "broken", workspace);
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .restore_saved_remote_rows(Some(std::slice::from_ref(&row)));
    let tree = serde_json::from_value(json!({
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": [{"id": row.session_id, "name": "broken", "windowID": "w", "windowName": "main",
            "workspaceID": "ws", "workspaceName": "work", "cwd": "/repo", "panes": []}]
    }))
    .unwrap();

    core.reattach_from(&tree, &read("studio", RemotePeer::Ngterm));

    assert!(core.library().store(window).unwrap().saved_remote_rows().is_empty());
    assert_eq!(banners(&mut core), [(
        "remote-relaunch-failed-studio".to_owned(), "Attach Remote".to_owned(),
        "1 session from studio not attached again (could not be attached): broken: studio reported panes ngterm cannot address".to_owned()
    )]);
}

/// A Mac running both apps: each app's saved rows get a listing of their own app, so one app being down, or answering
/// in the other's place, never reads the other's rows as gone.
#[test]
fn each_apps_saved_rows_are_listed_from_that_app_alone() {
    let (mut core, window, workspace) = core();
    let runner = Arc::new(Recording::default());
    core.set_remote_runner(runner.clone());
    let ngterm = saved("studio", "one", workspace);
    let agterm = RemoteSessionSnapshot {
        peer: Some(RemotePeer::Agterm),
        ..saved("studio", "two", workspace)
    };
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .restore_saved_remote_rows(Some(&[ngterm.clone(), agterm.clone()]));

    core.reattach_saved_remotes();

    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while runner.0.lock().unwrap().len() < 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let mut lines: Vec<Vec<String>> = runner.0.lock().unwrap().clone();
    lines.sort_by_key(|argv| argv.last().unwrap().contains("agtermctl"));
    let [ngterm_line, agterm_line] = lines.as_slice() else {
        panic!("{lines:?}");
    };
    for argv in [ngterm_line, agterm_line] {
        assert_eq!(argv[argv.len() - 2], "studio", "ssh is given the destination alone");
    }
    let (ngterm_tail, agterm_tail) = (ngterm_line.last().unwrap(), agterm_line.last().unwrap());
    assert!(ngterm_tail.contains("ngtermctl live tree --json"), "{ngterm_tail}");
    assert!(
        !ngterm_tail.contains("agtermctl"),
        "no fallback to agterm: {ngterm_tail}"
    );
    assert!(agterm_tail.contains("agtermctl zmx tree --json"), "{agterm_tail}");
    assert!(!agterm_tail.contains("ngtermctl live tree"), "{agterm_tail}");

    // agterm's empty answer drops agterm's row alone
    let tree = serde_json::from_value(json!({
        "endpoint": {"executable": "/zmx", "socketDirectory": "/tmp/agterm-zmx-x", "peer": "agterm"},
        "sessions": []
    }))
    .unwrap();
    core.reattach_from(&tree, &read("studio", RemotePeer::Agterm));
    assert_eq!(core.library().store(window).unwrap().saved_remote_rows(), &[ngterm]);
}

/// Both apps of one Mac drop rows after a reboot: each app's banner stands, neither replaces the other.
#[test]
fn each_apps_dropped_rows_get_a_banner_of_their_own() {
    let (mut core, window, workspace) = core();
    let ngterm = saved("studio", "one", workspace);
    let agterm = RemoteSessionSnapshot {
        peer: Some(RemotePeer::Agterm),
        ..saved("studio", "two", workspace)
    };
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .restore_saved_remote_rows(Some(&[ngterm, agterm]));
    let empty = |peer: &str| {
        serde_json::from_value::<ControlRemoteTree>(json!({
            "endpoint": {"executable": "/x", "socketDirectory": "/tmp/x", "peer": peer},
            "sessions": []
        }))
        .unwrap()
    };

    core.reattach_from(&empty("ngterm"), &read("studio", RemotePeer::Ngterm));
    core.reattach_from(&empty("agterm"), &read("studio", RemotePeer::Agterm));

    let identities: Vec<String> = banners(&mut core).into_iter().map(|(identity, ..)| identity).collect();
    assert_eq!(
        identities,
        ["remote-relaunch-gone-studio", "remote-relaunch-gone-studio-agterm"]
    );
}
